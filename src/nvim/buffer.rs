//! Functions for dealing with the buffer structure (`src/nvim/buffer.c`).

#![cfg_attr(test, allow(unused_variables))]
#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use crate::api::options::{BF_DUMMY, CharBuf, CtxSwitch, IOSIZE};
use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO, cstr_as_string, tv_dict_is_watched};
use crate::api::tabpage::{pos_T, tabpage_T, win_T};
use crate::arglist::{ML_EMPTY, alist_T, check_arg_idx};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_BUFENTER, EVENT_BUFHIDDEN, EVENT_BUFLEAVE, EVENT_BUFNEW,
    EVENT_BUFUNLOAD, EVENT_BUFWINENTER, EVENT_BUFWINLEAVE, EVENT_BUFWIPEOUT, EVENT_STDINREADPOST,
    apply_autocmds, apply_autocmds_retval, apply_autocmds_win, aubuflocal_remove, augroup_exists,
    block_autocmds, do_doautocmd, unblock_autocmds,
};
use crate::buffer_updates::{buf_free_callbacks, buf_updates_unload};
use crate::change::{changed, deleted_lines_mark, file_ff_differs, save_file_ff, unchanged};
use crate::channel::channel_job_running;
use crate::charset::{
    ascii_isdigit, ascii_isspace, buf_init_isk_chartab, getdigits_int, skiptowhite_esc, skipwhite,
    trans_characters, try_getdigits, vim_strsize,
};
use crate::clipboard::{TypvalVval, VarType, typval_T};
use crate::cmdexpand::{
    BUF_DIFF_FILTER, WILD_BUFLASTUSED, WILD_HOME_REPLACE, cmdidx_T, cmdline_fuzzy_complete, exarg_T,
};
use crate::context::{ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{check_cursor_col, check_cursor_lnum, colnr_T, linenr_T};
use crate::diff::{diff_buf_add, diff_buf_delete, diff_mode_buf, diffopt_hiddenoff};
use crate::digraph::{keymap_ga_clear, keymap_init};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, redrawing, status_redraw_all};
use crate::errors::*;
use crate::eval::buffer::varnumber_T;
use crate::eval::deprecated::{ChangedtickDictItem, FileID, buf_T, fmark_T, fmarkv_T, synblock_T};
use crate::eval::typval::{
    callback_free, tv_dict_add, tv_dict_alloc, tv_dict_find, tv_dict_item_copy,
    tv_dict_watcher_notify,
};
use crate::eval::vars::{init_var_dict, unref_var_dict, vars_clear};
use crate::ex_cmds::{ECMD_FORCEIT, ECMD_ONE, do_ecmd, getfile};
use crate::ex_cmds2::{autowrite, can_abandon, dialog_changed, dialog_close_terminal};
use crate::ex_docmd::{do_cmdline_cmd, ex_errmsg, tabpage_new};
use crate::ex_eval::{aborting, cleanup_T, enter_cleanup, leave_cleanup};
use crate::ex_getln::text_or_buf_locked;
use crate::extmark::extmark_free_all;
use crate::file_search::vim_chdirfile;
use crate::fileio::{
    READ_BUFFER, READ_DUMMY, READ_FIFO, READ_NEW, READ_NOFILE, READ_NOWINENTER, READ_STDIN,
    buf_check_timestamp, file_pat_to_reg_pat, prep_exarg, readfile, shorten_fnames,
};
use crate::fold::{clearFolding, cloneFoldGrowArray, deleteFoldRecurse, foldUpdateAll};
use crate::fuzzy::{FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_to_strmatches};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::hashtab::{hash_find, hash_init, hash_remove};
use crate::help::get_local_additions;
use crate::indent::inindent;
use crate::indent_c::parse_cino;
use crate::input::vgetc;
use crate::input_cmdatom::StringBuilder;
use crate::insexpand::clear_cpt_callbacks;
use crate::lua::stdlib::regmatch_T;
use crate::main_init::getout;
use crate::map::{Map, map_del_int_ptr_t, map_put_ref_int_ptr_t, mh_get_int, ptr_t};
use crate::map_glyph_cache::MH_TOMBSTONE;
use crate::mapping::map_clear_mode;
use crate::mark::{
    WinInfo, clear_fmark, clrallmarks, fmarks_check_names, free_fmark, mark_adjust_buf,
    mark_forget_file, mark_jumplist_forget_file, mark_view_make, mark_view_restore,
    set_last_cursor, setpcmark,
};
use crate::math::{FAIL, OK};
use crate::mbyte::utf_cp_bounds;
use crate::mcursor::{mc_buf_clear, mc_buf_free};
use crate::memline::{
    ml_close, ml_delete, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open, ml_recover,
    ml_setname, ml_timestamp,
};
use crate::memory::{entered_free_all_mem, xcalloc, xfree, xmalloc, xstrdup, xstrlcpy};
use crate::message::{
    emsg, message_filtered, msg, msg_delay, msg_ext_set_kind, msg_outtrans, msg_putchar, msg_puts,
    msg_start, msg_trunc, semsg, set_keep_msg, smsg,
};
use crate::r#move::{scroll_cursor_halfway, validate_virtcol};
use crate::normal::{end_visual_mode, reset_VIsual_and_resel};
use crate::option::{
    buf_copy_options, clear_winopt, copy_winopt, didset_window_options, do_set, shortmess,
};
use crate::option_vars::*;
use crate::optionstr::{UIExtension, clear_string_option};
use crate::os::env::{home_replace, home_replace_save};
#[cfg(unix)]
use crate::os::fs::os_getperm;
#[cfg(all(not(test), windows))]
use crate::os::fs::os_resolve_shortcut;
use crate::os::fs::{os_fileid, os_fileid_equal, os_isdir};
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::os::time::os_time;
use crate::path::{
    FullName_save, after_pathsep, concat_fnames_realloc, fix_fname, get_past_head, kPathCmpLiteral,
    path_equal, path_fix_case, path_tail, path_tail_with_sep, to_slash,
};
use crate::plines::win_get_fill;
use crate::quickfix::qf_stack_get_bufnr;
use crate::regexp::{RE_MAGIC, vim_regcomp, vim_regexec, vim_regfree};
use crate::runtime::{ETYPE_MODELINE, estack_pop, estack_push};
use crate::spell::parse_spelllang;
use crate::state::{MAP_ALL_MODES, MODE_INSERT};
use crate::statusline::build_stl_str_hl;
use crate::strings::{
    gettext, ngettext, qsort, strcmp, strlen, strncmp, vim_snprintf, vim_snprintf_safelen,
    vim_strchr, xstrnsave,
};
use crate::syntax::{reset_synblock, syntax_clear};
use crate::terminal::{Terminal, terminal_check_size, terminal_close, terminal_running};
use crate::ui::{ui_call_set_icon, ui_call_set_title, ui_has};
use crate::undo::{u_clearallandblockfree, u_sync, undo_fmt_time};
use crate::usercmd::uc_clear;
use crate::version::min_vim_version;
use crate::window::{
    WSP_BELOW, WSP_ROOM, WSP_VERT, check_can_set_curbuf_forceit, check_colorcolumn, close_windows,
    free_wininfo, global_stl_height, goto_tabpage_tp, goto_tabpage_win, last_window,
    lastwin_nofloating, one_window, swbuf_goto_win_with_buf, tabline_height, tabpage_index,
    update_cwd, win_close, win_enter, win_locked, win_move_after, win_split, win_valid,
    win_valid_any_tab, window_layout_lock, window_layout_unlock,
};
use crate::winfloat::{win_float_update_preview, win_set_minimal_style};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

pub use crate::option::OptInt;
pub use crate::optionstr::OptIndex;
pub type MetaIndex = usize;

pub const GETF_SETMARK: c_int = 0x01;
pub const GETF_ALT: c_int = 0x02;
pub const GETF_SWITCH: c_int = 0x04;

pub const GETFILE_ERROR: c_int = 1;
pub const GETFILE_NOT_WRITTEN: c_int = 2;
pub const GETFILE_SAME_FILE: c_int = 0;
pub const GETFILE_OPEN_OTHER: c_int = -1;
pub const GETFILE_UNUSED: c_int = 8;

pub const BLN_CURBUF: c_int = 1;
pub const BLN_LISTED: c_int = 2;
pub const BLN_DUMMY: c_int = 4;
pub const BLN_NEW: c_int = 8;
pub const BLN_NOOPT: c_int = 16;
pub const BLN_NOCURWIN: c_int = 128;

pub const DOBUF_GOTO: c_int = 0;
pub const DOBUF_SPLIT: c_int = 1;
pub const DOBUF_UNLOAD: c_int = 2;
pub const DOBUF_DEL: c_int = 3;
pub const DOBUF_WIPE: c_int = 4;

pub const DOBUF_CURRENT: c_int = 0;
pub const DOBUF_FIRST: c_int = 1;
pub const DOBUF_LAST: c_int = 2;
pub const DOBUF_MOD: c_int = 3;

pub const DOBUF_FORCEIT: c_int = 1;
pub const DOBUF_SKIPHELP: c_int = 4;

pub const BFA_DEL: c_int = 1;
pub const BFA_WIPE: c_int = 2;
pub const BFA_KEEP_UNDO: c_int = 4;
pub const BFA_IGNORE_ABORT: c_int = 8;

const kBffClearWinInfo: c_int = 1;
const kBffInitChangedtick: c_int = 2;

pub(crate) const NOTDONE: c_int = 2;
pub(crate) const MAXLNUM: linenr_T = 0x7fffffff;
pub(crate) const NMARKS: usize = 26;
pub(crate) const JUMPLISTSIZE: usize = 100;
pub(crate) const DO_NOT_FREE_CNT: c_int = c_int::MAX / 2;

pub(crate) const BF_CHECK_RO: c_int = 0x02;
pub(crate) const BF_NEVERLOADED: c_int = 0x04;
pub(crate) const BF_NOTEDITED: c_int = 0x08;
pub(crate) const BF_NEW: c_int = 0x10;
pub(crate) const BF_READERR: c_int = 0x40;
pub(crate) const BF_WRITE_MASK: c_int = BF_NOTEDITED + BF_NEW + BF_READERR;

pub(crate) const VALID_TOPLINE: c_int = 0x80;
pub(crate) const MF_DIRTY_YES: c_int = 1;
pub(crate) const MF_DIRTY_YES_NOSYNC: c_int = 2;

pub(crate) const kShmRo: c_int = b'r' as c_int;
const kShmMod: c_int = b'm' as c_int;
pub(crate) const kShmFileinfo: c_int = b'F' as c_int;
const kCpoIntmod: c_int = b'i' as c_int;

pub(crate) const kMarkAdjustNormal: c_int = 0;
pub(crate) const kExtmarkNoUndo: c_int = 2;

pub(crate) const SEA_NONE: c_int = 0;
pub(crate) const SEA_DIALOG: c_int = 1;
pub(crate) const SEA_QUIT: c_int = 2;
pub(crate) const SEA_RECOVER: c_int = 3;

pub(crate) const kOptJopFlagView: c_uint = 0x02;
const kOptJopFlagClean: c_uint = 0x04;

const kOptSwbFlagSplit: c_uint = 0x04;
pub(crate) const kOptSwbFlagNewtab: c_uint = 0x08;
pub(crate) const kOptSwbFlagVsplit: c_uint = 0x10;

pub(crate) const kCdCauseBuffer: c_int = 2;
pub(crate) const kCdCauseAuto: c_int = 3;

pub(crate) const BCO_ALWAYS: c_int = 2;

pub(crate) const KEYMAP_INIT: i16 = 1;

pub(crate) const VAR_FIXED: c_int = 2;
pub(crate) const VAR_SCOPE: c_int = 1;

pub(crate) const NO_LOCAL_UNDOLEVEL: OptInt = -123456;

pub(crate) const kUIMessages: UIExtension = 4;

pub(crate) const STL_IN_ICON: c_int = 1;
pub(crate) const STL_IN_TITLE: c_int = 2;
const kOptIconstring: OptIndex = 139;
const kOptTitlestring: OptIndex = 331;

pub(crate) const SID_MODELINE: c_int = -1;
pub(crate) const OPT_MODELINE: c_int = 0x04;

const CASE_INSENSITIVE_FILENAME: bool = cfg!(any(target_os = "windows", target_os = "macos"));

const INIT_FMARKV: fmarkv_T = fmarkv_T {
    topline_offset: MAXLNUM,
    skipcol: 0,
};

const INIT_FMARK: fmark_T = fmark_T {
    mark: pos_T {
        lnum: 0,
        col: 0,
        coladd: 0,
    },
    fnum: 0,
    timestamp: 0,
    view: INIT_FMARKV,
    additional_data: ptr::null_mut(),
};

use crate::api::autocmd::kCallbackNone;
use crate::api::buffer::{FORWARD, NL, NUL};
use crate::api::command::{CMOD_CONFIRM, CMOD_HIDE, CMOD_KEEPALT};
use crate::api::deprecated::OPT_LOCAL;
pub use crate::api::options::bufref_T;
use crate::api::vim::{BCO_ENTER, BCO_NOHELP, MAXPATHL};
use crate::api::win_config::{kCtxNoDisplay, kWinStyleMinimal};

#[repr(C)]
struct memfile_head_T {
    mf_fname: *mut c_char,
    mf_ffname: *mut c_char,
    mf_fd: c_int,
    mf_flags: c_int,
    mf_reopen: bool,
    mf_free_first: *mut c_void,
    mf_hash: Map<i64, ptr_t>,
    mf_trans: Map<i64, i64>,
    mf_blocknr_max: i64,
    mf_blocknr_min: i64,
    mf_neg_count: i64,
    mf_infile_count: i64,
    mf_page_size: c_uint,
    mf_dirty: c_int,
}

#[repr(C)]
struct bufmatch_T {
    buf: *mut buf_T,
    match_: *mut c_char,
}

static mut buf_free_count: c_int = 0;
static mut top_file_num: c_int = 1;
static mut no_position: fmark_T = fmark_T {
    mark: pos_T {
        lnum: 1,
        col: 0,
        coladd: 0,
    },
    fnum: 0,
    timestamp: 0,
    view: INIT_FMARKV,
    additional_data: ptr::null_mut(),
};
static mut lasttitle: *mut c_char = ptr::null_mut();
static mut lasticon: *mut c_char = ptr::null_mut();
static mut modelines_entered: c_int = 0;

const E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR: &CStr =
    c"E937: Attempt to delete a buffer that is in use: %s";

#[inline]
unsafe fn buf_is_changed(buf: *const buf_T) -> bool {
    unsafe {
        !bt_dontwrite(buf) && ((*buf).b_changed != 0 || file_ff_differs(buf.cast_mut(), true))
    }
}

#[inline]
unsafe fn curbuf_is_changed() -> bool {
    #[cfg(not(test))]
    unsafe {
        buf_is_changed(curbuf)
    }
    #[cfg(test)]
    {
        false
    }
}

#[inline]
pub unsafe fn buf_get_changedtick(buf: *const buf_T) -> varnumber_T {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

#[inline]
pub unsafe fn buf_meta_total(b: *const buf_T, m: usize) -> u32 {
    unsafe { (*b).b_marktree[0].meta_root[m] }
}

#[inline]
pub unsafe fn first_win_in_tab(tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        if tp == curtab {
            firstwin
        } else {
            (*tp).tp_firstwin
        }
    }
}

#[inline]
pub unsafe fn modifiable(buf: *const buf_T) -> bool {
    unsafe { (*buf).b_p_ma != 0 && (*buf).b_ro_locked == 0 }
}

#[inline]
unsafe fn strcpy_raw(mut dst: *mut c_char, mut src: *const c_char) {
    unsafe {
        loop {
            let c = *src;
            *dst = c;
            if c == NUL {
                break;
            }
            dst = dst.add(1);
            src = src.add(1);
        }
    }
}

#[inline]
unsafe fn reset_binding(wp: *mut win_T) {
    unsafe {
        (*wp).w_onebuf_opt.wo_scb = 0;
        (*wp).w_onebuf_opt.wo_crb = 0;
    }
}

unsafe fn trigger_undo_ftplugin(buf: *mut buf_T, win: *mut win_T) {
    unsafe {
        window_layout_lock();
        (*buf).b_locked += 1;
        (*win).w_locked += 1;
        do_cmdline_cmd(c"if exists('b:undo_ftplugin') | exe b:undo_ftplugin | endif".as_ptr());
        (*buf).b_locked -= 1;
        (*win).w_locked -= 1;
        window_layout_unlock();
    }
}

/// Calculate the percentage that `part` is of the `whole`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn calc_percentage(part: i64, whole: i64) -> c_int {
    if part > 1_000_000 {
        (part / (whole / 100)) as c_int
    } else {
        ((part * 100) / whole) as c_int
    }
}

/// Return the highest possible buffer number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_highest_fnum() -> c_int {
    unsafe { top_file_num - 1 }
}

/// Read data from buffer for retrying.
unsafe fn read_buffer(read_stdin: bool, eap: *mut exarg_T, flags: c_int) -> c_int {
    unsafe {
        let silent = shortmess(kShmFileinfo);

        let mut line_count = (*curbuf).b_ml.ml_line_count;
        let mut retval = readfile(
            if read_stdin {
                ptr::null_mut()
            } else {
                (*curbuf).b_ffname
            },
            if read_stdin {
                ptr::null_mut()
            } else {
                (*curbuf).b_fname
            },
            line_count,
            0,
            MAXLNUM,
            eap,
            flags | READ_BUFFER,
            silent,
        );
        if retval == OK {
            while line_count > 0 {
                line_count -= 1;
                ml_delete(1);
            }
        } else {
            while (*curbuf).b_ml.ml_line_count > line_count {
                ml_delete(line_count);
            }
        }
        (*curwin).w_cursor.lnum = 1;
        (*curwin).w_cursor.col = 0;

        if read_stdin {
            if !readonlymode && !buf_is_empty(curbuf) {
                changed(curbuf);
            } else if retval != FAIL {
                unchanged(curbuf, false, true);
            }

            apply_autocmds_retval(
                EVENT_STDINREADPOST,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
                &raw mut retval,
            );
        }
        retval
    }
}

/// Ensure buffer `buf` is loaded.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_ensure_loaded(buf: *mut buf_T) -> bool {
    unsafe {
        if !(*buf).b_ml.ml_mfp.is_null() {
            return true;
        }

        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);
        let mut aco: CtxSwitch = std::mem::zeroed();
        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        let status = open_buffer(false, ptr::null_mut(), 0);
        ctx_restore(&raw mut aco);
        status != FAIL && bufref_valid(&raw mut bufref) && !(*buf).b_ml.ml_mfp.is_null()
    }
}

/// Open current buffer, that is: open the memfile and read the file into
/// memory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn open_buffer(
    read_stdin: bool,
    eap: *mut exarg_T,
    flags_arg: c_int,
) -> c_int {
    unsafe {
        let mut flags = flags_arg;
        let mut retval = OK;
        let mut old_curbuf: bufref_T = std::mem::zeroed();
        let old_tw = (*curbuf).b_p_tw;
        #[cfg(unix)]
        let mut read_fifo = false;
        #[cfg(not(unix))]
        let read_fifo = false;
        let silent = shortmess(kShmFileinfo);

        if readonlymode
            && !(*curbuf).b_ffname.is_null()
            && ((*curbuf).b_flags & BF_NEVERLOADED) != 0
        {
            (*curbuf).b_p_ro = 1;
        }

        if ml_open(curbuf) == FAIL {
            close_buffer(curwin, curbuf, 0, false, false, false);

            curbuf = ptr::null_mut();
            let mut buf = firstbuf;
            while !buf.is_null() {
                if !(*buf).b_ml.ml_mfp.is_null() {
                    curbuf = buf;
                    break;
                }
                buf = (*buf).b_next;
            }

            if curbuf.is_null() {
                emsg(gettext(
                    c"E82: Cannot allocate any buffer, exiting...".as_ptr(),
                ));
                v_dying = 2;
                getout(2);
            }

            emsg(gettext(
                c"E83: Cannot allocate buffer, using other one...".as_ptr(),
            ));
            enter_buffer(curbuf);
            if old_tw != (*curbuf).b_p_tw {
                check_colorcolumn(ptr::null_mut(), curwin);
            }
            return FAIL;
        }

        if !(*curbuf).b_ml.ml_mfp.is_null() {
            (*(*curbuf).b_ml.ml_mfp.cast::<memfile_head_T>()).mf_dirty = MF_DIRTY_YES_NOSYNC;
        }

        set_bufref(&raw mut old_curbuf, curbuf);
        (*curbuf).b_modified_was_set = false;

        (*curwin).w_valid = 0;

        if bt_nofileread(curbuf) {
            flags |= READ_NOFILE;
        }

        if !(*curbuf).b_ffname.is_null() {
            #[cfg(unix)]
            let save_bin = (*curbuf).b_p_bin;
            #[cfg(unix)]
            {
                const S_IFMT: i32 = 0o170000;
                const S_IFIFO: i32 = 0o010000;
                const S_IFSOCK: i32 = 0o140000;
                let perm = os_getperm((*curbuf).b_ffname);
                if perm >= 0 {
                    let fmt = perm & S_IFMT;
                    #[cfg(any(
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                        target_os = "dragonfly",
                        target_os = "macos"
                    ))]
                    let is_chr_fd = fmt == 0o020000 && is_dev_fd_file((*curbuf).b_ffname);
                    #[cfg(not(any(
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                        target_os = "dragonfly",
                        target_os = "macos"
                    )))]
                    let is_chr_fd = false;

                    if fmt == S_IFIFO || fmt == S_IFSOCK || is_chr_fd {
                        read_fifo = true;
                    }
                }
                if read_fifo {
                    (*curbuf).b_p_bin = 1;
                }
            }

            retval = readfile(
                (*curbuf).b_ffname,
                (*curbuf).b_fname,
                0,
                0,
                MAXLNUM,
                eap,
                flags | READ_NEW | if read_fifo { READ_FIFO } else { 0 },
                silent,
            );
            #[cfg(unix)]
            if read_fifo {
                (*curbuf).b_p_bin = save_bin;
                if retval == OK {
                    retval = read_buffer(false, eap, flags);
                }
            }

            if bt_help(curbuf) {
                get_local_additions();
            }
        } else if read_stdin {
            let save_bin = (*curbuf).b_p_bin;

            (*curbuf).b_p_bin = 1;
            retval = readfile(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                ptr::null_mut(),
                flags | (READ_NEW + READ_STDIN),
                silent,
            );
            (*curbuf).b_p_bin = save_bin;
            if retval == OK {
                retval = read_buffer(true, eap, flags);
            }
        }

        if !(*curbuf).b_ml.ml_mfp.is_null() {
            let mfp = (*curbuf).b_ml.ml_mfp.cast::<memfile_head_T>();
            if (*mfp).mf_dirty == MF_DIRTY_YES_NOSYNC {
                (*mfp).mf_dirty = MF_DIRTY_YES;
            }
        }

        if ((*curbuf).b_flags & BF_NEVERLOADED) != 0 {
            buf_init_isk_chartab(curbuf);
            parse_cino(curbuf);
        }

        if (got_int && !vim_strchr(p_cpo, kCpoIntmod).is_null())
            || (*curbuf).b_modified_was_set
            || (aborting() && !vim_strchr(p_cpo, kCpoIntmod).is_null())
        {
            changed(curbuf);
        } else if retval != FAIL && !read_stdin && !read_fifo {
            unchanged(curbuf, false, true);
        }
        save_file_ff(curbuf);

        let tick = buf_get_changedtick(curbuf);
        (*curbuf).b_last_changedtick = tick;
        (*curbuf).b_last_changedtick_i = tick;
        (*curbuf).b_last_changedtick_pum = tick;

        if aborting() {
            (*curbuf).b_flags |= BF_READERR;
        }

        if retval == NOTDONE
            && *(*curbuf).b_p_ft == NUL
            && !(*curbuf).b_ffname.is_null()
            && after_pathsep(
                (*curbuf).b_ffname,
                (*curbuf).b_ffname.add(strlen((*curbuf).b_ffname)),
            ) != 0
            && augroup_exists(c"filetypedetect".as_ptr())
        {
            let mut bufref: bufref_T = std::mem::zeroed();
            set_bufref(&raw mut bufref, curbuf);
            do_doautocmd(
                c"filetypedetect BufRead".as_ptr().cast_mut(),
                false,
                ptr::null_mut(),
            );
            if !bufref_valid(&raw mut bufref) || curbuf != old_curbuf.br_buf || aborting() {
                return FAIL;
            }
        }

        foldUpdateAll(curwin);

        if ((*curwin).w_valid & VALID_TOPLINE) == 0 {
            (*curwin).w_topline = 1;
            (*curwin).w_topfill = 0;
        }
        apply_autocmds_retval(
            EVENT_BUFENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
            &raw mut retval,
        );

        if retval == FAIL {
            return retval;
        }

        if bufref_valid(&raw mut old_curbuf) && !(*old_curbuf.br_buf).b_ml.ml_mfp.is_null() {
            let mut aco: CtxSwitch = std::mem::zeroed();

            ctx_switch(
                &raw mut aco,
                ptr::null_mut(),
                ptr::null_mut(),
                old_curbuf.br_buf,
                0,
            );
            do_modelines(0);
            (*curbuf).b_flags &= !(BF_CHECK_RO | BF_NEVERLOADED);

            if (flags & READ_NOWINENTER) == 0 {
                apply_autocmds_retval(
                    EVENT_BUFWINENTER,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    curbuf,
                    &raw mut retval,
                );
            }

            ctx_restore(&raw mut aco);
        }

        retval
    }
}

/// Store `buf` in `bufref` and set the free count.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_bufref(bufref: *mut bufref_T, buf: *mut buf_T) {
    unsafe {
        (*bufref).br_buf = buf;
        (*bufref).br_fnum = if buf.is_null() { 0 } else { (*buf).b_fnum };
        (*bufref).br_buf_free_count = buf_free_count;
    }
}

/// Return true if `bufref->br_buf` points to the same buffer as when
/// `set_bufref()` was called and it is a valid buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bufref_valid(bufref: *mut bufref_T) -> bool {
    unsafe {
        if (*bufref).br_buf_free_count == buf_free_count {
            true
        } else {
            buf_valid((*bufref).br_buf) && (*bufref).br_fnum == (*(*bufref).br_buf).b_fnum
        }
    }
}

/// Check that `buf` points to a valid buffer in the buffer list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_valid(buf: *mut buf_T) -> bool {
    if buf.is_null() {
        return false;
    }
    #[cfg(not(test))]
    unsafe {
        let mut bp = lastbuf;
        while !bp.is_null() {
            if bp == buf {
                return true;
            }
            bp = (*bp).b_prev;
        }
    }
    false
}

unsafe fn can_unload_buffer(buf: *mut buf_T) -> bool {
    unsafe {
        let mut can_unload = (*buf).b_locked == 0;

        if can_unload && updating_screen {
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    can_unload = false;
                    break;
                }
                wp = (*wp).w_next;
            }
        }
        if can_unload && (*buf).b_saving {
            can_unload = false;
        }

        if !can_unload {
            let fname = if !(*buf).b_fname.is_null() {
                (*buf).b_fname
            } else {
                (*buf).b_ffname
            };
            semsg!(
                gettext(E_ATTEMPT_TO_DELETE_BUFFER_THAT_IS_IN_USE_STR.as_ptr()),
                if !fname.is_null() {
                    fname
                } else {
                    c"[No Name]".as_ptr()
                },
            );
        }
        can_unload
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_close_terminal(buf: *mut buf_T) {
    unsafe {
        debug_assert!(!(*buf).terminal.is_null());
        (*buf).b_locked += 1;
        terminal_close((&raw mut (*buf).terminal).cast::<*mut Terminal>(), -1);
        (*buf).b_locked -= 1;
    }
}

/// Close the link to a buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn close_buffer(
    win: *mut win_T,
    buf: *mut buf_T,
    action: c_int,
    abort_if_last: bool,
    ignore_abort: bool,
    set_context: bool,
) -> bool {
    unsafe {
        let mut unload_buf = action != 0;
        let mut del_buf = action == DOBUF_DEL || action == DOBUF_WIPE;
        let mut wipe_buf = action == DOBUF_WIPE;

        let is_curwin = !curwin.is_null() && curwin == win && (*curwin).w_buffer == buf;
        let the_curtab = curtab;

        if (*buf).terminal.is_null() {
            let bh0 = *(*buf).b_p_bh;
            if bh0 == b'd' as c_char {
                del_buf = true;
                unload_buf = true;
            } else if bh0 == b'w' as c_char {
                del_buf = true;
                unload_buf = true;
                wipe_buf = true;
            } else if bh0 == b'u' as c_char {
                unload_buf = true;
            }
        }

        if !(*buf).terminal.is_null() && (unload_buf || del_buf || wipe_buf) {
            unload_buf = true;
            del_buf = true;
            wipe_buf = true;
        }

        if (del_buf || wipe_buf) && !can_unload_buffer(buf) {
            return false;
        }

        let mut win_valid = win_valid_any_tab(win);
        if set_context && win_valid && (*win).w_buffer == buf {
            if (*buf).b_nwindows == 1 {
                set_last_cursor(win);
            }
            buflist_setfpos(
                buf,
                win,
                if (*win).w_cursor.lnum == 1 {
                    0
                } else {
                    (*win).w_cursor.lnum
                },
                (*win).w_cursor.col,
                true,
            );
        }

        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);

        if win_valid && (*win).w_buffer == buf && (*buf).b_nwindows == 1 {
            (*buf).b_locked += 1;
            (*buf).b_locked_split += 1;
            if apply_autocmds_win(
                EVENT_BUFWINLEAVE,
                (*buf).b_fname,
                (*buf).b_fname,
                false,
                buf,
                win,
            ) && !bufref_valid(&raw mut bufref)
            {
                emsg(gettext(e_auabort.as_ptr()));
                return false;
            }
            (*buf).b_locked -= 1;
            (*buf).b_locked_split -= 1;
            if abort_if_last && one_window(win, ptr::null_mut()) {
                emsg(gettext(e_auabort.as_ptr()));
                return false;
            }

            if !unload_buf {
                (*buf).b_locked += 1;
                (*buf).b_locked_split += 1;
                if apply_autocmds(EVENT_BUFHIDDEN, (*buf).b_fname, (*buf).b_fname, false, buf)
                    && !bufref_valid(&raw mut bufref)
                {
                    emsg(gettext(e_auabort.as_ptr()));
                    return false;
                }
                (*buf).b_locked -= 1;
                (*buf).b_locked_split -= 1;
                if abort_if_last && one_window(win, ptr::null_mut()) {
                    emsg(gettext(e_auabort.as_ptr()));
                    return false;
                }
            }
            if !ignore_abort && aborting() {
                return false;
            }
            win_valid = win_valid && win_valid_any_tab(win);
        }

        if is_curwin && curwin != win && win_valid {
            block_autocmds();
            goto_tabpage_win(the_curtab, win);
            unblock_autocmds();
        }

        if bt_quickfix(buf) && win_valid && (*win).w_buffer == buf {
            (*win).w_onebuf_opt.wo_wfh = 0;
        }

        let hiding_buf = (*buf).b_nwindows <= 0
            || (win_valid && (*win).w_buffer == buf && (*buf).b_nwindows == 1);

        if diffopt_hiddenoff() && !unload_buf && hiding_buf {
            diff_buf_delete(buf);
        }

        if !hiding_buf || !unload_buf {
            return false;
        }

        if (*buf).b_ffname.is_null() {
            del_buf = true;
        }

        if !buf_freeall(
            buf,
            (if del_buf { BFA_DEL } else { 0 })
                + (if wipe_buf { BFA_WIPE } else { 0 })
                + (if ignore_abort { BFA_IGNORE_ABORT } else { 0 }),
        ) {
            return false;
        }

        let mut clear_w_buf = false;
        win_valid = win_valid && win_valid_any_tab(win);
        if win_valid && (*win).w_buffer == buf {
            (*buf).b_nwindows -= 1;
            clear_w_buf = true;
        }

        if wipe_buf
            && (*buf).b_nwindows <= 0
            && (!(*buf).b_prev.is_null() || !(*buf).b_next.is_null() || entered_free_all_mem)
        {
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    mark_forget_file(wp, (*buf).b_fnum);
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
            if (*buf).b_sfname != (*buf).b_ffname {
                xfree((*buf).b_sfname.cast());
            }
            (*buf).b_sfname = ptr::null_mut();
            xfree((*buf).b_ffname.cast());
            (*buf).b_ffname = ptr::null_mut();
            if (*buf).b_prev.is_null() {
                firstbuf = (*buf).b_next;
            } else {
                (*(*buf).b_prev).b_next = (*buf).b_next;
            }
            if (*buf).b_next.is_null() {
                lastbuf = (*buf).b_prev;
            } else {
                (*(*buf).b_next).b_prev = (*buf).b_prev;
            }
            free_buffer(buf);
        } else {
            if del_buf {
                free_buffer_stuff(buf, kBffClearWinInfo | kBffInitChangedtick);
                (*buf).b_flags = BF_CHECK_RO | BF_NEVERLOADED;
                (*buf).b_p_initialized = false;
            }
            textlock += 1;
            buf_clear_file(buf);
            textlock -= 1;
            if clear_w_buf {
                (*win).w_buffer = ptr::null_mut();
            }
            if del_buf {
                (*buf).b_p_bl = 0;
            }
        }

        debug_assert!(!win_valid || !is_curwin || win == curwin);
        true
    }
}

/// Make buffer not contain a file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_clear_file(buf: *mut buf_T) {
    unsafe {
        (*buf).b_ml.ml_line_count = 1;
        unchanged(buf, true, true);
        (*buf).b_p_eof = 0;
        (*buf).b_start_eof = 0;
        (*buf).b_p_eol = 1;
        (*buf).b_start_eol = 1;
        (*buf).b_p_bomb = 0;
        (*buf).b_start_bomb = 0;
        (*buf).b_ml.ml_mfp = ptr::null_mut();
        (*buf).b_ml.ml_flags = ML_EMPTY;
    }
}

/// Clears the current buffer contents.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_clear() {
    unsafe {
        let line_count = (*curbuf).b_ml.ml_line_count;
        extmark_free_all(curbuf);
        mc_buf_free(curbuf);
        while ((*curbuf).b_ml.ml_flags & ML_EMPTY) == 0 {
            ml_delete(1);
        }
        deleted_lines_mark(1, line_count);
    }
}

/// Free all things allocated for a buffer that are related to the file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_freeall(buf: *mut buf_T, flags: c_int) -> bool {
    unsafe {
        let is_curbuf = buf == curbuf;
        let is_curwin = !curwin.is_null() && (*curwin).w_buffer == buf;
        let the_curwin = curwin;
        let the_curtab = curtab;

        (*buf).b_locked += 1;
        (*buf).b_locked_split += 1;

        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);

        if !(*buf).terminal.is_null() {
            buf_close_terminal(buf);
        }

        buf_updates_unload(buf, false);

        if !(*buf).b_ml.ml_mfp.is_null()
            && apply_autocmds(EVENT_BUFUNLOAD, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(&raw mut bufref)
        {
            return false;
        }
        if (flags & BFA_DEL) != 0
            && (*buf).b_p_bl != 0
            && apply_autocmds(EVENT_BUFDELETE, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(&raw mut bufref)
        {
            return false;
        }
        if (flags & BFA_WIPE) != 0
            && apply_autocmds(EVENT_BUFWIPEOUT, (*buf).b_fname, (*buf).b_fname, false, buf)
            && !bufref_valid(&raw mut bufref)
        {
            return false;
        }
        (*buf).b_locked -= 1;
        (*buf).b_locked_split -= 1;

        if is_curwin && curwin != the_curwin && win_valid_any_tab(the_curwin) {
            block_autocmds();
            goto_tabpage_win(the_curtab, the_curwin);
            unblock_autocmds();
        }
        if (flags & BFA_IGNORE_ABORT) == 0 && aborting() {
            return false;
        }

        if buf == curbuf && !is_curbuf {
            return false;
        }

        if buf == curbuf && Visual.active && !entered_free_all_mem {
            end_visual_mode();
        }

        diff_buf_delete(buf);
        if !curwin.is_null() && (*curwin).w_buffer == buf {
            reset_synblock(curwin);
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut win = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !win.is_null() {
                if (*win).w_buffer == buf {
                    clearFolding(win);
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }

        if !(*buf).terminal.is_null() {
            block_autocmds();
            buf_close_terminal(buf);
            unblock_autocmds();
        }

        let count = (*buf).b_ml.ml_line_count;
        ml_close(buf, 1);
        (*buf).b_ml.ml_line_count = 0;
        mc_buf_clear(buf);

        if bt_nofilename(buf) && !exiting {
            mark_adjust_buf(
                buf,
                1,
                count,
                MAXLNUM,
                -count,
                false,
                kMarkAdjustNormal,
                kExtmarkNoUndo,
            );
        }

        if (flags & BFA_KEEP_UNDO) == 0 {
            u_clearallandblockfree(buf);
        }
        syntax_clear(&raw mut (*buf).b_s);
        (*buf).b_flags &= !BF_READERR;
        true
    }
}

unsafe fn free_buffer(buf: *mut buf_T) {
    unsafe {
        map_del_int_ptr_t(&raw mut buffer_handles, (*buf).b_fnum, ptr::null_mut());
        buf_free_count += 1;
        free_buffer_stuff(buf, kBffClearWinInfo);
        if (*(*buf).b_vars).dv_refcount > DO_NOT_FREE_CNT {
            tv_dict_add(
                (*buf).b_vars,
                tv_dict_item_copy((&raw mut (*buf).changedtick_di).cast()),
            );
        }
        unref_var_dict((*buf).b_vars);
        aubuflocal_remove(buf);
        xfree((*buf).additional_data);
        xfree((*buf).b_prompt_text.cast());
        (*buf).b_wininfo.destroy();
        callback_free(&raw mut (*buf).b_prompt_callback);
        callback_free(&raw mut (*buf).b_prompt_interrupt);
        clear_fmark(&raw mut (*buf).b_last_cursor, 0);
        clear_fmark(&raw mut (*buf).b_last_insert, 0);
        clear_fmark(&raw mut (*buf).b_last_change, 0);
        clear_fmark(&raw mut (*buf).b_prompt_start, 0);
        let namedm_ptr = (&raw mut (*buf).b_namedm).cast::<fmark_T>();
        for i in 0..NMARKS {
            free_fmark(*namedm_ptr.add(i));
        }
        let changelist_ptr = (&raw mut (*buf).b_changelist).cast::<fmark_T>();
        for i in 0..(*buf).b_changelistlen as usize {
            free_fmark(*changelist_ptr.add(i));
        }
        if autocmd_busy {
            ptr::write_bytes(namedm_ptr, 0, NMARKS);
            ptr::write_bytes(changelist_ptr, 0, JUMPLISTSIZE);
            (*buf).b_next = au_pending_free_buf;
            au_pending_free_buf = buf;
        } else {
            xfree(buf.cast());
            if curbuf == buf {
                curbuf = ptr::null_mut();
            }
        }
    }
}

unsafe fn clear_wininfo(buf: *mut buf_T) {
    unsafe {
        for i in 0..(*buf).b_wininfo.size {
            let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
            free_wininfo(wip, buf);
        }
        (*buf).b_wininfo.size = 0;
    }
}

unsafe fn free_buffer_stuff(buf: *mut buf_T, free_flags: c_int) {
    unsafe {
        if (free_flags & kBffClearWinInfo) != 0 {
            clear_wininfo(buf);
            free_buf_options(buf, true);
            ga_clear(&raw mut (*buf).b_s.b_langp);
        }
        {
            let changedtick_hi = hash_find(
                &raw const (*(*buf).b_vars).dv_hashtab,
                c"changedtick".as_ptr(),
            );
            debug_assert!(!changedtick_hi.is_null());
            hash_remove(&raw mut (*(*buf).b_vars).dv_hashtab, changedtick_hi);
        }
        vars_clear(&raw mut (*(*buf).b_vars).dv_hashtab);
        hash_init(&raw mut (*(*buf).b_vars).dv_hashtab);
        if (free_flags & kBffInitChangedtick) != 0 {
            buf_init_changedtick(buf);
        }
        uc_clear(&raw mut (*buf).b_ucmds);
        extmark_free_all(buf);
        mc_buf_free(buf);
        map_clear_mode(buf, MAP_ALL_MODES, true, false);
        map_clear_mode(buf, MAP_ALL_MODES, true, true);
        xfree((*buf).b_start_fenc.cast());
        (*buf).b_start_fenc = ptr::null_mut();
        xfree((*buf).b_localdir.cast());
        (*buf).b_localdir = ptr::null_mut();
        xfree((*buf).b_prevdir.cast());
        (*buf).b_prevdir = ptr::null_mut();

        buf_free_callbacks(buf);
    }
}

/// Go to another buffer. Handles the result of the ATTENTION dialog.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn goto_buffer(eap: *mut exarg_T, start: c_int, dir: c_int, count: c_int) {
    unsafe {
        let save_sea = swap_exists_action;
        let skip_help_buf = matches!(
            std::mem::transmute::<c_int, cmdidx_T>((*eap).cmdidx),
            cmdidx_T::CMD_bnext
                | cmdidx_T::CMD_sbnext
                | cmdidx_T::CMD_bNext
                | cmdidx_T::CMD_bprevious
                | cmdidx_T::CMD_sbNext
                | cmdidx_T::CMD_sbprevious
        );

        let mut old_curbuf: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut old_curbuf, curbuf);

        if swap_exists_action == SEA_NONE {
            swap_exists_action = SEA_DIALOG;
        }
        let _ = do_buffer_ext(
            if *(*eap).cmd == b's' as c_char {
                DOBUF_SPLIT
            } else {
                DOBUF_GOTO
            },
            start,
            dir,
            count,
            (if (*eap).forceit != 0 {
                DOBUF_FORCEIT
            } else {
                0
            }) | (if skip_help_buf { DOBUF_SKIPHELP } else { 0 }),
        );

        if swap_exists_action == SEA_QUIT && *(*eap).cmd == b's' as c_char {
            let mut cs: cleanup_T = std::mem::zeroed();
            enter_cleanup(&raw mut cs);
            win_close(curwin, true, false);
            swap_exists_action = save_sea;
            swap_exists_did_quit = true;
            leave_cleanup(&raw mut cs);
        } else {
            handle_swap_exists(&raw mut old_curbuf);
        }
    }
}

/// Handle the situation of `swap_exists_action` being set.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn handle_swap_exists(old_curbuf: *mut bufref_T) {
    unsafe {
        let mut cs: cleanup_T = std::mem::zeroed();
        let old_tw = (*curbuf).b_p_tw;

        if swap_exists_action == SEA_QUIT {
            enter_cleanup(&raw mut cs);

            swap_exists_action = SEA_NONE;
            swap_exists_did_quit = true;
            close_buffer(curwin, curbuf, DOBUF_UNLOAD, false, false, true);
            let buf = if old_curbuf.is_null()
                || !bufref_valid(old_curbuf)
                || (*old_curbuf).br_buf == curbuf
            {
                block_autocmds();
                let b = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_CURBUF | BLN_LISTED);
                unblock_autocmds();
                b
            } else {
                (*old_curbuf).br_buf
            };
            if !buf.is_null() {
                enter_buffer(buf);
                if old_tw != (*curbuf).b_p_tw {
                    check_colorcolumn(ptr::null_mut(), curwin);
                }
            }

            leave_cleanup(&raw mut cs);
        } else if swap_exists_action == SEA_RECOVER {
            enter_cleanup(&raw mut cs);

            msg_scroll = 1;
            ml_recover(false);
            msg_puts(c"\n".as_ptr());
            cmdline_row = msg_row;
            do_modelines(0);

            leave_cleanup(&raw mut cs);
        }
        swap_exists_action = SEA_NONE;
    }
}

/// Delete or unload buffer(s).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_bufdel(
    command: c_int,
    mut arg: *mut c_char,
    addr_count: c_int,
    start_bnr: c_int,
    end_bnr: c_int,
    forceit: c_int,
) -> *mut c_char {
    unsafe {
        let mut do_current = 0;
        let mut deleted: c_int = 0;
        let mut errormsg: *mut c_char = ptr::null_mut();
        let mut bnr: c_int;

        if addr_count == 0 {
            do_buffer(command, DOBUF_CURRENT, FORWARD, 0, forceit);
        } else {
            if addr_count == 2 {
                if *arg != NUL {
                    return ex_errmsg(e_trailing_arg.as_ptr(), arg);
                }
                bnr = start_bnr;
            } else {
                bnr = end_bnr;
            }

            while !got_int {
                if bnr == (*curbuf).b_fnum {
                    do_current = bnr;
                } else if do_buffer(command, DOBUF_FIRST, FORWARD, bnr, forceit) == OK {
                    deleted += 1;
                }

                if addr_count == 2 {
                    bnr += 1;
                    if bnr > end_bnr {
                        break;
                    }
                } else {
                    arg = skipwhite(arg);
                    if *arg == NUL {
                        break;
                    }
                    if !ascii_isdigit(*arg as c_int) {
                        let p = skiptowhite_esc(arg);
                        bnr = buflist_findpat(arg, p, command == DOBUF_WIPE, false, false);
                        if bnr < 0 {
                            break;
                        }
                        arg = p;
                    } else {
                        bnr = getdigits_int(&raw mut arg, false, 0);
                    }
                }
                os_breakcheck();
            }
            if !got_int
                && do_current != 0
                && do_buffer(command, DOBUF_FIRST, FORWARD, do_current, forceit) == OK
            {
                deleted += 1;
            }

            if deleted == 0 {
                let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
                if command == DOBUF_UNLOAD {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E515: No buffers were unloaded".as_ptr()),
                        IOSIZE,
                    );
                } else if command == DOBUF_DEL {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E516: No buffers were deleted".as_ptr()),
                        IOSIZE,
                    );
                } else {
                    xstrlcpy(
                        iobuff_ptr,
                        gettext(c"E517: No buffers were wiped out".as_ptr()),
                        IOSIZE,
                    );
                }
                errormsg = iobuff_ptr;
            } else if i64::from(deleted) >= p_report {
                if command == DOBUF_UNLOAD {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer unloaded".as_ptr(),
                            c"%d buffers unloaded".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                } else if command == DOBUF_DEL {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer deleted".as_ptr(),
                            c"%d buffers deleted".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                } else {
                    smsg!(
                        0,
                        ngettext(
                            c"%d buffer wiped out".as_ptr(),
                            c"%d buffers wiped out".as_ptr(),
                            deleted as usize,
                        ),
                        deleted,
                    );
                }
            }
        }

        errormsg
    }
}

unsafe fn empty_curbuf(close_others: bool, forceit: c_int, action: c_int) -> c_int {
    unsafe {
        let buf = curbuf;

        if action == DOBUF_UNLOAD {
            emsg(gettext(c"E90: Cannot unload last buffer".as_ptr()));
            return FAIL;
        }

        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);

        if close_others {
            let mut can_close_all_others = true;
            if (*curwin).w_floating {
                can_close_all_others = false;
                let mut wp = firstwin;
                while !(*wp).w_floating {
                    if (*wp).w_buffer != curbuf {
                        can_close_all_others = true;
                        break;
                    }
                    wp = (*wp).w_next;
                }
            }
            close_windows(buf, can_close_all_others);
        }

        setpcmark();
        let retval = do_ecmd(
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ECMD_ONE,
            if forceit != 0 { ECMD_FORCEIT } else { 0 },
            curwin,
        );

        if buf != curbuf && bufref_valid(&raw mut bufref) && (*buf).b_nwindows == 0 {
            close_buffer(ptr::null_mut(), buf, action, false, false, false);
        }

        if !close_others {
            need_fileinfo = false;
        } else if retval == OK && !shortmess(kShmFileinfo) {
            need_fileinfo = true;
        }

        retval
    }
}

unsafe fn do_buffer_ext(
    action: c_int,
    start: c_int,
    dir: c_int,
    mut count: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let mut buf: *mut buf_T;
        let mut bp: *mut buf_T;
        let mut update_jumplist = true;
        let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;

        buf = match start {
            DOBUF_FIRST => firstbuf,
            DOBUF_LAST => lastbuf,
            _ => curbuf,
        };
        if start == DOBUF_MOD {
            while count > 0 {
                count -= 1;
                loop {
                    buf = (*buf).b_next;
                    if buf.is_null() {
                        buf = firstbuf;
                    }
                    if buf == curbuf || buf_is_changed(buf) {
                        break;
                    }
                }
            }
            if !buf_is_changed(buf) {
                emsg(gettext(c"E84: No modified buffer found".as_ptr()));
                return FAIL;
            }
        } else if start == DOBUF_FIRST && count != 0 {
            while !buf.is_null() && (*buf).b_fnum != count {
                buf = (*buf).b_next;
            }
        } else {
            let help_only = (flags & DOBUF_SKIPHELP) != 0 && (*buf).b_help;

            bp = ptr::null_mut();
            while count > 0
                || (bp != buf
                    && !unload
                    && !(if help_only {
                        (*buf).b_help
                    } else {
                        (*buf).b_p_bl != 0
                    }))
            {
                if bp.is_null() {
                    bp = buf;
                }
                buf = if dir == FORWARD {
                    if !(*buf).b_next.is_null() {
                        (*buf).b_next
                    } else {
                        firstbuf
                    }
                } else if !(*buf).b_prev.is_null() {
                    (*buf).b_prev
                } else {
                    lastbuf
                };
                if unload
                    || (if help_only {
                        (*buf).b_help
                    } else {
                        (*buf).b_p_bl != 0 && ((flags & DOBUF_SKIPHELP) == 0 || !(*buf).b_help)
                    })
                {
                    count -= 1;
                    bp = ptr::null_mut();
                }
                if bp == buf {
                    emsg(gettext(c"E85: There is no listed buffer".as_ptr()));
                    return FAIL;
                }
            }
        }

        if buf.is_null() {
            if start == DOBUF_FIRST {
                if !unload {
                    semsg!(gettext(e_nobufnr.as_ptr()), i64::from(count));
                }
            } else if dir == FORWARD {
                emsg(gettext(c"E87: Cannot go beyond last buffer".as_ptr()));
            } else {
                emsg(gettext(c"E88: Cannot go before first buffer".as_ptr()));
            }
            return FAIL;
        }

        if action == DOBUF_GOTO
            && buf != curbuf
            && !check_can_set_curbuf_forceit(flags & DOBUF_FORCEIT)
        {
            return FAIL;
        }

        if (action == DOBUF_GOTO || action == DOBUF_SPLIT) && ((*buf).b_flags & BF_DUMMY) != 0 {
            semsg!(gettext(e_nobufnr.as_ptr()), count);
            return FAIL;
        }

        if unload {
            let mut bufref: bufref_T = std::mem::zeroed();
            if !can_unload_buffer(buf) {
                return FAIL;
            }
            set_bufref(&raw mut bufref, buf);

            if action != DOBUF_WIPE && (*buf).b_ml.ml_mfp.is_null() && (*buf).b_p_bl == 0 {
                return FAIL;
            }

            if (flags & DOBUF_FORCEIT) == 0 && buf_is_changed(buf) {
                if (p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && p_write != 0 {
                    dialog_changed(buf, false);
                    if !bufref_valid(&raw mut bufref) {
                        return FAIL;
                    }
                    if buf_is_changed(buf) {
                        return FAIL;
                    }
                } else {
                    semsg!(
                        gettext(
                            e_no_write_since_last_change_for_buffer_nr_add_bang_to_override
                                .as_ptr(),
                        ),
                        (*buf).b_fnum,
                    );
                    return FAIL;
                }
            }

            if (flags & DOBUF_FORCEIT) == 0
                && !(*buf).terminal.is_null()
                && terminal_running((*buf).terminal.cast())
            {
                if p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0 {
                    if !dialog_close_terminal(buf) {
                        return FAIL;
                    }
                } else {
                    semsg!(
                        gettext(c"E89: %s will be killed (add ! to override)".as_ptr()),
                        (*buf).b_fname,
                    );
                    return FAIL;
                }
            }

            let buf_fnum = (*buf).b_fnum;

            if buf == curbuf && Visual.active {
                end_visual_mode();
            }

            bp = ptr::null_mut();
            let mut bp2 = firstbuf;
            while !bp2.is_null() {
                if (*bp2).b_p_bl != 0 && bp2 != buf {
                    bp = bp2;
                    break;
                }
                bp2 = (*bp2).b_next;
            }
            if bp.is_null() && buf == curbuf {
                return empty_curbuf(true, flags & DOBUF_FORCEIT, action);
            }

            while buf == curbuf
                && !(win_locked(curwin) != 0 || (*(*curwin).w_buffer).b_locked > 0)
                && (is_ctx_win(lastwin) || !last_window(curwin))
            {
                if win_close(curwin, false, false) == FAIL {
                    break;
                }
            }

            if buf != curbuf {
                if (jop_flags & kOptJopFlagClean) != 0 {
                    mark_jumplist_forget_file(curwin, buf_fnum);
                }

                close_windows(buf, false);
                if buf != curbuf
                    && bufref_valid(&raw mut bufref)
                    && (*firstwin).w_buffer == buf
                    && one_window(firstwin, ptr::null_mut())
                {
                    debug_assert!((*curwin).w_floating);
                    let mut switchwin: CtxSwitch = std::mem::zeroed();
                    let rv = ctx_switch(
                        &raw mut switchwin,
                        firstwin,
                        curtab,
                        ptr::null_mut(),
                        kCtxNoDisplay,
                    );
                    debug_assert!(rv);
                    do_buffer_ext(action, start, dir, count, flags);
                    ctx_restore(&raw mut switchwin);
                }

                if buf != curbuf && bufref_valid(&raw mut bufref) && (*buf).b_nwindows <= 0 {
                    close_buffer(ptr::null_mut(), buf, action, false, false, false);
                }
                return OK;
            }

            buf = ptr::null_mut();
            bp = ptr::null_mut();
            if !au_new_curbuf.br_buf.is_null()
                && bufref_valid(&raw mut au_new_curbuf)
                && (*au_new_curbuf.br_buf).b_locked_split == 0
            {
                buf = au_new_curbuf.br_buf;
            } else if (*curwin).w_jumplistlen > 0 {
                if (jop_flags & kOptJopFlagClean) != 0 {
                    mark_jumplist_forget_file(curwin, buf_fnum);
                }

                if (*curwin).w_jumplistlen > 0 {
                    let mut jumpidx = (*curwin).w_jumplistidx;

                    if (jop_flags & kOptJopFlagClean) != 0 {
                        if jumpidx == (*curwin).w_jumplistlen {
                            jumpidx = (*curwin).w_jumplistlen - 1;
                            (*curwin).w_jumplistidx = jumpidx;
                        }
                    } else {
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = (*curwin).w_jumplistlen - 1;
                        }
                    }

                    let forward = jumpidx;
                    while (jop_flags & kOptJopFlagClean) != 0 || jumpidx != (*curwin).w_jumplistidx
                    {
                        buf = buflist_findnr((*curwin).w_jumplist[jumpidx as usize].fmark.fnum);

                        if !buf.is_null() {
                            if buf == curbuf
                                || (*buf).b_p_bl == 0
                                || bt_quickfix(buf)
                                || (*buf).b_locked_split != 0
                            {
                                buf = ptr::null_mut();
                            } else if (*buf).b_ml.ml_mfp.is_null() {
                                if bp.is_null() {
                                    bp = buf;
                                }
                                buf = ptr::null_mut();
                            }
                        }
                        if !buf.is_null() {
                            if (jop_flags & kOptJopFlagClean) != 0 {
                                (*curwin).w_jumplistidx = jumpidx;
                                update_jumplist = false;
                            }
                            break;
                        }
                        if jumpidx == 0 && (*curwin).w_jumplistidx == (*curwin).w_jumplistlen {
                            break;
                        }
                        jumpidx -= 1;
                        if jumpidx < 0 {
                            jumpidx = (*curwin).w_jumplistlen - 1;
                        }
                        if jumpidx == forward {
                            break;
                        }
                    }
                }
            }

            if buf.is_null() {
                let mut forward = true;
                buf = (*curbuf).b_next;
                loop {
                    if buf.is_null() {
                        if !forward {
                            break;
                        }
                        buf = (*curbuf).b_prev;
                        forward = false;
                        continue;
                    }
                    if (*buf).b_help == (*curbuf).b_help
                        && (*buf).b_p_bl != 0
                        && !bt_quickfix(buf)
                        && (*buf).b_locked_split == 0
                    {
                        if !(*buf).b_ml.ml_mfp.is_null() {
                            break;
                        }
                        if bp.is_null() {
                            bp = buf;
                        }
                    }
                    buf = if forward {
                        (*buf).b_next
                    } else {
                        (*buf).b_prev
                    };
                }
            }
            if buf.is_null() {
                buf = bp;
            }
            if buf.is_null() {
                let mut buf2 = firstbuf;
                while !buf2.is_null() {
                    if (*buf2).b_p_bl != 0
                        && buf2 != curbuf
                        && !bt_quickfix(buf2)
                        && (*buf2).b_locked_split == 0
                    {
                        buf = buf2;
                        break;
                    }
                    buf2 = (*buf2).b_next;
                }
            }
            if buf.is_null() {
                buf = if !(*curbuf).b_next.is_null() {
                    (*curbuf).b_next
                } else {
                    (*curbuf).b_prev
                };
                if bt_quickfix(buf)
                    || (!buf.is_null() && buf != curbuf && (*buf).b_locked_split != 0)
                {
                    buf = ptr::null_mut();
                }
            }
        }

        if buf.is_null() {
            return empty_curbuf(false, flags & DOBUF_FORCEIT, action);
        }

        if action == DOBUF_SPLIT && !swbuf_goto_win_with_buf(buf).is_null() {
            return OK;
        }
        if buf != curbuf && (*buf).b_locked_split != 0 {
            emsg(gettext(e_cannot_switch_to_a_closing_buffer.as_ptr()));
            return FAIL;
        }
        if action == DOBUF_SPLIT && win_split(0, 0) == FAIL {
            return FAIL;
        }

        if buf == curbuf {
            return OK;
        }

        if action == DOBUF_GOTO && !can_abandon(curbuf, (flags & DOBUF_FORCEIT) != 0) {
            if (p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && p_write != 0 {
                let mut bufref: bufref_T = std::mem::zeroed();
                set_bufref(&raw mut bufref, buf);
                dialog_changed(curbuf, false);
                if !bufref_valid(&raw mut bufref) {
                    return FAIL;
                }
            }
            if buf_is_changed(curbuf) {
                no_write_message();
                return FAIL;
            }
        }

        set_curbuf(buf, action, update_jumplist);

        if action == DOBUF_SPLIT {
            reset_binding(curwin);
        }

        if aborting() {
            return FAIL;
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_buffer(
    action: c_int,
    start: c_int,
    dir: c_int,
    count: c_int,
    forceit: c_int,
) -> c_int {
    unsafe {
        do_buffer_ext(
            action,
            start,
            dir,
            count,
            if forceit != 0 { DOBUF_FORCEIT } else { 0 },
        )
    }
}

/// Set current buffer to `buf`. Executes autocommands and closes current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_curbuf(buf: *mut buf_T, action: c_int, update_jumplist: bool) {
    unsafe {
        let unload = action == DOBUF_UNLOAD || action == DOBUF_DEL || action == DOBUF_WIPE;
        let old_tw = (*curbuf).b_p_tw;

        if update_jumplist {
            setpcmark();
        }

        if (cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
            (*curwin).w_alt_fnum = (*curbuf).b_fnum;
        }
        buflist_altfpos(curwin);

        Visual.reselect = 0;

        let prevbuf = curbuf;
        let mut newbufref: bufref_T = std::mem::zeroed();
        let mut prevbufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut prevbufref, prevbuf);
        set_bufref(&raw mut newbufref, buf);

        if !apply_autocmds(
            EVENT_BUFLEAVE,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        ) || (bufref_valid(&raw mut prevbufref)
            && bufref_valid(&raw mut newbufref)
            && !aborting())
        {
            if prevbuf == (*curwin).w_buffer {
                reset_synblock(curwin);
            }
            if unload {
                close_windows(prevbuf, false);
            }
            if bufref_valid(&raw mut prevbufref) && !aborting() {
                if prevbuf == curbuf && ((State & MODE_INSERT) == 0 || (*curbuf).b_nwindows <= 1) {
                    u_sync(false);
                }
                close_buffer(
                    curwin,
                    prevbuf,
                    if unload {
                        action
                    } else if action == DOBUF_GOTO && !buf_hide(prevbuf) && !buf_is_changed(prevbuf)
                    {
                        DOBUF_UNLOAD
                    } else {
                        0
                    },
                    false,
                    false,
                    true,
                );
            }
        }

        let valid = buf_valid(buf);
        if (valid && buf != curbuf && !aborting()) || (*curwin).w_buffer.is_null() {
            enter_buffer(if valid { buf } else { lastbuf });
            if old_tw != (*curbuf).b_p_tw {
                check_colorcolumn(ptr::null_mut(), curwin);
            }
        }

        if bufref_valid(&raw mut prevbufref) && !(*prevbuf).terminal.is_null() {
            terminal_check_size((*prevbuf).terminal.cast());
        }

        update_cwd(kCdCauseBuffer);
    }
}

/// Enter a new current buffer.
unsafe fn enter_buffer(buf: *mut buf_T) {
    unsafe {
        if Visual.active && !entered_free_all_mem {
            end_visual_mode();
        }

        if !(*curwin).w_buffer.is_null() {
            (*(*curwin).w_buffer).b_nwindows -= 1;
        }

        (*curwin).w_buffer = buf;
        curbuf = buf;
        (*curbuf).b_nwindows += 1;

        buf_copy_options(buf, BCO_ENTER | BCO_NOHELP);
        if !(*buf).b_help {
            get_winopts(buf);
        } else {
            clearFolding(curwin);
        }
        foldUpdateAll(curwin);

        if (*curwin).w_onebuf_opt.wo_diff != 0 {
            diff_buf_add(curbuf);
        }

        (*curwin).w_s = (&raw mut (*curbuf).b_s).cast();

        (*curwin).w_cursor.lnum = 1;
        (*curwin).w_cursor.col = 0;
        (*curwin).w_cursor.coladd = 0;
        (*curwin).w_set_curswant = true;
        (*curwin).w_topline_was_set = false;

        (*curwin).w_valid = 0;

        if (*curbuf).b_ml.ml_mfp.is_null() {
            if *(*curbuf).b_p_ft == NUL {
                (*curbuf).b_did_filetype = false;
            }
            open_buffer(false, ptr::null_mut(), 0);
        } else {
            if msg_silent == 0 && !shortmess(kShmFileinfo) {
                need_fileinfo = true;
            }
            buf_check_timestamp(curbuf);

            (*curwin).w_topline = 1;
            (*curwin).w_topfill = 0;
            apply_autocmds(
                EVENT_BUFENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            apply_autocmds(
                EVENT_BUFWINENTER,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }

        if (*curwin).w_cursor.lnum == 1 && inindent(0) {
            buflist_getfpos();
        }

        check_arg_idx(curwin);
        maketitle();
        win_float_update_preview(curwin);
        if (*curwin).w_topline == 1 && !(*curwin).w_topline_was_set {
            scroll_cursor_halfway(curwin, false, false);
        }

        do_autochdir();

        if ((*curbuf).b_kmap_state & KEYMAP_INIT) != 0 {
            keymap_init();
        }
        if !(*curbuf).b_help
            && (*curwin).w_onebuf_opt.wo_spell != 0
            && *(*(*curwin).w_s.cast::<synblock_T>()).b_p_spl != NUL
        {
            parse_spelllang(curwin);
        }
        (*curbuf).b_last_used = os_time() as i64;

        if !(*curbuf).terminal.is_null() {
            terminal_check_size((*curbuf).terminal.cast());
        }

        redraw_later(curwin, UPD_NOT_VALID);
    }
}

/// Change to the directory of the current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_autochdir() {
    unsafe {
        if p_acd != 0
            && starting == 0
            && !(*curbuf).b_ffname.is_null()
            && vim_chdirfile((*curbuf).b_ffname, kCdCauseAuto) == OK
        {
            last_chdir_reason = c"autochdir".as_ptr().cast_mut();
            shorten_fnames(1);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn no_write_message_buf(buf: *mut buf_T) {
    unsafe {
        if !(*buf).terminal.is_null() && channel_job_running((*buf).b_p_channel as u64) {
            emsg(gettext(
                e_job_still_running_add_bang_to_end_the_job.as_ptr(),
            ));
        } else {
            semsg!(
                gettext(e_no_write_since_last_change_for_buffer_nr_add_bang_to_override.as_ptr()),
                (*buf).b_fnum,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn no_write_message() {
    unsafe {
        if !(*curbuf).terminal.is_null() && channel_job_running((*curbuf).b_p_channel as u64) {
            emsg(gettext(
                e_job_still_running_add_bang_to_end_the_job.as_ptr(),
            ));
        } else {
            emsg(gettext(
                e_no_write_since_last_change_add_bang_to_override.as_ptr(),
            ));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn no_write_message_nobang(buf: *const buf_T) {
    unsafe {
        if !(*buf).terminal.is_null() && channel_job_running((*buf).b_p_channel as u64) {
            emsg(gettext(e_job_still_running.as_ptr()));
        } else {
            emsg(gettext(e_no_write_since_last_change.as_ptr()));
        }
    }
}

#[inline]
unsafe fn buf_init_changedtick(buf: *mut buf_T) {
    unsafe {
        let mut key = [0 as c_char; 12];
        let src = b"changedtick\0";
        for i in 0..12 {
            key[i] = src[i] as c_char;
        }
        (*buf).changedtick_di = ChangedtickDictItem {
            di_tv: typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_number: buf_get_changedtick(buf),
                },
            },
            di_flags: DI_FLAGS_RO | DI_FLAGS_FIX,
            di_key: key,
        };
        tv_dict_add((*buf).b_vars, (&raw mut (*buf).changedtick_di).cast());
    }
}

/// Add a file name to the buffer list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_new(
    ffname_arg: *mut c_char,
    sfname_arg: *mut c_char,
    lnum: linenr_T,
    flags: c_int,
) -> *mut buf_T {
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut buf: *mut buf_T;

        fname_expand(curbuf, &raw mut ffname, &raw mut sfname);

        let mut file_id: FileID = std::mem::zeroed();
        let file_id_valid = !sfname.is_null() && os_fileid(sfname, (&raw mut file_id).cast());
        if !ffname.is_null() && (flags & (BLN_DUMMY | BLN_NEW)) == 0 {
            buf = buflist_findname_file_id(ffname, &raw mut file_id, file_id_valid);
            if !buf.is_null() {
                xfree(ffname.cast());
                if lnum != 0 {
                    buflist_setfpos(
                        buf,
                        if (flags & BLN_NOCURWIN) != 0 {
                            ptr::null_mut()
                        } else {
                            curwin
                        },
                        lnum,
                        0,
                        false,
                    );
                }
                if (flags & BLN_NOOPT) == 0 {
                    buf_copy_options(buf, 0);
                }
                if (flags & BLN_LISTED) != 0 && (*buf).b_p_bl == 0 {
                    (*buf).b_p_bl = 1;
                    let mut bufref: bufref_T = std::mem::zeroed();
                    set_bufref(&raw mut bufref, buf);
                    if (flags & BLN_DUMMY) == 0
                        && apply_autocmds(
                            EVENT_BUFADD,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            false,
                            buf,
                        )
                        && !bufref_valid(&raw mut bufref)
                    {
                        return ptr::null_mut();
                    }
                }
                return buf;
            }
        }

        buf = ptr::null_mut();
        if (flags & BLN_CURBUF) != 0 && curbuf_reusable() {
            let mut bufref: bufref_T = std::mem::zeroed();

            debug_assert!(!curbuf.is_null());
            buf = curbuf;
            set_bufref(&raw mut bufref, buf);
            trigger_undo_ftplugin(buf, curwin);
            buf_freeall(buf, BFA_WIPE | BFA_DEL);
            if aborting() {
                xfree(ffname.cast());
                return ptr::null_mut();
            }
            if !bufref_valid(&raw mut bufref) {
                buf = ptr::null_mut();
            }
        }
        if buf != curbuf || curbuf.is_null() {
            buf = xcalloc(1, std::mem::size_of::<buf_T>()).cast::<buf_T>();
            (*buf).b_vars = tv_dict_alloc();
            init_var_dict((*buf).b_vars, (&raw mut (*buf).b_bufvar).cast(), VAR_SCOPE);
            buf_init_changedtick(buf);
        }

        if !ffname.is_null() {
            debug_assert!(!sfname.is_null());
            (*buf).b_ffname = ffname;
            (*buf).b_sfname = xstrdup(sfname);
            to_slash((*buf).b_sfname);
        }

        clear_wininfo(buf);
        let curwin_info = xcalloc(1, std::mem::size_of::<WinInfo>()).cast::<WinInfo>();
        (*buf).b_wininfo.push(curwin_info.cast());

        if buf == curbuf {
            free_buffer_stuff(buf, kBffInitChangedtick);
            (*buf).b_p_initialized = false;
            buf_copy_options(buf, BCO_ENTER);
            (*curbuf).b_kmap_state |= KEYMAP_INIT;
        } else {
            (*buf).b_next = ptr::null_mut();
            if firstbuf.is_null() {
                (*buf).b_prev = ptr::null_mut();
                firstbuf = buf;
            } else {
                (*lastbuf).b_next = buf;
                (*buf).b_prev = lastbuf;
            }
            lastbuf = buf;

            (*buf).b_fnum = top_file_num;
            top_file_num += 1;
            *map_put_ref_int_ptr_t(
                &raw mut buffer_handles,
                (*buf).b_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
            ) = buf.cast();
            if top_file_num < 0 {
                emsg(gettext(
                    c"W14: Warning: List of file names overflow".as_ptr(),
                ));
                if emsg_silent == 0 && !in_assert_fails {
                    msg_delay(3001, true);
                }
                top_file_num = 1;
            }

            buf_copy_options(buf, BCO_ALWAYS);
        }

        (*curwin_info).wi_mark = INIT_FMARK;
        (*curwin_info).wi_mark.mark.lnum = lnum;
        (*curwin_info).wi_win = curwin;

        hash_init(&raw mut (*buf).b_s.b_keywtab);
        hash_init(&raw mut (*buf).b_s.b_keywtab_ic);

        (*buf).b_fname = (*buf).b_sfname;
        if !file_id_valid {
            (*buf).file_id_valid = false;
        } else {
            (*buf).file_id_valid = true;
            (*buf).file_id = file_id;
        }
        (*buf).b_u_synced = true;
        (*buf).b_flags = BF_CHECK_RO | BF_NEVERLOADED;
        if (flags & BLN_DUMMY) != 0 {
            (*buf).b_flags |= BF_DUMMY;
        }
        buf_clear_file(buf);
        clrallmarks(buf, 0);
        fmarks_check_names(buf);
        (*buf).b_p_bl = c_int::from((flags & BLN_LISTED) != 0);
        (*buf).update_channels.destroy();
        (*buf).update_channels.init();
        (*buf).update_callbacks.destroy();
        (*buf).update_callbacks.init();
        if (flags & BLN_DUMMY) == 0 {
            let mut bufref: bufref_T = std::mem::zeroed();
            set_bufref(&raw mut bufref, buf);
            if apply_autocmds(EVENT_BUFNEW, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(&raw mut bufref)
            {
                return ptr::null_mut();
            }
            if (flags & BLN_LISTED) != 0
                && apply_autocmds(EVENT_BUFADD, ptr::null_mut(), ptr::null_mut(), false, buf)
                && !bufref_valid(&raw mut bufref)
            {
                return ptr::null_mut();
            }
            if aborting() {
                return ptr::null_mut();
            }
        }

        (*buf).b_prompt_callback.cb_type = kCallbackNone;
        (*buf).b_prompt_interrupt.cb_type = kCallbackNone;
        (*buf).b_prompt_text = ptr::null_mut();
        (*buf).b_prompt_start = INIT_FMARK;
        (*buf).b_prompt_start.mark.col = 2;
        (*buf).b_prompt_append_new_line = true;

        buf
    }
}

/// Return true if the current buffer is empty, unnamed, unmodified and used in
/// only one window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn curbuf_reusable() -> bool {
    unsafe {
        !curbuf.is_null()
            && (*curbuf).b_ffname.is_null()
            && (*curbuf).b_nwindows <= 1
            && (*curbuf).terminal.is_null()
            && ((*curbuf).b_ml.ml_mfp.is_null() || buf_is_empty(curbuf))
            && !bt_quickfix(curbuf)
            && !curbuf_is_changed()
    }
}

/// Free the memory for the options of a buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_buf_options(buf: *mut buf_T, free_p_ff: bool) {
    unsafe {
        if free_p_ff {
            clear_string_option(&raw mut (*buf).b_p_fenc);
            clear_string_option(&raw mut (*buf).b_p_ff);
            clear_string_option(&raw mut (*buf).b_p_bh);
            clear_string_option(&raw mut (*buf).b_p_bt);
        }
        clear_string_option(&raw mut (*buf).b_p_def);
        clear_string_option(&raw mut (*buf).b_p_inc);
        callback_free(&raw mut (*buf).b_p_inex);
        callback_free(&raw mut (*buf).b_p_inde);
        clear_string_option(&raw mut (*buf).b_p_indk);
        clear_string_option(&raw mut (*buf).b_p_fp);
        callback_free(&raw mut (*buf).b_p_fex);
        clear_string_option(&raw mut (*buf).b_p_kp);
        clear_string_option(&raw mut (*buf).b_p_mps);
        clear_string_option(&raw mut (*buf).b_p_fo);
        clear_string_option(&raw mut (*buf).b_p_flp);
        clear_string_option(&raw mut (*buf).b_p_isk);
        clear_string_option(&raw mut (*buf).b_p_vsts);
        xfree((*buf).b_p_vsts_nopaste.cast());
        (*buf).b_p_vsts_nopaste = ptr::null_mut();
        xfree((*buf).b_p_vsts_array.cast());
        (*buf).b_p_vsts_array = ptr::null_mut();
        clear_string_option(&raw mut (*buf).b_p_vts);
        xfree((*buf).b_p_vts_array.cast());
        (*buf).b_p_vts_array = ptr::null_mut();
        clear_string_option(&raw mut (*buf).b_p_keymap);
        keymap_ga_clear(&raw mut (*buf).b_kmap_ga);
        ga_clear(&raw mut (*buf).b_kmap_ga);
        clear_string_option(&raw mut (*buf).b_p_com);
        clear_string_option(&raw mut (*buf).b_p_cms);
        clear_string_option(&raw mut (*buf).b_p_nf);
        clear_string_option(&raw mut (*buf).b_p_syn);
        clear_string_option(&raw mut (*buf).b_s.b_syn_isk);
        clear_string_option(&raw mut (*buf).b_s.b_p_spc);
        clear_string_option(&raw mut (*buf).b_s.b_p_spf);
        vim_regfree((*buf).b_s.b_cap_prog.cast());
        (*buf).b_s.b_cap_prog = ptr::null_mut();
        clear_string_option(&raw mut (*buf).b_s.b_p_spl);
        clear_string_option(&raw mut (*buf).b_s.b_p_spo);
        clear_string_option(&raw mut (*buf).b_p_sua);
        clear_string_option(&raw mut (*buf).b_p_ft);
        clear_string_option(&raw mut (*buf).b_p_cink);
        clear_string_option(&raw mut (*buf).b_p_cino);
        clear_string_option(&raw mut (*buf).b_p_lop);
        clear_string_option(&raw mut (*buf).b_p_cinsd);
        clear_string_option(&raw mut (*buf).b_p_cinw);
        clear_string_option(&raw mut (*buf).b_p_cot);
        clear_string_option(&raw mut (*buf).b_p_cpt);
        #[cfg(windows)]
        clear_string_option(&raw mut (*buf).b_p_csl);
        callback_free(&raw mut (*buf).b_p_cfu);
        callback_free(&raw mut (*buf).b_p_ofu);
        callback_free(&raw mut (*buf).b_p_tsrfu);
        clear_cpt_callbacks(&raw mut (*buf).b_p_cpt_cb, (*buf).b_p_cpt_count);
        (*buf).b_p_cpt_count = 0;
        clear_string_option(&raw mut (*buf).b_p_gefm);
        clear_string_option(&raw mut (*buf).b_p_gp);
        clear_string_option(&raw mut (*buf).b_p_mp);
        clear_string_option(&raw mut (*buf).b_p_efm);
        clear_string_option(&raw mut (*buf).b_p_ep);
        clear_string_option(&raw mut (*buf).b_p_path);
        clear_string_option(&raw mut (*buf).b_p_tags);
        clear_string_option(&raw mut (*buf).b_p_tc);
        callback_free(&raw mut (*buf).b_p_tfu);
        callback_free(&raw mut (*buf).b_p_ffu);
        clear_string_option(&raw mut (*buf).b_p_dict);
        clear_string_option(&raw mut (*buf).b_p_dia);
        clear_string_option(&raw mut (*buf).b_p_tsr);
        clear_string_option(&raw mut (*buf).b_p_qe);
        (*buf).b_p_ac = -1;
        (*buf).b_p_ar = -1;
        (*buf).b_p_fs = -1;
        (*buf).b_p_ul = NO_LOCAL_UNDOLEVEL;
        clear_string_option(&raw mut (*buf).b_p_lw);
        clear_string_option(&raw mut (*buf).b_p_bkc);
        clear_string_option(&raw mut (*buf).b_p_menc);
    }
}

/// Get alternate file `n`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_getfile(
    n: c_int,
    mut lnum: linenr_T,
    options: c_int,
    forceit: c_int,
) -> c_int {
    unsafe {
        let mut fm: *mut fmark_T = ptr::null_mut();

        let buf = buflist_findnr(n);
        if buf.is_null() {
            if (options & GETF_ALT) != 0 && n == 0 {
                emsg(gettext(e_noalt.as_ptr()));
            } else {
                semsg!(gettext(e_buffer_nr_not_found.as_ptr()), n);
            }
            return FAIL;
        }

        if buf == curbuf {
            return OK;
        }

        if text_or_buf_locked() {
            return FAIL;
        }

        let col: colnr_T;
        let mut restore_view = false;
        if lnum == 0 {
            fm = buflist_findfmark(buf);
            lnum = (*fm).mark.lnum;
            col = (*fm).mark.col;
            restore_view = true;
        } else {
            col = 0;
        }

        if (options & GETF_SWITCH) != 0 {
            let wp = swbuf_goto_win_with_buf(buf);

            if wp.is_null()
                && (swb_flags & (kOptSwbFlagVsplit | kOptSwbFlagSplit | kOptSwbFlagNewtab)) != 0
                && !buf_is_empty(curbuf)
            {
                if (swb_flags & kOptSwbFlagNewtab) != 0 {
                    tabpage_new();
                } else if win_split(
                    0,
                    if (swb_flags & kOptSwbFlagVsplit) != 0 {
                        WSP_VERT
                    } else {
                        0
                    },
                ) == FAIL
                {
                    return FAIL;
                }
                reset_binding(curwin);
            }
        }

        RedrawingDisabled += 1;
        if getfile(
            (*buf).b_fnum,
            ptr::null_mut(),
            ptr::null_mut(),
            (options & GETF_SETMARK) != 0,
            lnum,
            forceit != 0,
        ) <= 0
        {
            RedrawingDisabled -= 1;

            if p_sol == 0 && col != 0 {
                (*curwin).w_cursor.col = col;
                check_cursor_col(curwin);
                (*curwin).w_cursor.coladd = 0;
                (*curwin).w_set_curswant = true;
            }
            if (jop_flags & kOptJopFlagView) != 0 && restore_view {
                mark_view_restore(fm);
            }
            return OK;
        }
        RedrawingDisabled -= 1;
        FAIL
    }
}

unsafe fn buflist_getfpos() {
    unsafe {
        let fm = buflist_findfmark(curbuf);
        let fpos = &raw const (*fm).mark;

        (*curwin).w_cursor.lnum = (*fpos).lnum;
        check_cursor_lnum(curwin);

        if p_sol != 0 {
            (*curwin).w_cursor.col = 0;
        } else {
            (*curwin).w_cursor.col = (*fpos).col;
            check_cursor_col(curwin);
            (*curwin).w_cursor.coladd = 0;
            (*curwin).w_set_curswant = true;
        }

        if (jop_flags & kOptJopFlagView) != 0 {
            mark_view_restore(fm);
        }
    }
}

/// Find file in buffer list by name (it has to be for the current window).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findname_exp(fname: *mut c_char) -> *mut buf_T {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();
        let ffname = FullName_save(fname, cfg!(unix));
        if !ffname.is_null() {
            buf = buflist_findname(ffname);
            xfree(ffname.cast());
        }
        buf
    }
}

/// Find file in buffer list by name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findname(ffname: *mut c_char) -> *mut buf_T {
    unsafe {
        let mut file_id: FileID = std::mem::zeroed();
        let file_id_valid = os_fileid(ffname, (&raw mut file_id).cast());
        buflist_findname_file_id(ffname, &raw mut file_id, file_id_valid)
    }
}

unsafe fn buflist_findname_file_id(
    ffname: *mut c_char,
    file_id: *mut FileID,
    file_id_valid: bool,
) -> *mut buf_T {
    unsafe {
        let mut buf = lastbuf;
        while !buf.is_null() {
            if ((*buf).b_flags & BF_DUMMY) == 0
                && !otherfile_buf(buf, ffname, file_id, file_id_valid)
            {
                return buf;
            }
            buf = (*buf).b_prev;
        }
        ptr::null_mut()
    }
}

/// Find file in buffer list by a regexp pattern.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findpat(
    pattern: *const c_char,
    pattern_end: *const c_char,
    unlisted: bool,
    diffmode: bool,
    curtab_only: bool,
) -> c_int {
    unsafe {
        let mut match_fnum = -1;

        if pattern_end == pattern.add(1)
            && (*pattern == b'%' as c_char || *pattern == b'#' as c_char)
        {
            match_fnum = if *pattern == b'%' as c_char {
                (*curbuf).b_fnum
            } else {
                (*curwin).w_alt_fnum
            };
            let found_buf = buflist_findnr(match_fnum);
            if diffmode && (found_buf.is_null() || !diff_mode_buf(found_buf)) {
                match_fnum = -1;
            }
        } else {
            let pat = file_pat_to_reg_pat(pattern, pattern_end, ptr::null_mut(), 0);
            if pat.is_null() {
                return -1;
            }
            let patend = pat.add(strlen(pat).saturating_sub(1));
            let toggledollar = patend > pat && *patend == b'$' as c_char;

            let mut find_listed = 1;
            loop {
                for attempt in 0..=3 {
                    if toggledollar {
                        *patend = if attempt < 2 { NUL } else { b'$' as c_char };
                    }
                    let mut p = pat;
                    if *p == b'^' as c_char && (attempt & 1) == 0 {
                        p = p.add(1);
                    }

                    let mut regmatch: regmatch_T = std::mem::zeroed();
                    regmatch.regprog = vim_regcomp(p, if p_magic != 0 { RE_MAGIC } else { 0 });

                    let mut buf = lastbuf;
                    while !buf.is_null() {
                        if regmatch.regprog.is_null() {
                            xfree(pat.cast());
                            return -1;
                        }
                        if (*buf).b_p_bl == find_listed
                            && (!diffmode || diff_mode_buf(buf))
                            && !buflist_match(&raw mut regmatch, buf, false).is_null()
                        {
                            if curtab_only {
                                let mut found_window = false;
                                let mut wp = firstwin;
                                while !wp.is_null() {
                                    if (*wp).w_buffer == buf {
                                        found_window = true;
                                        break;
                                    }
                                    wp = (*wp).w_next;
                                }
                                if !found_window {
                                    buf = (*buf).b_prev;
                                    continue;
                                }
                            }
                            if match_fnum >= 0 {
                                match_fnum = -2;
                                break;
                            }
                            match_fnum = (*buf).b_fnum;
                        }
                        buf = (*buf).b_prev;
                    }

                    vim_regfree(regmatch.regprog);
                    if match_fnum >= 0 {
                        break;
                    }
                }

                if !unlisted || find_listed == 0 || match_fnum != -1 {
                    break;
                }
                find_listed = 0;
            }

            xfree(pat.cast());
        }

        if match_fnum == -2 {
            semsg!(
                gettext(c"E93: More than one match for %s".as_ptr()),
                pattern,
            );
        } else if match_fnum < 0 {
            semsg!(gettext(c"E94: No matching buffer for %s".as_ptr()), pattern);
        }
        match_fnum
    }
}

unsafe extern "C" fn buf_time_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let buf1 = *s1.cast::<*const buf_T>();
        let buf2 = *s2.cast::<*const buf_T>();

        if (*buf1).b_last_used == (*buf2).b_last_used {
            0
        } else if (*buf1).b_last_used > (*buf2).b_last_used {
            -1
        } else {
            1
        }
    }
}

/// Find all buffer names that match.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ExpandBufnames(
    pat: *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
    options: c_int,
) -> c_int {
    unsafe {
        let mut matches: *mut bufmatch_T = ptr::null_mut();
        let mut to_free = false;

        *num_file = 0;
        *file = ptr::null_mut();

        if (options & BUF_DIFF_FILTER) != 0 && (*curwin).w_onebuf_opt.wo_diff == 0 {
            return FAIL;
        }

        let fuzzy = cmdline_fuzzy_complete(pat);

        let mut patc: *mut c_char = ptr::null_mut();
        let mut fuzmatch: *mut fuzmatch_str_T = ptr::null_mut();
        let mut regmatch: regmatch_T = std::mem::zeroed();

        if !fuzzy {
            if *pat == b'^' as c_char && *pat.add(1) != NUL {
                patc = xstrdup(pat.add(1));
                to_free = true;
            } else if *pat == b'^' as c_char {
                patc = c"".as_ptr().cast_mut();
            } else {
                patc = pat;
            }
            regmatch.regprog = vim_regcomp(patc, RE_MAGIC);
        }

        let mut count = 0;
        let mut score = 0;
        for round in 1..=2 {
            count = 0;
            let mut buf = firstbuf;
            while !buf.is_null() {
                if (*buf).b_p_bl == 0 {
                    buf = (*buf).b_next;
                    continue;
                }
                if (options & BUF_DIFF_FILTER) != 0 && (buf == curbuf || !diff_mode_buf(buf)) {
                    buf = (*buf).b_next;
                    continue;
                }

                let mut p: *mut c_char = ptr::null_mut();
                if !fuzzy {
                    if regmatch.regprog.is_null() {
                        if to_free {
                            xfree(patc.cast());
                        }
                        return FAIL;
                    }
                    p = buflist_match(&raw mut regmatch, buf, p_wic != 0);
                } else {
                    score = fuzzy_match_str((*buf).b_sfname, pat);
                    if score != FUZZY_SCORE_NONE {
                        p = (*buf).b_sfname;
                    }
                    if p.is_null() {
                        score = fuzzy_match_str((*buf).b_ffname, pat);
                        if score != FUZZY_SCORE_NONE {
                            p = (*buf).b_ffname;
                        }
                    }
                }

                if p.is_null() {
                    buf = (*buf).b_next;
                    continue;
                }

                if round == 1 {
                    count += 1;
                    buf = (*buf).b_next;
                    continue;
                }

                p = if (options & WILD_HOME_REPLACE) != 0 {
                    home_replace_save(buf, p)
                } else {
                    xstrdup(p)
                };

                if !fuzzy {
                    if !matches.is_null() {
                        (*matches.add(count as usize)).buf = buf;
                        (*matches.add(count as usize)).match_ = p;
                        count += 1;
                    } else {
                        *(*file).add(count as usize) = p;
                        count += 1;
                    }
                } else {
                    (*fuzmatch.add(count as usize)).idx = count;
                    (*fuzmatch.add(count as usize)).str_ptr = p;
                    (*fuzmatch.add(count as usize)).score = score;
                    count += 1;
                }
                buf = (*buf).b_next;
            }
            if count == 0 {
                break;
            }
            if round == 1 {
                if !fuzzy {
                    *file = xmalloc(count as usize * std::mem::size_of::<*mut c_char>()).cast();
                    if (options & WILD_BUFLASTUSED) != 0 {
                        matches =
                            xmalloc(count as usize * std::mem::size_of::<bufmatch_T>()).cast();
                    }
                } else {
                    fuzmatch =
                        xmalloc(count as usize * std::mem::size_of::<fuzmatch_str_T>()).cast();
                }
            }
        }

        if !fuzzy {
            vim_regfree(regmatch.regprog);
            if to_free {
                xfree(patc.cast());
            }
        }

        if !fuzzy {
            if !matches.is_null() {
                if count > 1 {
                    qsort(
                        matches.cast(),
                        count as usize,
                        std::mem::size_of::<bufmatch_T>(),
                        buf_time_compare,
                    );
                }

                if (*matches).buf == curbuf {
                    for i in 1..count {
                        *(*file).add((i - 1) as usize) = (*matches.add(i as usize)).match_;
                    }
                    *(*file).add((count - 1) as usize) = (*matches).match_;
                } else {
                    for i in 0..count {
                        *(*file).add(i as usize) = (*matches.add(i as usize)).match_;
                    }
                }
                xfree(matches.cast());
            }
        } else {
            fuzzymatches_to_strmatches(fuzmatch, file, count, false);
        }

        *num_file = count;
        if count == 0 { FAIL } else { OK }
    }
}

unsafe fn buflist_match(rmp: *mut regmatch_T, buf: *mut buf_T, ignore_case: bool) -> *mut c_char {
    unsafe {
        let mut match_ = fname_match(rmp, (*buf).b_sfname, ignore_case);
        if match_.is_null() && !(*rmp).regprog.is_null() {
            match_ = fname_match(rmp, (*buf).b_ffname, ignore_case);
        }
        match_
    }
}

unsafe fn fname_match(rmp: *mut regmatch_T, name: *mut c_char, ignore_case: bool) -> *mut c_char {
    unsafe {
        let mut match_: *mut c_char = ptr::null_mut();

        if name.is_null() || (*rmp).regprog.is_null() {
            return ptr::null_mut();
        }

        (*rmp).rm_ic = p_fic != 0 || ignore_case;
        if vim_regexec(rmp, name, 0) {
            match_ = name;
        } else if !(*rmp).regprog.is_null() {
            let p = home_replace_save(ptr::null_mut(), name);
            if vim_regexec(rmp, p, 0) {
                match_ = name;
            }
            xfree(p.cast());
        }

        match_
    }
}

/// Find a file in the buffer list by buffer number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findnr(mut nr: c_int) -> *mut buf_T {
    unsafe {
        if nr == 0 {
            nr = (*curwin).w_alt_fnum;
        }
        let k = mh_get_int(&raw mut buffer_handles.set, nr);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*buffer_handles.values.add(k as usize)).cast::<buf_T>()
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_nr2name(
    n: c_int,
    fullname: c_int,
    helptail: c_int,
) -> *mut c_char {
    unsafe {
        let buf = buflist_findnr(n);
        if buf.is_null() {
            return ptr::null_mut();
        }
        home_replace_save(
            if helptail != 0 { buf } else { ptr::null_mut() },
            if fullname != 0 {
                (*buf).b_ffname
            } else {
                (*buf).b_fname
            },
        )
    }
}

/// Set the line and column numbers for the given buffer and window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_setfpos(
    buf: *mut buf_T,
    win: *mut win_T,
    mut lnum: linenr_T,
    col: colnr_T,
    copy_options: bool,
) {
    unsafe {
        let mut wip: *mut WinInfo = ptr::null_mut();
        let mut i = 0usize;
        while i < (*buf).b_wininfo.size {
            wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
            if (*wip).wi_win == win {
                break;
            }
            i += 1;
        }

        if i == (*buf).b_wininfo.size {
            wip = xcalloc(1, std::mem::size_of::<WinInfo>()).cast::<WinInfo>();
            (*wip).wi_win = win;
            if lnum == 0 {
                lnum = 1;
            }
        } else {
            (*buf).b_wininfo.size -= 1;
            if i < (*buf).b_wininfo.size {
                ptr::copy(
                    (*buf).b_wininfo.items.add(i + 1),
                    (*buf).b_wininfo.items.add(i),
                    (*buf).b_wininfo.size - i,
                );
            }
            if copy_options && (*wip).wi_optset {
                clear_winopt(&raw mut (*wip).wi_opt);
                deleteFoldRecurse(buf, &raw mut (*wip).wi_folds);
            }
        }
        if lnum != 0 {
            (*wip).wi_mark.mark.lnum = lnum;
            (*wip).wi_mark.mark.col = col;
            if !win.is_null() {
                (*wip).wi_mark.view = mark_view_make(win, (*wip).wi_mark.mark);
            }
        }
        if !win.is_null() {
            (*wip).wi_changelistidx = (*win).w_changelistidx;
        }
        if copy_options && !win.is_null() {
            copy_winopt(&raw mut (*win).w_onebuf_opt, &raw mut (*wip).wi_opt);
            (*wip).wi_fold_manual = (*win).w_fold_manual;
            cloneFoldGrowArray(&raw mut (*win).w_folds, &raw mut (*wip).wi_folds);
            (*wip).wi_optset = true;
        }

        (*buf).b_wininfo.push(ptr::null_mut());
        if (*buf).b_wininfo.size > 1 {
            ptr::copy(
                (*buf).b_wininfo.items,
                (*buf).b_wininfo.items.add(1),
                (*buf).b_wininfo.size - 1,
            );
        }
        *(*buf).b_wininfo.items = wip.cast();
    }
}

unsafe fn wininfo_other_tab_diff(wip: *mut WinInfo) -> bool {
    unsafe {
        if (*wip).wi_opt.wo_diff == 0 {
            return false;
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wip).wi_win == wp {
                return false;
            }
            wp = (*wp).w_next;
        }
        true
    }
}

unsafe fn find_wininfo(
    buf: *mut buf_T,
    need_options: bool,
    skip_diff_buffer: bool,
) -> *mut WinInfo {
    unsafe {
        for i in 0..(*buf).b_wininfo.size {
            let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
            if (*wip).wi_win == curwin
                && (!skip_diff_buffer || !wininfo_other_tab_diff(wip))
                && (!need_options || (*wip).wi_optset)
            {
                return wip;
            }
        }

        if skip_diff_buffer {
            for i in 0..(*buf).b_wininfo.size {
                let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
                if !wininfo_other_tab_diff(wip)
                    && (!need_options
                        || (*wip).wi_optset
                        || (!(*wip).wi_win.is_null() && (*(*wip).wi_win).w_buffer == buf))
                {
                    return wip;
                }
            }
        } else if (*buf).b_wininfo.size != 0 {
            return (*(*buf).b_wininfo.items).cast::<WinInfo>();
        }
        ptr::null_mut()
    }
}

/// Reset the local window options to the values last used in this window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_winopts(buf: *mut buf_T) {
    unsafe {
        clear_winopt(&raw mut (*curwin).w_onebuf_opt);
        clearFolding(curwin);

        let wip = find_wininfo(buf, true, true);
        if !wip.is_null()
            && (*wip).wi_win != curwin
            && !(*wip).wi_win.is_null()
            && (*(*wip).wi_win).w_buffer == buf
            && (*(*wip).wi_win).w_config.style != kWinStyleMinimal
        {
            let wp = (*wip).wi_win;
            copy_winopt(&raw mut (*wp).w_onebuf_opt, &raw mut (*curwin).w_onebuf_opt);
            (*curwin).w_fold_manual = (*wp).w_fold_manual;
            (*curwin).w_foldinvalid = true;
            cloneFoldGrowArray(&raw mut (*wp).w_folds, &raw mut (*curwin).w_folds);
        } else if !wip.is_null()
            && (*wip).wi_optset
            && ((*wip).wi_win.is_null()
                || (*wip).wi_win == curwin
                || (*(*wip).wi_win).w_config.style != kWinStyleMinimal)
        {
            copy_winopt(&raw mut (*wip).wi_opt, &raw mut (*curwin).w_onebuf_opt);
            (*curwin).w_fold_manual = (*wip).wi_fold_manual;
            (*curwin).w_foldinvalid = true;
            cloneFoldGrowArray(&raw mut (*wip).wi_folds, &raw mut (*curwin).w_folds);
        } else {
            copy_winopt(
                &raw mut (*curwin).w_allbuf_opt,
                &raw mut (*curwin).w_onebuf_opt,
            );
        }
        if !wip.is_null() {
            (*curwin).w_changelistidx = (*wip).wi_changelistidx;
        }

        if (*curwin).w_config.style == kWinStyleMinimal {
            didset_window_options(curwin, false);
            win_set_minimal_style(curwin);
        }

        if p_fdls >= 0 {
            (*curwin).w_onebuf_opt.wo_fdl = p_fdls;
        }
        didset_window_options(curwin, false);
    }
}

/// Find the mark for the buffer `buf` for the current window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findfmark(buf: *mut buf_T) -> *mut fmark_T {
    unsafe {
        let wip = find_wininfo(buf, false, false);
        if wip.is_null() {
            &raw mut no_position
        } else {
            &raw mut (*wip).wi_mark
        }
    }
}

/// Find the lnum for the buffer `buf` for the current window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_findlnum(buf: *mut buf_T) -> linenr_T {
    unsafe { (*buflist_findfmark(buf)).mark.lnum }
}

/// List all known file names (for `:files` and `:buffers` command).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_list(eap: *mut exarg_T) {
    unsafe {
        let mut buf = firstbuf;
        let mut buflist: garray_T = std::mem::zeroed();
        let mut buflist_data: *mut *mut buf_T = ptr::null_mut();

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if !vim_strchr((*eap).arg, b't' as c_int).is_null() {
            ga_init(
                &raw mut buflist,
                std::mem::size_of::<*mut buf_T>() as c_int,
                50,
            );
            while !buf.is_null() {
                ga_grow(&raw mut buflist, 1);
                *(buflist.ga_data.cast::<*mut buf_T>()).add(buflist.ga_len as usize) = buf;
                buflist.ga_len += 1;
                buf = (*buf).b_next;
            }

            qsort(
                buflist.ga_data,
                buflist.ga_len as usize,
                std::mem::size_of::<*mut buf_T>(),
                buf_time_compare,
            );

            buflist_data = buflist.ga_data.cast::<*mut buf_T>();
            buf = if !buflist_data.is_null() {
                *buflist_data
            } else {
                ptr::null_mut()
            };
        }
        let mut p = buflist_data;

        while !buf.is_null() && !got_int {
            let is_terminal = !(*buf).terminal.is_null();
            let job_running = is_terminal && terminal_running((*buf).terminal.cast());

            let mut next_buf = || {
                if !buflist_data.is_null() {
                    p = p.add(1);
                    if p < buflist_data.add(buflist.ga_len as usize) {
                        *p
                    } else {
                        ptr::null_mut()
                    }
                } else {
                    (*buf).b_next
                }
            };

            if ((*buf).b_p_bl == 0
                && (*eap).forceit == 0
                && vim_strchr((*eap).arg, b'u' as c_int).is_null())
                || (!vim_strchr((*eap).arg, b'u' as c_int).is_null() && (*buf).b_p_bl != 0)
                || (!vim_strchr((*eap).arg, b'+' as c_int).is_null()
                    && (((*buf).b_flags & BF_READERR) != 0 || !buf_is_changed(buf)))
                || (!vim_strchr((*eap).arg, b'a' as c_int).is_null()
                    && ((*buf).b_ml.ml_mfp.is_null() || (*buf).b_nwindows == 0))
                || (!vim_strchr((*eap).arg, b'h' as c_int).is_null()
                    && ((*buf).b_ml.ml_mfp.is_null() || (*buf).b_nwindows != 0))
                || (!vim_strchr((*eap).arg, b'R' as c_int).is_null()
                    && (!is_terminal || !job_running))
                || (!vim_strchr((*eap).arg, b'F' as c_int).is_null()
                    && (!is_terminal || job_running))
                || (!vim_strchr((*eap).arg, b'-' as c_int).is_null() && (*buf).b_p_ma != 0)
                || (!vim_strchr((*eap).arg, b'=' as c_int).is_null() && (*buf).b_p_ro == 0)
                || (!vim_strchr((*eap).arg, b'x' as c_int).is_null()
                    && ((*buf).b_flags & BF_READERR) == 0)
                || (!vim_strchr((*eap).arg, b'%' as c_int).is_null() && buf != curbuf)
                || (!vim_strchr((*eap).arg, b'#' as c_int).is_null()
                    && (buf == curbuf || (*curwin).w_alt_fnum != (*buf).b_fnum))
            {
                buf = next_buf();
                continue;
            }

            let name = buf_spname(buf);
            let namebuff_ptr = (&raw mut NameBuff).cast::<c_char>();
            if !name.is_null() {
                xstrlcpy(namebuff_ptr, name, MAXPATHL);
            } else {
                home_replace(buf, (*buf).b_fname, namebuff_ptr, MAXPATHL, true);
            }

            if message_filtered(namebuff_ptr) {
                buf = next_buf();
                continue;
            }

            let changed_char = if ((*buf).b_flags & BF_READERR) != 0 {
                b'x' as c_int
            } else if buf_is_changed(buf) {
                b'+' as c_int
            } else {
                b' ' as c_int
            };
            let mut ro_char = if (*buf).b_p_ma == 0 {
                b'-' as c_int
            } else if (*buf).b_p_ro != 0 {
                b'=' as c_int
            } else {
                b' ' as c_int
            };
            if is_terminal {
                ro_char = if terminal_running((*buf).terminal.cast()) {
                    b'R' as c_int
                } else {
                    b'F' as c_int
                };
            }

            if !ui_has(kUIMessages) || msg_col > 0 {
                msg_putchar(b'\n' as c_int);
            }
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            let mut len = vim_snprintf_safelen!(
                iobuff_ptr,
                IOSIZE - 20,
                c"%3d%c%c%c%c%c \"%s\"".as_ptr(),
                (*buf).b_fnum,
                if (*buf).b_p_bl != 0 {
                    b' ' as c_int
                } else {
                    b'u' as c_int
                },
                if buf == curbuf {
                    b'%' as c_int
                } else if (*curwin).w_alt_fnum == (*buf).b_fnum {
                    b'#' as c_int
                } else {
                    b' ' as c_int
                },
                if (*buf).b_ml.ml_mfp.is_null() {
                    b' ' as c_int
                } else if (*buf).b_nwindows == 0 {
                    b'h' as c_int
                } else {
                    b'a' as c_int
                },
                ro_char,
                changed_char,
                namebuff_ptr,
            )
            .min(IOSIZE - 20);

            let mut i = 40 - vim_strsize(iobuff_ptr);
            loop {
                *iobuff_ptr.add(len) = b' ' as c_char;
                len += 1;
                i -= 1;
                if i <= 0 || len >= IOSIZE - 18 {
                    break;
                }
            }
            if !vim_strchr((*eap).arg, b't' as c_int).is_null() && (*buf).b_last_used != 0 {
                undo_fmt_time(iobuff_ptr.add(len), IOSIZE - len, (*buf).b_last_used);
            } else {
                vim_snprintf!(
                    iobuff_ptr.add(len),
                    IOSIZE - len,
                    gettext(c"line %lld".as_ptr()),
                    if buf == curbuf {
                        i64::from((*curwin).w_cursor.lnum)
                    } else {
                        i64::from(buflist_findlnum(buf))
                    },
                );
            }

            msg_outtrans(iobuff_ptr, 0, false);
            line_breakcheck();
            buf = next_buf();
        }

        if !buflist_data.is_null() {
            ga_clear(&raw mut buflist);
        }
    }
}

/// Get file name and line number for file `fnum`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_name_nr(
    fnum: c_int,
    fname: *mut *mut c_char,
    lnum: *mut linenr_T,
) -> c_int {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() || (*buf).b_fname.is_null() {
            return FAIL;
        }

        *fname = (*buf).b_fname;
        *lnum = buflist_findlnum(buf);

        OK
    }
}

/// Set the file name for `buf` to `ffname_arg`, short file name to `sfname_arg`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setfname(
    buf: *mut buf_T,
    ffname_arg: *mut c_char,
    sfname_arg: *mut c_char,
    message: bool,
) -> c_int {
    unsafe {
        let mut ffname = ffname_arg;
        let mut sfname = sfname_arg;
        let mut obuf: *mut buf_T = ptr::null_mut();
        let mut file_id: FileID = std::mem::zeroed();
        let mut file_id_valid = false;

        if ffname.is_null() || *ffname == NUL {
            if (*buf).b_sfname != (*buf).b_ffname {
                xfree((*buf).b_sfname.cast());
            }
            (*buf).b_sfname = ptr::null_mut();
            xfree((*buf).b_ffname.cast());
            (*buf).b_ffname = ptr::null_mut();
        } else {
            fname_expand(buf, &raw mut ffname, &raw mut sfname);
            if ffname.is_null() {
                return FAIL;
            }

            file_id_valid = os_fileid(ffname, (&raw mut file_id).cast());
            if ((*buf).b_flags & BF_DUMMY) == 0 {
                obuf = buflist_findname_file_id(ffname, &raw mut file_id, file_id_valid);
            }
            if !obuf.is_null() && obuf != buf {
                let mut in_use = false;

                let mut tab = first_tabpage;
                while !tab.is_null() {
                    let mut win = if tab == curtab {
                        firstwin
                    } else {
                        (*tab).tp_firstwin
                    };
                    while !win.is_null() {
                        if (*win).w_buffer == obuf {
                            in_use = true;
                        }
                        win = (*win).w_next;
                    }
                    tab = (*tab).tp_next;
                }

                if !(*obuf).b_ml.ml_mfp.is_null() || in_use {
                    if message {
                        emsg(gettext(
                            c"E95: Buffer with this name already exists".as_ptr(),
                        ));
                    }
                    xfree(ffname.cast());
                    return FAIL;
                }
                close_buffer(ptr::null_mut(), obuf, DOBUF_WIPE, false, false, false);
            }
            sfname = xstrdup(sfname);
            if CASE_INSENSITIVE_FILENAME {
                to_slash(sfname);
                path_fix_case(sfname);
            }
            if (*buf).b_sfname != (*buf).b_ffname {
                xfree((*buf).b_sfname.cast());
            }
            xfree((*buf).b_ffname.cast());
            (*buf).b_ffname = ffname;
            (*buf).b_sfname = sfname;
        }
        (*buf).b_fname = (*buf).b_sfname;
        if !file_id_valid {
            (*buf).file_id_valid = false;
        } else {
            (*buf).file_id_valid = true;
            (*buf).file_id = file_id;
        }

        buf_name_changed(buf);
        OK
    }
}

/// Crude way of changing the name of a buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_set_name(fnum: c_int, name: *mut c_char) {
    unsafe {
        let buf = buflist_findnr(fnum);
        if buf.is_null() {
            return;
        }

        if (*buf).b_sfname != (*buf).b_ffname {
            xfree((*buf).b_sfname.cast());
        }
        xfree((*buf).b_ffname.cast());
        (*buf).b_ffname = xstrdup(name);
        (*buf).b_sfname = ptr::null_mut();
        fname_expand(buf, &raw mut (*buf).b_ffname, &raw mut (*buf).b_sfname);
        (*buf).b_fname = (*buf).b_sfname;
    }
}

/// Take care of what needs to be done when the name of buffer `buf` has changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_name_changed(buf: *mut buf_T) {
    unsafe {
        if !(*buf).b_ml.ml_mfp.is_null() {
            ml_setname(buf);
        }

        if (*curwin).w_buffer == buf {
            check_arg_idx(curwin);
        }
        maketitle();
        status_redraw_all();
        fmarks_check_names(buf);
        ml_timestamp(buf);
    }
}

/// Set alternate file name for current window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setaltfname(
    ffname: *mut c_char,
    sfname: *mut c_char,
    lnum: linenr_T,
) -> *mut buf_T {
    unsafe {
        let buf = buflist_new(ffname, sfname, lnum, 0);
        if !buf.is_null() && (cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
            (*curwin).w_alt_fnum = (*buf).b_fnum;
        }
        buf
    }
}

/// Get alternate file name for current window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getaltfname(errmsg: bool) -> *mut c_char {
    unsafe {
        let mut fname: *mut c_char = ptr::null_mut();
        let mut dummy: linenr_T = 0;

        if buflist_name_nr(0, &raw mut fname, &raw mut dummy) == FAIL {
            if errmsg {
                emsg(gettext(e_noalt.as_ptr()));
            }
            return ptr::null_mut();
        }
        fname
    }
}

/// Add a file name to the buflist and return its number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_add(fname: *mut c_char, flags: c_int) -> c_int {
    unsafe {
        let buf = buflist_new(fname, ptr::null_mut(), 0, flags);
        if !buf.is_null() { (*buf).b_fnum } else { 0 }
    }
}

/// Set alternate cursor position for the current buffer and window `win`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buflist_altfpos(win: *mut win_T) {
    unsafe {
        buflist_setfpos(curbuf, win, (*win).w_cursor.lnum, (*win).w_cursor.col, true);
    }
}

/// Check that `ffname` is not the same file as current file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn otherfile(ffname: *mut c_char) -> bool {
    unsafe { otherfile_buf(curbuf, ffname, ptr::null_mut(), false) }
}

unsafe fn otherfile_buf(
    buf: *mut buf_T,
    ffname: *mut c_char,
    mut file_id_p: *mut FileID,
    mut file_id_valid: bool,
) -> bool {
    unsafe {
        if ffname.is_null() || *ffname == NUL || (*buf).b_ffname.is_null() {
            return true;
        }
        if path_equal(ffname, (*buf).b_ffname, kPathCmpLiteral) {
            return false;
        }
        {
            let mut file_id: FileID = std::mem::zeroed();
            if file_id_p.is_null() {
                file_id_p = &raw mut file_id;
                file_id_valid = os_fileid(ffname, file_id_p.cast());
            }
            if !file_id_valid {
                return true;
            }
            if buf_same_file_id(buf, file_id_p) {
                buf_set_file_id(buf);
                if buf_same_file_id(buf, file_id_p) {
                    return false;
                }
            }
        }
        true
    }
}

/// Set `file_id` for a buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_set_file_id(buf: *mut buf_T) {
    unsafe {
        let mut file_id: FileID = std::mem::zeroed();
        if !(*buf).b_fname.is_null() && os_fileid((*buf).b_fname, (&raw mut file_id).cast()) {
            (*buf).file_id_valid = true;
            (*buf).file_id = file_id;
        } else {
            (*buf).file_id_valid = false;
        }
    }
}

#[inline]
unsafe fn buf_same_file_id(buf: *mut buf_T, file_id: *mut FileID) -> bool {
    unsafe {
        (*buf).file_id_valid && os_fileid_equal((&raw const (*buf).file_id).cast(), file_id.cast())
    }
}

/// Print info about the current buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fileinfo(fullname: c_int, shorthelp: c_int, dont_truncate: bool) {
    unsafe {
        let buffer = xmalloc(IOSIZE).cast::<c_char>();
        let mut bufferlen = 0usize;

        if fullname > 1 {
            bufferlen =
                vim_snprintf_safelen!(buffer, IOSIZE, c"buf %d: ".as_ptr(), (*curbuf).b_fnum);
        }

        *buffer.add(bufferlen) = b'"' as c_char;
        bufferlen += 1;

        let mut name = buf_spname(curbuf);
        if !name.is_null() {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                c"%s".as_ptr(),
                name,
            );
        } else {
            name = if fullname == 0 && !(*curbuf).b_fname.is_null() {
                (*curbuf).b_fname
            } else {
                (*curbuf).b_ffname
            };
            bufferlen += home_replace(
                if shorthelp != 0 {
                    curbuf
                } else {
                    ptr::null_mut()
                },
                name,
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                true,
            );
        }

        let dontwrite = bt_dontwrite(curbuf);
        let is_changed = curbuf_is_changed();
        let mod_str = if is_changed {
            if shortmess(kShmMod) {
                c" [+]".as_ptr()
            } else {
                gettext(c" [Modified]".as_ptr())
            }
        } else {
            c" ".as_ptr()
        };
        let notedited_str = if ((*curbuf).b_flags & BF_NOTEDITED) != 0 && !dontwrite {
            gettext(c"[Not edited]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let new_str = if ((*curbuf).b_flags & BF_NEW) != 0 && !dontwrite {
            gettext(c"[New]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let readerr_str = if ((*curbuf).b_flags & BF_READERR) != 0 {
            gettext(c"[Read errors]".as_ptr())
        } else {
            c"".as_ptr()
        };
        let ro_str = if (*curbuf).b_p_ro != 0 {
            if shortmess(kShmRo) {
                gettext(c"[RO]".as_ptr())
            } else {
                gettext(c"[readonly]".as_ptr())
            }
        } else {
            c"".as_ptr()
        };
        let sep_str =
            if is_changed || ((*curbuf).b_flags & BF_WRITE_MASK) != 0 || (*curbuf).b_p_ro != 0 {
                c" ".as_ptr()
            } else {
                c"".as_ptr()
            };

        bufferlen += vim_snprintf_safelen!(
            buffer.add(bufferlen),
            IOSIZE - bufferlen,
            c"\"%s%s%s%s%s%s".as_ptr(),
            mod_str,
            notedited_str,
            new_str,
            readerr_str,
            ro_str,
            sep_str,
        );

        let line_count = i64::from((*curbuf).b_ml.ml_line_count);
        let cur_lnum = i64::from((*curwin).w_cursor.lnum);
        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                c"%s".as_ptr(),
                gettext(no_lines_msg.as_ptr()),
            );
        } else if p_ru != 0 {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                ngettext(
                    c"%lld line --%d%%--".as_ptr(),
                    c"%lld lines --%d%%--".as_ptr(),
                    line_count as usize,
                ),
                line_count,
                calc_percentage(cur_lnum, line_count),
            );
        } else {
            bufferlen += vim_snprintf_safelen!(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                gettext(c"line %lld of %lld --%d%%-- col ".as_ptr()),
                cur_lnum,
                line_count,
                calc_percentage(cur_lnum, line_count),
            );
            validate_virtcol(curwin);
            bufferlen += col_print(
                buffer.add(bufferlen),
                IOSIZE - bufferlen,
                (*curwin).w_cursor.col + 1,
                (*curwin).w_virtcol + 1,
            ) as usize;
        }

        append_arg_number(curwin, buffer.add(bufferlen), IOSIZE - bufferlen);

        if dont_truncate {
            msg_start();
            let n = msg_scroll;
            msg_scroll = 1;
            msg(buffer, 0);
            msg_scroll = n;
        } else {
            let p = msg_trunc(buffer, false, 0);
            if restart_edit != 0 || (msg_scrolled != 0 && !need_wait_return) {
                set_keep_msg(p, 0);
            }
        }

        xfree(buffer.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn col_print(
    buf: *mut c_char,
    buflen: usize,
    col: c_int,
    vcol: c_int,
) -> c_int {
    unsafe {
        if col == vcol {
            vim_snprintf_safelen!(buf, buflen, c"%d".as_ptr(), col) as c_int
        } else {
            vim_snprintf_safelen!(buf, buflen, c"%d-%d".as_ptr(), col, vcol) as c_int
        }
    }
}

/// Put the title name in the title bar and icon of the window.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn maketitle() {
    unsafe {
        let mut title_str: *mut c_char = ptr::null_mut();
        let mut icon_str: *mut c_char = ptr::null_mut();
        let mut buf = [0 as c_char; IOSIZE];
        let outbuf = CharBuf {
            data: buf.as_mut_ptr(),
            size: IOSIZE,
        };

        if !redrawing() {
            need_maketitle = true;
            return;
        }

        need_maketitle = false;
        if p_title == 0 && p_icon == 0 && lasttitle.is_null() && lasticon.is_null() {
            return;
        }

        if p_title != 0 {
            let mut maxlen = 0;

            if p_titlelen > 0 {
                maxlen = ((p_titlelen * i64::from(Columns) / 100) as c_int).max(10);
            }

            if *p_titlestring != NUL {
                if (stl_syntax & STL_IN_TITLE) != 0 {
                    build_stl_str_hl(
                        curwin,
                        outbuf,
                        p_titlestring,
                        kOptTitlestring,
                        0,
                        0,
                        maxlen,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                    title_str = buf.as_mut_ptr();
                } else {
                    title_str = p_titlestring;
                }
            } else {
                #[cfg(windows)]
                let p_ssl_save = p_ssl;
                #[cfg(windows)]
                {
                    p_ssl = 1;
                }
                let default_titlestring = c"%t%( %M%)%( (%{expand('%:p:~:h')})%)%a - Nvim"
                    .as_ptr()
                    .cast_mut();
                build_stl_str_hl(
                    curwin,
                    outbuf,
                    default_titlestring,
                    kOptTitlestring,
                    0,
                    0,
                    maxlen,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                title_str = buf.as_mut_ptr();
                #[cfg(windows)]
                {
                    p_ssl = p_ssl_save;
                }
            }
        }
        let mut mustset = value_change(title_str, &raw mut lasttitle);

        if p_icon != 0 {
            icon_str = buf.as_mut_ptr();
            if *p_iconstring != NUL {
                if (stl_syntax & STL_IN_ICON) != 0 {
                    build_stl_str_hl(
                        curwin,
                        outbuf,
                        p_iconstring,
                        kOptIconstring,
                        0,
                        0,
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                } else {
                    icon_str = p_iconstring;
                }
            } else {
                let mut name = buf_spname(curbuf);
                if name.is_null() {
                    name = path_tail((*curbuf).b_ffname);
                }
                let mut namelen = strlen(name) as c_int;
                if namelen > 100 {
                    namelen -= 100;
                    namelen += c_int::from(utf_cp_bounds(name, name.add(namelen as usize)).end_off);
                    name = name.add(namelen as usize);
                }
                strcpy_raw(buf.as_mut_ptr(), name);
                trans_characters(buf.as_mut_ptr(), IOSIZE as c_int);
            }
        }

        mustset |= value_change(icon_str, &raw mut lasticon);

        if mustset {
            resettitle();
        }
    }
}

unsafe fn value_change(str_: *mut c_char, last: *mut *mut c_char) -> bool {
    unsafe {
        if str_.is_null() != (*last).is_null()
            || (!str_.is_null() && !(*last).is_null() && strcmp(str_, *last) != 0)
        {
            xfree((*last).cast());
            if str_.is_null() {
                *last = ptr::null_mut();
                resettitle();
            } else {
                *last = xstrdup(str_);
                return true;
            }
        }
        false
    }
}

/// Set current window title.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn resettitle() {
    unsafe {
        ui_call_set_icon(cstr_as_string(lasticon));
        ui_call_set_title(cstr_as_string(lasttitle));
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_titles() {
    unsafe {
        xfree(lasttitle.cast());
        xfree(lasticon.cast());
    }
}

/// Get relative cursor position in window into `buf[]`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_rel_pos(wp: *mut win_T, buf: *mut c_char, buflen: c_int) -> c_int {
    unsafe {
        if buflen < 3 {
            return 0;
        }

        let mut above = (*wp).w_topline - 1;
        above += win_get_fill(wp, (*wp).w_topline) - (*wp).w_topfill;
        if (*wp).w_topline == 1 && (*wp).w_topfill >= 1 {
            above = 0;
        }
        let below = (*(*wp).w_buffer).b_ml.ml_line_count - (*wp).w_botline + 1;
        if below <= 0 {
            return vim_snprintf_safelen!(
                buf,
                buflen as usize,
                c"%s".as_ptr(),
                if above == 0 {
                    gettext(c"All".as_ptr())
                } else {
                    gettext(c"Bot".as_ptr())
                },
            ) as c_int;
        }

        if above <= 0 {
            return vim_snprintf_safelen!(
                buf,
                buflen as usize,
                c"%s".as_ptr(),
                gettext(c"Top".as_ptr()),
            ) as c_int;
        }

        let perc = calc_percentage(i64::from(above), i64::from(above + below));
        let mut tmp = [0 as c_char; 8];
        vim_snprintf!(tmp.as_mut_ptr(), tmp.len(), gettext(c"%d%%".as_ptr()), perc);
        vim_snprintf_safelen!(buf, buflen as usize, gettext(c"%3s".as_ptr()), tmp.as_ptr()) as c_int
    }
}

/// Append `(2 of 8)` to `buf[]`, if editing more than one file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn append_arg_number(
    wp: *mut win_T,
    buf: *mut c_char,
    buflen: usize,
) -> c_int {
    unsafe {
        let argcount = (*(*curwin).w_alist.cast::<alist_T>()).al_ga.ga_len;
        if argcount <= 1 {
            return 0;
        }

        let msg = if (*wp).w_arg_idx_invalid {
            gettext(c" ((%d) of %d)".as_ptr())
        } else {
            gettext(c" (%d of %d)".as_ptr())
        };

        vim_snprintf_safelen!(buf, buflen, msg, (*wp).w_arg_idx + 1, argcount) as c_int
    }
}

/// Make `*ffname` a full file name, set `*sfname` to `*ffname` if not NULL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fname_expand(
    buf: *mut buf_T,
    ffname: *mut *mut c_char,
    sfname: *mut *mut c_char,
) {
    unsafe {
        #[cfg(not(windows))]
        let _ = buf;
        if (*ffname).is_null() {
            return;
        }
        if (*sfname).is_null() {
            *sfname = *ffname;
        }
        *ffname = fix_fname(*ffname);
        if (*ffname).is_null() {
            *sfname = ptr::null_mut();
            return;
        }
        if os_isdir(*ffname) {
            let name = xstrdup(*sfname);
            let name_len = strlen(name);
            if name_len > 0
                && after_pathsep(name, name.add(name_len)) != 0
                && name.add(name_len) > get_past_head(name)
            {
                *path_tail_with_sep(name) = NUL;
            }
            let full = fix_fname(name);
            xfree(name.cast());
            if !full.is_null() {
                xfree((*ffname).cast());
                *ffname = full;
            }
            *ffname = concat_fnames_realloc(*ffname, c"".as_ptr(), true);
        }

        #[cfg(windows)]
        #[cfg(not(test))]
        if (*buf).b_p_bin == 0 {
            let rfname = os_resolve_shortcut(*ffname);
            if !rfname.is_null() {
                xfree((*ffname).cast());
                *ffname = rfname;
                *sfname = rfname;
            }
        }
    }
}

/// Return true if `buf` is a prompt buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_prompt(buf: *mut buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b'p' as c_char }
}

/// Return true if `buf` is the `|cmdwin|` scratch buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_cmdwin(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && buf == cmdwin_buf.cast_const() }
}

/// Open a window for a number of buffers.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_buffer_all(eap: *mut exarg_T) {
    unsafe {
        let mut wpnext: *mut win_T;
        let mut split_ret = OK;
        let mut open_wins = 0;
        let had_tab = cmdmod.cmod_tab;

        let count = if (*eap).addr_count == 0 {
            9999
        } else {
            (*eap).line2
        };

        let all = (*eap).cmdidx != cmdidx_T::CMD_unhide as c_int
            && (*eap).cmdidx != cmdidx_T::CMD_sunhide as c_int;

        reset_VIsual_and_resel();
        setpcmark();

        if had_tab > 0 {
            goto_tabpage_tp(first_tabpage, true, true);
        }
        loop {
            let mut tpnext = (*curtab).tp_next;
            let mut wp = if (*lastwin).w_floating {
                lastwin
            } else {
                firstwin
            };
            while !wp.is_null() {
                wpnext = if (*wp).w_floating {
                    if (*(*wp).w_prev).w_floating {
                        (*wp).w_prev
                    } else {
                        firstwin
                    }
                } else if (*wp).w_next.is_null() || (*(*wp).w_next).w_floating {
                    ptr::null_mut()
                } else {
                    (*wp).w_next
                };
                let size_mismatch = if (cmdmod.cmod_split & WSP_VERT) != 0 {
                    i64::from((*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height)
                        < i64::from(Rows)
                            - p_ch
                            - i64::from(tabline_height())
                            - i64::from(global_stl_height())
                } else {
                    (*wp).w_width != Columns
                };
                if ((*(*wp).w_buffer).b_nwindows > 1
                    || (*wp).w_floating
                    || size_mismatch
                    || (had_tab > 0 && wp != firstwin))
                    && firstwin != lastwin
                    && !(win_locked(wp) != 0 || (*(*wp).w_buffer).b_locked > 0)
                    && !is_ctx_win(wp)
                {
                    if win_close(wp, false, false) == FAIL {
                        break;
                    }
                    wpnext = if (*lastwin).w_floating {
                        lastwin
                    } else {
                        firstwin
                    };
                    tpnext = first_tabpage;
                    open_wins = 0;
                } else {
                    open_wins += 1;
                }
                wp = wpnext;
            }

            if had_tab == 0 || tpnext.is_null() {
                break;
            }
            goto_tabpage_tp(tpnext, true, true);
        }

        autocmd_no_enter += 1;
        win_enter(lastwin_nofloating(ptr::null_mut()), false);
        autocmd_no_leave += 1;
        let mut buf = firstbuf;
        while !buf.is_null() && open_wins < count {
            if (!all && (*buf).b_ml.ml_mfp.is_null()) || (*buf).b_p_bl == 0 {
                buf = (*buf).b_next;
                continue;
            }

            let mut wp: *mut win_T;
            if had_tab != 0 {
                wp = if (*buf).b_nwindows > 0 {
                    lastwin
                } else {
                    ptr::null_mut()
                };
            } else {
                wp = firstwin;
                while !wp.is_null() {
                    if !(*wp).w_floating && (*wp).w_buffer == buf {
                        break;
                    }
                    wp = (*wp).w_next;
                }
                if !wp.is_null() {
                    win_move_after(wp, curwin);
                }
            }

            if wp.is_null() && split_ret == OK {
                let mut bufref: bufref_T = std::mem::zeroed();
                set_bufref(&raw mut bufref, buf);
                let p_ea_save = p_ea;
                p_ea = 1;
                split_ret = win_split(0, WSP_ROOM | WSP_BELOW);
                open_wins += 1;
                p_ea = p_ea_save;
                if split_ret == FAIL {
                    buf = (*buf).b_next;
                    continue;
                }

                swap_exists_action = SEA_DIALOG;
                set_curbuf(buf, DOBUF_GOTO, (jop_flags & kOptJopFlagClean) == 0);
                if !bufref_valid(&raw mut bufref) {
                    swap_exists_action = SEA_NONE;
                    break;
                }
                if swap_exists_action == SEA_QUIT {
                    let mut cs: cleanup_T = std::mem::zeroed();
                    enter_cleanup(&raw mut cs);
                    win_close(curwin, true, false);
                    open_wins -= 1;
                    swap_exists_action = SEA_NONE;
                    swap_exists_did_quit = true;
                    leave_cleanup(&raw mut cs);
                } else {
                    handle_swap_exists(ptr::null_mut());
                }
            }

            os_breakcheck();
            if got_int {
                let _ = vgetc();
                break;
            }
            if aborting() {
                break;
            }
            if had_tab > 0 && i64::from(tabpage_index(ptr::null_mut())) <= p_tpm {
                cmdmod.cmod_tab = 9999;
            }
            buf = (*buf).b_next;
        }
        autocmd_no_enter -= 1;
        win_enter(firstwin, false);
        autocmd_no_leave -= 1;

        let mut wp = lastwin;
        while open_wins > count {
            let r = (buf_hide((*wp).w_buffer)
                || !buf_is_changed((*wp).w_buffer)
                || autowrite((*wp).w_buffer, false) == OK)
                && !is_ctx_win(wp);
            if !win_valid(wp) {
                wp = lastwin;
            } else if r {
                win_close(wp, !buf_hide((*wp).w_buffer), false);
                open_wins -= 1;
                wp = lastwin;
            } else {
                wp = (*wp).w_prev;
                if wp.is_null() {
                    break;
                }
            }
        }
    }
}

/// Process mode lines for the current file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_modelines(flags: c_int) {
    unsafe {
        let mut nmlines = p_mls as c_int;
        if (*curbuf).b_p_ml == 0 || nmlines == 0 {
            return;
        }

        if modelines_entered != 0 {
            return;
        }

        modelines_entered += 1;
        let mut lnum = 1;
        while (*curbuf).b_p_ml != 0 && lnum <= (*curbuf).b_ml.ml_line_count && lnum <= nmlines {
            if chk_modeline(lnum, flags) == FAIL {
                nmlines = 0;
            }
            lnum += 1;
        }

        lnum = (*curbuf).b_ml.ml_line_count;
        while (*curbuf).b_p_ml != 0
            && lnum > 0
            && lnum > nmlines
            && lnum > (*curbuf).b_ml.ml_line_count - nmlines
        {
            if chk_modeline(lnum, flags) == FAIL {
                nmlines = 0;
            }
            lnum -= 1;
        }
        modelines_entered -= 1;
    }
}

unsafe fn chk_modeline(lnum: linenr_T, flags: c_int) -> c_int {
    unsafe {
        let mut e: *mut c_char;
        let mut retval = OK;

        let mut prev: c_int = -1;
        let mut s = ml_get(lnum);
        let mut line_end = s.add(ml_get_len(lnum) as usize);
        while *s != NUL {
            if prev == -1 || ascii_isspace(prev) {
                if (prev != -1 && strncmp(s, c"ex:".as_ptr(), 3) == 0)
                    || strncmp(s, c"vi:".as_ptr(), 3) == 0
                {
                    break;
                }
                if (*s == b'v' as c_char || *s == b'V' as c_char)
                    && *s.add(1) == b'i' as c_char
                    && *s.add(2) == b'm' as c_char
                {
                    let s3 = *s.add(3);
                    if s3 == b'<' as c_char || s3 == b'=' as c_char || s3 == b'>' as c_char {
                        e = s.add(4);
                    } else {
                        e = s.add(3);
                    }
                    let mut vers: i64 = 0;
                    if !try_getdigits(&raw mut e, &raw mut vers) {
                        prev = *s as u8 as c_int;
                        s = s.add(1);
                        continue;
                    }

                    let vim_version = i64::from(min_vim_version());
                    if *e == b':' as c_char
                        && (*s != b'V' as c_char
                            || strncmp(skipwhite(e.add(1)), c"set".as_ptr(), 3) == 0)
                        && (s3 == b':' as c_char
                            || (vim_version >= vers && (s3 as u8).is_ascii_digit())
                            || (vim_version < vers && s3 == b'<' as c_char)
                            || (vim_version > vers && s3 == b'>' as c_char)
                            || (vim_version == vers && s3 == b'=' as c_char))
                    {
                        break;
                    }
                }
            }
            prev = *s as u8 as c_int;
            s = s.add(1);
        }

        if *s == NUL {
            return retval;
        }

        loop {
            s = s.add(1);
            if *s.sub(1) == b':' as c_char {
                break;
            }
        }

        let len = line_end.offset_from(s) as usize;
        let linecopy = xstrnsave(s, len);
        s = linecopy;
        line_end = s.add(len);

        estack_push(ETYPE_MODELINE, c"modelines".as_ptr().cast_mut(), lnum);

        let mut end = false;
        while !end {
            s = skipwhite(s);
            if *s == NUL {
                break;
            }

            e = s;
            while *e != b':' as c_char && *e != NUL {
                if *e == b'\\' as c_char && *e.add(1) == b':' as c_char {
                    ptr::copy(e.add(1), e, line_end.offset_from(e.add(1)) as usize + 1);
                    line_end = line_end.sub(1);
                }
                e = e.add(1);
            }
            if *e == NUL {
                end = true;
            }

            if strncmp(s, c"set ".as_ptr(), 4) == 0 || strncmp(s, c"se ".as_ptr(), 3) == 0 {
                if *e != b':' as c_char {
                    break;
                }
                end = true;
                s = s.add(if *s.add(2) == b' ' as c_char { 3 } else { 4 });
            }
            *e = NUL;

            if *s != NUL {
                let secure_save = secure;
                let save_current_sctx = current_sctx;
                current_sctx.sc_sid = SID_MODELINE;
                current_sctx.sc_seq = 0;
                current_sctx.sc_lnum = lnum;
                secure = 1;

                retval = do_set(s, OPT_MODELINE | OPT_LOCAL | flags);

                secure = secure_save;
                current_sctx = save_current_sctx;
                if retval == FAIL {
                    break;
                }
            }
            s = if e == line_end { e } else { e.add(1) };
        }

        estack_pop();
        xfree(linecopy.cast());

        retval
    }
}

/// Return true if `buf` is a help buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_help(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && (*buf).b_help }
}

/// Return true if `buf` is a normal buffer, `'buftype'` is empty.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_normal(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == NUL }
}

/// Return true if `buf` is the quickfix buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_quickfix(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b'q' as c_char }
}

/// Return true if `buf` is a terminal buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_terminal(buf: *const buf_T) -> bool {
    unsafe { !buf.is_null() && *(*buf).b_p_bt == b't' as c_char }
}

/// Return true if `buf` is a `"nofile"`, `"acwrite"`, `"terminal"` or `"prompt"` buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_nofilename(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && ((*(*buf).b_p_bt == b'n' as c_char && *(*buf).b_p_bt.add(2) == b'f' as c_char)
                || *(*buf).b_p_bt == b'a' as c_char
                || !(*buf).terminal.is_null()
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

unsafe fn bt_nofileread(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && ((*(*buf).b_p_bt == b'n' as c_char && *(*buf).b_p_bt.add(2) == b'f' as c_char)
                || *(*buf).b_p_bt == b't' as c_char
                || *(*buf).b_p_bt == b'q' as c_char
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

/// Return true if `buf` has `'buftype'` set to `"nofile"`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_nofile(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && *(*buf).b_p_bt == b'n' as c_char
            && *(*buf).b_p_bt.add(2) == b'f' as c_char
    }
}

/// Return true if `buf` is a `"nowrite"`, `"nofile"`, `"terminal"` or `"prompt"` buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_dontwrite(buf: *const buf_T) -> bool {
    unsafe {
        !buf.is_null()
            && (*(*buf).b_p_bt == b'n' as c_char
                || !(*buf).terminal.is_null()
                || *(*buf).b_p_bt == b'p' as c_char)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bt_dontwrite_msg(buf: *const buf_T) -> bool {
    unsafe {
        if bt_dontwrite(buf) {
            emsg(gettext(
                c"E382: Cannot write, 'buftype' option is set".as_ptr(),
            ));
            return true;
        }
        false
    }
}

/// Return true if the buffer should be hidden, according to `'hidden'`, `":hide"` and `'bufhidden'`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_hide(buf: *const buf_T) -> bool {
    unsafe {
        match *(*buf).b_p_bh as u8 {
            b'u' | b'w' | b'd' => false,
            b'h' => true,
            _ => p_hid != 0 || (cmdmod.cmod_flags & CMOD_HIDE) != 0,
        }
    }
}

/// Return special buffer name or NULL when the buffer has a normal file name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_spname(buf: *mut buf_T) -> *mut c_char {
    unsafe {
        if bt_quickfix(buf) {
            if (*buf).b_fnum == qf_stack_get_bufnr() {
                return gettext(msg_qflist);
            }
            return gettext(msg_loclist);
        }
        if bt_nofilename(buf) {
            if !(*buf).b_fname.is_null() {
                return (*buf).b_fname;
            }
            if bt_cmdwin(buf) {
                return gettext(c"[Command Line]".as_ptr());
            }
            if bt_prompt(buf) {
                return gettext(c"[Prompt]".as_ptr());
            }
            return gettext(c"[Scratch]".as_ptr());
        }
        if (*buf).b_fname.is_null() {
            return buf_get_fname(buf);
        }
        ptr::null_mut()
    }
}

/// Get `buf->b_fname`, use `"[No Name]"` if it is NULL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_get_fname(buf: *const buf_T) -> *mut c_char {
    unsafe {
        if (*buf).b_fname.is_null() {
            gettext(c"[No Name]".as_ptr())
        } else {
            (*buf).b_fname
        }
    }
}

/// Set `'buflisted'` for `curbuf` to `on` and trigger autocommands if it changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_buflisted(on: c_int) {
    unsafe {
        if on == (*curbuf).b_p_bl {
            return;
        }

        (*curbuf).b_p_bl = on;
        if on != 0 {
            apply_autocmds(
                EVENT_BUFADD,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        } else {
            apply_autocmds(
                EVENT_BUFDELETE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }
    }
}

/// Read the file for `buf` again and check if the contents changed.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_contents_changed(buf: *mut buf_T) -> bool {
    unsafe {
        let mut differ = true;

        let newbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if newbuf.is_null() {
            return true;
        }

        let mut ea: exarg_T = std::mem::zeroed();
        prep_exarg(&raw mut ea, buf);

        let mut aco: CtxSwitch = std::mem::zeroed();
        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), newbuf, 0);

        block_autocmds();

        if ml_open(curbuf) == OK
            && readfile(
                (*buf).b_ffname,
                (*buf).b_fname,
                0,
                0,
                MAXLNUM,
                &raw mut ea,
                READ_NEW | READ_DUMMY,
                false,
            ) == OK
            && (*buf).b_ml.ml_line_count == (*curbuf).b_ml.ml_line_count
        {
            differ = false;
            for lnum in 1..=(*curbuf).b_ml.ml_line_count {
                if strcmp(ml_get_buf(buf, lnum), ml_get(lnum)) != 0 {
                    differ = true;
                    break;
                }
            }
        }
        xfree(ea.cmd.cast());

        ctx_restore(&raw mut aco);

        if curbuf != newbuf {
            wipe_buffer(newbuf, false);
        }

        unblock_autocmds();

        differ
    }
}

/// Wipe out a (typically temporary) buffer.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn wipe_buffer(buf: *mut buf_T, aucmd: bool) {
    unsafe {
        if !aucmd {
            block_autocmds();
        }
        close_buffer(ptr::null_mut(), buf, DOBUF_WIPE, false, true, false);
        if !aucmd {
            unblock_autocmds();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_is_empty(buf: *mut buf_T) -> bool {
    unsafe { (*buf).b_ml.ml_line_count == 1 && *ml_get_buf(buf, 1) == NUL }
}

/// Increment `b:changedtick` value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_inc_changedtick(buf: *mut buf_T) {
    unsafe {
        buf_set_changedtick(buf, buf_get_changedtick(buf) + 1);
    }
}

/// Set `b:changedtick`, also checking `b:` for consistency in debug build.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_set_changedtick(buf: *mut buf_T, changedtick: varnumber_T) {
    unsafe {
        let old_val = (*buf).changedtick_di.di_tv;

        let changedtick_di = tv_dict_find((*buf).b_vars, c"changedtick".as_ptr(), 11);
        debug_assert!(!changedtick_di.is_null());
        debug_assert!((*changedtick_di).di_tv.v_type == VarType::VAR_NUMBER);
        debug_assert!((*changedtick_di).di_tv.v_lock == VAR_FIXED);
        debug_assert!((*changedtick_di).di_flags == (DI_FLAGS_RO | DI_FLAGS_FIX));
        debug_assert!(changedtick_di == (&raw mut (*buf).changedtick_di).cast());
        let _ = changedtick_di;

        (*buf).changedtick_di.di_tv.vval.v_number = changedtick;

        if tv_dict_is_watched((*buf).b_vars) {
            (*buf).b_locked += 1;
            tv_dict_watcher_notify(
                (*buf).b_vars,
                (&raw const (*buf).changedtick_di.di_key).cast(),
                &raw mut (*buf).changedtick_di.di_tv,
                &raw const old_val as *mut _,
            );
            (*buf).b_locked -= 1;
        }
    }
}

/// Read the given buffer contents into a string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn read_buffer_into(
    buf: *mut buf_T,
    start: linenr_T,
    end: linenr_T,
    sb: *mut StringBuilder,
) {
    unsafe {
        debug_assert!(!buf.is_null());
        debug_assert!(!sb.is_null());

        if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 {
            return;
        }

        let mut written = 0usize;
        let mut len: usize;
        let mut lnum = start;
        let mut lp = ml_get_buf(buf, lnum);
        let mut lplen = ml_get_buf_len(buf, lnum) as usize;

        loop {
            if lplen == 0 {
                len = 0;
            } else if *lp.add(written) == NL {
                len = 1;
                (*sb).push(NUL);
            } else {
                #[cfg(not(test))]
                let s = vim_strchr(lp.add(written), NL as c_int);
                #[cfg(test)]
                let s: *mut c_char = ptr::null_mut();
                len = if s.is_null() {
                    lplen - written
                } else {
                    s.offset_from(lp.add(written)) as usize
                };
                (*sb).concat_len(lp.add(written), len);
            }

            if len == lplen - written {
                if lnum != end
                    || ((*buf).b_p_bin == 0 && (*buf).b_p_fixeol != 0)
                    || (lnum != (*buf).b_no_eol_lnum
                        && (lnum != (*buf).b_ml.ml_line_count || (*buf).b_p_eol != 0))
                {
                    (*sb).push(NL);
                }
                lnum += 1;
                if lnum > end {
                    break;
                }
                lp = ml_get_buf(buf, lnum);
                lplen = ml_get_buf_len(buf, lnum) as usize;
                written = 0;
            } else if len > 0 {
                written += len;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_percentage_and_bt_predicates_work() {
        unsafe {
            assert_eq!(calc_percentage(50, 200), 25);
            assert_eq!(calc_percentage(2_000_000, 4_000_000), 50);
            assert!(!bt_help(ptr::null()));
            assert!(!bt_normal(ptr::null()));
            assert!(!bt_quickfix(ptr::null()));
            assert!(!bt_terminal(ptr::null()));
            assert!(!bt_nofile(ptr::null()));
            assert!(!bt_dontwrite(ptr::null()));
        }
    }
}
