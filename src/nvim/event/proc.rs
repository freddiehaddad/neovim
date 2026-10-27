//! Process spawning, waiting, stopping, and teardown (`src/nvim/event/proc.c`).

use crate::eval::typval::dict_T;
pub use crate::event::libuv_proc::LibuvProc;
use crate::event::libuv_proc::{libuv_proc_close, libuv_proc_spawn};
use crate::event::r#loop::{
    Loop, loop_children_kill_timer, loop_exit_delay_timer, loop_poll_events, loop_queues,
    main_loop_ptr, uv_close, uv_handle_get_loop, uv_handle_t, uv_pipe_init, uv_timer_start,
    uv_timer_stop, uv_timer_t,
};
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::rstream::{RStream, rstream_may_close};
use crate::event::stream::{Stream, StreamInternalData, stream_init, stream_may_close};
use crate::globals::*;
use crate::main_init::{os_exit, preserve_exit};
use crate::memory::ARENA_BLOCK_SIZE;
use crate::os::proc::os_proc_tree_kill;
use crate::os::pty_proc::{
    pty_proc_close, pty_proc_close_master, pty_proc_flush_master, pty_proc_spawn, pty_proc_teardown,
};
use crate::os::time::os_hrtime;
use std::ffi::c_int;
use std::ptr;

const KILL_TIMEOUT_MS: u64 = 2000;

use crate::channel::{kProcTypePty, kProcTypeUv};
use crate::os::signal::{SIGHUP, SIGKILL, SIGTERM};

#[repr(C)]
pub struct Proc {
    pub r#type: c_int,
    pub loop_: *mut Loop,
    pub data: *mut crate::channel::Channel,
    pub pid: c_int,
    pub status: c_int,
    pub refcount: c_int,
    pub exit_signal: u8,
    pub stopped_time: u64,
    pub cwd: Option<std::ffi::CString>,
    pub argv: Vec<std::ffi::CString>,
    pub exepath: Option<std::ffi::CString>,
    pub env: *mut dict_T,
    pub r#in: Stream,
    pub out: RStream,
    pub err: RStream,
    pub closed: bool,
    pub detach: bool,
    pub overlapped: bool,
    pub fwd_err: bool,
    pub stdio_noinherit: bool,
    pub events: *mut MultiQueue,
    pub width: u16,
    pub height: u16,
    #[cfg(unix)]
    pub winsize: crate::os::pty_proc_unix::winsize,
    #[cfg(unix)]
    pub tty_fd: c_int,
    #[cfg(windows)]
    pub conpty: Option<Box<crate::os::pty_conpty_win::conpty_t>>,
    #[cfg(windows)]
    pub finish_wait: crate::os::os_win_console::HANDLE,
    #[cfg(windows)]
    pub proc_handle: crate::os::os_win_console::HANDLE,
}

impl Proc {
    pub const fn new() -> Self {
        Self {
            r#type: 0,
            loop_: ptr::null_mut(),
            data: ptr::null_mut(),
            pid: 0,
            status: 0,
            refcount: 0,
            exit_signal: 0,
            stopped_time: 0,
            cwd: None,
            argv: Vec::new(),
            exepath: None,
            env: ptr::null_mut(),
            r#in: Stream::new(),
            out: RStream::new(),
            err: RStream::new(),
            closed: false,
            detach: false,
            overlapped: false,
            fwd_err: false,
            stdio_noinherit: false,
            events: ptr::null_mut(),
            width: 0,
            height: 0,
            #[cfg(unix)]
            winsize: crate::os::pty_proc_unix::winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
            #[cfg(unix)]
            tty_fd: 0,
            #[cfg(windows)]
            conpty: None,
            #[cfg(windows)]
            finish_wait: ptr::null_mut(),
            #[cfg(windows)]
            proc_handle: ptr::null_mut(),
        }
    }
}

pub type PtyProc = Proc;

struct ProcModuleState {
    proc_is_tearing_down: bool,
    exit_need_delay: c_int,
    exit_delay_status: c_int,
}

impl ProcModuleState {
    const fn new() -> Self {
        Self {
            proc_is_tearing_down: false,
            exit_need_delay: 0,
            exit_delay_status: 0,
        }
    }
}

static PROC_STATE: crate::globals::GlobalState<ProcModuleState> = crate::globals::GlobalState::new(ProcModuleState::new());

#[inline]
fn proc_st() -> &'static mut ProcModuleState {
    unsafe { &mut *PROC_STATE.as_ptr() }
}

use crate::event::r#loop::{uv_recv_buffer_size, uv_unref};

#[inline]
pub fn libuv_proc_uv(proc_: &mut Proc) -> *mut uv_handle_t {
    unsafe { (&raw mut (*(&raw mut *proc_).cast::<LibuvProc>()).uv).cast() }
}

#[inline]
fn loop_process_events(loop_: *mut Loop, multiqueue: *mut MultiQueue, timeout: i64) {
    unsafe {
        if !multiqueue.is_null() && !multiqueue_empty(multiqueue) {
            multiqueue_process_events(multiqueue);
        } else {
            loop_poll_events(&mut *loop_, timeout);
        }
    }
}

#[inline]
fn loop_process_events_until<F: FnMut() -> bool>(
    loop_: *mut Loop,
    multiqueue: *mut MultiQueue,
    timeout: i64,
    mut cond: F,
) {
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

/// @returns zero on success, or negative error code
pub fn proc_spawn(proc_: &mut Proc, in_: bool, out: bool, err: bool) -> c_int {
    unsafe {
        let s_in = &raw mut proc_.r#in;
        let s_out = &raw mut proc_.out.s;
        let s_err = &raw mut proc_.err.s;

        debug_assert!(!(err && proc_.fwd_err));

        if in_ {
            uv_pipe_init(proc_.loop_.cast(), (&raw mut (*s_in).uv).cast(), 0);
        } else {
            (*s_in).closed = true;
        }

        if out {
            uv_pipe_init(proc_.loop_.cast(), (&raw mut (*s_out).uv).cast(), 0);
        } else {
            (*s_out).closed = true;
        }

        if err {
            uv_pipe_init(proc_.loop_.cast(), (&raw mut (*s_err).uv).cast(), 0);
        } else {
            (*s_err).closed = true;
        }

        let status = match proc_.r#type {
            kProcTypeUv => libuv_proc_spawn(&mut *(&raw mut *proc_).cast()),
            kProcTypePty => pty_proc_spawn(&mut *(&raw mut *proc_).cast()),
            _ => unreachable!(),
        };

        if status != 0 {
            if in_ {
                uv_close((&raw mut (*s_in).uv).cast(), None);
            }
            if out {
                uv_close((&raw mut (*s_out).uv).cast(), None);
            }
            if err {
                uv_close((&raw mut (*s_err).uv).cast(), None);
            }

            if proc_.r#type == kProcTypeUv {
                uv_close(libuv_proc_uv(proc_), None);
            } else {
                proc_close(proc_);
            }
            proc_.status = -1;
            return status;
        }

        if in_ {
            stream_init(ptr::null_mut(), &mut *s_in, -1, true);
            (*s_in).internal_data = StreamInternalData::Proc(proc_);
            proc_.refcount += 1;
        }

        if out {
            stream_init(ptr::null_mut(), &mut *s_out, -1, true);
            (*s_out).internal_data = StreamInternalData::Proc(proc_);
            proc_.refcount += 1;
        }

        if err {
            stream_init(ptr::null_mut(), &mut *s_err, -1, true);
            (*s_err).internal_data = StreamInternalData::Proc(proc_);
            proc_.refcount += 1;
        }

        proc_.refcount += 1;

        let qc = loop_queues(&mut *proc_.loop_);
        (*qc).children.push(proc_);

        0
    }
}

pub fn proc_teardown(loop_: &mut Loop) {
    unsafe {
        proc_st().proc_is_tearing_down = true;
        let qc = loop_queues(loop_);
        for &proc_ in &(*qc).children {
            if (*proc_).detach || (*proc_).r#type == kProcTypePty {
                if !(*qc).events.is_null() {
                    multiqueue_put_event((*qc).events, Event::ProcCloseHandles(proc_));
                } else {
                    proc_close_handles(&mut *proc_);
                }
            } else {
                proc_stop(&mut *proc_);
            }
        }

        loop_process_events_until(loop_, (*qc).events, -1, || {
            (*qc).children.is_empty() && multiqueue_empty((*qc).events)
        });
        pty_proc_teardown(loop_);
    }
}

pub fn proc_close_streams(proc_: &mut Proc) {
    stream_may_close(&mut proc_.r#in);
        rstream_may_close(&mut proc_.out);
        rstream_may_close(&mut proc_.err);
}

/// Synchronously wait for a process to finish
pub fn proc_wait(proc_: *mut Proc, ms: c_int, mut events: *mut MultiQueue) -> c_int {
    unsafe {
        if (*proc_).refcount == 0 {
            let status = (*proc_).status;
            loop_process_events((*proc_).loop_, (*proc_).events, 0);
            return status;
        }

        if events.is_null() {
            events = (*proc_).events;
        }

        (*proc_).refcount += 1;
        loop_process_events_until((*proc_).loop_, events, i64::from(ms), || {
            g_st().got_int || (*proc_).refcount == 1
        });

        if g_st().got_int {
            g_st().got_int = false;
            proc_stop(&mut *proc_);
            if ms == -1 {
                loop_process_events_until((*proc_).loop_, events, -1, || (*proc_).refcount == 1);
            } else {
                loop_process_events((*proc_).loop_, events, 0);
            }

            (*proc_).status = -2;
        }

        if (*proc_).refcount == 1 {
            proc_decref(&mut *proc_);
            if !(*proc_).events.is_null() {
                multiqueue_process_events((*proc_).events);
            }
        } else {
            (*proc_).refcount -= 1;
        }

        (*proc_).status
    }
}

/// Ask a process to terminate and eventually kill if it doesn't respond
pub fn proc_stop(proc_: &mut Proc) {
    unsafe {
        let exited = proc_.status >= 0;
        if exited || proc_.stopped_time != 0 {
            return;
        }
        proc_.stopped_time = os_hrtime();

        match proc_.r#type {
            kProcTypeUv => {
                proc_.exit_signal = SIGTERM as u8;
                os_proc_tree_kill(proc_.pid, SIGTERM);
            }
            kProcTypePty => {
                proc_.exit_signal = SIGHUP as u8;
                proc_close_streams(proc_);
                pty_proc_close_master(&mut *(&raw mut *proc_).cast());
            }
            _ => unreachable!(),
        }

        uv_timer_start(
            loop_children_kill_timer(&mut *proc_.loop_),
            Some(children_kill_cb),
            KILL_TIMEOUT_MS,
            0,
        );
    }
}

unsafe extern "C" fn children_kill_cb(handle: *mut uv_timer_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let loop_ = *uv_loop.cast::<*mut Loop>();
        let qc = loop_queues(&mut *loop_);

        for &proc_ in &(*qc).children {
            let exited = (*proc_).status >= 0;
            if exited || (*proc_).stopped_time == 0 {
                continue;
            }
            let term_sent = (*proc_).stopped_time == u64::MAX;
            if (*proc_).r#type != kProcTypePty || term_sent {
                (*proc_).exit_signal = SIGKILL as u8;
                os_proc_tree_kill((*proc_).pid, SIGKILL);
            } else {
                (*proc_).exit_signal = SIGTERM as u8;
                os_proc_tree_kill((*proc_).pid, SIGTERM);
                (*proc_).stopped_time = u64::MAX;
                uv_timer_start(
                    loop_children_kill_timer(&mut *(*proc_).loop_),
                    Some(children_kill_cb),
                    KILL_TIMEOUT_MS,
                    0,
                );
            }
        }
    }
}

pub fn proc_close_event(proc_: &mut Proc) {
    unsafe {
        if !proc_.data.is_null() {
            let status = proc_.status;
            let data = proc_.data;
            crate::channel::channel_proc_exit_cb(proc_, status, &mut *data);
        }
    }
}

pub(crate) fn proc_decref(proc_: &mut Proc) {
    unsafe {
        proc_.refcount -= 1;
        if proc_.refcount != 0 {
            return;
        }

        let loop_ = proc_.loop_;
        let qc = loop_queues(&mut *loop_);
        let idx = (*qc).children.iter().position(|&p| p == proc_).unwrap();
        (*qc).children.remove(idx);
        if !proc_.events.is_null() {
            multiqueue_put_event(proc_.events, Event::ProcClose(&raw mut *proc_));
        } else {
            proc_close_event(proc_);
        }
    }
}

fn proc_close(proc_: &mut Proc) {
    unsafe {
        if proc_st().proc_is_tearing_down
            && proc_.closed
            && (proc_.detach || proc_.r#type == kProcTypePty)
        {
            return;
        }
        debug_assert!(!proc_.closed);
        proc_.closed = true;

        if proc_.detach && proc_.r#type == kProcTypeUv {
            uv_unref(libuv_proc_uv(proc_));
        }

        match proc_.r#type {
            kProcTypeUv => libuv_proc_close(&mut *(&raw mut *proc_).cast()),
            kProcTypePty => pty_proc_close(&mut *(&raw mut *proc_).cast()),
            _ => unreachable!(),
        }
    }
}

fn flush_stream(proc_: *mut Proc, stream: *mut RStream) {
    unsafe {
        if stream.is_null() || (*stream).s.closed {
            return;
        }

        let mut max_bytes = usize::MAX;
        let limit_bytes = if cfg!(windows) {
            true
        } else {
            (*proc_).r#type != kProcTypePty || proc_st().proc_is_tearing_down
        };
        if limit_bytes {
            let mut system_buffer_size: c_int = 0;
            let err = uv_recv_buffer_size(
                (&raw mut (*stream).s.uv).cast(),
                &raw mut system_buffer_size,
            );
            if err != 0 {
                system_buffer_size = ARENA_BLOCK_SIZE as c_int;
            }
            max_bytes = (*stream).num_bytes + system_buffer_size as usize;
        }

        while !ptr::read_volatile(&raw const (*stream).s.closed)
            && ptr::read_volatile(&raw const (*stream).num_bytes) < max_bytes
        {
            let num_bytes = (*stream).num_bytes;

            if (*proc_).r#type == kProcTypePty && !(*stream).did_eof {
                pty_proc_flush_master(&mut *(&raw mut *proc_).cast());
            }
            loop_poll_events(&mut *(*proc_).loop_, 0);
            if !(*stream).s.events.is_null() {
                multiqueue_process_events((*stream).s.events);
            }

            if num_bytes == (*stream).num_bytes {
                if !(*stream).did_eof {
                    let _ = (*stream)
                        .s
                        .cb_data
                        .call_read_cb(stream, (*stream).buffer.as_ptr(), 0, true);
                }
                break;
            }
        }
    }
}

pub fn proc_close_handles(proc_: &mut Proc) {
    proc_st().exit_need_delay += 1;
        flush_stream(&raw mut *proc_, &raw mut proc_.out);
        flush_stream(&raw mut *proc_, &raw mut proc_.err);

        proc_close_streams(proc_);
        proc_close(proc_);
        proc_st().exit_need_delay -= 1;
}

unsafe extern "C" fn exit_delay_cb(_handle: *mut uv_timer_t) {
    unsafe {
        let mloop = main_loop_ptr();
        let exit_timer = loop_exit_delay_timer(&mut *mloop);
        uv_timer_stop(exit_timer);
        let qc = loop_queues(&mut *mloop);
        let status = proc_st().exit_delay_status;
        multiqueue_put_event((*qc).fast_events, Event::ProcExit(status));
    }
}

pub fn exit_event(status: c_int) {
    unsafe {
        if proc_st().exit_need_delay != 0 {
            let mloop = main_loop_ptr();
            let exit_timer = loop_exit_delay_timer(&mut *mloop);
            proc_st().exit_delay_status = status;
            uv_timer_start(exit_timer, Some(exit_delay_cb), 0, 0);
            return;
        }

        if !g_st().exiting {
            if g_st().ui_client_channel_id != 0 {
                g_st().ui_client_exit_status = status;
                os_exit(status);
            } else {
                debug_assert_eq!(status, 0);
                preserve_exit(ptr::null());
            }
        }
    }
}

/// Performs self-exit because the primary RPC channel was closed.
pub fn exit_on_closed_chan(status: c_int) {
    unsafe {
        let qc = loop_queues(&mut *main_loop_ptr());
        multiqueue_put_event((*qc).fast_events, Event::ProcExit(status));
    }
}

pub(crate) fn on_proc_exit(proc_: &mut Proc) {
    unsafe {
        let qc = loop_queues(&mut *proc_.loop_);
        let queue = if !proc_.events.is_null() {
            proc_.events
        } else {
            (*qc).events
        };
        if !queue.is_null() {
            multiqueue_put_event(queue, Event::ProcCloseHandles(&raw mut *proc_));
        } else {
            proc_close_handles(proc_);
        }
    }
}
