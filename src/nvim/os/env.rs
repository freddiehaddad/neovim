//! Environment variable inspection and path expansion (`src/nvim/os/env.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::FileBuf;
use crate::charset::skipwhite;
#[cfg(unix)]
use crate::charset::vim_isfilec;
use crate::cursor::CursorPos;
use crate::eval::fs::modify_fname;
use crate::eval::skip_expr;
use crate::eval::vars::{VV_PROGPATH, get_vim_var_str};
use crate::globals::*;
use crate::lua::converter::LuaRef;
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
use crate::runtime::Sctx;
use crate::strings::vim_strsave_escaped;
use std::ffi::{CStr, c_char};
use std::ptr;

pub(crate) const EXPAND_BUF_LEN: usize = 256;
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
#[repr(C)]
pub struct ExpandCtx {
    pub xp_pattern: *mut c_char,
    pub xp_context: i32,
    pub xp_pattern_len: usize,
    pub xp_prefix: i32,
    pub xp_arg: *mut c_char,
    pub xp_luaref: LuaRef,
    pub xp_script_ctx: Sctx,
    pub xp_backslash: i32,
    #[cfg(not(windows))]
    pub xp_shell: bool,
    pub xp_numfiles: i32,
    pub xp_col: i32,
    pub xp_selected: i32,
    pub xp_orig: Option<std::ffi::CString>,
    pub xp_files: Vec<std::ffi::CString>,
    pub xp_files_abbr: Vec<Option<std::ffi::CString>>,
    pub xp_files_kind: Vec<Option<std::ffi::CString>>,
    pub xp_files_menu: Vec<Option<std::ffi::CString>>,
    pub xp_files_info: Vec<Option<std::ffi::CString>>,
    pub xp_line: *mut c_char,
    pub xp_buf: [c_char; EXPAND_BUF_LEN],
    pub xp_search_dir: i32,
    pub xp_pre_incsearch_pos: CursorPos,
}

impl ExpandCtx {
    pub const fn new() -> Self {
        Self {
            xp_pattern: ptr::null_mut(),
            xp_context: 0,
            xp_pattern_len: 0,
            xp_prefix: crate::cmdexpand::XP_PREFIX_NONE,
            xp_arg: ptr::null_mut(),
            xp_luaref: 0,
            xp_script_ctx: Sctx {
                sc_sid: 0,
                sc_seq: 0,
                sc_lnum: 0,
                sc_chan: 0,
            },
            xp_backslash: crate::cmdexpand::XP_BS_NONE,
            #[cfg(not(windows))]
            xp_shell: false,
            xp_numfiles: -1,
            xp_col: 0,
            xp_selected: 0,
            xp_orig: None,
            xp_files: Vec::new(),
            xp_files_abbr: Vec::new(),
            xp_files_kind: Vec::new(),
            xp_files_menu: Vec::new(),
            xp_files_info: Vec::new(),
            xp_line: ptr::null_mut(),
            xp_buf: [0; EXPAND_BUF_LEN],
            xp_search_dir: 0,
            xp_pre_incsearch_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
        }
    }
}

impl Default for ExpandCtx {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ExpandCtx {
    fn drop(&mut self) {
        crate::cmdexpand::expand_cleanup(self);
    }
}

struct EnvModuleState {
    homedir: Option<std::ffi::CString>,
    homedir_buf: [c_char; MAXPATHL],
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
    fn _NSGetEnviron() -> *mut *mut *mut c_char;
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[allow(non_upper_case_globals)]
    static environ: *mut *mut c_char;

    #[cfg(windows)]
    fn _putenv_s(varname: *const c_char, value_string: *const c_char) -> i32;

    #[cfg(not(windows))]
    fn gethostname(name: *mut c_char, len: usize) -> i32;
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn os_environ_ptr() -> *mut *mut *mut c_char {
    #[cfg(target_os = "macos")]
    unsafe {
        _NSGetEnviron()
    }
    #[cfg(not(target_os = "macos"))]
    {
        (&raw const environ).cast_mut()
    }
}

use crate::charset::NUL;
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
    {
        #[cfg(windows)]
        {
            !crate::memory::cstr_is_empty(name) && utf_valid_string(name, ptr::null())
        }
        #[cfg(not(windows))]
        {
            !crate::memory::cstr_is_empty(name)
        }
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
        let mut buf = [0 as c_char; INIT_SIZE];
        let mut r = unsafe { uv_os_getenv(name, buf.as_mut_ptr(), &mut size) };
        let res = if r == UV_ENOBUFS {
            let mut vec_buf = vec![0 as c_char; size + 1];
            r = unsafe { uv_os_getenv(name, vec_buf.as_mut_ptr(), &mut size) };
            if r != 0 || size == 0 || vec_buf[0] == NUL {
                None
            } else {
                let bytes = crate::memory::cstr_slice(vec_buf.as_ptr(), size);
                Some(cstr_from_bytes(bytes))
            }
        } else if r != 0 || size == 0 || buf[0] == NUL {
            None
        } else {
            let bytes = crate::memory::cstr_slice(buf.as_ptr(), size);
            Some(cstr_from_bytes(bytes))
        };
        assert_ne!(r, UV_EINVAL);
        res
    }
}

/// Like `getenv()`, but stores the value in `buf` instead of allocating.
pub fn os_getenv_buf<S: crate::strings::IntoCCharPtr>(
    name: S,
    buf: *mut c_char,
    bufsize: usize,
) -> *mut c_char {
    let name = name.into_c_char_ptr();
    {
        if !env_name_valid(name) {
            return ptr::null_mut();
        }
        {
            let mut size = bufsize;
            let mut r = unsafe { uv_os_getenv(name, buf, &mut size) };
            if r == UV_ENOBUFS {
                let mut e = vec![0 as c_char; size];
                r = unsafe { uv_os_getenv(name, e.as_mut_ptr(), &mut size) };
                if r == 0 && size != 0 && e[0] != NUL {
                    crate::memory::cstr_ncpy(buf, e.as_ptr(), bufsize.min(size) - 1);
                }
            }
            assert_ne!(r, UV_EINVAL);

            if r != 0 || size == 0 || crate::memory::cstr_is_empty(buf) {
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
            let mut buf = [0 as c_char; 2];
            let mut size = buf.len();
            let r = unsafe { uv_os_getenv(name, buf.as_mut_ptr(), &mut size) };
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
                let r = unsafe { _putenv_s(name, value) };
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
        let env_arr: *const *mut c_char = (*os_environ_ptr()).cast_const();

        for i in 0..=index {
            if (*env_arr.add(i)).is_null() {
                return None;
            }
        }
        let str_ptr = *env_arr.add(index);
        assert!(!str_ptr.is_null());
        let bytes = crate::memory::cstr_as_bytes(str_ptr);
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
pub fn os_get_hostname(hostname: *mut c_char, size: usize) {
    #[cfg(not(windows))]
    unsafe {
        if gethostname(hostname, size) < 0 {
            *hostname = NUL;
        } else if size > 0 {
            *hostname.add(size - 1) = NUL;
        }
    }
    #[cfg(windows)]
    {
        let mut host_utf16 = [0u16; 16];
        let mut host_wsize = host_utf16.len() as u32;
        if unsafe { GetComputerNameW(host_utf16.as_mut_ptr(), &mut host_wsize) } == 0 {
            unsafe {
                *hostname = NUL;
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
            xstrlcpy(hostname, host_cstr.as_ptr(), size);
        }
    }
}

/// Gets the "real", resolved user home directory as determined by `init_homedir`.
#[cfg(windows)]
pub fn os_homedir() -> *const c_char {
    let Some(ref hd) = env_st().homedir else {
        emsg(c"os_homedir failed: homedir not initialized");
        return ptr::null();
    };
    hd.as_ptr()
}

fn os_uv_homedir() -> *mut c_char {
    unsafe {
        let buf = (&raw mut env_st().homedir_buf).cast::<c_char>();
        *buf = NUL;
        let mut homedir_size = MAXPATHL;
        let ret_value = uv_os_homedir(buf, &mut homedir_size);
        if ret_value == 0 && homedir_size < MAXPATHL {
            return buf;
        }
        *buf = NUL;
        ptr::null_mut()
    }
}

/// Sets `homedir` to the "real", resolved user home directory.
pub fn init_homedir() {
    env_st().homedir = None;

    let home_cstr = os_getenv_cstr(c"HOME");
    let mut var: *mut c_char = home_cstr
        .as_ref()
        .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());

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
                let buf = (&raw mut g_st().os_buf).cast::<c_char>();
                xstrlcpy(buf, hd.as_ptr(), MAXPATHL);
                {
                    let _dst: *mut c_char = buf;
                    let _dsize: usize = MAXPATHL;
                    let _dlen = crate::memory::cstr_len(_dst);
                    crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), homepath_ptr, _dsize - _dlen);
                };
                if !crate::memory::cstr_is_empty(buf) {
                    var = buf;
                }
            }
        }
        if var.is_null() {
            var = os_uv_homedir();
        }

        if !var.is_null() && crate::memory::cstr_first_byte(var) == b'%' {
            let rest = crate::memory::cstr_as_bytes(var.wrapping_add(1));
            if let Some(pos) = rest.iter().position(|&b| b == b'%') {
                let p = var.wrapping_add(1 + pos);
                let buf = (&raw mut g_st().os_buf).cast::<c_char>();
                crate::memory::cstr_ncpy(buf, var.wrapping_add(1), pos);
                var = ptr::null_mut();
                if let Some(exp) = os_getenv_cstr(buf)
                    && !exp.as_bytes().is_empty()
                    && exp.as_bytes().len() + crate::memory::cstr_len(p) < MAXPATHL
                {
                    xstrlcpy(buf, exp.as_ptr(), MAXPATHL);
                    {
                        let _dst: *mut c_char = buf;
                        let _dsize: usize = MAXPATHL;
                        let _dlen = crate::memory::cstr_len(_dst);
                        crate::memory::xstrlcpy(
                            _dst.wrapping_add(_dlen),
                            p.wrapping_add(1),
                            _dsize - _dlen,
                        );
                    };
                    var = buf;
                }
            }
        }

        if crate::memory::cstr_is_empty(var) {
            var = c"C:/".as_ptr().cast_mut();
        }
    }

    #[cfg(not(windows))]
    {
        if var.is_null() {
            var = os_uv_homedir();
        }

        if !var.is_null() && !os_realpath(var, g_st().iobuff.as_mut_ptr(), IOSIZE).is_null() {
            var = g_st().iobuff.as_mut_ptr();
        }

        if (crate::memory::cstr_is_empty(var))
            && os_dirname(&raw mut g_st().os_buf, MAXPATHL)
        {
            var = (&raw mut g_st().os_buf).cast();
        }
    }

    if !var.is_null() {
        env_st().homedir = to_slash_save(var);
    }
}

pub fn free_homedir() {
    env_st().homedir = None;
}

/// Call `expand_env()` and store the result in an allocated string.
pub fn expand_env_save(src: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let src = src.into_c_char_ptr();
    expand_env_save_opt(src, false, ptr::null_mut())
}

/// Similar to `expand_env_save()` with `one` and `esc_chars`.
pub fn expand_env_save_opt(
    src: impl crate::strings::IntoCCharPtr,
    one: bool,
    esc_chars: impl crate::strings::IntoCCharPtr,
) -> std::ffi::CString {
    let src = src.into_c_char_ptr();
    let esc_chars = esc_chars.into_c_char_ptr();
    let mut p = [0 as c_char; MAXPATHL];
    let _ = expand_env_esc(
        src,
        p.as_mut_ptr(),
        MAXPATHL as i32,
        esc_chars,
        one,
        ptr::null_mut(),
    );
    cstr_to_cstring(p.as_ptr())
}

/// Expand environment variable with path name.
pub fn expand_env(src: impl crate::strings::IntoCCharPtr, dst: *mut c_char, dstlen: i32) -> usize {
    let src = src.into_c_char_ptr().cast_mut();
    expand_env_esc(src, dst, dstlen, ptr::null_mut(), false, ptr::null_mut())
}

/// Expand environment variable with path name and escaping.
pub fn expand_env_esc(
    srcp: impl crate::strings::IntoCCharPtr,
    mut dst: *mut c_char,
    mut dstlen: i32,
    esc_chars: impl crate::strings::IntoCCharPtr,
    one: bool,
    prefix: impl crate::strings::IntoCCharPtr,
) -> usize {
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

        let mut src = skipwhite(srcp);
        dstlen -= 1;
        while unsafe { *src } != 0 && dstlen > 0 {
            if crate::memory::cstr_first_byte(src) == b'`'
                && crate::memory::cstr_byte_at(src, 1) == b'='
            {
                let var = src;
                src = src.wrapping_add(2);
                let _ = skip_expr(&raw mut src, None);
                if crate::memory::cstr_first_byte(src) == b'`' {
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
            if crate::memory::cstr_first_byte(src) == b'$'
                || (crate::memory::cstr_first_byte(src) == b'~' && at_start)
            {
                let mut var_cstr: Option<std::ffi::CString> = None;
                let mut tail: *mut c_char;
                let mut var: *mut c_char;

                if crate::memory::cstr_first_byte(src) != b'~' {
                    tail = src.wrapping_add(1);
                    var = dst;
                    #[cfg(not(windows))]
                    let mut c = dstlen - 1;
                    #[cfg(windows)]
                    let c = dstlen - 1;

                    #[cfg(not(windows))]
                    if crate::memory::cstr_first_byte(tail) == b'{' {
                        tail = tail.wrapping_add(1);
                        while c > 0
                            && !crate::memory::cstr_is_empty(tail)
                            && crate::memory::cstr_first_byte(tail) != b'}'
                        {
                            c -= 1;
                            unsafe {
                                *var = *tail;
                            }
                            var = var.wrapping_add(1);
                            tail = tail.wrapping_add(1);
                        }
                    } else {
                        let len = env_name_len(tail).min(c.max(0) as usize);
                        unsafe { ptr::copy_nonoverlapping(tail, var, len) };
                        var = var.wrapping_add(len);
                        tail = tail.wrapping_add(len);
                    }

                    #[cfg(windows)]
                    {
                        let len = env_name_len(tail).min(c.max(0) as usize);
                        unsafe { ptr::copy_nonoverlapping(tail, var, len) };
                        var = var.wrapping_add(len);
                        tail = tail.wrapping_add(len);
                    }

                    #[cfg(not(windows))]
                    if crate::memory::cstr_byte_at(src, 1) == b'{'
                        && crate::memory::cstr_first_byte(tail) != b'}'
                    {
                        var = ptr::null_mut();
                    } else {
                        if crate::memory::cstr_byte_at(src, 1) == b'{' {
                            tail = tail.wrapping_add(1);
                        }
                        unsafe {
                            *var = NUL;
                        }
                        var_cstr = vim_getenv(dst);
                        if let Some(ref c) = var_cstr {
                            to_slash(c.as_ptr().cast_mut());
                        }
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());
                    }

                    #[cfg(windows)]
                    {
                        unsafe {
                            *var = NUL;
                        }
                        var_cstr = vim_getenv(dst);
                        if let Some(ref c) = var_cstr {
                            to_slash(c.as_ptr().cast_mut());
                        }
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());
                    }
                } else if crate::memory::cstr_byte_at(src, 1) == 0
                    || vim_ispathsep(i32::from(crate::memory::cstr_byte_at(src, 1)))
                    || crate::strings::vim_haschr(
                        c" ,\t\n",
                        i32::from(crate::memory::cstr_byte_at(src, 1)),
                    )
                {
                    var = unsafe { (&raw const env_st().homedir).as_ref() }
                        .unwrap()
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                    tail = src.wrapping_add(1);
                } else {
                    #[cfg(not(windows))]
                    {
                        tail = src;
                        var = dst;
                        let mut c = dstlen - 1;
                        while c > 0
                            && unsafe { *tail } != 0
                            && vim_isfilec(i32::from(crate::memory::cstr_first_byte(tail)))
                            && !vim_ispathsep(i32::from(crate::memory::cstr_first_byte(tail)))
                        {
                            c -= 1;
                            unsafe {
                                *var = *tail;
                            }
                            var = var.wrapping_add(1);
                            tail = tail.wrapping_add(1);
                        }
                        unsafe {
                            *var = NUL;
                        }
                        if crate::memory::cstr_is_empty(dst) {
                            var = ptr::null_mut();
                        } else {
                            var_cstr = os_get_userdir(crate::memory::cstr_as_opt_cstr(
                                dst.wrapping_add(1),
                            ));
                            var = var_cstr
                                .as_ref()
                                .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());
                        }
                    }
                    #[cfg(windows)]
                    {
                        var = ptr::null_mut();
                        tail = c"".as_ptr().cast_mut();
                    }
                }

                if !esc_chars.is_null() && !var.is_null() {
                    let var_bytes = crate::memory::cstr_as_bytes(var);
                    let esc_bytes = crate::memory::cstr_as_bytes(esc_chars);
                    if var_bytes.iter().any(|b| esc_bytes.contains(b)) {
                        var_cstr = Some(vim_strsave_escaped(var, esc_chars));
                        var = var_cstr
                            .as_ref()
                            .map_or(ptr::null_mut(), |c| c.as_ptr().cast_mut());
                    }
                }

                if !crate::memory::cstr_is_empty(var) {
                    let c = crate::memory::cstr_len(var);
                    if c + crate::memory::cstr_len(tail) + 1 < (dstlen as usize) {
                        unsafe { ptr::copy_nonoverlapping(var, dst, c + 1) };
                        dstlen -= c as i32;
                        let ends_with_sep = after_pathsep(dst, dst.wrapping_add(c)) != 0;
                        #[cfg(windows)]
                        let not_colon = crate::memory::cstr_byte_at(dst, c - 1) != b':';
                        #[cfg(not(windows))]
                        let not_colon = true;
                        if ends_with_sep
                            && not_colon
                            && vim_ispathsep(i32::from(crate::memory::cstr_first_byte(tail)))
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
                if crate::memory::cstr_first_byte(src) == b'\\'
                    && crate::memory::cstr_byte_at(src, 1) != 0
                {
                    unsafe {
                        *dst = *src;
                    }
                    dst = dst.wrapping_add(1);
                    src = src.wrapping_add(1);
                    dstlen -= 1;
                } else if (crate::memory::cstr_first_byte(src) == b' '
                    || crate::memory::cstr_first_byte(src) == b',')
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
                            std::slice::from_raw_parts(
                                src.wrapping_sub(prefix_len).cast::<u8>(),
                                prefix_len,
                            )
                        } == crate::memory::cstr_slice(prefix, prefix_len)
                    {
                        at_start = true;
                    }
                }
            }
        }
        unsafe {
            *dst = NUL;
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
    path: *mut c_char,
    pend: *mut c_char,
    dirname: impl crate::strings::IntoCCharPtr,
) -> *mut c_char {
    let dirname = dirname.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(dirname);
        if (pend as usize) > (path as usize) + len {
            let new_tail = pend.wrapping_sub(len + 1);
            if path_cmp(opt_vars().p_fic != 0, new_tail, dirname, len) == 0
                && (new_tail == path || after_pathsep(path, new_tail) != 0)
            {
                return new_tail;
            }
        }
    }
    pend
}

/// Iterates `$PATH`-like delimited list `val`.
pub fn vim_env_iter(
    delim: c_char,
    val: impl crate::strings::IntoCCharPtr,
    iter: impl crate::strings::IntoCCharPtr,
    dir: *mut *const c_char,
    len: &mut usize,
) -> *const c_char {
    let val = val.into_c_char_ptr();
    let iter = iter.into_c_char_ptr();
    {
        let varval = if iter.is_null() { val } else { iter };
        unsafe {
            *dir = varval;
        }
        let bytes = crate::memory::cstr_as_bytes(varval);
        if let Some(pos) = bytes.iter().position(|&b| b == (delim as u8)) {
            *len = pos;
            varval.wrapping_add(pos + 1)
        } else {
            *len = bytes.len();
            ptr::null()
        }
    }
}

/// Iterates `$PATH`-like delimited list `val` in reverse order.
pub fn vim_env_iter_rev(
    delim: c_char,
    val: impl crate::strings::IntoCCharPtr,
    iter: impl crate::strings::IntoCCharPtr,
    dir: *mut *const c_char,
    len: &mut usize,
) -> *const c_char {
    let val = val.into_c_char_ptr();
    let iter = iter.into_c_char_ptr();
    {
        let varend = if iter.is_null() {
            val.wrapping_add(crate::memory::cstr_len(val))
                .wrapping_sub(1)
        } else {
            iter
        };
        let varlen = (varend as usize).wrapping_sub(val as usize).wrapping_add(1);
        let colon = {
            let _s = (val).cast::<u8>();
            let _sz = varlen;
            if _sz == 0 {
                std::ptr::null_mut::<std::ffi::c_void>()
            } else {
                unsafe { std::slice::from_raw_parts(_s, _sz) }
                    .iter()
                    .rposition(|&_b| _b == delim as u8)
                    .map_or(std::ptr::null_mut::<std::ffi::c_void>(), |_i| {
                        _s.wrapping_add(_i).cast_mut().cast::<std::ffi::c_void>()
                    })
            }
        }
        .cast::<c_char>();
        if colon.is_null() {
            *len = varlen;
            unsafe {
                *dir = val;
            }
            return ptr::null();
        }
        unsafe {
            *dir = colon.wrapping_add(1);
        }
        *len = (varend as usize) - (colon as usize);
        colon.wrapping_sub(1)
    }
}

pub fn vim_get_prefix_from_exepath(exe_name: *mut c_char) {
    unsafe {
        xstrlcpy(exe_name, get_vim_var_str(VV_PROGPATH), MAXPATHL);
        let path_end = path_tail_with_sep(exe_name);
        *path_end = NUL;
        let path_end2 = path_tail(exe_name);
        *path_end2 = NUL;
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
            to_slash(c.as_ptr().cast_mut());
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
                let mut vim_path: *mut c_char = ptr::null_mut();
                if !opt_vars().p_hf.is_null()
                    && !crate::strings::vim_haschr(opt_vars().p_hf, b'$' as i32)
                {
                    vim_path = opt_vars().p_hf;
                }

                let mut exe_name = [0 as c_char; MAXPATHL];
                if vim_path.is_null() {
                    vim_get_prefix_from_exepath(exe_name.as_mut_ptr());
                    if append_path(exe_name.as_mut_ptr(), c"share/nvim/runtime/", MAXPATHL) {
                        vim_path = exe_name.as_mut_ptr();
                    }
                }

                if !vim_path.is_null() {
                    let mut vim_path_end = path_tail(vim_path);
                    if vim_path == opt_vars().p_hf {
                        vim_path_end = remove_tail(vim_path, vim_path_end, c"doc");
                    }
                    if !vimruntime {
                        vim_path_end = remove_tail(vim_path, vim_path_end, RUNTIME_DIRNAME);
                    }
                    if (vim_path_end as usize) > (vim_path as usize)
                        && after_pathsep(vim_path, vim_path_end) != 0
                    {
                        vim_path_end = vim_path_end.wrapping_sub(1);
                    }
                    assert!((vim_path_end as usize) >= (vim_path as usize));
                    let cand = std::ffi::CString::new(crate::memory::cstr_slice(
                        vim_path,
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
pub fn home_replace(
    buf: Option<&FileBuf>,
    src: impl crate::strings::IntoCCharPtr,
    dst: *mut c_char,
    mut dstlen: usize,
    one: bool,
) -> usize {
    let mut src = src.into_c_char_ptr();
    {
        if src.is_null() {
            unsafe {
                *dst = NUL;
            }
            return 0;
        }

        if buf.is_some() && buf.unwrap().b_help {
            let dlen = xstrlcpy(dst, path_tail(src), dstlen);
            return dlen.min(dstlen - 1);
        }

        let homedir_ptr = unsafe { (&raw const env_st().homedir).as_ref() }
            .unwrap()
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let dirlen = if homedir_ptr.is_null() {
            0
        } else {
            crate::memory::cstr_len(homedir_ptr)
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
            to_slash(c.as_ptr().cast_mut());
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
        let homedir_env_mod = homedir_env_mod.cast::<c_char>();

        let envlen = if homedir_env_mod.is_null() {
            0
        } else {
            crate::memory::cstr_len(homedir_env_mod)
        };

        if !one {
            src = skipwhite(src);
        }
        let mut dst_p = dst;
        while unsafe { *src } != 0 && dstlen > 0 {
            let mut p = homedir_ptr;
            let mut len = dirlen;
            loop {
                let cmp_ok = len != 0 && path_cmp(opt_vars().p_fic != 0, src, p, len) == 0;

                if cmp_ok
                    && (vim_ispathsep(i32::from(crate::memory::cstr_byte_at(src, len)))
                        || (!one
                            && (crate::memory::cstr_byte_at(src, len) == b','
                                || crate::memory::cstr_byte_at(src, len) == b' '))
                        || crate::memory::cstr_byte_at(src, len) == 0)
                {
                    src = src.wrapping_add(len);
                    dstlen -= 1;
                    if dstlen > 0 {
                        unsafe {
                            *dst_p = b'~' as c_char;
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
                    || (crate::memory::cstr_first_byte(src) != b','
                        && crate::memory::cstr_first_byte(src) != b' '))
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
            while crate::memory::cstr_first_byte(src) == b' '
                || crate::memory::cstr_first_byte(src) == b','
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
            *dst_p = NUL;
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
        let mut dst = vec![0 as c_char; len];
        let _ = home_replace(buf, src, dst.as_mut_ptr(), len, true);
        cstr_to_cstring(dst.as_ptr())
    }
}

/// Function given to `ExpandGeneric()` to obtain an environment variable name.
pub fn get_env_name(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    assert!(idx >= 0);
    if let Some(envname) = os_getenvname_at_index(idx as usize) {
        xstrlcpy(xp.xp_buf.as_mut_ptr(), envname.as_ptr(), EXPAND_BUF_LEN);
        return xp.xp_buf.as_mut_ptr();
    }
    ptr::null_mut()
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
        let buf = (&raw mut g_st().os_buf).cast::<c_char>();
        assert!((tail as usize) >= (fname as usize) && dirlen + 1 < MAXPATHL);
        crate::memory::cstr_ncpy(buf, fname, dirlen);
        let path_cstr = os_getenv_cstr(c"PATH");
        let pathlen = path_cstr.as_ref().map_or(0, |c| c.as_bytes().len());
        let newlen = pathlen + dirlen + 2;
        let mut retval = false;
        if newlen < MAX_ENVPATHLEN {
            let mut temp_vec = vec![0 as c_char; newlen];
            let temp = temp_vec.as_mut_ptr();
            if let Some(ref path) = path_cstr {
                xstrlcpy(temp, path.as_ptr(), newlen);
                if path.as_bytes().last().copied() != Some(ENV_SEPCHAR) {
                    {
                        let _dst: *mut c_char = temp;
                        let _dsize: usize = newlen;
                        let _dlen = crate::memory::cstr_len(_dst);
                        crate::memory::xstrlcpy(
                            _dst.wrapping_add(_dlen),
                            ENV_SEPSTR,
                            _dsize - _dlen,
                        );
                    };
                }
            }
            {
                let _dst: *mut c_char = temp;
                let _dsize: usize = newlen;
                let _dlen = crate::memory::cstr_len(_dst);
                crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), buf, _dsize - _dlen);
            };
            let _ = os_setenv(c"PATH", temp, 1);
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
            let mut buf = [0 as c_char; MAXPATHL];
            let comspec = os_getenv_buf(c"COMSPEC", buf.as_mut_ptr(), buf.len());
            return !comspec.is_null()
                && crate::memory::cstr_as_bytes(path_tail(comspec))
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
            let mut iter: *const c_char = ptr::null();
            let mut dir: *const c_char = ptr::null();
            let mut len: usize = 0;

            iter = vim_env_iter(b':' as c_char, val.as_ptr(), iter, &raw mut dir, &mut len);
            assert_eq!(len, 2);
            assert!(!iter.is_null());

            iter = vim_env_iter(b':' as c_char, val.as_ptr(), iter, &raw mut dir, &mut len);
            assert_eq!(len, 3);
            assert!(!iter.is_null());

            iter = vim_env_iter(b':' as c_char, val.as_ptr(), iter, &raw mut dir, &mut len);
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

            let mut buf = [0 as c_char; 64];
            assert_eq!(os_setenv(k.as_ptr(), c"val2", 0), 0);
            assert_eq!(
                crate::memory::cstr_from_ptr(os_getenv_buf(
                    k.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len()
                ))
                .to_str()
                .unwrap(),
                "val1"
            );
            assert_eq!(os_setenv(k.as_ptr(), c"val2", 1), 0);
            assert_eq!(
                crate::memory::cstr_from_ptr(os_getenv_buf(
                    k.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len()
                ))
                .to_str()
                .unwrap(),
                "val2"
            );

            assert!(!os_getenv_buf(k.as_ptr(), buf.as_mut_ptr(), buf.len()).is_null());
            assert_eq!(
                crate::memory::cstr_from_ptr(buf.as_ptr()).to_str().unwrap(),
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
                crate::memory::cstr_from_ptr(buf.as_ptr()).to_str().unwrap(),
                "val2/sub"
            );
            let _ = os_unsetenv(k.as_ptr());

            assert!(os_get_pid() > 0);
            os_get_hostname(buf.as_mut_ptr(), buf.len());
            assert!(!crate::memory::cstr_as_bytes(buf.as_ptr()).is_empty());
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
