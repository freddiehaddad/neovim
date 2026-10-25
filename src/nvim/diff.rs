//! Diff mode functions (`src/nvim/diff.c`).
use crate::api::private::helpers::{boolean_obj, static_cstr_as_obj};
use crate::autocmd::{EVENT_DIFFUPDATED, apply_autocmds};
use crate::buffer::{
    MAXLNUM, bt_prompt, buf_T, buf_get_changedtick, buf_is_empty, buf_valid, buflist_findnr,
    buflist_findpat, bufref_T, bufref_valid, modifiable, set_bufref,
};
use crate::bufwrite::buf_write;
use crate::change::{change_warning, changed_lines};
use crate::charset::{ascii_isdigit, ascii_iswhite, getdigits_int, getdigits_int32, skipwhite};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{check_cursor, colnr_T, linenr_T};
use crate::decoration::decor_conceal_line;
use crate::drawscreen::{UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, kCallbackNone, tv_get_lnum, tv_get_number, typval_T, varnumber_T,
};
use crate::eval::vars::{eval_diff, eval_patch};
use crate::ex_cmds::{append_redir, ex_file};
use crate::ex_docmd::{
    ADDR_LINES, CMOD_LOCKMARKS, do_cmdline_cmd, do_exedit, exarg_T, get_address,
};
use crate::ex_getln::hlf_T;
use crate::extmark::{TriState, extmark_adjust};
use crate::fileio::{FILE, buf_check_timestamp, fclose, fwrite, vim_fgets, vim_tempname};
#[cfg(unix)]
use crate::fileio::{shorten_fnames, vim_gettempdir};
use crate::fold::{
    foldUpdate, foldUpdateAll, foldmethodIsDiff, foldmethodIsManual, hasFolding, newFoldLevel,
};
use crate::globals::*;
use crate::linematch::linematch_nbuffers;
use crate::mark::{mark_adjust, setpcmark};
use crate::mbyte::{
    mb_get_class_tab, mb_stricmp, utf_char2bytes, utf_char2len, utf_fold, utf_head_off,
    utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{ml_append, ml_delete, ml_get_buf, ml_get_buf_len};
use crate::memory::{memchrsub, strequal, xfree, xmalloc, xstrdup};
use crate::message::{emsg, semsg};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, changed_window_setting, check_topfill,
    invalidate_botline_win, validate_cursor,
};
use crate::normal::check_scrollbind;
use crate::option::{set_option_direct_for, set_option_value_give_err};
use crate::option_vars::*;
use crate::optionstr::{OptIndex, OptKeyDict_dip, free_string_option, opt_keyset};
use crate::os::env::{os_env_exists, os_unsetenv};
use crate::os::fs::{FileInfo, os_fileinfo, os_fileinfo_size, os_fopen, os_remove};
use crate::os::shell::call_shell;
use crate::strings::{
    atol, gettext, strcat, strcmp, strcpy, strlen, strncmp, tolower, vim_snprintf, vim_strchr,
    vim_strsave_shellescape, xstrnsave,
};
use crate::ui::vim_beep;
use crate::undo::{u_save, u_sync};
use crate::window::{
    WSP_VERT, frames_locked, scroll_to_fraction, set_fraction, tabpage_T, win_T, win_split,
    win_valid, window_layout_lock, window_layout_unlock,
};
use crate::xdiff::xdiffi::xdl_diff;
use crate::xdiff::{mmfile_t, xdemitcb_t, xdemitconf_t, xpparam_t};
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;
const DB_COUNT: usize = 8;
const MAX_DIFF_ANCHORS: usize = 20;
const LBUFLEN: usize = 50;
const MAX_XDIFF_SIZE: c_int = 1024 * 1024 * 1023;
const DIFF_FILLER: c_int = 0x001;
const DIFF_IBLANK: c_int = 0x002;
const DIFF_ICASE: c_int = 0x004;
const DIFF_IWHITE: c_int = 0x008;
const DIFF_IWHITEALL: c_int = 0x010;
const DIFF_IWHITEEOL: c_int = 0x020;
const DIFF_HORIZONTAL: c_int = 0x040;
const DIFF_VERTICAL: c_int = 0x080;
const DIFF_HIDDEN_OFF: c_int = 0x100;
const DIFF_INTERNAL: c_int = 0x200;
const DIFF_CLOSE_OFF: c_int = 0x400;
const DIFF_FOLLOWWRAP: c_int = 0x800;
const DIFF_LINEMATCH: c_int = 0x1000;
const DIFF_INLINE_NONE: c_int = 0x2000;
const DIFF_INLINE_SIMPLE: c_int = 0x4000;
const DIFF_INLINE_CHAR: c_int = 0x8000;
const DIFF_INLINE_WORD: c_int = 0x10000;
const DIFF_ANCHOR: c_int = 0x20000;
const ALL_WHITE_DIFF: c_int = DIFF_IWHITE | DIFF_IWHITEALL | DIFF_IWHITEEOL;
const ALL_INLINE: c_int =
    DIFF_INLINE_NONE | DIFF_INLINE_SIMPLE | DIFF_INLINE_CHAR | DIFF_INLINE_WORD;
const ALL_INLINE_DIFF: c_int = DIFF_INLINE_CHAR | DIFF_INLINE_WORD;
const XDF_NEED_MINIMAL: c_int = 1 << 0;
pub(crate) const XDF_IGNORE_WHITESPACE: c_ulong = 1 << 1;
pub(crate) const XDF_IGNORE_WHITESPACE_CHANGE: c_ulong = 1 << 2;
pub(crate) const XDF_IGNORE_WHITESPACE_AT_EOL: c_ulong = 1 << 3;
pub(crate) const XDF_IGNORE_BLANK_LINES: c_ulong = 1 << 7;
const XDF_PATIENCE_DIFF: c_int = 1 << 14;
const XDF_HISTOGRAM_DIFF: c_int = 1 << 15;
const XDF_INDENT_HEURISTIC: c_int = 1 << 23;
pub(crate) const kOptDiff: OptIndex = 70;
pub(crate) const kOptDiffopt: OptIndex = 73;
pub(crate) const kOptFoldmethod: OptIndex = 109;
const CMD_diffget: c_int = 121;
const CMD_diffput: c_int = 124;
const CMD_split: c_int = 426;
pub(crate) const kShellOptFilter: c_int = 1;
pub(crate) const kShellOptDoOut: c_int = 4;
pub(crate) const kShellOptSilent: c_int = 8;
pub(crate) const kOptBoFlagOperator: c_uint = 0x4000;
pub(crate) const HLF_ADD: hlf_T = 30;
pub(crate) const HLF_CHD: hlf_T = 31;
pub(crate) const HLF_TXD: hlf_T = 33;
pub(crate) const HLF_TXA: hlf_T = 34;
const KEYSET_OPTIDX_dip__inline_: u64 = 5;
const KEYSET_OPTIDX_dip__context: u64 = 7;
const KEYSET_OPTIDX_dip__linematch: u64 = 11;
const KEYSET_OPTIDX_dip__algorithm: u64 = 13;
const KEYSET_OPTIDX_dip__foldcolumn: u64 = 16;
#[repr(C)]
pub struct diff_T {
    pub df_id: u64,
    pub df_lnum: [linenr_T; DB_COUNT],
    pub df_count: [linenr_T; DB_COUNT],
    pub is_linematched: bool,
    pub has_changes: bool,
    pub df_changes: Vec<diffline_change_T>,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct diffline_change_T {
    pub dc_start: [colnr_T; DB_COUNT],
    pub dc_end: [colnr_T; DB_COUNT],
    pub dc_start_lnum_off: [c_int; DB_COUNT],
    pub dc_end_lnum_off: [c_int; DB_COUNT],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct diffline_T {
    pub changes: *mut diffline_change_T,
    pub num_changes: c_int,
    pub bufidx: c_int,
    pub lineoff: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct diffin_T {
    din_fname: *mut c_char,
    din_mmfile: mmfile_t,
}
#[repr(C)]
struct diffout_T {
    dout_fname: *mut c_char,
    dout_ga: Vec<diffhunk_T>,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct diffhunk_T {
    lnum_orig: linenr_T,
    count_orig: c_int,
    lnum_new: linenr_T,
    count_new: c_int,
}
#[repr(C)]
struct diffio_T {
    dio_orig: diffin_T,
    dio_new: diffin_T,
    dio_diff: diffout_T,
    dio_internal: c_int,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum diffstyle_T {
    DIFF_ED,
    DIFF_UNIFIED,
    DIFF_NONE,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct linemap_entry_T {
    byte_start: colnr_T,
    num_bytes: colnr_T,
    lineoff: c_int,
}
use crate::arglist::ML_EMPTY;
use crate::charset::{NL, NUL};
use crate::cmdexpand::MB_MAXBYTES;
use crate::extmark::{kExtmarkNOOP, kExtmarkUndo, kFalse, kNone, kTrue};
use crate::math::{FAIL, OK};
use crate::option::{OPT_LOCAL, kOptScopeWin};
#[cfg(unix)]
use crate::os::fs::MAXPATHL;
use crate::regexp::MAXCOL;
use crate::search::{BACKWARD, FORWARD};
static mut diff_busy: bool = false;
static mut diff_need_update: bool = false;
static mut diff_flags: c_int =
    DIFF_INTERNAL | DIFF_FILLER | DIFF_CLOSE_OFF | DIFF_LINEMATCH | DIFF_INLINE_CHAR;
static mut diff_algorithm: c_int = XDF_INDENT_HEURISTIC;
static mut diff_word_gap: c_int = 5;
static mut linematch_lines: c_int = 40;
static mut diff_a_works: TriState = kNone;
static mut next_diff_id: u64 = 1;
static mut diff_finger_dp_idx: Option<usize> = None;
static mut diff_finger_tp: *mut tabpage_T = ptr::null_mut();
static mut diff_finger_idx: c_int = -1;
static mut diff_finger_lnum: linenr_T = 0;
static mut simple_diffline_change: diffline_change_T = diffline_change_T {
    dc_start: [0; DB_COUNT],
    dc_end: [0; DB_COUNT],
    dc_start_lnum_off: [0; DB_COUNT],
    dc_end_lnum_off: [0; DB_COUNT],
};
static mut f_diff_hlid_prev_lnum: linenr_T = 0;
static mut f_diff_hlid_changedtick: varnumber_T = 0;
static mut f_diff_hlid_fnum: c_int = 0;
static mut f_diff_hlid_prev_diff_flags: c_int = 0;
static mut f_diff_hlid_change_start: c_int = 0;
static mut f_diff_hlid_change_end: c_int = 0;
static mut f_diff_hlid_hlID: hlf_T = 0;
#[inline]
const fn has_dip_key(v: &OptKeyDict_dip, idx: u64) -> bool {
    (v.is_set__dip_ & (1u64 << idx)) != 0
}
unsafe fn diff_scan_start(tp: *mut tabpage_T, idx: c_int, lnum: linenr_T) -> usize {
    unsafe {
        if let Some(dp_idx) = diff_finger_dp_idx
            && diff_finger_tp == tp
            && diff_finger_idx == idx
            && lnum >= diff_finger_lnum
            && dp_idx < (*tp).tp_diffs.len()
        {
            return dp_idx;
        }
        0
    }
}
unsafe fn diff_scan_remember(tp: *mut tabpage_T, idx: c_int, lnum: linenr_T, dp_idx: Option<usize>) {
    unsafe {
        diff_finger_tp = tp;
        diff_finger_idx = idx;
        diff_finger_lnum = lnum;
        diff_finger_dp_idx = Some(dp_idx.unwrap_or(0));
    }
}
pub unsafe fn diff_buf_delete(buf: *mut buf_T) {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let i = diff_buf_idx(buf, tp);
            if i != DB_COUNT as c_int {
                (*tp).tp_diffbuf[i as usize] = ptr::null_mut();
                (*tp).tp_diff_invalid = 1;
                if tp == curtab {
                    need_diff_redraw = true;
                    redraw_later(curwin, UPD_VALID);
                }
            }
            tp = (*tp).tp_next;
        }
    }
}
pub unsafe fn diff_buf_adjust(win: *mut win_T) {
    unsafe {
        if (*win).w_onebuf_opt.wo_diff == 0 {
            let mut found_win = false;
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_buffer == (*win).w_buffer && (*wp).w_onebuf_opt.wo_diff != 0 {
                    found_win = true;
                }
                wp = (*wp).w_next;
            }
            if !found_win {
                let i = diff_buf_idx((*win).w_buffer, curtab);
                if i != DB_COUNT as c_int {
                    (*curtab).tp_diffbuf[i as usize] = ptr::null_mut();
                    (*curtab).tp_diff_invalid = 1;
                    diff_redraw(true);
                }
            }
        } else {
            diff_buf_add((*win).w_buffer);
        }
    }
}
pub unsafe fn diff_buf_add(buf: *mut buf_T) {
    unsafe {
        if diff_buf_idx(buf, curtab) != DB_COUNT as c_int {
            return;
        }
        for i in 0..DB_COUNT {
            if (*curtab).tp_diffbuf[i].is_null() {
                (*curtab).tp_diffbuf[i] = buf;
                (*curtab).tp_diff_invalid = 1;
                diff_redraw(true);
                return;
            }
        }
        semsg!(
            gettext(c"E96: Cannot diff more than %d buffers".as_ptr()),
            DB_COUNT as c_int,
        );
    }
}
unsafe fn diff_buf_clear() {
    unsafe {
        for i in 0..DB_COUNT {
            if !(*curtab).tp_diffbuf[i].is_null() {
                (*curtab).tp_diffbuf[i] = ptr::null_mut();
                (*curtab).tp_diff_invalid = 1;
                diff_redraw(true);
            }
        }
    }
}
unsafe fn diff_buf_idx(buf: *mut buf_T, tp: *mut tabpage_T) -> c_int {
    unsafe {
        let mut idx: usize = 0;
        while idx < DB_COUNT {
            if (*tp).tp_diffbuf[idx] == buf {
                break;
            }
            idx += 1;
        }
        idx as c_int
    }
}
pub unsafe fn diff_invalidate(buf: *mut buf_T) {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let i = diff_buf_idx(buf, tp);
            if i != DB_COUNT as c_int {
                (*tp).tp_diff_invalid = 1;
                if tp == curtab {
                    diff_redraw(true);
                }
            }
            tp = (*tp).tp_next;
        }
    }
}
pub unsafe fn diff_mark_adjust(
    buf: *mut buf_T,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let idx = diff_buf_idx(buf, tp);
            if idx != DB_COUNT as c_int {
                diff_mark_adjust_tp(tp, idx as usize, line1, line2, amount, amount_after);
            }
            tp = (*tp).tp_next;
        }
    }
}
unsafe fn diff_mark_adjust_tp(
    tp: *mut tabpage_T,
    idx: usize,
    line1: linenr_T,
    line2: linenr_T,
    amount: linenr_T,
    amount_after: linenr_T,
) {
    unsafe {
        diff_finger_dp_idx = None;
        if diff_internal() != 0 {
            (*tp).tp_diff_invalid = 1;
            (*tp).tp_diff_update = 1;
        }
        let inserted: linenr_T;
        let mut deleted: linenr_T;
        if line2 == MAXLNUM {
            inserted = amount;
            deleted = 0;
        } else if amount_after > 0 {
            inserted = amount_after;
            deleted = 0;
        } else {
            inserted = 0;
            deleted = -amount_after;
        }
        let mut dprev_idx: Option<usize> = None;
        let mut dp_idx: usize = 0;
        let mut lnum_deleted = line1;
        loop {
            let dp_past_line2 = dp_idx >= (*tp).tp_diffs.len()
                || (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] - 1 > line2
                || (line2 == MAXLNUM && (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] > line1);
            let dprev_before_line1 = match dprev_idx {
                None => true,
                Some(pi) => {
                    (&(*tp).tp_diffs)[pi].df_lnum[idx] + (&(*tp).tp_diffs)[pi].df_count[idx] < line1
                }
            };
            if dp_past_line2 && dprev_before_line1 && !diff_busy {
                let dprev_copy = dprev_idx.map(|pi| {
                    (
                        (&(*tp).tp_diffs)[pi].df_lnum,
                        (&(*tp).tp_diffs)[pi].df_count,
                    )
                });
                let dnext_idx = diff_alloc_new(tp, dp_idx);
                dp_idx += 1;
                (&mut (*tp).tp_diffs)[dnext_idx].df_lnum[idx] = line1;
                (&mut (*tp).tp_diffs)[dnext_idx].df_count[idx] = inserted;
                for i in 0..DB_COUNT {
                    if !(*tp).tp_diffbuf[i].is_null() && i != idx {
                        (&mut (*tp).tp_diffs)[dnext_idx].df_lnum[i] = match dprev_copy {
                            None => line1,
                            Some((prev_lnum, prev_count)) => {
                                line1 + (prev_lnum[i] + prev_count[i])
                                    - (prev_lnum[idx] + prev_count[idx])
                            }
                        };
                        (&mut (*tp).tp_diffs)[dnext_idx].df_count[i] = deleted;
                    }
                }
            }
            if dp_idx >= (*tp).tp_diffs.len() {
                break;
            }
            let last =
                (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] + (&(*tp).tp_diffs)[dp_idx].df_count[idx] - 1;
            if last >= line1 - 1 {
                if diff_busy {
                    if (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] > line2 {
                        (&mut (*tp).tp_diffs)[dp_idx].df_lnum[idx] += amount_after;
                    }
                    dprev_idx = Some(dp_idx);
                    dp_idx += 1;
                    continue;
                }
                if (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] - linenr_T::from(deleted + inserted != 0)
                    > line2
                {
                    if amount_after == 0 {
                        break;
                    }
                    (&mut (*tp).tp_diffs)[dp_idx].df_lnum[idx] += amount_after;
                } else {
                    let mut check_unchanged = false;
                    if deleted > 0 {
                        let mut n: linenr_T;
                        let mut off: linenr_T = 0;
                        let next_lnum = (*tp)
                            .tp_diffs
                            .as_slice()
                            .get(dp_idx + 1)
                            .map(|nd| nd.df_lnum[idx]);
                        let dp = &mut (&mut (*tp).tp_diffs)[dp_idx];
                        if dp.df_lnum[idx] >= line1 {
                            if last <= line2 {
                                if let Some(nl) = next_lnum
                                    && nl - 1 <= line2
                                {
                                    n = nl - lnum_deleted;
                                    deleted -= n;
                                    n -= dp.df_count[idx];
                                    lnum_deleted = nl;
                                } else {
                                    n = deleted - dp.df_count[idx];
                                }
                                dp.df_count[idx] = 0;
                            } else {
                                off = dp.df_lnum[idx] - lnum_deleted;
                                n = off;
                                dp.df_count[idx] -= line2 - dp.df_lnum[idx] + 1;
                                check_unchanged = true;
                            }
                            dp.df_lnum[idx] = line1;
                        } else if last < line2 {
                            dp.df_count[idx] -= last - lnum_deleted + 1;
                            if let Some(nl) = next_lnum
                                && nl - 1 <= line2
                            {
                                n = nl - 1 - last;
                                deleted -= nl - lnum_deleted;
                                lnum_deleted = nl;
                            } else {
                                n = line2 - last;
                            }
                            check_unchanged = true;
                        } else {
                            n = 0;
                            dp.df_count[idx] -= deleted;
                        }
                        for i in 0..DB_COUNT {
                            if !(*tp).tp_diffbuf[i].is_null() && i != idx {
                                if dp.df_lnum[i] > off {
                                    dp.df_lnum[i] -= off;
                                } else {
                                    dp.df_lnum[i] = 1;
                                }
                                dp.df_count[i] += n;
                            }
                        }
                    } else if (&(*tp).tp_diffs)[dp_idx].df_lnum[idx] <= line1 {
                        (&mut (*tp).tp_diffs)[dp_idx].df_count[idx] += inserted;
                        check_unchanged = true;
                    } else {
                        (&mut (*tp).tp_diffs)[dp_idx].df_lnum[idx] += inserted;
                    }
                    if check_unchanged {
                        diff_check_unchanged(tp, dp_idx);
                    }
                }
            }
            if let Some(pi) = dprev_idx
                && !(&(*tp).tp_diffs)[dp_idx].is_linematched
                && !diff_busy
                && (&(*tp).tp_diffs)[pi].df_lnum[idx] + (&(*tp).tp_diffs)[pi].df_count[idx]
                    == (&(*tp).tp_diffs)[dp_idx].df_lnum[idx]
            {
                let dp_counts = (&(*tp).tp_diffs)[dp_idx].df_count;
                for (i, &c) in dp_counts.iter().enumerate() {
                    if !(*tp).tp_diffbuf[i].is_null() {
                        (&mut (*tp).tp_diffs)[pi].df_count[i] += c;
                    }
                }
                diff_free(tp, dp_idx);
            } else {
                dprev_idx = Some(dp_idx);
                dp_idx += 1;
            }
        }
        let mut i_dp = 0;
        while i_dp < (*tp).tp_diffs.len() {
            let mut i = 0;
            while i < DB_COUNT {
                if !(*tp).tp_diffbuf[i].is_null() && (&(*tp).tp_diffs)[i_dp].df_count[i] != 0 {
                    break;
                }
                i += 1;
            }
            if i == DB_COUNT {
                diff_free(tp, i_dp);
            } else {
                i_dp += 1;
            }
        }
        if tp == curtab {
            need_diff_redraw = true;
            diff_need_scrollbind = true;
        }
    }
}
unsafe fn diff_alloc_new(tp: *mut tabpage_T, idx: usize) -> usize {
    unsafe {
        let df_id = next_diff_id;
        next_diff_id += 1;
        diff_finger_dp_idx = None;
        (*tp).tp_diffs.insert(
            idx,
            diff_T {
                df_id,
                df_lnum: [0; DB_COUNT],
                df_count: [0; DB_COUNT],
                is_linematched: false,
                has_changes: false,
                df_changes: Vec::new(),
            },
        );
        idx
    }
}
unsafe fn diff_free(tp: *mut tabpage_T, idx: usize) {
    unsafe {
        diff_finger_dp_idx = None;
        (*tp).tp_diffs.remove(idx);
    }
}
unsafe fn diff_check_unchanged(tp: *mut tabpage_T, dp_idx: usize) {
    unsafe {
        let mut i_org = 0;
        while i_org < DB_COUNT {
            if !(*tp).tp_diffbuf[i_org].is_null() {
                break;
            }
            i_org += 1;
        }
        if i_org == DB_COUNT {
            return;
        }
        if diff_check_sanity(tp, &(&(*tp).tp_diffs)[dp_idx]) == FAIL {
            return;
        }
        let mut off_org: linenr_T = 0;
        let mut off_new: linenr_T = 0;
        let mut dir = FORWARD;
        loop {
            while (&(*tp).tp_diffs)[dp_idx].df_count[i_org] > 0 {
                if dir == BACKWARD {
                    off_org = (&(*tp).tp_diffs)[dp_idx].df_count[i_org] - 1;
                }
                let line_org = xstrdup(ml_get_buf(
                    (*tp).tp_diffbuf[i_org],
                    (&(*tp).tp_diffs)[dp_idx].df_lnum[i_org] + off_org,
                ));
                let mut i_new = i_org + 1;
                while i_new < DB_COUNT {
                    if (*tp).tp_diffbuf[i_new].is_null() {
                        i_new += 1;
                        continue;
                    }
                    if dir == BACKWARD {
                        off_new = (&(*tp).tp_diffs)[dp_idx].df_count[i_new] - 1;
                    }
                    if off_new < 0 || off_new >= (&(*tp).tp_diffs)[dp_idx].df_count[i_new] {
                        break;
                    }
                    if diff_cmp(
                        line_org,
                        ml_get_buf(
                            (*tp).tp_diffbuf[i_new],
                            (&(*tp).tp_diffs)[dp_idx].df_lnum[i_new] + off_new,
                        ),
                    ) != 0
                    {
                        break;
                    }
                    i_new += 1;
                }
                xfree(line_org.cast());
                if i_new != DB_COUNT {
                    break;
                }
                for idx in i_org..DB_COUNT {
                    if !(*tp).tp_diffbuf[idx].is_null() {
                        if dir == FORWARD {
                            (&mut (*tp).tp_diffs)[dp_idx].df_lnum[idx] += 1;
                        }
                        (&mut (*tp).tp_diffs)[dp_idx].df_count[idx] -= 1;
                    }
                }
            }
            if dir == BACKWARD {
                break;
            }
            dir = BACKWARD;
        }
    }
}
unsafe fn diff_check_sanity(tp: *mut tabpage_T, dp: *const diff_T) -> c_int {
    unsafe {
        for i in 0..DB_COUNT {
            let buf = (*tp).tp_diffbuf[i];
            if !buf.is_null()
                && (*dp).df_lnum[i] + (*dp).df_count[i] - 1 > (*buf).b_ml.ml_line_count
            {
                return FAIL;
            }
        }
        OK
    }
}
pub unsafe fn diff_redraw(dofold: bool) {
    unsafe {
        let mut wp_other: *mut win_T = ptr::null_mut();
        let mut used_max_fill_other = false;
        let mut used_max_fill_curwin = false;
        need_diff_redraw = false;
        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_onebuf_opt.wo_diff == 0 || !buf_valid((*wp).w_buffer) {
                wp = (*wp).w_next;
                continue;
            }
            redraw_later(wp, UPD_SOME_VALID);
            if wp != curwin {
                wp_other = wp;
            }
            if dofold && foldmethodIsDiff(wp) {
                foldUpdateAll(wp);
            }
            let n = diff_check_fill(wp, (*wp).w_topline);
            if (wp != curwin && (*wp).w_topfill > 0) || n > 0 {
                if (*wp).w_topfill > n {
                    (*wp).w_topfill = n.max(0);
                } else if n > 0 && n > (*wp).w_topfill {
                    (*wp).w_topfill = n;
                    if wp == curwin {
                        used_max_fill_curwin = true;
                    } else if !wp_other.is_null() {
                        used_max_fill_other = true;
                    }
                }
                check_topfill(wp, false);
            }
            wp = (*wp).w_next;
        }
        if !wp_other.is_null() && (*curwin).w_onebuf_opt.wo_scb != 0 {
            if used_max_fill_curwin {
                diff_set_topline(wp_other, curwin);
            } else if used_max_fill_other {
                diff_set_topline(curwin, wp_other);
            }
        }
    }
}
unsafe fn clear_diffin(din: *mut diffin_T) {
    unsafe {
        if (*din).din_fname.is_null() {
            xfree((*din).din_mmfile.ptr.cast());
            (*din).din_mmfile.ptr = ptr::null_mut();
        } else {
            let _ = os_remove((*din).din_fname);
        }
    }
}
unsafe fn clear_diffout(dout: *mut diffout_T) {
    unsafe {
        if (*dout).dout_fname.is_null() {
            (*dout).dout_ga = Vec::new();
        } else {
            let _ = os_remove((*dout).dout_fname);
        }
    }
}
unsafe fn diff_write_buffer(
    buf: *mut buf_T,
    m: *mut mmfile_t,
    start: linenr_T,
    mut end: linenr_T,
) -> c_int {
    unsafe {
        if end < 0 {
            end = (*buf).b_ml.ml_line_count;
        }
        if ((*buf).b_ml.ml_flags & ML_EMPTY) != 0 || end < start {
            (*m).ptr = ptr::null_mut();
            (*m).size = 0;
            return OK;
        }
        let mut len: usize = 0;
        for lnum in start..=end {
            len += ml_get_buf_len(buf, lnum) as usize + 1;
        }
        let ptr = xmalloc(len).cast::<c_char>();
        (*m).ptr = ptr;
        (*m).size = len as c_int;
        len = 0;
        for lnum in start..=end {
            let mut s = ml_get_buf(buf, lnum);
            if (diff_flags & DIFF_ICASE) != 0 {
                while *s != NUL {
                    let c: c_int;
                    let mut c_len: c_int = 1;
                    let mut cbuf: [c_char; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];
                    if *s == NL {
                        c = NUL as c_int;
                    } else {
                        let ch = utf_ptr2char(s);
                        c_len = utf_char2len(ch);
                        c = utf_fold(ch);
                    }
                    let orig_len = utfc_ptr2len(s);
                    if utf_char2bytes(c, (&raw mut cbuf).cast()) != c_len {
                        ptr::copy(s, ptr.add(len), orig_len as usize);
                    } else {
                        ptr::copy((&raw const cbuf).cast(), ptr.add(len), c_len as usize);
                        if orig_len > c_len {
                            ptr::copy(
                                s.add(c_len as usize),
                                ptr.add(len + c_len as usize),
                                (orig_len - c_len) as usize,
                            );
                        }
                    }
                    s = s.add(orig_len as usize);
                    len += orig_len as usize;
                }
            } else {
                let slen = strlen(s);
                ptr::copy(s, ptr.add(len), slen);
                memchrsub(ptr.add(len).cast(), NL, NUL, slen);
                len += slen;
            }
            *ptr.add(len) = NL;
            len += 1;
        }
        OK
    }
}
unsafe fn diff_write(
    buf: *mut buf_T,
    din: *mut diffin_T,
    start: linenr_T,
    mut end: linenr_T,
) -> c_int {
    unsafe {
        if (*din).din_fname.is_null() {
            return diff_write_buffer(buf, &raw mut (*din).din_mmfile, start, end);
        }
        if frames_locked() {
            return FAIL;
        }
        if end < 0 {
            end = (*buf).b_ml.ml_line_count;
        }
        let save_ml_flags = (*buf).b_ml.ml_flags;
        let save_ff = (*buf).b_p_ff;
        (*buf).b_p_ff = xstrdup(c"unix".as_ptr());
        let save_cmod_flags = cmdmod.cmod_flags != 0;
        cmdmod.cmod_flags |= CMOD_LOCKMARKS;
        if end < start {
            end = start;
            (*buf).b_ml.ml_flags |= ML_EMPTY;
        }
        let r = buf_write(
            buf,
            (*din).din_fname,
            ptr::null_mut(),
            start,
            end,
            ptr::null_mut(),
            false,
            false,
            false,
            true,
        );
        cmdmod.cmod_flags = c_int::from(save_cmod_flags);
        free_string_option((*buf).b_p_ff);
        (*buf).b_p_ff = save_ff;
        (*buf).b_ml.ml_flags = ((*buf).b_ml.ml_flags & !ML_EMPTY) | (save_ml_flags & ML_EMPTY);
        r
    }
}
unsafe fn diff_try_update(dio: *mut diffio_T, idx_orig: usize, eap: *mut exarg_T) {
    unsafe {
        'theend: {
            if (*dio).dio_internal != 0 {
                (*dio).dio_diff.dout_ga.clear();
            } else {
                (*dio).dio_orig.din_fname = vim_tempname();
                (*dio).dio_new.din_fname = vim_tempname();
                (*dio).dio_diff.dout_fname = vim_tempname();
                if (*dio).dio_orig.din_fname.is_null()
                    || (*dio).dio_new.din_fname.is_null()
                    || (*dio).dio_diff.dout_fname.is_null()
                {
                    break 'theend;
                }
                if check_external_diff(dio) == FAIL {
                    break 'theend;
                }
            }
            if !eap.is_null() && (*eap).forceit != 0 {
                for idx_new in idx_orig..DB_COUNT {
                    let buf = (*curtab).tp_diffbuf[idx_new];
                    if buf_valid(buf) {
                        let _ = buf_check_timestamp(buf);
                    }
                }
            }
            let mut num_anchors: usize = usize::MAX;
            let mut anchors: [[linenr_T; MAX_DIFF_ANCHORS]; DB_COUNT] =
                [[0; MAX_DIFF_ANCHORS]; DB_COUNT];
            if (diff_flags & DIFF_ANCHOR) != 0 {
                for idx in 0..DB_COUNT {
                    if (*curtab).tp_diffbuf[idx].is_null() {
                        continue;
                    }
                    let mut buf_num_anchors: c_int = 0;
                    if parse_diffanchors(
                        false,
                        (*curtab).tp_diffbuf[idx],
                        (&raw mut anchors[idx]).cast(),
                        &raw mut buf_num_anchors,
                    ) != OK
                    {
                        emsg(gettext(
                            (&raw const e_failed_to_find_all_diff_anchors).cast(),
                        ));
                        num_anchors = 0;
                        anchors = [[0; MAX_DIFF_ANCHORS]; DB_COUNT];
                        break;
                    }
                    if (buf_num_anchors as usize) < num_anchors {
                        num_anchors = buf_num_anchors as usize;
                    }
                    if buf_num_anchors > 0 {
                        anchors[idx][..buf_num_anchors as usize].sort_unstable();
                    }
                }
            }
            if num_anchors == usize::MAX {
                num_anchors = 0;
            }
            for anchor_i in 0..=num_anchors {
                let mut orig_diff: Vec<diff_T> = Vec::new();
                if anchor_i != 0 {
                    orig_diff = std::mem::take(&mut (*curtab).tp_diffs);
                }
                let mut lnum_start = if anchor_i == 0 {
                    1
                } else {
                    anchors[idx_orig][anchor_i - 1]
                };
                let mut lnum_end = if anchor_i == num_anchors {
                    -1
                } else {
                    anchors[idx_orig][anchor_i] - 1
                };
                let mut buf = (*curtab).tp_diffbuf[idx_orig];
                if diff_write(buf, &raw mut (*dio).dio_orig, lnum_start, lnum_end) == FAIL {
                    diff_clear(curtab);
                    break 'theend;
                }
                for (idx_new, anchor_row) in
                    anchors.iter().enumerate().take(DB_COUNT).skip(idx_orig + 1)
                {
                    buf = (*curtab).tp_diffbuf[idx_new];
                    if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
                        continue;
                    }
                    lnum_start = if anchor_i == 0 {
                        1
                    } else {
                        anchor_row[anchor_i - 1]
                    };
                    lnum_end = if anchor_i == num_anchors {
                        -1
                    } else {
                        anchor_row[anchor_i] - 1
                    };
                    if diff_write(buf, &raw mut (*dio).dio_new, lnum_start, lnum_end) == FAIL {
                        continue;
                    }
                    if diff_file(dio) == FAIL {
                        continue;
                    }
                    diff_read(idx_orig, idx_new, dio);
                    clear_diffin(&raw mut (*dio).dio_new);
                    clear_diffout(&raw mut (*dio).dio_diff);
                }
                clear_diffin(&raw mut (*dio).dio_orig);
                if anchor_i != 0 {
                    diff_finger_dp_idx = None;
                    for dp in &mut (*curtab).tp_diffs {
                        for (idx, anchor_row) in anchors.iter().enumerate() {
                            if anchor_row[anchor_i - 1] > 0 {
                                dp.df_lnum[idx] += anchor_row[anchor_i - 1] - 1;
                            }
                        }
                    }
                    if !orig_diff.is_empty() {
                        orig_diff.append(&mut (*curtab).tp_diffs);
                        (*curtab).tp_diffs = orig_diff;
                    }
                }
            }
        }
        xfree((*dio).dio_orig.din_fname.cast());
        xfree((*dio).dio_new.din_fname.cast());
        xfree((*dio).dio_diff.dout_fname.cast());
    }
}
pub unsafe fn diff_internal() -> c_int {
    unsafe { c_int::from((diff_flags & DIFF_INTERNAL) != 0 && p_dex.cb_type() == kCallbackNone) }
}
pub unsafe fn ex_diffupdate(eap: *mut exarg_T) {
    unsafe {
        if diff_busy {
            diff_need_update = true;
            return;
        }
        let had_diffs = !(*curtab).tp_diffs.is_empty();
        diff_clear(curtab);
        (*curtab).tp_diff_invalid = 0;
        'theend: {
            let mut idx_orig = 0;
            while idx_orig < DB_COUNT {
                if !(*curtab).tp_diffbuf[idx_orig].is_null() {
                    break;
                }
                idx_orig += 1;
            }
            if idx_orig == DB_COUNT {
                break 'theend;
            }
            let mut idx_new = idx_orig + 1;
            while idx_new < DB_COUNT {
                if !(*curtab).tp_diffbuf[idx_new].is_null() {
                    break;
                }
                idx_new += 1;
            }
            if idx_new == DB_COUNT {
                break 'theend;
            }
            let mut diffio = diffio_T {
                dio_orig: diffin_T {
                    din_fname: ptr::null_mut(),
                    din_mmfile: mmfile_t {
                        ptr: ptr::null_mut(),
                        size: 0,
                    },
                },
                dio_new: diffin_T {
                    din_fname: ptr::null_mut(),
                    din_mmfile: mmfile_t {
                        ptr: ptr::null_mut(),
                        size: 0,
                    },
                },
                dio_diff: diffout_T {
                    dout_fname: ptr::null_mut(),
                    dout_ga: Vec::new(),
                },
                dio_internal: diff_internal(),
            };
            diff_try_update(&raw mut diffio, idx_orig, eap);
            (*curwin).w_valid_cursor.lnum = 0;
        }
        if had_diffs || !(*curtab).tp_diffs.is_empty() {
            diff_redraw(true);
            window_layout_lock();
            apply_autocmds(
                EVENT_DIFFUPDATED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            window_layout_unlock();
        }
    }
}
unsafe fn check_external_diff(diffio: *mut diffio_T) -> c_int {
    unsafe {
        let mut io_error = false;
        let mut ok: TriState;
        loop {
            ok = kFalse;
            let mut fd = os_fopen((*diffio).dio_orig.din_fname, c"w".as_ptr());
            if fd.is_null() {
                io_error = true;
            } else {
                if fwrite(c"line1\n".as_ptr().cast(), 6, 1, fd) != 1 {
                    io_error = true;
                }
                fclose(fd);
                fd = os_fopen((*diffio).dio_new.din_fname, c"w".as_ptr());
                if fd.is_null() {
                    io_error = true;
                } else {
                    if fwrite(c"line2\n".as_ptr().cast(), 6, 1, fd) != 1 {
                        io_error = true;
                    }
                    fclose(fd);
                    fd = if diff_file(diffio) == OK {
                        os_fopen((*diffio).dio_diff.dout_fname, c"r".as_ptr())
                    } else {
                        ptr::null_mut()
                    };
                    if fd.is_null() {
                        io_error = true;
                    } else {
                        let mut linebuf: [c_char; LBUFLEN] = [0; LBUFLEN];
                        loop {
                            if vim_fgets((&raw mut linebuf).cast(), LBUFLEN as c_int, fd) {
                                break;
                            }
                            if strncmp((&raw const linebuf).cast(), c"1c1".as_ptr(), 3) == 0
                                || strncmp((&raw const linebuf).cast(), c"@@ -1 +1 @@".as_ptr(), 11)
                                    == 0
                            {
                                ok = kTrue;
                            }
                        }
                        fclose(fd);
                    }
                    let _ = os_remove((*diffio).dio_diff.dout_fname);
                    let _ = os_remove((*diffio).dio_new.din_fname);
                }
                let _ = os_remove((*diffio).dio_orig.din_fname);
            }
            if p_dex.cb_type() != kCallbackNone {
                break;
            }
            if diff_a_works != kNone {
                break;
            }
            diff_a_works = ok;
            if ok != 0 {
                break;
            }
        }
        if ok == 0 {
            if io_error {
                emsg(gettext(c"E810: Cannot read or write temp files".as_ptr()));
            }
            emsg(gettext(c"E97: Cannot create diffs".as_ptr()));
            diff_a_works = kNone;
            return FAIL;
        }
        OK
    }
}
unsafe fn diff_file_internal(diffio: *mut diffio_T) -> c_int {
    unsafe {
        let mut param = xpparam_t {
            flags: diff_algorithm as c_ulong,
        };
        let emit_cfg = xdemitconf_t {
            ctxlen: 0,
            interhunkctxlen: 0,
            flags: 0,
            hunk_func: Some(xdiff_out),
        };
        let mut emit_cb = xdemitcb_t {
            priv_data: (&raw mut (*diffio).dio_diff).cast(),
            out_hunk: None,
            out_line: None,
        };
        if (diff_flags & DIFF_IWHITE) != 0 {
            param.flags |= XDF_IGNORE_WHITESPACE_CHANGE;
        }
        if (diff_flags & DIFF_IWHITEALL) != 0 {
            param.flags |= XDF_IGNORE_WHITESPACE;
        }
        if (diff_flags & DIFF_IWHITEEOL) != 0 {
            param.flags |= XDF_IGNORE_WHITESPACE_AT_EOL;
        }
        if (diff_flags & DIFF_IBLANK) != 0 {
            param.flags |= XDF_IGNORE_BLANK_LINES;
        }
        if (*diffio).dio_orig.din_mmfile.size > MAX_XDIFF_SIZE
            || (*diffio).dio_new.din_mmfile.size > MAX_XDIFF_SIZE
        {
            emsg(gettext(
                (&raw const e_problem_creating_internal_diff).cast(),
            ));
            return FAIL;
        }
        if xdl_diff(
            &raw mut (*diffio).dio_orig.din_mmfile,
            &raw mut (*diffio).dio_new.din_mmfile,
            &raw const param,
            &raw const emit_cfg,
            &raw mut emit_cb,
        ) < 0
        {
            emsg(gettext(
                (&raw const e_problem_creating_internal_diff).cast(),
            ));
            return FAIL;
        }
        OK
    }
}
unsafe fn diff_file(dio: *mut diffio_T) -> c_int {
    unsafe {
        let tmp_orig = (*dio).dio_orig.din_fname;
        let tmp_new = (*dio).dio_new.din_fname;
        let tmp_diff = (*dio).dio_diff.dout_fname;
        if p_dex.cb_type() != kCallbackNone {
            eval_diff(tmp_orig, tmp_new, tmp_diff);
            return OK;
        }
        if (*dio).dio_internal != 0 {
            return diff_file_internal(dio);
        }
        let len = strlen(tmp_orig) + strlen(tmp_new) + strlen(tmp_diff) + strlen(p_srr) + 27;
        let cmd = xmalloc(len).cast::<c_char>();
        if os_env_exists(c"DIFF_OPTIONS".as_ptr(), true) {
            let _ = os_unsetenv(c"DIFF_OPTIONS".as_ptr());
        }
        vim_snprintf!(
            cmd,
            len,
            c"diff %s%s%s%s%s%s%s%s %s".as_ptr(),
            if diff_a_works == kFalse {
                c"".as_ptr()
            } else {
                c"-a ".as_ptr()
            },
            c"".as_ptr(),
            if (diff_flags & DIFF_IWHITE) != 0 {
                c"-b ".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (diff_flags & DIFF_IWHITEALL) != 0 {
                c"-w ".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (diff_flags & DIFF_IWHITEEOL) != 0 {
                c"-Z ".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (diff_flags & DIFF_IBLANK) != 0 {
                c"-B ".as_ptr()
            } else {
                c"".as_ptr()
            },
            if (diff_flags & DIFF_ICASE) != 0 {
                c"-i ".as_ptr()
            } else {
                c"".as_ptr()
            },
            tmp_orig,
            tmp_new,
        );
        append_redir(cmd, len, p_srr, tmp_diff);
        crate::autocmd::block_autocmds();
        call_shell(
            cmd,
            kShellOptFilter | kShellOptSilent | kShellOptDoOut,
            ptr::null_mut(),
        );
        crate::autocmd::unblock_autocmds();
        xfree(cmd.cast());
        OK
    }
}
pub unsafe fn ex_diffpatch(eap: *mut exarg_T) {
    unsafe {
        let mut buf: *mut c_char = ptr::null_mut();
        let old_curwin = curwin;
        let mut newname: *mut c_char = ptr::null_mut();
        let mut esc_name: *mut c_char = ptr::null_mut();
        #[cfg(unix)]
        let mut fullname: *mut c_char = ptr::null_mut();
        let tmp_orig = vim_tempname();
        let tmp_new = vim_tempname();
        'theend: {
            if tmp_orig.is_null() || tmp_new.is_null() {
                break 'theend;
            }
            if buf_write(
                curbuf,
                tmp_orig,
                ptr::null_mut(),
                1,
                (*curbuf).b_ml.ml_line_count,
                ptr::null_mut(),
                false,
                false,
                false,
                true,
            ) == FAIL
            {
                break 'theend;
            }
            #[cfg(unix)]
            {
                fullname = crate::path::FullName_save((*eap).arg, false);
                esc_name = vim_strsave_shellescape(
                    if !fullname.is_null() {
                        fullname
                    } else {
                        (*eap).arg
                    },
                    true,
                    true,
                );
            }
            #[cfg(not(unix))]
            {
                esc_name = vim_strsave_shellescape((*eap).arg, true, true);
            }
            let buflen = strlen(tmp_orig) + strlen(esc_name) + strlen(tmp_new) + 16;
            buf = xmalloc(buflen).cast::<c_char>();
            #[cfg(unix)]
            let mut dirbuf: [c_char; MAXPATHL] = [0; MAXPATHL];
            #[cfg(unix)]
            {
                if crate::os::fs::os_dirname((&raw mut dirbuf).cast(), MAXPATHL) != OK
                    || crate::os::fs::os_chdir((&raw const dirbuf).cast()) != 0
                {
                    dirbuf[0] = NUL;
                } else {
                    let mut tempdir: *const c_char = vim_gettempdir();
                    if tempdir.is_null() {
                        tempdir = c"/tmp".as_ptr();
                    }
                    let _ = crate::os::fs::os_chdir(tempdir);
                    shorten_fnames(1);
                }
            }
            if p_pex.cb_type() != kCallbackNone {
                #[cfg(unix)]
                eval_patch(
                    tmp_orig,
                    if !fullname.is_null() {
                        fullname
                    } else {
                        (*eap).arg
                    },
                    tmp_new,
                );
                #[cfg(not(unix))]
                eval_patch(tmp_orig, (*eap).arg, tmp_new);
            } else {
                vim_snprintf!(
                    buf,
                    buflen,
                    c"patch -o %s %s < %s".as_ptr(),
                    tmp_new,
                    tmp_orig,
                    esc_name,
                );
                crate::autocmd::block_autocmds();
                call_shell(buf, kShellOptFilter, ptr::null_mut());
                crate::autocmd::unblock_autocmds();
            }
            #[cfg(unix)]
            if dirbuf[0] != NUL {
                if crate::os::fs::os_chdir((&raw const dirbuf).cast()) != 0 {
                    emsg(gettext((&raw const e_prev_dir).cast()));
                }
                shorten_fnames(1);
            }
            strcpy(buf, tmp_new);
            strcat(buf, c".orig".as_ptr());
            let _ = os_remove(buf);
            strcpy(buf, tmp_new);
            strcat(buf, c".rej".as_ptr());
            let _ = os_remove(buf);
            let mut file_info = FileInfo::new();
            let info_ok = os_fileinfo(tmp_new, &raw mut file_info);
            let filesize = os_fileinfo_size(&raw const file_info);
            if !info_ok || filesize == 0 {
                emsg(gettext(c"E816: Cannot read patch output".as_ptr()));
            } else {
                if !(*curbuf).b_fname.is_null() {
                    newname = xstrnsave((*curbuf).b_fname, strlen((*curbuf).b_fname) + 4);
                    strcat(newname, c".new".as_ptr());
                }
                cmdmod.cmod_tab = 0;
                if win_split(
                    0,
                    if (diff_flags & DIFF_VERTICAL) != 0 {
                        WSP_VERT
                    } else {
                        0
                    },
                ) != FAIL
                {
                    (*eap).cmdidx = CMD_split;
                    (*eap).arg = tmp_new;
                    do_exedit(eap, old_curwin);
                    if curwin != old_curwin && win_valid(old_curwin) {
                        diff_win_options(curwin, true);
                        diff_win_options(old_curwin, true);
                        if !newname.is_null() {
                            (*eap).arg = newname;
                            ex_file(eap);
                            if crate::autocmd::augroup_exists(c"filetypedetect".as_ptr()) {
                                do_cmdline_cmd(c":doau filetypedetect BufRead".as_ptr());
                            }
                        }
                    }
                }
            }
        }
        if !tmp_orig.is_null() {
            let _ = os_remove(tmp_orig);
        }
        xfree(tmp_orig.cast());
        if !tmp_new.is_null() {
            let _ = os_remove(tmp_new);
        }
        xfree(tmp_new.cast());
        xfree(newname.cast());
        xfree(buf.cast());
        #[cfg(unix)]
        xfree(fullname.cast());
        xfree(esc_name.cast());
    }
}
pub unsafe fn ex_diffsplit(eap: *mut exarg_T) {
    unsafe {
        let old_curwin = curwin;
        let mut old_curbuf = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&raw mut old_curbuf, curbuf);
        validate_cursor(curwin);
        set_fraction(curwin);
        cmdmod.cmod_tab = 0;
        if win_split(
            0,
            if (diff_flags & DIFF_VERTICAL) != 0 {
                WSP_VERT
            } else {
                0
            },
        ) == FAIL
        {
            return;
        }
        (*eap).cmdidx = CMD_split;
        (*curwin).w_onebuf_opt.wo_diff = 1;
        do_exedit(eap, old_curwin);
        if curwin == old_curwin {
            return;
        }
        diff_win_options(curwin, true);
        if win_valid(old_curwin) {
            diff_win_options(old_curwin, true);
            if bufref_valid(&raw mut old_curbuf) {
                (*curwin).w_cursor.lnum = diff_get_corresponding_line(
                    old_curbuf.br_buf.cast(),
                    (*old_curwin).w_cursor.lnum,
                );
            }
        }
        scroll_to_fraction(curwin, (*curwin).w_height);
    }
}
pub unsafe fn ex_diffthis(_eap: *mut exarg_T) {
    unsafe {
        diff_win_options(curwin, true);
    }
}
unsafe fn set_diff_option(wp: *mut win_T, value: bool) {
    unsafe {
        let old_curwin = curwin;
        curwin = wp;
        curbuf = (*curwin).w_buffer;
        (*curbuf).b_ro_locked += 1;
        set_option_value_give_err(kOptDiff, boolean_obj(value), OPT_LOCAL);
        (*curbuf).b_ro_locked -= 1;
        curwin = old_curwin;
        curbuf = (*curwin).w_buffer;
    }
}
pub unsafe fn diff_win_options(wp: *mut win_T, addbuf: bool) {
    unsafe {
        let old_curwin = curwin;
        curwin = wp;
        newFoldLevel();
        curwin = old_curwin;
        if (*wp).w_onebuf_opt.wo_diff == 0 {
            (*wp).w_onebuf_opt.wo_scb_save = (*wp).w_onebuf_opt.wo_scb;
        }
        (*wp).w_onebuf_opt.wo_scb = 1;
        if (*wp).w_onebuf_opt.wo_diff == 0 {
            (*wp).w_onebuf_opt.wo_crb_save = (*wp).w_onebuf_opt.wo_crb;
        }
        (*wp).w_onebuf_opt.wo_crb = 1;
        if (diff_flags & DIFF_FOLLOWWRAP) == 0 {
            if (*wp).w_onebuf_opt.wo_diff == 0 {
                (*wp).w_onebuf_opt.wo_wrap_save = (*wp).w_onebuf_opt.wo_wrap;
            }
            (*wp).w_onebuf_opt.wo_wrap = 0;
            (*wp).w_skipcol = 0;
        }
        if (*wp).w_onebuf_opt.wo_diff == 0 {
            if (*wp).w_onebuf_opt.wo_diff_saved != 0 {
                free_string_option((*wp).w_onebuf_opt.wo_fdm_save);
            }
            (*wp).w_onebuf_opt.wo_fdm_save = xstrdup((*wp).w_onebuf_opt.wo_fdm);
        }
        set_option_direct_for(
            kOptFoldmethod,
            static_cstr_as_obj(c"diff"),
            OPT_LOCAL,
            0,
            kOptScopeWin,
            wp.cast(),
        );
        if (*wp).w_onebuf_opt.wo_diff == 0 {
            (*wp).w_onebuf_opt.wo_fen_save = (*wp).w_onebuf_opt.wo_fen;
            (*wp).w_onebuf_opt.wo_fdl_save = (*wp).w_onebuf_opt.wo_fdl;
            if (*wp).w_onebuf_opt.wo_diff_saved != 0 {
                free_string_option((*wp).w_onebuf_opt.wo_fdc_save);
            }
            (*wp).w_onebuf_opt.wo_fdc_save = xstrdup((*wp).w_onebuf_opt.wo_fdc);
        }
        free_string_option((*wp).w_onebuf_opt.wo_fdc);
        (*wp).w_onebuf_opt.wo_fdc = xstrdup(c"2".as_ptr());
        debug_assert!(diff_foldcolumn >= 0 && diff_foldcolumn <= 9);
        vim_snprintf!(
            (*wp).w_onebuf_opt.wo_fdc,
            strlen((*wp).w_onebuf_opt.wo_fdc) + 1,
            c"%d".as_ptr(),
            diff_foldcolumn,
        );
        (*wp).w_onebuf_opt.wo_fen = 1;
        (*wp).w_onebuf_opt.wo_fdl = 0;
        foldUpdateAll(wp);
        changed_window_setting(wp);
        if vim_strchr(p_sbo, b'h' as c_int).is_null() {
            do_cmdline_cmd(c"set sbo+=hor".as_ptr());
        }
        (*wp).w_onebuf_opt.wo_diff_saved = 1;
        set_diff_option(wp, true);
        if addbuf {
            diff_buf_add((*wp).w_buffer);
        }
        redraw_later(wp, UPD_NOT_VALID);
    }
}
pub unsafe fn ex_diffoff(eap: *mut exarg_T) {
    unsafe {
        let mut diffwin = false;
        let mut wp = firstwin;
        while !wp.is_null() {
            let cond = if (*eap).forceit != 0 {
                (*wp).w_onebuf_opt.wo_diff != 0
            } else {
                wp == curwin
            };
            if cond {
                set_diff_option(wp, false);
                if (*wp).w_onebuf_opt.wo_diff_saved != 0 {
                    if (*wp).w_onebuf_opt.wo_scb != 0 {
                        (*wp).w_onebuf_opt.wo_scb = (*wp).w_onebuf_opt.wo_scb_save;
                    }
                    if (*wp).w_onebuf_opt.wo_crb != 0 {
                        (*wp).w_onebuf_opt.wo_crb = (*wp).w_onebuf_opt.wo_crb_save;
                    }
                    if (diff_flags & DIFF_FOLLOWWRAP) == 0
                        && (*wp).w_onebuf_opt.wo_wrap == 0
                        && (*wp).w_onebuf_opt.wo_wrap_save != 0
                    {
                        (*wp).w_onebuf_opt.wo_wrap = 1;
                        (*wp).w_leftcol = 0;
                    }
                    free_string_option((*wp).w_onebuf_opt.wo_fdm);
                    (*wp).w_onebuf_opt.wo_fdm =
                        xstrdup(if *(*wp).w_onebuf_opt.wo_fdm_save != NUL {
                            (*wp).w_onebuf_opt.wo_fdm_save
                        } else {
                            c"manual".as_ptr()
                        });
                    free_string_option((*wp).w_onebuf_opt.wo_fdc);
                    (*wp).w_onebuf_opt.wo_fdc =
                        xstrdup(if *(*wp).w_onebuf_opt.wo_fdc_save != NUL {
                            (*wp).w_onebuf_opt.wo_fdc_save
                        } else {
                            c"0".as_ptr()
                        });
                    if (*wp).w_onebuf_opt.wo_fdl == 0 {
                        (*wp).w_onebuf_opt.wo_fdl = (*wp).w_onebuf_opt.wo_fdl_save;
                    }
                    if (*wp).w_onebuf_opt.wo_fen != 0 {
                        (*wp).w_onebuf_opt.wo_fen = if foldmethodIsManual(wp) {
                            0
                        } else {
                            (*wp).w_onebuf_opt.wo_fen_save
                        };
                    }
                    foldUpdateAll(wp);
                }
                (*wp).w_topfill = 0;
                changed_window_setting(wp);
                diff_buf_adjust(wp);
            }
            diffwin |= (*wp).w_onebuf_opt.wo_diff != 0;
            wp = (*wp).w_next;
        }
        if (*eap).forceit != 0 {
            diff_buf_clear();
        }
        if !diffwin {
            diff_need_update = false;
            (*curtab).tp_diff_invalid = 0;
            (*curtab).tp_diff_update = 0;
            diff_clear(curtab);
        }
        if !diffwin && !vim_strchr(p_sbo, b'h' as c_int).is_null() {
            do_cmdline_cmd(c"set sbo-=hor".as_ptr());
        }
    }
}
unsafe fn extract_hunk_internal(
    dout: *mut diffout_T,
    hunk: *mut diffhunk_T,
    line_idx: *mut c_int,
) -> bool {
    unsafe {
        let eof = *line_idx >= (*dout).dout_ga.len() as c_int;
        if !eof {
            *hunk = (&(*dout).dout_ga)[*line_idx as usize];
            *line_idx += 1;
        }
        eof
    }
}
unsafe fn extract_hunk(fd: *mut FILE, hunk: *mut diffhunk_T, diffstyle: *mut diffstyle_T) -> bool {
    unsafe {
        loop {
            let mut line: [c_char; LBUFLEN] = [0; LBUFLEN];
            if vim_fgets((&raw mut line).cast(), LBUFLEN as c_int, fd) {
                return true;
            }
            if *diffstyle == diffstyle_T::DIFF_NONE {
                if ascii_isdigit(line[0]) {
                    *diffstyle = diffstyle_T::DIFF_ED;
                } else if strncmp((&raw const line).cast(), c"@@ ".as_ptr(), 3) == 0
                    || (strncmp((&raw const line).cast(), c"--- ".as_ptr(), 4) == 0
                        && !vim_fgets((&raw mut line).cast(), LBUFLEN as c_int, fd)
                        && strncmp((&raw const line).cast(), c"+++ ".as_ptr(), 4) == 0
                        && !vim_fgets((&raw mut line).cast(), LBUFLEN as c_int, fd)
                        && strncmp((&raw const line).cast(), c"@@ ".as_ptr(), 3) == 0)
                {
                    *diffstyle = diffstyle_T::DIFF_UNIFIED;
                } else {
                    continue;
                }
            }
            if *diffstyle == diffstyle_T::DIFF_ED {
                if !ascii_isdigit(line[0]) {
                    continue;
                }
                if parse_diff_ed((&raw mut line).cast(), hunk) == FAIL {
                    continue;
                }
            } else {
                debug_assert!(*diffstyle == diffstyle_T::DIFF_UNIFIED);
                if strncmp((&raw const line).cast(), c"@@ ".as_ptr(), 3) != 0 {
                    continue;
                }
                if parse_diff_unified((&raw mut line).cast(), hunk) == FAIL {
                    continue;
                }
            }
            return false;
        }
    }
}
unsafe fn process_hunk(
    dp_idx_p: &mut usize,
    idx_orig: usize,
    idx_new: usize,
    hunk: *const diffhunk_T,
    notsetp: *mut bool,
) {
    unsafe {
        let mut dp_idx = *dp_idx_p;
        while dp_idx < (*curtab).tp_diffs.len()
            && (*hunk).lnum_orig
                > (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_orig]
                    + (&(*curtab).tp_diffs)[dp_idx].df_count[idx_orig]
        {
            if *notsetp {
                diff_copy_entry(dp_idx, idx_orig, idx_new);
            }
            dp_idx += 1;
            *notsetp = true;
        }
        if dp_idx < (*curtab).tp_diffs.len()
            && (*hunk).lnum_orig
                <= (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_orig]
                    + (&(*curtab).tp_diffs)[dp_idx].df_count[idx_orig]
            && (*hunk).lnum_orig + (*hunk).count_orig
                >= (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_orig]
        {
            let mut dpl_idx = dp_idx;
            while dpl_idx + 1 < (*curtab).tp_diffs.len() {
                if (*hunk).lnum_orig + (*hunk).count_orig
                    < (&(*curtab).tp_diffs)[dpl_idx + 1].df_lnum[idx_orig]
                {
                    break;
                }
                dpl_idx += 1;
            }
            let dpl_lnum = (&(*curtab).tp_diffs)[dpl_idx].df_lnum;
            let dpl_count = (&(*curtab).tp_diffs)[dpl_idx].df_count;
            let dp = &mut (&mut (*curtab).tp_diffs)[dp_idx];
            let mut off = dp.df_lnum[idx_orig] - (*hunk).lnum_orig;
            if off > 0 {
                for i in idx_orig..idx_new {
                    if !(*curtab).tp_diffbuf[i].is_null() {
                        dp.df_lnum[i] -= off;
                        dp.df_count[i] += off;
                    }
                }
                dp.df_lnum[idx_new] = (*hunk).lnum_new;
                dp.df_count[idx_new] = (*hunk).count_new;
            } else if *notsetp {
                dp.df_lnum[idx_new] = (*hunk).lnum_new + off;
                dp.df_count[idx_new] = (*hunk).count_new - off;
            } else {
                let orig_size_in_dp = (*hunk)
                    .count_orig
                    .min(dp.df_lnum[idx_orig] + dp.df_count[idx_orig] - (*hunk).lnum_orig);
                let size_diff = (*hunk).count_new - orig_size_in_dp;
                dp.df_count[idx_new] += size_diff;
                off = (*hunk).lnum_new + (*hunk).count_new
                    - (dp.df_lnum[idx_new] + dp.df_count[idx_new]);
                if off > 0 {
                    dp.df_count[idx_new] += off;
                }
            }
            off = ((*hunk).lnum_orig + (*hunk).count_orig)
                - (dpl_lnum[idx_orig] + dpl_count[idx_orig]);
            if off < 0 {
                if *notsetp || dp_idx != dpl_idx {
                    dp.df_count[idx_new] += -off;
                }
                off = 0;
            }
            for i in idx_orig..idx_new {
                if !(*curtab).tp_diffbuf[i].is_null() {
                    dp.df_count[i] = dpl_lnum[i] + dpl_count[i] - dp.df_lnum[i] + off;
                }
            }
            if dpl_idx > dp_idx {
                diff_finger_dp_idx = None;
                (*curtab).tp_diffs.drain((dp_idx + 1)..=dpl_idx);
            }
        } else {
            dp_idx = diff_alloc_new(curtab, dp_idx);
            (&mut (*curtab).tp_diffs)[dp_idx].df_lnum[idx_orig] = (*hunk).lnum_orig;
            (&mut (*curtab).tp_diffs)[dp_idx].df_count[idx_orig] = (*hunk).count_orig;
            (&mut (*curtab).tp_diffs)[dp_idx].df_lnum[idx_new] = (*hunk).lnum_new;
            (&mut (*curtab).tp_diffs)[dp_idx].df_count[idx_new] = (*hunk).count_new;
            for i in (idx_orig + 1)..idx_new {
                if !(*curtab).tp_diffbuf[i].is_null() {
                    diff_copy_entry(dp_idx, idx_orig, i);
                }
            }
        }
        *notsetp = false;
        *dp_idx_p = dp_idx;
    }
}
unsafe fn diff_read(idx_orig: usize, idx_new: usize, dio: *mut diffio_T) {
    unsafe {
        let mut fd: *mut FILE = ptr::null_mut();
        let mut line_hunk_idx: c_int = 0;
        let mut dp_idx: usize = 0;
        let dout = &raw mut (*dio).dio_diff;
        let mut notset = true;
        let mut diffstyle = diffstyle_T::DIFF_NONE;
        if (*dio).dio_internal == 0 {
            fd = os_fopen((*dout).dout_fname, c"r".as_ptr());
            if fd.is_null() {
                emsg(gettext(c"E98: Cannot read diff output".as_ptr()));
                return;
            }
        }
        loop {
            let mut hunk = diffhunk_T {
                lnum_orig: 0,
                count_orig: 0,
                lnum_new: 0,
                count_new: 0,
            };
            let eof = if (*dio).dio_internal != 0 {
                extract_hunk_internal(dout, &raw mut hunk, &raw mut line_hunk_idx)
            } else {
                extract_hunk(fd, &raw mut hunk, &raw mut diffstyle)
            };
            if eof {
                break;
            }
            process_hunk(
                &mut dp_idx,
                idx_orig,
                idx_new,
                &raw const hunk,
                &raw mut notset,
            );
        }
        while dp_idx < (*curtab).tp_diffs.len() {
            if notset {
                diff_copy_entry(dp_idx, idx_orig, idx_new);
            }
            dp_idx += 1;
            notset = true;
        }
        if !fd.is_null() {
            fclose(fd);
        }
    }
}
unsafe fn diff_copy_entry(dp_idx: usize, idx_orig: usize, idx_new: usize) {
    unsafe {
        let off = if dp_idx == 0 {
            0
        } else {
            let dprev = &(&(*curtab).tp_diffs)[dp_idx - 1];
            (dprev.df_lnum[idx_orig] + dprev.df_count[idx_orig])
                - (dprev.df_lnum[idx_new] + dprev.df_count[idx_new])
        };
        let dp = &mut (&mut (*curtab).tp_diffs)[dp_idx];
        dp.df_lnum[idx_new] = dp.df_lnum[idx_orig] - off;
        dp.df_count[idx_new] = dp.df_count[idx_orig];
    }
}
pub unsafe fn diff_clear(tp: *mut tabpage_T) {
    unsafe {
        diff_finger_dp_idx = None;
        (*tp).tp_diffs.clear();
    }
}
pub unsafe fn diff_linematch(dp: *const diff_T) -> bool {
    unsafe {
        if (diff_flags & DIFF_LINEMATCH) == 0 {
            return false;
        }
        let mut tsize: c_int = 0;
        for i in 0..DB_COUNT {
            if !(*curtab).tp_diffbuf[i].is_null() {
                if (*dp).df_count[i] < 0 {
                    return false;
                }
                tsize += (*dp).df_count[i];
            }
        }
        tsize <= linematch_lines
    }
}
unsafe fn get_max_diff_length(dp: *const diff_T) -> c_int {
    unsafe {
        let mut maxlength: c_int = 0;
        for k in 0..DB_COUNT {
            if !(*curtab).tp_diffbuf[k].is_null() && (*dp).df_count[k] > maxlength {
                maxlength = (*dp).df_count[k];
            }
        }
        maxlength
    }
}
unsafe fn find_top_diff_block(
    thistopdiff: &mut Option<usize>,
    next_adjacent_blocks: &mut Option<usize>,
    fromidx: usize,
    topline: c_int,
) {
    unsafe {
        let mut localtopdiff: Option<usize> = None;
        let mut topdiffchange = false;
        let diffs = &(*curtab).tp_diffs;
        for topdiff in 0..diffs.len() {
            if localtopdiff.is_none() || topdiffchange {
                localtopdiff = Some(topdiff);
                topdiffchange = false;
            }
            if topline >= diffs[topdiff].df_lnum[fromidx]
                && topline <= diffs[topdiff].df_lnum[fromidx] + diffs[topdiff].df_count[fromidx]
                && thistopdiff.is_none()
            {
                *thistopdiff = localtopdiff;
            }
            if !(topdiff + 1 < diffs.len()
                && diffs[topdiff + 1].df_lnum[fromidx]
                    == diffs[topdiff].df_lnum[fromidx] + diffs[topdiff].df_count[fromidx])
            {
                topdiffchange = true;
                if thistopdiff.is_some() {
                    *next_adjacent_blocks = if topdiff + 1 < diffs.len() {
                        Some(topdiff + 1)
                    } else {
                        None
                    };
                    break;
                }
            }
        }
    }
}
unsafe fn calculate_topfill_and_topline(
    fromidx: usize,
    toidx: usize,
    from_topline: c_int,
    from_topfill: c_int,
    topfill: *mut c_int,
    topline: *mut linenr_T,
) {
    unsafe {
        let mut thistopdiff: Option<usize> = None;
        let mut next_adjacent_blocks: Option<usize> = None;
        let mut virtual_lines_passed: c_int = 0;
        find_top_diff_block(
            &mut thistopdiff,
            &mut next_adjacent_blocks,
            fromidx,
            from_topline,
        );
        let diffs = &(*curtab).tp_diffs;
        let mut curdif = thistopdiff;
        while let Some(ci) = curdif
            && ci < diffs.len()
            && diffs[ci].df_lnum[fromidx] + diffs[ci].df_count[fromidx] <= from_topline
        {
            virtual_lines_passed += get_max_diff_length(&diffs[ci]);
            curdif = if ci + 1 < diffs.len() {
                Some(ci + 1)
            } else {
                None
            };
        }
        if curdif != next_adjacent_blocks
            && let Some(ci) = curdif
        {
            virtual_lines_passed += from_topline - diffs[ci].df_lnum[fromidx];
        }
        virtual_lines_passed -= from_topfill;
        if virtual_lines_passed < 0 {
            virtual_lines_passed = 0;
        }
        let mut curlinenum_to = match thistopdiff {
            Some(ti) => diffs[ti].df_lnum[toidx],
            None => 1,
        };
        let mut virt_lines_left = virtual_lines_passed;
        curdif = thistopdiff;
        while virt_lines_left > 0
            && curdif != next_adjacent_blocks
            && let Some(ci) = curdif
            && ci < diffs.len()
        {
            curlinenum_to += virt_lines_left.min(diffs[ci].df_count[toidx]);
            virt_lines_left -= virt_lines_left.min(get_max_diff_length(&diffs[ci]));
            curdif = if ci + 1 < diffs.len() {
                Some(ci + 1)
            } else {
                None
            };
        }
        let mut max_virt_lines: c_int = 0;
        if let Some(start_idx) = thistopdiff {
            for dp in &diffs[start_idx..] {
                if dp.df_lnum[toidx] + dp.df_count[toidx] <= curlinenum_to {
                    max_virt_lines += get_max_diff_length(dp);
                } else {
                    if dp.df_lnum[toidx] <= curlinenum_to {
                        max_virt_lines += curlinenum_to - dp.df_lnum[toidx];
                    }
                    break;
                }
            }
        }
        if (diff_flags & DIFF_FILLER) != 0 {
            *topfill = max_virt_lines - virtual_lines_passed;
        }
        *topline = curlinenum_to;
    }
}
unsafe fn apply_linematch_results(dp_idx: usize, decisions: &[c_int]) {
    unsafe {
        let mut line_numbers: [linenr_T; DB_COUNT] = [0; DB_COUNT];
        let mut outputmap: [usize; DB_COUNT] = [0; DB_COUNT];
        let mut ndiffs: usize = 0;
        for (i, line_num) in line_numbers.iter_mut().enumerate().take(DB_COUNT) {
            if !(*curtab).tp_diffbuf[i].is_null() {
                *line_num = (&(*curtab).tp_diffs)[dp_idx].df_lnum[i];
                (&mut (*curtab).tp_diffs)[dp_idx].df_count[i] = 0;
                outputmap[ndiffs] = i;
                ndiffs += 1;
            }
        }
        let mut dp_s_idx = dp_idx;
        for (i, &dec) in decisions.iter().enumerate() {
            if i != 0 && decisions[i - 1] != dec {
                dp_s_idx = diff_alloc_new(curtab, dp_s_idx + 1);
                (&mut (*curtab).tp_diffs)[dp_s_idx].is_linematched = true;
                for (j, &line_num) in line_numbers.iter().enumerate().take(DB_COUNT) {
                    if !(*curtab).tp_diffbuf[j].is_null() {
                        (&mut (*curtab).tp_diffs)[dp_s_idx].df_lnum[j] = line_num;
                        (&mut (*curtab).tp_diffs)[dp_s_idx].df_count[j] = 0;
                    }
                }
            }
            for j in 0..ndiffs {
                if (dec & (1 << j)) != 0 {
                    (&mut (*curtab).tp_diffs)[dp_s_idx].df_count[outputmap[j]] += 1;
                    line_numbers[outputmap[j]] += 1;
                }
            }
        }
        (&mut (*curtab).tp_diffs)[dp_idx].is_linematched = true;
    }
}
unsafe fn run_linematch_algorithm(dp_idx: usize) {
    unsafe {
        let mut diffbufs_mm: [mmfile_t; DB_COUNT] = [mmfile_t {
            ptr: ptr::null_mut(),
            size: 0,
        }; DB_COUNT];
        let mut diff_length: [c_int; DB_COUNT] = [0; DB_COUNT];
        let mut ndiffs: usize = 0;
        let dp_lnum = (&(*curtab).tp_diffs)[dp_idx].df_lnum;
        let dp_count = (&(*curtab).tp_diffs)[dp_idx].df_count;
        for i in 0..DB_COUNT {
            if !(*curtab).tp_diffbuf[i].is_null() {
                if dp_count[i] > 0 {
                    diff_write_buffer(
                        (*curtab).tp_diffbuf[i],
                        &raw mut diffbufs_mm[ndiffs],
                        dp_lnum[i],
                        dp_lnum[i] + dp_count[i] - 1,
                    );
                } else {
                    diffbufs_mm[ndiffs].size = 0;
                    diffbufs_mm[ndiffs].ptr = ptr::null_mut();
                }
                diff_length[ndiffs] = dp_count[i];
                ndiffs += 1;
            }
        }
        let diffbuf_refs: Vec<&mmfile_t> = diffbufs_mm[..ndiffs].iter().collect();
        let iwhite = (diff_flags & (DIFF_IWHITEALL | DIFF_IWHITE)) != 0;
        let decisions = linematch_nbuffers(&diffbuf_refs, &diff_length[..ndiffs], iwhite);
        for mm in diffbufs_mm.iter_mut().take(ndiffs) {
            xfree(mm.ptr.cast());
            mm.ptr = ptr::null_mut();
        }
        apply_linematch_results(dp_idx, &decisions);
    }
}
pub unsafe fn diff_check_with_linestatus(
    wp: *mut win_T,
    lnum: linenr_T,
    linestatus: *mut c_int,
) -> c_int {
    unsafe {
        let buf = (*wp).w_buffer;
        if !linestatus.is_null() {
            *linestatus = 0;
        }
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        if (*curtab).tp_diffs.is_empty() || (*wp).w_onebuf_opt.wo_diff == 0 {
            return 0;
        }
        if lnum < 1 || lnum > (*buf).b_ml.ml_line_count + 1 {
            return 0;
        }
        let idx = diff_buf_idx(buf, curtab);
        if idx == DB_COUNT as c_int {
            return 0;
        }
        let u_idx = idx as usize;
        if hasFolding(wp, lnum, ptr::null_mut(), ptr::null_mut())
            || decor_conceal_line(wp, lnum - 1, false)
        {
            return 0;
        }
        let mut le: Option<usize> = None;
        let mut dp_idx = diff_scan_start(curtab, idx, lnum);
        while dp_idx < (*curtab).tp_diffs.len() {
            if (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx] <= lnum {
                le = Some(dp_idx);
            }
            if lnum
                <= (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx]
                    + (&(*curtab).tp_diffs)[dp_idx].df_count[u_idx]
            {
                break;
            }
            dp_idx += 1;
        }
        diff_scan_remember(curtab, idx, lnum, le);
        if dp_idx >= (*curtab).tp_diffs.len() || lnum < (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx]
        {
            return 0;
        }
        if lnum >= (*wp).w_topline
            && lnum < (*wp).w_botline
            && !(&(*curtab).tp_diffs)[dp_idx].is_linematched
            && diff_linematch(&(&(*curtab).tp_diffs)[dp_idx])
            && diff_check_sanity(curtab, &(&(*curtab).tp_diffs)[dp_idx]) != 0
        {
            run_linematch_algorithm(dp_idx);
        }
        let mut num_fill: c_int = 0;
        while lnum
            == (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx]
                + (&(*curtab).tp_diffs)[dp_idx].df_count[u_idx]
        {
            if (diff_flags & DIFF_FILLER) != 0 {
                let maxcount = get_max_diff_length(&(&(*curtab).tp_diffs)[dp_idx]);
                num_fill += maxcount - (&(*curtab).tp_diffs)[dp_idx].df_count[u_idx];
            }
            if dp_idx + 1 < (*curtab).tp_diffs.len()
                && lnum >= (&(*curtab).tp_diffs)[dp_idx + 1].df_lnum[u_idx]
                && lnum
                    <= (&(*curtab).tp_diffs)[dp_idx + 1].df_lnum[u_idx]
                        + (&(*curtab).tp_diffs)[dp_idx + 1].df_count[u_idx]
            {
                dp_idx += 1;
            } else {
                break;
            }
        }
        let dp = &(&(*curtab).tp_diffs)[dp_idx];
        if lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx] {
            let mut zero = false;
            let mut cmp = false;
            for i in 0..DB_COUNT {
                if i != u_idx && !(*curtab).tp_diffbuf[i].is_null() {
                    if dp.df_count[i] == 0 {
                        zero = true;
                    } else {
                        if dp.df_count[i] != dp.df_count[u_idx] {
                            if !linestatus.is_null() {
                                *linestatus = -1;
                            }
                            return num_fill;
                        }
                        cmp = true;
                    }
                }
            }
            if cmp {
                for i in 0..DB_COUNT {
                    if i != u_idx
                        && !(*curtab).tp_diffbuf[i].is_null()
                        && dp.df_count[i] != 0
                        && !diff_equal_entry(dp, u_idx, i)
                    {
                        if !linestatus.is_null() {
                            *linestatus = -1;
                        }
                        return num_fill;
                    }
                }
            }
            if !zero {
                return num_fill;
            }
            if !linestatus.is_null() {
                *linestatus = -2;
            }
            return num_fill;
        }
        num_fill
    }
}
pub unsafe fn diff_check_fill(wp: *mut win_T, lnum: linenr_T) -> c_int {
    unsafe {
        if (diff_flags & DIFF_FILLER) == 0 {
            return 0;
        }
        let n = diff_check_with_linestatus(wp, lnum, ptr::null_mut());
        n.max(0)
    }
}
unsafe fn diff_equal_entry(dp: *const diff_T, idx1: usize, idx2: usize) -> bool {
    unsafe {
        if (*dp).df_count[idx1] != (*dp).df_count[idx2] {
            return false;
        }
        if diff_check_sanity(curtab, dp) == FAIL {
            return false;
        }
        for i in 0..(*dp).df_count[idx1] {
            let line = xstrdup(ml_get_buf(
                (*curtab).tp_diffbuf[idx1],
                (*dp).df_lnum[idx1] + i,
            ));
            let cmp = diff_cmp(
                line,
                ml_get_buf((*curtab).tp_diffbuf[idx2], (*dp).df_lnum[idx2] + i),
            );
            xfree(line.cast());
            if cmp != 0 {
                return false;
            }
        }
        true
    }
}
unsafe fn diff_equal_char(p1: *const c_char, p2: *const c_char, len: *mut c_int) -> bool {
    unsafe {
        let l = utfc_ptr2len(p1);
        if l != utfc_ptr2len(p2) {
            return false;
        }
        if l > 1 {
            if strncmp(p1, p2, l as usize) != 0
                && ((diff_flags & DIFF_ICASE) == 0
                    || utf_fold(utf_ptr2char(p1)) != utf_fold(utf_ptr2char(p2)))
            {
                return false;
            }
            *len = l;
        } else {
            if *p1 != *p2
                && ((diff_flags & DIFF_ICASE) == 0
                    || tolower(*p1 as u8 as c_int) != tolower(*p2 as u8 as c_int))
            {
                return false;
            }
            *len = 1;
        }
        true
    }
}
unsafe fn diff_cmp(s1: *const c_char, s2: *const c_char) -> c_int {
    unsafe {
        if (diff_flags & DIFF_IBLANK) != 0 && (*skipwhite(s1) == NUL || *skipwhite(s2) == NUL) {
            return 0;
        }
        if (diff_flags & (DIFF_ICASE | ALL_WHITE_DIFF)) == 0 {
            return strcmp(s1, s2);
        }
        if (diff_flags & DIFF_ICASE) != 0 && (diff_flags & ALL_WHITE_DIFF) == 0 {
            return mb_stricmp(s1, s2);
        }
        let mut p1: *const c_char = s1;
        let mut p2: *const c_char = s2;
        while *p1 != NUL && *p2 != NUL {
            if ((diff_flags & DIFF_IWHITE) != 0 && ascii_iswhite(*p1) && ascii_iswhite(*p2))
                || ((diff_flags & DIFF_IWHITEALL) != 0
                    && (ascii_iswhite(*p1) || ascii_iswhite(*p2)))
            {
                p1 = skipwhite(p1);
                p2 = skipwhite(p2);
            } else {
                let mut l: c_int = 0;
                if !diff_equal_char(p1, p2, &raw mut l) {
                    break;
                }
                p1 = p1.add(l as usize);
                p2 = p2.add(l as usize);
            }
        }
        p1 = skipwhite(p1);
        p2 = skipwhite(p2);
        if *p1 != NUL || *p2 != NUL {
            return 1;
        }
        0
    }
}
pub unsafe fn diff_set_topline(fromwin: *mut win_T, towin: *mut win_T) {
    unsafe {
        let frombuf = (*fromwin).w_buffer;
        let fromidx = diff_buf_idx(frombuf, curtab);
        if fromidx == DB_COUNT as c_int {
            return;
        }
        let u_fromidx = fromidx as usize;
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        let lnum = (*fromwin).w_topline;
        (*towin).w_topfill = 0;
        let dp_opt = (*curtab)
            .tp_diffs
            .iter()
            .find(|dp| lnum <= dp.df_lnum[u_fromidx] + dp.df_count[u_fromidx]);
        if let Some(dp) = dp_opt {
            let toidx = diff_buf_idx((*towin).w_buffer, curtab);
            if toidx == DB_COUNT as c_int {
                return;
            }
            let u_toidx = toidx as usize;
            (*towin).w_topline = lnum + (dp.df_lnum[u_toidx] - dp.df_lnum[u_fromidx]);
            if lnum >= dp.df_lnum[u_fromidx] {
                calculate_topfill_and_topline(
                    u_fromidx,
                    u_toidx,
                    (*fromwin).w_topline,
                    (*fromwin).w_topfill,
                    &raw mut (*towin).w_topfill,
                    &raw mut (*towin).w_topline,
                );
            }
        } else {
            (*towin).w_topline =
                (*(*towin).w_buffer).b_ml.ml_line_count - ((*frombuf).b_ml.ml_line_count - lnum);
        }
        (*towin).w_botfill = false;
        if (*towin).w_topline > (*(*towin).w_buffer).b_ml.ml_line_count {
            (*towin).w_topline = (*(*towin).w_buffer).b_ml.ml_line_count;
            (*towin).w_botfill = true;
        }
        if (*towin).w_topline < 1 {
            (*towin).w_topline = 1;
            (*towin).w_topfill = 0;
        }
        invalidate_botline_win(towin);
        changed_line_abv_curs_win(towin);
        check_topfill(towin, false);
        hasFolding(
            towin,
            (*towin).w_topline,
            &raw mut (*towin).w_topline,
            ptr::null_mut(),
        );
    }
}
unsafe fn parse_diffanchors(
    check_only: bool,
    buf: *mut buf_T,
    anchors: *mut linenr_T,
    num_anchors: *mut c_int,
) -> c_int {
    unsafe {
        let mut dia = if *(*buf).b_p_dia == NUL {
            p_dia
        } else {
            (*buf).b_p_dia
        };
        let orig_curbuf = curbuf;
        let orig_curwin = curwin;
        let bufwin: *mut win_T = if check_only {
            curwin
        } else {
            let mut bw = firstwin;
            while !bw.is_null() {
                if (*bw).w_buffer == buf && (*bw).w_onebuf_opt.wo_diff != 0 {
                    break;
                }
                bw = (*bw).w_next;
            }
            if bw.is_null() && *dia != NUL {
                emsg(gettext(
                    (&raw const e_diff_anchors_with_hidden_windows).cast(),
                ));
                return FAIL;
            }
            bw
        };
        let mut i: usize = 0;
        while i < MAX_DIFF_ANCHORS && *dia != NUL {
            if *dia == b',' as c_char {
                return FAIL;
            }
            curbuf = buf;
            curwin = bufwin;
            let mut errormsg: *const c_char = ptr::null();
            let lnum = get_address(
                ptr::null_mut(),
                &raw mut dia,
                ADDR_LINES,
                check_only,
                true,
                0,
                1,
                &raw mut errormsg,
            );
            curbuf = orig_curbuf;
            curwin = orig_curwin;
            if !errormsg.is_null() {
                emsg(errormsg);
            }
            if dia.is_null() {
                return FAIL;
            }
            if *dia != b',' as c_char && *dia != NUL {
                return FAIL;
            }
            if !check_only && (lnum == MAXLNUM || lnum <= 0 || lnum > (*buf).b_ml.ml_line_count + 1)
            {
                emsg(gettext((&raw const e_invrange).cast()));
                return FAIL;
            }
            if !anchors.is_null() {
                *anchors.add(i) = lnum;
            }
            if *dia == b',' as c_char {
                dia = dia.add(1);
            }
            i += 1;
        }
        if i == MAX_DIFF_ANCHORS && *dia != NUL {
            semsg!(
                gettext((&raw const e_cannot_have_more_than_nr_diff_anchors).cast()),
                MAX_DIFF_ANCHORS as c_int,
            );
            return FAIL;
        }
        if !num_anchors.is_null() {
            *num_anchors = i as c_int;
        }
        OK
    }
}
pub unsafe fn diffanchors_changed(buflocal: bool) -> c_int {
    unsafe {
        let result = parse_diffanchors(true, curbuf, ptr::null_mut(), ptr::null_mut());
        if result == OK && (diff_flags & DIFF_ANCHOR) != 0 {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                if !buflocal {
                    (*tp).tp_diff_invalid = 1;
                } else {
                    for idx in 0..DB_COUNT {
                        if (*tp).tp_diffbuf[idx] == curbuf {
                            (*tp).tp_diff_invalid = 1;
                            break;
                        }
                    }
                }
                tp = (*tp).tp_next;
            }
        }
        result
    }
}
pub unsafe fn diffopt_changed() -> c_int {
    unsafe {
        let v = opt_keyset(p_dip, kOptDiffopt, ptr::null_mut()).cast::<OptKeyDict_dip>();
        let mut flags = (if (*v).filler { DIFF_FILLER } else { 0 })
            | (if (*v).anchor { DIFF_ANCHOR } else { 0 })
            | (if (*v).iblank { DIFF_IBLANK } else { 0 })
            | (if (*v).icase { DIFF_ICASE } else { 0 })
            | (if (*v).iwhiteall { DIFF_IWHITEALL } else { 0 })
            | (if (*v).iwhiteeol { DIFF_IWHITEEOL } else { 0 })
            | (if (*v).iwhite { DIFF_IWHITE } else { 0 })
            | (if (*v).horizontal { DIFF_HORIZONTAL } else { 0 })
            | (if (*v).vertical { DIFF_VERTICAL } else { 0 })
            | (if (*v).closeoff { DIFF_CLOSE_OFF } else { 0 })
            | (if (*v).hiddenoff { DIFF_HIDDEN_OFF } else { 0 })
            | (if (*v).followwrap { DIFF_FOLLOWWRAP } else { 0 })
            | (if (*v).internal { DIFF_INTERNAL } else { 0 });
        if has_dip_key(&*v, KEYSET_OPTIDX_dip__linematch) {
            flags |= DIFF_LINEMATCH | DIFF_FILLER;
        }
        if has_dip_key(&*v, KEYSET_OPTIDX_dip__inline_) {
            flags &= !ALL_INLINE;
            let inline_ptr = (&raw const (*v).inline_).cast::<c_char>();
            if strequal(inline_ptr, c"simple".as_ptr()) {
                flags |= DIFF_INLINE_SIMPLE;
            } else if strequal(inline_ptr, c"char".as_ptr()) {
                flags |= DIFF_INLINE_CHAR;
            } else if strequal(inline_ptr, c"word".as_ptr()) {
                flags |= DIFF_INLINE_WORD;
            } else {
                flags |= DIFF_INLINE_NONE;
            }
        }
        let mut algorithm = if (*v).indent_heuristic {
            XDF_INDENT_HEURISTIC
        } else {
            0
        };
        if has_dip_key(&*v, KEYSET_OPTIDX_dip__algorithm) {
            let algo_ptr = (&raw const (*v).algorithm).cast::<c_char>();
            if strequal(algo_ptr, c"minimal".as_ptr()) {
                algorithm |= XDF_NEED_MINIMAL;
            } else if strequal(algo_ptr, c"patience".as_ptr()) {
                algorithm |= XDF_PATIENCE_DIFF;
            } else if strequal(algo_ptr, c"histogram".as_ptr()) {
                algorithm |= XDF_HISTOGRAM_DIFF;
            }
        }
        let mut ret = OK;
        if (flags & DIFF_HORIZONTAL) != 0 && (flags & DIFF_VERTICAL) != 0 {
            ret = FAIL;
        } else {
            if diff_flags != flags || diff_algorithm != algorithm {
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    (*tp).tp_diff_invalid = 1;
                    tp = (*tp).tp_next;
                }
            }
            let context = if has_dip_key(&*v, KEYSET_OPTIDX_dip__context) {
                (*v).context as c_int
            } else {
                6
            };
            diff_flags = flags;
            diff_context = if context == 0 { 1 } else { context };
            linematch_lines = if has_dip_key(&*v, KEYSET_OPTIDX_dip__linematch) {
                (*v).linematch as c_int
            } else {
                0
            };
            diff_foldcolumn = if has_dip_key(&*v, KEYSET_OPTIDX_dip__foldcolumn) {
                (*v).foldcolumn as c_int
            } else {
                2
            };
            diff_algorithm = algorithm;
            diff_redraw(true);
            check_scrollbind(0, 0);
        }
        ret
    }
}
pub unsafe fn diffopt_horizontal() -> bool {
    unsafe { (diff_flags & DIFF_HORIZONTAL) != 0 }
}
pub unsafe fn diffopt_hiddenoff() -> bool {
    unsafe { (diff_flags & DIFF_HIDDEN_OFF) != 0 }
}
pub unsafe fn diffopt_closeoff() -> bool {
    unsafe { (diff_flags & DIFF_CLOSE_OFF) != 0 }
}
pub unsafe fn diffopt_filler() -> bool {
    unsafe { (diff_flags & DIFF_FILLER) != 0 }
}
pub unsafe fn diff_update_line(lnum: linenr_T) {
    unsafe {
        if (diff_flags & ALL_INLINE_DIFF) == 0 {
            return;
        }
        let idx = diff_buf_idx(curbuf, curtab);
        if idx == DB_COUNT as c_int {
            return;
        }
        let u_idx = idx as usize;
        if let Some(dp) = (*curtab)
            .tp_diffs
            .iter_mut()
            .find(|dp| lnum <= dp.df_lnum[u_idx] + dp.df_count[u_idx])
        {
            dp.has_changes = false;
            dp.df_changes.clear();
        }
    }
}
pub unsafe fn diff_change_parse(
    diffline: *mut diffline_T,
    change: *mut diffline_change_T,
    change_start: *mut c_int,
    change_end: *mut c_int,
) -> bool {
    unsafe {
        let bufidx = (*diffline).bufidx as usize;
        if (*change).dc_start_lnum_off[bufidx] < (*diffline).lineoff {
            *change_start = 0;
        } else {
            *change_start = (*change).dc_start[bufidx];
        }
        if (*change).dc_end_lnum_off[bufidx] > (*diffline).lineoff {
            *change_end = c_int::MAX;
        } else {
            *change_end = (*change).dc_end[bufidx];
        }
        if ptr::eq(change, &raw const simple_diffline_change) {
            return false;
        }
        for i in 0..DB_COUNT {
            if i == bufidx {
                continue;
            }
            if (*change).dc_start[i] != (*change).dc_end[i]
                || (*change).dc_end_lnum_off[i] != (*change).dc_start_lnum_off[i]
            {
                return false;
            }
        }
        true
    }
}
unsafe fn diff_find_change_simple(
    wp: *mut win_T,
    lnum: linenr_T,
    dp: *const diff_T,
    idx: usize,
    startp: *mut c_int,
    endp: *mut c_int,
) -> bool {
    unsafe {
        let line_org = if (diff_flags & DIFF_INLINE_NONE) != 0 {
            ptr::null_mut()
        } else {
            xstrdup(ml_get_buf((*wp).w_buffer, lnum))
        };
        let mut added = true;
        let off = lnum - (*dp).df_lnum[idx];
        for i in 0..DB_COUNT {
            if !(*curtab).tp_diffbuf[i].is_null() && i != idx {
                if off >= (*dp).df_count[i] {
                    continue;
                }
                added = false;
                if (diff_flags & DIFF_INLINE_NONE) != 0 {
                    break;
                }
                let line_new = ml_get_buf((*curtab).tp_diffbuf[i], (*dp).df_lnum[i] + off);
                let mut si_org: c_int = 0;
                let mut si_new: c_int = 0;
                while *line_org.add(si_org as usize) != NUL {
                    if ((diff_flags & DIFF_IWHITE) != 0
                        && ascii_iswhite(*line_org.add(si_org as usize))
                        && ascii_iswhite(*line_new.add(si_new as usize)))
                        || ((diff_flags & DIFF_IWHITEALL) != 0
                            && (ascii_iswhite(*line_org.add(si_org as usize))
                                || ascii_iswhite(*line_new.add(si_new as usize))))
                    {
                        si_org =
                            skipwhite(line_org.add(si_org as usize)).offset_from(line_org) as c_int;
                        si_new =
                            skipwhite(line_new.add(si_new as usize)).offset_from(line_new) as c_int;
                    } else {
                        let mut l: c_int = 0;
                        if !diff_equal_char(
                            line_org.add(si_org as usize),
                            line_new.add(si_new as usize),
                            &raw mut l,
                        ) {
                            break;
                        }
                        si_org += l;
                        si_new += l;
                    }
                }
                si_org -= utf_head_off(line_org, line_org.add(si_org as usize));
                si_new -= utf_head_off(line_new, line_new.add(si_new as usize));
                *startp = (*startp).min(si_org);
                if *line_org.add(si_org as usize) != NUL || *line_new.add(si_new as usize) != NUL {
                    let mut ei_org = strlen(line_org) as c_int;
                    let mut ei_new = strlen(line_new) as c_int;
                    while ei_org >= *startp && ei_new >= si_new && ei_org >= 0 && ei_new >= 0 {
                        if ((diff_flags & DIFF_IWHITE) != 0
                            && ascii_iswhite(*line_org.add(ei_org as usize))
                            && ascii_iswhite(*line_new.add(ei_new as usize)))
                            || ((diff_flags & DIFF_IWHITEALL) != 0
                                && (ascii_iswhite(*line_org.add(ei_org as usize))
                                    || ascii_iswhite(*line_new.add(ei_new as usize))))
                        {
                            while ei_org >= *startp && ascii_iswhite(*line_org.add(ei_org as usize))
                            {
                                ei_org -= 1;
                            }
                            while ei_new >= si_new && ascii_iswhite(*line_new.add(ei_new as usize))
                            {
                                ei_new -= 1;
                            }
                        } else {
                            let mut p1: *const c_char = line_org.add(ei_org as usize);
                            let mut p2: *const c_char = line_new.add(ei_new as usize);
                            p1 = p1.sub(utf_head_off(line_org, p1) as usize);
                            p2 = p2.sub(utf_head_off(line_new, p2) as usize);
                            let mut l: c_int = 0;
                            if !diff_equal_char(p1, p2, &raw mut l) {
                                break;
                            }
                            ei_org -= l;
                            ei_new -= l;
                        }
                    }
                    *endp = (*endp).max(ei_org);
                }
            }
        }
        xfree(line_org.cast());
        added
    }
}
unsafe fn diff_refine_inline_char_highlight(
    linemap: &[Vec<linemap_entry_T>; DB_COUNT],
    idx1: usize,
) {
    unsafe {
        let mut pass: c_int = 1;
        loop {
            let mut has_unmerged_gaps = false;
            let mut has_merged_gaps = false;
            let mut di: usize = 0;
            while di + 1 < (*curtab).tp_diffs.len() {
                let lm = &linemap[idx1];
                let lm_len = lm.len() as c_int;
                if (&(*curtab).tp_diffs)[di].df_lnum[idx1]
                    + (&(*curtab).tp_diffs)[di].df_count[idx1]
                    > lm_len
                    || (&(*curtab).tp_diffs)[di + 1].df_lnum[idx1] > lm_len
                {
                    di += 1;
                    continue;
                }
                let entry1 = &lm[((&(*curtab).tp_diffs)[di].df_lnum[idx1]
                    + (&(*curtab).tp_diffs)[di].df_count[idx1]
                    - 1) as usize];
                let entry2 = &lm[((&(*curtab).tp_diffs)[di + 1].df_lnum[idx1] - 1) as usize];
                if entry1.lineoff != entry2.lineoff {
                    di += 1;
                    continue;
                }
                let gap = (&(*curtab).tp_diffs)[di + 1].df_lnum[idx1]
                    - ((&(*curtab).tp_diffs)[di].df_lnum[idx1]
                        + (&(*curtab).tp_diffs)[di].df_count[idx1]);
                if gap <= 3 {
                    let mut max_df_count: linenr_T = 0;
                    for i in 0..DB_COUNT {
                        max_df_count = max_df_count.max(
                            (&(*curtab).tp_diffs)[di].df_count[i]
                                + (&(*curtab).tp_diffs)[di + 1].df_count[i],
                        );
                    }
                    if max_df_count >= gap * 4 {
                        let next_lnum = (&(*curtab).tp_diffs)[di + 1].df_lnum;
                        let next_count = (&(*curtab).tp_diffs)[di + 1].df_count;
                        for i in 0..DB_COUNT {
                            (&mut (*curtab).tp_diffs)[di].df_count[i] = next_lnum[i]
                                + next_count[i]
                                - (&(*curtab).tp_diffs)[di].df_lnum[i];
                        }
                        diff_free(curtab, di + 1);
                        has_merged_gaps = true;
                        continue;
                    } else {
                        has_unmerged_gaps = true;
                    }
                }
                di += 1;
            }
            if !has_unmerged_gaps || !has_merged_gaps {
                break;
            }
            let prev_pass = pass;
            pass += 1;
            if prev_pass >= 4 {
                break;
            }
        }
    }
}
unsafe fn diff_refine_inline_word_highlight(
    linemap: &[Vec<linemap_entry_T>; DB_COUNT],
    idx1: usize,
    start_lnum: linenr_T,
) {
    unsafe {
        let mut pass: c_int = 1;
        loop {
            let mut di: usize = 0;
            while di + 1 < (*curtab).tp_diffs.len() {
                let idx_entry1 = (&(*curtab).tp_diffs)[di].df_lnum[idx1]
                    + (&(*curtab).tp_diffs)[di].df_count[idx1]
                    - 2;
                let idx_entry2 = (&(*curtab).tp_diffs)[di + 1].df_lnum[idx1] - 1;
                let lm1 = &linemap[idx1];
                let lm1_len = lm1.len() as c_int;
                if idx_entry1 < 0
                    || idx_entry1 >= lm1_len
                    || idx_entry2 < 0
                    || idx_entry2 >= lm1_len
                {
                    di += 1;
                    continue;
                }
                let entry1 = &lm1[idx_entry1 as usize];
                let entry2 = &lm1[idx_entry2 as usize];
                if entry1.lineoff != entry2.lineoff {
                    di += 1;
                    continue;
                }
                let gap_start = entry1.byte_start + entry1.num_bytes;
                let gap_end = entry2.byte_start;
                let gap_size = gap_end - gap_start;
                if gap_size <= 0 || gap_size > diff_word_gap {
                    di += 1;
                    continue;
                }
                let line = ml_get_buf((*curtab).tp_diffbuf[idx1], start_lnum + entry1.lineoff);
                let gap_text = line.add(gap_start as usize);
                let mut only_non_word = true;
                let mut has_content = false;
                let mut i: c_int = 0;
                while i < gap_size && *gap_text.add(i as usize) != NUL {
                    has_content = true;
                    let char_class = mb_get_class_tab(
                        gap_text.add(i as usize),
                        (&raw const (*(*curtab).tp_diffbuf[idx1]).b_chartab).cast(),
                    );
                    if char_class == 2 {
                        only_non_word = false;
                        break;
                    }
                    i += 1;
                }
                if has_content && only_non_word {
                    let mut total_change_bytes: i64 = 0;
                    for (buf_i, lmi) in linemap.iter().enumerate() {
                        if !(*curtab).tp_diffbuf[buf_i].is_null() {
                            for k in 0..(&(*curtab).tp_diffs)[di].df_count[buf_i] {
                                let idx = (&(*curtab).tp_diffs)[di].df_lnum[buf_i] + k - 1;
                                if let Some(entry) = lmi.get(idx as usize) {
                                    total_change_bytes += entry.num_bytes as i64;
                                }
                            }
                            for k in 0..(&(*curtab).tp_diffs)[di + 1].df_count[buf_i] {
                                let idx = (&(*curtab).tp_diffs)[di + 1].df_lnum[buf_i] + k - 1;
                                if let Some(entry) = lmi.get(idx as usize) {
                                    total_change_bytes += entry.num_bytes as i64;
                                }
                            }
                        }
                    }
                    if total_change_bytes >= (gap_size * 2) as i64 {
                        let next_lnum = (&(*curtab).tp_diffs)[di + 1].df_lnum;
                        let next_count = (&(*curtab).tp_diffs)[di + 1].df_count;
                        for buf_i in 0..DB_COUNT {
                            if !(*curtab).tp_diffbuf[buf_i].is_null() {
                                (&mut (*curtab).tp_diffs)[di].df_count[buf_i] = next_lnum[buf_i]
                                    + next_count[buf_i]
                                    - (&(*curtab).tp_diffs)[di].df_lnum[buf_i];
                            }
                        }
                        diff_free(curtab, di + 1);
                        continue;
                    }
                }
                di += 1;
            }
            let prev_pass = pass;
            pass += 1;
            if prev_pass >= 4 {
                break;
            }
        }
    }
}
unsafe fn diff_find_change_inline_diff(dp_idx: usize) {
    unsafe {
        let save_diff_algorithm = diff_algorithm;
        let mut dio = diffio_T {
            dio_orig: diffin_T {
                din_fname: ptr::null_mut(),
                din_mmfile: mmfile_t {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
            },
            dio_new: diffin_T {
                din_fname: ptr::null_mut(),
                din_mmfile: mmfile_t {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
            },
            dio_diff: diffout_T {
                dout_fname: ptr::null_mut(),
                dout_ga: Vec::new(),
            },
            dio_internal: 1,
        };
        diff_algorithm |= XDF_INDENT_HEURISTIC;
        let dp_lnum = (&(*curtab).tp_diffs)[dp_idx].df_lnum;
        let dp_count = (&(*curtab).tp_diffs)[dp_idx].df_count;
        let orig_diff = std::mem::take(&mut (*curtab).tp_diffs);
        let orig_diffbuf = (*curtab).tp_diffbuf;
        let mut linemap: [Vec<linemap_entry_T>; DB_COUNT] = std::array::from_fn(|_| Vec::new());
        let mut file1_str: Vec<u8> = Vec::new();
        let mut file2_str: Vec<u8> = Vec::new();
        let mut changes: Vec<diffline_change_T> = Vec::new();
        'done: {
            let mut file1_idx: isize = -1;
            for (i, lm) in linemap.iter_mut().enumerate().take(DB_COUNT) {
                dio.dio_diff.dout_ga.clear();
                let buf = (*curtab).tp_diffbuf[i];
                if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
                    continue;
                }
                if dp_count[i] == 0 {
                    (*curtab).tp_diffbuf[i] = ptr::null_mut();
                    continue;
                }
                if file1_idx == -1 {
                    file1_idx = i as isize;
                }
                let curstr: &mut Vec<u8> = if file1_idx != i as isize {
                    &mut file2_str
                } else {
                    &mut file1_str
                };
                let mut numlines: linenr_T = 0;
                curstr.clear();
                for off in 0..dp_count[i] {
                    let curline = ml_get_buf((*curtab).tp_diffbuf[i], dp_lnum[i] + off);
                    let mut in_keyword = false;
                    let mut last_white = false;
                    let mut eol_ga_len: c_int = -1;
                    let mut eol_linemap_len: c_int = -1;
                    let mut eol_numlines: linenr_T = -1;
                    let mut s = curline;
                    while *s != NUL {
                        let mut new_in_keyword = false;
                        if (diff_flags & DIFF_INLINE_WORD) != 0 {
                            new_in_keyword = mb_get_class_tab(
                                s,
                                (&raw const (*(*curtab).tp_diffbuf[file1_idx as usize]).b_chartab)
                                    .cast(),
                            ) == 2;
                        }
                        if in_keyword && !new_in_keyword {
                            curstr.push(NL as u8);
                            numlines += 1;
                        }
                        if ascii_iswhite(*s) {
                            if (diff_flags & DIFF_IWHITEALL) != 0 {
                                in_keyword = false;
                                s = skipwhite(s);
                                continue;
                            } else if ((diff_flags & DIFF_IWHITEEOL) != 0
                                || (diff_flags & DIFF_IWHITE) != 0)
                                && !last_white
                            {
                                eol_ga_len = curstr.len() as c_int;
                                eol_linemap_len = lm.len() as c_int;
                                eol_numlines = numlines;
                                last_white = true;
                            }
                        } else if (diff_flags & DIFF_IWHITEEOL) != 0
                            || (diff_flags & DIFF_IWHITE) != 0
                        {
                            last_white = false;
                            eol_ga_len = -1;
                            eol_linemap_len = -1;
                            eol_numlines = -1;
                        }
                        let mut char_len: c_int = 1;
                        if *s == NL {
                            curstr.push(NUL as u8);
                        } else {
                            char_len = utfc_ptr2len(s);
                            if ascii_iswhite(*s) && (diff_flags & DIFF_IWHITE) != 0 {
                                char_len = skipwhite(s).offset_from(s) as c_int;
                            }
                            if (diff_flags & DIFF_ICASE) != 0 {
                                let mut c = utf_ptr2char(s);
                                let c_len = utf_char2len(c);
                                c = utf_fold(c);
                                let mut cbuf: [c_char; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];
                                let c_fold_len = utf_char2bytes(c, (&raw mut cbuf).cast());
                                curstr.extend_from_slice(std::slice::from_raw_parts(
                                    cbuf.as_ptr().cast::<u8>(),
                                    c_fold_len as usize,
                                ));
                                if char_len > c_len {
                                    curstr.extend_from_slice(std::slice::from_raw_parts(
                                        s.add(c_len as usize).cast::<u8>(),
                                        (char_len - c_len) as usize,
                                    ));
                                }
                            } else {
                                curstr.extend_from_slice(std::slice::from_raw_parts(
                                    s.cast::<u8>(),
                                    char_len as usize,
                                ));
                            }
                        }
                        if !new_in_keyword {
                            curstr.push(NL as u8);
                            numlines += 1;
                        }
                        if !new_in_keyword || (new_in_keyword && !in_keyword) {
                            lm.push(linemap_entry_T {
                                byte_start: s.offset_from(curline) as colnr_T,
                                num_bytes: char_len,
                                lineoff: off,
                            });
                        } else if let Some(last) = lm.last_mut() {
                            last.num_bytes += char_len;
                        }
                        in_keyword = new_in_keyword;
                        s = s.add(char_len as usize);
                    }
                    if in_keyword {
                        curstr.push(NL as u8);
                        numlines += 1;
                    }
                    if ((diff_flags & DIFF_IWHITEEOL) != 0 || (diff_flags & DIFF_IWHITE) != 0)
                        && eol_ga_len != -1
                    {
                        curstr.truncate(eol_ga_len as usize);
                        lm.truncate(eol_linemap_len as usize);
                        numlines = eol_numlines;
                    }
                    if (diff_flags & DIFF_IWHITEALL) == 0 {
                        curstr.push(NL as u8);
                        let _ = numlines;
                        lm.push(linemap_entry_T {
                            byte_start: s.offset_from(curline) as colnr_T,
                            num_bytes: size_of::<c_int>() as colnr_T,
                            lineoff: off,
                        });
                    }
                }
                let ptr = if curstr.is_empty() {
                    ptr::null_mut()
                } else {
                    curstr.as_mut_ptr().cast::<c_char>()
                };
                if file1_idx != i as isize {
                    dio.dio_new.din_mmfile.ptr = ptr;
                    dio.dio_new.din_mmfile.size = curstr.len() as c_int;
                } else {
                    dio.dio_orig.din_mmfile.ptr = ptr;
                    dio.dio_orig.din_mmfile.size = curstr.len() as c_int;
                }
                if file1_idx != i as isize {
                    let diff_status = diff_file_internal(&raw mut dio);
                    if diff_status == FAIL {
                        break 'done;
                    }
                    diff_read(0, i, &raw mut dio);
                    clear_diffout(&raw mut dio.dio_diff);
                }
            }
            if (diff_flags & DIFF_INLINE_WORD) != 0 && file1_idx != -1 {
                diff_refine_inline_word_highlight(
                    &linemap,
                    file1_idx as usize,
                    dp_lnum[file1_idx as usize],
                );
            } else if (diff_flags & DIFF_INLINE_CHAR) != 0 && file1_idx != -1 {
                diff_refine_inline_char_highlight(&linemap, file1_idx as usize);
            }
            for new_diff in &(*curtab).tp_diffs {
                let mut change = diffline_change_T {
                    dc_start: [0; DB_COUNT],
                    dc_end: [0; DB_COUNT],
                    dc_start_lnum_off: [0; DB_COUNT],
                    dc_end_lnum_off: [0; DB_COUNT],
                };
                for (i, lm) in linemap.iter().enumerate() {
                    if new_diff.df_lnum[i] <= 0 {
                        continue;
                    }
                    let lm_len = lm.len() as c_int;
                    let diff_lnum = new_diff.df_lnum[i] - 1;
                    let diff_lnum_end = diff_lnum + new_diff.df_count[i];
                    if diff_lnum >= lm_len {
                        change.dc_start[i] = MAXCOL;
                        change.dc_start_lnum_off[i] = c_int::MAX;
                    } else {
                        let e = &lm[diff_lnum as usize];
                        change.dc_start[i] = e.byte_start;
                        change.dc_start_lnum_off[i] = e.lineoff;
                    }
                    if diff_lnum == diff_lnum_end {
                        change.dc_end[i] = change.dc_start[i];
                        change.dc_end_lnum_off[i] = change.dc_start_lnum_off[i];
                    } else if diff_lnum_end > lm_len {
                        change.dc_end[i] = MAXCOL;
                        change.dc_end_lnum_off[i] = c_int::MAX;
                    } else {
                        let e = &lm[(diff_lnum_end - 1) as usize];
                        change.dc_end[i] = e.byte_start + e.num_bytes;
                        change.dc_end_lnum_off[i] = e.lineoff;
                    }
                }
                changes.push(change);
            }
        }
        diff_algorithm = save_diff_algorithm;
        diff_clear(curtab);
        (*curtab).tp_diffs = orig_diff;
        (*curtab).tp_diffbuf = orig_diffbuf;
        (&mut (*curtab).tp_diffs)[dp_idx].df_changes = changes;
        (&mut (*curtab).tp_diffs)[dp_idx].has_changes = true;
        clear_diffout(&raw mut dio.dio_diff);
    }
}
pub unsafe fn diff_find_change(wp: *mut win_T, lnum: linenr_T, diffline: *mut diffline_T) -> bool {
    unsafe {
        let idx = diff_buf_idx((*wp).w_buffer, curtab);
        if idx == DB_COUNT as c_int {
            return false;
        }
        let u_idx = idx as usize;
        let Some(dp_idx) = (*curtab)
            .tp_diffs
            .iter()
            .position(|dp| lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx])
        else {
            return false;
        };
        if diff_check_sanity(curtab, &(&(*curtab).tp_diffs)[dp_idx]) == FAIL {
            return false;
        }
        let off = lnum - (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx];
        if (diff_flags & ALL_INLINE_DIFF) == 0 {
            let mut change_start: c_int = MAXCOL;
            let mut change_end: c_int = -1;
            let ret = diff_find_change_simple(
                wp,
                lnum,
                &(&(*curtab).tp_diffs)[dp_idx],
                u_idx,
                &raw mut change_start,
                &raw mut change_end,
            );
            change_end += 1;
            simple_diffline_change = diffline_change_T {
                dc_start: [0; DB_COUNT],
                dc_end: [0; DB_COUNT],
                dc_start_lnum_off: [0; DB_COUNT],
                dc_end_lnum_off: [0; DB_COUNT],
            };
            (*diffline).changes = &raw mut simple_diffline_change;
            (*diffline).num_changes = 1;
            (*diffline).bufidx = idx;
            (*diffline).lineoff = lnum - (&(*curtab).tp_diffs)[dp_idx].df_lnum[u_idx];
            simple_diffline_change.dc_start[u_idx] = change_start;
            simple_diffline_change.dc_end[u_idx] = change_end;
            simple_diffline_change.dc_start_lnum_off[u_idx] = off;
            simple_diffline_change.dc_end_lnum_off[u_idx] = off;
            return ret;
        }
        if !(&(*curtab).tp_diffs)[dp_idx].has_changes {
            diff_find_change_inline_diff(dp_idx);
        }
        let dp = &mut (&mut (*curtab).tp_diffs)[dp_idx];
        let changes_len = dp.df_changes.len() as c_int;
        let mut num_changes: c_int = 0;
        let mut change_idx: c_int = 0;
        (*diffline).changes = ptr::null_mut();
        while change_idx < changes_len {
            let change = dp.df_changes.as_mut_ptr().add(change_idx as usize);
            if (*change).dc_end_lnum_off[u_idx] < off {
                change_idx += 1;
                continue;
            }
            if (*change).dc_start_lnum_off[u_idx] > off {
                break;
            }
            if (*diffline).changes.is_null() {
                (*diffline).changes = change;
            }
            num_changes += 1;
            change_idx += 1;
        }
        (*diffline).num_changes = num_changes;
        (*diffline).bufidx = idx;
        (*diffline).lineoff = off;
        let mut added = false;
        if num_changes == 1 && change_idx == changes_len {
            added = true;
            for i in 0..DB_COUNT {
                if u_idx == i {
                    continue;
                }
                if (*curtab).tp_diffbuf[i].is_null() {
                    continue;
                }
                let change = &dp.df_changes[(changes_len - 1) as usize];
                if change.dc_start_lnum_off[i] != c_int::MAX {
                    added = false;
                    break;
                }
            }
        }
        added
    }
}
pub unsafe fn diff_infold(wp: *mut win_T, lnum: linenr_T) -> bool {
    unsafe {
        if (*wp).w_onebuf_opt.wo_diff == 0 {
            return false;
        }
        let mut idx: c_int = -1;
        let mut other = false;
        for i in 0..DB_COUNT {
            if (*curtab).tp_diffbuf[i] == (*wp).w_buffer {
                idx = i as c_int;
            } else if !(*curtab).tp_diffbuf[i].is_null() {
                other = true;
            }
        }
        if idx == -1 || !other {
            return false;
        }
        let u_idx = idx as usize;
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        if (*curtab).tp_diffs.is_empty() {
            return true;
        }
        let mut le: Option<usize> = None;
        let mut result = true;
        let mut dp_idx = diff_scan_start(curtab, idx, lnum);
        while dp_idx < (*curtab).tp_diffs.len() {
            let dp = &(&(*curtab).tp_diffs)[dp_idx];
            if dp.df_lnum[u_idx] <= lnum {
                le = Some(dp_idx);
            }
            if dp.df_lnum[u_idx] - diff_context > lnum {
                break;
            }
            if dp.df_lnum[u_idx] + dp.df_count[u_idx] + diff_context > lnum {
                result = false;
                break;
            }
            dp_idx += 1;
        }
        diff_scan_remember(curtab, idx, lnum, le);
        result
    }
}
pub unsafe fn nv_diffgetput(put: bool, count: usize) {
    unsafe {
        if bt_prompt(curbuf) {
            vim_beep(kOptBoFlagOperator);
            return;
        }
        let mut ea = exarg_T::new();
        let mut buf: [c_char; 30] = [0; 30];
        if count == 0 {
            ea.arg = c"".as_ptr().cast_mut();
        } else {
            vim_snprintf!((&raw mut buf).cast(), buf.len(), c"%zu".as_ptr(), count);
            ea.arg = (&raw mut buf).cast();
        }
        ea.cmdidx = if put { CMD_diffput } else { CMD_diffget };
        ea.addr_count = 0;
        ea.line1 = (*curwin).w_cursor.lnum;
        ea.line2 = (*curwin).w_cursor.lnum;
        ex_diffgetput(&raw mut ea);
    }
}
unsafe fn valid_diff(df_id: Option<u64>) -> Option<usize> {
    unsafe {
        let id = df_id?;
        (*curtab).tp_diffs.iter().position(|dp| dp.df_id == id)
    }
}
pub unsafe fn ex_diffgetput(eap: *mut exarg_T) {
    unsafe {
        let idx_other: usize;
        let idx_cur_i = diff_buf_idx(curbuf, curtab);
        if idx_cur_i == DB_COUNT as c_int {
            emsg(gettext(c"E99: Current buffer is not in diff mode".as_ptr()));
            return;
        }
        let idx_cur = idx_cur_i as usize;
        if *(*eap).arg == NUL {
            let mut found_not_ma = false;
            let mut idx = 0;
            while idx < DB_COUNT {
                if (*curtab).tp_diffbuf[idx] != curbuf && !(*curtab).tp_diffbuf[idx].is_null() {
                    if (*eap).cmdidx != CMD_diffput || modifiable((*curtab).tp_diffbuf[idx]) {
                        break;
                    }
                    found_not_ma = true;
                }
                idx += 1;
            }
            idx_other = idx;
            if idx_other == DB_COUNT {
                if found_not_ma {
                    emsg(gettext(
                        c"E793: No other buffer in diff mode is modifiable".as_ptr(),
                    ));
                } else {
                    emsg(gettext(c"E100: No other buffer in diff mode".as_ptr()));
                }
                return;
            }
            for i in (idx_other + 1)..DB_COUNT {
                if (*curtab).tp_diffbuf[i] != curbuf
                    && !(*curtab).tp_diffbuf[i].is_null()
                    && ((*eap).cmdidx != CMD_diffput || modifiable((*curtab).tp_diffbuf[i]))
                {
                    emsg(gettext(
                        c"E101: More than two buffers in diff mode, don't know which one to use"
                            .as_ptr(),
                    ));
                    return;
                }
            }
        } else {
            let mut p = (*eap).arg.add(strlen((*eap).arg));
            while p > (*eap).arg && ascii_iswhite(*p.sub(1)) {
                p = p.sub(1);
            }
            let mut i: usize = 0;
            while ascii_isdigit(*(*eap).arg.add(i)) && (*eap).arg.add(i) < p {
                i += 1;
            }
            let bufnr = if (*eap).arg.add(i) == p {
                atol((*eap).arg) as c_int
            } else {
                let nr = buflist_findpat((*eap).arg, p, false, true, false);
                if nr < 0 {
                    return;
                }
                nr
            };
            let buf = buflist_findnr(bufnr);
            if buf.is_null() {
                semsg!(
                    gettext(c"E102: Can't find buffer \"%s\"".as_ptr()),
                    (*eap).arg,
                );
                return;
            }
            if buf == curbuf {
                return;
            }
            let idx_o = diff_buf_idx(buf, curtab);
            if idx_o == DB_COUNT as c_int {
                semsg!(
                    gettext(c"E103: Buffer \"%s\" is not in diff mode".as_ptr()),
                    (*eap).arg,
                );
                return;
            }
            idx_other = idx_o as usize;
        }
        diff_busy = true;
        if (*eap).addr_count == 0 {
            let mut linestatus: c_int = 0;
            if (*eap).line1 == (*curbuf).b_ml.ml_line_count
                && diff_check_with_linestatus(curwin, (*eap).line1, &raw mut linestatus) == 0
                && linestatus == 0
                && ((*eap).line1 == 1
                    || (diff_check_with_linestatus(curwin, (*eap).line1 - 1, &raw mut linestatus)
                        >= 0
                        && linestatus == 0))
            {
                (*eap).line2 += 1;
            } else if (*eap).line1 > 0 {
                (*eap).line1 -= 1;
            }
        }
        let mut aco = CtxSwitch::new();
        if (*eap).cmdidx != CMD_diffget {
            ctx_switch(
                &raw mut aco,
                ptr::null_mut(),
                ptr::null_mut(),
                (*curtab).tp_diffbuf[idx_other],
                0,
            );
        }
        let idx_from = if (*eap).cmdidx == CMD_diffget {
            idx_other
        } else {
            idx_cur
        };
        let idx_to = if (*eap).cmdidx == CMD_diffget {
            idx_cur
        } else {
            idx_other
        };
        'theend: {
            if (*curbuf).b_changed == 0 {
                change_warning(curbuf, 0);
                if diff_buf_idx(curbuf, curtab) != idx_to as c_int {
                    emsg(gettext(e_buffer_changed_unexpectedly.as_ptr()));
                    ctx_restore(&raw mut aco);
                    break 'theend;
                }
            }
            diffgetput(
                (*eap).addr_count,
                idx_cur,
                idx_from,
                idx_to,
                (*eap).line1,
                (*eap).line2,
            );
            if (*eap).cmdidx != CMD_diffget {
                if KeyTyped {
                    u_sync(false);
                }
                ctx_restore(&raw mut aco);
            }
        }
        diff_busy = false;
        if diff_need_update {
            ex_diffupdate(ptr::null_mut());
        }
        check_cursor(curwin);
        changed_line_abv_curs();
        if (*curtab).tp_diffs.is_empty() {
            let mut wp = firstwin;
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_diff != 0
                    && *(*wp).w_onebuf_opt.wo_fdm == b'd' as c_char
                    && (*wp).w_onebuf_opt.wo_fen != 0
                {
                    foldUpdateAll(wp);
                }
                wp = (*wp).w_next;
            }
        }
        if diff_need_update {
            diff_need_update = false;
        } else {
            diff_redraw(false);
            window_layout_lock();
            apply_autocmds(
                EVENT_DIFFUPDATED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            window_layout_unlock();
        }
    }
}
unsafe fn diffgetput(
    addr_count: c_int,
    idx_cur: usize,
    idx_from: usize,
    idx_to: usize,
    line1: linenr_T,
    line2: linenr_T,
) {
    unsafe {
        let mut off: linenr_T = 0;
        let mut dp_idx: usize = 0;
        while dp_idx < (*curtab).tp_diffs.len() {
            if addr_count == 0 {
                while dp_idx + 1 < (*curtab).tp_diffs.len()
                    && (&(*curtab).tp_diffs)[dp_idx + 1].df_lnum[idx_cur]
                        == (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_cur]
                            + (&(*curtab).tp_diffs)[dp_idx].df_count[idx_cur]
                    && (&(*curtab).tp_diffs)[dp_idx + 1].df_lnum[idx_cur] == line1 + off + 1
                {
                    dp_idx += 1;
                }
            }
            if (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_cur] > line2 + off {
                break;
            }
            let mut dfree_lnum = [0; DB_COUNT];
            let mut dfree_count = [0; DB_COUNT];
            let mut did_free = false;
            let mut lnum = (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_to];
            let mut count = (&(*curtab).tp_diffs)[dp_idx].df_count[idx_to];
            let undo_bot = lnum + count + linenr_T::from(count == 0 && buf_is_empty(curbuf));
            if (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_cur]
                + (&(*curtab).tp_diffs)[dp_idx].df_count[idx_cur]
                > line1 + off
                && u_save(lnum - 1, undo_bot) != FAIL
            {
                let mut start_skip: linenr_T = 0;
                let mut end_skip: linenr_T = 0;
                if addr_count > 0 {
                    start_skip = line1 + off - (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_cur];
                    if start_skip > 0 {
                        if start_skip > count {
                            lnum += count;
                            count = 0;
                        } else {
                            count -= start_skip;
                            lnum += start_skip;
                        }
                    } else {
                        start_skip = 0;
                    }
                    end_skip = (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_cur]
                        + (&(*curtab).tp_diffs)[dp_idx].df_count[idx_cur]
                        - 1
                        - (line2 + off);
                    if end_skip > 0 {
                        if idx_cur == idx_from {
                            count = count.min(
                                (&(*curtab).tp_diffs)[dp_idx].df_count[idx_cur]
                                    - start_skip
                                    - end_skip,
                            );
                        } else {
                            count -= end_skip;
                            end_skip = ((&(*curtab).tp_diffs)[dp_idx].df_count[idx_from]
                                - start_skip
                                - count)
                                .max(0);
                        }
                    } else {
                        end_skip = 0;
                    }
                }
                let mut buf_empty = buf_is_empty(curbuf);
                let mut added: c_int = 0;
                for _ in 0..count {
                    buf_empty = (*curbuf).b_ml.ml_line_count == 1;
                    if ml_delete(lnum) == OK {
                        added -= 1;
                    }
                }
                let copy_count =
                    (&(*curtab).tp_diffs)[dp_idx].df_count[idx_from] - start_skip - end_skip;
                let from_lnum = (&(*curtab).tp_diffs)[dp_idx].df_lnum[idx_from];
                for i in 0..copy_count {
                    let nr = from_lnum + start_skip + i;
                    if nr > (*(*curtab).tp_diffbuf[idx_from]).b_ml.ml_line_count {
                        break;
                    }
                    let p = xstrdup(ml_get_buf((*curtab).tp_diffbuf[idx_from], nr));
                    let _ = ml_append(lnum + i - 1, p, 0, false);
                    xfree(p.cast());
                    added += 1;
                    if buf_empty && (*curbuf).b_ml.ml_line_count == 2 {
                        buf_empty = false;
                        let _ = ml_delete(2);
                    }
                }
                let new_count = (&(*curtab).tp_diffs)[dp_idx].df_count[idx_to] + added;
                (&mut (*curtab).tp_diffs)[dp_idx].df_count[idx_to] = new_count;
                let mut target_id = Some((&(*curtab).tp_diffs)[dp_idx].df_id);
                if start_skip == 0 && end_skip == 0 {
                    let mut i = 0;
                    while i < DB_COUNT {
                        if !(*curtab).tp_diffbuf[i].is_null()
                            && i != idx_from
                            && i != idx_to
                            && !diff_equal_entry(&(&(*curtab).tp_diffs)[dp_idx], idx_from, i)
                        {
                            break;
                        }
                        i += 1;
                    }
                    if i == DB_COUNT {
                        dfree_lnum = (&(*curtab).tp_diffs)[dp_idx].df_lnum;
                        dfree_count = (&(*curtab).tp_diffs)[dp_idx].df_count;
                        did_free = true;
                        target_id = (*curtab)
                            .tp_diffs
                            .as_slice()
                            .get(dp_idx + 1)
                            .map(|d| d.df_id);
                        diff_free(curtab, dp_idx);
                    }
                }
                if added != 0 {
                    mark_adjust(lnum, lnum + count - 1, MAXLNUM, added, kExtmarkNOOP);
                    if (*curwin).w_cursor.lnum >= lnum {
                        if (*curwin).w_cursor.lnum >= lnum + count {
                            (*curwin).w_cursor.lnum += added;
                            (*curwin).w_cursor.lnum =
                                (*curwin).w_cursor.lnum.min((*curbuf).b_ml.ml_line_count);
                        } else if added < 0 {
                            (*curwin).w_cursor.lnum = lnum;
                        }
                    }
                }
                extmark_adjust(curbuf, lnum, lnum + count - 1, MAXLNUM, added, kExtmarkUndo);
                changed_lines(curbuf, lnum, 0, lnum + count, added, true);
                if did_free {
                    diff_fold_update(&dfree_lnum, &dfree_count, idx_to);
                }
                if added != 0 {
                    if let Some(new_idx) = valid_diff(target_id) {
                        dp_idx = new_idx;
                    } else {
                        break;
                    }
                }
                if !did_free {
                    (&mut (*curtab).tp_diffs)[dp_idx].df_count[idx_to] = new_count;
                }
                if idx_cur == idx_to {
                    off += added;
                }
            }
            if !did_free {
                dp_idx += 1;
            }
        }
    }
}
unsafe fn diff_fold_update(
    df_lnum: &[linenr_T; DB_COUNT],
    df_count: &[linenr_T; DB_COUNT],
    skip_idx: usize,
) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            for i in 0..DB_COUNT {
                if (*curtab).tp_diffbuf[i] == (*wp).w_buffer && i != skip_idx {
                    foldUpdate(wp, df_lnum[i], df_lnum[i] + df_count[i]);
                }
            }
            wp = (*wp).w_next;
        }
    }
}
pub unsafe fn diff_mode_buf(buf: *mut buf_T) -> bool {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            if diff_buf_idx(buf, tp) != DB_COUNT as c_int {
                return true;
            }
            tp = (*tp).tp_next;
        }
        false
    }
}
pub unsafe fn diff_move_to(dir: c_int, mut count: c_int) -> c_int {
    unsafe {
        let mut lnum = (*curwin).w_cursor.lnum;
        let idx = diff_buf_idx(curbuf, curtab);
        if idx == DB_COUNT as c_int || (*curtab).tp_diffs.is_empty() {
            return FAIL;
        }
        let u_idx = idx as usize;
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        if (*curtab).tp_diffs.is_empty() {
            return FAIL;
        }
        while {
            count -= 1;
            count >= 0
        } {
            let diffs = &(*curtab).tp_diffs;
            if dir == BACKWARD && lnum <= diffs[0].df_lnum[u_idx] {
                break;
            }
            for di in 0..diffs.len() {
                if (dir == FORWARD && lnum < diffs[di].df_lnum[u_idx])
                    || (dir == BACKWARD
                        && (di + 1 == diffs.len() || lnum <= diffs[di + 1].df_lnum[u_idx]))
                {
                    lnum = diffs[di].df_lnum[u_idx];
                    break;
                }
            }
        }
        lnum = lnum.min((*curbuf).b_ml.ml_line_count);
        if lnum == (*curwin).w_cursor.lnum {
            return FAIL;
        }
        setpcmark();
        (*curwin).w_cursor.lnum = lnum;
        (*curwin).w_cursor.col = 0;
        OK
    }
}
unsafe fn diff_get_corresponding_line_int(buf1: *mut buf_T, lnum1: linenr_T) -> linenr_T {
    unsafe {
        let mut baseline: linenr_T = 0;
        let idx1 = diff_buf_idx(buf1, curtab);
        let idx2 = diff_buf_idx(curbuf, curtab);
        if idx1 == DB_COUNT as c_int
            || idx2 == DB_COUNT as c_int
            || (*curtab).tp_diffs.is_empty()
        {
            return lnum1;
        }
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        if (*curtab).tp_diffs.is_empty() {
            return lnum1;
        }
        let u_idx1 = idx1 as usize;
        let u_idx2 = idx2 as usize;
        for dp in &(*curtab).tp_diffs {
            if dp.df_lnum[u_idx1] > lnum1 {
                return lnum1 - baseline;
            }
            if dp.df_lnum[u_idx1] + dp.df_count[u_idx1] > lnum1 {
                baseline = lnum1 - dp.df_lnum[u_idx1];
                baseline = baseline.min(dp.df_count[u_idx2]);
                return dp.df_lnum[u_idx2] + baseline;
            }
            if dp.df_lnum[u_idx1] == lnum1
                && dp.df_count[u_idx1] == 0
                && dp.df_lnum[u_idx2] <= (*curwin).w_cursor.lnum
                && dp.df_lnum[u_idx2] + dp.df_count[u_idx2] > (*curwin).w_cursor.lnum
            {
                return (*curwin).w_cursor.lnum;
            }
            baseline = (dp.df_lnum[u_idx1] + dp.df_count[u_idx1])
                - (dp.df_lnum[u_idx2] + dp.df_count[u_idx2]);
        }
        lnum1 - baseline
    }
}
pub unsafe fn diff_get_corresponding_line(buf1: *mut buf_T, lnum1: linenr_T) -> linenr_T {
    unsafe {
        let lnum = diff_get_corresponding_line_int(buf1, lnum1);
        lnum.min((*curbuf).b_ml.ml_line_count)
    }
}
pub unsafe fn diff_lnum_win(lnum: linenr_T, wp: *mut win_T) -> linenr_T {
    unsafe {
        let idx = diff_buf_idx(curbuf, curtab);
        if idx == DB_COUNT as c_int {
            return 0;
        }
        let u_idx = idx as usize;
        if (*curtab).tp_diff_invalid != 0 {
            ex_diffupdate(ptr::null_mut());
        }
        let Some(dp) = (*curtab)
            .tp_diffs
            .iter()
            .find(|dp| lnum <= dp.df_lnum[u_idx] + dp.df_count[u_idx])
        else {
            return (*(*wp).w_buffer).b_ml.ml_line_count - ((*curbuf).b_ml.ml_line_count - lnum);
        };
        let i = diff_buf_idx((*wp).w_buffer, curtab);
        if i == DB_COUNT as c_int {
            return 0;
        }
        let u_i = i as usize;
        let n = lnum + (dp.df_lnum[u_i] - dp.df_lnum[u_idx]);
        n.min(dp.df_lnum[u_i] + dp.df_count[u_i])
    }
}
unsafe fn parse_diff_ed(line: *mut c_char, hunk: *mut diffhunk_T) -> c_int {
    unsafe {
        let mut p = line;
        let f1 = getdigits_int32(&raw mut p, true, 0);
        let l1 = if *p == b',' as c_char {
            p = p.add(1);
            getdigits_int(&raw mut p, true, 0)
        } else {
            f1
        };
        if *p != b'a' as c_char && *p != b'c' as c_char && *p != b'd' as c_char {
            return FAIL;
        }
        let difftype = *p as u8;
        p = p.add(1);
        let f2 = getdigits_int(&raw mut p, true, 0);
        let l2 = if *p == b',' as c_char {
            p = p.add(1);
            getdigits_int(&raw mut p, true, 0)
        } else {
            f2
        };
        if l1 < f1 || l2 < f2 {
            return FAIL;
        }
        if difftype == b'a' {
            (*hunk).lnum_orig = f1 + 1;
            (*hunk).count_orig = 0;
        } else {
            (*hunk).lnum_orig = f1;
            (*hunk).count_orig = l1 - f1 + 1;
        }
        if difftype == b'd' {
            (*hunk).lnum_new = f2 + 1;
            (*hunk).count_new = 0;
        } else {
            (*hunk).lnum_new = f2;
            (*hunk).count_new = l2 - f2 + 1;
        }
        OK
    }
}
unsafe fn parse_diff_unified(line: *mut c_char, hunk: *mut diffhunk_T) -> c_int {
    unsafe {
        let mut p = line;
        if *p == b'@' as c_char
            && *p.add(1) == b'@' as c_char
            && *p.add(2) == b' ' as c_char
            && *p.add(3) == b'-' as c_char
        {
            p = p.add(4);
            let mut oldline = getdigits_int32(&raw mut p, true, 0);
            let oldcount = if *p == b',' as c_char {
                p = p.add(1);
                getdigits_int(&raw mut p, true, 0)
            } else {
                1
            };
            if *p == b' ' as c_char && *p.add(1) == b'+' as c_char {
                p = p.add(2);
                let mut newline = getdigits_int(&raw mut p, true, 0);
                let newcount = if *p == b',' as c_char {
                    p = p.add(1);
                    getdigits_int(&raw mut p, true, 0)
                } else {
                    1
                };
                if oldcount == 0 {
                    oldline += 1;
                }
                if newcount == 0 {
                    newline += 1;
                }
                if newline == 0 {
                    newline = 1;
                }
                (*hunk).lnum_orig = oldline;
                (*hunk).count_orig = oldcount;
                (*hunk).lnum_new = newline;
                (*hunk).count_new = newcount;
                return OK;
            }
            return FAIL;
        }
        FAIL
    }
}
unsafe fn xdiff_out(
    start_a: c_int,
    count_a: c_int,
    start_b: c_int,
    count_b: c_int,
    priv_data: *mut c_void,
) -> c_int {
    unsafe {
        let dout = priv_data.cast::<diffout_T>();
        (*dout).dout_ga.push(diffhunk_T {
            lnum_orig: start_a + 1,
            count_orig: count_a,
            lnum_new: start_b + 1,
            count_new: count_b,
        });
        0
    }
}
pub unsafe fn f_diff_filler(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number =
            diff_check_fill(curwin, tv_get_lnum(argvars)).max(0) as varnumber_T;
    }
}
pub unsafe fn f_diff_hlID(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut diffline = diffline_T {
            changes: ptr::null_mut(),
            num_changes: 0,
            bufidx: 0,
            lineoff: 0,
        };
        let cache_results = (diff_flags & ALL_INLINE_DIFF) == 0;
        let mut lnum = tv_get_lnum(argvars);
        if lnum < 0 {
            lnum = 0;
        }
        if !cache_results
            || lnum != f_diff_hlid_prev_lnum
            || f_diff_hlid_changedtick != buf_get_changedtick(curbuf)
            || f_diff_hlid_fnum != (*curbuf).b_fnum
            || diff_flags != f_diff_hlid_prev_diff_flags
        {
            let mut linestatus: c_int = 0;
            diff_check_with_linestatus(curwin, lnum, &raw mut linestatus);
            if linestatus < 0 {
                if linestatus == -1 {
                    f_diff_hlid_change_start = MAXCOL;
                    f_diff_hlid_change_end = -1;
                    if diff_find_change(curwin, lnum, &raw mut diffline) {
                        f_diff_hlid_hlID = HLF_ADD;
                    } else {
                        f_diff_hlid_hlID = HLF_CHD;
                        if diffline.num_changes > 0 && cache_results {
                            f_diff_hlid_change_start =
                                (*diffline.changes).dc_start[diffline.bufidx as usize];
                            f_diff_hlid_change_end =
                                (*diffline.changes).dc_end[diffline.bufidx as usize];
                        }
                    }
                } else {
                    f_diff_hlid_hlID = HLF_ADD;
                }
            } else {
                f_diff_hlid_hlID = 0;
            }
            if cache_results {
                f_diff_hlid_prev_lnum = lnum;
                f_diff_hlid_changedtick = buf_get_changedtick(curbuf);
                f_diff_hlid_fnum = (*curbuf).b_fnum;
                f_diff_hlid_prev_diff_flags = diff_flags;
            }
        }
        if f_diff_hlid_hlID == HLF_CHD || f_diff_hlid_hlID == HLF_TXD {
            let col = tv_get_number(argvars.add(1)) as c_int - 1;
            if cache_results {
                if col >= f_diff_hlid_change_start && col < f_diff_hlid_change_end {
                    f_diff_hlid_hlID = HLF_TXD;
                } else {
                    f_diff_hlid_hlID = HLF_CHD;
                }
            } else {
                f_diff_hlid_hlID = HLF_CHD;
                for i in 0..diffline.num_changes {
                    let added = diff_change_parse(
                        &raw mut diffline,
                        diffline.changes.add(i as usize),
                        &raw mut f_diff_hlid_change_start,
                        &raw mut f_diff_hlid_change_end,
                    );
                    if col >= f_diff_hlid_change_start && col < f_diff_hlid_change_end {
                        f_diff_hlid_hlID = if added { HLF_TXA } else { HLF_TXD };
                        break;
                    }
                    if col < f_diff_hlid_change_start {
                        break;
                    }
                }
            }
        }
        (*rettv).vval.v_number = f_diff_hlid_hlID as varnumber_T;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_diff_ed_and_unified_work() {
        unsafe {
            let mut hunk = diffhunk_T {
                lnum_orig: 0,
                count_orig: 0,
                lnum_new: 0,
                count_new: 0,
            };
            let mut ed_line = *b"2,4c5,6\n\0";
            assert_eq!(
                parse_diff_ed(ed_line.as_mut_ptr().cast(), &raw mut hunk),
                OK
            );
            assert_eq!(hunk.lnum_orig, 2);
            assert_eq!(hunk.count_orig, 3);
            assert_eq!(hunk.lnum_new, 5);
            assert_eq!(hunk.count_new, 2);
            let mut uni_line = *b"@@ -10,3 +12,4 @@\n\0";
            assert_eq!(
                parse_diff_unified(uni_line.as_mut_ptr().cast(), &raw mut hunk),
                OK
            );
            assert_eq!(hunk.lnum_orig, 10);
            assert_eq!(hunk.count_orig, 3);
            assert_eq!(hunk.lnum_new, 12);
            assert_eq!(hunk.count_new, 4);
        }
    }
}
