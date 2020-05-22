#![warn(clippy::all)]
#![warn(clippy::pedantic)]

use nix::libc::{ioctl, TIOCGWINSZ};
use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::io::{self, Error, ErrorKind, LineWriter, Read, SeekFrom, Write};
use std::os::raw::c_short;
use std::os::unix::prelude::*;
use std::path::Path;
use std::time::{Duration, Instant};
use termios::{
    Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, TCSAFLUSH,
    VMIN, VTIME,
};

const TAB_SIZE: u8 = 4;

/// The cursor's position relative to the terminal
#[derive(Copy, Clone, Default)]
struct CursorPosition {
    x: usize,
    y: usize,
}

/// An enum representing a navigation key press
enum NavigationKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

enum Action {
    Quit,
    Refresh,
    Escape,
    Save,
    Delete,
    Enter,
    Input(char),
}

impl From<u8> for Action {
    fn from(c: u8) -> Self {
        if c == ctrl_key('q') {
            Action::Quit
        } else if c == ctrl_key('x') {
            Action::Refresh
        } else if c == ctrl_key('s') {
            Action::Save
        } else if c == b'\x1b' {
            Action::Escape
        } else if c == 27 || c == 127 {
            Action::Delete
        } else if c == b'\r' {
            Action::Enter
        } else {
            Action::Input(c as char)
        }
    }
}

/// Various commands we might issue to the terminal
enum CtrlSeq {
    /// Clears the entire line
    ClearLine,
    /// Clears the entire screen, appropriate before either drawing or quitting rilo
    ClearScreen,
    /// A shortcut to go the start of the terminal (0, 0)
    GotoStart,
    /// Hides the cursor, useful to prevent flashing
    HideCursor,
    /// Displays the cursor after we hide it
    ShowCursor,
    /// Moves the cursor to a position in the terminal
    MoveCursor(CursorPosition),
    InverteColor,
    NormalColor,
}

impl From<CtrlSeq> for Vec<u8> {
    fn from(ctrl: CtrlSeq) -> Self {
        match ctrl {
            CtrlSeq::ClearLine => b"\x1b[K".to_vec(),
            CtrlSeq::ClearScreen => b"\x1b[2J".to_vec(),
            CtrlSeq::GotoStart => b"\x1b[H".to_vec(),
            CtrlSeq::HideCursor => b"\x1b[?25l".to_vec(),
            CtrlSeq::ShowCursor => b"\x1b[?25h".to_vec(),
            CtrlSeq::MoveCursor(cp) => format!("\x1b[{};{}H", cp.y + 1, cp.x + 1)
                .as_bytes()
                .to_vec(),
            CtrlSeq::InverteColor => b"\x1b[7m".to_vec(),
            CtrlSeq::NormalColor => b"\x1b[m".to_vec(),
        }
    }
}

/// A helper function to write data into stdout
fn stdout_write(buff: impl AsRef<[u8]>) {
    io::stdout().lock().write_all(buff.as_ref()).unwrap();
    io::stdout().lock().flush().unwrap();
}

struct RawMode {
    inner: Termios,
}

impl RawMode {
    pub fn enable_raw_mode() -> Self {
        let fd = io::stdin().as_raw_fd();
        let mut term = Termios::from_fd(fd).unwrap();
        let raw_mode = Self { inner: term };

        term.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
        term.c_oflag &= !(OPOST);
        term.c_cflag |= CS8;
        term.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
        term.c_cc[VMIN] = 0;
        term.c_cc[VTIME] = 1;

        termios::tcsetattr(fd, TCSAFLUSH, &term).unwrap();
        raw_mode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &self.inner).unwrap();
    }
}

#[derive(Default)]
#[repr(C)]
struct WindowSize {
    ws_row: c_short,
    ws_col: c_short,
    ws_xpixel: c_short,
    ws_ypxiel: c_short,
}

struct SystemMessage {
    message: Option<String>,
    time: Instant,
}

impl Default for SystemMessage {
    fn default() -> Self {
        SystemMessage {
            message: None,
            time: Instant::now(),
        }
    }
}

impl SystemMessage {
    fn new(message: &str) -> Self {
        SystemMessage {
            message: Some(message.to_string()),
            time: Instant::now(),
        }
    }
}

type Row = String;

struct Editor {
    _mode: RawMode,
    term_rows: usize,
    term_cols: usize,
    cur_pos: CursorPosition,
    row_offset: usize,
    col_offset: usize,
    tab_size: u8,
    file: Option<File>,
    rows: Vec<Row>,
    message: SystemMessage,
    dirty_flag: bool,
    path: Option<String>,
}

impl Editor {
    fn new() -> Self {
        let mode = RawMode::enable_raw_mode();

        let (rows, cols) = get_window_size().expect("Couldn't get window size from terminal.");

        Editor {
            _mode: mode,
            term_rows: (rows - 2) as usize, // -2 to leave a row for the status bar
            term_cols: (cols - 1) as usize,
            cur_pos: CursorPosition::default(),
            row_offset: 0,
            col_offset: 0,
            tab_size: TAB_SIZE,
            file: Default::default(),
            rows: Default::default(),
            message: SystemMessage::new("HELP: Ctrl-S = save | Ctrl-Q = quit"),
            dirty_flag: false,
            path: None,
        }
    }

    /// Handles both the internal state held in the Editor, and moves the cursor on the terminal
    fn move_cursor(&mut self, ak: &NavigationKey) {
        match ak {
            NavigationKey::Left => {
                if self.cur_pos.x != 0 {
                    self.cur_pos.x -= 1;
                } else if self.cur_pos.x == 0 && self.col_offset != 0 {
                    self.col_offset -= 1;
                } else if self.cur_pos.x == 0 && self.col_offset == 0 {
                    if self.cur_pos.y == 0 && self.row_offset != 0 {
                        self.row_offset -= 1;
                    } else if !(self.cur_pos.y == 0 && self.row_offset == 0) {
                        self.cur_pos.y -= 1;
                    }

                    if !(self.row_offset == 0 && self.cur_pos.y == 0) {
                        if let Some(current_line) = self.current_line() {
                            let line_length = current_line.len().saturating_sub(1);
                            if line_length > self.term_cols {
                                self.col_offset = line_length - self.term_cols;
                                self.cur_pos.x = self.term_cols;
                            } else {
                                self.col_offset = 0;
                                self.cur_pos.x = line_length;
                            }
                        }
                    }
                }
            }
            NavigationKey::Right => {
                if let Some(current_line) = self.current_line() {
                    let line_length = current_line.len();
                    if self.cur_pos.x == line_length
                        || self.cur_pos.x + self.col_offset == line_length
                    {
                        if self.cur_pos.y + self.row_offset != self.rows.len() {
                            self.cur_pos.x = 0;
                            self.col_offset = 0;

                            if self.cur_pos.y == self.term_rows {
                                self.row_offset += 1
                            } else {
                                self.cur_pos.y += 1;
                            }
                        }
                    } else if self.cur_pos.x != self.term_cols {
                        self.cur_pos.x += 1;
                    } else if self.cur_pos.x == self.term_cols {
                        self.col_offset += 1;
                    }
                }
            }
            NavigationKey::Up => {
                if self.cur_pos.y != 0 {
                    self.cur_pos.y -= 1;
                } else if self.cur_pos.y == 0 && self.row_offset != 0 {
                    self.row_offset -= 1;
                }

                if let Some(next_line) = self.current_line() {
                    if self.cur_pos.x > next_line.len() {
                        self.cur_pos.x = next_line.len();
                    }
                }
            }
            NavigationKey::Down => {
                let file_length = self.rows.len() - 1;
                if self.row_offset + self.cur_pos.y != file_length {
                    if self.cur_pos.y != self.term_rows {
                        self.cur_pos.y += 1;
                    } else if self.cur_pos.y == self.term_rows
                        && self.row_offset + self.term_rows != self.rows.len()
                    {
                        self.row_offset += 1;
                    }

                    if let Some(next_line) = self.current_line() {
                        if self.cur_pos.x > next_line.len() {
                            self.cur_pos.x = next_line.len().saturating_sub(1);
                        }
                    }
                }
            }
            NavigationKey::Home => {
                self.cur_pos.x = 0;
                self.col_offset = 0;
            }
            NavigationKey::End => {
                let current_line_len = self.current_line().unwrap().len();
                self.cur_pos.x = match self.term_cols.cmp(&current_line_len) {
                    Ordering::Greater | Ordering::Equal => current_line_len,
                    Ordering::Less => {
                        self.col_offset = current_line_len - self.term_cols;
                        self.term_cols
                    }
                };
            }
            NavigationKey::PageUp => {
                self.cur_pos.y = 0;
                if let Some(next_line) = self.current_line() {
                    if self.cur_pos.x > next_line.len() {
                        self.cur_pos.x = next_line.len();
                    }
                }
            }
            NavigationKey::PageDown => {
                self.cur_pos.y = self.term_rows;
                if let Some(next_line) = self.current_line() {
                    if self.cur_pos.x > next_line.len() {
                        self.cur_pos.x = next_line.len();
                    }
                }
            }
        };

        // Render correct rx
        // self.rx = if let Some(curr_line) = self.current_line() {
        //     cx_to_rx(curr_line, self.cur_pos.x)
        // } else {
        //     0
        // };

        send_esc_seq(CtrlSeq::MoveCursor(CursorPosition {
            x: self.rx(),
            y: self.cur_pos.y,
        }));
    }

    /// Open a file to edit/read
    fn open(&mut self, filename: impl AsRef<Path> + Clone) -> io::Result<()> {
        if filename.as_ref().is_file() {
            self.file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&filename)
                .ok();

            self.path = Some(String::from(filename.as_ref().to_str().unwrap()));

            self.rows = io::BufReader::new(self.file.as_ref().unwrap())
                .lines()
                .map(std::result::Result::unwrap)
                .collect();
        }

        Ok(())
    }

    fn save(&mut self) -> io::Result<()> {
        // TODO: Move all system message handeling from main loop to this function
        if let Some(f) = &mut self.file {
            f.seek(SeekFrom::Start(0))?;
            f.set_len(0)?;
            let mut writer = LineWriter::new(f);
            self.rows.iter().for_each(|row| {
                writer.write_all(format!("{}\n", row).as_bytes()).unwrap();
            });

            writer.flush()?;

            self.dirty_flag = false;
        } else {
            // TODO: prompt some "save as" stuff
        }

        Ok(())
    }

    /// Draws out the current state held in the editor to the terminal
    fn draw(&mut self) {
        // We use a Vec we can push all the data on screen into, and then write it in one go into stdout
        let mut append_buffer: Vec<u8> = Vec::new();
        append_buffer.append(&mut CtrlSeq::ClearLine.into());
        for idx in self.row_offset..=self.term_rows + self.row_offset {
            if idx < self.rows.len() {
                let line = &self.rows[idx];
                // If the line is long enough to see anything because of horizontal scrolling
                if line.len() > self.col_offset {
                    let range = if line.len() > self.col_offset + self.term_cols {
                        self.col_offset..self.col_offset + self.term_cols
                    } else {
                        self.col_offset..line.len()
                    };
                    let ranged_line = line[range].to_string();
                    append_buffer.extend(render_row(&ranged_line, self.tab_size));
                }
            } else {
                append_buffer.push(b'~');
            }

            append_buffer.push(b'\r');
            append_buffer.push(b'\n');
            append_buffer.append(&mut CtrlSeq::ClearLine.into());
        }

        append_buffer.extend(self.render_status_bar());

        send_esc_seq(CtrlSeq::HideCursor);
        send_esc_seq(CtrlSeq::GotoStart);
        stdout_write(append_buffer);
        // self.rx = if let Some(curr_line) = self.current_line() {
        //     cx_to_rx(curr_line, self.cur_pos.x)
        // } else {
        //     0
        // };
        send_esc_seq(CtrlSeq::MoveCursor(CursorPosition {
            x: self.rx(),
            y: self.cur_pos.y,
        }));
        send_esc_seq(CtrlSeq::ShowCursor);
    }

    fn current_line(&self) -> Option<&Row> {
        let current_line_idx = self.row_offset + self.cur_pos.y;
        self.rows.get(current_line_idx)
    }

    fn rx(&self) -> usize {
        if let Some(line) = self.current_line() {
            line[0..self.cur_pos.x]
                .chars()
                .fold(0, |acc, c| match c.cmp(&'\t') {
                    Ordering::Equal => acc + 4,
                    _ => acc + 1,
                })
        } else {
            0
        }
    }

    fn render_status_bar(&self) -> Vec<u8> {
        //TODO: Make the status bar nicer
        let mut v = Vec::new();
        v.append(&mut CtrlSeq::InverteColor.into());

        match self.file {
            None => v.extend(b"[No open file]"),
            Some(_) => {
                let open_file = format!("[Open: {}]        ", self.path.as_ref().unwrap());
                v.extend(open_file.as_bytes());
                let current_line_idx = self.cur_pos.y + self.row_offset;
                let precenteges = ((current_line_idx + 1) * 100)
                    .checked_div(self.rows.len())
                    .unwrap_or(0);
                let lines = format!("{}/{}", current_line_idx + 1, self.rows.len());
                v.extend(lines.as_bytes());
                let formated = format!("        {}%", precenteges);
                v.extend(formated.as_bytes());

                if let Some(message) = &self.message.message {
                    if self.message.time.elapsed() < Duration::from_secs(5) {
                        let display_message = format!("        {}", message);
                        v.extend(display_message.as_bytes());
                    }
                }
            }
        }

        v.extend(vec![b' '; self.term_cols.saturating_sub(v.len())]);

        // TODO: This is not good :(
        let mut v = v[0..self.term_cols].to_vec();
        v.append(&mut CtrlSeq::NormalColor.into());
        v
    }

    fn insert_newline(&mut self) {
        self.dirty_flag = true;
        let x = self.cur_pos.x + self.col_offset;
        let y = self.cur_pos.y + self.row_offset;
        let curr_line = self.rows[y].clone();
        self.rows.insert(y, String::new());
        self.rows[y] = curr_line[0..x].to_string();
        self.rows[y + 1] = curr_line[x..].to_string();
        self.cur_pos.x = 0;
        self.col_offset = 0;
        self.cur_pos.y += 1;
        self.col_offset = 0;
    }

    fn insert_char(&mut self, c: char) {
        self.dirty_flag = true;
        let x = self.cur_pos.x + self.col_offset;
        let y = self.cur_pos.y + self.row_offset;

        // If we are on the last row in the file
        if y == self.rows.len() {
            let mut row = String::new();
            row.push(c);
            self.rows.push(row);
        } else {
            let row = self.rows[y].clone();
            let new_row = [&row[0..x], c.to_string().as_str(), &row[x..]].concat();
            self.rows[y] = new_row;
        }

        self.move_cursor(&NavigationKey::Right);
    }

    fn remove_char(&mut self) {
        self.dirty_flag = true;
        let x = self.cur_pos.x + self.col_offset;
        let y = self.cur_pos.y + self.row_offset;

        // Remove row and move one up
        if x == 0 {
            if let Some(line) = &mut self.current_line() {
                self.rows[y - 1] = [self.rows[y - 1].clone(), line.to_string()].concat();
                self.rows.remove(y);
            }
        } else {
            let row = self.rows[y].clone();
            if x != 0 {
                self.rows[y] = [&row[0..x - 1], &row[x..]].concat();
            };
        }

        self.move_cursor(&NavigationKey::Left);
    }
}

fn main() -> io::Result<()> {
    let mut e = Editor::new();

    refresh_screen();
    let args: Vec<String> = std::env::args().collect();

    // TODO: Change to clap or another library that handles command line arguments
    if let Some(filename) = args.get(1) {
        e.open(filename)?
    }

    e.draw();

    let mut buff = [0; 1];
    loop {
        if io::stdin().read(&mut buff)? != 0 {
            match buff[0].into() {
                Action::Quit => {
                    send_esc_seq(CtrlSeq::ClearScreen);
                    send_esc_seq(CtrlSeq::GotoStart);
                    break;
                }
                Action::Refresh => {
                    refresh_screen();
                }
                Action::Escape => {
                    if let Ok(ak) = handle_escape_seq() {
                        e.move_cursor(&ak);
                    }
                }
                Action::Save => {
                    if e.dirty_flag {
                        e.message = SystemMessage::new(match e.save() {
                            Ok(_) => "File saved successfully!",
                            Err(_) => "Error saving file!",
                        })
                    } else {
                        e.message = SystemMessage::new("No Changes Made!");
                        e.dirty_flag = false;
                    }
                }
                Action::Delete => {
                    e.remove_char();
                }
                Action::Enter => e.insert_newline(),
                Action::Input(c) => {
                    if !c.is_ascii_control() {
                        e.insert_char(c)
                    }
                }
            }

            e.draw();
        }
    }

    Ok(())
}

fn handle_escape_seq() -> io::Result<NavigationKey> {
    let mut buffer = [0; 3];
    // We need to use read() because some esc sequences are 3 bytes and some are 2
    io::stdin().lock().read(&mut buffer)?;
    if buffer[0] == b'[' {
        let movement = match buffer[1] {
            b'A' => NavigationKey::Up,
            b'B' => NavigationKey::Down,
            b'C' => NavigationKey::Right,
            b'D' => NavigationKey::Left,
            b'H' => NavigationKey::Home,
            b'F' => NavigationKey::End,
            b'5' => NavigationKey::PageUp,
            b'6' => NavigationKey::PageDown,
            _ => return Err(Error::from(ErrorKind::InvalidData)),
        };

        Ok(movement)
    } else {
        Err(Error::from(ErrorKind::InvalidData))
    }
}

fn refresh_screen() {
    send_esc_seq(CtrlSeq::HideCursor);
    send_esc_seq(CtrlSeq::ClearScreen);
    send_esc_seq(CtrlSeq::ShowCursor);
}

fn render_row(row: &str, tab_size: u8) -> Vec<u8> {
    row.chars()
        .flat_map(|c| match c.cmp(&'\t') {
            Ordering::Equal => vec![b' '; tab_size.into()],
            _ => vec![c as u8],
        })
        .collect()
}

/// Send an escape sequence to the actual terminal
fn send_esc_seq(ctrl: CtrlSeq) {
    stdout_write(Vec::from(ctrl));
}

fn ctrl_key(c: char) -> u8 {
    c as u8 & 0x1f
}

/// Gets terminal size as (X, Y) tuple. **Note:** libc returns a value in the  [1..N] range, so we do the same
fn get_window_size() -> io::Result<(i16, i16)> {
    let fd = io::stdin().as_raw_fd();
    let mut winsize = WindowSize::default();

    let return_code = unsafe { ioctl(fd, TIOCGWINSZ, &mut winsize as *mut _) };
    if (return_code == -1) || (winsize.ws_col == 0) {
        Err(Error::new(
            ErrorKind::Other,
            "get_window_size: ioctl failed or returned invalid value",
        ))
    } else {
        Ok((winsize.ws_row, winsize.ws_col))
    }
}
