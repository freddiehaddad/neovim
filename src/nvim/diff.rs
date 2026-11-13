//! Diff mode functions (`src/nvim/diff.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{boolean_obj, static_cstr_as_obj};
use crate::autocmd::{EVENT_DIFFUPDATED, apply_autocmds};
use crate::buffer::{
    Bufref, FileBuf, MAXLNUM, bt_prompt, buf_get_changedtick, buf_is_empty, buf_valid,
    buflist_findnr, buflist_findpat, bufref_valid, modifiable, set_bufref,
};
use crate::bufwrite::buf_write;
use crate::change::{change_warning, changed_lines};
use crate::charset::{ascii_isdigit, ascii_iswhite, getdigits_int, getdigits_int32};
use crate::cmdexpand::Cmdidx;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{Colnr, Linenr, check_cursor};
use crate::decoration::decor_conceal_line;
use crate::drawscreen::{UPD_NOT_VALID, UPD_SOME_VALID, UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, K_CALLBACK_NONE_CONST, TypVal, Varnumber, tv_get_lnum, tv_get_number,
};
use crate::eval::vars::{eval_diff, eval_patch};
use crate::ex_cmds::{append_redir, ex_file};
use crate::ex_docmd::{ADDR_LINES, CMOD_LOCKMARKS, Exarg, do_cmdline_cmd, do_exedit, get_address};
use crate::ex_getln::Hlf;
use crate::extmark::{TriState, extmark_adjust};
use crate::fileio::{buf_check_timestamp, vim_fgets, vim_tempname};
#[cfg(unix)]
use crate::fileio::{shorten_fnames, vim_gettempdir};
use crate::fold::{
    fold_update, fold_update_all, foldmethod_is_diff, foldmethod_is_manual, has_folding,
    new_fold_level,
};
use crate::globals::*;
use crate::linematch::linematch_nbuffers;
use crate::mark::{mark_adjust, setpcmark};
use crate::mbyte::{mb_get_class_tab, mb_stricmp, utf_char2bytes, utf_char2len, utf_fold};
use crate::memline::{ml_append, ml_delete, ml_get_buf, ml_get_buf_len};
use crate::message::{emsg, semsg};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, changed_window_setting, check_topfill,
    invalidate_botline_win, set_topline_valid, validate_cursor,
};
use crate::normal::check_scrollbind;
use crate::option::{OptTarget, set_option_direct_for, set_option_value_give_err};
use crate::option_vars::*;
use crate::optionstr::{OptIndex, OptKeyDict_dip, opt_keyset};
use crate::os::env::{os_env_exists, os_unsetenv};
use crate::os::fs::{CFile, FileInfo, os_fileinfo, os_fileinfo_size, os_remove};
use crate::os::shell::call_shell;
use crate::strings::{format_to_buf, gettext, vim_strsave_shellescape};
use crate::ui::vim_beep;
use crate::undo::{u_save, u_sync};
use crate::window::{
    TabPageInfo, WSP_VERT, WinInfoInfo, frames_locked, scroll_to_fraction, set_fraction, win_split,
    win_valid, window_layout_lock, window_layout_unlock,
};
use crate::xdiff::xdiffi::xdl_diff;
use crate::xdiff::{Mmfile, Xdemitcb, Xdemitconf, Xpparam};
use std::ffi::CString;
const DB_COUNT: usize = 8;
const MAX_DIFF_ANCHORS: usize = 20;
const LBUFLEN: usize = 50;
const MAX_XDIFF_SIZE: i32 = 1024 * 1024 * 1023;
const DIFF_FILLER: i32 = 0x001;
const DIFF_IBLANK: i32 = 0x002;
const DIFF_ICASE: i32 = 0x004;
const DIFF_IWHITE: i32 = 0x008;
const DIFF_IWHITEALL: i32 = 0x010;
const DIFF_IWHITEEOL: i32 = 0x020;
const DIFF_HORIZONTAL: i32 = 0x040;
const DIFF_VERTICAL: i32 = 0x080;
const DIFF_HIDDEN_OFF: i32 = 0x100;
const DIFF_INTERNAL: i32 = 0x200;
const DIFF_CLOSE_OFF: i32 = 0x400;
const DIFF_FOLLOWWRAP: i32 = 0x800;
const DIFF_LINEMATCH: i32 = 0x1000;
const DIFF_INLINE_NONE: i32 = 0x2000;
const DIFF_INLINE_SIMPLE: i32 = 0x4000;
const DIFF_INLINE_CHAR: i32 = 0x8000;
const DIFF_INLINE_WORD: i32 = 0x10000;
const DIFF_ANCHOR: i32 = 0x20000;
const ALL_WHITE_DIFF: i32 = DIFF_IWHITE | DIFF_IWHITEALL | DIFF_IWHITEEOL;
const ALL_INLINE: i32 = DIFF_INLINE_NONE | DIFF_INLINE_SIMPLE | DIFF_INLINE_CHAR | DIFF_INLINE_WORD;
const ALL_INLINE_DIFF: i32 = DIFF_INLINE_CHAR | DIFF_INLINE_WORD;
const XDF_NEED_MINIMAL: i32 = 1 << 0;
pub(crate) const XDF_IGNORE_WHITESPACE: u64 = 1 << 1;
pub(crate) const XDF_IGNORE_WHITESPACE_CHANGE: u64 = 1 << 2;
pub(crate) const XDF_IGNORE_WHITESPACE_AT_EOL: u64 = 1 << 3;
pub(crate) const XDF_IGNORE_BLANK_LINES: u64 = 1 << 7;
const XDF_PATIENCE_DIFF: i32 = 1 << 14;
const XDF_HISTOGRAM_DIFF: i32 = 1 << 15;
const XDF_INDENT_HEURISTIC: i32 = 1 << 23;
pub(crate) const K_OPT_DIFF: OptIndex = 70;
pub(crate) const K_OPT_DIFFOPT: OptIndex = 73;
pub(crate) const K_OPT_FOLDMETHOD: OptIndex = 109;
pub(crate) const K_SHELL_OPT_FILTER: i32 = 1;
pub(crate) const K_SHELL_OPT_DO_OUT: i32 = 4;
pub(crate) const K_SHELL_OPT_SILENT: i32 = 8;
pub(crate) const K_OPT_BO_FLAG_OPERATOR: u32 = 0x4000;
pub(crate) const HLF_ADD: Hlf = 30;
pub(crate) const HLF_CHD: Hlf = 31;
pub(crate) const HLF_TXD: Hlf = 33;
pub(crate) const HLF_TXA: Hlf = 34;
const KEYSET_OPTIDX_DIP_INLINE: u64 = 5;
const KEYSET_OPTIDX_DIP_CONTEXT: u64 = 7;
const KEYSET_OPTIDX_DIP_LINEMATCH: u64 = 11;
const KEYSET_OPTIDX_DIP_ALGORITHM: u64 = 13;
const KEYSET_OPTIDX_DIP_FOLDCOLUMN: u64 = 16;
#[repr(C)]
pub struct DiffBlock {
    pub df_id: u64,
    pub df_lnum: [Linenr; DB_COUNT],
    pub df_count: [Linenr; DB_COUNT],
    pub is_linematched: bool,
    pub has_changes: bool,
    pub df_changes: Vec<DifflineChange>,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DifflineChange {
    pub dc_start: [Colnr; DB_COUNT],
    pub dc_end: [Colnr; DB_COUNT],
    pub dc_start_lnum_off: [i32; DB_COUNT],
    pub dc_end_lnum_off: [i32; DB_COUNT],
}
#[repr(C)]
#[derive(Clone, Default)]
pub struct Diffline {
    pub changes: Vec<DifflineChange>,
    pub is_simple: bool,
    pub bufidx: i32,
    pub lineoff: i32,
}
#[repr(C)]
struct Diffin {
    din_fname: Option<CString>,
    din_buf: Vec<u8>,
}
#[repr(C)]
pub(crate) struct Diffout {
    dout_fname: Option<CString>,
    dout_ga: Vec<Diffhunk>,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Diffhunk {
    lnum_orig: Linenr,
    count_orig: i32,
    lnum_new: Linenr,
    count_new: i32,
}
#[repr(C)]
struct Diffio {
    dio_orig: Diffin,
    dio_new: Diffin,
    dio_diff: Diffout,
    dio_internal: i32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Diffstyle {
    DiffEd,
    DiffUnified,
    DiffNone,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct LinemapEntry {
    byte_start: Colnr,
    num_bytes: Colnr,
    lineoff: i32,
}
use crate::arglist::ML_EMPTY;
use crate::charset::{NL, NUL};
use crate::cmdexpand::MB_MAXBYTES;
use crate::extmark::{K_EXTMARK_NOOP, K_EXTMARK_UNDO, K_FALSE, K_NONE, K_TRUE};
use crate::option::OPT_LOCAL;
#[cfg(unix)]
use crate::os::fs::MAXPATHL;
use crate::regexp::MAXCOL;
use crate::search::{BACKWARD, FORWARD};
const DIFF_WORD_GAP: i32 = 5;

struct DiffState {
    diff_busy: std::cell::Cell<bool>,
    diff_need_update: std::cell::Cell<bool>,
    diff_flags: std::cell::Cell<i32>,
    diff_algorithm: std::cell::Cell<i32>,
    linematch_lines: std::cell::Cell<i32>,
    diff_a_works: std::cell::Cell<TriState>,
    next_diff_id: std::cell::Cell<u64>,
    diff_finger_dp_idx: std::cell::Cell<Option<usize>>,
    diff_finger_tp: crate::api::private::validate::CloneCell<crate::globals::TabpagePtr>,
    diff_finger_idx: std::cell::Cell<i32>,
    diff_finger_lnum: std::cell::Cell<Linenr>,
    f_diff_hlid_prev_lnum: std::cell::Cell<Linenr>,
    f_diff_hlid_changedtick: std::cell::Cell<Varnumber>,
    f_diff_hlid_fnum: std::cell::Cell<i32>,
    f_diff_hlid_prev_diff_flags: std::cell::Cell<i32>,
    f_diff_hlid_change_start: std::cell::Cell<i32>,
    f_diff_hlid_change_end: std::cell::Cell<i32>,
    f_diff_hlid_hl_id: std::cell::Cell<Hlf>,
}

impl DiffState {
    const fn new() -> Self {
        Self {
            diff_busy: std::cell::Cell::new(false),
            diff_need_update: std::cell::Cell::new(false),
            diff_flags: std::cell::Cell::new(
                DIFF_INTERNAL | DIFF_FILLER | DIFF_CLOSE_OFF | DIFF_LINEMATCH | DIFF_INLINE_CHAR,
            ),
            diff_algorithm: std::cell::Cell::new(XDF_INDENT_HEURISTIC),
            linematch_lines: std::cell::Cell::new(40),
            diff_a_works: std::cell::Cell::new(K_NONE),
            next_diff_id: std::cell::Cell::new(1),
            diff_finger_dp_idx: std::cell::Cell::new(None),
            diff_finger_tp: crate::api::private::validate::CloneCell::new(
                crate::globals::TabpagePtr::NULL,
            ),
            diff_finger_idx: std::cell::Cell::new(-1),
            diff_finger_lnum: std::cell::Cell::new(0),
            f_diff_hlid_prev_lnum: std::cell::Cell::new(0),
            f_diff_hlid_changedtick: std::cell::Cell::new(0),
            f_diff_hlid_fnum: std::cell::Cell::new(0),
            f_diff_hlid_prev_diff_flags: std::cell::Cell::new(0),
            f_diff_hlid_change_start: std::cell::Cell::new(0),
            f_diff_hlid_change_end: std::cell::Cell::new(0),
            f_diff_hlid_hl_id: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static DIFF_STATE: DiffState = const { DiffState::new() };
}

#[inline]
fn diff_st<R>(f: impl FnOnce(&DiffState) -> R) -> R {
    DIFF_STATE.with(f)
}

#[inline]
fn diff_flags() -> i32 {
    diff_st(|st| st.diff_flags.get())
}

#[inline]
const fn has_dip_key(v: &OptKeyDict_dip, idx: u64) -> bool {
    (v.is_set_dip & (1u64 << idx)) != 0
}
fn diff_scan_start(tp: &TabPageInfo, idx: i32, lnum: Linenr) -> usize {
    let tp_ptr = crate::globals::TabpagePtr::from(tp);
    let dp_len = tp.tp_diffs.borrow().len();
    diff_st(|st| {
        if let Some(dp_idx) = st.diff_finger_dp_idx.get()
            && st.diff_finger_tp.get() == tp_ptr
            && st.diff_finger_idx.get() == idx
            && lnum >= st.diff_finger_lnum.get()
            && dp_idx < dp_len
        {
            dp_idx
        } else {
            0
        }
    })
}
fn diff_scan_remember(tp: &TabPageInfo, idx: i32, lnum: Linenr, dp_idx: Option<usize>) {
    let tp_ptr = crate::globals::TabpagePtr::from(tp);
    diff_st(|st| {
        st.diff_finger_tp.set(tp_ptr);
        st.diff_finger_idx.set(idx);
        st.diff_finger_lnum.set(lnum);
        st.diff_finger_dp_idx.set(Some(dp_idx.unwrap_or(0)));
    });
}
pub fn diff_buf_delete(buf: &FileBuf) {
    for tp in for_all_tabpages_ptr() {
        let i = diff_buf_idx(buf, &tp);
        if i != DB_COUNT as i32 {
            tp.tp_diffbuf[i as usize].set(crate::globals::BufPtr::NULL);
            tp.tp_diff_invalid.set(1);
            if g_st().curtab.get() == tp {
                g_st().need_diff_redraw.set(true);
                redraw_later(Some(&crate::globals::curwin()), UPD_VALID);
            }
        }
    }
}
pub fn diff_buf_adjust(win: &WinInfoInfo) {
    if !win.w_onebuf_opt.wo_diff.get() {
        let mut found_win = false;
        for wp in for_all_windows_ptr() {
            if wp.w_buffer.get() == win.w_buffer.get() && wp.w_onebuf_opt.wo_diff.get() {
                found_win = true;
            }
        }
        if !found_win {
            let i = diff_buf_idx(&win.buf(), &curtab());
            if i != DB_COUNT as i32 {
                curtab().tp_diffbuf[i as usize].set(crate::globals::BufPtr::NULL);
                curtab().tp_diff_invalid.set(1);
                diff_redraw(true);
            }
        }
    } else {
        diff_buf_add(&win.buf());
    }
}
pub fn diff_buf_add(buf: &FileBuf) {
    if diff_buf_idx(buf, &curtab()) != DB_COUNT as i32 {
        return;
    }
    for i in 0..DB_COUNT {
        if curtab().tp_diffbuf[i].get().is_null() {
            curtab().tp_diffbuf[i].set(crate::globals::BufPtr::from(buf));
            curtab().tp_diff_invalid.set(1);
            diff_redraw(true);
            return;
        }
    }
    semsg!(
        gettext(c"E96: Cannot diff more than %d buffers"),
        DB_COUNT as i32,
    );
}
fn diff_buf_clear() {
    for i in 0..DB_COUNT {
        if !curtab().tp_diffbuf[i].get().is_null() {
            curtab().tp_diffbuf[i].set(crate::globals::BufPtr::NULL);
            curtab().tp_diff_invalid.set(1);
            diff_redraw(true);
        }
    }
}
fn diff_buf_idx(buf: &FileBuf, tp: &TabPageInfo) -> i32 {
    let mut idx: usize = 0;
    while idx < DB_COUNT {
        if tp.tp_diffbuf[idx].get() == buf {
            break;
        }
        idx += 1;
    }
    idx as i32
}
pub fn diff_invalidate(buf: &FileBuf) {
    for tp in for_all_tabpages_ptr() {
        let i = diff_buf_idx(buf, &tp);
        if i != DB_COUNT as i32 {
            tp.tp_diff_invalid.set(1);
            if g_st().curtab.get() == tp {
                diff_redraw(true);
            }
        }
    }
}
pub fn diff_mark_adjust(
    buf: &FileBuf,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    for tp in for_all_tabpages_ptr() {
        let idx = diff_buf_idx(buf, &tp);
        if idx != DB_COUNT as i32 {
            diff_mark_adjust_tp(&tp, idx as usize, line1, line2, amount, amount_after);
        }
    }
}
fn diff_mark_adjust_tp(
    tp: &TabPageInfo,
    idx: usize,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    diff_st(|st| st.diff_finger_dp_idx.set(None));
    if diff_internal() != 0 {
        tp.tp_diff_invalid.set(1);
        tp.tp_diff_update.set(1);
    }
    let inserted: Linenr;
    let mut deleted: Linenr;
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
        let dp_past_line2 = {
            let diffs = tp.tp_diffs.borrow();
            dp_idx >= diffs.len()
                || diffs[dp_idx].df_lnum[idx] - 1 > line2
                || (line2 == MAXLNUM && diffs[dp_idx].df_lnum[idx] > line1)
        };
        let dprev_before_line1 = match dprev_idx {
            None => true,
            Some(pi) => {
                let diffs = tp.tp_diffs.borrow();
                diffs[pi].df_lnum[idx] + diffs[pi].df_count[idx] < line1
            }
        };
        let busy = diff_st(|st| st.diff_busy.get());
        if dp_past_line2 && dprev_before_line1 && !busy {
            let dprev_copy = dprev_idx.map(|pi| {
                let diffs = tp.tp_diffs.borrow();
                (diffs[pi].df_lnum, diffs[pi].df_count)
            });
            let dnext_idx = diff_alloc_new(tp, dp_idx);
            dp_idx += 1;
            {
                let mut diffs = tp.tp_diffs.borrow_mut();
                diffs[dnext_idx].df_lnum[idx] = line1;
                diffs[dnext_idx].df_count[idx] = inserted;
                for i in 0..DB_COUNT {
                    if !tp.tp_diffbuf[i].get().is_null() && i != idx {
                        diffs[dnext_idx].df_lnum[i] = match dprev_copy {
                            None => line1,
                            Some((prev_lnum, prev_count)) => {
                                line1 + (prev_lnum[i] + prev_count[i])
                                    - (prev_lnum[idx] + prev_count[idx])
                            }
                        };
                        diffs[dnext_idx].df_count[i] = deleted;
                    }
                }
            }
        }
        if dp_idx >= tp.tp_diffs.borrow().len() {
            break;
        }
        let last = {
            let diffs = tp.tp_diffs.borrow();
            diffs[dp_idx].df_lnum[idx] + diffs[dp_idx].df_count[idx] - 1
        };
        if last >= line1 - 1 {
            if busy {
                let mut diffs = tp.tp_diffs.borrow_mut();
                if diffs[dp_idx].df_lnum[idx] > line2 {
                    diffs[dp_idx].df_lnum[idx] += amount_after;
                }
                dprev_idx = Some(dp_idx);
                dp_idx += 1;
                continue;
            }
            if tp.tp_diffs.borrow()[dp_idx].df_lnum[idx] - Linenr::from(deleted + inserted != 0)
                > line2
            {
                if amount_after == 0 {
                    break;
                }
                tp.tp_diffs.borrow_mut()[dp_idx].df_lnum[idx] += amount_after;
            } else {
                let mut check_unchanged = false;
                if deleted > 0 {
                    let mut n: Linenr;
                    let mut off: Linenr = 0;
                    let next_lnum = tp
                        .tp_diffs
                        .borrow()
                        .as_slice()
                        .get(dp_idx + 1)
                        .map(|nd| nd.df_lnum[idx]);
                    let mut diffs = tp.tp_diffs.borrow_mut();
                    let dp = &mut diffs[dp_idx];
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
                        if !tp.tp_diffbuf[i].get().is_null() && i != idx {
                            if dp.df_lnum[i] > off {
                                dp.df_lnum[i] -= off;
                            } else {
                                dp.df_lnum[i] = 1;
                            }
                            dp.df_count[i] += n;
                        }
                    }
                } else if tp.tp_diffs.borrow()[dp_idx].df_lnum[idx] <= line1 {
                    tp.tp_diffs.borrow_mut()[dp_idx].df_count[idx] += inserted;
                    check_unchanged = true;
                } else {
                    tp.tp_diffs.borrow_mut()[dp_idx].df_lnum[idx] += inserted;
                }
                if check_unchanged {
                    diff_check_unchanged(tp, dp_idx);
                }
            }
        }
        let can_merge = if let Some(pi) = dprev_idx {
            let diffs = tp.tp_diffs.borrow();
            !diffs[dp_idx].is_linematched
                && !diff_st(|st| st.diff_busy.get())
                && diffs[pi].df_lnum[idx] + diffs[pi].df_count[idx] == diffs[dp_idx].df_lnum[idx]
        } else {
            false
        };
        if let Some(pi) = dprev_idx
            && can_merge
        {
            let dp_counts = tp.tp_diffs.borrow()[dp_idx].df_count;
            {
                let mut diffs = tp.tp_diffs.borrow_mut();
                for (i, &c) in dp_counts.iter().enumerate() {
                    if !tp.tp_diffbuf[i].get().is_null() {
                        diffs[pi].df_count[i] += c;
                    }
                }
            }
            diff_free(tp, dp_idx);
        } else {
            dprev_idx = Some(dp_idx);
            dp_idx += 1;
        }
    }
    let mut i_dp = 0;
    while i_dp < tp.tp_diffs.borrow().len() {
        let mut i = 0;
        while i < DB_COUNT {
            if !tp.tp_diffbuf[i].get().is_null() && tp.tp_diffs.borrow()[i_dp].df_count[i] != 0 {
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
    if g_st().curtab.get() == tp {
        g_st().need_diff_redraw.set(true);
        g_st().diff_need_scrollbind.set(true);
    }
}
fn diff_alloc_new(tp: &TabPageInfo, idx: usize) -> usize {
    let df_id = diff_st(|st| {
        let id = st.next_diff_id.get();
        st.next_diff_id.set(id + 1);
        st.diff_finger_dp_idx.set(None);
        id
    });
    tp.tp_diffs.borrow_mut().insert(
        idx,
        DiffBlock {
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
fn diff_free(tp: &TabPageInfo, idx: usize) {
    diff_st(|st| st.diff_finger_dp_idx.set(None));
    tp.tp_diffs.borrow_mut().remove(idx);
}
fn diff_check_unchanged(tp: &TabPageInfo, dp_idx: usize) {
    let mut i_org = 0;
    while i_org < DB_COUNT {
        if !tp.tp_diffbuf[i_org].get().is_null() {
            break;
        }
        i_org += 1;
    }
    if i_org == DB_COUNT {
        return;
    }
    if !diff_check_sanity(tp, &tp.tp_diffs.borrow()[dp_idx]) {
        return;
    }
    let mut off_org: Linenr = 0;
    let mut off_new: Linenr = 0;
    let mut dir = FORWARD;
    loop {
        while tp.tp_diffs.borrow()[dp_idx].df_count[i_org] > 0 {
            if dir == BACKWARD {
                off_org = tp.tp_diffs.borrow()[dp_idx].df_count[i_org] - 1;
            }
            let lnum_org = tp.tp_diffs.borrow()[dp_idx].df_lnum[i_org] + off_org;
            let line_org = crate::memory::bytes_to_cstring(&crate::memline::ml_get_buf_slice(
                &tp.diffbuf(i_org),
                lnum_org,
            ));
            let mut i_new = i_org + 1;
            while i_new < DB_COUNT {
                if tp.tp_diffbuf[i_new].get().is_null() {
                    i_new += 1;
                    continue;
                }
                if dir == BACKWARD {
                    off_new = tp.tp_diffs.borrow()[dp_idx].df_count[i_new] - 1;
                }
                if off_new < 0 || off_new >= tp.tp_diffs.borrow()[dp_idx].df_count[i_new] {
                    break;
                }
                let lnum_new = tp.tp_diffs.borrow()[dp_idx].df_lnum[i_new] + off_new;
                if diff_cmp(
                    line_org.to_bytes(),
                    &crate::memline::ml_get_buf_slice(&tp.diffbuf(i_new), lnum_new),
                ) != 0
                {
                    break;
                }
                i_new += 1;
            }
            if i_new != DB_COUNT {
                break;
            }
            let mut diffs = tp.tp_diffs.borrow_mut();
            for idx in i_org..DB_COUNT {
                if !tp.tp_diffbuf[idx].get().is_null() {
                    if dir == FORWARD {
                        diffs[dp_idx].df_lnum[idx] += 1;
                    }
                    diffs[dp_idx].df_count[idx] -= 1;
                }
            }
        }
        if dir == BACKWARD {
            break;
        }
        dir = BACKWARD;
    }
}
fn diff_check_sanity(tp: &TabPageInfo, dp: &DiffBlock) -> bool {
    for i in 0..DB_COUNT {
        if let Some(buf) = tp.diffbuf_opt(i)
            && dp.df_lnum[i] + dp.df_count[i] - 1 > buf.b_ml.ml_line_count.get()
        {
            return false;
        }
    }
    true
}
pub fn diff_redraw(dofold: bool) {
    let mut wp_other = crate::globals::WinPtr::NULL;
    let mut used_max_fill_other = false;
    let mut used_max_fill_curwin = false;
    g_st().need_diff_redraw.set(false);
    for wp in for_all_windows_ptr() {
        if !wp.w_onebuf_opt.wo_diff.get() || !buf_valid(wp.buf_opt().as_deref()) {
            continue;
        }
        redraw_later(Some(&wp), UPD_SOME_VALID);
        if wp != curwin_ptr() {
            wp_other = wp.clone();
        }
        if dofold && foldmethod_is_diff(&wp) {
            fold_update_all(&wp);
        }
        let n = diff_check_fill(&wp, wp.w_topline.get());
        if (wp != curwin_ptr() && wp.w_topfill.get() > 0) || n > 0 {
            if wp.w_topfill.get() > n {
                wp.w_topfill.set(n.max(0));
            } else if n > 0 && n > wp.w_topfill.get() {
                wp.w_topfill.set(n);
                if wp == curwin_ptr() {
                    used_max_fill_curwin = true;
                } else if !wp_other.is_null() {
                    used_max_fill_other = true;
                }
            }
            check_topfill(&wp, false);
        }
    }
    if let Some(wp_other) = wp_other.opt()
        && curwin().w_onebuf_opt.wo_scb.get()
    {
        if used_max_fill_curwin {
            diff_set_topline(wp_other, &curwin());
        } else if used_max_fill_other {
            diff_set_topline(&curwin(), wp_other);
        }
    }
}
fn clear_diffin(din: &mut Diffin) {
    if let Some(ref fname) = din.din_fname {
        let _ = os_remove(fname);
    } else {
        din.din_buf = Vec::new();
    }
}
fn clear_diffout(dout: &mut Diffout) {
    if let Some(ref fname) = dout.dout_fname {
        let _ = os_remove(fname);
    } else {
        dout.dout_ga = Vec::new();
    }
}
fn diff_write_buffer(buf: &FileBuf, backing: &mut Vec<u8>, start: Linenr, mut end: Linenr) -> bool {
    if end < 0 {
        end = buf.b_ml.ml_line_count.get();
    }
    if (buf.b_ml.ml_flags.get() & ML_EMPTY) != 0 || end < start {
        *backing = Vec::new();
        return true;
    }
    let mut len: usize = 0;
    for lnum in start..=end {
        len += ml_get_buf_len(buf, lnum) as usize + 1;
    }
    *backing = vec![0u8; len];
    len = 0;
    for lnum in start..=end {
        let line_cstr = ml_get_buf(buf, lnum);
        let mut s = line_cstr.to_bytes();
        if (diff_flags() & DIFF_ICASE) != 0 {
            while !s.is_empty() {
                let c: i32;
                let mut c_len: i32 = 1;
                let mut cbuf: [u8; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];
                if s[0] == NL as u8 {
                    c = NUL as i32;
                } else {
                    let ch = crate::mbyte::utf_slice2char(s);
                    c_len = utf_char2len(ch);
                    c = utf_fold(ch);
                }
                let orig_len = crate::mbyte::utfc_slice2len(s);
                let s_bytes = &s[..orig_len];
                if crate::mbyte::utf_char2slice(c, &mut cbuf) != c_len as usize {
                    backing[len..len + orig_len].copy_from_slice(s_bytes);
                } else {
                    let cl = c_len as usize;
                    backing[len..len + cl].copy_from_slice(&cbuf[..cl]);
                    if orig_len > cl {
                        backing[len + cl..len + orig_len].copy_from_slice(&s_bytes[cl..]);
                    }
                }
                s = &s[orig_len..];
                len += orig_len;
            }
        } else {
            let s_bytes = s;
            let slen = s_bytes.len();
            backing[len..len + slen].copy_from_slice(s_bytes);
            for b in &mut backing[len..len + slen] {
                if *b == b'\n' {
                    *b = 0;
                }
            }
            len += slen;
        }
        backing[len] = b'\n';
        len += 1;
    }
    true
}
fn diff_write(buf: &FileBuf, din: &mut Diffin, start: Linenr, mut end: Linenr) -> bool {
    let Some(ref din_fname) = din.din_fname else {
        return diff_write_buffer(buf, &mut din.din_buf, start, end);
    };
    if frames_locked() {
        return false;
    }
    if end < 0 {
        end = buf.b_ml.ml_line_count.get();
    }
    let save_ml_flags = buf.b_ml.ml_flags.get();
    let save_ff = buf
        .b_p_ff
        .replace(crate::option_vars::OptString::from_cstr(c"unix"));
    let save_cmod_flags = g_st().cmdmod.borrow().cmod_flags != 0;
    g_st().cmdmod.borrow_mut().cmod_flags |= CMOD_LOCKMARKS;
    if end < start {
        end = start;
        buf.b_ml.ml_flags.set(buf.b_ml.ml_flags.get() | (ML_EMPTY));
    }
    let r = buf_write(
        buf,
        Some(din_fname.as_c_str()),
        None,
        start,
        end,
        None,
        false,
        false,
        false,
        true,
    );
    g_st().cmdmod.borrow_mut().cmod_flags = i32::from(save_cmod_flags);
    buf.b_p_ff.set(save_ff);
    buf.b_ml
        .ml_flags
        .set((buf.b_ml.ml_flags.get() & !ML_EMPTY) | (save_ml_flags & ML_EMPTY));
    r
}
fn diff_try_update(dio: &mut Diffio, idx_orig: usize, eap: &mut Exarg) {
    let take_tempname = || vim_tempname();
    'theend: {
        if dio.dio_internal != 0 {
            dio.dio_diff.dout_ga.clear();
        } else {
            dio.dio_orig.din_fname = take_tempname();
            dio.dio_new.din_fname = take_tempname();
            dio.dio_diff.dout_fname = take_tempname();
            if dio.dio_orig.din_fname.is_none()
                || dio.dio_new.din_fname.is_none()
                || dio.dio_diff.dout_fname.is_none()
            {
                break 'theend;
            }
            if !check_external_diff(dio) {
                break 'theend;
            }
        }
        if eap.forceit {
            for idx_new in idx_orig..DB_COUNT {
                if let Some(buf) = curtab().diffbuf_opt(idx_new)
                    && buf_valid(Some(&buf))
                {
                    let _ = buf_check_timestamp(&buf);
                }
            }
        }
        let mut num_anchors: usize = usize::MAX;
        let mut anchors: [[Linenr; MAX_DIFF_ANCHORS]; DB_COUNT] = [[0; MAX_DIFF_ANCHORS]; DB_COUNT];
        if (diff_flags() & DIFF_ANCHOR) != 0 {
            for idx in 0..DB_COUNT {
                let Some(buf) = curtab().diffbuf_opt(idx) else {
                    continue;
                };
                let mut buf_num_anchors: i32 = 0;
                if !parse_diffanchors(
                    false,
                    &buf,
                    Some(&mut anchors[idx]),
                    Some(&mut buf_num_anchors),
                ) {
                    emsg(gettext(E_FAILED_TO_FIND_ALL_DIFF_ANCHORS));
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
            let mut orig_diff: Vec<DiffBlock> = Vec::new();
            if anchor_i != 0 {
                orig_diff = std::mem::take(&mut *curtab().tp_diffs.borrow_mut());
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
            let buf = curtab().diffbuf(idx_orig);
            if !diff_write(&buf, &mut dio.dio_orig, lnum_start, lnum_end) {
                diff_clear(&curtab());
                break 'theend;
            }
            for (idx_new, anchor_row) in
                anchors.iter().enumerate().take(DB_COUNT).skip(idx_orig + 1)
            {
                let Some(buf) = curtab().diffbuf_opt(idx_new) else {
                    continue;
                };
                if buf.b_ml.ml_mfp.borrow().is_none() {
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
                if !diff_write(&buf, &mut dio.dio_new, lnum_start, lnum_end) {
                    continue;
                }
                if !diff_file(dio) {
                    continue;
                }
                diff_read(idx_orig, idx_new, dio);
                clear_diffin(&mut dio.dio_new);
                clear_diffout(&mut dio.dio_diff);
            }
            clear_diffin(&mut dio.dio_orig);
            if anchor_i != 0 {
                diff_st(|st| st.diff_finger_dp_idx.set(None));
                for dp in curtab().tp_diffs.borrow_mut().iter_mut() {
                    for (idx, anchor_row) in anchors.iter().enumerate() {
                        if anchor_row[anchor_i - 1] > 0 {
                            dp.df_lnum[idx] += anchor_row[anchor_i - 1] - 1;
                        }
                    }
                }
                if !orig_diff.is_empty() {
                    orig_diff.append(&mut *curtab().tp_diffs.borrow_mut());
                    *curtab().tp_diffs.borrow_mut() = orig_diff;
                }
            }
        }
    }
}
pub fn diff_internal() -> i32 {
    i32::from(
        (diff_flags() & DIFF_INTERNAL) != 0
            && crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex)
                == K_CALLBACK_NONE_CONST,
    )
}
pub fn ex_diffupdate(eap: &mut Exarg) {
    if diff_st(|st| st.diff_busy.get()) {
        diff_st(|st| st.diff_need_update.set(true));
        return;
    }
    let had_diffs = !curtab().tp_diffs.borrow().is_empty();
    diff_clear(&curtab());
    curtab().tp_diff_invalid.set(0);
    'theend: {
        let mut idx_orig = 0;
        while idx_orig < DB_COUNT {
            if !curtab().tp_diffbuf[idx_orig].get().is_null() {
                break;
            }
            idx_orig += 1;
        }
        if idx_orig == DB_COUNT {
            break 'theend;
        }
        let mut idx_new = idx_orig + 1;
        while idx_new < DB_COUNT {
            if !curtab().tp_diffbuf[idx_new].get().is_null() {
                break;
            }
            idx_new += 1;
        }
        if idx_new == DB_COUNT {
            break 'theend;
        }
        let mut diffio = Diffio {
            dio_orig: Diffin {
                din_fname: None,
                din_buf: Vec::new(),
            },
            dio_new: Diffin {
                din_fname: None,
                din_buf: Vec::new(),
            },
            dio_diff: Diffout {
                dout_fname: None,
                dout_ga: Vec::new(),
            },
            dio_internal: diff_internal(),
        };
        diff_try_update(&mut diffio, idx_orig, eap);
        changed_line_abv_curs();
    }
    if had_diffs || !curtab().tp_diffs.borrow().is_empty() {
        diff_redraw(true);
        window_layout_lock();
        apply_autocmds(
            EVENT_DIFFUPDATED,
            None,
            None,
            false,
            curbuf_opt().as_deref(),
        );
        window_layout_unlock();
    }
}
fn check_external_diff(diffio: &mut Diffio) -> bool {
    let mut io_error = false;
    let mut ok: TriState;
    let orig_fname = diffio.dio_orig.din_fname.clone().unwrap();
    let new_fname = diffio.dio_new.din_fname.clone().unwrap();
    let diff_fname = diffio.dio_diff.dout_fname.clone().unwrap();
    loop {
        ok = K_FALSE;
        let Some(fd1) = CFile::open(&orig_fname, c"w") else {
            io_error = true;
            if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex)
                != K_CALLBACK_NONE_CONST
                || diff_st(|st| st.diff_a_works.get()) != K_NONE
            {
                break;
            }
            diff_st(|st| st.diff_a_works.set(ok));
            continue;
        };
        if !fd1.write_all(b"line1\n") {
            io_error = true;
        }
        drop(fd1);
        if let Some(fd2) = CFile::open(&new_fname, c"w") {
            if !fd2.write_all(b"line2\n") {
                io_error = true;
            }
            drop(fd2);
            let fd3 = diff_file(diffio)
                .then(|| CFile::open(&diff_fname, c"r"))
                .flatten();
            if let Some(fd3) = fd3 {
                let mut linebuf: [u8; LBUFLEN] = [0; LBUFLEN];
                while !vim_fgets(&mut linebuf, &fd3) {
                    if linebuf.starts_with(b"1c1") || linebuf.starts_with(b"@@ -1 +1 @@") {
                        ok = K_TRUE;
                    }
                }
            } else {
                io_error = true;
            }
            let _ = os_remove(&diff_fname);
            let _ = os_remove(&new_fname);
        } else {
            io_error = true;
        }
        let _ = os_remove(&orig_fname);
        if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex) != K_CALLBACK_NONE_CONST {
            break;
        }
        if diff_st(|st| st.diff_a_works.get()) != K_NONE {
            break;
        }
        diff_st(|st| st.diff_a_works.set(ok));
        if ok != 0 {
            break;
        }
    }
    if ok == 0 {
        if io_error {
            emsg(gettext(c"E810: Cannot read or write temp files"));
        }
        emsg(gettext(c"E97: Cannot create diffs"));
        diff_st(|st| st.diff_a_works.set(K_NONE));
        return false;
    }
    true
}
fn diff_file_internal(diffio: &mut Diffio) -> bool {
    let mut param = Xpparam {
        flags: diff_st(|st| st.diff_algorithm.get()) as u64,
    };
    let emit_cfg = Xdemitconf {
        ctxlen: 0,
        interhunkctxlen: 0,
        flags: 0,
    };
    let mut emit_cb = Xdemitcb::Diff(&mut diffio.dio_diff);
    let flags = diff_flags();
    if (flags & DIFF_IWHITE) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE_CHANGE;
    }
    if (flags & DIFF_IWHITEALL) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE;
    }
    if (flags & DIFF_IWHITEEOL) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE_AT_EOL;
    }
    if (flags & DIFF_IBLANK) != 0 {
        param.flags |= XDF_IGNORE_BLANK_LINES;
    }
    if diffio.dio_orig.din_buf.len() > MAX_XDIFF_SIZE as usize
        || diffio.dio_new.din_buf.len() > MAX_XDIFF_SIZE as usize
    {
        emsg(gettext(E_PROBLEM_CREATING_INTERNAL_DIFF));
        return false;
    }
    let mut mf1 = Mmfile {
        slice: &diffio.dio_orig.din_buf,
    };
    let mut mf2 = Mmfile {
        slice: &diffio.dio_new.din_buf,
    };
    if xdl_diff(&mut mf1, &mut mf2, &param, &emit_cfg, &mut emit_cb) < 0 {
        emsg(gettext(E_PROBLEM_CREATING_INTERNAL_DIFF));
        return false;
    }
    true
}
fn diff_file(dio: &mut Diffio) -> bool {
    let tmp_orig = dio.dio_orig.din_fname.as_deref().unwrap_or(c"");
    let tmp_new = dio.dio_new.din_fname.as_deref().unwrap_or(c"");
    let tmp_diff = dio.dio_diff.dout_fname.as_deref().unwrap_or(c"");
    if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex) != K_CALLBACK_NONE_CONST {
        eval_diff(tmp_orig, tmp_new, tmp_diff);
        return true;
    }
    if dio.dio_internal != 0 {
        return diff_file_internal(dio);
    }
    let len = tmp_orig.to_bytes().len()
        + tmp_new.to_bytes().len()
        + tmp_diff.to_bytes().len()
        + opt_vars().p_srr.as_bytes().len()
        + 27;
    let mut cmd_vec: Vec<u8> = vec![0; len];
    if os_env_exists(c"DIFF_OPTIONS", true) {
        let _ = os_unsetenv(c"DIFF_OPTIONS");
    }
    let flags = diff_flags();
    format_to_buf!(
        &mut cmd_vec,
        c"diff %s%s%s%s%s%s%s%s %s",
        if diff_st(|st| st.diff_a_works.get()) == K_FALSE {
            c""
        } else {
            c"-a "
        },
        c"",
        if (flags & DIFF_IWHITE) != 0 {
            c"-b "
        } else {
            c""
        },
        if (flags & DIFF_IWHITEALL) != 0 {
            c"-w "
        } else {
            c""
        },
        if (flags & DIFF_IWHITEEOL) != 0 {
            c"-Z "
        } else {
            c""
        },
        if (flags & DIFF_IBLANK) != 0 {
            c"-B "
        } else {
            c""
        },
        if (flags & DIFF_ICASE) != 0 {
            c"-i "
        } else {
            c""
        },
        tmp_orig,
        tmp_new,
    );
    append_redir(&mut cmd_vec, &opt_vars().p_srr.as_cstr(), tmp_diff);
    crate::autocmd::block_autocmds();
    call_shell(
        Some(crate::memory::bytes_as_cstr(&cmd_vec)),
        K_SHELL_OPT_FILTER | K_SHELL_OPT_SILENT | K_SHELL_OPT_DO_OUT,
        None,
    );
    crate::autocmd::unblock_autocmds();
    true
}
pub fn ex_diffpatch(eap: &mut Exarg) {
    let mut buf_vec: Vec<u8>;
    let old_curwin = curwin_ptr();
    let mut newname_vec: Vec<u8> = Vec::new();
    let esc_name: Option<CString>;
    #[cfg(unix)]
    let fullname: Option<CString>;
    let tmp_orig_owned = vim_tempname();
    let tmp_new_owned = vim_tempname();
    'theend: {
        let (Some(tmp_orig_cstr), Some(tmp_new_cstr)) = (&tmp_orig_owned, &tmp_new_owned) else {
            break 'theend;
        };
        if !buf_write(
            &curbuf(),
            Some(tmp_orig_cstr.as_c_str()),
            None,
            1,
            curbuf().b_ml.ml_line_count.get(),
            None,
            false,
            false,
            false,
            true,
        ) {
            break 'theend;
        }
        #[cfg(unix)]
        {
            fullname = crate::path::FullName_save(eap.arg.as_opt_cstr(), false);
            esc_name = Some(vim_strsave_shellescape(
                fullname
                    .as_ref()
                    .map_or_else(|| eap.arg.as_bytes(), |s| s.as_bytes()),
                true,
                true,
            ));
        }
        #[cfg(not(unix))]
        {
            esc_name = Some(vim_strsave_shellescape(eap.arg.as_bytes(), true, true));
        }
        let esc_cstr = esc_name.as_deref().unwrap_or(c"");
        let esc_len = esc_cstr.to_bytes().len();
        let buflen = tmp_orig_cstr.as_bytes().len() + esc_len + tmp_new_cstr.as_bytes().len() + 16;
        buf_vec = vec![0; buflen];
        #[cfg(unix)]
        let mut dirbuf: [u8; MAXPATHL] = [0; MAXPATHL];
        #[cfg(unix)]
        {
            if !crate::os::fs::os_dirname(&mut dirbuf)
                || crate::os::fs::os_chdir(crate::memory::bytes_as_cstr(&dirbuf)) != 0
            {
                dirbuf[0] = 0;
            } else {
                let tempdir = vim_gettempdir();
                let _ = crate::os::fs::os_chdir(tempdir.as_deref().unwrap_or(c"/tmp"));
                shorten_fnames(true);
            }
        }
        if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_pex) != K_CALLBACK_NONE_CONST {
            #[cfg(unix)]
            eval_patch(
                tmp_orig_cstr,
                fullname.as_deref().unwrap_or_else(|| eap.arg.as_cstr()),
                tmp_new_cstr,
            );
            #[cfg(not(unix))]
            eval_patch(tmp_orig_cstr, eap.arg.as_cstr(), tmp_new_cstr);
        } else {
            format_to_buf!(
                &mut buf_vec,
                c"patch -o %s %s < %s",
                tmp_new_cstr,
                tmp_orig_cstr,
                esc_cstr,
            );
            crate::autocmd::block_autocmds();
            let buf_cstr = crate::memory::bytes_as_cstr(&buf_vec);
            call_shell(Some(buf_cstr), K_SHELL_OPT_FILTER, None);
            crate::autocmd::unblock_autocmds();
        }
        #[cfg(unix)]
        if dirbuf[0] != 0 {
            if crate::os::fs::os_chdir(crate::memory::bytes_as_cstr(&dirbuf)) != 0 {
                emsg(gettext(E_PREV_DIR));
            }
            shorten_fnames(true);
        }
        let tmp_new_bytes = tmp_new_cstr.as_bytes();
        buf_vec[..tmp_new_bytes.len()].copy_from_slice(tmp_new_bytes);
        buf_vec[tmp_new_bytes.len()..tmp_new_bytes.len() + 6].copy_from_slice(b".orig\0");
        let _ = os_remove(crate::memory::bytes_as_cstr(&buf_vec));
        buf_vec[tmp_new_bytes.len()..tmp_new_bytes.len() + 5].copy_from_slice(b".rej\0");
        let _ = os_remove(crate::memory::bytes_as_cstr(&buf_vec));
        let mut file_info = FileInfo::new();
        let info_ok = os_fileinfo(tmp_new_cstr, &mut file_info);
        let filesize = os_fileinfo_size(&file_info);
        if !info_ok || filesize == 0 {
            emsg(gettext(c"E816: Cannot read patch output"));
        } else {
            if curbuf().fname_opt().is_some() {
                let fname_bytes = curbuf().fname_bytes();
                newname_vec = Vec::with_capacity(fname_bytes.len() + 5);
                newname_vec.extend_from_slice(&fname_bytes);
                newname_vec.extend_from_slice(b".new\0");
            }
            g_st().cmdmod.borrow_mut().cmod_tab = 0;
            if win_split(
                0,
                if (diff_flags() & DIFF_VERTICAL) != 0 {
                    WSP_VERT
                } else {
                    0
                },
            ) {
                eap.cmdidx = Cmdidx::CmdSplit;
                eap.arg = crate::strings::OwnedSlice::from_cstr(tmp_new_cstr);
                do_exedit(eap, old_curwin.opt());
                if curwin_ptr() != old_curwin && win_valid(old_curwin.opt()) {
                    diff_win_options(&curwin(), true);
                    diff_win_options(old_curwin.borrow(), true);
                    if !newname_vec.is_empty() {
                        eap.arg = crate::strings::OwnedSlice::from_bytes(&newname_vec);
                        ex_file(eap);
                        if crate::autocmd::augroup_exists(Some(c"filetypedetect")) {
                            do_cmdline_cmd(c":doau filetypedetect BufRead");
                        }
                    }
                }
            }
        }
    }
    if let Some(ref tmp_orig) = tmp_orig_owned {
        let _ = os_remove(tmp_orig);
    }
    if let Some(ref tmp_new) = tmp_new_owned {
        let _ = os_remove(tmp_new);
    }
}
pub fn ex_diffsplit(eap: &mut Exarg) {
    let old_curwin = curwin_ptr();
    let mut old_curbuf = Bufref {
        br_buf: crate::globals::BufPtr::NULL,
        br_fnum: 0,
        br_buf_free_count: 0,
    };
    set_bufref(&mut old_curbuf, g_st().curbuf.get().opt());
    validate_cursor(&curwin());
    set_fraction(&curwin());
    g_st().cmdmod.borrow_mut().cmod_tab = 0;
    if !win_split(
        0,
        if (diff_flags() & DIFF_VERTICAL) != 0 {
            WSP_VERT
        } else {
            0
        },
    ) {
        return;
    }
    eap.cmdidx = Cmdidx::CmdSplit;
    curwin().w_onebuf_opt.wo_diff.set(true);
    do_exedit(eap, old_curwin.opt());
    if curwin_ptr() == old_curwin {
        return;
    }
    diff_win_options(&curwin(), true);
    if win_valid(old_curwin.opt()) {
        let old_curwin = old_curwin.borrow();
        diff_win_options(old_curwin, true);
        if bufref_valid(Some(&old_curbuf))
            && let Some(old_buf) = old_curbuf.buf_opt()
        {
            curwin().w_cursor.set_lnum(diff_get_corresponding_line(
                old_buf,
                old_curwin.w_cursor.get().lnum,
            ));
        }
    }
    scroll_to_fraction(&curwin(), curwin().w_height.get());
}
pub fn ex_diffthis(_eap: &mut Exarg) {
    diff_win_options(&curwin(), true);
}
fn set_diff_option(wp: &WinInfoInfo, value: bool) {
    let old_curwin = curwin_ptr();
    set_curwin(wp);
    g_st().curbuf.set(curwin().w_buffer.get());
    curbuf().b_ro_locked.set(curbuf().b_ro_locked.get() + (1));
    set_option_value_give_err(K_OPT_DIFF, boolean_obj(value), OPT_LOCAL);
    curbuf().b_ro_locked.set(curbuf().b_ro_locked.get() - (1));
    set_curwin(old_curwin);
    g_st().curbuf.set(curwin().w_buffer.get());
}
pub fn diff_win_options(wp: &WinInfoInfo, addbuf: bool) {
    let old_curwin = curwin_ptr();
    set_curwin(wp);
    new_fold_level();
    set_curwin(old_curwin);
    if !wp.w_onebuf_opt.wo_diff.get() {
        wp.w_onebuf_opt
            .wo_scb_save
            .set(wp.w_onebuf_opt.wo_scb.get());
    }
    wp.w_onebuf_opt.wo_scb.set(true);
    if !wp.w_onebuf_opt.wo_diff.get() {
        wp.w_onebuf_opt
            .wo_crb_save
            .set(wp.w_onebuf_opt.wo_crb.get());
    }
    wp.w_onebuf_opt.wo_crb.set(true);
    if (diff_flags() & DIFF_FOLLOWWRAP) == 0 {
        if !wp.w_onebuf_opt.wo_diff.get() {
            wp.w_onebuf_opt
                .wo_wrap_save
                .set(wp.w_onebuf_opt.wo_wrap.get());
        }
        wp.w_onebuf_opt.wo_wrap.set(false);
        wp.w_skipcol.set(0);
    }
    if !wp.w_onebuf_opt.wo_diff.get() {
        if wp.w_onebuf_opt.wo_diff_saved.get() {
            wp.w_onebuf_opt.wo_fdm_save.clear_option_str();
        }
        wp.w_onebuf_opt
            .wo_fdm_save
            .set(wp.w_onebuf_opt.wo_fdm.clone());
    }
    set_option_direct_for(
        K_OPT_FOLDMETHOD,
        static_cstr_as_obj(c"diff"),
        OPT_LOCAL,
        0,
        OptTarget::Win(wp.into()),
    );
    if !wp.w_onebuf_opt.wo_diff.get() {
        wp.w_onebuf_opt
            .wo_fen_save
            .set(wp.w_onebuf_opt.wo_fen.get());
        wp.w_onebuf_opt
            .wo_fdl_save
            .set(wp.w_onebuf_opt.wo_fdl.get());
        if wp.w_onebuf_opt.wo_diff_saved.get() {
            wp.w_onebuf_opt.wo_fdc_save.clear_option_str();
        }
        wp.w_onebuf_opt
            .wo_fdc_save
            .set(wp.w_onebuf_opt.wo_fdc.clone());
    }
    debug_assert!(g_st().diff_foldcolumn.get() >= 0 && g_st().diff_foldcolumn.get() <= 9);
    let fdc_buf: [u8; 1] = [b'0' + (g_st().diff_foldcolumn.get() as u8)];
    wp.w_onebuf_opt.wo_fdc.set_option_bytes(&fdc_buf);
    wp.w_onebuf_opt.wo_fen.set(true);
    wp.w_onebuf_opt.wo_fdl.set(0);
    fold_update_all(wp);
    changed_window_setting(wp);
    if !crate::strings::vim_haschr(&opt_vars().p_sbo.as_cstr(), b'h' as i32) {
        do_cmdline_cmd(c"set sbo+=hor");
    }
    wp.w_onebuf_opt.wo_diff_saved.set(true);
    set_diff_option(wp, true);
    if addbuf {
        diff_buf_add(&wp.buf());
    }
    redraw_later(Some(wp), UPD_NOT_VALID);
}
pub fn ex_diffoff(eap: &mut Exarg) {
    let mut diffwin = false;
    for wp in for_all_windows_ptr() {
        let cond = if eap.forceit {
            wp.w_onebuf_opt.wo_diff.get()
        } else {
            wp == curwin_ptr()
        };
        if cond {
            set_diff_option(&wp, false);
            if wp.w_onebuf_opt.wo_diff_saved.get() {
                if wp.w_onebuf_opt.wo_scb.get() {
                    wp.w_onebuf_opt
                        .wo_scb
                        .set(wp.w_onebuf_opt.wo_scb_save.get());
                }
                if wp.w_onebuf_opt.wo_crb.get() {
                    wp.w_onebuf_opt
                        .wo_crb
                        .set(wp.w_onebuf_opt.wo_crb_save.get());
                }
                if (diff_flags() & DIFF_FOLLOWWRAP) == 0
                    && !wp.w_onebuf_opt.wo_wrap.get()
                    && wp.w_onebuf_opt.wo_wrap_save.get()
                {
                    wp.w_onebuf_opt.wo_wrap.set(true);
                    wp.w_leftcol.set(0);
                }
                let fdm_save = wp.w_onebuf_opt.wo_fdm_save.as_cstr();
                wp.w_onebuf_opt
                    .wo_fdm
                    .set_option_cstr(if !fdm_save.is_empty() {
                        &fdm_save
                    } else {
                        c"manual"
                    });
                let fdc_save = wp.w_onebuf_opt.wo_fdc_save.as_cstr();
                wp.w_onebuf_opt
                    .wo_fdc
                    .set_option_cstr(if !fdc_save.is_empty() {
                        &fdc_save
                    } else {
                        c"0"
                    });
                if wp.w_onebuf_opt.wo_fdl.get() == 0 {
                    wp.w_onebuf_opt
                        .wo_fdl
                        .set(wp.w_onebuf_opt.wo_fdl_save.get());
                }
                if wp.w_onebuf_opt.wo_fen.get() {
                    let is_manual = foldmethod_is_manual(&wp);
                    wp.w_onebuf_opt.wo_fen.set(if is_manual {
                        false
                    } else {
                        wp.w_onebuf_opt.wo_fen_save.get()
                    });
                }
                fold_update_all(&wp);
            }
            wp.w_topfill.set(0);
            changed_window_setting(&wp);
            diff_buf_adjust(&wp);
        }
        diffwin |= wp.w_onebuf_opt.wo_diff.get();
    }
    if eap.forceit {
        diff_buf_clear();
    }
    if !diffwin {
        diff_st(|st| st.diff_need_update.set(false));
        curtab().tp_diff_invalid.set(0);
        curtab().tp_diff_update.set(0);
        diff_clear(&curtab());
    }
    if !diffwin && crate::strings::vim_haschr(&opt_vars().p_sbo.as_cstr(), b'h' as i32) {
        do_cmdline_cmd(c"set sbo-=hor");
    }
}
fn extract_hunk_internal(dout: &mut Diffout, hunk: &mut Diffhunk, line_idx: &mut i32) -> bool {
    let eof = *line_idx >= dout.dout_ga.len() as i32;
    if !eof {
        *hunk = (&dout.dout_ga)[*line_idx as usize];
        *line_idx += 1;
    }
    eof
}
fn extract_hunk(fd: &CFile, hunk: &mut Diffhunk, diffstyle: &mut Diffstyle) -> bool {
    loop {
        let mut line: [u8; LBUFLEN] = [0; LBUFLEN];
        if vim_fgets(&mut line, fd) {
            return true;
        }
        if *diffstyle == Diffstyle::DiffNone {
            if ascii_isdigit(i32::from(line[0])) {
                *diffstyle = Diffstyle::DiffEd;
            } else if line.starts_with(b"@@ ")
                || (line.starts_with(b"--- ")
                    && !vim_fgets(&mut line, fd)
                    && line.starts_with(b"+++ ")
                    && !vim_fgets(&mut line, fd)
                    && line.starts_with(b"@@ "))
            {
                *diffstyle = Diffstyle::DiffUnified;
            } else {
                continue;
            }
        }
        if *diffstyle == Diffstyle::DiffEd {
            if !ascii_isdigit(i32::from(line[0])) {
                continue;
            }
            if !parse_diff_ed(crate::memory::bytes_until_nul(&line), hunk) {
                continue;
            }
        } else {
            debug_assert!(*diffstyle == Diffstyle::DiffUnified);
            if !line.starts_with(b"@@ ") {
                continue;
            }
            if !parse_diff_unified(crate::memory::bytes_until_nul(&line), hunk) {
                continue;
            }
        }
        return false;
    }
}
fn process_hunk(
    dp_idx_p: &mut usize,
    idx_orig: usize,
    idx_new: usize,
    hunk: &Diffhunk,
    notsetp: &mut bool,
) {
    let mut dp_idx = *dp_idx_p;
    while dp_idx < curtab().tp_diffs.borrow().len()
        && hunk.lnum_orig
            > curtab().tp_diffs.borrow()[dp_idx].df_lnum[idx_orig]
                + curtab().tp_diffs.borrow()[dp_idx].df_count[idx_orig]
    {
        if *notsetp {
            diff_copy_entry(dp_idx, idx_orig, idx_new);
        }
        dp_idx += 1;
        *notsetp = true;
    }
    if dp_idx < curtab().tp_diffs.borrow().len()
        && hunk.lnum_orig
            <= curtab().tp_diffs.borrow()[dp_idx].df_lnum[idx_orig]
                + curtab().tp_diffs.borrow()[dp_idx].df_count[idx_orig]
        && hunk.lnum_orig + hunk.count_orig >= curtab().tp_diffs.borrow()[dp_idx].df_lnum[idx_orig]
    {
        let mut dpl_idx = dp_idx;
        while dpl_idx + 1 < curtab().tp_diffs.borrow().len() {
            if hunk.lnum_orig + hunk.count_orig
                < curtab().tp_diffs.borrow()[dpl_idx + 1].df_lnum[idx_orig]
            {
                break;
            }
            dpl_idx += 1;
        }
        let dpl_lnum = curtab().tp_diffs.borrow()[dpl_idx].df_lnum;
        let dpl_count = curtab().tp_diffs.borrow()[dpl_idx].df_count;
        {
            let ct = curtab();
            let mut diffs = ct.tp_diffs.borrow_mut();
            let dp = &mut diffs[dp_idx];
            let mut off = dp.df_lnum[idx_orig] - hunk.lnum_orig;
            if off > 0 {
                for i in idx_orig..idx_new {
                    if !ct.tp_diffbuf[i].get().is_null() {
                        dp.df_lnum[i] -= off;
                        dp.df_count[i] += off;
                    }
                }
                dp.df_lnum[idx_new] = hunk.lnum_new;
                dp.df_count[idx_new] = hunk.count_new;
            } else if *notsetp {
                dp.df_lnum[idx_new] = hunk.lnum_new + off;
                dp.df_count[idx_new] = hunk.count_new - off;
            } else {
                let orig_size_in_dp = hunk
                    .count_orig
                    .min(dp.df_lnum[idx_orig] + dp.df_count[idx_orig] - hunk.lnum_orig);
                let size_diff = hunk.count_new - orig_size_in_dp;
                dp.df_count[idx_new] += size_diff;
                off = hunk.lnum_new + hunk.count_new - (dp.df_lnum[idx_new] + dp.df_count[idx_new]);
                if off > 0 {
                    dp.df_count[idx_new] += off;
                }
            }
            off = (hunk.lnum_orig + hunk.count_orig) - (dpl_lnum[idx_orig] + dpl_count[idx_orig]);
            if off < 0 {
                if *notsetp || dp_idx != dpl_idx {
                    dp.df_count[idx_new] += -off;
                }
                off = 0;
            }
            for i in idx_orig..idx_new {
                if !ct.tp_diffbuf[i].get().is_null() {
                    dp.df_count[i] = dpl_lnum[i] + dpl_count[i] - dp.df_lnum[i] + off;
                }
            }
            if dpl_idx > dp_idx {
                diff_st(|st| st.diff_finger_dp_idx.set(None));
                diffs.drain((dp_idx + 1)..=dpl_idx);
            }
        }
    } else {
        dp_idx = diff_alloc_new(&curtab(), dp_idx);
        {
            let ct = curtab();
            let mut diffs = ct.tp_diffs.borrow_mut();
            diffs[dp_idx].df_lnum[idx_orig] = hunk.lnum_orig;
            diffs[dp_idx].df_count[idx_orig] = hunk.count_orig;
            diffs[dp_idx].df_lnum[idx_new] = hunk.lnum_new;
            diffs[dp_idx].df_count[idx_new] = hunk.count_new;
        }
        for i in (idx_orig + 1)..idx_new {
            if !curtab().tp_diffbuf[i].get().is_null() {
                diff_copy_entry(dp_idx, idx_orig, i);
            }
        }
    }
    *notsetp = false;
    *dp_idx_p = dp_idx;
}
fn diff_read(idx_orig: usize, idx_new: usize, dio: &mut Diffio) {
    let mut fd: Option<CFile> = None;
    let mut line_hunk_idx: i32 = 0;
    let mut dp_idx: usize = 0;
    let dout = &mut dio.dio_diff;
    let mut notset = true;
    let mut diffstyle = Diffstyle::DiffNone;
    if dio.dio_internal == 0 {
        let Some(f) = CFile::open(dout.dout_fname.as_ref().unwrap(), c"r") else {
            emsg(gettext(c"E98: Cannot read diff output"));
            return;
        };
        fd = Some(f);
    }
    loop {
        let mut hunk = Diffhunk {
            lnum_orig: 0,
            count_orig: 0,
            lnum_new: 0,
            count_new: 0,
        };
        let eof = if dio.dio_internal != 0 {
            extract_hunk_internal(dout, &mut hunk, &mut line_hunk_idx)
        } else {
            extract_hunk(fd.as_ref().unwrap(), &mut hunk, &mut diffstyle)
        };
        if eof {
            break;
        }
        process_hunk(&mut dp_idx, idx_orig, idx_new, &hunk, &mut notset);
    }
    while dp_idx < curtab().tp_diffs.borrow().len() {
        if notset {
            diff_copy_entry(dp_idx, idx_orig, idx_new);
        }
        dp_idx += 1;
        notset = true;
    }
}
fn diff_copy_entry(dp_idx: usize, idx_orig: usize, idx_new: usize) {
    let off = if dp_idx == 0 {
        0
    } else {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
        let dprev = &diffs[dp_idx - 1];
        (dprev.df_lnum[idx_orig] + dprev.df_count[idx_orig])
            - (dprev.df_lnum[idx_new] + dprev.df_count[idx_new])
    };
    let ct = curtab();
    let mut diffs = ct.tp_diffs.borrow_mut();
    let dp = &mut diffs[dp_idx];
    dp.df_lnum[idx_new] = dp.df_lnum[idx_orig] - off;
    dp.df_count[idx_new] = dp.df_count[idx_orig];
}
pub fn diff_clear(tp: &TabPageInfo) {
    diff_st(|st| st.diff_finger_dp_idx.set(None));
    tp.tp_diffs.borrow_mut().clear();
}
pub fn diff_linematch(dp: &DiffBlock) -> bool {
    if (diff_flags() & DIFF_LINEMATCH) == 0 {
        return false;
    }
    let mut tsize: i32 = 0;
    for i in 0..DB_COUNT {
        if !curtab().tp_diffbuf[i].get().is_null() {
            if dp.df_count[i] < 0 {
                return false;
            }
            tsize += dp.df_count[i];
        }
    }
    tsize <= diff_st(|st| st.linematch_lines.get())
}
fn get_max_diff_length(dp: &DiffBlock) -> i32 {
    let mut maxlength: i32 = 0;
    for k in 0..DB_COUNT {
        if !curtab().tp_diffbuf[k].get().is_null() && dp.df_count[k] > maxlength {
            maxlength = dp.df_count[k];
        }
    }
    maxlength
}
fn find_top_diff_block(
    thistopdiff: &mut Option<usize>,
    next_adjacent_blocks: &mut Option<usize>,
    fromidx: usize,
    topline: i32,
) {
    let mut localtopdiff: Option<usize> = None;
    let mut topdiffchange = false;
    let ct = curtab();
    let diffs = ct.tp_diffs.borrow();
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
fn calculate_topfill_and_topline(
    fromidx: usize,
    toidx: usize,
    from_topline: i32,
    from_topfill: i32,
    topfill: &mut i32,
    topline: &mut Linenr,
) {
    let mut thistopdiff: Option<usize> = None;
    let mut next_adjacent_blocks: Option<usize> = None;
    let mut virtual_lines_passed: i32 = 0;
    find_top_diff_block(
        &mut thistopdiff,
        &mut next_adjacent_blocks,
        fromidx,
        from_topline,
    );
    let ct = curtab();
    let diffs = ct.tp_diffs.borrow();
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
    let mut max_virt_lines: i32 = 0;
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
    if (diff_flags() & DIFF_FILLER) != 0 {
        *topfill = max_virt_lines - virtual_lines_passed;
    }
    *topline = curlinenum_to;
}
fn apply_linematch_results(dp_idx: usize, decisions: &[i32]) {
    let mut line_numbers: [Linenr; DB_COUNT] = [0; DB_COUNT];
    let mut outputmap: [usize; DB_COUNT] = [0; DB_COUNT];
    let mut ndiffs: usize = 0;
    for (i, line_num) in line_numbers.iter_mut().enumerate().take(DB_COUNT) {
        if !curtab().tp_diffbuf[i].get().is_null() {
            let ct = curtab();
            let mut diffs = ct.tp_diffs.borrow_mut();
            *line_num = diffs[dp_idx].df_lnum[i];
            diffs[dp_idx].df_count[i] = 0;
            outputmap[ndiffs] = i;
            ndiffs += 1;
        }
    }
    let mut dp_s_idx = dp_idx;
    for (i, &dec) in decisions.iter().enumerate() {
        if i != 0 && decisions[i - 1] != dec {
            dp_s_idx = diff_alloc_new(&curtab(), dp_s_idx + 1);
            let ct = curtab();
            let mut diffs = ct.tp_diffs.borrow_mut();
            diffs[dp_s_idx].is_linematched = true;
            for (j, &line_num) in line_numbers.iter().enumerate().take(DB_COUNT) {
                if !ct.tp_diffbuf[j].get().is_null() {
                    diffs[dp_s_idx].df_lnum[j] = line_num;
                    diffs[dp_s_idx].df_count[j] = 0;
                }
            }
        }
        for j in 0..ndiffs {
            if (dec & (1 << j)) != 0 {
                curtab().tp_diffs.borrow_mut()[dp_s_idx].df_count[outputmap[j]] += 1;
                line_numbers[outputmap[j]] += 1;
            }
        }
    }
    curtab().tp_diffs.borrow_mut()[dp_idx].is_linematched = true;
}
fn run_linematch_algorithm(dp_idx: usize) {
    let mut diffbufs_vec: [Vec<u8>; DB_COUNT] = [const { Vec::new() }; DB_COUNT];
    let mut diff_length: [i32; DB_COUNT] = [0; DB_COUNT];
    let mut ndiffs: usize = 0;
    let dp_lnum = curtab().tp_diffs.borrow()[dp_idx].df_lnum;
    let dp_count = curtab().tp_diffs.borrow()[dp_idx].df_count;
    for i in 0..DB_COUNT {
        if let Some(buf) = curtab().diffbuf_opt(i) {
            if dp_count[i] > 0 {
                diff_write_buffer(
                    &buf,
                    &mut diffbufs_vec[ndiffs],
                    dp_lnum[i],
                    dp_lnum[i] + dp_count[i] - 1,
                );
            }
            diff_length[ndiffs] = dp_count[i];
            ndiffs += 1;
        }
    }
    let diffbufs_mm: [Mmfile<'_>; DB_COUNT] = std::array::from_fn(|i| Mmfile {
        slice: &diffbufs_vec[i],
    });
    let diffbuf_refs: Vec<&Mmfile<'_>> = diffbufs_mm[..ndiffs].iter().collect();
    let iwhite = (diff_flags() & (DIFF_IWHITEALL | DIFF_IWHITE)) != 0;
    let decisions = linematch_nbuffers(&diffbuf_refs, &diff_length[..ndiffs], iwhite);
    drop(diffbufs_vec);
    apply_linematch_results(dp_idx, &decisions);
}
pub fn diff_check_with_linestatus(
    wp: &WinInfoInfo,
    lnum: Linenr,
    mut linestatus: Option<&mut i32>,
) -> i32 {
    if let Some(linestatus) = linestatus.as_deref_mut() {
        *linestatus = 0;
    }
    if curtab().tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab().tp_diffs.borrow().is_empty() || !wp.w_onebuf_opt.wo_diff.get() {
        return 0;
    }
    if lnum < 1 || lnum > wp.buf().b_ml.ml_line_count.get() + 1 {
        return 0;
    }
    let idx = diff_buf_idx(&wp.buf(), &curtab());
    if idx == DB_COUNT as i32 {
        return 0;
    }
    let u_idx = idx as usize;
    if has_folding(wp, lnum, None, None) || decor_conceal_line(wp, lnum - 1, false) {
        return 0;
    }
    let mut le: Option<usize> = None;
    let mut dp_idx = diff_scan_start(&curtab(), idx, lnum);
    while dp_idx < curtab().tp_diffs.borrow().len() {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
        if diffs[dp_idx].df_lnum[u_idx] <= lnum {
            le = Some(dp_idx);
        }
        if lnum <= diffs[dp_idx].df_lnum[u_idx] + diffs[dp_idx].df_count[u_idx] {
            break;
        }
        dp_idx += 1;
    }
    diff_scan_remember(&curtab(), idx, lnum, le);
    if dp_idx >= curtab().tp_diffs.borrow().len()
        || lnum < curtab().tp_diffs.borrow()[dp_idx].df_lnum[u_idx]
    {
        return 0;
    }
    let should_linematch = lnum >= wp.w_topline.get() && lnum < wp.w_botline.get() && {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
        !diffs[dp_idx].is_linematched
            && diff_linematch(&diffs[dp_idx])
            && diff_check_sanity(&ct, &diffs[dp_idx])
    };
    if should_linematch {
        run_linematch_algorithm(dp_idx);
    }
    let mut num_fill: i32 = 0;
    while {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
        lnum == diffs[dp_idx].df_lnum[u_idx] + diffs[dp_idx].df_count[u_idx]
    } {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
        if (diff_flags() & DIFF_FILLER) != 0 {
            let maxcount = get_max_diff_length(&diffs[dp_idx]);
            num_fill += maxcount - diffs[dp_idx].df_count[u_idx];
        }
        if dp_idx + 1 < diffs.len()
            && lnum >= diffs[dp_idx + 1].df_lnum[u_idx]
            && lnum <= diffs[dp_idx + 1].df_lnum[u_idx] + diffs[dp_idx + 1].df_count[u_idx]
        {
            dp_idx += 1;
        } else {
            break;
        }
    }
    let ct = curtab();
    let diffs = ct.tp_diffs.borrow();
    let dp = &diffs[dp_idx];
    if lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx] {
        let mut zero = false;
        let mut cmp = false;
        for i in 0..DB_COUNT {
            if i != u_idx && !ct.tp_diffbuf[i].get().is_null() {
                if dp.df_count[i] == 0 {
                    zero = true;
                } else {
                    if dp.df_count[i] != dp.df_count[u_idx] {
                        if let Some(linestatus) = linestatus.as_deref_mut() {
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
                    && !ct.tp_diffbuf[i].get().is_null()
                    && dp.df_count[i] != 0
                    && !diff_equal_entry(dp, u_idx, i)
                {
                    if let Some(linestatus) = linestatus.as_deref_mut() {
                        *linestatus = -1;
                    }
                    return num_fill;
                }
            }
        }
        if !zero {
            return num_fill;
        }
        if let Some(linestatus) = linestatus {
            *linestatus = -2;
        }
        return num_fill;
    }
    num_fill
}
pub fn diff_check_fill(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    if (diff_flags() & DIFF_FILLER) == 0 {
        return 0;
    }
    let n = diff_check_with_linestatus(wp, lnum, None);
    n.max(0)
}
fn diff_equal_entry(dp: &DiffBlock, idx1: usize, idx2: usize) -> bool {
    if dp.df_count[idx1] != dp.df_count[idx2] {
        return false;
    }
    if !diff_check_sanity(&curtab(), dp) {
        return false;
    }
    for i in 0..dp.df_count[idx1] {
        let line = crate::memory::bytes_to_cstring(&crate::memline::ml_get_buf_slice(
            &curtab().diffbuf(idx1),
            dp.df_lnum[idx1] + i,
        ));
        let cmp = diff_cmp(
            line.to_bytes(),
            &crate::memline::ml_get_buf_slice(&curtab().diffbuf(idx2), dp.df_lnum[idx2] + i),
        );
        if cmp != 0 {
            return false;
        }
    }
    true
}
fn diff_equal_char(p1: &[u8], p2: &[u8], len: &mut i32) -> bool {
    let l = crate::mbyte::utfc_slice2len(p1) as i32;
    if l != crate::mbyte::utfc_slice2len(p2) as i32 {
        return false;
    }
    let flags = diff_flags();
    if l > 1 {
        let lu = l as usize;
        if p1.get(..lu) != p2.get(..lu)
            && ((flags & DIFF_ICASE) == 0
                || utf_fold(crate::mbyte::utf_slice2char(p1))
                    != utf_fold(crate::mbyte::utf_slice2char(p2)))
        {
            return false;
        }
        *len = l;
    } else {
        let b1 = p1.first().copied().unwrap_or(0);
        let b2 = p2.first().copied().unwrap_or(0);
        if b1 != b2 && ((flags & DIFF_ICASE) == 0 || !b1.eq_ignore_ascii_case(&b2)) {
            return false;
        }
        *len = 1;
    }
    true
}
fn diff_cmp(s1: &[u8], s2: &[u8]) -> i32 {
    let flags = diff_flags();
    if (flags & DIFF_IBLANK) != 0
        && (crate::charset::skipwhite_slice(s1).is_empty()
            || crate::charset::skipwhite_slice(s2).is_empty())
    {
        return 0;
    }
    if (flags & (DIFF_ICASE | ALL_WHITE_DIFF)) == 0 {
        return s1.cmp(s2) as i32;
    }
    if (flags & DIFF_ICASE) != 0 && (flags & ALL_WHITE_DIFF) == 0 {
        let s1_cstr = CString::new(crate::memory::bytes_until_nul(s1)).unwrap_or_default();
        let s2_cstr = CString::new(crate::memory::bytes_until_nul(s2)).unwrap_or_default();
        return mb_stricmp(s1_cstr.as_c_str(), s2_cstr.as_c_str());
    }
    let mut p1 = s1;
    let mut p2 = s2;
    while !p1.is_empty() && !p2.is_empty() {
        if ((flags & DIFF_IWHITE) != 0
            && ascii_iswhite(p1[0] as i32)
            && ascii_iswhite(p2[0] as i32))
            || ((flags & DIFF_IWHITEALL) != 0
                && (ascii_iswhite(p1[0] as i32) || ascii_iswhite(p2[0] as i32)))
        {
            p1 = crate::charset::skipwhite_slice(p1);
            p2 = crate::charset::skipwhite_slice(p2);
        } else {
            let mut l: i32 = 0;
            if !diff_equal_char(p1, p2, &mut l) {
                break;
            }
            p1 = &p1[(l as usize).min(p1.len())..];
            p2 = &p2[(l as usize).min(p2.len())..];
        }
    }
    p1 = crate::charset::skipwhite_slice(p1);
    p2 = crate::charset::skipwhite_slice(p2);
    i32::from(!p1.is_empty() || !p2.is_empty())
}
pub fn diff_set_topline(fromwin: &WinInfoInfo, towin: &WinInfoInfo) {
    let fromidx = diff_buf_idx(&fromwin.buf(), &curtab());
    if fromidx == DB_COUNT as i32 {
        return;
    }
    let u_fromidx = fromidx as usize;
    if curtab().tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    let lnum = fromwin.w_topline.get();
    towin.w_topfill.set(0);
    let dp_lnum_opt = curtab()
        .tp_diffs
        .borrow()
        .iter()
        .find(|dp| lnum <= dp.df_lnum[u_fromidx] + dp.df_count[u_fromidx])
        .map(|dp| dp.df_lnum);
    if let Some(dp_lnum) = dp_lnum_opt {
        let toidx = diff_buf_idx(&towin.buf(), &curtab());
        if toidx == DB_COUNT as i32 {
            return;
        }
        let u_toidx = toidx as usize;
        towin
            .w_topline
            .set(lnum + (dp_lnum[u_toidx] - dp_lnum[u_fromidx]));
        if lnum >= dp_lnum[u_fromidx] {
            let mut tf = towin.w_topfill.get();
            let mut tl = towin.w_topline.get();
            calculate_topfill_and_topline(
                u_fromidx,
                u_toidx,
                fromwin.w_topline.get(),
                fromwin.w_topfill.get(),
                &mut tf,
                &mut tl,
            );
            towin.w_topfill.set(tf);
            towin.w_topline.set(tl);
        }
    } else {
        towin.w_topline.set(
            towin.buf().b_ml.ml_line_count.get() - (fromwin.buf().b_ml.ml_line_count.get() - lnum),
        );
    }
    towin.w_botfill.set(false);
    if towin.w_topline.get() > towin.buf().b_ml.ml_line_count.get() {
        towin.w_topline.set(towin.buf().b_ml.ml_line_count.get());
        towin.w_botfill.set(true);
    }
    if towin.w_topline.get() < 1 {
        towin.w_topline.set(1);
        towin.w_topfill.set(0);
    }
    invalidate_botline_win(towin);
    changed_line_abv_curs_win(towin);
    check_topfill(towin, false);
    let mut topline = towin.w_topline.get();
    has_folding(towin, topline, Some(&mut topline), None);
    towin.w_topline.set(topline);
    set_topline_valid(towin);
}
fn parse_diffanchors(
    check_only: bool,
    buf: &FileBuf,
    mut anchors: Option<&mut [Linenr; MAX_DIFF_ANCHORS]>,
    num_anchors: Option<&mut i32>,
) -> bool {
    let dia_owned = if buf.b_p_dia.is_empty() {
        opt_vars().p_dia.to_owned()
    } else {
        buf.b_p_dia.to_owned()
    };
    let mut dia = dia_owned.as_c_str();
    let orig_curbuf = g_st().curbuf.get();
    let orig_curwin = curwin_ptr();
    let bufwin = if check_only {
        curwin_ptr()
    } else {
        let mut found_bw = crate::globals::WinPtr::NULL;
        for bw in crate::globals::win_iter_ptr(firstwin()) {
            if bw.w_buffer.get() == buf && bw.w_onebuf_opt.wo_diff.get() {
                found_bw = bw;
                break;
            }
        }
        if found_bw.is_null() && !dia.is_empty() {
            emsg(gettext(E_DIFF_ANCHORS_WITH_HIDDEN_WINDOWS));
            return false;
        }
        found_bw
    };
    let mut i: usize = 0;
    while i < MAX_DIFF_ANCHORS && !dia.is_empty() {
        if dia.to_bytes().first() == Some(&b',') {
            return false;
        }
        g_st().curbuf.set((buf).into());
        set_curwin(&bufwin);
        let mut errormsg = None;
        let mut dia_opt = Some(dia);
        let lnum = get_address(
            None,
            &mut dia_opt,
            ADDR_LINES,
            check_only,
            true,
            0,
            1,
            &mut errormsg,
        );
        g_st().curbuf.set(orig_curbuf.clone());
        set_curwin(&orig_curwin);
        if let Some(msg) = errormsg {
            emsg(&msg);
        }
        let Some(next_dia) = dia_opt else {
            return false;
        };
        dia = next_dia;
        if dia.to_bytes().first() != Some(&b',') && !dia.is_empty() {
            return false;
        }
        if !check_only && (lnum == MAXLNUM || lnum <= 0 || lnum > buf.b_ml.ml_line_count.get() + 1)
        {
            emsg(gettext(E_INVRANGE));
            return false;
        }
        if let Some(ref mut a) = anchors {
            a[i] = lnum;
        }
        if dia.to_bytes().first() == Some(&b',') {
            dia = &dia[1..];
        }
        i += 1;
    }
    if i == MAX_DIFF_ANCHORS && !dia.is_empty() {
        semsg!(
            gettext(E_CANNOT_HAVE_MORE_THAN_NR_DIFF_ANCHORS),
            MAX_DIFF_ANCHORS as i32,
        );
        return false;
    }
    if let Some(num_anchors) = num_anchors {
        *num_anchors = i as i32;
    }
    true
}
pub fn diffanchors_changed(buflocal: bool) -> bool {
    let result = parse_diffanchors(true, &curbuf(), None, None);
    if result && (diff_flags() & DIFF_ANCHOR) != 0 {
        for tp in for_all_tabpages_ptr() {
            if !buflocal {
                tp.tp_diff_invalid.set(1);
            } else {
                for idx in 0..DB_COUNT {
                    if tp.tp_diffbuf[idx].get() == g_st().curbuf.get() {
                        tp.tp_diff_invalid.set(1);
                        break;
                    }
                }
            }
        }
    }
    result
}
pub fn diffopt_changed() -> bool {
    let v: &OptKeyDict_dip = &opt_keyset(&opt_vars().p_dip.as_bytes(), K_OPT_DIFFOPT);
    let mut flags = (if v.filler { DIFF_FILLER } else { 0 })
        | (if v.anchor { DIFF_ANCHOR } else { 0 })
        | (if v.iblank { DIFF_IBLANK } else { 0 })
        | (if v.icase { DIFF_ICASE } else { 0 })
        | (if v.iwhiteall { DIFF_IWHITEALL } else { 0 })
        | (if v.iwhiteeol { DIFF_IWHITEEOL } else { 0 })
        | (if v.iwhite { DIFF_IWHITE } else { 0 })
        | (if v.horizontal { DIFF_HORIZONTAL } else { 0 })
        | (if v.vertical { DIFF_VERTICAL } else { 0 })
        | (if v.closeoff { DIFF_CLOSE_OFF } else { 0 })
        | (if v.hiddenoff { DIFF_HIDDEN_OFF } else { 0 })
        | (if v.followwrap { DIFF_FOLLOWWRAP } else { 0 })
        | (if v.internal { DIFF_INTERNAL } else { 0 });
    if has_dip_key(v, KEYSET_OPTIDX_DIP_LINEMATCH) {
        flags |= DIFF_LINEMATCH | DIFF_FILLER;
    }
    if has_dip_key(v, KEYSET_OPTIDX_DIP_INLINE) {
        flags &= !ALL_INLINE;
        match crate::memory::bytes_until_nul(&v.inline_) {
            b"simple" => flags |= DIFF_INLINE_SIMPLE,
            b"char" => flags |= DIFF_INLINE_CHAR,
            b"word" => flags |= DIFF_INLINE_WORD,
            _ => flags |= DIFF_INLINE_NONE,
        }
    }
    let mut algorithm = if v.indent_heuristic {
        XDF_INDENT_HEURISTIC
    } else {
        0
    };
    if has_dip_key(v, KEYSET_OPTIDX_DIP_ALGORITHM) {
        match crate::memory::bytes_until_nul(&v.algorithm) {
            b"minimal" => algorithm |= XDF_NEED_MINIMAL,
            b"patience" => algorithm |= XDF_PATIENCE_DIFF,
            b"histogram" => algorithm |= XDF_HISTOGRAM_DIFF,
            _ => {}
        }
    }
    let mut ret = true;
    if (flags & DIFF_HORIZONTAL) != 0 && (flags & DIFF_VERTICAL) != 0 {
        ret = false;
    } else {
        if diff_flags() != flags || diff_st(|st| st.diff_algorithm.get()) != algorithm {
            for tp in for_all_tabpages_ptr() {
                tp.tp_diff_invalid.set(1);
            }
        }
        let context = if has_dip_key(v, KEYSET_OPTIDX_DIP_CONTEXT) {
            v.context as i32
        } else {
            6
        };
        let lm_lines = if has_dip_key(v, KEYSET_OPTIDX_DIP_LINEMATCH) {
            v.linematch as i32
        } else {
            0
        };
        diff_st(|st| {
            st.diff_flags.set(flags);
            st.linematch_lines.set(lm_lines);
            st.diff_algorithm.set(algorithm);
        });
        g_st()
            .diff_context
            .set(if context == 0 { 1 } else { context });
        g_st()
            .diff_foldcolumn
            .set(if has_dip_key(v, KEYSET_OPTIDX_DIP_FOLDCOLUMN) {
                v.foldcolumn as i32
            } else {
                2
            });
        diff_redraw(true);
        check_scrollbind(0, 0);
    }
    ret
}
pub fn diffopt_horizontal() -> bool {
    (diff_flags() & DIFF_HORIZONTAL) != 0
}
pub fn diffopt_hiddenoff() -> bool {
    (diff_flags() & DIFF_HIDDEN_OFF) != 0
}
pub fn diffopt_closeoff() -> bool {
    (diff_flags() & DIFF_CLOSE_OFF) != 0
}
pub fn diffopt_filler() -> bool {
    (diff_flags() & DIFF_FILLER) != 0
}
pub fn diff_update_line(lnum: Linenr) {
    if (diff_flags() & ALL_INLINE_DIFF) == 0 {
        return;
    }
    let idx = diff_buf_idx(&curbuf(), &curtab());
    if idx == DB_COUNT as i32 {
        return;
    }
    let u_idx = idx as usize;
    if let Some(dp) = curtab()
        .tp_diffs
        .borrow_mut()
        .iter_mut()
        .find(|dp| lnum <= dp.df_lnum[u_idx] + dp.df_count[u_idx])
    {
        dp.has_changes = false;
        dp.df_changes.clear();
    }
}
pub fn diff_change_parse(
    diffline: &Diffline,
    change: &DifflineChange,
    change_start: &mut i32,
    change_end: &mut i32,
) -> bool {
    let bufidx = diffline.bufidx as usize;
    if change.dc_start_lnum_off[bufidx] < diffline.lineoff {
        *change_start = 0;
    } else {
        *change_start = change.dc_start[bufidx];
    }
    if change.dc_end_lnum_off[bufidx] > diffline.lineoff {
        *change_end = i32::MAX;
    } else {
        *change_end = change.dc_end[bufidx];
    }
    if diffline.is_simple {
        return false;
    }
    for i in 0..DB_COUNT {
        if i == bufidx {
            continue;
        }
        if change.dc_start[i] != change.dc_end[i]
            || change.dc_end_lnum_off[i] != change.dc_start_lnum_off[i]
        {
            return false;
        }
    }
    true
}
fn diff_find_change_simple(
    wp: &WinInfoInfo,
    lnum: Linenr,
    dp: &DiffBlock,
    idx: usize,
    startp: &mut i32,
    endp: &mut i32,
) -> bool {
    let flags = diff_flags();
    let line_org_owned = if (flags & DIFF_INLINE_NONE) != 0 {
        None
    } else {
        Some(crate::memory::bytes_to_cstring(
            &crate::memline::ml_get_buf_slice(&wp.buf(), lnum),
        ))
    };
    let mut added = true;
    let off = lnum - dp.df_lnum[idx];
    for i in 0..DB_COUNT {
        if !curtab().tp_diffbuf[i].get().is_null() && i != idx {
            if off >= dp.df_count[i] {
                continue;
            }
            added = false;
            if (flags & DIFF_INLINE_NONE) != 0 {
                break;
            }
            let line_new_cstr = ml_get_buf(&curtab().diffbuf(i), dp.df_lnum[i] + off);
            let org_cstr = line_org_owned.as_deref().unwrap_or(c"");
            let org_slice = org_cstr.to_bytes();
            let new_slice = line_new_cstr.to_bytes();
            let org_bytes = org_cstr.to_bytes_with_nul();
            let new_bytes = line_new_cstr.to_bytes_with_nul();
            let mut si_org: i32 = 0;
            let mut si_new: i32 = 0;
            while org_bytes.get(si_org as usize).copied().unwrap_or(0) != 0 {
                let c_org = org_bytes.get(si_org as usize).copied().unwrap_or(0) as i32;
                let c_new = new_bytes.get(si_new as usize).copied().unwrap_or(0) as i32;
                if ((flags & DIFF_IWHITE) != 0 && ascii_iswhite(c_org) && ascii_iswhite(c_new))
                    || ((flags & DIFF_IWHITEALL) != 0
                        && (ascii_iswhite(c_org) || ascii_iswhite(c_new)))
                {
                    si_org = (org_slice.len()
                        - crate::charset::skipwhite_slice(
                            org_slice.get(si_org as usize..).unwrap_or(&[]),
                        )
                        .len()) as i32;
                    si_new = (new_slice.len()
                        - crate::charset::skipwhite_slice(
                            new_slice.get(si_new as usize..).unwrap_or(&[]),
                        )
                        .len()) as i32;
                } else {
                    let mut l: i32 = 0;
                    if !diff_equal_char(
                        org_slice.get(si_org as usize..).unwrap_or(&[]),
                        new_slice.get(si_new as usize..).unwrap_or(&[]),
                        &mut l,
                    ) {
                        break;
                    }
                    si_org += l;
                    si_new += l;
                }
            }
            si_org -= crate::mbyte::utf_head_off_slice(org_bytes, si_org as usize);
            si_new -= crate::mbyte::utf_head_off_slice(new_bytes, si_new as usize);
            *startp = (*startp).min(si_org);
            if org_bytes.get(si_org as usize).copied().unwrap_or(0) != 0
                || new_bytes.get(si_new as usize).copied().unwrap_or(0) != 0
            {
                let mut ei_org = (org_bytes.len() - 1) as i32;
                let mut ei_new = (new_bytes.len() - 1) as i32;
                while ei_org >= *startp && ei_new >= si_new && ei_org >= 0 && ei_new >= 0 {
                    let c_org = org_bytes.get(ei_org as usize).copied().unwrap_or(0) as i32;
                    let c_new = new_bytes.get(ei_new as usize).copied().unwrap_or(0) as i32;
                    if ((flags & DIFF_IWHITE) != 0 && ascii_iswhite(c_org) && ascii_iswhite(c_new))
                        || ((flags & DIFF_IWHITEALL) != 0
                            && (ascii_iswhite(c_org) || ascii_iswhite(c_new)))
                    {
                        while ei_org >= *startp
                            && ascii_iswhite(
                                org_bytes.get(ei_org as usize).copied().unwrap_or(0) as i32
                            )
                        {
                            ei_org -= 1;
                        }
                        while ei_new >= si_new
                            && ascii_iswhite(
                                new_bytes.get(ei_new as usize).copied().unwrap_or(0) as i32
                            )
                        {
                            ei_new -= 1;
                        }
                    } else {
                        let p1_idx = (ei_org
                            - crate::mbyte::utf_head_off_slice(org_slice, ei_org as usize))
                        .max(0) as usize;
                        let p2_idx = (ei_new
                            - crate::mbyte::utf_head_off_slice(new_slice, ei_new as usize))
                        .max(0) as usize;
                        let mut l: i32 = 0;
                        if !diff_equal_char(
                            org_slice.get(p1_idx..).unwrap_or(&[]),
                            new_slice.get(p2_idx..).unwrap_or(&[]),
                            &mut l,
                        ) {
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
    drop(line_org_owned);
    added
}
fn diff_refine_inline_char_highlight(linemap: &[Vec<LinemapEntry>; DB_COUNT], idx1: usize) {
    let mut pass: i32 = 1;
    loop {
        let mut has_unmerged_gaps = false;
        let mut has_merged_gaps = false;
        let mut di: usize = 0;
        while di + 1 < curtab().tp_diffs.borrow().len() {
            let lm = &linemap[idx1];
            let lm_len = lm.len() as i32;
            let (di_lnum, di_count, next_lnum, next_count) = {
                let ct = curtab();
                let diffs = ct.tp_diffs.borrow();
                (
                    diffs[di].df_lnum,
                    diffs[di].df_count,
                    diffs[di + 1].df_lnum,
                    diffs[di + 1].df_count,
                )
            };
            if di_lnum[idx1] + di_count[idx1] > lm_len || next_lnum[idx1] > lm_len {
                di += 1;
                continue;
            }
            let entry1 = &lm[(di_lnum[idx1] + di_count[idx1] - 1) as usize];
            let entry2 = &lm[(next_lnum[idx1] - 1) as usize];
            if entry1.lineoff != entry2.lineoff {
                di += 1;
                continue;
            }
            let gap = next_lnum[idx1] - (di_lnum[idx1] + di_count[idx1]);
            if gap <= 3 {
                let mut max_df_count: Linenr = 0;
                for i in 0..DB_COUNT {
                    max_df_count = max_df_count.max(di_count[i] + next_count[i]);
                }
                if max_df_count >= gap * 4 {
                    for i in 0..DB_COUNT {
                        curtab().tp_diffs.borrow_mut()[di].df_count[i] =
                            next_lnum[i] + next_count[i] - di_lnum[i];
                    }
                    diff_free(&curtab(), di + 1);
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
fn diff_refine_inline_word_highlight(
    linemap: &[Vec<LinemapEntry>; DB_COUNT],
    idx1: usize,
    start_lnum: Linenr,
) {
    let mut pass: i32 = 1;
    loop {
        let mut di: usize = 0;
        while di + 1 < curtab().tp_diffs.borrow().len() {
            let (di_lnum, di_count, next_lnum, next_count) = {
                let ct = curtab();
                let diffs = ct.tp_diffs.borrow();
                (
                    diffs[di].df_lnum,
                    diffs[di].df_count,
                    diffs[di + 1].df_lnum,
                    diffs[di + 1].df_count,
                )
            };
            let idx_entry1 = di_lnum[idx1] + di_count[idx1] - 2;
            let idx_entry2 = next_lnum[idx1] - 1;
            let lm1 = &linemap[idx1];
            let lm1_len = lm1.len() as i32;
            if idx_entry1 < 0 || idx_entry1 >= lm1_len || idx_entry2 < 0 || idx_entry2 >= lm1_len {
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
            if gap_size <= 0 || gap_size > DIFF_WORD_GAP {
                di += 1;
                continue;
            }
            let line_cstr = ml_get_buf(&curtab().diffbuf(idx1), start_lnum + entry1.lineoff);
            let line_bytes = line_cstr.to_bytes();
            let gap_text = line_bytes.get(gap_start as usize..).unwrap_or(&[]);
            let mut only_non_word = true;
            let mut has_content = false;
            let mut i: usize = 0;
            while i < (gap_size as usize).min(gap_text.len()) {
                has_content = true;
                let char_class =
                    mb_get_class_tab(&gap_text[i..], &curtab().diffbuf(idx1).b_chartab.get());
                if char_class == 2 {
                    only_non_word = false;
                    break;
                }
                i += 1;
            }
            if has_content && only_non_word {
                let mut total_change_bytes: i64 = 0;
                for (buf_i, lmi) in linemap.iter().enumerate() {
                    if !curtab().tp_diffbuf[buf_i].get().is_null() {
                        for k in 0..di_count[buf_i] {
                            let idx = di_lnum[buf_i] + k - 1;
                            if let Some(entry) = lmi.get(idx as usize) {
                                total_change_bytes += entry.num_bytes as i64;
                            }
                        }
                        for k in 0..next_count[buf_i] {
                            let idx = next_lnum[buf_i] + k - 1;
                            if let Some(entry) = lmi.get(idx as usize) {
                                total_change_bytes += entry.num_bytes as i64;
                            }
                        }
                    }
                }
                if total_change_bytes >= (gap_size * 2) as i64 {
                    for buf_i in 0..DB_COUNT {
                        if !curtab().tp_diffbuf[buf_i].get().is_null() {
                            curtab().tp_diffs.borrow_mut()[di].df_count[buf_i] =
                                next_lnum[buf_i] + next_count[buf_i] - di_lnum[buf_i];
                        }
                    }
                    diff_free(&curtab(), di + 1);
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
fn diff_find_change_inline_diff(dp_idx: usize) {
    let save_diff_algorithm = diff_st(|st| {
        let old = st.diff_algorithm.get();
        st.diff_algorithm.set(old | XDF_INDENT_HEURISTIC);
        old
    });
    let mut dio = Diffio {
        dio_orig: Diffin {
            din_fname: None,
            din_buf: Vec::new(),
        },
        dio_new: Diffin {
            din_fname: None,
            din_buf: Vec::new(),
        },
        dio_diff: Diffout {
            dout_fname: None,
            dout_ga: Vec::new(),
        },
        dio_internal: 1,
    };
    let flags = diff_flags();
    let dp_lnum = curtab().tp_diffs.borrow()[dp_idx].df_lnum;
    let dp_count = curtab().tp_diffs.borrow()[dp_idx].df_count;
    let orig_diff = std::mem::take(&mut *curtab().tp_diffs.borrow_mut());
    let orig_diffbuf: [crate::globals::BufPtr; DB_COUNT] =
        std::array::from_fn(|i| curtab().tp_diffbuf[i].get());
    let mut linemap: [Vec<LinemapEntry>; DB_COUNT] = std::array::from_fn(|_| Vec::new());
    let mut changes: Vec<DifflineChange> = Vec::new();
    'done: {
        let mut file1_idx: isize = -1;
        for (i, lm) in linemap.iter_mut().enumerate().take(DB_COUNT) {
            dio.dio_diff.dout_ga.clear();
            let Some(buf) = curtab().diffbuf_opt(i) else {
                continue;
            };
            if buf.b_ml.ml_mfp.borrow().is_none() {
                continue;
            }
            if dp_count[i] == 0 {
                curtab().tp_diffbuf[i].set(crate::globals::BufPtr::NULL);
                continue;
            }
            if file1_idx == -1 {
                file1_idx = i as isize;
            }
            let curstr: &mut Vec<u8> = if file1_idx != i as isize {
                &mut dio.dio_new.din_buf
            } else {
                &mut dio.dio_orig.din_buf
            };
            let mut numlines: Linenr = 0;
            curstr.clear();
            for off in 0..dp_count[i] {
                let curline_cstr = ml_get_buf(&curtab().diffbuf(i), dp_lnum[i] + off);
                let curline_bytes = curline_cstr.to_bytes();
                let mut in_keyword = false;
                let mut last_white = false;
                let mut eol_ga_len: i32 = -1;
                let mut eol_linemap_len: i32 = -1;
                let mut eol_numlines: Linenr = -1;
                let mut s = curline_bytes;
                while !s.is_empty() {
                    let mut new_in_keyword = false;
                    if (flags & DIFF_INLINE_WORD) != 0 {
                        new_in_keyword = mb_get_class_tab(
                            s,
                            &curtab().diffbuf(file1_idx as usize).b_chartab.get(),
                        ) == 2;
                    }
                    if in_keyword && !new_in_keyword {
                        curstr.push(NL as u8);
                        numlines += 1;
                    }
                    if ascii_iswhite(s[0]) {
                        if (flags & DIFF_IWHITEALL) != 0 {
                            in_keyword = false;
                            s = crate::charset::skipwhite_slice(s);
                            continue;
                        } else if ((flags & DIFF_IWHITEEOL) != 0 || (flags & DIFF_IWHITE) != 0)
                            && !last_white
                        {
                            eol_ga_len = curstr.len() as i32;
                            eol_linemap_len = lm.len() as i32;
                            eol_numlines = numlines;
                            last_white = true;
                        }
                    } else if (flags & DIFF_IWHITEEOL) != 0 || (flags & DIFF_IWHITE) != 0 {
                        last_white = false;
                        eol_ga_len = -1;
                        eol_linemap_len = -1;
                        eol_numlines = -1;
                    }
                    let mut char_len: i32 = 1;
                    if s[0] == NL as u8 {
                        curstr.push(NUL as u8);
                    } else {
                        char_len = crate::mbyte::utfc_slice2len(s) as i32;
                        if ascii_iswhite(s[0]) && (flags & DIFF_IWHITE) != 0 {
                            char_len = (s.len() - crate::charset::skipwhite_slice(s).len()) as i32;
                        }
                        if (flags & DIFF_ICASE) != 0 {
                            let mut c = crate::mbyte::utf_slice2char(s);
                            let c_len = utf_char2len(c);
                            c = utf_fold(c);
                            let mut cbuf: [u8; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];
                            let c_fold_len = utf_char2bytes(c, &mut cbuf);
                            curstr.extend_from_slice(&cbuf[..c_fold_len as usize]);
                            if char_len > c_len {
                                curstr.extend_from_slice(&s[c_len as usize..char_len as usize]);
                            }
                        } else {
                            curstr.extend_from_slice(&s[..char_len as usize]);
                        }
                    }
                    if !new_in_keyword {
                        curstr.push(NL as u8);
                        numlines += 1;
                    }
                    if !new_in_keyword || (new_in_keyword && !in_keyword) {
                        lm.push(LinemapEntry {
                            byte_start: (curline_bytes.len() - s.len()) as Colnr,
                            num_bytes: char_len,
                            lineoff: off,
                        });
                    } else if let Some(last) = lm.last_mut() {
                        last.num_bytes += char_len;
                    }
                    in_keyword = new_in_keyword;
                    s = &s[char_len as usize..];
                }
                if in_keyword {
                    curstr.push(NL as u8);
                    numlines += 1;
                }
                if ((flags & DIFF_IWHITEEOL) != 0 || (flags & DIFF_IWHITE) != 0) && eol_ga_len != -1
                {
                    curstr.truncate(eol_ga_len as usize);
                    lm.truncate(eol_linemap_len as usize);
                    numlines = eol_numlines;
                }
                if (flags & DIFF_IWHITEALL) == 0 {
                    curstr.push(NL as u8);
                    let _ = numlines;
                    lm.push(LinemapEntry {
                        byte_start: (curline_bytes.len() - s.len()) as Colnr,
                        num_bytes: size_of::<i32>() as Colnr,
                        lineoff: off,
                    });
                }
            }

            if file1_idx != i as isize {
                let diff_status = diff_file_internal(&mut dio);
                if !diff_status {
                    break 'done;
                }
                diff_read(0, i, &mut dio);
                clear_diffout(&mut dio.dio_diff);
            }
        }
        if (flags & DIFF_INLINE_WORD) != 0 && file1_idx != -1 {
            diff_refine_inline_word_highlight(
                &linemap,
                file1_idx as usize,
                dp_lnum[file1_idx as usize],
            );
        } else if (flags & DIFF_INLINE_CHAR) != 0 && file1_idx != -1 {
            diff_refine_inline_char_highlight(&linemap, file1_idx as usize);
        }
        for new_diff in curtab().tp_diffs.borrow().iter() {
            let mut change = DifflineChange {
                dc_start: [0; DB_COUNT],
                dc_end: [0; DB_COUNT],
                dc_start_lnum_off: [0; DB_COUNT],
                dc_end_lnum_off: [0; DB_COUNT],
            };
            for (i, lm) in linemap.iter().enumerate() {
                if new_diff.df_lnum[i] <= 0 {
                    continue;
                }
                let lm_len = lm.len() as i32;
                let diff_lnum = new_diff.df_lnum[i] - 1;
                let diff_lnum_end = diff_lnum + new_diff.df_count[i];
                if diff_lnum >= lm_len {
                    change.dc_start[i] = MAXCOL;
                    change.dc_start_lnum_off[i] = i32::MAX;
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
                    change.dc_end_lnum_off[i] = i32::MAX;
                } else {
                    let e = &lm[(diff_lnum_end - 1) as usize];
                    change.dc_end[i] = e.byte_start + e.num_bytes;
                    change.dc_end_lnum_off[i] = e.lineoff;
                }
            }
            changes.push(change);
        }
    }
    diff_st(|st| st.diff_algorithm.set(save_diff_algorithm));
    diff_clear(&curtab());
    *curtab().tp_diffs.borrow_mut() = orig_diff;
    for (i, b) in orig_diffbuf.iter().enumerate() {
        curtab().tp_diffbuf[i].set(b.clone());
    }
    {
        let ct = curtab();
        let mut diffs = ct.tp_diffs.borrow_mut();
        diffs[dp_idx].df_changes = changes;
        diffs[dp_idx].has_changes = true;
    }
    clear_diffout(&mut dio.dio_diff);
}
pub fn diff_find_change(wp: &WinInfoInfo, lnum: Linenr, diffline: &mut Diffline) -> bool {
    let idx = diff_buf_idx(&wp.buf(), &curtab());
    if idx == DB_COUNT as i32 {
        return false;
    }
    let u_idx = idx as usize;
    let Some(dp_idx) = curtab()
        .tp_diffs
        .borrow()
        .iter()
        .position(|dp| lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx])
    else {
        return false;
    };
    if !diff_check_sanity(&curtab(), &curtab().tp_diffs.borrow()[dp_idx]) {
        return false;
    }
    let off = lnum - curtab().tp_diffs.borrow()[dp_idx].df_lnum[u_idx];
    if (diff_flags() & ALL_INLINE_DIFF) == 0 {
        let mut change_start: i32 = MAXCOL;
        let mut change_end: i32 = -1;
        let ret = diff_find_change_simple(
            wp,
            lnum,
            &curtab().tp_diffs.borrow()[dp_idx],
            u_idx,
            &mut change_start,
            &mut change_end,
        );
        change_end += 1;
        let mut simple_change = DifflineChange {
            dc_start: [0; DB_COUNT],
            dc_end: [0; DB_COUNT],
            dc_start_lnum_off: [0; DB_COUNT],
            dc_end_lnum_off: [0; DB_COUNT],
        };
        simple_change.dc_start[u_idx] = change_start;
        simple_change.dc_end[u_idx] = change_end;
        simple_change.dc_start_lnum_off[u_idx] = off;
        simple_change.dc_end_lnum_off[u_idx] = off;
        diffline.changes.clear();
        diffline.changes.push(simple_change);
        diffline.is_simple = true;
        diffline.bufidx = idx;
        diffline.lineoff = lnum - curtab().tp_diffs.borrow()[dp_idx].df_lnum[u_idx];
        return ret;
    }
    if !curtab().tp_diffs.borrow()[dp_idx].has_changes {
        diff_find_change_inline_diff(dp_idx);
    }
    let ct = curtab();
    let mut diffs = ct.tp_diffs.borrow_mut();
    let dp = &mut diffs[dp_idx];
    let changes_len = dp.df_changes.len() as i32;
    let mut change_idx: i32 = 0;
    diffline.changes.clear();
    diffline.is_simple = false;
    while change_idx < changes_len {
        let change = &dp.df_changes[change_idx as usize];
        if change.dc_end_lnum_off[u_idx] < off {
            change_idx += 1;
            continue;
        }
        if change.dc_start_lnum_off[u_idx] > off {
            break;
        }
        diffline.changes.push(*change);
        change_idx += 1;
    }
    let num_changes = diffline.changes.len() as i32;
    diffline.bufidx = idx;
    diffline.lineoff = off;
    let mut added = false;
    if num_changes == 1 && change_idx == changes_len {
        added = true;
        for i in 0..DB_COUNT {
            if u_idx == i {
                continue;
            }
            if curtab().tp_diffbuf[i].get().is_null() {
                continue;
            }
            let change = &dp.df_changes[(changes_len - 1) as usize];
            if change.dc_start_lnum_off[i] != i32::MAX {
                added = false;
                break;
            }
        }
    }
    added
}
pub fn diff_infold(wp: &WinInfoInfo, tp: Option<crate::globals::TabpagePtr>, lnum: Linenr) -> bool {
    if !wp.w_onebuf_opt.wo_diff.get() {
        return false;
    }

    let tp = tp.unwrap_or_else(curtab);

    let mut idx: i32 = -1;
    let mut other = false;
    for i in 0..DB_COUNT {
        if tp.tp_diffbuf[i].get() == wp.w_buffer.get() {
            idx = i as i32;
        } else if !tp.tp_diffbuf[i].get().is_null() {
            other = true;
        }
    }
    if idx == -1 || !other {
        return false;
    }
    let u_idx = idx as usize;
    if g_st().curtab.get() == tp && tp.tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if tp.tp_diffs.borrow().is_empty() {
        return true;
    }
    let mut le: Option<usize> = None;
    let mut result = true;
    let mut dp_idx = diff_scan_start(&tp, idx, lnum);
    while dp_idx < tp.tp_diffs.borrow().len() {
        let diffs = tp.tp_diffs.borrow();
        let dp = &diffs[dp_idx];
        if dp.df_lnum[u_idx] <= lnum {
            le = Some(dp_idx);
        }
        if dp.df_lnum[u_idx] - g_st().diff_context.get() > lnum {
            break;
        }
        if dp.df_lnum[u_idx] + dp.df_count[u_idx] + g_st().diff_context.get() > lnum {
            result = false;
            break;
        }
        dp_idx += 1;
    }
    diff_scan_remember(&tp, idx, lnum, le);
    result
}
pub fn nv_diffgetput(put: bool, count: usize) {
    if bt_prompt(Some(&crate::globals::curbuf())) {
        vim_beep(K_OPT_BO_FLAG_OPERATOR);
        return;
    }
    let mut ea = Exarg::new();
    let mut buf: [u8; 30] = [0; 30];
    if count == 0 {
        ea.arg = crate::strings::OwnedSlice::from_cstr(c"");
    } else {
        format_to_buf!(&mut buf, c"%zu", count);
        ea.arg = crate::strings::OwnedSlice::from_bytes(&buf);
    }
    ea.cmdidx = if put {
        Cmdidx::CmdDiffput
    } else {
        Cmdidx::CmdDiffget
    };
    ea.addr_count = 0;
    ea.line1 = curwin().w_cursor.get().lnum;
    ea.line2 = curwin().w_cursor.get().lnum;
    ex_diffgetput(&mut ea);
}
fn valid_diff(df_id: Option<u64>) -> Option<usize> {
    let id = df_id?;
    curtab()
        .tp_diffs
        .borrow()
        .iter()
        .position(|dp| dp.df_id == id)
}
pub fn ex_diffgetput(eap: &mut Exarg) {
    let idx_other: usize;
    let idx_cur_i = diff_buf_idx(&curbuf(), &curtab());
    if idx_cur_i == DB_COUNT as i32 {
        emsg(gettext(c"E99: Current buffer is not in diff mode"));
        return;
    }
    let idx_cur = idx_cur_i as usize;
    if eap.arg.is_empty() {
        let mut found_not_ma = false;
        let mut idx = 0;
        while idx < DB_COUNT {
            if curtab().tp_diffbuf[idx].get() != g_st().curbuf.get()
                && let Some(buf) = curtab().diffbuf_opt(idx)
            {
                if eap.cmdidx != Cmdidx::CmdDiffput || modifiable(&buf) {
                    break;
                }
                found_not_ma = true;
            }
            idx += 1;
        }
        idx_other = idx;
        if idx_other == DB_COUNT {
            if found_not_ma {
                emsg(gettext(c"E793: No other buffer in diff mode is modifiable"));
            } else {
                emsg(gettext(c"E100: No other buffer in diff mode"));
            }
            return;
        }
        for i in (idx_other + 1)..DB_COUNT {
            if curtab().tp_diffbuf[i].get() != g_st().curbuf.get()
                && let Some(buf) = curtab().diffbuf_opt(i)
                && (eap.cmdidx != Cmdidx::CmdDiffput || modifiable(&buf))
            {
                emsg(gettext(
                    c"E101: More than two buffers in diff mode, don't know which one to use",
                ));
                return;
            }
        }
    } else {
        let arg_bytes = eap.arg.as_bytes();
        let mut p_idx = arg_bytes.len();
        while p_idx > 0 && ascii_iswhite(arg_bytes[p_idx - 1] as i32) {
            p_idx -= 1;
        }
        let bufnr = if arg_bytes[..p_idx].iter().all(u8::is_ascii_digit) {
            crate::charset::parse_i64_prefix(arg_bytes) as i32
        } else {
            let nr = buflist_findpat(eap.arg.slice(p_idx), false, true, false);
            if nr < 0 {
                return;
            }
            nr
        };
        let Some(buf) = buflist_findnr(bufnr) else {
            semsg!(gettext(c"E102: Can't find buffer \"%s\""), eap.arg,);
            return;
        };
        if g_st().curbuf.get() == buf {
            return;
        }
        let idx_o = diff_buf_idx(&buf, &curtab());
        if idx_o == DB_COUNT as i32 {
            semsg!(gettext(c"E103: Buffer \"%s\" is not in diff mode"), eap.arg,);
            return;
        }
        idx_other = idx_o as usize;
    }
    diff_st(|st| st.diff_busy.set(true));
    if eap.addr_count == 0 {
        let mut linestatus: i32 = 0;
        if eap.line1 == curbuf().b_ml.ml_line_count.get()
            && diff_check_with_linestatus(&curwin(), eap.line1, Some(&mut linestatus)) == 0
            && linestatus == 0
            && (eap.line1 == 1
                || (diff_check_with_linestatus(&curwin(), eap.line1 - 1, Some(&mut linestatus))
                    >= 0
                    && linestatus == 0))
        {
            eap.line2 += 1;
        } else if eap.line1 > 0 {
            eap.line1 -= 1;
        }
    }
    let mut aco = CtxSwitch::new();
    if eap.cmdidx != Cmdidx::CmdDiffget {
        ctx_switch(
            &mut aco,
            None,
            None,
            curtab().diffbuf_opt(idx_other).as_deref(),
            0,
        );
    }
    let idx_from = if eap.cmdidx == Cmdidx::CmdDiffget {
        idx_other
    } else {
        idx_cur
    };
    let idx_to = if eap.cmdidx == Cmdidx::CmdDiffget {
        idx_cur
    } else {
        idx_other
    };
    'theend: {
        if curbuf().b_changed.get() == 0 {
            change_warning(&curbuf(), 0);
            if diff_buf_idx(&curbuf(), &curtab()) != idx_to as i32 {
                emsg(gettext(E_BUFFER_CHANGED_UNEXPECTEDLY));
                ctx_restore(&mut aco);
                break 'theend;
            }
        }
        diffgetput(
            eap.addr_count,
            idx_cur,
            idx_from,
            idx_to,
            eap.line1,
            eap.line2,
        );
        if eap.cmdidx != Cmdidx::CmdDiffget {
            if g_st().key_typed.get() {
                u_sync(false);
            }
            ctx_restore(&mut aco);
        }
    }
    let need_update = diff_st(|st| {
        st.diff_busy.set(false);
        st.diff_need_update.get()
    });
    if need_update {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    check_cursor(&curwin());
    changed_line_abv_curs();
    if curtab().tp_diffs.borrow().is_empty() {
        for wp in for_all_windows_ptr() {
            if wp.w_onebuf_opt.wo_diff.get()
                && wp.w_onebuf_opt.wo_fdm.first_byte() == b'd'
                && wp.w_onebuf_opt.wo_fen.get()
            {
                fold_update_all(&wp);
            }
        }
    }
    if diff_st(|st| st.diff_need_update.replace(false)) {
    } else {
        diff_redraw(false);
        window_layout_lock();
        apply_autocmds(
            EVENT_DIFFUPDATED,
            None,
            None,
            false,
            curbuf_opt().as_deref(),
        );
        window_layout_unlock();
    }
}
fn diffgetput(
    addr_count: i32,
    idx_cur: usize,
    idx_from: usize,
    idx_to: usize,
    line1: Linenr,
    line2: Linenr,
) {
    let mut off: Linenr = 0;
    let mut dp_idx: usize = 0;
    while dp_idx < curtab().tp_diffs.borrow().len() {
        if addr_count == 0 {
            while {
                let ct = curtab();
                let diffs = ct.tp_diffs.borrow();
                dp_idx + 1 < diffs.len()
                    && diffs[dp_idx + 1].df_lnum[idx_cur]
                        == diffs[dp_idx].df_lnum[idx_cur] + diffs[dp_idx].df_count[idx_cur]
                    && diffs[dp_idx + 1].df_lnum[idx_cur] == line1 + off + 1
            } {
                dp_idx += 1;
            }
        }
        if curtab().tp_diffs.borrow()[dp_idx].df_lnum[idx_cur] > line2 + off {
            break;
        }
        let mut dfree_lnum = [0; DB_COUNT];
        let mut dfree_count = [0; DB_COUNT];
        let mut did_free = false;
        let (mut lnum, mut count, dp_lnum_cur, dp_count_cur, dp_lnum_from, dp_count_from) = {
            let ct = curtab();
            let diffs = ct.tp_diffs.borrow();
            let dp = &diffs[dp_idx];
            (
                dp.df_lnum[idx_to],
                dp.df_count[idx_to],
                dp.df_lnum[idx_cur],
                dp.df_count[idx_cur],
                dp.df_lnum[idx_from],
                dp.df_count[idx_from],
            )
        };
        let undo_bot = lnum + count + Linenr::from(count == 0 && buf_is_empty(&curbuf()));
        if dp_lnum_cur + dp_count_cur > line1 + off && u_save(lnum - 1, undo_bot) {
            let mut start_skip: Linenr = 0;
            let mut end_skip: Linenr = 0;
            if addr_count > 0 {
                start_skip = line1 + off - dp_lnum_cur;
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
                end_skip = dp_lnum_cur + dp_count_cur - 1 - (line2 + off);
                if end_skip > 0 {
                    if idx_cur == idx_from {
                        count = count.min(dp_count_cur - start_skip - end_skip);
                    } else {
                        count -= end_skip;
                        end_skip = (dp_count_from - start_skip - count).max(0);
                    }
                } else {
                    end_skip = 0;
                }
            }
            let mut buf_empty = buf_is_empty(&curbuf());
            let mut added: i32 = 0;
            for _ in 0..count {
                buf_empty = curbuf().b_ml.ml_line_count.get() == 1;
                if ml_delete(lnum) {
                    added -= 1;
                }
            }
            let copy_count = dp_count_from - start_skip - end_skip;
            let from_lnum = dp_lnum_from;
            for i in 0..copy_count {
                let nr = from_lnum + start_skip + i;
                let from_buf = curtab().diffbuf(idx_from);
                if nr > from_buf.b_ml.ml_line_count.get() {
                    break;
                }
                let p = crate::memory::bytes_to_cstring(&crate::memline::ml_get_buf_slice(
                    &from_buf, nr,
                ));
                let _ = ml_append(lnum + i - 1, p.to_bytes(), 0, false);
                added += 1;
                if buf_empty && curbuf().b_ml.ml_line_count.get() == 2 {
                    buf_empty = false;
                    let _ = ml_delete(2);
                }
            }
            let new_count = curtab().tp_diffs.borrow()[dp_idx].df_count[idx_to] + added;
            curtab().tp_diffs.borrow_mut()[dp_idx].df_count[idx_to] = new_count;
            let mut target_id = Some(curtab().tp_diffs.borrow()[dp_idx].df_id);
            if start_skip == 0 && end_skip == 0 {
                let mut i = 0;
                while i < DB_COUNT {
                    if !curtab().tp_diffbuf[i].get().is_null()
                        && i != idx_from
                        && i != idx_to
                        && !diff_equal_entry(&curtab().tp_diffs.borrow()[dp_idx], idx_from, i)
                    {
                        break;
                    }
                    i += 1;
                }
                if i == DB_COUNT {
                    dfree_lnum = curtab().tp_diffs.borrow()[dp_idx].df_lnum;
                    dfree_count = curtab().tp_diffs.borrow()[dp_idx].df_count;
                    did_free = true;
                    target_id = curtab()
                        .tp_diffs
                        .borrow()
                        .as_slice()
                        .get(dp_idx + 1)
                        .map(|d| d.df_id);
                    diff_free(&curtab(), dp_idx);
                }
            }
            if added != 0 {
                mark_adjust(lnum, lnum + count - 1, MAXLNUM, added, K_EXTMARK_NOOP);
                if curwin().w_cursor.get().lnum >= lnum {
                    if curwin().w_cursor.get().lnum >= lnum + count {
                        curwin().w_cursor.add_lnum(added);
                        curwin().w_cursor.set_lnum(
                            curwin()
                                .w_cursor
                                .get()
                                .lnum
                                .min(curbuf().b_ml.ml_line_count.get()),
                        );
                    } else if added < 0 {
                        curwin().w_cursor.set_lnum(lnum);
                    }
                }
            }
            extmark_adjust(
                &curbuf(),
                lnum,
                lnum + count - 1,
                MAXLNUM,
                added,
                K_EXTMARK_UNDO,
            );
            changed_lines(&curbuf(), lnum, 0, lnum + count, added, true);
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
                curtab().tp_diffs.borrow_mut()[dp_idx].df_count[idx_to] = new_count;
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
fn diff_fold_update(df_lnum: &[Linenr; DB_COUNT], df_count: &[Linenr; DB_COUNT], skip_idx: usize) {
    for wp in for_all_windows_ptr() {
        for i in 0..DB_COUNT {
            if curtab().tp_diffbuf[i].get() == wp.w_buffer.get() && i != skip_idx {
                fold_update(&wp, df_lnum[i], df_lnum[i] + df_count[i]);
            }
        }
    }
}
pub fn diff_mode_buf(buf: &FileBuf) -> bool {
    for tp in for_all_tabpages_ptr() {
        if diff_buf_idx(buf, &tp) != DB_COUNT as i32 {
            return true;
        }
    }
    false
}
pub fn diff_move_to(dir: i32, mut count: i32) -> bool {
    let mut lnum = curwin().w_cursor.get().lnum;
    let idx = diff_buf_idx(&curbuf(), &curtab());
    if idx == DB_COUNT as i32 || curtab().tp_diffs.borrow().is_empty() {
        return false;
    }
    let u_idx = idx as usize;
    if curtab().tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab().tp_diffs.borrow().is_empty() {
        return false;
    }
    while {
        count -= 1;
        count >= 0
    } {
        let ct = curtab();
        let diffs = ct.tp_diffs.borrow();
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
    lnum = lnum.min(curbuf().b_ml.ml_line_count.get());
    if lnum == curwin().w_cursor.get().lnum {
        return false;
    }
    setpcmark();
    curwin().w_cursor.set_lnum(lnum);
    curwin().w_cursor.set_col(0);
    true
}
fn diff_get_corresponding_line_int(buf1: &FileBuf, lnum1: Linenr) -> Linenr {
    let mut baseline: Linenr = 0;
    let idx1 = diff_buf_idx(buf1, &curtab());
    let idx2 = diff_buf_idx(&curbuf(), &curtab());
    if idx1 == DB_COUNT as i32 || idx2 == DB_COUNT as i32 || curtab().tp_diffs.borrow().is_empty() {
        return lnum1;
    }
    if curtab().tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab().tp_diffs.borrow().is_empty() {
        return lnum1;
    }
    let u_idx1 = idx1 as usize;
    let u_idx2 = idx2 as usize;
    for dp in curtab().tp_diffs.borrow().iter() {
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
            && dp.df_lnum[u_idx2] <= curwin().w_cursor.get().lnum
            && dp.df_lnum[u_idx2] + dp.df_count[u_idx2] > curwin().w_cursor.get().lnum
        {
            return curwin().w_cursor.get().lnum;
        }
        baseline =
            (dp.df_lnum[u_idx1] + dp.df_count[u_idx1]) - (dp.df_lnum[u_idx2] + dp.df_count[u_idx2]);
    }
    lnum1 - baseline
}
pub fn diff_get_corresponding_line(buf1: &FileBuf, lnum1: Linenr) -> Linenr {
    let lnum = diff_get_corresponding_line_int(buf1, lnum1);
    lnum.min(curbuf().b_ml.ml_line_count.get())
}
pub fn diff_lnum_win(lnum: Linenr, wp: &WinInfoInfo) -> Linenr {
    let idx = diff_buf_idx(&curbuf(), &curtab());
    if idx == DB_COUNT as i32 {
        return 0;
    }
    let u_idx = idx as usize;
    if curtab().tp_diff_invalid.get() != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    let Some((dp_lnum, dp_count)) = curtab()
        .tp_diffs
        .borrow()
        .iter()
        .find(|dp| lnum <= dp.df_lnum[u_idx] + dp.df_count[u_idx])
        .map(|dp| (dp.df_lnum, dp.df_count))
    else {
        return wp.buf().b_ml.ml_line_count.get() - (curbuf().b_ml.ml_line_count.get() - lnum);
    };
    let i = diff_buf_idx(&wp.buf(), &curtab());
    if i == DB_COUNT as i32 {
        return 0;
    }
    let u_i = i as usize;
    let n = lnum + (dp_lnum[u_i] - dp_lnum[u_idx]);
    n.min(dp_lnum[u_i] + dp_count[u_i])
}
fn parse_diff_ed(line: &[u8], hunk: &mut Diffhunk) -> bool {
    let mut p = line;
    let f1 = getdigits_int32(&mut p, true, 0);
    let l1 = if let Some(after_comma) = p.strip_prefix(b",") {
        p = after_comma;
        getdigits_int(&mut p, true, 0)
    } else {
        f1
    };
    let Some(&difftype @ (b'a' | b'c' | b'd')) = p.first() else {
        return false;
    };
    p = &p[1..];
    let f2 = getdigits_int(&mut p, true, 0);
    let l2 = if let Some(after_comma) = p.strip_prefix(b",") {
        p = after_comma;
        getdigits_int(&mut p, true, 0)
    } else {
        f2
    };
    if l1 < f1 || l2 < f2 {
        return false;
    }
    if difftype == b'a' {
        hunk.lnum_orig = f1 + 1;
        hunk.count_orig = 0;
    } else {
        hunk.lnum_orig = f1;
        hunk.count_orig = l1 - f1 + 1;
    }
    if difftype == b'd' {
        hunk.lnum_new = f2 + 1;
        hunk.count_new = 0;
    } else {
        hunk.lnum_new = f2;
        hunk.count_new = l2 - f2 + 1;
    }
    true
}
fn parse_diff_unified(line: &[u8], hunk: &mut Diffhunk) -> bool {
    let Some(mut p) = line.strip_prefix(b"@@ -") else {
        return false;
    };
    let mut oldline = getdigits_int32(&mut p, true, 0);
    let oldcount = if let Some(after_comma) = p.strip_prefix(b",") {
        p = after_comma;
        getdigits_int(&mut p, true, 0)
    } else {
        1
    };
    let Some(mut p) = p.strip_prefix(b" +") else {
        return false;
    };
    let mut newline = getdigits_int(&mut p, true, 0);
    let newcount = if let Some(after_comma) = p.strip_prefix(b",") {
        p = after_comma;
        getdigits_int(&mut p, true, 0)
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
    hunk.lnum_orig = oldline;
    hunk.count_orig = oldcount;
    hunk.lnum_new = newline;
    hunk.count_new = newcount;
    true
}
pub(crate) fn xdiff_out(
    start_a: i32,
    count_a: i32,
    start_b: i32,
    count_b: i32,
    dout: &mut Diffout,
) -> i32 {
    dout.dout_ga.push(Diffhunk {
        lnum_orig: start_a + 1,
        count_orig: count_a,
        lnum_new: start_b + 1,
        count_new: count_b,
    });
    0
}
pub fn f_diff_filler(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        diff_check_fill(&curwin(), tv_get_lnum(&argvars[0])).max(0) as Varnumber,
    );
}
pub fn f_diff_hl_id(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut diffline = Diffline::default();
    let flags = diff_flags();
    let cache_results = (flags & ALL_INLINE_DIFF) == 0;
    let mut lnum = tv_get_lnum(&argvars[0]);
    if lnum < 0 {
        lnum = 0;
    }
    let cur_tick = buf_get_changedtick(&curbuf());
    let cur_fnum = curbuf().b_fnum.get();
    let need_recompute = !cache_results
        || diff_st(|st| {
            lnum != st.f_diff_hlid_prev_lnum.get()
                || st.f_diff_hlid_changedtick.get() != cur_tick
                || st.f_diff_hlid_fnum.get() != cur_fnum
                || flags != st.f_diff_hlid_prev_diff_flags.get()
        });
    if need_recompute {
        let mut linestatus: i32 = 0;
        diff_check_with_linestatus(&curwin(), lnum, Some(&mut linestatus));
        let mut change_start = diff_st(|st| st.f_diff_hlid_change_start.get());
        let mut change_end = diff_st(|st| st.f_diff_hlid_change_end.get());
        let hl_id = if linestatus < 0 {
            if linestatus == -1 {
                change_start = MAXCOL;
                change_end = -1;
                if diff_find_change(&curwin(), lnum, &mut diffline) {
                    HLF_ADD
                } else {
                    if !diffline.changes.is_empty() && cache_results {
                        change_start = diffline.changes[0].dc_start[diffline.bufidx as usize];
                        change_end = diffline.changes[0].dc_end[diffline.bufidx as usize];
                    }
                    HLF_CHD
                }
            } else {
                HLF_ADD
            }
        } else {
            0
        };
        let new_tick = buf_get_changedtick(&curbuf());
        let new_fnum = curbuf().b_fnum.get();
        diff_st(|st| {
            st.f_diff_hlid_change_start.set(change_start);
            st.f_diff_hlid_change_end.set(change_end);
            st.f_diff_hlid_hl_id.set(hl_id);
            if cache_results {
                st.f_diff_hlid_prev_lnum.set(lnum);
                st.f_diff_hlid_changedtick.set(new_tick);
                st.f_diff_hlid_fnum.set(new_fnum);
                st.f_diff_hlid_prev_diff_flags.set(flags);
            }
        });
    }
    let mut hl_id = diff_st(|st| st.f_diff_hlid_hl_id.get());
    if hl_id == HLF_CHD || hl_id == HLF_TXD {
        let col = tv_get_number(&argvars[1]) as i32 - 1;
        if cache_results {
            let (start, end) = diff_st(|st| {
                (
                    st.f_diff_hlid_change_start.get(),
                    st.f_diff_hlid_change_end.get(),
                )
            });
            hl_id = if col >= start && col < end {
                HLF_TXD
            } else {
                HLF_CHD
            };
        } else {
            hl_id = HLF_CHD;
            let mut change_start = diff_st(|st| st.f_diff_hlid_change_start.get());
            let mut change_end = diff_st(|st| st.f_diff_hlid_change_end.get());
            for change in &diffline.changes {
                let added =
                    diff_change_parse(&diffline, change, &mut change_start, &mut change_end);
                if col >= change_start && col < change_end {
                    hl_id = if added { HLF_TXA } else { HLF_TXD };
                    break;
                }
                if col < change_start {
                    break;
                }
            }
            diff_st(|st| {
                st.f_diff_hlid_change_start.set(change_start);
                st.f_diff_hlid_change_end.set(change_end);
            });
        }
        diff_st(|st| st.f_diff_hlid_hl_id.set(hl_id));
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(hl_id as Varnumber);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_diff_ed_and_unified_work() {
        let mut hunk = Diffhunk {
            lnum_orig: 0,
            count_orig: 0,
            lnum_new: 0,
            count_new: 0,
        };
        assert!(parse_diff_ed(b"2,4c5,6\n", &mut hunk));
        assert_eq!(hunk.lnum_orig, 2);
        assert_eq!(hunk.count_orig, 3);
        assert_eq!(hunk.lnum_new, 5);
        assert_eq!(hunk.count_new, 2);
        assert!(parse_diff_unified(b"@@ -10,3 +12,4 @@\n", &mut hunk));
        assert_eq!(hunk.lnum_orig, 10);
        assert_eq!(hunk.count_orig, 3);
        assert_eq!(hunk.lnum_new, 12);
        assert_eq!(hunk.count_new, 4);
    }
}
