//! Functions related to changing text (`src/nvim/change.c`).

use crate::autocmd::{EVENT_FILECHANGEDRO, apply_autocmds, aucmd_defer_modified};
use crate::buffer::{
    BF_NEVERLOADED, BF_NEW, MAXLNUM, bt_dontwrite, bt_prompt, buf_T, buf_inc_changedtick,
    buf_meta_total, kUIMessages,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, getdigits_int, getwhitecols_curline, skipwhite, vim_strnsize,
};
use crate::context::INIT_FMARKV;
use crate::cursor::{
    check_cursor_lnum, check_visual_pos, coladvance_force, colnr_T, get_cursor_line_len,
    get_cursor_line_ptr, get_cursor_pos_ptr, getviscol, linenr_T, pos_T,
};
use crate::diff::{diff_internal, diff_lnum_win, diff_update_line};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_buf_status_later, redraw_later, redrawWinline,
    set_must_redraw, showmode,
};
use crate::eval::vars::{VV_WARNINGMSG, set_vim_var_string};
use crate::extmark::{
    bcount_t, extmark_adjust, extmark_splice, extmark_splice_cols, kExtmarkNOOP, kExtmarkUndo,
};
use crate::fold::{find_wl_entry, foldUpdate, hasAnyFolding, hasFoldingWin};
use crate::globals::*;
use crate::indent::{
    copy_indent, fixthisline, get_indent, get_lisp_indent, get_sw_value, indent_size_ts, may_do_si,
    set_indent, use_indentexpr_for_lisp,
};
use crate::indent_c::{check_linecomment, cin_is_cinword, do_c_expr_indent, in_cinkeys};
use crate::insert::{prompt_text, replace_push, replace_push_nul, truncate_spaces};
use crate::insexpand::ins_compl_active;
use crate::mark::{JUMPLISTSIZE, mark_adjust, mark_col_adjust, mark_view_make, reset_fmark};
use crate::marktree::{kMTMetaInline, kMTMetaLines};
use crate::mbyte::{
    mb_adjust_cursor, ptr2cells, utf_char2bytes, utf_composinglike, utf_head_off,
    utf_iscomposing_first, utf_ptr2char, utf_ptr2len, utfc_ptr2len, utfc_ptr2len_len,
};
use crate::mcursor::mc_replaying;
use crate::memline::{
    ml_add_deleted_len, ml_append, ml_delete_flags, ml_get, ml_get_buf, ml_get_len,
    ml_line_alloced, ml_open_file, ml_replace, ml_setflags,
};
use crate::message::{
    msg_clr_eos, msg_delay, msg_end, msg_ext_set_kind, msg_puts_hl, msg_source, msg_start, siemsg,
    wait_return,
};
use crate::r#move::{
    approximate_botline_win, changed_cline_bef_curs, changed_line_abv_curs_win,
    invalidate_botline_win, set_topline, sms_marker_overlap,
};
use crate::option::{copy_option_part, get_ve_flags};
use crate::option_vars::*;
use crate::plines::{getvcol, linetabsize_eol, win_chartabsize};
use crate::search::{BACKWARD, FORWARD, findmatch, linewhite, showmatch};
use crate::spell::spell_check_window;
use crate::state::{MODE_INSERT, REPLACE_FLAG, VREPLACE_FLAG, virtual_active};
use crate::strings::{concat_str, tr, vim_strchr};
use crate::textformat::{comp_textwidth, has_format_option};
use crate::ui::{ui_active, ui_has};
use crate::undo::{curbufIsChanged, u_clearline, u_save_cursor, u_savedel};
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use crate::arglist::ML_EMPTY;
use crate::eval::typval::kCallbackNone;
use crate::ex_docmd::{CMOD_KEEPJUMPS, CMOD_LOCKMARKS};
use crate::math::{FAIL, OK};
use crate::regexp::{GRAPHEME_STATE_INIT, MAXCOL};

pub const OPENLINE_DELSPACES: c_int = 0x01;
pub const OPENLINE_DO_COM: c_int = 0x02;
pub const OPENLINE_KEEPTRAIL: c_int = 0x04;
pub const OPENLINE_MARKFIX: c_int = 0x08;
pub const OPENLINE_COM_LIST: c_int = 0x10;
pub const OPENLINE_FORMAT: c_int = 0x20;
pub const OPENLINE_FORCE_INDENT: c_int = 0x40;

pub(crate) const TAB: c_char = b'\t' as c_char;

pub(crate) const MB_MAXCHAR: usize = 6;
pub(crate) const COM_MAX_LEN: usize = 50;

const COM_NEST: c_char = b'n' as c_char;
const COM_BLANK: c_char = b'b' as c_char;
pub(crate) const COM_START: c_char = b's' as c_char;
pub(crate) const COM_MIDDLE: c_char = b'm' as c_char;
pub(crate) const COM_END: c_char = b'e' as c_char;
const COM_AUTO_END: c_char = b'x' as c_char;
pub(crate) const COM_FIRST: c_char = b'f' as c_char;
pub(crate) const COM_LEFT: c_char = b'l' as c_char;
pub(crate) const COM_RIGHT: c_char = b'r' as c_char;
const COM_NOBACK: c_char = b'O' as c_char;

pub(crate) const KEY_OPEN_FORW: c_int = 0x101;
pub(crate) const KEY_OPEN_BACK: c_int = 0x102;

pub(crate) const SIN_INSERT: c_int = 2;
pub(crate) const SIN_NOMARK: c_int = 8;

pub(crate) const ML_DEL_MESSAGE: c_int = 1;

pub(crate) const kOptVeFlagOnemore: c_uint = 0x08;
pub(crate) const kCpoDollar: c_int = b'$' as c_int;
pub(crate) const kCpoListwm: c_int = b'L' as c_int;
const kFoNoOpenComs: c_int = b'/' as c_int;

pub(crate) const HLF_W: c_int = 26;

#[inline]
pub const fn replace_normal(s: c_int) -> bool {
    (s & REPLACE_FLAG) != 0 && (s & VREPLACE_FLAG) == 0
}

#[inline]
fn for_all_tab_windows(mut f: impl FnMut(*mut win_T)) {
    unsafe {
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                let next_wp = (*wp).w_next;
                f(wp);
                wp = next_wp;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// If the file is readonly, give a warning message with the first change.
pub fn change_warning(buf: *mut buf_T, col: c_int) {
    unsafe {
        const W_READONLY: &CStr = c"W10: Warning: Changing a readonly file";

        if !(*buf).b_did_warn && !curbufIsChanged() && !g_st().autocmd_busy && (*buf).b_p_ro != 0 {
            (*buf).b_ro_locked += 1;
            apply_autocmds(
                EVENT_FILECHANGEDRO,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                buf,
            );
            (*buf).b_ro_locked -= 1;
            if (*buf).b_p_ro == 0 {
                return;
            }
            msg_start();
            if g_st().msg_row == g_st().Rows - 1 {
                g_st().msg_col = col;
            }
            msg_source(HLF_W);
            msg_ext_set_kind(c"wmsg".as_ptr());
            msg_puts_hl(tr(W_READONLY.as_ptr()), HLF_W, true);
            set_vim_var_string(VV_WARNINGMSG, tr(W_READONLY.as_ptr()), -1);
            msg_clr_eos();
            msg_end();
            if g_st().msg_silent == 0 && !g_st().silent_mode && ui_active() != 0 {
                msg_delay(1002, true);
            }
            (*buf).b_did_warn = true;
            g_st().redraw_cmdline = false;
            if g_st().msg_row < g_st().Rows - 1 {
                showmode();
            }
        }
    }
}

/// Call this function when something in a buffer is changed.
pub fn changed(buf: *mut buf_T) {
    unsafe {
        if (*buf).b_changed == 0 {
            let save_msg_scroll = g_st().msg_scroll;

            change_warning(buf, 0);

            if (*buf).b_may_swap && !bt_dontwrite(buf) {
                let save_need_wait_return = g_st().need_wait_return;

                g_st().need_wait_return = false;
                ml_open_file(&mut *buf);

                if g_st().need_wait_return && g_st().emsg_silent == 0 && !g_st().in_assert_fails && !ui_has(kUIMessages)
                {
                    msg_delay(2002, true);
                    wait_return(c_int::from(true));
                    g_st().msg_scroll = save_msg_scroll;
                } else {
                    g_st().need_wait_return = save_need_wait_return;
                }
            }
            changed_internal(&mut *buf);
        }
        buf_inc_changedtick(buf);

        if buf == g_st().curbuf {
            g_st().Search.hl_match = false;
        }
    }
}

/// Internal part of `changed()`, no user interaction.
pub fn changed_internal(buf: &mut buf_T) {
    let was_changed = buf.b_changed != 0;
        buf.b_changed = 1;
        ml_setflags(buf);
        redraw_buf_status_later(buf);
        g_st().redraw_tabline = true;
        g_st().need_maketitle = true;
        if !was_changed {
            aucmd_defer_modified(buf, true);
        }
}

/// Invalidate a window's `w_valid` flags and `w_lines[]` entries after changing lines.
fn changed_lines_invalidate_win(
    wp: &mut win_T,
    lnum: linenr_T,
    col: colnr_T,
    mut lnume: linenr_T,
    xtra: linenr_T,
) {
    unsafe {
        if wp.w_cursor.lnum <= lnum {
            let i = find_wl_entry(&mut *wp, lnum);
            if i >= 0 && wp.w_cursor.lnum > (&wp.w_lines)[i as usize].wl_lnum {
                changed_line_abv_curs_win(&mut *wp);
            }
        }

        if wp.w_cursor.lnum > lnum {
            changed_line_abv_curs_win(&mut *wp);
        } else if wp.w_cursor.lnum == lnum && wp.w_cursor.col >= col {
            changed_cline_bef_curs(wp);
        }
        if wp.w_botline >= lnum {
            if xtra < 0 {
                invalidate_botline_win(&mut *wp);
            } else {
                approximate_botline_win(wp);
            }
        }

        if (xtra < 0
            && wp.w_onebuf_opt.wo_wrap != 0
            && buf_meta_total(&*wp.w_buffer, kMTMetaInline) != 0)
            || (xtra != 0 && buf_meta_total(&*wp.w_buffer, kMTMetaLines) != 0)
        {
            lnume += 1;
        }

        let valid_len = wp.w_lines_valid.max(0) as usize;
        for (i, wl) in (&mut wp.w_lines)[..valid_len].iter_mut().enumerate() {
            if wl.wl_valid {
                if wl.wl_lnum >= lnum {
                    if i == 0 || wl.wl_lnum < lnume {
                        wl.wl_valid = false;
                    } else if xtra != 0 {
                        wl.wl_lnum += xtra;
                        wl.wl_foldend += xtra;
                        wl.wl_lastlnum += xtra;
                    }
                } else if wl.wl_lastlnum >= lnum {
                    wl.wl_valid = false;
                }
            }
        }
    }
}

/// Like `changed_lines_invalidate_win()`, but for all windows displaying a buffer.
pub fn changed_lines_invalidate_buf(buf: &mut buf_T,
    lnum: linenr_T,
    col: colnr_T,
    lnume: linenr_T,
    xtra: linenr_T,
) {
    let buf: *mut buf_T = &raw mut *buf;
    unsafe {
        for_all_tab_windows(|wp| {
            if (*wp).w_buffer == buf {
                changed_lines_invalidate_win(&mut *wp, lnum, col, lnume, xtra);
            }
        });
    }
}

/// Common code for when a change was made.
fn changed_common(
    buf: *mut buf_T,
    mut lnum: linenr_T,
    col: colnr_T,
    lnume: linenr_T,
    xtra: linenr_T,
) {
    unsafe {
        changed(buf);

        let mut win = g_st().firstwin;
        while !win.is_null() {
            if (*win).w_buffer == buf {
                if (*win).w_onebuf_opt.wo_diff != 0 && diff_internal() != 0 {
                    (*g_st().curtab).tp_diff_update = 1;
                    diff_update_line(lnum);
                }

                if xtra != 0 {
                    (*win).w_redr_status = true;
                }
            }
            win = (*win).w_next;
        }

        if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 {
            let mut view = INIT_FMARKV;
            if (**curwin.as_ptr()).w_buffer == buf
                && lnum >= (**curwin.as_ptr()).w_topline
                && lnum <= (**curwin.as_ptr()).w_botline
            {
                view = mark_view_make(&**curwin.as_ptr(), (**curwin.as_ptr()).w_cursor);
            }
            reset_fmark(
                &mut (*buf).b_last_change,
                pos_T {
                    lnum,
                    col,
                    coladd: 0,
                },
                (*buf).b_fnum,
                view,
            );
        }

        if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 && !mc_replaying() {
            if (*buf).b_new_change || (*buf).b_changelistlen == 0 {
                let add = if (*buf).b_changelistlen == 0 {
                    true
                } else {
                    let p =
                        &raw const (*buf).b_changelist[((*buf).b_changelistlen - 1) as usize].mark;
                    if (*p).lnum != lnum {
                        true
                    } else {
                        let mut cols = comp_textwidth(false);
                        if cols == 0 {
                            cols = 79;
                        }
                        (*p).col + cols < col || col + cols < (*p).col
                    }
                };
                if add {
                    (*buf).b_new_change = false;

                    if (*buf).b_changelistlen == JUMPLISTSIZE {
                        (*buf).b_changelistlen = JUMPLISTSIZE - 1;
                        ptr::copy(
                            (*buf).b_changelist.as_ptr().add(1),
                            (*buf).b_changelist.as_mut_ptr(),
                            (JUMPLISTSIZE - 1) as usize,
                        );
                        for_all_tab_windows(|wp| {
                            if (*wp).w_buffer == buf && (*wp).w_changelistidx > 0 {
                                (*wp).w_changelistidx -= 1;
                            }
                        });
                    }
                    for_all_tab_windows(|wp| {
                        if (*wp).w_buffer == buf && (*wp).w_changelistidx == (*buf).b_changelistlen
                        {
                            (*wp).w_changelistidx += 1;
                        }
                    });
                    (*buf).b_changelistlen += 1;
                }
            }
            (*buf).b_changelist[((*buf).b_changelistlen - 1) as usize] =
                (*buf).b_last_change.clone();
            if (**curwin.as_ptr()).w_buffer == buf {
                (**curwin.as_ptr()).w_changelistidx = (*buf).b_changelistlen;
            }
        }

        if (**curwin.as_ptr()).w_buffer == buf && g_st().Visual.active {
            check_visual_pos();
        }

        for_all_tab_windows(|wp| {
            if (*wp).w_buffer == buf {
                if !g_st().redraw_not_allowed && (*wp).w_redr_type < UPD_VALID {
                    (*wp).w_redr_type = UPD_VALID;
                }

                if xtra != 0 && (*wp).w_redraw_top != 0 {
                    redraw_later(wp, UPD_NOT_VALID);
                }

                let mut last = lnume + xtra - 1;

                if (*wp).w_skipcol > 0
                    && (last < (*wp).w_topline
                        || ((*wp).w_topline >= lnum
                            && (*wp).w_topline < lnume
                            && linetabsize_eol(wp, (*wp).w_topline)
                                <= (*wp).w_skipcol + sms_marker_overlap(wp, -1)))
                {
                    (*wp).w_skipcol = 0;
                }

                foldUpdate(wp, lnum, last);

                let mut folded = hasFoldingWin(
                    wp,
                    lnum,
                    &raw mut lnum,
                    ptr::null_mut(),
                    false,
                    None,
                );
                if (*wp).w_cursor.lnum == lnum {
                    (*wp).w_cline_folded = folded;
                }
                folded = hasFoldingWin(
                    wp,
                    last,
                    ptr::null_mut(),
                    &raw mut last,
                    false,
                    None,
                );
                if (*wp).w_cursor.lnum == last {
                    (*wp).w_cline_folded = folded;
                }

                changed_lines_invalidate_win(&mut *wp, lnum, col, lnume, xtra);

                if hasAnyFolding(&mut *wp) != 0 {
                    set_topline(&mut *wp, (*wp).w_topline);
                }

                if (*wp).w_onebuf_opt.wo_rnu != 0 && xtra != 0 {
                    (*wp).w_last_cursor_lnum_rnu = 0;
                }

                if (*wp).w_onebuf_opt.wo_cul != 0 && (*wp).w_last_cursorline >= lnum {
                    if (*wp).w_last_cursorline < lnume {
                        (*wp).w_last_cursorline = 0;
                    } else {
                        (*wp).w_last_cursorline += xtra;
                    }
                }
            }

            if wp == *curwin.as_ptr() && xtra != 0 && g_st().search_hl_has_cursor_lnum >= lnum {
                g_st().search_hl_has_cursor_lnum += xtra;
            }
        });

        set_must_redraw(UPD_VALID);

        if g_st().last_cursormoved_win == *curwin.as_ptr()
            && (**curwin.as_ptr()).w_buffer == buf
            && lnum <= (**curwin.as_ptr()).w_cursor.lnum
            && lnume + xtra.abs() > (**curwin.as_ptr()).w_cursor.lnum
        {
            g_st().last_cursormoved.lnum = 0;
        }
    }
}

/// Changed bytes within a single line for the current buffer.
pub fn changed_bytes(lnum: linenr_T, col: colnr_T) {
    unsafe {
        changed_lines_redraw_buf(g_st().curbuf, lnum, lnum + 1, 0);
        changed_common(g_st().curbuf, lnum, col, lnum + 1, 0);
        if spell_check_window(&mut **curwin.as_ptr())
            && lnum < (*g_st().curbuf).b_ml.ml_line_count
            && vim_strchr(opt_vars().p_cpo, kCpoDollar).is_null()
        {
            redrawWinline(*curwin.as_ptr(), lnum + 1);
        }
        buf_updates_send_changes(g_st().curbuf, lnum, 1, 1);

        if (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0 {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_diff != 0 && wp != *curwin.as_ptr() {
                    redraw_later(wp, UPD_VALID);
                    let wlnum = diff_lnum_win(lnum, &mut *wp);
                    if wlnum > 0 {
                        changed_lines_redraw_buf((*wp).w_buffer, wlnum, wlnum + 1, 0);
                    }
                }
                wp = (*wp).w_next;
            }
        }
    }
}

/// Insert/delete bytes at column.
pub fn inserted_bytes(lnum: linenr_T, start_col: colnr_T, old_col: c_int, new_col: c_int) {
    if g_st().curbuf_splice_pending == 0 {
            extmark_splice_cols(g_st().curbuf, lnum - 1, start_col, old_col, new_col, kExtmarkUndo);
        }

        changed_bytes(lnum, start_col);
}

/// Appended `count` lines below line `lnum` in the given buffer.
pub fn appended_lines_buf(buf: *mut buf_T, lnum: linenr_T, count: linenr_T) {
    changed_lines(buf, lnum + 1, 0, lnum + 1, count, true);
}

/// Appended `count` lines below line `lnum` in the current buffer.
pub fn appended_lines(lnum: linenr_T, count: linenr_T) {
    appended_lines_buf(g_st().curbuf, lnum, count);
}

/// Like `appended_lines()`, but adjust marks first.
pub fn appended_lines_mark(lnum: linenr_T, count: c_int) {
    mark_adjust(lnum + 1, MAXLNUM, count, 0, kExtmarkUndo);
        changed_lines(g_st().curbuf, lnum + 1, 0, lnum + 1, count, true);
}

/// Deleted `count` lines at line `lnum` in the given buffer.
pub fn deleted_lines_buf(buf: *mut buf_T, lnum: linenr_T, count: linenr_T) {
    changed_lines(buf, lnum, 0, lnum + count, -count, true);
}

/// Deleted `count` lines at line `lnum` in the current buffer.
pub fn deleted_lines(lnum: linenr_T, count: linenr_T) {
    deleted_lines_buf(g_st().curbuf, lnum, count);
}

/// Like `deleted_lines()`, but adjust marks first.
pub fn deleted_lines_mark(lnum: linenr_T, count: c_int) {
    unsafe {
        let made_empty = count > 0 && ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0;

        mark_adjust(lnum, lnum + count - 1, MAXLNUM, -count, kExtmarkNOOP);
        extmark_adjust(
            g_st().curbuf,
            lnum,
            lnum + count - 1,
            MAXLNUM,
            -count + c_int::from(made_empty),
            kExtmarkUndo,
        );
        changed_lines(g_st().curbuf, lnum, 0, lnum + count, -count, true);
    }
}

/// Marks the area to be redrawn after a change.
pub fn changed_lines_redraw_buf(
    buf: *mut buf_T,
    lnum: linenr_T,
    mut lnume: linenr_T,
    xtra: linenr_T,
) {
    unsafe {
        if xtra != 0 && (*buf).b_marktree.n_keys > 0 {
            lnume += 1 + c_int::from(xtra < 0 && buf_meta_total(&*buf, kMTMetaLines) != 0);
        }

        if (*buf).b_mod_set {
            (*buf).b_mod_top = (*buf).b_mod_top.min(lnum);
            if lnum < (*buf).b_mod_bot {
                (*buf).b_mod_bot += xtra;
                (*buf).b_mod_bot = (*buf).b_mod_bot.max(lnum);
            }
            (*buf).b_mod_bot = (*buf).b_mod_bot.max(lnume + xtra);
            (*buf).b_mod_xlines += xtra;
        } else {
            (*buf).b_mod_set = true;
            (*buf).b_mod_top = lnum;
            (*buf).b_mod_bot = lnume + xtra;
            (*buf).b_mod_xlines = xtra;
        }
    }
}

/// Changed lines for a buffer.
pub fn changed_lines(
    buf: *mut buf_T,
    lnum: linenr_T,
    col: colnr_T,
    lnume: linenr_T,
    xtra: linenr_T,
    do_buf_event: bool,
) {
    unsafe {
        changed_lines_redraw_buf(buf, lnum, lnume, xtra);

        if xtra == 0
            && (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0
            && (**curwin.as_ptr()).w_buffer == buf
            && diff_internal() == 0
        {
            let mut wp = g_st().firstwin;
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_diff != 0 && wp != *curwin.as_ptr() {
                    redraw_later(wp, UPD_VALID);
                    let wlnum = diff_lnum_win(lnum, &mut *wp);
                    if wlnum > 0 {
                        changed_lines_redraw_buf((*wp).w_buffer, wlnum, lnume - lnum + wlnum, 0);
                    }
                }
                wp = (*wp).w_next;
            }
        }

        changed_common(buf, lnum, col, lnume, xtra);

        if do_buf_event {
            let num_added = i64::from(lnume + xtra - lnum);
            let num_removed = i64::from(lnume - lnum);
            buf_updates_send_changes(buf, lnum, num_added, num_removed);
        }
    }
}

/// Called when the changed flag must be reset for buffer `buf`.
pub fn unchanged(buf: *mut buf_T, ff: bool, always_inc_changedtick: bool) {
    unsafe {
        if (*buf).b_changed != 0 || (ff && file_ff_differs(buf, false)) {
            let was_changed = (*buf).b_changed != 0;
            (*buf).b_changed = 0;
            ml_setflags(buf);
            if ff {
                save_file_ff(&mut *buf);
            }
            redraw_buf_status_later(buf);
            g_st().redraw_tabline = true;
            g_st().need_maketitle = true;
            buf_inc_changedtick(buf);
            if was_changed {
                aucmd_defer_modified(buf, false);
            }
        } else if always_inc_changedtick {
            buf_inc_changedtick(buf);
        }
    }
}

/// Save the current values of `'fileformat'` and `'fileencoding'`.
pub fn save_file_ff(buf: &mut buf_T) {
    unsafe {
        buf.b_start_ffc = c_int::from(*buf.b_p_ff as u8);
        buf.b_start_eof = buf.b_p_eof;
        buf.b_start_eol = buf.b_p_eol;
        buf.b_start_bomb = buf.b_p_bomb;

        let p_fenc_bytes = CStr::from_ptr(buf.b_p_fenc).to_bytes();
        if buf
            .b_start_fenc
            .as_ref()
            .is_none_or(|s| s.to_bytes() != p_fenc_bytes)
        {
            buf.b_start_fenc = Some(CStr::from_ptr(buf.b_p_fenc).to_owned());
        }
    }
}

/// Return true if `'fileformat'` and/or `'fileencoding'` has a different value
/// from when editing started.
pub fn file_ff_differs(buf: *mut buf_T, ignore_empty: bool) -> bool {
    unsafe {
        if ((*buf).b_flags & BF_NEVERLOADED) != 0 {
            return false;
        }
        if ignore_empty
            && ((*buf).b_flags & BF_NEW) != 0
            && (*buf).b_ml.ml_line_count == 1
            && *ml_get_buf(buf, 1) == NUL
        {
            return false;
        }
        if (*buf).b_start_ffc != c_int::from(*(*buf).b_p_ff) {
            return true;
        }
        if ((*buf).b_p_bin != 0 || (*buf).b_p_fixeol == 0)
            && ((*buf).b_start_eof != (*buf).b_p_eof || (*buf).b_start_eol != (*buf).b_p_eol)
        {
            return true;
        }
        if (*buf).b_p_bin == 0 && (*buf).b_start_bomb != (*buf).b_p_bomb {
            return true;
        }
        let Some(ref start_fenc) = (*buf).b_start_fenc else {
            return *(*buf).b_p_fenc != NUL;
        };
        start_fenc.to_bytes() != CStr::from_ptr((*buf).b_p_fenc).to_bytes()
    }
}

/// Insert string `p` at the cursor position. Stops at a NUL byte.
pub fn ins_bytes(p: *mut c_char) {
    unsafe {
        ins_bytes_len(p, CStr::from_ptr(p).to_bytes().len());
    }
}

/// Insert string `p` with length `len` at the cursor position.
pub fn ins_bytes_len(p: *mut c_char, len: usize) {
    unsafe {
        let mut i: usize = 0;
        while i < len {
            let n = utfc_ptr2len_len(p.add(i), (len - i) as c_int) as usize;
            ins_char_bytes(p.add(i), n);
            i += n;
        }
    }
}

/// Insert or replace a single character at the cursor position.
pub fn ins_char(c: c_int) {
    let mut buf: [c_char; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
        let n = utf_char2bytes(c, buf.as_mut_ptr()) as usize;

        if buf[0] == 0 {
            buf[0] = b'\n' as c_char;
        }
        ins_char_bytes(buf.as_mut_ptr(), n);
}

pub fn ins_char_bytes(buf: *mut c_char, charlen: usize) {
    unsafe {
        if virtual_active(*curwin.as_ptr()) && (**curwin.as_ptr()).w_cursor.coladd > 0 {
            coladvance_force(getviscol());
        }

        let col = (**curwin.as_ptr()).w_cursor.col as usize;
        let lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let oldp = ml_get(lnum);
        let linelen = ml_get_len(lnum) as usize + 1;

        let mut oldlen: usize = 0;
        let mut newlen: usize = charlen;

        if (g_st().State & REPLACE_FLAG) != 0 {
            if (g_st().State & VREPLACE_FLAG) != 0 {
                let old_list = (**curwin.as_ptr()).w_onebuf_opt.wo_list;
                if old_list != 0 && vim_strchr(opt_vars().p_cpo, kCpoListwm).is_null() {
                    (**curwin.as_ptr()).w_onebuf_opt.wo_list = 0;
                }
                let mut vcol: colnr_T = 0;
                getvcol(
                    &mut **curwin.as_ptr(),
                    &mut (**curwin.as_ptr()).w_cursor,
                    ptr::null_mut(),
                    &raw mut vcol,
                    ptr::null_mut(),
                    0,
                );
                let new_vcol = vcol + win_chartabsize(&mut **curwin.as_ptr(), buf, vcol);
                while *oldp.add(col + oldlen) != NUL && vcol < new_vcol {
                    vcol += win_chartabsize(&mut **curwin.as_ptr(), oldp.add(col + oldlen), vcol);
                    if vcol > new_vcol && *oldp.add(col + oldlen) == TAB {
                        break;
                    }
                    oldlen += utfc_ptr2len(oldp.add(col + oldlen)) as usize;
                    if vcol > new_vcol {
                        newlen += (vcol - new_vcol) as usize;
                    }
                }
                (**curwin.as_ptr()).w_onebuf_opt.wo_list = old_list;
            } else if *oldp.add(col) != NUL {
                oldlen = utfc_ptr2len(oldp.add(col)) as usize;
            }

            replace_push_nul();
            replace_push(oldp.add(col), oldlen);
        }

        let mut newp_vec = vec![0 as c_char; linelen + newlen - oldlen];
        let newp = newp_vec.as_mut_ptr();

        if col > 0 {
            ptr::copy_nonoverlapping(oldp, newp, col);
        }

        let p = newp.add(col);
        if linelen > col + oldlen {
            ptr::copy_nonoverlapping(
                oldp.add(col + oldlen),
                p.add(newlen),
                linelen - col - oldlen,
            );
        }

        ptr::copy_nonoverlapping(buf, p, charlen);

        for i in charlen..newlen {
            *p.add(i) = b' ' as c_char;
        }

        ml_replace(lnum, newp, true);
        drop(newp_vec);
        inserted_bytes(lnum, col as colnr_T, oldlen as c_int, newlen as c_int);

        if opt_vars().p_sm != 0 && (g_st().State & MODE_INSERT) != 0 && g_st().msg_silent == 0 && !ins_compl_active() {
            showmatch(utf_ptr2char(buf));
        }

        if opt_vars().p_ri == 0 || (g_st().State & REPLACE_FLAG) != 0 {
            (**curwin.as_ptr()).w_cursor.col += charlen as colnr_T;
        }
    }
}

/// Insert a string at the cursor position.
pub fn ins_str(s: *mut c_char, slen: usize) {
    unsafe {
        let lnum = (**curwin.as_ptr()).w_cursor.lnum;

        if virtual_active(*curwin.as_ptr()) && (**curwin.as_ptr()).w_cursor.coladd > 0 {
            coladvance_force(getviscol());
        }

        let col = (**curwin.as_ptr()).w_cursor.col;
        let oldp = ml_get(lnum);
        let oldlen = ml_get_len(lnum);

        let mut newp_vec = vec![0 as c_char; oldlen as usize + slen + 1];
        let newp = newp_vec.as_mut_ptr();
        if col > 0 {
            ptr::copy_nonoverlapping(oldp, newp, col as usize);
        }
        ptr::copy_nonoverlapping(s, newp.add(col as usize), slen);
        let bytes = oldlen - col + 1;
        debug_assert!(bytes >= 0);
        ptr::copy_nonoverlapping(
            oldp.add(col as usize),
            newp.add(col as usize + slen),
            bytes as usize,
        );
        ml_replace(lnum, newp, true);
        drop(newp_vec);
        inserted_bytes(lnum, col, 0, slen as c_int);
        (**curwin.as_ptr()).w_cursor.col += slen as colnr_T;
    }
}

/// Delete one character under the cursor.
pub fn del_char(fixpos: bool) -> c_int {
    unsafe {
        mb_adjust_cursor();
        if *get_cursor_pos_ptr() == NUL {
            return FAIL;
        }
        del_chars(1, c_int::from(fixpos))
    }
}

/// Like `del_bytes()`, but delete characters instead of bytes.
pub fn del_chars(count: c_int, fixpos: c_int) -> c_int {
    unsafe {
        let mut bytes: colnr_T = 0;
        let mut p = get_cursor_pos_ptr();
        let mut i: c_int = 0;
        while i < count && *p != NUL {
            let l = utfc_ptr2len(p);
            bytes += l;
            p = p.add(l as usize);
            i += 1;
        }
        del_bytes(bytes, fixpos != 0, true)
    }
}

/// Delete `count` bytes under the cursor.
pub fn del_bytes(mut count: colnr_T, fixpos_arg: bool, use_delcombine: bool) -> c_int {
    unsafe {
        let lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let mut col = (**curwin.as_ptr()).w_cursor.col;
        let mut fixpos = fixpos_arg;
        let oldp = ml_get(lnum);
        let oldlen = ml_get_len(lnum);

        if col >= oldlen {
            return FAIL;
        }
        if count == 0 {
            return OK;
        }
        if count < 1 {
            siemsg!(
                c"E292: Invalid count for del_bytes(): %ld".as_ptr(),
                i64::from(count),
            );
            return FAIL;
        }

        if opt_vars().p_deco != 0 && use_delcombine && utfc_ptr2len(oldp.add(col as usize)) >= count {
            let p0 = oldp.add(col as usize);
            let mut state = GRAPHEME_STATE_INIT;
            if utf_composinglike(p0, p0.add(utf_ptr2len(p0) as usize), &raw mut state) {
                let mut n = col;
                loop {
                    col = n;
                    count = utf_ptr2len(oldp.add(n as usize));
                    n += count;
                    if !utf_composinglike(
                        oldp.add(col as usize),
                        oldp.add(n as usize),
                        &raw mut state,
                    ) {
                        break;
                    }
                }
                fixpos = false;
            }
        }

        let mut movelen = oldlen - col - count + 1;
        if movelen <= 1 {
            if col > 0
                && fixpos
                && g_st().restart_edit == 0
                && (get_ve_flags(*curwin.as_ptr()) & kOptVeFlagOnemore) == 0
            {
                (**curwin.as_ptr()).w_cursor.col -= 1;
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                (**curwin.as_ptr()).w_cursor.col -=
                    utf_head_off(oldp, oldp.add((**curwin.as_ptr()).w_cursor.col as usize));
            }
            count = oldlen - col;
            movelen = 1;
        }
        let newlen = oldlen - count;

        let alloc_newp = ml_line_alloced() == 0;
        let mut newp_vec: Vec<c_char> = Vec::new();
        let newp: *mut c_char = if !alloc_newp {
            ml_add_deleted_len((*g_st().curbuf).b_ml.ml_line_ptr, oldlen as isize);
            oldp
        } else {
            newp_vec = vec![0 as c_char; newlen as usize + 1];
            let p = newp_vec.as_mut_ptr();
            ptr::copy_nonoverlapping(oldp, p, col as usize);
            p
        };
        ptr::copy(
            oldp.add((col + count) as usize),
            newp.add(col as usize),
            movelen as usize,
        );
        if alloc_newp {
            ml_replace(lnum, newp, true);
        } else {
            (*g_st().curbuf).b_ml.ml_line_textlen = newlen + 1;
        }
        drop(newp_vec);

        inserted_bytes(lnum, col, count, 0);

        OK
    }
}

/// Add a new line below or above the current line.
pub fn open_line(
    dir: c_int,
    flags: c_int,
    second_line_indent: c_int,
    did_do_comment: *mut bool,
) -> bool {
    unsafe {
        let mut next_line_vec: Vec<c_char> = Vec::new();
        let mut p_extra: *mut c_char = ptr::null_mut();
        let mut less_cols: colnr_T = 0;
        let mut less_cols_off: colnr_T = 0;
        let mut newcol: colnr_T = 0;
        let mut newindent: c_int = 0;
        let mut trunc_line = false;
        let mut retval = false;
        let mut extra_len: c_int = 0;
        let mut lead_len: c_int;
        let mut comment_start: c_int = 0;
        let mut lead_flags: *mut c_char = ptr::null_mut();
        let mut leader: *mut c_char = ptr::null_mut();
        let mut allocated_vec: Vec<c_char> = Vec::new();
        let mut saved_char: c_char = NUL;
        let do_si = may_do_si();
        let mut no_si = false;
        let mut first_char: c_int = 0;
        let saved_pi = (*g_st().curbuf).b_p_pi;

        let lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let mincol = (**curwin.as_ptr()).w_cursor.col + 1;

        let mut saved_line_vec =
            std::slice::from_raw_parts(get_cursor_line_ptr(), get_cursor_line_len() as usize + 1)
                .to_vec();
        let saved_line = saved_line_vec.as_mut_ptr();

        if (g_st().State & VREPLACE_FLAG) != 0 {
            if (**curwin.as_ptr()).w_cursor.lnum < g_st().orig_line_count {
                let next_lnum = (**curwin.as_ptr()).w_cursor.lnum + 1;
                next_line_vec = std::slice::from_raw_parts(
                    ml_get(next_lnum),
                    ml_get_len(next_lnum) as usize + 1,
                )
                .to_vec();
            } else {
                next_line_vec = vec![NUL];
            }

            replace_push_nul();
            replace_push_nul();
            let p = saved_line.add((**curwin.as_ptr()).w_cursor.col as usize);
            replace_push(p, CStr::from_ptr(p).to_bytes().len());
            *saved_line.add((**curwin.as_ptr()).w_cursor.col as usize) = NUL;
        }

        if (g_st().State & MODE_INSERT) != 0 && (g_st().State & VREPLACE_FLAG) == 0 {
            p_extra = saved_line.add((**curwin.as_ptr()).w_cursor.col as usize);
            if do_si {
                let p = skipwhite(p_extra);
                first_char = c_int::from(*p as u8);
            }
            extra_len = CStr::from_ptr(p_extra).to_bytes().len() as c_int;
            saved_char = *p_extra;
            *p_extra = NUL;
        }

        u_clearline(&mut *g_st().curbuf);
        g_st().Ins.did_si = false;
        g_st().Ins.ai_col = 0;

        if dir == FORWARD && g_st().Ins.did_ai {
            trunc_line = true;
        }

        if (flags & OPENLINE_FORCE_INDENT) != 0 {
            newindent = second_line_indent;
        } else if (*g_st().curbuf).b_p_ai != 0 || do_si {
            newindent = indent_size_ts(saved_line, (*g_st().curbuf).b_p_ts, &(*g_st().curbuf).b_p_vts_array);
            if newindent == 0 && (flags & OPENLINE_COM_LIST) == 0 {
                newindent = second_line_indent;
            }

            if !trunc_line
                && do_si
                && *saved_line != NUL
                && (p_extra.is_null() || first_char != b'{' as c_int)
            {
                let old_cursor = (**curwin.as_ptr()).w_cursor;
                let mut ptr = saved_line;
                if (flags & OPENLINE_DO_COM) != 0 {
                    lead_len = get_leader_len(ptr, ptr::null_mut(), false, true);
                } else {
                    lead_len = 0;
                }
                if dir == FORWARD {
                    if lead_len == 0 && *ptr == b'#' as c_char {
                        while *ptr == b'#' as c_char && (**curwin.as_ptr()).w_cursor.lnum > 1 {
                            (**curwin.as_ptr()).w_cursor.lnum -= 1;
                            ptr = ml_get((**curwin.as_ptr()).w_cursor.lnum);
                        }
                        newindent = get_indent();
                    }
                    if (flags & OPENLINE_DO_COM) != 0 {
                        lead_len = get_leader_len(ptr, ptr::null_mut(), false, true);
                    } else {
                        lead_len = 0;
                    }
                    if lead_len > 0 {
                        let mut p = skipwhite(ptr);
                        if *p == b'/' as c_char && *p.add(1) == b'*' as c_char {
                            p = p.add(1);
                        }
                        if *p == b'*' as c_char {
                            p = p.add(1);
                            while *p != NUL {
                                if *p == b'/' as c_char && *p.sub(1) == b'*' as c_char {
                                    (**curwin.as_ptr()).w_cursor.col = p.offset_from(ptr) as colnr_T;
                                    let pos = findmatch(ptr::null_mut(), 0);
                                    if !pos.is_null() {
                                        (**curwin.as_ptr()).w_cursor.lnum = (*pos).lnum;
                                        newindent = get_indent();
                                        break;
                                    }
                                    ptr = ml_get((**curwin.as_ptr()).w_cursor.lnum);
                                    p = ptr.add((**curwin.as_ptr()).w_cursor.col as usize);
                                }
                                p = p.add(1);
                            }
                        }
                    } else {
                        let mut p = ptr.add(CStr::from_ptr(ptr).to_bytes().len()).sub(1);
                        while p > ptr && ascii_iswhite(*p) {
                            p = p.sub(1);
                        }
                        let last_char = *p;

                        if last_char == b'{' as c_char || last_char == b';' as c_char {
                            if p > ptr {
                                p = p.sub(1);
                            }
                            while p > ptr && ascii_iswhite(*p) {
                                p = p.sub(1);
                            }
                        }
                        if *p == b')' as c_char {
                            (**curwin.as_ptr()).w_cursor.col = p.offset_from(ptr) as colnr_T;
                            let pos = findmatch(ptr::null_mut(), b'(' as c_int);
                            if !pos.is_null() {
                                (**curwin.as_ptr()).w_cursor.lnum = (*pos).lnum;
                                newindent = get_indent();
                                ptr = get_cursor_line_ptr();
                            }
                        }
                        if last_char == b'{' as c_char {
                            g_st().Ins.did_si = true;
                            no_si = true;
                        } else if last_char != b';' as c_char
                            && last_char != b'}' as c_char
                            && cin_is_cinword(ptr)
                        {
                            g_st().Ins.did_si = true;
                        }
                    }
                } else {
                    if lead_len == 0 && *ptr == b'#' as c_char {
                        let mut was_backslashed = false;

                        while (*ptr == b'#' as c_char || was_backslashed)
                            && (**curwin.as_ptr()).w_cursor.lnum < (*g_st().curbuf).b_ml.ml_line_count
                        {
                            was_backslashed = *ptr != NUL
                                && *ptr.add(CStr::from_ptr(ptr).to_bytes().len() - 1)
                                    == b'\\' as c_char;
                            (**curwin.as_ptr()).w_cursor.lnum += 1;
                            ptr = ml_get((**curwin.as_ptr()).w_cursor.lnum);
                        }
                        if was_backslashed {
                            newindent = 0;
                        } else {
                            newindent = get_indent();
                        }
                    }
                    let p = skipwhite(ptr);
                    if *p == b'}' as c_char {
                        g_st().Ins.did_si = true;
                    } else {
                        g_st().Ins.can_si_back = true;
                    }
                }
                (**curwin.as_ptr()).w_cursor = old_cursor;
            }
            if do_si {
                g_st().Ins.can_si = true;
            }

            g_st().Ins.did_ai = true;
        }

        let do_cindent = opt_vars().p_paste == 0
            && ((*g_st().curbuf).b_p_cin != 0 || (*g_st().curbuf).b_p_inde.cb_type() != kCallbackNone)
            && in_cinkeys(
                if dir == FORWARD {
                    KEY_OPEN_FORW
                } else {
                    KEY_OPEN_BACK
                },
                b' ' as c_int,
                linewhite((**curwin.as_ptr()).w_cursor.lnum),
            )
            && (flags & OPENLINE_FORCE_INDENT) == 0;

        g_st().Ins.end_comment_pending = 0;
        if (flags & OPENLINE_DO_COM) != 0 {
            lead_len = get_leader_len(saved_line, &raw mut lead_flags, dir == BACKWARD, true);
            if lead_len == 0
                && (*g_st().curbuf).b_p_cin != 0
                && do_cindent
                && dir == FORWARD
                && (!has_format_option(kFoNoOpenComs) || (flags & OPENLINE_FORMAT) != 0)
            {
                comment_start = check_linecomment(saved_line);
                if comment_start != MAXCOL {
                    lead_len = get_leader_len(
                        saved_line.add(comment_start as usize),
                        &raw mut lead_flags,
                        false,
                        true,
                    );
                    if lead_len != 0 {
                        lead_len += comment_start;
                        if !did_do_comment.is_null() {
                            *did_do_comment = true;
                        }
                    }
                }
            }
        } else {
            lead_len = 0;
        }

        if lead_len > 0 {
            let mut lead_repl: *mut c_char = ptr::null_mut();
            let mut lead_repl_len: c_int = 0;
            let mut lead_middle: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
            let lead_middle_len: c_int;
            let mut lead_end: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
            let mut comment_end: *mut c_char = ptr::null_mut();
            let mut extra_space = false;
            let mut require_blank = false;

            let mut p = lead_flags;
            while *p != NUL && *p != b':' as c_char {
                if *p == COM_BLANK {
                    require_blank = true;
                    p = p.add(1);
                    continue;
                }
                if *p == COM_START || *p == COM_MIDDLE {
                    let current_flag = *p;
                    if *p == COM_START {
                        if dir == BACKWARD {
                            lead_len = 0;
                            break;
                        }
                        copy_option_part(
                            &raw mut p,
                            lead_middle.as_mut_ptr(),
                            COM_MAX_LEN,
                            c",".as_ptr(),
                        );
                        require_blank = false;
                    }

                    while *p != NUL && *p.sub(1) != b':' as c_char {
                        if *p == COM_BLANK {
                            require_blank = true;
                        }
                        p = p.add(1);
                    }
                    lead_middle_len = copy_option_part(
                        &raw mut p,
                        lead_middle.as_mut_ptr(),
                        COM_MAX_LEN,
                        c",".as_ptr(),
                    ) as c_int;

                    while *p != NUL && *p.sub(1) != b':' as c_char {
                        if *p == COM_AUTO_END {
                            g_st().Ins.end_comment_pending = -1;
                        }
                        p = p.add(1);
                    }
                    let n = copy_option_part(
                        &raw mut p,
                        lead_end.as_mut_ptr(),
                        COM_MAX_LEN,
                        c",".as_ptr(),
                    );

                    if g_st().Ins.end_comment_pending == -1 {
                        g_st().Ins.end_comment_pending = c_int::from(lead_end[n - 1] as u8);
                    }

                    if dir == FORWARD {
                        let mut q = saved_line.add(lead_len as usize);
                        let end_bytes = &lead_end[..n];
                        while *q != NUL {
                            let q_bytes = CStr::from_ptr(q).to_bytes();
                            if q_bytes.len() >= n
                                && std::slice::from_raw_parts(q.cast::<c_char>(), n) == end_bytes
                            {
                                comment_end = q;
                                lead_len = 0;
                                break;
                            }
                            q = q.add(1);
                        }
                    }

                    if lead_len > 0 {
                        if current_flag == COM_START {
                            lead_repl = lead_middle.as_mut_ptr();
                            lead_repl_len = lead_middle_len;
                        }

                        if !ascii_iswhite(*saved_line.add((lead_len - 1) as usize))
                            && ((!p_extra.is_null() && (**curwin.as_ptr()).w_cursor.col == lead_len)
                                || (p_extra.is_null() && *saved_line.add(lead_len as usize) == NUL)
                                || require_blank)
                        {
                            extra_space = true;
                        }
                    }
                    break;
                }
                if *p == COM_END {
                    if dir == FORWARD {
                        comment_end = skipwhite(saved_line);
                        lead_len = 0;
                        break;
                    }

                    while p > (*g_st().curbuf).b_p_com && *p != b',' as c_char {
                        p = p.sub(1);
                    }
                    lead_repl = p;
                    while lead_repl > (*g_st().curbuf).b_p_com && *lead_repl.sub(1) != b':' as c_char {
                        lead_repl = lead_repl.sub(1);
                    }
                    lead_repl_len = p.offset_from(lead_repl) as c_int;

                    extra_space = true;

                    let mut p2 = p;
                    while *p2 != NUL && *p2 != b':' as c_char {
                        if *p2 == COM_AUTO_END {
                            g_st().Ins.end_comment_pending = -1;
                        }
                        p2 = p2.add(1);
                    }
                    if g_st().Ins.end_comment_pending == -1 {
                        while *p2 != NUL && *p2 != b',' as c_char {
                            p2 = p2.add(1);
                        }
                        g_st().Ins.end_comment_pending = c_int::from(*p2.sub(1) as u8);
                    }
                    break;
                }
                if *p == COM_FIRST {
                    if dir == BACKWARD {
                        lead_len = 0;
                    } else {
                        lead_repl = c"".as_ptr().cast_mut();
                        lead_repl_len = 0;
                    }
                    break;
                }
                p = p.add(1);
            }

            if lead_len > 0 {
                let bytes = lead_len
                    + lead_repl_len
                    + c_int::from(extra_space)
                    + extra_len
                    + second_line_indent.max(0)
                    + 1;
                debug_assert!(bytes >= 0);
                allocated_vec = vec![0 as c_char; bytes as usize];
                leader = allocated_vec.as_mut_ptr();

                { let _d = (leader).cast::<u8>(); let _n = lead_len as usize; if _n > 0 { std::ptr::copy_nonoverlapping((saved_line).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };

                for li in 0..comment_start {
                    if !ascii_iswhite(*leader.add(li as usize)) {
                        *leader.add(li as usize) = b' ' as c_char;
                    }
                }

                if !lead_repl.is_null() {
                    let mut c: c_char = 0;
                    let mut off: c_int = 0;

                    let mut p = lead_flags;
                    while *p != NUL && *p != b':' as c_char {
                        if *p == COM_RIGHT || *p == COM_LEFT {
                            c = *p;
                            p = p.add(1);
                        } else if ascii_isdigit(*p) || *p == b'-' as c_char {
                            off = getdigits_int(&raw mut p, true, 0);
                        } else {
                            p = p.add(1);
                        }
                    }
                    if c == COM_RIGHT {
                        let mut p = leader.add((lead_len - 1) as usize);
                        while p > leader && ascii_iswhite(*p) {
                            p = p.sub(1);
                        }
                        p = p.add(1);

                        {
                            let repl_size = vim_strnsize(lead_repl, lead_repl_len);
                            let mut old_size: c_int = 0;
                            let endp = p;

                            while old_size < repl_size && p > leader {
                                p = p.sub(utf_head_off(leader, p.sub(1)) as usize + 1);
                                old_size += ptr2cells(p);
                            }
                            let l = lead_repl_len - endp.offset_from(p) as c_int;
                            if l != 0 {
                                ptr::copy(
                                    endp,
                                    endp.offset(l as isize),
                                    leader.add(lead_len as usize).offset_from(endp) as usize,
                                );
                            }
                            lead_len += l;
                        }
                        ptr::copy(lead_repl, p, lead_repl_len as usize);
                        if p.add(lead_repl_len as usize) > leader.add(lead_len as usize) {
                            *p.add(lead_repl_len as usize) = NUL;
                        }

                        while {
                            p = p.sub(1);
                            p >= leader
                        } {
                            let mut l = utf_head_off(leader, p);
                            if l > 1 {
                                p = p.sub(l as usize);
                                if ptr2cells(p) > 1 {
                                    *p.add(1) = b' ' as c_char;
                                    l -= 1;
                                }
                                ptr::copy(
                                    p.add(l as usize + 1),
                                    p.add(1),
                                    leader
                                        .add(lead_len as usize)
                                        .offset_from(p.add(l as usize + 1))
                                        as usize,
                                );
                                lead_len -= l;
                                *p = b' ' as c_char;
                            } else if !ascii_iswhite(*p) {
                                *p = b' ' as c_char;
                            }
                        }
                    } else {
                        let mut p = skipwhite(leader);
                        {
                            let repl_size = vim_strnsize(lead_repl, lead_repl_len);
                            let mut i: c_int = 0;
                            while i < lead_len && *p.add(i as usize) != NUL {
                                let l = utfc_ptr2len(p.add(i as usize));
                                if vim_strnsize(p, i + l) > repl_size {
                                    break;
                                }
                                i += l;
                            }
                            if i != lead_repl_len {
                                ptr::copy(
                                    p.add(i as usize),
                                    p.add(lead_repl_len as usize),
                                    (lead_len - i - p.offset_from(leader) as c_int) as usize,
                                );
                                lead_len += lead_repl_len - i;
                            }
                        }
                        ptr::copy(lead_repl, p, lead_repl_len as usize);

                        p = p.add(lead_repl_len as usize);
                        while p < leader.add(lead_len as usize) {
                            if !ascii_iswhite(*p) {
                                if p.add(1) < leader.add(lead_len as usize) && *p.add(1) == TAB {
                                    lead_len -= 1;
                                    ptr::copy(
                                        p.add(1),
                                        p,
                                        leader.add(lead_len as usize).offset_from(p) as usize,
                                    );
                                } else {
                                    let mut l = utfc_ptr2len(p);
                                    if l > 1 {
                                        if ptr2cells(p) > 1 {
                                            l -= 1;
                                            *p = b' ' as c_char;
                                            p = p.add(1);
                                        }
                                        ptr::copy(
                                            p.add(l as usize),
                                            p.add(1),
                                            leader.add(lead_len as usize).offset_from(p) as usize,
                                        );
                                        lead_len -= l - 1;
                                    }
                                    *p = b' ' as c_char;
                                }
                            }
                            p = p.add(1);
                        }
                        *p = NUL;
                    }

                    if (*g_st().curbuf).b_p_ai != 0 || do_si {
                        newindent =
                            indent_size_ts(leader, (*g_st().curbuf).b_p_ts, &(*g_st().curbuf).b_p_vts_array);
                    }

                    if newindent + off < 0 {
                        off = -newindent;
                        newindent = 0;
                    } else {
                        newindent += off;
                    }

                    while off > 0
                        && lead_len > 0
                        && *leader.add((lead_len - 1) as usize) == b' ' as c_char
                    {
                        if !vim_strchr(skipwhite(leader), b'\t' as c_int).is_null() {
                            break;
                        }
                        lead_len -= 1;
                        off -= 1;
                    }

                    if lead_len > 0 && ascii_iswhite(*leader.add((lead_len - 1) as usize)) {
                        extra_space = false;
                    }
                    *leader.add(lead_len as usize) = NUL;
                }

                if extra_space {
                    *leader.add(lead_len as usize) = b' ' as c_char;
                    lead_len += 1;
                    *leader.add(lead_len as usize) = NUL;
                }

                newcol = lead_len;

                if newindent != 0 || g_st().Ins.did_si {
                    while lead_len != 0 && ascii_iswhite(*leader) {
                        lead_len -= 1;
                        newcol -= 1;
                        leader = leader.add(1);
                    }
                }
                g_st().Ins.did_si = false;
                g_st().Ins.can_si = false;
            } else if !comment_end.is_null()
                && *comment_end == b'*' as c_char
                && *comment_end.add(1) == b'/' as c_char
                && ((*g_st().curbuf).b_p_ai != 0 || do_si)
            {
                let old_cursor = (**curwin.as_ptr()).w_cursor;
                (**curwin.as_ptr()).w_cursor.col = comment_end.offset_from(saved_line) as colnr_T;
                let pos = findmatch(ptr::null_mut(), 0);
                if !pos.is_null() {
                    (**curwin.as_ptr()).w_cursor.lnum = (*pos).lnum;
                    newindent = get_indent();
                }
                (**curwin.as_ptr()).w_cursor = old_cursor;
            }
        }

        if !p_extra.is_null() {
            *p_extra = saved_char;

            if replace_normal(g_st().State) {
                replace_push_nul();
            }
            if (*g_st().curbuf).b_p_ai != 0 || (flags & OPENLINE_DELSPACES) != 0 {
                while (*p_extra == b' ' as c_char || *p_extra == b'\t' as c_char)
                    && !utf_iscomposing_first(utf_ptr2char(p_extra.add(1)))
                {
                    if replace_normal(g_st().State) {
                        replace_push(p_extra, 1);
                    }
                    p_extra = p_extra.add(1);
                    less_cols_off += 1;
                }
            }

            less_cols = p_extra.offset_from(saved_line) as colnr_T;
        }

        if p_extra.is_null() {
            p_extra = c"".as_ptr().cast_mut();
        }

        if lead_len > 0 {
            if (flags & OPENLINE_COM_LIST) != 0 && second_line_indent > 0 {
                let padding = second_line_indent
                    - (newindent + CStr::from_ptr(leader).to_bytes().len() as c_int);
                for _ in 0..padding {
                    { let d = leader; let s = c" ".as_ptr(); std::ptr::copy_nonoverlapping(s, d.add(std::ffi::CStr::from_ptr(d).to_bytes().len()), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
                    less_cols -= 1;
                    newcol += 1;
                }
            }
            { let d = leader; let s = p_extra; std::ptr::copy_nonoverlapping(s, d.add(std::ffi::CStr::from_ptr(d).to_bytes().len()), std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len()); };
            p_extra = leader;
            g_st().Ins.did_ai = true;
            less_cols -= lead_len;
        } else {
            g_st().Ins.end_comment_pending = 0;
        }

        g_st().curbuf_splice_pending += 1;
        let old_cursor = (**curwin.as_ptr()).w_cursor;
        let old_cmod_flags = g_st().cmdmod.cmod_flags;
        let mut prompt_moved: Option<std::ffi::CString> = None;

        if dir == BACKWARD {
            if bt_prompt(g_st().curbuf) && (**curwin.as_ptr()).w_cursor.lnum == (*g_st().curbuf).b_prompt_start.mark.lnum {
                let prompt_line = ml_get((**curwin.as_ptr()).w_cursor.lnum);
                let prompt = prompt_text();
                let prompt_len = CStr::from_ptr(prompt).to_bytes().len();

                if CStr::from_ptr(prompt_line)
                    .to_bytes()
                    .starts_with(CStr::from_ptr(prompt).to_bytes())
                {
                    let rem_len = CStr::from_ptr(prompt_line.add(prompt_len)).to_bytes().len() + 1;
                    ptr::copy(prompt_line.add(prompt_len), prompt_line, rem_len);
                    g_st().cmdmod.cmod_flags |= CMOD_LOCKMARKS;
                    ml_replace((**curwin.as_ptr()).w_cursor.lnum, prompt_line, true);
                    prompt_moved = Some(concat_str(std::ffi::CStr::from_ptr(prompt), std::ffi::CStr::from_ptr(p_extra)));
                    p_extra = prompt_moved
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                }
            }
            (**curwin.as_ptr()).w_cursor.lnum -= 1;
        }

        let mut did_append: bool;
        let append_failed = if (g_st().State & VREPLACE_FLAG) == 0 || old_cursor.lnum >= g_st().orig_line_count {
            if ml_append((**curwin.as_ptr()).w_cursor.lnum, p_extra, 0, false) == FAIL {
                did_append = false;
                true
            } else {
                mark_adjust((**curwin.as_ptr()).w_cursor.lnum + 1, MAXLNUM, 1, 0, kExtmarkNOOP);
                did_append = true;
                false
            }
        } else {
            (**curwin.as_ptr()).w_cursor.lnum += 1;
            if (**curwin.as_ptr()).w_cursor.lnum >= g_st().Ins.start.lnum + g_st().vr_lines_changed {
                u_save_cursor();
                g_st().vr_lines_changed += 1;
            }
            ml_replace((**curwin.as_ptr()).w_cursor.lnum, p_extra, true);
            changed_bytes((**curwin.as_ptr()).w_cursor.lnum, 0);
            (**curwin.as_ptr()).w_cursor.lnum -= 1;
            did_append = false;
            false
        };

        if !append_failed {
            g_st().inhibit_delete_count += 1;
            if newindent != 0 || g_st().Ins.did_si {
                (**curwin.as_ptr()).w_cursor.lnum += 1;
                if g_st().Ins.did_si {
                    let sw = get_sw_value(g_st().curbuf);
                    if opt_vars().p_sr != 0 {
                        newindent -= newindent % sw;
                    }
                    newindent += sw;
                }
                if (*g_st().curbuf).b_p_ci != 0 {
                    copy_indent(newindent, saved_line);
                    (*g_st().curbuf).b_p_pi = 1;
                } else {
                    set_indent(newindent, SIN_INSERT | SIN_NOMARK);
                }
                less_cols -= (**curwin.as_ptr()).w_cursor.col;

                g_st().Ins.ai_col = (**curwin.as_ptr()).w_cursor.col;

                if replace_normal(g_st().State) {
                    for _ in 0..(**curwin.as_ptr()).w_cursor.col {
                        replace_push_nul();
                    }
                }
                newcol += (**curwin.as_ptr()).w_cursor.col;
                if no_si {
                    g_st().Ins.did_si = false;
                }
            }
            g_st().inhibit_delete_count -= 1;

            if replace_normal(g_st().State) {
                while lead_len > 0 {
                    replace_push_nul();
                    lead_len -= 1;
                }
            }

            (**curwin.as_ptr()).w_cursor = old_cursor;

            if dir == FORWARD {
                if trunc_line || (g_st().State & MODE_INSERT) != 0 {
                    *saved_line.add((**curwin.as_ptr()).w_cursor.col as usize) = NUL;
                    if trunc_line && (flags & OPENLINE_KEEPTRAIL) == 0 {
                        truncate_spaces(saved_line, (**curwin.as_ptr()).w_cursor.col as usize);
                    }
                    ml_replace((**curwin.as_ptr()).w_cursor.lnum, saved_line, true);

                    let new_len = CStr::from_ptr(saved_line).to_bytes().len() as c_int;

                    let mut cols_spliced: c_int = 0;
                    if new_len < (**curwin.as_ptr()).w_cursor.col {
                        extmark_splice_cols(
                            g_st().curbuf,
                            (**curwin.as_ptr()).w_cursor.lnum - 1,
                            new_len,
                            (**curwin.as_ptr()).w_cursor.col - new_len,
                            0,
                            kExtmarkUndo,
                        );
                        cols_spliced = (**curwin.as_ptr()).w_cursor.col - new_len;
                    }

                    if did_append {
                        let cols_added = mincol - 1 + less_cols_off - less_cols;
                        extmark_splice(
                            g_st().curbuf,
                            lnum - 1,
                            mincol - 1 - cols_spliced,
                            0,
                            less_cols_off,
                            less_cols_off as bcount_t,
                            1,
                            cols_added,
                            (1 + cols_added) as bcount_t,
                            kExtmarkUndo,
                        );

                        changed_lines(
                            g_st().curbuf,
                            (**curwin.as_ptr()).w_cursor.lnum,
                            (**curwin.as_ptr()).w_cursor.col,
                            (**curwin.as_ptr()).w_cursor.lnum + 1,
                            1,
                            true,
                        );
                        did_append = false;

                        if (flags & OPENLINE_MARKFIX) != 0 {
                            mark_col_adjust(
                                (**curwin.as_ptr()).w_cursor.lnum,
                                (**curwin.as_ptr()).w_cursor.col + less_cols_off,
                                1,
                                -less_cols,
                                0,
                            );
                        }
                    } else {
                        changed_bytes((**curwin.as_ptr()).w_cursor.lnum, (**curwin.as_ptr()).w_cursor.col);
                    }
                }

                (**curwin.as_ptr()).w_cursor.lnum = old_cursor.lnum + 1;
            }
            if did_append {
                let extra = ml_get_len((**curwin.as_ptr()).w_cursor.lnum) as bcount_t;
                extmark_splice(
                    g_st().curbuf,
                    (**curwin.as_ptr()).w_cursor.lnum - 1,
                    0,
                    0,
                    0,
                    0,
                    1,
                    0,
                    1 + extra,
                    kExtmarkUndo,
                );
                changed_lines(
                    g_st().curbuf,
                    (**curwin.as_ptr()).w_cursor.lnum,
                    0,
                    (**curwin.as_ptr()).w_cursor.lnum,
                    1,
                    true,
                );
            }
            g_st().curbuf_splice_pending -= 1;

            (**curwin.as_ptr()).w_cursor.col = newcol;
            (**curwin.as_ptr()).w_cursor.coladd = 0;

            let vreplace_mode = if (g_st().State & VREPLACE_FLAG) != 0 {
                let m = g_st().State;
                g_st().State = MODE_INSERT;
                m
            } else {
                0
            };

            if opt_vars().p_paste == 0 {
                if leader.is_null()
                    && !use_indentexpr_for_lisp()
                    && (*g_st().curbuf).b_p_lisp != 0
                    && (*g_st().curbuf).b_p_ai != 0
                {
                    fixthisline(get_lisp_indent);
                    g_st().Ins.ai_col = getwhitecols_curline() as colnr_T;
                } else if do_cindent || ((*g_st().curbuf).b_p_ai != 0 && use_indentexpr_for_lisp()) {
                    do_c_expr_indent();
                    g_st().Ins.ai_col = getwhitecols_curline() as colnr_T;
                }
            }

            if vreplace_mode != 0 {
                g_st().State = vreplace_mode;
            }

            if (g_st().State & VREPLACE_FLAG) != 0 {
                let mut extra_vec = std::slice::from_raw_parts(
                    get_cursor_line_ptr(),
                    get_cursor_line_len() as usize + 1,
                )
                .to_vec();
                ml_replace((**curwin.as_ptr()).w_cursor.lnum, next_line_vec.as_mut_ptr(), true);
                (**curwin.as_ptr()).w_cursor.col = 0;
                (**curwin.as_ptr()).w_cursor.coladd = 0;
                ins_bytes(extra_vec.as_mut_ptr());
            }

            retval = true;
        }

        (*g_st().curbuf).b_p_pi = saved_pi;
        drop(saved_line_vec);
        drop(next_line_vec);
        drop(allocated_vec);
        drop(prompt_moved);
        g_st().cmdmod.cmod_flags = old_cmod_flags;
        retval
    }
}

/// Delete from cursor to end of line.
pub fn truncate_line(fixpos: c_int) {
    unsafe {
        let lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let col = (**curwin.as_ptr()).w_cursor.col;
        let old_line = ml_get(lnum);
        let mut newp_vec: Vec<c_char> = if col == 0 {
            vec![NUL]
        } else {
            let mut v = Vec::with_capacity(col as usize + 1);
            v.extend_from_slice(std::slice::from_raw_parts(old_line, col as usize));
            v.push(NUL);
            v
        };
        let deleted = ml_get_len(lnum) - col;

        ml_replace(lnum, newp_vec.as_mut_ptr(), true);
        drop(newp_vec);
        inserted_bytes(lnum, (**curwin.as_ptr()).w_cursor.col, deleted, 0);

        if fixpos != 0 && (**curwin.as_ptr()).w_cursor.col > 0 {
            (**curwin.as_ptr()).w_cursor.col -= 1;
        }
    }
}

/// Delete `nlines` lines at the cursor.
pub fn del_lines(nlines: linenr_T, undo: bool) {
    unsafe {
        let first = (**curwin.as_ptr()).w_cursor.lnum;

        if nlines <= 0 {
            return;
        }

        if undo && u_savedel(first, nlines) == FAIL {
            return;
        }

        let mut n: c_int = 0;
        while n < nlines {
            if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
                break;
            }

            ml_delete_flags(first, ML_DEL_MESSAGE);
            n += 1;

            if first > (*g_st().curbuf).b_ml.ml_line_count {
                break;
            }
        }

        (**curwin.as_ptr()).w_cursor.col = 0;
        check_cursor_lnum(&mut **curwin.as_ptr());

        deleted_lines_mark(first, n);
    }
}

/// Returns the length in bytes of the prefix of the given string which introduces a comment.
pub fn get_leader_len(
    line: *mut c_char,
    flags: *mut *mut c_char,
    backward: bool,
    include_space: bool,
) -> c_int {
    unsafe {
        let mut got_com = false;
        let mut part_buf: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
        let mut middle_match_len: usize = 0;
        let mut saved_flags: *mut c_char = ptr::null_mut();

        let mut result: usize = 0;
        let mut i: usize = 0;
        while ascii_iswhite(*line.add(i)) {
            i += 1;
        }

        while *line.add(i) != NUL {
            let mut found_one = false;
            let mut list = (*g_st().curbuf).b_p_com;
            while *list != NUL {
                if !got_com && !flags.is_null() {
                    *flags = list;
                }
                let prev_list = list;
                copy_option_part(
                    &raw mut list,
                    part_buf.as_mut_ptr(),
                    COM_MAX_LEN,
                    c",".as_ptr(),
                );
                let mut string = vim_strchr(part_buf.as_ptr(), b':' as c_int);
                if string.is_null() {
                    continue;
                }
                *string = NUL;
                string = string.add(1);

                if middle_match_len != 0
                    && vim_strchr(part_buf.as_ptr(), c_int::from(COM_MIDDLE as u8)).is_null()
                    && vim_strchr(part_buf.as_ptr(), c_int::from(COM_END as u8)).is_null()
                {
                    break;
                }

                if got_com && vim_strchr(part_buf.as_ptr(), c_int::from(COM_NEST as u8)).is_null() {
                    continue;
                }

                if backward
                    && !vim_strchr(part_buf.as_ptr(), c_int::from(COM_NOBACK as u8)).is_null()
                {
                    continue;
                }

                if ascii_iswhite(*string) {
                    if i == 0 || !ascii_iswhite(*line.add(i - 1)) {
                        continue;
                    }
                    while ascii_iswhite(*string) {
                        string = string.add(1);
                    }
                }
                let mut j: usize = 0;
                while *string.add(j) != NUL && *string.add(j) == *line.add(i + j) {
                    j += 1;
                }
                if *string.add(j) != NUL {
                    continue;
                }

                if !vim_strchr(part_buf.as_ptr(), c_int::from(COM_BLANK as u8)).is_null()
                    && !ascii_iswhite(*line.add(i + j))
                    && *line.add(i + j) != NUL
                {
                    continue;
                }

                if !vim_strchr(part_buf.as_ptr(), c_int::from(COM_MIDDLE as u8)).is_null() {
                    if middle_match_len == 0 {
                        middle_match_len = j;
                        saved_flags = prev_list;
                    }
                    continue;
                }
                if middle_match_len != 0 && j > middle_match_len {
                    middle_match_len = 0;
                }

                if middle_match_len == 0 {
                    i += j;
                }
                found_one = true;
                break;
            }

            if middle_match_len != 0 {
                if !got_com && !flags.is_null() {
                    *flags = saved_flags;
                }
                i += middle_match_len;
                found_one = true;
            }

            if !found_one {
                break;
            }

            result = i;

            while ascii_iswhite(*line.add(i)) {
                i += 1;
            }

            if include_space {
                result = i;
            }

            got_com = true;
            if vim_strchr(part_buf.as_ptr(), c_int::from(COM_NEST as u8)).is_null() {
                break;
            }
        }
        result as c_int
    }
}

/// Return the offset at which the last comment in `line` starts, or -1 if none.
pub fn get_last_leader_offset(line: *mut c_char, flags: *mut *mut c_char) -> c_int {
    unsafe {
        let mut result: c_int = -1;
        let mut lower_check_bound: c_int = 0;
        let mut com_leader: *mut c_char = ptr::null_mut();
        let mut com_flags: *mut c_char = ptr::null_mut();
        let mut part_buf: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];

        let mut i = CStr::from_ptr(line).to_bytes().len() as c_int;
        while {
            i -= 1;
            i >= lower_check_bound
        } {
            let mut found_one = false;
            let mut list = (*g_st().curbuf).b_p_com;
            while *list != NUL {
                let flags_save = list;

                copy_option_part(
                    &raw mut list,
                    part_buf.as_mut_ptr(),
                    COM_MAX_LEN,
                    c",".as_ptr(),
                );
                let mut string = vim_strchr(part_buf.as_ptr(), b':' as c_int);
                if string.is_null() {
                    continue;
                }
                *string = NUL;
                string = string.add(1);
                com_leader = string;

                if ascii_iswhite(*string) {
                    if i == 0 || !ascii_iswhite(*line.add((i - 1) as usize)) {
                        continue;
                    }
                    while ascii_iswhite(*string) {
                        string = string.add(1);
                    }
                }
                let mut j: usize = 0;
                while *string.add(j) != NUL && *string.add(j) == *line.add(i as usize + j) {
                    j += 1;
                }
                if *string.add(j) != NUL {
                    continue;
                }

                if !vim_strchr(part_buf.as_ptr(), c_int::from(COM_BLANK as u8)).is_null()
                    && !ascii_iswhite(*line.add(i as usize + j))
                    && *line.add(i as usize + j) != NUL
                {
                    continue;
                }

                if !vim_strchr(part_buf.as_ptr(), c_int::from(COM_MIDDLE as u8)).is_null() {
                    let mut k: c_int = 0;
                    while k <= i && ascii_iswhite(*line.add(k as usize)) {
                        k += 1;
                    }
                    if k < i {
                        continue;
                    }
                }

                found_one = true;
                if !flags.is_null() {
                    *flags = flags_save;
                }
                com_flags = flags_save;
                break;
            }

            if found_one {
                let mut part_buf2: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];

                result = i;
                if !vim_strchr(part_buf.as_ptr(), c_int::from(COM_NEST as u8)).is_null() {
                    continue;
                }

                lower_check_bound = i;

                while ascii_iswhite(*com_leader) {
                    com_leader = com_leader.add(1);
                }
                let len1 = CStr::from_ptr(com_leader).to_bytes().len() as c_int;

                let mut list = (*g_st().curbuf).b_p_com;
                while *list != NUL {
                    let flags_save = list;

                    copy_option_part(
                        &raw mut list,
                        part_buf2.as_mut_ptr(),
                        COM_MAX_LEN,
                        c",".as_ptr(),
                    );
                    if flags_save == com_flags {
                        continue;
                    }
                    let mut string = vim_strchr(part_buf2.as_ptr(), b':' as c_int);
                    if string.is_null() {
                        continue;
                    }
                    string = string.add(1);
                    while ascii_iswhite(*string) {
                        string = string.add(1);
                    }
                    let len2 = CStr::from_ptr(string).to_bytes().len() as c_int;
                    if len2 == 0 {
                        continue;
                    }

                    let mut off = len2.min(i);
                    while off > 0 && off + len1 > len2 {
                        off -= 1;
                        let cmp_len = (len2 - off) as usize;
                        if std::slice::from_raw_parts(string.add(off as usize), cmp_len)
                            == std::slice::from_raw_parts(com_leader, cmp_len)
                        {
                            lower_check_bound = lower_check_bound.min(i - off);
                        }
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_normal_and_strcat_raw_helpers() {
        assert!(replace_normal(REPLACE_FLAG));
        assert!(!replace_normal(REPLACE_FLAG | VREPLACE_FLAG));
        assert!(!replace_normal(MODE_INSERT));
    }
}
