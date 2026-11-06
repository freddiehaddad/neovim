//! Functions for calling external native shared libraries (`src/nvim/os/dl.c`).

use crate::event::r#loop::{uv_dlclose, uv_dlerror, uv_dlopen, uv_dlsym};
use crate::message::semsg;
use crate::strings::ByteSliceExt;
use std::ffi::{CString, c_void};

#[repr(C)]
pub struct UvLib {
    pub _opaque: [*mut c_void; 4],
}

/// Call a function in a dynamically loadable library (`os_libcall`).
pub fn os_libcall(
    libname: Option<&std::ffi::CStr>,
    funcname: Option<&std::ffi::CStr>,
    argv: Option<&std::ffi::CStr>,
    argi: i32,
    str_out: Option<&mut Option<CString>>,
    int_out: Option<&mut i32>,
) -> bool {
    let (Some(libname), Some(funcname)) = (libname, funcname) else {
        return false;
    };
    let libname = libname.as_ptr();
    let funcname = funcname.as_ptr();
    let argv = crate::strings::ByteSlice::from_opt_cstr(argv);

    unsafe {
        let mut lib = UvLib {
            _opaque: [std::ptr::null_mut(); 4],
        };

        if uv_dlopen(libname, &mut lib) != 0 {
            semsg!(
                c"dlerror = \"%s\"",
                crate::strings::ByteSlice::from_ptr(uv_dlerror(&lib))
            );
            uv_dlclose(&mut lib);
            return false;
        }

        let mut fn_ptr: *mut c_void = std::ptr::null_mut();
        if uv_dlsym(&mut lib, funcname, &mut fn_ptr) != 0 {
            semsg!(
                c"dlerror = \"%s\"",
                crate::strings::ByteSlice::from_ptr(uv_dlerror(&lib))
            );
            uv_dlclose(&mut lib);
            return false;
        }

        if let Some(out) = str_out {
            let res = crate::globals::GraphPtr::<u8>::of(if !argv.is_null() {
                let sfn: unsafe extern "C" fn(*const u8) -> *const u8 = std::mem::transmute(fn_ptr);
                sfn(argv.cast())
            } else {
                let ifn: unsafe extern "C" fn(i32) -> *const u8 = std::mem::transmute(fn_ptr);
                ifn(argi)
            })
            .as_ptr()
            .cast_const();

            let addr = res as isize;
            *out = if !res.is_null() && addr != 1 && addr != -1 {
                Some(crate::strings::ByteSlice::from_ptr(res).to_cstring())
            } else {
                None
            };
        } else if let Some(int_out) = int_out {
            *int_out = if !argv.is_null() {
                let sfn: unsafe extern "C" fn(*const u8) -> i32 = std::mem::transmute(fn_ptr);
                sfn(argv.cast())
            } else {
                let ifn: unsafe extern "C" fn(i32) -> i32 = std::mem::transmute(fn_ptr);
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
        assert!(!os_libcall(None, Some(c"fn"), None, 0, None, None));
        assert!(!os_libcall(Some(c"lib"), None, None, 0, None, None));
    }
}
