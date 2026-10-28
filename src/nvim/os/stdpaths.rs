//! XDG standard directory paths (`src/nvim/os/stdpaths.c`).

use crate::fileio::vim_gettempdir;
use crate::globals::*;
use crate::memory::cstr_from_bytes;
#[cfg(unix)]
use crate::os::env::os_env_exists;
use crate::os::env::{expand_env_save, os_getenv_buf, os_getenv_cstr};
#[cfg(windows)]
use crate::os::fs::os_realpath_cstr;
use crate::os::fs::{ENV_SEPCHAR, MAXPATHL, PATHSEP};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{concat_fnames_cstr, kPathCmpLiteral, path_equal, path_is_absolute};
use std::ffi::{CStr, CString, c_char};

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
pub fn get_appname(namelike: bool) -> *const c_char {
    unsafe {
        let name_buff = (&raw mut g_st().NameBuff).cast::<u8>();
        let env_val = os_getenv_buf(c"NVIM_APPNAME".as_ptr(), name_buff.cast(), MAXPATHL);
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
pub fn appname_is_valid() -> bool {
    let appname_ptr = get_appname(false);
    if path_is_absolute(appname_ptr) {
        return false;
    }
    let bytes =  { crate::memory::cstr_as_bytes(appname_ptr) };
    if matches!(bytes, b"/" | b"\\" | b"." | b"..") {
        return false;
    }
    if bytes.windows(3).any(|w| w == b"/.." || w == b"../") {
        return false;
    }
    true
}

fn xdg_remove_duplicate(ret: CString, sep: u8) -> Option<CString> {
    let input_bytes = ret.as_bytes();
    let mut token_cstrs: Vec<CString> = Vec::new();

    for part in input_bytes.split(|&b| b == sep) {
        if part.is_empty() {
            continue;
        }
        let Ok(cand) = CString::new(part) else {
            continue;
        };
        let is_dup = token_cstrs.iter().any(|existing| path_equal(existing.as_ptr(), cand.as_ptr(), kPathCmpLiteral));
        if !is_dup {
            token_cstrs.push(cand);
        }
    }

    if token_cstrs.is_empty() {
        return None;
    }

    let mut joined: Vec<u8> = Vec::new();
    for (i, tok) in token_cstrs.iter().enumerate() {
        if i > 0 {
            joined.push(sep);
        }
        joined.extend_from_slice(tok.as_bytes());
    }
    Some(cstr_from_bytes(&joined))
}

/// Return owned XDG variable value for `idx` (`stdpaths_get_xdg_var`).
pub fn stdpaths_get_xdg_var(idx: XDGVarType) -> Option<CString> {
     {
        let i = idx as usize;
        let env = XDG_ENV_VARS[i].as_ptr();
        let fallback = XDG_DEFAULTS[i];

        let mut env_val = os_getenv_cstr(env);

        #[cfg(target_os = "windows")]
        {
            if env_val.is_none()
                && let Some(def_env) = XDG_DEFAULTS_ENV_VARS[i]
            {
                env_val = os_getenv_cstr(def_env.as_ptr());
            }
            if let Some(ref ev) = env_val
                && idx == XDGVarType::kXDGCacheHome
                && let Some(real_path) = os_realpath_cstr(ev.as_ptr(), MAXPATHL)
            {
                env_val = Some(real_path);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            if env_val.is_none() && os_env_exists(env, false) {
                env_val = Some(CString::default());
            }
        }

        #[cfg(windows)]
        if let Some(ref ev) = env_val {
            path_to_slash(ev.as_ptr().cast_mut());
        }

        let mut ret: Option<CString> = if env_val.is_some() {
            env_val
        } else if let Some(fb) = fallback {
            Some(expand_env_save(fb.as_ptr().cast_mut()))
        } else if idx == XDGVarType::kXDGRuntimeDir {
            let tempdir = vim_gettempdir();
            let raw = if tempdir.is_null() {
                b"/tmp/" as &[u8]
            } else {
                crate::memory::cstr_as_bytes(tempdir)
            };
            let trimmed_len = if raw.len() >= 2 { raw.len() - 1 } else { 0 };
            Some(cstr_from_bytes(&raw[..trimmed_len]))
        } else {
            None
        };

        if matches!(idx, XDGVarType::kXDGDataDirs | XDGVarType::kXDGConfigDirs)
            && let Some(r) = ret
        {
            ret = xdg_remove_duplicate(r, ENV_SEPCHAR);
        }

        ret
    }
}

/// Return Nvim-specific XDG directory subpath (`get_xdg_home`).
pub fn get_xdg_home(idx: XDGVarType) -> Option<CString> {
    let dir = stdpaths_get_xdg_var(idx)?;

    let appname_bytes =  { crate::memory::cstr_as_bytes(get_appname(false)) };
    let mut suffix = Vec::with_capacity(appname_bytes.len() + 6);
    suffix.extend_from_slice(appname_bytes);
    #[cfg(target_os = "windows")]
    if matches!(idx, XDGVarType::kXDGDataHome | XDGVarType::kXDGStateHome) {
        suffix.extend_from_slice(b"-data");
    }
    let suffix_cstr = cstr_from_bytes(&suffix);

    Some(concat_fnames_cstr(dir.as_ptr(), suffix_cstr.as_ptr(), true))
}

/// Return `$XDG_CONFIG_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_conf_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
pub fn stdpaths_user_conf_subpath(fname: &std::ffi::CStr) -> CString {
    let home = get_xdg_home(XDGVarType::kXDGConfigHome).unwrap_or_default();
    concat_fnames_cstr(home.as_ptr(), fname.as_ptr(), true)
}

/// Return `$XDG_STATE_HOME/$NVIM_APPNAME/{fname}` with optional trailing path separators and
/// comma escaping (`stdpaths_user_state_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
pub fn stdpaths_user_state_subpath(
    fname: *const c_char,
    trailing_pathseps: usize,
    escape_commas: bool,
) -> CString {
    let home = get_xdg_home(XDGVarType::kXDGStateHome).unwrap_or_default();
    let ret = concat_fnames_cstr(home.as_ptr(), fname, true);
    let bytes = ret.as_bytes();
    let numcommas = if escape_commas {
        bytes.iter().filter(|&&b| b == b',').count()
    } else {
        0
    };

    if numcommas == 0 && trailing_pathseps == 0 {
        return ret;
    }

    let mut out = Vec::with_capacity(bytes.len() + numcommas + trailing_pathseps);
    for &b in bytes {
        if escape_commas && b == b',' {
            out.push(b'\\');
        }
        out.push(b);
    }
    for _ in 0..trailing_pathseps {
        out.push(PATHSEP as u8);
    }
    cstr_from_bytes(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_subpath_escapes_commas_and_appends_separators() {
        let p = stdpaths_user_state_subpath(c"a,b".as_ptr(), 2, true);
            let s = p.as_bytes();
            let expected_suffix = [b'/', b'a', b'\\', b',', b'b', PATHSEP as u8, PATHSEP as u8];
            assert!(s.ends_with(&expected_suffix));
    }
}
