#![warn(clippy::all)]
#![warn(clippy::pedantic)]

use crate::KeyPress::Key;
use nix::libc::{ioctl, TIOCGWINSZ};
use std::cell::RefCell;
use std::io::{self, Error, ErrorKind, Read, Write};
use std::os::raw::c_short;
use std::os::unix::prelude::*;
use termios::*;

enum EscSeq {
    ClearLine,
    ClearScreen,
    GotoStart,
    HideCursor,
    ShowCursor,
}

impl Into<&[u8]> for EscSeq {
    fn into(self) -> &'static [u8] {
        match self {
            EscSeq::ClearLine => b"\x1b[K",
            EscSeq::ClearScreen => b"\x1b[2J",
            EscSeq::GotoStart => b"\x1b[H",
            EscSeq::HideCursor => b"\x1b[?25l",
            EscSeq::ShowCursor => b"\x1b[?25h",
        }
    }
}

fn send_esc_seq(esc: EscSeq) {
    stdout_write(esc.into());
}

const WELCOME_MESSAGE: &str = "rilo Editor - version 0.0.1\r\n";

fn ctrl_key(c: char) -> u8 {
    c as u8 & 0x1f
}

fn stdout_write(buff: &[u8]) -> io::Result<usize> {
    let written = io::stdout().lock().write(buff);
    io::stdout().lock().flush();

    match written {
        Ok(len) => {
            if len == buff.len() {
                Ok(len)
            } else {
                Err(Error::from(ErrorKind::Other))
            }
        }
        Err(e) => Err(e),
    }
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

impl Drop for RawMode {
    fn drop(&mut self) {
        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &self.inner).unwrap();
    }
}

#[repr(C)]
struct WindowSize {
    ws_row: c_short,
    ws_col: c_short,
    ws_xpixel: c_short,
    ws_ypxiel: c_short,
}

struct Editor {
    _mode: RawMode,
    rows: i16,
    cols: i16,
}

impl Editor {
    pub fn new() -> Self {
        let mode = RawMode::enable_raw_mode();

        let (rows, cols) = get_window_size().unwrap();

        Editor {
            _mode: mode,
            rows,
            cols,
        }
    }
}

fn get_window_size() -> io::Result<(i16, i16)> {
    let fd = io::stdin().as_raw_fd();
    let mut winsize = WindowSize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypxiel: 0,
    };

    let return_code = unsafe { ioctl(fd, TIOCGWINSZ, &mut winsize as *mut _) };
    if (return_code == -1) || (winsize.ws_col == 0) {
        Error::new(
            ErrorKind::Other,
            "get_window_size: ioctl failed or returned invalid value",
        );
    }

    Ok((winsize.ws_row, winsize.ws_col))
}

enum KeyPress {
    Quit,
    Refresh,
    Escape,
    Key(u8),
}

thread_local!(static EDITOR: RefCell<Editor> = RefCell::new(Editor::new()));

fn main() -> io::Result<()> {
    EDITOR.with(|ref_e| {
        let _e = ref_e.borrow();
    });

    refresh_screen();
    draw_rows();
    send_esc_seq(EscSeq::GotoStart);

    let mut one_buff = [0; 1];

    while let len = io::stdin().read(&mut one_buff).unwrap() {
        if len != 0 {
            match handle_key(one_buff[0]) {
                KeyPress::Quit => {
                    refresh_screen();
                    break;
                }
                KeyPress::Refresh => {
                    refresh_screen();
                }
                KeyPress::Escape => {}
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

fn refresh_screen() {
    send_esc_seq(EscSeq::HideCursor);
    send_esc_seq(EscSeq::GotoStart);
    send_esc_seq(EscSeq::ShowCursor);
}

fn draw_rows() {
    EDITOR.with(|e_ref| {
        let e = e_ref.borrow();
        for idx in 0..e.rows {
            send_esc_seq(EscSeq::ClearLine);
            if idx == e.rows / 3 {
                stdout_write(WELCOME_MESSAGE.as_bytes());
            } else {
                stdout_write(b"~");
                if idx < e.rows - 1 {
                    stdout_write(b"\r\n");
                }
            }
        }
    });
}
