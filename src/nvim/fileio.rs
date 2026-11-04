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
    BF_CHECK_RO, BF_NEW, BF_NOTEDITED, BLN_DUMMY, Bufref, FileBuf, K_SHM_FILEINFO, K_SHM_RO,
    K_UIMESSAGES, MAXLNUM, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC, NOTDONE, SEA_QUIT, bt_dontwrite,
    bt_nofilename, bt_normal, buf_contents_changed, buf_is_empty, buflist_new, bufref_valid,
    do_modelines, set_bufref, setfname, wipe_buffer,
};
use crate::buffer_updates::buf_updates_unload;
use crate::change::{HLF_W, appended_lines_mark, save_file_ff, unchanged};
#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "macos",
    test
))]
use crate::charset::ascii_isdigit;
#[cfg(windows)]
use crate::charset::vim_isfilec;
use crate::charset::{EOL_MAC, ascii_isspace};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor, check_cursor_lnum};
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
use crate::ex_docmd::{CAR, CMOD_LOCKMARKS, Exarg};
use crate::ex_eval::aborting;
use crate::fold::{fold_update_all, foldmethod_is_manual};
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
use crate::memory::cstr_to_cstring;
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
#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "macos"
))]
use crate::os::fs::s_ischr;
use crate::os::fs::{
    Directory, FileInfo, TEMP_FILE_PATH_MAXLEN, UV_ENOENT, VimAcl, os_close, os_closedir, os_copy,
    os_dirname, os_errno, os_file_is_writable, os_file_owned, os_fileinfo, os_fileinfo_fd,
    os_fileinfo_id_equal, os_fileinfo_size, os_free_acl, os_get_acl, os_getperm, os_isdir,
    os_isrealdir, os_mkdir, os_mkdtemp, os_open, os_path_exists, os_remove, os_rename, os_rmdir,
    os_scandir, os_scandir_next, os_set_acl, os_set_cloexec, os_set_errno,
};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo_link, os_setperm, s_islnk};
use crate::os::input::os_breakcheck;
use crate::os::time::{os_hrtime, time_t};
use crate::os::users::os_get_username;
use crate::path::{
    K_PATH_CMP_LITERAL, add_pathsep, after_pathsep, dir_of_file_exists, path_equal,
    path_is_absolute, path_shorten_fname, path_tail, path_with_url, vim_full_name, vim_ispathsep,
};
use crate::regexp::{RE_MAGIC, Regmatch, Regprog, vim_regcomp, vim_regexec};
use crate::sha256::ContextSha256;
use crate::shada::check_marks_read;
use crate::state::{MODE_CMDLINE, MODE_NORMAL_BUSY};
use crate::strings::{ngettext, tr, vim_snprintf};
use crate::ui::{ui_flush, ui_has};
use crate::undo::{
    buf_is_changed_fn, u_clearallandblockfree, u_clearline, u_compute_hash, u_find_first_changed,
    u_read_undo, u_savecommon, u_sync, u_unchanged, u_write_undo,
};
use std::ffi::{CStr, c_void};
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::os_open_conin_fd;

pub type Off = i64;
#[allow(non_camel_case_types)]
pub type iconv_t = isize;
pub type FILE = c_void;

pub const READ_NEW: i32 = 0x01;
pub const READ_FILTER: i32 = 0x02;
pub const READ_STDIN: i32 = 0x04;
pub const READ_BUFFER: i32 = 0x08;
pub const READ_DUMMY: i32 = 0x10;
pub const READ_KEEP_UNDO: i32 = 0x20;
pub const READ_FIFO: i32 = 0x40;
pub const READ_NOWINENTER: i32 = 0x80;
pub const READ_NOFILE: i32 = 0x100;

pub const FIO_LATIN1: i32 = 0x01;
pub const FIO_UTF8: i32 = 0x02;
pub const FIO_UCS2: i32 = 0x04;
pub const FIO_UCS4: i32 = 0x08;
pub const FIO_UTF16: i32 = 0x10;
pub const FIO_ENDIAN_L: i32 = 0x80;
pub const FIO_NOCONVERT: i32 = 0x2000;
pub const FIO_UCSBOM: i32 = 0x4000;
pub const FIO_ALL: i32 = -1;

pub const CONV_RESTLEN: usize = 30;
pub const WRITEBUFSIZE: usize = 8192;
pub const ICONV_MULT: isize = 8;

const CTRL_Z: u8 = 0x1a;

pub(crate) const EOF: i32 = -1;

const MAXCOL: isize = 0x7fffffff;

pub(crate) const EOL_UNKNOWN: i32 = -1;
pub(crate) const EOL_UNIX: i32 = 0;
pub(crate) const EOL_DOS: i32 = 1;

const BAD_REPLACE: i32 = b'?' as i32;

pub(crate) const K_OPT_FILEENCODING: i32 = 92;

pub(crate) const ENC_UNICODE: i32 = 0x04;
pub(crate) const ENC_ENDIAN_L: i32 = 0x20;
pub(crate) const ENC_2BYTE: i32 = 0x40;
pub(crate) const ENC_4BYTE: i32 = 0x80;
pub(crate) const ENC_2WORD: i32 = 0x100;
pub(crate) const ENC_LATIN1: i32 = 0x200;

const BF_NEW_W: i32 = 0x20;

const K_CPO_FNAMER: i32 = b'f' as i32;
pub(crate) const K_SHM_OVER: i32 = b'o' as i32;
const K_SHM_LINES: i32 = b'l' as i32;

pub(crate) const O_RDONLY: i32 = 0;
pub(crate) const EINTR: i32 = 4;
pub(crate) const ICONV_EINVAL: i32 = 22;
const UV_FS_COPYFILE_EXCL: i32 = 0x0001;

pub(crate) const S_IFMT: u64 = 0o170000;
pub(crate) const S_IFIFO: u64 = 0o010000;
pub(crate) const S_IFDIR: u64 = 0o040000;
pub(crate) const S_IFREG: u64 = 0o100000;
pub(crate) const S_IFSOCK: u64 = 0o140000;

const LOG_DEBUG: i32 = 1;
const LOG_WARN: i32 = 3;
const LOG_ERR: i32 = 4;

const NONASCII_MASK: u64 = (u64::MAX / 0xFF) * 0x80;

#[cfg(windows)]
const UV_EFBIG: i32 = -4036;
#[cfg(not(windows))]
const UV_EFBIG: i32 = -27;

#[cfg(target_os = "linux")]
const EOVERFLOW: i32 = 75;
#[cfg(all(unix, not(target_os = "linux")))]
const EOVERFLOW: i32 = 84;

#[cfg(windows)]
const BASENAMELEN: usize = 260 - 5;
#[cfg(not(windows))]
const BASENAMELEN: usize = 255 - 5;

use crate::os::fs::PATHSEPSTR;
const BACKSLASH_IN_FILENAME_BOOL: bool = cfg!(windows);

#[cfg(unix)]
const LOCK_SH: i32 = 1;
pub(crate) const SEEK_SET: i32 = 0;
pub(crate) const SEEK_END: i32 = 2;

static E_AUCHANGEDBUF: &CStr = c"E812: Autocommands changed buffer or buffer name";
static MSG_IS_A_DIRECTORY: &CStr = c"is a directory";

#[cfg(unix)]
type DIR = c_void;

struct FileioModuleState {
    already_warned: bool,
    vim_tempdir: Option<std::ffi::CString>,
    #[cfg(unix)]
    vim_tempdir_dp: *mut DIR,
    notfound: i32,
}

impl FileioModuleState {
    const fn new() -> Self {
        Self {
            already_warned: false,
            vim_tempdir: None,
            #[cfg(unix)]
            vim_tempdir_dp: ptr::null_mut(),
            notfound: 0,
        }
    }
}

static FIO_STATE: GlobalState<FileioModuleState> = GlobalState::new(FileioModuleState::new());

#[inline]
fn fio_st() -> &'static mut FileioModuleState {
    FIO_STATE.as_mut()
}

use crate::arglist::ML_EMPTY;
use crate::eval::funcs::VIM_WARNING;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_cmds::{BL_FIX, BL_WHITE, K_SHM_OVERALL};
use crate::ex_docmd::UNDO_HASH_SIZE;
use crate::math::{FAIL, OK};
use crate::option::OPT_LOCAL;
use crate::os::fs::MAXPATHL;

unsafe extern "C" {
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
    pub fn tcdrain(fd: i32) -> i32;
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
    pub fn close(fd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_dup")]
    pub fn dup(oldfd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_dup2")]
    pub fn dup2(oldfd: i32, newfd: i32) -> i32;
    #[cfg_attr(windows, link_name = "_lseeki64")]
    #[cfg_attr(not(windows), link_name = "lseek")]
    pub fn vim_lseek(fd: i32, offset: Off, whence: i32) -> Off;
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
    pub fn _setmode(fd: i32, mode: i32) -> i32;
    #[cfg(windows)]
    pub fn _getdrive() -> i32;
    #[cfg(windows)]
    pub fn _read(fd: i32, buf: *mut c_void, count: u32) -> i32;
    #[cfg(windows)]
    pub fn _write(fd: i32, buf: *const c_void, count: u32) -> i32;
    #[cfg(windows)]
    fn _umask(pmode: i32) -> i32;

    #[cfg(not(windows))]
    pub fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    #[cfg(not(windows))]
    pub fn getuid() -> u32;
    #[cfg(not(windows))]
    pub fn getgid() -> u32;
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
    fn opendir(name: *const u8) -> *mut DIR;
    #[cfg(not(windows))]
    fn closedir(dirp: *mut DIR) -> i32;
    #[cfg(not(windows))]
    fn dirfd(dirp: *mut DIR) -> i32;
    #[cfg(not(windows))]
    fn flock(fd: i32, operation: i32) -> i32;
}

#[inline]
pub fn freopen(filename: &CStr, mode: &CStr, stream: &crate::os::fs::CFile) -> bool {
    !unsafe {
        c_freopen(
            filename.as_ptr().cast(),
            mode.as_ptr().cast(),
            stream.as_ptr().cast(),
        )
    }
    .is_null()
}

#[cfg(not(windows))]
#[inline]
pub fn call_iconv_open(tocode: &CStr, fromcode: &CStr) -> iconv_t {
    unsafe { c_iconv_open(tocode.as_ptr().cast(), fromcode.as_ptr().cast()) as iconv_t }
}

use crate::os::fs::{s_isdir, s_isfifo, s_isreg, s_issock};

#[cfg(windows)]
mod win_iconv {
    use super::*;
    use crate::os::fs::errno_ptr;

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
            unsafe { *errno_ptr() = EINVAL };
            return -1;
        };
        let Ok(from_str) = fromcode.to_str() else {
            unsafe { *errno_ptr() = EINVAL };
            return -1;
        };
        let (Some(to), Some(from)) = (parse_win_enc(to_str), parse_win_enc(from_str)) else {
            unsafe { *errno_ptr() = EINVAL };
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
            unsafe { *errno_ptr() = EINVAL };
            return usize::MAX;
        }
        let desc = unsafe { &mut *(cd as *mut WinIconvDesc) };
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
                        unsafe { *errno_ptr() = EILSEQ };
                        return usize::MAX;
                    };
                    if enc_len > outbuf.len() {
                        unsafe { *errno_ptr() = E2BIG };
                        return usize::MAX;
                    }
                    outbuf[..enc_len].copy_from_slice(&encoded[..enc_len]);
                    *outbuf = &mut std::mem::take(outbuf)[enc_len..];
                    *inbuf = &inbuf[consumed..];
                }
                StepResult::Incomplete => {
                    unsafe { *errno_ptr() = EINVAL };
                    return usize::MAX;
                }
                StepResult::Invalid => {
                    unsafe { *errno_ptr() = EILSEQ };
                    return usize::MAX;
                }
            }
        }

        0
    }

    pub fn close(cd: iconv_t) -> i32 {
        if cd != -1 && cd != 0 {
            let _ = unsafe { Box::from_raw(cd as *mut WinIconvDesc) };
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
        let mut out_ptr: *mut u8 = outbuf.as_mut_ptr();
        let mut out_left: usize = outbuf.len();
        let orig_out_len = out_left;
        let ret = if let Some(inbuf) = inbuf {
            let mut in_ptr: *const u8 = inbuf.as_ptr();
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
fn call_umask(mask: u32) -> u32 {
    #[cfg(windows)]
    unsafe {
        _umask(mask as i32) as u32
    }
    #[cfg(not(windows))]
    unsafe {
        umask(mask)
    }
}

/// Shows a message about file `name`, e.g. `"foo.txt" [New]`.
fn filemess(buf: Option<&FileBuf>, name: Option<&CStr>, s: &CStr, msg_id: Option<&CStr>) {
    let prev_msg_col = g_st().msg_col;

    if g_st().msg_silent != 0 {
        return;
    }

    let iobuf_ptr = g_st().iobuff.as_mut_ptr();
    add_quoted_fname(&mut g_st().iobuff_bytes_mut()[..IOSIZE - 100], buf, name);

    crate::memory::append_bytes_to_c_buf(g_st().iobuff_bytes_mut(), s.to_bytes());

    let msg_scroll_save = g_st().msg_scroll;
    if shortmess(K_SHM_OVERALL)
        && g_st().msg_listdo_overwrite == 0
        && !g_st().exiting
        && opt_vars().p_verbose == 0
    {
        g_st().msg_scroll = 0;
    }
    if g_st().msg_scroll == 0 {
        msg_check_for_delay(false);
    }
    msg_start();
    if prev_msg_col != 0 && g_st().msg_col == 0 {
        msg_putchar(b'\r' as i32);
    }
    g_st().msg_scroll = msg_scroll_save;
    g_st().msg_scrolled_ign = true;
    if let Some(msg_id) = msg_id {
        msg_with_id(
            iobuf_ptr,
            msg_id.as_ptr().cast_mut(),
            c"bufwrite",
            None,
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

/// Shows file `name` before writing it, as a "bufwrite" message with `msg_id`, which buf_write()
/// replaces when done (or with a progress-message, if slow).
pub fn filemess_bufwrite(buf: Option<&FileBuf>, name: Option<&CStr>, msg_id: Option<&CStr>) {
    filemess(buf, name, c"", msg_id);
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
    s: Option<&CStr>,
    msg_id: &CStr,
    status: &CStr,
    percent: i32,
    hist: bool,
) -> Option<&'static CStr> {
    let buf = g_st().curbuf;

    unsafe {
        (*buf).b_locked += 1;
    }

    let rv = msg_progress(
        s.map_or(ptr::null_mut(), |cs| cs.as_ptr().cast_mut()),
        msg_id.as_ptr().cast_mut(),
        status.as_ptr().cast_mut(),
        percent,
        0,
        hist,
        s.is_some(),
        false,
    );

    unsafe {
        (*buf).b_locked -= 1;
    }

    (!rv.is_null()).then(|| unsafe { CStr::from_ptr(rv) })
}

/// Read lines from file "fname" into the buffer after line "from".
pub fn readfile(
    fname_opt: Option<&CStr>,
    sfname_opt: Option<&CStr>,
    from: Linenr,
    lines_to_skip: Linenr,
    lines_to_read: Linenr,
    eap: Option<&Exarg>,
    flags: i32,
    silent: bool,
) -> i32 {
    let fname: *mut u8 = fname_opt.map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
    let mut sfname: *mut u8 = sfname_opt.map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());
    #[cfg(unix)]
    let mut fname = fname;
    {
        let mut retval = FAIL;
        let mut fd: i32 = if g_st().stdin_fd >= 0 {
            g_st().stdin_fd
        } else {
            0
        };
        let newfile = (flags & READ_NEW) != 0;
        let filtering = (flags & READ_FILTER) != 0;
        let read_stdin = (flags & READ_STDIN) != 0;
        let read_buffer = (flags & READ_BUFFER) != 0;
        let read_fifo = (flags & READ_FIFO) != 0;
        let set_options = newfile || read_buffer || (eap.is_some() && eap.unwrap().read_edit != 0);
        let mut read_buf_lnum: Linenr = 1;
        let mut read_buf_col: Colnr = 0;
        let mut lnum = from;
        let mut ptr: *mut u8 = ptr::null_mut();
        let mut buffer_vec: Vec<u8> = Vec::new();
        let mut buffer: *mut u8 = ptr::null_mut();
        let mut line_start: *mut u8 = ptr::null_mut();
        let mut size: isize = 0;
        let mut p: *mut u8;
        let mut filesize: Off = 0;
        let mut skip_read = false;
        let mut sha_ctx = ContextSha256::new();
        let mut read_undo_file = false;
        let mut split: i32 = 0;
        let mut error = false;
        let mut ff_error: i32 = EOL_UNKNOWN;
        let mut linerest: isize = 0;
        let mut perm: i32 = 0;
        #[cfg(unix)]
        let mut swap_mode: i32 = -1;
        let mut fileformat: i32 = 0;
        let mut keep_fileformat = false;
        let mut file_info: FileInfo = FileInfo::new();
        let mut skip_count: Linenr = 0;
        let mut read_count: Linenr = 0;
        let msg_save = g_st().msg_scroll;
        let mut read_no_eol_lnum: Linenr = 0;
        let mut file_rewind = false;
        let mut conv_error: Linenr = 0;
        let mut illegal_byte: Linenr = 0;
        let mut keep_dest_enc = false;
        let mut bad_char_behavior: i32 = BAD_REPLACE;
        let mut tmpname: Option<std::ffi::CString> = None;
        let mut fio_flags: i32;
        let mut fenc: *mut u8;
        let mut fenc_owned: Option<std::ffi::CString> = None;
        let mut advance_fenc = false;
        let mut real_size: i32 = 0;
        let mut iconv_fd: iconv_t = -1;
        let mut did_iconv = false;
        let mut converted = false;
        let mut notconverted = false;
        let mut conv_rest: [u8; CONV_RESTLEN] = [0; CONV_RESTLEN];
        let mut conv_restlen: i32 = 0;
        let mut orig_start: CursorPos;

        curbuf_mut().b_au_did_filetype = false;
        curbuf_mut().b_no_eol_lnum = 0;

        'theend: {
            if curbuf_mut().b_ffname.is_null()
                && !filtering
                && !fname.is_null()
                && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_FNAMER)
                && (flags & READ_DUMMY) == 0
                && !set_rw_fname(
                    crate::memory::cstr_as_opt_cstr(fname.cast()),
                    crate::memory::cstr_as_opt_cstr(sfname.cast()),
                )
            {
                break 'theend;
            }

            let old_curbuf = g_st().curbuf;
            let old_b_ffname = curbuf_mut().b_ffname;
            let old_b_fname = curbuf_mut().b_fname;
            let using_b_ffname =
                fname == curbuf_mut().b_ffname.cast() || sfname == curbuf_mut().b_ffname.cast();
            let using_b_fname =
                fname == curbuf_mut().b_fname.cast() || sfname == curbuf_mut().b_fname.cast();

            g_st().need_fileinfo = false;

            if sfname.is_null() {
                sfname = fname;
            }
            #[cfg(unix)]
            {
                fname = sfname;
            }

            if !filtering && !read_stdin && !read_buffer {
                orig_start = curbuf_mut().b_op_start;

                curbuf_mut().b_op_start.lnum = if from == 0 { 1 } else { from };
                curbuf_mut().b_op_start.col = 0;

                let sfname_opt = crate::memory::cstr_as_opt_cstr(sfname.cast());
                if newfile {
                    if apply_autocmds_exarg(
                        EVENT_BUFREADCMD,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    ) {
                        retval = OK;
                        if aborting() {
                            retval = FAIL;
                        }

                        if retval == OK
                            && !curbuf_mut().b_au_did_filetype
                            && !crate::memory::cstr_is_empty(curbuf_mut().b_p_ft)
                        {
                            apply_autocmds(
                                EVENT_FILETYPE,
                                crate::memory::cstr_as_opt_cstr(curbuf_mut().b_p_ft),
                                crate::memory::cstr_as_opt_cstr(curbuf_mut().b_fname),
                                true,
                                curbuf_opt_mut(),
                            );
                            if aborting() {
                                retval = FAIL;
                            }
                        }
                        if retval == OK {
                            curbuf_mut().b_flags &= !BF_NOTEDITED;
                        }
                        break 'theend;
                    }
                } else if apply_autocmds_exarg(
                    EVENT_FILEREADCMD,
                    sfname_opt,
                    sfname_opt,
                    false,
                    None,
                    eap,
                ) {
                    retval = if aborting() { FAIL } else { OK };
                    break 'theend;
                }

                curbuf_mut().b_op_start = orig_start;

                if (flags & READ_NOFILE) != 0 {
                    retval = NOTDONE;
                    break 'theend;
                }
            }

            if ((shortmess(K_SHM_OVER) && g_st().msg_listdo_overwrite == 0) || curbuf_mut().b_help)
                && opt_vars().p_verbose == 0
            {
                g_st().msg_scroll = 0;
            } else {
                g_st().msg_scroll = 1;
            }

            if !crate::memory::cstr_is_empty(fname.cast()) {
                let fnamelen = crate::memory::cstr_len(fname.cast());

                if fnamelen >= MAXPATHL {
                    filemess(
                        unsafe { g_st().curbuf.as_ref() },
                        crate::memory::cstr_as_opt_cstr(fname.cast()),
                        tr(c"Illegal file name"),
                        None,
                    );
                    msg_end();
                    g_st().msg_scroll = msg_save;
                    break 'theend;
                }

                if after_pathsep(fname.cast(), fname.wrapping_add(fnamelen).cast()) != 0 {
                    if !silent {
                        filemess(
                            unsafe { g_st().curbuf.as_ref() },
                            crate::memory::cstr_as_opt_cstr(fname.cast()),
                            tr(MSG_IS_A_DIRECTORY),
                            None,
                        );
                    }
                    msg_end();
                    g_st().msg_scroll = msg_save;
                    retval = NOTDONE;
                    break 'theend;
                }
            }

            if !read_stdin && !fname.is_null() {
                perm = os_getperm(fname.cast());
            }

            #[cfg(any(
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly",
                target_os = "macos"
            ))]
            let is_chr_dev =
                s_ischr(perm as u64) && is_dev_fd_file(crate::memory::cstr_as_bytes(fname.cast()));
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
                        filemess(
                            unsafe { g_st().curbuf.as_ref() },
                            crate::memory::cstr_as_opt_cstr(fname.cast()),
                            tr(MSG_IS_A_DIRECTORY),
                            None,
                        );
                    }
                    retval = NOTDONE;
                } else {
                    filemess(
                        unsafe { g_st().curbuf.as_ref() },
                        crate::memory::cstr_as_opt_cstr(fname.cast()),
                        tr(c"is not a file"),
                        None,
                    );
                }
                msg_end();
                g_st().msg_scroll = msg_save;
                break 'theend;
            }

            set_file_options(set_options, (eap.is_some()).then(|| eap.unwrap()));

            let check_readonly = newfile && (curbuf_mut().b_flags & BF_CHECK_RO) != 0;
            if check_readonly && !g_st().readonlymode {
                curbuf_mut().b_p_ro = 0;
            }

            if newfile && !read_stdin && !read_buffer && !read_fifo {
                if os_fileinfo(fname.cast(), &mut file_info) {
                    buf_store_file_info(curbuf_mut(), &mut file_info);
                    curbuf_mut().b_mtime_read = curbuf_mut().b_mtime;
                    curbuf_mut().b_mtime_read_ns = curbuf_mut().b_mtime_ns;
                    #[cfg(unix)]
                    {
                        swap_mode = ((file_info.stat.st_mode as i32) & 0o644) | 0o600;
                    }
                } else {
                    curbuf_mut().b_mtime = 0;
                    curbuf_mut().b_mtime_ns = 0;
                    curbuf_mut().b_mtime_read = 0;
                    curbuf_mut().b_mtime_read_ns = 0;
                    curbuf_mut().b_orig_size = 0;
                    curbuf_mut().b_orig_mode = 0;
                }

                curbuf_mut().b_flags &= !(BF_NEW | BF_NEW_W);
            }

            let mut file_readonly = false;
            if !read_buffer && !read_stdin {
                if !newfile
                    || g_st().readonlymode
                    || (perm & 0o222) == 0
                    || os_file_is_writable(fname.cast()) == 0
                {
                    file_readonly = true;
                }
                fd = os_open(fname.cast(), O_RDONLY, 0);
            }

            if fd < 0 {
                g_st().msg_scroll = msg_save;
                if !newfile {
                    break 'theend;
                }
                if perm == UV_ENOENT {
                    curbuf_mut().b_flags |= BF_NEW;

                    if !bt_dontwrite(Some(crate::globals::curbuf_mut())) {
                        check_need_swap(newfile);
                        if g_st().curbuf != old_curbuf
                            || (using_b_ffname && old_b_ffname != curbuf_mut().b_ffname)
                            || (using_b_fname && old_b_fname != curbuf_mut().b_fname)
                        {
                            emsg(tr(E_AUCHANGEDBUF));
                            break 'theend;
                        }
                    }
                    if !silent {
                        if dir_of_file_exists(fname.cast()) {
                            filemess(
                                unsafe { g_st().curbuf.as_ref() },
                                crate::memory::cstr_as_opt_cstr(sfname.cast()),
                                tr(c"[New]"),
                                None,
                            );
                        } else {
                            filemess(
                                unsafe { g_st().curbuf.as_ref() },
                                crate::memory::cstr_as_opt_cstr(sfname.cast()),
                                tr(c"[New DIRECTORY]"),
                                None,
                            );
                        }
                    }
                    check_marks_read();
                    if let Some(eap) = eap {
                        set_forced_fenc(eap);
                    }
                    let sfname_opt = crate::memory::cstr_as_opt_cstr(sfname.cast());
                    apply_autocmds_exarg(
                        EVENT_BUFNEWFILE,
                        sfname_opt,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );
                    save_file_ff(curbuf_mut());

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
                    unsafe { g_st().curbuf.as_ref() },
                    crate::memory::cstr_as_opt_cstr(sfname.cast()),
                    if is_too_big {
                        tr(c"[File too big]")
                    } else {
                        tr(c"[Permission Denied]")
                    },
                    None,
                );
                curbuf_mut().b_p_ro = 1;

                break 'theend;
            }

            if (check_readonly && file_readonly) || curbuf_mut().b_help {
                curbuf_mut().b_p_ro = 1;
            }

            if set_options {
                if !read_buffer {
                    curbuf_mut().b_p_eof = 0;
                    curbuf_mut().b_start_eof = 0;
                    curbuf_mut().b_p_eol = 1;
                    curbuf_mut().b_start_eol = 1;
                }
                curbuf_mut().b_p_bomb = 0;
                curbuf_mut().b_start_bomb = 0;
            }

            if !bt_dontwrite(Some(crate::globals::curbuf_mut())) {
                check_need_swap(newfile);
                if !read_stdin
                    && (g_st().curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != curbuf_mut().b_ffname)
                        || (using_b_fname && old_b_fname != curbuf_mut().b_fname))
                {
                    emsg(tr(E_AUCHANGEDBUF));
                    if !read_buffer {
                        unsafe { close(fd) };
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                {
                    let mfp = curbuf_mut().b_ml.mfp_ptr();
                    if swap_mode > 0
                        && !mfp.is_null()
                        && let Some(ref swap_fname) = *unsafe { &(*mfp).mf_fname }
                    {
                        let swap_fname = swap_fname.as_ptr();
                        if (swap_mode & 0o44) == 0o40 {
                            let mut swap_info: FileInfo = FileInfo::new();
                            if os_fileinfo(swap_fname, &mut swap_info)
                                && file_info.stat.st_gid != swap_info.stat.st_gid
                                && os_fchown(
                                    unsafe { (*mfp).mf_fd },
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
                    unsafe { close(fd) };
                }
                break 'theend;
            }

            g_st().no_wait_return += 1;

            orig_start = curbuf_mut().b_op_start;
            curbuf_mut().b_op_start.lnum = if from == 0 { 1 } else { from };
            curbuf_mut().b_op_start.col = 0;

            let mut try_mac = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'm' as i32));
            let mut try_dos = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'd' as i32));
            let mut try_unix = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'x' as i32));

            if !read_buffer {
                let m = g_st().msg_scroll;
                let n = g_st().msg_scrolled;

                if !read_stdin {
                    unsafe { close(fd) };
                }

                g_st().msg_scroll = 1;
                let sfname_opt = crate::memory::cstr_as_opt_cstr(sfname.cast());
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );
                } else if read_stdin {
                    apply_autocmds_exarg(
                        EVENT_STDINREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );
                } else if newfile {
                    apply_autocmds_exarg(
                        EVENT_BUFREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );
                } else {
                    apply_autocmds_exarg(
                        EVENT_FILEREADPRE,
                        sfname_opt,
                        sfname_opt,
                        false,
                        None,
                        eap,
                    );
                }

                try_mac = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'm' as i32));
                try_dos = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'd' as i32));
                try_unix = i32::from(crate::strings::vim_haschr(opt_vars().p_ffs, b'x' as i32));
                curbuf_mut().b_op_start = orig_start;

                if g_st().msg_scrolled == n {
                    g_st().msg_scroll = m;
                }

                if aborting() {
                    g_st().no_wait_return -= 1;
                    g_st().msg_scroll = msg_save;
                    curbuf_mut().b_p_ro = 1;
                    break 'theend;
                }

                if !read_stdin
                    && (g_st().curbuf != old_curbuf
                        || (using_b_ffname && old_b_ffname != curbuf_mut().b_ffname)
                        || (using_b_fname && old_b_fname != curbuf_mut().b_fname)
                        || {
                            fd = os_open(fname.cast(), O_RDONLY, 0);
                            fd < 0
                        })
                {
                    g_st().no_wait_return -= 1;
                    g_st().msg_scroll = msg_save;
                    if fd < 0 {
                        emsg(tr(c"E200: *ReadPre autocommands made the file unreadable"));
                    } else {
                        emsg(tr(
                            c"E201: *ReadPre autocommands must not change current buffer",
                        ));
                    }
                    curbuf_mut().b_p_ro = 1;
                    break 'theend;
                }
            }

            let wasempty = (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0;

            let show_msg = !g_st().recoverymode
                && !filtering
                && (flags & READ_DUMMY) == 0
                && !read_stdin
                && !read_buffer
                && g_st().msg_silent == 0;
            if show_msg && !silent {
                filemess(
                    unsafe { g_st().curbuf.as_ref() },
                    crate::memory::cstr_as_opt_cstr(sfname.cast()),
                    c"",
                    None,
                );
            }

            let mut msg_id = [0u8; MAXPATHL + 32];
            fname_msg_id(
                &mut msg_id,
                c"nvim.bufread",
                unsafe { g_st().curbuf.as_ref() },
                crate::memory::cstr_as_opt_cstr(sfname.cast()),
            );
            let mut progress = false;
            let mut percent: i32 = -1;
            let read_start = os_hrtime();

            g_st().msg_scroll = 0;

            let mut linecnt = curbuf_mut().b_ml.ml_line_count;

            if let Some(eap) = eap
                && eap.bad_char != 0
            {
                bad_char_behavior = eap.bad_char;
                if set_options {
                    curbuf_mut().b_bad_char = eap.bad_char;
                }
            } else {
                curbuf_mut().b_bad_char = 0;
            }

            let mut fenc_next: &[u8] = b"";
            if let Some(eap) = eap
                && eap.force_enc != 0
            {
                fenc_owned = Some(enc_canonize(eap.cmd.wrapping_add(eap.force_enc as usize)));
                fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut().cast();
                keep_dest_enc = true;
            } else if curbuf_mut().b_p_bin != 0 {
                fenc = c"".as_ptr().cast_mut().cast();
            } else if curbuf_mut().b_help {
                fenc_next = b"latin1";
                fenc = c"utf-8".as_ptr().cast_mut().cast();
            } else if crate::memory::cstr_is_empty(opt_vars().p_fencs) {
                fenc = curbuf_mut().b_p_fenc.cast();
            } else {
                fenc_next = crate::memory::cstr_as_bytes(opt_vars().p_fencs);
                fenc_owned = Some(next_fenc(&mut fenc_next));
                fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut().cast();
            }

            'retry: loop {
                if file_rewind {
                    if read_buffer {
                        read_buf_lnum = 1;
                        read_buf_col = 0;
                    } else if read_stdin || unsafe { vim_lseek(fd, 0, SEEK_SET) } != 0 {
                        error = true;
                        break 'retry;
                    }
                    while lnum > from {
                        ml_delete(lnum);
                        lnum -= 1;
                    }
                    file_rewind = false;
                    if set_options {
                        curbuf_mut().b_p_bomb = 0;
                        curbuf_mut().b_start_bomb = 0;
                    }
                    conv_error = 0;
                }

                if keep_fileformat {
                    keep_fileformat = false;
                } else if eap.is_some() && eap.unwrap().force_ff != 0 {
                    fileformat =
                        get_fileformat_force(curbuf_mut(), (eap.is_some()).then(|| eap.unwrap()));
                    try_unix = 0;
                    try_dos = 0;
                    try_mac = 0;
                } else if curbuf_mut().b_p_bin != 0 {
                    fileformat = EOL_UNIX;
                } else if crate::memory::cstr_is_empty(opt_vars().p_ffs) {
                    fileformat = get_fileformat(curbuf_mut());
                } else {
                    fileformat = EOL_UNKNOWN;
                }

                if iconv_fd != -1 {
                    call_iconv_close(iconv_fd);
                    iconv_fd = -1;
                }

                if advance_fenc {
                    advance_fenc = false;

                    if eap.is_some() && eap.unwrap().force_enc != 0 {
                        notconverted = true;
                        conv_error = 0;
                        fenc_owned = None;
                        fenc = c"".as_ptr().cast_mut().cast();
                    } else if !fenc_next.is_empty() {
                        fenc_owned = Some(next_fenc(&mut fenc_next));
                        fenc = fenc_owned.as_ref().unwrap().as_ptr().cast_mut().cast();
                    } else {
                        fenc_owned = None;
                        fenc = c"".as_ptr().cast_mut().cast();
                    }
                    if let Some(ref tn) = tmpname {
                        os_remove(tn.as_ptr());
                        tmpname = None;
                    }
                }

                fio_flags = 0;
                let fenc_opt = crate::memory::cstr_as_opt_cstr(fenc.cast());
                converted = need_conversion(fenc_opt);
                if converted {
                    if fenc_opt == Some(c"ucs-bom") {
                        fio_flags = FIO_UCSBOM;
                    } else {
                        fio_flags = get_fio_flags(fenc_opt);
                    }

                    if fio_flags == 0 && !did_iconv {
                        iconv_fd =
                            my_iconv_open(c"utf-8", fenc.cast()).map_or(-1, |fd| fd as iconv_t);
                    }

                    if fio_flags == 0
                        && !read_stdin
                        && !read_buffer
                        && crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv)
                            != K_CALLBACK_NONE_CONST
                        && !read_fifo
                        && iconv_fd == -1
                    {
                        did_iconv = false;
                        if tmpname.is_none() {
                            tmpname = readfile_charconvert(
                                crate::memory::cstr_from_ptr(fname.cast()),
                                crate::memory::cstr_from_ptr(fenc.cast()),
                                &mut fd,
                            );
                            if tmpname.is_none() {
                                advance_fenc = true;
                                if fd < 0 {
                                    emsg(tr(c"E202: Conversion made file unreadable!"));
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

                let can_retry = !crate::memory::cstr_is_empty(fenc.cast())
                    && !read_stdin
                    && !keep_dest_enc
                    && !read_fifo;

                if !skip_read {
                    linerest = 0;
                    filesize = 0;
                    skip_count = lines_to_skip;
                    read_count = lines_to_read;
                    conv_restlen = 0;
                    read_undo_file = newfile
                        && (flags & READ_KEEP_UNDO) == 0
                        && !curbuf_mut().b_ffname.is_null()
                        && curbuf_mut().b_p_udf != 0
                        && !filtering
                        && !read_fifo
                        && !read_stdin
                        && !read_buffer;
                    if read_undo_file {
                        sha_ctx = ContextSha256::new();
                    }
                }

                while !error && !g_st().got_int {
                    {
                        if !skip_read {
                            size = (0x10000 + linerest).min(0x100000);
                        }

                        if size < 0 || size + linerest + 1 < 0 || linerest >= MAXCOL - size {
                            split += 1;
                            unsafe { *ptr = b'\n' };
                            size = 1;
                        } else if !skip_read {
                            let mut new_buffer_vec: Vec<u8> =
                                vec![0; (size as usize) + (linerest as usize) + 1];
                            let new_buffer = new_buffer_vec.as_mut_ptr();
                            if linerest != 0 {
                                unsafe {
                                    ptr::copy(
                                        ptr.wrapping_offset(-linerest),
                                        new_buffer,
                                        linerest as usize,
                                    )
                                };
                            }
                            buffer_vec = new_buffer_vec;
                            buffer = buffer_vec.as_mut_ptr();
                            ptr = buffer.wrapping_offset(linerest);
                            line_start = buffer;

                            real_size = size as i32;
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
                                unsafe {
                                    ptr::copy(conv_rest.as_ptr(), ptr, conv_restlen as usize)
                                };
                                ptr = ptr.wrapping_offset(conv_restlen as isize);
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
                                            .wrapping_offset(read_buf_col as isize);
                                        let mut n =
                                            (ml_get_len(read_buf_lnum) - read_buf_col) as isize;
                                        if tlen + n + 1 > size {
                                            n = size - tlen;
                                            for ni in 0..n {
                                                let ch = unsafe { *p.wrapping_offset(ni) };
                                                unsafe {
                                                    *ptr.wrapping_offset(tlen) =
                                                        if ch == b'\n' { 0 } else { ch };
                                                }
                                                tlen += 1;
                                            }
                                            read_buf_col += n as Colnr;
                                            break;
                                        }

                                        for ni in 0..n {
                                            let ch = unsafe { *p.wrapping_offset(ni) };
                                            unsafe {
                                                *ptr.wrapping_offset(tlen) =
                                                    if ch == b'\n' { 0 } else { ch };
                                            }
                                            tlen += 1;
                                        }
                                        unsafe { *ptr.wrapping_offset(tlen) = b'\n' };
                                        tlen += 1;
                                        read_buf_col = 0;
                                        read_buf_lnum += 1;
                                        if read_buf_lnum > from {
                                            if curbuf_mut().b_p_eol == 0 {
                                                tlen -= 1;
                                            }
                                            size = tlen;
                                            break;
                                        }
                                    }
                                }
                            } else {
                                let read_size = size as usize;
                                let read_slice =
                                    unsafe { std::slice::from_raw_parts_mut(ptr, read_size) };
                                size = read_eintr(fd, read_slice);
                            }

                            if size <= 0 {
                                if size < 0 {
                                    error = true;
                                } else if conv_restlen > 0 {
                                    if fio_flags != 0 || iconv_fd != -1 {
                                        if can_retry {
                                            if crate::eval::typval::Callback::raw_cb_type(
                                                &opt_vars().p_ccv,
                                            ) != K_CALLBACK_NONE_CONST
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
                                                curbuf_mut().b_ml.ml_line_count - linecnt + 1;
                                        }
                                    } else if illegal_byte == 0 {
                                        illegal_byte =
                                            curbuf_mut().b_ml.ml_line_count - linecnt + 1;
                                    }
                                    if bad_char_behavior == BAD_DROP {
                                        unsafe {
                                            *ptr.wrapping_offset(-(conv_restlen as isize)) = 0;
                                        }
                                        conv_restlen = 0;
                                    } else {
                                        if bad_char_behavior != BAD_KEEP
                                            && (fio_flags != 0 || iconv_fd != -1)
                                        {
                                            while conv_restlen > 0 {
                                                ptr = ptr.wrapping_sub(1);
                                                unsafe { *ptr = bad_char_behavior as u8 };
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
                                || (curbuf_mut().b_p_bomb == 0
                                    && tmpname.is_none()
                                    && (crate::memory::cstr_first_byte(fenc.cast()) == b'u'
                                        || crate::memory::cstr_is_empty(fenc.cast()))))
                        {
                            let mut blen: i32 = 0;
                            let ccname = if size < 2 || curbuf_mut().b_p_bin != 0 {
                                None
                            } else {
                                check_for_bom(
                                    unsafe { std::slice::from_raw_parts(ptr, size as usize) },
                                    &mut blen,
                                    if fio_flags == FIO_UCSBOM {
                                        FIO_ALL
                                    } else {
                                        get_fio_flags(crate::memory::cstr_as_opt_cstr(fenc.cast()))
                                    },
                                )
                            };
                            if ccname.is_some() {
                                filesize += i64::from(blen);
                                size -= blen as isize;
                                unsafe {
                                    ptr::copy(
                                        ptr.wrapping_offset(blen as isize),
                                        ptr,
                                        size as usize,
                                    )
                                };
                                if set_options {
                                    curbuf_mut().b_p_bomb = 1;
                                    curbuf_mut().b_start_bomb = 1;
                                }
                            }

                            if fio_flags == FIO_UCSBOM {
                                if let Some(ccname) = ccname {
                                    fenc_owned = None;
                                    fenc = ccname.as_ptr().cast_mut().cast();
                                } else {
                                    advance_fenc = true;
                                }
                                skip_read = true;
                                continue 'retry;
                            }
                        }

                        ptr = ptr.wrapping_offset(-(conv_restlen as isize));
                        size += conv_restlen as isize;
                        conv_restlen = 0;
                        if size <= 0 {
                            break;
                        }

                        if iconv_fd != -1 {
                            let from_total = size as usize;
                            let to_total = (real_size as isize - size) as usize;
                            let (from_part, to_part) = unsafe {
                                std::slice::from_raw_parts_mut(ptr, from_total + to_total)
                                    .split_at_mut(from_total)
                            };
                            let mut from_rem: &[u8] = from_part;
                            let mut to_rem: &mut [u8] = to_part;
                            ptr = ptr.wrapping_offset(size);

                            while (call_iconv(iconv_fd, Some(&mut from_rem), &mut to_rem)
                                == usize::MAX
                                && os_errno() != ICONV_EINVAL)
                                || from_rem.len() > CONV_RESTLEN
                            {
                                if can_retry {
                                    if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv)
                                        != K_CALLBACK_NONE_CONST
                                        && iconv_fd != -1
                                    {
                                        did_iconv = true;
                                    } else {
                                        advance_fenc = true;
                                    }
                                    file_rewind = true;
                                    continue 'retry;
                                }
                                let out_written = to_total - to_rem.len();
                                if conv_error == 0 {
                                    conv_error = readfile_linenr(linecnt, unsafe {
                                        std::slice::from_raw_parts(ptr, out_written)
                                    });
                                }

                                let skipped_byte = from_rem[0];
                                from_rem = &from_rem[1..];
                                if bad_char_behavior == BAD_KEEP {
                                    to_rem[0] = skipped_byte;
                                    to_rem = &mut to_rem[1..];
                                } else if bad_char_behavior != BAD_DROP {
                                    to_rem[0] = bad_char_behavior as u8;
                                    to_rem = &mut to_rem[1..];
                                }
                            }

                            if !from_rem.is_empty() {
                                conv_rest[..from_rem.len()].copy_from_slice(from_rem);
                                conv_restlen = from_rem.len() as i32;
                            }

                            let out_written = to_total - to_rem.len();
                            line_start = ptr.wrapping_offset(-linerest);
                            unsafe { ptr::copy(buffer, line_start, linerest as usize) };
                            size = out_written as isize;
                        }

                        if fio_flags != 0 {
                            let mut u8c: u32;
                            let mut tail: *mut u8 = ptr::null_mut();

                            let mut dest = ptr.wrapping_offset(real_size as isize);
                            if fio_flags == FIO_LATIN1 || fio_flags == FIO_UTF8 {
                                p = ptr.wrapping_offset(size);
                                if fio_flags == FIO_UTF8 {
                                    tail = ptr.wrapping_offset(size - 1);
                                    while tail > ptr && (unsafe { *tail } & 0xc0) == 0x80 {
                                        tail = tail.wrapping_sub(1);
                                    }
                                    if tail
                                        .wrapping_offset(
                                            utf_byte2len(i32::from(unsafe { *tail })) as isize
                                        )
                                        <= ptr.wrapping_offset(size)
                                    {
                                        tail = ptr::null_mut();
                                    } else {
                                        p = tail;
                                    }
                                }
                            } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                p = ptr.wrapping_offset(size & !1);
                                if (size & 1) != 0 {
                                    tail = p;
                                }
                                if (fio_flags & FIO_UTF16) != 0 && p > ptr {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p }) << 8;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p });
                                    } else {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p });
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 8;
                                    }
                                    if (0xd800..=0xdbff).contains(&u8c) {
                                        tail = p;
                                    } else {
                                        p = p.wrapping_add(2);
                                    }
                                }
                            } else {
                                p = ptr.wrapping_offset(size & !3);
                                if (size & 3) != 0 {
                                    tail = p;
                                }
                            }

                            if !tail.is_null() {
                                conv_restlen =
                                    crate::memory::ptr_offset_from(ptr.wrapping_offset(size), tail)
                                        as i32;
                                unsafe {
                                    ptr::copy(tail, conv_rest.as_mut_ptr(), conv_restlen as usize)
                                };
                                size -= conv_restlen as isize;
                            }

                            while p > ptr {
                                if (fio_flags & FIO_LATIN1) != 0 {
                                    p = p.wrapping_sub(1);
                                    u8c = u32::from(unsafe { *p });
                                } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p }) << 8;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p });
                                    } else {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p });
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 8;
                                    }
                                    if (fio_flags & FIO_UTF16) != 0
                                        && (0xdc00..=0xdfff).contains(&u8c)
                                    {
                                        if p == ptr {
                                            if can_retry {
                                                if crate::eval::typval::Callback::raw_cb_type(
                                                    &opt_vars().p_ccv,
                                                ) != K_CALLBACK_NONE_CONST
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
                                                conv_error = readfile_linenr(linecnt, unsafe {
                                                    std::slice::from_raw_parts(
                                                        ptr,
                                                        crate::memory::ptr_offset_from(p, ptr)
                                                            .max(0)
                                                            as usize,
                                                    )
                                                });
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as u32;
                                            }
                                        }

                                        let mut u16c: i32;
                                        if (fio_flags & FIO_ENDIAN_L) != 0 {
                                            p = p.wrapping_sub(1);
                                            u16c = i32::from(unsafe { *p }) << 8;
                                            p = p.wrapping_sub(1);
                                            u16c += i32::from(unsafe { *p });
                                        } else {
                                            p = p.wrapping_sub(1);
                                            u16c = i32::from(unsafe { *p });
                                            p = p.wrapping_sub(1);
                                            u16c += i32::from(unsafe { *p }) << 8;
                                        }
                                        u8c = 0x10000
                                            + (((u16c as u32) & 0x3ff) << 10)
                                            + (u8c & 0x3ff);

                                        if !(0xd800..=0xdbff).contains(&u16c) {
                                            if can_retry {
                                                if crate::eval::typval::Callback::raw_cb_type(
                                                    &opt_vars().p_ccv,
                                                ) != K_CALLBACK_NONE_CONST
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
                                                conv_error = readfile_linenr(linecnt, unsafe {
                                                    std::slice::from_raw_parts(
                                                        ptr,
                                                        crate::memory::ptr_offset_from(p, ptr)
                                                            .max(0)
                                                            as usize,
                                                    )
                                                });
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as u32;
                                            }
                                        }
                                    }
                                } else if (fio_flags & FIO_UCS4) != 0 {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p }) << 24;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 16;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 8;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p });
                                    } else {
                                        p = p.wrapping_sub(1);
                                        u8c = u32::from(unsafe { *p });
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 8;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 16;
                                        p = p.wrapping_sub(1);
                                        u8c += u32::from(unsafe { *p }) << 24;
                                    }
                                    if u8c > i32::MAX as u32 {
                                        u8c = 0xfffd;
                                    }
                                } else {
                                    p = p.wrapping_sub(1);
                                    if unsafe { *p } < 0x80 {
                                        u8c = u32::from(unsafe { *p });
                                    } else {
                                        let len = utf_head_off(ptr.cast(), p.cast());
                                        p = p.wrapping_offset(-(len as isize));
                                        u8c = utf_ptr2char(p.cast()) as u32;
                                        if len == 0 {
                                            if can_retry {
                                                if crate::eval::typval::Callback::raw_cb_type(
                                                    &opt_vars().p_ccv,
                                                ) != K_CALLBACK_NONE_CONST
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
                                                conv_error = readfile_linenr(linecnt, unsafe {
                                                    std::slice::from_raw_parts(
                                                        ptr,
                                                        crate::memory::ptr_offset_from(p, ptr)
                                                            .max(0)
                                                            as usize,
                                                    )
                                                });
                                            }
                                            if bad_char_behavior == BAD_DROP {
                                                continue;
                                            }
                                            if bad_char_behavior != BAD_KEEP {
                                                u8c = bad_char_behavior as u32;
                                            }
                                        }
                                    }
                                }
                                debug_assert!(u8c <= i32::MAX as u32);
                                dest = dest.wrapping_offset(-(utf_char2len(u8c as i32) as isize));
                                utf_char2bytes(u8c as i32, dest.cast());
                            }

                            line_start = dest.wrapping_offset(-linerest);
                            unsafe { ptr::copy(buffer, line_start, linerest as usize) };
                            size = crate::memory::ptr_offset_from(
                                ptr.wrapping_offset(real_size as isize),
                                dest,
                            );
                            ptr = dest;
                        } else if curbuf_mut().b_p_bin == 0 {
                            let mut incomplete_tail = false;

                            p = ptr;
                            loop {
                                {
                                    let ascii_end = ptr.wrapping_offset(size);
                                    while crate::memory::ptr_offset_from(ascii_end, p)
                                        >= size_of::<u64>() as isize
                                    {
                                        let word = unsafe { ptr::read_unaligned(p.cast::<u64>()) };
                                        if (word & NONASCII_MASK) != 0 {
                                            break;
                                        }
                                        p = p.wrapping_add(size_of::<u64>());
                                    }
                                    while p < ascii_end && unsafe { *p } < 0x80 {
                                        p = p.wrapping_add(1);
                                    }
                                }

                                let todo =
                                    crate::memory::ptr_offset_from(ptr.wrapping_offset(size), p)
                                        as i32;
                                if todo <= 0 {
                                    break;
                                }
                                if unsafe { *p } >= 0x80 {
                                    let l = utf_ptr2len_len(p.cast(), todo);
                                    if l > todo && !incomplete_tail {
                                        if p > ptr || filesize > 0 {
                                            incomplete_tail = true;
                                        }
                                        if p > ptr {
                                            conv_restlen = todo;
                                            unsafe {
                                                ptr::copy(
                                                    p,
                                                    conv_rest.as_mut_ptr(),
                                                    conv_restlen as usize,
                                                )
                                            };
                                            size -= conv_restlen as isize;
                                            break;
                                        }
                                    }
                                    if l == 1 || l > todo {
                                        if can_retry && !incomplete_tail {
                                            break;
                                        }

                                        let err_slice = unsafe {
                                            std::slice::from_raw_parts(
                                                ptr,
                                                crate::memory::ptr_offset_from(p, ptr).max(0)
                                                    as usize,
                                            )
                                        };
                                        if iconv_fd != -1 && conv_error == 0 {
                                            conv_error = readfile_linenr(linecnt, err_slice);
                                        }

                                        if conv_error == 0 && illegal_byte == 0 {
                                            illegal_byte = readfile_linenr(linecnt, err_slice);
                                        }

                                        if bad_char_behavior == BAD_DROP {
                                            unsafe {
                                                ptr::copy(p.wrapping_add(1), p, (todo - 1) as usize)
                                            };
                                            size -= 1;
                                        } else {
                                            if bad_char_behavior != BAD_KEEP {
                                                unsafe { *p = bad_char_behavior as u8 };
                                            }
                                            p = p.wrapping_add(1);
                                        }
                                    } else {
                                        p = p.wrapping_offset(l as isize);
                                    }
                                }
                            }
                            if p < ptr.wrapping_offset(size) && !incomplete_tail {
                                if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_ccv)
                                    != K_CALLBACK_NONE_CONST
                                    && iconv_fd != -1
                                {
                                    did_iconv = true;
                                } else {
                                    advance_fenc = true;
                                }
                                file_rewind = true;
                                continue 'retry;
                            }
                        }

                        filesize += size as Off;

                        if fileformat == EOL_UNKNOWN {
                            if try_dos != 0 || try_unix != 0 {
                                if try_mac != 0 {
                                    try_mac = 1;
                                }

                                p = ptr;
                                let end_p = ptr.wrapping_offset(size);
                                while p < end_p {
                                    if unsafe { *p } == b'\n' {
                                        if try_unix == 0
                                            || (try_dos != 0
                                                && p > ptr
                                                && unsafe { *p.wrapping_sub(1) } == b'\r')
                                        {
                                            fileformat = EOL_DOS;
                                        } else {
                                            fileformat = EOL_UNIX;
                                        }
                                        break;
                                    } else if unsafe { *p } == b'\r' && try_mac != 0 {
                                        try_mac += 1;
                                    }
                                    p = p.wrapping_add(1);
                                }

                                if fileformat == EOL_UNIX && try_mac != 0 {
                                    try_mac = 1;
                                    try_unix = 1;
                                    while p >= ptr && unsafe { *p } != b'\r' {
                                        p = p.wrapping_sub(1);
                                    }
                                    if p >= ptr {
                                        p = ptr;
                                        while p < end_p {
                                            if unsafe { *p } == b'\n' {
                                                try_unix += 1;
                                            } else if unsafe { *p } == b'\r' {
                                                try_mac += 1;
                                            }
                                            p = p.wrapping_add(1);
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
                            let c = unsafe { *ptr };
                            if c != 0 && c != CAR && c != b'\n' {
                                continue;
                            }
                            if c == 0 {
                                unsafe { *ptr = b'\n' };
                            } else if c == b'\n' {
                                unsafe { *ptr = CAR };
                            } else {
                                if skip_count == 0 {
                                    unsafe { *ptr = 0 };
                                    let len = (crate::memory::ptr_offset_from(ptr, line_start) + 1)
                                        as Colnr;
                                    if !ml_append(lnum, line_start.cast(), len, newfile) {
                                        error = true;
                                        break;
                                    }
                                    if read_undo_file && len > 0 {
                                        sha_ctx.update(unsafe {
                                            std::slice::from_raw_parts(line_start, len as usize)
                                        });
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
                                line_start = ptr.wrapping_add(1);
                            }
                        }
                    } else {
                        let end = ptr.wrapping_offset(size);

                        while ptr < end {
                            let rem = crate::memory::ptr_offset_from(end, ptr) as usize;
                            let slice = crate::memory::cstr_slice(ptr.cast(), rem);
                            let Some(nl_pos) = slice.iter().position(|&b| b == b'\n') else {
                                let rem_slice = unsafe { std::slice::from_raw_parts_mut(ptr, rem) };
                                for b in rem_slice {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                                ptr = end;
                                break;
                            };

                            let nl = ptr.wrapping_add(nl_pos);
                            if nl_pos > 0 {
                                let scan_slice =
                                    unsafe { std::slice::from_raw_parts_mut(ptr, nl_pos) };
                                for b in scan_slice {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                            }

                            ptr = nl;
                            if skip_count == 0 {
                                unsafe { *ptr = 0 };
                                let mut len =
                                    (crate::memory::ptr_offset_from(ptr, line_start) + 1) as Colnr;
                                if fileformat == EOL_DOS {
                                    if ptr > line_start && unsafe { *ptr.wrapping_sub(1) } == CAR {
                                        unsafe { *ptr.wrapping_sub(1) = 0 };
                                        len -= 1;
                                    } else if ff_error != EOL_DOS {
                                        if try_unix != 0
                                            && !read_stdin
                                            && (read_buffer
                                                || unsafe { vim_lseek(fd, 0, SEEK_SET) } == 0)
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
                                if !ml_append(lnum, line_start.cast(), len, newfile) {
                                    error = true;
                                    break;
                                }
                                if read_undo_file && len > 0 {
                                    sha_ctx.update(unsafe {
                                        std::slice::from_raw_parts(line_start, len as usize)
                                    });
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
                            line_start = ptr.wrapping_add(1);
                            ptr = ptr.wrapping_add(1);
                        }
                        size = -1;
                    }
                    linerest = crate::memory::ptr_offset_from(ptr, line_start);

                    if show_msg && (progress || os_hrtime() - read_start > slow_io_ns()) {
                        let mut info: FileInfo = FileInfo::new();
                        let total = if os_fileinfo_fd(fd, &mut info) {
                            os_fileinfo_size(&info)
                        } else {
                            0
                        };
                        let pct = ((filesize as u64) * 100)
                            .checked_div(total)
                            .map_or(-1, |v| (v.min(100)) as i32);
                        if !progress || pct != percent {
                            progress = true;
                            percent = pct;
                            add_quoted_fname(
                                g_st().iobuff_bytes_mut(),
                                unsafe { g_st().curbuf.as_ref() },
                                crate::memory::cstr_as_opt_cstr(sfname.cast()),
                            );
                            let iobuf_cstr = crate::memory::cstr_until_nul(g_st().iobuff_bytes());
                            let _ = readfile_progress(
                                Some(iobuf_cstr),
                                crate::memory::cstr_until_nul(&msg_id),
                                c"running",
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
                && curbuf_mut().b_p_bin == 0
                && fileformat == EOL_DOS
                && unsafe { *ptr.wrapping_sub(1) } == CTRL_Z
            {
                ptr = ptr.wrapping_sub(1);
                linerest -= 1;
                if set_options {
                    curbuf_mut().b_p_eof = 1;
                }
            }

            if !error && !g_st().got_int && linerest != 0 {
                if set_options {
                    curbuf_mut().b_p_eol = 0;
                }
                unsafe { *ptr = 0 };
                let len = (crate::memory::ptr_offset_from(ptr, line_start) + 1) as Colnr;
                if !ml_append(lnum, line_start.cast(), len, newfile) {
                    error = true;
                } else {
                    if read_undo_file && len > 0 {
                        sha_ctx.update(unsafe {
                            std::slice::from_raw_parts(line_start, len as usize)
                        });
                    }
                    lnum += 1;
                    read_no_eol_lnum = lnum;
                }
            }

            if set_options {
                save_file_ff(curbuf_mut());
                set_option_direct(
                    K_OPT_FILEENCODING,
                    cstr_as_obj(crate::memory::cstr_as_opt_cstr(fenc.cast())),
                    OPT_LOCAL,
                    0,
                );
            }
            drop(fenc_owned);
            if iconv_fd != -1 {
                call_iconv_close(iconv_fd);
            }

            if !read_buffer && !read_stdin {
                unsafe { close(fd) };
            } else {
                os_set_cloexec(fd);
            }
            drop(buffer_vec);

            if read_stdin {
                unsafe { close(fd) };
                if g_st().stdin_fd < 0 {
                    #[cfg(not(windows))]
                    {
                        g_st().vim_ignored = unsafe { dup(2) };
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
                if newfile && wasempty && (curbuf_mut().b_ml.ml_flags & ML_EMPTY) == 0 {
                    ml_delete(curbuf_mut().b_ml.ml_line_count);
                    linecnt -= 1;
                }
                curbuf_mut().deleted_bytes = 0;
                curbuf_mut().deleted_bytes2 = 0;
                curbuf_mut().deleted_codepoints = 0;
                curbuf_mut().deleted_codeunits = 0;
                linecnt = curbuf_mut().b_ml.ml_line_count - linecnt;
                if filesize == 0 {
                    linecnt = 0;
                }
                if newfile || read_buffer {
                    redraw_curbuf_later(UPD_NOT_VALID);
                    diff_invalidate(curbuf_mut());
                    fold_update_all(curwin_mut());
                } else if linecnt != 0 {
                    appended_lines_mark(from, linecnt);
                }

                if g_st().got_int {
                    if progress {
                        let _ = readfile_progress(
                            None,
                            crate::memory::cstr_until_nul(&msg_id),
                            c"cancel",
                            -1,
                            false,
                        );
                    }
                    if (flags & READ_DUMMY) == 0 {
                        filemess(
                            unsafe { g_st().curbuf.as_ref() },
                            crate::memory::cstr_as_opt_cstr(sfname.cast()),
                            tr(E_INTERR),
                            None,
                        );
                        if newfile {
                            curbuf_mut().b_p_ro = 1;
                        }
                    }
                    g_st().msg_scroll = msg_save;
                    check_marks_read();
                    retval = OK;
                    break 'theend;
                }

                if !filtering && (flags & READ_DUMMY) == 0 && !silent {
                    let iobuf_ptr = g_st().iobuff.as_mut_ptr();
                    add_quoted_fname(
                        g_st().iobuff_bytes_mut(),
                        unsafe { g_st().curbuf.as_ref() },
                        crate::memory::cstr_as_opt_cstr(sfname.cast()),
                    );
                    let mut c = false;

                    let mut buflen = crate::memory::cstr_len(iobuf_ptr) as i32;
                    #[cfg(unix)]
                    {
                        if s_isfifo(perm as u64) {
                            buflen += vim_snprintf!(
                                unsafe { iobuf_ptr.offset(buflen as isize) },
                                (IOSIZE as i32 - buflen) as usize,
                                c"%s",
                                tr(c"[fifo]"),
                            );
                            c = true;
                        }
                        if s_issock(perm as u64) {
                            buflen += vim_snprintf!(
                                unsafe { iobuf_ptr.offset(buflen as isize) },
                                (IOSIZE as i32 - buflen) as usize,
                                c"%s",
                                tr(c"[socket]"),
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
                        if s_ischr(perm as u64) {
                            buflen += vim_snprintf!(
                                iobuf_ptr.wrapping_offset(buflen as isize),
                                (IOSIZE as i32 - buflen) as usize,
                                c"%s",
                                tr(c"[character special]"),
                            );
                            c = true;
                        }
                    }
                    if curbuf_mut().b_p_ro != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            if shortmess(K_SHM_RO) {
                                tr(c"[RO]")
                            } else {
                                tr(c"[readonly]")
                            },
                        );
                        c = true;
                    }
                    if read_no_eol_lnum != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[noeol]"),
                        );
                        c = true;
                    }
                    if ff_error == EOL_DOS {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[CR missing]"),
                        );
                        c = true;
                    }
                    if split != 0 {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[long lines split]"),
                        );
                        c = true;
                    }
                    if notconverted {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[NOT converted]"),
                        );
                        c = true;
                    } else if converted {
                        buflen += vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[converted]"),
                        );
                        c = true;
                    }
                    if conv_error != 0 {
                        vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            tr(c"[CONVERSION ERROR in line %lld]"),
                            i64::from(conv_error),
                        );
                        c = true;
                    } else if illegal_byte > 0 {
                        vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            tr(c"[ILLEGAL BYTE in line %lld]"),
                            i64::from(illegal_byte),
                        );
                        c = true;
                    } else if error {
                        vim_snprintf!(
                            iobuf_ptr.wrapping_offset(buflen as isize),
                            (IOSIZE as i32 - buflen) as usize,
                            c"%s",
                            tr(c"[READ ERRORS]"),
                        );
                        c = true;
                    }
                    if msg_add_fileformat(fileformat) {
                        c = true;
                    }

                    msg_add_lines(i32::from(c), linecnt, filesize);

                    g_st().keep_msg = None;
                    p = ptr::null_mut();
                    g_st().msg_scrolled_ign = true;

                    if !read_stdin && !read_buffer {
                        if g_st().msg_col > 0 {
                            msg_putchar(b'\r' as i32);
                        }
                        p = if progress {
                            let iobuf_cstr = crate::memory::cstr_until_nul(g_st().iobuff_bytes());
                            readfile_progress(
                                Some(iobuf_cstr),
                                crate::memory::cstr_until_nul(&msg_id),
                                if error { c"failed" } else { c"success" },
                                -1,
                                true,
                            )
                            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast())
                        } else {
                            msg_trunc(iobuf_ptr, false, 0).cast()
                        };
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
                        None,
                        crate::memory::cstr_until_nul(&msg_id),
                        if error { c"failed" } else { c"success" },
                        -1,
                        false,
                    );
                }

                if newfile
                    && (error
                        || conv_error != 0
                        || (illegal_byte > 0 && bad_char_behavior != BAD_KEEP))
                {
                    curbuf_mut().b_p_ro = 1;
                }

                u_clearline(curbuf_mut());

                curwin_mut().w_cursor.lnum = from + 1;
                check_cursor_lnum(curwin_mut());
                beginline(BL_WHITE | BL_FIX);

                if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                    curbuf_mut().b_op_start.lnum = from + 1;
                    curbuf_mut().b_op_start.col = 0;
                    curbuf_mut().b_op_end.lnum = from + linecnt;
                    curbuf_mut().b_op_end.col = 0;
                }
            }
            g_st().msg_scroll = msg_save;

            check_marks_read();

            curbuf_mut().b_no_eol_lnum = read_no_eol_lnum;

            if (flags & READ_KEEP_UNDO) != 0 {
                u_find_first_changed();
            }

            if read_undo_file {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                sha_ctx.finish_into(&mut hash);
                u_read_undo(None, &hash, crate::memory::cstr_as_opt_cstr(fname.cast()));
            }

            if !read_stdin && !read_fifo && (!read_buffer || !sfname.is_null()) {
                let m = g_st().msg_scroll;
                let n = g_st().msg_scrolled;

                if set_options {
                    save_file_ff(curbuf_mut());
                }

                g_st().msg_scroll = 1;
                let sfname_opt = crate::memory::cstr_as_opt_cstr(sfname.cast());
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPOST,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );
                } else if newfile || (read_buffer && !sfname.is_null()) {
                    let did_bufrp = apply_autocmds_exarg(
                        EVENT_BUFREADPOST,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut(),
                        eap,
                    );

                    if !curbuf_mut().b_au_did_filetype
                        && !crate::memory::cstr_is_empty(curbuf_mut().b_p_ft)
                    {
                        apply_autocmds(
                            EVENT_FILETYPE,
                            crate::memory::cstr_as_opt_cstr(curbuf_mut().b_p_ft),
                            crate::memory::cstr_as_opt_cstr(curbuf_mut().b_fname),
                            true,
                            curbuf_opt_mut(),
                        );
                    }

                    if !did_bufrp
                        && crate::memory::cstr_is_empty(curbuf_mut().b_p_ft)
                        && augroup_exists(Some(c"filetypedetect"))
                    {
                        apply_autocmds_group(
                            EVENT_BUFREADPOST,
                            sfname_opt,
                            None,
                            true,
                            augroup_find(c"filetypedetect"),
                            curbuf_opt_mut(),
                            Some(crate::globals::curwin_mut()),
                            None,
                            None,
                            false,
                        );
                    }
                } else {
                    apply_autocmds_exarg(
                        EVENT_FILEREADPOST,
                        sfname_opt,
                        sfname_opt,
                        false,
                        None,
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

        let mfp = curbuf_mut().b_ml.mfp_ptr();
        if !mfp.is_null() && unsafe { (*mfp).mf_dirty } == MF_DIRTY_YES_NOSYNC {
            unsafe {
                (*mfp).mf_dirty = MF_DIRTY_YES;
            }
        }

        retval
    }
}

/// Returns true if the file name argument is of the form "/dev/fd/\d\+".
#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "macos",
    test
))]
pub fn is_dev_fd_file(bytes: &[u8]) -> bool {
    bytes.starts_with(b"/dev/fd/")
        && bytes.len() > 8
        && bytes[8..].iter().all(|&b| ascii_isdigit(b as i32))
        && (bytes.len() > 9 || !matches!(bytes[8], b'0' | b'1' | b'2'))
}

/// Estimate the line number where we are now.
fn readfile_linenr(linecnt: Linenr, slice: &[u8]) -> Linenr {
    let newlines = slice.iter().filter(|&&b| b == b'\n').count() as Linenr;
    curbuf_mut().b_ml.ml_line_count - linecnt + 1 + newlines
}

/// Fill "*eap" to force the 'fileencoding', 'fileformat' and 'binary' to be
/// equal to the buffer "buf".
pub fn prep_exarg(eap: &mut Exarg, buf: &FileBuf) -> std::ffi::CString {
    let fenc_bytes = crate::memory::cstr_as_bytes(buf.b_p_fenc);
    let mut cmd_vec = Vec::with_capacity(8 + fenc_bytes.len());
    cmd_vec.extend_from_slice(b"e ++enc=");
    cmd_vec.extend_from_slice(fenc_bytes);
    let cmd_cstr = std::ffi::CString::new(cmd_vec).unwrap_or_default();
    eap.cmd = cmd_cstr.as_ptr().cast_mut();
    eap.force_enc = 8;
    eap.bad_char = buf.b_bad_char;
    eap.force_ff = (crate::memory::cstr_first_byte(buf.b_p_ff)) as i32;
    eap.force_bin = if buf.b_p_bin != 0 {
        FORCE_BIN
    } else {
        FORCE_NOBIN
    };
    eap.read_edit = 0;
    eap.forceit = 0;
    cmd_cstr
}

/// Set default or forced 'fileformat' and 'binary'.
pub fn set_file_options(set_options: bool, eap: Option<&Exarg>) {
    if set_options {
        if eap.is_some() && eap.unwrap().force_ff != 0 {
            set_fileformat(
                get_fileformat_force(curbuf_mut(), (eap.is_some()).then(|| eap.unwrap())),
                OPT_LOCAL,
            );
        } else if !crate::memory::cstr_is_empty(opt_vars().p_ffs) {
            set_fileformat(default_fileformat(), OPT_LOCAL);
        }
    }

    if let Some(eap) = eap
        && eap.force_bin != 0
    {
        let oldval = curbuf_mut().b_p_bin;
        curbuf_mut().b_p_bin = i32::from(eap.force_bin == FORCE_BIN);
        set_options_bin(oldval, curbuf_mut().b_p_bin, OPT_LOCAL);
    }
}

/// Set forced 'fileencoding'.
pub fn set_forced_fenc(eap: &Exarg) {
    if eap.force_enc == 0 {
        return;
    }

    let fenc = enc_canonize(eap.cmd.wrapping_add(eap.force_enc as usize));
    set_option_direct(
        K_OPT_FILEENCODING,
        cstr_as_obj(crate::memory::cstr_as_opt_cstr(fenc.as_ptr())),
        OPT_LOCAL,
        0,
    );
}

/// Find next fileencoding to use from 'fileencodings'.
fn next_fenc(pp: &mut &[u8]) -> std::ffi::CString {
    if pp.is_empty() || pp[0] == 0 {
        *pp = b"";
        return std::ffi::CString::default();
    }
    if let Some(comma_pos) = pp.iter().position(|&b| b == b',') {
        let tmp = std::ffi::CString::new(&pp[..comma_pos]).unwrap_or_default();
        *pp = &pp[comma_pos + 1..];
        enc_canonize(tmp.as_ptr())
    } else {
        let tmp = std::ffi::CString::new(crate::memory::bytes_until_nul(pp)).unwrap_or_default();
        *pp = b"";
        enc_canonize(tmp.as_ptr())
    }
}

/// Convert a file with the 'charconvert' expression.
fn readfile_charconvert(fname: &CStr, fenc: &CStr, fdp: &mut i32) -> Option<std::ffi::CString> {
    let mut errmsg: Option<&std::ffi::CStr> = None;

    let mut tmpname = vim_tempname();
    match tmpname {
        None => {
            errmsg = Some(tr(c"Can't find temp file for conversion"));
        }
        Some(ref tn) => {
            os_close(*fdp);
            *fdp = -1;
            if !eval_charconvert(fenc, c"utf-8", fname, tn) {
                errmsg = Some(tr(c"Conversion with 'charconvert' failed"));
            }
            if errmsg.is_none() {
                *fdp = os_open(tn.as_ptr(), O_RDONLY, 0);
                if *fdp < 0 {
                    errmsg = Some(tr(c"can't read output of 'charconvert'"));
                }
            }
        }
    }

    if let Some(errmsg) = errmsg {
        msg(errmsg, 0);
        if let Some(ref tn) = tmpname {
            os_remove(tn.as_ptr());
            tmpname = None;
        }
    }

    if *fdp < 0 {
        *fdp = os_open(fname.as_ptr(), O_RDONLY, 0);
    }

    tmpname
}

/// Set the name of the current buffer.
pub fn set_rw_fname(fname: Option<&CStr>, sfname: Option<&CStr>) -> bool {
    let fname_ptr = fname.map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let sfname_ptr = sfname.map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let buf = g_st().curbuf;

    if curbuf_mut().b_p_bl != 0 {
        apply_autocmds(EVENT_BUFDELETE, None, None, false, curbuf_opt_mut());
    }
    apply_autocmds(EVENT_BUFWIPEOUT, None, None, false, curbuf_opt_mut());
    if aborting() {
        return false;
    }
    if g_st().curbuf != buf {
        emsg(tr(E_AUCHANGEDBUF));
        return false;
    }

    if setfname(
        curbuf_mut(),
        crate::memory::cstr_as_opt_cstr(fname_ptr),
        crate::memory::cstr_as_opt_cstr(sfname_ptr),
        false,
    ) {
        curbuf_mut().b_flags |= BF_NOTEDITED;
    }

    apply_autocmds(EVENT_BUFNEW, None, None, false, curbuf_opt_mut());
    if curbuf_mut().b_p_bl != 0 {
        apply_autocmds(EVENT_BUFADD, None, None, false, curbuf_opt_mut());
    }
    if aborting() {
        return false;
    }

    if crate::memory::cstr_is_empty(curbuf_mut().b_p_ft) {
        if augroup_exists(Some(c"filetypedetect")) {
            do_doautocmd(c"filetypedetect BufRead", false, None);
        }
        do_modelines(0);
    }

    true
}

/// Put file name into the specified buffer with quotes.
pub fn add_quoted_fname(ret_buf: &mut [u8], buf: Option<&FileBuf>, fname: Option<&CStr>) {
    let fname_cstr = fname.unwrap_or(c"-stdin-");
    let bufsize = ret_buf.len();
    ret_buf[0] = b'"';
    let mut len: usize = 1;
    len += home_replace(
        buf,
        fname_cstr.as_ptr(),
        ret_buf[len..].as_mut_ptr(),
        bufsize.saturating_sub(4),
        true,
    );
    crate::memory::copy_bytes_to_c_buf(&mut ret_buf[len..], b"\" ");
}

/// Sets progress-message id of reading/writing file `fname`, e.g. `nvim.bufread "foo.txt"`.
pub fn fname_msg_id(
    msg_slice: &mut [u8],
    prefix: &std::ffi::CStr,
    buf: Option<&FileBuf>,
    fname: Option<&CStr>,
) {
    let size = msg_slice.len();
    let mut len = crate::memory::copy_bytes_to_c_buf(msg_slice, prefix.to_bytes());
    if len < size {
        len += crate::memory::copy_bytes_to_c_buf(&mut msg_slice[len..], b" ");
    }
    if len < size {
        add_quoted_fname(&mut msg_slice[len..], buf, fname);
    }
    let total = crate::memory::bytes_until_nul(msg_slice).len();
    if total > 0 {
        msg_slice[total - 1] = 0;
    }
}

/// Append message for text mode to IObuff.
pub fn msg_add_fileformat(eol_type: i32) -> bool {
    #[cfg(not(windows))]
    if eol_type == EOL_DOS {
        crate::memory::append_bytes_to_c_buf(g_st().iobuff_bytes_mut(), tr(c"[dos]").to_bytes());
        return true;
    }
    if eol_type == EOL_MAC {
        crate::memory::append_bytes_to_c_buf(g_st().iobuff_bytes_mut(), tr(c"[mac]").to_bytes());
        return true;
    }
    #[cfg(windows)]
    if eol_type == EOL_UNIX {
        crate::memory::append_bytes_to_c_buf(g_st().iobuff_bytes_mut(), tr(c"[unix]").to_bytes());
        return true;
    }
    false
}

/// Append line and character count to IObuff.
pub fn msg_add_lines(insert_space: i32, lnum: Linenr, nchars: Off) {
    let iobuf_ptr = g_st().iobuff.as_mut_ptr();
    let mut len = crate::memory::cstr_len(iobuf_ptr);
    let space = if insert_space != 0 {
        c" ".as_ptr()
    } else {
        c"".as_ptr()
    };

    if shortmess(K_SHM_LINES) {
        vim_snprintf!(
            iobuf_ptr.wrapping_add(len),
            IOSIZE - len,
            tr(c"%s%lldL, %lldB"),
            space,
            i64::from(lnum),
            nchars,
        );
    } else {
        len += vim_snprintf!(
            iobuf_ptr.wrapping_add(len),
            IOSIZE - len,
            ngettext(c"%s%lld line, ", c"%s%lld lines, ", lnum as usize,),
            space,
            i64::from(lnum),
        ) as usize;
        vim_snprintf!(
            iobuf_ptr.wrapping_add(len),
            IOSIZE - len,
            ngettext(c"%lld byte", c"%lld bytes", nchars as usize,),
            nchars,
        );
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
pub fn need_conversion(fenc: Option<&CStr>) -> bool {
    let same_encoding: bool;
    let fenc_flags: i32;

    let p_enc_opt = crate::memory::cstr_as_opt_cstr(opt_vars().p_enc);
    if fenc.is_none_or(|s| s.to_bytes().is_empty()) || p_enc_opt == fenc {
        same_encoding = true;
        fenc_flags = 0;
    } else {
        let enc_flags = get_fio_flags(p_enc_opt);
        fenc_flags = get_fio_flags(fenc);
        same_encoding = enc_flags != 0 && fenc_flags == enc_flags;
    }
    if same_encoding {
        return false;
    }

    fenc_flags != FIO_UTF8
}

/// Return the FIO_ flags needed for the internal conversion if 'name' was
/// unicode or latin1, otherwise 0.
pub fn get_fio_flags(name: Option<&CStr>) -> i32 {
    let name_cstr = match name.filter(|s| !s.to_bytes().is_empty()) {
        Some(s) => s,
        None => crate::memory::cstr_as_opt_cstr(opt_vars().p_enc).unwrap_or(c"utf-8"),
    };
    let prop = enc_canon_props(name_cstr.to_bytes());
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

/// Check for a Unicode BOM (Byte Order Mark) at the start of `bytes`.
fn check_for_bom(bytes: &[u8], lenp: &mut i32, flags: i32) -> Option<&'static CStr> {
    let mut name: Option<&'static CStr> = None;
    let mut len: i32 = 2;

    if bytes.starts_with(&[0xef, 0xbb, 0xbf])
        && (flags == FIO_ALL || flags == FIO_UTF8 || flags == 0)
    {
        name = Some(c"utf-8");
        len = 3;
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.starts_with(&[0xff, 0xfe, 0, 0])
            && (flags == FIO_ALL || flags == (FIO_UCS4 | FIO_ENDIAN_L))
        {
            name = Some(c"ucs-4le");
            len = 4;
        } else if flags == (FIO_UCS2 | FIO_ENDIAN_L) {
            name = Some(c"ucs-2le");
        } else if flags == FIO_ALL || flags == (FIO_UTF16 | FIO_ENDIAN_L) {
            name = Some(c"utf-16le");
        }
    } else if bytes.starts_with(&[0xfe, 0xff])
        && (flags == FIO_ALL || flags == FIO_UCS2 || flags == FIO_UTF16)
    {
        if flags == FIO_UCS2 {
            name = Some(c"ucs-2");
        } else {
            name = Some(c"utf-16");
        }
    } else if bytes.starts_with(&[0, 0, 0xfe, 0xff]) && (flags == FIO_ALL || flags == FIO_UCS4) {
        name = Some(c"ucs-4");
        len = 4;
    }

    *lenp = len;
    name
}

/// Shorten filename of a buffer.
pub fn shorten_buf_fname(buf: &mut FileBuf, dirname: &CStr, force: i32) -> bool {
    if buf.b_fname.is_null()
        || bt_nofilename(Some(&*buf))
        || path_with_url(buf.b_fname)
        || !(force != 0 || buf.b_sfname.is_null() || path_is_absolute(buf.b_sfname))
    {
        return false;
    }

    let p = path_shorten_fname(buf.b_ffname, dirname.as_ptr().cast_mut().cast());
    let shorten = !crate::memory::cstr_is_empty(p);
    if shorten
        && !buf.b_sfname.is_null()
        && crate::memory::cstr_from_ptr(p) == crate::memory::cstr_from_ptr(buf.b_sfname)
    {
        return false;
    }

    let new_fname = if shorten { p } else { buf.b_ffname };
    let changed = new_fname.is_null()
        || crate::memory::cstr_from_ptr(new_fname) != crate::memory::cstr_from_ptr(buf.b_fname);

    buf.set_sfname(shorten.then(|| cstr_to_cstring(p)));
    if shorten {
        buf.b_fname = buf.b_sfname;
    } else {
        buf.b_fname = buf.b_ffname;
    }
    changed
}

/// Shortens filenames for all buffers.
pub fn shorten_fnames(force: i32) {
    let mut dirname: [u8; MAXPATHL] = [0; MAXPATHL];
    let mut changed = false;

    os_dirname(dirname.as_mut_ptr(), MAXPATHL);
    let dirname_cstr = crate::memory::cstr_until_nul(&dirname);
    for buf in for_all_buffers_mut() {
        changed |= shorten_buf_fname(buf, dirname_cstr, force);
        mf_fullname(buf.b_ml.mfp_opt());
    }
    if changed {
        status_redraw_all();
        g_st().redraw_tabline = true;
    }
}

/// Get new filename ended by given extension.
pub fn modname(
    fname: Option<&CStr>,
    ext: &CStr,
    mut prepend_dot: bool,
) -> Option<std::ffi::CString> {
    let ext_bytes = ext.to_bytes();
    let extlen = ext_bytes.len();
    let fname_bytes = fname.map_or(&[][..], CStr::to_bytes);

    let mut retval_vec: Vec<u8>;
    let fnamelen: usize;

    if fname_bytes.is_empty() {
        retval_vec = vec![0; MAXPATHL + extlen + 3];
        if !os_dirname(retval_vec.as_mut_ptr(), MAXPATHL) || retval_vec[0] == 0 {
            return None;
        }
        add_pathsep(retval_vec.as_mut_ptr().cast());
        fnamelen = crate::memory::cstr_until_nul(&retval_vec).to_bytes().len();
        prepend_dot = false;
    } else {
        fnamelen = fname_bytes.len();
        retval_vec = vec![0; fnamelen + extlen + 3];
        retval_vec[..fnamelen].copy_from_slice(fname_bytes);
    }

    let mut ptr_idx = fnamelen;
    while ptr_idx > 0 {
        if vim_ispathsep(retval_vec.get(ptr_idx).copied().unwrap_or(0) as i32) {
            ptr_idx += 1;
            break;
        }
        let head_off = utf_head_off(
            retval_vec.as_ptr().cast(),
            retval_vec[(ptr_idx - 1)..].as_ptr().cast(),
        ) as usize;
        ptr_idx -= head_off + 1;
    }

    let mut ptrlen = fnamelen - ptr_idx;
    if ptrlen > BASENAMELEN {
        ptrlen = BASENAMELEN;
        retval_vec[ptr_idx + ptrlen] = 0;
    }

    let mut s_idx = ptr_idx + ptrlen;
    retval_vec[s_idx..(s_idx + extlen)].copy_from_slice(ext_bytes);
    retval_vec[s_idx + extlen] = 0;

    let e_ptr = path_tail(crate::memory::cstr_until_nul(&retval_vec));
    let e_idx = crate::memory::ptr_offset_from(e_ptr, retval_vec.as_ptr().cast()) as usize;
    if prepend_dot && retval_vec[e_idx] != b'.' {
        let end_idx = fnamelen + extlen;
        retval_vec.copy_within(e_idx..=end_idx, e_idx + 1);
        retval_vec[e_idx] = b'.';
    }

    if fname.is_some() && fname_bytes == crate::memory::cstr_until_nul(&retval_vec).to_bytes() {
        let mut replaced = false;
        while s_idx > ptr_idx {
            s_idx -= 1;
            if retval_vec[s_idx] != b'_' {
                retval_vec[s_idx] = b'_';
                replaced = true;
                break;
            }
        }
        if !replaced {
            retval_vec[ptr_idx] = b'v';
        }
    }
    Some(crate::memory::cstring_until_nul(&retval_vec))
}

/// Like fgets(), but if the file line is too long, it is truncated and the
/// rest of the line is thrown away.
pub fn vim_fgets(buf: &mut [u8], fp: &crate::os::fs::CFile) -> bool {
    let size = buf.len();
    if size < 2 {
        if size == 1 {
            buf[0] = 0;
        }
        return true;
    }

    buf[size - 2] = 0;

    let mut got_line: bool;
    loop {
        os_set_errno(0);
        got_line = fp.gets(buf);
        if got_line || os_errno() != EINTR || !fp.has_error() {
            break;
        }
    }

    let penultimate = buf[size - 2];
    if penultimate != 0 && penultimate != b'\n' {
        let mut tbuf: [u8; 200] = [0; 200];
        buf[size - 1] = 0;

        loop {
            tbuf[tbuf.len() - 2] = 0;
            os_set_errno(0);
            got_line = fp.gets(&mut tbuf);
            if !got_line && (fp.is_eof() || os_errno() != EINTR) {
                break;
            }
            if tbuf[tbuf.len() - 2] == 0 || tbuf[tbuf.len() - 2] == b'\n' {
                break;
            }
        }
    }
    !got_line
}

/// Read 2 bytes from "fd" and turn them into an int, MSB first.
pub fn get2c(fd: &crate::os::fs::CFile) -> i32 {
    let n = fd.getc();
    if n == EOF {
        return -1;
    }
    let c = fd.getc();
    if c == EOF {
        return -1;
    }
    (n << 8) + c
}

/// Read 3 bytes from "fd" and turn them into an int, MSB first.
pub fn get3c(fd: &crate::os::fs::CFile) -> i32 {
    let mut n = fd.getc();
    if n == EOF {
        return -1;
    }
    let mut c = fd.getc();
    if c == EOF {
        return -1;
    }
    n = (n << 8) + c;
    c = fd.getc();
    if c == EOF {
        return -1;
    }
    (n << 8) + c
}

/// Read 4 bytes from "fd" and turn them into an int, MSB first.
pub fn get4c(fd: &crate::os::fs::CFile) -> i32 {
    let mut c = fd.getc();
    if c == EOF {
        return -1;
    }
    let mut n = c as u32;
    c = fd.getc();
    if c == EOF {
        return -1;
    }
    n = (n << 8) + (c as u32);
    c = fd.getc();
    if c == EOF {
        return -1;
    }
    n = (n << 8) + (c as u32);
    c = fd.getc();
    if c == EOF {
        return -1;
    }
    n = (n << 8) + (c as u32);
    n as i32
}

/// Read 8 bytes from `fd` and turn them into a time_t, MSB first.
pub fn get8ctime(fd: &crate::os::fs::CFile) -> time_t {
    let mut n: time_t = 0;

    for _ in 0..8 {
        let c = fd.getc();
        if c == EOF {
            return -1;
        }
        n = (n << 8) + i64::from(c);
    }
    n
}

/// Reads a string of length "cnt" from "fd" into allocated memory.
pub fn read_string(fd: &crate::os::fs::CFile, cnt: usize) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(cnt + 1);
    for _ in 0..cnt {
        let c = fd.getc();
        if c == EOF {
            return None;
        }
        buf.push(c as u8);
    }
    buf.push(0);
    Some(buf)
}

/// Writes a number to file "fd", most significant bit first, in "len" bytes.
pub fn put_bytes(fd: &crate::os::fs::CFile, number: u64, len: usize) -> bool {
    debug_assert!(len > 0);
    for i in (0..len).rev() {
        if fd.putc(((number >> (i * 8)) & 0xff) as i32) == EOF {
            return false;
        }
    }
    true
}

/// Writes time_t to file "fd" in 8 bytes.
pub fn put_time(fd: &crate::os::fs::CFile, time_: time_t) -> bool {
    let buf: [u8; 8] = (time_ as u64).to_be_bytes();
    fd.write_all(&buf)
}

fn rename_with_tmp(from: &CStr, to: &CStr) -> i32 {
    let from_bytes = from.to_bytes();
    let from_ptr = from.as_ptr();
    let to_ptr = to.as_ptr();
    if from_bytes.len() >= MAXPATHL - 5 {
        return -1;
    }

    let mut tempname: [u8; MAXPATHL + 1] = [0; MAXPATHL + 1];
    crate::memory::copy_bytes_to_c_buf(&mut tempname, from_bytes);
    for n in 123..99999 {
        let tail = path_tail(tempname.as_ptr().cast());
        let rem = (MAXPATHL + 1)
            - crate::memory::ptr_offset_from(tail, tempname.as_ptr().cast()) as usize;
        vim_snprintf!(tail, rem, c"%d", n);

        if !os_path_exists(tempname.as_ptr().cast()) {
            if os_rename(from_ptr, tempname.as_ptr().cast()) {
                if os_rename(tempname.as_ptr().cast(), to_ptr) {
                    return 0;
                }
                if !os_rename(tempname.as_ptr().cast(), from_ptr) {
                    logmsg!(
                        LOG_ERR,
                        None,
                        Some(c"rename_with_tmp"),
                        0,
                        true,
                        c"failed to restore %s, file is at: %s",
                        from_ptr,
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

/// Rename a file, falling back to copying across filesystems if needed.
pub fn vim_rename(from: &CStr, to: &CStr) -> i32 {
    let from_ptr = from.as_ptr();
    let to_ptr = to.as_ptr();
    let mut use_tmp_file = false;

    if path_equal(from_ptr, to_ptr, K_PATH_CMP_LITERAL) {
        if opt_vars().p_fic != 0
            && crate::memory::cstr_from_ptr(path_tail(from_ptr))
                != crate::memory::cstr_from_ptr(path_tail(to_ptr))
        {
            use_tmp_file = true;
        } else {
            return 0;
        }
    }

    let mut from_info: FileInfo = FileInfo::new();
    if !os_fileinfo(from_ptr, &mut from_info) {
        return -1;
    }

    let mut to_info: FileInfo = FileInfo::new();
    if os_fileinfo(to_ptr, &mut to_info) && os_fileinfo_id_equal(&from_info, &to_info) {
        use_tmp_file = true;
    }

    if use_tmp_file {
        return rename_with_tmp(from, to);
    }

    if os_rename(from_ptr, to_ptr) {
        return 0;
    }

    if os_remove(to_ptr) == 0 && os_rename(from_ptr, to_ptr) {
        return 0;
    }

    if !vim_copyfile(from, to) {
        return -1;
    }

    if os_fileinfo(from_ptr, &mut from_info) {
        os_remove(from_ptr);
    }

    0
}

/// Create the new file with same permissions as the original.
pub fn vim_copyfile(from: &CStr, to: &CStr) -> bool {
    let from_ptr = from.as_ptr();
    let to_ptr = to.as_ptr();
    #[cfg(not(windows))]
    {
        let mut from_info: FileInfo = FileInfo::new();
        if os_fileinfo_link(from_ptr, &mut from_info) && s_islnk(from_info.stat.st_mode) {
            let mut ret = -1;
            let mut linkbuf: [u8; MAXPATHL + 1] = [0; MAXPATHL + 1];
            let len = unsafe { readlink(from_ptr.cast(), linkbuf.as_mut_ptr(), MAXPATHL) };
            if len > 0 {
                linkbuf[len as usize] = 0;
                ret = unsafe { symlink(linkbuf.as_ptr(), to_ptr.cast()) };
            }
            return ret == 0;
        }
    }

    let acl: VimAcl = os_get_acl(from_ptr);

    if os_copy(from_ptr, to_ptr, UV_FS_COPYFILE_EXCL) != 0 {
        os_free_acl(acl);
        return false;
    }

    os_set_acl(to_ptr, acl);
    os_free_acl(acl);

    true
}

/// Check if any not hidden buffer has been changed.
pub fn check_timestamps(focus: i32) -> i32 {
    if g_st().no_check_timestamps > 0 {
        return 0;
    }

    if focus != 0 && g_st().did_check_timestamps {
        g_st().need_check_timestamps = true;
        return 0;
    }

    let mut didit: i32 = 0;

    if !stuff_empty()
        || g_st().global_busy != 0
        || typebuf_typed() == 0
        || g_st().autocmd_busy
        || curbuf_mut().b_ro_locked > 0
        || g_st().allbuf_lock > 0
    {
        g_st().need_check_timestamps = true;
    } else {
        g_st().no_wait_return += 1;
        g_st().did_check_timestamps = true;
        fio_st().already_warned = false;
        let mut buf_opt = unsafe { g_st().firstbuf.as_mut() };
        while let Some(buf) = buf_opt {
            let next_ptr = buf.b_next;
            if buf.b_nwindows > 0 {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, Some(&mut *buf));
                let n = buf_check_timestamp(buf);
                didit = didit.max(n);
                if n > 0 && !bufref_valid(Some(&bufref)) {
                    buf_opt = unsafe { g_st().firstbuf.as_mut() };
                    continue;
                }
            }
            buf_opt = unsafe { next_ptr.as_mut() };
        }
        g_st().no_wait_return -= 1;
        g_st().need_check_timestamps = false;
        if g_st().need_wait_return && didit == 2 {
            msg_puts(c"\n");
            ui_flush();
        }
    }
    didit
}

/// Move all the lines from buffer "frombuf" to buffer "tobuf".
fn move_lines(frombuf: &mut FileBuf, tobuf: &mut FileBuf) -> bool {
    let tbuf = g_st().curbuf;
    let mut retval = true;

    g_st().curbuf = ptr::from_mut(tobuf);
    for lnum in 1..=frombuf.b_ml.ml_line_count {
        let p = crate::memory::cstr_to_cstring(ml_get_buf(frombuf, lnum));
        if !ml_append(lnum - 1, p.as_ptr().cast_mut(), 0, false) {
            retval = false;
            break;
        }
    }

    if retval {
        g_st().curbuf = ptr::from_mut(frombuf);
        let mut lnum = curbuf_mut().b_ml.ml_line_count;
        while lnum > 0 {
            if !ml_delete(lnum) {
                retval = false;
                break;
            }
            lnum -= 1;
        }
    }

    g_st().curbuf = tbuf;
    retval
}

/// Check if buffer "buf" has been changed.
pub fn buf_check_timestamp(buf: &mut FileBuf) -> i32 {
    static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    const RELOAD_NONE: i32 = 0;
    const RELOAD_NORMAL: i32 = 1;
    const RELOAD_DETECT: i32 = 2;

    let mut retval: i32 = 0;
    let mut mesg: Option<&std::ffi::CStr> = None;
    let mut mesg2: &std::ffi::CStr = c"";
    let mut helpmesg = false;
    let mut reload: i32 = RELOAD_NONE;
    let mut can_reload = false;
    let orig_size = buf.b_orig_size;
    let orig_mode = buf.b_orig_mode;

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(&mut *buf));

    if !buf.terminal.is_null()
        || buf.b_ffname.is_null()
        || buf.b_ml.ml_mfp.is_none()
        || !bt_normal(Some(&*buf))
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
                || (file_info.stat.st_mode as i32) != buf.b_orig_mode
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
            && !buf_is_changed_fn(buf)
            && file_info_ok
        {
            reload = RELOAD_NORMAL;
        } else {
            let (reason, reasonlen) = if !file_info_ok {
                (c"deleted", 7isize)
            } else if buf_is_changed_fn(buf) {
                (c"conflict", 8isize)
            } else if orig_size != buf.b_orig_size || buf_contents_changed(&mut *buf) {
                (c"changed", 7isize)
            } else if orig_mode != buf.b_orig_mode {
                (c"mode", 4isize)
            } else {
                (c"time", 4isize)
            };

            BUSY.store(true, std::sync::atomic::Ordering::Relaxed);
            set_vim_var_string(VV_FCS_REASON, Some(reason), reasonlen);
            set_vim_var_string(VV_FCS_CHOICE, Some(c""), 0);
            g_st().allbuf_lock += 1;
            let b_fname = crate::memory::cstr_as_opt_cstr(buf.b_fname);
            let mut n = apply_autocmds(
                EVENT_FILECHANGEDSHELL,
                b_fname,
                b_fname,
                false,
                Some(&mut *buf),
            );
            g_st().allbuf_lock -= 1;
            BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
            if n {
                if !bufref_valid(Some(&bufref)) {
                    emsg(tr(c"E246: FileChangedShell autocommand deleted buffer"));
                }
                let s = crate::memory::cstr_from_ptr(get_vim_var_str(VV_FCS_CHOICE));
                if s == c"reload" && crate::memory::cstr_first_byte(reason) != b'd' {
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
                if crate::memory::cstr_first_byte(reason) == b'd' {
                    if prev_b_mtime != -1 {
                        mesg = Some(tr(c"E211: File \"%s\" no longer available"));
                    }
                } else {
                    helpmesg = true;
                    can_reload = true;

                    let reason_bytes = crate::memory::cstr_as_bytes(reason);
                    if reason_bytes.get(2) == Some(&b'n') {
                        mesg = Some(tr(
                            c"W12: Warning: File \"%s\" has changed and the buffer was changed in Vim as well",
                        ));
                        mesg2 = tr(c"See \":help W12\" for more info.");
                    } else if reason_bytes.get(1) == Some(&b'h') {
                        mesg = Some(tr(
                            c"W11: Warning: File \"%s\" has changed since editing started",
                        ));
                        mesg2 = tr(c"See \":help W11\" for more info.");
                    } else if crate::memory::cstr_first_byte(reason) == b'm' {
                        mesg = Some(tr(
                            c"W16: Warning: Mode of file \"%s\" has changed since editing started",
                        ));
                        mesg2 = tr(c"See \":help W16\" for more info.");
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
        mesg = Some(tr(
            c"W13: Warning: File \"%s\" has been created after editing started",
        ));
        buf.b_flags |= BF_NEW_W;
        can_reload = true;
    }

    if let Some(mesg) = mesg {
        let path = home_replace_save(Some(buf), buf.b_fname);
        if !helpmesg {
            mesg2 = c"";
        }
        let tbufsize = path.as_bytes().len() + mesg.to_bytes().len() + mesg2.to_bytes().len() + 3;
        let mut tbuf_vec = vec![0u8; tbufsize];
        let tbuf = tbuf_vec.as_mut_ptr();
        let tbuflen = vim_snprintf!(tbuf.cast(), tbufsize, mesg, path.as_ptr());
        set_vim_var_string(
            VV_WARNINGMSG,
            Some(crate::memory::cstr_until_nul(&tbuf_vec)),
            tbuflen as isize,
        );
        if can_reload {
            if !mesg2.to_bytes().is_empty() {
                vim_snprintf!(
                    tbuf.wrapping_offset(tbuflen as isize).cast(),
                    tbufsize - (tbuflen as usize),
                    c"\n%s",
                    mesg2,
                );
            }
            match do_dialog(
                VIM_WARNING,
                Some(tr(c"Warning")),
                crate::memory::cstr_from_ptr(tbuf.cast()),
                tr(c"&OK\n&Load File\nLoad File &and Options"),
                1,
                None,
                1,
            ) {
                2 => reload = RELOAD_NORMAL,
                3 => reload = RELOAD_DETECT,
                _ => {}
            }
        } else if g_st().state_global > MODE_NORMAL_BUSY
            || (g_st().state_global & MODE_CMDLINE) != 0
            || fio_st().already_warned
        {
            if !mesg2.to_bytes().is_empty() {
                vim_snprintf!(
                    tbuf.wrapping_offset(tbuflen as isize).cast(),
                    tbufsize - (tbuflen as usize),
                    c"; %s",
                    mesg2,
                );
            }
            emsg(tbuf.cast());
            retval = 2;
        } else {
            if !g_st().autocmd_busy {
                msg_start();
                msg_puts_hl(tbuf.cast(), HLF_E, true);
                if !mesg2.to_bytes().is_empty() {
                    msg_puts_hl(mesg2, HLF_W, true);
                }
                msg_clr_eos();
                msg_end();
                if g_st().emsg_silent == 0 && !g_st().in_assert_fails && !ui_has(K_UIMESSAGES) {
                    msg_delay(1004, true);
                    g_st().redraw_cmdline = false;
                }
            }
            fio_st().already_warned = true;
        }

        drop(tbuf_vec);
        drop(path);
    }

    if reload != RELOAD_NONE {
        buf_reload(&mut *buf, orig_mode, reload == RELOAD_DETECT);
        if bufref_valid(Some(&bufref)) && buf.b_p_udf != 0 && !buf.b_ffname.is_null() {
            let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
            u_compute_hash(buf, &mut hash);
            u_write_undo(None, false, buf, &hash);
        }
    }

    if bufref_valid(Some(&bufref)) && retval != 0 {
        let b_fname = crate::memory::cstr_as_opt_cstr(buf.b_fname);
        apply_autocmds(
            EVENT_FILECHANGEDSHELLPOST,
            b_fname,
            b_fname,
            false,
            Some(&mut *buf),
        );
    }
    retval
}

/// Reload a buffer that is already loaded.
pub fn buf_reload(buf: &mut FileBuf, orig_mode: i32, reload_options: bool) {
    let mut ea: Exarg = Exarg::new();
    let old_ro = buf.b_p_ro;
    let savebuf: *mut FileBuf;
    let mut bufref: Bufref = Bufref::new();
    let mut saved = OK;
    let mut aco: CtxSwitch = CtxSwitch::new();
    let mut flags = READ_NEW;

    ctx_switch(&mut aco, None, None, Some(&mut *buf), 0);

    mc_buf_clear(&mut *buf);

    let _ea_cmd = if !reload_options {
        Some(prep_exarg(&mut ea, &*buf))
    } else {
        None
    };

    let old_cursor = curwin_mut().w_cursor;
    let old_topline = curwin_mut().w_topline;

    if opt_vars().p_ur < 0 || i64::from(curbuf_mut().b_ml.ml_line_count) <= opt_vars().p_ur {
        u_sync(false);
        saved = if u_savecommon(
            curbuf_mut(),
            0,
            curbuf_mut().b_ml.ml_line_count + 1,
            0,
            true,
        ) {
            OK
        } else {
            FAIL
        };
        flags |= READ_KEEP_UNDO;
    }

    if buf_is_empty(curbuf_mut()) || saved == FAIL {
        savebuf = ptr::null_mut();
    } else {
        savebuf = buflist_new(None, None, 1, BLN_DUMMY).map_or(ptr::null_mut(), ptr::from_mut);
        set_bufref(&mut bufref, unsafe { savebuf.as_mut() });
        if !savebuf.is_null() && ptr::eq(buf, g_st().curbuf) {
            g_st().curbuf = savebuf;
            curwin_mut().w_buffer = savebuf;
            saved = if ml_open(curbuf_mut()) { OK } else { FAIL };
            g_st().curbuf = buf;
            curwin_mut().w_buffer = buf;
        }
        if savebuf.is_null()
            || saved == FAIL
            || !ptr::eq(buf, g_st().curbuf)
            || !move_lines(&mut *buf, unsafe { &mut *savebuf })
        {
            semsg!(
                tr(c"E462: Could not prepare for reloading \"%s\""),
                buf.b_fname,
            );
            saved = FAIL;
        }
    }

    if saved == OK {
        curbuf_mut().b_flags |= BF_CHECK_RO;
        curbuf_mut().b_keep_filetype = true;
        if readfile(
            crate::memory::cstr_as_opt_cstr(buf.b_ffname),
            crate::memory::cstr_as_opt_cstr(buf.b_fname),
            0,
            0,
            MAXLNUM,
            Some(&ea),
            flags,
            shortmess(K_SHM_FILEINFO),
        ) != OK
        {
            if !aborting() {
                semsg!(tr(c"E321: Could not reload \"%s\""), buf.b_fname,);
            }
            if bufref_valid(Some(&bufref))
                && let Some(sb) = bufref.buf_opt()
                && ptr::eq(buf, g_st().curbuf)
            {
                while !buf_is_empty(curbuf_mut()) {
                    if !ml_delete(buf.b_ml.ml_line_count) {
                        break;
                    }
                }
                move_lines(sb, buf);
            }
        } else if ptr::eq(buf, g_st().curbuf) {
            unchanged(&mut *buf, true, true);
            if (flags & READ_KEEP_UNDO) == 0 {
                u_clearallandblockfree(&mut *buf);
            } else {
                u_unchanged(curbuf_mut());
            }
            buf_updates_unload(curbuf_mut(), true);
            curbuf_mut().b_mod_set = true;
        }
    }
    drop(_ea_cmd);

    if !savebuf.is_null() && bufref_valid(Some(&bufref)) {
        wipe_buffer(unsafe { &mut *savebuf }, false);
    }

    diff_invalidate(curbuf_mut());

    curwin_mut().w_topline = old_topline.min(curbuf_mut().b_ml.ml_line_count);
    curwin_mut().w_cursor = old_cursor;
    check_cursor(curwin_mut());
    update_topline(curwin_mut());
    curbuf_mut().b_keep_filetype = false;

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&*tp) {
            if wp.w_buffer == curwin_mut().w_buffer && !foldmethod_is_manual(wp) {
                fold_update_all(wp);
            }
        }
    }

    if orig_mode == curbuf_mut().b_orig_mode {
        curbuf_mut().b_p_ro |= old_ro;
    }

    do_modelines(0);

    ctx_restore(&mut aco);
}

pub fn buf_store_file_info(buf: &mut FileBuf, file_info: &mut FileInfo) {
    buf.b_mtime = file_info.stat.st_mtim.tv_sec as _;
    buf.b_mtime_ns = file_info.stat.st_mtim.tv_nsec as _;
    buf.b_orig_size = os_fileinfo_size(file_info);
    buf.b_orig_mode = file_info.stat.st_mode as i32;
}

/// Adjust the line with missing eol, used for the next write.
pub fn write_lnum_adjust(offset: Linenr) {
    if curbuf_mut().b_no_eol_lnum != 0 {
        curbuf_mut().b_no_eol_lnum += offset;
    }
}

/// Creates a directory for private use by this instance of Nvim.
fn vim_mktempdir() {
    #[cfg(windows)]
    const TEMP_DIRS: [&CStr; 5] = [c"$TMPDIR", c"$TMP", c"$TEMP", c"$USERPROFILE", c""];
    #[cfg(not(windows))]
    const TEMP_DIRS: [&CStr; 4] = [c"$TMPDIR", c"/tmp", c".", c"~"];

    let mut tmp: [u8; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
    let mut path: [u8; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
    let mut user: [u8; 40] = [0; 40];

    os_get_username(&mut user);
    for b in &mut user {
        if *b == b'/' || *b == b'\\' {
            *b = b'_';
        }
    }

    let umask_save = call_umask(0o077);
    for temp_dir in TEMP_DIRS {
        let mut tmplen = expand_env(
            temp_dir.as_ptr().cast_mut(),
            tmp.as_mut_ptr(),
            (TEMP_FILE_PATH_MAXLEN - 64) as i32,
        );
        if !os_isdir(tmp.as_ptr().cast()) {
            if crate::memory::cstr_as_opt_bytes(temp_dir.as_ptr()) == Some(b"$TMPDIR") {
                if !os_env_exists(c"TMPDIR", true) {
                    logmsg!(
                        LOG_DEBUG,
                        None,
                        Some(c"vim_mktempdir"),
                        0,
                        true,
                        c"$TMPDIR is unset",
                    );
                } else {
                    logmsg!(
                        LOG_WARN,
                        None,
                        Some(c"vim_mktempdir"),
                        0,
                        true,
                        c"$TMPDIR tempdir not a directory (or does not exist): \"%s\"",
                        tmp.as_ptr(),
                    );
                }
            }
            continue;
        }

        if after_pathsep(
            tmp.as_ptr().cast(),
            tmp.as_ptr().wrapping_add(tmplen).cast(),
        ) == 0
        {
            tmplen += vim_snprintf!(
                tmp.as_mut_ptr().wrapping_add(tmplen).cast(),
                tmp.len() - tmplen,
                c"%s",
                PATHSEPSTR,
            ) as usize;
            debug_assert!(tmplen < tmp.len());
        }
        tmplen += vim_snprintf!(
            tmp.as_mut_ptr().wrapping_add(tmplen).cast(),
            tmp.len() - tmplen,
            c"nvim.%s",
            user.as_ptr(),
        ) as usize;
        debug_assert!(tmplen < tmp.len());
        os_mkdir(tmp.as_ptr().cast(), 0o700);
        let owned = os_file_owned(tmp.as_ptr().cast());
        let isdir = os_isdir(tmp.as_ptr().cast());
        #[cfg(unix)]
        let perm = os_getperm(tmp.as_ptr().cast());
        #[cfg(unix)]
        let valid = isdir && owned && 0o700 == (perm & 0o777);
        #[cfg(not(unix))]
        let valid = isdir && owned;
        if valid {
            if after_pathsep(
                tmp.as_ptr().cast(),
                tmp.as_ptr().wrapping_add(tmplen).cast(),
            ) == 0
            {
                tmplen += vim_snprintf!(
                    tmp.as_mut_ptr().wrapping_add(tmplen).cast(),
                    tmp.len() - tmplen,
                    c"%s",
                    PATHSEPSTR,
                ) as usize;
                debug_assert!(tmplen < tmp.len());
            }
        } else {
            if !owned {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_mktempdir"),
                    0,
                    true,
                    c"tempdir root not owned by current user (%s): %s",
                    user.as_ptr(),
                    tmp.as_ptr(),
                );
            } else if !isdir {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_mktempdir"),
                    0,
                    true,
                    c"tempdir root not a directory: %s",
                    tmp.as_ptr(),
                );
            }
            #[cfg(unix)]
            if 0o700 != (perm & 0o777) {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_mktempdir"),
                    0,
                    true,
                    c"tempdir root has invalid permissions (%o): %s",
                    perm,
                    tmp.as_ptr(),
                );
            }
            tmplen -= crate::memory::cstr_len(user.as_ptr().cast());
            tmp[tmplen] = 0;
        }

        tmplen += vim_snprintf!(
            tmp.as_mut_ptr().wrapping_add(tmplen).cast(),
            tmp.len() - tmplen,
            c"XXXXXX",
        ) as usize;
        debug_assert!(tmplen < tmp.len());
        let _ = tmplen;
        let r = os_mkdtemp(tmp.as_ptr().cast(), &mut path);
        if r != 0 {
            logmsg!(
                LOG_WARN,
                None,
                Some(c"vim_mktempdir"),
                0,
                true,
                c"tempdir create failed: %s: %s",
                unsafe { uv_strerror(r) },
                tmp.as_ptr(),
            );
            continue;
        }

        if vim_settempdir(crate::memory::cstr_until_nul(&path)) {
            break;
        }
        os_rmdir(path.as_ptr().cast());
    }
    call_umask(umask_save);
}

/// Core part of "readdir()" function.
pub fn readdir_core(
    entries: &mut Vec<std::ffi::CString>,
    path: &CStr,
    mut context: Option<&mut crate::eval::typval::TypVal>,
) -> bool {
    let path_ptr = path.as_ptr();
    entries.clear();

    let mut dir = Directory::new();
    if !os_scandir(&mut dir, path_ptr) {
        smsg!(0, tr(E_NOTOPEN), path_ptr);
        return false;
    }

    while let Some(p) = os_scandir_next(&raw mut dir) {
        let p_bytes = p.to_bytes();

        let mut ignore = p_bytes == b"." || p_bytes == b"..";
        if !ignore && let Some(ref mut ctx) = context {
            let r = crate::eval::fs::readdir_checkitem(ctx, p);
            if r < 0 {
                break;
            }
            if r == 0 {
                ignore = true;
            }
        }

        if !ignore {
            entries.push(p.to_owned());
        }
    }

    os_closedir(&mut dir);

    entries.sort_unstable();

    true
}

/// Delete "name" and everything in it, recursively.
pub fn delete_recursive(name: &CStr) -> i32 {
    let mut result: i32 = 0;

    if os_isrealdir(name.as_ptr()) {
        let exp = name.to_owned();
        let mut entries: Vec<std::ffi::CString> = Vec::new();
        if readdir_core(&mut entries, &exp, None) {
            let namebuff_ptr = g_st().name_buff.as_mut_ptr();
            let len = vim_snprintf!(namebuff_ptr, MAXPATHL, c"%s/", exp.as_ptr());

            for entry in &entries {
                vim_snprintf!(
                    namebuff_ptr.wrapping_offset(len as isize),
                    MAXPATHL - (len as usize),
                    c"%s",
                    entry.as_ptr(),
                );
                let sub_cstr = unsafe { CStr::from_ptr(g_st().name_buff.as_ptr()) };
                if delete_recursive(sub_cstr) != 0 {
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
        result = if os_remove(name.as_ptr()) == 0 { 0 } else { -1 };
    }

    result
}

#[cfg(unix)]
fn vim_opentempdir() {
    if !fio_st().vim_tempdir_dp.is_null() {
        return;
    }

    let Some(ref td) = fio_st().vim_tempdir else {
        return;
    };
    let dp = unsafe { opendir(td.as_ptr().cast()) };
    if dp.is_null() {
        return;
    }

    fio_st().vim_tempdir_dp = dp;

    unsafe { flock(dirfd(fio_st().vim_tempdir_dp), LOCK_SH) };
}

#[cfg(unix)]
fn vim_closetempdir() {
    if fio_st().vim_tempdir_dp.is_null() {
        return;
    }

    unsafe { closedir(fio_st().vim_tempdir_dp) };

    fio_st().vim_tempdir_dp = ptr::null_mut();
}

/// Delete the temp directory and all files it contains.
pub fn vim_deltempdir() {
    let Some(td) = fio_st().vim_tempdir.take() else {
        return;
    };

    #[cfg(unix)]
    vim_closetempdir();

    let mut td_vec: Vec<u8> = td.into_bytes_with_nul();
    let tail_ptr = path_tail(crate::memory::cstr_until_nul(&td_vec));
    let tail_off = crate::memory::ptr_offset_from(tail_ptr, td_vec.as_ptr().cast()) as usize;
    if tail_off > 0 {
        td_vec[tail_off - 1] = 0;
    }
    delete_recursive(crate::memory::cstr_until_nul(&td_vec));
}

/// Gets path to Nvim's own temp dir (ending with slash).
pub fn vim_gettempdir() -> Option<&'static CStr> {
    if fio_st()
        .vim_tempdir
        .as_ref()
        .is_none_or(|td| !os_isdir(td.as_ptr()))
    {
        if let Some(current_td) = fio_st().vim_tempdir.take() {
            fio_st().notfound += 1;
            if fio_st().notfound == 1 {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_gettempdir"),
                    0,
                    true,
                    c"tempdir disappeared (antivirus or broken cleanup job?): %s",
                    current_td.as_ptr(),
                );
            }
            if fio_st().notfound > 1 {
                msg_schedule_semsg!(c"E5431: tempdir disappeared (%d times)", fio_st().notfound);
            }
        }
        vim_mktempdir();
    }
    fio_st().vim_tempdir.as_deref()
}

/// Sets Nvim's own temporary directory name to `tempdir`.
fn vim_settempdir(tempdir: &CStr) -> bool {
    let mut buf_vec: Vec<u8> = vec![0; MAXPATHL + 2];

    vim_full_name(
        tempdir.as_ptr(),
        buf_vec.as_mut_ptr(),
        MAXPATHL,
        cfg!(windows),
    );

    let mut buflen = crate::memory::cstr_until_nul(&buf_vec).to_bytes().len();
    if after_pathsep(buf_vec.as_ptr().cast(), buf_vec[buflen..].as_ptr().cast()) == 0 {
        let sep_bytes = PATHSEPSTR.to_bytes_with_nul();
        buf_vec[buflen..(buflen + sep_bytes.len())].copy_from_slice(sep_bytes);
        buflen += sep_bytes.len() - 1;
    }
    fio_st().vim_tempdir = Some(std::ffi::CString::new(&buf_vec[..buflen]).unwrap_or_default());
    #[cfg(unix)]
    vim_opentempdir();
    true
}

/// Return a unique name that can be used for a temp file.
pub fn vim_tempname() -> Option<std::ffi::CString> {
    static TEMP_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let tempdir = vim_gettempdir()?;

    let mut templ: [u8; TEMP_FILE_PATH_MAXLEN] = [0; TEMP_FILE_PATH_MAXLEN];
    let count = TEMP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _ = vim_snprintf!(
        templ.as_mut_ptr().cast(),
        TEMP_FILE_PATH_MAXLEN,
        c"%s%llu",
        tempdir.as_ptr(),
        count,
    );
    Some(crate::memory::cstr_to_cstring(templ.as_ptr().cast()))
}

/// Tries matching a filename with a "pattern" ("prog" is None), or use the
/// precompiled regprog "prog" ("pattern" is None).
pub fn match_file_pat(
    pattern: Option<&CStr>,
    prog: Option<&Regprog>,
    fname: Option<&CStr>,
    sfname: Option<&CStr>,
    tail: &CStr,
    allow_dirs: i32,
) -> bool {
    let mut regmatch: Regmatch = Regmatch::new();
    let mut result = false;

    regmatch.rm_ic = opt_vars().p_fic != 0;
    if let Some(prog) = prog {
        regmatch.borrow_prog(Some(prog));
    } else {
        regmatch.set_prog(vim_regcomp(pattern.unwrap_or(c""), RE_MAGIC));
    }

    if !regmatch.regprog.is_null()
        && ((allow_dirs != 0
            && (vim_regexec(&mut regmatch, fname.unwrap_or(c""), 0)
                || sfname.is_some_and(|sf| vim_regexec(&mut regmatch, sf, 0))))
            || (allow_dirs == 0 && vim_regexec(&mut regmatch, tail, 0)))
    {
        result = true;
    }

    if prog.is_none() {
        regmatch.free_prog();
    }
    result
}

/// Check if a file matches with a pattern in "list".
pub fn match_file_list(list: &CStr, sfname: Option<&CStr>, ffname: &CStr) -> bool {
    let sfname_ptr = sfname.map_or(ptr::null(), CStr::as_ptr);
    let tail = unsafe { CStr::from_ptr(path_tail(sfname_ptr)) };

    let mut p = list.to_bytes();
    while !p.is_empty() {
        let mut buf: [u8; MAXPATHL] = [0; MAXPATHL];
        let part_len = copy_option_part(&mut p, &mut buf, c",");
        let mut allow_dirs = false;
        let Some(regpat) = file_pat_to_reg_pat(&buf[..part_len], Some(&mut allow_dirs), 0) else {
            break;
        };
        let matched = match_file_pat(
            Some(&regpat),
            None,
            Some(ffname),
            sfname,
            tail,
            i32::from(allow_dirs),
        );
        if matched {
            return true;
        }
    }
    false
}

/// Convert the given pattern "pat" which has shell style wildcards in it, into
/// a regular expression, and return the result in allocated memory.
pub fn file_pat_to_reg_pat(
    pat_slice: &[u8],
    mut allow_dirs: Option<&mut bool>,
    no_bslash: i32,
) -> Option<std::ffi::CString> {
    if let Some(ref mut ad) = allow_dirs {
        **ad = false;
    }

    let pat_len = pat_slice.len();
    if pat_len == 0 {
        return Some(c"^$".to_owned());
    }

    let mut size: usize = 2;
    for &b in pat_slice {
        match b {
            b'*' | b'.' | b',' | b'{' | b'}' | b'~' => size += 2,
            #[cfg(windows)]
            b'\\' | b'/' => size += 4,
            _ => size += 1,
        }
    }
    let mut reg_pat_vec: Vec<u8> = Vec::with_capacity(size + 1);

    let mut start_idx = 0usize;
    if pat_slice[0] == b'*' {
        while start_idx + 1 < pat_len && pat_slice[start_idx] == b'*' {
            start_idx += 1;
        }
    } else {
        reg_pat_vec.push(b'^');
    }

    let mut end_idx = pat_len - 1;
    let mut add_dollar = true;
    if end_idx >= start_idx && pat_slice[end_idx] == b'*' {
        while end_idx > start_idx && pat_slice[end_idx] == b'*' {
            end_idx -= 1;
        }
        add_dollar = false;
    }

    let mut nested: i32 = 0;
    let mut idx = start_idx;
    while idx <= end_idx && pat_slice[idx] != 0 && nested >= 0 {
        match pat_slice[idx] {
            b'*' => {
                reg_pat_vec.extend_from_slice(b".*");
                while pat_slice.get(idx + 1) == Some(&b'*') {
                    idx += 1;
                }
            }
            b'.' | b'~' => {
                reg_pat_vec.push(b'\\');
                reg_pat_vec.push(pat_slice[idx]);
            }
            b'?' => {
                reg_pat_vec.push(b'.');
            }
            b'\\' => {
                let next_ch = pat_slice.get(idx + 1).copied().unwrap_or(0);
                if next_ch == 0 {
                    break;
                }
                #[cfg(windows)]
                if no_bslash == 0
                    && (vim_isfilec(i32::from(next_ch)) || next_ch == b'*' || next_ch == b'?')
                    && next_ch != b'+'
                {
                    reg_pat_vec.extend_from_slice(b"[\\/]");
                    if let Some(ref mut ad) = allow_dirs {
                        **ad = true;
                    }
                    idx += 1;
                    continue;
                }
                idx += 1;
                let ch = next_ch;
                if ch == b'?' && (!BACKSLASH_IN_FILENAME_BOOL || no_bslash != 0) {
                    reg_pat_vec.push(b'?');
                } else if ch == b','
                    || ch == b'%'
                    || ch == b'#'
                    || ascii_isspace(ch as i32)
                    || ch == b'{'
                    || ch == b'}'
                {
                    reg_pat_vec.push(ch);
                } else if ch == b'\\'
                    && pat_slice.get(idx + 1) == Some(&b'\\')
                    && pat_slice.get(idx + 2) == Some(&b'{')
                {
                    reg_pat_vec.extend_from_slice(b"\\{");
                    idx += 2;
                } else {
                    if vim_ispathsep(i32::from(ch))
                        && (!BACKSLASH_IN_FILENAME_BOOL || no_bslash != 0 || ch != b'\\')
                        && let Some(ref mut ad) = allow_dirs
                    {
                        **ad = true;
                    }
                    reg_pat_vec.push(b'\\');
                    reg_pat_vec.push(ch);
                }
            }
            #[cfg(windows)]
            b'/' => {
                reg_pat_vec.extend_from_slice(b"[\\/]");
                if let Some(ref mut ad) = allow_dirs {
                    **ad = true;
                }
            }
            b'{' => {
                reg_pat_vec.extend_from_slice(b"\\(");
                nested += 1;
            }
            b'}' => {
                reg_pat_vec.extend_from_slice(b"\\)");
                nested -= 1;
            }
            b',' => {
                if nested != 0 {
                    reg_pat_vec.extend_from_slice(b"\\|");
                } else {
                    reg_pat_vec.push(b',');
                }
            }
            b => {
                if vim_ispathsep(i32::from(b))
                    && let Some(ref mut ad) = allow_dirs
                {
                    **ad = true;
                }
                reg_pat_vec.push(b);
            }
        }
        idx += 1;
    }
    if add_dollar {
        reg_pat_vec.push(b'$');
    }
    reg_pat_vec.push(0);
    if nested != 0 {
        if nested < 0 {
            emsg(tr(c"E219: Missing {."));
        } else {
            emsg(tr(c"E220: Missing }."));
        }
        return None;
    }
    Some(crate::memory::cstring_until_nul(&reg_pat_vec))
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

pub fn vim_fprintf_fmt(
    stream_file: &crate::os::fs::CFile,
    fmt: &CStr,
    args: &[crate::strings::FmtVal],
) -> i32 {
    let stream = stream_file.as_ptr();
    let fmt_bytes = fmt.to_bytes();
    if args.is_empty() && !fmt_bytes.contains(&b'%') {
        if fmt_bytes.is_empty() {
            return 0;
        }
        return if unsafe { fwrite(fmt_bytes.as_ptr().cast(), 1, fmt_bytes.len(), stream) }
            == fmt_bytes.len()
        {
            fmt_bytes.len() as i32
        } else {
            -1
        };
    }
    let mut stack_buf = [0u8; 1024];
    let needed = crate::strings::vim_snprintf_fmt(
        stack_buf.as_mut_ptr().cast(),
        stack_buf.len(),
        fmt.as_ptr(),
        args,
    );
    if needed < 0 {
        return -1;
    }
    let len = needed as usize;
    if len < stack_buf.len() {
        if len == 0 || unsafe { fwrite(stack_buf.as_ptr().cast(), 1, len, stream) } == len {
            needed
        } else {
            -1
        }
    } else {
        let mut heap_buf = vec![0u8; len + 1];
        let actual = crate::strings::vim_snprintf_fmt(
            heap_buf.as_mut_ptr().cast(),
            heap_buf.len(),
            fmt.as_ptr(),
            args,
        );
        if actual < 0 {
            return -1;
        }
        let alen = actual as usize;
        if alen == 0 || unsafe { fwrite(heap_buf.as_ptr().cast(), 1, alen, stream) } == alen {
            actual
        } else {
            -1
        }
    }
}

pub fn vim_printf_fmt(fmt: &CStr, args: &[crate::strings::FmtVal]) -> i32 {
    vim_fprintf_fmt(&crate::log::log_get_stdout(), fmt, args)
}

#[macro_export]
macro_rules! fprintf {
    ($stream:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::fileio::vim_fprintf_fmt(
            &$stream,
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
        assert!(!is_dev_fd_file(b"/dev/fd/0"));
        assert!(!is_dev_fd_file(b"/dev/fd/1"));
        assert!(!is_dev_fd_file(b"/dev/fd/2"));
        assert!(is_dev_fd_file(b"/dev/fd/3"));
        assert!(is_dev_fd_file(b"/dev/fd/10"));
        assert!(!is_dev_fd_file(b"/dev/fd/10a"));
        assert!(!is_dev_fd_file(b"/tmp/foo"));
    }

    #[test]
    fn test_check_for_bom() {
        let mut len = 0;
        let utf8_bom = [0xefu8, 0xbb, 0xbf, b'a'];
        let enc = check_for_bom(&utf8_bom, &mut len, FIO_ALL);
        assert_eq!(enc, Some(c"utf-8"));
        assert_eq!(len, 3);

        let utf16le_bom = [0xffu8, 0xfe, b'a', 0];
        let enc = check_for_bom(&utf16le_bom, &mut len, FIO_ALL);
        assert_eq!(enc, Some(c"utf-16le"));
        assert_eq!(len, 2);

        let ucs4le_bom = [0xffu8, 0xfe, 0x00, 0x00];
        let enc = check_for_bom(&ucs4le_bom, &mut len, FIO_ALL);
        assert_eq!(enc, Some(c"ucs-4le"));
        assert_eq!(len, 4);
    }

    #[test]
    fn test_file_pat_to_reg_pat() {
        let check = |pat: &CStr, expected: &CStr| {
            let res = file_pat_to_reg_pat(pat.to_bytes(), None, 1);
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
        {
            if crate::memory::cstr_is_empty(crate::eval::vars::get_vim_var_str(
                crate::eval::vars::VV_PROGPATH,
            )) {
                crate::eval::vars::set_vim_var_string(
                    crate::eval::vars::VV_PROGPATH,
                    Some(c"nvim"),
                    -1,
                );
            }
            vim_deltempdir();
            let dir1_cstr = vim_gettempdir().unwrap();
            let dir1 = dir1_cstr.to_str().unwrap().to_string();
            assert!(!dir1.is_empty());
            let dir2 = vim_gettempdir().unwrap().to_str().unwrap().to_string();
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
