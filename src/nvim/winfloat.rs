//! Floating window creation, configuration, and layout (`src/nvim/winfloat.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    api_clear_error, api_set_error, cstr_as_obj, error_init, error_set, find_window_by_handle,
    has_key,
};
use crate::api::private::validate::{Error, ErrorType};
use crate::api::vim::nvim_create_buf;
use crate::api::win_config::{
    K_ALIGN_CENTER, K_BORDER_TEXT_TITLE, K_FLOAT_ANCHOR_EAST, K_FLOAT_ANCHOR_SOUTH,
    K_FLOAT_RELATIVE_CURSOR, K_FLOAT_RELATIVE_EDITOR, K_FLOAT_RELATIVE_LASTSTATUS,
    K_FLOAT_RELATIVE_MOUSE, K_FLOAT_RELATIVE_WINDOW, K_UIMULTIGRID, K_WIN_STYLE_MINIMAL,
    WIN_CONFIG_INIT, parse_bordertext, parse_winborder,
};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::cursor::{CursorPos, Linenr};
use crate::drawscreen::{UPD_NOT_VALID, UPD_VALID, redraw_later, set_must_redraw};
use crate::errors::*;
use crate::fileio::shorten_buf_fname;
use crate::globals::*;
use crate::message::{K_ZINDEX_MESSAGES, emsg};
use crate::mouse::mouse_find_win_inner;
use crate::r#move::{textpos2screenpos, validate_cursor};
use crate::option::{
    K_OPT_BUFHIDDEN, OPT_LOCAL, OptTarget, parse_winhl_opt, set_option_direct_for,
};
use crate::option_vars::*;
use crate::optionstr::{K_OPT_PREVIEWPOPUP, OptKeyDict_pvp, opt_keyset};
use crate::os::fs::{MAXPATHL, os_dirname};
use crate::plines::{plines_m_win, win_max_displaywidth};
use crate::strings::{concat_str, gettext};
use crate::ui::ui_has;
use crate::window::{
    STATUS_HEIGHT, TabPageInfo, WinConfig, WinInfoInfo, WinKind, last_status, lastwin_nofloating,
    merge_win_config, tabpage_win_valid, win_alloc, win_append, win_close, win_comp_pos, win_enter,
    win_find_tabpage, win_free, win_init, win_is_focusable, win_remove, win_remove_status_line,
    win_set_buf, win_set_inner_size, win_valid, winframe_remove,
};
use std::ptr;

const KEYSET_OPTIDX_PVP_WIDTH: u64 = 1;
const KEYSET_OPTIDX_PVP_BORDER: u64 = 2;
const KEYSET_OPTIDX_PVP_HEIGHT: u64 = 3;

use crate::ex_cmds::K_WIN_PREVIEW;

/// Creates a new float, or transforms an existing window to a float (`win_new_float`).
pub fn win_new_float(
    wp_opt: Option<&mut WinInfoInfo>,
    last: bool,
    fconfig: WinConfig,
    err: &mut Error,
) -> Option<&'static mut WinInfoInfo> {
    let wp = if let Some(wp) = wp_opt {
        let wp = crate::globals::WinPtr::from(&*wp).borrow_mut();
        debug_assert!(!last);
        debug_assert!(!wp.w_floating.get());

        let win_tp = crate::globals::TabpagePtr::from(win_find_tabpage(wp));
        debug_assert!(!win_tp.is_null());
        if (win_tp == g_st().curtab.get()
            && g_st().firstwin.get() == wp
            && ptr::eq(lastwin_nofloating(None), wp))
            || win_tp != g_st().curtab.get()
                && win_tp.tp_firstwin.get() == wp
                && ptr::eq(lastwin_nofloating(win_tp.opt()), wp)
        {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Cannot change last window into float",
            );
            return None;
        }
        let tp = if win_tp == g_st().curtab.get() {
            crate::globals::TabpagePtr::NULL
        } else {
            win_tp.clone()
        };
        let mut dir: i32 = 0;

        winframe_remove(wp, &mut dir, tp.opt(), None);

        if !wp.w_frame.get().is_null() {
            wp.w_frame.get().clear_links();
        }

        wp.w_frame.set(crate::globals::FramePtr::NULL);

        win_remove(wp, tp.opt());

        if win_tp == g_st().curtab.get() {
            last_status(false);
            win_comp_pos();
        }
        let prev = lastwin_nofloating(tp.opt());
        win_append(Some(prev), wp, tp.opt());
        wp
    } else {
        let mut tp_last = if last {
            g_st().lastwin.get()
        } else {
            crate::globals::WinPtr::from(lastwin_nofloating(None))
        };
        if fconfig.window != 0 {
            debug_assert!(!last);
            let parent_wp = find_window_by_handle(fconfig.window, err);
            if parent_wp.is_null() {
                return None;
            }
            let tp = win_find_tabpage(parent_wp.borrow())?;
            tp_last =
                crate::globals::WinPtr::from(lastwin_nofloating(if tp == g_st().curtab.get() {
                    None
                } else {
                    tp.opt()
                }));
        }
        let wp = win_alloc(tp_last.opt_mut(), false);
        win_init(wp, curwin_mut(), 0);
        if !wp.w_onebuf_opt.wo_wbr.is_null() && fconfig.height == 1 {
            wp.w_onebuf_opt.wo_wbr.clear_option_str();
        }
        if !wp.w_onebuf_opt.wo_stl.is_null() {
            wp.w_onebuf_opt.wo_stl.clear_option_str();
        }
        wp
    };
    wp.w_floating.set(true);
    wp.w_status_height.set(
        if !wp.w_onebuf_opt.wo_stl.is_empty()
            && (opt_vars().p_ls.get() == 1 || opt_vars().p_ls.get() == 2)
        {
            STATUS_HEIGHT
        } else {
            0
        },
    );
    wp.w_winbar_height.set(0);
    wp.w_hsep_height.set(0);
    wp.w_vsep_width.set(0);

    win_config_float(&mut *wp, fconfig);
    redraw_later(Some(&mut *wp), UPD_VALID);
    Some(wp)
}

pub fn win_set_minimal_style(wp: &mut WinInfoInfo) {
    wp.w_onebuf_opt.wo_nu.set(false);
    wp.w_onebuf_opt.wo_rnu.set(false);
    wp.w_onebuf_opt.wo_cul.set(false);
    wp.w_onebuf_opt.wo_cuc.set(false);
    wp.w_onebuf_opt.wo_spell.set(false);
    wp.w_onebuf_opt.wo_list.set(false);

    // Hide EOB region: use " " fillchar and cleared highlighting
    if wp.w_p_fcs_chars.eob != b' ' as u32 {
        let old = wp.w_onebuf_opt.wo_fcs.as_bytes();
        if old.is_empty() {
            wp.w_onebuf_opt.wo_fcs.set_option_cstr(c"eob: ");
        } else {
            let joined = concat_str(old, b",eob: ");
            wp.w_onebuf_opt.wo_fcs.set_option_cstr(joined.as_c_str());
        }
    }

    let old = wp.w_onebuf_opt.wo_winhl.as_bytes();
    if old.is_empty() {
        wp.w_onebuf_opt.wo_winhl.set_option_cstr(c"EndOfBuffer:");
    } else {
        let joined = concat_str(old, b",EndOfBuffer:");
        wp.w_onebuf_opt.wo_winhl.set_option_cstr(joined.as_c_str());
    }
    parse_winhl_opt(None, Some(&mut *wp));

    // signcolumn: use 'auto'
    let scl_len = wp.w_onebuf_opt.wo_scl.as_bytes().len();
    if wp.w_onebuf_opt.wo_scl.first_byte() != b'a' || scl_len >= 8 {
        wp.w_onebuf_opt.wo_scl.set_option_cstr(c"auto");
    }

    // foldcolumn: use '0'
    if wp.w_onebuf_opt.wo_fdc.first_byte() != b'0' {
        wp.w_onebuf_opt.wo_fdc.set_option_cstr(c"0");
    }

    // colorcolumn: cleared
    if !wp.w_onebuf_opt.wo_cc.is_empty() {
        wp.w_onebuf_opt.wo_cc.set_option_cstr(c"");
    }

    // statuscolumn: cleared
    if !wp.w_onebuf_opt.wo_stc.is_empty() {
        wp.w_onebuf_opt.wo_stc.clear_option_str();
    }

    // statusline: cleared (for floating windows)
    if wp.w_floating.get() && !wp.w_onebuf_opt.wo_stl.is_empty() {
        wp.w_onebuf_opt.wo_stl.clear_option_str();
        if wp.w_status_height.get() > 0 {
            let cfg = wp.w_config.clone();
            win_config_float(&mut *wp, cfg);
        }
    }
}

pub fn win_border_height(wp: &mut WinInfoInfo) -> i32 {
    wp.w_border_adj[0].get() + wp.w_border_adj[2].get()
}

pub fn win_border_width(wp: &mut WinInfoInfo) -> i32 {
    wp.w_border_adj[1].get() + wp.w_border_adj[3].get()
}

pub fn win_config_float(wp: &mut WinInfoInfo, mut fconfig: WinConfig) {
    // Process statusline changes before applying new height from config
    let show_stl = !wp.w_onebuf_opt.wo_stl.is_empty()
        && (opt_vars().p_ls.get() == 1 || opt_vars().p_ls.get() == 2);
    if wp.w_status_height.get() != 0 && !show_stl {
        win_remove_status_line(wp, false);
    } else if wp.w_status_height.get() == 0 && show_stl {
        wp.w_status_height.set(STATUS_HEIGHT);
    }

    wp.w_width.set(fconfig.width.max(1));
    wp.w_height.set(fconfig.height.max(1));

    if fconfig.relative == K_FLOAT_RELATIVE_CURSOR {
        validate_cursor(curwin_mut());
        fconfig.relative = K_FLOAT_RELATIVE_WINDOW;
        fconfig.row += curwin_mut().w_wrow.get() as f64;
        fconfig.col += curwin_mut().w_wcol.get() as f64;
        fconfig.window = curwin_mut().handle;
    } else if fconfig.relative == K_FLOAT_RELATIVE_MOUSE {
        let mut row = g_st().mouse_row.get();
        let mut col = g_st().mouse_col.get();
        let mut grid = g_st().mouse_grid.get();
        if let Some(mouse_win) = mouse_find_win_inner(&mut grid, &mut row, &mut col) {
            fconfig.relative = K_FLOAT_RELATIVE_WINDOW;
            fconfig.row += row as f64;
            fconfig.col += col as f64;
            fconfig.window = mouse_win.handle;
        }
    }

    let change_external = fconfig.external != wp.w_config.external;
    let mut change_border =
        fconfig.border != wp.w_config.border || fconfig.border_hl_ids != wp.w_config.border_hl_ids;

    merge_win_config(&mut wp.w_config, fconfig);

    let has_border = wp.w_floating.get() && wp.w_config.border;
    for i in 0..4 {
        let new_adj = i32::from(has_border && wp.w_config.border_chars[2 * i + 1][0] != 0);
        if new_adj != wp.w_border_adj[i].get() {
            change_border = true;
            wp.w_border_adj[i].set(new_adj);
        }
    }

    if !ui_has(K_UIMULTIGRID) {
        let above_ch = if wp.w_config.zindex < K_ZINDEX_MESSAGES {
            opt_vars().p_ch.get() as i32
        } else {
            0
        };
        let max_h = g_st().rows_global.get() - win_border_height(wp) - above_ch;
        wp.w_height.set(wp.w_height.get().min(max_h));
        let max_w = g_st().columns_global.get() - win_border_width(wp);
        wp.w_width.set(wp.w_width.get().min(max_w));
    }

    win_set_inner_size(wp, true);
    set_must_redraw(UPD_VALID);
    wp.w_redr_status.set(wp.w_status_height.get() != 0);
    wp.w_pos_changed.set(true);
    if change_external || change_border {
        wp.w_hl_needs_update.set(1);
        redraw_later(Some(&mut *wp), UPD_NOT_VALID);
    }

    // compute initial position
    if wp.w_config.relative == K_FLOAT_RELATIVE_WINDOW {
        let mut row = wp.w_config.row as i32;
        let mut col = wp.w_config.col as i32;
        let mut dummy = error_init();
        let parent = find_window_by_handle(wp.w_config.window, &mut dummy);
        if let Some(parent) = parent.opt_mut() {
            row += parent.w_winrow.get();
            col += parent.w_wincol.get();
            let _ = crate::grid::grid_adjust_opt(&mut parent.w_grid, &mut row, &mut col);
            if wp.w_config.bufpos.lnum >= 0 {
                let mut pos = CursorPos {
                    lnum: (wp.w_config.bufpos.lnum + 1)
                        .min((*parent).buf_mut().b_ml.ml_line_count.get()),
                    col: wp.w_config.bufpos.col,
                    coladd: 0,
                };
                let mut trow: i32 = 0;
                let mut tcol: i32 = 0;
                let mut tcolc: i32 = 0;
                let mut tcole: i32 = 0;
                textpos2screenpos(
                    &mut *parent,
                    &mut pos,
                    &mut trow,
                    &mut tcol,
                    &mut tcolc,
                    &mut tcole,
                    true,
                );
                row += trow - 1;
                col += tcol - 1;
            }
        }
        api_clear_error(&mut dummy);
        wp.w_winrow.set(row);
        wp.w_wincol.set(col);
    } else {
        wp.w_winrow.set(wp.w_config.row as i32);
        wp.w_wincol.set(wp.w_config.col as i32);
    }

    // changing border style while keeping border only requires redrawing border
    if wp.w_config.border {
        wp.w_redr_border.set(true);
        redraw_later(Some(&mut *wp), UPD_VALID);
    }
}

pub fn win_float_remove(bang: bool, mut count: i32) {
    let mut float_win_arr: Vec<crate::globals::WinPtr> = Vec::new();
    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating.get()) {
        if !wp.w_config.hide && !wp.w_onebuf_opt.wo_wp.get() {
            float_win_arr.push(crate::globals::WinPtr::from(&*wp));
        }
    }
    if !float_win_arr.is_empty() {
        float_win_arr.sort_unstable_by_key(|&w| std::cmp::Reverse(w.w_config.zindex));
    }
    for mut w in float_win_arr {
        if win_valid(w.opt()) && !win_close(&mut w, false, false) {
            break;
        }
        if !bang {
            count -= 1;
            if count == 0 {
                break;
            }
        }
    }
}

pub fn win_check_anchored_floats(win: &mut WinInfoInfo) {
    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating.get()) {
        if wp.w_config.relative == K_FLOAT_RELATIVE_WINDOW && wp.w_config.window == win.handle {
            wp.w_pos_changed.set(true);
        }
    }
}

pub fn win_float_update_statusline() {
    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating.get()) {
        let has_status = wp.w_status_height.get() > 0;
        let should_show = !wp.w_onebuf_opt.wo_stl.is_empty()
            && (opt_vars().p_ls.get() == 1 || opt_vars().p_ls.get() == 2);
        if should_show != has_status {
            let cfg = wp.w_config.clone();
            win_config_float(wp, cfg);
        }
    }
}

pub fn win_float_anchor_laststatus() {
    for win in for_all_windows_mut() {
        if win.w_config.relative == K_FLOAT_RELATIVE_LASTSTATUS {
            win.w_pos_changed.set(true);
        }
    }
}

pub fn win_reconfig_floats() {
    for wp in for_all_windows_rev_ptr().take_while(|wp| wp.w_floating.get()) {
        let cfg = wp.w_config.clone();
        win_config_float(wp, cfg);
    }
}

/// Find the floating window of the given kind in the current tabpage (`win_float_find`).
pub fn win_float_find(kind: WinKind) -> Option<&'static mut WinInfoInfo> {
    for_all_windows_rev_ptr()
        .take_while(|wp| wp.w_floating.get())
        .find(|wp| wp.w_kind.get() == kind)
        .map(|v| &mut *v)
}

/// Select an alternative window to `win` (assumed floating) in tabpage `tp` (`win_float_find_altwin`).
pub fn win_float_find_altwin(
    win: &WinInfoInfo,
    tp: Option<&TabPageInfo>,
) -> &'static mut WinInfoInfo {
    let Some(tp) = tp else {
        return if win_valid(g_st().prevwin.get().opt())
            && let Some(wp) = prevwin_opt()
            && !ptr::eq(wp, win)
            && win_is_focusable(wp)
        {
            wp
        } else {
            firstwin_mut()
        };
    };

    debug_assert!(g_st().curtab.get() != tp);
    let wp = if tabpage_win_valid(Some(tp), tp.tp_prevwin.get().opt()) {
        tp.tp_prevwin.get()
    } else {
        tp.tp_firstwin.get()
    };
    if let Some(wp_ref) = wp.opt_mut()
        && wp != win
        && win_is_focusable(wp_ref)
    {
        wp_ref
    } else {
        tp.tp_firstwin.get().borrow_mut()
    }
}

/// Abort `win_float_special()`: emit any error, tear down the half-built window, unblock autocmds.
#[inline]
fn win_float_special_fail(
    wp: Option<&'static mut WinInfoInfo>,
    err: &mut Error,
) -> Option<&'static mut WinInfoInfo> {
    if error_set(err) {
        emsg(err.msg_cstr().unwrap_or(c""));
        api_clear_error(err);
    }
    if let Some(wp) = wp {
        win_remove(wp, None);
        win_free(wp, None);
    }
    unblock_autocmds();
    None
}

/// Snapshot a 'previewpopup' preview's anchor: the current cursor's screen cell (`win_float_anchor_preview`).
pub fn win_float_anchor_preview(wp: &mut WinInfoInfo) {
    wp.w_wantline
        .set(curwin_mut().w_winrow.get() + curwin_mut().w_wrow.get());
    wp.w_wantcol
        .set(curwin_mut().w_wincol.get() + curwin_mut().w_wcol.get());
}

/// Creates a special floatwin: a 'previewpopup' preview or a pum-info popup (`win_float_special`).
pub fn win_float_special(
    enter: bool,
    new_buf: bool,
    kind: WinKind,
) -> Option<&'static mut WinInfoInfo> {
    let mut config = WIN_CONFIG_INIT;
    config.col = curwin_mut().w_wcol.get() as f64;
    config.row = curwin_mut().w_wrow.get() as f64;
    config.relative = K_FLOAT_RELATIVE_EDITOR;
    config.focusable = false;
    config.mouse = true;
    config.anchor = 0; // NW
    config.noautocmd = true;
    config.hide = true;
    config.style = K_WIN_STYLE_MINIMAL;
    let mut err = error_init();
    let preview = kind == K_WIN_PREVIEW;
    if preview && !win_previewpopup_config(opt_vars().p_pvp.as_cstr(), &mut config) {
        emsg(gettext(E_INVARG));
        return None;
    }

    block_autocmds();
    let Some(wp) = win_new_float(None, false, config, &mut err) else {
        return win_float_special_fail(None, &mut err);
    };

    if new_buf {
        let b = nvim_create_buf(false, true, &mut err);
        if b == 0 {
            return win_float_special_fail(Some(wp), &mut err);
        }
        let Some(buf) = crate::api::private::helpers::find_buffer_by_handle_mut(b, &mut err) else {
            return win_float_special_fail(Some(wp), &mut err);
        };
        buf.b_p_bl.set(false); // unlist
        set_option_direct_for(
            K_OPT_BUFHIDDEN,
            cstr_as_obj(Some(c"wipe")),
            OPT_LOCAL,
            0,
            OptTarget::Buf(buf.into()),
        );
        win_set_buf(&mut *wp, buf, &mut err);
        if error_set(&err) {
            return win_float_special_fail(Some(wp), &mut err);
        }
    }
    unblock_autocmds();
    wp.w_onebuf_opt.wo_diff.set(false);
    wp.w_onebuf_opt.wo_wrap.set(true); // 'wrap' is default on
    wp.w_onebuf_opt.wo_so.set(0); // 'scrolloff' zero

    if preview {
        wp.w_onebuf_opt.wo_pvw.set(true);
        wp.w_onebuf_opt.wo_scb.set(false);
        wp.w_onebuf_opt.wo_crb.set(false);
        win_float_anchor_preview(&mut *wp);
    }
    wp.w_kind.set(kind);

    if enter {
        win_enter(wp, false);
    }
    Some(wp)
}

/// Closes a specified floating window used for previews or popups (`win_float_close`).
pub fn win_float_close(kind: WinKind) {
    if let Some(wp) = win_float_find(kind) {
        win_close(wp, false, false);
    }
}

/// Re-layout a 'previewpopup' window for its current content (`win_float_update_preview`).
pub fn win_float_update_preview(wp: &mut WinInfoInfo) {
    if wp.w_kind.get() != K_WIN_PREVIEW || !wp.w_floating.get() || wp.w_buffer.get().is_null() {
        return;
    }

    // Title is the previewed file's name, only known once it is loaded.
    if wp.w_config.border && wp.buf_mut().fname_opt().is_some() {
        let mut dirname = [0u8; MAXPATHL];
        os_dirname(&mut dirname);
        shorten_buf_fname(wp.buf_mut(), crate::memory::bytes_as_cstr(&dirname), 0);

        wp.w_config.title_chunks.clear();
        let mut err = error_init();
        parse_bordertext(
            cstr_as_obj(wp.buf_mut().fname_opt()),
            K_BORDER_TEXT_TITLE,
            &mut wp.w_config,
            &mut err,
        );
        api_clear_error(&mut err);
        wp.w_config.title_pos = K_ALIGN_CENTER;
    }

    let border_h = win_border_height(&mut *wp);
    let border_w = win_border_width(&mut *wp);
    let row = wp.w_wantline.get().max(0).min(g_st().rows_global.get() - 1);
    let col = wp
        .w_wantcol
        .get()
        .max(0)
        .min(g_st().columns_global.get() - 1);

    let v: OptKeyDict_pvp = opt_keyset(opt_vars().p_pvp.as_bytes(), K_OPT_PREVIEWPOPUP);
    let mut want_w = if has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_WIDTH) {
        v.width as i32
    } else {
        0
    };
    let mut want_h = if has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_HEIGHT) {
        v.height as i32
    } else {
        0
    };

    // If a dimension is not given in 'previewpopup', autosize to the content.
    let last: Linenr = wp.buf_mut().b_ml.ml_line_count.get();
    if want_w == 0 {
        want_w = win_max_displaywidth(&mut *wp, 1, last, g_st().columns_global.get()).max(1);
    }
    want_w = want_w.min(g_st().columns_global.get());

    if want_h == 0 {
        let saved_view_width = wp.w_view_width.get();
        wp.w_view_width.set(want_w);
        want_h = plines_m_win(wp, 1, last, g_st().rows_global.get()).max(1);
        wp.w_view_width.set(saved_view_width);
    }
    want_h = want_h.min(g_st().rows_global.get());

    let need_h = want_h + border_h;
    let need_w = want_w + border_w;
    let space_below = g_st().rows_global.get() - row - 1 - (opt_vars().p_ch.get() as i32);
    let space_above = row - 1;
    let space_right = g_st().columns_global.get() - col - 1;
    let space_left = col;
    // Prefer below/right; flip only if this side can't fit but the other can.
    let below = space_below >= need_h || space_above < need_h;
    let right = space_right >= need_w || space_left < need_w;
    let max_height = (if below { space_below } else { space_above }) - border_h;
    let max_width = (if right { space_right } else { space_left }) - border_w;

    wp.w_config.height = want_h.min(max_height).max(1);
    wp.w_config.width = want_w.min(max_width).max(1);
    wp.w_config.anchor = (if below { 0 } else { K_FLOAT_ANCHOR_SOUTH })
        | (if right { 0 } else { K_FLOAT_ANCHOR_EAST });
    wp.w_config.row = (row + if below { 1 } else { 0 }) as f64;
    wp.w_config.col = (col + if right { 1 } else { 0 }) as f64;
    wp.w_config.hide = false;
    let cfg = wp.w_config.clone();
    win_config_float(&mut *wp, cfg);
}

/// Applies a 'previewpopup' value ("height:N,width:N,border:style") to `config` (`win_previewpopup_config`).
pub fn win_previewpopup_config(value: &std::ffi::CStr, config: &mut WinConfig) -> bool {
    {
        let v: OptKeyDict_pvp = opt_keyset(value.to_bytes(), K_OPT_PREVIEWPOPUP);

        if (has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_HEIGHT) && v.height < 1)
            || (has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_WIDTH) && v.width < 1)
        {
            return false;
        }
        config.height = if has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_HEIGHT) {
            v.height as i32
        } else {
            0
        };
        config.width = if has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_WIDTH) {
            v.width as i32
        } else {
            0
        };

        // Use 'previewpopup' border if set, else fallback to 'winborder'.
        let p_winborder = opt_vars().p_winborder.to_owned();
        let border = if has_key(v.is_set_pvp, KEYSET_OPTIDX_PVP_BORDER) {
            Some(crate::memory::bytes_until_nul(&v.border))
        } else {
            Some(p_winborder.as_bytes()).filter(|b| !b.is_empty())
        };
        if let Some(border) = border {
            let mut err = error_init();
            let ok = parse_winborder(Some(&mut *config), border, &mut err);
            api_clear_error(&mut err);
            if !ok {
                return false;
            }
        }

        if !config.border {
            config.title = false;
        }
        true
    }
}
