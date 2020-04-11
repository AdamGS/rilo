use std::io::{self, BufRead, Error, Read, Write};
use std::os::unix::prelude::*;
use termios::*;

fn ctrl_key(c: char) -> char {
    (c as u8 & 0x1f).into()
}

struct RawMode {
    inner: Termios,
}

impl RawMode {
    fn enable_raw_mode() -> Self {
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

struct Editor {
    mode: RawMode,
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            mode: RawMode::enable_raw_mode(),
        }
    }
}

fn main() -> io::Result<()> {
    // Initial terminal setup
    let mut e = Editor::new();
    refresh_screen();
    draw_rows();
    goto_start();

    let mut one_buff = [0; 1];

    while let len = io::stdin().read(&mut one_buff) {
        if len.unwrap() != 0 {
            let c = one_buff[0] as char;
            if c == ctrl_key('q') {
                refresh_screen();
                std::mem::drop(e);
                break;
            }

            if c == ctrl_key('x') {
                refresh_screen();
                continue;
            }

            if !c.is_control() {
                print!("I Got: {}\r\n", c);
            } else {
                print!("Code: {}\r\n", c as u8);
            }
        }
    }

    Ok(())
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
    for _ in 0..24 {
        print!("~\r\n");
    }

    io::stdout().flush();
}
