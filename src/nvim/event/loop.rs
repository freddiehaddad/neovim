//! Event loop wrapper around `uv_loop_t` (`src/nvim/event/loop.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::eval::funcs::UV_TIMER;
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_free, multiqueue_move_events,
    multiqueue_new, multiqueue_new_child, multiqueue_process_events, multiqueue_purge_events,
    multiqueue_put_event, multiqueue_size,
};
use crate::log::{LOGLVL_ERR, log_uv_handles, logmsg};
use crate::memory::{xfree, xmalloc};
use crate::os::time::os_hrtime;
use std::ffi::{c_int, c_void};
use std::ptr;

const UV_ASYNC: c_int = 1;
pub(crate) const UV_SIGNAL: c_int = 16;

pub(crate) const UV_RUN_DEFAULT: c_int = 0;
const UV_RUN_ONCE: c_int = 1;
pub(crate) const UV_RUN_NOWAIT: c_int = 2;

#[cfg(windows)]
const UV_EBUSY: c_int = -4082;
#[cfg(not(windows))]
const UV_EBUSY: c_int = -16;

#[cfg(any(windows, target_os = "linux"))]
const UV_MUTEX_SIZE: usize = 40;
#[cfg(target_os = "macos")]
const UV_MUTEX_SIZE: usize = 64;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
const UV_MUTEX_SIZE: usize = 8;

const NS_PER_SEC: u64 = 1_000_000_000;

pub type Loop = c_void;
pub type uv_loop_t = c_void;
pub type uv_handle_t = c_void;
pub type uv_signal_t = c_void;
pub type uv_timer_t = c_void;
pub type uv_async_t = c_void;
pub type uv_mutex_t = c_void;
pub type uv_connect_t = c_void;
pub type uv_fs_t = c_void;
#[cfg(windows)]
pub type uv_uid_t = u8;
#[cfg(not(windows))]
pub type uv_uid_t = u32;
#[cfg(windows)]
pub type uv_gid_t = u8;
#[cfg(not(windows))]
pub type uv_gid_t = u32;
pub type uv_thread_t = usize;

pub type uv_async_cb = Option<unsafe extern "C" fn(handle: *mut uv_async_t)>;
pub type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;
pub type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;
pub type uv_walk_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t, arg: *mut c_void)>;
pub type uv_signal_cb = Option<unsafe extern "C" fn(handle: *mut uv_signal_t, signum: c_int)>;
pub type uv_connect_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: c_int)>;
pub type uv_write_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: c_int)>;
pub type uv_fs_cb = Option<unsafe extern "C" fn(req: *mut uv_fs_t)>;

#[repr(C)]
pub struct LoopQueuesAndChildren {
    pub events: *mut MultiQueue,
    pub thread_events: *mut MultiQueue,
    pub fast_events: *mut MultiQueue,
    pub children_size: usize,
    pub children_capacity: usize,
    pub children_items: *mut *mut c_void,
}

unsafe extern "C" {
    pub fn uv_loop_size() -> usize;
    pub fn uv_handle_size(h_type: c_int) -> usize;
    pub fn uv_req_size(r_type: c_int) -> usize;
    pub fn uv_handle_get_loop(handle: *const uv_handle_t) -> *mut uv_loop_t;
    pub fn uv_handle_get_type(handle: *const uv_handle_t) -> c_int;
    pub fn uv_guess_handle(file: c_int) -> c_int;
    pub fn uv_strerror(err: c_int) -> *const std::ffi::c_char;
    pub fn uv_err_name(err: c_int) -> *const std::ffi::c_char;
    pub fn uv_translate_sys_error(sys_errno: c_int) -> c_int;

    pub fn uv_loop_init(loop_: *mut uv_loop_t) -> c_int;
    pub fn uv_loop_close(loop_: *mut uv_loop_t) -> c_int;
    pub fn uv_run(loop_: *mut uv_loop_t, mode: c_int) -> c_int;
    pub fn uv_stop(loop_: *mut uv_loop_t);
    pub fn uv_walk(loop_: *mut uv_loop_t, walk_cb: uv_walk_cb, arg: *mut c_void);

    pub fn uv_mutex_init(handle: *mut uv_mutex_t) -> c_int;
    pub fn uv_mutex_destroy(handle: *mut uv_mutex_t);
    pub fn uv_mutex_lock(handle: *mut uv_mutex_t);
    pub fn uv_mutex_unlock(handle: *mut uv_mutex_t);

    pub fn uv_async_init(
        loop_: *mut uv_loop_t,
        async_: *mut uv_async_t,
        async_cb: uv_async_cb,
    ) -> c_int;
    pub fn uv_async_send(async_: *mut uv_async_t) -> c_int;

    pub fn uv_signal_init(loop_: *mut uv_loop_t, handle: *mut uv_signal_t) -> c_int;

    pub fn uv_timer_init(loop_: *mut uv_loop_t, handle: *mut uv_timer_t) -> c_int;
    pub fn uv_timer_start(
        handle: *mut uv_timer_t,
        cb: uv_timer_cb,
        timeout: u64,
        repeat: u64,
    ) -> c_int;
    pub fn uv_timer_stop(handle: *mut uv_timer_t) -> c_int;

    pub fn uv_is_closing(handle: *const uv_handle_t) -> c_int;
    pub fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);

    pub fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut c_void, ipc: c_int) -> c_int;
    pub fn uv_pipe_open(handle: *mut c_void, file: c_int) -> c_int;
    pub fn uv_pipe_connect(
        req: *mut c_void,
        handle: *mut c_void,
        name: *const std::ffi::c_char,
        cb: uv_connect_cb,
    );
    pub fn uv_tty_init(loop_: *mut uv_loop_t, tty: *mut c_void, fd: c_int, unused: c_int) -> c_int;
    pub fn uv_tty_set_mode(tty: *mut c_void, mode: c_int) -> c_int;
    pub fn uv_write(
        req: *mut c_void,
        handle: *mut c_void,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: std::ffi::c_uint,
        cb: uv_write_cb,
    ) -> c_int;
    pub fn uv_fs_req_cleanup(req: *mut c_void);
    pub fn uv_fs_get_result(req: *const c_void) -> isize;
    pub fn uv_chdir(dir: *const std::ffi::c_char) -> c_int;
    pub fn uv_cwd(buffer: *mut std::ffi::c_char, size: *mut usize) -> c_int;
    pub fn uv_kill(pid: c_int, signum: c_int) -> c_int;
    pub fn uv_signal_start(
        handle: *mut uv_signal_t,
        signal_cb: uv_signal_cb,
        signum: c_int,
    ) -> c_int;
    pub fn uv_signal_stop(handle: *mut uv_signal_t) -> c_int;
    pub fn uv_dlopen(filename: *const std::ffi::c_char, lib: *mut crate::os::dl::UvLib) -> c_int;
    pub fn uv_dlclose(lib: *mut crate::os::dl::UvLib);
    pub fn uv_dlsym(
        lib: *mut crate::os::dl::UvLib,
        name: *const std::ffi::c_char,
        ptr: *mut *mut c_void,
    ) -> c_int;
    pub fn uv_dlerror(lib: *const crate::os::dl::UvLib) -> *const std::ffi::c_char;
    pub fn uv_freeaddrinfo(ai: *mut c_void);
}

#[inline]
pub unsafe fn loop_queues(loop_: *mut Loop) -> *mut LoopQueuesAndChildren {
    unsafe {
        loop_
            .cast::<u8>()
            .add(uv_loop_size())
            .cast::<LoopQueuesAndChildren>()
    }
}

#[inline]
pub unsafe fn main_loop_ptr() -> *mut Loop {
    (&raw mut crate::globals::main_loop).cast()
}

#[inline]
pub unsafe fn main_loop_events() -> *mut MultiQueue {
    unsafe { (*loop_queues((&raw mut crate::globals::main_loop).cast())).events }
}

#[inline]
pub unsafe fn loop_children_watcher(loop_: *mut Loop) -> *mut uv_signal_t {
    unsafe {
        loop_
            .cast::<u8>()
            .add(uv_loop_size() + size_of::<LoopQueuesAndChildren>())
            .cast()
    }
}

#[inline]
pub unsafe fn loop_children_kill_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe {
        loop_children_watcher(loop_)
            .cast::<u8>()
            .add(uv_handle_size(UV_SIGNAL))
            .cast()
    }
}

#[inline]
pub unsafe fn loop_poll_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe {
        loop_children_kill_timer(loop_)
            .cast::<u8>()
            .add(uv_handle_size(UV_TIMER))
            .cast()
    }
}

#[inline]
pub unsafe fn loop_exit_delay_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe {
        loop_poll_timer(loop_)
            .cast::<u8>()
            .add(uv_handle_size(UV_TIMER))
            .cast()
    }
}

#[inline]
pub unsafe fn loop_async(loop_: *mut Loop) -> *mut uv_async_t {
    unsafe {
        loop_exit_delay_timer(loop_)
            .cast::<u8>()
            .add(uv_handle_size(UV_TIMER))
            .cast()
    }
}

#[inline]
pub unsafe fn loop_mutex(loop_: *mut Loop) -> *mut uv_mutex_t {
    unsafe {
        loop_async(loop_)
            .cast::<u8>()
            .add(uv_handle_size(UV_ASYNC))
            .cast()
    }
}

#[inline]
pub unsafe fn loop_recursive_ptr(loop_: *mut Loop) -> *mut c_int {
    unsafe { loop_mutex(loop_).cast::<u8>().add(UV_MUTEX_SIZE).cast() }
}

#[inline]
pub unsafe fn loop_closing_ptr(loop_: *mut Loop) -> *mut bool {
    unsafe {
        loop_recursive_ptr(loop_)
            .cast::<u8>()
            .add(size_of::<c_int>())
            .cast()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_init(loop_: *mut Loop, _data: *mut c_void) {
    unsafe {
        debug_assert!(
            (loop_closing_ptr(loop_)
                .cast::<u8>()
                .offset_from(loop_.cast::<u8>()) as usize)
                + size_of::<bool>()
                <= size_of::<crate::globals::MainLoopStorage>()
        );
        uv_loop_init(loop_.cast());
        *loop_recursive_ptr(loop_) = 0;
        *loop_closing_ptr(loop_) = false;
        *loop_.cast::<*mut Loop>() = loop_;

        let qc = loop_queues(loop_);
        (*qc).children_size = 0;
        (*qc).children_capacity = 0;
        (*qc).children_items = ptr::null_mut();
        (*qc).events = multiqueue_new(Some(loop_on_put), loop_);
        (*qc).fast_events = multiqueue_new_child((*qc).events);
        (*qc).thread_events = multiqueue_new(None, ptr::null_mut());

        uv_mutex_init(loop_mutex(loop_));
        uv_async_init(loop_.cast(), loop_async(loop_), Some(async_cb));
        uv_signal_init(loop_.cast(), loop_children_watcher(loop_));
        uv_timer_init(loop_.cast(), loop_children_kill_timer(loop_));
        let poll_timer = loop_poll_timer(loop_);
        uv_timer_init(loop_.cast(), poll_timer);
        uv_timer_init(loop_.cast(), loop_exit_delay_timer(loop_));
        *poll_timer.cast::<*mut c_void>() = xmalloc(size_of::<bool>()); // "timeout expired" flag
    }
}

/// Process `Loop.uv` events with a timeout.
unsafe fn loop_uv_run(loop_: *mut Loop, ms: i64) -> bool {
    unsafe {
        let rec_ptr = loop_recursive_ptr(loop_);
        let old_rec = *rec_ptr;
        *rec_ptr = old_rec + 1;
        if old_rec != 0 {
            std::process::abort(); // Should not re-enter uv_run
        }

        let mut mode: c_int = UV_RUN_ONCE;
        let poll_timer = loop_poll_timer(loop_);
        let timeout_expired = *poll_timer.cast::<*mut bool>();
        *timeout_expired = false;

        if ms > 0 {
            uv_timer_start(poll_timer, Some(timer_cb), ms as u64, ms as u64);
        } else if ms == 0 {
            mode = UV_RUN_NOWAIT;
        }

        uv_run(loop_.cast(), mode);

        if ms > 0 {
            uv_timer_stop(poll_timer);
        }

        *rec_ptr -= 1; // Can re-enter uv_run now
        *timeout_expired
    }
}

/// Processes one `Loop.uv` event (at most).
/// Processes all `Loop.fast_events` events.
/// Does NOT process `Loop.events`, that is an application-specific decision.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_poll_events(loop_: *mut Loop, ms: i64) -> bool {
    unsafe {
        let timeout_expired = loop_uv_run(loop_, ms);
        let qc = loop_queues(loop_);
        multiqueue_process_events((*qc).fast_events);
        timeout_expired
    }
}

/// Schedules a fast event from another thread.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_schedule_fast(loop_: *mut Loop, event: Event) {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        multiqueue_put_event((*qc).thread_events, event);
        uv_async_send(loop_async(loop_));
        uv_mutex_unlock(mutex);
    }
}

/// Schedules an event from another thread. Unlike loop_schedule_fast(), the
/// event is forwarded to `Loop.events`, instead of being processed immediately.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_schedule_deferred(loop_: *mut Loop, event: Event) {
    unsafe {
        let eventp: *mut Event = xmalloc(size_of::<Event>()).cast();
        *eventp = event;
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = loop_;
        argv[1] = eventp.cast();
        loop_schedule_fast(
            loop_,
            Event {
                handler: Some(loop_deferred_event),
                argv,
            },
        );
    }
}

unsafe extern "C" fn loop_deferred_event(argv: *mut *mut c_void) {
    unsafe {
        let loop_ = *argv as *mut Loop;
        let eventp = *argv.add(1) as *mut Event;
        let qc = loop_queues(loop_);
        multiqueue_put_event((*qc).events, *eventp);
        xfree(eventp.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_on_put(_queue: *mut MultiQueue, data: *mut c_void) {
    unsafe {
        let loop_: *mut Loop = data;
        if *loop_recursive_ptr(loop_) != 0 {
            uv_stop(loop_.cast());
        }
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_close(loop_: *mut Loop, wait: bool) -> bool {
    unsafe {
        let mut rv = true;
        *loop_closing_ptr(loop_) = true;
        uv_mutex_destroy(loop_mutex(loop_));
        uv_close(loop_children_watcher(loop_).cast(), None);
        uv_close(loop_children_kill_timer(loop_).cast(), None);
        uv_close(loop_poll_timer(loop_).cast(), Some(timer_close_cb));
        uv_close(loop_exit_delay_timer(loop_).cast(), None);
        uv_close(loop_async(loop_).cast(), None);
        let start = if wait { os_hrtime() } else { 0 };
        let mut didstop = false;
        loop {
            uv_run(
                loop_.cast(),
                if didstop {
                    UV_RUN_DEFAULT
                } else {
                    UV_RUN_NOWAIT
                },
            );
            if uv_loop_close(loop_.cast()) != UV_EBUSY || !wait {
                break;
            }
            let elapsed_s = (os_hrtime() - start) / NS_PER_SEC;
            if elapsed_s >= 2 {
                rv = false;
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"loop_close".as_ptr(),
                    line!() as c_int,
                    true,
                    c"uv_loop_close() hang?".as_ptr(),
                );
                log_uv_handles(loop_.cast());
                break;
            }
            if !didstop {
                uv_stop(loop_.cast());
                uv_walk(loop_.cast(), Some(loop_walk_cb), ptr::null_mut());
                didstop = true;
            }
        }
        let qc = loop_queues(loop_);
        multiqueue_free((*qc).fast_events);
        multiqueue_free((*qc).thread_events);
        multiqueue_free((*qc).events);
        xfree((*qc).children_items.cast());
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_purge(loop_: *mut Loop) {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        multiqueue_purge_events((*qc).thread_events);
        multiqueue_purge_events((*qc).fast_events);
        uv_mutex_unlock(mutex);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn loop_size(loop_: *mut Loop) -> usize {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        let rv = multiqueue_size((*qc).thread_events);
        uv_mutex_unlock(mutex);
        rv
    }
}

unsafe extern "C" fn async_cb(handle: *mut uv_async_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let l = *uv_loop.cast::<*mut Loop>();
        let mutex = loop_mutex(l);
        let qc = loop_queues(l);
        uv_mutex_lock(mutex);
        multiqueue_move_events((*qc).fast_events, (*qc).thread_events);
        uv_mutex_unlock(mutex);
    }
}

unsafe extern "C" fn timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let timeout_expired = *handle.cast::<*mut bool>();
        *timeout_expired = true;
    }
}

unsafe extern "C" fn timer_close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let data = *handle.cast::<*mut c_void>();
        xfree(data);
    }
}
