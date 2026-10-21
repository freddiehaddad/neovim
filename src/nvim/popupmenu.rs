//! Popup menu (PUM) (`src/nvim/popupmenu.c`).

use crate::api::buffer::nvim_buf_set_lines;
use crate::api::private::helpers::{
    add_c, api_clear_error, api_free_array, arena_array, array_obj, boolean_obj, cstr_as_obj,
    cstr_as_string, string_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString, Object};
use crate::api::win_config::{WIN_CONFIG_INIT, kUIMultigrid, parse_winborder};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{bt_nofile, buf_clear, first_win_in_tab};
use crate::charset::{NUL, transstr, vim_strsize};
use crate::cmdexpand::{
    cmdline_compl_is_fuzzy, cmdline_compl_pattern, kUICmdline, kUIPopupmenu, kUIWildmenu,
};
use crate::cursor::linenr_T;
use crate::drawscreen::{
    DEFAULT_GRID_HANDLE, UPD_NOT_VALID, UPD_SOME_VALID, conceal_cursor_line, redraw_later,
    setcursor_mayforce, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{dict_T, tv_dict_add_bool, tv_dict_add_float, tv_dict_add_nr};
use crate::ex_cmds::{Ctrl_C, ECMD_ONE, do_ecmd, prepare_tagpreview};
use crate::ex_docmd::exarg_T;
use crate::fuzzy::fuzzy_match_str_with_pos;
use crate::garray::{ga_clear, garray_T};
use crate::globals::*;
use crate::grid::{
    get_win_by_grid_handle, grid_alloc, grid_assign_handle, grid_draw_border, grid_free,
    grid_invalidate, grid_line_fill, grid_line_flush, grid_line_put_schar, grid_line_puts, schar_T,
    schar_from_ascii, schar_from_str, screengrid_line_start,
};
use crate::highlight::{HLF_PNI, HLF_PST, hl_combine_attr, hl_get_ui_attr, win_hl_attr};
use crate::highlight_group::syn_check_group;
use crate::input::{vgetc, vungetc};
use crate::insexpand::{
    compl_match_curr_select, get_cot_flags, ins_compl_active, ins_compl_leader, kOptCotFlagFuzzy,
    kWinInfo,
};
use crate::keycodes::{
    CAR, ESC, K_DOWN, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTMOUSE_NM, K_MOUSEDOWN, K_MOUSEMOVE, K_MOUSEUP,
    K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_UP, NL,
};
use crate::mbyte::{mb_string2cells, mb_strnicmp, ptr2cells, utf_ptr2cells, utfc_ptr2len};
use crate::memory::{
    Arena, arena_finish, arena_mem_free, strequal, xcalloc, xfree, xmalloc, xrealloc,
    xstrdup,
};
use crate::menu::{execute_menu, get_menu_mode_flag, menu_find, menu_is_separator, vimmenu_T};
use crate::message::emsg;
use crate::mouse::mouse_find_win_outer;
use crate::r#move::{update_topline, validate_cheight, validate_cursor, validate_cursor_col};
use crate::option::{
    OPT_LOCAL, kOptBufhidden, kOptBuflisted, kOptBuftype, kOptSwapfile, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::plines::{plines_m_win, win_linetabsize};
use crate::strings::{gettext, reverse_text, strchr};
use crate::ui::{
    ui_call_grid_destroy, ui_call_grid_resize, ui_call_option_set, ui_call_popupmenu_hide,
    ui_call_popupmenu_select, ui_call_popupmenu_show, ui_call_win_close, ui_call_win_float_pos,
    ui_has, ui_pum_get_height, ui_pum_get_pos,
};
use crate::ui_compositor::{ui_comp_compose_grid, ui_comp_put_grid, ui_comp_remove_grid};
use crate::window::{goto_tabpage_tp, valid_tabpage, win_T, win_enter, win_setheight, win_valid};
use crate::winfloat::{win_config_float, win_float_close, win_float_find, win_float_special};
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

use crate::api::vim::kOptCotFlagPopup;
use crate::change::TAB;
use crate::diff::kOptDiff;
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::ex_getln::hlf_T;
use crate::math::OK;
use crate::regexp::MAXCOL;
use crate::state::MODE_CMDLINE;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pumitem_T {
    pub pum_text: *mut c_char,
    pub pum_kind: *mut c_char,
    pub pum_extra: *mut c_char,
    pub pum_info: *mut c_char,
    pub pum_cpt_source_idx: c_int,
    pub pum_user_abbr_hlattr: c_int,
    pub pum_user_kind_hlattr: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PumWant {
    pub active: bool,
    pub item: c_int,
    pub insert: bool,
    pub finish: bool,
}

const PUM_DEF_HEIGHT: c_int = 10;

const CPT_ABBR: c_int = 0;
const CPT_KIND: c_int = 1;
const CPT_MENU: c_int = 2;

const HLF_PSI: hlf_T = 42;
const HLF_PMNI: hlf_T = 43;
const HLF_PMSI: hlf_T = 44;
const HLF_PNK: hlf_T = 45;
const HLF_PSK: hlf_T = 46;
const HLF_PNX: hlf_T = 47;
const HLF_PSX: hlf_T = 48;
const HLF_PSB: hlf_T = 49;
const HLF_PBR: hlf_T = 51;

const kZIndexPopupMenu: c_int = 100;
const kZIndexCmdlinePopupMenu: c_int = 250;

const kOptCotFlagPreview: c_uint = 0x800;

static mut PUM_ARRAY: *mut pumitem_T = ptr::null_mut();
static mut PUM_SIZE: c_int = 0;
static mut PUM_SELECTED: c_int = 0;
static mut PUM_FIRST: c_int = 0;

static mut PUM_HEIGHT: c_int = 0;
static mut PUM_WIDTH: c_int = 0;
static mut PUM_BASE_WIDTH: c_int = 0;
static mut PUM_KIND_WIDTH: c_int = 0;
static mut PUM_EXTRA_WIDTH: c_int = 0;
static mut PUM_SCROLLBAR: c_int = 0;
static mut PUM_RL: bool = false;

static mut PUM_ANCHOR_GRID: c_int = 0;
static mut PUM_ROW: c_int = 0;
static mut PUM_COL: c_int = 0;
static mut PUM_WIN_ROW_OFFSET: c_int = 0;
static mut PUM_WIN_COL_OFFSET: c_int = 0;
static mut PUM_LEFT_COL: c_int = 0;
static mut PUM_RIGHT_COL: c_int = 0;
static mut PUM_ABOVE: bool = false;

static mut PUM_IS_VISIBLE: bool = false;
static mut PUM_IS_DRAWN: bool = false;
static mut PUM_EXTERNAL: bool = false;
static mut PUM_INVALID: bool = false;

use crate::api::private::helpers::cstr_to_string;

#[inline]
unsafe fn kv_add_obj(a: &mut Array, v: Object) {
    unsafe {
        if a.size == a.capacity {
            a.capacity = if a.capacity == 0 { 8 } else { a.capacity * 2 };
            a.items = xrealloc(a.items.cast(), a.capacity * size_of::<Object>()).cast();
        }
        *a.items.add(a.size) = v;
        a.size += 1;
    }
}

unsafe fn pum_compute_size() {
    unsafe {
        PUM_BASE_WIDTH = 0;
        PUM_KIND_WIDTH = 0;
        PUM_EXTRA_WIDTH = 0;
        for i in 0..(PUM_SIZE as usize) {
            let item = &*PUM_ARRAY.add(i);
            if !item.pum_text.is_null() {
                let w = vim_strsize(item.pum_text);
                if PUM_BASE_WIDTH < w {
                    PUM_BASE_WIDTH = w;
                }
            }
            if !item.pum_kind.is_null() {
                let w = vim_strsize(item.pum_kind) + 1;
                if PUM_KIND_WIDTH < w {
                    PUM_KIND_WIDTH = w;
                }
            }
            if !item.pum_extra.is_null() {
                let w = vim_strsize(item.pum_extra) + 1;
                if PUM_EXTRA_WIDTH < w {
                    PUM_EXTRA_WIDTH = w;
                }
            }
        }
    }
}

/// Calculate vertical placement for popup menu (`pum_compute_vertical_placement`).
unsafe fn pum_compute_vertical_placement(
    size: c_int,
    target_win: *mut win_T,
    pum_win_row: c_int,
    above_row: c_int,
    below_row: c_int,
    pum_border_size: c_int,
) {
    unsafe {
        let context_lines: c_int;

        PUM_HEIGHT = size.min(PUM_DEF_HEIGHT);
        if p_ph > 0 && i64::from(PUM_HEIGHT) > p_ph {
            PUM_HEIGHT = p_ph as c_int;
        }

        if pum_win_row + 2 + pum_border_size >= below_row - PUM_HEIGHT
            && pum_win_row - above_row > (below_row - above_row) / 2
        {
            PUM_ABOVE = true;

            if (State & MODE_CMDLINE) != 0 && target_win.is_null() {
                context_lines = 0;
            } else {
                context_lines = 2.min((*target_win).w_wrow - (*target_win).w_cline_row);
            }

            if pum_win_row >= size + context_lines {
                PUM_ROW = pum_win_row - size - context_lines;
                PUM_HEIGHT = size;
            } else {
                PUM_ROW = 0;
                PUM_HEIGHT = pum_win_row - context_lines;
            }
            if p_ph > 0 && i64::from(PUM_HEIGHT) > p_ph {
                PUM_ROW += PUM_HEIGHT - (p_ph as c_int);
                PUM_HEIGHT = p_ph as c_int;
            }

            if pum_border_size > 0 && pum_border_size + PUM_ROW + PUM_HEIGHT >= pum_win_row {
                if PUM_ROW < 2 {
                    PUM_HEIGHT -= pum_border_size;
                } else {
                    PUM_ROW -= pum_border_size;
                }
            }
        } else {
            PUM_ABOVE = false;

            if (State & MODE_CMDLINE) != 0 && target_win.is_null() {
                context_lines = 0;
            } else {
                validate_cheight(target_win);
                let cline_visible_offset =
                    (*target_win).w_cline_row + (*target_win).w_cline_height - (*target_win).w_wrow;
                context_lines = 3.min(cline_visible_offset);
            }

            PUM_ROW = pum_win_row + context_lines;
            PUM_HEIGHT = (below_row - PUM_ROW).min(size);
            if p_ph > 0 && i64::from(PUM_HEIGHT) > p_ph {
                PUM_HEIGHT = p_ph as c_int;
            }

            if PUM_ROW + PUM_HEIGHT + pum_border_size >= cmdline_row {
                PUM_HEIGHT -= pum_border_size;
            }
        }

        if above_row > 0 && PUM_ROW < above_row && PUM_HEIGHT > above_row {
            PUM_ROW = above_row;
            PUM_HEIGHT = pum_win_row - above_row;
        }
    }
}

/// Try to set `pum_width` so that it fits within `available_width` (`set_pum_width_aligned_with_cursor`).
unsafe fn set_pum_width_aligned_with_cursor(mut width: c_int, available_width: c_int) -> bool {
    unsafe {
        let mut end_padding = true;

        if i64::from(width) < p_pw {
            width = p_pw as c_int;
            end_padding = false;
        }
        if p_pmw > 0 && i64::from(width) > p_pmw {
            width = p_pmw as c_int;
            end_padding = false;
        }

        PUM_WIDTH = width
            + if end_padding && i64::from(width) >= p_pw {
                1
            } else {
                0
            };
        available_width >= PUM_WIDTH
    }
}

/// Calculate horizontal placement for popup menu (`pum_compute_horizontal_placement`).
unsafe fn pum_compute_horizontal_placement(
    target_win: *mut win_T,
    cursor_col: c_int,
    border_width: c_int,
) {
    unsafe {
        let max_col = Columns.max(if !target_win.is_null() {
            (*target_win).w_wincol + (*target_win).w_view_width
        } else {
            0
        });
        let desired_width = PUM_BASE_WIDTH + PUM_KIND_WIDTH + PUM_EXTRA_WIDTH;
        let mut available_width = if PUM_RL {
            cursor_col - PUM_SCROLLBAR + 1 - border_width
        } else {
            max_col - cursor_col - PUM_SCROLLBAR - border_width
        };

        PUM_COL = cursor_col;
        if set_pum_width_aligned_with_cursor(desired_width, available_width) {
            return;
        }

        if i64::from(available_width) > p_pw {
            PUM_WIDTH = available_width;
            return;
        }

        if PUM_RL {
            available_width = max_col - PUM_SCROLLBAR - border_width;
        } else {
            available_width += cursor_col;
        }

        if i64::from(available_width) > p_pw {
            PUM_WIDTH = (p_pw as c_int) + 1;
            if PUM_RL {
                PUM_COL = PUM_WIDTH + PUM_SCROLLBAR + border_width;
            } else {
                PUM_COL = max_col - PUM_WIDTH - PUM_SCROLLBAR - border_width;
            }
            return;
        }

        if PUM_RL {
            PUM_COL = max_col - 1;
        } else {
            PUM_COL = 0;
        }
        PUM_WIDTH = max_col - PUM_SCROLLBAR - border_width;
    }
}

#[inline]
unsafe fn pum_border_width() -> c_int {
    unsafe {
        if *p_pumborder == NUL || strequal(p_pumborder, opt_winborder_values[7]) {
            0
        } else if strequal(p_pumborder, opt_winborder_values[3]) {
            1
        } else {
            2
        }
    }
}

/// Show the popup menu with items `array[size]` (`pum_display`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_display(
    array: *mut pumitem_T,
    size: c_int,
    selected: c_int,
    array_changed: bool,
    cmd_startcol: c_int,
) {
    unsafe {
        let mut redo_count: c_int = 0;

        if !PUM_IS_VISIBLE {
            PUM_EXTERNAL =
                ui_has(kUIPopupmenu) || ((State & MODE_CMDLINE) != 0 && ui_has(kUIWildmenu));
        }

        PUM_RL = (State & MODE_CMDLINE) == 0 && (*curwin).w_onebuf_opt.wo_rl != 0;

        let border_width = pum_border_width();
        loop {
            PUM_IS_VISIBLE = true;
            PUM_IS_DRAWN = true;
            validate_cursor_col(curwin);
            let mut above_row: c_int = 0;
            let mut below_row = cmdline_row.max((*curwin).w_winrow + (*curwin).w_view_height);
            if (State & MODE_CMDLINE) != 0 {
                below_row = cmdline_row;
            }
            let target_win = if (State & MODE_CMDLINE) != 0 {
                cmdline_win
            } else {
                curwin
            };
            PUM_WIN_ROW_OFFSET = 0;
            PUM_WIN_COL_OFFSET = 0;

            let mut pum_win_row: c_int;
            let mut cursor_col: c_int;

            if (State & MODE_CMDLINE) != 0 {
                pum_win_row = if !cmdline_win.is_null() {
                    (*cmdline_win).w_wrow
                } else if ui_has(kUICmdline) {
                    0
                } else {
                    cmdline_row
                };
                cursor_col = (if !cmdline_win.is_null() {
                    (*cmdline_win).w_config._cmdline_offset
                } else {
                    0
                }) + cmd_startcol;
                cursor_col %= if !cmdline_win.is_null() {
                    (*cmdline_win).w_view_width
                } else {
                    Columns
                };
                PUM_ANCHOR_GRID = if ui_has(kUICmdline) {
                    -1
                } else {
                    DEFAULT_GRID_HANDLE
                };
            } else {
                pum_win_row = (*curwin).w_wrow;
                let mut wcol = (*curwin).w_wcol;
                if (*curwin).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(curwin) {
                    wcol -= (*curwin).w_wcol_conceal_off;
                    if wcol < 0 {
                        wcol = 0;
                    }
                }
                if PUM_RL {
                    cursor_col = (*curwin).w_view_width - wcol - 1;
                } else {
                    cursor_col = wcol;
                }
            }

            if !target_win.is_null() {
                PUM_ANCHOR_GRID = (*(*target_win).w_grid.target).handle;
                pum_win_row += (*target_win).w_grid.row_offset;
                cursor_col += (*target_win).w_grid.col_offset;
                if !ptr::eq((*target_win).w_grid.target, &raw const default_grid) {
                    pum_win_row += (*target_win).w_winrow;
                    cursor_col += (*target_win).w_wincol;
                    if !ui_has(kUIMultigrid) {
                        PUM_ANCHOR_GRID = DEFAULT_GRID_HANDLE;
                    } else {
                        PUM_WIN_ROW_OFFSET = (*target_win).w_winrow;
                        PUM_WIN_COL_OFFSET = (*target_win).w_wincol;
                    }
                }
            }

            if PUM_EXTERNAL {
                if array_changed {
                    let mut arena = Arena {
                        cur_blk: ptr::null_mut(),
                        pos: 0,
                        size: 0,
                    };
                    let mut arr = arena_array(&raw mut arena, size as usize);
                    for i in 0..(size as usize) {
                        let elem = &*array.add(i);
                        let mut item = arena_array(&raw mut arena, 4);
                        add_c(&mut item, cstr_as_obj(elem.pum_text));
                        add_c(&mut item, cstr_as_obj(elem.pum_kind));
                        add_c(&mut item, cstr_as_obj(elem.pum_extra));
                        add_c(&mut item, cstr_as_obj(elem.pum_info));
                        add_c(&mut arr, array_obj(item));
                    }
                    ui_call_popupmenu_show(
                        arr,
                        i64::from(selected),
                        i64::from(pum_win_row - PUM_WIN_ROW_OFFSET),
                        i64::from(cursor_col - PUM_WIN_COL_OFFSET),
                        i64::from(PUM_ANCHOR_GRID),
                    );
                    arena_mem_free(arena_finish(&raw mut arena));
                } else {
                    ui_call_popupmenu_select(i64::from(selected));
                    return;
                }
            }

            let mut pvwin: *mut win_T = ptr::null_mut();
            let mut wp = first_win_in_tab(curtab);
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_pvw != 0 {
                    pvwin = wp;
                    break;
                }
                wp = (*wp).w_next;
            }

            if !pvwin.is_null() {
                if (*pvwin).w_winrow < (*curwin).w_winrow {
                    above_row = (*pvwin).w_winrow + (*pvwin).w_height;
                } else if (*pvwin).w_winrow > (*curwin).w_winrow + (*curwin).w_height {
                    below_row = (*pvwin).w_winrow;
                }
            }

            pum_compute_vertical_placement(
                size,
                target_win,
                pum_win_row,
                above_row,
                below_row,
                border_width,
            );

            if border_width == 0 && (PUM_HEIGHT < 1 || (PUM_HEIGHT == 1 && size > 1)) {
                return;
            }

            PUM_ARRAY = array;
            PUM_SIZE = size;

            if PUM_EXTERNAL {
                return;
            }

            pum_compute_size();

            PUM_SCROLLBAR = if PUM_HEIGHT < size { 1 } else { 0 };

            pum_compute_horizontal_placement(target_win, cursor_col, border_width);

            if !(pum_set_selected(selected, redo_count) && {
                redo_count += 1;
                redo_count <= 2
            }) {
                break;
            }
        }

        pum_grid.zindex = if (State & MODE_CMDLINE) != 0 {
            kZIndexCmdlinePopupMenu
        } else {
            kZIndexPopupMenu
        };
        pum_redraw();
    }
}

/// Computes attributes of text on the popup menu (`pum_compute_text_attrs`).
unsafe fn pum_compute_text_attrs(text: *mut c_char, hlf: hlf_T, user_hlattr: c_int) -> *mut c_int {
    unsafe {
        if *text == NUL
            || (hlf != HLF_PSI && hlf != HLF_PNI)
            || (win_hl_attr(curwin, HLF_PMSI) == win_hl_attr(curwin, HLF_PSI)
                && win_hl_attr(curwin, HLF_PMNI) == win_hl_attr(curwin, HLF_PNI))
        {
            return ptr::null_mut();
        }

        let leader = if (State & MODE_CMDLINE) != 0 {
            cmdline_compl_pattern()
        } else {
            ins_compl_leader()
        };
        if leader.is_null() || *leader == NUL {
            return ptr::null_mut();
        }

        let attrs = xmalloc(size_of::<c_int>() * (vim_strsize(text) as usize)).cast::<c_int>();
        let in_fuzzy = if (State & MODE_CMDLINE) != 0 {
            cmdline_compl_is_fuzzy()
        } else {
            (get_cot_flags() & kOptCotFlagFuzzy) != 0
        };
        let leader_len = CStr::from_ptr(leader).to_bytes().len();

        let mut ga: *mut garray_T = ptr::null_mut();
        let mut matched_len: c_int = -1;

        if in_fuzzy {
            ga = fuzzy_match_str_with_pos(text, leader);
            if ga.is_null() {
                xfree(attrs.cast());
                return ptr::null_mut();
            }
        }

        let mut ptr = text as *const c_char;
        let mut cell_idx: usize = 0;
        let mut char_pos: u32 = 0;
        let is_select = hlf == HLF_PSI;

        while *ptr != NUL {
            let mut new_attr = win_hl_attr(curwin, hlf);

            if !ga.is_null() {
                for i in 0..((*ga).ga_len as usize) {
                    if char_pos == *((*ga).ga_data.cast::<u32>().add(i)) {
                        new_attr = win_hl_attr(curwin, if is_select { HLF_PMSI } else { HLF_PMNI });
                        new_attr = hl_combine_attr(win_hl_attr(curwin, HLF_PMNI), new_attr);
                        new_attr = hl_combine_attr(win_hl_attr(curwin, hlf), new_attr);
                        break;
                    }
                }
            } else {
                if matched_len < 0 && mb_strnicmp(ptr, leader, leader_len) == 0 {
                    matched_len = leader_len as c_int;
                }
                if matched_len > 0 {
                    new_attr = win_hl_attr(curwin, if is_select { HLF_PMSI } else { HLF_PMNI });
                    new_attr = hl_combine_attr(win_hl_attr(curwin, HLF_PMNI), new_attr);
                    new_attr = hl_combine_attr(win_hl_attr(curwin, hlf), new_attr);
                    matched_len -= 1;
                }
            }

            new_attr = hl_combine_attr(win_hl_attr(curwin, HLF_PNI), new_attr);

            if user_hlattr > 0 {
                new_attr = hl_combine_attr(new_attr, user_hlattr);
            }

            let char_cells = utf_ptr2cells(ptr) as usize;
            for i in 0..char_cells {
                *attrs.add(cell_idx + i) = new_attr;
            }
            cell_idx += char_cells;

            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
            char_pos += 1;
        }

        if !ga.is_null() {
            ga_clear(ga);
            xfree(ga.cast());
        }
        attrs
    }
}

/// Displays text on the popup menu with specific attributes (`pum_grid_puts_with_attrs`).
unsafe fn pum_grid_puts_with_attrs(
    mut col: c_int,
    cells: c_int,
    text: *const c_char,
    textlen: c_int,
    attrs: *const c_int,
) {
    unsafe {
        let col_start = col;
        let mut ptr = text;

        while *ptr != NUL && (textlen < 0 || ptr < text.add(textlen as usize)) {
            let char_len = utfc_ptr2len(ptr);
            let attr_idx = if PUM_RL {
                col_start + cells - col - 1
            } else {
                col - col_start
            };
            let attr = *attrs.add(attr_idx as usize);
            grid_line_puts(col, ptr, char_len, attr);
            col += utf_ptr2cells(ptr);
            ptr = ptr.add(char_len as usize);
        }
    }
}

#[inline]
unsafe fn pum_align_order(order: &mut [c_int; 3]) {
    unsafe {
        let is_default = cia_flags == 0;
        order[0] = if is_default {
            CPT_ABBR
        } else {
            (cia_flags / 100) as c_int
        };
        order[1] = if is_default {
            CPT_KIND
        } else {
            ((cia_flags / 10) % 10) as c_int
        };
        order[2] = if is_default {
            CPT_MENU
        } else {
            (cia_flags % 10) as c_int
        };
    }
}

#[inline]
unsafe fn pum_get_item(index: c_int, type_: c_int) -> *mut c_char {
    unsafe {
        let item = &*PUM_ARRAY.add(index as usize);
        match type_ {
            CPT_ABBR => item.pum_text,
            CPT_KIND => item.pum_kind,
            CPT_MENU => item.pum_extra,
            _ => ptr::null_mut(),
        }
    }
}

#[inline]
unsafe fn pum_user_attr_combine(idx: c_int, type_: c_int, attr: c_int) -> c_int {
    unsafe {
        let item = &*PUM_ARRAY.add(idx as usize);
        let user_attr = [item.pum_user_abbr_hlattr, item.pum_user_kind_hlattr];
        let u = user_attr[type_ as usize];
        if u > 0 {
            hl_combine_attr(attr, u)
        } else {
            attr
        }
    }
}

/// Redraw the popup menu, using `pum_first` and `pum_selected` (`pum_redraw`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_redraw() {
    unsafe {
        let mut row: c_int = 0;
        let attr_scroll = win_hl_attr(curwin, HLF_PSB);
        let attr_thumb = win_hl_attr(curwin, HLF_PST);
        let mut thumb_pos: c_int = 0;
        let mut thumb_height: c_int = 1;
        let fcs_trunc: schar_T = if PUM_RL {
            (*curwin).w_p_fcs_chars.truncrl
        } else {
            (*curwin).w_p_fcs_chars.trunc
        };

        const HLFS_NORM: [hlf_T; 3] = [HLF_PNI, HLF_PNK, HLF_PNX];
        const HLFS_SEL: [hlf_T; 3] = [HLF_PSI, HLF_PSK, HLF_PSX];

        let mut grid_width = PUM_WIDTH;
        let mut col_off: c_int = 0;
        let mut extra_space = false;
        if PUM_RL {
            col_off = PUM_WIDTH - 1;
            debug_assert!((State & MODE_CMDLINE) == 0);
            let win_end_col = (*curwin).w_wincol + (*curwin).w_view_width;
            if PUM_COL < win_end_col - 1 {
                grid_width += 1;
                extra_space = true;
            }
        } else {
            let min_col: c_int = 0;
            if PUM_COL > min_col {
                grid_width += 1;
                col_off = 1;
                extra_space = true;
            }
        }

        let mut fconfig = WIN_CONFIG_INIT;
        let border_width = pum_border_width();
        let mut border_attr: c_int = 0;
        let mut border_char: schar_T = 0;
        let fill_char: schar_T = schar_from_ascii(b' ');
        let has_border = border_width > 0;

        if border_width > 0 {
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            if !parse_winborder(&raw mut fconfig, p_pumborder, &raw mut err) {
                if err.err_type != ErrorType::kErrorTypeNone {
                    emsg(err.msg);
                }
                api_clear_error(&raw mut err);
                return;
            }

            if strequal(p_pumborder, opt_winborder_values[3]) {
                fconfig.shadow = true;
                let blend = syn_check_group(c"PmenuShadow".as_ptr(), 11);
                let through = syn_check_group(c"PmenuShadowThrough".as_ptr(), 18);
                fconfig.border_hl_ids[2] = through;
                fconfig.border_hl_ids[3] = blend;
                fconfig.border_hl_ids[4] = blend;
                fconfig.border_hl_ids[5] = blend;
                fconfig.border_hl_ids[6] = through;
            }

            for i in 0..8 {
                let mut attr = *hl_attr_active.add(HLF_PBR as usize);
                if fconfig.border_hl_ids[i] != 0 {
                    attr = hl_get_ui_attr(-1, HLF_PBR, fconfig.border_hl_ids[i], false);
                }
                fconfig.border_attr[i] = attr;
            }
            api_clear_error(&raw mut err);
            if PUM_SCROLLBAR != 0 {
                border_char = schar_from_str(fconfig.border_chars[3].as_ptr());
                border_attr = fconfig.border_attr[3];
            }
        }

        if PUM_SCROLLBAR > 0 && (!fconfig.border || fconfig.shadow) {
            grid_width += 1;
            if PUM_RL {
                col_off += 1;
            }
        }
        pum_grid.blending = p_pb > 0 || fconfig.shadow;
        grid_assign_handle(&raw mut pum_grid);

        PUM_LEFT_COL = PUM_COL - col_off;
        PUM_RIGHT_COL = PUM_LEFT_COL + grid_width;
        let moved = ui_comp_put_grid(
            &raw mut pum_grid,
            PUM_ROW,
            PUM_LEFT_COL,
            PUM_HEIGHT + border_width,
            grid_width + border_width,
            false,
            true,
        );
        let invalid_grid = moved || PUM_INVALID;
        PUM_INVALID = false;
        must_redraw_pum = false;

        if pum_grid.chars.is_null()
            || pum_grid.rows != PUM_HEIGHT + border_width
            || pum_grid.cols != grid_width + border_width
        {
            grid_alloc(
                &raw mut pum_grid,
                PUM_HEIGHT + border_width,
                grid_width + border_width,
                !invalid_grid,
                false,
            );
            ui_call_grid_resize(
                i64::from(pum_grid.handle),
                i64::from(pum_grid.cols),
                i64::from(pum_grid.rows),
            );
        } else if invalid_grid {
            grid_invalidate(&raw mut pum_grid);
        }
        if ui_has(kUIMultigrid) {
            pum_pos_ui_flush(true);
        }

        let scroll_range = PUM_SIZE - PUM_HEIGHT;
        if fconfig.border {
            grid_draw_border(
                &raw mut pum_grid,
                &raw mut fconfig,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                0,
            );
            if !fconfig.shadow {
                row += 1;
                col_off += 1;
            }
        }

        PUM_FIRST = PUM_FIRST.min(scroll_range);

        if PUM_SCROLLBAR != 0 {
            thumb_height = PUM_HEIGHT * PUM_HEIGHT / PUM_SIZE;
            if thumb_height == 0 {
                thumb_height = 1;
            }
            thumb_pos = (PUM_FIRST * (PUM_HEIGHT - thumb_height) + scroll_range / 2) / scroll_range;
        }

        for i in 0..PUM_HEIGHT {
            let idx = i + PUM_FIRST;
            let selected = idx == PUM_SELECTED;
            let hlfs = if selected { &HLFS_SEL } else { &HLFS_NORM };
            let trunc_attr = win_hl_attr(curwin, if selected { HLF_PSI } else { HLF_PNI });
            let mut hlf = hlfs[0];
            let mut attr = win_hl_attr(curwin, hlf);
            attr = hl_combine_attr(win_hl_attr(curwin, HLF_PNI), attr);

            screengrid_line_start(&raw mut pum_grid, row, 0);

            if extra_space {
                if PUM_RL {
                    grid_line_puts(col_off + 1, c" ".as_ptr(), 1, attr);
                } else {
                    grid_line_puts(col_off - 1, c" ".as_ptr(), 1, attr);
                }
            }

            let mut grid_col = col_off;
            let mut totwidth: c_int = 0;
            let mut need_fcs_trunc = false;
            let mut order = [0 as c_int; 3];
            let items_width_array = [PUM_BASE_WIDTH, PUM_KIND_WIDTH, PUM_EXTRA_WIDTH];
            pum_align_order(&mut order);
            let basic_width = items_width_array[order[0] as usize];
            let last_isabbr = order[2] == CPT_ABBR;
            let mut orig_attr: c_int = -1;

            for j in 0..3 {
                let item_type = order[j];
                hlf = hlfs[item_type as usize];
                attr = win_hl_attr(curwin, hlf);
                attr = hl_combine_attr(win_hl_attr(curwin, HLF_PNI), attr);
                orig_attr = attr;
                if item_type < 2 {
                    attr = pum_user_attr_combine(idx, item_type, attr);
                }
                let mut width: c_int = 0;
                let mut s: *mut c_char = ptr::null_mut();
                let mut p = pum_get_item(idx, item_type);

                let next_isempty = j + 1 >= 3 || pum_get_item(idx, order[j + 1]).is_null();

                if !p.is_null() {
                    loop {
                        if s.is_null() {
                            s = p;
                        }
                        let w = ptr2cells(p);
                        if *p != NUL && *p != TAB && totwidth + w <= PUM_WIDTH {
                            width += w;
                            p = p.add(utfc_ptr2len(p) as usize);
                            continue;
                        }
                        let width_limit = PUM_WIDTH;

                        let saved = *p;
                        if saved != NUL {
                            *p = NUL;
                        }
                        let st = transstr(s, true);
                        if saved != NUL {
                            *p = saved;
                        }

                        let mut attrs: *mut c_int = ptr::null_mut();
                        if item_type == CPT_ABBR {
                            attrs = pum_compute_text_attrs(
                                st,
                                hlf,
                                (*PUM_ARRAY.add(idx as usize)).pum_user_abbr_hlattr,
                            );
                        }

                        if PUM_RL {
                            let mut rt = reverse_text(st);
                            let rt_start = rt;
                            let mut cells = mb_string2cells(rt) as c_int;
                            let pad = if next_isempty { 0 } else { 2 };
                            if width_limit - totwidth < cells + pad {
                                need_fcs_trunc = true;
                            }

                            if grid_col - cells < col_off - width_limit {
                                loop {
                                    cells -= utf_ptr2cells(rt);
                                    rt = rt.add(utfc_ptr2len(rt) as usize);
                                    if grid_col - cells >= col_off - width_limit {
                                        break;
                                    }
                                }

                                if grid_col - cells > col_off - width_limit {
                                    rt = rt.sub(1);
                                    *rt = b'<' as c_char;
                                    cells += 1;
                                }
                            }

                            if attrs.is_null() {
                                grid_line_puts(grid_col - cells + 1, rt, -1, attr);
                            } else {
                                pum_grid_puts_with_attrs(
                                    grid_col - cells + 1,
                                    cells,
                                    rt,
                                    -1,
                                    attrs,
                                );
                            }

                            xfree(rt_start.cast());
                            xfree(st.cast());
                            grid_col -= width;
                        } else {
                            let mut cells = mb_string2cells(st) as c_int;
                            let pad = if next_isempty { 0 } else { 2 };
                            if width_limit - totwidth < cells + pad {
                                need_fcs_trunc = true;
                            }
                            if need_fcs_trunc {
                                let available_cells = width_limit - totwidth;
                                let mut p_end = st;
                                let mut displayed: c_int = 0;
                                while *p_end != NUL {
                                    let char_cells = utf_ptr2cells(p_end);
                                    if displayed + char_cells > available_cells {
                                        break;
                                    }
                                    displayed += char_cells;
                                    p_end = p_end.add(utfc_ptr2len(p_end) as usize);
                                }
                                *p_end = NUL;
                                cells = displayed;
                                width = displayed;
                            }

                            if attrs.is_null() {
                                grid_line_puts(grid_col, st, -1, attr);
                            } else {
                                pum_grid_puts_with_attrs(grid_col, cells, st, -1, attrs);
                            }

                            xfree(st.cast());
                            grid_col += width;
                        }

                        if !attrs.is_null() {
                            xfree(attrs.cast());
                        }

                        if *p != TAB {
                            break;
                        }

                        if PUM_RL {
                            grid_line_puts(grid_col - 1, c"  ".as_ptr(), 2, attr);
                            grid_col -= 2;
                        } else {
                            grid_line_puts(grid_col, c"  ".as_ptr(), 2, attr);
                            grid_col += 2;
                        }
                        totwidth += 2;
                        s = ptr::null_mut();
                        width = 0;
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                }

                let n = if j > 0 {
                    items_width_array[order[1] as usize] + if last_isabbr { 0 } else { 1 }
                } else if order[j] == CPT_ABBR {
                    1
                } else {
                    0
                };

                if j == 2
                    || (next_isempty
                        && (j == 1 || (j == 0 && pum_get_item(idx, order[j + 2]).is_null())))
                    || basic_width + n >= PUM_WIDTH
                {
                    break;
                }

                if PUM_RL {
                    grid_line_fill(
                        col_off - basic_width - n + 1,
                        grid_col + 1,
                        schar_from_ascii(b' '),
                        orig_attr,
                    );
                    grid_col = col_off - basic_width - n;
                } else {
                    grid_line_fill(
                        grid_col,
                        col_off + basic_width + n,
                        schar_from_ascii(b' '),
                        orig_attr,
                    );
                    grid_col = col_off + basic_width + n;
                }
                totwidth = basic_width + n;
            }

            if PUM_RL {
                let lcol = col_off - PUM_WIDTH + 1;
                grid_line_fill(lcol, grid_col + 1, schar_from_ascii(b' '), orig_attr);
                if need_fcs_trunc {
                    *linebuf_char.add(lcol as usize) = if fcs_trunc != 0 {
                        fcs_trunc
                    } else {
                        schar_from_ascii(b'<')
                    };
                    *linebuf_attr.add(lcol as usize) = trunc_attr;
                    if PUM_WIDTH > 1 && *linebuf_char.add((lcol + 1) as usize) == 0 {
                        *linebuf_char.add((lcol + 1) as usize) = schar_from_ascii(b' ');
                    }
                }
            } else {
                let rcol = col_off + PUM_WIDTH;
                grid_line_fill(grid_col, rcol, schar_from_ascii(b' '), orig_attr);
                if need_fcs_trunc {
                    if PUM_WIDTH > 1 && *linebuf_char.add((rcol - 1) as usize) == 0 {
                        *linebuf_char.add((rcol - 2) as usize) = schar_from_ascii(b' ');
                    }
                    *linebuf_char.add((rcol - 1) as usize) = if fcs_trunc != 0 {
                        fcs_trunc
                    } else {
                        schar_from_ascii(b'>')
                    };
                    *linebuf_attr.add((rcol - 1) as usize) = trunc_attr;
                }
            }

            if PUM_SCROLLBAR > 0 {
                let thumb = i >= thumb_pos && i < thumb_pos + thumb_height;
                let scrollbar_col = col_off + if PUM_RL { -PUM_WIDTH } else { PUM_WIDTH };
                let use_border_style = has_border && !fconfig.shadow;
                grid_line_put_schar(
                    scrollbar_col,
                    if use_border_style && !thumb {
                        border_char
                    } else {
                        fill_char
                    },
                    if thumb {
                        attr_thumb
                    } else if use_border_style {
                        border_attr
                    } else {
                        attr_scroll
                    },
                );
            }
            grid_line_flush();
            row += 1;
        }
    }
}

/// Set the informational text in the preview buffer (`pum_preview_set_text`).
unsafe fn pum_preview_set_text(
    win: *mut win_T,
    info: *mut c_char,
    lnum: *mut linenr_T,
    max_width: *mut c_int,
) {
    unsafe {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut replacement = Array {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let buf = (*win).w_buffer;
        (*buf).b_p_ma = 1;

        let mut curr = info;
        while !curr.is_null() {
            let next = strchr(curr, b'\n' as c_int);
            if !next.is_null() {
                *next = NUL;
            }
            if *curr == NUL && next.is_null() {
                break;
            }
            let save_wrap = (*win).w_onebuf_opt.wo_wrap;
            (*win).w_onebuf_opt.wo_wrap = 0;
            let line_width = win_linetabsize(win, 0, curr, MAXCOL);
            (*win).w_onebuf_opt.wo_wrap = save_wrap;
            *max_width = (*max_width).max(line_width);
            kv_add_obj(&mut replacement, string_obj(cstr_to_string(curr)));
            *lnum += 1;

            if !next.is_null() {
                *next = b'\n' as c_char;
                curr = next.add(1);
            } else {
                curr = ptr::null_mut();
            }
        }

        let original_textlock = textlock;
        textlock = 0;
        nvim_buf_set_lines(
            0,
            (*buf).b_fnum,
            0,
            -1,
            false,
            replacement,
            &raw mut arena,
            &raw mut err,
        );
        textlock = original_textlock;
        if err.err_type != ErrorType::kErrorTypeNone {
            emsg(err.msg);
            api_clear_error(&raw mut err);
        }
        arena_mem_free(arena_finish(&raw mut arena));
        api_free_array(replacement);
        (*buf).b_p_ma = 0;
    }
}

/// Adjust floating info preview window position (`pum_adjust_info_position`).
unsafe fn pum_adjust_info_position(wp: *mut win_T, width: c_int) -> bool {
    unsafe {
        let border_width = pum_border_width();
        let col = PUM_COL + PUM_WIDTH + 1 + border_width.max(PUM_SCROLLBAR);
        let right_extra = Columns - col;
        let left_extra = PUM_COL - 2;

        let max_extra = right_extra.max(left_extra);
        if max_extra < 10 {
            (*wp).w_config.hide = true;
            return false;
        }

        if right_extra > width {
            (*wp).w_config.width = width;
            (*wp).w_config.col = f64::from(col - 1);
        } else if left_extra > width {
            (*wp).w_config.width = width;
            (*wp).w_config.col = f64::from(PUM_COL - (*wp).w_config.width - 1);
        } else {
            let place_in_right = right_extra > left_extra;
            (*wp).w_config.width = max_extra;
            (*wp).w_config.col = f64::from(if place_in_right {
                col - 1
            } else {
                PUM_COL - (*wp).w_config.width - 1
            });
        }
        (*wp).w_config.anchor = 0;
        let count = (*(*wp).w_buffer).b_ml.ml_line_count;
        (*wp).w_view_width = (*wp).w_config.width;
        (*wp).w_config.height = plines_m_win(wp, (*wp).w_topline, count, Rows);
        (*wp).w_config.row = f64::from(PUM_ROW);
        (*wp).w_config.hide = false;
        win_config_float(wp, (*wp).w_config);
        true
    }
}

/// Used for `nvim__complete_set` (`pum_set_info`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_set_info(selected: c_int, info: *mut c_char) -> *mut win_T {
    unsafe {
        if !PUM_IS_VISIBLE || !compl_match_curr_select(selected) {
            return ptr::null_mut();
        }
        block_autocmds();
        RedrawingDisabled += 1;
        no_u_sync += 1;
        let mut wp = win_float_find(kWinInfo);
        if wp.is_null() {
            wp = win_float_special(false, true, kWinInfo);
            if wp.is_null() {
                return ptr::null_mut();
            }
            (*wp).w_topline = 1;
            (*wp).w_onebuf_opt.wo_wfb = 1;
        }
        let mut lnum: linenr_T = 0;
        let mut max_info_width: c_int = 0;
        pum_preview_set_text(wp, info, &raw mut lnum, &raw mut max_info_width);
        no_u_sync -= 1;
        RedrawingDisabled -= 1;
        redraw_later(wp, UPD_NOT_VALID);

        if !pum_adjust_info_position(wp, max_info_width) {
            wp = ptr::null_mut();
        }
        unblock_autocmds();
        wp
    }
}

/// Set the index of the currently selected item (`pum_set_selected`).
unsafe fn pum_set_selected(n: c_int, repeat: c_int) -> bool {
    unsafe {
        let mut resized = false;
        let mut context = PUM_HEIGHT / 2;
        let prev_selected = PUM_SELECTED;

        PUM_SELECTED = n;
        let scroll_offset = PUM_SELECTED - PUM_HEIGHT;
        let cur_cot_flags = get_cot_flags();
        let use_float = (cur_cot_flags & kOptCotFlagPopup) != 0;

        if use_float
            && (PUM_SELECTED < 0 || (*PUM_ARRAY.add(PUM_SELECTED as usize)).pum_info.is_null())
        {
            let wp = win_float_find(kWinInfo);
            if !wp.is_null() {
                (*wp).w_config.hide = true;
                win_config_float(wp, (*wp).w_config);
            }
        }

        if PUM_SELECTED >= 0 && PUM_SELECTED < PUM_SIZE {
            if PUM_FIRST > PUM_SELECTED - 4 {
                if PUM_FIRST > PUM_SELECTED - 2 {
                    PUM_FIRST -= PUM_HEIGHT - 2;
                    if PUM_FIRST < 0 {
                        PUM_FIRST = 0;
                    } else if PUM_FIRST > PUM_SELECTED {
                        PUM_FIRST = PUM_SELECTED;
                    }
                } else {
                    PUM_FIRST = PUM_SELECTED;
                }
            } else if PUM_FIRST < scroll_offset + 5 {
                if PUM_FIRST < scroll_offset + 3 {
                    PUM_FIRST = (PUM_FIRST + PUM_HEIGHT - 2).max(scroll_offset + 1);
                } else {
                    PUM_FIRST = scroll_offset + 1;
                }
            }

            context = context.min(3);

            if PUM_HEIGHT > 2 {
                if PUM_FIRST > PUM_SELECTED - context {
                    PUM_FIRST = (PUM_SELECTED - context).max(0);
                } else if PUM_FIRST < PUM_SELECTED + context - PUM_HEIGHT + 1 {
                    PUM_FIRST = PUM_SELECTED + context - PUM_HEIGHT + 1;
                }
            }
            PUM_FIRST = PUM_FIRST.min(PUM_SIZE - PUM_HEIGHT);

            if !(*PUM_ARRAY.add(PUM_SELECTED as usize)).pum_info.is_null()
                && Rows > 10
                && repeat <= 1
                && (cur_cot_flags & (kOptCotFlagPreview | kOptCotFlagPopup)) != 0
                && !((cur_cot_flags & kOptCotFlagPreview) != 0 && !cmdwin_buf.is_null())
            {
                let curwin_save = curwin;
                let curtab_save = curtab;

                if use_float {
                    block_autocmds();
                }

                g_do_tagpreview = 3;
                if p_pvh > 0 && p_pvh < i64::from(g_do_tagpreview) {
                    g_do_tagpreview = p_pvh as c_int;
                }
                RedrawingDisabled += 1;
                no_u_sync += 1;

                if !use_float {
                    resized = prepare_tagpreview(false, false);
                } else {
                    let mut wp = win_float_find(kWinInfo);
                    if !wp.is_null() {
                        win_enter(wp, false);
                    } else {
                        wp = win_float_special(true, true, kWinInfo);
                        if !wp.is_null() {
                            resized = true;
                        }
                    }
                }

                no_u_sync -= 1;
                RedrawingDisabled -= 1;
                g_do_tagpreview = 0;

                if (*curwin).w_onebuf_opt.wo_pvw != 0 || (*curwin).w_kind == kWinInfo {
                    let mut res = OK;
                    if !resized
                        && (*curbuf).b_nwindows == 1
                        && (*curbuf).b_fname.is_null()
                        && bt_nofile(curbuf)
                        && *(*curbuf).b_p_bh == b'w' as c_char
                    {
                        buf_clear();
                    } else {
                        no_u_sync += 1;
                        res = do_ecmd(
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ECMD_ONE,
                            0,
                            ptr::null_mut(),
                        );
                        no_u_sync -= 1;

                        if res == OK {
                            set_option_value_give_err(kOptSwapfile, boolean_obj(false), OPT_LOCAL);
                            set_option_value_give_err(kOptBuflisted, boolean_obj(false), OPT_LOCAL);
                            set_option_value_give_err(
                                kOptBuftype,
                                cstr_as_obj(c"nofile".as_ptr()),
                                OPT_LOCAL,
                            );
                            set_option_value_give_err(
                                kOptBufhidden,
                                cstr_as_obj(c"wipe".as_ptr()),
                                OPT_LOCAL,
                            );
                            set_option_value_give_err(kOptDiff, boolean_obj(false), OPT_LOCAL);
                        }
                    }

                    if res == OK {
                        let mut lnum: linenr_T = 0;
                        let mut max_info_width: c_int = 0;
                        pum_preview_set_text(
                            curwin,
                            (*PUM_ARRAY.add(PUM_SELECTED as usize)).pum_info,
                            &raw mut lnum,
                            &raw mut max_info_width,
                        );
                        if repeat == 0 && !use_float {
                            lnum = lnum.min(p_pvh as linenr_T);
                            if (*curwin).w_height < lnum {
                                win_setheight(lnum);
                                resized = true;
                            }
                        }

                        (*curbuf).b_changed = 0;
                        (*curbuf).b_p_ma = 0;
                        if PUM_SELECTED != prev_selected {
                            (*curwin).w_topline = 1;
                        } else if (*curwin).w_topline > (*curbuf).b_ml.ml_line_count {
                            (*curwin).w_topline = (*curbuf).b_ml.ml_line_count;
                        }
                        (*curwin).w_cursor.lnum = 1;
                        (*curwin).w_cursor.col = 0;

                        if use_float
                            && !pum_adjust_info_position(curwin, max_info_width)
                            && win_valid(curwin_save)
                        {
                            win_enter(curwin_save, false);
                        }

                        if (curwin != curwin_save && win_valid(curwin_save))
                            || (curtab != curtab_save && valid_tabpage(curtab_save))
                        {
                            if curtab != curtab_save && valid_tabpage(curtab_save) {
                                goto_tabpage_tp(curtab_save, false, false);
                            }

                            if ins_compl_active() && !resized {
                                (*curwin).w_redr_status = false;
                            }

                            validate_cursor(curwin);
                            redraw_later(curwin, UPD_SOME_VALID);

                            if resized && win_valid(curwin_save) {
                                no_u_sync += 1;
                                win_enter(curwin_save, true);
                                no_u_sync -= 1;
                                update_topline(curwin);
                            }

                            let save_pum_is_drawn = PUM_IS_DRAWN;

                            PUM_IS_VISIBLE = false;
                            PUM_IS_DRAWN = false;
                            update_screen();
                            PUM_IS_VISIBLE = true;
                            PUM_IS_DRAWN = save_pum_is_drawn;

                            if !resized && win_valid(curwin_save) {
                                no_u_sync += 1;
                                win_enter(curwin_save, true);
                                no_u_sync -= 1;
                            }

                            PUM_IS_VISIBLE = false;
                            PUM_IS_DRAWN = false;
                            update_screen();
                            PUM_IS_VISIBLE = true;
                            PUM_IS_DRAWN = save_pum_is_drawn;
                        }
                    }
                }

                if use_float {
                    unblock_autocmds();
                }
            }
        }

        resized
    }
}

/// Undisplay the popup menu (`pum_undisplay`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_undisplay(immediate: bool) {
    unsafe {
        PUM_IS_VISIBLE = false;
        PUM_ARRAY = ptr::null_mut();
        must_redraw_pum = false;

        if immediate {
            pum_check_clear();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_check_clear() {
    unsafe {
        if !PUM_IS_VISIBLE && PUM_IS_DRAWN {
            if PUM_EXTERNAL {
                ui_call_popupmenu_hide();
            } else {
                ui_comp_remove_grid(&raw mut pum_grid);
                if ui_has(kUIMultigrid) {
                    ui_call_win_close(i64::from(pum_grid.handle));
                    ui_call_grid_destroy(i64::from(pum_grid.handle));
                }
                grid_free(&raw mut pum_grid);
            }
            PUM_IS_DRAWN = false;
            PUM_EXTERNAL = false;
            win_float_close(kWinInfo);
        }
    }
}

/// Clear the popup menu (`pum_clear`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_clear() {
    unsafe {
        PUM_FIRST = 0;
    }
}

/// Return true if the popup menu is displayed (`pum_visible`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_visible() -> bool {
    unsafe { PUM_IS_VISIBLE }
}

/// Return true if the popup menu is displayed and drawn on the grid (`pum_drawn`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_drawn() -> bool {
    unsafe { pum_visible() && !PUM_EXTERNAL }
}

/// Screen was cleared, need to redraw next time (`pum_invalidate`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_invalidate() {
    unsafe {
        PUM_INVALID = true;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_recompose() {
    unsafe {
        ui_comp_compose_grid(&raw mut pum_grid);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_ext_select_item(item: c_int, insert: bool, finish: bool) {
    unsafe {
        if !pum_visible() || item < -1 || item >= PUM_SIZE {
            return;
        }
        pum_want.active = true;
        pum_want.item = item;
        pum_want.insert = insert;
        pum_want.finish = finish;
    }
}

/// Gets the height of the menu (`pum_get_height`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_get_height() -> c_int {
    unsafe {
        if PUM_EXTERNAL {
            let ui_pum_height = ui_pum_get_height();
            if ui_pum_height != 0 {
                return ui_pum_height;
            }
        }
        PUM_HEIGHT
    }
}

/// Add size information about the pum to `dict` (`pum_set_event_info`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_set_event_info(dict: *mut dict_T) {
    unsafe {
        if !pum_visible() {
            return;
        }
        let mut w: f64 = 0.0;
        let mut h: f64 = 0.0;
        let mut r: f64 = 0.0;
        let mut c: f64 = 0.0;
        if !ui_pum_get_pos(&raw mut w, &raw mut h, &raw mut r, &raw mut c) {
            w = f64::from(PUM_WIDTH);
            h = f64::from(PUM_HEIGHT);
            r = f64::from(PUM_ROW);
            c = f64::from(PUM_COL);
        }
        tv_dict_add_float(dict, c"height".as_ptr(), 6, h);
        tv_dict_add_float(dict, c"width".as_ptr(), 5, w);
        tv_dict_add_float(dict, c"row".as_ptr(), 3, r);
        tv_dict_add_float(dict, c"col".as_ptr(), 3, c);
        tv_dict_add_nr(dict, c"size".as_ptr(), 4, i64::from(PUM_SIZE));
        tv_dict_add_bool(
            dict,
            c"scrollbar".as_ptr(),
            9,
            if PUM_SCROLLBAR != 0 {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );
    }
}

unsafe fn pum_position_at_mouse(min_width: c_int) {
    unsafe {
        let min_row: c_int = 0;
        let min_col: c_int = 0;
        let mut max_row = Rows;
        let mut max_col = Columns;
        let mut grid = mouse_grid;
        let mut row = mouse_row;
        let mut col = mouse_col;
        PUM_WIN_ROW_OFFSET = 0;
        PUM_WIN_COL_OFFSET = 0;

        if ui_has(kUIMultigrid) && grid == 0 {
            mouse_find_win_outer(&raw mut grid, &raw mut row, &raw mut col);
        }
        if grid > 1 {
            let wp = get_win_by_grid_handle(grid);
            if !wp.is_null() {
                row += (*wp).w_winrow;
                col += (*wp).w_wincol;
                PUM_WIN_ROW_OFFSET = (*wp).w_winrow;
                PUM_WIN_COL_OFFSET = (*wp).w_wincol;

                if (*wp).w_view_height > 0 || (*wp).w_view_width > 0 {
                    max_row = (Rows - (*wp).w_winrow).max((*wp).w_winrow + (*wp).w_view_height);
                    max_col = (Columns - (*wp).w_wincol).max((*wp).w_wincol + (*wp).w_view_width);
                }
            }
        }
        if pum_grid.handle != 0 && grid == pum_grid.handle {
            row += PUM_ROW;
            col += PUM_LEFT_COL;
        } else {
            PUM_ANCHOR_GRID = grid;
        }

        let border_width = pum_border_width();
        let border_height = border_width;
        if max_row - row > PUM_SIZE + border_height || max_row - row > row - min_row {
            PUM_ABOVE = false;
            PUM_ROW = row + 1;
            if PUM_HEIGHT + border_height > max_row - PUM_ROW {
                PUM_HEIGHT = max_row - PUM_ROW - border_height;
            }
        } else {
            PUM_ABOVE = true;
            PUM_ROW = row - PUM_SIZE - border_height;
            if PUM_ROW < min_row {
                PUM_HEIGHT += PUM_ROW - min_row;
                PUM_ROW = min_row;
            }
        }

        if PUM_RL {
            if col - min_col + 1 >= PUM_BASE_WIDTH + border_width
                || col - min_col + 1 > min_width + border_width
            {
                PUM_COL = col;
            } else {
                PUM_COL =
                    min_col + (PUM_BASE_WIDTH + border_width).min(min_width + border_width) - 1;
            }
            PUM_WIDTH = PUM_COL - min_col + 1 - border_width;
        } else {
            if max_col - col >= PUM_BASE_WIDTH + border_width
                || max_col - col > min_width + border_width
            {
                PUM_COL = col;
            } else {
                PUM_COL = max_col - (PUM_BASE_WIDTH + border_width).min(min_width + border_width);
            }
            PUM_WIDTH = max_col - PUM_COL - border_width;
        }

        PUM_WIDTH = PUM_WIDTH.min(PUM_BASE_WIDTH + 1);
    }
}

/// Select the pum entry at the mouse position (`pum_select_mouse_pos`).
unsafe fn pum_select_mouse_pos() {
    unsafe {
        let mut grid = mouse_grid;
        let mut row = mouse_row;
        let mut col = mouse_col;

        if grid == 0 {
            mouse_find_win_outer(&raw mut grid, &raw mut row, &raw mut col);
        }

        if grid == pum_grid.handle {
            let border_offset = if pum_border_width() == 2 { 1 } else { 0 };
            let item = row - border_offset;
            PUM_SELECTED = if item >= 0 && item < PUM_HEIGHT {
                item
            } else {
                -1
            };
            return;
        }

        if grid != PUM_ANCHOR_GRID
            || col < PUM_LEFT_COL - PUM_WIN_COL_OFFSET
            || col >= PUM_RIGHT_COL - PUM_WIN_COL_OFFSET
        {
            PUM_SELECTED = -1;
            return;
        }

        let idx = row - (PUM_ROW - PUM_WIN_ROW_OFFSET);

        if idx < 0 || idx >= PUM_HEIGHT {
            PUM_SELECTED = -1;
        } else if *(*PUM_ARRAY.add(idx as usize)).pum_text != NUL {
            PUM_SELECTED = idx;
        }
    }
}

/// Execute the currently selected popup menu item (`pum_execute_menu`).
unsafe fn pum_execute_menu(menu: *mut vimmenu_T, mode: c_int) {
    unsafe {
        let mut idx: c_int = 0;
        let mut mp = (*menu).children;
        while !mp.is_null() {
            if ((*mp).modes & (*mp).enabled & mode) != 0 {
                let cur_idx = idx;
                idx += 1;
                if cur_idx == PUM_SELECTED {
                    let ea: exarg_T = std::mem::zeroed();
                    execute_menu(&raw const ea, mp, -1);
                    break;
                }
            }
            mp = (*mp).next;
        }
    }
}

/// Open the terminal version of the popup menu (`pum_show_popupmenu`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_show_popupmenu(menu: *mut vimmenu_T) {
    unsafe {
        pum_undisplay(true);
        PUM_SIZE = 0;
        let mode = get_menu_mode_flag();

        let mut mp = (*menu).children;
        while !mp.is_null() {
            if menu_is_separator((*mp).dname) || ((*mp).modes & (*mp).enabled & mode) != 0 {
                PUM_SIZE += 1;
            }
            mp = (*mp).next;
        }

        if PUM_SIZE <= 0 {
            emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
            return;
        }

        let mut idx: usize = 0;
        let array = xcalloc(PUM_SIZE as usize, size_of::<pumitem_T>()).cast::<pumitem_T>();

        mp = (*menu).children;
        while !mp.is_null() {
            let mut s: *const c_char = ptr::null();
            if menu_is_separator((*mp).dname) {
                s = c"".as_ptr();
            } else if ((*mp).modes & (*mp).enabled & mode) != 0 {
                s = (*mp).dname;
            }
            if !s.is_null() {
                (*array.add(idx)).pum_text = xstrdup(s);
                idx += 1;
            }
            mp = (*mp).next;
        }

        PUM_ARRAY = array;
        pum_compute_size();
        PUM_SCROLLBAR = 0;
        PUM_HEIGHT = PUM_SIZE;
        PUM_RL = (*curwin).w_onebuf_opt.wo_rl != 0;
        pum_position_at_mouse(20);

        PUM_SELECTED = -1;
        PUM_FIRST = 0;
        if p_mousemev == 0 {
            ui_call_option_set(
                NvimString {
                    data: c"mousemoveevent".as_ptr().cast_mut(),
                    size: 14,
                },
                boolean_obj(true),
            );
        }

        loop {
            PUM_IS_VISIBLE = true;
            PUM_IS_DRAWN = true;
            pum_grid.zindex = kZIndexCmdlinePopupMenu;
            pum_redraw();
            setcursor_mayforce(curwin, true);

            let c = vgetc();

            if c == ESC || c == Ctrl_C || PUM_ARRAY.is_null() {
                break;
            } else if c == CAR || c == NL {
                pum_execute_menu(menu, mode);
                break;
            } else if c == b'k' as c_int || c == K_UP || c == K_MOUSEUP {
                while PUM_SELECTED > 0 {
                    PUM_SELECTED -= 1;
                    if *(*array.add(PUM_SELECTED as usize)).pum_text != NUL {
                        break;
                    }
                }
            } else if c == b'j' as c_int || c == K_DOWN || c == K_MOUSEDOWN {
                while PUM_SELECTED < PUM_SIZE - 1 {
                    PUM_SELECTED += 1;
                    if *(*array.add(PUM_SELECTED as usize)).pum_text != NUL {
                        break;
                    }
                }
            } else if c == K_RIGHTMOUSE {
                vungetc(c);
                break;
            } else if c == K_LEFTDRAG || c == K_RIGHTDRAG || c == K_MOUSEMOVE {
                pum_select_mouse_pos();
            } else if c == K_LEFTMOUSE || c == K_LEFTMOUSE_NM || c == K_RIGHTRELEASE {
                pum_select_mouse_pos();
                if PUM_SELECTED >= 0 {
                    pum_execute_menu(menu, mode);
                    break;
                }
                if c == K_LEFTMOUSE || c == K_LEFTMOUSE_NM {
                    break;
                }
            }
        }

        for i in 0..(PUM_SIZE as usize) {
            xfree((*array.add(i)).pum_text.cast());
        }
        xfree(array.cast());
        pum_undisplay(true);
        if p_mousemev == 0 {
            ui_call_option_set(
                NvimString {
                    data: c"mousemoveevent".as_ptr().cast_mut(),
                    size: 14,
                },
                boolean_obj(false),
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_make_popup(path_name: *const c_char, use_mouse_pos: c_int) {
    unsafe {
        if use_mouse_pos == 0 {
            mouse_row = (*curwin).w_grid.row_offset + (*curwin).w_wrow;
            mouse_col = (*curwin).w_grid.col_offset
                + if (*curwin).w_onebuf_opt.wo_rl != 0 {
                    (*curwin).w_view_width - (*curwin).w_wcol - 1
                } else {
                    (*curwin).w_wcol
                };
            if ui_has(kUIMultigrid) {
                mouse_grid = (*(*curwin).w_grid.target).handle;
            } else if !ptr::eq((*curwin).w_grid.target, &raw const default_grid) {
                mouse_grid = 0;
                mouse_row += (*curwin).w_winrow;
                mouse_col += (*curwin).w_wincol;
            }
        }

        let menu = menu_find(path_name);
        if !menu.is_null() {
            pum_show_popupmenu(menu);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pum_pos_ui_flush(force: bool) {
    unsafe {
        if force
            || (ui_has(kUIMultigrid)
                && PUM_IS_DRAWN
                && !PUM_EXTERNAL
                && pum_grid.handle != 0
                && pum_grid.pending_comp_index_update)
        {
            let anchor = if PUM_ABOVE {
                c"SW".as_ptr()
            } else {
                c"NW".as_ptr()
            };
            let row_off = if PUM_ABOVE { -PUM_HEIGHT } else { 0 };
            ui_call_win_float_pos(
                i64::from(pum_grid.handle),
                -1,
                cstr_as_string(anchor),
                i64::from(PUM_ANCHOR_GRID),
                f64::from(PUM_ROW - row_off - PUM_WIN_ROW_OFFSET),
                f64::from(PUM_LEFT_COL - PUM_WIN_COL_OFFSET),
                false,
                i64::from(pum_grid.zindex),
                pum_grid.comp_index as i64,
                i64::from(pum_grid.comp_row),
                i64::from(pum_grid.comp_col),
            );
            pum_grid.pending_comp_index_update = false;
        }
    }
}
