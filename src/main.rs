#![warn(clippy::all)]
#![warn(clippy::pedantic)]

use crate::EscSeq::GotoStart;
use crate::KeyPress::Key;
use nix::libc::{ioctl, TIOCGWINSZ};
use nix::sys::socket::send;
use nix::unistd::SysconfVar::MQ_OPEN_MAX;
use std::cell::RefCell;
use std::io::{self, Error, ErrorKind, Read, Write};
use std::os::raw::c_short;
use std::os::unix::prelude::*;
use termios::{
    Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, TCSAFLUSH,
    VMIN, VTIME,
};

#[derive(Copy, Clone, Default)]
struct CursorPosition {
    x: i16,
    y: i16,
}

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

enum EscSeq {
    ClearLine,
    ClearScreen,
    GotoStart,
    HideCursor,
    ShowCursor,
    MoveCursor(CursorPosition),
}

impl Into<Vec<u8>> for EscSeq {
    fn into(self) -> Vec<u8> {
        match self {
            EscSeq::ClearLine => b"\x1b[K".to_vec(),
            EscSeq::ClearScreen => b"\x1b[2J".to_vec(),
            EscSeq::GotoStart => b"\x1b[H".to_vec(),
            EscSeq::HideCursor => b"\x1b[?25l".to_vec(),
            EscSeq::ShowCursor => b"\x1b[?25h".to_vec(),
            EscSeq::MoveCursor(cp) => format!("\x1b[{};{}H", cp.y + 1, cp.x + 1)
                .as_bytes()
                .to_vec(),
        }
    }
}

fn send_esc_seq(esc: EscSeq) {
    let v: Vec<u8> = esc.into();
    stdout_write(&v);
}

const WELCOME_MESSAGE: &str = "rilo Editor - version 0.0.1\r\n";

fn ctrl_key(c: char) -> u8 {
    c as u8 & 0x1f
}

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
        term.c_cflag |= (CS8);
        term.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
        term.c_cc[VMIN] = 0;
        term.c_cc[VTIME] = 1;

        termios::tcsetattr(fd, TCSAFLUSH, &term).unwrap();
        raw_mode
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

struct Editor {
    _mode: RawMode,
    term_rows: i16,
    term_cols: i16,
    cur_pos: CursorPosition,
    rows: Vec<Row>,
}

type Row = String;

impl Editor {
    fn new() -> Self {
        let mode = RawMode::enable_raw_mode();

        let (rows, cols) = get_window_size().unwrap();
        let cur_pos = CursorPosition::default();

        let mut content_rows: Vec<Row> = Default::default();
        content_rows.push("Hello World".to_string());

        Editor {
            _mode: mode,
            term_rows: rows - 1,
            term_cols: cols - 1,
            cur_pos,
            rows: content_rows,
        }
    }

    fn move_cursor(&mut self, ak: ArrowKey) {
        match ak {
            ArrowKey::Left => {
                if self.cur_pos.x != 0 {
                    self.cur_pos.x -= 1;
                }
            }
            ArrowKey::Right => {
                if self.cur_pos.x != self.term_cols {
                    self.cur_pos.x += 1;
                }
            }
            ArrowKey::Up => {
                if self.cur_pos.y != 0 {
                    self.cur_pos.y -= 1;
                }
            }
            ArrowKey::Down => {
                if self.cur_pos.y != self.term_rows {
                    self.cur_pos.y += 1;
                }
            }
            ArrowKey::Home => {
                self.cur_pos.x = 0;
            }
            ArrowKey::End => {
                self.cur_pos.x = self.term_cols;
            }
            ArrowKey::PageUp => {
                self.cur_pos.y = 0;
            }
            ArrowKey::PageDown => {
                self.cur_pos.y = self.term_rows;
            }
            ArrowKey::Delete => {}
        };

        send_esc_seq(EscSeq::MoveCursor(self.cur_pos));
    }

    fn draw(&self) {
        send_esc_seq(EscSeq::GotoStart);
        for idx in 0..self.term_rows {
            send_esc_seq(EscSeq::ClearLine);
            if (idx as usize) < self.rows.len() {
                stdout_write(format!("{}\r\n", &self.rows[idx as usize]));
            } else {
                //send_esc_seq(EscSeq::ClearLine);
                if idx == self.term_rows / 3 {
                    stdout_write(WELCOME_MESSAGE.as_bytes());
                } else {
                    stdout_write(b"~");
                    if idx < self.term_rows - 1 {
                        stdout_write(b"\r\n");
                    }
                }
            }
        }

        send_esc_seq(EscSeq::MoveCursor(self.cur_pos));
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &self._mode.inner).unwrap();
    }
}

fn get_window_size() -> io::Result<(i16, i16)> {
    let fd = io::stdin().as_raw_fd();
    let mut winsize = WindowSize::default();

    let return_code = unsafe { ioctl(fd, TIOCGWINSZ, &mut winsize as *mut _) };
    if (return_code == -1) || (winsize.ws_col == 0) {
        Error::new(
            ErrorKind::Other,
            "get_window_size: ioctl failed or returned invalid value",
        );
    }

    Ok((winsize.ws_row, winsize.ws_col))
}

fn main() -> io::Result<()> {
    // This is a hack to make the EDITOR to initilize, buy maybe everything should be a method of editor?
    let mut e = Editor::new();

    refresh_screen();
    e.draw();
    //send_esc_seq(EscSeq::GotoStart);

    let mut buff = [0; 1];

    while let len = io::stdin().read(&mut buff).unwrap() {
        if len != 0 {
            match handle_key(buff[0]) {
                KeyPress::Quit => {
                    send_esc_seq(EscSeq::ClearScreen);
                    send_esc_seq(EscSeq::GotoStart);
                    // With impl_editor, the drop should work!
                    break;
                }
                KeyPress::Refresh => {
                    refresh_screen();
                }
                KeyPress::Escape => {
                    let ak = handle_escape_seq().unwrap();
                    e.move_cursor(ak);
                }
                Key(_) => {}
            }
        }
    }

    Ok(())
}

fn handle_key(c: u8) -> KeyPress {
    if c == ctrl_key('q') {
        KeyPress::Quit
    } else if c == ctrl_key('x') {
        KeyPress::Refresh
    } else if c == 27 {
        KeyPress::Escape
    } else {
        KeyPress::Key(c)
    }
}

fn handle_escape_seq() -> io::Result<ArrowKey> {
    let mut buffer = [0; 3];

    io::stdin().lock().read(&mut buffer).unwrap();
    if buffer[0] == '[' as u8 {
        let movement = match buffer[1] as char {
            'A' => ArrowKey::Up,
            'B' => ArrowKey::Down,
            'C' => ArrowKey::Right,
            'D' => ArrowKey::Left,
            'H' => ArrowKey::Home,
            'F' => ArrowKey::End,
            '3' => ArrowKey::Delete,
            '5' => ArrowKey::PageUp,
            '6' => ArrowKey::PageDown,
            _ => return Err(Error::from(ErrorKind::InvalidData)),
        };

        return Ok(movement);
    }

    Err(Error::from(ErrorKind::InvalidData))
}

fn refresh_screen() {
    send_esc_seq(EscSeq::HideCursor);
    send_esc_seq(EscSeq::ClearScreen);
    send_esc_seq(EscSeq::ShowCursor);
}
