//! Unix PTY process spawning (`src/nvim/os/pty_proc_unix.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::eval::typval::tv_dict_to_env;
#[allow(unused_imports)]
use crate::event::r#loop::{uv_handle_get_loop, uv_strerror};
use crate::log::logmsg;
#[allow(unused_imports)]
use crate::os::fs::os_set_cloexec;
#[allow(unused_imports)]
use crate::strings::strerror;
use std::ffi::{c_char, c_int, c_ulong, c_ushort, c_void};
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct winsize {
    pub ws_row: c_ushort,
    pub ws_col: c_ushort,
    pub ws_xpixel: c_ushort,
    pub ws_ypixel: c_ushort,
}

#[inline]
pub const fn wifstopped(stat: c_int) -> bool {
    (stat & 0xff) == 0x7f
}

#[inline]
pub const fn wifcontinued(stat: c_int) -> bool {
    stat == 0xffff
}

#[inline]
pub const fn wifexited(stat: c_int) -> bool {
    (stat & 0x7f) == 0
}

#[inline]
pub const fn wexitstatus(stat: c_int) -> c_int {
    (stat >> 8) & 0xff
}

#[inline]
pub const fn wifsignaled(stat: c_int) -> bool {
    (((stat & 0x7f) + 1) as i8 >> 1) > 0
}

#[inline]
pub const fn wtermsig(stat: c_int) -> c_int {
    stat & 0x7f
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use crate::eval::decode::dict_T;
    use crate::event::r#loop::{Loop, loop_children_watcher, loop_queues};
    use crate::event::proc::{
        Proc, PtyProc, proc_err, proc_header, proc_in, proc_out, proc_size, proc_tail,
    };
    use crate::event::stream::{stream_closed_ptr, stream_uv_ptr, uv_loop_t, uv_pipe_t};

    type uv_signal_t = c_void;
    type uv_handle_t = c_void;
    type uv_signal_cb = Option<unsafe extern "C" fn(handle: *mut uv_signal_t, signum: c_int)>;

    #[cfg(target_os = "linux")]
    type tcflag_t = u32;
    #[cfg(not(target_os = "linux"))]
    type tcflag_t = c_ulong;

    #[cfg(target_os = "linux")]
    type speed_t = u32;
    #[cfg(not(target_os = "linux"))]
    type speed_t = c_ulong;

    const LOGLVL_ERR: c_int = 4;

    const SIGHUP: c_int = 1;
    const SIGINT: c_int = 2;
    const SIGQUIT: c_int = 3;
    const SIGKILL: c_int = 9;
    const SIGALRM: c_int = 14;
    const SIGTERM: c_int = 15;
    #[cfg(target_os = "linux")]
    const SIGCHLD: c_int = 17;
    #[cfg(not(target_os = "linux"))]
    const SIGCHLD: c_int = 20;
    #[cfg(target_os = "linux")]
    const SIGCONT: c_int = 18;
    #[cfg(not(target_os = "linux"))]
    const SIGCONT: c_int = 19;

    const SIG_DFL: usize = 0;

    const F_GETFL: c_int = 3;
    const F_SETFL: c_int = 4;
    #[cfg(target_os = "linux")]
    const O_NONBLOCK: c_int = 0o4000;
    #[cfg(not(target_os = "linux"))]
    const O_NONBLOCK: c_int = 0x0004;

    const EINTR: c_int = 4;
    const POLLIN: i16 = 1;

    const WNOHANG: c_int = 1;
    const WUNTRACED: c_int = 2;
    #[cfg(target_os = "linux")]
    const WCONTINUED: c_int = 8;
    #[cfg(not(target_os = "linux"))]
    const WCONTINUED: c_int = 0x10;

    #[cfg(target_os = "linux")]
    const TIOCSWINSZ: c_ulong = 0x5414;
    #[cfg(not(target_os = "linux"))]
    const TIOCSWINSZ: c_ulong = 0x8008_7467;

    #[cfg(target_os = "linux")]
    mod termios_consts {
        use super::tcflag_t;
        pub const NCCS: usize = 32;
        pub const ICRNL: tcflag_t = 0o000400;
        pub const IXON: tcflag_t = 0o002000;
        pub const IUTF8: tcflag_t = 0o040000;
        pub const OPOST: tcflag_t = 0o000001;
        pub const ONLCR: tcflag_t = 0o000004;
        pub const CS8: tcflag_t = 0o000060;
        pub const CREAD: tcflag_t = 0o000200;
        pub const ISIG: tcflag_t = 0o000001;
        pub const ICANON: tcflag_t = 0o000002;
        pub const ECHO: tcflag_t = 0o000010;
        pub const ECHOE: tcflag_t = 0o000020;
        pub const ECHOK: tcflag_t = 0o000040;
        pub const ECHOCTL: tcflag_t = 0o001000;
        pub const ECHOKE: tcflag_t = 0o004000;
        pub const IEXTEN: tcflag_t = 0o100000;

        pub const VINTR: usize = 0;
        pub const VQUIT: usize = 1;
        pub const VERASE: usize = 2;
        pub const VKILL: usize = 3;
        pub const VEOF: usize = 4;
        pub const VTIME: usize = 5;
        pub const VMIN: usize = 6;
        pub const VSTART: usize = 8;
        pub const VSTOP: usize = 9;
        pub const VSUSP: usize = 10;
        pub const VEOL: usize = 11;
        pub const VREPRINT: usize = 12;
        pub const VWERASE: usize = 14;
        pub const VLNEXT: usize = 15;
        pub const VEOL2: usize = 16;
        pub const POSIX_VDISABLE: u8 = 0;
    }

    #[cfg(not(target_os = "linux"))]
    mod termios_consts {
        use super::tcflag_t;
        pub const NCCS: usize = 20;
        pub const ICRNL: tcflag_t = 0x0000_0100;
        pub const IXON: tcflag_t = 0x0000_0200;
        pub const IUTF8: tcflag_t = 0x0000_4000;
        pub const OPOST: tcflag_t = 0x0000_0001;
        pub const ONLCR: tcflag_t = 0x0000_0002;
        pub const CS8: tcflag_t = 0x0000_0300;
        pub const CREAD: tcflag_t = 0x0000_0800;
        pub const ISIG: tcflag_t = 0x0000_0080;
        pub const ICANON: tcflag_t = 0x0000_0100;
        pub const ECHO: tcflag_t = 0x0000_0008;
        pub const ECHOE: tcflag_t = 0x0000_0002;
        pub const ECHOK: tcflag_t = 0x0000_0004;
        pub const ECHOCTL: tcflag_t = 0x0000_0040;
        pub const ECHOKE: tcflag_t = 0x0000_0001;
        pub const IEXTEN: tcflag_t = 0x0000_0400;

        pub const VEOF: usize = 0;
        pub const VEOL: usize = 1;
        pub const VEOL2: usize = 2;
        pub const VERASE: usize = 3;
        pub const VWERASE: usize = 4;
        pub const VKILL: usize = 5;
        pub const VREPRINT: usize = 6;
        pub const VINTR: usize = 8;
        pub const VQUIT: usize = 9;
        pub const VSUSP: usize = 10;
        pub const VSTART: usize = 12;
        pub const VSTOP: usize = 13;
        pub const VLNEXT: usize = 14;
        pub const VMIN: usize = 16;
        pub const VTIME: usize = 17;
        pub const POSIX_VDISABLE: u8 = 0xff;
    }

    use termios_consts::*;

    #[repr(C)]
    pub struct termios {
        pub c_iflag: tcflag_t,
        pub c_oflag: tcflag_t,
        pub c_cflag: tcflag_t,
        pub c_lflag: tcflag_t,
        #[cfg(target_os = "linux")]
        pub c_line: u8,
        pub c_cc: [u8; NCCS],
        pub c_ispeed: speed_t,
        pub c_ospeed: speed_t,
    }

    impl termios {
        const ZERO: Self = Self {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: 0,
            #[cfg(target_os = "linux")]
            c_line: 0,
            c_cc: [0; NCCS],
            c_ispeed: 0,
            c_ospeed: 0,
        };
    }

    #[repr(C)]
    struct PtyProcUnixTail {
        width: u16,
        height: u16,
        winsize: winsize,
        tty_fd: c_int,
    }

    #[repr(C)]
    struct pollfd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    static mut termios_default: termios = termios::ZERO;

    use crate::event::r#loop::{uv_chdir, uv_pipe_open, uv_signal_start, uv_signal_stop};
    use crate::fileio::{close, dup, fcntl, kill};
    #[cfg(all(not(test), target_os = "macos"))]
    use crate::os::env::_NSGetEnviron;
    #[cfg(all(not(test), not(target_os = "macos")))]
    use crate::os::env::environ;
    use crate::os::signal::signal;

    unsafe extern "C" {
        fn uv_disable_stdio_inheritance();

        fn forkpty(
            amaster: *mut c_int,
            name: *mut c_char,
            termp: *const termios,
            winp: *const winsize,
        ) -> c_int;
        fn ptsname(fd: c_int) -> *mut c_char;
        fn cfsetispeed(termios_p: *mut termios, speed: speed_t) -> c_int;
        fn cfsetospeed(termios_p: *mut termios, speed: speed_t) -> c_int;
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
        fn killpg(pgrp: c_int, sig: c_int) -> c_int;
        fn waitpid(pid: c_int, wstatus: *mut c_int, options: c_int) -> c_int;
        fn setsid() -> c_int;
        fn execvp(file: *const c_char, argv: *const *mut c_char) -> c_int;
        fn _exit(status: c_int) -> !;
        #[cfg(target_os = "linux")]
        fn poll(fds: *mut pollfd, nfds: c_ulong, timeout: c_int) -> c_int;
    }

    #[inline]
    unsafe fn get_errno() -> c_int {
        unsafe { crate::os::fs::os_errno() }
    }

    #[inline]
    unsafe fn pty_proc_unix_tail(ptyproc: *mut PtyProc) -> *mut PtyProcUnixTail {
        unsafe {
            ptyproc
                .cast::<u8>()
                .add(proc_size())
                .cast::<PtyProcUnixTail>()
        }
    }

    #[inline]
    unsafe fn proc_get_exepath(proc_: *mut Proc) -> *const c_char {
        unsafe {
            let hdr = proc_header(proc_);
            if !(*hdr).exepath.is_null() {
                (*hdr).exepath
            } else {
                *(*hdr).argv
            }
        }
    }

    /// @returns zero on success, or negative error code
    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_spawn(ptyproc: *mut PtyProc) -> c_int {
        unsafe {
            if termios_default.c_cflag == 0 {
                init_termios(&raw mut termios_default);
            }

            let proc_: *mut Proc = ptyproc.cast();
            let hdr = proc_header(proc_);
            let ptail = pty_proc_unix_tail(ptyproc);
            let s_in = proc_in(proc_);
            let r_out = proc_out(proc_);
            let r_err = proc_err(proc_);

            debug_assert!(*stream_closed_ptr(r_err.cast()));
            uv_signal_start(
                loop_children_watcher((*hdr).loop_),
                Some(chld_handler),
                SIGCHLD,
            );
            (*ptail).winsize = winsize {
                ws_row: (*ptail).height,
                ws_col: (*ptail).width,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            uv_disable_stdio_inheritance();
            let mut master: c_int = -1;
            let pid = forkpty(
                &raw mut master,
                ptr::null_mut(),
                &raw const termios_default,
                &raw const (*ptail).winsize,
            );

            if pid < 0 {
                let err = get_errno();
                let status = -err;
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"pty_proc_spawn".as_ptr(),
                    line!() as c_int,
                    true,
                    c"forkpty failed: %s".as_ptr(),
                    strerror(err),
                );
                return status;
            } else if pid == 0 {
                init_child(ptyproc);
            }

            let mut status: c_int;
            'error: {
                let master_status_flags = fcntl(master, F_GETFL);
                if master_status_flags == -1 {
                    let err = get_errno();
                    status = -err;
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"pty_proc_spawn".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Failed to get master descriptor status flags: %s".as_ptr(),
                        strerror(err),
                    );
                    break 'error;
                }
                if fcntl(master, F_SETFL, master_status_flags | O_NONBLOCK) == -1 {
                    let err = get_errno();
                    status = -err;
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"pty_proc_spawn".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Failed to make master descriptor non-blocking: %s".as_ptr(),
                        strerror(err),
                    );
                    break 'error;
                }

                if os_set_cloexec(master) == -1 {
                    status = -get_errno();
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"pty_proc_spawn".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Failed to set CLOEXEC on ptmx file descriptor".as_ptr(),
                    );
                    break 'error;
                }

                if !*stream_closed_ptr(s_in) {
                    status = set_duplicating_descriptor(master, stream_uv_ptr(s_in).cast());
                    if status != 0 {
                        break 'error;
                    }
                }
                if !*stream_closed_ptr(r_out.cast()) {
                    status = set_duplicating_descriptor(master, stream_uv_ptr(r_out.cast()).cast());
                    if status != 0 {
                        break 'error;
                    }
                }

                (*ptail).tty_fd = master;
                (*hdr).pid = pid;
                return 0;
            }

            close(master);
            kill(pid, SIGKILL);
            waitpid(pid, ptr::null_mut(), 0);
            status
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_tty_name(ptyproc: *mut PtyProc) -> *const c_char {
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            ptsname((*ptail).tty_fd)
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_resize(ptyproc: *mut PtyProc, width: u16, height: u16) {
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            (*ptail).winsize = winsize {
                ws_row: height,
                ws_col: width,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            ioctl((*ptail).tty_fd, TIOCSWINSZ, &raw const (*ptail).winsize);
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_resume(ptyproc: *mut PtyProc) {
        unsafe {
            let hdr = proc_header(ptyproc.cast());
            killpg((*hdr).pid, SIGCONT);
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_flush_master(ptyproc: *mut PtyProc) {
        #[cfg(target_os = "linux")]
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            let mut pfd = pollfd {
                fd: (*ptail).tty_fd,
                events: POLLIN,
                revents: 0,
            };
            loop {
                let n = poll(&raw mut pfd, 1, 0);
                if !(n < 0 && get_errno() == EINTR) {
                    break;
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = ptyproc;
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_close(ptyproc: *mut PtyProc) {
        unsafe {
            pty_proc_close_master(ptyproc);
            let proc_: *mut Proc = ptyproc.cast();
            let proc_t = proc_tail(proc_);
            if let Some(cb) = (*proc_t).internal_close_cb {
                cb(proc_);
            }
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_close_master(ptyproc: *mut PtyProc) {
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            if (*ptail).tty_fd >= 0 {
                close((*ptail).tty_fd);
                (*ptail).tty_fd = -1;
            }
        }
    }

    #[cfg_attr(not(test), unsafe(no_mangle))]
    pub unsafe extern "C" fn pty_proc_teardown(loop_: *mut Loop) {
        unsafe {
            uv_signal_stop(loop_children_watcher(loop_));
        }
    }

    unsafe fn init_child(ptyproc: *mut PtyProc) -> ! {
        unsafe {
            setsid();

            signal(SIGCHLD, SIG_DFL);
            signal(SIGHUP, SIG_DFL);
            signal(SIGINT, SIG_DFL);
            signal(SIGQUIT, SIG_DFL);
            signal(SIGTERM, SIG_DFL);
            signal(SIGALRM, SIG_DFL);

            let proc_: *mut Proc = ptyproc.cast();
            let hdr = proc_header(proc_);
            if !(*hdr).cwd.is_null() {
                let err = uv_chdir((*hdr).cwd);
                if err != 0 {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"init_child".as_ptr(),
                        line!() as c_int,
                        true,
                        c"chdir(%s) failed: %s".as_ptr(),
                        (*hdr).cwd,
                        uv_strerror(err),
                    );
                    _exit(122);
                }
            }

            let prog = proc_get_exepath(proc_);
            debug_assert!(!(*hdr).env.is_null());
            let new_env = tv_dict_to_env((*hdr).env);
            #[cfg(target_os = "macos")]
            {
                *_NSGetEnviron() = new_env;
            }
            #[cfg(not(target_os = "macos"))]
            {
                environ = new_env;
            }
            execvp(prog, (*hdr).argv.cast_const());
            let err = get_errno();
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"init_child".as_ptr(),
                line!() as c_int,
                true,
                c"execvp(%s) failed: %s".as_ptr(),
                prog,
                strerror(err),
            );
            _exit(122);
        }
    }

    unsafe fn init_termios(t: *mut termios) {
        unsafe {
            (*t).c_iflag = ICRNL | IXON | IUTF8;
            (*t).c_oflag = OPOST | ONLCR;
            (*t).c_cflag = CS8 | CREAD;
            (*t).c_lflag = ISIG | ICANON | IEXTEN | ECHO | ECHOE | ECHOK | ECHOCTL | ECHOKE;

            cfsetispeed(t, 38400);
            cfsetospeed(t, 38400);

            (*t).c_cc[VINTR] = 0x1f & b'C';
            (*t).c_cc[VQUIT] = 0x1f & b'\\';
            (*t).c_cc[VERASE] = 0x7f;
            (*t).c_cc[VKILL] = 0x1f & b'U';
            (*t).c_cc[VEOF] = 0x1f & b'D';
            (*t).c_cc[VEOL] = POSIX_VDISABLE;
            (*t).c_cc[VEOL2] = POSIX_VDISABLE;
            (*t).c_cc[VSTART] = 0x1f & b'Q';
            (*t).c_cc[VSTOP] = 0x1f & b'S';
            (*t).c_cc[VSUSP] = 0x1f & b'Z';
            (*t).c_cc[VREPRINT] = 0x1f & b'R';
            (*t).c_cc[VWERASE] = 0x1f & b'W';
            (*t).c_cc[VLNEXT] = 0x1f & b'V';
            (*t).c_cc[VMIN] = 1;
            (*t).c_cc[VTIME] = 0;
        }
    }

    unsafe fn set_duplicating_descriptor(fd: c_int, pipe: *mut uv_pipe_t) -> c_int {
        unsafe {
            let fd_dup = dup(fd);
            if fd_dup < 0 {
                let err = get_errno();
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"set_duplicating_descriptor".as_ptr(),
                    line!() as c_int,
                    true,
                    c"Failed to dup descriptor %d: %s".as_ptr(),
                    fd,
                    strerror(err),
                );
                return -err;
            }

            let status: c_int;
            'error: {
                if os_set_cloexec(fd_dup) == -1 {
                    status = -get_errno();
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"set_duplicating_descriptor".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Failed to set CLOEXEC on duplicate fd".as_ptr(),
                    );
                    break 'error;
                }

                status = uv_pipe_open(pipe, fd_dup);
                if status != 0 {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"set_duplicating_descriptor".as_ptr(),
                        line!() as c_int,
                        true,
                        c"Failed to set pipe to descriptor %d: %s".as_ptr(),
                        fd_dup,
                        uv_strerror(status),
                    );
                    break 'error;
                }
                return status;
            }

            close(fd_dup);
            status
        }
    }

    unsafe extern "C" fn chld_handler(handle: *mut uv_signal_t, _signum: c_int) {
        unsafe {
            let mut stat: c_int = 0;
            let uv_loop = uv_handle_get_loop(handle.cast());
            let loop_ = *uv_loop.cast::<*mut Loop>();
            let qc = loop_queues(loop_);

            for i in 0..(*qc).children_size {
                let proc_ = *(*qc).children_items.add(i) as *mut Proc;
                let hdr = proc_header(proc_);
                let proc_t = proc_tail(proc_);

                let mut pid: c_int;
                loop {
                    pid = waitpid((*hdr).pid, &raw mut stat, WNOHANG | WUNTRACED | WCONTINUED);
                    if !(pid < 0 && get_errno() == EINTR) {
                        break;
                    }
                }

                if pid <= 0 {
                    continue;
                }

                if wifstopped(stat) {
                    if let Some(state_cb) = (*proc_t).state_cb {
                        state_cb(proc_, true, (*hdr).data);
                    }
                    continue;
                }
                if wifcontinued(stat) {
                    if let Some(state_cb) = (*proc_t).state_cb {
                        state_cb(proc_, false, (*hdr).data);
                    }
                    continue;
                }

                if wifexited(stat) {
                    (*hdr).status = wexitstatus(stat);
                } else if wifsignaled(stat) {
                    (*hdr).status = 128 + wtermsig(stat);
                }
                if let Some(exit_cb) = (*proc_t).internal_exit_cb {
                    exit_cb(proc_);
                }
            }
        }
    }
}

#[cfg(unix)]
pub use unix_impl::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_status_macros_decode_exit_signal_and_stop() {
        // Exited with code 42 -> (42 << 8)
        let exited = 42 << 8;
        assert!(wifexited(exited));
        assert!(!wifsignaled(exited));
        assert!(!wifstopped(exited));
        assert_eq!(wexitstatus(exited), 42);

        // Killed by SIGTERM (15)
        let signaled = 15;
        assert!(!wifexited(signaled));
        assert!(wifsignaled(signaled));
        assert!(!wifstopped(signaled));
        assert_eq!(wtermsig(signaled), 15);

        // Stopped (0x7f | (19 << 8))
        let stopped = 0x7f | (19 << 8);
        assert!(wifstopped(stopped));
        assert!(!wifexited(stopped));
        assert!(!wifsignaled(stopped));

        // Continued (0xffff)
        assert!(wifcontinued(0xffff));
    }
}
