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
use crate::strings::gettext;
#[cfg(unix)]
use crate::strings::strerror;
use crate::ui::ui_call_chdir;
use std::ffi::{CStr, c_void};
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
    pub tv_sec: std::ffi::c_long,
    pub tv_nsec: std::ffi::c_long,
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
    pub r#type: i32,
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
pub type VimAcl = *mut c_void;
#[allow(non_camel_case_types)]
pub type uv_fs_buf_t = [u64; 80];
#[cfg(any(unix, test))]
use crate::event::r#loop::{uv_fs_fchown, uv_gid_t, uv_uid_t};

const NODE_NORMAL: i32 = 0;
pub(crate) const NODE_OTHER: i32 = 2;

const K_PATH_GENERIC: i32 = 1;
#[cfg(windows)]
const K_PATH_DRIVE: i32 = 2;
#[cfg(windows)]
const K_PATH_UNC: i32 = 3;
#[cfg(windows)]
const K_PATH_DEVICE: i32 = 4;

const UV_FS: i32 = 6;

pub(crate) const UV_EOF: i32 = -4095;
pub(crate) const UV_UNKNOWN: i32 = -4094;

pub(crate) const UV_ENOENT: i32 = if cfg!(windows) { -4058 } else { -2 };
pub(crate) const UV_EINTR: i32 = if cfg!(windows) { -4072 } else { -4 };
pub(crate) const UV_EINVAL: i32 = if cfg!(windows) { -4071 } else { -22 };
pub(crate) const UV_EIO: i32 = if cfg!(windows) { -4070 } else { -5 };
pub(crate) const UV_EROFS: i32 = if cfg!(windows) { -4043 } else { -30 };
pub(crate) const UV_ENOTSUP: i32 = if cfg!(windows) {
    -4049
} else if cfg!(target_os = "linux") {
    -95
} else {
    -45
};
pub(crate) const UV_ENOBUFS: i32 = if cfg!(windows) {
    -4060
} else if cfg!(target_os = "linux") {
    -105
} else {
    -55
};
const UV_EAGAIN: i32 = if cfg!(windows) {
    -4088
} else if cfg!(target_os = "linux") {
    -11
} else {
    -35
};

const S_IFBLK: u64 = 0o060000;
pub(crate) const S_IFLNK: u64 = 0o120000;

const R_OK: i32 = 4;
const W_OK: i32 = 2;
#[cfg(unix)]
const X_OK: i32 = 1;

pub(crate) const O_WRONLY: i32 = 1;
pub(crate) const O_RDWR: i32 = 2;
pub(crate) const O_CREAT: i32 = if cfg!(windows) {
    0x0100
} else if cfg!(target_os = "linux") {
    0o100
} else {
    0x0200
};
pub(crate) const O_TRUNC: i32 = if cfg!(any(windows, target_os = "linux")) {
    0x0200
} else {
    0x0400
};
pub(crate) const O_EXCL: i32 = if cfg!(windows) {
    0x0400
} else if cfg!(target_os = "linux") {
    0o200
} else {
    0x0800
};
pub(crate) const O_APPEND: i32 = if cfg!(target_os = "linux") {
    0o2000
} else {
    0x0008
};
pub(crate) const O_NOFOLLOW: i32 = if cfg!(windows) {
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
pub(crate) const O_BINARY: i32 = 0x8000;
pub(crate) const ENV_SEPCHAR: u8 = if cfg!(windows) { b';' } else { b':' };
#[cfg_attr(not(windows), cfg(test))]
pub(crate) const ENV_SEPSTR: &CStr = if cfg!(windows) { c";" } else { c":" };
pub(crate) const PATHSEP: u8 = b'/';
pub(crate) const PATHSEPSTR: &CStr = c"/";
#[cfg(not(windows))]
const F_GETFD: i32 = 1;
#[cfg(not(windows))]
const F_SETFD: i32 = 2;
#[cfg(not(windows))]
const FD_CLOEXEC: i32 = 1;

#[cfg(target_os = "linux")]
const E2BIG: i32 = 7;
#[cfg(target_os = "linux")]
const EACCES: i32 = 13;
#[cfg(target_os = "linux")]
const EPERM: i32 = 1;
#[cfg(target_os = "linux")]
const ERANGE: i32 = 34;
#[cfg(target_os = "linux")]
const ENOTSUP: i32 = 95;

#[repr(C)]
pub struct NvimStats {
    pub fsync: i64,
    pub redraw: i64,
    pub log_skip: i16,
}

#[repr(C)]
pub struct uv_dirent_t {
    pub name: *const u8,
    pub r#type: i32,
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
        cch: i32,
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
use crate::fileio::{EINTR, FILE, O_RDONLY, Off, S_IFDIR, S_IFMT, S_IFREG, iconv_t};
#[cfg(windows)]
use crate::memline::K_PATH_DEVICE_UNC;

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
    pub fn errno_ptr() -> *mut i32;

    #[cfg(not(windows))]
    fn readv(fd: i32, iov: *const iovec, iovcnt: i32) -> isize;

    #[cfg(target_os = "linux")]
    fn listxattr(path: *const u8, list: *mut u8, size: usize) -> isize;
    #[cfg(target_os = "linux")]
    fn getxattr(path: *const u8, name: *const u8, value: *mut c_void, size: usize) -> isize;
    #[cfg(target_os = "linux")]
    fn setxattr(
        path: *const u8,
        name: *const u8,
        value: *const c_void,
        size: usize,
        flags: i32,
    ) -> i32;

    #[link_name = "fopen"]
    pub(crate) fn c_fopen(filename: *const u8, mode: *const u8) -> *mut FILE;
    #[link_name = "freopen"]
    fn c_freopen(filename: *const u8, mode: *const u8, stream: *mut c_void) -> *mut c_void;
    pub fn fclose(stream: *mut FILE) -> i32;
    pub fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    #[link_name = "fgets"]
    pub(crate) fn c_fgets(s: *mut u8, n: i32, stream: *mut FILE) -> *mut u8;
    #[link_name = "fputs"]
    pub(crate) fn c_fputs(s: *const u8, stream: *mut FILE) -> i32;
    pub fn fgetc(stream: *mut FILE) -> i32;
    pub fn fputc(c: i32, stream: *mut FILE) -> i32;
    pub fn ungetc(c: i32, stream: *mut FILE) -> i32;
    pub fn fflush(stream: *mut FILE) -> i32;
    pub fn setbuf(stream: *mut FILE, buffer: *mut u8);
    pub fn setvbuf(stream: *mut FILE, buffer: *mut u8, mode: i32, size: usize) -> i32;
    pub fn ferror(stream: *mut FILE) -> i32;
    pub fn feof(stream: *mut FILE) -> i32;
    #[cfg(unix)]
    #[link_name = "tcdrain"]
    fn c_tcdrain(fd: i32) -> i32;
    #[cfg(unix)]
    pub fn getppid() -> i32;
    #[cfg(unix)]
    pub fn getpgid(pid: i32) -> i32;
    #[cfg(unix)]
    pub fn getpgrp() -> i32;

    #[cfg(not(windows))]
    #[link_name = "iconv_open"]
    fn c_iconv_open(tocode: *const u8, fromcode: *const u8) -> *mut c_void;
    #[cfg(not(windows))]
    pub fn iconv(
        cd: iconv_t,
        inbuf: *mut *const u8,
        inbytesleft: *mut usize,
        outbuf: *mut *mut u8,
        outbytesleft: *mut usize,
    ) -> usize;
    #[cfg(not(windows))]
    pub fn iconv_close(cd: iconv_t) -> i32;
    #[cfg_attr(windows, link_name = "_close")]
    #[cfg_attr(not(windows), link_name = "close")]
    fn c_close(fd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_dup")]
    #[cfg_attr(not(windows), link_name = "dup")]
    fn c_dup(oldfd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_dup2")]
    #[cfg_attr(not(windows), link_name = "dup2")]
    fn c_dup2(oldfd: i32, newfd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_lseeki64")]
    #[cfg_attr(not(windows), link_name = "lseek")]
    fn c_vim_lseek(fd: i32, offset: Off, whence: i32) -> Off;
    #[cfg_attr(windows, link_name = "_fseeki64")]
    #[cfg_attr(not(windows), link_name = "fseeko")]
    pub fn vim_fseek(stream: *mut FILE, offset: Off, whence: i32) -> i32;
    #[cfg_attr(windows, link_name = "_ftelli64")]
    #[cfg_attr(not(windows), link_name = "ftello")]
    pub fn vim_ftell(stream: *mut FILE) -> Off;
    #[cfg_attr(windows, link_name = "_fdopen")]
    #[cfg_attr(not(windows), link_name = "fdopen")]
    pub(crate) fn c_fdopen(fd: i32, mode: *const u8) -> *mut FILE;
    #[cfg_attr(windows, link_name = "_fileno")]
    pub fn fileno(stream: *mut FILE) -> i32;

    #[cfg(windows)]
    pub fn _get_osfhandle(fd: i32) -> isize;
    #[cfg(windows)]
    pub fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    #[cfg(windows)]
    #[link_name = "_setmode"]
    fn c_setmode(fd: i32, mode: i32) -> i32;
    #[cfg(windows)]
    #[link_name = "_getdrive"]
    fn c_getdrive() -> i32;
    #[cfg(windows)]
    pub fn _read(fd: i32, buf: *mut c_void, count: u32) -> i32;
    #[cfg(windows)]
    pub fn _write(fd: i32, buf: *const c_void, count: u32) -> i32;
    #[cfg(windows)]
    fn _umask(pmode: i32) -> i32;

    #[cfg(not(windows))]
    pub fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    #[cfg(not(windows))]
    #[link_name = "getuid"]
    fn c_getuid() -> u32;
    #[cfg(not(windows))]
    #[link_name = "getgid"]
    fn c_getgid() -> u32;
    #[cfg(not(windows))]
    pub fn read(fd: i32, buf: *mut c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    pub fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    pub fn kill(pid: i32, sig: i32) -> i32;
    #[cfg(not(windows))]
    pub fn umask(mask: u32) -> u32;
    #[cfg(not(windows))]
    pub fn readlink(path: *const u8, buf: *mut u8, bufsiz: usize) -> isize;
    #[cfg(not(windows))]
    fn symlink(target: *const u8, linkpath: *const u8) -> i32;
    #[cfg(not(windows))]
    fn opendir(name: *const u8) -> *mut c_void;
    #[cfg(not(windows))]
    fn closedir(dirp: *mut c_void) -> i32;
    #[cfg(not(windows))]
    fn dirfd(dirp: *mut c_void) -> i32;
    #[cfg(not(windows))]
    fn flock(fd: i32, operation: i32) -> i32;
}

#[inline]
pub fn close(fd: i32) -> i32 {
    unsafe { c_close(fd) }
}

#[inline]
pub fn dup(oldfd: i32) -> i32 {
    unsafe { c_dup(oldfd) }
}

#[inline]
pub fn dup2(oldfd: i32, newfd: i32) -> i32 {
    unsafe { c_dup2(oldfd, newfd) }
}

#[cfg(not(windows))]
#[inline]
pub fn os_dupfd_cloexec(fd: i32, min_fd: i32) -> i32 {
    #[cfg(target_os = "linux")]
    const F_DUPFD_CLOEXEC: i32 = 1030;
    #[cfg(target_os = "macos")]
    const F_DUPFD_CLOEXEC: i32 = 67;
    #[cfg(target_os = "freebsd")]
    const F_DUPFD_CLOEXEC: i32 = 17;
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
    const F_DUPFD_CLOEXEC: i32 = 12;
    unsafe { fcntl(fd, F_DUPFD_CLOEXEC, min_fd) }
}

#[inline]
pub fn vim_lseek(fd: i32, offset: Off, whence: i32) -> Off {
    unsafe { c_vim_lseek(fd, offset, whence) }
}

#[cfg(windows)]
#[inline]
pub fn _getdrive() -> i32 {
    unsafe { c_getdrive() }
}

#[cfg(windows)]
#[inline]
pub fn _setmode(fd: i32, mode: i32) -> i32 {
    unsafe { c_setmode(fd, mode) }
}

#[cfg(unix)]
#[inline]
pub fn tcdrain(fd: i32) -> i32 {
    unsafe { c_tcdrain(fd) }
}

#[cfg(not(windows))]
#[inline]
pub fn getuid() -> u32 {
    unsafe { c_getuid() }
}

#[cfg(not(windows))]
#[inline]
pub fn getgid() -> u32 {
    unsafe { c_getgid() }
}

#[cfg(not(windows))]
#[inline]
pub fn os_readlink(path: &CStr, buf: &mut [u8]) -> isize {
    unsafe { readlink(path.as_ptr().cast(), buf.as_mut_ptr(), buf.len()) }
}

#[cfg(not(windows))]
#[inline]
pub fn os_symlink(target: &CStr, linkpath: &CStr) -> i32 {
    unsafe { symlink(target.as_ptr().cast(), linkpath.as_ptr().cast()) }
}

#[inline]
pub fn freopen(filename: &CStr, mode: &CStr, stream: &CFile) -> bool {
    !unsafe {
        c_freopen(
            filename.as_ptr().cast(),
            mode.as_ptr().cast(),
            stream.as_nonnull().as_ptr().cast(),
        )
    }
    .is_null()
}

#[cfg(not(windows))]
#[inline]
pub fn call_iconv_open(tocode: &CStr, fromcode: &CStr) -> iconv_t {
    unsafe { c_iconv_open(tocode.as_ptr().cast(), fromcode.as_ptr().cast()) as iconv_t }
}

#[cfg(windows)]
mod win_iconv {
    use super::*;

    const E2BIG: i32 = 7;
    const EINVAL: i32 = 22;
    const EILSEQ: i32 = 42;
    const MB_ERR_INVALID_CHARS: u32 = 0x08;
    const WC_ERR_INVALID_CHARS: u32 = 0x80;
    const WC_NO_BEST_FIT_CHARS: u32 = 0x0400;

    unsafe extern "system" {
        fn IsValidCodePage(CodePage: u32) -> i32;
        fn IsDBCSLeadByteEx(CodePage: u32, TestChar: u8) -> i32;
        fn MultiByteToWideChar(
            CodePage: u32,
            dwFlags: u32,
            lpMultiByteStr: *const u8,
            cbMultiByte: i32,
            lpWideCharStr: *mut u16,
            cchWideChar: i32,
        ) -> i32;
        fn WideCharToMultiByte(
            CodePage: u32,
            dwFlags: u32,
            lpWideCharStr: *const u16,
            cchWideChar: i32,
            lpMultiByteStr: *mut u8,
            cbMultiByte: i32,
            lpDefaultChar: *const u8,
            lpUsedDefaultChar: *mut i32,
        ) -> i32;
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum WinEnc {
        Utf8,
        Utf16Le,
        Utf16Be,
        Utf16Auto,
        Ucs2Le,
        Ucs2Be,
        Ucs4Le,
        Ucs4Be,
        CodePage(u32),
    }

    struct WinIconvDesc {
        from: WinEnc,
        to: WinEnc,
        from_resolved: Option<WinEnc>,
    }

    fn parse_win_enc(name: &str) -> Option<WinEnc> {
        let s = name.trim().to_ascii_lowercase();
        let s = s.split("//").next().unwrap_or(&s);
        match s {
            "utf-8" | "utf8" => Some(WinEnc::Utf8),
            "utf-16le" | "utf16le" => Some(WinEnc::Utf16Le),
            "utf-16be" | "utf16be" => Some(WinEnc::Utf16Be),
            "utf-16" | "utf16" => Some(WinEnc::Utf16Auto),
            "ucs-2le" | "ucs2le" => Some(WinEnc::Ucs2Le),
            "ucs-2" | "ucs2" | "ucs-2be" | "ucs2be" | "unicode" => Some(WinEnc::Ucs2Be),
            "ucs-4le" | "ucs4le" | "utf-32le" | "utf32le" => Some(WinEnc::Ucs4Le),
            "ucs-4" | "ucs4" | "ucs-4be" | "ucs4be" | "utf-32" | "utf32" | "utf-32be"
            | "utf32be" => Some(WinEnc::Ucs4Be),
            "ascii" | "us-ascii" => Some(WinEnc::CodePage(20127)),
            "latin1" | "iso-8859-1" => Some(WinEnc::CodePage(28591)),
            "latin2" | "iso-8859-2" => Some(WinEnc::CodePage(28592)),
            "latin3" | "iso-8859-3" => Some(WinEnc::CodePage(28593)),
            "latin4" | "iso-8859-4" => Some(WinEnc::CodePage(28594)),
            "cyrillic" | "iso-8859-5" => Some(WinEnc::CodePage(28595)),
            "arabic" | "iso-8859-6" => Some(WinEnc::CodePage(28596)),
            "greek" | "iso-8859-7" => Some(WinEnc::CodePage(28597)),
            "hebrew" | "iso-8859-8" => Some(WinEnc::CodePage(28598)),
            "latin5" | "turkish" | "iso-8859-9" => Some(WinEnc::CodePage(28599)),
            "thai" | "iso-8859-11" => Some(WinEnc::CodePage(874)),
            "latin7" | "iso-8859-13" => Some(WinEnc::CodePage(28603)),
            "latin9" | "iso-8859-15" => Some(WinEnc::CodePage(28605)),
            "koi8-r" | "koi8r" => Some(WinEnc::CodePage(20866)),
            "koi8-u" | "koi8u" => Some(WinEnc::CodePage(21866)),
            "macroman" | "mac" => Some(WinEnc::CodePage(10000)),
            "sjis" | "shift-jis" | "shift_jis" | "cp932" | "932" => Some(WinEnc::CodePage(932)),
            "euc-jp" | "eucjp" | "ujis" | "japan" => Some(WinEnc::CodePage(20932)),
            "gbk" | "cp936" | "936" | "euc-cn" | "euccn" | "gb2312" => Some(WinEnc::CodePage(936)),
            "gb18030" | "cp54936" => Some(WinEnc::CodePage(54936)),
            "euc-kr" | "euckr" | "cp949" | "949" => Some(WinEnc::CodePage(949)),
            "big5" | "cp950" | "950" => Some(WinEnc::CodePage(950)),
            _ => {
                let digits = s.strip_prefix("cp").unwrap_or(s);
                if let Ok(cp) = digits.parse::<u32>()
                    && cp > 0
                    && unsafe { IsValidCodePage(cp) } != 0
                {
                    Some(WinEnc::CodePage(cp))
                } else {
                    None
                }
            }
        }
    }

    enum StepResult {
        Char(char, usize),
        SkipBom(usize),
        Incomplete,
        Invalid,
    }

    fn decode_one(desc: &mut WinIconvDesc, src: &[u8]) -> StepResult {
        let mut enc = desc.from;
        if enc == WinEnc::Utf16Auto {
            if let Some(res) = desc.from_resolved {
                enc = res;
            } else if src.len() >= 2 {
                if src[0] == 0xFE && src[1] == 0xFF {
                    desc.from_resolved = Some(WinEnc::Utf16Be);
                    return StepResult::SkipBom(2);
                } else if src[0] == 0xFF && src[1] == 0xFE {
                    desc.from_resolved = Some(WinEnc::Utf16Le);
                    return StepResult::SkipBom(2);
                } else {
                    desc.from_resolved = Some(WinEnc::Utf16Be);
                    enc = WinEnc::Utf16Be;
                }
            } else {
                return StepResult::Incomplete;
            }
        }

        match enc {
            WinEnc::Utf8 => match std::str::from_utf8(&src[..src.len().min(4)]) {
                Ok(s) => {
                    let ch = s.chars().next().unwrap();
                    StepResult::Char(ch, ch.len_utf8())
                }
                Err(e) => {
                    if e.valid_up_to() > 0 {
                        let s = std::str::from_utf8(&src[..e.valid_up_to()]).unwrap_or("");
                        let ch = s.chars().next().unwrap();
                        StepResult::Char(ch, ch.len_utf8())
                    } else if e.error_len().is_none() {
                        StepResult::Incomplete
                    } else {
                        StepResult::Invalid
                    }
                }
            },
            WinEnc::Utf16Le | WinEnc::Utf16Be | WinEnc::Ucs2Le | WinEnc::Ucs2Be => {
                if src.len() < 2 {
                    return StepResult::Incomplete;
                }
                let le = matches!(enc, WinEnc::Utf16Le | WinEnc::Ucs2Le);
                let is_ucs2 = matches!(enc, WinEnc::Ucs2Le | WinEnc::Ucs2Be);
                let u1 = if le {
                    u16::from_le_bytes([src[0], src[1]])
                } else {
                    u16::from_be_bytes([src[0], src[1]])
                };
                if (0xD800..=0xDBFF).contains(&u1) {
                    if is_ucs2 {
                        return StepResult::Invalid;
                    }
                    if src.len() < 4 {
                        return StepResult::Incomplete;
                    }
                    let u2 = if le {
                        u16::from_le_bytes([src[2], src[3]])
                    } else {
                        u16::from_be_bytes([src[2], src[3]])
                    };
                    if (0xDC00..=0xDFFF).contains(&u2) {
                        let cp = 0x10000 + (((u1 - 0xD800) as u32) << 10) + ((u2 - 0xDC00) as u32);
                        if let Some(ch) = char::from_u32(cp) {
                            return StepResult::Char(ch, 4);
                        }
                    }
                    StepResult::Invalid
                } else if (0xDC00..=0xDFFF).contains(&u1) {
                    StepResult::Invalid
                } else {
                    StepResult::Char(char::from_u32(u32::from(u1)).unwrap_or('\u{FFFD}'), 2)
                }
            }
            WinEnc::Ucs4Le | WinEnc::Ucs4Be => {
                if src.len() < 4 {
                    return StepResult::Incomplete;
                }
                let b = [src[0], src[1], src[2], src[3]];
                let u = if enc == WinEnc::Ucs4Le {
                    u32::from_le_bytes(b)
                } else {
                    u32::from_be_bytes(b)
                };
                match char::from_u32(u) {
                    Some(ch) => StepResult::Char(ch, 4),
                    None => StepResult::Invalid,
                }
            }
            WinEnc::CodePage(cp) => {
                let b0 = src[0];
                if cp == 20127 && b0 >= 0x80 {
                    return StepResult::Invalid;
                }
                let mut tmp = [0u8; 4];
                let (in_ptr, in_len, consumed) = if cp == 54936 {
                    if b0 < 0x80 {
                        (src.as_ptr(), 1, 1)
                    } else if (0x81..=0xFE).contains(&b0) {
                        if src.len() < 2 {
                            return StepResult::Incomplete;
                        }
                        let b1 = src[1];
                        if (0x40..=0xFE).contains(&b1) && b1 != 0x7F {
                            (src.as_ptr(), 2, 2)
                        } else if (0x30..=0x39).contains(&b1) {
                            if src.len() < 4 {
                                if src.len() == 3 && !(0x81..=0xFE).contains(&src[2]) {
                                    return StepResult::Invalid;
                                }
                                return StepResult::Incomplete;
                            }
                            if (0x81..=0xFE).contains(&src[2]) && (0x30..=0x39).contains(&src[3]) {
                                (src.as_ptr(), 4, 4)
                            } else {
                                return StepResult::Invalid;
                            }
                        } else {
                            return StepResult::Invalid;
                        }
                    } else {
                        return StepResult::Invalid;
                    }
                } else if cp == 20932 {
                    if b0 < 0x80 {
                        (src.as_ptr(), 1, 1)
                    } else if b0 == 0x8E || (0xA1..=0xFE).contains(&b0) {
                        if src.len() < 2 {
                            return StepResult::Incomplete;
                        }
                        (src.as_ptr(), 2, 2)
                    } else if b0 == 0x8F {
                        if src.len() < 3 {
                            return StepResult::Incomplete;
                        }
                        tmp[0] = src[1];
                        tmp[1] = src[2] & 0x7F;
                        (tmp.as_ptr(), 2, 3)
                    } else {
                        return StepResult::Invalid;
                    }
                } else if unsafe { IsDBCSLeadByteEx(cp, b0) } != 0 {
                    if src.len() < 2 {
                        return StepResult::Incomplete;
                    }
                    (src.as_ptr(), 2, 2)
                } else {
                    (src.as_ptr(), 1, 1)
                };

                let mut wide = [0u16; 2];
                let wlen = unsafe {
                    MultiByteToWideChar(
                        cp,
                        MB_ERR_INVALID_CHARS,
                        in_ptr.cast(),
                        in_len,
                        wide.as_mut_ptr(),
                        2,
                    )
                };
                if wlen <= 0 {
                    return StepResult::Invalid;
                }
                let mut iter = char::decode_utf16(wide[..wlen as usize].iter().copied());
                match iter.next() {
                    Some(Ok(ch)) => StepResult::Char(ch, consumed),
                    _ => StepResult::Invalid,
                }
            }
            WinEnc::Utf16Auto => unreachable!(),
        }
    }

    fn encode_one(to: WinEnc, ch: char, buf: &mut [u8; 8]) -> Option<usize> {
        match to {
            WinEnc::Utf8 => Some(ch.encode_utf8(buf).len()),
            WinEnc::Utf16Le | WinEnc::Utf16Be | WinEnc::Utf16Auto => {
                let le = to == WinEnc::Utf16Le;
                let mut w = [0u16; 2];
                let wslice = ch.encode_utf16(&mut w);
                for (i, &u) in wslice.iter().enumerate() {
                    let b = if le { u.to_le_bytes() } else { u.to_be_bytes() };
                    buf[i * 2] = b[0];
                    buf[i * 2 + 1] = b[1];
                }
                Some(wslice.len() * 2)
            }
            WinEnc::Ucs2Le | WinEnc::Ucs2Be => {
                let cp = ch as u32;
                if cp > 0xFFFF {
                    return None;
                }
                let b = if to == WinEnc::Ucs2Le {
                    (cp as u16).to_le_bytes()
                } else {
                    (cp as u16).to_be_bytes()
                };
                buf[0] = b[0];
                buf[1] = b[1];
                Some(2)
            }
            WinEnc::Ucs4Le | WinEnc::Ucs4Be => {
                let b = if to == WinEnc::Ucs4Le {
                    (ch as u32).to_le_bytes()
                } else {
                    (ch as u32).to_be_bytes()
                };
                buf[..4].copy_from_slice(&b);
                Some(4)
            }
            WinEnc::CodePage(cp) => {
                if cp == 20127 {
                    let u = ch as u32;
                    if u < 0x80 {
                        buf[0] = u as u8;
                        return Some(1);
                    }
                    return None;
                }
                let mut wide = [0u16; 2];
                let wlen = ch.encode_utf16(&mut wide).len() as i32;
                if cp == 54936 || cp == 65001 {
                    let n = unsafe {
                        WideCharToMultiByte(
                            cp,
                            WC_ERR_INVALID_CHARS,
                            wide.as_ptr(),
                            wlen,
                            buf.as_mut_ptr().cast(),
                            8,
                            ptr::null(),
                            ptr::null_mut(),
                        )
                    };
                    if n <= 0 { None } else { Some(n as usize) }
                } else {
                    let mut used_def: i32 = 0;
                    let n = unsafe {
                        WideCharToMultiByte(
                            cp,
                            WC_NO_BEST_FIT_CHARS,
                            wide.as_ptr(),
                            wlen,
                            buf.as_mut_ptr().cast(),
                            8,
                            ptr::null(),
                            &raw mut used_def,
                        )
                    };
                    if n <= 0 || used_def != 0 {
                        None
                    } else if cp == 20932 && n == 2 && (buf[0] >= 0x80 && buf[1] < 0x80) {
                        buf[2] = buf[1] | 0x80;
                        buf[1] = buf[0];
                        buf[0] = 0x8F;
                        Some(3)
                    } else {
                        Some(n as usize)
                    }
                }
            }
        }
    }

    pub fn open(tocode: &CStr, fromcode: &CStr) -> iconv_t {
        let Ok(to_str) = tocode.to_str() else {
            os_set_errno(EINVAL);
            return -1;
        };
        let Ok(from_str) = fromcode.to_str() else {
            os_set_errno(EINVAL);
            return -1;
        };
        let (Some(to), Some(from)) = (parse_win_enc(to_str), parse_win_enc(from_str)) else {
            os_set_errno(EINVAL);
            return -1;
        };
        let boxed = Box::new(WinIconvDesc {
            from,
            to,
            from_resolved: None,
        });
        Box::into_raw(boxed) as iconv_t
    }

    pub fn convert(cd: iconv_t, inbuf: Option<&mut &[u8]>, outbuf: &mut &mut [u8]) -> usize {
        if cd == -1 || cd == 0 {
            os_set_errno(EINVAL);
            return usize::MAX;
        }
        let desc = crate::globals::GraphPtr::of(cd as *mut WinIconvDesc).borrow_mut();
        let Some(inbuf) = inbuf else {
            desc.from_resolved = None;
            return 0;
        };

        while !inbuf.is_empty() {
            match decode_one(desc, inbuf) {
                StepResult::SkipBom(consumed) => {
                    *inbuf = &inbuf[consumed..];
                }
                StepResult::Char(ch, consumed) => {
                    let mut encoded = [0u8; 8];
                    let Some(enc_len) = encode_one(desc.to, ch, &mut encoded) else {
                        os_set_errno(EILSEQ);
                        return usize::MAX;
                    };
                    if enc_len > outbuf.len() {
                        os_set_errno(E2BIG);
                        return usize::MAX;
                    }
                    outbuf[..enc_len].copy_from_slice(&encoded[..enc_len]);
                    *outbuf = &mut std::mem::take(outbuf)[enc_len..];
                    *inbuf = &inbuf[consumed..];
                }
                StepResult::Incomplete => {
                    os_set_errno(EINVAL);
                    return usize::MAX;
                }
                StepResult::Invalid => {
                    os_set_errno(EILSEQ);
                    return usize::MAX;
                }
            }
        }

        0
    }

    pub fn close(cd: iconv_t) -> i32 {
        if cd != -1 && cd != 0 {
            unsafe {
                drop(Box::from_raw(cd as *mut WinIconvDesc));
            }
        }
        0
    }
}

#[cfg(windows)]
#[inline]
pub fn call_iconv_open(tocode: &CStr, fromcode: &CStr) -> iconv_t {
    win_iconv::open(tocode, fromcode)
}

#[inline]
pub fn call_iconv(cd: iconv_t, inbuf: Option<&mut &[u8]>, outbuf: &mut &mut [u8]) -> usize {
    #[cfg(windows)]
    {
        win_iconv::convert(cd, inbuf, outbuf)
    }
    #[cfg(not(windows))]
    unsafe {
        let mut out_ptr = crate::globals::GraphPtr::<u8>::of(outbuf.as_mut_ptr()).as_ptr();
        let mut out_left: usize = outbuf.len();
        let orig_out_len = out_left;
        let ret = if let Some(inbuf) = inbuf {
            let mut in_ptr = crate::globals::GraphPtr::<u8>::of(inbuf.as_ptr())
                .as_ptr()
                .cast_const();
            let mut in_left: usize = inbuf.len();
            let orig_in_len = in_left;
            let r = iconv(
                cd,
                &raw mut in_ptr,
                &raw mut in_left,
                &raw mut out_ptr,
                &raw mut out_left,
            );
            let in_used = orig_in_len.saturating_sub(in_left);
            *inbuf = &inbuf[in_used..];
            r
        } else {
            iconv(
                cd,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut out_ptr,
                &raw mut out_left,
            )
        };
        let out_used = orig_out_len.saturating_sub(out_left);
        *outbuf = &mut std::mem::take(outbuf)[out_used..];
        ret
    }
}

#[inline]
pub fn call_iconv_close(cd: iconv_t) -> i32 {
    #[cfg(windows)]
    {
        win_iconv::close(cd)
    }
    #[cfg(not(windows))]
    unsafe {
        iconv_close(cd)
    }
}

#[inline]
pub fn call_umask(mask: u32) -> u32 {
    #[cfg(windows)]
    unsafe {
        _umask(mask as i32) as u32
    }
    #[cfg(not(windows))]
    unsafe {
        umask(mask)
    }
}

/// Version of read() that retries when interrupted by EINTR.
pub fn read_eintr(fd: i32, buf: &mut [u8]) -> isize {
    loop {
        #[cfg(windows)]
        let ret = unsafe { _read(fd, buf.as_mut_ptr().cast(), buf.len() as u32) } as isize;
        #[cfg(not(windows))]
        let ret = unsafe { read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if ret >= 0 || os_errno() != EINTR {
            return ret;
        }
    }
}

/// Version of write() that retries when interrupted by EINTR.
pub fn write_eintr(fd: i32, buf: &[u8]) -> isize {
    let mut ret: isize = 0;
    let bufsize = buf.len();

    while (ret as usize) < bufsize {
        let rem = &buf[ret as usize..];
        #[cfg(windows)]
        let wlen = unsafe { _write(fd, rem.as_ptr().cast(), rem.len() as u32) } as isize;
        #[cfg(not(windows))]
        let wlen = unsafe { write(fd, rem.as_ptr().cast(), rem.len()) };
        if wlen < 0 {
            if os_errno() != EINTR {
                break;
            }
        } else {
            ret += wlen;
        }
    }
    ret
}

#[cfg(unix)]
pub struct OsLockedDir(std::ptr::NonNull<c_void>);

#[cfg(unix)]
impl OsLockedDir {
    pub fn open_shared(path: &CStr) -> Option<Self> {
        const LOCK_SH: i32 = 1;
        let dp = unsafe { opendir(path.as_ptr().cast()) };
        let nonnull = std::ptr::NonNull::new(dp)?;
        unsafe {
            flock(dirfd(nonnull.as_ptr()), LOCK_SH);
        }
        Some(Self(nonnull))
    }
}

#[cfg(unix)]
impl Drop for OsLockedDir {
    fn drop(&mut self) {
        unsafe {
            closedir(self.0.as_ptr());
        }
    }
}

#[cfg(windows)]
unsafe extern "system" {
    fn SetHandleInformation(hObject: *mut c_void, dwMask: u32, dwFlags: u32) -> i32;
    fn CoInitialize(pvReserved: *mut c_void) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: &GUID,
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

#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "macos"
))]
#[inline]
pub const fn s_ischr(m: u64) -> bool {
    (m & S_IFMT) == 0o020000
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
pub fn os_errno() -> i32 {
    unsafe { *errno_ptr() }
}

#[inline]
pub fn os_set_errno(val: i32) {
    unsafe {
        *errno_ptr() = val;
    }
}

/// Changes the current directory to `path`.
pub fn os_chdir(path: &CStr) -> i32 {
    if opt_vars().p_verbose >= 5 {
        verbose_enter();
        let _ = smsg!(0, c"chdir(%s)", path);
        verbose_leave();
    }
    let err = unsafe { uv_chdir(path.as_ptr()) };
    if err == 0 {
        ui_call_chdir(cstr_as_string(Some(path)));
    }
    err
}

/// Guess the libuv handle type (`UV_TTY`, `UV_FILE`, `UV_NAMED_PIPE`, `UV_TCP`, etc.) for `fd`.
#[inline]
pub fn os_guess_handle(fd: i32) -> i32 {
    unsafe { crate::event::r#loop::uv_guess_handle(fd) }
}

/// Get the current working directory into `buf` without NUL-termination (`uv_cwd`).
#[inline]
pub fn os_cwd(buf: &mut [u8]) -> Option<&[u8]> {
    let mut len = buf.len();
    (unsafe { uv_cwd(buf.as_mut_ptr().cast(), &mut len) } == 0).then(|| &buf[..len])
}

/// Get the name of current directory.
pub fn os_dirname(buf: &mut [u8]) -> bool {
    let mut len = buf.len();
    let error_number = unsafe { uv_cwd(buf.as_mut_ptr().cast(), &mut len) };
    if error_number != 0 {
        crate::memory::copy_bytes_to_c_buf(buf, uv_strerror(error_number).to_bytes());
        return false;
    }
    to_slash(buf);
    true
}

/// Get the current working directory as an owned `CString` (`os_dirname_cstr`).
pub fn os_dirname_cstr() -> Option<std::ffi::CString> {
    let mut cwd = [0u8; MAXPATHL];
    os_dirname(&mut cwd).then(|| crate::memory::cstring_until_nul(&cwd))
}

/// Check if the given path is a directory and not a symlink to a directory.
pub fn os_isrealdir(name: &CStr) -> bool {
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        if uv_fs_lstat(
            ptr::null_mut(),
            &raw mut request,
            name.as_ptr(),
            ptr::null(),
        ) != 0
        {
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
pub fn os_isdir(name: &CStr) -> bool {
    let mode = os_getperm(name);
    if mode < 0 {
        return false;
    }
    s_isdir(mode as u64)
}

/// Check what `name` is (`NODE_NORMAL`, `NODE_WRITABLE`, or `NODE_OTHER`).
pub fn os_nodetype(name: &CStr) -> i32 {
    #[cfg(not(windows))]
    {
        let mut statbuf = uv_stat_t::default();
        if os_stat(name, &mut statbuf) != 0 {
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
    {
        let bytes = name.to_bytes();
        if bytes.starts_with(b"\\\\.\\") {
            return NODE_WRITABLE;
        }
        let fd = os_open(name, O_RDONLY, 0);
        if fd < 0 {
            return NODE_NORMAL;
        }
        let guess = unsafe { uv_guess_handle(fd) };
        let _ = close(fd);
        match guess {
            UV_TTY => NODE_WRITABLE,
            UV_FILE => NODE_NORMAL,
            _ => NODE_OTHER,
        }
    }
}

/// Gets the absolute path of the currently running executable.
pub fn os_exepath(buffer: &mut [u8], size: &mut usize) -> i32 {
    let r = unsafe { uv_exepath(buffer.as_mut_ptr().cast(), size) };
    to_slash(buffer);
    r
}

/// Checks if the file `name` is executable.
pub fn os_can_exe(
    name: &CStr,
    abspath: Option<&mut Option<std::ffi::CString>>,
    use_path: bool,
) -> bool {
    let name_ptr = name.as_ptr();
    if !use_path || gettail_dir(name).as_ptr() != name_ptr {
        #[cfg(windows)]
        return is_executable_ext(name, abspath);
        #[cfg(not(windows))]
        return (use_path || gettail_dir(name).as_ptr() != name_ptr)
            && is_executable(name, abspath);
    }
    is_executable_in_path(name, abspath)
}

fn is_executable(name: &CStr, abspath: Option<&mut Option<std::ffi::CString>>) -> bool {
    let mode = os_getperm(name);
    if mode < 0 {
        return false;
    }

    #[cfg(windows)]
    let ok = s_isreg(mode as u64);

    #[cfg(not(windows))]
    let ok = s_isreg(mode as u64)
        && unsafe {
            let mut req: uv_fs_buf_t = [0; 80];
            let r = uv_fs_access(
                ptr::null_mut(),
                &raw mut req,
                name.as_ptr(),
                X_OK,
                ptr::null(),
            );
            uv_fs_req_cleanup((&raw mut req).cast());
            r == 0
        };

    if ok && let Some(out) = abspath {
        *out = save_abs_path(name);
    }
    ok
}

#[cfg(windows)]
fn is_executable_ext(
    name_cstr: &CStr,
    mut abspath: Option<&mut Option<std::ffi::CString>>,
) -> bool {
    let p_sh = opt_vars().p_sh.to_owned();
    let sh_tail = path_tail(&p_sh).to_bytes();
    let has_sub =
        |haystack: &[u8], needle: &[u8]| haystack.windows(needle.len()).any(|w| w == needle);
    let is_unix_shell =
        !has_sub(sh_tail, b"powershell") && !has_sub(sh_tail, b"pwsh") && has_sub(sh_tail, b"sh");

    let name_bytes = name_cstr.to_bytes();
    let nameext = name_bytes
        .iter()
        .rposition(|&b| b == b'.')
        .map_or(&b""[..], |pos| &name_bytes[pos..]);

    let copied_len =
        crate::memory::copy_bytes_to_c_buf(g_st().os_buf_bytes_mut(), name_bytes).min(MAXPATHL - 1);
    let mut pathext_buf = [0u8; MAXPATHL];
    let pathext = os_getenv_buf(c"PATHEXT", &mut pathext_buf)
        .unwrap_or(c".com;.exe;.bat;.cmd")
        .to_bytes();
    let mut ext_rem = pathext;
    while !ext_rem.is_empty() {
        if ext_rem.first() == Some(&b'.') && ext_rem.get(1).is_none_or(|&b| b == ENV_SEPCHAR) {
            if is_executable(name_cstr, abspath.as_deref_mut()) {
                return true;
            }
            ext_rem = &ext_rem[1..];
            if !ext_rem.is_empty() {
                ext_rem = &ext_rem[1..];
            }
            continue;
        }

        let ext_start = ext_rem;
        let ext_len = {
            let mut g = g_st();
            let dst = &mut g.os_buf_bytes_mut()[copied_len..MAXPATHL];
            copy_option_part(&mut ext_rem, dst, ENV_SEPSTR)
        };
        if ext_len != 0 {
            let in_pathext = nameext.len() == ext_len
                && mb_strnicmp(nameext, &ext_start[..ext_len], ext_len) == 0;
            let cand_cstr = crate::memory::cstring_until_nul(g_st().os_buf_bytes_mut());
            if ((in_pathext || is_unix_shell) && is_executable(name_cstr, abspath.as_deref_mut()))
                || is_executable(&cand_cstr, abspath.as_deref_mut())
            {
                return true;
            }
        }
    }
    false
}

#[cfg(not(windows))]
#[inline]
fn is_executable_ext(name: &CStr, abspath: Option<&mut Option<std::ffi::CString>>) -> bool {
    is_executable(name, abspath)
}

fn is_executable_in_path(
    name_cstr: &CStr,
    mut abspath: Option<&mut Option<std::ffi::CString>>,
) -> bool {
    let Some(path_env) = os_getenv_cstr(c"PATH") else {
        return false;
    };

    #[cfg(windows)]
    let path_vec: Vec<u8> = {
        let p_sh = opt_vars().p_sh.to_owned();
        let sh_tail = path_tail(&p_sh).to_bytes();
        let env_bytes = path_env.as_bytes();
        if !os_env_exists(c"NoDefaultCurrentDirectoryInExePath", false)
            && sh_tail.windows(7).any(|w| w == b"cmd.exe")
        {
            let mut v = Vec::with_capacity(env_bytes.len() + 2);
            v.push(b'.');
            v.push(b';');
            v.extend_from_slice(env_bytes);
            v
        } else {
            env_bytes.to_vec()
        }
    };
    #[cfg(windows)]
    let path_bytes: &[u8] = &path_vec;
    #[cfg(not(windows))]
    let path_bytes: &[u8] = path_env.as_bytes();

    let bufsize = name_cstr.to_bytes().len() + path_bytes.len() + 2;
    let mut buf_vec = vec![0u8; bufsize];

    let mut rem = path_bytes;
    let mut rv = false;
    loop {
        let seg_len = rem
            .iter()
            .position(|&b| b == ENV_SEPCHAR)
            .unwrap_or(rem.len());
        buf_vec[..seg_len].copy_from_slice(&rem[..seg_len]);
        buf_vec[seg_len] = 0;
        let _ = append_path(&mut buf_vec, name_cstr);

        if is_executable_ext(
            crate::memory::bytes_as_cstr(&buf_vec),
            abspath.as_deref_mut(),
        ) {
            rv = true;
            break;
        }
        if seg_len >= rem.len() {
            break;
        }
        rem = &rem[seg_len + 1..];
    }

    rv
}

/// Opens or creates a file and returns a file descriptor, or negative libuv error code.
pub fn os_open(path: &CStr, flags: i32, mode: i32) -> i32 {
    if path.to_bytes().is_empty() {
        return UV_ENOENT;
    }
    let mut req: uv_fs_buf_t = [0; 80];
    let r = unsafe {
        uv_fs_open(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
            flags,
            mode,
            ptr::null(),
        )
    };
    unsafe { uv_fs_req_cleanup((&raw mut req).cast()) };
    r
}

pub struct CFile {
    ptr: std::ptr::NonNull<crate::fileio::FILE>,
    owned: bool,
}

impl CFile {
    #[inline]
    pub fn from_raw(ptr: Option<std::ptr::NonNull<crate::fileio::FILE>>) -> Option<Self> {
        ptr.map(|ptr| Self { ptr, owned: true })
    }

    #[inline]
    pub fn from_std_stream(ptr: std::ptr::NonNull<crate::fileio::FILE>) -> Self {
        Self { ptr, owned: false }
    }

    #[inline]
    pub fn stdout() -> Self {
        unsafe extern "C" {
            #[cfg(windows)]
            fn __acrt_iob_func(ix: u32) -> *mut crate::fileio::FILE;
            #[cfg(not(windows))]
            #[cfg_attr(target_os = "linux", link_name = "stdout")]
            #[cfg_attr(not(target_os = "linux"), link_name = "__stdoutp")]
            static C_STDOUT: *mut crate::fileio::FILE;
        }
        #[cfg(windows)]
        let fp = unsafe { __acrt_iob_func(1) };
        #[cfg(not(windows))]
        let fp = unsafe { C_STDOUT };
        Self::from_std_stream(std::ptr::NonNull::new(fp).unwrap())
    }

    #[inline]
    pub fn stderr() -> Self {
        unsafe extern "C" {
            #[cfg(windows)]
            fn __acrt_iob_func(ix: u32) -> *mut crate::fileio::FILE;
            #[cfg(not(windows))]
            #[cfg_attr(target_os = "linux", link_name = "stderr")]
            #[cfg_attr(not(target_os = "linux"), link_name = "__stderrp")]
            static C_STDERR: *mut crate::fileio::FILE;
        }
        #[cfg(windows)]
        let fp = unsafe { __acrt_iob_func(2) };
        #[cfg(not(windows))]
        let fp = unsafe { C_STDERR };
        Self::from_std_stream(std::ptr::NonNull::new(fp).unwrap())
    }

    #[inline]
    pub fn open(path: &CStr, flags: &std::ffi::CStr) -> Option<Self> {
        Self::from_raw(os_fopen(path, flags))
    }

    #[inline]
    pub fn open_libc(path: &CStr, mode: &std::ffi::CStr) -> Option<Self> {
        Self::from_raw(std::ptr::NonNull::new(unsafe {
            crate::fileio::c_fopen(path.as_ptr().cast(), mode.as_ptr().cast())
        }))
    }

    #[inline]
    pub fn from_fd(fd: i32, mode: &std::ffi::CStr) -> Option<Self> {
        Self::from_raw(std::ptr::NonNull::new(unsafe {
            crate::fileio::c_fdopen(fd, mode.as_ptr().cast())
        }))
    }

    #[inline]
    pub fn as_nonnull(&self) -> std::ptr::NonNull<crate::fileio::FILE> {
        self.ptr
    }

    #[inline]
    pub fn close(self) -> i32 {
        let this = std::mem::ManuallyDrop::new(self);
        if this.owned {
            unsafe { crate::fileio::fclose(this.ptr.as_ptr()) }
        } else {
            0
        }
    }

    #[inline]
    pub fn fileno(&self) -> i32 {
        unsafe { crate::fileio::fileno(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn flush(&self) -> i32 {
        unsafe { crate::fileio::fflush(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn setvbuf(&self, buf: &mut [u8], mode: i32) -> i32 {
        unsafe {
            crate::fileio::setvbuf(self.ptr.as_ptr(), buf.as_mut_ptr().cast(), mode, buf.len())
        }
    }

    #[inline]
    pub fn set_unbuffered(&self) {
        unsafe { crate::fileio::setbuf(self.ptr.as_ptr(), std::ptr::null_mut()) };
    }

    #[inline]
    pub fn is_eof(&self) -> bool {
        unsafe { crate::fileio::feof(self.ptr.as_ptr()) != 0 }
    }

    #[inline]
    pub fn has_error(&self) -> bool {
        unsafe { crate::fileio::ferror(self.ptr.as_ptr()) != 0 }
    }

    #[inline]
    pub fn ferror(&self) -> i32 {
        unsafe { crate::fileio::ferror(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn getc(&self) -> i32 {
        unsafe { crate::fileio::fgetc(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn ungetc(&self, c: i32) -> i32 {
        unsafe { crate::fileio::ungetc(c, self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn putc(&self, c: i32) -> i32 {
        unsafe { crate::fileio::fputc(c, self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn puts(&self, s: &CStr) -> i32 {
        unsafe { crate::fileio::c_fputs(s.as_ptr().cast(), self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn gets(&self, buf: &mut [u8]) -> bool {
        !unsafe { crate::fileio::c_fgets(buf.as_mut_ptr(), buf.len() as i32, self.ptr.as_ptr()) }
            .is_null()
    }

    #[inline]
    pub fn seek(&self, offset: crate::fileio::Off, whence: i32) -> i32 {
        unsafe { crate::fileio::vim_fseek(self.ptr.as_ptr(), offset, whence) }
    }

    #[inline]
    pub fn tell(&self) -> crate::fileio::Off {
        unsafe { crate::fileio::vim_ftell(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn write_all(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        unsafe {
            crate::fileio::fwrite(bytes.as_ptr().cast(), bytes.len(), 1, self.ptr.as_ptr()) == 1
        }
    }

    #[inline]
    pub fn read_bytes(&self, buf: &mut [u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }
        unsafe { crate::fileio::fread(buf.as_mut_ptr().cast(), 1, buf.len(), self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn read_exact(&self, buf: &mut [u8]) -> bool {
        if buf.is_empty() {
            return true;
        }
        unsafe {
            crate::fileio::fread(buf.as_mut_ptr().cast(), buf.len(), 1, self.ptr.as_ptr()) == 1
        }
    }
}

impl Drop for CFile {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                crate::fileio::fclose(self.ptr.as_ptr());
            }
        }
    }
}

/// Compatibility wrapper conforming to `fopen(3)`.
fn os_fopen(path: &CStr, flags: &std::ffi::CStr) -> Option<std::ptr::NonNull<crate::fileio::FILE>> {
    let fbytes = flags.to_bytes();
    assert!((1..=2).contains(&fbytes.len()));

    let f0 = fbytes[0];
    let f1 = fbytes.get(1).copied().unwrap_or(0);
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
        return None;
    }
    std::ptr::NonNull::new(unsafe { c_fdopen(fd, flags.as_ptr().cast()) })
}

/// Sets file descriptor `fd` to close-on-exec (Unix) or non-inheritable (Windows).
pub fn os_set_cloexec(fd: i32) -> i32 {
    #[cfg(not(windows))]
    unsafe {
        let fdflags = fcntl(fd, F_GETFD);
        if fdflags < 0 {
            let e = *errno_ptr();
            let _ = logmsg!(
                4,
                None,
                Some(c"os_set_cloexec"),
                line!() as i32,
                true,
                c"Failed to get flags on descriptor %d: %s",
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
                None,
                Some(c"os_set_cloexec"),
                line!() as i32,
                true,
                c"Failed to set CLOEXEC on descriptor %d: %s",
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
pub fn os_close(fd: i32) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_close(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Duplicate file descriptor.
pub fn os_dup(fd: i32) -> i32 {
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
pub fn os_dup_cloexec(fd: i32) -> i32 {
    let newfd = os_dup(fd);
    if newfd >= 0 {
        let _ = os_set_cloexec(newfd);
    }
    newfd
}

/// Open the file descriptor for stdin.
pub fn os_open_stdin_fd() -> i32 {
    if g_st().stdin_fd > 0 {
        g_st().stdin_fd
    } else {
        let fd = os_dup(STDIN_FILENO);
        #[cfg(windows)]
        os_redirect_stdin_to_conin();
        fd
    }
}

/// Read from a file.
pub fn os_read(fd: i32, ret_eof: &mut bool, ret_buf: &mut [u8], non_blocking: bool) -> isize {
    *ret_eof = false;
    let size = ret_buf.len();
    if size == 0 {
        return 0;
    }
    let mut read_bytes: usize = 0;
    while read_bytes != size {
        assert!(size >= read_bytes);
        #[cfg(windows)]
        let cur_read_bytes = unsafe {
            _read(
                fd,
                ret_buf.as_mut_ptr().wrapping_add(read_bytes).cast(),
                (size - read_bytes) as u32,
            )
        } as isize;
        #[cfg(not(windows))]
        let cur_read_bytes = unsafe {
            read(
                fd,
                ret_buf.as_mut_ptr().wrapping_add(read_bytes).cast(),
                size - read_bytes,
            )
        };

        if cur_read_bytes > 0 {
            read_bytes += cur_read_bytes as usize;
        }
        if cur_read_bytes < 0 {
            let error = unsafe {
                let ep = errno_ptr();
                let err = uv_translate_sys_error(*ep);
                *ep = 0;
                err
            };
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

#[cfg(unix)]
pub fn os_readv_two_slices(
    fd: i32,
    ret_eof: &mut bool,
    buf1: &mut [u8],
    buf2: &mut [u8],
    non_blocking: bool,
) -> isize {
    let mut iov = [
        iovec {
            iov_base: buf1.as_mut_ptr().cast::<c_void>(),
            iov_len: buf1.len(),
        },
        iovec {
            iov_base: buf2.as_mut_ptr().cast::<c_void>(),
            iov_len: buf2.len(),
        },
    ];
    os_readv(fd, ret_eof, &mut iov, non_blocking)
}

/// Read from a file to multiple buffers at once.
#[cfg(unix)]
pub fn os_readv(fd: i32, ret_eof: &mut bool, iov_slice: &mut [iovec], non_blocking: bool) -> isize {
    let mut iov = iov_slice.as_mut_ptr();
    let mut iov_size = iov_slice.len();
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
                let mut cur_read_bytes = readv(fd, iov, iov_size as i32);
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
pub fn os_write(fd: i32, buf: &[u8], non_blocking: bool) -> isize {
    let size = buf.len();
    if size == 0 {
        return 0;
    }
    let mut written_bytes: usize = 0;
    while written_bytes != size {
        assert!(size >= written_bytes);
        #[cfg(windows)]
        let cur_written_bytes = unsafe {
            _write(
                fd,
                buf.as_ptr().wrapping_add(written_bytes).cast(),
                (size - written_bytes) as u32,
            )
        } as isize;
        #[cfg(not(windows))]
        let cur_written_bytes = unsafe {
            write(
                fd,
                buf.as_ptr().wrapping_add(written_bytes).cast(),
                size - written_bytes,
            )
        };

        if cur_written_bytes > 0 {
            written_bytes += cur_written_bytes as usize;
        }
        if cur_written_bytes < 0 {
            let error = unsafe {
                let ep = errno_ptr();
                let err = uv_translate_sys_error(*ep);
                *ep = 0;
                err
            };
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

/// Copies a file from `path` to `new_path`.
pub fn os_copy(path: &CStr, new_path: &CStr, flags: i32) -> i32 {
    let mut req: uv_fs_buf_t = [0; 80];
    let r = unsafe {
        uv_fs_copyfile(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
            new_path.as_ptr(),
            flags,
            ptr::null(),
        )
    };
    unsafe { uv_fs_req_cleanup((&raw mut req).cast()) };
    r
}

/// Flushes file modifications to disk.
pub fn os_fsync(fd: i32) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fsync(ptr::null_mut(), &raw mut req, fd, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        g_st().g_stats.fsync += 1;
        r
    }
}

fn os_stat(name: &CStr, statbuf: &mut uv_stat_t) -> i32 {
    if name.to_bytes().is_empty() {
        return UV_ENOENT;
    }
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let result = uv_fs_stat(
            ptr::null_mut(),
            &raw mut request,
            name.as_ptr(),
            ptr::null(),
        );
        if result == 0 {
            *statbuf = *uv_fs_get_statbuf(&raw mut request);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        result
    }
}

/// Get the file permissions for a given file.
pub fn os_getperm(name: &CStr) -> i32 {
    let mut statbuf = uv_stat_t::default();
    let stat_result = os_stat(name, &mut statbuf);
    if stat_result == 0 {
        statbuf.st_mode as i32
    } else {
        stat_result
    }
}

/// Set the permission of a file.
pub fn os_setperm(name: &CStr, perm: i32) -> bool {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_chmod(
            ptr::null_mut(),
            &raw mut req,
            name.as_ptr(),
            perm,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r == 0
    }
}

/// Set the permission of the file referred to by the open file descriptor.
#[cfg(unix)]
pub fn os_fsetperm(fd: i32, perm: i32) -> bool {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fchmod(ptr::null_mut(), &raw mut req, fd, perm, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r == 0
    }
}

/// Copy extended attributes `from_file` to `to_file`.
pub fn os_copy_xattr(from_file_opt: Option<&CStr>, to_file_opt: Option<&CStr>) {
    #[cfg(not(target_os = "linux"))]
    let _ = (from_file_opt, to_file_opt);
    #[cfg(target_os = "linux")]
    unsafe {
        let Some(from_file) = from_file_opt else {
            return;
        };
        let to_file_ptr = to_file_opt.map_or(ptr::null(), CStr::as_ptr).cast::<u8>();
        let mut size = listxattr(from_file.as_ptr().cast(), ptr::null_mut(), 0);
        if size <= 0 {
            return;
        }
        let mut xattr_buf = vec![0u8; size as usize];
        size = listxattr(
            from_file.as_ptr().cast(),
            xattr_buf.as_mut_ptr(),
            size as usize,
        );
        let tsize = size;

        let ep = errno_ptr();
        *ep = 0;

        let mut max_vallen: isize = 0;
        let mut val_buf: Vec<u8> = Vec::new();
        let mut errmsg: Option<&CStr> = None;

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
                    from_file.as_ptr().cast(),
                    key,
                    val_ptr.cast(),
                    if round != 0 { max_vallen as usize } else { 0 },
                );
                if vallen >= 0
                    && round != 0
                    && setxattr(to_file_ptr, key, val_ptr.cast(), vallen as usize, 0) == 0
                {
                } else if *ep != 0 {
                    match *ep {
                        E2BIG => {
                            errmsg = Some(c"E1508: Size of the extended attribute value is larger than the maximum size allowed");
                            break 'outer;
                        }
                        ENOTSUP | EACCES | EPERM => {}
                        ERANGE => {
                            errmsg = Some(c"E1506: Buffer too small to copy xattr value or key");
                            break 'outer;
                        }
                        _ => {
                            errmsg = Some(
                                c"E1509: Error occurred when reading or writing extended attribute",
                            );
                            break 'outer;
                        }
                    }
                }

                if round == 0 && vallen > max_vallen {
                    max_vallen = vallen;
                }

                let keylen = (CStr::from_ptr(key.cast()).to_bytes().len() + 1) as isize;
                size -= keylen;
                key = key.add(keylen as usize);
            }
            if round != 0 {
                break;
            }
            val_buf = vec![0u8; (max_vallen as usize) + 1];
        }

        if let Some(errmsg) = errmsg {
            emsg(gettext(errmsg));
        }
    }
}

pub fn os_get_acl(_fname: &CStr) -> VimAcl {
    ptr::null_mut()
}

pub fn os_set_acl(_fname: &CStr, _aclent: VimAcl) {}

pub fn os_free_acl(_aclent: VimAcl) {}

/// Checks if the current user owns a file.
pub fn os_file_owned(fname: &CStr) -> bool {
    #[cfg(not(windows))]
    {
        let uid = u64::from(getuid());
        let mut finfo = FileInfo::new();
        let file_owned = os_fileinfo(fname, &mut finfo) && finfo.stat.st_uid == uid;
        let link_owned = os_fileinfo_link(fname, &mut finfo) && finfo.stat.st_uid == uid;
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
pub fn os_chown(path: &CStr, owner: uv_uid_t, group: uv_gid_t) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_chown(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
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
pub fn os_fchown(fd: i32, owner: uv_uid_t, group: uv_gid_t) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_fchown(ptr::null_mut(), &raw mut req, fd, owner, group, ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Check if a path exists.
pub fn os_path_exists(path: &CStr) -> bool {
    let mut statbuf = uv_stat_t::default();
    os_stat(path, &mut statbuf) == 0
}

/// Sets file access and modification times.
#[cfg(unix)]
pub fn os_file_settime(path: &CStr, atime: f64, mtime: f64) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_utime(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
            atime,
            mtime,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Check if a file is readable.
pub fn os_file_is_readable(name: &CStr) -> bool {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_access(
            ptr::null_mut(),
            &raw mut req,
            name.as_ptr(),
            R_OK,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r == 0
    }
}

/// Check if a file is writable (`0` = not writable, `1` = writable file, `2` = writable dir).
pub fn os_file_is_writable(name: &CStr) -> i32 {
    let mut req: uv_fs_buf_t = [0; 80];
    let r = unsafe {
        uv_fs_access(
            ptr::null_mut(),
            &raw mut req,
            name.as_ptr(),
            W_OK,
            ptr::null(),
        )
    };
    unsafe { uv_fs_req_cleanup((&raw mut req).cast()) };
    if r == 0 {
        if os_isdir(name) { 2 } else { 1 }
    } else {
        0
    }
}

/// Rename a file or directory.
pub fn os_rename(path: &CStr, new_path: &CStr) -> bool {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut r = uv_fs_rename(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
            new_path.as_ptr(),
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        #[cfg(windows)]
        {
            const UV_EPERM: i32 = -4048;
            const UV_EBUSY: i32 = -4082;
            let mut ms: u64 = 1;
            while (r == UV_EPERM || r == UV_EBUSY) && ms <= 64 {
                crate::os::time::os_sleep(ms);
                r = uv_fs_rename(
                    ptr::null_mut(),
                    &raw mut req,
                    path.as_ptr(),
                    new_path.as_ptr(),
                    ptr::null(),
                );
                uv_fs_req_cleanup((&raw mut req).cast());
                ms *= 4;
            }
        }
        r == 0
    }
}

/// Make a directory.
pub fn os_mkdir(path: &CStr, mode: i32) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_mkdir(
            ptr::null_mut(),
            &raw mut req,
            path.as_ptr(),
            mode,
            ptr::null(),
        );
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Make a directory, with higher levels when needed.
pub fn os_mkdir_recurse(
    dir: &CStr,
    mode: i32,
    failed_dir: &mut Option<std::ffi::CString>,
    mut created: Option<&mut Option<std::ffi::CString>>,
) -> i32 {
    let dirlen = dir.to_bytes().len();
    let mut curdir_vec: Vec<u8> = dir.to_bytes_with_nul().to_vec();
    let past_head_idx = (get_past_head(dir).as_ptr() as usize) - (dir.as_ptr() as usize);
    let mut e_idx = dirlen;
    let past_head_save = curdir_vec[past_head_idx];

    while !os_isdir(crate::memory::bytes_as_cstr(&curdir_vec)) {
        let cur_cstr = crate::memory::bytes_as_cstr(&curdir_vec);
        e_idx = (path_tail_with_sep(cur_cstr).as_ptr() as usize) - (cur_cstr.as_ptr() as usize);
        if e_idx <= past_head_idx {
            curdir_vec[past_head_idx] = 0;
            break;
        }
        curdir_vec[e_idx] = 0;
    }
    while e_idx < dirlen {
        if e_idx > past_head_idx {
            curdir_vec[e_idx] = PATHSEP;
        } else {
            curdir_vec[past_head_idx] = past_head_save;
        }
        let component_len = crate::memory::bytes_until_nul(&curdir_vec[e_idx..]).len();
        e_idx += component_len;
        if e_idx == dirlen
            && component_len > 0
            && curdir_vec[e_idx - component_len..e_idx]
                .iter()
                .all(|&b| b == PATHSEP)
        {
            break;
        }
        let curdir_cstr = crate::memory::bytes_as_cstr(&curdir_vec);
        let ret = os_mkdir(curdir_cstr, mode);
        if ret != 0 {
            *failed_dir = Some(curdir_cstr.to_owned());
            return ret;
        } else if let Some(ref mut c_out) = created
            && c_out.is_none()
        {
            **c_out = FullName_save(Some(curdir_cstr), false);
        }
    }
    0
}

/// Create the parent directory of a file if it does not exist.
pub fn os_file_mkdir(fname_in: &CStr, mode: i32) -> i32 {
    let mut fname_vec = fname_in.to_bytes_with_nul().to_vec();
    if !dir_of_file_exists(fname_in) {
        let tail_cstr = path_tail_with_sep(fname_in);
        let tail_off = (tail_cstr.as_ptr() as usize) - (fname_in.as_ptr() as usize);
        if fname_vec.len() >= 2 && vim_ispathsep(i32::from(fname_vec[fname_vec.len() - 2])) {
            emsg(gettext(E_NONAME));
            return -1;
        }
        let c = fname_vec[tail_off];
        fname_vec[tail_off] = (NUL) as u8;
        let mut failed_dir: Option<std::ffi::CString> = None;
        let r = os_mkdir_recurse(
            crate::memory::bytes_as_cstr(&fname_vec),
            mode,
            &mut failed_dir,
            None,
        );
        if r < 0 {
            let _ = semsg!(gettext(E_MKDIR), failed_dir.as_deref(), uv_strerror(r));
        }
        fname_vec[tail_off] = c;
        return r;
    }
    0
}

/// Create a unique temporary directory.
pub fn os_mkdtemp(templ: &CStr, path: &mut [u8]) -> i32 {
    let mut request: uv_fs_buf_t = [0; 80];
    let result = unsafe {
        uv_fs_mkdtemp(
            ptr::null_mut(),
            &raw mut request,
            templ.as_ptr(),
            ptr::null(),
        )
    };
    if result == 0 {
        let res_path = unsafe { uv_fs_get_path(&raw const request) };
        let res_bytes = if res_path.is_null() {
            &[][..]
        } else {
            unsafe { CStr::from_ptr(res_path).to_bytes() }
        };
        crate::memory::copy_bytes_to_c_buf(path, res_bytes);
        to_slash(path);
    }
    unsafe { uv_fs_req_cleanup((&raw mut request).cast()) };
    result
}

/// Remove a directory.
pub fn os_rmdir(path: &CStr) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_rmdir(ptr::null_mut(), &raw mut req, path.as_ptr(), ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Opens a directory.
pub fn os_scandir(dir: &mut Directory, path: &CStr) -> bool {
    debug_assert!(
        unsafe { uv_req_size(UV_FS) } + size_of::<uv_dirent_t>() <= size_of::<Directory>()
    );
    let req = (&raw mut *dir).cast::<uv_fs_buf_t>();
    let r = unsafe { uv_fs_scandir(ptr::null_mut(), req, path.as_ptr(), 0, ptr::null()) };
    if r < 0 {
        os_closedir(dir);
    }
    r >= 0
}

/// Increments the directory pointer.
pub fn os_scandir_next(dir_in: &mut Directory) -> Option<&'static CStr> {
    let dir = crate::globals::GraphPtr::<Directory>::from_ref(dir_in);
    let req = dir.cast::<uv_fs_buf_t>();
    let ent = dir
        .cast::<u8>()
        .wrapping_add(unsafe { uv_req_size(UV_FS) })
        .cast::<uv_dirent_t>();
    let err = unsafe { uv_fs_scandir_next(req, ent) };
    if err != UV_EOF {
        let name = crate::globals::GraphPtr::of(ent).borrow().name;
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name.cast()) })
    } else {
        None
    }
}

/// Frees memory associated with `os_scandir()`.
pub fn os_closedir(dir: &mut Directory) {
    unsafe {
        uv_fs_req_cleanup((&raw mut *dir).cast());
    }
}

/// Remove a file.
pub fn os_remove(path: &CStr) -> i32 {
    unsafe {
        let mut req: uv_fs_buf_t = [0; 80];
        let r = uv_fs_unlink(ptr::null_mut(), &raw mut req, path.as_ptr(), ptr::null());
        uv_fs_req_cleanup((&raw mut req).cast());
        r
    }
}

/// Get the file information for a given path.
pub fn os_fileinfo(path: &CStr, file_info: &mut FileInfo) -> bool {
    *file_info = FileInfo::new();
    os_stat(path, &mut file_info.stat) == 0
}

/// Parses `path` into a `FileInfo` structure.
pub fn os_fileinfo2(path: &CStr, info: &mut FileInfo) -> bool {
    *info = FileInfo::new();
    if path_with_url(path.to_bytes()) {
        return true;
    }
    #[cfg(windows)]
    let mut p = path_skip_sep(path, false);
    #[cfg(not(windows))]
    let p = path_skip_sep(path, false);
    let path_len = path.to_bytes().len();
    let leading_slashes = path_len - p.to_bytes().len();

    #[cfg(windows)]
    {
        let p_bytes = p.to_bytes();
        let p0 = p_bytes.first().copied().unwrap_or(0);
        let p1 = p_bytes.get(1).copied().unwrap_or(0);
        if leading_slashes == 0 && p0.is_ascii_alphabetic() && p1 == b':' {
            info.r#type = K_PATH_DRIVE;
            p = path_skip_sep(&p[2..], false);
            info.rest_off = path_len - p.to_bytes().len();
            return true;
        }
        if leading_slashes >= 2 && (p0 == b'?' || p0 == b'.') {
            if !vim_ispathsep_nocolon(i32::from(p1)) {
                return true;
            }
            info.r#type = K_PATH_DEVICE;
            info.prefix_off = leading_slashes - 2;
            p = path_skip_sep(&p[2..], false);
            let p_bytes = p.to_bytes();
            if p_bytes
                .get(..3)
                .is_some_and(|s| s.eq_ignore_ascii_case(b"unc"))
                && p_bytes
                    .get(3)
                    .is_some_and(|&b| vim_ispathsep_nocolon(i32::from(b)))
            {
                info.r#type = K_PATH_DEVICE_UNC;
                p = path_skip_sep(&p[4..], false);
                info.root_off = path_len - p.to_bytes().len();
                p = path_skip_sep(path_next_component(p), false);
                p = path_skip_sep(path_next_component(p), false);
                info.rest_off = path_len - p.to_bytes().len();
                return true;
            }
            info.root_off = path_len - p.to_bytes().len();
            if p_bytes.first().is_some_and(u8::is_ascii_alphabetic) && p_bytes.get(1) == Some(&b':')
            {
                p = &p[2..];
            }
            p = path_skip_sep(path_next_component(p), false);
            info.rest_off = path_len - p.to_bytes().len();
            return true;
        }
        if leading_slashes == 2 {
            info.r#type = K_PATH_UNC;
            p = path_skip_sep(path_next_component(p), false);
            p = path_skip_sep(path_next_component(p), false);
            info.rest_off = path_len - p.to_bytes().len();
            return true;
        }
    }

    info.r#type = K_PATH_GENERIC;
    info.rest_off = leading_slashes;
    info.root_off = if leading_slashes > 2 {
        leading_slashes - 1
    } else {
        0
    };
    info.prefix_off = info.root_off;
    true
}

/// Get the file information for a given path without following links.
pub fn os_fileinfo_link(path: &CStr, file_info: &mut FileInfo) -> bool {
    *file_info = FileInfo::new();
    if path.to_bytes().is_empty() {
        return false;
    }
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let ok = uv_fs_lstat(
            ptr::null_mut(),
            &raw mut request,
            path.as_ptr(),
            ptr::null(),
        ) == 0;
        if ok {
            file_info.stat = *uv_fs_get_statbuf(&raw mut request);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        ok
    }
}

/// Get the file information for a given file descriptor.
pub fn os_fileinfo_fd(file_descriptor: i32, file_info: &mut FileInfo) -> bool {
    *file_info = FileInfo::new();
    {
        let mut request: uv_fs_buf_t = [0; 80];
        let ok = unsafe {
            uv_fs_fstat(
                ptr::null_mut(),
                &raw mut request,
                file_descriptor,
                ptr::null(),
            )
        } == 0;
        if ok {
            file_info.stat = unsafe { *uv_fs_get_statbuf(&raw mut request) };
        }
        unsafe { uv_fs_req_cleanup((&raw mut request).cast()) };
        ok
    }
}

/// Compare the inodes of two `FileInfo`s.
pub fn os_fileinfo_id_equal(file_info_1: &FileInfo, file_info_2: &FileInfo) -> bool {
    file_info_1.stat.st_ino == file_info_2.stat.st_ino
        && file_info_1.stat.st_dev == file_info_2.stat.st_dev
}

/// Get the `FileID` of a `FileInfo`.
#[cfg(test)]
pub fn os_fileinfo_id(file_info: &FileInfo, file_id: &mut FileID) {
    file_id.inode = file_info.stat.st_ino;
    file_id.device_id = file_info.stat.st_dev;
}

/// Get the inode of a `FileInfo`.
pub fn os_fileinfo_inode(file_info: &FileInfo) -> u64 {
    file_info.stat.st_ino
}

/// Get the size of a file from a `FileInfo`.
pub fn os_fileinfo_size(file_info: &FileInfo) -> u64 {
    file_info.stat.st_size
}

/// Get the number of hardlinks from a `FileInfo`.
pub fn os_fileinfo_hardlinks(file_info: &FileInfo) -> u64 {
    file_info.stat.st_nlink
}

/// Get the blocksize from a `FileInfo`.
pub fn os_fileinfo_blocksize(file_info: &FileInfo) -> u64 {
    file_info.stat.st_blksize
}

/// Get the `FileID` for a given path.
pub fn os_fileid(path: &CStr, file_id: &mut FileID) -> bool {
    let mut statbuf = uv_stat_t::default();
    if os_stat(path, &mut statbuf) == 0 {
        file_id.inode = statbuf.st_ino;
        file_id.device_id = statbuf.st_dev;
        true
    } else {
        false
    }
}

/// Check if two `FileID`s are equal.
pub fn os_fileid_equal(file_id_1: &FileID, file_id_2: &FileID) -> bool {
    file_id_1.inode == file_id_2.inode && file_id_1.device_id == file_id_2.device_id
}

/// Return the canonicalized absolute pathname.
pub fn os_realpath<'a>(name: &CStr, buf: &'a mut [u8]) -> Option<&'a CStr> {
    unsafe {
        let mut request: uv_fs_buf_t = [0; 80];
        let result = uv_fs_realpath(
            ptr::null_mut(),
            &raw mut request,
            name.as_ptr(),
            ptr::null(),
        );
        if result == 0 {
            let res_ptr = uv_fs_get_ptr(&raw const request).cast::<std::ffi::c_char>();
            let res_bytes = if res_ptr.is_null() {
                &[][..]
            } else {
                CStr::from_ptr(res_ptr).to_bytes()
            };
            crate::memory::copy_bytes_to_c_buf(buf, res_bytes);
            to_slash(buf);
        }
        uv_fs_req_cleanup((&raw mut request).cast());
        (result == 0).then(|| crate::memory::bytes_as_cstr(buf))
    }
}

#[cfg(windows)]
pub fn os_realpath_cstr(name: &CStr, len: usize) -> Option<std::ffi::CString> {
    let mut buf = vec![0u8; len];
    os_realpath(name, &mut buf).map(CStr::to_owned)
}

#[cfg(windows)]
pub fn os_resolve_shortcut(fname_opt: Option<&CStr>) -> Option<std::ffi::CString> {
    let fname = fname_opt?;
    #[cfg(windows)]
    unsafe {
        let bytes = fname.to_bytes();
        if bytes.len() <= 4 || !bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".lnk") {
            return None;
        }

        let mut rfname: Option<std::ffi::CString> = None;
        let mut pslw: *mut IShellLinkW = ptr::null_mut();
        let mut ppf: *mut IPersistFile = ptr::null_mut();

        let _ = CoInitialize(ptr::null_mut());

        let hr = CoCreateInstance(
            &CLSID_SHELL_LINK,
            ptr::null_mut(),
            1, // CLSCTX_INPROC_SERVER
            &raw const IID_ISHELL_LINK_W,
            (&raw mut pslw).cast(),
        );
        if hr == 0 {
            let mut p: Vec<u16> = Vec::new();
            let r = utf8_to_utf16(bytes, -1, &mut p);
            if r != 0 {
                semsg!(c"utf8_to_utf16 failed: %d", r);
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
                            let mut rfname_cstr: Option<std::ffi::CString> = None;
                            let r2 = utf16_to_utf8(&wsz, -1, &mut rfname_cstr);
                            if r2 != 0 {
                                semsg!(c"utf16_to_utf8 failed: %d", r2);
                            } else if let Some(cstr) = rfname_cstr {
                                let mut v = cstr.into_bytes_with_nul();
                                to_slash(&mut v);
                                rfname = Some(std::ffi::CString::from_vec_with_nul(v).unwrap());
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
        rfname
    }
    #[cfg(not(windows))]
    {
        let _ = fname;
        None
    }
}

#[cfg(windows)]
pub fn os_is_reparse_point_include(path: &CStr) -> bool {
    #[cfg(windows)]
    {
        let mut utf16_vec: Vec<u16> = Vec::new();
        let r = utf8_to_utf16(path.to_bytes(), -1, &mut utf16_vec);
        if r != 0 {
            semsg!(c"utf8_to_utf16 failed: %d", r);
            return false;
        }
        let utf16_path = utf16_vec.as_ptr();

        let is_sep = |c: u16| c == (b'\\' as u16) || c == (b'/' as u16);
        let mut p = utf16_path;
        let p0 = (*crate::globals::GraphPtr::of(p).borrow() & 0xff) as u8;
        if p0.is_ascii_alphabetic()
            && *crate::globals::GraphPtr::of(p.wrapping_add(1)).borrow() == (b':' as u16)
            && is_sep(*crate::globals::GraphPtr::of(p.wrapping_add(2)).borrow())
        {
            p = p.wrapping_add(3);
        } else if is_sep(*crate::globals::GraphPtr::of(p).borrow())
            && is_sep(*crate::globals::GraphPtr::of(p.wrapping_add(1)).borrow())
        {
            p = p.wrapping_add(2);
        }

        let mut buf = [0u16; 260];
        let mut result = false;

        while *crate::globals::GraphPtr::of(p).borrow() != 0 {
            let mut q = p;
            while *crate::globals::GraphPtr::of(q).borrow() != 0
                && !is_sep(*crate::globals::GraphPtr::of(q).borrow())
            {
                q = q.wrapping_add(1);
            }
            if *crate::globals::GraphPtr::of(q).borrow() == 0 {
                p = q;
            } else {
                p = q.wrapping_add(1);
            }
            let len = ((q as usize) - (utf16_path as usize)) / size_of::<u16>();
            if len >= 260 {
                break;
            }
            crate::globals::GraphPtr::copy_to(utf16_path, buf.as_mut_ptr(), len);
            buf[len] = 0;
            let attr = unsafe { GetFileAttributesW(buf.as_ptr()) };
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
        let mut fi1 = FileInfo::new();
        fi1.stat.st_ino = 42;
        fi1.stat.st_dev = 7;
        fi1.stat.st_size = 1234;
        fi1.stat.st_nlink = 2;
        fi1.stat.st_blksize = 4096;

        let fi2 = fi1;
        assert!(os_fileinfo_id_equal(&fi1, &fi2));
        assert_eq!(os_fileinfo_inode(&fi1), 42);
        assert_eq!(os_fileinfo_size(&fi1), 1234);
        assert_eq!(os_fileinfo_hardlinks(&fi1), 2);
        assert_eq!(os_fileinfo_blocksize(&fi1), 4096);

        let mut id1 = FileID::new();
        os_fileinfo_id(&fi1, &mut id1);
        let id2 = FileID {
            inode: 42,
            device_id: 7,
        };
        assert!(os_fileid_equal(&id1, &id2));
    }

    #[test]
    fn dirname_chdir_isdir_nodetype_and_can_exe() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut cwd = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd));
            let cwd_len = crate::memory::bytes_until_nul(&cwd).len();
            assert!(cwd_len > 0);
            let mut tiny = [0u8; 4];
            assert!(!os_dirname(&mut tiny[..cwd_len.min(3)]));

            // os_chdir("~") fails and leaves cwd unchanged
            assert!(!os_isdir(c"~"));
            assert_ne!(os_chdir(c"~"), 0);
            assert_ne!(os_chdir(c"~/"), 0);

            assert!(!os_isdir(c""));
            assert!(!os_isdir(c"non-existing-directory-xyz"));
            assert!(!os_isdir(c"/non-existing-directory-xyz"));
            assert!(os_isdir(c"."));
            assert!(os_isdir(c".."));
            assert!(os_isdir(crate::memory::bytes_as_cstr(&cwd)));

            assert_eq!(os_nodetype(c"non-existing-file-xyz"), NODE_NORMAL);
            #[cfg(not(windows))]
            assert_eq!(os_nodetype(c"/dev/stderr"), NODE_WRITABLE);

            let mut abspath: Option<CString> = None;
            assert!(!os_can_exe(c".", Some(&mut abspath), true));
            assert!(abspath.is_none());
            assert!(!os_can_exe(
                c"does-not-exist.file",
                Some(&mut abspath),
                true
            ));
            assert!(abspath.is_none());

            #[cfg(windows)]
            let tool = c"cmd";
            #[cfg(not(windows))]
            let tool = c"sh";
            assert!(os_can_exe(tool, Some(&mut abspath), true));
            let abspath_cstr = abspath.unwrap();
            assert!(crate::path::path_is_absolute(abspath_cstr.to_bytes()));
        }
    }

    #[test]
    fn open_read_write_rename_remove_mkdir_and_permissions() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let mut cwd_buf = [0u8; MAXPATHL];
            assert!(os_dirname(&mut cwd_buf));
            let cwd = crate::memory::bytes_as_cstr(&cwd_buf).to_str().unwrap();
            let base = format!("{cwd}/target/ut_fs_{}", std::process::id());
            let _ = std::fs::remove_dir_all(&base);

            let base_c = CString::new(base.clone()).unwrap();
            assert_eq!(os_mkdir(base_c.as_c_str(), 0o700), 0);
            assert_ne!(os_mkdir(base_c.as_c_str(), 0o700), 0);
            assert!(os_isdir(base_c.as_c_str()));
            assert!(os_path_exists(base_c.as_c_str()));
            assert!(!os_path_exists(c"non-existing-file-xyz"));
            assert_eq!(os_getperm(c"non-existing-file-xyz"), UV_ENOENT);
            assert!(os_getperm(base_c.as_c_str()) > 0);
            assert!(!os_setperm(c"non-existing-file-xyz", 0o700));
            assert_eq!(os_file_is_writable(base_c.as_c_str()), 2);

            // os_open / os_write / os_read / os_close
            assert_eq!(os_open(c"non-existing-file-xyz", O_RDWR, 0), UV_ENOENT);
            assert!(os_close(-1) < 0);

            let fpath = format!("{base}/test.dat");
            let fpath_c = CString::new(fpath.clone()).unwrap();
            let fd = os_open(fpath_c.as_c_str(), O_CREAT | O_RDWR, 0o600);
            assert!(fd >= 0);
            assert_eq!(
                os_open(fpath_c.as_c_str(), O_CREAT | O_EXCL, 0o600),
                if cfg!(windows) { -4075 } else { -17 }
            );

            let mut payload = Vec::with_capacity(4096);
            for _ in 0..16 {
                for b in 0..=255u8 {
                    payload.push(b);
                }
            }
            assert_eq!(os_write(fd, &[], false), 0);
            assert_eq!(os_write(fd, &payload, false), payload.len() as isize);
            assert_eq!(os_fchown(fd, uv_uid_t::MAX, uv_gid_t::MAX), 0);

            let mut fi_fd = FileInfo::new();
            assert!(!os_fileinfo_fd(-1, &mut fi_fd));
            assert!(os_fileinfo_fd(fd, &mut fi_fd));
            assert_eq!(os_fileinfo_size(&fi_fd), payload.len() as u64);
            assert_eq!(os_close(fd), 0);

            assert!(os_file_is_readable(fpath_c.as_c_str()));
            assert_eq!(os_file_is_writable(fpath_c.as_c_str()), 1);

            let rfd = os_open(fpath_c.as_c_str(), O_RDONLY, 0);
            assert!(rfd >= 0);
            let mut eof = true;
            assert_eq!(os_read(rfd, &mut eof, &mut [], false), 0);
            assert!(!eof);

            let mut rbuf = vec![0u8; payload.len()];
            assert_eq!(
                os_read(rfd, &mut eof, &mut rbuf, false),
                payload.len() as isize
            );
            assert!(!eof);
            assert_eq!(rbuf, payload);
            assert_eq!(os_read(rfd, &mut eof, &mut rbuf[..16], false), 0);
            assert!(eof);
            assert_eq!(os_close(rfd), 0);

            // os_fileinfo & os_fileid
            let mut fi = FileInfo::new();
            assert!(!os_fileinfo(c"", &mut fi));
            assert!(!os_fileinfo(c"/non-existent-xyz", &mut fi));
            assert!(os_fileinfo(fpath_c.as_c_str(), &mut fi));
            assert!(os_fileinfo_link(fpath_c.as_c_str(), &mut fi));
            assert_eq!(os_fileinfo_size(&fi), payload.len() as u64);
            assert!(os_fileinfo_blocksize(&fi) > 0);

            let mut fid = FileID::new();
            assert!(!os_fileid(c"/non-existent-xyz", &mut fid));
            assert!(os_fileid(fpath_c.as_c_str(), &mut fid));
            assert!(os_fileid_equal(&fid, &fid));

            // os_rename & os_remove
            let renamed = format!("{base}/renamed.dat");
            let renamed_c = CString::new(renamed.clone()).unwrap();
            assert!(os_rename(fpath_c.as_c_str(), renamed_c.as_c_str()));
            assert!(!os_path_exists(fpath_c.as_c_str()));
            assert!(os_path_exists(renamed_c.as_c_str()));
            assert!(!os_rename(fpath_c.as_c_str(), renamed_c.as_c_str()));

            // os_mkdir_recurse & os_rmdir
            let mut failed_dir: Option<CString> = None;
            let mut created_dir: Option<CString> = None;
            let nested = CString::new(format!("{base}/nested/1/2///")).unwrap();
            assert_eq!(
                os_mkdir_recurse(
                    nested.as_c_str(),
                    0o700,
                    &mut failed_dir,
                    Some(&mut created_dir)
                ),
                0
            );
            assert!(failed_dir.is_none());
            assert!(created_dir.is_some());

            // Cannot mkdir over an existing file
            let conflict = CString::new(format!("{renamed}/sub")).unwrap();
            assert_ne!(
                os_mkdir_recurse(conflict.as_c_str(), 0o700, &mut failed_dir, None),
                0
            );
            assert!(failed_dir.is_some());

            assert_eq!(os_remove(renamed_c.as_c_str()), 0);
            assert_ne!(os_remove(renamed_c.as_c_str()), 0);

            let d2 = CString::new(format!("{base}/nested/1/2")).unwrap();
            let d1 = CString::new(format!("{base}/nested/1")).unwrap();
            let d0 = CString::new(format!("{base}/nested")).unwrap();
            assert_eq!(os_rmdir(d2.as_c_str()), 0);
            assert_eq!(os_rmdir(d1.as_c_str()), 0);
            assert_eq!(os_rmdir(d0.as_c_str()), 0);
            assert_eq!(os_rmdir(base_c.as_c_str()), 0);
            assert_ne!(os_rmdir(base_c.as_c_str()), 0);
        }
    }
}
