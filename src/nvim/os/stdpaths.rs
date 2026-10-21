//! XDG standard directory paths (`src/nvim/os/stdpaths.c`).

use crate::fileio::vim_gettempdir;
use crate::garray::K_PATH_CMP_LITERAL;
use crate::globals::*;
#[cfg(unix)]
use crate::memory::xstrdup;
use crate::memory::{xfree, xmemdupz, xrealloc};
#[cfg(unix)]
use crate::os::env::os_env_exists;
use crate::os::env::{expand_env_save, os_getenv, os_getenv_noalloc};
#[cfg(windows)]
use crate::os::fs::os_realpath;
use crate::os::fs::{ENV_SEPCHAR, MAXPATHL, PATHSEP};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{concat_fnames_realloc, path_equal, path_is_absolute};
use std::ffi::{CStr, CString, c_char, c_void};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Get the value of `$NVIM_APPNAME`, or `"nvim"` if not set (`get_appname`).
#[unsafe(no_mangle)]
pub extern "C" fn get_appname(namelike: bool) -> *const c_char {
    unsafe {
        let env_val = os_getenv_noalloc(c"NVIM_APPNAME".as_ptr());
        let name_buff = (&raw mut NameBuff).cast::<u8>();
        if env_val.is_null() {
            name_buff.copy_from_nonoverlapping(c"nvim".as_ptr().cast::<u8>(), 5);
        }
        #[cfg(windows)]
        path_to_slash(name_buff.cast::<c_char>());
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
}

/// Return true if `$NVIM_APPNAME` is a valid name or relative path (`appname_is_valid`).
#[unsafe(no_mangle)]
pub extern "C" fn appname_is_valid() -> bool {
    let appname_ptr = get_appname(false);
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
    unsafe { xmemdupz(joined.as_ptr().cast(), joined.len()).cast::<c_char>() }
}

/// Return allocated XDG variable value for `idx` (`stdpaths_get_xdg_var`).
#[unsafe(no_mangle)]
pub extern "C" fn stdpaths_get_xdg_var(idx: XDGVarType) -> *mut c_char {
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

        #[cfg(windows)]
        if !env_val.is_null() {
            path_to_slash(env_val);
        }

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
            xmemdupz(raw.as_ptr().cast(), trimmed_len).cast::<c_char>()
        } else {
            std::ptr::null_mut()
        };

        if matches!(idx, XDGVarType::kXDGDataDirs | XDGVarType::kXDGConfigDirs) && !ret.is_null() {
            ret = xdg_remove_duplicate(ret, ENV_SEPCHAR);
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

    unsafe { concat_fnames_realloc(dir, suffix.as_ptr().cast::<c_char>(), true) }
}

/// Return `$XDG_CACHE_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_cache_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_cache_subpath(fname: *const c_char) -> *mut c_char {
    unsafe { concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGCacheHome), fname, true) }
}

/// Return `$XDG_CONFIG_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_conf_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_conf_subpath(fname: *const c_char) -> *mut c_char {
    unsafe { concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGConfigHome), fname, true) }
}

/// Return `$XDG_DATA_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_data_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stdpaths_user_data_subpath(fname: *const c_char) -> *mut c_char {
    unsafe { concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGDataHome), fname, true) }
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
    let mut ret =
        unsafe { concat_fnames_realloc(get_xdg_home(XDGVarType::kXDGStateHome), fname, true) };

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
                std::ptr::write_bytes(p.add(len + numcommas), PATHSEP as u8, trailing_pathseps);
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
            let p = stdpaths_user_state_subpath(c"a,b".as_ptr(), 2, true);
            let s = CStr::from_ptr(p).to_bytes();
            let expected_suffix = [b'/', b'a', b'\\', b',', b'b', PATHSEP as u8, PATHSEP as u8];
            assert!(s.ends_with(&expected_suffix));
            xfree(p.cast::<c_void>());
        }
    }
}
