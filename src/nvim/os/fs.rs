//! Filesystem access and libuv file operations (`src/nvim/os/fs.c`).

use crate::api::private::helpers::cstr_as_string;
use crate::errors::*;

#[cfg(windows)]
use crate::event::r#loop::uv_guess_handle;
use crate::event::r#loop::{
    uv_chdir, uv_cwd, uv_fs_req_cleanup, uv_req_size, uv_strerror, uv_translate_sys_error,
};
use crate::globals::*;
#[cfg(unix)]
use crate::log::logmsg;
#[cfg(windows)]
use crate::mbyte::{mb_strnicmp, utf8_to_utf16, utf16_to_utf8};
use crate::memory::{cstr_take, memcnt, xmalloc, xmemcpyz, xstrchrnul, xstrdup, xstrlcpy};
use crate::message::{emsg, semsg, smsg, verbose_enter, verbose_leave};
#[cfg(windows)]
use crate::option::copy_option_part;
use crate::option_vars::*;
use crate::os::env::os_getenv_cstr;
#[cfg(windows)]
use crate::os::env::{os_env_exists, os_getenv_buf};
use crate::path::{
    FullName_save, append_path, dir_of_file_exists, get_past_head, gettail_dir, path_skip_sep,
    path_tail_with_sep, path_with_url, save_abs_path, to_slash, vim_ispathsep,
};
#[cfg(windows)]
use crate::path::{path_next_component, path_tail, vim_ispathsep_nocolon};
#[cfg(unix)]
use crate::strings::strerror;
use crate::strings::{gettext, strlen};
use crate::ui::ui_call_chdir;
#[cfg(windows)]
use std::ffi::c_uint;
use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::ptr;

pub(crate) const MAXPATHL: usize = 4096;

#[cfg(windows)]
use crate::os::os_win_console::os_redirect_stdin_to_conin;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileID {
    pub inode: u64,
    pub device_id: u64,
}

impl FileID {
    #[inline]
    pub const fn new() -> Self {
        Self {
            inode: 0,
            device_id: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct uv_timespec_t {
    pub tv_sec: c_long,
    pub tv_nsec: c_long,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct uv_stat_t {
    pub st_dev: u64,
    pub st_mode: u64,
    pub st_nlink: u64,
    pub st_uid: u64,
    pub st_gid: u64,
    pub st_rdev: u64,
    pub st_ino: u64,
    pub st_size: u64,
    pub st_blksize: u64,
    pub st_blocks: u64,
    pub st_flags: u64,
    pub st_gen: u64,
    pub st_atim: uv_timespec_t,
    pub st_mtim: uv_timespec_t,
    pub st_ctim: uv_timespec_t,
    pub st_birthtim: uv_timespec_t,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileInfo {
    pub stat: uv_stat_t,
    pub prefix_off: usize,
    pub root_off: usize,
    pub rest_off: usize,
    pub r#type: c_int,
}

impl FileInfo {
    #[inline]
    pub const fn new() -> Self {
        Self {
            stat: uv_stat_t {
                st_dev: 0,
                st_mode: 0,
                st_nlink: 0,
                st_uid: 0,
                st_gid: 0,
                st_rdev: 0,
                st_ino: 0,
                st_size: 0,
                st_blksize: 0,
                st_blocks: 0,
                st_flags: 0,
                st_gen: 0,
                st_atim: uv_timespec_t {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
                st_mtim: uv_timespec_t {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
                st_ctim: uv_timespec_t {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
                st_birthtim: uv_timespec_t {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
            },
            prefix_off: 0,
            root_off: 0,
            rest_off: 0,
            r#type: 0,
        }
    }
}

#[repr(C)]
pub struct Directory {
    pub _opaque: [u64; 88],
}

impl Directory {
    pub const fn new() -> Self {
        Self { _opaque: [0; 88] }
    }
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}
pub type vim_acl_T = *mut c_void;
pub type uv_fs_buf_t = [u64; 80];
#[cfg(any(unix, test))]
use crate::event::r#loop::{uv_fs_fchown, uv_gid_t, uv_uid_t};

const NODE_NORMAL: c_int = 0;
pub(crate) const NODE_OTHER: c_int = 2;

const kPathGeneric: c_int = 1;
#[cfg(windows)]
const kPathDrive: c_int = 2;
#[cfg(windows)]
const kPathUNC: c_int = 3;
#[cfg(windows)]
const kPathDevice: c_int = 4;

const UV_FS: c_int = 6;

pub(crate) const UV_EOF: c_int = -4095;
pub(crate) const UV_UNKNOWN: c_int = -4094;

pub(crate) const UV_ENOENT: c_int = if cfg!(windows) { -4058 } else { -2 };
pub(crate) const UV_EINTR: c_int = if cfg!(windows) { -4072 } else { -4 };
pub(crate) const UV_EINVAL: c_int = if cfg!(windows) { -4071 } else { -22 };
pub(crate) const UV_EIO: c_int = if cfg!(windows) { -4070 } else { -5 };
pub(crate) const UV_EROFS: c_int = if cfg!(windows) { -4043 } else { -30 };
pub(crate) const UV_ENOTSUP: c_int = if cfg!(windows) {
    -4049
} else if cfg!(target_os = "linux") {
    -95
} else {
    -45
};
pub(crate) const UV_ENOBUFS: c_int = if cfg!(windows) {
    -4060
} else if cfg!(target_os = "linux") {
    -105
} else {
    -55
};
const UV_EAGAIN: c_int = if cfg!(windows) {
    -4088
} else if cfg!(target_os = "linux") {
    -11
} else {
    -35
};

const S_IFBLK: u64 = 0o060000;
pub(crate) const S_IFLNK: u64 = 0o120000;

const R_OK: c_int = 4;
const W_OK: c_int = 2;
#[cfg(unix)]
const X_OK: c_int = 1;

pub(crate) const O_WRONLY: c_int = 1;
pub(crate) const O_RDWR: c_int = 2;
pub(crate) const O_CREAT: c_int = if cfg!(windows) {
    0x0100
} else if cfg!(target_os = "linux") {
    0o100
} else {
    0x0200
};
pub(crate) const O_TRUNC: c_int = if cfg!(any(windows, target_os = "linux")) {
    0x0200
} else {
    0x0400
};
pub(crate) const O_EXCL: c_int = if cfg!(windows) {
    0x0400
} else if cfg!(target_os = "linux") {
    0o200
} else {
    0x0800
};
pub(crate) const O_APPEND: c_int = if cfg!(target_os = "linux") {
    0o2000
} else {
    0x0008
};
pub(crate) const O_NOFOLLOW: c_int = if cfg!(windows) {
    0
} else if cfg!(all(
    target_os = "linux",
    any(target_arch = "arm", target_arch = "aarch64")
)) {
    0o100000
} else if cfg!(target_os = "linux") {
    0o400000
} else {
    0x0100
};
pub(crate) const TEMP_FILE_PATH_MAXLEN: usize = if cfg!(windows) { 260 } else { 256 };
pub(crate) const CASE_INSENSITIVE_FILENAME: bool =
    cfg!(any(target_os = "windows", target_os = "macos"));

#[cfg(windows)]
pub(crate) const O_BINARY: c_int = 0x8000;
pub(crate) const ENV_SEPCHAR: u8 = if cfg!(windows) { b';' } else { b':' };
#[cfg_attr(not(windows), cfg(test))]
pub(crate) const ENV_SEPSTR: &CStr = if cfg!(windows) { c";" } else { c":" };
pub(crate) const PATHSEP: c_char = b'/' as c_char;
pub(crate) const PATHSEPSTR: &CStr = c"/";
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

#[cfg(unix)]
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

use crate::charset::NUL;

use crate::bufwrite::NODE_WRITABLE;
use crate::channel::STDIN_FILENO;
#[cfg(windows)]
use crate::event::stream::{UV_FILE, UV_TTY};
#[cfg(windows)]
use crate::fileio::{_get_osfhandle, _read, _write, close};
use crate::fileio::{O_RDONLY, S_IFDIR, S_IFMT, S_IFREG, dup, fdopen};
#[cfg(not(windows))]
use crate::fileio::{fcntl, getuid, read, write};
use crate::math::{FAIL, OK};
#[cfg(windows)]
use crate::memline::kPathDeviceUNC;

use crate::event::r#loop::{
    uv_exepath, uv_fs_access, uv_fs_chmod, uv_fs_close, uv_fs_copyfile, uv_fs_fstat, uv_fs_fsync,
    uv_fs_get_path, uv_fs_get_ptr, uv_fs_get_statbuf, uv_fs_lstat, uv_fs_mkdir, uv_fs_mkdtemp,
    uv_fs_open, uv_fs_realpath, uv_fs_rename, uv_fs_rmdir, uv_fs_scandir, uv_fs_scandir_next,
    uv_fs_stat, uv_fs_unlink,
};
#[cfg(unix)]
use crate::event::r#loop::{uv_fs_chown, uv_fs_fchmod, uv_fs_utime};

unsafe extern "C" {
    #[cfg_attr(windows, link_name = "_errno")]
    #[cfg_attr(target_os = "linux", link_name = "__errno_location")]
    #[cfg_attr(all(not(windows), not(target_os = "linux")), link_name = "__error")]
    pub fn errno_ptr() -> *mut c_int;

    #[cfg(not(windows))]
    fn readv(fd: c_int, iov: *const iovec, iovcnt: c_int) -> isize;

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
}

#[cfg(windows)]
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
pub const fn s_isdir(m: u64) -> bool {
    (m & S_IFMT) == S_IFDIR
}

#[inline]
pub const fn s_isreg(m: u64) -> bool {
    (m & S_IFMT) == S_IFREG
}

#[inline]
pub const fn s_isblk(m: u64) -> bool {
    cfg!(unix) && (m & S_IFMT) == S_IFBLK
}

#[inline]
pub const fn s_islnk(m: u64) -> bool {
    (m & S_IFMT) == S_IFLNK
}

#[inline]
pub const fn s_isfifo(m: u64) -> bool {
    (m & S_IFMT) == crate::fileio::S_IFIFO
}

#[inline]
pub const fn s_issock(m: u64) -> bool {
    cfg!(unix) && (m & S_IFMT) == crate::fileio::S_IFSOCK
}

#[inline]
pub unsafe fn os_errno() -> c_int {
    unsafe { *errno_ptr() }
}

#[inline]
pub unsafe fn os_set_errno(val: c_int) {
    unsafe {
        *errno_ptr() = val;
    }
}

/// Changes the current directory to `path`.
pub unsafe fn os_chdir(path: *const c_char) -> c_int {
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
}

/// Get the name of current directory.
pub unsafe fn os_dirname(buf: *mut c_char, mut len: usize) -> c_int {
    unsafe {
        let error_number = uv_cwd(buf, &raw mut len);
        if error_number != 0 {
            xstrlcpy(buf, uv_strerror(error_number), len);
            return FAIL;
        }
        to_slash(buf);
        OK
    }
}

/// Check if the given path is a directory and not a symlink to a directory.
pub unsafe fn os_isrealdir(name: *const c_char) -> bool {
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        if uv_fs_lstat(ptr::null_mut(), &raw mut request, name, ptr::null()) != 0 {
            uv_fs_req_cleanup((&raw mut request).cast());
            return false;
        }
        let mode = (*uv_fs_get_statbuf(&raw mut request)).st_mode;
        uv_fs_req_cleanup((&raw mut request).cast());
        if s_islnk(mode) {
            return false;
        }
        s_isdir(mode)
    }
}

/// Check if the given path exists and is a directory.
pub unsafe fn os_isdir(name: *const c_char) -> bool {
    unsafe {
        let mode = os_getperm(name);
        if mode < 0 {
            return false;
        }
        s_isdir(mode as u64)
    }
}

/// Check what `name` is (`NODE_NORMAL`, `NODE_WRITABLE`, or `NODE_OTHER`).
pub unsafe fn os_nodetype(name: *const c_char) -> c_int {
    #[cfg(not(windows))]
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
    #[cfg(windows)]
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
        let _ = close(fd);
        match guess {
            UV_TTY => NODE_WRITABLE,
            UV_FILE => NODE_NORMAL,
            _ => NODE_OTHER,
        }
    }
}

/// Gets the absolute path of the currently running executable.
pub unsafe fn os_exepath(buffer: *mut c_char, size: *mut usize) -> c_int {
    unsafe {
        let r = uv_exepath(buffer, size);
        to_slash(buffer);
        r
    }
}

/// Checks if the file `name` is executable.
pub unsafe fn os_can_exe(name: *const c_char, abspath: *mut *mut c_char, use_path: bool) -> bool {
    unsafe {
        if !use_path || gettail_dir(name) != name {
            #[cfg(windows)]
            return is_executable_ext(name, abspath);
            #[cfg(not(windows))]
            return (use_path || gettail_dir(name) != name) && is_executable(name, abspath);
        }
        is_executable_in_path(name, abspath)
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
        let ok = s_isreg(mode as u64) && {
            {
                let mut req: uv_fs_buf_t = [0; 80];
                let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, X_OK, ptr::null());
                uv_fs_req_cleanup((&raw mut req).cast());
                r == 0
            }
        };

        if ok && !abspath.is_null() {
            *abspath = save_abs_path(name);
        }
        ok
    }
}

#[cfg(windows)]
unsafe fn is_executable_ext(name: *const c_char, abspath: *mut *mut c_char) -> bool {
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
        let mut pathext_buf = [0 as c_char; MAXPATHL];
        let mut pathext: *const c_char = os_getenv_buf(
            c"PATHEXT".as_ptr(),
            pathext_buf.as_mut_ptr(),
            pathext_buf.len(),
        );
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
}

#[cfg(not(windows))]
#[inline]
unsafe fn is_executable_ext(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    unsafe { is_executable(name, abspath) }
}

unsafe fn is_executable_in_path(name: *const c_char, abspath: *mut *mut c_char) -> bool {
    unsafe {
        let Some(path_env) = os_getenv_cstr(c"PATH".as_ptr()) else {
            return false;
        };

        #[cfg(windows)]
        let path_vec: Vec<c_char> = {
            let sh_tail = CStr::from_ptr(path_tail(p_sh)).to_bytes();
            let env_bytes = path_env.as_bytes_with_nul();
            if !os_env_exists(c"NoDefaultCurrentDirectoryInExePath".as_ptr(), false)
                && sh_tail.windows(7).any(|w| w == b"cmd.exe")
            {
                let mut v = Vec::with_capacity(env_bytes.len() + 2);
                v.push(b'.' as c_char);
                v.push(b';' as c_char);
                v.extend(env_bytes.iter().map(|&b| b as c_char));
                v
            } else {
                env_bytes.iter().map(|&b| b as c_char).collect()
            }
        };
        #[cfg(windows)]
        let path = path_vec.as_ptr();
        #[cfg(not(windows))]
        let path = path_env.as_ptr();

        let bufsize = strlen(name) + strlen(path) + 2;
        let mut buf_vec = vec![0 as c_char; bufsize];
        let buf = buf_vec.as_mut_ptr();

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

        rv
    }
}

/// Opens or creates a file and returns a file descriptor, or negative libuv error code.
pub unsafe fn os_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    if path.is_null() {
        return UV_EINVAL;
    }
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_open(
            ptr::null_mut(),
            &raw mut req,
            path,
            flags,
            mode,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Compatibility wrapper conforming to `fopen(3)`.
pub unsafe fn os_fopen(path: *const c_char, flags: *const c_char) -> *mut crate::fileio::FILE {
    unsafe {
        assert!(!flags.is_null());
        let flen = strlen(flags);
        assert!((1..=2).contains(&flen));

        let f0 = *flags as u8;
        let f1 = *flags.add(1) as u8;
        let iflags = if f1 == 0 || f1 == b'b' {
            let base = match f0 {
                b'r' => O_RDONLY,
                b'w' => O_WRONLY | O_CREAT | O_TRUNC,
                b'a' => O_WRONLY | O_CREAT | O_APPEND,
                _ => std::process::abort(),
            };
            #[cfg(windows)]
            {
                if f1 == b'b' { base | O_BINARY } else { base }
            }
            #[cfg(not(windows))]
            {
                base
            }
        } else {
            assert_eq!(f1, b'+');
            match f0 {
                b'r' => O_RDWR,
                b'w' => O_RDWR | O_CREAT | O_TRUNC,
                b'a' => O_RDWR | O_CREAT | O_APPEND,
                _ => std::process::abort(),
            }
        };

        let fd = os_open(path, iflags, 0o666);
        if fd < 0 {
            return ptr::null_mut();
        }
        fdopen(fd, flags)
    }
}

/// Sets file descriptor `fd` to close-on-exec (Unix) or non-inheritable (Windows).
pub unsafe fn os_set_cloexec(fd: c_int) -> c_int {
    #[cfg(not(windows))]
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
    #[cfg(windows)]
    unsafe {
        let h = _get_osfhandle(fd);
        if h == -1 || SetHandleInformation(h as *mut c_void, 1, 0) == 0 {
            return -1;
        }
        0
    }
}

/// Close a file.
pub unsafe fn os_close(fd: c_int) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_close(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Duplicate file descriptor.
pub unsafe fn os_dup(fd: c_int) -> c_int {
    unsafe {
        loop {
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
}

/// Duplicates file descriptor `fd` and marks the copy close-on-exec.
#[cfg(windows)]
pub unsafe fn os_dup_cloexec(fd: c_int) -> c_int {
    unsafe {
        let newfd = os_dup(fd);
        if newfd >= 0 {
            let _ = os_set_cloexec(newfd);
        }
        newfd
    }
}

/// Open the file descriptor for stdin.
pub unsafe fn os_open_stdin_fd() -> c_int {
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
}

/// Read from a file.
pub unsafe fn os_read(
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
    }
}

/// Read from a file to multiple buffers at once.
#[cfg(unix)]
pub unsafe fn os_readv(
    fd: c_int,
    ret_eof: *mut bool,
    mut iov: *mut iovec,
    mut iov_size: usize,
    non_blocking: bool,
) -> isize {
    unsafe {
        *ret_eof = false;
        #[cfg(not(windows))]
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
        #[cfg(windows)]
        {
            let _ = (fd, &mut iov, &mut iov_size, non_blocking);
            0
        }
    }
}

/// Write to a file.
pub unsafe fn os_write(fd: c_int, buf: *const c_char, size: usize, non_blocking: bool) -> isize {
    if buf.is_null() {
        assert_eq!(size, 0);
        return 0;
    }
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
                return UV_UNKNOWN as isize;
            }
        }
        written_bytes as isize
    }
}

/// Copies a file from `path` to `new_path`.
pub unsafe fn os_copy(path: *const c_char, new_path: *const c_char, flags: c_int) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_copyfile(
            ptr::null_mut(),
            &raw mut req,
            path,
            new_path,
            flags,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Flushes file modifications to disk.
pub unsafe fn os_fsync(fd: c_int) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fsync(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        g_stats.fsync += 1;
        r
    }
}

unsafe fn os_stat(name: *const c_char, statbuf: *mut uv_stat_t) -> c_int {
    if name.is_null() {
        return UV_EINVAL;
    }
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let result = uv_fs_stat(ptr::null_mut(), &raw mut request, name, ptr::null());
        if result == 0 {
            *statbuf = *uv_fs_get_statbuf(&raw mut request);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        result
    }
}

/// Get the file permissions for a given file.
pub unsafe fn os_getperm(name: *const c_char) -> i32 {
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
pub unsafe fn os_setperm(name: *const c_char, perm: c_int) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_chmod(ptr::null_mut(), &raw mut req, name, perm, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        if r == 0 { OK } else { FAIL }
    }
}

/// Set the permission of the file referred to by the open file descriptor.
#[cfg(unix)]
pub unsafe fn os_fsetperm(fd: c_int, perm: c_int) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fchmod(ptr::null_mut(), &raw mut req, fd, perm, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        if r == 0 { OK } else { FAIL }
    }
}

/// Copy extended attributes `from_file` to `to_file`.
pub unsafe fn os_copy_xattr(from_file: *const c_char, to_file: *const c_char) {
    #[cfg(not(target_os = "linux"))]
    let _ = (from_file, to_file);
    #[cfg(target_os = "linux")]
    unsafe {
        if from_file.is_null() {
            return;
        }
        let mut size = listxattr(from_file, ptr::null_mut(), 0);
        if size <= 0 {
            return;
        }
        let mut xattr_buf = vec![0 as c_char; size as usize];
        size = listxattr(from_file, xattr_buf.as_mut_ptr(), size as usize);
        let tsize = size;

        let ep = errno_ptr();
        *ep = 0;

        let mut max_vallen: isize = 0;
        let mut val_buf: Vec<c_char> = Vec::new();
        let mut errmsg: *const c_char = ptr::null();

        'outer: for round in 0..2 {
            let mut key = xattr_buf.as_ptr();
            if round == 1 {
                size = tsize;
            }

            while size > 0 {
                let val_ptr = if round != 0 {
                    val_buf.as_mut_ptr()
                } else {
                    ptr::null_mut()
                };
                let vallen = getxattr(
                    from_file,
                    key,
                    val_ptr.cast(),
                    if round != 0 { max_vallen as usize } else { 0 },
                );
                if vallen >= 0
                    && round != 0
                    && setxattr(to_file, key, val_ptr.cast(), vallen as usize, 0) == 0
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

                let keylen = (strlen(key) + 1) as isize;
                size -= keylen;
                key = key.add(keylen as usize);
            }
            if round != 0 {
                break;
            }
            val_buf = vec![0 as c_char; (max_vallen as usize) + 1];
        }

        if !errmsg.is_null() {
            emsg(gettext(errmsg));
        }
    }
}

pub fn os_get_acl(_fname: *const c_char) -> vim_acl_T {
    ptr::null_mut()
}

pub fn os_set_acl(_fname: *const c_char, _aclent: vim_acl_T) {}

pub fn os_free_acl(_aclent: vim_acl_T) {}

/// Checks if the current user owns a file.
pub unsafe fn os_file_owned(fname: *const c_char) -> bool {
    #[cfg(not(windows))]
    unsafe {
        let uid = u64::from(getuid());
        let mut finfo = FileInfo::new();
        let file_owned = os_fileinfo(fname, &raw mut finfo) && finfo.stat.st_uid == uid;
        let link_owned = os_fileinfo_link(fname, &raw mut finfo) && finfo.stat.st_uid == uid;
        file_owned && link_owned
    }
    #[cfg(windows)]
    {
        let _ = fname;
        true
    }
}

/// Changes the owner and group of a file, like `chown(2)`.
#[cfg(unix)]
pub unsafe fn os_chown(path: *const c_char, owner: uv_uid_t, group: uv_gid_t) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_chown(
            ptr::null_mut(),
            &raw mut req,
            path,
            owner,
            group,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Changes the owner and group of an open file descriptor, like `fchown(2)`.
#[cfg_attr(not(unix), cfg(test))]
pub unsafe fn os_fchown(fd: c_int, owner: uv_uid_t, group: uv_gid_t) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fchown(ptr::null_mut(), &raw mut req, fd, owner, group, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Check if a path exists.
pub unsafe fn os_path_exists(path: *const c_char) -> bool {
    unsafe {
        let mut statbuf = uv_stat_t::default();
        os_stat(path, &raw mut statbuf) == 0
    }
}

/// Sets file access and modification times.
#[cfg(unix)]
pub unsafe fn os_file_settime(path: *const c_char, atime: f64, mtime: f64) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_utime(
            ptr::null_mut(),
            &raw mut req,
            path,
            atime,
            mtime,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Check if a file is readable.
pub unsafe fn os_file_is_readable(name: *const c_char) -> bool {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, R_OK, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r == 0
    }
}

/// Check if a file is writable (`0` = not writable, `1` = writable file, `2` = writable dir).
pub unsafe fn os_file_is_writable(name: *const c_char) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_access(ptr::null_mut(), &raw mut req, name, W_OK, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        if r == 0 {
            if os_isdir(name) { 2 } else { 1 }
        } else {
            0
        }
    }
}

/// Rename a file or directory.
pub unsafe fn os_rename(path: *const c_char, new_path: *const c_char) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_rename(ptr::null_mut(), &raw mut req, path, new_path, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        if r == 0 { OK } else { FAIL }
    }
}

/// Make a directory.
pub unsafe fn os_mkdir(path: *const c_char, mode: i32) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_mkdir(ptr::null_mut(), &raw mut req, path, mode, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Make a directory, with higher levels when needed.
pub unsafe fn os_mkdir_recurse(
    dir: *const c_char,
    mode: i32,
    failed_dir: *mut *mut c_char,
    created: *mut *mut c_char,
) -> c_int {
    unsafe {
        let dirlen = strlen(dir);
        let mut curdir_vec: Vec<c_char> = std::slice::from_raw_parts(dir, dirlen + 1).to_vec();
        let curdir = curdir_vec.as_mut_ptr();
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
            let component_len = strlen(e);
            e = e.add(component_len);
            if ptr::eq(e, real_end)
                && memcnt(e.sub(component_len).cast(), PATHSEP, component_len) == component_len
            {
                break;
            }
            let ret = os_mkdir(curdir, mode);
            if ret != 0 {
                *failed_dir = xstrdup(curdir);
                return ret;
            } else if !created.is_null() && (*created).is_null() {
                *created = FullName_save(curdir, false);
            }
        }
        0
    }
}

/// Create the parent directory of a file if it does not exist.
pub unsafe fn os_file_mkdir(fname: *mut c_char, mode: i32) -> c_int {
    unsafe {
        if !dir_of_file_exists(fname) {
            let tail = path_tail_with_sep(fname);
            let last_char = tail.add(strlen(tail)).sub(1);
            if vim_ispathsep(c_int::from(*last_char as u8)) {
                emsg(gettext(e_noname.as_ptr()));
                return -1;
            }
            let c = *tail;
            *tail = NUL;
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let r = os_mkdir_recurse(fname, mode, &raw mut failed_dir, ptr::null_mut());
            let failed_dir = cstr_take(failed_dir);
            if r < 0 {
                let _ = semsg!(
                    gettext(e_mkdir.as_ptr()),
                    failed_dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                    uv_strerror(r),
                );
            }
            *tail = c;
            return r;
        }
        0
    }
}

/// Create a unique temporary directory.
pub unsafe fn os_mkdtemp(templ: *const c_char, path: *mut c_char) -> c_int {
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let result = uv_fs_mkdtemp(ptr::null_mut(), &raw mut request, templ, ptr::null());
        if result == 0 {
            xstrlcpy(
                path,
                uv_fs_get_path(&raw const request),
                TEMP_FILE_PATH_MAXLEN,
            );
            to_slash(path);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        result
    }
}

/// Remove a directory.
pub unsafe fn os_rmdir(path: *const c_char) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_rmdir(ptr::null_mut(), &raw mut req, path, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Opens a directory.
pub unsafe fn os_scandir(dir: *mut Directory, path: *const c_char) -> bool {
    unsafe {
        debug_assert!(uv_req_size(UV_FS) + size_of::<uv_dirent_t>() <= size_of::<Directory>());
        let req = dir.cast::<uv_fs_buf_t>();
        let r = uv_fs_scandir(ptr::null_mut(), req, path, 0, ptr::null());
        if r < 0 {
            os_closedir(dir);
        }
        r >= 0
    }
}

/// Increments the directory pointer.
pub unsafe fn os_scandir_next(dir: *mut Directory) -> *const c_char {
    unsafe {
        let req = dir.cast::<uv_fs_buf_t>();
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
}

/// Frees memory associated with `os_scandir()`.
pub unsafe fn os_closedir(dir: *mut Directory) {
    unsafe {
        uv_fs_req_cleanup(dir.cast());
    }
}

/// Remove a file.
pub unsafe fn os_remove(path: *const c_char) -> c_int {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_unlink(ptr::null_mut(), &raw mut req, path, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Get the file information for a given path.
pub unsafe fn os_fileinfo(path: *const c_char, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::new();
        os_stat(path, &raw mut (*file_info).stat) == 0
    }
}

/// Parses `path` into a `FileInfo` structure.
pub unsafe fn os_fileinfo2(path: *const c_char, info: *mut FileInfo) -> bool {
    unsafe {
        *info = FileInfo::new();
        {
            if path_with_url(path) != 0 {
                return true;
            }
            #[cfg(windows)]
            let mut p: *const c_char = path_skip_sep(path, false);
            #[cfg(not(windows))]
            let p: *const c_char = path_skip_sep(path, false);
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
pub unsafe fn os_fileinfo_link(path: *const c_char, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::new();
        if path.is_null() {
            return false;
        }
        {
            let mut request: uv_fs_buf_t = [0; 80];
            let ok = uv_fs_lstat(ptr::null_mut(), &raw mut request, path, ptr::null()) == 0;
            if ok {
                (*file_info).stat = *uv_fs_get_statbuf(&raw mut request);
            }
            uv_fs_req_cleanup((&raw mut request).cast());
            ok
        }
    }
}

/// Get the file information for a given file descriptor.
pub unsafe fn os_fileinfo_fd(file_descriptor: c_int, file_info: *mut FileInfo) -> bool {
    unsafe {
        *file_info = FileInfo::new();
        {
            let mut request: uv_fs_buf_t = [0; 80];
            let ok = uv_fs_fstat(
                ptr::null_mut(),
                &raw mut request,
                file_descriptor,
                ptr::null(),
            ) == 0;
            if ok {
                (*file_info).stat = *uv_fs_get_statbuf(&raw mut request);
            }
            uv_fs_req_cleanup((&raw mut request).cast());
            ok
        }
    }
}

/// Compare the inodes of two `FileInfo`s.
pub unsafe fn os_fileinfo_id_equal(
    file_info_1: *const FileInfo,
    file_info_2: *const FileInfo,
) -> bool {
    unsafe {
        (*file_info_1).stat.st_ino == (*file_info_2).stat.st_ino
            && (*file_info_1).stat.st_dev == (*file_info_2).stat.st_dev
    }
}

/// Get the `FileID` of a `FileInfo`.
#[cfg(test)]
pub unsafe fn os_fileinfo_id(file_info: *const FileInfo, file_id: *mut FileID) {
    unsafe {
        (*file_id).inode = (*file_info).stat.st_ino;
        (*file_id).device_id = (*file_info).stat.st_dev;
    }
}

/// Get the inode of a `FileInfo`.
pub unsafe fn os_fileinfo_inode(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_ino }
}

/// Get the size of a file from a `FileInfo`.
pub unsafe fn os_fileinfo_size(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_size }
}

/// Get the number of hardlinks from a `FileInfo`.
pub unsafe fn os_fileinfo_hardlinks(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_nlink }
}

/// Get the blocksize from a `FileInfo`.
pub unsafe fn os_fileinfo_blocksize(file_info: *const FileInfo) -> u64 {
    unsafe { (*file_info).stat.st_blksize }
}

/// Get the `FileID` for a given path.
pub unsafe fn os_fileid(path: *const c_char, file_id: *mut FileID) -> bool {
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
pub unsafe fn os_fileid_equal(file_id_1: *const FileID, file_id_2: *const FileID) -> bool {
    unsafe {
        (*file_id_1).inode == (*file_id_2).inode && (*file_id_1).device_id == (*file_id_2).device_id
    }
}

/// Return the canonicalized absolute pathname.
pub unsafe fn os_realpath(name: *const c_char, mut buf: *mut c_char, len: usize) -> *mut c_char {
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let result = uv_fs_realpath(ptr::null_mut(), &raw mut request, name, ptr::null());
        if result == 0 {
            if buf.is_null() {
                buf = xmalloc(len).cast::<c_char>();
            }
            xstrlcpy(buf, uv_fs_get_ptr(&raw const request).cast(), len);
            to_slash(buf);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        if result == 0 { buf } else { ptr::null_mut() }
    }
}

#[cfg(windows)]
pub unsafe fn os_resolve_shortcut(fname: *const c_char) -> *mut c_char {
    #[cfg(windows)]
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
            let mut p: Vec<u16> = Vec::new();
            let r = utf8_to_utf16(fname, -1, &mut p);
            if r != 0 {
                semsg!(c"utf8_to_utf16 failed: %d".as_ptr(), r);
            } else if !p.is_empty() {
                let hr2 = ((*(*pslw).lp_vtbl).query_interface)(
                    pslw,
                    &raw const IID_IPERSIST_FILE,
                    (&raw mut ppf).cast(),
                );
                if hr2 == 0 {
                    let hr3 = ((*(*ppf).lp_vtbl).load)(ppf, p.as_ptr(), 0);
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
    #[cfg(not(windows))]
    {
        let _ = fname;
        ptr::null_mut()
    }
}

#[cfg(windows)]
pub unsafe fn os_is_reparse_point_include(path: *const c_char) -> bool {
    #[cfg(windows)]
    unsafe {
        let mut utf16_vec: Vec<u16> = Vec::new();
        let r = utf8_to_utf16(path, -1, &mut utf16_vec);
        if r != 0 {
            semsg!(c"utf8_to_utf16 failed: %d".as_ptr(), r);
            return false;
        }
        let utf16_path = utf16_vec.as_ptr();

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

        while *p != 0 {
            let mut q = p;
            while *q != 0 && !is_sep(*q) {
                q = q.add(1);
            }
            if *q == 0 {
                p = q;
            } else {
                p = q.add(1);
            }
            let len = ((q as usize) - (utf16_path as usize)) / size_of::<u16>();
            if len >= 260 {
                break;
            }
            ptr::copy_nonoverlapping(utf16_path, buf.as_mut_ptr(), len);
            buf[len] = 0;
            let attr = GetFileAttributesW(buf.as_ptr());
            if attr != 0xffff_ffff && (attr & 0x0400) != 0 {
                result = true;
                break;
            }
        }
        result
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn fileinfo_and_fileid_accessors_work() {
        unsafe {
            let mut fi1 = FileInfo::new();
            fi1.stat.st_ino = 42;
            fi1.stat.st_dev = 7;
            fi1.stat.st_size = 1234;
            fi1.stat.st_nlink = 2;
            fi1.stat.st_blksize = 4096;

            let fi2 = fi1;
            assert!(os_fileinfo_id_equal(&raw const fi1, &raw const fi2));
            assert_eq!(os_fileinfo_inode(&raw const fi1), 42);
            assert_eq!(os_fileinfo_size(&raw const fi1), 1234);
            assert_eq!(os_fileinfo_hardlinks(&raw const fi1), 2);
            assert_eq!(os_fileinfo_blocksize(&raw const fi1), 4096);

            let mut id1 = FileID::new();
            os_fileinfo_id(&raw const fi1, &raw mut id1);
            let id2 = FileID {
                inode: 42,
                device_id: 7,
            };
            assert!(os_fileid_equal(&raw const id1, &raw const id2));
        }
    }

    #[test]
    fn dirname_chdir_isdir_nodetype_and_can_exe() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            let mut cwd = [0 as c_char; MAXPATHL];
            assert_eq!(os_dirname(cwd.as_mut_ptr(), MAXPATHL), OK);
            let cwd_len = CStr::from_ptr(cwd.as_ptr()).to_bytes().len();
            assert!(cwd_len > 0);
            let mut tiny = [0 as c_char; 4];
            assert_eq!(os_dirname(tiny.as_mut_ptr(), cwd_len.min(3)), FAIL);

            // os_chdir("~") fails and leaves cwd unchanged
            assert!(!os_isdir(c"~".as_ptr()));
            assert_ne!(os_chdir(c"~".as_ptr()), 0);
            assert_ne!(os_chdir(c"~/".as_ptr()), 0);

            assert!(!os_isdir(c"".as_ptr()));
            assert!(!os_isdir(c"non-existing-directory-xyz".as_ptr()));
            assert!(!os_isdir(c"/non-existing-directory-xyz".as_ptr()));
            assert!(os_isdir(c".".as_ptr()));
            assert!(os_isdir(c"..".as_ptr()));
            assert!(os_isdir(cwd.as_ptr()));

            assert_eq!(os_nodetype(c"non-existing-file-xyz".as_ptr()), NODE_NORMAL);
            #[cfg(not(windows))]
            assert_eq!(os_nodetype(c"/dev/stderr".as_ptr()), NODE_WRITABLE);

            let mut abspath: *mut c_char = ptr::null_mut();
            assert!(!os_can_exe(c".".as_ptr(), &raw mut abspath, true));
            assert!(abspath.is_null());
            assert!(!os_can_exe(
                c"does-not-exist.file".as_ptr(),
                &raw mut abspath,
                true
            ));
            assert!(abspath.is_null());

            #[cfg(windows)]
            let tool = c"cmd";
            #[cfg(not(windows))]
            let tool = c"sh";
            assert!(os_can_exe(tool.as_ptr(), &raw mut abspath, true));
            let abspath_cstr = cstr_take(abspath).unwrap();
            assert!(crate::path::path_is_absolute(abspath_cstr.as_ptr()));
        }
    }

    #[test]
    fn open_read_write_rename_remove_mkdir_and_permissions() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            let mut cwd_buf = [0 as c_char; MAXPATHL];
            assert_eq!(os_dirname(cwd_buf.as_mut_ptr(), MAXPATHL), OK);
            let cwd = CStr::from_ptr(cwd_buf.as_ptr()).to_str().unwrap();
            let base = format!("{cwd}/target/ut_fs_{}", std::process::id());
            let _ = std::fs::remove_dir_all(&base);

            let base_c = CString::new(base.clone()).unwrap();
            assert_eq!(os_mkdir(base_c.as_ptr(), 0o700), 0);
            assert_ne!(os_mkdir(base_c.as_ptr(), 0o700), 0);
            assert!(os_isdir(base_c.as_ptr()));
            assert!(os_path_exists(base_c.as_ptr()));
            assert!(!os_path_exists(c"non-existing-file-xyz".as_ptr()));
            assert_eq!(os_getperm(c"non-existing-file-xyz".as_ptr()), UV_ENOENT);
            assert!(os_getperm(base_c.as_ptr()) > 0);
            assert_eq!(os_setperm(c"non-existing-file-xyz".as_ptr(), 0o700), FAIL);
            assert_eq!(os_file_is_writable(base_c.as_ptr()), 2);

            // os_open / os_write / os_read / os_close
            assert_eq!(
                os_open(c"non-existing-file-xyz".as_ptr(), O_RDWR, 0),
                UV_ENOENT
            );
            assert!(os_close(-1) < 0);

            let fpath = format!("{base}/test.dat");
            let fpath_c = CString::new(fpath.clone()).unwrap();
            let fd = os_open(fpath_c.as_ptr(), O_CREAT | O_RDWR, 0o600);
            assert!(fd >= 0);
            assert_eq!(
                os_open(fpath_c.as_ptr(), O_CREAT | O_EXCL, 0o600),
                if cfg!(windows) { -4075 } else { -17 }
            );

            let mut payload = Vec::with_capacity(4096);
            for _ in 0..16 {
                for b in 0..=255u8 {
                    payload.push(b);
                }
            }
            assert_eq!(os_write(fd, ptr::null(), 0, false), 0);
            assert_eq!(
                os_write(fd, payload.as_ptr().cast(), payload.len(), false),
                payload.len() as isize
            );
            assert_eq!(os_fchown(fd, uv_uid_t::MAX, uv_gid_t::MAX), 0);

            let mut fi_fd = FileInfo::new();
            assert!(!os_fileinfo_fd(-1, &raw mut fi_fd));
            assert!(os_fileinfo_fd(fd, &raw mut fi_fd));
            assert_eq!(os_fileinfo_size(&raw const fi_fd), payload.len() as u64);
            assert_eq!(os_close(fd), 0);

            assert!(os_file_is_readable(fpath_c.as_ptr()));
            assert_eq!(os_file_is_writable(fpath_c.as_ptr()), 1);

            let rfd = os_open(fpath_c.as_ptr(), O_RDONLY, 0);
            assert!(rfd >= 0);
            let mut eof = true;
            assert_eq!(os_read(rfd, &raw mut eof, ptr::null_mut(), 0, false), 0);
            assert!(!eof);

            let mut rbuf = vec![0u8; payload.len()];
            assert_eq!(
                os_read(
                    rfd,
                    &raw mut eof,
                    rbuf.as_mut_ptr().cast(),
                    rbuf.len(),
                    false
                ),
                payload.len() as isize
            );
            assert!(!eof);
            assert_eq!(rbuf, payload);
            assert_eq!(
                os_read(rfd, &raw mut eof, rbuf.as_mut_ptr().cast(), 16, false),
                0
            );
            assert!(eof);
            assert_eq!(os_close(rfd), 0);

            // os_fileinfo & os_fileid
            let mut fi = FileInfo::new();
            assert!(!os_fileinfo(ptr::null(), &raw mut fi));
            assert!(!os_fileinfo(c"/non-existent-xyz".as_ptr(), &raw mut fi));
            assert!(os_fileinfo(fpath_c.as_ptr(), &raw mut fi));
            assert!(os_fileinfo_link(fpath_c.as_ptr(), &raw mut fi));
            assert_eq!(os_fileinfo_size(&raw const fi), payload.len() as u64);
            assert!(os_fileinfo_blocksize(&raw const fi) > 0);

            let mut fid = FileID::new();
            assert!(!os_fileid(c"/non-existent-xyz".as_ptr(), &raw mut fid));
            assert!(os_fileid(fpath_c.as_ptr(), &raw mut fid));
            assert!(os_fileid_equal(&raw const fid, &raw const fid));

            // os_rename & os_remove
            let renamed = format!("{base}/renamed.dat");
            let renamed_c = CString::new(renamed.clone()).unwrap();
            assert_eq!(os_rename(fpath_c.as_ptr(), renamed_c.as_ptr()), OK);
            assert!(!os_path_exists(fpath_c.as_ptr()));
            assert!(os_path_exists(renamed_c.as_ptr()));
            assert_eq!(os_rename(fpath_c.as_ptr(), renamed_c.as_ptr()), FAIL);

            // os_mkdir_recurse & os_rmdir
            let mut failed_dir: *mut c_char = ptr::null_mut();
            let mut created_dir: *mut c_char = ptr::null_mut();
            let nested = CString::new(format!("{base}/nested/1/2///")).unwrap();
            assert_eq!(
                os_mkdir_recurse(
                    nested.as_ptr(),
                    0o700,
                    &raw mut failed_dir,
                    &raw mut created_dir
                ),
                0
            );
            assert!(failed_dir.is_null());
            let _ = cstr_take(created_dir).unwrap();

            // Cannot mkdir over an existing file
            let conflict = CString::new(format!("{renamed}/sub")).unwrap();
            assert_ne!(
                os_mkdir_recurse(
                    conflict.as_ptr(),
                    0o700,
                    &raw mut failed_dir,
                    ptr::null_mut()
                ),
                0
            );
            let _ = cstr_take(failed_dir).unwrap();

            assert_eq!(os_remove(renamed_c.as_ptr()), 0);
            assert_ne!(os_remove(renamed_c.as_ptr()), 0);

            let d2 = CString::new(format!("{base}/nested/1/2")).unwrap();
            let d1 = CString::new(format!("{base}/nested/1")).unwrap();
            let d0 = CString::new(format!("{base}/nested")).unwrap();
            assert_eq!(os_rmdir(d2.as_ptr()), 0);
            assert_eq!(os_rmdir(d1.as_ptr()), 0);
            assert_eq!(os_rmdir(d0.as_ptr()), 0);
            assert_eq!(os_rmdir(base_c.as_ptr()), 0);
            assert_ne!(os_rmdir(base_c.as_ptr()), 0);
        }
    }
}
