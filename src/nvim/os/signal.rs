//! OS signal handling (`src/nvim/os/signal.c`).

use crate::autocmd::{EVENT_SIGNAL, apply_autocmds};
use crate::eval::vars::{VV_DYING, set_vim_var_nr};
use crate::event::signal::{
    SignalWatcher, signal_watcher_close, signal_watcher_init, signal_watcher_start,
    signal_watcher_stop,
};
use crate::ex_cmds2::autowrite_all;
use crate::globals::{IOSIZE, g_st};
use crate::log::{LOGLVL_ERR, logmsg};
use crate::main_init::preserve_exit;
#[cfg(unix)]
use crate::memline::ml_sync_all;
use crate::option_vars::*;
use crate::strings::vim_snprintf;
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::c_void;
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::os_clear_hwnd;

pub(crate) const SIGHUP: i32 = 1;
pub(crate) const SIGINT: i32 = 2;
pub(crate) const SIGQUIT: i32 = 3;
pub(crate) const SIGKILL: i32 = 9;
pub(crate) const SIGTERM: i32 = 15;
pub(crate) const SIGWINCH: i32 = 28;

#[cfg(unix)]
const SIGPIPE: i32 = 13;
#[cfg(target_os = "linux")]
const SIGUSR1: i32 = 10;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGUSR1: i32 = 30;
#[cfg(target_os = "linux")]
const SIGTSTP: i32 = 20;
#[cfg(all(unix, not(target_os = "linux")))]
const SIGTSTP: i32 = 18;
#[cfg(target_os = "linux")]
const SIGPWR: i32 = 30;

#[cfg(target_os = "linux")]
const SIG_SETMASK: i32 = 2;
#[cfg(all(unix, not(target_os = "linux")))]
const SIG_SETMASK: i32 = 3;
#[cfg(unix)]
const SIG_IGN: usize = 1;

struct SignalModuleState {
    spipe: SignalWatcher,
    shup: SignalWatcher,
    sint: SignalWatcher,
    squit: SignalWatcher,
    sterm: SignalWatcher,
    #[cfg(unix)]
    susr1: SignalWatcher,
    swinch: SignalWatcher,
    ststp: SignalWatcher,
    #[cfg(target_os = "linux")]
    spwr: SignalWatcher,
    rejecting_deadly: bool,
}

impl SignalModuleState {
    const fn new() -> Self {
        Self {
            spipe: SignalWatcher::new(),
            shup: SignalWatcher::new(),
            sint: SignalWatcher::new(),
            squit: SignalWatcher::new(),
            sterm: SignalWatcher::new(),
            #[cfg(unix)]
            susr1: SignalWatcher::new(),
            swinch: SignalWatcher::new(),
            ststp: SignalWatcher::new(),
            #[cfg(target_os = "linux")]
            spwr: SignalWatcher::new(),
            rejecting_deadly: false,
        }
    }
}

static SIG_STATE: crate::globals::GlobalState<SignalModuleState> =
    crate::globals::GlobalState::new(SignalModuleState::new());

#[inline]
fn sig_st() -> &'static mut SignalModuleState {
    unsafe { &mut *SIG_STATE.as_ptr() }
}

unsafe extern "C" {
    #[cfg(unix)]
    fn sigemptyset(set: *mut c_void) -> i32;
    #[cfg(unix)]
    fn pthread_sigmask(how: i32, set: *const c_void, oldset: *mut c_void) -> i32;
    #[cfg(unix)]
    pub fn signal(signum: i32, handler: usize) -> usize;
}

pub fn signal_init() {
    {
        #[cfg(unix)]
        unsafe {
            let mut mask = [0u64; 16];
            sigemptyset((&raw mut mask).cast());
            if pthread_sigmask(SIG_SETMASK, (&raw const mask).cast(), ptr::null_mut()) != 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"signal_init",
                    line!() as i32,
                    true,
                    c"Could not unblock signals, nvim might behave strangely.",
                );
            }
        }

        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().spipe,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().shup,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().sint,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().squit,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().sterm,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().ststp,
            ptr::null_mut(),
        );
        #[cfg(target_os = "linux")]
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().spwr,
            ptr::null_mut(),
        );
        #[cfg(unix)]
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().susr1,
            ptr::null_mut(),
        );
        signal_watcher_init(
            &mut g_st().main_loop,
            &raw mut sig_st().swinch,
            ptr::null_mut(),
        );
        signal_start();
    }
}

/// During shutdown, we don't want the default actions of these signals.
fn signal_ignore_deadly() {
    #[cfg(unix)]
    unsafe {
        signal(SIGHUP, SIG_IGN);
        signal(SIGINT, SIG_IGN);
        signal(SIGTERM, SIG_IGN);
        signal(SIGQUIT, SIG_IGN);
    }
}

pub fn signal_teardown() {
    signal_stop();
    signal_ignore_deadly();
    signal_watcher_close(&mut sig_st().spipe);
    signal_watcher_close(&mut sig_st().shup);
    signal_watcher_close(&mut sig_st().sint);
    signal_watcher_close(&mut sig_st().squit);
    signal_watcher_close(&mut sig_st().sterm);
    signal_watcher_close(&mut sig_st().ststp);
    #[cfg(target_os = "linux")]
    signal_watcher_close(&mut sig_st().spwr);
    #[cfg(unix)]
    signal_watcher_close(&mut sig_st().susr1);
    signal_watcher_close(&mut sig_st().swinch);
}

pub fn signal_start() {
    #[cfg(unix)]
    signal_watcher_start(&mut sig_st().spipe, SIGPIPE);
    signal_watcher_start(&mut sig_st().shup, SIGHUP);
    signal_watcher_start(&mut sig_st().sint, SIGINT);
    signal_watcher_start(&mut sig_st().squit, SIGQUIT);
    signal_watcher_start(&mut sig_st().sterm, SIGTERM);
    #[cfg(unix)]
    signal_watcher_start(&mut sig_st().ststp, SIGTSTP);
    #[cfg(target_os = "linux")]
    signal_watcher_start(&mut sig_st().spwr, SIGPWR);
    #[cfg(unix)]
    signal_watcher_start(&mut sig_st().susr1, SIGUSR1);
    signal_watcher_start(&mut sig_st().swinch, SIGWINCH);
}

pub fn signal_stop() {
    #[cfg(unix)]
    signal_watcher_stop(&mut sig_st().spipe);
    signal_watcher_stop(&mut sig_st().shup);
    signal_watcher_stop(&mut sig_st().sint);
    signal_watcher_stop(&mut sig_st().squit);
    signal_watcher_stop(&mut sig_st().sterm);
    signal_watcher_stop(&mut sig_st().ststp);
    #[cfg(target_os = "linux")]
    signal_watcher_stop(&mut sig_st().spwr);
    #[cfg(unix)]
    signal_watcher_stop(&mut sig_st().susr1);
    signal_watcher_stop(&mut sig_st().swinch);
}

pub fn signal_reject_deadly() {
    sig_st().rejecting_deadly = true;
}

pub fn signal_accept_deadly() {
    sig_st().rejecting_deadly = false;
}

fn signal_name(signum: i32) -> &'static CStr {
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
fn deadly_signal(signum: i32) -> ! {
    // Set the v:dying variable.
    set_vim_var_nr(VV_DYING, 1);
    g_st().v_dying = 1;

    let io_buf = g_st().iobuff.as_mut_ptr();
    vim_snprintf!(
        io_buf,
        IOSIZE,
        c"Nvim: Caught deadly signal '%s'\n",
        signal_name(signum).as_ptr(),
    );

    if opt_vars().p_awa != 0 && signum != SIGTERM && signum != SIGINT {
        autowrite_all();
    }

    // Preserve files and exit.
    preserve_exit(io_buf);
}

pub(crate) fn on_signal(signum: i32) {
    debug_assert!(signum >= 0);
    {
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
            if opt_vars().p_awa != 0 {
                autowrite_all();
            }
            return;
        }
        #[cfg(unix)]
        if signum == SIGUSR1 {
            apply_autocmds(
                EVENT_SIGNAL,
                c"SIGUSR1",
                crate::globals::curbuf_mut().b_fname,
                true,
                g_st().curbuf,
            );
            return;
        }
        match signum {
            SIGHUP => {
                #[cfg(windows)]
                os_clear_hwnd();
                if !sig_st().rejecting_deadly {
                    deadly_signal(signum);
                }
            }
            SIGINT | SIGTERM | SIGQUIT => {
                if !sig_st().rejecting_deadly {
                    deadly_signal(signum);
                }
            }
            SIGWINCH => {
                apply_autocmds(
                    EVENT_SIGNAL,
                    c"SIGWINCH",
                    crate::globals::curbuf_mut().b_fname,
                    true,
                    g_st().curbuf,
                );
            }
            _ => {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"on_signal",
                    line!() as i32,
                    true,
                    c"invalid signal: %d",
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
        signal_reject_deadly();
        assert!(sig_st().rejecting_deadly);
        signal_accept_deadly();
        assert!(!sig_st().rejecting_deadly);
    }
}
