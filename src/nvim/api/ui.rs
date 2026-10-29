//! Remote UI API and msgpack redraw packing (`src/nvim/api/ui.c`).

use crate::api::private::helpers::{
    api_clear_error, api_set_error, api_typename, arena_array, arena_dict, array_obj, boolean_obj,
    cstr_as_string, dict_obj, error_set, integer_obj, string_obj, window_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType, api_err_exp,
};
use crate::api::win_config::UIExtension;
use crate::autocmd::{do_autocmd_focusgained, may_trigger_vim_suspend_resume};
use crate::channel::channel_close;
use crate::errors::*;
use crate::event::r#loop::{loop_poll_events, loop_queues};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::wstream::wstream_new_vec_buffer;
use crate::globals::*;
use crate::grid::{Sattr, Schar, schar_from_ascii, schar_get, schar_get_adv};
use crate::highlight::{HlAttrs, hl_get_url, hlattrs2dict, syn_attr2entry};
use crate::lua::converter::{Boolean, Float};
use crate::mbyte::utf_ambiguous_width;
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::channel::{RemoteUI, find_channel, rpc_write_raw};
use crate::msgpack_rpc::packer::{
    Integer, ObjHandle, PackerBuffer, mpack_array, mpack_bool, mpack_object_array, mpack_uint,
    mpack_w,
};
use crate::option::set_tty_option;
use crate::option_vars::*;
use crate::ui::{
    ui_active, ui_attach_impl, ui_call_ui_send, ui_can_attach_more, ui_detach_impl, ui_grid_resize,
    ui_refresh, ui_set_ext_option,
};
use std::ffi::{CStr, c_char};
use std::ptr;

#[cfg(windows)]
use crate::channel::K_CHANNEL_STREAM_STDIO;
#[cfg(windows)]
use crate::os::os_win_console::os_detach_ui_console;

pub(crate) const MAX_SCHAR_SIZE: usize = 32;
const UI_BUF_SIZE: usize = 4096;
const EVENT_BUF_SIZE: usize = 256;
pub(crate) const HLATTRS_DICT_SIZE: usize = 24;
pub(crate) const NO_SCREEN: i32 = 2;
pub(crate) const CHAN_STDIO: u64 = 1;

pub(crate) const K_CHANNEL_PART_ALL: i32 = 4;

pub(crate) const K_UICMDLINE: usize = 0;
const K_UIPOPUPMENU: usize = 1;
const K_UIWILDMENU: usize = 3;
pub(crate) const K_UIMESSAGES: usize = 4;
pub(crate) const K_UILINEGRID: usize = 5;
pub(crate) const K_UIMULTIGRID: usize = 6;
pub(crate) const K_UIHL_STATE: usize = 7;
pub(crate) const K_UITERM_COLORS: usize = 8;
pub(crate) const K_UIEXT_COUNT: usize = 10;

pub(crate) const K_LINE_FLAG_WRAP: i32 = 1;

pub(crate) const ARRAY_INIT: Array = Array {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

const DICT_INIT: Dict = Dict {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

struct ApiUiState {
    connected_uis: std::collections::BTreeMap<u64, Box<RemoteUI>>,
}

impl ApiUiState {
    const fn new() -> Self {
        Self {
            connected_uis: std::collections::BTreeMap::new(),
        }
    }
}

static API_UI_STATE: crate::globals::GlobalState<ApiUiState> =
    crate::globals::GlobalState::new(ApiUiState::new());

#[inline]
fn api_ui_st() -> &'static mut ApiUiState {
    unsafe { &mut *API_UI_STATE.as_ptr() }
}

use crate::msgpack_rpc::packer::mpack_w2;

#[inline]
fn buf_pos(ui: &RemoteUI) -> usize {
    crate::memory::ptr_offset_from(ui.packer.ptr, ui.packer.startptr) as usize
}

fn get_ui_or_err(chan_id: u64, err: &mut Error) -> Option<&'static mut RemoteUI> {
    let ui = api_ui_st()
        .connected_uis
        .get_mut(&chan_id)
        .map(|b| &mut **b);
    if ui.is_none() {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"UI not attached to channel: %lld",
            chan_id as i64,
        );
    }
    ui
}

fn mpack_array_dyn16(buf: *mut *mut u8) -> *mut u8 {
    mpack_w(buf, 0xdc);
    let pos = unsafe { *buf };
    mpack_w2(buf, 0xFFEF);
    pos
}

fn mpack_str_small(buf: *mut *mut u8, str_: impl crate::strings::IntoCCharPtr, len: usize) {
    let str_ = str_.into_c_char_ptr();
    unsafe {
        debug_assert!(len < 0x20);
        mpack_w(buf, (0xa0 | len) as u8);
        ptr::copy_nonoverlapping(str_.cast::<u8>(), *buf, len);
        *buf = (*buf).wrapping_add(len);
    }
}

/// Removes the client on the given channel from the list of UIs.
///
/// @param err  if non-NULL and there is no UI on the channel, set an error
/// @param send_error_exit  send an "error_exit" event with 0 status first
// DLLEXPORT void remote_ui_disconnect(uint64_t channel_id, Error *err, bool send_error_exit);
pub fn remote_ui_disconnect(channel_id: u64, err: &mut Error, send_error_exit: bool) {
    let Some(ui) = get_ui_or_err(channel_id, err) else {
        return;
    };
    if send_error_exit {
        let mut args_items: [Object; 1] = [integer_obj(0)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"error_exit", args);
        ui_flush_buf(ui, false);
    }
    let mut ui_box = api_ui_st().connected_uis.remove(&channel_id);
    let ui_detached = ui_box.as_deref_mut().unwrap_or(ui);
    ui_detach_impl(ui_detached, channel_id);
    let chan = find_channel(channel_id);
    if !chan.is_null() {
        unsafe {
            (*chan).rpc.has_ui = false;
        }
    }
    drop(ui_box);
}

/// Detaches the UI on channel `chan` (server keeps running). Sets `channel.detach`, sends an
/// "error_exit" UI event, and closes the channel.
///
/// @param chan  UI channel to disconnect.
/// @param[out] err Error details, if any.
// DLLEXPORT void ui_detach_channel(uint64_t chan, Error *err);
pub fn ui_detach_channel(chan: u64, err: &mut Error) {
    let c = find_channel(chan);
    if c.is_null() {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            (&raw const E_INVCHAN).cast::<c_char>(),
        );
        return;
    }
    let c = unsafe { &mut *c };
    #[cfg(windows)]
    let detach_stdio = c.streamtype == K_CHANNEL_STREAM_STDIO;
    c.detach = true;
    remote_ui_disconnect(chan, err, true);
    if error_set(err) {
        return;
    }
    channel_close(chan, K_CHANNEL_PART_ALL, ptr::null_mut());
    #[cfg(windows)]
    {
        os_detach_ui_console(detach_stdio);
    }
}

/// Detaches every UI except `keep_chan` (server keeps running).
///
/// @param keep_chan  UI channel to preserve.
/// @return Number of UIs detached.
// DLLEXPORT size_t ui_detach_others(uint64_t keep_chan);
pub fn ui_detach_others(keep_chan: u64) -> usize {
    let mut chans = Vec::<u64>::new();
    for &chan in api_ui_st().connected_uis.keys() {
        if chan != keep_chan {
            chans.push(chan);
        }
    }

    let mut detached: usize = 0;
    for chan in chans {
        let mut err = Error::new();
        ui_detach_channel(chan, &mut err);
        if error_set(&err) {
            api_clear_error(&mut err);
        } else {
            detached += 1;
        }
    }

    detached
}

// DLLEXPORT void remote_ui_free_all_mem(void);
pub fn remote_ui_free_all_mem() {
    api_ui_st().connected_uis.clear();
}

/// Wait until UI has connected.
// DLLEXPORT void remote_ui_wait_for_attach(void);
pub fn remote_ui_wait_for_attach() {
    unsafe {
        while ui_active() == 0 {
            let events = loop_queues(&mut g_st().main_loop).events;
            if !multiqueue_empty(&*events) {
                multiqueue_process_events(&mut *events);
            } else {
                loop_poll_events(&mut g_st().main_loop, -1);
                multiqueue_process_events(&mut *events);
            }
        }
    }
}

/// Activates UI events on the channel.
///
/// Entry point of all UI clients.  Allows |--embed| to continue startup.
/// Implies that the client is ready to show the UI.  Adds the client to the
/// list of UIs. |nvim_list_uis()|
///
/// @note If multiple UI clients are attached, the global screen dimensions
///       degrade to the smallest client. E.g. if client A requests 80x40 but
///       client B requests 200x100, the global screen has size 80x40.
///
/// @param channel_id
/// @param width  Requested screen columns
/// @param height  Requested screen rows
/// @param options  |ui-option| map
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_attach(uint64_t channel_id, Integer width, Integer height, Dict options,
//                               Error *err) FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    options: Dict,
    err: &mut Error,
) {
    if api_ui_st().connected_uis.contains_key(&channel_id) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"UI already attached to channel: %lld",
            channel_id as i64,
        );
        return;
    }
    if !ui_can_attach_more() {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Maximum UI count reached",
        );
        return;
    }
    if width <= 0 || height <= 0 {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Expected width > 0 and height > 0",
        );
        return;
    }

    let mut ui_box = Box::new(RemoteUI {
        rgb: true,
        r#override: false,
        composed: false,
        ui_ext: [false; K_UIEXT_COUNT],
        width: width as i32,
        height: height as i32,
        pum_nlines: 0,
        pum_pos: false,
        pum_row: -1.0,
        pum_col: -1.0,
        pum_height: 0.0,
        pum_width: 0.0,
        term_name: None,
        term_colors: 0,
        stdin_tty: false,
        stdout_tty: false,
        channel_id,
        packer: crate::msgpack_rpc::packer::PackerBuffer::new(),
        cur_event: None,
        nevents_pos: None,
        ncalls_pos: None,
        nevents: 0,
        ncalls: 0,
        flushed_events: false,
        incomplete_event: false,
        ncells_pending: 0,
        hl_id: 0,
        cursor_row: 0,
        cursor_col: 0,
        client_row: 0,
        client_col: -1,
        wildmenu_active: false,
    });
    let ui: &mut RemoteUI = &mut ui_box;

    for i in 0..options.size {
        let item = options[i];
        ui_set_option(ui, true, item.key, item.value, err);
        if error_set(err) {
            return;
        }
    }

    if ui.ui_ext[K_UIHL_STATE] || ui.ui_ext[K_UIMULTIGRID] {
        ui.ui_ext[K_UILINEGRID] = true;
    }
    if ui.ui_ext[K_UIMESSAGES] {
        ui.ui_ext[K_UILINEGRID] = true;
        ui.ui_ext[K_UICMDLINE] = true;
    }

    ui.cur_event = None;
    ui.hl_id = 0;
    ui.client_col = -1;
    ui.nevents_pos = None;
    ui.nevents = 0;
    ui.flushed_events = false;
    ui.incomplete_event = false;
    ui.ncalls_pos = None;
    ui.ncalls = 0;
    ui.ncells_pending = 0;
    api_ui_st().connected_uis.insert(channel_id, ui_box);
    let ui: &mut RemoteUI = api_ui_st().connected_uis.get_mut(&channel_id).unwrap();
    ui.packer = PackerBuffer {
        startptr: ptr::null_mut(),
        ptr: ptr::null_mut(),
        endptr: ptr::null_mut(),
        anydata: crate::msgpack_rpc::packer::PackerData::Ui(&raw mut *ui),
        anyint: 0,
        owned_buf: Vec::new(),
    };
    ui.wildmenu_active = false;

    g_st().current_ui = channel_id;
    ui_attach_impl(ui, channel_id);

    let chan = find_channel(channel_id);
    if !chan.is_null() {
        unsafe {
            (*chan).rpc.has_ui = true;
        }
    }

    may_trigger_vim_suspend_resume(false);
}

/// @deprecated
// DLLEXPORT void ui_attach(uint64_t channel_id, Integer width, Integer height, Boolean enable_rgb,
//                          Error *err) FUNC_API_DEPRECATED_SINCE(1);
pub fn ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    enable_rgb: Boolean,
    err: &mut Error,
) {
    let mut opts_items: [KeyValuePair; 1] = [KeyValuePair {
        key: cstr_as_string(c"rgb"),
        value: boolean_obj(enable_rgb),
    }];
    let opts = Dict {
        items: opts_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    nvim_ui_attach(channel_id, width, height, opts, err);
}

/// Tells the nvim server if focus was gained or lost by the GUI
// DLLEXPORT void nvim_ui_set_focus(uint64_t channel_id, Boolean gained, Error *error)
//   FUNC_API_SINCE(11) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_set_focus(channel_id: u64, gained: Boolean, error: &mut Error) {
    if get_ui_or_err(channel_id, error).is_none() {
        return;
    }
    if gained {
        g_st().current_ui = channel_id;
        may_trigger_vim_suspend_resume(false);
    }
    do_autocmd_focusgained(gained);
}

/// Deactivates UI events on the channel.
///
/// Removes the client from the list of UIs. |nvim_list_uis()|
///
/// @param channel_id
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_detach(uint64_t channel_id, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_detach(channel_id: u64, err: &mut Error) {
    remote_ui_disconnect(channel_id, err, false);
}

// DLLEXPORT void remote_ui_connect(uint64_t channel_id, char *server_addr, Error *err);
pub fn remote_ui_connect(
    channel_id: u64,
    server_addr: impl crate::strings::IntoCCharPtr,
    err: &mut Error,
) {
    let server_addr = server_addr.into_c_char_ptr();
    let Some(ui) = get_ui_or_err(channel_id, err) else {
        return;
    };
    let mut args_items: [Object; 1] = [string_obj(cstr_as_string(server_addr))];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"connect", args);
}

/// Sends a "restart" event to all UIs. (Not autogenerated because `chan_config` is per-channel.)
// DLLEXPORT void remote_ui_restart(String listen_addr);
pub fn remote_ui_restart(listen_addr: NvimString) {
    for ui in api_ui_st().connected_uis.values_mut() {
        let chan = find_channel(ui.channel_id);
        let detach_val = !chan.is_null() && unsafe { (*chan).detach };
        let mut cfg_items: [KeyValuePair; 1] = [KeyValuePair {
            key: cstr_as_string(c"detach"),
            value: boolean_obj(detach_val),
        }];
        let chan_config = Dict {
            items: cfg_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        let mut args_items: [Object; 2] = [string_obj(listen_addr), dict_obj(chan_config)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"restart", args);
    }
}

// DLLEXPORT void nvim_ui_try_resize(uint64_t channel_id, Integer width, Integer height, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_try_resize(channel_id: u64, width: Integer, height: Integer, err: &mut Error) {
    let Some(ui) = get_ui_or_err(channel_id, err) else {
        return;
    };
    if width <= 0 || height <= 0 {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Expected width > 0 and height > 0",
        );
        return;
    }
    ui.width = width as i32;
    ui.height = height as i32;
    ui_refresh();
}

// DLLEXPORT void nvim_ui_set_option(uint64_t channel_id, String name, Object value, Error *error)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_set_option(channel_id: u64, name: NvimString, value: Object, error: &mut Error) {
    let Some(ui) = get_ui_or_err(channel_id, error) else {
        return;
    };
    ui_set_option(ui, false, name, value, error);
}

fn ui_set_option(ui: &mut RemoteUI, init: bool, name: NvimString, value: Object, err: &mut Error) {
    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"override") {
        if value.obj_type != ObjectType::ObjectTypeBoolean {
            api_err_exp(
                err,
                c"override",
                api_typename(ObjectType::ObjectTypeBoolean),
                api_typename(value.obj_type),
            );
            return;
        }
        ui.r#override = value.data.boolean();
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"rgb") {
        if value.obj_type != ObjectType::ObjectTypeBoolean {
            api_err_exp(
                err,
                c"rgb",
                api_typename(ObjectType::ObjectTypeBoolean),
                api_typename(value.obj_type),
            );
            return;
        }
        ui.rgb = value.data.boolean();
        if !init && !ui.ui_ext[K_UILINEGRID] {
            ui_refresh();
        }
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"term_name") {
        if value.obj_type != ObjectType::ObjectTypeString {
            api_err_exp(
                err,
                c"term_name",
                api_typename(ObjectType::ObjectTypeString),
                api_typename(value.obj_type),
            );
            return;
        }
        let s = value.data.string();
        let slice = if s.data.is_null() || s.size == 0 {
            &[][..]
        } else {
            crate::memory::cstr_slice(s.data, s.size)
        };
        let term_cstr = std::ffi::CString::new(slice).unwrap();
        set_tty_option(c"term", Some(term_cstr.clone()));
        ui.term_name = Some(term_cstr);
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"term_colors") {
        if value.obj_type != ObjectType::ObjectTypeInteger {
            api_err_exp(
                err,
                c"term_colors",
                api_typename(ObjectType::ObjectTypeInteger),
                api_typename(value.obj_type),
            );
            return;
        }
        g_st().t_colors = value.data.integer() as i32;
        ui.term_colors = value.data.integer() as i32;
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"stdin_fd") {
        if value.obj_type != ObjectType::ObjectTypeInteger {
            api_err_exp(
                err,
                c"stdin_fd",
                api_typename(ObjectType::ObjectTypeInteger),
                api_typename(value.obj_type),
            );
            return;
        }
        if value.data.integer() < 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid stdin_fd: %lld",
                value.data.integer(),
            );
            return;
        }
        if starting_val() != NO_SCREEN {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"stdin_fd can only be used with first attached UI",
            );
            return;
        }
        g_st().stdin_fd = value.data.integer() as i32;
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"stdin_tty") {
        if value.obj_type != ObjectType::ObjectTypeBoolean {
            api_err_exp(
                err,
                c"stdin_tty",
                api_typename(ObjectType::ObjectTypeBoolean),
                api_typename(value.obj_type),
            );
            return;
        }
        if ui.channel_id == CHAN_STDIO {
            g_st().stdin_isatty = value.data.boolean();
        }
        ui.stdin_tty = value.data.boolean();
        return;
    }

    if crate::memory::cstr_as_opt_bytes(name.data) == Some(b"stdout_tty") {
        if value.obj_type != ObjectType::ObjectTypeBoolean {
            api_err_exp(
                err,
                c"stdout_tty",
                api_typename(ObjectType::ObjectTypeBoolean),
                api_typename(value.obj_type),
            );
            return;
        }
        if ui.channel_id == CHAN_STDIO {
            g_st().stdout_isatty = value.data.boolean();
        }
        ui.stdout_tty = value.data.boolean();
        return;
    }

    let is_popupmenu = crate::memory::cstr_as_opt_bytes(name.data) == Some(b"popupmenu_external");
    for (i, ext_name) in UI_EXT_NAMES.iter().enumerate().take(K_UIEXT_COUNT) {
        if (crate::memory::cstr_as_opt_bytes(name.data)
            == crate::memory::cstr_as_opt_bytes(ext_name.as_ptr()))
            || (i == K_UIPOPUPMENU && is_popupmenu)
        {
            if value.obj_type != ObjectType::ObjectTypeBoolean {
                api_err_exp(err, name.data, c"Boolean", api_typename(value.obj_type));
                return;
            }
            let boolval = value.data.boolean();
            if !init && i == K_UILINEGRID && boolval != ui.ui_ext[i] {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"ext_linegrid option cannot be changed",
                );
            }
            ui.ui_ext[i] = boolval;
            if !init {
                ui_set_ext_option(ui, i as UIExtension, boolval);
            }
            return;
        }
    }

    api_set_error!(
        err,
        ErrorType::ErrorTypeValidation,
        c"Invalid UI option: '%s'",
        name.data,
    );
}

/// Tell Nvim to resize a grid. Triggers a grid_resize event with the requested
/// grid size or the maximum size if it exceeds size limits.
///
/// On invalid grid handle, fails with error.
///
/// @param channel_id
/// @param grid    The handle of the grid to be changed.
/// @param width   The new requested width.
/// @param height  The new requested height.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_try_resize_grid(uint64_t channel_id, Integer grid, Integer width,
//                                        Integer height, Error *err)
//   FUNC_API_SINCE(6) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_try_resize_grid(
    channel_id: u64,
    grid: Integer,
    width: Integer,
    height: Integer,
    err: &mut Error,
) {
    if get_ui_or_err(channel_id, err).is_none() {
        return;
    }
    if grid == Integer::from(crate::drawscreen::DEFAULT_GRID_HANDLE) {
        nvim_ui_try_resize(channel_id, width, height, err);
    } else {
        ui_grid_resize(grid as ObjHandle, width as i32, height as i32, err);
    }
}

/// Tells Nvim the number of elements displaying in the popupmenu, to decide
/// [<PageUp>] and [<PageDown>] movement.
///
/// @param channel_id
/// @param height  Popupmenu height, must be greater than zero.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_pum_set_height(uint64_t channel_id, Integer height, Error *err)
//   FUNC_API_SINCE(6) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_pum_set_height(channel_id: u64, height: Integer, err: &mut Error) {
    let Some(ui) = get_ui_or_err(channel_id, err) else {
        return;
    };
    if height <= 0 {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Expected pum height > 0",
        );
        return;
    }
    if !ui.ui_ext[K_UIPOPUPMENU] {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"UI must support the ext_popupmenu option",
        );
        return;
    }
    ui.pum_nlines = height as i32;
}

/// Tells Nvim the geometry of the popupmenu, to align floating windows with an
/// external popup menu.
///
/// Note that this method is not to be confused with |nvim_ui_pum_set_height()|,
/// which sets the number of visible items in the popup menu, while this
/// function sets the bounding box of the popup menu, including visual
/// elements such as borders and sliders. Floats need not use the same font
/// size, nor be anchored to exact grid corners, so one can set floating-point
/// numbers to the popup menu geometry.
///
/// @param channel_id
/// @param width   Popupmenu width.
/// @param height  Popupmenu height.
/// @param row     Popupmenu row.
/// @param col     Popupmenu height.
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_ui_pum_set_bounds(uint64_t channel_id, Float width, Float height, Float row,
//                                       Float col, Error *err)
//   FUNC_API_SINCE(7) FUNC_API_REMOTE_ONLY;
pub fn nvim_ui_pum_set_bounds(
    channel_id: u64,
    width: Float,
    height: Float,
    row: Float,
    col: Float,
    err: &mut Error,
) {
    let Some(ui) = get_ui_or_err(channel_id, err) else {
        return;
    };
    if !ui.ui_ext[K_UIPOPUPMENU] {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"UI must support the ext_popupmenu option",
        );
        return;
    }
    if width <= 0.0 {
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected width > 0",);
        return;
    } else if height <= 0.0 {
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected height > 0",);
        return;
    }
    ui.pum_row = row;
    ui.pum_col = col;
    ui.pum_width = width;
    ui.pum_height = height;
    ui.pum_pos = true;
}

fn flush_event(ui: &mut RemoteUI) {
    if ui.cur_event.is_some() {
        if let Some(off) = ui.ncalls_pos {
            let mut pos = ui.packer.startptr.wrapping_add(off);
            mpack_w2(&raw mut pos, 1 + ui.ncalls);
        }
        ui.cur_event = None;
        ui.ncalls_pos = None;
        ui.ncalls = 0;
    }
}

fn ui_alloc_buf(ui: &mut RemoteUI) {
    ui.packer.owned_buf = vec![0; UI_BUF_SIZE];
    ui.packer.startptr = ui.packer.owned_buf.as_mut_ptr();
    ui.packer.ptr = ui.packer.startptr;
    ui.packer.endptr = ui.packer.startptr.wrapping_add(UI_BUF_SIZE);
}

fn prepare_call(ui: &mut RemoteUI, name: &'static CStr) {
    if !ui.packer.startptr.is_null()
        && (buf_pos(ui) > UI_BUF_SIZE - EVENT_BUF_SIZE || ui.ncells_pending >= 500)
    {
        ui_flush_buf(ui, false);
    }

    if ui.packer.startptr.is_null() {
        ui_alloc_buf(ui);
    }

    if ui.cur_event != Some(name) {
        let buf = &raw mut ui.packer.ptr;
        if ui.nevents_pos.is_none() {
            mpack_array(buf, 3);
            mpack_uint(buf, 2);
            mpack_str_small(buf, c"redraw", 6);
            let pos = mpack_array_dyn16(buf);
            ui.nevents_pos = Some(crate::memory::ptr_offset_from(pos, ui.packer.startptr) as usize);
            debug_assert!(ui.cur_event.is_none());
        }
        flush_event(ui);
        ui.cur_event = Some(name);
        let pos = mpack_array_dyn16(buf);
        ui.ncalls_pos = Some(crate::memory::ptr_offset_from(pos, ui.packer.startptr) as usize);
        mpack_str_small(buf, name.as_ptr(), name.to_bytes().len());
        ui.nevents += 1;
        ui.ncalls = 1;
    } else {
        ui.ncalls += 1;
    }
}

/// Pushes data into RemoteUI, to be consumed later by remote_ui_flush().
// DLLEXPORT void push_call(RemoteUI *ui, const char *name, Array args);
pub fn push_call(ui: &mut RemoteUI, name: &'static CStr, args: Array) {
    prepare_call(ui, name);
    mpack_object_array(args, &mut ui.packer);
}

pub(crate) fn ui_flush_callback(packer: &mut PackerBuffer) {
    let crate::msgpack_rpc::packer::PackerData::Ui(ui) = packer.anydata else {
        unreachable!()
    };
    unsafe {
        if !packer.startptr.is_null() && packer.ptr != packer.startptr {
            (*ui).incomplete_event = true;
            if (*ui).cur_event.is_some() {
                if let Some(off) = (*ui).ncalls_pos {
                    let mut pos = packer.startptr.wrapping_add(off);
                    mpack_w2(&raw mut pos, 1 + (*ui).ncalls);
                }
                (*ui).cur_event = None;
                (*ui).ncalls_pos = None;
                (*ui).ncalls = 0;
            }
            if let Some(off) = (*ui).nevents_pos {
                let mut pos = packer.startptr.wrapping_add(off);
                mpack_w2(&raw mut pos, (*ui).nevents);
                (*ui).nevents = 0;
                (*ui).nevents_pos = None;
            }
            let len = crate::memory::ptr_offset_from(packer.ptr, packer.startptr) as usize;
            let mut vec = std::mem::take(&mut packer.owned_buf);
            vec.truncate(len);
            let buf = wstream_new_vec_buffer(vec);
            rpc_write_raw((*ui).channel_id, buf);
            (*ui).flushed_events = true;
            (*ui).ncells_pending = 0;
        }
        packer.owned_buf = vec![0; UI_BUF_SIZE];
        packer.startptr = packer.owned_buf.as_mut_ptr();
        packer.ptr = packer.startptr;
        packer.endptr = packer.startptr.wrapping_add(UI_BUF_SIZE);
    }
}

// DLLEXPORT void remote_ui_grid_clear(RemoteUI *ui, Integer grid);
pub fn remote_ui_grid_clear(ui: &mut RemoteUI, grid: Integer) {
    let mut args_items: [Object; 1] = [integer_obj(grid)];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: usize::from(ui.ui_ext[K_UILINEGRID]),
        capacity: 1,
    };
    let name = if ui.ui_ext[K_UILINEGRID] {
        c"grid_clear"
    } else {
        c"clear"
    };
    push_call(ui, name, args);
}

// DLLEXPORT void remote_ui_grid_resize(RemoteUI *ui, Integer grid, Integer width, Integer height);
pub fn remote_ui_grid_resize(ui: &mut RemoteUI, grid: Integer, width: Integer, height: Integer) {
    let mut args_items = [Object::default(); 3];
    let mut size = 0;
    if ui.ui_ext[K_UILINEGRID] {
        args_items[size] = integer_obj(grid);
        size += 1;
    } else {
        ui.client_col = -1;
    }
    args_items[size] = integer_obj(width);
    size += 1;
    args_items[size] = integer_obj(height);
    size += 1;
    let args = Array {
        items: args_items.as_mut_ptr(),
        size,
        capacity: 3,
    };
    let name = if ui.ui_ext[K_UILINEGRID] {
        c"grid_resize"
    } else {
        c"resize"
    };
    push_call(ui, name, args);
}

// DLLEXPORT void remote_ui_grid_scroll(RemoteUI *ui, Integer grid, Integer top, Integer bot,
//                                      Integer left, Integer right, Integer rows, Integer cols);
pub fn remote_ui_grid_scroll(
    ui: &mut RemoteUI,
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    if ui.ui_ext[K_UILINEGRID] {
        let mut args_items: [Object; 7] = [
            integer_obj(grid),
            integer_obj(top),
            integer_obj(bot),
            integer_obj(left),
            integer_obj(right),
            integer_obj(rows),
            integer_obj(cols),
        ];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 7,
            capacity: 7,
        };
        push_call(ui, c"grid_scroll", args);
    } else {
        let mut args_items: [Object; 4] = [
            integer_obj(top),
            integer_obj(bot - 1),
            integer_obj(left),
            integer_obj(right - 1),
        ];
        push_call(
            ui,
            c"set_scroll_region",
            Array {
                items: args_items.as_mut_ptr(),
                size: 4,
                capacity: 4,
            },
        );

        args_items[0] = integer_obj(rows);
        push_call(
            ui,
            c"scroll",
            Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 4,
            },
        );

        args_items[0] = integer_obj(0);
        args_items[1] = integer_obj(Integer::from(ui.height - 1));
        args_items[2] = integer_obj(0);
        args_items[3] = integer_obj(Integer::from(ui.width - 1));
        push_call(
            ui,
            c"set_scroll_region",
            Array {
                items: args_items.as_mut_ptr(),
                size: 4,
                capacity: 4,
            },
        );
    }
}

// DLLEXPORT void remote_ui_default_colors_set(RemoteUI *ui, Integer rgb_fg, Integer rgb_bg,
//                                             Integer rgb_sp, Integer cterm_fg, Integer cterm_bg);
pub fn remote_ui_default_colors_set(
    ui: &mut RemoteUI,
    mut rgb_fg: Integer,
    mut rgb_bg: Integer,
    mut rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    if !ui.ui_ext[K_UITERM_COLORS] {
        let dark = crate::memory::cstr_first_byte(opt_vars().p_bg) == b'd';
        if rgb_fg == -1 {
            rgb_fg = if dark { 0xFFFFFF } else { 0x000000 };
        }
        if rgb_bg == -1 {
            rgb_bg = if dark { 0x000000 } else { 0xFFFFFF };
        }
        if rgb_sp == -1 {
            rgb_sp = 0xFF0000;
        }
    }
    let mut args_items: [Object; 5] = [
        integer_obj(rgb_fg),
        integer_obj(rgb_bg),
        integer_obj(rgb_sp),
        integer_obj(cterm_fg),
        integer_obj(cterm_bg),
    ];
    push_call(
        ui,
        c"default_colors_set",
        Array {
            items: args_items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        },
    );

    if !ui.ui_ext[K_UILINEGRID] {
        args_items[0] = integer_obj(if ui.rgb { rgb_fg } else { cterm_fg - 1 });
        push_call(
            ui,
            c"update_fg",
            Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 5,
            },
        );

        args_items[0] = integer_obj(if ui.rgb { rgb_bg } else { cterm_bg - 1 });
        push_call(
            ui,
            c"update_bg",
            Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 5,
            },
        );

        args_items[0] = integer_obj(if ui.rgb { rgb_sp } else { -1 });
        push_call(
            ui,
            c"update_sp",
            Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 5,
            },
        );
    }
}

// DLLEXPORT void remote_ui_hl_attr_define(RemoteUI *ui, Integer id, HlAttrs rgb_attrs,
//                                         HlAttrs cterm_attrs, Array info);
pub fn remote_ui_hl_attr_define(
    ui: &mut RemoteUI,
    id: Integer,
    rgb_attrs: HlAttrs,
    _cterm_attrs: HlAttrs,
    info: Array,
) {
    if !ui.ui_ext[K_UILINEGRID] {
        return;
    }

    let mut rgb_items = [KeyValuePair::default(); HLATTRS_DICT_SIZE];
    let mut rgb = Dict {
        items: rgb_items.as_mut_ptr(),
        size: 0,
        capacity: HLATTRS_DICT_SIZE,
    };
    let mut cterm_items = [KeyValuePair::default(); HLATTRS_DICT_SIZE];
    let mut cterm = Dict {
        items: cterm_items.as_mut_ptr(),
        size: 0,
        capacity: HLATTRS_DICT_SIZE,
    };
    hlattrs2dict(&mut rgb, None, rgb_attrs, true, false);
    hlattrs2dict(&mut cterm, None, rgb_attrs, false, false);

    if rgb_attrs.url >= 0 {
        let url = hl_get_url(rgb_attrs.url as u32);
        *rgb.next_slot_mut() = KeyValuePair {
            key: cstr_as_string(c"url"),
            value: string_obj(cstr_as_string(url)),
        };
        rgb.size += 1;
    }

    let mut args_items: [Object; 4] = [
        integer_obj(id),
        dict_obj(rgb),
        dict_obj(cterm),
        array_obj(if ui.ui_ext[K_UIHL_STATE] {
            info
        } else {
            ARRAY_INIT
        }),
    ];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 4,
        capacity: 4,
    };
    push_call(ui, c"hl_attr_define", args);
}

// DLLEXPORT void remote_ui_highlight_set(RemoteUI *ui, int id);
pub fn remote_ui_highlight_set(ui: &mut RemoteUI, id: i32) {
    if ui.hl_id == id {
        return;
    }
    ui.hl_id = id;
    let mut dict_items = [KeyValuePair::default(); HLATTRS_DICT_SIZE];
    let mut dict = Dict {
        items: dict_items.as_mut_ptr(),
        size: 0,
        capacity: HLATTRS_DICT_SIZE,
    };
    hlattrs2dict(&mut dict, None, syn_attr2entry(id), ui.rgb, false);
    let mut args_items: [Object; 1] = [dict_obj(dict)];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"highlight_set", args);
}

/// "true" cursor used only for input focus
// DLLEXPORT void remote_ui_grid_cursor_goto(RemoteUI *ui, Integer grid, Integer row, Integer col);
pub fn remote_ui_grid_cursor_goto(ui: &mut RemoteUI, grid: Integer, row: Integer, col: Integer) {
    if ui.ui_ext[K_UILINEGRID] {
        let mut args_items: [Object; 3] = [integer_obj(grid), integer_obj(row), integer_obj(col)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };
        push_call(ui, c"grid_cursor_goto", args);
    } else {
        ui.cursor_row = row;
        ui.cursor_col = col;
        remote_ui_cursor_goto(ui, row, col);
    }
}

/// emulated cursor used both for drawing and for input focus
// DLLEXPORT void remote_ui_cursor_goto(RemoteUI *ui, Integer row, Integer col);
pub fn remote_ui_cursor_goto(ui: &mut RemoteUI, row: Integer, col: Integer) {
    if ui.client_row == row && ui.client_col == col {
        return;
    }
    ui.client_row = row;
    ui.client_col = col;
    let mut args_items: [Object; 2] = [integer_obj(row), integer_obj(col)];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"cursor_goto", args);
}

// DLLEXPORT void remote_ui_put(RemoteUI *ui, const char *cell);
pub fn remote_ui_put(ui: &mut RemoteUI, cell: impl crate::strings::IntoCCharPtr) {
    let cell = cell.into_c_char_ptr();
    ui.client_col += 1;
    let mut args_items: [Object; 1] = [string_obj(cstr_as_string(cell))];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"put", args);
}

// DLLEXPORT void remote_ui_raw_line(RemoteUI *ui, Integer grid, Integer row, Integer startcol,
//                                   Integer endcol, Integer clearcol, Integer clearattr,
//                                   LineFlags flags, const Schar *chunk, const Sattr *attrs);
pub fn remote_ui_raw_line(
    ui: &mut RemoteUI,
    grid: Integer,
    row: Integer,
    startcol: Integer,
    endcol: Integer,
    clearcol: Integer,
    clearattr: Integer,
    flags: i32,
    chunk: *const Schar,
    attrs: *const Sattr,
) {
    unsafe {
        if ui.ui_ext[K_UILINEGRID] {
            prepare_call(ui, c"grid_line");

            let buf = &raw mut ui.packer.ptr;
            mpack_array(buf, 5);
            mpack_uint(buf, grid as u32);
            mpack_uint(buf, row as u32);
            mpack_uint(buf, startcol as u32);
            let mut lenpos = mpack_array_dyn16(buf);

            let mut repeat: u32 = 0;
            let ncells = (endcol - startcol) as usize;
            let mut last_hl: i32 = -1;
            let mut nelem: u32 = 0;
            let mut was_space = false;

            for i in 0..ncells {
                repeat += 1;
                if i == ncells - 1
                    || *attrs.wrapping_add(i) != *attrs.wrapping_add(i + 1)
                    || *chunk.wrapping_add(i) != *chunk.wrapping_add(i + 1)
                {
                    if UI_BUF_SIZE - buf_pos(ui) < 2 * (1 + 2 + MAX_SCHAR_SIZE + 5 + 5) + 1
                        || ui.ncells_pending >= 500
                    {
                        if was_space {
                            nelem += 1;
                            ui.ncells_pending += 1;
                            mpack_array(buf, 3);
                            mpack_str_small(buf, c" ", 1);
                            mpack_uint(buf, clearattr as u32);
                            mpack_uint(buf, 0);
                        }
                        mpack_w2(&raw mut lenpos, nelem);
                        mpack_bool(buf, false);
                        ui_flush_buf(ui, false);

                        prepare_call(ui, c"grid_line");
                        mpack_array(buf, 5);
                        mpack_uint(buf, grid as u32);
                        mpack_uint(buf, row as u32);
                        mpack_uint(buf, (startcol as u32) + (i as u32) - repeat + 1);
                        lenpos = mpack_array_dyn16(buf);
                        nelem = 0;
                        last_hl = -1;
                    }
                    let csize: u32 = if repeat > 1 {
                        3
                    } else if (*attrs.wrapping_add(i)) != last_hl {
                        2
                    } else {
                        1
                    };
                    nelem += 1;
                    mpack_array(buf, csize);
                    let size_byte = *buf;
                    *buf = (*buf).wrapping_add(1);
                    let len = schar_get_adv(buf.cast::<*mut c_char>(), *chunk.wrapping_add(i));
                    *size_byte = (0xa0 | len) as u8;
                    if csize >= 2 {
                        mpack_uint(buf, *attrs.wrapping_add(i) as u32);
                        if csize >= 3 {
                            mpack_uint(buf, repeat);
                        }
                    }
                    ui.ncells_pending += repeat.min(2) as usize;
                    last_hl = *attrs.wrapping_add(i);
                    repeat = 0;
                    was_space = *chunk.wrapping_add(i) == schar_from_ascii(b' ');
                }
            }

            if endcol < clearcol || was_space {
                nelem += 1;
                ui.ncells_pending += 1;
                mpack_array(buf, 3);
                mpack_str_small(buf, c" ", 1);
                mpack_uint(buf, clearattr as u32);
                mpack_uint(buf, (clearcol - endcol) as u32);
            }
            mpack_w2(&raw mut lenpos, nelem);
            mpack_bool(buf, (flags & K_LINE_FLAG_WRAP) != 0);
        } else {
            let count = (endcol - startcol) as usize;
            for i in 0..count {
                remote_ui_cursor_goto(ui, row, startcol + i as Integer);
                remote_ui_highlight_set(ui, *attrs.wrapping_add(i));
                let mut sc_buf: [c_char; MAX_SCHAR_SIZE] = [0; MAX_SCHAR_SIZE];
                schar_get(sc_buf.as_mut_ptr(), *chunk.wrapping_add(i));
                remote_ui_put(ui, sc_buf.as_ptr());
                if utf_ambiguous_width(sc_buf.as_ptr()) {
                    ui.client_col = -1;
                }
            }
            if endcol < clearcol {
                remote_ui_cursor_goto(ui, row, endcol);
                remote_ui_highlight_set(ui, clearattr as i32);
                if clearattr == 0 && clearcol == Integer::from(g_st().columns_global) {
                    push_call(ui, c"eol_clear", ARRAY_INIT);
                } else {
                    let mut c = endcol;
                    while c < clearcol {
                        remote_ui_put(ui, c" ");
                        c += 1;
                    }
                }
            }
        }
    }
}

fn ui_flush_buf(ui: &mut RemoteUI, incomplete_event: bool) {
    if ui.packer.startptr.is_null() || buf_pos(ui) == 0 {
        return;
    }
    ui.incomplete_event = incomplete_event;

    flush_event(ui);
    if let Some(off) = ui.nevents_pos {
        let mut pos = ui.packer.startptr.wrapping_add(off);
        mpack_w2(&raw mut pos, ui.nevents);
        ui.nevents = 0;
        ui.nevents_pos = None;
    }

    let len = buf_pos(ui);
    let mut vec = std::mem::take(&mut ui.packer.owned_buf);
    vec.truncate(len);
    let buf = wstream_new_vec_buffer(vec);
    rpc_write_raw(ui.channel_id, buf);

    ui.packer.startptr = ptr::null_mut();
    ui.packer.ptr = ptr::null_mut();
    ui.packer.endptr = ptr::null_mut();

    ui.flushed_events = true;
    ui.ncells_pending = 0;
}

pub(crate) fn remote_ui_flush_if_incomplete(channel_id: u64) {
    if let Some(ui) = get_ui_or_err(channel_id, &mut Error::new())
        && ui.incomplete_event
    {
        remote_ui_flush_pending_data(ui);
    }
}

/// An intentional flush (vsync) when Nvim is finished redrawing the screen
///
/// Clients can know this happened by a final "flush" event at the end of the
/// "redraw" batch.
// DLLEXPORT void remote_ui_flush(RemoteUI *ui);
pub fn remote_ui_flush(ui: &mut RemoteUI) {
    if ui.nevents > 0 || ui.flushed_events {
        if !ui.ui_ext[K_UILINEGRID] {
            remote_ui_cursor_goto(ui, ui.cursor_row, ui.cursor_col);
        }
        push_call(ui, c"flush", ARRAY_INIT);
        ui_flush_buf(ui, false);
        ui.flushed_events = false;
    }
}

// DLLEXPORT void remote_ui_ui_send(RemoteUI *ui, String content);
pub fn remote_ui_ui_send(ui: &mut RemoteUI, content: NvimString) {
    if !ui.stdout_tty {
        return;
    }
    let mut args_items: [Object; 1] = [string_obj(content)];
    let args = Array {
        items: args_items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"ui_send", args);
}

// DLLEXPORT void remote_ui_flush_pending_data(RemoteUI *ui);
pub fn remote_ui_flush_pending_data(ui: &mut RemoteUI) {
    ui_flush_buf(ui, false);
}

fn translate_contents(ui: &mut RemoteUI, contents: Array, arena: &mut Arena) -> Array {
    let mut new_contents = arena_array(arena, contents.size);
    for i in 0..contents.size {
        let item = contents[i].data.array();
        let mut new_item = arena_array(arena, 2);
        let attr = item[0].data.integer() as i32;
        if attr != 0 {
            let mut rgb_attrs = arena_dict(arena, HLATTRS_DICT_SIZE);
            hlattrs2dict(&mut rgb_attrs, None, syn_attr2entry(attr), ui.rgb, false);
            new_item[0] = dict_obj(rgb_attrs);
        } else {
            new_item[0] = dict_obj(DICT_INIT);
        }
        new_item[1] = item[1];
        new_item.size = 2;
        new_contents[i] = array_obj(new_item);
    }
    new_contents.size = contents.size;
    new_contents
}

fn translate_firstarg(ui: &mut RemoteUI, args: Array, arena: &mut Arena) -> Array {
    let mut new_args = arena_array(arena, args.size);
    let contents = args[0].data.array();
    new_args[0] = array_obj(translate_contents(ui, contents, arena));
    for i in 1..args.size {
        new_args[i] = args[i];
    }
    new_args.size = args.size;
    new_args
}

// DLLEXPORT void remote_ui_event(RemoteUI *ui, char *name, Array args);
pub fn remote_ui_event(ui: &mut RemoteUI, mut name: &'static CStr, args: Array) {
    let mut arena = Arena {
        cur_blk: ptr::null_mut(),
        pos: 0,
        size: 0,
    };
    if !ui.ui_ext[K_UILINEGRID] {
        if name == c"cmdline_show" {
            let new_args = translate_firstarg(ui, args, &mut arena);
            push_call(ui, name, new_args);
            arena_mem_free(arena_finish(&mut arena));
            return;
        } else if name == c"cmdline_block_show" {
            let block = args[0].data.array();
            let mut new_block = arena_array(&mut arena, block.size);
            for i in 0..block.size {
                new_block[i] = array_obj(translate_contents(ui, block[i].data.array(), &mut arena));
            }
            new_block.size = block.size;
            let mut new_args_items: [Object; 1] = [array_obj(new_block)];
            let new_args = Array {
                items: new_args_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            push_call(ui, name, new_args);
            arena_mem_free(arena_finish(&mut arena));
            return;
        } else if name == c"cmdline_block_append" {
            let new_args = translate_firstarg(ui, args, &mut arena);
            push_call(ui, name, new_args);
            arena_mem_free(arena_finish(&mut arena));
            return;
        }
    }

    if ui.ui_ext[K_UIWILDMENU] {
        if name == c"popupmenu_show" {
            ui.wildmenu_active = args[4].data.integer() == -1 || !ui.ui_ext[K_UIPOPUPMENU];
            if ui.wildmenu_active {
                let items = args[0].data.array();
                let mut new_items = arena_array(&mut arena, items.size);
                for i in 0..items.size {
                    new_items[i] = unsafe { *items[i].data.array().items };
                }
                new_items.size = items.size;
                let mut new_args_items: [Object; 1] = [array_obj(new_items)];
                push_call(
                    ui,
                    c"wildmenu_show",
                    Array {
                        items: new_args_items.as_mut_ptr(),
                        size: 1,
                        capacity: 1,
                    },
                );
                if args[1].data.integer() != -1 {
                    new_args_items[0] = args[1];
                    push_call(
                        ui,
                        c"wildmenu_select",
                        Array {
                            items: new_args_items.as_mut_ptr(),
                            size: 1,
                            capacity: 1,
                        },
                    );
                }
                arena_mem_free(arena_finish(&mut arena));
                return;
            }
        } else if name == c"popupmenu_select" {
            if ui.wildmenu_active {
                name = c"wildmenu_select";
            }
        } else if name == c"popupmenu_hide" && ui.wildmenu_active {
            name = c"wildmenu_hide";
        }
    }

    push_call(ui, name, args);
}

/// Sends arbitrary data to a UI. Use this instead of |nvim_chan_send()| or `io.stdout:write()`, if
/// you really want to write to the |TUI| host terminal.
///
/// Emits a "ui_send" event to all UIs with the "stdout_tty" |ui-option| set. UIs are expected to
/// write the received data to a connected TTY if one exists.
///
/// @param channel_id
/// @param content Content to write to the TTY
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_send(uint64_t channel_id, String content, Error *err) FUNC_API_SINCE(14);
pub fn nvim_ui_send(_channel_id: u64, content: NvimString, _err: &mut Error) {
    ui_call_ui_send(content);
}

pub fn remote_ui_mode_info_set(ui: &mut RemoteUI, enabled: Boolean, cursor_styles: Array) {
    let mut items = [boolean_obj(enabled), array_obj(cursor_styles)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"mode_info_set", args);
}

pub fn remote_ui_update_menu(ui: &mut RemoteUI) {
    push_call(ui, c"update_menu", ARRAY_INIT);
}

pub fn remote_ui_busy_start(ui: &mut RemoteUI) {
    push_call(ui, c"busy_start", ARRAY_INIT);
}

pub fn remote_ui_busy_stop(ui: &mut RemoteUI) {
    push_call(ui, c"busy_stop", ARRAY_INIT);
}

pub fn remote_ui_mouse_on(ui: &mut RemoteUI) {
    push_call(ui, c"mouse_on", ARRAY_INIT);
}

pub fn remote_ui_mouse_off(ui: &mut RemoteUI) {
    push_call(ui, c"mouse_off", ARRAY_INIT);
}

pub fn remote_ui_mode_change(ui: &mut RemoteUI, mode: NvimString, mode_idx: Integer) {
    let mut items = [string_obj(mode), integer_obj(mode_idx)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"mode_change", args);
}

pub fn remote_ui_bell(ui: &mut RemoteUI) {
    push_call(ui, c"bell", ARRAY_INIT);
}

pub fn remote_ui_visual_bell(ui: &mut RemoteUI) {
    push_call(ui, c"visual_bell", ARRAY_INIT);
}

pub fn remote_ui_suspend(ui: &mut RemoteUI) {
    push_call(ui, c"suspend", ARRAY_INIT);
}

pub fn remote_ui_set_title(ui: &mut RemoteUI, title: NvimString) {
    let mut items = [string_obj(title)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"set_title", args);
}

pub fn remote_ui_set_icon(ui: &mut RemoteUI, icon: NvimString) {
    let mut items = [string_obj(icon)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"set_icon", args);
}

pub fn remote_ui_screenshot(ui: &mut RemoteUI, path: NvimString) {
    let mut items = [string_obj(path)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"screenshot", args);
}

pub fn remote_ui_option_set(ui: &mut RemoteUI, name: NvimString, value: Object) {
    let mut items = [string_obj(name), value];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"option_set", args);
}

pub fn remote_ui_chdir(ui: &mut RemoteUI, path: NvimString) {
    let mut items = [string_obj(path)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"chdir", args);
}

pub fn remote_ui_hl_group_set(ui: &mut RemoteUI, name: NvimString, id: Integer) {
    let mut items = [string_obj(name), integer_obj(id)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"hl_group_set", args);
}

pub fn remote_ui_msg_set_pos(
    ui: &mut RemoteUI,
    grid: Integer,
    row: Integer,
    scrolled: Boolean,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    let mut items = [
        integer_obj(grid),
        integer_obj(row),
        boolean_obj(scrolled),
        string_obj(sep_char),
        integer_obj(zindex),
        integer_obj(compindex),
    ];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 6,
        capacity: 6,
    };
    push_call(ui, c"msg_set_pos", args);
}

pub fn remote_ui_win_viewport(
    ui: &mut RemoteUI,
    grid: Integer,
    win: ObjHandle,
    topline: Integer,
    botline: Integer,
    curline: Integer,
    curcol: Integer,
    line_count: Integer,
    scroll_delta: Integer,
) {
    let mut items = [
        integer_obj(grid),
        window_obj(win),
        integer_obj(topline),
        integer_obj(botline),
        integer_obj(curline),
        integer_obj(curcol),
        integer_obj(line_count),
        integer_obj(scroll_delta),
    ];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 8,
        capacity: 8,
    };
    push_call(ui, c"win_viewport", args);
}

pub fn remote_ui_win_viewport_margins(
    ui: &mut RemoteUI,
    grid: Integer,
    win: ObjHandle,
    top: Integer,
    bottom: Integer,
    left: Integer,
    right: Integer,
) {
    let mut items = [
        integer_obj(grid),
        window_obj(win),
        integer_obj(top),
        integer_obj(bottom),
        integer_obj(left),
        integer_obj(right),
    ];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 6,
        capacity: 6,
    };
    push_call(ui, c"win_viewport_margins", args);
}

pub fn remote_ui_error_exit(ui: &mut RemoteUI, status: Integer) {
    let mut items = [integer_obj(status)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 1,
        capacity: 1,
    };
    push_call(ui, c"error_exit", args);
}

pub fn remote_ui_set_restart_on_crash_exit(ui: &mut RemoteUI, progpath: NvimString, argv: Array) {
    let mut items = [string_obj(progpath), array_obj(argv)];
    let args = Array {
        items: items.as_mut_ptr(),
        size: 2,
        capacity: 2,
    };
    push_call(ui, c"_set_restart_on_crash_exit", args);
}
