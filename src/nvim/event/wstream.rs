//! Writeable libuv stream wrapper (`src/nvim/event/wstream.c`).

use crate::event::r#loop::{
    Loop, uv_fs_get_result, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop, uv_write,
};
use crate::event::stream::{Stream, stream_init};
#[cfg(windows)]
use std::ffi::c_ulong;
use std::ffi::c_void;

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
    pub fn from_mut_slice(slice: &mut [u8]) -> Self {
        #[cfg(windows)]
        {
            Self {
                len: slice.len() as c_ulong,
                base: slice.as_mut_ptr(),
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                len: slice.len(),
                base: slice.as_mut_ptr(),
            }
        }
    }

    #[inline]
    pub fn from_slice(slice: &[u8]) -> Self {
        #[cfg(windows)]
        {
            Self {
                len: slice.len() as c_ulong,
                base: slice.as_ptr().cast_mut(),
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                base: slice.as_ptr().cast_mut(),
                len: slice.len(),
            }
        }
    }
}

#[repr(C, align(8))]
struct UvWriteReq([u8; 256]);

#[repr(C)]
struct WRequest {
    stream: Stream,
    buffer: std::rc::Rc<WBuffer>,
    uv: UvWriteReq,
}

use crate::event::r#loop::uv_fs_write;

pub fn wstream_init_fd(loop_: &Loop, stream: &Stream, fd: i32, maxmem: usize) {
    unsafe { stream_init(Some(loop_), stream, fd, false) };
    wstream_init(stream, maxmem);
}

pub fn wstream_init(stream: &Stream, maxmem: usize) {
    stream
        .maxmem
        .set(if maxmem != 0 { maxmem } else { DEFAULT_MAXMEM });
}

pub fn wstream_set_write_cb(stream: &Stream) {
    stream.notify_write.set(true);
}

pub fn wstream_write(stream: &Stream, buffer: std::rc::Rc<WBuffer>) -> i32 {
    unsafe {
        assert!(stream.uv.is_active());
        assert!(stream.maxmem.get() != 0);
        assert!(!stream.closed.get());

        let uvbuf = uv_buf_t::from_slice(&buffer.data);

        if !stream.has_uvstream.get() {
            let mut req_buf = [0u64; 128];
            let req_ptr = req_buf.as_mut_ptr().cast::<uv_fs_t>();
            let idle_loop = uv_handle_get_loop(stream.uv.as_ptr());
            let err = uv_fs_write(
                idle_loop,
                req_ptr,
                stream.fd.get(),
                &uvbuf,
                1,
                stream.fpos.get(),
                None,
            );
            let req_result = uv_fs_get_result(req_ptr);
            uv_fs_req_cleanup(req_ptr);

            drop(buffer);
            assert!(!stream.notify_write.get());

            if req_result > 0 {
                stream.fpos.set(stream.fpos.get() + req_result as i64);
                return 0;
            }
            return if err != 0 { err } else { UV_UNKNOWN };
        }

        if stream.curmem.get() > stream.maxmem.get() {
            return UV_ENOMEM;
        }

        stream.curmem.set(stream.curmem.get() + buffer.size());

        let data = Box::into_raw(Box::new(WRequest {
            stream: stream.clone(),
            buffer,
            uv: UvWriteReq([0; 256]),
        }));
        let uv_req: *mut uv_write_t = (&raw mut (*data).uv).cast();
        *uv_req.cast::<*mut WRequest>() = data;

        let err = uv_write(
            uv_req,
            stream.uvstream().unwrap_or(std::ptr::null_mut()),
            &uvbuf,
            1,
            Some(write_cb),
        );
        if err != 0 {
            stream
                .curmem
                .set(stream.curmem.get() - (*data).buffer.size());
            let _ = Box::from_raw(data);
            return err;
        }

        stream.pending_reqs.set(stream.pending_reqs.get() + 1);
        0
    }
}

pub fn wstream_new_vec_buffer(buf: Vec<u8>) -> std::rc::Rc<WBuffer> {
    std::rc::Rc::new(WBuffer { data: buf })
}

unsafe extern "C" fn write_cb(req: *mut uv_write_t, status: i32) {
    unsafe {
        let data = Box::from_raw(*req.cast::<*mut WRequest>());
        let stream = data.stream.clone();

        stream.curmem.set(stream.curmem.get() - data.buffer.size());
        drop(data);

        if stream.notify_write.get() {
            crate::os::shell::shell_write_cb(&stream, status);
        }

        stream.finish_request();
    }
}
