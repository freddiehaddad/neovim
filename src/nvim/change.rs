#![forbid(unsafe_code)]

//! Functions related to changing text (`src/nvim/change.c`).

use crate::autocmd::{EVENT_FILECHANGEDRO, apply_autocmds, aucmd_defer_modified};
use crate::buffer::{
    BF_NEVERLOADED, BF_NEW, FileBuf, K_UIMESSAGES, MAXLNUM, bt_dontwrite, bt_prompt,
    buf_inc_changedtick, buf_meta_total,
};
use crate::buffer_updates::buf_updates_send_changes;
use crate::charset::{
    ascii_isdigit, ascii_iswhite, getdigits_int, getwhitecols_curline, vim_strnsize,
};
use crate::context::INIT_FMARKV;
use crate::cursor::{
    Colnr, CursorPos, Linenr, check_cursor_lnum, check_visual_pos, coladvance_force, getviscol,
};
use crate::diff::{diff_internal, diff_lnum_win, diff_update_line};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, redraw_buf_status_later, redraw_later, redraw_winline,
    set_must_redraw, showmode,
};
use crate::eval::vars::{VV_WARNINGMSG, set_vim_var_string};
use crate::extmark::{
    Bcount, K_EXTMARK_NOOP, K_EXTMARK_UNDO, extmark_adjust, extmark_splice, extmark_splice_cols,
};
use crate::fold::{find_wl_entry, fold_update, has_any_folding, has_folding_win};
use crate::globals::*;
use crate::indent::{
    copy_indent, fixthisline, get_indent, get_lisp_indent, get_sw_value, indent_size_ts, may_do_si,
    set_indent, use_indentexpr_for_lisp,
};
use crate::indent_c::{check_linecomment, cin_is_cinword, do_c_expr_indent, in_cinkeys};
use crate::insert::{prompt_text, replace_push, replace_push_nul, truncate_spaces};
use crate::insexpand::ins_compl_active;
use crate::mark::{JUMPLISTSIZE, mark_adjust, mark_col_adjust, mark_view_make, reset_fmark};
use crate::marktree::{K_MTMETA_INLINE, K_MTMETA_LINES};
use crate::mbyte::{
    mb_adjust_cursor, ptr2cells, utf_char2slice, utf_composinglike, utf_iscomposing_first,
    utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::mcursor::mc_replaying;
use crate::memline::{
    ml_add_deleted_len, ml_append, ml_delete_flags, ml_get, ml_get_len, ml_line_alloced,
    ml_open_file, ml_replace, ml_setflags,
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
use crate::plines::{getvcol, linetabsize_disp_eol, win_chartabsize};
use crate::search::{BACKWARD, FORWARD, findmatch, linewhite, showmatch};
use crate::spell::spell_check_window;
use crate::state::{MODE_INSERT, REPLACE_FLAG, VREPLACE_FLAG, virtual_active};
use crate::strings::{concat_str, tr};
use crate::textformat::{comp_textwidth, has_format_option};
use crate::ui::{ui_active, ui_has};
use crate::undo::{curbuf_is_changed_fn, u_clearline, u_save_cursor, u_savedel};
use crate::window::WinInfoInfo;
use std::ffi::CStr;
use std::ptr;

use crate::arglist::ML_EMPTY;
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_docmd::{CMOD_KEEPJUMPS, CMOD_LOCKMARKS};
use crate::regexp::{GRAPHEME_STATE_INIT, MAXCOL};

pub const OPENLINE_DELSPACES: i32 = 0x01;
pub const OPENLINE_DO_COM: i32 = 0x02;
pub const OPENLINE_KEEPTRAIL: i32 = 0x04;
pub const OPENLINE_MARKFIX: i32 = 0x08;
pub const OPENLINE_COM_LIST: i32 = 0x10;
pub const OPENLINE_FORMAT: i32 = 0x20;
pub const OPENLINE_FORCE_INDENT: i32 = 0x40;

pub(crate) const TAB: u8 = b'\t';

pub(crate) const MB_MAXCHAR: usize = 6;
pub(crate) const COM_MAX_LEN: usize = 50;

const COM_NEST: u8 = b'n';
const COM_BLANK: u8 = b'b';
pub(crate) const COM_START: u8 = b's';
pub(crate) const COM_MIDDLE: u8 = b'm';
pub(crate) const COM_END: u8 = b'e';
const COM_AUTO_END: u8 = b'x';
pub(crate) const COM_FIRST: u8 = b'f';
pub(crate) const COM_LEFT: u8 = b'l';
pub(crate) const COM_RIGHT: u8 = b'r';
const COM_NOBACK: u8 = b'O';

pub(crate) const KEY_OPEN_FORW: i32 = 0x101;
pub(crate) const KEY_OPEN_BACK: i32 = 0x102;

pub(crate) const SIN_INSERT: i32 = 2;
pub(crate) const SIN_NOMARK: i32 = 8;

pub(crate) const ML_DEL_MESSAGE: i32 = 1;

pub(crate) const K_OPT_VE_FLAG_ONEMORE: u32 = 0x08;
pub(crate) const K_CPO_DOLLAR: i32 = b'$' as i32;
pub(crate) const K_CPO_LISTWM: i32 = b'L' as i32;
const K_FO_NO_OPEN_COMS: i32 = b'/' as i32;

pub(crate) const HLF_W: i32 = 26;

#[inline]
pub const fn replace_normal(s: i32) -> bool {
    (s & REPLACE_FLAG) != 0 && (s & VREPLACE_FLAG) == 0
}

#[inline]
fn for_all_tab_windows(mut f: impl FnMut(&'static mut WinInfoInfo)) {
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            f(wp);
        }
    }
}

/// If the file is readonly, give a warning message with the first change.
pub fn change_warning(buf: &mut FileBuf, col: i32) {
    const W_READONLY: &CStr = c"W10: Warning: Changing a readonly file";

    let buf_ptr = crate::globals::BufPtr::from(&mut *buf);
    if !buf_ptr.b_did_warn.get()
        && !curbuf_is_changed_fn()
        && !g_st().autocmd_busy.get()
        && buf_ptr.b_p_ro.get()
    {
        buf_ptr.b_ro_locked.set(buf_ptr.b_ro_locked.get() + (1));
        apply_autocmds(EVENT_FILECHANGEDRO, None, None, false, buf_ptr.opt_mut());
        buf_ptr.b_ro_locked.set(buf_ptr.b_ro_locked.get() - (1));
        if !buf_ptr.b_p_ro.get() {
            return;
        }
        msg_start();
        if g_st().msg_row.get() == g_st().rows_global.get() - 1 {
            g_st().msg_col.set(col);
        }
        msg_source(HLF_W);
        msg_ext_set_kind(c"wmsg");
        msg_puts_hl(tr(W_READONLY), HLF_W, true);
        set_vim_var_string(VV_WARNINGMSG, Some(tr(W_READONLY)), -1);
        msg_clr_eos();
        msg_end();
        if g_st().msg_silent.get() == 0 && !g_st().silent_mode.get() && ui_active() != 0 {
            msg_delay(1002, true);
        }
        buf.b_did_warn.set(true);
        g_st().redraw_cmdline.set(false);
        if g_st().msg_row.get() < g_st().rows_global.get() - 1 {
            showmode();
        }
    }
}

/// Call this function when something in a buffer is changed.
pub fn changed(buf: &mut FileBuf) {
    if buf.b_changed.get() == 0 {
        let save_msg_scroll = g_st().msg_scroll.get();

        change_warning(buf, 0);

        if buf.b_may_swap.get() && !bt_dontwrite(Some(&*buf)) {
            let save_need_wait_return = g_st().need_wait_return.get();

            g_st().need_wait_return.set(false);
            ml_open_file(buf);

            if g_st().need_wait_return.get()
                && g_st().emsg_silent.get() == 0
                && !g_st().in_assert_fails.get()
                && !ui_has(K_UIMESSAGES)
            {
                msg_delay(2002, true);
                wait_return(i32::from(true));
                g_st().msg_scroll.set(save_msg_scroll);
            } else {
                g_st().need_wait_return.set(save_need_wait_return);
            }
        }
        changed_internal(buf);
    }
    buf_inc_changedtick(buf);

    if g_st().curbuf.get() == buf {
        g_st().search_global.hl_match.set(false);
    }
}

/// Internal part of `changed()`, no user interaction.
pub fn changed_internal(buf: &mut FileBuf) {
    let was_changed = buf.b_changed.get() != 0;
    buf.b_changed.set(1);
    ml_setflags(&mut *buf);
    redraw_buf_status_later(buf);
    g_st().redraw_tabline.set(true);
    g_st().need_maketitle.set(true);
    if !was_changed {
        aucmd_defer_modified(&mut *buf, true);
    }
}

/// Invalidate a window's `w_valid` flags and `w_lines[]` entries after changing lines.
pub fn changed_lines_invalidate_win(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    col: Colnr,
    mut lnume: Linenr,
    xtra: Linenr,
) {
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
    if wp.w_botline.get() >= lnum {
        if xtra < 0 {
            invalidate_botline_win(&mut *wp);
        } else {
            approximate_botline_win(wp);
        }
    }

    if (xtra < 0
        && wp.w_onebuf_opt.wo_wrap.get()
        && buf_meta_total(wp.buf_mut(), K_MTMETA_INLINE) != 0)
        || (xtra != 0 && buf_meta_total(wp.buf_mut(), K_MTMETA_LINES) != 0)
    {
        lnume += 1;
    }

    let valid_len = wp.w_lines_valid.get().max(0) as usize;
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

pub fn changed_topline_skipcol(wp: &mut WinInfoInfo, line1: Linenr, line2: Linenr) {
    if wp.w_skipcol.get() > 0
        && wp.w_topline.get() >= line1
        && wp.w_topline.get() <= line2
        && linetabsize_disp_eol(wp, wp.w_topline.get())
            <= wp.w_skipcol.get() + sms_marker_overlap(wp, -1)
    {
        wp.w_skipcol.set(0);
    }
}

/// Like `changed_lines_invalidate_win()`, but for all windows displaying a buffer.
pub fn changed_lines_invalidate_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    lnume: Linenr,
    xtra: Linenr,
) {
    for_all_tab_windows(|wp| {
        if wp.w_buffer.get() == buf {
            changed_lines_invalidate_win(&mut *wp, lnum, col, lnume, xtra);
        }
    });
}

/// Common code for when a change was made.
fn changed_common(buf: &mut FileBuf, mut lnum: Linenr, col: Colnr, lnume: Linenr, xtra: Linenr) {
    changed(buf);

    for win in for_all_windows_mut() {
        if win.w_buffer.get() == (&raw mut *buf) {
            if win.w_onebuf_opt.wo_diff.get() && diff_internal() != 0 {
                curtab_mut().tp_diff_update.set(1);
                diff_update_line(lnum);
            }

            if xtra != 0 {
                win.w_redr_status.set(true);
            }
        }
    }

    if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 {
        let mut view = INIT_FMARKV;
        if curwin_mut().w_buffer.get() == (&raw mut *buf)
            && lnum >= curwin_mut().w_topline.get()
            && lnum <= curwin_mut().w_botline.get()
        {
            view = mark_view_make(curwin_mut(), curwin_mut().w_cursor);
        }
        reset_fmark(
            &mut buf.b_last_change,
            CursorPos {
                lnum,
                col,
                coladd: 0,
            },
            buf.b_fnum.get(),
            view,
        );
    }

    if (g_st().cmdmod.cmod_flags & CMOD_KEEPJUMPS) == 0 && !mc_replaying() {
        if buf.b_new_change.get() || buf.b_changelistlen.get() == 0 {
            let add = if buf.b_changelistlen.get() == 0 {
                true
            } else {
                let p = &buf.b_changelist[(buf.b_changelistlen.get() - 1) as usize].mark;
                if p.lnum != lnum {
                    true
                } else {
                    let mut cols = comp_textwidth(false);
                    if cols == 0 {
                        cols = 79;
                    }
                    p.col + cols < col || col + cols < p.col
                }
            };
            if add {
                buf.b_new_change.set(false);

                if buf.b_changelistlen.get() == JUMPLISTSIZE {
                    buf.b_changelistlen.set(JUMPLISTSIZE - 1);
                    buf.b_changelist.rotate_left(1);
                    for_all_tab_windows(|wp| {
                        if wp.w_buffer.get() == (&raw mut *buf) && wp.w_changelistidx.get() > 0 {
                            wp.w_changelistidx.set(wp.w_changelistidx.get() - (1));
                        }
                    });
                }
                for_all_tab_windows(|wp| {
                    if wp.w_buffer.get() == (&raw mut *buf)
                        && wp.w_changelistidx.get() == buf.b_changelistlen.get()
                    {
                        wp.w_changelistidx.set(wp.w_changelistidx.get() + (1));
                    }
                });
                buf.b_changelistlen.set(buf.b_changelistlen.get() + (1));
            }
        }
        buf.b_changelist[(buf.b_changelistlen.get() - 1) as usize] = buf.b_last_change.clone();
        if curwin_mut().w_buffer.get() == (&raw mut *buf) {
            curwin_mut().w_changelistidx.set(buf.b_changelistlen.get());
        }
    }

    if curwin_mut().w_buffer.get() == (&raw mut *buf) && g_st().visual_global.active {
        check_visual_pos();
    }

    for_all_tab_windows(|wp| {
        if wp.w_buffer.get() == (&raw mut *buf) {
            if !g_st().redraw_not_allowed.get() && wp.w_redr_type.get() < UPD_VALID {
                wp.w_redr_type.set(UPD_VALID);
            }

            if xtra != 0 && wp.w_redraw_top.get() != 0 {
                redraw_later(Some(&mut *wp), UPD_NOT_VALID);
            }

            let mut last = lnume + xtra - 1;

            if last < wp.w_topline.get() {
                wp.w_skipcol.set(0);
            }
            changed_topline_skipcol(&mut *wp, lnum, lnume - 1);

            fold_update(&mut *(wp), lnum, last);

            let mut folded = has_folding_win(&mut *wp, lnum, Some(&mut lnum), None, false, None);
            if wp.w_cursor.lnum == lnum {
                wp.w_cline_folded.set(folded);
            }
            folded = has_folding_win(&mut *wp, last, None, Some(&mut last), false, None);
            if wp.w_cursor.lnum == last {
                wp.w_cline_folded.set(folded);
            }

            changed_lines_invalidate_win(&mut *wp, lnum, col, lnume, xtra);

            if has_any_folding(&mut *wp) != 0 {
                set_topline(wp, wp.w_topline.get());
            }

            if wp.w_onebuf_opt.wo_rnu.get() && xtra != 0 {
                wp.w_last_cursor_lnum_rnu.set(0);
            }

            if wp.w_onebuf_opt.wo_cul.get() && wp.w_last_cursorline.get() >= lnum {
                if wp.w_last_cursorline.get() < lnume {
                    wp.w_last_cursorline.set(0);
                } else {
                    wp.w_last_cursorline
                        .set(wp.w_last_cursorline.get() + (xtra));
                }
            }
        }

        if wp == curwin_ptr() && xtra != 0 && g_st().search_hl_has_cursor_lnum.get() >= lnum {
            g_st()
                .search_hl_has_cursor_lnum
                .set(g_st().search_hl_has_cursor_lnum.get() + (xtra));
        }
    });

    set_must_redraw(UPD_VALID);

    if g_st().last_cursormoved_win.get() == curwin_ptr()
        && curwin_mut().w_buffer.get() == (&raw mut *buf)
        && lnum <= curwin_mut().w_cursor.lnum
        && lnume + xtra.abs() > curwin_mut().w_cursor.lnum
    {
        g_st().last_cursormoved.lnum = 0;
    }
}

/// Changed bytes within a single line for the current buffer.
pub fn changed_bytes(lnum: Linenr, col: Colnr) {
    changed_lines_redraw_buf(curbuf_mut(), lnum, lnum + 1, 0);
    changed_common(curbuf_mut(), lnum, col, lnum + 1, 0);
    if spell_check_window(curwin_mut())
        && lnum < curbuf_mut().b_ml.ml_line_count.get()
        && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_DOLLAR)
    {
        redraw_winline(crate::globals::curwin_mut(), lnum + 1);
    }
    buf_updates_send_changes(curbuf_mut(), lnum, 1, 1);

    if curwin_mut().w_onebuf_opt.wo_diff.get() {
        for wp in for_all_windows_ptr() {
            if wp.w_onebuf_opt.wo_diff.get() && wp != curwin_ptr() {
                redraw_later(Some(&mut *wp), UPD_VALID);
                let wlnum = diff_lnum_win(lnum, &mut *wp);
                if wlnum > 0 {
                    changed_lines_redraw_buf((*wp).buf_mut(), wlnum, wlnum + 1, 0);
                }
            }
        }
    }
}

/// Insert/delete bytes at column.
pub fn inserted_bytes(lnum: Linenr, start_col: Colnr, old_col: i32, new_col: i32) {
    if g_st().curbuf_splice_pending.get() == 0 {
        extmark_splice_cols(
            curbuf_mut(),
            lnum - 1,
            start_col,
            old_col,
            new_col,
            K_EXTMARK_UNDO,
        );
    }

    changed_bytes(lnum, start_col);
}

/// Appended `count` lines below line `lnum` in the given buffer.
pub fn appended_lines_buf(buf: &mut FileBuf, lnum: Linenr, count: Linenr) {
    changed_lines(buf, lnum + 1, 0, lnum + 1, count, true);
}

/// Appended `count` lines below line `lnum` in the current buffer.
pub fn appended_lines(lnum: Linenr, count: Linenr) {
    appended_lines_buf(curbuf_mut(), lnum, count);
}

/// Like `appended_lines()`, but adjust marks first.
pub fn appended_lines_mark(lnum: Linenr, count: i32) {
    mark_adjust(lnum + 1, MAXLNUM, count, 0, K_EXTMARK_UNDO);
    changed_lines(curbuf_mut(), lnum + 1, 0, lnum + 1, count, true);
}

/// Deleted `count` lines at line `lnum` in the given buffer.
pub fn deleted_lines_buf(buf: &mut FileBuf, lnum: Linenr, count: Linenr) {
    changed_lines(buf, lnum, 0, lnum + count, -count, true);
}

/// Deleted `count` lines at line `lnum` in the current buffer.
pub fn deleted_lines(lnum: Linenr, count: Linenr) {
    deleted_lines_buf(crate::globals::curbuf_mut(), lnum, count);
}

/// Like `deleted_lines()`, but adjust marks first.
pub fn deleted_lines_mark(lnum: Linenr, count: i32) {
    let made_empty = count > 0 && (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0;

    mark_adjust(lnum, lnum + count - 1, MAXLNUM, -count, K_EXTMARK_NOOP);
    extmark_adjust(
        curbuf_mut(),
        lnum,
        lnum + count - 1,
        MAXLNUM,
        -count + i32::from(made_empty),
        K_EXTMARK_UNDO,
    );
    changed_lines(curbuf_mut(), lnum, 0, lnum + count, -count, true);
}

/// Marks the area to be redrawn after a change.
pub fn changed_lines_redraw_buf(buf: &mut FileBuf, lnum: Linenr, mut lnume: Linenr, xtra: Linenr) {
    if xtra != 0 && buf.b_marktree.n_keys > 0 {
        lnume += 1 + i32::from(xtra < 0 && buf_meta_total(buf, K_MTMETA_LINES) != 0);
    }

    if buf.b_mod_set.get() {
        buf.b_mod_top.set(buf.b_mod_top.get().min(lnum));
        if lnum < buf.b_mod_bot.get() {
            buf.b_mod_bot.set(buf.b_mod_bot.get() + (xtra));
            buf.b_mod_bot.set(buf.b_mod_bot.get().max(lnum));
        }
        buf.b_mod_bot.set(buf.b_mod_bot.get().max(lnume + xtra));
        buf.b_mod_xlines.set(buf.b_mod_xlines.get() + (xtra));
    } else {
        buf.b_mod_set.set(true);
        buf.b_mod_top.set(lnum);
        buf.b_mod_bot.set(lnume + xtra);
        buf.b_mod_xlines.set(xtra);
    }
}

/// Changed lines for a buffer.
pub fn changed_lines(
    buf: &mut FileBuf,
    lnum: Linenr,
    col: Colnr,
    lnume: Linenr,
    xtra: Linenr,
    do_buf_event: bool,
) {
    changed_lines_redraw_buf(buf, lnum, lnume, xtra);

    if xtra == 0
        && curwin_mut().w_onebuf_opt.wo_diff.get()
        && curwin_mut().w_buffer.get() == (&raw mut *buf)
        && diff_internal() == 0
    {
        for wp in for_all_windows_ptr() {
            if wp.w_onebuf_opt.wo_diff.get() && wp != curwin_ptr() {
                redraw_later(Some(&mut *wp), UPD_VALID);
                let wlnum = diff_lnum_win(lnum, &mut *wp);
                if wlnum > 0 {
                    changed_lines_redraw_buf((*wp).buf_mut(), wlnum, lnume - lnum + wlnum, 0);
                }
            }
        }
    }

    changed_common(buf, lnum, col, lnume, xtra);

    if do_buf_event {
        let num_added = i64::from(lnume + xtra - lnum);
        let num_removed = i64::from(lnume - lnum);
        buf_updates_send_changes(buf, lnum, num_added, num_removed);
    }
}

/// Called when the changed flag must be reset for buffer `buf`.
pub fn unchanged(buf: &mut FileBuf, ff: bool, always_inc_changedtick: bool) {
    if buf.b_changed.get() != 0 || (ff && file_ff_differs(buf, false)) {
        let was_changed = buf.b_changed.get() != 0;
        buf.b_changed.set(0);
        ml_setflags(buf);
        if ff {
            save_file_ff(buf);
        }
        redraw_buf_status_later(buf);
        g_st().redraw_tabline.set(true);
        g_st().need_maketitle.set(true);
        buf_inc_changedtick(buf);
        if was_changed {
            aucmd_defer_modified(buf, false);
        }
    } else if always_inc_changedtick {
        buf_inc_changedtick(buf);
    }
}

/// Save the current values of `'fileformat'` and `'fileencoding'`.
pub fn save_file_ff(buf: &mut FileBuf) {
    buf.b_start_ffc.set(i32::from(buf.b_p_ff.first_byte()));
    buf.b_start_eof.set(buf.b_p_eof.get());
    buf.b_start_eol.set(buf.b_p_eol.get());
    buf.b_start_bomb.set(buf.b_p_bomb.get());

    let p_fenc_bytes = buf.b_p_fenc.as_bytes();
    if buf
        .b_start_fenc
        .get()
        .as_ref()
        .is_none_or(|s| s.to_bytes() != p_fenc_bytes)
    {
        buf.b_start_fenc.set(Some(buf.b_p_fenc.to_owned()));
    }
}

/// Return true if `'fileformat'` and/or `'fileencoding'` has a different value
/// from when editing started.
pub fn file_ff_differs(buf: &mut FileBuf, ignore_empty: bool) -> bool {
    if (buf.b_flags.get() & BF_NEVERLOADED) != 0 {
        return false;
    }
    if ignore_empty
        && (buf.b_flags.get() & BF_NEW) != 0
        && buf.b_ml.ml_line_count.get() == 1
        && crate::memline::ml_get_buf_slice(buf, 1).is_empty()
    {
        return false;
    }
    if buf.b_start_ffc.get() != i32::from(buf.b_p_ff.first_byte()) {
        return true;
    }
    if (buf.b_p_bin.get() || !buf.b_p_fixeol.get())
        && (buf.b_start_eof.get() != buf.b_p_eof.get()
            || buf.b_start_eol.get() != buf.b_p_eol.get())
    {
        return true;
    }
    if !buf.b_p_bin.get() && buf.b_start_bomb.get() != buf.b_p_bomb.get() {
        return true;
    }
    let Some(ref start_fenc) = buf.b_start_fenc.get() else {
        return !buf.b_p_fenc.is_empty();
    };
    start_fenc.to_bytes() != buf.b_p_fenc.as_bytes()
}

/// Insert string `p` at the cursor position. Stops at a NUL byte.
pub fn ins_bytes(p: &[u8]) {
    let len = p.iter().position(|&b| b == 0).unwrap_or(p.len());
    ins_bytes_len(p, len);
}

/// Insert string `p` with length `len` at the cursor position.
pub fn ins_bytes_len(p: &[u8], len: usize) {
    let p = &p[..len.min(p.len())];
    let mut i: usize = 0;
    while i < p.len() {
        let n = crate::mbyte::utfc_slice2len(&p[i..]).min(p.len() - i);
        ins_char_bytes(&p[i..], n);
        i += n;
    }
}

/// Insert or replace a single character at the cursor position.
pub fn ins_char(c: i32) {
    let mut buf: [u8; MB_MAXCHAR + 1] = [0; MB_MAXCHAR + 1];
    let n = utf_char2slice(c, &mut buf);

    if buf[0] == 0 {
        buf[0] = b'\n';
    }
    ins_char_bytes(&buf, n);
}

pub fn ins_char_bytes(buf: &[u8], charlen: usize) {
    let buf_slice = &buf[..charlen.min(buf.len())];
    let mut buf_cstr = [0u8; MB_MAXCHAR + 1];
    let copy_len = buf_slice.len().min(MB_MAXCHAR);
    buf_cstr[..copy_len].copy_from_slice(&buf_slice[..copy_len]);
    let buf_bytes = &buf_cstr[..copy_len];
    {
        if virtual_active(curwin_mut()) && curwin_mut().w_cursor.coladd > 0 {
            coladvance_force(getviscol());
        }

        let col = curwin_mut().w_cursor.col as usize;
        let lnum = curwin_mut().w_cursor.lnum;
        let oldp_with_nul = ml_get(lnum).to_bytes_with_nul();
        let oldp_bytes = &oldp_with_nul[..oldp_with_nul.len() - 1];
        let linelen = ml_get_len(lnum) as usize + 1;

        let mut oldlen: usize = 0;
        let mut newlen: usize = charlen;

        if (g_st().state_global.get() & REPLACE_FLAG) != 0 {
            if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
                let old_list = curwin_mut().w_onebuf_opt.wo_list.get();
                if old_list && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_LISTWM)
                {
                    curwin_mut().w_onebuf_opt.wo_list.set(false);
                }
                let mut vcol: Colnr = 0;
                let mut cursor = curwin_mut().w_cursor;
                getvcol(curwin_mut(), &mut cursor, None, Some(&mut vcol), None, 0);
                curwin_mut().w_cursor = cursor;
                let new_vcol = vcol + win_chartabsize(curwin_mut(), buf_bytes, vcol);
                while col + oldlen < oldp_bytes.len() && vcol < new_vcol {
                    let rem = &oldp_bytes[col + oldlen..];
                    vcol += win_chartabsize(curwin_mut(), rem, vcol);
                    if vcol > new_vcol && rem.first() == Some(&TAB) {
                        break;
                    }
                    oldlen += utfc_ptr2len(rem) as usize;
                    if vcol > new_vcol {
                        newlen += (vcol - new_vcol) as usize;
                    }
                }
                curwin_mut().w_onebuf_opt.wo_list.set(old_list);
            } else if col < oldp_bytes.len() {
                oldlen = utfc_ptr2len(&oldp_bytes[col..]) as usize;
            }

            replace_push_nul();
            let push_end = (col + oldlen).min(oldp_with_nul.len());
            replace_push(&oldp_with_nul[col.min(push_end)..push_end]);
        }

        let old_slice = &oldp_with_nul[..linelen.min(oldp_with_nul.len())];
        let mut newp_vec = vec![0u8; linelen + newlen - oldlen];
        newp_vec[..col].copy_from_slice(&old_slice[..col]);
        if linelen > col + oldlen {
            newp_vec[col + newlen..].copy_from_slice(&old_slice[col + oldlen..]);
        }
        newp_vec[col..col + charlen].copy_from_slice(buf_slice);
        newp_vec[col + charlen..col + newlen].fill(b' ');

        ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
        drop(newp_vec);
        inserted_bytes(lnum, col as Colnr, oldlen as i32, newlen as i32);

        if opt_vars().p_sm.get()
            && (g_st().state_global.get() & MODE_INSERT) != 0
            && g_st().msg_silent.get() == 0
            && !ins_compl_active()
        {
            showmatch(crate::mbyte::utf_slice2char(buf_slice));
        }

        if !opt_vars().p_ri.get() || (g_st().state_global.get() & REPLACE_FLAG) != 0 {
            curwin_mut().w_cursor.col += charlen as Colnr;
        }
    }
}

/// Insert a string at the cursor position.
pub fn ins_str(s: &[u8], slen: usize) {
    let lnum = curwin_mut().w_cursor.lnum;

    if virtual_active(curwin_mut()) && curwin_mut().w_cursor.coladd > 0 {
        coladvance_force(getviscol());
    }

    let col = curwin_mut().w_cursor.col;
    let old_slice = crate::memline::ml_get_slice(lnum);
    let col_usize = col as usize;
    let s_slice = &s[..slen.min(s.len())];

    let mut newp_vec = Vec::with_capacity(old_slice.len() + slen + 1);
    newp_vec.extend_from_slice(&old_slice[..col_usize]);
    newp_vec.extend_from_slice(s_slice);
    newp_vec.extend_from_slice(&old_slice[col_usize..]);
    newp_vec.push(0);

    ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
    drop(newp_vec);
    inserted_bytes(lnum, col, 0, slen as i32);
    curwin_mut().w_cursor.col += slen as Colnr;
}

/// Delete one character under the cursor.
pub fn del_char(fixpos: bool) -> bool {
    mb_adjust_cursor();
    if crate::cursor::get_cursor_pos_slice().is_empty() {
        return false;
    }
    del_chars(1, i32::from(fixpos))
}

/// Like `del_bytes()`, but delete characters instead of bytes.
pub fn del_chars(count: i32, fixpos: i32) -> bool {
    let mut bytes: Colnr = 0;
    let mut rem = crate::cursor::get_cursor_pos_slice();
    let mut i: i32 = 0;
    while i < count && !rem.is_empty() {
        let l = crate::mbyte::utfc_slice2len(rem);
        bytes += l as Colnr;
        rem = &rem[l..];
        i += 1;
    }
    del_bytes(bytes, fixpos != 0, true)
}

/// Delete `count` bytes under the cursor.
pub fn del_bytes(mut count: Colnr, fixpos_arg: bool, use_delcombine: bool) -> bool {
    let lnum = curwin_mut().w_cursor.lnum;
    let mut col = curwin_mut().w_cursor.col;
    let mut fixpos = fixpos_arg;
    let oldp_with_nul = ml_get(lnum).to_bytes_with_nul();
    let oldp_bytes = &oldp_with_nul[..oldp_with_nul.len() - 1];
    let oldlen = ml_get_len(lnum);

    if col >= oldlen {
        return false;
    }
    if count == 0 {
        return true;
    }
    if count < 1 {
        siemsg!(
            c"E292: Invalid count for del_bytes(): %ld",
            i64::from(count),
        );
        return false;
    }

    let col_u = col as usize;
    if opt_vars().p_deco.get()
        && use_delcombine
        && utfc_ptr2len(oldp_bytes.get(col_u..).unwrap_or(&[])) >= count
    {
        let p0 = oldp_bytes.get(col_u..).unwrap_or(&[]);
        let step0 = (utf_ptr2len(p0) as usize).min(p0.len());
        let mut state = GRAPHEME_STATE_INIT;
        if utf_composinglike(p0, &p0[step0..], Some(&mut state)) {
            let mut n = col;
            loop {
                col = n;
                let rem_n = oldp_bytes.get(n as usize..).unwrap_or(&[]);
                count = utf_ptr2len(rem_n);
                n += count;
                if !utf_composinglike(
                    oldp_bytes.get(col as usize..).unwrap_or(&[]),
                    oldp_bytes.get(n as usize..).unwrap_or(&[]),
                    Some(&mut state),
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
            && g_st().restart_edit.get() == 0
            && (get_ve_flags(curwin_mut()) & K_OPT_VE_FLAG_ONEMORE) == 0
        {
            curwin_mut().w_cursor.col -= 1;
            curwin_mut().w_cursor.coladd = 0;
            curwin_mut().w_cursor.col -=
                crate::mbyte::utf_head_off_slice(oldp_bytes, curwin_mut().w_cursor.col as usize);
        }
        count = oldlen - col;
        movelen = 1;
    }
    let newlen = oldlen - count;

    if ml_line_alloced() == 0 {
        let old_slice =
            &oldp_with_nul[..((col + count + movelen) as usize).min(oldp_with_nul.len())];
        let mut newp_vec = vec![0u8; newlen as usize + 1];
        newp_vec[..col as usize].copy_from_slice(&old_slice[..col as usize]);
        newp_vec[col as usize..col as usize + movelen as usize]
            .copy_from_slice(&old_slice[(col + count) as usize..]);
        ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
        drop(newp_vec);
    } else {
        let ml = &mut curbuf_mut().b_ml;
        ml_add_deleted_len(&ml.ml_line_buf[..oldlen as usize]);

        ml.ml_line_buf.copy_within(
            (col + count) as usize..(col + count + movelen) as usize,
            col as usize,
        );

        ml.ml_line_textlen.set(newlen + 1);
        ml.ml_line_buf.truncate((newlen + 1) as usize);
    }

    inserted_bytes(lnum, col, count, 0);

    true
}

/// Add a new line below or above the current line.
pub fn open_line(
    dir: i32,
    flags: i32,
    second_line_indent: i32,
    mut did_do_comment: Option<&mut bool>,
) -> bool {
    let mut next_line_vec: Vec<u8> = Vec::new();
    let mut p_extra_idx: Option<usize> = None;
    let mut less_cols: Colnr = 0;
    let mut less_cols_off: Colnr = 0;
    let mut newcol: Colnr = 0;
    let mut newindent: i32 = 0;
    let mut trunc_line = false;
    let mut retval = false;
    let mut extra_len: i32 = 0;
    let mut lead_len: i32;
    let mut comment_start: i32 = 0;
    let mut lead_flags: Option<&'static CStr> = None;
    let mut leader_idx: usize = 0;
    let mut allocated_vec: Vec<u8> = Vec::new();
    let mut saved_char: u8 = 0;
    let do_si = may_do_si();
    let mut no_si = false;
    let mut first_char: i32 = 0;
    let saved_pi = curbuf_mut().b_p_pi.get();

    let lnum = curwin_mut().w_cursor.lnum;
    let mincol = curwin_mut().w_cursor.col + 1;

    let mut saved_line_vec = crate::cursor::get_cursor_line_slice().to_vec();
    saved_line_vec.push(0);

    if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
        if curwin_mut().w_cursor.lnum < g_st().orig_line_count.get() {
            let next_lnum = curwin_mut().w_cursor.lnum + 1;
            next_line_vec = crate::memline::ml_get_slice(next_lnum).to_vec();
            next_line_vec.push(0);
        } else {
            next_line_vec = vec![0];
        }

        replace_push_nul();
        replace_push_nul();
        let col_u = curwin_mut().w_cursor.col as usize;
        replace_push(crate::memory::bytes_until_nul(&saved_line_vec[col_u..]));

        saved_line_vec[col_u] = 0;
    }

    if (g_st().state_global.get() & MODE_INSERT) != 0
        && (g_st().state_global.get() & VREPLACE_FLAG) == 0
    {
        let col_u = curwin_mut().w_cursor.col as usize;
        p_extra_idx = Some(col_u);
        if do_si {
            let rem = crate::charset::skipwhite_slice(&saved_line_vec[col_u..]);
            first_char = i32::from(rem.first().copied().unwrap_or(0));
        }
        extra_len = crate::memory::bytes_until_nul(&saved_line_vec[col_u..]).len() as i32;

        saved_char = saved_line_vec[col_u];

        saved_line_vec[col_u] = 0;
    }

    u_clearline(curbuf_mut());
    g_st().ins_global.did_si = false;
    g_st().ins_global.ai_col = 0;

    if dir == FORWARD && g_st().ins_global.did_ai {
        trunc_line = true;
    }

    if (flags & OPENLINE_FORCE_INDENT) != 0 {
        newindent = second_line_indent;
    } else if curbuf_mut().b_p_ai.get() || do_si {
        let saved_line_bytes = crate::memory::bytes_until_nul(&saved_line_vec);
        newindent = indent_size_ts(
            saved_line_bytes,
            curbuf_mut().b_p_ts.get(),
            &curbuf_mut().b_p_vts_array,
        );
        if newindent == 0 && (flags & OPENLINE_COM_LIST) == 0 {
            newindent = second_line_indent;
        }

        if !trunc_line
            && do_si
            && saved_line_vec[0] != 0
            && (p_extra_idx.is_none() || first_char != b'{' as i32)
        {
            let old_cursor = curwin_mut().w_cursor;
            let mut ptr_bytes = saved_line_bytes;
            if (flags & OPENLINE_DO_COM) != 0 {
                lead_len = get_leader_len(ptr_bytes, None, false, true);
            } else {
                lead_len = 0;
            }
            if dir == FORWARD {
                if lead_len == 0 && ptr_bytes.first() == Some(&b'#') {
                    while ptr_bytes.first() == Some(&b'#') && curwin_mut().w_cursor.lnum > 1 {
                        curwin_mut().w_cursor.lnum -= 1;
                        ptr_bytes = crate::memline::ml_get_slice(curwin_mut().w_cursor.lnum);
                    }
                    newindent = get_indent();
                }
                if (flags & OPENLINE_DO_COM) != 0 {
                    lead_len = get_leader_len(ptr_bytes, None, false, true);
                } else {
                    lead_len = 0;
                }
                if lead_len > 0 {
                    let mut p_slice = crate::charset::skipwhite_slice(ptr_bytes);
                    if p_slice.starts_with(b"/*") {
                        p_slice = &p_slice[1..];
                    }
                    if let Some(after_star) = p_slice.strip_prefix(b"*") {
                        let mut idx = ptr_bytes.len() - after_star.len();
                        while idx < ptr_bytes.len() {
                            if ptr_bytes[idx] == b'/' && ptr_bytes[idx - 1] == b'*' {
                                curwin_mut().w_cursor.col = idx as Colnr;
                                if let Some(pos) = findmatch(None, 0) {
                                    curwin_mut().w_cursor.lnum = pos.lnum;

                                    newindent = get_indent();
                                    break;
                                }
                                ptr_bytes =
                                    crate::memline::ml_get_slice(curwin_mut().w_cursor.lnum);
                                idx = (curwin_mut().w_cursor.col as usize).min(ptr_bytes.len());
                            }
                            idx += 1;
                        }
                    }
                } else {
                    let mut p_idx = ptr_bytes.len().saturating_sub(1);
                    while p_idx > 0 && ascii_iswhite(ptr_bytes[p_idx]) {
                        p_idx -= 1;
                    }
                    let last_char = ptr_bytes.get(p_idx).copied().unwrap_or(0);

                    if last_char == b'{' || last_char == b';' {
                        p_idx = p_idx.saturating_sub(1);
                        while p_idx > 0 && ascii_iswhite(ptr_bytes[p_idx]) {
                            p_idx -= 1;
                        }
                    }
                    if ptr_bytes.get(p_idx) == Some(&b')') {
                        curwin_mut().w_cursor.col = p_idx as Colnr;
                        if let Some(pos) = findmatch(None, b'(' as i32) {
                            curwin_mut().w_cursor.lnum = pos.lnum;

                            newindent = get_indent();
                            ptr_bytes = crate::cursor::get_cursor_line_slice();
                        }
                    }
                    if last_char == b'{' {
                        g_st().ins_global.did_si = true;
                        no_si = true;
                    } else if last_char != b';' && last_char != b'}' && cin_is_cinword(ptr_bytes) {
                        g_st().ins_global.did_si = true;
                    }
                }
            } else {
                if lead_len == 0 && ptr_bytes.first() == Some(&b'#') {
                    let mut was_backslashed = false;

                    while (ptr_bytes.first() == Some(&b'#') || was_backslashed)
                        && curwin_mut().w_cursor.lnum < curbuf_mut().b_ml.ml_line_count.get()
                    {
                        was_backslashed = ptr_bytes.last() == Some(&b'\\');
                        curwin_mut().w_cursor.lnum += 1;
                        ptr_bytes = crate::memline::ml_get_slice(curwin_mut().w_cursor.lnum);
                    }
                    if was_backslashed {
                        newindent = 0;
                    } else {
                        newindent = get_indent();
                    }
                }
                if crate::charset::skipwhite_slice(ptr_bytes).first() == Some(&b'}') {
                    g_st().ins_global.did_si = true;
                } else {
                    g_st().ins_global.can_si_back = true;
                }
            }
            curwin_mut().w_cursor = old_cursor;
        }
        if do_si {
            g_st().ins_global.can_si = true;
        }

        g_st().ins_global.did_ai = true;
    }

    let do_cindent = !opt_vars().p_paste.get()
        && (curbuf_mut().b_p_cin.get() || curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST)
        && in_cinkeys(
            if dir == FORWARD {
                KEY_OPEN_FORW
            } else {
                KEY_OPEN_BACK
            },
            b' ' as i32,
            linewhite(curwin_mut().w_cursor.lnum),
        )
        && (flags & OPENLINE_FORCE_INDENT) == 0;

    g_st().ins_global.end_comment_pending = 0;
    if (flags & OPENLINE_DO_COM) != 0 {
        let saved_line_bytes = crate::memory::bytes_until_nul(&saved_line_vec);
        lead_len = get_leader_len(
            saved_line_bytes,
            Some(&mut lead_flags),
            dir == BACKWARD,
            true,
        );
        if lead_len == 0
            && curbuf_mut().b_p_cin.get()
            && do_cindent
            && dir == FORWARD
            && (!has_format_option(K_FO_NO_OPEN_COMS) || (flags & OPENLINE_FORMAT) != 0)
        {
            comment_start = check_linecomment(saved_line_bytes);
            if comment_start != MAXCOL {
                lead_len = get_leader_len(
                    &saved_line_bytes[(comment_start as usize).min(saved_line_bytes.len())..],
                    Some(&mut lead_flags),
                    false,
                    true,
                );
                if lead_len != 0 {
                    lead_len += comment_start;
                    if let Some(ref mut p) = did_do_comment {
                        **p = true;
                    }
                }
            }
        }
    } else {
        lead_len = 0;
    }

    if lead_len > 0 {
        let mut lead_repl_opt: Option<Vec<u8>> = None;
        let mut lead_repl_len: i32 = 0;
        let mut lead_middle: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
        let lead_middle_len: i32;
        let mut lead_end: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
        let mut comment_end_idx: Option<usize> = None;
        let mut extra_space = false;
        let mut require_blank = false;

        let lead_flags_bytes = lead_flags.map_or(&[][..], CStr::to_bytes);
        let mut p_off: usize = 0;
        while p_off < lead_flags_bytes.len() && lead_flags_bytes[p_off] != b':' {
            let p_byte = lead_flags_bytes[p_off];
            if p_byte == COM_BLANK {
                require_blank = true;
                p_off += 1;
                continue;
            }
            if p_byte == COM_START || p_byte == COM_MIDDLE {
                let current_flag = p_byte;
                let mut p_bytes = &lead_flags_bytes[p_off..];
                if p_byte == COM_START {
                    if dir == BACKWARD {
                        lead_len = 0;
                        break;
                    }
                    copy_option_part(&mut p_bytes, &mut lead_middle[..COM_MAX_LEN], c",");
                    require_blank = false;
                }

                if let Some(colon_pos) = p_bytes.iter().position(|&b| b == b':') {
                    if p_bytes[..colon_pos].contains(&COM_BLANK) {
                        require_blank = true;
                    }
                    p_bytes = &p_bytes[colon_pos + 1..];
                } else {
                    p_bytes = &[];
                }
                lead_middle_len =
                    copy_option_part(&mut p_bytes, &mut lead_middle[..COM_MAX_LEN], c",") as i32;

                if let Some(colon_pos) = p_bytes.iter().position(|&b| b == b':') {
                    if p_bytes[..colon_pos].contains(&COM_AUTO_END) {
                        g_st().ins_global.end_comment_pending = -1;
                    }
                    p_bytes = &p_bytes[colon_pos + 1..];
                } else {
                    p_bytes = &[];
                }
                let n = copy_option_part(&mut p_bytes, &mut lead_end[..COM_MAX_LEN], c",");

                if g_st().ins_global.end_comment_pending == -1 {
                    g_st().ins_global.end_comment_pending = i32::from(lead_end[n - 1]);
                }

                if dir == FORWARD {
                    let mut q_idx = lead_len as usize;
                    let end_bytes = &lead_end[..n];
                    let saved_bytes = crate::memory::bytes_until_nul(&saved_line_vec);
                    while q_idx < saved_bytes.len() {
                        let q_bytes = &saved_bytes[q_idx..];
                        if q_bytes.len() >= n && &q_bytes[..n] == end_bytes {
                            comment_end_idx = Some(q_idx);
                            lead_len = 0;
                            break;
                        }
                        q_idx += 1;
                    }
                }

                if lead_len > 0 {
                    if current_flag == COM_START {
                        lead_repl_len = lead_middle_len;
                        lead_repl_opt = Some(lead_middle[..lead_middle_len as usize].to_vec());
                    }

                    if !ascii_iswhite(saved_line_vec[(lead_len - 1) as usize])
                        && ((p_extra_idx.is_some() && curwin_mut().w_cursor.col == lead_len)
                            || (p_extra_idx.is_none() && saved_line_vec[lead_len as usize] == 0)
                            || require_blank)
                    {
                        extra_space = true;
                    }
                }
                break;
            }
            if p_byte == COM_END {
                if dir == FORWARD {
                    comment_end_idx = Some(crate::charset::skipwhite_len(&saved_line_vec));
                    lead_len = 0;
                    break;
                }

                let com_bytes = curbuf_mut().b_p_com.as_bytes();
                let base_addr = com_bytes.as_ptr() as usize;
                let mut cur_idx = ((lead_flags_bytes.as_ptr() as usize) - base_addr) + p_off;
                while cur_idx > 0 && com_bytes[cur_idx] != b',' {
                    cur_idx -= 1;
                }
                let mut repl_idx = cur_idx;
                while repl_idx > 0 && com_bytes[repl_idx - 1] != b':' {
                    repl_idx -= 1;
                }
                lead_repl_len = (cur_idx - repl_idx) as i32;
                lead_repl_opt = Some(com_bytes[repl_idx..cur_idx].to_vec());

                extra_space = true;

                let mut p2_idx = cur_idx;
                while p2_idx < com_bytes.len() && com_bytes[p2_idx] != b':' {
                    if com_bytes[p2_idx] == COM_AUTO_END {
                        g_st().ins_global.end_comment_pending = -1;
                    }
                    p2_idx += 1;
                }
                if g_st().ins_global.end_comment_pending == -1 {
                    while p2_idx < com_bytes.len() && com_bytes[p2_idx] != b',' {
                        p2_idx += 1;
                    }

                    if p2_idx > 0 {
                        g_st().ins_global.end_comment_pending = i32::from(com_bytes[p2_idx - 1]);
                    }
                }
                break;
            }
            if p_byte == COM_FIRST {
                if dir == BACKWARD {
                    lead_len = 0;
                } else {
                    lead_repl_opt = Some(Vec::new());
                    lead_repl_len = 0;
                }
                break;
            }
            p_off += 1;
        }

        if lead_len > 0 {
            let bytes = lead_len
                + lead_repl_len
                + i32::from(extra_space)
                + extra_len
                + second_line_indent.max(0)
                + 1;
            debug_assert!(bytes >= 0);
            allocated_vec = vec![0u8; bytes as usize];
            allocated_vec[..lead_len as usize]
                .copy_from_slice(&saved_line_vec[..lead_len as usize]);
            allocated_vec[lead_len as usize] = 0;
            for b in &mut allocated_vec[..comment_start as usize] {
                if !ascii_iswhite(*b) {
                    *b = b' ';
                }
            }
            leader_idx = 0;

            if let Some(ref lead_repl) = lead_repl_opt {
                let mut c: u8 = 0;
                let mut off: i32 = 0;

                let mut p_off: usize = 0;
                while p_off < lead_flags_bytes.len() && lead_flags_bytes[p_off] != b':' {
                    let pb = lead_flags_bytes[p_off];
                    if pb == COM_RIGHT || pb == COM_LEFT {
                        c = pb;
                        p_off += 1;
                    } else if ascii_isdigit(pb) || pb == b'-' {
                        let p_bytes = &lead_flags_bytes[p_off..];
                        let mut rem = p_bytes;
                        off = getdigits_int(&mut rem, true, 0);
                        p_off += p_bytes.len() - rem.len();
                    } else {
                        p_off += 1;
                    }
                }
                let repl_len_u = lead_repl_len.max(0) as usize;
                let repl_slice = &lead_repl[..repl_len_u.min(lead_repl.len())];
                if c == COM_RIGHT {
                    let mut p_idx = (lead_len - 1) as usize;
                    while p_idx > 0 && ascii_iswhite(allocated_vec[p_idx]) {
                        p_idx -= 1;
                    }
                    p_idx += 1;

                    {
                        let repl_size = vim_strnsize(repl_slice, lead_repl_len);
                        let mut old_size: i32 = 0;
                        let endp_idx = p_idx;

                        while old_size < repl_size && p_idx > 0 {
                            let prev_idx = p_idx - 1;
                            p_idx -= crate::mbyte::utf_head_off_slice(&allocated_vec, prev_idx)
                                as usize
                                + 1;
                            old_size += ptr2cells(&allocated_vec[p_idx..]);
                        }
                        let l = lead_repl_len - ((endp_idx as isize) - (p_idx as isize)) as i32;
                        if l != 0 {
                            let copy_len = (lead_len as usize).saturating_sub(endp_idx);
                            let dst_idx = ((endp_idx as isize) + (l as isize)) as usize;
                            allocated_vec.copy_within(endp_idx..endp_idx + copy_len, dst_idx);
                        }
                        lead_len += l;
                    }

                    allocated_vec[p_idx..p_idx + repl_slice.len()].copy_from_slice(repl_slice);

                    if p_idx + repl_len_u > lead_len as usize {
                        allocated_vec[p_idx + repl_len_u] = 0;
                    }

                    let mut scan_p = p_idx as isize;
                    while {
                        scan_p -= 1;
                        scan_p >= 0
                    } {
                        let cur_p = scan_p as usize;
                        let mut l = crate::mbyte::utf_head_off_slice(&allocated_vec, cur_p);
                        if l > 1 {
                            scan_p -= l as isize;
                            let base_p = scan_p as usize;
                            if ptr2cells(&allocated_vec[base_p..]) > 1 {
                                allocated_vec[base_p + 1] = b' ';
                                l -= 1;
                            }

                            let src_start = base_p + (l as usize) + 1;
                            let copy_len = (lead_len as usize).saturating_sub(src_start);
                            allocated_vec.copy_within(src_start..src_start + copy_len, base_p + 1);

                            lead_len -= l;

                            allocated_vec[base_p] = b' ';
                        } else if !ascii_iswhite(allocated_vec[cur_p]) {
                            allocated_vec[cur_p] = b' ';
                        }
                    }
                } else {
                    let mut p_idx = crate::charset::skipwhite_len(&allocated_vec);
                    {
                        let repl_size = vim_strnsize(repl_slice, lead_repl_len);
                        let mut i: i32 = 0;
                        while i < lead_len && allocated_vec[p_idx + (i as usize)] != 0 {
                            let l = utfc_ptr2len(&allocated_vec[p_idx + (i as usize)..]);
                            let span = (i + l).max(0) as usize;
                            if vim_strnsize(&allocated_vec[p_idx..p_idx + span], i + l) > repl_size
                            {
                                break;
                            }
                            i += l;
                        }
                        if i != lead_repl_len {
                            let src_start = p_idx + (i as usize);
                            let copy_len = (lead_len - i - (p_idx as i32)) as usize;
                            allocated_vec
                                .copy_within(src_start..src_start + copy_len, p_idx + repl_len_u);

                            lead_len += lead_repl_len - i;
                        }
                    }

                    allocated_vec[p_idx..p_idx + repl_slice.len()].copy_from_slice(repl_slice);

                    p_idx += repl_len_u;
                    while p_idx < lead_len as usize {
                        if !ascii_iswhite(allocated_vec[p_idx]) {
                            if p_idx + 1 < lead_len as usize && allocated_vec[p_idx + 1] == TAB {
                                lead_len -= 1;
                                let copy_len = (lead_len as usize).saturating_sub(p_idx);
                                allocated_vec.copy_within(p_idx + 1..p_idx + 1 + copy_len, p_idx);
                            } else {
                                let mut l = utfc_ptr2len(&allocated_vec[p_idx..]);
                                if l > 1 {
                                    if ptr2cells(&allocated_vec[p_idx..]) > 1 {
                                        l -= 1;
                                        allocated_vec[p_idx] = b' ';
                                        p_idx += 1;
                                    }

                                    let copy_len = (lead_len as usize).saturating_sub(p_idx);
                                    let src_start = p_idx + (l as usize);
                                    allocated_vec
                                        .copy_within(src_start..src_start + copy_len, p_idx + 1);

                                    lead_len -= l - 1;
                                }

                                allocated_vec[p_idx] = b' ';
                            }
                        }
                        p_idx += 1;
                    }

                    allocated_vec[p_idx] = 0;
                }

                if curbuf_mut().b_p_ai.get() || do_si {
                    newindent = indent_size_ts(
                        crate::memory::bytes_until_nul(&allocated_vec[leader_idx..]),
                        curbuf_mut().b_p_ts.get(),
                        &curbuf_mut().b_p_vts_array,
                    );
                }

                if newindent + off < 0 {
                    off = -newindent;
                    newindent = 0;
                } else {
                    newindent += off;
                }

                while off > 0 && lead_len > 0 && allocated_vec[(lead_len - 1) as usize] == b' ' {
                    let skip_n = crate::charset::skipwhite_len(&allocated_vec);
                    if crate::strings::vim_haschr(
                        crate::memory::bytes_as_cstr(&allocated_vec[skip_n..]),
                        b'\t' as i32,
                    ) {
                        break;
                    }
                    lead_len -= 1;
                    off -= 1;
                }

                if lead_len > 0 && ascii_iswhite(allocated_vec[(lead_len - 1) as usize]) {
                    extra_space = false;
                }

                allocated_vec[lead_len as usize] = 0;
            }

            if extra_space {
                allocated_vec[lead_len as usize] = b' ';

                lead_len += 1;

                allocated_vec[lead_len as usize] = 0;
            }

            newcol = lead_len;

            if newindent != 0 || g_st().ins_global.did_si {
                while lead_len != 0 && ascii_iswhite(allocated_vec[leader_idx]) {
                    lead_len -= 1;
                    newcol -= 1;
                    leader_idx += 1;
                }
            }
            g_st().ins_global.did_si = false;
            g_st().ins_global.can_si = false;
        } else if let Some(ce_idx) = comment_end_idx
            && saved_line_vec[ce_idx..].starts_with(b"*/")
            && (curbuf_mut().b_p_ai.get() || do_si)
        {
            let old_cursor = curwin_mut().w_cursor;
            curwin_mut().w_cursor.col = ce_idx as Colnr;
            if let Some(pos) = findmatch(None, 0) {
                curwin_mut().w_cursor.lnum = pos.lnum;

                newindent = get_indent();
            }
            curwin_mut().w_cursor = old_cursor;
        }
    }

    let mut p_extra_off: Option<usize> = None;
    if let Some(mut pe_idx) = p_extra_idx {
        saved_line_vec[pe_idx] = saved_char;

        if replace_normal(g_st().state_global.get()) {
            replace_push_nul();
        }
        if curbuf_mut().b_p_ai.get() || (flags & OPENLINE_DELSPACES) != 0 {
            while (saved_line_vec[pe_idx] == b' ' || saved_line_vec[pe_idx] == b'\t')
                && !utf_iscomposing_first(utf_ptr2char(&saved_line_vec[pe_idx + 1..]))
            {
                if replace_normal(g_st().state_global.get()) {
                    replace_push(&saved_line_vec[pe_idx..pe_idx + 1]);
                }
                pe_idx += 1;
                less_cols_off += 1;
            }
        }

        p_extra_off = Some(pe_idx);
        less_cols = pe_idx as Colnr;
    }

    let mut p_extra_in_allocated: Option<usize> = None;
    if lead_len > 0 {
        let lead_off = leader_idx;
        let mut cur_len = crate::memory::bytes_as_cstr(&allocated_vec[lead_off..])
            .to_bytes()
            .len();
        if (flags & OPENLINE_COM_LIST) != 0 && second_line_indent > 0 {
            let padding = second_line_indent - (newindent + cur_len as i32);
            for _ in 0..padding {
                allocated_vec[lead_off + cur_len] = b' ';
                cur_len += 1;
                less_cols -= 1;
                newcol += 1;
            }
        }
        let extra_bytes = match p_extra_off {
            Some(off) => crate::memory::bytes_as_cstr(&saved_line_vec[off..]).to_bytes_with_nul(),
            None => c"".to_bytes_with_nul(),
        };
        allocated_vec[lead_off + cur_len..lead_off + cur_len + extra_bytes.len()]
            .copy_from_slice(extra_bytes);
        p_extra_in_allocated = Some(lead_off);
        g_st().ins_global.did_ai = true;
        less_cols -= lead_len;
    } else {
        g_st().ins_global.end_comment_pending = 0;
    }

    g_st()
        .curbuf_splice_pending
        .set(g_st().curbuf_splice_pending.get() + (1));
    let old_cursor = curwin_mut().w_cursor;
    let old_cmod_flags = g_st().cmdmod.cmod_flags;
    let mut prompt_moved: Option<std::ffi::CString> = None;

    fn current_extra_bytes<'a>(
        p_extra_in_allocated: Option<usize>,
        p_extra_off: Option<usize>,
        allocated_vec: &'a [u8],
        saved_line_vec: &'a [u8],
    ) -> &'a [u8] {
        if let Some(lead_off) = p_extra_in_allocated {
            crate::memory::bytes_until_nul(&allocated_vec[lead_off..])
        } else if let Some(off) = p_extra_off {
            crate::memory::bytes_until_nul(&saved_line_vec[off..])
        } else {
            &[]
        }
    }

    if dir == BACKWARD {
        if bt_prompt(Some(crate::globals::curbuf_mut()))
            && curwin_mut().w_cursor.lnum == curbuf_mut().b_prompt_start.mark.lnum
        {
            let prompt_line_cstr = ml_get(curwin_mut().w_cursor.lnum);
            let prompt_line_bytes = prompt_line_cstr.to_bytes();
            let prompt = prompt_text();
            let prompt_len = prompt.to_bytes().len();

            if prompt_line_bytes.starts_with(prompt.to_bytes()) {
                let rem_bytes = prompt_line_bytes[prompt_len..].to_vec();

                g_st().cmdmod.cmod_flags |= CMOD_LOCKMARKS;
                ml_replace(curwin_mut().w_cursor.lnum, &rem_bytes, true);
                let cur_extra = current_extra_bytes(
                    p_extra_in_allocated,
                    p_extra_off,
                    &allocated_vec,
                    &saved_line_vec,
                );
                prompt_moved = Some(concat_str(prompt.to_bytes(), cur_extra));
            }
        }
        curwin_mut().w_cursor.lnum -= 1;
    }

    let p_extra_bytes = if let Some(ref pm) = prompt_moved {
        pm.as_bytes()
    } else {
        current_extra_bytes(
            p_extra_in_allocated,
            p_extra_off,
            &allocated_vec,
            &saved_line_vec,
        )
    };
    let mut did_append: bool;
    let append_failed = if (g_st().state_global.get() & VREPLACE_FLAG) == 0
        || old_cursor.lnum >= g_st().orig_line_count.get()
    {
        if !ml_append(curwin_mut().w_cursor.lnum, p_extra_bytes, 0, false) {
            did_append = false;
            true
        } else {
            mark_adjust(
                curwin_mut().w_cursor.lnum + 1,
                MAXLNUM,
                1,
                0,
                K_EXTMARK_NOOP,
            );
            did_append = true;
            false
        }
    } else {
        curwin_mut().w_cursor.lnum += 1;
        if curwin_mut().w_cursor.lnum
            >= g_st().ins_global.start.lnum + g_st().vr_lines_changed.get()
        {
            u_save_cursor();
            g_st()
                .vr_lines_changed
                .set(g_st().vr_lines_changed.get() + (1));
        }
        ml_replace(curwin_mut().w_cursor.lnum, p_extra_bytes, true);
        changed_bytes(curwin_mut().w_cursor.lnum, 0);
        curwin_mut().w_cursor.lnum -= 1;
        did_append = false;
        false
    };

    if !append_failed {
        g_st()
            .inhibit_delete_count
            .set(g_st().inhibit_delete_count.get() + (1));
        if newindent != 0 || g_st().ins_global.did_si {
            curwin_mut().w_cursor.lnum += 1;
            if g_st().ins_global.did_si {
                let sw = get_sw_value(curbuf_mut());
                if opt_vars().p_sr.get() {
                    newindent -= newindent % sw;
                }
                newindent += sw;
            }
            if curbuf_mut().b_p_ci.get() {
                copy_indent(newindent, crate::memory::bytes_until_nul(&saved_line_vec));
                curbuf_mut().b_p_pi.set(true);
            } else {
                set_indent(newindent, SIN_INSERT | SIN_NOMARK);
            }
            less_cols -= curwin_mut().w_cursor.col;

            g_st().ins_global.ai_col = curwin_mut().w_cursor.col;

            if replace_normal(g_st().state_global.get()) {
                for _ in 0..curwin_mut().w_cursor.col {
                    replace_push_nul();
                }
            }
            newcol += curwin_mut().w_cursor.col;
            if no_si {
                g_st().ins_global.did_si = false;
            }
        }
        g_st()
            .inhibit_delete_count
            .set(g_st().inhibit_delete_count.get() - (1));

        if replace_normal(g_st().state_global.get()) {
            while lead_len > 0 {
                replace_push_nul();
                lead_len -= 1;
            }
        }

        curwin_mut().w_cursor = old_cursor;

        if dir == FORWARD {
            if trunc_line || (g_st().state_global.get() & MODE_INSERT) != 0 {
                saved_line_vec[curwin_mut().w_cursor.col as usize] = 0;

                if trunc_line && (flags & OPENLINE_KEEPTRAIL) == 0 {
                    let col_idx = curwin_mut().w_cursor.col as usize;
                    truncate_spaces(&mut saved_line_vec, col_idx);
                }
                let saved_line_bytes = crate::memory::bytes_until_nul(&saved_line_vec);
                ml_replace(curwin_mut().w_cursor.lnum, saved_line_bytes, true);

                let new_len = saved_line_bytes.len() as i32;

                let mut cols_spliced: i32 = 0;
                if new_len < curwin_mut().w_cursor.col {
                    extmark_splice_cols(
                        curbuf_mut(),
                        curwin_mut().w_cursor.lnum - 1,
                        new_len,
                        curwin_mut().w_cursor.col - new_len,
                        0,
                        K_EXTMARK_UNDO,
                    );
                    cols_spliced = curwin_mut().w_cursor.col - new_len;
                }

                if did_append {
                    let cols_added = mincol - 1 + less_cols_off - less_cols;
                    extmark_splice(
                        curbuf_mut(),
                        lnum - 1,
                        mincol - 1 - cols_spliced,
                        0,
                        less_cols_off,
                        less_cols_off as Bcount,
                        1,
                        cols_added,
                        (1 + cols_added) as Bcount,
                        K_EXTMARK_UNDO,
                    );

                    changed_lines(
                        curbuf_mut(),
                        curwin_mut().w_cursor.lnum,
                        curwin_mut().w_cursor.col,
                        curwin_mut().w_cursor.lnum + 1,
                        1,
                        true,
                    );
                    did_append = false;

                    if (flags & OPENLINE_MARKFIX) != 0 {
                        mark_col_adjust(
                            curwin_mut().w_cursor.lnum,
                            curwin_mut().w_cursor.col + less_cols_off,
                            1,
                            -less_cols,
                            0,
                        );
                    }
                } else {
                    changed_bytes(curwin_mut().w_cursor.lnum, curwin_mut().w_cursor.col);
                }
            }

            curwin_mut().w_cursor.lnum = old_cursor.lnum + 1;
        }
        if did_append {
            let extra = ml_get_len(curwin_mut().w_cursor.lnum) as Bcount;
            extmark_splice(
                curbuf_mut(),
                curwin_mut().w_cursor.lnum - 1,
                0,
                0,
                0,
                0,
                1,
                0,
                1 + extra,
                K_EXTMARK_UNDO,
            );
            changed_lines(
                curbuf_mut(),
                curwin_mut().w_cursor.lnum,
                0,
                curwin_mut().w_cursor.lnum,
                1,
                true,
            );
        }
        g_st()
            .curbuf_splice_pending
            .set(g_st().curbuf_splice_pending.get() - (1));

        curwin_mut().w_cursor.col = newcol;
        curwin_mut().w_cursor.coladd = 0;

        let vreplace_mode = if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
            let m = g_st().state_global.get();
            g_st().state_global.set(MODE_INSERT);
            m
        } else {
            0
        };

        if !opt_vars().p_paste.get() {
            if allocated_vec.is_empty()
                && !use_indentexpr_for_lisp()
                && curbuf_mut().b_p_lisp.get()
                && curbuf_mut().b_p_ai.get()
            {
                fixthisline(get_lisp_indent);
                g_st().ins_global.ai_col = getwhitecols_curline() as Colnr;
            } else if do_cindent || (curbuf_mut().b_p_ai.get() && use_indentexpr_for_lisp()) {
                do_c_expr_indent();
                g_st().ins_global.ai_col = getwhitecols_curline() as Colnr;
            }
        }

        if vreplace_mode != 0 {
            g_st().state_global.set(vreplace_mode);
        }

        if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
            let extra_vec = crate::cursor::get_cursor_line_slice().to_vec();
            ml_replace(
                curwin_mut().w_cursor.lnum,
                crate::memory::bytes_until_nul(&next_line_vec),
                true,
            );
            curwin_mut().w_cursor.col = 0;
            curwin_mut().w_cursor.coladd = 0;
            ins_bytes(&extra_vec);
        }

        retval = true;
    }

    curbuf_mut().b_p_pi.set(saved_pi);
    drop(saved_line_vec);
    drop(next_line_vec);
    drop(allocated_vec);
    drop(prompt_moved);
    g_st().cmdmod.cmod_flags = old_cmod_flags;
    retval
}

/// Delete from cursor to end of line.
pub fn truncate_line(fixpos: i32) {
    let lnum = curwin_mut().w_cursor.lnum;
    let col = curwin_mut().w_cursor.col;
    let old_line = crate::memline::ml_get_slice(lnum);
    let newp_vec: Vec<u8> = if col == 0 {
        vec![0]
    } else {
        let col_u = (col as usize).min(old_line.len());
        let mut v = Vec::with_capacity(col_u + 1);
        v.extend_from_slice(&old_line[..col_u]);
        v.push(0);
        v
    };
    let deleted = ml_get_len(lnum) - col;

    ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
    drop(newp_vec);
    inserted_bytes(lnum, curwin_mut().w_cursor.col, deleted, 0);

    if fixpos != 0 && curwin_mut().w_cursor.col > 0 {
        curwin_mut().w_cursor.col -= 1;
    }
}

/// Delete `nlines` lines at the cursor.
pub fn del_lines(nlines: Linenr, undo: bool) {
    let first = curwin_mut().w_cursor.lnum;

    if nlines <= 0 {
        return;
    }

    if undo && !u_savedel(first, nlines) {
        return;
    }

    let mut n: i32 = 0;
    while n < nlines {
        if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
            break;
        }

        ml_delete_flags(first, ML_DEL_MESSAGE);
        n += 1;

        if first > curbuf_mut().b_ml.ml_line_count.get() {
            break;
        }
    }

    curwin_mut().w_cursor.col = 0;
    check_cursor_lnum(curwin_mut());

    deleted_lines_mark(first, n);
}

/// Returns the length in bytes of the prefix of the given string which introduces a comment.
pub fn get_leader_len(
    line_bytes: &[u8],
    mut flags: Option<&mut Option<&'static CStr>>,
    backward: bool,
    include_space: bool,
) -> i32 {
    let mut got_com = false;
    let mut part_buf: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
    let mut middle_match_len: usize = 0;
    let mut saved_flags: Option<&'static CStr> = None;

    let mut result: usize = 0;
    let mut i: usize = 0;
    while i < line_bytes.len() && ascii_iswhite(line_bytes[i]) {
        i += 1;
    }

    while i < line_bytes.len() {
        let mut found_one = false;
        let mut part_flags_range: std::ops::Range<usize> = 0..0;
        let mut list = curbuf_mut().b_p_com.as_cstr();
        while !list.to_bytes().is_empty() {
            if !got_com && let Some(flags_out) = flags.as_deref_mut() {
                *flags_out = Some(list);
            }
            let prev_list = list;
            let mut list_bytes = list.to_bytes();
            let old_len = list_bytes.len();
            let part_len = copy_option_part(&mut list_bytes, &mut part_buf[..COM_MAX_LEN], c",");
            list = &list[old_len - list_bytes.len()..];
            let part = &part_buf[..part_len];
            let Some(colon_idx) = part.iter().position(|&b| b == b':') else {
                part_flags_range = 0..part_len;
                continue;
            };
            part_flags_range = 0..colon_idx;
            let part_flags = &part[..colon_idx];
            let mut string = &part[colon_idx + 1..];

            if middle_match_len != 0
                && !part_flags.contains(&COM_MIDDLE)
                && !part_flags.contains(&COM_END)
            {
                break;
            }

            if got_com && !part_flags.contains(&COM_NEST) {
                continue;
            }

            if backward && part_flags.contains(&COM_NOBACK) {
                continue;
            }

            if string.first().is_some_and(|&b| ascii_iswhite(b)) {
                if i == 0 || !ascii_iswhite(line_bytes[i - 1]) {
                    continue;
                }
                while let Some((&b, rest)) = string.split_first() {
                    if !ascii_iswhite(b) {
                        break;
                    }
                    string = rest;
                }
            }
            if !line_bytes[i..].starts_with(string) {
                continue;
            }
            let j = string.len();

            if part_flags.contains(&COM_BLANK)
                && line_bytes.get(i + j).is_some_and(|&b| !ascii_iswhite(b))
            {
                continue;
            }

            if part_flags.contains(&COM_MIDDLE) {
                if middle_match_len == 0 {
                    middle_match_len = j;
                    saved_flags = Some(prev_list);
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
            if !got_com && let Some(flags_out) = flags.as_deref_mut() {
                *flags_out = saved_flags;
            }
            i += middle_match_len;
            found_one = true;
        }

        if !found_one {
            break;
        }

        result = i;

        while i < line_bytes.len() && ascii_iswhite(line_bytes[i]) {
            i += 1;
        }

        if include_space {
            result = i;
        }

        got_com = true;
        if !part_buf[part_flags_range].contains(&COM_NEST) {
            break;
        }
    }
    result as i32
}

/// Return the offset at which the last comment in `line` starts, or -1 if none.
pub fn get_last_leader_offset(
    line_bytes: &[u8],
    mut flags: Option<&mut Option<&'static CStr>>,
) -> i32 {
    let mut result: i32 = -1;
    let mut lower_check_bound: i32 = 0;
    let mut com_leader_range: std::ops::Range<usize> = 0..0;
    let mut com_flags: Option<&'static CStr> = None;
    let mut part_buf: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];

    let mut i = line_bytes.len() as i32;
    while {
        i -= 1;
        i >= lower_check_bound
    } {
        let i_usize = i as usize;
        let mut found_one = false;
        let mut part_flags_range: std::ops::Range<usize> = 0..0;
        let mut list = curbuf_mut().b_p_com.as_cstr();
        while !list.to_bytes().is_empty() {
            let flags_save = list;
            let mut list_bytes = list.to_bytes();
            let old_len = list_bytes.len();
            let part_len = copy_option_part(&mut list_bytes, &mut part_buf[..COM_MAX_LEN], c",");
            list = &list[old_len - list_bytes.len()..];
            let part = &part_buf[..part_len];
            let Some(colon_idx) = part.iter().position(|&b| b == b':') else {
                continue;
            };
            part_flags_range = 0..colon_idx;
            com_leader_range = (colon_idx + 1)..part_len;
            let part_flags = &part[..colon_idx];
            let mut string = &part[colon_idx + 1..];

            if string.first().is_some_and(|&b| ascii_iswhite(b)) {
                if i == 0 || !ascii_iswhite(line_bytes[i_usize - 1]) {
                    continue;
                }
                while let Some((&b, rest)) = string.split_first() {
                    if !ascii_iswhite(b) {
                        break;
                    }
                    string = rest;
                }
            }
            if !line_bytes[i_usize..].starts_with(string) {
                continue;
            }
            let j = string.len();

            if part_flags.contains(&COM_BLANK)
                && line_bytes
                    .get(i_usize + j)
                    .is_some_and(|&b| !ascii_iswhite(b))
            {
                continue;
            }

            if part_flags.contains(&COM_MIDDLE)
                && line_bytes[..i_usize].iter().any(|&b| !ascii_iswhite(b))
            {
                continue;
            }

            found_one = true;
            if let Some(flags_out) = flags.as_deref_mut() {
                *flags_out = Some(flags_save);
            }
            com_flags = Some(flags_save);
            break;
        }

        if found_one {
            let mut part_buf2: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];

            result = i;
            if part_buf[part_flags_range].contains(&COM_NEST) {
                continue;
            }

            lower_check_bound = i;

            let mut com_leader = &part_buf[com_leader_range.clone()];
            while let Some((&b, rest)) = com_leader.split_first() {
                if !ascii_iswhite(b) {
                    break;
                }
                com_leader = rest;
            }
            let len1 = com_leader.len() as i32;

            let mut list = curbuf_mut().b_p_com.as_cstr();
            while !list.to_bytes().is_empty() {
                let flags_save = list;
                let mut list_bytes = list.to_bytes();
                let old_len = list_bytes.len();
                let part2_len =
                    copy_option_part(&mut list_bytes, &mut part_buf2[..COM_MAX_LEN], c",");
                list = &list[old_len - list_bytes.len()..];
                if com_flags.is_some_and(|cf| ptr::eq(flags_save.as_ptr(), cf.as_ptr())) {
                    continue;
                }
                let part2 = &part_buf2[..part2_len];
                let Some(colon_idx) = part2.iter().position(|&b| b == b':') else {
                    continue;
                };
                let mut string = &part2[colon_idx + 1..];
                while let Some((&b, rest)) = string.split_first() {
                    if !ascii_iswhite(b) {
                        break;
                    }
                    string = rest;
                }
                let len2 = string.len() as i32;
                if len2 == 0 {
                    continue;
                }

                let mut off = len2.min(i);
                while off > 0 && off + len1 > len2 {
                    off -= 1;
                    let off_usize = off as usize;
                    let cmp_len = (len2 - off) as usize;
                    if string[off_usize..off_usize + cmp_len] == com_leader[..cmp_len] {
                        lower_check_bound = lower_check_bound.min(i - off);
                    }
                }
            }
        }
    }
    result
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
