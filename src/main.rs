use crate::KeyPress::Key;
use nix::libc::{ioctl, TIOCGWINSZ};
use std::cell::RefCell;
use std::io::{self, BufRead, Error, Read, Write};
use std::os::raw::c_short;
use std::os::unix::prelude::*;
use termios::*;

fn ctrl_key(c: char) -> char {
    (c as u8 & 0x1f).into()
}

struct RawMode {
    inner: Termios,
}

impl RawMode {
    pub fn enable_raw_mode() -> Self {
        let fd = io::stdin().as_raw_fd();
        let mut term = Termios::from_fd(fd).unwrap();
        let mut raw_mode = Self {
            inner: term.clone(),
        };

        term.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
        term.c_oflag &= !(OPOST);
        term.c_cflag |= (CS8);
        term.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
        term.c_cc[VMIN] = 0;
        term.c_cc[VTIME] = 1;

        termios::tcsetattr(fd, TCSAFLUSH, &term);
        raw_mode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &self.inner);
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
    mode: RawMode,
    rows: i16,
    cols: i16,
}

impl Editor {
    pub fn new() -> Self {
        let fd = io::stdin().as_raw_fd();
        let mut winsize: WindowSize;

        unsafe {
            winsize = std::mem::zeroed();
            ioctl(fd, TIOCGWINSZ.into(), &mut winsize as *mut _);
        }

        let rows = winsize.ws_row;
        let cols = winsize.ws_col;

        Editor {
            mode: RawMode::enable_raw_mode(),
            rows,
            cols,
        }
    }
}

enum KeyPress {
    Quit,
    Refresh,
    Key(u8),
}

thread_local!(static EDITOR: RefCell<Editor> = RefCell::new(Editor::new()));

fn main() -> io::Result<()> {
    EDITOR.with(|ref_e| {
        let e = ref_e.borrow();
    });
    // Initial terminal setup
    refresh_screen();
    draw_rows();
    goto_start();

    let mut one_buff = [0; 1];

    while let len = io::stdin().read(&mut one_buff) {
        if len.unwrap() != 0 {
            match handle_key(one_buff[0]) {
                KeyPress::Quit => {
                    refresh_screen();
                    break;
                }
                KeyPress::Refresh => {
                    refresh_screen();
                }
                Key(_) => {}
            }
        }
    }

    Ok(())
}

fn handle_key(c: u8) -> KeyPress {
    if c == ctrl_key('q') as u8 {
        KeyPress::Quit
    } else if c == ctrl_key('x') as u8 {
        KeyPress::Refresh
    } else if !(c as char).is_control() {
        KeyPress::Key(c)
    } else {
        KeyPress::Key(c)
    }
}

fn refresh_screen() {
    clear_screen();
    goto_start();
}

fn clear_screen() {
    print!("\x1B[2J");
    io::stdout().flush();
}

fn goto_start() {
    print!("\x1b[H");
    io::stdout().flush();
}

fn draw_rows() {
    EDITOR.with(|e_ref| {
        let e = e_ref.borrow();
        for _ in 0..e.rows {
            print!("~\r\n");
        }
    });

    io::stdout().flush();
}
