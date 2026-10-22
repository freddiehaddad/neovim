//! Operating system user information (`src/nvim/os/users.c`).

#[cfg(unix)]
use crate::fileio::getuid;
use crate::garray::{ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::math::{FAIL, OK};
#[cfg(windows)]
use crate::mbyte::utf16_to_utf8;
#[cfg(unix)]
use crate::memory::xstrlcpy;
use crate::memory::{xfree, xstrdup};
#[cfg(windows)]
use crate::message::semsg;
#[cfg(unix)]
use crate::os::env::os_getenv_buf;
use crate::strings::vim_snprintf;
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::event::r#loop::uv_uid_t;

static mut GA_USERS: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 1,
    ga_data: std::ptr::null_mut(),
};

static mut LAZY_INIT_DONE: bool = false;

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

unsafe fn add_user(users: *mut garray_T, user: *mut c_char, need_copy: bool) {
    let user_copy = if !user.is_null() && need_copy {
        unsafe { xstrdup(user) }
    } else {
        user
    };

    if user_copy.is_null() || unsafe { *user_copy == 0 } {
        if need_copy {
            unsafe {
                xfree(user_copy.cast::<c_void>());
            }
        }
        return;
    }

    unsafe {
        ga_grow(users, 1);
        let gap = &mut *users;
        let slot = gap.ga_data.cast::<*mut c_char>().add(gap.ga_len as usize);
        *slot = user_copy;
        gap.ga_len += 1;
    }
}

/// Initialize `users` garray and fill it with OS usernames (`os_get_usernames`).
///
/// # Safety
/// `users` must be null or a valid writable pointer to a `garray_T`.
pub unsafe fn os_get_usernames(users: *mut garray_T) -> c_int {
    if users.is_null() {
        return FAIL;
    }
    unsafe {
        ga_init(users, std::mem::size_of::<*mut c_char>() as c_int, 20);
    }

    #[cfg(unix)]
    unsafe {
        setpwent();
        loop {
            let pw = getpwent();
            if pw.is_null() {
                break;
            }
            add_user(users, (*pw).pw_name, true);
        }
        endpwent();

        let mut user_buf = [0 as c_char; 256];
        let user_env = os_getenv_buf(c"USER".as_ptr(), user_buf.as_mut_ptr(), user_buf.len());

        if !user_env.is_null() && *user_env != 0 {
            let gap = &*users;
            let items = gap.ga_data.cast::<*mut c_char>();
            let mut found = false;
            let env_bytes = CStr::from_ptr(user_env).to_bytes();
            for i in 0..gap.ga_len.max(0) as usize {
                let local_user = *items.add(i);
                if CStr::from_ptr(local_user).to_bytes() == env_bytes {
                    found = true;
                    break;
                }
            }
            if !found {
                let pw = getpwnam(user_env);
                if !pw.is_null() {
                    add_user(users, (*pw).pw_name, true);
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
                let mut user: *mut c_char = std::ptr::null_mut();
                let res = utf16_to_utf8((*uinfo.add(i)).usri0_name, -1, &mut user);
                if res != 0 {
                    semsg!(c"utf16_to_utf8 failed: %d".as_ptr(), res);
                    break;
                }
                add_user(users, user, false);
            }
            NetApiBufferFree(uinfo.cast::<c_void>());
        }
    }

    OK
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

/// Get the home directory for `name`, or null on failure (`os_get_userdir`).
///
/// # Safety
/// `name` must be null or a valid C string.
#[cfg_attr(not(unix), cfg(test))]
pub unsafe fn os_get_userdir(name: *const c_char) -> *mut c_char {
    #[cfg(unix)]
    unsafe {
        if name.is_null() || *name == 0 {
            return std::ptr::null_mut();
        }
        let pw = getpwnam(name);
        if !pw.is_null() && !(*pw).pw_dir.is_null() {
            return xstrdup((*pw).pw_dir);
        }
    }
    #[cfg(not(unix))]
    let _ = name;

    std::ptr::null_mut()
}

/// Free cached usernames (`free_users`).
///
/// # Safety
/// Must only be called during exit cleanup.
pub unsafe fn free_users() {
    unsafe {
        ga_clear_strings(&raw mut GA_USERS);
    }
}

unsafe fn init_users() {
    unsafe {
        if LAZY_INIT_DONE {
            return;
        }
        LAZY_INIT_DONE = true;
        os_get_usernames(&raw mut GA_USERS);
    }
}

/// Obtain username at `idx` for `ExpandGeneric()` (`get_users`).
///
/// # Safety
/// Caller must ensure single-threaded access to user completion cache.
pub unsafe fn get_users(_xp: *mut crate::os::env::expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        init_users();
        if idx >= 0 && idx < GA_USERS.ga_len {
            *GA_USERS.ga_data.cast::<*mut c_char>().add(idx as usize)
        } else {
            std::ptr::null_mut()
        }
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
        let items = GA_USERS.ga_data.cast::<*mut c_char>();
        let mut result = 0;

        for i in 0..GA_USERS.ga_len.max(0) as usize {
            let entry = CStr::from_ptr(*items.add(i)).to_bytes();
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
            assert_eq!(os_get_usernames(std::ptr::null_mut()), FAIL);
            assert!(os_get_userdir(std::ptr::null()).is_null());

            let mut buf = [0 as c_char; 32];
            let res = os_get_uname(250 as uv_uid_t, buf.as_mut_ptr(), buf.len());
            if res == FAIL {
                assert_eq!(CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "250");
            }

            assert!(os_get_userdir(c"neovim_user_not_found_test".as_ptr()).is_null());

            let mut ga = garray_T {
                ga_len: 0,
                ga_maxlen: 0,
                ga_itemsize: 0,
                ga_growsize: 0,
                ga_data: std::ptr::null_mut(),
            };
            assert_eq!(os_get_usernames(&mut ga), OK);
            ga_clear_strings(&mut ga);
        }
    }
}
