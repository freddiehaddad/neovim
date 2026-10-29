//! Raw TTY echo and resize/signal fixture for functional tests (`test/functional/fixtures/tty-test.c`).
//!
//! Puts the terminal into raw mode, prints `"tty ready\n"` to `stderr`, echoes
//! input to `stdout`, reports window resize events (`SIGWINCH`), exits with code
//! `42` on `SIGHUP`, and exits after receiving two `CTRL-C` (`0x03`) bytes.

use std::env;
use std::io::{self, IsTerminal, Write};
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
    #[allow(non_camel_case_types)]
    type tcflag_t = u32;
    #[cfg(not(target_os = "linux"))]
    #[allow(non_camel_case_types)]
    type tcflag_t = c_ulong;

    #[cfg(target_os = "linux")]
    #[allow(non_camel_case_types)]
    type speed_t = u32;
    #[cfg(not(target_os = "linux"))]
    #[allow(non_camel_case_types)]
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
    use std::ffi::{c_int, c_ulong, c_void};
    use std::ptr;

    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const UV_TTY_MODE_NORMAL: c_int = 0;
    const UV_TTY_MODE_RAW: c_int = 1;
    const UV_RUN_DEFAULT: c_int = 0;
    const SIGWINCH: c_int = 28;

    #[repr(C)]
    struct uv_buf_t {
        len: c_ulong,
        base: *mut u8,
    }

    #[repr(C, align(8))]
    struct AlignedBuf<const N: usize>([u8; N]);

    static mut TTY_IN: AlignedBuf<512> = AlignedBuf([0; 512]);
    static mut TTY_OUT: AlignedBuf<512> = AlignedBuf([0; 512]);
    static mut INTERRUPTED: c_int = 0;

    unsafe extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> *mut c_void;
        fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> i32;
    }

    #[link(name = "libuv", kind = "static")]
    #[link(name = "advapi32")]
    #[link(name = "iphlpapi")]
    #[link(name = "ole32")]
    #[link(name = "psapi")]
    #[link(name = "shell32")]
    #[link(name = "user32")]
    #[link(name = "userenv")]
    #[link(name = "ws2_32")]
    unsafe extern "C" {
        fn uv_default_loop() -> *mut c_void;
        fn uv_loop_init(loop_: *mut c_void) -> c_int;
        fn uv_loop_close(loop_: *mut c_void) -> c_int;
        fn uv_prepare_init(loop_: *mut c_void, prepare: *mut c_void) -> c_int;
        fn uv_prepare_start(
            prepare: *mut c_void,
            cb: unsafe extern "C" fn(*mut c_void),
        ) -> c_int;
        fn uv_prepare_stop(prepare: *mut c_void) -> c_int;
        fn uv_tty_init(
            loop_: *mut c_void,
            tty: *mut c_void,
            fd: c_int,
            readable: c_int,
        ) -> c_int;
        fn uv_tty_get_winsize(
            tty: *mut c_void,
            width: *mut c_int,
            height: *mut c_int,
        ) -> c_int;
        fn uv_tty_set_mode(tty: *mut c_void, mode: c_int) -> c_int;
        fn uv_read_start(
            stream: *mut c_void,
            alloc_cb: unsafe extern "C" fn(*mut c_void, usize, *mut uv_buf_t),
            read_cb: unsafe extern "C" fn(*mut c_void, isize, *const uv_buf_t),
        ) -> c_int;
        fn uv_read_stop(stream: *mut c_void) -> c_int;
        fn uv_write(
            req: *mut c_void,
            handle: *mut c_void,
            bufs: *const uv_buf_t,
            nbufs: u32,
            cb: Option<unsafe extern "C" fn(*mut c_void, c_int)>,
        ) -> c_int;
        fn uv_signal_init(loop_: *mut c_void, handle: *mut c_void) -> c_int;
        fn uv_signal_start(
            handle: *mut c_void,
            cb: unsafe extern "C" fn(*mut c_void, c_int),
            signum: c_int,
        ) -> c_int;
        fn uv_walk(
            loop_: *mut c_void,
            walk_cb: unsafe extern "C" fn(*mut c_void, *mut c_void),
            arg: *mut c_void,
        );
        fn uv_is_closing(handle: *mut c_void) -> c_int;
        fn uv_close(
            handle: *mut c_void,
            close_cb: Option<unsafe extern "C" fn(*mut c_void)>,
        );
        fn uv_run(loop_: *mut c_void, mode: c_int) -> c_int;
    }

    pub fn owns_tty() -> bool {
        true
    }

    unsafe extern "C" fn walk_cb(handle: *mut c_void, _arg: *mut c_void) {
        unsafe {
            if uv_is_closing(handle) == 0 {
                uv_tty_set_mode((&raw mut TTY_IN).cast(), UV_TTY_MODE_NORMAL);
                uv_close(handle, None);
            }
        }
    }

    unsafe extern "C" fn sigwinch_cb(_handle: *mut c_void, _signum: c_int) {
        unsafe {
            let mut width: c_int = 0;
            let mut height: c_int = 0;
            uv_tty_get_winsize((&raw mut TTY_OUT).cast(), &raw mut width, &raw mut height);
            let msg = format!("rows: {height}, cols: {width}\n");
            let _ = io::stderr().write_all(msg.as_bytes());
            let _ = io::stderr().flush();
        }
    }

    unsafe extern "C" fn alloc_cb(
        _handle: *mut c_void,
        _suggested: usize,
        buf: *mut uv_buf_t,
    ) {
        unsafe {
            let mut v = vec![0u8; BUF_SIZE];
            let ptr = v.as_mut_ptr();
            std::mem::forget(v);
            (*buf).len = BUF_SIZE as c_ulong;
            (*buf).base = ptr;
        }
    }

    unsafe extern "C" fn read_cb(
        stream: *mut c_void,
        cnt: isize,
        buf: *const uv_buf_t,
    ) {
        unsafe {
            if cnt <= 0 {
                if !(*buf).base.is_null() {
                    drop(Vec::from_raw_parts((*buf).base, 0, BUF_SIZE));
                }
                uv_read_stop(stream);
                return;
            }
            let n = cnt as usize;
            let slice = std::slice::from_raw_parts((*buf).base, n);
            for &b in slice {
                if b == CTRL_C {
                    INTERRUPTED += 1;
                }
            }

            let mut write_loop = AlignedBuf::<1024>([0; 1024]);
            let mut out_tty = AlignedBuf::<512>([0; 512]);
            let mut req = AlignedBuf::<512>([0; 512]);
            uv_loop_init((&raw mut write_loop).cast());
            uv_tty_init((&raw mut write_loop).cast(), (&raw mut out_tty).cast(), 1, 0);
            let b = uv_buf_t {
                len: cnt as c_ulong,
                base: (*buf).base,
            };
            uv_write(
                (&raw mut req).cast(),
                (&raw mut out_tty).cast(),
                &raw const b,
                1,
                None,
            );
            uv_run((&raw mut write_loop).cast(), UV_RUN_DEFAULT);
            uv_close((&raw mut out_tty).cast(), None);
            uv_run((&raw mut write_loop).cast(), UV_RUN_DEFAULT);
            if uv_loop_close((&raw mut write_loop).cast()) != 0 {
                process::abort();
            }
            drop(Vec::from_raw_parts((*buf).base, 0, BUF_SIZE));

            if INTERRUPTED >= 2 {
                uv_walk(uv_default_loop(), walk_cb, ptr::null_mut());
            } else if INTERRUPTED == 1 {
                let _ = io::stderr().write_all(b"interrupt received, press again to exit\n");
                let _ = io::stderr().flush();
            }
        }
    }

    unsafe extern "C" fn prepare_cb(handle: *mut c_void) {
        unsafe {
            let _ = io::stderr().write_all(b"tty ready\n");
            let _ = io::stderr().flush();
            uv_prepare_stop(handle);
        }
    }

    pub fn run_interactive() {
        unsafe {
            INTERRUPTED = 0;
            let loop_ = uv_default_loop();
            let mut prepare = AlignedBuf::<256>([0; 256]);
            uv_prepare_init(loop_, (&raw mut prepare).cast());
            uv_prepare_start((&raw mut prepare).cast(), prepare_cb);

            uv_tty_init(loop_, (&raw mut TTY_IN).cast(), 0, 1);
            uv_tty_init(loop_, (&raw mut TTY_OUT).cast(), 1, 0);
            let mut width: c_int = 0;
            let mut height: c_int = 0;
            uv_tty_get_winsize((&raw mut TTY_OUT).cast(), &raw mut width, &raw mut height);
            uv_tty_set_mode((&raw mut TTY_IN).cast(), UV_TTY_MODE_RAW);

            let h_in = GetStdHandle(STD_INPUT_HANDLE);
            let h_out = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut dw_mode: u32 = 0;
            if GetConsoleMode(h_in, &raw mut dw_mode) != 0 {
                let _ = SetConsoleMode(h_in, dw_mode | ENABLE_VIRTUAL_TERMINAL_INPUT);
            }
            if GetConsoleMode(h_out, &raw mut dw_mode) != 0 {
                let _ = SetConsoleMode(h_out, dw_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }

            uv_read_start((&raw mut TTY_IN).cast(), alloc_cb, read_cb);
            let mut sigwinch_watcher = AlignedBuf::<256>([0; 256]);
            uv_signal_init(loop_, (&raw mut sigwinch_watcher).cast());
            uv_signal_start((&raw mut sigwinch_watcher).cast(), sigwinch_cb, SIGWINCH);
            uv_run(loop_, UV_RUN_DEFAULT);
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
