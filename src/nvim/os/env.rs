//! Environment variable inspection and path expansion (`src/nvim/os/env.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::FileBuf;
#[cfg(unix)]
use crate::charset::vim_isfilec;
pub(crate) use crate::cmdexpand::EXPAND_BUF_LEN;
pub use crate::cmdexpand::ExpandCtx;
use crate::eval::fs::modify_fname;
use crate::eval::skip_expr;
use crate::eval::vars::{VV_PROGPATH, get_vim_var_str};
use crate::globals::*;
use crate::mbyte::utf_isalnum;
#[cfg(windows)]
use crate::mbyte::{utf_valid_string, utf16_to_utf8};
use crate::memory::{cstr_from_bytes, cstr_to_cstring};
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
    ENV_STATE.as_mut()
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
fn env_name_valid(bytes: &[u8]) -> bool {
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

/// Gets the length (bytes) of the env var name at `bytes`.
pub fn env_name_len(bytes: &[u8]) -> usize {
    let mut idx = 0usize;
    while idx < bytes.len() {
        let b = bytes[idx];
        if b < 0x80 {
            if !b.is_ascii_alphanumeric() && b != b'_' {
                break;
            }
            idx += 1;
            continue;
        }
        let rem = &bytes[idx..];
        let l = crate::mbyte::utf_slice2len(rem);
        if l == 1 || !utf_isalnum(crate::mbyte::utf_slice2char(rem)) {
            break;
        }
        idx += l.min(rem.len());
    }
    idx
}

/// Like `getenv()`, but returns `None` if the variable is empty, without C heap allocations.
pub fn os_getenv_cstr(name: &CStr) -> Option<std::ffi::CString> {
    if !env_name_valid(name.to_bytes()) {
        return None;
    }
    let name_ptr = name.as_ptr();
    let mut size = INIT_SIZE;
    let mut buf = [0u8; INIT_SIZE];
    let mut r = unsafe { uv_os_getenv(name_ptr, buf.as_mut_ptr().cast(), &mut size) };
    let res = if r == UV_ENOBUFS {
        let mut vec_buf = vec![0u8; size + 1];
        r = unsafe { uv_os_getenv(name_ptr, vec_buf.as_mut_ptr().cast(), &mut size) };
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

/// Like `getenv()`, but stores the value in `buf` instead of allocating.
pub fn os_getenv_buf<'a>(name: &CStr, buf: &'a mut [u8]) -> Option<&'a CStr> {
    if !env_name_valid(name.to_bytes()) || buf.is_empty() {
        return None;
    }
    let name_ptr = name.as_ptr();
    let bufsize = buf.len();
    let mut size = bufsize;
    let mut r = unsafe { uv_os_getenv(name_ptr, buf.as_mut_ptr().cast(), &mut size) };
    if r == UV_ENOBUFS {
        let mut e = vec![0u8; size];
        r = unsafe { uv_os_getenv(name_ptr, e.as_mut_ptr().cast(), &mut size) };
        if r == 0 && size != 0 && e[0] != 0 {
            let copy_len = bufsize.min(size) - 1;
            buf[..copy_len].copy_from_slice(&e[..copy_len]);
            buf[copy_len] = 0;
        }
    }
    assert_ne!(r, UV_EINVAL);

    if r != 0 || size == 0 || buf[0] == 0 {
        return None;
    }
    Some(crate::memory::cstr_until_nul(buf))
}

/// Returns true if environment variable `name` is defined (even if empty).
pub fn os_env_exists(name: &CStr, nonempty: bool) -> bool {
    if !env_name_valid(name.to_bytes()) {
        return false;
    }
    let mut buf = [0u8; 2];
    let mut size = buf.len();
    let r = unsafe { uv_os_getenv(name.as_ptr(), buf.as_mut_ptr().cast(), &mut size) };
    assert_ne!(r, UV_EINVAL);
    (r == 0 && (!nonempty || size > 0)) || r == UV_ENOBUFS
}

/// Sets an environment variable.
pub fn os_setenv(name: &CStr, value: &CStr, overwrite: i32) -> i32 {
    if !env_name_valid(name.to_bytes()) {
        return -1;
    }
    #[cfg(windows)]
    {
        if overwrite == 0 && os_env_exists(name, true) {
            return 0;
        }
        if value.to_bytes().is_empty() {
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
        if name.to_bytes().eq_ignore_ascii_case(b"LC_ALL")
            || name.to_bytes().eq_ignore_ascii_case(b"LANGUAGE")
            || name.to_bytes().eq_ignore_ascii_case(b"LANG")
            || name.to_bytes().eq_ignore_ascii_case(b"LC_MESSAGES")
        {
            let r = unsafe { _putenv_s(name.as_ptr().cast(), value.as_ptr().cast()) };
            assert_eq!(r, 0);
        }
        let r = unsafe { uv_os_setenv(name.as_ptr(), value.as_ptr()) };
        assert_ne!(r, UV_EINVAL);
        if r == 0 { 0 } else { -1 }
    }
}

/// Unset environment variable.
pub fn os_unsetenv(name: &CStr) -> i32 {
    if !env_name_valid(name.to_bytes()) {
        return -1;
    }
    let r = unsafe { uv_os_unsetenv(name.as_ptr()) };
    if r == 0 { 0 } else { -1 }
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
pub fn os_get_hostname(hostname: &mut [u8]) {
    #[cfg(not(windows))]
    unsafe {
        let size = hostname.len();
        if gethostname(hostname.as_mut_ptr(), size) < 0 {
            if !hostname.is_empty() {
                hostname[0] = 0;
            }
        } else if size > 0 {
            hostname[size - 1] = 0;
        }
    }
    #[cfg(windows)]
    {
        let mut host_utf16 = [0u16; 16];
        let mut host_wsize = host_utf16.len() as u32;
        if unsafe { GetComputerNameW(host_utf16.as_mut_ptr(), &mut host_wsize) } == 0 {
            if !hostname.is_empty() {
                hostname[0] = 0;
            }
            let err = unsafe { GetLastError() };
            semsg!(c"GetComputerNameW failed: %d", err);
            return;
        }
        host_utf16[host_wsize as usize] = 0;

        let mut host_utf8: Option<std::ffi::CString> = None;
        let conversion_result = utf16_to_utf8(&host_utf16, -1, &mut host_utf8);
        if conversion_result != 0 {
            semsg!(c"utf16_to_utf8 failed: %d", conversion_result);
            return;
        }
        if let Some(host_cstr) = host_utf8 {
            crate::memory::copy_bytes_to_c_buf(hostname, host_cstr.to_bytes());
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

fn os_uv_homedir() -> Option<&'static CStr> {
    unsafe {
        let buf = (&raw mut env_st().homedir_buf).cast::<u8>();
        *buf = 0;
        let mut homedir_size = MAXPATHL;
        let ret_value = uv_os_homedir(buf.cast(), &mut homedir_size);
        if ret_value == 0 && homedir_size < MAXPATHL {
            return Some(CStr::from_ptr(buf.cast()));
        }
        *buf = 0;
        None
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
                crate::memory::copy_bytes_to_c_buf(g_st().os_buf_bytes_mut(), hd.as_bytes());
                crate::memory::append_bytes_to_c_buf(
                    g_st().os_buf_bytes_mut(),
                    crate::memory::cstr_as_bytes(homepath_ptr),
                );
                if !crate::memory::cstr_is_empty(buf.cast()) {
                    var = buf;
                }
            }
        }
        if var.is_null() {
            var = os_uv_homedir().map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
        }

        if !var.is_null() && crate::memory::cstr_first_byte(var.cast()) == b'%' {
            let rest = crate::memory::cstr_as_bytes(var.wrapping_add(1).cast());
            if let Some(pos) = rest.iter().position(|&b| b == b'%') {
                let p = var.wrapping_add(1 + pos);
                let buf = (&raw mut g_st().os_buf).cast::<u8>();
                for (i, &b) in rest[..pos].iter().enumerate() {
                    g_st().os_buf[i] = b as crate::strings::RawCChar;
                }
                g_st().os_buf[pos] = 0;
                var = ptr::null_mut();
                if let Some(exp) = os_getenv_cstr(crate::memory::cstr_from_ptr(buf.cast()))
                    && !exp.as_bytes().is_empty()
                    && exp.as_bytes().len() + crate::memory::cstr_len(p.cast()) < MAXPATHL
                {
                    crate::memory::copy_bytes_to_c_buf(g_st().os_buf_bytes_mut(), exp.as_bytes());
                    crate::memory::append_bytes_to_c_buf(
                        g_st().os_buf_bytes_mut(),
                        crate::memory::cstr_as_bytes(p.wrapping_add(1).cast()),
                    );
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
            var = os_uv_homedir().map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
        }

        if !var.is_null()
            && os_realpath(
                crate::memory::cstr_from_ptr(var.cast()),
                g_st().iobuff_bytes_mut(),
            )
            .is_some()
        {
            var = g_st().iobuff.as_mut_ptr().cast();
        }

        let os_buf_slice = unsafe {
            std::slice::from_raw_parts_mut(g_st().os_buf.as_mut_ptr().cast::<u8>(), MAXPATHL)
        };
        if crate::memory::cstr_is_empty(var.cast()) && os_dirname(os_buf_slice) {
            var = (&raw mut g_st().os_buf).cast();
        }
    }

    if !var.is_null() {
        env_st().homedir = to_slash_save(crate::memory::cstr_as_opt_cstr(var.cast()));
    }
}

pub fn free_homedir() {
    env_st().homedir = None;
}

/// Call `expand_env()` and store the result in an allocated string.
pub fn expand_env_save(src: &CStr) -> std::ffi::CString {
    expand_env_save_opt(src, false, None)
}

/// Similar to `expand_env_save()` with `one` and `esc_chars`.
pub fn expand_env_save_opt(src: &CStr, one: bool, esc_chars: Option<&CStr>) -> std::ffi::CString {
    let mut p = [0u8; MAXPATHL];
    let _ = expand_env_esc(src, &mut p, esc_chars, one, None);
    cstr_to_cstring(p.as_ptr().cast())
}

/// Expand environment variable with path name.
pub fn expand_env(src: &CStr, dst: &mut [u8]) -> usize {
    expand_env_esc(src, dst, None, false, None)
}

/// Expand environment variable with path name and escaping.
pub fn expand_env_esc(
    srcp_in: &CStr,
    dst_slice: &mut [u8],
    esc_chars: Option<&CStr>,
    one: bool,
    prefix: Option<&CStr>,
) -> usize {
    let dst = dst_slice.as_mut_ptr();
    let esc_chars = esc_chars.map_or(ptr::null(), CStr::as_ptr);
    let prefix = prefix.map_or(ptr::null(), CStr::as_ptr);
    let srcp = srcp_in.as_ptr();
    let mut dst_idx = 0usize;
    let mut dst_rem = dst_slice.len().saturating_sub(1) as i32;
    let mut at_start = true;
    let prefix_bytes = crate::memory::cstr_as_bytes(prefix);

    let mut src: *mut u8 = srcp
        .wrapping_add(crate::charset::skipwhite_len(crate::memory::cstr_as_bytes(
            srcp.cast(),
        )))
        .cast_mut()
        .cast();
    while crate::memory::cstr_first_byte(src.cast()) != 0 && dst_rem > 0 {
        let src_bytes = crate::memory::cstr_as_bytes(src.cast());
        if src_bytes.starts_with(b"`=") {
            let var = src;
            src = src.wrapping_add(2);
            let mut src_cstr = unsafe { CStr::from_ptr(src.cast()) };
            let _ = skip_expr(&mut src_cstr, None);
            src = src_cstr.as_ptr().cast_mut().cast();
            if crate::memory::cstr_first_byte(src.cast()) == b'`' {
                src = src.wrapping_add(1);
            }
            let len = ((src as usize) - (var as usize)).min(dst_rem as usize);
            dst_slice[dst_idx..dst_idx + len].copy_from_slice(&src_bytes[..len]);
            dst_idx += len;
            dst_rem -= len as i32;
            continue;
        }

        let mut copy_char = true;
        let first_b = src_bytes[0];
        if first_b == b'$' || (first_b == b'~' && at_start) {
            let mut var_cstr: Option<std::ffi::CString> = None;
            let mut tail: *mut u8;
            let mut var: *mut u8;

            if first_b != b'~' {
                tail = src.wrapping_add(1);
                let tail_bytes = &src_bytes[1..];
                #[cfg(not(windows))]
                let mut c = dst_rem - 1;
                #[cfg(windows)]
                let c = dst_rem - 1;
                #[cfg(not(windows))]
                let mut var_len = 0usize;
                #[cfg(windows)]
                let var_len: usize;

                #[cfg(not(windows))]
                if tail_bytes.first() == Some(&b'{') {
                    tail = tail.wrapping_add(1);
                    let mut ti = 1usize;
                    while c > 0 && ti < tail_bytes.len() && tail_bytes[ti] != b'}' {
                        c -= 1;
                        dst_slice[dst_idx + var_len] = tail_bytes[ti];
                        var_len += 1;
                        ti += 1;
                        tail = tail.wrapping_add(1);
                    }
                } else {
                    let len = env_name_len(tail_bytes).min(c.max(0) as usize);
                    dst_slice[dst_idx..dst_idx + len].copy_from_slice(&tail_bytes[..len]);
                    var_len = len;
                    tail = tail.wrapping_add(len);
                }

                #[cfg(windows)]
                {
                    let len = env_name_len(tail_bytes).min(c.max(0) as usize);
                    dst_slice[dst_idx..dst_idx + len].copy_from_slice(&tail_bytes[..len]);
                    var_len = len;
                    tail = tail.wrapping_add(len);
                }

                #[cfg(not(windows))]
                if src_bytes.get(1) == Some(&b'{')
                    && crate::memory::cstr_first_byte(tail.cast()) != b'}'
                {
                    var = ptr::null_mut();
                } else {
                    if src_bytes.get(1) == Some(&b'{') {
                        tail = tail.wrapping_add(1);
                    }
                    dst_slice[dst_idx + var_len] = 0;
                    var_cstr = vim_getenv(crate::memory::cstr_until_nul(&dst_slice[dst_idx..]));
                    if let Some(ref c) = var_cstr {
                        to_slash(crate::memory::cstr_as_mut_bytes(
                            c.as_ptr().cast_mut().cast(),
                        ));
                    }
                    var = var_cstr
                        .as_ref()
                        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                }

                #[cfg(windows)]
                {
                    dst_slice[dst_idx + var_len] = 0;
                    var_cstr = vim_getenv(crate::memory::cstr_until_nul(&dst_slice[dst_idx..]));
                    if let Some(ref c) = var_cstr {
                        to_slash(crate::memory::cstr_as_mut_bytes(
                            c.as_ptr().cast_mut().cast(),
                        ));
                    }
                    var = var_cstr
                        .as_ref()
                        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                }
            } else if src_bytes.len() == 1
                || vim_ispathsep(i32::from(src_bytes[1]))
                || matches!(src_bytes[1], b' ' | b',' | b'\t' | b'\n')
            {
                var = env_st()
                    .homedir
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
                tail = src.wrapping_add(1);
            } else {
                #[cfg(not(windows))]
                {
                    tail = src;
                    let mut c = dst_rem - 1;
                    let mut var_len = 0usize;
                    while c > 0
                        && var_len < src_bytes.len()
                        && vim_isfilec(i32::from(src_bytes[var_len]))
                        && !vim_ispathsep(i32::from(src_bytes[var_len]))
                    {
                        c -= 1;
                        dst_slice[dst_idx + var_len] = src_bytes[var_len];
                        var_len += 1;
                        tail = tail.wrapping_add(1);
                    }
                    dst_slice[dst_idx + var_len] = 0;
                    if dst_slice[dst_idx] == 0 {
                        var = ptr::null_mut();
                    } else {
                        var_cstr = os_get_userdir(crate::memory::cstr_as_opt_cstr(
                            dst_slice[dst_idx + 1..].as_ptr().cast(),
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
                    var_cstr = Some(vim_strsave_escaped(var_bytes, esc_bytes));
                    var = var_cstr
                        .as_ref()
                        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut().cast());
                }
            }

            let var_bytes_with_nul = crate::memory::cstr_from_ptr(var.cast()).to_bytes_with_nul();
            if var_bytes_with_nul.len() > 1 {
                let c = var_bytes_with_nul.len() - 1;
                let tail_bytes = crate::memory::cstr_as_bytes(tail.cast());
                if c + tail_bytes.len() + 1 < (dst_rem as usize) {
                    dst_slice[dst_idx..dst_idx + c + 1].copy_from_slice(var_bytes_with_nul);
                    dst_rem -= c as i32;
                    let cur_dst = dst.wrapping_add(dst_idx);
                    let ends_with_sep = after_pathsep(crate::memory::ptr_span_bytes(
                        cur_dst.cast(),
                        cur_dst.wrapping_add(c).cast(),
                    )) != 0;
                    #[cfg(windows)]
                    let not_colon = dst_slice[dst_idx + c - 1] != b':';
                    #[cfg(not(windows))]
                    let not_colon = true;
                    if ends_with_sep
                        && not_colon
                        && tail_bytes
                            .first()
                            .is_some_and(|&b| vim_ispathsep(i32::from(b)))
                    {
                        tail = tail.wrapping_add(1);
                    }
                    dst_idx += c;
                    src = tail;
                    copy_char = false;
                }
            }
            drop(var_cstr);
        }

        if copy_char {
            at_start = false;
            if first_b == b'\\' && src_bytes.len() > 1 {
                dst_slice[dst_idx] = b'\\';
                dst_idx += 1;
                src = src.wrapping_add(1);
                dst_rem -= 1;
            } else if matches!(first_b, b' ' | b',') && !one {
                at_start = true;
            }
            if dst_rem > 0 {
                dst_slice[dst_idx] = crate::memory::cstr_first_byte(src.cast());
                dst_idx += 1;
                src = src.wrapping_add(1);
                dst_rem -= 1;

                if !prefix_bytes.is_empty()
                    && (src as usize) >= (srcp as usize) + prefix_bytes.len()
                    && crate::memory::cstr_slice(
                        src.wrapping_sub(prefix_bytes.len()).cast(),
                        prefix_bytes.len(),
                    ) == prefix_bytes
                {
                    at_start = true;
                }
            }
        }
    }
    if !dst_slice.is_empty() {
        dst_slice[dst_idx] = 0;
    }

    dst_idx
}

fn vim_runtime_dir(vimdir: Option<&CStr>) -> Option<std::ffi::CString> {
    let vimdir = vimdir?;
    if vimdir.to_bytes().is_empty() {
        return None;
    }
    let p_cstr = concat_fnames(
        NvimString {
            data: vimdir.as_ptr().cast_mut(),
            size: vimdir.to_bytes().len(),
        },
        NvimString {
            data: RUNTIME_DIRNAME.as_ptr().cast_mut(),
            size: 7,
        },
        true,
    );
    os_isdir(&p_cstr).then_some(p_cstr)
}

fn remove_tail(path_slice: &[u8], pend_off: usize, dirname: &[u8]) -> usize {
    let path = path_slice.as_ptr();
    let pend = path.wrapping_add(pend_off);
    let len = dirname.len();
    if pend_off > len {
        let new_tail = pend.wrapping_sub(len + 1);
        let dirname_cstr = cstr_from_bytes(dirname);
        if path_cmp(
            opt_vars().p_fic,
            crate::memory::cstr_from_ptr(new_tail.cast()),
            &dirname_cstr,
            len,
        ) == 0
            && (new_tail == path
                || after_pathsep(crate::memory::ptr_span_bytes(path.cast(), new_tail.cast())) != 0)
        {
            return pend_off - (len + 1);
        }
    }
    pend_off
}

/// Iterates `$PATH`-like delimited list `val`.
pub fn vim_env_iter<'a>(
    delim: u8,
    val: &'a [u8],
    iter: Option<&'a [u8]>,
    dir: &mut &'a [u8],
) -> Option<&'a [u8]> {
    let rem = iter.unwrap_or(val);
    if let Some(pos) = rem.iter().position(|&b| b == delim) {
        *dir = &rem[..pos];
        Some(&rem[pos + 1..])
    } else {
        *dir = rem;
        None
    }
}

/// Iterates `$PATH`-like delimited list `val`.
pub fn vim_env_iter_rev<'a>(
    delim: u8,
    val: &'a [u8],
    iter: Option<&'a [u8]>,
    dir: &mut &'a [u8],
) -> Option<&'a [u8]> {
    let rem = iter.unwrap_or(val);
    if let Some(pos) = rem.iter().rposition(|&b| b == delim) {
        *dir = &rem[pos + 1..];
        Some(&rem[..pos])
    } else {
        *dir = rem;
        None
    }
}

pub fn vim_get_prefix_from_exepath(exe_name: &mut [u8]) {
    crate::memory::copy_bytes_to_c_buf(
        exe_name,
        crate::memory::cstr_as_bytes((get_vim_var_str(VV_PROGPATH)).as_ptr()),
    );
    let ptr = exe_name.as_mut_ptr();
    let path_end: *mut u8 = path_tail_with_sep(crate::memory::cstr_from_ptr(ptr.cast()))
        .as_ptr()
        .cast_mut()
        .cast();
    unsafe {
        *path_end = 0;
    }
    let path_end2: *mut u8 = path_tail(crate::memory::cstr_from_ptr(ptr.cast()))
        .as_ptr()
        .cast_mut()
        .cast();
    unsafe {
        *path_end2 = 0;
    }
}

/// Vim `getenv()` wrapper with special handling of `$HOME`, `$VIM`, `$VIMRUNTIME`.
pub fn vim_getenv(name: &CStr) -> Option<std::ffi::CString> {
    let name_bytes = name.to_bytes();
    {
        assert!(!crate::memory::cstr_is_empty(
            (get_vim_var_str(VV_PROGPATH)).as_ptr()
        ));

        #[cfg(windows)]
        if name_bytes == b"HOME" {
            return env_st().homedir.clone();
        }

        let kos_env_path = os_getenv_cstr(name);
        #[cfg(windows)]
        if let Some(ref c) = kos_env_path
            && (name_bytes.eq_ignore_ascii_case(b"VIMRUNTIME")
                || name_bytes.eq_ignore_ascii_case(b"PATH")
                || name_bytes.eq_ignore_ascii_case(b"CDPATH")
                || name_bytes.eq_ignore_ascii_case(b"TMPDIR")
                || name_bytes.eq_ignore_ascii_case(b"TMP")
                || name_bytes.eq_ignore_ascii_case(b"TEMP")
                || name_bytes.eq_ignore_ascii_case(b"VIM")
                || name_bytes.eq_ignore_ascii_case(b"MYVIMRC"))
        {
            to_slash(crate::memory::cstr_as_mut_bytes(
                c.as_ptr().cast_mut().cast(),
            ));
        }
        if kos_env_path.is_some() {
            return kos_env_path;
        }

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
                vim_path_cstr = vim_runtime_dir(Some(kos_cstr.as_c_str())).or(Some(kos_cstr));
            }

            if vim_path_cstr.is_none() {
                let mut vim_path: *mut u8 = ptr::null_mut();
                if !opt_vars().p_hf.is_null()
                    && !crate::strings::vim_haschr(opt_vars().p_hf.as_cstr(), b'$' as i32)
                {
                    vim_path = opt_vars().p_hf.as_ptr().cast_mut().cast();
                }

                let mut exe_name = [0u8; MAXPATHL];
                if vim_path.is_null() {
                    vim_get_prefix_from_exepath(&mut exe_name);
                    if append_path(&mut exe_name, c"share/nvim/runtime/") {
                        vim_path = exe_name.as_mut_ptr();
                    }
                }

                if !vim_path.is_null() {
                    let vim_path_end: *mut u8 =
                        path_tail(crate::memory::cstr_from_ptr(vim_path.cast()))
                            .as_ptr()
                            .cast_mut()
                            .cast();
                    let mut end_off = (vim_path_end as usize) - (vim_path as usize);
                    let vim_path_slice = crate::memory::cstr_slice(vim_path.cast(), end_off);
                    if vim_path == opt_vars().p_hf.as_ptr().cast_mut().cast() {
                        end_off = remove_tail(vim_path_slice, end_off, b"doc");
                    }
                    if !vimruntime {
                        end_off = remove_tail(vim_path_slice, end_off, RUNTIME_DIRNAME.to_bytes());
                    }
                    if end_off > 0
                        && after_pathsep(crate::memory::ptr_span_bytes(
                            vim_path.cast(),
                            vim_path.wrapping_add(end_off).cast(),
                        )) != 0
                    {
                        end_off -= 1;
                    }
                    let cand =
                        std::ffi::CString::new(crate::memory::cstr_slice(vim_path.cast(), end_off))
                            .unwrap_or_default();
                    if os_isdir(&cand) {
                        vim_path_cstr = Some(cand);
                    }
                }
            }

            if vim_path_cstr.is_none() {
                if vimruntime && !default_vimruntime_dir().to_bytes().is_empty() {
                    vim_path_cstr = Some(default_vimruntime_dir().to_owned());
                } else if !default_vim_dir().to_bytes().is_empty() {
                    if vimruntime {
                        vim_path_cstr = vim_runtime_dir(Some(default_vim_dir()));
                    }
                    if vim_path_cstr.is_none() {
                        vim_path_cstr = Some(default_vim_dir().to_owned());
                    }
                }
            }

            if let Some(vim_path_cstr) = vim_path_cstr {
                if vimruntime {
                    let _ = os_setenv(c"VIMRUNTIME", vim_path_cstr.as_c_str(), 1);
                    g_st().didset_vimruntime = true;
                } else {
                    let _ = os_setenv(c"VIM", vim_path_cstr.as_c_str(), 1);
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
pub fn home_replace(
    buf: Option<&FileBuf>,
    src_in: Option<&CStr>,
    dst_slice: &mut [u8],
    one: bool,
) -> usize {
    let mut dstlen = dst_slice.len();
    let Some(src_cstr) = src_in else {
        if !dst_slice.is_empty() {
            dst_slice[0] = 0;
        }
        return 0;
    };
    let mut src: *const u8 = src_cstr.as_ptr().cast();

    if buf.is_some() && buf.unwrap().b_help {
        let dlen = crate::memory::copy_bytes_to_c_buf(
            dst_slice,
            crate::memory::cstr_as_bytes(
                (path_tail(crate::memory::cstr_from_ptr(src.cast()))).as_ptr(),
            ),
        );
        return dlen.min(dstlen - 1);
    }

    let homedir_ptr: *mut u8 = env_st()
        .homedir
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
        to_slash(crate::memory::cstr_as_mut_bytes(
            c.as_ptr().cast_mut().cast(),
        ));
    }
    let homedir_env = homedir_env_cstr
        .as_ref()
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let mut homedir_env_mod = homedir_env.cast::<u8>();
    let mut homedir_env_mod_vec: Vec<u8>;

    if !homedir_env_mod.is_null() && crate::memory::cstr_first_byte(homedir_env_mod.cast()) == b'~'
    {
        let mut usedlen: usize = 0;
        let mut fbuf = crate::memory::cstr_to_cstring(homedir_env_mod.cast()).into_bytes_with_nul();
        let mut fname_off = 0usize;
        let mut flen = fbuf.len() - 1;
        let _ = modify_fname(
            b":p",
            false,
            &mut usedlen,
            &mut fbuf,
            &mut fname_off,
            &mut flen,
            false,
        );
        if !fbuf.is_empty() {
            if fname_off > 0 {
                fbuf.drain(..fname_off);
            }
            flen = crate::memory::cstr_len(fbuf.as_ptr().cast());
            if flen > 0 && vim_ispathsep(i32::from(fbuf[flen - 1])) {
                fbuf[flen - 1] = 0;
            }
            homedir_env_mod_vec = fbuf;
            homedir_env_mod = homedir_env_mod_vec.as_mut_ptr();
        } else {
            homedir_env_mod = ptr::null_mut();
        }
    }

    let envlen = if homedir_env_mod.is_null() {
        0
    } else {
        crate::memory::cstr_len(homedir_env_mod.cast())
    };

    if !one {
        src = {
            let _sw_p = src.cast::<std::ffi::c_char>();
            _sw_p.wrapping_add(crate::charset::skipwhite_len(crate::memory::cstr_as_bytes(
                _sw_p.cast(),
            )))
        }
        .cast();
    }
    let src_bytes = crate::memory::cstr_as_bytes(src.cast());
    let mut src_idx = 0usize;
    let mut dst_idx = 0usize;
    while src_idx < src_bytes.len() && dstlen > 0 {
        let mut p = homedir_ptr;
        let mut len = dirlen;
        loop {
            let cmp_ok = len != 0
                && path_cmp(
                    opt_vars().p_fic,
                    crate::memory::cstr_from_ptr(src_bytes[src_idx..].as_ptr().cast()),
                    crate::memory::cstr_from_ptr(p.cast()),
                    len,
                ) == 0;

            let next_b = src_bytes.get(src_idx + len).copied().unwrap_or(0);
            if cmp_ok
                && (vim_ispathsep(i32::from(next_b))
                    || (!one && (next_b == b',' || next_b == b' '))
                    || next_b == 0)
            {
                src_idx += len;
                dstlen -= 1;
                if dstlen > 0 {
                    dst_slice[dst_idx] = b'~';
                    dst_idx += 1;
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
        while src_idx < src_bytes.len()
            && (one || (src_bytes[src_idx] != b',' && src_bytes[src_idx] != b' '))
        {
            dstlen -= 1;
            if dstlen == 0 {
                break;
            }
            dst_slice[dst_idx] = src_bytes[src_idx];
            dst_idx += 1;
            src_idx += 1;
        }
        if dstlen == 0 {
            break;
        }
        while src_idx < src_bytes.len()
            && (src_bytes[src_idx] == b' ' || src_bytes[src_idx] == b',')
        {
            dstlen -= 1;
            if dstlen == 0 {
                break;
            }
            dst_slice[dst_idx] = src_bytes[src_idx];
            dst_idx += 1;
            src_idx += 1;
        }
    }

    if !dst_slice.is_empty() {
        dst_slice[dst_idx] = 0;
    }
    dst_idx
}

/// Like `home_replace`, store the replaced string in an owned `CString`.
pub fn home_replace_save(buf: Option<&FileBuf>, src: Option<&CStr>) -> std::ffi::CString {
    let mut len: usize = 3;
    if let Some(s) = src {
        len += s.to_bytes().len();
    }
    let mut dst = vec![0u8; len];
    let _ = home_replace(buf, src, &mut dst, true);
    cstr_to_cstring(dst.as_ptr().cast())
}

/// Function given to `ExpandGeneric()` to obtain an environment variable name.
pub fn get_env_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    assert!(idx >= 0);
    if let Some(envname) = os_getenvname_at_index(idx as usize) {
        crate::memory::copy_bytes_to_c_buf(&mut xp.xp_buf, envname.to_bytes());
        return Some(unsafe { CStr::from_ptr(xp.xp_buf.as_ptr().cast()) });
    }
    None
}

/// Appends the head of `fname` to `$PATH` and sets it in the environment.
#[cfg_attr(not(windows), cfg(test))]
pub fn os_setenv_append_path(fname_cstr: &CStr) -> bool {
    let fname = fname_cstr.as_ptr();
    {
        if !path_is_absolute(crate::memory::cstr_as_bytes((fname).cast())) {
            internal_error(c"os_setenv_append_path()");
            return false;
        }
        let tail = path_tail_with_sep(fname_cstr).as_ptr();
        let dirlen = (tail as usize) - (fname as usize);
        assert!((tail as usize) >= (fname as usize) && dirlen + 1 < MAXPATHL);
        for (i, &b) in crate::memory::cstr_slice(fname, dirlen).iter().enumerate() {
            g_st().os_buf[i] = b as crate::strings::RawCChar;
        }
        g_st().os_buf[dirlen] = 0;
        let path_cstr = os_getenv_cstr(c"PATH");
        let pathlen = path_cstr.as_ref().map_or(0, |c| c.as_bytes().len());
        let newlen = pathlen + dirlen + 2;
        let mut retval = false;
        if newlen < MAX_ENVPATHLEN {
            let mut temp_vec = vec![0u8; newlen];
            if let Some(ref path) = path_cstr {
                crate::memory::copy_bytes_to_c_buf(&mut temp_vec, path.as_bytes());
                if path.as_bytes().last().copied() != Some(ENV_SEPCHAR) {
                    crate::memory::append_bytes_to_c_buf(&mut temp_vec, ENV_SEPSTR.to_bytes());
                }
            }
            let dir_slice = crate::memory::cstr_slice(fname, dirlen);
            crate::memory::append_bytes_to_c_buf(&mut temp_vec, dir_slice);
            let _ = os_setenv(c"PATH", crate::memory::cstr_until_nul(&temp_vec), 1);
            retval = true;
        }
        retval
    }
}

/// Returns true if `sh` looks like it resolves to `"cmd.exe"`.
#[cfg_attr(not(windows), cfg(test))]
pub fn os_shell_is_cmdexe(sh: &CStr) -> bool {
    let sh_bytes = sh.to_bytes();
    if sh_bytes.is_empty() {
        return false;
    }
    if sh_bytes.eq_ignore_ascii_case(b"$COMSPEC") {
        let mut buf = [0u8; MAXPATHL];
        let Some(comspec) = os_getenv_buf(c"COMSPEC", &mut buf) else {
            return false;
        };
        return crate::memory::cstr_as_bytes((path_tail(comspec)).as_ptr())
            .eq_ignore_ascii_case(b"cmd.exe");
    }
    if sh_bytes.eq_ignore_ascii_case(b"cmd.exe") || sh_bytes.eq_ignore_ascii_case(b"cmd") {
        return true;
    }
    crate::memory::cstr_as_bytes((path_tail(sh)).as_ptr()).eq_ignore_ascii_case(b"cmd.exe")
}

/// Removes environment variable `var` and takes care of side effects.
pub fn vim_unsetenv_ext(var: &CStr) {
    let _ = os_unsetenv(var);
    let var_bytes = var.to_bytes();
    if var_bytes.eq_ignore_ascii_case(b"VIM") {
        g_st().didset_vim = false;
    } else if var_bytes.eq_ignore_ascii_case(b"VIMRUNTIME") {
        g_st().didset_vimruntime = false;
    }
}

/// Set environment variable `name` and take care of side effects.
pub fn vim_setenv_ext(name: &CStr, val: &CStr) {
    let _ = os_setenv(name, val, 1);
    let name_bytes = name.to_bytes();
    if name_bytes.eq_ignore_ascii_case(b"HOME") {
        init_homedir();
    } else if g_st().didset_vim && name_bytes.eq_ignore_ascii_case(b"VIM") {
        g_st().didset_vim = false;
    } else if g_st().didset_vimruntime && name_bytes.eq_ignore_ascii_case(b"VIMRUNTIME") {
        g_st().didset_vimruntime = false;
    }
}

/// Restore a previous environment variable value, or unset it if `None`.
#[cfg(windows)]
pub fn restore_env_var(name: &std::ffi::CStr, old_value: Option<std::ffi::CString>) {
    if let Some(old_value) = old_value {
        let _ = os_setenv(name, old_value.as_c_str(), 1);
        return;
    }
    let _ = os_unsetenv(name);
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
            assert_eq!(env_name_len(b"FOO_123/bar"), 7);
            assert_eq!(env_name_len(b"-invalid"), 0);

            let val = b"/a:/bb:/ccc";
            let mut iter: Option<&[u8]> = None;
            let mut dir: &[u8] = &[];

            iter = vim_env_iter(b':', val, iter, &mut dir);
            assert_eq!(dir.len(), 2);
            assert!(iter.is_some());

            iter = vim_env_iter(b':', val, iter, &mut dir);
            assert_eq!(dir.len(), 3);
            assert!(iter.is_some());

            iter = vim_env_iter(b':', val, iter, &mut dir);
            assert_eq!(dir.len(), 4);
            assert!(iter.is_none());

            if crate::eval::vars::get_vim_var_str(VV_PROGPATH)
                .to_bytes()
                .is_empty()
            {
                crate::eval::vars::set_vim_var_string(VV_PROGPATH, Some(c"nvim"), -1);
            }

            assert!(!os_env_exists(c"", false));
            assert!(!os_env_exists(c"   ", true));
            let k = c"NVIM_RUST_ENV_TEST_KEY";
            let _ = os_unsetenv(k);
            assert!(!os_env_exists(k, false));
            assert_eq!(os_setenv(k, c"val1", 1), 0);
            assert!(os_env_exists(k, false));
            assert!(os_env_exists(k, true));

            let got = os_getenv_cstr(k).unwrap();
            assert_eq!(got.to_str().unwrap(), "val1");

            let mut buf = [0u8; 64];
            assert_eq!(os_setenv(k, c"val2", 0), 0);
            assert_eq!(
                os_getenv_buf(k, &mut buf).unwrap().to_str().unwrap(),
                "val1"
            );
            assert_eq!(os_setenv(k, c"val2", 1), 0);
            assert_eq!(
                os_getenv_buf(k, &mut buf).unwrap().to_str().unwrap(),
                "val2"
            );

            assert!(os_getenv_buf(k, &mut buf).is_some());
            assert_eq!(
                crate::memory::cstr_from_ptr(buf.as_ptr().cast())
                    .to_str()
                    .unwrap(),
                "val2"
            );

            expand_env_esc(c"$NVIM_RUST_ENV_TEST_KEY/sub", &mut buf, None, true, None);
            assert_eq!(
                crate::memory::cstr_from_ptr(buf.as_ptr().cast())
                    .to_str()
                    .unwrap(),
                "val2/sub"
            );
            let _ = os_unsetenv(k);

            assert!(os_get_pid() > 0);
            os_get_hostname(&mut buf);
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
            assert!(os_setenv_append_path(abs_path));
        }
    }
}
