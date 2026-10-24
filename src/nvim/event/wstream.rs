//! Writeable libuv stream wrapper (`src/nvim/event/wstream.c`).

use crate::event::r#loop::{
    Loop, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop, uv_write,
};
use crate::event::stream::{Stream, stream_close_handle, stream_init, stream_write_cb};
#[cfg(windows)]
use std::ffi::c_ulong;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::os::fs::UV_UNKNOWN;

const DEFAULT_MAXMEM: usize = 1024 * 1024 * 2000;
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

#[repr(C, align(8))]
struct UvWriteReq([u8; 256]);

#[repr(C)]
struct WRequest {
    stream: *mut Stream,
    buffer: *mut WBuffer,
    uv: UvWriteReq,
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
        (*stream).maxmem = if maxmem != 0 { maxmem } else { DEFAULT_MAXMEM };
    }
}

pub unsafe fn wstream_set_write_cb(stream: *mut Stream, cb: stream_write_cb) {
    unsafe {
        (*stream).write_cb = cb;
    }
}

pub unsafe fn wstream_write(stream: *mut Stream, buffer: *mut WBuffer) -> c_int {
    unsafe {
        assert!((*stream).maxmem != 0);
        assert!(!(*stream).closed);

        let uvbuf = uv_buf_t::new((*buffer).data, (*buffer).size);

        if (*stream).uvstream.is_null() {
            let mut req_buf = [0u64; 128];
            let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
            let idle_loop = uv_handle_get_loop((&raw mut (*stream).uv).cast());
            let err = uv_fs_write(
                idle_loop,
                req_ptr,
                (*stream).fd,
                &uvbuf,
                1,
                (*stream).fpos,
                None,
            );
            let req_result = uv_fs_get_result(req_ptr);
            uv_fs_req_cleanup(req_ptr);

            wstream_release_wbuffer(buffer);
            assert!((*stream).write_cb.is_none());

            if req_result > 0 {
                (*stream).fpos += req_result as i64;
                return 0;
            }
            return if err != 0 { err } else { UV_UNKNOWN };
        }

        if (*stream).curmem > (*stream).maxmem {
            wstream_release_wbuffer(buffer);
            return UV_ENOMEM;
        }

        (*stream).curmem += (*buffer).size;

        let data = Box::into_raw(Box::new(WRequest {
            stream,
            buffer,
            uv: UvWriteReq([0; 256]),
        }));
        let uv_req: *mut uv_write_t = (&raw mut (*data).uv).cast();
        *uv_req.cast::<*mut WRequest>() = data;

        let err = uv_write(uv_req, (*stream).uvstream, &uvbuf, 1, Some(write_cb));
        if err != 0 {
            let _ = Box::from_raw(data);
            wstream_release_wbuffer(buffer);
            return err;
        }

        (*stream).pending_reqs += 1;
        0
    }
}

pub unsafe fn wstream_new_buffer(
    data: *mut c_char,
    size: usize,
    refcount: usize,
    cb: wbuffer_data_finalizer,
) -> *mut WBuffer {
    Box::into_raw(Box::new(WBuffer {
        size,
        refcount,
        cb,
        data,
    }))
}

unsafe extern "C" fn write_cb(req: *mut uv_write_t, status: c_int) {
    unsafe {
        let data = *req.cast::<*mut WRequest>();
        let stream = (*data).stream;
        let buffer = (*data).buffer;

        (*stream).curmem -= (*buffer).size;

        wstream_release_wbuffer(buffer);

        if let Some(cb) = (*stream).write_cb {
            cb(stream, status);
        }

        (*stream).pending_reqs -= 1;

        if (*stream).closed && (*stream).pending_reqs == 0 {
            stream_close_handle(stream);
        }

        let _ = Box::from_raw(data);
    }
}

pub unsafe fn wstream_release_wbuffer(buffer: *mut WBuffer) {
    unsafe {
        (*buffer).refcount -= 1;
        if (*buffer).refcount == 0 {
            if let Some(cb) = (*buffer).cb {
                cb((*buffer).data.cast());
            }
            drop(Box::from_raw(buffer));
        }
    }
}
