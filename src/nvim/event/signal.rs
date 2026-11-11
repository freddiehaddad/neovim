//! Signal watcher wrapper around libuv `uv_signal_t` (`src/nvim/event/signal.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_signal_init, uv_signal_start, uv_signal_stop, uv_signal_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::time::loop_fast_events;

#[repr(C)]
pub struct SignalWatcher {
    pub uv: [u64; 40],
    pub data: crate::globals::GraphPtr<crate::tui::tui::TUIData>,
    pub events: MultiQueue,
    pub signum: i32,
}

impl SignalWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 40],
            data: crate::globals::GraphPtr::NULL,
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
        watcher.data = crate::globals::GraphPtr::NULL;
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

pub fn signal_event(watcher: &mut SignalWatcher) {
    if watcher.data.is_null() {
        crate::os::signal::on_signal(watcher.signum);
    } else {
        crate::tui::tui::sigwinch_cb(watcher.data.borrow_mut());
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, signum: i32) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        (*watcher).signum = signum;
        let events = (*watcher).events;
        if let Some(events) = events.opt_mut() {
            multiqueue_put_event(events, Event::Signal(watcher.into()));
        } else {
            signal_event(&mut *watcher);
        }
    }
}
