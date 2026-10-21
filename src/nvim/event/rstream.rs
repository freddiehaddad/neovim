//! Readable libuv stream wrapper (`src/nvim/event/rstream.c`).

use crate::event::r#loop::{
    Loop, uv_err_name, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop,
    uv_handle_get_type, uv_handle_t, uv_strerror,
};
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event, multiqueue_put_event};
use crate::event::stream::{
    Stream, UV_TTY, stream_close_handle, stream_closed_ptr, stream_fields, stream_init,
    stream_may_close, stream_size, stream_uv_ptr, uv_idle_t, uv_stream_t,
};
use crate::event::wstream::uv_buf_t;
use crate::log::{LOGLVL_DBG, g_min_log_level, logmsg};
use crate::memory::{ARENA_BLOCK_SIZE, alloc_block, free_block};
use crate::os::fs::{UV_ENOBUFS, UV_EOF};
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

pub type RStream = c_void;
pub type stream_read_cb = Option<
    unsafe extern "C" fn(
        stream: *mut RStream,
        read_data: *const c_char,
        count: usize,
        data: *mut c_void,
        eof: bool,
    ) -> usize,
>;

#[repr(C)]
pub struct RStreamFields {
    pub did_eof: bool,
    pub want_read: bool,
    pub pending_read: bool,
    pub paused_full: bool,
    pub buffer: *mut c_char,
    pub read_pos: *mut c_char,
    pub write_pos: *mut c_char,
    pub uvbuf: uv_buf_t,
    pub read_cb: stream_read_cb,
    pub num_bytes: usize,
}

use crate::event::r#loop::{uv_fs_read, uv_idle_start, uv_idle_stop, uv_read_start, uv_read_stop};

#[inline]
pub unsafe fn rstream_fields(stream: *mut RStream) -> *mut RStreamFields {
    unsafe {
        stream
            .cast::<u8>()
            .add(stream_size())
            .cast::<RStreamFields>()
    }
}

#[inline]
pub unsafe fn rstream_size() -> usize {
    unsafe { stream_size() + std::mem::size_of::<RStreamFields>() }
}

#[inline]
unsafe fn rstream_space(stream: *mut RStream) -> usize {
    unsafe {
        let r = rstream_fields(stream);
        ((*r).buffer.add(ARENA_BLOCK_SIZE) as usize) - ((*r).write_pos as usize)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_init_fd(loop_: *mut Loop, stream: *mut RStream, fd: c_int) {
    unsafe {
        stream_init(loop_, stream.cast(), fd, ptr::null_mut());
        rstream_init(stream);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_init_stream(stream: *mut RStream, uvstream: *mut uv_stream_t) {
    unsafe {
        stream_init(ptr::null_mut(), stream.cast(), -1, uvstream);
        rstream_init(stream);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_init(stream: *mut RStream) {
    unsafe {
        let r = rstream_fields(stream);
        let s = stream_fields(stream);
        (*r).read_cb = None;
        (*r).num_bytes = 0;
        (*r).buffer = alloc_block();
        (*r).read_pos = (*r).buffer;
        (*r).write_pos = (*r).buffer;
        (*s).close_cb = Some(rstream_close_cb);
        (*s).close_cb_data = stream;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_start_inner(stream: *mut RStream) {
    unsafe {
        let s = stream_fields(stream);
        if !(*s).uvstream.is_null() {
            uv_read_start((*s).uvstream, Some(alloc_cb), Some(read_cb));
        } else {
            uv_idle_start(stream_uv_ptr(stream).cast(), Some(fread_idle_cb));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_start(
    stream: *mut RStream,
    cb: stream_read_cb,
    data: *mut c_void,
) {
    unsafe {
        let r = rstream_fields(stream);
        let s = stream_fields(stream);
        (*r).read_cb = cb;
        (*s).cb_data = data;
        (*r).want_read = true;
        if !(*r).paused_full {
            rstream_start_inner(stream);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_stop_inner(stream: *mut RStream) {
    unsafe {
        let s = stream_fields(stream);
        if !(*s).uvstream.is_null() {
            uv_read_stop((*s).uvstream);
        } else {
            uv_idle_stop(stream_uv_ptr(stream).cast());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_stop(stream: *mut RStream) {
    unsafe {
        rstream_stop_inner(stream);
        (*rstream_fields(stream)).want_read = false;
    }
}

unsafe extern "C" fn alloc_cb(handle: *mut uv_handle_t, _suggested: usize, buf: *mut uv_buf_t) {
    unsafe {
        let stream = *handle.cast::<*mut RStream>();
        let r = rstream_fields(stream);
        *buf = uv_buf_t::new((*r).write_pos, rstream_space(stream));
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
        let r = rstream_fields(stream);
        (*r).num_bytes += nread;
        (*r).write_pos = (*r).write_pos.add(nread);
        invoke_read_cb(stream, false);
    }
}

unsafe extern "C" fn fread_idle_cb(handle: *mut uv_idle_t) {
    unsafe {
        let mut req_buf = [0u64; 128];
        let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
        let stream = *handle.cast::<*mut RStream>();
        let r = rstream_fields(stream);
        let s = stream_fields(stream);

        (*r).uvbuf = uv_buf_t::new((*r).write_pos, rstream_space(stream));

        let loop_ptr = uv_handle_get_loop(handle.cast());
        uv_fs_read(loop_ptr, req_ptr, (*s).fd, &(*r).uvbuf, 1, (*s).fpos, None);
        let req_result = uv_fs_get_result(req_ptr);
        uv_fs_req_cleanup(req_ptr);

        if req_result <= 0 {
            uv_idle_stop(stream_uv_ptr(stream).cast());
            invoke_read_cb(stream, true);
            return;
        }

        (*r).write_pos = (*r).write_pos.add(req_result as usize);
        (*s).fpos += req_result as i64;
        invoke_read_cb(stream, false);
    }
}

unsafe extern "C" fn read_event(argv: *mut *mut c_void) {
    unsafe {
        let stream = *argv;
        let r = rstream_fields(stream);
        let s = stream_fields(stream);
        (*r).pending_read = false;
        if let Some(read_cb) = (*r).read_cb {
            let available = rstream_available(stream);
            let consumed = read_cb(
                stream,
                (*r).read_pos.cast_const(),
                available,
                (*s).cb_data,
                (*r).did_eof,
            );
            assert!(consumed <= available);
            rstream_consume(stream, consumed);
        }
        (*s).pending_reqs -= 1;
        if *stream_closed_ptr(stream) && (*s).pending_reqs == 0 {
            stream_close_handle(stream);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_available(stream: *mut RStream) -> usize {
    unsafe {
        let r = rstream_fields(stream);
        ((*r).write_pos as usize) - ((*r).read_pos as usize)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_consume(stream: *mut RStream, consumed: usize) {
    unsafe {
        let r = rstream_fields(stream);
        (*r).read_pos = (*r).read_pos.add(consumed);
        let remaining = ((*r).write_pos as usize) - ((*r).read_pos as usize);
        if remaining > 0 && ((*r).read_pos as usize) > ((*r).buffer as usize) {
            ptr::copy((*r).read_pos, (*r).buffer, remaining);
            (*r).read_pos = (*r).buffer;
            (*r).write_pos = (*r).buffer.add(remaining);
        } else if remaining == 0 {
            (*r).read_pos = (*r).buffer;
            (*r).write_pos = (*r).buffer;
        }

        if (*r).want_read && (*r).paused_full && rstream_space(stream) != 0 {
            assert!((*r).read_cb.is_some());
            (*r).paused_full = false;
            rstream_start_inner(stream);
        }
    }
}

unsafe fn invoke_read_cb(stream: *mut RStream, eof: bool) {
    unsafe {
        let r = rstream_fields(stream);
        let s = stream_fields(stream);
        (*r).did_eof |= eof;

        if rstream_space(stream) == 0 {
            rstream_stop_inner(stream);
            (*r).paused_full = true;
        }

        if (*r).pending_read {
            return;
        }

        (*s).pending_reqs += 1;
        (*r).pending_read = true;
        let events = (*s).events;
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = stream;
        if !events.is_null() {
            multiqueue_put_event(
                events,
                Event {
                    handler: Some(read_event),
                    argv,
                },
            );
        } else {
            read_event(argv.as_mut_ptr());
        }
    }
}

unsafe extern "C" fn rstream_close_cb(s: *mut Stream, data: *mut c_void) {
    unsafe {
        let stream = data;
        assert!(!stream.is_null() && ptr::eq(s, stream));
        let r = rstream_fields(stream);
        if !(*r).buffer.is_null() {
            free_block((*r).buffer);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rstream_may_close(stream: *mut RStream) {
    unsafe {
        stream_may_close(stream);
    }
}
