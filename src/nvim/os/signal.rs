//! OS signal handling (`src/nvim/os/signal.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::autocmd::apply_autocmds;
use crate::eval::vars::set_vim_var_nr;
use crate::event::signal::{
    SignalWatcher, signal_watcher_close, signal_watcher_init, signal_watcher_start,
    signal_watcher_stop,
};
use crate::ex_cmds2::autowrite_all;
use crate::globals::main_loop;
use crate::globals::*;
use crate::log::logmsg;
use crate::main_init::preserve_exit;
#[cfg(unix)]
use crate::memline::ml_sync_all;
use crate::option_vars::*;
use crate::strings::vim_snprintf;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::os_clear_hwnd;

const IOSIZE: usize = 1025;
const VV_DYING: c_int = 29;
const EVENT_SIGNAL: c_int = 104;
const LOGLVL_ERR: c_int = 4;

const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGQUIT: c_int = 3;
const SIGTERM: c_int = 15;
const SIGWINCH: c_int = 28;

#[cfg(unix)]
const SIGPIPE: c_int = 13;
#[cfg(target_os = "linux")]
const SIGUSR1: c_int = 10;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGUSR1: c_int = 30;
#[cfg(target_os = "linux")]
const SIGTSTP: c_int = 20;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGTSTP: c_int = 18;
#[cfg(target_os = "linux")]
const SIGPWR: c_int = 30;

#[cfg(target_os = "linux")]
const SIG_SETMASK: c_int = 2;
#[cfg(all(unix, not(target_os = "linux")))]
const SIG_SETMASK: c_int = 3;
#[cfg(unix)]
const SIG_IGN: usize = 1;

#[repr(C, align(8))]
struct SignalWatcherBuf([u8; 512]);

static mut spipe: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut shup: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut sint: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut squit: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut sterm: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
#[cfg(unix)]
static mut susr1: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut swinch: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
static mut ststp: SignalWatcherBuf = SignalWatcherBuf([0; 512]);
#[cfg(target_os = "linux")]
static mut spwr: SignalWatcherBuf = SignalWatcherBuf([0; 512]);

static mut rejecting_deadly: bool = false;

unsafe extern "C" {
    #[cfg(unix)]
    fn sigemptyset(set: *mut c_void) -> c_int;
    #[cfg(unix)]
    fn pthread_sigmask(how: c_int, set: *const c_void, oldset: *mut c_void) -> c_int;
    #[cfg(unix)]
    pub fn signal(signum: c_int, handler: usize) -> usize;
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_init() {
    unsafe {
        #[cfg(unix)]
        {
            let mut mask = [0u64; 16];
            sigemptyset((&raw mut mask).cast());
            if pthread_sigmask(SIG_SETMASK, (&raw const mask).cast(), ptr::null_mut()) != 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"signal_init".as_ptr(),
                    line!() as c_int,
                    true,
                    c"Could not unblock signals, nvim might behave strangely.".as_ptr(),
                );
            }
        }

        let loop_ptr: *mut c_void = (&raw mut main_loop).cast();
        signal_watcher_init(loop_ptr, (&raw mut spipe).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut shup).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut sint).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut squit).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut sterm).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut ststp).cast(), ptr::null_mut());
        #[cfg(target_os = "linux")]
        signal_watcher_init(loop_ptr, (&raw mut spwr).cast(), ptr::null_mut());
        #[cfg(unix)]
        signal_watcher_init(loop_ptr, (&raw mut susr1).cast(), ptr::null_mut());
        signal_watcher_init(loop_ptr, (&raw mut swinch).cast(), ptr::null_mut());
        signal_start();
    }
}

/// During shutdown, we don't want the default actions of these signals.
unsafe fn signal_ignore_deadly() {
    #[cfg(unix)]
    unsafe {
        signal(SIGHUP, SIG_IGN);
        signal(SIGINT, SIG_IGN);
        signal(SIGTERM, SIG_IGN);
        signal(SIGQUIT, SIG_IGN);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_teardown() {
    unsafe {
        signal_stop();
        signal_ignore_deadly();
        signal_watcher_close((&raw mut spipe).cast(), None);
        signal_watcher_close((&raw mut shup).cast(), None);
        signal_watcher_close((&raw mut sint).cast(), None);
        signal_watcher_close((&raw mut squit).cast(), None);
        signal_watcher_close((&raw mut sterm).cast(), None);
        signal_watcher_close((&raw mut ststp).cast(), None);
        #[cfg(target_os = "linux")]
        signal_watcher_close((&raw mut spwr).cast(), None);
        #[cfg(unix)]
        signal_watcher_close((&raw mut susr1).cast(), None);
        signal_watcher_close((&raw mut swinch).cast(), None);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_start() {
    unsafe {
        #[cfg(unix)]
        signal_watcher_start((&raw mut spipe).cast(), Some(on_signal), SIGPIPE);
        signal_watcher_start((&raw mut shup).cast(), Some(on_signal), SIGHUP);
        signal_watcher_start((&raw mut sint).cast(), Some(on_signal), SIGINT);
        signal_watcher_start((&raw mut squit).cast(), Some(on_signal), SIGQUIT);
        signal_watcher_start((&raw mut sterm).cast(), Some(on_signal), SIGTERM);
        #[cfg(unix)]
        signal_watcher_start((&raw mut ststp).cast(), Some(on_signal), SIGTSTP);
        #[cfg(target_os = "linux")]
        signal_watcher_start((&raw mut spwr).cast(), Some(on_signal), SIGPWR);
        #[cfg(unix)]
        signal_watcher_start((&raw mut susr1).cast(), Some(on_signal), SIGUSR1);
        signal_watcher_start((&raw mut swinch).cast(), Some(on_signal), SIGWINCH);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_stop() {
    unsafe {
        #[cfg(unix)]
        signal_watcher_stop((&raw mut spipe).cast());
        signal_watcher_stop((&raw mut shup).cast());
        signal_watcher_stop((&raw mut sint).cast());
        signal_watcher_stop((&raw mut squit).cast());
        signal_watcher_stop((&raw mut sterm).cast());
        signal_watcher_stop((&raw mut ststp).cast());
        #[cfg(target_os = "linux")]
        signal_watcher_stop((&raw mut spwr).cast());
        #[cfg(unix)]
        signal_watcher_stop((&raw mut susr1).cast());
        signal_watcher_stop((&raw mut swinch).cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_reject_deadly() {
    unsafe {
        rejecting_deadly = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_accept_deadly() {
    unsafe {
        rejecting_deadly = false;
    }
}

fn signal_name(signum: c_int) -> &'static CStr {
    #[cfg(target_os = "linux")]
    if signum == SIGPWR {
        return c"SIGPWR";
    }
    #[cfg(unix)]
    if signum == SIGPIPE {
        return c"SIGPIPE";
    }
    #[cfg(unix)]
    if signum == SIGTSTP {
        return c"SIGTSTP";
    }
    #[cfg(unix)]
    if signum == SIGUSR1 {
        return c"SIGUSR1";
    }
    match signum {
        SIGTERM => c"SIGTERM",
        SIGQUIT => c"SIGQUIT",
        SIGHUP => c"SIGHUP",
        SIGINT => c"SIGINT",
        SIGWINCH => c"SIGWINCH",
        _ => c"Unknown",
    }
}

/// This function handles deadly signals.
unsafe fn deadly_signal(signum: c_int) -> ! {
    unsafe {
        // Set the v:dying variable.
        set_vim_var_nr(VV_DYING, 1);
        v_dying = 1;

        let io_buf = (&raw mut IObuff).cast::<c_char>();
        vim_snprintf!(
            io_buf,
            IOSIZE,
            c"Nvim: Caught deadly signal '%s'\n".as_ptr(),
            signal_name(signum).as_ptr(),
        );

        if p_awa != 0 && signum != SIGTERM && signum != SIGINT {
            autowrite_all();
        }

        // Preserve files and exit.
        preserve_exit(io_buf);
    }
}

unsafe extern "C" fn on_signal(_handle: *mut SignalWatcher, signum: c_int, _data: *mut c_void) {
    debug_assert!(signum >= 0);
    unsafe {
        #[cfg(target_os = "linux")]
        if signum == SIGPWR {
            ml_sync_all(0, 0, true);
            return;
        }
        #[cfg(unix)]
        if signum == SIGPIPE {
            return;
        }
        #[cfg(unix)]
        if signum == SIGTSTP {
            if p_awa != 0 {
                autowrite_all();
            }
            return;
        }
        #[cfg(unix)]
        if signum == SIGUSR1 {
            apply_autocmds(
                EVENT_SIGNAL,
                c"SIGUSR1".as_ptr().cast_mut(),
                (*curbuf).b_fname,
                true,
                curbuf,
            );
            return;
        }
        match signum {
            SIGHUP => {
                #[cfg(windows)]
                os_clear_hwnd();
                if !rejecting_deadly {
                    deadly_signal(signum);
                }
            }
            SIGINT | SIGTERM | SIGQUIT => {
                if !rejecting_deadly {
                    deadly_signal(signum);
                }
            }
            SIGWINCH => {
                apply_autocmds(
                    EVENT_SIGNAL,
                    c"SIGWINCH".as_ptr().cast_mut(),
                    (*curbuf).b_fname,
                    true,
                    curbuf,
                );
            }
            _ => {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"on_signal".as_ptr(),
                    line!() as c_int,
                    true,
                    c"invalid signal: %d".as_ptr(),
                    signum,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_name_and_deadly_toggle_work() {
        assert_eq!(signal_name(SIGINT).to_bytes(), b"SIGINT");
        assert_eq!(signal_name(SIGTERM).to_bytes(), b"SIGTERM");
        assert_eq!(signal_name(999).to_bytes(), b"Unknown");
        unsafe {
            signal_reject_deadly();
            assert!(rejecting_deadly);
            signal_accept_deadly();
            assert!(!rejecting_deadly);
        }
    }
}
