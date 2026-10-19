//! Functions for calling external native shared libraries (`src/nvim/os/dl.c`).

#[cfg(not(test))]
use crate::message::semsg;
use std::ffi::{c_char, c_int, c_void};

use crate::memory::xstrdup;

#[repr(C)]
pub struct UvLib {
    pub _opaque: [*mut c_void; 4],
}

#[cfg(not(test))]
unsafe extern "C" {
    fn uv_dlopen(filename: *const c_char, lib: *mut UvLib) -> c_int;
    fn uv_dlclose(lib: *mut UvLib);
    fn uv_dlsym(lib: *mut UvLib, name: *const c_char, ptr: *mut *mut c_void) -> c_int;
    fn uv_dlerror(lib: *const UvLib) -> *const c_char;
}

/// Call a function in a dynamically loadable library (`os_libcall`).
///
/// # Safety
/// Pointer arguments must be null or valid C strings / output pointers, and the target symbol
/// signature in `libname` must match the `(argv != null, str_out != null)` calling convention.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_libcall(
    libname: *const c_char,
    funcname: *const c_char,
    argv: *const c_char,
    argi: c_int,
    str_out: *mut *mut c_char,
    int_out: *mut c_int,
) -> bool {
    if libname.is_null() || funcname.is_null() {
        return false;
    }

    #[cfg(test)]
    {
        let _ = (
            argv,
            argi,
            str_out,
            int_out,
            xstrdup,
            UvLib {
                _opaque: [std::ptr::null_mut(); 4],
            },
        );
        false
    }

    #[cfg(not(test))]
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

        if !str_out.is_null() {
            let res: *const c_char = if !argv.is_null() {
                let sfn: unsafe extern "C" fn(*const c_char) -> *const c_char =
                    std::mem::transmute(fn_ptr);
                sfn(argv)
            } else {
                let ifn: unsafe extern "C" fn(c_int) -> *const c_char = std::mem::transmute(fn_ptr);
                ifn(argi)
            };

            let addr = res as isize;
            *str_out = if !res.is_null() && addr != 1 && addr != -1 {
                xstrdup(res)
            } else {
                std::ptr::null_mut()
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
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ));
            assert!(!os_libcall(
                c"lib".as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ));
        }
    }
}
