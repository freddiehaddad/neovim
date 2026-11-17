//! Owned libuv signal watchers with immutable callback routing.

use crate::event::r#loop::{
    Loop, uv_close, uv_handle_size, uv_handle_t, uv_signal_init, uv_signal_start, uv_signal_stop,
    uv_signal_t,
};
use crate::event::multiqueue::{Event, MultiQueue, multiqueue_put_event};
use crate::event::owned::SharedCallbackHandle;
use crate::os::fs::UV_EINVAL;
use std::ffi::c_void;

pub(crate) const SIGNAL_HANDLE_WORDS: usize = 40;
const UV_SIGNAL: i32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SignalTarget {
    Editor,
    Tui,
}

struct SignalState {
    loop_: &'static Loop,
    events: MultiQueue,
    target: SignalTarget,
}

type SignalHandle = SharedCallbackHandle<SignalState, SIGNAL_HANDLE_WORDS>;

#[must_use]
pub struct SignalWatcher {
    handle: SignalHandle,
}

impl SignalWatcher {
    pub fn new(
        loop_: &'static Loop,
        events: MultiQueue,
        target: SignalTarget,
    ) -> Result<Self, i32> {
        let handle = SignalHandle::new(SignalState {
            loop_,
            events,
            target,
        });
        let result = unsafe {
            assert!(uv_handle_size(UV_SIGNAL) <= SIGNAL_HANDLE_WORDS * size_of::<u64>());
            uv_signal_init(loop_.uv_ptr(), handle.as_ptr())
        };
        if result != 0 {
            return Err(result);
        }
        unsafe { handle.register() };
        loop_.register_handle(handle.as_ptr(), close_registered);
        Ok(Self { handle })
    }

    pub fn start(&self, signum: i32) -> Result<(), i32> {
        if !self.handle.is_active() {
            return Err(UV_EINVAL);
        }
        let result =
            unsafe { uv_signal_start(self.handle.as_ptr(), Some(signal_watcher_cb), signum) };
        if result == 0 { Ok(()) } else { Err(result) }
    }

    pub fn stop(&self) {
        if self.handle.is_active() {
            unsafe { uv_signal_stop(self.handle.as_ptr()) };
        }
    }

    pub fn close(&self) {
        close_handle(&self.handle);
    }
}

impl Drop for SignalWatcher {
    fn drop(&mut self) {
        self.close();
    }
}

fn close_handle(handle: &SignalHandle) {
    if handle.begin_close() {
        unsafe { uv_close(handle.as_ptr(), Some(close_cb)) };
    }
}

unsafe fn close_registered(handle: *mut c_void) {
    close_handle(&unsafe { SignalHandle::from_callback(handle) });
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    let owner = unsafe { SignalHandle::from_callback(handle) };
    owner.data().loop_.unregister_handle(handle);
    unsafe { owner.finish_close() };
}

pub fn signal_event(signum: i32, is_tui: bool) {
    if !is_tui {
        crate::os::signal::on_signal(signum);
    } else {
        crate::ui_client::with_tui_mut(crate::tui::tui::sigwinch_cb);
    }
}

unsafe extern "C" fn signal_watcher_cb(handle: *mut uv_signal_t, signum: i32) {
    let owner = unsafe { SignalHandle::from_callback(handle) };
    let (events, is_tui) = {
        let data = owner.data();
        (data.events, data.target == SignalTarget::Tui)
    };
    if let Some(events) = events.opt() {
        multiqueue_put_event(events, Event::Signal { signum, is_tui });
    } else {
        signal_event(signum, is_tui);
    }
}
