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
    rows: i16,
    cols: i16,
    cur_pos: CursorPosition,
}

impl Editor {
    pub fn new() -> Self {
        let mode = RawMode::enable_raw_mode();

        let (rows, cols) = get_window_size().unwrap();
        let cur_pos = CursorPosition::default();

        Editor {
            _mode: mode,
            rows,
            cols,
            cur_pos,
        }
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

enum KeyPress {
    Quit,
    Refresh,
    Escape,
    Key(u8),
}

thread_local!(static EDITOR: RefCell<Editor> = RefCell::new(Editor::new()));

fn main() -> io::Result<()> {
    // This is a hack to make the EDITOR to initilize, buy maybe everything should be a method of editor?
    EDITOR.with(|ref_e| {
        let _e = ref_e.borrow();
    });

    refresh_screen();
    draw_rows();
    //send_esc_seq(EscSeq::GotoStart);

    let mut one_buff = [0; 1];

    while let len = io::stdin().read(&mut one_buff).unwrap() {
        if len != 0 {
            match handle_key(one_buff[0]) {
                KeyPress::Quit => {
                    send_esc_seq(EscSeq::ClearScreen);
                    send_esc_seq(EscSeq::GotoStart);
                    // Cleaning up here, because Drop implementation caused weird problems.
                    EDITOR.with(|r| {
                        let editor = r.borrow();
                        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &editor._mode.inner)
                            .unwrap();
                    });
                    break;
                }
                KeyPress::Refresh => {
                    refresh_screen();
                }
                KeyPress::Escape => {
                    handle_escape_seq();
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

fn handle_escape_seq() {
    let mut buffer = [0; 3];

    io::stdin().lock().read(&mut buffer).unwrap();
    if buffer[0] == '[' as u8 {
        let movement = match buffer[1] as char {
            'A' => ArrowKey::Up,
            'B' => ArrowKey::Down,
            'C' => ArrowKey::Right,
            'D' => ArrowKey::Left,
            _ => unreachable!(),
        };

        EDITOR.with(|r| {
            let mut editor = r.borrow_mut();
            match movement {
                ArrowKey::Left => {
                    if editor.cur_pos.x != 0 {
                        editor.cur_pos.x -= 1;
                    }
                }
                ArrowKey::Right => {
                    if editor.cur_pos.x != editor.cols {
                        editor.cur_pos.x += 1;
                    }
                }
                ArrowKey::Up => {
                    if editor.cur_pos.y != 0 {
                        editor.cur_pos.y -= 1;
                    }
                }
                ArrowKey::Down => {
                    if editor.cur_pos.y != editor.rows {
                        editor.cur_pos.y += 1;
                    }
                }
            };

            send_esc_seq(EscSeq::MoveCursor(editor.cur_pos));
        });
    }
}

fn refresh_screen() {
    send_esc_seq(EscSeq::HideCursor);
    send_esc_seq(EscSeq::ClearScreen);
    draw_rows();
    send_esc_seq(EscSeq::ShowCursor);
}

fn draw_rows() {
    send_esc_seq(EscSeq::GotoStart);
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

        send_esc_seq(EscSeq::MoveCursor(e.cur_pos));
    });
}
