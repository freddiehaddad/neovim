//! Signal watcher wrapper around libuv `uv_signal_t` (`src/nvim/event/signal.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_signal_init, uv_signal_start, uv_signal_stop, uv_signal_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::time::loop_fast_events;
use std::ffi::c_int;
use std::ptr;

#[repr(C)]
pub struct SignalWatcher {
    pub uv: [u64; 40],
    pub data: *mut crate::tui::tui::TUIData,
    pub events: *mut MultiQueue,
    pub signum: c_int,
}

impl SignalWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 40],
            data: ptr::null_mut(),
            events: ptr::null_mut(),
            signum: 0,
        }
    }
}

impl Default for SignalWatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn signal_watcher_init(
    loop_: &mut Loop,
    watcher: *mut SignalWatcher,
    data: *mut crate::tui::tui::TUIData,
) {
    unsafe {
        uv_signal_init((&raw mut loop_.uv).cast(), (&raw mut (*watcher).uv).cast());
        *(&raw mut (*watcher).uv).cast::<*mut SignalWatcher>() = watcher;
        (*watcher).data = data;
        (*watcher).events = loop_fast_events(loop_);
        (*watcher).signum = 0;
    }
}

pub fn signal_watcher_start(watcher: &mut SignalWatcher, signum: c_int) {
    unsafe {
        watcher.signum = signum;
        uv_signal_start(
            (&raw mut watcher.uv).cast(),
            Some(signal_watcher_cb),
            signum,
        );
    }
}

pub fn signal_watcher_stop(watcher: &mut SignalWatcher) {
    unsafe {
        uv_signal_stop((&raw mut watcher.uv).cast());
    }
}

pub fn signal_watcher_close(watcher: &mut SignalWatcher) {
    unsafe {
        uv_close((&raw mut watcher.uv).cast(), None);
    }
}

pub fn signal_event(watcher: &mut SignalWatcher) {
    unsafe {
        if watcher.data.is_null() {
            crate::os::signal::on_signal(watcher.signum);
        } else {
            crate::tui::tui::sigwinch_cb(&mut *watcher.data);
        }
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, signum: c_int) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        (*watcher).signum = signum;
        let events = (*watcher).events;
        if !events.is_null() {
            multiqueue_put_event(events, Event::Signal(watcher));
        } else {
            signal_event(&mut *watcher);
        }
    }
}
