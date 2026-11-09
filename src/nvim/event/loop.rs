//! Event loop wrapper around `uv_loop_t` (`src/nvim/event/loop.c`).

pub use crate::event::ffi::*;
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_free, multiqueue_move_events, multiqueue_new,
    multiqueue_new_child, multiqueue_process_events, multiqueue_purge_events, multiqueue_put_event,
    multiqueue_size,
};
use crate::log::{LOGLVL_ERR, log_uv_handles, logmsg};
use crate::os::time::os_hrtime;
use std::ffi::c_void;
use std::ptr;

#[repr(C)]
pub struct Loop {
    pub uv: [u64; 192],
    pub events: crate::globals::GraphPtr<MultiQueue>,
    pub thread_events: crate::globals::GraphPtr<MultiQueue>,
    pub fast_events: crate::globals::GraphPtr<MultiQueue>,
    pub children: Vec<crate::globals::GraphPtr<crate::event::proc::Proc>>,
    pub children_watcher: [u64; 40],
    pub children_kill_timer: [u64; 32],
    pub poll_timer: [u64; 32],
    pub exit_delay_timer: [u64; 32],
    pub async_: [u64; 32],
    pub mutex: [u64; 16],
    pub recursive: i32,
    pub closing: bool,
    pub timeout_expired: bool,
}

impl Loop {
    pub const fn new() -> Self {
        Self {
            uv: [0; 192],
            events: crate::globals::GraphPtr::NULL,
            thread_events: crate::globals::GraphPtr::NULL,
            fast_events: crate::globals::GraphPtr::NULL,
            children: Vec::new(),
            children_watcher: [0; 40],
            children_kill_timer: [0; 32],
            poll_timer: [0; 32],
            exit_delay_timer: [0; 32],
            async_: [0; 32],
            mutex: [0; 16],
            recursive: 0,
            closing: false,
            timeout_expired: false,
        }
    }
}

#[inline]
pub fn loop_queues(loop_: &mut Loop) -> &'static mut Loop {
    let p = crate::globals::GraphPtr::<Loop>::from_ref(loop_);
    p.borrow_mut()
}

#[inline]
pub fn main_loop_ptr() -> &'static mut Loop {
    &mut crate::globals::g_st().borrow_mut().main_loop
}

#[inline]
pub fn main_loop_events() -> &'static mut MultiQueue {
    crate::globals::g_st().main_loop.events.borrow_mut()
}

#[inline]
pub fn loop_children_watcher(loop_: &mut Loop) -> &mut uv_signal_t {
    unsafe { &mut *(&raw mut loop_.children_watcher).cast() }
}

#[inline]
pub fn loop_children_kill_timer(loop_: &mut Loop) -> &mut uv_timer_t {
    unsafe { &mut *(&raw mut loop_.children_kill_timer).cast() }
}

#[inline]
pub fn loop_poll_timer(loop_: &mut Loop) -> &mut uv_timer_t {
    unsafe { &mut *(&raw mut loop_.poll_timer).cast() }
}

#[inline]
pub fn loop_exit_delay_timer(loop_: &mut Loop) -> &mut uv_timer_t {
    unsafe { &mut *(&raw mut loop_.exit_delay_timer).cast() }
}

#[inline]
pub fn loop_async(loop_: &mut Loop) -> &mut uv_async_t {
    unsafe { &mut *(&raw mut loop_.async_).cast() }
}

#[inline]
pub fn loop_mutex(loop_: &mut Loop) -> &mut uv_mutex_t {
    unsafe { &mut *(&raw mut loop_.mutex).cast() }
}

pub fn loop_init(loop_: &mut Loop) {
    unsafe {
        debug_assert!(uv_loop_size() <= size_of::<[u64; 192]>());
        let uv_ptr: *mut uv_loop_t = (&raw mut loop_.uv).cast();
        uv_loop_init(uv_ptr);
        loop_.recursive = 0;
        loop_.closing = false;
        *uv_ptr.cast::<*mut Loop>() = &raw mut *loop_;

        loop_.children.clear();
        loop_.events = crate::globals::GraphPtr::from(multiqueue_new(Some(&mut *loop_)));
        loop_.fast_events =
            crate::globals::GraphPtr::from(multiqueue_new_child(loop_.events.borrow_mut()));
        loop_.thread_events = crate::globals::GraphPtr::from(multiqueue_new(None));

        uv_mutex_init(loop_mutex(loop_));
        uv_async_init(uv_ptr, loop_async(loop_), Some(async_cb));
        uv_signal_init(uv_ptr, loop_children_watcher(loop_));
        uv_timer_init(uv_ptr, loop_children_kill_timer(loop_));
        let poll_timer = ptr::from_mut(loop_poll_timer(loop_));
        uv_timer_init(uv_ptr, poll_timer);
        uv_timer_init(uv_ptr, loop_exit_delay_timer(loop_));
        loop_.timeout_expired = false;
        *poll_timer.cast::<*mut bool>() = &raw mut loop_.timeout_expired;
    }
}

/// Process `Loop.uv` events with a timeout.
fn loop_uv_run(loop_in: &mut Loop, ms: i64) -> bool {
    let mut loop_ = crate::globals::GraphPtr::<Loop>::from_ref(loop_in);
    unsafe {
        let old_rec = loop_.recursive;
        loop_.recursive = old_rec + 1;
        if old_rec != 0 {
            std::process::abort(); // Should not re-enter uv_run
        }

        let mut mode: i32 = UV_RUN_ONCE;
        let poll_timer = ptr::from_mut(loop_poll_timer(loop_.borrow_mut()));
        let timeout_expired = *poll_timer.cast::<*mut bool>();
        *timeout_expired = false;

        if ms > 0 {
            uv_timer_start(poll_timer, Some(timer_cb), ms as u64, ms as u64);
        } else if ms == 0 {
            mode = UV_RUN_NOWAIT;
        }

        uv_run((&raw mut loop_.uv).cast(), mode);

        if ms > 0 {
            uv_timer_stop(poll_timer);
        }

        loop_.recursive -= 1; // Can re-enter uv_run now
        ptr::read_volatile(timeout_expired)
    }
}

/// Processes one `Loop.uv` event (at most).
/// Processes all `Loop.fast_events` events.
/// Does NOT process `Loop.events`, that is an application-specific decision.
pub fn loop_poll_events(loop_in: &mut Loop, ms: i64) -> bool {
    let loop_ = crate::globals::GraphPtr::<Loop>::from_ref(loop_in);
    let timeout_expired = loop_uv_run(loop_.borrow_mut(), ms);
    multiqueue_process_events(loop_.fast_events.borrow_mut());
    timeout_expired
}

/// Schedules a fast event from another thread.
pub fn loop_schedule_fast(loop_: &mut Loop, event: Event) {
    unsafe {
        let mutex = ptr::from_mut(loop_mutex(loop_));
        uv_mutex_lock(mutex);
        multiqueue_put_event(loop_.thread_events.borrow_mut(), event);
        uv_async_send(loop_async(loop_));
        uv_mutex_unlock(mutex);
    }
}

/// Schedules an event from another thread. Unlike loop_schedule_fast(), the
/// event is forwarded to `Loop.events`, instead of being processed immediately.
pub fn loop_schedule_deferred(loop_: &mut Loop, event: Event) {
    let loop_ptr = &raw mut *loop_;
    loop_schedule_fast(
        loop_,
        Event::LoopDeferred {
            loop_: loop_ptr.into(),
            event: Box::new(event),
        },
    );
}

pub fn loop_deferred_event(loop_: &mut Loop, event: Event) {
    multiqueue_put_event(loop_.events.borrow_mut(), event);
}

pub fn loop_on_put(loop_: &mut Loop) {
    if loop_.recursive != 0 {
        unsafe { uv_stop((&raw mut loop_.uv).cast()) };
    }
}

unsafe extern "C" fn loop_walk_cb(handle: *mut uv_handle_t, _arg: *mut c_void) {
    unsafe {
        if uv_is_closing(handle) == 0 {
            uv_close(handle, None);
        }
    }
}

/// Closes `loop` and its handles, and frees its structures.
pub fn loop_close(loop_: &mut Loop, wait: bool) -> bool {
    let mut rv = true;
    loop_.closing = true;
    unsafe { uv_mutex_destroy(loop_mutex(loop_)) };
    unsafe { uv_close(ptr::from_mut(loop_children_watcher(loop_)).cast(), None) };
    unsafe { uv_close(ptr::from_mut(loop_children_kill_timer(loop_)).cast(), None) };
    unsafe { uv_close(ptr::from_mut(loop_poll_timer(loop_)).cast(), None) };
    unsafe { uv_close(ptr::from_mut(loop_exit_delay_timer(loop_)).cast(), None) };
    unsafe { uv_close(ptr::from_mut(loop_async(loop_)).cast(), None) };
    let start = if wait { os_hrtime() } else { 0 };
    let mut didstop = false;
    loop {
        unsafe {
            uv_run(
                (&raw mut loop_.uv).cast(),
                if didstop {
                    UV_RUN_DEFAULT
                } else {
                    UV_RUN_NOWAIT
                },
            )
        };
        if unsafe { uv_loop_close((&raw mut loop_.uv).cast()) } != UV_EBUSY || !wait {
            break;
        }
        let elapsed_s = (os_hrtime() - start) / NS_PER_SEC;
        if elapsed_s >= 2 {
            rv = false;
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"loop_close"),
                line!() as i32,
                true,
                c"uv_loop_close() hang?",
            );
            log_uv_handles(unsafe { &mut *(&raw mut loop_.uv).cast() });
            break;
        }
        if !didstop {
            unsafe { uv_stop((&raw mut loop_.uv).cast()) };
            unsafe {
                uv_walk(
                    (&raw mut loop_.uv).cast(),
                    Some(loop_walk_cb),
                    ptr::null_mut(),
                )
            };
            didstop = true;
        }
    }
    let qc = loop_queues(loop_);
    multiqueue_free(qc.fast_events.borrow_mut());
    multiqueue_free(qc.thread_events.borrow_mut());
    multiqueue_free(qc.events.borrow_mut());
    qc.children = Vec::new();
    rv
}

pub fn loop_purge(loop_: &mut Loop) {
    unsafe {
        let mutex = ptr::from_mut(loop_mutex(loop_));
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        multiqueue_purge_events(qc.thread_events.borrow_mut());
        multiqueue_purge_events(qc.fast_events.borrow_mut());
        uv_mutex_unlock(mutex);
    }
}

pub fn loop_size(loop_: &mut Loop) -> usize {
    unsafe {
        let mutex = ptr::from_mut(loop_mutex(loop_));
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        let rv = multiqueue_size(qc.thread_events.borrow());
        uv_mutex_unlock(mutex);
        rv
    }
}

unsafe extern "C" fn async_cb(handle: *mut uv_async_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let l = *uv_loop.cast::<*mut Loop>();
        let mutex = ptr::from_mut(loop_mutex(&mut *l));
        let qc = loop_queues(&mut *l);
        uv_mutex_lock(mutex);
        multiqueue_move_events(qc.fast_events.borrow_mut(), qc.thread_events.borrow_mut());
        uv_mutex_unlock(mutex);
    }
}

unsafe extern "C" fn timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let timeout_expired = *handle.cast::<*mut bool>();
        *timeout_expired = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::libuv_proc::UvProcessHandle;
    use crate::event::signal::SignalWatcher;
    use crate::event::socket::UvSocketHandle;
    use crate::event::stream::{UV_NAMED_PIPE, UV_TCP, UV_TTY, UvStreamHandle};
    use crate::event::time::TimeWatcher;

    #[test]
    fn uv_handle_and_loop_storage_sizes_are_sufficient() {
        const UV_ASYNC: i32 = 1;
        const UV_IDLE: i32 = 6;
        const UV_PROCESS: i32 = 10;
        const UV_TIMER: i32 = 13;
        const UV_SIGNAL: i32 = 16;

        unsafe {
            let loop_dummy = Loop::new();
            assert!(size_of_val(&loop_dummy.uv) >= uv_loop_size());
            assert!(size_of_val(&loop_dummy.children_watcher) >= uv_handle_size(UV_SIGNAL));
            assert!(size_of_val(&loop_dummy.children_kill_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.poll_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.exit_delay_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.async_) >= uv_handle_size(UV_ASYNC));

            let sig_dummy = SignalWatcher::new();
            assert!(size_of_val(&sig_dummy.uv) >= uv_handle_size(UV_SIGNAL));

            let time_dummy = TimeWatcher::new();
            assert!(size_of_val(&time_dummy.uv) >= uv_handle_size(UV_TIMER));

            let max_stream_size = uv_handle_size(UV_NAMED_PIPE)
                .max(uv_handle_size(UV_TCP))
                .max(uv_handle_size(UV_TTY))
                .max(uv_handle_size(UV_IDLE));
            assert!(size_of::<UvStreamHandle>() >= max_stream_size);
            assert!(size_of::<UvSocketHandle>() >= max_stream_size);
            assert!(size_of::<UvProcessHandle>() >= uv_handle_size(UV_PROCESS));
        }
    }
}
