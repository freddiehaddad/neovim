//! Low-level libuv stream wrapper (`src/nvim/event/stream.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::event::multiqueue::MultiQueue;

pub const UV_IDLE: c_int = 6;
pub const UV_NAMED_PIPE: c_int = 7;
pub const UV_STREAM: c_int = 11;
pub const UV_TCP: c_int = 12;
pub const UV_TTY: c_int = 14;
pub const UV_FILE: c_int = 17;

const UV_RUN_NOWAIT: c_int = 2;
const UV_TTY_MODE_NORMAL: c_int = 0;
const UV_TTY_MODE_RAW: c_int = 1;

#[cfg(windows)]
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

const LOGLVL_DBG: c_int = 1;
const LOGLVL_WRN: c_int = 3;

pub type Loop = c_void;
pub type Stream = c_void;
pub type uv_loop_t = c_void;
pub type uv_stream_t = c_void;
pub type uv_pipe_t = c_void;
pub type uv_idle_t = c_void;
pub type uv_tty_t = c_void;
pub type uv_handle_t = c_void;

pub type stream_close_cb = Option<unsafe extern "C" fn(stream: *mut Stream, data: *mut c_void)>;
pub type stream_write_cb =
    Option<unsafe extern "C" fn(stream: *mut Stream, data: *mut c_void, status: c_int)>;
type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;

#[repr(C)]
pub struct StreamFields {
    pub uvstream: *mut uv_stream_t,
    pub fd: c_int,
    pub fpos: i64,
    pub cb_data: *mut c_void,
    pub before_close_cb: stream_close_cb,
    pub close_cb: stream_close_cb,
    pub internal_close_cb: stream_close_cb,
    pub close_cb_data: *mut c_void,
    pub internal_data: *mut c_void,
    pub pending_reqs: usize,
    pub events: *mut MultiQueue,
    pub write_cb: stream_write_cb,
    pub curmem: usize,
    pub maxmem: usize,
}

unsafe extern "C" {
    static mut g_min_log_level: c_int;

    fn uv_handle_size(h_type: c_int) -> usize;
    fn uv_loop_init(loop_: *mut uv_loop_t) -> c_int;
    fn uv_loop_close(loop_: *mut uv_loop_t) -> c_int;
    fn uv_run(loop_: *mut uv_loop_t, mode: c_int) -> c_int;
    fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut uv_pipe_t, ipc: c_int) -> c_int;
    fn uv_pipe_open(handle: *mut uv_pipe_t, file: c_int) -> c_int;
    fn uv_stream_set_blocking(handle: *mut uv_stream_t, blocking: c_int) -> c_int;
    fn uv_stream_get_write_queue_size(stream: *const uv_stream_t) -> usize;
    fn uv_idle_init(loop_: *mut uv_loop_t, idle: *mut uv_idle_t) -> c_int;
    fn uv_guess_handle(file: c_int) -> c_int;
    fn uv_is_closing(handle: *const uv_handle_t) -> c_int;
    fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);

    #[cfg(windows)]
    fn uv_tty_init(loop_: *mut uv_loop_t, tty: *mut uv_tty_t, fd: c_int, readable: c_int) -> c_int;
    #[cfg(windows)]
    fn uv_tty_set_mode(tty: *mut uv_tty_t, mode: c_int) -> c_int;

    fn logmsg(
        log_level: c_int,
        context: *const c_char,
        func_name: *const c_char,
        line_num: c_int,
        eol: bool,
        fmt: *const c_char,
        ...
    ) -> bool;
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> c_int;
    fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> c_int;
}

#[inline]
pub unsafe fn stream_uv_offset() -> usize {
    std::mem::size_of::<usize>()
}

#[inline]
pub unsafe fn stream_uv_size() -> usize {
    unsafe {
        let sz = uv_handle_size(UV_NAMED_PIPE)
            .max(uv_handle_size(UV_TCP))
            .max(uv_handle_size(UV_IDLE));
        #[cfg(windows)]
        let sz = sz.max(uv_handle_size(UV_TTY));
        sz
    }
}

#[inline]
pub unsafe fn stream_fields_offset() -> usize {
    unsafe { stream_uv_offset() + stream_uv_size() }
}

#[inline]
pub unsafe fn stream_size() -> usize {
    unsafe { stream_fields_offset() + std::mem::size_of::<StreamFields>() }
}

#[inline]
pub unsafe fn stream_closed_ptr(stream: *mut Stream) -> *mut bool {
    stream.cast::<bool>()
}

#[inline]
pub unsafe fn stream_uv_ptr(stream: *mut Stream) -> *mut c_void {
    unsafe { stream.cast::<u8>().add(stream_uv_offset()).cast() }
}

#[inline]
pub unsafe fn stream_fields(stream: *mut Stream) -> *mut StreamFields {
    unsafe {
        stream
            .cast::<u8>()
            .add(stream_fields_offset())
            .cast::<StreamFields>()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stream_set_blocking(fd: c_int, blocking: bool) -> c_int {
    unsafe {
        let mut loop_buf = [0u64; 256];
        let mut pipe_buf = [0u64; 128];
        let loop_ptr = loop_buf.as_mut_ptr().cast::<uv_loop_t>();
        let pipe_ptr = pipe_buf.as_mut_ptr().cast::<uv_pipe_t>();
        uv_loop_init(loop_ptr);
        uv_pipe_init(loop_ptr, pipe_ptr, 0);
        uv_pipe_open(pipe_ptr, fd);
        let retval = uv_stream_set_blocking(pipe_ptr.cast(), c_int::from(blocking));
        uv_close(pipe_ptr.cast(), None);
        uv_run(loop_ptr, UV_RUN_NOWAIT);
        uv_loop_close(loop_ptr);
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stream_init(
    loop_: *mut Loop,
    stream: *mut Stream,
    fd: c_int,
    uvstream: *mut uv_stream_t,
) {
    unsafe {
        assert!(if uvstream.is_null() {
            fd >= 0 && !loop_.is_null()
        } else {
            fd < 0 && loop_.is_null()
        });
        let fields = stream_fields(stream);
        let uv_ptr = stream_uv_ptr(stream);
        (*fields).uvstream = uvstream;

        if fd >= 0 {
            let h_type = uv_guess_handle(fd);
            (*fields).fd = fd;

            if h_type == UV_FILE {
                uv_idle_init(loop_.cast(), uv_ptr.cast());
                *uv_ptr.cast::<*mut c_void>() = stream;
            } else {
                #[cfg(windows)]
                if h_type == UV_TTY {
                    uv_tty_init(loop_.cast(), uv_ptr.cast(), fd, 0);
                    uv_tty_set_mode(uv_ptr.cast(), UV_TTY_MODE_RAW);
                    let tty_handle = *uv_ptr
                        .cast::<u8>()
                        .add(uv_handle_size(UV_STREAM))
                        .cast::<*mut c_void>();
                    let mut dw_mode: u32 = 0;
                    if GetConsoleMode(tty_handle, &mut dw_mode) != 0 {
                        dw_mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
                        SetConsoleMode(tty_handle, dw_mode);
                    }
                    (*fields).uvstream = uv_ptr.cast();
                } else {
                    assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                    uv_pipe_init(loop_.cast(), uv_ptr.cast(), 0);
                    uv_pipe_open(uv_ptr.cast(), fd);
                    (*fields).uvstream = uv_ptr.cast();
                }

                #[cfg(not(windows))]
                {
                    assert!(h_type == UV_NAMED_PIPE || h_type == UV_TTY);
                    uv_pipe_init(loop_.cast(), uv_ptr.cast(), 0);
                    uv_pipe_open(uv_ptr.cast(), fd);
                    (*fields).uvstream = uv_ptr.cast();
                }
            }
        }

        if !(*fields).uvstream.is_null() {
            *(*fields).uvstream.cast::<*mut c_void>() = stream;
        }

        (*fields).fpos = 0;
        (*fields).internal_data = ptr::null_mut();
        (*fields).curmem = 0;
        (*fields).maxmem = 0;
        (*fields).pending_reqs = 0;
        (*fields).write_cb = None;
        (*fields).close_cb = None;
        (*fields).internal_close_cb = None;
        *stream_closed_ptr(stream) = false;
        (*fields).events = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stream_may_close(stream: *mut Stream) {
    unsafe {
        if *stream_closed_ptr(stream) {
            return;
        }
        if g_min_log_level <= LOGLVL_DBG {
            logmsg(
                LOGLVL_DBG,
                ptr::null(),
                c"stream_may_close".as_ptr(),
                94,
                true,
                c"closing Stream: %p".as_ptr(),
                stream,
            );
        }
        *stream_closed_ptr(stream) = true;

        let fields = stream_fields(stream);
        #[cfg(windows)]
        {
            if uv_guess_handle((*fields).fd) == UV_TTY {
                uv_tty_set_mode(stream_uv_ptr(stream).cast(), UV_TTY_MODE_NORMAL);
            }
        }

        if (*fields).pending_reqs == 0 {
            stream_close_handle(stream);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stream_close_handle(stream: *mut Stream) {
    unsafe {
        let fields = stream_fields(stream);
        let handle: *mut uv_handle_t = if !(*fields).uvstream.is_null() {
            let queued = uv_stream_get_write_queue_size((*fields).uvstream);
            if queued > 0 {
                logmsg(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"stream_close_handle".as_ptr(),
                    116,
                    true,
                    c"closed Stream (%p) with %zu unwritten bytes".as_ptr(),
                    stream,
                    queued,
                );
            }
            (*fields).uvstream.cast()
        } else {
            stream_uv_ptr(stream).cast()
        };

        assert!(!handle.is_null());

        if let Some(before_close_cb) = (*fields).before_close_cb {
            (*fields).pending_reqs += 1;
            before_close_cb(stream, (*fields).close_cb_data);
            (*fields).pending_reqs -= 1;
        }
        if uv_is_closing(handle) == 0 {
            uv_close(handle, Some(close_cb));
        }
    }
}

unsafe extern "C" fn close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let stream = *handle.cast::<*mut Stream>();
        if !stream.is_null() {
            let fields = stream_fields(stream);
            if let Some(cb) = (*fields).close_cb {
                cb(stream, (*fields).close_cb_data);
            }
            if let Some(internal_cb) = (*fields).internal_close_cb {
                internal_cb(stream, (*fields).internal_data);
            }
        }
    }
}
