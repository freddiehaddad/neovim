//! UI subsystem (`src/nvim/ui.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::ui::{
    remote_ui__set_restart_on_crash_exit, remote_ui_bell, remote_ui_busy_start,
    remote_ui_busy_stop, remote_ui_chdir, remote_ui_default_colors_set, remote_ui_error_exit,
    remote_ui_event, remote_ui_flush, remote_ui_grid_clear, remote_ui_grid_cursor_goto,
    remote_ui_grid_resize, remote_ui_grid_scroll, remote_ui_hl_group_set, remote_ui_mode_change,
    remote_ui_mouse_off, remote_ui_mouse_on, remote_ui_option_set, remote_ui_raw_line,
    remote_ui_screenshot, remote_ui_set_icon, remote_ui_set_title, remote_ui_suspend,
    remote_ui_ui_send, remote_ui_update_menu, remote_ui_visual_bell, remote_ui_win_viewport,
    remote_ui_win_viewport_margins,
};
#[allow(unused_imports)]
use crate::api::ui::{remote_ui_hl_attr_define, remote_ui_mode_info_set, remote_ui_msg_set_pos};
#[allow(unused_imports)]
use crate::buffer::resettitle;
use crate::cursor_shape::shape_table;
use crate::globals::*;
use crate::log::logmsg;
#[allow(unused_imports)]
use crate::lua::executor::api_free_luaref;
#[allow(unused_imports)]
use crate::lua::executor::nlua_call_ref_ctx;
#[allow(unused_imports)]
use crate::message::{msg, msg_ext_ui_flush, msg_scroll_flush, msg_source, msg_ui_refresh};
use crate::message::{msg_schedule_semsg, msg_schedule_semsg_multiline};
#[allow(unused_imports)]
use crate::option::set_option_value;
#[allow(unused_imports)]
use crate::option::ui_refresh_options;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::strings::vim_strchr;
#[allow(unused_imports)]
use crate::ui_compositor::ui_comp_msg_set_pos;
#[allow(unused_imports)]
use crate::ui_compositor::ui_comp_raw_line;
#[allow(unused_imports)]
use crate::ui_compositor::{ui_comp_grid_cursor_goto, ui_comp_grid_resize, ui_comp_grid_scroll};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::mem::size_of;
use std::ptr;

use crate::api::extmark::describe_ns;
use crate::api::private::helpers::{api_clear_error, arena_array, arena_dict, cstr_as_string};
use crate::api::private::validate::{
    Array, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::ScreenGrid;
use crate::api::win_config::UIExtension;
use crate::autocmd::do_autocmd_uienter;
use crate::cursor_shape::{cursor_get_mode_idx, cursorentry_T, mode_style_array};
use crate::drawscreen::{conceal_check_cursor_line, screen_resize};
use crate::eval::vars::LuaRef;
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, multiqueue_free, multiqueue_put_event,
};
use crate::ex_getln::cmdline_ui_flush;
use crate::grid::{get_win_by_grid_handle, sattr_T, schar_T};
use crate::highlight::{highlight_use_hlstate, ui_send_all_hls};
use crate::map::{Map, Set, map_del_uint32_t_ptr_t, map_put_ref_uint32_t_ptr_t, mh_get_uint32_t};
use crate::map_glyph_cache::{MH_TOMBSTONE, MapHash, NvimString};
use crate::memory::{Arena, arena_finish, arena_mem_free, xcalloc, xfree};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::os::time::{os_hrtime, os_sleep};
use crate::ui_compositor::{
    ui_comp_attach, ui_comp_detach, ui_comp_get_grid_at_coord, ui_comp_init, ui_comp_should_draw,
};
use crate::window::{win_set_inner_size, win_ui_flush};
use crate::winfloat::win_config_float;

const NUL: c_char = 0;
const MAXPATHL: usize = 4096;
const MAX_UI_COUNT: usize = 16;
const DEFAULT_GRID_HANDLE: handle_T = 1;
const SHAPE_IDX_N: c_int = 0;
const SHAPE_IDX_R: c_int = 3;
const CB_MAX_ERROR: u8 = 3;
const LOGLVL_ERR: c_int = 4;

const kUICmdline: usize = 0;
const kUIMessages: usize = 4;
const kUILinegrid: usize = 5;
const kUIGlobalCount: usize = kUILinegrid;
const kUIMultigrid: usize = 6;
const kUIHlState: usize = 7;
const kUITermColors: usize = 8;
const kUIFloatDebug: usize = 9;
const kUIExtCount: usize = 10;

const kLineFlagWrap: c_int = 1;
const kLineFlagInvalid: c_int = 2;

const kOptBoFlagAll: c_uint = 0x01;
const kOptRdbFlagLine: c_uint = 0x10;
const kOptRdbFlagFlush: c_uint = 0x20;
const kOptCmdheight: c_int = 44;

const HLF_W: c_int = 26;

const MODE_NORMAL: c_int = 0x01;
const MODE_INSERT: c_int = 0x10;
const MODE_CMDLINE: c_int = 0x08;
const MODE_HITRETURN: c_int = 0x2000 | MODE_NORMAL;
const MODE_ASKMORE: c_int = 0x3000;
const MODE_SETWSIZE: c_int = 0x4000;
const MODE_EXTERNCMD: c_int = 0x5000;

const kMouseNormal: c_int = b'n' as c_int;
const kMouseVisual: c_int = b'v' as c_int;
const kMouseReturn: c_int = b'r' as c_int;
const kMouseInsert: c_int = b'i' as c_int;
const kMouseCommand: c_int = b'c' as c_int;
const kMouseHelp: c_char = b'h' as c_char;
const MOUSE_A: &CStr = c"nvich";

const kRetNilBool: c_int = 1;

pub use crate::msgpack_rpc::channel::RemoteUI;

#[repr(C)]
struct UIEventCallback {
    cb: LuaRef,
    errors: u8,
    ext_widgets: [bool; kUIGlobalCount],
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut uis: [*mut RemoteUI; MAX_UI_COUNT] = [ptr::null_mut(); MAX_UI_COUNT];
static mut ui_ext: [bool; kUIExtCount] = [false; kUIExtCount];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ui_count: usize = 0;
static mut ui_mode_idx: c_int = SHAPE_IDX_N;
static mut cursor_row: c_int = 0;
static mut cursor_col: c_int = 0;
static mut pending_cursor_update: bool = false;
static mut busy: c_int = 0;
static mut pending_mode_info_update: bool = false;
static mut pending_mode_update: bool = false;
static mut cursor_grid_handle: handle_T = DEFAULT_GRID_HANDLE;

static mut ui_event_cbs: Map<u32, *mut c_void> = Map {
    set: Set {
        h: MapHash {
            n_buckets: 0,
            size: 0,
            n_occupied: 0,
            upper_bound: 0,
            n_keys: 0,
            keys_capacity: 0,
            hash: ptr::null_mut(),
        },
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};
static mut ui_cb_ext: [bool; kUIExtCount] = [false; kUIExtCount];

static mut has_mouse: bool = false;
static mut pending_has_mouse: c_int = -1;
static mut pending_default_colors: bool = false;
static mut beep_count: c_int = 0;
static mut beep_start_time: u64 = 0;
static mut ui_flush_was_busy: bool = false;
static mut ui_flush_cursor_was_obscured: bool = false;

unsafe extern "C" {

    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn uv_cwd(buffer: *mut c_char, size: *mut usize) -> c_int;
    fn gettext(msgid: *const c_char) -> *mut c_char;

}

#[inline]
const fn bool_obj(b: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
const fn int_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_init() {
    unsafe {
        default_grid.handle = 1;
        msg_grid_adj.target = &raw mut default_grid;
        ui_comp_init();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_free_all_mem() {
    unsafe {
        for i in 0..ui_event_cbs.set.h.n_keys as usize {
            let event_cb = (*ui_event_cbs.values.add(i)).cast::<UIEventCallback>();
            if !event_cb.is_null() {
                free_ui_event_callback(event_cb);
            }
        }
        xfree(ui_event_cbs.set.keys.cast());
        xfree(ui_event_cbs.set.h.hash.cast());
        ui_event_cbs.set = Set {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        };
        xfree(ui_event_cbs.values.cast());
        ui_event_cbs.values = ptr::null_mut();

        multiqueue_free(resize_events);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_rgb_attached() -> bool {
    unsafe {
        if p_tgc != 0 {
            return true;
        }
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            let is_tui = (*ui).stdin_tty || (*ui).stdout_tty;
            if !is_tui && (*ui).rgb {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_gui_attached() -> bool {
    unsafe {
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            let is_tui = (*ui).stdin_tty || (*ui).stdout_tty;
            if !is_tui {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_override() -> bool {
    unsafe {
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            if (*ui).r#override {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_active() -> usize {
    unsafe { ui_count }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_refresh() {
    unsafe {
        if ui_client_channel_id != 0 {
            std::process::abort();
        }

        let mut width = c_int::MAX;
        let mut height = c_int::MAX;
        let active = ui_active() != 0;
        let mut ext_widgets = [active; kUIExtCount];
        let inclusive = ui_override();
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();

        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            width = width.min((*ui).width);
            height = height.min((*ui).height);
            for j in 0..kUIExtCount {
                ext_widgets[j] &= (*ui).ui_ext[j] || inclusive;
            }
        }

        cursor_row = 0;
        cursor_col = 0;
        pending_cursor_update = true;

        let ui_ext_ptr = (&raw mut ui_ext).cast::<bool>();
        let ui_cb_ext_ptr = (&raw const ui_cb_ext).cast::<bool>();
        let ext_names_ptr = (&raw const ui_ext_names).cast::<*const c_char>();
        let had_message = *ui_ext_ptr.add(kUIMessages);

        for i in 0..kUIExtCount {
            let val = ext_widgets[i] | *ui_cb_ext_ptr.add(i);
            *ui_ext_ptr.add(i) = val;
            if i < kUIGlobalCount {
                ui_call_option_set(cstr_as_string(*ext_names_ptr.add(i)), bool_obj(val));
            }
        }

        if had_message != *ui_ext_ptr.add(kUIMessages) {
            if ui_refresh_cmdheight {
                set_option_value(
                    kOptCmdheight,
                    int_obj(Integer::from(had_message)),
                    0,
                    true,
                    ptr::null_mut(),
                );
                let mut tp = first_tabpage;
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

        if updating_screen {
            ui_schedule_refresh();
            return;
        }

        ui_default_colors_set();

        let save_p_lz = p_lz;
        p_lz = 0;
        screen_resize(width, height);
        p_lz = save_p_lz;

        ui_mode_info_set();
        pending_mode_update = true;
        ui_cursor_shape();
        pending_has_mouse = -1;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_pum_get_height() -> c_int {
    unsafe {
        let mut pum_height: c_int = 0;
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            let ui_pum_height = (*(*uis_ptr.add(i))).pum_nlines;
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_pum_get_pos(
    pwidth: *mut f64,
    pheight: *mut f64,
    prow: *mut f64,
    pcol: *mut f64,
) -> bool {
    unsafe {
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            if !(*ui).pum_pos {
                continue;
            }
            *pwidth = (*ui).pum_width;
            *pheight = (*ui).pum_height;
            *prow = (*ui).pum_row;
            *pcol = (*ui).pum_col;
            return true;
        }
        false
    }
}

unsafe extern "C" fn ui_refresh_event(_argv: *mut *mut c_void) {
    unsafe {
        ui_refresh();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_schedule_refresh() {
    unsafe {
        multiqueue_put_event(
            resize_events,
            Event {
                handler: Some(ui_refresh_event),
                argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
            },
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_default_colors_set() {
    unsafe {
        pending_default_colors = true;
        if starting == 0 {
            ui_may_set_default_colors();
        }
    }
}

unsafe fn ui_may_set_default_colors() {
    unsafe {
        if pending_default_colors {
            pending_default_colors = false;
            ui_call_default_colors_set(
                Integer::from(normal_fg),
                Integer::from(normal_bg),
                Integer::from(normal_sp),
                Integer::from(cterm_normal_fg_color),
                Integer::from(cterm_normal_bg_color),
            );
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_busy_start() {
    unsafe {
        let prev = busy;
        busy += 1;
        if prev == 0 {
            ui_call_busy_start();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_busy_stop() {
    unsafe {
        busy -= 1;
        if busy == 0 {
            ui_call_busy_stop();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_beep(val: c_uint) {
    unsafe {
        if emsg_silent != 0 || in_assert_fails {
            did_beep_silent += 1;
            return;
        }
        did_beep += 1;

        if (bo_flags & val) == 0 && (bo_flags & kOptBoFlagAll) == 0 {
            let now = os_hrtime();
            if beep_start_time == 0 || now.wrapping_sub(beep_start_time) > 500_000_000 {
                beep_count = 0;
                beep_start_time = now;
            }
            beep_count += 1;
            if beep_count <= 3 {
                if p_vb != 0 {
                    ui_call_visual_bell();
                } else {
                    ui_call_bell();
                }
            }
        }

        if !vim_strchr(p_debug, b'e' as c_int).is_null() {
            msg_source(HLF_W);
            msg(gettext(c"Beep!".as_ptr()), HLF_W);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_autocmd_uienter_all() {
    unsafe {
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        for i in 0..ui_count {
            do_autocmd_uienter((*(*uis_ptr.add(i))).channel_id, true);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_can_attach_more() -> bool {
    unsafe { ui_count < MAX_UI_COUNT }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_attach_impl(ui: *mut RemoteUI, chanid: u64) {
    unsafe {
        if ui_count >= MAX_UI_COUNT {
            std::process::abort();
        }
        if !(*ui).ui_ext[kUIMultigrid] && !(*ui).ui_ext[kUIFloatDebug] && ui_client_channel_id == 0
        {
            ui_comp_attach(ui.cast());
        }

        *(&raw mut uis).cast::<*mut RemoteUI>().add(ui_count) = ui;
        ui_count += 1;
        ui_refresh_options();
        resettitle();

        let mut cwd: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut cwdlen = MAXPATHL;
        if uv_cwd(cwd.as_mut_ptr(), &raw mut cwdlen) == 0 {
            ui_call_chdir(NvimString {
                data: cwd.as_mut_ptr(),
                size: cwdlen,
            });
        }

        for i in kUIGlobalCount..kUIExtCount {
            ui_set_ext_option(ui, i as UIExtension, (*ui).ui_ext[i]);
        }

        let mut sent = false;
        if (*ui).ui_ext[kUIHlState] {
            sent = highlight_use_hlstate();
        }
        if !sent {
            ui_send_all_hls(ui.cast());
        }
        ui_refresh();

        do_autocmd_uienter(chanid, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_detach_impl(ui: *mut RemoteUI, chanid: u64) {
    unsafe {
        if ui_count > MAX_UI_COUNT {
            std::process::abort();
        }
        let uis_ptr = (&raw mut uis).cast::<*mut RemoteUI>();
        let mut shift_index = MAX_UI_COUNT;

        for i in 0..ui_count {
            if *uis_ptr.add(i) == ui {
                shift_index = i;
                break;
            }
        }

        if shift_index >= MAX_UI_COUNT {
            std::process::abort();
        }

        while shift_index < ui_count - 1 {
            *uis_ptr.add(shift_index) = *uis_ptr.add(shift_index + 1);
            shift_index += 1;
        }

        ui_count -= 1;
        if ui_count != 0 && !exiting {
            ui_schedule_refresh();
        }

        if !(*ui).ui_ext[kUIMultigrid] && !(*ui).ui_ext[kUIFloatDebug] {
            ui_comp_detach(ui.cast());
        }

        do_autocmd_uienter(chanid, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_set_ext_option(ui: *mut RemoteUI, ext: UIExtension, active: bool) {
    unsafe {
        let idx = ext as usize;
        if idx < kUIGlobalCount {
            ui_refresh();
            return;
        }
        let name = *(&raw const ui_ext_names).cast::<*const c_char>().add(idx);
        if *name != b'_' as c_char || active {
            remote_ui_option_set(ui, cstr_as_string(name), bool_obj(active));
        }
        if idx == kUITermColors {
            ui_default_colors_set();
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_line(
    grid: *mut ScreenGrid,
    row: c_int,
    invalid_row: bool,
    startcol: c_int,
    endcol: c_int,
    clearcol: c_int,
    clearattr: c_int,
    wrap: bool,
) {
    unsafe {
        debug_assert!(0 <= row && row < (*grid).rows);
        let mut flags = if wrap { kLineFlagWrap } else { 0 };
        if startcol == 0 && invalid_row {
            flags |= kLineFlagInvalid;
        }

        ui_may_set_default_colors();

        let off = *(*grid).line_offset.add(row as usize) + startcol as usize;
        ui_call_raw_line(
            Integer::from((*grid).handle),
            Integer::from(row),
            Integer::from(startcol),
            Integer::from(endcol),
            Integer::from(clearcol),
            Integer::from(clearattr),
            flags,
            (*grid).chars.add(off),
            (*grid).attrs.add(off),
        );

        if p_wd != 0 && (rdb_flags & kOptRdbFlagLine) != 0 {
            ui_call_grid_cursor_goto(
                Integer::from((*grid).handle),
                Integer::from(row),
                Integer::from(clearcol.min((*grid).cols - 1)),
            );
            ui_call_flush();
            os_sleep(p_wd.unsigned_abs());
            pending_cursor_update = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_cursor_goto(new_row: c_int, new_col: c_int) {
    unsafe {
        ui_grid_cursor_goto(DEFAULT_GRID_HANDLE, new_row, new_col);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_grid_cursor_goto(
    grid_handle: handle_T,
    new_row: c_int,
    new_col: c_int,
) {
    unsafe {
        if new_row == cursor_row && new_col == cursor_col && grid_handle == cursor_grid_handle {
            return;
        }
        cursor_row = new_row;
        cursor_col = new_col;
        cursor_grid_handle = grid_handle;
        pending_cursor_update = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_check_cursor_grid(grid_handle: handle_T) {
    unsafe {
        if cursor_grid_handle == grid_handle {
            pending_cursor_update = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_mode_info_set() {
    unsafe {
        pending_mode_info_update = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_current_row() -> c_int {
    unsafe { cursor_row }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_current_col() -> c_int {
    unsafe { cursor_col }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_flush() {
    unsafe {
        debug_assert!(ui_client_channel_id == 0);
        if ui_active() == 0 {
            return;
        }

        if (State & MODE_CMDLINE) == 0 && (*curwin).w_floating && (*curwin).w_config.hide {
            if !ui_flush_was_busy {
                ui_call_busy_start();
                ui_flush_was_busy = true;
            }
        } else if ui_flush_was_busy {
            ui_call_busy_stop();
            ui_flush_was_busy = false;
        }

        win_ui_flush(false);
        if textlock == 0 && expr_map_lock == 0 {
            cmdline_ui_flush();
            msg_ext_ui_flush();
        }
        msg_scroll_flush();

        if pending_cursor_update {
            ui_call_grid_cursor_goto(
                Integer::from(cursor_grid_handle),
                Integer::from(cursor_row),
                Integer::from(cursor_col),
            );
            pending_cursor_update = false;
            win_ui_flush(false);
        }
        if pending_mode_info_update {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let style = mode_style_array(&raw mut arena);
            let enabled = *p_guicursor != NUL;
            ui_call_mode_info_set(enabled, style);
            arena_mem_free(arena_finish(&raw mut arena));
            pending_mode_info_update = false;
        }

        let cursor_obscured = ui_cursor_is_behind_floatwin();
        if (cursor_obscured != ui_flush_cursor_was_obscured || pending_mode_update) && starting == 0
        {
            let idx = if cursor_obscured {
                SHAPE_IDX_R
            } else {
                ui_mode_idx
            };
            let full_name = (*(&raw const shape_table)
                .cast::<cursorentry_T>()
                .add(idx as usize))
            .full_name;
            ui_call_mode_change(cstr_as_string(full_name), Integer::from(idx));
            pending_mode_update = false;
            ui_flush_cursor_was_obscured = cursor_obscured;
        }

        if pending_has_mouse != c_int::from(has_mouse) {
            if has_mouse {
                ui_call_mouse_on();
            } else {
                ui_call_mouse_off();
            }
            pending_has_mouse = c_int::from(has_mouse);
        }
        ui_call_flush();

        if p_wd != 0 && (rdb_flags & kOptRdbFlagFlush) != 0 {
            os_sleep(p_wd.unsigned_abs());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_check_mouse() {
    unsafe {
        has_mouse = false;
        if *p_mouse == NUL {
            return;
        }

        let checkfor = if Visual.active {
            kMouseVisual
        } else if State == MODE_HITRETURN || State == MODE_ASKMORE || State == MODE_SETWSIZE {
            kMouseReturn
        } else if (State & MODE_INSERT) != 0 {
            kMouseInsert
        } else if (State & MODE_CMDLINE) != 0 {
            kMouseCommand
        } else if State == MODE_EXTERNCMD {
            b' ' as c_int
        } else {
            kMouseNormal
        };

        if ui_mouse_has(checkfor) {
            has_mouse = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_mouse_has(mode: c_int) -> bool {
    unsafe {
        let mut p = p_mouse;
        while *p != NUL {
            if *p == b'a' as c_char {
                if !vim_strchr(MOUSE_A.as_ptr(), mode).is_null() {
                    return true;
                }
            } else if *p == kMouseHelp {
                if mode != kMouseReturn && (*curbuf).b_help {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_cursor_shape_no_check_conceal() {
    unsafe {
        if !full_screen {
            return;
        }
        let new_mode_idx = cursor_get_mode_idx();
        if new_mode_idx != ui_mode_idx {
            ui_mode_idx = new_mode_idx;
            pending_mode_update = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_cursor_shape() {
    unsafe {
        ui_cursor_shape_no_check_conceal();
        conceal_check_cursor_line();
    }
}

unsafe fn ui_cursor_is_behind_floatwin() -> bool {
    unsafe {
        if (State & MODE_CMDLINE) != 0 || !ui_comp_should_draw() {
            return false;
        }

        let crow = (*curwin).w_winrow + (*curwin).w_winrow_off + (*curwin).w_wrow;
        let ccol = (*curwin).w_wincol
            + (*curwin).w_wincol_off
            + if (*curwin).w_onebuf_opt.wo_rl != 0 {
                (*curwin).w_view_width - (*curwin).w_wcol - 1
            } else {
                (*curwin).w_wcol
            };

        let top_grid = ui_comp_get_grid_at_coord(crow, ccol);
        top_grid != &raw mut (*curwin).w_grid_alloc
            && top_grid != &raw mut default_grid
            && (*top_grid).zindex >= (*curwin).w_grid_alloc.zindex + 50
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_has(ext: UIExtension) -> bool {
    unsafe { *(&raw const ui_ext).cast::<bool>().add(ext as usize) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_array(arena: *mut Arena) -> Array {
    unsafe {
        let mut all_uis = arena_array(arena, ui_count);
        let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
        let ext_names_ptr = (&raw const ui_ext_names).cast::<*const c_char>();

        for i in 0..ui_count {
            let ui = *uis_ptr.add(i);
            let mut info = arena_dict(arena, 10 + kUIExtCount);
            let mut put_info = |k: *const c_char, v: Object| {
                *info.items.add(info.size) = KeyValuePair {
                    key: cstr_as_string(k),
                    value: v,
                };
                info.size += 1;
            };

            put_info(c"width".as_ptr(), int_obj(Integer::from((*ui).width)));
            put_info(c"height".as_ptr(), int_obj(Integer::from((*ui).height)));
            put_info(c"rgb".as_ptr(), bool_obj((*ui).rgb));
            put_info(c"override".as_ptr(), bool_obj((*ui).r#override));
            put_info(
                c"term_name".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string((*ui).term_name),
                    },
                },
            );
            put_info(
                c"term_background".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: NvimString {
                            data: c"".as_ptr().cast_mut(),
                            size: 0,
                        },
                    },
                },
            );
            put_info(
                c"term_colors".as_ptr(),
                int_obj(Integer::from((*ui).term_colors)),
            );
            put_info(c"stdin_tty".as_ptr(), bool_obj((*ui).stdin_tty));
            put_info(c"stdout_tty".as_ptr(), bool_obj((*ui).stdout_tty));

            for j in 0..kUIExtCount {
                let ename = *ext_names_ptr.add(j);
                if *ename != b'_' as c_char || (*ui).ui_ext[j] {
                    put_info(ename, bool_obj((*ui).ui_ext[j]));
                }
            }
            put_info(c"chan".as_ptr(), int_obj((*ui).channel_id as Integer));

            *all_uis.items.add(all_uis.size) = Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: info },
            };
            all_uis.size += 1;
        }
        all_uis
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_grid_resize(
    grid_handle: handle_T,
    width: c_int,
    height: c_int,
    err: *mut Error,
) {
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
                win_config_float(wp, (*wp).w_config);
            }
        } else {
            (*wp).w_height_request = height.max(0);
            (*wp).w_width_request = width.max(0);
            win_set_inner_size(wp, true);
        }
    }
}

unsafe fn ui_attach_error(ns_id: u32, name: *const c_char, msg: *const c_char) {
    unsafe {
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
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_event(name: *mut c_char, args: Array) {
    unsafe {
        let save_expr_map_lock = expr_map_lock;
        let save_textlock = textlock;
        expr_map_lock = 0;
        textlock = 0;

        let mut handled = false;
        let fast = msg_ext_fast && strcmp(c"msg_show".as_ptr(), name) == 0;
        for i in 0..ui_event_cbs.set.h.n_keys as usize {
            ui_event_ns_id = *ui_event_cbs.set.keys.add(i);
            let event_cb = (*ui_event_cbs.values.add(i)).cast::<UIEventCallback>();
            if event_cb.is_null() {
                continue;
            }
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let ns_id = ui_event_ns_id;
            let res = nlua_call_ref_ctx(
                fast,
                (*event_cb).cb,
                name,
                args,
                kRetNilBool,
                ptr::null_mut(),
                &raw mut err,
            );
            ui_event_ns_id = 0;
            if res.obj_type == ObjectType::kObjectTypeBoolean && res.data.boolean {
                handled = true;
            }
            if !matches!(err.err_type, ErrorType::kErrorTypeNone) {
                ui_attach_error(ns_id, name, err.msg);
                ui_remove_cb(ns_id, true);
            }
            api_clear_error(&raw mut err);
        }
        expr_map_lock = save_expr_map_lock;
        textlock = save_textlock;

        if !handled {
            let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
            for i in 0..ui_count {
                remote_ui_event(*uis_ptr.add(i), name, args);
            }
        }
    }
}

unsafe fn ui_cb_update_ext() {
    unsafe {
        let ui_cb_ext_ptr = (&raw mut ui_cb_ext).cast::<bool>();
        ptr::write_bytes(ui_cb_ext_ptr, 0, kUIExtCount);

        for i in 0..kUIGlobalCount {
            for k in 0..ui_event_cbs.set.h.n_keys as usize {
                let event_cb = (*ui_event_cbs.values.add(k)).cast::<UIEventCallback>();
                if !event_cb.is_null() && (*event_cb).ext_widgets[i] {
                    *ui_cb_ext_ptr.add(i) = true;
                    break;
                }
            }
        }
    }
}

unsafe fn free_ui_event_callback(event_cb: *mut UIEventCallback) {
    unsafe {
        api_free_luaref((*event_cb).cb);
        xfree(event_cb.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_add_cb(ns_id: u32, cb: LuaRef, ext_widgets: *mut bool) {
    unsafe {
        let event_cb = xcalloc(1, size_of::<UIEventCallback>()).cast::<UIEventCallback>();
        (*event_cb).cb = cb;
        ptr::copy_nonoverlapping(
            ext_widgets,
            (&raw mut (*event_cb).ext_widgets).cast::<bool>(),
            kUIGlobalCount,
        );
        if (*event_cb).ext_widgets[kUIMessages] {
            (*event_cb).ext_widgets[kUICmdline] = true;
        }

        let item = map_put_ref_uint32_t_ptr_t(
            &raw mut ui_event_cbs,
            ns_id,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if !(*item).is_null() {
            free_ui_event_callback((*item).cast::<UIEventCallback>());
        }
        *item = event_cb.cast();

        ui_cb_update_ext();
        ui_refresh();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_remove_cb(ns_id: u32, checkerr: bool) {
    unsafe {
        let k = mh_get_uint32_t(&raw mut ui_event_cbs.set, ns_id);
        let item = if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*ui_event_cbs.values.add(k as usize)).cast::<UIEventCallback>()
        };
        if !item.is_null()
            && (!checkerr || {
                (*item).errors += 1;
                (*item).errors > CB_MAX_ERROR
            })
        {
            map_del_uint32_t_ptr_t(&raw mut ui_event_cbs, ns_id, ptr::null_mut());
            free_ui_event_callback(item);
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

const LOGLVL_DBG: c_int = 1;
static mut UILOG_SEEN: usize = 0;
static mut UILOG_LAST_EVENT: *const c_char = ptr::null();

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_log(funname: *const c_char) {
    #[cfg(debug_assertions)]
    unsafe {
        if crate::memory::entered_free_all_mem {
            return;
        }

        if UILOG_LAST_EVENT == funname {
            UILOG_SEEN += 1;
        } else {
            if UILOG_SEEN > 0 {
                logmsg!(
                    LOGLVL_DBG,
                    c"UI: ".as_ptr(),
                    ptr::null(),
                    -1,
                    true,
                    c"%s (+%zu times...)".as_ptr(),
                    UILOG_LAST_EVENT,
                    UILOG_SEEN,
                );
            }
            logmsg!(
                LOGLVL_DBG,
                c"UI: ".as_ptr(),
                ptr::null(),
                -1,
                true,
                c"%s".as_ptr(),
                funname,
            );
            UILOG_SEEN = 0;
            UILOG_LAST_EVENT = funname;
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = funname;
    }
}

const NOARGS_ARRAY: Array = Array {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

#[inline]
const fn str_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn arr_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
const fn float_obj(f: f64) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeFloat,
        data: ObjectData { floating: f },
    }
}

#[inline]
const fn win_obj(w: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeWindow,
        data: ObjectData {
            integer: w as Integer,
        },
    }
}

#[inline]
const fn buf_obj(b: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBuffer,
        data: ObjectData {
            integer: b as Integer,
        },
    }
}

#[inline]
const fn tab_obj(t: handle_T) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeTabpage,
        data: ObjectData {
            integer: t as Integer,
        },
    }
}

macro_rules! ui_call_loop {
    ($cond:expr, $funname:literal, |$ui:ident| $call:expr) => {
        unsafe {
            let mut any_call = false;
            let uis_ptr = (&raw const uis).cast::<*mut RemoteUI>();
            for i in 0..ui_count {
                let $ui = *uis_ptr.add(i);
                if $cond {
                    $call;
                    any_call = true;
                }
            }
            if any_call {
                ui_log($funname.as_ptr());
            }
        }
    };
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_mode_info_set(enabled: bool, cursor_styles: Array) {
    ui_call_loop!(true, c"mode_info_set", |ui| {
        remote_ui_mode_info_set(ui, enabled, cursor_styles)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_update_menu() {
    ui_call_loop!(true, c"update_menu", |ui| remote_ui_update_menu(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_busy_start() {
    ui_call_loop!(true, c"busy_start", |ui| remote_ui_busy_start(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_busy_stop() {
    ui_call_loop!(true, c"busy_stop", |ui| remote_ui_busy_stop(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_mouse_on() {
    ui_call_loop!(true, c"mouse_on", |ui| remote_ui_mouse_on(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_mouse_off() {
    ui_call_loop!(true, c"mouse_off", |ui| remote_ui_mouse_off(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_mode_change(mode: NvimString, mode_idx: Integer) {
    ui_call_loop!(true, c"mode_change", |ui| {
        remote_ui_mode_change(ui, mode, mode_idx)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_bell() {
    ui_call_loop!(true, c"bell", |ui| remote_ui_bell(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_visual_bell() {
    ui_call_loop!(true, c"visual_bell", |ui| remote_ui_visual_bell(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_flush() {
    ui_call_loop!(true, c"flush", |ui| remote_ui_flush(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_suspend() {
    ui_call_loop!(true, c"suspend", |ui| remote_ui_suspend(ui));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_set_title(title: NvimString) {
    ui_call_loop!(true, c"set_title", |ui| remote_ui_set_title(ui, title));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_set_icon(icon: NvimString) {
    ui_call_loop!(true, c"set_icon", |ui| remote_ui_set_icon(ui, icon));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_screenshot(path: NvimString) {
    ui_call_loop!(true, c"screenshot", |ui| remote_ui_screenshot(ui, path));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_option_set(name: NvimString, value: Object) {
    ui_call_loop!(true, c"option_set", |ui| {
        remote_ui_option_set(ui, name, value)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_chdir(path: NvimString) {
    ui_call_loop!(true, c"chdir", |ui| remote_ui_chdir(ui, path));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_ui_send(content: NvimString) {
    ui_call_loop!(true, c"ui_send", |ui| remote_ui_ui_send(ui, content));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_update_fg(fg: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(fg)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"update_fg".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_update_bg(bg: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(bg)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"update_bg".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_update_sp(sp: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(sp)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"update_sp".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_resize(width: Integer, height: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(width), int_obj(height)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"resize".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_clear() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"clear".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_eol_clear() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"eol_clear".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cursor_goto(row: Integer, col: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(row), int_obj(col)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"cursor_goto".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_put(str_in: NvimString) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [str_obj(str_in)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"put".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_set_scroll_region(
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(top), int_obj(bot), int_obj(left), int_obj(right)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 4,
            capacity: 4,
        };
        ui_call_event(c"set_scroll_region".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_scroll(count: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(count)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"scroll".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_default_colors_set(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_hl_attr_define(
    id: Integer,
    rgb_attrs: crate::map::HlAttrs,
    cterm_attrs: crate::map::HlAttrs,
    info: Array,
) {
    ui_call_loop!(true, c"hl_attr_define", |ui| {
        remote_ui_hl_attr_define(ui, id, rgb_attrs, cterm_attrs, info)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_hl_group_set(name: NvimString, id: Integer) {
    ui_call_loop!(true, c"hl_group_set", |ui| {
        remote_ui_hl_group_set(ui, name, id)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_resize(grid: Integer, width: Integer, height: Integer) {
    unsafe {
        ui_comp_grid_resize(grid, width, height);
    }
    ui_call_loop!(!(*ui).composed, c"grid_resize", |ui| {
        remote_ui_grid_resize(ui, grid, width, height)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_composed_call_grid_resize(
    grid: Integer,
    width: Integer,
    height: Integer,
) {
    ui_call_loop!((*ui).composed, c"grid_resize", |ui| {
        remote_ui_grid_resize(ui, grid, width, height)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_clear(grid: Integer) {
    ui_call_loop!(true, c"grid_clear", |ui| remote_ui_grid_clear(ui, grid));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_cursor_goto(grid: Integer, row: Integer, col: Integer) {
    unsafe {
        ui_comp_grid_cursor_goto(grid, row, col);
    }
    ui_call_loop!(!(*ui).composed, c"grid_cursor_goto", |ui| {
        remote_ui_grid_cursor_goto(ui, grid, row, col)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_composed_call_grid_cursor_goto(
    grid: Integer,
    row: Integer,
    col: Integer,
) {
    ui_call_loop!((*ui).composed, c"grid_cursor_goto", |ui| {
        remote_ui_grid_cursor_goto(ui, grid, row, col)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_line(
    grid: Integer,
    row: Integer,
    col_start: Integer,
    data: Array,
    wrap: bool,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            int_obj(grid),
            int_obj(row),
            int_obj(col_start),
            arr_obj(data),
            bool_obj(wrap),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        };
        ui_call_event(c"grid_line".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_scroll(
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    unsafe {
        ui_comp_grid_scroll(grid, top, bot, left, right, rows, cols);
    }
    ui_call_loop!(!(*ui).composed, c"grid_scroll", |ui| {
        remote_ui_grid_scroll(ui, grid, top, bot, left, right, rows, cols)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_composed_call_grid_scroll(
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    ui_call_loop!((*ui).composed, c"grid_scroll", |ui| {
        remote_ui_grid_scroll(ui, grid, top, bot, left, right, rows, cols)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_grid_destroy(grid: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(grid)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"grid_destroy".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_raw_line(
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
    unsafe {
        ui_comp_raw_line(
            grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        );
    }
    ui_call_loop!(!(*ui).composed, c"raw_line", |ui| {
        remote_ui_raw_line(
            ui, grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        )
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_composed_call_raw_line(
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
    ui_call_loop!((*ui).composed, c"raw_line", |ui| {
        remote_ui_raw_line(
            ui, grid, row, startcol, endcol, clearcol, clearattr, flags, chunk, attrs,
        )
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_pos(
    grid: Integer,
    win: handle_T,
    startrow: Integer,
    startcol: Integer,
    width: Integer,
    height: Integer,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            int_obj(grid),
            win_obj(win),
            int_obj(startrow),
            int_obj(startcol),
            int_obj(width),
            int_obj(height),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 6,
            capacity: 6,
        };
        ui_call_event(c"win_pos".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_float_pos(
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
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            int_obj(grid),
            win_obj(win),
            str_obj(anchor),
            int_obj(anchor_grid),
            float_obj(anchor_row),
            float_obj(anchor_col),
            bool_obj(mouse_enabled),
            int_obj(zindex),
            int_obj(compindex),
            int_obj(screen_row),
            int_obj(screen_col),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 11,
            capacity: 11,
        };
        ui_call_event(c"win_float_pos".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_external_pos(grid: Integer, win: handle_T) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(grid), win_obj(win)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"win_external_pos".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_hide(grid: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(grid)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"win_hide".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_close(grid: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(grid)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"win_close".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_set_pos(
    grid: Integer,
    row: Integer,
    scrolled: bool,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    unsafe {
        ui_comp_msg_set_pos(grid, row, scrolled, sep_char, zindex, compindex);
    }
    ui_call_loop!(!(*ui).composed, c"msg_set_pos", |ui| {
        remote_ui_msg_set_pos(ui, grid, row, scrolled, sep_char, zindex, compindex)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_composed_call_msg_set_pos(
    grid: Integer,
    row: Integer,
    scrolled: bool,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    ui_call_loop!((*ui).composed, c"msg_set_pos", |ui| {
        remote_ui_msg_set_pos(ui, grid, row, scrolled, sep_char, zindex, compindex)
    });
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_viewport(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_viewport_margins(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_win_extmark(
    grid: Integer,
    win: handle_T,
    ns_id: Integer,
    mark_id: Integer,
    row: Integer,
    col: Integer,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            int_obj(grid),
            win_obj(win),
            int_obj(ns_id),
            int_obj(mark_id),
            int_obj(row),
            int_obj(col),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 6,
            capacity: 6,
        };
        ui_call_event(c"win_extmark".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_popupmenu_show(
    items_arr: Array,
    selected: Integer,
    row: Integer,
    col: Integer,
    grid: Integer,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            arr_obj(items_arr),
            int_obj(selected),
            int_obj(row),
            int_obj(col),
            int_obj(grid),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        };
        ui_call_event(c"popupmenu_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_popupmenu_hide() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"popupmenu_hide".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_popupmenu_select(selected: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(selected)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"popupmenu_select".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_tabline_update(
    current: handle_T,
    tabs: Array,
    current_buffer: handle_T,
    buffers: Array,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            tab_obj(current),
            arr_obj(tabs),
            buf_obj(current_buffer),
            arr_obj(buffers),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 4,
            capacity: 4,
        };
        ui_call_event(c"tabline_update".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_show(
    content: Array,
    pos: Integer,
    firstc: NvimString,
    prompt: NvimString,
    indent: Integer,
    level: Integer,
    hl_id: Integer,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            arr_obj(content),
            int_obj(pos),
            str_obj(firstc),
            str_obj(prompt),
            int_obj(indent),
            int_obj(level),
            int_obj(hl_id),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 7,
            capacity: 7,
        };
        ui_call_event(c"cmdline_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_pos(pos: Integer, level: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(pos), int_obj(level)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"cmdline_pos".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_special_char(c: NvimString, shift: bool, level: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [str_obj(c), bool_obj(shift), int_obj(level)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        ui_call_event(c"cmdline_special_char".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_hide(level: Integer, abort: bool) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(level), bool_obj(abort)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"cmdline_hide".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_block_show(lines: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(lines)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"cmdline_block_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_block_append(lines: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(lines)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"cmdline_block_append".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_cmdline_block_hide() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"cmdline_block_hide".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_wildmenu_show(items_arr: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(items_arr)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"wildmenu_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_wildmenu_select(selected: Integer) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [int_obj(selected)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"wildmenu_select".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_wildmenu_hide() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"wildmenu_hide".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_show(
    kind: NvimString,
    content: Array,
    replace_last: bool,
    history: bool,
    append: bool,
    id: Object,
    trigger: NvimString,
) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [
            str_obj(kind),
            arr_obj(content),
            bool_obj(replace_last),
            bool_obj(history),
            bool_obj(append),
            id,
            str_obj(trigger),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 7,
            capacity: 7,
        };
        ui_call_event(c"msg_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_clear() {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        ui_call_event(c"msg_clear".as_ptr().cast_mut(), NOARGS_ARRAY);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_showcmd(content: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(content)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"msg_showcmd".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_showmode(content: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(content)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"msg_showmode".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_ruler(content: Array) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(content)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        ui_call_event(c"msg_ruler".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_msg_history_show(entries: Array, prev_cmd: bool) {
    static mut ENTERED: bool = false;
    unsafe {
        if ENTERED {
            return;
        }
        ENTERED = true;
        let mut items = [arr_obj(entries), bool_obj(prev_cmd)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        ui_call_event(c"msg_history_show".as_ptr().cast_mut(), args);
        ENTERED = false;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call_error_exit(status: Integer) {
    ui_call_loop!(true, c"error_exit", |ui| remote_ui_error_exit(ui, status));
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_call__set_restart_on_crash_exit(progpath: NvimString, argv: Array) {
    ui_call_loop!(true, c"_set_restart_on_crash_exit", |ui| {
        remote_ui__set_restart_on_crash_exit(ui, progpath, argv)
    });
}
