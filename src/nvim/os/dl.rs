//! Functions for calling external native shared libraries (`src/nvim/os/dl.c`).

use crate::event::r#loop::{uv_dlclose, uv_dlerror, uv_dlopen, uv_dlsym};
use crate::memory::cstr_to_cstring;
use crate::message::semsg;
use std::ffi::{CString, c_char, c_int, c_void};

#[repr(C)]
pub struct UvLib {
    pub _opaque: [*mut c_void; 4],
}

/// Call a function in a dynamically loadable library (`os_libcall`).
///
/// # Safety
/// Pointer arguments must be null or valid C strings / output pointers, and the target symbol
/// signature in `libname` must match the `(argv != null, str_out != null)` calling convention.
pub unsafe fn os_libcall(
    libname: *const c_char,
    funcname: *const c_char,
    argv: *const c_char,
    argi: c_int,
    str_out: Option<&mut Option<CString>>,
    int_out: *mut c_int,
) -> bool {
    if libname.is_null() || funcname.is_null() {
        return false;
    }

    unsafe {
        let mut lib = UvLib {
            _opaque: [std::ptr::null_mut(); 4],
        };

        if uv_dlopen(libname, &mut lib) != 0 {
            semsg!(c"dlerror = \"%s\"".as_ptr(), uv_dlerror(&lib));
            uv_dlclose(&mut lib);
            return false;
        }

        let mut fn_ptr: *mut c_void = std::ptr::null_mut();
        if uv_dlsym(&mut lib, funcname, &mut fn_ptr) != 0 {
            semsg!(c"dlerror = \"%s\"".as_ptr(), uv_dlerror(&lib));
            uv_dlclose(&mut lib);
            return false;
        }

        if let Some(out) = str_out {
            let res: *const c_char = if !argv.is_null() {
                let sfn: unsafe extern "C" fn(*const c_char) -> *const c_char =
                    std::mem::transmute(fn_ptr);
                sfn(argv)
            } else {
                let ifn: unsafe extern "C" fn(c_int) -> *const c_char = std::mem::transmute(fn_ptr);
                ifn(argi)
            };

            let addr = res as isize;
            *out = if !res.is_null() && addr != 1 && addr != -1 {
                Some(cstr_to_cstring(res))
            } else {
                None
            };
        } else if !int_out.is_null() {
            *int_out = if !argv.is_null() {
                let sfn: unsafe extern "C" fn(*const c_char) -> c_int = std::mem::transmute(fn_ptr);
                sfn(argv)
            } else {
                let ifn: unsafe extern "C" fn(c_int) -> c_int = std::mem::transmute(fn_ptr);
                ifn(argi)
            };
        }

        uv_dlclose(&mut lib);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_libname_or_funcname_returns_false() {
        unsafe {
            assert!(!os_libcall(
                std::ptr::null(),
                c"fn".as_ptr(),
                std::ptr::null(),
                0,
                None,
                std::ptr::null_mut()
            ));
            assert!(!os_libcall(
                c"lib".as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                None,
                std::ptr::null_mut()
            ));
        }
    }
}
