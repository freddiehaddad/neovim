//! Multi-level undo facility (`src/nvim/undo.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;

use crate::api::buffer::VisualState;
use crate::api::command::exarg_T;
use crate::api::options::CtxSwitch;
use crate::api::tabpage::{tabpage_T, win_T};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer_updates::{buf_updates_changedtick, buf_updates_unload};
use crate::change::{
    change_warning, changed, changed_bytes, changed_lines, file_ff_differs, unchanged,
};
use crate::clipboard::{VarType, list_T, typval_T};
use crate::context::{ctx_restore, ctx_switch};
use crate::cursor::{
    check_cursor, check_cursor_col, check_cursor_lnum, check_pos, coladvance, colnr_T, getviscol,
    linenr_T,
};
use crate::drawscreen::{redraw_later, redrawWinline};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{EvalFuncData, buf_T, fmark_T, pos_T, visualinfo_T};
use crate::eval::fs::FileInfo;
use crate::extmark::{
    ExtmarkMove, ExtmarkOp, ExtmarkSplice, ExtmarkUndoData, ExtmarkUndoObject, UndoObjectType,
    extmark_apply_undo, extmark_splice_cols, extmark_undo_vec_t,
};
use crate::garray::{ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::input_cmdatom::{atom_did_global_op, atom_origin_pos};
use crate::kvec::KVec;
use crate::map_glyph_cache::NvimString;
use crate::mark::{free_fmark, mark_adjust, setpcmark};
use crate::mbyte::utfc_ptr2len;
use crate::mcursor::mc_undo_time;
use crate::memory::{time_to_bytes, xcalloc, xfree, xmalloc, xmallocz, xstrdup, xstrlcat};
use crate::os::fs::{
    os_close, os_fileinfo_fd, os_fileinfo_size, os_fopen, os_free_acl, os_fsync, os_get_acl,
    os_getperm, os_isdir, os_mkdir_recurse, os_open, os_path_exists, os_remove, os_set_acl,
    os_setperm, vim_acl_T,
};
#[cfg(unix)]
use crate::os::fs::{os_fchown, os_fileinfo, uv_gid_t, uv_uid_t};
use crate::os::input::fast_breakcheck;
use crate::os::time::{Timestamp, Tm, os_localtime_r, os_time, time_t};
use crate::path::{FullName_save, concat_fnames, path_tail, vim_ispathsep, vim_ispathsep_nocolon};
use crate::sha256::{context_sha256_T, sha256_finish, sha256_start, sha256_update};
use crate::state::virtual_active;

pub type varnumber_T = i64;
pub type OptInt = i64;
pub type off_T = i64;
pub type FILE = c_void;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const EOF: c_int = -1;

const MAXLNUM: linenr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const NMARKS: usize = 26;
const UNDO_HASH_SIZE: usize = 32;
const NO_LOCAL_UNDOLEVEL: OptInt = -123456;

const UH_CHANGED: c_int = 0x01;
const UH_EMPTYBUF: c_int = 0x02;
const UH_RELOAD: c_int = 0x04;

const ML_EMPTY: c_int = 0x01;
const BL_SOL: c_int = 2;
const BL_FIX: c_int = 4;
const UPD_NOT_VALID: c_int = 30;
const HLF_T: c_int = 23;

const kExtmarkNOOP: ExtmarkOp = 0;
const kExtmarkUndo: ExtmarkOp = 1;
const kExtmarkSplice: UndoObjectType = 0;
const kExtmarkMove: UndoObjectType = 1;
const kListLenMayKnow: isize = -3;

const kCpoUndo: c_int = b'u' as c_int;
const kShmUndo: c_int = b'u' as c_int;
const kOptFdoFlagUndo: c_uint = 0x200;

const UF_START_MAGIC: &[u8; 9] = b"Vim\x9fUnDo\xe5";
const UF_START_MAGIC_LEN: usize = 9;
const UF_HEADER_MAGIC: c_int = 0x5fd0;
const UF_HEADER_END_MAGIC: c_int = 0xe7aa;
const UF_ENTRY_MAGIC: c_int = 0xf518;
const UF_ENTRY_END_MAGIC: c_int = 0x3581;
const UF_VERSION: c_int = 3;
const UF_LAST_SAVE_NR: c_int = 1;
const UHP_SAVE_NR: c_int = 1;

const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;

#[cfg(windows)]
const O_CREAT: c_int = 0x0100;
#[cfg(windows)]
const O_EXCL: c_int = 0x0400;
#[cfg(windows)]
const O_NOFOLLOW: c_int = 0;

#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "linux")]
const O_EXCL: c_int = 0o200;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "arm", target_arch = "aarch64"))
))]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(all(target_os = "linux", any(target_arch = "arm", target_arch = "aarch64")))]
const O_NOFOLLOW: c_int = 0o100000;

#[cfg(not(any(windows, target_os = "linux")))]
const O_NOFOLLOW: c_int = 0x0100;
#[cfg(not(any(windows, target_os = "linux")))]
const O_CREAT: c_int = 0x0200;
#[cfg(not(any(windows, target_os = "linux")))]
const O_EXCL: c_int = 0x0800;

const E_UNDO_LIST_CORRUPT: &CStr = c"E439: Undo list corrupt";
const E_UNDO_LINE_MISSING: &CStr = c"E440: Undo line missing";
const E_WRITE_ERROR_IN_UNDO_FILE_STR: &CStr = c"E829: Write error in undo file: %s";
const E_NOT_OPEN: &CStr = c"E828: Cannot open undo file for writing: %s";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct u_entry_T {
    pub ue_next: *mut u_entry_T,
    pub ue_top: linenr_T,
    pub ue_bot: linenr_T,
    pub ue_lcount: linenr_T,
    pub ue_array: *mut *mut c_char,
    pub ue_size: linenr_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union u_header_link_T {
    pub ptr: *mut u_header_T,
    pub seq: c_int,
}

#[repr(C)]
pub struct u_header_T {
    pub uh_next: u_header_link_T,
    pub uh_prev: u_header_link_T,
    pub uh_alt_next: u_header_link_T,
    pub uh_alt_prev: u_header_link_T,
    pub uh_seq: c_int,
    pub uh_walk: c_int,
    pub uh_entry: *mut u_entry_T,
    pub uh_getbot_entry: *mut u_entry_T,
    pub uh_cursor: pos_T,
    pub uh_cursor_after: pos_T,
    pub uh_cursor_vcol: colnr_T,
    pub uh_flags: c_int,
    pub uh_namedm: [fmark_T; NMARKS],
    pub uh_extmark: extmark_undo_vec_t,
    pub uh_visual: visualinfo_T,
    pub uh_time: time_t,
    pub uh_save_nr: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UndoCheckpoint {
    pub uc_oldhead: *mut u_header_T,
    pub uc_newhead: *mut u_header_T,
    pub uc_curhead: *mut u_header_T,
    pub uc_numhead: c_int,
    pub uc_synced: bool,
    pub uc_seq_last: c_int,
    pub uc_save_nr_last: c_int,
    pub uc_seq_cur: c_int,
    pub uc_time_cur: time_t,
    pub uc_save_nr_cur: c_int,
    pub uc_line_ptr: *mut c_char,
    pub uc_line_lnum: linenr_T,
    pub uc_line_colnr: colnr_T,
    pub uc_undolevels: OptInt,
    pub uc_changedtick: varnumber_T,
}

struct bufinfo_T {
    bi_buf: *mut buf_T,
    bi_fp: *mut FILE,
    bi_fsize: off_T,
}

static mut u_newcount: c_int = 0;
static mut u_oldcount: c_int = 0;
static mut undo_undoes: bool = false;
static mut lastmark: c_int = 0;

unsafe extern "C" {
    static mut curwin: *mut win_T;
    static mut firstwin: *mut win_T;
    static mut curbuf: *mut buf_T;
    static mut curtab: *mut tabpage_T;
    static mut firstbuf: *mut buf_T;
    static mut Visual: VisualState;
    static mut KeyTyped: bool;
    static mut got_int: bool;
    static mut sandbox: c_int;
    static mut textlock: c_int;
    static mut no_u_sync: c_int;
    static mut global_busy: c_int;
    static mut IObuff: [c_char; IOSIZE];

    static mut p_ul: OptInt;
    static mut p_udir: *mut c_char;
    static mut p_verbose: OptInt;
    static mut p_fs: c_int;
    static mut p_cpo: *mut c_char;
    static mut fdo_flags: c_uint;

    static e_modifiable: [c_char; 0];
    static e_sandbox: [c_char; 0];
    static e_textlock: [c_char; 0];

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn ngettext(msgid1: *const c_char, msgid2: *const c_char, n: c_ulong) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn iemsg(s: *const c_char);
    fn internal_error(where_: *const c_char);
    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn smsg(hl_id: c_int, s: *const c_char, ...) -> c_int;
    fn smsg_keep(hl_id: c_int, s: *const c_char, ...) -> c_int;
    fn verb_msg(s: *const c_char) -> c_int;
    fn give_warning(message: *const c_char, hl: bool, with_source: bool);
    fn messaging() -> bool;
    fn msg_ext_set_kind(msg_kind: *const c_char);
    fn msg_start();
    fn msg_end() -> bool;
    fn msg_putchar(c: c_int);
    fn msg_puts(s: *const c_char);
    fn msg_puts_hl(s: *const c_char, hl_id: c_int, hist: bool);
    fn verbose_enter();
    fn verbose_leave();
    fn beep_flush();

    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn vim_snprintf_add(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn sort_strings(files: *mut *mut c_char, count: c_int);
    fn copy_option_part(
        option: *mut *mut c_char,
        buf: *mut c_char,
        maxlen: usize,
        sep_chars: *const c_char,
    ) -> usize;
    fn shortmess(x: c_int) -> bool;

    fn expr_map_locked() -> bool;
    fn foldOpenCursor();
    fn text_locked() -> bool;
    fn text_locked_msg();
    fn beginline(flags: c_int);
    fn spell_check_window(wp: *mut win_T) -> bool;

    fn ml_get(lnum: linenr_T) -> *mut c_char;
    fn ml_get_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char;
    fn ml_delete(lnum: linenr_T) -> c_int;
    fn ml_replace(lnum: linenr_T, line: *mut c_char, copy: bool) -> c_int;
    fn ml_append_flags(lnum: linenr_T, line: *mut c_char, len: colnr_T, flags: c_int) -> c_int;
    #[cfg(unix)]
    fn resolve_symlink(fname: *const c_char, buf: *mut c_char) -> c_int;

    fn buf_is_empty(buf: *mut buf_T) -> bool;
    fn bt_prompt(buf: *mut buf_T) -> bool;
    fn bt_dontwrite(buf: *const buf_T) -> bool;
    fn buf_set_changedtick(buf: *mut buf_T, changedtick: varnumber_T);

    fn read_eintr(fd: c_int, buf: *mut c_void, bufsize: usize) -> isize;
    fn get2c(fd: *mut FILE) -> c_int;
    fn get4c(fd: *mut FILE) -> c_int;
    fn get8ctime(fd: *mut FILE) -> time_t;
    fn uv_strerror(err: c_int) -> *const c_char;

    fn tv_list_alloc(len: isize) -> *mut list_T;
    fn tv_list_append_dict(l: *mut list_T, dict: *mut dict_T);
    fn tv_dict_alloc() -> *mut dict_T;
    fn tv_dict_alloc_ret(ret_tv: *mut typval_T);
    fn tv_dict_add_nr(d: *mut dict_T, key: *const c_char, key_len: usize, nr: varnumber_T)
    -> c_int;
    fn tv_dict_add_list(
        d: *mut dict_T,
        key: *const c_char,
        key_len: usize,
        list: *mut list_T,
    ) -> c_int;
    fn tv_get_string(tv: *const typval_T) -> *const c_char;
    fn get_buf_arg(arg: *mut typval_T) -> *mut buf_T;

    #[cfg_attr(target_os = "windows", link_name = "_time64")]
    #[cfg_attr(not(target_os = "windows"), link_name = "time")]
    fn c_time(tloc: *mut time_t) -> time_t;
    fn strftime(s: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;

    #[cfg(windows)]
    fn _fdopen(fd: c_int, mode: *const c_char) -> *mut FILE;
    #[cfg(not(windows))]
    fn fdopen(fd: c_int, mode: *const c_char) -> *mut FILE;
    #[cfg(windows)]
    fn _fileno(stream: *mut FILE) -> c_int;
    #[cfg(not(windows))]
    fn fileno(stream: *mut FILE) -> c_int;
    #[cfg(windows)]
    fn _ftelli64(stream: *mut FILE) -> i64;
    #[cfg(not(windows))]
    fn ftello(stream: *mut FILE) -> off_T;
    #[cfg(unix)]
    fn getuid() -> u32;

    fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    fn fflush(stream: *mut FILE) -> c_int;
    fn fclose(stream: *mut FILE) -> c_int;
    fn fgetc(stream: *mut FILE) -> c_int;
    fn feof(stream: *mut FILE) -> c_int;
    fn qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
    );
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strlen(s: *const c_char) -> usize;
    fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int;
}

#[inline]
unsafe fn c_fdopen(fd: c_int, mode: *const c_char) -> *mut FILE {
    #[cfg(windows)]
    {
        unsafe { _fdopen(fd, mode) }
    }
    #[cfg(not(windows))]
    {
        unsafe { fdopen(fd, mode) }
    }
}

#[inline]
unsafe fn c_fileno(stream: *mut FILE) -> c_int {
    #[cfg(windows)]
    {
        unsafe { _fileno(stream) }
    }
    #[cfg(not(windows))]
    {
        unsafe { fileno(stream) }
    }
}

#[inline]
unsafe fn vim_ftell(stream: *mut FILE) -> off_T {
    #[cfg(windows)]
    {
        unsafe { _ftelli64(stream) }
    }
    #[cfg(not(windows))]
    {
        unsafe { ftello(stream) }
    }
}

#[inline]
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
fn clearpos(a: &mut pos_T) {
    a.lnum = 0;
    a.col = 0;
    a.coladd = 0;
}

#[inline]
unsafe fn buf_get_changedtick(buf: *const buf_T) -> varnumber_T {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

/// Save the current line for both the `"u"` and `"U"` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_save_cursor() -> c_int {
    unsafe {
        let cur = (*curwin).w_cursor.lnum;
        let top = if cur > 0 { cur - 1 } else { 0 };
        let bot = cur + 1;
        u_save(top, bot)
    }
}

/// Save the lines between `top` and `bot` for both the `"u"` and `"U"` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_save(top: linenr_T, bot: linenr_T) -> c_int {
    unsafe { u_save_buf(curbuf, top, bot) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_save_buf(buf: *mut buf_T, top: linenr_T, bot: linenr_T) -> c_int {
    unsafe {
        if top >= bot || bot > (*buf).b_ml.ml_line_count + 1 {
            return FAIL;
        }

        if top + 2 == bot {
            u_saveline(buf, top + 1);
        }

        u_savecommon(buf, top, bot, 0, false)
    }
}

/// Save the line `lnum` (used by `":s"` and `"~"` command).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_savesub(lnum: linenr_T) -> c_int {
    unsafe { u_savecommon(curbuf, lnum - 1, lnum + 1, lnum + 1, false) }
}

/// A new line is inserted before line `lnum` (used by `:s` command).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_inssub(lnum: linenr_T) -> c_int {
    unsafe { u_savecommon(curbuf, lnum - 1, lnum, lnum + 1, false) }
}

/// Save the lines `lnum` - `lnum + nlines` (used by delete command).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_savedel(lnum: linenr_T, nlines: linenr_T) -> c_int {
    unsafe {
        u_savecommon(
            curbuf,
            lnum - 1,
            lnum + nlines,
            if nlines == (*curbuf).b_ml.ml_line_count {
                2
            } else {
                lnum
            },
            false,
        )
    }
}

/// Return true when undo is allowed. Otherwise print an error message and return false.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn undo_allowed(buf: *mut buf_T) -> bool {
    unsafe {
        if (*buf).b_p_ma == 0 {
            emsg(gettext((&raw const e_modifiable).cast()));
            return false;
        }

        if sandbox != 0 {
            emsg(gettext((&raw const e_sandbox).cast()));
            return false;
        }

        if textlock != 0 || expr_map_locked() {
            emsg(gettext((&raw const e_textlock).cast()));
            return false;
        }

        true
    }
}

unsafe fn get_undolevel(buf: *mut buf_T) -> OptInt {
    unsafe {
        if (*buf).b_p_ul == NO_LOCAL_UNDOLEVEL {
            p_ul
        } else {
            (*buf).b_p_ul
        }
    }
}

#[inline]
unsafe fn zero_fmark_additional_data(fmarks: *mut fmark_T) {
    unsafe {
        for i in 0..NMARKS {
            xfree((*fmarks.add(i)).additional_data.cast());
            (*fmarks.add(i)).additional_data = ptr::null_mut();
        }
    }
}

/// Common code for various ways to save text before a change.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_savecommon(
    buf: *mut buf_T,
    top: linenr_T,
    bot: linenr_T,
    newbot: linenr_T,
    reload: bool,
) -> c_int {
    unsafe {
        if !reload {
            if !undo_allowed(buf) {
                return FAIL;
            }

            if buf == curbuf {
                change_warning(buf, 0);
            }

            if bot > (*buf).b_ml.ml_line_count + 1 {
                emsg(gettext(c"E881: Line count changed unexpectedly".as_ptr()));
                return FAIL;
            }
        }

        let size = bot - top - 1;

        if (*buf).b_u_synced {
            (*buf).b_new_change = true;

            let uhp: *mut u_header_T = if get_undolevel(buf) >= 0 {
                let p = xmalloc(size_of::<u_header_T>()).cast::<u_header_T>();
                (*p).uh_extmark = KVec::new();
                p
            } else {
                ptr::null_mut()
            };

            let mut old_curhead = (*buf).b_u_curhead.cast::<u_header_T>();
            if !old_curhead.is_null() {
                (*buf).b_u_newhead = (*old_curhead).uh_next.ptr.cast();
                (*buf).b_u_curhead = ptr::null_mut();
            }

            while ((*buf).b_u_numhead as OptInt) > get_undolevel(buf)
                && !(*buf).b_u_oldhead.is_null()
            {
                let mut uhfree = (*buf).b_u_oldhead.cast::<u_header_T>();

                if uhfree == old_curhead {
                    u_freebranch(buf, uhfree, &raw mut old_curhead);
                } else if (*uhfree).uh_alt_next.ptr.is_null() {
                    u_freeheader(buf, uhfree, &raw mut old_curhead);
                } else {
                    while !(*uhfree).uh_alt_next.ptr.is_null() {
                        uhfree = (*uhfree).uh_alt_next.ptr;
                    }
                    u_freebranch(buf, uhfree, &raw mut old_curhead);
                }
            }

            if uhp.is_null() {
                if !old_curhead.is_null() {
                    u_freebranch(buf, old_curhead, ptr::null_mut());
                }
                (*buf).b_u_synced = false;
                return OK;
            }

            (*uhp).uh_prev.ptr = ptr::null_mut();
            (*uhp).uh_next.ptr = (*buf).b_u_newhead.cast::<u_header_T>();
            (*uhp).uh_alt_next.ptr = old_curhead;
            if !old_curhead.is_null() {
                (*uhp).uh_alt_prev.ptr = (*old_curhead).uh_alt_prev.ptr;
                if !(*uhp).uh_alt_prev.ptr.is_null() {
                    (*(*uhp).uh_alt_prev.ptr).uh_alt_next.ptr = uhp;
                }
                (*old_curhead).uh_alt_prev.ptr = uhp;
                if (*buf).b_u_oldhead.cast::<u_header_T>() == old_curhead {
                    (*buf).b_u_oldhead = uhp.cast();
                }
            } else {
                (*uhp).uh_alt_prev.ptr = ptr::null_mut();
            }

            if !(*buf).b_u_newhead.is_null() {
                (*(*buf).b_u_newhead.cast::<u_header_T>()).uh_prev.ptr = uhp;
            }

            (*buf).b_u_seq_last += 1;
            (*uhp).uh_seq = (*buf).b_u_seq_last;
            (*buf).b_u_seq_cur = (*uhp).uh_seq;
            (*uhp).uh_time = c_time(ptr::null_mut());
            (*uhp).uh_save_nr = 0;
            (*buf).b_u_time_cur = (*uhp).uh_time + 1;

            (*uhp).uh_walk = 0;
            (*uhp).uh_entry = ptr::null_mut();
            (*uhp).uh_getbot_entry = ptr::null_mut();
            (*uhp).uh_cursor = atom_origin_pos(buf);
            if virtual_active(curwin)
                && (*curwin).w_cursor.coladd > 0
                && equalpos((*uhp).uh_cursor, (*curwin).w_cursor)
            {
                (*uhp).uh_cursor_vcol = getviscol();
            } else {
                (*uhp).uh_cursor_vcol = -1;
            }
            clearpos(&mut (*uhp).uh_cursor_after);

            (*uhp).uh_flags = (if (*buf).b_changed != 0 { UH_CHANGED } else { 0 })
                + (if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 {
                    UH_EMPTYBUF
                } else {
                    0
                });

            zero_fmark_additional_data((&raw mut (*buf).b_namedm).cast::<fmark_T>());
            (*uhp).uh_namedm = (*buf).b_namedm;
            (*uhp).uh_visual = (*buf).b_visual;

            (*buf).b_u_newhead = uhp.cast();
            if (*buf).b_u_oldhead.is_null() {
                (*buf).b_u_oldhead = uhp.cast();
            }
            (*buf).b_u_numhead += 1;
        } else {
            if get_undolevel(buf) < 0 {
                return OK;
            }

            if size == 1 {
                let mut uep = u_get_headentry(buf);
                let mut prev_uep: *mut u_entry_T = ptr::null_mut();
                for i in 0..10 {
                    if uep.is_null() {
                        break;
                    }

                    if u_entry_resized(buf, uep)
                        || ((*uep).ue_size > 1
                            && top >= (*uep).ue_top
                            && top + 2 <= (*uep).ue_top + (*uep).ue_size + 1)
                    {
                        break;
                    }

                    if (*uep).ue_size == 1 && (*uep).ue_top == top {
                        let newhead = (*buf).b_u_newhead.cast::<u_header_T>();
                        if i > 0 {
                            u_getbot(buf);
                            (*buf).b_u_synced = false;

                            (*prev_uep).ue_next = (*uep).ue_next;
                            (*uep).ue_next = (*newhead).uh_entry;
                            (*newhead).uh_entry = uep;
                        }

                        if newbot != 0 {
                            (*uep).ue_bot = newbot;
                        } else if bot > (*buf).b_ml.ml_line_count {
                            (*uep).ue_bot = 0;
                        } else {
                            (*uep).ue_lcount = (*buf).b_ml.ml_line_count;
                            (*newhead).uh_getbot_entry = uep;
                        }
                        return OK;
                    }
                    prev_uep = uep;
                    uep = (*uep).ue_next;
                }
            }

            u_getbot(buf);
        }

        let uep = xmalloc(size_of::<u_entry_T>()).cast::<u_entry_T>();
        *uep = u_entry_T {
            ue_next: ptr::null_mut(),
            ue_top: top,
            ue_bot: 0,
            ue_lcount: 0,
            ue_array: ptr::null_mut(),
            ue_size: size,
        };

        let newhead = (*buf).b_u_newhead.cast::<u_header_T>();
        if newbot != 0 {
            (*uep).ue_bot = newbot;
        } else if bot > (*buf).b_ml.ml_line_count {
            (*uep).ue_bot = 0;
        } else {
            (*uep).ue_lcount = (*buf).b_ml.ml_line_count;
            (*newhead).uh_getbot_entry = uep;
        }

        if size > 0 {
            (*uep).ue_array =
                xmalloc(size_of::<*mut c_char>() * (size as usize)).cast::<*mut c_char>();
            for i in 0..size {
                fast_breakcheck();
                if got_int {
                    u_freeentry(uep, i);
                    return FAIL;
                }
                *(*uep).ue_array.add(i as usize) = u_save_line_buf(buf, top + 1 + i);
            }
        } else {
            (*uep).ue_array = ptr::null_mut();
        }

        (*uep).ue_next = (*newhead).uh_entry;
        (*newhead).uh_entry = uep;
        if reload {
            (*newhead).uh_flags |= UH_RELOAD;
        }
        (*buf).b_u_synced = false;
        undo_undoes = false;

        OK
    }
}

unsafe fn u_entry_resized(buf: *mut buf_T, uep: *mut u_entry_T) -> bool {
    unsafe {
        let newhead = (*buf).b_u_newhead.cast::<u_header_T>();
        if (*newhead).uh_getbot_entry != uep {
            (*uep).ue_top + (*uep).ue_size + 1
                != if (*uep).ue_bot == 0 {
                    (*buf).b_ml.ml_line_count + 1
                } else {
                    (*uep).ue_bot
                }
        } else {
            (*uep).ue_lcount != (*buf).b_ml.ml_line_count
        }
    }
}

/// Compute the hash for a buffer text into `hash[UNDO_HASH_SIZE]`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_compute_hash(buf: *mut buf_T, hash: *mut u8) {
    unsafe {
        let mut ctx = context_sha256_T::default();
        sha256_start(&raw mut ctx);
        for lnum in 1..=(*buf).b_ml.ml_line_count {
            let p = ml_get_buf(buf, lnum);
            sha256_update(&raw mut ctx, p.cast::<u8>(), strlen(p) + 1);
        }
        sha256_finish(&raw mut ctx, hash);
    }
}

/// Return an allocated string of the full path of the target undofile.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_get_undo_file_name(
    buf_ffname: *const c_char,
    reading: bool,
) -> *mut c_char {
    unsafe {
        #[allow(unused_mut)]
        let mut ffname = buf_ffname;
        if ffname.is_null() {
            return ptr::null_mut();
        }

        #[cfg(unix)]
        let mut fname_buf = [0 as c_char; MAXPATHL];
        #[cfg(unix)]
        {
            if resolve_symlink(ffname, fname_buf.as_mut_ptr()) == OK {
                ffname = fname_buf.as_ptr();
            }
        }

        let mut dir_name = [0 as c_char; MAXPATHL + 1];
        let mut munged_name = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let mut undo_file_name: *mut c_char = ptr::null_mut();

        let ffname_len = strlen(ffname);
        let mut dirp = p_udir;
        while *dirp != NUL {
            let mut dir_len = copy_option_part(
                &raw mut dirp,
                dir_name.as_mut_ptr(),
                MAXPATHL,
                c",".as_ptr(),
            );
            if dir_len == 1 && dir_name[0] == b'.' as c_char {
                undo_file_name = xmalloc(ffname_len + 6).cast::<c_char>();
                ptr::copy(ffname, undo_file_name, ffname_len + 1);
                let tail = path_tail(undo_file_name);
                let tail_len = ffname_len - (tail.offset_from(undo_file_name) as usize);
                ptr::copy(tail, tail.add(1), tail_len + 1);
                *tail = b'.' as c_char;
                ptr::copy(c".un~".as_ptr(), tail.add(tail_len + 1), 5);
            } else {
                dir_name[dir_len] = NUL;

                while dir_len > 1 && vim_ispathsep_nocolon(dir_name[dir_len - 1] as c_int) {
                    dir_len -= 1;
                    dir_name[dir_len] = NUL;
                }

                let mut has_directory = os_isdir(dir_name.as_ptr());
                if !has_directory && *dirp == NUL && !reading {
                    let mut failed_dir: *mut c_char = ptr::null_mut();
                    let ret = os_mkdir_recurse(
                        dir_name.as_ptr(),
                        0o755,
                        &raw mut failed_dir,
                        ptr::null_mut(),
                    );
                    if ret != 0 {
                        semsg(
                            gettext(
                                c"E5003: Unable to create directory \"%s\" for undo file: %s"
                                    .as_ptr(),
                            ),
                            failed_dir,
                            uv_strerror(ret),
                        );
                        xfree(failed_dir.cast());
                    } else {
                        has_directory = true;
                    }
                }
                if has_directory {
                    if munged_name.data.is_null() {
                        let d = xmallocz(ffname_len).cast::<c_char>();
                        ptr::copy_nonoverlapping(ffname, d, ffname_len);
                        munged_name = NvimString {
                            data: d,
                            size: ffname_len,
                        };
                        let mut p = munged_name.data;
                        while *p != NUL {
                            if vim_ispathsep(*p as c_int) {
                                *p = b'%' as c_char;
                            }
                            p = p.add(utfc_ptr2len(p) as usize);
                        }
                    }
                    let dir_str = NvimString {
                        data: dir_name.as_mut_ptr(),
                        size: dir_len,
                    };
                    undo_file_name = concat_fnames(dir_str, munged_name, true).data;
                }
            }

            if !undo_file_name.is_null() && (!reading || os_path_exists(undo_file_name)) {
                break;
            }
            xfree(undo_file_name.cast());
            undo_file_name = ptr::null_mut();
        }

        xfree(munged_name.data.cast());
        undo_file_name
    }
}

unsafe fn corruption_error(mesg: *const c_char, file_name: *const c_char) {
    unsafe {
        semsg(
            gettext(c"E825: Corrupted undo file (%s): %s".as_ptr()),
            mesg,
            file_name,
        );
    }
}

unsafe fn u_free_uhp(uhp: *mut u_header_T) {
    unsafe {
        let mut uep = (*uhp).uh_entry;
        while !uep.is_null() {
            let nuep = (*uep).ue_next;
            u_freeentry(uep, (*uep).ue_size);
            uep = nuep;
        }
        (*uhp).uh_extmark.destroy();
        xfree(uhp.cast());
    }
}

unsafe fn serialize_header(bi: &mut bufinfo_T, hash: *mut u8) -> bool {
    unsafe {
        let buf = bi.bi_buf;
        let fp = bi.bi_fp;

        if fwrite(UF_START_MAGIC.as_ptr().cast(), UF_START_MAGIC_LEN, 1, fp) != 1 {
            return false;
        }

        undo_write_bytes(bi, UF_VERSION as u64, 2);

        if !undo_write(bi, hash, UNDO_HASH_SIZE) {
            return false;
        }

        undo_write_bytes(bi, (*buf).b_ml.ml_line_count as u64, 4);
        let len = if !(*buf).b_u_line_ptr.is_null() {
            strlen((*buf).b_u_line_ptr)
        } else {
            0
        };
        undo_write_bytes(bi, len as u64, 4);
        if len > 0 && !undo_write(bi, (*buf).b_u_line_ptr.cast::<u8>(), len) {
            return false;
        }
        undo_write_bytes(bi, (*buf).b_u_line_lnum as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_line_colnr as u64, 4);

        put_header_ptr(bi, (*buf).b_u_oldhead.cast::<u_header_T>());
        put_header_ptr(bi, (*buf).b_u_newhead.cast::<u_header_T>());
        put_header_ptr(bi, (*buf).b_u_curhead.cast::<u_header_T>());

        undo_write_bytes(bi, (*buf).b_u_numhead as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_seq_last as u64, 4);
        undo_write_bytes(bi, (*buf).b_u_seq_cur as u64, 4);
        let mut time_buf = [0u8; 8];
        time_to_bytes((*buf).b_u_time_cur, time_buf.as_mut_ptr());
        undo_write(bi, time_buf.as_mut_ptr(), time_buf.len());

        undo_write_bytes(bi, 4, 1);
        undo_write_bytes(bi, UF_LAST_SAVE_NR as u64, 1);
        undo_write_bytes(bi, (*buf).b_u_save_nr_last as u64, 4);

        undo_write_bytes(bi, 0, 1);

        true
    }
}

unsafe fn serialize_uhp(bi: &mut bufinfo_T, uhp: *mut u_header_T) -> bool {
    unsafe {
        if !undo_write_bytes(bi, UF_HEADER_MAGIC as u64, 2) {
            return false;
        }

        put_header_ptr(bi, (*uhp).uh_next.ptr);
        put_header_ptr(bi, (*uhp).uh_prev.ptr);
        put_header_ptr(bi, (*uhp).uh_alt_next.ptr);
        put_header_ptr(bi, (*uhp).uh_alt_prev.ptr);
        undo_write_bytes(bi, (*uhp).uh_seq as u64, 4);
        serialize_pos(bi, (*uhp).uh_cursor);
        undo_write_bytes(bi, (*uhp).uh_cursor_vcol as u64, 4);
        undo_write_bytes(bi, (*uhp).uh_flags as u64, 2);
        for i in 0..NMARKS {
            serialize_pos(bi, (*uhp).uh_namedm[i].mark);
        }
        serialize_visualinfo(bi, &raw mut (*uhp).uh_visual);
        let mut time_buf = [0u8; 8];
        time_to_bytes((*uhp).uh_time, time_buf.as_mut_ptr());
        undo_write(bi, time_buf.as_mut_ptr(), time_buf.len());

        undo_write_bytes(bi, 4, 1);
        undo_write_bytes(bi, UHP_SAVE_NR as u64, 1);
        undo_write_bytes(bi, (*uhp).uh_save_nr as u64, 4);

        undo_write_bytes(bi, 0, 1);

        let mut uep = (*uhp).uh_entry;
        while !uep.is_null() {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            if !serialize_uep(bi, uep) {
                return false;
            }
            uep = (*uep).ue_next;
        }
        undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

        for i in 0..(*uhp).uh_extmark.size {
            if !serialize_extmark(bi, *(*uhp).uh_extmark.items.add(i)) {
                return false;
            }
        }
        undo_write_bytes(bi, UF_ENTRY_END_MAGIC as u64, 2);

        true
    }
}

unsafe fn unserialize_uhp(bi: &mut bufinfo_T, file_name: *const c_char) -> *mut u_header_T {
    unsafe {
        let uhp = xmalloc(size_of::<u_header_T>()).cast::<u_header_T>();
        ptr::write_bytes(uhp, 0, 1);
        (*uhp).uh_next.seq = undo_read_4c(bi);
        (*uhp).uh_prev.seq = undo_read_4c(bi);
        (*uhp).uh_alt_next.seq = undo_read_4c(bi);
        (*uhp).uh_alt_prev.seq = undo_read_4c(bi);
        (*uhp).uh_seq = undo_read_4c(bi);
        if (*uhp).uh_seq <= 0 {
            corruption_error(c"uh_seq".as_ptr(), file_name);
            xfree(uhp.cast());
            return ptr::null_mut();
        }
        unserialize_pos(bi, &raw mut (*uhp).uh_cursor);
        (*uhp).uh_cursor_vcol = undo_read_4c(bi);
        (*uhp).uh_flags = undo_read_2c(bi);
        let cur_timestamp: Timestamp = os_time();
        for i in 0..NMARKS {
            unserialize_pos(bi, &raw mut (*uhp).uh_namedm[i].mark);
            (*uhp).uh_namedm[i].timestamp = cur_timestamp;
            (*uhp).uh_namedm[i].fnum = 0;
        }
        unserialize_visualinfo(bi, &raw mut (*uhp).uh_visual);
        (*uhp).uh_time = undo_read_time(bi);

        loop {
            let mut len = undo_read_byte(bi);
            if len == EOF {
                corruption_error(c"truncated".as_ptr(), file_name);
                u_free_uhp(uhp);
                return ptr::null_mut();
            }
            if len == 0 {
                break;
            }
            let what = undo_read_byte(bi);
            match what {
                UHP_SAVE_NR => {
                    (*uhp).uh_save_nr = undo_read_4c(bi);
                }
                _ => {
                    while {
                        len -= 1;
                        len >= 0
                    } {
                        undo_read_byte(bi);
                    }
                }
            }
        }

        let mut last_uep: *mut u_entry_T = ptr::null_mut();
        let mut c = undo_read_2c(bi);
        while c == UF_ENTRY_MAGIC {
            let mut error = false;
            let uep = unserialize_uep(bi, &mut error, file_name);
            if last_uep.is_null() {
                (*uhp).uh_entry = uep;
            } else {
                (*last_uep).ue_next = uep;
            }
            last_uep = uep;
            if error {
                u_free_uhp(uhp);
                return ptr::null_mut();
            }
            c = undo_read_2c(bi);
        }
        if c != UF_ENTRY_END_MAGIC {
            corruption_error(c"entry end".as_ptr(), file_name);
            u_free_uhp(uhp);
            return ptr::null_mut();
        }

        (*uhp).uh_extmark = KVec::new();
        c = undo_read_2c(bi);
        while c == UF_ENTRY_MAGIC {
            let mut error = false;
            let extup = unserialize_extmark(bi, &mut error, file_name);
            if error {
                xfree(extup.cast());
                u_free_uhp(uhp);
                return ptr::null_mut();
            }
            (*uhp).uh_extmark.push(*extup);
            xfree(extup.cast());
            c = undo_read_2c(bi);
        }
        if c != UF_ENTRY_END_MAGIC {
            corruption_error(c"entry end".as_ptr(), file_name);
            u_free_uhp(uhp);
            return ptr::null_mut();
        }

        uhp
    }
}

unsafe fn serialize_extmark(bi: &mut bufinfo_T, extup: ExtmarkUndoObject) -> bool {
    unsafe {
        if extup.type_ == kExtmarkSplice {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            undo_write_bytes(bi, extup.type_ as u64, 4);
            if !undo_write(
                bi,
                (&raw const extup.data.splice).cast::<u8>().cast_mut(),
                size_of::<ExtmarkSplice>(),
            ) {
                return false;
            }
        } else if extup.type_ == kExtmarkMove {
            undo_write_bytes(bi, UF_ENTRY_MAGIC as u64, 2);
            undo_write_bytes(bi, extup.type_ as u64, 4);
            if !undo_write(
                bi,
                (&raw const extup.data.move_).cast::<u8>().cast_mut(),
                size_of::<ExtmarkMove>(),
            ) {
                return false;
            }
        }
        true
    }
}

unsafe fn unserialize_extmark(
    bi: &mut bufinfo_T,
    error: &mut bool,
    _filename: *const c_char,
) -> *mut ExtmarkUndoObject {
    unsafe {
        let buf: *mut u8;
        let extup = xmalloc(size_of::<ExtmarkUndoObject>()).cast::<ExtmarkUndoObject>();
        let type_ = undo_read_4c(bi) as UndoObjectType;
        (*extup).type_ = type_;
        if type_ == kExtmarkSplice {
            let n_elems = size_of::<ExtmarkSplice>();
            buf = xcalloc(n_elems, 1).cast::<u8>();
            if !undo_read(bi, buf, n_elems) {
                xfree(extup.cast());
                xfree(buf.cast());
                *error = true;
                return ptr::null_mut();
            }
            (*extup).data = ExtmarkUndoData {
                splice: *buf.cast::<ExtmarkSplice>(),
            };
        } else if type_ == kExtmarkMove {
            let n_elems = size_of::<ExtmarkMove>();
            buf = xcalloc(n_elems, 1).cast::<u8>();
            if !undo_read(bi, buf, n_elems) {
                xfree(extup.cast());
                xfree(buf.cast());
                *error = true;
                return ptr::null_mut();
            }
            (*extup).data = ExtmarkUndoData {
                move_: *buf.cast::<ExtmarkMove>(),
            };
        } else {
            xfree(extup.cast());
            *error = true;
            return ptr::null_mut();
        }

        xfree(buf.cast());
        extup
    }
}

unsafe fn serialize_uep(bi: &mut bufinfo_T, uep: *mut u_entry_T) -> bool {
    unsafe {
        undo_write_bytes(bi, (*uep).ue_top as u64, 4);
        undo_write_bytes(bi, (*uep).ue_bot as u64, 4);
        undo_write_bytes(bi, (*uep).ue_lcount as u64, 4);
        undo_write_bytes(bi, (*uep).ue_size as u64, 4);

        for i in 0..(*uep).ue_size as usize {
            let line = *(*uep).ue_array.add(i);
            let len = strlen(line);
            if !undo_write_bytes(bi, len as u64, 4) {
                return false;
            }
            if len > 0 && !undo_write(bi, line.cast::<u8>(), len) {
                return false;
            }
        }
        true
    }
}

unsafe fn unserialize_uep(
    bi: &mut bufinfo_T,
    error: &mut bool,
    file_name: *const c_char,
) -> *mut u_entry_T {
    unsafe {
        let uep = xmalloc(size_of::<u_entry_T>()).cast::<u_entry_T>();
        *uep = u_entry_T {
            ue_next: ptr::null_mut(),
            ue_top: undo_read_4c(bi),
            ue_bot: undo_read_4c(bi),
            ue_lcount: undo_read_4c(bi),
            ue_array: ptr::null_mut(),
            ue_size: 0,
        };
        if (*uep).ue_top < 0 || (*uep).ue_bot < 0 || (*uep).ue_lcount < 0 {
            corruption_error(c"entry lnum".as_ptr(), file_name);
            *error = true;
            return uep;
        }

        (*uep).ue_size = undo_read_len(bi, c"entry size".as_ptr(), file_name);
        if (*uep).ue_size < 0 {
            (*uep).ue_size = 0;
            *error = true;
            return uep;
        }

        let array = if (*uep).ue_size > 0 {
            xcalloc((*uep).ue_size as usize, size_of::<*mut c_char>()).cast::<*mut c_char>()
        } else {
            ptr::null_mut()
        };
        (*uep).ue_array = array;

        for i in 0..(*uep).ue_size as usize {
            let line_len = undo_read_len(bi, c"line length".as_ptr(), file_name);
            let line = if line_len < 0 {
                ptr::null_mut()
            } else {
                undo_read_string(bi, line_len as usize)
            };
            if line.is_null() {
                *error = true;
                return uep;
            }
            *array.add(i) = line;
        }
        uep
    }
}

unsafe fn serialize_pos(bi: &mut bufinfo_T, pos: pos_T) {
    unsafe {
        undo_write_bytes(bi, pos.lnum as u64, 4);
        undo_write_bytes(bi, pos.col as u64, 4);
        undo_write_bytes(bi, pos.coladd as u64, 4);
    }
}

unsafe fn unserialize_pos(bi: &mut bufinfo_T, pos: *mut pos_T) {
    unsafe {
        (*pos).lnum = undo_read_4c(bi).max(0);
        (*pos).col = undo_read_4c(bi).max(0);
        (*pos).coladd = undo_read_4c(bi).max(0);
    }
}

unsafe fn serialize_visualinfo(bi: &mut bufinfo_T, info: *mut visualinfo_T) {
    unsafe {
        serialize_pos(bi, (*info).vi_start);
        serialize_pos(bi, (*info).vi_end);
        undo_write_bytes(bi, (*info).vi_mode as u64, 4);
        undo_write_bytes(bi, (*info).vi_curswant as u64, 4);
    }
}

unsafe fn unserialize_visualinfo(bi: &mut bufinfo_T, info: *mut visualinfo_T) {
    unsafe {
        unserialize_pos(bi, &raw mut (*info).vi_start);
        unserialize_pos(bi, &raw mut (*info).vi_end);
        (*info).vi_mode = undo_read_4c(bi);
        (*info).vi_curswant = undo_read_4c(bi);
    }
}

/// Write the undo tree in an undo file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_write_undo(
    name: *const c_char,
    forceit: bool,
    buf: *mut buf_T,
    hash: *mut u8,
) {
    unsafe {
        let file_name = if name.is_null() {
            let f = u_get_undo_file_name((*buf).b_ffname, false);
            if f.is_null() {
                if p_verbose > 0 {
                    verbose_enter();
                    smsg(
                        0,
                        c"%s".as_ptr(),
                        gettext(c"Cannot write undo file in any directory in 'undodir'".as_ptr()),
                    );
                    verbose_leave();
                }
                return;
            }
            f
        } else {
            name.cast_mut()
        };

        let mut perm: c_int = 0o600;
        if !(*buf).b_ffname.is_null() {
            perm = os_getperm((*buf).b_ffname);
            if perm < 0 {
                perm = 0o600;
            }
        }
        perm &= 0o666;

        'theend: {
            if os_path_exists(file_name) {
                if name.is_null() || !forceit {
                    let fd = os_open(file_name, O_RDONLY, 0);
                    if fd < 0 {
                        if !name.is_null() || p_verbose > 0 {
                            if name.is_null() {
                                verbose_enter();
                            }
                            smsg(
                                0,
                                gettext(
                                    c"Will not overwrite with undo file, cannot read: %s".as_ptr(),
                                ),
                                file_name,
                            );
                            if name.is_null() {
                                verbose_leave();
                            }
                        }
                        break 'theend;
                    } else {
                        let mut mbuf = [0u8; UF_START_MAGIC_LEN];
                        let len = read_eintr(fd, mbuf.as_mut_ptr().cast(), UF_START_MAGIC_LEN);
                        os_close(fd);
                        if len < UF_START_MAGIC_LEN as isize
                            || memcmp(
                                mbuf.as_ptr().cast(),
                                UF_START_MAGIC.as_ptr().cast(),
                                UF_START_MAGIC_LEN,
                            ) != 0
                        {
                            if !name.is_null() || p_verbose > 0 {
                                if name.is_null() {
                                    verbose_enter();
                                }
                                smsg(
                                    0,
                                    gettext(
                                        c"Will not overwrite, this is not an undo file: %s"
                                            .as_ptr(),
                                    ),
                                    file_name,
                                );
                                if name.is_null() {
                                    verbose_leave();
                                }
                            }
                            break 'theend;
                        }
                    }
                }
                os_remove(file_name);
            }

            if (*buf).b_u_numhead == 0 && (*buf).b_u_line_ptr.is_null() {
                if p_verbose > 0 {
                    verb_msg(gettext(
                        c"Skipping undo file write, nothing to undo".as_ptr(),
                    ));
                }
                break 'theend;
            }

            let fd = os_open(file_name, O_CREAT | O_WRONLY | O_EXCL | O_NOFOLLOW, perm);
            if fd < 0 {
                semsg(gettext(E_NOT_OPEN.as_ptr()), file_name);
                break 'theend;
            }
            os_setperm(file_name, perm);
            if p_verbose > 0 {
                verbose_enter();
                smsg(0, gettext(c"Writing undo file: %s".as_ptr()), file_name);
                verbose_leave();
            }

            #[cfg(unix)]
            {
                let mut file_info_old = FileInfo::default();
                let mut file_info_new = FileInfo::default();
                if !(*buf).b_ffname.is_null()
                    && os_fileinfo((*buf).b_ffname, &raw mut file_info_old)
                    && os_fileinfo(file_name, &raw mut file_info_new)
                    && file_info_old.stat.st_gid != file_info_new.stat.st_gid
                    && os_fchown(
                        fd,
                        u32::MAX as uv_uid_t,
                        file_info_old.stat.st_gid as uv_gid_t,
                    ) != 0
                {
                    os_setperm(file_name, (perm & 0o707) | ((perm & 0o7) << 3));
                }
            }

            let fp = c_fdopen(fd, c"w".as_ptr());
            if fp.is_null() {
                semsg(gettext(E_NOT_OPEN.as_ptr()), file_name);
                os_close(fd);
                os_remove(file_name);
                break 'theend;
            }

            u_sync(true);

            let mut write_ok = false;
            let mut bi = bufinfo_T {
                bi_buf: buf,
                bi_fp: fp,
                bi_fsize: 0,
            };

            'write_error: {
                if !serialize_header(&mut bi, hash) {
                    break 'write_error;
                }

                lastmark += 1;
                let mark = lastmark;
                let mut uhp = (*buf).b_u_oldhead.cast::<u_header_T>();
                while !uhp.is_null() {
                    if (*uhp).uh_walk != mark {
                        (*uhp).uh_walk = mark;
                        if !serialize_uhp(&mut bi, uhp) {
                            break 'write_error;
                        }
                    }

                    if !(*uhp).uh_prev.ptr.is_null() && (*(*uhp).uh_prev.ptr).uh_walk != mark {
                        uhp = (*uhp).uh_prev.ptr;
                    } else if !(*uhp).uh_alt_next.ptr.is_null()
                        && (*(*uhp).uh_alt_next.ptr).uh_walk != mark
                    {
                        uhp = (*uhp).uh_alt_next.ptr;
                    } else if !(*uhp).uh_next.ptr.is_null()
                        && (*uhp).uh_alt_prev.ptr.is_null()
                        && (*(*uhp).uh_next.ptr).uh_walk != mark
                    {
                        uhp = (*uhp).uh_next.ptr;
                    } else if !(*uhp).uh_alt_prev.ptr.is_null() {
                        uhp = (*uhp).uh_alt_prev.ptr;
                    } else {
                        uhp = (*uhp).uh_next.ptr;
                    }
                }

                if undo_write_bytes(&mut bi, UF_HEADER_END_MAGIC as u64, 2) {
                    write_ok = true;
                }

                let do_fsync = if (*buf).b_p_fs >= 0 {
                    (*buf).b_p_fs != 0
                } else {
                    p_fs != 0
                };
                if do_fsync && fflush(fp) == 0 && os_fsync(fd) != 0 {
                    write_ok = false;
                }
            }

            fclose(fp);
            if !write_ok {
                semsg(gettext(E_WRITE_ERROR_IN_UNDO_FILE_STR.as_ptr()), file_name);
            }

            if !(*buf).b_ffname.is_null() {
                let acl: vim_acl_T = os_get_acl((*buf).b_ffname);
                os_set_acl(file_name, acl);
                os_free_acl(acl);
            }
        }

        if file_name.cast_const() != name {
            xfree(file_name.cast());
        }
    }
}

unsafe extern "C" fn uhp_seq_cmp(v1: *const c_void, v2: *const c_void) -> c_int {
    unsafe {
        let u1 = *v1.cast::<*const u_header_T>();
        let u2 = *v2.cast::<*const u_header_T>();
        if (*u1).uh_seq == (*u2).uh_seq {
            0
        } else if (*u1).uh_seq > (*u2).uh_seq {
            1
        } else {
            -1
        }
    }
}

unsafe fn uhp_table_find(uhp_table: *mut *mut u_header_T, num_head: c_int, seq: c_int) -> c_int {
    unsafe {
        let mut lo: c_int = 0;
        let mut hi: c_int = num_head - 1;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let mid_seq = (**uhp_table.add(mid as usize)).uh_seq;
            if mid_seq < seq {
                lo = mid + 1;
            } else if mid_seq > seq {
                hi = mid - 1;
            } else {
                return mid;
            }
        }
        -1
    }
}

/// Loads the undo tree from an undo file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_read_undo(
    name: *mut c_char,
    hash: *const u8,
    #[allow(unused_variables)] orig_name: *const c_char,
) {
    unsafe {
        let mut uhp_table: *mut *mut u_header_T = ptr::null_mut();
        let mut line_ptr: *mut c_char = ptr::null_mut();
        let mut num_read_uhps: c_int = 0;

        let file_name = if name.is_null() {
            let f = u_get_undo_file_name((*curbuf).b_ffname, true);
            if f.is_null() {
                return;
            }

            #[cfg(unix)]
            {
                let mut file_info_orig = FileInfo::default();
                let mut file_info_undo = FileInfo::default();
                if os_fileinfo(orig_name, &raw mut file_info_orig)
                    && os_fileinfo(f, &raw mut file_info_undo)
                    && file_info_orig.stat.st_uid != file_info_undo.stat.st_uid
                    && file_info_undo.stat.st_uid != u64::from(getuid())
                {
                    if p_verbose > 0 {
                        verbose_enter();
                        smsg(
                            0,
                            gettext(c"Not reading undo file, owner differs: %s".as_ptr()),
                            f,
                        );
                        verbose_leave();
                    }
                    xfree(f.cast());
                    return;
                }
            }
            f
        } else {
            name
        };

        if p_verbose > 0 {
            verbose_enter();
            smsg(0, gettext(c"Reading undo file: %s".as_ptr()), file_name);
            verbose_leave();
        }

        let fp = os_fopen(file_name, c"r".as_ptr());
        'theend: {
            'error: {
                if fp.is_null() {
                    if !name.is_null() || p_verbose > 0 {
                        semsg(
                            gettext(c"E822: Cannot open undo file for reading: %s".as_ptr()),
                            file_name,
                        );
                    }
                    break 'error;
                }

                let mut file_info = FileInfo::default();
                let fsize = if os_fileinfo_fd(c_fileno(fp), &raw mut file_info) {
                    os_fileinfo_size(&raw const file_info) as off_T
                } else {
                    0
                };
                let mut bi = bufinfo_T {
                    bi_buf: curbuf,
                    bi_fp: fp,
                    bi_fsize: fsize,
                };

                let mut magic_buf = [0u8; UF_START_MAGIC_LEN];
                if fread(magic_buf.as_mut_ptr().cast(), UF_START_MAGIC_LEN, 1, fp) != 1
                    || memcmp(
                        magic_buf.as_ptr().cast(),
                        UF_START_MAGIC.as_ptr().cast(),
                        UF_START_MAGIC_LEN,
                    ) != 0
                {
                    semsg(gettext(c"E823: Not an undo file: %s".as_ptr()), file_name);
                    break 'error;
                }
                let version = get2c(fp);
                if version != UF_VERSION {
                    semsg(
                        gettext(c"E824: Incompatible undo file: %s".as_ptr()),
                        file_name,
                    );
                    break 'error;
                }

                let mut read_hash = [0u8; UNDO_HASH_SIZE];
                if !undo_read(&mut bi, read_hash.as_mut_ptr(), UNDO_HASH_SIZE) {
                    corruption_error(c"hash".as_ptr(), file_name);
                    break 'error;
                }
                let line_count = undo_read_4c(&mut bi) as linenr_T;
                if memcmp(hash.cast(), read_hash.as_ptr().cast(), UNDO_HASH_SIZE) != 0
                    || line_count != (*curbuf).b_ml.ml_line_count
                {
                    if p_verbose > 0 || !name.is_null() {
                        if name.is_null() {
                            verbose_enter();
                        }
                        give_warning(
                            gettext(c"File contents changed, cannot use undo info".as_ptr()),
                            true,
                            true,
                        );
                        if name.is_null() {
                            verbose_leave();
                        }
                    }
                    break 'error;
                }

                let str_len = undo_read_len(&mut bi, c"line length".as_ptr(), file_name);
                if str_len < 0 {
                    break 'error;
                }

                if str_len > 0 {
                    line_ptr = undo_read_string(&mut bi, str_len as usize);
                    if line_ptr.is_null() {
                        corruption_error(c"truncated".as_ptr(), file_name);
                        break 'error;
                    }
                }
                let line_lnum = undo_read_4c(&mut bi) as linenr_T;
                let line_colnr = undo_read_4c(&mut bi) as colnr_T;
                if line_lnum < 0 || line_colnr < 0 {
                    corruption_error(c"line lnum/col".as_ptr(), file_name);
                    break 'error;
                }

                let old_header_seq = undo_read_4c(&mut bi);
                let new_header_seq = undo_read_4c(&mut bi);
                let cur_header_seq = undo_read_4c(&mut bi);
                let num_head = undo_read_len(&mut bi, c"num_head".as_ptr(), file_name);
                if num_head < 0 {
                    break 'error;
                }
                let seq_last = undo_read_4c(&mut bi);
                let seq_cur = undo_read_4c(&mut bi);
                let seq_time = undo_read_time(&mut bi);

                let mut last_save_nr: c_int = 0;
                loop {
                    let mut len = undo_read_byte(&mut bi);
                    if len == 0 || len == EOF {
                        break;
                    }
                    let what = undo_read_byte(&mut bi);
                    match what {
                        UF_LAST_SAVE_NR => {
                            last_save_nr = undo_read_4c(&mut bi);
                        }
                        _ => {
                            while {
                                len -= 1;
                                len >= 0
                            } {
                                undo_read_byte(&mut bi);
                            }
                        }
                    }
                }

                if num_head > 0 {
                    uhp_table = xcalloc(num_head as usize, size_of::<*mut u_header_T>()).cast();
                }

                let mut c = undo_read_2c(&mut bi);
                while c == UF_HEADER_MAGIC {
                    if num_read_uhps >= num_head {
                        corruption_error(c"num_head too small".as_ptr(), file_name);
                        break 'error;
                    }

                    let uhp = unserialize_uhp(&mut bi, file_name);
                    if uhp.is_null() {
                        break 'error;
                    }
                    *uhp_table.add(num_read_uhps as usize) = uhp;
                    num_read_uhps += 1;
                    c = undo_read_2c(&mut bi);
                }

                if num_read_uhps != num_head {
                    corruption_error(c"num_head".as_ptr(), file_name);
                    break 'error;
                }
                if c != UF_HEADER_END_MAGIC {
                    corruption_error(c"end marker".as_ptr(), file_name);
                    break 'error;
                }

                if num_head > 0 {
                    qsort(
                        uhp_table.cast(),
                        num_head as usize,
                        size_of::<*mut u_header_T>(),
                        uhp_seq_cmp,
                    );
                }

                for i in 0..(num_head - 1) {
                    if (**uhp_table.add(i as usize)).uh_seq
                        == (**uhp_table.add((i + 1) as usize)).uh_seq
                    {
                        corruption_error(c"duplicate uh_seq".as_ptr(), file_name);
                        break 'error;
                    }
                }

                let mut old_idx: c_int = -1;
                let mut new_idx: c_int = -1;
                let mut cur_idx: c_int = -1;
                for i in 0..num_head {
                    let uhp = *uhp_table.add(i as usize);

                    let fidx_next = uhp_table_find(uhp_table, num_head, (*uhp).uh_next.seq);
                    (*uhp).uh_next.ptr = if fidx_next >= 0 && fidx_next != i {
                        *uhp_table.add(fidx_next as usize)
                    } else {
                        ptr::null_mut()
                    };

                    let fidx_prev = uhp_table_find(uhp_table, num_head, (*uhp).uh_prev.seq);
                    (*uhp).uh_prev.ptr = if fidx_prev >= 0 && fidx_prev != i {
                        *uhp_table.add(fidx_prev as usize)
                    } else {
                        ptr::null_mut()
                    };

                    let fidx_anext = uhp_table_find(uhp_table, num_head, (*uhp).uh_alt_next.seq);
                    (*uhp).uh_alt_next.ptr = if fidx_anext >= 0 && fidx_anext != i {
                        *uhp_table.add(fidx_anext as usize)
                    } else {
                        ptr::null_mut()
                    };

                    let fidx_aprev = uhp_table_find(uhp_table, num_head, (*uhp).uh_alt_prev.seq);
                    (*uhp).uh_alt_prev.ptr = if fidx_aprev >= 0 && fidx_aprev != i {
                        *uhp_table.add(fidx_aprev as usize)
                    } else {
                        ptr::null_mut()
                    };

                    if old_header_seq > 0 && old_idx < 0 && (*uhp).uh_seq == old_header_seq {
                        old_idx = i;
                    }
                    if new_header_seq > 0 && new_idx < 0 && (*uhp).uh_seq == new_header_seq {
                        new_idx = i;
                    }
                    if cur_header_seq > 0 && cur_idx < 0 && (*uhp).uh_seq == cur_header_seq {
                        cur_idx = i;
                    }
                }

                u_blockfree(curbuf);
                (*curbuf).b_u_oldhead = if old_idx < 0 {
                    ptr::null_mut()
                } else {
                    (*uhp_table.add(old_idx as usize)).cast()
                };
                (*curbuf).b_u_newhead = if new_idx < 0 {
                    ptr::null_mut()
                } else {
                    (*uhp_table.add(new_idx as usize)).cast()
                };
                (*curbuf).b_u_curhead = if cur_idx < 0 {
                    ptr::null_mut()
                } else {
                    (*uhp_table.add(cur_idx as usize)).cast()
                };
                (*curbuf).b_u_line_ptr = line_ptr;
                (*curbuf).b_u_line_lnum = line_lnum;
                (*curbuf).b_u_line_colnr = line_colnr;
                (*curbuf).b_u_numhead = num_head;
                (*curbuf).b_u_seq_last = seq_last;
                (*curbuf).b_u_seq_cur = seq_cur;
                (*curbuf).b_u_time_cur = seq_time;
                (*curbuf).b_u_save_nr_last = last_save_nr;
                (*curbuf).b_u_save_nr_cur = last_save_nr;

                (*curbuf).b_u_synced = true;
                xfree(uhp_table.cast());

                if !name.is_null() {
                    smsg(
                        0,
                        gettext(c"Finished reading undo file %s".as_ptr()),
                        file_name,
                    );
                }
                break 'theend;
            }

            xfree(line_ptr.cast());
            if !uhp_table.is_null() {
                for i in 0..num_read_uhps as usize {
                    let p = *uhp_table.add(i);
                    if !p.is_null() {
                        u_free_uhp(p);
                    }
                }
                xfree(uhp_table.cast());
            }
        }

        if !fp.is_null() {
            fclose(fp);
        }
        if file_name != name {
            xfree(file_name.cast());
        }
    }
}

unsafe fn undo_write(bi: &mut bufinfo_T, ptr: *mut u8, len: usize) -> bool {
    unsafe { fwrite(ptr.cast(), len, 1, bi.bi_fp) == 1 }
}

unsafe fn undo_write_bytes(bi: &mut bufinfo_T, nr: u64, len: usize) -> bool {
    unsafe {
        assert!(len > 0);
        let mut buf = [0u8; 8];
        let mut i = len - 1;
        for bufi in 0..len {
            buf[bufi] = (nr >> (i * 8)) as u8;
            i = i.wrapping_sub(1);
        }
        undo_write(bi, buf.as_mut_ptr(), len)
    }
}

unsafe fn put_header_ptr(bi: &mut bufinfo_T, uhp: *mut u_header_T) {
    unsafe {
        assert!(uhp.is_null() || (*uhp).uh_seq >= 0);
        undo_write_bytes(
            bi,
            if !uhp.is_null() {
                (*uhp).uh_seq as u64
            } else {
                0
            },
            4,
        );
    }
}

unsafe fn undo_read_4c(bi: &mut bufinfo_T) -> c_int {
    unsafe { get4c(bi.bi_fp) }
}

unsafe fn undo_read_len(
    bi: &mut bufinfo_T,
    what: *const c_char,
    file_name: *const c_char,
) -> c_int {
    unsafe {
        let len = undo_read_4c(bi);
        let pos = vim_ftell(bi.bi_fp);
        if len < 0 || (bi.bi_fsize > 0 && (pos < 0 || (len as off_T) > bi.bi_fsize - pos)) {
            corruption_error(
                if feof(bi.bi_fp) != 0 {
                    c"truncated".as_ptr()
                } else {
                    what
                },
                file_name,
            );
            return -1;
        }
        len
    }
}

unsafe fn undo_read_2c(bi: &mut bufinfo_T) -> c_int {
    unsafe { get2c(bi.bi_fp) }
}

unsafe fn undo_read_byte(bi: &mut bufinfo_T) -> c_int {
    unsafe { fgetc(bi.bi_fp) }
}

unsafe fn undo_read_time(bi: &mut bufinfo_T) -> time_t {
    unsafe { get8ctime(bi.bi_fp) }
}

unsafe fn undo_read(bi: &mut bufinfo_T, buffer: *mut u8, size: usize) -> bool {
    unsafe {
        let retval = fread(buffer.cast(), size, 1, bi.bi_fp) == 1;
        if !retval {
            ptr::write_bytes(buffer, 0, size);
        }
        retval
    }
}

unsafe fn undo_read_string(bi: &mut bufinfo_T, len: usize) -> *mut c_char {
    unsafe {
        let ptr = xmallocz(len).cast::<c_char>();
        if len > 0 && !undo_read(bi, ptr.cast::<u8>(), len) {
            xfree(ptr.cast());
            return ptr::null_mut();
        }
        ptr
    }
}

/// Undo `count` times.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_undo(mut count: c_int) {
    unsafe {
        if !(*curbuf).b_u_synced {
            u_sync(true);
            count = 1;
        }

        if vim_strchr(p_cpo, kCpoUndo).is_null() {
            undo_undoes = true;
        } else {
            undo_undoes = !undo_undoes;
        }
        u_doit(count, false, true);
    }
}

/// Redo `count` times.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_redo(count: c_int) {
    unsafe {
        if vim_strchr(p_cpo, kCpoUndo).is_null() {
            undo_undoes = false;
        }

        u_doit(count, false, true);
    }
}

/// Undo and remove the branch from the undo tree.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_undo_and_forget(mut count: c_int, do_buf_event: bool) -> bool {
    unsafe {
        if !(*curbuf).b_u_synced {
            u_sync(true);
            count = 1;
        }
        undo_undoes = true;
        u_doit(count, true, do_buf_event);

        if (*curbuf).b_u_curhead.is_null() {
            return false;
        }

        u_forget_header(curbuf, (*curbuf).b_u_curhead.cast::<u_header_T>());
        true
    }
}

unsafe fn u_doit(startcount: c_int, quiet: bool, do_buf_event: bool) {
    unsafe {
        if !undo_allowed(curbuf) {
            return;
        }
        atom_did_global_op();

        u_newcount = 0;
        u_oldcount = 0;
        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            u_oldcount = -1;
        }

        msg_ext_set_kind(c"undo".as_ptr());
        let mut count = startcount;
        while {
            let prev = count;
            count -= 1;
            prev != 0
        } {
            change_warning(curbuf, 0);

            if undo_undoes {
                if (*curbuf).b_u_curhead.is_null() {
                    (*curbuf).b_u_curhead = (*curbuf).b_u_newhead;
                } else if get_undolevel(curbuf) > 0 {
                    (*curbuf).b_u_curhead = (*(*curbuf).b_u_curhead.cast::<u_header_T>())
                        .uh_next
                        .ptr
                        .cast();
                }
                if (*curbuf).b_u_numhead == 0 || (*curbuf).b_u_curhead.is_null() {
                    (*curbuf).b_u_curhead = (*curbuf).b_u_oldhead;
                    beep_flush();
                    if count == startcount - 1 {
                        if !shortmess(kShmUndo) {
                            msg(gettext(c"Already at oldest change".as_ptr()), 0);
                        }
                        return;
                    }
                    break;
                }

                u_undoredo(true, do_buf_event);
            } else {
                if (*curbuf).b_u_curhead.is_null() || get_undolevel(curbuf) <= 0 {
                    beep_flush();
                    if count == startcount - 1 {
                        if !shortmess(kShmUndo) {
                            msg(gettext(c"Already at newest change".as_ptr()), 0);
                        }
                        return;
                    }
                    break;
                }

                u_undoredo(false, do_buf_event);

                let curhead = (*curbuf).b_u_curhead.cast::<u_header_T>();
                if (*curhead).uh_prev.ptr.is_null() {
                    (*curbuf).b_u_newhead = curhead.cast();
                }
                (*curbuf).b_u_curhead = (*curhead).uh_prev.ptr.cast();
            }
        }
        u_undo_end(undo_undoes, false, quiet);
    }
}

/// Undo or redo over the timeline.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn undo_time(step: c_int, sec: bool, file: bool, absolute: bool) {
    unsafe {
        if text_locked() {
            text_locked_msg();
            return;
        }
        mc_undo_time();

        if !(*curbuf).b_u_synced {
            u_sync(true);
        }

        u_newcount = 0;
        u_oldcount = 0;
        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            u_oldcount = -1;
        }

        let mut target: c_int;
        let mut closest: c_int;
        let mut uhp: *mut u_header_T = ptr::null_mut();
        let mut dosec = sec;
        let mut dofile = file;
        let mut above = false;
        let mut did_undo = true;

        if absolute {
            target = step;
            closest = -1;
        } else {
            if dosec {
                target = ((*curbuf).b_u_time_cur as c_int) + step;
            } else if dofile {
                if step < 0 {
                    uhp = (*curbuf).b_u_curhead.cast::<u_header_T>();
                    if !uhp.is_null() {
                        uhp = (*uhp).uh_next.ptr;
                    } else {
                        uhp = (*curbuf).b_u_newhead.cast::<u_header_T>();
                    }
                    if !uhp.is_null() && (*uhp).uh_save_nr != 0 {
                        target = (*curbuf).b_u_save_nr_cur + step;
                    } else {
                        target = (*curbuf).b_u_save_nr_cur + step + 1;
                    }
                    if target <= 0 {
                        dofile = false;
                    }
                } else {
                    target = (*curbuf).b_u_save_nr_cur + step;
                    if target > (*curbuf).b_u_save_nr_last {
                        target = (*curbuf).b_u_seq_last + 1;
                        dofile = false;
                    }
                }
            } else {
                target = (*curbuf).b_u_seq_cur + step;
            }
            if step < 0 {
                target = target.max(0);
                closest = -1;
            } else {
                if dosec {
                    closest = (os_time() + 1) as c_int;
                } else if dofile {
                    closest = (*curbuf).b_u_save_nr_last + 2;
                } else {
                    closest = (*curbuf).b_u_seq_last + 2;
                }
                if target >= closest {
                    target = closest - 1;
                }
            }
        }
        let closest_start = closest;
        let mut closest_seq = (*curbuf).b_u_seq_cur;
        let mut mark = lastmark;
        let mut nomark: c_int = 0;

        if target != 0 {
            for round in 1..=2 {
                lastmark += 1;
                mark = lastmark;
                lastmark += 1;
                nomark = lastmark;

                uhp = if (*curbuf).b_u_curhead.is_null() {
                    (*curbuf).b_u_newhead.cast::<u_header_T>()
                } else {
                    (*curbuf).b_u_curhead.cast::<u_header_T>()
                };

                while !uhp.is_null() {
                    (*uhp).uh_walk = mark;
                    let val = if dosec {
                        (*uhp).uh_time as c_int
                    } else if dofile {
                        (*uhp).uh_save_nr
                    } else {
                        (*uhp).uh_seq
                    };

                    if round == 1 && !(dofile && val == 0) {
                        let dir_ok = if step < 0 {
                            (*uhp).uh_seq <= (*curbuf).b_u_seq_cur
                        } else {
                            (*uhp).uh_seq > (*curbuf).b_u_seq_cur
                        };
                        let closer = if dosec && val == closest {
                            if step < 0 {
                                (*uhp).uh_seq < closest_seq
                            } else {
                                (*uhp).uh_seq > closest_seq
                            }
                        } else {
                            closest == closest_start
                                || if val > target {
                                    if closest > target {
                                        val - target <= closest - target
                                    } else {
                                        val - target <= target - closest
                                    }
                                } else if closest > target {
                                    target - val <= closest - target
                                } else {
                                    target - val <= target - closest
                                }
                        };
                        if dir_ok && closer {
                            closest = val;
                            closest_seq = (*uhp).uh_seq;
                        }
                    }

                    if target == val && !dosec {
                        target = (*uhp).uh_seq;
                        break;
                    }

                    if !(*uhp).uh_prev.ptr.is_null()
                        && (*(*uhp).uh_prev.ptr).uh_walk != nomark
                        && (*(*uhp).uh_prev.ptr).uh_walk != mark
                    {
                        uhp = (*uhp).uh_prev.ptr;
                    } else if !(*uhp).uh_alt_next.ptr.is_null()
                        && (*(*uhp).uh_alt_next.ptr).uh_walk != nomark
                        && (*(*uhp).uh_alt_next.ptr).uh_walk != mark
                    {
                        uhp = (*uhp).uh_alt_next.ptr;
                    } else if !(*uhp).uh_next.ptr.is_null()
                        && (*uhp).uh_alt_prev.ptr.is_null()
                        && (*(*uhp).uh_next.ptr).uh_walk != nomark
                        && (*(*uhp).uh_next.ptr).uh_walk != mark
                    {
                        if uhp == (*curbuf).b_u_curhead.cast::<u_header_T>() {
                            (*uhp).uh_walk = nomark;
                        }
                        uhp = (*uhp).uh_next.ptr;
                    } else {
                        (*uhp).uh_walk = nomark;
                        if !(*uhp).uh_alt_prev.ptr.is_null() {
                            uhp = (*uhp).uh_alt_prev.ptr;
                        } else {
                            uhp = (*uhp).uh_next.ptr;
                        }
                    }
                }

                if !uhp.is_null() {
                    break;
                }

                if absolute {
                    semsg(
                        gettext(c"E830: Undo number %lld not found".as_ptr()),
                        step as i64,
                    );
                    return;
                }

                if closest == closest_start {
                    if !shortmess(kShmUndo) {
                        if step < 0 {
                            msg(gettext(c"Already at oldest change".as_ptr()), 0);
                        } else {
                            msg(gettext(c"Already at newest change".as_ptr()), 0);
                        }
                    }
                    return;
                }

                target = closest_seq;
                dosec = false;
                dofile = false;
                if step < 0 {
                    above = true;
                }
            }
        }

        if !uhp.is_null() || target == 0 {
            while !got_int {
                change_warning(curbuf, 0);

                uhp = (*curbuf).b_u_curhead.cast::<u_header_T>();
                if uhp.is_null() {
                    uhp = (*curbuf).b_u_newhead.cast::<u_header_T>();
                } else {
                    uhp = (*uhp).uh_next.ptr;
                }
                if uhp.is_null()
                    || (target > 0 && (*uhp).uh_walk != mark)
                    || ((*uhp).uh_seq == target && !above)
                {
                    break;
                }
                (*curbuf).b_u_curhead = uhp.cast();
                u_undoredo(true, true);
                if target > 0 {
                    (*uhp).uh_walk = nomark;
                }
            }

            if target > 0 {
                while !got_int {
                    change_warning(curbuf, 0);

                    uhp = (*curbuf).b_u_curhead.cast::<u_header_T>();
                    if uhp.is_null() {
                        break;
                    }

                    while !(*uhp).uh_alt_prev.ptr.is_null()
                        && (*(*uhp).uh_alt_prev.ptr).uh_walk == mark
                    {
                        uhp = (*uhp).uh_alt_prev.ptr;
                    }

                    let mut last = uhp;
                    while !(*last).uh_alt_next.ptr.is_null()
                        && (*(*last).uh_alt_next.ptr).uh_walk == mark
                    {
                        last = (*last).uh_alt_next.ptr;
                    }
                    if last != uhp {
                        while !(*uhp).uh_alt_prev.ptr.is_null() {
                            uhp = (*uhp).uh_alt_prev.ptr;
                        }
                        if !(*last).uh_alt_next.ptr.is_null() {
                            (*(*last).uh_alt_next.ptr).uh_alt_prev.ptr = (*last).uh_alt_prev.ptr;
                        }
                        (*(*last).uh_alt_prev.ptr).uh_alt_next.ptr = (*last).uh_alt_next.ptr;
                        (*last).uh_alt_prev.ptr = ptr::null_mut();
                        (*last).uh_alt_next.ptr = uhp;
                        (*uhp).uh_alt_prev.ptr = last;

                        if (*curbuf).b_u_oldhead.cast::<u_header_T>() == uhp {
                            (*curbuf).b_u_oldhead = last.cast();
                        }
                        uhp = last;
                        if !(*uhp).uh_next.ptr.is_null() {
                            (*(*uhp).uh_next.ptr).uh_prev.ptr = uhp;
                        }
                    }
                    (*curbuf).b_u_curhead = uhp.cast();

                    if (*uhp).uh_walk != mark {
                        break;
                    }

                    if (*uhp).uh_seq == target && above {
                        (*curbuf).b_u_seq_cur = target - 1;
                        break;
                    }

                    u_undoredo(false, true);

                    if (*uhp).uh_prev.ptr.is_null() {
                        (*curbuf).b_u_newhead = uhp.cast();
                    }
                    (*curbuf).b_u_curhead = (*uhp).uh_prev.ptr.cast();
                    did_undo = false;

                    if (*uhp).uh_seq == target {
                        break;
                    }

                    uhp = (*uhp).uh_prev.ptr;
                    if uhp.is_null() || (*uhp).uh_walk != mark {
                        internal_error(c"undo_time()".as_ptr());
                        break;
                    }
                }
            }
        }
        u_undo_end(did_undo, absolute, false);
    }
}

unsafe fn u_undoredo(undo: bool, do_buf_event: bool) {
    unsafe {
        let mut newarray: *mut *mut c_char;
        let mut newlnum: linenr_T = MAXLNUM;
        let mut new_curpos = (*curwin).w_cursor;
        let mut nuep: *mut u_entry_T;
        let mut newlist: *mut u_entry_T = ptr::null_mut();
        let curhead = (*curbuf).b_u_curhead.cast::<u_header_T>();

        block_autocmds();

        let old_flags = (*curhead).uh_flags;
        let new_flags = (if (*curbuf).b_changed != 0 {
            UH_CHANGED
        } else {
            0
        }) | (if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            UH_EMPTYBUF
        } else {
            0
        }) | (old_flags & UH_RELOAD);
        setpcmark();

        zero_fmark_additional_data((&raw mut (*curbuf).b_namedm).cast::<fmark_T>());
        let namedm = (*curbuf).b_namedm;
        let visualinfo = (*curbuf).b_visual;
        (*curbuf).b_op_start.lnum = (*curbuf).b_ml.ml_line_count;
        (*curbuf).b_op_start.col = 0;
        (*curbuf).b_op_end.lnum = 0;
        (*curbuf).b_op_end.col = 0;

        let mut uep = (*curhead).uh_entry;
        while !uep.is_null() {
            let top = (*uep).ue_top;
            let mut bot = (*uep).ue_bot;
            if bot == 0 {
                bot = (*curbuf).b_ml.ml_line_count + 1;
            }
            if top > (*curbuf).b_ml.ml_line_count
                || top >= bot
                || bot > (*curbuf).b_ml.ml_line_count + 1
            {
                unblock_autocmds();
                iemsg(gettext(c"E438: u_undo: line numbers wrong".as_ptr()));
                changed(curbuf);
                return;
            }

            let oldsize = bot - top - 1;
            let newsize = (*uep).ue_size;

            {
                let lnum = (*curhead).uh_cursor.lnum;
                if lnum >= top && lnum <= top + newsize + 1 {
                    new_curpos = (*curhead).uh_cursor;
                    newlnum = -1;
                } else if top < newlnum {
                    let mut i: linenr_T = 0;
                    while i < newsize && i < oldsize {
                        if strcmp(*(*uep).ue_array.add(i as usize), ml_get(top + 1 + i)) != 0 {
                            break;
                        }
                        i += 1;
                    }
                    if i == newsize && newlnum == MAXLNUM && (*uep).ue_next.is_null() {
                        newlnum = top;
                        new_curpos.lnum = newlnum + 1;
                    } else if i < newsize {
                        newlnum = top + i;
                        new_curpos.lnum = newlnum + 1;
                    }
                }
            }

            let mut empty_buffer = false;

            if oldsize > 0 {
                newarray =
                    xmalloc(size_of::<*mut c_char>() * (oldsize as usize)).cast::<*mut c_char>();
                let mut lnum = bot - 1;
                let mut i = oldsize;
                while {
                    i -= 1;
                    i >= 0
                } {
                    *newarray.add(i as usize) = u_save_line(lnum);
                    if (*curbuf).b_ml.ml_line_count == 1 {
                        empty_buffer = true;
                    }
                    ml_delete(lnum);
                    lnum -= 1;
                }
            } else {
                newarray = ptr::null_mut();
            }

            check_cursor_lnum(curwin);

            if newsize != 0 {
                for i in 0..newsize as usize {
                    let lnum = top + i as linenr_T;
                    let line = *(*uep).ue_array.add(i);
                    if empty_buffer && lnum == 0 {
                        ml_replace(1, line, true);
                    } else {
                        ml_append_flags(lnum, line, 0, 0);
                    }
                    xfree(line.cast());
                }
                xfree((*uep).ue_array.cast());
            }

            if oldsize != newsize {
                mark_adjust(
                    top + 1,
                    top + oldsize,
                    MAXLNUM,
                    newsize - oldsize,
                    kExtmarkNOOP,
                );
                if (*curbuf).b_op_start.lnum > top + oldsize {
                    (*curbuf).b_op_start.lnum += newsize - oldsize;
                }
                if (*curbuf).b_op_end.lnum > top + oldsize {
                    (*curbuf).b_op_end.lnum += newsize - oldsize;
                }
            }

            if oldsize > 0 || newsize > 0 {
                changed_lines(curbuf, top + 1, 0, bot, newsize - oldsize, do_buf_event);
                if spell_check_window(curwin) && bot <= (*curbuf).b_ml.ml_line_count {
                    redrawWinline(curwin, bot);
                }
            }

            (*curbuf).b_op_start.lnum = (*curbuf).b_op_start.lnum.min(top + 1);
            if newsize == 0 && top + 1 > (*curbuf).b_op_end.lnum {
                (*curbuf).b_op_end.lnum = top + 1;
            } else if top + newsize > (*curbuf).b_op_end.lnum {
                (*curbuf).b_op_end.lnum = top + newsize;
            }

            u_newcount += newsize;
            u_oldcount += oldsize;
            (*uep).ue_size = oldsize;
            (*uep).ue_array = newarray;
            (*uep).ue_bot = top + newsize + 1;

            nuep = (*uep).ue_next;
            (*uep).ue_next = newlist;
            newlist = uep;
            uep = nuep;
        }

        (*curbuf).b_op_start.lnum = (*curbuf).b_op_start.lnum.min((*curbuf).b_ml.ml_line_count);
        (*curbuf).b_op_end.lnum = (*curbuf).b_op_end.lnum.min((*curbuf).b_ml.ml_line_count);

        if undo {
            let mut i = (*curhead).uh_extmark.size as isize - 1;
            while i > -1 {
                extmark_apply_undo(*(*curhead).uh_extmark.items.add(i as usize), undo);
                i -= 1;
            }
        } else {
            for i in 0..(*curhead).uh_extmark.size {
                extmark_apply_undo(*(*curhead).uh_extmark.items.add(i), undo);
            }
        }
        if ((*curhead).uh_flags & UH_RELOAD) != 0 {
            buf_updates_unload(curbuf, true);
        }

        (*curwin).w_cursor = new_curpos;
        check_cursor_lnum(curwin);

        (*curhead).uh_entry = newlist;
        (*curhead).uh_flags = new_flags;
        if (old_flags & UH_EMPTYBUF) != 0 && buf_is_empty(curbuf) {
            (*curbuf).b_ml.ml_flags |= ML_EMPTY;
        }
        if (old_flags & UH_CHANGED) != 0 {
            changed(curbuf);
        } else {
            unchanged(curbuf, false, true);
        }

        if do_buf_event {
            buf_updates_changedtick(curbuf);
        }

        for i in 0..NMARKS {
            if (*curhead).uh_namedm[i].mark.lnum != 0 {
                free_fmark((*curbuf).b_namedm[i]);
                (*curbuf).b_namedm[i] = (*curhead).uh_namedm[i];
            }
            if namedm[i].mark.lnum != 0 {
                (*curhead).uh_namedm[i] = namedm[i];
            } else {
                (*curhead).uh_namedm[i].mark.lnum = 0;
            }
        }
        if (*curhead).uh_visual.vi_start.lnum != 0 {
            (*curbuf).b_visual = (*curhead).uh_visual;
            (*curhead).uh_visual = visualinfo;
        }

        if undo
            && (*curhead).uh_cursor.lnum >= 1
            && (*curhead).uh_cursor.lnum <= (*curbuf).b_ml.ml_line_count
        {
            (*curwin).w_cursor = (*curhead).uh_cursor;
            if virtual_active(curwin) && (*curhead).uh_cursor_vcol >= 0 {
                coladvance(curwin, (*curhead).uh_cursor_vcol);
            }
        } else {
            if (*curhead).uh_cursor.lnum + 1 == (*curwin).w_cursor.lnum
                && (*curwin).w_cursor.lnum > 1
            {
                (*curwin).w_cursor.lnum -= 1;
            }
            if (*curwin).w_cursor.lnum <= (*curbuf).b_ml.ml_line_count {
                if (*curhead).uh_cursor.lnum == (*curwin).w_cursor.lnum {
                    (*curwin).w_cursor.col = (*curhead).uh_cursor.col;
                    if virtual_active(curwin) && (*curhead).uh_cursor_vcol >= 0 {
                        coladvance(curwin, (*curhead).uh_cursor_vcol);
                    } else {
                        (*curwin).w_cursor.coladd = 0;
                    }
                } else {
                    beginline(BL_SOL | BL_FIX);
                }
            } else {
                (*curwin).w_cursor.col = 0;
                (*curwin).w_cursor.coladd = 0;
            }
        }

        check_cursor(curwin);

        if !undo && (*curhead).uh_cursor_after.lnum > 0 {
            (*curwin).w_cursor = (*curhead).uh_cursor_after;
            check_cursor(curwin);
        }

        (*curbuf).b_u_seq_cur = (*curhead).uh_seq;
        if undo {
            (*curbuf).b_u_seq_cur = if !(*curhead).uh_next.ptr.is_null() {
                (*(*curhead).uh_next.ptr).uh_seq
            } else {
                0
            };
        }

        if (*curhead).uh_save_nr != 0 {
            if undo {
                (*curbuf).b_u_save_nr_cur = (*curhead).uh_save_nr - 1;
            } else {
                (*curbuf).b_u_save_nr_cur = (*curhead).uh_save_nr;
            }
        }

        (*curbuf).b_u_time_cur = (*curhead).uh_time;

        unblock_autocmds();
    }
}

unsafe fn u_undo_end(mut did_undo: bool, absolute: bool, quiet: bool) {
    unsafe {
        if (fdo_flags & kOptFdoFlagUndo) != 0 && KeyTyped {
            foldOpenCursor();
        }

        if quiet || global_busy != 0 || !messaging() || shortmess(kShmUndo) {
            return;
        }

        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            u_newcount -= 1;
        }

        u_oldcount -= u_newcount;
        let msgstr = if u_oldcount == -1 {
            c"more line".as_ptr()
        } else if u_oldcount < 0 {
            c"more lines".as_ptr()
        } else if u_oldcount == 1 {
            c"line less".as_ptr()
        } else if u_oldcount > 1 {
            c"fewer lines".as_ptr()
        } else {
            u_oldcount = u_newcount;
            if u_newcount == 1 {
                c"change".as_ptr()
            } else {
                c"changes".as_ptr()
            }
        };

        let curhead = (*curbuf).b_u_curhead.cast::<u_header_T>();
        let uhp = if !curhead.is_null() {
            if absolute && !(*curhead).uh_next.ptr.is_null() {
                did_undo = false;
                (*curhead).uh_next.ptr
            } else if did_undo {
                curhead
            } else {
                (*curhead).uh_next.ptr
            }
        } else {
            (*curbuf).b_u_newhead.cast::<u_header_T>()
        };

        let mut msgbuf = [0 as c_char; 80];
        if !uhp.is_null() {
            undo_fmt_time(msgbuf.as_mut_ptr(), msgbuf.len(), (*uhp).uh_time);
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_buffer == curbuf && (*wp).w_onebuf_opt.wo_cole > 0 {
                redraw_later(wp, UPD_NOT_VALID);
            }
            wp = (*wp).w_next;
        }

        if Visual.active {
            check_pos(curbuf, &raw mut Visual.start);
        }

        smsg_keep(
            0,
            gettext(c"%lld %s; %s #%lld  %s".as_ptr()),
            if u_oldcount < 0 {
                -(u_oldcount as i64)
            } else {
                u_oldcount as i64
            },
            gettext(msgstr),
            if did_undo {
                gettext(c"before".as_ptr())
            } else {
                gettext(c"after".as_ptr())
            },
            if uhp.is_null() {
                0i64
            } else {
                (*uhp).uh_seq as i64
            },
            msgbuf.as_ptr(),
        );
    }
}

/// Put the timestamp of an undo header in `buf[buflen]` in a nice format.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn undo_fmt_time(buf: *mut c_char, buflen: usize, tt: time_t) {
    unsafe {
        let now = c_time(ptr::null_mut());
        if now - tt >= 100 {
            let mut curtime = Tm { _fields: [0; 16] };
            os_localtime_r(&raw const tt, &raw mut curtime);
            let n = if c_time(ptr::null_mut()) - tt < 60 * 60 * 12 {
                strftime(buf, buflen, c"%H:%M:%S".as_ptr(), &raw const curtime)
            } else {
                strftime(
                    buf,
                    buflen,
                    c"%Y/%m/%d %H:%M:%S".as_ptr(),
                    &raw const curtime,
                )
            };
            if n == 0 {
                *buf = NUL;
            }
        } else {
            let seconds = now - tt;
            vim_snprintf(
                buf,
                buflen,
                ngettext(
                    c"%lld second ago".as_ptr(),
                    c"%lld seconds ago".as_ptr(),
                    seconds as u32 as c_ulong,
                ),
                seconds,
            );
        }
    }
}

/// `u_sync`: stop adding to the current entry list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_sync(force: bool) {
    unsafe {
        if (*curbuf).b_u_synced || (!force && no_u_sync > 0) {
            return;
        }

        if get_undolevel(curbuf) < 0 {
            (*curbuf).b_u_synced = true;
        } else {
            u_getbot(curbuf);
            (*curbuf).b_u_curhead = ptr::null_mut();
        }
    }
}

/// `":undolist"`: List the leafs of the undo tree.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_undolist(_eap: *mut exarg_T) {
    unsafe {
        let mut changes: c_int = 1;

        lastmark += 1;
        let mark = lastmark;
        lastmark += 1;
        let nomark = lastmark;
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut ga, size_of::<*mut c_char>() as c_int, 20);

        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
        let mut uhp = (*curbuf).b_u_oldhead.cast::<u_header_T>();
        while !uhp.is_null() {
            if (*uhp).uh_prev.ptr.is_null() && (*uhp).uh_walk != nomark && (*uhp).uh_walk != mark {
                vim_snprintf(
                    iobuf_ptr,
                    IOSIZE,
                    c"%6d %7d  ".as_ptr(),
                    (*uhp).uh_seq,
                    changes,
                );
                let cur_len = strlen(iobuf_ptr);
                undo_fmt_time(iobuf_ptr.add(cur_len), IOSIZE - cur_len, (*uhp).uh_time);
                if (*uhp).uh_save_nr > 0 {
                    while strlen(iobuf_ptr) < 33 {
                        xstrlcat(iobuf_ptr, c" ".as_ptr(), IOSIZE);
                    }
                    vim_snprintf_add(iobuf_ptr, IOSIZE, c"  %3d".as_ptr(), (*uhp).uh_save_nr);
                }
                ga_grow(&raw mut ga, 1);
                *(ga.ga_data.cast::<*mut c_char>()).add(ga.ga_len as usize) = xstrdup(iobuf_ptr);
                ga.ga_len += 1;
            }

            (*uhp).uh_walk = mark;

            if !(*uhp).uh_prev.ptr.is_null()
                && (*(*uhp).uh_prev.ptr).uh_walk != nomark
                && (*(*uhp).uh_prev.ptr).uh_walk != mark
            {
                uhp = (*uhp).uh_prev.ptr;
                changes += 1;
            } else if !(*uhp).uh_alt_next.ptr.is_null()
                && (*(*uhp).uh_alt_next.ptr).uh_walk != nomark
                && (*(*uhp).uh_alt_next.ptr).uh_walk != mark
            {
                uhp = (*uhp).uh_alt_next.ptr;
            } else if !(*uhp).uh_next.ptr.is_null()
                && (*uhp).uh_alt_prev.ptr.is_null()
                && (*(*uhp).uh_next.ptr).uh_walk != nomark
                && (*(*uhp).uh_next.ptr).uh_walk != mark
            {
                uhp = (*uhp).uh_next.ptr;
                changes -= 1;
            } else {
                (*uhp).uh_walk = nomark;
                if !(*uhp).uh_alt_prev.ptr.is_null() {
                    uhp = (*uhp).uh_alt_prev.ptr;
                } else {
                    uhp = (*uhp).uh_next.ptr;
                    changes -= 1;
                }
            }
        }

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if ga.ga_len <= 0 {
            msg(gettext(c"Nothing to undo".as_ptr()), 0);
        } else {
            sort_strings(ga.ga_data.cast::<*mut c_char>(), ga.ga_len);

            msg_start();
            msg_puts_hl(
                gettext(c"number changes  when               saved".as_ptr()),
                HLF_T,
                false,
            );
            let mut i = 0;
            while i < ga.ga_len && !got_int {
                msg_putchar(b'\n' as c_int);
                if got_int {
                    break;
                }
                msg_puts(*(ga.ga_data.cast::<*const c_char>()).add(i as usize));
                i += 1;
            }
            msg_end();

            ga_clear_strings(&raw mut ga);
        }
    }
}

/// `":undojoin"`: continue adding to the last entry list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_undojoin(_eap: *mut exarg_T) {
    unsafe {
        if (*curbuf).b_u_newhead.is_null() {
            return;
        }
        if !(*curbuf).b_u_curhead.is_null() {
            emsg(gettext(
                c"E790: undojoin is not allowed after undo".as_ptr(),
            ));
            return;
        }
        if !(*curbuf).b_u_synced {
            return;
        }
        if get_undolevel(curbuf) < 0 {
            return;
        }
        (*curbuf).b_u_synced = false;
    }
}

/// Called after writing or reloading the file and setting `b_changed` to false.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_unchanged(buf: *mut buf_T) {
    unsafe {
        u_unch_branch((*buf).b_u_oldhead.cast::<u_header_T>());
        (*buf).b_did_warn = false;
    }
}

/// After reloading a buffer which was saved for `'undoreload'`: Find the first
/// line that was changed and set the cursor there.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_find_first_changed() {
    unsafe {
        let uhp = (*curbuf).b_u_newhead.cast::<u_header_T>();
        if !(*curbuf).b_u_curhead.is_null() || uhp.is_null() {
            return;
        }
        let uep = (*uhp).uh_entry;
        if (*uep).ue_top != 0 || (*uep).ue_bot != 0 {
            return;
        }

        let mut lnum: linenr_T = 1;
        while lnum < (*curbuf).b_ml.ml_line_count && lnum <= (*uep).ue_size {
            if strcmp(
                ml_get_buf(curbuf, lnum),
                *(*uep).ue_array.add((lnum - 1) as usize),
            ) != 0
            {
                clearpos(&mut (*uhp).uh_cursor);
                (*uhp).uh_cursor.lnum = lnum;
                return;
            }
            lnum += 1;
        }
        if (*curbuf).b_ml.ml_line_count != (*uep).ue_size {
            clearpos(&mut (*uhp).uh_cursor);
            (*uhp).uh_cursor.lnum = lnum;
        }
    }
}

/// Increase the write count, store it in the last undo header, what would be used for `"u"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_update_save_nr(buf: *mut buf_T) {
    unsafe {
        (*buf).b_u_save_nr_last += 1;
        (*buf).b_u_save_nr_cur = (*buf).b_u_save_nr_last;
        let mut uhp = (*buf).b_u_curhead.cast::<u_header_T>();
        if !uhp.is_null() {
            uhp = (*uhp).uh_next.ptr;
        } else {
            uhp = (*buf).b_u_newhead.cast::<u_header_T>();
        }
        if !uhp.is_null() {
            (*uhp).uh_save_nr = (*buf).b_u_save_nr_last;
        }
    }
}

unsafe fn u_unch_branch(uhp: *mut u_header_T) {
    unsafe {
        let mut uh = uhp;
        while !uh.is_null() {
            (*uh).uh_flags |= UH_CHANGED;
            if !(*uh).uh_alt_next.ptr.is_null() {
                u_unch_branch((*uh).uh_alt_next.ptr);
            }
            uh = (*uh).uh_prev.ptr;
        }
    }
}

unsafe fn u_get_headentry(buf: *mut buf_T) -> *mut u_entry_T {
    unsafe {
        let newhead = (*buf).b_u_newhead.cast::<u_header_T>();
        if newhead.is_null() || (*newhead).uh_entry.is_null() {
            iemsg(gettext(E_UNDO_LIST_CORRUPT.as_ptr()));
            return ptr::null_mut();
        }
        (*newhead).uh_entry
    }
}

unsafe fn u_getbot(buf: *mut buf_T) {
    unsafe {
        let uep = u_get_headentry(buf);
        if uep.is_null() {
            return;
        }

        let newhead = (*buf).b_u_newhead.cast::<u_header_T>();
        let bot_entry = (*newhead).uh_getbot_entry;
        if !bot_entry.is_null() {
            let extra = (*buf).b_ml.ml_line_count - (*bot_entry).ue_lcount;
            (*bot_entry).ue_bot = (*bot_entry).ue_top + (*bot_entry).ue_size + 1 + extra;
            if (*bot_entry).ue_bot < 1 || (*bot_entry).ue_bot > (*buf).b_ml.ml_line_count {
                iemsg(gettext(E_UNDO_LINE_MISSING.as_ptr()));
                (*bot_entry).ue_bot = (*bot_entry).ue_top + 1;
            }

            (*newhead).uh_getbot_entry = ptr::null_mut();
        }

        (*buf).b_u_synced = true;
    }
}

unsafe fn u_freeheader(buf: *mut buf_T, uhp: *mut u_header_T, uhpp: *mut *mut u_header_T) {
    unsafe {
        if !(*uhp).uh_alt_next.ptr.is_null() {
            u_freebranch(buf, (*uhp).uh_alt_next.ptr, uhpp);
        }

        if !(*uhp).uh_alt_prev.ptr.is_null() {
            (*(*uhp).uh_alt_prev.ptr).uh_alt_next.ptr = ptr::null_mut();
        }

        if (*uhp).uh_next.ptr.is_null() {
            (*buf).b_u_oldhead = (*uhp).uh_prev.ptr.cast();
        } else {
            (*(*uhp).uh_next.ptr).uh_prev.ptr = (*uhp).uh_prev.ptr;
        }

        if (*uhp).uh_prev.ptr.is_null() {
            (*buf).b_u_newhead = (*uhp).uh_next.ptr.cast();
        } else {
            let mut uhap = (*uhp).uh_prev.ptr;
            while !uhap.is_null() {
                (*uhap).uh_next.ptr = (*uhp).uh_next.ptr;
                uhap = (*uhap).uh_alt_next.ptr;
            }
        }

        u_freeentries(buf, uhp, uhpp);
    }
}

unsafe fn u_freebranch(buf: *mut buf_T, uhp: *mut u_header_T, uhpp: *mut *mut u_header_T) {
    unsafe {
        if uhp == (*buf).b_u_oldhead.cast::<u_header_T>() {
            while !(*buf).b_u_oldhead.is_null() {
                u_freeheader(buf, (*buf).b_u_oldhead.cast::<u_header_T>(), uhpp);
            }
            return;
        }

        if !(*uhp).uh_alt_prev.ptr.is_null() {
            (*(*uhp).uh_alt_prev.ptr).uh_alt_next.ptr = ptr::null_mut();
        }

        let mut next = uhp;
        while !next.is_null() {
            let tofree = next;
            if !(*tofree).uh_alt_next.ptr.is_null() {
                u_freebranch(buf, (*tofree).uh_alt_next.ptr, uhpp);
            }
            next = (*tofree).uh_prev.ptr;
            u_freeentries(buf, tofree, uhpp);
        }
    }
}

unsafe fn u_forget_header(buf: *mut buf_T, uhp: *mut u_header_T) {
    unsafe {
        let parent = (*uhp).uh_next.ptr;
        let alt = (*uhp).uh_alt_next.ptr;
        if !alt.is_null() {
            (*alt).uh_alt_prev.ptr = (*uhp).uh_alt_prev.ptr;
        }
        if !(*uhp).uh_alt_prev.ptr.is_null() {
            (*(*uhp).uh_alt_prev.ptr).uh_alt_next.ptr = alt;
        }
        if !parent.is_null() {
            (*parent).uh_prev.ptr = alt;
        }
        if (*buf).b_u_oldhead.cast::<u_header_T>() == uhp {
            (*buf).b_u_oldhead = alt.cast();
        }
        (*uhp).uh_alt_next.ptr = ptr::null_mut();
        (*uhp).uh_alt_prev.ptr = ptr::null_mut();
        (*buf).b_u_newhead = parent.cast();
        (*buf).b_u_curhead = alt.cast();
        (*buf).b_u_seq_cur = if !parent.is_null() {
            (*parent).uh_seq
        } else {
            0
        };
        if (*buf).b_u_seq_last == (*uhp).uh_seq {
            (*buf).b_u_seq_last -= 1;
        }
        (*buf).b_u_synced = true;
        u_freebranch(buf, uhp, ptr::null_mut());
    }
}

unsafe fn u_freeentries(buf: *mut buf_T, uhp: *mut u_header_T, uhpp: *mut *mut u_header_T) {
    unsafe {
        if (*buf).b_u_curhead.cast::<u_header_T>() == uhp {
            (*buf).b_u_curhead = ptr::null_mut();
        }
        if (*buf).b_u_newhead.cast::<u_header_T>() == uhp {
            (*buf).b_u_newhead = ptr::null_mut();
        }
        if !uhpp.is_null() && uhp == *uhpp {
            *uhpp = ptr::null_mut();
        }

        let mut uep = (*uhp).uh_entry;
        while !uep.is_null() {
            let nuep = (*uep).ue_next;
            u_freeentry(uep, (*uep).ue_size);
            uep = nuep;
        }

        (*uhp).uh_extmark.destroy();

        xfree(uhp.cast());
        (*buf).b_u_numhead -= 1;
    }
}

unsafe fn u_freeentry(uep: *mut u_entry_T, mut n: c_int) {
    unsafe {
        while n > 0 {
            n -= 1;
            xfree((*(*uep).ue_array.add(n as usize)).cast());
        }
        xfree((*uep).ue_array.cast());
        xfree(uep.cast());
    }
}

/// Prunes the newest undo entry if it has no changes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_forget_unchanged(buf: *mut buf_T) {
    unsafe {
        let uhp = (*buf).b_u_newhead.cast::<u_header_T>();
        let uep = if !uhp.is_null() {
            (*uhp).uh_entry
        } else {
            ptr::null_mut()
        };
        if uep.is_null() || !(*buf).b_u_curhead.is_null() || u_entry_resized(buf, uep) {
            return;
        }
        for i in 0..(*uep).ue_size {
            if strcmp(
                *(*uep).ue_array.add(i as usize),
                ml_get_buf(buf, (*uep).ue_top + 1 + i),
            ) != 0
            {
                return;
            }
        }

        (*uhp).uh_entry = (*uep).ue_next;
        if (*uhp).uh_getbot_entry == uep {
            (*uhp).uh_getbot_entry = ptr::null_mut();
        }
        u_freeentry(uep, (*uep).ue_size);
        if (*uhp).uh_entry.is_null() {
            u_forget_header(buf, uhp);
        }
    }
}

/// Invalidate the undo buffer; called when storage has already been released.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_clearall(buf: *mut buf_T) {
    unsafe {
        (*buf).b_u_newhead = ptr::null_mut();
        (*buf).b_u_oldhead = ptr::null_mut();
        (*buf).b_u_curhead = ptr::null_mut();
        (*buf).b_u_synced = true;
        (*buf).b_u_numhead = 0;
        (*buf).b_u_line_ptr = ptr::null_mut();
        (*buf).b_u_line_lnum = 0;
    }
}

/// Free all allocated memory blocks for the buffer `buf`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_blockfree(buf: *mut buf_T) {
    unsafe {
        while !(*buf).b_u_oldhead.is_null() {
            let previous_oldhead = (*buf).b_u_oldhead;
            u_freeheader(
                buf,
                (*buf).b_u_oldhead.cast::<u_header_T>(),
                ptr::null_mut(),
            );
            assert!((*buf).b_u_oldhead != previous_oldhead);
        }
        xfree((*buf).b_u_line_ptr.cast());
    }
}

/// Free all allocated memory blocks for the buffer `buf` and invalidate the undo buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_clearallandblockfree(buf: *mut buf_T) {
    unsafe {
        u_blockfree(buf);
        u_clearall(buf);
    }
}

unsafe fn u_saveline(buf: *mut buf_T, lnum: linenr_T) {
    unsafe {
        if lnum == (*buf).b_u_line_lnum {
            return;
        }
        if lnum < 1 || lnum > (*buf).b_ml.ml_line_count {
            return;
        }
        u_clearline(buf);
        (*buf).b_u_line_lnum = lnum;
        if (*curwin).w_buffer == buf && (*curwin).w_cursor.lnum == lnum {
            (*buf).b_u_line_colnr = (*curwin).w_cursor.col;
        } else {
            (*buf).b_u_line_colnr = 0;
        }
        (*buf).b_u_line_ptr = u_save_line_buf(buf, lnum);
    }
}

/// Clear the line saved for the `"U"` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_clearline(buf: *mut buf_T) {
    unsafe {
        if (*buf).b_u_line_ptr.is_null() {
            return;
        }

        xfree((*buf).b_u_line_ptr.cast());
        (*buf).b_u_line_ptr = ptr::null_mut();
        (*buf).b_u_line_lnum = 0;
    }
}

/// Implementation of the `"U"` command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_undoline() {
    unsafe {
        if (*curbuf).b_u_line_ptr.is_null()
            || (*curbuf).b_u_line_lnum > (*curbuf).b_ml.ml_line_count
        {
            beep_flush();
            return;
        }
        atom_did_global_op();

        if u_savecommon(
            curbuf,
            (*curbuf).b_u_line_lnum - 1,
            (*curbuf).b_u_line_lnum + 1,
            0,
            false,
        ) == FAIL
        {
            return;
        }

        let oldp = u_save_line((*curbuf).b_u_line_lnum);
        ml_replace((*curbuf).b_u_line_lnum, (*curbuf).b_u_line_ptr, true);
        extmark_splice_cols(
            curbuf,
            (*curbuf).b_u_line_lnum - 1,
            0,
            strlen(oldp) as colnr_T,
            strlen((*curbuf).b_u_line_ptr) as colnr_T,
            kExtmarkUndo,
        );
        changed_bytes((*curbuf).b_u_line_lnum, 0);
        xfree((*curbuf).b_u_line_ptr.cast());
        (*curbuf).b_u_line_ptr = oldp;

        let t = (*curbuf).b_u_line_colnr;
        if (*curwin).w_cursor.lnum == (*curbuf).b_u_line_lnum {
            (*curbuf).b_u_line_colnr = (*curwin).w_cursor.col;
        }
        (*curwin).w_cursor.col = t;
        (*curwin).w_cursor.lnum = (*curbuf).b_u_line_lnum;
        check_cursor_col(curwin);
    }
}

unsafe fn u_save_line(lnum: linenr_T) -> *mut c_char {
    unsafe { u_save_line_buf(curbuf, lnum) }
}

unsafe fn u_save_line_buf(buf: *mut buf_T, lnum: linenr_T) -> *mut c_char {
    unsafe { xstrdup(ml_get_buf(buf, lnum)) }
}

/// Check if the `'modified'` flag is set, or `'ff'` has changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bufIsChanged(buf: *mut buf_T) -> bool {
    unsafe {
        if bt_prompt(buf) {
            (*buf).b_modified_was_set
        } else {
            !bt_dontwrite(buf) && ((*buf).b_changed != 0 || file_ff_differs(buf, true))
        }
    }
}

/// Return true if any buffer has changes. Also buffers that are not written.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn anyBufIsChanged() -> bool {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            if bufIsChanged(buf) {
                return true;
            }
            buf = (*buf).b_next;
        }
        false
    }
}

/// Return true if the current buffer has changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn curbufIsChanged() -> bool {
    unsafe { bufIsChanged(curbuf) }
}

unsafe fn u_eval_tree(buf: *mut buf_T, first_uhp: *const u_header_T) -> *mut list_T {
    unsafe {
        let list = tv_list_alloc(kListLenMayKnow);

        let mut uhp = first_uhp;
        while !uhp.is_null() {
            let dict = tv_dict_alloc();
            tv_dict_add_nr(dict, c"seq".as_ptr(), 3, (*uhp).uh_seq as varnumber_T);
            tv_dict_add_nr(dict, c"time".as_ptr(), 4, (*uhp).uh_time as varnumber_T);
            if uhp == (*buf).b_u_newhead.cast::<u_header_T>().cast_const() {
                tv_dict_add_nr(dict, c"newhead".as_ptr(), 7, 1);
            }
            if uhp == (*buf).b_u_curhead.cast::<u_header_T>().cast_const() {
                tv_dict_add_nr(dict, c"curhead".as_ptr(), 7, 1);
            }
            if (*uhp).uh_save_nr > 0 {
                tv_dict_add_nr(dict, c"save".as_ptr(), 4, (*uhp).uh_save_nr as varnumber_T);
            }

            if !(*uhp).uh_alt_next.ptr.is_null() {
                tv_dict_add_list(
                    dict,
                    c"alt".as_ptr(),
                    3,
                    u_eval_tree(buf, (*uhp).uh_alt_next.ptr),
                );
            }

            tv_list_append_dict(list, dict);
            uhp = (*uhp).uh_prev.ptr;
        }

        list
    }
}

/// `"undofile(name)"` function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_undofile(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let fname = tv_get_string(argvars);

        if *fname == NUL {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            let ffname = FullName_save(fname, true);
            if !ffname.is_null() {
                (*rettv).vval.v_string = u_get_undo_file_name(ffname, false);
            }
            xfree(ffname.cast());
        }
    }
}

/// `"undotree(expr)"` function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_undotree(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);

        let tv = argvars;
        let buf = if (*tv).v_type == VarType::VAR_UNKNOWN {
            curbuf
        } else {
            get_buf_arg(tv)
        };
        if buf.is_null() {
            return;
        }

        let dict = (*rettv).vval.v_dict;

        tv_dict_add_nr(
            dict,
            c"synced".as_ptr(),
            6,
            varnumber_T::from((*buf).b_u_synced),
        );
        tv_dict_add_nr(
            dict,
            c"seq_last".as_ptr(),
            8,
            (*buf).b_u_seq_last as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"save_last".as_ptr(),
            9,
            (*buf).b_u_save_nr_last as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"seq_cur".as_ptr(),
            7,
            (*buf).b_u_seq_cur as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"time_cur".as_ptr(),
            8,
            (*buf).b_u_time_cur as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"save_cur".as_ptr(),
            8,
            (*buf).b_u_save_nr_cur as varnumber_T,
        );

        tv_dict_add_list(
            dict,
            c"entries".as_ptr(),
            7,
            u_eval_tree(buf, (*buf).b_u_oldhead.cast::<u_header_T>()),
        );
    }
}

/// Drops named mark `idx` from the pending undo snapshot.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_update_named_mark(buf: *mut buf_T, idx: c_int) {
    unsafe {
        let uhp = if !(*buf).b_u_curhead.is_null() {
            (*buf).b_u_curhead.cast::<u_header_T>()
        } else {
            (*buf).b_u_newhead.cast::<u_header_T>()
        };
        if !uhp.is_null() {
            (*uhp).uh_namedm[idx as usize].mark.lnum = 0;
        }
    }
}

/// Given the buffer, return the undo header. If none is set, set one first.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_force_get_undo_header(buf: *mut buf_T) -> *mut u_header_T {
    unsafe {
        let mut uhp: *mut u_header_T = if !(*buf).b_u_curhead.is_null() {
            (*buf).b_u_curhead.cast::<u_header_T>()
        } else if !(*buf).b_u_newhead.is_null() {
            (*buf).b_u_newhead.cast::<u_header_T>()
        } else {
            ptr::null_mut()
        };

        if uhp.is_null() {
            u_savecommon(buf, 0, 1, 1, true);
            uhp = (*buf).b_u_curhead.cast::<u_header_T>();
            if uhp.is_null() {
                uhp = (*buf).b_u_newhead.cast::<u_header_T>();
                if get_undolevel(buf) > 0 && uhp.is_null() {
                    std::process::abort();
                }
            }
        }
        uhp
    }
}

/// Checkpoints the undo state of `buf` and detaches its undotree.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_checkpoint(uc: *mut UndoCheckpoint, buf: *mut buf_T) {
    unsafe {
        (*uc).uc_synced = (*buf).b_u_synced;
        (*uc).uc_oldhead = (*buf).b_u_oldhead.cast::<u_header_T>();
        (*uc).uc_newhead = (*buf).b_u_newhead.cast::<u_header_T>();
        (*uc).uc_curhead = (*buf).b_u_curhead.cast::<u_header_T>();
        (*uc).uc_numhead = (*buf).b_u_numhead;
        (*uc).uc_seq_last = (*buf).b_u_seq_last;
        (*uc).uc_save_nr_last = (*buf).b_u_save_nr_last;
        (*uc).uc_seq_cur = (*buf).b_u_seq_cur;
        (*uc).uc_time_cur = (*buf).b_u_time_cur;
        (*uc).uc_save_nr_cur = (*buf).b_u_save_nr_cur;
        (*uc).uc_line_ptr = (*buf).b_u_line_ptr;
        (*uc).uc_line_lnum = (*buf).b_u_line_lnum;
        (*uc).uc_line_colnr = (*buf).b_u_line_colnr;
        (*uc).uc_undolevels = (*buf).b_p_ul;
        (*uc).uc_changedtick = buf_get_changedtick(buf);

        u_clearall(buf);
        (*buf).b_p_ul = OptInt::from(c_int::MAX);
    }
}

/// Reverts `buf` from a checkpoint.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn u_rollback(uc: *mut UndoCheckpoint, buf: *mut buf_T) {
    unsafe {
        if (*buf).b_u_seq_cur != (*uc).uc_seq_cur {
            let mut count: c_int = 0;
            let mut uhp = if !(*buf).b_u_curhead.is_null() {
                (*buf).b_u_curhead.cast::<u_header_T>()
            } else {
                (*buf).b_u_newhead.cast::<u_header_T>()
            };
            while !uhp.is_null() {
                uhp = (*uhp).uh_next.ptr;
                count += 1;
            }

            let mut cs: CtxSwitch = std::mem::zeroed();
            ctx_switch(&raw mut cs, ptr::null_mut(), ptr::null_mut(), buf, 0);
            if !(*curbuf).b_u_synced {
                u_sync(true);
            }
            if !u_undo_and_forget(count, false) {
                std::process::abort();
            }
            ctx_restore(&raw mut cs);
        }

        u_blockfree(buf);
        (*buf).b_u_oldhead = (*uc).uc_oldhead.cast();
        (*buf).b_u_newhead = (*uc).uc_newhead.cast();
        (*buf).b_u_curhead = (*uc).uc_curhead.cast();
        (*buf).b_u_numhead = (*uc).uc_numhead;
        (*buf).b_u_seq_last = (*uc).uc_seq_last;
        (*buf).b_u_save_nr_last = (*uc).uc_save_nr_last;
        (*buf).b_u_seq_cur = (*uc).uc_seq_cur;
        (*buf).b_u_time_cur = (*uc).uc_time_cur;
        (*buf).b_u_save_nr_cur = (*uc).uc_save_nr_cur;
        (*buf).b_u_line_ptr = (*uc).uc_line_ptr;
        (*buf).b_u_line_lnum = (*uc).uc_line_lnum;
        (*buf).b_u_line_colnr = (*uc).uc_line_colnr;
        if (*buf).b_u_curhead.is_null() {
            (*buf).b_u_synced = (*uc).uc_synced;
        }

        if (*uc).uc_changedtick != buf_get_changedtick(buf) {
            buf_set_changedtick(buf, (*uc).uc_changedtick);
        }
        (*buf).b_p_ul = (*uc).uc_undolevels;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_and_clearpos_helpers_work() {
        let mut p = pos_T {
            lnum: 5,
            col: 3,
            coladd: 1,
        };
        assert!(equalpos(
            p,
            pos_T {
                lnum: 5,
                col: 3,
                coladd: 1,
            }
        ));
        clearpos(&mut p);
        assert_eq!(p.lnum, 0);
        assert_eq!(p.col, 0);
        assert_eq!(p.coladd, 0);
    }
}
