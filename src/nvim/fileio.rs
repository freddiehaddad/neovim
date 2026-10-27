//! File reading, encoding conversion, timestamp checking, and temp files (`src/nvim/fileio.c`).

use crate::api::private::helpers::{HLF_E, cstr_as_obj};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_BUFNEW, EVENT_BUFNEWFILE, EVENT_BUFREADCMD,
    EVENT_BUFREADPOST, EVENT_BUFREADPRE, EVENT_BUFWIPEOUT, EVENT_FILECHANGEDSHELL,
    EVENT_FILECHANGEDSHELLPOST, EVENT_FILEREADCMD, EVENT_FILEREADPOST, EVENT_FILEREADPRE,
    EVENT_FILETYPE, EVENT_FILTERREADPOST, EVENT_FILTERREADPRE, EVENT_STDINREADPRE, apply_autocmds,
    apply_autocmds_exarg, apply_autocmds_group, augroup_exists, augroup_find, do_doautocmd,
};
use crate::buffer::{
    BF_CHECK_RO, BF_NEW, BF_NOTEDITED, BLN_DUMMY, MAXLNUM, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC,
    NOTDONE, SEA_QUIT, bt_dontwrite, bt_nofilename, bt_normal, buf_T, buf_contents_changed,
    buf_is_empty, buflist_new, bufref_T, bufref_valid, do_modelines, kShmFileinfo, kShmRo,
    kUIMessages, set_bufref, setfname, wipe_buffer,
};
use crate::buffer_updates::buf_updates_unload;
use crate::change::{HLF_W, appended_lines_mark, save_file_ff, unchanged};
#[cfg(windows)]
use crate::charset::vim_isfilec;
use crate::charset::{EOL_MAC, ascii_isspace};
#[cfg(test)]
use crate::charset::{ascii_isdigit, skipdigits};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{check_cursor, check_cursor_lnum, colnr_T, linenr_T, pos_T};
use crate::dialog::do_dialog;
use crate::diff::diff_invalidate;
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later, status_redraw_all};
use crate::errors::*;

use crate::eval::vars::{
    BAD_DROP, BAD_KEEP, FORCE_BIN, FORCE_NOBIN, VV_FCS_CHOICE, VV_FCS_REASON, VV_TESTING,
    VV_WARNINGMSG, eval_charconvert, get_vim_var_nr, get_vim_var_str, set_vim_var_string,
};
use crate::event::r#loop::uv_strerror;
#[cfg(unix)]
use crate::event::r#loop::{uv_gid_t, uv_uid_t};
use crate::ex_docmd::{CAR, CMOD_LOCKMARKS, exarg_T};
use crate::ex_eval::aborting;
use crate::fold::{foldUpdateAll, foldmethodIsManual};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::input::{stuff_empty, typebuf_typed};
use crate::insert::beginline;
use crate::log::logmsg;
use crate::mbyte::{
    enc_canon_props, enc_canonize, my_iconv_open, utf_byte2len, utf_char2bytes, utf_char2len,
    utf_head_off, utf_ptr2char, utf_ptr2len_len,
};
use crate::mcursor::mc_buf_clear;
use crate::memfile::mf_fullname;
use crate::memline::{
    check_need_swap, ml_append, ml_delete, ml_get, ml_get_buf, ml_get_len, ml_open,
};
use crate::memory::{cstr_to_cstring, xstrlcpy};
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
    Directory, FileInfo, TEMP_FILE_PATH_MAXLEN, UV_ENOENT, os_closedir, os_copy, os_dirname,
    os_errno, os_file_is_writable, os_file_owned, os_fileinfo, os_fileinfo_fd,
    os_fileinfo_id_equal, os_fileinfo_size, os_free_acl, os_get_acl, os_getperm, os_isdir,
    os_isrealdir, os_mkdir, os_mkdtemp, os_open, os_path_exists, os_remove, os_rename, os_rmdir,
    os_scandir, os_scandir_next, os_set_acl, os_set_cloexec, os_set_errno, vim_acl_T,
};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo_link, os_setperm, s_islnk};
use crate::os::input::os_breakcheck;
use crate::os::time::{os_hrtime, time_t};
use crate::os::users::os_get_username;
use crate::path::{
    add_pathsep, after_pathsep, dir_of_file_exists, kPathCmpLiteral, path_equal, path_is_absolute,
    path_shorten_fname, path_tail, path_with_url, vim_FullName, vim_ispathsep,
};
use crate::regexp::{RE_MAGIC, regmatch_T, regprog_T, vim_regcomp, vim_regexec, vim_regfree};
use crate::sha256::context_sha256_T;
use crate::shada::check_marks_read;
use crate::state::{MODE_CMDLINE, MODE_NORMAL_BUSY};
use crate::strings::{ngettext, tr, vim_snprintf, vim_strchr};
use crate::ui::{ui_flush, ui_has};
use crate::undo::{
    bufIsChanged, u_clearallandblockfree, u_clearline, u_compute_hash, u_find_first_changed,
    u_read_undo, u_savecommon, u_sync, u_unchanged, u_write_undo,
};
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::os_open_conin_fd;

pub type off_T = i64;
pub type iconv_t = isize;
pub type FILE = c_void;

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

const CTRL_Z: c_char = 0x1a;

pub(crate) const EOF: c_int = -1;

const MAXCOL: isize = 0x7fffffff;

pub(crate) const EOL_UNKNOWN: c_int = -1;
pub(crate) const EOL_UNIX: c_int = 0;
pub(crate) const EOL_DOS: c_int = 1;

const BAD_REPLACE: c_int = b'?' as c_int;

pub(crate) const kOptFileencoding: c_int = 92;

pub(crate) const ENC_UNICODE: c_int = 0x04;
pub(crate) const ENC_ENDIAN_L: c_int = 0x20;
pub(crate) const ENC_2BYTE: c_int = 0x40;
pub(crate) const ENC_4BYTE: c_int = 0x80;
pub(crate) const ENC_2WORD: c_int = 0x100;
pub(crate) const ENC_LATIN1: c_int = 0x200;

const BF_NEW_W: c_int = 0x20;

const kCpoFnamer: c_int = b'f' as c_int;
pub(crate) const kShmOver: c_int = b'o' as c_int;
const kShmLines: c_int = b'l' as c_int;

pub(crate) const O_RDONLY: c_int = 0;
pub(crate) const EINTR: c_int = 4;
pub(crate) const ICONV_EINVAL: c_int = 22;
const UV_FS_COPYFILE_EXCL: c_int = 0x0001;

pub(crate) const S_IFMT: u64 = 0o170000;
pub(crate) const S_IFIFO: u64 = 0o010000;
pub(crate) const S_IFDIR: u64 = 0o040000;
pub(crate) const S_IFREG: u64 = 0o100000;
pub(crate) const S_IFSOCK: u64 = 0o140000;

const LOG_DEBUG: c_int = 1;
const LOG_WARN: c_int = 3;
const LOG_ERR: c_int = 4;

const NONASCII_MASK: u64 = (u64::MAX / 0xFF) * 0x80;

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

use crate::os::fs::PATHSEPSTR;
const BACKSLASH_IN_FILENAME_BOOL: bool = cfg!(windows);

#[cfg(unix)]
const LOCK_SH: c_int = 1;
pub(crate) const SEEK_SET: c_int = 0;
pub(crate) const SEEK_END: c_int = 2;

static E_AUCHANGEDBUF: &CStr = c"E812: Autocommands changed buffer or buffer name";
static MSG_IS_A_DIRECTORY: &CStr = c"is a directory";

#[cfg(unix)]
type DIR = c_void;

struct FileioModuleState {
    ALREADY_WARNED: bool,
    VIM_TEMPDIR: Option<std::ffi::CString>,
    #[cfg(unix)]
    VIM_TEMPDIR_DP: *mut DIR,
    NOTFOUND: c_int,
}

impl FileioModuleState {
    const fn new() -> Self {
        Self {
            ALREADY_WARNED: false,
            VIM_TEMPDIR: None,
            #[cfg(unix)]
            VIM_TEMPDIR_DP: ptr::null_mut(),
            NOTFOUND: 0,
        }
    }
}

static FIO_STATE: GlobalState<FileioModuleState> = GlobalState::new(FileioModuleState::new());

#[inline]
fn fio_st() -> &'static mut FileioModuleState {
    unsafe { &mut *FIO_STATE.as_ptr() }
}

use crate::arglist::ML_EMPTY;
use crate::charset::{NL, NUL};
use crate::eval::funcs::VIM_WARNING;
use crate::eval::typval::kCallbackNone;
use crate::ex_cmds::{BL_FIX, BL_WHITE, kShmOverall};
use crate::ex_docmd::UNDO_HASH_SIZE;
use crate::math::{FAIL, OK};
use crate::option::OPT_LOCAL;
use crate::os::fs::MAXPATHL;

unsafe extern "C" {
    pub fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
    pub fn freopen(
        filename: *const c_char,
        mode: *const c_char,
        stream: *mut c_void,
    ) -> *mut c_void;
    pub fn fclose(stream: *mut FILE) -> c_int;
    pub fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fgets(s: *mut c_char, n: c_int, stream: *mut FILE) -> *mut c_char;
    pub fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
    pub fn fgetc(stream: *mut FILE) -> c_int;
    pub fn fputc(c: c_int, stream: *mut FILE) -> c_int;
    pub fn ungetc(c: c_int, stream: *mut FILE) -> c_int;
    pub fn fflush(stream: *mut FILE) -> c_int;
    pub fn setbuf(stream: *mut FILE, buffer: *mut c_char);
    pub fn setvbuf(stream: *mut FILE, buffer: *mut c_char, mode: c_int, size: usize) -> c_int;
    pub fn fseek(stream: *mut FILE, offset: c_long, whence: c_int) -> c_int;
    pub fn ftell(stream: *mut FILE) -> c_long;
    pub fn ferror(stream: *mut FILE) -> c_int;
    pub fn feof(stream: *mut FILE) -> c_int;
    #[cfg(unix)]
    pub fn tcdrain(fd: c_int) -> c_int;
    #[cfg(unix)]
    pub fn getppid() -> c_int;
    #[cfg(unix)]
    pub fn getpgid(pid: c_int) -> c_int;
    #[cfg(unix)]
    pub fn getpgrp() -> c_int;

    #[cfg(not(windows))]
    #[link_name = "iconv_open"]
    fn c_iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
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
    #[cfg_attr(windows, link_name = "_close")]
    pub fn close(fd: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_dup")]
    pub fn dup(oldfd: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_dup2")]
    pub fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_lseeki64")]
    #[cfg_attr(not(windows), link_name = "lseek")]
    pub fn vim_lseek(fd: c_int, offset: off_T, whence: c_int) -> off_T;
    #[cfg_attr(windows, link_name = "_fseeki64")]
    #[cfg_attr(not(windows), link_name = "fseeko")]
    pub fn vim_fseek(stream: *mut FILE, offset: off_T, whence: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_ftelli64")]
    #[cfg_attr(not(windows), link_name = "ftello")]
    pub fn vim_ftell(stream: *mut FILE) -> off_T;
    #[cfg_attr(windows, link_name = "_fdopen")]
    pub fn fdopen(fd: c_int, mode: *const c_char) -> *mut FILE;
    #[cfg_attr(windows, link_name = "_fileno")]
    pub fn fileno(stream: *mut FILE) -> c_int;

    #[cfg(windows)]
    pub fn _get_osfhandle(fd: c_int) -> isize;
    #[cfg(windows)]
    pub fn _open_osfhandle(osfhandle: isize, flags: c_int) -> c_int;
    #[cfg(windows)]
    pub fn _setmode(fd: c_int, mode: c_int) -> c_int;
    #[cfg(windows)]
    pub fn _getdrive() -> c_int;
    #[cfg(windows)]
    pub fn _read(fd: c_int, buf: *mut c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    pub fn _write(fd: c_int, buf: *const c_void, count: c_uint) -> c_int;
    #[cfg(windows)]
    fn _umask(pmode: c_int) -> c_int;

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
    pub fn kill(pid: c_int, sig: c_int) -> c_int;
    #[cfg(not(windows))]
    pub fn umask(mask: c_uint) -> c_uint;
    #[cfg(not(windows))]
    pub fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: usize) -> isize;
    #[cfg(not(windows))]
    fn symlink(target: *const c_char, linkpath: *const c_char) -> c_int;
    #[cfg(not(windows))]
    fn opendir(name: *const c_char) -> *mut DIR;
    #[cfg(not(windows))]
    fn closedir(dirp: *mut DIR) -> c_int;
    #[cfg(not(windows))]
    fn dirfd(dirp: *mut DIR) -> c_int;
    #[cfg(not(windows))]
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

#[cfg(not(windows))]
#[inline]
pub fn call_iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void {
    unsafe { c_iconv_open(tocode, fromcode) }
}

use crate::os::fs::{s_isdir, s_isfifo, s_isreg, s_issock};

#[cfg(windows)]
mod win_iconv {
    use super::*;
    use crate::os::fs::errno_ptr;

    const E2BIG: c_int = 7;
    const EINVAL: c_int = 22;
    const EILSEQ: c_int = 42;
    const MB_ERR_INVALID_CHARS: u32 = 0x08;
    const WC_ERR_INVALID_CHARS: u32 = 0x80;
    const WC_NO_BEST_FIT_CHARS: u32 = 0x0400;

    unsafe extern "system" {
        fn IsValidCodePage(CodePage: u32) -> c_int;
        fn IsDBCSLeadByteEx(CodePage: u32, TestChar: u8) -> c_int;
        fn MultiByteToWideChar(
            CodePage: u32,
            dwFlags: u32,
            lpMultiByteStr: *const c_char,
            cbMultiByte: c_int,
            lpWideCharStr: *mut u16,
            cchWideChar: c_int,
        ) -> c_int;
        fn WideCharToMultiByte(
            CodePage: u32,
            dwFlags: u32,
            lpWideCharStr: *const u16,
            cchWideChar: c_int,
            lpMultiByteStr: *mut c_char,
            cbMultiByte: c_int,
            lpDefaultChar: *const c_char,
            lpUsedDefaultChar: *mut c_int,
        ) -> c_int;
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
                        let s = unsafe { std::str::from_utf8_unchecked(&src[..e.valid_up_to()]) };
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
                    StepResult::Char(unsafe { char::from_u32_unchecked(u32::from(u1)) }, 2)
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
                let wlen = ch.encode_utf16(&mut wide).len() as c_int;
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
                    let mut used_def: c_int = 0;
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

    pub fn open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void {
        if tocode.is_null() || fromcode.is_null() {
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX as *mut c_void;
        }
        let Ok(to_str) = (unsafe { CStr::from_ptr(tocode) }).to_str() else {
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX as *mut c_void;
        };
        let Ok(from_str) = (unsafe { CStr::from_ptr(fromcode) }).to_str() else {
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX as *mut c_void;
        };
        let (Some(to), Some(from)) = (parse_win_enc(to_str), parse_win_enc(from_str)) else {
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX as *mut c_void;
        };
        let boxed = Box::new(WinIconvDesc {
            from,
            to,
            from_resolved: None,
        });
        Box::into_raw(boxed).cast()
    }

    pub fn convert(
        cd: iconv_t,
        inbuf: *mut *const c_char,
        inbytesleft: *mut usize,
        outbuf: *mut *mut c_char,
        outbytesleft: *mut usize,
    ) -> usize {
        if cd == -1 || cd == 0 {
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX;
        }
        let desc = unsafe { &mut *(cd as *mut WinIconvDesc) };
        if inbuf.is_null() || unsafe { (*inbuf).is_null() } {
            desc.from_resolved = None;
            return 0;
        }
        unsafe {
            while *inbytesleft > 0 {
                let src = std::slice::from_raw_parts((*inbuf).cast::<u8>(), *inbytesleft);
                match decode_one(desc, src) {
                    StepResult::SkipBom(consumed) => {
                        *inbuf = (*inbuf).add(consumed);
                        *inbytesleft -= consumed;
                    }
                    StepResult::Char(ch, consumed) => {
                        let mut encoded = [0u8; 8];
                        let Some(enc_len) = encode_one(desc.to, ch, &mut encoded) else {
                            *errno_ptr() = EILSEQ;
                            return usize::MAX;
                        };
                        if enc_len > *outbytesleft {
                            *errno_ptr() = E2BIG;
                            return usize::MAX;
                        }
                        ptr::copy_nonoverlapping(encoded.as_ptr().cast(), *outbuf, enc_len);
                        *outbuf = (*outbuf).add(enc_len);
                        *outbytesleft -= enc_len;
                        *inbuf = (*inbuf).add(consumed);
                        *inbytesleft -= consumed;
                    }
                    StepResult::Incomplete => {
                        *errno_ptr() = EINVAL;
                        return usize::MAX;
                    }
                    StepResult::Invalid => {
                        *errno_ptr() = EILSEQ;
                        return usize::MAX;
                    }
                }
            }
        }
        0
    }

    pub fn close(cd: iconv_t) -> c_int {
        if cd != -1 && cd != 0 {
            let _ = unsafe { Box::from_raw(cd as *mut WinIconvDesc) };
        }
        0
    }
}

#[cfg(windows)]
#[inline]
pub fn call_iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void {
    win_iconv::open(tocode, fromcode)
}

#[inline]
pub fn call_iconv<T: IntoIconvT>(
    cd: T,
    inbuf: *mut *const c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    #[cfg(windows)]
    {
        win_iconv::convert(cd.into_iconv_t(), inbuf, inbytesleft, outbuf, outbytesleft)
    }
    #[cfg(not(windows))]
    unsafe {
        iconv(cd.into_iconv_t(), inbuf, inbytesleft, outbuf, outbytesleft)
    }
}

#[inline]
pub fn call_iconv_close<T: IntoIconvT>(cd: T) -> c_int {
    #[cfg(windows)]
    {
        win_iconv::close(cd.into_iconv_t())
    }
    #[cfg(not(windows))]
    unsafe {
        iconv_close(cd.into_iconv_t())
    }
}

pub trait IntoIconvT {
    fn into_iconv_t(self) -> iconv_t;
}
impl IntoIconvT for iconv_t {
    #[inline]
    fn into_iconv_t(self) -> iconv_t {
        self
    }
}
impl IntoIconvT for *mut c_void {
    #[inline]
    fn into_iconv_t(self) -> iconv_t {
        self as iconv_t
    }
}

#[inline]
fn call_umask(mask: c_uint) -> c_uint {
    #[cfg(windows)]
    unsafe {
        _umask(mask as c_int) as c_uint
    }
    #[cfg(not(windows))]
    unsafe {
        umask(mask)
    }
}

/// Shows a message about file `name`, e.g. `"foo.txt" [New]`.
fn filemess(buf: *mut buf_T, name: *mut c_char, s: *const c_char, msg_id: *const c_char) {
    unsafe {
        let prev_msg_col = g_st().msg_col;

        if g_st().msg_silent != 0 {
            return;
        }

        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        add_quoted_fname(iobuf_ptr, IOSIZE - 100, buf, name);

        { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), s, _dsize - _dlen); };

        let msg_scroll_save = g_st().msg_scroll;
        if shortmess(kShmOverall) && g_st().msg_listdo_overwrite == 0 && !g_st().exiting && opt_vars().p_verbose == 0 {
            g_st().msg_scroll = 0;
        }
        if g_st().msg_scroll == 0 {
            msg_check_for_delay(false);
        }
        msg_start();
        if prev_msg_col != 0 && g_st().msg_col == 0 {
            msg_putchar(b'\r' as c_int);
        }
        g_st().msg_scroll = msg_scroll_save;
        g_st().msg_scrolled_ign = true;
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
        g_st().msg_scrolled_ign = false;
    }
}

/// Shows file `name` before writing it, as a "bufwrite" message with `msg_id`, which buf_write()
/// replaces when done (or with a progress-message, if slow).
pub fn filemess_bufwrite(buf: *mut buf_T, name: *mut c_char, msg_id: *const c_char) {
    filemess(buf, name, c"".as_ptr(), msg_id);
}

/// If a file read/write is slower than this, emit a progress-message.
pub fn slow_io_ns() -> u64 {
    if g_st().nvim_testing && get_vim_var_nr(VV_TESTING) != 0 {
            0
        } else {
            1_000_000_000 / 2
        }
}

/// Shows the progress-message of a slow readfile(). Locks `curbuf` so Progress handlers cannot
/// accidentally unload it while it is loading.
fn readfile_progress(
    s: *const c_char,
    msg_id: *const c_char,
    status: *const c_char,
    percent: c_int,
    hist: bool,
) -> *mut c_char {
    unsafe {
        let buf = g_st().curbuf;
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
pub fn readfile(
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
        let mut fd: c_int = if g_st().stdin_fd >= 0 { g_st().stdin_fd } else { 0 };
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
        let mut buffer_vec: Vec<c_char> = Vec::new();
        let mut buffer: *mut c_char = ptr::null_mut();
        let mut line_start: *mut c_char = ptr::null_mut();
        let mut size: isize = 0;
        let mut p: *mut u8;
        let mut filesize: off_T = 0;
        let mut skip_read = false;
        let mut sha_ctx = context_sha256_T::new();
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
        let mut file_info: FileInfo = FileInfo::new();
        let mut skip_count: linenr_T = 0;
        let mut read_count: linenr_T = 0;
        let msg_save = g_st().msg_scroll;
        let mut read_no_eol_lnum: linenr_T = 0;
        let mut file_rewind = false;
        let mut conv_error: linenr_T = 0;
        let mut illegal_byte: linenr_T = 0;
        let mut keep_dest_enc = false;
        let mut bad_char_behavior: c_int = BAD_REPLACE;
        let mut tmpname: Option<std::ffi::CString> = None;
        let mut fio_flags: c_int;
        let mut fenc: *mut c_char;
        let mut fenc_owned: Option<std::ffi::CString> = None;
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

        (*g_st().curbuf).b_au_did_filetype = false;
        (*g_st().curbuf).b_no_eol_lnum = 0;

        'theend: {
            if (*g_st().curbuf).b_ffname.is_null()
                && !filtering
                && !fname.is_null()
                && !vim_strchr(opt_vars().p_cpo, kCpoFnamer).is_null()
                && (flags & READ_DUMMY) == 0
                && set_rw_fname(fname, sfname) == FAIL
            {
                break 'theend;
            }

            let old_curbuf = g_st().curbuf;
            let old_b_ffname = (*g_st().curbuf).b_ffname;
            let old_b_fname = (*g_st().curbuf).b_fname;
            let using_b_ffname = fname == (*g_st().curbuf).b_ffname || sfname == (*g_st().curbuf).b_ffname;
            let using_b_fname = fname == (*g_st().curbuf).b_fname || sfname == (*g_st().curbuf).b_fname;

            g_st().need_fileinfo = false;

            if sfname.is_null() {
                sfname = fname;
            }
            #[cfg(unix)]
            {
                fname = sfname;
            }

            if !filtering && !read_stdin && !read_buffer {
                orig_start = (*g_st().curbuf).b_op_start;

                (*g_st().curbuf).b_op_start.lnum = if from == 0 { 1 } else { from };
                (*g_st().curbuf).b_op_start.col = 0;

                if newfile {
                    if apply_autocmds_exarg(
                        EVENT_BUFREADCMD,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
                        eap,
                    ) {
                        retval = OK;
                        if aborting() {
                            retval = FAIL;
                        }

                        if retval == OK && !(*g_st().curbuf).b_au_did_filetype && *(*g_st().curbuf).b_p_ft != NUL
                        {
                            apply_autocmds(
                                EVENT_FILETYPE,
                                (*g_st().curbuf).b_p_ft,
                                (*g_st().curbuf).b_fname,
                                true,
                                g_st().curbuf,
                            );
                            if aborting() {
                                retval = FAIL;
                            }
                        }
                        if retval == OK {
                            (*g_st().curbuf).b_flags &= !BF_NOTEDITED;
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

                (*g_st().curbuf).b_op_start = orig_start;

                if (flags & READ_NOFILE) != 0 {
                    retval = NOTDONE;
                    break 'theend;
                }
            }

            if ((shortmess(kShmOver) && g_st().msg_listdo_overwrite == 0) || (*g_st().curbuf).b_help)
                && opt_vars().p_verbose == 0
            {
                g_st().msg_scroll = 0;
            } else {
                g_st().msg_scroll = 1;
            }

            if !fname.is_null() && *fname != NUL {
                let fnamelen = std::ffi::CStr::from_ptr(fname).to_bytes().len();

                if fnamelen >= MAXPATHL {
                    filemess(
                        g_st().curbuf,
                        fname,
                        tr(c"Illegal file name".as_ptr()),
                        ptr::null(),
                    );
                    msg_end();
                    g_st().msg_scroll = msg_save;
                    break 'theend;
                }

                if after_pathsep(fname, fname.add(fnamelen)) != 0 {
                    if !silent {
                        filemess(g_st().curbuf, fname, tr(MSG_IS_A_DIRECTORY.as_ptr()), ptr::null());
                    }
                    msg_end();
                    g_st().msg_scroll = msg_save;
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
                && !s_isreg(perm as u64)
                && !s_isfifo(perm as u64)
                && !s_issock(perm as u64)
                && !is_chr_dev
            {
                if s_isdir(perm as u64) {
                    if !silent {
                        filemess(g_st().curbuf, fname, tr(MSG_IS_A_DIRECTORY.as_ptr()), ptr::null());
                    }
                    retval = NOTDONE;
                } else {
                    filemess(g_st().curbuf, fname, tr(c"is not a file".as_ptr()), ptr::null());
                }
                msg_end();
                g_st().msg_scroll = msg_save;
                break 'theend;
            }

            set_file_options(set_options, eap);

            let check_readonly = newfile && ((*g_st().curbuf).b_flags & BF_CHECK_RO) != 0;
            if check_readonly && !g_st().readonlymode {
                (*g_st().curbuf).b_p_ro = 0;
            }

            if newfile && !read_stdin && !read_buffer && !read_fifo {
                if os_fileinfo(fname, &mut file_info) {
                    buf_store_file_info(&mut *g_st().curbuf, &mut file_info);
                    (*g_st().curbuf).b_mtime_read = (*g_st().curbuf).b_mtime;
                    (*g_st().curbuf).b_mtime_read_ns = (*g_st().curbuf).b_mtime_ns;
                    #[cfg(unix)]
                    {
                        swap_mode = ((file_info.stat.st_mode as c_int) & 0o644) | 0o600;
                    }
                } else {
                    (*g_st().curbuf).b_mtime = 0;
                    (*g_st().curbuf).b_mtime_ns = 0;
                    (*g_st().curbuf).b_mtime_read = 0;
                    (*g_st().curbuf).b_mtime_read_ns = 0;
                    (*g_st().curbuf).b_orig_size = 0;
                    (*g_st().curbuf).b_orig_mode = 0;
                }

                (*g_st().curbuf).b_flags &= !(BF_NEW | BF_NEW_W);
            }

            let mut file_readonly = false;
            if !read_buffer && !read_stdin {
                if !newfile
                    || g_st().readonlymode
                    || (perm & 0o222) == 0
                    || os_file_is_writable(fname) == 0
                {
                    file_readonly = true;
                }
                fd = os_open(fname, O_RDONLY, 0);
            }

            if fd < 0 {
                g_st().msg_scroll = msg_save;
                if !newfile {
                    break 'theend;
                }
                if perm == UV_ENOENT {
                    (*g_st().curbuf).b_flags |= BF_NEW;

                    if !bt_dontwrite(g_st().curbuf) {
                        check_need_swap(newfile);
                        if g_st().curbuf != old_curbuf
                            || (using_b_ffname && old_b_ffname != (*g_st().curbuf).b_ffname)
                            || (using_b_fname && old_b_fname != (*g_st().curbuf).b_fname)
                        {
                            emsg(tr(E_AUCHANGEDBUF.as_ptr()));
                            break 'theend;
                        }
                    }
                    if !silent {
                        if dir_of_file_exists(fname) {
                            filemess(g_st().curbuf, sfname, tr(c"[New]".as_ptr()), ptr::null());
                        } else {
                            filemess(g_st().curbuf, sfname, tr(c"[New DIRECTORY]".as_ptr()), ptr::null());
                        }
                    }
                    check_marks_read();
                    if !eap.is_null() {
                        set_forced_fenc(&mut *eap);
                    }
                    apply_autocmds_exarg(EVENT_BUFNEWFILE, sfname, sfname, false, g_st().curbuf, eap);
                    save_file_ff(&mut *g_st().curbuf);

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
                    g_st().curbuf,
                    sfname,
                    if is_too_big {
                        tr(c"[File too big]".as_ptr())
                    } else {
                        tr(c"[Permission Denied]".as_ptr())
                    },
                    ptr::null(),
                );
                (*g_st().curbuf).b_p_ro = 1;

                break 'theend;
            }

            if (check_readonly && file_readonly) || (*g_st().curbuf).b_help {
                (*g_st().curbuf).b_p_ro = 1;
            }

            if set_options {
                if !read_buffer {
                    (*g_st().curbuf).b_p_eof = 0;
                    (*g_st().curbuf).b_start_eof = 0;
                    (*g_st().curbuf).b_p_eol = 1;
                    (*g_st().curbuf).b_start_eol = 1;
                }
                (*g_st().curbuf).b_p_bomb = 0;
                (*g_st().curbuf).b_start_bomb = 0;
            }

            if !bt_dontwrite(g_st().curbuf) {
                check_need_swap(newfile);
                if !read_stdin
                    && (g_st().curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != (*g_st().curbuf).b_ffname)
                        || (using_b_fname && old_b_fname != (*g_st().curbuf).b_fname))
                {
                    emsg(tr(E_AUCHANGEDBUF.as_ptr()));
                    if !read_buffer {
                        close(fd);
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                {
                    let mfp = (*g_st().curbuf).b_ml.mfp_ptr();
                    if swap_mode > 0
                        && !mfp.is_null()
                        && let Some(ref swap_fname) = (*mfp).mf_fname
                    {
                        let swap_fname = swap_fname.as_ptr();
                        if (swap_mode & 0o44) == 0o40 {
                            let mut swap_info: FileInfo = FileInfo::new();
                            if os_fileinfo(swap_fname, &mut swap_info)
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

            if g_st().swap_exists_action == SEA_QUIT {
                if !read_buffer && !read_stdin {
                    close(fd);
                }
                break 'theend;
            }

            g_st().no_wait_return += 1;

            orig_start = (*g_st().curbuf).b_op_start;
            (*g_st().curbuf).b_op_start.lnum = if from == 0 { 1 } else { from };
            (*g_st().curbuf).b_op_start.col = 0;

            let mut try_mac = c_int::from(!vim_strchr(opt_vars().p_ffs, b'm' as c_int).is_null());
            let mut try_dos = c_int::from(!vim_strchr(opt_vars().p_ffs, b'd' as c_int).is_null());
            let mut try_unix = c_int::from(!vim_strchr(opt_vars().p_ffs, b'x' as c_int).is_null());

            if !read_buffer {
                let m = g_st().msg_scroll;
                let n = g_st().msg_scrolled;

                if !read_stdin {
                    close(fd);
                }

                g_st().msg_scroll = 1;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
                        eap,
                    );
                } else if read_stdin {
                    apply_autocmds_exarg(
                        EVENT_STDINREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
                        eap,
                    );
                } else if newfile {
                    apply_autocmds_exarg(
                        EVENT_BUFREADPRE,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
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

                try_mac = c_int::from(!vim_strchr(opt_vars().p_ffs, b'm' as c_int).is_null());
                try_dos = c_int::from(!vim_strchr(opt_vars().p_ffs, b'd' as c_int).is_null());
                try_unix = c_int::from(!vim_strchr(opt_vars().p_ffs, b'x' as c_int).is_null());
                (*g_st().curbuf).b_op_start = orig_start;

                if g_st().msg_scrolled == n {
                    g_st().msg_scroll = m;
                }

                if aborting() {
                    g_st().no_wait_return -= 1;
                    g_st().msg_scroll = msg_save;
                    (*g_st().curbuf).b_p_ro = 1;
                    break 'theend;
                }

                if !read_stdin
                    && (g_st().curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != (*g_st().curbuf).b_ffname)
                        || (using_b_fname && old_b_fname != (*g_st().curbuf).b_fname)
                        || {
                            fd = os_open(fname, O_RDONLY, 0);
                            fd < 0
                        })
                {
                    g_st().no_wait_return -= 1;
                    g_st().msg_scroll = msg_save;
                    if fd < 0 {
                        emsg(tr(
                            c"E200: *ReadPre autocommands made the file unreadable".as_ptr()
                        ));
                    } else {
                        emsg(tr(
                            c"E201: *ReadPre autocommands must not change current buffer".as_ptr(),
                        ));
                    }
                    (*g_st().curbuf).b_p_ro = 1;
                    break 'theend;
                }
            }

            let wasempty = ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0;

            let show_msg = !g_st().recoverymode
                && !filtering
                && (flags & READ_DUMMY) == 0
                && !read_stdin
                && !read_buffer
                && g_st().msg_silent == 0;
            if show_msg && !silent {
                filemess(g_st().curbuf, sfname, c"".as_ptr(), ptr::null());
            }

            let mut msg_id = [0 as c_char; MAXPATHL + 32];
            fname_msg_id(
                msg_id.as_mut_ptr(),
                msg_id.len(),
                c"nvim.bufread".as_ptr(),
                g_st().curbuf,
                sfname,
            );
            let mut progress = false;
            let mut percent: c_int = -1;
            let read_start = os_hrtime();

            g_st().msg_scroll = 0;

            let mut linecnt = (*g_st().curbuf).b_ml.ml_line_count;

            if !eap.is_null() && (*eap).bad_char != 0 {
                bad_char_behavior = (*eap).bad_char;
                if set_options {
                    (*g_st().curbuf).b_bad_char = (*eap).bad_char;
                }
            } else {
                (*g_st().curbuf).b_bad_char = 0;
            }

            if !eap.is_null() && (*eap).force_enc != 0 {
                fenc_owned = Some(enc_canonize((*eap).cmd.add((*eap).force_enc as usize)));
                fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut();
                keep_dest_enc = true;
            } else if (*g_st().curbuf).b_p_bin != 0 {
                fenc = c"".as_ptr().cast_mut();
            } else if (*g_st().curbuf).b_help {
                fenc_next = c"latin1".as_ptr().cast_mut();
                fenc = c"utf-8".as_ptr().cast_mut();
            } else if *opt_vars().p_fencs == NUL {
                fenc = (*g_st().curbuf).b_p_fenc;
            } else {
                fenc_next = opt_vars().p_fencs;
                fenc_owned = Some(next_fenc(&raw mut fenc_next));
                fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut();
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
                        (*g_st().curbuf).b_p_bomb = 0;
                        (*g_st().curbuf).b_start_bomb = 0;
                    }
                    conv_error = 0;
                }

                if keep_fileformat {
                    keep_fileformat = false;
                } else if !eap.is_null() && (*eap).force_ff != 0 {
                    fileformat = get_fileformat_force(g_st().curbuf, eap);
                    try_unix = 0;
                    try_dos = 0;
                    try_mac = 0;
                } else if (*g_st().curbuf).b_p_bin != 0 {
                    fileformat = EOL_UNIX;
                } else if *opt_vars().p_ffs == NUL {
                    fileformat = get_fileformat(&*g_st().curbuf);
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
                        fenc_owned = None;
                        fenc = c"".as_ptr().cast_mut();
                    } else if !fenc_next.is_null() {
                        fenc_owned = Some(next_fenc(&raw mut fenc_next));
                        fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut();
                    } else {
                        fenc_owned = None;
                        fenc = c"".as_ptr().cast_mut();
                    }
                    if let Some(ref tn) = tmpname {
                        os_remove(tn.as_ptr());
                        tmpname = None;
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
                        iconv_fd = my_iconv_open(c"utf-8".as_ptr().cast_mut(), fenc)
                            .map_or(-1, |fd| fd as iconv_t);
                    }

                    if fio_flags == 0
                        && !read_stdin
                        && !read_buffer
                        && crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone
                        && !read_fifo
                        && iconv_fd == -1
                    {
                        did_iconv = false;
                        if tmpname.is_none() {
                            tmpname = readfile_charconvert(fname, fenc, &mut fd);
                            if tmpname.is_none() {
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
                        && !(*g_st().curbuf).b_ffname.is_null()
                        && (*g_st().curbuf).b_p_udf != 0
                        && !filtering
                        && !read_fifo
                        && !read_stdin
                        && !read_buffer;
                    if read_undo_file {
                        sha_ctx = context_sha256_T::new();
                    }
                }

                while !error && !g_st().got_int {
                    {
                        if !skip_read {
                            size = (0x10000 + linerest).min(0x100000);
                        }

                        if size < 0 || size + linerest + 1 < 0 || linerest >= MAXCOL - size {
                            split += 1;
                            *ptr = NL;
                            size = 1;
                        } else if !skip_read {
                            let mut new_buffer_vec: Vec<c_char> =
                                vec![0; (size as usize) + (linerest as usize) + 1];
                            let new_buffer = new_buffer_vec.as_mut_ptr();
                            if linerest != 0 {
                                ptr::copy(ptr.offset(-linerest), new_buffer, linerest as usize);
                            }
                            buffer_vec = new_buffer_vec;
                            buffer = buffer_vec.as_mut_ptr();
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
                                            if (*g_st().curbuf).b_p_eol == 0 {
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
                                            if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone && iconv_fd != -1 {
                                                did_iconv = true;
                                            } else {
                                                advance_fenc = true;
                                            }
                                            file_rewind = true;
                                            continue 'retry;
                                        }
                                        if conv_error == 0 {
                                            conv_error = (*g_st().curbuf).b_ml.ml_line_count - linecnt + 1;
                                        }
                                    } else if illegal_byte == 0 {
                                        illegal_byte = (*g_st().curbuf).b_ml.ml_line_count - linecnt + 1;
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
                                || ((*g_st().curbuf).b_p_bomb == 0
                                    && tmpname.is_none()
                                    && (*fenc == b'u' as c_char || *fenc == NUL)))
                        {
                            let mut blen: c_int = 0;
                            let ccname = if size < 2 || (*g_st().curbuf).b_p_bin != 0 {
                                ptr::null()
                            } else {
                                check_for_bom(
                                    ptr,
                                    size as c_int,
                                    &mut blen,
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
                                    (*g_st().curbuf).b_p_bomb = 1;
                                    (*g_st().curbuf).b_start_bomb = 1;
                                }
                            }

                            if fio_flags == FIO_UCSBOM {
                                if ccname.is_null() {
                                    advance_fenc = true;
                                } else {
                                    fenc_owned = None;
                                    fenc = ccname.cast_mut();
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
                                && os_errno() != ICONV_EINVAL)
                                || from_size > CONV_RESTLEN
                            {
                                if can_retry {
                                    if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone && iconv_fd != -1 {
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
                                                if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone
                                                    && iconv_fd != -1
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
                                                if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone
                                                    && iconv_fd != -1
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
                                                if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone
                                                    && iconv_fd != -1
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
                        } else if (*g_st().curbuf).b_p_bin == 0 {
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
                                if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv) != kCallbackNone && iconv_fd != -1 {
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
                                    if read_undo_file && len > 0 {
                                        sha_ctx.update(std::slice::from_raw_parts(
                                            line_start.cast::<u8>(),
                                            len as usize,
                                        ));
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
                                if read_undo_file && len > 0 {
                                    sha_ctx.update(std::slice::from_raw_parts(
                                        line_start.cast::<u8>(),
                                        len as usize,
                                    ));
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
                        let mut info: FileInfo = FileInfo::new();
                        let total = if os_fileinfo_fd(fd, &mut info) {
                            os_fileinfo_size(&info)
                        } else {
                            0
                        };
                        let pct = ((filesize as u64) * 100)
                            .checked_div(total)
                            .map_or(-1, |v| (v.min(100)) as c_int);
                        if !progress || pct != percent {
                            progress = true;
                            percent = pct;
                            let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                            add_quoted_fname(iobuf_ptr, IOSIZE, g_st().curbuf, sfname);
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
                && (*g_st().curbuf).b_p_bin == 0
                && fileformat == EOL_DOS
                && *ptr.sub(1) == CTRL_Z
            {
                ptr = ptr.sub(1);
                linerest -= 1;
                if set_options {
                    (*g_st().curbuf).b_p_eof = 1;
                }
            }

            if !error && !g_st().got_int && linerest != 0 {
                if set_options {
                    (*g_st().curbuf).b_p_eol = 0;
                }
                *ptr = NUL;
                let len = (ptr.offset_from(line_start) + 1) as colnr_T;
                if ml_append(lnum, line_start, len, newfile) == FAIL {
                    error = true;
                } else {
                    if read_undo_file && len > 0 {
                        sha_ctx.update(std::slice::from_raw_parts(
                            line_start.cast::<u8>(),
                            len as usize,
                        ));
                    }
                    lnum += 1;
                    read_no_eol_lnum = lnum;
                }
            }

            if set_options {
                save_file_ff(&mut *g_st().curbuf);
                set_option_direct(kOptFileencoding, cstr_as_obj(fenc), OPT_LOCAL, 0);
            }
            drop(fenc_owned);
            if iconv_fd != -1 {
                call_iconv_close(iconv_fd);
            }

            if !read_buffer && !read_stdin {
                close(fd);
            } else {
                os_set_cloexec(fd);
            }
            drop(buffer_vec);

            if read_stdin {
                close(fd);
                if g_st().stdin_fd < 0 {
                    #[cfg(not(windows))]
                    {
                        g_st().vim_ignored = dup(2);
                    }
                    #[cfg(windows)]
                    {
                        g_st().vim_ignored = os_open_conin_fd();
                    }
                }
            }

            if let Some(ref tn) = tmpname {
                os_remove(tn.as_ptr());
            }
            g_st().no_wait_return -= 1;

            if !g_st().recoverymode {
                if newfile && wasempty && ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
                    ml_delete((*g_st().curbuf).b_ml.ml_line_count);
                    linecnt -= 1;
                }
                (*g_st().curbuf).deleted_bytes = 0;
                (*g_st().curbuf).deleted_bytes2 = 0;
                (*g_st().curbuf).deleted_codepoints = 0;
                (*g_st().curbuf).deleted_codeunits = 0;
                linecnt = (*g_st().curbuf).b_ml.ml_line_count - linecnt;
                if filesize == 0 {
                    linecnt = 0;
                }
                if newfile || read_buffer {
                    redraw_curbuf_later(UPD_NOT_VALID);
                    diff_invalidate(g_st().curbuf);
                    foldUpdateAll(&mut **curwin.as_ptr());
                } else if linecnt != 0 {
                    appended_lines_mark(from, linecnt);
                }

                if g_st().got_int {
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
                        filemess(g_st().curbuf, sfname, tr(e_interr.as_ptr()), ptr::null());
                        if newfile {
                            (*g_st().curbuf).b_p_ro = 1;
                        }
                    }
                    g_st().msg_scroll = msg_save;
                    check_marks_read();
                    retval = OK;
                    break 'theend;
                }

                if !filtering && (flags & READ_DUMMY) == 0 && !silent {
                    let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
                    add_quoted_fname(iobuf_ptr, IOSIZE, g_st().curbuf, sfname);
                    let mut c = false;

                    let mut buflen = std::ffi::CStr::from_ptr(iobuf_ptr).to_bytes().len() as c_int;
                    #[cfg(unix)]
                    {
                        if s_isfifo(perm as u64) {
                            buflen += vim_snprintf!(
                                iobuf_ptr.offset(buflen as isize),
                                (IOSIZE as c_int - buflen) as usize,
                                c"%s".as_ptr(),
                                tr(c"[fifo]".as_ptr()),
                            );
                            c = true;
                        }
                        if s_issock(perm as u64) {
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
                    if (*g_st().curbuf).b_p_ro != 0 {
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

                    g_st().keep_msg = None;
                    p = ptr::null_mut();
                    g_st().msg_scrolled_ign = true;

                    if !read_stdin && !read_buffer {
                        if g_st().msg_col > 0 {
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
                        || g_st().restart_edit != 0
                        || (g_st().msg_scrolled != 0 && !g_st().need_wait_return)
                    {
                        set_keep_msg(p.cast(), 0);
                    }
                    g_st().msg_scrolled_ign = false;
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
                    (*g_st().curbuf).b_p_ro = 1;
                }

                u_clearline(&mut *g_st().curbuf);

                (**curwin.as_ptr()).w_cursor.lnum = from + 1;
                check_cursor_lnum(&mut **curwin.as_ptr());
                beginline(BL_WHITE | BL_FIX);

                if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                    (*g_st().curbuf).b_op_start.lnum = from + 1;
                    (*g_st().curbuf).b_op_start.col = 0;
                    (*g_st().curbuf).b_op_end.lnum = from + linecnt;
                    (*g_st().curbuf).b_op_end.col = 0;
                }
            }
            g_st().msg_scroll = msg_save;

            check_marks_read();

            (*g_st().curbuf).b_no_eol_lnum = read_no_eol_lnum;

            if (flags & READ_KEEP_UNDO) != 0 {
                u_find_first_changed();
            }

            if read_undo_file {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                sha_ctx.finish_into(&mut hash);
                u_read_undo(ptr::null_mut(), hash.as_ptr(), fname);
            }

            if !read_stdin && !read_fifo && (!read_buffer || !sfname.is_null()) {
                let m = g_st().msg_scroll;
                let n = g_st().msg_scrolled;

                if set_options {
                    save_file_ff(&mut *g_st().curbuf);
                }

                g_st().msg_scroll = 1;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPOST,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
                        eap,
                    );
                } else if newfile || (read_buffer && !sfname.is_null()) {
                    let did_bufrp = apply_autocmds_exarg(
                        EVENT_BUFREADPOST,
                        ptr::null_mut(),
                        sfname,
                        false,
                        g_st().curbuf,
                        eap,
                    );

                    if !(*g_st().curbuf).b_au_did_filetype && *(*g_st().curbuf).b_p_ft != NUL {
                        apply_autocmds(
                            EVENT_FILETYPE,
                            (*g_st().curbuf).b_p_ft,
                            (*g_st().curbuf).b_fname,
                            true,
                            g_st().curbuf,
                        );
                    }

                    if !did_bufrp
                        && *(*g_st().curbuf).b_p_ft == NUL
                        && augroup_exists(c"filetypedetect".as_ptr())
                    {
                        apply_autocmds_group(
                            EVENT_BUFREADPOST,
                            sfname,
                            ptr::null_mut(),
                            true,
                            augroup_find(c"filetypedetect".as_ptr()),
                            g_st().curbuf,
                            *curwin.as_ptr(),
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
                if g_st().msg_scrolled == n {
                    g_st().msg_scroll = m;
                }
                if aborting() {
                    return FAIL;
                }
            }

            if !(g_st().recoverymode && error) {
                retval = OK;
            }
        }

        let mfp = (*g_st().curbuf).b_ml.mfp_ptr();
        if !mfp.is_null() && (*mfp).mf_dirty == MF_DIRTY_YES_NOSYNC {
            (*mfp).mf_dirty = MF_DIRTY_YES;
        }

        retval
    }
}

/// Returns true if the file name argument is of the form "/dev/fd/\d\+".
#[cfg(test)]
pub fn is_dev_fd_file(fname: *mut c_char) -> bool {
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
fn readfile_linenr(linecnt: linenr_T, p: *mut c_char, endp: *const c_char) -> linenr_T {
    unsafe {
        let mut lnum = (*g_st().curbuf).b_ml.ml_line_count - linecnt + 1;
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
pub fn prep_exarg(eap: &mut exarg_T, buf: &buf_T) -> std::ffi::CString {
    unsafe {
        let fenc_bytes = CStr::from_ptr(buf.b_p_fenc).to_bytes();
        let mut cmd_vec = Vec::with_capacity(8 + fenc_bytes.len());
        cmd_vec.extend_from_slice(b"e ++enc=");
        cmd_vec.extend_from_slice(fenc_bytes);
        let cmd_cstr = std::ffi::CString::new(cmd_vec).unwrap_or_default();
        eap.cmd = cmd_cstr.as_ptr().cast_mut();
        eap.force_enc = 8;
        eap.bad_char = buf.b_bad_char;
        eap.force_ff = (*buf.b_p_ff as u8) as c_int;
        eap.force_bin = if buf.b_p_bin != 0 {
            FORCE_BIN
        } else {
            FORCE_NOBIN
        };
        eap.read_edit = 0;
        eap.forceit = 0;
        cmd_cstr
    }
}

/// Set default or forced 'fileformat' and 'binary'.
pub fn set_file_options(set_options: bool, eap: *mut exarg_T) {
    unsafe {
        if set_options {
            if !eap.is_null() && (*eap).force_ff != 0 {
                set_fileformat(get_fileformat_force(g_st().curbuf, eap), OPT_LOCAL);
            } else if *opt_vars().p_ffs != NUL {
                set_fileformat(default_fileformat(), OPT_LOCAL);
            }
        }

        if !eap.is_null() && (*eap).force_bin != 0 {
            let oldval = (*g_st().curbuf).b_p_bin;
            (*g_st().curbuf).b_p_bin = c_int::from((*eap).force_bin == FORCE_BIN);
            set_options_bin(oldval, (*g_st().curbuf).b_p_bin, OPT_LOCAL);
        }
    }
}

/// Set forced 'fileencoding'.
pub fn set_forced_fenc(eap: &mut exarg_T) {
    unsafe {
        if eap.force_enc == 0 {
            return;
        }

        let fenc = enc_canonize(eap.cmd.add(eap.force_enc as usize));
        set_option_direct(kOptFileencoding, cstr_as_obj(fenc.as_ptr()), OPT_LOCAL, 0);
    }
}

/// Find next fileencoding to use from 'fileencodings'.
fn next_fenc(pp: *mut *mut c_char) -> std::ffi::CString {
    unsafe {
        if **pp == NUL {
            *pp = ptr::null_mut();
            return std::ffi::CString::default();
        }
        let p = vim_strchr(*pp, b',' as c_int);
        if p.is_null() {
            let res = enc_canonize(*pp);
            *pp = (*pp).add(std::ffi::CStr::from_ptr(*pp).to_bytes().len());
            res
        } else {
            let len = p.offset_from(*pp) as usize;
            let tmp = std::ffi::CString::new(std::slice::from_raw_parts((*pp).cast::<u8>(), len))
                .unwrap_or_default();
            *pp = p.add(1);
            enc_canonize(tmp.as_ptr())
        }
    }
}

/// Convert a file with the 'charconvert' expression.
fn readfile_charconvert(
    fname: *mut c_char,
    fenc: *mut c_char,
    fdp: &mut c_int,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut errmsg: *mut c_char = ptr::null_mut();

        let mut tmpname = vim_tempname();
        match tmpname {
            None => {
                errmsg = tr(c"Can't find temp file for conversion".as_ptr());
            }
            Some(ref tn) => {
                close(*fdp);
                *fdp = -1;
                if eval_charconvert(fenc, c"utf-8".as_ptr(), fname, tn.as_ptr()) == FAIL {
                    errmsg = tr(c"Conversion with 'charconvert' failed".as_ptr());
                }
                if errmsg.is_null() {
                    *fdp = os_open(tn.as_ptr(), O_RDONLY, 0);
                    if *fdp < 0 {
                        errmsg = tr(c"can't read output of 'charconvert'".as_ptr());
                    }
                }
            }
        }

        if !errmsg.is_null() {
            msg(errmsg, 0);
            if let Some(ref tn) = tmpname {
                os_remove(tn.as_ptr());
                tmpname = None;
            }
        }

        if *fdp < 0 {
            *fdp = os_open(fname, O_RDONLY, 0);
        }

        tmpname
    }
}

/// Set the name of the current buffer.
pub fn set_rw_fname(fname: *mut c_char, sfname: *mut c_char) -> c_int {
    unsafe {
        let buf = g_st().curbuf;

        if (*g_st().curbuf).b_p_bl != 0 {
            apply_autocmds(
                EVENT_BUFDELETE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        apply_autocmds(
            EVENT_BUFWIPEOUT,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        if aborting() {
            return FAIL;
        }
        if g_st().curbuf != buf {
            emsg(tr(E_AUCHANGEDBUF.as_ptr()));
            return FAIL;
        }

        if setfname(g_st().curbuf, fname, sfname, false) == OK {
            (*g_st().curbuf).b_flags |= BF_NOTEDITED;
        }

        apply_autocmds(
            EVENT_BUFNEW,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        if (*g_st().curbuf).b_p_bl != 0 {
            apply_autocmds(
                EVENT_BUFADD,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                g_st().curbuf,
            );
        }
        if aborting() {
            return FAIL;
        }

        if *(*g_st().curbuf).b_p_ft == NUL {
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
pub fn add_quoted_fname(
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
pub fn fname_msg_id(
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
        let total = std::ffi::CStr::from_ptr(msg_id).to_bytes().len();
        if total > 0 {
            *msg_id.add(total - 1) = NUL;
        }
    }
}

/// Append message for text mode to IObuff.
pub fn msg_add_fileformat(eol_type: c_int) -> bool {
    unsafe {
        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        #[cfg(not(windows))]
        if eol_type == EOL_DOS {
            { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), tr(c"[dos]".as_ptr()), _dsize - _dlen); };
            return true;
        }
        if eol_type == EOL_MAC {
            { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), tr(c"[mac]".as_ptr()), _dsize - _dlen); };
            return true;
        }
        #[cfg(windows)]
        if eol_type == EOL_UNIX {
            { let _dst: *mut c_char = iobuf_ptr; let _dsize: usize = IOSIZE; let _dlen = std::ffi::CStr::from_ptr(_dst).to_bytes().len(); crate::memory::xstrlcpy(_dst.add(_dlen), tr(c"[unix]".as_ptr()), _dsize - _dlen); };
            return true;
        }
        false
    }
}

/// Append line and character count to IObuff.
pub fn msg_add_lines(insert_space: c_int, lnum: linenr_T, nchars: off_T) {
    unsafe {
        let iobuf_ptr = (&raw mut g_st().IObuff).cast::<c_char>();
        let mut len = std::ffi::CStr::from_ptr(iobuf_ptr).to_bytes().len();
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

pub fn time_differs(file_info: &FileInfo, mtime: i64, mtime_ns: i64) -> bool {
    let sec: i64 = file_info.stat.st_mtim.tv_sec as _;
        let nsec: i64 = file_info.stat.st_mtim.tv_nsec as _;
        #[cfg(any(target_os = "linux", windows))]
        {
            nsec != mtime_ns || sec - mtime > 1 || mtime - sec > 1
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            nsec != mtime_ns || sec != mtime
        }
}

/// Return true if file encoding "fenc" requires conversion from or to 'encoding'.
pub fn need_conversion(fenc: *const c_char) -> bool {
    unsafe {
        let same_encoding: bool;
        let fenc_flags: c_int;

        if *fenc == NUL || CStr::from_ptr(opt_vars().p_enc) == CStr::from_ptr(fenc) {
            same_encoding = true;
            fenc_flags = 0;
        } else {
            let enc_flags = get_fio_flags(opt_vars().p_enc);
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
pub fn get_fio_flags(mut name: *const c_char) -> c_int {
    unsafe {
        if name.is_null() || *name == NUL {
            name = if opt_vars().p_enc.is_null() {
                c"utf-8".as_ptr()
            } else {
                opt_vars().p_enc
            };
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
fn check_for_bom(
    p_in: *const c_char,
    size: c_int,
    lenp: &mut c_int,
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
pub fn shorten_buf_fname(buf: &mut buf_T, dirname: *mut c_char, force: c_int) -> bool {
    unsafe {
        if buf.b_fname.is_null()
            || bt_nofilename(&raw mut *buf)
            || path_with_url(buf.b_fname) != 0
            || !(force != 0 || buf.b_sfname.is_null() || path_is_absolute(buf.b_sfname))
        {
            return false;
        }

        let p = path_shorten_fname(buf.b_ffname, dirname);
        let shorten = !p.is_null() && *p != NUL;
        if shorten
            && !buf.b_sfname.is_null()
            && CStr::from_ptr(p) == CStr::from_ptr(buf.b_sfname)
        {
            return false;
        }

        let new_fname = if shorten { p } else { buf.b_ffname };
        let changed =
            new_fname.is_null() || CStr::from_ptr(new_fname) != CStr::from_ptr(buf.b_fname);

        buf.set_sfname(shorten.then(|| cstr_to_cstring(p)));
        if shorten {
            buf.b_fname = buf.b_sfname;
        } else {
            buf.b_fname = buf.b_ffname;
        }
        changed
    }
}

/// Shortens filenames for all buffers.
pub fn shorten_fnames(force: c_int) {
    unsafe {
        let mut dirname: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut changed = false;

        os_dirname(dirname.as_mut_ptr(), MAXPATHL);
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            changed |= shorten_buf_fname(&mut *buf, dirname.as_mut_ptr(), force);
            mf_fullname((*buf).b_ml.mfp_ptr());
            buf = (*buf).b_next;
        }
        if changed {
            status_redraw_all();
            g_st().redraw_tabline = true;
        }
    }
}

/// Get new filename ended by given extension.
pub fn modname(
    fname: *const c_char,
    ext: *const c_char,
    mut prepend_dot: bool,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut retval_vec: Vec<c_char>;
        let retval: *mut c_char;
        let fnamelen: usize;
        let extlen = std::ffi::CStr::from_ptr(ext).to_bytes().len();

        if fname.is_null() || *fname == NUL {
            retval_vec = vec![0; MAXPATHL + extlen + 3];
            retval = retval_vec.as_mut_ptr();
            if os_dirname(retval, MAXPATHL) == FAIL || std::ffi::CStr::from_ptr(retval).to_bytes().is_empty() {
                return None;
            }
            add_pathsep(retval);
            fnamelen = std::ffi::CStr::from_ptr(retval).to_bytes().len();
            prepend_dot = false;
        } else {
            fnamelen = std::ffi::CStr::from_ptr(fname).to_bytes().len();
            retval_vec = vec![0; fnamelen + extlen + 3];
            retval = retval_vec.as_mut_ptr();
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
        Some(CStr::from_ptr(retval).to_owned())
    }
}

/// Like fgets(), but if the file line is too long, it is truncated and the
/// rest of the line is thrown away.
pub fn vim_fgets(buf: *mut c_char, size: c_int, fp: *mut FILE) -> bool {
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
            os_set_errno(0);
            retval = fgets(buf, size, fp);
            if !retval.is_null() || os_errno() != EINTR || ferror(fp) == 0 {
                break;
            }
        }

        let penultimate = *buf.offset((size - 2) as isize);
        if penultimate != NUL && penultimate != b'\n' as c_char {
            let mut tbuf: [c_char; 200] = [0; 200];

            *buf.offset((size - 1) as isize) = NUL;

            loop {
                tbuf[tbuf.len() - 2] = NUL;
                os_set_errno(0);
                retval = fgets(tbuf.as_mut_ptr(), tbuf.len() as c_int, fp);
                if retval.is_null() && (feof(fp) != 0 || os_errno() != EINTR) {
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
pub fn get2c(fd: *mut FILE) -> c_int {
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
pub fn get3c(fd: *mut FILE) -> c_int {
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
pub fn get4c(fd: *mut FILE) -> c_int {
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
pub fn get8ctime(fd: *mut FILE) -> time_t {
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
pub fn read_string(fd: *mut FILE, cnt: usize) -> Option<Vec<c_char>> {
    unsafe {
        let mut buf: Vec<c_char> = Vec::with_capacity(cnt + 1);
        for _ in 0..cnt {
            let c = fgetc(fd);
            if c == EOF {
                return None;
            }
            buf.push(c as c_char);
        }
        buf.push(0);
        Some(buf)
    }
}

/// Writes a number to file "fd", most significant bit first, in "len" bytes.
pub fn put_bytes(fd: *mut FILE, number: u64, len: usize) -> bool {
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
pub fn put_time(fd: *mut FILE, time_: time_t) -> c_int {
    unsafe {
        let buf: [u8; 8] = (time_ as u64).to_be_bytes();
        if fwrite(buf.as_ptr().cast(), size_of::<u8>(), buf.len(), fd) == 1 {
            OK
        } else {
            FAIL
        }
    }
}

fn rename_with_tmp(from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        if std::ffi::CStr::from_ptr(from).to_bytes().len() >= MAXPATHL - 5 {
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
pub fn vim_rename(from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        let mut use_tmp_file = false;

        if path_equal(from, to, kPathCmpLiteral) {
            if opt_vars().p_fic != 0 && CStr::from_ptr(path_tail(from)) != CStr::from_ptr(path_tail(to)) {
                use_tmp_file = true;
            } else {
                return 0;
            }
        }

        let mut from_info: FileInfo = FileInfo::new();
        if !os_fileinfo(from, &mut from_info) {
            return -1;
        }

        let mut to_info: FileInfo = FileInfo::new();
        if os_fileinfo(to, &mut to_info)
            && os_fileinfo_id_equal(&from_info, &to_info)
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

        if os_fileinfo(from, &mut from_info) {
            os_remove(from);
        }

        0
    }
}

/// Create the new file with same permissions as the original.
pub fn vim_copyfile(from: *const c_char, to: *const c_char) -> c_int {
    #[cfg(not(windows))]
    unsafe {
        let mut from_info: FileInfo = FileInfo::new();
        if os_fileinfo_link(from, &mut from_info) && s_islnk(from_info.stat.st_mode) {
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

/// Check if any not hidden buffer has been changed.
pub fn check_timestamps(focus: c_int) -> c_int {
    unsafe {
        if g_st().no_check_timestamps > 0 {
            return 0;
        }

        if focus != 0 && g_st().did_check_timestamps {
            g_st().need_check_timestamps = true;
            return 0;
        }

        let mut didit: c_int = 0;

        if !stuff_empty()
            || g_st().global_busy != 0
            || typebuf_typed() == 0
            || g_st().autocmd_busy
            || (*g_st().curbuf).b_ro_locked > 0
            || g_st().allbuf_lock > 0
        {
            g_st().need_check_timestamps = true;
        } else {
            g_st().no_wait_return += 1;
            g_st().did_check_timestamps = true;
            fio_st().ALREADY_WARNED = false;
            let mut buf = g_st().firstbuf;
            while !buf.is_null() {
                if (*buf).b_nwindows > 0 {
                    let mut bufref: bufref_T = bufref_T::new();
                    set_bufref(&mut bufref, buf);
                    let n = buf_check_timestamp(&mut *buf);
                    didit = didit.max(n);
                    if n > 0 && !bufref_valid(&raw mut bufref) {
                        buf = g_st().firstbuf;
                        continue;
                    }
                }
                buf = (*buf).b_next;
            }
            g_st().no_wait_return -= 1;
            g_st().need_check_timestamps = false;
            if g_st().need_wait_return && didit == 2 {
                msg_puts(c"\n".as_ptr());
                ui_flush();
            }
        }
        didit
    }
}

/// Move all the lines from buffer "frombuf" to buffer "tobuf".
fn move_lines(frombuf: *mut buf_T, tobuf: *mut buf_T) -> c_int {
    unsafe {
        let tbuf = g_st().curbuf;
        let mut retval = OK;

        g_st().curbuf = tobuf;
        for lnum in 1..=(*frombuf).b_ml.ml_line_count {
            let p = CStr::from_ptr(ml_get_buf(frombuf, lnum)).to_owned();
            if ml_append(lnum - 1, p.as_ptr().cast_mut(), 0, false) == FAIL {
                retval = FAIL;
                break;
            }
        }

        if retval != FAIL {
            g_st().curbuf = frombuf;
            let mut lnum = (*g_st().curbuf).b_ml.ml_line_count;
            while lnum > 0 {
                if ml_delete(lnum) == FAIL {
                    retval = FAIL;
                    break;
                }
                lnum -= 1;
            }
        }

        g_st().curbuf = tbuf;
        retval
    }
}

/// Check if buffer "buf" has been changed.
pub fn buf_check_timestamp(buf: &mut buf_T) -> c_int {
    unsafe {
        static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

        const RELOAD_NONE: c_int = 0;
        const RELOAD_NORMAL: c_int = 1;
        const RELOAD_DETECT: c_int = 2;

        let mut retval: c_int = 0;
        let mut mesg: *mut c_char = ptr::null_mut();
        let mut mesg2: *mut c_char = c"".as_ptr().cast_mut();
        let mut helpmesg = false;
        let mut reload: c_int = RELOAD_NONE;
        let mut can_reload = false;
        let orig_size = buf.b_orig_size;
        let orig_mode = buf.b_orig_mode;

        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, &raw mut *buf);

        if !buf.terminal.is_null()
            || buf.b_ffname.is_null()
            || buf.b_ml.ml_mfp.is_none()
            || !bt_normal(&raw mut *buf)
            || buf.b_saving
            || BUSY.load(std::sync::atomic::Ordering::Relaxed)
        {
            return 0;
        }

        let mut file_info: FileInfo = FileInfo::new();
        let file_info_ok: bool;
        if (buf.b_flags & BF_NOTEDITED) == 0
            && buf.b_mtime != 0
            && ({
                file_info_ok = os_fileinfo(buf.b_ffname, &mut file_info);
                !file_info_ok
                    || time_differs(&file_info, buf.b_mtime, buf.b_mtime_ns)
                    || (file_info.stat.st_mode as c_int) != buf.b_orig_mode
            })
        {
            let prev_b_mtime = buf.b_mtime;

            retval = 1;

            if !file_info_ok {
                buf.b_mtime = -1;
                buf.b_orig_size = 0;
                buf.b_orig_mode = 0;
            } else {
                buf_store_file_info(&mut *buf, &mut file_info);
            }

            if os_isdir(buf.b_fname) {
                // Don't do anything for a directory.
            } else if (if buf.b_p_ar >= 0 {
                buf.b_p_ar
            } else {
                opt_vars().p_ar
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
                } else if orig_size != buf.b_orig_size || buf_contents_changed(&raw mut *buf) {
                    (c"changed".as_ptr(), 7isize)
                } else if orig_mode != buf.b_orig_mode {
                    (c"mode".as_ptr(), 4isize)
                } else {
                    (c"time".as_ptr(), 4isize)
                };

                BUSY.store(true, std::sync::atomic::Ordering::Relaxed);
                set_vim_var_string(VV_FCS_REASON, reason, reasonlen);
                set_vim_var_string(VV_FCS_CHOICE, c"".as_ptr(), 0);
                g_st().allbuf_lock += 1;
                let mut n = apply_autocmds(
                    EVENT_FILECHANGEDSHELL,
                    buf.b_fname,
                    buf.b_fname,
                    false,
                    &raw mut *buf,
                );
                g_st().allbuf_lock -= 1;
                BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
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
                            buf.b_mtime_read = buf.b_mtime;
                            buf.b_mtime_read_ns = buf.b_mtime_ns;
                        }
                    }
                }
            }
        } else if (buf.b_flags & BF_NEW) != 0
            && (buf.b_flags & BF_NEW_W) == 0
            && os_path_exists(buf.b_ffname)
        {
            retval = 1;
            mesg = tr(c"W13: Warning: File \"%s\" has been created after editing started".as_ptr());
            buf.b_flags |= BF_NEW_W;
            can_reload = true;
        }

        if !mesg.is_null() {
            let path = home_replace_save(&raw mut *buf, buf.b_fname);
            if !helpmesg {
                mesg2 = c"".as_ptr().cast_mut();
            }
            let tbufsize = path.as_bytes().len() + std::ffi::CStr::from_ptr(mesg).to_bytes().len() + std::ffi::CStr::from_ptr(mesg2).to_bytes().len() + 3;
            let mut tbuf_vec = vec![0 as c_char; tbufsize];
            let tbuf = tbuf_vec.as_mut_ptr();
            let tbuflen = vim_snprintf!(tbuf, tbufsize, mesg, path.as_ptr());
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
            } else if g_st().State > MODE_NORMAL_BUSY || (g_st().State & MODE_CMDLINE) != 0 || fio_st().ALREADY_WARNED {
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
                if !g_st().autocmd_busy {
                    msg_start();
                    msg_puts_hl(tbuf, HLF_E, true);
                    if *mesg2 != NUL {
                        msg_puts_hl(mesg2, HLF_W, true);
                    }
                    msg_clr_eos();
                    msg_end();
                    if g_st().emsg_silent == 0 && !g_st().in_assert_fails && !ui_has(kUIMessages) {
                        msg_delay(1004, true);
                        g_st().redraw_cmdline = false;
                    }
                }
                fio_st().ALREADY_WARNED = true;
            }

            drop(tbuf_vec);
            drop(path);
        }

        if reload != RELOAD_NONE {
            buf_reload(&raw mut *buf, orig_mode, reload == RELOAD_DETECT);
            if bufref_valid(&raw mut bufref) && buf.b_p_udf != 0 && !buf.b_ffname.is_null() {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                u_compute_hash(buf, hash.as_mut_ptr());
                u_write_undo(ptr::null(), false, &raw mut *buf, hash.as_mut_ptr());
            }
        }

        if bufref_valid(&raw mut bufref) && retval != 0 {
            apply_autocmds(
                EVENT_FILECHANGEDSHELLPOST,
                buf.b_fname,
                buf.b_fname,
                false,
                &raw mut *buf,
            );
        }
        retval
    }
}

/// Reload a buffer that is already loaded.
pub fn buf_reload(buf: *mut buf_T, orig_mode: c_int, reload_options: bool) {
    unsafe {
        let mut ea: exarg_T = exarg_T::new();
        let old_ro = (*buf).b_p_ro;
        let savebuf: *mut buf_T;
        let mut bufref: bufref_T = bufref_T::new();
        let mut saved = OK;
        let mut aco: CtxSwitch = CtxSwitch::new();
        let mut flags = READ_NEW;

        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);

        mc_buf_clear(buf);

        let _ea_cmd = if !reload_options {
            Some(prep_exarg(&mut ea, &*buf))
        } else {
            None
        };

        let old_cursor = (**curwin.as_ptr()).w_cursor;
        let old_topline = (**curwin.as_ptr()).w_topline;

        if opt_vars().p_ur < 0 || i64::from((*g_st().curbuf).b_ml.ml_line_count) <= opt_vars().p_ur {
            u_sync(false);
            saved = u_savecommon(g_st().curbuf, 0, (*g_st().curbuf).b_ml.ml_line_count + 1, 0, true);
            flags |= READ_KEEP_UNDO;
        }

        if buf_is_empty(&mut *g_st().curbuf) || saved == FAIL {
            savebuf = ptr::null_mut();
        } else {
            savebuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
            set_bufref(&mut bufref, savebuf);
            if !savebuf.is_null() && buf == g_st().curbuf {
                g_st().curbuf = savebuf;
                (**curwin.as_ptr()).w_buffer = savebuf;
                saved = ml_open(&mut *g_st().curbuf);
                g_st().curbuf = buf;
                (**curwin.as_ptr()).w_buffer = buf;
            }
            if savebuf.is_null()
                || saved == FAIL
                || buf != g_st().curbuf
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
            (*g_st().curbuf).b_flags |= BF_CHECK_RO;
            (*g_st().curbuf).b_keep_filetype = true;
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
                if !savebuf.is_null() && bufref_valid(&raw mut bufref) && buf == g_st().curbuf {
                    while !buf_is_empty(&mut *g_st().curbuf) {
                        if ml_delete((*buf).b_ml.ml_line_count) == FAIL {
                            break;
                        }
                    }
                    move_lines(savebuf, buf);
                }
            } else if buf == g_st().curbuf {
                unchanged(buf, true, true);
                if (flags & READ_KEEP_UNDO) == 0 {
                    u_clearallandblockfree(&mut *buf);
                } else {
                    u_unchanged(&mut *g_st().curbuf);
                }
                buf_updates_unload(&mut *g_st().curbuf, true);
                (*g_st().curbuf).b_mod_set = true;
            }
        }
        drop(_ea_cmd);

        if !savebuf.is_null() && bufref_valid(&raw mut bufref) {
            wipe_buffer(savebuf, false);
        }

        diff_invalidate(g_st().curbuf);

        (**curwin.as_ptr()).w_topline = old_topline.min((*g_st().curbuf).b_ml.ml_line_count);
        (**curwin.as_ptr()).w_cursor = old_cursor;
        check_cursor(*curwin.as_ptr());
        update_topline(*curwin.as_ptr());
        (*g_st().curbuf).b_keep_filetype = false;

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if (*wp).w_buffer == (**curwin.as_ptr()).w_buffer && !foldmethodIsManual(&mut *wp) {
                    foldUpdateAll(&mut *wp);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        if orig_mode == (*g_st().curbuf).b_orig_mode {
            (*g_st().curbuf).b_p_ro |= old_ro;
        }

        do_modelines(0);

        ctx_restore(&mut aco);
    }
}

pub fn buf_store_file_info(buf: &mut buf_T, file_info: &mut FileInfo) {
    buf.b_mtime = file_info.stat.st_mtim.tv_sec as _;
        buf.b_mtime_ns = file_info.stat.st_mtim.tv_nsec as _;
        buf.b_orig_size = os_fileinfo_size(file_info);
        buf.b_orig_mode = file_info.stat.st_mode as c_int;
}

/// Adjust the line with missing eol, used for the next write.
pub fn write_lnum_adjust(offset: linenr_T) {
    unsafe {
        if (*g_st().curbuf).b_no_eol_lnum != 0 {
            (*g_st().curbuf).b_no_eol_lnum += offset;
        }
    }
}

/// Creates a directory for private use by this instance of Nvim.
fn vim_mktempdir() {
    unsafe {
        #[cfg(windows)]
        const TEMP_DIRS: [&CStr; 5] = [c"$TMPDIR", c"$TMP", c"$TEMP", c"$USERPROFILE", c""];
        #[cfg(not(windows))]
        const TEMP_DIRS: [&CStr; 4] = [c"$TMPDIR", c"/tmp", c".", c"~"];

        let mut tmp: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let mut path: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let mut user: [c_char; 40] = [0; 40];

        os_get_username(user.as_mut_ptr(), user.len());
        { let _d = (user.as_mut_ptr()).cast::<u8>(); let _sz = user.len(); if _sz > 0 { let _c = (b'/' as c_char) as u8; let _x = (b'_' as c_char) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
        { let _d = (user.as_mut_ptr()).cast::<u8>(); let _sz = user.len(); if _sz > 0 { let _c = (b'\\' as c_char) as u8; let _x = (b'_' as c_char) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };

        let umask_save = call_umask(0o077);
        for temp_dir in TEMP_DIRS {
            let mut tmplen = expand_env(
                temp_dir.as_ptr().cast_mut(),
                tmp.as_mut_ptr(),
                (TEMP_FILE_PATH_MAXLEN - 64) as c_int,
            );
            if !os_isdir(tmp.as_ptr()) {
                if crate::memory::cstr_as_opt_bytes(temp_dir.as_ptr()) == Some(b"$TMPDIR") {
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
                tmplen -= std::ffi::CStr::from_ptr(user.as_ptr()).to_bytes().len();
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
pub fn readdir_core(
    entries: &mut Vec<std::ffi::CString>,
    path: *const c_char,
    context: *mut crate::eval::typval::typval_T,
) -> c_int {
    unsafe {
        entries.clear();

        let mut dir = Directory::new();
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
            if !ignore && !context.is_null() {
                let r = crate::eval::fs::readdir_checkitem(&mut *context, p);
                if r < 0 {
                    break;
                }
                if r == 0 {
                    ignore = true;
                }
            }

            if !ignore {
                entries.push(CStr::from_ptr(p).to_owned());
            }
        }

        os_closedir(&raw mut dir);

        entries.sort_unstable();

        OK
    }
}

/// Delete "name" and everything in it, recursively.
pub fn delete_recursive(name: *const c_char) -> c_int {
    unsafe {
        let mut result: c_int = 0;

        if os_isrealdir(name) {
            let exp = CStr::from_ptr(name).to_owned();
            let mut entries: Vec<std::ffi::CString> = Vec::new();
            if readdir_core(&mut entries, exp.as_ptr(), ptr::null_mut()) == OK {
                let namebuff_ptr = (&raw mut g_st().NameBuff).cast::<c_char>();
                let len = vim_snprintf!(namebuff_ptr, MAXPATHL, c"%s/".as_ptr(), exp.as_ptr());

                for entry in &entries {
                    vim_snprintf!(
                        namebuff_ptr.offset(len as isize),
                        MAXPATHL - (len as usize),
                        c"%s".as_ptr(),
                        entry.as_ptr(),
                    );
                    if delete_recursive(namebuff_ptr) != 0 {
                        result = -1;
                    }
                }
                drop(entries);
                if os_rmdir(exp.as_ptr()) != 0 {
                    result = -1;
                }
            } else {
                result = -1;
            }
        } else {
            result = if os_remove(name) == 0 { 0 } else { -1 };
        }

        result
    }
}

#[cfg(unix)]
fn vim_opentempdir() {
    unsafe {
        if !fio_st().VIM_TEMPDIR_DP.is_null() {
            return;
        }

        let Some(ref td) = fio_st().VIM_TEMPDIR else {
            return;
        };
        let dp = opendir(td.as_ptr());
        if dp.is_null() {
            return;
        }

        fio_st().VIM_TEMPDIR_DP = dp;
        flock(dirfd(fio_st().VIM_TEMPDIR_DP), LOCK_SH);
    }
}

#[cfg(unix)]
fn vim_closetempdir() {
    unsafe {
        if fio_st().VIM_TEMPDIR_DP.is_null() {
            return;
        }

        closedir(fio_st().VIM_TEMPDIR_DP);
        fio_st().VIM_TEMPDIR_DP = ptr::null_mut();
    }
}

/// Delete the temp directory and all files it contains.
pub fn vim_deltempdir() {
    unsafe {
        let Some(td) = fio_st().VIM_TEMPDIR.take() else {
            return;
        };

        #[cfg(unix)]
        vim_closetempdir();

        let mut td_vec: Vec<c_char> = td
            .into_bytes_with_nul()
            .into_iter()
            .map(|b| b as c_char)
            .collect();
        *path_tail(td_vec.as_mut_ptr()).sub(1) = NUL;
        delete_recursive(td_vec.as_ptr());
    }
}

/// Gets path to Nvim's own temp dir (ending with slash).
pub fn vim_gettempdir() -> *mut c_char {
    unsafe {
        let current_td = (&raw const fio_st().VIM_TEMPDIR)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        if current_td.is_null() || !os_isdir(current_td) {
            if !current_td.is_null() {
                fio_st().NOTFOUND += 1;
                if fio_st().NOTFOUND == 1 {
                    logmsg!(
                        LOG_ERR,
                        ptr::null(),
                        c"vim_gettempdir".as_ptr(),
                        0,
                        true,
                        c"tempdir disappeared (antivirus or broken cleanup job?): %s".as_ptr(),
                        current_td,
                    );
                }
                if fio_st().NOTFOUND > 1 {
                    msg_schedule_semsg!(
                        c"E5431: tempdir disappeared (%d times)".as_ptr(),
                        fio_st().NOTFOUND
                    );
                }
                fio_st().VIM_TEMPDIR = None;
            }
            vim_mktempdir();
        }
        (&raw const fio_st().VIM_TEMPDIR)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

/// Sets Nvim's own temporary directory name to `tempdir`.
fn vim_settempdir(tempdir: *mut c_char) -> bool {
    unsafe {
        let mut buf_vec: Vec<c_char> = vec![0; MAXPATHL + 2];
        let buf = buf_vec.as_mut_ptr();

        vim_FullName(tempdir, buf, MAXPATHL, cfg!(windows));

        let mut buflen = std::ffi::CStr::from_ptr(buf).to_bytes().len();
        if after_pathsep(buf, buf.add(buflen)) == 0 {
            let sep_bytes = PATHSEPSTR.to_bytes_with_nul();
            ptr::copy_nonoverlapping(
                sep_bytes.as_ptr().cast::<c_char>(),
                buf.add(buflen),
                sep_bytes.len(),
            );
            buflen += sep_bytes.len() - 1;
        }
        fio_st().VIM_TEMPDIR = Some(
            std::ffi::CString::new(std::slice::from_raw_parts(buf.cast::<u8>(), buflen))
                .unwrap_or_default(),
        );
        #[cfg(unix)]
        vim_opentempdir();
        true
    }
}

/// Return a unique name that can be used for a temp file.
pub fn vim_tempname() -> Option<std::ffi::CString> {
    static TEMP_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let tempdir = vim_gettempdir();
        if tempdir.is_null() {
            return None;
        }

        let mut templ: [c_char; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
        let count = TEMP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = vim_snprintf!(
            templ.as_mut_ptr(),
            TEMP_FILE_PATH_MAXLEN,
            c"%s%llu".as_ptr(),
            tempdir,
            count,
        );
        Some(crate::memory::cstr_to_cstring(templ.as_ptr()))
}

/// Tries matching a filename with a "pattern" ("prog" is None), or use the
/// precompiled regprog "prog" ("pattern" is NULL).
pub fn match_file_pat(
    pattern: *mut c_char,
    prog: Option<&regprog_T>,
    fname: *mut c_char,
    sfname: *mut c_char,
    tail: *mut c_char,
    allow_dirs: c_int,
) -> bool {
    let mut regmatch: regmatch_T = regmatch_T::new();
        let mut result = false;

        regmatch.rm_ic = opt_vars().p_fic != 0;
        regmatch.regprog = if let Some(prog) = prog {
            ptr::from_ref(prog).cast_mut()
        } else {
            vim_regcomp(pattern, RE_MAGIC)
        };

        if !regmatch.regprog.is_null()
            && ((allow_dirs != 0
                && (vim_regexec(&mut regmatch, fname, 0)
                    || (!sfname.is_null() && vim_regexec(&mut regmatch, sfname, 0))))
                || (allow_dirs == 0 && vim_regexec(&mut regmatch, tail, 0)))
        {
            result = true;
        }

        if prog.is_none() {
            vim_regfree(regmatch.regprog);
        }
        result
}

/// Check if a file matches with a pattern in "list".
pub fn match_file_list(list: *mut c_char, sfname: *mut c_char, ffname: *mut c_char) -> bool {
    unsafe {
        let tail = path_tail(sfname);

        let mut p = list;
        while *p != NUL {
            let mut buf: [c_char; MAXPATHL] = [0; MAXPATHL];
            copy_option_part(&raw mut p, buf.as_mut_ptr(), buf.len(), c",".as_ptr());
            let mut allow_dirs: c_char = 0;
            let Some(regpat) =
                file_pat_to_reg_pat(buf.as_ptr(), ptr::null(), &raw mut allow_dirs, 0)
            else {
                break;
            };
            let matched = match_file_pat(
                regpat.as_ptr().cast_mut(),
                None,
                ffname,
                sfname,
                tail,
                c_int::from(allow_dirs),
            );
            if matched {
                return true;
            }
        }
        false
    }
}

/// Convert the given pattern "pat" which has shell style wildcards in it, into
/// a regular expression, and return the result in allocated memory.
pub fn file_pat_to_reg_pat(
    mut pat: *const c_char,
    mut pat_end: *const c_char,
    allow_dirs: *mut c_char,
    no_bslash: c_int,
) -> Option<std::ffi::CString> {
    unsafe {
        if !allow_dirs.is_null() {
            *allow_dirs = 0;
        }

        if pat_end.is_null() {
            pat_end = pat.add(std::ffi::CStr::from_ptr(pat).to_bytes().len());
        }

        if pat_end == pat {
            return Some(c"^$".to_owned());
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
        let mut reg_pat_vec: Vec<c_char> = vec![0; size + 1];
        let reg_pat = reg_pat_vec.as_mut_ptr();

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
            if nested < 0 {
                emsg(tr(c"E219: Missing {.".as_ptr()));
            } else {
                emsg(tr(c"E220: Missing }.".as_ptr()));
            }
            return None;
        }
        Some(CStr::from_ptr(reg_pat).to_owned())
    }
}

/// Version of read() that retries when interrupted by EINTR.
pub fn read_eintr(fd: c_int, buf: *mut c_void, bufsize: usize) -> isize {
    unsafe {
        loop {
            #[cfg(windows)]
            let ret = _read(fd, buf, bufsize as c_uint) as isize;
            #[cfg(not(windows))]
            let ret = read(fd, buf, bufsize);
            if ret >= 0 || os_errno() != EINTR {
                return ret;
            }
        }
    }
}

/// Version of write() that retries when interrupted by EINTR.
pub fn write_eintr(fd: c_int, buf: *const c_void, bufsize: usize) -> isize {
    unsafe {
        let mut ret: isize = 0;

        while (ret as usize) < bufsize {
            let rem = bufsize - (ret as usize);
            #[cfg(windows)]
            let wlen = _write(fd, buf.cast::<u8>().offset(ret).cast(), rem as c_uint) as isize;
            #[cfg(not(windows))]
            let wlen = write(fd, buf.cast::<u8>().offset(ret).cast(), rem);
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
}

pub fn vim_fprintf_fmt(
    stream: *mut FILE,
    fmt: *const c_char,
    args: &[crate::strings::FmtVal],
) -> c_int {
    unsafe {
        if args.is_empty() && !CStr::from_ptr(fmt).to_bytes().contains(&b'%') {
            let bytes = CStr::from_ptr(fmt).to_bytes();
            if bytes.is_empty() {
                return 0;
            }
            return if fwrite(bytes.as_ptr().cast(), 1, bytes.len(), stream) == bytes.len() {
                bytes.len() as c_int
            } else {
                -1
            };
        }
        let mut stack_buf = [0 as c_char; 1024];
        let needed =
            crate::strings::vim_snprintf_fmt(stack_buf.as_mut_ptr(), stack_buf.len(), fmt, args);
        if needed < 0 {
            return -1;
        }
        let len = needed as usize;
        if len < stack_buf.len() {
            if len == 0 || fwrite(stack_buf.as_ptr().cast(), 1, len, stream) == len {
                needed
            } else {
                -1
            }
        } else {
            let mut heap_buf = vec![0 as c_char; len + 1];
            let actual =
                crate::strings::vim_snprintf_fmt(heap_buf.as_mut_ptr(), heap_buf.len(), fmt, args);
            if actual < 0 {
                return -1;
            }
            let alen = actual as usize;
            if alen == 0 || fwrite(heap_buf.as_ptr().cast(), 1, alen, stream) == alen {
                actual
            } else {
                -1
            }
        }
    }
}

pub fn vim_printf_fmt(fmt: *const c_char, args: &[crate::strings::FmtVal]) -> c_int {
    vim_fprintf_fmt(crate::log::log_get_stdout(), fmt, args)
}

#[macro_export]
macro_rules! fprintf {
    ($stream:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::fileio::vim_fprintf_fmt(
            $stream,
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
pub use fprintf;

#[macro_export]
macro_rules! printf {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::fileio::vim_printf_fmt(
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
pub use printf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dev_fd_file() {
        assert!(!is_dev_fd_file(c"/dev/fd/0".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/1".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/2".as_ptr().cast_mut()));
            assert!(is_dev_fd_file(c"/dev/fd/3".as_ptr().cast_mut()));
            assert!(is_dev_fd_file(c"/dev/fd/10".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/dev/fd/10a".as_ptr().cast_mut()));
            assert!(!is_dev_fd_file(c"/tmp/foo".as_ptr().cast_mut()));
    }

    #[test]
    fn test_check_for_bom() {
        unsafe {
            let mut len = 0;
            let utf8_bom = [0xefu8, 0xbb, 0xbf, b'a'];
            let enc = check_for_bom(utf8_bom.as_ptr().cast(), 4, &mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"utf-8");
            assert_eq!(len, 3);

            let utf16le_bom = [0xffu8, 0xfe, b'a', 0];
            let enc = check_for_bom(utf16le_bom.as_ptr().cast(), 4, &mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"utf-16le");
            assert_eq!(len, 2);

            let ucs4le_bom = [0xffu8, 0xfe, 0x00, 0x00];
            let enc = check_for_bom(ucs4le_bom.as_ptr().cast(), 4, &mut len, FIO_ALL);
            assert!(!enc.is_null());
            assert_eq!(CStr::from_ptr(enc), c"ucs-4le");
            assert_eq!(len, 4);
        }
    }

    #[test]
    fn test_file_pat_to_reg_pat() {
        let check = |pat: &CStr, expected: &CStr| {
                let res = file_pat_to_reg_pat(pat.as_ptr(), ptr::null(), ptr::null_mut(), 1);
                assert_eq!(res.as_deref(), Some(expected));
            };
            check(c"path", c"^path$");
            check(c"*path", c"path$");
            check(c"path*", c"^path");
            check(c"*path*", c"path");
            check(c"foo?bar", c"^foo.bar$");
            check(c"foo*bar", c"^foo.*bar$");
            check(c"foo\\?bar", c"^foo?bar$");
            check(c"foo\\%bar", c"^foo%bar$");
            check(c"foo\\,bar", c"^foo,bar$");
            check(c"foo\\ bar", c"^foo bar$");
            check(c"foo.bar", c"^foo\\.bar$");
            check(c"foo{bar,baz}", c"^foo\\(bar\\|baz\\)$");
            check(c"foo*******bar", c"^foo.*bar$");
            check(c"********foobar", c"foobar$");
            check(c"foobar********", c"^foobar");
            check(c"^blah", c"^^blah$");
            check(c"foo^bar", c"^foo^bar$");
            check(c"blah$", c"^blah$$");
            check(c"foo$bar", c"^foo$bar$");
    }

    #[test]
    fn test_tempdir_and_tempname() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            if *crate::eval::vars::get_vim_var_str(crate::eval::vars::VV_PROGPATH) == NUL {
                crate::eval::vars::set_vim_var_string(
                    crate::eval::vars::VV_PROGPATH,
                    c"nvim".as_ptr(),
                    -1,
                );
            }
            vim_deltempdir();
            let dir1_ptr = vim_gettempdir();
            assert!(!dir1_ptr.is_null());
            let dir1 = CStr::from_ptr(dir1_ptr).to_str().unwrap().to_string();
            assert!(!dir1.is_empty());
            let dir2 = CStr::from_ptr(vim_gettempdir())
                .to_str()
                .unwrap()
                .to_string();
            assert_eq!(dir1, dir2);

            let f1 = vim_tempname().unwrap();
            let f2 = vim_tempname().unwrap();
            let f1_str = f1.to_str().unwrap().to_string();
            let f2_str = f2.to_str().unwrap().to_string();
            assert_ne!(f1_str, f2_str);
            assert!(f1_str.starts_with(&dir1));
            vim_deltempdir();
        }
    }
}
