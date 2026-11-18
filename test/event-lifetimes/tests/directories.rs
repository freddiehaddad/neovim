//! Exercise the production directory owner without linking libuv.
#![allow(dead_code, non_camel_case_types)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};
use std::ptr;

#[path = "../../../src/nvim/os/scandir.rs"]
mod scandir;

mod os {
    pub mod fs {
        pub type uv_fs_buf_t = [u64; 80];

        #[repr(C)]
        pub struct uv_dirent_t {
            pub name: *const u8,
            pub r#type: i32,
        }
    }
}

#[derive(Default)]
struct Driver {
    opened: usize,
    closed: usize,
    advanced: usize,
    fail_open: bool,
    fail_next: bool,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
}

struct NativeScan {
    next: usize,
    current: Option<CString>,
}

#[repr(C)]
struct NativeRequest {
    state: *mut NativeScan,
}

mod event {
    pub mod r#loop {
        use super::super::*;
        use crate::os::fs::{uv_dirent_t, uv_fs_buf_t};

        pub unsafe fn uv_req_size(kind: i32) -> usize {
            assert_eq!(kind, 6);
            size_of::<NativeRequest>()
        }

        pub unsafe fn uv_fs_scandir(
            _: *mut c_void,
            request: *mut uv_fs_buf_t,
            _: *const std::ffi::c_char,
            _: i32,
            _: *const c_void,
        ) -> i32 {
            let failed = DRIVER.with_borrow_mut(|driver| {
                driver.opened += 1;
                driver.fail_open
            });
            let state = Box::into_raw(Box::new(NativeScan {
                next: 0,
                current: None,
            }));
            unsafe {
                request
                    .cast::<NativeRequest>()
                    .write(NativeRequest { state })
            };
            if failed { -1 } else { 2 }
        }

        pub unsafe fn uv_fs_scandir_next(
            request: *mut uv_fs_buf_t,
            entry: *mut uv_dirent_t,
        ) -> i32 {
            let failed = DRIVER.with_borrow_mut(|driver| {
                driver.advanced += 1;
                driver.fail_next
            });
            let state = unsafe { &mut *(*request.cast::<NativeRequest>()).state };
            state.current = None;
            if failed {
                return -1;
            }
            let names = [c"first", c"second"];
            let Some(name) = names.get(state.next) else {
                return -4095;
            };
            state.next += 1;
            state.current = Some((*name).to_owned());
            unsafe {
                entry.write(uv_dirent_t {
                    name: state.current.as_ref().unwrap().as_ptr().cast(),
                    r#type: 0,
                });
            }
            0
        }

        pub unsafe fn uv_fs_req_cleanup(request: *mut c_void) {
            let request = unsafe { &mut *request.cast::<NativeRequest>() };
            assert!(!request.state.is_null(), "request cleaned up twice");
            let state = std::mem::replace(&mut request.state, ptr::null_mut());
            unsafe { drop(Box::from_raw(state)) };
            DRIVER.with_borrow_mut(|driver| driver.closed += 1);
        }
    }
}

fn reset() {
    DRIVER.with_borrow_mut(|driver| *driver = Driver::default());
}

#[test]
fn owner_moves_without_moving_native_request() {
    reset();
    let mut scan = scandir::Directory::open(c".").unwrap();
    let first = scan.next_entry().unwrap().to_owned();
    let mut moved = Box::new(scan);
    assert_eq!(first, c"first");
    assert_eq!(moved.next_entry(), Some(c"second" as &CStr));
    assert!(moved.next_entry().is_none());
    assert!(moved.next_entry().is_none());
    DRIVER.with_borrow(|driver| assert_eq!(driver.advanced, 3));
    drop(moved);
    DRIVER.with_borrow(|driver| assert_eq!(driver.closed, 1));
}

#[test]
fn dropping_a_partial_scan_releases_its_current_entry() {
    reset();
    {
        let mut scan = scandir::Directory::open(c".").unwrap();
        assert_eq!(scan.next_entry(), Some(c"first" as &CStr));
    }
    DRIVER.with_borrow(|driver| assert_eq!((driver.opened, driver.closed), (1, 1)));
}

#[test]
fn failed_initialization_still_cleans_the_native_request() {
    reset();
    DRIVER.with_borrow_mut(|driver| driver.fail_open = true);
    assert!(matches!(scandir::Directory::open(c"."), Err(-1)));
    DRIVER.with_borrow(|driver| assert_eq!((driver.opened, driver.closed), (1, 1)));
}

#[test]
fn iteration_error_does_not_return_the_previous_entry() {
    reset();
    let mut scan = scandir::Directory::open(c".").unwrap();
    assert_eq!(scan.next_entry(), Some(c"first" as &CStr));
    DRIVER.with_borrow_mut(|driver| driver.fail_next = true);
    assert!(scan.next_entry().is_none());
    assert!(scan.next_entry().is_none());
    drop(scan);
    DRIVER.with_borrow(|driver| assert_eq!(driver.closed, 1));
}
