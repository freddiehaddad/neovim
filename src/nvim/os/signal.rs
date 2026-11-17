//! OS signal handling (`src/nvim/os/signal.c`).

use crate::autocmd::{EVENT_SIGNAL, apply_autocmds};
use crate::eval::vars::{VV_DYING, set_vim_var_nr};
use crate::event::r#loop::Loop;
use crate::event::signal::{SignalTarget, SignalWatcher};
use crate::ex_cmds2::autowrite_all;
use crate::globals::exc_state;
use crate::log::{LOGLVL_ERR, logmsg};
use crate::main_init::preserve_exit;
#[cfg(target_os = "linux")]
use crate::memline::ml_sync_all;
use crate::option_vars::*;
use crate::strings::format_to_buf;
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::c_void;
use std::rc::Rc;

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

struct SignalWatchers {
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
}

impl SignalWatchers {
    fn new(loop_: &'static Loop) -> Result<Self, i32> {
        let new = || SignalWatcher::new(loop_, loop_.fast_events.get(), SignalTarget::Editor);
        Ok(Self {
            spipe: new()?,
            shup: new()?,
            sint: new()?,
            squit: new()?,
            sterm: new()?,
            #[cfg(unix)]
            susr1: new()?,
            swinch: new()?,
            ststp: new()?,
            #[cfg(target_os = "linux")]
            spwr: new()?,
        })
    }

    fn start(&self) {
        #[cfg(unix)]
        let _ = self.spipe.start(SIGPIPE);
        let _ = self.shup.start(SIGHUP);
        let _ = self.sint.start(SIGINT);
        let _ = self.squit.start(SIGQUIT);
        let _ = self.sterm.start(SIGTERM);
        #[cfg(unix)]
        let _ = self.ststp.start(SIGTSTP);
        #[cfg(target_os = "linux")]
        let _ = self.spwr.start(SIGPWR);
        #[cfg(unix)]
        let _ = self.susr1.start(SIGUSR1);
        let _ = self.swinch.start(SIGWINCH);
    }

    fn stop(&self) {
        #[cfg(unix)]
        self.spipe.stop();
        self.shup.stop();
        self.sint.stop();
        self.squit.stop();
        self.sterm.stop();
        self.ststp.stop();
        #[cfg(target_os = "linux")]
        self.spwr.stop();
        #[cfg(unix)]
        self.susr1.stop();
        self.swinch.stop();
    }

    fn close(&self) {
        self.spipe.close();
        self.shup.close();
        self.sint.close();
        self.squit.close();
        self.sterm.close();
        self.ststp.close();
        #[cfg(target_os = "linux")]
        self.spwr.close();
        #[cfg(unix)]
        self.susr1.close();
        self.swinch.close();
    }
}

thread_local! {
    static SIG_WATCHERS: std::cell::RefCell<Option<Rc<SignalWatchers>>> = const {
        std::cell::RefCell::new(None)
    };
    static REJECTING_DEADLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

unsafe extern "C" {
    #[cfg(unix)]
    fn sigemptyset(set: *mut c_void) -> i32;
    #[cfg(unix)]
    fn pthread_sigmask(how: i32, set: *const c_void, oldset: *mut c_void) -> i32;
    #[cfg(unix)]
    pub fn signal(signum: i32, handler: usize) -> usize;
}

#[cfg(unix)]
#[inline]
pub fn os_reset_sigpipe() {
    unsafe {
        signal(SIGPIPE, 0 /* SIG_DFL */);
    }
}

#[cfg(unix)]
pub fn os_suspend_process() {
    #[cfg(target_os = "linux")]
    const SIGSTOP: i32 = 19;
    #[cfg(not(target_os = "linux"))]
    const SIGSTOP: i32 = 17;
    unsafe {
        let ppid = crate::fileio::getppid();
        if crate::fileio::getpgid(ppid) == crate::fileio::getpgrp() {
            crate::fileio::kill(ppid, SIGSTOP);
        }
        crate::fileio::kill(0, SIGSTOP);
    }
}

pub fn signal_init() {
    #[cfg(unix)]
    unsafe {
        let mut mask = [0u64; 16];
        sigemptyset((&raw mut mask).cast());
        if pthread_sigmask(SIG_SETMASK, (&raw const mask).cast(), std::ptr::null_mut()) != 0 {
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"signal_init"),
                line!() as i32,
                true,
                c"Could not unblock signals, nvim might behave strangely.",
            );
        }
    }

    let watchers = Rc::new(
        SignalWatchers::new(crate::event::r#loop::main_loop()).expect("uv_signal_init failed"),
    );
    let previous = SIG_WATCHERS.with(|cell| cell.replace(Some(watchers)));
    if let Some(previous) = previous {
        previous.close();
    }
    signal_start();
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
    let watchers = SIG_WATCHERS.with(|cell| cell.borrow_mut().take());
    if let Some(watchers) = &watchers {
        watchers.stop();
    }
    signal_ignore_deadly();
    if let Some(watchers) = watchers {
        watchers.close();
    }
}

pub fn signal_start() {
    let watchers = SIG_WATCHERS.with(|cell| cell.borrow().clone());
    if let Some(watchers) = watchers {
        watchers.start();
    }
}

pub fn signal_stop() {
    let watchers = SIG_WATCHERS.with(|cell| cell.borrow().clone());
    if let Some(watchers) = watchers {
        watchers.stop();
    }
}

pub fn signal_reject_deadly() {
    REJECTING_DEADLY.set(true);
}

pub fn signal_accept_deadly() {
    REJECTING_DEADLY.set(false);
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
    exc_state().v_dying.set(1);

    let mut iobuff = [0u8; crate::globals::IOSIZE];
    format_to_buf!(
        &mut iobuff,
        c"Nvim: Caught deadly signal '%s'\n",
        signal_name(signum),
    );

    if opt_vars().p_awa.get() && signum != SIGTERM && signum != SIGINT {
        autowrite_all();
    }

    // Preserve files and exit.
    preserve_exit(Some(crate::memory::bytes_as_cstr(&iobuff)));
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
            if opt_vars().p_awa.get() {
                autowrite_all();
            }
            return;
        }
        #[cfg(unix)]
        if signum == SIGUSR1 {
            apply_autocmds(
                EVENT_SIGNAL,
                Some(c"SIGUSR1"),
                crate::globals::curbuf().fname_opt().as_deref(),
                true,
                crate::globals::curbuf_opt().as_deref(),
            );
            return;
        }
        match signum {
            SIGHUP => {
                #[cfg(windows)]
                os_clear_hwnd();
                if !REJECTING_DEADLY.get() {
                    deadly_signal(signum);
                }
            }
            SIGINT | SIGTERM | SIGQUIT => {
                if !REJECTING_DEADLY.get() {
                    deadly_signal(signum);
                }
            }
            SIGWINCH => {
                apply_autocmds(
                    EVENT_SIGNAL,
                    Some(c"SIGWINCH"),
                    crate::globals::curbuf().fname_opt().as_deref(),
                    true,
                    crate::globals::curbuf_opt().as_deref(),
                );
            }
            _ => {
                logmsg!(
                    LOGLVL_ERR,
                    None,
                    Some(c"on_signal"),
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
        assert!(REJECTING_DEADLY.get());
        signal_accept_deadly();
        assert!(!REJECTING_DEADLY.get());
    }
}
