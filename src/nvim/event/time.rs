//! Timer watcher wrapper around libuv `uv_timer_t` (`src/nvim/event/time.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_t, uv_timer_init, uv_timer_start, uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_empty, multiqueue_put_event};
use std::ptr;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimeWatcherData {
    None,
    EvalTimer(*mut crate::eval::VimTimer),
    WaitDummy,
    TerminalRefresh,
}

#[repr(C)]
pub struct TimeWatcher {
    pub uv: [u64; 32],
    pub data: TimeWatcherData,
    pub events: *mut MultiQueue,
    pub blockable: bool,
}

impl TimeWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 32],
            data: TimeWatcherData::None,
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
pub fn loop_fast_events(loop_: &mut Loop) -> *mut MultiQueue {
    loop_.fast_events
}

pub fn time_watcher_init(loop_: &mut Loop, watcher: &mut TimeWatcher, data: TimeWatcherData) {
    unsafe {
        uv_timer_init((&raw mut loop_.uv).cast(), (&raw mut watcher.uv).cast());
        *(&raw mut watcher.uv).cast::<*mut TimeWatcher>() = watcher;
        watcher.data = data;
        watcher.events = loop_fast_events(loop_);
        watcher.blockable = false;
    }
}

pub fn time_watcher_start(watcher: &mut TimeWatcher, timeout: u64, repeat: u64) {
    {
        unsafe {
            uv_timer_start(
                (&raw mut watcher.uv).cast(),
                Some(time_watcher_cb),
                timeout,
                repeat,
            )
        };
    }
}

pub fn time_watcher_stop(watcher: &mut TimeWatcher) {
    unsafe {
        uv_timer_stop((&raw mut watcher.uv).cast());
    }
}

pub fn time_watcher_close(watcher: &mut TimeWatcher) {
    unsafe {
        uv_close((&raw mut watcher.uv).cast(), Some(close_cb));
    }
}

pub fn time_event(watcher: &mut TimeWatcher) {
    {
        match watcher.data {
            TimeWatcherData::None => {}
            TimeWatcherData::EvalTimer(timer) => {
                crate::eval::timer_due_cb(watcher, unsafe { &mut *timer })
            }
            TimeWatcherData::WaitDummy => crate::eval::funcs::dummy_timer_due_cb(watcher),
            TimeWatcherData::TerminalRefresh => crate::terminal::refresh_timer_cb(),
        }
    }
}

unsafe extern "C" fn time_watcher_cb(handle: *mut uv_timer_t) {
    unsafe {
        {
            {
                let watcher = *handle.cast::<*mut TimeWatcher>();
                let watcher = { &mut *watcher };
                if watcher.blockable && !multiqueue_empty(&*watcher.events) {
                    return;
                }
                if !watcher.events.is_null() {
                    multiqueue_put_event(&mut *watcher.events, Event::TimeWatcher(watcher));
                } else {
                    time_event(&mut *watcher);
                }
            }
        }
    }
}

pub fn close_event(watcher: &mut TimeWatcher) {
    unsafe {
        match watcher.data {
            TimeWatcherData::None | TimeWatcherData::TerminalRefresh => {}
            TimeWatcherData::EvalTimer(timer) => crate::eval::timer_close_cb(watcher, &mut *timer),
            TimeWatcherData::WaitDummy => drop(Box::from_raw(watcher)),
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        {
            let watcher = { *handle.cast::<*mut TimeWatcher>() };
            let watcher = { &mut *watcher };
            if matches!(
                watcher.data,
                TimeWatcherData::EvalTimer(_) | TimeWatcherData::WaitDummy
            ) {
                if !watcher.events.is_null() {
                    multiqueue_put_event(&mut *watcher.events, Event::TimeWatcherClose(watcher));
                } else {
                    close_event(&mut *watcher);
                }
            }
        }
    }
}
