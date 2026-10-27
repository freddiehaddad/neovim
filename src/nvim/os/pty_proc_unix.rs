//! Unix PTY process spawning (`src/nvim/os/pty_proc_unix.c`).

#[cfg(unix)]
use crate::eval::typval::tv_dict_to_env;
#[cfg(unix)]
use crate::event::libuv_proc::proc_get_exepath;
#[cfg(unix)]
use crate::event::r#loop::{uv_disable_stdio_inheritance, uv_handle_get_loop, uv_strerror};
#[cfg(unix)]
use crate::log::logmsg;
#[cfg(unix)]
use crate::os::fs::{os_errno, os_set_cloexec};
#[cfg(unix)]
use crate::strings::strerror;
#[cfg(any(unix, test))]
use std::ffi::c_int;
#[cfg(unix)]
use std::ffi::{c_char, c_ulong, c_ushort};
#[cfg(unix)]
use std::ptr;

#[cfg(unix)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct winsize {
    pub ws_row: c_ushort,
    pub ws_col: c_ushort,
    pub ws_xpixel: c_ushort,
    pub ws_ypixel: c_ushort,
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wifstopped(stat: c_int) -> bool {
    (stat & 0xff) == 0x7f
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wifcontinued(stat: c_int) -> bool {
    stat == 0xffff
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wifexited(stat: c_int) -> bool {
    (stat & 0x7f) == 0
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wexitstatus(stat: c_int) -> c_int {
    (stat >> 8) & 0xff
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wifsignaled(stat: c_int) -> bool {
    (((stat & 0x7f) + 1) as i8 >> 1) > 0
}

#[cfg(any(unix, test))]
#[inline]
pub const fn wtermsig(stat: c_int) -> c_int {
    stat & 0x7f
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use crate::event::r#loop::{Loop, loop_children_watcher, loop_queues, uv_signal_t};
    use crate::event::proc::{Proc, PtyProc};
    use crate::event::stream::uv_pipe_t;

    #[cfg(target_os = "linux")]
    type tcflag_t = u32;
    #[cfg(not(target_os = "linux"))]
    type tcflag_t = c_ulong;

    #[cfg(target_os = "linux")]
    type speed_t = u32;
    #[cfg(not(target_os = "linux"))]
    type speed_t = c_ulong;

    use crate::fileio::EINTR;
    use crate::log::LOGLVL_ERR;
    use crate::os::signal::{SIGHUP, SIGINT, SIGKILL, SIGQUIT, SIGTERM};

    const SIGALRM: c_int = 14;
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

    pub use termios_consts::*;

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

        pub const fn new() -> Self {
            Self::ZERO
        }
    }

    impl Default for termios {
        fn default() -> Self {
            Self::ZERO
        }
    }

    #[repr(C)]
    struct pollfd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    static TERMIOS_DEFAULT: crate::globals::GlobalState<termios> =
        crate::globals::GlobalState::new(termios::ZERO);

    use crate::event::r#loop::{uv_chdir, uv_pipe_open, uv_signal_start, uv_signal_stop};
    use crate::fileio::{close, dup, fcntl, kill};
    use crate::os::env::os_environ_ptr;
    use crate::os::signal::signal;

    unsafe extern "C" {
        fn forkpty(
            amaster: *mut c_int,
            name: *mut c_char,
            termp: *const termios,
            winp: *const winsize,
        ) -> c_int;
        fn ptsname(fd: c_int) -> *mut c_char;
        fn cfsetispeed(termios_p: *mut termios, speed: speed_t) -> c_int;
        fn cfsetospeed(termios_p: *mut termios, speed: speed_t) -> c_int;
        pub fn tcgetattr(fd: c_int, termios_p: *mut termios) -> c_int;
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
    unsafe fn pty_proc_unix_tail(ptyproc: *mut PtyProc) -> *mut PtyProc {
        ptyproc
    }

    /// @returns zero on success, or negative error code
    pub unsafe fn pty_proc_spawn(ptyproc: *mut PtyProc) -> c_int {
        unsafe {
            let termios_default_ptr = TERMIOS_DEFAULT.as_ptr();
            if (*termios_default_ptr).c_cflag == 0 {
                init_termios(termios_default_ptr);
            }

            let proc_: *mut Proc = ptyproc;
            let hdr = proc_;
            let ptail = pty_proc_unix_tail(ptyproc);
            let s_in = &raw mut (*proc_).r#in;
            let r_out = &raw mut (*proc_).out;
            let r_err = &raw mut (*proc_).err;

            debug_assert!((*r_err).s.closed);
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
            let prog = proc_get_exepath(proc_);
            debug_assert!(!(*hdr).env.is_null());
            let new_env = tv_dict_to_env((*hdr).env);
            let mut new_env_ptrs: Vec<*mut c_char> = new_env
                .iter()
                .map(|s| s.as_ptr().cast_mut())
                .chain(std::iter::once(ptr::null_mut()))
                .collect();
            let argv_ptrs: Vec<*mut c_char> = (*hdr)
                .argv
                .iter()
                .map(|s| s.as_ptr().cast_mut())
                .chain(std::iter::once(ptr::null_mut()))
                .collect();
            let mut master: c_int = -1;
            let pid = forkpty(
                &raw mut master,
                ptr::null_mut(),
                termios_default_ptr.cast_const(),
                &raw const (*ptail).winsize,
            );

            if pid < 0 {
                let err = os_errno();
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
                init_child(ptyproc, prog, argv_ptrs.as_ptr(), new_env_ptrs.as_mut_ptr());
            }
            drop(new_env_ptrs);
            drop(new_env);
            drop(argv_ptrs);

            let mut status: c_int;
            'error: {
                let master_status_flags = fcntl(master, F_GETFL);
                if master_status_flags == -1 {
                    let err = os_errno();
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
                    let err = os_errno();
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
                    status = -os_errno();
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

                if !(*s_in).closed {
                    status = set_duplicating_descriptor(master, (&raw mut (*s_in).uv).cast());
                    if status != 0 {
                        break 'error;
                    }
                }
                if !(*r_out).s.closed {
                    status = set_duplicating_descriptor(master, (&raw mut (*r_out).s.uv).cast());
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

    pub unsafe fn pty_proc_tty_name(ptyproc: *mut PtyProc) -> *const c_char {
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            ptsname((*ptail).tty_fd)
        }
    }

    pub unsafe fn pty_proc_resize(ptyproc: *mut PtyProc, width: u16, height: u16) {
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

    pub unsafe fn pty_proc_resume(ptyproc: *mut PtyProc) {
        unsafe {
            killpg((*ptyproc).pid, SIGCONT);
        }
    }

    pub unsafe fn pty_proc_flush_master(ptyproc: *mut PtyProc) {
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
                if !(n < 0 && os_errno() == EINTR) {
                    break;
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = ptyproc;
    }

    pub unsafe fn pty_proc_close(ptyproc: *mut PtyProc) {
        unsafe {
            pty_proc_close_master(ptyproc);
            let proc_: *mut Proc = ptyproc;
            crate::event::proc::proc_decref(proc_);
        }
    }

    pub unsafe fn pty_proc_close_master(ptyproc: *mut PtyProc) {
        unsafe {
            let ptail = pty_proc_unix_tail(ptyproc);
            if (*ptail).tty_fd >= 0 {
                close((*ptail).tty_fd);
                (*ptail).tty_fd = -1;
            }
        }
    }

    pub unsafe fn pty_proc_teardown(loop_: *mut Loop) {
        unsafe {
            uv_signal_stop(loop_children_watcher(loop_));
        }
    }

    unsafe fn init_child(
        ptyproc: *mut PtyProc,
        prog: *const c_char,
        argv_ptrs: *const *mut c_char,
        new_env: *mut *mut c_char,
    ) -> ! {
        unsafe {
            setsid();

            signal(SIGCHLD, SIG_DFL);
            signal(SIGHUP, SIG_DFL);
            signal(SIGINT, SIG_DFL);
            signal(SIGQUIT, SIG_DFL);
            signal(SIGTERM, SIG_DFL);
            signal(SIGALRM, SIG_DFL);

            let proc_: *mut Proc = ptyproc;
            let hdr = proc_;
            if let Some(ref cwd) = (*hdr).cwd {
                let err = uv_chdir(cwd.as_ptr());
                if err != 0 {
                    logmsg!(
                        LOGLVL_ERR,
                        ptr::null(),
                        c"init_child".as_ptr(),
                        line!() as c_int,
                        true,
                        c"chdir(%s) failed: %s".as_ptr(),
                        cwd.as_ptr(),
                        uv_strerror(err),
                    );
                    _exit(122);
                }
            }

            *os_environ_ptr() = new_env;
            execvp(prog, argv_ptrs);
            let err = os_errno();
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
                let err = os_errno();
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
                    status = -os_errno();
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

            for &proc_ in &(*qc).children {
                let hdr = proc_;
                let proc_t = proc_;

                let mut pid: c_int;
                loop {
                    pid = waitpid((*hdr).pid, &raw mut stat, WNOHANG | WUNTRACED | WCONTINUED);
                    if !(pid < 0 && os_errno() == EINTR) {
                        break;
                    }
                }

                if pid <= 0 {
                    continue;
                }

                if wifstopped(stat) {
                    if !(*hdr).data.is_null() {
                        crate::channel::channel_proc_state_cb(proc_, true, (*hdr).data);
                    }
                    continue;
                }
                if wifcontinued(stat) {
                    if !(*hdr).data.is_null() {
                        crate::channel::channel_proc_state_cb(proc_, false, (*hdr).data);
                    }
                    continue;
                }

                if wifexited(stat) {
                    (*hdr).status = wexitstatus(stat);
                } else if wifsignaled(stat) {
                    (*hdr).status = 128 + wtermsig(stat);
                }
                let _ = proc_t;
                crate::event::proc::on_proc_exit(proc_);
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
