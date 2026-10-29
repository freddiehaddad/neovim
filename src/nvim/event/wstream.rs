//! Writeable libuv stream wrapper (`src/nvim/event/wstream.c`).

use crate::event::r#loop::{
    Loop, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop, uv_write,
};
use crate::event::stream::{Stream, stream_close_handle, stream_init};
use std::ffi::{c_ulong, c_void};

use crate::os::fs::UV_UNKNOWN;

const DEFAULT_MAXMEM: usize = 1024 * 1024 * 2000;
#[cfg(windows)]
const UV_ENOMEM: i32 = -4057;
#[cfg(not(windows))]
const UV_ENOMEM: i32 = -12;

#[allow(non_camel_case_types)]
pub type uv_write_t = c_void;

#[repr(C)]
pub struct WBuffer {
    pub data: Vec<u8>,
}

impl WBuffer {
    #[inline]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn data_ptr(&self) -> *mut u8 {
        self.data.as_ptr().cast_mut()
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_buf_t {
    pub len: c_ulong,
    pub base: *mut u8,
}

#[cfg(not(windows))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct uv_buf_t {
    pub base: *mut u8,
    pub len: usize,
}

impl uv_buf_t {
    #[inline]
    pub fn new(base: *mut u8, size: usize) -> Self {
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
    buffer: std::rc::Rc<WBuffer>,
    uv: UvWriteReq,
}

use crate::event::r#loop::uv_fs_write;

pub fn wstream_init_fd(loop_: &mut Loop, stream: &mut Stream, fd: i32, maxmem: usize) {
    stream_init(&raw mut *loop_, stream, fd, false);
    wstream_init(stream, maxmem);
}

pub fn wstream_init(stream: &mut Stream, maxmem: usize) {
    stream.maxmem = if maxmem != 0 { maxmem } else { DEFAULT_MAXMEM };
}

pub fn wstream_set_write_cb(stream: &mut Stream) {
    stream.notify_write = true;
}

pub fn wstream_write(stream: &mut Stream, buffer: std::rc::Rc<WBuffer>) -> i32 {
    unsafe {
        assert!(stream.maxmem != 0);
        assert!(!stream.closed);

        let uvbuf = uv_buf_t::new(buffer.data_ptr().cast(), buffer.size());

        if !stream.has_uvstream {
            let mut req_buf = [0u64; 128];
            let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
            let idle_loop = uv_handle_get_loop((&raw mut stream.uv).cast());
            let err = uv_fs_write(idle_loop, req_ptr, stream.fd, &uvbuf, 1, stream.fpos, None);
            let req_result = uv_fs_get_result(req_ptr);
            uv_fs_req_cleanup(req_ptr);

            drop(buffer);
            assert!(!stream.notify_write);

            if req_result > 0 {
                stream.fpos += req_result as i64;
                return 0;
            }
            return if err != 0 { err } else { UV_UNKNOWN };
        }

        if stream.curmem > stream.maxmem {
            return UV_ENOMEM;
        }

        stream.curmem += buffer.size();

        let data = Box::into_raw(Box::new(WRequest {
            stream,
            buffer,
            uv: UvWriteReq([0; 256]),
        }));
        let uv_req: *mut uv_write_t = (&raw mut (*data).uv).cast();
        *uv_req.cast::<*mut WRequest>() = data;

        let err = uv_write(uv_req, stream.uvstream_ptr(), &uvbuf, 1, Some(write_cb));
        if err != 0 {
            let _ = Box::from_raw(data);
            return err;
        }

        stream.pending_reqs += 1;
        0
    }
}

pub fn wstream_new_vec_buffer(buf: Vec<u8>) -> std::rc::Rc<WBuffer> {
    std::rc::Rc::new(WBuffer { data: buf })
}

unsafe extern "C" fn write_cb(req: *mut uv_write_t, status: i32) {
    unsafe {
        let data = Box::from_raw(*req.cast::<*mut WRequest>());
        let stream = data.stream;
        let stream = &mut *stream;

        stream.curmem -= data.buffer.size();
        drop(data);

        if stream.notify_write {
            crate::os::shell::shell_write_cb(&mut *stream, status);
        }

        stream.pending_reqs -= 1;

        if stream.closed && stream.pending_reqs == 0 {
            stream_close_handle(&mut *stream);
        }
    }
}
