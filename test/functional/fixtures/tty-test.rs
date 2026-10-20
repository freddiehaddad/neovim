//! Raw TTY echo and resize/signal fixture for functional tests (`test/functional/fixtures/tty-test.c`).
//!
//! Puts the terminal into raw mode, prints `"tty ready\n"` to `stderr`, echoes
//! input to `stdout`, reports window resize events (`SIGWINCH`), exits with code
//! `42` on `SIGHUP`, and exits after receiving two `CTRL-C` (`0x03`) bytes.

#![allow(non_camel_case_types, unused_imports)]

use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::process;

const CTRL_C: u8 = 0x03;
const BUF_SIZE: usize = 0xfff;

fn parse_line_count(arg: &str) -> Option<usize> {
    let trimmed = arg.trim_start();
    if trimmed.is_empty() {
        return Some(0);
    }
    let mut end = 0;
    for (idx, ch) in trimmed.char_indices() {
        if (idx == 0 && (ch == '+' || ch == '-')) || ch.is_ascii_digit() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return Some(0);
    }
    match trimmed[..end].parse::<i64>() {
        Ok(v) if (0..=99_999).contains(&v) => Some(v as usize),
        Ok(_) => Some(0),
        Err(_) => None,
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::{c_int, c_ulong, c_ushort};
    use std::thread;
    use std::time::Duration;

    const SIGHUP: c_int = 1;
    const SIGWINCH: c_int = 28;
    const EINTR: i32 = 4;
    const TCSADRAIN: c_int = 1;

    #[cfg(target_os = "linux")]
    type tcflag_t = u32;
    #[cfg(not(target_os = "linux"))]
    type tcflag_t = c_ulong;

    #[cfg(target_os = "linux")]
    type speed_t = u32;
    #[cfg(not(target_os = "linux"))]
    type speed_t = c_ulong;

    #[cfg(target_os = "linux")]
    const NCCS: usize = 32;
    #[cfg(not(target_os = "linux"))]
    const NCCS: usize = 20;

    #[cfg(target_os = "linux")]
    const ONLCR: tcflag_t = 0o000004;
    #[cfg(not(target_os = "linux"))]
    const ONLCR: tcflag_t = 0x0002;

    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: c_ulong = 0x5413;
    #[cfg(not(target_os = "linux"))]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct termios {
        c_iflag: tcflag_t,
        c_oflag: tcflag_t,
        c_cflag: tcflag_t,
        c_lflag: tcflag_t,
        #[cfg(target_os = "linux")]
        c_line: u8,
        c_cc: [u8; NCCS],
        c_ispeed: speed_t,
        c_ospeed: speed_t,
    }

    #[repr(C)]
    struct winsize {
        ws_row: c_ushort,
        ws_col: c_ushort,
        ws_xpixel: c_ushort,
        ws_ypixel: c_ushort,
    }

    unsafe extern "C" {
        fn getsid(pid: c_int) -> c_int;
        fn getpid() -> c_int;
        fn tcgetattr(fd: c_int, termios_p: *mut termios) -> c_int;
        fn tcsetattr(fd: c_int, optional_actions: c_int, termios_p: *const termios) -> c_int;
        fn cfmakeraw(termios_p: *mut termios);
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
        fn signal(signum: c_int, handler: usize) -> usize;
        fn read(fd: c_int, buf: *mut std::ffi::c_void, count: usize) -> isize;
        fn _exit(status: c_int) -> !;
    }

    pub fn owns_tty() -> bool {
        unsafe { getsid(0) == getpid() }
    }

    unsafe extern "C" fn sig_handler(signum: c_int) {
        unsafe {
            match signum {
                SIGWINCH => {
                    let mut ws = winsize {
                        ws_row: 0,
                        ws_col: 0,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    if ioctl(2, TIOCGWINSZ, &raw mut ws) == 0 {
                        let msg = format!("rows: {}, cols: {}\n", ws.ws_row, ws.ws_col);
                        let _ = io::stderr().write_all(msg.as_bytes());
                        let _ = io::stderr().flush();
                    }
                }
                SIGHUP => {
                    _exit(42);
                }
                _ => {}
            }
        }
    }

    pub fn run_interactive() {
        unsafe {
            let mut orig: termios = std::mem::zeroed();
            if tcgetattr(2, &raw mut orig) == 0 {
                let mut raw = orig;
                let orig_oflag = raw.c_oflag;
                cfmakeraw(&raw mut raw);
                raw.c_oflag = orig_oflag | ONLCR;
                let _ = tcsetattr(2, TCSADRAIN, &raw const raw);
            }

            signal(SIGHUP, sig_handler as *const () as usize);
            signal(SIGWINCH, sig_handler as *const () as usize);

            eprintln!("tty ready");
            let _ = io::stderr().flush();

            let mut interrupted = 0usize;
            let mut buf = [0u8; BUF_SIZE];
            loop {
                let n = read(2, buf.as_mut_ptr().cast(), BUF_SIZE);
                if n < 0 {
                    if io::Error::last_os_error().raw_os_error() == Some(EINTR) {
                        continue;
                    }
                    break;
                }
                if n == 0 {
                    break;
                }
                let chunk = &buf[..(n as usize)];
                for &b in chunk {
                    if b == CTRL_C {
                        interrupted += 1;
                    }
                }
                let _ = io::stdout().write_all(chunk);
                let _ = io::stdout().flush();

                if interrupted >= 2 {
                    return;
                } else if interrupted == 1 {
                    eprintln!("interrupt received, press again to exit");
                    let _ = io::stderr().flush();
                }
            }

            // Wait so SIGHUP handler can be delivered on master PTY close.
            thread::sleep(Duration::from_secs(100));
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;

    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

    unsafe extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> i32;
    }

    pub fn owns_tty() -> bool {
        true
    }

    pub fn run_interactive() {
        unsafe {
            let h_in = GetStdHandle(STD_INPUT_HANDLE);
            let h_out = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut orig_in_mode: u32 = 0;
            let mut orig_out_mode: u32 = 0;
            if GetConsoleMode(h_in, &raw mut orig_in_mode) != 0 {
                let _ = SetConsoleMode(h_in, ENABLE_VIRTUAL_TERMINAL_INPUT);
            }
            if GetConsoleMode(h_out, &raw mut orig_out_mode) != 0 {
                let _ = SetConsoleMode(h_out, orig_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }

            eprintln!("tty ready");
            let _ = io::stderr().flush();

            let mut interrupted = 0usize;
            let mut buf = [0u8; BUF_SIZE];
            let mut stdin = io::stdin();
            while let Ok(n) = stdin.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                for &b in chunk {
                    if b == CTRL_C {
                        interrupted += 1;
                    }
                }
                let _ = io::stdout().write_all(chunk);
                let _ = io::stdout().flush();

                if interrupted >= 2 {
                    let _ = SetConsoleMode(h_in, orig_in_mode);
                    break;
                } else if interrupted == 1 {
                    eprintln!("interrupt received, press again to exit");
                    let _ = io::stderr().flush();
                }
            }
        }
    }
}

fn main() {
    if !platform::owns_tty() {
        eprintln!("process does not own the terminal");
        process::exit(2);
    }
    if !io::stdin().is_terminal() {
        eprintln!("stdin is not a terminal");
        process::exit(2);
    }
    if !io::stdout().is_terminal() {
        eprintln!("stdout is not a terminal");
        process::exit(2);
    }
    if !io::stderr().is_terminal() {
        eprintln!("stderr is not a terminal");
        process::exit(2);
    }

    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let Some(count) = parse_line_count(&args[1]) else {
            process::abort();
        };
        let mut out = io::stdout().lock();
        for i in 0..count {
            let _ = writeln!(out, "line{i}");
        }
        let _ = out.flush();
        return;
    }

    platform::run_interactive();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_count_clamps_out_of_range_and_parses_valid_counts() {
        assert_eq!(parse_line_count("10"), Some(10));
        assert_eq!(parse_line_count("-5"), Some(0));
        assert_eq!(parse_line_count("100000"), Some(0));
    }
}
