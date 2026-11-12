#![forbid(unsafe_code)]

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
    K_UIMESSAGES, MAXLNUM, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC, SEA_QUIT, bt_dontwrite,
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
    BAD_DROP, BAD_KEEP, VV_FCS_CHOICE, VV_FCS_REASON, VV_TESTING, VV_WARNINGMSG, eval_charconvert,
    get_vim_var_nr, get_vim_var_str, set_vim_var_string,
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
    utf_ptr2char, utf_ptr2len_len,
};
use crate::mcursor::mc_buf_clear;
use crate::memfile::mf_fullname;
use crate::memline::{
    check_need_swap, ml_append, ml_delete, ml_get, ml_get_buf, ml_get_len, ml_open,
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
use crate::strings::{format_to_buf, ngettext, tr};
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

static E_AUCHANGEDBUF: &CStr = c"E812: Autocommands changed buffer or buffer name";
static MSG_IS_A_DIRECTORY: &CStr = c"is a directory";

struct FileioModuleState {
    already_warned: std::cell::Cell<bool>,
    vim_tempdir: std::cell::RefCell<Option<std::ffi::CString>>,
    #[cfg(unix)]
    vim_tempdir_dp: std::cell::RefCell<Option<crate::os::fs::OsLockedDir>>,
    notfound: std::cell::Cell<i32>,
}

impl FileioModuleState {
    const fn new() -> Self {
        Self {
            already_warned: std::cell::Cell::new(false),
            vim_tempdir: std::cell::RefCell::new(None),
            #[cfg(unix)]
            vim_tempdir_dp: std::cell::RefCell::new(None),
            notfound: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static FIO_STATE: FileioModuleState = const { FileioModuleState::new() };
}

#[inline]
fn with_fio<R>(f: impl FnOnce(&FileioModuleState) -> R) -> R {
    FIO_STATE.with(f)
}

use crate::arglist::ML_EMPTY;
use crate::eval::funcs::VIM_WARNING;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_cmds::{BL_FIX, BL_WHITE, K_SHM_OVERALL};
use crate::ex_docmd::UNDO_HASH_SIZE;
use crate::option::OPT_LOCAL;
use crate::os::fs::MAXPATHL;
use crate::os::fs::{s_isdir, s_isfifo, s_isreg, s_issock};

pub(crate) const SEEK_SET: i32 = 0;
pub(crate) const SEEK_END: i32 = 2;

#[cfg(windows)]
pub use crate::os::fs::{_get_osfhandle, _getdrive, _open_osfhandle, _setmode};
pub(crate) use crate::os::fs::{c_fdopen, c_fgets, c_fopen, c_fputs};
pub use crate::os::fs::{
    call_iconv, call_iconv_close, call_iconv_open, call_umask, close, dup2, fclose, feof, ferror,
    fflush, fgetc, fileno, fputc, fread, freopen, fwrite, read_eintr, setbuf, setvbuf, ungetc,
    vim_fseek, vim_ftell, vim_lseek, write_eintr,
};
#[cfg(not(windows))]
pub use crate::os::fs::{
    dup, fcntl, getgid, getuid, kill, os_dupfd_cloexec, os_readlink, os_symlink,
};
#[cfg(unix)]
pub use crate::os::fs::{getpgid, getpgrp, getppid, tcdrain};

/// Shows a message about file `name`, e.g. `"foo.txt" [New]`.
fn filemess(buf: Option<&FileBuf>, name: Option<&CStr>, s: &CStr, msg_id: Option<&CStr>) {
    let prev_msg_col = g_st().msg_col.get();

    if g_st().msg_silent.get() != 0 {
        return;
    }

    add_quoted_fname(&mut g_st().iobuff_bytes_mut()[..IOSIZE - 100], buf, name);

    crate::memory::append_bytes_to_c_buf(&mut *g_st().iobuff_bytes_mut(), s.to_bytes());

    let msg_scroll_save = g_st().msg_scroll.get();
    if shortmess(K_SHM_OVERALL)
        && g_st().msg_listdo_overwrite.get() == 0
        && !g_st().exiting.get()
        && opt_vars().p_verbose.get() == 0
    {
        g_st().msg_scroll.set(0);
    }
    if g_st().msg_scroll.get() == 0 {
        msg_check_for_delay(false);
    }
    msg_start();
    if prev_msg_col != 0 && g_st().msg_col.get() == 0 {
        msg_putchar(b'\r' as i32);
    }
    g_st().msg_scroll.set(msg_scroll_save);
    g_st().msg_scrolled_ign.set(true);
    if let Some(msg_id) = msg_id {
        let mut iob = *g_st().iobuff.borrow();
        msg_with_id(
            Some(&mut iob),
            msg_id,
            c"bufwrite",
            None,
            0,
            false,
            true,
            false,
        );
        *g_st().iobuff.borrow_mut() = iob;
    } else {
        let mut iob = *g_st().iobuff.borrow();
        let trunc = msg_may_trunc(false, &mut iob);
        msg_outtrans(trunc, 0, false);
        *g_st().iobuff.borrow_mut() = iob;
    }
    msg_clr_eos();
    g_st().msg_scrolled_ign.set(false);
}

/// Shows file `name` before writing it, as a "bufwrite" message with `msg_id`, which buf_write()
/// replaces when done (or with a progress-message, if slow).
pub fn filemess_bufwrite(buf: Option<&FileBuf>, name: Option<&CStr>, msg_id: Option<&CStr>) {
    filemess(buf, name, c"", msg_id);
}

/// If a file read/write is slower than this, emit a progress-message.
pub fn slow_io_ns() -> u64 {
    if g_st().nvim_testing.get() && get_vim_var_nr(VV_TESTING) != 0 {
        0
    } else {
        1_000_000_000 / 2
    }
}

/// Shows the progress-message of a slow readfile(). Locks `curbuf` so Progress handlers cannot
/// accidentally unload it while it is loading.
fn readfile_progress<'a>(
    s: Option<&'a mut [u8]>,
    msg_id: &CStr,
    status: &CStr,
    percent: i32,
    hist: bool,
) -> Option<&'a CStr> {
    let buf = g_st().curbuf.get();
    buf.b_locked.set(buf.b_locked.get() + (1));

    let trunc = s.is_some();
    let rv = msg_progress(s, msg_id, status, percent, 0, hist, trunc, false);

    buf.b_locked.set(buf.b_locked.get() - (1));

    rv
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
) -> Option<bool> {
    let fname = fname_opt;
    let mut sfname = sfname_opt;
    #[cfg(unix)]
    let mut fname = fname;
    {
        let mut retval = Some(false);
        let mut fd: i32 = if g_st().stdin_fd.get() >= 0 {
            g_st().stdin_fd.get()
        } else {
            0
        };
        let newfile = (flags & READ_NEW) != 0;
        let filtering = (flags & READ_FILTER) != 0;
        let read_stdin = (flags & READ_STDIN) != 0;
        let read_buffer = (flags & READ_BUFFER) != 0;
        let read_fifo = (flags & READ_FIFO) != 0;
        let set_options = newfile || read_buffer || (eap.is_some() && eap.unwrap().read_edit);
        let mut read_buf_lnum: Linenr = 1;
        let mut read_buf_col: Colnr = 0;
        let mut lnum = from;
        let mut ptr_idx: usize = 0;
        let mut buffer_vec: Vec<u8> = Vec::new();
        let mut line_start_idx: usize = 0;
        let mut size: isize = 0;
        let mut p_idx: usize;
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
        let msg_save = g_st().msg_scroll.get();
        let mut read_no_eol_lnum: Linenr = 0;
        let mut file_rewind = false;
        let mut conv_error: Linenr = 0;
        let mut illegal_byte: Linenr = 0;
        let mut keep_dest_enc = false;
        let mut bad_char_behavior: i32 = BAD_REPLACE;
        let mut tmpname: Option<std::ffi::CString> = None;
        let mut fio_flags: i32;
        let mut fenc_owned: Option<std::ffi::CString>;
        let mut advance_fenc = false;
        let mut real_size: i32 = 0;
        let mut iconv_fd: iconv_t = -1;
        let mut did_iconv = false;
        let mut converted = false;
        let mut notconverted = false;
        let mut conv_rest: [u8; CONV_RESTLEN] = [0; CONV_RESTLEN];
        let mut conv_restlen: i32 = 0;
        let mut orig_start: CursorPos;

        curbuf_mut().b_au_did_filetype.set(false);
        curbuf_mut().b_no_eol_lnum.set(0);

        'theend: {
            if curbuf_mut().b_ffname.get().is_none()
                && !filtering
                && fname.is_some()
                && crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_FNAMER)
                && (flags & READ_DUMMY) == 0
                && !set_rw_fname(fname, sfname)
            {
                break 'theend;
            }

            let old_curbuf = g_st().curbuf.get();
            let old_b_ffname = curbuf_mut()
                .ffname_opt()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            let old_b_fname = curbuf_mut()
                .fname_opt()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            let fname_ptr = fname.map_or(ptr::null(), CStr::as_ptr);
            let sfname_ptr = sfname.map_or(ptr::null(), CStr::as_ptr);
            let using_b_ffname =
                fname_ptr == old_b_ffname.cast() || sfname_ptr == old_b_ffname.cast();
            let using_b_fname = fname_ptr == old_b_fname.cast() || sfname_ptr == old_b_fname.cast();

            g_st().need_fileinfo.set(false);

            if sfname.is_none() {
                sfname = fname;
            }
            #[cfg(unix)]
            {
                fname = sfname;
            }

            if !filtering && !read_stdin && !read_buffer {
                orig_start = curbuf_mut().b_op_start.get();

                curbuf_mut()
                    .b_op_start
                    .set_lnum(if from == 0 { 1 } else { from });
                curbuf_mut().b_op_start.set_col(0);

                let sfname_opt = sfname;
                if newfile {
                    if apply_autocmds_exarg(
                        EVENT_BUFREADCMD,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    ) {
                        retval = Some(!aborting());

                        if retval == Some(true)
                            && !curbuf_mut().b_au_did_filetype.get()
                            && !curbuf_mut().b_p_ft.is_empty()
                        {
                            apply_autocmds(
                                EVENT_FILETYPE,
                                Some(&curbuf_mut().b_p_ft.as_cstr()),
                                curbuf_mut().fname_opt().as_deref(),
                                true,
                                curbuf_opt_mut().as_deref(),
                            );
                            if aborting() {
                                retval = Some(false);
                            }
                        }
                        if retval == Some(true) {
                            curbuf_mut()
                                .b_flags
                                .set(curbuf_mut().b_flags.get() & (!BF_NOTEDITED));
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
                    retval = Some(!aborting());
                    break 'theend;
                }

                curbuf_mut().b_op_start.set(orig_start);

                if (flags & READ_NOFILE) != 0 {
                    retval = None;
                    break 'theend;
                }
            }

            if ((shortmess(K_SHM_OVER) && g_st().msg_listdo_overwrite.get() == 0)
                || curbuf_mut().b_help.get())
                && opt_vars().p_verbose.get() == 0
            {
                g_st().msg_scroll.set(0);
            } else {
                g_st().msg_scroll.set(1);
            }

            if let Some(fname_cstr) = fname
                && !fname_cstr.is_empty()
            {
                let fname_bytes = fname_cstr.to_bytes();
                let fnamelen = fname_bytes.len();

                if fnamelen >= MAXPATHL {
                    filemess(
                        g_st().curbuf.get().opt(),
                        fname,
                        tr(c"Illegal file name"),
                        None,
                    );
                    msg_end();
                    g_st().msg_scroll.set(msg_save);
                    break 'theend;
                }

                if after_pathsep(fname_bytes) != 0 {
                    if !silent {
                        filemess(
                            g_st().curbuf.get().opt(),
                            fname,
                            tr(MSG_IS_A_DIRECTORY),
                            None,
                        );
                    }
                    msg_end();
                    g_st().msg_scroll.set(msg_save);
                    retval = None;
                    break 'theend;
                }
            }

            if !read_stdin && let Some(fname_cstr) = fname {
                perm = os_getperm(fname_cstr);
            }

            #[cfg(any(
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly",
                target_os = "macos"
            ))]
            let is_chr_dev =
                s_ischr(perm as u64) && is_dev_fd_file(fname.unwrap_or(c"").to_bytes());
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
                            g_st().curbuf.get().opt(),
                            fname,
                            tr(MSG_IS_A_DIRECTORY),
                            None,
                        );
                    }
                    retval = None;
                } else {
                    filemess(g_st().curbuf.get().opt(), fname, tr(c"is not a file"), None);
                }
                msg_end();
                g_st().msg_scroll.set(msg_save);
                break 'theend;
            }

            set_file_options(set_options, (eap.is_some()).then(|| eap.unwrap()));

            let check_readonly = newfile && (curbuf_mut().b_flags.get() & BF_CHECK_RO) != 0;
            if check_readonly && !g_st().readonlymode.get() {
                curbuf_mut().b_p_ro.set(false);
            }

            if newfile && !read_stdin && !read_buffer && !read_fifo {
                if os_fileinfo(fname.unwrap_or(c""), &mut file_info) {
                    buf_store_file_info(&curbuf_mut(), &mut file_info);
                    curbuf_mut().b_mtime_read.set(curbuf_mut().b_mtime.get());
                    curbuf_mut()
                        .b_mtime_read_ns
                        .set(curbuf_mut().b_mtime_ns.get());
                    #[cfg(unix)]
                    {
                        swap_mode = ((file_info.stat.st_mode as i32) & 0o644) | 0o600;
                    }
                } else {
                    curbuf_mut().b_mtime.set(0);
                    curbuf_mut().b_mtime_ns.set(0);
                    curbuf_mut().b_mtime_read.set(0);
                    curbuf_mut().b_mtime_read_ns.set(0);
                    curbuf_mut().b_orig_size.set(0);
                    curbuf_mut().b_orig_mode.set(0);
                }

                curbuf_mut()
                    .b_flags
                    .set(curbuf_mut().b_flags.get() & (!(BF_NEW | BF_NEW_W)));
            }

            let mut file_readonly = false;
            if !read_buffer && !read_stdin {
                if !newfile
                    || g_st().readonlymode.get()
                    || (perm & 0o222) == 0
                    || os_file_is_writable(fname.unwrap_or(c"")) == 0
                {
                    file_readonly = true;
                }
                fd = os_open(fname.unwrap_or(c""), O_RDONLY, 0);
            }

            if fd < 0 {
                g_st().msg_scroll.set(msg_save);
                if !newfile {
                    break 'theend;
                }
                if perm == UV_ENOENT {
                    curbuf_mut()
                        .b_flags
                        .set(curbuf_mut().b_flags.get() | (BF_NEW));

                    if !bt_dontwrite(Some(&crate::globals::curbuf_mut())) {
                        check_need_swap(newfile);
                        if g_st().curbuf.get() != old_curbuf
                            || (using_b_ffname
                                && old_b_ffname
                                    != curbuf_mut()
                                        .ffname_opt()
                                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()))
                            || (using_b_fname
                                && old_b_fname
                                    != curbuf_mut()
                                        .fname_opt()
                                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()))
                        {
                            emsg(tr(E_AUCHANGEDBUF));
                            break 'theend;
                        }
                    }
                    if !silent {
                        if dir_of_file_exists(fname.unwrap_or(c"")) {
                            filemess(g_st().curbuf.get().opt(), sfname, tr(c"[New]"), None);
                        } else {
                            filemess(
                                g_st().curbuf.get().opt(),
                                sfname,
                                tr(c"[New DIRECTORY]"),
                                None,
                            );
                        }
                    }
                    check_marks_read();
                    if let Some(eap) = eap {
                        set_forced_fenc(eap);
                    }
                    let sfname_opt = sfname;
                    apply_autocmds_exarg(
                        EVENT_BUFNEWFILE,
                        sfname_opt,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    );
                    save_file_ff(&curbuf_mut());

                    if !aborting() {
                        retval = Some(true);
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                let is_too_big = fd == UV_EFBIG || fd == -EOVERFLOW;
                #[cfg(not(unix))]
                let is_too_big = fd == UV_EFBIG;
                filemess(
                    g_st().curbuf.get().opt(),
                    sfname,
                    if is_too_big {
                        tr(c"[File too big]")
                    } else {
                        tr(c"[Permission Denied]")
                    },
                    None,
                );
                curbuf_mut().b_p_ro.set(true);

                break 'theend;
            }

            if (check_readonly && file_readonly) || curbuf_mut().b_help.get() {
                curbuf_mut().b_p_ro.set(true);
            }

            if set_options {
                if !read_buffer {
                    curbuf_mut().b_p_eof.set(false);
                    curbuf_mut().b_start_eof.set(false);
                    curbuf_mut().b_p_eol.set(true);
                    curbuf_mut().b_start_eol.set(true);
                }
                curbuf_mut().b_p_bomb.set(false);
                curbuf_mut().b_start_bomb.set(false);
            }

            if !bt_dontwrite(Some(&crate::globals::curbuf_mut())) {
                check_need_swap(newfile);
                if !read_stdin
                    && (g_st().curbuf.get() != old_curbuf
                        || (using_b_ffname
                            && old_b_ffname
                                != curbuf_mut()
                                    .ffname_opt()
                                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()))
                        || (using_b_fname
                            && old_b_fname
                                != curbuf_mut()
                                    .fname_opt()
                                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())))
                {
                    emsg(tr(E_AUCHANGEDBUF));
                    if !read_buffer {
                        close(fd);
                    }
                    break 'theend;
                }
                #[cfg(unix)]
                {
                    if swap_mode > 0
                        && let Some(mfp) = curbuf_mut().b_ml.mfp_opt()
                        && let Some(ref swap_fname) = mfp.mf_fname
                    {
                        let swap_cstr = swap_fname.as_c_str();
                        if (swap_mode & 0o44) == 0o40 {
                            let mut swap_info: FileInfo = FileInfo::new();
                            if os_fileinfo(swap_cstr, &mut swap_info)
                                && file_info.stat.st_gid != swap_info.stat.st_gid
                                && os_fchown(
                                    mfp.mf_fd,
                                    u32::MAX as uv_uid_t,
                                    file_info.stat.st_gid as uv_gid_t,
                                ) == -1
                            {
                                swap_mode &= 0o600;
                            }
                        }
                        os_setperm(swap_cstr, swap_mode);
                    }
                }
            }

            if g_st().swap_exists_action.get() == SEA_QUIT {
                if !read_buffer && !read_stdin {
                    close(fd);
                }
                break 'theend;
            }

            g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));

            orig_start = curbuf_mut().b_op_start.get();
            curbuf_mut()
                .b_op_start
                .set_lnum(if from == 0 { 1 } else { from });
            curbuf_mut().b_op_start.set_col(0);

            let mut try_mac = i32::from(crate::strings::vim_haschr(
                &opt_vars().p_ffs.as_cstr(),
                b'm' as i32,
            ));
            let mut try_dos = i32::from(crate::strings::vim_haschr(
                &opt_vars().p_ffs.as_cstr(),
                b'd' as i32,
            ));
            let mut try_unix = i32::from(crate::strings::vim_haschr(
                &opt_vars().p_ffs.as_cstr(),
                b'x' as i32,
            ));

            if !read_buffer {
                let m = g_st().msg_scroll.get();
                let n = g_st().msg_scrolled.get();

                if !read_stdin {
                    close(fd);
                }

                g_st().msg_scroll.set(1);
                let sfname_opt = sfname;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    );
                } else if read_stdin {
                    apply_autocmds_exarg(
                        EVENT_STDINREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    );
                } else if newfile {
                    apply_autocmds_exarg(
                        EVENT_BUFREADPRE,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
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

                try_mac = i32::from(crate::strings::vim_haschr(
                    &opt_vars().p_ffs.as_cstr(),
                    b'm' as i32,
                ));
                try_dos = i32::from(crate::strings::vim_haschr(
                    &opt_vars().p_ffs.as_cstr(),
                    b'd' as i32,
                ));
                try_unix = i32::from(crate::strings::vim_haschr(
                    &opt_vars().p_ffs.as_cstr(),
                    b'x' as i32,
                ));
                curbuf_mut().b_op_start.set(orig_start);

                if g_st().msg_scrolled.get() == n {
                    g_st().msg_scroll.set(m);
                }

                if aborting() {
                    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
                    g_st().msg_scroll.set(msg_save);
                    curbuf_mut().b_p_ro.set(true);
                    break 'theend;
                }

                if !read_stdin
                    && (g_st().curbuf.get() != old_curbuf
                        || (using_b_ffname
                            && old_b_ffname
                                != curbuf_mut()
                                    .ffname_opt()
                                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()))
                        || (using_b_fname
                            && old_b_fname
                                != curbuf_mut()
                                    .fname_opt()
                                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()))
                        || {
                            fd = os_open(fname.unwrap_or(c""), O_RDONLY, 0);
                            fd < 0
                        })
                {
                    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
                    g_st().msg_scroll.set(msg_save);
                    if fd < 0 {
                        emsg(tr(c"E200: *ReadPre autocommands made the file unreadable"));
                    } else {
                        emsg(tr(
                            c"E201: *ReadPre autocommands must not change current buffer",
                        ));
                    }
                    curbuf_mut().b_p_ro.set(true);
                    break 'theend;
                }
            }

            let wasempty = (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0;

            let show_msg = !g_st().recoverymode.get()
                && !filtering
                && (flags & READ_DUMMY) == 0
                && !read_stdin
                && !read_buffer
                && g_st().msg_silent.get() == 0;
            if show_msg && !silent {
                filemess(g_st().curbuf.get().opt(), sfname, c"", None);
            }

            let mut msg_id = [0u8; MAXPATHL + 32];
            fname_msg_id(
                &mut msg_id,
                c"nvim.bufread",
                g_st().curbuf.get().opt(),
                sfname,
            );
            let mut progress = false;
            let mut percent: i32 = -1;
            let read_start = os_hrtime();

            g_st().msg_scroll.set(0);

            let mut linecnt = curbuf_mut().b_ml.ml_line_count.get();

            if let Some(eap) = eap
                && eap.bad_char != 0
            {
                bad_char_behavior = eap.bad_char;
                if set_options {
                    curbuf_mut().b_bad_char.set(eap.bad_char);
                }
            } else {
                curbuf_mut().b_bad_char.set(0);
            }

            let fencs_rc;
            let mut fenc_next: &[u8] = b"";
            if let Some(eap) = eap
                && eap.force_enc != 0
            {
                fenc_owned = Some(enc_canonize(
                    eap.cmd.wrapping_add(eap.force_enc as usize).as_cstr(),
                ));
                keep_dest_enc = true;
            } else if curbuf_mut().b_p_bin.get() {
                fenc_owned = Some(c"".to_owned());
            } else if curbuf_mut().b_help.get() {
                fenc_next = b"latin1";
                fenc_owned = Some(c"utf-8".to_owned());
            } else if opt_vars().p_fencs.is_empty() {
                fenc_owned = Some(curbuf_mut().b_p_fenc.to_owned());
            } else {
                fencs_rc = opt_vars().p_fencs.as_bytes();
                fenc_next = &fencs_rc;
                fenc_owned = Some(next_fenc(&mut fenc_next));
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
                        curbuf_mut().b_p_bomb.set(false);
                        curbuf_mut().b_start_bomb.set(false);
                    }
                    conv_error = 0;
                }

                if keep_fileformat {
                    keep_fileformat = false;
                } else if eap.is_some() && eap.unwrap().force_ff != 0 {
                    fileformat =
                        get_fileformat_force(&curbuf_mut(), (eap.is_some()).then(|| eap.unwrap()));
                    try_unix = 0;
                    try_dos = 0;
                    try_mac = 0;
                } else if curbuf_mut().b_p_bin.get() {
                    fileformat = EOL_UNIX;
                } else if opt_vars().p_ffs.is_empty() {
                    fileformat = get_fileformat(&curbuf_mut());
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
                        fenc_owned = Some(c"".to_owned());
                    } else if !fenc_next.is_empty() {
                        fenc_owned = Some(next_fenc(&mut fenc_next));
                    } else {
                        fenc_owned = Some(c"".to_owned());
                    }
                    if let Some(ref tn) = tmpname {
                        os_remove(tn);
                        tmpname = None;
                    }
                }

                fio_flags = 0;
                let fenc_opt = fenc_owned.as_deref();
                let fenc_cstr = fenc_opt.unwrap_or(c"");
                converted = need_conversion(fenc_opt);
                if converted {
                    if fenc_opt == Some(c"ucs-bom") {
                        fio_flags = FIO_UCSBOM;
                    } else {
                        fio_flags = get_fio_flags(fenc_opt);
                    }

                    if fio_flags == 0 && !did_iconv {
                        iconv_fd =
                            my_iconv_open(c"utf-8", fenc_cstr).map_or(-1, |fd| fd as iconv_t);
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
                            tmpname =
                                readfile_charconvert(fname.unwrap_or(c""), fenc_cstr, &mut fd);
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

                let can_retry = fenc_opt.is_some_and(|s| !s.is_empty())
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
                        && curbuf_mut().b_ffname.get().is_some()
                        && curbuf_mut().b_p_udf.get()
                        && !filtering
                        && !read_fifo
                        && !read_stdin
                        && !read_buffer;
                    if read_undo_file {
                        sha_ctx = ContextSha256::new();
                    }
                }

                while !error && !g_st().got_int.get() {
                    {
                        if !skip_read {
                            size = (0x10000 + linerest).min(0x100000);
                        }

                        if size < 0 || size + linerest + 1 < 0 || linerest >= MAXCOL - size {
                            split += 1;
                            buffer_vec[ptr_idx] = b'\n';
                            size = 1;
                        } else if !skip_read {
                            let mut new_buffer_vec: Vec<u8> =
                                vec![0; (size as usize) + (linerest as usize) + 1];
                            if linerest != 0 {
                                let lr = linerest as usize;
                                new_buffer_vec[..lr]
                                    .copy_from_slice(&buffer_vec[ptr_idx - lr..ptr_idx]);
                            }
                            buffer_vec = new_buffer_vec;
                            ptr_idx = linerest as usize;
                            line_start_idx = 0;

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
                                let cr = conv_restlen as usize;
                                buffer_vec[ptr_idx..ptr_idx + cr].copy_from_slice(&conv_rest[..cr]);
                                ptr_idx += cr;
                                size -= conv_restlen as isize;
                            }

                            if read_buffer {
                                if read_buf_lnum > from {
                                    size = 0;
                                } else {
                                    let mut tlen: isize = 0;
                                    loop {
                                        let line_bytes = &ml_get(read_buf_lnum).to_bytes()
                                            [read_buf_col as usize..];
                                        let mut n =
                                            (ml_get_len(read_buf_lnum) - read_buf_col) as isize;
                                        if tlen + n + 1 > size {
                                            n = size - tlen;
                                            for &ch in &line_bytes[..n as usize] {
                                                buffer_vec[ptr_idx + (tlen as usize)] =
                                                    if ch == b'\n' { 0 } else { ch };
                                                tlen += 1;
                                            }
                                            read_buf_col += n as Colnr;
                                            break;
                                        }

                                        for &ch in &line_bytes[..n as usize] {
                                            buffer_vec[ptr_idx + (tlen as usize)] =
                                                if ch == b'\n' { 0 } else { ch };
                                            tlen += 1;
                                        }
                                        buffer_vec[ptr_idx + (tlen as usize)] = b'\n';
                                        tlen += 1;
                                        read_buf_col = 0;
                                        read_buf_lnum += 1;
                                        if read_buf_lnum > from {
                                            if !curbuf_mut().b_p_eol.get() {
                                                tlen -= 1;
                                            }
                                            size = tlen;
                                            break;
                                        }
                                    }
                                }
                            } else {
                                let read_size = size as usize;
                                let read_slice = &mut buffer_vec[ptr_idx..ptr_idx + read_size];
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
                                                curbuf_mut().b_ml.ml_line_count.get() - linecnt + 1;
                                        }
                                    } else if illegal_byte == 0 {
                                        illegal_byte =
                                            curbuf_mut().b_ml.ml_line_count.get() - linecnt + 1;
                                    }
                                    if bad_char_behavior == BAD_DROP {
                                        buffer_vec[ptr_idx - (conv_restlen as usize)] = 0;
                                        conv_restlen = 0;
                                    } else {
                                        if bad_char_behavior != BAD_KEEP
                                            && (fio_flags != 0 || iconv_fd != -1)
                                        {
                                            while conv_restlen > 0 {
                                                ptr_idx -= 1;
                                                buffer_vec[ptr_idx] = bad_char_behavior as u8;
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
                                || (!curbuf_mut().b_p_bomb.get()
                                    && tmpname.is_none()
                                    && fenc_opt
                                        .is_none_or(|s| s.is_empty() || s.to_bytes()[0] == b'u')))
                        {
                            let mut blen: i32 = 0;
                            let ccname = if size < 2 || curbuf_mut().b_p_bin.get() {
                                None
                            } else {
                                check_for_bom(
                                    &buffer_vec[ptr_idx..ptr_idx + (size as usize)],
                                    &mut blen,
                                    if fio_flags == FIO_UCSBOM {
                                        FIO_ALL
                                    } else {
                                        get_fio_flags(fenc_owned.as_deref())
                                    },
                                )
                            };
                            if ccname.is_some() {
                                filesize += i64::from(blen);
                                size -= blen as isize;
                                let b_off = blen as usize;
                                let sz = size as usize;
                                buffer_vec
                                    .copy_within(ptr_idx + b_off..ptr_idx + b_off + sz, ptr_idx);
                                if set_options {
                                    curbuf_mut().b_p_bomb.set(true);
                                    curbuf_mut().b_start_bomb.set(true);
                                }
                            }

                            if fio_flags == FIO_UCSBOM {
                                if let Some(ccname) = ccname {
                                    fenc_owned = Some(ccname.to_owned());
                                } else {
                                    advance_fenc = true;
                                }
                                skip_read = true;
                                continue 'retry;
                            }
                        }

                        ptr_idx -= conv_restlen as usize;
                        size += conv_restlen as isize;
                        conv_restlen = 0;
                        if size <= 0 {
                            break;
                        }

                        if iconv_fd != -1 {
                            let from_total = size as usize;
                            let to_total = (real_size as isize - size) as usize;
                            let orig_ptr_idx = ptr_idx;
                            ptr_idx += from_total;

                            let (conv_retry, out_written) = {
                                let (from_part, to_part) = buffer_vec
                                    [orig_ptr_idx..orig_ptr_idx + from_total + to_total]
                                    .split_at_mut(from_total);
                                let mut from_rem: &[u8] = from_part;
                                let mut out_written = 0usize;
                                let mut retry = false;

                                loop {
                                    let mut to_rem: &mut [u8] = &mut to_part[out_written..];
                                    let iconv_res =
                                        call_iconv(iconv_fd, Some(&mut from_rem), &mut to_rem);
                                    out_written = to_total - to_rem.len();
                                    if (iconv_res != usize::MAX || os_errno() == ICONV_EINVAL)
                                        && from_rem.len() <= CONV_RESTLEN
                                    {
                                        break;
                                    }
                                    if can_retry {
                                        retry = true;
                                        break;
                                    }
                                    if conv_error == 0 {
                                        conv_error =
                                            readfile_linenr(linecnt, &to_part[..out_written]);
                                    }

                                    let skipped_byte = from_rem[0];
                                    from_rem = &from_rem[1..];
                                    if bad_char_behavior == BAD_KEEP {
                                        to_part[out_written] = skipped_byte;
                                        out_written += 1;
                                    } else if bad_char_behavior != BAD_DROP {
                                        to_part[out_written] = bad_char_behavior as u8;
                                        out_written += 1;
                                    }
                                }

                                if !retry && !from_rem.is_empty() {
                                    conv_rest[..from_rem.len()].copy_from_slice(from_rem);
                                    conv_restlen = from_rem.len() as i32;
                                }

                                (retry, out_written)
                            };

                            if conv_retry {
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

                            let lr = linerest as usize;
                            line_start_idx = ptr_idx - lr;
                            buffer_vec.copy_within(0..lr, line_start_idx);
                            size = out_written as isize;
                        }

                        if fio_flags != 0 {
                            let mut u8c: u32;
                            let mut tail_opt: Option<usize> = None;

                            let mut dest_idx = ptr_idx + (real_size as usize);
                            let size_usize = size as usize;
                            if fio_flags == FIO_LATIN1 || fio_flags == FIO_UTF8 {
                                p_idx = ptr_idx + size_usize;
                                if fio_flags == FIO_UTF8 {
                                    let mut t_idx = ptr_idx + size_usize - 1;
                                    while t_idx > ptr_idx && (buffer_vec[t_idx] & 0xc0) == 0x80 {
                                        t_idx -= 1;
                                    }
                                    if t_idx + (utf_byte2len(i32::from(buffer_vec[t_idx])) as usize)
                                        > ptr_idx + size_usize
                                    {
                                        tail_opt = Some(t_idx);
                                        p_idx = t_idx;
                                    }
                                }
                            } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                p_idx = ptr_idx + (size_usize & !1);
                                if (size_usize & 1) != 0 {
                                    tail_opt = Some(p_idx);
                                }
                                if (fio_flags & FIO_UTF16) != 0 && p_idx > ptr_idx {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]) << 8;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]);
                                    } else {
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]);
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 8;
                                    }
                                    if (0xd800..=0xdbff).contains(&u8c) {
                                        tail_opt = Some(p_idx);
                                    } else {
                                        p_idx += 2;
                                    }
                                }
                            } else {
                                p_idx = ptr_idx + (size_usize & !3);
                                if (size_usize & 3) != 0 {
                                    tail_opt = Some(p_idx);
                                }
                            }

                            if let Some(t_idx) = tail_opt {
                                let cr = (ptr_idx + size_usize) - t_idx;
                                conv_restlen = cr as i32;
                                conv_rest[..cr].copy_from_slice(&buffer_vec[t_idx..t_idx + cr]);
                                size -= conv_restlen as isize;
                            }

                            while p_idx > ptr_idx {
                                if (fio_flags & FIO_LATIN1) != 0 {
                                    p_idx -= 1;
                                    u8c = u32::from(buffer_vec[p_idx]);
                                } else if (fio_flags & (FIO_UCS2 | FIO_UTF16)) != 0 {
                                    if (fio_flags & FIO_ENDIAN_L) != 0 {
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]) << 8;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]);
                                    } else {
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]);
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 8;
                                    }
                                    if (fio_flags & FIO_UTF16) != 0
                                        && (0xdc00..=0xdfff).contains(&u8c)
                                    {
                                        if p_idx == ptr_idx {
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
                                                conv_error = readfile_linenr(
                                                    linecnt,
                                                    &buffer_vec[ptr_idx..p_idx],
                                                );
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
                                            p_idx -= 1;
                                            u16c = i32::from(buffer_vec[p_idx]) << 8;
                                            p_idx -= 1;
                                            u16c += i32::from(buffer_vec[p_idx]);
                                        } else {
                                            p_idx -= 1;
                                            u16c = i32::from(buffer_vec[p_idx]);
                                            p_idx -= 1;
                                            u16c += i32::from(buffer_vec[p_idx]) << 8;
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
                                                conv_error = readfile_linenr(
                                                    linecnt,
                                                    &buffer_vec[ptr_idx..p_idx],
                                                );
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
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]) << 24;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 16;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 8;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]);
                                    } else {
                                        p_idx -= 1;
                                        u8c = u32::from(buffer_vec[p_idx]);
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 8;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 16;
                                        p_idx -= 1;
                                        u8c += u32::from(buffer_vec[p_idx]) << 24;
                                    }
                                    if u8c > i32::MAX as u32 {
                                        u8c = 0xfffd;
                                    }
                                } else {
                                    p_idx -= 1;
                                    if buffer_vec[p_idx] < 0x80 {
                                        u8c = u32::from(buffer_vec[p_idx]);
                                    } else {
                                        let rel_idx = p_idx.saturating_sub(ptr_idx);
                                        let end_slice =
                                            (ptr_idx + rel_idx + 6).min(buffer_vec.len());
                                        let len = crate::mbyte::utf_head_off_slice(
                                            &buffer_vec[ptr_idx..end_slice],
                                            rel_idx,
                                        );
                                        p_idx -= len as usize;
                                        let char_end = (p_idx + 6).min(buffer_vec.len());
                                        u8c = utf_ptr2char(&buffer_vec[p_idx..char_end]) as u32;
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
                                                conv_error = readfile_linenr(
                                                    linecnt,
                                                    &buffer_vec[ptr_idx..p_idx],
                                                );
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
                                dest_idx -= utf_char2len(u8c as i32) as usize;
                                let dest_end = (dest_idx + 6).min(buffer_vec.len());
                                utf_char2bytes(u8c as i32, &mut buffer_vec[dest_idx..dest_end]);
                            }

                            let lr = linerest as usize;
                            line_start_idx = dest_idx - lr;
                            buffer_vec.copy_within(0..lr, line_start_idx);
                            size = ((ptr_idx + (real_size as usize)) - dest_idx) as isize;
                            ptr_idx = dest_idx;
                        } else if !curbuf_mut().b_p_bin.get() {
                            let mut incomplete_tail = false;

                            p_idx = ptr_idx;
                            loop {
                                {
                                    let ascii_end = ptr_idx + (size as usize);
                                    while (ascii_end as isize) - (p_idx as isize)
                                        >= size_of::<u64>() as isize
                                    {
                                        let word = u64::from_ne_bytes(
                                            buffer_vec[p_idx..p_idx + 8].try_into().unwrap(),
                                        );
                                        if (word & NONASCII_MASK) != 0 {
                                            break;
                                        }
                                        p_idx += size_of::<u64>();
                                    }
                                    while p_idx < ascii_end && buffer_vec[p_idx] < 0x80 {
                                        p_idx += 1;
                                    }
                                }

                                let todo = ((ptr_idx + (size as usize)) as isize - (p_idx as isize))
                                    as i32;
                                if todo <= 0 {
                                    break;
                                }
                                if buffer_vec[p_idx] >= 0x80 {
                                    let l = utf_ptr2len_len(
                                        &buffer_vec[p_idx..p_idx + (todo as usize)],
                                        todo,
                                    );
                                    if l > todo && !incomplete_tail {
                                        if p_idx > ptr_idx || filesize > 0 {
                                            incomplete_tail = true;
                                        }
                                        if p_idx > ptr_idx {
                                            conv_restlen = todo;
                                            let cr = conv_restlen as usize;
                                            conv_rest[..cr]
                                                .copy_from_slice(&buffer_vec[p_idx..p_idx + cr]);
                                            size -= conv_restlen as isize;
                                            break;
                                        }
                                    }
                                    if l == 1 || l > todo {
                                        if can_retry && !incomplete_tail {
                                            break;
                                        }

                                        let err_slice = &buffer_vec[ptr_idx..p_idx];
                                        if iconv_fd != -1 && conv_error == 0 {
                                            conv_error = readfile_linenr(linecnt, err_slice);
                                        }

                                        if conv_error == 0 && illegal_byte == 0 {
                                            illegal_byte = readfile_linenr(linecnt, err_slice);
                                        }

                                        if bad_char_behavior == BAD_DROP {
                                            let rem_len = (todo - 1) as usize;
                                            buffer_vec
                                                .copy_within(p_idx + 1..p_idx + 1 + rem_len, p_idx);
                                            size -= 1;
                                        } else {
                                            if bad_char_behavior != BAD_KEEP {
                                                buffer_vec[p_idx] = bad_char_behavior as u8;
                                            }
                                            p_idx += 1;
                                        }
                                    } else {
                                        p_idx += l as usize;
                                    }
                                }
                            }
                            if p_idx < ptr_idx + (size as usize) && !incomplete_tail {
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

                                p_idx = ptr_idx;
                                let end_idx = ptr_idx + (size as usize);
                                while p_idx < end_idx {
                                    if buffer_vec[p_idx] == b'\n' {
                                        if try_unix == 0
                                            || (try_dos != 0
                                                && p_idx > ptr_idx
                                                && buffer_vec[p_idx - 1] == b'\r')
                                        {
                                            fileformat = EOL_DOS;
                                        } else {
                                            fileformat = EOL_UNIX;
                                        }
                                        break;
                                    } else if buffer_vec[p_idx] == b'\r' && try_mac != 0 {
                                        try_mac += 1;
                                    }
                                    p_idx += 1;
                                }

                                if fileformat == EOL_UNIX && try_mac != 0 {
                                    try_mac = 1;
                                    try_unix = 1;
                                    let mut scan_p = p_idx as isize;
                                    while scan_p >= (ptr_idx as isize)
                                        && buffer_vec[scan_p as usize] != b'\r'
                                    {
                                        scan_p -= 1;
                                    }
                                    if scan_p >= (ptr_idx as isize) {
                                        p_idx = ptr_idx;
                                        while p_idx < end_idx {
                                            if buffer_vec[p_idx] == b'\n' {
                                                try_unix += 1;
                                            } else if buffer_vec[p_idx] == b'\r' {
                                                try_mac += 1;
                                            }
                                            p_idx += 1;
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
                        let mut cur_ptr = (ptr_idx as isize) - 1;
                        loop {
                            cur_ptr += 1;
                            ptr_idx = cur_ptr as usize;
                            size -= 1;
                            if size < 0 {
                                break;
                            }
                            let c = buffer_vec[ptr_idx];
                            if c != 0 && c != CAR && c != b'\n' {
                                continue;
                            }
                            if c == 0 {
                                buffer_vec[ptr_idx] = b'\n';
                            } else if c == b'\n' {
                                buffer_vec[ptr_idx] = CAR;
                            } else {
                                if skip_count == 0 {
                                    buffer_vec[ptr_idx] = 0;
                                    let len = ((ptr_idx - line_start_idx) + 1) as Colnr;
                                    if !ml_append(
                                        lnum,
                                        &buffer_vec[line_start_idx
                                            ..line_start_idx + (len as usize).saturating_sub(1)],
                                        len,
                                        newfile,
                                    ) {
                                        error = true;
                                        break;
                                    }
                                    if read_undo_file && len > 0 {
                                        sha_ctx.update(
                                            &buffer_vec
                                                [line_start_idx..line_start_idx + (len as usize)],
                                        );
                                    }
                                    lnum += 1;
                                    read_count -= 1;
                                    if read_count == 0 {
                                        error = true;
                                        line_start_idx = ptr_idx;
                                        break;
                                    }
                                } else {
                                    skip_count -= 1;
                                }
                                line_start_idx = ptr_idx + 1;
                            }
                        }
                    } else {
                        let end_idx = ptr_idx + (size as usize);

                        while ptr_idx < end_idx {
                            let rem = end_idx - ptr_idx;
                            let slice = &buffer_vec[ptr_idx..ptr_idx + rem];
                            let Some(nl_pos) = slice.iter().position(|&b| b == b'\n') else {
                                for b in &mut buffer_vec[ptr_idx..ptr_idx + rem] {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                                ptr_idx = end_idx;
                                break;
                            };

                            let nl_idx = ptr_idx + nl_pos;
                            if nl_pos > 0 {
                                for b in &mut buffer_vec[ptr_idx..nl_idx] {
                                    if *b == 0 {
                                        *b = b'\n';
                                    }
                                }
                            }

                            ptr_idx = nl_idx;
                            if skip_count == 0 {
                                buffer_vec[ptr_idx] = 0;
                                let mut len = ((ptr_idx - line_start_idx) + 1) as Colnr;
                                if fileformat == EOL_DOS {
                                    if ptr_idx > line_start_idx && buffer_vec[ptr_idx - 1] == CAR {
                                        buffer_vec[ptr_idx - 1] = 0;
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
                                if !ml_append(
                                    lnum,
                                    &buffer_vec[line_start_idx
                                        ..line_start_idx + (len as usize).saturating_sub(1)],
                                    len,
                                    newfile,
                                ) {
                                    error = true;
                                    break;
                                }
                                if read_undo_file && len > 0 {
                                    sha_ctx.update(
                                        &buffer_vec
                                            [line_start_idx..line_start_idx + (len as usize)],
                                    );
                                }
                                lnum += 1;
                                read_count -= 1;
                                if read_count == 0 {
                                    error = true;
                                    line_start_idx = ptr_idx;
                                    break;
                                }
                            } else {
                                skip_count -= 1;
                            }
                            line_start_idx = ptr_idx + 1;
                            ptr_idx += 1;
                        }
                        size = -1;
                    }
                    linerest = (ptr_idx as isize) - (line_start_idx as isize);

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
                                &mut *g_st().iobuff_bytes_mut(),
                                g_st().curbuf.get().opt(),
                                sfname,
                            );
                            let mut iob = *g_st().iobuff.borrow();
                            let _ = readfile_progress(
                                Some(&mut iob),
                                crate::memory::bytes_as_cstr(&msg_id),
                                c"running",
                                percent,
                                false,
                            );
                            *g_st().iobuff.borrow_mut() = iob;
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
                && !curbuf_mut().b_p_bin.get()
                && fileformat == EOL_DOS
                && buffer_vec[ptr_idx - 1] == CTRL_Z
            {
                ptr_idx -= 1;
                linerest -= 1;
                if set_options {
                    curbuf_mut().b_p_eof.set(true);
                }
            }

            if !error && !g_st().got_int.get() && linerest != 0 {
                if set_options {
                    curbuf_mut().b_p_eol.set(false);
                }
                buffer_vec[ptr_idx] = 0;
                let len = ((ptr_idx - line_start_idx) + 1) as Colnr;
                if !ml_append(
                    lnum,
                    &buffer_vec[line_start_idx..line_start_idx + (len as usize).saturating_sub(1)],
                    len,
                    newfile,
                ) {
                    error = true;
                } else {
                    if read_undo_file && len > 0 {
                        sha_ctx
                            .update(&buffer_vec[line_start_idx..line_start_idx + (len as usize)]);
                    }
                    lnum += 1;
                    read_no_eol_lnum = lnum;
                }
            }

            if set_options {
                save_file_ff(&curbuf_mut());
                set_option_direct(
                    K_OPT_FILEENCODING,
                    cstr_as_obj(fenc_owned.as_deref()),
                    OPT_LOCAL,
                    0,
                );
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
                if g_st().stdin_fd.get() < 0 {
                    #[cfg(not(windows))]
                    {
                        g_st().vim_ignored.set(dup(2));
                    }
                    #[cfg(windows)]
                    {
                        g_st().vim_ignored.set(os_open_conin_fd());
                    }
                }
            }

            if let Some(ref tn) = tmpname {
                os_remove(tn);
            }
            g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));

            if !g_st().recoverymode.get() {
                if newfile && wasempty && (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) == 0 {
                    ml_delete(curbuf_mut().b_ml.ml_line_count.get());
                    linecnt -= 1;
                }
                curbuf_mut().deleted_bytes.set(0);
                curbuf_mut().deleted_bytes2.set(0);
                curbuf_mut().deleted_codepoints.set(0);
                curbuf_mut().deleted_codeunits.set(0);
                linecnt = curbuf_mut().b_ml.ml_line_count.get() - linecnt;
                if filesize == 0 {
                    linecnt = 0;
                }
                if newfile || read_buffer {
                    redraw_curbuf_later(UPD_NOT_VALID);
                    diff_invalidate(&curbuf_mut());
                    fold_update_all(&curwin_mut());
                } else if linecnt != 0 {
                    appended_lines_mark(from, linecnt);
                }

                if g_st().got_int.get() {
                    if progress {
                        let _ = readfile_progress(
                            None,
                            crate::memory::bytes_as_cstr(&msg_id),
                            c"cancel",
                            -1,
                            false,
                        );
                    }
                    if (flags & READ_DUMMY) == 0 {
                        filemess(g_st().curbuf.get().opt(), sfname, tr(E_INTERR), None);
                        if newfile {
                            curbuf_mut().b_p_ro.set(true);
                        }
                    }
                    g_st().msg_scroll.set(msg_save);
                    check_marks_read();
                    retval = Some(true);
                    break 'theend;
                }

                if !filtering && (flags & READ_DUMMY) == 0 && !silent {
                    add_quoted_fname(
                        &mut *g_st().iobuff_bytes_mut(),
                        g_st().curbuf.get().opt(),
                        sfname,
                    );
                    let mut c = false;

                    let mut buflen = g_st().iobuff_cstr().to_bytes().len() as i32;
                    #[cfg(unix)]
                    {
                        if s_isfifo(perm as u64) {
                            buflen += format_to_buf!(
                                &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                                c"%s",
                                tr(c"[fifo]"),
                            );
                            c = true;
                        }
                        if s_issock(perm as u64) {
                            buflen += format_to_buf!(
                                &mut g_st().iobuff.borrow_mut()[buflen as usize..],
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
                            buflen += format_to_buf!(
                                &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                                c"%s",
                                tr(c"[character special]"),
                            );
                            c = true;
                        }
                    }
                    if curbuf_mut().b_p_ro.get() {
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
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
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[noeol]"),
                        );
                        c = true;
                    }
                    if ff_error == EOL_DOS {
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[CR missing]"),
                        );
                        c = true;
                    }
                    if split != 0 {
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[long lines split]"),
                        );
                        c = true;
                    }
                    if notconverted {
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[NOT converted]"),
                        );
                        c = true;
                    } else if converted {
                        buflen += format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[converted]"),
                        );
                        c = true;
                    }
                    if conv_error != 0 {
                        format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            tr(c"[CONVERSION ERROR in line %lld]"),
                            i64::from(conv_error),
                        );
                        c = true;
                    } else if illegal_byte > 0 {
                        format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            tr(c"[ILLEGAL BYTE in line %lld]"),
                            i64::from(illegal_byte),
                        );
                        c = true;
                    } else if error {
                        format_to_buf!(
                            &mut g_st().iobuff.borrow_mut()[buflen as usize..],
                            c"%s",
                            tr(c"[READ ERRORS]"),
                        );
                        c = true;
                    }
                    if msg_add_fileformat(fileformat) {
                        c = true;
                    }

                    msg_add_lines(i32::from(c), linecnt, filesize);

                    g_st().keep_msg.set(None);
                    let mut keep_cstr: Option<std::ffi::CString> = None;
                    g_st().msg_scrolled_ign.set(true);

                    if !read_stdin && !read_buffer {
                        if g_st().msg_col.get() > 0 {
                            msg_putchar(b'\r' as i32);
                        }
                        let mut iob = *g_st().iobuff.borrow();
                        let keep_owned = if progress {
                            readfile_progress(
                                Some(&mut iob),
                                crate::memory::bytes_as_cstr(&msg_id),
                                if error { c"failed" } else { c"success" },
                                -1,
                                true,
                            )
                        } else {
                            msg_trunc(&mut iob, false, 0)
                        }
                        .map(CStr::to_owned);
                        *g_st().iobuff.borrow_mut() = iob;
                        keep_cstr = keep_owned;
                    }

                    if read_stdin
                        || read_buffer
                        || g_st().restart_edit.get() != 0
                        || (g_st().msg_scrolled.get() != 0 && !g_st().need_wait_return.get())
                    {
                        set_keep_msg(keep_cstr.as_deref(), 0);
                    }
                    g_st().msg_scrolled_ign.set(false);
                } else if progress {
                    let _ = readfile_progress(
                        None,
                        crate::memory::bytes_as_cstr(&msg_id),
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
                    curbuf_mut().b_p_ro.set(true);
                }

                u_clearline(&curbuf_mut());

                curwin_mut().w_cursor.set_lnum(from + 1);
                check_cursor_lnum(&curwin_mut());
                beginline(BL_WHITE | BL_FIX);

                if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
                    curbuf_mut().b_op_start.set_lnum(from + 1);
                    curbuf_mut().b_op_start.set_col(0);
                    curbuf_mut().b_op_end.set_lnum(from + linecnt);
                    curbuf_mut().b_op_end.set_col(0);
                }
            }
            g_st().msg_scroll.set(msg_save);

            check_marks_read();

            curbuf_mut().b_no_eol_lnum.set(read_no_eol_lnum);

            if (flags & READ_KEEP_UNDO) != 0 {
                u_find_first_changed();
            }

            if read_undo_file {
                let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
                sha_ctx.finish_into(&mut hash);
                u_read_undo(None, &hash, fname);
            }

            if !read_stdin && !read_fifo && (!read_buffer || sfname.is_some()) {
                let m = g_st().msg_scroll.get();
                let n = g_st().msg_scrolled.get();

                if set_options {
                    save_file_ff(&curbuf_mut());
                }

                g_st().msg_scroll.set(1);
                let sfname_opt = sfname;
                if filtering {
                    apply_autocmds_exarg(
                        EVENT_FILTERREADPOST,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    );
                } else if newfile || (read_buffer && sfname.is_some()) {
                    let did_bufrp = apply_autocmds_exarg(
                        EVENT_BUFREADPOST,
                        None,
                        sfname_opt,
                        false,
                        curbuf_opt_mut().as_deref(),
                        eap,
                    );

                    if !curbuf_mut().b_au_did_filetype.get() && !curbuf_mut().b_p_ft.is_empty() {
                        apply_autocmds(
                            EVENT_FILETYPE,
                            Some(&curbuf_mut().b_p_ft.as_cstr()),
                            curbuf_mut().fname_opt().as_deref(),
                            true,
                            curbuf_opt_mut().as_deref(),
                        );
                    }

                    if !did_bufrp
                        && curbuf_mut().b_p_ft.is_empty()
                        && augroup_exists(Some(c"filetypedetect"))
                    {
                        apply_autocmds_group(
                            EVENT_BUFREADPOST,
                            sfname_opt,
                            None,
                            true,
                            augroup_find(c"filetypedetect"),
                            curbuf_opt_mut().as_deref(),
                            Some(&crate::globals::curwin_mut()),
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
                if g_st().msg_scrolled.get() == n {
                    g_st().msg_scroll.set(m);
                }
                if aborting() {
                    return Some(false);
                }
            }

            if !(g_st().recoverymode.get() && error) {
                retval = Some(true);
            }
        }

        if let Some(mut mfp) = curbuf_mut().b_ml.mfp_opt()
            && mfp.mf_dirty == MF_DIRTY_YES_NOSYNC
        {
            mfp.mf_dirty = MF_DIRTY_YES;
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
    curbuf_mut().b_ml.ml_line_count.get() - linecnt + 1 + newlines
}

/// Fill "*eap" to force the 'fileencoding', 'fileformat' and 'binary' to be
/// equal to the buffer "buf".
pub fn prep_exarg(eap: &mut Exarg, buf: &FileBuf) -> std::ffi::CString {
    let fenc_bytes = buf.b_p_fenc.as_bytes();
    let mut cmd_vec = Vec::with_capacity(8 + fenc_bytes.len());
    cmd_vec.extend_from_slice(b"e ++enc=");
    cmd_vec.extend_from_slice(&fenc_bytes);
    let cmd_cstr = std::ffi::CString::new(cmd_vec).unwrap_or_default();
    eap.cmd = crate::strings::OwnedSlice::from_cstr(&cmd_cstr);
    eap.force_enc = 8;
    eap.bad_char = buf.b_bad_char.get();
    eap.force_ff = (buf.b_p_ff.first_byte()) as i32;
    eap.force_bin = Some(buf.b_p_bin.get());
    eap.read_edit = false;
    eap.forceit = false;
    cmd_cstr
}

/// Set default or forced 'fileformat' and 'binary'.
pub fn set_file_options(set_options: bool, eap: Option<&Exarg>) {
    if set_options {
        if eap.is_some() && eap.unwrap().force_ff != 0 {
            set_fileformat(
                get_fileformat_force(&curbuf_mut(), (eap.is_some()).then(|| eap.unwrap())),
                OPT_LOCAL,
            );
        } else if !opt_vars().p_ffs.is_empty() {
            set_fileformat(default_fileformat(), OPT_LOCAL);
        }
    }

    if let Some(eap) = eap
        && eap.force_bin.is_some()
    {
        let oldval = curbuf_mut().b_p_bin.get();
        curbuf_mut().b_p_bin.set(eap.force_bin == Some(true));
        set_options_bin(
            i32::from(oldval),
            i32::from(curbuf_mut().b_p_bin.get()),
            OPT_LOCAL,
        );
    }
}

/// Set forced 'fileencoding'.
pub fn set_forced_fenc(eap: &Exarg) {
    if eap.force_enc == 0 {
        return;
    }

    let fenc = enc_canonize(eap.cmd.wrapping_add(eap.force_enc as usize).as_cstr());
    set_option_direct(K_OPT_FILEENCODING, cstr_as_obj(Some(&*fenc)), OPT_LOCAL, 0);
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
        enc_canonize(&tmp)
    } else {
        let tmp = std::ffi::CString::new(crate::memory::bytes_until_nul(pp)).unwrap_or_default();
        *pp = b"";
        enc_canonize(&tmp)
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
                *fdp = os_open(tn, O_RDONLY, 0);
                if *fdp < 0 {
                    errmsg = Some(tr(c"can't read output of 'charconvert'"));
                }
            }
        }
    }

    if let Some(errmsg) = errmsg {
        msg(errmsg, 0);
        if let Some(ref tn) = tmpname {
            os_remove(tn);
            tmpname = None;
        }
    }

    if *fdp < 0 {
        *fdp = os_open(fname, O_RDONLY, 0);
    }

    tmpname
}

/// Set the name of the current buffer.
pub fn set_rw_fname(fname: Option<&CStr>, sfname: Option<&CStr>) -> bool {
    let fname_opt = fname;
    let sfname_opt = sfname;
    let buf = g_st().curbuf.get();

    if curbuf_mut().b_p_bl.get() {
        apply_autocmds(
            EVENT_BUFDELETE,
            None,
            None,
            false,
            curbuf_opt_mut().as_deref(),
        );
    }
    apply_autocmds(
        EVENT_BUFWIPEOUT,
        None,
        None,
        false,
        curbuf_opt_mut().as_deref(),
    );
    if aborting() {
        return false;
    }
    if g_st().curbuf.get() != buf {
        emsg(tr(E_AUCHANGEDBUF));
        return false;
    }

    if setfname(&curbuf_mut(), fname_opt, sfname_opt, false) {
        curbuf_mut()
            .b_flags
            .set(curbuf_mut().b_flags.get() | (BF_NOTEDITED));
    }

    apply_autocmds(EVENT_BUFNEW, None, None, false, curbuf_opt_mut().as_deref());
    if curbuf_mut().b_p_bl.get() {
        apply_autocmds(EVENT_BUFADD, None, None, false, curbuf_opt_mut().as_deref());
    }
    if aborting() {
        return false;
    }

    if curbuf_mut().b_p_ft.is_empty() {
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
    let max_len = bufsize.saturating_sub(4);
    len += home_replace(
        buf,
        Some(fname_cstr),
        &mut ret_buf[len..max_len.max(len)],
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
        crate::memory::append_bytes_to_c_buf(
            &mut *g_st().iobuff_bytes_mut(),
            tr(c"[dos]").to_bytes(),
        );
        return true;
    }
    if eol_type == EOL_MAC {
        crate::memory::append_bytes_to_c_buf(
            &mut *g_st().iobuff_bytes_mut(),
            tr(c"[mac]").to_bytes(),
        );
        return true;
    }
    #[cfg(windows)]
    if eol_type == EOL_UNIX {
        crate::memory::append_bytes_to_c_buf(
            &mut *g_st().iobuff_bytes_mut(),
            tr(c"[unix]").to_bytes(),
        );
        return true;
    }
    false
}

/// Append line and character count to IObuff.
pub fn msg_add_lines(insert_space: i32, lnum: Linenr, nchars: Off) {
    let mut len = g_st().iobuff_cstr().to_bytes().len();
    let space = if insert_space != 0 { c" " } else { c"" };

    if shortmess(K_SHM_LINES) {
        format_to_buf!(
            &mut g_st().iobuff.borrow_mut()[len..],
            tr(c"%s%lldL, %lldB"),
            space,
            i64::from(lnum),
            nchars,
        );
    } else {
        len += format_to_buf!(
            &mut g_st().iobuff.borrow_mut()[len..],
            ngettext(c"%s%lld line, ", c"%s%lld lines, ", lnum as usize),
            space,
            i64::from(lnum),
        ) as usize;
        format_to_buf!(
            &mut g_st().iobuff.borrow_mut()[len..],
            ngettext(c"%lld byte", c"%lld bytes", nchars as usize),
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

    let p_enc = opt_vars().p_enc.to_owned();
    let p_enc_opt = Some(p_enc.as_c_str());
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
    let p_enc = opt_vars().p_enc.to_owned();
    let name_cstr = match name.filter(|s| !s.to_bytes().is_empty()) {
        Some(s) => s,
        None => {
            if opt_vars().p_enc.is_null() {
                c"utf-8"
            } else {
                p_enc.as_c_str()
            }
        }
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
pub fn shorten_buf_fname(buf: &FileBuf, dirname: &CStr, force: i32) -> bool {
    if buf.fname_opt().is_none()
        || bt_nofilename(Some(buf))
        || path_with_url(&buf.fname_bytes())
        || !(force != 0 || buf.b_sfname.get().is_none() || path_is_absolute(&buf.sfname_bytes()))
    {
        return false;
    }

    let ffname_opt = buf.ffname_opt();
    let p = path_shorten_fname(ffname_opt.as_deref(), dirname);
    let shorten = p.is_some_and(|s| !s.to_bytes().is_empty());
    if shorten && buf.b_sfname.get().is_some() && p == Some(&buf.sfname_cstr()) {
        return false;
    }

    let new_fname = if shorten { p } else { ffname_opt.as_deref() };
    let changed = new_fname.is_none() || new_fname != buf.fname_opt().as_deref();

    buf.set_sfname(shorten.then(|| p.unwrap().to_owned()));
    if shorten {
        buf.b_fname.set(crate::buffer::BufFname::Short);
    } else {
        buf.b_fname.set(crate::buffer::BufFname::Full);
    }
    changed
}

/// Shortens filenames for all buffers.
pub fn shorten_fnames(force: bool) {
    let mut dirname: [u8; MAXPATHL] = [0; MAXPATHL];
    let mut changed = false;

    os_dirname(&mut dirname);
    let dirname_cstr = crate::memory::bytes_as_cstr(&dirname);
    for buf in for_all_buffers_mut() {
        changed |= shorten_buf_fname(&buf, dirname_cstr, i32::from(force));
        mf_fullname(buf.b_ml.mfp_opt().as_deref_mut());
    }
    if changed {
        status_redraw_all();
        g_st().redraw_tabline.set(true);
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
        if !os_dirname(&mut retval_vec) || retval_vec[0] == 0 {
            return None;
        }
        add_pathsep(&mut retval_vec);
        fnamelen = crate::memory::bytes_until_nul(&retval_vec).len();
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
        let head_off = crate::mbyte::utf_head_off_slice(&retval_vec, ptr_idx - 1) as usize;
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

    let e_ptr = path_tail(crate::memory::bytes_as_cstr(&retval_vec)).as_ptr();
    let e_idx = (e_ptr as usize) - (retval_vec.as_ptr() as usize);
    if prepend_dot && retval_vec[e_idx] != b'.' {
        let end_idx = fnamelen + extlen;
        retval_vec.copy_within(e_idx..=end_idx, e_idx + 1);
        retval_vec[e_idx] = b'.';
    }

    if fname.is_some() && fname_bytes == crate::memory::bytes_until_nul(&retval_vec) {
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
    if from_bytes.len() >= MAXPATHL - 5 {
        return -1;
    }

    let mut tempname: [u8; MAXPATHL + 1] = [0; MAXPATHL + 1];
    crate::memory::copy_bytes_to_c_buf(&mut tempname, from_bytes);
    for n in 123..99999 {
        let tail = path_tail(crate::memory::bytes_as_cstr(&tempname)).as_ptr();
        let tail_off = (tail as usize) - (tempname.as_ptr() as usize);
        format_to_buf!(&mut tempname[tail_off..], c"%d", n);

        let temp_cstr = crate::memory::bytes_as_cstr(&tempname);
        if !os_path_exists(temp_cstr) {
            if os_rename(from, temp_cstr) {
                if os_rename(temp_cstr, to) {
                    return 0;
                }
                if !os_rename(temp_cstr, from) {
                    logmsg!(
                        LOG_ERR,
                        None,
                        Some(c"rename_with_tmp"),
                        0,
                        true,
                        c"failed to restore %s, file is at: %s",
                        from,
                        &tempname,
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
    let mut use_tmp_file = false;

    if path_equal(from, to, K_PATH_CMP_LITERAL) {
        if opt_vars().p_fic.get() && path_tail(from) != path_tail(to) {
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
    if os_fileinfo(to, &mut to_info) && os_fileinfo_id_equal(&from_info, &to_info) {
        use_tmp_file = true;
    }

    if use_tmp_file {
        return rename_with_tmp(from, to);
    }

    if os_rename(from, to) {
        return 0;
    }

    if os_remove(to) == 0 && os_rename(from, to) {
        return 0;
    }

    if !vim_copyfile(from, to) {
        return -1;
    }

    if os_fileinfo(from, &mut from_info) {
        os_remove(from);
    }

    0
}

/// Create the new file with same permissions as the original.
pub fn vim_copyfile(from: &CStr, to: &CStr) -> bool {
    #[cfg(not(windows))]
    {
        let mut from_info: FileInfo = FileInfo::new();
        if os_fileinfo_link(from, &mut from_info) && s_islnk(from_info.stat.st_mode) {
            let mut ret = -1;
            let mut linkbuf: [u8; MAXPATHL + 1] = [0; MAXPATHL + 1];
            let len = os_readlink(from, &mut linkbuf[..MAXPATHL]);
            if len > 0 {
                linkbuf[len as usize] = 0;
                ret = os_symlink(crate::memory::bytes_as_cstr(&linkbuf), to);
            }
            return ret == 0;
        }
    }

    let acl: VimAcl = os_get_acl(from);

    if os_copy(from, to, UV_FS_COPYFILE_EXCL) != 0 {
        os_free_acl(acl);
        return false;
    }

    os_set_acl(to, acl);
    os_free_acl(acl);

    true
}

/// Check if any not hidden buffer has been changed.
pub fn check_timestamps(focus: i32) -> i32 {
    if g_st().no_check_timestamps.get() > 0 {
        return 0;
    }

    if focus != 0 && g_st().did_check_timestamps.get() {
        g_st().need_check_timestamps.set(true);
        return 0;
    }

    let mut didit: i32 = 0;

    if !stuff_empty()
        || g_st().global_busy.get() != 0
        || typebuf_typed() == 0
        || g_st().autocmd_busy.get()
        || curbuf_mut().b_ro_locked.get() > 0
        || g_st().allbuf_lock.get() > 0
    {
        g_st().need_check_timestamps.set(true);
    } else {
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        g_st().did_check_timestamps.set(true);
        with_fio(|st| st.already_warned.set(false));
        let mut buf_opt = g_st().firstbuf.get();
        while !buf_opt.is_null() {
            let next_ptr = buf_opt.b_next.get();
            if buf_opt.b_nwindows.get() > 0 {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, Some(&buf_opt));
                let n = buf_check_timestamp(&buf_opt);
                didit = didit.max(n);
                if n > 0 && !bufref_valid(Some(&bufref)) {
                    buf_opt = g_st().firstbuf.get();
                    continue;
                }
            }
            buf_opt = next_ptr;
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
        g_st().need_check_timestamps.set(false);
        if g_st().need_wait_return.get() && didit == 2 {
            msg_puts(c"\n");
            ui_flush();
        }
    }
    didit
}

/// Move all the lines from buffer "frombuf" to buffer "tobuf".
fn move_lines(frombuf: &FileBuf, tobuf: &FileBuf) -> bool {
    let tbuf = g_st().curbuf.get();
    let mut retval = true;

    g_st().curbuf.set(crate::globals::BufPtr::from(tobuf));
    for lnum in 1..=frombuf.b_ml.ml_line_count.get() {
        let p = ml_get_buf(frombuf, lnum).to_owned();
        if !ml_append(lnum - 1, p.to_bytes(), 0, false) {
            retval = false;
            break;
        }
    }

    if retval {
        g_st().curbuf.set(crate::globals::BufPtr::from(frombuf));
        let mut lnum = curbuf_mut().b_ml.ml_line_count.get();
        while lnum > 0 {
            if !ml_delete(lnum) {
                retval = false;
                break;
            }
            lnum -= 1;
        }
    }

    g_st().curbuf.set(tbuf);
    retval
}

/// Check if buffer "buf" has been changed.
pub fn buf_check_timestamp(buf: &FileBuf) -> i32 {
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
    let orig_size = buf.b_orig_size.get();
    let orig_mode = buf.b_orig_mode.get();

    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(buf));

    if !buf.terminal.get().is_null()
        || buf.b_ffname.get().is_none()
        || buf.b_ml.ml_mfp.borrow().is_none()
        || !bt_normal(Some(buf))
        || buf.b_saving.get()
        || BUSY.load(std::sync::atomic::Ordering::Relaxed)
    {
        return 0;
    }

    let mut file_info: FileInfo = FileInfo::new();
    let file_info_ok: bool;
    if (buf.b_flags.get() & BF_NOTEDITED) == 0
        && buf.b_mtime.get() != 0
        && ({
            file_info_ok = os_fileinfo(&buf.ffname_cstr(), &mut file_info);
            !file_info_ok
                || time_differs(&file_info, buf.b_mtime.get(), buf.b_mtime_ns.get())
                || (file_info.stat.st_mode as i32) != buf.b_orig_mode.get()
        })
    {
        let prev_b_mtime = buf.b_mtime.get();

        retval = 1;

        if !file_info_ok {
            buf.b_mtime.set(-1);
            buf.b_orig_size.set(0);
            buf.b_orig_mode.set(0);
        } else {
            buf_store_file_info(buf, &mut file_info);
        }

        if os_isdir(&buf.fname_cstr()) {
            // Don't do anything for a directory.
        } else if buf.b_p_ar.get().unwrap_or(opt_vars().p_ar.get())
            && !buf_is_changed_fn(buf)
            && file_info_ok
        {
            reload = RELOAD_NORMAL;
        } else {
            let (reason, reasonlen) = if !file_info_ok {
                (c"deleted", 7isize)
            } else if buf_is_changed_fn(buf) {
                (c"conflict", 8isize)
            } else if orig_size != buf.b_orig_size.get() || buf_contents_changed(buf) {
                (c"changed", 7isize)
            } else if orig_mode != buf.b_orig_mode.get() {
                (c"mode", 4isize)
            } else {
                (c"time", 4isize)
            };

            BUSY.store(true, std::sync::atomic::Ordering::Relaxed);
            set_vim_var_string(VV_FCS_REASON, Some(reason), reasonlen);
            set_vim_var_string(VV_FCS_CHOICE, Some(c""), 0);
            g_st().allbuf_lock.set(g_st().allbuf_lock.get() + (1));
            let b_fname = buf.fname_opt().as_deref().map(CStr::to_owned);
            let mut n = apply_autocmds(
                EVENT_FILECHANGEDSHELL,
                b_fname.as_deref(),
                b_fname.as_deref(),
                false,
                Some(buf),
            );
            g_st().allbuf_lock.set(g_st().allbuf_lock.get() - (1));
            BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
            if n {
                if !bufref_valid(Some(&bufref)) {
                    emsg(tr(c"E246: FileChangedShell autocommand deleted buffer"));
                }
                let s = get_vim_var_str(VV_FCS_CHOICE);
                if s == c"reload" && reason.to_bytes().first().copied().unwrap_or(0) != b'd' {
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
                if reason.to_bytes().first().copied().unwrap_or(0) == b'd' {
                    if prev_b_mtime != -1 {
                        mesg = Some(tr(c"E211: File \"%s\" no longer available"));
                    }
                } else {
                    helpmesg = true;
                    can_reload = true;

                    let reason_bytes = reason.to_bytes();
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
                    } else if reason.to_bytes().first().copied().unwrap_or(0) == b'm' {
                        mesg = Some(tr(
                            c"W16: Warning: Mode of file \"%s\" has changed since editing started",
                        ));
                        mesg2 = tr(c"See \":help W16\" for more info.");
                    } else {
                        buf.b_mtime_read.set(buf.b_mtime.get());
                        buf.b_mtime_read_ns.set(buf.b_mtime_ns.get());
                    }
                }
            }
        }
    } else if (buf.b_flags.get() & BF_NEW) != 0
        && (buf.b_flags.get() & BF_NEW_W) == 0
        && os_path_exists(&buf.ffname_cstr())
    {
        retval = 1;
        mesg = Some(tr(
            c"W13: Warning: File \"%s\" has been created after editing started",
        ));
        buf.b_flags.set(buf.b_flags.get() | (BF_NEW_W));
        can_reload = true;
    }

    if let Some(mesg) = mesg {
        let path = home_replace_save(Some(buf), buf.fname_opt().as_deref());
        if !helpmesg {
            mesg2 = c"";
        }
        let tbufsize = path.as_bytes().len() + mesg.to_bytes().len() + mesg2.to_bytes().len() + 3;
        let mut tbuf_vec = vec![0u8; tbufsize];
        let tbuflen = format_to_buf!(&mut tbuf_vec, mesg, &path);
        set_vim_var_string(
            VV_WARNINGMSG,
            Some(crate::memory::bytes_as_cstr(&tbuf_vec)),
            tbuflen as isize,
        );
        if can_reload {
            if !mesg2.to_bytes().is_empty() {
                format_to_buf!(&mut tbuf_vec[tbuflen as usize..], c"\n%s", mesg2);
            }
            match do_dialog(
                VIM_WARNING,
                Some(tr(c"Warning")),
                crate::memory::bytes_as_cstr(&tbuf_vec),
                tr(c"&OK\n&Load File\nLoad File &and Options"),
                1,
                None,
                1,
            ) {
                2 => reload = RELOAD_NORMAL,
                3 => reload = RELOAD_DETECT,
                _ => {}
            }
        } else if g_st().state_global.get() > MODE_NORMAL_BUSY
            || (g_st().state_global.get() & MODE_CMDLINE) != 0
            || with_fio(|st| st.already_warned.get())
        {
            if !mesg2.to_bytes().is_empty() {
                format_to_buf!(&mut tbuf_vec[tbuflen as usize..], c"; %s", mesg2);
            }
            emsg(crate::memory::bytes_as_cstr(&tbuf_vec));
            retval = 2;
        } else {
            if !g_st().autocmd_busy.get() {
                msg_start();
                msg_puts_hl(crate::memory::bytes_as_cstr(&tbuf_vec), HLF_E, true);
                if !mesg2.to_bytes().is_empty() {
                    msg_puts_hl(mesg2, HLF_W, true);
                }
                msg_clr_eos();
                msg_end();
                if g_st().emsg_silent.get() == 0
                    && !g_st().in_assert_fails.get()
                    && !ui_has(K_UIMESSAGES)
                {
                    msg_delay(1004, true);
                    g_st().redraw_cmdline.set(false);
                }
            }
            with_fio(|st| st.already_warned.set(true));
        }

        drop(tbuf_vec);
        drop(path);
    }

    if reload != RELOAD_NONE {
        buf_reload(buf, orig_mode, reload == RELOAD_DETECT);
        if bufref_valid(Some(&bufref)) && buf.b_p_udf.get() && buf.b_ffname.get().is_some() {
            let mut hash: [u8; UNDO_HASH_SIZE] = [0; UNDO_HASH_SIZE];
            u_compute_hash(buf, &mut hash);
            u_write_undo(None, false, buf, &hash);
        }
    }

    if bufref_valid(Some(&bufref)) && retval != 0 {
        let b_fname = buf.fname_opt().as_deref().map(CStr::to_owned);
        apply_autocmds(
            EVENT_FILECHANGEDSHELLPOST,
            b_fname.as_deref(),
            b_fname.as_deref(),
            false,
            Some(buf),
        );
    }
    retval
}

/// Reload a buffer that is already loaded.
pub fn buf_reload(buf: &FileBuf, orig_mode: i32, reload_options: bool) {
    let mut ea: Exarg = Exarg::new();
    let old_ro = buf.b_p_ro.get();
    let savebuf: crate::globals::BufPtr;
    let mut bufref: Bufref = Bufref::new();
    let mut saved = true;
    let mut aco: CtxSwitch = CtxSwitch::new();
    let mut flags = READ_NEW;

    ctx_switch(&mut aco, None, None, Some(buf), 0);

    mc_buf_clear(buf);

    let _ea_cmd = if !reload_options {
        Some(prep_exarg(&mut ea, buf))
    } else {
        None
    };

    let old_cursor = curwin_mut().w_cursor.get();
    let old_topline = curwin_mut().w_topline.get();

    if opt_vars().p_ur.get() < 0
        || i64::from(curbuf_mut().b_ml.ml_line_count.get()) <= opt_vars().p_ur.get()
    {
        u_sync(false);
        saved = u_savecommon(
            &curbuf_mut(),
            0,
            curbuf_mut().b_ml.ml_line_count.get() + 1,
            0,
            true,
        );
        flags |= READ_KEEP_UNDO;
    }

    if buf_is_empty(&curbuf_mut()) || !saved {
        savebuf = crate::globals::BufPtr::NULL;
    } else {
        savebuf = crate::globals::BufPtr::from(buflist_new(None, None, 1, BLN_DUMMY));
        set_bufref(&mut bufref, savebuf.opt_mut());
        if !savebuf.is_null() && g_st().curbuf.get() == buf {
            g_st().curbuf.set(savebuf.clone());
            curwin_mut().w_buffer.set(savebuf.clone());
            saved = ml_open(&curbuf_mut());
            g_st().curbuf.set((buf).into());
            curwin_mut().w_buffer.set((buf).into());
        }
        if savebuf.is_null()
            || !saved
            || g_st().curbuf.get() != buf
            || !move_lines(buf, savebuf.borrow_mut())
        {
            semsg!(
                tr(c"E462: Could not prepare for reloading \"%s\""),
                buf.fname_cstr(),
            );
            saved = false;
        }
    }

    let b_fname = buf.fname_opt();
    let b_ffname = buf.ffname_opt();
    if saved {
        curbuf_mut()
            .b_flags
            .set(curbuf_mut().b_flags.get() | (BF_CHECK_RO));
        curbuf_mut().b_keep_filetype.set(true);
        if readfile(
            b_ffname.as_deref(),
            b_fname.as_deref(),
            0,
            0,
            MAXLNUM,
            Some(&ea),
            flags,
            shortmess(K_SHM_FILEINFO),
        ) != Some(true)
        {
            if !aborting() {
                semsg!(tr(c"E321: Could not reload \"%s\""), buf.fname_cstr(),);
            }
            if bufref_valid(Some(&bufref))
                && let Some(sb) = bufref.buf_opt()
                && g_st().curbuf.get() == buf
            {
                while !buf_is_empty(&curbuf_mut()) {
                    if !ml_delete(buf.b_ml.ml_line_count.get()) {
                        break;
                    }
                }
                move_lines(sb, buf);
            }
        } else if g_st().curbuf.get() == buf {
            unchanged(buf, true, true);
            if (flags & READ_KEEP_UNDO) == 0 {
                u_clearallandblockfree(buf);
            } else {
                u_unchanged(&curbuf_mut());
            }
            buf_updates_unload(&curbuf_mut(), true);
            curbuf_mut().b_mod_set.set(true);
        }
    }
    drop(_ea_cmd);

    if !savebuf.is_null() && bufref_valid(Some(&bufref)) {
        wipe_buffer(savebuf.borrow_mut(), false);
    }

    diff_invalidate(&curbuf_mut());

    curwin_mut()
        .w_topline
        .set(old_topline.min(curbuf_mut().b_ml.ml_line_count.get()));
    curwin_mut().w_cursor.set(old_cursor);
    check_cursor(&curwin_mut());
    update_topline(&curwin_mut());
    crate::window::check_lnums(false);
    curbuf_mut().b_keep_filetype.set(false);

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&tp) {
            if wp.w_buffer.get() == curwin_mut().w_buffer.get() && !foldmethod_is_manual(&wp) {
                fold_update_all(&wp);
            }
        }
    }

    if orig_mode == curbuf_mut().b_orig_mode.get() {
        curbuf_mut()
            .b_p_ro
            .set(curbuf_mut().b_p_ro.get() | (old_ro));
    }

    do_modelines(0);

    ctx_restore(&mut aco);
}

pub fn buf_store_file_info(buf: &FileBuf, file_info: &mut FileInfo) {
    buf.b_mtime.set(file_info.stat.st_mtim.tv_sec as _);
    buf.b_mtime_ns.set(file_info.stat.st_mtim.tv_nsec as _);
    buf.b_orig_size.set(os_fileinfo_size(file_info));
    buf.b_orig_mode.set(file_info.stat.st_mode as i32);
}

/// Adjust the line with missing eol, used for the next write.
pub fn write_lnum_adjust(offset: Linenr) {
    if curbuf_mut().b_no_eol_lnum.get() != 0 {
        curbuf_mut()
            .b_no_eol_lnum
            .set(curbuf_mut().b_no_eol_lnum.get() + (offset));
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
        let mut tmplen = expand_env(temp_dir, &mut tmp[..TEMP_FILE_PATH_MAXLEN - 64]);
        if !os_isdir(crate::memory::bytes_as_cstr(&tmp)) {
            if temp_dir.to_bytes() == b"$TMPDIR" {
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
                        &tmp,
                    );
                }
            }
            continue;
        }

        if after_pathsep(&tmp[..tmplen]) == 0 {
            tmplen += format_to_buf!(&mut tmp[tmplen..], c"%s", PATHSEPSTR) as usize;
            debug_assert!(tmplen < tmp.len());
        }
        tmplen += format_to_buf!(&mut tmp[tmplen..], c"nvim.%s", &user) as usize;
        debug_assert!(tmplen < tmp.len());
        os_mkdir(crate::memory::bytes_as_cstr(&tmp), 0o700);
        let owned = os_file_owned(crate::memory::bytes_as_cstr(&tmp));
        let isdir = os_isdir(crate::memory::bytes_as_cstr(&tmp));
        #[cfg(unix)]
        let perm = os_getperm(crate::memory::bytes_as_cstr(&tmp));
        #[cfg(unix)]
        let valid = isdir && owned && 0o700 == (perm & 0o777);
        #[cfg(not(unix))]
        let valid = isdir && owned;
        if valid {
            if after_pathsep(&tmp[..tmplen]) == 0 {
                tmplen += format_to_buf!(&mut tmp[tmplen..], c"%s", PATHSEPSTR) as usize;
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
                    &user,
                    &tmp,
                );
            } else if !isdir {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_mktempdir"),
                    0,
                    true,
                    c"tempdir root not a directory: %s",
                    &tmp,
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
                    &tmp,
                );
            }
            tmplen -= crate::memory::bytes_until_nul(&user).len();
            tmp[tmplen] = 0;
        }

        tmplen += format_to_buf!(&mut tmp[tmplen..], c"XXXXXX") as usize;
        debug_assert!(tmplen < tmp.len());
        let _ = tmplen;
        let r = os_mkdtemp(crate::memory::bytes_as_cstr(&tmp), &mut path);
        if r != 0 {
            logmsg!(
                LOG_WARN,
                None,
                Some(c"vim_mktempdir"),
                0,
                true,
                c"tempdir create failed: %s: %s",
                uv_strerror(r),
                &tmp,
            );
            continue;
        }

        if vim_settempdir(crate::memory::bytes_as_cstr(&path)) {
            break;
        }
        os_rmdir(crate::memory::bytes_as_cstr(&path));
    }
    call_umask(umask_save);
}

/// Core part of "readdir()" function.
pub fn readdir_core(
    entries: &mut Vec<std::ffi::CString>,
    path: &CStr,
    mut context: Option<&mut crate::eval::typval::TypVal>,
) -> bool {
    entries.clear();

    let mut dir = Directory::new();
    if !os_scandir(&mut dir, path) {
        smsg!(0, tr(E_NOTOPEN), path);
        return false;
    }

    while let Some(p) = os_scandir_next(&mut dir) {
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

    if os_isrealdir(name) {
        let exp = name.to_owned();
        let mut entries: Vec<std::ffi::CString> = Vec::new();
        if readdir_core(&mut entries, &exp, None) {
            let len = format_to_buf!(&mut *g_st().name_buff.borrow_mut(), c"%s/", &exp) as usize;

            for entry in &entries {
                format_to_buf!(&mut g_st().name_buff.borrow_mut()[len..], c"%s", entry);
                let sub_cstr = g_st().name_buff_cstr();
                if delete_recursive(&sub_cstr) != 0 {
                    result = -1;
                }
            }
            drop(entries);
            if os_rmdir(&exp) != 0 {
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

#[cfg(unix)]
fn vim_opentempdir() {
    with_fio(|st| {
        if st.vim_tempdir_dp.borrow().is_some() {
            return;
        }
        let Some(ref td) = *st.vim_tempdir.borrow() else {
            return;
        };
        *st.vim_tempdir_dp.borrow_mut() = crate::os::fs::OsLockedDir::open_shared(td);
    });
}

#[cfg(unix)]
fn vim_closetempdir() {
    with_fio(|st| *st.vim_tempdir_dp.borrow_mut() = None);
}

/// Delete the temp directory and all files it contains.
pub fn vim_deltempdir() {
    let Some(td) = with_fio(|st| st.vim_tempdir.borrow_mut().take()) else {
        return;
    };

    #[cfg(unix)]
    vim_closetempdir();

    let mut td_vec: Vec<u8> = td.into_bytes_with_nul();
    let tail_ptr = path_tail(crate::memory::bytes_as_cstr(&td_vec)).as_ptr();
    let tail_off = (tail_ptr as usize) - (td_vec.as_ptr() as usize);
    if tail_off > 0 {
        td_vec[tail_off - 1] = 0;
    }
    delete_recursive(crate::memory::bytes_as_cstr(&td_vec));
}

/// Gets path to Nvim's own temp dir (ending with slash).
pub fn vim_gettempdir() -> Option<std::ffi::CString> {
    if with_fio(|st| {
        st.vim_tempdir
            .borrow()
            .as_ref()
            .is_none_or(|td| !os_isdir(td))
    }) {
        if let Some(current_td) = with_fio(|st| st.vim_tempdir.borrow_mut().take()) {
            let notfound = with_fio(|st| {
                let n = st.notfound.get() + 1;
                st.notfound.set(n);
                n
            });
            if notfound == 1 {
                logmsg!(
                    LOG_ERR,
                    None,
                    Some(c"vim_gettempdir"),
                    0,
                    true,
                    c"tempdir disappeared (antivirus or broken cleanup job?): %s",
                    &current_td,
                );
            }
            if notfound > 1 {
                msg_schedule_semsg!(c"E5431: tempdir disappeared (%d times)", notfound);
            }
        }
        vim_mktempdir();
    }
    with_fio(|st| st.vim_tempdir.borrow().clone())
}

/// Sets Nvim's own temporary directory name to `tempdir`.
fn vim_settempdir(tempdir: &CStr) -> bool {
    let mut buf_vec: Vec<u8> = vec![0; MAXPATHL + 2];

    vim_full_name(Some(tempdir), &mut buf_vec[..MAXPATHL], cfg!(windows));

    let mut buflen = crate::memory::bytes_until_nul(&buf_vec).len();
    if after_pathsep(&buf_vec[..buflen]) == 0 {
        let sep_bytes = PATHSEPSTR.to_bytes_with_nul();
        buf_vec[buflen..(buflen + sep_bytes.len())].copy_from_slice(sep_bytes);
        buflen += sep_bytes.len() - 1;
    }
    with_fio(|st| {
        *st.vim_tempdir.borrow_mut() =
            Some(std::ffi::CString::new(&buf_vec[..buflen]).unwrap_or_default());
    });
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
    let _ = format_to_buf!(&mut templ, c"%s%llu", tempdir, count);
    Some(crate::memory::cstring_until_nul(&templ))
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

    regmatch.rm_ic = opt_vars().p_fic.get();
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
    let tail = path_tail(sfname.unwrap_or(c""));

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

pub fn vim_fprintf_fmt(
    stream_file: &crate::os::fs::CFile,
    fmt: &CStr,
    args: &[crate::strings::FmtVal],
) -> i32 {
    let fmt_bytes = fmt.to_bytes();
    if args.is_empty() && !fmt_bytes.contains(&b'%') {
        if fmt_bytes.is_empty() {
            return 0;
        }
        return if stream_file.write_all(fmt_bytes) {
            fmt_bytes.len() as i32
        } else {
            -1
        };
    }
    let mut stack_buf = [0u8; 1024];
    let needed = crate::strings::format_to_buf_fmt(&mut stack_buf, Some(fmt), args);
    if needed < 0 {
        return -1;
    }
    let len = needed as usize;
    if len < stack_buf.len() {
        if len == 0 || stream_file.write_all(&stack_buf[..len]) {
            needed
        } else {
            -1
        }
    } else {
        let mut heap_buf = vec![0u8; len + 1];
        let actual = crate::strings::format_to_buf_fmt(&mut heap_buf, Some(fmt), args);
        if actual < 0 {
            return -1;
        }
        let alen = actual as usize;
        if alen == 0 || stream_file.write_all(&heap_buf[..alen]) {
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
            &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
        )
    };
}
pub use fprintf;

#[macro_export]
macro_rules! printf {
    ($fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::fileio::vim_printf_fmt(
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val(&$arg)),*],
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
            if crate::eval::vars::get_vim_var_str(crate::eval::vars::VV_PROGPATH)
                .to_bytes()
                .is_empty()
            {
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
