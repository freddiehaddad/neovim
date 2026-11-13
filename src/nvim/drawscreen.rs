//! Code for updating all the windows on the screen (`src/nvim/drawscreen.c`).
#![forbid(unsafe_code)]

use crate::autocmd::{EVENT_VIMRESIZED, apply_autocmds};
use crate::buffer::{
    FileBuf, K_UIMESSAGES, MAXLNUM, STL_IN_ICON, STL_IN_TITLE, VALID_TOPLINE, buf_meta_total,
    maketitle,
};
use crate::change::{changed_lines_invalidate_win, changed_topline_skipcol};
use crate::charset::{vim_isprintc, vim_strsize};
use crate::cmdexpand::{K_UICMDLINE, cmdline_pum_display};
use crate::context::ctx_saved_curwin;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::decoration::{
    DecorVirtText, DecorVirtTextData, K_VPOS_WIN_COL, NS, SCL_NUM, VirtTextChunk,
    buf_signcols_count_range, decor_conceal_line, decor_range_add_virt, decor_redraw_reset,
    decor_virt_lines, win_lines_concealed,
};
use crate::decoration_provider::{
    decor_providers_invoke_buf, decor_providers_invoke_conceal, decor_providers_invoke_end,
    decor_providers_invoke_win, decor_providers_start,
};
use crate::diff::diff_redraw;
use crate::digraph::{HLF_CM, get_keymap_str};
use crate::drawline::{HLF_AT, HLF_COUNT, HLF_FC, K_CPO_NUMCOL, VALID_WCOL, win_line};
use crate::eval::typval::{K_CALLBACK_NONE_CONST, Varnumber};
use crate::eval::vars::{VV_ECHOSPACE, set_vim_var_nr};
use crate::ex_getln::{
    Hlf, cmdline_screen_cleared, compute_cmdrow, get_cmdline_info, redrawcmdline,
};
use crate::fold::{fold_info, foldmethod_is_syntax, has_any_folding, has_folding};
use crate::globals::*;
use crate::grid::{
    Schar, Wline, grid_adjust, grid_alloc, grid_allocated, grid_clear, grid_clear_line,
    grid_del_lines, grid_draw_border, grid_ins_lines, grid_invalidate, grid_line_clear_end,
    grid_line_fill, grid_line_flush, grid_line_getchar, grid_line_mirror, grid_line_put_schar,
    grid_line_start, schar_cache_clear_if_full, schar_from_ascii, win_grid_alloc,
};
use crate::highlight::{
    hl_combine_attr, update_window_hl, win_bg_attr, win_check_ns_hl, win_hl_attr,
};
use crate::highlight_group::highlight_changed;
use crate::input::char_avail;
use crate::insexpand::ins_compl_show_pum;
use crate::marktree::{DECOR_PRIORITY_BASE, K_MTMETA_SIGN_TEXT};
use crate::r#match::{init_search_hl, prepare_search_hl};
use crate::memline::ml_get_buf_len;
use crate::message::{
    msg_check_for_delay, msg_clr_cmdline, msg_clr_eos, msg_ext_flush_showmode, msg_ext_ui_flush,
    msg_grid_set_pos, msg_grid_validate, msg_puts_hl, msg_reset_scroll, msg_scrollsize,
    msg_use_grid, repeat_message,
};
use crate::r#move::{
    changed_line_abv_curs, changed_line_abv_curs_win, changed_window_setting, curs_columns,
    invalidate_botline_win, plines_correct_topline, set_empty_rows, update_curswant,
    update_topline, validate_cursor, validate_virtcol, win_col_off, win_col_off2,
};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::normal::{clear_showcmd, do_check_scrollbind};
use crate::option::{OptInt, get_ve_flags, shortmess};
use crate::option_vars::*;
use crate::plines::{
    getvcols, getvvcol, has_extconceal, linetabsize_disp_eol, plines_m_win, plines_win,
    win_get_fill, win_may_fill,
};
use crate::popupmenu::{pum_check_clear, pum_drawn, pum_invalidate, pum_redraw};
use crate::profile::profile_setlimit;
use crate::regexp::{CTRL_V_CONST, MAXCOL, re_multiline};
use crate::search::last_pat_prog;
use crate::spell::spell_check_window;
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_INSERT, MODE_LANGMAP,
    MODE_NORMAL, MODE_SETWSIZE, MODE_TERMINAL, MODE_VISUAL, REPLACE_FLAG, VREPLACE_FLAG,
    get_real_state,
};
use crate::statusline::{
    Foldinfo, draw_tabline, redraw_ruler, stl_alloc_click_defs, stl_clear_click_defs,
    win_redr_status, win_redr_winbar,
};
use crate::strings::gettext;
use crate::syntax::{
    syn_set_timeout, syn_stack_apply_changes, syntax_check_changed, syntax_end_parsing,
    syntax_present,
};
use crate::terminal::{terminal_check_size, terminal_suspended};
use crate::ui::{
    ui_call_grid_clear, ui_call_grid_resize, ui_call_msg_clear, ui_call_win_extmark, ui_flush,
    ui_grid_cursor_goto, ui_has,
};
use crate::ui_compositor::ui_comp_set_screen_valid;
use crate::version::{intro_message, may_show_intro};
use crate::window::{
    Frame, MIN_COLUMNS, WinInfoInfo, frame2win, global_stl_height, last_stl_height,
    min_cols_for_all_tabpages, min_rows, min_rows_for_all_tabpages, win_fdccol_count,
    win_new_screensize, win_ui_flush,
};
use std::ffi::CStr;
use std::ptr;

use crate::api::ui::NO_SCREEN;
use crate::api::vim::{HLF_N, HLF_SC};
use crate::api::win_config::{FR_COL, K_UIMULTIGRID};
use crate::extmark::K_FALSE;

pub const UPD_VALID: i32 = 10;
pub const UPD_INVERTED: i32 = 20;
pub const UPD_INVERTED_ALL: i32 = 25;
pub const UPD_REDRAW_TOP: i32 = 30;
pub const UPD_SOME_VALID: i32 = 35;
pub const UPD_NOT_VALID: i32 = 40;
pub const UPD_CLEAR: i32 = 50;

pub(crate) const DEFAULT_GRID_HANDLE: ObjHandle = 1;
const COL_RULER: i32 = 18;
pub(crate) const SHOWCMD_COLS: i32 = 10;
pub(crate) const SIGN_WIDTH: i32 = 2;

pub(crate) const FR_LEAF: u8 = 0;
pub(crate) const FR_ROW: u8 = 1;

pub(crate) const VALID_BOTLINE: i32 = 0x20;

pub(crate) const GETVCOL_END_EXCL_LBR: i32 = 1;

const HLF_EOB: Hlf = 2;
pub(crate) const HLF_C: i32 = 21;
pub(crate) const HLF_MSG: usize = 63;

pub(crate) const K_SHM_COMPLETIONMENU: i32 = b'c' as i32;
const K_SHM_RECORDING: i32 = b'q' as i32;
pub(crate) const K_OPT_VE_FLAG_BLOCK: u32 = 0x05;
const K_OPT_DY_FLAG_LASTLINE: u32 = 0x01;
const K_OPT_DY_FLAG_TRUNCATE: u32 = 0x02;

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

#[derive(Clone, Copy)]
pub struct WinExtmark {
    pub ns_id: NS,
    pub mark_id: u64,
    pub win_row: i32,
    pub win_col: i32,
}

#[derive(Clone, Copy, Default)]
pub struct Spellvars {
    pub spv_has_spell: bool,
    pub spv_unchanged: bool,
    pub spv_checked_col: i32,
    pub spv_checked_lnum: Linenr,
    pub spv_cap_col: i32,
    pub spv_capcol_lnum: Linenr,
}

struct DrawscreenState {
    redraw_popupmenu: std::cell::Cell<bool>,
    msg_grid_invalid: std::cell::Cell<bool>,
    resizing_autocmd: std::cell::Cell<bool>,
    conceal_cursor_used: std::cell::Cell<bool>,
    conceal_visual: std::cell::Cell<[Linenr; 2]>,
    default_grid_resizing: std::cell::Cell<bool>,
    still_may_intro: std::cell::Cell<bool>,
    win_update_recursive: std::cell::Cell<bool>,
    suspended_virt_text: std::cell::Cell<crate::decoration::DecorVirtId>,
}

impl DrawscreenState {
    const fn new() -> Self {
        Self {
            redraw_popupmenu: std::cell::Cell::new(false),
            msg_grid_invalid: std::cell::Cell::new(false),
            resizing_autocmd: std::cell::Cell::new(false),
            conceal_cursor_used: std::cell::Cell::new(false),
            conceal_visual: std::cell::Cell::new([0, 0]),
            default_grid_resizing: std::cell::Cell::new(false),
            still_may_intro: std::cell::Cell::new(true),
            win_update_recursive: std::cell::Cell::new(false),
            suspended_virt_text: std::cell::Cell::new(crate::decoration::DecorVirtId::NULL),
        }
    }
}

std::thread_local! {
    static DS_STATE: DrawscreenState = const { DrawscreenState::new() };
}

#[inline]
fn ds_st<R>(f: impl FnOnce(&DrawscreenState) -> R) -> R {
    DS_STATE.with(f)
}

#[inline]
pub fn w_endcol(wp: &WinInfoInfo) -> i32 {
    wp.w_wincol.get() + wp.w_width.get()
}

#[inline]
pub fn w_endrow(wp: &WinInfoInfo) -> i32 {
    wp.w_winrow.get() + wp.w_height.get()
}

#[inline]
fn w_lines_at(wp: &WinInfoInfo, idx: usize) -> std::cell::RefMut<'_, Wline> {
    std::cell::RefMut::map(wp.w_lines.borrow_mut(), |v| &mut v[idx])
}

/// Redraws the cursor line of `wp` after 'concealcursor' revealed or concealed it again.
pub fn conceal_redraw_cursor_line(wp: &WinInfoInfo) {
    let cursor_lnum = wp.w_cursor.get().lnum;
    conceal_reveal_changed(wp, cursor_lnum, cursor_lnum);
    if decor_conceal_line(wp, cursor_lnum - 1, true) {
        changed_window_setting(wp);
    }
}

pub fn conceal_check_cursor_line() {
    conceal_check_visual();

    let should_conceal = conceal_cursor_line(&curwin());
    if curwin().w_onebuf_opt.wo_cole.get() <= 0
        || ds_st(|st| st.conceal_cursor_used.get()) == should_conceal
    {
        return;
    }

    conceal_redraw_cursor_line(&crate::globals::curwin());
    curs_columns(&curwin(), true);
}

/// Redraws the lines that entered or left the Visual area since the last check: 'concealcursor'
/// reveals the area (see conceal_cursor_reveals_line()).
pub fn conceal_check_visual() {
    let cur = if g_st().visual_global.borrow().active {
        [
            g_st()
                .visual_global
                .borrow()
                .start
                .lnum
                .min(curwin().w_cursor.get().lnum),
            g_st()
                .visual_global
                .borrow()
                .start
                .lnum
                .max(curwin().w_cursor.get().lnum),
        ]
    } else {
        [0, 0]
    };
    let old = ds_st(|st| st.conceal_visual.replace(cur));

    if old[0] == 0 && cur[0] == 0 {
        return;
    }
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            if g_st().curbuf.get() != wp.w_buffer.get() {
                continue;
            }
            if old[0] == 0 {
                conceal_reveal_changed(&wp, cur[0], cur[1]);
            } else if cur[0] == 0 {
                conceal_reveal_changed(&wp, old[0], old[1]);
            } else {
                for i in 0..2 {
                    if cur[i] != old[i] {
                        conceal_reveal_changed(&wp, cur[i].min(old[i]), cur[i].max(old[i]));
                    }
                }
            }
        }
    }
}

pub fn default_grid_alloc() -> bool {
    if ds_st(|st| st.default_grid_resizing.replace(true)) {
        return false;
    }

    if (grid_allocated(&g_st().default_grid)
        && g_st().rows_global.get() == g_st().default_grid.rows.get()
        && g_st().columns_global.get() == g_st().default_grid.cols.get())
        || g_st().rows_global.get() == 0
        || g_st().columns_global.get() == 0
    {
        ds_st(|st| st.default_grid_resizing.set(false));
        return false;
    }

    grid_alloc(
        &g_st().default_grid,
        g_st().rows_global.get(),
        g_st().columns_global.get(),
        true,
        true,
    );

    stl_clear_click_defs(&mut g_st().tab_page_click_defs.borrow_mut());
    stl_alloc_click_defs(
        &mut g_st().tab_page_click_defs.borrow_mut(),
        g_st().columns_global.get(),
    );

    g_st()
        .default_grid
        .comp_height
        .set(g_st().rows_global.get());
    g_st()
        .default_grid
        .comp_width
        .set(g_st().columns_global.get());
    g_st().default_grid.handle.set(DEFAULT_GRID_HANDLE);

    ds_st(|st| st.default_grid_resizing.set(false));
    true
}

pub fn screenclear() {
    msg_check_for_delay(false);

    if starting_val() == NO_SCREEN || !grid_allocated(&g_st().default_grid) {
        return;
    }

    for i in 0..g_st().default_grid.rows.get() {
        let off = g_st().default_grid.line_offset.borrow()[i as usize];
        grid_clear_line(
            &g_st().default_grid,
            off,
            g_st().default_grid.cols.get(),
            true,
        );
    }

    ui_call_grid_clear(1);
    ui_comp_set_screen_valid(true);

    g_st().ns_hl_fast.set(-1);

    g_st().clear_cmdline.set(false);
    g_st().mode_displayed.set(false);

    redraw_all_later(UPD_NOT_VALID);
    g_st().cmdline_was_last_drawn.set(false);
    g_st().redraw_cmdline.set(true);
    g_st().redraw_tabline.set(true);
    ds_st(|st| st.redraw_popupmenu.set(true));
    pum_invalidate();

    for wp in for_all_windows_ptr() {
        if wp.w_floating.get() {
            wp.w_redr_type.set(UPD_CLEAR);
        }
    }

    if g_st().must_redraw.get() == UPD_CLEAR {
        g_st().must_redraw.set(UPD_NOT_VALID);
    }
    compute_cmdrow();
    g_st().msg_row.set(g_st().cmdline_row.get());
    g_st().msg_col.set(0);
    msg_reset_scroll();
    g_st().msg_didany.set(false);
    g_st().msg_didout.set(false);
    if crate::highlight::hl_attr(HLF_MSG as i32) > 0
        && msg_use_grid()
        && grid_allocated(&g_st().msg_grid)
    {
        grid_invalidate(&g_st().msg_grid);
        msg_grid_validate();
        ds_st(|st| st.msg_grid_invalid.set(false));
        g_st().clear_cmdline.set(true);
    }
}

fn cmdline_number_prompt() -> bool {
    !ui_has(K_UIMESSAGES)
        && (g_st().state_global.get() & MODE_CMDLINE) != 0
        && get_cmdline_info().mouse_used.get().is_some()
}

pub fn screen_resize(mut width: i32, mut height: i32) {
    if g_st().updating_screen.get() || g_st().resizing_screen.get() || cmdline_number_prompt() {
        return;
    }

    if width < 0 || height < 0 {
        return;
    }

    if g_st().state_global.get() == MODE_HITRETURN || g_st().state_global.get() == MODE_SETWSIZE {
        g_st().state_global.set(MODE_SETWSIZE);
        return;
    }

    g_st().resizing_screen.set(true);

    g_st().rows_global.set(height);
    g_st().columns_global.set(width);
    check_screensize();
    if !ui_has(K_UIMESSAGES) {
        let max_p_ch = OptInt::from(g_st().rows_global.get() - min_rows(&curtab()) + 1);
        if opt_vars().p_ch.get() > 0 && opt_vars().p_ch.get() > max_p_ch {
            opt_vars().p_ch.set(max_p_ch.max(1));
            curtab().tp_ch_used.set(opt_vars().p_ch.get());
        }
        for tp in for_all_tabpages_ptr() {
            if g_st().curtab.get() != tp {
                let max_tp_ch = OptInt::from(g_st().rows_global.get() - min_rows(&tp) + 1);
                if tp.tp_ch_used.get() > 0 && tp.tp_ch_used.get() > max_tp_ch {
                    tp.tp_ch_used.set(max_tp_ch.max(1));
                }
            }
        }
    }
    height = g_st().rows_global.get();
    width = g_st().columns_global.get();
    opt_vars()
        .p_lines
        .set(OptInt::from(g_st().rows_global.get()));
    opt_vars()
        .p_columns
        .set(OptInt::from(g_st().columns_global.get()));

    ui_call_grid_resize(1, Integer::from(width), Integer::from(height));

    let mut retry_count = 0;
    ds_st(|st| st.resizing_autocmd.set(true));

    while default_grid_alloc() {
        ui_comp_set_screen_valid(false);
        if grid_allocated(&g_st().msg_grid) {
            ds_st(|st| st.msg_grid_invalid.set(true));
        }

        g_st()
            .redrawing_disabled
            .set(g_st().redrawing_disabled.get() + (1));

        win_new_screensize();
        comp_col();

        g_st()
            .redrawing_disabled
            .set(g_st().redrawing_disabled.get() - (1));

        retry_count += 1;
        if retry_count > 3 {
            break;
        }

        apply_autocmds(EVENT_VIMRESIZED, None, None, false, curbuf_opt().as_deref());
    }

    ds_st(|st| st.resizing_autocmd.set(false));
    redraw_all_later(UPD_CLEAR);

    if g_st().state_global.get() != MODE_ASKMORE && g_st().state_global.get() != MODE_EXTERNCMD {
        screenclear();
    }

    if starting_val() != NO_SCREEN {
        maketitle();

        changed_line_abv_curs();
        invalidate_botline_win(&curwin());

        if g_st().state_global.get() == MODE_ASKMORE
            || g_st().state_global.get() == MODE_EXTERNCMD
            || ((g_st().state_global.get() & MODE_CMDLINE) != 0 && get_cmdline_info().one_key.get())
        {
            if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
                update_screen();
            }
            if grid_allocated(&g_st().msg_grid) {
                msg_grid_validate();
            }
            ui_comp_set_screen_valid(true);
            repeat_message();
        } else {
            if curwin().w_onebuf_opt.wo_scb.get() {
                do_check_scrollbind(true);
            }
            if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
                ds_st(|st| st.redraw_popupmenu.set(false));
                update_screen();
                redrawcmdline();
                if pum_drawn() {
                    cmdline_pum_display(false);
                }
            } else {
                update_topline(&curwin());
                if pum_drawn() {
                    ds_st(|st| st.redraw_popupmenu.set(false));
                    ins_compl_show_pum();
                }
                update_screen();
                if redrawing() {
                    setcursor();
                }
            }
        }
        if g_st().state_global.get() != MODE_ASKMORE {
            ui_flush();
        }
    }
    g_st().resizing_screen.set(false);
}

pub fn check_screensize() {
    g_st().rows_global.set(
        g_st()
            .rows_global
            .get()
            .max(min_rows_for_all_tabpages())
            .min(1000),
    );
    g_st().columns_global.set(
        g_st()
            .columns_global
            .get()
            .max(MIN_COLUMNS)
            .max(min_cols_for_all_tabpages())
            .min(10000),
    );
}

pub fn redrawing() -> bool {
    g_st().redrawing_disabled.get() == 0
        && !(opt_vars().p_lz.get()
            && char_avail()
            && !g_st().key_typed.get()
            && !g_st().do_redraw.get())
}

pub fn update_screen() -> bool {
    if ds_st(|st| st.still_may_intro.get()) && !may_show_intro() {
        redraw_later(Some(&crate::globals::firstwin()), UPD_NOT_VALID);
        ds_st(|st| st.still_may_intro.set(false));
    }

    let is_stl_global = global_stl_height() > 0;

    if ds_st(|st| st.resizing_autocmd.get()) || !grid_allocated(&g_st().default_grid) {
        return false;
    }

    if g_st().need_diff_redraw.get() {
        diff_redraw(true);
    }

    if !redrawing()
        || g_st().updating_screen.get()
        || cmdline_number_prompt()
        || g_st().state_global.get() == MODE_HITRETURN
        || g_st().state_global.get() == MODE_SETWSIZE
    {
        return false;
    }

    let save_curwin = ctx_saved_curwin();
    let restore_curwin = if save_curwin.is_some() {
        curwin_ptr()
    } else {
        crate::globals::WinPtr::NULL
    };
    if let Some(save_curwin) = save_curwin {
        set_curwin(save_curwin);
        g_st().curbuf.set(curwin().w_buffer.get());
    }

    let mut type_ = g_st().must_redraw.get();
    g_st().must_redraw.set(0);

    g_st().updating_screen.set(true);
    inc_display_tick();

    if schar_cache_clear_if_full() {
        type_ = type_.max(UPD_CLEAR);
    }

    if g_st().msg_did_scroll.get() {
        g_st().msg_did_scroll.set(false);
        g_st().msg_scrolled_at_flush.set(0);
    }

    if type_ >= UPD_CLEAR || !g_st().default_grid.valid.get() {
        ui_comp_set_screen_valid(false);
    }

    if g_st().msg_scrolled.get() != 0 || ds_st(|st| st.msg_grid_invalid.get()) {
        g_st().clear_cmdline.set(true);
        let valid = (g_st().rows_global.get() - msg_scrollsize()).max(0);
        if grid_allocated(&g_st().msg_grid) {
            let limit = msg_scrollsize().min(g_st().msg_grid.rows.get());
            for i in 0..limit {
                let off = g_st().msg_grid.line_offset.borrow()[i as usize];
                grid_clear_line(
                    &g_st().msg_grid,
                    off,
                    g_st().msg_grid.cols.get(),
                    OptInt::from(i) < opt_vars().p_ch.get(),
                );
            }
        }
        g_st().msg_grid.throttled.set(false);
        let mut was_invalidated = false;

        if type_ == UPD_NOT_VALID && !ui_has(K_UIMULTIGRID) && g_st().msg_scrolled.get() != 0 {
            was_invalidated = ui_comp_set_screen_valid(false);
            let mut i = valid;
            while i < g_st().rows_global.get() - (opt_vars().p_ch.get() as i32) {
                let off = g_st().default_grid.line_offset.borrow()[i as usize];
                grid_clear_line(
                    &g_st().default_grid,
                    off,
                    g_st().columns_global.get(),
                    false,
                );
                i += 1;
            }
            for wp in for_all_windows_ptr() {
                if !wp.w_floating.get() {
                    if w_endrow(&wp) > valid {
                        wp.w_redr_type.set(wp.w_redr_type.get().max(UPD_NOT_VALID));
                    }
                    if !is_stl_global && w_endrow(&wp) + wp.w_status_height.get() > valid {
                        wp.w_redr_status.set(true);
                    }
                }
            }
            if is_stl_global
                && g_st().rows_global.get() - (opt_vars().p_ch.get() as i32) - 1 > valid
            {
                curwin().w_redr_status.set(true);
            }
        }
        msg_grid_set_pos(
            g_st().rows_global.get() - (opt_vars().p_ch.get() as i32),
            false,
        );
        ds_st(|st| st.msg_grid_invalid.set(false));
        if was_invalidated {
            ui_comp_set_screen_valid(true);
        }
        g_st().msg_scrolled.set(0);
        g_st().msg_scrolled_at_flush.set(0);
        g_st().msg_grid_scroll_discount.set(0);
        g_st().need_wait_return.set(false);
    }

    win_ui_flush(true);
    compute_cmdrow();

    let mut hl_changed = false;
    if g_st().need_highlight_changed.get() {
        highlight_changed();
        hl_changed = true;
    }

    if type_ == UPD_CLEAR {
        screenclear();
        cmdline_screen_cleared();
        if ui_has(K_UIMESSAGES) {
            ui_call_msg_clear();
        }
        type_ = UPD_NOT_VALID;
        g_st().must_redraw.set(0);
    } else if !g_st().default_grid.valid.get() {
        grid_invalidate(&g_st().default_grid);
        g_st().default_grid.valid.set(true);
    }

    if type_ == UPD_NOT_VALID && g_st().clear_cmdline.get() && !ui_has(K_UIMESSAGES) {
        grid_clear(
            &g_st().default_gridview,
            g_st().rows_global.get() - (opt_vars().p_ch.get() as i32),
            g_st().rows_global.get(),
            0,
            g_st().columns_global.get(),
            0,
        );
    }

    ui_comp_set_screen_valid(true);

    decor_providers_start();

    if win_check_ns_hl(None) {
        g_st().redraw_cmdline.set(true);
        g_st().redraw_tabline.set(true);
    }

    if g_st().clear_cmdline.get() {
        msg_check_for_delay(false);
    }

    if curwin().w_redr_type.get() < UPD_NOT_VALID {
        let curwin_nrw = if curwin().w_onebuf_opt.wo_nu.get()
            || curwin().w_onebuf_opt.wo_rnu.get()
            || !curwin().w_onebuf_opt.wo_stc.is_empty()
        {
            number_width(&curwin())
        } else {
            0
        };
        if curwin().w_nrwidth.get() != curwin_nrw {
            curwin().w_redr_type.set(UPD_NOT_VALID);
        }
    }

    if curwin().w_redr_type.get() == UPD_INVERTED {
        update_curswant();
    }

    if g_st().redraw_tabline.get() || type_ >= UPD_NOT_VALID {
        update_window_hl(&curwin(), type_ >= UPD_NOT_VALID);
        for tp in for_all_tabpages_ptr() {
            if g_st().curtab.get() != tp {
                update_window_hl(&tp.curwin(), type_ >= UPD_NOT_VALID);
            }
        }
        draw_tabline();
    }

    for wp in for_all_windows_ptr() {
        if !(wp.w_config.borrow().hide && !wp.w_grid_alloc.chars.borrow().is_empty()) {
            update_window_hl(&wp, type_ >= UPD_NOT_VALID || hl_changed);

            let buf = wp.buf();
            if buf.b_mod_set.get() {
                if buf.b_mod_tick_syn.get() < display_tick_val() && syntax_present(&wp) {
                    syn_stack_apply_changes(&buf);
                    buf.b_mod_tick_syn.set(display_tick_val());
                }
                if buf.b_mod_tick_decor.get() < display_tick_val() {
                    decor_providers_invoke_buf(&buf);
                    buf.b_mod_tick_decor.set(display_tick_val());
                }
            }
        }
    }

    let mut did_one = false;
    g_st().screen_search_hl.borrow_mut().rm.free_prog();

    for wp in for_all_windows_ptr() {
        if wp.w_config.borrow().hide && !wp.w_grid_alloc.chars.borrow().is_empty() {
            if wp == curwin_ptr() && global_stl_height() > 0 {
                win_redr_status(&wp);
            }
            wp.w_redr_type.set(0);
            continue;
        }

        if wp.w_redr_type.get() == UPD_CLEAR
            && wp.w_floating.get()
            && !wp.w_grid_alloc.chars.borrow().is_empty()
        {
            grid_invalidate(&wp.w_grid_alloc);
            wp.w_redr_type.set(UPD_NOT_VALID);
        }

        win_check_ns_hl(Some(&wp));
        win_grid_alloc(&wp);

        if wp.w_redr_border.get() || wp.w_redr_type.get() >= UPD_NOT_VALID {
            let adj = wp.w_border_adj.each_ref().map(std::cell::Cell::get);
            grid_draw_border(
                &wp.w_grid_alloc,
                &mut wp.w_config.borrow_mut(),
                Some(&adj),
                wp.w_onebuf_opt.wo_winbl.get() as i32,
                wp.w_ns_hl_attr.get().as_deref(),
                wp.w_hl_attr_normal.get(),
            );
        }

        if wp.w_redr_type.get() != 0 {
            if !did_one {
                did_one = true;
                start_search_hl();
            }
            win_update(&wp);
        }

        if wp.w_redr_status.get() {
            win_redr_winbar(&wp);
            win_redr_status(&wp);
        }
    }

    if did_one {
        for wp in for_all_windows_ptr() {
            draw_sep_connectors_win(&wp);
        }
    }

    end_search_hl();

    if pum_drawn() && g_st().must_redraw_pum.get() {
        win_check_ns_hl(Some(&crate::globals::curwin()));
        pum_redraw();
    } else if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
        pum_check_clear();
    }

    win_check_ns_hl(None);

    for wp in for_all_windows_ptr() {
        wp.buf().b_mod_set.set(false);
    }

    g_st().updating_screen.set(false);

    if g_st().need_maketitle.get() {
        maketitle();
    }

    if g_st().clear_cmdline.get() || g_st().redraw_cmdline.get() || g_st().redraw_mode.get() {
        showmode();
    }

    if ds_st(|st| st.still_may_intro.get()) {
        intro_message(false);
    }
    repeat_message();

    decor_providers_invoke_end();

    if !ui_has(K_UICMDLINE) {
        g_st().cmdline_was_last_drawn.set(false);
    }

    if !restore_curwin.is_null() {
        set_curwin(restore_curwin);
        g_st().curbuf.set(curwin().w_buffer.get());
    }

    true
}

pub fn start_search_hl() {
    if !opt_vars().p_hls.get() || g_st().search_global.no_hlsearch.get() {
        return;
    }

    end_search_hl();
    last_pat_prog(&mut g_st().screen_search_hl.borrow_mut().rm);
    g_st().screen_search_hl.borrow_mut().tm = profile_setlimit(opt_vars().p_rdt.get());
}

pub fn end_search_hl() {
    g_st().screen_search_hl.borrow_mut().rm.free_prog();
}

pub fn setcursor() {
    setcursor_mayforce(&curwin(), false);
}

pub fn setcursor_mayforce(wp: &WinInfoInfo, force: bool) {
    if force || redrawing() {
        validate_cursor(wp);

        let mut row = wp.w_wrow.get();
        let mut col = wp.w_wcol.get();
        if wp.w_onebuf_opt.wo_rl.get() {
            let cur_slice = &crate::memline::ml_get_buf_slice(&wp.buf(), wp.w_cursor.get().lnum)
                [wp.w_cursor.get().col as usize..];
            let wide = crate::mbyte::utf_slice2cells(cur_slice) == 2
                && vim_isprintc(crate::mbyte::utf_slice2char(cur_slice));
            col = wp.w_view_width.get() - wp.w_wcol.get() - if wide { 2 } else { 1 };
        }

        if let Some(grid) = crate::grid::grid_adjust_opt(&wp.w_grid, &mut row, &mut col) {
            ui_grid_cursor_goto(grid.handle.get(), row, col);
        }
    }
}

pub fn redraw_custom_title_later() -> bool {
    if (opt_vars().p_icon.get() && (g_st().stl_syntax.get() & STL_IN_ICON) != 0)
        || (opt_vars().p_title.get() && (g_st().stl_syntax.get() & STL_IN_TITLE) != 0)
    {
        g_st().need_maketitle.set(true);
        return true;
    }
    false
}

pub fn show_cursor_info_later(force: bool) {
    let state = get_real_state();
    let empty_line = (g_st().state_global.get() & MODE_INSERT) == 0
        && crate::memline::ml_get_buf_slice(&curwin().buf(), curwin().w_cursor.get().lnum)
            .is_empty();

    validate_virtcol(&curwin());
    if force
        || curwin().w_cursor.get().lnum != curwin().w_stl_cursor.get().lnum
        || curwin().w_cursor.get().col != curwin().w_stl_cursor.get().col
        || curwin().w_virtcol.get() != curwin().w_stl_virtcol.get()
        || curwin().w_cursor.get().coladd != curwin().w_stl_cursor.get().coladd
        || curwin().w_topline.get() != curwin().w_stl_topline.get()
        || curwin().buf().b_ml.ml_line_count.get() != curwin().w_stl_line_count.get()
        || curwin().w_topfill.get() != curwin().w_stl_topfill.get()
        || empty_line != curwin().w_stl_empty.get()
        || g_st().reg_recording.get() != curwin().w_stl_recording.get()
        || state != curwin().w_stl_state.get()
        || (g_st().visual_global.borrow().active
            && (g_st().visual_global.borrow().mode != curwin().w_stl_visual_mode.get()
                || g_st().visual_global.borrow().start.lnum
                    != curwin().w_stl_visual_pos.get().lnum
                || g_st().visual_global.borrow().start.col != curwin().w_stl_visual_pos.get().col
                || g_st().visual_global.borrow().start.coladd
                    != curwin().w_stl_visual_pos.get().coladd))
    {
        if curwin().w_status_height.get() != 0 || global_stl_height() != 0 {
            curwin().w_redr_status.set(true);
        } else {
            g_st().redraw_cmdline.set(true);
        }

        if !opt_vars().p_wbr.is_empty() || !curwin().w_onebuf_opt.wo_wbr.is_empty() {
            curwin().w_redr_status.set(true);
        }

        redraw_custom_title_later();
    }

    curwin().w_stl_cursor.set(curwin().w_cursor.get());
    curwin().w_stl_virtcol.set(curwin().w_virtcol.get());
    curwin().w_stl_empty.set(empty_line);
    curwin().w_stl_topline.set(curwin().w_topline.get());
    curwin()
        .w_stl_line_count
        .set(curwin().buf().b_ml.ml_line_count.get());
    curwin().w_stl_topfill.set(curwin().w_topfill.get());
    curwin().w_stl_recording.set(g_st().reg_recording.get());
    curwin().w_stl_state.set(state);
    if g_st().visual_global.borrow().active {
        curwin()
            .w_stl_visual_mode
            .set(g_st().visual_global.borrow().mode);
        curwin()
            .w_stl_visual_pos
            .set(g_st().visual_global.borrow().start);
    }
}

pub fn skip_showmode() -> bool {
    if g_st().global_busy.get() != 0
        || g_st().msg_silent.get() != 0
        || !redrawing()
        || (char_avail() && !g_st().key_typed.get())
    {
        g_st().redraw_mode.set(true);
        return true;
    }
    false
}

pub fn showmode() -> i32 {
    let mut length: i32 = 0;

    msg_ext_ui_flush();
    msg_grid_validate();

    let do_mode = (opt_vars().p_smd.get() && g_st().msg_silent.get() == 0)
        && ((g_st().state_global.get() & MODE_TERMINAL) != 0
            || (g_st().state_global.get() & MODE_INSERT) != 0
            || g_st().restart_edit.get() != 0
            || g_st().visual_global.borrow().active);

    let can_show_mode = opt_vars().p_ch.get() != 0 || ui_has(K_UIMESSAGES);
    if (do_mode || g_st().reg_recording.get() != 0) && can_show_mode {
        if skip_showmode() {
            return 0;
        }

        let nwr_save = g_st().need_wait_return.get();
        msg_check_for_delay(false);

        let mut need_clear = g_st().clear_cmdline.get();
        if g_st().clear_cmdline.get() && g_st().cmdline_row.get() < g_st().rows_global.get() - 1 {
            msg_clr_cmdline();
        }

        msg_pos_mode();
        let hl_id = HLF_CM;

        g_st().msg_no_more.set(true);
        let save_lines_left = g_st().lines_left.get();
        g_st().lines_left.set(0);

        if do_mode {
            msg_puts_hl(c"--", hl_id, false);
            if let Some(submode) = g_st().edit_submode.get()
                && !shortmess(K_SHM_COMPLETIONMENU)
            {
                length = if ui_has(K_UIMESSAGES) {
                    i32::MAX
                } else {
                    (g_st().rows_global.get() - g_st().msg_row.get()) * g_st().columns_global.get()
                        - 3
                };
                if let Some(ref extra) = g_st().edit_submode_extra.get() {
                    length -= vim_strsize(extra);
                }
                if length > 0 {
                    if let Some(pre) = g_st().edit_submode_pre.get() {
                        length -= vim_strsize(pre);
                    }
                    if length - vim_strsize(submode) > 0 {
                        if let Some(pre) = g_st().edit_submode_pre.get() {
                            msg_puts_hl(pre, hl_id, false);
                        }
                        msg_puts_hl(submode, hl_id, false);
                    }
                    if let Some(extra) = g_st().edit_submode_extra.get().clone() {
                        msg_puts_hl(c" ", hl_id, false);
                        let sub_id = if g_st().edit_submode_highl.get() < HLF_COUNT {
                            g_st().edit_submode_highl.get()
                        } else {
                            hl_id
                        };
                        msg_puts_hl(&extra, sub_id, false);
                    }
                }
            } else {
                if (g_st().state_global.get() & MODE_TERMINAL) != 0 {
                    msg_puts_hl(gettext(c" TERMINAL"), hl_id, false);
                } else if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
                    msg_puts_hl(gettext(c" VREPLACE"), hl_id, false);
                } else if (g_st().state_global.get() & REPLACE_FLAG) != 0 {
                    msg_puts_hl(gettext(c" REPLACE"), hl_id, false);
                } else if (g_st().state_global.get() & MODE_INSERT) != 0 {
                    if opt_vars().p_ri.get() {
                        msg_puts_hl(gettext(c" REVERSE"), hl_id, false);
                    }
                    msg_puts_hl(gettext(c" INSERT"), hl_id, false);
                } else if g_st().restart_edit.get() == i32::from(b'I')
                    || g_st().restart_edit.get() == i32::from(b'i')
                    || g_st().restart_edit.get() == i32::from(b'a')
                    || g_st().restart_edit.get() == i32::from(b'A')
                {
                    if !curbuf().terminal.get().is_null() {
                        msg_puts_hl(gettext(c" (terminal)"), hl_id, false);
                    } else {
                        msg_puts_hl(gettext(c" (insert)"), hl_id, false);
                    }
                } else if g_st().restart_edit.get() == i32::from(b'R') {
                    msg_puts_hl(gettext(c" (replace)"), hl_id, false);
                } else if g_st().restart_edit.get() == i32::from(b'V') {
                    msg_puts_hl(gettext(c" (vreplace)"), hl_id, false);
                }
                if (g_st().state_global.get() & MODE_LANGMAP) != 0 {
                    if curwin().w_onebuf_opt.wo_arab.get() {
                        msg_puts_hl(gettext(c" Arabic"), hl_id, false);
                    } else {
                        let mut name_buff = [0u8; crate::os::fs::MAXPATHL];
                        if get_keymap_str(&curwin(), c" (%s)", &mut name_buff) > 0 {
                            msg_puts_hl(crate::memory::bytes_as_cstr(&name_buff), hl_id, false);
                        }
                    }
                }
                if (g_st().state_global.get() & MODE_INSERT) != 0 && opt_vars().p_paste.get() {
                    msg_puts_hl(gettext(c" (paste)"), hl_id, false);
                }

                if g_st().visual_global.borrow().active {
                    let code = (if g_st().visual_global.borrow().select {
                        4
                    } else {
                        0
                    }) + (if g_st().visual_global.borrow().mode == CTRL_V_CONST {
                        2
                    } else {
                        0
                    }) + (if g_st().visual_global.borrow().mode == i32::from(b'V') {
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
                    msg_puts_hl(gettext(p), hl_id, false);
                }
                msg_puts_hl(c" --", hl_id, false);
            }

            need_clear = true;
        }
        if g_st().reg_recording.get() != 0 && g_st().edit_submode.get().is_none() {
            recording_mode(hl_id);
            need_clear = true;
        }

        g_st().mode_displayed.set(true);
        if need_clear || g_st().clear_cmdline.get() || g_st().redraw_mode.get() {
            msg_clr_eos();
        }
        g_st().msg_didout.set(false);
        length = g_st().msg_col.get();
        g_st().msg_col.set(0);
        g_st().msg_no_more.set(false);
        g_st().lines_left.set(save_lines_left);
        g_st().need_wait_return.set(nwr_save);
    } else if g_st().clear_cmdline.get() && g_st().msg_silent.get() == 0 {
        msg_clr_cmdline();
    } else if g_st().redraw_mode.get() {
        msg_pos_mode();
        msg_clr_eos();
    }

    msg_ext_flush_showmode();

    if g_st().visual_global.borrow().active {
        clear_showcmd();
    }

    redraw_ruler();
    g_st().redraw_cmdline.set(false);
    g_st().redraw_mode.set(false);
    g_st().clear_cmdline.set(false);

    length
}

fn msg_pos_mode() {
    g_st().msg_col.set(0);
    g_st().msg_row.set(g_st().rows_global.get() - 1);
}

pub fn unshowmode(force: bool) {
    if !redrawing() || (!force && char_avail() && !g_st().key_typed.get()) {
        g_st().redraw_cmdline.set(true);
    } else {
        clearmode();
    }
}

pub fn clearmode() {
    let save_msg_row = g_st().msg_row.get();
    let save_msg_col = g_st().msg_col.get();

    msg_ext_ui_flush();
    msg_pos_mode();
    if g_st().reg_recording.get() != 0 {
        recording_mode(HLF_CM);
    }
    msg_clr_eos();
    msg_ext_flush_showmode();

    g_st().msg_col.set(save_msg_col);
    g_st().msg_row.set(save_msg_row);
}

fn recording_mode(hl_id: i32) {
    if shortmess(K_SHM_RECORDING) {
        return;
    }

    msg_puts_hl(gettext(c"recording"), hl_id, false);
    let s: [u8; 4] = [b' ', b'@', g_st().reg_recording.get() as u8, 0];
    msg_puts_hl(CStr::from_bytes_until_nul(&s).unwrap_or(c""), hl_id, false);
}

pub fn comp_col() {
    let last_has_status = last_stl_height(false) > 0;

    g_st().sc_col.set(0);
    g_st().ru_col.set(0);
    if opt_vars().p_ru.get() {
        g_st().ru_col.set(if g_st().ru_wid.get() != 0 {
            g_st().ru_wid.get()
        } else {
            COL_RULER
        });
        if !last_has_status {
            g_st().sc_col.set(g_st().ru_col.get());
        }
    }
    if opt_vars().p_sc.get() && opt_vars().p_sloc.first_byte() == b'l' {
        g_st().sc_col.set(g_st().sc_col.get() + (SHOWCMD_COLS));
        if !opt_vars().p_ru.get() || last_has_status {
            g_st().sc_col.set(g_st().sc_col.get() + (1));
        }
    }
    assert!(
        g_st().sc_col.get() >= 0 && i32::MIN + g_st().sc_col.get() <= g_st().columns_global.get()
    );
    g_st()
        .sc_col
        .set(g_st().columns_global.get() - g_st().sc_col.get());
    assert!(
        g_st().ru_col.get() >= 0 && i32::MIN + g_st().ru_col.get() <= g_st().columns_global.get()
    );
    g_st()
        .ru_col
        .set(g_st().columns_global.get() - g_st().ru_col.get());
    if g_st().sc_col.get() <= 0 {
        g_st().sc_col.set(1);
    }
    if g_st().ru_col.get() <= 0 {
        g_st().ru_col.set(1);
    }
    set_vim_var_nr(VV_ECHOSPACE, Varnumber::from(g_st().sc_col.get() - 1));
}

fn win_redraw_signcols(wp: &WinInfoInfo) -> bool {
    let buf = wp.buf();

    if !buf.b_signcols.autom.get()
        && (!wp.w_onebuf_opt.wo_stc.is_empty()
            || (wp.w_maxscwidth.get() > 1 && wp.w_minscwidth.get() != wp.w_maxscwidth.get()))
    {
        buf.b_signcols.autom.set(true);
        let last_line = buf.b_ml.ml_line_count.get() - 1;
        buf_signcols_count_range(&buf, 0, last_line, MAXLNUM, K_FALSE);
    }

    while buf.b_signcols.max.get() > 0
        && buf.b_signcols.count[(buf.b_signcols.max.get() - 1) as usize].get() == 0
    {
        buf.b_signcols.max.set(buf.b_signcols.max.get() - 1);
    }

    let mut width = wp.w_maxscwidth.get().min(buf.b_signcols.max.get());
    let rebuild_stc = buf.b_signcols.max.get() != buf.b_signcols.last_max.get()
        && !wp.w_onebuf_opt.wo_stc.is_empty();

    if rebuild_stc {
        wp.w_nrwidth_line_count.set(0);
    } else if wp.w_minscwidth.get() == 0 && wp.w_maxscwidth.get() == 1 {
        width = i32::from(buf_meta_total(&buf, K_MTMETA_SIGN_TEXT) > 0);
    }

    let scwidth = wp.w_scwidth.get();
    wp.w_scwidth.set(0.max(wp.w_minscwidth.get()).max(width));
    wp.w_scwidth.get() != scwidth || rebuild_stc
}

fn hsep_connected(wp: &WinInfoInfo, corner: WindowCorner) -> bool {
    let before = corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft;
    let sep_row = if corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight {
        wp.w_winrow.get() - 1
    } else {
        w_endrow(wp)
    };
    let mut fr = wp.frame();

    while !fr.fr_parent.get().is_null() {
        if fr.parent().fr_layout.get() == FR_ROW
            && !(if before {
                fr.fr_prev.get()
            } else {
                fr.fr_next.get()
            })
            .is_null()
        {
            fr = if before { fr.prev() } else { fr.next() };
            break;
        }
        fr = fr.parent();
    }
    if fr.fr_parent.get().is_null() {
        return false;
    }
    while fr.fr_layout.get() != FR_LEAF {
        fr = fr.child();
        if fr.parent().fr_layout.get() == FR_ROW && before {
            while !fr.fr_next.get().is_null() {
                fr = fr.next();
            }
        } else {
            while !fr.fr_next.get().is_null()
                && frame2win(&fr).w_winrow.get() + fr.fr_height.get() < sep_row
            {
                fr = fr.next();
            }
        }
    }

    sep_row == fr.win().w_winrow.get() - 1 || sep_row == w_endrow(&fr.win())
}

fn vsep_connected(wp: &WinInfoInfo, corner: WindowCorner) -> bool {
    let before = corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight;
    let sep_col = if corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft {
        wp.w_wincol.get() - 1
    } else {
        w_endcol(wp)
    };
    let mut fr = wp.frame();

    while !fr.fr_parent.get().is_null() {
        if fr.parent().fr_layout.get() == FR_COL
            && !(if before {
                fr.fr_prev.get()
            } else {
                fr.fr_next.get()
            })
            .is_null()
        {
            fr = if before { fr.prev() } else { fr.next() };
            break;
        }
        fr = fr.parent();
    }
    if fr.fr_parent.get().is_null() {
        return false;
    }
    while fr.fr_layout.get() != FR_LEAF {
        fr = fr.child();
        if fr.parent().fr_layout.get() == FR_COL && before {
            while !fr.fr_next.get().is_null() {
                fr = fr.next();
            }
        } else {
            while !fr.fr_next.get().is_null()
                && frame2win(&fr).w_wincol.get() + fr.fr_width.get() < sep_col
            {
                fr = fr.next();
            }
        }
    }

    sep_col == fr.win().w_wincol.get() - 1 || sep_col == w_endcol(&fr.win())
}

fn draw_vsep_win(wp: &WinInfoInfo) {
    if wp.w_vsep_width.get() == 0 {
        return;
    }

    for row in wp.w_winrow.get()..w_endrow(wp) {
        grid_line_start(&g_st().default_gridview, row);
        grid_line_put_schar(
            w_endcol(wp),
            wp.w_p_fcs_chars.get().vert,
            win_hl_attr(wp, HLF_C),
        );
        grid_line_flush();
    }
}

fn draw_hsep_win(wp: &WinInfoInfo) {
    if wp.w_hsep_height.get() == 0 {
        return;
    }

    grid_line_start(&g_st().default_gridview, w_endrow(wp));
    grid_line_fill(
        wp.w_wincol.get(),
        w_endcol(wp),
        wp.w_p_fcs_chars.get().horiz,
        win_hl_attr(wp, HLF_C),
    );
    grid_line_flush();
}

fn get_corner_sep_connector(wp: &WinInfoInfo, corner: WindowCorner) -> Schar {
    if vsep_connected(wp, corner) {
        if hsep_connected(wp, corner) {
            wp.w_p_fcs_chars.get().verthoriz
        } else if corner == WindowCorner::TopLeft || corner == WindowCorner::BottomLeft {
            wp.w_p_fcs_chars.get().vertright
        } else {
            wp.w_p_fcs_chars.get().vertleft
        }
    } else if corner == WindowCorner::TopLeft || corner == WindowCorner::TopRight {
        wp.w_p_fcs_chars.get().horizdown
    } else {
        wp.w_p_fcs_chars.get().horizup
    }
}

fn draw_sep_connectors_win(wp: &WinInfoInfo) {
    if global_stl_height() == 0 || !(wp.w_hsep_height.get() == 1 || wp.w_vsep_width.get() == 1) {
        return;
    }

    let hl = win_hl_attr(wp, HLF_C);

    let win_at_bottom = wp.w_hsep_height.get() == 0;
    let win_at_right = wp.w_vsep_width.get() == 0;

    let mut frp = wp.frame();
    while !frp.fr_parent.get().is_null() {
        if frp.parent().fr_layout.get() == FR_COL && !frp.fr_prev.get().is_null() {
            break;
        }
        frp = frp.parent();
    }
    let win_at_top = frp.fr_parent.get().is_null();

    frp = wp.frame();
    while !frp.fr_parent.get().is_null() {
        if frp.parent().fr_layout.get() == FR_ROW && !frp.fr_prev.get().is_null() {
            break;
        }
        frp = frp.parent();
    }
    let win_at_left = frp.fr_parent.get().is_null();

    let top_left = !(win_at_top || win_at_left);
    let top_right = !(win_at_top || win_at_right);
    let bot_left = !(win_at_bottom || win_at_left);
    let bot_right = !(win_at_bottom || win_at_right);

    if top_left {
        grid_line_start(&g_st().default_gridview, wp.w_winrow.get() - 1);
        grid_line_put_schar(
            wp.w_wincol.get() - 1,
            get_corner_sep_connector(wp, WindowCorner::TopLeft),
            hl,
        );
        grid_line_flush();
    }
    if top_right {
        grid_line_start(&g_st().default_gridview, wp.w_winrow.get() - 1);
        grid_line_put_schar(
            w_endcol(wp),
            get_corner_sep_connector(wp, WindowCorner::TopRight),
            hl,
        );
        grid_line_flush();
    }
    if bot_left {
        grid_line_start(&g_st().default_gridview, w_endrow(wp));
        grid_line_put_schar(
            wp.w_wincol.get() - 1,
            get_corner_sep_connector(wp, WindowCorner::BottomLeft),
            hl,
        );
        grid_line_flush();
    }
    if bot_right {
        grid_line_start(&g_st().default_gridview, w_endrow(wp));
        grid_line_put_schar(
            w_endcol(wp),
            get_corner_sep_connector(wp, WindowCorner::BottomRight),
            hl,
        );
        grid_line_flush();
    }
}

fn win_update(wp: &WinInfoInfo) {
    if wp.w_grid.target.get() == crate::grid::GridTarget::Default
        && wp.w_wincol.get() >= g_st().columns_global.get()
    {
        return;
    }

    let mut top_end: i32 = 0;
    let mut mid_start: i32 = 999;
    let mut mid_end: i32 = 0;
    let mut bot_start: i32 = 999;
    let mut scrolled_down = false;
    let mut scrolled_for_mod = false;
    let mut top_to_mod = false;
    let mut bot_scroll_start: i32 = 999;
    let mut did_update = DidUpdate::None;

    let mut syntax_last_parsed: Linenr = 0;
    let mut mod_top: Linenr = 0;
    let mut mod_bot: Linenr = 0;

    let mut type_ = wp.w_redr_type.get();

    if type_ >= UPD_NOT_VALID {
        wp.w_redr_status.set(true);
        wp.w_lines_valid.set(0);
    }

    if wp.w_view_height.get() == 0 {
        draw_vsep_win(wp);
        draw_hsep_win(wp);
        wp.w_redr_type.set(0);
        return;
    }

    if wp.w_view_width.get() == 0 {
        draw_vsep_win(wp);
        wp.w_redr_type.set(0);
        return;
    }

    let buf = wp.buf();

    let save_got_int = g_st().got_int.get();
    g_st().got_int.set(false);
    let syntax_tm = profile_setlimit(opt_vars().p_rdt.get());
    syn_set_timeout(Some(syntax_tm));

    g_st().win_extmark_arr.borrow_mut().clear();

    decor_redraw_reset(wp, &mut g_st().decor_state.borrow_mut());
    decor_providers_invoke_win(wp);

    if let Some(term) = buf.terminal.get().opt()
        && terminal_suspended(term)
    {
        let mut susp_vt = ds_st(|st| st.suspended_virt_text.get());
        if susp_vt.is_null() {
            let mut vt = DecorVirtText {
                flags: 0,
                hl_mode: 0,
                priority: DECOR_PRIORITY_BASE,
                width: 0,
                col: 0,
                pos: K_VPOS_WIN_COL,
                data: DecorVirtTextData::empty(),
                next: crate::decoration::DecorVirtId::NULL,
            };
            vt.data.virt_text_mut().push(VirtTextChunk {
                text: Some(crate::option_vars::RcCStr::from_static(
                    c"[Process suspended]",
                )),
                hl_id: -1,
            });
            susp_vt = crate::decoration::decor_put_vt(vt, crate::decoration::DecorVirtId::NULL);
            ds_st(|st| st.suspended_virt_text.set(susp_vt));
        }
        decor_range_add_virt(
            &mut g_st().decor_state.borrow_mut(),
            buf.b_ml.ml_line_count.get() - 1,
            0,
            buf.b_ml.ml_line_count.get() - 1,
            0,
            susp_vt,
            false,
        );
    }

    for win in for_all_windows_ptr() {
        if wp.w_buffer.get() == win.w_buffer.get() && win_redraw_signcols(&win) {
            changed_line_abv_curs_win(&win);
            redraw_later(Some(&win), UPD_NOT_VALID);
        }
    }
    buf.b_signcols.last_max.set(buf.b_signcols.max.get());

    validate_virtcol(wp);
    type_ = wp.w_redr_type.get();

    init_search_hl(wp, &mut g_st().screen_search_hl.borrow_mut());

    if wp.w_skipcol.get() > 0 && wp.w_view_width.get() > win_col_off(wp) {
        let mut w: i32 = 0;
        let width1 = wp.w_view_width.get() - win_col_off(wp);
        let width2 = width1 + win_col_off2(wp);
        let mut add = width1;

        while w < wp.w_skipcol.get() {
            if w > 0 {
                add = width2;
            }
            w += add;
        }
        if w != wp.w_skipcol.get() {
            wp.w_skipcol.set(w - add);
        }
    }
    if wp.w_skipcol.get() > 0 && wp.w_cursor.get().lnum != wp.w_topline.get() {
        let tl = wp.w_topline.get();
        if linetabsize_disp_eol(wp, tl) <= wp.w_skipcol.get() {
            wp.w_skipcol.set(0);
        }
    }

    let nrwidth_before = wp.w_nrwidth.get();
    let nrwidth_new = if wp.w_onebuf_opt.wo_nu.get()
        || wp.w_onebuf_opt.wo_rnu.get()
        || !wp.w_onebuf_opt.wo_stc.is_empty()
    {
        number_width(wp)
    } else {
        0
    };

    if wp.w_nrwidth.get() != nrwidth_new {
        type_ = UPD_NOT_VALID;
        changed_line_abv_curs_win(wp);
        wp.w_nrwidth.set(nrwidth_new);
    } else {
        mod_top = wp.w_redraw_top.get();
        mod_bot = if wp.w_redraw_bot.get() != 0 {
            wp.w_redraw_bot.get() + 1
        } else {
            0
        };
        if buf.b_mod_set.get() {
            if mod_top == 0 || mod_top > buf.b_mod_top.get() {
                mod_top = buf.b_mod_top.get();
                if syntax_present(wp) {
                    mod_top -= buf.b_s.b_syn_sync_linebreaks.get();
                    mod_top = mod_top.max(1);
                }
            }
            if mod_bot == 0 || mod_bot < buf.b_mod_bot.get() {
                mod_bot = buf.b_mod_bot.get();
            }

            let search_regprog = g_st().screen_search_hl.borrow().rm.regprog.clone();
            if let Some(regprog) = search_regprog.opt()
                && re_multiline(regprog) != 0
            {
                top_to_mod = true;
            } else {
                top_to_mod = wp.w_match_head.borrow().iter().any(|cur| {
                    cur.mit_match
                        .regprog
                        .opt()
                        .is_some_and(|rp| re_multiline(rp) != 0)
                });
            }
        }

        if g_st().search_hl_has_cursor_lnum.get() > 0 {
            if mod_top == 0 || mod_top > g_st().search_hl_has_cursor_lnum.get() {
                mod_top = g_st().search_hl_has_cursor_lnum.get();
            }
            if mod_bot == 0 || mod_bot < g_st().search_hl_has_cursor_lnum.get() + 1 {
                mod_bot = g_st().search_hl_has_cursor_lnum.get() + 1;
            }
        }

        if mod_top != 0 && win_lines_concealed(wp) {
            let mut lnumt = wp.w_topline.get();
            let mut lnumb = MAXLNUM;
            for i in 0..wp.w_lines_valid.get() {
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

            has_folding(wp, mod_top, Some(&mut mod_top), None);
            mod_top = mod_top.min(lnumt);

            mod_bot -= 1;
            has_folding(wp, mod_bot, None, Some(&mut mod_bot));
            mod_bot += 1;
            mod_bot = mod_bot.max(lnumb);
        }

        if mod_top != 0 && mod_top < wp.w_topline.get() {
            if mod_bot > wp.w_topline.get() {
                mod_top = wp.w_topline.get();
            } else if syntax_present(wp) {
                top_end = 1;
            }
        }
    }

    wp.w_redraw_top.set(0);
    wp.w_redraw_bot.set(0);
    g_st().search_hl_has_cursor_lnum.set(0);

    if type_ == UPD_REDRAW_TOP {
        let mut j: i32 = 0;
        for i in 0..wp.w_lines_valid.get() {
            j += i32::from(w_lines_at(wp, i as usize).wl_size);
            if j >= wp.w_upd_rows.get() {
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

    let mut topline_conceal = wp.w_topline.get();
    while topline_conceal < buf.b_ml.ml_line_count.get()
        && decor_conceal_line(wp, topline_conceal - 1, false)
    {
        topline_conceal += 1;
        has_folding(wp, topline_conceal, None, Some(&mut topline_conceal));
    }

    if (type_ == UPD_VALID
        || type_ == UPD_SOME_VALID
        || type_ == UPD_INVERTED
        || type_ == UPD_INVERTED_ALL)
        && !wp.w_botfill.get()
        && !wp.w_old_botfill.get()
    {
        let wl0 = *w_lines_at(wp, 0);
        if mod_top != 0
            && wp.w_topline.get() == mod_top
            && (!wl0.wl_valid || topline_conceal == wl0.wl_lnum)
        {
            // w_topline is the first changed line and window is not scrolled
        } else if wl0.wl_valid
            && (topline_conceal < wl0.wl_lnum
                || (topline_conceal == wl0.wl_lnum && wp.w_topfill.get() > wp.w_old_topfill.get()))
        {
            let mut j: i32;
            if win_lines_concealed(wp) {
                j = 0;
                let mut ln = wp.w_topline.get();
                while ln < wl0.wl_lnum {
                    j += i32::from(!decor_conceal_line(wp, ln - 1, false));
                    if j >= wp.w_view_height.get() - 2 {
                        break;
                    }
                    has_folding(wp, ln, None, Some(&mut ln));
                    ln += 1;
                }
            } else {
                j = wl0.wl_lnum - wp.w_topline.get();
            }
            if j < wp.w_view_height.get() - 2 {
                let mut i = plines_m_win(
                    wp,
                    wp.w_topline.get(),
                    wl0.wl_lnum - 1,
                    wp.w_view_height.get(),
                );
                if wl0.wl_lnum != wp.w_topline.get() {
                    i += win_get_fill(wp, wl0.wl_lnum) - wp.w_old_topfill.get();
                }
                if i != 0 && i < wp.w_view_height.get() - 2 {
                    win_scroll_lines(wp, 0, i);
                    bot_scroll_start = 0;
                    if wp.w_lines_valid.get() != 0 {
                        top_end = i;
                        scrolled_down = true;

                        wp.w_lines_valid.set(wp.w_lines_valid.get() + (j));
                        if wp.w_lines_valid.get() > wp.w_view_height.get() {
                            wp.w_lines_valid.set(wp.w_view_height.get());
                        }
                        let mut idx = wp.w_lines_valid.get();
                        while idx - j >= 0 {
                            let src_wl = *w_lines_at(wp, (idx - j) as usize);
                            *w_lines_at(wp, idx as usize) = src_wl;
                            idx -= 1;
                        }
                        while idx >= 0 {
                            w_lines_at(wp, idx as usize).wl_valid = false;
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
            let mut j: i32 = -1;
            let mut row: i32 = 0;
            for i in 0..wp.w_lines_valid.get() {
                let wl = *w_lines_at(wp, i as usize);
                if wl.wl_valid && wl.wl_lnum == wp.w_topline.get() {
                    j = i;
                    break;
                }
                row += i32::from(wl.wl_size);
            }
            if j == -1 {
                mid_start = 0;
            } else {
                if wl0.wl_lnum == wp.w_topline.get() {
                    row += wp.w_old_topfill.get();
                } else {
                    row += win_get_fill(wp, wp.w_topline.get());
                }
                row -= wp.w_topfill.get();
                if row > 0 {
                    win_scroll_lines(wp, 0, -row);
                    bot_start = wp.w_view_height.get() - row;
                    bot_scroll_start = bot_start;
                }
                if (row == 0 || bot_start < 999) && wp.w_lines_valid.get() != 0 {
                    bot_start = 0;
                    let mut idx: i32 = 0;
                    loop {
                        let wlj = *w_lines_at(wp, j as usize);
                        *w_lines_at(wp, idx as usize) = wlj;
                        if row > 0
                            && bot_start + row + i32::from(wlj.wl_size) > wp.w_view_height.get()
                        {
                            wp.w_lines_valid.set(idx + 1);
                            break;
                        }
                        bot_start += i32::from(wlj.wl_size);
                        idx += 1;

                        j += 1;
                        if j >= wp.w_lines_valid.get() {
                            wp.w_lines_valid.set(idx);
                            break;
                        }
                    }

                    if win_may_fill(wp) && bot_start > 0 {
                        w_lines_at(wp, 0).wl_size =
                            plines_correct_topline(wp, wp.w_topline.get(), None, true, None) as u16;
                    }
                }
            }
        }

        if mid_start == 0 {
            mid_end = wp.w_view_height.get();
        }
    } else {
        mid_start = 0;
        mid_end = wp.w_view_height.get();
    }

    if type_ == UPD_SOME_VALID {
        mid_start = 0;
        mid_end = wp.w_view_height.get();
        type_ = UPD_NOT_VALID;
    }

    if (g_st().visual_global.borrow().active && curwin().w_buffer.get() == buf)
        || (wp.w_old_cursor_lnum.get() != 0 && type_ != UPD_NOT_VALID)
    {
        let mut from: Linenr;
        let mut to: Linenr;

        if g_st().visual_global.borrow().active {
            if g_st().visual_global.borrow().mode != i32::from(wp.w_old_visual_mode.get())
                || type_ == UPD_INVERTED_ALL
            {
                if curwin().w_cursor.get().lnum < g_st().visual_global.borrow().start.lnum {
                    from = curwin().w_cursor.get().lnum;
                    to = g_st().visual_global.borrow().start.lnum;
                } else {
                    from = g_st().visual_global.borrow().start.lnum;
                    to = curwin().w_cursor.get().lnum;
                }
                from = from
                    .min(wp.w_old_cursor_lnum.get())
                    .min(wp.w_old_visual_lnum.get());
                to = to
                    .max(wp.w_old_cursor_lnum.get())
                    .max(wp.w_old_visual_lnum.get());
            } else {
                if curwin().w_cursor.get().lnum < wp.w_old_cursor_lnum.get() {
                    from = curwin().w_cursor.get().lnum;
                    to = wp.w_old_cursor_lnum.get();
                } else {
                    from = wp.w_old_cursor_lnum.get();
                    to = curwin().w_cursor.get().lnum;
                    if from == 0 {
                        from = to;
                    }
                }

                if g_st().visual_global.borrow().start.lnum != wp.w_old_visual_lnum.get()
                    || g_st().visual_global.borrow().start.col != wp.w_old_visual_col.get()
                {
                    if wp.w_old_visual_lnum.get() < from && wp.w_old_visual_lnum.get() != 0 {
                        from = wp.w_old_visual_lnum.get();
                    }
                    to = to
                        .max(wp.w_old_visual_lnum.get())
                        .max(g_st().visual_global.borrow().start.lnum);
                    from = from.min(g_st().visual_global.borrow().start.lnum);
                }
            }

            if g_st().visual_global.borrow().mode == CTRL_V_CONST {
                let mut fromc: Colnr = 0;
                let mut toc: Colnr = 0;
                let mut cursor = curwin().w_cursor.get();
                let mut vstart = g_st().visual_global.borrow().start;
                getvcols(
                    wp,
                    &mut vstart,
                    &mut cursor,
                    &mut fromc,
                    &mut toc,
                    GETVCOL_END_EXCL_LBR,
                );
                g_st().visual_global.borrow_mut().start = vstart;
                curwin().w_cursor.set(cursor);
                toc += 1;
                if curwin().w_curswant.get() == MAXCOL {
                    if (get_ve_flags(&curwin()) & K_OPT_VE_FLAG_BLOCK) != 0 {
                        let cursor_above =
                            curwin().w_cursor.get().lnum < g_st().visual_global.borrow().start.lnum;
                        toc = 0;
                        let mut pos = CursorPos {
                            lnum: curwin().w_cursor.get().lnum,
                            col: 0,
                            coladd: 0,
                        };
                        while if cursor_above {
                            pos.lnum <= g_st().visual_global.borrow().start.lnum
                        } else {
                            pos.lnum >= g_st().visual_global.borrow().start.lnum
                        } {
                            let mut t: Colnr = 0;
                            pos.col = ml_get_buf_len(&wp.buf(), pos.lnum);
                            getvvcol(wp, &mut pos, None, None, Some(&mut t), 0);
                            toc = toc.max(t);
                            pos.lnum += if cursor_above { 1 } else { -1 };
                        }
                        toc += 1;
                    } else {
                        toc = MAXCOL;
                    }
                }

                if fromc != wp.w_old_cursor_fcol.get() || toc != wp.w_old_cursor_lcol.get() {
                    from = from.min(g_st().visual_global.borrow().start.lnum);
                    to = to.max(g_st().visual_global.borrow().start.lnum);
                }
                wp.w_old_cursor_fcol.set(fromc);
                wp.w_old_cursor_lcol.set(toc);
            }
        } else if wp.w_old_cursor_lnum.get() < wp.w_old_visual_lnum.get() {
            from = wp.w_old_cursor_lnum.get();
            to = wp.w_old_visual_lnum.get();
        } else {
            from = wp.w_old_visual_lnum.get();
            to = wp.w_old_cursor_lnum.get();
        }

        from = from.max(wp.w_topline.get());

        if (wp.w_valid.get() & VALID_BOTLINE) != 0 {
            from = from.min(wp.w_botline.get());
            to = to.min(wp.w_botline.get());
        }

        if mid_start > 0 {
            let mut lnum = wp.w_topline.get();
            let mut idx: i32 = 0;
            let mut srow: i32 = 0;
            mid_start = if scrolled_down { top_end } else { 0 };
            while lnum < from && idx < wp.w_lines_valid.get() {
                let wl = *w_lines_at(wp, idx as usize);
                if wl.wl_valid {
                    mid_start += i32::from(wl.wl_size);
                } else if !scrolled_down {
                    srow += i32::from(wl.wl_size);
                }
                idx += 1;
                if idx < wp.w_lines_valid.get() && w_lines_at(wp, idx as usize).wl_valid {
                    lnum = w_lines_at(wp, idx as usize).wl_lnum;
                } else {
                    lnum += 1;
                }
            }
            srow += mid_start;
            mid_end = wp.w_view_height.get();
            while idx < wp.w_lines_valid.get() {
                let wl = *w_lines_at(wp, idx as usize);
                if wl.wl_valid && wl.wl_lnum > to {
                    mid_end = srow;
                    break;
                }
                srow += i32::from(wl.wl_size);
                idx += 1;
            }
        }
    }

    if g_st().visual_global.borrow().active && curwin().w_buffer.get() == buf {
        wp.w_old_visual_mode
            .set(g_st().visual_global.borrow().mode as u8);
        wp.w_old_cursor_lnum.set(curwin().w_cursor.get().lnum);
        wp.w_old_visual_lnum
            .set(g_st().visual_global.borrow().start.lnum);
        wp.w_old_visual_col
            .set(g_st().visual_global.borrow().start.col);
        wp.w_old_curswant.set(curwin().w_curswant.get());
    } else {
        wp.w_old_visual_mode.set(0);
        wp.w_old_cursor_lnum.set(0);
        wp.w_old_visual_lnum.set(0);
        wp.w_old_visual_col.set(0);
    }

    let mut cursorline_fi = Foldinfo::new();
    win_update_cursorline(wp, &mut cursorline_fi);
    if wp == curwin_ptr() {
        let used = conceal_cursor_line(&curwin());
        ds_st(|st| st.conceal_cursor_used.set(used));
    }

    win_check_ns_hl(Some(wp));

    let mut spv = Spellvars::default();
    let mut lnum = wp.w_topline.get();
    if spell_check_window(wp) {
        spv.spv_has_spell = true;
        spv.spv_unchanged = mod_top == 0;
    }

    let mut idx: i32 = 0;
    let mut row: i32 = 0;
    let mut srow: i32 = 0;

    let mut eof = false;
    let mut didline = false;

    'statuscol_retry: loop {
        loop {
            if row == wp.w_view_height.get() {
                didline = true;
                break;
            }

            if lnum > buf.b_ml.ml_line_count.get() {
                eof = true;
                break;
            }

            srow = row;

            if row < top_end
                || (row >= mid_start && row < mid_end)
                || top_to_mod
                || idx >= wp.w_lines_valid.get()
                || (row + i32::from(w_lines_at(wp, idx as usize).wl_size) > bot_start)
                || (mod_top != 0
                    && (lnum == mod_top
                        || (lnum >= mod_top
                            && (lnum < mod_bot
                                || did_update == DidUpdate::Fold
                                || (did_update == DidUpdate::Line
                                    && syntax_present(wp)
                                    && ((foldmethod_is_syntax(wp) && has_any_folding(wp) != 0)
                                        || syntax_check_changed(lnum)))
                                || (!wp.w_match_head.borrow().is_empty()
                                    && buf.b_mod_set.get()
                                    && buf.b_mod_xlines.get() != 0)))))
                || lnum == wp.w_cursorline.get()
                || lnum == wp.w_last_cursorline.get()
            {
                if lnum == mod_top {
                    top_to_mod = false;
                }

                let foldinfo = if wp.w_onebuf_opt.wo_cul.get() && lnum == wp.w_cursor.get().lnum {
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

                    let mut old_cline_height: i32 = 0;
                    let mut old_rows: i32 = 0;
                    let mut i = idx;

                    while i < wp.w_lines_valid.get() {
                        let wli = *w_lines_at(wp, i as usize);
                        if wli.wl_valid && wli.wl_lnum == mod_bot {
                            break;
                        }
                        if wli.wl_lnum == wp.w_cursor.get().lnum {
                            old_cline_height = i32::from(wli.wl_size);
                        }
                        old_rows += i32::from(wli.wl_size);
                        if wli.wl_valid && wli.wl_lastlnum + 1 == mod_bot {
                            i += 1;
                            while i < wp.w_lines_valid.get() && !w_lines_at(wp, i as usize).wl_valid
                            {
                                old_rows += i32::from(w_lines_at(wp, i as usize).wl_size);
                                i += 1;
                            }
                            break;
                        }
                        i += 1;
                    }

                    if i >= wp.w_lines_valid.get() {
                        bot_start = 0;
                        bot_scroll_start = 0;
                    } else {
                        let mut new_rows: i32 = 0;
                        let mut j = idx;
                        let mut l = lnum;
                        while l < mod_bot {
                            if g_st().dollar_vcol.get() >= 0
                                && wp == curwin_ptr()
                                && old_cline_height > 0
                                && l == wp.w_cursor.get().lnum
                            {
                                new_rows += old_cline_height;
                                j += 1;
                            } else {
                                let n = plines_correct_topline(wp, l, Some(&mut l), true, None);
                                new_rows += n;
                                j += i32::from(n > 0);
                            }
                            if new_rows > wp.w_view_height.get() - row - 2 {
                                new_rows = 9999;
                                break;
                            }
                            l += 1;
                        }
                        let xtra_rows = new_rows - old_rows;
                        if xtra_rows < 0 {
                            if row - xtra_rows >= wp.w_view_height.get() - 2 {
                                mod_bot = MAXLNUM;
                            } else {
                                win_scroll_lines(wp, row, xtra_rows);
                                bot_start = wp.w_view_height.get() + xtra_rows;
                                bot_scroll_start = bot_start;
                            }
                        } else if xtra_rows > 0 {
                            if row + xtra_rows >= wp.w_view_height.get() - 2 {
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
                                    if i >= wp.w_lines_valid.get() {
                                        wp.w_lines_valid.set(j);
                                        break;
                                    }
                                    let wli = *w_lines_at(wp, i as usize);
                                    *w_lines_at(wp, j as usize) = wli;
                                    if x + i32::from(wli.wl_size) > wp.w_view_height.get() {
                                        wp.w_lines_valid.set(j + 1);
                                        break;
                                    }
                                    x += i32::from(wli.wl_size);
                                    j += 1;
                                    i += 1;
                                }
                                bot_start = bot_start.min(x);
                            } else {
                                j -= i;
                                wp.w_lines_valid.set(wp.w_lines_valid.get() + (j));
                                wp.w_lines_valid
                                    .set(wp.w_lines_valid.get().min(wp.w_view_height.get()));
                                i = wp.w_lines_valid.get();
                                while i - j >= idx {
                                    let src_wl = *w_lines_at(wp, (i - j) as usize);
                                    *w_lines_at(wp, i as usize) = src_wl;
                                    i -= 1;
                                }
                                while i >= idx {
                                    w_lines_at(wp, i as usize).wl_size = 0;
                                    w_lines_at(wp, i as usize).wl_valid = false;
                                    i -= 1;
                                }
                            }
                        }
                    }
                }

                let wl_cur = *w_lines_at(wp, idx as usize);
                if foldinfo.fi_lines == 0
                    && idx < wp.w_lines_valid.get()
                    && wl_cur.wl_valid
                    && wl_cur.wl_lnum == lnum
                    && lnum > wp.w_topline.get()
                    && (opt_vars().dy_flags.get()
                        & (K_OPT_DY_FLAG_LASTLINE | K_OPT_DY_FLAG_TRUNCATE))
                        == 0
                    && srow + i32::from(wl_cur.wl_size) > wp.w_view_height.get()
                    && win_get_fill(wp, lnum) == 0
                {
                    row = wp.w_view_height.get() + 1;
                } else {
                    prepare_search_hl(wp, &mut g_st().screen_search_hl.borrow_mut(), lnum);
                    if syntax_last_parsed != 0
                        && syntax_last_parsed + 1 < lnum
                        && syntax_present(wp)
                    {
                        syntax_end_parsing(wp, syntax_last_parsed + 1);
                    }

                    let display_buf_line = !concealed
                        && (foldinfo.fi_lines == 0
                            || wp.w_onebuf_opt.wo_fdt.cb_type() == K_CALLBACK_NONE_CONST);

                    if display_buf_line {
                        decor_providers_invoke_conceal(wp, lnum - 1, false);
                    }

                    let mut zero_spv = Spellvars::default();
                    row = win_line(
                        wp,
                        lnum,
                        srow,
                        wp.w_view_height.get(),
                        0,
                        concealed,
                        if display_buf_line {
                            &mut spv
                        } else {
                            &mut zero_spv
                        },
                        foldinfo,
                    );

                    if display_buf_line {
                        syntax_last_parsed = lnum;
                    } else {
                        spv.spv_capcol_lnum = 0;
                    }

                    let lastlnum = lnum + foldinfo.fi_lines - Linenr::from(foldinfo.fi_lines > 0);
                    {
                        let mut wl_ptr = w_lines_at(wp, idx as usize);
                        wl_ptr.wl_folded = foldinfo.fi_lines > 0;
                        wl_ptr.wl_foldend = lastlnum;
                        wl_ptr.wl_lastlnum = lastlnum;
                    }
                    did_update = if foldinfo.fi_lines > 0 {
                        DidUpdate::Fold
                    } else {
                        DidUpdate::Line
                    };

                    let mut virt_below =
                        decor_virt_lines(wp, lastlnum, lastlnum + 1, None, None, true) > 0;
                    while !virt_below
                        && w_lines_at(wp, idx as usize).wl_lastlnum < buf.b_ml.ml_line_count.get()
                        && {
                            let lastlnum = w_lines_at(wp, idx as usize).wl_lastlnum;
                            decor_conceal_line(wp, lastlnum, false)
                        }
                    {
                        virt_below = false;
                        let mut last_lnum = w_lines_at(wp, idx as usize).wl_lastlnum + 1;
                        has_folding(wp, last_lnum, None, Some(&mut last_lnum));
                        w_lines_at(wp, idx as usize).wl_lastlnum = last_lnum;
                    }
                }

                w_lines_at(wp, idx as usize).wl_lnum = lnum;
                w_lines_at(wp, idx as usize).wl_valid = true;

                let is_curline = wp == curwin_ptr() && lnum == wp.w_cursor.get().lnum;

                if row > wp.w_view_height.get() {
                    if g_st().dollar_vcol.get() == -1 || !is_curline {
                        w_lines_at(wp, idx as usize).wl_size = plines_win(wp, lnum, true) as u16;
                    }
                    idx += 1;
                    break;
                }
                if g_st().dollar_vcol.get() == -1 || !is_curline {
                    w_lines_at(wp, idx as usize).wl_size = (row - srow) as u16;
                }
                lnum = w_lines_at(wp, idx as usize).wl_lastlnum + 1;
                idx += 1;
            } else {
                if (wp.w_onebuf_opt.wo_nu.get()
                    && mod_top != 0
                    && lnum >= mod_bot
                    && buf.b_mod_set.get()
                    && buf.b_mod_xlines.get() != 0)
                    || (wp.w_onebuf_opt.wo_rnu.get()
                        && wp.w_last_cursor_lnum_rnu.get() != wp.w_cursor.get().lnum)
                {
                    let info = if wp.w_onebuf_opt.wo_cul.get() && lnum == wp.w_cursor.get().lnum {
                        cursorline_fi
                    } else {
                        fold_info(wp, lnum)
                    };
                    let view_height = wp.w_view_height.get();
                    let wl_size = i32::from(w_lines_at(wp, idx as usize).wl_size);
                    win_line(wp, lnum, srow, view_height, wl_size, false, &mut spv, info);
                }

                row += i32::from(w_lines_at(wp, idx as usize).wl_size);
                idx += 1;
                if row > wp.w_view_height.get() {
                    break;
                }
                lnum = w_lines_at(wp, (idx - 1) as usize).wl_lastlnum + 1;
                did_update = DidUpdate::None;
                spv.spv_capcol_lnum = 0;
            }

            if wp.w_redr_statuscol.get() {
                wp.w_redr_statuscol.set(false);
                idx = 0;
                row = 0;
                lnum = wp.w_topline.get();
                wp.w_lines_valid.set(0);
                wp.w_valid.set(wp.w_valid.get() & (!VALID_WCOL));
                decor_redraw_reset(wp, &mut g_st().decor_state.borrow_mut());
                decor_providers_invoke_win(wp);
                continue;
            }

            if lnum > buf.b_ml.ml_line_count.get() {
                eof = true;
                break;
            }
        }

        wp.w_last_cursorline.set(wp.w_cursorline.get());
        wp.w_last_cursor_lnum_rnu
            .set(if wp.w_onebuf_opt.wo_rnu.get() {
                wp.w_cursor.get().lnum
            } else {
                0
            });
        wp.w_lines_valid.set(wp.w_lines_valid.get().max(idx));
        wp.w_display_tick.set(display_tick_val());

        if syntax_last_parsed != 0 && syntax_present(wp) {
            syntax_end_parsing(wp, syntax_last_parsed + 1);
        }

        let old_botline = wp.w_botline.get();

        wp.w_empty_rows.set(0);
        wp.w_filler_rows.set(0);
        if !eof && !didline {
            let at_attr = hl_combine_attr(win_bg_attr(wp), win_hl_attr(wp, HLF_AT));
            if lnum == wp.w_topline.get() {
                wp.w_botline.set(lnum + 1);
            } else if win_get_fill(wp, lnum) >= wp.w_view_height.get() - srow {
                wp.w_botline.set(lnum);
                wp.w_filler_rows.set(wp.w_view_height.get() - srow);
            } else if (opt_vars().dy_flags.get() & K_OPT_DY_FLAG_TRUNCATE) != 0 {
                grid_line_start(&wp.w_grid, wp.w_view_height.get() - 1);
                grid_line_fill(
                    0,
                    wp.w_view_width.get().min(3),
                    wp.w_p_fcs_chars.get().lastline,
                    at_attr,
                );
                grid_line_fill(3, wp.w_view_width.get(), schar_from_ascii(b' '), at_attr);
                grid_line_flush();
                set_empty_rows(wp, srow);
                wp.w_botline.set(lnum);
            } else if (opt_vars().dy_flags.get() & K_OPT_DY_FLAG_LASTLINE) != 0 {
                grid_line_start(&wp.w_grid, wp.w_view_height.get() - 1);
                let width = if grid_line_getchar((wp.w_view_width.get() - 3).max(0), None) == 0 {
                    4
                } else {
                    3
                };
                grid_line_fill(
                    (wp.w_view_width.get() - width).max(0),
                    wp.w_view_width.get(),
                    wp.w_p_fcs_chars.get().lastline,
                    at_attr,
                );
                grid_line_flush();
                set_empty_rows(wp, srow);
                wp.w_botline.set(lnum);
            } else {
                win_draw_end(
                    wp,
                    wp.w_p_fcs_chars.get().lastline,
                    true,
                    srow,
                    wp.w_view_height.get(),
                    HLF_AT,
                );
                set_empty_rows(wp, srow);
                wp.w_botline.set(lnum);
            }
        } else {
            if eof {
                wp.w_botline.set(buf.b_ml.ml_line_count.get() + 1);
                let j = win_get_fill(wp, wp.w_botline.get());
                if j > 0 && !wp.w_botfill.get() && row < wp.w_view_height.get() {
                    let mut zero_spv = Spellvars::default();
                    let zero_foldinfo = Foldinfo::new();
                    row = win_line(
                        wp,
                        wp.w_botline.get(),
                        row,
                        wp.w_view_height.get(),
                        0,
                        false,
                        &mut zero_spv,
                        zero_foldinfo,
                    );
                    if wp.w_redr_statuscol.get() {
                        eof = false;
                        didline = false;
                        wp.w_redr_statuscol.set(false);
                        idx = 0;
                        row = 0;
                        lnum = wp.w_topline.get();
                        wp.w_lines_valid.set(0);
                        wp.w_valid.set(wp.w_valid.get() & (!VALID_WCOL));
                        decor_redraw_reset(wp, &mut g_st().decor_state.borrow_mut());
                        decor_providers_invoke_win(wp);
                        continue 'statuscol_retry;
                    }
                }
            } else if g_st().dollar_vcol.get() == -1 || wp != curwin_ptr() {
                wp.w_botline.set(lnum);
            }

            let mut lastline = bot_scroll_start;
            if mid_end >= row {
                lastline = lastline.min(mid_start);
            }
            if mod_bot > buf.b_ml.ml_line_count.get() {
                lastline = 0;
            }

            win_draw_end(
                wp,
                wp.w_p_fcs_chars.get().eob,
                false,
                lastline.max(row),
                wp.w_view_height.get(),
                HLF_EOB,
            );
            set_empty_rows(wp, row);
        }

        if wp.w_redr_type.get() >= UPD_REDRAW_TOP {
            draw_vsep_win(wp);
            draw_hsep_win(wp);
        }
        syn_set_timeout(None);

        wp.w_redr_type.set(0);
        wp.w_old_topfill.set(wp.w_topfill.get());
        wp.w_old_botfill.set(wp.w_botfill.get());

        for n in 0..g_st().win_extmark_arr.borrow().len() {
            let item = g_st().win_extmark_arr.borrow().as_slice()[n];
            ui_call_win_extmark(
                Integer::from(wp.w_grid_alloc.handle.get()),
                wp.handle,
                Integer::from(item.ns_id),
                item.mark_id as Integer,
                Integer::from(item.win_row),
                Integer::from(item.win_col),
            );
        }

        if g_st().dollar_vcol.get() == -1 || wp != curwin_ptr() {
            wp.w_valid.set(wp.w_valid.get() | (VALID_BOTLINE));
            wp.w_viewport_invalid.set(true);
            if wp == curwin_ptr()
                && wp.w_botline.get() != old_botline
                && !ds_st(|st| st.win_update_recursive.replace(true))
            {
                curwin()
                    .w_valid
                    .set(curwin().w_valid.get() & (!VALID_TOPLINE));
                update_topline(&curwin());
                if g_st().must_redraw.get() != 0 {
                    let mod_set = curbuf().b_mod_set.get();
                    curbuf().b_mod_set.set(false);
                    curs_columns(&curwin(), true);
                    win_update(&curwin());
                    g_st().must_redraw.set(0);
                    curbuf().b_mod_set.set(mod_set);
                }
                ds_st(|st| st.win_update_recursive.set(false));
            }
        }

        if nrwidth_before != wp.w_nrwidth.get()
            && let Some(term) = buf.terminal.get().opt()
        {
            terminal_check_size(term);
        }

        if !g_st().got_int.get() {
            g_st().got_int.set(save_got_int);
        }
        break 'statuscol_retry;
    }
}

pub fn win_scroll_lines(wp: &WinInfoInfo, row: i32, line_count: i32) {
    if !redrawing() || line_count == 0 {
        return;
    }

    let mut col: i32 = 0;
    let mut row_off: i32 = 0;
    let grid = grid_adjust(&wp.w_grid, &mut row_off, &mut col);

    let checked_width = (grid.cols.get() - col).min(wp.w_view_width.get());
    let checked_height = (grid.rows.get() - row_off).min(wp.w_view_height.get());

    if checked_width <= 0 || row + line_count.abs() >= checked_height {
        return;
    }

    if line_count < 0 {
        grid_del_lines(
            &grid,
            row + row_off,
            -line_count,
            checked_height + row_off,
            col,
            checked_width,
        );
    } else {
        grid_ins_lines(
            &grid,
            row + row_off,
            line_count,
            checked_height + row_off,
            col,
            checked_width,
        );
    }
}

pub fn win_draw_end(
    wp: &WinInfoInfo,
    c1: Schar,
    draw_margin: bool,
    startrow: i32,
    endrow: i32,
    hl: Hlf,
) {
    assert!((0..HLF_COUNT).contains(&hl));
    let view_width = wp.w_view_width.get();
    let fdc = compute_foldcolumn(wp, 0);
    let scwidth = wp.w_scwidth.get();

    for row in startrow..endrow {
        grid_line_start(&wp.w_grid, row);

        let mut n: i32 = 0;
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

            if (wp.w_onebuf_opt.wo_nu.get() || wp.w_onebuf_opt.wo_rnu.get())
                && !crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_NUMCOL)
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

        if wp.w_onebuf_opt.wo_rl.get() {
            grid_line_mirror(view_width);
        }
        grid_line_flush();
    }
}

pub fn compute_foldcolumn(wp: &WinInfoInfo, col: i32) -> i32 {
    let fdc = win_fdccol_count(wp);
    let wmw = if wp == curwin_ptr() && opt_vars().p_wmw.get() == 0 {
        1
    } else {
        opt_vars().p_wmw.get() as i32
    };
    let n = wp.w_view_width.get() - (col + wmw);
    fdc.min(n)
}

pub fn number_width(wp: &WinInfoInfo) -> i32 {
    let mut lnum = if wp.w_onebuf_opt.wo_rnu.get() && !wp.w_onebuf_opt.wo_nu.get() {
        wp.w_view_height.get()
    } else {
        wp.buf().b_ml.ml_line_count.get()
    };

    if lnum == wp.w_nrwidth_line_count.get() {
        return wp.w_nrwidth_width.get();
    }
    wp.w_nrwidth_line_count.set(lnum);

    if !wp.w_onebuf_opt.wo_stc.is_empty() {
        wp.w_statuscol_line_count.set(0);
        wp.w_nrwidth_width.set(
            i32::from(wp.w_onebuf_opt.wo_nu.get() || wp.w_onebuf_opt.wo_rnu.get())
                * (wp.w_onebuf_opt.wo_nuw.get() as i32),
        );
        return wp.w_nrwidth_width.get();
    }

    let mut n: i32 = 0;
    loop {
        lnum /= 10;
        n += 1;
        if lnum <= 0 {
            break;
        }
    }

    n = n.max((wp.w_onebuf_opt.wo_nuw.get() as i32) - 1);

    if n < 2
        && buf_meta_total(&wp.buf(), K_MTMETA_SIGN_TEXT) != 0
        && wp.w_minscwidth.get() == SCL_NUM
    {
        n = 2;
    }

    wp.w_nrwidth_width.set(n);
    n
}

pub fn redraw_later(wp: Option<&WinInfoInfo>, type_: i32) {
    assert!(wp.is_some() || g_st().exiting.get());
    if !g_st().exiting.get()
        && !g_st().redraw_not_allowed.get()
        && wp.unwrap().w_redr_type.get() < type_
    {
        wp.unwrap().w_redr_type.set(type_);
        if type_ >= UPD_NOT_VALID {
            wp.unwrap().w_lines_valid.set(0);
        }
        g_st().must_redraw.set(g_st().must_redraw.get().max(type_));
    }
}

pub fn redraw_all_later(type_: i32) {
    for wp in for_all_windows_ptr() {
        redraw_later(Some(&wp), type_);
    }
    set_must_redraw(type_);
}

pub fn set_must_redraw(type_: i32) {
    if !g_st().redraw_not_allowed.get() {
        g_st().must_redraw.set(g_st().must_redraw.get().max(type_));
    }
}

pub fn screen_invalidate_highlights() {
    for wp in for_all_windows_ptr() {
        redraw_later(Some(&wp), UPD_NOT_VALID);
        wp.w_grid_alloc.valid.set(false);
    }
}

pub fn redraw_curbuf_later(type_: i32) {
    redraw_buf_later(&curbuf(), type_);
}

pub fn redraw_buf_later(buf: &FileBuf, type_: i32) {
    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf {
            redraw_later(Some(&wp), type_);
        }
    }
}

pub fn redraw_buf_line_later(buf: &FileBuf, line: Linenr, force: bool) {
    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf {
            redraw_winline(&wp, line.min(buf.b_ml.ml_line_count.get()));
            if force && line > buf.b_ml.ml_line_count.get() {
                wp.w_redraw_bot.set(line);
            }
        }
    }
}

pub fn redraw_win_range_later(wp: &WinInfoInfo, first: Linenr, last: Linenr) {
    if last >= wp.w_topline.get() && first < wp.w_botline.get() {
        if wp.w_redraw_top.get() == 0 || wp.w_redraw_top.get() > first {
            wp.w_redraw_top.set(first);
        }
        if wp.w_redraw_bot.get() == 0 || wp.w_redraw_bot.get() < last {
            wp.w_redraw_bot.set(last);
        }
        redraw_later(Some(wp), UPD_VALID);
    }
}

pub fn redraw_winline(wp: &WinInfoInfo, lnum: Linenr) {
    redraw_win_range_later(wp, lnum, lnum);
}

pub fn redraw_buf_range_later(buf: &FileBuf, first: Linenr, last: Linenr) {
    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf {
            redraw_win_range_later(&wp, first, last);
        }
    }
}

pub fn redraw_buf_status_later(buf: &FileBuf) {
    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf
            && (wp.w_status_height.get() != 0
                || (wp == curwin_ptr() && global_stl_height() != 0)
                || wp.w_winbar_height.get() != 0)
        {
            wp.w_redr_status.set(true);
            set_must_redraw(UPD_VALID);
        }
    }
    if opt_vars().p_ru.get()
        && ptr::eq(curwin().w_buffer.get().as_ptr(), buf)
        && curwin().w_status_height.get() == 0
        && !curwin().w_redr_status.get()
    {
        g_st().redraw_cmdline.set(true);
    }
}

pub fn status_redraw_all() {
    let is_stl_global = global_stl_height() != 0;

    for wp in for_all_windows_ptr() {
        if (!is_stl_global && wp.w_status_height.get() != 0)
            || wp == curwin_ptr()
            || wp.w_winbar_height.get() != 0
        {
            wp.w_redr_status.set(true);
            redraw_later(Some(&wp), UPD_VALID);
        }
    }
}

pub fn status_redraw_curbuf() {
    status_redraw_buf(&curbuf());
}

pub fn status_redraw_buf(buf: &FileBuf) {
    let is_stl_global = global_stl_height() != 0;

    for wp in for_all_windows_ptr() {
        if wp.w_buffer.get() == buf
            && ((!is_stl_global && wp.w_status_height.get() != 0)
                || (is_stl_global && wp == curwin_ptr())
                || wp.w_winbar_height.get() != 0)
        {
            wp.w_redr_status.set(true);
            redraw_later(Some(&wp), UPD_VALID);
        }
    }
    if opt_vars().p_ru.get() && curwin().w_status_height.get() == 0 && !curwin().w_redr_status.get()
    {
        g_st().redraw_cmdline.set(true);
        redraw_later(Some(&crate::globals::curwin()), UPD_VALID);
    }
}

pub fn redraw_statuslines() {
    for wp in for_all_windows_ptr() {
        if wp.w_redr_status.get() {
            win_check_ns_hl(Some(&wp));
            win_redr_winbar(&wp);
            win_redr_status(&wp);
        }
    }

    win_check_ns_hl(None);
    if g_st().redraw_tabline.get() {
        draw_tabline();
    }

    if g_st().need_maketitle.get() {
        maketitle();
    }
}

pub fn win_redraw_last_status(frp: &Frame) {
    if frp.fr_layout.get() == FR_LEAF {
        frp.fr_win.get().w_redr_status.set(true);
    } else if frp.fr_layout.get() == FR_ROW {
        for child in frame_iter_ptr(frp.fr_child.get()) {
            win_redraw_last_status(&child);
        }
    } else {
        assert!(frp.fr_layout.get() == FR_COL);

        let mut cur = frp.child_opt().unwrap();

        while let Some(next) = cur.next_opt() {
            cur = next;
        }
        win_redraw_last_status(&cur);
    }
}

pub fn conceal_cursor_line(wp: &WinInfoInfo) -> bool {
    if wp.w_onebuf_opt.wo_cocu.is_empty() {
        return false;
    }
    let c = if (get_real_state() & MODE_VISUAL) != 0 {
        i32::from(b'v')
    } else if (g_st().state_global.get() & MODE_INSERT) != 0 {
        i32::from(b'i')
    } else if (g_st().state_global.get() & MODE_NORMAL) != 0 {
        i32::from(b'n')
    } else if (g_st().state_global.get() & MODE_CMDLINE) != 0 {
        i32::from(b'c')
    } else {
        return false;
    };
    crate::strings::vim_haschr(&wp.w_onebuf_opt.wo_cocu.as_cstr(), c)
}

/// Redraws lines `line1` to `line2` of `wp` and invalidates their heights, after 'concealcursor'
/// revealed or concealed them again.
pub fn conceal_reveal_changed(wp: &WinInfoInfo, line1: Linenr, line2: Linenr) {
    redraw_win_range_later(wp, line1, line2);
    if has_extconceal(wp) {
        changed_lines_invalidate_win(wp, line1, 0, line2 + 1, 0);
        changed_topline_skipcol(wp, line1, line2);
    }
}

/// Whether 'concealcursor' reveals line `lnum` in window `wp`: the cursor line outside the
/// listed modes, and every line of the Visual area unless "v" is listed. Drawing and height
/// calculations must ask this the same way, or they disagree about how tall a concealed line is.
pub fn conceal_cursor_reveals_line(wp: &WinInfoInfo, lnum: Linenr) -> bool {
    if wp == curwin_ptr() && lnum == wp.w_cursor.get().lnum && !conceal_cursor_line(wp) {
        return true;
    }
    g_st().visual_global.borrow().active
        && ptr::eq(wp.w_buffer.get().as_ptr(), curwin().w_buffer.get().as_ptr())
        && lnum
            >= g_st()
                .visual_global
                .borrow()
                .start
                .lnum
                .min(curwin().w_cursor.get().lnum)
        && lnum
            <= g_st()
                .visual_global
                .borrow()
                .start
                .lnum
                .max(curwin().w_cursor.get().lnum)
        && !crate::strings::vim_haschr(&wp.w_onebuf_opt.wo_cocu.as_cstr(), i32::from(b'v'))
}

pub fn win_cursorline_standout(wp: &WinInfoInfo) -> bool {
    wp.w_onebuf_opt.wo_cul.get()
        || (wp == curwin_ptr() && wp.w_onebuf_opt.wo_cole.get() > 0 && !conceal_cursor_line(wp))
}

pub fn win_update_cursorline(wp: &WinInfoInfo, foldinfo: &mut Foldinfo) {
    let cl = if win_cursorline_standout(wp) {
        wp.w_cursor.get().lnum
    } else {
        0
    };
    wp.w_cursorline.set(cl);
    if wp.w_onebuf_opt.wo_cul.get() {
        *foldinfo = fold_info(wp, wp.w_cursor.get().lnum);
        if foldinfo.fi_level != 0 && foldinfo.fi_lines > 0 {
            wp.w_cursorline.set(foldinfo.fi_lnum);
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
