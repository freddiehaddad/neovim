//! Owned libuv timers with reentrancy-safe dispatch and deferred close cleanup.

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_get_loop, uv_handle_size, uv_handle_t, uv_timer_init, uv_timer_start,
    uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_empty, multiqueue_put_event};
use crate::event::timer::{TIMER_HANDLE_WORDS, TimerCloseEvent, TimerHandle};
use std::cell::Cell;
use std::ffi::c_void;

const UV_TIMER: i32 = 13;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimeWatcherData {
    None,
    EvalTimer(u64),
    WaitDummy,
    TerminalRefresh,
}

pub struct TimeWatcherState {
    data: Cell<TimeWatcherData>,
    events: Cell<MultiQueue>,
    blockable: Cell<bool>,
}

#[derive(Clone)]
pub struct TimeWatcher {
    timer: TimerHandle<TimeWatcherState>,
}

pub type TimeWatcherCloseEvent = TimerCloseEvent<TimeWatcherState>;

impl TimeWatcher {
    pub fn new() -> Self {
        Self {
            timer: TimerHandle::new(TimeWatcherState {
                data: Cell::new(TimeWatcherData::None),
                events: Cell::new(MultiQueue::NULL),
                blockable: Cell::new(false),
            }),
        }
    }

    pub fn events(&self) -> MultiQueue {
        self.timer.data().events.get()
    }

    pub fn set_events(&self, events: MultiQueue) {
        self.timer.data().events.set(events);
    }

    pub fn set_blockable(&self, blockable: bool) {
        self.timer.data().blockable.set(blockable);
    }

    unsafe fn from_callback(handle: *mut c_void) -> Self {
        Self {
            timer: unsafe { TimerHandle::from_callback(handle) },
        }
    }
}

impl Default for TimeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn loop_fast_events(loop_: &Loop) -> MultiQueue {
    loop_.fast_events.get()
}

#[must_use]
pub struct WaitTimer(TimeWatcher);

impl WaitTimer {
    pub fn start(loop_: &Loop, interval: u64) -> Self {
        let watcher = TimeWatcher::new();
        time_watcher_init(loop_, &watcher, TimeWatcherData::WaitDummy);
        time_watcher_start(&watcher, interval, interval);
        Self(watcher)
    }

    pub fn stop_and_close(self) {
        drop(self);
    }
}

impl Drop for WaitTimer {
    fn drop(&mut self) {
        time_watcher_stop(&self.0);
        time_watcher_close(&self.0);
    }
}

fn initialize<T: 'static>(loop_: &Loop, timer: &TimerHandle<T>, close: unsafe fn(*mut c_void)) {
    assert!(timer.can_initialize());
    unsafe {
        assert!(uv_handle_size(UV_TIMER) <= TIMER_HANDLE_WORDS * size_of::<u64>());
        assert_eq!(uv_timer_init(loop_.uv_ptr(), timer.as_ptr()), 0);
        timer.register();
    }
    loop_.register_handle(timer.as_ptr(), close);
}

unsafe fn unregister(handle: *mut uv_handle_t) {
    let loop_ = unsafe { &*uv_handle_get_loop(handle).cast::<*const Loop>().read() };
    loop_.unregister_handle(handle);
}

pub fn time_watcher_init(loop_: &Loop, watcher: &TimeWatcher, data: TimeWatcherData) {
    initialize(loop_, &watcher.timer, close_registered);
    watcher.timer.data().data.set(data);
    watcher.set_events(loop_fast_events(loop_));
    watcher.set_blockable(false);
}

pub fn time_watcher_start(watcher: &TimeWatcher, timeout: u64, repeat: u64) {
    if !watcher.timer.is_active() {
        return;
    }
    unsafe {
        crate::event::r#loop::uv_update_time(uv_handle_get_loop(watcher.timer.as_ptr()));
        uv_timer_start(
            watcher.timer.as_ptr(),
            Some(time_watcher_cb),
            timeout,
            repeat,
        );
    }
}

pub fn time_watcher_stop(watcher: &TimeWatcher) {
    if watcher.timer.is_active() {
        unsafe { uv_timer_stop(watcher.timer.as_ptr()) };
    }
}

pub fn time_watcher_close(watcher: &TimeWatcher) {
    if watcher.timer.begin_close() {
        unsafe { uv_close(watcher.timer.as_ptr(), Some(close_cb)) };
    }
}

pub fn time_event(watcher: TimeWatcher) {
    match watcher.timer.data().data.get() {
        TimeWatcherData::None => {}
        TimeWatcherData::EvalTimer(timer_id) => crate::eval::timer_due_cb(timer_id),
        TimeWatcherData::WaitDummy => crate::eval::funcs::dummy_timer_due_cb(&watcher),
        TimeWatcherData::TerminalRefresh => crate::terminal::refresh_timer_cb(),
    }
}

unsafe extern "C" fn time_watcher_cb(handle: *mut uv_timer_t) {
    let watcher = unsafe { TimeWatcher::from_callback(handle) };
    let events = watcher.events();
    if watcher.timer.data().blockable.get() && !multiqueue_empty(events) {
        return;
    }
    if let Some(events) = events.opt() {
        multiqueue_put_event(events, Event::TimeWatcher(watcher));
    } else {
        time_event(watcher);
    }
}

fn cleanup_watcher(state: &TimeWatcherState) {
    if let TimeWatcherData::EvalTimer(timer_id) = state.data.get() {
        crate::eval::timer_close_cb(timer_id);
    }
}

unsafe fn close_registered(handle: *mut c_void) {
    time_watcher_close(&unsafe { TimeWatcher::from_callback(handle) });
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    let watcher = unsafe { TimeWatcher::from_callback(handle) };
    let events = watcher.events();
    let deferred = matches!(
        watcher.timer.data().data.get(),
        TimeWatcherData::EvalTimer(_) | TimeWatcherData::WaitDummy
    );
    unsafe {
        unregister(handle);
        watcher.timer.finish_close();
    }
    if deferred {
        let event = TimerCloseEvent::new(watcher.timer, cleanup_watcher);
        if let Some(events) = events.opt() {
            multiqueue_put_event(events, Event::TimeWatcherClose(event));
        } else {
            drop(event);
        }
    }
}

type RawTimerHandle = TimerHandle<Cell<Option<fn()>>>;

#[must_use]
pub struct UvRawTimer {
    timer: RawTimerHandle,
}

impl UvRawTimer {
    pub fn new() -> Self {
        Self {
            timer: TimerHandle::new(Cell::new(None)),
        }
    }

    pub fn init(&self, loop_: &Loop) {
        initialize(loop_, &self.timer, raw_close_registered);
    }

    pub fn start(&self, cb: fn(), timeout: u64, repeat: u64) {
        if !self.timer.is_active() {
            return;
        }
        self.timer.data().set(Some(cb));
        unsafe {
            uv_timer_start(self.timer.as_ptr(), Some(uv_raw_timer_cb), timeout, repeat);
        }
    }

    pub fn stop(&self) {
        if self.timer.is_active() {
            unsafe { uv_timer_stop(self.timer.as_ptr()) };
        }
    }

    pub fn close(&self) {
        close_raw_timer(&self.timer);
    }

    pub fn due_in(&self) -> u64 {
        if !self.timer.is_active() {
            return 0;
        }
        unsafe { crate::event::r#loop::uv_timer_get_due_in(self.timer.as_ptr()) }
    }
}

impl Default for UvRawTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for UvRawTimer {
    fn drop(&mut self) {
        self.close();
    }
}

fn close_raw_timer(timer: &RawTimerHandle) {
    if timer.begin_close() {
        unsafe { uv_close(timer.as_ptr(), Some(raw_close_cb)) };
    }
}

unsafe fn raw_close_registered(handle: *mut c_void) {
    close_raw_timer(&unsafe { RawTimerHandle::from_callback(handle) });
}

unsafe extern "C" fn raw_close_cb(handle: *mut uv_handle_t) {
    let timer = unsafe { RawTimerHandle::from_callback(handle) };
    unsafe {
        unregister(handle);
        timer.finish_close();
    }
}

unsafe extern "C" fn uv_raw_timer_cb(handle: *mut uv_timer_t) {
    let timer = unsafe { RawTimerHandle::from_callback(handle) };
    let callback = timer.data().get();
    if let Some(callback) = callback {
        callback();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#loop::{loop_poll_events, main_loop, uv_is_closing};
    use crate::event::multiqueue::multiqueue_cancel_events;
    use std::cell::RefCell;
    use std::rc::Rc;

    thread_local! {
        static RAW_TIMER: RefCell<Option<UvRawTimer>> = const { RefCell::new(None) };
        static RAW_FIRED: Cell<bool> = const { Cell::new(false) };
    }

    #[test]
    fn raw_timer_survives_a_move_and_its_callback_dropping_the_owner() {
        fn close_self() {
            let timer = RAW_TIMER.with(|slot| slot.borrow_mut().take().unwrap());
            let native = timer.timer.clone();
            drop(timer);
            assert_ne!(unsafe { uv_is_closing(native.as_ptr()) }, 0);
            RAW_FIRED.set(true);
        }

        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _editor = crate::main_init::test_editor();
        let timer = UvRawTimer::new();
        timer.init(main_loop());
        assert!(RAW_TIMER.with(|slot| slot.replace(Some(timer))).is_none());
        RAW_TIMER.with(|slot| slot.borrow().as_ref().unwrap().start(close_self, 0, 0));
        loop_poll_events(main_loop(), 0);
        assert!(RAW_FIRED.get());
        assert!(RAW_TIMER.with(|slot| slot.borrow().is_none()));
    }

    #[test]
    fn dropping_wait_guard_requests_native_close() {
        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _editor = crate::main_init::test_editor();
        let guard = WaitTimer::start(main_loop(), 60000);
        let watcher = guard.0.clone();
        drop(guard);
        assert_ne!(unsafe { uv_is_closing(watcher.timer.as_ptr()) }, 0);
        loop_poll_events(main_loop(), 0);
        assert!(!watcher.timer.is_active());
    }

    #[test]
    fn cancelled_close_notification_releases_the_vim_timer() {
        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _editor = crate::main_init::test_editor();
        let id = crate::eval::timer_start(60000, 1, crate::eval::typval::Callback::NONE);
        let timer = crate::eval::find_timer_by_nr(id as i64).unwrap();
        let weak = Rc::downgrade(&timer);
        let events = timer.watcher.events();
        crate::eval::timer_stop(&timer);
        drop(timer);
        loop_poll_events(main_loop(), 0);
        assert!(weak.upgrade().is_some());
        multiqueue_cancel_events(events);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn editor_shutdown_releases_stopped_timer_registry_owners() {
        let _lock = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let weak = {
            let _editor = crate::main_init::test_editor();
            let id = crate::eval::timer_start(60000, -1, crate::eval::typval::Callback::NONE);
            let timer = crate::eval::find_timer_by_nr(id as i64).unwrap();
            Rc::downgrade(&timer)
        };
        assert!(weak.upgrade().is_none());
    }
}
