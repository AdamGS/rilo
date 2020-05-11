#![warn(clippy::all)]
#![warn(clippy::pedantic)]

use nix::libc::{ioctl, TIOCGWINSZ};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, Error, ErrorKind, Read, Write};
use std::os::raw::c_short;
use std::os::unix::prelude::*;
use std::path::Path;
use termios::{
    Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, TCSAFLUSH,
    VMIN, VTIME,
};

/// The cursor's position, **inside** the terminal.
#[derive(Copy, Clone, Default)]
struct CursorPosition {
    x: usize,
    y: usize,
}

/// An enum representing a navigation key press
enum ArrowKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
}

enum KeyPress {
    Quit,
    Refresh,
    Escape,
    Key(u8),
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
        }
    }
}

/// Send an escape sequence to the actual terminal
fn send_esc_seq(ctrl: CtrlSeq) {
    stdout_write(Vec::from(ctrl));
}

fn ctrl_key(c: char) -> u8 {
    c as u8 & 0x1f
}

/// Gets terminal size as (X, Y) tuple. **Note:** libc returns a value in the  [1..N] range,
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
}

impl Editor {
    fn new() -> Self {
        let mode = RawMode::enable_raw_mode();

        let (rows, cols) = get_window_size().expect("Couldn't get window size from terminal.");

        Editor {
            _mode: mode,
            term_rows: (rows - 1) as usize,
            term_cols: (cols - 1) as usize,
            cur_pos: CursorPosition::default(),
            row_offset: 0,
            col_offset: 0,
            tab_size: 4,
            file: Default::default(),
            rows: Default::default(),
        }
    }

    /// Handles both the internal state held in the Editor, and moves the cursor on the terminal
    fn move_cursor(&mut self, ak: &ArrowKey) {
        match ak {
            ArrowKey::Left => {
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
                            let line_length = current_line.len();
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
            ArrowKey::Right => {
                if let Some(current_line) = self.current_line() {
                    if self.cur_pos.x == current_line.len() {
                        self.cur_pos.x = 0;
                        self.col_offset = 0;

                        if self.cur_pos.y == self.term_rows {
                            self.row_offset += 1
                        } else {
                            self.cur_pos.y += 1;
                        }
                    } else if self.cur_pos.x != self.term_cols {
                        self.cur_pos.x += 1;
                    } else if self.cur_pos.x == self.term_cols {
                        self.col_offset += 1;
                    }
                }
            }
            ArrowKey::Up => {
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
            ArrowKey::Down => {
                let file_length = self.rows.len();
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
                            self.cur_pos.x = next_line.len();
                        }
                    }
                }
            }
            ArrowKey::Home => {
                self.cur_pos.x = 0;
                self.col_offset = 0;
            }
            ArrowKey::End => {
                let current_line_len = self.current_line().unwrap().len();
                self.cur_pos.x = match self.term_cols.cmp(&current_line_len) {
                    Ordering::Greater | Ordering::Equal => current_line_len,
                    Ordering::Less => {
                        self.col_offset = current_line_len - self.term_cols;
                        self.term_cols
                    }
                };
            }
            ArrowKey::PageUp => {
                self.cur_pos.y = 0;
                if let Some(next_line) = self.current_line() {
                    if self.cur_pos.x > next_line.len() {
                        self.cur_pos.x = next_line.len();
                    }
                }
            }
            ArrowKey::PageDown => {
                self.cur_pos.y = self.term_rows;
                if let Some(next_line) = self.current_line() {
                    if self.cur_pos.x > next_line.len() {
                        self.cur_pos.x = next_line.len();
                    }
                }
            }
            ArrowKey::Delete => {}
        };

        send_esc_seq(CtrlSeq::MoveCursor(self.cur_pos));
    }

    /// Open a file to edit/read
    fn open(&mut self, filename: impl AsRef<Path>) -> io::Result<()> {
        use std::io::BufRead;

        if filename.as_ref().is_file() {
            self.file = Some(File::open(filename)?);
            self.rows = io::BufReader::new(self.file.as_ref().unwrap())
                .lines()
                .map(std::result::Result::unwrap)
                .collect();
        }

        Ok(())
    }

    /// Draws out the current state held in the editor to the terminal
    fn draw(&self) {
        // We use a Vec we can push all the data on screen into, and then write it in one go into stdout
        let mut append_buffer: Vec<u8> = Vec::new();
        append_buffer.append(&mut CtrlSeq::ClearLine.into());
        for idx in self.row_offset..=self.term_rows + self.row_offset {
            if idx < self.rows.len() {
                let line = &self.rows[idx];
                if line.len() > self.col_offset {
                    let range = if line.len() < self.term_cols {
                        self.col_offset..line.len()
                    } else if line.len() > self.col_offset + self.term_cols {
                        self.col_offset..self.col_offset + self.term_cols
                    } else {
                        self.col_offset..line.len()
                    };
                    let ranged_line = line[range].to_string();
                    let mut rendered_line = render_row(&ranged_line, self.tab_size);
                    append_buffer.append(&mut rendered_line)
                }
            } else {
                append_buffer.push(b'~');
            }

            if idx < self.term_rows + self.row_offset {
                append_buffer.push(b'\r');
                append_buffer.push(b'\n');
                append_buffer.append(&mut CtrlSeq::ClearLine.into());
            }
        }

        send_esc_seq(CtrlSeq::HideCursor);
        send_esc_seq(CtrlSeq::GotoStart);
        stdout_write(append_buffer);
        send_esc_seq(CtrlSeq::MoveCursor(self.cur_pos));
        send_esc_seq(CtrlSeq::ShowCursor);
    }

    fn current_line(&self) -> Option<&Row> {
        let current_line_idx = self.row_offset + self.cur_pos.y;
        self.rows.get(current_line_idx)
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
            match handle_key(buff[0]) {
                KeyPress::Quit => {
                    send_esc_seq(CtrlSeq::ClearScreen);
                    send_esc_seq(CtrlSeq::GotoStart);
                    break;
                }
                KeyPress::Refresh => {
                    refresh_screen();
                    e.draw();
                }
                KeyPress::Escape => {
                    if let Ok(ak) = handle_escape_seq() {
                        e.move_cursor(&ak);
                    }
                }
                KeyPress::Key(_) => {}
            }

            e.draw();
        }
    }

    Ok(())
}

fn handle_key(c: u8) -> KeyPress {
    if c == ctrl_key('q') {
        KeyPress::Quit
    } else if c == ctrl_key('x') {
        KeyPress::Refresh
    } else if c == b'\x1b' {
        KeyPress::Escape
    } else {
        KeyPress::Key(c)
    }
}

fn handle_escape_seq() -> io::Result<ArrowKey> {
    let mut buffer = [0; 3];

    io::stdin().lock().read(&mut buffer).unwrap();
    if buffer[0] == b'[' {
        let movement = match buffer[1] {
            b'A' => ArrowKey::Up,
            b'B' => ArrowKey::Down,
            b'C' => ArrowKey::Right,
            b'D' => ArrowKey::Left,
            b'H' => ArrowKey::Home,
            b'F' => ArrowKey::End,
            b'3' => ArrowKey::Delete,
            b'5' => ArrowKey::PageUp,
            b'6' => ArrowKey::PageDown,
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
        .collect::<Vec<_>>()
}
