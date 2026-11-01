//! Environment variable inspection and path expansion (`src/nvim/os/env.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::FileBuf;
use crate::charset::skipwhite;
#[cfg(unix)]
use crate::charset::vim_isfilec;
pub(crate) use crate::cmdexpand::EXPAND_BUF_LEN;
pub use crate::cmdexpand::ExpandCtx;
use crate::eval::fs::modify_fname;
use crate::eval::skip_expr;
use crate::eval::vars::{VV_PROGPATH, get_vim_var_str};
use crate::globals::*;
use crate::mbyte::{utf_isalnum, utf_ptr2char, utf_ptr2len};
#[cfg(windows)]
use crate::mbyte::{utf_valid_string, utf16_to_utf8};
use crate::memory::{cstr_from_bytes, cstr_to_cstring, xstrlcpy};
#[cfg(windows)]
use crate::message::emsg;
#[cfg(any(windows, test))]
use crate::message::internal_error;
#[cfg(windows)]
use crate::message::semsg;
use crate::option_vars::*;
use crate::os::fs::{UV_EINVAL, UV_ENOBUFS, os_isdir};
#[cfg(unix)]
use crate::os::fs::{os_dirname, os_realpath};
#[cfg(unix)]
use crate::os::users::os_get_userdir;
#[cfg(any(windows, test))]
use crate::path::path_is_absolute;
use crate::path::{
    after_pathsep, append_path, concat_fnames, path_cmp, path_tail, path_tail_with_sep, to_slash,
    to_slash_save, vim_ispathsep,
};
use crate::strings::vim_strsave_escaped;
use std::ffi::CStr;
use std::ptr;

const INIT_SIZE: usize = 64;
const RUNTIME_DIRNAME: &CStr = c"runtime";

#[cfg(windows)]
use crate::os::fs::PATHSEPSTR;
#[cfg(any(windows, test))]
use crate::os::fs::{ENV_SEPCHAR, ENV_SEPSTR};
#[cfg_attr(not(windows), cfg(test))]
const MAX_ENVPATHLEN: usize = if cfg!(windows) {
    8192
} else {
    i32::MAX as usize
};

struct EnvModuleState {
    homedir: Option<std::ffi::CString>,
    homedir_buf: [u8; MAXPATHL],
}

impl EnvModuleState {
    const fn new() -> Self {
        Self {
            homedir: None,
            homedir_buf: [0; MAXPATHL],
        }
    }
}

static ENV_STATE: crate::globals::GlobalState<EnvModuleState> =
    crate::globals::GlobalState::new(EnvModuleState::new());

#[inline]
fn env_st() -> &'static mut EnvModuleState {
    unsafe { &mut *ENV_STATE.as_ptr() }
}

use crate::version::{default_vim_dir, default_vimruntime_dir};

use crate::event::r#loop::{uv_os_getenv, uv_os_homedir, uv_os_setenv, uv_os_unsetenv};

unsafe extern "C" {
    #[cfg(target_os = "macos")]
    fn _NSGetEnviron() -> *mut *mut *mut u8;
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[allow(non_upper_case_globals)]
    static environ: *mut *mut u8;

    #[cfg(windows)]
    fn _putenv_s(varname: *const u8, value_string: *const u8) -> i32;

    #[cfg(not(windows))]
    fn gethostname(name: *mut u8, len: usize) -> i32;
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn os_environ_ptr() -> *mut *mut *mut u8 {
    #[cfg(target_os = "macos")]
    unsafe {
        _NSGetEnviron()
    }
    #[cfg(not(target_os = "macos"))]
    {
        (&raw const environ).cast_mut()
    }
}

#[cfg(unix)]
use crate::globals::IOSIZE;
use crate::os::fs::MAXPATHL;
#[cfg(windows)]
use crate::os::os_win_console::GetLastError;

#[cfg(windows)]
unsafe extern "system" {
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(lpszEnvironmentBlock: *mut u16) -> i32;
    fn GetComputerNameW(lpBuffer: *mut u16, nSize: &mut u32) -> i32;
}

#[inline]
fn env_name_valid(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    let bytes = crate::memory::cstr_as_bytes(name);
    #[cfg(windows)]
    {
        !bytes.is_empty() && utf_valid_string(bytes)
    }
    #[cfg(not(windows))]
    {
        !bytes.is_empty()
    }
}

pub fn env_init() {
    g_st().nvim_testing = os_env_exists(c"NVIM_TEST", false);
}

/// Gets the length (bytes) of the env var name at `p`.
pub fn env_name_len(p: impl crate::strings::IntoCCharPtr) -> usize {
    let p = p.into_c_char_ptr();
    {
        let mut s = p;
        loop {
            let b = crate::memory::cstr_first_byte(s);
            if b < 0x80 {
                if !b.is_ascii_alphanumeric() && b != b'_' {
                    break;
                }
                s = s.wrapping_add(1);
                continue;
            }
            let (len, is_alnum) = {
                let l = utf_ptr2len(s);
                (l, l != 1 && utf_isalnum(utf_ptr2char(s)))
            };
            if !is_alnum {
                break;
            }
            s = s.wrapping_add(len as usize);
        }
        (s as usize) - (p as usize)
    }
}

/// Like `getenv()`, but returns `None` if the variable is empty, without C heap allocations.
pub fn os_getenv_cstr<S: crate::strings::IntoCCharPtr>(name: S) -> Option<std::ffi::CString> {
    let name = name.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return None;
        }
        let mut size = INIT_SIZE;
        let mut buf = [0u8; INIT_SIZE];
        let mut r = unsafe { uv_os_getenv(name, buf.as_mut_ptr().cast(), &mut size) };
        let res = if r == UV_ENOBUFS {
            let mut vec_buf = vec![0u8; size + 1];
            r = unsafe { uv_os_getenv(name, vec_buf.as_mut_ptr().cast(), &mut size) };
            if r != 0 || size == 0 || vec_buf[0] == 0 {
                None
            } else {
                let bytes = crate::memory::cstr_slice(vec_buf.as_ptr().cast(), size);
                Some(cstr_from_bytes(bytes))
            }
        } else if r != 0 || size == 0 || buf[0] == 0 {
            None
        } else {
            let bytes = crate::memory::cstr_slice(buf.as_ptr().cast(), size);
            Some(cstr_from_bytes(bytes))
        };
        assert_ne!(r, UV_EINVAL);
        res
    }
}

/// Like `getenv()`, but stores the value in `buf` instead of allocating.
pub fn os_getenv_buf<S: crate::strings::IntoCCharPtr, T>(
    name: S,
    buf: *mut T,
    bufsize: usize,
) -> *mut T {
    let name = name.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return ptr::null_mut();
        }
        {
            let mut size = bufsize;
            let mut r = unsafe { uv_os_getenv(name, buf.cast(), &mut size) };
            if r == UV_ENOBUFS {
                let mut e = vec![0u8; size];
                r = unsafe { uv_os_getenv(name, e.as_mut_ptr().cast(), &mut size) };
                if r == 0 && size != 0 && e[0] != 0 {
                    crate::memory::cstr_ncpy(buf.cast(), e.as_ptr().cast(), bufsize.min(size) - 1);
                }
            }
            assert_ne!(r, UV_EINVAL);

            if r != 0 || size == 0 || crate::memory::cstr_is_empty(buf.cast()) {
                return ptr::null_mut();
            }
            buf
        }
    }
}

/// Returns true if environment variable `name` is defined (even if empty).
pub fn os_env_exists<S: crate::strings::IntoCCharPtr>(name: S, nonempty: bool) -> bool {
    let name = name.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return false;
        }
        {
            let mut buf = [0u8; 2];
            let mut size = buf.len();
            let r = unsafe { uv_os_getenv(name, buf.as_mut_ptr().cast(), &mut size) };
            assert_ne!(r, UV_EINVAL);
            (r == 0 && (!nonempty || size > 0)) || r == UV_ENOBUFS
        }
    }
}

/// Sets an environment variable.
pub fn os_setenv<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    name: S1,
    value: S2,
    overwrite: i32,
) -> i32 {
    let name = name.into_c_char_ptr();
    let value = value.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return -1;
        }
        #[cfg(windows)]
        {
            if overwrite == 0 && os_env_exists(name, true) {
                return 0;
            }
            if crate::memory::cstr_is_empty(value) {
                return os_unsetenv(name);
            }
        }
        #[cfg(not(windows))]
        {
            if overwrite == 0 && os_env_exists(name, false) {
                return 0;
            }
        }
        {
            #[cfg(windows)]
            if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"LC_ALL")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"LANGUAGE")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"LANG")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"LC_MESSAGES")
            {
                let r = unsafe { _putenv_s(name.cast(), value.cast()) };
                assert_eq!(r, 0);
            }
            let r = unsafe { uv_os_setenv(name, value) };
            assert_ne!(r, UV_EINVAL);
            if r == 0 { 0 } else { -1 }
        }
    }
}

/// Unset environment variable.
pub fn os_unsetenv(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return -1;
        }
        {
            let r = unsafe { uv_os_unsetenv(name) };
            if r == 0 { 0 } else { -1 }
        }
    }
}

/// Copy name of the environment variable at `index` in the current environment variables block.
pub fn os_getenvname_at_index(index: usize) -> Option<std::ffi::CString> {
    #[cfg(windows)]
    {
        let envstrings = unsafe { GetEnvironmentStringsW() };
        if envstrings.is_null() {
            return None;
        }
        let mut p = envstrings;
        let mut name: Option<std::ffi::CString> = None;
        let mut i: usize = 0;
        loop {
            let l = {
                let mut n = 0usize;
                while unsafe { *p.wrapping_add(n) } != 0 {
                    n += 1;
                }
                n
            };
            if l == 0 || i > index {
                break;
            }
            if i == index {
                let slice = unsafe { std::slice::from_raw_parts(p, l) };
                let Ok(utf8_str) = String::from_utf16(slice) else {
                    semsg!(c"utf16_to_utf8 failed: %d", 1);
                    break;
                };
                let bytes = utf8_str.as_bytes();
                let start_off = usize::from(bytes.first() == Some(&b'='));
                if let Some(pos) = bytes[start_off..].iter().position(|&b| b == b'=') {
                    let len = start_off + pos;
                    assert!(len > 0);
                    name = std::ffi::CString::new(&bytes[..len]).ok();
                }
                break;
            }
            p = p.wrapping_add(l + 1);
            i += 1;
        }
        unsafe { FreeEnvironmentStringsW(envstrings) };
        name
    }
    #[cfg(not(windows))]
    unsafe {
        let env_arr: *const *mut u8 = (*os_environ_ptr()).cast_const();

        for i in 0..=index {
            if (*env_arr.add(i)).is_null() {
                return None;
            }
        }
        let str_ptr = *env_arr.add(index);
        assert!(!str_ptr.is_null());
        let bytes = crate::memory::cstr_as_bytes(str_ptr.cast());
        let pos = bytes.iter().position(|&b| b == b'=')?;
        assert!(pos > 0);
        std::ffi::CString::new(&bytes[..pos]).ok()
    }
}

/// Get the process ID of the Nvim process.
pub fn os_get_pid() -> i64 {
    i64::from(std::process::id())
}

/// Signals to the OS that Nvim is an application for "interactive work".
pub fn os_hint_priority() {}

/// Gets the hostname of the current machine.
pub fn os_get_hostname<T>(hostname: *mut T, size: usize) {
    let hostname = hostname.cast::<u8>();
    #[cfg(not(windows))]
    unsafe {
        if gethostname(hostname, size) < 0 {
            *hostname = 0;
        } else if size > 0 {
            *hostname.add(size - 1) = 0;
        }
    }
    #[cfg(windows)]
    {
        let mut host_utf16 = [0u16; 16];
        let mut host_wsize = host_utf16.len() as u32;
        if unsafe { GetComputerNameW(host_utf16.as_mut_ptr(), &mut host_wsize) } == 0 {
            unsafe {
                *hostname = 0;
            }
            let err = unsafe { GetLastError() };
            semsg!(c"GetComputerNameW failed: %d", err);
            return;
        }
        host_utf16[host_wsize as usize] = 0;

        let mut host_utf8: Option<std::ffi::CString> = None;
        let conversion_result = utf16_to_utf8(host_utf16.as_ptr(), -1, &mut host_utf8);
        if conversion_result != 0 {
            semsg!(c"utf16_to_utf8 failed: %d", conversion_result);
            return;
        }
        if let Some(host_cstr) = host_utf8 {
            xstrlcpy(hostname.cast(), host_cstr.as_ptr(), size);
        }
    }
}

/// Gets the "real", resolved user home directory as determined by `init_homedir`.
#[cfg(windows)]
pub fn os_homedir() -> Option<&'static CStr> {
    let Some(ref hd) = env_st().homedir else {
        emsg(c"os_homedir failed: homedir not initialized");
        return None;
    };
    Some(hd.as_c_str())
}

fn os_uv_homedir() -> *mut u8 {
    unsafe {
        let buf = (&raw mut env_st().homedir_buf).cast::<u8>();
        *buf = 0;
        let mut homedir_size = MAXPATHL;
        let ret_value = uv_os_homedir(buf.cast(), &mut homedir_size);
        if ret_value == 0 && homedir_size < MAXPATHL {
            return buf;
        }
        *buf = 0;
        ptr::null_mut()
    }
}

/// Sets `homedir` to the "real", resolved user home directory.
pub fn init_homedir() {
    env_st().homedir = None;

    let home_cstr = os_getenv_cstr(c"HOME");
    let mut var: *mut u8 = home_cstr
        .as_ref()
        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());

    #[cfg(windows)]
    {
        if var.is_null() {
            let homedrive = os_getenv_cstr(c"HOMEDRIVE");
            let homepath = os_getenv_cstr(c"HOMEPATH");
            let homepath_ptr = homepath
                .as_ref()
                .map_or(PATHSEPSTR.as_ptr(), |c| c.as_ptr());
            if let Some(ref hd) = homedrive
                && hd.as_bytes().len() + crate::memory::cstr_len(homepath_ptr) < MAXPATHL
            {
                let buf = (&raw mut g_st().os_buf).cast::<u8>();
                xstrlcpy(buf.cast(), hd.as_ptr(), MAXPATHL);
                {
                    let _dst = buf;
                    let _dsize: usize = MAXPATHL;
                    let _dlen = crate::memory::cstr_len(_dst.cast());
                    crate::memory::xstrlcpy(
                        _dst.wrapping_add(_dlen).cast(),
                        homepath_ptr,
                        _dsize - _dlen,
                    );
                };
                if !crate::memory::cstr_is_empty(buf.cast()) {
                    var = buf;
                }
            }
        }
        if var.is_null() {
            var = os_uv_homedir();
        }

        if !var.is_null() && crate::memory::cstr_first_byte(var.cast()) == b'%' {
            let rest = crate::memory::cstr_as_bytes(var.wrapping_add(1).cast());
            if let Some(pos) = rest.iter().position(|&b| b == b'%') {
                let p = var.wrapping_add(1 + pos);
                let buf = (&raw mut g_st().os_buf).cast::<u8>();
                crate::memory::cstr_ncpy(buf.cast(), var.wrapping_add(1).cast(), pos);
                var = ptr::null_mut();
                if let Some(exp) = os_getenv_cstr(buf.cast())
                    && !exp.as_bytes().is_empty()
                    && exp.as_bytes().len() + crate::memory::cstr_len(p.cast()) < MAXPATHL
                {
                    xstrlcpy(buf.cast(), exp.as_ptr(), MAXPATHL);
                    {
                        let _dst = buf;
                        let _dsize: usize = MAXPATHL;
                        let _dlen = crate::memory::cstr_len(_dst.cast());
                        crate::memory::xstrlcpy(
                            _dst.wrapping_add(_dlen).cast(),
                            p.wrapping_add(1).cast(),
                            _dsize - _dlen,
                        );
                    };
                    var = buf;
                }
            }
        }

        if crate::memory::cstr_is_empty(var.cast()) {
            var = c"C:/".as_ptr().cast_mut().cast();
        }
    }

    #[cfg(not(windows))]
    {
        if var.is_null() {
            var = os_uv_homedir();
        }

        if !var.is_null() && !os_realpath(var.cast(), g_st().iobuff.as_mut_ptr(), IOSIZE).is_null()
        {
            var = g_st().iobuff.as_mut_ptr().cast();
        }

        if (crate::memory::cstr_is_empty(var.cast()))
            && os_dirname(&raw mut g_st().os_buf, MAXPATHL)
        {
            var = (&raw mut g_st().os_buf).cast();
        }
    }

    if !var.is_null() {
        env_st().homedir = to_slash_save(var.cast());
    }
}

pub fn free_homedir() {
    env_st().homedir = None;
}

/// Call `expand_env()` and store the result in an allocated string.
pub fn expand_env_save(src: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let src = src.into_c_char_ptr();
    expand_env_save_opt(src, false, None::<&CStr>)
}

/// Similar to `expand_env_save()` with `one` and `esc_chars`.
pub fn expand_env_save_opt(
    src: impl crate::strings::IntoCCharPtr,
    one: bool,
    esc_chars: impl crate::strings::IntoCCharPtr,
) -> std::ffi::CString {
    let src = src.into_c_char_ptr();
    let esc_chars = esc_chars.into_c_char_ptr();
    let mut p = [0u8; MAXPATHL];
    let _ = expand_env_esc(
        src,
        p.as_mut_ptr(),
        MAXPATHL as i32,
        esc_chars,
        one,
        None::<&CStr>,
    );
    cstr_to_cstring(p.as_ptr().cast())
}

/// Expand environment variable with path name.
pub fn expand_env<T>(src: impl crate::strings::IntoCCharPtr, dst: *mut T, dstlen: i32) -> usize {
    let src = src.into_c_char_ptr();
    expand_env_esc(src, dst, dstlen, None::<&CStr>, false, None::<&CStr>)
}

/// Expand environment variable with path name and escaping.
pub fn expand_env_esc<T>(
    srcp: impl crate::strings::IntoCCharPtr,
    dst: *mut T,
    mut dstlen: i32,
    esc_chars: impl crate::strings::IntoCCharPtr,
    one: bool,
    prefix: impl crate::strings::IntoCCharPtr,
) -> usize {
    let mut dst = dst.cast::<u8>();
    let esc_chars = esc_chars.into_c_char_ptr();
    let prefix = prefix.into_c_char_ptr();
    let srcp = srcp.into_c_char_ptr();
    {
        let mut at_start = true;
        let dst_start = dst;
        let prefix_len: usize = if prefix.is_null() {
            0
        } else {
            crate::memory::cstr_len(prefix)
        };

        let mut src: *mut u8 = skipwhite(srcp).cast();
        dstlen -= 1;
        while unsafe { *src } != 0 && dstlen > 0 {
            if crate::memory::cstr_first_byte(src.cast()) == b'`'
                && crate::memory::cstr_byte_at(src.cast(), 1) == b'='
            {
                let var = src;
                src = src.wrapping_add(2);
                let _ = skip_expr(&raw mut src, None);
                if crate::memory::cstr_first_byte(src.cast()) == b'`' {
                    src = src.wrapping_add(1);
                }
                let mut len = (src as usize) - (var as usize);
                if len > (dstlen as usize) {
                    len = dstlen as usize;
                }
                unsafe { ptr::copy_nonoverlapping(var, dst, len) };
                dst = dst.wrapping_add(len);
                dstlen -= len as i32;
                continue;
            }

            let mut copy_char = true;
            if crate::memory::cstr_first_byte(src.cast()) == b'$'
                || (crate::memory::cstr_first_byte(src.cast()) == b'~' && at_start)
            {
                let mut var_cstr: Option<std::ffi::CString> = None;
                let mut tail: *mut u8;
                let mut var: *mut u8;

                if crate::memory::cstr_first_byte(src.cast()) != b'~' {
                    tail = src.wrapping_add(1);
                    var = dst;
                    #[cfg(not(windows))]
                    let mut c = dstlen - 1;
                    #[cfg(windows)]
                    let c = dstlen - 1;

                    #[cfg(not(windows))]
                    if crate::memory::cstr_first_byte(tail.cast()) == b'{' {
                        tail = tail.wrapping_add(1);
                        while c > 0
                            && !crate::memory::cstr_is_empty(tail.cast())
                            && crate::memory::cstr_first_byte(tail.cast()) != b'}'
                        {
                            c -= 1;
                            unsafe {
                                *var = *tail;
                            }
                            var = var.wrapping_add(1);
                            tail = tail.wrapping_add(1);
                        }
                    } else {
                        let len = env_name_len(tail.cast()).min(c.max(0) as usize);
                        unsafe { ptr::copy_nonoverlapping(tail, var, len) };
                        var = var.wrapping_add(len);
                        tail = tail.wrapping_add(len);
                    }

                    #[cfg(windows)]
                    {
                        let len = env_name_len(tail.cast()).min(c.max(0) as usize);
                        unsafe { ptr::copy_nonoverlapping(tail, var, len) };
                        var = var.wrapping_add(len);
                        tail = tail.wrapping_add(len);
                    }

                    #[cfg(not(windows))]
                    if crate::memory::cstr_byte_at(src.cast(), 1) == b'{'
                        && crate::memory::cstr_first_byte(tail.cast()) != b'}'
                    {
                        var = ptr::null_mut();
                    } else {
                        if crate::memory::cstr_byte_at(src.cast(), 1) == b'{' {
                            tail = tail.wrapping_add(1);
                        }
                        unsafe {
                            *var = 0;
                        }
                        var_cstr = vim_getenv(dst.cast());
                        if let Some(ref c) = var_cstr {
                            to_slash(c.as_ptr().cast_mut().cast());
                        }
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                    }

                    #[cfg(windows)]
                    {
                        unsafe {
                            *var = 0;
                        }
                        var_cstr = vim_getenv(dst.cast());
                        if let Some(ref c) = var_cstr {
                            to_slash(c.as_ptr().cast_mut().cast());
                        }
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                    }
                } else if crate::memory::cstr_byte_at(src.cast(), 1) == 0
                    || vim_ispathsep(i32::from(crate::memory::cstr_byte_at(src.cast(), 1)))
                    || crate::strings::vim_haschr(
                        c" ,\t\n",
                        i32::from(crate::memory::cstr_byte_at(src.cast(), 1)),
                    )
                {
                    var = unsafe { (&raw const env_st().homedir).as_ref() }
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
                    tail = src.wrapping_add(1);
                } else {
                    #[cfg(not(windows))]
                    {
                        tail = src;
                        var = dst;
                        let mut c = dstlen - 1;
                        while c > 0
                            && unsafe { *tail } != 0
                            && vim_isfilec(i32::from(crate::memory::cstr_first_byte(tail.cast())))
                            && !vim_ispathsep(i32::from(crate::memory::cstr_first_byte(
                                tail.cast(),
                            )))
                        {
                            c -= 1;
                            unsafe {
                                *var = *tail;
                            }
                            var = var.wrapping_add(1);
                            tail = tail.wrapping_add(1);
                        }
                        unsafe {
                            *var = 0;
                        }
                        if crate::memory::cstr_is_empty(dst.cast()) {
                            var = ptr::null_mut();
                        } else {
                            var_cstr = os_get_userdir(crate::memory::cstr_as_opt_cstr(
                                dst.wrapping_add(1).cast(),
                            ));
                            var = var_cstr
                                .as_ref()
                                .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                        }
                    }
                    #[cfg(windows)]
                    {
                        var = ptr::null_mut();
                        tail = c"".as_ptr().cast_mut().cast();
                    }
                }

                if !esc_chars.is_null() && !var.is_null() {
                    let var_bytes = crate::memory::cstr_as_bytes(var.cast());
                    let esc_bytes = crate::memory::cstr_as_bytes(esc_chars);
                    if var_bytes.iter().any(|b| esc_bytes.contains(b)) {
                        var_cstr = Some(vim_strsave_escaped(var.cast(), esc_chars));
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                    }
                }

                if !crate::memory::cstr_is_empty(var.cast()) {
                    let c = crate::memory::cstr_len(var.cast());
                    if c + crate::memory::cstr_len(tail.cast()) + 1 < (dstlen as usize) {
                        unsafe { ptr::copy_nonoverlapping(var, dst, c + 1) };
                        dstlen -= c as i32;
                        let ends_with_sep =
                            after_pathsep(dst.cast(), dst.wrapping_add(c).cast()) != 0;
                        #[cfg(windows)]
                        let not_colon = crate::memory::cstr_byte_at(dst.cast(), c - 1) != b':';
                        #[cfg(not(windows))]
                        let not_colon = true;
                        if ends_with_sep
                            && not_colon
                            && vim_ispathsep(i32::from(crate::memory::cstr_first_byte(tail.cast())))
                        {
                            tail = tail.wrapping_add(1);
                        }
                        dst = dst.wrapping_add(c);
                        src = tail;
                        copy_char = false;
                    }
                }
                drop(var_cstr);
            }

            if copy_char {
                at_start = false;
                if crate::memory::cstr_first_byte(src.cast()) == b'\\'
                    && crate::memory::cstr_byte_at(src.cast(), 1) != 0
                {
                    unsafe {
                        *dst = *src;
                    }
                    dst = dst.wrapping_add(1);
                    src = src.wrapping_add(1);
                    dstlen -= 1;
                } else if (crate::memory::cstr_first_byte(src.cast()) == b' '
                    || crate::memory::cstr_first_byte(src.cast()) == b',')
                    && !one
                {
                    at_start = true;
                }
                if dstlen > 0 {
                    unsafe {
                        *dst = *src;
                    }
                    dst = dst.wrapping_add(1);
                    src = src.wrapping_add(1);
                    dstlen -= 1;

                    if !prefix.is_null()
                        && (src as usize) >= (srcp as usize) + prefix_len
                        && unsafe {
                            std::slice::from_raw_parts(src.wrapping_sub(prefix_len), prefix_len)
                        } == crate::memory::cstr_slice(prefix, prefix_len)
                    {
                        at_start = true;
                    }
                }
            }
        }
        unsafe {
            *dst = 0;
        }

        (dst as usize) - (dst_start as usize)
    }
}

fn vim_runtime_dir(vimdir: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let vimdir = vimdir.into_c_char_ptr();
    if vimdir.is_null() {
        return None;
    }
    {
        if crate::memory::cstr_is_empty(vimdir) {
            return None;
        }
        let vimdir_len = crate::memory::cstr_len(vimdir);
        let p_cstr = concat_fnames(
            NvimString {
                data: vimdir.cast_mut(),
                size: vimdir_len,
            },
            NvimString {
                data: RUNTIME_DIRNAME.as_ptr().cast_mut(),
                size: 7,
            },
            true,
        );
        os_isdir(p_cstr.as_ptr()).then_some(p_cstr)
    }
}

fn remove_tail(
    path: *mut u8,
    pend: *mut u8,
    dirname: impl crate::strings::IntoCCharPtr,
) -> *mut u8 {
    let dirname = dirname.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(dirname);
        if (pend as usize) > (path as usize) + len {
            let new_tail = pend.wrapping_sub(len + 1);
            if path_cmp(opt_vars().p_fic != 0, new_tail.cast(), dirname, len) == 0
                && (new_tail == path || after_pathsep(path.cast(), new_tail.cast()) != 0)
            {
                return new_tail;
            }
        }
    }
    pend
}

/// Iterates `$PATH`-like delimited list `val`.
pub fn vim_env_iter<T>(
    delim: u8,
    val: impl crate::strings::IntoCCharPtr,
    iter: impl crate::strings::IntoCCharPtr,
    dir: *mut *const T,
    len: &mut usize,
) -> *const T {
    let val: *const u8 = val.into_c_char_ptr().cast();
    let iter: *const u8 = iter.into_c_char_ptr().cast();
    {
        let varval = if iter.is_null() { val } else { iter };
        unsafe {
            *dir = varval.cast();
        }
        let bytes = crate::memory::cstr_as_bytes(varval.cast());
        if let Some(pos) = bytes.iter().position(|&b| b == delim) {
            *len = pos;
            varval.wrapping_add(pos + 1).cast()
        } else {
            *len = bytes.len();
            ptr::null()
        }
    }
}

/// Iterates `$PATH`-like delimited list `val` in reverse order.
pub fn vim_env_iter_rev<T>(
    delim: u8,
    val: impl crate::strings::IntoCCharPtr,
    iter: impl crate::strings::IntoCCharPtr,
    dir: *mut *const T,
    len: &mut usize,
) -> *const T {
    let val: *const u8 = val.into_c_char_ptr().cast();
    let iter: *const u8 = iter.into_c_char_ptr().cast();
    {
        let varend = if iter.is_null() {
            val.wrapping_add(crate::memory::cstr_len(val.cast()))
                .wrapping_sub(1)
        } else {
            iter
        };
        let varlen = (varend as usize).wrapping_sub(val as usize).wrapping_add(1);
        let colon: *const u8 = if varlen == 0 {
            ptr::null()
        } else {
            unsafe { std::slice::from_raw_parts(val, varlen) }
                .iter()
                .rposition(|&_b| _b == delim)
                .map_or(ptr::null(), |_i| val.wrapping_add(_i))
        };
        if colon.is_null() {
            *len = varlen;
            unsafe {
                *dir = val.cast();
            }
            return ptr::null();
        }
        unsafe {
            *dir = colon.wrapping_add(1).cast();
        }
        *len = (varend as usize) - (colon as usize);
        colon.wrapping_sub(1).cast()
    }
}

pub fn vim_get_prefix_from_exepath<T>(exe_name: *mut T) {
    let exe_name = exe_name.cast::<u8>();
    unsafe {
        xstrlcpy(exe_name.cast(), get_vim_var_str(VV_PROGPATH), MAXPATHL);
        let path_end: *mut u8 = path_tail_with_sep(exe_name.cast()).cast();
        *path_end = 0;
        let path_end2: *mut u8 = path_tail(exe_name.cast()).cast();
        *path_end2 = 0;
    }
}

/// Vim `getenv()` wrapper with special handling of `$HOME`, `$VIM`, `$VIMRUNTIME`.
pub fn vim_getenv(name: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let name = name.into_c_char_ptr();
    {
        assert!(!crate::memory::cstr_is_empty(get_vim_var_str(VV_PROGPATH)));

        #[cfg(windows)]
        if crate::memory::cstr_as_bytes(name) == b"HOME" {
            return env_st().homedir.clone();
        }

        let kos_env_path = os_getenv_cstr(name);
        #[cfg(windows)]
        if let Some(ref c) = kos_env_path
            && (crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"VIMRUNTIME")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"PATH")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"CDPATH")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"TMPDIR")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"TMP")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"TEMP")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"VIM")
                || crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"MYVIMRC"))
        {
            to_slash(c.as_ptr().cast_mut().cast());
        }
        if kos_env_path.is_some() {
            return kos_env_path;
        }

        let name_bytes = crate::memory::cstr_as_bytes(name);
        let vimruntime = name_bytes == b"VIMRUNTIME";
        if !vimruntime && name_bytes != b"VIM" {
            return None;
        }

        {
            let mut vim_path_cstr: Option<std::ffi::CString> = None;
            if vimruntime
                && default_vimruntime_dir().to_bytes().is_empty()
                && let Some(kos_cstr) = os_getenv_cstr(c"VIM")
            {
                vim_path_cstr = vim_runtime_dir(kos_cstr.as_ptr()).or(Some(kos_cstr));
            }

            if vim_path_cstr.is_none() {
                let mut vim_path: *mut u8 = ptr::null_mut();
                if !opt_vars().p_hf.is_null()
                    && !crate::strings::vim_haschr(opt_vars().p_hf, b'$' as i32)
                {
                    vim_path = opt_vars().p_hf.cast();
                }

                let mut exe_name = [0u8; MAXPATHL];
                if vim_path.is_null() {
                    vim_get_prefix_from_exepath(exe_name.as_mut_ptr());
                    if append_path(
                        exe_name.as_mut_ptr().cast(),
                        c"share/nvim/runtime/",
                        MAXPATHL,
                    ) {
                        vim_path = exe_name.as_mut_ptr();
                    }
                }

                if !vim_path.is_null() {
                    let mut vim_path_end: *mut u8 = path_tail(vim_path.cast()).cast();
                    if vim_path == opt_vars().p_hf.cast() {
                        vim_path_end = remove_tail(vim_path, vim_path_end, c"doc");
                    }
                    if !vimruntime {
                        vim_path_end = remove_tail(vim_path, vim_path_end, RUNTIME_DIRNAME);
                    }
                    if (vim_path_end as usize) > (vim_path as usize)
                        && after_pathsep(vim_path.cast(), vim_path_end.cast()) != 0
                    {
                        vim_path_end = vim_path_end.wrapping_sub(1);
                    }
                    assert!((vim_path_end as usize) >= (vim_path as usize));
                    let cand = std::ffi::CString::new(crate::memory::cstr_slice(
                        vim_path.cast(),
                        (vim_path_end as usize) - (vim_path as usize),
                    ))
                    .unwrap_or_default();
                    if os_isdir(cand.as_ptr()) {
                        vim_path_cstr = Some(cand);
                    }
                }
            }

            if vim_path_cstr.is_none() {
                if vimruntime && !default_vimruntime_dir().to_bytes().is_empty() {
                    vim_path_cstr = Some(default_vimruntime_dir().to_owned());
                } else if !default_vim_dir().to_bytes().is_empty() {
                    if vimruntime {
                        vim_path_cstr = vim_runtime_dir(default_vim_dir());
                    }
                    if vim_path_cstr.is_none() {
                        vim_path_cstr = Some(default_vim_dir().to_owned());
                    }
                }
            }

            if let Some(vim_path_cstr) = vim_path_cstr {
                if vimruntime {
                    let _ = os_setenv(c"VIMRUNTIME", vim_path_cstr.as_ptr(), 1);
                    g_st().didset_vimruntime = true;
                } else {
                    let _ = os_setenv(c"VIM", vim_path_cstr.as_ptr(), 1);
                    g_st().didset_vim = true;
                }
                Some(vim_path_cstr)
            } else {
                None
            }
        }
    }
}

/// Replace home directory by `"~"` in each space or comma separated file name in `src`.
pub fn home_replace<T>(
    buf: Option<&FileBuf>,
    src: impl crate::strings::IntoCCharPtr,
    dst: *mut T,
    mut dstlen: usize,
    one: bool,
) -> usize {
    let dst = dst.cast::<u8>();
    let mut src: *const u8 = src.into_c_char_ptr().cast();
    {
        if src.is_null() {
            unsafe {
                *dst = 0;
            }
            return 0;
        }

        if buf.is_some() && buf.unwrap().b_help {
            let dlen = xstrlcpy(dst.cast(), path_tail(src.cast()), dstlen);
            return dlen.min(dstlen - 1);
        }

        let homedir_ptr: *mut u8 = unsafe { (&raw const env_st().homedir).as_ref() }
            .unwrap()
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
        let dirlen = if homedir_ptr.is_null() {
            0
        } else {
            crate::memory::cstr_len(homedir_ptr.cast())
        };

        #[cfg(windows)]
        let mut homedir_env_cstr = os_getenv_cstr(c"HOME");
        #[cfg(not(windows))]
        let homedir_env_cstr = os_getenv_cstr(c"HOME");
        #[cfg(windows)]
        if homedir_env_cstr.is_none() {
            homedir_env_cstr = os_getenv_cstr(c"USERPROFILE");
        }
        if let Some(ref c) = homedir_env_cstr {
            to_slash(c.as_ptr().cast_mut().cast());
        }
        let homedir_env = homedir_env_cstr
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let mut homedir_env_mod = homedir_env.cast::<u8>();
        let mut homedir_env_mod_vec: Vec<u8>;

        if !homedir_env_mod.is_null()
            && crate::memory::cstr_first_byte(homedir_env_mod.cast()) == b'~'
        {
            let mut usedlen: usize = 0;
            let mut flen = crate::memory::cstr_len(homedir_env_mod.cast());
            let mut fbuf: Option<Vec<u8>> = None;
            let _ = modify_fname(
                c":p",
                false,
                &mut usedlen,
                &raw mut homedir_env_mod,
                &mut fbuf,
                &mut flen,
                false,
            );
            flen = crate::memory::cstr_len(homedir_env_mod.cast());
            assert_ne!(homedir_env_mod, homedir_env.cast());
            if let Some(mod_vec) = fbuf {
                homedir_env_mod_vec = mod_vec;
                homedir_env_mod = homedir_env_mod_vec.as_mut_ptr();
            }
            if flen > 0
                && vim_ispathsep(i32::from(crate::memory::cstr_byte_at(
                    homedir_env_mod.cast(),
                    flen - 1,
                )))
            {
                unsafe {
                    *homedir_env_mod.wrapping_add(flen - 1) = 0;
                }
            }
        }

        let envlen = if homedir_env_mod.is_null() {
            0
        } else {
            crate::memory::cstr_len(homedir_env_mod.cast())
        };

        if !one {
            src = skipwhite(src.cast()).cast();
        }
        let mut dst_p = dst;
        while unsafe { *src } != 0 && dstlen > 0 {
            let mut p = homedir_ptr;
            let mut len = dirlen;
            loop {
                let cmp_ok =
                    len != 0 && path_cmp(opt_vars().p_fic != 0, src.cast(), p.cast(), len) == 0;

                if cmp_ok
                    && (vim_ispathsep(i32::from(crate::memory::cstr_byte_at(src.cast(), len)))
                        || (!one
                            && (crate::memory::cstr_byte_at(src.cast(), len) == b','
                                || crate::memory::cstr_byte_at(src.cast(), len) == b' '))
                        || crate::memory::cstr_byte_at(src.cast(), len) == 0)
                {
                    src = src.wrapping_add(len);
                    dstlen -= 1;
                    if dstlen > 0 {
                        unsafe {
                            *dst_p = b'~';
                        }
                        dst_p = dst_p.wrapping_add(1);
                    }
                    break;
                }
                if p == homedir_env_mod {
                    break;
                }
                p = homedir_env_mod;
                len = envlen;
            }

            if dstlen == 0 {
                break;
            }
            while unsafe { *src } != 0
                && (one
                    || (crate::memory::cstr_first_byte(src.cast()) != b','
                        && crate::memory::cstr_first_byte(src.cast()) != b' '))
            {
                dstlen -= 1;
                if dstlen == 0 {
                    break;
                }
                unsafe {
                    *dst_p = *src;
                }
                dst_p = dst_p.wrapping_add(1);
                src = src.wrapping_add(1);
            }
            if dstlen == 0 {
                break;
            }
            while crate::memory::cstr_first_byte(src.cast()) == b' '
                || crate::memory::cstr_first_byte(src.cast()) == b','
            {
                dstlen -= 1;
                if dstlen == 0 {
                    break;
                }
                unsafe {
                    *dst_p = *src;
                }
                dst_p = dst_p.wrapping_add(1);
                src = src.wrapping_add(1);
            }
        }

        unsafe {
            *dst_p = 0;
        }
        (dst_p as usize) - (dst as usize)
    }
}

/// Like `home_replace`, store the replaced string in an owned `CString`.
pub fn home_replace_save(
    buf: Option<&FileBuf>,
    src: impl crate::strings::IntoCCharPtr,
) -> std::ffi::CString {
    let src = src.into_c_char_ptr();
    {
        let mut len: usize = 3;
        if !src.is_null() {
            len += crate::memory::cstr_len(src);
        }
        let mut dst = vec![0u8; len];
        let _ = home_replace(buf, src, dst.as_mut_ptr(), len, true);
        cstr_to_cstring(dst.as_ptr().cast())
    }
}

/// Function given to `ExpandGeneric()` to obtain an environment variable name.
pub fn get_env_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    assert!(idx >= 0);
    if let Some(envname) = os_getenvname_at_index(idx as usize) {
        xstrlcpy(
            xp.xp_buf.as_mut_ptr().cast(),
            envname.as_ptr(),
            EXPAND_BUF_LEN,
        );
        return Some(unsafe { CStr::from_ptr(xp.xp_buf.as_ptr().cast()) });
    }
    None
}

/// Appends the head of `fname` to `$PATH` and sets it in the environment.
#[cfg_attr(not(windows), cfg(test))]
pub fn os_setenv_append_path(fname: impl crate::strings::IntoCCharPtr) -> bool {
    let fname = fname.into_c_char_ptr();
    {
        if !path_is_absolute(fname) {
            internal_error(c"os_setenv_append_path()");
            return false;
        }
        let tail = path_tail_with_sep(fname.cast_mut());
        let dirlen = (tail as usize) - (fname as usize);
        let buf = (&raw mut g_st().os_buf).cast::<u8>();
        assert!((tail as usize) >= (fname as usize) && dirlen + 1 < MAXPATHL);
        crate::memory::cstr_ncpy(buf.cast(), fname, dirlen);
        let path_cstr = os_getenv_cstr(c"PATH");
        let pathlen = path_cstr.as_ref().map_or(0, |c| c.as_bytes().len());
        let newlen = pathlen + dirlen + 2;
        let mut retval = false;
        if newlen < MAX_ENVPATHLEN {
            let mut temp_vec = vec![0u8; newlen];
            let temp = temp_vec.as_mut_ptr();
            if let Some(ref path) = path_cstr {
                xstrlcpy(temp.cast(), path.as_ptr(), newlen);
                if path.as_bytes().last().copied() != Some(ENV_SEPCHAR) {
                    {
                        let _dst = temp;
                        let _dsize: usize = newlen;
                        let _dlen = crate::memory::cstr_len(_dst.cast());
                        crate::memory::xstrlcpy(
                            _dst.wrapping_add(_dlen).cast(),
                            ENV_SEPSTR,
                            _dsize - _dlen,
                        );
                    };
                }
            }
            {
                let _dst = temp;
                let _dsize: usize = newlen;
                let _dlen = crate::memory::cstr_len(_dst.cast());
                crate::memory::xstrlcpy(
                    _dst.wrapping_add(_dlen).cast(),
                    buf.cast(),
                    _dsize - _dlen,
                );
            };
            let _ = os_setenv(c"PATH", temp.cast(), 1);
            retval = true;
        }
        retval
    }
}

/// Returns true if `sh` looks like it resolves to `"cmd.exe"`.
#[cfg_attr(not(windows), cfg(test))]
pub fn os_shell_is_cmdexe(sh: impl crate::strings::IntoCCharPtr) -> bool {
    let sh = sh.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(sh) {
            return false;
        }
        if crate::memory::cstr_as_bytes(sh).eq_ignore_ascii_case(b"$COMSPEC") {
            let mut buf = [0u8; MAXPATHL];
            let comspec = os_getenv_buf(c"COMSPEC", buf.as_mut_ptr(), buf.len());
            return !comspec.is_null()
                && crate::memory::cstr_as_bytes(path_tail(comspec.cast()))
                    .eq_ignore_ascii_case(b"cmd.exe");
        }
        if crate::memory::cstr_as_bytes(sh).eq_ignore_ascii_case(b"cmd.exe")
            || crate::memory::cstr_as_bytes(sh).eq_ignore_ascii_case(b"cmd")
        {
            return true;
        }
        crate::memory::cstr_as_bytes(path_tail(sh)).eq_ignore_ascii_case(b"cmd.exe")
    }
}

/// Removes environment variable `var` and takes care of side effects.
pub fn vim_unsetenv_ext(var: impl crate::strings::IntoCCharPtr) {
    let var = var.into_c_char_ptr();
    {
        let _ = os_unsetenv(var);
        if crate::memory::cstr_as_bytes(var).eq_ignore_ascii_case(b"VIM") {
            g_st().didset_vim = false;
        } else if crate::memory::cstr_as_bytes(var).eq_ignore_ascii_case(b"VIMRUNTIME") {
            g_st().didset_vimruntime = false;
        }
    }
}

/// Set environment variable `name` and take care of side effects.
pub fn vim_setenv_ext(
    name: impl crate::strings::IntoCCharPtr,
    val: impl crate::strings::IntoCCharPtr,
) {
    let name = name.into_c_char_ptr();
    let val = val.into_c_char_ptr();
    {
        let _ = os_setenv(name, val, 1);
        if crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"HOME") {
            init_homedir();
        } else {
            if g_st().didset_vim && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"VIM")
            {
                g_st().didset_vim = false;
            } else if g_st().didset_vimruntime
                && crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"VIMRUNTIME")
            {
                g_st().didset_vimruntime = false;
            }
        }
    }
}

/// Restore a previous environment variable value, or unset it if `None`.
#[cfg(windows)]
pub fn restore_env_var(name: &std::ffi::CStr, old_value: Option<std::ffi::CString>) {
    if let Some(old_value) = old_value {
        let _ = os_setenv(name.as_ptr(), old_value.as_ptr(), 1);
        return;
    }
    let _ = os_unsetenv(name.as_ptr());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_name_len_and_vim_env_iter_work() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            assert_eq!(env_name_len(c"FOO_123/bar"), 7);
            assert_eq!(env_name_len(c"-invalid"), 0);

            let val = c"/a:/bb:/ccc";
            let mut iter: *const u8 = ptr::null();
            let mut dir: *const u8 = ptr::null();
            let mut len: usize = 0;

            iter = vim_env_iter(b':', val.as_ptr(), iter.cast(), &raw mut dir, &mut len);
            assert_eq!(len, 2);
            assert!(!iter.is_null());

            iter = vim_env_iter(b':', val.as_ptr(), iter.cast(), &raw mut dir, &mut len);
            assert_eq!(len, 3);
            assert!(!iter.is_null());

            iter = vim_env_iter(b':', val.as_ptr(), iter.cast(), &raw mut dir, &mut len);
            assert_eq!(len, 4);
            assert!(iter.is_null());

            if crate::memory::cstr_is_empty(crate::eval::vars::get_vim_var_str(VV_PROGPATH)) {
                crate::eval::vars::set_vim_var_string(VV_PROGPATH, c"nvim", -1);
            }

            assert!(!os_env_exists(c"", false));
            assert!(!os_env_exists(c"   ", true));
            let k = c"NVIM_RUST_ENV_TEST_KEY";
            let _ = os_unsetenv(k.as_ptr());
            assert!(!os_env_exists(k.as_ptr(), false));
            assert_eq!(os_setenv(k.as_ptr(), c"val1", 1), 0);
            assert!(os_env_exists(k.as_ptr(), false));
            assert!(os_env_exists(k.as_ptr(), true));

            let got = os_getenv_cstr(k.as_ptr()).unwrap();
            assert_eq!(got.to_str().unwrap(), "val1");

            let mut buf = [0u8; 64];
            assert_eq!(os_setenv(k.as_ptr(), c"val2", 0), 0);
            assert_eq!(
                crate::memory::cstr_from_ptr(
                    os_getenv_buf(k.as_ptr(), buf.as_mut_ptr(), buf.len()).cast()
                )
                .to_str()
                .unwrap(),
                "val1"
            );
            assert_eq!(os_setenv(k.as_ptr(), c"val2", 1), 0);
            assert_eq!(
                crate::memory::cstr_from_ptr(
                    os_getenv_buf(k.as_ptr(), buf.as_mut_ptr(), buf.len()).cast()
                )
                .to_str()
                .unwrap(),
                "val2"
            );

            assert!(!os_getenv_buf(k.as_ptr(), buf.as_mut_ptr(), buf.len()).is_null());
            assert_eq!(
                crate::memory::cstr_from_ptr(buf.as_ptr().cast())
                    .to_str()
                    .unwrap(),
                "val2"
            );

            expand_env_esc(
                c"$NVIM_RUST_ENV_TEST_KEY/sub",
                buf.as_mut_ptr(),
                buf.len() as i32,
                ptr::null_mut(),
                true,
                ptr::null_mut(),
            );
            assert_eq!(
                crate::memory::cstr_from_ptr(buf.as_ptr().cast())
                    .to_str()
                    .unwrap(),
                "val2/sub"
            );
            let _ = os_unsetenv(k.as_ptr());

            assert!(os_get_pid() > 0);
            os_get_hostname(buf.as_mut_ptr(), buf.len());
            assert!(!crate::memory::cstr_as_bytes(buf.as_ptr().cast()).is_empty());
            assert!(os_getenvname_at_index(usize::MAX).is_none());

            assert!(os_shell_is_cmdexe(c"cmd.exe"));
            assert!(os_shell_is_cmdexe(c"CMD"));
            assert!(!os_shell_is_cmdexe(c"powershell"));

            let abs_path = if cfg!(windows) {
                c"C:\\foo\\bar\\baz.exe"
            } else {
                c"/foo/bar/baz.exe"
            };
            assert!(os_setenv_append_path(abs_path.as_ptr()));
        }
    }
}
