//! File reading, encoding conversion, timestamp checking, and temp files (`src/nvim/fileio.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    clippy::useless_conversion
)]

use crate::api::command::exarg_T;
use crate::api::options::CtxSwitch;
use crate::api::private::helpers::cstr_as_obj;
use crate::api::tabpage::pos_T;
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_BUFNEW, EVENT_BUFNEWFILE, EVENT_BUFREADCMD,
    EVENT_BUFREADPOST, EVENT_BUFREADPRE, EVENT_BUFWIPEOUT, EVENT_FILECHANGEDSHELL,
    EVENT_FILECHANGEDSHELLPOST, EVENT_FILEREADCMD, EVENT_FILEREADPOST, EVENT_FILEREADPRE,
    EVENT_FILETYPE, EVENT_FILTERREADPOST, EVENT_FILTERREADPRE, EVENT_STDINREADPRE, apply_autocmds,
    apply_autocmds_exarg, apply_autocmds_group, augroup_exists, augroup_find, do_doautocmd,
};
use crate::buffer::{
    bt_dontwrite, bt_nofilename, bt_normal, buf_contents_changed, buf_is_empty, buflist_new,
    bufref_T, bufref_valid, do_modelines, set_bufref, setfname, wipe_buffer,
};
use crate::buffer_updates::buf_updates_unload;
use crate::change::{appended_lines_mark, save_file_ff, unchanged};
#[cfg(windows)]
use crate::charset::vim_isfilec;
use crate::charset::{ascii_isdigit, ascii_isspace, skipdigits};
use crate::context::{ctx_restore, ctx_switch};
use crate::cursor::{check_cursor, check_cursor_lnum, colnr_T, linenr_T};
use crate::dialog::do_dialog;
use crate::diff::diff_invalidate;
use crate::drawscreen::{redraw_curbuf_later, status_redraw_all};
use crate::errors::*;
use crate::eval::deprecated::buf_T;
use crate::eval::fs::FileInfo;
use crate::eval::vars::{
    VV_FCS_CHOICE, VV_FCS_REASON, VV_WARNINGMSG, eval_charconvert, get_vim_var_str,
    set_vim_var_string,
};
#[cfg(not(test))]
use crate::eval::vars::{VV_TESTING, get_vim_var_nr};
use crate::event::r#loop::uv_strerror;
use crate::ex_eval::aborting;
use crate::fold::{foldUpdateAll, foldmethodIsManual};
use crate::garray::{ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::input::{stuff_empty, typebuf_typed};
use crate::insert::beginline;
use crate::log::logmsg;
use crate::mbyte::{
    enc_canon_props, enc_canonize, my_iconv_open, utf_byte2len, utf_char2bytes, utf_char2len,
    utf_head_off, utf_ptr2char, utf_ptr2len_len,
};
use crate::mcursor::mc_buf_clear;
use crate::memfile::{memfile_T, mf_fullname};
use crate::memline::{
    check_need_swap, ml_append, ml_delete, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open,
};
use crate::memory::{
    memchrsub, strequal, time_to_bytes, verbose_try_malloc, xfree, xmalloc, xmallocz, xmemdupz,
    xstrdup, xstrlcat, xstrlcpy,
};
use crate::message::{
    emsg, msg, msg_check_for_delay, msg_clr_eos, msg_delay, msg_end, msg_may_trunc, msg_outtrans,
    msg_progress, msg_putchar, msg_puts, msg_puts_hl, msg_schedule_semsg, msg_start, msg_trunc,
    msg_with_id, semsg, set_keep_msg, smsg,
};
use crate::r#move::update_topline;
use crate::option::{
    copy_option_part, default_fileformat, get_fileformat, get_fileformat_force, set_fileformat,
    set_option_direct, set_options_bin, shortmess,
};
use crate::option_vars::*;
use crate::os::env::{expand_env, home_replace, home_replace_save, os_env_exists};
use crate::os::fs::{
    Directory, os_closedir, os_copy, os_dirname, os_file_is_writable, os_file_owned, os_fileinfo,
    os_fileinfo_fd, os_fileinfo_id_equal, os_fileinfo_size, os_free_acl, os_get_acl, os_getperm,
    os_isdir, os_isrealdir, os_mkdir, os_mkdtemp, os_open, os_path_exists, os_remove, os_rename,
    os_rmdir, os_scandir, os_scandir_next, os_set_acl, os_set_cloexec, vim_acl_T,
};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo_link, os_setperm, uv_gid_t, uv_uid_t};
use crate::os::input::os_breakcheck;
use crate::os::time::{os_hrtime, time_t};
use crate::os::users::os_get_username;
use crate::path::{
    add_pathsep, after_pathsep, dir_of_file_exists, path_equal, path_is_absolute,
    path_shorten_fname, path_tail, path_with_url, vim_FullName, vim_ispathsep,
};
use crate::regexp::{vim_regcomp, vim_regexec, vim_regfree};
use crate::sha256::{context_sha256_T, sha256_finish, sha256_start, sha256_update};
use crate::shada::check_marks_read;
use crate::state::{MODE_CMDLINE, MODE_NORMAL_BUSY};
use crate::strings::{
    gettext as tr, ngettext, sort_strings, strlen as c_strlen, vim_snprintf, vim_strchr,
};
use crate::ui::{ui_flush, ui_has};
use crate::undo::{
    bufIsChanged, u_clearallandblockfree, u_clearline, u_compute_hash, u_find_first_changed,
    u_read_undo, u_savecommon, u_sync, u_unchanged, u_write_undo,
};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::os_open_conin_fd;

pub use crate::eval::buffer::varnumber_T;
pub use crate::option::OptInt;
pub type off_T = i64;
pub type iconv_t = isize;
pub type FILE = c_void;
pub use crate::eval::fs::CheckItem;
pub use crate::regexp::regprog_T;

pub const READ_NEW: c_int = 0x01;
pub const READ_FILTER: c_int = 0x02;
pub const READ_STDIN: c_int = 0x04;
pub const READ_BUFFER: c_int = 0x08;
pub const READ_DUMMY: c_int = 0x10;
pub const READ_KEEP_UNDO: c_int = 0x20;
pub const READ_FIFO: c_int = 0x40;
pub const READ_NOWINENTER: c_int = 0x80;
pub const READ_NOFILE: c_int = 0x100;

pub const FIO_LATIN1: c_int = 0x01;
pub const FIO_UTF8: c_int = 0x02;
pub const FIO_UCS2: c_int = 0x04;
pub const FIO_UCS4: c_int = 0x08;
pub const FIO_UTF16: c_int = 0x10;
pub const FIO_ENDIAN_L: c_int = 0x80;
pub const FIO_NOCONVERT: c_int = 0x2000;
pub const FIO_UCSBOM: c_int = 0x4000;
pub const FIO_ALL: c_int = -1;

pub const CONV_RESTLEN: usize = 30;
pub const WRITEBUFSIZE: usize = 8192;
pub const ICONV_MULT: isize = 8;

const NUL: c_char = 0;
const NL: c_char = b'\n' as c_char;
const CAR: c_char = b'\r' as c_char;
const CTRL_Z: c_char = 0x1a;

const FAIL: c_int = 0;
const OK: c_int = 1;
const NOTDONE: c_int = 2;
const EOF: c_int = -1;

const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: isize = 0x7fffffff;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const UNDO_HASH_SIZE: usize = 32;

const EOL_UNKNOWN: c_int = -1;
const EOL_UNIX: c_int = 0;
const EOL_DOS: c_int = 1;
const EOL_MAC: c_int = 2;

const BAD_REPLACE: c_int = b'?' as c_int;
const BAD_KEEP: c_int = -1;
const BAD_DROP: c_int = -2;

const FORCE_BIN: c_int = 1;
const FORCE_NOBIN: c_int = 2;

const OPT_LOCAL: c_int = 0x02;
const kOptFileencoding: c_int = 92;

const ENC_UNICODE: c_int = 0x04;
const ENC_ENDIAN_L: c_int = 0x20;
const ENC_2BYTE: c_int = 0x40;
const ENC_4BYTE: c_int = 0x80;
const ENC_2WORD: c_int = 0x100;
const ENC_LATIN1: c_int = 0x200;

const BF_CHECK_RO: c_int = 0x02;
const BF_NOTEDITED: c_int = 0x08;
const BF_NEW: c_int = 0x10;
const BF_NEW_W: c_int = 0x20;

const ML_EMPTY: c_int = 0x01;
const MF_DIRTY_YES: c_int = 1;
const MF_DIRTY_YES_NOSYNC: c_int = 2;

const SEA_QUIT: c_int = 2;
const BL_WHITE: c_int = 1;
const BL_FIX: c_int = 4;
const BLN_DUMMY: c_int = 4;
const UPD_NOT_VALID: c_int = 40;
const CMOD_LOCKMARKS: c_int = 0x0800;

const HLF_E: c_int = 6;
const HLF_W: c_int = 26;
const VIM_WARNING: c_int = 2;
const kUIMessages: c_int = 4;
const kCallbackNone: c_int = 0;
const kPathCmpLiteral: c_int = 1 << 0;
const RE_MAGIC: c_int = 1;

const kCpoFnamer: c_int = b'f' as c_int;
const kShmOverall: c_int = b'O' as c_int;
const kShmOver: c_int = b'o' as c_int;
const kShmRo: c_int = b'r' as c_int;
const kShmLines: c_int = b'l' as c_int;
const kShmFileinfo: c_int = b'F' as c_int;

const O_RDONLY: c_int = 0;
const SEEK_SET: c_int = 0;
const EINTR: c_int = 4;
const ICONV_EINVAL: c_int = 22;
const UV_FS_COPYFILE_EXCL: c_int = 0x0001;

const S_IFMT: u64 = 0o170000;
const S_IFIFO: u64 = 0o010000;
const S_IFDIR: u64 = 0o040000;
const S_IFREG: u64 = 0o100000;
#[cfg(unix)]
const S_IFLNK: u64 = 0o120000;
const S_IFSOCK: u64 = 0o140000;

const LOG_DEBUG: c_int = 1;
const LOG_WARN: c_int = 3;
const LOG_ERR: c_int = 4;

const NONASCII_MASK: u64 = (u64::MAX / 0xFF) * 0x80;

#[cfg(windows)]
const UV_ENOENT: c_int = -4058;
#[cfg(not(windows))]
const UV_ENOENT: c_int = -2;

#[cfg(windows)]
const UV_EFBIG: c_int = -4036;
#[cfg(not(windows))]
const UV_EFBIG: c_int = -27;

#[cfg(target_os = "linux")]
const EOVERFLOW: c_int = 75;
#[cfg(all(unix, not(target_os = "linux")))]
const EOVERFLOW: c_int = 84;

#[cfg(windows)]
const BASENAMELEN: usize = 260 - 5;
#[cfg(not(windows))]
const BASENAMELEN: usize = 255 - 5;

#[cfg(windows)]
const TEMP_FILE_PATH_MAXLEN: usize = 260;
#[cfg(not(windows))]
const TEMP_FILE_PATH_MAXLEN: usize = 256;

#[cfg(windows)]
const PATHSEPSTR: &CStr = c"\\";
#[cfg(not(windows))]
const PATHSEPSTR: &CStr = c"/";

#[cfg(windows)]
const BACKSLASH_IN_FILENAME_BOOL: bool = true;
#[cfg(not(windows))]
const BACKSLASH_IN_FILENAME_BOOL: bool = false;

#[cfg(all(not(test), unix))]
const LOCK_SH: c_int = 1;

static E_AUCHANGEDBUF: &CStr = c"E812: Autocommands changed buffer or buffer name";
static MSG_IS_A_DIRECTORY: &CStr = c"is a directory";

static mut ALREADY_WARNED: bool = false;
static mut VIM_TEMPDIR: *mut c_char = ptr::null_mut();
#[cfg(all(not(test), unix))]
static mut VIM_TEMPDIR_DP: *mut c_void = ptr::null_mut();

pub use crate::api::command::regmatch_T;

unsafe extern "C" {

    pub fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
    pub fn fclose(stream: *mut FILE) -> c_int;
    pub fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fgets(s: *mut c_char, n: c_int, stream: *mut FILE) -> *mut c_char;
    pub fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
    pub fn fgetc(stream: *mut FILE) -> c_int;
    pub fn fputc(c: c_int, stream: *mut FILE) -> c_int;
    pub fn fprintf(stream: *mut FILE, format: *const c_char, ...) -> c_int;
    pub fn printf(format: *const c_char, ...) -> c_int;
    pub fn fflush(stream: *mut FILE) -> c_int;
    pub fn fseek(stream: *mut FILE, offset: c_long, whence: c_int) -> c_int;
    pub fn ftell(stream: *mut FILE) -> c_long;
    pub fn ferror(stream: *mut FILE) -> c_int;
    pub fn feof(stream: *mut FILE) -> c_int;

    #[cfg(windows)]
    pub fn libiconv(
        cd: iconv_t,
        inbuf: *mut *const c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize;
    #[cfg(windows)]
    pub fn libiconv_close(cd: iconv_t) -> c_int;
    #[cfg(windows)]
    pub fn _close(fd: c_int) -> c_int;
    #[cfg(windows)]
    pub fn _dup(fd: c_int) -> c_int;
    #[cfg(windows)]
    pub fn _lseeki64(fd: c_int, offset: i64, origin: c_int) -> i64;
    #[cfg(windows)]
    pub fn _fseeki64(stream: *mut FILE, offset: i64, origin: c_int) -> c_int;
    #[cfg(windows)]
    pub fn _ftelli64(stream: *mut FILE) -> i64;
    #[cfg(windows)]
    pub fn _fdopen(fd: c_int, mode: *const c_char) -> *mut FILE;
    #[cfg(windows)]
    pub fn _fileno(stream: *mut FILE) -> c_int;
    #[cfg(windows)]
    pub fn _get_osfhandle(fd: c_int) -> isize;
    #[cfg(windows)]
    pub fn _read(fd: c_int, buf: *mut c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    pub fn _write(fd: c_int, buf: *const c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    fn _umask(pmode: c_int) -> c_int;

    #[cfg(not(windows))]
    pub fn iconv(
        cd: iconv_t,
        inbuf: *mut *const c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize;
    #[cfg(not(windows))]
    pub fn iconv_close(cd: iconv_t) -> c_int;
    #[cfg(not(windows))]
    pub fn close(fd: c_int) -> c_int;
    #[cfg(not(windows))]
    pub fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    #[cfg(not(windows))]
    pub fn fseeko(stream: *mut FILE, offset: i64, whence: c_int) -> c_int;
    #[cfg(not(windows))]
    pub fn ftello(stream: *mut FILE) -> i64;
    #[cfg(not(windows))]
    pub fn fdopen(fd: c_int, mode: *const c_char) -> *mut FILE;
    #[cfg(not(windows))]
    pub fn fileno(stream: *mut FILE) -> c_int;
    #[cfg(not(windows))]
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    #[cfg(not(windows))]
    pub fn getuid() -> u32;
    #[cfg(not(windows))]
    pub fn getgid() -> u32;
    #[cfg(not(windows))]
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    pub fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    #[cfg(not(windows))]
    pub fn dup(oldfd: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_dup2")]
    pub fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    #[cfg(not(windows))]
    pub fn kill(pid: c_int, sig: c_int) -> c_int;
    #[cfg(not(windows))]
    pub fn umask(mask: c_uint) -> c_uint;
    #[cfg(not(windows))]
    pub fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize;
    #[cfg(not(windows))]
    fn symlink(target: *const c_char, linkpath: *const c_char) -> c_int;
    #[cfg(all(not(test), not(windows)))]
    fn opendir(name: *const c_char) -> *mut c_void;
    #[cfg(all(not(test), not(windows)))]
    fn closedir(dirp: *mut c_void) -> c_int;
    #[cfg(all(not(test), not(windows)))]
    fn dirfd(dirp: *mut c_void) -> c_int;
    #[cfg(all(not(test), not(windows)))]
    fn flock(fd: c_int, operation: c_int) -> c_int;

}

#[inline]
const fn s_isdir(m: i32) -> bool {
    ((m as u64) & S_IFMT) == S_IFDIR
}

#[inline]
const fn s_isreg(m: i32) -> bool {
    ((m as u64) & S_IFMT) == S_IFREG
}

#[inline]
const fn s_isfifo(m: i32) -> bool {
    cfg!(unix) && ((m as u64) & S_IFMT) == S_IFIFO
}

#[inline]
const fn s_issock(m: i32) -> bool {
    cfg!(unix) && ((m as u64) & S_IFMT) == S_IFSOCK
}

#[cfg(unix)]
#[inline]
const fn s_islnk(m: u64) -> bool {
    (m & S_IFMT) == S_IFLNK
}

#[inline]
unsafe fn get_errno() -> c_int {
    unsafe { crate::os::fs::os_errno() }
}

#[inline]
unsafe fn set_errno(val: c_int) {
    unsafe { crate::os::fs::os_set_errno(val) }
}

#[inline]
unsafe fn close_fd(fd: c_int) -> c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        _close(fd)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        close(fd)
    }
    #[cfg(test)]
    {
        let _ = fd;
        0
    }
}

#[inline]
unsafe fn vim_lseek(fd: c_int, offset: off_T, whence: c_int) -> off_T {
    #[cfg(all(not(test), windows))]
    unsafe {
        _lseeki64(fd, offset, whence)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        lseek(fd, offset, whence)
    }
    #[cfg(test)]
    {
        let _ = (fd, offset, whence);
        -1
    }
}

#[inline]
unsafe fn call_iconv(
    cd: iconv_t,
    inbuf: *mut *const c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    #[cfg(all(not(test), windows))]
    unsafe {
        libiconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        iconv(cd, inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(test)]
    {
        let _ = (cd, inbuf, inbytesleft, outbuf, outbytesleft);
        usize::MAX
    }
}

#[inline]
unsafe fn call_iconv_close(cd: iconv_t) -> c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        libiconv_close(cd)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        iconv_close(cd)
    }
    #[cfg(test)]
    {
        let _ = cd;
        0
    }
}

#[inline]
unsafe fn call_umask(mask: c_uint) -> c_uint {
    #[cfg(all(not(test), windows))]
    unsafe {
        _umask(mask as c_int) as c_uint
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        umask(mask)
    }
    #[cfg(test)]
    {
        let _ = mask;
        0
    }
}

/// Shows a message about file `name`, e.g. `"foo.txt" [New]`.
unsafe fn filemess(buf: *mut buf_T, name: *mut c_char, s: *const c_char, msg_id: *const c_char) {
    unsafe {
        let prev_msg_col = msg_col;

        if msg_silent != 0 {
            return;
        }

        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
        add_quoted_fname(iobuf_ptr, IOSIZE - 100, buf, name);

        xstrlcat(iobuf_ptr, s, IOSIZE);

        let msg_scroll_save = msg_scroll;
        if shortmess(kShmOverall) && msg_listdo_overwrite == 0 && !exiting && p_verbose == 0 {
            msg_scroll = 0;
        }
        if msg_scroll == 0 {
            msg_check_for_delay(false);
        }
        msg_start();
        if prev_msg_col != 0 && msg_col == 0 {
            msg_putchar(b'\r' as c_int);
        }
        msg_scroll = msg_scroll_save;
        msg_scrolled_ign = true;
        if !msg_id.is_null() {
            msg_with_id(
                iobuf_ptr,
                msg_id.cast_mut(),
                c"bufwrite".as_ptr(),
                ptr::null_mut(),
                0,
                false,
                true,
                false,
            );
        } else {
            msg_outtrans(msg_may_trunc(false, iobuf_ptr), 0, false);
        }
        msg_clr_eos();
        msg_scrolled_ign = false;
    }
}

/// Shows file `name` before writing it, as a "bufwrite" message with `msg_id`, which buf_write()
/// replaces when done (or with a progress-message, if slow).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn filemess_bufwrite(
    buf: *mut buf_T,
    name: *mut c_char,
    msg_id: *const c_char,
) {
    unsafe {
        filemess(buf, name, c"".as_ptr(), msg_id);
    }
}

/// If a file read/write is slower than this, emit a progress-message.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn slow_io_ns() -> u64 {
    #[cfg(not(test))]
    unsafe {
        if nvim_testing && get_vim_var_nr(VV_TESTING) != 0 {
            0
        } else {
            1_000_000_000 / 2
        }
    }
    #[cfg(test)]
    {
        1_000_000_000 / 2
    }
}

/// Shows the progress-message of a slow readfile(). Locks `curbuf` so Progress handlers cannot
/// accidentally unload it while it is loading.
unsafe fn readfile_progress(
    s: *const c_char,
    msg_id: *const c_char,
    status: *const c_char,
    percent: c_int,
    hist: bool,
) -> *mut c_char {
    unsafe {
        let buf = curbuf;
        (*buf).b_locked += 1;
        let rv = msg_progress(
            s.cast_mut(),
            msg_id.cast_mut(),
            status.cast_mut(),
            percent,
            0,
            hist,
            !s.is_null(),
            false,
        );
        (*buf).b_locked -= 1;
        rv
    }
}

/// Read lines from file "fname" into the buffer after line "from".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn readfile(
    fname: *mut c_char,
    mut sfname: *mut c_char,
    from: linenr_T,
    lines_to_skip: linenr_T,
    lines_to_read: linenr_T,
    eap: *mut exarg_T,
    flags: c_int,
    silent: bool,
) -> c_int {
    #[cfg(unix)]
    let mut fname = fname;
    unsafe {
        let mut retval = FAIL;
        let mut fd: c_int = if stdin_fd >= 0 { stdin_fd } else { 0 };
        let newfile = (flags & READ_NEW) != 0;
        let filtering = (flags & READ_FILTER) != 0;
        let read_stdin = (flags & READ_STDIN) != 0;
        let read_buffer = (flags & READ_BUFFER) != 0;
        let read_fifo = (flags & READ_FIFO) != 0;
        let set_options = newfile || read_buffer || (!eap.is_null() && (*eap).read_edit != 0);
        let mut read_buf_lnum: linenr_T = 1;
        let mut read_buf_col: colnr_T = 0;
        let mut lnum = from;
        let mut ptr: *mut c_char = ptr::null_mut();
        let mut buffer: *mut c_char = ptr::null_mut();
        let mut new_buffer: *mut c_char = ptr::null_mut();
        let mut line_start: *mut c_char = ptr::null_mut();
        let mut size: isize = 0;
        let mut p: *mut u8;
        let mut filesize: off_T = 0;
        let mut skip_read = false;
        let mut sha_ctx: context_sha256_T = std::mem::zeroed();
        let mut read_undo_file = false;
        let mut split: c_int = 0;
        let mut error = false;
        let mut ff_error: c_int = EOL_UNKNOWN;
        let mut linerest: isize = 0;
        let mut perm: i32 = 0;
        #[cfg(unix)]
        let mut swap_mode: c_int = -1;
        let mut fileformat: c_int = 0;
        let mut keep_fileformat = false;
        let mut file_info: FileInfo = std::mem::zeroed();
        let mut skip_count: linenr_T = 0;
        let mut read_count: linenr_T = 0;
        let msg_save = msg_scroll;
        let mut read_no_eol_lnum: linenr_T = 0;
        let mut file_rewind = false;
        let mut conv_error: linenr_T = 0;
        let mut illegal_byte: linenr_T = 0;
        let mut keep_dest_enc = false;
        let mut bad_char_behavior: c_int = BAD_REPLACE;
        let mut tmpname: *mut c_char = ptr::null_mut();
        let mut fio_flags: c_int;
        let mut fenc: *mut c_char;
        let mut fenc_alloced = false;
        let mut fenc_next: *mut c_char = ptr::null_mut();
        let mut advance_fenc = false;
        let mut real_size: c_int = 0;
        let mut iconv_fd: iconv_t = -1;
        let mut did_iconv = false;
        let mut converted = false;
        let mut notconverted = false;
        let mut conv_rest: [c_char; CONV_RESTLEN] = [0; CONV_RESTLEN];
        let mut conv_restlen: c_int = 0;
        let mut orig_start: pos_T;

        (*curbuf).b_au_did_filetype = false;
        (*curbuf).b_no_eol_lnum = 0;

        'theend: {
            if (*curbuf).b_ffname.is_null()
                && !filtering
                && !fname.is_null()
                && !vim_strchr(p_cpo, kCpoFnamer).is_null()
                && (flags & READ_DUMMY) == 0
                && set_rw_fname(fname, sfname) == FAIL
            {
                break 'theend;
            }

            let old_curbuf = curbuf;
            let old_b_ffname = (*curbuf).b_ffname;
            let old_b_fname = (*curbuf).b_fname;
            let using_b_ffname = fname == (*curbuf).b_ffname || sfname == (*curbuf).b_ffname;
            let using_b_fname = fname == (*curbuf).b_fname || sfname == (*curbuf).b_fname;

            need_fileinfo = false;

            if sfname.is_null() {
                sfname = fname;
            }
            #[cfg(unix)]
            {
                fname = sfname;
            }

            if !filtering && !read_stdin && !read_buffer {
                orig_start = (*curbuf).b_op_start;

                (*curbuf).b_op_start.lnum = if from == 0 { 1 } else { from };
                (*curbuf).b_op_start.col = 0;

                if newfile {
                    if apply_autocmds_exarg(
                        EVENT_BUFREADCMD,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    ) {
                        retval = OK;
                        if aborting() {
                            retval = FAIL;
                        }

                        if retval == OK && !(*curbuf).b_au_did_filetype && *(*curbuf).b_p_ft != NUL
                        {
                            apply_autocmds(
                                EVENT_FILETYPE,
                                (*curbuf).b_p_ft,
                                (*curbuf).b_fname,
                                true,
                                curbuf,
                            );
                            if aborting() {
                                retval = FAIL;
                            }
                        }
                        if retval == OK {
                            (*curbuf).b_flags &= !BF_NOTEDITED;
                        }
                        break 'theend;
                    }
                } else if apply_autocmds_exarg(
                    EVENT_FILEREADCMD,
                    sfname,
                    sfname,
                    false,
                    ptr::null_mut(),
                    eap,
                ) {
                    retval = if aborting() { FAIL } else { OK };
                    break 'theend;
                }

                (*curbuf).b_op_start = orig_start;

                if (flags & READ_NOFILE) != 0 {
                    retval = NOTDONE;
                    break 'theend;
                }
            }

            if ((shortmess(kShmOver) && msg_listdo_overwrite == 0) || (*curbuf).b_help)
                && p_verbose == 0
            {
                msg_scroll = 0;
            } else {
                msg_scroll = 1;
            }

            if !fname.is_null() && *fname != NUL {
                let fnamelen = c_strlen(fname);

                if fnamelen >= MAXPATHL {
                    filemess(
                        curbuf,
                        fname,
                        tr(c"Illegal file name".as_ptr()),
                        ptr::null(),
                    );
                    msg_end();
                    msg_scroll = msg_save;
                    break 'theend;
                }

                if after_pathsep(fname, fname.add(fnamelen)) != 0 {
                    if !silent {
                        filemess(curbuf, fname, tr(MSG_IS_A_DIRECTORY.as_ptr()), ptr::null());
                    }
                    msg_end();
                    msg_scroll = msg_save;
                    retval = NOTDONE;
                    break 'theend;
                }
            }

            if !read_stdin && !fname.is_null() {
                perm = os_getperm(fname);
            }

            #[cfg(any(
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly",
                target_os = "macos"
            ))]
            let is_chr_dev = s_ischr(perm) && is_dev_fd_file(fname);
            #[cfg(not(any(
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly",
                target_os = "macos"
            )))]
            let is_chr_dev = false;

            if !read_stdin
                && !read_buffer
                && !read_fifo
                && perm >= 0
                && !s_isreg(perm)
                && !s_isfifo(perm)
                && !s_issock(perm)
                && !is_chr_dev
            {
                if s_isdir(perm) {
                    if !silent {
                        filemess(curbuf, fname, tr(MSG_IS_A_DIRECTORY.as_ptr()), ptr::null());
                    }
                    retval = NOTDONE;
                } else {
                    filemess(curbuf, fname, tr(c"is not a file".as_ptr()), ptr::null());
                }
                msg_end();
                msg_scroll = msg_save;
                break 'theend;
            }

            set_file_options(set_options, eap);

            let check_readonly = newfile && ((*curbuf).b_flags & BF_CHECK_RO) != 0;
            if check_readonly && !readonlymode {
                (*curbuf).b_p_ro = 0;
            }

            if newfile && !read_stdin && !read_buffer && !read_fifo {
                if os_fileinfo(fname, &raw mut file_info) {
                    buf_store_file_info(curbuf, &raw mut file_info);
                    (*curbuf).b_mtime_read = (*curbuf).b_mtime;
                    (*curbuf).b_mtime_read_ns = (*curbuf).b_mtime_ns;
                    #[cfg(unix)]
                    {
                        swap_mode = ((file_info.stat.st_mode as c_int) & 0o644) | 0o600;
                    }
                } else {
                    (*curbuf).b_mtime = 0;
                    (*curbuf).b_mtime_ns = 0;
                    (*curbuf).b_mtime_read = 0;
                    (*curbuf).b_mtime_read_ns = 0;
                    (*curbuf).b_orig_size = 0;
                    (*curbuf).b_orig_mode = 0;
                }

                (*curbuf).b_flags &= !(BF_NEW | BF_NEW_W);
            }

            let mut file_readonly = false;
            if !read_buffer && !read_stdin {
                if !newfile
                    || readonlymode
                    || (perm & 0o222) == 0
                    || os_file_is_writable(fname) == 0
                {
                    file_readonly = true;
                }
                fd = os_open(fname, O_RDONLY, 0);
            }

            if fd < 0 {
                msg_scroll = msg_save;
                if !newfile {
                    break 'theend;
                }
                if perm == UV_ENOENT {
                    (*curbuf).b_flags |= BF_NEW;

                    if !bt_dontwrite(curbuf) {
                        check_need_swap(newfile);
                        if curbuf != old_curbuf
                            || (using_b_ffname && old_b_ffname != (*curbuf).b_ffname)
                            || (using_b_fname && old_b_fname != (*curbuf).b_fname)
                        {
                            emsg(tr(E_AUCHANGEDBUF.as_ptr()));
                            break 'theend;
                        }
                    }
                    if !silent {
                        if dir_of_file_exists(fname) {
                            filemess(curbuf, sfname, tr(c"[New]".as_ptr()), ptr::null());
                        } else {
                            filemess(curbuf, sfname, tr(c"[New DIRECTORY]".as_ptr()), ptr::null());
                        }
                    }
                    check_marks_read();
                    if !eap.is_null() {
                        set_forced_fenc(eap);
                    }
                    apply_autocmds_exarg(EVENT_BUFNEWFILE, sfname, sfname, false, curbuf, eap);
                    save_file_ff(curbuf);

                    if !aborting() {
                        retval = OK;
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                let is_too_big = fd == UV_EFBIG || fd == -EOVERFLOW;
                #[cfg(not(unix))]
                let is_too_big = fd == UV_EFBIG;
                filemess(
                    curbuf,
                    sfname,
                    if is_too_big {
                        tr(c"[File too big]".as_ptr())
                    } else {
                        tr(c"[Permission Denied]".as_ptr())
                    },
                    ptr::null(),
                );
                (*curbuf).b_p_ro = 1;

                break 'theend;
            }

            if (check_readonly && file_readonly) || (*curbuf).b_help {
                (*curbuf).b_p_ro = 1;
            }

            if set_options {
                if !read_buffer {
                    (*curbuf).b_p_eof = 0;
                    (*curbuf).b_start_eof = 0;
                    (*curbuf).b_p_eol = 1;
                    (*curbuf).b_start_eol = 1;
                }
                (*curbuf).b_p_bomb = 0;
                (*curbuf).b_start_bomb = 0;
            }

            if !bt_dontwrite(curbuf) {
                check_need_swap(newfile);
                if !read_stdin
                    && (curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != (*curbuf).b_ffname)
                        || (using_b_fname && old_b_fname != (*curbuf).b_fname))
                {
                    emsg(tr(E_AUCHANGEDBUF.as_ptr()));
                    if !read_buffer {
                        close_fd(fd);
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                {
                    let mfp = (*curbuf).b_ml.ml_mfp.cast::<memfile_T>();
                    if swap_mode > 0 && !mfp.is_null() && !(*mfp).mf_fname.is_null() {
                        let swap_fname = (*mfp).mf_fname;
                        if (swap_mode & 0o44) == 0o40 {
                            let mut swap_info: FileInfo = std::mem::zeroed();
                            if os_fileinfo(swap_fname, &raw mut swap_info)
                                && file_info.stat.st_gid != swap_info.stat.st_gid
                                && os_fchown(
                                    (*mfp).mf_fd,
                                    u32::MAX as uv_uid_t,
                                    file_info.stat.st_gid as uv_gid_t,
                                ) == -1
                            {
                                swap_mode &= 0o600;
                            }
                        }
                        os_setperm(swap_fname, swap_mode);
                    }
                }
            }

            if swap_exists_action == SEA_QUIT {
                if !read_buffer && !read_stdin {
                    close_fd(fd);
                }
                break 'theend;
            }

            no_wait_return += 1;

            orig_start = (*curbuf).b_op_start;
            (*curbuf).b_op_start.lnum = if from == 0 { 1 } else { from };
            (*curbuf).b_op_start.col = 0;

            let mut try_mac = c_int::from(!vim_strchr(p_ffs, b'm' as c_int).is_null());
            let mut try_dos = c_int::from(!vim_strchr(p_ffs, b'd' as c_int).is_null());
            let mut try_unix = c_int::from(!vim_strchr(p_ffs, b'x' as c_int).is_null());

            if !read_buffer {
                let m = msg_scroll;
                let n = msg_scrolled;

                if !read_stdin {
                    close_fd(fd);
                }

                msg_scroll = 1;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                } else if read_stdin {
                    apply_autocmds_exarg(
                        EVENT_STDINREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                } else if newfile {
                    apply_autocmds_exarg(
                        EVENT_BUFREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                } else {
                    apply_autocmds_exarg(
                        EVENT_FILEREADPRE,
                        sfname,
                        sfname,
                        false,
                        ptr::null_mut(),
                        eap,
                    );
                }

                try_mac = c_int::from(!vim_strchr(p_ffs, b'm' as c_int).is_null());
                try_dos = c_int::from(!vim_strchr(p_ffs, b'd' as c_int).is_null());
                try_unix = c_int::from(!vim_strchr(p_ffs, b'x' as c_int).is_null());
                (*curbuf).b_op_start = orig_start;

                if msg_scrolled == n {
                    msg_scroll = m;
                }

                if aborting() {
                    no_wait_return -= 1;
                    msg_scroll = msg_save;
                    (*curbuf).b_p_ro = 1;
                    break 'theend;
                }

                if !read_stdin
                    && (curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != (*curbuf).b_ffname)
                        || (using_b_fname && old_b_fname != (*curbuf).b_fname)
                        || {
                            fd = os_open(fname, O_RDONLY, 0);
                            fd < 0
                        })
                {
                    no_wait_return -= 1;
                    msg_scroll = msg_save;
                    if fd < 0 {
                        emsg(tr(
                            c"E200: *ReadPre autocommands made the file unreadable".as_ptr()
                        ));
                    } else {
                        emsg(tr(
                            c"E201: *ReadPre autocommands must not change current buffer".as_ptr(),
                        ));
                    }
                    (*curbuf).b_p_ro = 1;
                    break 'theend;
                }
            }

            let wasempty = ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0;

            let show_msg = !recoverymode
                && !filtering
                && (flags & READ_DUMMY) == 0
                && !read_stdin
                && !read_buffer
                && msg_silent == 0;
            if show_msg && !silent {
                filemess(curbuf, sfname, c"".as_ptr(), ptr::null());
            }

            let mut msg_id = [0 as c_char; MAXPATHL + 32];
            fname_msg_id(
                msg_id.as_mut_ptr(),
                msg_id.len(),
                c"nvim.bufread".as_ptr(),
                curbuf,
                sfname,
            );
            let mut progress = false;
            let mut percent: c_int = -1;
            let read_start = os_hrtime();

            msg_scroll = 0;

            let mut linecnt = (*curbuf).b_ml.ml_line_count;

            if !eap.is_null() && (*eap).bad_char != 0 {
                bad_char_behavior = (*eap).bad_char;
                if set_options {
                    (*curbuf).b_bad_char = (*eap).bad_char;
                }
            } else {
                (*curbuf).b_bad_char = 0;
            }

            if !eap.is_null() && (*eap).force_enc != 0 {
                fenc = enc_canonize((*eap).cmd.add((*eap).force_enc as usize));
                fenc_alloced = true;
                keep_dest_enc = true;
            } else if (*curbuf).b_p_bin != 0 {
                fenc = c"".as_ptr().cast_mut();
                fenc_alloced = false;
            } else if (*curbuf).b_help {
                fenc_next = c"latin1".as_ptr().cast_mut();
                fenc = c"utf-8".as_ptr().cast_mut();
                fenc_alloced = false;
            } else if *p_fencs == NUL {
                fenc = (*curbuf).b_p_fenc;
                fenc_alloced = false;
            } else {
                fenc_next = p_fencs;
                fenc = next_fenc(&raw mut fenc_next, &raw mut fenc_alloced);
            }

            'retry: loop {
                if file_rewind {
                    if read_buffer {
                        read_buf_lnum = 1;
                        read_buf_col = 0;
                    } else if read_stdin || vim_lseek(fd, 0, SEEK_SET) != 0 {
                        error = true;
                        break 'retry;
                    }
                    while lnum > from {
                        ml_delete(lnum);
                        lnum -= 1;
                    }
                    file_rewind = false;
                    if set_options {
                        (*curbuf).b_p_bomb = 0;
                        (*curbuf).b_start_bomb = 0;
                    }
                    conv_error = 0;
                }

                if keep_fileformat {
                    keep_fileformat = false;
                } else if !eap.is_null() && (*eap).force_ff != 0 {
                    fileformat = get_fileformat_force(curbuf, eap);
                    try_unix = 0;
                    try_dos = 0;
                    try_mac = 0;
                } else if (*curbuf).b_p_bin != 0 {
                    fileformat = EOL_UNIX;
                } else if *p_ffs == NUL {
                    fileformat = get_fileformat(curbuf);
                } else {
                    fileformat = EOL_UNKNOWN;
                }

                if iconv_fd != -1 {
                    call_iconv_close(iconv_fd);
                    iconv_fd = -1;
                }

                if advance_fenc {
                    advance_fenc = false;

                    if !eap.is_null() && (*eap).force_enc != 0 {
                        notconverted = true;
                        conv_error = 0;
                        if fenc_alloced {
                            xfree(fenc.cast());
                        }
                        fenc = c"".as_ptr().cast_mut();
                        fenc_alloced = false;
                    } else {
                        if fenc_alloced {
                            xfree(fenc.cast());
                        }
                        if !fenc_next.is_null() {
                            fenc = next_fenc(&raw mut fenc_next, &raw mut fenc_alloced);
                        } else {
                            fenc = c"".as_ptr().cast_mut();
                            fenc_alloced = false;
                        }
                    }
                    if !tmpname.is_null() {
                        os_remove(tmpname);
                        xfree(tmpname.cast());
                        tmpname = ptr::null_mut();
                    }
                }

                fio_flags = 0;
                converted = need_conversion(fenc);
                if converted {
                    if CStr::from_ptr(fenc) == c"ucs-bom" {
                        fio_flags = FIO_UCSBOM;
                    } else {
                        fio_flags = get_fio_flags(fenc);
                    }

                    if fio_flags == 0 && !did_iconv {
                        iconv_fd = my_iconv_open(c"utf-8".as_ptr().cast_mut(), fenc) as iconv_t;
                    }

                    if fio_flags == 0
                        && !read_stdin
                        && !read_buffer
                        && p_ccv.cb_type != kCallbackNone
                        && !read_fifo
                        && iconv_fd == -1
                    {
                        did_iconv = false;
                        if tmpname.is_null() {
                            tmpname = readfile_charconvert(fname, fenc, &raw mut fd);
                            if tmpname.is_null() {
                                advance_fenc = true;
                                if fd < 0 {
                                    emsg(tr(c"E202: Conversion made file unreadable!".as_ptr()));
                                    error = true;
                                    break 'retry;
                                }
                                continue 'retry;
                            }
                        }
                    } else if fio_flags == 0 && iconv_fd == -1 {
                        advance_fenc = true;
                        continue 'retry;
                    }
                }

                let can_retry = *fenc != NUL && !read_stdin && !keep_dest_enc && !read_fifo;

                if !skip_read {
                    linerest = 0;
                    filesize = 0;
                    skip_count = lines_to_skip;
                    read_count = lines_to_read;
                    conv_restlen = 0;
                    read_undo_file = newfile
                        && (flags & READ_KEEP_UNDO) == 0
                        && !(*curbuf).b_ffname.is_null()
                        && (*curbuf).b_p_udf != 0
                        && !filtering
                        && !read_fifo
                        && !read_stdin
                        && !read_buffer;
                    if read_undo_file {
                        sha256_start(&raw mut sha_ctx);
                    }
                }

                while !error && !got_int {
                    {
                        if !skip_read {
                            size = (0x10000 + linerest).min(0x100000);
                        }

                        if size < 0 || size + linerest + 1 < 0 || linerest >= MAXCOL - size {
                            split += 1;
                            *ptr = NL;
                            size = 1;
                        } else if !skip_read {
                            while size >= 10 {
                                new_buffer =
                                    verbose_try_malloc((size as usize) + (linerest as usize) + 1)
                                        .cast::<c_char>();
                                if !new_buffer.is_null() {
                                    break;
                                }
                                size /= 2;
                            }
                            if new_buffer.is_null() {
                                error = true;
                                break;
                            }
                            if linerest != 0 {
                                ptr::copy(ptr.offset(-linerest), new_buffer, linerest as usize);
                            }
                            xfree(buffer.cast());
                            buffer = new_buffer;
                            ptr = buffer.offset(linerest);
                            line_start = buffer;

                            real_size = size as c_int;
                            if iconv_fd != -1 {
                                size /= ICONV_MULT;
                            } else if (fio_flags & FIO_LATIN1) != 0 {
                                size /= 2;
                            } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                size = (size * 2 / 3) & !1;
                            } else if (fio_flags & FIO_UCS4) != 0 {
                                size = (size * 2 / 3) & !3;
                            } else if fio_flags == FIO_UCSBOM {
                                size /= ICONV_MULT;
                            }

                            if conv_restlen > 0 {
                                ptr::copy(conv_rest.as_ptr(), ptr, conv_restlen as usize);
                                ptr = ptr.offset(conv_restlen as isize);
                                size -= conv_restlen as isize;
                            }

                            if read_buffer {
                                if read_buf_lnum > from {
                                    size = 0;
                                } else {
                                    let mut tlen: isize = 0;
                                    loop {
                                        p = ml_get(read_buf_lnum)
                                            .cast::<u8>()
                                            .offset(read_buf_col as isize);
                                        let mut n =
                                            (ml_get_len(read_buf_lnum) - read_buf_col) as isize;
                                        if tlen + n + 1 > size {
                                            n = size - tlen;
                                            for ni in 0..n {
                                                let ch = *p.offset(ni);
                                                *ptr.offset(tlen) =
                                                    if ch == b'\n' { NUL } else { ch as c_char };
                                                tlen += 1;
                                            }
                                            read_buf_col += n as colnr_T;
                                            break;
                                        }

                                        for ni in 0..n {
                                            let ch = *p.offset(ni);
                                            *ptr.offset(tlen) =
                                                if ch == b'\n' { NUL } else { ch as c_char };
                                            tlen += 1;
                                        }
                                        *ptr.offset(tlen) = NL;
                                        tlen += 1;
                                        read_buf_col = 0;
                                        read_buf_lnum += 1;
                                        if read_buf_lnum > from {
                                            if (*curbuf).b_p_eol == 0 {
                                                tlen -= 1;
                                            }
                                            size = tlen;
                                            break;
                                        }
                                    }
                                }
                            } else {
                                let read_size = size as usize;
                                size = read_eintr(fd, ptr.cast(), read_size);
                            }

                            if size <= 0 {
                                if size < 0 {
                                    error = true;
                                } else if conv_restlen > 0 {
                                    if fio_flags != 0 || iconv_fd != -1 {
                                        if can_retry {
                                            if p_ccv.cb_type != kCallbackNone && iconv_fd != -1 {
                                                did_iconv = true;
                                            } else {
                                                advance_fenc = true;
                                            }
                                            file_rewind = true;
                                            continue 'retry;
                                        }
                                        if conv_error == 0 {
                                            conv_error = (*curbuf).b_ml.ml_line_count - linecnt + 1;
                                        }
                                    } else if illegal_byte == 0 {
                                        illegal_byte = (*curbuf).b_ml.ml_line_count - linecnt + 1;
                                    }
                                    if bad_char_behavior == BAD_DROP {
                                        *ptr.offset(-(conv_restlen as isize)) = NUL;
                                        conv_restlen = 0;
                                    } else {
                                        if bad_char_behavior != BAD_KEEP
                                            && (fio_flags != 0 || iconv_fd != -1)
                                        {
                                            while conv_restlen > 0 {
                                                ptr = ptr.sub(1);
                                                *ptr = bad_char_behavior as c_char;
                                                conv_restlen -= 1;
                                            }
                                        }
                                        fio_flags = 0;
                                        if iconv_fd != -1 {
                                            call_iconv_close(iconv_fd);
                                            iconv_fd = -1;
                                        }
                                    }
                                }
                            }
                        }

                        skip_read = false;

                        if filesize == 0
                            && (fio_flags == FIO_UCSBOM
                                || ((*curbuf).b_p_bomb == 0
                                    && tmpname.is_null()
                                    && (*fenc == b'u' as c_char || *fenc == NUL)))
                        {
                            let mut blen: c_int = 0;
                            let ccname = if size < 2 || (*curbuf).b_p_bin != 0 {
                                ptr::null()
                            } else {
                                check_for_bom(
                                    ptr,
                                    size as c_int,
                                    &raw mut blen,
                                    if fio_flags == FIO_UCSBOM {
                                        FIO_ALL
                                    } else {
                                        get_fio_flags(fenc)
                                    },
                                )
                            };
                            if !ccname.is_null() {
                                filesize += i64::from(blen);
                                size -= blen as isize;
                                ptr::copy(ptr.offset(blen as isize), ptr, size as usize);
                                if set_options {
                                    (*curbuf).b_p_bomb = 1;
                                    (*curbuf).b_start_bomb = 1;
                                }
                            }

                            if fio_flags == FIO_UCSBOM {
                                if ccname.is_null() {
                                    advance_fenc = true;
                                } else {
                                    if fenc_alloced {
                                        xfree(fenc.cast());
                                    }
                                    fenc = ccname.cast_mut();
                                    fenc_alloced = false;
                                }
                                skip_read = true;
                                continue 'retry;
                            }
                        }

                        ptr = ptr.offset(-(conv_restlen as isize));
                        size += conv_restlen as isize;
                        conv_restlen = 0;
                        if size <= 0 {
                            break;
                        }

                        if iconv_fd != -1 {
                            let mut fromp: *const c_char = ptr;
                            let mut from_size = size as usize;
                            ptr = ptr.offset(size);
                            let mut top: *mut c_char = ptr;
                            let mut to_size = (real_size as isize - size) as usize;

                            while (call_iconv(
                                iconv_fd,
                                &raw mut fromp,
                                &raw mut from_size,
                                &raw mut top,
                                &raw mut to_size,
                            ) == usize::MAX
                                && get_errno() != ICONV_EINVAL)
                                || from_size > CONV_RESTLEN
                            {
                                if can_retry {
                                    if p_ccv.cb_type != kCallbackNone && iconv_fd != -1 {
                                        did_iconv = true;
                                    } else {
                                        advance_fenc = true;
                                    }
                                    file_rewind = true;
                                    continue 'retry;
                                }
                                if conv_error == 0 {
                                    conv_error = readfile_linenr(linecnt, ptr, top);
                                }

                                fromp = fromp.add(1);
                                from_size -= 1;
                                if bad_char_behavior == BAD_KEEP {
                                    *top = *fromp.sub(1);
                                    top = top.add(1);
                                    to_size -= 1;
                                } else if bad_char_behavior != BAD_DROP {
                                    *top = bad_char_behavior as c_char;
                                    top = top.add(1);
                                    to_size -= 1;
                                }
                            }

                            if from_size > 0 {
                                ptr::copy(fromp, conv_rest.as_mut_ptr(), from_size);
                                conv_restlen = from_size as c_int;
                            }

                            line_start = ptr.offset(-linerest);
                            ptr::copy(buffer, line_start, linerest as usize);
                            size = top.offset_from(ptr);
                        }

                        if fio_flags != 0 {
                            let mut u8c: c_uint;
                            let mut tail: *mut c_char = ptr::null_mut();

                            let mut dest = ptr.offset(real_size as isize);
                            if fio_flags == FIO_LATIN1 || fio_flags == FIO_UTF8 {
                                p = ptr.cast::<u8>().offset(size);
                                if fio_flags == FIO_UTF8 {
                                    tail = ptr.offset(size - 1);
                                    while tail > ptr && (*tail as u8 & 0xc0) == 0x80 {
                                        tail = tail.sub(1);
                                    }
                                    if tail.offset(utf_byte2len(*tail as u8 as c_int) as isize)
                                        <= ptr.offset(size)
                                    {
                                        tail = ptr::null_mut();
                                    } else {
                                        p = tail.cast::<u8>();
                                    }
                                }
                            } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                p = ptr.cast::<u8>().offset(size & !1);
                                if (size & 1) != 0 {
                                    tail = p.cast::<c_char>();
                                }
                                if (fio_flags & FIO_UTF16) != 0 && p > ptr.cast::<u8>() {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p) << 8;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p);
                                    } else {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p);
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 8;
                                    }
                                    if (0xd800..=0xdbff).contains(&u8c) {
                                        tail = p.cast::<c_char>();
                                    } else {
                                        p = p.add(2);
                                    }
                                }
                            } else {
                                p = ptr.cast::<u8>().offset(size & !3);
                                if (size & 3) != 0 {
                                    tail = p.cast::<c_char>();
                                }
                            }

                            if !tail.is_null() {
                                conv_restlen = ptr.offset(size).offset_from(tail) as c_int;
                                ptr::copy(tail, conv_rest.as_mut_ptr(), conv_restlen as usize);
                                size -= conv_restlen as isize;
                            }

                            while p > ptr.cast::<u8>() {
                                if (fio_flags & FIO_LATIN1) != 0 {
                                    p = p.sub(1);
                                    u8c = c_uint::from(*p);
                                } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p) << 8;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p);
                                    } else {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p);
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 8;
                                    }
                                    if (fio_flags & FIO_UTF16) != 0
                                        && (0xdc00..=0xdfff).contains(&u8c)
                                    {
                                        if p == ptr.cast::<u8>() {
                                            if can_retry {
                                                if p_ccv.cb_type != kCallbackNone && iconv_fd != -1
                                                {
                                                    did_iconv = true;
                                                } else {
                                                    advance_fenc = true;
                                                }
                                                file_rewind = true;
                                                continue 'retry;
                                            }
                                            if conv_error == 0 {
                                                conv_error =
                                                    readfile_linenr(linecnt, ptr, p.cast());
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as c_uint;
                                            }
                                        }

                                        let mut u16c: c_int;
                                        if (fio_flags & FIO_ENDIAN_L) != 0 {
                                            p = p.sub(1);
                                            u16c = c_int::from(*p) << 8;
                                            p = p.sub(1);
                                            u16c += c_int::from(*p);
                                        } else {
                                            p = p.sub(1);
                                            u16c = c_int::from(*p);
                                            p = p.sub(1);
                                            u16c += c_int::from(*p) << 8;
                                        }
                                        u8c = 0x10000
                                            + (((u16c as c_uint) & 0x3ff) << 10)
                                            + (u8c & 0x3ff);

                                        if !(0xd800..=0xdbff).contains(&u16c) {
                                            if can_retry {
                                                if p_ccv.cb_type != kCallbackNone && iconv_fd != -1
                                                {
                                                    did_iconv = true;
                                                } else {
                                                    advance_fenc = true;
                                                }
                                                file_rewind = true;
                                                continue 'retry;
                                            }
                                            if conv_error == 0 {
                                                conv_error =
                                                    readfile_linenr(linecnt, ptr, p.cast());
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as c_uint;
                                            }
                                        }
                                    }
                                } else if (fio_flags & FIO_UCS4) != 0 {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p) << 24;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 16;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 8;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p);
                                    } else {
                                        p = p.sub(1);
                                        u8c = c_uint::from(*p);
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 8;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 16;
                                        p = p.sub(1);
                                        u8c += c_uint::from(*p) << 24;
                                    }
                                    if u8c > c_int::MAX as c_uint {
                                        u8c = 0xfffd;
                                    }
                                } else {
                                    p = p.sub(1);
                                    if *p < 0x80 {
                                        u8c = c_uint::from(*p);
                                    } else {
                                        let len = utf_head_off(ptr, p.cast());
                                        p = p.offset(-(len as isize));
                                        u8c = utf_ptr2char(p.cast()) as c_uint;
                                        if len == 0 {
                                            if can_retry {
                                                if p_ccv.cb_type != kCallbackNone && iconv_fd != -1
                                                {
                                                    did_iconv = true;
                                                } else {
                                                    advance_fenc = true;
                                                }
                                                file_rewind = true;
                                                continue 'retry;
                                            }
                                            if conv_error == 0 {
                                                conv_error =
                                                    readfile_linenr(linecnt, ptr, p.cast());
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as c_uint;
                                            }
                                        }
                                    }
                                }
                                debug_assert!(u8c <= c_int::MAX as c_uint);
                                dest = dest.offset(-(utf_char2len(u8c as c_int) as isize));
                                utf_char2bytes(u8c as c_int, dest);
                            }

                            line_start = dest.offset(-linerest);
                            ptr::copy(buffer, line_start, linerest as usize);
                            size = ptr.offset(real_size as isize).offset_from(dest);
                            ptr = dest;
                        } else if (*curbuf).b_p_bin == 0 {
                            let mut incomplete_tail = false;

                            p = ptr.cast::<u8>();
                            loop {
                                {
                                    let ascii_end = ptr.cast::<u8>().offset(size);
                                    while ascii_end.offset_from(p) >= size_of::<u64>() as isize {
                                        let word = ptr::read_unaligned(p.cast::<u64>());
                                        if (word & NONASCII_MASK) != 0 {
                                            break;
                                        }
                                        p = p.add(size_of::<u64>());
                                    }
                                    while p < ascii_end && *p < 0x80 {
                                        p = p.add(1);
                                    }
                                }

                                let todo = ptr.cast::<u8>().offset(size).offset_from(p) as c_int;
                                if todo <= 0 {
                                    break;
                                }
                                if *p >= 0x80 {
                                    let l = utf_ptr2len_len(p.cast(), todo);
                                    if l > todo && !incomplete_tail {
                                        if p > ptr.cast::<u8>() || filesize > 0 {
                                            incomplete_tail = true;
                                        }
                                        if p > ptr.cast::<u8>() {
                                            conv_restlen = todo;
                                            ptr::copy(
                                                p.cast::<c_char>(),
                                                conv_rest.as_mut_ptr(),
                                                conv_restlen as usize,
                                            );
                                            size -= conv_restlen as isize;
                                            break;
                                        }
                                    }
                                    if l == 1 || l > todo {
                                        if can_retry && !incomplete_tail {
                                            break;
                                        }

                                        if iconv_fd != -1 && conv_error == 0 {
                                            conv_error = readfile_linenr(linecnt, ptr, p.cast());
                                        }

                                        if conv_error == 0 && illegal_byte == 0 {
                                            illegal_byte = readfile_linenr(linecnt, ptr, p.cast());
                                        }

                                        if bad_char_behavior == BAD_DROP {
                                            ptr::copy(p.add(1), p, (todo - 1) as usize);
                                            size -= 1;
                                        } else {
                                            if bad_char_behavior != BAD_KEEP {
                                                *p = bad_char_behavior as u8;
                                            }
                                            p = p.add(1);
                                        }
                                    } else {
                                        p = p.offset(l as isize);
                                    }
                                }
                            }
                            if p < ptr.cast::<u8>().offset(size) && !incomplete_tail {
                                if p_ccv.cb_type != kCallbackNone && iconv_fd != -1 {
                                    did_iconv = true;
                                } else {
                                    advance_fenc = true;
                                }
                                file_rewind = true;
                                continue 'retry;
                            }
                        }

                        filesize += size as off_T;

                        if fileformat == EOL_UNKNOWN {
                            if try_dos != 0 || try_unix != 0 {
                                if try_mac != 0 {
                                    try_mac = 1;
                                }

                                p = ptr.cast::<u8>();
                                let end_p = ptr.cast::<u8>().offset(size);
                                while p < end_p {
                                    if *p == b'\n' {
                                        if try_unix == 0
                                            || (try_dos != 0
                                                && p > ptr.cast::<u8>()
                                                && *p.sub(1) == b'\r')
                                        {
                                            fileformat = EOL_DOS;
                                        } else {
                                            fileformat = EOL_UNIX;
                                        }
                                        break;
                                    } else if *p == b'\r' && try_mac != 0 {
                                        try_mac += 1;
                                    }
                                    p = p.add(1);
                                }

                                if fileformat == EOL_UNIX && try_mac != 0 {
                                    try_mac = 1;
                                    try_unix = 1;
                                    while p >= ptr.cast::<u8>() && *p != b'\r' {
                                        p = p.wrapping_sub(1);
                                    }
                                    if p >= ptr.cast::<u8>() {
                                        p = ptr.cast::<u8>();
                                        while p < end_p {
                                            if *p == b'\n' {
                                                try_unix += 1;
                                            } else if *p == b'\r' {
                                                try_mac += 1;
                                            }
                                            p = p.add(1);
                                        }
                                        if try_mac > try_unix {
                                            fileformat = EOL_MAC;
                                        }
                                    }
                                } else if fileformat == EOL_UNKNOWN && try_mac == 1 {
                                    fileformat = default_fileformat();
                                }
                            }

                            if fileformat == EOL_UNKNOWN && try_mac != 0 {
                                fileformat = EOL_MAC;
                            }

                            if fileformat == EOL_UNKNOWN {
                                fileformat = default_fileformat();
                            }

                            if set_options {
                                set_fileformat(fileformat, OPT_LOCAL);
                            }
                        }
                    }

                    if fileformat == EOL_MAC {
                        ptr = ptr.wrapping_sub(1);
                        loop {
                            ptr = ptr.wrapping_add(1);
                            size -= 1;
                            if size < 0 {
                                break;
                            }
                            let c = *ptr;
                            if c != NUL && c != CAR && c != NL {
                                continue;
                            }
                            if c == NUL {
                                *ptr = NL;
                            } else if c == NL {
                                *ptr = CAR;
                            } else {
                                if skip_count == 0 {
                                    *ptr = NUL;
                                    let len = (ptr.offset_from(line_start) + 1) as colnr_T;
                                    if ml_append(lnum, line_start, len, newfile) == FAIL {
                                        error = true;
                                        break;
                                    }
                                    if read_undo_file {
                                        sha256_update(
                                            &raw mut sha_ctx,
                                            line_start.cast::<u8>(),
                                            len as usize,
                                        );
                                    }
                                    lnum += 1;
                                    read_count -= 1;
                                    if read_count == 0 {
                                        error = true;
                                        line_start = ptr;
                                        break;
                                    }
                                } else {
                                    skip_count -= 1;
                                }
                                line_start = ptr.add(1);
                            }
                        }
                    } else {
                        let end = ptr.offset(size);

                        while ptr < end {
                            let rem = end.offset_from(ptr) as usize;
                            let slice = std::slice::from_raw_parts(ptr.cast::<u8>(), rem);
                            let Some(nl_pos) = slice.iter().position(|&b| b == b'\n') else {
                                let rem_slice =
                                    std::slice::from_raw_parts_mut(ptr.cast::<u8>(), rem);
                                for b in rem_slice {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                                ptr = end;
                                break;
                            };

                            let nl = ptr.add(nl_pos);
                            if nl_pos > 0 {
                                let scan_slice =
                                    std::slice::from_raw_parts_mut(ptr.cast::<u8>(), nl_pos);
                                for b in scan_slice {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                            }

                            ptr = nl;
                            if skip_count == 0 {
                                *ptr = NUL;
                                let mut len = (ptr.offset_from(line_start) + 1) as colnr_T;
                                if fileformat == EOL_DOS {
                                    if ptr > line_start && *ptr.sub(1) == CAR {
                                        *ptr.sub(1) = NUL;
                                        len -= 1;
                                    } else if ff_error != EOL_DOS {
                                        if try_unix != 0
                                            && !read_stdin
                                            && (read_buffer || vim_lseek(fd, 0, SEEK_SET) == 0)
                                        {
                                            fileformat = EOL_UNIX;
                                            if set_options {
                                                set_fileformat(EOL_UNIX, OPT_LOCAL);
                                            }
                                            file_rewind = true;
                                            keep_fileformat = true;
                                            continue 'retry;
                                        }
                                        ff_error = EOL_DOS;
                                    }
                                }
                                if ml_append(lnum, line_start, len, newfile) == FAIL {
                                    error = true;
                                    break;
                                }
                                if read_undo_file {
                                    sha256_update(
                                        &raw mut sha_ctx,
                                        line_start.cast::<u8>(),
                                        len as usize,
                                    );
                                }
                                lnum += 1;
                                read_count -= 1;
                                if read_count == 0 {
                                    error = true;
                                    line_start = ptr;
                                    break;
                                }
                            } else {
                                skip_count -= 1;
                            }
                            line_start = ptr.add(1);
                            ptr = ptr.add(1);
                        }
                        size = -1;
                    }
                    linerest = ptr.offset_from(line_start);

                    if show_msg && (progress || os_hrtime() - read_start > slow_io_ns()) {
                        let mut info: FileInfo = std::mem::zeroed();
                        let total = if os_fileinfo_fd(fd, &raw mut info) {
                            os_fileinfo_size(&raw const info)
                        } else {
                            0
                        };
                        let pct = ((filesize as u64) * 100)
                            .checked_div(total)
                            .map_or(-1, |v| (v.min(100)) as c_int);
                        if !progress || pct != percent {
                            progress = true;
                            percent = pct;
                            let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                            add_quoted_fname(iobuf_ptr, IOSIZE, curbuf, sfname);
                            let _ = readfile_progress(
                                iobuf_ptr,
                                msg_id.as_ptr(),
                                c"running".as_ptr(),
                                percent,
                                false,
                            );
                        }
                    }

                    os_breakcheck();
                }

                break 'retry;
            }

            if error && read_count == 0 {
                error = false;
            }

            if linerest != 0
                && (*curbuf).b_p_bin == 0
                && fileformat == EOL_DOS
                && *ptr.sub(1) == CTRL_Z
            {
                ptr = ptr.sub(1);
                linerest -= 1;
                if set_options {
                    (*curbuf).b_p_eof = 1;
                }
            }

            if !error && !got_int && linerest != 0 {
                if set_options {
                    (*curbuf).b_p_eol = 0;
                }
                *ptr = NUL;
                let len = (ptr.offset_from(line_start) + 1) as colnr_T;
                if ml_append(lnum, line_start, len, newfile) == FAIL {
                    error = true;
                } else {
                    if read_undo_file {
                        sha256_update(&raw mut sha_ctx, line_start.cast::<u8>(), len as usize);
                    }
                    lnum += 1;
                    read_no_eol_lnum = lnum;
                }
            }

            if set_options {
                save_file_ff(curbuf);
                set_option_direct(kOptFileencoding, cstr_as_obj(fenc), OPT_LOCAL, 0);
            }
            if fenc_alloced {
                xfree(fenc.cast());
            }
            if iconv_fd != -1 {
                call_iconv_close(iconv_fd);
            }

            if !read_buffer && !read_stdin {
                close_fd(fd);
            } else {
                os_set_cloexec(fd);
            }
            xfree(buffer.cast());

            if read_stdin {
                close_fd(fd);
                if stdin_fd < 0 {
                    #[cfg(not(windows))]
                    {
                        vim_ignored = dup(2);
                    }
                    #[cfg(windows)]
                    {
                        vim_ignored = os_open_conin_fd();
                    }
                }
            }

            if !tmpname.is_null() {
                os_remove(tmpname);
                xfree(tmpname.cast());
            }
            no_wait_return -= 1;

            if !recoverymode {
                if newfile && wasempty && ((*curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
                    ml_delete((*curbuf).b_ml.ml_line_count);
                    linecnt -= 1;
                }
                (*curbuf).deleted_bytes = 0;
                (*curbuf).deleted_bytes2 = 0;
                (*curbuf).deleted_codepoints = 0;
                (*curbuf).deleted_codeunits = 0;
                linecnt = (*curbuf).b_ml.ml_line_count - linecnt;
                if filesize == 0 {
                    linecnt = 0;
                }
                if newfile || read_buffer {
                    redraw_curbuf_later(UPD_NOT_VALID);
                    diff_invalidate(curbuf);
                    foldUpdateAll(curwin);
                } else if linecnt != 0 {
                    appended_lines_mark(from, linecnt);
                }

                if got_int {
                    if progress {
                        let _ = readfile_progress(
                            ptr::null(),
                            msg_id.as_ptr(),
                            c"cancel".as_ptr(),
                            -1,
                            false,
                        );
                    }
                    if (flags & READ_DUMMY) == 0 {
                        filemess(curbuf, sfname, tr(e_interr.as_ptr()), ptr::null());
                        if newfile {
                            (*curbuf).b_p_ro = 1;
                        }
                    }
                    msg_scroll = msg_save;
                    check_marks_read();
                    retval = OK;
                    break 'theend;
                }

                if !filtering && (flags & READ_DUMMY) == 0 && !silent {
                    let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                    add_quoted_fname(iobuf_ptr, IOSIZE, curbuf, sfname);
                    let mut c = false;

                    let mut buflen = c_strlen(iobuf_ptr) as c_int;
                    #[cfg(unix)]
                    {
                        if s_isfifo(perm) {
                            buflen += vim_snprintf!(
                                iobuf_ptr.offset(buflen as isize),
                                (IOSIZE as c_int - buflen) as usize,
                                c"%s".as_ptr(),
                                tr(c"[fifo]".as_ptr()),
                            );
                            c = true;
                        }
                        if s_issock(perm) {
                            buflen += vim_snprintf!(
                                iobuf_ptr.offset(buflen as isize),
                                (IOSIZE as c_int - buflen) as usize,
                                c"%s".as_ptr(),
                                tr(c"[socket]".as_ptr()),
                            );
                            c = true;
                        }
                        #[cfg(any(
                            target_os = "freebsd",
                            target_os = "openbsd",
                            target_os = "netbsd",
                            target_os = "dragonfly",
                            target_os = "macos"
                        ))]
                        if s_ischr(perm) {
                            buflen += vim_snprintf!(
                                iobuf_ptr.offset(buflen as isize),
                                (IOSIZE as c_int - buflen) as usize,
                                c"%s".as_ptr(),
                                tr(c"[character special]".as_ptr()),
                            );
                            c = true;
                        }
                    }
                    if (*curbuf).b_p_ro != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            if shortmess(kShmRo) {
                                tr(c"[RO]".as_ptr())
                            } else {
                                tr(c"[readonly]".as_ptr())
                            },
                        );
                        c = true;
                    }
                    if read_no_eol_lnum != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[noeol]".as_ptr()),
                        );
                        c = true;
                    }
                    if ff_error == EOL_DOS {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[CR missing]".as_ptr()),
                        );
                        c = true;
                    }
                    if split != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[long lines split]".as_ptr()),
                        );
                        c = true;
                    }
                    if notconverted {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[NOT converted]".as_ptr()),
                        );
                        c = true;
                    } else if converted {
                        buflen += vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[converted]".as_ptr()),
                        );
                        c = true;
                    }
                    if conv_error != 0 {
                        vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            tr(c"[CONVERSION ERROR in line %lld]".as_ptr()),
                            i64::from(conv_error),
                        );
                        c = true;
                    } else if illegal_byte > 0 {
                        vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            tr(c"[ILLEGAL BYTE in line %lld]".as_ptr()),
                            i64::from(illegal_byte),
                        );
                        c = true;
                    } else if error {
                        vim_snprintf!(
                            iobuf_ptr.offset(buflen as isize),
                            (IOSIZE as c_int - buflen) as usize,
                            c"%s".as_ptr(),
                            tr(c"[READ ERRORS]".as_ptr()),
                        );
                        c = true;
                    }
                    if msg_add_fileformat(fileformat) {
                        c = true;
                    }

                    msg_add_lines(c_int::from(c), linecnt, filesize);

                    xfree(keep_msg.cast());
                    keep_msg = ptr::null_mut();
                    p = ptr::null_mut();
                    msg_scrolled_ign = true;

                    if !read_stdin && !read_buffer {
                        if msg_col > 0 {
                            msg_putchar(b'\r' as c_int);
                        }
                        p = (if progress {
                            readfile_progress(
                                iobuf_ptr,
                                msg_id.as_ptr(),
                                if error {
                                    c"failed".as_ptr()
                                } else {
                                    c"success".as_ptr()
                                },
                                -1,
                                true,
                            )
                        } else {
                            msg_trunc(iobuf_ptr, false, 0)
                        })
                        .cast::<u8>();
                    }

                    if read_stdin
                        || read_buffer
                        || restart_edit != 0
                        || (msg_scrolled != 0 && !need_wait_return)
                    {
                        set_keep_msg(p.cast(), 0);
                    }
                    msg_scrolled_ign = false;
                } else if progress {
                    let _ = readfile_progress(
                        ptr::null(),
                        msg_id.as_ptr(),
                        if error {
                            c"failed".as_ptr()
                        } else {
                            c"success".as_ptr()
                        },
                        -1,
                        false,
                    );
                }

                if newfile
                    && (error
                        || conv_error != 0
                        || (illegal_byte > 0 && bad_char_behavior != BAD_KEEP))
                {
                    (*curbuf).b_p_ro = 1;
                }

                u_clearline(curbuf);

                (*curwin).w_cursor.lnum = from + 1;
                check_cursor_lnum(curwin);
                beginline(BL_WHITE | BL_FIX);

                if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                    (*curbuf).b_op_start.lnum = from + 1;
                    (*curbuf).b_op_start.col = 0;
                    (*curbuf).b_op_end.lnum = from + linecnt;
                    (*curbuf).b_op_end.col = 0;
                }
            }
            msg_scroll = msg_save;

            check_marks_read();

            (*curbuf).b_no_eol_lnum = read_no_eol_lnum;

            if (flags & READ_KEEP_UNDO) != 0 {
                u_find_first_changed();
            }

            if read_undo_file {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                sha256_finish(&raw mut sha_ctx, hash.as_mut_ptr());
                u_read_undo(ptr::null_mut(), hash.as_ptr(), fname);
            }

            if !read_stdin && !read_fifo && (!read_buffer || !sfname.is_null()) {
                let m = msg_scroll;
                let n = msg_scrolled;

                if set_options {
                    save_file_ff(curbuf);
                }

                msg_scroll = 1;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPOST,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );
                } else if newfile || (read_buffer && !sfname.is_null()) {
                    let did_bufrp = apply_autocmds_exarg(
                        EVENT_BUFREADPOST,
                        ptr::null_mut(),
                        sfname,
                        false,
                        curbuf,
                        eap,
                    );

                    if !(*curbuf).b_au_did_filetype && *(*curbuf).b_p_ft != NUL {
                        apply_autocmds(
                            EVENT_FILETYPE,
                            (*curbuf).b_p_ft,
                            (*curbuf).b_fname,
                            true,
                            curbuf,
                        );
                    }

                    if !did_bufrp
                        && *(*curbuf).b_p_ft == NUL
                        && augroup_exists(c"filetypedetect".as_ptr())
                    {
                        apply_autocmds_group(
                            EVENT_BUFREADPOST,
                            sfname,
                            ptr::null_mut(),
                            true,
                            augroup_find(c"filetypedetect".as_ptr()),
                            curbuf,
                            curwin,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            false,
                        );
                    }
                } else {
                    apply_autocmds_exarg(
                        EVENT_FILEREADPOST,
                        sfname,
                        sfname,
                        false,
                        ptr::null_mut(),
                        eap,
                    );
                }
                if msg_scrolled == n {
                    msg_scroll = m;
                }
                if aborting() {
                    return FAIL;
                }
            }

            if !(recoverymode && error) {
                retval = OK;
            }
        }

        let mfp = (*curbuf).b_ml.ml_mfp.cast::<memfile_T>();
        if !mfp.is_null() && (*mfp).mf_dirty == MF_DIRTY_YES_NOSYNC {
            (*mfp).mf_dirty = MF_DIRTY_YES;
        }

        retval
    }
}

/// Returns true if the file name argument is of the form "/dev/fd/\d\+".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn is_dev_fd_file(fname: *mut c_char) -> bool {
    unsafe {
        let bytes = CStr::from_ptr(fname).to_bytes();
        bytes.starts_with(b"/dev/fd/")
            && bytes.len() > 8
            && ascii_isdigit(bytes[8])
            && *skipdigits(fname.add(9)) == NUL
            && (*fname.add(9) != NUL
                || (*fname.add(8) != b'0' as c_char
                    && *fname.add(8) != b'1' as c_char
                    && *fname.add(8) != b'2' as c_char))
    }
}

/// Estimate the line number where we are now.
unsafe fn readfile_linenr(linecnt: linenr_T, p: *mut c_char, endp: *const c_char) -> linenr_T {
    unsafe {
        let mut lnum = (*curbuf).b_ml.ml_line_count - linecnt + 1;
        let mut s: *const c_char = p;
        while s < endp {
            if *s == b'\n' as c_char {
                lnum += 1;
            }
            s = s.add(1);
        }
        lnum
    }
}

/// Fill "*eap" to force the 'fileencoding', 'fileformat' and 'binary' to be
/// equal to the buffer "buf".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prep_exarg(eap: *mut exarg_T, buf: *const buf_T) {
    unsafe {
        let fenc_bytes = CStr::from_ptr((*buf).b_p_fenc).to_bytes();
        let cmd_len = 15 + fenc_bytes.len();
        let cmd = xmalloc(cmd_len).cast::<u8>();
        let prefix = b"e ++enc=";
        ptr::copy_nonoverlapping(prefix.as_ptr(), cmd, prefix.len());
        ptr::copy_nonoverlapping(fenc_bytes.as_ptr(), cmd.add(prefix.len()), fenc_bytes.len());
        *cmd.add(prefix.len() + fenc_bytes.len()) = 0;
        (*eap).cmd = cmd.cast::<c_char>();
        (*eap).force_enc = 8;
        (*eap).bad_char = (*buf).b_bad_char;
        (*eap).force_ff = (*(*buf).b_p_ff as u8) as c_int;
        (*eap).force_bin = if (*buf).b_p_bin != 0 {
            FORCE_BIN
        } else {
            FORCE_NOBIN
        };
        (*eap).read_edit = 0;
        (*eap).forceit = 0;
    }
}

/// Set default or forced 'fileformat' and 'binary'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_file_options(set_options: bool, eap: *mut exarg_T) {
    unsafe {
        if set_options {
            if !eap.is_null() && (*eap).force_ff != 0 {
                set_fileformat(get_fileformat_force(curbuf, eap), OPT_LOCAL);
            } else if *p_ffs != NUL {
                set_fileformat(default_fileformat(), OPT_LOCAL);
            }
        }

        if !eap.is_null() && (*eap).force_bin != 0 {
            let oldval = (*curbuf).b_p_bin;
            (*curbuf).b_p_bin = c_int::from((*eap).force_bin == FORCE_BIN);
            set_options_bin(oldval, (*curbuf).b_p_bin, OPT_LOCAL);
        }
    }
}

/// Set forced 'fileencoding'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_forced_fenc(eap: *mut exarg_T) {
    unsafe {
        if (*eap).force_enc == 0 {
            return;
        }

        let fenc = enc_canonize((*eap).cmd.add((*eap).force_enc as usize));
        set_option_direct(kOptFileencoding, cstr_as_obj(fenc), OPT_LOCAL, 0);
        xfree(fenc.cast());
    }
}

/// Find next fileencoding to use from 'fileencodings'.
unsafe fn next_fenc(pp: *mut *mut c_char, alloced: *mut bool) -> *mut c_char {
    unsafe {
        *alloced = false;
        if **pp == NUL {
            *pp = ptr::null_mut();
            return c"".as_ptr().cast_mut();
        }
        let p = vim_strchr(*pp, b',' as c_int);
        let r = if p.is_null() {
            let res = enc_canonize(*pp);
            *pp = (*pp).add(c_strlen(*pp));
            res
        } else {
            let len = p.offset_from(*pp) as usize;
            let tmp = xmemdupz((*pp).cast(), len).cast::<c_char>();
            *pp = p.add(1);
            let res = enc_canonize(tmp);
            xfree(tmp.cast());
            res
        };
        *alloced = true;
        r
    }
}

/// Convert a file with the 'charconvert' expression.
unsafe fn readfile_charconvert(
    fname: *mut c_char,
    fenc: *mut c_char,
    fdp: *mut c_int,
) -> *mut c_char {
    unsafe {
        let mut errmsg: *mut c_char = ptr::null_mut();

        let mut tmpname = vim_tempname();
        if tmpname.is_null() {
            errmsg = tr(c"Can't find temp file for conversion".as_ptr());
        } else {
            close_fd(*fdp);
            *fdp = -1;
            if eval_charconvert(fenc, c"utf-8".as_ptr(), fname, tmpname) == FAIL {
                errmsg = tr(c"Conversion with 'charconvert' failed".as_ptr());
            }
            if errmsg.is_null() {
                *fdp = os_open(tmpname, O_RDONLY, 0);
                if *fdp < 0 {
                    errmsg = tr(c"can't read output of 'charconvert'".as_ptr());
                }
            }
        }

        if !errmsg.is_null() {
            msg(errmsg, 0);
            if !tmpname.is_null() {
                os_remove(tmpname);
                xfree(tmpname.cast());
                tmpname = ptr::null_mut();
            }
        }

        if *fdp < 0 {
            *fdp = os_open(fname, O_RDONLY, 0);
        }

        tmpname
    }
}

/// Set the name of the current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_rw_fname(fname: *mut c_char, sfname: *mut c_char) -> c_int {
    unsafe {
        let buf = curbuf;

        if (*curbuf).b_p_bl != 0 {
            apply_autocmds(
                EVENT_BUFDELETE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
        apply_autocmds(
            EVENT_BUFWIPEOUT,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        if aborting() {
            return FAIL;
        }
        if curbuf != buf {
            emsg(tr(E_AUCHANGEDBUF.as_ptr()));
            return FAIL;
        }

        if setfname(curbuf, fname, sfname, false) == OK {
            (*curbuf).b_flags |= BF_NOTEDITED;
        }

        apply_autocmds(
            EVENT_BUFNEW,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        if (*curbuf).b_p_bl != 0 {
            apply_autocmds(
                EVENT_BUFADD,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
        if aborting() {
            return FAIL;
        }

        if *(*curbuf).b_p_ft == NUL {
            if augroup_exists(c"filetypedetect".as_ptr()) {
                do_doautocmd(
                    c"filetypedetect BufRead".as_ptr().cast_mut(),
                    false,
                    ptr::null_mut(),
                );
            }
            do_modelines(0);
        }

        OK
    }
}

/// Put file name into the specified buffer with quotes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn add_quoted_fname(
    ret_buf: *mut c_char,
    bufsize: usize,
    buf: *const buf_T,
    mut fname: *const c_char,
) {
    unsafe {
        if fname.is_null() {
            fname = c"-stdin-".as_ptr();
        }
        let mut len: usize = 0;
        *ret_buf.add(len) = b'"' as c_char;
        len += 1;
        len += home_replace(buf.cast_mut(), fname, ret_buf.add(len), bufsize - 4, true);
        xstrlcpy(ret_buf.add(len), c"\" ".as_ptr(), bufsize - len);
    }
}

/// Sets progress-message id of reading/writing file `fname`, e.g. `nvim.bufread "foo.txt"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fname_msg_id(
    msg_id: *mut c_char,
    size: usize,
    prefix: *const c_char,
    buf: *const buf_T,
    fname: *const c_char,
) {
    unsafe {
        let mut len = xstrlcpy(msg_id, prefix, size);
        if len < size {
            len += xstrlcpy(msg_id.add(len), c" ".as_ptr(), size - len);
        }
        if len < size {
            add_quoted_fname(msg_id.add(len), size - len, buf, fname);
        }
        let total = c_strlen(msg_id);
        if total > 0 {
            *msg_id.add(total - 1) = NUL;
        }
    }
}

/// Append message for text mode to IObuff.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn msg_add_fileformat(eol_type: c_int) -> bool {
    unsafe {
        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
        #[cfg(not(windows))]
        if eol_type == EOL_DOS {
            xstrlcat(iobuf_ptr, tr(c"[dos]".as_ptr()), IOSIZE);
            return true;
        }
        if eol_type == EOL_MAC {
            xstrlcat(iobuf_ptr, tr(c"[mac]".as_ptr()), IOSIZE);
            return true;
        }
        #[cfg(windows)]
        if eol_type == EOL_UNIX {
            xstrlcat(iobuf_ptr, tr(c"[unix]".as_ptr()), IOSIZE);
            return true;
        }
        false
    }
}

/// Append line and character count to IObuff.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn msg_add_lines(insert_space: c_int, lnum: linenr_T, nchars: off_T) {
    unsafe {
        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
        let mut len = c_strlen(iobuf_ptr);
        let space = if insert_space != 0 {
            c" ".as_ptr()
        } else {
            c"".as_ptr()
        };

        if shortmess(kShmLines) {
            vim_snprintf!(
                iobuf_ptr.add(len),
                IOSIZE - len,
                tr(c"%s%lldL, %lldB".as_ptr()),
                space,
                i64::from(lnum),
                nchars,
            );
        } else {
            len += vim_snprintf!(
                iobuf_ptr.add(len),
                IOSIZE - len,
                ngettext(
                    c"%s%lld line, ".as_ptr(),
                    c"%s%lld lines, ".as_ptr(),
                    lnum as usize,
                ),
                space,
                i64::from(lnum),
            ) as usize;
            vim_snprintf!(
                iobuf_ptr.add(len),
                IOSIZE - len,
                ngettext(
                    c"%lld byte".as_ptr(),
                    c"%lld bytes".as_ptr(),
                    nchars as usize,
                ),
                nchars,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn time_differs(
    file_info: *const FileInfo,
    mtime: i64,
    mtime_ns: i64,
) -> bool {
    unsafe {
        let sec = i64::from((*file_info).stat.st_mtim.tv_sec);
        let nsec = i64::from((*file_info).stat.st_mtim.tv_nsec);
        #[cfg(any(target_os = "linux", windows))]
        {
            nsec != mtime_ns || sec - mtime > 1 || mtime - sec > 1
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            nsec != mtime_ns || sec != mtime
        }
    }
}

/// Return true if file encoding "fenc" requires conversion from or to 'encoding'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn need_conversion(fenc: *const c_char) -> bool {
    unsafe {
        let same_encoding: bool;
        let fenc_flags: c_int;

        if *fenc == NUL || CStr::from_ptr(p_enc) == CStr::from_ptr(fenc) {
            same_encoding = true;
            fenc_flags = 0;
        } else {
            let enc_flags = get_fio_flags(p_enc);
            fenc_flags = get_fio_flags(fenc);
            same_encoding = enc_flags != 0 && fenc_flags == enc_flags;
        }
        if same_encoding {
            return false;
        }

        fenc_flags != FIO_UTF8
    }
}

/// Return the FIO_ flags needed for the internal conversion if 'name' was
/// unicode or latin1, otherwise 0.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_fio_flags(mut name: *const c_char) -> c_int {
    unsafe {
        #[cfg(not(test))]
        if *name == NUL {
            name = p_enc;
        }
        #[cfg(test)]
        if *name == NUL {
            name = c"utf-8".as_ptr();
        }
        let prop = enc_canon_props(name);
        if (prop & ENC_UNICODE) != 0 {
            if (prop & ENC_2BYTE) != 0 {
                if (prop & ENC_ENDIAN_L) != 0 {
                    return FIO_UCS2 | FIO_ENDIAN_L;
                }
                return FIO_UCS2;
            }
            if (prop & ENC_4BYTE) != 0 {
                if (prop & ENC_ENDIAN_L) != 0 {
                    return FIO_UCS4 | FIO_ENDIAN_L;
                }
                return FIO_UCS4;
            }
            if (prop & ENC_2WORD) != 0 {
                if (prop & ENC_ENDIAN_L) != 0 {
                    return FIO_UTF16 | FIO_ENDIAN_L;
                }
                return FIO_UTF16;
            }
            return FIO_UTF8;
        }
        if (prop & ENC_LATIN1) != 0 {
            return FIO_LATIN1;
        }
        0
    }
}

/// Check for a Unicode BOM (Byte Order Mark) at the start of p[size].
unsafe fn check_for_bom(
    p_in: *const c_char,
    size: c_int,
    lenp: *mut c_int,
    flags: c_int,
) -> *const c_char {
    unsafe {
        let p = p_in.cast::<u8>();
        let mut name: *const c_char = ptr::null();
        let mut len: c_int = 2;

        if *p == 0xef
            && *p.add(1) == 0xbb
            && size >= 3
            && *p.add(2) == 0xbf
            && (flags == FIO_ALL || flags == FIO_UTF8 || flags == 0)
        {
            name = c"utf-8".as_ptr();
            len = 3;
        } else if *p == 0xff && *p.add(1) == 0xfe {
            if size >= 4
                && *p.add(2) == 0
                && *p.add(3) == 0
                && (flags == FIO_ALL || flags == (FIO_UCS4 | FIO_ENDIAN_L))
            {
                name = c"ucs-4le".as_ptr();
                len = 4;
            } else if flags == (FIO_UCS2 | FIO_ENDIAN_L) {
                name = c"ucs-2le".as_ptr();
            } else if flags == FIO_ALL || flags == (FIO_UTF16 | FIO_ENDIAN_L) {
                name = c"utf-16le".as_ptr();
            }
        } else if *p == 0xfe
            && *p.add(1) == 0xff
            && (flags == FIO_ALL || flags == FIO_UCS2 || flags == FIO_UTF16)
        {
            if flags == FIO_UCS2 {
                name = c"ucs-2".as_ptr();
            } else {
                name = c"utf-16".as_ptr();
            }
        } else if size >= 4
            && *p == 0
            && *p.add(1) == 0
            && *p.add(2) == 0xfe
            && *p.add(3) == 0xff
            && (flags == FIO_ALL || flags == FIO_UCS4)
        {
            name = c"ucs-4".as_ptr();
            len = 4;
        }

        *lenp = len;
        name
    }
}

/// Shorten filename of a buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shorten_buf_fname(
    buf: *mut buf_T,
    dirname: *mut c_char,
    force: c_int,
) -> bool {
    unsafe {
        if (*buf).b_fname.is_null()
            || bt_nofilename(buf)
            || path_with_url((*buf).b_fname) != 0
            || !(force != 0 || (*buf).b_sfname.is_null() || path_is_absolute((*buf).b_sfname))
        {
            return false;
        }

        let p = path_shorten_fname((*buf).b_ffname, dirname);
        let shorten = !p.is_null() && *p != NUL;
        if shorten
            && !(*buf).b_sfname.is_null()
            && CStr::from_ptr(p) == CStr::from_ptr((*buf).b_sfname)
        {
            return false;
        }

        let new_fname = if shorten { p } else { (*buf).b_ffname };
        let changed =
            new_fname.is_null() || CStr::from_ptr(new_fname) != CStr::from_ptr((*buf).b_fname);

        if (*buf).b_sfname != (*buf).b_ffname {
            xfree((*buf).b_sfname.cast());
            (*buf).b_sfname = ptr::null_mut();
        }
        if shorten {
            (*buf).b_sfname = xstrdup(p);
            (*buf).b_fname = (*buf).b_sfname;
        } else {
            (*buf).b_fname = (*buf).b_ffname;
        }
        changed
    }
}

/// Shortens filenames for all buffers.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shorten_fnames(force: c_int) {
    unsafe {
        let mut dirname: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut changed = false;

        os_dirname(dirname.as_mut_ptr(), MAXPATHL);
        let mut buf = firstbuf;
        while !buf.is_null() {
            changed |= shorten_buf_fname(buf, dirname.as_mut_ptr(), force);
            mf_fullname((*buf).b_ml.ml_mfp.cast::<memfile_T>());
            buf = (*buf).b_next;
        }
        if changed {
            status_redraw_all();
            redraw_tabline = true;
        }
    }
}

/// Get new filename ended by given extension.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn modname(
    fname: *const c_char,
    ext: *const c_char,
    mut prepend_dot: bool,
) -> *mut c_char {
    unsafe {
        let retval: *mut c_char;
        let fnamelen: usize;
        let extlen = c_strlen(ext);

        if fname.is_null() || *fname == NUL {
            retval = xmalloc(MAXPATHL + extlen + 3).cast::<c_char>();
            if os_dirname(retval, MAXPATHL) == FAIL || c_strlen(retval) == 0 {
                xfree(retval.cast());
                return ptr::null_mut();
            }
            add_pathsep(retval);
            fnamelen = c_strlen(retval);
            prepend_dot = false;
        } else {
            fnamelen = c_strlen(fname);
            retval = xmalloc(fnamelen + extlen + 3).cast::<c_char>();
            ptr::copy_nonoverlapping(fname, retval, fnamelen + 1);
        }

        let mut ptr = retval.add(fnamelen);
        while ptr > retval {
            if vim_ispathsep(c_int::from(*ptr)) {
                ptr = ptr.add(1);
                break;
            }
            ptr = ptr.offset(-((utf_head_off(retval, ptr.sub(1)) + 1) as isize));
        }

        let mut ptrlen = fnamelen - ptr.offset_from(retval) as usize;
        if ptrlen > BASENAMELEN {
            ptrlen = BASENAMELEN;
            *ptr.add(ptrlen) = NUL;
        }

        let mut s = ptr.add(ptrlen);

        ptr::copy_nonoverlapping(ext, s, extlen + 1);

        let e = path_tail(retval);
        if prepend_dot && *e != b'.' as c_char {
            let move_len = ((fnamelen + extlen) - e.offset_from(retval) as usize) + 1;
            ptr::copy(e, e.add(1), move_len);
            *e = b'.' as c_char;
        }

        if !fname.is_null() && CStr::from_ptr(fname) == CStr::from_ptr(retval) {
            loop {
                s = s.sub(1);
                if s < ptr {
                    break;
                }
                if *s != b'_' as c_char {
                    *s = b'_' as c_char;
                    break;
                }
            }
            if s < ptr {
                *ptr = b'v' as c_char;
            }
        }
        retval
    }
}

/// Like fgets(), but if the file line is too long, it is truncated and the
/// rest of the line is thrown away.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_fgets(buf: *mut c_char, size: c_int, fp: *mut FILE) -> bool {
    unsafe {
        if size < 2 {
            if size == 1 {
                *buf = NUL;
            }
            return true;
        }

        *buf.offset((size - 2) as isize) = NUL;

        let mut retval: *mut c_char;
        loop {
            set_errno(0);
            retval = fgets(buf, size, fp);
            if !retval.is_null() || get_errno() != EINTR || ferror(fp) == 0 {
                break;
            }
        }

        let penultimate = *buf.offset((size - 2) as isize);
        if penultimate != NUL && penultimate != b'\n' as c_char {
            let mut tbuf: [c_char; 200] = [0; 200];

            *buf.offset((size - 1) as isize) = NUL;

            loop {
                tbuf[tbuf.len() - 2] = NUL;
                set_errno(0);
                retval = fgets(tbuf.as_mut_ptr(), tbuf.len() as c_int, fp);
                if retval.is_null() && (feof(fp) != 0 || get_errno() != EINTR) {
                    break;
                }
                if tbuf[tbuf.len() - 2] == NUL || tbuf[tbuf.len() - 2] == b'\n' as c_char {
                    break;
                }
            }
        }
        retval.is_null()
    }
}

/// Read 2 bytes from "fd" and turn them into an int, MSB first.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get2c(fd: *mut FILE) -> c_int {
    unsafe {
        let n = fgetc(fd);
        if n == EOF {
            return -1;
        }
        let c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        (n << 8) + c
    }
}

/// Read 3 bytes from "fd" and turn them into an int, MSB first.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get3c(fd: *mut FILE) -> c_int {
    unsafe {
        let mut n = fgetc(fd);
        if n == EOF {
            return -1;
        }
        let mut c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        n = (n << 8) + c;
        c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        (n << 8) + c
    }
}

/// Read 4 bytes from "fd" and turn them into an int, MSB first.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get4c(fd: *mut FILE) -> c_int {
    unsafe {
        let mut c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        let mut n = c as c_uint;
        c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        n = (n << 8) + (c as c_uint);
        c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        n = (n << 8) + (c as c_uint);
        c = fgetc(fd);
        if c == EOF {
            return -1;
        }
        n = (n << 8) + (c as c_uint);
        n as c_int
    }
}

/// Read 8 bytes from `fd` and turn them into a time_t, MSB first.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get8ctime(fd: *mut FILE) -> time_t {
    unsafe {
        let mut n: time_t = 0;

        for _ in 0..8 {
            let c = fgetc(fd);
            if c == EOF {
                return -1;
            }
            n = (n << 8) + i64::from(c);
        }
        n
    }
}

/// Reads a string of length "cnt" from "fd" into allocated memory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn read_string(fd: *mut FILE, cnt: usize) -> *mut c_char {
    unsafe {
        let str_ = xmallocz(cnt).cast::<c_char>();
        for i in 0..cnt {
            let c = fgetc(fd);
            if c == EOF {
                xfree(str_.cast());
                return ptr::null_mut();
            }
            *str_.add(i) = c as c_char;
        }
        str_
    }
}

/// Writes a number to file "fd", most significant bit first, in "len" bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_bytes(fd: *mut FILE, number: u64, len: usize) -> bool {
    unsafe {
        debug_assert!(len > 0);
        for i in (0..len).rev() {
            if fputc(((number >> (i * 8)) & 0xff) as c_int, fd) == EOF {
                return false;
            }
        }
        true
    }
}

/// Writes time_t to file "fd" in 8 bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn put_time(fd: *mut FILE, time_: time_t) -> c_int {
    unsafe {
        let mut buf: [u8; 8] = [0; 8];
        time_to_bytes(time_, buf.as_mut_ptr());
        if fwrite(buf.as_ptr().cast(), size_of::<u8>(), buf.len(), fd) == 1 {
            OK
        } else {
            FAIL
        }
    }
}

unsafe fn rename_with_tmp(from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        if c_strlen(from) >= MAXPATHL - 5 {
            return -1;
        }

        let mut tempname: [c_char; MAXPATHL + 1] = [0; MAXPATHL + 1];
        xstrlcpy(tempname.as_mut_ptr(), from, tempname.len());
        for n in 123..99999 {
            let tail = path_tail(tempname.as_ptr());
            let rem = (MAXPATHL + 1) - tail.offset_from(tempname.as_ptr()) as usize;
            vim_snprintf!(tail, rem, c"%d".as_ptr(), n);

            if !os_path_exists(tempname.as_ptr()) {
                if os_rename(from, tempname.as_ptr()) == OK {
                    if os_rename(tempname.as_ptr(), to) == OK {
                        return 0;
                    }
                    if os_rename(tempname.as_ptr(), from) != OK {
                        logmsg!(
                            LOG_ERR,
                            ptr::null(),
                            c"rename_with_tmp".as_ptr(),
                            0,
                            true,
                            c"failed to restore %s, file is at: %s".as_ptr(),
                            from,
                            tempname.as_ptr(),
                        );
                    }
                    return -1;
                }
                return -1;
            }
        }
        -1
    }
}

/// Rename a file, falling back to copying across filesystems if needed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_rename(from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        let mut use_tmp_file = false;

        if path_equal(from, to, kPathCmpLiteral) {
            if p_fic != 0 && CStr::from_ptr(path_tail(from)) != CStr::from_ptr(path_tail(to)) {
                use_tmp_file = true;
            } else {
                return 0;
            }
        }

        let mut from_info: FileInfo = std::mem::zeroed();
        if !os_fileinfo(from, &raw mut from_info) {
            return -1;
        }

        let mut to_info: FileInfo = std::mem::zeroed();
        if os_fileinfo(to, &raw mut to_info)
            && os_fileinfo_id_equal(&raw const from_info, &raw const to_info)
        {
            use_tmp_file = true;
        }

        if use_tmp_file {
            return rename_with_tmp(from, to);
        }

        if os_rename(from, to) == OK {
            return 0;
        }

        if os_remove(to) == 0 && os_rename(from, to) == OK {
            return 0;
        }

        let ret = vim_copyfile(from, to);
        if ret != OK {
            return -1;
        }

        if os_fileinfo(from, &raw mut from_info) {
            os_remove(from);
        }

        0
    }
}

/// Create the new file with same permissions as the original.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_copyfile(from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        #[cfg(not(windows))]
        {
            let mut from_info: FileInfo = std::mem::zeroed();
            if os_fileinfo_link(from, &raw mut from_info) && s_islnk(from_info.stat.st_mode) {
                let mut ret = -1;
                let mut linkbuf: [c_char; MAXPATHL + 1] = [0; MAXPATHL + 1];
                let len = readlink(from, linkbuf.as_mut_ptr(), MAXPATHL);
                if len > 0 {
                    linkbuf[len as usize] = NUL;
                    ret = symlink(linkbuf.as_ptr(), to);
                }
                return if ret == 0 { OK } else { FAIL };
            }
        }

        let acl: vim_acl_T = os_get_acl(from);

        if os_copy(from, to, UV_FS_COPYFILE_EXCL) != 0 {
            os_free_acl(acl);
            return FAIL;
        }

        os_set_acl(to, acl);
        os_free_acl(acl);
        OK
    }
}

/// Check if any not hidden buffer has been changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_timestamps(focus: c_int) -> c_int {
    unsafe {
        if no_check_timestamps > 0 {
            return 0;
        }

        if focus != 0 && did_check_timestamps {
            need_check_timestamps = true;
            return 0;
        }

        let mut didit: c_int = 0;

        if !stuff_empty()
            || global_busy != 0
            || typebuf_typed() == 0
            || autocmd_busy
            || (*curbuf).b_ro_locked > 0
            || allbuf_lock > 0
        {
            need_check_timestamps = true;
        } else {
            no_wait_return += 1;
            did_check_timestamps = true;
            ALREADY_WARNED = false;
            let mut buf = firstbuf;
            while !buf.is_null() {
                if (*buf).b_nwindows > 0 {
                    let mut bufref: bufref_T = std::mem::zeroed();
                    set_bufref(&raw mut bufref, buf);
                    let n = buf_check_timestamp(buf);
                    didit = didit.max(n);
                    if n > 0 && !bufref_valid(&raw mut bufref) {
                        buf = firstbuf;
                        continue;
                    }
                }
                buf = (*buf).b_next;
            }
            no_wait_return -= 1;
            need_check_timestamps = false;
            if need_wait_return && didit == 2 {
                msg_puts(c"\n".as_ptr());
                ui_flush();
            }
        }
        didit
    }
}

/// Move all the lines from buffer "frombuf" to buffer "tobuf".
unsafe fn move_lines(frombuf: *mut buf_T, tobuf: *mut buf_T) -> c_int {
    unsafe {
        let tbuf = curbuf;
        let mut retval = OK;

        curbuf = tobuf;
        for lnum in 1..=(*frombuf).b_ml.ml_line_count {
            let p = xmemdupz(
                ml_get_buf(frombuf, lnum).cast(),
                ml_get_buf_len(frombuf, lnum) as usize,
            )
            .cast::<c_char>();
            if ml_append(lnum - 1, p, 0, false) == FAIL {
                xfree(p.cast());
                retval = FAIL;
                break;
            }
            xfree(p.cast());
        }

        if retval != FAIL {
            curbuf = frombuf;
            let mut lnum = (*curbuf).b_ml.ml_line_count;
            while lnum > 0 {
                if ml_delete(lnum) == FAIL {
                    retval = FAIL;
                    break;
                }
                lnum -= 1;
            }
        }

        curbuf = tbuf;
        retval
    }
}

/// Check if buffer "buf" has been changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_check_timestamp(buf: *mut buf_T) -> c_int {
    unsafe {
        static mut BUSY: bool = false;

        const RELOAD_NONE: c_int = 0;
        const RELOAD_NORMAL: c_int = 1;
        const RELOAD_DETECT: c_int = 2;

        let mut retval: c_int = 0;
        let mut mesg: *mut c_char = ptr::null_mut();
        let mut mesg2: *mut c_char = c"".as_ptr().cast_mut();
        let mut helpmesg = false;
        let mut reload: c_int = RELOAD_NONE;
        let mut can_reload = false;
        let orig_size = (*buf).b_orig_size;
        let orig_mode = (*buf).b_orig_mode;

        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);

        if !(*buf).terminal.is_null()
            || (*buf).b_ffname.is_null()
            || (*buf).b_ml.ml_mfp.is_null()
            || !bt_normal(buf)
            || (*buf).b_saving
            || BUSY
        {
            return 0;
        }

        let mut file_info: FileInfo = std::mem::zeroed();
        let file_info_ok: bool;
        if ((*buf).b_flags & BF_NOTEDITED) == 0
            && (*buf).b_mtime != 0
            && ({
                file_info_ok = os_fileinfo((*buf).b_ffname, &raw mut file_info);
                !file_info_ok
                    || time_differs(&raw const file_info, (*buf).b_mtime, (*buf).b_mtime_ns)
                    || (file_info.stat.st_mode as c_int) != (*buf).b_orig_mode
            })
        {
            let prev_b_mtime = (*buf).b_mtime;

            retval = 1;

            if !file_info_ok {
                (*buf).b_mtime = -1;
                (*buf).b_orig_size = 0;
                (*buf).b_orig_mode = 0;
            } else {
                buf_store_file_info(buf, &raw mut file_info);
            }

            if os_isdir((*buf).b_fname) {
                // Don't do anything for a directory.
            } else if (if (*buf).b_p_ar >= 0 {
                (*buf).b_p_ar
            } else {
                p_ar
            }) != 0
                && !bufIsChanged(buf)
                && file_info_ok
            {
                reload = RELOAD_NORMAL;
            } else {
                let (reason, reasonlen) = if !file_info_ok {
                    (c"deleted".as_ptr(), 7isize)
                } else if bufIsChanged(buf) {
                    (c"conflict".as_ptr(), 8isize)
                } else if orig_size != (*buf).b_orig_size || buf_contents_changed(buf) {
                    (c"changed".as_ptr(), 7isize)
                } else if orig_mode != (*buf).b_orig_mode {
                    (c"mode".as_ptr(), 4isize)
                } else {
                    (c"time".as_ptr(), 4isize)
                };

                BUSY = true;
                set_vim_var_string(VV_FCS_REASON, reason, reasonlen);
                set_vim_var_string(VV_FCS_CHOICE, c"".as_ptr(), 0);
                allbuf_lock += 1;
                let mut n = apply_autocmds(
                    EVENT_FILECHANGEDSHELL,
                    (*buf).b_fname,
                    (*buf).b_fname,
                    false,
                    buf,
                );
                allbuf_lock -= 1;
                BUSY = false;
                if n {
                    if !bufref_valid(&raw mut bufref) {
                        emsg(tr(
                            c"E246: FileChangedShell autocommand deleted buffer".as_ptr()
                        ));
                    }
                    let s = CStr::from_ptr(get_vim_var_str(VV_FCS_CHOICE));
                    if s == c"reload" && *reason != b'd' as c_char {
                        reload = RELOAD_NORMAL;
                    } else if s == c"edit" {
                        reload = RELOAD_DETECT;
                    } else if s == c"ask" {
                        n = false;
                    } else {
                        return 2;
                    }
                }
                if !n {
                    if *reason == b'd' as c_char {
                        if prev_b_mtime != -1 {
                            mesg = tr(c"E211: File \"%s\" no longer available".as_ptr());
                        }
                    } else {
                        helpmesg = true;
                        can_reload = true;

                        if *reason.add(2) == b'n' as c_char {
                            mesg = tr(
                                c"W12: Warning: File \"%s\" has changed and the buffer was changed in Vim as well"
                                    .as_ptr(),
                            );
                            mesg2 = tr(c"See \":help W12\" for more info.".as_ptr());
                        } else if *reason.add(1) == b'h' as c_char {
                            mesg = tr(
                                c"W11: Warning: File \"%s\" has changed since editing started"
                                    .as_ptr(),
                            );
                            mesg2 = tr(c"See \":help W11\" for more info.".as_ptr());
                        } else if *reason == b'm' as c_char {
                            mesg = tr(
                                c"W16: Warning: Mode of file \"%s\" has changed since editing started"
                                    .as_ptr(),
                            );
                            mesg2 = tr(c"See \":help W16\" for more info.".as_ptr());
                        } else {
                            (*buf).b_mtime_read = (*buf).b_mtime;
                            (*buf).b_mtime_read_ns = (*buf).b_mtime_ns;
                        }
                    }
                }
            }
        } else if ((*buf).b_flags & BF_NEW) != 0
            && ((*buf).b_flags & BF_NEW_W) == 0
            && os_path_exists((*buf).b_ffname)
        {
            retval = 1;
            mesg = tr(c"W13: Warning: File \"%s\" has been created after editing started".as_ptr());
            (*buf).b_flags |= BF_NEW_W;
            can_reload = true;
        }

        if !mesg.is_null() {
            let path = home_replace_save(buf, (*buf).b_fname);
            if !helpmesg {
                mesg2 = c"".as_ptr().cast_mut();
            }
            let tbufsize = c_strlen(path) + c_strlen(mesg) + c_strlen(mesg2) + 3;
            let tbuf = xmalloc(tbufsize).cast::<c_char>();
            let tbuflen = vim_snprintf!(tbuf, tbufsize, mesg, path);
            set_vim_var_string(VV_WARNINGMSG, tbuf, tbuflen as isize);
            if can_reload {
                if *mesg2 != NUL {
                    vim_snprintf!(
                        tbuf.offset(tbuflen as isize),
                        tbufsize - (tbuflen as usize),
                        c"\n%s".as_ptr(),
                        mesg2,
                    );
                }
                match do_dialog(
                    VIM_WARNING,
                    tr(c"Warning".as_ptr()),
                    tbuf,
                    tr(c"&OK\n&Load File\nLoad File &and Options".as_ptr()),
                    1,
                    ptr::null_mut(),
                    1,
                ) {
                    2 => reload = RELOAD_NORMAL,
                    3 => reload = RELOAD_DETECT,
                    _ => {}
                }
            } else if State > MODE_NORMAL_BUSY || (State & MODE_CMDLINE) != 0 || ALREADY_WARNED {
                if *mesg2 != NUL {
                    vim_snprintf!(
                        tbuf.offset(tbuflen as isize),
                        tbufsize - (tbuflen as usize),
                        c"; %s".as_ptr(),
                        mesg2,
                    );
                }
                emsg(tbuf);
                retval = 2;
            } else {
                if !autocmd_busy {
                    msg_start();
                    msg_puts_hl(tbuf, HLF_E, true);
                    if *mesg2 != NUL {
                        msg_puts_hl(mesg2, HLF_W, true);
                    }
                    msg_clr_eos();
                    msg_end();
                    if emsg_silent == 0 && !in_assert_fails && !ui_has(kUIMessages) {
                        msg_delay(1004, true);
                        redraw_cmdline = false;
                    }
                }
                ALREADY_WARNED = true;
            }

            xfree(tbuf.cast());
            xfree(path.cast());
        }

        if reload != RELOAD_NONE {
            buf_reload(buf, orig_mode, reload == RELOAD_DETECT);
            if bufref_valid(&raw mut bufref) && (*buf).b_p_udf != 0 && !(*buf).b_ffname.is_null() {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                u_compute_hash(buf, hash.as_mut_ptr());
                u_write_undo(ptr::null(), false, buf, hash.as_mut_ptr());
            }
        }

        if bufref_valid(&raw mut bufref) && retval != 0 {
            apply_autocmds(
                EVENT_FILECHANGEDSHELLPOST,
                (*buf).b_fname,
                (*buf).b_fname,
                false,
                buf,
            );
        }
        retval
    }
}

/// Reload a buffer that is already loaded.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_reload(buf: *mut buf_T, orig_mode: c_int, reload_options: bool) {
    unsafe {
        let mut ea: exarg_T = std::mem::zeroed();
        let old_ro = (*buf).b_p_ro;
        let savebuf: *mut buf_T;
        let mut bufref: bufref_T = std::mem::zeroed();
        let mut saved = OK;
        let mut aco: CtxSwitch = std::mem::zeroed();
        let mut flags = READ_NEW;

        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);

        mc_buf_clear(buf);

        if !reload_options {
            prep_exarg(&raw mut ea, buf);
        }

        let old_cursor = (*curwin).w_cursor;
        let old_topline = (*curwin).w_topline;

        if p_ur < 0 || i64::from((*curbuf).b_ml.ml_line_count) <= p_ur {
            u_sync(false);
            saved = u_savecommon(curbuf, 0, (*curbuf).b_ml.ml_line_count + 1, 0, true);
            flags |= READ_KEEP_UNDO;
        }

        if buf_is_empty(curbuf) || saved == FAIL {
            savebuf = ptr::null_mut();
        } else {
            savebuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
            set_bufref(&raw mut bufref, savebuf);
            if !savebuf.is_null() && buf == curbuf {
                curbuf = savebuf;
                (*curwin).w_buffer = savebuf;
                saved = ml_open(curbuf);
                curbuf = buf;
                (*curwin).w_buffer = buf;
            }
            if savebuf.is_null()
                || saved == FAIL
                || buf != curbuf
                || move_lines(buf, savebuf) == FAIL
            {
                semsg!(
                    tr(c"E462: Could not prepare for reloading \"%s\"".as_ptr()),
                    (*buf).b_fname,
                );
                saved = FAIL;
            }
        }

        if saved == OK {
            (*curbuf).b_flags |= BF_CHECK_RO;
            (*curbuf).b_keep_filetype = true;
            if readfile(
                (*buf).b_ffname,
                (*buf).b_fname,
                0,
                0,
                MAXLNUM,
                &raw mut ea,
                flags,
                shortmess(kShmFileinfo),
            ) != OK
            {
                if !aborting() {
                    semsg!(
                        tr(c"E321: Could not reload \"%s\"".as_ptr()),
                        (*buf).b_fname,
                    );
                }
                if !savebuf.is_null() && bufref_valid(&raw mut bufref) && buf == curbuf {
                    while !buf_is_empty(curbuf) {
                        if ml_delete((*buf).b_ml.ml_line_count) == FAIL {
                            break;
                        }
                    }
                    move_lines(savebuf, buf);
                }
            } else if buf == curbuf {
                unchanged(buf, true, true);
                if (flags & READ_KEEP_UNDO) == 0 {
                    u_clearallandblockfree(buf);
                } else {
                    u_unchanged(curbuf);
                }
                buf_updates_unload(curbuf, true);
                (*curbuf).b_mod_set = true;
            }
        }
        xfree(ea.cmd.cast());

        if !savebuf.is_null() && bufref_valid(&raw mut bufref) {
            wipe_buffer(savebuf, false);
        }

        diff_invalidate(curbuf);

        (*curwin).w_topline = old_topline.min((*curbuf).b_ml.ml_line_count);
        (*curwin).w_cursor = old_cursor;
        check_cursor(curwin);
        update_topline(curwin);
        (*curbuf).b_keep_filetype = false;

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (*wp).w_buffer == (*curwin).w_buffer && !foldmethodIsManual(wp) {
                    foldUpdateAll(wp);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        if orig_mode == (*curbuf).b_orig_mode {
            (*curbuf).b_p_ro |= old_ro;
        }

        do_modelines(0);

        ctx_restore(&raw mut aco);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_store_file_info(buf: *mut buf_T, file_info: *mut FileInfo) {
    unsafe {
        (*buf).b_mtime = i64::from((*file_info).stat.st_mtim.tv_sec);
        (*buf).b_mtime_ns = i64::from((*file_info).stat.st_mtim.tv_nsec);
        (*buf).b_orig_size = os_fileinfo_size(file_info);
        (*buf).b_orig_mode = (*file_info).stat.st_mode as c_int;
    }
}

/// Adjust the line with missing eol, used for the next write.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn write_lnum_adjust(offset: linenr_T) {
    unsafe {
        if (*curbuf).b_no_eol_lnum != 0 {
            (*curbuf).b_no_eol_lnum += offset;
        }
    }
}

/// Creates a directory for private use by this instance of Nvim.
unsafe fn vim_mktempdir() {
    unsafe {
        #[cfg(windows)]
        const TEMP_DIRS: [&CStr; 5] = [c"$TMPDIR", c"$TMP", c"$TEMP", c"$USERPROFILE", c""];
        #[cfg(not(windows))]
        const TEMP_DIRS: [&CStr; 4] = [c"$TMPDIR", c"/tmp", c".", c"~"];

        let mut tmp: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let mut path: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let mut user: [c_char; 40] = [0; 40];

        os_get_username(user.as_mut_ptr(), user.len());
        memchrsub(
            user.as_mut_ptr().cast(),
            b'/' as c_char,
            b'_' as c_char,
            user.len(),
        );
        memchrsub(
            user.as_mut_ptr().cast(),
            b'\\' as c_char,
            b'_' as c_char,
            user.len(),
        );

        let umask_save = call_umask(0o077);
        for temp_dir in TEMP_DIRS {
            let mut tmplen = expand_env(
                temp_dir.as_ptr().cast_mut(),
                tmp.as_mut_ptr(),
                (TEMP_FILE_PATH_MAXLEN - 64) as c_int,
            );
            if !os_isdir(tmp.as_ptr()) {
                if strequal(c"$TMPDIR".as_ptr(), temp_dir.as_ptr()) {
                    if !os_env_exists(c"TMPDIR".as_ptr(), true) {
                        logmsg!(
                            LOG_DEBUG,
                            ptr::null(),
                            c"vim_mktempdir".as_ptr(),
                            0,
                            true,
                            c"$TMPDIR is unset".as_ptr(),
                        );
                    } else {
                        logmsg!(
                            LOG_WARN,
                            ptr::null(),
                            c"vim_mktempdir".as_ptr(),
                            0,
                            true,
                            c"$TMPDIR tempdir not a directory (or does not exist): \"%s\"".as_ptr(),
                            tmp.as_ptr(),
                        );
                    }
                }
                continue;
            }

            if after_pathsep(tmp.as_ptr(), tmp.as_ptr().add(tmplen)) == 0 {
                tmplen += vim_snprintf!(
                    tmp.as_mut_ptr().add(tmplen),
                    tmp.len() - tmplen,
                    c"%s".as_ptr(),
                    PATHSEPSTR.as_ptr(),
                ) as usize;
                debug_assert!(tmplen < tmp.len());
            }
            tmplen += vim_snprintf!(
                tmp.as_mut_ptr().add(tmplen),
                tmp.len() - tmplen,
                c"nvim.%s".as_ptr(),
                user.as_ptr(),
            ) as usize;
            debug_assert!(tmplen < tmp.len());
            os_mkdir(tmp.as_ptr(), 0o700);
            let owned = os_file_owned(tmp.as_ptr());
            let isdir = os_isdir(tmp.as_ptr());
            #[cfg(unix)]
            let perm = os_getperm(tmp.as_ptr());
            #[cfg(unix)]
            let valid = isdir && owned && 0o700 == (perm & 0o777);
            #[cfg(not(unix))]
            let valid = isdir && owned;
            if valid {
                if after_pathsep(tmp.as_ptr(), tmp.as_ptr().add(tmplen)) == 0 {
                    tmplen += vim_snprintf!(
                        tmp.as_mut_ptr().add(tmplen),
                        tmp.len() - tmplen,
                        c"%s".as_ptr(),
                        PATHSEPSTR.as_ptr(),
                    ) as usize;
                    debug_assert!(tmplen < tmp.len());
                }
            } else {
                if !owned {
                    logmsg!(
                        LOG_ERR,
                        ptr::null(),
                        c"vim_mktempdir".as_ptr(),
                        0,
                        true,
                        c"tempdir root not owned by current user (%s): %s".as_ptr(),
                        user.as_ptr(),
                        tmp.as_ptr(),
                    );
                } else if !isdir {
                    logmsg!(
                        LOG_ERR,
                        ptr::null(),
                        c"vim_mktempdir".as_ptr(),
                        0,
                        true,
                        c"tempdir root not a directory: %s".as_ptr(),
                        tmp.as_ptr(),
                    );
                }
                #[cfg(unix)]
                if 0o700 != (perm & 0o777) {
                    logmsg!(
                        LOG_ERR,
                        ptr::null(),
                        c"vim_mktempdir".as_ptr(),
                        0,
                        true,
                        c"tempdir root has invalid permissions (%o): %s".as_ptr(),
                        perm,
                        tmp.as_ptr(),
                    );
                }
                tmplen -= c_strlen(user.as_ptr());
                tmp[tmplen] = NUL;
            }

            tmplen += vim_snprintf!(
                tmp.as_mut_ptr().add(tmplen),
                tmp.len() - tmplen,
                c"XXXXXX".as_ptr(),
            ) as usize;
            debug_assert!(tmplen < tmp.len());
            let _ = tmplen;
            let r = os_mkdtemp(tmp.as_ptr(), path.as_mut_ptr());
            if r != 0 {
                logmsg!(
                    LOG_WARN,
                    ptr::null(),
                    c"vim_mktempdir".as_ptr(),
                    0,
                    true,
                    c"tempdir create failed: %s: %s".as_ptr(),
                    uv_strerror(r),
                    tmp.as_ptr(),
                );
                continue;
            }

            if vim_settempdir(path.as_mut_ptr()) {
                break;
            }
            os_rmdir(path.as_ptr());
        }
        call_umask(umask_save);
    }
}

/// Core part of "readdir()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn readdir_core(
    gap: *mut garray_T,
    path: *const c_char,
    context: *mut c_void,
    checkitem: CheckItem,
) -> c_int {
    unsafe {
        ga_init(gap, size_of::<*mut c_char>() as c_int, 20);

        let mut dir: Directory = std::mem::zeroed();
        if !os_scandir(&raw mut dir, path) {
            smsg!(0, tr(e_notopen.as_ptr()), path);
            return FAIL;
        }

        loop {
            let p = os_scandir_next(&raw mut dir);
            if p.is_null() {
                break;
            }

            let mut ignore = *p == b'.' as c_char
                && (*p.add(1) == NUL || (*p.add(1) == b'.' as c_char && *p.add(2) == NUL));
            if !ignore && let Some(cb) = checkitem {
                let r = cb(context, p);
                if r < 0 {
                    break;
                }
                if r == 0 {
                    ignore = true;
                }
            }

            if !ignore {
                ga_grow(gap, 1);
                let data = (*gap).ga_data.cast::<*mut c_char>();
                *data.offset((*gap).ga_len as isize) = xstrdup(p);
                (*gap).ga_len += 1;
            }
        }

        os_closedir(&raw mut dir);

        if (*gap).ga_len > 0 {
            sort_strings((*gap).ga_data.cast::<*mut c_char>(), (*gap).ga_len);
        }

        OK
    }
}

/// Delete "name" and everything in it, recursively.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn delete_recursive(name: *const c_char) -> c_int {
    unsafe {
        let mut result: c_int = 0;

        if os_isrealdir(name) {
            let exp = xstrdup(name);
            let mut ga: garray_T = std::mem::zeroed();
            if readdir_core(&raw mut ga, exp, ptr::null_mut(), None) == OK {
                let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();
                let len = vim_snprintf!(namebuff_ptr, MAXPATHL, c"%s/".as_ptr(), exp);

                for i in 0..ga.ga_len {
                    let entry = *ga.ga_data.cast::<*mut c_char>().offset(i as isize);
                    vim_snprintf!(
                        namebuff_ptr.offset(len as isize),
                        MAXPATHL - (len as usize),
                        c"%s".as_ptr(),
                        entry,
                    );
                    if delete_recursive(namebuff_ptr) != 0 {
                        result = -1;
                    }
                }
                ga_clear_strings(&raw mut ga);
                if os_rmdir(exp) != 0 {
                    result = -1;
                }
            } else {
                result = -1;
            }
            xfree(exp.cast());
        } else {
            result = if os_remove(name) == 0 { 0 } else { -1 };
        }

        result
    }
}

#[cfg(all(not(test), unix))]
unsafe fn vim_opentempdir() {
    unsafe {
        if !VIM_TEMPDIR_DP.is_null() {
            return;
        }

        let dp = opendir(VIM_TEMPDIR);
        if dp.is_null() {
            return;
        }

        VIM_TEMPDIR_DP = dp;
        flock(dirfd(VIM_TEMPDIR_DP), LOCK_SH);
    }
}

#[cfg(all(not(test), unix))]
unsafe fn vim_closetempdir() {
    unsafe {
        if VIM_TEMPDIR_DP.is_null() {
            return;
        }

        closedir(VIM_TEMPDIR_DP);
        VIM_TEMPDIR_DP = ptr::null_mut();
    }
}

/// Delete the temp directory and all files it contains.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_deltempdir() {
    unsafe {
        if VIM_TEMPDIR.is_null() {
            return;
        }

        #[cfg(all(not(test), unix))]
        vim_closetempdir();

        *path_tail(VIM_TEMPDIR).sub(1) = NUL;
        delete_recursive(VIM_TEMPDIR);
        xfree(VIM_TEMPDIR.cast());
        VIM_TEMPDIR = ptr::null_mut();
    }
}

/// Gets path to Nvim's own temp dir (ending with slash).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_gettempdir() -> *mut c_char {
    unsafe {
        static mut NOTFOUND: c_int = 0;
        if VIM_TEMPDIR.is_null() || !os_isdir(VIM_TEMPDIR) {
            if !VIM_TEMPDIR.is_null() {
                NOTFOUND += 1;
                if NOTFOUND == 1 {
                    logmsg!(
                        LOG_ERR,
                        ptr::null(),
                        c"vim_gettempdir".as_ptr(),
                        0,
                        true,
                        c"tempdir disappeared (antivirus or broken cleanup job?): %s".as_ptr(),
                        VIM_TEMPDIR,
                    );
                }
                if NOTFOUND > 1 {
                    msg_schedule_semsg!(
                        c"E5431: tempdir disappeared (%d times)".as_ptr(),
                        NOTFOUND
                    );
                }
                xfree(VIM_TEMPDIR.cast());
                VIM_TEMPDIR = ptr::null_mut();
            }
            vim_mktempdir();
        }
        VIM_TEMPDIR
    }
}

/// Sets Nvim's own temporary directory name to `tempdir`.
unsafe fn vim_settempdir(tempdir: *mut c_char) -> bool {
    unsafe {
        let buf = verbose_try_malloc(MAXPATHL + 2).cast::<c_char>();
        if buf.is_null() {
            return false;
        }

        vim_FullName(tempdir, buf, MAXPATHL, cfg!(windows));

        let mut buflen = c_strlen(buf);
        if after_pathsep(buf, buf.add(buflen)) == 0 {
            let sep_bytes = PATHSEPSTR.to_bytes_with_nul();
            ptr::copy_nonoverlapping(
                sep_bytes.as_ptr().cast::<c_char>(),
                buf.add(buflen),
                sep_bytes.len(),
            );
            buflen += sep_bytes.len() - 1;
        }
        VIM_TEMPDIR = xmemdupz(buf.cast(), buflen).cast::<c_char>();
        #[cfg(all(not(test), unix))]
        vim_opentempdir();
        xfree(buf.cast());
        true
    }
}

/// Return a unique name that can be used for a temp file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_tempname() -> *mut c_char {
    unsafe {
        static mut TEMP_COUNT: u64 = 0;

        let tempdir = vim_gettempdir();
        if tempdir.is_null() {
            return ptr::null_mut();
        }

        let mut templ: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let count = TEMP_COUNT;
        TEMP_COUNT += 1;
        let itmplen = vim_snprintf!(
            templ.as_mut_ptr(),
            TEMP_FILE_PATH_MAXLEN,
            c"%s%llu".as_ptr(),
            tempdir,
            count,
        );
        xmemdupz(templ.as_ptr().cast(), itmplen as usize).cast::<c_char>()
    }
}

/// Tries matching a filename with a "pattern" ("prog" is NULL), or use the
/// precompiled regprog "prog" ("pattern" is NULL).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn match_file_pat(
    pattern: *mut c_char,
    prog: *mut *mut regprog_T,
    fname: *mut c_char,
    sfname: *mut c_char,
    tail: *mut c_char,
    allow_dirs: c_int,
) -> bool {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut result = false;

        regmatch.rm_ic = p_fic != 0;
        regmatch.regprog = if !prog.is_null() {
            *prog
        } else {
            vim_regcomp(pattern, RE_MAGIC)
        };

        if !regmatch.regprog.is_null()
            && ((allow_dirs != 0
                && (vim_regexec(&raw mut regmatch, fname, 0)
                    || (!sfname.is_null() && vim_regexec(&raw mut regmatch, sfname, 0))))
                || (allow_dirs == 0 && vim_regexec(&raw mut regmatch, tail, 0)))
        {
            result = true;
        }

        if !prog.is_null() {
            *prog = regmatch.regprog;
        } else {
            vim_regfree(regmatch.regprog);
        }
        result
    }
}

/// Check if a file matches with a pattern in "list".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn match_file_list(
    list: *mut c_char,
    sfname: *mut c_char,
    ffname: *mut c_char,
) -> bool {
    unsafe {
        let tail = path_tail(sfname);

        let mut p = list;
        while *p != NUL {
            let mut buf: [c_char; MAXPATHL] = [0; MAXPATHL];
            copy_option_part(&raw mut p, buf.as_mut_ptr(), buf.len(), c",".as_ptr());
            let mut allow_dirs: c_char = 0;
            let regpat = file_pat_to_reg_pat(buf.as_ptr(), ptr::null(), &raw mut allow_dirs, 0);
            if regpat.is_null() {
                break;
            }
            let matched = match_file_pat(
                regpat,
                ptr::null_mut(),
                ffname,
                sfname,
                tail,
                c_int::from(allow_dirs),
            );
            xfree(regpat.cast());
            if matched {
                return true;
            }
        }
        false
    }
}

/// Convert the given pattern "pat" which has shell style wildcards in it, into
/// a regular expression, and return the result in allocated memory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn file_pat_to_reg_pat(
    mut pat: *const c_char,
    mut pat_end: *const c_char,
    allow_dirs: *mut c_char,
    no_bslash: c_int,
) -> *mut c_char {
    unsafe {
        if !allow_dirs.is_null() {
            *allow_dirs = 0;
        }

        if pat_end.is_null() {
            pat_end = pat.add(c_strlen(pat));
        }

        if pat_end == pat {
            return xstrdup(c"^$".as_ptr());
        }

        let mut size: usize = 2;

        let mut p = pat;
        while p < pat_end {
            match *p as u8 {
                b'*' | b'.' | b',' | b'{' | b'}' | b'~' => size += 2,
                #[cfg(windows)]
                b'\\' | b'/' => size += 4,
                _ => size += 1,
            }
            p = p.add(1);
        }
        let mut reg_pat = xmalloc(size + 1).cast::<c_char>();

        let mut i: usize = 0;

        if *pat == b'*' as c_char {
            while *pat == b'*' as c_char && pat < pat_end.sub(1) {
                pat = pat.add(1);
            }
        } else {
            *reg_pat.add(i) = b'^' as c_char;
            i += 1;
        }
        let mut endp = pat_end.sub(1);
        let mut add_dollar = true;
        if endp >= pat && *endp == b'*' as c_char {
            while endp.offset_from(pat) > 0 && *endp == b'*' as c_char {
                endp = endp.sub(1);
            }
            add_dollar = false;
        }
        let mut nested: c_int = 0;
        p = pat;
        while *p != NUL && nested >= 0 && p <= endp {
            match *p as u8 {
                b'*' => {
                    *reg_pat.add(i) = b'.' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b'*' as c_char;
                    i += 1;
                    while *p.add(1) == b'*' as c_char {
                        p = p.add(1);
                    }
                }
                b'.' | b'~' => {
                    *reg_pat.add(i) = b'\\' as c_char;
                    i += 1;
                    *reg_pat.add(i) = *p;
                    i += 1;
                }
                b'?' => {
                    *reg_pat.add(i) = b'.' as c_char;
                    i += 1;
                }
                b'\\' => {
                    if *p.add(1) == NUL {
                        break;
                    }
                    #[cfg(windows)]
                    if no_bslash == 0 {
                        let next_ch = *p.add(1) as u8;
                        if (vim_isfilec(c_int::from(next_ch)) || next_ch == b'*' || next_ch == b'?')
                            && next_ch != b'+'
                        {
                            *reg_pat.add(i) = b'[' as c_char;
                            i += 1;
                            *reg_pat.add(i) = b'\\' as c_char;
                            i += 1;
                            *reg_pat.add(i) = b'/' as c_char;
                            i += 1;
                            *reg_pat.add(i) = b']' as c_char;
                            i += 1;
                            if !allow_dirs.is_null() {
                                *allow_dirs = 1;
                            }
                            p = p.add(1);
                            continue;
                        }
                    }
                    p = p.add(1);
                    let ch = *p;
                    if ch == b'?' as c_char && (!BACKSLASH_IN_FILENAME_BOOL || no_bslash != 0) {
                        *reg_pat.add(i) = b'?' as c_char;
                        i += 1;
                    } else if ch == b',' as c_char
                        || ch == b'%' as c_char
                        || ch == b'#' as c_char
                        || ascii_isspace(ch)
                        || ch == b'{' as c_char
                        || ch == b'}' as c_char
                    {
                        *reg_pat.add(i) = ch;
                        i += 1;
                    } else if ch == b'\\' as c_char
                        && *p.add(1) == b'\\' as c_char
                        && *p.add(2) == b'{' as c_char
                    {
                        *reg_pat.add(i) = b'\\' as c_char;
                        i += 1;
                        *reg_pat.add(i) = b'{' as c_char;
                        i += 1;
                        p = p.add(2);
                    } else {
                        if !allow_dirs.is_null()
                            && vim_ispathsep(c_int::from(ch))
                            && (!BACKSLASH_IN_FILENAME_BOOL
                                || no_bslash != 0
                                || ch != b'\\' as c_char)
                        {
                            *allow_dirs = 1;
                        }
                        *reg_pat.add(i) = b'\\' as c_char;
                        i += 1;
                        *reg_pat.add(i) = ch;
                        i += 1;
                    }
                }
                #[cfg(windows)]
                b'/' => {
                    *reg_pat.add(i) = b'[' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b'\\' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b'/' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b']' as c_char;
                    i += 1;
                    if !allow_dirs.is_null() {
                        *allow_dirs = 1;
                    }
                }
                b'{' => {
                    *reg_pat.add(i) = b'\\' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b'(' as c_char;
                    i += 1;
                    nested += 1;
                }
                b'}' => {
                    *reg_pat.add(i) = b'\\' as c_char;
                    i += 1;
                    *reg_pat.add(i) = b')' as c_char;
                    i += 1;
                    nested -= 1;
                }
                b',' => {
                    if nested != 0 {
                        *reg_pat.add(i) = b'\\' as c_char;
                        i += 1;
                        *reg_pat.add(i) = b'|' as c_char;
                        i += 1;
                    } else {
                        *reg_pat.add(i) = b',' as c_char;
                        i += 1;
                    }
                }
                _ => {
                    if !allow_dirs.is_null() && vim_ispathsep(c_int::from(*p)) {
                        *allow_dirs = 1;
                    }
                    *reg_pat.add(i) = *p;
                    i += 1;
                }
            }
            p = p.add(1);
        }
        if add_dollar {
            *reg_pat.add(i) = b'$' as c_char;
            i += 1;
        }
        *reg_pat.add(i) = NUL;
        if nested != 0 {
            #[cfg(not(test))]
            if nested < 0 {
                emsg(tr(c"E219: Missing {.".as_ptr()));
            } else {
                emsg(tr(c"E220: Missing }.".as_ptr()));
            }
            xfree(reg_pat.cast());
            reg_pat = ptr::null_mut();
        }
        reg_pat
    }
}

/// Version of read() that retries when interrupted by EINTR.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn read_eintr(fd: c_int, buf: *mut c_void, bufsize: usize) -> isize {
    #[cfg(not(test))]
    unsafe {
        loop {
            #[cfg(windows)]
            let ret = _read(fd, buf, bufsize as c_uint) as isize;
            #[cfg(not(windows))]
            let ret = read(fd, buf, bufsize);
            if ret >= 0 || get_errno() != EINTR {
                return ret;
            }
        }
    }
    #[cfg(test)]
    {
        let _ = (fd, buf, bufsize);
        -1
    }
}

/// Version of write() that retries when interrupted by EINTR.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn write_eintr(fd: c_int, buf: *const c_void, bufsize: usize) -> isize {
    #[cfg(not(test))]
    unsafe {
        let mut ret: isize = 0;

        while (ret as usize) < bufsize {
            let rem = bufsize - (ret as usize);
            #[cfg(windows)]
            let wlen = _write(fd, buf.cast::<u8>().offset(ret).cast(), rem as c_uint) as isize;
            #[cfg(not(windows))]
            let wlen = write(fd, buf.cast::<u8>().offset(ret).cast(), rem);
            if wlen < 0 {
                if get_errno() != EINTR {
                    break;
                }
            } else {
                ret += wlen;
            }
        }
        ret
    }
    #[cfg(test)]
    {
        let _ = (fd, buf, bufsize);
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dev_fd_file() {
        unsafe {
            assert!(!is_dev_fd_file(c"/dev/fd/0".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/1".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/2".as_ptr().cast_mut()));
            assert!(is_dev_fd_file(c"/dev/fd/3".as_ptr().cast_mut()));
            assert!(is_dev_fd_file(c"/dev/fd/10".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/10a".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/tmp/foo".as_ptr().cast_mut()));
        }
    }

    #[test]
    fn test_check_for_bom() {
        unsafe {
            let mut len = 0;
            let utf8_bom = [0xefu8, 0xbb, 0xbf, b'a'];
            let enc = check_for_bom(utf8_bom.as_ptr().cast(), 4, &raw mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"utf-8");
            assert_eq!(len, 3);

            let utf16le_bom = [0xffu8, 0xfe, b'a', 0];
            let enc = check_for_bom(utf16le_bom.as_ptr().cast(), 4, &raw mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"utf-16le");
            assert_eq!(len, 2);

            let ucs4le_bom = [0xffu8, 0xfe, 0x00, 0x00];
            let enc = check_for_bom(ucs4le_bom.as_ptr().cast(), 4, &raw mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"ucs-4le");
            assert_eq!(len, 4);
        }
    }

    #[test]
    fn test_file_pat_to_reg_pat() {
        unsafe {
            let check = |pat: &CStr, expected: &CStr| {
                let res = file_pat_to_reg_pat(pat.as_ptr(), ptr::null(), ptr::null_mut(), 0);
                assert!(!res.is_null());
                assert_eq!(CStr::from_ptr(res), expected);
                xfree(res.cast());
            };
            check(c"path", c"^path$");
            check(c"*path", c"path$");
            check(c"path*", c"^path");
            check(c"*path*", c"path");
            check(c"foo?bar", c"^foo.bar$");
            check(c"foo*bar", c"^foo.*bar$");
            check(c"foo.bar", c"^foo\\.bar$");
            check(c"foo{bar,baz}", c"^foo\\(bar\\|baz\\)$");
        }
    }
}
