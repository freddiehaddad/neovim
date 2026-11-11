//! Code for folding (`src/nvim/fold.c`).
#![forbid(unsafe_code)]

use crate::api::extmark::parse_virt_text;
use crate::api::private::helpers::{api_clear_error, api_free_object};
use crate::api::private::validate::{Error, ErrorType, ObjectType};
use crate::buffer::{FileBuf, MAXLNUM};
use crate::buffer_updates::buf_updates_send_changes;
use crate::change::{TAB, changed_lines};
use crate::charset::{ascii_isdigit, ascii_iswhite, transstr, vim_isprintc};
use crate::cursor::{Colnr, CursorPos, Linenr, check_cursor_col, ltoreq};
use crate::decoration::{VirtText, next_virt_text_chunk};
use crate::diff::{diff_infold, diff_lnum_win};
use crate::drawscreen::{
    UPD_INVERTED, UPD_NOT_VALID, redraw_buf_later, redraw_later, redraw_win_range_later,
};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, K_CALLBACK_NONE_CONST, TypVal, VarType, Varnumber, tv_get_lnum,
};
use crate::eval::vars::{
    VV_FOLDDASHES, VV_FOLDEND, VV_FOLDLEVEL, VV_FOLDSTART, VV_LNUM, get_vim_var_nr,
    get_vim_var_str, set_vim_var_nr, set_vim_var_string,
};
use crate::eval::{eval_foldexpr, eval_foldtext};
use crate::ex_session::{put_eol, put_line};
use crate::extmark::{K_EXTMARK_UNDO, K_FALSE, K_NONE, K_TRUE, TriState, extmark_splice_cols};
use crate::globals::*;
use crate::indent::{get_indent_buf, get_sw_value};
use crate::mark::setpcmark;
use crate::mbyte::{mb_adjust_cursor, ptr2cells, utf_ptr2char, utfc_ptr2len};
use crate::mcursor::mc_replaying;
use crate::memline::{ml_get, ml_get_len, ml_replace_buf};
use crate::message::emsg;
use crate::r#move::changed_window_setting;
use crate::ops::skip_comment;
use crate::option_vars::*;
use crate::os::fs::CFile;
use crate::os::input::line_breakcheck;
use crate::plines::plines_win_nofold;
use crate::search::{FORWARD, linewhite};
use crate::statusline::Foldinfo;
use crate::strings::{format_to_buf, gettext};
use crate::syntax::syn_get_foldlevel;
use crate::undo::u_save;
use crate::window::WinInfoInfo;
use std::ffi::CStr;

use crate::state::MODE_INSERT;

pub const FOLD_TEXT_LEN: usize = 51;
const MAX_LEVEL: i32 = 20;

const FD_OPEN: u8 = 0;
const FD_CLOSED: u8 = 1;
const FD_LEVEL: u8 = 2;

const DONE_NOTHING: i32 = 0;
const DONE_ACTION: i32 = 1;
const DONE_FOLD: i32 = 2;

const K_WIN_OPT_FOLDTEXT: usize = 22;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LevelGetterKind {
    Marker,
    Expr,
    Syntax,
    Diff,
    Indent,
}

#[repr(C)]
#[derive(Clone)]
pub struct FoldEntry {
    fd_top: Linenr,
    fd_len: Linenr,
    fd_nested: Vec<FoldEntry>,
    fd_flags: u8,
    fd_small: TriState,
}

struct Fline {
    wp: crate::globals::WinPtr,
    lnum: Linenr,
    off: Linenr,
    lnum_save: Linenr,
    lvl: i32,
    lvl_next: i32,
    start: i32,
    end: i32,
    had_end: i32,
}

impl Fline {
    #[inline]
    fn wp_mut(&self) -> &'static mut WinInfoInfo {
        self.wp.borrow_mut()
    }
}

struct FoldModuleState {
    fold_changed: std::cell::Cell<bool>,
    invalid_top: std::cell::Cell<Linenr>,
    invalid_bot: std::cell::Cell<Linenr>,
    prev_lnum: std::cell::Cell<Linenr>,
    prev_lnum_lvl: std::cell::Cell<i32>,
    foldstartmarkerlen: std::cell::Cell<usize>,
    foldendmarker: std::cell::RefCell<Vec<u8>>,
    foldendmarkerlen: std::cell::Cell<usize>,
    got_fdt_error: std::cell::Cell<bool>,
    last_wp: std::cell::Cell<crate::globals::WinPtr>,
    fdt_last_lnum: std::cell::Cell<Linenr>,
    fdt_result_entered: std::cell::Cell<bool>,
}

impl FoldModuleState {
    const fn new() -> Self {
        Self {
            fold_changed: std::cell::Cell::new(false),
            invalid_top: std::cell::Cell::new(0),
            invalid_bot: std::cell::Cell::new(0),
            prev_lnum: std::cell::Cell::new(0),
            prev_lnum_lvl: std::cell::Cell::new(-1),
            foldstartmarkerlen: std::cell::Cell::new(0),
            foldendmarker: std::cell::RefCell::new(Vec::new()),
            foldendmarkerlen: std::cell::Cell::new(0),
            got_fdt_error: std::cell::Cell::new(false),
            last_wp: std::cell::Cell::new(crate::globals::WinPtr::NULL),
            fdt_last_lnum: std::cell::Cell::new(0),
            fdt_result_entered: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static FOLD_STATE: FoldModuleState = const { FoldModuleState::new() };
}

fn clone_foldendmarker() -> Vec<u8> {
    FOLD_STATE.with(|st| st.foldendmarker.borrow().clone())
}

static E_NOFOLD: &CStr = c"E490: No fold found";

use crate::drawscreen::redraw_curbuf_later;
use crate::strings::ngettext;

/// Copy that folding state from window "wp_from" to window "wp_to".
pub fn copy_folding_state(wp_from: &mut WinInfoInfo, wp_to: &mut WinInfoInfo) {
    wp_to.w_fold_manual = wp_from.w_fold_manual;
    wp_to.w_foldinvalid = wp_from.w_foldinvalid;
    wp_to.w_folds.clone_from(&wp_from.w_folds);
}

/// True if there may be folded lines in window `win`.
pub fn has_any_folding(win: &mut WinInfoInfo) -> i32 {
    i32::from(
        win.buf_mut().terminal.get().is_null()
            && win.w_onebuf_opt.wo_fen.get()
            && !mc_replaying()
            && (!foldmethod_is_manual(win) || !win.w_folds.is_empty()),
    )
}

/// Return true if line "lnum" in window "win" is part of a closed fold.
pub fn has_folding(
    win: &mut WinInfoInfo,
    lnum: Linenr,
    firstp: Option<&mut Linenr>,
    lastp: Option<&mut Linenr>,
) -> bool {
    has_folding_win(win, lnum, firstp, lastp, true, None)
}

/// Search folds starting at lnum.
pub fn has_folding_win(
    win: &mut WinInfoInfo,
    lnum: Linenr,
    mut firstp: Option<&mut Linenr>,
    mut lastp: Option<&mut Linenr>,
    cache: bool,
    mut infop: Option<&mut Foldinfo>,
) -> bool {
    checkupdate(win);

    if has_any_folding(win) == 0 {
        if let Some(infop) = infop.as_deref_mut() {
            infop.fi_level = 0;
        }
        return false;
    }

    let mut had_folded = false;
    let mut first: Linenr = 0;
    let mut last: Linenr = 0;

    if cache {
        let x = find_wl_entry(win, lnum);
        if x >= 0 {
            let wl = &(&win.w_lines)[x as usize];
            first = wl.wl_lnum;
            last = wl.wl_foldend;
            had_folded = wl.wl_folded;
        }
    }

    let mut lnum_rel = lnum;
    let mut level: i32 = 0;
    let mut low_level: i32 = 0;
    let mut maybe_small = false;
    let mut use_level = false;

    if first == 0 {
        let mut folds = std::mem::take(&mut win.w_folds);
        let mut gap: &mut [FoldEntry] = &mut folds;
        while let (true, idx) = fold_find(gap, lnum_rel) {
            let fp = &mut gap[idx];
            if lnum_rel == fp.fd_top && low_level == 0 {
                low_level = level + 1;
            }

            first += fp.fd_top;
            last += fp.fd_top;

            had_folded = check_closed(
                win,
                fp,
                &mut use_level,
                level,
                &mut maybe_small,
                lnum - lnum_rel,
            );
            if had_folded {
                last += fp.fd_len - 1;
                break;
            }

            lnum_rel -= fp.fd_top;
            gap = &mut fp.fd_nested;
            level += 1;
        }
        win.w_folds = folds;
    }

    if !had_folded {
        if let Some(infop) = infop.as_deref_mut() {
            infop.fi_level = level;
            infop.fi_lnum = lnum - lnum_rel;
            infop.fi_low_level = if low_level == 0 { level } else { low_level };
        }
        return false;
    }

    last = last.min(win.buf_mut().b_ml.ml_line_count);
    if let Some(ref mut p) = lastp {
        **p = last;
    }
    if let Some(ref mut p) = firstp {
        **p = first;
    }
    if let Some(infop) = infop {
        infop.fi_level = level + 1;
        infop.fi_lnum = first;
        infop.fi_low_level = if low_level == 0 { level + 1 } else { low_level };
    }
    true
}

fn fold_level(lnum: Linenr) -> i32 {
    let (invalid_top, invalid_bot, prev_lnum, prev_lnum_lvl) = FOLD_STATE.with(|st| {
        (
            st.invalid_top.get(),
            st.invalid_bot.get(),
            st.prev_lnum.get(),
            st.prev_lnum_lvl.get(),
        )
    });
    if invalid_top == 0 {
        checkupdate(curwin_mut());
    } else if lnum == prev_lnum && prev_lnum_lvl >= 0 {
        return prev_lnum_lvl;
    } else if lnum >= invalid_top && lnum <= invalid_bot {
        return -1;
    }

    if has_any_folding(curwin_mut()) == 0 {
        return 0;
    }

    fold_level_win(curwin_mut(), lnum)
}

/// Low level function to check if a line is folded.
pub fn line_folded(win: &mut WinInfoInfo, lnum: Linenr) -> bool {
    fold_info(win, lnum).fi_lines != 0
}

/// Count the number of lines that are folded at line number "lnum".
pub fn fold_info(win: &mut WinInfoInfo, lnum: Linenr) -> Foldinfo {
    let mut info = Foldinfo {
        fi_lnum: 0,
        fi_level: 0,
        fi_low_level: 0,
        fi_lines: 0,
    };
    let mut last: Linenr = 0;

    if has_folding_win(win, lnum, None, Some(&mut last), false, Some(&mut info)) {
        info.fi_lines = last - lnum + 1;
    } else {
        info.fi_lines = 0;
    }

    info
}

/// Return true if 'foldmethod' is "manual".
pub fn foldmethod_is_manual(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.as_bytes().get(3) == Some(&b'u')
}

/// Return true if 'foldmethod' is "indent".
pub fn foldmethod_is_indent(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.first_byte() == b'i'
}

/// Return true if 'foldmethod' is "expr".
pub fn foldmethod_is_expr(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.as_bytes().get(1) == Some(&b'x')
}

/// Return true if 'foldmethod' is "marker".
pub fn foldmethod_is_marker(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.as_bytes().get(2) == Some(&b'r')
}

/// Return true if 'foldmethod' is "syntax".
pub fn foldmethod_is_syntax(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.first_byte() == b's'
}

/// Return true if 'foldmethod' is "diff".
pub fn foldmethod_is_diff(wp: &mut WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_fdm.first_byte() == b'd'
}

/// Close fold for current window at position "pos".
pub fn close_fold(pos: CursorPos, count: i32) {
    set_fold_repeat(pos, count, false);
}

/// Close fold for current window at position `pos` recursively.
pub fn close_fold_recurse(pos: CursorPos) {
    set_manual_fold(pos, false, true, None);
}

/// Open or Close folds for current window in lines "first" to "last".
pub fn op_fold_range(
    firstpos: CursorPos,
    lastpos: CursorPos,
    opening: bool,
    recurse: bool,
    had_visual: bool,
) {
    let mut done = DONE_NOTHING;
    let first = firstpos.lnum;
    let last = lastpos.lnum;
    let mut lnum = first;

    while lnum <= last {
        let temp = CursorPos {
            lnum,
            col: 0,
            coladd: 0,
        };
        let mut lnum_next = lnum;
        if opening && !recurse {
            has_folding(curwin_mut(), lnum, None, Some(&mut lnum_next));
        }
        set_manual_fold(temp, opening, recurse, Some(&mut done));
        if !opening && !recurse {
            has_folding(curwin_mut(), lnum, None, Some(&mut lnum_next));
        }
        lnum = lnum_next + 1;
    }
    if done == DONE_NOTHING {
        emsg(gettext(E_NOFOLD));
    }
    if had_visual {
        redraw_curbuf_later(UPD_INVERTED);
    }
}

/// Open fold for current window at position "pos".
pub fn open_fold(pos: CursorPos, count: i32) {
    set_fold_repeat(pos, count, true);
}

/// Open fold for current window at position `pos` recursively.
pub fn open_fold_recurse(pos: CursorPos) {
    set_manual_fold(pos, true, true, None);
}

/// Open folds until the cursor line is not in a closed fold.
pub fn fold_open_cursor() {
    checkupdate(curwin_mut());
    if has_any_folding(curwin_mut()) != 0 {
        loop {
            let mut done = DONE_NOTHING;
            set_manual_fold(curwin_mut().w_cursor, true, false, Some(&mut done));
            if (done & DONE_ACTION) == 0 {
                break;
            }
        }
    }
}

/// Set new foldlevel for current window.
pub fn new_fold_level() {
    new_fold_level_win(curwin_mut());

    if foldmethod_is_diff(curwin_mut()) && curwin_mut().w_onebuf_opt.wo_scb.get() {
        for wp in for_all_windows_ptr() {
            if wp != curwin_ptr() && foldmethod_is_diff(&mut *wp) && wp.w_onebuf_opt.wo_scb.get() {
                wp.w_onebuf_opt
                    .wo_fdl
                    .set(curwin_mut().w_onebuf_opt.wo_fdl.get());
                new_fold_level_win(&mut *wp);
            }
        }
    }
}

fn new_fold_level_win(wp: &mut WinInfoInfo) {
    checkupdate(wp);
    if wp.w_fold_manual {
        for entry in &mut wp.w_folds {
            entry.fd_flags = FD_LEVEL;
        }
        wp.w_fold_manual = false;
    }
    changed_window_setting(&mut *wp);
}

/// Apply 'foldlevel' to all folds that don't contain the cursor.
pub fn fold_check_close() {
    if opt_vars().p_fcl.is_empty() {
        return;
    }

    checkupdate(curwin_mut());
    if check_close_rec(
        &mut curwin_mut().w_folds,
        curwin_mut().w_cursor.lnum,
        curwin_mut().w_onebuf_opt.wo_fdl.get() as i32,
    ) {
        changed_window_setting(curwin_mut());
    }
}

fn check_close_rec(gap: &mut [FoldEntry], lnum: Linenr, level: i32) -> bool {
    let mut retval = false;
    for entry in gap {
        if entry.fd_flags == FD_OPEN {
            if level <= 0 && (lnum < entry.fd_top || lnum >= entry.fd_top + entry.fd_len) {
                entry.fd_flags = FD_LEVEL;
                retval = true;
            } else {
                retval |= check_close_rec(&mut entry.fd_nested, lnum - entry.fd_top, level - 1);
            }
        }
    }
    retval
}

pub fn fold_manual_allowed(create: bool) -> i32 {
    if foldmethod_is_manual(curwin_mut()) || foldmethod_is_marker(curwin_mut()) {
        return 1;
    }
    if create {
        emsg(gettext(
            c"E350: Cannot create fold with current 'foldmethod'",
        ));
    } else {
        emsg(gettext(
            c"E351: Cannot delete fold with current 'foldmethod'",
        ));
    }
    0
}

/// Create a fold from line "start" to line "end" (inclusive) in the current
/// window.
pub fn fold_create(wp: &mut WinInfoInfo, mut start: CursorPos, mut end: CursorPos) {
    let mut use_level = false;
    let mut closed = false;
    let mut level: i32 = 0;
    let mut start_rel = start;
    let mut end_rel = end;

    if start.lnum > end.lnum {
        end = start_rel;
        start = end_rel;
        start_rel = start;
        end_rel = end;
    }

    if foldmethod_is_marker(&mut *wp) {
        fold_create_markers(wp, start, end);
        return;
    }

    checkupdate(wp);

    let mut i_usize: usize = 0;
    let mut gap = &mut wp.w_folds;
    if !gap.is_empty() {
        loop {
            let (found, idx) = fold_find(gap, start_rel.lnum);
            i_usize = idx;
            if !found {
                break;
            }
            if gap[idx].fd_top + gap[idx].fd_len > end_rel.lnum {
                let fp = &mut gap[idx];
                start_rel.lnum -= fp.fd_top;
                end_rel.lnum -= fp.fd_top;
                if use_level || fp.fd_flags == FD_LEVEL {
                    use_level = true;
                    if i64::from(level) >= wp.w_onebuf_opt.wo_fdl.get() {
                        closed = true;
                    }
                } else if fp.fd_flags == FD_CLOSED {
                    closed = true;
                }
                gap = &mut fp.fd_nested;
                level += 1;
            } else {
                break;
            }
        }
    }

    {
        let gap_ref = gap;
        let mut fold_ga: Vec<FoldEntry> = Vec::new();

        let mut cont: usize = 0;
        while i_usize + cont < gap_ref.len() {
            if gap_ref[i_usize + cont].fd_top > end_rel.lnum {
                break;
            }
            cont += 1;
        }
        if cont > 0 {
            start_rel.lnum = start_rel.lnum.min(gap_ref[i_usize].fd_top);
            let last_cont = &gap_ref[i_usize + cont - 1];
            end_rel.lnum = end_rel.lnum.max(last_cont.fd_top + last_cont.fd_len - 1);
            for mut child in gap_ref.drain(i_usize..(i_usize + cont)) {
                child.fd_top -= start_rel.lnum;
                fold_ga.push(child);
            }
        }
        gap_ref.insert(
            i_usize,
            FoldEntry {
                fd_top: start_rel.lnum,
                fd_len: end_rel.lnum - start_rel.lnum + 1,
                fd_nested: fold_ga,
                fd_flags: FD_CLOSED,
                fd_small: K_NONE,
            },
        );

        if use_level && !closed && i64::from(level) < wp.w_onebuf_opt.wo_fdl.get() {
            close_fold(start, 1);
        }
        if !use_level {
            wp.w_fold_manual = true;
        }

        changed_window_setting(&mut *wp);
    }
}

#[expect(clippy::too_many_arguments)]
fn delete_fold_one(
    wp: &mut WinInfoInfo,
    gap: &mut Vec<FoldEntry>,
    lnum: Linenr,
    lnum_off: Linenr,
    level: &mut i32,
    use_level: &mut bool,
    maybe_small: &mut bool,
    recursive: bool,
    did_one: &mut bool,
    first_lnum: &mut Linenr,
    last_lnum: &mut Linenr,
) -> Option<Linenr> {
    let (true, idx) = fold_find(gap, lnum - lnum_off) else {
        return None;
    };
    if !check_closed(wp, &mut gap[idx], use_level, *level, maybe_small, lnum_off) {
        let child_off = lnum_off + gap[idx].fd_top;
        *level += 1;
        if let Some(next_lnum) = delete_fold_one(
            wp,
            &mut gap[idx].fd_nested,
            lnum,
            child_off,
            level,
            use_level,
            maybe_small,
            recursive,
            did_one,
            first_lnum,
            last_lnum,
        ) {
            return Some(next_lnum);
        }
    }

    let next_lnum = (gap[idx].fd_top + gap[idx].fd_len) + lnum_off;
    if foldmethod_is_manual(&mut *wp) {
        delete_fold_entry(gap, idx as i32, recursive);
    } else {
        *first_lnum = (*first_lnum).min(gap[idx].fd_top + lnum_off);
        *last_lnum = (*last_lnum).max(next_lnum);
        if !*did_one {
            parse_marker(&mut *wp);
        }
        delete_fold_markers(wp, &mut gap[idx], recursive, lnum_off);
    }
    *did_one = true;
    changed_window_setting(&mut *wp);
    Some(next_lnum)
}

pub fn delete_fold(
    wp: &mut WinInfoInfo,
    start: Linenr,
    end: Linenr,
    recursive: i32,
    had_visual: bool,
) {
    let mut maybe_small = false;
    let mut level: i32 = 0;
    let mut lnum = start;
    let mut did_one = false;
    let mut first_lnum = MAXLNUM;
    let mut last_lnum: Linenr = 0;

    checkupdate(wp);

    let mut folds = std::mem::take(&mut wp.w_folds);
    while lnum <= end {
        let mut use_level = false;
        if let Some(next_lnum) = delete_fold_one(
            wp,
            &mut folds,
            lnum,
            0,
            &mut level,
            &mut use_level,
            &mut maybe_small,
            recursive != 0,
            &mut did_one,
            &mut first_lnum,
            &mut last_lnum,
        ) {
            lnum = next_lnum;
        } else {
            lnum += 1;
        }
    }
    wp.w_folds = folds;
    if !did_one {
        emsg(gettext(E_NOFOLD));
        if had_visual {
            redraw_buf_later(wp.buf_mut(), UPD_INVERTED);
        }
    } else {
        check_cursor_col(&mut *wp);
    }

    if last_lnum > 0 {
        changed_lines(wp.buf_mut(), first_lnum, 0, last_lnum, 0, false);

        let num_changed = i64::from(last_lnum - first_lnum);
        buf_updates_send_changes(wp.buf_mut(), first_lnum, num_changed, num_changed);
    }
}

/// Remove all folding for window "win".
pub fn clear_folding(win: &mut WinInfoInfo) {
    win.w_folds.clear();
    win.w_foldinvalid = false;
}

/// Update folds for changes in the buffer of a window.
pub fn fold_update(wp: &mut WinInfoInfo, top: Linenr, bot: Linenr) {
    if g_st().disable_fold_update != 0
        || ((g_st().state_global & MODE_INSERT) != 0 && !foldmethod_is_indent(&mut *wp))
    {
        return;
    }

    if g_st().need_diff_redraw {
        return;
    }

    if !wp.w_folds.is_empty() {
        let maybe_small_start = top.min(bot);
        let maybe_small_end = top.max(bot);

        let folds = &mut wp.w_folds;
        let (_, mut idx) = fold_find(folds, maybe_small_start);
        while idx < folds.len() && folds[idx].fd_top <= maybe_small_end {
            folds[idx].fd_small = K_NONE;
            idx += 1;
        }
    }

    if foldmethod_is_indent(&mut *wp)
        || foldmethod_is_expr(&mut *wp)
        || foldmethod_is_marker(&mut *wp)
        || foldmethod_is_diff(&mut *wp)
        || foldmethod_is_syntax(&mut *wp)
    {
        let save_got_int = g_st().got_int;
        g_st().got_int = false;
        fold_update_iems(&mut *wp, top, bot);
        g_st().got_int |= save_got_int;
    }
}

/// Updates folds when leaving insert-mode.
pub fn fold_update_after_insert() {
    if foldmethod_is_manual(curwin_mut())
        || foldmethod_is_syntax(curwin_mut())
        || foldmethod_is_expr(curwin_mut())
    {
        return;
    }

    fold_update_all(curwin_mut());
    fold_open_cursor();
}

/// Update all lines in a window for folding.
pub fn fold_update_all(win: &mut WinInfoInfo) {
    win.w_foldinvalid = true;
    redraw_later(Some(&mut *win), UPD_NOT_VALID);
}

pub fn fold_move_to(updown: bool, dir: i32, count: i32) -> bool {
    let mut retval = false;

    checkupdate(curwin_mut());

    for _ in 0..count {
        let mut lnum_off: Linenr = 0;
        let mut gap = &mut curwin_mut().w_folds;
        if gap.is_empty() {
            break;
        }
        let mut use_level = false;
        let mut maybe_small = false;
        let mut lnum_found = curwin_mut().w_cursor.lnum;
        let mut level: i32 = 0;
        let mut last = false;
        loop {
            let (found, idx) = fold_find(gap, curwin_mut().w_cursor.lnum - lnum_off);
            if !found {
                if !updown || gap.is_empty() {
                    break;
                }
                if dir == FORWARD {
                    if idx >= gap.len() {
                        break;
                    }
                    let lnum = gap[idx].fd_top + lnum_off;
                    if lnum > curwin_mut().w_cursor.lnum {
                        lnum_found = lnum;
                    }
                } else {
                    if idx == 0 {
                        break;
                    }
                    let prev_fp = &gap[idx - 1];
                    let lnum = prev_fp.fd_top + lnum_off + prev_fp.fd_len - 1;
                    if lnum < curwin_mut().w_cursor.lnum {
                        lnum_found = lnum;
                    }
                }
                break;
            }

            if !last {
                if check_closed(
                    curwin_mut(),
                    &mut gap[idx],
                    &mut use_level,
                    level,
                    &mut maybe_small,
                    lnum_off,
                ) {
                    last = true;
                }
                if last && !updown {
                    break;
                }
            }

            if updown {
                if dir == FORWARD {
                    if idx + 1 < gap.len() {
                        let lnum = gap[idx + 1].fd_top + lnum_off;
                        if lnum > curwin_mut().w_cursor.lnum {
                            lnum_found = lnum;
                        }
                    }
                } else if idx > 0 {
                    let prev_fp = &gap[idx - 1];
                    let lnum = prev_fp.fd_top + lnum_off + prev_fp.fd_len - 1;
                    if lnum < curwin_mut().w_cursor.lnum {
                        lnum_found = lnum;
                    }
                }
            } else if dir == FORWARD {
                let lnum = gap[idx].fd_top + lnum_off + gap[idx].fd_len - 1;
                if lnum > curwin_mut().w_cursor.lnum {
                    lnum_found = lnum;
                }
            } else {
                let lnum = gap[idx].fd_top + lnum_off;
                if lnum < curwin_mut().w_cursor.lnum {
                    lnum_found = lnum;
                }
            }

            if last {
                break;
            }

            let top = gap[idx].fd_top;
            gap = &mut gap[idx].fd_nested;
            lnum_off += top;
            level += 1;
        }
        if lnum_found != curwin_mut().w_cursor.lnum {
            if !retval {
                setpcmark();
            }
            curwin_mut().w_cursor.lnum = lnum_found;
            curwin_mut().w_cursor.col = 0;
            retval = true;
        } else {
            break;
        }
    }

    retval
}

/// Init the fold info in a new window.
pub fn fold_init_win(new_win: &mut WinInfoInfo) {
    new_win.w_folds = Vec::new();
}

/// Find an entry in the win->w_lines[] array for buffer line "lnum".
pub fn find_wl_entry(win: &mut WinInfoInfo, lnum: Linenr) -> i32 {
    let valid_len = win.w_lines_valid.max(0) as usize;
    for (i, wl) in (&win.w_lines)[..valid_len].iter().enumerate() {
        if wl.wl_valid {
            if lnum < wl.wl_lnum {
                return -1;
            }
            if lnum <= wl.wl_foldend {
                return i as i32;
            }
        }
    }
    -1
}

/// Adjust the Visual area to include any fold at the start or end completely.
pub fn fold_adjust_visual() {
    if !g_st().visual_global.active || has_any_folding(curwin_mut()) == 0 {
        return;
    }

    let visual_before_cursor = ltoreq(g_st().visual_global.start, curwin_mut().w_cursor);
    let (mut start, mut end) = if visual_before_cursor {
        (g_st().visual_global.start, curwin_mut().w_cursor)
    } else {
        (curwin_mut().w_cursor, g_st().visual_global.start)
    };
    if has_folding(curwin_mut(), start.lnum, Some(&mut start.lnum), None) {
        start.col = 0;
        if visual_before_cursor {
            g_st().visual_global.start = start;
        } else {
            curwin_mut().w_cursor = start;
        }
    }

    if !has_folding(curwin_mut(), end.lnum, None, Some(&mut end.lnum)) {
        return;
    }

    let mut end_col = ml_get_len(end.lnum);
    if end_col > 0 && opt_vars().p_sel.first_byte() == b'o' {
        end_col -= 1;
    }
    end.col = end_col;
    if visual_before_cursor {
        curwin_mut().w_cursor = end;
    } else {
        g_st().visual_global.start = end;
    }
    mb_adjust_cursor();
}

/// Move the cursor to the first line of a closed fold.
pub fn fold_adjust_cursor(wp: &mut WinInfoInfo) {
    let mut lnum = wp.w_cursor.lnum;
    has_folding(wp, lnum, Some(&mut lnum), None);
    wp.w_cursor.lnum = lnum;
}

fn fold_find(gap: &[FoldEntry], lnum: Linenr) -> (bool, usize) {
    let mut low: usize = 0;
    let mut high: isize = gap.len() as isize - 1;
    while (low as isize) <= high {
        let i = ((low as isize + high) / 2) as usize;
        let entry = &gap[i];
        if entry.fd_top > lnum {
            high = i as isize - 1;
        } else if entry.fd_top + entry.fd_len <= lnum {
            low = i + 1;
        } else {
            return (true, i);
        }
    }
    (false, low)
}

fn fold_level_win(wp: &WinInfoInfo, lnum: Linenr) -> i32 {
    let mut lnum_rel = lnum;
    let mut level: i32 = 0;

    let mut gap: &[FoldEntry] = &wp.w_folds;
    while let (true, idx) = fold_find(gap, lnum_rel) {
        lnum_rel -= gap[idx].fd_top;
        gap = &gap[idx].fd_nested;
        level += 1;
    }

    level
}

fn checkupdate(wp: &mut WinInfoInfo) {
    if !wp.w_foldinvalid {
        return;
    }

    fold_update(wp, 1, MAXLNUM);
    wp.w_foldinvalid = false;
}

fn set_fold_repeat(pos: CursorPos, count: i32, do_open: bool) {
    for n in 0..count {
        let mut done = DONE_NOTHING;
        set_manual_fold(pos, do_open, false, Some(&mut done));
        if (done & DONE_ACTION) == 0 {
            if n == 0 && (done & DONE_FOLD) == 0 {
                emsg(gettext(E_NOFOLD));
            }
            break;
        }
    }
}

fn set_manual_fold(
    pos: CursorPos,
    opening: bool,
    recurse: bool,
    donep: Option<&mut i32>,
) -> Linenr {
    if foldmethod_is_diff(curwin_mut()) && curwin_mut().w_onebuf_opt.wo_scb.get() {
        for wp in for_all_windows_ptr() {
            if wp != curwin_ptr() && foldmethod_is_diff(&mut *wp) && wp.w_onebuf_opt.wo_scb.get() {
                let dlnum = diff_lnum_win(curwin_mut().w_cursor.lnum, &mut *wp);
                if dlnum != 0 {
                    set_manual_fold_win(&mut *wp, dlnum, opening, recurse, None);
                }
            }
        }
    }

    set_manual_fold_win(curwin_mut(), pos.lnum, opening, recurse, donep)
}

fn set_manual_fold_win(
    wp: &mut WinInfoInfo,
    mut lnum: Linenr,
    opening: bool,
    recurse: bool,
    mut donep: Option<&mut i32>,
) -> Linenr {
    let mut level: i32 = 0;
    let mut use_level = false;
    let mut found_fold = false;
    let mut next = MAXLNUM;
    let mut off: Linenr = 0;
    let mut done: i32 = 0;

    checkupdate(wp);

    let wo_fdl = wp.w_onebuf_opt.wo_fdl.get();
    let mut gap: &mut [FoldEntry] = &mut wp.w_folds;
    loop {
        let (found_in_gap, idx) = fold_find(gap, lnum);
        if !found_in_gap {
            if idx < gap.len() {
                next = gap[idx].fd_top + off;
            }
            break;
        }

        found_fold = true;

        if idx + 1 < gap.len() {
            next = gap[idx + 1].fd_top + off;
        }

        let fp = &mut gap[idx];
        if use_level || fp.fd_flags == FD_LEVEL {
            use_level = true;
            fp.fd_flags = if i64::from(level) >= wo_fdl {
                FD_CLOSED
            } else {
                FD_OPEN
            };
            for child in &mut fp.fd_nested {
                child.fd_flags = FD_LEVEL;
            }
        }

        if !opening && recurse {
            if fp.fd_flags != FD_CLOSED {
                done |= DONE_ACTION;
                fp.fd_flags = FD_CLOSED;
            }
        } else if fp.fd_flags == FD_CLOSED {
            if opening {
                fp.fd_flags = FD_OPEN;
                done |= DONE_ACTION;
                if recurse {
                    fold_open_nested(&mut *fp);
                }
            }
            break;
        }

        lnum -= fp.fd_top;
        off += fp.fd_top;
        let (child_found, child_idx) = fold_find(&fp.fd_nested, lnum);
        if !child_found {
            if !opening {
                fp.fd_flags = FD_CLOSED;
                done |= DONE_ACTION;
            }
            if child_idx < fp.fd_nested.len() {
                next = fp.fd_nested[child_idx].fd_top + off;
            }
            break;
        }
        gap = &mut fp.fd_nested;
        level += 1;
    }
    if found_fold {
        wp.w_fold_manual = true;
        if (done & DONE_ACTION) != 0 {
            changed_window_setting(&mut *wp);
        }
        done |= DONE_FOLD;
    } else if donep.is_none() && wp == curwin_ptr() {
        emsg(gettext(E_NOFOLD));
    }

    if let Some(ref mut p) = donep {
        **p |= done;
    }

    next
}

fn fold_open_nested(fpr: &mut FoldEntry) {
    for child in &mut fpr.fd_nested {
        fold_open_nested(child);
        child.fd_flags = FD_OPEN;
    }
}

fn delete_fold_entry(gap: &mut Vec<FoldEntry>, idx: i32, recursive: bool) {
    let idx = idx as usize;
    if recursive || gap[idx].fd_nested.is_empty() {
        gap.remove(idx);
    } else {
        let parent = gap.remove(idx);
        let mut nested = parent.fd_nested;
        for child in &mut nested {
            child.fd_top += parent.fd_top;
            if parent.fd_flags == FD_LEVEL {
                child.fd_flags = FD_LEVEL;
            }
            if parent.fd_small == K_NONE {
                child.fd_small = K_NONE;
            }
        }
        gap.splice(idx..idx, nested);
    }
}

/// Update line numbers of folds for inserted/deleted lines.
pub fn fold_mark_adjust(
    wp: &mut WinInfoInfo,
    mut line1: Linenr,
    mut line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    if amount == MAXLNUM && line2 >= line1 && line2 - line1 >= -amount_after {
        line2 = line1 - amount_after - 1;
    }
    if line2 < line1 {
        line2 = line1;
    }
    if (g_st().state_global & MODE_INSERT) != 0 && amount == 1 && line2 == MAXLNUM {
        line1 -= 1;
    }
    fold_mark_adjust_recurse(&mut wp.w_folds, line1, line2, amount, amount_after);
}

fn fold_mark_adjust_recurse(
    gap: &mut Vec<FoldEntry>,
    line1: Linenr,
    line2: Linenr,
    amount: Linenr,
    amount_after: Linenr,
) {
    if gap.is_empty() {
        return;
    }

    let top = if (g_st().state_global & MODE_INSERT) != 0 && amount == 1 && line2 == MAXLNUM {
        line1 + 1
    } else {
        line1
    };

    let (_, mut i) = fold_find(gap, line1);

    while i < gap.len() {
        let last = gap[i].fd_top + gap[i].fd_len - 1;

        if last < line1 {
            i += 1;
            continue;
        }

        if gap[i].fd_top > line2 {
            if amount_after == 0 {
                break;
            }
            gap[i].fd_top += amount_after;
        } else if gap[i].fd_top >= top && last <= line2 {
            if amount == MAXLNUM {
                delete_fold_entry(gap, i as i32, true);
                continue;
            } else {
                gap[i].fd_top += amount;
            }
        } else if gap[i].fd_top < top {
            let fp = &mut gap[i];
            fold_mark_adjust_recurse(
                &mut fp.fd_nested,
                line1 - fp.fd_top,
                line2 - fp.fd_top,
                amount,
                amount_after,
            );
            if last <= line2 {
                if amount == MAXLNUM {
                    fp.fd_len = line1 - fp.fd_top;
                } else {
                    fp.fd_len += amount;
                }
            } else {
                fp.fd_len += amount_after;
            }
        } else if amount == MAXLNUM {
            let fp = &mut gap[i];
            fold_mark_adjust_recurse(
                &mut fp.fd_nested,
                0,
                line2 - fp.fd_top,
                amount,
                amount_after + (fp.fd_top - top),
            );
            fp.fd_len -= line2 - fp.fd_top + 1;
            fp.fd_top = line1;
        } else {
            let fp = &mut gap[i];
            fold_mark_adjust_recurse(
                &mut fp.fd_nested,
                0,
                line2 - fp.fd_top,
                amount,
                amount_after - amount,
            );
            fp.fd_len += amount_after - amount;
            fp.fd_top += amount;
        }

        i += 1;
    }
}

/// Get the lowest 'foldlevel' value that makes the deepest nested fold in
/// window `wp`.
pub fn get_deepest_nesting(wp: &mut WinInfoInfo) -> i32 {
    checkupdate(&mut *wp);
    get_deepest_nesting_recurse(&wp.w_folds)
}

fn get_deepest_nesting_recurse(gap: &[FoldEntry]) -> i32 {
    let mut maxlevel: i32 = 0;
    for entry in gap {
        let level = get_deepest_nesting_recurse(&entry.fd_nested) + 1;
        maxlevel = maxlevel.max(level);
    }
    maxlevel
}

fn check_closed(
    wp: &mut WinInfoInfo,
    fp: &mut FoldEntry,
    use_levelp: &mut bool,
    level: i32,
    maybe_smallp: &mut bool,
    lnum_off: Linenr,
) -> bool {
    let mut closed = false;

    if *use_levelp || fp.fd_flags == FD_LEVEL {
        *use_levelp = true;
        if i64::from(level) >= wp.w_onebuf_opt.wo_fdl.get() {
            closed = true;
        }
    } else if fp.fd_flags == FD_CLOSED {
        closed = true;
    }

    if fp.fd_small == K_NONE {
        *maybe_smallp = true;
    }
    if closed {
        if *maybe_smallp {
            fp.fd_small = K_NONE;
        }
        check_small(wp, fp, lnum_off);
        if fp.fd_small == K_TRUE {
            closed = false;
        }
    }
    closed
}

fn check_small(wp: &mut WinInfoInfo, fp: &mut FoldEntry, lnum_off: Linenr) {
    if fp.fd_small != K_NONE {
        return;
    }

    set_small_maybe(&mut fp.fd_nested);

    if i64::from(fp.fd_len) > wp.w_onebuf_opt.wo_fml.get() {
        fp.fd_small = K_FALSE;
    } else {
        let mut count: i32 = 0;
        for n in 0..fp.fd_len {
            count += plines_win_nofold(wp, fp.fd_top + lnum_off + n);
            if i64::from(count) > wp.w_onebuf_opt.wo_fml.get() {
                fp.fd_small = K_FALSE;
                return;
            }
        }
        fp.fd_small = K_TRUE;
    }
}

fn set_small_maybe(gap: &mut [FoldEntry]) {
    for entry in gap {
        entry.fd_small = K_NONE;
    }
}

fn fold_create_markers(wp: &mut WinInfoInfo, start: CursorPos, end: CursorPos) {
    let buf = wp.buf_mut();
    if !buf.b_p_ma.get() {
        emsg(gettext(E_MODIFIABLE));
        return;
    }
    parse_marker(&mut *wp);

    let start_len = FOLD_STATE.with(|st| st.foldstartmarkerlen.get());
    let end_marker = clone_foldendmarker();
    fold_add_marker(buf, start, &wp.w_onebuf_opt.wo_fmr.as_bytes()[..start_len]);
    fold_add_marker(buf, end, &end_marker);

    changed_lines(buf, start.lnum, 0, end.lnum, 0, false);

    let num_changed = 1 + i64::from(end.lnum - start.lnum);
    buf_updates_send_changes(buf, start.lnum, num_changed, num_changed);
}

fn fold_add_marker(buf: &mut FileBuf, pos: CursorPos, marker: &[u8]) {
    let markerlen = marker.len();
    let cms_owned = buf.b_p_cms.to_owned();
    let cms_bytes = cms_owned.as_bytes();
    let p_off_opt = cms_bytes.windows(2).position(|w| w == b"%s");
    let mut line_is_comment = false;
    let lnum = pos.lnum;

    let line = crate::memline::ml_get_buf_slice(buf, lnum);

    if !u_save(lnum - 1, lnum + 1) {
        return;
    }

    skip_comment(line, false, false, &mut line_is_comment);
    let line = crate::memline::ml_get_buf_slice(buf, lnum);
    let line_len = line.len();
    let cms_len = cms_bytes.len();
    let mut newline_vec = Vec::with_capacity(line_len + markerlen + cms_len + 1);
    newline_vec.extend_from_slice(line);
    let added = match p_off_opt {
        Some(p_off) if !line_is_comment => {
            newline_vec.extend_from_slice(&cms_bytes[..p_off]);
            newline_vec.extend_from_slice(marker);
            newline_vec.extend_from_slice(&cms_bytes[p_off + 2..]);
            markerlen + cms_len - 2
        }
        _ => {
            newline_vec.extend_from_slice(marker);
            markerlen
        }
    };
    newline_vec.push(0);
    ml_replace_buf(
        buf,
        lnum,
        crate::memory::bytes_until_nul(&newline_vec),
        true,
        false,
    );
    if added != 0 {
        extmark_splice_cols(
            buf,
            lnum - 1,
            line_len as Colnr,
            0,
            added as Colnr,
            K_EXTMARK_UNDO,
        );
    }
}

fn delete_fold_markers(
    wp: &mut WinInfoInfo,
    fp: &mut FoldEntry,
    recursive: bool,
    lnum_off: Linenr,
) {
    if recursive {
        for child in &mut fp.fd_nested {
            delete_fold_markers(wp, child, true, lnum_off + fp.fd_top);
        }
    }
    let start_len = FOLD_STATE.with(|st| st.foldstartmarkerlen.get());
    let end_marker = clone_foldendmarker();
    fold_del_marker(
        wp.buf_mut(),
        fp.fd_top + lnum_off,
        &wp.w_onebuf_opt.wo_fmr.as_bytes()[..start_len],
    );
    fold_del_marker(
        wp.buf_mut(),
        fp.fd_top + lnum_off + fp.fd_len - 1,
        &end_marker,
    );
}

fn fold_del_marker(buf: &mut FileBuf, lnum: Linenr, marker_slice: &[u8]) {
    let markerlen = marker_slice.len();
    if lnum > buf.b_ml.ml_line_count {
        return;
    }

    let cms_owned = buf.b_p_cms.to_owned();
    let cms_bytes = cms_owned.as_bytes();
    let line_slice = crate::memline::ml_get_buf_slice(buf, lnum);
    let mut idx = 0;
    while idx < line_slice.len() {
        let rem = &line_slice[idx..];
        if !rem.starts_with(marker_slice) {
            idx += 1;
            continue;
        }
        let mut len = markerlen;
        if rem.get(len).is_some_and(u8::is_ascii_digit) {
            len += 1;
        }
        if !cms_bytes.is_empty()
            && let Some(cms_pos) = cms_bytes.windows(2).position(|w| w == b"%s")
        {
            let prefix = &cms_bytes[..cms_pos];
            let suffix = &cms_bytes[cms_pos + 2..];
            if idx >= prefix.len()
                && &line_slice[idx - prefix.len()..idx] == prefix
                && line_slice[idx + len..].starts_with(suffix)
            {
                idx -= prefix.len();
                len += cms_bytes.len() - 2;
            }
        }
        if u_save(lnum - 1, lnum + 1) {
            let mut newline_vec = Vec::with_capacity(line_slice.len() - len + 1);
            newline_vec.extend_from_slice(&line_slice[..idx]);
            newline_vec.extend_from_slice(&line_slice[idx + len..]);
            newline_vec.push(0);
            ml_replace_buf(
                buf,
                lnum,
                crate::memory::bytes_until_nul(&newline_vec),
                true,
                false,
            );
            extmark_splice_cols(buf, lnum - 1, idx as Colnr, len as Colnr, 0, K_EXTMARK_UNDO);
        }
        break;
    }
}

/// Generates text to display.
pub fn get_foldtext(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    lnume: Linenr,
    foldinfo: Foldinfo,
    vt: &mut VirtText,
) -> std::ffi::CString {
    let mut text: Option<std::ffi::CString> = None;
    let save_did_emsg = g_st().did_emsg;

    let mut got_fdt_error = FOLD_STATE.with(|st| {
        if st.last_wp.get() != wp || st.fdt_last_lnum.get() > lnum || st.fdt_last_lnum.get() == 0 {
            st.got_fdt_error.set(false);
        }
        st.got_fdt_error.get()
    });

    if !got_fdt_error {
        g_st().did_emsg = 0;
    }

    if wp.w_onebuf_opt.wo_fdt.cb_type() != K_CALLBACK_NONE_CONST {
        let mut dashes = [0u8; (MAX_LEVEL as usize) + 2];

        set_vim_var_nr(VV_FOLDSTART, Varnumber::from(lnum));
        set_vim_var_nr(VV_FOLDEND, Varnumber::from(lnume));

        let level = foldinfo.fi_level.min((dashes.len() - 1) as i32).max(0);
        dashes[..level as usize].fill(b'-');
        dashes[level as usize] = 0;
        set_vim_var_string(
            VV_FOLDDASHES,
            Some(crate::memory::bytes_as_cstr(&dashes)),
            level as isize,
        );
        set_vim_var_nr(VV_FOLDLEVEL, Varnumber::from(level));

        if !got_fdt_error {
            let save_curwin = curwin_ptr();
            let saved_sctx = g_st().current_sctx;

            set_curwin(&raw mut *wp);
            g_st().curbuf = wp.w_buffer;
            g_st().current_sctx = wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_FOLDTEXT].get();

            g_st().emsg_off += 1;

            let obj = eval_foldtext(wp);
            if obj.obj_type == ObjectType::ObjectTypeArray {
                let mut err = Error::new();
                *vt = parse_virt_text(obj.data.array(), &mut err, None, false);
                if err.err_type == ErrorType::ErrorTypeNone {
                    text = Some(std::ffi::CString::default());
                }
                api_clear_error(&mut err);
            } else if obj.obj_type == ObjectType::ObjectTypeString {
                text = (!obj.data.string().data.is_null()).then(|| obj.data.string().to_cstring());
            }
            api_free_object(obj);

            g_st().emsg_off -= 1;

            if text.is_none() || g_st().did_emsg != 0 {
                got_fdt_error = true;
                FOLD_STATE.with(|st| st.got_fdt_error.set(true));
            }

            set_curwin(save_curwin);
            g_st().curbuf = curwin_mut().w_buffer;
            g_st().current_sctx = saved_sctx;
        }
        let _ = got_fdt_error;
        let wp_ptr = crate::globals::WinPtr::from(&mut *wp);
        FOLD_STATE.with(|st| {
            st.fdt_last_lnum.set(lnum);
            st.last_wp.set(wp_ptr);
        });
        set_vim_var_string(VV_FOLDDASHES, None, -1);

        if g_st().did_emsg == 0 && save_did_emsg != 0 {
            g_st().did_emsg = save_did_emsg;
        }

        if let Some(t) = text.take() {
            let mut bytes = t.into_bytes_with_nul();
            let mut idx = 0;
            let slen = bytes.len() - 1;
            while idx < slen && bytes[idx] != 0 {
                let len = utfc_ptr2len(&bytes[idx..slen]) as usize;
                if len > 1 {
                    if !vim_isprintc(utf_ptr2char(&bytes[idx..slen])) {
                        break;
                    }
                    idx += len;
                } else if bytes[idx] == TAB {
                    bytes[idx] = b' ';
                    idx += 1;
                } else if ptr2cells(&bytes[idx..]) > 1 {
                    break;
                } else {
                    idx += 1;
                }
            }
            text = if idx < slen && bytes[idx] != 0 {
                Some(transstr(&bytes, true))
            } else {
                Some(CStr::from_bytes_with_nul(&bytes).unwrap().to_owned())
            };
        }
    }
    text.unwrap_or_else(|| {
        let mut buf = [0u8; FOLD_TEXT_LEN];
        let count = lnume - lnum + 1;
        format_to_buf!(
            &mut buf,
            ngettext(c"+--%3d line folded", c"+--%3d lines folded ", count as u64),
            count,
        );
        crate::memory::cstring_until_nul(&buf)
    })
}

fn foldtext_cleanup(str_slice: &mut [u8]) {
    let cms_raw = curbuf_mut().b_p_cms.as_bytes();
    let mut cms_trimmed = crate::charset::skipwhite_slice(cms_raw);
    while let Some(&last) = cms_trimmed.last()
        && ascii_iswhite(i32::from(last))
    {
        cms_trimmed = &cms_trimmed[..cms_trimmed.len() - 1];
    }

    let (cms_start_slice, cms_end_slice) =
        if let Some(idx) = cms_trimmed.windows(2).position(|w| w == b"%s") {
            let mut head = &cms_trimmed[..idx];
            while let Some(&last) = head.last()
                && ascii_iswhite(i32::from(last))
            {
                head = &head[..head.len() - 1];
            }
            let tail_raw = &cms_trimmed[idx + 2..];
            (head, Some(crate::charset::skipwhite_slice(tail_raw)))
        } else {
            (cms_trimmed, None)
        };
    parse_marker(curwin_mut());

    let mut did1 = false;
    let mut did2 = false;

    let (start_len, end_len) =
        FOLD_STATE.with(|st| (st.foldstartmarkerlen.get(), st.foldendmarkerlen.get()));
    let fmr_slice = &curwin_mut().w_onebuf_opt.wo_fmr.as_bytes()[..start_len];
    let end_vec = clone_foldendmarker();
    let end_slice = end_vec.as_slice();

    let mut total_len = crate::memory::bytes_until_nul(str_slice).len();
    let mut idx = 0;
    while idx < total_len {
        let s_bytes = &str_slice[idx..total_len];
        let mut len: usize = 0;
        if s_bytes.starts_with(fmr_slice) {
            len = start_len;
        } else if s_bytes.starts_with(end_slice) {
            len = end_len;
        }
        if len > 0 {
            if s_bytes
                .get(len)
                .is_some_and(|&b| ascii_isdigit(i32::from(b)))
            {
                len += 1;
            }

            let mut p_idx = idx;
            while p_idx > 0 && ascii_iswhite(i32::from(str_slice[p_idx - 1])) {
                p_idx -= 1;
            }
            if p_idx >= cms_start_slice.len()
                && &str_slice[p_idx - cms_start_slice.len()..p_idx] == cms_start_slice
            {
                len += (idx - p_idx) + cms_start_slice.len();
                idx = p_idx - cms_start_slice.len();
            }
        } else if let Some(cms_end_bytes) = cms_end_slice {
            if !did1 && !cms_start_slice.is_empty() && s_bytes.starts_with(cms_start_slice) {
                len = cms_start_slice.len();
                did1 = true;
            } else if !did2 && !cms_end_bytes.is_empty() && s_bytes.starts_with(cms_end_bytes) {
                len = cms_end_bytes.len();
                did2 = true;
            }
        }
        if len != 0 {
            while idx + len < total_len && ascii_iswhite(i32::from(str_slice[idx + len])) {
                len += 1;
            }
            str_slice.copy_within(idx + len..=total_len, idx);
            total_len -= len;
        } else {
            idx += crate::mbyte::utfc_slice2len(&str_slice[idx..total_len]);
        }
    }
}

fn call_getlevel(kind: LevelGetterKind, flp: &mut Fline) {
    match kind {
        LevelGetterKind::Marker => foldlevel_marker(flp),
        LevelGetterKind::Expr => foldlevel_expr(flp),
        LevelGetterKind::Syntax => foldlevel_syntax(flp),
        LevelGetterKind::Diff => foldlevel_diff(flp),
        LevelGetterKind::Indent => foldlevel_indent(flp),
    }
}

fn fold_update_iems(wp: &mut WinInfoInfo, mut top: Linenr, mut bot: Linenr) {
    if FOLD_STATE.with(|st| st.invalid_top.get()) != 0 {
        return;
    }

    if wp.w_foldinvalid {
        top = 1;
        bot = wp.buf_mut().b_ml.ml_line_count;
        wp.w_foldinvalid = false;

        set_small_maybe(&mut wp.w_folds);
    }

    if foldmethod_is_diff(wp) {
        if top > g_st().diff_context {
            top -= g_st().diff_context;
        } else {
            top = 1;
        }
        bot += g_st().diff_context;
    }

    top = top.min(wp.buf_mut().b_ml.ml_line_count);

    FOLD_STATE.with(|st| st.fold_changed.set(false));
    let mut fline = Fline {
        wp: crate::globals::WinPtr::from(&mut *wp),
        lnum: 0,
        off: 0,
        lnum_save: 0,
        lvl: 0,
        lvl_next: -1,
        start: 0,
        end: MAX_LEVEL + 1,
        had_end: MAX_LEVEL + 1,
    };

    FOLD_STATE.with(|st| {
        st.invalid_top.set(top);
        st.invalid_bot.set(bot);
    });

    let getlevel: LevelGetterKind;

    if foldmethod_is_marker(wp) {
        getlevel = LevelGetterKind::Marker;

        parse_marker(wp);

        if top > 1 {
            let level = fold_level_win(wp, top - 1);

            fline.lnum = top - 1;
            fline.lvl = level;
            call_getlevel(getlevel, &mut fline);

            if fline.lvl > level {
                fline.lvl = level - (fline.lvl - fline.lvl_next);
            } else {
                fline.lvl = fline.lvl_next;
            }
        }
        fline.lnum = top;
        call_getlevel(getlevel, &mut fline);
    } else {
        fline.lnum = top;
        if foldmethod_is_expr(wp) {
            getlevel = LevelGetterKind::Expr;
            if top > 1 {
                fline.lnum -= 1;
            }
        } else if foldmethod_is_syntax(wp) {
            getlevel = LevelGetterKind::Syntax;
        } else if foldmethod_is_diff(wp) {
            getlevel = LevelGetterKind::Diff;
        } else {
            getlevel = LevelGetterKind::Indent;
            if top > 1 {
                fline.lnum -= 1;
            }
        }

        fline.lvl = -1;
        while !g_st().got_int {
            fline.lvl_next = -1;
            call_getlevel(getlevel, &mut fline);
            if fline.lvl >= 0 {
                break;
            }
            fline.lnum -= 1;
        }
    }

    if getlevel == LevelGetterKind::Syntax {
        let mut gap: &[FoldEntry] = &wp.w_folds;
        let mut fpn: Option<&FoldEntry> = None;
        let mut current_fdl: i32 = 0;
        let mut fold_start_lnum: Linenr = 0;
        let mut lnum_rel = fline.lnum;

        while current_fdl < fline.lvl {
            let (found, idx) = fold_find(gap, lnum_rel);
            if !found {
                break;
            }
            current_fdl += 1;

            let entry = &gap[idx];
            fold_start_lnum += entry.fd_top;
            gap = &entry.fd_nested;
            lnum_rel -= entry.fd_top;
            fpn = Some(entry);
        }
        if let Some(entry) = fpn
            && current_fdl == fline.lvl
        {
            let fold_end_lnum = fold_start_lnum + entry.fd_len;
            bot = bot.max(fold_end_lnum);
        }
    }

    let mut start = fline.lnum;
    let mut end = bot;
    if start > end && end < wp.buf_mut().b_ml.ml_line_count {
        end = start;
    }

    while !g_st().got_int {
        if fline.lnum > wp.buf_mut().b_ml.ml_line_count {
            break;
        }
        if fline.lnum > end {
            if getlevel != LevelGetterKind::Marker
                && getlevel != LevelGetterKind::Syntax
                && getlevel != LevelGetterKind::Expr
            {
                break;
            }
            let folds = &wp.w_folds;
            if start <= end
                && let (true, idx) = fold_find(folds, end)
                && folds[idx].fd_top + folds[idx].fd_len - 1 > end
            {
                end = folds[idx].fd_top + folds[idx].fd_len - 1;
            } else if fline.lvl == 0
                && let (true, idx) = fold_find(folds, fline.lnum)
                && folds[idx].fd_top < fline.lnum
            {
                end = folds[idx].fd_top + folds[idx].fd_len - 1;
            } else if getlevel == LevelGetterKind::Syntax
                && fold_level_win(wp, fline.lnum) != fline.lvl
            {
                end = fline.lnum;
            } else {
                break;
            }
        }

        if fline.lvl > 0 {
            FOLD_STATE.with(|st| {
                st.invalid_top.set(fline.lnum);
                st.invalid_bot.set(end);
            });
            end = fold_update_iemsrecurse(
                &mut wp.w_folds,
                1,
                start,
                &mut fline,
                getlevel,
                end,
                FD_LEVEL,
            );
            start = fline.lnum;
        } else {
            if fline.lnum == wp.buf_mut().b_ml.ml_line_count {
                break;
            }
            fline.lnum += 1;
            fline.lvl = fline.lvl_next;
            call_getlevel(getlevel, &mut fline);
        }
    }

    fold_remove(&mut wp.w_folds, start, end);

    if FOLD_STATE.with(|st| st.fold_changed.get()) && wp.w_onebuf_opt.wo_fen.get() {
        changed_window_setting(wp);
    }

    if end != bot {
        redraw_win_range_later(wp, top, end);
    }

    FOLD_STATE.with(|st| st.invalid_top.set(0));
}

fn fold_update_iemsrecurse(
    gap: &mut Vec<FoldEntry>,
    level: i32,
    startlnum: Linenr,
    flp: &mut Fline,
    getlevel: LevelGetterKind,
    mut bot: Linenr,
    topflags: u8,
) -> Linenr {
    let mut fp_idx: Option<usize> = None;

    if getlevel == LevelGetterKind::Marker && flp.start <= flp.lvl - level && flp.lvl > 0 {
        let (_, idx) = fold_find(gap, startlnum - 1);
        if idx < gap.len() && gap[idx].fd_top < startlnum {
            fp_idx = Some(idx);
        }
    }

    let mut lvl = level;
    let mut startlnum2 = startlnum;
    let firstlnum = flp.lnum;
    let mut finish = false;
    let linecount = flp.wp_mut().buf_mut().b_ml.ml_line_count - flp.off;

    flp.lnum_save = flp.lnum;
    while !g_st().got_int {
        line_breakcheck();

        lvl = flp.lvl.min(MAX_LEVEL);
        if flp.lnum > firstlnum && (level > lvl - flp.start || level >= flp.had_end) {
            lvl = 0;
        }

        if flp.lnum > bot
            && !finish
            && let Some(fpi) = fp_idx
        {
            if getlevel != LevelGetterKind::Marker
                && getlevel != LevelGetterKind::Expr
                && getlevel != LevelGetterKind::Syntax
            {
                break;
            }
            let mut i: i32 = 0;
            if lvl >= level {
                let mut ll = flp.lnum - gap[fpi].fd_top;
                let mut cur_nested: &[FoldEntry] = &gap[fpi].fd_nested;
                while let (true, ni) = fold_find(cur_nested, ll) {
                    i += 1;
                    ll -= cur_nested[ni].fd_top;
                    cur_nested = &cur_nested[ni].fd_nested;
                }
            }
            if lvl < level + i {
                let (_, ni) = fold_find(&gap[fpi].fd_nested, flp.lnum - gap[fpi].fd_top);
                if ni < gap[fpi].fd_nested.len() {
                    let f2 = &gap[fpi].fd_nested[ni];
                    bot = f2.fd_top + f2.fd_len - 1 + gap[fpi].fd_top;
                }
            } else if gap[fpi].fd_top + gap[fpi].fd_len <= flp.lnum && lvl >= level {
                finish = true;
            } else {
                break;
            }
        }

        if fp_idx.is_none()
            && (lvl != level
                || flp.lnum_save >= bot
                || flp.start != 0
                || flp.had_end <= MAX_LEVEL
                || flp.lnum == linecount)
        {
            while !g_st().got_int {
                let concat: Linenr = if flp.start != 0 || flp.had_end <= MAX_LEVEL {
                    0
                } else {
                    1
                };

                let mut cand_idx = 0;
                let has_cand = !gap.is_empty() && {
                    let (found1, idx1) = fold_find(gap, startlnum);
                    cand_idx = idx1;
                    if found1 || (cand_idx < gap.len() && gap[cand_idx].fd_top <= firstlnum) {
                        true
                    } else {
                        let (found2, idx2) = fold_find(gap, firstlnum - concat);
                        cand_idx = idx2;
                        found2
                            || (cand_idx < gap.len()
                                && ((lvl < level && gap[cand_idx].fd_top < flp.lnum)
                                    || (lvl >= level && gap[cand_idx].fd_top <= flp.lnum_save)))
                    }
                };

                if has_cand {
                    if gap[cand_idx].fd_top + gap[cand_idx].fd_len + concat > firstlnum {
                        let mut fpi = cand_idx;
                        if gap[fpi].fd_top == firstlnum {
                            // exact match
                        } else if gap[fpi].fd_top >= startlnum {
                            let fp = &mut gap[fpi];
                            if fp.fd_top > firstlnum {
                                fold_mark_adjust_recurse(
                                    &mut fp.fd_nested,
                                    0,
                                    MAXLNUM,
                                    fp.fd_top - firstlnum,
                                    0,
                                );
                            } else {
                                fold_mark_adjust_recurse(
                                    &mut fp.fd_nested,
                                    0,
                                    firstlnum - fp.fd_top - 1,
                                    MAXLNUM,
                                    fp.fd_top - firstlnum,
                                );
                            }
                            fp.fd_len += fp.fd_top - firstlnum;
                            fp.fd_top = firstlnum;
                            fp.fd_small = K_NONE;
                            FOLD_STATE.with(|st| st.fold_changed.set(true));
                        } else if (flp.start != 0 && lvl == level) || firstlnum != startlnum {
                            let (breakstart, breakend) = if firstlnum != startlnum {
                                (startlnum, firstlnum)
                            } else {
                                (flp.lnum, flp.lnum)
                            };
                            let fp_top = gap[fpi].fd_top;
                            fold_remove(
                                &mut gap[fpi].fd_nested,
                                breakstart - fp_top,
                                breakend - fp_top,
                            );
                            fold_split(gap, fpi as i32, breakstart, breakend - 1);
                            fpi += 1;
                            if getlevel == LevelGetterKind::Marker
                                || getlevel == LevelGetterKind::Expr
                                || getlevel == LevelGetterKind::Syntax
                            {
                                finish = true;
                            }
                        }
                        if gap[fpi].fd_top == startlnum
                            && concat != 0
                            && fpi != 0
                            && gap[fpi - 1].fd_top + gap[fpi - 1].fd_len == gap[fpi].fd_top
                        {
                            fold_merge(gap, fpi - 1);
                            fpi -= 1;
                        }
                        fp_idx = Some(fpi);
                        break;
                    }
                    if gap[cand_idx].fd_top >= startlnum {
                        delete_fold_entry(gap, cand_idx as i32, true);
                    } else {
                        let fp = &mut gap[cand_idx];
                        fp.fd_len = startlnum - fp.fd_top;
                        fold_mark_adjust_recurse(&mut fp.fd_nested, fp.fd_len, MAXLNUM, MAXLNUM, 0);
                        FOLD_STATE.with(|st| st.fold_changed.set(true));
                    }
                } else {
                    let i = cand_idx;
                    fold_insert(gap, i as i32);
                    let prev_flags = if i > 0 { gap[i - 1].fd_flags } else { 0 };
                    let fp = &mut gap[i];
                    fp.fd_top = firstlnum;
                    fp.fd_len = bot - firstlnum + 1;
                    if topflags == FD_OPEN {
                        flp.wp_mut().w_fold_manual = true;
                        fp.fd_flags = FD_OPEN;
                    } else if i == 0 {
                        fp.fd_flags = topflags;
                        if topflags != FD_LEVEL {
                            flp.wp_mut().w_fold_manual = true;
                        }
                    } else {
                        fp.fd_flags = prev_flags;
                    }
                    fp.fd_small = K_NONE;
                    if getlevel == LevelGetterKind::Marker
                        || getlevel == LevelGetterKind::Expr
                        || getlevel == LevelGetterKind::Syntax
                    {
                        finish = true;
                    }
                    FOLD_STATE.with(|st| st.fold_changed.set(true));
                    fp_idx = Some(i);
                    break;
                }
            }
        }

        if lvl < level || flp.lnum > linecount {
            break;
        }

        if lvl > level
            && let Some(fpi) = fp_idx
        {
            bot = bot.max(flp.lnum);

            let fp_top = gap[fpi].fd_top;
            let fp_flags = gap[fpi].fd_flags;
            flp.lnum = flp.lnum_save - fp_top;
            flp.off += fp_top;
            bot = fold_update_iemsrecurse(
                &mut gap[fpi].fd_nested,
                level + 1,
                startlnum2 - fp_top,
                flp,
                getlevel,
                bot - fp_top,
                fp_flags,
            );
            let fp_top = gap[fpi].fd_top;
            flp.lnum += fp_top;
            flp.lnum_save += fp_top;
            flp.off -= fp_top;
            bot += fp_top;
            startlnum2 = flp.lnum;
        } else {
            flp.lnum = flp.lnum_save;
            let ll = flp.lnum + 1;
            while !g_st().got_int {
                FOLD_STATE.with(|st| {
                    st.prev_lnum.set(flp.lnum);
                    st.prev_lnum_lvl.set(flp.lvl);
                });

                flp.lnum += 1;
                if flp.lnum > linecount {
                    break;
                }
                flp.lvl = flp.lvl_next;
                call_getlevel(getlevel, flp);
                if flp.lvl >= 0 || flp.had_end <= MAX_LEVEL {
                    break;
                }
            }
            FOLD_STATE.with(|st| st.prev_lnum.set(0));
            if flp.lnum > linecount {
                break;
            }

            flp.lnum_save = flp.lnum;
            flp.lnum = ll;
        }
    }

    let Some(fpi) = fp_idx else {
        return bot;
    };

    if gap[fpi].fd_len < flp.lnum - gap[fpi].fd_top {
        gap[fpi].fd_len = flp.lnum - gap[fpi].fd_top;
        gap[fpi].fd_small = K_NONE;
        FOLD_STATE.with(|st| st.fold_changed.set(true));
    } else if gap[fpi].fd_top + gap[fpi].fd_len > linecount {
        gap[fpi].fd_len = linecount - gap[fpi].fd_top + 1;
    }

    let fp_top = gap[fpi].fd_top;
    fold_remove(
        &mut gap[fpi].fd_nested,
        startlnum2 - fp_top,
        flp.lnum - 1 - fp_top,
    );

    if lvl < level && gap[fpi].fd_len != flp.lnum - gap[fpi].fd_top {
        if gap[fpi].fd_top + gap[fpi].fd_len - 1 > bot {
            if getlevel == LevelGetterKind::Marker
                || getlevel == LevelGetterKind::Expr
                || getlevel == LevelGetterKind::Syntax
            {
                bot = gap[fpi].fd_top + gap[fpi].fd_len - 1;
                gap[fpi].fd_len = flp.lnum - gap[fpi].fd_top;
            } else {
                fold_split(gap, fpi as i32, flp.lnum, bot);
            }
        } else {
            gap[fpi].fd_len = flp.lnum - gap[fpi].fd_top;
        }
        FOLD_STATE.with(|st| st.fold_changed.set(true));
    }

    loop {
        let next_idx = fpi + 1;
        if next_idx >= gap.len() || gap[next_idx].fd_top > flp.lnum {
            break;
        }
        if gap[next_idx].fd_top + gap[next_idx].fd_len > flp.lnum {
            if gap[next_idx].fd_top < flp.lnum {
                let fp2 = &mut gap[next_idx];
                fold_mark_adjust_recurse(
                    &mut fp2.fd_nested,
                    0,
                    flp.lnum - fp2.fd_top - 1,
                    MAXLNUM,
                    fp2.fd_top - flp.lnum,
                );
                fp2.fd_len -= flp.lnum - fp2.fd_top;
                fp2.fd_top = flp.lnum;
                FOLD_STATE.with(|st| st.fold_changed.set(true));
            }

            if lvl >= level {
                fold_merge(gap, fpi);
            }
            break;
        }
        FOLD_STATE.with(|st| st.fold_changed.set(true));
        delete_fold_entry(gap, next_idx as i32, true);
    }

    bot = bot.max(flp.lnum - 1);
    bot
}

fn fold_insert(gap: &mut Vec<FoldEntry>, i: i32) {
    gap.insert(
        i as usize,
        FoldEntry {
            fd_top: 0,
            fd_len: 0,
            fd_nested: Vec::new(),
            fd_flags: 0,
            fd_small: K_NONE,
        },
    );
}

fn fold_split(gap: &mut Vec<FoldEntry>, i: i32, top: Linenr, bot: Linenr) {
    fold_insert(gap, i + 1);

    let gap_slice = gap.as_mut_slice();
    let (left, right) = gap_slice.split_at_mut((i + 1) as usize);
    let fp = &mut left[i as usize];
    let next_fp = &mut right[0];
    next_fp.fd_top = bot + 1;
    debug_assert!(next_fp.fd_top > bot);
    next_fp.fd_len = fp.fd_len - (next_fp.fd_top - fp.fd_top);
    next_fp.fd_flags = fp.fd_flags;
    next_fp.fd_small = K_NONE;
    fp.fd_small = K_NONE;

    let delta = next_fp.fd_top - fp.fd_top;
    let (_, split_idx) = fold_find(&fp.fd_nested, bot + 1 - fp.fd_top);
    if split_idx < fp.fd_nested.len() {
        for mut child in fp.fd_nested.drain(split_idx..) {
            child.fd_top -= delta;
            next_fp.fd_nested.push(child);
        }
    }
    fp.fd_len = top - fp.fd_top;
    FOLD_STATE.with(|st| st.fold_changed.set(true));
}

fn fold_remove(gap: &mut Vec<FoldEntry>, top: Linenr, bot: Linenr) {
    if bot < top {
        return;
    }

    loop {
        if gap.is_empty() {
            break;
        }
        let (found, idx) = fold_find(gap, top);
        if found && gap[idx].fd_top < top {
            let fp_top = gap[idx].fd_top;
            fold_remove(&mut gap[idx].fd_nested, top - fp_top, bot - fp_top);
            if gap[idx].fd_top + gap[idx].fd_len - 1 > bot {
                fold_split(gap, idx as i32, top, bot);
            } else {
                gap[idx].fd_len = top - gap[idx].fd_top;
            }
            FOLD_STATE.with(|st| st.fold_changed.set(true));
            continue;
        }
        if idx >= gap.len() || gap[idx].fd_top > bot {
            break;
        }
        if gap[idx].fd_top >= top {
            FOLD_STATE.with(|st| st.fold_changed.set(true));
            if gap[idx].fd_top + gap[idx].fd_len - 1 > bot {
                let fp = &mut gap[idx];
                fold_mark_adjust_recurse(
                    &mut fp.fd_nested,
                    0,
                    bot - fp.fd_top,
                    MAXLNUM,
                    fp.fd_top - bot - 1,
                );
                fp.fd_len -= bot - fp.fd_top + 1;
                fp.fd_top = bot + 1;
                break;
            }

            delete_fold_entry(gap, idx as i32, true);
        }
    }
}

fn fold_reverse_order(gap: &mut [FoldEntry], start: Linenr, end: Linenr) {
    if start < end {
        gap[(start as usize)..=(end as usize)].reverse();
    }
}

fn truncate_fold(fp: &mut FoldEntry, mut end: Linenr) {
    end += 1;
    fold_remove(&mut fp.fd_nested, end - fp.fd_top, MAXLNUM);
    fp.fd_len = end - fp.fd_top;
}

#[inline]
const fn fold_end(fp: &FoldEntry) -> Linenr {
    fp.fd_top + fp.fd_len - 1
}

/// Move folds within the inclusive range "line1" to "line2" to after "dest".
pub fn fold_move_range(gap: &mut [FoldEntry], line1: Linenr, line2: Linenr, dest: Linenr) {
    let range_len = line2 - line1 + 1;
    let move_len = dest - line2;
    let (at_start, mut idx) = fold_find(gap, line1 - 1);

    if at_start {
        if fold_end(&gap[idx]) > dest {
            let fp = &mut gap[idx];
            fold_move_range(
                &mut fp.fd_nested,
                line1 - fp.fd_top,
                line2 - fp.fd_top,
                dest - fp.fd_top,
            );
            return;
        } else if fold_end(&gap[idx]) > line2 {
            let fp = &mut gap[idx];
            fold_mark_adjust_recurse(
                &mut fp.fd_nested,
                line1 - fp.fd_top,
                line2 - fp.fd_top,
                MAXLNUM,
                -range_len,
            );
            fp.fd_len -= range_len;
        } else {
            truncate_fold(&mut gap[idx], line1 - 1);
        }
        idx += 1;
    }

    if idx >= gap.len() || gap[idx].fd_top > dest {
        return;
    } else if gap[idx].fd_top > line2 {
        while idx < gap.len() && fold_end(&gap[idx]) <= dest {
            gap[idx].fd_top -= range_len;
            idx += 1;
        }
        if idx < gap.len() && gap[idx].fd_top <= dest {
            truncate_fold(&mut gap[idx], dest);
            gap[idx].fd_top -= range_len;
        }
        return;
    } else if fold_end(&gap[idx]) > dest {
        let fp = &mut gap[idx];
        fold_mark_adjust_recurse(
            &mut fp.fd_nested,
            line2 + 1 - fp.fd_top,
            dest - fp.fd_top,
            MAXLNUM,
            -move_len,
        );
        fp.fd_len -= move_len;
        fp.fd_top += move_len;
        return;
    }

    let move_start = idx;
    let mut move_end: usize = 0;
    while idx < gap.len() && gap[idx].fd_top <= dest {
        if gap[idx].fd_top <= line2 {
            if fold_end(&gap[idx]) > line2 {
                truncate_fold(&mut gap[idx], line2);
            }
            gap[idx].fd_top += move_len;
            idx += 1;
            continue;
        }

        if move_end == 0 {
            move_end = idx;
        }

        if fold_end(&gap[idx]) > dest {
            truncate_fold(&mut gap[idx], dest);
        }

        gap[idx].fd_top -= range_len;
        idx += 1;
    }
    let dest_index = idx;

    if move_end == 0 {
        return;
    }
    fold_reverse_order(gap, move_start as Linenr, (dest_index - 1) as Linenr);
    fold_reverse_order(
        gap,
        move_start as Linenr,
        (move_start + dest_index - move_end - 1) as Linenr,
    );
    fold_reverse_order(
        gap,
        (move_start + dest_index - move_end) as Linenr,
        (dest_index - 1) as Linenr,
    );
}

fn fold_merge_into(fp1: &mut FoldEntry, mut fp2: FoldEntry) {
    if let (true, idx3) = fold_find(&fp1.fd_nested, fp1.fd_len - 1)
        && let (true, idx4) = fold_find(&fp2.fd_nested, 0)
    {
        let child2 = fp2.fd_nested.remove(idx4);
        fold_merge_into(&mut fp1.fd_nested[idx3], child2);
    }

    let fp1_len = fp1.fd_len;
    for mut child in fp2.fd_nested {
        child.fd_top += fp1_len;
        fp1.fd_nested.push(child);
    }

    fp1.fd_len += fp2.fd_len;
    FOLD_STATE.with(|st| st.fold_changed.set(true));
}

fn fold_merge(gap: &mut Vec<FoldEntry>, idx1: usize) {
    let fp2 = gap.remove(idx1 + 1);
    fold_merge_into(&mut gap[idx1], fp2);
}

fn foldlevel_indent(flp: &mut Fline) {
    let lnum = flp.lnum + flp.off;
    let buf = flp.wp_mut().buf_mut();
    let s_rem = crate::charset::skipwhite_slice(crate::memline::ml_get_buf_slice(buf, lnum));

    if s_rem.is_empty()
        || crate::strings::vim_haschr(
            flp.wp_mut().w_onebuf_opt.wo_fdi.as_cstr(),
            i32::from(s_rem[0]),
        )
    {
        flp.lvl = if lnum == 1 || lnum == buf.b_ml.ml_line_count {
            0
        } else {
            -1
        };
    } else {
        flp.lvl = get_indent_buf(buf, lnum) / get_sw_value(buf);
    }
    flp.lvl = flp
        .lvl
        .min(flp.wp_mut().w_onebuf_opt.wo_fdn.get().max(0) as i32);
}

fn foldlevel_diff(flp: &mut Fline) {
    flp.lvl = i32::from(diff_infold(flp.wp_mut(), flp.lnum + flp.off));
}

fn foldlevel_expr(flp: &mut Fline) {
    let lnum = flp.lnum + flp.off;

    let win = curwin_ptr();
    set_curwin(flp.wp);
    g_st().curbuf = flp.wp_mut().w_buffer;
    set_vim_var_nr(VV_LNUM, Varnumber::from(lnum));

    flp.start = 0;
    flp.had_end = flp.end;
    flp.end = MAX_LEVEL + 1;
    if lnum <= 1 {
        flp.lvl = 0;
    }

    let save_keytyped = g_st().key_typed;
    let mut c: i32 = 0;
    let n = eval_foldexpr(flp.wp_mut(), &mut c);
    g_st().key_typed = save_keytyped;

    match c as u8 {
        b'a' => {
            if flp.lvl >= 0 {
                flp.lvl += n;
                flp.lvl_next = flp.lvl;
            }
            flp.start = n;
        }
        b's' => {
            if flp.lvl >= 0 {
                if n > flp.lvl {
                    flp.lvl_next = 0;
                } else {
                    flp.lvl_next = flp.lvl - n;
                }
                flp.end = flp.lvl_next + 1;
            }
        }
        b'>' => {
            flp.lvl = n;
            flp.lvl_next = n;
            flp.start = 1;
        }
        b'<' => {
            flp.lvl_next = flp.lvl.min(n - 1);
            flp.end = n;
        }
        b'=' => {
            flp.lvl_next = flp.lvl;
        }
        _ => {
            if n < 0 {
                flp.lvl_next = flp.lvl;
            } else {
                flp.lvl_next = n;
            }
            flp.lvl = n;
        }
    }

    if flp.lvl < 0 {
        if lnum <= 1 {
            flp.lvl = 0;
            flp.lvl_next = 0;
        }
        if lnum == curbuf_mut().b_ml.ml_line_count {
            flp.lvl_next = 0;
        }
    }

    set_curwin(win);
    g_st().curbuf = curwin_mut().w_buffer;
}

fn parse_marker(wp: &mut WinInfoInfo) {
    let fmr = wp.w_onebuf_opt.wo_fmr.as_bytes();
    let comma_idx = fmr.iter().position(|&b| b == b',').unwrap_or(fmr.len());
    let end_vec = fmr.get(comma_idx + 1..).unwrap_or(&[]).to_vec();
    let end_len = end_vec.len();
    FOLD_STATE.with(|st| {
        st.foldstartmarkerlen.set(comma_idx);
        *st.foldendmarker.borrow_mut() = end_vec;
        st.foldendmarkerlen.set(end_len);
    });
}

fn foldlevel_marker(flp: &mut Fline) {
    let start_lvl = flp.lvl;
    let startmarker_full = flp.wp_mut().w_onebuf_opt.wo_fmr.as_bytes();
    let cstart = startmarker_full.first().copied().unwrap_or(0);
    let (start_len, end_len) =
        FOLD_STATE.with(|st| (st.foldstartmarkerlen.get(), st.foldendmarkerlen.get()));
    let end_vec = clone_foldendmarker();
    let end_slice = end_vec.as_slice();
    let cend = end_slice.first().copied().unwrap_or(0);

    let start_rest = &startmarker_full[1..start_len];
    let end_rest = &end_slice[1..];

    flp.start = 0;
    flp.lvl_next = flp.lvl;

    let mut rem = crate::memline::ml_get_buf_slice(flp.wp_mut().buf_mut(), flp.lnum + flp.off);
    while let Some((&s_b, rem_bytes)) = rem.split_first() {
        if s_b == cstart && rem_bytes.starts_with(start_rest) {
            rem = &rem[start_len..];
            if rem.first().is_some_and(u8::is_ascii_digit) {
                let n = crate::charset::parse_i64_prefix(rem) as i32;
                if n > 0 {
                    flp.lvl = n;
                    flp.lvl_next = n;
                    flp.start = (n - start_lvl).max(1);
                }
            } else {
                flp.lvl += 1;
                flp.lvl_next += 1;
                flp.start += 1;
            }
        } else if s_b == cend && rem_bytes.starts_with(end_rest) {
            rem = &rem[end_len..];
            if rem.first().is_some_and(u8::is_ascii_digit) {
                let n = crate::charset::parse_i64_prefix(rem) as i32;
                if n > 0 {
                    flp.lvl = n;
                    flp.lvl_next = (n - 1).min(start_lvl);
                }
            } else {
                flp.lvl_next -= 1;
            }
        } else {
            rem = &rem[crate::mbyte::utfc_slice2len(rem)..];
        }
    }

    flp.lvl_next = flp.lvl_next.max(0);
}

fn foldlevel_syntax(flp: &mut Fline) {
    let lnum = flp.lnum + flp.off;

    flp.lvl = syn_get_foldlevel(flp.wp_mut(), lnum);
    flp.start = 0;
    if lnum < flp.wp_mut().buf_mut().b_ml.ml_line_count {
        let n = syn_get_foldlevel(flp.wp_mut(), lnum + 1);
        if n > flp.lvl {
            flp.start = n - flp.lvl;
            flp.lvl = n;
        }
    }
}

/// Write commands to "fd" to restore the manual folds in window "wp".
pub fn put_folds(fd: &CFile, wp: &mut WinInfoInfo) -> bool {
    if foldmethod_is_manual(wp)
        && (!put_line(fd, c"silent! normal! zE")
            || !put_folds_recurse(fd, &wp.w_folds, 0)
            || !put_line(fd, c"let &fdl = &fdl"))
    {
        return false;
    }

    if wp.w_fold_manual {
        return put_foldopen_recurse(fd, wp, &wp.w_folds, 0);
    }

    true
}

fn put_folds_recurse(fd: &CFile, gap: &[FoldEntry], off: Linenr) -> bool {
    for fp in gap {
        if !put_folds_recurse(fd, &fp.fd_nested, off + fp.fd_top) {
            return false;
        }
        let mut buf = [0u8; 64];
        format_to_buf!(
            &mut buf,
            c"sil! %lld,%lldfold",
            i64::from(fp.fd_top + off),
            i64::from(fp.fd_top + off + fp.fd_len - 1),
        );
        if fd.puts(crate::memory::bytes_as_cstr(&buf)) < 0 || !put_eol(fd) {
            return false;
        }
    }
    true
}

fn put_foldopen_recurse(fd: &CFile, wp: &WinInfoInfo, gap: &[FoldEntry], off: Linenr) -> bool {
    for fp in gap {
        if fp.fd_flags != FD_LEVEL {
            if !fp.fd_nested.is_empty() {
                let mut buf = [0u8; 32];
                format_to_buf!(&mut buf, c"%lld", i64::from(fp.fd_top + off));
                if fd.puts(crate::memory::bytes_as_cstr(&buf)) < 0
                    || !put_eol(fd)
                    || !put_line(fd, c"sil! normal! zo")
                {
                    return false;
                }
                if !put_foldopen_recurse(fd, wp, &fp.fd_nested, off + fp.fd_top) {
                    return false;
                }
                if fp.fd_flags == FD_CLOSED && !put_fold_open_close(fd, fp, off) {
                    return false;
                }
            } else {
                let level = i64::from(fold_level_win(wp, off + fp.fd_top));
                if ((fp.fd_flags == FD_CLOSED && wp.w_onebuf_opt.wo_fdl.get() >= level)
                    || (fp.fd_flags != FD_CLOSED && wp.w_onebuf_opt.wo_fdl.get() < level))
                    && !put_fold_open_close(fd, fp, off)
                {
                    return false;
                }
            }
        }
    }

    true
}

fn put_fold_open_close(fd: &CFile, fp: &FoldEntry, off: Linenr) -> bool {
    let mut buf = [0u8; 32];
    format_to_buf!(&mut buf, c"%d", fp.fd_top + off);
    if fd.puts(crate::memory::bytes_as_cstr(&buf)) < 0 || !put_eol(fd) {
        return false;
    }
    let cmd = if fp.fd_flags == FD_CLOSED {
        c"sil! normal! zc"
    } else {
        c"sil! normal! zo"
    };
    if fd.puts(cmd) < 0 || !put_eol(fd) {
        return false;
    }
    true
}

fn foldclosed_both(argvars: &mut [TypVal], rettv: &mut TypVal, end: bool) {
    let lnum = tv_get_lnum(&argvars[0]);
    if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count {
        let mut first: Linenr = 0;
        let mut last: Linenr = 0;
        if has_folding_win(
            curwin_mut(),
            lnum,
            Some(&mut first),
            Some(&mut last),
            false,
            None,
        ) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(if end {
                last
            } else {
                first
            }));
            return;
        }
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
}

/// "foldclosed()" function
pub fn f_foldclosed(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    foldclosed_both(argvars, rettv, false);
}

/// "foldclosedend()" function
pub fn f_foldclosedend(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    foldclosed_both(argvars, rettv, true);
}

/// "foldlevel()" function
pub fn f_foldlevel(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let lnum = tv_get_lnum(&argvars[0]);
    if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count {
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(fold_level(lnum)));
    }
}

/// "foldtext()" function
pub fn f_foldtext(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    let foldstart = get_vim_var_nr(VV_FOLDSTART) as Linenr;
    let foldend = get_vim_var_nr(VV_FOLDEND) as Linenr;
    let dashes = get_vim_var_str(VV_FOLDDASHES);
    if foldstart > 0 && foldend <= curbuf_mut().b_ml.ml_line_count {
        let mut lnum = foldstart;
        while lnum < foldend {
            if !linewhite(lnum) {
                break;
            }
            lnum += 1;
        }

        let mut s = crate::charset::skipwhite(ml_get(lnum));
        let s_bytes = s.to_bytes();
        if s_bytes.starts_with(b"/*") || s_bytes.starts_with(b"//") {
            s = crate::charset::skipwhite(&s[2..]);
            if crate::charset::skipwhite(s).is_empty() && lnum + 1 < foldend {
                s = crate::charset::skipwhite(ml_get(lnum + 1));
                if s.to_bytes().first() == Some(&b'*') {
                    s = crate::charset::skipwhite(&s[1..]);
                }
            }
        }
        let count = foldend - foldstart + 1;
        let txt = ngettext(c"+-%s%3d line: ", c"+-%s%3d lines: ", count as u64);
        let s_bytes = s.to_bytes_with_nul();
        let s_len = s_bytes.len() - 1;
        let len = txt.to_bytes().len() + dashes.to_bytes().len() + 20 + s_len;
        let mut r_vec = vec![0u8; len];
        format_to_buf!(&mut r_vec, txt, dashes, count);
        let r_len = crate::memory::bytes_until_nul(&r_vec).len();
        r_vec[r_len..r_len + s_bytes.len()].copy_from_slice(s_bytes);
        foldtext_cleanup(&mut r_vec[r_len..]);
        let final_len = crate::memory::bytes_until_nul(&r_vec).len();
        crate::eval::typval::tv_set_string_len(rettv, Some(&r_vec[..final_len]));
    }
}

/// "foldtextresult(lnum)" function
pub fn f_foldtextresult(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    if FOLD_STATE.with(|st| st.fdt_result_entered.replace(true)) {
        return;
    }
    let lnum = tv_get_lnum(&argvars[0]).max(0);

    let info = fold_info(crate::globals::curwin_mut(), lnum);
    if info.fi_lines > 0 {
        let mut vt: VirtText = Vec::new();
        let mut text_bytes =
            get_foldtext(curwin_mut(), lnum, lnum + info.fi_lines - 1, info, &mut vt).into_bytes();
        if !vt.is_empty() {
            debug_assert!(text_bytes.is_empty());
            let mut i: usize = 0;
            while i < vt.len() {
                let mut attr: i32 = 0;
                let Some(new_text) = next_virt_text_chunk(&vt, &mut i, &mut attr) else {
                    break;
                };
                text_bytes.extend_from_slice(new_text.to_bytes());
            }
        }
        crate::eval::typval::tv_set_bytes(rettv, &text_bytes);
    }

    FOLD_STATE.with(|st| st.fdt_result_entered.set(false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_end_computes_inclusive_bottom_line() {
        let f = FoldEntry {
            fd_top: 10,
            fd_len: 5,
            fd_nested: Vec::new(),
            fd_flags: FD_CLOSED,
            fd_small: K_NONE,
        };
        assert_eq!(fold_end(&f), 14);
    }
}
