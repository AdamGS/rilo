use std::io::{self, BufRead, Error, Read, Write};
use std::os::unix::prelude::*;
use termios::*;

fn ctrl_key(c: char) -> char {
    (c as u8 & 0x1f).into()
}

fn main() -> io::Result<()> {
    // Initial terminal setup
    let initial_state = enable_raw_mode();
    refresh_screen();

    loop {
        for b in io::stdin()
            .lock()
            .bytes()
            .map(|x| x.and_then(|y| Ok(y as char)))
        {
            match b {
                Ok(c) => {
                    if c == ctrl_key('q') {
                        refresh_screen();
                        termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &initial_state);
                        std::process::exit(0);
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
                Err(_) => {
                    eprintln!("Error reading key press");
                    std::process::exit(1);
                }
            }
        }
    }

    termios::tcsetattr(io::stdin().as_raw_fd(), TCSAFLUSH, &initial_state);
    Ok(())
}

fn enable_raw_mode() -> Termios {
    let fd = io::stdin().as_raw_fd();
    let mut term = Termios::from_fd(fd).unwrap();
    let mut initial = term.clone();

    term.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
    term.c_oflag &= !(OPOST);
    term.c_cflag |= (CS8);
    term.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
    term.c_cc[VMIN] = 0;
    term.c_cc[VTIME] = 1;

    termios::tcsetattr(fd, TCSAFLUSH, &term);
    initial
}

fn refresh_screen() {
    print!("\x1B[2J");
    print!("\x1b[H");
    io::stdout().flush();
}
