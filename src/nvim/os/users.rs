//! Operating system user information (`src/nvim/os/users.c`).

#[cfg(unix)]
use crate::fileio::getuid;
#[cfg(unix)]
use crate::os::env::os_getenv_buf;
use crate::strings::format_to_buf;
#[cfg(windows)]
use std::ffi::c_void;
use std::ffi::{CStr, CString};

use crate::event::r#loop::uv_uid_t;

struct OsUsersState {
    users: Vec<CString>,
    lazy_init_done: bool,
}

impl OsUsersState {
    const fn new() -> Self {
        Self {
            users: Vec::new(),
            lazy_init_done: false,
        }
    }
}

std::thread_local! {
    static USERS_STATE: std::cell::RefCell<OsUsersState> =
        const { std::cell::RefCell::new(OsUsersState::new()) };
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
    pw_name: *mut u8,
    pw_passwd: *mut u8,
    pw_uid: u32,
    pw_gid: u32,
    pw_change: isize,
    pw_class: *mut u8,
    pw_gecos: *mut u8,
    pw_dir: *mut u8,
    pw_shell: *mut u8,
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
    pw_name: *mut u8,
    pw_passwd: *mut u8,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: *mut u8,
    pw_dir: *mut u8,
    pw_shell: *mut u8,
}

#[cfg(unix)]
unsafe extern "C" {
    fn setpwent();
    fn getpwent() -> *mut Passwd;
    fn endpwent();
    fn getpwnam(name: *const u8) -> *mut Passwd;
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
        entriesread: &mut u32,
        totalentries: &mut u32,
        resume_handle: *mut u32,
    ) -> u32;
    fn NetApiBufferFree(buffer: *mut c_void) -> u32;
}

#[cfg(unix)]
fn add_user(users: &mut Vec<CString>, user: Option<&std::ffi::CStr>) {
    let Some(user) = user else {
        return;
    };
    if user.to_bytes().is_empty() {
        return;
    }
    users.push(user.to_owned());
}

/// Collect all OS usernames (`os_get_usernames`).
fn collect_usernames() -> Vec<CString> {
    let mut users = Vec::new();

    #[cfg(unix)]
    unsafe {
        setpwent();
        loop {
            let pw = getpwent();
            if pw.is_null() {
                break;
            }
            let pw = &mut *pw;
            add_user(
                &mut users,
                (!pw.pw_name.is_null()).then(|| CStr::from_ptr(pw.pw_name.cast())),
            );
        }
        endpwent();

        let mut user_buf = [0u8; 256];
        if let Some(user_env) = os_getenv_buf(c"USER", &mut user_buf)
            && !user_env.to_bytes().is_empty()
        {
            let env_bytes = user_env.to_bytes();
            let found = users.iter().any(|u| u.as_bytes() == env_bytes);
            if !found {
                let pw = getpwnam(user_env.as_ptr().cast());
                if !pw.is_null() {
                    add_user(
                        &mut users,
                        (!(*pw).pw_name.is_null()).then(|| CStr::from_ptr((*pw).pw_name.cast())),
                    );
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut nusers: u32 = 0;
        let mut ntotal: u32 = 0;
        let mut uinfo: *mut UserInfo0 = std::ptr::null_mut();

        if unsafe {
            NetUserEnum(
                std::ptr::null(),
                0,
                0,
                &mut uinfo,
                u32::MAX,
                &mut nusers,
                &mut ntotal,
                std::ptr::null_mut(),
            )
        } == 0
        {
            for i in 0..nusers as usize {
                let name_ptr = crate::globals::GraphPtr::of(uinfo.wrapping_add(i))
                    .borrow()
                    .usri0_name;
                if !name_ptr.is_null() {
                    let mut len = 0usize;
                    while *crate::globals::GraphPtr::of(name_ptr.wrapping_add(len)).borrow() != 0 {
                        len += 1;
                    }
                    if len > 0 {
                        let slice = unsafe { std::slice::from_raw_parts(name_ptr, len) };
                        if let Ok(s) = String::from_utf16(slice)
                            && let Ok(cstr) = CString::new(s)
                        {
                            users.push(cstr);
                        }
                    }
                }
            }
            unsafe { NetApiBufferFree(uinfo.cast::<c_void>()) };
        }
    }

    users
}

/// Get the username that owns the current Nvim process (`os_get_username`).
pub fn os_get_username(s: &mut [u8]) -> bool {
    #[cfg(unix)]
    let uid = getuid();
    #[cfg(not(unix))]
    let uid = 0;

    os_get_uname(uid, s)
}

/// Get the username associated with `uid` (`os_get_uname`).
pub fn os_get_uname(uid: uv_uid_t, s: &mut [u8]) -> bool {
    #[cfg(unix)]
    unsafe {
        let pw = getpwuid(uid);
        if !pw.is_null() && !(*pw).pw_name.is_null() && *(*pw).pw_name != 0 {
            crate::memory::copy_bytes_to_c_buf(s, CStr::from_ptr((*pw).pw_name.cast()).to_bytes());
            return true;
        }
    }

    format_to_buf!(&mut *s, c"%d", uid as i32);
    false
}

/// Get the home directory for `name`, or `None` on failure (`os_get_userdir`).
#[cfg_attr(not(unix), cfg(test))]
pub fn os_get_userdir(name: Option<&CStr>) -> Option<CString> {
    #[cfg(unix)]
    unsafe {
        let name = name.filter(|s| !s.to_bytes().is_empty())?;
        let pw = getpwnam(name.as_ptr().cast());
        if !pw.is_null() && !(*pw).pw_dir.is_null() {
            return Some(CStr::from_ptr((*pw).pw_dir.cast()).to_owned());
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
pub fn free_users() {
    USERS_STATE.with_borrow_mut(|st| st.users.clear());
}

fn init_users() {
    if USERS_STATE.with_borrow(|st| st.lazy_init_done) {
        return;
    }
    let collected = collect_usernames();
    USERS_STATE.with_borrow_mut(|st| {
        st.lazy_init_done = true;
        st.users = collected;
    });
}

/// Obtain username at `idx` for `ExpandGeneric()` (`get_users`).
///
/// # Safety
/// Caller must ensure single-threaded access to user completion cache.
pub fn get_users(xp: &mut crate::os::env::ExpandCtx, idx: i32) -> Option<&CStr> {
    let i = usize::try_from(idx).ok()?;
    init_users();
    let user = USERS_STATE.with_borrow(|st| st.users.get(i).cloned())?;
    Some(xp.set_item(user))
}

/// Check whether `name` matches a cached username (`match_user`).
pub fn match_user(target: &[u8]) -> i32 {
    init_users();
    USERS_STATE.with_borrow(|st| {
        let mut result = 0;

        for entry_cstr in &st.users {
            let entry = entry_cstr.as_bytes();
            if entry == target {
                return 2;
            }
            if entry.starts_with(target) {
                result = 1;
            }
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_usernames_null_returns_fail_and_uname_formats_fallback() {
        assert!(os_get_userdir(None).is_none());

        let mut buf = [0u8; 32];
        let res = os_get_uname(250 as uv_uid_t, &mut buf);
        if !res {
            assert_eq!(
                std::ffi::CStr::from_bytes_until_nul(&buf)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "250"
            );
        }

        assert!(os_get_userdir(Some(c"neovim_user_not_found_test")).is_none());

        let users = collect_usernames();
        assert!(users.iter().all(|u| !u.as_bytes().is_empty()));
    }
}
