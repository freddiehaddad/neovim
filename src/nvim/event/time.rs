//! Timer watcher wrapper around libuv `uv_timer_t` (`src/nvim/event/time.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use std::ffi::{c_int, c_void};
use std::ptr;

use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_empty, multiqueue_put_event,
};

const UV_TIMER: c_int = 13;

pub use crate::event::r#loop::Loop;
pub type TimeWatcher = c_void;
pub use crate::event::r#loop::uv_handle_t;
pub use crate::event::r#loop::uv_loop_t;
pub use crate::event::r#loop::uv_timer_t;

pub type time_cb = Option<unsafe extern "C" fn(watcher: *mut TimeWatcher, data: *mut c_void)>;
type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;
type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;

#[repr(C)]
pub struct TimeWatcherFields {
    pub data: *mut c_void,
    pub cb: time_cb,
    pub close_cb: time_cb,
    pub events: *mut MultiQueue,
    pub blockable: bool,
}

unsafe extern "C" {
    fn uv_loop_size() -> usize;
    fn uv_handle_size(h_type: c_int) -> usize;
    fn uv_timer_init(loop_: *mut uv_loop_t, handle: *mut uv_timer_t) -> c_int;
    fn uv_timer_start(handle: *mut uv_timer_t, cb: uv_timer_cb, timeout: u64, repeat: u64)
    -> c_int;
    fn uv_timer_stop(handle: *mut uv_timer_t) -> c_int;
    fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);
}

#[inline]
pub unsafe fn watcher_fields(watcher: *mut TimeWatcher) -> *mut TimeWatcherFields {
    unsafe {
        watcher
            .cast::<u8>()
            .add(uv_handle_size(UV_TIMER))
            .cast::<TimeWatcherFields>()
    }
}

#[inline]
unsafe fn loop_fast_events(loop_: *mut Loop) -> *mut MultiQueue {
    unsafe {
        *loop_
            .cast::<u8>()
            .add(uv_loop_size() + 2 * std::mem::size_of::<*mut MultiQueue>())
            .cast::<*mut MultiQueue>()
    }
}

#[inline]
unsafe fn create_event(
    events: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    watcher: *mut TimeWatcher,
) {
    unsafe {
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = watcher;
        if !events.is_null() {
            multiqueue_put_event(
                events,
                Event {
                    handler: Some(handler),
                    argv,
                },
            );
        } else {
            handler(argv.as_mut_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn time_watcher_init(
    loop_: *mut Loop,
    watcher: *mut TimeWatcher,
    data: *mut c_void,
) {
    unsafe {
        uv_timer_init(loop_.cast(), watcher.cast());
        *watcher.cast::<*mut c_void>() = watcher;
        let fields = watcher_fields(watcher);
        (*fields).data = data;
        (*fields).events = loop_fast_events(loop_);
        (*fields).blockable = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn time_watcher_start(
    watcher: *mut TimeWatcher,
    cb: time_cb,
    timeout: u64,
    repeat: u64,
) {
    unsafe {
        let fields = watcher_fields(watcher);
        (*fields).cb = cb;
        uv_timer_start(watcher.cast(), Some(time_watcher_cb), timeout, repeat);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn time_watcher_stop(watcher: *mut TimeWatcher) {
    unsafe {
        uv_timer_stop(watcher.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn time_watcher_close(watcher: *mut TimeWatcher, cb: time_cb) {
    unsafe {
        let fields = watcher_fields(watcher);
        (*fields).close_cb = cb;
        uv_close(watcher.cast(), Some(close_cb));
    }
}

unsafe extern "C" fn time_event(argv: *mut *mut c_void) {
    unsafe {
        let watcher = *argv;
        let fields = watcher_fields(watcher);
        if let Some(cb) = (*fields).cb {
            cb(watcher, (*fields).data);
        }
    }
}

unsafe extern "C" fn time_watcher_cb(handle: *mut uv_timer_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        let fields = watcher_fields(watcher);
        if (*fields).blockable && !multiqueue_empty((*fields).events) {
            return;
        }
        create_event((*fields).events, time_event, watcher);
    }
}

unsafe extern "C" fn close_event(argv: *mut *mut c_void) {
    unsafe {
        let watcher = *argv;
        let fields = watcher_fields(watcher);
        if let Some(cb) = (*fields).close_cb {
            cb(watcher, (*fields).data);
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        let fields = watcher_fields(watcher);
        if (*fields).close_cb.is_some() {
            create_event((*fields).events, close_event, watcher);
        }
    }
}
