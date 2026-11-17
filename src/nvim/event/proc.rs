//! Shared process ownership across reentrant waiting, draining, and teardown.

pub use crate::event::libuv_proc::LibuvProc;
use crate::event::libuv_proc::{libuv_proc_close, libuv_proc_spawn};
use crate::event::r#loop::{
    Loop, loop_children_kill_timer, loop_exit_delay_timer, loop_poll_events, main_loop,
    uv_handle_t, uv_pipe_init, uv_recv_buffer_size, uv_timer_start, uv_timer_stop, uv_timer_t,
    uv_unref,
};
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::owned::CallbackHandle;
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
use std::cell::Cell;
use std::ffi::{CString, c_void};
use std::rc::Rc;

const KILL_TIMEOUT_MS: u64 = 2000;

use crate::channel::{K_PROC_TYPE_PTY, K_PROC_TYPE_UV};
use crate::os::signal::{SIGHUP, SIGINT, SIGKILL, SIGTERM};

#[derive(Clone)]
pub struct Proc(Rc<ProcState>);

pub struct ProcState {
    pub r#type: Cell<i32>,
    pub chan_id: Cell<u64>,
    pub pid: Cell<i32>,
    pub status: Cell<i32>,
    pub refcount: Cell<i32>,
    pub exit_signal: Cell<u8>,
    pub stopped_time: Cell<u64>,
    pub cwd: CloneCell<Option<CString>>,
    pub argv: CloneCell<Vec<CString>>,
    pub exepath: CloneCell<Option<CString>>,
    pub env: CloneCell<crate::eval::typval::DictId>,
    pub r#in: Stream,
    pub out: RStream,
    pub err: RStream,
    pub closed: Cell<bool>,
    pub detach: Cell<bool>,
    pub overlapped: Cell<bool>,
    pub fwd_err: Cell<bool>,
    pub interrupt: Cell<bool>,
    pub stdio_noinherit: Cell<bool>,
    pub events: Cell<MultiQueue>,
    pub width: Cell<u16>,
    pub height: Cell<u16>,
    pub(super) uv: CallbackHandle<ProcState, 40>,
    pub(super) spawned: Cell<bool>,
    #[cfg(unix)]
    pub winsize: Cell<crate::os::pty_proc_unix::winsize>,
    #[cfg(unix)]
    pub tty_fd: Cell<i32>,
    #[cfg(windows)]
    conpty: std::cell::RefCell<Option<Box<crate::os::pty_conpty_win::Conpty>>>,
    #[cfg(windows)]
    pub finish_wait: Cell<crate::os::os_win_console::HANDLE>,
    #[cfg(windows)]
    pub proc_handle: Cell<crate::os::os_win_console::HANDLE>,
    #[cfg(windows)]
    pub finish_context: Cell<*mut crate::os::pty_proc_win::FinishContext>,
}

impl std::ops::Deref for Proc {
    type Target = ProcState;

    fn deref(&self) -> &ProcState {
        &self.0
    }
}

impl Proc {
    pub fn new() -> Self {
        Self(Rc::new(ProcState {
            r#type: Cell::new(0),
            chan_id: Cell::new(0),
            pid: Cell::new(0),
            status: Cell::new(0),
            refcount: Cell::new(0),
            exit_signal: Cell::new(0),
            stopped_time: Cell::new(0),
            cwd: CloneCell::new(None),
            argv: CloneCell::new(Vec::new()),
            exepath: CloneCell::new(None),
            env: CloneCell::new(crate::eval::typval::DictId::NULL),
            r#in: Stream::new(),
            out: RStream::new(),
            err: RStream::new(),
            closed: Cell::new(false),
            detach: Cell::new(false),
            overlapped: Cell::new(false),
            fwd_err: Cell::new(false),
            interrupt: Cell::new(false),
            stdio_noinherit: Cell::new(false),
            events: Cell::new(MultiQueue::NULL),
            width: Cell::new(0),
            height: Cell::new(0),
            uv: CallbackHandle::new(),
            spawned: Cell::new(false),
            #[cfg(unix)]
            winsize: Cell::new(crate::os::pty_proc_unix::winsize::default()),
            #[cfg(unix)]
            tty_fd: Cell::new(-1),
            #[cfg(windows)]
            conpty: std::cell::RefCell::new(None),
            #[cfg(windows)]
            finish_wait: Cell::new(std::ptr::null_mut()),
            #[cfg(windows)]
            proc_handle: Cell::new(std::ptr::null_mut()),
            #[cfg(windows)]
            finish_context: Cell::new(std::ptr::null_mut()),
        }))
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn is_spawned(&self) -> bool {
        self.spawned.get()
    }

    pub(super) fn owner(&self) -> Rc<ProcState> {
        self.0.clone()
    }

    pub(super) unsafe fn from_callback(handle: *mut c_void) -> Self {
        Self(unsafe { CallbackHandle::<ProcState, 40>::callback_owner(handle) })
    }

    #[cfg(windows)]
    pub fn take_conpty(&self) -> Option<Box<crate::os::pty_conpty_win::Conpty>> {
        self.conpty.borrow_mut().take()
    }

    #[cfg(windows)]
    pub fn set_conpty(&self, value: Option<Box<crate::os::pty_conpty_win::Conpty>>) {
        let old = self.conpty.replace(value);
        drop(old);
    }

    #[cfg(windows)]
    pub fn resize_conpty(&self, width: u16, height: u16) {
        if let Some(conpty) = self.conpty.borrow().as_ref() {
            crate::os::pty_conpty_win::os_conpty_set_size(conpty, width, height);
        }
    }
}

pub type PtyProc = Proc;

struct ProcModuleState {
    proc_is_tearing_down: Cell<bool>,
    exit_need_delay: Cell<i32>,
    exit_delay_status: Cell<i32>,
}

thread_local! {
    static PROC_STATE: ProcModuleState = const {
        ProcModuleState {
            proc_is_tearing_down: Cell::new(false),
            exit_need_delay: Cell::new(0),
            exit_delay_status: Cell::new(0),
        }
    };
}

fn proc_st<R>(f: impl FnOnce(&ProcModuleState) -> R) -> R {
    PROC_STATE.with(f)
}

pub fn libuv_proc_uv(proc_: &Proc) -> *mut uv_handle_t {
    proc_.uv.as_ptr()
}

fn loop_process_events(loop_: &Loop, queue: Option<MultiQueue>, timeout: i64) {
    if let Some(queue) = queue
        && !multiqueue_empty(queue)
    {
        multiqueue_process_events(queue);
    } else {
        loop_poll_events(loop_, timeout);
    }
}

fn loop_process_events_until(
    loop_: &Loop,
    queue: Option<MultiQueue>,
    timeout: i64,
    mut condition: impl FnMut() -> bool,
) {
    let start = if timeout >= 0 { os_hrtime() } else { 0 };
    let mut remaining = timeout;
    let mut expired = false;
    while !condition() && !expired {
        loop_process_events(loop_, queue, remaining);
        if timeout >= 0 {
            remaining = timeout - ((os_hrtime() - start) / 1_000_000) as i64;
            expired = remaining <= 0;
        }
    }
}

pub fn proc_spawn(proc_: &Proc, stdin: bool, stdout: bool, stderr: bool) -> i32 {
    assert!(!proc_.spawned.get());
    assert!(proc_.uv.can_initialize());
    assert!(!(stderr && proc_.fwd_err.get()));
    let streams = [
        (&proc_.r#in, stdin),
        (&proc_.out.s, stdout),
        (&proc_.err.s, stderr),
    ];
    for (stream, enabled) in streams {
        if enabled {
            assert!(stream.uv.can_initialize());
            assert_eq!(stream.pending_reqs.get(), 0);
            unsafe { uv_pipe_init(main_loop().uv_ptr(), stream.uv.as_ptr(), 0) };
            unsafe { stream_init(None, stream, -1, true) };
        } else {
            stream.closed.set(true);
        }
    }
    let status = match proc_.r#type.get() {
        K_PROC_TYPE_UV => libuv_proc_spawn(proc_),
        K_PROC_TYPE_PTY => pty_proc_spawn(proc_),
        _ => unreachable!(),
    };
    if status != 0 {
        for (stream, enabled) in streams {
            if enabled {
                stream_may_close(stream);
            }
        }
        if proc_.r#type.get() == K_PROC_TYPE_UV {
            libuv_proc_close(proc_);
        } else {
            pty_proc_close(proc_);
        }
        proc_.status.set(-1);
        return status;
    }
    proc_.spawned.set(true);
    for (stream, enabled) in streams {
        if enabled {
            stream
                .internal_data
                .set(StreamInternalData::Proc(proc_.clone()));
            proc_.refcount.set(proc_.refcount.get() + 1);
        }
    }
    proc_.refcount.set(proc_.refcount.get() + 1);
    main_loop().add_child(proc_.clone());
    0
}

pub fn proc_teardown(loop_: &Loop) {
    proc_st(|state| state.proc_is_tearing_down.set(true));
    for proc_ in loop_.children() {
        if proc_.detach.get() || proc_.r#type.get() == K_PROC_TYPE_PTY {
            if let Some(events) = loop_.events.get().opt() {
                multiqueue_put_event(events, Event::ProcCloseHandles(proc_));
            } else {
                proc_close_handles(&proc_);
            }
        } else {
            proc_stop(&proc_);
        }
    }
    loop_process_events_until(loop_, loop_.events.get().opt(), -1, || {
        !loop_.has_children() && multiqueue_empty(loop_.events.get())
    });
    pty_proc_teardown(loop_);
}

pub fn proc_close_streams(proc_: &Proc) {
    stream_may_close(&proc_.r#in);
    rstream_may_close(&proc_.out);
    rstream_may_close(&proc_.err);
}

pub fn proc_wait(proc_: &Proc, ms: i32, events: Option<MultiQueue>) -> i32 {
    if proc_.refcount.get() == 0 {
        let status = proc_.status.get();
        loop_process_events(main_loop(), proc_.events.get().opt(), 0);
        return status;
    }
    let events = events.unwrap_or(proc_.events.get());
    proc_.refcount.set(proc_.refcount.get() + 1);
    loop_process_events_until(main_loop(), events.opt(), i64::from(ms), || {
        input_state().got_int.get() || proc_.refcount.get() == 1
    });
    if input_state().got_int.replace(false) {
        proc_stop(proc_);
        if ms == -1 {
            loop_process_events_until(main_loop(), events.opt(), -1, || proc_.refcount.get() == 1);
        } else {
            loop_process_events(main_loop(), events.opt(), 0);
        }
        if !proc_.interrupt.get() {
            proc_.status.set(-2);
        }
    }
    if proc_.refcount.get() == 1 {
        proc_decref(proc_);
        if let Some(events) = proc_.events.get().opt() {
            multiqueue_process_events(events);
        }
    } else {
        proc_.refcount.set(proc_.refcount.get() - 1);
    }
    proc_.status.get()
}

pub fn proc_stop(proc_: &Proc) {
    if proc_.closed.get() || proc_.status.get() >= 0 || proc_.stopped_time.get() != 0 {
        return;
    }
    proc_.stopped_time.set(os_hrtime());
    match proc_.r#type.get() {
        K_PROC_TYPE_UV => {
            let signal = if !cfg!(windows) && proc_.interrupt.get() {
                SIGINT
            } else {
                SIGTERM
            };
            proc_.exit_signal.set(signal as u8);
            os_proc_tree_kill(proc_.pid.get(), signal);
        }
        K_PROC_TYPE_PTY => {
            proc_.exit_signal.set(SIGHUP as u8);
            proc_close_streams(proc_);
            pty_proc_close_master(proc_);
        }
        _ => unreachable!(),
    }
    unsafe {
        crate::event::r#loop::uv_update_time(main_loop().uv_ptr());
        uv_timer_start(
            loop_children_kill_timer(main_loop()),
            Some(children_kill_cb),
            KILL_TIMEOUT_MS,
            0,
        );
    }
}

unsafe extern "C" fn children_kill_cb(_handle: *mut uv_timer_t) {
    for proc_ in main_loop().children() {
        if proc_.status.get() >= 0 || proc_.stopped_time.get() == 0 {
            continue;
        }
        if (proc_.r#type.get() != K_PROC_TYPE_PTY && !proc_.interrupt.get())
            || proc_.stopped_time.get() == u64::MAX
        {
            proc_.exit_signal.set(SIGKILL as u8);
            os_proc_tree_kill(proc_.pid.get(), SIGKILL);
        } else {
            proc_.exit_signal.set(SIGTERM as u8);
            os_proc_tree_kill(proc_.pid.get(), SIGTERM);
            proc_.stopped_time.set(u64::MAX);
            unsafe {
                uv_timer_start(
                    loop_children_kill_timer(main_loop()),
                    Some(children_kill_cb),
                    KILL_TIMEOUT_MS,
                    0,
                );
            }
        }
    }
}

pub fn proc_close_event(proc_: &Proc) {
    if let Some(chan) = crate::msgpack_rpc::channel::find_channel_opt(proc_.chan_id.get()) {
        crate::channel::channel_proc_exit_cb(proc_.status.get(), &chan);
    }
}

pub(crate) fn proc_decref(proc_: &Proc) {
    assert!(proc_.refcount.get() > 0);
    proc_
        .refcount
        .set(proc_.refcount.get().checked_sub(1).unwrap());
    if proc_.refcount.get() != 0 {
        return;
    }
    main_loop().remove_child(proc_);
    if let Some(events) = proc_.events.get().opt() {
        multiqueue_put_event(events, Event::ProcClose(proc_.clone()));
    } else {
        proc_close_event(proc_);
    }
}

fn proc_close(proc_: &Proc) {
    if proc_st(|state| state.proc_is_tearing_down.get())
        && proc_.closed.get()
        && (proc_.detach.get() || proc_.r#type.get() == K_PROC_TYPE_PTY)
    {
        return;
    }
    assert!(!proc_.closed.replace(true));
    if proc_.detach.get() && proc_.r#type.get() == K_PROC_TYPE_UV {
        unsafe { uv_unref(libuv_proc_uv(proc_)) };
    }
    match proc_.r#type.get() {
        K_PROC_TYPE_UV => libuv_proc_close(proc_),
        K_PROC_TYPE_PTY => pty_proc_close(proc_),
        _ => unreachable!(),
    }
}

fn flush_stream(proc_: &Proc, stderr: bool) {
    let stream = if stderr { &proc_.err } else { &proc_.out };
    if stream.closed.get() {
        return;
    }
    let mut max_bytes = usize::MAX;
    if cfg!(windows)
        || proc_.r#type.get() != K_PROC_TYPE_PTY
        || proc_st(|state| state.proc_is_tearing_down.get())
    {
        let mut size = 0;
        if unsafe { uv_recv_buffer_size(stream.uv.as_ptr(), &mut size) } != 0 {
            size = ARENA_BLOCK_SIZE as i32;
        }
        max_bytes = stream.num_bytes.get().saturating_add(size as usize);
    }
    while !stream.closed.get() && stream.num_bytes.get() < max_bytes && !input_state().got_int.get()
    {
        let count = stream.num_bytes.get();
        if proc_.r#type.get() == K_PROC_TYPE_PTY && !stream.did_eof.get() {
            pty_proc_flush_master(proc_);
        }
        loop_poll_events(main_loop(), 0);
        if let Some(events) = stream.events.get().opt() {
            multiqueue_process_events(events);
        }
        if count == stream.num_bytes.get() {
            if !stream.did_eof.get() {
                let _ = stream
                    .cb_data
                    .get()
                    .call_read_cb(stream.fd.get(), &[], true);
            }
            break;
        }
    }
}

struct ExitDelayGuard;

impl Drop for ExitDelayGuard {
    fn drop(&mut self) {
        proc_st(|state| state.exit_need_delay.set(state.exit_need_delay.get() - 1));
    }
}

pub fn proc_close_handles(proc_: &Proc) {
    proc_st(|state| state.exit_need_delay.set(state.exit_need_delay.get() + 1));
    let _delay = ExitDelayGuard;
    flush_stream(proc_, false);
    flush_stream(proc_, true);
    proc_close_streams(proc_);
    proc_close(proc_);
}

unsafe extern "C" fn exit_delay_cb(_handle: *mut uv_timer_t) {
    unsafe { uv_timer_stop(loop_exit_delay_timer(main_loop())) };
    let status = proc_st(|state| state.exit_delay_status.get());
    multiqueue_put_event(main_loop().fast_events.get(), Event::ProcExit(status));
}

pub fn exit_event(status: i32) {
    if proc_st(|state| state.exit_need_delay.get()) != 0 {
        proc_st(|state| state.exit_delay_status.set(status));
        unsafe {
            uv_timer_start(
                loop_exit_delay_timer(main_loop()),
                Some(exit_delay_cb),
                0,
                0,
            )
        };
        return;
    }
    if !exc_state().exiting.get() {
        if screen_state().ui_client_channel_id.get() != 0 {
            screen_state().ui_client_exit_status.set(status);
            os_exit(status);
        } else {
            debug_assert_eq!(status, 0);
            preserve_exit(None);
        }
    }
}

pub fn exit_on_closed_chan(status: i32) {
    multiqueue_put_event(main_loop().fast_events.get(), Event::ProcExit(status));
}

pub(crate) fn on_proc_exit(proc_: &Proc) {
    let events = proc_.events.get().opt().unwrap_or(main_loop().events.get());
    if events.is_null() {
        proc_close_handles(proc_);
    } else {
        multiqueue_put_event(events, Event::ProcCloseHandles(proc_.clone()));
    }
}
