//! Environment variable inspection and path expansion (`src/nvim/os/env.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::buf_T;
use crate::charset::skipwhite;
#[cfg(unix)]
use crate::charset::vim_isfilec;
use crate::cursor::pos_T;
use crate::eval::fs::modify_fname;
use crate::eval::skip_expr;
use crate::eval::vars::{VV_PROGPATH, get_vim_var_str};
use crate::globals::*;
use crate::lua::converter::LuaRef;
use crate::mbyte::{mb_stricmp, utf_isalnum, utf_ptr2char, utf_ptr2len};
#[cfg(windows)]
use crate::mbyte::{utf_valid_string, utf16_to_utf8};
use crate::memory::{xfree, xmalloc, xmemcpyz, xmemdupz, xmemrchr, xstrdup, xstrlcat, xstrlcpy};
#[cfg(windows)]
use crate::message::semsg;
use crate::message::{emsg, internal_error};
use crate::option_vars::*;
#[cfg(not(windows))]
use crate::os::fs::ENV_SEPCHAR;
use crate::os::fs::{UV_EINVAL, os_isdir};
#[cfg(unix)]
use crate::os::fs::{os_dirname, os_realpath};
#[cfg(unix)]
use crate::os::users::os_get_userdir;
use crate::path::{
    after_pathsep, append_path, concat_fnames, path_cmp, path_is_absolute, path_tail,
    path_tail_with_sep, to_slash, to_slash_save, vim_ispathsep,
};
use crate::runtime::sctx_T;
#[cfg(windows)]
use crate::strings::wcslen;
use crate::strings::{strlen as c_strlen, vim_strchr, vim_strsave_escaped};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub(crate) const EXPAND_BUF_LEN: usize = 256;
const INIT_SIZE: usize = 64;
const RUNTIME_DIRNAME: &CStr = c"runtime";

#[cfg(windows)]
const ENV_SEPCHAR: u8 = b';';
#[cfg(windows)]
const ENV_SEPSTR: &CStr = c";";
#[cfg(windows)]
const PATHSEPSTR: &CStr = c"\\";
#[cfg(windows)]
const MAX_ENVPATHLEN: usize = 8192;

#[cfg(not(windows))]
const ENV_SEPSTR: &CStr = c":";
#[cfg(windows)]
#[cfg(not(windows))]
const PATHSEPSTR: &CStr = c"/";
#[cfg(not(windows))]
const MAX_ENVPATHLEN: usize = c_int::MAX as usize;
#[cfg(windows)]
const UV_ENOBUFS: c_int = -4060;
#[cfg(target_os = "linux")]
const UV_ENOBUFS: c_int = -105;
#[cfg(all(not(windows), not(target_os = "linux")))]
const UV_ENOBUFS: c_int = -55;

#[repr(C)]
pub struct expand_T {
    pub xp_pattern: *mut c_char,
    pub xp_context: c_int,
    pub xp_pattern_len: usize,
    pub xp_prefix: c_int,
    pub xp_arg: *mut c_char,
    pub xp_luaref: LuaRef,
    pub xp_script_ctx: sctx_T,
    pub xp_backslash: c_int,
    #[cfg(not(windows))]
    pub xp_shell: bool,
    pub xp_numfiles: c_int,
    pub xp_col: c_int,
    pub xp_selected: c_int,
    pub xp_orig: *mut c_char,
    pub xp_files: *mut *mut c_char,
    pub xp_files_abbr: *mut *mut c_char,
    pub xp_files_kind: *mut *mut c_char,
    pub xp_files_menu: *mut *mut c_char,
    pub xp_files_info: *mut *mut c_char,
    pub xp_line: *mut c_char,
    pub xp_buf: [c_char; EXPAND_BUF_LEN],
    pub xp_search_dir: c_int,
    pub xp_pre_incsearch_pos: pos_T,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct Utsname {
    sysname: [c_char; 65],
    nodename: [c_char; 65],
    release: [c_char; 65],
    version: [c_char; 65],
    machine: [c_char; 65],
    domainname: [c_char; 65],
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct Utsname {
    sysname: [c_char; 256],
    nodename: [c_char; 256],
    release: [c_char; 256],
    version: [c_char; 256],
    machine: [c_char; 256],
}

static mut HOMEDIR: *mut c_char = ptr::null_mut();
static mut HOMEDIR_BUF: [c_char; MAXPATHL] = [0; MAXPATHL];

use crate::version::{default_vim_dir, default_vimruntime_dir};

use crate::event::r#loop::{uv_os_getenv, uv_os_homedir, uv_os_setenv, uv_os_unsetenv};

unsafe extern "C" {
    #[cfg(target_os = "macos")]
    pub fn _NSGetEnviron() -> *mut *mut *mut c_char;
    #[cfg(all(not(windows), not(target_os = "macos")))]
    pub static mut environ: *mut *mut c_char;

    #[cfg(windows)]
    fn _putenv_s(varname: *const c_char, value_string: *const c_char) -> c_int;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn uname(buf: *mut Utsname) -> c_int;
    #[cfg(all(not(windows), not(any(target_os = "linux", target_os = "macos"))))]
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
}

use crate::charset::NUL;
#[cfg(unix)]
use crate::globals::IOSIZE;
use crate::math::OK;
use crate::os::fs::MAXPATHL;
#[cfg(windows)]
use crate::os::os_win_console::GetLastError;

#[cfg(windows)]
unsafe extern "system" {
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(lpszEnvironmentBlock: *mut u16) -> c_int;
    fn GetComputerNameW(lpBuffer: *mut u16, nSize: *mut u32) -> c_int;
}

#[inline]
unsafe fn striequal(a: *const c_char, b: *const c_char) -> bool {
    unsafe { mb_stricmp(a, b) == 0 }
}

#[inline]
unsafe fn env_name_valid(name: *const c_char) -> bool {
    unsafe {
        #[cfg(windows)]
        {
            *name != NUL && utf_valid_string(name, ptr::null())
        }
        #[cfg(not(windows))]
        {
            *name != NUL
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn env_init() {
    unsafe {
        nvim_testing = os_env_exists(c"NVIM_TEST".as_ptr(), false);
    }
}

/// Gets the length (bytes) of the env var name at `p`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn env_name_len(p: *const c_char) -> usize {
    unsafe {
        let mut s = p;
        loop {
            let b = *s as u8;
            if b < 0x80 {
                if !b.is_ascii_alphanumeric() && b != b'_' {
                    break;
                }
                s = s.add(1);
                continue;
            }
            let (len, is_alnum) = {
                let l = utf_ptr2len(s);
                (l, l != 1 && utf_isalnum(utf_ptr2char(s)))
            };
            if !is_alnum {
                break;
            }
            s = s.add(len as usize);
        }
        (s as usize) - (p as usize)
    }
}

/// Like `getenv()`, but returns NULL if the variable is empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_getenv(name: *const c_char) -> *mut c_char {
    unsafe {
        if !env_name_valid(name) {
            return ptr::null_mut();
        }
        {
            let mut size = INIT_SIZE;
            let mut buf = [0 as c_char; INIT_SIZE];
            let mut r = uv_os_getenv(name, buf.as_mut_ptr(), &raw mut size);
            let e: *mut c_char = if r == UV_ENOBUFS {
                let ptr = xmalloc(size).cast::<c_char>();
                r = uv_os_getenv(name, ptr, &raw mut size);
                if r != 0 || size == 0 || *ptr == NUL {
                    xfree(ptr.cast());
                    ptr::null_mut()
                } else {
                    ptr
                }
            } else if r != 0 || size == 0 || buf[0] == NUL {
                ptr::null_mut()
            } else {
                xmemdupz(buf.as_ptr().cast(), size).cast::<c_char>()
            };
            assert_ne!(r, UV_EINVAL);
            e
        }
    }
}

/// Like `getenv()`, but stores the value in `buf` instead of allocating.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_getenv_buf(
    name: *const c_char,
    buf: *mut c_char,
    bufsize: usize,
) -> *mut c_char {
    unsafe {
        if !env_name_valid(name) {
            return ptr::null_mut();
        }
        {
            let mut size = bufsize;
            let mut r = uv_os_getenv(name, buf, &raw mut size);
            if r == UV_ENOBUFS {
                let e = xmalloc(size).cast::<c_char>();
                r = uv_os_getenv(name, e, &raw mut size);
                if r == 0 && size != 0 && *e != NUL {
                    let _ = xmemcpyz(buf.cast(), e.cast(), bufsize.min(size) - 1);
                }
                xfree(e.cast());
            }
            assert_ne!(r, UV_EINVAL);

            if r != 0 || size == 0 || *buf == NUL {
                return ptr::null_mut();
            }
            buf
        }
    }
}

/// Like `getenv()`, but use `NameBuff` instead of allocating.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_getenv_noalloc(name: *const c_char) -> *mut c_char {
    unsafe { os_getenv_buf(name, (&raw mut NameBuff).cast(), MAXPATHL) }
}

/// Returns true if environment variable `name` is defined (even if empty).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_env_exists(name: *const c_char, nonempty: bool) -> bool {
    unsafe {
        if !env_name_valid(name) {
            return false;
        }
        {
            let mut buf = [0 as c_char; 2];
            let mut size = buf.len();
            let r = uv_os_getenv(name, buf.as_mut_ptr(), &raw mut size);
            assert_ne!(r, UV_EINVAL);
            (r == 0 && (!nonempty || size > 0)) || r == UV_ENOBUFS
        }
    }
}

/// Sets an environment variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_setenv(
    name: *const c_char,
    value: *const c_char,
    overwrite: c_int,
) -> c_int {
    unsafe {
        if !env_name_valid(name) {
            return -1;
        }
        #[cfg(windows)]
        {
            if overwrite == 0 && !os_env_exists(name, true) {
                return 0;
            }
            if *value == NUL {
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
            if striequal(name, c"LC_ALL".as_ptr())
                || striequal(name, c"LANGUAGE".as_ptr())
                || striequal(name, c"LANG".as_ptr())
                || striequal(name, c"LC_MESSAGES".as_ptr())
            {
                let r = _putenv_s(name, value);
                assert_eq!(r, 0);
            }
            let r = uv_os_setenv(name, value);
            assert_ne!(r, UV_EINVAL);
            if r == 0 { 0 } else { -1 }
        }
    }
}

/// Unset environment variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_unsetenv(name: *const c_char) -> c_int {
    unsafe {
        if !env_name_valid(name) {
            return -1;
        }
        {
            let r = uv_os_unsetenv(name);
            if r == 0 { 0 } else { -1 }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_free_fullenv(env: *mut *mut c_char) {
    if env.is_null() {
        return;
    }
    unsafe {
        let mut it = env;
        while !(*it).is_null() {
            xfree((*it).cast());
            *it = ptr::null_mut();
            it = it.add(1);
        }
        xfree(env.cast());
    }
}

/// Copy name of the environment variable at `index` in the current environment variables block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_getenvname_at_index(index: usize) -> *mut c_char {
    #[cfg(windows)]
    unsafe {
        let envstrings = GetEnvironmentStringsW();
        if envstrings.is_null() {
            return ptr::null_mut();
        }
        let mut p = envstrings;
        let mut name: *mut c_char = ptr::null_mut();
        let mut i: usize = 0;
        loop {
            let l = wcslen(p);
            if l == 0 || i > index {
                break;
            }
            if i == index {
                let mut utf8_str: *mut c_char = ptr::null_mut();
                let conversion_result = utf16_to_utf8(p, -1, &raw mut utf8_str);
                if conversion_result != 0 {
                    semsg!(c"utf16_to_utf8 failed: %d".as_ptr(), conversion_result);
                    break;
                }
                let start_off = usize::from(*utf8_str == b'=' as c_char);
                let s_bytes = CStr::from_ptr(utf8_str.add(start_off)).to_bytes();
                if let Some(pos) = s_bytes.iter().position(|&b| b == b'=') {
                    let len = start_off + pos;
                    assert!(len > 0);
                    name = xmemdupz(utf8_str.cast(), len).cast::<c_char>();
                }
                xfree(utf8_str.cast());
                break;
            }
            p = p.add(l + 1);
            i += 1;
        }
        FreeEnvironmentStringsW(envstrings);
        name
    }
    #[cfg(not(windows))]
    unsafe {
        #[cfg(target_os = "macos")]
        let env_arr: *const *mut c_char = (*_NSGetEnviron()).cast_const();
        #[cfg(not(target_os = "macos"))]
        let env_arr: *const *mut c_char = environ;

        for i in 0..=index {
            if (*env_arr.add(i)).is_null() {
                return ptr::null_mut();
            }
        }
        let str_ptr = *env_arr.add(index);
        assert!(!str_ptr.is_null());
        let bytes = CStr::from_ptr(str_ptr).to_bytes();
        let pos = bytes.iter().position(|&b| b == b'=').unwrap();
        assert!(pos > 0);
        xmemdupz(str_ptr.cast(), pos).cast::<c_char>()
    }
}

/// Get the process ID of the Nvim process.
#[unsafe(no_mangle)]
pub extern "C" fn os_get_pid() -> i64 {
    i64::from(std::process::id())
}

/// Signals to the OS that Nvim is an application for "interactive work".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_hint_priority() {}

/// Gets the hostname of the current machine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_get_hostname(hostname: *mut c_char, size: usize) {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        let mut vutsname: Utsname = std::mem::zeroed();
        if uname(&raw mut vutsname) < 0 {
            *hostname = NUL;
        } else {
            xstrlcpy(hostname, vutsname.nodename.as_ptr(), size);
        }
    }
    #[cfg(all(not(windows), not(any(target_os = "linux", target_os = "macos"))))]
    unsafe {
        if gethostname(hostname, size) < 0 {
            *hostname = NUL;
        } else if size > 0 {
            *hostname.add(size - 1) = NUL;
        }
    }
    #[cfg(windows)]
    unsafe {
        let mut host_utf16 = [0u16; 16];
        let mut host_wsize = host_utf16.len() as u32;
        if GetComputerNameW(host_utf16.as_mut_ptr(), &raw mut host_wsize) == 0 {
            *hostname = NUL;
            let err = GetLastError();
            semsg!(c"GetComputerNameW failed: %d".as_ptr(), err);
            return;
        }
        host_utf16[host_wsize as usize] = 0;

        let mut host_utf8: *mut c_char = ptr::null_mut();
        let conversion_result = utf16_to_utf8(host_utf16.as_ptr(), -1, &raw mut host_utf8);
        if conversion_result != 0 {
            semsg!(c"utf16_to_utf8 failed: %d".as_ptr(), conversion_result);
            return;
        }
        xstrlcpy(hostname, host_utf8, size);
        xfree(host_utf8.cast());
    }
}

/// Gets the "real", resolved user home directory as determined by `init_homedir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_homedir() -> *const c_char {
    unsafe {
        if HOMEDIR.is_null() {
            emsg(c"os_homedir failed: homedir not initialized".as_ptr());
            return ptr::null();
        }
        HOMEDIR
    }
}

unsafe fn os_uv_homedir() -> *mut c_char {
    unsafe {
        let buf = (&raw mut HOMEDIR_BUF).cast::<c_char>();
        *buf = NUL;
        let mut homedir_size = MAXPATHL;
        let ret_value = uv_os_homedir(buf, &raw mut homedir_size);
        if ret_value == 0 && homedir_size < MAXPATHL {
            return buf;
        }
        *buf = NUL;
        ptr::null_mut()
    }
}

/// Sets `homedir` to the "real", resolved user home directory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_homedir() {
    unsafe {
        xfree(HOMEDIR.cast());
        HOMEDIR = ptr::null_mut();

        let mut var = os_getenv(c"HOME".as_ptr());
        let tofree = var;

        #[cfg(windows)]
        {
            if var.is_null() {
                let homedrive = os_getenv(c"HOMEDRIVE".as_ptr());
                let mut homepath = os_getenv(c"HOMEPATH".as_ptr());
                if homepath.is_null() {
                    homepath = xstrdup(PATHSEPSTR.as_ptr());
                }
                if !homedrive.is_null() && c_strlen(homedrive) + c_strlen(homepath) < MAXPATHL {
                    let buf = (&raw mut os_buf).cast::<c_char>();
                    xstrlcpy(buf, homedrive, MAXPATHL);
                    xstrlcat(buf, homepath, MAXPATHL);
                    if *buf != NUL {
                        var = buf;
                    }
                }
                xfree(homepath.cast());
                xfree(homedrive.cast());
            }
            if var.is_null() {
                var = os_uv_homedir();
            }

            if !var.is_null() && *var == b'%' as c_char {
                let rest = CStr::from_ptr(var.add(1)).to_bytes();
                if let Some(pos) = rest.iter().position(|&b| b == b'%') {
                    let p = var.add(1 + pos);
                    let buf = (&raw mut os_buf).cast::<c_char>();
                    let _ = xmemcpyz(buf.cast(), var.add(1).cast(), pos);
                    var = ptr::null_mut();
                    let exp = os_getenv(buf);
                    if !exp.is_null() {
                        if *exp != NUL && c_strlen(exp) + c_strlen(p) < MAXPATHL {
                            xstrlcpy(buf, exp, MAXPATHL);
                            xstrlcat(buf, p.add(1), MAXPATHL);
                            var = buf;
                        }
                        xfree(exp.cast());
                    }
                }
            }

            if var.is_null() || *var == NUL {
                var = c"C:/".as_ptr().cast_mut();
            }
        }

        #[cfg(not(windows))]
        {
            if var.is_null() {
                var = os_uv_homedir();
            }

            if !var.is_null() && !os_realpath(var, (&raw mut IObuff).cast(), IOSIZE).is_null() {
                var = (&raw mut IObuff).cast();
            }

            if (var.is_null() || *var == NUL)
                && os_dirname((&raw mut os_buf).cast(), MAXPATHL) == OK
            {
                var = (&raw mut os_buf).cast();
            }
        }

        if !var.is_null() {
            HOMEDIR = to_slash_save(var);
        }
        xfree(tofree.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_homedir() {
    unsafe {
        xfree(HOMEDIR.cast());
        HOMEDIR = ptr::null_mut();
    }
}

/// Call `expand_env()` and store the result in an allocated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_env_save(src: *mut c_char) -> *mut c_char {
    unsafe { expand_env_save_opt(src, false, ptr::null_mut()) }
}

/// Similar to `expand_env_save()` with `one` and `esc_chars`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_env_save_opt(
    src: *mut c_char,
    one: bool,
    esc_chars: *mut c_char,
) -> *mut c_char {
    unsafe {
        let p = xmalloc(MAXPATHL).cast::<c_char>();
        let _ = expand_env_esc(src, p, MAXPATHL as c_int, esc_chars, one, ptr::null_mut());
        p
    }
}

/// Expand environment variable with path name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_env(src: *mut c_char, dst: *mut c_char, dstlen: c_int) -> usize {
    unsafe { expand_env_esc(src, dst, dstlen, ptr::null_mut(), false, ptr::null_mut()) }
}

/// Expand environment variable with path name and escaping.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_env_esc(
    srcp: *const c_char,
    mut dst: *mut c_char,
    mut dstlen: c_int,
    esc_chars: *mut c_char,
    one: bool,
    prefix: *mut c_char,
) -> usize {
    unsafe {
        let mut at_start = true;
        let dst_start = dst;
        let prefix_len: usize = if prefix.is_null() {
            0
        } else {
            c_strlen(prefix)
        };

        let mut src = skipwhite(srcp);
        dstlen -= 1;
        while *src != 0 && dstlen > 0 {
            if *src == b'`' as c_char && *src.add(1) == b'=' as c_char {
                let var = src;
                src = src.add(2);
                let _ = skip_expr(&raw mut src, ptr::null_mut());
                if *src == b'`' as c_char {
                    src = src.add(1);
                }
                let mut len = (src as usize) - (var as usize);
                if len > (dstlen as usize) {
                    len = dstlen as usize;
                }
                ptr::copy_nonoverlapping(var, dst, len);
                dst = dst.add(len);
                dstlen -= len as c_int;
                continue;
            }

            let mut copy_char = true;
            if *src == b'$' as c_char || (*src == b'~' as c_char && at_start) {
                let mut mustfree = false;
                let mut tail: *mut c_char;
                let mut var: *mut c_char;

                if *src != b'~' as c_char {
                    tail = src.add(1);
                    var = dst;
                    #[cfg(not(windows))]
                    let mut c = dstlen - 1;
                    #[cfg(windows)]
                    let c = dstlen - 1;

                    #[cfg(not(windows))]
                    if *tail == b'{' as c_char {
                        tail = tail.add(1);
                        while c > 0 && *tail != NUL && *tail != b'}' as c_char {
                            c -= 1;
                            *var = *tail;
                            var = var.add(1);
                            tail = tail.add(1);
                        }
                    } else {
                        let len = env_name_len(tail).min(c.max(0) as usize);
                        ptr::copy_nonoverlapping(tail, var, len);
                        var = var.add(len);
                        tail = tail.add(len);
                    }

                    #[cfg(windows)]
                    {
                        let len = env_name_len(tail).min(c.max(0) as usize);
                        ptr::copy_nonoverlapping(tail, var, len);
                        var = var.add(len);
                        tail = tail.add(len);
                    }

                    #[cfg(not(windows))]
                    if *src.add(1) == b'{' as c_char && *tail != b'}' as c_char {
                        var = ptr::null_mut();
                    } else {
                        if *src.add(1) == b'{' as c_char {
                            tail = tail.add(1);
                        }
                        *var = NUL;
                        var = vim_getenv(dst);
                        to_slash(var);
                        mustfree = true;
                    }

                    #[cfg(windows)]
                    {
                        *var = NUL;
                        var = vim_getenv(dst);
                        to_slash(var);
                        mustfree = true;
                    }
                } else if *src.add(1) == NUL
                    || vim_ispathsep(c_int::from(*src.add(1) as u8))
                    || !vim_strchr(c" ,\t\n".as_ptr(), c_int::from(*src.add(1) as u8)).is_null()
                {
                    var = HOMEDIR;
                    tail = src.add(1);
                } else {
                    #[cfg(not(windows))]
                    {
                        tail = src;
                        var = dst;
                        let mut c = dstlen - 1;
                        while c > 0
                            && *tail != 0
                            && vim_isfilec(c_int::from(*tail as u8))
                            && !vim_ispathsep(c_int::from(*tail as u8))
                        {
                            c -= 1;
                            *var = *tail;
                            var = var.add(1);
                            tail = tail.add(1);
                        }
                        *var = NUL;
                        var = if *dst == NUL {
                            ptr::null_mut()
                        } else {
                            os_get_userdir(dst.add(1))
                        };
                        mustfree = true;
                    }
                    #[cfg(windows)]
                    {
                        var = ptr::null_mut();
                        tail = c"".as_ptr().cast_mut();
                    }
                }

                if !esc_chars.is_null() && !var.is_null() {
                    let var_bytes = CStr::from_ptr(var).to_bytes();
                    let esc_bytes = CStr::from_ptr(esc_chars).to_bytes();
                    if var_bytes.iter().any(|b| esc_bytes.contains(b)) {
                        let p = vim_strsave_escaped(var, esc_chars);
                        if mustfree {
                            xfree(var.cast());
                        }
                        var = p;
                        mustfree = true;
                    }
                }

                if !var.is_null() && *var != NUL {
                    let c = c_strlen(var);
                    if c + c_strlen(tail) + 1 < (dstlen as usize) {
                        ptr::copy_nonoverlapping(var, dst, c + 1);
                        dstlen -= c as c_int;
                        let ends_with_sep = after_pathsep(dst, dst.add(c)) != 0;
                        #[cfg(windows)]
                        let not_colon = *dst.add(c - 1) != b':' as c_char;
                        #[cfg(not(windows))]
                        let not_colon = true;
                        if ends_with_sep && not_colon && vim_ispathsep(c_int::from(*tail as u8)) {
                            tail = tail.add(1);
                        }
                        dst = dst.add(c);
                        src = tail;
                        copy_char = false;
                    }
                }
                if mustfree {
                    xfree(var.cast());
                }
            }

            if copy_char {
                at_start = false;
                if *src == b'\\' as c_char && *src.add(1) != NUL {
                    *dst = *src;
                    dst = dst.add(1);
                    src = src.add(1);
                    dstlen -= 1;
                } else if (*src == b' ' as c_char || *src == b',' as c_char) && !one {
                    at_start = true;
                }
                if dstlen > 0 {
                    *dst = *src;
                    dst = dst.add(1);
                    src = src.add(1);
                    dstlen -= 1;

                    if !prefix.is_null()
                        && (src as usize) >= (srcp as usize) + prefix_len
                        && std::slice::from_raw_parts(src.sub(prefix_len).cast::<u8>(), prefix_len)
                            == std::slice::from_raw_parts(prefix.cast::<u8>(), prefix_len)
                    {
                        at_start = true;
                    }
                }
            }
        }
        *dst = NUL;

        (dst as usize) - (dst_start as usize)
    }
}

unsafe fn vim_runtime_dir(vimdir: *const c_char) -> *mut c_char {
    if vimdir.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        if *vimdir == NUL {
            return ptr::null_mut();
        }
        {
            let vimdir_len = c_strlen(vimdir);
            let p = concat_fnames(
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
            if os_isdir(p.data) {
                return p.data;
            }
            xfree(p.data.cast());
        }
        ptr::null_mut()
    }
}

unsafe fn remove_tail(path: *mut c_char, pend: *mut c_char, dirname: *const c_char) -> *mut c_char {
    unsafe {
        let len = c_strlen(dirname);
        if (pend as usize) > (path as usize) + len {
            let new_tail = pend.sub(len + 1);
            if path_cmp(p_fic != 0, new_tail, dirname, len) == 0
                && (new_tail == path || after_pathsep(path, new_tail) != 0)
            {
                return new_tail;
            }
        }
    }
    pend
}

/// Iterates `$PATH`-like delimited list `val`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_env_iter(
    delim: c_char,
    val: *const c_char,
    iter: *const c_void,
    dir: *mut *const c_char,
    len: *mut usize,
) -> *const c_void {
    unsafe {
        let varval = if iter.is_null() {
            val
        } else {
            iter.cast::<c_char>()
        };
        *dir = varval;
        let bytes = CStr::from_ptr(varval).to_bytes();
        if let Some(pos) = bytes.iter().position(|&b| b == (delim as u8)) {
            *len = pos;
            varval.add(pos + 1).cast::<c_void>()
        } else {
            *len = bytes.len();
            ptr::null()
        }
    }
}

/// Iterates `$PATH`-like delimited list `val` in reverse order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_env_iter_rev(
    delim: c_char,
    val: *const c_char,
    iter: *const c_void,
    dir: *mut *const c_char,
    len: *mut usize,
) -> *const c_void {
    unsafe {
        let varend = if iter.is_null() {
            val.add(c_strlen(val)).sub(1)
        } else {
            iter.cast::<c_char>()
        };
        let varlen = (varend as usize).wrapping_sub(val as usize).wrapping_add(1);
        let colon = xmemrchr(val.cast(), delim as u8, varlen).cast::<c_char>();
        if colon.is_null() {
            *len = varlen;
            *dir = val;
            return ptr::null();
        }
        *dir = colon.add(1);
        *len = (varend as usize) - (colon as usize);
        colon.sub(1).cast::<c_void>()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_get_prefix_from_exepath(exe_name: *mut c_char) {
    unsafe {
        xstrlcpy(exe_name, get_vim_var_str(VV_PROGPATH), MAXPATHL);
        let path_end = path_tail_with_sep(exe_name);
        *path_end = NUL;
        let path_end2 = path_tail(exe_name);
        *path_end2 = NUL;
    }
}

/// Vim `getenv()` wrapper with special handling of `$HOME`, `$VIM`, `$VIMRUNTIME`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_getenv(name: *const c_char) -> *mut c_char {
    unsafe {
        assert!(*get_vim_var_str(VV_PROGPATH) != NUL);

        #[cfg(windows)]
        if CStr::from_ptr(name).to_bytes() == b"HOME" {
            return xstrdup(HOMEDIR);
        }

        let mut kos_env_path = os_getenv(name);
        #[cfg(windows)]
        if striequal(name, c"VIMRUNTIME".as_ptr())
            || striequal(name, c"PATH".as_ptr())
            || striequal(name, c"CDPATH".as_ptr())
            || striequal(name, c"TMPDIR".as_ptr())
            || striequal(name, c"TMP".as_ptr())
            || striequal(name, c"TEMP".as_ptr())
            || striequal(name, c"VIM".as_ptr())
            || striequal(name, c"MYVIMRC".as_ptr())
        {
            to_slash(kos_env_path);
        }
        if !kos_env_path.is_null() {
            return kos_env_path;
        }

        let name_bytes = CStr::from_ptr(name).to_bytes();
        let vimruntime = name_bytes == b"VIMRUNTIME";
        if !vimruntime && name_bytes != b"VIM" {
            return ptr::null_mut();
        }

        {
            let mut vim_path: *mut c_char = ptr::null_mut();
            if vimruntime && *default_vimruntime_dir == NUL {
                kos_env_path = os_getenv(c"VIM".as_ptr());
                if !kos_env_path.is_null() {
                    vim_path = vim_runtime_dir(kos_env_path);
                    if vim_path.is_null() {
                        vim_path = kos_env_path;
                    } else {
                        xfree(kos_env_path.cast());
                    }
                }
            }

            if vim_path.is_null() {
                if !p_hf.is_null() && vim_strchr(p_hf, b'$' as c_int).is_null() {
                    vim_path = p_hf;
                }

                let mut exe_name = [0 as c_char; MAXPATHL];
                if vim_path.is_null() {
                    vim_get_prefix_from_exepath(exe_name.as_mut_ptr());
                    if append_path(
                        exe_name.as_mut_ptr(),
                        c"share/nvim/runtime/".as_ptr(),
                        MAXPATHL,
                    ) == OK
                    {
                        vim_path = exe_name.as_mut_ptr();
                    }
                }

                if !vim_path.is_null() {
                    let mut vim_path_end = path_tail(vim_path);
                    if vim_path == p_hf {
                        vim_path_end = remove_tail(vim_path, vim_path_end, c"doc".as_ptr());
                    }
                    if !vimruntime {
                        vim_path_end =
                            remove_tail(vim_path, vim_path_end, RUNTIME_DIRNAME.as_ptr());
                    }
                    if (vim_path_end as usize) > (vim_path as usize)
                        && after_pathsep(vim_path, vim_path_end) != 0
                    {
                        vim_path_end = vim_path_end.sub(1);
                    }
                    assert!((vim_path_end as usize) >= (vim_path as usize));
                    vim_path = xmemdupz(
                        vim_path.cast(),
                        (vim_path_end as usize) - (vim_path as usize),
                    )
                    .cast::<c_char>();
                    if !os_isdir(vim_path) {
                        xfree(vim_path.cast());
                        vim_path = ptr::null_mut();
                    }
                }
            }

            if vim_path.is_null() {
                if vimruntime && *default_vimruntime_dir != NUL {
                    vim_path = xstrdup(default_vimruntime_dir);
                } else if *default_vim_dir != NUL {
                    if vimruntime {
                        vim_path = vim_runtime_dir(default_vim_dir);
                    }
                    if vim_path.is_null() {
                        vim_path = xstrdup(default_vim_dir);
                    }
                }
            }

            if !vim_path.is_null() {
                if vimruntime {
                    let _ = os_setenv(c"VIMRUNTIME".as_ptr(), vim_path, 1);
                    didset_vimruntime = true;
                } else {
                    let _ = os_setenv(c"VIM".as_ptr(), vim_path, 1);
                    didset_vim = true;
                }
            }
            vim_path
        }
    }
}

/// Replace home directory by `"~"` in each space or comma separated file name in `src`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn home_replace(
    buf: *const buf_T,
    mut src: *const c_char,
    dst: *mut c_char,
    mut dstlen: usize,
    one: bool,
) -> usize {
    unsafe {
        if src.is_null() {
            *dst = NUL;
            return 0;
        }

        if !buf.is_null() && (*buf).b_help {
            let dlen = xstrlcpy(dst, path_tail(src), dstlen);
            return dlen.min(dstlen - 1);
        }

        let dirlen = if HOMEDIR.is_null() {
            0
        } else {
            c_strlen(HOMEDIR)
        };

        #[cfg(windows)]
        let mut homedir_env = os_getenv(c"HOME".as_ptr());
        #[cfg(not(windows))]
        let homedir_env = os_getenv(c"HOME".as_ptr());
        #[cfg(windows)]
        if homedir_env.is_null() {
            homedir_env = os_getenv(c"USERPROFILE".as_ptr());
        }
        to_slash(homedir_env);
        let mut homedir_env_mod = homedir_env;
        let mut must_free = false;

        if !homedir_env_mod.is_null() && *homedir_env_mod == b'~' as c_char {
            must_free = true;
            let mut usedlen: usize = 0;
            let mut flen = c_strlen(homedir_env_mod);
            let mut fbuf: *mut c_char = ptr::null_mut();
            let _ = modify_fname(
                c":p".as_ptr().cast_mut(),
                false,
                &raw mut usedlen,
                &raw mut homedir_env_mod,
                &raw mut fbuf,
                &raw mut flen,
                false,
            );
            flen = c_strlen(homedir_env_mod);
            assert_ne!(homedir_env_mod, homedir_env);
            if flen > 0 && vim_ispathsep(c_int::from(*homedir_env_mod.add(flen - 1) as u8)) {
                *homedir_env_mod.add(flen - 1) = NUL;
            }
        }

        let envlen = if homedir_env_mod.is_null() {
            0
        } else {
            c_strlen(homedir_env_mod)
        };

        if !one {
            src = skipwhite(src);
        }
        let mut dst_p = dst;
        while *src != 0 && dstlen > 0 {
            let mut p = HOMEDIR;
            let mut len = dirlen;
            loop {
                let cmp_ok = len != 0 && path_cmp(p_fic != 0, src, p, len) == 0;

                if cmp_ok
                    && (vim_ispathsep(c_int::from(*src.add(len) as u8))
                        || (!one
                            && (*src.add(len) == b',' as c_char
                                || *src.add(len) == b' ' as c_char))
                        || *src.add(len) == NUL)
                {
                    src = src.add(len);
                    dstlen -= 1;
                    if dstlen > 0 {
                        *dst_p = b'~' as c_char;
                        dst_p = dst_p.add(1);
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
            while *src != 0 && (one || (*src != b',' as c_char && *src != b' ' as c_char)) {
                dstlen -= 1;
                if dstlen == 0 {
                    break;
                }
                *dst_p = *src;
                dst_p = dst_p.add(1);
                src = src.add(1);
            }
            if dstlen == 0 {
                break;
            }
            while *src == b' ' as c_char || *src == b',' as c_char {
                dstlen -= 1;
                if dstlen == 0 {
                    break;
                }
                *dst_p = *src;
                dst_p = dst_p.add(1);
                src = src.add(1);
            }
        }

        *dst_p = NUL;
        xfree(homedir_env.cast());
        if must_free {
            xfree(homedir_env_mod.cast());
        }
        (dst_p as usize) - (dst as usize)
    }
}

/// Like `home_replace`, store the replaced string in allocated memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn home_replace_save(buf: *mut buf_T, src: *const c_char) -> *mut c_char {
    unsafe {
        let mut len: usize = 3;
        if !src.is_null() {
            len += c_strlen(src);
        }
        let dst = xmalloc(len).cast::<c_char>();
        let _ = home_replace(buf, src, dst, len, true);
        dst
    }
}

/// Function given to `ExpandGeneric()` to obtain an environment variable name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_env_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    assert!(idx >= 0);
    unsafe {
        let envname = os_getenvname_at_index(idx as usize);
        if !envname.is_null() {
            xstrlcpy((*xp).xp_buf.as_mut_ptr(), envname, EXPAND_BUF_LEN);
            xfree(envname.cast());
            return (*xp).xp_buf.as_mut_ptr();
        }
        ptr::null_mut()
    }
}

/// Appends the head of `fname` to `$PATH` and sets it in the environment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_setenv_append_path(fname: *const c_char) -> bool {
    unsafe {
        if !path_is_absolute(fname) {
            internal_error(c"os_setenv_append_path()".as_ptr());
            return false;
        }
        let tail = path_tail_with_sep(fname.cast_mut());
        let dirlen = (tail as usize) - (fname as usize);
        let buf = (&raw mut os_buf).cast::<c_char>();
        assert!((tail as usize) >= (fname as usize) && dirlen + 1 < MAXPATHL);
        let _ = xmemcpyz(buf.cast(), fname.cast(), dirlen);
        let path = os_getenv(c"PATH".as_ptr());
        let pathlen = if path.is_null() { 0 } else { c_strlen(path) };
        let newlen = pathlen + dirlen + 2;
        let mut retval = false;
        if newlen < MAX_ENVPATHLEN {
            let temp = xmalloc(newlen).cast::<c_char>();
            if pathlen == 0 {
                *temp = NUL;
            } else {
                xstrlcpy(temp, path, newlen);
                if (*path.add(pathlen - 1) as u8) != ENV_SEPCHAR {
                    xstrlcat(temp, ENV_SEPSTR.as_ptr(), newlen);
                }
            }
            xstrlcat(temp, buf, newlen);
            let _ = os_setenv(c"PATH".as_ptr(), temp, 1);
            xfree(temp.cast());
            retval = true;
        }
        xfree(path.cast());
        retval
    }
}

/// Returns true if `sh` looks like it resolves to `"cmd.exe"`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_shell_is_cmdexe(sh: *const c_char) -> bool {
    unsafe {
        if *sh == NUL {
            return false;
        }
        if striequal(sh, c"$COMSPEC".as_ptr()) {
            let comspec = os_getenv_noalloc(c"COMSPEC".as_ptr());
            return !comspec.is_null() && striequal(c"cmd.exe".as_ptr(), path_tail(comspec));
        }
        if striequal(sh, c"cmd.exe".as_ptr()) || striequal(sh, c"cmd".as_ptr()) {
            return true;
        }
        striequal(c"cmd.exe".as_ptr(), path_tail(sh))
    }
}

/// Removes environment variable `var` and takes care of side effects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_unsetenv_ext(var: *const c_char) {
    unsafe {
        let _ = os_unsetenv(var);
        if striequal(var, c"VIM".as_ptr()) {
            didset_vim = false;
        } else if striequal(var, c"VIMRUNTIME".as_ptr()) {
            didset_vimruntime = false;
        }
    }
}

/// Set environment variable `name` and take care of side effects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_setenv_ext(name: *const c_char, val: *const c_char) {
    unsafe {
        let _ = os_setenv(name, val, 1);
        if striequal(name, c"HOME".as_ptr()) {
            init_homedir();
        } else {
            if didset_vim && striequal(name, c"VIM".as_ptr()) {
                didset_vim = false;
            } else if didset_vimruntime && striequal(name, c"VIMRUNTIME".as_ptr()) {
                didset_vimruntime = false;
            }
        }
    }
}

/// Restore a previous environment variable value, or unset it if NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn restore_env_var(
    name: *const c_char,
    old_value: *mut c_char,
    must_free: bool,
) {
    unsafe {
        if !old_value.is_null() {
            let _ = os_setenv(name, old_value, 1);
            if must_free {
                xfree(old_value.cast());
            }
            return;
        }
        let _ = os_unsetenv(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_name_len_and_vim_env_iter_work() {
        unsafe {
            assert_eq!(env_name_len(c"FOO_123/bar".as_ptr()), 7);
            assert_eq!(env_name_len(c"-invalid".as_ptr()), 0);

            let val = c"/a:/bb:/ccc";
            let mut iter: *const c_void = ptr::null();
            let mut dir: *const c_char = ptr::null();
            let mut len: usize = 0;

            iter = vim_env_iter(
                b':' as c_char,
                val.as_ptr(),
                iter,
                &raw mut dir,
                &raw mut len,
            );
            assert_eq!(len, 2);
            assert!(!iter.is_null());

            iter = vim_env_iter(
                b':' as c_char,
                val.as_ptr(),
                iter,
                &raw mut dir,
                &raw mut len,
            );
            assert_eq!(len, 3);
            assert!(!iter.is_null());

            iter = vim_env_iter(
                b':' as c_char,
                val.as_ptr(),
                iter,
                &raw mut dir,
                &raw mut len,
            );
            assert_eq!(len, 4);
            assert!(iter.is_null());
        }
    }
}
