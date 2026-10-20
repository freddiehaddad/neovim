//! Code for updating all the windows on the screen (`src/nvim/drawscreen.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::buffer::buf_meta_total;
#[allow(unused_imports)]
use crate::buffer::maketitle;
#[allow(unused_imports)]
use crate::cmdexpand::cmdline_pum_display;
#[allow(unused_imports)]
use crate::diff::diff_redraw;
#[allow(unused_imports)]
use crate::drawline::win_line;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_nr;
#[allow(unused_imports)]
use crate::ex_getln::{cmdline_screen_cleared, compute_cmdrow, get_cmdline_info, redrawcmdline};
#[allow(unused_imports)]
use crate::fold::{fold_info, hasFolding};
#[allow(unused_imports)]
use crate::fold::{foldmethodIsSyntax, hasAnyFolding};
use crate::globals::*;
#[allow(unused_imports)]
use crate::grid::schar_from_ascii;
#[allow(unused_imports)]
use crate::highlight::win_hl_attr;
#[allow(unused_imports)]
use crate::input::char_avail;
#[allow(unused_imports)]
use crate::insexpand::ins_compl_show_pum;
#[allow(unused_imports)]
use crate::memline::{ml_get_buf, ml_get_buf_len};
#[allow(unused_imports)]
use crate::message::{
    msg_check_for_delay, msg_clr_cmdline, msg_clr_eos, msg_ext_flush_showmode, msg_ext_ui_flush,
    msg_grid_set_pos, msg_grid_validate, msg_puts_hl, msg_reset_scroll, msg_scrollsize,
    msg_use_grid, repeat_message,
};
#[allow(unused_imports)]
use crate::normal::{clear_showcmd, do_check_scrollbind};
#[allow(unused_imports)]
use crate::option::{get_ve_flags, shortmess};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::regexp::{re_multiline, vim_regfree};
#[allow(unused_imports)]
use crate::search::last_pat_prog;
#[allow(unused_imports)]
use crate::spell::spell_check_window;
#[allow(unused_imports)]
use crate::strings::gettext;
#[allow(unused_imports)]
use crate::strings::vim_strchr;
#[allow(unused_imports)]
use crate::syntax::{
    syn_set_timeout, syn_stack_apply_changes, syntax_check_changed, syntax_end_parsing,
    syntax_present,
};
#[allow(unused_imports)]
use crate::ui::ui_call_win_extmark;
#[allow(unused_imports)]
use crate::ui::{
    ui_call_grid_clear, ui_call_grid_resize, ui_call_msg_clear, ui_flush, ui_grid_cursor_goto,
    ui_has,
};
#[allow(unused_imports)]
use crate::version::{intro_message, may_show_intro};
#[allow(unused_imports)]
use crate::window::{
    frame2win, global_stl_height, last_stl_height, min_rows, min_rows_for_all_tabpages,
    win_fdccol_count, win_new_screensize, win_ui_flush,
};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use crate::api::autocmd::kCallbackNone;
use crate::api::deprecated::{DecorVirtText, DecorVirtTextData, VirtText, VirtTextChunk};
use crate::api::extmark::NS;
use crate::api::tabpage::{frame_T, win_T};
use crate::autocmd::{EVENT_VIMRESIZED, apply_autocmds};
use crate::charset::{vim_isprintc, vim_strsize};
use crate::context::ctx_saved_curwin;
use crate::cursor::{colnr_T, linenr_T};
use crate::decoration::{
    buf_signcols_count_range, decor_conceal_line, decor_range_add_virt, decor_redraw_reset,
    decor_virt_lines, win_lines_concealed,
};
use crate::decoration_provider::{
    decor_providers_invoke_buf, decor_providers_invoke_end, decor_providers_invoke_win,
    decor_providers_start,
};
use crate::digraph::get_keymap_str;
use crate::eval::deprecated::{buf_T, pos_T};
use crate::extmark::TriState;
use crate::grid::{
    UIExtension, grid_adjust, grid_alloc, grid_clear, grid_clear_line, grid_del_lines,
    grid_draw_border, grid_ins_lines, grid_invalidate, grid_line_clear_end, grid_line_fill,
    grid_line_flush, grid_line_getchar, grid_line_mirror, grid_line_put_schar, grid_line_start,
    schar_T, schar_cache_clear_if_full, win_grid_alloc, wline_T,
};
use crate::highlight::{hl_combine_attr, update_window_hl, win_bg_attr, win_check_ns_hl};
use crate::highlight_group::highlight_changed;
use crate::marktree::{DECOR_PRIORITY_BASE, kMTMetaSignText};
use crate::r#match::{init_search_hl, matchitem_T, prepare_search_hl, varnumber_T};
use crate::mbyte::{utf_ptr2cells, utf_ptr2char};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, changed_window_setting, curs_columns,
    invalidate_botline_win, plines_correct_topline, set_empty_rows, update_curswant,
    update_topline, validate_cursor, validate_virtcol, win_col_off, win_col_off2,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::plines::{getvcols, getvvcol, plines_m_win, plines_win, win_get_fill, win_may_fill};
use crate::popupmenu::{pum_check_clear, pum_drawn, pum_invalidate, pum_redraw};
use crate::profile::profile_setlimit;
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP,
    MODE_NORMAL, MODE_SETWSIZE, MODE_TERMINAL, MODE_VISUAL, REPLACE_FLAG, VREPLACE_FLAG,
    get_real_state,
};
use crate::statusline::{
    draw_tabline, foldinfo_T, hlf_T, redraw_ruler, stl_alloc_click_defs, stl_clear_click_defs,
    win_redr_status, win_redr_winbar,
};
use crate::terminal::{Terminal, terminal_check_size, terminal_suspended};
use crate::ui_compositor::ui_comp_set_screen_valid;

pub use crate::ex_getln::disptick_T;
pub use crate::option::OptInt;

const NUL: c_char = 0;
const TAB: c_int = b'\t' as c_int;
const Ctrl_V: c_int = 22;
const FAIL: c_int = 0;
const OK: c_int = 1;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXCOL: colnr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;

pub const UPD_VALID: c_int = 10;
pub const UPD_INVERTED: c_int = 20;
pub const UPD_INVERTED_ALL: c_int = 25;
pub const UPD_REDRAW_TOP: c_int = 30;
pub const UPD_SOME_VALID: c_int = 35;
pub const UPD_NOT_VALID: c_int = 40;
pub const UPD_CLEAR: c_int = 50;

const DEFAULT_GRID_HANDLE: handle_T = 1;
const NO_SCREEN: c_int = 2;
const MIN_COLUMNS: c_int = 12;
const COL_RULER: c_int = 18;
const SHOWCMD_COLS: c_int = 10;
const SIGN_WIDTH: c_int = 2;
const SCL_NUM: c_int = -2;

const STL_IN_ICON: c_int = 1;
const STL_IN_TITLE: c_int = 2;

const FR_LEAF: c_char = 0;
const FR_ROW: c_char = 1;
const FR_COL: c_char = 2;

const VALID_WCOL: c_int = 0x02;
const VALID_BOTLINE: c_int = 0x20;
const VALID_TOPLINE: c_int = 0x80;

const GETVCOL_END_EXCL_LBR: c_int = 1;
const kVPosWinCol: c_int = 5;

const kUICmdline: UIExtension = 0;
const kUIMessages: UIExtension = 4;
const kUIMultigrid: UIExtension = 6;

const HLF_EOB: hlf_T = 2;
const HLF_AT: hlf_T = 4;
const HLF_CM: c_int = 11;
const HLF_N: c_int = 12;
const HLF_C: c_int = 21;
const HLF_FC: c_int = 29;
const HLF_SC: c_int = 35;
const HLF_MSG: usize = 63;
const HLF_COUNT: c_int = 76;

const kShmCompletionmenu: c_int = b'c' as c_int;
const kShmRecording: c_int = b'q' as c_int;
const kCpoNumcol: c_int = b'n' as c_int;
const kOptVeFlagBlock: c_uint = 0x05;
const kOptDyFlagLastline: c_uint = 0x01;
const kOptDyFlagTruncate: c_uint = 0x02;

const VV_ECHOSPACE: c_int = 87;

const kFalse: TriState = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowCorner {
    TopLeft = 0,
    TopRight = 1,
    BottomLeft = 2,
    BottomRight = 3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DidUpdate {
    None = 1,
    Line = 2,
    Fold = 3,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WinExtmark {
    pub ns_id: NS,
    pub mark_id: u64,
    pub win_row: c_int,
    pub win_col: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct spellvars_T {
    pub spv_has_spell: bool,
    pub spv_unchanged: bool,
    pub spv_checked_col: c_int,
    pub spv_checked_lnum: linenr_T,
    pub spv_cap_col: c_int,
    pub spv_capcol_lnum: linenr_T,
}

static mut redraw_popupmenu: bool = false;
static mut msg_grid_invalid: bool = false;
static mut resizing_autocmd: bool = false;
static mut conceal_cursor_used: bool = false;
static mut default_grid_resizing: bool = false;
static mut still_may_intro: bool = true;
static mut win_update_recursive: bool = false;

static mut suspended_chunk: VirtTextChunk = VirtTextChunk {
    text: ptr::null_mut(),
    hl_id: -1,
};
static mut suspended_virt_text: DecorVirtText = DecorVirtText {
    flags: 0,
    hl_mode: 0,
    priority: DECOR_PRIORITY_BASE,
    width: 0,
    col: 0,
    pos: kVPosWinCol,
    data: DecorVirtTextData {
        virt_text: VirtText {
            size: 1,
            capacity: 0,
            items: ptr::null_mut(),
        },
    },
    next: ptr::null_mut(),
};

#[inline]
unsafe fn w_endcol(wp: *const win_T) -> c_int {
    unsafe { (*wp).w_wincol + (*wp).w_width }
}

#[inline]
unsafe fn w_endrow(wp: *const win_T) -> c_int {
    unsafe { (*wp).w_winrow + (*wp).w_height }
}

#[inline]
unsafe fn w_lines_at(wp: *mut win_T, idx: usize) -> *mut wline_T {
    unsafe { (*wp).w_lines.cast::<wline_T>().add(idx) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn conceal_check_cursor_line() {
    unsafe {
        let should_conceal = conceal_cursor_line(curwin);
        if (*curwin).w_onebuf_opt.wo_cole <= 0 || conceal_cursor_used == should_conceal {
            return;
        }

        redrawWinline(curwin, (*curwin).w_cursor.lnum);

        if decor_conceal_line(curwin, (*curwin).w_cursor.lnum - 1, true) {
            changed_window_setting(curwin);
        }
        curs_columns(curwin, 1);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn default_grid_alloc() -> bool {
    unsafe {
        if default_grid_resizing {
            return false;
        }
        default_grid_resizing = true;

        if (!default_grid.chars.is_null()
            && Rows == default_grid.rows
            && Columns == default_grid.cols)
            || Rows == 0
            || Columns == 0
        {
            default_grid_resizing = false;
            return false;
        }

        grid_alloc(&raw mut default_grid, Rows, Columns, true, true);

        stl_clear_click_defs(tab_page_click_defs, tab_page_click_defs_size);
        tab_page_click_defs = stl_alloc_click_defs(
            tab_page_click_defs,
            Columns,
            &raw mut tab_page_click_defs_size,
        );

        default_grid.comp_height = Rows;
        default_grid.comp_width = Columns;
        default_grid.handle = DEFAULT_GRID_HANDLE;

        default_grid_resizing = false;
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn screenclear() {
    unsafe {
        msg_check_for_delay(false);

        if starting == NO_SCREEN || default_grid.chars.is_null() {
            return;
        }

        for i in 0..default_grid.rows {
            grid_clear_line(
                &raw mut default_grid,
                *default_grid.line_offset.add(i as usize),
                default_grid.cols,
                true,
            );
        }

        ui_call_grid_clear(1);
        ui_comp_set_screen_valid(true);

        ns_hl_fast = -1;

        clear_cmdline = false;
        mode_displayed = false;

        redraw_all_later(UPD_NOT_VALID);
        cmdline_was_last_drawn = false;
        redraw_cmdline = true;
        redraw_tabline = true;
        redraw_popupmenu = true;
        pum_invalidate();

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_floating {
                (*wp).w_redr_type = UPD_CLEAR;
            }
            wp = (*wp).w_next;
        }

        if must_redraw == UPD_CLEAR {
            must_redraw = UPD_NOT_VALID;
        }
        compute_cmdrow();
        msg_row = cmdline_row;
        msg_col = 0;
        msg_reset_scroll();
        msg_didany = false;
        msg_didout = false;
        if *hl_attr_active.add(HLF_MSG) > 0 && msg_use_grid() && !msg_grid.chars.is_null() {
            grid_invalidate(&raw mut msg_grid);
            msg_grid_validate();
            msg_grid_invalid = false;
            clear_cmdline = true;
        }
    }
}

unsafe fn cmdline_number_prompt() -> bool {
    unsafe {
        !ui_has(kUIMessages)
            && (State & MODE_CMDLINE) != 0
            && !(*get_cmdline_info()).mouse_used.is_null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn screen_resize(mut width: c_int, mut height: c_int) {
    unsafe {
        if updating_screen || resizing_screen || cmdline_number_prompt() {
            return;
        }

        if width < 0 || height < 0 {
            return;
        }

        if State == MODE_HITRETURN || State == MODE_SETWSIZE {
            State = MODE_SETWSIZE;
            return;
        }

        resizing_screen = true;

        Rows = height;
        Columns = width;
        check_screensize();
        if !ui_has(kUIMessages) {
            let max_p_ch = OptInt::from(Rows - min_rows(curtab) + 1);
            if p_ch > 0 && p_ch > max_p_ch {
                p_ch = max_p_ch.max(1);
                (*curtab).tp_ch_used = p_ch;
            }
            let mut tp = first_tabpage;
            while !tp.is_null() {
                if !ptr::eq(tp, curtab) {
                    let max_tp_ch = OptInt::from(Rows - min_rows(tp) + 1);
                    if (*tp).tp_ch_used > 0 && (*tp).tp_ch_used > max_tp_ch {
                        (*tp).tp_ch_used = max_tp_ch.max(1);
                    }
                }
                tp = (*tp).tp_next;
            }
        }
        height = Rows;
        width = Columns;
        p_lines = OptInt::from(Rows);
        p_columns = OptInt::from(Columns);

        ui_call_grid_resize(1, Integer::from(width), Integer::from(height));

        let mut retry_count = 0;
        resizing_autocmd = true;

        while default_grid_alloc() {
            ui_comp_set_screen_valid(false);
            if !msg_grid.chars.is_null() {
                msg_grid_invalid = true;
            }

            RedrawingDisabled += 1;

            win_new_screensize();
            comp_col();

            RedrawingDisabled -= 1;

            retry_count += 1;
            if retry_count > 3 {
                break;
            }

            apply_autocmds(
                EVENT_VIMRESIZED,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
        }

        resizing_autocmd = false;
        redraw_all_later(UPD_CLEAR);

        if State != MODE_ASKMORE && State != MODE_EXTERNCMD {
            screenclear();
        }

        if starting != NO_SCREEN {
            maketitle();

            changed_line_abv_curs();
            invalidate_botline_win(curwin);

            if State == MODE_ASKMORE
                || State == MODE_EXTERNCMD
                || ((State & MODE_CMDLINE) != 0 && (*get_cmdline_info()).one_key)
            {
                if (State & MODE_CMDLINE) != 0 {
                    update_screen();
                }
                if !msg_grid.chars.is_null() {
                    msg_grid_validate();
                }
                ui_comp_set_screen_valid(true);
                repeat_message();
            } else {
                if (*curwin).w_onebuf_opt.wo_scb != 0 {
                    do_check_scrollbind(true);
                }
                if (State & MODE_CMDLINE) != 0 {
                    redraw_popupmenu = false;
                    update_screen();
                    redrawcmdline();
                    if pum_drawn() {
                        cmdline_pum_display(false);
                    }
                } else {
                    update_topline(curwin);
                    if pum_drawn() {
                        redraw_popupmenu = false;
                        ins_compl_show_pum();
                    }
                    update_screen();
                    if redrawing() {
                        setcursor();
                    }
                }
            }
            if State != MODE_ASKMORE {
                ui_flush();
            }
        }
        resizing_screen = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_screensize() {
    unsafe {
        Rows = Rows.max(min_rows_for_all_tabpages()).min(1000);
        Columns = Columns.clamp(MIN_COLUMNS, 10000);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redrawing() -> bool {
    unsafe { RedrawingDisabled == 0 && !(p_lz != 0 && char_avail() && !KeyTyped && !do_redraw) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_screen() -> c_int {
    unsafe {
        if still_may_intro && !may_show_intro() {
            redraw_later(firstwin, UPD_NOT_VALID);
            still_may_intro = false;
        }

        let is_stl_global = global_stl_height() > 0;

        if resizing_autocmd || default_grid.chars.is_null() {
            return FAIL;
        }

        if need_diff_redraw {
            diff_redraw(true);
        }

        if !redrawing() || updating_screen || cmdline_number_prompt() {
            return FAIL;
        }

        let save_curwin = ctx_saved_curwin();
        let restore_curwin = if !save_curwin.is_null() {
            curwin
        } else {
            ptr::null_mut()
        };
        if !save_curwin.is_null() {
            curwin = save_curwin;
            curbuf = (*curwin).w_buffer;
        }

        let mut type_ = must_redraw;
        must_redraw = 0;

        updating_screen = true;
        display_tick += 1;

        if schar_cache_clear_if_full() {
            type_ = type_.max(UPD_CLEAR);
        }

        if msg_did_scroll {
            msg_did_scroll = false;
            msg_scrolled_at_flush = 0;
        }

        if type_ >= UPD_CLEAR || !default_grid.valid {
            ui_comp_set_screen_valid(false);
        }

        if msg_scrolled != 0 || msg_grid_invalid {
            clear_cmdline = true;
            let valid = (Rows - msg_scrollsize()).max(0);
            if !msg_grid.chars.is_null() {
                let limit = msg_scrollsize().min(msg_grid.rows);
                for i in 0..limit {
                    grid_clear_line(
                        &raw mut msg_grid,
                        *msg_grid.line_offset.add(i as usize),
                        msg_grid.cols,
                        OptInt::from(i) < p_ch,
                    );
                }
            }
            msg_grid.throttled = false;
            let mut was_invalidated = false;

            if type_ == UPD_NOT_VALID && !ui_has(kUIMultigrid) && msg_scrolled != 0 {
                was_invalidated = ui_comp_set_screen_valid(false);
                let mut i = valid;
                while i < Rows - (p_ch as c_int) {
                    grid_clear_line(
                        &raw mut default_grid,
                        *default_grid.line_offset.add(i as usize),
                        Columns,
                        false,
                    );
                    i += 1;
                }
                let mut wp = firstwin;
                while !wp.is_null() {
                    if !(*wp).w_floating {
                        if w_endrow(wp) > valid {
                            (*wp).w_redr_type = (*wp).w_redr_type.max(UPD_NOT_VALID);
                        }
                        if !is_stl_global && w_endrow(wp) + (*wp).w_status_height > valid {
                            (*wp).w_redr_status = true;
                        }
                    }
                    wp = (*wp).w_next;
                }
                if is_stl_global && Rows - (p_ch as c_int) - 1 > valid {
                    (*curwin).w_redr_status = true;
                }
            }
            msg_grid_set_pos(Rows - (p_ch as c_int), false);
            msg_grid_invalid = false;
            if was_invalidated {
                ui_comp_set_screen_valid(true);
            }
            msg_scrolled = 0;
            msg_scrolled_at_flush = 0;
            msg_grid_scroll_discount = 0;
            need_wait_return = false;
        }

        win_ui_flush(true);
        compute_cmdrow();

        let mut hl_changed = false;
        if need_highlight_changed {
            highlight_changed();
            hl_changed = true;
        }

        if type_ == UPD_CLEAR {
            screenclear();
            cmdline_screen_cleared();
            if ui_has(kUIMessages) {
                ui_call_msg_clear();
            }
            type_ = UPD_NOT_VALID;
            must_redraw = 0;
        } else if !default_grid.valid {
            grid_invalidate(&raw mut default_grid);
            default_grid.valid = true;
        }

        if type_ == UPD_NOT_VALID && clear_cmdline && !ui_has(kUIMessages) {
            grid_clear(
                &raw mut default_gridview,
                Rows - (p_ch as c_int),
                Rows,
                0,
                Columns,
                0,
            );
        }

        ui_comp_set_screen_valid(true);

        decor_providers_start();

        if win_check_ns_hl(ptr::null_mut()) {
            redraw_cmdline = true;
            redraw_tabline = true;
        }

        if clear_cmdline {
            msg_check_for_delay(false);
        }

        if (*curwin).w_redr_type < UPD_NOT_VALID {
            let curwin_nrw = if (*curwin).w_onebuf_opt.wo_nu != 0
                || (*curwin).w_onebuf_opt.wo_rnu != 0
                || *(*curwin).w_onebuf_opt.wo_stc != 0
            {
                number_width(curwin)
            } else {
                0
            };
            if (*curwin).w_nrwidth != curwin_nrw {
                (*curwin).w_redr_type = UPD_NOT_VALID;
            }
        }

        if (*curwin).w_redr_type == UPD_INVERTED {
            update_curswant();
        }

        if redraw_tabline || type_ >= UPD_NOT_VALID {
            update_window_hl(curwin, type_ >= UPD_NOT_VALID);
            let mut tp = first_tabpage;
            while !tp.is_null() {
                if !ptr::eq(tp, curtab) {
                    update_window_hl((*tp).tp_curwin, type_ >= UPD_NOT_VALID);
                }
                tp = (*tp).tp_next;
            }
            draw_tabline();
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            if !((*wp).w_config.hide && !(*wp).w_grid_alloc.chars.is_null()) {
                update_window_hl(wp, type_ >= UPD_NOT_VALID || hl_changed);

                let buf = (*wp).w_buffer;
                if (*buf).b_mod_set {
                    if (*buf).b_mod_tick_syn < display_tick && syntax_present(wp) {
                        syn_stack_apply_changes(buf);
                        (*buf).b_mod_tick_syn = display_tick;
                    }
                    if (*buf).b_mod_tick_decor < display_tick {
                        decor_providers_invoke_buf(buf);
                        (*buf).b_mod_tick_decor = display_tick;
                    }
                }
            }
            wp = (*wp).w_next;
        }

        let mut did_one = false;
        screen_search_hl.rm.regprog = ptr::null_mut();

        wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_config.hide && !(*wp).w_grid_alloc.chars.is_null() {
                if ptr::eq(wp, curwin) && global_stl_height() > 0 {
                    win_redr_status(wp);
                }
                (*wp).w_redr_type = 0;
                wp = (*wp).w_next;
                continue;
            }

            if (*wp).w_redr_type == UPD_CLEAR
                && (*wp).w_floating
                && !(*wp).w_grid_alloc.chars.is_null()
            {
                grid_invalidate(&raw mut (*wp).w_grid_alloc);
                (*wp).w_redr_type = UPD_NOT_VALID;
            }

            win_check_ns_hl(wp);
            win_grid_alloc(wp);

            if (*wp).w_redr_border || (*wp).w_redr_type >= UPD_NOT_VALID {
                grid_draw_border(
                    &raw mut (*wp).w_grid_alloc,
                    &raw mut (*wp).w_config,
                    (*wp).w_border_adj.as_mut_ptr(),
                    (*wp).w_onebuf_opt.wo_winbl as c_int,
                    (*wp).w_ns_hl_attr,
                    (*wp).w_hl_attr_normal,
                );
            }

            if (*wp).w_redr_type != 0 {
                if !did_one {
                    did_one = true;
                    start_search_hl();
                }
                win_update(wp);
            }

            if (*wp).w_redr_status {
                win_redr_winbar(wp);
                win_redr_status(wp);
            }

            wp = (*wp).w_next;
        }

        if did_one {
            wp = firstwin;
            while !wp.is_null() {
                draw_sep_connectors_win(wp);
                wp = (*wp).w_next;
            }
        }

        end_search_hl();

        if pum_drawn() && must_redraw_pum {
            win_check_ns_hl(curwin);
            pum_redraw();
        } else if (State & MODE_CMDLINE) != 0 {
            pum_check_clear();
        }

        win_check_ns_hl(ptr::null_mut());

        wp = firstwin;
        while !wp.is_null() {
            (*(*wp).w_buffer).b_mod_set = false;
            wp = (*wp).w_next;
        }

        updating_screen = false;

        if need_maketitle {
            maketitle();
        }

        if clear_cmdline || redraw_cmdline || redraw_mode {
            showmode();
        }

        if still_may_intro {
            intro_message(false);
        }
        repeat_message();

        decor_providers_invoke_end();

        if !ui_has(kUICmdline) {
            cmdline_was_last_drawn = false;
        }

        if !restore_curwin.is_null() {
            curwin = restore_curwin;
            curbuf = (*curwin).w_buffer;
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn start_search_hl() {
    unsafe {
        if p_hls == 0 || Search.no_hlsearch {
            return;
        }

        end_search_hl();
        last_pat_prog(&raw mut screen_search_hl.rm);
        screen_search_hl.tm = profile_setlimit(p_rdt);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn end_search_hl() {
    unsafe {
        if screen_search_hl.rm.regprog.is_null() {
            return;
        }

        vim_regfree(screen_search_hl.rm.regprog);
        screen_search_hl.rm.regprog = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setcursor() {
    unsafe {
        setcursor_mayforce(curwin, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn setcursor_mayforce(wp: *mut win_T, force: bool) {
    unsafe {
        if force || redrawing() {
            validate_cursor(wp);

            let mut row = (*wp).w_wrow;
            let mut col = (*wp).w_wcol;
            if (*wp).w_onebuf_opt.wo_rl != 0 {
                let cursor = ml_get_buf((*wp).w_buffer, (*wp).w_cursor.lnum)
                    .add((*wp).w_cursor.col as usize);
                let wide = utf_ptr2cells(cursor) == 2 && vim_isprintc(utf_ptr2char(cursor));
                col = (*wp).w_view_width - (*wp).w_wcol - if wide { 2 } else { 1 };
            }

            let grid = grid_adjust(&raw mut (*wp).w_grid, &raw mut row, &raw mut col);
            if !grid.is_null() {
                ui_grid_cursor_goto((*grid).handle, row, col);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_custom_title_later() -> bool {
    unsafe {
        if (p_icon != 0 && (stl_syntax & STL_IN_ICON) != 0)
            || (p_title != 0 && (stl_syntax & STL_IN_TITLE) != 0)
        {
            need_maketitle = true;
            return true;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn show_cursor_info_later(force: bool) {
    unsafe {
        let state = get_real_state();
        let empty_line = c_int::from(
            (State & MODE_INSERT) == 0
                && *ml_get_buf((*curwin).w_buffer, (*curwin).w_cursor.lnum) == NUL,
        );

        validate_virtcol(curwin);
        if force
            || (*curwin).w_cursor.lnum != (*curwin).w_stl_cursor.lnum
            || (*curwin).w_cursor.col != (*curwin).w_stl_cursor.col
            || (*curwin).w_virtcol != (*curwin).w_stl_virtcol
            || (*curwin).w_cursor.coladd != (*curwin).w_stl_cursor.coladd
            || (*curwin).w_topline != (*curwin).w_stl_topline
            || (*(*curwin).w_buffer).b_ml.ml_line_count != (*curwin).w_stl_line_count
            || (*curwin).w_topfill != (*curwin).w_stl_topfill
            || (empty_line as c_char) != (*curwin).w_stl_empty
            || reg_recording != (*curwin).w_stl_recording
            || state != (*curwin).w_stl_state
            || (Visual.active
                && (Visual.mode != (*curwin).w_stl_visual_mode
                    || Visual.start.lnum != (*curwin).w_stl_visual_pos.lnum
                    || Visual.start.col != (*curwin).w_stl_visual_pos.col
                    || Visual.start.coladd != (*curwin).w_stl_visual_pos.coladd))
        {
            if (*curwin).w_status_height != 0 || global_stl_height() != 0 {
                (*curwin).w_redr_status = true;
            } else {
                redraw_cmdline = true;
            }

            if *p_wbr != NUL || *(*curwin).w_onebuf_opt.wo_wbr != NUL {
                (*curwin).w_redr_status = true;
            }

            redraw_custom_title_later();
        }

        (*curwin).w_stl_cursor = (*curwin).w_cursor;
        (*curwin).w_stl_virtcol = (*curwin).w_virtcol;
        (*curwin).w_stl_empty = empty_line as c_char;
        (*curwin).w_stl_topline = (*curwin).w_topline;
        (*curwin).w_stl_line_count = (*(*curwin).w_buffer).b_ml.ml_line_count;
        (*curwin).w_stl_topfill = (*curwin).w_topfill;
        (*curwin).w_stl_recording = reg_recording;
        (*curwin).w_stl_state = state;
        if Visual.active {
            (*curwin).w_stl_visual_mode = Visual.mode;
            (*curwin).w_stl_visual_pos = Visual.start;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn skip_showmode() -> bool {
    unsafe {
        if global_busy != 0 || msg_silent != 0 || !redrawing() || (char_avail() && !KeyTyped) {
            redraw_mode = true;
            return true;
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn showmode() -> c_int {
    unsafe {
        let mut length: c_int = 0;

        msg_ext_ui_flush();
        msg_grid_validate();

        let do_mode = (p_smd != 0 && msg_silent == 0)
            && ((State & MODE_TERMINAL) != 0
                || (State & MODE_INSERT) != 0
                || restart_edit != 0
                || Visual.active);

        let can_show_mode = p_ch != 0 || ui_has(kUIMessages);
        if (do_mode || reg_recording != 0) && can_show_mode {
            if skip_showmode() {
                return 0;
            }

            let nwr_save = need_wait_return;
            msg_check_for_delay(false);

            let mut need_clear = clear_cmdline;
            if clear_cmdline && cmdline_row < Rows - 1 {
                msg_clr_cmdline();
            }

            msg_pos_mode();
            let hl_id = HLF_CM;

            msg_no_more = true;
            let save_lines_left = lines_left;
            lines_left = 0;

            if do_mode {
                msg_puts_hl(c"--".as_ptr(), hl_id, false);
                if !edit_submode.is_null() && !shortmess(kShmCompletionmenu) {
                    length = if ui_has(kUIMessages) {
                        c_int::MAX
                    } else {
                        (Rows - msg_row) * Columns - 3
                    };
                    if !edit_submode_extra.is_null() {
                        length -= vim_strsize(edit_submode_extra);
                    }
                    if length > 0 {
                        if !edit_submode_pre.is_null() {
                            length -= vim_strsize(edit_submode_pre);
                        }
                        if length - vim_strsize(edit_submode) > 0 {
                            if !edit_submode_pre.is_null() {
                                msg_puts_hl(edit_submode_pre, hl_id, false);
                            }
                            msg_puts_hl(edit_submode, hl_id, false);
                        }
                        if !edit_submode_extra.is_null() {
                            msg_puts_hl(c" ".as_ptr(), hl_id, false);
                            let sub_id = if edit_submode_highl < HLF_COUNT {
                                edit_submode_highl
                            } else {
                                hl_id
                            };
                            msg_puts_hl(edit_submode_extra, sub_id, false);
                        }
                    }
                } else {
                    if (State & MODE_TERMINAL) != 0 {
                        msg_puts_hl(gettext(c" TERMINAL".as_ptr()), hl_id, false);
                    } else if (State & VREPLACE_FLAG) != 0 {
                        msg_puts_hl(gettext(c" VREPLACE".as_ptr()), hl_id, false);
                    } else if (State & REPLACE_FLAG) != 0 {
                        msg_puts_hl(gettext(c" REPLACE".as_ptr()), hl_id, false);
                    } else if (State & MODE_INSERT) != 0 {
                        if p_ri != 0 {
                            msg_puts_hl(gettext(c" REVERSE".as_ptr()), hl_id, false);
                        }
                        msg_puts_hl(gettext(c" INSERT".as_ptr()), hl_id, false);
                    } else if restart_edit == c_int::from(b'I')
                        || restart_edit == c_int::from(b'i')
                        || restart_edit == c_int::from(b'a')
                        || restart_edit == c_int::from(b'A')
                    {
                        if !(*curbuf).terminal.is_null() {
                            msg_puts_hl(gettext(c" (terminal)".as_ptr()), hl_id, false);
                        } else {
                            msg_puts_hl(gettext(c" (insert)".as_ptr()), hl_id, false);
                        }
                    } else if restart_edit == c_int::from(b'R') {
                        msg_puts_hl(gettext(c" (replace)".as_ptr()), hl_id, false);
                    } else if restart_edit == c_int::from(b'V') {
                        msg_puts_hl(gettext(c" (vreplace)".as_ptr()), hl_id, false);
                    }
                    if (State & MODE_LANGMAP) != 0 {
                        if (*curwin).w_onebuf_opt.wo_arab != 0 {
                            msg_puts_hl(gettext(c" Arabic".as_ptr()), hl_id, false);
                        } else if get_keymap_str(
                            curwin,
                            c" (%s)".as_ptr().cast_mut(),
                            (&raw mut NameBuff).cast::<c_char>(),
                            MAXPATHL as c_int,
                        ) > 0
                        {
                            msg_puts_hl((&raw const NameBuff).cast::<c_char>(), hl_id, false);
                        }
                    }
                    if (State & MODE_INSERT) != 0 && p_paste != 0 {
                        msg_puts_hl(gettext(c" (paste)".as_ptr()), hl_id, false);
                    }

                    if Visual.active {
                        let code = (if Visual.select { 4 } else { 0 })
                            + (if Visual.mode == Ctrl_V { 2 } else { 0 })
                            + (if Visual.mode == c_int::from(b'V') {
                                1
                            } else {
                                0
                            });
                        let p: &CStr = match code {
                            0 => c" VISUAL",
                            1 => c" VISUAL LINE",
                            2 => c" VISUAL BLOCK",
                            4 => c" SELECT",
                            5 => c" SELECT LINE",
                            _ => c" SELECT BLOCK",
                        };
                        msg_puts_hl(gettext(p.as_ptr()), hl_id, false);
                    }
                    msg_puts_hl(c" --".as_ptr(), hl_id, false);
                }

                need_clear = true;
            }
            if reg_recording != 0 && edit_submode.is_null() {
                recording_mode(hl_id);
                need_clear = true;
            }

            mode_displayed = true;
            if need_clear || clear_cmdline || redraw_mode {
                msg_clr_eos();
            }
            msg_didout = false;
            length = msg_col;
            msg_col = 0;
            msg_no_more = false;
            lines_left = save_lines_left;
            need_wait_return = nwr_save;
        } else if clear_cmdline && msg_silent == 0 {
            msg_clr_cmdline();
        } else if redraw_mode {
            msg_pos_mode();
            msg_clr_eos();
        }

        msg_ext_flush_showmode();

        if Visual.active {
            clear_showcmd();
        }

        redraw_ruler();
        redraw_cmdline = false;
        redraw_mode = false;
        clear_cmdline = false;

        length
    }
}

unsafe fn msg_pos_mode() {
    unsafe {
        msg_col = 0;
        msg_row = Rows - 1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn unshowmode(force: bool) {
    unsafe {
        if !redrawing() || (!force && char_avail() && !KeyTyped) {
            redraw_cmdline = true;
        } else {
            clearmode();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clearmode() {
    unsafe {
        let save_msg_row = msg_row;
        let save_msg_col = msg_col;

        msg_ext_ui_flush();
        msg_pos_mode();
        if reg_recording != 0 {
            recording_mode(HLF_CM);
        }
        msg_clr_eos();
        msg_ext_flush_showmode();

        msg_col = save_msg_col;
        msg_row = save_msg_row;
    }
}

unsafe fn recording_mode(hl_id: c_int) {
    unsafe {
        if shortmess(kShmRecording) {
            return;
        }

        msg_puts_hl(gettext(c"recording".as_ptr()), hl_id, false);
        let s: [c_char; 4] = [
            b' ' as c_char,
            b'@' as c_char,
            reg_recording as u8 as c_char,
            0,
        ];
        msg_puts_hl(s.as_ptr(), hl_id, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn comp_col() {
    unsafe {
        let last_has_status = last_stl_height(false) > 0;

        sc_col = 0;
        ru_col = 0;
        if p_ru != 0 {
            ru_col = if ru_wid != 0 { ru_wid } else { COL_RULER };
            if !last_has_status {
                sc_col = ru_col;
            }
        }
        if p_sc != 0 && *p_sloc == b'l' as c_char {
            sc_col += SHOWCMD_COLS;
            if p_ru == 0 || last_has_status {
                sc_col += 1;
            }
        }
        assert!(sc_col >= 0 && c_int::MIN + sc_col <= Columns);
        sc_col = Columns - sc_col;
        assert!(ru_col >= 0 && c_int::MIN + ru_col <= Columns);
        ru_col = Columns - ru_col;
        if sc_col <= 0 {
            sc_col = 1;
        }
        if ru_col <= 0 {
            ru_col = 1;
        }
        set_vim_var_nr(VV_ECHOSPACE, varnumber_T::from(sc_col - 1));
    }
}

unsafe fn win_redraw_signcols(wp: *mut win_T) -> bool {
    unsafe {
        let buf = (*wp).w_buffer;

        if !(*buf).b_signcols.autom
            && (*(*wp).w_onebuf_opt.wo_stc != NUL
                || ((*wp).w_maxscwidth > 1 && (*wp).w_minscwidth != (*wp).w_maxscwidth))
        {
            (*buf).b_signcols.autom = true;
            buf_signcols_count_range(buf, 0, (*buf).b_ml.ml_line_count - 1, MAXLNUM, kFalse);
        }

        while (*buf).b_signcols.max > 0
            && (*buf).b_signcols.count[((*buf).b_signcols.max - 1) as usize] == 0
        {
            (*buf).b_signcols.max -= 1;
        }

        let mut width = (*wp).w_maxscwidth.min((*buf).b_signcols.max);
        let rebuild_stc = (*buf).b_signcols.max != (*buf).b_signcols.last_max
            && *(*wp).w_onebuf_opt.wo_stc != NUL;

        if rebuild_stc {
            (*wp).w_nrwidth_line_count = 0;
        } else if (*wp).w_minscwidth == 0 && (*wp).w_maxscwidth == 1 {
            width = c_int::from(buf_meta_total(buf, kMTMetaSignText) > 0);
        }

        let scwidth = (*wp).w_scwidth;
        (*wp).w_scwidth = 0.max((*wp).w_minscwidth).max(width);
        (*wp).w_scwidth != scwidth || rebuild_stc
    }
}

unsafe fn hsep_connected(wp: *mut win_T, corner: WindowCorner) -> bool {
    unsafe {
        let before = corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft;
        let sep_row = if corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight {
            (*wp).w_winrow - 1
        } else {
            w_endrow(wp)
        };
        let mut fr = (*wp).w_frame;

        while !(*fr).fr_parent.is_null() {
            if (*(*fr).fr_parent).fr_layout == FR_ROW
                && !(if before { (*fr).fr_prev } else { (*fr).fr_next }).is_null()
            {
                fr = if before { (*fr).fr_prev } else { (*fr).fr_next };
                break;
            }
            fr = (*fr).fr_parent;
        }
        if (*fr).fr_parent.is_null() {
            return false;
        }
        while (*fr).fr_layout != FR_LEAF {
            fr = (*fr).fr_child;
            if (*(*fr).fr_parent).fr_layout == FR_ROW && before {
                while !(*fr).fr_next.is_null() {
                    fr = (*fr).fr_next;
                }
            } else {
                while !(*fr).fr_next.is_null()
                    && (*frame2win(fr)).w_winrow + (*fr).fr_height < sep_row
                {
                    fr = (*fr).fr_next;
                }
            }
        }

        sep_row == (*(*fr).fr_win).w_winrow - 1 || sep_row == w_endrow((*fr).fr_win)
    }
}

unsafe fn vsep_connected(wp: *mut win_T, corner: WindowCorner) -> bool {
    unsafe {
        let before = corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight;
        let sep_col = if corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft {
            (*wp).w_wincol - 1
        } else {
            w_endcol(wp)
        };
        let mut fr = (*wp).w_frame;

        while !(*fr).fr_parent.is_null() {
            if (*(*fr).fr_parent).fr_layout == FR_COL
                && !(if before { (*fr).fr_prev } else { (*fr).fr_next }).is_null()
            {
                fr = if before { (*fr).fr_prev } else { (*fr).fr_next };
                break;
            }
            fr = (*fr).fr_parent;
        }
        if (*fr).fr_parent.is_null() {
            return false;
        }
        while (*fr).fr_layout != FR_LEAF {
            fr = (*fr).fr_child;
            if (*(*fr).fr_parent).fr_layout == FR_COL && before {
                while !(*fr).fr_next.is_null() {
                    fr = (*fr).fr_next;
                }
            } else {
                while !(*fr).fr_next.is_null()
                    && (*frame2win(fr)).w_wincol + (*fr).fr_width < sep_col
                {
                    fr = (*fr).fr_next;
                }
            }
        }

        sep_col == (*(*fr).fr_win).w_wincol - 1 || sep_col == w_endcol((*fr).fr_win)
    }
}

unsafe fn draw_vsep_win(wp: *mut win_T) {
    unsafe {
        if (*wp).w_vsep_width == 0 {
            return;
        }

        for row in (*wp).w_winrow..w_endrow(wp) {
            grid_line_start(&raw mut default_gridview, row);
            grid_line_put_schar(
                w_endcol(wp),
                (*wp).w_p_fcs_chars.vert,
                win_hl_attr(wp, HLF_C),
            );
            grid_line_flush();
        }
    }
}

unsafe fn draw_hsep_win(wp: *mut win_T) {
    unsafe {
        if (*wp).w_hsep_height == 0 {
            return;
        }

        grid_line_start(&raw mut default_gridview, w_endrow(wp));
        grid_line_fill(
            (*wp).w_wincol,
            w_endcol(wp),
            (*wp).w_p_fcs_chars.horiz,
            win_hl_attr(wp, HLF_C),
        );
        grid_line_flush();
    }
}

unsafe fn get_corner_sep_connector(wp: *mut win_T, corner: WindowCorner) -> schar_T {
    unsafe {
        if vsep_connected(wp, corner) {
            if hsep_connected(wp, corner) {
                (*wp).w_p_fcs_chars.verthoriz
            } else if corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft {
                (*wp).w_p_fcs_chars.vertright
            } else {
                (*wp).w_p_fcs_chars.vertleft
            }
        } else if corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight {
            (*wp).w_p_fcs_chars.horizdown
        } else {
            (*wp).w_p_fcs_chars.horizup
        }
    }
}

unsafe fn draw_sep_connectors_win(wp: *mut win_T) {
    unsafe {
        if global_stl_height() == 0 || !((*wp).w_hsep_height == 1 || (*wp).w_vsep_width == 1) {
            return;
        }

        let hl = win_hl_attr(wp, HLF_C);

        let win_at_bottom = (*wp).w_hsep_height == 0;
        let win_at_right = (*wp).w_vsep_width == 0;

        let mut frp = (*wp).w_frame;
        while !(*frp).fr_parent.is_null() {
            if (*(*frp).fr_parent).fr_layout == FR_COL && !(*frp).fr_prev.is_null() {
                break;
            }
            frp = (*frp).fr_parent;
        }
        let win_at_top = (*frp).fr_parent.is_null();

        frp = (*wp).w_frame;
        while !(*frp).fr_parent.is_null() {
            if (*(*frp).fr_parent).fr_layout == FR_ROW && !(*frp).fr_prev.is_null() {
                break;
            }
            frp = (*frp).fr_parent;
        }
        let win_at_left = (*frp).fr_parent.is_null();

        let top_left = !(win_at_top || win_at_left);
        let top_right = !(win_at_top || win_at_right);
        let bot_left = !(win_at_bottom || win_at_left);
        let bot_right = !(win_at_bottom || win_at_right);

        if top_left {
            grid_line_start(&raw mut default_gridview, (*wp).w_winrow - 1);
            grid_line_put_schar(
                (*wp).w_wincol - 1,
                get_corner_sep_connector(wp, WindowCorner::TopLeft),
                hl,
            );
            grid_line_flush();
        }
        if top_right {
            grid_line_start(&raw mut default_gridview, (*wp).w_winrow - 1);
            grid_line_put_schar(
                w_endcol(wp),
                get_corner_sep_connector(wp, WindowCorner::TopRight),
                hl,
            );
            grid_line_flush();
        }
        if bot_left {
            grid_line_start(&raw mut default_gridview, w_endrow(wp));
            grid_line_put_schar(
                (*wp).w_wincol - 1,
                get_corner_sep_connector(wp, WindowCorner::BottomLeft),
                hl,
            );
            grid_line_flush();
        }
        if bot_right {
            grid_line_start(&raw mut default_gridview, w_endrow(wp));
            grid_line_put_schar(
                w_endcol(wp),
                get_corner_sep_connector(wp, WindowCorner::BottomRight),
                hl,
            );
            grid_line_flush();
        }
    }
}

unsafe fn win_update(wp: *mut win_T) {
    unsafe {
        if ptr::eq((*wp).w_grid.target, &raw mut default_grid) && (*wp).w_wincol >= Columns {
            return;
        }

        let mut top_end: c_int = 0;
        let mut mid_start: c_int = 999;
        let mut mid_end: c_int = 0;
        let mut bot_start: c_int = 999;
        let mut scrolled_down = false;
        let mut scrolled_for_mod = false;
        let mut top_to_mod = false;
        let mut bot_scroll_start: c_int = 999;
        let mut did_update = DidUpdate::None;

        let mut syntax_last_parsed: linenr_T = 0;
        let mut mod_top: linenr_T = 0;
        let mut mod_bot: linenr_T = 0;

        let mut type_ = (*wp).w_redr_type;

        if type_ >= UPD_NOT_VALID {
            (*wp).w_redr_status = true;
            (*wp).w_lines_valid = 0;
        }

        if (*wp).w_view_height == 0 {
            draw_vsep_win(wp);
            draw_hsep_win(wp);
            (*wp).w_redr_type = 0;
            return;
        }

        if (*wp).w_view_width == 0 {
            draw_vsep_win(wp);
            (*wp).w_redr_type = 0;
            return;
        }

        let buf = (*wp).w_buffer;

        let save_got_int = got_int;
        got_int = false;
        let mut syntax_tm = profile_setlimit(p_rdt);
        syn_set_timeout(&raw mut syntax_tm);

        win_extmark_arr.size = 0;

        decor_redraw_reset(wp, &raw mut decor_state);
        decor_providers_invoke_win(wp);

        if !(*buf).terminal.is_null() && terminal_suspended((*buf).terminal.cast::<Terminal>()) {
            suspended_chunk.text = c"[Process suspended]".as_ptr().cast_mut();
            suspended_chunk.hl_id = -1;
            suspended_virt_text.data.virt_text.items = &raw mut suspended_chunk;
            suspended_virt_text.data.virt_text.size = 1;
            decor_range_add_virt(
                &raw mut decor_state,
                (*buf).b_ml.ml_line_count - 1,
                0,
                (*buf).b_ml.ml_line_count - 1,
                0,
                &raw mut suspended_virt_text,
                false,
            );
        }

        let mut win = firstwin;
        while !win.is_null() {
            if ptr::eq((*win).w_buffer, (*wp).w_buffer) && win_redraw_signcols(win) {
                changed_line_abv_curs_win(win);
                redraw_later(win, UPD_NOT_VALID);
            }
            win = (*win).w_next;
        }
        (*buf).b_signcols.last_max = (*buf).b_signcols.max;

        validate_virtcol(wp);
        type_ = (*wp).w_redr_type;

        init_search_hl(wp, &raw mut screen_search_hl);

        if (*wp).w_skipcol > 0 && (*wp).w_view_width > win_col_off(wp) {
            let mut w: c_int = 0;
            let width1 = (*wp).w_view_width - win_col_off(wp);
            let width2 = width1 + win_col_off2(wp);
            let mut add = width1;

            while w < (*wp).w_skipcol {
                if w > 0 {
                    add = width2;
                }
                w += add;
            }
            if w != (*wp).w_skipcol {
                (*wp).w_skipcol = w - add;
            }
        }

        let nrwidth_before = (*wp).w_nrwidth;
        let nrwidth_new = if (*wp).w_onebuf_opt.wo_nu != 0
            || (*wp).w_onebuf_opt.wo_rnu != 0
            || *(*wp).w_onebuf_opt.wo_stc != 0
        {
            number_width(wp)
        } else {
            0
        };

        if (*wp).w_nrwidth != nrwidth_new {
            type_ = UPD_NOT_VALID;
            changed_line_abv_curs_win(wp);
            (*wp).w_nrwidth = nrwidth_new;
        } else {
            mod_top = (*wp).w_redraw_top;
            mod_bot = if (*wp).w_redraw_bot != 0 {
                (*wp).w_redraw_bot + 1
            } else {
                0
            };
            if (*buf).b_mod_set {
                if mod_top == 0 || mod_top > (*buf).b_mod_top {
                    mod_top = (*buf).b_mod_top;
                    if syntax_present(wp) {
                        mod_top -= (*buf).b_s.b_syn_sync_linebreaks;
                        mod_top = mod_top.max(1);
                    }
                }
                if mod_bot == 0 || mod_bot < (*buf).b_mod_bot {
                    mod_bot = (*buf).b_mod_bot;
                }

                if !screen_search_hl.rm.regprog.is_null()
                    && re_multiline(screen_search_hl.rm.regprog) != 0
                {
                    top_to_mod = true;
                } else {
                    let mut cur = (*wp).w_match_head.cast::<matchitem_T>();
                    while !cur.is_null() {
                        if !(*cur).mit_match.regprog.is_null()
                            && re_multiline((*cur).mit_match.regprog) != 0
                        {
                            top_to_mod = true;
                            break;
                        }
                        cur = (*cur).mit_next;
                    }
                }
            }

            if search_hl_has_cursor_lnum > 0 {
                if mod_top == 0 || mod_top > search_hl_has_cursor_lnum {
                    mod_top = search_hl_has_cursor_lnum;
                }
                if mod_bot == 0 || mod_bot < search_hl_has_cursor_lnum + 1 {
                    mod_bot = search_hl_has_cursor_lnum + 1;
                }
            }

            if mod_top != 0 && win_lines_concealed(wp) {
                let mut lnumt = (*wp).w_topline;
                let mut lnumb = MAXLNUM;
                for i in 0..(*wp).w_lines_valid {
                    let wl = *w_lines_at(wp, i as usize);
                    if wl.wl_valid {
                        if wl.wl_lastlnum < mod_top {
                            lnumt = wl.wl_lastlnum + 1;
                        }
                        if lnumb == MAXLNUM && wl.wl_lnum >= mod_bot {
                            lnumb = wl.wl_lnum;
                            if compute_foldcolumn(wp, 0) > 0 {
                                lnumb += 1;
                            }
                        }
                    }
                }

                hasFolding(wp, mod_top, &raw mut mod_top, ptr::null_mut());
                mod_top = mod_top.min(lnumt);

                mod_bot -= 1;
                hasFolding(wp, mod_bot, ptr::null_mut(), &raw mut mod_bot);
                mod_bot += 1;
                mod_bot = mod_bot.max(lnumb);
            }

            if mod_top != 0 && mod_top < (*wp).w_topline {
                if mod_bot > (*wp).w_topline {
                    mod_top = (*wp).w_topline;
                } else if syntax_present(wp) {
                    top_end = 1;
                }
            }
        }

        (*wp).w_redraw_top = 0;
        (*wp).w_redraw_bot = 0;
        search_hl_has_cursor_lnum = 0;

        if type_ == UPD_REDRAW_TOP {
            let mut j: c_int = 0;
            for i in 0..(*wp).w_lines_valid {
                j += c_int::from((*w_lines_at(wp, i as usize)).wl_size);
                if j >= (*wp).w_upd_rows {
                    top_end = j;
                    break;
                }
            }
            if top_end == 0 {
                type_ = UPD_NOT_VALID;
            } else {
                type_ = UPD_VALID;
            }
        }

        let mut topline_conceal = (*wp).w_topline;
        while topline_conceal < (*buf).b_ml.ml_line_count
            && decor_conceal_line(wp, topline_conceal - 1, false)
        {
            topline_conceal += 1;
            hasFolding(
                wp,
                topline_conceal,
                ptr::null_mut(),
                &raw mut topline_conceal,
            );
        }

        if (type_ == UPD_VALID
            || type_ == UPD_SOME_VALID
            || type_ == UPD_INVERTED
            || type_ == UPD_INVERTED_ALL)
            && !(*wp).w_botfill
            && !(*wp).w_old_botfill
        {
            let wl0 = *w_lines_at(wp, 0);
            if mod_top != 0
                && (*wp).w_topline == mod_top
                && (!wl0.wl_valid || topline_conceal == wl0.wl_lnum)
            {
                // w_topline is the first changed line and window is not scrolled
            } else if wl0.wl_valid
                && (topline_conceal < wl0.wl_lnum
                    || (topline_conceal == wl0.wl_lnum && (*wp).w_topfill > (*wp).w_old_topfill))
            {
                let mut j: c_int;
                if win_lines_concealed(wp) {
                    j = 0;
                    let mut ln = (*wp).w_topline;
                    while ln < wl0.wl_lnum {
                        j += c_int::from(!decor_conceal_line(wp, ln - 1, false));
                        if j >= (*wp).w_view_height - 2 {
                            break;
                        }
                        hasFolding(wp, ln, ptr::null_mut(), &raw mut ln);
                        ln += 1;
                    }
                } else {
                    j = wl0.wl_lnum - (*wp).w_topline;
                }
                if j < (*wp).w_view_height - 2 {
                    let mut i =
                        plines_m_win(wp, (*wp).w_topline, wl0.wl_lnum - 1, (*wp).w_view_height);
                    if wl0.wl_lnum != (*wp).w_topline {
                        i += win_get_fill(wp, wl0.wl_lnum) - (*wp).w_old_topfill;
                    }
                    if i != 0 && i < (*wp).w_view_height - 2 {
                        win_scroll_lines(wp, 0, i);
                        bot_scroll_start = 0;
                        if (*wp).w_lines_valid != 0 {
                            top_end = i;
                            scrolled_down = true;

                            (*wp).w_lines_valid += j;
                            if (*wp).w_lines_valid > (*wp).w_view_height {
                                (*wp).w_lines_valid = (*wp).w_view_height;
                            }
                            let mut idx = (*wp).w_lines_valid;
                            while idx - j >= 0 {
                                *w_lines_at(wp, idx as usize) = *w_lines_at(wp, (idx - j) as usize);
                                idx -= 1;
                            }
                            while idx >= 0 {
                                (*w_lines_at(wp, idx as usize)).wl_valid = false;
                                idx -= 1;
                            }
                        }
                    } else {
                        mid_start = 0;
                    }
                } else {
                    mid_start = 0;
                }
            } else {
                let mut j: c_int = -1;
                let mut row: c_int = 0;
                for i in 0..(*wp).w_lines_valid {
                    let wl = *w_lines_at(wp, i as usize);
                    if wl.wl_valid && wl.wl_lnum == (*wp).w_topline {
                        j = i;
                        break;
                    }
                    row += c_int::from(wl.wl_size);
                }
                if j == -1 {
                    mid_start = 0;
                } else {
                    if wl0.wl_lnum == (*wp).w_topline {
                        row += (*wp).w_old_topfill;
                    } else {
                        row += win_get_fill(wp, (*wp).w_topline);
                    }
                    row -= (*wp).w_topfill;
                    if row > 0 {
                        win_scroll_lines(wp, 0, -row);
                        bot_start = (*wp).w_view_height - row;
                        bot_scroll_start = bot_start;
                    }
                    if (row == 0 || bot_start < 999) && (*wp).w_lines_valid != 0 {
                        bot_start = 0;
                        let mut idx: c_int = 0;
                        loop {
                            let wlj = *w_lines_at(wp, j as usize);
                            *w_lines_at(wp, idx as usize) = wlj;
                            if row > 0
                                && bot_start + row + c_int::from(wlj.wl_size) > (*wp).w_view_height
                            {
                                (*wp).w_lines_valid = idx + 1;
                                break;
                            }
                            bot_start += c_int::from(wlj.wl_size);
                            idx += 1;

                            j += 1;
                            if j >= (*wp).w_lines_valid {
                                (*wp).w_lines_valid = idx;
                                break;
                            }
                        }

                        if win_may_fill(wp) && bot_start > 0 {
                            (*w_lines_at(wp, 0)).wl_size = plines_correct_topline(
                                wp,
                                (*wp).w_topline,
                                ptr::null_mut(),
                                true,
                                ptr::null_mut(),
                            ) as u16;
                        }
                    }
                }
            }

            if mid_start == 0 {
                mid_end = (*wp).w_view_height;
            }
        } else {
            mid_start = 0;
            mid_end = (*wp).w_view_height;
        }

        if type_ == UPD_SOME_VALID {
            mid_start = 0;
            mid_end = (*wp).w_view_height;
            type_ = UPD_NOT_VALID;
        }

        if (Visual.active && ptr::eq(buf, (*curwin).w_buffer))
            || ((*wp).w_old_cursor_lnum != 0 && type_ != UPD_NOT_VALID)
        {
            let mut from: linenr_T;
            let mut to: linenr_T;

            if Visual.active {
                if Visual.mode != c_int::from((*wp).w_old_visual_mode as u8)
                    || type_ == UPD_INVERTED_ALL
                {
                    if (*curwin).w_cursor.lnum < Visual.start.lnum {
                        from = (*curwin).w_cursor.lnum;
                        to = Visual.start.lnum;
                    } else {
                        from = Visual.start.lnum;
                        to = (*curwin).w_cursor.lnum;
                    }
                    from = from
                        .min((*wp).w_old_cursor_lnum)
                        .min((*wp).w_old_visual_lnum);
                    to = to.max((*wp).w_old_cursor_lnum).max((*wp).w_old_visual_lnum);
                } else {
                    if (*curwin).w_cursor.lnum < (*wp).w_old_cursor_lnum {
                        from = (*curwin).w_cursor.lnum;
                        to = (*wp).w_old_cursor_lnum;
                    } else {
                        from = (*wp).w_old_cursor_lnum;
                        to = (*curwin).w_cursor.lnum;
                        if from == 0 {
                            from = to;
                        }
                    }

                    if Visual.start.lnum != (*wp).w_old_visual_lnum
                        || Visual.start.col != (*wp).w_old_visual_col
                    {
                        if (*wp).w_old_visual_lnum < from && (*wp).w_old_visual_lnum != 0 {
                            from = (*wp).w_old_visual_lnum;
                        }
                        to = to.max((*wp).w_old_visual_lnum).max(Visual.start.lnum);
                        from = from.min(Visual.start.lnum);
                    }
                }

                if Visual.mode == Ctrl_V {
                    let mut fromc: colnr_T = 0;
                    let mut toc: colnr_T = 0;
                    getvcols(
                        wp,
                        &raw mut Visual.start,
                        &raw mut (*curwin).w_cursor,
                        &raw mut fromc,
                        &raw mut toc,
                        GETVCOL_END_EXCL_LBR,
                    );
                    toc += 1;
                    if (*curwin).w_curswant == MAXCOL {
                        if (get_ve_flags(curwin) & kOptVeFlagBlock) != 0 {
                            let cursor_above = (*curwin).w_cursor.lnum < Visual.start.lnum;
                            toc = 0;
                            let mut pos = pos_T {
                                lnum: (*curwin).w_cursor.lnum,
                                col: 0,
                                coladd: 0,
                            };
                            while if cursor_above {
                                pos.lnum <= Visual.start.lnum
                            } else {
                                pos.lnum >= Visual.start.lnum
                            } {
                                let mut t: colnr_T = 0;
                                pos.col = ml_get_buf_len((*wp).w_buffer, pos.lnum);
                                getvvcol(
                                    wp,
                                    &raw mut pos,
                                    ptr::null_mut(),
                                    ptr::null_mut(),
                                    &raw mut t,
                                    0,
                                );
                                toc = toc.max(t);
                                pos.lnum += if cursor_above { 1 } else { -1 };
                            }
                            toc += 1;
                        } else {
                            toc = MAXCOL;
                        }
                    }

                    if fromc != (*wp).w_old_cursor_fcol || toc != (*wp).w_old_cursor_lcol {
                        from = from.min(Visual.start.lnum);
                        to = to.max(Visual.start.lnum);
                    }
                    (*wp).w_old_cursor_fcol = fromc;
                    (*wp).w_old_cursor_lcol = toc;
                }
            } else if (*wp).w_old_cursor_lnum < (*wp).w_old_visual_lnum {
                from = (*wp).w_old_cursor_lnum;
                to = (*wp).w_old_visual_lnum;
            } else {
                from = (*wp).w_old_visual_lnum;
                to = (*wp).w_old_cursor_lnum;
            }

            from = from.max((*wp).w_topline);

            if ((*wp).w_valid & VALID_BOTLINE) != 0 {
                from = from.min((*wp).w_botline);
                to = to.min((*wp).w_botline);
            }

            if mid_start > 0 {
                let mut lnum = (*wp).w_topline;
                let mut idx: c_int = 0;
                let mut srow: c_int = 0;
                mid_start = if scrolled_down { top_end } else { 0 };
                while lnum < from && idx < (*wp).w_lines_valid {
                    let wl = *w_lines_at(wp, idx as usize);
                    if wl.wl_valid {
                        mid_start += c_int::from(wl.wl_size);
                    } else if !scrolled_down {
                        srow += c_int::from(wl.wl_size);
                    }
                    idx += 1;
                    if idx < (*wp).w_lines_valid && (*w_lines_at(wp, idx as usize)).wl_valid {
                        lnum = (*w_lines_at(wp, idx as usize)).wl_lnum;
                    } else {
                        lnum += 1;
                    }
                }
                srow += mid_start;
                mid_end = (*wp).w_view_height;
                while idx < (*wp).w_lines_valid {
                    let wl = *w_lines_at(wp, idx as usize);
                    if wl.wl_valid && wl.wl_lnum > to {
                        mid_end = srow;
                        break;
                    }
                    srow += c_int::from(wl.wl_size);
                    idx += 1;
                }
            }
        }

        if Visual.active && ptr::eq(buf, (*curwin).w_buffer) {
            (*wp).w_old_visual_mode = Visual.mode as c_char;
            (*wp).w_old_cursor_lnum = (*curwin).w_cursor.lnum;
            (*wp).w_old_visual_lnum = Visual.start.lnum;
            (*wp).w_old_visual_col = Visual.start.col;
            (*wp).w_old_curswant = (*curwin).w_curswant;
        } else {
            (*wp).w_old_visual_mode = 0;
            (*wp).w_old_cursor_lnum = 0;
            (*wp).w_old_visual_lnum = 0;
            (*wp).w_old_visual_col = 0;
        }

        let mut cursorline_fi: foldinfo_T = std::mem::zeroed();
        win_update_cursorline(wp, &raw mut cursorline_fi);
        if ptr::eq(wp, curwin) {
            conceal_cursor_used = conceal_cursor_line(curwin);
        }

        win_check_ns_hl(wp);

        let mut spv = spellvars_T::default();
        let mut lnum = (*wp).w_topline;
        if spell_check_window(wp) {
            spv.spv_has_spell = true;
            spv.spv_unchanged = mod_top == 0;
        }

        let mut idx: c_int = 0;
        let mut row: c_int = 0;
        let mut srow: c_int = 0;

        let mut eof = false;
        let mut didline = false;

        'statuscol_retry: loop {
            loop {
                if row == (*wp).w_view_height {
                    didline = true;
                    break;
                }

                if lnum > (*buf).b_ml.ml_line_count {
                    eof = true;
                    break;
                }

                srow = row;

                if row < top_end
                    || (row >= mid_start && row < mid_end)
                    || top_to_mod
                    || idx >= (*wp).w_lines_valid
                    || (row + c_int::from((*w_lines_at(wp, idx as usize)).wl_size) > bot_start)
                    || (mod_top != 0
                        && (lnum == mod_top
                            || (lnum >= mod_top
                                && (lnum < mod_bot
                                    || did_update == DidUpdate::Fold
                                    || (did_update == DidUpdate::Line
                                        && syntax_present(wp)
                                        && ((foldmethodIsSyntax(wp) && hasAnyFolding(wp) != 0)
                                            || syntax_check_changed(lnum)))
                                    || (!(*wp).w_match_head.is_null()
                                        && (*buf).b_mod_set
                                        && (*buf).b_mod_xlines != 0)))))
                    || lnum == (*wp).w_cursorline
                    || lnum == (*wp).w_last_cursorline
                {
                    if lnum == mod_top {
                        top_to_mod = false;
                    }

                    let foldinfo = if (*wp).w_onebuf_opt.wo_cul != 0 && lnum == (*wp).w_cursor.lnum
                    {
                        cursorline_fi
                    } else {
                        fold_info(wp, lnum)
                    };

                    let concealed = decor_conceal_line(wp, lnum - 1, false);
                    if concealed && win_get_fill(wp, lnum) == 0 {
                        let step = if foldinfo.fi_lines != 0 {
                            foldinfo.fi_lines
                        } else {
                            1
                        };
                        if lnum == mod_top && lnum < mod_bot {
                            mod_top += step;
                        }
                        lnum += step;
                        spv.spv_capcol_lnum = 0;
                        continue;
                    }

                    if !scrolled_for_mod
                        && mod_bot != MAXLNUM
                        && lnum >= mod_top
                        && lnum < mod_bot.max(mod_top + 1)
                        && (!scrolled_down || row >= top_end)
                    {
                        scrolled_for_mod = true;

                        let mut old_cline_height: c_int = 0;
                        let mut old_rows: c_int = 0;
                        let mut i = idx;

                        while i < (*wp).w_lines_valid {
                            let wli = *w_lines_at(wp, i as usize);
                            if wli.wl_valid && wli.wl_lnum == mod_bot {
                                break;
                            }
                            if wli.wl_lnum == (*wp).w_cursor.lnum {
                                old_cline_height = c_int::from(wli.wl_size);
                            }
                            old_rows += c_int::from(wli.wl_size);
                            if wli.wl_valid && wli.wl_lastlnum + 1 == mod_bot {
                                i += 1;
                                while i < (*wp).w_lines_valid
                                    && !(*w_lines_at(wp, i as usize)).wl_valid
                                {
                                    old_rows += c_int::from((*w_lines_at(wp, i as usize)).wl_size);
                                    i += 1;
                                }
                                break;
                            }
                            i += 1;
                        }

                        if i >= (*wp).w_lines_valid {
                            bot_start = 0;
                            bot_scroll_start = 0;
                        } else {
                            let mut new_rows: c_int = 0;
                            let mut j = idx;
                            let mut l = lnum;
                            while l < mod_bot {
                                if dollar_vcol >= 0
                                    && ptr::eq(wp, curwin)
                                    && old_cline_height > 0
                                    && l == (*wp).w_cursor.lnum
                                {
                                    new_rows += old_cline_height;
                                    j += 1;
                                } else {
                                    let n = plines_correct_topline(
                                        wp,
                                        l,
                                        &raw mut l,
                                        true,
                                        ptr::null_mut(),
                                    );
                                    new_rows += n;
                                    j += c_int::from(n > 0);
                                }
                                if new_rows > (*wp).w_view_height - row - 2 {
                                    new_rows = 9999;
                                    break;
                                }
                                l += 1;
                            }
                            let xtra_rows = new_rows - old_rows;
                            if xtra_rows < 0 {
                                if row - xtra_rows >= (*wp).w_view_height - 2 {
                                    mod_bot = MAXLNUM;
                                } else {
                                    win_scroll_lines(wp, row, xtra_rows);
                                    bot_start = (*wp).w_view_height + xtra_rows;
                                    bot_scroll_start = bot_start;
                                }
                            } else if xtra_rows > 0 {
                                if row + xtra_rows >= (*wp).w_view_height - 2 {
                                    mod_bot = MAXLNUM;
                                } else {
                                    win_scroll_lines(wp, row + old_rows, xtra_rows);
                                    bot_scroll_start = 0;
                                    if top_end > row + old_rows {
                                        top_end += xtra_rows;
                                    }
                                }
                            }

                            if mod_bot != MAXLNUM && i != j {
                                if j < i {
                                    let mut x = row + new_rows;
                                    loop {
                                        if i >= (*wp).w_lines_valid {
                                            (*wp).w_lines_valid = j;
                                            break;
                                        }
                                        let wli = *w_lines_at(wp, i as usize);
                                        *w_lines_at(wp, j as usize) = wli;
                                        if x + c_int::from(wli.wl_size) > (*wp).w_view_height {
                                            (*wp).w_lines_valid = j + 1;
                                            break;
                                        }
                                        x += c_int::from(wli.wl_size);
                                        j += 1;
                                        i += 1;
                                    }
                                    bot_start = bot_start.min(x);
                                } else {
                                    j -= i;
                                    (*wp).w_lines_valid += j;
                                    (*wp).w_lines_valid =
                                        (*wp).w_lines_valid.min((*wp).w_view_height);
                                    i = (*wp).w_lines_valid;
                                    while i - j >= idx {
                                        *w_lines_at(wp, i as usize) =
                                            *w_lines_at(wp, (i - j) as usize);
                                        i -= 1;
                                    }
                                    while i >= idx {
                                        (*w_lines_at(wp, i as usize)).wl_size = 0;
                                        (*w_lines_at(wp, i as usize)).wl_valid = false;
                                        i -= 1;
                                    }
                                }
                            }
                        }
                    }

                    let wl_cur = *w_lines_at(wp, idx as usize);
                    if foldinfo.fi_lines == 0
                        && idx < (*wp).w_lines_valid
                        && wl_cur.wl_valid
                        && wl_cur.wl_lnum == lnum
                        && lnum > (*wp).w_topline
                        && (dy_flags & (kOptDyFlagLastline | kOptDyFlagTruncate)) == 0
                        && srow + c_int::from(wl_cur.wl_size) > (*wp).w_view_height
                        && win_get_fill(wp, lnum) == 0
                    {
                        row = (*wp).w_view_height + 1;
                    } else {
                        prepare_search_hl(wp, &raw mut screen_search_hl, lnum);
                        if syntax_last_parsed != 0
                            && syntax_last_parsed + 1 < lnum
                            && syntax_present(wp)
                        {
                            syntax_end_parsing(wp, syntax_last_parsed + 1);
                        }

                        let display_buf_line = !concealed
                            && (foldinfo.fi_lines == 0
                                || (*wp).w_onebuf_opt.wo_fdt.cb_type == kCallbackNone);

                        let mut zero_spv = spellvars_T::default();
                        row = win_line(
                            wp,
                            lnum,
                            srow,
                            (*wp).w_view_height,
                            0,
                            concealed,
                            if display_buf_line {
                                &raw mut spv
                            } else {
                                &raw mut zero_spv
                            },
                            foldinfo,
                        );

                        if display_buf_line {
                            syntax_last_parsed = lnum;
                        } else {
                            spv.spv_capcol_lnum = 0;
                        }

                        let lastlnum =
                            lnum + foldinfo.fi_lines - linenr_T::from(foldinfo.fi_lines > 0);
                        let wl_ptr = w_lines_at(wp, idx as usize);
                        (*wl_ptr).wl_folded = foldinfo.fi_lines > 0;
                        (*wl_ptr).wl_foldend = lastlnum;
                        (*wl_ptr).wl_lastlnum = lastlnum;
                        did_update = if foldinfo.fi_lines > 0 {
                            DidUpdate::Fold
                        } else {
                            DidUpdate::Line
                        };

                        let mut virt_below = decor_virt_lines(
                            wp,
                            lastlnum,
                            lastlnum + 1,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            true,
                        ) > 0;
                        while !virt_below
                            && (*w_lines_at(wp, idx as usize)).wl_lastlnum
                                < (*buf).b_ml.ml_line_count
                            && decor_conceal_line(
                                wp,
                                (*w_lines_at(wp, idx as usize)).wl_lastlnum,
                                false,
                            )
                        {
                            virt_below = false;
                            (*w_lines_at(wp, idx as usize)).wl_lastlnum += 1;
                            let last_ptr = &raw mut (*w_lines_at(wp, idx as usize)).wl_lastlnum;
                            hasFolding(wp, *last_ptr, ptr::null_mut(), last_ptr);
                        }
                    }

                    (*w_lines_at(wp, idx as usize)).wl_lnum = lnum;
                    (*w_lines_at(wp, idx as usize)).wl_valid = true;

                    let is_curline = ptr::eq(wp, curwin) && lnum == (*wp).w_cursor.lnum;

                    if row > (*wp).w_view_height {
                        if dollar_vcol == -1 || !is_curline {
                            (*w_lines_at(wp, idx as usize)).wl_size =
                                plines_win(wp, lnum, true) as u16;
                        }
                        idx += 1;
                        break;
                    }
                    if dollar_vcol == -1 || !is_curline {
                        (*w_lines_at(wp, idx as usize)).wl_size = (row - srow) as u16;
                    }
                    lnum = (*w_lines_at(wp, idx as usize)).wl_lastlnum + 1;
                    idx += 1;
                } else {
                    if ((*wp).w_onebuf_opt.wo_nu != 0
                        && mod_top != 0
                        && lnum >= mod_bot
                        && (*buf).b_mod_set
                        && (*buf).b_mod_xlines != 0)
                        || ((*wp).w_onebuf_opt.wo_rnu != 0
                            && (*wp).w_last_cursor_lnum_rnu != (*wp).w_cursor.lnum)
                    {
                        let info = if (*wp).w_onebuf_opt.wo_cul != 0 && lnum == (*wp).w_cursor.lnum
                        {
                            cursorline_fi
                        } else {
                            fold_info(wp, lnum)
                        };
                        win_line(
                            wp,
                            lnum,
                            srow,
                            (*wp).w_view_height,
                            c_int::from((*w_lines_at(wp, idx as usize)).wl_size),
                            false,
                            &raw mut spv,
                            info,
                        );
                    }

                    row += c_int::from((*w_lines_at(wp, idx as usize)).wl_size);
                    idx += 1;
                    if row > (*wp).w_view_height {
                        break;
                    }
                    lnum = (*w_lines_at(wp, (idx - 1) as usize)).wl_lastlnum + 1;
                    did_update = DidUpdate::None;
                    spv.spv_capcol_lnum = 0;
                }

                if (*wp).w_redr_statuscol {
                    (*wp).w_redr_statuscol = false;
                    idx = 0;
                    row = 0;
                    lnum = (*wp).w_topline;
                    (*wp).w_lines_valid = 0;
                    (*wp).w_valid &= !VALID_WCOL;
                    decor_redraw_reset(wp, &raw mut decor_state);
                    decor_providers_invoke_win(wp);
                    continue;
                }

                if lnum > (*buf).b_ml.ml_line_count {
                    eof = true;
                    break;
                }
            }

            (*wp).w_last_cursorline = (*wp).w_cursorline;
            (*wp).w_last_cursor_lnum_rnu = if (*wp).w_onebuf_opt.wo_rnu != 0 {
                (*wp).w_cursor.lnum
            } else {
                0
            };
            (*wp).w_lines_valid = (*wp).w_lines_valid.max(idx);
            (*wp).w_display_tick = display_tick;

            if syntax_last_parsed != 0 && syntax_present(wp) {
                syntax_end_parsing(wp, syntax_last_parsed + 1);
            }

            let old_botline = (*wp).w_botline;

            (*wp).w_empty_rows = 0;
            (*wp).w_filler_rows = 0;
            if !eof && !didline {
                let at_attr = hl_combine_attr(win_bg_attr(wp), win_hl_attr(wp, HLF_AT));
                if lnum == (*wp).w_topline {
                    (*wp).w_botline = lnum + 1;
                } else if win_get_fill(wp, lnum) >= (*wp).w_view_height - srow {
                    (*wp).w_botline = lnum;
                    (*wp).w_filler_rows = (*wp).w_view_height - srow;
                } else if (dy_flags & kOptDyFlagTruncate) != 0 {
                    grid_line_start(&raw mut (*wp).w_grid, (*wp).w_view_height - 1);
                    grid_line_fill(
                        0,
                        (*wp).w_view_width.min(3),
                        (*wp).w_p_fcs_chars.lastline,
                        at_attr,
                    );
                    grid_line_fill(3, (*wp).w_view_width, schar_from_ascii(b' '), at_attr);
                    grid_line_flush();
                    set_empty_rows(wp, srow);
                    (*wp).w_botline = lnum;
                } else if (dy_flags & kOptDyFlagLastline) != 0 {
                    grid_line_start(&raw mut (*wp).w_grid, (*wp).w_view_height - 1);
                    let width =
                        if grid_line_getchar(((*wp).w_view_width - 3).max(0), ptr::null_mut()) == 0
                        {
                            4
                        } else {
                            3
                        };
                    grid_line_fill(
                        ((*wp).w_view_width - width).max(0),
                        (*wp).w_view_width,
                        (*wp).w_p_fcs_chars.lastline,
                        at_attr,
                    );
                    grid_line_flush();
                    set_empty_rows(wp, srow);
                    (*wp).w_botline = lnum;
                } else {
                    win_draw_end(
                        wp,
                        (*wp).w_p_fcs_chars.lastline,
                        true,
                        srow,
                        (*wp).w_view_height,
                        HLF_AT,
                    );
                    set_empty_rows(wp, srow);
                    (*wp).w_botline = lnum;
                }
            } else {
                if eof {
                    (*wp).w_botline = (*buf).b_ml.ml_line_count + 1;
                    let j = win_get_fill(wp, (*wp).w_botline);
                    if j > 0 && !(*wp).w_botfill && row < (*wp).w_view_height {
                        let mut zero_spv = spellvars_T::default();
                        let zero_foldinfo: foldinfo_T = std::mem::zeroed();
                        row = win_line(
                            wp,
                            (*wp).w_botline,
                            row,
                            (*wp).w_view_height,
                            0,
                            false,
                            &raw mut zero_spv,
                            zero_foldinfo,
                        );
                        if (*wp).w_redr_statuscol {
                            eof = false;
                            didline = false;
                            (*wp).w_redr_statuscol = false;
                            idx = 0;
                            row = 0;
                            lnum = (*wp).w_topline;
                            (*wp).w_lines_valid = 0;
                            (*wp).w_valid &= !VALID_WCOL;
                            decor_redraw_reset(wp, &raw mut decor_state);
                            decor_providers_invoke_win(wp);
                            continue 'statuscol_retry;
                        }
                    }
                } else if dollar_vcol == -1 || !ptr::eq(wp, curwin) {
                    (*wp).w_botline = lnum;
                }

                let mut lastline = bot_scroll_start;
                if mid_end >= row {
                    lastline = lastline.min(mid_start);
                }
                if mod_bot > (*buf).b_ml.ml_line_count {
                    lastline = 0;
                }

                win_draw_end(
                    wp,
                    (*wp).w_p_fcs_chars.eob,
                    false,
                    lastline.max(row),
                    (*wp).w_view_height,
                    HLF_EOB,
                );
                set_empty_rows(wp, row);
            }

            if (*wp).w_redr_type >= UPD_REDRAW_TOP {
                draw_vsep_win(wp);
                draw_hsep_win(wp);
            }
            syn_set_timeout(ptr::null_mut());

            (*wp).w_redr_type = 0;
            (*wp).w_old_topfill = (*wp).w_topfill;
            (*wp).w_old_botfill = (*wp).w_botfill;

            for n in 0..win_extmark_arr.size {
                let item = *win_extmark_arr.items.add(n);
                ui_call_win_extmark(
                    Integer::from((*wp).w_grid_alloc.handle),
                    (*wp).handle,
                    Integer::from(item.ns_id),
                    item.mark_id as Integer,
                    Integer::from(item.win_row),
                    Integer::from(item.win_col),
                );
            }

            if dollar_vcol == -1 || !ptr::eq(wp, curwin) {
                (*wp).w_valid |= VALID_BOTLINE;
                (*wp).w_viewport_invalid = true;
                if ptr::eq(wp, curwin) && (*wp).w_botline != old_botline && !win_update_recursive {
                    win_update_recursive = true;
                    (*curwin).w_valid &= !VALID_TOPLINE;
                    update_topline(curwin);
                    if must_redraw != 0 {
                        let mod_set = (*curbuf).b_mod_set;
                        (*curbuf).b_mod_set = false;
                        curs_columns(curwin, 1);
                        win_update(curwin);
                        must_redraw = 0;
                        (*curbuf).b_mod_set = mod_set;
                    }
                    win_update_recursive = false;
                }
            }

            if nrwidth_before != (*wp).w_nrwidth && !(*buf).terminal.is_null() {
                terminal_check_size((*buf).terminal.cast::<Terminal>());
            }

            if !got_int {
                got_int = save_got_int;
            }
            break 'statuscol_retry;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_scroll_lines(wp: *mut win_T, row: c_int, line_count: c_int) {
    unsafe {
        if !redrawing() || line_count == 0 {
            return;
        }

        let mut col: c_int = 0;
        let mut row_off: c_int = 0;
        let grid = grid_adjust(&raw mut (*wp).w_grid, &raw mut row_off, &raw mut col);

        let checked_width = ((*grid).cols - col).min((*wp).w_view_width);
        let checked_height = ((*grid).rows - row_off).min((*wp).w_view_height);

        if row + line_count.abs() >= checked_height {
            return;
        }

        if line_count < 0 {
            grid_del_lines(
                grid,
                row + row_off,
                -line_count,
                checked_height + row_off,
                col,
                checked_width,
            );
        } else {
            grid_ins_lines(
                grid,
                row + row_off,
                line_count,
                checked_height + row_off,
                col,
                checked_width,
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_draw_end(
    wp: *mut win_T,
    c1: schar_T,
    draw_margin: bool,
    startrow: c_int,
    endrow: c_int,
    hl: hlf_T,
) {
    unsafe {
        assert!((0..HLF_COUNT).contains(&hl));
        let view_width = (*wp).w_view_width;
        let fdc = compute_foldcolumn(wp, 0);
        let scwidth = (*wp).w_scwidth;

        for row in startrow..endrow {
            grid_line_start(&raw mut (*wp).w_grid, row);

            let mut n: c_int = 0;
            if draw_margin {
                if fdc > 0 {
                    n = grid_line_fill(
                        n,
                        view_width.min(n + fdc),
                        schar_from_ascii(b' '),
                        win_hl_attr(wp, HLF_FC),
                    );
                }

                if scwidth > 0 {
                    n = grid_line_fill(
                        n,
                        view_width.min(n + scwidth * SIGN_WIDTH),
                        schar_from_ascii(b' '),
                        win_hl_attr(wp, HLF_SC),
                    );
                }

                if ((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
                    && vim_strchr(p_cpo, kCpoNumcol).is_null()
                {
                    let width = number_width(wp) + 1;
                    n = grid_line_fill(
                        n,
                        view_width.min(n + width),
                        schar_from_ascii(b' '),
                        win_hl_attr(wp, HLF_N),
                    );
                }
            }

            let attr = win_hl_attr(wp, hl);

            if n < view_width {
                grid_line_put_schar(n, c1, attr);
                n += 1;
            }

            grid_line_clear_end(n, view_width, win_bg_attr(wp), attr);

            if (*wp).w_onebuf_opt.wo_rl != 0 {
                grid_line_mirror(view_width);
            }
            grid_line_flush();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compute_foldcolumn(wp: *mut win_T, col: c_int) -> c_int {
    unsafe {
        let fdc = win_fdccol_count(wp);
        let wmw = if ptr::eq(wp, curwin) && p_wmw == 0 {
            1
        } else {
            p_wmw as c_int
        };
        let n = (*wp).w_view_width - (col + wmw);
        fdc.min(n)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn number_width(wp: *mut win_T) -> c_int {
    unsafe {
        let mut lnum = if (*wp).w_onebuf_opt.wo_rnu != 0 && (*wp).w_onebuf_opt.wo_nu == 0 {
            (*wp).w_view_height
        } else {
            (*(*wp).w_buffer).b_ml.ml_line_count
        };

        if lnum == (*wp).w_nrwidth_line_count {
            return (*wp).w_nrwidth_width;
        }
        (*wp).w_nrwidth_line_count = lnum;

        if *(*wp).w_onebuf_opt.wo_stc != NUL {
            (*wp).w_statuscol_line_count = 0;
            (*wp).w_nrwidth_width =
                c_int::from((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
                    * ((*wp).w_onebuf_opt.wo_nuw as c_int);
            return (*wp).w_nrwidth_width;
        }

        let mut n: c_int = 0;
        loop {
            lnum /= 10;
            n += 1;
            if lnum <= 0 {
                break;
            }
        }

        n = n.max(((*wp).w_onebuf_opt.wo_nuw as c_int) - 1);

        if n < 2
            && buf_meta_total((*wp).w_buffer, kMTMetaSignText) != 0
            && (*wp).w_minscwidth == SCL_NUM
        {
            n = 2;
        }

        (*wp).w_nrwidth_width = n;
        n
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_later(wp: *mut win_T, type_: c_int) {
    unsafe {
        assert!(!wp.is_null() || exiting);
        if !exiting && !redraw_not_allowed && (*wp).w_redr_type < type_ {
            (*wp).w_redr_type = type_;
            if type_ >= UPD_NOT_VALID {
                (*wp).w_lines_valid = 0;
            }
            must_redraw = must_redraw.max(type_);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_all_later(type_: c_int) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            redraw_later(wp, type_);
            wp = (*wp).w_next;
        }
        set_must_redraw(type_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_must_redraw(type_: c_int) {
    unsafe {
        if !redraw_not_allowed {
            must_redraw = must_redraw.max(type_);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn screen_invalidate_highlights() {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            redraw_later(wp, UPD_NOT_VALID);
            (*wp).w_grid_alloc.valid = false;
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_curbuf_later(type_: c_int) {
    unsafe {
        redraw_buf_later(curbuf, type_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_buf_later(buf: *mut buf_T, type_: c_int) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf) {
                redraw_later(wp, type_);
            }
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_buf_line_later(buf: *mut buf_T, line: linenr_T, force: bool) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf) {
                redrawWinline(wp, line.min((*buf).b_ml.ml_line_count));
                if force && line > (*buf).b_ml.ml_line_count {
                    (*wp).w_redraw_bot = line;
                }
            }
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_win_range_later(wp: *mut win_T, first: linenr_T, last: linenr_T) {
    unsafe {
        if last >= (*wp).w_topline && first < (*wp).w_botline {
            if (*wp).w_redraw_top == 0 || (*wp).w_redraw_top > first {
                (*wp).w_redraw_top = first;
            }
            if (*wp).w_redraw_bot == 0 || (*wp).w_redraw_bot < last {
                (*wp).w_redraw_bot = last;
            }
            redraw_later(wp, UPD_VALID);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redrawWinline(wp: *mut win_T, lnum: linenr_T) {
    unsafe {
        redraw_win_range_later(wp, lnum, lnum);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_buf_range_later(buf: *mut buf_T, first: linenr_T, last: linenr_T) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf) {
                redraw_win_range_later(wp, first, last);
            }
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_buf_status_later(buf: *mut buf_T) {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf)
                && ((*wp).w_status_height != 0
                    || (ptr::eq(wp, curwin) && global_stl_height() != 0)
                    || (*wp).w_winbar_height != 0)
            {
                (*wp).w_redr_status = true;
                set_must_redraw(UPD_VALID);
            }
            wp = (*wp).w_next;
        }
        if p_ru != 0
            && ptr::eq((*curwin).w_buffer, buf)
            && (*curwin).w_status_height == 0
            && !(*curwin).w_redr_status
        {
            redraw_cmdline = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn status_redraw_all() {
    unsafe {
        let is_stl_global = global_stl_height() != 0;

        let mut wp = firstwin;
        while !wp.is_null() {
            if (!is_stl_global && (*wp).w_status_height != 0)
                || ptr::eq(wp, curwin)
                || (*wp).w_winbar_height != 0
            {
                (*wp).w_redr_status = true;
                redraw_later(wp, UPD_VALID);
            }
            wp = (*wp).w_next;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn status_redraw_curbuf() {
    unsafe {
        status_redraw_buf(curbuf);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn status_redraw_buf(buf: *mut buf_T) {
    unsafe {
        let is_stl_global = global_stl_height() != 0;

        let mut wp = firstwin;
        while !wp.is_null() {
            if ptr::eq((*wp).w_buffer, buf)
                && ((!is_stl_global && (*wp).w_status_height != 0)
                    || (is_stl_global && ptr::eq(wp, curwin))
                    || (*wp).w_winbar_height != 0)
            {
                (*wp).w_redr_status = true;
                redraw_later(wp, UPD_VALID);
            }
            wp = (*wp).w_next;
        }
        if p_ru != 0 && (*curwin).w_status_height == 0 && !(*curwin).w_redr_status {
            redraw_cmdline = true;
            redraw_later(curwin, UPD_VALID);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn redraw_statuslines() {
    unsafe {
        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_redr_status {
                win_check_ns_hl(wp);
                win_redr_winbar(wp);
                win_redr_status(wp);
            }
            wp = (*wp).w_next;
        }

        win_check_ns_hl(ptr::null_mut());
        if redraw_tabline {
            draw_tabline();
        }

        if need_maketitle {
            maketitle();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_redraw_last_status(mut frp: *const frame_T) {
    unsafe {
        if (*frp).fr_layout == FR_LEAF {
            (*(*frp).fr_win).w_redr_status = true;
        } else if (*frp).fr_layout == FR_ROW {
            let mut child = (*frp).fr_child;
            while !child.is_null() {
                win_redraw_last_status(child);
                child = (*child).fr_next;
            }
        } else {
            assert!((*frp).fr_layout == FR_COL);
            frp = (*frp).fr_child;
            while !(*frp).fr_next.is_null() {
                frp = (*frp).fr_next;
            }
            win_redraw_last_status(frp);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn conceal_cursor_line(wp: *const win_T) -> bool {
    unsafe {
        if *(*wp).w_onebuf_opt.wo_cocu == NUL {
            return false;
        }
        let c = if (get_real_state() & MODE_VISUAL) != 0 {
            c_int::from(b'v')
        } else if (State & MODE_INSERT) != 0 {
            c_int::from(b'i')
        } else if (State & MODE_NORMAL) != 0 {
            c_int::from(b'n')
        } else if (State & MODE_CMDLINE) != 0 {
            c_int::from(b'c')
        } else {
            return false;
        };
        !vim_strchr((*wp).w_onebuf_opt.wo_cocu, c).is_null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_cursorline_standout(wp: *const win_T) -> bool {
    unsafe {
        (*wp).w_onebuf_opt.wo_cul != 0
            || (ptr::eq(wp, curwin) && (*wp).w_onebuf_opt.wo_cole > 0 && !conceal_cursor_line(wp))
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_update_cursorline(wp: *mut win_T, foldinfo: *mut foldinfo_T) {
    unsafe {
        (*wp).w_cursorline = if win_cursorline_standout(wp) {
            (*wp).w_cursor.lnum
        } else {
            0
        };
        if (*wp).w_onebuf_opt.wo_cul != 0 {
            *foldinfo = fold_info(wp, (*wp).w_cursor.lnum);
            if (*foldinfo).fi_level != 0 && (*foldinfo).fi_lines > 0 {
                (*wp).w_cursorline = (*foldinfo).fi_lnum;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schar_from_ascii_and_upd_constants_match() {
        assert_eq!(schar_from_ascii(b' '), u32::from(b' '));
        assert_eq!(schar_from_ascii(b'~'), u32::from(b'~'));
        let prios = [
            UPD_VALID,
            UPD_INVERTED,
            UPD_INVERTED_ALL,
            UPD_REDRAW_TOP,
            UPD_SOME_VALID,
            UPD_NOT_VALID,
            UPD_CLEAR,
        ];
        assert!(prios.windows(2).all(|w| w[0] < w[1]));
    }
}
