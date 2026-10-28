//! UI subsystem (`src/nvim/ui.c`).

use crate::api::extmark::describe_ns;
use crate::api::private::helpers::{
    api_clear_error, api_set_error, arena_array, arena_dict, array_obj, boolean_obj,
    cstr_as_string, float_obj, integer_obj, string_obj, window_obj,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::ui::{
    kLineFlagWrap, kUICmdline, kUIExtCount, kUIHlState, kUILinegrid, kUIMessages, kUIMultigrid,
    kUITermColors, remote_ui__set_restart_on_crash_exit, remote_ui_bell, remote_ui_busy_start,
    remote_ui_busy_stop, remote_ui_chdir, remote_ui_default_colors_set, remote_ui_error_exit,
    remote_ui_event, remote_ui_flush, remote_ui_grid_clear, remote_ui_grid_cursor_goto,
    remote_ui_grid_resize, remote_ui_grid_scroll, remote_ui_hl_attr_define, remote_ui_hl_group_set,
    remote_ui_mode_change, remote_ui_mode_info_set, remote_ui_mouse_off, remote_ui_mouse_on,
    remote_ui_msg_set_pos, remote_ui_option_set, remote_ui_raw_line, remote_ui_screenshot,
    remote_ui_set_icon, remote_ui_set_title, remote_ui_suspend, remote_ui_ui_send,
    remote_ui_update_menu, remote_ui_visual_bell, remote_ui_win_viewport,
    remote_ui_win_viewport_margins,
};
use crate::api::win_config::UIExtension;
use crate::autocmd::do_autocmd_uienter;
use crate::buffer::resettitle;
use crate::cursor_shape::{cursor_get_mode_idx, cursorentry_T, mode_style_array, shape_table_mut};
use crate::drawscreen::{DEFAULT_GRID_HANDLE, conceal_check_cursor_line, screen_resize};
use crate::event::r#loop::uv_cwd;
use crate::event::multiqueue::{Event, multiqueue_free, multiqueue_put_event};
use crate::ex_getln::cmdline_ui_flush;
use crate::globals::*;
use crate::grid::{ScreenGrid, get_win_by_grid_handle, sattr_T, schar_T};
use crate::highlight::{highlight_use_hlstate, ui_send_all_hls};
use crate::log::{LOGLVL_ERR, logmsg};
use crate::lua::converter::LuaRef;
use crate::lua::executor::{api_free_luaref, kRetNilBool, nlua_call_ref_ctx};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::message::{
    msg, msg_ext_ui_flush, msg_schedule_semsg, msg_schedule_semsg_multiline, msg_scroll_flush,
    msg_source, msg_ui_refresh,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::option::{kOptCmdheight, set_option_value, ui_refresh_options};
use crate::option_vars::*;
use crate::os::time::{os_hrtime, os_sleep};
use crate::strings::{gettext, vim_strchr};
use crate::ui_compositor::{
    ui_comp_attach, ui_comp_detach, ui_comp_get_grid_at_coord, ui_comp_grid_cursor_goto,
    ui_comp_grid_resize, ui_comp_grid_scroll, ui_comp_init, ui_comp_msg_set_pos, ui_comp_raw_line,
    ui_comp_should_draw,
};
use crate::window::{win_set_inner_size, win_ui_flush};
use crate::winfloat::win_config_float;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

const MAX_UI_COUNT: usize = 16;
const SHAPE_IDX_N: c_int = 0;
const SHAPE_IDX_R: c_int = 3;

pub(crate) const kUIGlobalCount: usize = kUILinegrid;
const kUIFloatDebug: usize = 9;

pub(crate) const kLineFlagInvalid: c_int = 2;

const kOptBoFlagAll: c_uint = 0x01;
const kOptRdbFlagLine: c_uint = 0x10;
const kOptRdbFlagFlush: c_uint = 0x20;

const kMouseNormal: c_int = b'n' as c_int;
const kMouseReturn: c_int = b'r' as c_int;
const kMouseInsert: c_int = b'i' as c_int;
const kMouseCommand: c_int = b'c' as c_int;
const kMouseHelp: c_char = b'h' as c_char;
const MOUSE_A: &CStr = c"nvich";

use crate::change::HLF_W;
use crate::charset::NUL;
use crate::decoration_provider::CB_MAX_ERROR;
#[cfg(debug_assertions)]
use crate::log::LOGLVL_DBG;
use crate::mouse::kMouseVisual;
use crate::msgpack_rpc::channel::RemoteUI;
use crate::os::fs::MAXPATHL;
use crate::state::{
    MODE_ASKMORE, MODE_CMDLINE, MODE_EXTERNCMD, MODE_HITRETURN, MODE_INSERT, MODE_SETWSIZE,
};

#[repr(C)]
struct UIEventCallback {
    cb: LuaRef,
    errors: u8,
    ext_widgets: [bool; kUIGlobalCount],
}

impl Drop for UIEventCallback {
    fn drop(&mut self) {
        api_free_luaref(self.cb);
    }
}

struct UiState {
    uis: Vec<*mut RemoteUI>,
    ui_ext: [bool; kUIExtCount],
    ui_mode_idx: c_int,
    cursor_row: c_int,
    cursor_col: c_int,
    pending_cursor_update: bool,
    busy: c_int,
    pending_mode_info_update: bool,
    pending_mode_update: bool,
    cursor_grid_handle: handle_T,
    ui_event_cbs: std::collections::BTreeMap<u32, UIEventCallback>,
    ui_cb_ext: [bool; kUIExtCount],
    has_mouse: bool,
    pending_has_mouse: c_int,
    pending_default_colors: bool,
    beep_count: c_int,
    beep_start_time: u64,
    ui_flush_was_busy: bool,
    ui_flush_cursor_was_obscured: bool,
    #[cfg(debug_assertions)]
    uilog_seen: usize,
    #[cfg(debug_assertions)]
    uilog_last_event: Option<&'static std::ffi::CStr>,
}

impl UiState {
    const fn new() -> Self {
        Self {
            uis: Vec::new(),
            ui_ext: [false; kUIExtCount],
            ui_mode_idx: SHAPE_IDX_N,
            cursor_row: 0,
            cursor_col: 0,
            pending_cursor_update: false,
            busy: 0,
            pending_mode_info_update: false,
            pending_mode_update: false,
            cursor_grid_handle: DEFAULT_GRID_HANDLE,
            ui_event_cbs: std::collections::BTreeMap::new(),
            ui_cb_ext: [false; kUIExtCount],
            has_mouse: false,
            pending_has_mouse: -1,
            pending_default_colors: false,
            beep_count: 0,
            beep_start_time: 0,
            ui_flush_was_busy: false,
            ui_flush_cursor_was_obscured: false,
            #[cfg(debug_assertions)]
            uilog_seen: 0,
            #[cfg(debug_assertions)]
            uilog_last_event: None,
        }
    }
}

static UI_STATE: GlobalState<UiState> = GlobalState::new(UiState::new());

#[inline]
fn ui_state() -> &'static mut UiState {
    unsafe { &mut *UI_STATE.as_ptr() }
}

pub fn ui_init() {
    g_st().default_grid.handle = 1;
        g_st().msg_grid_adj.target = &raw mut g_st().default_grid;
        ui_comp_init();
}

pub fn ui_free_all_mem() {
    unsafe {
        let cbs_ptr = &raw mut ui_state().ui_event_cbs;
        (*cbs_ptr).clear();
        ui_state().uis = Vec::new();

        multiqueue_free(g_st().resize_events);
    }
}

pub fn ui_rgb_attached() -> bool {
    unsafe {
        if opt_vars().p_tgc != 0 {
            return true;
        }
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            let is_tui = ui.stdin_tty || ui.stdout_tty;
            if !is_tui && ui.rgb {
                return true;
            }
        }
        false
    }
}

pub fn ui_gui_attached() -> bool {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            let is_tui = ui.stdin_tty || ui.stdout_tty;
            if !is_tui {
                return true;
            }
        }
        false
    }
}

pub fn ui_override() -> bool {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            if ui.r#override {
                return true;
            }
        }
        false
    }
}

pub fn ui_active() -> usize {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        (*uis_ptr).len()
    }
}

pub fn ui_refresh() {
    unsafe {
        if g_st().ui_client_channel_id != 0 {
            std::process::abort();
        }

        let mut width = c_int::MAX;
        let mut height = c_int::MAX;
        let active = ui_active() != 0;
        let mut ext_widgets = [active; kUIExtCount];
        let inclusive = ui_override();
        let uis_ptr = &raw const ui_state().uis;

        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            width = width.min(ui.width);
            height = height.min(ui.height);
            for (j, widget) in ext_widgets.iter_mut().enumerate().take(kUIExtCount) {
                *widget &= ui.ui_ext[j] || inclusive;
            }
        }

        ui_state().cursor_row = 0;
        ui_state().cursor_col = 0;
        ui_state().pending_cursor_update = true;

        let ui_ext_ptr = (&raw mut ui_state().ui_ext).cast::<bool>();
        let ui_cb_ext_ptr = (&raw const ui_state().ui_cb_ext).cast::<bool>();
        let had_message = *ui_ext_ptr.add(kUIMessages);

        for (i, &widget) in ext_widgets.iter().enumerate().take(kUIExtCount) {
            let val = widget | *ui_cb_ext_ptr.add(i);
            *ui_ext_ptr.add(i) = val;
            if i < kUIGlobalCount {
                ui_call_option_set(cstr_as_string(ui_ext_names[i].as_ptr()), boolean_obj(val));
            }
        }

        if had_message != *ui_ext_ptr.add(kUIMessages) {
            if g_st().ui_refresh_cmdheight {
                set_option_value(
                    kOptCmdheight,
                    integer_obj(Integer::from(had_message)),
                    0,
                    true,
                    None,
                );
                let mut tp = g_st().first_tabpage;
                while !tp.is_null() {
                    (*tp).tp_ch_used = i64::from(had_message);
                    tp = (*tp).tp_next;
                }
            }
            msg_scroll_flush();
        }
        msg_ui_refresh();

        if !active {
            return;
        }

        if g_st().updating_screen {
            ui_schedule_refresh();
            return;
        }

        ui_default_colors_set();

        let save_p_lz = opt_vars().p_lz;
        opt_vars().p_lz = 0;
        screen_resize(width, height);
        opt_vars().p_lz = save_p_lz;

        ui_mode_info_set();
        ui_state().pending_mode_update = true;
        ui_cursor_shape();
        ui_state().pending_has_mouse = -1;
    }
}

pub fn ui_pum_get_height() -> c_int {
    unsafe {
        let mut pum_height: c_int = 0;
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            let ui_pum_height = ui.pum_nlines;
            if ui_pum_height != 0 {
                pum_height = if pum_height != 0 {
                    pum_height.min(ui_pum_height)
                } else {
                    ui_pum_height
                };
            }
        }
        pum_height
    }
}

pub fn ui_pum_get_pos(
    pwidth: &mut f64,
    pheight: &mut f64,
    prow: &mut f64,
    pcol: &mut f64,
) -> bool {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            if !ui.pum_pos {
                continue;
            }
            *pwidth = ui.pum_width;
            *pheight = ui.pum_height;
            *prow = ui.pum_row;
            *pcol = ui.pum_col;
            return true;
        }
        false
    }
}

pub fn ui_refresh_event() {
    ui_refresh();
}

pub fn ui_schedule_refresh() {
    multiqueue_put_event(g_st().resize_events, Event::UiRefresh);
}

pub fn ui_default_colors_set() {
    unsafe {
        ui_state().pending_default_colors = true;
        if *starting.as_ptr() == 0 {
            ui_may_set_default_colors();
        }
    }
}

fn ui_may_set_default_colors() {
    if ui_state().pending_default_colors {
            ui_state().pending_default_colors = false;
            ui_call_default_colors_set(
                Integer::from(g_st().normal_fg),
                Integer::from(g_st().normal_bg),
                Integer::from(g_st().normal_sp),
                Integer::from(g_st().cterm_normal_fg_color),
                Integer::from(g_st().cterm_normal_bg_color),
            );
        }
}

pub fn ui_busy_start() {
    let prev = ui_state().busy;
        ui_state().busy += 1;
        if prev == 0 {
            ui_call_busy_start();
        }
}

pub fn ui_busy_stop() {
    ui_state().busy -= 1;
        if ui_state().busy == 0 {
            ui_call_busy_stop();
        }
}

pub fn vim_beep(val: c_uint) {
    if g_st().emsg_silent != 0 || g_st().in_assert_fails {
            g_st().did_beep_silent += 1;
            return;
        }
        g_st().did_beep += 1;

        if (opt_vars().bo_flags & val) == 0 && (opt_vars().bo_flags & kOptBoFlagAll) == 0 {
            let now = os_hrtime();
            if ui_state().beep_start_time == 0 || now.wrapping_sub(ui_state().beep_start_time) > 500_000_000 {
                ui_state().beep_count = 0;
                ui_state().beep_start_time = now;
            }
            ui_state().beep_count += 1;
            if ui_state().beep_count <= 3 {
                if opt_vars().p_vb != 0 {
                    ui_call_visual_bell();
                } else {
                    ui_call_bell();
                }
            }
        }

        if !vim_strchr(opt_vars().p_debug, b'e' as c_int).is_null() {
            msg_source(HLF_W);
            msg(gettext(c"Beep!".as_ptr()), HLF_W);
        }
}

pub fn do_autocmd_uienter_all() {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            do_autocmd_uienter(ui.channel_id, true);
        }
    }
}

pub fn ui_can_attach_more() -> bool {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        (*uis_ptr).len() < MAX_UI_COUNT
    }
}

pub fn ui_attach_impl(ui: &mut RemoteUI, chanid: u64) {
    unsafe {
        let uis_ptr = &raw mut ui_state().uis;
        if (*uis_ptr).len() >= MAX_UI_COUNT {
            std::process::abort();
        }
        if !ui.ui_ext[kUIMultigrid] && !ui.ui_ext[kUIFloatDebug] && g_st().ui_client_channel_id == 0
        {
            ui_comp_attach(ui);
        }

        (*uis_ptr).push(&raw mut *ui);
        ui_refresh_options();
        resettitle();

        let mut cwd: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut cwdlen = MAXPATHL;
        if uv_cwd(cwd.as_mut_ptr(), &mut cwdlen) == 0 {
            ui_call_chdir(NvimString {
                data: cwd.as_mut_ptr(),
                size: cwdlen,
            });
        }

        for i in kUIGlobalCount..kUIExtCount {
            ui_set_ext_option(ui, i as UIExtension, ui.ui_ext[i]);
        }

        let mut sent = false;
        if ui.ui_ext[kUIHlState] {
            sent = highlight_use_hlstate();
        }
        if !sent {
            ui_send_all_hls(ui);
        }
        ui_refresh();

        do_autocmd_uienter(chanid, true);
    }
}

pub fn ui_detach_impl(ui: &mut RemoteUI, chanid: u64) {
    unsafe {
        let uis_ptr = &raw mut ui_state().uis;
        if (*uis_ptr).len() > MAX_UI_COUNT {
            std::process::abort();
        }
        let Some(shift_index) = (*uis_ptr).iter().position(|&u| u == ui) else {
            std::process::abort();
        };
        (*uis_ptr).remove(shift_index);

        if !(*uis_ptr).is_empty() && !g_st().exiting {
            ui_schedule_refresh();
        }

        if !ui.ui_ext[kUIMultigrid] && !ui.ui_ext[kUIFloatDebug] {
            ui_comp_detach(ui);
        }

        do_autocmd_uienter(chanid, false);
    }
}

pub fn ui_set_ext_option(ui: &mut RemoteUI, ext: UIExtension, active: bool) {
    unsafe {
        let idx = ext as usize;
        if idx < kUIGlobalCount {
            ui_refresh();
            return;
        }
        let name = ui_ext_names[idx].as_ptr();
        if *name != b'_' as c_char || active {
            remote_ui_option_set(ui, cstr_as_string(name), boolean_obj(active));
        }
        if idx == kUITermColors {
            ui_default_colors_set();
        }
    }
}

pub fn ui_line(
    grid: &mut ScreenGrid,
    row: c_int,
    invalid_row: bool,
    startcol: c_int,
    endcol: c_int,
    clearcol: c_int,
    clearattr: c_int,
    wrap: bool,
) {
    unsafe {
        debug_assert!(0 <= row && row < grid.rows);
        let mut flags = if wrap { kLineFlagWrap } else { 0 };
        if startcol == 0 && invalid_row {
            flags |= kLineFlagInvalid;
        }

        ui_may_set_default_colors();

        let grid = &mut *grid;
        let off = grid.line_offset[row as usize] + startcol as usize;
        ui_call_raw_line(
            Integer::from(grid.handle),
            Integer::from(row),
            Integer::from(startcol),
            Integer::from(endcol),
            Integer::from(clearcol),
            Integer::from(clearattr),
            flags,
            grid.chars.as_mut_ptr().add(off),
            grid.attrs.as_mut_ptr().add(off),
        );

        if opt_vars().p_wd != 0 && (opt_vars().rdb_flags & kOptRdbFlagLine) != 0 {
            ui_call_grid_cursor_goto(
                Integer::from(grid.handle),
                Integer::from(row),
                Integer::from(clearcol.min(grid.cols - 1)),
            );
            ui_call_flush();
            os_sleep(opt_vars().p_wd.unsigned_abs());
            ui_state().pending_cursor_update = true;
        }
    }
}

pub fn ui_cursor_goto(new_row: c_int, new_col: c_int) {
    ui_grid_cursor_goto(DEFAULT_GRID_HANDLE, new_row, new_col);
}

pub fn ui_grid_cursor_goto(grid_handle: handle_T, new_row: c_int, new_col: c_int) {
    if new_row == ui_state().cursor_row && new_col == ui_state().cursor_col && grid_handle == ui_state().cursor_grid_handle {
            return;
        }
        ui_state().cursor_row = new_row;
        ui_state().cursor_col = new_col;
        ui_state().cursor_grid_handle = grid_handle;
        ui_state().pending_cursor_update = true;
}

pub fn ui_check_cursor_grid(grid_handle: handle_T) {
    if ui_state().cursor_grid_handle == grid_handle {
            ui_state().pending_cursor_update = true;
        }
}

pub fn ui_mode_info_set() {
    ui_state().pending_mode_info_update = true;
}

pub fn ui_current_row() -> c_int {
    ui_state().cursor_row
}

pub fn ui_current_col() -> c_int {
    ui_state().cursor_col
}

pub fn ui_flush() {
    unsafe {
        debug_assert!(g_st().ui_client_channel_id == 0);
        if ui_active() == 0 {
            return;
        }

        if (g_st().State & MODE_CMDLINE) == 0 && (**curwin.as_ptr()).w_floating && (**curwin.as_ptr()).w_config.hide {
            if !ui_state().ui_flush_was_busy {
                ui_call_busy_start();
                ui_state().ui_flush_was_busy = true;
            }
        } else if ui_state().ui_flush_was_busy {
            ui_call_busy_stop();
            ui_state().ui_flush_was_busy = false;
        }

        win_ui_flush(false);
        if g_st().textlock == 0 && g_st().expr_map_lock == 0 {
            cmdline_ui_flush();
            msg_ext_ui_flush();
        }
        msg_scroll_flush();

        if ui_state().pending_cursor_update {
            ui_call_grid_cursor_goto(
                Integer::from(ui_state().cursor_grid_handle),
                Integer::from(ui_state().cursor_row),
                Integer::from(ui_state().cursor_col),
            );
            ui_state().pending_cursor_update = false;
            win_ui_flush(false);
        }
        if ui_state().pending_mode_info_update {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let style = mode_style_array(&mut arena);
            let enabled = *opt_vars().p_guicursor != NUL;
            ui_call_mode_info_set(enabled, style);
            arena_mem_free(arena_finish(&mut arena));
            ui_state().pending_mode_info_update = false;
        }

        let cursor_obscured = ui_cursor_is_behind_floatwin();
        if (cursor_obscured != ui_state().ui_flush_cursor_was_obscured || ui_state().pending_mode_update) && *starting.as_ptr() == 0
        {
            let idx = if cursor_obscured {
                SHAPE_IDX_R
            } else {
                ui_state().ui_mode_idx
            };
            let full_name = (*(&raw const (*shape_table_mut()))
                .cast::<cursorentry_T>()
                .add(idx as usize))
            .full_name;
            ui_call_mode_change(cstr_as_string(full_name.as_ptr()), Integer::from(idx));
            ui_state().pending_mode_update = false;
            ui_state().ui_flush_cursor_was_obscured = cursor_obscured;
        }

        if ui_state().pending_has_mouse != c_int::from(ui_state().has_mouse) {
            if ui_state().has_mouse {
                ui_call_mouse_on();
            } else {
                ui_call_mouse_off();
            }
            ui_state().pending_has_mouse = c_int::from(ui_state().has_mouse);
        }
        ui_call_flush();

        if opt_vars().p_wd != 0 && (opt_vars().rdb_flags & kOptRdbFlagFlush) != 0 {
            os_sleep(opt_vars().p_wd.unsigned_abs());
        }
    }
}

pub fn ui_check_mouse() {
    unsafe {
        ui_state().has_mouse = false;
        if *opt_vars().p_mouse == NUL {
            return;
        }

        let checkfor = if g_st().Visual.active {
            kMouseVisual
        } else if g_st().State == MODE_HITRETURN || g_st().State == MODE_ASKMORE || g_st().State == MODE_SETWSIZE {
            kMouseReturn
        } else if (g_st().State & MODE_INSERT) != 0 {
            kMouseInsert
        } else if (g_st().State & MODE_CMDLINE) != 0 {
            kMouseCommand
        } else if g_st().State == MODE_EXTERNCMD {
            b' ' as c_int
        } else {
            kMouseNormal
        };

        if ui_mouse_has(checkfor) {
            ui_state().has_mouse = true;
        }
    }
}

pub fn ui_mouse_has(mode: c_int) -> bool {
    unsafe {
        let mut p = opt_vars().p_mouse;
        while *p != NUL {
            if *p == b'a' as c_char {
                if !vim_strchr(MOUSE_A.as_ptr(), mode).is_null() {
                    return true;
                }
            } else if *p == kMouseHelp {
                if mode != kMouseReturn && (*g_st().curbuf).b_help {
                    return true;
                }
            } else if mode == *p as c_int {
                return true;
            }
            p = p.add(1);
        }
        false
    }
}

pub fn ui_cursor_shape_no_check_conceal() {
    if !g_st().full_screen {
            return;
        }
        let new_mode_idx = cursor_get_mode_idx();
        if new_mode_idx != ui_state().ui_mode_idx {
            ui_state().ui_mode_idx = new_mode_idx;
            ui_state().pending_mode_update = true;
        }
}

pub fn ui_cursor_shape() {
    ui_cursor_shape_no_check_conceal();
        conceal_check_cursor_line();
}

fn ui_cursor_is_behind_floatwin() -> bool {
    unsafe {
        if (g_st().State & MODE_CMDLINE) != 0 || !ui_comp_should_draw() {
            return false;
        }

        let crow = (**curwin.as_ptr()).w_winrow + (**curwin.as_ptr()).w_winrow_off + (**curwin.as_ptr()).w_wrow;
        let ccol = (**curwin.as_ptr()).w_wincol
            + (**curwin.as_ptr()).w_wincol_off
            + if (**curwin.as_ptr()).w_onebuf_opt.wo_rl != 0 {
                (**curwin.as_ptr()).w_view_width - (**curwin.as_ptr()).w_wcol - 1
            } else {
                (**curwin.as_ptr()).w_wcol
            };

        let top_grid = ui_comp_get_grid_at_coord(crow, ccol);
        top_grid != &raw mut (**curwin.as_ptr()).w_grid_alloc
            && top_grid != &raw mut g_st().default_grid
            && (*top_grid).zindex >= (**curwin.as_ptr()).w_grid_alloc.zindex + 50
    }
}

pub fn ui_has(ext: UIExtension) -> bool {
    ui_state().ui_ext[ext as usize]
}

pub fn ui_array(arena: &mut Arena) -> Array {
    unsafe {
        let uis_ptr = &raw const ui_state().uis;
        let mut all_uis = arena_array(arena, (*uis_ptr).len());

        for &ui_ptr in &*uis_ptr {
            let ui = &mut *ui_ptr;
            let mut info = arena_dict(arena, 10 + kUIExtCount);
            let mut put_info = |k: *const c_char, v: Object| {
                *info.items.add(info.size) = KeyValuePair {
                    key: cstr_as_string(k),
                    value: v,
                };
                info.size += 1;
            };

            put_info(c"width".as_ptr(), integer_obj(Integer::from(ui.width)));
            put_info(c"height".as_ptr(), integer_obj(Integer::from(ui.height)));
            put_info(c"rgb".as_ptr(), boolean_obj(ui.rgb));
            put_info(c"override".as_ptr(), boolean_obj(ui.r#override));
            put_info(
                c"term_name".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(cstr_as_string(
                            ui.term_name.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        )),
                },
            );
            put_info(
                c"term_background".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(NvimString {
                            data: c"".as_ptr().cast_mut(),
                            size: 0,
                        }),
                },
            );
            put_info(
                c"term_colors".as_ptr(),
                integer_obj(Integer::from(ui.term_colors)),
            );
            put_info(c"stdin_tty".as_ptr(), boolean_obj(ui.stdin_tty));
            put_info(c"stdout_tty".as_ptr(), boolean_obj(ui.stdout_tty));

            for (j, ext_name) in ui_ext_names.iter().enumerate().take(kUIExtCount) {
                let ename = ext_name.as_ptr();
                if *ename != b'_' as c_char || ui.ui_ext[j] {
                    put_info(ename, boolean_obj(ui.ui_ext[j]));
                }
            }
            put_info(c"chan".as_ptr(), integer_obj(ui.channel_id as Integer));

            *all_uis.items.add(all_uis.size) = Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData::Dict(info),
            };
            all_uis.size += 1;
        }
        all_uis
    }
}

pub fn ui_grid_resize(grid_handle: handle_T, width: c_int, height: c_int, err: &mut Error) {
    unsafe {
        if grid_handle == DEFAULT_GRID_HANDLE {
            screen_resize(width, height);
            return;
        }

        let wp = get_win_by_grid_handle(grid_handle);
        if wp.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid window handle: %lld".as_ptr(),
                i64::from(grid_handle),
            );
            return;
        }

        if (*wp).w_floating {
            if width != (*wp).w_width || height != (*wp).w_height {
                (*wp).w_config.width = width.max(1);
                (*wp).w_config.height = height.max(1);
                win_config_float(&mut *wp, (*wp).w_config.clone());
            }
        } else {
            (*wp).w_height_request = height.max(0);
            (*wp).w_width_request = width.max(0);
            win_set_inner_size(&mut *wp, true);
        }
    }
}

fn ui_attach_error(ns_id: u32, name: *const c_char, msg: *const c_char) {
    let ns = describe_ns(ns_id as c_int, c"(UNKNOWN PLUGIN)".as_ptr());
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            c"ui_attach_error".as_ptr(),
            785,
            true,
            c"Error in \"%s\" UI event handler (ns=%s):\n%s".as_ptr(),
            name,
            ns,
            msg,
        );
        msg_schedule_semsg_multiline!(
            c"Error in \"%s\" UI event handler (ns=%s):\n%s".as_ptr(),
            name,
            ns,
            msg,
        );
}

pub fn ui_call_event(name: &'static std::ffi::CStr, args: Array) {
    unsafe {
        let save_expr_map_lock = g_st().expr_map_lock;
        let save_textlock = g_st().textlock;
        g_st().expr_map_lock = 0;
        g_st().textlock = 0;

        let mut handled = false;
        let fast = g_st().msg_ext_fast && name == c"msg_show";
        let cbs_ptr = &raw const ui_state().ui_event_cbs;
        let entries: Vec<(u32, LuaRef)> = (*cbs_ptr).iter().map(|(&k, v)| (k, v.cb)).collect();
        for (ns_id, cb_ref) in entries {
            if !(*cbs_ptr).contains_key(&ns_id) {
                continue;
            }
            g_st().ui_event_ns_id = ns_id;
            let mut err = Error::new();
            let res = nlua_call_ref_ctx(
                fast,
                cb_ref,
                name.as_ptr(),
                args,
                kRetNilBool,
                None,
                Some(&mut err),
            );
            g_st().ui_event_ns_id = 0;
            if res.obj_type == ObjectType::kObjectTypeBoolean && res.data.boolean() {
                handled = true;
            }
            if !matches!(err.err_type, ErrorType::kErrorTypeNone) {
                ui_attach_error(ns_id, name.as_ptr(), err.msg_ptr());
                ui_remove_cb(ns_id, true);
            }
            api_clear_error(&mut err);
        }
        g_st().expr_map_lock = save_expr_map_lock;
        g_st().textlock = save_textlock;

        if !handled {
            let uis_ptr = &raw const ui_state().uis;
            for &ui_ptr in &*uis_ptr {
                remote_ui_event(&mut *ui_ptr, name, args);
            }
        }
    }
}

fn ui_cb_update_ext() {
    unsafe {
        ui_state().ui_cb_ext = [false; kUIExtCount];
        let ui_cb_ext_ptr = (&raw mut ui_state().ui_cb_ext).cast::<bool>();

        let cbs_ptr = &raw const ui_state().ui_event_cbs;
        for i in 0..kUIGlobalCount {
            for event_cb in (*cbs_ptr).values() {
                if event_cb.ext_widgets[i] {
                    *ui_cb_ext_ptr.add(i) = true;
                    break;
                }
            }
        }
    }
}

pub fn ui_add_cb(ns_id: u32, cb: LuaRef, ext_widgets: &[bool; kUIGlobalCount]) {
    unsafe {
        let mut event_cb = UIEventCallback {
            cb,
            errors: 0,
            ext_widgets: *ext_widgets,
        };
        if event_cb.ext_widgets[kUIMessages] {
            event_cb.ext_widgets[kUICmdline] = true;
        }

        let cbs_ptr = &raw mut ui_state().ui_event_cbs;
        (*cbs_ptr).insert(ns_id, event_cb);

        ui_cb_update_ext();
        ui_refresh();
    }
}

pub fn ui_remove_cb(ns_id: u32, checkerr: bool) {
    unsafe {
        let cbs_ptr = &raw mut ui_state().ui_event_cbs;
        let should_remove = if let Some(item) = (*cbs_ptr).get_mut(&ns_id) {
            if !checkerr {
                true
            } else {
                item.errors += 1;
                item.errors > CB_MAX_ERROR
            }
        } else {
            false
        };
        if should_remove {
            (*cbs_ptr).remove(&ns_id);
            ui_cb_update_ext();
            ui_refresh();
            if checkerr {
                let ns = describe_ns(ns_id as c_int, c"(UNKNOWN PLUGIN)".as_ptr());
                msg_schedule_semsg!(
                    c"Excessive errors in vim.ui_attach() callback (ns=%s)".as_ptr(),
                    ns,
                );
            }
        }
    }
}


pub fn ui_log(funname: &'static std::ffi::CStr) {
    #[cfg(not(debug_assertions))]
    let _ = funname;
    #[cfg(debug_assertions)]
    {
        if crate::memory::mem_st().entered_free_all_mem {
            return;
        }

        if ui_state().uilog_last_event == Some(funname) {
            ui_state().uilog_seen += 1;
        } else {
            if ui_state().uilog_seen > 0 {
                logmsg!(
                    LOGLVL_DBG,
                    c"UI: ".as_ptr(),
                    ptr::null(),
                    -1,
                    true,
                    c"%s (+%zu times...)".as_ptr(),
                    ui_state().uilog_last_event.map_or(ptr::null(), |s| s.as_ptr()),
                    ui_state().uilog_seen,
                );
            }
            logmsg!(
                LOGLVL_DBG,
                c"UI: ".as_ptr(),
                ptr::null(),
                -1,
                true,
                c"%s".as_ptr(),
                funname.as_ptr(),
            );
            ui_state().uilog_seen = 0;
            ui_state().uilog_last_event = Some(funname);
        }
    }
}

const NOARGS_ARRAY: Array = Array {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

#[inline]
const fn buf_obj(b: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBuffer,
        data: ObjectData::Integer(b as Integer),
    }
}

#[inline]
const fn tab_obj(t: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeTabpage,
        data: ObjectData::Integer(t as Integer),
    }
}

macro_rules! ui_call_loop {
    ($cond:expr, $funname:literal, |$ui:ident| $call:expr) => {
        unsafe {
            let mut any_call = false;
            let uis_ptr = &raw const ui_state().uis;
            for &_ui_ptr in &*uis_ptr {
                let $ui = &mut *_ui_ptr;
                if $cond {
                    $call;
                    any_call = true;
                }
            }
            if any_call {
                ui_log($funname);
            }
        }
    };
}

macro_rules! ui_call_event_guarded {
    ($event:literal) => {{
        static ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !ENTERED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            ui_call_event($event, NOARGS_ARRAY);
            ENTERED.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }};
    ($event:literal, [$($item:expr),+ $(,)?]) => {{
        static ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !ENTERED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let mut items = [$($item),+];
            let len = items.len();
            let args = Array {
                items: items.as_mut_ptr(),
                size: len,
                capacity: len,
            };
            ui_call_event($event, args);
            ENTERED.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }};
}

pub fn ui_call_mode_info_set(enabled: bool, cursor_styles: Array) {
    ui_call_loop!(true, c"mode_info_set", |ui| {
        remote_ui_mode_info_set(ui, enabled, cursor_styles)
    });
}

pub fn ui_call_update_menu() {
    ui_call_loop!(true, c"update_menu", |ui| remote_ui_update_menu(ui));
}

pub fn ui_call_busy_start() {
    ui_call_loop!(true, c"busy_start", |ui| remote_ui_busy_start(ui));
}

pub fn ui_call_busy_stop() {
    ui_call_loop!(true, c"busy_stop", |ui| remote_ui_busy_stop(ui));
}

pub fn ui_call_mouse_on() {
    ui_call_loop!(true, c"mouse_on", |ui| remote_ui_mouse_on(ui));
}

pub fn ui_call_mouse_off() {
    ui_call_loop!(true, c"mouse_off", |ui| remote_ui_mouse_off(ui));
}

pub fn ui_call_mode_change(mode: NvimString, mode_idx: Integer) {
    ui_call_loop!(true, c"mode_change", |ui| {
        remote_ui_mode_change(ui, mode, mode_idx)
    });
}

pub fn ui_call_bell() {
    ui_call_loop!(true, c"bell", |ui| remote_ui_bell(ui));
}

pub fn ui_call_visual_bell() {
    ui_call_loop!(true, c"visual_bell", |ui| remote_ui_visual_bell(ui));
}

pub fn ui_call_flush() {
    ui_call_loop!(true, c"flush", |ui| remote_ui_flush(ui));
}

pub fn ui_call_suspend() {
    ui_call_loop!(true, c"suspend", |ui| remote_ui_suspend(ui));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_call_set_title(title: NvimString) {
    ui_call_loop!(true, c"set_title", |ui| remote_ui_set_title(ui, title));
}

pub fn ui_call_set_icon(icon: NvimString) {
    ui_call_loop!(true, c"set_icon", |ui| remote_ui_set_icon(ui, icon));
}

pub fn ui_call_screenshot(path: NvimString) {
    ui_call_loop!(true, c"screenshot", |ui| remote_ui_screenshot(ui, path));
}

pub fn ui_call_option_set(name: NvimString, value: Object) {
    ui_call_loop!(true, c"option_set", |ui| {
        remote_ui_option_set(ui, name, value)
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_call_chdir(path: NvimString) {
    ui_call_loop!(true, c"chdir", |ui| remote_ui_chdir(ui, path));
}

pub fn ui_call_ui_send(content: NvimString) {
    ui_call_loop!(true, c"ui_send", |ui| remote_ui_ui_send(ui, content));
}

pub fn ui_call_default_colors_set(
    rgb_fg: Integer,
    rgb_bg: Integer,
    rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    ui_call_loop!(true, c"default_colors_set", |ui| {
        remote_ui_default_colors_set(ui, rgb_fg, rgb_bg, rgb_sp, cterm_fg, cterm_bg)
    });
}

pub fn ui_call_hl_attr_define(
    id: Integer,
    rgb_attrs: crate::highlight::HlAttrs,
    cterm_attrs: crate::highlight::HlAttrs,
    info: Array,
) {
    ui_call_loop!(true, c"hl_attr_define", |ui| {
        remote_ui_hl_attr_define(ui, id, rgb_attrs, cterm_attrs, info)
    });
}

pub fn ui_call_hl_group_set(name: NvimString, id: Integer) {
    ui_call_loop!(true, c"hl_group_set", |ui| {
        remote_ui_hl_group_set(ui, name, id)
    });
}

pub fn ui_call_grid_resize(grid: Integer, width: Integer, height: Integer) {
    ui_comp_grid_resize(grid, width, height);
    ui_call_loop!(!ui.composed, c"grid_resize", |ui| {
        remote_ui_grid_resize(ui, grid, width, height)
    });
}

pub fn ui_composed_call_grid_resize(grid: Integer, width: Integer, height: Integer) {
    ui_call_loop!(ui.composed, c"grid_resize", |ui| {
        remote_ui_grid_resize(ui, grid, width, height)
    });
}

pub fn ui_call_grid_clear(grid: Integer) {
    ui_call_loop!(true, c"grid_clear", |ui| remote_ui_grid_clear(ui, grid));
}

pub fn ui_call_grid_cursor_goto(grid: Integer, row: Integer, col: Integer) {
    ui_comp_grid_cursor_goto(grid, row, col);
    ui_call_loop!(!ui.composed, c"grid_cursor_goto", |ui| {
        remote_ui_grid_cursor_goto(ui, grid, row, col)
    });
}

pub fn ui_composed_call_grid_cursor_goto(grid: Integer, row: Integer, col: Integer) {
    ui_call_loop!(ui.composed, c"grid_cursor_goto", |ui| {
        remote_ui_grid_cursor_goto(ui, grid, row, col)
    });
}

pub fn ui_call_grid_scroll(
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    ui_comp_grid_scroll(grid, top, bot, left, right, rows, cols);
    ui_call_loop!(!ui.composed, c"grid_scroll", |ui| {
        remote_ui_grid_scroll(ui, grid, top, bot, left, right, rows, cols)
    });
}

pub fn ui_composed_call_grid_scroll(
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    ui_call_loop!(ui.composed, c"grid_scroll", |ui| {
        remote_ui_grid_scroll(ui, grid, top, bot, left, right, rows, cols)
    });
}

pub fn ui_call_grid_destroy(grid: Integer) {
    ui_call_event_guarded!(c"grid_destroy", [integer_obj(grid)]);
}

pub fn ui_call_raw_line(
    grid: Integer,
    row: Integer,
    startcol: Integer,
    endcol: Integer,
    clearcol: Integer,
    clearattr: Integer,
    flags: c_int,
    chunk: *const schar_T,
    attrs: *const sattr_T,
) {
    ui_comp_raw_line(
            grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        );
    ui_call_loop!(!ui.composed, c"raw_line", |ui| {
        remote_ui_raw_line(
            ui, grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        )
    });
}

pub fn ui_composed_call_raw_line(
    grid: Integer,
    row: Integer,
    startcol: Integer,
    endcol: Integer,
    clearcol: Integer,
    clearattr: Integer,
    flags: c_int,
    chunk: *const schar_T,
    attrs: *const sattr_T,
) {
    ui_call_loop!(ui.composed, c"raw_line", |ui| {
        remote_ui_raw_line(
            ui, grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        )
    });
}

pub fn ui_call_win_pos(
    grid: Integer,
    win: handle_T,
    startrow: Integer,
    startcol: Integer,
    width: Integer,
    height: Integer,
) {
    ui_call_event_guarded!(
        c"win_pos",
        [
            integer_obj(grid),
            window_obj(win),
            integer_obj(startrow),
            integer_obj(startcol),
            integer_obj(width),
            integer_obj(height),
        ]
    );
}

pub fn ui_call_win_float_pos(
    grid: Integer,
    win: handle_T,
    anchor: NvimString,
    anchor_grid: Integer,
    anchor_row: f64,
    anchor_col: f64,
    mouse_enabled: bool,
    zindex: Integer,
    compindex: Integer,
    screen_row: Integer,
    screen_col: Integer,
) {
    ui_call_event_guarded!(
        c"win_float_pos",
        [
            integer_obj(grid),
            window_obj(win),
            string_obj(anchor),
            integer_obj(anchor_grid),
            float_obj(anchor_row),
            float_obj(anchor_col),
            boolean_obj(mouse_enabled),
            integer_obj(zindex),
            integer_obj(compindex),
            integer_obj(screen_row),
            integer_obj(screen_col),
        ]
    );
}

pub fn ui_call_win_external_pos(grid: Integer, win: handle_T) {
    ui_call_event_guarded!(c"win_external_pos", [integer_obj(grid), window_obj(win)]);
}

pub fn ui_call_win_hide(grid: Integer) {
    ui_call_event_guarded!(c"win_hide", [integer_obj(grid)]);
}

pub fn ui_call_win_close(grid: Integer) {
    ui_call_event_guarded!(c"win_close", [integer_obj(grid)]);
}

pub fn ui_call_msg_set_pos(
    grid: Integer,
    row: Integer,
    scrolled: bool,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    ui_comp_msg_set_pos(grid, row, scrolled, sep_char, zindex, compindex);
    ui_call_loop!(!ui.composed, c"msg_set_pos", |ui| {
        remote_ui_msg_set_pos(ui, grid, row, scrolled, sep_char, zindex, compindex)
    });
}

pub fn ui_call_win_viewport(
    grid: Integer,
    win: handle_T,
    topline: Integer,
    botline: Integer,
    curline: Integer,
    curcol: Integer,
    line_count: Integer,
    scroll_delta: Integer,
) {
    ui_call_loop!(true, c"win_viewport", |ui| {
        remote_ui_win_viewport(
            ui,
            grid,
            win,
            topline,
            botline,
            curline,
            curcol,
            line_count,
            scroll_delta,
        )
    });
}

pub fn ui_call_win_viewport_margins(
    grid: Integer,
    win: handle_T,
    top: Integer,
    bottom: Integer,
    left: Integer,
    right: Integer,
) {
    ui_call_loop!(true, c"win_viewport_margins", |ui| {
        remote_ui_win_viewport_margins(ui, grid, win, top, bottom, left, right)
    });
}

pub fn ui_call_win_extmark(
    grid: Integer,
    win: handle_T,
    ns_id: Integer,
    mark_id: Integer,
    row: Integer,
    col: Integer,
) {
    ui_call_event_guarded!(
        c"win_extmark",
        [
            integer_obj(grid),
            window_obj(win),
            integer_obj(ns_id),
            integer_obj(mark_id),
            integer_obj(row),
            integer_obj(col),
        ]
    );
}

pub fn ui_call_popupmenu_show(
    items_arr: Array,
    selected: Integer,
    row: Integer,
    col: Integer,
    grid: Integer,
) {
    ui_call_event_guarded!(
        c"popupmenu_show",
        [
            array_obj(items_arr),
            integer_obj(selected),
            integer_obj(row),
            integer_obj(col),
            integer_obj(grid),
        ]
    );
}

pub fn ui_call_popupmenu_hide() {
    ui_call_event_guarded!(c"popupmenu_hide");
}

pub fn ui_call_popupmenu_select(selected: Integer) {
    ui_call_event_guarded!(c"popupmenu_select", [integer_obj(selected)]);
}

pub fn ui_call_tabline_update(
    current: handle_T,
    tabs: Array,
    current_buffer: handle_T,
    buffers: Array,
) {
    ui_call_event_guarded!(
        c"tabline_update",
        [
            tab_obj(current),
            array_obj(tabs),
            buf_obj(current_buffer),
            array_obj(buffers),
        ]
    );
}

pub fn ui_call_cmdline_show(
    content: Array,
    pos: Integer,
    firstc: NvimString,
    prompt: NvimString,
    indent: Integer,
    level: Integer,
    hl_id: Integer,
) {
    ui_call_event_guarded!(
        c"cmdline_show",
        [
            array_obj(content),
            integer_obj(pos),
            string_obj(firstc),
            string_obj(prompt),
            integer_obj(indent),
            integer_obj(level),
            integer_obj(hl_id),
        ]
    );
}

pub fn ui_call_cmdline_pos(pos: Integer, level: Integer) {
    ui_call_event_guarded!(c"cmdline_pos", [integer_obj(pos), integer_obj(level)]);
}

pub fn ui_call_cmdline_special_char(c: NvimString, shift: bool, level: Integer) {
    ui_call_event_guarded!(
        c"cmdline_special_char",
        [string_obj(c), boolean_obj(shift), integer_obj(level)]
    );
}

pub fn ui_call_cmdline_hide(level: Integer, abort: bool) {
    ui_call_event_guarded!(c"cmdline_hide", [integer_obj(level), boolean_obj(abort)]);
}

pub fn ui_call_cmdline_block_show(lines: Array) {
    ui_call_event_guarded!(c"cmdline_block_show", [array_obj(lines)]);
}

pub fn ui_call_cmdline_block_append(lines: Array) {
    ui_call_event_guarded!(c"cmdline_block_append", [array_obj(lines)]);
}

pub fn ui_call_cmdline_block_hide() {
    ui_call_event_guarded!(c"cmdline_block_hide");
}

pub fn ui_call_msg_show(
    kind: NvimString,
    content: Array,
    replace_last: bool,
    history: bool,
    append: bool,
    id: Object,
    trigger: NvimString,
) {
    ui_call_event_guarded!(
        c"msg_show",
        [
            string_obj(kind),
            array_obj(content),
            boolean_obj(replace_last),
            boolean_obj(history),
            boolean_obj(append),
            id,
            string_obj(trigger),
        ]
    );
}

pub fn ui_call_msg_clear() {
    ui_call_event_guarded!(c"msg_clear");
}

pub fn ui_call_msg_showcmd(content: Array) {
    ui_call_event_guarded!(c"msg_showcmd", [array_obj(content)]);
}

pub fn ui_call_msg_showmode(content: Array) {
    ui_call_event_guarded!(c"msg_showmode", [array_obj(content)]);
}

pub fn ui_call_msg_ruler(content: Array) {
    ui_call_event_guarded!(c"msg_ruler", [array_obj(content)]);
}

pub fn ui_call_msg_history_show(entries: Array, prev_cmd: bool) {
    ui_call_event_guarded!(
        c"msg_history_show",
        [array_obj(entries), boolean_obj(prev_cmd)]
    );
}

pub fn ui_call_error_exit(status: Integer) {
    ui_call_loop!(true, c"error_exit", |ui| remote_ui_error_exit(ui, status));
}

pub fn ui_call__set_restart_on_crash_exit(progpath: NvimString, argv: Array) {
    ui_call_loop!(true, c"_set_restart_on_crash_exit", |ui| {
        remote_ui__set_restart_on_crash_exit(ui, progpath, argv)
    });
}
