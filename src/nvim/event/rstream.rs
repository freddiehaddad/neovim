//! Readable libuv stream wrapper (`src/nvim/event/rstream.c`).

use crate::event::r#loop::{
    Loop, uv_err_name, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop,
    uv_handle_get_type, uv_handle_t, uv_strerror,
};
use crate::event::multiqueue::{Event, multiqueue_put_event};
use crate::event::stream::{
    Stream, StreamCbData, UV_TTY, stream_close_handle, stream_init, stream_may_close, uv_idle_t,
    uv_stream_t,
};
use crate::event::wstream::uv_buf_t;
use crate::log::{LOGLVL_DBG, g_min_log_level, logmsg};
use crate::memory::ARENA_BLOCK_SIZE;
use crate::os::fs::{UV_ENOBUFS, UV_EOF};
use std::ffi::{c_char, c_int};
use std::ptr;

#[repr(C)]
pub struct RStream {
    pub s: Stream,
    pub did_eof: bool,
    pub want_read: bool,
    pub pending_read: bool,
    pub paused_full: bool,
    pub buffer: Vec<c_char>,
    pub read_pos: usize,
    pub write_pos: usize,
    pub uvbuf: uv_buf_t,
    pub num_bytes: usize,
}

impl RStream {
    pub const fn new() -> Self {
        Self {
            s: Stream::new(),
            did_eof: false,
            want_read: false,
            pending_read: false,
            paused_full: false,
            buffer: Vec::new(),
            read_pos: 0,
            write_pos: 0,
            uvbuf: uv_buf_t {
                #[cfg(windows)]
                len: 0,
                #[cfg(windows)]
                base: ptr::null_mut(),
                #[cfg(not(windows))]
                base: ptr::null_mut(),
                #[cfg(not(windows))]
                len: 0,
            },
            num_bytes: 0,
        }
    }

    #[inline]
    pub fn read_ptr(&self) -> *const c_char {
        unsafe { self.buffer.as_ptr().add(self.read_pos) }
    }
}

use crate::event::r#loop::{uv_fs_read, uv_idle_start, uv_idle_stop, uv_read_start, uv_read_stop};

#[inline]
unsafe fn rstream_space(stream: *mut RStream) -> usize {
    unsafe { (*stream).buffer.len().saturating_sub((*stream).write_pos) }
}

pub unsafe fn rstream_init_fd(loop_: *mut Loop, stream: *mut RStream, fd: c_int) {
    unsafe {
        stream_init(loop_, &raw mut (*stream).s, fd, false);
        rstream_init(stream);
    }
}

pub unsafe fn rstream_init(stream: *mut RStream) {
    unsafe {
        (*stream).num_bytes = 0;
        (*stream).buffer = vec![0; ARENA_BLOCK_SIZE];
        (*stream).read_pos = 0;
        (*stream).write_pos = 0;
        (*stream).s.is_rstream = true;
    }
}

pub unsafe fn rstream_start_inner(stream: *mut RStream) {
    unsafe {
        if (*stream).s.has_uvstream {
            uv_read_start((*stream).s.uvstream_ptr(), Some(alloc_cb), Some(read_cb));
        } else {
            uv_idle_start((&raw mut (*stream).s.uv).cast(), Some(fread_idle_cb));
        }
    }
}

pub unsafe fn rstream_start(stream: *mut RStream, data: StreamCbData) {
    unsafe {
        (*stream).s.cb_data = data;
        (*stream).want_read = true;
        if !(*stream).paused_full {
            rstream_start_inner(stream);
        }
    }
}

pub unsafe fn rstream_stop_inner(stream: *mut RStream) {
    unsafe {
        if (*stream).s.has_uvstream {
            uv_read_stop((*stream).s.uvstream_ptr());
        } else {
            uv_idle_stop((&raw mut (*stream).s.uv).cast());
        }
    }
}

pub unsafe fn rstream_stop(stream: *mut RStream) {
    unsafe {
        rstream_stop_inner(stream);
        (*stream).want_read = false;
    }
}

unsafe extern "C" fn alloc_cb(handle: *mut uv_handle_t, _suggested: usize, buf: *mut uv_buf_t) {
    unsafe {
        let stream = *handle.cast::<*mut RStream>();
        let write_ptr = (*stream).buffer.as_mut_ptr().add((*stream).write_pos);
        *buf = uv_buf_t::new(write_ptr, rstream_space(stream));
    }
}

unsafe extern "C" fn read_cb(uvstream: *mut uv_stream_t, cnt: isize, _buf: *const uv_buf_t) {
    unsafe {
        let stream = *uvstream.cast::<*mut RStream>();

        if cnt <= 0 {
            if cnt == UV_ENOBUFS as isize || cnt == 0 {
                return;
            } else if cnt == UV_EOF as isize && uv_handle_get_type(uvstream.cast()) == UV_TTY {
                invoke_read_cb(stream, true);
            } else {
                if g_min_log_level <= LOGLVL_DBG {
                    let err_code = cnt as c_int;
                    logmsg!(
                        LOGLVL_DBG,
                        ptr::null(),
                        c"read_cb".as_ptr(),
                        115,
                        true,
                        c"closing Stream (%p): %s (%s)".as_ptr(),
                        stream,
                        uv_err_name(err_code),
                        uv_strerror(err_code),
                    );
                }
                uv_read_stop(uvstream);
                invoke_read_cb(stream, true);
            }
            return;
        }

        let nread = cnt as usize;
        (*stream).num_bytes += nread;
        (*stream).write_pos += nread;
        invoke_read_cb(stream, false);
    }
}

unsafe extern "C" fn fread_idle_cb(handle: *mut uv_idle_t) {
    unsafe {
        let mut req_buf = [0u64; 128];
        let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
        let stream = *handle.cast::<*mut RStream>();

        let write_ptr = (*stream).buffer.as_mut_ptr().add((*stream).write_pos);
        (*stream).uvbuf = uv_buf_t::new(write_ptr, rstream_space(stream));

        let loop_ptr = uv_handle_get_loop(handle.cast());
        uv_fs_read(
            loop_ptr,
            req_ptr,
            (*stream).s.fd,
            &(*stream).uvbuf,
            1,
            (*stream).s.fpos,
            None,
        );
        let req_result = uv_fs_get_result(req_ptr);
        uv_fs_req_cleanup(req_ptr);

        if req_result <= 0 {
            uv_idle_stop((&raw mut (*stream).s.uv).cast());
            invoke_read_cb(stream, true);
            return;
        }

        (*stream).write_pos += req_result as usize;
        (*stream).s.fpos += req_result as i64;
        invoke_read_cb(stream, false);
    }
}

pub unsafe fn read_event(stream: *mut RStream) {
    unsafe {
        (*stream).pending_read = false;
        let available = rstream_available(stream);
        let read_ptr = (*stream).buffer.as_ptr().add((*stream).read_pos);
        if let Some(consumed) = (*stream).s.cb_data.call_read_cb(
            stream,
            read_ptr,
            available,
            (*stream).did_eof,
        ) {
            assert!(consumed <= available);
            rstream_consume(stream, consumed);
        }
        (*stream).s.pending_reqs -= 1;
        if (*stream).s.closed && (*stream).s.pending_reqs == 0 {
            stream_close_handle(&raw mut (*stream).s);
        }
    }
}

pub unsafe fn rstream_available(stream: *mut RStream) -> usize {
    unsafe { (*stream).write_pos.saturating_sub((*stream).read_pos) }
}

pub unsafe fn rstream_consume(stream: *mut RStream, consumed: usize) {
    unsafe {
        (*stream).read_pos += consumed;
        let remaining = (*stream).write_pos - (*stream).read_pos;
        if remaining > 0 && (*stream).read_pos > 0 {
            let rpos = (*stream).read_pos;
            (*stream).buffer.copy_within(rpos..rpos + remaining, 0);
            (*stream).read_pos = 0;
            (*stream).write_pos = remaining;
        } else if remaining == 0 {
            (*stream).read_pos = 0;
            (*stream).write_pos = 0;
        }

        if (*stream).want_read && (*stream).paused_full && rstream_space(stream) != 0 {
            assert!((*stream).s.cb_data.has_read_cb());
            (*stream).paused_full = false;
            rstream_start_inner(stream);
        }
    }
}

unsafe fn invoke_read_cb(stream: *mut RStream, eof: bool) {
    unsafe {
        (*stream).did_eof |= eof;

        if rstream_space(stream) == 0 {
            rstream_stop_inner(stream);
            (*stream).paused_full = true;
        }

        if (*stream).pending_read {
            return;
        }

        (*stream).s.pending_reqs += 1;
        (*stream).pending_read = true;
        let events = (*stream).s.events;
        if !events.is_null() {
            multiqueue_put_event(events, Event::RStreamRead(stream));
        } else {
            read_event(stream);
        }
    }
}

pub(crate) unsafe fn rstream_close_cb(s: *mut Stream) {
    unsafe {
        let stream = s.cast::<RStream>();
        assert!(!stream.is_null());
        (*stream).buffer = Vec::new();
    }
}

pub unsafe fn rstream_may_close(stream: *mut RStream) {
    unsafe {
        stream_may_close(&raw mut (*stream).s);
    }
}
