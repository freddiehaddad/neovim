//! Environment variable inspection and path expansion (`src/nvim/os/env.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::FileBuf;
#[cfg(unix)]
use crate::charset::vim_isfilec;
pub use crate::cmdexpand::ExpandCtx;
use crate::eval::fs::modify_fname;
use crate::eval::skip_expr;
use crate::eval::vars::{VV_PROGPATH, get_vim_var_str};
use crate::globals::*;
use crate::mbyte::utf_isalnum;
#[cfg(windows)]
use crate::mbyte::{utf_valid_string, utf16_to_utf8};
use crate::memory::bytes_to_cstring;
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

std::thread_local! {
    static HOMEDIR: std::cell::RefCell<Option<std::ffi::CString>> =
        const { std::cell::RefCell::new(None) };
}

use crate::version::{default_vim_dir, default_vimruntime_dir};

use crate::event::r#loop::{
    uv_env_item_t, uv_os_environ, uv_os_free_environ, uv_os_getenv, uv_os_homedir, uv_os_setenv,
    uv_os_unsetenv,
};

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
    g_st().nvim_testing.set(os_env_exists(c"NVIM_TEST", false));
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
            Some(bytes_to_cstring(&vec_buf[..size]))
        }
    } else if r != 0 || size == 0 || buf[0] == 0 {
        None
    } else {
        Some(bytes_to_cstring(&buf[..size]))
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
    Some(crate::memory::bytes_as_cstr(buf))
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
        let env_arr = (*os_environ_ptr()).cast_const();

        for i in 0..=index {
            if (*env_arr.add(i)).is_null() {
                return None;
            }
        }
        let str_ptr = *env_arr.add(index);
        assert!(!str_ptr.is_null());
        let bytes = CStr::from_ptr(str_ptr.cast()).to_bytes();
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
pub fn os_homedir() -> Option<std::ffi::CString> {
    let hd = HOMEDIR.with_borrow(|h| h.clone());
    if hd.is_none() {
        emsg(c"os_homedir failed: homedir not initialized");
    }
    hd
}

fn os_uv_homedir() -> Option<std::ffi::CString> {
    unsafe {
        let mut homedir_buf = [0u8; MAXPATHL];
        let mut homedir_size = MAXPATHL;
        let ret_value = uv_os_homedir(homedir_buf.as_mut_ptr().cast(), &mut homedir_size);
        if ret_value == 0 && homedir_size < MAXPATHL {
            return Some(crate::memory::bytes_as_cstr(&homedir_buf).to_owned());
        }
        None
    }
}

/// Sets `homedir` to the "real", resolved user home directory.
pub fn init_homedir() {
    HOMEDIR.with_borrow_mut(|h| *h = None);

    let mut var = os_getenv_cstr(c"HOME");

    #[cfg(windows)]
    {
        if var.is_none() {
            let homedrive = os_getenv_cstr(c"HOMEDRIVE");
            let homepath = os_getenv_cstr(c"HOMEPATH");
            let homepath_bytes = homepath
                .as_ref()
                .map_or(PATHSEPSTR.to_bytes(), |c| c.as_bytes());
            if let Some(ref hd) = homedrive
                && hd.as_bytes().len() + homepath_bytes.len() < MAXPATHL
            {
                crate::memory::copy_bytes_to_c_buf(&mut *g_st().os_buf_bytes_mut(), hd.as_bytes());
                crate::memory::append_bytes_to_c_buf(
                    &mut *g_st().os_buf_bytes_mut(),
                    homepath_bytes,
                );
                if g_st().os_buf.borrow()[0] != 0 {
                    var = Some(crate::memory::cstring_until_nul(
                        &*g_st().os_buf_bytes_mut(),
                    ));
                }
            }
        }
        if var.is_none() {
            var = os_uv_homedir();
        }

        if let Some(ref cur_var) = var
            && cur_var.as_bytes().first() == Some(&b'%')
        {
            let rest = &cur_var.as_bytes()[1..];
            if let Some(pos) = rest.iter().position(|&b| b == b'%') {
                let tail_bytes = rest[pos + 1..].to_vec();
                let env_name = bytes_to_cstring(&rest[..pos]);
                var = None;
                if let Some(exp) = os_getenv_cstr(&env_name)
                    && !exp.as_bytes().is_empty()
                    && exp.as_bytes().len() + 1 + tail_bytes.len() < MAXPATHL
                {
                    crate::memory::copy_bytes_to_c_buf(
                        &mut *g_st().os_buf_bytes_mut(),
                        exp.as_bytes(),
                    );
                    crate::memory::append_bytes_to_c_buf(
                        &mut *g_st().os_buf_bytes_mut(),
                        &tail_bytes,
                    );
                    var = Some(crate::memory::cstring_until_nul(
                        &*g_st().os_buf_bytes_mut(),
                    ));
                }
            }
        }

        if var.as_ref().is_none_or(|s| s.as_bytes().is_empty()) {
            var = Some(c"C:/".to_owned());
        }
    }

    #[cfg(not(windows))]
    {
        if var.is_none() {
            var = os_uv_homedir();
        }

        if let Some(ref cur_var) = var
            && os_realpath(cur_var, (&mut *g_st().iobuff_bytes_mut())).is_some()
        {
            var = Some(g_st().iobuff_cstr().to_owned());
        }

        if var.as_ref().is_none_or(|s| s.as_bytes().is_empty())
            && os_dirname((&mut *g_st().os_buf_bytes_mut()))
        {
            var = Some(crate::memory::cstring_until_nul(
                (&mut *g_st().os_buf_bytes_mut()),
            ));
        }
    }

    if let Some(ref v) = var {
        let saved = to_slash_save(Some(v));
        HOMEDIR.with_borrow_mut(|h| *h = saved);
    }
}

pub fn free_homedir() {
    HOMEDIR.with_borrow_mut(|h| *h = None);
}

/// Call `expand_env()` and store the result in an allocated string.
pub fn expand_env_save(src: &CStr) -> std::ffi::CString {
    expand_env_save_opt(src, false, None)
}

/// Similar to `expand_env_save()` with `one` and `esc_chars`.
pub fn expand_env_save_opt(src: &CStr, one: bool, esc_chars: Option<&CStr>) -> std::ffi::CString {
    let mut p = [0u8; MAXPATHL];
    let _ = expand_env_esc(src, &mut p, esc_chars, one, None);
    crate::memory::cstring_until_nul(&p)
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
    let mut dst_idx = 0usize;
    let mut dst_rem = dst_slice.len().saturating_sub(1) as i32;
    let mut at_start = true;
    let prefix_bytes = prefix.map_or(&b""[..], CStr::to_bytes);

    let mut src_cstr = crate::charset::skipwhite(srcp_in);
    let srcp_base = srcp_in.as_ptr() as usize;
    while !src_cstr.to_bytes().is_empty() && dst_rem > 0 {
        let src_bytes = src_cstr.to_bytes();
        if src_bytes.starts_with(b"`=") {
            let start_ptr = src_cstr.as_ptr() as usize;
            let mut expr_cstr = crate::memory::CStrSubslice::advance(src_cstr, 2);
            let _ = skip_expr(&mut expr_cstr, None);
            if expr_cstr.to_bytes().first() == Some(&b'`') {
                expr_cstr = crate::memory::CStrSubslice::advance(expr_cstr, 1);
            }
            let adv = (expr_cstr.as_ptr() as usize) - start_ptr;
            let len = adv.min(dst_rem as usize);
            dst_slice[dst_idx..dst_idx + len].copy_from_slice(&src_bytes[..len]);
            dst_idx += len;
            dst_rem -= len as i32;
            src_cstr = expr_cstr;
            continue;
        }

        let mut copy_char = true;
        let first_b = src_bytes[0];
        if first_b == b'$' || (first_b == b'~' && at_start) {
            let mut var_cstr: Option<std::ffi::CString>;
            let mut tail_off: usize;

            if first_b != b'~' {
                tail_off = 1;
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
                    tail_off += 1;
                    let mut ti = 1usize;
                    while c > 0 && ti < tail_bytes.len() && tail_bytes[ti] != b'}' {
                        c -= 1;
                        dst_slice[dst_idx + var_len] = tail_bytes[ti];
                        var_len += 1;
                        ti += 1;
                        tail_off += 1;
                    }
                } else {
                    let len = env_name_len(tail_bytes).min(c.max(0) as usize);
                    dst_slice[dst_idx..dst_idx + len].copy_from_slice(&tail_bytes[..len]);
                    var_len = len;
                    tail_off += len;
                }

                #[cfg(windows)]
                {
                    let len = env_name_len(tail_bytes).min(c.max(0) as usize);
                    dst_slice[dst_idx..dst_idx + len].copy_from_slice(&tail_bytes[..len]);
                    var_len = len;
                    tail_off += len;
                }

                #[cfg(not(windows))]
                if src_bytes.get(1) == Some(&b'{') && src_bytes.get(tail_off) != Some(&b'}') {
                    var_cstr = None;
                } else {
                    if src_bytes.get(1) == Some(&b'{') {
                        tail_off += 1;
                    }
                    dst_slice[dst_idx + var_len] = 0;
                    var_cstr = vim_getenv(crate::memory::bytes_as_cstr(&dst_slice[dst_idx..]));
                    if let Some(ref mut c) = var_cstr {
                        crate::memory::cstring_modify(c, to_slash);
                    }
                }

                #[cfg(windows)]
                {
                    dst_slice[dst_idx + var_len] = 0;
                    var_cstr = vim_getenv(crate::memory::bytes_as_cstr(&dst_slice[dst_idx..]));
                    if let Some(ref mut c) = var_cstr {
                        crate::memory::cstring_modify(c, to_slash);
                    }
                }
            } else if src_bytes.len() == 1
                || vim_ispathsep(i32::from(src_bytes[1]))
                || matches!(src_bytes[1], b' ' | b',' | b'\t' | b'\n')
            {
                var_cstr = HOMEDIR.with_borrow(|h| h.clone());
                tail_off = 1;
            } else {
                #[cfg(not(windows))]
                {
                    tail_off = 0;
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
                        tail_off += 1;
                    }
                    dst_slice[dst_idx + var_len] = 0;
                    if dst_slice[dst_idx] == 0 {
                        var_cstr = None;
                    } else {
                        var_cstr = os_get_userdir(Some(crate::memory::bytes_as_cstr(
                            &dst_slice[dst_idx + 1..],
                        )));
                    }
                }
                #[cfg(windows)]
                {
                    var_cstr = None;
                    tail_off = src_bytes.len();
                }
            }

            if let (Some(esc), Some(v)) = (esc_chars, var_cstr.as_ref()) {
                let var_bytes = v.as_bytes();
                let esc_bytes = esc.to_bytes();
                if var_bytes.iter().any(|b| esc_bytes.contains(b)) {
                    var_cstr = Some(vim_strsave_escaped(var_bytes, esc_bytes));
                }
            }

            if let Some(ref v) = var_cstr
                && !v.as_bytes().is_empty()
            {
                let var_bytes_with_nul = v.as_bytes_with_nul();
                let c = var_bytes_with_nul.len() - 1;
                let tail_bytes = &src_bytes[tail_off.min(src_bytes.len())..];
                if c + tail_bytes.len() + 1 < (dst_rem as usize) {
                    dst_slice[dst_idx..dst_idx + c + 1].copy_from_slice(var_bytes_with_nul);
                    dst_rem -= c as i32;
                    let ends_with_sep = after_pathsep(&dst_slice[dst_idx..dst_idx + c]) != 0;
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
                        tail_off += 1;
                    }
                    dst_idx += c;
                    src_cstr = crate::memory::CStrSubslice::advance(
                        src_cstr,
                        tail_off.min(src_bytes.len()),
                    );
                    copy_char = false;
                }
            }
        }

        if copy_char {
            at_start = false;
            if first_b == b'\\' && src_bytes.len() > 1 {
                dst_slice[dst_idx] = b'\\';
                dst_idx += 1;
                src_cstr = crate::memory::CStrSubslice::advance(src_cstr, 1);
                dst_rem -= 1;
            } else if matches!(first_b, b' ' | b',') && !one {
                at_start = true;
            }
            if dst_rem > 0 {
                dst_slice[dst_idx] = src_cstr.to_bytes()[0];
                dst_idx += 1;
                src_cstr = crate::memory::CStrSubslice::advance(src_cstr, 1);
                dst_rem -= 1;

                let cur_off = (src_cstr.as_ptr() as usize) - srcp_base;
                if !prefix_bytes.is_empty()
                    && cur_off >= prefix_bytes.len()
                    && &srcp_in.to_bytes()[cur_off - prefix_bytes.len()..cur_off] == prefix_bytes
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

/// Iterate over current process environment items via `uv_os_environ`.
pub fn os_get_environ(mut cb: impl FnMut(&CStr, Option<&CStr>)) {
    let mut envitems: *mut uv_env_item_t = std::ptr::null_mut();
    let mut envcount: i32 = 0;
    if unsafe { uv_os_environ(&raw mut envitems, &mut envcount) } == 0 {
        for i in 0..(envcount as usize) {
            let item = unsafe { &*envitems.wrapping_add(i) };
            let item_name = if item.name.is_null() {
                c""
            } else {
                unsafe { CStr::from_ptr(item.name.cast()) }
            };
            let item_val =
                (!item.value.is_null()).then(|| unsafe { CStr::from_ptr(item.value.cast()) });
            cb(item_name, item_val);
        }
        unsafe { uv_os_free_environ(envitems, envcount) };
    }
}

fn vim_runtime_dir(vimdir: Option<&CStr>) -> Option<std::ffi::CString> {
    let vimdir = vimdir?;
    if vimdir.to_bytes().is_empty() {
        return None;
    }
    let p_cstr = concat_fnames(
        NvimString::from_cstr(vimdir),
        NvimString::from_cstr(RUNTIME_DIRNAME),
        true,
    );
    os_isdir(&p_cstr).then_some(p_cstr)
}

fn remove_tail(path_slice: &[u8], pend_off: usize, dirname: &[u8]) -> usize {
    let len = dirname.len();
    if pend_off > len {
        let new_tail_off = pend_off - len - 1;
        let new_tail_cstr = bytes_to_cstring(&path_slice[new_tail_off..pend_off]);
        let dirname_cstr = bytes_to_cstring(dirname);
        if path_cmp(opt_vars().p_fic.get(), &new_tail_cstr, &dirname_cstr, len) == 0
            && (new_tail_off == 0 || after_pathsep(&path_slice[..new_tail_off]) != 0)
        {
            return new_tail_off;
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
    crate::memory::copy_bytes_to_c_buf(exe_name, get_vim_var_str(VV_PROGPATH).to_bytes());
    let base = exe_name.as_ptr() as usize;
    let end1 =
        (path_tail_with_sep(crate::memory::bytes_as_cstr(exe_name)).as_ptr() as usize) - base;
    exe_name[end1] = 0;
    let end2 = (path_tail(crate::memory::bytes_as_cstr(exe_name)).as_ptr() as usize) - base;
    exe_name[end2] = 0;
}

/// Vim `getenv()` wrapper with special handling of `$HOME`, `$VIM`, `$VIMRUNTIME`.
pub fn vim_getenv(name: &CStr) -> Option<std::ffi::CString> {
    let name_bytes = name.to_bytes();
    {
        assert!(!get_vim_var_str(VV_PROGPATH).is_empty());

        #[cfg(windows)]
        if name_bytes == b"HOME" {
            return HOMEDIR.with_borrow(|h| h.clone());
        }

        #[cfg(windows)]
        let mut kos_env_path = os_getenv_cstr(name);
        #[cfg(not(windows))]
        let kos_env_path = os_getenv_cstr(name);
        #[cfg(windows)]
        if let Some(ref mut c) = kos_env_path
            && (name_bytes.eq_ignore_ascii_case(b"VIMRUNTIME")
                || name_bytes.eq_ignore_ascii_case(b"PATH")
                || name_bytes.eq_ignore_ascii_case(b"CDPATH")
                || name_bytes.eq_ignore_ascii_case(b"TMPDIR")
                || name_bytes.eq_ignore_ascii_case(b"TMP")
                || name_bytes.eq_ignore_ascii_case(b"TEMP")
                || name_bytes.eq_ignore_ascii_case(b"VIM")
                || name_bytes.eq_ignore_ascii_case(b"MYVIMRC"))
        {
            crate::memory::cstring_modify(c, to_slash);
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
                let mut is_helpfile = false;
                let mut vim_path_opt: Option<std::ffi::CString> = None;
                if !opt_vars().p_hf.is_null()
                    && !crate::strings::vim_haschr(opt_vars().p_hf.as_cstr(), b'$' as i32)
                {
                    vim_path_opt = Some(opt_vars().p_hf.to_owned());
                    is_helpfile = true;
                }

                let mut exe_name = [0u8; MAXPATHL];
                if vim_path_opt.is_none() {
                    vim_get_prefix_from_exepath(&mut exe_name);
                    if append_path(&mut exe_name, c"share/nvim/runtime/") {
                        vim_path_opt = Some(crate::memory::cstring_until_nul(&exe_name));
                    }
                }

                if let Some(ref vim_path) = vim_path_opt {
                    let vim_path_end = path_tail(vim_path).as_ptr() as usize;
                    let mut end_off = vim_path_end - (vim_path.as_ptr() as usize);
                    let vim_path_slice = &vim_path.as_bytes()[..end_off];
                    if is_helpfile {
                        end_off = remove_tail(vim_path_slice, end_off, b"doc");
                    }
                    if !vimruntime {
                        end_off = remove_tail(vim_path_slice, end_off, RUNTIME_DIRNAME.to_bytes());
                    }
                    if end_off > 0 && after_pathsep(&vim_path.as_bytes()[..end_off]) != 0 {
                        end_off -= 1;
                    }
                    let cand =
                        std::ffi::CString::new(&vim_path.as_bytes()[..end_off]).unwrap_or_default();
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
                    g_st().didset_vimruntime.set(true);
                } else {
                    let _ = os_setenv(c"VIM", vim_path_cstr.as_c_str(), 1);
                    g_st().didset_vim.set(true);
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
    let Some(mut src_cstr) = src_in else {
        if !dst_slice.is_empty() {
            dst_slice[0] = 0;
        }
        return 0;
    };

    if buf.is_some() && buf.unwrap().b_help.get() {
        let dlen = crate::memory::copy_bytes_to_c_buf(dst_slice, path_tail(src_cstr).to_bytes());
        return dlen.min(dstlen - 1);
    }

    let homedir_owned = HOMEDIR.with_borrow(|h| h.clone());
    let homedir_opt = homedir_owned.as_deref();
    let dirlen = homedir_opt.map_or(0, |s| s.to_bytes().len());

    let mut homedir_env_cstr = os_getenv_cstr(c"HOME");
    #[cfg(windows)]
    if homedir_env_cstr.is_none() {
        homedir_env_cstr = os_getenv_cstr(c"USERPROFILE");
    }
    if let Some(ref mut c) = homedir_env_cstr {
        crate::memory::cstring_modify(c, to_slash);
    }

    if let Some(ref env_c) = homedir_env_cstr
        && env_c.as_bytes().first() == Some(&b'~')
    {
        let mut usedlen: usize = 0;
        let mut fbuf = env_c.clone().into_bytes_with_nul();
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
            flen = crate::memory::bytes_until_nul(&fbuf).len();
            if flen > 0 && vim_ispathsep(i32::from(fbuf[flen - 1])) {
                fbuf[flen - 1] = 0;
            }
            homedir_env_cstr = Some(crate::memory::cstring_until_nul(&fbuf));
        } else {
            homedir_env_cstr = None;
        }
    }

    let homedir_env_opt = homedir_env_cstr.as_deref();
    let envlen = homedir_env_opt.map_or(0, |s| s.to_bytes().len());

    if !one {
        src_cstr = crate::charset::skipwhite(src_cstr);
    }
    let src_bytes = src_cstr.to_bytes();
    let mut src_idx = 0usize;
    let mut dst_idx = 0usize;
    while src_idx < src_bytes.len() && dstlen > 0 {
        for (p_opt, len) in [(homedir_opt, dirlen), (homedir_env_opt, envlen)] {
            let cmp_ok = len != 0
                && p_opt.is_some_and(|p| {
                    path_cmp(
                        opt_vars().p_fic.get(),
                        crate::memory::CStrSubslice::advance(src_cstr, src_idx),
                        p,
                        len,
                    ) == 0
                });

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
    crate::memory::cstring_until_nul(&dst)
}

/// Function given to `ExpandGeneric()` to obtain an environment variable name.
pub fn get_env_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    assert!(idx >= 0);
    if let Some(envname) = os_getenvname_at_index(idx as usize) {
        crate::memory::copy_bytes_to_c_buf(&mut xp.xp_buf, envname.to_bytes());
        return Some(crate::memory::bytes_as_cstr(&xp.xp_buf));
    }
    None
}

/// Appends the head of `fname` to `$PATH` and sets it in the environment.
#[cfg_attr(not(windows), cfg(test))]
pub fn os_setenv_append_path(fname_cstr: &CStr) -> bool {
    let fname_bytes = fname_cstr.to_bytes();
    if !path_is_absolute(fname_bytes) {
        internal_error(c"os_setenv_append_path()");
        return false;
    }
    let tail = path_tail_with_sep(fname_cstr).as_ptr();
    let dirlen = (tail as usize) - (fname_cstr.as_ptr() as usize);
    assert!((tail as usize) >= (fname_cstr.as_ptr() as usize) && dirlen + 1 < MAXPATHL);
    for (i, &b) in fname_bytes[..dirlen].iter().enumerate() {
        g_st().os_buf.borrow_mut()[i] = b;
    }
    g_st().os_buf.borrow_mut()[dirlen] = 0;
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
        let dir_slice = &fname_bytes[..dirlen];
        crate::memory::append_bytes_to_c_buf(&mut temp_vec, dir_slice);
        let _ = os_setenv(c"PATH", crate::memory::bytes_as_cstr(&temp_vec), 1);
        retval = true;
    }
    retval
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
        return path_tail(comspec)
            .to_bytes()
            .eq_ignore_ascii_case(b"cmd.exe");
    }
    if sh_bytes.eq_ignore_ascii_case(b"cmd.exe") || sh_bytes.eq_ignore_ascii_case(b"cmd") {
        return true;
    }
    path_tail(sh).to_bytes().eq_ignore_ascii_case(b"cmd.exe")
}

/// Removes environment variable `var` and takes care of side effects.
pub fn vim_unsetenv_ext(var: &CStr) {
    let _ = os_unsetenv(var);
    let var_bytes = var.to_bytes();
    if var_bytes.eq_ignore_ascii_case(b"VIM") {
        g_st().didset_vim.set(false);
    } else if var_bytes.eq_ignore_ascii_case(b"VIMRUNTIME") {
        g_st().didset_vimruntime.set(false);
    }
}

/// Set environment variable `name` and take care of side effects.
pub fn vim_setenv_ext(name: &CStr, val: &CStr) {
    let _ = os_setenv(name, val, 1);
    let name_bytes = name.to_bytes();
    if name_bytes.eq_ignore_ascii_case(b"HOME") {
        init_homedir();
    } else if g_st().didset_vim.get() && name_bytes.eq_ignore_ascii_case(b"VIM") {
        g_st().didset_vim.set(false);
    } else if g_st().didset_vimruntime.get() && name_bytes.eq_ignore_ascii_case(b"VIMRUNTIME") {
        g_st().didset_vimruntime.set(false);
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
            assert_eq!(crate::memory::bytes_as_cstr(&buf).to_str().unwrap(), "val2");

            expand_env_esc(c"$NVIM_RUST_ENV_TEST_KEY/sub", &mut buf, None, true, None);
            assert_eq!(
                crate::memory::bytes_as_cstr(&buf).to_str().unwrap(),
                "val2/sub"
            );
            let _ = os_unsetenv(k);

            assert!(os_get_pid() > 0);
            os_get_hostname(&mut buf);
            assert!(!crate::memory::bytes_until_nul(&buf).is_empty());
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
