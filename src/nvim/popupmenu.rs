//! Popup menu (PUM) (`src/nvim/popupmenu.c`).

use crate::api::buffer::nvim_buf_set_lines;
use crate::api::private::helpers::{
    add_c, api_clear_error, api_free_array, arena_array, array_obj, boolean_obj, cstr_as_obj,
    cstr_as_string, string_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString};
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
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::menu::{execute_menu, get_menu_mode_flag, menu_find, menu_is_separator, vimmenu_T};
use crate::message::emsg;
use crate::mouse::mouse_find_win_outer;
use crate::r#move::{update_topline, validate_cheight, validate_cursor, validate_cursor_col};
use crate::option::{
    OPT_LOCAL, kOptBufhidden, kOptBuflisted, kOptBuftype, kOptSwapfile, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::plines::{plines_m_win, win_linetabsize};
use crate::strings::{gettext, reverse_text};
use crate::ui::{
    ui_call_grid_destroy, ui_call_grid_resize, ui_call_option_set, ui_call_popupmenu_hide,
    ui_call_popupmenu_select, ui_call_popupmenu_show, ui_call_win_close, ui_call_win_float_pos,
    ui_has, ui_pum_get_height, ui_pum_get_pos,
};
use crate::ui_compositor::{ui_comp_put_grid, ui_comp_remove_grid};
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
#[derive(Clone, Default)]
pub struct pumitem_T {
    pub pum_text: Option<std::ffi::CString>,
    pub pum_kind: Option<std::ffi::CString>,
    pub pum_extra: Option<std::ffi::CString>,
    pub pum_info: Option<std::ffi::CString>,
    pub pum_cpt_source_idx: c_int,
    pub pum_user_abbr_hlattr: c_int,
    pub pum_user_kind_hlattr: c_int,
    pub pum_comp_idx: Option<usize>,
}

impl pumitem_T {
    pub const fn new() -> Self {
        Self {
            pum_text: None,
            pum_kind: None,
            pum_extra: None,
            pum_info: None,
            pum_cpt_source_idx: 0,
            pum_user_abbr_hlattr: 0,
            pum_user_kind_hlattr: 0,
            pum_comp_idx: None,
        }
    }

    #[inline]
    pub fn text_ptr(&self) -> *mut c_char {
        self.pum_text
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn kind_ptr(&self) -> *mut c_char {
        self.pum_kind
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn extra_ptr(&self) -> *mut c_char {
        self.pum_extra
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }

    #[inline]
    pub fn info_ptr(&self) -> *mut c_char {
        self.pum_info
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
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

const kOptCotFlagPreview: c_uint = 0x1000;

struct PopupmenuState {
    array: *mut pumitem_T,
    size: c_int,
    selected: c_int,
    first: c_int,
    height: c_int,
    width: c_int,
    base_width: c_int,
    kind_width: c_int,
    extra_width: c_int,
    scrollbar: c_int,
    rl: bool,
    anchor_grid: c_int,
    row: c_int,
    col: c_int,
    win_row_offset: c_int,
    win_col_offset: c_int,
    left_col: c_int,
    right_col: c_int,
    above: bool,
    is_visible: bool,
    is_drawn: bool,
    external: bool,
    invalid: bool,
}

impl PopupmenuState {
    const fn new() -> Self {
        Self {
            array: ptr::null_mut(),
            size: 0,
            selected: 0,
            first: 0,
            height: 0,
            width: 0,
            base_width: 0,
            kind_width: 0,
            extra_width: 0,
            scrollbar: 0,
            rl: false,
            anchor_grid: 0,
            row: 0,
            col: 0,
            win_row_offset: 0,
            win_col_offset: 0,
            left_col: 0,
            right_col: 0,
            above: false,
            is_visible: false,
            is_drawn: false,
            external: false,
            invalid: false,
        }
    }
}

static PUM: GlobalState<PopupmenuState> = GlobalState::new(PopupmenuState::new());

#[inline]
fn pum() -> &'static mut PopupmenuState {
    unsafe { &mut *PUM.as_ptr() }
}

use crate::api::private::helpers::cstr_to_string;

fn pum_compute_size() {
    unsafe {
        pum().base_width = 0;
        pum().kind_width = 0;
        pum().extra_width = 0;
        for i in 0..(pum().size as usize) {
            let item = &*pum().array.add(i);
            if let Some(ref text) = item.pum_text {
                let w = vim_strsize(text.as_ptr().cast_mut());
                if pum().base_width < w {
                    pum().base_width = w;
                }
            }
            if let Some(ref kind) = item.pum_kind {
                let w = vim_strsize(kind.as_ptr().cast_mut()) + 1;
                if pum().kind_width < w {
                    pum().kind_width = w;
                }
            }
            if let Some(ref extra) = item.pum_extra {
                let w = vim_strsize(extra.as_ptr().cast_mut()) + 1;
                if pum().extra_width < w {
                    pum().extra_width = w;
                }
            }
        }
    }
}

/// Calculate vertical placement for popup menu (`pum_compute_vertical_placement`).
fn pum_compute_vertical_placement(
    size: c_int,
    target_win: *mut win_T,
    pum_win_row: c_int,
    above_row: c_int,
    below_row: c_int,
    pum_border_size: c_int,
) {
    unsafe {
        let context_lines: c_int;

        pum().height = size.min(PUM_DEF_HEIGHT);
        if opt_vars().p_ph > 0 && i64::from(pum().height) > opt_vars().p_ph {
            pum().height = opt_vars().p_ph as c_int;
        }

        if pum_win_row + 2 + pum_border_size >= below_row - pum().height
            && pum_win_row - above_row > (below_row - above_row) / 2
        {
            pum().above = true;

            if (g_st().State & MODE_CMDLINE) != 0 && target_win.is_null() {
                context_lines = 0;
            } else {
                context_lines = 2.min((*target_win).w_wrow - (*target_win).w_cline_row);
            }

            if pum_win_row >= size + context_lines {
                pum().row = pum_win_row - size - context_lines;
                pum().height = size;
            } else {
                pum().row = 0;
                pum().height = pum_win_row - context_lines;
            }
            if opt_vars().p_ph > 0 && i64::from(pum().height) > opt_vars().p_ph {
                pum().row += pum().height - (opt_vars().p_ph as c_int);
                pum().height = opt_vars().p_ph as c_int;
            }

            if pum_border_size > 0 && pum_border_size + pum().row + pum().height >= pum_win_row {
                if pum().row < 2 {
                    pum().height -= pum_border_size;
                } else {
                    pum().row -= pum_border_size;
                }
            }
        } else {
            pum().above = false;

            if (g_st().State & MODE_CMDLINE) != 0 && target_win.is_null() {
                context_lines = 0;
            } else {
                validate_cheight(target_win);
                let cline_visible_offset =
                    (*target_win).w_cline_row + (*target_win).w_cline_height - (*target_win).w_wrow;
                context_lines = 3.min(cline_visible_offset);
            }

            pum().row = pum_win_row + context_lines;
            pum().height = (below_row - pum().row).min(size);
            if opt_vars().p_ph > 0 && i64::from(pum().height) > opt_vars().p_ph {
                pum().height = opt_vars().p_ph as c_int;
            }

            if pum().row + pum().height + pum_border_size >= g_st().cmdline_row {
                pum().height -= pum_border_size;
            }
        }

        if above_row > 0 && pum().row < above_row && pum().height > above_row {
            pum().row = above_row;
            pum().height = pum_win_row - above_row;
        }
    }
}

/// Try to set `pum_width` so that it fits within `available_width` (`set_pum_width_aligned_with_cursor`).
fn set_pum_width_aligned_with_cursor(mut width: c_int, available_width: c_int) -> bool {
    let mut end_padding = true;

        if i64::from(width) < opt_vars().p_pw {
            width = opt_vars().p_pw as c_int;
            end_padding = false;
        }
        if opt_vars().p_pmw > 0 && i64::from(width) > opt_vars().p_pmw {
            width = opt_vars().p_pmw as c_int;
            end_padding = false;
        }

        pum().width = width
            + if end_padding && i64::from(width) >= opt_vars().p_pw {
                1
            } else {
                0
            };
        available_width >= pum().width
}

/// Calculate horizontal placement for popup menu (`pum_compute_horizontal_placement`).
fn pum_compute_horizontal_placement(
    target_win: *mut win_T,
    cursor_col: c_int,
    border_width: c_int,
) {
    unsafe {
        let max_col = g_st().Columns.max(if !target_win.is_null() {
            (*target_win).w_wincol + (*target_win).w_view_width
        } else {
            0
        });
        let desired_width = pum().base_width + pum().kind_width + pum().extra_width;
        let mut available_width = if pum().rl {
            cursor_col - pum().scrollbar + 1 - border_width
        } else {
            max_col - cursor_col - pum().scrollbar - border_width
        };

        pum().col = cursor_col;
        if set_pum_width_aligned_with_cursor(desired_width, available_width) {
            return;
        }

        if i64::from(available_width) > opt_vars().p_pw {
            pum().width = available_width;
            return;
        }

        if pum().rl {
            available_width = max_col - pum().scrollbar - border_width;
        } else {
            available_width += cursor_col;
        }

        if i64::from(available_width) > opt_vars().p_pw {
            pum().width = (opt_vars().p_pw as c_int) + 1;
            if pum().rl {
                pum().col = pum().width + pum().scrollbar + border_width;
            } else {
                pum().col = max_col - pum().width - pum().scrollbar - border_width;
            }
            return;
        }

        if pum().rl {
            pum().col = max_col - 1;
        } else {
            pum().col = 0;
        }
        pum().width = max_col - pum().scrollbar - border_width;
    }
}

#[inline]
fn pum_border_width() -> c_int {
    unsafe {
        if *opt_vars().p_pumborder == NUL || (crate::memory::cstr_as_opt_bytes(opt_vars().p_pumborder) == crate::memory::cstr_as_opt_bytes(opt_winborder_values[7].as_ptr())) {
            0
        } else if crate::memory::cstr_as_opt_bytes(opt_vars().p_pumborder) == crate::memory::cstr_as_opt_bytes(opt_winborder_values[3].as_ptr()) {
            1
        } else {
            2
        }
    }
}

/// Show the popup menu with items `array[size]` (`pum_display`).
pub fn pum_display(
    array: *mut pumitem_T,
    size: c_int,
    selected: c_int,
    array_changed: bool,
    cmd_startcol: c_int,
) {
    unsafe {
        let mut redo_count: c_int = 0;

        if !pum().is_visible {
            pum().external =
                ui_has(kUIPopupmenu) || ((g_st().State & MODE_CMDLINE) != 0 && ui_has(kUIWildmenu));
        }

        pum().rl = (g_st().State & MODE_CMDLINE) == 0 && (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0;

        let border_width = pum_border_width();
        loop {
            pum().is_visible = true;
            pum().is_drawn = true;
            validate_cursor_col(&mut **curwin.as_ptr());
            let mut above_row: c_int = 0;
            let mut below_row = g_st().cmdline_row.max((**curwin.as_ptr()).w_winrow + (**curwin.as_ptr()).w_view_height);
            if (g_st().State & MODE_CMDLINE) != 0 {
                below_row = g_st().cmdline_row;
            }
            let target_win = if (g_st().State & MODE_CMDLINE) != 0 {
                g_st().cmdline_win
            } else {
                *curwin.as_ptr()
            };
            pum().win_row_offset = 0;
            pum().win_col_offset = 0;

            let mut pum_win_row: c_int;
            let mut cursor_col: c_int;

            if (g_st().State & MODE_CMDLINE) != 0 {
                pum_win_row = if !g_st().cmdline_win.is_null() {
                    (*g_st().cmdline_win).w_wrow
                } else if ui_has(kUICmdline) {
                    0
                } else {
                    g_st().cmdline_row
                };
                cursor_col = (if !g_st().cmdline_win.is_null() {
                    (*g_st().cmdline_win).w_config._cmdline_offset
                } else {
                    0
                }) + cmd_startcol;
                cursor_col %= if !g_st().cmdline_win.is_null() {
                    (*g_st().cmdline_win).w_view_width
                } else {
                    g_st().Columns
                };
                pum().anchor_grid = if ui_has(kUICmdline) {
                    -1
                } else {
                    DEFAULT_GRID_HANDLE
                };
            } else {
                pum_win_row = (**curwin.as_ptr()).w_wrow;
                let mut wcol = (**curwin.as_ptr()).w_wcol;
                if (**curwin.as_ptr()).w_onebuf_opt.wo_cole > 0 && conceal_cursor_line(*curwin.as_ptr()) {
                    wcol -= (**curwin.as_ptr()).w_wcol_conceal_off;
                    if wcol < 0 {
                        wcol = 0;
                    }
                }
                if pum().rl {
                    cursor_col = (**curwin.as_ptr()).w_view_width - wcol - 1;
                } else {
                    cursor_col = wcol;
                }
            }

            if !target_win.is_null() {
                pum().anchor_grid = (*(*target_win).w_grid.target).handle;
                pum_win_row += (*target_win).w_grid.row_offset;
                cursor_col += (*target_win).w_grid.col_offset;
                if !ptr::eq((*target_win).w_grid.target, &raw const g_st().default_grid) {
                    pum_win_row += (*target_win).w_winrow;
                    cursor_col += (*target_win).w_wincol;
                    if !ui_has(kUIMultigrid) {
                        pum().anchor_grid = DEFAULT_GRID_HANDLE;
                    } else {
                        pum().win_row_offset = (*target_win).w_winrow;
                        pum().win_col_offset = (*target_win).w_wincol;
                    }
                }
            }

            if pum().external {
                if array_changed {
                    let mut arena = Arena {
                        cur_blk: ptr::null_mut(),
                        pos: 0,
                        size: 0,
                    };
                    let mut arr = arena_array(&mut arena, size as usize);
                    for i in 0..(size as usize) {
                        let elem = &*array.add(i);
                        let mut item = arena_array(&mut arena, 4);
                        add_c(&mut item, cstr_as_obj(elem.text_ptr()));
                        add_c(&mut item, cstr_as_obj(elem.kind_ptr()));
                        add_c(&mut item, cstr_as_obj(elem.extra_ptr()));
                        add_c(&mut item, cstr_as_obj(elem.info_ptr()));
                        add_c(&mut arr, array_obj(item));
                    }
                    ui_call_popupmenu_show(
                        arr,
                        i64::from(selected),
                        i64::from(pum_win_row - pum().win_row_offset),
                        i64::from(cursor_col - pum().win_col_offset),
                        i64::from(pum().anchor_grid),
                    );
                    arena_mem_free(arena_finish(&mut arena));
                } else {
                    ui_call_popupmenu_select(i64::from(selected));
                    return;
                }
            }

            let mut pvwin: *mut win_T = ptr::null_mut();
            let mut wp = first_win_in_tab(g_st().curtab);
            while !wp.is_null() {
                if (*wp).w_onebuf_opt.wo_pvw != 0 {
                    pvwin = wp;
                    break;
                }
                wp = (*wp).w_next;
            }

            if !pvwin.is_null() {
                if (*pvwin).w_winrow < (**curwin.as_ptr()).w_winrow {
                    above_row = (*pvwin).w_winrow + (*pvwin).w_height;
                } else if (*pvwin).w_winrow > (**curwin.as_ptr()).w_winrow + (**curwin.as_ptr()).w_height {
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

            if border_width == 0 && (pum().height < 1 || (pum().height == 1 && size > 1)) {
                return;
            }

            pum().array = array;
            pum().size = size;

            if pum().external {
                return;
            }

            pum_compute_size();

            pum().scrollbar = if pum().height < size { 1 } else { 0 };

            pum_compute_horizontal_placement(target_win, cursor_col, border_width);

            if !(pum_set_selected(selected, redo_count) && {
                redo_count += 1;
                redo_count <= 2
            }) {
                break;
            }
        }

        g_st().pum_grid.zindex = if (g_st().State & MODE_CMDLINE) != 0 {
            kZIndexCmdlinePopupMenu
        } else {
            kZIndexPopupMenu
        };
        pum_redraw();
    }
}

/// Computes attributes of text on the popup menu (`pum_compute_text_attrs`).
fn pum_compute_text_attrs(
    text: *mut c_char,
    hlf: hlf_T,
    user_hlattr: c_int,
) -> Option<Vec<c_int>> {
    unsafe {
        if *text == NUL
            || (hlf != HLF_PSI && hlf != HLF_PNI)
            || (win_hl_attr(*curwin.as_ptr(), HLF_PMSI) == win_hl_attr(*curwin.as_ptr(), HLF_PSI)
                && win_hl_attr(*curwin.as_ptr(), HLF_PMNI) == win_hl_attr(*curwin.as_ptr(), HLF_PNI))
        {
            return None;
        }

        let leader = if (g_st().State & MODE_CMDLINE) != 0 {
            cmdline_compl_pattern()
        } else {
            ins_compl_leader()
        };
        if leader.is_null() || *leader == NUL {
            return None;
        }

        let in_fuzzy = if (g_st().State & MODE_CMDLINE) != 0 {
            cmdline_compl_is_fuzzy()
        } else {
            (get_cot_flags() & kOptCotFlagFuzzy) != 0
        };
        let leader_len = CStr::from_ptr(leader).to_bytes().len();

        let mut match_positions: Option<Vec<u32>> = None;
        let mut matched_len: c_int = -1;

        if in_fuzzy {
            match_positions = fuzzy_match_str_with_pos(text, leader);
            match_positions.as_ref()?;
        }

        let mut attrs = vec![0; vim_strsize(text) as usize];
        let mut ptr = text as *const c_char;
        let mut cell_idx: usize = 0;
        let mut char_pos: u32 = 0;
        let is_select = hlf == HLF_PSI;

        while *ptr != NUL {
            let mut new_attr = win_hl_attr(*curwin.as_ptr(), hlf);

            if let Some(ref positions) = match_positions {
                if positions.contains(&char_pos) {
                    new_attr = win_hl_attr(*curwin.as_ptr(), if is_select { HLF_PMSI } else { HLF_PMNI });
                    new_attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), HLF_PMNI), new_attr);
                    new_attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), hlf), new_attr);
                }
            } else {
                if matched_len < 0 && mb_strnicmp(ptr, leader, leader_len) == 0 {
                    matched_len = leader_len as c_int;
                }
                if matched_len > 0 {
                    new_attr = win_hl_attr(*curwin.as_ptr(), if is_select { HLF_PMSI } else { HLF_PMNI });
                    new_attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), HLF_PMNI), new_attr);
                    new_attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), hlf), new_attr);
                    matched_len -= 1;
                }
            }

            new_attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), HLF_PNI), new_attr);

            if user_hlattr > 0 {
                new_attr = hl_combine_attr(new_attr, user_hlattr);
            }

            let char_cells = utf_ptr2cells(ptr) as usize;
            for i in 0..char_cells {
                attrs[cell_idx + i] = new_attr;
            }
            cell_idx += char_cells;

            ptr = ptr.add(utfc_ptr2len(ptr) as usize);
            char_pos += 1;
        }

        Some(attrs)
    }
}

/// Displays text on the popup menu with specific attributes (`pum_grid_puts_with_attrs`).
fn pum_grid_puts_with_attrs(
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
            let attr_idx = if pum().rl {
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
fn pum_align_order(order: &mut [c_int; 3]) {
    let is_default = opt_vars().cia_flags == 0;
        order[0] = if is_default {
            CPT_ABBR
        } else {
            (opt_vars().cia_flags / 100) as c_int
        };
        order[1] = if is_default {
            CPT_KIND
        } else {
            ((opt_vars().cia_flags / 10) % 10) as c_int
        };
        order[2] = if is_default {
            CPT_MENU
        } else {
            (opt_vars().cia_flags % 10) as c_int
        };
}

#[inline]
fn pum_get_item(index: c_int, type_: c_int) -> *mut c_char {
    unsafe {
        let item = &*pum().array.add(index as usize);
        match type_ {
            CPT_ABBR => item.text_ptr(),
            CPT_KIND => item.kind_ptr(),
            CPT_MENU => item.extra_ptr(),
            _ => ptr::null_mut(),
        }
    }
}

#[inline]
fn pum_user_attr_combine(idx: c_int, type_: c_int, attr: c_int) -> c_int {
    unsafe {
        let item = &*pum().array.add(idx as usize);
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
pub fn pum_redraw() {
    unsafe {
        let mut row: c_int = 0;
        let attr_scroll = win_hl_attr(*curwin.as_ptr(), HLF_PSB);
        let attr_thumb = win_hl_attr(*curwin.as_ptr(), HLF_PST);
        let mut thumb_pos: c_int = 0;
        let mut thumb_height: c_int = 1;
        let fcs_trunc: schar_T = if pum().rl {
            (**curwin.as_ptr()).w_p_fcs_chars.truncrl
        } else {
            (**curwin.as_ptr()).w_p_fcs_chars.trunc
        };

        const HLFS_NORM: [hlf_T; 3] = [HLF_PNI, HLF_PNK, HLF_PNX];
        const HLFS_SEL: [hlf_T; 3] = [HLF_PSI, HLF_PSK, HLF_PSX];

        let mut grid_width = pum().width;
        let mut col_off: c_int = 0;
        let mut extra_space = false;
        if pum().rl {
            col_off = pum().width - 1;
            debug_assert!((g_st().State & MODE_CMDLINE) == 0);
            let win_end_col = (**curwin.as_ptr()).w_wincol + (**curwin.as_ptr()).w_view_width;
            if pum().col < win_end_col - 1 {
                grid_width += 1;
                extra_space = true;
            }
        } else {
            let min_col: c_int = 0;
            if pum().col > min_col {
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
            let mut err = Error::new();
            if !parse_winborder(&raw mut fconfig, opt_vars().p_pumborder, &mut err) {
                if err.err_type != ErrorType::kErrorTypeNone {
                    emsg(err.msg_ptr());
                }
                api_clear_error(&mut err);
                return;
            }

            if crate::memory::cstr_as_opt_bytes(opt_vars().p_pumborder) == crate::memory::cstr_as_opt_bytes(opt_winborder_values[3].as_ptr()) {
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
                let mut attr = crate::highlight::hl_attr(HLF_PBR);
                if fconfig.border_hl_ids[i] != 0 {
                    attr = hl_get_ui_attr(-1, HLF_PBR, fconfig.border_hl_ids[i], false);
                }
                fconfig.border_attr[i] = attr;
            }
            api_clear_error(&mut err);
            if pum().scrollbar != 0 {
                border_char = schar_from_str(fconfig.border_chars[3].as_ptr());
                border_attr = fconfig.border_attr[3];
            }
        }

        if pum().scrollbar > 0 && (!fconfig.border || fconfig.shadow) {
            grid_width += 1;
            if pum().rl {
                col_off += 1;
            }
        }
        g_st().pum_grid.blending = opt_vars().p_pb > 0 || fconfig.shadow;
        grid_assign_handle(&mut g_st().pum_grid);

        pum().left_col = pum().col - col_off;
        pum().right_col = pum().left_col + grid_width;
        let moved = ui_comp_put_grid(
            &mut g_st().pum_grid,
            pum().row,
            pum().left_col,
            pum().height + border_width,
            grid_width + border_width,
            false,
            true,
        );
        let invalid_grid = moved || pum().invalid;
        pum().invalid = false;
        g_st().must_redraw_pum = false;

        if !crate::grid::grid_allocated(&g_st().pum_grid)
            || g_st().pum_grid.rows != pum().height + border_width
            || g_st().pum_grid.cols != grid_width + border_width
        {
            grid_alloc(
                &mut g_st().pum_grid,
                pum().height + border_width,
                grid_width + border_width,
                !invalid_grid,
                false,
            );
            ui_call_grid_resize(
                i64::from(g_st().pum_grid.handle),
                i64::from(g_st().pum_grid.cols),
                i64::from(g_st().pum_grid.rows),
            );
        } else if invalid_grid {
            grid_invalidate(&raw mut g_st().pum_grid);
        }
        if ui_has(kUIMultigrid) {
            pum_pos_ui_flush(true);
        }

        let scroll_range = pum().size - pum().height;
        if fconfig.border {
            grid_draw_border(
                &mut g_st().pum_grid,
                &mut fconfig,
                ptr::null_mut(),
                0,
                None,
                0,
            );
            if !fconfig.shadow {
                row += 1;
                col_off += 1;
            }
        }

        pum().first = pum().first.min(scroll_range);

        if pum().scrollbar != 0 {
            thumb_height = pum().height * pum().height / pum().size;
            if thumb_height == 0 {
                thumb_height = 1;
            }
            thumb_pos = (pum().first * (pum().height - thumb_height) + scroll_range / 2) / scroll_range;
        }

        for i in 0..pum().height {
            let idx = i + pum().first;
            let selected = idx == pum().selected;
            let hlfs = if selected { &HLFS_SEL } else { &HLFS_NORM };
            let trunc_attr = win_hl_attr(*curwin.as_ptr(), if selected { HLF_PSI } else { HLF_PNI });
            let mut hlf = hlfs[0];
            let mut attr = win_hl_attr(*curwin.as_ptr(), hlf);
            attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), HLF_PNI), attr);

            screengrid_line_start(&raw mut g_st().pum_grid, row, 0);

            if extra_space {
                if pum().rl {
                    grid_line_puts(col_off + 1, c" ".as_ptr(), 1, attr);
                } else {
                    grid_line_puts(col_off - 1, c" ".as_ptr(), 1, attr);
                }
            }

            let mut grid_col = col_off;
            let mut totwidth: c_int = 0;
            let mut need_fcs_trunc = false;
            let mut order = [0 as c_int; 3];
            let items_width_array = [pum().base_width, pum().kind_width, pum().extra_width];
            pum_align_order(&mut order);
            let basic_width = items_width_array[order[0] as usize];
            let last_isabbr = order[2] == CPT_ABBR;
            let mut orig_attr: c_int = -1;

            for j in 0..3 {
                let item_type = order[j];
                hlf = hlfs[item_type as usize];
                attr = win_hl_attr(*curwin.as_ptr(), hlf);
                attr = hl_combine_attr(win_hl_attr(*curwin.as_ptr(), HLF_PNI), attr);
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
                        if *p != NUL && *p != TAB && totwidth + w <= pum().width {
                            width += w;
                            p = p.add(utfc_ptr2len(p) as usize);
                            continue;
                        }
                        let width_limit = pum().width;

                        let saved = *p;
                        if saved != NUL {
                            *p = NUL;
                        }
                        let mut st_bytes = transstr(s, true).into_bytes_with_nul();
                        let st = st_bytes.as_mut_ptr().cast::<c_char>();
                        if saved != NUL {
                            *p = saved;
                        }

                        let attrs = if item_type == CPT_ABBR {
                            pum_compute_text_attrs(
                                st,
                                hlf,
                                (*pum().array.add(idx as usize)).pum_user_abbr_hlattr,
                            )
                        } else {
                            None
                        };

                        if pum().rl {
                            let mut rt_bytes = reverse_text(std::ffi::CStr::from_ptr(st)).into_bytes_with_nul();
                            let mut rt = rt_bytes.as_mut_ptr().cast::<c_char>();
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

                            if let Some(ref attrs_vec) = attrs {
                                pum_grid_puts_with_attrs(
                                    grid_col - cells + 1,
                                    cells,
                                    rt,
                                    -1,
                                    attrs_vec.as_ptr(),
                                );
                            } else {
                                grid_line_puts(grid_col - cells + 1, rt, -1, attr);
                            }

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

                            if let Some(ref attrs_vec) = attrs {
                                pum_grid_puts_with_attrs(
                                    grid_col,
                                    cells,
                                    st,
                                    -1,
                                    attrs_vec.as_ptr(),
                                );
                            } else {
                                grid_line_puts(grid_col, st, -1, attr);
                            }

                            grid_col += width;
                        }

                        if *p != TAB {
                            break;
                        }

                        if pum().rl {
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
                    || basic_width + n >= pum().width
                {
                    break;
                }

                if pum().rl {
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

            if pum().rl {
                let lcol = col_off - pum().width + 1;
                grid_line_fill(lcol, grid_col + 1, schar_from_ascii(b' '), orig_attr);
                if need_fcs_trunc {
                    g_st().linebuf_char[lcol as usize] = if fcs_trunc != 0 {
                        fcs_trunc
                    } else {
                        schar_from_ascii(b'<')
                    };
                    g_st().linebuf_attr[lcol as usize] = trunc_attr;
                    if pum().width > 1 && g_st().linebuf_char[(lcol + 1) as usize] == 0 {
                        g_st().linebuf_char[(lcol + 1) as usize] = schar_from_ascii(b' ');
                    }
                }
            } else {
                let rcol = col_off + pum().width;
                grid_line_fill(grid_col, rcol, schar_from_ascii(b' '), orig_attr);
                if need_fcs_trunc {
                    if pum().width > 1 && g_st().linebuf_char[(rcol - 1) as usize] == 0 {
                        g_st().linebuf_char[(rcol - 2) as usize] = schar_from_ascii(b' ');
                    }
                    g_st().linebuf_char[(rcol - 1) as usize] = if fcs_trunc != 0 {
                        fcs_trunc
                    } else {
                        schar_from_ascii(b'>')
                    };
                    g_st().linebuf_attr[(rcol - 1) as usize] = trunc_attr;
                }
            }

            if pum().scrollbar > 0 {
                let thumb = i >= thumb_pos && i < thumb_pos + thumb_height;
                let scrollbar_col = col_off + if pum().rl { -pum().width } else { pum().width };
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
fn pum_preview_set_text(
    win: &mut win_T,
    info: *mut c_char,
    lnum: &mut linenr_T,
    max_width: &mut c_int,
) {
    unsafe {
        let mut err = Error::new();
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
        let buf = win.w_buffer;
        (*buf).b_p_ma = 1;

        let mut curr = info;
        while !curr.is_null() {
            let next = CStr::from_ptr(curr).to_bytes().iter().position(|&b| b == b'\n').map_or(ptr::null_mut(), |i| curr.add(i));
            if !next.is_null() {
                *next = NUL;
            }
            if *curr == NUL && next.is_null() {
                break;
            }
            let save_wrap = win.w_onebuf_opt.wo_wrap;
            win.w_onebuf_opt.wo_wrap = 0;
            let line_width = win_linetabsize(&raw mut *win, 0, curr, MAXCOL);
            win.w_onebuf_opt.wo_wrap = save_wrap;
            *max_width = (*max_width).max(line_width);
            replacement.push(string_obj(cstr_to_string(curr)));
            *lnum += 1;

            if !next.is_null() {
                *next = b'\n' as c_char;
                curr = next.add(1);
            } else {
                curr = ptr::null_mut();
            }
        }

        let original_textlock = g_st().textlock;
        g_st().textlock = 0;
        nvim_buf_set_lines(
            0,
            (*buf).b_fnum,
            0,
            -1,
            false,
            replacement,
            &mut arena,
            &mut err,
        );
        g_st().textlock = original_textlock;
        if err.err_type != ErrorType::kErrorTypeNone {
            emsg(err.msg_ptr());
            api_clear_error(&mut err);
        }
        arena_mem_free(arena_finish(&mut arena));
        api_free_array(replacement);
        (*buf).b_p_ma = 0;
    }
}

/// Adjust floating info preview window position (`pum_adjust_info_position`).
fn pum_adjust_info_position(wp: &mut win_T, width: c_int) -> bool {
    unsafe {
        let border_width = pum_border_width();
        let col = pum().col + pum().width + 1 + border_width.max(pum().scrollbar);
        let right_extra = g_st().Columns - col;
        let left_extra = pum().col - 2;

        let max_extra = right_extra.max(left_extra);
        if max_extra < 10 {
            wp.w_config.hide = true;
            return false;
        }

        if right_extra > width {
            wp.w_config.width = width;
            wp.w_config.col = f64::from(col - 1);
        } else if left_extra > width {
            wp.w_config.width = width;
            wp.w_config.col = f64::from(pum().col - wp.w_config.width - 1);
        } else {
            let place_in_right = right_extra > left_extra;
            wp.w_config.width = max_extra;
            wp.w_config.col = f64::from(if place_in_right {
                col - 1
            } else {
                pum().col - wp.w_config.width - 1
            });
        }
        wp.w_config.anchor = 0;
        let count = (*wp.w_buffer).b_ml.ml_line_count;
        wp.w_view_width = wp.w_config.width;
        wp.w_config.height = plines_m_win(wp, wp.w_topline, count, g_st().Rows);
        wp.w_config.row = f64::from(pum().row);
        wp.w_config.hide = false;
        let cfg = wp.w_config.clone();
        win_config_float(&mut *wp, cfg);
        true
    }
}

/// Used for `nvim__complete_set` (`pum_set_info`).
pub fn pum_set_info(selected: c_int, info: *mut c_char) -> *mut win_T {
    unsafe {
        if !pum().is_visible || !compl_match_curr_select(selected) {
            return ptr::null_mut();
        }
        block_autocmds();
        g_st().RedrawingDisabled += 1;
        g_st().no_u_sync += 1;
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
        pum_preview_set_text(&mut *wp, info, &mut lnum, &mut max_info_width);
        g_st().no_u_sync -= 1;
        g_st().RedrawingDisabled -= 1;
        redraw_later(wp, UPD_NOT_VALID);

        if !pum_adjust_info_position(&mut *wp, max_info_width) {
            wp = ptr::null_mut();
        }
        unblock_autocmds();
        wp
    }
}

/// Set the index of the currently selected item (`pum_set_selected`).
fn pum_set_selected(n: c_int, repeat: c_int) -> bool {
    unsafe {
        let mut resized = false;
        let mut context = pum().height / 2;
        let prev_selected = pum().selected;

        pum().selected = n;
        let scroll_offset = pum().selected - pum().height;
        let cur_cot_flags = get_cot_flags();
        let use_float = (cur_cot_flags & kOptCotFlagPopup) != 0;

        if use_float
            && (pum().selected < 0 || (*pum().array.add(pum().selected as usize)).pum_info.is_none())
        {
            let wp = win_float_find(kWinInfo);
            if !wp.is_null() {
                (*wp).w_config.hide = true;
                win_config_float(&mut *wp, (*wp).w_config.clone());
            }
        }

        if pum().selected >= 0 && pum().selected < pum().size {
            if pum().first > pum().selected - 4 {
                if pum().first > pum().selected - 2 {
                    pum().first -= pum().height - 2;
                    if pum().first < 0 {
                        pum().first = 0;
                    } else if pum().first > pum().selected {
                        pum().first = pum().selected;
                    }
                } else {
                    pum().first = pum().selected;
                }
            } else if pum().first < scroll_offset + 5 {
                if pum().first < scroll_offset + 3 {
                    pum().first = (pum().first + pum().height - 2).max(scroll_offset + 1);
                } else {
                    pum().first = scroll_offset + 1;
                }
            }

            context = context.min(3);

            if pum().height > 2 {
                if pum().first > pum().selected - context {
                    pum().first = (pum().selected - context).max(0);
                } else if pum().first < pum().selected + context - pum().height + 1 {
                    pum().first = pum().selected + context - pum().height + 1;
                }
            }
            pum().first = pum().first.min(pum().size - pum().height);

            if (*pum().array.add(pum().selected as usize)).pum_info.is_some()
                && g_st().Rows > 10
                && repeat <= 1
                && (cur_cot_flags & (kOptCotFlagPopup | kOptCotFlagPreview)) != 0
                && !((cur_cot_flags & kOptCotFlagPreview) != 0 && !g_st().cmdwin_buf.is_null())
            {
                let curwin_save = *curwin.as_ptr();
                let curtab_save = g_st().curtab;

                if use_float {
                    block_autocmds();
                }

                g_st().g_do_tagpreview = 3;
                if opt_vars().p_pvh > 0 && opt_vars().p_pvh < i64::from(g_st().g_do_tagpreview) {
                    g_st().g_do_tagpreview = opt_vars().p_pvh as c_int;
                }
                g_st().RedrawingDisabled += 1;
                g_st().no_u_sync += 1;

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

                g_st().no_u_sync -= 1;
                g_st().RedrawingDisabled -= 1;
                g_st().g_do_tagpreview = 0;

                if (**curwin.as_ptr()).w_onebuf_opt.wo_pvw != 0 || (**curwin.as_ptr()).w_kind == kWinInfo {
                    let mut res = OK;
                    if !resized
                        && (*g_st().curbuf).b_nwindows == 1
                        && (*g_st().curbuf).b_fname.is_null()
                        && bt_nofile(g_st().curbuf)
                        && *(*g_st().curbuf).b_p_bh == b'w' as c_char
                    {
                        buf_clear();
                    } else {
                        g_st().no_u_sync += 1;
                        res = do_ecmd(
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ECMD_ONE,
                            0,
                            ptr::null_mut(),
                        );
                        g_st().no_u_sync -= 1;

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
                            &mut **curwin.as_ptr(),
                            (*pum().array.add(pum().selected as usize)).info_ptr(),
                            &mut lnum,
                            &mut max_info_width,
                        );
                        if repeat == 0 && !use_float {
                            lnum = lnum.min(opt_vars().p_pvh as linenr_T);
                            if (**curwin.as_ptr()).w_height < lnum {
                                win_setheight(lnum);
                                resized = true;
                            }
                        }

                        (*g_st().curbuf).b_changed = 0;
                        (*g_st().curbuf).b_p_ma = 0;
                        if pum().selected != prev_selected {
                            (**curwin.as_ptr()).w_topline = 1;
                        } else if (**curwin.as_ptr()).w_topline > (*g_st().curbuf).b_ml.ml_line_count {
                            (**curwin.as_ptr()).w_topline = (*g_st().curbuf).b_ml.ml_line_count;
                        }
                        (**curwin.as_ptr()).w_cursor.lnum = 1;
                        (**curwin.as_ptr()).w_cursor.col = 0;

                        if use_float
                            && !pum_adjust_info_position(&mut **curwin.as_ptr(), max_info_width)
                            && win_valid(curwin_save)
                        {
                            win_enter(curwin_save, false);
                        }

                        if (*curwin.as_ptr() != curwin_save && win_valid(curwin_save))
                            || (g_st().curtab != curtab_save && valid_tabpage(curtab_save))
                        {
                            if g_st().curtab != curtab_save && valid_tabpage(curtab_save) {
                                goto_tabpage_tp(curtab_save, false, false);
                            }

                            if ins_compl_active() && !resized {
                                (**curwin.as_ptr()).w_redr_status = false;
                            }

                            validate_cursor(*curwin.as_ptr());
                            redraw_later(*curwin.as_ptr(), UPD_SOME_VALID);

                            if resized && win_valid(curwin_save) {
                                g_st().no_u_sync += 1;
                                win_enter(curwin_save, true);
                                g_st().no_u_sync -= 1;
                                update_topline(*curwin.as_ptr());
                            }

                            let save_pum_is_drawn = pum().is_drawn;

                            pum().is_visible = false;
                            pum().is_drawn = false;
                            update_screen();
                            pum().is_visible = true;
                            pum().is_drawn = save_pum_is_drawn;

                            if !resized && win_valid(curwin_save) {
                                g_st().no_u_sync += 1;
                                win_enter(curwin_save, true);
                                g_st().no_u_sync -= 1;
                            }

                            pum().is_visible = false;
                            pum().is_drawn = false;
                            update_screen();
                            pum().is_visible = true;
                            pum().is_drawn = save_pum_is_drawn;
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
pub fn pum_undisplay(immediate: bool) {
    pum().is_visible = false;
        pum().array = ptr::null_mut();
        g_st().must_redraw_pum = false;

        if immediate {
            pum_check_clear();
        }
}

pub fn pum_check_clear() {
    if !pum().is_visible && pum().is_drawn {
            if pum().external {
                ui_call_popupmenu_hide();
            } else {
                ui_comp_remove_grid(&mut g_st().pum_grid);
                if ui_has(kUIMultigrid) {
                    ui_call_win_close(i64::from(g_st().pum_grid.handle));
                    ui_call_grid_destroy(i64::from(g_st().pum_grid.handle));
                }
                grid_free(&mut g_st().pum_grid);
            }
            pum().is_drawn = false;
            pum().external = false;
            win_float_close(kWinInfo);
        }
}

/// Clear the popup menu (`pum_clear`).
pub fn pum_clear() {
    pum().first = 0;
}

/// Return true if the popup menu is displayed (`pum_visible`).
pub fn pum_visible() -> bool {
    pum().is_visible
}

/// Return true if the popup menu is displayed and drawn on the grid (`pum_drawn`).
pub fn pum_drawn() -> bool {
    pum_visible() && !pum().external
}

/// Screen was cleared, need to redraw next time (`pum_invalidate`).
pub fn pum_invalidate() {
    pum().invalid = true;
}

pub fn pum_ext_select_item(item: c_int, insert: bool, finish: bool) {
    if !pum_visible() || item < -1 || item >= pum().size {
            return;
        }
        g_st().pum_want.active = true;
        g_st().pum_want.item = item;
        g_st().pum_want.insert = insert;
        g_st().pum_want.finish = finish;
}

/// Gets the height of the menu (`pum_get_height`).
pub fn pum_get_height() -> c_int {
    if pum().external {
            let ui_pum_height = ui_pum_get_height();
            if ui_pum_height != 0 {
                return ui_pum_height;
            }
        }
        pum().height
}

/// Add size information about the pum to `dict` (`pum_set_event_info`).
pub fn pum_set_event_info(dict: *mut dict_T) {
    if !pum_visible() {
            return;
        }
        let mut w: f64 = 0.0;
        let mut h: f64 = 0.0;
        let mut r: f64 = 0.0;
        let mut c: f64 = 0.0;
        if !ui_pum_get_pos(&raw mut w, &raw mut h, &raw mut r, &raw mut c) {
            w = f64::from(pum().width);
            h = f64::from(pum().height);
            r = f64::from(pum().row);
            c = f64::from(pum().col);
        }
        tv_dict_add_float(dict, c"height".as_ptr(), 6, h);
        tv_dict_add_float(dict, c"width".as_ptr(), 5, w);
        tv_dict_add_float(dict, c"row".as_ptr(), 3, r);
        tv_dict_add_float(dict, c"col".as_ptr(), 3, c);
        tv_dict_add_nr(dict, c"size".as_ptr(), 4, i64::from(pum().size));
        tv_dict_add_bool(
            dict,
            c"scrollbar".as_ptr(),
            9,
            if pum().scrollbar != 0 {
                kBoolVarTrue
            } else {
                kBoolVarFalse
            },
        );
}

fn pum_position_at_mouse(min_width: c_int) {
    unsafe {
        let min_row: c_int = 0;
        let min_col: c_int = 0;
        let mut max_row = g_st().Rows;
        let mut max_col = g_st().Columns;
        let mut grid = g_st().mouse_grid;
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;
        pum().win_row_offset = 0;
        pum().win_col_offset = 0;

        if ui_has(kUIMultigrid) && grid == 0 {
            mouse_find_win_outer(&raw mut grid, &raw mut row, &raw mut col);
        }
        if grid > 1 {
            let wp = get_win_by_grid_handle(grid);
            if !wp.is_null() {
                row += (*wp).w_winrow;
                col += (*wp).w_wincol;
                pum().win_row_offset = (*wp).w_winrow;
                pum().win_col_offset = (*wp).w_wincol;

                if (*wp).w_view_height > 0 || (*wp).w_view_width > 0 {
                    max_row = (g_st().Rows - (*wp).w_winrow).max((*wp).w_winrow + (*wp).w_view_height);
                    max_col = (g_st().Columns - (*wp).w_wincol).max((*wp).w_wincol + (*wp).w_view_width);
                }
            }
        }
        if g_st().pum_grid.handle != 0 && grid == g_st().pum_grid.handle {
            row += pum().row;
            col += pum().left_col;
        } else {
            pum().anchor_grid = grid;
        }

        let border_width = pum_border_width();
        let border_height = border_width;
        if max_row - row > pum().size + border_height || max_row - row > row - min_row {
            pum().above = false;
            pum().row = row + 1;
            if pum().height + border_height > max_row - pum().row {
                pum().height = max_row - pum().row - border_height;
            }
        } else {
            pum().above = true;
            pum().row = row - pum().size - border_height;
            if pum().row < min_row {
                pum().height += pum().row - min_row;
                pum().row = min_row;
            }
        }

        if pum().rl {
            if col - min_col + 1 >= pum().base_width + border_width
                || col - min_col + 1 > min_width + border_width
            {
                pum().col = col;
            } else {
                pum().col =
                    min_col + (pum().base_width + border_width).min(min_width + border_width) - 1;
            }
            pum().width = pum().col - min_col + 1 - border_width;
        } else {
            if max_col - col >= pum().base_width + border_width
                || max_col - col > min_width + border_width
            {
                pum().col = col;
            } else {
                pum().col = max_col - (pum().base_width + border_width).min(min_width + border_width);
            }
            pum().width = max_col - pum().col - border_width;
        }

        pum().width = pum().width.min(pum().base_width + 1);
    }
}

/// Select the pum entry at the mouse position (`pum_select_mouse_pos`).
fn pum_select_mouse_pos() {
    unsafe {
        let mut grid = g_st().mouse_grid;
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;

        if grid == 0 {
            mouse_find_win_outer(&raw mut grid, &raw mut row, &raw mut col);
        }

        if grid == g_st().pum_grid.handle {
            let border_offset = if pum_border_width() == 2 { 1 } else { 0 };
            let item = row - border_offset;
            pum().selected = if item >= 0 && item < pum().height {
                item
            } else {
                -1
            };
            return;
        }

        if grid != pum().anchor_grid
            || col < pum().left_col - pum().win_col_offset
            || col >= pum().right_col - pum().win_col_offset
        {
            pum().selected = -1;
            return;
        }

        let idx = row - (pum().row - pum().win_row_offset);

        if idx < 0 || idx >= pum().height {
            pum().selected = -1;
        } else if (*pum().array.add(idx as usize))
            .pum_text
            .as_ref()
            .is_some_and(|s| !s.to_bytes().is_empty())
        {
            pum().selected = idx;
        }
    }
}

/// Execute the currently selected popup menu item (`pum_execute_menu`).
fn pum_execute_menu(menu: &mut vimmenu_T, mode: c_int) {
    let mut idx: c_int = 0;
        for mp in &mut menu.children {
            if (mp.modes & mp.enabled & mode) != 0 {
                let cur_idx = idx;
                idx += 1;
                if cur_idx == pum().selected {
                    let ea: exarg_T = exarg_T::new();
                    execute_menu(&ea, mp, -1);
                    break;
                }
            }
        }
}

/// Open the terminal version of the popup menu (`pum_show_popupmenu`).
pub fn pum_show_popupmenu(menu: &mut vimmenu_T) {
    unsafe {
        pum_undisplay(true);
        pum().size = 0;
        let mode = get_menu_mode_flag();

        for mp in &menu.children {
            if menu_is_separator(mp.dname.as_ptr().cast_mut())
                || (mp.modes & mp.enabled & mode) != 0
            {
                pum().size += 1;
            }
        }

        if pum().size <= 0 {
            emsg(gettext(e_menu_only_exists_in_another_mode.as_ptr()));
            return;
        }

        let mut idx: usize = 0;
        let mut array_vec: Vec<pumitem_T> = vec![pumitem_T::new(); pum().size as usize];
        let array = array_vec.as_mut_ptr();

        for mp in &menu.children {
            let mut s: *const c_char = ptr::null();
            if menu_is_separator(mp.dname.as_ptr().cast_mut()) {
                s = c"".as_ptr();
            } else if (mp.modes & mp.enabled & mode) != 0 {
                s = mp.dname.as_ptr();
            }
            if !s.is_null() {
                array_vec[idx].pum_text = Some(CStr::from_ptr(s).to_owned());
                idx += 1;
            }
        }

        pum().array = array;
        pum_compute_size();
        pum().scrollbar = 0;
        pum().height = pum().size;
        pum().rl = (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0;
        pum_position_at_mouse(20);

        pum().selected = -1;
        pum().first = 0;
        if opt_vars().p_mousemev == 0 {
            ui_call_option_set(
                NvimString {
                    data: c"mousemoveevent".as_ptr().cast_mut(),
                    size: 14,
                },
                boolean_obj(true),
            );
        }

        loop {
            pum().is_visible = true;
            pum().is_drawn = true;
            g_st().pum_grid.zindex = kZIndexCmdlinePopupMenu;
            pum_redraw();
            setcursor_mayforce(&mut **curwin.as_ptr(), true);

            let c = vgetc();

            if c == ESC || c == Ctrl_C || pum().array.is_null() {
                break;
            } else if c == CAR || c == NL {
                pum_execute_menu(menu, mode);
                break;
            } else if c == b'k' as c_int || c == K_UP || c == K_MOUSEUP {
                while pum().selected > 0 {
                    pum().selected -= 1;
                    if (*array.add(pum().selected as usize))
                        .pum_text
                        .as_ref()
                        .is_some_and(|s| !s.to_bytes().is_empty())
                    {
                        break;
                    }
                }
            } else if c == b'j' as c_int || c == K_DOWN || c == K_MOUSEDOWN {
                while pum().selected < pum().size - 1 {
                    pum().selected += 1;
                    if (*array.add(pum().selected as usize))
                        .pum_text
                        .as_ref()
                        .is_some_and(|s| !s.to_bytes().is_empty())
                    {
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
                if pum().selected >= 0 {
                    pum_execute_menu(menu, mode);
                    break;
                }
                if c == K_LEFTMOUSE || c == K_LEFTMOUSE_NM {
                    break;
                }
            }
        }

        pum_undisplay(true);
        if opt_vars().p_mousemev == 0 {
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

pub fn pum_make_popup(path_name: *const c_char, use_mouse_pos: c_int) {
    unsafe {
        if use_mouse_pos == 0 {
            g_st().mouse_row = (**curwin.as_ptr()).w_grid.row_offset + (**curwin.as_ptr()).w_wrow;
            g_st().mouse_col = (**curwin.as_ptr()).w_grid.col_offset
                + if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 {
                    (**curwin.as_ptr()).w_view_width - (**curwin.as_ptr()).w_wcol - 1
                } else {
                    (**curwin.as_ptr()).w_wcol
                };
            if ui_has(kUIMultigrid) {
                g_st().mouse_grid = (*(**curwin.as_ptr()).w_grid.target).handle;
            } else if !ptr::eq((**curwin.as_ptr()).w_grid.target, &raw const g_st().default_grid) {
                g_st().mouse_grid = 0;
                g_st().mouse_row += (**curwin.as_ptr()).w_winrow;
                g_st().mouse_col += (**curwin.as_ptr()).w_wincol;
            }
        }

        let menu = menu_find(path_name);
        if !menu.is_null() {
            pum_show_popupmenu(&mut *menu);
        }
    }
}

pub fn pum_pos_ui_flush(force: bool) {
    if force
            || (ui_has(kUIMultigrid)
                && pum().is_drawn
                && !pum().external
                && g_st().pum_grid.handle != 0
                && g_st().pum_grid.pending_comp_index_update)
        {
            let anchor = if pum().above {
                c"SW".as_ptr()
            } else {
                c"NW".as_ptr()
            };
            let row_off = if pum().above { -pum().height } else { 0 };
            ui_call_win_float_pos(
                i64::from(g_st().pum_grid.handle),
                -1,
                cstr_as_string(anchor),
                i64::from(pum().anchor_grid),
                f64::from(pum().row - row_off - pum().win_row_offset),
                f64::from(pum().left_col - pum().win_col_offset),
                false,
                i64::from(g_st().pum_grid.zindex),
                g_st().pum_grid.comp_index as i64,
                i64::from(g_st().pum_grid.comp_row),
                i64::from(g_st().pum_grid.comp_col),
            );
            g_st().pum_grid.pending_comp_index_update = false;
        }
}
