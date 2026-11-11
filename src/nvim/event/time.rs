//! Timer watcher wrapper around libuv `uv_timer_t` (`src/nvim/event/time.c`).

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_t, uv_timer_init, uv_timer_start, uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_empty, multiqueue_put_event};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimeWatcherData {
    None,
    EvalTimer(u64),
    WaitDummy,
    TerminalRefresh,
}

#[repr(C)]
pub struct TimeWatcher {
    pub uv: [u64; 32],
    pub data: TimeWatcherData,
    pub events: crate::globals::GraphPtr<MultiQueue>,
    pub blockable: bool,
}

impl TimeWatcher {
    pub const fn new() -> Self {
        Self {
            uv: [0; 32],
            data: TimeWatcherData::None,
            events: crate::globals::GraphPtr::NULL,
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
pub fn loop_fast_events(loop_: &mut Loop) -> Option<&'static mut MultiQueue> {
    loop_.fast_events.opt_mut()
}

pub fn time_watcher_init(loop_: &mut Loop, watcher: &mut TimeWatcher, data: TimeWatcherData) {
    unsafe {
        uv_timer_init((&raw mut loop_.uv).cast(), (&raw mut watcher.uv).cast());
        *(&raw mut watcher.uv).cast::<*mut TimeWatcher>() = watcher;
        watcher.data = data;
        watcher.events = crate::globals::GraphPtr::from(loop_fast_events(loop_));
        watcher.blockable = false;
    }
}

pub fn time_watcher_start(watcher: &mut TimeWatcher, timeout: u64, repeat: u64) {
    unsafe {
        crate::event::r#loop::uv_update_time(crate::event::r#loop::uv_handle_get_loop(
            (&raw const watcher.uv).cast(),
        ));
        uv_timer_start(
            (&raw mut watcher.uv).cast(),
            Some(time_watcher_cb),
            timeout,
            repeat,
        )
    };
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

pub fn time_event(watcher_in: &mut TimeWatcher) {
    let watcher = crate::globals::GraphPtr::<TimeWatcher>::of(watcher_in).as_ptr();
    unsafe {
        match (*watcher).data {
            TimeWatcherData::None => {}
            TimeWatcherData::EvalTimer(timer_id) => {
                crate::eval::timer_due_cb(&mut *watcher, timer_id);
            }
            TimeWatcherData::WaitDummy => crate::eval::funcs::dummy_timer_due_cb(&mut *watcher),
            TimeWatcherData::TerminalRefresh => crate::terminal::refresh_timer_cb(),
        }
    }
}

unsafe extern "C" fn time_watcher_cb(handle: *mut uv_timer_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        if (*watcher).blockable && !multiqueue_empty((*watcher).events.borrow()) {
            return;
        }
        if let Some(events) = (*watcher).events.opt_mut() {
            multiqueue_put_event(events, Event::TimeWatcher(watcher.into()));
        } else {
            time_event(&mut *watcher);
        }
    }
}

pub fn close_event(watcher_in: &mut TimeWatcher) {
    let watcher = crate::globals::GraphPtr::<TimeWatcher>::of(watcher_in).as_ptr();
    unsafe {
        match (*watcher).data {
            TimeWatcherData::None | TimeWatcherData::TerminalRefresh => {}
            TimeWatcherData::EvalTimer(timer_id) => {
                crate::eval::timer_close_cb(&mut *watcher, timer_id);
            }
            TimeWatcherData::WaitDummy => drop(Box::from_raw(watcher)),
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let watcher = *handle.cast::<*mut TimeWatcher>();
        if matches!(
            (*watcher).data,
            TimeWatcherData::EvalTimer(_) | TimeWatcherData::WaitDummy
        ) {
            if let Some(events) = (*watcher).events.opt_mut() {
                multiqueue_put_event(events, Event::TimeWatcherClose(watcher.into()));
            } else {
                close_event(&mut *watcher);
            }
        }
    }
}

#[repr(C)]
pub struct UvRawTimer<T> {
    handle: [u64; 32],
    data: *mut T,
    cb: Option<fn(&mut T)>,
}

impl<T> UvRawTimer<T> {
    pub const fn new() -> Self {
        Self {
            handle: [0; 32],
            data: std::ptr::null_mut(),
            cb: None,
        }
    }

    pub fn init(&mut self, loop_: &mut Loop, data: &mut T) {
        self.data = std::ptr::from_mut(data);
        let th: *mut uv_timer_t = (&raw mut self.handle).cast();
        let self_ptr: *mut Self = self;
        unsafe {
            uv_timer_init((&raw mut *loop_).cast(), th);
            *th.cast::<*mut Self>() = self_ptr;
        }
    }

    pub fn start(&mut self, cb: fn(&mut T), timeout: u64, repeat: u64) {
        self.cb = Some(cb);
        let th: *mut uv_timer_t = (&raw mut self.handle).cast();
        unsafe {
            uv_timer_start(th, Some(uv_raw_timer_cb::<T>), timeout, repeat);
        }
    }

    pub fn stop(&mut self) {
        let th: *mut uv_timer_t = (&raw mut self.handle).cast();
        unsafe {
            uv_timer_stop(th);
        }
    }

    pub fn close(&mut self) {
        let th: *mut uv_handle_t = (&raw mut self.handle).cast();
        unsafe {
            uv_close(th, None);
        }
    }

    pub fn due_in(&mut self) -> u64 {
        let th: *mut uv_timer_t = (&raw mut self.handle).cast();
        unsafe { crate::event::r#loop::uv_timer_get_due_in(th) }
    }
}

impl<T> Default for UvRawTimer<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" fn uv_raw_timer_cb<T>(handle: *mut uv_timer_t) {
    unsafe {
        let timer = &mut **handle.cast::<*mut UvRawTimer<T>>();
        if let Some(cb) = timer.cb {
            cb(&mut *timer.data);
        }
    }
}
