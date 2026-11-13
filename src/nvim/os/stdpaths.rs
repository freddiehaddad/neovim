//! XDG standard directory paths (`src/nvim/os/stdpaths.c`).
#![forbid(unsafe_code)]

use crate::fileio::vim_gettempdir;
use crate::memory::bytes_to_cstring;
#[cfg(unix)]
use crate::os::env::os_env_exists;
use crate::os::env::{expand_env_save, os_getenv_buf, os_getenv_cstr};
#[cfg(windows)]
use crate::os::fs::os_realpath_cstr;
use crate::os::fs::{ENV_SEPCHAR, MAXPATHL, PATHSEP};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{K_PATH_CMP_LITERAL, concat_fnames_cstr, path_equal, path_is_absolute};
use std::ffi::{CStr, CString};
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XDGVarType {
    XDGNone = -1,
    XDGConfigHome = 0,
    XDGDataHome = 1,
    XDGCacheHome = 2,
    XDGStateHome = 3,
    XDGRuntimeDir = 4,
    XDGConfigDirs = 5,
    XDGDataDirs = 6,
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
pub fn get_appname(namelike: bool) -> std::borrow::Cow<'static, CStr> {
    let mut name_buff = [0u8; MAXPATHL];
    let env_val = os_getenv_buf(c"NVIM_APPNAME", &mut name_buff);
    if env_val.is_none() {
        return std::borrow::Cow::Borrowed(c"nvim");
    }
    #[cfg(windows)]
    path_to_slash(&mut name_buff);
    if namelike {
        for b in name_buff.iter_mut() {
            if *b == 0 {
                break;
            }
            if *b == b'/' || *b == b'\\' {
                *b = b'-';
            }
        }
    }
    let c = CStr::from_bytes_until_nul(&name_buff).unwrap_or(c"nvim");
    std::borrow::Cow::Owned(c.to_owned())
}

/// Return true if `$NVIM_APPNAME` is a valid name or relative path (`appname_is_valid`).
pub fn appname_is_valid() -> bool {
    let appname = get_appname(false);
    if path_is_absolute((appname).to_bytes()) {
        return false;
    }
    let bytes = appname.to_bytes();
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
        let is_dup = token_cstrs
            .iter()
            .any(|existing| path_equal(existing, &cand, K_PATH_CMP_LITERAL));
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
    Some(bytes_to_cstring(&joined))
}

/// Return owned XDG variable value for `idx` (`stdpaths_get_xdg_var`).
pub fn stdpaths_get_xdg_var(idx: XDGVarType) -> Option<CString> {
    let i = idx as usize;
    let env = XDG_ENV_VARS[i];
    let fallback = XDG_DEFAULTS[i];

    let mut env_val = os_getenv_cstr(env);

    #[cfg(target_os = "windows")]
    {
        if env_val.is_none()
            && let Some(def_env) = XDG_DEFAULTS_ENV_VARS[i]
        {
            env_val = os_getenv_cstr(def_env);
        }
        if let Some(ref ev) = env_val
            && idx == XDGVarType::XDGCacheHome
            && let Some(real_path) = os_realpath_cstr(ev, MAXPATHL)
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
    if let Some(ref mut ev) = env_val {
        crate::memory::cstring_modify(ev, |b| {
            path_to_slash(b);
        });
    }

    let mut ret: Option<CString> = if env_val.is_some() {
        env_val
    } else if let Some(fb) = fallback {
        Some(expand_env_save(fb))
    } else if idx == XDGVarType::XDGRuntimeDir {
        let tempdir = vim_gettempdir();
        let raw = tempdir.as_deref().map_or(b"/tmp/" as &[u8], CStr::to_bytes);
        let trimmed_len = if raw.len() >= 2 { raw.len() - 1 } else { 0 };
        Some(bytes_to_cstring(&raw[..trimmed_len]))
    } else {
        None
    };

    if matches!(idx, XDGVarType::XDGDataDirs | XDGVarType::XDGConfigDirs)
        && let Some(r) = ret
    {
        ret = xdg_remove_duplicate(r, ENV_SEPCHAR);
    }

    ret
}

/// Return Nvim-specific XDG directory subpath (`get_xdg_home`).
pub fn get_xdg_home(idx: XDGVarType) -> Option<CString> {
    let dir = stdpaths_get_xdg_var(idx)?;

    let appname = get_appname(false);
    let appname_bytes = appname.to_bytes();
    let mut suffix = Vec::with_capacity(appname_bytes.len() + 6);
    suffix.extend_from_slice(appname_bytes);
    #[cfg(target_os = "windows")]
    if matches!(idx, XDGVarType::XDGDataHome | XDGVarType::XDGStateHome) {
        suffix.extend_from_slice(b"-data");
    }
    let suffix_cstr = bytes_to_cstring(&suffix);

    Some(concat_fnames_cstr(&dir, &suffix_cstr, true))
}

/// Return `$XDG_CONFIG_HOME/$NVIM_APPNAME/{fname}` (`stdpaths_user_conf_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
pub fn stdpaths_user_conf_subpath(fname: &std::ffi::CStr) -> CString {
    let home = get_xdg_home(XDGVarType::XDGConfigHome).unwrap_or_default();
    concat_fnames_cstr(&home, fname, true)
}

/// Return `$XDG_STATE_HOME/$NVIM_APPNAME/{fname}` with optional trailing path separators and
/// comma escaping (`stdpaths_user_state_subpath`).
///
/// # Safety
/// `fname` must be a valid C string.
pub fn stdpaths_user_state_subpath(
    fname: &std::ffi::CStr,
    trailing_pathseps: usize,
    escape_commas: bool,
) -> CString {
    let home = get_xdg_home(XDGVarType::XDGStateHome).unwrap_or_default();
    let ret = concat_fnames_cstr(&home, fname, true);
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
    out.extend(std::iter::repeat_n(PATHSEP, trailing_pathseps));
    bytes_to_cstring(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_subpath_escapes_commas_and_appends_separators() {
        let p = stdpaths_user_state_subpath(c"a,b", 2, true);
        let s = p.as_bytes();
        let expected_suffix = [b'/', b'a', b'\\', b',', b'b', PATHSEP, PATHSEP];
        assert!(s.ends_with(&expected_suffix));
    }
}
