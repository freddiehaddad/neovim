//! Filesystem access and libuv file operations (`src/nvim/os/fs.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::missing_safety_doc,
    clippy::upper_case_acronyms
)]

use crate::errors::*;
use crate::globals::*;
use crate::log::logmsg;
#[allow(unused_imports)]
use crate::mbyte::mb_strnicmp;
#[allow(unused_imports)]
use crate::message::{emsg, verbose_enter, verbose_leave};
use crate::message::{semsg, smsg};
#[allow(unused_imports)]
use crate::option::copy_option_part;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::path::path_next_component;
#[allow(unused_imports)]
use crate::path::{
    FullName_save, append_path, dir_of_file_exists, get_past_head, gettail_dir, path_skip_sep,
    path_tail, path_tail_with_sep, path_with_url, save_abs_path, vim_ispathsep,
    vim_ispathsep_nocolon,
};
#[allow(unused_imports)]
use crate::ui::ui_call_chdir;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::eval::fs::{FileInfo, uv_stat_t};
use crate::file_search::FileID;
use crate::map_glyph_cache::NvimString;
use crate::memory::{
    memcnt, xfree, xmalloc, xmallocz, xmemcpyz, xmemdupz, xstrchrnul, xstrdup, xstrlcpy,
};
use crate::os::env::{os_env_exists, os_getenv, os_getenv_noalloc};
#[cfg(windows)]
use crate::os::os_win_console::os_redirect_stdin_to_conin;

#[repr(C)]
pub struct Directory {
    pub _opaque: [u64; 88],
}
pub type vim_acl_T = *mut c_void;
pub type uv_fs_t = [u64; 80];

#[cfg(windows)]
pub type uv_uid_t = u8;
#[cfg(not(windows))]
pub type uv_uid_t = u32;

#[cfg(windows)]
pub type uv_gid_t = u8;
#[cfg(not(windows))]
pub type uv_gid_t = u32;

const NUL: c_char = 0;
const OK: c_int = 1;
const FAIL: c_int = 0;
const MAXPATHL: usize = 4096;
const STDIN_FILENO: c_int = 0;

const NODE_NORMAL: c_int = 0;
const NODE_WRITABLE: c_int = 1;
const NODE_OTHER: c_int = 2;

const kPathGeneric: c_int = 1;
const kPathDrive: c_int = 2;
const kPathUNC: c_int = 3;
const kPathDevice: c_int = 4;
const kPathDeviceUNC: c_int = 5;

const UV_FS: c_int = 6;
const UV_TTY: c_int = 14;
const UV_FILE: c_int = 17;

const UV_EOF: c_int = -4095;
const UV_UNKNOWN: isize = -4094;

#[cfg(windows)]
const UV_EAGAIN: c_int = -4088;
#[cfg(windows)]
const UV_EINTR: c_int = -4072;
#[cfg(windows)]
const UV_EINVAL: c_int = -4071;

#[cfg(target_os = "linux")]
const UV_EAGAIN: c_int = -11;
#[cfg(all(not(windows), not(target_os = "linux")))]
const UV_EAGAIN: c_int = -35;
#[cfg(not(windows))]
const UV_EINTR: c_int = -4;
#[cfg(not(windows))]
const UV_EINVAL: c_int = -22;

const S_IFMT: u64 = 0o170000;
const S_IFDIR: u64 = 0o040000;
const S_IFREG: u64 = 0o100000;
const S_IFBLK: u64 = 0o060000;
const S_IFLNK: u64 = 0o120000;

const R_OK: c_int = 4;
const W_OK: c_int = 2;
const X_OK: c_int = 1;

const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;
const O_RDWR: c_int = 2;

#[cfg(windows)]
const O_APPEND: c_int = 0x0008;
#[cfg(windows)]
const O_CREAT: c_int = 0x0100;
#[cfg(windows)]
const O_TRUNC: c_int = 0x0200;
#[cfg(windows)]
const O_BINARY: c_int = 0x8000;
#[cfg(windows)]
const ENV_SEPCHAR: u8 = b';';
#[cfg(windows)]
const ENV_SEPSTR: &CStr = c";";
#[cfg(windows)]
const PATHSEP: c_char = b'\\' as c_char;
#[cfg(windows)]
const TEMP_FILE_PATH_MAXLEN: usize = 260;

#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "linux")]
const O_TRUNC: c_int = 0o1000;
#[cfg(target_os = "linux")]
const O_APPEND: c_int = 0o2000;

#[cfg(not(any(windows, target_os = "linux")))]
const O_APPEND: c_int = 0x0008;
#[cfg(not(any(windows, target_os = "linux")))]
const O_CREAT: c_int = 0x0200;
#[cfg(not(any(windows, target_os = "linux")))]
const O_TRUNC: c_int = 0x0400;

#[cfg(not(windows))]
const ENV_SEPCHAR: u8 = b':';
#[cfg(not(windows))]
const PATHSEP: c_char = b'/' as c_char;
#[cfg(not(windows))]
const TEMP_FILE_PATH_MAXLEN: usize = 256;
#[cfg(not(windows))]
const F_GETFD: c_int = 1;
#[cfg(not(windows))]
const F_SETFD: c_int = 2;
#[cfg(not(windows))]
const FD_CLOEXEC: c_int = 1;

#[cfg(target_os = "linux")]
const E2BIG: c_int = 7;
#[cfg(target_os = "linux")]
const EACCES: c_int = 13;
#[cfg(target_os = "linux")]
const EPERM: c_int = 1;
#[cfg(target_os = "linux")]
const ERANGE: c_int = 34;
#[cfg(target_os = "linux")]
const ENOTSUP: c_int = 95;

#[repr(C)]
pub struct NvimStats {
    pub fsync: i64,
    pub redraw: i64,
    pub log_skip: i16,
}

#[repr(C)]
pub struct uv_dirent_t {
    pub name: *const c_char,
    pub r#type: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct iovec {
    pub iov_base: *mut c_void,
    pub iov_len: usize,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct GUID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[cfg(windows)]
static CLSID_SHELL_LINK: GUID = GUID {
    data1: 0x00021401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[cfg(windows)]
static IID_ISHELL_LINK_W: GUID = GUID {
    data1: 0x000214F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[cfg(windows)]
static IID_IPERSIST_FILE: GUID = GUID {
    data1: 0x0000010b,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[cfg(windows)]
#[repr(C)]
struct IShellLinkWVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut IShellLinkW,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(this: *mut IShellLinkW) -> u32,
    release: unsafe extern "system" fn(this: *mut IShellLinkW) -> u32,
    get_path: unsafe extern "system" fn(
        this: *mut IShellLinkW,
        psz_file: *mut u16,
        cch: c_int,
        pfd: *mut c_void,
        f_flags: u32,
    ) -> i32,
}

#[cfg(windows)]
#[repr(C)]
struct IShellLinkW {
    lp_vtbl: *const IShellLinkWVtbl,
}

#[cfg(windows)]
#[repr(C)]
struct IPersistFileVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut IPersistFile,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(this: *mut IPersistFile) -> u32,
    release: unsafe extern "system" fn(this: *mut IPersistFile) -> u32,
    get_class_id: *const c_void,
    is_dirty: *const c_void,
    load: unsafe extern "system" fn(
        this: *mut IPersistFile,
        psz_file_name: *const u16,
        dw_mode: u32,
    ) -> i32,
}

#[cfg(windows)]
#[repr(C)]
struct IPersistFile {
    lp_vtbl: *const IPersistFileVtbl,
}

#[cfg(not(test))]
unsafe extern "C" {

    fn gettext(msgid: *const c_char) -> *mut c_char;

    fn uv_req_size(r_type: c_int) -> usize;
    fn uv_chdir(dir: *const c_char) -> c_int;
    fn uv_cwd(buffer: *mut c_char, size: *mut usize) -> c_int;
    fn uv_exepath(buffer: *mut c_char, size: *mut usize) -> c_int;
    fn uv_guess_handle(file: c_int) -> c_int;
    fn uv_strerror(err: c_int) -> *const c_char;
    fn uv_translate_sys_error(sys_errno: c_int) -> c_int;

    fn uv_fs_req_cleanup(req: *mut uv_fs_t);
    fn uv_fs_get_ptr(req: *const uv_fs_t) -> *mut c_void;
    fn uv_fs_get_path(req: *const uv_fs_t) -> *const c_char;
    fn uv_fs_get_statbuf(req: *mut uv_fs_t) -> *mut uv_stat_t;

    fn uv_fs_stat(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_fstat(loop_: *mut c_void, req: *mut uv_fs_t, file: c_int, cb: *const c_void) -> c_int;
    fn uv_fs_lstat(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_access(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_open(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        flags: c_int,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_close(loop_: *mut c_void, req: *mut uv_fs_t, file: c_int, cb: *const c_void) -> c_int;
    fn uv_fs_copyfile(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        new_path: *const c_char,
        flags: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_fsync(loop_: *mut c_void, req: *mut uv_fs_t, file: c_int, cb: *const c_void) -> c_int;
    fn uv_fs_chmod(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_fchmod(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        file: c_int,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_chown(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_fchown(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        file: c_int,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_utime(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        atime: f64,
        mtime: f64,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_rename(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        new_path: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_mkdir(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_mkdtemp(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        tpl: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_rmdir(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_scandir(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        flags: c_int,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_scandir_next(req: *mut uv_fs_t, ent: *mut uv_dirent_t) -> c_int;
    fn uv_fs_unlink(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        cb: *const c_void,
    ) -> c_int;
    fn uv_fs_realpath(
        loop_: *mut c_void,
        req: *mut uv_fs_t,
        path: *const c_char,
        cb: *const c_void,
    ) -> c_int;

    #[cfg(windows)]
    fn path_to_slash(path: *mut c_char) -> *mut c_char;
    #[cfg(windows)]
    fn utf8_to_utf16(utf8: *const c_char, utf8len: c_int, utf16: *mut *mut u16) -> c_int;
    #[cfg(windows)]
    fn utf16_to_utf8(utf16: *const u16, utf16len: c_int, utf8: *mut *mut c_char) -> c_int;

    #[cfg(windows)]
    fn _fdopen(fd: c_int, mode: *const c_char) -> *mut c_void;
    #[cfg(windows)]
    fn _close(fd: c_int) -> c_int;
    #[cfg(windows)]
    fn _dup(fd: c_int) -> c_int;
    #[cfg(windows)]
    fn _read(fd: c_int, buf: *mut c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    fn _write(fd: c_int, buf: *const c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    fn _get_osfhandle(fd: c_int) -> isize;
    #[cfg(windows)]
    fn _errno() -> *mut c_int;
    #[cfg(windows)]
    fn wcslen(s: *const u16) -> usize;
    #[cfg(windows)]
    fn wcspbrk(s: *const u16, accept: *const u16) -> *mut u16;
    #[cfg(windows)]
    fn wcsncpy(dest: *mut u16, src: *const u16, n: usize) -> *mut u16;

    #[cfg(not(windows))]
    fn fdopen(fd: c_int, mode: *const c_char) -> *mut c_void;
    #[cfg(not(windows))]
    fn close(fd: c_int) -> c_int;
    #[cfg(not(windows))]
    fn dup(oldfd: c_int) -> c_int;
    #[cfg(not(windows))]
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    fn readv(fd: c_int, iov: *const iovec, iovcnt: c_int) -> isize;
    #[cfg(not(windows))]
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    #[cfg(not(windows))]
    fn strerror(errnum: c_int) -> *const c_char;
    #[cfg(not(windows))]
    #[cfg(not(windows))]
    fn getuid() -> u32;

    #[cfg(target_os = "linux")]
    fn __errno_location() -> *mut c_int;
    #[cfg(target_os = "linux")]
    fn listxattr(path: *const c_char, list: *mut c_char, size: usize) -> isize;
    #[cfg(target_os = "linux")]
    fn getxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: usize)
    -> isize;
    #[cfg(target_os = "linux")]
    fn setxattr(
        path: *const c_char,
        name: *const c_char,
        value: *const c_void,
        size: usize,
        flags: c_int,
    ) -> c_int;

    #[cfg(all(not(windows), not(target_os = "linux")))]
    fn __error() -> *mut c_int;
}

#[cfg(all(not(test), windows))]
unsafe extern "system" {
    fn SetHandleInformation(hObject: *mut c_void, dwMask: u32, dwFlags: u32) -> c_int;
    fn CoInitialize(pvReserved: *mut c_void) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const GUID,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> i32;
    fn GetFileAttributesW(lpFileName: *const u16) -> u32;
}

#[inline]
const fn s_isdir(m: u64) -> bool {
    (m & S_IFMT) == S_IFDIR
}

#[inline]
const fn s_isreg(m: u64) -> bool {
    (m & S_IFMT) == S_IFREG
}

#[inline]
const fn s_isblk(m: u64) -> bool {
    (m & S_IFMT) == S_IFBLK
}

#[inline]
const fn s_islnk(m: u64) -> bool {
    (m & S_IFMT) == S_IFLNK
}

#[inline]
unsafe fn c_strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        unsafe {
            NvimString {
                data: s.cast_mut(),
                size: c_strlen(s),
            }
        }
    }
}

#[inline]
unsafe fn to_slash(p: *mut c_char) {
    #[cfg(all(not(test), windows))]
    unsafe {
        if !p.is_null() {
            path_to_slash(p);
        }
    }
    #[cfg(any(test, not(windows)))]
    let _ = p;
}

#[inline]
unsafe fn errno_ptr() -> *mut c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        _errno()
    }
    #[cfg(all(not(test), target_os = "linux"))]
    unsafe {
        __errno_location()
    }
    #[cfg(all(not(test), not(windows), not(target_os = "linux")))]
    unsafe {
        __error()
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

/// Changes the current directory to `path`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_chdir(path: *const c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        if p_verbose >= 5 {
            verbose_enter();
            let _ = smsg!(0, c"chdir(%s)".as_ptr(), path);
            verbose_leave();
        }
        let err = uv_chdir(path);
        if err == 0 {
            ui_call_chdir(cstr_as_string(path));
        }
        err
    }
    #[cfg(test)]
    {
        let _ = path;
        -1
    }
}

/// Get the name of current directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_dirname(buf: *mut c_char, mut len: usize) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let error_number = uv_cwd(buf, &raw mut len);
        if error_number != 0 {
            xstrlcpy(buf, uv_strerror(error_number), len);
            return FAIL;
        }
        to_slash(buf);
        OK
    }
    #[cfg(test)]
    {
        let _ = (buf, len);
        FAIL
    }
}

/// Check if the given path is a directory and not a symlink to a directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_isrealdir(name: *const c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let mut request: uv_fs_t = [0; 80];
        if uv_fs_lstat(ptr::null_mut(), &raw mut request, name, ptr::null()) != 0 {
            uv_fs_req_cleanup(&raw mut request);
            return false;
        }
        let mode = (*uv_fs_get_statbuf(&raw mut request)).st_mode;
        uv_fs_req_cleanup(&raw mut request);
        if s_islnk(mode) {
            return false;
        }
        s_isdir(mode)
    }
    #[cfg(test)]
    {
        let _ = name;
        false
    }
}

/// Check if the given path exists and is a directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_isdir(name: *const c_char) -> bool {
    unsafe {
        let mode = os_getperm(name);
        if mode < 0 {
            return false;
        }
        s_isdir(mode as u64)
    }
}

/// Check what `name` is (`NODE_NORMAL`, `NODE_WRITABLE`, or `NODE_OTHER`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_nodetype(name: *const c_char) -> c_int {
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        let mut statbuf = uv_stat_t::default();
        if os_stat(name, &raw mut statbuf) != 0 {
            return NODE_NORMAL;
        }
        if s_isreg(statbuf.st_mode) || s_isdir(statbuf.st_mode) {
            return NODE_NORMAL;
        }
        if s_isblk(statbuf.st_mode) {
            return NODE_OTHER;
        }
        NODE_WRITABLE
    }
    #[cfg(all(not(test), windows))]
    unsafe {
        let bytes = CStr::from_ptr(name).to_bytes();
        if bytes.starts_with(b"\\\\.\\") {
            return NODE_WRITABLE;
        }
        let fd = os_open(name, O_RDONLY, 0);
        if fd < 0 {
            return NODE_NORMAL;
        }
        let guess = uv_guess_handle(fd);
        let _ = _close(fd);
        match guess {
            UV_TTY => NODE_WRITABLE,
            UV_FILE => NODE_NORMAL,
            _ => NODE_OTHER,
        }
    }
    #[cfg(test)]
    {
        let _ = name;
        NODE_NORMAL
    }
}

/// Gets the absolute path of the currently running executable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_exepath(buffer: *mut c_char, size: *mut usize) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let r = uv_exepath(buffer, size);
        to_slash(buffer);
        r
    }
    #[cfg(test)]
    {
        let _ = (buffer, size);
        -1
    }
}

/// Checks if the file `name` is executable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_can_exe(
    name: *const c_char,
    abspath: *mut *mut c_char,
    use_path: bool,
) -> bool {
    #[cfg(not(test))]
    unsafe {
        if !use_path || gettail_dir(name) != name {
            #[cfg(windows)]
            return is_executable_ext(name, abspath);
            #[cfg(not(windows))]
            return (use_path || gettail_dir(name) != name) && is_executable(name, abspath);
        }
        is_executable_in_path(name, abspath)
    }
    #[cfg(test)]
    {
        let _ = (name, abspath, use_path);
        false
    }
}

unsafe fn is_executable(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    unsafe {
        let mode = os_getperm(name);
        if mode < 0 {
            return false;
        }

        #[cfg(windows)]
        let ok = s_isreg(mode as u64);

        #[cfg(not(windows))]
        let ok = if s_isreg(mode as u64) {
            #[cfg(not(test))]
            {
                let mut req: uv_fs_t = [0; 80];
                let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, X_OK, ptr::null());
                uv_fs_req_cleanup(&raw mut req);
                r == 0
            }
            #[cfg(test)]
            {
                false
            }
        } else {
            false
        };

        #[cfg(not(test))]
        if ok && !abspath.is_null() {
            *abspath = save_abs_path(name);
        }
        ok
    }
}

#[cfg(windows)]
unsafe fn is_executable_ext(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let sh_tail = CStr::from_ptr(path_tail(p_sh)).to_bytes();
        let has_sub =
            |haystack: &[u8], needle: &[u8]| haystack.windows(needle.len()).any(|w| w == needle);
        let is_unix_shell = !has_sub(sh_tail, b"powershell")
            && !has_sub(sh_tail, b"pwsh")
            && has_sub(sh_tail, b"sh");

        let name_bytes = CStr::from_ptr(name).to_bytes();
        let (nameext, nameext_len) = if let Some(pos) = name_bytes.iter().rposition(|&b| b == b'.')
        {
            (name.add(pos), name_bytes.len() - pos)
        } else {
            (ptr::null(), 0)
        };

        let buf = (&raw mut os_buf).cast::<c_char>();
        xstrlcpy(buf, name, MAXPATHL);
        let buf_end = xstrchrnul(buf, NUL);
        let mut pathext: *const c_char = os_getenv_noalloc(c"PATHEXT".as_ptr());
        if pathext.is_null() {
            pathext = c".com;.exe;.bat;.cmd".as_ptr();
        }
        let mut ext = pathext;
        while *ext != 0 {
            if *ext == b'.' as c_char && (*ext.add(1) == NUL || (*ext.add(1) as u8) == ENV_SEPCHAR)
            {
                if is_executable(name, abspath) {
                    return true;
                }
                ext = ext.add(1);
                if *ext != 0 {
                    ext = ext.add(1);
                }
                continue;
            }

            let mut ext_end = ext.cast_mut();
            let rem = MAXPATHL - ((buf_end as usize) - (buf as usize));
            let ext_len = copy_option_part(&raw mut ext_end, buf_end, rem, ENV_SEPSTR.as_ptr());
            if ext_len != 0 {
                let in_pathext = nameext_len == ext_len && mb_strnicmp(nameext, ext, ext_len) == 0;
                if ((in_pathext || is_unix_shell) && is_executable(name, abspath))
                    || is_executable(buf, abspath)
                {
                    return true;
                }
            }
            ext = ext_end;
        }
        false
    }
    #[cfg(test)]
    {
        let _ = (name, abspath);
        false
    }
}

#[cfg(not(windows))]
#[inline]
unsafe fn is_executable_ext(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    unsafe { is_executable(name, abspath) }
}

unsafe fn is_executable_in_path(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let path_env = os_getenv(c"PATH".as_ptr());
        if path_env.is_null() {
            return false;
        }

        #[cfg(windows)]
        let path = {
            let sh_tail = CStr::from_ptr(path_tail(p_sh)).to_bytes();
            if !os_env_exists(c"NoDefaultCurrentDirectoryInExePath".as_ptr(), false)
                && sh_tail.windows(7).any(|w| w == b"cmd.exe")
            {
                let pathlen = c_strlen(path_env);
                let p = xmallocz(pathlen + 2).cast::<c_char>();
                ptr::copy_nonoverlapping(b".;".as_ptr().cast::<c_char>(), p, 2);
                ptr::copy_nonoverlapping(path_env, p.add(2), pathlen);
                p
            } else {
                xstrdup(path_env)
            }
        };
        #[cfg(not(windows))]
        let path = xstrdup(path_env);

        let bufsize = c_strlen(name) + c_strlen(path) + 2;
        let buf = xmalloc(bufsize).cast::<c_char>();

        let mut p = path;
        let mut rv = false;
        loop {
            let e = xstrchrnul(p, ENV_SEPCHAR as c_char);
            let _ = xmemcpyz(buf.cast(), p.cast(), (e as usize) - (p as usize));
            let _ = append_path(buf, name, bufsize);

            if is_executable_ext(buf, abspath) {
                rv = true;
                break;
            }
            if (*e as u8) != ENV_SEPCHAR {
                break;
            }
            p = e.add(1);
        }

        xfree(buf.cast());
        xfree(path.cast());
        xfree(path_env.cast());
        rv
    }
    #[cfg(test)]
    {
        let _ = (name, abspath);
        false
    }
}

/// Opens or creates a file and returns a file descriptor, or negative libuv error code.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    if path.is_null() {
        return UV_EINVAL;
    }
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_open(
            ptr::null_mut(),
            &raw mut req,
            path,
            flags,
            mode,
            ptr::null(),
        );
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (flags, mode);
        UV_EINVAL
    }
}

/// Compatibility wrapper conforming to `fopen(3)`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fopen(path: *const c_char, flags: *const c_char) -> *mut c_void {
    unsafe {
        assert!(!flags.is_null());
        let flen = c_strlen(flags);
        assert!((1..=2).contains(&flen));

        let mut iflags: c_int;
        let f0 = *flags as u8;
        let f1 = *flags.add(1) as u8;
        if f1 == 0 || f1 == b'b' {
            iflags = match f0 {
                b'r' => O_RDONLY,
                b'w' => O_WRONLY | O_CREAT | O_TRUNC,
                b'a' => O_WRONLY | O_CREAT | O_APPEND,
                _ => std::process::abort(),
            };
            #[cfg(windows)]
            if f1 == b'b' {
                iflags |= O_BINARY;
            }
        } else {
            assert_eq!(f1, b'+');
            iflags = match f0 {
                b'r' => O_RDWR,
                b'w' => O_RDWR | O_CREAT | O_TRUNC,
                b'a' => O_RDWR | O_CREAT | O_APPEND,
                _ => std::process::abort(),
            };
        }

        let fd = os_open(path, iflags, 0o666);
        if fd < 0 {
            return ptr::null_mut();
        }
        #[cfg(all(not(test), windows))]
        {
            _fdopen(fd, flags)
        }
        #[cfg(all(not(test), not(windows)))]
        {
            fdopen(fd, flags)
        }
        #[cfg(test)]
        {
            ptr::null_mut()
        }
    }
}

/// Sets file descriptor `fd` to close-on-exec (Unix) or non-inheritable (Windows).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_set_cloexec(fd: c_int) -> c_int {
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        let fdflags = fcntl(fd, F_GETFD);
        if fdflags < 0 {
            let e = *errno_ptr();
            let _ = logmsg!(
                4,
                ptr::null(),
                c"os_set_cloexec".as_ptr(),
                line!() as c_int,
                true,
                c"Failed to get flags on descriptor %d: %s".as_ptr(),
                fd,
                strerror(e),
            );
            *errno_ptr() = e;
            return -1;
        }
        if (fdflags & FD_CLOEXEC) == 0 && fcntl(fd, F_SETFD, fdflags | FD_CLOEXEC) == -1 {
            let e = *errno_ptr();
            let _ = logmsg!(
                4,
                ptr::null(),
                c"os_set_cloexec".as_ptr(),
                line!() as c_int,
                true,
                c"Failed to set CLOEXEC on descriptor %d: %s".as_ptr(),
                fd,
                strerror(e),
            );
            *errno_ptr() = e;
            return -1;
        }
        0
    }
    #[cfg(all(not(test), windows))]
    unsafe {
        let h = _get_osfhandle(fd);
        if h == -1 || SetHandleInformation(h as *mut c_void, 1, 0) == 0 {
            return -1;
        }
        0
    }
    #[cfg(test)]
    {
        let _ = fd;
        0
    }
}

/// Close a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_close(fd: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_close(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = fd;
        0
    }
}

/// Duplicate file descriptor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_dup(fd: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        loop {
            #[cfg(windows)]
            let ret = _dup(fd);
            #[cfg(not(windows))]
            let ret = dup(fd);

            if ret < 0 {
                let ep = errno_ptr();
                let error = uv_translate_sys_error(*ep);
                *ep = 0;
                if error == UV_EINTR {
                    continue;
                }
                return error;
            }
            return ret;
        }
    }
    #[cfg(test)]
    {
        let _ = fd;
        -1
    }
}

/// Duplicates file descriptor `fd` and marks the copy close-on-exec.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_dup_cloexec(fd: c_int) -> c_int {
    unsafe {
        let newfd = os_dup(fd);
        if newfd >= 0 {
            let _ = os_set_cloexec(newfd);
        }
        newfd
    }
}

/// Open the file descriptor for stdin.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_open_stdin_fd() -> c_int {
    #[cfg(not(test))]
    unsafe {
        if stdin_fd > 0 {
            stdin_fd
        } else {
            let fd = os_dup(STDIN_FILENO);
            #[cfg(windows)]
            os_redirect_stdin_to_conin();
            fd
        }
    }
    #[cfg(test)]
    {
        -1
    }
}

/// Read from a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_read(
    fd: c_int,
    ret_eof: *mut bool,
    ret_buf: *mut c_char,
    size: usize,
    non_blocking: bool,
) -> isize {
    unsafe {
        *ret_eof = false;
        if ret_buf.is_null() {
            assert_eq!(size, 0);
            return 0;
        }
        #[cfg(not(test))]
        {
            let mut read_bytes: usize = 0;
            while read_bytes != size {
                assert!(size >= read_bytes);
                #[cfg(windows)]
                let cur_read_bytes = _read(
                    fd,
                    ret_buf.add(read_bytes).cast(),
                    (size - read_bytes) as c_uint,
                ) as isize;
                #[cfg(not(windows))]
                let cur_read_bytes = read(fd, ret_buf.add(read_bytes).cast(), size - read_bytes);

                if cur_read_bytes > 0 {
                    read_bytes += cur_read_bytes as usize;
                }
                if cur_read_bytes < 0 {
                    let ep = errno_ptr();
                    let error = uv_translate_sys_error(*ep);
                    *ep = 0;
                    if non_blocking && error == UV_EAGAIN {
                        break;
                    } else if error == UV_EINTR || error == UV_EAGAIN {
                        continue;
                    } else {
                        return error as isize;
                    }
                }
                if cur_read_bytes == 0 {
                    *ret_eof = true;
                    break;
                }
            }
            read_bytes as isize
        }
        #[cfg(test)]
        {
            let _ = (fd, non_blocking);
            0
        }
    }
}

/// Read from a file to multiple buffers at once.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_readv(
    fd: c_int,
    ret_eof: *mut bool,
    mut iov: *mut iovec,
    mut iov_size: usize,
    non_blocking: bool,
) -> isize {
    unsafe {
        *ret_eof = false;
        #[cfg(all(not(test), not(windows)))]
        {
            let mut read_bytes: usize = 0;
            let mut toread: usize = 0;
            for i in 0..iov_size {
                let len = (*iov.add(i)).iov_len;
                assert!(toread <= usize::MAX - len);
                toread += len;
            }
            while read_bytes < toread && iov_size != 0 && !*ret_eof {
                let mut cur_read_bytes = readv(fd, iov, iov_size as c_int);
                if cur_read_bytes == 0 {
                    *ret_eof = true;
                }
                if cur_read_bytes > 0 {
                    read_bytes += cur_read_bytes as usize;
                    while iov_size != 0 && cur_read_bytes != 0 {
                        if (cur_read_bytes as usize) < (*iov).iov_len {
                            (*iov).iov_len -= cur_read_bytes as usize;
                            (*iov).iov_base = (*iov)
                                .iov_base
                                .cast::<u8>()
                                .add(cur_read_bytes as usize)
                                .cast();
                            cur_read_bytes = 0;
                        } else {
                            cur_read_bytes -= (*iov).iov_len as isize;
                            iov_size -= 1;
                            iov = iov.add(1);
                        }
                    }
                } else if cur_read_bytes < 0 {
                    let ep = errno_ptr();
                    let error = uv_translate_sys_error(*ep);
                    *ep = 0;
                    if non_blocking && error == UV_EAGAIN {
                        break;
                    } else if error == UV_EINTR || error == UV_EAGAIN {
                        continue;
                    } else {
                        return error as isize;
                    }
                }
            }
            read_bytes as isize
        }
        #[cfg(any(test, windows))]
        {
            let _ = (fd, &mut iov, &mut iov_size, non_blocking);
            0
        }
    }
}

/// Write to a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_write(
    fd: c_int,
    buf: *const c_char,
    size: usize,
    non_blocking: bool,
) -> isize {
    if buf.is_null() {
        assert_eq!(size, 0);
        return 0;
    }
    #[cfg(not(test))]
    unsafe {
        let mut written_bytes: usize = 0;
        while written_bytes != size {
            assert!(size >= written_bytes);
            #[cfg(windows)]
            let cur_written_bytes = _write(
                fd,
                buf.add(written_bytes).cast(),
                (size - written_bytes) as c_uint,
            ) as isize;
            #[cfg(not(windows))]
            let cur_written_bytes = write(fd, buf.add(written_bytes).cast(), size - written_bytes);

            if cur_written_bytes > 0 {
                written_bytes += cur_written_bytes as usize;
            }
            if cur_written_bytes < 0 {
                let ep = errno_ptr();
                let error = uv_translate_sys_error(*ep);
                *ep = 0;
                if non_blocking && error == UV_EAGAIN {
                    break;
                } else if error == UV_EINTR || error == UV_EAGAIN {
                    continue;
                } else {
                    return error as isize;
                }
            }
            if cur_written_bytes == 0 {
                return UV_UNKNOWN;
            }
        }
        written_bytes as isize
    }
    #[cfg(test)]
    {
        let _ = (fd, non_blocking);
        0
    }
}

/// Copies a file from `path` to `new_path`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_copy(
    path: *const c_char,
    new_path: *const c_char,
    flags: c_int,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_copyfile(
            ptr::null_mut(),
            &raw mut req,
            path,
            new_path,
            flags,
            ptr::null(),
        );
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (path, new_path, flags);
        -1
    }
}

/// Flushes file modifications to disk.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fsync(fd: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_fsync(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        g_stats.fsync += 1;
        r
    }
    #[cfg(test)]
    {
        let _ = fd;
        0
    }
}

unsafe fn os_stat(name: *const c_char, statbuf: *mut uv_stat_t) -> c_int {
    if name.is_null() {
        return UV_EINVAL;
    }
    #[cfg(not(test))]
    unsafe {
        let mut request: uv_fs_t = [0; 80];
        let result = uv_fs_stat(ptr::null_mut(), &raw mut request, name, ptr::null());
        if result == 0 {
            *statbuf = *uv_fs_get_statbuf(&raw mut request);
        }
        uv_fs_req_cleanup(&raw mut request);
        result
    }
    #[cfg(test)]
    {
        let _ = statbuf;
        UV_EINVAL
    }
}

/// Get the file permissions for a given file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_getperm(name: *const c_char) -> i32 {
    unsafe {
        let mut statbuf = uv_stat_t::default();
        let stat_result = os_stat(name, &raw mut statbuf);
        if stat_result == 0 {
            statbuf.st_mode as i32
        } else {
            stat_result
        }
    }
}

/// Set the permission of a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_setperm(name: *const c_char, perm: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_chmod(ptr::null_mut(), &raw mut req, name, perm, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        if r == 0 { OK } else { FAIL }
    }
    #[cfg(test)]
    {
        let _ = (name, perm);
        FAIL
    }
}

/// Set the permission of the file referred to by the open file descriptor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fsetperm(fd: c_int, perm: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_fchmod(ptr::null_mut(), &raw mut req, fd, perm, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        if r == 0 { OK } else { FAIL }
    }
    #[cfg(test)]
    {
        let _ = (fd, perm);
        FAIL
    }
}

/// Copy extended attributes `from_file` to `to_file`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_copy_xattr(from_file: *const c_char, to_file: *const c_char) {
    #[cfg(all(not(test), target_os = "linux"))]
    unsafe {
        if from_file.is_null() {
            return;
        }
        let mut size = listxattr(from_file, ptr::null_mut(), 0);
        if size <= 0 {
            return;
        }
        let xattr_buf = xmalloc(size as usize).cast::<c_char>();
        size = listxattr(from_file, xattr_buf, size as usize);
        let tsize = size;

        let ep = errno_ptr();
        *ep = 0;

        let mut max_vallen: isize = 0;
        let mut val: *mut c_char = ptr::null_mut();
        let mut errmsg: *const c_char = ptr::null();

        'outer: for round in 0..2 {
            let mut key = xattr_buf;
            if round == 1 {
                size = tsize;
            }

            while size > 0 {
                let vallen = getxattr(
                    from_file,
                    key,
                    val.cast(),
                    if round != 0 { max_vallen as usize } else { 0 },
                );
                if vallen >= 0
                    && round != 0
                    && setxattr(to_file, key, val.cast(), vallen as usize, 0) == 0
                {
                } else if *ep != 0 {
                    match *ep {
                        E2BIG => {
                            errmsg = c"E1508: Size of the extended attribute value is larger than the maximum size allowed".as_ptr();
                            break 'outer;
                        }
                        ENOTSUP | EACCES | EPERM => {}
                        ERANGE => {
                            errmsg = c"E1506: Buffer too small to copy xattr value or key".as_ptr();
                            break 'outer;
                        }
                        _ => {
                            errmsg =
                                c"E1509: Error occurred when reading or writing extended attribute"
                                    .as_ptr();
                            break 'outer;
                        }
                    }
                }

                if round == 0 && vallen > max_vallen {
                    max_vallen = vallen;
                }

                let keylen = (c_strlen(key) + 1) as isize;
                size -= keylen;
                key = key.add(keylen as usize);
            }
            if round != 0 {
                break;
            }
            val = xmalloc((max_vallen as usize) + 1).cast::<c_char>();
        }

        xfree(xattr_buf.cast());
        xfree(val.cast());
        if !errmsg.is_null() {
            emsg(gettext(errmsg));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn os_get_acl(_fname: *const c_char) -> vim_acl_T {
    ptr::null_mut()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn os_set_acl(_fname: *const c_char, _aclent: vim_acl_T) {}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn os_free_acl(_aclent: vim_acl_T) {}

/// Checks if the current user owns a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_file_owned(fname: *const c_char) -> bool {
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        let uid = u64::from(getuid());
        let mut finfo = FileInfo::default();
        let file_owned = os_fileinfo(fname, &raw mut finfo) && finfo.stat.st_uid == uid;
        let link_owned = os_fileinfo_link(fname, &raw mut finfo) && finfo.stat.st_uid == uid;
        file_owned && link_owned
    }
    #[cfg(any(test, windows))]
    {
        let _ = fname;
        true
    }
}

/// Changes the owner and group of a file, like `chown(2)`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_chown(path: *const c_char, owner: uv_uid_t, group: uv_gid_t) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_chown(
            ptr::null_mut(),
            &raw mut req,
            path,
            owner,
            group,
            ptr::null(),
        );
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (path, owner, group);
        -1
    }
}

/// Changes the owner and group of an open file descriptor, like `fchown(2)`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fchown(fd: c_int, owner: uv_uid_t, group: uv_gid_t) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_fchown(ptr::null_mut(), &raw mut req, fd, owner, group, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (fd, owner, group);
        -1
    }
}

/// Check if a path exists.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_path_exists(path: *const c_char) -> bool {
    unsafe {
        let mut statbuf = uv_stat_t::default();
        os_stat(path, &raw mut statbuf) == 0
    }
}

/// Sets file access and modification times.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_file_settime(path: *const c_char, atime: f64, mtime: f64) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_utime(
            ptr::null_mut(),
            &raw mut req,
            path,
            atime,
            mtime,
            ptr::null(),
        );
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (path, atime, mtime);
        -1
    }
}

/// Check if a file is readable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_file_is_readable(name: *const c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, R_OK, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r == 0
    }
    #[cfg(test)]
    {
        let _ = name;
        false
    }
}

/// Check if a file is writable (`0` = not writable, `1` = writable file, `2` = writable dir).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_file_is_writable(name: *const c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, W_OK, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        if r == 0 {
            if os_isdir(name) { 2 } else { 1 }
        } else {
            0
        }
    }
    #[cfg(test)]
    {
        let _ = name;
        0
    }
}

/// Rename a file or directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_rename(path: *const c_char, new_path: *const c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_rename(ptr::null_mut(), &raw mut req, path, new_path, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        if r == 0 { OK } else { FAIL }
    }
    #[cfg(test)]
    {
        let _ = (path, new_path);
        FAIL
    }
}

/// Make a directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_mkdir(path: *const c_char, mode: i32) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_mkdir(ptr::null_mut(), &raw mut req, path, mode, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = (path, mode);
        -1
    }
}

/// Make a directory, with higher levels when needed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_mkdir_recurse(
    dir: *const c_char,
    mode: i32,
    failed_dir: *mut *mut c_char,
    created: *mut *mut c_char,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let dirlen = c_strlen(dir);
        let curdir = xmemdupz(dir.cast(), dirlen).cast::<c_char>();
        let past_head = get_past_head(curdir);
        let mut e = curdir.add(dirlen);
        let real_end: *const c_char = e;
        let past_head_save = *past_head;

        while !os_isdir(curdir) {
            e = path_tail_with_sep(curdir);
            if (e as usize) <= (past_head as usize) {
                *past_head = NUL;
                break;
            }
            *e = NUL;
        }
        while !ptr::eq(e, real_end) {
            if (e as usize) > (past_head as usize) {
                *e = PATHSEP;
            } else {
                *past_head = past_head_save;
            }
            let component_len = c_strlen(e);
            e = e.add(component_len);
            if ptr::eq(e, real_end)
                && memcnt(e.sub(component_len).cast(), PATHSEP, component_len) == component_len
            {
                break;
            }
            let ret = os_mkdir(curdir, mode);
            if ret != 0 {
                *failed_dir = curdir;
                return ret;
            } else if !created.is_null() && (*created).is_null() {
                *created = FullName_save(curdir, false);
            }
        }
        xfree(curdir.cast());
        0
    }
    #[cfg(test)]
    {
        let _ = (dir, mode, failed_dir, created);
        -1
    }
}

/// Create the parent directory of a file if it does not exist.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_file_mkdir(fname: *mut c_char, mode: i32) -> c_int {
    #[cfg(not(test))]
    unsafe {
        if !dir_of_file_exists(fname) {
            let tail = path_tail_with_sep(fname);
            let last_char = tail.add(c_strlen(tail)).sub(1);
            if vim_ispathsep(c_int::from(*last_char as u8)) {
                emsg(gettext(e_noname.as_ptr()));
                return -1;
            }
            let c = *tail;
            *tail = NUL;
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let r = os_mkdir_recurse(fname, mode, &raw mut failed_dir, ptr::null_mut());
            if r < 0 {
                let _ = semsg!(gettext(e_mkdir.as_ptr()), failed_dir, uv_strerror(r));
                xfree(failed_dir.cast());
            }
            *tail = c;
            return r;
        }
        0
    }
    #[cfg(test)]
    {
        let _ = (fname, mode);
        0
    }
}

/// Create a unique temporary directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_mkdtemp(templ: *const c_char, path: *mut c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut request: uv_fs_t = [0; 80];
        let result = uv_fs_mkdtemp(ptr::null_mut(), &raw mut request, templ, ptr::null());
        if result == 0 {
            xstrlcpy(
                path,
                uv_fs_get_path(&raw const request),
                TEMP_FILE_PATH_MAXLEN,
            );
            to_slash(path);
        }
        uv_fs_req_cleanup(&raw mut request);
        result
    }
    #[cfg(test)]
    {
        let _ = (templ, path);
        -1
    }
}

/// Remove a directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_rmdir(path: *const c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_rmdir(ptr::null_mut(), &raw mut req, path, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = path;
        -1
    }
}

/// Opens a directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_scandir(dir: *mut Directory, path: *const c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let req = dir.cast::<uv_fs_t>();
        let r = uv_fs_scandir(ptr::null_mut(), req, path, 0, ptr::null());
        if r < 0 {
            os_closedir(dir);
        }
        r >= 0
    }
    #[cfg(test)]
    {
        let _ = (dir, path);
        false
    }
}

/// Increments the directory pointer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_scandir_next(dir: *mut Directory) -> *const c_char {
    #[cfg(not(test))]
    unsafe {
        let req = dir.cast::<uv_fs_t>();
        let ent = dir
            .cast::<u8>()
            .add(uv_req_size(UV_FS))
            .cast::<uv_dirent_t>();
        let err = uv_fs_scandir_next(req, ent);
        if err != UV_EOF {
            (*ent).name
        } else {
            ptr::null()
        }
    }
    #[cfg(test)]
    {
        let _ = dir;
        ptr::null()
    }
}

/// Frees memory associated with `os_scandir()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_closedir(dir: *mut Directory) {
    #[cfg(not(test))]
    unsafe {
        uv_fs_req_cleanup(dir.cast::<uv_fs_t>());
    }
}

/// Remove a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_remove(path: *const c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut req: uv_fs_t = [0; 80];
        let r = uv_fs_unlink(ptr::null_mut(), &raw mut req, path, ptr::null());
        uv_fs_req_cleanup(&raw mut req);
        r
    }
    #[cfg(test)]
    {
        let _ = path;
        -1
    }
}

/// Get the file information for a given path.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo(path: *const c_char, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::default();
        os_stat(path, &raw mut (*file_info).stat) == 0
    }
}

/// Parses `path` into a `FileInfo` structure.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo2(path: *const c_char, info: *mut FileInfo) -> bool {
    unsafe {
        *info = FileInfo::default();
        #[cfg(not(test))]
        {
            if path_with_url(path) != 0 {
                return true;
            }
            let mut p: *const c_char = path_skip_sep(path, false);
            let leading_slashes = (p as usize) - (path as usize);

            #[cfg(windows)]
            {
                let p0 = *p as u8;
                let p1 = *p.add(1) as u8;
                if leading_slashes == 0 && p0.is_ascii_alphabetic() && p1 == b':' {
                    (*info).r#type = kPathDrive;
                    p = path_skip_sep(p.add(2), false);
                    (*info).rest_off = (p as usize) - (path as usize);
                    return true;
                }
                if leading_slashes >= 2 && (p0 == b'?' || p0 == b'.') {
                    if !vim_ispathsep_nocolon(c_int::from(p1)) {
                        return true;
                    }
                    (*info).r#type = kPathDevice;
                    (*info).prefix_off = leading_slashes - 2;
                    p = path_skip_sep(p.add(2), false);
                    let p_bytes = [
                        (*p as u8).to_ascii_lowercase(),
                        (*p.add(1) as u8).to_ascii_lowercase(),
                        (*p.add(2) as u8).to_ascii_lowercase(),
                    ];
                    if &p_bytes == b"unc" && vim_ispathsep_nocolon(c_int::from(*p.add(3) as u8)) {
                        (*info).r#type = kPathDeviceUNC;
                        p = path_skip_sep(p.add(4), false);
                        (*info).root_off = (p as usize) - (path as usize);
                        p = path_skip_sep(path_next_component(p), false);
                        p = path_skip_sep(path_next_component(p), false);
                        (*info).rest_off = (p as usize) - (path as usize);
                        return true;
                    }
                    (*info).root_off = (p as usize) - (path as usize);
                    if (*p as u8).is_ascii_alphabetic() && (*p.add(1) as u8) == b':' {
                        p = p.add(2);
                    }
                    p = path_skip_sep(path_next_component(p), false);
                    (*info).rest_off = (p as usize) - (path as usize);
                    return true;
                }
                if leading_slashes == 2 {
                    (*info).r#type = kPathUNC;
                    p = path_skip_sep(path_next_component(p), false);
                    p = path_skip_sep(path_next_component(p), false);
                    (*info).rest_off = (p as usize) - (path as usize);
                    return true;
                }
            }

            (*info).r#type = kPathGeneric;
            (*info).rest_off = leading_slashes;
            (*info).root_off = if leading_slashes > 2 {
                leading_slashes - 1
            } else {
                0
            };
            (*info).prefix_off = (*info).root_off;
        }
        true
    }
}

/// Get the file information for a given path without following links.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_link(path: *const c_char, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::default();
        if path.is_null() {
            return false;
        }
        #[cfg(not(test))]
        {
            let mut request: uv_fs_t = [0; 80];
            let ok = uv_fs_lstat(ptr::null_mut(), &raw mut request, path, ptr::null()) == 0;
            if ok {
                (*file_info).stat = *uv_fs_get_statbuf(&raw mut request);
            }
            uv_fs_req_cleanup(&raw mut request);
            ok
        }
        #[cfg(test)]
        {
            false
        }
    }
}

/// Get the file information for a given file descriptor.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_fd(file_descriptor: c_int, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::default();
        #[cfg(not(test))]
        {
            let mut request: uv_fs_t = [0; 80];
            let ok = uv_fs_fstat(
                ptr::null_mut(),
                &raw mut request,
                file_descriptor,
                ptr::null(),
            ) == 0;
            if ok {
                (*file_info).stat = *uv_fs_get_statbuf(&raw mut request);
            }
            uv_fs_req_cleanup(&raw mut request);
            ok
        }
        #[cfg(test)]
        {
            let _ = file_descriptor;
            false
        }
    }
}

/// Compare the inodes of two `FileInfo`s.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_id_equal(
    file_info_1: *const FileInfo,
    file_info_2: *const FileInfo,
) -> bool {
    unsafe {
        (*file_info_1).stat.st_ino == (*file_info_2).stat.st_ino
            && (*file_info_1).stat.st_dev == (*file_info_2).stat.st_dev
    }
}

/// Get the `FileID` of a `FileInfo`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_id(file_info: *const FileInfo, file_id: *mut FileID) {
    unsafe {
        (*file_id).inode = (*file_info).stat.st_ino;
        (*file_id).device_id = (*file_info).stat.st_dev;
    }
}

/// Get the inode of a `FileInfo`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_inode(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_ino }
}

/// Get the size of a file from a `FileInfo`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_size(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_size }
}

/// Get the number of hardlinks from a `FileInfo`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_hardlinks(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_nlink }
}

/// Get the blocksize from a `FileInfo`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileinfo_blocksize(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_blksize }
}

/// Get the `FileID` for a given path.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileid(path: *const c_char, file_id: *mut FileID) -> bool {
    unsafe {
        let mut statbuf = uv_stat_t::default();
        if os_stat(path, &raw mut statbuf) == 0 {
            (*file_id).inode = statbuf.st_ino;
            (*file_id).device_id = statbuf.st_dev;
            true
        } else {
            false
        }
    }
}

/// Check if two `FileID`s are equal.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_fileid_equal(
    file_id_1: *const FileID,
    file_id_2: *const FileID,
) -> bool {
    unsafe {
        (*file_id_1).inode == (*file_id_2).inode && (*file_id_1).device_id == (*file_id_2).device_id
    }
}

/// Return the canonicalized absolute pathname.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_realpath(
    name: *const c_char,
    mut buf: *mut c_char,
    len: usize,
) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        let mut request: uv_fs_t = [0; 80];
        let result = uv_fs_realpath(ptr::null_mut(), &raw mut request, name, ptr::null());
        if result == 0 {
            if buf.is_null() {
                buf = xmalloc(len).cast::<c_char>();
            }
            xstrlcpy(buf, uv_fs_get_ptr(&raw const request).cast(), len);
            to_slash(buf);
        }
        uv_fs_req_cleanup(&raw mut request);
        if result == 0 { buf } else { ptr::null_mut() }
    }
    #[cfg(test)]
    {
        let _ = (name, &mut buf, len);
        ptr::null_mut()
    }
}

#[cfg_attr(all(not(test), windows), unsafe(no_mangle))]
pub unsafe extern "C" fn os_resolve_shortcut(fname: *const c_char) -> *mut c_char {
    #[cfg(all(not(test), windows))]
    unsafe {
        if fname.is_null() {
            return ptr::null_mut();
        }
        let bytes = CStr::from_ptr(fname).to_bytes();
        if bytes.len() <= 4 || !bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".lnk") {
            return ptr::null_mut();
        }

        let mut rfname: *mut c_char = ptr::null_mut();
        let mut pslw: *mut IShellLinkW = ptr::null_mut();
        let mut ppf: *mut IPersistFile = ptr::null_mut();

        let _ = CoInitialize(ptr::null_mut());

        let hr = CoCreateInstance(
            &raw const CLSID_SHELL_LINK,
            ptr::null_mut(),
            1, // CLSCTX_INPROC_SERVER
            &raw const IID_ISHELL_LINK_W,
            (&raw mut pslw).cast(),
        );
        if hr == 0 {
            let mut p: *mut u16 = ptr::null_mut();
            let r = utf8_to_utf16(fname, -1, &raw mut p);
            if r != 0 {
                semsg!(c"utf8_to_utf16 failed: %d".as_ptr(), r);
            } else if !p.is_null() {
                let hr2 = ((*(*pslw).lp_vtbl).query_interface)(
                    pslw,
                    &raw const IID_IPERSIST_FILE,
                    (&raw mut ppf).cast(),
                );
                if hr2 == 0 {
                    let hr3 = ((*(*ppf).lp_vtbl).load)(ppf, p, 0);
                    if hr3 == 0 {
                        let mut wsz = [0u16; 260];
                        let mut ffdw = [0u8; 600];
                        let hr4 = ((*(*pslw).lp_vtbl).get_path)(
                            pslw,
                            wsz.as_mut_ptr(),
                            260,
                            ffdw.as_mut_ptr().cast(),
                            0,
                        );
                        if hr4 == 0 && wsz[0] != 0 {
                            let r2 = utf16_to_utf8(wsz.as_ptr(), -1, &raw mut rfname);
                            if r2 != 0 {
                                semsg!(c"utf16_to_utf8 failed: %d".as_ptr(), r2);
                            }
                        }
                    }
                }
                xfree(p.cast());
            }
        }

        if !ppf.is_null() {
            let _ = ((*(*ppf).lp_vtbl).release)(ppf);
        }
        if !pslw.is_null() {
            let _ = ((*(*pslw).lp_vtbl).release)(pslw);
        }

        CoUninitialize();
        to_slash(rfname);
        rfname
    }
    #[cfg(any(test, not(windows)))]
    {
        let _ = fname;
        ptr::null_mut()
    }
}

#[cfg_attr(all(not(test), windows), unsafe(no_mangle))]
pub unsafe extern "C" fn os_is_reparse_point_include(path: *const c_char) -> bool {
    #[cfg(all(not(test), windows))]
    unsafe {
        let mut utf16_path: *mut u16 = ptr::null_mut();
        let r = utf8_to_utf16(path, -1, &raw mut utf16_path);
        if r != 0 {
            semsg!(c"utf8_to_utf16 failed: %d".as_ptr(), r);
            return false;
        }

        let is_sep = |c: u16| c == (b'\\' as u16) || c == (b'/' as u16);
        let mut p = utf16_path;
        let p0 = (*p & 0xff) as u8;
        if p0.is_ascii_alphabetic() && *p.add(1) == (b':' as u16) && is_sep(*p.add(2)) {
            p = p.add(3);
        } else if is_sep(*p) && is_sep(*p.add(1)) {
            p = p.add(2);
        }

        let mut buf = [0u16; 260];
        let mut result = false;
        let sep_accept = [b'\\' as u16, b'/' as u16, 0];

        while *p != 0 {
            let mut q = wcspbrk(p, sep_accept.as_ptr());
            if q.is_null() {
                q = utf16_path.add(wcslen(utf16_path));
                p = q;
            } else {
                p = q.add(1);
            }
            let len = ((q as usize) - (utf16_path as usize)) / size_of::<u16>();
            if len >= 260 {
                break;
            }
            let _ = wcsncpy(buf.as_mut_ptr(), utf16_path, len);
            buf[len] = 0;
            let attr = GetFileAttributesW(buf.as_ptr());
            if attr != 0xffff_ffff && (attr & 0x0400) != 0 {
                result = true;
                break;
            }
        }
        xfree(utf16_path.cast());
        result
    }
    #[cfg(any(test, not(windows)))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fileinfo_and_fileid_accessors_work() {
        unsafe {
            let mut fi1 = FileInfo::default();
            fi1.stat.st_ino = 42;
            fi1.stat.st_dev = 7;
            fi1.stat.st_size = 1234;
            fi1.stat.st_nlink = 2;
            fi1.stat.st_blksize = 4096;

            let mut fi2 = fi1;
            assert!(os_fileinfo_id_equal(&raw const fi1, &raw const fi2));
            assert_eq!(os_fileinfo_inode(&raw const fi1), 42);
            assert_eq!(os_fileinfo_size(&raw const fi1), 1234);
            assert_eq!(os_fileinfo_hardlinks(&raw const fi1), 2);
            assert_eq!(os_fileinfo_blocksize(&raw const fi1), 4096);

            let mut id1 = FileID::default();
            os_fileinfo_id(&raw const fi1, &raw mut id1);
            let id2 = FileID {
                inode: 42,
                device_id: 7,
            };
            assert!(os_fileid_equal(&raw const id1, &raw const id2));
        }
    }
}
