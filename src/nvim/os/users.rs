//! Operating system user information (`src/nvim/os/users.c`).

#[cfg(unix)]
use crate::fileio::getuid;
use crate::math::FAIL;
#[cfg(unix)]
use crate::math::OK;
#[cfg(unix)]
use crate::memory::xstrlcpy;
#[cfg(unix)]
use crate::os::env::os_getenv_buf;
use crate::strings::vim_snprintf;
#[cfg(windows)]
use std::ffi::c_void;
use std::ffi::{CStr, CString, c_char, c_int};

use crate::event::r#loop::uv_uid_t;

struct OsUsersState {
    USERS: Vec<CString>,
    LAZY_INIT_DONE: bool,
}

impl OsUsersState {
    const fn new() -> Self {
        Self {
            USERS: Vec::new(),
            LAZY_INIT_DONE: false,
        }
    }
}

static USERS_STATE: crate::globals::GlobalState<OsUsersState> = crate::globals::GlobalState::new(OsUsersState::new());

#[inline]
unsafe fn users_st() -> &'static mut OsUsersState {
    unsafe { &mut *USERS_STATE.as_ptr() }
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )
))]
#[repr(C)]
struct Passwd {
    pw_name: *mut c_char,
    pw_passwd: *mut c_char,
    pw_uid: u32,
    pw_gid: u32,
    pw_change: isize,
    pw_class: *mut c_char,
    pw_gecos: *mut c_char,
    pw_dir: *mut c_char,
    pw_shell: *mut c_char,
    pw_expire: isize,
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))
))]
#[repr(C)]
struct Passwd {
    pw_name: *mut c_char,
    pw_passwd: *mut c_char,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: *mut c_char,
    pw_dir: *mut c_char,
    pw_shell: *mut c_char,
}

#[cfg(unix)]
unsafe extern "C" {
    fn setpwent();
    fn getpwent() -> *mut Passwd;
    fn endpwent();
    fn getpwnam(name: *const c_char) -> *mut Passwd;
    fn getpwuid(uid: u32) -> *mut Passwd;
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct UserInfo0 {
    usri0_name: *mut u16,
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn NetUserEnum(
        servername: *const u16,
        level: u32,
        filter: u32,
        bufptr: *mut *mut UserInfo0,
        prefmaxlen: u32,
        entriesread: *mut u32,
        totalentries: *mut u32,
        resume_handle: *mut u32,
    ) -> u32;
    fn NetApiBufferFree(buffer: *mut c_void) -> u32;
}

#[cfg(unix)]
unsafe fn add_user(users: &mut Vec<CString>, user: *const c_char) {
    if user.is_null() || unsafe { *user == 0 } {
        return;
    }
    if let Ok(cstr) = CString::new(unsafe { CStr::from_ptr(user).to_bytes() }) {
        users.push(cstr);
    }
}

/// Collect all OS usernames (`os_get_usernames`).
unsafe fn collect_usernames() -> Vec<CString> {
    let mut users = Vec::new();

    #[cfg(unix)]
    unsafe {
        setpwent();
        loop {
            let pw = getpwent();
            if pw.is_null() {
                break;
            }
            add_user(&mut users, (*pw).pw_name);
        }
        endpwent();

        let mut user_buf = [0 as c_char; 256];
        let user_env = os_getenv_buf(c"USER".as_ptr(), user_buf.as_mut_ptr(), user_buf.len());

        if !user_env.is_null() && *user_env != 0 {
            let env_bytes = CStr::from_ptr(user_env).to_bytes();
            let found = users.iter().any(|u| u.as_bytes() == env_bytes);
            if !found {
                let pw = getpwnam(user_env);
                if !pw.is_null() {
                    add_user(&mut users, (*pw).pw_name);
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    unsafe {
        let mut nusers: u32 = 0;
        let mut ntotal: u32 = 0;
        let mut uinfo: *mut UserInfo0 = std::ptr::null_mut();

        if NetUserEnum(
            std::ptr::null(),
            0,
            0,
            &mut uinfo,
            u32::MAX,
            &mut nusers,
            &mut ntotal,
            std::ptr::null_mut(),
        ) == 0
        {
            for i in 0..nusers as usize {
                let name_ptr = (*uinfo.add(i)).usri0_name;
                if !name_ptr.is_null() {
                    let mut len = 0usize;
                    while *name_ptr.add(len) != 0 {
                        len += 1;
                    }
                    if len > 0 {
                        let slice = std::slice::from_raw_parts(name_ptr, len);
                        if let Ok(s) = String::from_utf16(slice)
                            && let Ok(cstr) = CString::new(s)
                        {
                            users.push(cstr);
                        }
                    }
                }
            }
            NetApiBufferFree(uinfo.cast::<c_void>());
        }
    }

    users
}

/// Get the username that owns the current Nvim process (`os_get_username`).
///
/// # Safety
/// `s` must be valid for `len` writable bytes.
pub unsafe fn os_get_username(s: *mut c_char, len: usize) -> c_int {
    #[cfg(unix)]
    let uid = unsafe { getuid() };
    #[cfg(not(unix))]
    let uid = 0;

    unsafe { os_get_uname(uid, s, len) }
}

/// Get the username associated with `uid` (`os_get_uname`).
///
/// # Safety
/// `s` must be valid for `len` writable bytes.
pub unsafe fn os_get_uname(uid: uv_uid_t, s: *mut c_char, len: usize) -> c_int {
    #[cfg(unix)]
    unsafe {
        let pw = getpwuid(uid);
        if !pw.is_null() && !(*pw).pw_name.is_null() && *(*pw).pw_name != 0 {
            xstrlcpy(s, (*pw).pw_name, len);
            return OK;
        }
    }

    unsafe {
        vim_snprintf!(s, len, c"%d".as_ptr(), uid as c_int);
    }
    FAIL
}

/// Get the home directory for `name`, or `None` on failure (`os_get_userdir`).
///
/// # Safety
/// `name` must be null or a valid C string.
#[cfg_attr(not(unix), cfg(test))]
pub unsafe fn os_get_userdir(name: *const c_char) -> Option<CString> {
    #[cfg(unix)]
    unsafe {
        if name.is_null() || *name == 0 {
            return None;
        }
        let pw = getpwnam(name);
        if !pw.is_null() && !(*pw).pw_dir.is_null() {
            return Some(CStr::from_ptr((*pw).pw_dir).to_owned());
        }
    }
    #[cfg(not(unix))]
    let _ = name;

    None
}

/// Free cached usernames (`free_users`).
///
/// # Safety
/// Must only be called during exit cleanup.
pub unsafe fn free_users() {
    unsafe {
        users_st().USERS = Vec::new();
    }
}

unsafe fn init_users() {
    unsafe {
        if users_st().LAZY_INIT_DONE {
            return;
        }
        users_st().LAZY_INIT_DONE = true;
        users_st().USERS = collect_usernames();
    }
}

/// Obtain username at `idx` for `ExpandGeneric()` (`get_users`).
///
/// # Safety
/// Caller must ensure single-threaded access to user completion cache.
pub unsafe fn get_users(_xp: *mut crate::os::env::expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 {
        return std::ptr::null_mut();
    }
    unsafe {
        init_users();
        let users_ptr = &raw const users_st().USERS;
        let users = &*users_ptr;
        users
            .get(idx as usize)
            .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

/// Check whether `name` matches a cached username (`match_user`).
///
/// # Safety
/// `name` must be a valid C string.
pub unsafe fn match_user(name: *mut c_char) -> c_int {
    unsafe {
        init_users();
        let target = CStr::from_ptr(name).to_bytes();
        let users_ptr = &raw const users_st().USERS;
        let users = &*users_ptr;
        let mut result = 0;

        for entry_cstr in users {
            let entry = entry_cstr.as_bytes();
            if entry == target {
                return 2;
            }
            if entry.starts_with(target) {
                result = 1;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_usernames_null_returns_fail_and_uname_formats_fallback() {
        unsafe {
            assert!(os_get_userdir(std::ptr::null()).is_none());

            let mut buf = [0 as c_char; 32];
            let res = os_get_uname(250 as uv_uid_t, buf.as_mut_ptr(), buf.len());
            if res == FAIL {
                assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "250");
            }

            assert!(os_get_userdir(c"neovim_user_not_found_test".as_ptr()).is_none());

            let users = collect_usernames();
            assert!(users.iter().all(|u| !u.as_bytes().is_empty()));
        }
    }
}
