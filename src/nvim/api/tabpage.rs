//! Tabpage API functions (`src/nvim/api/tabpage.c`).

#![allow(
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::empty_line_after_doc_comments,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::api::autocmd::OptionalKeys;
#[cfg(not(test))]
use crate::api::autocmd::TryState;
use crate::api::deprecated::VirtText;
use crate::api::private::converter::sctx_T;
#[cfg(not(test))]
use crate::api::private::helpers::{
    api_clear_error, api_set_error, arena_array, dict_get_value, dict_set_var, error_set,
    find_buffer_by_handle, find_tab_by_handle, find_window_by_handle, try_enter, try_leave,
};
use crate::api::private::helpers::{nil_obj, window_obj};
#[cfg(not(test))]
use crate::api::private::validate::ErrorType;
use crate::api::private::validate::{Array, Error, Object};
#[cfg(not(test))]
use crate::api::vim::nvim_get_current_win;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{Callback, buf_T, fmark_T};
use crate::garray::garray_T;
#[cfg(not(test))]
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::map::{Map, Set, StcClicks};
use crate::map_glyph_cache::NvimString;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::{Integer, handle_T};
#[cfg(not(test))]
use crate::window::{
    tabpage_index, tabpage_win_valid, valid_tabpage, win_goto, win_new_tabpage, win_set_buf,
};
use std::ffi::{c_char, c_int, c_uint, c_void};

pub use crate::eval::deprecated::pos_T;

pub type Buffer = handle_T;
pub type Window = handle_T;
pub type Tabpage = handle_T;

#[cfg(not(test))]
const KEYSET_OPTIDX_tabpage_config__after: u64 = 1;
const K_WIN_OPT_COUNT: usize = 53;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pos_save_T {
    pub w_topline_save: c_int,
    pub w_topline_corr: c_int,
    pub w_cursor_save: pos_T,
    pub w_cursor_corr: pos_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct lcs_chars_T {
    pub eol: u32,
    pub ext: u32,
    pub prec: u32,
    pub nbsp: u32,
    pub space: u32,
    pub tab1: u32,
    pub tab2: u32,
    pub tab3: u32,
    pub leadtab1: u32,
    pub leadtab2: u32,
    pub leadtab3: u32,
    pub lead: u32,
    pub trail: u32,
    pub multispace: *mut u32,
    pub leadmultispace: *mut u32,
    pub conceal: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fcs_chars_T {
    pub stl: u32,
    pub stlnc: u32,
    pub wbr: u32,
    pub horiz: u32,
    pub horizup: u32,
    pub horizdown: u32,
    pub vert: u32,
    pub vertleft: u32,
    pub vertright: u32,
    pub verthoriz: u32,
    pub fold: u32,
    pub foldopen: u32,
    pub foldclosed: u32,
    pub foldsep: u32,
    pub foldinner: u32,
    pub diff: u32,
    pub msgsep: u32,
    pub eob: u32,
    pub lastline: u32,
    pub trunc: u32,
    pub truncrl: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct winopt_T {
    pub wo_arab: c_int,
    pub wo_bri: c_int,
    pub wo_briopt: *mut c_char,
    pub wo_diff: c_int,
    pub wo_fdc: *mut c_char,
    pub wo_eiw: *mut c_char,
    pub wo_fdc_save: *mut c_char,
    pub wo_fen: c_int,
    pub wo_fen_save: c_int,
    pub wo_fdi: *mut c_char,
    pub wo_fdl: i64,
    pub wo_fdl_save: i64,
    pub wo_fdm: *mut c_char,
    pub wo_fdm_save: *mut c_char,
    pub wo_fml: i64,
    pub wo_fdn: i64,
    pub wo_fde: Callback,
    pub wo_fdt: Callback,
    pub wo_fmr: *mut c_char,
    pub wo_lbr: c_int,
    pub wo_list: c_int,
    pub wo_nu: c_int,
    pub wo_rnu: c_int,
    pub wo_ve: *mut c_char,
    pub wo_ve_flags: c_uint,
    pub wo_nuw: i64,
    pub wo_wfb: c_int,
    pub wo_wfh: c_int,
    pub wo_wfw: c_int,
    pub wo_wp: c_int,
    pub wo_pvw: c_int,
    pub wo_lhi: i64,
    pub wo_rl: c_int,
    pub wo_rlc: *mut c_char,
    pub wo_scr: i64,
    pub wo_sms: c_int,
    pub wo_spell: c_int,
    pub wo_cuc: c_int,
    pub wo_cul: c_int,
    pub wo_culopt: *mut c_char,
    pub wo_cc: *mut c_char,
    pub wo_sbr: *mut c_char,
    pub wo_stc: *mut c_char,
    pub wo_stl: *mut c_char,
    pub wo_wbr: *mut c_char,
    pub wo_scb: c_int,
    pub wo_diff_saved: c_int,
    pub wo_scb_save: c_int,
    pub wo_wrap: c_int,
    pub wo_wrap_save: c_int,
    pub wo_cocu: *mut c_char,
    pub wo_cole: i64,
    pub wo_crb: c_int,
    pub wo_crb_save: c_int,
    pub wo_scl: *mut c_char,
    pub wo_siso: i64,
    pub wo_so: i64,
    pub wo_sop: i64,
    pub wo_winhl: *mut c_char,
    pub wo_lcs: *mut c_char,
    pub wo_fcs: *mut c_char,
    pub wo_winbl: i64,
    pub wo_wrap_flags: u32,
    pub wo_stl_flags: u32,
    pub wo_wbr_flags: u32,
    pub wo_fde_flags: u32,
    pub wo_fdt_flags: u32,
    pub wo_script_ctx: [sctx_T; K_WIN_OPT_COUNT],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct lpos_T {
    pub lnum: i32,
    pub col: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct xfmark_T {
    pub fmark: fmark_T,
    pub fname: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct taggy_T {
    pub tagname: *mut c_char,
    pub fmark: fmark_T,
    pub cur_match: c_int,
    pub cur_fnum: c_int,
    pub user_data: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScreenGrid {
    pub handle: handle_T,
    pub chars: *mut u32,
    pub attrs: *mut i32,
    pub vcols: *mut i32,
    pub line_offset: *mut usize,
    pub dirty_col: *mut c_int,
    pub rows: c_int,
    pub cols: c_int,
    pub valid: bool,
    pub throttled: bool,
    pub blending: bool,
    pub mouse_enabled: bool,
    pub zindex: c_int,
    pub comp_row: c_int,
    pub comp_col: c_int,
    pub comp_width: c_int,
    pub comp_height: c_int,
    pub comp_index: usize,
    pub comp_disabled: bool,
    pub pending_comp_index_update: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GridView {
    pub target: *mut ScreenGrid,
    pub row_offset: c_int,
    pub col_offset: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WinConfig {
    pub window: Window,
    pub bufpos: lpos_T,
    pub height: c_int,
    pub width: c_int,
    pub row: f64,
    pub col: f64,
    pub anchor: c_int,
    pub relative: c_int,
    pub external: bool,
    pub focusable: bool,
    pub mouse: bool,
    pub split: c_int,
    pub zindex: c_int,
    pub style: c_int,
    pub border: bool,
    pub shadow: bool,
    pub border_chars: [[c_char; 32]; 8],
    pub border_hl_ids: [c_int; 8],
    pub border_attr: [c_int; 8],
    pub title: bool,
    pub title_pos: c_int,
    pub title_chunks: VirtText,
    pub title_width: c_int,
    pub footer: bool,
    pub footer_pos: c_int,
    pub footer_chunks: VirtText,
    pub footer_width: c_int,
    pub noautocmd: bool,
    pub fixed: bool,
    pub hide: bool,
    pub _cmdline_offset: c_int,
}

#[repr(C)]
pub struct frame_T {
    pub fr_layout: c_char,
    pub fr_width: c_int,
    pub fr_newwidth: c_int,
    pub fr_height: c_int,
    pub fr_newheight: c_int,
    pub fr_parent: *mut frame_T,
    pub fr_next: *mut frame_T,
    pub fr_prev: *mut frame_T,
    pub fr_child: *mut frame_T,
    pub fr_win: *mut win_T,
}

pub use crate::eval::deprecated::ScopeDictDictItem;

#[repr(C)]
pub struct win_T {
    pub handle: handle_T,
    pub w_buffer: *mut buf_T,
    pub w_s: *mut c_void,
    pub w_ns_hl: c_int,
    pub w_ns_hl_winhl: c_int,
    pub w_ns_hl_active: c_int,
    pub w_ns_hl_attr: *mut c_int,
    pub w_ns_set: Set<u32>,
    pub w_hl_id_normal: c_int,
    pub w_hl_attr_normal: c_int,
    pub w_hl_attr_normalnc: c_int,
    pub w_hl_needs_update: c_int,
    pub w_prev: *mut win_T,
    pub w_next: *mut win_T,
    pub w_locked: c_int,
    pub w_frame: *mut frame_T,
    pub w_cursor: pos_T,
    pub w_curswant: i32,
    pub w_set_curswant: bool,
    pub w_cursorline: i32,
    pub w_last_cursorline: i32,
    pub w_old_visual_mode: c_char,
    pub w_old_cursor_lnum: i32,
    pub w_old_cursor_fcol: i32,
    pub w_old_cursor_lcol: i32,
    pub w_old_visual_lnum: i32,
    pub w_old_visual_col: i32,
    pub w_old_curswant: i32,
    pub w_last_cursor_lnum_rnu: i32,
    pub w_p_lcs_chars: lcs_chars_T,
    pub w_p_fcs_chars: fcs_chars_T,
    pub w_topline: i32,
    pub w_topline_was_set: bool,
    pub w_topfill: c_int,
    pub w_old_topfill: c_int,
    pub w_botfill: bool,
    pub w_old_botfill: bool,
    pub w_leftcol: i32,
    pub w_skipcol: i32,
    pub w_last_topline: i32,
    pub w_last_topfill: c_int,
    pub w_last_leftcol: i32,
    pub w_last_skipcol: i32,
    pub w_last_width: c_int,
    pub w_last_height: c_int,
    pub w_winrow: c_int,
    pub w_height: c_int,
    pub w_prev_winrow: c_int,
    pub w_prev_height: c_int,
    pub w_status_height: c_int,
    pub w_winbar_height: c_int,
    pub w_wincol: c_int,
    pub w_width: c_int,
    pub w_hsep_height: c_int,
    pub w_vsep_width: c_int,
    pub w_save_cursor: pos_save_T,
    pub w_do_win_fix_cursor: bool,
    pub w_wantline: c_int,
    pub w_wantcol: c_int,
    pub w_winrow_off: c_int,
    pub w_wincol_off: c_int,
    pub w_view_height: c_int,
    pub w_view_width: c_int,
    pub w_height_request: c_int,
    pub w_width_request: c_int,
    pub w_border_adj: [c_int; 4],
    pub w_height_outer: c_int,
    pub w_width_outer: c_int,
    pub w_valid: c_int,
    pub w_valid_cursor: pos_T,
    pub w_valid_leftcol: i32,
    pub w_valid_skipcol: i32,
    pub w_viewport_invalid: bool,
    pub w_viewport_last_topline: i32,
    pub w_viewport_last_botline: i32,
    pub w_viewport_last_topfill: i32,
    pub w_viewport_last_skipcol: i32,
    pub w_cline_height: c_int,
    pub w_cline_folded: bool,
    pub w_cline_row: c_int,
    pub w_virtcol: i32,
    pub w_wrow: c_int,
    pub w_wcol: c_int,
    pub w_wcol_conceal_off: c_int,
    pub w_botline: i32,
    pub w_empty_rows: c_int,
    pub w_filler_rows: c_int,
    pub w_lines_valid: c_int,
    pub w_lines: *mut c_void,
    pub w_lines_size: c_int,
    pub w_folds: garray_T,
    pub w_fold_manual: bool,
    pub w_foldinvalid: bool,
    pub w_nrwidth: c_int,
    pub w_scwidth: c_int,
    pub w_minscwidth: c_int,
    pub w_maxscwidth: c_int,
    pub w_redr_type: c_int,
    pub w_upd_rows: c_int,
    pub w_redraw_top: i32,
    pub w_redraw_bot: i32,
    pub w_redr_status: bool,
    pub w_redr_border: bool,
    pub w_redr_statuscol: bool,
    pub w_display_tick: u64,
    pub w_stl_cursor: pos_T,
    pub w_stl_virtcol: i32,
    pub w_stl_topline: i32,
    pub w_stl_line_count: i32,
    pub w_stl_topfill: c_int,
    pub w_stl_empty: c_char,
    pub w_stl_recording: c_int,
    pub w_stl_state: c_int,
    pub w_stl_visual_mode: c_int,
    pub w_stl_visual_pos: pos_T,
    pub w_alt_fnum: c_int,
    pub w_alist: *mut c_void,
    pub w_arg_idx: c_int,
    pub w_arg_idx_invalid: bool,
    pub w_localdir: *mut c_char,
    pub w_prevdir: *mut c_char,
    pub w_onebuf_opt: winopt_T,
    pub w_allbuf_opt: winopt_T,
    pub w_p_cc_cols: *mut c_int,
    pub w_p_culopt_flags: u8,
    pub w_briopt_min: c_int,
    pub w_briopt_shift: c_int,
    pub w_briopt_sbr: bool,
    pub w_briopt_list: c_int,
    pub w_briopt_vcol: c_int,
    pub w_scbind_pos: c_int,
    pub w_winvar: ScopeDictDictItem,
    pub w_vars: *mut dict_T,
    pub w_pcmark: pos_T,
    pub w_prev_pcmark: pos_T,
    pub w_jumplist: [xfmark_T; 100],
    pub w_jumplistlen: c_int,
    pub w_jumplistidx: c_int,
    pub w_changelistidx: c_int,
    pub w_match_head: *mut c_void,
    pub w_next_match_id: c_int,
    pub w_tagstack: [taggy_T; 20],
    pub w_tagstackidx: c_int,
    pub w_tagstacklen: c_int,
    pub w_grid: GridView,
    pub w_grid_alloc: ScreenGrid,
    pub w_pos_changed: bool,
    pub w_floating: bool,
    pub w_kind: c_int,
    pub w_config: WinConfig,
    pub w_fraction: c_int,
    pub w_prev_fraction_row: c_int,
    pub w_nrwidth_line_count: i32,
    pub w_statuscol_line_count: i32,
    pub w_nrwidth_width: c_int,
    pub w_llist: *mut c_void,
    pub w_llist_ref: *mut c_void,
    pub w_status_click_defs: *mut c_void,
    pub w_status_click_defs_size: usize,
    pub w_winbar_click_defs: *mut c_void,
    pub w_winbar_click_defs_size: usize,
    pub w_statuscol_click_defs: [Map<c_int, StcClicks>; 1],
}

#[repr(C)]
pub struct tabpage_T {
    pub handle: handle_T,
    pub tp_next: *mut tabpage_T,
    pub tp_topframe: *mut c_void,
    pub tp_curwin: *mut win_T,
    pub tp_prevwin: *mut win_T,
    pub tp_firstwin: *mut win_T,
    pub tp_lastwin: *mut win_T,
    pub tp_old_Rows_avail: i64,
    pub tp_old_Columns: i64,
    pub tp_ch_used: i64,
    pub tp_did_tabclosedpre: bool,
    pub tp_first_diff: *mut c_void,
    pub tp_diffbuf: [*mut buf_T; 8],
    pub tp_diff_invalid: c_int,
    pub tp_diff_update: c_int,
    pub tp_snapshot: [*mut c_void; 3],
    pub tp_winvar: ScopeDictDictItem,
    pub tp_vars: *mut dict_T,
    pub tp_localdir: *mut c_char,
    pub tp_prevdir: *mut c_char,
}

#[repr(C)]
pub struct KeyDict_tabpage_config {
    pub is_set__tabpage_config_: OptionalKeys,
    pub after: Integer,
}

#[cfg(not(test))]
#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

/// Gets the windows in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return List of windows in `tabpage`
// DLLEXPORT ArrayOf(Window) nvim_tabpage_list_wins(Tabpage tabpage, Arena *arena,
//                                                  Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_list_wins(
    tabpage: Tabpage,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    #[cfg(test)]
    {
        let _ = (tabpage, arena, err);
        Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        }
    }

    #[cfg(not(test))]
    unsafe {
        let mut rv = Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let tab = find_tab_by_handle(tabpage, err);

        if tab.is_null() || !valid_tabpage(tab) {
            return rv;
        }

        let first = if tab == curtab {
            firstwin
        } else {
            (*tab).tp_firstwin
        };
        let mut n = 0usize;
        let mut wp = first;
        while !wp.is_null() {
            n += 1;
            wp = (*wp).w_next;
        }

        rv = arena_array(arena, n);
        wp = first;
        while !wp.is_null() {
            *rv.items.add(rv.size) = window_obj((*wp).handle);
            rv.size += 1;
            wp = (*wp).w_next;
        }

        rv
    }
}

/// Gets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_tabpage_get_var(Tabpage tabpage, String name, Arena *arena,
//                                       Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_var(
    tabpage: Tabpage,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    #[cfg(test)]
    {
        let _ = (tabpage, name, arena, err);
        nil_obj()
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return nil_obj();
        }
        dict_get_value((*tab).tp_vars, name, arena, err)
    }
}

/// Sets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_var(Tabpage tabpage, String name, Object value,
//                                     Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    #[cfg(test)]
    let _ = (tabpage, name, value, err);

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars,
            name,
            value,
            false,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Removes a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_del_var(Tabpage tabpage, String name, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_del_var(tabpage: Tabpage, name: NvimString, err: *mut Error) {
    #[cfg(test)]
    let _ = (tabpage, name, err);

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars,
            name,
            nil_obj(),
            true,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Gets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return |window-ID|
// DLLEXPORT Window nvim_tabpage_get_win(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_win(tabpage: Tabpage, err: *mut Error) -> Window {
    #[cfg(test)]
    {
        let _ = (tabpage, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() || !valid_tabpage(tab) {
            return 0;
        }
        if tab == curtab {
            return nvim_get_current_win();
        }
        let mut wp = (*tab).tp_firstwin;
        while !wp.is_null() {
            if wp == (*tab).tp_curwin {
                return (*wp).handle;
            }
            wp = (*wp).w_next;
        }
        std::process::abort();
    }
}

/// Sets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param win |window-ID|, must already belong to {tabpage}
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_win(Tabpage tabpage, Window win, Error *err) FUNC_API_SINCE(12);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_win(tabpage: Tabpage, win: Window, err: *mut Error) {
    #[cfg(test)]
    let _ = (tabpage, win, err);

    #[cfg(not(test))]
    unsafe {
        let tp = find_tab_by_handle(tabpage, err);
        if tp.is_null() {
            return;
        }
        let wp = find_window_by_handle(win, err);
        if wp.is_null() {
            return;
        }
        if !tabpage_win_valid(tp, wp) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Window does not belong to tabpage %d".as_ptr(),
                (*tp).handle,
            );
            return;
        }
        if tp == curtab {
            let mut tstate: TryState = std::mem::zeroed();
            try_enter(&mut tstate);
            win_goto(wp);
            try_leave(&tstate, err);
        } else if (*tp).tp_curwin != wp {
            (*tp).tp_prevwin = (*tp).tp_curwin;
            (*tp).tp_curwin = wp;
        }
    }
}

/// Gets the tabpage number
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return Tabpage number
// DLLEXPORT Integer nvim_tabpage_get_number(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_number(tabpage: Tabpage, err: *mut Error) -> Integer {
    #[cfg(test)]
    {
        let _ = (tabpage, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return 0;
        }
        i64::from(tabpage_index(tab))
    }
}

/// Checks if a tabpage is valid
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @return true if the tabpage is valid, false otherwise
// DLLEXPORT Boolean nvim_tabpage_is_valid(Tabpage tabpage) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_is_valid(tabpage: Tabpage) -> Boolean {
    #[cfg(test)]
    {
        let _ = tabpage;
        false
    }

    #[cfg(not(test))]
    unsafe {
        let mut stub = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: std::ptr::null_mut(),
        };
        let ret = !find_tab_by_handle(tabpage, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
}

/// Opens a new tabpage.
///
/// @param buf Buffer to open in the first window of the new tabpage.
///               Use 0 for current buffer.
/// @param enter  Enter the tabpage (make it the current tabpage).
/// @param config Configuration for the new tabpage. Keys:
///   - after: Position to insert tabpage (default: -1; after current).
///            0 = first, N = after Nth.
/// @param[out] err Error details, if any
/// @return |tab-ID| of the new tabpage
// DLLEXPORT Tabpage nvim_open_tabpage(Buffer buf, Boolean enter, Dict(tabpage_config) *config,
//                                     Error *err) FUNC_API_SINCE(14) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_open_tabpage(
    buf: Buffer,
    enter: Boolean,
    config: *mut KeyDict_tabpage_config,
    err: *mut Error,
) -> Tabpage {
    #[cfg(test)]
    {
        let _ = (buf, enter, config, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return 0;
        }

        let after: c_int = if has_key(
            (*config).is_set__tabpage_config_,
            KEYSET_OPTIDX_tabpage_config__after,
        ) {
            (*config).after as c_int
        } else {
            -1
        };

        let mut wp: *mut win_T = std::ptr::null_mut();
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        let tp = win_new_tabpage(after + 1, std::ptr::null_mut(), enter, &mut wp);
        try_leave(&tstate, err);

        if tp.is_null() {
            if !error_set(err) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to create new tabpage".as_ptr(),
                );
            }
            return 0;
        }
        if !valid_tabpage(tp) {
            api_clear_error(err);
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Tabpage was closed immediately".as_ptr(),
            );
            return 0;
        }

        if tabpage_win_valid(tp, wp) && (*wp).w_buffer != b {
            let au_no_enter_leave = curwin != wp;
            if au_no_enter_leave {
                autocmd_no_enter += 1;
                autocmd_no_leave += 1;
            }
            win_set_buf(wp, b, err);
            if au_no_enter_leave {
                autocmd_no_enter -= 1;
                autocmd_no_leave -= 1;
            }
            if !valid_tabpage(tp) {
                api_clear_error(err);
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Tabpage was closed immediately".as_ptr(),
                );
                return 0;
            }
        }

        (*tp).handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::validate::ObjectType;

    #[test]
    fn window_obj_creates_window_object() {
        let o = window_obj(1002);
        assert_eq!(o.obj_type, ObjectType::kObjectTypeWindow);
        assert_eq!(unsafe { o.data.integer }, 1002);
    }
}
