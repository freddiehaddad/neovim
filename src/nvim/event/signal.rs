//! Signal watcher wrapper around libuv `uv_signal_t` (`src/nvim/event/signal.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::event::r#loop::{uv_close, uv_handle_size, uv_loop_size, uv_signal_init};
#[allow(unused_imports)]
use crate::event::r#loop::{uv_signal_start, uv_signal_stop};
use std::ffi::{c_int, c_void};
use std::ptr;

use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_put_event};

const UV_HANDLE: c_int = 5;
const UV_SIGNAL: c_int = 16;

pub use crate::event::r#loop::{
    Loop, uv_close_cb, uv_handle_t, uv_loop_t, uv_signal_cb, uv_signal_t,
};
pub type SignalWatcher = c_void;

pub type signal_cb =
    Option<unsafe extern "C" fn(watcher: *mut SignalWatcher, signum: c_int, data: *mut c_void)>;
pub type signal_close_cb =
    Option<unsafe extern "C" fn(watcher: *mut SignalWatcher, data: *mut c_void)>;

#[repr(C)]
struct SignalWatcherFields {
    data: *mut c_void,
    cb: signal_cb,
    close_cb: signal_close_cb,
    events: *mut MultiQueue,
}

#[inline]
unsafe fn watcher_fields(watcher: *mut SignalWatcher) -> *mut SignalWatcherFields {
    unsafe {
        watcher
            .cast::<u8>()
            .add(uv_handle_size(UV_SIGNAL))
            .cast::<SignalWatcherFields>()
    }
}

#[inline]
unsafe fn watcher_signum(watcher: *mut SignalWatcher) -> c_int {
    unsafe {
        *watcher
            .cast::<u8>()
            .add(uv_handle_size(UV_HANDLE) + std::mem::size_of::<*const c_void>())
            .cast::<c_int>()
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_watcher_init(
    loop_: *mut Loop,
    watcher: *mut SignalWatcher,
    data: *mut c_void,
) {
    unsafe {
        uv_signal_init(loop_.cast(), watcher.cast());
        *watcher.cast::<*mut c_void>() = watcher;
        let fields = watcher_fields(watcher);
        (*fields).data = data;
        (*fields).cb = None;
        (*fields).events = loop_fast_events(loop_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_watcher_start(
    watcher: *mut SignalWatcher,
    cb: signal_cb,
    signum: c_int,
) {
    unsafe {
        let fields = watcher_fields(watcher);
        (*fields).cb = cb;
        uv_signal_start(watcher.cast(), Some(signal_watcher_cb), signum);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_watcher_stop(watcher: *mut SignalWatcher) {
    unsafe {
        uv_signal_stop(watcher.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn signal_watcher_close(watcher: *mut SignalWatcher, cb: signal_close_cb) {
    unsafe {
        let fields = watcher_fields(watcher);
        (*fields).close_cb = cb;
        uv_close(watcher.cast(), Some(close_cb));
    }
}

unsafe extern "C" fn signal_event(argv: *mut *mut c_void) {
    unsafe {
        let watcher = *argv;
        let fields = watcher_fields(watcher);
        if let Some(cb) = (*fields).cb {
            cb(watcher, watcher_signum(watcher), (*fields).data);
        }
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, _signum: c_int) {
    unsafe {
        let watcher = *handle.cast::<*mut SignalWatcher>();
        let fields = watcher_fields(watcher);
        let events = (*fields).events;
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = watcher;
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
        let fields = watcher_fields(watcher);
        if let Some(cb) = (*fields).close_cb {
            cb(watcher, (*fields).data);
        }
    }
}
