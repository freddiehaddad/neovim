//! Operating system user information (`src/nvim/os/users.c`).

#![cfg_attr(test, allow(dead_code))]

#[cfg(unix)]
use crate::fileio::getuid;
use crate::garray::{ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::math::{FAIL, OK};
#[cfg(all(not(test), windows))]
use crate::mbyte::utf16_to_utf8;
use crate::memory::{xfree, xstrdup};
#[cfg(all(not(test), windows))]
use crate::message::semsg;
#[cfg(all(not(test), unix))]
use crate::os::env::os_getenv_noalloc;
use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

pub use crate::event::r#loop::uv_uid_t;

struct StaticUsers(UnsafeCell<garray_T>);
unsafe impl Sync for StaticUsers {}

static GA_USERS: StaticUsers = StaticUsers(UnsafeCell::new(garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 1,
    ga_data: std::ptr::null_mut(),
}));

static LAZY_INIT_DONE: AtomicBool = AtomicBool::new(false);

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

#[cfg(all(target_os = "windows", not(test)))]
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_get_usernames(users: *mut garray_T) -> c_int {
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

        #[cfg(not(test))]
        let user_env = os_getenv_noalloc(c"USER".as_ptr());
        #[cfg(test)]
        let user_env: *const c_char = std::ptr::null();

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

    #[cfg(all(target_os = "windows", not(test)))]
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_get_username(s: *mut c_char, len: usize) -> c_int {
    #[cfg(unix)]
    let uid = unsafe { getuid() };
    #[cfg(not(unix))]
    let uid = 0;

    unsafe { os_get_uname(uid, s, len) }
}

unsafe fn copy_cstr_truncated(dst: *mut c_char, dst_len: usize, src: &[u8]) {
    if dst.is_null() || dst_len == 0 {
        return;
    }
    let count = src.len().min(dst_len - 1);
    unsafe {
        if count > 0 {
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst.cast::<u8>(), count);
        }
        *dst.add(count) = 0;
    }
}

/// Get the username associated with `uid` (`os_get_uname`).
///
/// # Safety
/// `s` must be valid for `len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_get_uname(uid: uv_uid_t, s: *mut c_char, len: usize) -> c_int {
    #[cfg(unix)]
    unsafe {
        let pw = getpwuid(uid);
        if !pw.is_null() && !(*pw).pw_name.is_null() && *(*pw).pw_name != 0 {
            let name_bytes = CStr::from_ptr((*pw).pw_name).to_bytes();
            copy_cstr_truncated(s, len, name_bytes);
            return OK;
        }
    }

    let formatted = format!("{}", uid as c_int);
    unsafe {
        copy_cstr_truncated(s, len, formatted.as_bytes());
    }
    FAIL
}

/// Get the home directory for `name`, or null on failure (`os_get_userdir`).
///
/// # Safety
/// `name` must be null or a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_get_userdir(name: *const c_char) -> *mut c_char {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_users() {
    unsafe {
        ga_clear_strings(GA_USERS.0.get());
    }
}

unsafe fn init_users() {
    if LAZY_INIT_DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    unsafe {
        os_get_usernames(GA_USERS.0.get());
    }
}

/// Obtain username at `idx` for `ExpandGeneric()` (`get_users`).
///
/// # Safety
/// Caller must ensure single-threaded access to user completion cache.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_users(_xp: *mut crate::os::env::expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        init_users();
        let ga = &*GA_USERS.0.get();
        if idx >= 0 && idx < ga.ga_len {
            *ga.ga_data.cast::<*mut c_char>().add(idx as usize)
        } else {
            std::ptr::null_mut()
        }
    }
}

/// Check whether `name` matches a cached username (`match_user`).
///
/// # Safety
/// `name` must be a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn match_user(name: *mut c_char) -> c_int {
    unsafe {
        init_users();
        let target = CStr::from_ptr(name).to_bytes();
        let ga = &*GA_USERS.0.get();
        let items = ga.ga_data.cast::<*mut c_char>();
        let mut result = 0;

        for i in 0..ga.ga_len.max(0) as usize {
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
        }
    }
}
