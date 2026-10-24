//! Signal watcher wrapper around libuv `uv_signal_t` (`src/nvim/event/signal.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_t, uv_signal_init, uv_signal_start, uv_signal_stop, uv_signal_t,
};
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_put_event};
use crate::event::time::loop_fast_events;
use std::ffi::{c_int, c_void};
use std::ptr;

pub type signal_cb = Option<
    unsafe fn(watcher: *mut SignalWatcher, signum: c_int, data: *mut crate::tui::tui::TUIData),
>;
pub type signal_close_cb =
    Option<unsafe fn(watcher: *mut SignalWatcher, data: *mut crate::tui::tui::TUIData)>;

#[repr(C)]
pub struct SignalWatcher {
    pub uv: [u64; 32],
    pub data: *mut crate::tui::tui::TUIData,
    pub cb: signal_cb,
    pub close_cb: signal_close_cb,
    pub events: *mut MultiQueue,
    pub signum: c_int,
}

impl SignalWatcher {
    pub const fn zeroed() -> Self {
        Self {
            uv: [0; 32],
            data: ptr::null_mut(),
            cb: None,
            close_cb: None,
            events: ptr::null_mut(),
            signum: 0,
        }
    }
}

pub unsafe fn signal_watcher_init(
    loop_: *mut Loop,
    watcher: *mut SignalWatcher,
    data: *mut crate::tui::tui::TUIData,
) {
    unsafe {
        uv_signal_init(loop_.cast(), (&raw mut (*watcher).uv).cast());
        *(&raw mut (*watcher).uv).cast::<*mut SignalWatcher>() = watcher;
        (*watcher).data = data;
        (*watcher).cb = None;
        (*watcher).close_cb = None;
        (*watcher).events = loop_fast_events(loop_);
        (*watcher).signum = 0;
    }
}

pub unsafe fn signal_watcher_start(watcher: *mut SignalWatcher, cb: signal_cb, signum: c_int) {
    unsafe {
        (*watcher).cb = cb;
        (*watcher).signum = signum;
        uv_signal_start(
            (&raw mut (*watcher).uv).cast(),
            Some(signal_watcher_cb),
            signum,
        );
    }
}

pub unsafe fn signal_watcher_stop(watcher: *mut SignalWatcher) {
    unsafe {
        uv_signal_stop((&raw mut (*watcher).uv).cast());
    }
}

pub unsafe fn signal_watcher_close(watcher: *mut SignalWatcher, cb: signal_close_cb) {
    unsafe {
        (*watcher).close_cb = cb;
        uv_close((&raw mut (*watcher).uv).cast(), Some(close_cb));
    }
}

unsafe fn signal_event(argv: *mut *mut c_void) {
    unsafe {
        let watcher = (*argv).cast::<SignalWatcher>();
        if let Some(cb) = (*watcher).cb {
            cb(watcher, (*watcher).signum, (*watcher).data);
        }
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, signum: c_int) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        (*watcher).signum = signum;
        let events = (*watcher).events;
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = watcher.cast();
        if !events.is_null() {
            multiqueue_put_event(
                events,
                Event {
                    handler: Some(signal_event),
                    argv,
                },
            );
        } else {
            signal_event(argv.as_mut_ptr());
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        if let Some(cb) = (*watcher).close_cb {
            cb(watcher, (*watcher).data);
        }
    }
}
