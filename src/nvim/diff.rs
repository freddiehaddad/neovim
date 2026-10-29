//! Diff mode functions (`src/nvim/diff.c`).
use crate::api::private::helpers::{boolean_obj, static_cstr_as_obj};
use crate::autocmd::{EVENT_DIFFUPDATED, apply_autocmds};
use crate::buffer::{
    Bufref, FileBuf, MAXLNUM, bt_prompt, buf_get_changedtick, buf_is_empty, buf_valid,
    buflist_findnr, buflist_findpat, bufref_valid, modifiable, set_bufref,
};
use crate::bufwrite::buf_write;
use crate::change::{change_warning, changed_lines};
use crate::charset::{ascii_isdigit, ascii_iswhite, getdigits_int, getdigits_int32, skipwhite};
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
use crate::fileio::{FILE, buf_check_timestamp, fclose, fwrite, vim_fgets, vim_tempname};
#[cfg(unix)]
use crate::fileio::{shorten_fnames, vim_gettempdir};
use crate::fold::{
    fold_update, fold_update_all, foldmethod_is_diff, foldmethod_is_manual, has_folding,
    new_fold_level,
};
use crate::globals::*;
use crate::linematch::linematch_nbuffers;
use crate::mark::{mark_adjust, setpcmark};
use crate::mbyte::{
    mb_get_class_tab, mb_stricmp, utf_char2bytes, utf_char2len, utf_fold, utf_head_off,
    utf_ptr2char, utfc_ptr2len,
};
use crate::memline::{ml_append, ml_delete, ml_get_buf, ml_get_buf_len};
use crate::message::{emsg, semsg};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, changed_window_setting, check_topfill,
    invalidate_botline_win, validate_cursor,
};
use crate::normal::check_scrollbind;
use crate::option::{OptTarget, set_option_direct_for, set_option_value_give_err};
use crate::option_vars::*;
use crate::optionstr::{
    OptIndex, OptKeyDict_dip, alloc_string_option, free_string_option, opt_keyset,
};
use crate::os::env::{os_env_exists, os_unsetenv};
use crate::os::fs::{FileInfo, os_fileinfo, os_fileinfo_size, os_fopen, os_remove};
use crate::os::shell::call_shell;
use crate::strings::{gettext, vim_snprintf, vim_strsave_shellescape};
use crate::ui::vim_beep;
use crate::undo::{u_save, u_sync};
use crate::window::{
    TabPageInfo, WSP_VERT, WinInfoInfo, frames_locked, scroll_to_fraction, set_fraction, win_split,
    win_valid, window_layout_lock, window_layout_unlock,
};
use crate::xdiff::xdiffi::xdl_diff;
use crate::xdiff::{Mmfile, Xdemitcb, Xdemitconf, Xpparam};
use std::ffi::{CString, c_char};
use std::ptr;
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
const CMD_DIFFGET: i32 = 121;
const CMD_DIFFPUT: i32 = 124;
const CMD_SPLIT: i32 = 426;
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
    din_mmfile: Mmfile,
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
use crate::math::{FAIL, OK};
use crate::option::OPT_LOCAL;
#[cfg(unix)]
use crate::os::fs::MAXPATHL;
use crate::regexp::MAXCOL;
use crate::search::{BACKWARD, FORWARD};
const DIFF_WORD_GAP: i32 = 5;

struct DiffState {
    diff_busy: bool,
    diff_need_update: bool,
    diff_flags: i32,
    diff_algorithm: i32,
    linematch_lines: i32,
    diff_a_works: TriState,
    next_diff_id: u64,
    diff_finger_dp_idx: Option<usize>,
    diff_finger_tp: *mut TabPageInfo,
    diff_finger_idx: i32,
    diff_finger_lnum: Linenr,
    f_diff_hlid_prev_lnum: Linenr,
    f_diff_hlid_changedtick: Varnumber,
    f_diff_hlid_fnum: i32,
    f_diff_hlid_prev_diff_flags: i32,
    f_diff_hlid_change_start: i32,
    f_diff_hlid_change_end: i32,
    f_diff_hlid_hl_id: Hlf,
}

impl DiffState {
    const fn new() -> Self {
        Self {
            diff_busy: false,
            diff_need_update: false,
            diff_flags: DIFF_INTERNAL
                | DIFF_FILLER
                | DIFF_CLOSE_OFF
                | DIFF_LINEMATCH
                | DIFF_INLINE_CHAR,
            diff_algorithm: XDF_INDENT_HEURISTIC,
            linematch_lines: 40,
            diff_a_works: K_NONE,
            next_diff_id: 1,
            diff_finger_dp_idx: None,
            diff_finger_tp: ptr::null_mut(),
            diff_finger_idx: -1,
            diff_finger_lnum: 0,
            f_diff_hlid_prev_lnum: 0,
            f_diff_hlid_changedtick: 0,
            f_diff_hlid_fnum: 0,
            f_diff_hlid_prev_diff_flags: 0,
            f_diff_hlid_change_start: 0,
            f_diff_hlid_change_end: 0,
            f_diff_hlid_hl_id: 0,
        }
    }
}

static DIFF_STATE: GlobalState<DiffState> = GlobalState::new(DiffState::new());

#[inline]
fn diff_st() -> &'static mut DiffState {
    unsafe { &mut *DIFF_STATE.as_ptr() }
}
#[inline]
const fn has_dip_key(v: &OptKeyDict_dip, idx: u64) -> bool {
    (v.is_set_dip & (1u64 << idx)) != 0
}
fn diff_scan_start(tp: &mut TabPageInfo, idx: i32, lnum: Linenr) -> usize {
    if let Some(dp_idx) = diff_st().diff_finger_dp_idx
        && diff_st().diff_finger_tp == tp
        && diff_st().diff_finger_idx == idx
        && lnum >= diff_st().diff_finger_lnum
        && dp_idx < tp.tp_diffs.len()
    {
        return dp_idx;
    }
    0
}
fn diff_scan_remember(tp: &mut TabPageInfo, idx: i32, lnum: Linenr, dp_idx: Option<usize>) {
    diff_st().diff_finger_tp = tp;
    diff_st().diff_finger_idx = idx;
    diff_st().diff_finger_lnum = lnum;
    diff_st().diff_finger_dp_idx = Some(dp_idx.unwrap_or(0));
}
pub fn diff_buf_delete(buf: *mut FileBuf) {
    for tp in for_all_tabpages_ptr() {
        let i = diff_buf_idx(buf, &mut *tp);
        if i != DB_COUNT as i32 {
            tp.tp_diffbuf[i as usize] = ptr::null_mut();
            tp.tp_diff_invalid = 1;
            if ptr::eq(tp, g_st().curtab) {
                g_st().need_diff_redraw = true;
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
            }
        }
    }
}
pub fn diff_buf_adjust(win: &mut WinInfoInfo) {
    if win.w_onebuf_opt.wo_diff == 0 {
        let mut found_win = false;
        for wp in for_all_windows_mut() {
            if wp.w_buffer == win.w_buffer && wp.w_onebuf_opt.wo_diff != 0 {
                found_win = true;
            }
        }
        if !found_win {
            let i = diff_buf_idx(win.w_buffer, curtab_mut());
            if i != DB_COUNT as i32 {
                curtab_mut().tp_diffbuf[i as usize] = ptr::null_mut();
                curtab_mut().tp_diff_invalid = 1;
                diff_redraw(true);
            }
        }
    } else {
        diff_buf_add(win.w_buffer);
    }
}
pub fn diff_buf_add(buf: *mut FileBuf) {
    if diff_buf_idx(buf, curtab_mut()) != DB_COUNT as i32 {
        return;
    }
    for i in 0..DB_COUNT {
        if curtab_mut().tp_diffbuf[i].is_null() {
            curtab_mut().tp_diffbuf[i] = buf;
            curtab_mut().tp_diff_invalid = 1;
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
        if !curtab_mut().tp_diffbuf[i].is_null() {
            curtab_mut().tp_diffbuf[i] = ptr::null_mut();
            curtab_mut().tp_diff_invalid = 1;
            diff_redraw(true);
        }
    }
}
fn diff_buf_idx(buf: *mut FileBuf, tp: &mut TabPageInfo) -> i32 {
    let mut idx: usize = 0;
    while idx < DB_COUNT {
        if tp.tp_diffbuf[idx] == buf {
            break;
        }
        idx += 1;
    }
    idx as i32
}
pub fn diff_invalidate(buf: *mut FileBuf) {
    for tp in for_all_tabpages_ptr() {
        let i = diff_buf_idx(buf, &mut *tp);
        if i != DB_COUNT as i32 {
            tp.tp_diff_invalid = 1;
            if ptr::eq(tp, g_st().curtab) {
                diff_redraw(true);
            }
        }
    }
}
pub fn diff_mark_adjust(
    buf: &mut FileBuf,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    for tp in for_all_tabpages_mut() {
        let idx = diff_buf_idx(buf, tp);
        if idx != DB_COUNT as i32 {
            diff_mark_adjust_tp(tp, idx as usize, line1, line2, amount, amount_after);
        }
    }
}
fn diff_mark_adjust_tp(
    tp: &mut TabPageInfo,
    idx: usize,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    diff_st().diff_finger_dp_idx = None;
    if diff_internal() != 0 {
        tp.tp_diff_invalid = 1;
        tp.tp_diff_update = 1;
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
        let dp_past_line2 = dp_idx >= tp.tp_diffs.len()
            || (&tp.tp_diffs)[dp_idx].df_lnum[idx] - 1 > line2
            || (line2 == MAXLNUM && (&tp.tp_diffs)[dp_idx].df_lnum[idx] > line1);
        let dprev_before_line1 = match dprev_idx {
            None => true,
            Some(pi) => (&tp.tp_diffs)[pi].df_lnum[idx] + (&tp.tp_diffs)[pi].df_count[idx] < line1,
        };
        if dp_past_line2 && dprev_before_line1 && !diff_st().diff_busy {
            let dprev_copy =
                dprev_idx.map(|pi| ((&tp.tp_diffs)[pi].df_lnum, (&tp.tp_diffs)[pi].df_count));
            let dnext_idx = diff_alloc_new(tp, dp_idx);
            dp_idx += 1;
            (&mut tp.tp_diffs)[dnext_idx].df_lnum[idx] = line1;
            (&mut tp.tp_diffs)[dnext_idx].df_count[idx] = inserted;
            for i in 0..DB_COUNT {
                if !tp.tp_diffbuf[i].is_null() && i != idx {
                    (&mut tp.tp_diffs)[dnext_idx].df_lnum[i] = match dprev_copy {
                        None => line1,
                        Some((prev_lnum, prev_count)) => {
                            line1 + (prev_lnum[i] + prev_count[i])
                                - (prev_lnum[idx] + prev_count[idx])
                        }
                    };
                    (&mut tp.tp_diffs)[dnext_idx].df_count[i] = deleted;
                }
            }
        }
        if dp_idx >= tp.tp_diffs.len() {
            break;
        }
        let last = (&tp.tp_diffs)[dp_idx].df_lnum[idx] + (&tp.tp_diffs)[dp_idx].df_count[idx] - 1;
        if last >= line1 - 1 {
            if diff_st().diff_busy {
                if (&tp.tp_diffs)[dp_idx].df_lnum[idx] > line2 {
                    (&mut tp.tp_diffs)[dp_idx].df_lnum[idx] += amount_after;
                }
                dprev_idx = Some(dp_idx);
                dp_idx += 1;
                continue;
            }
            if (&tp.tp_diffs)[dp_idx].df_lnum[idx] - Linenr::from(deleted + inserted != 0) > line2 {
                if amount_after == 0 {
                    break;
                }
                (&mut tp.tp_diffs)[dp_idx].df_lnum[idx] += amount_after;
            } else {
                let mut check_unchanged = false;
                if deleted > 0 {
                    let mut n: Linenr;
                    let mut off: Linenr = 0;
                    let next_lnum = tp
                        .tp_diffs
                        .as_slice()
                        .get(dp_idx + 1)
                        .map(|nd| nd.df_lnum[idx]);
                    let dp = &mut (&mut tp.tp_diffs)[dp_idx];
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
                        if !tp.tp_diffbuf[i].is_null() && i != idx {
                            if dp.df_lnum[i] > off {
                                dp.df_lnum[i] -= off;
                            } else {
                                dp.df_lnum[i] = 1;
                            }
                            dp.df_count[i] += n;
                        }
                    }
                } else if (&tp.tp_diffs)[dp_idx].df_lnum[idx] <= line1 {
                    (&mut tp.tp_diffs)[dp_idx].df_count[idx] += inserted;
                    check_unchanged = true;
                } else {
                    (&mut tp.tp_diffs)[dp_idx].df_lnum[idx] += inserted;
                }
                if check_unchanged {
                    diff_check_unchanged(tp, dp_idx);
                }
            }
        }
        if let Some(pi) = dprev_idx
            && !(&tp.tp_diffs)[dp_idx].is_linematched
            && !diff_st().diff_busy
            && (&tp.tp_diffs)[pi].df_lnum[idx] + (&tp.tp_diffs)[pi].df_count[idx]
                == (&tp.tp_diffs)[dp_idx].df_lnum[idx]
        {
            let dp_counts = (&tp.tp_diffs)[dp_idx].df_count;
            for (i, &c) in dp_counts.iter().enumerate() {
                if !tp.tp_diffbuf[i].is_null() {
                    (&mut tp.tp_diffs)[pi].df_count[i] += c;
                }
            }
            diff_free(tp, dp_idx);
        } else {
            dprev_idx = Some(dp_idx);
            dp_idx += 1;
        }
    }
    let mut i_dp = 0;
    while i_dp < tp.tp_diffs.len() {
        let mut i = 0;
        while i < DB_COUNT {
            if !tp.tp_diffbuf[i].is_null() && (&tp.tp_diffs)[i_dp].df_count[i] != 0 {
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
    if ptr::eq(tp, g_st().curtab) {
        g_st().need_diff_redraw = true;
        g_st().diff_need_scrollbind = true;
    }
}
fn diff_alloc_new(tp: &mut TabPageInfo, idx: usize) -> usize {
    let df_id = diff_st().next_diff_id;
    diff_st().next_diff_id += 1;
    diff_st().diff_finger_dp_idx = None;
    tp.tp_diffs.insert(
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
fn diff_free(tp: &mut TabPageInfo, idx: usize) {
    diff_st().diff_finger_dp_idx = None;
    tp.tp_diffs.remove(idx);
}
fn diff_check_unchanged(tp: &mut TabPageInfo, dp_idx: usize) {
    let mut i_org = 0;
    while i_org < DB_COUNT {
        if !tp.tp_diffbuf[i_org].is_null() {
            break;
        }
        i_org += 1;
    }
    if i_org == DB_COUNT {
        return;
    }
    if !diff_check_sanity(tp, &(&tp.tp_diffs)[dp_idx]) {
        return;
    }
    let mut off_org: Linenr = 0;
    let mut off_new: Linenr = 0;
    let mut dir = FORWARD;
    loop {
        while (&tp.tp_diffs)[dp_idx].df_count[i_org] > 0 {
            if dir == BACKWARD {
                off_org = (&tp.tp_diffs)[dp_idx].df_count[i_org] - 1;
            }
            let line_org = crate::memory::cstr_to_cstring(ml_get_buf(
                unsafe { &mut *tp.tp_diffbuf[i_org] },
                (&tp.tp_diffs)[dp_idx].df_lnum[i_org] + off_org,
            ));
            let mut i_new = i_org + 1;
            while i_new < DB_COUNT {
                if tp.tp_diffbuf[i_new].is_null() {
                    i_new += 1;
                    continue;
                }
                if dir == BACKWARD {
                    off_new = (&tp.tp_diffs)[dp_idx].df_count[i_new] - 1;
                }
                if off_new < 0 || off_new >= (&tp.tp_diffs)[dp_idx].df_count[i_new] {
                    break;
                }
                if diff_cmp(
                    line_org.as_ptr(),
                    ml_get_buf(
                        unsafe { &mut *tp.tp_diffbuf[i_new] },
                        (&tp.tp_diffs)[dp_idx].df_lnum[i_new] + off_new,
                    ),
                ) != 0
                {
                    break;
                }
                i_new += 1;
            }
            if i_new != DB_COUNT {
                break;
            }
            for idx in i_org..DB_COUNT {
                if !tp.tp_diffbuf[idx].is_null() {
                    if dir == FORWARD {
                        (&mut tp.tp_diffs)[dp_idx].df_lnum[idx] += 1;
                    }
                    (&mut tp.tp_diffs)[dp_idx].df_count[idx] -= 1;
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
        let buf = tp.tp_diffbuf[i];
        if !buf.is_null()
            && dp.df_lnum[i] + dp.df_count[i] - 1 > unsafe { (*buf).b_ml.ml_line_count }
        {
            return false;
        }
    }
    true
}
pub fn diff_redraw(dofold: bool) {
    let mut wp_other: *mut WinInfoInfo = ptr::null_mut();
    let mut used_max_fill_other = false;
    let mut used_max_fill_curwin = false;
    g_st().need_diff_redraw = false;
    for wp in for_all_windows_ptr() {
        if wp.w_onebuf_opt.wo_diff == 0 || !buf_valid(wp.w_buffer) {
            continue;
        }
        redraw_later(Some(&mut *wp), UPD_SOME_VALID);
        if !std::ptr::eq(wp, curwin_ptr()) {
            wp_other = wp;
        }
        if dofold && foldmethod_is_diff(&mut *wp) {
            fold_update_all(&mut *wp);
        }
        let n = diff_check_fill(wp, wp.w_topline);
        if (!std::ptr::eq(wp, curwin_ptr()) && wp.w_topfill > 0) || n > 0 {
            if wp.w_topfill > n {
                wp.w_topfill = n.max(0);
            } else if n > 0 && n > wp.w_topfill {
                wp.w_topfill = n;
                if std::ptr::eq(wp, curwin_ptr()) {
                    used_max_fill_curwin = true;
                } else if !wp_other.is_null() {
                    used_max_fill_other = true;
                }
            }
            check_topfill(&mut *wp, false);
        }
    }
    if !wp_other.is_null() && curwin_mut().w_onebuf_opt.wo_scb != 0 {
        if used_max_fill_curwin {
            diff_set_topline(unsafe { &mut *wp_other }, curwin_mut());
        } else if used_max_fill_other {
            diff_set_topline(curwin_mut(), unsafe { &mut *wp_other });
        }
    }
}
fn clear_diffin(din: &mut Diffin) {
    if let Some(ref fname) = din.din_fname {
        let _ = os_remove(fname.as_ptr());
    } else {
        din.din_buf = Vec::new();
        din.din_mmfile.ptr = ptr::null_mut();
        din.din_mmfile.size = 0;
    }
}
fn clear_diffout(dout: &mut Diffout) {
    if let Some(ref fname) = dout.dout_fname {
        let _ = os_remove(fname.as_ptr());
    } else {
        dout.dout_ga = Vec::new();
    }
}
fn diff_write_buffer(
    buf: &mut FileBuf,
    m: &mut Mmfile,
    backing: &mut Vec<u8>,
    start: Linenr,
    mut end: Linenr,
) -> bool {
    if end < 0 {
        end = buf.b_ml.ml_line_count;
    }
    if (buf.b_ml.ml_flags & ML_EMPTY) != 0 || end < start {
        *backing = Vec::new();
        m.ptr = ptr::null_mut();
        m.size = 0;
        return true;
    }
    let mut len: usize = 0;
    for lnum in start..=end {
        len += ml_get_buf_len(buf, lnum) as usize + 1;
    }
    *backing = vec![0u8; len];
    let ptr = backing.as_mut_ptr();
    m.ptr = ptr;
    m.size = len as i32;
    len = 0;
    for lnum in start..=end {
        let mut s = ml_get_buf(buf, lnum);
        if (diff_st().diff_flags & DIFF_ICASE) != 0 {
            while !crate::memory::cstr_is_empty(s) {
                let c: i32;
                let mut c_len: i32 = 1;
                let mut cbuf: [u8; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];
                if unsafe { *s } == NL {
                    c = NUL as i32;
                } else {
                    let ch = utf_ptr2char(s);
                    c_len = utf_char2len(ch);
                    c = utf_fold(ch);
                }
                let orig_len = utfc_ptr2len(s) as usize;
                let s_bytes = crate::memory::cstr_slice(s, orig_len);
                if crate::mbyte::utf_char2slice(c, &mut cbuf) != c_len as usize {
                    backing[len..len + orig_len].copy_from_slice(s_bytes);
                } else {
                    let cl = c_len as usize;
                    backing[len..len + cl].copy_from_slice(&cbuf[..cl]);
                    if orig_len > cl {
                        backing[len + cl..len + orig_len].copy_from_slice(&s_bytes[cl..]);
                    }
                }
                s = s.wrapping_add(orig_len);
                len += orig_len;
            }
        } else {
            let s_bytes = crate::memory::cstr_as_bytes(s);
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
fn diff_write(buf: &mut FileBuf, din: &mut Diffin, start: Linenr, mut end: Linenr) -> bool {
    let Some(ref din_fname) = din.din_fname else {
        return diff_write_buffer(buf, &mut din.din_mmfile, &mut din.din_buf, start, end);
    };
    if frames_locked() {
        return false;
    }
    if end < 0 {
        end = buf.b_ml.ml_line_count;
    }
    let save_ml_flags = buf.b_ml.ml_flags;
    let save_ff = buf.b_p_ff;
    buf.b_p_ff = alloc_string_option(c"unix");
    let save_cmod_flags = g_st().cmdmod.cmod_flags != 0;
    g_st().cmdmod.cmod_flags |= CMOD_LOCKMARKS;
    if end < start {
        end = start;
        buf.b_ml.ml_flags |= ML_EMPTY;
    }
    let r = buf_write(
        &raw mut *buf,
        din_fname.as_ptr().cast_mut(),
        ptr::null_mut(),
        start,
        end,
        None,
        false,
        false,
        false,
        true,
    );
    g_st().cmdmod.cmod_flags = i32::from(save_cmod_flags);
    free_string_option(buf.b_p_ff);
    buf.b_p_ff = save_ff;
    buf.b_ml.ml_flags = (buf.b_ml.ml_flags & !ML_EMPTY) | (save_ml_flags & ML_EMPTY);
    r == OK
}
fn diff_try_update(dio: &mut Diffio, idx_orig: usize, eap: &mut Exarg) {
    unsafe {
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
            if eap.forceit != 0 {
                for idx_new in idx_orig..DB_COUNT {
                    let buf = curtab_mut().tp_diffbuf[idx_new];
                    if buf_valid(buf) {
                        let _ = buf_check_timestamp(&mut *buf);
                    }
                }
            }
            let mut num_anchors: usize = usize::MAX;
            let mut anchors: [[Linenr; MAX_DIFF_ANCHORS]; DB_COUNT] =
                [[0; MAX_DIFF_ANCHORS]; DB_COUNT];
            if (diff_st().diff_flags & DIFF_ANCHOR) != 0 {
                for idx in 0..DB_COUNT {
                    if curtab_mut().tp_diffbuf[idx].is_null() {
                        continue;
                    }
                    let mut buf_num_anchors: i32 = 0;
                    if !parse_diffanchors(
                        false,
                        &mut *(curtab_mut().tp_diffbuf[idx]),
                        (&raw mut anchors[idx]).cast(),
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
                    orig_diff = std::mem::take(&mut curtab_mut().tp_diffs);
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
                let mut buf = curtab_mut().tp_diffbuf[idx_orig];
                if !diff_write(&mut *buf, &mut dio.dio_orig, lnum_start, lnum_end) {
                    diff_clear(curtab_mut());
                    break 'theend;
                }
                for (idx_new, anchor_row) in
                    anchors.iter().enumerate().take(DB_COUNT).skip(idx_orig + 1)
                {
                    buf = curtab_mut().tp_diffbuf[idx_new];
                    if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
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
                    if !diff_write(&mut *buf, &mut dio.dio_new, lnum_start, lnum_end) {
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
                    diff_st().diff_finger_dp_idx = None;
                    for dp in &mut curtab_mut().tp_diffs {
                        for (idx, anchor_row) in anchors.iter().enumerate() {
                            if anchor_row[anchor_i - 1] > 0 {
                                dp.df_lnum[idx] += anchor_row[anchor_i - 1] - 1;
                            }
                        }
                    }
                    if !orig_diff.is_empty() {
                        orig_diff.append(&mut curtab_mut().tp_diffs);
                        curtab_mut().tp_diffs = orig_diff;
                    }
                }
            }
        }
    }
}
pub fn diff_internal() -> i32 {
    i32::from(
        (diff_st().diff_flags & DIFF_INTERNAL) != 0
            && crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex)
                == K_CALLBACK_NONE_CONST,
    )
}
pub fn ex_diffupdate(eap: &mut Exarg) {
    if diff_st().diff_busy {
        diff_st().diff_need_update = true;
        return;
    }
    let had_diffs = !curtab_mut().tp_diffs.is_empty();
    diff_clear(curtab_mut());
    curtab_mut().tp_diff_invalid = 0;
    'theend: {
        let mut idx_orig = 0;
        while idx_orig < DB_COUNT {
            if !curtab_mut().tp_diffbuf[idx_orig].is_null() {
                break;
            }
            idx_orig += 1;
        }
        if idx_orig == DB_COUNT {
            break 'theend;
        }
        let mut idx_new = idx_orig + 1;
        while idx_new < DB_COUNT {
            if !curtab_mut().tp_diffbuf[idx_new].is_null() {
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
                din_mmfile: Mmfile {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
                din_buf: Vec::new(),
            },
            dio_new: Diffin {
                din_fname: None,
                din_mmfile: Mmfile {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
                din_buf: Vec::new(),
            },
            dio_diff: Diffout {
                dout_fname: None,
                dout_ga: Vec::new(),
            },
            dio_internal: diff_internal(),
        };
        diff_try_update(&mut diffio, idx_orig, eap);
        curwin_mut().w_valid_cursor.lnum = 0;
    }
    if had_diffs || !curtab_mut().tp_diffs.is_empty() {
        diff_redraw(true);
        window_layout_lock();
        apply_autocmds(
            EVENT_DIFFUPDATED,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        window_layout_unlock();
    }
}
fn check_external_diff(diffio: &mut Diffio) -> bool {
    let mut io_error = false;
    let mut ok: TriState;
    let orig_fname = diffio.dio_orig.din_fname.as_ref().unwrap().as_ptr();
    let new_fname = diffio.dio_new.din_fname.as_ref().unwrap().as_ptr();
    let diff_fname = diffio.dio_diff.dout_fname.as_ref().unwrap().as_ptr();
    loop {
        ok = K_FALSE;
        let mut fd = os_fopen(orig_fname, c"w");
        if fd.is_null() {
            io_error = true;
        } else {
            if unsafe { fwrite(c"line1\n".as_ptr().cast(), 6, 1, fd) } != 1 {
                io_error = true;
            }
            unsafe { fclose(fd) };
            fd = os_fopen(new_fname, c"w");
            if fd.is_null() {
                io_error = true;
            } else {
                if unsafe { fwrite(c"line2\n".as_ptr().cast(), 6, 1, fd) } != 1 {
                    io_error = true;
                }
                unsafe { fclose(fd) };
                fd = if diff_file(diffio) {
                    os_fopen(diff_fname, c"r")
                } else {
                    ptr::null_mut()
                };
                if fd.is_null() {
                    io_error = true;
                } else {
                    let mut linebuf: [c_char; LBUFLEN] = [0; LBUFLEN];
                    loop {
                        if vim_fgets((&raw mut linebuf).cast(), LBUFLEN as i32, fd) {
                            break;
                        }
                        if (crate::memory::cstr_prefix_bytes((&raw const linebuf).cast(), 3)
                            == &b"1c1"[..3])
                            || (crate::memory::cstr_prefix_bytes((&raw const linebuf).cast(), 11)
                                == &b"@@ -1 +1 @@"[..11])
                        {
                            ok = K_TRUE;
                        }
                    }
                    unsafe { fclose(fd) };
                }
                let _ = os_remove(diff_fname);
                let _ = os_remove(new_fname);
            }
            let _ = os_remove(orig_fname);
        }
        if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex) != K_CALLBACK_NONE_CONST {
            break;
        }
        if diff_st().diff_a_works != K_NONE {
            break;
        }
        diff_st().diff_a_works = ok;
        if ok != 0 {
            break;
        }
    }
    if ok == 0 {
        if io_error {
            emsg(gettext(c"E810: Cannot read or write temp files"));
        }
        emsg(gettext(c"E97: Cannot create diffs"));
        diff_st().diff_a_works = K_NONE;
        return false;
    }
    true
}
fn diff_file_internal(diffio: &mut Diffio) -> bool {
    let mut param = Xpparam {
        flags: diff_st().diff_algorithm as u64,
    };
    let emit_cfg = Xdemitconf {
        ctxlen: 0,
        interhunkctxlen: 0,
        flags: 0,
    };
    let mut emit_cb = Xdemitcb::Diff(&raw mut diffio.dio_diff);
    if (diff_st().diff_flags & DIFF_IWHITE) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE_CHANGE;
    }
    if (diff_st().diff_flags & DIFF_IWHITEALL) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE;
    }
    if (diff_st().diff_flags & DIFF_IWHITEEOL) != 0 {
        param.flags |= XDF_IGNORE_WHITESPACE_AT_EOL;
    }
    if (diff_st().diff_flags & DIFF_IBLANK) != 0 {
        param.flags |= XDF_IGNORE_BLANK_LINES;
    }
    if diffio.dio_orig.din_mmfile.size > MAX_XDIFF_SIZE
        || diffio.dio_new.din_mmfile.size > MAX_XDIFF_SIZE
    {
        emsg(gettext(E_PROBLEM_CREATING_INTERNAL_DIFF));
        return false;
    }
    if xdl_diff(
        &mut diffio.dio_orig.din_mmfile,
        &mut diffio.dio_new.din_mmfile,
        &param,
        &emit_cfg,
        &mut emit_cb,
    ) < 0
    {
        emsg(gettext(E_PROBLEM_CREATING_INTERNAL_DIFF));
        return false;
    }
    true
}
fn diff_file(dio: &mut Diffio) -> bool {
    let tmp_orig = dio
        .dio_orig
        .din_fname
        .as_ref()
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let tmp_new = dio
        .dio_new
        .din_fname
        .as_ref()
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let tmp_diff = dio
        .dio_diff
        .dout_fname
        .as_ref()
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_dex) != K_CALLBACK_NONE_CONST {
        eval_diff(tmp_orig, tmp_new, tmp_diff);
        return true;
    }
    if dio.dio_internal != 0 {
        return diff_file_internal(dio);
    }
    let len = crate::memory::cstr_len(tmp_orig)
        + crate::memory::cstr_len(tmp_new)
        + crate::memory::cstr_len(tmp_diff)
        + crate::memory::cstr_len(opt_vars().p_srr)
        + 27;
    let mut cmd_vec: Vec<c_char> = vec![0; len];
    let cmd = cmd_vec.as_mut_ptr();
    if os_env_exists(c"DIFF_OPTIONS", true) {
        let _ = os_unsetenv(c"DIFF_OPTIONS");
    }
    vim_snprintf!(
        cmd,
        len,
        c"diff %s%s%s%s%s%s%s%s %s",
        if diff_st().diff_a_works == K_FALSE {
            c"".as_ptr()
        } else {
            c"-a ".as_ptr()
        },
        c"",
        if (diff_st().diff_flags & DIFF_IWHITE) != 0 {
            c"-b ".as_ptr()
        } else {
            c"".as_ptr()
        },
        if (diff_st().diff_flags & DIFF_IWHITEALL) != 0 {
            c"-w ".as_ptr()
        } else {
            c"".as_ptr()
        },
        if (diff_st().diff_flags & DIFF_IWHITEEOL) != 0 {
            c"-Z ".as_ptr()
        } else {
            c"".as_ptr()
        },
        if (diff_st().diff_flags & DIFF_IBLANK) != 0 {
            c"-B ".as_ptr()
        } else {
            c"".as_ptr()
        },
        if (diff_st().diff_flags & DIFF_ICASE) != 0 {
            c"-i ".as_ptr()
        } else {
            c"".as_ptr()
        },
        tmp_orig,
        tmp_new,
    );
    append_redir(cmd, len, opt_vars().p_srr, tmp_diff);
    crate::autocmd::block_autocmds();
    call_shell(
        cmd,
        K_SHELL_OPT_FILTER | K_SHELL_OPT_SILENT | K_SHELL_OPT_DO_OUT,
        ptr::null_mut(),
    );
    crate::autocmd::unblock_autocmds();
    true
}
pub fn ex_diffpatch(eap: &mut Exarg) {
    let mut buf_vec: Vec<c_char>;
    let old_curwin = curwin_ptr();
    let mut newname_vec: Vec<c_char>;
    let esc_name: Option<CString>;
    #[cfg(unix)]
    let fullname: Option<CString>;
    let tmp_orig_owned = vim_tempname();
    let tmp_new_owned = vim_tempname();
    'theend: {
        let (Some(tmp_orig_cstr), Some(tmp_new_cstr)) = (&tmp_orig_owned, &tmp_new_owned) else {
            break 'theend;
        };
        let tmp_orig = tmp_orig_cstr.as_ptr().cast_mut();
        let tmp_new = tmp_new_cstr.as_ptr().cast_mut();
        if buf_write(
            g_st().curbuf,
            tmp_orig,
            ptr::null_mut(),
            1,
            curbuf_mut().b_ml.ml_line_count,
            None,
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
            fullname = crate::path::FullName_save(eap.arg, false);
            esc_name = Some(vim_strsave_shellescape(
                fullname.as_ref().map_or(eap.arg, |s| s.as_ptr().cast_mut()),
                true,
                true,
            ));
        }
        #[cfg(not(unix))]
        {
            esc_name = Some(vim_strsave_shellescape(eap.arg, true, true));
        }
        let esc_ptr = esc_name.as_ref().map_or(c"".as_ptr(), |s| s.as_ptr());
        let buflen = crate::memory::cstr_len(tmp_orig)
            + crate::memory::cstr_len(esc_ptr)
            + crate::memory::cstr_len(tmp_new)
            + 16;
        buf_vec = vec![0; buflen];
        let buf = buf_vec.as_mut_ptr();
        #[cfg(unix)]
        let mut dirbuf: [c_char; MAXPATHL] = [0; MAXPATHL];
        #[cfg(unix)]
        {
            if !crate::os::fs::os_dirname((&raw mut dirbuf).cast(), MAXPATHL)
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
        if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_pex) != K_CALLBACK_NONE_CONST {
            #[cfg(unix)]
            eval_patch(
                tmp_orig,
                fullname.as_ref().map_or(eap.arg, |s| s.as_ptr().cast_mut()),
                tmp_new,
            );
            #[cfg(not(unix))]
            eval_patch(tmp_orig, eap.arg, tmp_new);
        } else {
            vim_snprintf!(
                buf,
                buflen,
                c"patch -o %s %s < %s",
                tmp_new,
                tmp_orig,
                esc_ptr,
            );
            crate::autocmd::block_autocmds();
            call_shell(buf, K_SHELL_OPT_FILTER, ptr::null_mut());
            crate::autocmd::unblock_autocmds();
        }
        #[cfg(unix)]
        if dirbuf[0] != NUL {
            if crate::os::fs::os_chdir((&raw const dirbuf).cast()) != 0 {
                emsg(gettext(E_PREV_DIR));
            }
            shorten_fnames(1);
        }
        crate::memory::cstr_cpy(buf, tmp_new);
        crate::memory::cstr_cat(buf, c".orig");
        let _ = os_remove(buf);
        crate::memory::cstr_cpy(buf, tmp_new);
        crate::memory::cstr_cat(buf, c".rej");
        let _ = os_remove(buf);
        let mut file_info = FileInfo::new();
        let info_ok = os_fileinfo(tmp_new, &mut file_info);
        let filesize = os_fileinfo_size(&file_info);
        if !info_ok || filesize == 0 {
            emsg(gettext(c"E816: Cannot read patch output"));
        } else {
            let mut newname: *mut c_char = ptr::null_mut();
            if !curbuf_mut().b_fname.is_null() {
                let fname_len = crate::memory::cstr_len(curbuf_mut().b_fname);
                newname_vec = vec![0; fname_len + 5];
                crate::memory::cstr_cpy(newname_vec.as_mut_ptr(), curbuf_mut().b_fname);
                crate::memory::cstr_cat(newname_vec.as_mut_ptr(), c".new");
                newname = newname_vec.as_mut_ptr();
            }
            g_st().cmdmod.cmod_tab = 0;
            if win_split(
                0,
                if (diff_st().diff_flags & DIFF_VERTICAL) != 0 {
                    WSP_VERT
                } else {
                    0
                },
            ) {
                eap.cmdidx = CMD_SPLIT;
                eap.arg = tmp_new;
                do_exedit(eap, old_curwin);
                if curwin_ptr() != old_curwin && win_valid(old_curwin) {
                    diff_win_options(curwin_mut(), true);
                    diff_win_options(unsafe { &mut *old_curwin }, true);
                    if !newname.is_null() {
                        eap.arg = newname;
                        ex_file(eap);
                        if crate::autocmd::augroup_exists(c"filetypedetect") {
                            do_cmdline_cmd(c":doau filetypedetect BufRead");
                        }
                    }
                }
            }
        }
    }
    if let Some(ref tmp_orig) = tmp_orig_owned {
        let _ = os_remove(tmp_orig.as_ptr());
    }
    if let Some(ref tmp_new) = tmp_new_owned {
        let _ = os_remove(tmp_new.as_ptr());
    }
}
pub fn ex_diffsplit(eap: &mut Exarg) {
    let old_curwin = curwin_ptr();
    let mut old_curbuf = Bufref {
        br_buf: ptr::null_mut(),
        br_fnum: 0,
        br_buf_free_count: 0,
    };
    set_bufref(&mut old_curbuf, g_st().curbuf);
    validate_cursor(curwin_mut());
    set_fraction(curwin_mut());
    g_st().cmdmod.cmod_tab = 0;
    if !win_split(
        0,
        if (diff_st().diff_flags & DIFF_VERTICAL) != 0 {
            WSP_VERT
        } else {
            0
        },
    ) {
        return;
    }
    eap.cmdidx = CMD_SPLIT;
    curwin_mut().w_onebuf_opt.wo_diff = 1;
    do_exedit(eap, old_curwin);
    if curwin_ptr() == old_curwin {
        return;
    }
    diff_win_options(curwin_mut(), true);
    if win_valid(old_curwin) {
        diff_win_options(unsafe { &mut *old_curwin }, true);
        if bufref_valid(Some(&old_curbuf)) {
            curwin_mut().w_cursor.lnum =
                diff_get_corresponding_line(old_curbuf.br_buf.cast(), unsafe {
                    (*old_curwin).w_cursor.lnum
                });
        }
    }
    scroll_to_fraction(curwin_mut(), curwin_mut().w_height);
}
pub fn ex_diffthis(_eap: &mut Exarg) {
    diff_win_options(curwin_mut(), true);
}
fn set_diff_option(wp: *mut WinInfoInfo, value: bool) {
    let old_curwin = curwin_ptr();
    set_curwin(wp);
    g_st().curbuf = curwin_mut().w_buffer;
    curbuf_mut().b_ro_locked += 1;
    set_option_value_give_err(K_OPT_DIFF, boolean_obj(value), OPT_LOCAL);
    curbuf_mut().b_ro_locked -= 1;
    set_curwin(old_curwin);
    g_st().curbuf = curwin_mut().w_buffer;
}
pub fn diff_win_options(wp: &mut WinInfoInfo, addbuf: bool) {
    let old_curwin = curwin_ptr();
    set_curwin(&raw mut *wp);
    new_fold_level();
    set_curwin(old_curwin);
    if wp.w_onebuf_opt.wo_diff == 0 {
        wp.w_onebuf_opt.wo_scb_save = wp.w_onebuf_opt.wo_scb;
    }
    wp.w_onebuf_opt.wo_scb = 1;
    if wp.w_onebuf_opt.wo_diff == 0 {
        wp.w_onebuf_opt.wo_crb_save = wp.w_onebuf_opt.wo_crb;
    }
    wp.w_onebuf_opt.wo_crb = 1;
    if (diff_st().diff_flags & DIFF_FOLLOWWRAP) == 0 {
        if wp.w_onebuf_opt.wo_diff == 0 {
            wp.w_onebuf_opt.wo_wrap_save = wp.w_onebuf_opt.wo_wrap;
        }
        wp.w_onebuf_opt.wo_wrap = 0;
        wp.w_skipcol = 0;
    }
    if wp.w_onebuf_opt.wo_diff == 0 {
        if wp.w_onebuf_opt.wo_diff_saved != 0 {
            free_string_option(wp.w_onebuf_opt.wo_fdm_save);
        }
        wp.w_onebuf_opt.wo_fdm_save = alloc_string_option(wp.w_onebuf_opt.wo_fdm);
    }
    set_option_direct_for(
        K_OPT_FOLDMETHOD,
        static_cstr_as_obj(c"diff"),
        OPT_LOCAL,
        0,
        OptTarget::Win(&raw mut *wp),
    );
    if wp.w_onebuf_opt.wo_diff == 0 {
        wp.w_onebuf_opt.wo_fen_save = wp.w_onebuf_opt.wo_fen;
        wp.w_onebuf_opt.wo_fdl_save = wp.w_onebuf_opt.wo_fdl;
        if wp.w_onebuf_opt.wo_diff_saved != 0 {
            free_string_option(wp.w_onebuf_opt.wo_fdc_save);
        }
        wp.w_onebuf_opt.wo_fdc_save = alloc_string_option(wp.w_onebuf_opt.wo_fdc);
    }
    free_string_option(wp.w_onebuf_opt.wo_fdc);
    debug_assert!(g_st().diff_foldcolumn >= 0 && g_st().diff_foldcolumn <= 9);
    let fdc_buf = [b'0' as c_char + (g_st().diff_foldcolumn as c_char), NUL];
    wp.w_onebuf_opt.wo_fdc = alloc_string_option(fdc_buf.as_ptr());
    wp.w_onebuf_opt.wo_fen = 1;
    wp.w_onebuf_opt.wo_fdl = 0;
    fold_update_all(wp);
    changed_window_setting(wp);
    if !crate::strings::vim_haschr(opt_vars().p_sbo, b'h' as i32) {
        do_cmdline_cmd(c"set sbo+=hor");
    }
    wp.w_onebuf_opt.wo_diff_saved = 1;
    set_diff_option((&raw mut *wp), true);
    if addbuf {
        diff_buf_add(wp.w_buffer);
    }
    redraw_later(Some(&mut *wp), UPD_NOT_VALID);
}
pub fn ex_diffoff(eap: &mut Exarg) {
    let mut diffwin = false;
    for wp in for_all_windows_ptr() {
        let cond = if eap.forceit != 0 {
            wp.w_onebuf_opt.wo_diff != 0
        } else {
            std::ptr::eq(wp, curwin_ptr())
        };
        if cond {
            set_diff_option(wp, false);
            if wp.w_onebuf_opt.wo_diff_saved != 0 {
                if wp.w_onebuf_opt.wo_scb != 0 {
                    wp.w_onebuf_opt.wo_scb = wp.w_onebuf_opt.wo_scb_save;
                }
                if wp.w_onebuf_opt.wo_crb != 0 {
                    wp.w_onebuf_opt.wo_crb = wp.w_onebuf_opt.wo_crb_save;
                }
                if (diff_st().diff_flags & DIFF_FOLLOWWRAP) == 0
                    && wp.w_onebuf_opt.wo_wrap == 0
                    && wp.w_onebuf_opt.wo_wrap_save != 0
                {
                    wp.w_onebuf_opt.wo_wrap = 1;
                    wp.w_leftcol = 0;
                }
                free_string_option(wp.w_onebuf_opt.wo_fdm);
                wp.w_onebuf_opt.wo_fdm = alloc_string_option(
                    if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_fdm_save) {
                        wp.w_onebuf_opt.wo_fdm_save
                    } else {
                        c"manual".as_ptr()
                    },
                );
                free_string_option(wp.w_onebuf_opt.wo_fdc);
                wp.w_onebuf_opt.wo_fdc = alloc_string_option(
                    if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_fdc_save) {
                        wp.w_onebuf_opt.wo_fdc_save
                    } else {
                        c"0".as_ptr()
                    },
                );
                if wp.w_onebuf_opt.wo_fdl == 0 {
                    wp.w_onebuf_opt.wo_fdl = wp.w_onebuf_opt.wo_fdl_save;
                }
                if wp.w_onebuf_opt.wo_fen != 0 {
                    wp.w_onebuf_opt.wo_fen = if foldmethod_is_manual(&mut *wp) {
                        0
                    } else {
                        wp.w_onebuf_opt.wo_fen_save
                    };
                }
                fold_update_all(&mut *wp);
            }
            wp.w_topfill = 0;
            changed_window_setting(&mut *wp);
            diff_buf_adjust(&mut *wp);
        }
        diffwin |= wp.w_onebuf_opt.wo_diff != 0;
    }
    if eap.forceit != 0 {
        diff_buf_clear();
    }
    if !diffwin {
        diff_st().diff_need_update = false;
        curtab_mut().tp_diff_invalid = 0;
        curtab_mut().tp_diff_update = 0;
        diff_clear(curtab_mut());
    }
    if !diffwin && crate::strings::vim_haschr(opt_vars().p_sbo, b'h' as i32) {
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
fn extract_hunk(fd: *mut FILE, hunk: &mut Diffhunk, diffstyle: &mut Diffstyle) -> bool {
    loop {
        let mut line: [c_char; LBUFLEN] = [0; LBUFLEN];
        if vim_fgets((&raw mut line).cast(), LBUFLEN as i32, fd) {
            return true;
        }
        if *diffstyle == Diffstyle::DiffNone {
            if ascii_isdigit(line[0]) {
                *diffstyle = Diffstyle::DiffEd;
            } else if (crate::memory::cstr_prefix_bytes((&raw const line).cast(), 3)
                == &b"@@ "[..3])
                || ((crate::memory::cstr_prefix_bytes((&raw const line).cast(), 4)
                    == &b"--- "[..4])
                    && !vim_fgets((&raw mut line).cast(), LBUFLEN as i32, fd)
                    && (crate::memory::cstr_prefix_bytes((&raw const line).cast(), 4)
                        == &b"+++ "[..4])
                    && !vim_fgets((&raw mut line).cast(), LBUFLEN as i32, fd)
                    && (crate::memory::cstr_prefix_bytes((&raw const line).cast(), 3)
                        == &b"@@ "[..3]))
            {
                *diffstyle = Diffstyle::DiffUnified;
            } else {
                continue;
            }
        }
        if *diffstyle == Diffstyle::DiffEd {
            if !ascii_isdigit(line[0]) {
                continue;
            }
            if !parse_diff_ed((&raw mut line).cast(), hunk) {
                continue;
            }
        } else {
            debug_assert!(*diffstyle == Diffstyle::DiffUnified);
            if !(crate::memory::cstr_prefix_bytes((&raw const line).cast(), 3) == &b"@@ "[..3]) {
                continue;
            }
            if !parse_diff_unified((&raw mut line).cast(), hunk) {
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
    while dp_idx < curtab_mut().tp_diffs.len()
        && hunk.lnum_orig
            > (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_orig]
                + (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_orig]
    {
        if *notsetp {
            diff_copy_entry(dp_idx, idx_orig, idx_new);
        }
        dp_idx += 1;
        *notsetp = true;
    }
    if dp_idx < curtab_mut().tp_diffs.len()
        && hunk.lnum_orig
            <= (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_orig]
                + (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_orig]
        && hunk.lnum_orig + hunk.count_orig >= (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_orig]
    {
        let mut dpl_idx = dp_idx;
        while dpl_idx + 1 < curtab_mut().tp_diffs.len() {
            if hunk.lnum_orig + hunk.count_orig
                < (&curtab_mut().tp_diffs)[dpl_idx + 1].df_lnum[idx_orig]
            {
                break;
            }
            dpl_idx += 1;
        }
        let dpl_lnum = (&curtab_mut().tp_diffs)[dpl_idx].df_lnum;
        let dpl_count = (&curtab_mut().tp_diffs)[dpl_idx].df_count;
        let dp = &mut (&mut curtab_mut().tp_diffs)[dp_idx];
        let mut off = dp.df_lnum[idx_orig] - hunk.lnum_orig;
        if off > 0 {
            for i in idx_orig..idx_new {
                if !curtab_mut().tp_diffbuf[i].is_null() {
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
            if !curtab_mut().tp_diffbuf[i].is_null() {
                dp.df_count[i] = dpl_lnum[i] + dpl_count[i] - dp.df_lnum[i] + off;
            }
        }
        if dpl_idx > dp_idx {
            diff_st().diff_finger_dp_idx = None;
            curtab_mut().tp_diffs.drain((dp_idx + 1)..=dpl_idx);
        }
    } else {
        dp_idx = diff_alloc_new(curtab_mut(), dp_idx);
        (&mut curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_orig] = hunk.lnum_orig;
        (&mut curtab_mut().tp_diffs)[dp_idx].df_count[idx_orig] = hunk.count_orig;
        (&mut curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_new] = hunk.lnum_new;
        (&mut curtab_mut().tp_diffs)[dp_idx].df_count[idx_new] = hunk.count_new;
        for i in (idx_orig + 1)..idx_new {
            if !curtab_mut().tp_diffbuf[i].is_null() {
                diff_copy_entry(dp_idx, idx_orig, i);
            }
        }
    }
    *notsetp = false;
    *dp_idx_p = dp_idx;
}
fn diff_read(idx_orig: usize, idx_new: usize, dio: &mut Diffio) {
    let mut fd: *mut FILE = ptr::null_mut();
    let mut line_hunk_idx: i32 = 0;
    let mut dp_idx: usize = 0;
    let dout = &mut dio.dio_diff;
    let mut notset = true;
    let mut diffstyle = Diffstyle::DiffNone;
    if dio.dio_internal == 0 {
        fd = os_fopen(dout.dout_fname.as_ref().unwrap().as_ptr(), c"r");
        if fd.is_null() {
            emsg(gettext(c"E98: Cannot read diff output"));
            return;
        }
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
            extract_hunk(fd, &mut hunk, &mut diffstyle)
        };
        if eof {
            break;
        }
        process_hunk(&mut dp_idx, idx_orig, idx_new, &hunk, &mut notset);
    }
    while dp_idx < curtab_mut().tp_diffs.len() {
        if notset {
            diff_copy_entry(dp_idx, idx_orig, idx_new);
        }
        dp_idx += 1;
        notset = true;
    }
    if !fd.is_null() {
        unsafe { fclose(fd) };
    }
}
fn diff_copy_entry(dp_idx: usize, idx_orig: usize, idx_new: usize) {
    let off = if dp_idx == 0 {
        0
    } else {
        let dprev = &(&curtab_mut().tp_diffs)[dp_idx - 1];
        (dprev.df_lnum[idx_orig] + dprev.df_count[idx_orig])
            - (dprev.df_lnum[idx_new] + dprev.df_count[idx_new])
    };
    let dp = &mut (&mut curtab_mut().tp_diffs)[dp_idx];
    dp.df_lnum[idx_new] = dp.df_lnum[idx_orig] - off;
    dp.df_count[idx_new] = dp.df_count[idx_orig];
}
pub fn diff_clear(tp: &mut TabPageInfo) {
    diff_st().diff_finger_dp_idx = None;
    tp.tp_diffs.clear();
}
pub fn diff_linematch(dp: &DiffBlock) -> bool {
    if (diff_st().diff_flags & DIFF_LINEMATCH) == 0 {
        return false;
    }
    let mut tsize: i32 = 0;
    for i in 0..DB_COUNT {
        if !curtab_mut().tp_diffbuf[i].is_null() {
            if dp.df_count[i] < 0 {
                return false;
            }
            tsize += dp.df_count[i];
        }
    }
    tsize <= diff_st().linematch_lines
}
fn get_max_diff_length(dp: &DiffBlock) -> i32 {
    let mut maxlength: i32 = 0;
    for k in 0..DB_COUNT {
        if !curtab_mut().tp_diffbuf[k].is_null() && dp.df_count[k] > maxlength {
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
    let diffs = &curtab_mut().tp_diffs;
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
    let diffs = &curtab_mut().tp_diffs;
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
    if (diff_st().diff_flags & DIFF_FILLER) != 0 {
        *topfill = max_virt_lines - virtual_lines_passed;
    }
    *topline = curlinenum_to;
}
fn apply_linematch_results(dp_idx: usize, decisions: &[i32]) {
    let mut line_numbers: [Linenr; DB_COUNT] = [0; DB_COUNT];
    let mut outputmap: [usize; DB_COUNT] = [0; DB_COUNT];
    let mut ndiffs: usize = 0;
    for (i, line_num) in line_numbers.iter_mut().enumerate().take(DB_COUNT) {
        if !curtab_mut().tp_diffbuf[i].is_null() {
            *line_num = (&curtab_mut().tp_diffs)[dp_idx].df_lnum[i];
            (&mut curtab_mut().tp_diffs)[dp_idx].df_count[i] = 0;
            outputmap[ndiffs] = i;
            ndiffs += 1;
        }
    }
    let mut dp_s_idx = dp_idx;
    for (i, &dec) in decisions.iter().enumerate() {
        if i != 0 && decisions[i - 1] != dec {
            dp_s_idx = diff_alloc_new(curtab_mut(), dp_s_idx + 1);
            (&mut curtab_mut().tp_diffs)[dp_s_idx].is_linematched = true;
            for (j, &line_num) in line_numbers.iter().enumerate().take(DB_COUNT) {
                if !curtab_mut().tp_diffbuf[j].is_null() {
                    (&mut curtab_mut().tp_diffs)[dp_s_idx].df_lnum[j] = line_num;
                    (&mut curtab_mut().tp_diffs)[dp_s_idx].df_count[j] = 0;
                }
            }
        }
        for j in 0..ndiffs {
            if (dec & (1 << j)) != 0 {
                (&mut curtab_mut().tp_diffs)[dp_s_idx].df_count[outputmap[j]] += 1;
                line_numbers[outputmap[j]] += 1;
            }
        }
    }
    (&mut curtab_mut().tp_diffs)[dp_idx].is_linematched = true;
}
fn run_linematch_algorithm(dp_idx: usize) {
    let mut diffbufs_mm: [Mmfile; DB_COUNT] = [Mmfile {
        ptr: ptr::null_mut(),
        size: 0,
    }; DB_COUNT];
    let mut diffbufs_vec: [Vec<u8>; DB_COUNT] = [const { Vec::new() }; DB_COUNT];
    let mut diff_length: [i32; DB_COUNT] = [0; DB_COUNT];
    let mut ndiffs: usize = 0;
    let dp_lnum = (&curtab_mut().tp_diffs)[dp_idx].df_lnum;
    let dp_count = (&curtab_mut().tp_diffs)[dp_idx].df_count;
    for i in 0..DB_COUNT {
        if !curtab_mut().tp_diffbuf[i].is_null() {
            if dp_count[i] > 0 {
                diff_write_buffer(
                    unsafe { &mut *curtab_mut().tp_diffbuf[i] },
                    &mut diffbufs_mm[ndiffs],
                    &mut diffbufs_vec[ndiffs],
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
    let diffbuf_refs: Vec<&Mmfile> = diffbufs_mm[..ndiffs].iter().collect();
    let iwhite = (diff_st().diff_flags & (DIFF_IWHITEALL | DIFF_IWHITE)) != 0;
    let decisions = linematch_nbuffers(&diffbuf_refs, &diff_length[..ndiffs], iwhite);
    drop(diffbufs_vec);
    apply_linematch_results(dp_idx, &decisions);
}
pub fn diff_check_with_linestatus(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    mut linestatus: Option<&mut i32>,
) -> i32 {
    let buf = wp.w_buffer;
    if let Some(linestatus) = linestatus.as_deref_mut() {
        *linestatus = 0;
    }
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab_mut().tp_diffs.is_empty() || wp.w_onebuf_opt.wo_diff == 0 {
        return 0;
    }
    if lnum < 1 || lnum > unsafe { (*buf).b_ml.ml_line_count } + 1 {
        return 0;
    }
    let idx = diff_buf_idx(buf, curtab_mut());
    if idx == DB_COUNT as i32 {
        return 0;
    }
    let u_idx = idx as usize;
    if has_folding(wp, lnum, None, ptr::null_mut()) || decor_conceal_line(wp, lnum - 1, false) {
        return 0;
    }
    let mut le: Option<usize> = None;
    let mut dp_idx = diff_scan_start(curtab_mut(), idx, lnum);
    while dp_idx < curtab_mut().tp_diffs.len() {
        if (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx] <= lnum {
            le = Some(dp_idx);
        }
        if lnum
            <= (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx]
                + (&curtab_mut().tp_diffs)[dp_idx].df_count[u_idx]
        {
            break;
        }
        dp_idx += 1;
    }
    diff_scan_remember(curtab_mut(), idx, lnum, le);
    if dp_idx >= curtab_mut().tp_diffs.len()
        || lnum < (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx]
    {
        return 0;
    }
    if lnum >= wp.w_topline
        && lnum < wp.w_botline
        && !(&curtab_mut().tp_diffs)[dp_idx].is_linematched
        && diff_linematch(&(&curtab_mut().tp_diffs)[dp_idx])
        && diff_check_sanity(curtab_mut(), &(&curtab_mut().tp_diffs)[dp_idx])
    {
        run_linematch_algorithm(dp_idx);
    }
    let mut num_fill: i32 = 0;
    while lnum
        == (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx]
            + (&curtab_mut().tp_diffs)[dp_idx].df_count[u_idx]
    {
        if (diff_st().diff_flags & DIFF_FILLER) != 0 {
            let maxcount = get_max_diff_length(&(&curtab_mut().tp_diffs)[dp_idx]);
            num_fill += maxcount - (&curtab_mut().tp_diffs)[dp_idx].df_count[u_idx];
        }
        if dp_idx + 1 < curtab_mut().tp_diffs.len()
            && lnum >= (&curtab_mut().tp_diffs)[dp_idx + 1].df_lnum[u_idx]
            && lnum
                <= (&curtab_mut().tp_diffs)[dp_idx + 1].df_lnum[u_idx]
                    + (&curtab_mut().tp_diffs)[dp_idx + 1].df_count[u_idx]
        {
            dp_idx += 1;
        } else {
            break;
        }
    }
    let dp = &(&curtab_mut().tp_diffs)[dp_idx];
    if lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx] {
        let mut zero = false;
        let mut cmp = false;
        for i in 0..DB_COUNT {
            if i != u_idx && !curtab_mut().tp_diffbuf[i].is_null() {
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
                    && !curtab_mut().tp_diffbuf[i].is_null()
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
pub fn diff_check_fill(wp: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    if (diff_st().diff_flags & DIFF_FILLER) == 0 {
        return 0;
    }
    let n = diff_check_with_linestatus(wp, lnum, None);
    n.max(0)
}
fn diff_equal_entry(dp: &DiffBlock, idx1: usize, idx2: usize) -> bool {
    if dp.df_count[idx1] != dp.df_count[idx2] {
        return false;
    }
    if !diff_check_sanity(curtab_mut(), dp) {
        return false;
    }
    for i in 0..dp.df_count[idx1] {
        let line = crate::memory::cstr_to_cstring(ml_get_buf(
            unsafe { &mut *curtab_mut().tp_diffbuf[idx1] },
            dp.df_lnum[idx1] + i,
        ));
        let cmp = diff_cmp(
            line.as_ptr(),
            ml_get_buf(
                unsafe { &mut *curtab_mut().tp_diffbuf[idx2] },
                dp.df_lnum[idx2] + i,
            ),
        );
        if cmp != 0 {
            return false;
        }
    }
    true
}
fn diff_equal_char(
    p1: impl crate::strings::IntoCCharPtr,
    p2: impl crate::strings::IntoCCharPtr,
    len: &mut i32,
) -> bool {
    let p1 = p1.into_c_char_ptr();
    let p2 = p2.into_c_char_ptr();
    {
        let l = utfc_ptr2len(p1);
        if l != utfc_ptr2len(p2) {
            return false;
        }
        if l > 1 {
            if (crate::memory::cstr_prefix_bytes(p1, l as usize)
                != crate::memory::cstr_prefix_bytes(p2, l as usize))
                && ((diff_st().diff_flags & DIFF_ICASE) == 0
                    || utf_fold(utf_ptr2char(p1)) != utf_fold(utf_ptr2char(p2)))
            {
                return false;
            }
            *len = l;
        } else {
            if unsafe { *p1 } != unsafe { *p2 }
                && ((diff_st().diff_flags & DIFF_ICASE) == 0
                    || !(crate::memory::cstr_first_byte(p1))
                        .eq_ignore_ascii_case(&(crate::memory::cstr_first_byte(p2))))
            {
                return false;
            }
            *len = 1;
        }
        true
    }
}
fn diff_cmp(s1: impl crate::strings::IntoCCharPtr, s2: impl crate::strings::IntoCCharPtr) -> i32 {
    let s1 = s1.into_c_char_ptr();
    let s2 = s2.into_c_char_ptr();
    {
        if (diff_st().diff_flags & DIFF_IBLANK) != 0
            && (crate::memory::cstr_is_empty(skipwhite(s1))
                || crate::memory::cstr_is_empty(skipwhite(s2)))
        {
            return 0;
        }
        if (diff_st().diff_flags & (DIFF_ICASE | ALL_WHITE_DIFF)) == 0 {
            return crate::memory::cstr_as_bytes(s1).cmp(crate::memory::cstr_as_bytes(s2)) as i32;
        }
        if (diff_st().diff_flags & DIFF_ICASE) != 0 && (diff_st().diff_flags & ALL_WHITE_DIFF) == 0
        {
            return mb_stricmp(s1, s2);
        }
        let mut p1: *const c_char = s1;
        let mut p2: *const c_char = s2;
        while !crate::memory::cstr_is_empty(p1) && !crate::memory::cstr_is_empty(p2) {
            if ((diff_st().diff_flags & DIFF_IWHITE) != 0
                && ascii_iswhite(crate::memory::cstr_first_char(p1))
                && ascii_iswhite(crate::memory::cstr_first_char(p2)))
                || ((diff_st().diff_flags & DIFF_IWHITEALL) != 0
                    && (ascii_iswhite(crate::memory::cstr_first_char(p1))
                        || ascii_iswhite(crate::memory::cstr_first_char(p2))))
            {
                p1 = skipwhite(p1);
                p2 = skipwhite(p2);
            } else {
                let mut l: i32 = 0;
                if !diff_equal_char(p1, p2, &mut l) {
                    break;
                }
                p1 = p1.wrapping_add(l as usize);
                p2 = p2.wrapping_add(l as usize);
            }
        }
        p1 = skipwhite(p1);
        p2 = skipwhite(p2);
        if !crate::memory::cstr_is_empty(p1) || !crate::memory::cstr_is_empty(p2) {
            return 1;
        }
        0
    }
}
pub fn diff_set_topline(fromwin: &mut WinInfoInfo, towin: &mut WinInfoInfo) {
    let frombuf = fromwin.w_buffer;
    let fromidx = diff_buf_idx(frombuf, curtab_mut());
    if fromidx == DB_COUNT as i32 {
        return;
    }
    let u_fromidx = fromidx as usize;
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    let lnum = fromwin.w_topline;
    towin.w_topfill = 0;
    let dp_opt = curtab_mut()
        .tp_diffs
        .iter()
        .find(|dp| lnum <= dp.df_lnum[u_fromidx] + dp.df_count[u_fromidx]);
    if let Some(dp) = dp_opt {
        let toidx = diff_buf_idx(towin.w_buffer, curtab_mut());
        if toidx == DB_COUNT as i32 {
            return;
        }
        let u_toidx = toidx as usize;
        towin.w_topline = lnum + (dp.df_lnum[u_toidx] - dp.df_lnum[u_fromidx]);
        if lnum >= dp.df_lnum[u_fromidx] {
            calculate_topfill_and_topline(
                u_fromidx,
                u_toidx,
                fromwin.w_topline,
                fromwin.w_topfill,
                &mut towin.w_topfill,
                &mut towin.w_topline,
            );
        }
    } else {
        towin.w_topline =
            towin.buf_mut().b_ml.ml_line_count - (unsafe { (*frombuf).b_ml.ml_line_count } - lnum);
    }
    towin.w_botfill = false;
    if towin.w_topline > towin.buf_mut().b_ml.ml_line_count {
        towin.w_topline = towin.buf_mut().b_ml.ml_line_count;
        towin.w_botfill = true;
    }
    if towin.w_topline < 1 {
        towin.w_topline = 1;
        towin.w_topfill = 0;
    }
    invalidate_botline_win(&mut *towin);
    changed_line_abv_curs_win(&mut *towin);
    check_topfill(towin, false);
    has_folding(
        &raw mut *towin,
        towin.w_topline,
        Some(&mut towin.w_topline),
        ptr::null_mut(),
    );
}
fn parse_diffanchors(
    check_only: bool,
    buf: &mut FileBuf,
    anchors: *mut Linenr,
    num_anchors: Option<&mut i32>,
) -> bool {
    let mut dia = if crate::memory::cstr_is_empty(buf.b_p_dia) {
        opt_vars().p_dia
    } else {
        buf.b_p_dia
    };
    let orig_curbuf = g_st().curbuf;
    let orig_curwin = curwin_ptr();
    let bufwin: *mut WinInfoInfo = if check_only {
        curwin_ptr()
    } else {
        let mut bw = g_st().firstwin;
        while !bw.is_null() {
            if ptr::eq(unsafe { (*bw).w_buffer }, buf) && unsafe { (*bw).w_onebuf_opt.wo_diff } != 0
            {
                break;
            }
            bw = unsafe { (*bw).w_next };
        }
        if bw.is_null() && !crate::memory::cstr_is_empty(dia) {
            emsg(gettext(E_DIFF_ANCHORS_WITH_HIDDEN_WINDOWS));
            return false;
        }
        bw
    };
    let mut i: usize = 0;
    while i < MAX_DIFF_ANCHORS && !crate::memory::cstr_is_empty(dia) {
        if crate::memory::cstr_first_byte(dia) == b',' {
            return false;
        }
        g_st().curbuf = buf;
        set_curwin(bufwin);
        let mut errormsg: *const c_char = ptr::null();
        let lnum = get_address(
            ptr::null_mut(),
            &raw mut dia,
            ADDR_LINES,
            check_only,
            true,
            0,
            1,
            &mut errormsg,
        );
        g_st().curbuf = orig_curbuf;
        set_curwin(orig_curwin);
        if !errormsg.is_null() {
            emsg(errormsg);
        }
        if dia.is_null() {
            return false;
        }
        if crate::memory::cstr_first_byte(dia) != b',' && !crate::memory::cstr_is_empty(dia) {
            return false;
        }
        if !check_only && (lnum == MAXLNUM || lnum <= 0 || lnum > buf.b_ml.ml_line_count + 1) {
            emsg(gettext(E_INVRANGE));
            return false;
        }
        if !anchors.is_null() {
            unsafe {
                *anchors.wrapping_add(i) = lnum;
            }
        }
        if crate::memory::cstr_first_byte(dia) == b',' {
            dia = dia.wrapping_add(1);
        }
        i += 1;
    }
    if i == MAX_DIFF_ANCHORS && !crate::memory::cstr_is_empty(dia) {
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
    let result = parse_diffanchors(true, curbuf_mut(), ptr::null_mut(), None);
    if result && (diff_st().diff_flags & DIFF_ANCHOR) != 0 {
        for tp in for_all_tabpages_mut() {
            if !buflocal {
                tp.tp_diff_invalid = 1;
            } else {
                for idx in 0..DB_COUNT {
                    if tp.tp_diffbuf[idx] == g_st().curbuf {
                        tp.tp_diff_invalid = 1;
                        break;
                    }
                }
            }
        }
    }
    result
}
pub fn diffopt_changed() -> bool {
    let v = unsafe {
        &*opt_keyset(opt_vars().p_dip, K_OPT_DIFFOPT, ptr::null_mut()).cast::<OptKeyDict_dip>()
    };
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
        let inline_ptr = (&raw const v.inline_).cast::<c_char>();
        if crate::memory::cstr_as_opt_bytes(inline_ptr) == Some(b"simple") {
            flags |= DIFF_INLINE_SIMPLE;
        } else if crate::memory::cstr_as_opt_bytes(inline_ptr) == Some(b"char") {
            flags |= DIFF_INLINE_CHAR;
        } else if crate::memory::cstr_as_opt_bytes(inline_ptr) == Some(b"word") {
            flags |= DIFF_INLINE_WORD;
        } else {
            flags |= DIFF_INLINE_NONE;
        }
    }
    let mut algorithm = if v.indent_heuristic {
        XDF_INDENT_HEURISTIC
    } else {
        0
    };
    if has_dip_key(v, KEYSET_OPTIDX_DIP_ALGORITHM) {
        let algo_ptr = (&raw const v.algorithm).cast::<c_char>();
        if crate::memory::cstr_as_opt_bytes(algo_ptr) == Some(b"minimal") {
            algorithm |= XDF_NEED_MINIMAL;
        } else if crate::memory::cstr_as_opt_bytes(algo_ptr) == Some(b"patience") {
            algorithm |= XDF_PATIENCE_DIFF;
        } else if crate::memory::cstr_as_opt_bytes(algo_ptr) == Some(b"histogram") {
            algorithm |= XDF_HISTOGRAM_DIFF;
        }
    }
    let mut ret = true;
    if (flags & DIFF_HORIZONTAL) != 0 && (flags & DIFF_VERTICAL) != 0 {
        ret = false;
    } else {
        if diff_st().diff_flags != flags || diff_st().diff_algorithm != algorithm {
            for tp in for_all_tabpages_mut() {
                tp.tp_diff_invalid = 1;
            }
        }
        let context = if has_dip_key(v, KEYSET_OPTIDX_DIP_CONTEXT) {
            v.context as i32
        } else {
            6
        };
        diff_st().diff_flags = flags;
        g_st().diff_context = if context == 0 { 1 } else { context };
        diff_st().linematch_lines = if has_dip_key(v, KEYSET_OPTIDX_DIP_LINEMATCH) {
            v.linematch as i32
        } else {
            0
        };
        g_st().diff_foldcolumn = if has_dip_key(v, KEYSET_OPTIDX_DIP_FOLDCOLUMN) {
            v.foldcolumn as i32
        } else {
            2
        };
        diff_st().diff_algorithm = algorithm;
        diff_redraw(true);
        check_scrollbind(0, 0);
    }
    ret
}
pub fn diffopt_horizontal() -> bool {
    (diff_st().diff_flags & DIFF_HORIZONTAL) != 0
}
pub fn diffopt_hiddenoff() -> bool {
    (diff_st().diff_flags & DIFF_HIDDEN_OFF) != 0
}
pub fn diffopt_closeoff() -> bool {
    (diff_st().diff_flags & DIFF_CLOSE_OFF) != 0
}
pub fn diffopt_filler() -> bool {
    (diff_st().diff_flags & DIFF_FILLER) != 0
}
pub fn diff_update_line(lnum: Linenr) {
    if (diff_st().diff_flags & ALL_INLINE_DIFF) == 0 {
        return;
    }
    let idx = diff_buf_idx(g_st().curbuf, curtab_mut());
    if idx == DB_COUNT as i32 {
        return;
    }
    let u_idx = idx as usize;
    if let Some(dp) = curtab_mut()
        .tp_diffs
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
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    dp: &DiffBlock,
    idx: usize,
    startp: &mut i32,
    endp: &mut i32,
) -> bool {
    let line_org_owned = if (diff_st().diff_flags & DIFF_INLINE_NONE) != 0 {
        None
    } else {
        Some(crate::memory::cstr_to_cstring(ml_get_buf(
            wp.buf_mut(),
            lnum,
        )))
    };
    let line_org = line_org_owned
        .as_ref()
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
    let mut added = true;
    let off = lnum - dp.df_lnum[idx];
    for i in 0..DB_COUNT {
        if !curtab_mut().tp_diffbuf[i].is_null() && i != idx {
            if off >= dp.df_count[i] {
                continue;
            }
            added = false;
            if (diff_st().diff_flags & DIFF_INLINE_NONE) != 0 {
                break;
            }
            let line_new = ml_get_buf(
                unsafe { &mut *curtab_mut().tp_diffbuf[i] },
                dp.df_lnum[i] + off,
            );
            let mut si_org: i32 = 0;
            let mut si_new: i32 = 0;
            while crate::memory::cstr_byte_at(line_org, si_org as usize) != 0 {
                if ((diff_st().diff_flags & DIFF_IWHITE) != 0
                    && ascii_iswhite(crate::memory::cstr_char_at(line_org, si_org as usize))
                    && ascii_iswhite(crate::memory::cstr_char_at(line_new, si_new as usize)))
                    || ((diff_st().diff_flags & DIFF_IWHITEALL) != 0
                        && (ascii_iswhite(crate::memory::cstr_char_at(line_org, si_org as usize))
                            || ascii_iswhite(crate::memory::cstr_char_at(
                                line_new,
                                si_new as usize,
                            ))))
                {
                    si_org = crate::memory::ptr_offset_from(
                        skipwhite(line_org.wrapping_add(si_org as usize)),
                        line_org,
                    ) as i32;
                    si_new = crate::memory::ptr_offset_from(
                        skipwhite(line_new.wrapping_add(si_new as usize)),
                        line_new,
                    ) as i32;
                } else {
                    let mut l: i32 = 0;
                    if !diff_equal_char(
                        line_org.wrapping_add(si_org as usize),
                        line_new.wrapping_add(si_new as usize),
                        &mut l,
                    ) {
                        break;
                    }
                    si_org += l;
                    si_new += l;
                }
            }
            si_org -= utf_head_off(line_org, line_org.wrapping_add(si_org as usize));
            si_new -= utf_head_off(line_new, line_new.wrapping_add(si_new as usize));
            *startp = (*startp).min(si_org);
            if crate::memory::cstr_byte_at(line_org, si_org as usize) != 0
                || crate::memory::cstr_byte_at(line_new, si_new as usize) != 0
            {
                let mut ei_org = crate::memory::cstr_len(line_org) as i32;
                let mut ei_new = crate::memory::cstr_len(line_new) as i32;
                while ei_org >= *startp && ei_new >= si_new && ei_org >= 0 && ei_new >= 0 {
                    if ((diff_st().diff_flags & DIFF_IWHITE) != 0
                        && ascii_iswhite(crate::memory::cstr_char_at(line_org, ei_org as usize))
                        && ascii_iswhite(crate::memory::cstr_char_at(line_new, ei_new as usize)))
                        || ((diff_st().diff_flags & DIFF_IWHITEALL) != 0
                            && (ascii_iswhite(crate::memory::cstr_char_at(
                                line_org,
                                ei_org as usize,
                            )) || ascii_iswhite(crate::memory::cstr_char_at(
                                line_new,
                                ei_new as usize,
                            ))))
                    {
                        while ei_org >= *startp
                            && ascii_iswhite(crate::memory::cstr_char_at(line_org, ei_org as usize))
                        {
                            ei_org -= 1;
                        }
                        while ei_new >= si_new
                            && ascii_iswhite(crate::memory::cstr_char_at(line_new, ei_new as usize))
                        {
                            ei_new -= 1;
                        }
                    } else {
                        let mut p1: *const c_char = line_org.wrapping_add(ei_org as usize);
                        let mut p2: *const c_char = line_new.wrapping_add(ei_new as usize);
                        p1 = p1.wrapping_sub(utf_head_off(line_org, p1) as usize);
                        p2 = p2.wrapping_sub(utf_head_off(line_new, p2) as usize);
                        let mut l: i32 = 0;
                        if !diff_equal_char(p1, p2, &mut l) {
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
        while di + 1 < curtab_mut().tp_diffs.len() {
            let lm = &linemap[idx1];
            let lm_len = lm.len() as i32;
            if (&curtab_mut().tp_diffs)[di].df_lnum[idx1]
                + (&curtab_mut().tp_diffs)[di].df_count[idx1]
                > lm_len
                || (&curtab_mut().tp_diffs)[di + 1].df_lnum[idx1] > lm_len
            {
                di += 1;
                continue;
            }
            let entry1 = &lm[((&curtab_mut().tp_diffs)[di].df_lnum[idx1]
                + (&curtab_mut().tp_diffs)[di].df_count[idx1]
                - 1) as usize];
            let entry2 = &lm[((&curtab_mut().tp_diffs)[di + 1].df_lnum[idx1] - 1) as usize];
            if entry1.lineoff != entry2.lineoff {
                di += 1;
                continue;
            }
            let gap = (&curtab_mut().tp_diffs)[di + 1].df_lnum[idx1]
                - ((&curtab_mut().tp_diffs)[di].df_lnum[idx1]
                    + (&curtab_mut().tp_diffs)[di].df_count[idx1]);
            if gap <= 3 {
                let mut max_df_count: Linenr = 0;
                for i in 0..DB_COUNT {
                    max_df_count = max_df_count.max(
                        (&curtab_mut().tp_diffs)[di].df_count[i]
                            + (&curtab_mut().tp_diffs)[di + 1].df_count[i],
                    );
                }
                if max_df_count >= gap * 4 {
                    let next_lnum = (&curtab_mut().tp_diffs)[di + 1].df_lnum;
                    let next_count = (&curtab_mut().tp_diffs)[di + 1].df_count;
                    for i in 0..DB_COUNT {
                        (&mut curtab_mut().tp_diffs)[di].df_count[i] =
                            next_lnum[i] + next_count[i] - (&curtab_mut().tp_diffs)[di].df_lnum[i];
                    }
                    diff_free(curtab_mut(), di + 1);
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
    unsafe {
        let mut pass: i32 = 1;
        loop {
            let mut di: usize = 0;
            while di + 1 < curtab_mut().tp_diffs.len() {
                let idx_entry1 = (&curtab_mut().tp_diffs)[di].df_lnum[idx1]
                    + (&curtab_mut().tp_diffs)[di].df_count[idx1]
                    - 2;
                let idx_entry2 = (&curtab_mut().tp_diffs)[di + 1].df_lnum[idx1] - 1;
                let lm1 = &linemap[idx1];
                let lm1_len = lm1.len() as i32;
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
                if gap_size <= 0 || gap_size > DIFF_WORD_GAP {
                    di += 1;
                    continue;
                }
                let line = ml_get_buf(
                    &mut *curtab_mut().tp_diffbuf[idx1],
                    start_lnum + entry1.lineoff,
                );
                let gap_text = line.wrapping_add(gap_start as usize);
                let mut only_non_word = true;
                let mut has_content = false;
                let mut i: i32 = 0;
                while i < gap_size && crate::memory::cstr_byte_at(gap_text, i as usize) != 0 {
                    has_content = true;
                    let char_class = mb_get_class_tab(
                        gap_text.wrapping_add(i as usize),
                        (&raw const (*curtab_mut().tp_diffbuf[idx1]).b_chartab).cast(),
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
                        if !curtab_mut().tp_diffbuf[buf_i].is_null() {
                            for k in 0..(&curtab_mut().tp_diffs)[di].df_count[buf_i] {
                                let idx = (&curtab_mut().tp_diffs)[di].df_lnum[buf_i] + k - 1;
                                if let Some(entry) = lmi.get(idx as usize) {
                                    total_change_bytes += entry.num_bytes as i64;
                                }
                            }
                            for k in 0..(&curtab_mut().tp_diffs)[di + 1].df_count[buf_i] {
                                let idx = (&curtab_mut().tp_diffs)[di + 1].df_lnum[buf_i] + k - 1;
                                if let Some(entry) = lmi.get(idx as usize) {
                                    total_change_bytes += entry.num_bytes as i64;
                                }
                            }
                        }
                    }
                    if total_change_bytes >= (gap_size * 2) as i64 {
                        let next_lnum = (&curtab_mut().tp_diffs)[di + 1].df_lnum;
                        let next_count = (&curtab_mut().tp_diffs)[di + 1].df_count;
                        for buf_i in 0..DB_COUNT {
                            if !curtab_mut().tp_diffbuf[buf_i].is_null() {
                                (&mut curtab_mut().tp_diffs)[di].df_count[buf_i] = next_lnum[buf_i]
                                    + next_count[buf_i]
                                    - (&curtab_mut().tp_diffs)[di].df_lnum[buf_i];
                            }
                        }
                        diff_free(curtab_mut(), di + 1);
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
fn diff_find_change_inline_diff(dp_idx: usize) {
    unsafe {
        let save_diff_algorithm = diff_st().diff_algorithm;
        let mut dio = Diffio {
            dio_orig: Diffin {
                din_fname: None,
                din_mmfile: Mmfile {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
                din_buf: Vec::new(),
            },
            dio_new: Diffin {
                din_fname: None,
                din_mmfile: Mmfile {
                    ptr: ptr::null_mut(),
                    size: 0,
                },
                din_buf: Vec::new(),
            },
            dio_diff: Diffout {
                dout_fname: None,
                dout_ga: Vec::new(),
            },
            dio_internal: 1,
        };
        diff_st().diff_algorithm |= XDF_INDENT_HEURISTIC;
        let dp_lnum = (&curtab_mut().tp_diffs)[dp_idx].df_lnum;
        let dp_count = (&curtab_mut().tp_diffs)[dp_idx].df_count;
        let orig_diff = std::mem::take(&mut curtab_mut().tp_diffs);
        let orig_diffbuf = curtab_mut().tp_diffbuf;
        let mut linemap: [Vec<LinemapEntry>; DB_COUNT] = std::array::from_fn(|_| Vec::new());
        let mut file1_str: Vec<u8> = Vec::new();
        let mut file2_str: Vec<u8> = Vec::new();
        let mut changes: Vec<DifflineChange> = Vec::new();
        'done: {
            let mut file1_idx: isize = -1;
            for (i, lm) in linemap.iter_mut().enumerate().take(DB_COUNT) {
                dio.dio_diff.dout_ga.clear();
                let buf = curtab_mut().tp_diffbuf[i];
                if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
                    continue;
                }
                if dp_count[i] == 0 {
                    curtab_mut().tp_diffbuf[i] = ptr::null_mut();
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
                let mut numlines: Linenr = 0;
                curstr.clear();
                for off in 0..dp_count[i] {
                    let curline = ml_get_buf(&mut *curtab_mut().tp_diffbuf[i], dp_lnum[i] + off);
                    let mut in_keyword = false;
                    let mut last_white = false;
                    let mut eol_ga_len: i32 = -1;
                    let mut eol_linemap_len: i32 = -1;
                    let mut eol_numlines: Linenr = -1;
                    let mut s = curline;
                    while !crate::memory::cstr_is_empty(s) {
                        let mut new_in_keyword = false;
                        if (diff_st().diff_flags & DIFF_INLINE_WORD) != 0 {
                            new_in_keyword = mb_get_class_tab(
                                s,
                                (&raw const (*curtab_mut().tp_diffbuf[file1_idx as usize])
                                    .b_chartab)
                                    .cast(),
                            ) == 2;
                        }
                        if in_keyword && !new_in_keyword {
                            curstr.push(NL as u8);
                            numlines += 1;
                        }
                        if ascii_iswhite(crate::memory::cstr_first_char(s)) {
                            if (diff_st().diff_flags & DIFF_IWHITEALL) != 0 {
                                in_keyword = false;
                                s = skipwhite(s);
                                continue;
                            } else if ((diff_st().diff_flags & DIFF_IWHITEEOL) != 0
                                || (diff_st().diff_flags & DIFF_IWHITE) != 0)
                                && !last_white
                            {
                                eol_ga_len = curstr.len() as i32;
                                eol_linemap_len = lm.len() as i32;
                                eol_numlines = numlines;
                                last_white = true;
                            }
                        } else if (diff_st().diff_flags & DIFF_IWHITEEOL) != 0
                            || (diff_st().diff_flags & DIFF_IWHITE) != 0
                        {
                            last_white = false;
                            eol_ga_len = -1;
                            eol_linemap_len = -1;
                            eol_numlines = -1;
                        }
                        let mut char_len: i32 = 1;
                        if *s == NL {
                            curstr.push(NUL as u8);
                        } else {
                            char_len = utfc_ptr2len(s);
                            if ascii_iswhite(crate::memory::cstr_first_char(s))
                                && (diff_st().diff_flags & DIFF_IWHITE) != 0
                            {
                                char_len = crate::memory::ptr_offset_from(skipwhite(s), s) as i32;
                            }
                            if (diff_st().diff_flags & DIFF_ICASE) != 0 {
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
                                        s.wrapping_add(c_len as usize).cast::<u8>(),
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
                            lm.push(LinemapEntry {
                                byte_start: crate::memory::ptr_offset_from(s, curline) as Colnr,
                                num_bytes: char_len,
                                lineoff: off,
                            });
                        } else if let Some(last) = lm.last_mut() {
                            last.num_bytes += char_len;
                        }
                        in_keyword = new_in_keyword;
                        s = s.wrapping_add(char_len as usize);
                    }
                    if in_keyword {
                        curstr.push(NL as u8);
                        numlines += 1;
                    }
                    if ((diff_st().diff_flags & DIFF_IWHITEEOL) != 0
                        || (diff_st().diff_flags & DIFF_IWHITE) != 0)
                        && eol_ga_len != -1
                    {
                        curstr.truncate(eol_ga_len as usize);
                        lm.truncate(eol_linemap_len as usize);
                        numlines = eol_numlines;
                    }
                    if (diff_st().diff_flags & DIFF_IWHITEALL) == 0 {
                        curstr.push(NL as u8);
                        let _ = numlines;
                        lm.push(LinemapEntry {
                            byte_start: crate::memory::ptr_offset_from(s, curline) as Colnr,
                            num_bytes: size_of::<i32>() as Colnr,
                            lineoff: off,
                        });
                    }
                }
                let ptr = if curstr.is_empty() {
                    ptr::null_mut()
                } else {
                    curstr.as_mut_ptr()
                };
                if file1_idx != i as isize {
                    dio.dio_new.din_mmfile.ptr = ptr;
                    dio.dio_new.din_mmfile.size = curstr.len() as i32;
                } else {
                    dio.dio_orig.din_mmfile.ptr = ptr;
                    dio.dio_orig.din_mmfile.size = curstr.len() as i32;
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
            if (diff_st().diff_flags & DIFF_INLINE_WORD) != 0 && file1_idx != -1 {
                diff_refine_inline_word_highlight(
                    &linemap,
                    file1_idx as usize,
                    dp_lnum[file1_idx as usize],
                );
            } else if (diff_st().diff_flags & DIFF_INLINE_CHAR) != 0 && file1_idx != -1 {
                diff_refine_inline_char_highlight(&linemap, file1_idx as usize);
            }
            for new_diff in &curtab_mut().tp_diffs {
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
        diff_st().diff_algorithm = save_diff_algorithm;
        diff_clear(curtab_mut());
        curtab_mut().tp_diffs = orig_diff;
        curtab_mut().tp_diffbuf = orig_diffbuf;
        (&mut curtab_mut().tp_diffs)[dp_idx].df_changes = changes;
        (&mut curtab_mut().tp_diffs)[dp_idx].has_changes = true;
        clear_diffout(&mut dio.dio_diff);
    }
}
pub fn diff_find_change(wp: &mut WinInfoInfo, lnum: Linenr, diffline: &mut Diffline) -> bool {
    let idx = diff_buf_idx(wp.w_buffer, curtab_mut());
    if idx == DB_COUNT as i32 {
        return false;
    }
    let u_idx = idx as usize;
    let Some(dp_idx) = curtab_mut()
        .tp_diffs
        .iter()
        .position(|dp| lnum < dp.df_lnum[u_idx] + dp.df_count[u_idx])
    else {
        return false;
    };
    if !diff_check_sanity(curtab_mut(), &(&curtab_mut().tp_diffs)[dp_idx]) {
        return false;
    }
    let off = lnum - (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx];
    if (diff_st().diff_flags & ALL_INLINE_DIFF) == 0 {
        let mut change_start: i32 = MAXCOL;
        let mut change_end: i32 = -1;
        let ret = diff_find_change_simple(
            wp,
            lnum,
            &(&curtab_mut().tp_diffs)[dp_idx],
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
        diffline.lineoff = lnum - (&curtab_mut().tp_diffs)[dp_idx].df_lnum[u_idx];
        return ret;
    }
    if !(&curtab_mut().tp_diffs)[dp_idx].has_changes {
        diff_find_change_inline_diff(dp_idx);
    }
    let dp = &mut (&mut curtab_mut().tp_diffs)[dp_idx];
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
            if curtab_mut().tp_diffbuf[i].is_null() {
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
pub fn diff_infold(wp: &mut WinInfoInfo, lnum: Linenr) -> bool {
    if wp.w_onebuf_opt.wo_diff == 0 {
        return false;
    }
    let mut idx: i32 = -1;
    let mut other = false;
    for i in 0..DB_COUNT {
        if curtab_mut().tp_diffbuf[i] == wp.w_buffer {
            idx = i as i32;
        } else if !curtab_mut().tp_diffbuf[i].is_null() {
            other = true;
        }
    }
    if idx == -1 || !other {
        return false;
    }
    let u_idx = idx as usize;
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab_mut().tp_diffs.is_empty() {
        return true;
    }
    let mut le: Option<usize> = None;
    let mut result = true;
    let mut dp_idx = diff_scan_start(curtab_mut(), idx, lnum);
    while dp_idx < curtab_mut().tp_diffs.len() {
        let dp = &(&curtab_mut().tp_diffs)[dp_idx];
        if dp.df_lnum[u_idx] <= lnum {
            le = Some(dp_idx);
        }
        if dp.df_lnum[u_idx] - g_st().diff_context > lnum {
            break;
        }
        if dp.df_lnum[u_idx] + dp.df_count[u_idx] + g_st().diff_context > lnum {
            result = false;
            break;
        }
        dp_idx += 1;
    }
    diff_scan_remember(curtab_mut(), idx, lnum, le);
    result
}
pub fn nv_diffgetput(put: bool, count: usize) {
    if bt_prompt(Some(crate::globals::curbuf_mut())) {
        vim_beep(K_OPT_BO_FLAG_OPERATOR);
        return;
    }
    let mut ea = Exarg::new();
    let mut buf: [c_char; 30] = [0; 30];
    if count == 0 {
        ea.arg = c"".as_ptr().cast_mut();
    } else {
        vim_snprintf!((&raw mut buf).cast(), buf.len(), c"%zu", count);
        ea.arg = (&raw mut buf).cast();
    }
    ea.cmdidx = if put { CMD_DIFFPUT } else { CMD_DIFFGET };
    ea.addr_count = 0;
    ea.line1 = curwin_mut().w_cursor.lnum;
    ea.line2 = curwin_mut().w_cursor.lnum;
    ex_diffgetput(&mut ea);
}
fn valid_diff(df_id: Option<u64>) -> Option<usize> {
    let id = df_id?;
    curtab_mut().tp_diffs.iter().position(|dp| dp.df_id == id)
}
pub fn ex_diffgetput(eap: &mut Exarg) {
    let idx_other: usize;
    let idx_cur_i = diff_buf_idx(g_st().curbuf, curtab_mut());
    if idx_cur_i == DB_COUNT as i32 {
        emsg(gettext(c"E99: Current buffer is not in diff mode"));
        return;
    }
    let idx_cur = idx_cur_i as usize;
    if crate::memory::cstr_is_empty(eap.arg) {
        let mut found_not_ma = false;
        let mut idx = 0;
        while idx < DB_COUNT {
            if curtab_mut().tp_diffbuf[idx] != g_st().curbuf
                && !curtab_mut().tp_diffbuf[idx].is_null()
            {
                if eap.cmdidx != CMD_DIFFPUT
                    || modifiable(unsafe { &*curtab_mut().tp_diffbuf[idx] })
                {
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
            if curtab_mut().tp_diffbuf[i] != g_st().curbuf
                && !curtab_mut().tp_diffbuf[i].is_null()
                && (eap.cmdidx != CMD_DIFFPUT
                    || modifiable(unsafe { &*curtab_mut().tp_diffbuf[i] }))
            {
                emsg(gettext(
                    c"E101: More than two buffers in diff mode, don't know which one to use",
                ));
                return;
            }
        }
    } else {
        let mut p = eap.arg.wrapping_add(crate::memory::cstr_len(eap.arg));
        while p > eap.arg && ascii_iswhite(unsafe { *p.wrapping_sub(1) }) {
            p = p.wrapping_sub(1);
        }
        let mut i: usize = 0;
        while ascii_isdigit(crate::memory::cstr_char_at(eap.arg, i)) && eap.arg.wrapping_add(i) < p
        {
            i += 1;
        }
        let bufnr = if eap.arg.wrapping_add(i) == p {
            crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(eap.arg)) as i32
        } else {
            let nr = buflist_findpat(eap.arg, p, false, true, false);
            if nr < 0 {
                return;
            }
            nr
        };
        let buf = buflist_findnr(bufnr);
        if buf.is_null() {
            semsg!(gettext(c"E102: Can't find buffer \"%s\""), eap.arg,);
            return;
        }
        if buf == g_st().curbuf {
            return;
        }
        let idx_o = diff_buf_idx(buf, curtab_mut());
        if idx_o == DB_COUNT as i32 {
            semsg!(gettext(c"E103: Buffer \"%s\" is not in diff mode"), eap.arg,);
            return;
        }
        idx_other = idx_o as usize;
    }
    diff_st().diff_busy = true;
    if eap.addr_count == 0 {
        let mut linestatus: i32 = 0;
        if eap.line1 == curbuf_mut().b_ml.ml_line_count
            && diff_check_with_linestatus(curwin_mut(), eap.line1, Some(&mut linestatus)) == 0
            && linestatus == 0
            && (eap.line1 == 1
                || (diff_check_with_linestatus(curwin_mut(), eap.line1 - 1, Some(&mut linestatus))
                    >= 0
                    && linestatus == 0))
        {
            eap.line2 += 1;
        } else if eap.line1 > 0 {
            eap.line1 -= 1;
        }
    }
    let mut aco = CtxSwitch::new();
    if eap.cmdidx != CMD_DIFFGET {
        ctx_switch(
            &mut aco,
            ptr::null_mut(),
            ptr::null_mut(),
            curtab_mut().tp_diffbuf[idx_other],
            0,
        );
    }
    let idx_from = if eap.cmdidx == CMD_DIFFGET {
        idx_other
    } else {
        idx_cur
    };
    let idx_to = if eap.cmdidx == CMD_DIFFGET {
        idx_cur
    } else {
        idx_other
    };
    'theend: {
        if curbuf_mut().b_changed == 0 {
            change_warning(curbuf_mut(), 0);
            if diff_buf_idx(g_st().curbuf, curtab_mut()) != idx_to as i32 {
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
        if eap.cmdidx != CMD_DIFFGET {
            if g_st().key_typed {
                u_sync(false);
            }
            ctx_restore(&mut aco);
        }
    }
    diff_st().diff_busy = false;
    if diff_st().diff_need_update {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    check_cursor(curwin_mut());
    changed_line_abv_curs();
    if curtab_mut().tp_diffs.is_empty() {
        for wp in for_all_windows_mut() {
            if wp.w_onebuf_opt.wo_diff != 0
                && crate::memory::cstr_first_byte(wp.w_onebuf_opt.wo_fdm) == b'd'
                && wp.w_onebuf_opt.wo_fen != 0
            {
                fold_update_all(wp);
            }
        }
    }
    if diff_st().diff_need_update {
        diff_st().diff_need_update = false;
    } else {
        diff_redraw(false);
        window_layout_lock();
        apply_autocmds(
            EVENT_DIFFUPDATED,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
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
    while dp_idx < curtab_mut().tp_diffs.len() {
        if addr_count == 0 {
            while dp_idx + 1 < curtab_mut().tp_diffs.len()
                && (&curtab_mut().tp_diffs)[dp_idx + 1].df_lnum[idx_cur]
                    == (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_cur]
                        + (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_cur]
                && (&curtab_mut().tp_diffs)[dp_idx + 1].df_lnum[idx_cur] == line1 + off + 1
            {
                dp_idx += 1;
            }
        }
        if (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_cur] > line2 + off {
            break;
        }
        let mut dfree_lnum = [0; DB_COUNT];
        let mut dfree_count = [0; DB_COUNT];
        let mut did_free = false;
        let mut lnum = (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_to];
        let mut count = (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_to];
        let undo_bot = lnum + count + Linenr::from(count == 0 && buf_is_empty(curbuf_mut()));
        if (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_cur]
            + (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_cur]
            > line1 + off
            && u_save(lnum - 1, undo_bot)
        {
            let mut start_skip: Linenr = 0;
            let mut end_skip: Linenr = 0;
            if addr_count > 0 {
                start_skip = line1 + off - (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_cur];
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
                end_skip = (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_cur]
                    + (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_cur]
                    - 1
                    - (line2 + off);
                if end_skip > 0 {
                    if idx_cur == idx_from {
                        count = count.min(
                            (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_cur]
                                - start_skip
                                - end_skip,
                        );
                    } else {
                        count -= end_skip;
                        end_skip = ((&curtab_mut().tp_diffs)[dp_idx].df_count[idx_from]
                            - start_skip
                            - count)
                            .max(0);
                    }
                } else {
                    end_skip = 0;
                }
            }
            let mut buf_empty = buf_is_empty(curbuf_mut());
            let mut added: i32 = 0;
            for _ in 0..count {
                buf_empty = curbuf_mut().b_ml.ml_line_count == 1;
                if ml_delete(lnum) {
                    added -= 1;
                }
            }
            let copy_count =
                (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_from] - start_skip - end_skip;
            let from_lnum = (&curtab_mut().tp_diffs)[dp_idx].df_lnum[idx_from];
            for i in 0..copy_count {
                let nr = from_lnum + start_skip + i;
                if nr > unsafe { (*curtab_mut().tp_diffbuf[idx_from]).b_ml.ml_line_count } {
                    break;
                }
                let p = crate::memory::cstr_to_cstring(ml_get_buf(
                    unsafe { &mut *curtab_mut().tp_diffbuf[idx_from] },
                    nr,
                ));
                let _ = ml_append(lnum + i - 1, p.as_ptr().cast_mut(), 0, false);
                added += 1;
                if buf_empty && curbuf_mut().b_ml.ml_line_count == 2 {
                    buf_empty = false;
                    let _ = ml_delete(2);
                }
            }
            let new_count = (&curtab_mut().tp_diffs)[dp_idx].df_count[idx_to] + added;
            (&mut curtab_mut().tp_diffs)[dp_idx].df_count[idx_to] = new_count;
            let mut target_id = Some((&curtab_mut().tp_diffs)[dp_idx].df_id);
            if start_skip == 0 && end_skip == 0 {
                let mut i = 0;
                while i < DB_COUNT {
                    if !curtab_mut().tp_diffbuf[i].is_null()
                        && i != idx_from
                        && i != idx_to
                        && !diff_equal_entry(&(&curtab_mut().tp_diffs)[dp_idx], idx_from, i)
                    {
                        break;
                    }
                    i += 1;
                }
                if i == DB_COUNT {
                    dfree_lnum = (&curtab_mut().tp_diffs)[dp_idx].df_lnum;
                    dfree_count = (&curtab_mut().tp_diffs)[dp_idx].df_count;
                    did_free = true;
                    target_id = curtab_mut()
                        .tp_diffs
                        .as_slice()
                        .get(dp_idx + 1)
                        .map(|d| d.df_id);
                    diff_free(curtab_mut(), dp_idx);
                }
            }
            if added != 0 {
                mark_adjust(lnum, lnum + count - 1, MAXLNUM, added, K_EXTMARK_NOOP);
                if curwin_mut().w_cursor.lnum >= lnum {
                    if curwin_mut().w_cursor.lnum >= lnum + count {
                        curwin_mut().w_cursor.lnum += added;
                        curwin_mut().w_cursor.lnum = curwin_mut()
                            .w_cursor
                            .lnum
                            .min(curbuf_mut().b_ml.ml_line_count);
                    } else if added < 0 {
                        curwin_mut().w_cursor.lnum = lnum;
                    }
                }
            }
            extmark_adjust(
                curbuf_mut(),
                lnum,
                lnum + count - 1,
                MAXLNUM,
                added,
                K_EXTMARK_UNDO,
            );
            changed_lines(curbuf_mut(), lnum, 0, lnum + count, added, true);
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
                (&mut curtab_mut().tp_diffs)[dp_idx].df_count[idx_to] = new_count;
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
            if curtab_mut().tp_diffbuf[i] == wp.w_buffer && i != skip_idx {
                fold_update(&mut *(wp), df_lnum[i], df_lnum[i] + df_count[i]);
            }
        }
    }
}
pub fn diff_mode_buf(buf: *mut FileBuf) -> bool {
    for tp in for_all_tabpages_mut() {
        if diff_buf_idx(buf, tp) != DB_COUNT as i32 {
            return true;
        }
    }
    false
}
pub fn diff_move_to(dir: i32, mut count: i32) -> bool {
    let mut lnum = curwin_mut().w_cursor.lnum;
    let idx = diff_buf_idx(g_st().curbuf, curtab_mut());
    if idx == DB_COUNT as i32 || curtab_mut().tp_diffs.is_empty() {
        return false;
    }
    let u_idx = idx as usize;
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab_mut().tp_diffs.is_empty() {
        return false;
    }
    while {
        count -= 1;
        count >= 0
    } {
        let diffs = &curtab_mut().tp_diffs;
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
    lnum = lnum.min(curbuf_mut().b_ml.ml_line_count);
    if lnum == curwin_mut().w_cursor.lnum {
        return false;
    }
    setpcmark();
    curwin_mut().w_cursor.lnum = lnum;
    curwin_mut().w_cursor.col = 0;
    true
}
fn diff_get_corresponding_line_int(buf1: *mut FileBuf, lnum1: Linenr) -> Linenr {
    let mut baseline: Linenr = 0;
    let idx1 = diff_buf_idx(buf1, curtab_mut());
    let idx2 = diff_buf_idx(g_st().curbuf, curtab_mut());
    if idx1 == DB_COUNT as i32 || idx2 == DB_COUNT as i32 || curtab_mut().tp_diffs.is_empty() {
        return lnum1;
    }
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    if curtab_mut().tp_diffs.is_empty() {
        return lnum1;
    }
    let u_idx1 = idx1 as usize;
    let u_idx2 = idx2 as usize;
    for dp in &curtab_mut().tp_diffs {
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
            && dp.df_lnum[u_idx2] <= curwin_mut().w_cursor.lnum
            && dp.df_lnum[u_idx2] + dp.df_count[u_idx2] > curwin_mut().w_cursor.lnum
        {
            return curwin_mut().w_cursor.lnum;
        }
        baseline =
            (dp.df_lnum[u_idx1] + dp.df_count[u_idx1]) - (dp.df_lnum[u_idx2] + dp.df_count[u_idx2]);
    }
    lnum1 - baseline
}
pub fn diff_get_corresponding_line(buf1: *mut FileBuf, lnum1: Linenr) -> Linenr {
    let lnum = diff_get_corresponding_line_int(buf1, lnum1);
    lnum.min(curbuf_mut().b_ml.ml_line_count)
}
pub fn diff_lnum_win(lnum: Linenr, wp: &mut WinInfoInfo) -> Linenr {
    let idx = diff_buf_idx(g_st().curbuf, curtab_mut());
    if idx == DB_COUNT as i32 {
        return 0;
    }
    let u_idx = idx as usize;
    if curtab_mut().tp_diff_invalid != 0 {
        ex_diffupdate(&mut crate::ex_docmd::Exarg::new());
    }
    let Some(dp) = curtab_mut()
        .tp_diffs
        .iter()
        .find(|dp| lnum <= dp.df_lnum[u_idx] + dp.df_count[u_idx])
    else {
        return wp.buf_mut().b_ml.ml_line_count - (curbuf_mut().b_ml.ml_line_count - lnum);
    };
    let i = diff_buf_idx(wp.w_buffer, curtab_mut());
    if i == DB_COUNT as i32 {
        return 0;
    }
    let u_i = i as usize;
    let n = lnum + (dp.df_lnum[u_i] - dp.df_lnum[u_idx]);
    n.min(dp.df_lnum[u_i] + dp.df_count[u_i])
}
fn parse_diff_ed(line: impl crate::strings::IntoCCharPtr, hunk: &mut Diffhunk) -> bool {
    let line = line.into_c_char_ptr().cast_mut();
    {
        let mut p = line;
        let f1 = getdigits_int32(&raw mut p, true, 0);
        let l1 = if crate::memory::cstr_first_byte(p) == b',' {
            p = p.wrapping_add(1);
            getdigits_int(&raw mut p, true, 0)
        } else {
            f1
        };
        if crate::memory::cstr_first_byte(p) != b'a'
            && crate::memory::cstr_first_byte(p) != b'c'
            && crate::memory::cstr_first_byte(p) != b'd'
        {
            return false;
        }
        let difftype = crate::memory::cstr_first_byte(p);
        p = p.wrapping_add(1);
        let f2 = getdigits_int(&raw mut p, true, 0);
        let l2 = if crate::memory::cstr_first_byte(p) == b',' {
            p = p.wrapping_add(1);
            getdigits_int(&raw mut p, true, 0)
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
}
fn parse_diff_unified(line: impl crate::strings::IntoCCharPtr, hunk: &mut Diffhunk) -> bool {
    let line = line.into_c_char_ptr().cast_mut();
    {
        let mut p = line;
        if crate::memory::cstr_first_byte(p) == b'@'
            && crate::memory::cstr_byte_at(p, 1) == b'@'
            && crate::memory::cstr_byte_at(p, 2) == b' '
            && crate::memory::cstr_byte_at(p, 3) == b'-'
        {
            p = p.wrapping_add(4);
            let mut oldline = getdigits_int32(&raw mut p, true, 0);
            let oldcount = if crate::memory::cstr_first_byte(p) == b',' {
                p = p.wrapping_add(1);
                getdigits_int(&raw mut p, true, 0)
            } else {
                1
            };
            if crate::memory::cstr_first_byte(p) == b' '
                && crate::memory::cstr_byte_at(p, 1) == b'+'
            {
                p = p.wrapping_add(2);
                let mut newline = getdigits_int(&raw mut p, true, 0);
                let newcount = if crate::memory::cstr_first_byte(p) == b',' {
                    p = p.wrapping_add(1);
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
                hunk.lnum_orig = oldline;
                hunk.count_orig = oldcount;
                hunk.lnum_new = newline;
                hunk.count_new = newcount;
                return true;
            }
            return false;
        }
        false
    }
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
        diff_check_fill(curwin_mut(), tv_get_lnum(&argvars[0])).max(0) as Varnumber,
    );
}
pub fn f_diff_hl_id(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut diffline = Diffline::default();
    let cache_results = (diff_st().diff_flags & ALL_INLINE_DIFF) == 0;
    let mut lnum = tv_get_lnum(&argvars[0]);
    if lnum < 0 {
        lnum = 0;
    }
    if !cache_results
        || lnum != diff_st().f_diff_hlid_prev_lnum
        || diff_st().f_diff_hlid_changedtick != buf_get_changedtick(curbuf_mut())
        || diff_st().f_diff_hlid_fnum != curbuf_mut().b_fnum
        || diff_st().diff_flags != diff_st().f_diff_hlid_prev_diff_flags
    {
        let mut linestatus: i32 = 0;
        diff_check_with_linestatus(curwin_mut(), lnum, Some(&mut linestatus));
        if linestatus < 0 {
            if linestatus == -1 {
                diff_st().f_diff_hlid_change_start = MAXCOL;
                diff_st().f_diff_hlid_change_end = -1;
                if diff_find_change(curwin_mut(), lnum, &mut diffline) {
                    diff_st().f_diff_hlid_hl_id = HLF_ADD;
                } else {
                    diff_st().f_diff_hlid_hl_id = HLF_CHD;
                    if !diffline.changes.is_empty() && cache_results {
                        diff_st().f_diff_hlid_change_start =
                            diffline.changes[0].dc_start[diffline.bufidx as usize];
                        diff_st().f_diff_hlid_change_end =
                            diffline.changes[0].dc_end[diffline.bufidx as usize];
                    }
                }
            } else {
                diff_st().f_diff_hlid_hl_id = HLF_ADD;
            }
        } else {
            diff_st().f_diff_hlid_hl_id = 0;
        }
        if cache_results {
            diff_st().f_diff_hlid_prev_lnum = lnum;
            diff_st().f_diff_hlid_changedtick = buf_get_changedtick(curbuf_mut());
            diff_st().f_diff_hlid_fnum = curbuf_mut().b_fnum;
            diff_st().f_diff_hlid_prev_diff_flags = diff_st().diff_flags;
        }
    }
    if diff_st().f_diff_hlid_hl_id == HLF_CHD || diff_st().f_diff_hlid_hl_id == HLF_TXD {
        let col = tv_get_number(&argvars[1]) as i32 - 1;
        if cache_results {
            if col >= diff_st().f_diff_hlid_change_start && col < diff_st().f_diff_hlid_change_end {
                diff_st().f_diff_hlid_hl_id = HLF_TXD;
            } else {
                diff_st().f_diff_hlid_hl_id = HLF_CHD;
            }
        } else {
            diff_st().f_diff_hlid_hl_id = HLF_CHD;
            for change in &diffline.changes {
                let added = diff_change_parse(
                    &diffline,
                    change,
                    &mut diff_st().f_diff_hlid_change_start,
                    &mut diff_st().f_diff_hlid_change_end,
                );
                if col >= diff_st().f_diff_hlid_change_start
                    && col < diff_st().f_diff_hlid_change_end
                {
                    diff_st().f_diff_hlid_hl_id = if added { HLF_TXA } else { HLF_TXD };
                    break;
                }
                if col < diff_st().f_diff_hlid_change_start {
                    break;
                }
            }
        }
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(diff_st().f_diff_hlid_hl_id as Varnumber);
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
        let mut ed_line = *b"2,4c5,6\n\0";
        assert!(parse_diff_ed(ed_line.as_mut_ptr().cast(), &mut hunk));
        assert_eq!(hunk.lnum_orig, 2);
        assert_eq!(hunk.count_orig, 3);
        assert_eq!(hunk.lnum_new, 5);
        assert_eq!(hunk.count_new, 2);
        let mut uni_line = *b"@@ -10,3 +12,4 @@\n\0";
        assert!(parse_diff_unified(uni_line.as_mut_ptr().cast(), &mut hunk));
        assert_eq!(hunk.lnum_orig, 10);
        assert_eq!(hunk.count_orig, 3);
        assert_eq!(hunk.lnum_new, 12);
        assert_eq!(hunk.count_new, 4);
    }
}
