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
use crate::grid::{sattr_T, schar_T, schar_from_ascii, schar_get, schar_get_adv};
use crate::highlight::{HlAttrs, hl_get_url, hlattrs2dict, syn_attr2entry};
use crate::lua::converter::{Boolean, Float};
use crate::mbyte::utf_ambiguous_width;
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::channel::{RemoteUI, find_channel, rpc_write_raw};
use crate::msgpack_rpc::packer::{
    Integer, PackerBuffer, handle_T, mpack_array, mpack_bool, mpack_object_array, mpack_uint,
    mpack_w,
};
use crate::option::set_tty_option;
use crate::option_vars::*;
use crate::ui::{
    ui_active, ui_attach_impl, ui_call_ui_send, ui_can_attach_more, ui_detach_impl, ui_grid_resize,
    ui_refresh, ui_set_ext_option,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

#[cfg(windows)]
use crate::channel::kChannelStreamStdio;
#[cfg(windows)]
use crate::os::os_win_console::os_detach_ui_console;

pub(crate) const MAX_SCHAR_SIZE: usize = 32;
const UI_BUF_SIZE: usize = 4096;
const EVENT_BUF_SIZE: usize = 256;
pub(crate) const HLATTRS_DICT_SIZE: usize = 24;
pub(crate) const NO_SCREEN: c_int = 2;
pub(crate) const CHAN_STDIO: u64 = 1;

pub(crate) const kChannelPartAll: c_int = 4;

pub(crate) const kUICmdline: usize = 0;
const kUIPopupmenu: usize = 1;
const kUIWildmenu: usize = 3;
pub(crate) const kUIMessages: usize = 4;
pub(crate) const kUILinegrid: usize = 5;
pub(crate) const kUIMultigrid: usize = 6;
pub(crate) const kUIHlState: usize = 7;
pub(crate) const kUITermColors: usize = 8;
pub(crate) const kUIExtCount: usize = 10;

pub(crate) const kLineFlagWrap: c_int = 1;

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

static API_UI_STATE: crate::globals::GlobalState<ApiUiState> = crate::globals::GlobalState::new(ApiUiState::new());

#[inline]
unsafe fn api_ui_st() -> &'static mut ApiUiState {
    unsafe { &mut *API_UI_STATE.as_ptr() }
}

use crate::msgpack_rpc::packer::mpack_w2;

#[inline]
unsafe fn buf_pos(ui: &RemoteUI) -> usize {
    unsafe { ui.packer.ptr.offset_from(ui.packer.startptr) as usize }
}

unsafe fn get_ui_or_err(chan_id: u64, err: *mut Error) -> Option<&'static mut RemoteUI> {
    unsafe {
        let uis_ptr = &raw mut api_ui_st().connected_uis;
        let ui = (*uis_ptr).get_mut(&chan_id).map(|b| &mut **b);
        if ui.is_none() && !err.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"UI not attached to channel: %lld".as_ptr(),
                chan_id as i64,
            );
        }
        ui
    }
}

unsafe fn mpack_array_dyn16(buf: *mut *mut c_char) -> *mut c_char {
    unsafe {
        mpack_w(buf, 0xdc);
        let pos = *buf;
        mpack_w2(buf, 0xFFEF);
        pos
    }
}

unsafe fn mpack_str_small(buf: *mut *mut c_char, str_: *const c_char, len: usize) {
    unsafe {
        debug_assert!(len < 0x20);
        mpack_w(buf, (0xa0 | len) as u8);
        ptr::copy_nonoverlapping(str_, *buf, len);
        *buf = (*buf).add(len);
    }
}

/// Removes the client on the given channel from the list of UIs.
///
/// @param err  if non-NULL and there is no UI on the channel, set an error
/// @param send_error_exit  send an "error_exit" event with 0 status first
// DLLEXPORT void remote_ui_disconnect(uint64_t channel_id, Error *err, bool send_error_exit);
pub unsafe fn remote_ui_disconnect(channel_id: u64, err: *mut Error, send_error_exit: bool) {
    unsafe {
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
        let uis_ptr = &raw mut api_ui_st().connected_uis;
        let mut ui_box = (*uis_ptr).remove(&channel_id);
        let ui_detached = ui_box.as_deref_mut().unwrap_or(ui);
        ui_detach_impl(ui_detached, channel_id);
        let chan = find_channel(channel_id);
        if !chan.is_null() {
            (*chan).rpc.has_ui = false;
        }
        drop(ui_box);
    }
}

/// Detaches the UI on channel `chan` (server keeps running). Sets `channel.detach`, sends an
/// "error_exit" UI event, and closes the channel.
///
/// @param chan  UI channel to disconnect.
/// @param[out] err Error details, if any.
// DLLEXPORT void ui_detach_channel(uint64_t chan, Error *err);
pub unsafe fn ui_detach_channel(chan: u64, err: *mut Error) {
    unsafe {
        let c = find_channel(chan);
        if c.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                (&raw const e_invchan).cast::<c_char>(),
            );
            return;
        }
        #[cfg(windows)]
        let detach_stdio = (*c).streamtype == kChannelStreamStdio;
        (*c).detach = true;
        remote_ui_disconnect(chan, err, true);
        if error_set(err) {
            return;
        }
        channel_close(chan, kChannelPartAll, ptr::null_mut());
        #[cfg(windows)]
        {
            os_detach_ui_console(detach_stdio);
        }
    }
}

/// Detaches every UI except `keep_chan` (server keeps running).
///
/// @param keep_chan  UI channel to preserve.
/// @return Number of UIs detached.
// DLLEXPORT size_t ui_detach_others(uint64_t keep_chan);
pub unsafe fn ui_detach_others(keep_chan: u64) -> usize {
    unsafe {
        let mut chans = Vec::<u64>::new();
        let uis_ptr = &raw const api_ui_st().connected_uis;
        for &chan in (*uis_ptr).keys() {
            if chan != keep_chan {
                chans.push(chan);
            }
        }

        let mut detached: usize = 0;
        for chan in chans {
            let mut err = Error::new();
            ui_detach_channel(chan, &raw mut err);
            if error_set(&raw const err) {
                api_clear_error(&raw mut err);
            } else {
                detached += 1;
            }
        }

        detached
    }
}

// DLLEXPORT void remote_ui_free_all_mem(void);
pub unsafe fn remote_ui_free_all_mem() {
    unsafe {
        let uis_ptr = &raw mut api_ui_st().connected_uis;
        (*uis_ptr).clear();
    }
}

/// Wait until UI has connected.
// DLLEXPORT void remote_ui_wait_for_attach(void);
pub unsafe fn remote_ui_wait_for_attach() {
    unsafe {
        while ui_active() == 0 {
            let events = (*loop_queues((&raw mut g_st().main_loop).cast())).events;
            if !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events((&raw mut g_st().main_loop).cast(), -1);
                multiqueue_process_events(events);
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    options: Dict,
    err: *mut Error,
) {
    unsafe {
        let uis_ptr = &raw const api_ui_st().connected_uis;
        if (*uis_ptr).contains_key(&channel_id) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"UI already attached to channel: %lld".as_ptr(),
                channel_id as i64,
            );
            return;
        }
        if !ui_can_attach_more() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Maximum UI count reached".as_ptr(),
            );
            return;
        }
        if width <= 0 || height <= 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected width > 0 and height > 0".as_ptr(),
            );
            return;
        }

        let mut ui_box = Box::new(RemoteUI {
            rgb: true,
            r#override: false,
            composed: false,
            ui_ext: [false; kUIExtCount],
            width: width as c_int,
            height: height as c_int,
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
        let ui: &mut RemoteUI = &mut *ui_box;

        for i in 0..options.size {
            let item = options[i];
            ui_set_option(ui, true, item.key, item.value, err);
            if error_set(err) {
                return;
            }
        }

        if ui.ui_ext[kUIHlState] || ui.ui_ext[kUIMultigrid] {
            ui.ui_ext[kUILinegrid] = true;
        }
        if ui.ui_ext[kUIMessages] {
            ui.ui_ext[kUILinegrid] = true;
            ui.ui_ext[kUICmdline] = true;
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
        let uis_ptr = &raw mut api_ui_st().connected_uis;
        (*uis_ptr).insert(channel_id, ui_box);
        let ui: &mut RemoteUI = &mut **(*uis_ptr).get_mut(&channel_id).unwrap();
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
            (*chan).rpc.has_ui = true;
        }

        may_trigger_vim_suspend_resume(false);
    }
}

/// @deprecated
// DLLEXPORT void ui_attach(uint64_t channel_id, Integer width, Integer height, Boolean enable_rgb,
//                          Error *err) FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    enable_rgb: Boolean,
    err: *mut Error,
) {
    unsafe {
        let mut opts_items: [KeyValuePair; 1] = [KeyValuePair {
            key: cstr_as_string(c"rgb".as_ptr()),
            value: boolean_obj(enable_rgb),
        }];
        let opts = Dict {
            items: opts_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        nvim_ui_attach(channel_id, width, height, opts, err);
    }
}

/// Tells the nvim server if focus was gained or lost by the GUI
// DLLEXPORT void nvim_ui_set_focus(uint64_t channel_id, Boolean gained, Error *error)
//   FUNC_API_SINCE(11) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_set_focus(channel_id: u64, gained: Boolean, error: *mut Error) {
    unsafe {
        if get_ui_or_err(channel_id, error).is_none() {
            return;
        }
        if gained {
            g_st().current_ui = channel_id;
            may_trigger_vim_suspend_resume(false);
        }
        do_autocmd_focusgained(gained);
    }
}

/// Deactivates UI events on the channel.
///
/// Removes the client from the list of UIs. |nvim_list_uis()|
///
/// @param channel_id
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_ui_detach(uint64_t channel_id, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_detach(channel_id: u64, err: *mut Error) {
    unsafe {
        remote_ui_disconnect(channel_id, err, false);
    }
}

// DLLEXPORT void remote_ui_connect(uint64_t channel_id, char *server_addr, Error *err);
pub unsafe fn remote_ui_connect(channel_id: u64, server_addr: *mut c_char, err: *mut Error) {
    unsafe {
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
}

/// Sends a "restart" event to all UIs. (Not autogenerated because `chan_config` is per-channel.)
// DLLEXPORT void remote_ui_restart(String listen_addr);
pub unsafe fn remote_ui_restart(listen_addr: NvimString) {
    unsafe {
        let uis_ptr = &raw mut api_ui_st().connected_uis;
        for ui in (*uis_ptr).values_mut() {
            let chan = find_channel(ui.channel_id);
            let detach_val = !chan.is_null() && (*chan).detach;
            let mut cfg_items: [KeyValuePair; 1] = [KeyValuePair {
                key: cstr_as_string(c"detach".as_ptr()),
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
            push_call(&mut **ui, c"restart", args);
        }
    }
}

// DLLEXPORT void nvim_ui_try_resize(uint64_t channel_id, Integer width, Integer height, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_try_resize(
    channel_id: u64,
    width: Integer,
    height: Integer,
    err: *mut Error,
) {
    unsafe {
        let Some(ui) = get_ui_or_err(channel_id, err) else {
            return;
        };
        if width <= 0 || height <= 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected width > 0 and height > 0".as_ptr(),
            );
            return;
        }
        ui.width = width as c_int;
        ui.height = height as c_int;
        ui_refresh();
    }
}

// DLLEXPORT void nvim_ui_set_option(uint64_t channel_id, String name, Object value, Error *error)
//   FUNC_API_SINCE(1) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_set_option(
    channel_id: u64,
    name: NvimString,
    value: Object,
    error: *mut Error,
) {
    unsafe {
        let Some(ui) = get_ui_or_err(channel_id, error) else {
            return;
        };
        ui_set_option(ui, false, name, value, error);
    }
}

unsafe fn ui_set_option(
    ui: &mut RemoteUI,
    init: bool,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"override" } {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"override".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            ui.r#override = value.data.boolean();
            return;
        }

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"rgb" } {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"rgb".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            ui.rgb = value.data.boolean();
            if !init && !ui.ui_ext[kUILinegrid] {
                ui_refresh();
            }
            return;
        }

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"term_name" } {
            if value.obj_type != ObjectType::kObjectTypeString {
                api_err_exp(
                    err,
                    c"term_name".as_ptr(),
                    api_typename(ObjectType::kObjectTypeString),
                    api_typename(value.obj_type),
                );
                return;
            }
            let s = value.data.string();
            let slice = if s.data.is_null() || s.size == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(s.data.cast::<u8>(), s.size)
            };
            let term_cstr = std::ffi::CString::new(slice).unwrap();
            set_tty_option(c"term".as_ptr(), Some(term_cstr.clone()));
            ui.term_name = Some(term_cstr);
            return;
        }

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"term_colors" } {
            if value.obj_type != ObjectType::kObjectTypeInteger {
                api_err_exp(
                    err,
                    c"term_colors".as_ptr(),
                    api_typename(ObjectType::kObjectTypeInteger),
                    api_typename(value.obj_type),
                );
                return;
            }
            g_st().t_colors = value.data.integer() as c_int;
            ui.term_colors = value.data.integer() as c_int;
            return;
        }

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"stdin_fd" } {
            if value.obj_type != ObjectType::kObjectTypeInteger {
                api_err_exp(
                    err,
                    c"stdin_fd".as_ptr(),
                    api_typename(ObjectType::kObjectTypeInteger),
                    api_typename(value.obj_type),
                );
                return;
            }
            if value.data.integer() < 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid stdin_fd: %lld".as_ptr(),
                    value.data.integer(),
                );
                return;
            }
            if *starting.as_ptr() != NO_SCREEN {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"stdin_fd can only be used with first attached UI".as_ptr(),
                );
                return;
            }
            g_st().stdin_fd = value.data.integer() as c_int;
            return;
        }

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"stdin_tty" } {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"stdin_tty".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
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

        if { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"stdout_tty" } {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"stdout_tty".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
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

        let is_popupmenu = { let _a = name.data; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"popupmenu_external" };
        for (i, ext_name) in ui_ext_names.iter().enumerate().take(kUIExtCount) {
            if { let _a = name.data; let _b = ext_name.as_ptr(); (_a.is_null() && _b.is_null()) || (!_a.is_null() && !_b.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == std::ffi::CStr::from_ptr(_b).to_bytes()) } || (i == kUIPopupmenu && is_popupmenu) {
                if value.obj_type != ObjectType::kObjectTypeBoolean {
                    api_err_exp(
                        err,
                        name.data,
                        c"Boolean".as_ptr(),
                        api_typename(value.obj_type),
                    );
                    return;
                }
                let boolval = value.data.boolean();
                if !init && i == kUILinegrid && boolval != ui.ui_ext[i] {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"ext_linegrid option cannot be changed".as_ptr(),
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
            ErrorType::kErrorTypeValidation,
            c"Invalid UI option: '%s'".as_ptr(),
            name.data,
        );
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_try_resize_grid(
    channel_id: u64,
    grid: Integer,
    width: Integer,
    height: Integer,
    err: *mut Error,
) {
    unsafe {
        if get_ui_or_err(channel_id, err).is_none() {
            return;
        }
        if grid == Integer::from(crate::drawscreen::DEFAULT_GRID_HANDLE) {
            nvim_ui_try_resize(channel_id, width, height, err);
        } else {
            ui_grid_resize(grid as handle_T, width as c_int, height as c_int, err);
        }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_pum_set_height(channel_id: u64, height: Integer, err: *mut Error) {
    unsafe {
        let Some(ui) = get_ui_or_err(channel_id, err) else {
            return;
        };
        if height <= 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected pum height > 0".as_ptr(),
            );
            return;
        }
        if !ui.ui_ext[kUIPopupmenu] {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"UI must support the ext_popupmenu option".as_ptr(),
            );
            return;
        }
        ui.pum_nlines = height as c_int;
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_pum_set_bounds(
    channel_id: u64,
    width: Float,
    height: Float,
    row: Float,
    col: Float,
    err: *mut Error,
) {
    unsafe {
        let Some(ui) = get_ui_or_err(channel_id, err) else {
            return;
        };
        if !ui.ui_ext[kUIPopupmenu] {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"UI must support the ext_popupmenu option".as_ptr(),
            );
            return;
        }
        if width <= 0.0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected width > 0".as_ptr(),
            );
            return;
        } else if height <= 0.0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected height > 0".as_ptr(),
            );
            return;
        }
        ui.pum_row = row;
        ui.pum_col = col;
        ui.pum_width = width;
        ui.pum_height = height;
        ui.pum_pos = true;
    }
}

unsafe fn flush_event(ui: &mut RemoteUI) {
    unsafe {
        if ui.cur_event.is_some() {
            if let Some(off) = ui.ncalls_pos {
                let mut pos = ui.packer.startptr.add(off);
                mpack_w2(&raw mut pos, 1 + ui.ncalls);
            }
            ui.cur_event = None;
            ui.ncalls_pos = None;
            ui.ncalls = 0;
        }
    }
}

unsafe fn ui_alloc_buf(ui: &mut RemoteUI) {
    unsafe {
        ui.packer.owned_buf = vec![0; UI_BUF_SIZE];
        ui.packer.startptr = ui.packer.owned_buf.as_mut_ptr();
        ui.packer.ptr = ui.packer.startptr;
        ui.packer.endptr = ui.packer.startptr.add(UI_BUF_SIZE);
    }
}

unsafe fn prepare_call(ui: &mut RemoteUI, name: &'static CStr) {
    unsafe {
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
                mpack_str_small(buf, c"redraw".as_ptr(), 6);
                let pos = mpack_array_dyn16(buf);
                ui.nevents_pos = Some(pos.offset_from(ui.packer.startptr) as usize);
                debug_assert!(ui.cur_event.is_none());
            }
            flush_event(ui);
            ui.cur_event = Some(name);
            let pos = mpack_array_dyn16(buf);
            ui.ncalls_pos = Some(pos.offset_from(ui.packer.startptr) as usize);
            mpack_str_small(buf, name.as_ptr(), name.to_bytes().len());
            ui.nevents += 1;
            ui.ncalls = 1;
        } else {
            ui.ncalls += 1;
        }
    }
}

/// Pushes data into RemoteUI, to be consumed later by remote_ui_flush().
// DLLEXPORT void push_call(RemoteUI *ui, const char *name, Array args);
pub unsafe fn push_call(ui: &mut RemoteUI, name: &'static CStr, args: Array) {
    unsafe {
        prepare_call(ui, name);
        mpack_object_array(args, &raw mut ui.packer);
    }
}

pub(crate) unsafe fn ui_flush_callback(packer: *mut PackerBuffer) {
    unsafe {
        let ui = &mut *(*packer).anydata.as_ui();
        ui_flush_buf(ui, true);
        ui_alloc_buf(ui);
    }
}

// DLLEXPORT void remote_ui_grid_clear(RemoteUI *ui, Integer grid);
pub unsafe fn remote_ui_grid_clear(ui: &mut RemoteUI, grid: Integer) {
    unsafe {
        let mut args_items: [Object; 1] = [integer_obj(grid)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: usize::from(ui.ui_ext[kUILinegrid]),
            capacity: 1,
        };
        let name = if ui.ui_ext[kUILinegrid] {
            c"grid_clear"
        } else {
            c"clear"
        };
        push_call(ui, name, args);
    }
}

// DLLEXPORT void remote_ui_grid_resize(RemoteUI *ui, Integer grid, Integer width, Integer height);
pub unsafe fn remote_ui_grid_resize(
    ui: &mut RemoteUI,
    grid: Integer,
    width: Integer,
    height: Integer,
) {
    unsafe {
        let mut args_items = [Object::default(); 3];
        let mut size = 0;
        if ui.ui_ext[kUILinegrid] {
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
        let name = if ui.ui_ext[kUILinegrid] {
            c"grid_resize"
        } else {
            c"resize"
        };
        push_call(ui, name, args);
    }
}

// DLLEXPORT void remote_ui_grid_scroll(RemoteUI *ui, Integer grid, Integer top, Integer bot,
//                                      Integer left, Integer right, Integer rows, Integer cols);
pub unsafe fn remote_ui_grid_scroll(
    ui: &mut RemoteUI,
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    unsafe {
        if ui.ui_ext[kUILinegrid] {
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
}

// DLLEXPORT void remote_ui_default_colors_set(RemoteUI *ui, Integer rgb_fg, Integer rgb_bg,
//                                             Integer rgb_sp, Integer cterm_fg, Integer cterm_bg);
pub unsafe fn remote_ui_default_colors_set(
    ui: &mut RemoteUI,
    mut rgb_fg: Integer,
    mut rgb_bg: Integer,
    mut rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    unsafe {
        if !ui.ui_ext[kUITermColors] {
            let dark = *opt_vars().p_bg == b'd' as c_char;
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

        if !ui.ui_ext[kUILinegrid] {
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
}

// DLLEXPORT void remote_ui_hl_attr_define(RemoteUI *ui, Integer id, HlAttrs rgb_attrs,
//                                         HlAttrs cterm_attrs, Array info);
pub unsafe fn remote_ui_hl_attr_define(
    ui: &mut RemoteUI,
    id: Integer,
    rgb_attrs: HlAttrs,
    _cterm_attrs: HlAttrs,
    info: Array,
) {
    unsafe {
        if !ui.ui_ext[kUILinegrid] {
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
        hlattrs2dict(&raw mut rgb, ptr::null_mut(), rgb_attrs, true, false);
        hlattrs2dict(&raw mut cterm, ptr::null_mut(), rgb_attrs, false, false);

        if rgb_attrs.url >= 0 {
            let url = hl_get_url(rgb_attrs.url as u32);
            *rgb.items.add(rgb.size) = KeyValuePair {
                key: cstr_as_string(c"url".as_ptr()),
                value: string_obj(cstr_as_string(url)),
            };
            rgb.size += 1;
        }

        let mut args_items: [Object; 4] = [
            integer_obj(id),
            dict_obj(rgb),
            dict_obj(cterm),
            array_obj(if ui.ui_ext[kUIHlState] {
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
}

// DLLEXPORT void remote_ui_highlight_set(RemoteUI *ui, int id);
pub unsafe fn remote_ui_highlight_set(ui: &mut RemoteUI, id: c_int) {
    unsafe {
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
        hlattrs2dict(
            &raw mut dict,
            ptr::null_mut(),
            syn_attr2entry(id),
            ui.rgb,
            false,
        );
        let mut args_items: [Object; 1] = [dict_obj(dict)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"highlight_set", args);
    }
}

/// "true" cursor used only for input focus
// DLLEXPORT void remote_ui_grid_cursor_goto(RemoteUI *ui, Integer grid, Integer row, Integer col);
pub unsafe fn remote_ui_grid_cursor_goto(
    ui: &mut RemoteUI,
    grid: Integer,
    row: Integer,
    col: Integer,
) {
    unsafe {
        if ui.ui_ext[kUILinegrid] {
            let mut args_items: [Object; 3] =
                [integer_obj(grid), integer_obj(row), integer_obj(col)];
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
}

/// emulated cursor used both for drawing and for input focus
// DLLEXPORT void remote_ui_cursor_goto(RemoteUI *ui, Integer row, Integer col);
pub unsafe fn remote_ui_cursor_goto(ui: &mut RemoteUI, row: Integer, col: Integer) {
    unsafe {
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
}

// DLLEXPORT void remote_ui_put(RemoteUI *ui, const char *cell);
pub unsafe fn remote_ui_put(ui: &mut RemoteUI, cell: *const c_char) {
    unsafe {
        ui.client_col += 1;
        let mut args_items: [Object; 1] = [string_obj(cstr_as_string(cell))];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"put", args);
    }
}

// DLLEXPORT void remote_ui_raw_line(RemoteUI *ui, Integer grid, Integer row, Integer startcol,
//                                   Integer endcol, Integer clearcol, Integer clearattr,
//                                   LineFlags flags, const schar_T *chunk, const sattr_T *attrs);
pub unsafe fn remote_ui_raw_line(
    ui: &mut RemoteUI,
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
        if ui.ui_ext[kUILinegrid] {
            prepare_call(ui, c"grid_line");

            let buf = &raw mut ui.packer.ptr;
            mpack_array(buf, 5);
            mpack_uint(buf, grid as u32);
            mpack_uint(buf, row as u32);
            mpack_uint(buf, startcol as u32);
            let mut lenpos = mpack_array_dyn16(buf);

            let mut repeat: u32 = 0;
            let ncells = (endcol - startcol) as usize;
            let mut last_hl: c_int = -1;
            let mut nelem: u32 = 0;
            let mut was_space = false;

            for i in 0..ncells {
                repeat += 1;
                if i == ncells - 1
                    || *attrs.add(i) != *attrs.add(i + 1)
                    || *chunk.add(i) != *chunk.add(i + 1)
                {
                    if UI_BUF_SIZE - buf_pos(ui) < 2 * (1 + 2 + MAX_SCHAR_SIZE + 5 + 5) + 1
                        || ui.ncells_pending >= 500
                    {
                        if was_space {
                            nelem += 1;
                            ui.ncells_pending += 1;
                            mpack_array(buf, 3);
                            mpack_str_small(buf, c" ".as_ptr(), 1);
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
                    } else if c_int::from(*attrs.add(i)) != last_hl {
                        2
                    } else {
                        1
                    };
                    nelem += 1;
                    mpack_array(buf, csize);
                    let size_byte = *buf;
                    *buf = (*buf).add(1);
                    let len = schar_get_adv(buf, *chunk.add(i));
                    *size_byte = (0xa0 | len) as c_char;
                    if csize >= 2 {
                        mpack_uint(buf, *attrs.add(i) as u32);
                        if csize >= 3 {
                            mpack_uint(buf, repeat);
                        }
                    }
                    ui.ncells_pending += repeat.min(2) as usize;
                    last_hl = c_int::from(*attrs.add(i));
                    repeat = 0;
                    was_space = *chunk.add(i) == schar_from_ascii(b' ');
                }
            }

            if endcol < clearcol || was_space {
                nelem += 1;
                ui.ncells_pending += 1;
                mpack_array(buf, 3);
                mpack_str_small(buf, c" ".as_ptr(), 1);
                mpack_uint(buf, clearattr as u32);
                mpack_uint(buf, (clearcol - endcol) as u32);
            }
            mpack_w2(&raw mut lenpos, nelem);
            mpack_bool(buf, (flags & kLineFlagWrap) != 0);
        } else {
            let count = (endcol - startcol) as usize;
            for i in 0..count {
                remote_ui_cursor_goto(ui, row, startcol + i as Integer);
                remote_ui_highlight_set(ui, c_int::from(*attrs.add(i)));
                let mut sc_buf: [c_char; MAX_SCHAR_SIZE] = [0; MAX_SCHAR_SIZE];
                schar_get(sc_buf.as_mut_ptr(), *chunk.add(i));
                remote_ui_put(ui, sc_buf.as_ptr());
                if utf_ambiguous_width(sc_buf.as_ptr()) {
                    ui.client_col = -1;
                }
            }
            if endcol < clearcol {
                remote_ui_cursor_goto(ui, row, endcol);
                remote_ui_highlight_set(ui, clearattr as c_int);
                if clearattr == 0 && clearcol == Integer::from(g_st().Columns) {
                    push_call(ui, c"eol_clear", ARRAY_INIT);
                } else {
                    let mut c = endcol;
                    while c < clearcol {
                        remote_ui_put(ui, c" ".as_ptr());
                        c += 1;
                    }
                }
            }
        }
    }
}

unsafe fn ui_flush_buf(ui: &mut RemoteUI, incomplete_event: bool) {
    unsafe {
        if ui.packer.startptr.is_null() || buf_pos(ui) == 0 {
            return;
        }
        ui.incomplete_event = incomplete_event;

        flush_event(ui);
        if let Some(off) = ui.nevents_pos {
            let mut pos = ui.packer.startptr.add(off);
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
}

pub(crate) unsafe fn remote_ui_flush_if_incomplete(channel_id: u64) {
    unsafe {
        if let Some(ui) = get_ui_or_err(channel_id, ptr::null_mut())
            && ui.incomplete_event
        {
            remote_ui_flush_pending_data(ui);
        }
    }
}

/// An intentional flush (vsync) when Nvim is finished redrawing the screen
///
/// Clients can know this happened by a final "flush" event at the end of the
/// "redraw" batch.
// DLLEXPORT void remote_ui_flush(RemoteUI *ui);
pub unsafe fn remote_ui_flush(ui: &mut RemoteUI) {
    unsafe {
        if ui.nevents > 0 || ui.flushed_events {
            if !ui.ui_ext[kUILinegrid] {
                remote_ui_cursor_goto(ui, ui.cursor_row, ui.cursor_col);
            }
            push_call(ui, c"flush", ARRAY_INIT);
            ui_flush_buf(ui, false);
            ui.flushed_events = false;
        }
    }
}

// DLLEXPORT void remote_ui_ui_send(RemoteUI *ui, String content);
pub unsafe fn remote_ui_ui_send(ui: &mut RemoteUI, content: NvimString) {
    unsafe {
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
}

// DLLEXPORT void remote_ui_flush_pending_data(RemoteUI *ui);
pub unsafe fn remote_ui_flush_pending_data(ui: &mut RemoteUI) {
    unsafe {
        ui_flush_buf(ui, false);
    }
}

unsafe fn translate_contents(ui: &mut RemoteUI, contents: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut new_contents = arena_array(arena, contents.size);
        for i in 0..contents.size {
            let item = contents[i].data.array();
            let mut new_item = arena_array(arena, 2);
            let attr = (*item.items).data.integer() as c_int;
            if attr != 0 {
                let mut rgb_attrs = arena_dict(arena, HLATTRS_DICT_SIZE);
                hlattrs2dict(
                    &raw mut rgb_attrs,
                    ptr::null_mut(),
                    syn_attr2entry(attr),
                    ui.rgb,
                    false,
                );
                *new_item.items.add(0) = dict_obj(rgb_attrs);
            } else {
                *new_item.items.add(0) = dict_obj(DICT_INIT);
            }
            *new_item.items.add(1) = item[1];
            new_item.size = 2;
            *new_contents.items.add(i) = array_obj(new_item);
        }
        new_contents.size = contents.size;
        new_contents
    }
}

unsafe fn translate_firstarg(ui: &mut RemoteUI, args: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut new_args = arena_array(arena, args.size);
        let contents = (*args.items).data.array();
        *new_args.items.add(0) = array_obj(translate_contents(ui, contents, arena));
        for i in 1..args.size {
            *new_args.items.add(i) = args[i];
        }
        new_args.size = args.size;
        new_args
    }
}

// DLLEXPORT void remote_ui_event(RemoteUI *ui, char *name, Array args);
pub unsafe fn remote_ui_event(ui: &mut RemoteUI, mut name: &'static CStr, args: Array) {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        if !ui.ui_ext[kUILinegrid] {
            if name == c"cmdline_show" {
                let new_args = translate_firstarg(ui, args, &raw mut arena);
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            } else if name == c"cmdline_block_show" {
                let block = (*args.items).data.array();
                let mut new_block = arena_array(&raw mut arena, block.size);
                for i in 0..block.size {
                    *new_block.items.add(i) = array_obj(translate_contents(
                        ui,
                        block[i].data.array(),
                        &raw mut arena,
                    ));
                }
                new_block.size = block.size;
                let mut new_args_items: [Object; 1] = [array_obj(new_block)];
                let new_args = Array {
                    items: new_args_items.as_mut_ptr(),
                    size: 1,
                    capacity: 1,
                };
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            } else if name == c"cmdline_block_append" {
                let new_args = translate_firstarg(ui, args, &raw mut arena);
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            }
        }

        if ui.ui_ext[kUIWildmenu] {
            if name == c"popupmenu_show" {
                ui.wildmenu_active =
                    args[4].data.integer() == -1 || !ui.ui_ext[kUIPopupmenu];
                if ui.wildmenu_active {
                    let items = (*args.items).data.array();
                    let mut new_items = arena_array(&raw mut arena, items.size);
                    for i in 0..items.size {
                        *new_items.items.add(i) = *items[i].data.array().items;
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
                    arena_mem_free(arena_finish(&raw mut arena));
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_send(_channel_id: u64, content: NvimString, _err: *mut Error) {
    unsafe {
        ui_call_ui_send(content);
    }
}

pub unsafe fn remote_ui_mode_info_set(ui: &mut RemoteUI, enabled: Boolean, cursor_styles: Array) {
    unsafe {
        let mut items = [boolean_obj(enabled), array_obj(cursor_styles)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"mode_info_set", args);
    }
}

pub unsafe fn remote_ui_update_menu(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"update_menu", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_busy_start(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"busy_start", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_busy_stop(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"busy_stop", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_mouse_on(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"mouse_on", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_mouse_off(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"mouse_off", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_mode_change(ui: &mut RemoteUI, mode: NvimString, mode_idx: Integer) {
    unsafe {
        let mut items = [string_obj(mode), integer_obj(mode_idx)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"mode_change", args);
    }
}

pub unsafe fn remote_ui_bell(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"bell", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_visual_bell(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"visual_bell", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_suspend(ui: &mut RemoteUI) {
    unsafe {
        push_call(ui, c"suspend", ARRAY_INIT);
    }
}

pub unsafe fn remote_ui_set_title(ui: &mut RemoteUI, title: NvimString) {
    unsafe {
        let mut items = [string_obj(title)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"set_title", args);
    }
}

pub unsafe fn remote_ui_set_icon(ui: &mut RemoteUI, icon: NvimString) {
    unsafe {
        let mut items = [string_obj(icon)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"set_icon", args);
    }
}

pub unsafe fn remote_ui_screenshot(ui: &mut RemoteUI, path: NvimString) {
    unsafe {
        let mut items = [string_obj(path)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"screenshot", args);
    }
}

pub unsafe fn remote_ui_option_set(ui: &mut RemoteUI, name: NvimString, value: Object) {
    unsafe {
        let mut items = [string_obj(name), value];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"option_set", args);
    }
}

pub unsafe fn remote_ui_chdir(ui: &mut RemoteUI, path: NvimString) {
    unsafe {
        let mut items = [string_obj(path)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"chdir", args);
    }
}

pub unsafe fn remote_ui_hl_group_set(ui: &mut RemoteUI, name: NvimString, id: Integer) {
    unsafe {
        let mut items = [string_obj(name), integer_obj(id)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"hl_group_set", args);
    }
}

pub unsafe fn remote_ui_msg_set_pos(
    ui: &mut RemoteUI,
    grid: Integer,
    row: Integer,
    scrolled: Boolean,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    unsafe {
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
}

pub unsafe fn remote_ui_win_viewport(
    ui: &mut RemoteUI,
    grid: Integer,
    win: handle_T,
    topline: Integer,
    botline: Integer,
    curline: Integer,
    curcol: Integer,
    line_count: Integer,
    scroll_delta: Integer,
) {
    unsafe {
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
}

pub unsafe fn remote_ui_win_viewport_margins(
    ui: &mut RemoteUI,
    grid: Integer,
    win: handle_T,
    top: Integer,
    bottom: Integer,
    left: Integer,
    right: Integer,
) {
    unsafe {
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
}

pub unsafe fn remote_ui_error_exit(ui: &mut RemoteUI, status: Integer) {
    unsafe {
        let mut items = [integer_obj(status)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"error_exit", args);
    }
}

pub unsafe fn remote_ui__set_restart_on_crash_exit(
    ui: &mut RemoteUI,
    progpath: NvimString,
    argv: Array,
) {
    unsafe {
        let mut items = [string_obj(progpath), array_obj(argv)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"_set_restart_on_crash_exit", args);
    }
}
