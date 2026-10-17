//! XDG standard directory paths (`src/nvim/os/stdpaths.c`).

#![cfg_attr(any(test, target_os = "windows"), allow(dead_code, unused_imports))]

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use crate::memory::{xfree, xmallocz, xrealloc, xstrdup};

const MAXPATHL: usize = 4096;
const K_PATH_CMP_LITERAL: c_int = 1 << 0;

#[cfg(target_os = "windows")]
const ENV_SEP_BYTE: u8 = b';';
#[cfg(not(target_os = "windows"))]
const ENV_SEP_BYTE: u8 = b':';

#[cfg(target_os = "windows")]
const PATHSEP_BYTE: u8 = b'\\';
#[cfg(not(target_os = "windows"))]
const PATHSEP_BYTE: u8 = b'/';

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum XDGVarType {
    kXDGNone = -1,
    kXDGConfigHome = 0,
    kXDGDataHome = 1,
    kXDGCacheHome = 2,
    kXDGStateHome = 3,
    kXDGRuntimeDir = 4,
    kXDGConfigDirs = 5,
    kXDGDataDirs = 6,
}

const XDG_ENV_VARS: [&CStr; 7] = [
    c"XDG_CONFIG_HOME",
    c"XDG_DATA_HOME",
    c"XDG_CACHE_HOME",
    c"XDG_STATE_HOME",
    c"XDG_RUNTIME_DIR",
    c"XDG_CONFIG_DIRS",
    c"XDG_DATA_DIRS",
];

#[cfg(target_os = "windows")]
const XDG_DEFAULTS_ENV_VARS: [Option<&CStr>; 7] = [
    Some(c"LOCALAPPDATA"),
    Some(c"LOCALAPPDATA"),
    Some(c"TEMP"),
    Some(c"LOCALAPPDATA"),
    None,
    None,
    None,
];

#[cfg(target_os = "windows")]
const XDG_DEFAULTS: [Option<&CStr>; 7] = [
    Some(c"~/AppData/Local"),
    Some(c"~/AppData/Local"),
    Some(c"~/AppData/Local/Temp"),
    Some(c"~/AppData/Local"),
    None,
    None,
    None,
];

#[cfg(not(target_os = "windows"))]
const XDG_DEFAULTS: [Option<&CStr>; 7] = [
    Some(c"~/.config"),
    Some(c"~/.local/share"),
    Some(c"~/.cache"),
    Some(c"~/.local/state"),
    None,
    Some(c"/etc/xdg/"),
    Some(c"/usr/local/share/:/usr/share/"),
];

#[cfg(not(test))]
unsafe extern "C" {
    static mut NameBuff: [c_char; MAXPATHL];
    fn os_getenv_noalloc(name: *const c_char) -> *const c_char;
    fn os_getenv(name: *const c_char) -> *mut c_char;
    #[cfg(not(target_os = "windows"))]
    fn os_env_exists(name: *const c_char, nonempty: bool) -> bool;
    #[cfg(target_os = "windows")]
    fn os_realpath(name: *const c_char, buf: *mut c_char, len: usize) -> *mut c_char;
    fn expand_env_save(src: *mut c_char) -> *mut c_char;
    fn vim_gettempdir() -> *mut c_char;
    fn path_is_absolute(fname: *const c_char) -> bool;
    fn path_equal(s1: *const c_char, s2: *const c_char, flags: c_int) -> bool;
    fn concat_fnames_realloc(fname1: *mut c_char, fname2: *const c_char, sep: bool) -> *mut c_char;
}

#[inline]
unsafe fn to_slash_in_place(p: *mut c_char) {
    #[cfg(target_os = "windows")]
    if !p.is_null() {
        let mut cur = p.cast::<u8>();
        unsafe {
            while *cur != 0 {
                if *cur == b'\\' {
                    *cur = b'/';
                }
                cur = cur.add(1);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = p;
}

#[inline]
unsafe fn c_bytes_to_xmallocz(bytes: &[u8]) -> *mut c_char {
    let ptr = unsafe { xmallocz(bytes.len()).cast::<u8>() };
    unsafe {
        if !bytes.is_empty() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        }
        *ptr.add(bytes.len()) = 0;
    }
    ptr.cast::<c_char>()
}

/// Get the value of `$NVIM_APPNAME`, or `"nvim"` if not set (`get_appname`).
#[unsafe(no_mangle)]
pub extern "C" fn get_appname(namelike: bool) -> *const c_char {
    #[cfg(not(test))]
    unsafe {
        let env_val = os_getenv_noalloc(c"NVIM_APPNAME".as_ptr());
        let name_buff = (&raw mut NameBuff).cast::<u8>();
        if env_val.is_null() {
            name_buff.copy_from_nonoverlapping(c"nvim".as_ptr().cast::<u8>(), 5);
        }
        to_slash_in_place(name_buff.cast::<c_char>());
        if namelike {
            for i in 0..MAXPATHL {
                let b = *name_buff.add(i);
                if b == 0 {
                    break;
                }
                if b == b'/' || b == b'\\' {
                    *name_buff.add(i) = b'-';
                }
            }
        }
        name_buff.cast::<c_char>()
    }
    #[cfg(test)]
    {
        let _ = namelike;
        c"nvim".as_ptr()
    }
}

/// Return true if `$NVIM_APPNAME` is a valid name or relative path (`appname_is_valid`).
#[unsafe(no_mangle)]
pub extern "C" fn appname_is_valid() -> bool {
    let appname_ptr = get_appname(false);
    #[cfg(not(test))]
    if unsafe { path_is_absolute(appname_ptr) } {
        return false;
    }
    let bytes = unsafe { CStr::from_ptr(appname_ptr).to_bytes() };
    if matches!(bytes, b"/" | b"\\" | b"." | b"..") {
        return false;
    }
    if bytes.windows(3).any(|w| w == b"/.." || w == b"../") {
        return false;
    }
    true
}

#[cfg(not(test))]
unsafe fn xdg_remove_duplicate(ret: *mut c_char, sep: u8) -> *mut c_char {
    let input_bytes = unsafe { CStr::from_ptr(ret).to_bytes() };
    let mut token_cstrs: Vec<CString> = Vec::new();

    for part in input_bytes.split(|&b| b == sep) {
        if part.is_empty() {
            continue;
        }
        let Ok(cand) = CString::new(part) else {
            continue;
        };
        let is_dup = token_cstrs.iter().any(|existing| unsafe {
            path_equal(existing.as_ptr(), cand.as_ptr(), K_PATH_CMP_LITERAL)
        });
        if !is_dup {
            token_cstrs.push(cand);
        }
    }

    unsafe {
        xfree(ret.cast::<c_void>());
    }

    if token_cstrs.is_empty() {
        return std::ptr::null_mut();
    }

    let mut joined: Vec<u8> = Vec::new();
    for (i, tok) in token_cstrs.iter().enumerate() {
        if i > 0 {
            joined.push(sep);
        }
        joined.extend_from_slice(tok.as_bytes());
    }
    unsafe { c_bytes_to_xmallocz(&joined) }
}

/// Return allocated XDG variable value for `idx` (`stdpaths_get_xdg_var`).
#[unsafe(no_mangle)]
pub extern "C" fn stdpaths_get_xdg_var(idx: XDGVarType) -> *mut c_char {
    #[cfg(test)]
    {
        let _ = (
            idx,
            ENV_SEP_BYTE,
            K_PATH_CMP_LITERAL,
            &XDG_ENV_VARS,
            &XDG_DEFAULTS,
            c_bytes_to_xmallocz as unsafe fn(&[u8]) -> *mut c_char,
            to_slash_in_place as unsafe fn(*mut c_char),
        );
        unsafe { c_bytes_to_xmallocz(b"/tmp/nvim_test_xdg") }
    }

    #[cfg(not(test))]
    unsafe {
        let i = idx as usize;
        let env = XDG_ENV_VARS[i].as_ptr();
        let fallback = XDG_DEFAULTS[i];

        let mut env_val = os_getenv(env);

        #[cfg(target_os = "windows")]
        {
            if env_val.is_null()
                && let Some(def_env) = XDG_DEFAULTS_ENV_VARS[i]
            {
                env_val = os_getenv(def_env.as_ptr());
            }
            if !env_val.is_null() && idx == XDGVarType::kXDGCacheHome {
                let real_path = os_realpath(env_val, std::ptr::null_mut(), MAXPATHL);
                if !real_path.is_null() {
                    xfree(env_val.cast::<c_void>());
                    env_val = real_path;
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            if env_val.is_null() && os_env_exists(env, false) {
                env_val = xstrdup(c"".as_ptr());
            }
        }

        to_slash_in_place(env_val);

        let mut ret: *mut c_char = if !env_val.is_null() {
            env_val
        } else if let Some(fb) = fallback {
            expand_env_save(fb.as_ptr().cast_mut())
        } else if idx == XDGVarType::kXDGRuntimeDir {
            let tempdir = vim_gettempdir();
            let raw = if tempdir.is_null() {
                b"/tmp/" as &[u8]
            } else {
                CStr::from_ptr(tempdir).to_bytes()
            };
            let trimmed_len = if raw.len() >= 2 { raw.len() - 1 } else { 0 };
            c_bytes_to_xmallocz(&raw[..trimmed_len])
        } else {
            std::ptr::null_mut()
        };

        if matches!(idx, XDGVarType::kXDGDataDirs | XDGVarType::kXDGConfigDirs) && !ret.is_null() {
            ret = xdg_remove_duplicate(ret, ENV_SEP_BYTE);
        }

        ret
    }
}

/// Return Nvim-specific XDG directory subpath (`get_xdg_home`).
#[unsafe(no_mangle)]
pub extern "C" fn get_xdg_home(idx: XDGVarType) -> *mut c_char {
    let dir = stdpaths_get_xdg_var(idx);
    if dir.is_null() {
        return std::ptr::null_mut();
    }

    let appname_bytes = unsafe { CStr::from_ptr(get_appname(false)).to_bytes() };
    let mut suffix = Vec::with_capacity(appname_bytes.len() + 6);
    suffix.extend_from_slice(appname_bytes);
    #[cfg(target_os = "windows")]
    if matches!(idx, XDGVarType::kXDGDataHome | XDGVarType::kXDGStateHome) {
        suffix.extend_from_slice(b"-data");
    }
    suffix.push(0);

    #[cfg(not(test))]
    unsafe {
        concat_fnames_realloc(dir, suffix.as_ptr().cast::<c_char>(), true)
    }
    #[cfg(test)]
    dir
}

/// Return `$XDG_CACHE_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_cache_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_cache_subpath(fname: *const c_char) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGCacheHome), fname, true)
    }
    #[cfg(test)]
    {
        let _ = fname;
        get_xdg_home(XDGVarType::kXDGCacheHome)
    }
}

/// Return `$XDG_CONFIG_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_conf_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_conf_subpath(fname: *const c_char) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGConfigHome), fname, true)
    }
    #[cfg(test)]
    {
        let _ = fname;
        get_xdg_home(XDGVarType::kXDGConfigHome)
    }
}

/// Return `$XDG_DATA_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_data_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_data_subpath(fname: *const c_char) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGDataHome), fname, true)
    }
    #[cfg(test)]
    {
        let _ = fname;
        get_xdg_home(XDGVarType::kXDGDataHome)
    }
}

/// Return `$XDG_STATE_HOME/$NVIM_APPNAME/{fname}` with optional trailing path separators and
/// comma escaping (`stdpaths_user_state_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_state_subpath(
    fname: *const c_char,
    trailing_pathseps: usize,
    escape_commas: bool,
) -> *mut c_char {
    #[cfg(not(test))]
    let mut ret =
        unsafe { concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGStateHome), fname, true) };
    #[cfg(test)]
    let mut ret = {
        let _ = fname;
        unsafe { c_bytes_to_xmallocz(b"/state/a,b") }
    };

    let mut len = unsafe { CStr::from_ptr(ret).to_bytes().len() };
    let mut numcommas = if escape_commas {
        let bytes = unsafe { std::slice::from_raw_parts(ret.cast::<u8>(), len) };
        bytes.iter().filter(|&&b| b == b',').count()
    } else {
        0
    };

    if numcommas > 0 || trailing_pathseps > 0 {
        let mut newlen = len + numcommas + trailing_pathseps;
        ret = unsafe { xrealloc(ret.cast::<c_void>(), newlen + 1).cast::<c_char>() };
        unsafe {
            let p = ret.cast::<u8>();
            *p.add(newlen) = 0;
            if trailing_pathseps > 0 {
                std::ptr::write_bytes(p.add(len + numcommas), PATHSEP_BYTE, trailing_pathseps);
            }
            newlen -= trailing_pathseps;

            while numcommas > 0 && len > 0 {
                newlen -= 1;
                len -= 1;
                let ch = *p.add(len);
                *p.add(newlen) = ch;
                if ch == b',' {
                    newlen -= 1;
                    *p.add(newlen) = b'\\';
                    numcommas -= 1;
                }
            }
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_subpath_escapes_commas_and_appends_separators() {
        unsafe {
            let p = stdpaths_user_state_subpath(c"shada".as_ptr(), 2, true);
            let s = CStr::from_ptr(p).to_bytes();
            assert_eq!(s[..11], *b"/state/a\\,b");
            assert_eq!(s[11..13], [PATHSEP_BYTE, PATHSEP_BYTE]);
            xfree(p.cast::<c_void>());
        }
    }
}
