//! Signal watcher wrapper around libuv `uv_signal_t` (`src/nvim/event/signal.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_signal_init, uv_signal_start, uv_signal_stop, uv_signal_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::time::loop_fast_events;

#[repr(C)]
pub struct SignalWatcher {
    pub uv: [u64; 40],
    pub is_tui: bool,
    pub events: MultiQueue,
    pub signum: i32,
}

impl SignalWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 40],
            is_tui: false,
            events: MultiQueue::NULL,
            signum: 0,
        }
    }
}

impl Default for SignalWatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn signal_watcher_init(loop_: &mut Loop, watcher: &mut SignalWatcher) {
    unsafe {
        uv_signal_init((&raw mut loop_.uv).cast(), (&raw mut watcher.uv).cast());
        *(&raw mut watcher.uv).cast::<*mut SignalWatcher>() = watcher;
        watcher.is_tui = false;
        watcher.events = loop_fast_events(loop_);
        watcher.signum = 0;
    }
}

pub fn signal_watcher_start(watcher: &mut SignalWatcher, signum: i32) {
    watcher.signum = signum;
    unsafe {
        uv_signal_start(
            (&raw mut watcher.uv).cast(),
            Some(signal_watcher_cb),
            signum,
        )
    };
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

pub fn signal_event(signum: i32, is_tui: bool) {
    if !is_tui {
        crate::os::signal::on_signal(signum);
    } else {
        crate::ui_client::with_tui_mut(crate::tui::tui::sigwinch_cb);
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, signum: i32) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        (*watcher).signum = signum;
        let is_tui = (*watcher).is_tui;
        let events = (*watcher).events;
        if let Some(events) = events.opt_mut() {
            multiqueue_put_event(events, Event::Signal { signum, is_tui });
        } else {
            signal_event(signum, is_tui);
        }
    }
}
