//! Writeable libuv stream wrapper (`src/nvim/event/wstream.c`).

use crate::event::r#loop::{
    Loop, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop, uv_req_size, uv_write,
};
use crate::event::stream::{
    Stream, stream_close_handle, stream_closed_ptr, stream_fields, stream_init, stream_uv_ptr,
    stream_write_cb,
};
use crate::memory::{xfree, xmalloc};
#[cfg(windows)]
use std::ffi::c_ulong;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::os::fs::UV_UNKNOWN;

const DEFAULT_MAXMEM: usize = 1024 * 1024 * 2000;
const UV_WRITE: c_int = 3;
#[cfg(windows)]
const UV_ENOMEM: c_int = -4057;
#[cfg(not(windows))]
const UV_ENOMEM: c_int = -12;

pub type uv_write_t = c_void;
pub type wbuffer_data_finalizer = Option<unsafe fn(data: *mut c_void)>;

#[repr(C)]
pub struct WBuffer {
    pub size: usize,
    pub refcount: usize,
    pub data: *mut c_char,
    pub cb: wbuffer_data_finalizer,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_buf_t {
    pub len: c_ulong,
    pub base: *mut c_char,
}

#[cfg(not(windows))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_buf_t {
    pub base: *mut c_char,
    pub len: usize,
}

impl uv_buf_t {
    #[inline]
    pub fn new(base: *mut c_char, size: usize) -> Self {
        #[cfg(windows)]
        {
            Self {
                len: size as c_ulong,
                base,
            }
        }
        #[cfg(not(windows))]
        {
            Self { base, len: size }
        }
    }
}

#[repr(C)]
struct WRequestHeader {
    stream: *mut Stream,
    buffer: *mut WBuffer,
}

use crate::event::r#loop::uv_fs_write;

pub unsafe fn wstream_init_fd(loop_: *mut Loop, stream: *mut Stream, fd: c_int, maxmem: usize) {
    unsafe {
        stream_init(loop_, stream, fd, ptr::null_mut());
        wstream_init(stream, maxmem);
    }
}

pub unsafe fn wstream_init(stream: *mut Stream, maxmem: usize) {
    unsafe {
        let fields = stream_fields(stream);
        (*fields).maxmem = if maxmem != 0 { maxmem } else { DEFAULT_MAXMEM };
    }
}

pub unsafe fn wstream_set_write_cb(stream: *mut Stream, cb: stream_write_cb, data: *mut c_void) {
    unsafe {
        let fields = stream_fields(stream);
        (*fields).write_cb = cb;
        (*fields).cb_data = data;
    }
}

pub unsafe fn wstream_write(stream: *mut Stream, buffer: *mut WBuffer) -> c_int {
    unsafe {
        let fields = stream_fields(stream);
        assert!((*fields).maxmem != 0);
        assert!(!*stream_closed_ptr(stream));

        let uvbuf = uv_buf_t::new((*buffer).data, (*buffer).size);

        if (*fields).uvstream.is_null() {
            let mut req_buf = [0u64; 128];
            let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
            let idle_loop = uv_handle_get_loop(stream_uv_ptr(stream).cast());
            let err = uv_fs_write(
                idle_loop,
                req_ptr,
                (*fields).fd,
                &uvbuf,
                1,
                (*fields).fpos,
                None,
            );
            let req_result = uv_fs_get_result(req_ptr);
            uv_fs_req_cleanup(req_ptr);

            wstream_release_wbuffer(buffer);
            assert!((*fields).write_cb.is_none());

            if req_result > 0 {
                (*fields).fpos += req_result as i64;
                return 0;
            }
            return if err != 0 { err } else { UV_UNKNOWN };
        }

        if (*fields).curmem > (*fields).maxmem {
            wstream_release_wbuffer(buffer);
            return UV_ENOMEM;
        }

        (*fields).curmem += (*buffer).size;

        let hdr_size = std::mem::size_of::<WRequestHeader>();
        let data = xmalloc(hdr_size + uv_req_size(UV_WRITE)).cast::<WRequestHeader>();
        (*data).stream = stream;
        (*data).buffer = buffer;
        let uv_req = data.cast::<u8>().add(hdr_size).cast::<uv_write_t>();
        *uv_req.cast::<*mut c_void>() = data.cast();

        let err = uv_write(uv_req, (*fields).uvstream, &uvbuf, 1, Some(write_cb));
        if err != 0 {
            xfree(data.cast());
            wstream_release_wbuffer(buffer);
            return err;
        }

        (*fields).pending_reqs += 1;
        0
    }
}

pub unsafe fn wstream_new_buffer(
    data: *mut c_char,
    size: usize,
    refcount: usize,
    cb: wbuffer_data_finalizer,
) -> *mut WBuffer {
    unsafe {
        let rv = xmalloc(std::mem::size_of::<WBuffer>()).cast::<WBuffer>();
        (*rv).size = size;
        (*rv).refcount = refcount;
        (*rv).cb = cb;
        (*rv).data = data;
        rv
    }
}

unsafe extern "C" fn write_cb(req: *mut uv_write_t, status: c_int) {
    unsafe {
        let data = *req.cast::<*mut WRequestHeader>();
        let stream = (*data).stream;
        let buffer = (*data).buffer;
        let fields = stream_fields(stream);

        (*fields).curmem -= (*buffer).size;

        wstream_release_wbuffer(buffer);

        if let Some(cb) = (*fields).write_cb {
            cb(stream, (*fields).cb_data, status);
        }

        (*fields).pending_reqs -= 1;

        if *stream_closed_ptr(stream) && (*fields).pending_reqs == 0 {
            stream_close_handle(stream);
        }

        xfree(data.cast());
    }
}

pub unsafe fn wstream_release_wbuffer(buffer: *mut WBuffer) {
    unsafe {
        (*buffer).refcount -= 1;
        if (*buffer).refcount == 0 {
            if let Some(cb) = (*buffer).cb {
                cb((*buffer).data.cast());
            }
            xfree(buffer.cast());
        }
    }
}
