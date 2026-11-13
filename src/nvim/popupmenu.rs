//! Popup menu (PUM) (`src/nvim/popupmenu.c`).
#![forbid(unsafe_code)]

use crate::api::buffer::nvim_buf_set_lines;
use crate::api::private::helpers::{
    add_c, api_clear_error, api_free_array, array_obj, boolean_obj, cstr_as_obj, cstr_as_string,
    string_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString};
use crate::api::win_config::{K_UIMULTIGRID, WIN_CONFIG_INIT, parse_winborder};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{bt_nofile, buf_clear, first_win_in_tab};
use crate::charset::{transstr, vim_strsize};
use crate::cmdexpand::{
    K_UICMDLINE, K_UIPOPUPMENU_CONST, K_UIWILDMENU_CONST, cmdline_compl_is_fuzzy,
    cmdline_compl_pattern,
};
use crate::cursor::Linenr;
use crate::drawscreen::{
    DEFAULT_GRID_HANDLE, UPD_NOT_VALID, UPD_SOME_VALID, conceal_cursor_line, redraw_later,
    setcursor_mayforce, update_screen,
};
use crate::errors::*;
use crate::eval::typval::{TvDict, tv_dict_add_bool, tv_dict_add_float, tv_dict_add_nr};
use crate::ex_cmds::{CTRL_C_CONST, ECMD_ONE, do_ecmd, prepare_tagpreview};
use crate::ex_docmd::Exarg;
use crate::fuzzy::fuzzy_match_str_with_pos;
use crate::globals::*;
use crate::grid::{
    Schar, get_win_by_grid_handle, grid_alloc, grid_assign_handle, grid_draw_border, grid_free,
    grid_invalidate, grid_line_fill, grid_line_flush, grid_line_put_schar, grid_line_puts,
    schar_from_ascii, screengrid_line_start,
};
use crate::highlight::{HLF_PNI, HLF_PST, hl_combine_attr, hl_get_ui_attr, win_hl_attr};
use crate::highlight_group::syn_check_group;
use crate::input::{vgetc, vungetc};
use crate::insexpand::{
    K_OPT_COT_FLAG_FUZZY, K_WIN_INFO, compl_match_curr_select, get_cot_flags, ins_compl_active,
    ins_compl_leader,
};
use crate::keycodes::{
    CAR, ESC, K_DOWN, K_LEFTDRAG, K_LEFTMOUSE, K_LEFTMOUSE_NM, K_MOUSEDOWN, K_MOUSEMOVE, K_MOUSEUP,
    K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_UP, NL,
};
use crate::mbyte::{mb_string2cells, mb_strnicmp, utf_ptr2cells, utfc_ptr2len};
use crate::menu::{Vimmenu, execute_menu, get_menu_mode_flag, menu_find, menu_is_separator};
use crate::message::emsg;
use crate::mouse::mouse_find_win_outer;
use crate::r#move::{update_topline, validate_cheight, validate_cursor, validate_cursor_col};
use crate::option::{
    K_OPT_BUFHIDDEN, K_OPT_BUFLISTED, K_OPT_BUFTYPE, K_OPT_SWAPFILE, OPT_LOCAL,
    set_option_value_give_err,
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
use crate::window::{
    WinInfoInfo, goto_tabpage_tp, valid_tabpage, win_enter, win_setheight, win_valid,
};
use crate::winfloat::{win_config_float, win_float_close, win_float_find, win_float_special};
use std::ffi::CStr;

use crate::api::vim::K_OPT_COT_FLAG_POPUP;
use crate::change::TAB;
use crate::diff::K_OPT_DIFF;
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::ex_getln::Hlf;
use crate::regexp::MAXCOL;
use crate::state::MODE_CMDLINE;

#[repr(C)]
#[derive(Clone, Default)]
pub struct Pumitem {
    pub pum_text: Option<std::ffi::CString>,
    pub pum_kind: Option<std::ffi::CString>,
    pub pum_extra: Option<std::ffi::CString>,
    pub pum_info: Option<std::ffi::CString>,
    pub pum_cpt_source_idx: i32,
    pub pum_user_abbr_hlattr: i32,
    pub pum_user_kind_hlattr: i32,
    pub pum_comp_idx: Option<usize>,
}

impl Pumitem {
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
    pub fn text_cstr(&self) -> Option<&CStr> {
        self.pum_text.as_deref()
    }

    #[inline]
    pub fn kind_cstr(&self) -> Option<&CStr> {
        self.pum_kind.as_deref()
    }

    #[inline]
    pub fn extra_cstr(&self) -> Option<&CStr> {
        self.pum_extra.as_deref()
    }

    #[inline]
    pub fn info_cstr(&self) -> Option<&CStr> {
        self.pum_info.as_deref()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct PumWant {
    pub active: std::cell::Cell<bool>,
    pub item: std::cell::Cell<i32>,
    pub insert: std::cell::Cell<bool>,
    pub finish: std::cell::Cell<bool>,
}

const PUM_DEF_HEIGHT: i32 = 10;

const CPT_ABBR: i32 = 0;
const CPT_KIND: i32 = 1;
const CPT_MENU: i32 = 2;

const HLF_PSI: Hlf = 42;
const HLF_PMNI: Hlf = 43;
const HLF_PMSI: Hlf = 44;
const HLF_PNK: Hlf = 45;
const HLF_PSK: Hlf = 46;
const HLF_PNX: Hlf = 47;
const HLF_PSX: Hlf = 48;
const HLF_PSB: Hlf = 49;
const HLF_PBR: Hlf = 51;

const K_ZINDEX_POPUP_MENU: i32 = 100;
const K_ZINDEX_CMDLINE_POPUP_MENU: i32 = 250;

const K_OPT_COT_FLAG_PREVIEW: u32 = 0x1000;

struct PopupmenuState {
    size: std::cell::Cell<i32>,
    selected: std::cell::Cell<i32>,
    first: std::cell::Cell<i32>,
    height: std::cell::Cell<i32>,
    width: std::cell::Cell<i32>,
    base_width: std::cell::Cell<i32>,
    kind_width: std::cell::Cell<i32>,
    extra_width: std::cell::Cell<i32>,
    scrollbar: std::cell::Cell<i32>,
    rl: std::cell::Cell<bool>,
    anchor_grid: std::cell::Cell<i32>,
    row: std::cell::Cell<i32>,
    col: std::cell::Cell<i32>,
    win_row_offset: std::cell::Cell<i32>,
    win_col_offset: std::cell::Cell<i32>,
    left_col: std::cell::Cell<i32>,
    right_col: std::cell::Cell<i32>,
    above: std::cell::Cell<bool>,
    is_visible: std::cell::Cell<bool>,
    is_drawn: std::cell::Cell<bool>,
    external: std::cell::Cell<bool>,
    invalid: std::cell::Cell<bool>,
}

impl PopupmenuState {
    const fn new() -> Self {
        Self {
            size: std::cell::Cell::new(0),
            selected: std::cell::Cell::new(0),
            first: std::cell::Cell::new(0),
            height: std::cell::Cell::new(0),
            width: std::cell::Cell::new(0),
            base_width: std::cell::Cell::new(0),
            kind_width: std::cell::Cell::new(0),
            extra_width: std::cell::Cell::new(0),
            scrollbar: std::cell::Cell::new(0),
            rl: std::cell::Cell::new(false),
            anchor_grid: std::cell::Cell::new(0),
            row: std::cell::Cell::new(0),
            col: std::cell::Cell::new(0),
            win_row_offset: std::cell::Cell::new(0),
            win_col_offset: std::cell::Cell::new(0),
            left_col: std::cell::Cell::new(0),
            right_col: std::cell::Cell::new(0),
            above: std::cell::Cell::new(false),
            is_visible: std::cell::Cell::new(false),
            is_drawn: std::cell::Cell::new(false),
            external: std::cell::Cell::new(false),
            invalid: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static PUM: PopupmenuState = const { PopupmenuState::new() };
    static PUM_ITEMS: std::cell::RefCell<Option<std::rc::Rc<[Pumitem]>>> =
        const { std::cell::RefCell::new(None) };
}

#[inline]
fn pum_has_items() -> bool {
    PUM_ITEMS.with(|c| c.borrow().is_some())
}

#[inline]
fn pum_item(idx: i32) -> Pumitem {
    PUM_ITEMS.with(|c| c.borrow().as_ref().unwrap()[idx as usize].clone())
}

use crate::api::private::helpers::cstr_to_string;

fn pum_compute_size() {
    PUM.with(|s| s.base_width.set(0));
    PUM.with(|s| s.kind_width.set(0));
    PUM.with(|s| s.extra_width.set(0));
    let Some(items) = PUM_ITEMS.with(|c| c.borrow().clone()) else {
        return;
    };
    let size = (PUM.with(|s| s.size.get()).max(0) as usize).min(items.len());
    for item in &items[..size] {
        if let Some(ref text) = item.pum_text {
            let w = vim_strsize(text);
            if PUM.with(|s| s.base_width.get()) < w {
                PUM.with(|s| s.base_width.set(w));
            }
        }
        if let Some(ref kind) = item.pum_kind {
            let w = vim_strsize(kind) + 1;
            if PUM.with(|s| s.kind_width.get()) < w {
                PUM.with(|s| s.kind_width.set(w));
            }
        }
        if let Some(ref extra) = item.pum_extra {
            let w = vim_strsize(extra) + 1;
            if PUM.with(|s| s.extra_width.get()) < w {
                PUM.with(|s| s.extra_width.set(w));
            }
        }
    }
}

/// Calculate vertical placement for popup menu (`pum_compute_vertical_placement`).
fn pum_compute_vertical_placement(
    size: i32,
    target_win: Option<&WinInfoInfo>,
    pum_win_row: i32,
    above_row: i32,
    below_row: i32,
    pum_border_size: i32,
) {
    let context_lines: i32;

    PUM.with(|s| s.height.set(size.min(PUM_DEF_HEIGHT)));
    if opt_vars().p_ph.get() > 0 && i64::from(PUM.with(|s| s.height.get())) > opt_vars().p_ph.get()
    {
        PUM.with(|s| s.height.set(opt_vars().p_ph.get() as i32));
    }

    if pum_win_row + 2 + pum_border_size >= below_row - PUM.with(|s| s.height.get())
        && pum_win_row - above_row > (below_row - above_row) / 2
    {
        PUM.with(|s| s.above.set(true));

        if (g_st().state_global.get() & MODE_CMDLINE) != 0 && target_win.is_none() {
            context_lines = 0;
        } else {
            context_lines =
                2.min(target_win.unwrap().w_wrow.get() - target_win.unwrap().w_cline_row.get());
        }

        if pum_win_row >= size + context_lines {
            PUM.with(|s| s.row.set(pum_win_row - size - context_lines));
            PUM.with(|s| s.height.set(size));
        } else {
            PUM.with(|s| s.row.set(0));
            PUM.with(|s| s.height.set(pum_win_row - context_lines));
        }
        if opt_vars().p_ph.get() > 0
            && i64::from(PUM.with(|s| s.height.get())) > opt_vars().p_ph.get()
        {
            PUM.with(|s| {
                s.row.set(
                    s.row.get() + (PUM.with(|s| s.height.get()) - (opt_vars().p_ph.get() as i32)),
                )
            });
            PUM.with(|s| s.height.set(opt_vars().p_ph.get() as i32));
        }

        if pum_border_size > 0
            && pum_border_size + PUM.with(|s| s.row.get()) + PUM.with(|s| s.height.get())
                >= pum_win_row
        {
            if PUM.with(|s| s.row.get()) < 2 {
                PUM.with(|s| s.height.set(s.height.get() - (pum_border_size)));
            } else {
                PUM.with(|s| s.row.set(s.row.get() - (pum_border_size)));
            }
        }
    } else {
        PUM.with(|s| s.above.set(false));

        if (g_st().state_global.get() & MODE_CMDLINE) != 0 && target_win.is_none() {
            context_lines = 0;
        } else {
            validate_cheight(target_win.unwrap());
            let cline_visible_offset = target_win.unwrap().w_cline_row.get()
                + target_win.unwrap().w_cline_height.get()
                - target_win.unwrap().w_wrow.get();
            context_lines = 3.min(cline_visible_offset);
        }

        PUM.with(|s| s.row.set(pum_win_row + context_lines));
        PUM.with(|s| {
            s.height
                .set((below_row - PUM.with(|s| s.row.get())).min(size))
        });
        if opt_vars().p_ph.get() > 0
            && i64::from(PUM.with(|s| s.height.get())) > opt_vars().p_ph.get()
        {
            PUM.with(|s| s.height.set(opt_vars().p_ph.get() as i32));
        }

        if PUM.with(|s| s.row.get()) + PUM.with(|s| s.height.get()) + pum_border_size
            >= g_st().cmdline_row.get()
        {
            PUM.with(|s| s.height.set(s.height.get() - (pum_border_size)));
        }
    }

    if above_row > 0
        && PUM.with(|s| s.row.get()) < above_row
        && PUM.with(|s| s.height.get()) > above_row
    {
        PUM.with(|s| s.row.set(above_row));
        PUM.with(|s| s.height.set(pum_win_row - above_row));
    }
}

/// Try to set `pum_width` so that it fits within `available_width` (`set_pum_width_aligned_with_cursor`).
fn set_pum_width_aligned_with_cursor(mut width: i32, available_width: i32) -> bool {
    let mut end_padding = true;

    if i64::from(width) < opt_vars().p_pw.get() {
        width = opt_vars().p_pw.get() as i32;
        end_padding = false;
    }
    if opt_vars().p_pmw.get() > 0 && i64::from(width) > opt_vars().p_pmw.get() {
        width = opt_vars().p_pmw.get() as i32;
        end_padding = false;
    }

    PUM.with(|s| {
        s.width.set(
            width
                + if end_padding && i64::from(width) >= opt_vars().p_pw.get() {
                    1
                } else {
                    0
                },
        )
    });
    available_width >= PUM.with(|s| s.width.get())
}

/// Calculate horizontal placement for popup menu (`pum_compute_horizontal_placement`).
fn pum_compute_horizontal_placement(
    target_win: Option<&WinInfoInfo>,
    cursor_col: i32,
    border_width: i32,
) {
    let max_col = g_st()
        .columns_global
        .get()
        .max(if let Some(target_win) = target_win {
            target_win.w_wincol.get() + target_win.w_view_width.get()
        } else {
            0
        });
    let desired_width = PUM.with(|s| s.base_width.get())
        + PUM.with(|s| s.kind_width.get())
        + PUM.with(|s| s.extra_width.get());
    let mut available_width = if PUM.with(|s| s.rl.get()) {
        cursor_col - PUM.with(|s| s.scrollbar.get()) + 1 - border_width
    } else {
        max_col - cursor_col - PUM.with(|s| s.scrollbar.get()) - border_width
    };

    PUM.with(|s| s.col.set(cursor_col));
    if set_pum_width_aligned_with_cursor(desired_width, available_width) {
        return;
    }

    if i64::from(available_width) > opt_vars().p_pw.get() {
        PUM.with(|s| s.width.set(available_width));
        return;
    }

    if PUM.with(|s| s.rl.get()) {
        available_width = max_col - PUM.with(|s| s.scrollbar.get()) - border_width;
    } else {
        available_width += cursor_col;
    }

    if i64::from(available_width) > opt_vars().p_pw.get() {
        PUM.with(|s| s.width.set((opt_vars().p_pw.get() as i32) + 1));
        if PUM.with(|s| s.rl.get()) {
            PUM.with(|s| {
                s.col.set(
                    PUM.with(|s| s.width.get()) + PUM.with(|s| s.scrollbar.get()) + border_width,
                )
            });
        } else {
            PUM.with(|s| {
                s.col.set(
                    max_col
                        - PUM.with(|s| s.width.get())
                        - PUM.with(|s| s.scrollbar.get())
                        - border_width,
                )
            });
        }
        return;
    }

    if PUM.with(|s| s.rl.get()) {
        PUM.with(|s| s.col.set(max_col - 1));
    } else {
        PUM.with(|s| s.col.set(0));
    }
    PUM.with(|s| {
        s.width
            .set(max_col - PUM.with(|s| s.scrollbar.get()) - border_width)
    });
}

#[inline]
fn pum_border_width() -> i32 {
    if opt_vars().p_pumborder.is_empty()
        || opt_vars().p_pumborder.as_bytes() == OPT_WINBORDER_VALUES[7].to_bytes()
    {
        0
    } else if opt_vars().p_pumborder.as_bytes() == OPT_WINBORDER_VALUES[3].to_bytes() {
        1
    } else {
        2
    }
}

/// Show the popup menu with items `items` (`pum_display`).
pub fn pum_display(
    items: std::rc::Rc<[Pumitem]>,
    selected: i32,
    array_changed: bool,
    cmd_startcol: i32,
) {
    let size = items.len() as i32;
    let mut redo_count: i32 = 0;

    if !PUM.with(|s| s.is_visible.get()) {
        PUM.with(|s| {
            s.external.set(
                ui_has(K_UIPOPUPMENU_CONST)
                    || ((g_st().state_global.get() & MODE_CMDLINE) != 0
                        && ui_has(K_UIWILDMENU_CONST)),
            )
        });
    }

    PUM.with(|s| {
        s.rl.set(
            (g_st().state_global.get() & MODE_CMDLINE) == 0
                && curwin_mut().w_onebuf_opt.wo_rl.get(),
        )
    });

    let border_width = pum_border_width();
    loop {
        PUM.with(|s| s.is_visible.set(true));
        PUM.with(|s| s.is_drawn.set(true));
        validate_cursor_col(&curwin_mut());
        let mut above_row: i32 = 0;
        let mut below_row = g_st()
            .cmdline_row
            .get()
            .max(curwin_mut().w_winrow.get() + curwin_mut().w_view_height.get());
        if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            below_row = g_st().cmdline_row.get();
        }
        let target_win = if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            g_st().cmdline_win.get()
        } else {
            curwin_ptr()
        };
        PUM.with(|s| s.win_row_offset.set(0));
        PUM.with(|s| s.win_col_offset.set(0));

        let mut pum_win_row: i32;
        let mut cursor_col: i32;

        if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            let cw_ptr = g_st().cmdline_win.get();
            let cmdline_win_ref = cw_ptr.opt();
            pum_win_row = if let Some(cw) = cmdline_win_ref {
                cw.w_wrow.get()
            } else if ui_has(K_UICMDLINE) {
                0
            } else {
                g_st().cmdline_row.get()
            };
            cursor_col =
                cmdline_win_ref.map_or(0, |cw| cw.w_config.borrow()._cmdline_offset) + cmd_startcol;
            cursor_col %=
                cmdline_win_ref.map_or(g_st().columns_global.get(), |cw| cw.w_view_width.get());
            PUM.with(|s| {
                s.anchor_grid.set(if ui_has(K_UICMDLINE) {
                    -1
                } else {
                    DEFAULT_GRID_HANDLE
                })
            });
        } else {
            pum_win_row = curwin_mut().w_wrow.get();
            let mut wcol = curwin_mut().w_wcol.get();
            if curwin_mut().w_onebuf_opt.wo_cole.get() > 0 && conceal_cursor_line(&curwin_mut()) {
                wcol -= curwin_mut().w_wcol_conceal_off.get();
                if wcol < 0 {
                    wcol = 0;
                }
            }
            if PUM.with(|s| s.rl.get()) {
                cursor_col = curwin_mut().w_view_width.get() - wcol - 1;
            } else {
                cursor_col = wcol;
            }
        }

        if let Some(target_win) = target_win.opt() {
            PUM.with(|s| {
                s.anchor_grid
                    .set(target_win.w_grid.target.get().borrow().handle.get())
            });

            pum_win_row += target_win.w_grid.row_offset.get();

            cursor_col += target_win.w_grid.col_offset.get();

            if target_win.w_grid.target.get() != crate::grid::GridTarget::Default {
                pum_win_row += target_win.w_winrow.get();

                cursor_col += target_win.w_wincol.get();

                if !ui_has(K_UIMULTIGRID) {
                    PUM.with(|s| s.anchor_grid.set(DEFAULT_GRID_HANDLE));
                } else {
                    PUM.with(|s| s.win_row_offset.set(target_win.w_winrow.get()));

                    PUM.with(|s| s.win_col_offset.set(target_win.w_wincol.get()));
                }
            }
        }

        if PUM.with(|s| s.external.get()) {
            if array_changed {
                let mut arr = Array::with_capacity(size as usize);
                for elem in &items[..(size as usize)] {
                    let mut item = Array::with_capacity(4);
                    add_c(&mut item, cstr_as_obj(elem.text_cstr()));
                    add_c(&mut item, cstr_as_obj(elem.kind_cstr()));
                    add_c(&mut item, cstr_as_obj(elem.extra_cstr()));
                    add_c(&mut item, cstr_as_obj(elem.info_cstr()));
                    add_c(&mut arr, array_obj(item));
                }
                ui_call_popupmenu_show(
                    arr,
                    i64::from(selected),
                    i64::from(pum_win_row - PUM.with(|s| s.win_row_offset.get())),
                    i64::from(cursor_col - PUM.with(|s| s.win_col_offset.get())),
                    i64::from(PUM.with(|s| s.anchor_grid.get())),
                );
            } else {
                ui_call_popupmenu_select(i64::from(selected));
                return;
            }
        }

        let mut pvwin: Option<crate::globals::WinPtr> = None;
        for wp in win_iter_ptr(first_win_in_tab(&curtab_mut())) {
            if wp.w_onebuf_opt.wo_pvw.get() {
                pvwin = Some(wp);
                break;
            }
        }

        if let Some(pvwin) = pvwin {
            if pvwin.w_winrow.get() < curwin_mut().w_winrow.get() {
                above_row = pvwin.w_winrow.get() + pvwin.w_height.get();
            } else if pvwin.w_winrow.get()
                > curwin_mut().w_winrow.get() + curwin_mut().w_height.get()
            {
                below_row = pvwin.w_winrow.get();
            }
        }

        pum_compute_vertical_placement(
            size,
            target_win.opt_mut(),
            pum_win_row,
            above_row,
            below_row,
            border_width,
        );

        if border_width == 0
            && (PUM.with(|s| s.height.get()) < 1 || (PUM.with(|s| s.height.get()) == 1 && size > 1))
        {
            return;
        }

        PUM_ITEMS.with(|c| *c.borrow_mut() = Some(items.clone()));
        PUM.with(|s| s.size.set(size));

        if PUM.with(|s| s.external.get()) {
            return;
        }

        pum_compute_size();

        PUM.with(|s| {
            s.scrollbar.set(if PUM.with(|s| s.height.get()) < size {
                1
            } else {
                0
            })
        });

        pum_compute_horizontal_placement(target_win.opt(), cursor_col, border_width);

        if !(pum_set_selected(selected, redo_count) && {
            redo_count += 1;
            redo_count <= 2
        }) {
            break;
        }
    }

    g_st()
        .pum_grid
        .zindex
        .set(if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            K_ZINDEX_CMDLINE_POPUP_MENU
        } else {
            K_ZINDEX_POPUP_MENU
        });
    pum_redraw();
}

/// Computes attributes of text on the popup menu (`pum_compute_text_attrs`).
fn pum_compute_text_attrs(
    text_cstr: &std::ffi::CStr,
    hlf: Hlf,
    user_hlattr: i32,
) -> Option<Vec<i32>> {
    if text_cstr.is_empty()
        || (hlf != HLF_PSI && hlf != HLF_PNI)
        || (win_hl_attr(&curwin_mut(), HLF_PMSI) == win_hl_attr(&curwin_mut(), HLF_PSI)
            && win_hl_attr(&curwin_mut(), HLF_PMNI) == win_hl_attr(&curwin_mut(), HLF_PNI))
    {
        return None;
    }

    let leader: Option<std::borrow::Cow<'_, std::ffi::CStr>> =
        if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
            cmdline_compl_pattern().map(std::borrow::Cow::Owned)
        } else {
            ins_compl_leader().map(std::borrow::Cow::Owned)
        };
    let leader = leader.filter(|s| !s.is_empty())?;

    let in_fuzzy = if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
        cmdline_compl_is_fuzzy()
    } else {
        (get_cot_flags() & K_OPT_COT_FLAG_FUZZY) != 0
    };
    let leader_len = leader.to_bytes().len();

    let mut match_positions: Option<Vec<u32>> = None;
    let mut matched_len: i32 = -1;

    if in_fuzzy {
        match_positions =
            fuzzy_match_str_with_pos(Some(text_cstr.to_bytes()), Some(leader.to_bytes()));
        match_positions.as_ref()?;
    }

    let mut attrs = vec![0; vim_strsize(text_cstr) as usize];
    let mut rem_bytes = text_cstr.to_bytes();
    let mut cell_idx: usize = 0;
    let mut char_pos: u32 = 0;
    let is_select = hlf == HLF_PSI;

    while !rem_bytes.is_empty() {
        let mut new_attr = win_hl_attr(&curwin_mut(), hlf);

        if let Some(ref positions) = match_positions {
            if positions.contains(&char_pos) {
                new_attr = win_hl_attr(&curwin_mut(), if is_select { HLF_PMSI } else { HLF_PMNI });
                new_attr = hl_combine_attr(win_hl_attr(&curwin_mut(), HLF_PMNI), new_attr);
                new_attr = hl_combine_attr(win_hl_attr(&curwin_mut(), hlf), new_attr);
            }
        } else {
            if matched_len < 0 && mb_strnicmp(rem_bytes, leader.to_bytes(), leader_len) == 0 {
                matched_len = leader_len as i32;
            }
            if matched_len > 0 {
                new_attr = win_hl_attr(&curwin_mut(), if is_select { HLF_PMSI } else { HLF_PMNI });
                new_attr = hl_combine_attr(win_hl_attr(&curwin_mut(), HLF_PMNI), new_attr);
                new_attr = hl_combine_attr(win_hl_attr(&curwin_mut(), hlf), new_attr);
                matched_len -= 1;
            }
        }

        new_attr = hl_combine_attr(win_hl_attr(&curwin_mut(), HLF_PNI), new_attr);

        if user_hlattr > 0 {
            new_attr = hl_combine_attr(new_attr, user_hlattr);
        }

        let char_cells = utf_ptr2cells(rem_bytes) as usize;
        for i in 0..char_cells {
            attrs[cell_idx + i] = new_attr;
        }
        cell_idx += char_cells;

        let step = crate::mbyte::utfc_slice2len(rem_bytes).max(1);
        rem_bytes = &rem_bytes[step.min(rem_bytes.len())..];
        char_pos += 1;
    }

    Some(attrs)
}

/// Displays text on the popup menu with specific attributes (`pum_grid_puts_with_attrs`).
fn pum_grid_puts_with_attrs(
    mut col: i32,
    cells: i32,
    text_cstr: &std::ffi::CStr,
    textlen: i32,
    attrs: &[i32],
) {
    let col_start = col;
    let text_bytes = text_cstr.to_bytes();
    let max_off = if textlen < 0 {
        text_bytes.len()
    } else {
        (textlen as usize).min(text_bytes.len())
    };
    let mut off = 0usize;

    while off < max_off {
        let cur_cstr = &text_cstr[off..];
        let char_len = utfc_ptr2len(cur_cstr.to_bytes());
        let attr_idx = if PUM.with(|s| s.rl.get()) {
            col_start + cells - col - 1
        } else {
            col - col_start
        };
        let attr = attrs[attr_idx as usize];
        grid_line_puts(col, cur_cstr, char_len, attr);
        col += utf_ptr2cells(cur_cstr.to_bytes());
        off += (char_len.max(1) as usize).min(max_off - off);
    }
}

#[inline]
fn pum_align_order(order: &mut [i32; 3]) {
    let is_default = opt_vars().cia_flags.get() == 0;
    order[0] = if is_default {
        CPT_ABBR
    } else {
        (opt_vars().cia_flags.get() / 100) as i32
    };
    order[1] = if is_default {
        CPT_KIND
    } else {
        ((opt_vars().cia_flags.get() / 10) % 10) as i32
    };
    order[2] = if is_default {
        CPT_MENU
    } else {
        (opt_vars().cia_flags.get() % 10) as i32
    };
}

#[inline]
fn pum_get_item(index: i32, type_: i32) -> Option<std::ffi::CString> {
    let item = pum_item(index);
    match type_ {
        CPT_ABBR => item.pum_text,
        CPT_KIND => item.pum_kind,
        CPT_MENU => item.pum_extra,
        _ => None,
    }
}

#[inline]
fn pum_user_attr_combine(idx: i32, type_: i32, attr: i32) -> i32 {
    let item = pum_item(idx);
    let user_attr = [item.pum_user_abbr_hlattr, item.pum_user_kind_hlattr];
    let u = user_attr[type_ as usize];
    if u > 0 {
        hl_combine_attr(attr, u)
    } else {
        attr
    }
}

/// Redraw the popup menu, using `pum_first` and `pum_selected` (`pum_redraw`).
pub fn pum_redraw() {
    let mut row: i32 = 0;
    let attr_scroll = win_hl_attr(&curwin_mut(), HLF_PSB);
    let attr_thumb = win_hl_attr(&curwin_mut(), HLF_PST);
    let mut thumb_pos: i32 = 0;
    let mut thumb_height: i32 = 1;
    let fcs_trunc: Schar = if PUM.with(|s| s.rl.get()) {
        curwin_mut().w_p_fcs_chars.get().truncrl
    } else {
        curwin_mut().w_p_fcs_chars.get().trunc
    };

    const HLFS_NORM: [Hlf; 3] = [HLF_PNI, HLF_PNK, HLF_PNX];
    const HLFS_SEL: [Hlf; 3] = [HLF_PSI, HLF_PSK, HLF_PSX];

    let mut grid_width = PUM.with(|s| s.width.get());
    let mut col_off: i32 = 0;
    let mut extra_space = false;
    if PUM.with(|s| s.rl.get()) {
        col_off = PUM.with(|s| s.width.get()) - 1;
        debug_assert!((g_st().state_global.get() & MODE_CMDLINE) == 0);
        let win_end_col = curwin_mut().w_wincol.get() + curwin_mut().w_view_width.get();
        if PUM.with(|s| s.col.get()) < win_end_col - 1 {
            grid_width += 1;
            extra_space = true;
        }
    } else {
        let min_col: i32 = 0;
        if PUM.with(|s| s.col.get()) > min_col {
            grid_width += 1;
            col_off = 1;
            extra_space = true;
        }
    }

    let mut fconfig = WIN_CONFIG_INIT;
    let border_width = pum_border_width();
    let mut border_attr: i32 = 0;
    let mut border_char: Schar = 0;
    let fill_char: Schar = schar_from_ascii(b' ');
    let has_border = border_width > 0;

    if border_width > 0 {
        let mut err = Error::new();
        if !parse_winborder(
            Some(&mut fconfig),
            &opt_vars().p_pumborder.as_bytes(),
            &mut err,
        ) {
            if err.err_type != ErrorType::ErrorTypeNone {
                emsg(err.msg_cstr().unwrap_or(c""));
            }
            api_clear_error(&mut err);
            return;
        }

        if opt_vars().p_pumborder.as_bytes() == OPT_WINBORDER_VALUES[3].to_bytes() {
            fconfig.shadow = true;
            let blend = syn_check_group(b"PmenuShadow");
            let through = syn_check_group(b"PmenuShadowThrough");
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
        if PUM.with(|s| s.scrollbar.get()) != 0 {
            let bchars = &fconfig.border_chars[3];
            let blen = bchars.iter().position(|&b| b == 0).unwrap_or(bchars.len());
            border_char = crate::grid::schar_from_bytes(&bchars[..blen]);
            border_attr = fconfig.border_attr[3];
        }
    }

    if PUM.with(|s| s.scrollbar.get()) > 0 && (!fconfig.border || fconfig.shadow) {
        grid_width += 1;
        if PUM.with(|s| s.rl.get()) {
            col_off += 1;
        }
    }
    g_st()
        .pum_grid
        .blending
        .set(opt_vars().p_pb.get() > 0 || fconfig.shadow);
    grid_assign_handle(&g_st().pum_grid);

    PUM.with(|s| s.left_col.set(PUM.with(|s| s.col.get()) - col_off));
    PUM.with(|s| s.right_col.set(PUM.with(|s| s.left_col.get()) + grid_width));
    let moved = ui_comp_put_grid(
        &g_st().pum_grid,
        PUM.with(|s| s.row.get()),
        PUM.with(|s| s.left_col.get()),
        PUM.with(|s| s.height.get()) + border_width,
        grid_width + border_width,
        false,
        true,
    );
    let invalid_grid = moved || PUM.with(|s| s.invalid.get());
    PUM.with(|s| s.invalid.set(false));
    g_st().must_redraw_pum.set(false);

    if !crate::grid::grid_allocated(&g_st().pum_grid)
        || g_st().pum_grid.rows.get() != PUM.with(|s| s.height.get()) + border_width
        || g_st().pum_grid.cols.get() != grid_width + border_width
    {
        grid_alloc(
            &g_st().pum_grid,
            PUM.with(|s| s.height.get()) + border_width,
            grid_width + border_width,
            !invalid_grid,
            false,
        );
        ui_call_grid_resize(
            i64::from(g_st().pum_grid.handle.get()),
            i64::from(g_st().pum_grid.cols.get()),
            i64::from(g_st().pum_grid.rows.get()),
        );
    } else if invalid_grid {
        grid_invalidate(&g_st().pum_grid);
    }
    if ui_has(K_UIMULTIGRID) {
        pum_pos_ui_flush(true);
    }

    let scroll_range = PUM.with(|s| s.size.get()) - PUM.with(|s| s.height.get());
    if fconfig.border {
        grid_draw_border(&g_st().pum_grid, &mut fconfig, None, 0, None, 0);
        if !fconfig.shadow {
            row += 1;
            col_off += 1;
        }
    }

    PUM.with(|s| s.first.set(PUM.with(|s| s.first.get()).min(scroll_range)));

    if PUM.with(|s| s.scrollbar.get()) != 0 {
        thumb_height = PUM.with(|s| s.height.get()) * PUM.with(|s| s.height.get())
            / PUM.with(|s| s.size.get());
        if thumb_height == 0 {
            thumb_height = 1;
        }
        thumb_pos = (PUM.with(|s| s.first.get()) * (PUM.with(|s| s.height.get()) - thumb_height)
            + scroll_range / 2)
            / scroll_range;
    }

    for i in 0..PUM.with(|s| s.height.get()) {
        let idx = i + PUM.with(|s| s.first.get());
        let selected = idx == PUM.with(|s| s.selected.get());
        let hlfs = if selected { &HLFS_SEL } else { &HLFS_NORM };
        let trunc_attr = win_hl_attr(&curwin_mut(), if selected { HLF_PSI } else { HLF_PNI });
        let mut hlf = hlfs[0];
        let mut attr = win_hl_attr(&curwin_mut(), hlf);
        attr = hl_combine_attr(win_hl_attr(&curwin_mut(), HLF_PNI), attr);

        screengrid_line_start(&g_st().pum_grid, row, 0);

        if extra_space {
            if PUM.with(|s| s.rl.get()) {
                grid_line_puts(col_off + 1, c" ", 1, attr);
            } else {
                grid_line_puts(col_off - 1, c" ", 1, attr);
            }
        }

        let mut grid_col = col_off;
        let mut totwidth: i32 = 0;
        let mut need_fcs_trunc = false;
        let mut order = [0_i32; 3];
        let items_width_array = [
            PUM.with(|s| s.base_width.get()),
            PUM.with(|s| s.kind_width.get()),
            PUM.with(|s| s.extra_width.get()),
        ];
        pum_align_order(&mut order);
        let basic_width = items_width_array[order[0] as usize];
        let last_isabbr = order[2] == CPT_ABBR;
        let mut orig_attr: i32 = -1;

        for j in 0..3 {
            let item_type = order[j];
            hlf = hlfs[item_type as usize];
            attr = win_hl_attr(&curwin_mut(), hlf);
            attr = hl_combine_attr(win_hl_attr(&curwin_mut(), HLF_PNI), attr);
            orig_attr = attr;
            if item_type < 2 {
                attr = pum_user_attr_combine(idx, item_type, attr);
            }
            let mut width: i32 = 0;
            let item_cstr = pum_get_item(idx, item_type);

            let next_isempty = j + 1 >= 3 || pum_get_item(idx, order[j + 1]).is_none();

            if let Some(item_cstr) = item_cstr {
                let item_bytes = item_cstr.to_bytes_with_nul();
                let mut p_idx = 0usize;
                let mut s_idx: Option<usize> = None;
                loop {
                    let seg_start = *s_idx.get_or_insert(p_idx);
                    let w = crate::mbyte::slice2cells(&item_bytes[p_idx..]);
                    if item_bytes[p_idx] != 0
                        && item_bytes[p_idx] != TAB
                        && totwidth + w <= PUM.with(|s| s.width.get())
                    {
                        width += w;
                        p_idx += crate::mbyte::utfc_slice2len(&item_bytes[p_idx..]).max(1);
                        continue;
                    }
                    let width_limit = PUM.with(|s| s.width.get());

                    let seg_cstr = crate::memory::bytes_to_cstring(&item_bytes[seg_start..p_idx]);
                    let mut st_bytes = transstr(seg_cstr.as_bytes(), true).into_bytes_with_nul();

                    let attrs = if item_type == CPT_ABBR {
                        pum_compute_text_attrs(
                            crate::memory::bytes_as_cstr(&st_bytes),
                            hlf,
                            pum_item(idx).pum_user_abbr_hlattr,
                        )
                    } else {
                        None
                    };

                    if PUM.with(|s| s.rl.get()) {
                        let st_slice = &st_bytes[..st_bytes.len().saturating_sub(1)];
                        let mut rt_bytes = reverse_text(st_slice).into_bytes_with_nul();
                        let mut rt_idx = 0usize;
                        let mut cells =
                            mb_string2cells(&rt_bytes[..rt_bytes.len().saturating_sub(1)]) as i32;
                        let pad = if next_isempty { 0 } else { 2 };
                        if width_limit - totwidth < cells + pad {
                            need_fcs_trunc = true;
                        }

                        if grid_col - cells < col_off - width_limit {
                            loop {
                                cells -= crate::mbyte::utf_slice2cells(&rt_bytes[rt_idx..]);
                                rt_idx += crate::mbyte::utfc_slice2len(&rt_bytes[rt_idx..]).max(1);
                                if grid_col - cells >= col_off - width_limit {
                                    break;
                                }
                            }

                            if grid_col - cells > col_off - width_limit {
                                rt_idx -= 1;
                                rt_bytes[rt_idx] = b'<';
                                cells += 1;
                            }
                        }

                        let rt_cstr = crate::memory::bytes_as_cstr(&rt_bytes[rt_idx..]);
                        if let Some(ref attrs_vec) = attrs {
                            pum_grid_puts_with_attrs(
                                grid_col - cells + 1,
                                cells,
                                rt_cstr,
                                -1,
                                attrs_vec,
                            );
                        } else {
                            grid_line_puts(grid_col - cells + 1, rt_cstr, -1, attr);
                        }

                        grid_col -= width;
                    } else {
                        let mut cells =
                            mb_string2cells(&st_bytes[..st_bytes.len().saturating_sub(1)]) as i32;
                        let pad = if next_isempty { 0 } else { 2 };
                        if width_limit - totwidth < cells + pad {
                            need_fcs_trunc = true;
                        }
                        if need_fcs_trunc {
                            let available_cells = width_limit - totwidth;
                            let mut p_end = 0usize;
                            let mut displayed: i32 = 0;
                            while st_bytes[p_end] != 0 {
                                let char_cells = crate::mbyte::utf_slice2cells(&st_bytes[p_end..]);
                                if displayed + char_cells > available_cells {
                                    break;
                                }
                                displayed += char_cells;
                                p_end += crate::mbyte::utfc_slice2len(&st_bytes[p_end..]).max(1);
                            }
                            st_bytes[p_end] = 0;
                            cells = displayed;
                            width = displayed;
                        }

                        let st_cstr = crate::memory::bytes_as_cstr(&st_bytes);
                        if let Some(ref attrs_vec) = attrs {
                            pum_grid_puts_with_attrs(grid_col, cells, st_cstr, -1, attrs_vec);
                        } else {
                            grid_line_puts(grid_col, st_cstr, -1, attr);
                        }

                        grid_col += width;
                    }

                    if item_bytes[p_idx] != TAB {
                        break;
                    }

                    if PUM.with(|s| s.rl.get()) {
                        grid_line_puts(grid_col - 1, c"  ", 2, attr);
                        grid_col -= 2;
                    } else {
                        grid_line_puts(grid_col, c"  ", 2, attr);
                        grid_col += 2;
                    }
                    totwidth += 2;
                    s_idx = None;
                    width = 0;
                    p_idx += crate::mbyte::utfc_slice2len(&item_bytes[p_idx..]).max(1);
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
                    && (j == 1 || (j == 0 && pum_get_item(idx, order[j + 2]).is_none())))
                || basic_width + n >= PUM.with(|s| s.width.get())
            {
                break;
            }

            if PUM.with(|s| s.rl.get()) {
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

        if PUM.with(|s| s.rl.get()) {
            let lcol = col_off - PUM.with(|s| s.width.get()) + 1;
            grid_line_fill(lcol, grid_col + 1, schar_from_ascii(b' '), orig_attr);
            if need_fcs_trunc {
                g_st().linebuf_char.borrow_mut()[lcol as usize] = if fcs_trunc != 0 {
                    fcs_trunc
                } else {
                    schar_from_ascii(b'<')
                };
                g_st().linebuf_attr.borrow_mut()[lcol as usize] = trunc_attr;
                if PUM.with(|s| s.width.get()) > 1
                    && g_st().linebuf_char.borrow()[(lcol + 1) as usize] == 0
                {
                    g_st().linebuf_char.borrow_mut()[(lcol + 1) as usize] = schar_from_ascii(b' ');
                }
            }
        } else {
            let rcol = col_off + PUM.with(|s| s.width.get());
            grid_line_fill(grid_col, rcol, schar_from_ascii(b' '), orig_attr);
            if need_fcs_trunc {
                if PUM.with(|s| s.width.get()) > 1
                    && g_st().linebuf_char.borrow()[(rcol - 1) as usize] == 0
                {
                    g_st().linebuf_char.borrow_mut()[(rcol - 2) as usize] = schar_from_ascii(b' ');
                }
                g_st().linebuf_char.borrow_mut()[(rcol - 1) as usize] = if fcs_trunc != 0 {
                    fcs_trunc
                } else {
                    schar_from_ascii(b'>')
                };
                g_st().linebuf_attr.borrow_mut()[(rcol - 1) as usize] = trunc_attr;
            }
        }

        if PUM.with(|s| s.scrollbar.get()) > 0 {
            let thumb = i >= thumb_pos && i < thumb_pos + thumb_height;
            let scrollbar_col = col_off
                + if PUM.with(|s| s.rl.get()) {
                    -PUM.with(|s| s.width.get())
                } else {
                    PUM.with(|s| s.width.get())
                };
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

/// Set the informational text in the preview buffer (`pum_preview_set_text`).
fn pum_preview_set_text(
    win: &WinInfoInfo,
    info: Option<&[u8]>,
    lnum: &mut Linenr,
    max_width: &mut i32,
) {
    let mut err = Error::new();
    let mut replacement = Array::new();
    let buf = win.w_buffer.get();
    buf.b_p_ma.set(true);

    if let Some(info_bytes) = info.map(crate::memory::bytes_until_nul) {
        let mut rem_opt: Option<&[u8]> = Some(info_bytes);
        while let Some(rem) = rem_opt {
            let (curr_slice, next_opt) = if let Some(nl_idx) = rem.iter().position(|&b| b == b'\n')
            {
                (&rem[..nl_idx], Some(&rem[nl_idx + 1..]))
            } else {
                (rem, None)
            };
            if curr_slice.is_empty() && next_opt.is_none() {
                break;
            }
            let curr_cstr = std::ffi::CString::new(curr_slice).unwrap_or_default();
            let save_wrap = win.w_onebuf_opt.wo_wrap.get();
            win.w_onebuf_opt.wo_wrap.set(false);
            let line_width = win_linetabsize(win, 0, &curr_cstr, MAXCOL);
            win.w_onebuf_opt.wo_wrap.set(save_wrap);
            *max_width = (*max_width).max(line_width);
            replacement.push(string_obj(cstr_to_string(Some(&*curr_cstr))));
            *lnum += 1;
            rem_opt = next_opt;
        }
    }

    let original_textlock = g_st().textlock.get();
    g_st().textlock.set(0);
    nvim_buf_set_lines(
        0,
        buf.b_fnum.get(),
        0,
        -1,
        false,
        replacement.clone(),
        &mut err,
    );
    g_st().textlock.set(original_textlock);
    if err.err_type != ErrorType::ErrorTypeNone {
        emsg(err.msg_cstr().unwrap_or(c""));
        api_clear_error(&mut err);
    }
    api_free_array(replacement);
    buf.b_p_ma.set(false);
}

/// Adjust floating info preview window position (`pum_adjust_info_position`).
fn pum_adjust_info_position(wp: &WinInfoInfo, width: i32) -> bool {
    let border_width = pum_border_width();
    let col = PUM.with(|s| s.col.get())
        + PUM.with(|s| s.width.get())
        + 1
        + border_width.max(PUM.with(|s| s.scrollbar.get()));
    let right_extra = g_st().columns_global.get() - col;
    let left_extra = PUM.with(|s| s.col.get()) - 2;

    let max_extra = right_extra.max(left_extra);
    if max_extra < 10 {
        wp.w_config.borrow_mut().hide = true;
        return false;
    }

    if right_extra > width {
        wp.w_config.borrow_mut().width = width;
        wp.w_config.borrow_mut().col = f64::from(col - 1);
    } else if left_extra > width {
        wp.w_config.borrow_mut().width = width;
        wp.w_config.borrow_mut().col = f64::from(PUM.with(|s| s.col.get()) - width - 1);
    } else {
        let place_in_right = right_extra > left_extra;
        wp.w_config.borrow_mut().width = max_extra;
        let col_val = f64::from(if place_in_right {
            col - 1
        } else {
            PUM.with(|s| s.col.get()) - max_extra - 1
        });
        wp.w_config.borrow_mut().col = col_val;
    }
    wp.w_config.borrow_mut().anchor = 0;
    let count = wp.buf_mut().b_ml.ml_line_count.get();
    wp.w_view_width.set(wp.w_config.borrow().width);
    let height = plines_m_win(wp, wp.w_topline.get(), count, g_st().rows_global.get());
    wp.w_config.borrow_mut().height = height;
    wp.w_config.borrow_mut().row = f64::from(PUM.with(|s| s.row.get()));
    wp.w_config.borrow_mut().hide = false;
    let cfg = wp.w_config.borrow().clone();
    win_config_float(wp, cfg);
    true
}

/// Used for `nvim__complete_set` (`pum_set_info`).
pub fn pum_set_info(selected: i32, info: Option<&[u8]>) -> Option<crate::globals::WinPtr> {
    if !PUM.with(|s| s.is_visible.get()) || !compl_match_curr_select(selected) {
        return None;
    }
    block_autocmds();
    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() + (1));
    g_st().no_u_sync.set(g_st().no_u_sync.get() + (1));
    let wp_ref = if let Some(wp) = win_float_find(K_WIN_INFO) {
        wp
    } else {
        let wp_ref = win_float_special(false, true, K_WIN_INFO)?;
        wp_ref.w_topline.set(1);
        wp_ref.w_onebuf_opt.wo_wfb.set(true);
        wp_ref
    };
    let mut lnum: Linenr = 0;
    let mut max_info_width: i32 = 0;
    pum_preview_set_text(&wp_ref, info, &mut lnum, &mut max_info_width);
    g_st().no_u_sync.set(g_st().no_u_sync.get() - (1));
    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() - (1));
    redraw_later(Some(&wp_ref), UPD_NOT_VALID);

    let ok = pum_adjust_info_position(&wp_ref, max_info_width);
    unblock_autocmds();
    ok.then_some(wp_ref)
}

/// Set the index of the currently selected item (`pum_set_selected`).
fn pum_set_selected(n: i32, repeat: i32) -> bool {
    let mut resized = false;
    let mut context = PUM.with(|s| s.height.get()) / 2;
    let prev_selected = PUM.with(|s| s.selected.get());

    PUM.with(|s| s.selected.set(n));
    let scroll_offset = PUM.with(|s| s.selected.get()) - PUM.with(|s| s.height.get());
    let cur_cot_flags = get_cot_flags();
    let use_float = (cur_cot_flags & K_OPT_COT_FLAG_POPUP) != 0;

    if use_float
        && (PUM.with(|s| s.selected.get()) < 0
            || pum_item(PUM.with(|s| s.selected.get())).pum_info.is_none())
        && let Some(wp) = win_float_find(K_WIN_INFO)
    {
        wp.w_config.borrow_mut().hide = true;
        let cfg = wp.w_config.borrow().clone();
        win_config_float(&wp, cfg);
    }

    if PUM.with(|s| s.selected.get()) >= 0
        && PUM.with(|s| s.selected.get()) < PUM.with(|s| s.size.get())
    {
        if PUM.with(|s| s.first.get()) > PUM.with(|s| s.selected.get()) - 4 {
            if PUM.with(|s| s.first.get()) > PUM.with(|s| s.selected.get()) - 2 {
                PUM.with(|s| {
                    s.first
                        .set(s.first.get() - (PUM.with(|s| s.height.get()) - 2))
                });
                if PUM.with(|s| s.first.get()) < 0 {
                    PUM.with(|s| s.first.set(0));
                } else if PUM.with(|s| s.first.get()) > PUM.with(|s| s.selected.get()) {
                    PUM.with(|s| s.first.set(PUM.with(|s| s.selected.get())));
                }
            } else {
                PUM.with(|s| s.first.set(PUM.with(|s| s.selected.get())));
            }
        } else if PUM.with(|s| s.first.get()) < scroll_offset + 5 {
            if PUM.with(|s| s.first.get()) < scroll_offset + 3 {
                PUM.with(|s| {
                    s.first.set(
                        (PUM.with(|s| s.first.get()) + PUM.with(|s| s.height.get()) - 2)
                            .max(scroll_offset + 1),
                    )
                });
            } else {
                PUM.with(|s| s.first.set(scroll_offset + 1));
            }
        }

        context = context.min(3);

        if PUM.with(|s| s.height.get()) > 2 {
            if PUM.with(|s| s.first.get()) > PUM.with(|s| s.selected.get()) - context {
                PUM.with(|s| {
                    s.first
                        .set((PUM.with(|s| s.selected.get()) - context).max(0))
                });
            } else if PUM.with(|s| s.first.get())
                < PUM.with(|s| s.selected.get()) + context - PUM.with(|s| s.height.get()) + 1
            {
                PUM.with(|s| {
                    s.first.set(
                        PUM.with(|s| s.selected.get()) + context - PUM.with(|s| s.height.get()) + 1,
                    )
                });
            }
        }
        PUM.with(|s| {
            s.first.set(
                PUM.with(|s| s.first.get())
                    .min(PUM.with(|s| s.size.get()) - PUM.with(|s| s.height.get())),
            )
        });

        if pum_item(PUM.with(|s| s.selected.get())).pum_info.is_some()
            && g_st().rows_global.get() > 10
            && repeat <= 1
            && (cur_cot_flags & (K_OPT_COT_FLAG_POPUP | K_OPT_COT_FLAG_PREVIEW)) != 0
            && !((cur_cot_flags & K_OPT_COT_FLAG_PREVIEW) != 0
                && !g_st().cmdwin_buf.get().is_null())
        {
            let curwin_save = curwin_ptr();
            let curtab_save = g_st().curtab.get().clone();

            if use_float {
                block_autocmds();
            }

            g_st().g_do_tagpreview.set(3);
            if opt_vars().p_pvh.get() > 0
                && opt_vars().p_pvh.get() < i64::from(g_st().g_do_tagpreview.get())
            {
                g_st().g_do_tagpreview.set(opt_vars().p_pvh.get() as i32);
            }
            g_st()
                .redrawing_disabled
                .set(g_st().redrawing_disabled.get() + (1));
            g_st().no_u_sync.set(g_st().no_u_sync.get() + (1));

            if !use_float {
                resized = prepare_tagpreview(false, false);
            } else if let Some(wp) = win_float_find(K_WIN_INFO) {
                win_enter(&wp, false);
            } else if win_float_special(true, true, K_WIN_INFO).is_some() {
                resized = true;
            }

            g_st().no_u_sync.set(g_st().no_u_sync.get() - (1));
            g_st()
                .redrawing_disabled
                .set(g_st().redrawing_disabled.get() - (1));
            g_st().g_do_tagpreview.set(0);

            if curwin_mut().w_onebuf_opt.wo_pvw.get() || curwin_mut().w_kind.get() == K_WIN_INFO {
                let mut res = true;
                if !resized
                    && curbuf_mut().b_nwindows.get() == 1
                    && curbuf_mut().fname_opt().is_none()
                    && bt_nofile(Some(&crate::globals::curbuf_mut()))
                    && curbuf_mut().b_p_bh.first_byte() == b'w'
                {
                    buf_clear();
                } else {
                    g_st().no_u_sync.set(g_st().no_u_sync.get() + (1));
                    res = do_ecmd(0, None, None, None, ECMD_ONE, 0, None);
                    g_st().no_u_sync.set(g_st().no_u_sync.get() - (1));

                    if res {
                        set_option_value_give_err(K_OPT_SWAPFILE, boolean_obj(false), OPT_LOCAL);
                        set_option_value_give_err(K_OPT_BUFLISTED, boolean_obj(false), OPT_LOCAL);
                        set_option_value_give_err(
                            K_OPT_BUFTYPE,
                            cstr_as_obj(Some(c"nofile")),
                            OPT_LOCAL,
                        );
                        set_option_value_give_err(
                            K_OPT_BUFHIDDEN,
                            cstr_as_obj(Some(c"wipe")),
                            OPT_LOCAL,
                        );
                        set_option_value_give_err(K_OPT_DIFF, boolean_obj(false), OPT_LOCAL);
                    }
                }

                if res {
                    let mut lnum: Linenr = 0;
                    let mut max_info_width: i32 = 0;

                    let sel_item = pum_item(PUM.with(|s| s.selected.get()));
                    pum_preview_set_text(
                        &curwin_mut(),
                        sel_item.info_cstr().map(CStr::to_bytes),
                        &mut lnum,
                        &mut max_info_width,
                    );

                    if repeat == 0 && !use_float {
                        lnum = lnum.min(opt_vars().p_pvh.get() as Linenr);
                        if curwin_mut().w_height.get() < lnum {
                            win_setheight(lnum);
                            resized = true;
                        }
                    }

                    curbuf_mut().b_changed.set(0);
                    curbuf_mut().b_p_ma.set(false);
                    if PUM.with(|s| s.selected.get()) != prev_selected {
                        curwin_mut().w_topline.set(1);
                    } else if curwin_mut().w_topline.get() > curbuf_mut().b_ml.ml_line_count.get() {
                        curwin_mut()
                            .w_topline
                            .set(curbuf_mut().b_ml.ml_line_count.get());
                    }
                    curwin_mut().w_cursor.set_lnum(1);
                    curwin_mut().w_cursor.set_col(0);

                    if use_float
                        && !pum_adjust_info_position(&curwin_mut(), max_info_width)
                        && win_valid(curwin_save.opt())
                    {
                        win_enter(curwin_save.borrow_mut(), false);
                    }

                    if (curwin_ptr() != curwin_save && win_valid(curwin_save.opt()))
                        || (g_st().curtab.get() != curtab_save && valid_tabpage(curtab_save.opt()))
                    {
                        if g_st().curtab.get() != curtab_save && valid_tabpage(curtab_save.opt()) {
                            goto_tabpage_tp(curtab_save.opt(), false, false);
                        }

                        if ins_compl_active() && !resized {
                            curwin_mut().w_redr_status.set(false);
                        }

                        validate_cursor(&curwin_mut());
                        redraw_later(Some(&crate::globals::curwin_mut()), UPD_SOME_VALID);

                        if resized && win_valid(curwin_save.opt()) {
                            g_st().no_u_sync.set(g_st().no_u_sync.get() + (1));
                            win_enter(curwin_save.borrow_mut(), true);
                            g_st().no_u_sync.set(g_st().no_u_sync.get() - (1));
                            update_topline(&curwin_mut());
                        }

                        let save_pum_is_drawn = PUM.with(|s| s.is_drawn.get());

                        PUM.with(|s| s.is_visible.set(false));
                        PUM.with(|s| s.is_drawn.set(false));
                        update_screen();
                        PUM.with(|s| s.is_visible.set(true));
                        PUM.with(|s| s.is_drawn.set(save_pum_is_drawn));

                        if !resized && win_valid(curwin_save.opt()) {
                            g_st().no_u_sync.set(g_st().no_u_sync.get() + (1));
                            win_enter(curwin_save.borrow_mut(), true);
                            g_st().no_u_sync.set(g_st().no_u_sync.get() - (1));
                        }

                        PUM.with(|s| s.is_visible.set(false));
                        PUM.with(|s| s.is_drawn.set(false));
                        update_screen();
                        PUM.with(|s| s.is_visible.set(true));
                        PUM.with(|s| s.is_drawn.set(save_pum_is_drawn));
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

/// Undisplay the popup menu (`pum_undisplay`).
pub fn pum_undisplay(immediate: bool) {
    PUM.with(|s| s.is_visible.set(false));
    PUM_ITEMS.with(|c| *c.borrow_mut() = None);
    g_st().must_redraw_pum.set(false);

    if immediate {
        pum_check_clear();
    }
}

pub fn pum_check_clear() {
    if !PUM.with(|s| s.is_visible.get()) && PUM.with(|s| s.is_drawn.get()) {
        if PUM.with(|s| s.external.get()) {
            ui_call_popupmenu_hide();
        } else {
            ui_comp_remove_grid(&g_st().pum_grid);
            if ui_has(K_UIMULTIGRID) {
                ui_call_win_close(i64::from(g_st().pum_grid.handle.get()));
                ui_call_grid_destroy(i64::from(g_st().pum_grid.handle.get()));
            }
            grid_free(&g_st().pum_grid);
        }
        PUM.with(|s| s.is_drawn.set(false));
        PUM.with(|s| s.external.set(false));
        win_float_close(K_WIN_INFO);
    }
}

/// Clear the popup menu (`pum_clear`).
pub fn pum_clear() {
    PUM.with(|s| s.first.set(0));
}

/// Return true if the popup menu is displayed (`pum_visible`).
pub fn pum_visible() -> bool {
    PUM.with(|s| s.is_visible.get())
}

/// Return true if the popup menu is displayed and drawn on the grid (`pum_drawn`).
pub fn pum_drawn() -> bool {
    pum_visible() && !PUM.with(|s| s.external.get())
}

/// Screen was cleared, need to redraw next time (`pum_invalidate`).
pub fn pum_invalidate() {
    PUM.with(|s| s.invalid.set(true));
}

pub fn pum_ext_select_item(item: i32, insert: bool, finish: bool) {
    if !pum_visible() || item < -1 || item >= PUM.with(|s| s.size.get()) {
        return;
    }
    g_st().pum_want.active.set(true);
    g_st().pum_want.item.set(item);
    g_st().pum_want.insert.set(insert);
    g_st().pum_want.finish.set(finish);
}

/// Gets the height of the menu (`pum_get_height`).
pub fn pum_get_height() -> i32 {
    if PUM.with(|s| s.external.get()) {
        let ui_pum_height = ui_pum_get_height();
        if ui_pum_height != 0 {
            return ui_pum_height;
        }
    }
    PUM.with(|s| s.height.get())
}

/// Add size information about the pum to `dict` (`pum_set_event_info`).
pub fn pum_set_event_info(dict: &TvDict) {
    if !pum_visible() {
        return;
    }
    let mut w: f64 = 0.0;
    let mut h: f64 = 0.0;
    let mut r: f64 = 0.0;
    let mut c: f64 = 0.0;
    if !ui_pum_get_pos(&mut w, &mut h, &mut r, &mut c) {
        w = f64::from(PUM.with(|s| s.width.get()));
        h = f64::from(PUM.with(|s| s.height.get()));
        r = f64::from(PUM.with(|s| s.row.get()));
        c = f64::from(PUM.with(|s| s.col.get()));
    }
    tv_dict_add_float(dict, c"height", h);
    tv_dict_add_float(dict, c"width", w);
    tv_dict_add_float(dict, c"row", r);
    tv_dict_add_float(dict, c"col", c);
    tv_dict_add_nr(dict, c"size", i64::from(PUM.with(|s| s.size.get())));
    tv_dict_add_bool(
        dict,
        c"scrollbar",
        if PUM.with(|s| s.scrollbar.get()) != 0 {
            K_BOOL_VAR_TRUE_CONST
        } else {
            K_BOOL_VAR_FALSE_CONST
        },
    );
}

fn pum_position_at_mouse(min_width: i32) {
    let min_row: i32 = 0;
    let min_col: i32 = 0;
    let mut max_row = g_st().rows_global.get();
    let mut max_col = g_st().columns_global.get();
    let mut grid = g_st().mouse_grid.get();
    let mut row = g_st().mouse_row.get();
    let mut col = g_st().mouse_col.get();
    PUM.with(|s| s.win_row_offset.set(0));
    PUM.with(|s| s.win_col_offset.set(0));

    if ui_has(K_UIMULTIGRID) && grid == 0 {
        mouse_find_win_outer(&mut grid, &mut row, &mut col);
    }
    if grid > 1
        && let Some(wp) = get_win_by_grid_handle(grid)
    {
        row += wp.w_winrow.get();
        col += wp.w_wincol.get();
        PUM.with(|s| s.win_row_offset.set(wp.w_winrow.get()));
        PUM.with(|s| s.win_col_offset.set(wp.w_wincol.get()));

        if wp.w_view_height.get() > 0 || wp.w_view_width.get() > 0 {
            max_row = (g_st().rows_global.get() - wp.w_winrow.get())
                .max(wp.w_winrow.get() + wp.w_view_height.get());
            max_col = (g_st().columns_global.get() - wp.w_wincol.get())
                .max(wp.w_wincol.get() + wp.w_view_width.get());
        }
    }
    if g_st().pum_grid.handle.get() != 0 && grid == g_st().pum_grid.handle.get() {
        row += PUM.with(|s| s.row.get());
        col += PUM.with(|s| s.left_col.get());
    } else {
        PUM.with(|s| s.anchor_grid.set(grid));
    }

    let border_width = pum_border_width();
    let border_height = border_width;
    if max_row - row > PUM.with(|s| s.size.get()) + border_height || max_row - row > row - min_row {
        PUM.with(|s| s.above.set(false));
        PUM.with(|s| s.row.set(row + 1));
        if PUM.with(|s| s.height.get()) + border_height > max_row - PUM.with(|s| s.row.get()) {
            PUM.with(|s| {
                s.height
                    .set(max_row - PUM.with(|s| s.row.get()) - border_height)
            });
        }
    } else {
        PUM.with(|s| s.above.set(true));
        PUM.with(|s| s.row.set(row - PUM.with(|s| s.size.get()) - border_height));
        if PUM.with(|s| s.row.get()) < min_row {
            PUM.with(|s| {
                s.height
                    .set(s.height.get() + (PUM.with(|s| s.row.get()) - min_row))
            });
            PUM.with(|s| s.row.set(min_row));
        }
    }

    if PUM.with(|s| s.rl.get()) {
        if col - min_col + 1 >= PUM.with(|s| s.base_width.get()) + border_width
            || col - min_col + 1 > min_width + border_width
        {
            PUM.with(|s| s.col.set(col));
        } else {
            PUM.with(|s| {
                s.col.set(
                    min_col
                        + (PUM.with(|s| s.base_width.get()) + border_width)
                            .min(min_width + border_width)
                        - 1,
                )
            });
        }
        PUM.with(|s| {
            s.width
                .set(PUM.with(|s| s.col.get()) - min_col + 1 - border_width)
        });
    } else {
        if max_col - col >= PUM.with(|s| s.base_width.get()) + border_width
            || max_col - col > min_width + border_width
        {
            PUM.with(|s| s.col.set(col));
        } else {
            PUM.with(|s| {
                s.col.set(
                    max_col
                        - (PUM.with(|s| s.base_width.get()) + border_width)
                            .min(min_width + border_width),
                )
            });
        }
        PUM.with(|s| {
            s.width
                .set(max_col - PUM.with(|s| s.col.get()) - border_width)
        });
    }

    PUM.with(|s| {
        s.width.set(
            PUM.with(|s| s.width.get())
                .min(PUM.with(|s| s.base_width.get()) + 1),
        )
    });
}

/// Select the pum entry at the mouse position (`pum_select_mouse_pos`).
fn pum_select_mouse_pos() {
    let mut grid = g_st().mouse_grid.get();
    let mut row = g_st().mouse_row.get();
    let mut col = g_st().mouse_col.get();

    if grid == 0 {
        mouse_find_win_outer(&mut grid, &mut row, &mut col);
    }

    if grid == g_st().pum_grid.handle.get() {
        let border_offset = if pum_border_width() == 2 { 1 } else { 0 };
        let item = row - border_offset;
        PUM.with(|s| {
            s.selected
                .set(if item >= 0 && item < PUM.with(|s| s.height.get()) {
                    item
                } else {
                    -1
                })
        });
        return;
    }

    if grid != PUM.with(|s| s.anchor_grid.get())
        || col < PUM.with(|s| s.left_col.get()) - PUM.with(|s| s.win_col_offset.get())
        || col >= PUM.with(|s| s.right_col.get()) - PUM.with(|s| s.win_col_offset.get())
    {
        PUM.with(|s| s.selected.set(-1));
        return;
    }

    let idx = row - (PUM.with(|s| s.row.get()) - PUM.with(|s| s.win_row_offset.get()));

    if idx < 0 || idx >= PUM.with(|s| s.height.get()) {
        PUM.with(|s| s.selected.set(-1));
    } else if pum_item(idx)
        .pum_text
        .as_ref()
        .is_some_and(|s| !s.to_bytes().is_empty())
    {
        PUM.with(|s| s.selected.set(idx));
    }
}

/// Execute the currently selected popup menu item (`pum_execute_menu`).
fn pum_execute_menu(menu: &mut Vimmenu, path_name: Option<&CStr>, mode: i32) {
    if let Some(path_name) = path_name {
        if let Some(fresh) = menu_find(path_name) {
            *menu = fresh;
        }
    } else if let Some(fresh) = g_st()
        .root_menu
        .borrow()
        .iter()
        .find(|m| m.name == menu.name)
        .cloned()
    {
        *menu = fresh;
    }
    let mut idx: i32 = 0;
    for mp in &mut menu.children {
        if (mp.modes & mp.enabled & mode) != 0 {
            let cur_idx = idx;
            idx += 1;
            if cur_idx == PUM.with(|s| s.selected.get()) {
                let ea: Exarg = Exarg::new();
                execute_menu(&ea, mp, -1);
                break;
            }
        }
    }
}

/// Open the terminal version of the popup menu (`pum_show_popupmenu`).
pub fn pum_show_popupmenu(menu: &mut Vimmenu, path_name: Option<&CStr>) {
    pum_undisplay(true);
    PUM.with(|s| s.size.set(0));
    let mode = get_menu_mode_flag();

    for mp in &menu.children {
        if menu_is_separator(mp.dname.as_bytes()) || (mp.modes & mp.enabled & mode) != 0 {
            PUM.with(|s| s.size.set(s.size.get() + (1)));
        }
    }

    if PUM.with(|s| s.size.get()) <= 0 {
        emsg(gettext(E_MENU_ONLY_EXISTS_IN_ANOTHER_MODE));
        return;
    }

    let mut idx: usize = 0;
    let mut array_vec: Vec<Pumitem> = vec![Pumitem::new(); PUM.with(|s| s.size.get()) as usize];

    for mp in &menu.children {
        let mut s: Option<&CStr> = None;
        if menu_is_separator(mp.dname.as_bytes()) {
            s = Some(c"");
        } else if (mp.modes & mp.enabled & mode) != 0 {
            s = Some(mp.dname.as_c_str());
        }
        if let Some(s) = s {
            array_vec[idx].pum_text = Some(s.to_owned());
            idx += 1;
        }
    }

    let array_rc: std::rc::Rc<[Pumitem]> = array_vec.into();
    PUM_ITEMS.with(|c| *c.borrow_mut() = Some(array_rc.clone()));
    pum_compute_size();
    PUM.with(|s| s.scrollbar.set(0));
    PUM.with(|s| s.height.set(PUM.with(|s| s.size.get())));
    PUM.with(|s| s.rl.set(curwin_mut().w_onebuf_opt.wo_rl.get()));
    pum_position_at_mouse(20);

    PUM.with(|s| s.selected.set(-1));
    PUM.with(|s| s.first.set(0));
    if !opt_vars().p_mousemev.get() {
        ui_call_option_set(NvimString::from_cstr(c"mousemoveevent"), boolean_obj(true));
    }

    loop {
        PUM.with(|s| s.is_visible.set(true));
        PUM.with(|s| s.is_drawn.set(true));
        g_st().pum_grid.zindex.set(K_ZINDEX_CMDLINE_POPUP_MENU);
        pum_redraw();
        setcursor_mayforce(&curwin_mut(), true);

        let c = vgetc();

        if c == ESC || c == CTRL_C_CONST || !pum_has_items() {
            break;
        } else if c == CAR || c == NL {
            pum_execute_menu(menu, path_name, mode);
            break;
        } else if c == b'k' as i32 || c == K_UP || c == K_MOUSEUP {
            while PUM.with(|s| s.selected.get()) > 0 {
                PUM.with(|s| s.selected.set(s.selected.get() - (1)));
                if array_rc[PUM.with(|s| s.selected.get()) as usize]
                    .pum_text
                    .as_ref()
                    .is_some_and(|s| !s.to_bytes().is_empty())
                {
                    break;
                }
            }
        } else if c == b'j' as i32 || c == K_DOWN || c == K_MOUSEDOWN {
            while PUM.with(|s| s.selected.get()) < PUM.with(|s| s.size.get()) - 1 {
                PUM.with(|s| s.selected.set(s.selected.get() + (1)));
                if array_rc[PUM.with(|s| s.selected.get()) as usize]
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
            if PUM.with(|s| s.selected.get()) >= 0 {
                pum_execute_menu(menu, path_name, mode);
                break;
            }
            if c == K_LEFTMOUSE || c == K_LEFTMOUSE_NM {
                break;
            }
        }
    }

    pum_undisplay(true);
    if !opt_vars().p_mousemev.get() {
        ui_call_option_set(NvimString::from_cstr(c"mousemoveevent"), boolean_obj(false));
    }
}

pub fn pum_make_popup(path_name: &std::ffi::CStr, use_mouse_pos: bool) {
    if !use_mouse_pos {
        g_st()
            .mouse_row
            .set(curwin_mut().w_grid.row_offset.get() + curwin_mut().w_wrow.get());
        g_st().mouse_col.set(
            curwin_mut().w_grid.col_offset.get()
                + if curwin_mut().w_onebuf_opt.wo_rl.get() {
                    curwin_mut().w_view_width.get() - curwin_mut().w_wcol.get() - 1
                } else {
                    curwin_mut().w_wcol.get()
                },
        );
        if ui_has(K_UIMULTIGRID) {
            g_st()
                .mouse_grid
                .set(curwin_mut().w_grid.target.get().borrow().handle.get());
        } else if curwin_mut().w_grid.target.get() != crate::grid::GridTarget::Default {
            g_st().mouse_grid.set(0);
            g_st()
                .mouse_row
                .set(g_st().mouse_row.get() + (curwin_mut().w_winrow.get()));
            g_st()
                .mouse_col
                .set(g_st().mouse_col.get() + (curwin_mut().w_wincol.get()));
        }
    }

    if let Some(mut menu) = menu_find(path_name) {
        pum_show_popupmenu(&mut menu, Some(path_name));
    }
}

pub fn pum_pos_ui_flush(force: bool) {
    if force
        || (ui_has(K_UIMULTIGRID)
            && PUM.with(|s| s.is_drawn.get())
            && !PUM.with(|s| s.external.get())
            && g_st().pum_grid.handle.get() != 0
            && g_st().pum_grid.pending_comp_index_update.get())
    {
        let anchor = if PUM.with(|s| s.above.get()) {
            c"SW"
        } else {
            c"NW"
        };
        let row_off = if PUM.with(|s| s.above.get()) {
            -PUM.with(|s| s.height.get())
        } else {
            0
        };
        ui_call_win_float_pos(
            i64::from(g_st().pum_grid.handle.get()),
            -1,
            cstr_as_string(Some(anchor)),
            i64::from(PUM.with(|s| s.anchor_grid.get())),
            f64::from(PUM.with(|s| s.row.get()) - row_off - PUM.with(|s| s.win_row_offset.get())),
            f64::from(PUM.with(|s| s.left_col.get()) - PUM.with(|s| s.win_col_offset.get())),
            false,
            i64::from(g_st().pum_grid.zindex.get()),
            g_st().pum_grid.comp_index.get() as i64,
            i64::from(g_st().pum_grid.comp_row.get()),
            i64::from(g_st().pum_grid.comp_col.get()),
        );
        g_st().pum_grid.pending_comp_index_update.set(false);
    }
}
