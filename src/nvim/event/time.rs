//! Timer watcher wrapper around libuv `uv_timer_t` (`src/nvim/event/time.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_t, uv_timer_init, uv_timer_start, uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_empty, multiqueue_put_event};
use std::ptr;

pub type time_cb = Option<unsafe fn(watcher: *mut TimeWatcher, data: *mut crate::eval::timer_T)>;

#[repr(C)]
pub struct TimeWatcher {
    pub uv: [u64; 32],
    pub data: *mut crate::eval::timer_T,
    pub cb: time_cb,
    pub close_cb: time_cb,
    pub events: *mut MultiQueue,
    pub blockable: bool,
}

impl TimeWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 32],
            data: ptr::null_mut(),
            cb: None,
            close_cb: None,
            events: ptr::null_mut(),
            blockable: false,
        }
    }
}

impl Default for TimeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub unsafe fn loop_fast_events(loop_: *mut Loop) -> *mut MultiQueue {
    unsafe { (*loop_).fast_events }
}

pub unsafe fn time_watcher_init(
    loop_: *mut Loop,
    watcher: *mut TimeWatcher,
    data: *mut crate::eval::timer_T,
) {
    unsafe {
        uv_timer_init(loop_.cast(), (&raw mut (*watcher).uv).cast());
        *(&raw mut (*watcher).uv).cast::<*mut TimeWatcher>() = watcher;
        (*watcher).data = data;
        (*watcher).cb = None;
        (*watcher).close_cb = None;
        (*watcher).events = loop_fast_events(loop_);
        (*watcher).blockable = false;
    }
}

pub unsafe fn time_watcher_start(
    watcher: *mut TimeWatcher,
    cb: time_cb,
    timeout: u64,
    repeat: u64,
) {
    unsafe {
        (*watcher).cb = cb;
        uv_timer_start(
            (&raw mut (*watcher).uv).cast(),
            Some(time_watcher_cb),
            timeout,
            repeat,
        );
    }
}

pub unsafe fn time_watcher_stop(watcher: *mut TimeWatcher) {
    unsafe {
        uv_timer_stop((&raw mut (*watcher).uv).cast());
    }
}

pub unsafe fn time_watcher_close(watcher: *mut TimeWatcher, cb: time_cb) {
    unsafe {
        (*watcher).close_cb = cb;
        uv_close((&raw mut (*watcher).uv).cast(), Some(close_cb));
    }
}

pub unsafe fn time_event(watcher: *mut TimeWatcher) {
    unsafe {
        if let Some(cb) = (*watcher).cb {
            cb(watcher, (*watcher).data);
        }
    }
}

unsafe extern "C" fn time_watcher_cb(handle: *mut uv_timer_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        if (*watcher).blockable && !multiqueue_empty((*watcher).events) {
            return;
        }
        if !(*watcher).events.is_null() {
            multiqueue_put_event((*watcher).events, Event::TimeWatcher(watcher));
        } else {
            time_event(watcher);
        }
    }
}

pub unsafe fn close_event(watcher: *mut TimeWatcher) {
    unsafe {
        if let Some(cb) = (*watcher).close_cb {
            cb(watcher, (*watcher).data);
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        if (*watcher).close_cb.is_some() {
            if !(*watcher).events.is_null() {
                multiqueue_put_event((*watcher).events, Event::TimeWatcherClose(watcher));
            } else {
                close_event(watcher);
            }
        }
    }
}
