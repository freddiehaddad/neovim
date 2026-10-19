//! Process spawning, waiting, stopping, and teardown (`src/nvim/event/proc.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::event::libuv_proc::{libuv_proc_close, libuv_proc_spawn};
use crate::globals::*;
#[allow(unused_imports)]
use crate::main_init::{os_exit, preserve_exit};
#[allow(unused_imports)]
use crate::os::pty_proc::{
    pty_proc_close, pty_proc_close_master, pty_proc_flush_master, pty_proc_spawn, pty_proc_teardown,
};
#[allow(unused_imports)]
use crate::os::shell::shell_free_argv;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::eval::decode::dict_T;
use crate::event::r#loop::{
    Loop, loop_children_kill_timer, loop_exit_delay_timer, loop_poll_events, loop_queues,
};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_empty, multiqueue_process_events,
    multiqueue_put_event,
};
use crate::event::rstream::{RStream, RStreamFields, rstream_fields, rstream_may_close};
use crate::event::stream::{
    Stream, stream_closed_ptr, stream_fields, stream_init, stream_may_close, stream_size,
    stream_uv_ptr, uv_handle_t, uv_loop_t, uv_pipe_t,
};
use crate::memory::{ARENA_BLOCK_SIZE, xrealloc};
use crate::os::proc::os_proc_tree_kill;
use crate::os::time::os_hrtime;

const KILL_TIMEOUT_MS: u64 = 2000;

const kProcTypeUv: c_int = 0;
const kProcTypePty: c_int = 1;

const SIGHUP: c_int = 1;
const SIGKILL: c_int = 9;
const SIGTERM: c_int = 15;

pub type Proc = c_void;
pub type LibuvProc = c_void;
pub type PtyProc = c_void;
pub use crate::event::r#loop::uv_timer_t;

pub type proc_exit_cb =
    Option<unsafe extern "C" fn(proc_: *mut Proc, status: c_int, data: *mut c_void)>;
pub type proc_state_cb =
    Option<unsafe extern "C" fn(proc_: *mut Proc, suspended: bool, data: *mut c_void)>;
pub type internal_proc_cb = Option<unsafe extern "C" fn(proc_: *mut Proc)>;
type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;
type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;

#[repr(C)]
pub struct ProcHeader {
    pub r#type: c_int,
    pub loop_: *mut Loop,
    pub data: *mut c_void,
    pub pid: c_int,
    pub status: c_int,
    pub refcount: c_int,
    pub exit_signal: u8,
    pub stopped_time: u64,
    pub cwd: *const c_char,
    pub argv: *mut *mut c_char,
    pub exepath: *const c_char,
    pub env: *mut dict_T,
}

#[repr(C)]
pub struct ProcTail {
    pub cb: proc_exit_cb,
    pub state_cb: proc_state_cb,
    pub internal_exit_cb: internal_proc_cb,
    pub internal_close_cb: internal_proc_cb,
    pub closed: bool,
    pub detach: bool,
    pub overlapped: bool,
    pub fwd_err: bool,
    pub stdio_noinherit: bool,
    pub events: *mut MultiQueue,
}

static mut proc_is_tearing_down: bool = false;
static mut exit_need_delay: c_int = 0;

unsafe extern "C" {
    static mut main_loop: [u8; 0];

    fn uv_handle_get_loop(handle: *const uv_handle_t) -> *mut uv_loop_t;
    fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut uv_pipe_t, ipc: c_int) -> c_int;
    fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);
    fn uv_unref(handle: *mut uv_handle_t);
    fn uv_recv_buffer_size(handle: *mut uv_handle_t, value: *mut c_int) -> c_int;
    fn uv_timer_start(handle: *mut uv_timer_t, cb: uv_timer_cb, timeout: u64, repeat: u64)
    -> c_int;
    fn uv_timer_stop(handle: *mut uv_timer_t) -> c_int;

}

#[inline]
unsafe fn main_loop_ptr() -> *mut Loop {
    (&raw mut main_loop).cast()
}

#[inline]
pub unsafe fn rstream_size() -> usize {
    unsafe { stream_size() + size_of::<RStreamFields>() }
}

#[inline]
pub unsafe fn proc_header(proc_: *mut Proc) -> *mut ProcHeader {
    proc_.cast::<ProcHeader>()
}

#[inline]
pub unsafe fn proc_in(proc_: *mut Proc) -> *mut Stream {
    unsafe { proc_.cast::<u8>().add(size_of::<ProcHeader>()).cast() }
}

#[inline]
pub unsafe fn proc_out(proc_: *mut Proc) -> *mut RStream {
    unsafe { proc_in(proc_).cast::<u8>().add(stream_size()).cast() }
}

#[inline]
pub unsafe fn proc_err(proc_: *mut Proc) -> *mut RStream {
    unsafe { proc_out(proc_).cast::<u8>().add(rstream_size()).cast() }
}

#[inline]
pub unsafe fn proc_tail(proc_: *mut Proc) -> *mut ProcTail {
    unsafe { proc_err(proc_).cast::<u8>().add(rstream_size()).cast() }
}

#[inline]
pub unsafe fn proc_size() -> usize {
    unsafe { size_of::<ProcHeader>() + stream_size() + 2 * rstream_size() + size_of::<ProcTail>() }
}

#[inline]
pub unsafe fn libuv_proc_uv(proc_: *mut Proc) -> *mut uv_handle_t {
    unsafe { proc_.cast::<u8>().add(proc_size()).cast() }
}

#[inline]
unsafe fn create_event_1(
    multiqueue: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    arg0: *mut c_void,
) {
    unsafe {
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = arg0;
        if !multiqueue.is_null() {
            multiqueue_put_event(
                multiqueue,
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

#[inline]
unsafe fn loop_process_events(loop_: *mut Loop, multiqueue: *mut MultiQueue, timeout: i64) {
    unsafe {
        if !multiqueue.is_null() && !multiqueue_empty(multiqueue) {
            multiqueue_process_events(multiqueue);
        } else {
            loop_poll_events(loop_, timeout);
        }
    }
}

#[inline]
unsafe fn loop_process_events_until<F: FnMut() -> bool>(
    loop_: *mut Loop,
    multiqueue: *mut MultiQueue,
    timeout: i64,
    mut cond: F,
) {
    unsafe {
        let mut remaining = timeout;
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !cond() {
            loop_process_events(loop_, multiqueue, remaining);
            if remaining == 0 {
                break;
            } else if remaining > 0 {
                let now = os_hrtime();
                remaining -= ((now - before) / 1_000_000) as i64;
                before = now;
                if remaining <= 0 {
                    break;
                }
            }
        }
    }
}

/// @returns zero on success, or negative error code
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_spawn(proc_: *mut Proc, in_: bool, out: bool, err: bool) -> c_int {
    unsafe {
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        let s_in = proc_in(proc_);
        let r_out = proc_out(proc_);
        let r_err = proc_err(proc_);
        let s_out: *mut Stream = r_out.cast();
        let s_err: *mut Stream = r_err.cast();

        debug_assert!(!(err && (*tail).fwd_err));

        if in_ {
            uv_pipe_init((*hdr).loop_.cast(), stream_uv_ptr(s_in).cast(), 0);
        } else {
            *stream_closed_ptr(s_in) = true;
        }

        if out {
            uv_pipe_init((*hdr).loop_.cast(), stream_uv_ptr(s_out).cast(), 0);
        } else {
            *stream_closed_ptr(s_out) = true;
        }

        if err {
            uv_pipe_init((*hdr).loop_.cast(), stream_uv_ptr(s_err).cast(), 0);
        } else {
            *stream_closed_ptr(s_err) = true;
        }

        let status = match (*hdr).r#type {
            kProcTypeUv => libuv_proc_spawn(proc_.cast()),
            kProcTypePty => pty_proc_spawn(proc_.cast()),
            _ => unreachable!(),
        };

        if status != 0 {
            if in_ {
                uv_close(stream_uv_ptr(s_in).cast(), None);
            }
            if out {
                uv_close(stream_uv_ptr(s_out).cast(), None);
            }
            if err {
                uv_close(stream_uv_ptr(s_err).cast(), None);
            }

            if (*hdr).r#type == kProcTypeUv {
                uv_close(libuv_proc_uv(proc_), None);
            } else {
                proc_close(proc_);
            }
            proc_free(proc_);
            (*hdr).status = -1;
            return status;
        }

        if in_ {
            stream_init(ptr::null_mut(), s_in, -1, stream_uv_ptr(s_in).cast());
            let f_in = stream_fields(s_in);
            (*f_in).internal_data = proc_;
            (*f_in).internal_close_cb = Some(on_proc_stream_close);
            (*hdr).refcount += 1;
        }

        if out {
            stream_init(ptr::null_mut(), s_out, -1, stream_uv_ptr(s_out).cast());
            let f_out = stream_fields(s_out);
            (*f_out).internal_data = proc_;
            (*f_out).internal_close_cb = Some(on_proc_stream_close);
            (*hdr).refcount += 1;
        }

        if err {
            stream_init(ptr::null_mut(), s_err, -1, stream_uv_ptr(s_err).cast());
            let f_err = stream_fields(s_err);
            (*f_err).internal_data = proc_;
            (*f_err).internal_close_cb = Some(on_proc_stream_close);
            (*hdr).refcount += 1;
        }

        (*tail).internal_exit_cb = Some(on_proc_exit);
        (*tail).internal_close_cb = Some(decref);
        (*hdr).refcount += 1;

        let qc = loop_queues((*hdr).loop_);
        if (*qc).children_size == (*qc).children_capacity {
            (*qc).children_capacity = if (*qc).children_capacity != 0 {
                (*qc).children_capacity << 1
            } else {
                8
            };
            (*qc).children_items = xrealloc(
                (*qc).children_items.cast(),
                (*qc).children_capacity * size_of::<*mut c_void>(),
            )
            .cast();
        }
        *(*qc).children_items.add((*qc).children_size) = proc_;
        (*qc).children_size += 1;

        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_teardown(loop_: *mut Loop) {
    unsafe {
        proc_is_tearing_down = true;
        let qc = loop_queues(loop_);
        for i in 0..(*qc).children_size {
            let proc_ = *(*qc).children_items.add(i) as *mut Proc;
            let hdr = proc_header(proc_);
            let tail = proc_tail(proc_);
            if (*tail).detach || (*hdr).r#type == kProcTypePty {
                create_event_1((*qc).events, proc_close_handles, proc_);
            } else {
                proc_stop(proc_);
            }
        }

        loop_process_events_until(loop_, (*qc).events, -1, || {
            (*qc).children_size == 0 && multiqueue_empty((*qc).events)
        });
        pty_proc_teardown(loop_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_close_streams(proc_: *mut Proc) {
    unsafe {
        stream_may_close(proc_in(proc_));
        rstream_may_close(proc_out(proc_));
        rstream_may_close(proc_err(proc_));
    }
}

/// Synchronously wait for a process to finish
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_wait(
    proc_: *mut Proc,
    ms: c_int,
    mut events: *mut MultiQueue,
) -> c_int {
    unsafe {
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        if (*hdr).refcount == 0 {
            let status = (*hdr).status;
            loop_process_events((*hdr).loop_, (*tail).events, 0);
            return status;
        }

        if events.is_null() {
            events = (*tail).events;
        }

        (*hdr).refcount += 1;
        loop_process_events_until((*hdr).loop_, events, i64::from(ms), || {
            got_int || (*hdr).refcount == 1
        });

        if got_int {
            got_int = false;
            proc_stop(proc_);
            if ms == -1 {
                loop_process_events_until((*hdr).loop_, events, -1, || (*hdr).refcount == 1);
            } else {
                loop_process_events((*hdr).loop_, events, 0);
            }

            (*hdr).status = -2;
        }

        if (*hdr).refcount == 1 {
            decref(proc_);
            if !(*tail).events.is_null() {
                multiqueue_process_events((*tail).events);
            }
        } else {
            (*hdr).refcount -= 1;
        }

        (*hdr).status
    }
}

/// Ask a process to terminate and eventually kill if it doesn't respond
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_stop(proc_: *mut Proc) {
    unsafe {
        let hdr = proc_header(proc_);
        let exited = (*hdr).status >= 0;
        if exited || (*hdr).stopped_time != 0 {
            return;
        }
        (*hdr).stopped_time = os_hrtime();

        match (*hdr).r#type {
            kProcTypeUv => {
                (*hdr).exit_signal = SIGTERM as u8;
                os_proc_tree_kill((*hdr).pid, SIGTERM);
            }
            kProcTypePty => {
                (*hdr).exit_signal = SIGHUP as u8;
                proc_close_streams(proc_);
                pty_proc_close_master(proc_.cast());
            }
            _ => unreachable!(),
        }

        uv_timer_start(
            loop_children_kill_timer((*hdr).loop_),
            Some(children_kill_cb),
            KILL_TIMEOUT_MS,
            0,
        );
    }
}

/// Frees process-owned resources.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn proc_free(proc_: *mut Proc) {
    unsafe {
        let hdr = proc_header(proc_);
        if !(*hdr).argv.is_null() {
            shell_free_argv((*hdr).argv);
            (*hdr).argv = ptr::null_mut();
        }
    }
}

unsafe extern "C" fn children_kill_cb(handle: *mut uv_timer_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let loop_ = *uv_loop.cast::<*mut Loop>();
        let qc = loop_queues(loop_);

        for i in 0..(*qc).children_size {
            let proc_ = *(*qc).children_items.add(i) as *mut Proc;
            let hdr = proc_header(proc_);
            let exited = (*hdr).status >= 0;
            if exited || (*hdr).stopped_time == 0 {
                continue;
            }
            let term_sent = (*hdr).stopped_time == u64::MAX;
            if (*hdr).r#type != kProcTypePty || term_sent {
                (*hdr).exit_signal = SIGKILL as u8;
                os_proc_tree_kill((*hdr).pid, SIGKILL);
            } else {
                (*hdr).exit_signal = SIGTERM as u8;
                os_proc_tree_kill((*hdr).pid, SIGTERM);
                (*hdr).stopped_time = u64::MAX;
                uv_timer_start(
                    loop_children_kill_timer((*hdr).loop_),
                    Some(children_kill_cb),
                    KILL_TIMEOUT_MS,
                    0,
                );
            }
        }
    }
}

unsafe extern "C" fn proc_close_event(argv: *mut *mut c_void) {
    unsafe {
        let proc_ = *argv as *mut Proc;
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        if let Some(cb) = (*tail).cb {
            cb(proc_, (*hdr).status, (*hdr).data);
        } else {
            proc_free(proc_);
        }
    }
}

unsafe extern "C" fn decref(proc_: *mut Proc) {
    unsafe {
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        (*hdr).refcount -= 1;
        if (*hdr).refcount != 0 {
            return;
        }

        let loop_ = (*hdr).loop_;
        let qc = loop_queues(loop_);
        let mut idx = (*qc).children_size;
        for i in 0..(*qc).children_size {
            if *(*qc).children_items.add(i) == proc_ {
                idx = i;
                break;
            }
        }
        debug_assert!(idx < (*qc).children_size);
        if idx < (*qc).children_size - 1 {
            ptr::copy(
                (*qc).children_items.add(idx + 1),
                (*qc).children_items.add(idx),
                (*qc).children_size - (idx + 1),
            );
        }
        (*qc).children_size -= 1;
        create_event_1((*tail).events, proc_close_event, proc_);
    }
}

unsafe fn proc_close(proc_: *mut Proc) {
    unsafe {
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        if proc_is_tearing_down
            && (*tail).closed
            && ((*tail).detach || (*hdr).r#type == kProcTypePty)
        {
            return;
        }
        debug_assert!(!(*tail).closed);
        (*tail).closed = true;

        if (*tail).detach && (*hdr).r#type == kProcTypeUv {
            uv_unref(libuv_proc_uv(proc_));
        }

        match (*hdr).r#type {
            kProcTypeUv => libuv_proc_close(proc_.cast()),
            kProcTypePty => pty_proc_close(proc_.cast()),
            _ => unreachable!(),
        }
    }
}

unsafe fn flush_stream(proc_: *mut Proc, stream: *mut RStream) {
    unsafe {
        if stream.is_null() || *stream_closed_ptr(stream.cast()) {
            return;
        }

        let hdr = proc_header(proc_);
        let s_fields = stream_fields(stream.cast());
        let r_fields = rstream_fields(stream);

        let mut max_bytes = usize::MAX;
        let limit_bytes = if cfg!(windows) {
            true
        } else {
            (*hdr).r#type != kProcTypePty || proc_is_tearing_down
        };
        if limit_bytes {
            let mut system_buffer_size: c_int = 0;
            let err = uv_recv_buffer_size(
                stream_uv_ptr(stream.cast()).cast(),
                &raw mut system_buffer_size,
            );
            if err != 0 {
                system_buffer_size = ARENA_BLOCK_SIZE as c_int;
            }
            max_bytes = (*r_fields).num_bytes + system_buffer_size as usize;
        }

        while !*stream_closed_ptr(stream.cast()) && (*r_fields).num_bytes < max_bytes {
            let num_bytes = (*r_fields).num_bytes;

            if (*hdr).r#type == kProcTypePty && !(*r_fields).did_eof {
                pty_proc_flush_master(proc_.cast());
            }
            loop_poll_events((*hdr).loop_, 0);
            if !(*s_fields).events.is_null() {
                multiqueue_process_events((*s_fields).events);
            }

            if num_bytes == (*r_fields).num_bytes {
                if let Some(read_cb) = (*r_fields).read_cb
                    && !(*r_fields).did_eof
                {
                    read_cb(stream, (*r_fields).buffer, 0, (*s_fields).cb_data, true);
                }
                break;
            }
        }
    }
}

unsafe extern "C" fn proc_close_handles(argv: *mut *mut c_void) {
    unsafe {
        let proc_ = *argv as *mut Proc;

        exit_need_delay += 1;
        flush_stream(proc_, proc_out(proc_));
        flush_stream(proc_, proc_err(proc_));

        proc_close_streams(proc_);
        proc_close(proc_);
        exit_need_delay -= 1;
    }
}

unsafe extern "C" fn exit_delay_cb(_handle: *mut uv_timer_t) {
    unsafe {
        let mloop = main_loop_ptr();
        let exit_timer = loop_exit_delay_timer(mloop);
        uv_timer_stop(exit_timer);
        let qc = loop_queues(mloop);
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = *exit_timer.cast::<*mut c_void>();
        multiqueue_put_event(
            (*qc).fast_events,
            Event {
                handler: Some(exit_event),
                argv,
            },
        );
    }
}

unsafe extern "C" fn exit_event(argv: *mut *mut c_void) {
    unsafe {
        let status = (*argv as isize) as c_int;
        if exit_need_delay != 0 {
            let mloop = main_loop_ptr();
            let exit_timer = loop_exit_delay_timer(mloop);
            *exit_timer.cast::<*mut c_void>() = *argv;
            uv_timer_start(exit_timer, Some(exit_delay_cb), 0, 0);
            return;
        }

        if !exiting {
            if ui_client_channel_id != 0 {
                ui_client_exit_status = status;
                os_exit(status);
            } else {
                debug_assert_eq!(status, 0);
                preserve_exit(ptr::null());
            }
        }
    }
}

/// Performs self-exit because the primary RPC channel was closed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn exit_on_closed_chan(status: c_int) {
    unsafe {
        let qc = loop_queues(main_loop_ptr());
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = (status as isize) as *mut c_void;
        multiqueue_put_event(
            (*qc).fast_events,
            Event {
                handler: Some(exit_event),
                argv,
            },
        );
    }
}

unsafe extern "C" fn on_proc_exit(proc_: *mut Proc) {
    unsafe {
        let hdr = proc_header(proc_);
        let tail = proc_tail(proc_);
        let qc = loop_queues((*hdr).loop_);
        let queue = if !(*tail).events.is_null() {
            (*tail).events
        } else {
            (*qc).events
        };
        create_event_1(queue, proc_close_handles, proc_);
    }
}

unsafe extern "C" fn on_proc_stream_close(_stream: *mut Stream, data: *mut c_void) {
    unsafe {
        let proc_: *mut Proc = data;
        decref(proc_);
    }
}
