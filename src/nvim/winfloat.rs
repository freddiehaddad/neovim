//! Floating window creation, configuration, and layout (`src/nvim/winfloat.c`).

use crate::api::private::helpers::{
    api_clear_error, api_set_error, cstr_as_obj, error_init, error_set,
    find_window_by_handle, has_key,
};
use crate::api::private::validate::{Error, ErrorType};
use crate::api::vim::nvim_create_buf;
use crate::api::win_config::{
    WIN_CONFIG_INIT, kAlignCenter, kBorderTextTitle, kFloatAnchorEast, kFloatAnchorSouth,
    kFloatRelativeCursor, kFloatRelativeEditor, kFloatRelativeLaststatus, kFloatRelativeMouse,
    kFloatRelativeWindow, kUIMultigrid, kWinStyleMinimal, parse_bordertext, parse_winborder,
};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::cursor::{linenr_T, pos_T};
use crate::drawscreen::{UPD_NOT_VALID, UPD_VALID, redraw_later, set_must_redraw};
use crate::errors::*;
use crate::fileio::shorten_buf_fname;
use crate::globals::*;
use crate::grid::grid_adjust;
use crate::message::{emsg, kZIndexMessages};
use crate::mouse::mouse_find_win_inner;
use crate::r#move::{textpos2screenpos, validate_cursor};
use crate::option::{OPT_LOCAL, OptTarget, kOptBufhidden, parse_winhl_opt, set_option_direct_for};
use crate::option_vars::*;
use crate::optionstr::{
    OptKeyDict_pvp, alloc_string_option, free_string_option, kOptPreviewpopup, opt_keyset,
};
use crate::os::fs::{MAXPATHL, os_dirname};
use crate::plines::{plines_m_win, win_max_displaywidth};
use crate::strings::{concat_str, gettext};
use crate::ui::ui_has;
use crate::window::{
    STATUS_HEIGHT, WinConfig, WinKind, last_status, lastwin_nofloating, merge_win_config,
    tabpage_T, tabpage_win_valid, win_T, win_alloc, win_append, win_close, win_comp_pos, win_enter,
    win_find_tabpage, win_free, win_init, win_remove, win_remove_status_line, win_set_buf,
    win_set_inner_size, win_valid, winframe_remove,
};
use std::ffi::{c_char, c_int};
use std::ptr;

const KEYSET_OPTIDX_pvp__width: u64 = 1;
const KEYSET_OPTIDX_pvp__border: u64 = 2;
const KEYSET_OPTIDX_pvp__height: u64 = 3;

use crate::charset::NUL;
use crate::ex_cmds::kWinPreview;

/// Creates a new float, or transforms an existing window to a float (`win_new_float`).
pub fn win_new_float(
    mut wp: *mut win_T,
    last: bool,
    fconfig: WinConfig,
    err: &mut Error,
) -> *mut win_T {
    unsafe {
        let empty_opt = empty_string_option.as_ptr().cast::<c_char>();
        if wp.is_null() {
            let mut tp_last = if last {
                g_st().lastwin
            } else {
                lastwin_nofloating(ptr::null_mut())
            };
            if fconfig.window != 0 {
                debug_assert!(!last);
                let parent_wp = find_window_by_handle(fconfig.window, err);
                if parent_wp.is_null() {
                    return ptr::null_mut();
                }
                let tp = win_find_tabpage(parent_wp);
                if tp.is_null() {
                    return ptr::null_mut();
                }
                tp_last = lastwin_nofloating(if tp == g_st().curtab { ptr::null_mut() } else { tp });
            }
            wp = win_alloc(tp_last, false);
            win_init(&mut *wp, curwin_mut(), 0);
            if !(*wp).w_onebuf_opt.wo_wbr.is_null() && fconfig.height == 1 {
                if (*wp).w_onebuf_opt.wo_wbr != empty_opt {
                    free_string_option((*wp).w_onebuf_opt.wo_wbr);
                }
                (*wp).w_onebuf_opt.wo_wbr = empty_opt;
            }
            if !(*wp).w_onebuf_opt.wo_stl.is_null() && (*wp).w_onebuf_opt.wo_stl != empty_opt {
                free_string_option((*wp).w_onebuf_opt.wo_stl);
                (*wp).w_onebuf_opt.wo_stl = empty_opt;
            }
        } else {
            debug_assert!(!last);
            debug_assert!(!(*wp).w_floating);
            let win_tp = win_find_tabpage(wp);
            debug_assert!(!win_tp.is_null());
            if (win_tp == g_st().curtab && g_st().firstwin == wp && lastwin_nofloating(ptr::null_mut()) == wp)
                || (win_tp != g_st().curtab
                    && (*win_tp).tp_firstwin == wp
                    && lastwin_nofloating(win_tp) == wp)
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Cannot change last window into float",
                );
                return ptr::null_mut();
            }
            let tp = if win_tp == g_st().curtab {
                ptr::null_mut()
            } else {
                win_tp
            };
            let mut dir: c_int = 0;
            winframe_remove( &mut *wp, &mut dir, tp, ptr::null_mut());
            if !(*wp).w_frame.is_null() {
                drop(Box::from_raw((*wp).w_frame));
            }
            (*wp).w_frame = ptr::null_mut();
            win_remove( &mut *wp, tp);
            if win_tp == g_st().curtab {
                last_status(false);
                win_comp_pos();
            }
            win_append(lastwin_nofloating(tp), wp, tp);
        }
        (*wp).w_floating = true;
        (*wp).w_status_height = if !(*wp).w_onebuf_opt.wo_stl.is_null()
            && *(*wp).w_onebuf_opt.wo_stl != NUL
            && (opt_vars().p_ls == 1 || opt_vars().p_ls == 2)
        {
            STATUS_HEIGHT
        } else {
            0
        };
        (*wp).w_winbar_height = 0;
        (*wp).w_hsep_height = 0;
        (*wp).w_vsep_width = 0;

        win_config_float(&mut *wp, fconfig);
        redraw_later(wp.as_mut(), UPD_VALID);
        wp
    }
}

pub fn win_set_minimal_style(wp: &mut win_T) {
    unsafe {
        let empty_opt = empty_string_option.as_ptr().cast::<c_char>();
        wp.w_onebuf_opt.wo_nu = 0;
        wp.w_onebuf_opt.wo_rnu = 0;
        wp.w_onebuf_opt.wo_cul = 0;
        wp.w_onebuf_opt.wo_cuc = 0;
        wp.w_onebuf_opt.wo_spell = 0;
        wp.w_onebuf_opt.wo_list = 0;

        // Hide EOB region: use " " fillchar and cleared highlighting
        if wp.w_p_fcs_chars.eob != b' ' as u32 {
            let old = wp.w_onebuf_opt.wo_fcs;
            wp.w_onebuf_opt.wo_fcs = if *old == NUL {
                alloc_string_option(c"eob: ")
            } else {
                alloc_string_option(concat_str(old, c",eob: ").as_ptr())
            };
            free_string_option(old);
        }

        let old = wp.w_onebuf_opt.wo_winhl;
        wp.w_onebuf_opt.wo_winhl = if *old == NUL {
            alloc_string_option(c"EndOfBuffer:")
        } else {
            alloc_string_option(concat_str(old, c",EndOfBuffer:").as_ptr())
        };
        free_string_option(old);
        parse_winhl_opt(ptr::null(), Some(&mut *wp));

        // signcolumn: use 'auto'
        let scl_len = crate::memory::cstr_len(wp.w_onebuf_opt.wo_scl);
        if *wp.w_onebuf_opt.wo_scl != b'a' as c_char || scl_len >= 8 {
            free_string_option(wp.w_onebuf_opt.wo_scl);
            wp.w_onebuf_opt.wo_scl = alloc_string_option(c"auto");
        }

        // foldcolumn: use '0'
        if *wp.w_onebuf_opt.wo_fdc != b'0' as c_char {
            free_string_option(wp.w_onebuf_opt.wo_fdc);
            wp.w_onebuf_opt.wo_fdc = alloc_string_option(c"0");
        }

        // colorcolumn: cleared
        if !wp.w_onebuf_opt.wo_cc.is_null() && *wp.w_onebuf_opt.wo_cc != NUL {
            free_string_option(wp.w_onebuf_opt.wo_cc);
            wp.w_onebuf_opt.wo_cc = alloc_string_option(c"");
        }

        // statuscolumn: cleared
        if !wp.w_onebuf_opt.wo_stc.is_null() && *wp.w_onebuf_opt.wo_stc != NUL {
            free_string_option(wp.w_onebuf_opt.wo_stc);
            wp.w_onebuf_opt.wo_stc = empty_opt;
        }

        // statusline: cleared (for floating windows)
        if wp.w_floating
            && !wp.w_onebuf_opt.wo_stl.is_null()
            && *wp.w_onebuf_opt.wo_stl != NUL
        {
            free_string_option(wp.w_onebuf_opt.wo_stl);
            wp.w_onebuf_opt.wo_stl = empty_opt;
            if wp.w_status_height > 0 {
                let cfg = wp.w_config.clone();
                win_config_float(&mut *wp, cfg);
            }
        }
    }
}

pub fn win_border_height(wp: &mut win_T) -> c_int {
    wp.w_border_adj[0] + wp.w_border_adj[2]
}

pub fn win_border_width(wp: &mut win_T) -> c_int {
    wp.w_border_adj[1] + wp.w_border_adj[3]
}

pub fn win_config_float(wp: &mut win_T, mut fconfig: WinConfig) {
    unsafe {
        // Process statusline changes before applying new height from config
        let show_stl = *wp.w_onebuf_opt.wo_stl != NUL && (opt_vars().p_ls == 1 || opt_vars().p_ls == 2);
        if wp.w_status_height != 0 && !show_stl {
            win_remove_status_line(wp, false);
        } else if wp.w_status_height == 0 && show_stl {
            wp.w_status_height = STATUS_HEIGHT;
        }

        wp.w_width = fconfig.width.max(1);
        wp.w_height = fconfig.height.max(1);

        if fconfig.relative == kFloatRelativeCursor {
            validate_cursor(curwin_mut());
            fconfig.relative = kFloatRelativeWindow;
            fconfig.row += curwin_mut().w_wrow as f64;
            fconfig.col += curwin_mut().w_wcol as f64;
            fconfig.window = curwin_mut().handle;
        } else if fconfig.relative == kFloatRelativeMouse {
            let mut row = g_st().mouse_row;
            let mut col = g_st().mouse_col;
            let mut grid = g_st().mouse_grid;
            let mouse_win = mouse_find_win_inner(&mut grid, &mut row, &mut col);
            if !mouse_win.is_null() {
                fconfig.relative = kFloatRelativeWindow;
                fconfig.row += row as f64;
                fconfig.col += col as f64;
                fconfig.window = (*mouse_win).handle;
            }
        }

        let change_external = fconfig.external != wp.w_config.external;
        let mut change_border = fconfig.border != wp.w_config.border
            || fconfig.border_hl_ids != wp.w_config.border_hl_ids;

        merge_win_config(&mut wp.w_config, fconfig);

        let has_border = wp.w_floating && wp.w_config.border;
        for i in 0..4 {
            let new_adj = c_int::from(has_border && wp.w_config.border_chars[2 * i + 1][0] != 0);
            if new_adj != wp.w_border_adj[i] {
                change_border = true;
                wp.w_border_adj[i] = new_adj;
            }
        }

        if !ui_has(kUIMultigrid) {
            let above_ch = if wp.w_config.zindex < kZIndexMessages {
                opt_vars().p_ch as c_int
            } else {
                0
            };
            wp.w_height = wp.w_height.min(g_st().Rows - win_border_height(wp) - above_ch);
            wp.w_width = wp.w_width.min(g_st().Columns - win_border_width(wp));
        }

        win_set_inner_size(wp, true);
        set_must_redraw(UPD_VALID);
        wp.w_redr_status = wp.w_status_height != 0;
        wp.w_pos_changed = true;
        if change_external || change_border {
            wp.w_hl_needs_update = 1;
            redraw_later(Some(&mut *wp), UPD_NOT_VALID);
        }

        // compute initial position
        if wp.w_config.relative == kFloatRelativeWindow {
            let mut row = wp.w_config.row as c_int;
            let mut col = wp.w_config.col as c_int;
            let mut dummy = error_init();
            let parent = find_window_by_handle(wp.w_config.window, &mut dummy);
            if !parent.is_null() {
                row += (*parent).w_winrow;
                col += (*parent).w_wincol;
                grid_adjust(&mut (*parent).w_grid, &mut row, &mut col);
                if wp.w_config.bufpos.lnum >= 0 {
                    let mut pos = pos_T {
                        lnum: (wp.w_config.bufpos.lnum + 1)
                            .min((*parent).buf_mut().b_ml.ml_line_count),
                        col: wp.w_config.bufpos.col,
                        coladd: 0,
                    };
                    let mut trow: c_int = 0;
                    let mut tcol: c_int = 0;
                    let mut tcolc: c_int = 0;
                    let mut tcole: c_int = 0;
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
            wp.w_winrow = row;
            wp.w_wincol = col;
        } else {
            wp.w_winrow = wp.w_config.row as c_int;
            wp.w_wincol = wp.w_config.col as c_int;
        }

        // changing border style while keeping border only requires redrawing border
        if wp.w_config.border {
            wp.w_redr_border = true;
            redraw_later(Some(&mut *wp), UPD_VALID);
        }
    }
}

pub fn win_float_remove(bang: bool, mut count: c_int) {
    unsafe {
        let mut float_win_arr: Vec<*mut win_T> = Vec::new();
        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            if !(*wp).w_config.hide && (*wp).w_onebuf_opt.wo_wp == 0 {
                float_win_arr.push(wp);
            }
            wp = (*wp).w_prev;
        }
        if !float_win_arr.is_empty() {
            float_win_arr.sort_unstable_by_key(|&w| std::cmp::Reverse((*w).w_config.zindex));
        }
        for w in float_win_arr {
            if win_valid(w) && !win_close(w, false, false) {
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
}

pub fn win_check_anchored_floats(win: &mut win_T) {
    unsafe {
        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            if (*wp).w_config.relative == kFloatRelativeWindow
                && (*wp).w_config.window == win.handle
            {
                (*wp).w_pos_changed = true;
            }
            wp = (*wp).w_prev;
        }
    }
}

pub fn win_float_update_statusline() {
    unsafe {
        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            let has_status = (*wp).w_status_height > 0;
            let should_show = *(*wp).w_onebuf_opt.wo_stl != NUL && (opt_vars().p_ls == 1 || opt_vars().p_ls == 2);
            if should_show != has_status {
                win_config_float(&mut *wp, (*wp).w_config.clone());
            }
            wp = (*wp).w_prev;
        }
    }
}

pub fn win_float_anchor_laststatus() {
     {
        for win in for_all_windows_mut() {
            if win.w_config.relative == kFloatRelativeLaststatus {
                win.w_pos_changed = true;
            }
        }
    }
}

pub fn win_reconfig_floats() {
    unsafe {
        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            win_config_float(&mut *wp, (*wp).w_config.clone());
            wp = (*wp).w_prev;
        }
    }
}

/// Find the floating window of the given kind in the current tabpage (`win_float_find`).
pub fn win_float_find(kind: WinKind) -> *mut win_T {
    unsafe {
        let mut wp = g_st().lastwin;
        while !wp.is_null() && (*wp).w_floating {
            if (*wp).w_kind == kind {
                return wp;
            }
            wp = (*wp).w_prev;
        }
        ptr::null_mut()
    }
}

/// Select an alternative window to `win` (assumed floating) in tabpage `tp` (`win_float_find_altwin`).
pub fn win_float_find_altwin(win: *const win_T, tp: *const tabpage_T) -> *mut win_T {
    unsafe {
        let mut wp = g_st().prevwin;
        if tp.is_null() {
            return if win_valid(wp)
                && !ptr::eq(wp, win)
                && (*wp).w_config.focusable
                && !(*wp).w_config.hide
            {
                wp
            } else {
                g_st().firstwin
            };
        }

        debug_assert!(!ptr::eq(tp, g_st().curtab));
        wp = if tabpage_win_valid(tp, (*tp).tp_prevwin) {
            (*tp).tp_prevwin
        } else {
            (*tp).tp_firstwin
        };
        if !ptr::eq(wp, win) && (*wp).w_config.focusable && !(*wp).w_config.hide {
            wp
        } else {
            (*tp).tp_firstwin
        }
    }
}

/// Abort `win_float_special()`: emit any error, tear down the half-built window, unblock autocmds.
#[inline]
fn win_float_special_fail(wp: *mut win_T, err: &mut Error) -> *mut win_T {
    unsafe {
        if error_set(err) {
            emsg(err.msg_ptr());
            api_clear_error(err);
        }
        if !wp.is_null() {
            win_remove( &mut *wp, ptr::null_mut());
            win_free(wp, ptr::null_mut());
        }
        unblock_autocmds();
        ptr::null_mut()
    }
}

/// Snapshot a 'previewpopup' preview's anchor: the current cursor's screen cell (`win_float_anchor_preview`).
pub fn win_float_anchor_preview(wp: &mut win_T) {
     {
        wp.w_wantline = curwin_mut().w_winrow + curwin_mut().w_wrow;
        wp.w_wantcol = curwin_mut().w_wincol + curwin_mut().w_wcol;
    }
}

/// Creates a special floatwin: a 'previewpopup' preview or a pum-info popup (`win_float_special`).
pub fn win_float_special(enter: bool, new_buf: bool, kind: WinKind) -> *mut win_T {
    unsafe {
        let mut config = WIN_CONFIG_INIT;
        config.col = curwin_mut().w_wcol as f64;
        config.row = curwin_mut().w_wrow as f64;
        config.relative = kFloatRelativeEditor;
        config.focusable = false;
        config.mouse = true;
        config.anchor = 0; // NW
        config.noautocmd = true;
        config.hide = true;
        config.style = kWinStyleMinimal;
        let mut err = error_init();
        let preview = kind == kWinPreview;
        if preview && !win_previewpopup_config(opt_vars().p_pvp, &mut config) {
            emsg(gettext(e_invarg));
            return ptr::null_mut();
        }

        block_autocmds();
        let wp = win_new_float(ptr::null_mut(), false, config, &mut err);
        if wp.is_null() {
            return win_float_special_fail(wp, &mut err);
        }

        if new_buf {
            let b = nvim_create_buf(false, true, &mut err);
            if b == 0 {
                return win_float_special_fail(wp, &mut err);
            }
            let Some(buf) = crate::api::private::helpers::find_buffer_by_handle_mut(b, &mut err) else {
                return win_float_special_fail(wp, &mut err);
            };
            buf.b_p_bl = 0; // unlist
            set_option_direct_for(
                kOptBufhidden,
                cstr_as_obj(c"wipe"),
                OPT_LOCAL,
                0,
                OptTarget::Buf(&raw mut *buf),
            );
            win_set_buf(&mut *wp, buf, &mut err);
            if error_set(&err) {
                return win_float_special_fail(wp, &mut err);
            }
        }
        unblock_autocmds();
        (*wp).w_onebuf_opt.wo_diff = 0;
        (*wp).w_onebuf_opt.wo_wrap = 1; // 'wrap' is default on
        (*wp).w_onebuf_opt.wo_so = 0; // 'scrolloff' zero

        if preview {
            (*wp).w_onebuf_opt.wo_pvw = 1;
            (*wp).w_onebuf_opt.wo_scb = 0;
            (*wp).w_onebuf_opt.wo_crb = 0;
            win_float_anchor_preview(&mut *wp);
        }
        (*wp).w_kind = kind;

        if enter {
            win_enter(wp, false);
        }
        wp
    }
}

/// Closes a specified floating window used for previews or popups (`win_float_close`).
pub fn win_float_close(kind: WinKind) {
    let wp = win_float_find(kind);
        if !wp.is_null() {
            win_close(wp, false, false);
        }
}

/// Re-layout a 'previewpopup' window for its current content (`win_float_update_preview`).
pub fn win_float_update_preview(wp: &mut win_T) {
    unsafe {
        if wp.w_kind != kWinPreview || !wp.w_floating || wp.w_buffer.is_null() {
            return;
        }

        // Title is the previewed file's name, only known once it is loaded.
        if wp.w_config.border && !wp.buf_mut().b_fname.is_null() {
            let mut dirname = [0 as c_char; MAXPATHL];
            os_dirname(dirname.as_mut_ptr(), MAXPATHL);
            shorten_buf_fname(wp.buf_mut(), dirname.as_mut_ptr(), 0);

            wp.w_config.title_chunks.clear();
            let mut err = error_init();
            parse_bordertext(
                cstr_as_obj(wp.buf_mut().b_fname),
                kBorderTextTitle,
                &mut wp.w_config,
                &mut err,
            );
            api_clear_error(&mut err);
            wp.w_config.title_pos = kAlignCenter;
        }

        let border_h = win_border_height(&mut *wp);
        let border_w = win_border_width(&mut *wp);
        let row = wp.w_wantline.max(0).min(g_st().Rows - 1);
        let col = wp.w_wantcol.max(0).min(g_st().Columns - 1);

        let v = opt_keyset(opt_vars().p_pvp, kOptPreviewpopup, ptr::null_mut()).cast::<OptKeyDict_pvp>();
        let mut want_w = if has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__width) {
            (*v).width as c_int
        } else {
            0
        };
        let mut want_h = if has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__height) {
            (*v).height as c_int
        } else {
            0
        };

        // If a dimension is not given in 'previewpopup', autosize to the content.
        let last: linenr_T = wp.buf_mut().b_ml.ml_line_count;
        if want_w == 0 {
            want_w = win_max_displaywidth(&mut *wp, 1, last, g_st().Columns).max(1);
        }
        want_w = want_w.min(g_st().Columns);

        if want_h == 0 {
            let saved_view_width = wp.w_view_width;
            wp.w_view_width = want_w;
            want_h = plines_m_win(wp, 1, last, g_st().Rows).max(1);
            wp.w_view_width = saved_view_width;
        }
        want_h = want_h.min(g_st().Rows);

        let need_h = want_h + border_h;
        let need_w = want_w + border_w;
        let space_below = g_st().Rows - row - 1 - (opt_vars().p_ch as c_int);
        let space_above = row - 1;
        let space_right = g_st().Columns - col - 1;
        let space_left = col;
        // Prefer below/right; flip only if this side can't fit but the other can.
        let below = space_below >= need_h || space_above < need_h;
        let right = space_right >= need_w || space_left < need_w;
        let max_height = (if below { space_below } else { space_above }) - border_h;
        let max_width = (if right { space_right } else { space_left }) - border_w;

        wp.w_config.height = want_h.min(max_height).max(1);
        wp.w_config.width = want_w.min(max_width).max(1);
        wp.w_config.anchor = (if below { 0 } else { kFloatAnchorSouth })
            | (if right { 0 } else { kFloatAnchorEast });
        wp.w_config.row = (row + if below { 1 } else { 0 }) as f64;
        wp.w_config.col = (col + if right { 1 } else { 0 }) as f64;
        wp.w_config.hide = false;
        let cfg = wp.w_config.clone();
        win_config_float(&mut *wp, cfg);
    }
}

/// Applies a 'previewpopup' value ("height:N,width:N,border:style") to `config` (`win_previewpopup_config`).
pub fn win_previewpopup_config(value: *const c_char, config: &mut WinConfig) -> bool {
    unsafe {
        let v = opt_keyset(value, kOptPreviewpopup, ptr::null_mut()).cast::<OptKeyDict_pvp>();

        if (has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__height) && (*v).height < 1)
            || (has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__width) && (*v).width < 1)
        {
            return false;
        }
        config.height = if has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__height) {
            (*v).height as c_int
        } else {
            0
        };
        config.width = if has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__width) {
            (*v).width as c_int
        } else {
            0
        };

        // Use 'previewpopup' border if set, else fallback to 'winborder'.
        let mut border: *mut c_char = ptr::null_mut();
        if has_key((*v).is_set__pvp_, KEYSET_OPTIDX_pvp__border) {
            border = (*v).border.as_mut_ptr();
        } else if !crate::memory::cstr_is_empty(opt_vars().p_winborder) {
            border = opt_vars().p_winborder;
        }
        if !border.is_null() {
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
