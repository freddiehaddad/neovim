//! Owned synchronous libuv directory scans.

use std::ffi::CStr;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use crate::event::r#loop::{uv_fs_req_cleanup, uv_fs_scandir, uv_fs_scandir_next, uv_req_size};
use crate::os::fs::{uv_dirent_t, uv_fs_buf_t};

const UV_FS: i32 = 6;

#[must_use]
pub struct Directory {
    request: Box<[u64]>,
    finished: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Directory {
    pub fn open(path: &CStr) -> Result<Self, i32> {
        let size = unsafe { uv_req_size(UV_FS) };
        let mut directory = Self {
            request: vec![0; size.div_ceil(size_of::<u64>())].into_boxed_slice(),
            finished: false,
            _thread: PhantomData,
        };
        // Synchronous requests require cleanup even when the operation fails.
        let result = unsafe {
            uv_fs_scandir(
                ptr::null_mut(),
                directory.request_ptr(),
                path.as_ptr(),
                0,
                ptr::null(),
            )
        };
        if result < 0 {
            Err(result)
        } else {
            Ok(directory)
        }
    }

    /// The entry remains valid until the next scan operation or owner destruction.
    pub fn next_entry(&mut self) -> Option<&CStr> {
        if self.finished {
            return None;
        }
        let mut entry = uv_dirent_t {
            name: ptr::null(),
            r#type: 0,
        };
        let result = unsafe { uv_fs_scandir_next(self.request_ptr(), &raw mut entry) };
        if result != 0 {
            self.finished = true;
            return None;
        }
        if entry.name.is_null() {
            return None;
        }
        // libuv owns this name and releases it on advance or request cleanup.
        Some(unsafe { CStr::from_ptr(entry.name.cast()) })
    }

    fn request_ptr(&mut self) -> *mut uv_fs_buf_t {
        self.request.as_mut_ptr().cast()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        unsafe { uv_fs_req_cleanup(self.request_ptr().cast()) };
    }
}
