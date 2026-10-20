//! Remote UI API and msgpack redraw packing (`src/nvim/api/ui.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_assignments,
    unused_imports,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::api_typename;
use crate::errors::*;
use crate::globals::*;
#[allow(unused_imports)]
use crate::option::set_tty_option;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::ui::ui_call_ui_send;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::api::private::helpers::{
    api_clear_error, arena_array, arena_dict, cstr_as_string, string_to_cstr,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_exp,
};
use crate::api::win_config::UIExtension;
use crate::autocmd::{do_autocmd_focusgained, may_trigger_vim_suspend_resume};
use crate::channel::channel_close;
use crate::event::r#loop::{Loop, loop_poll_events, loop_queues};
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
use crate::event::wstream::wstream_new_buffer;
use crate::grid::{sattr_T, schar_T, schar_get, schar_get_adv};
use crate::highlight::{hl_get_url, hlattrs2dict, syn_attr2entry};
use crate::kvec::KVec;
use crate::lua::converter::{Boolean, Float};
use crate::map::{
    HlAttrs, Map, Set, map_del_uint64_t_ptr_t, map_put_ref_uint64_t_ptr_t, mh_get_uint64_t,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, MapHash, NvimString};
use crate::mbyte::utf_ambiguous_width;
use crate::memory::{
    Arena, alloc_block, arena_finish, arena_mem_free, free_block, strequal, xcalloc, xfree,
};
use crate::msgpack_rpc::channel::{
    RemoteUI, channel_prefix, channel_tail, find_channel, rpc_write_raw,
};
use crate::msgpack_rpc::packer::{
    Integer, PackerBuffer, handle_T, mpack_array, mpack_bool, mpack_object_array, mpack_uint,
    mpack_w,
};
#[cfg(windows)]
use crate::os::os_win_console::os_detach_ui_console;
use crate::ui::{
    ui_active, ui_attach_impl, ui_can_attach_more, ui_detach_impl, ui_grid_resize, ui_refresh,
    ui_set_ext_option,
};

const MAX_SCHAR_SIZE: usize = 32;
const UI_BUF_SIZE: usize = 4096;
const EVENT_BUF_SIZE: usize = 256;
const HLATTRS_DICT_SIZE: usize = 24;
const DEFAULT_GRID_HANDLE: Integer = 1;
const NO_SCREEN: c_int = 2;
const CHAN_STDIO: u64 = 1;

const kChannelStreamStdio: c_int = 2;
const kChannelPartAll: c_int = 4;

const kUICmdline: usize = 0;
const kUIPopupmenu: usize = 1;
const kUIWildmenu: usize = 3;
const kUIMessages: usize = 4;
const kUILinegrid: usize = 5;
const kUIMultigrid: usize = 6;
const kUIHlState: usize = 7;
const kUITermColors: usize = 8;
const kUIExtCount: usize = 10;

const kLineFlagWrap: c_int = 1;

const ARRAY_INIT: Array = Array {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

const DICT_INIT: Dict = Dict {
    items: ptr::null_mut(),
    size: 0,
    capacity: 0,
};

static mut connected_uis: Map<u64, *mut c_void> = Map {
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

#[inline]
const fn error_set(e: *const Error) -> bool {
    unsafe { !matches!((*e).err_type, ErrorType::kErrorTypeNone) }
}

#[inline]
const fn int_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn bool_obj(b: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

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
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
const fn schar_from_ascii(x: u8) -> schar_T {
    if cfg!(target_endian = "big") {
        (x as schar_T) << 24
    } else {
        x as schar_T
    }
}

#[inline]
unsafe fn mpack_w2(b: *mut *mut c_char, v: u32) {
    unsafe {
        **b = ((v >> 8) & 0xff) as c_char;
        *(*b).add(1) = (v & 0xff) as c_char;
        *b = (*b).add(2);
    }
}

unsafe extern "C" fn free_block_cb(block: *mut c_void) {
    unsafe {
        free_block(block.cast());
    }
}

#[inline]
unsafe fn buf_pos(ui: *const RemoteUI) -> usize {
    unsafe { (*ui).packer.ptr.offset_from((*ui).packer.startptr) as usize }
}

unsafe fn get_ui_or_err(chan_id: u64, err: *mut Error) -> *mut RemoteUI {
    unsafe {
        let k = mh_get_uint64_t(&raw mut connected_uis.set, chan_id);
        let ui = if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*connected_uis.values.add(k as usize)).cast::<RemoteUI>()
        };
        if ui.is_null() && !err.is_null() {
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

unsafe fn remote_ui_destroy(ui: *mut RemoteUI) {
    unsafe {
        xfree((*ui).packer.startptr.cast());
        xfree((*ui).term_name.cast());
        (*ui).term_name = ptr::null_mut();
        xfree(ui.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_disconnect(
    channel_id: u64,
    err: *mut Error,
    send_error_exit: bool,
) {
    unsafe {
        let ui = get_ui_or_err(channel_id, err);
        if ui.is_null() {
            return;
        }
        if send_error_exit {
            let mut args_items: [Object; 1] = [int_obj(0)];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            push_call(ui, c"error_exit".as_ptr(), args);
            ui_flush_buf(ui, false);
        }
        map_del_uint64_t_ptr_t(&raw mut connected_uis, channel_id, ptr::null_mut());
        ui_detach_impl(ui.cast(), channel_id);
        let chan = find_channel(channel_id);
        if !chan.is_null() && (*channel_tail(chan)).rpc.ui == ui {
            (*channel_tail(chan)).rpc.ui = ptr::null_mut();
        }

        remote_ui_destroy(ui);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_detach_channel(chan: u64, err: *mut Error) {
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
        let detach_stdio = (*channel_prefix(c)).streamtype == kChannelStreamStdio;
        (*channel_tail(c)).detach = true;
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_detach_others(keep_chan: u64) -> usize {
    unsafe {
        let mut chans = KVec::<u64>::new();
        for i in 0..connected_uis.set.h.n_keys as usize {
            let chan = *connected_uis.set.keys.add(i);
            let val = *connected_uis.values.add(i);
            if !val.is_null() && chan != keep_chan {
                chans.push(chan);
            }
        }

        let mut detached: usize = 0;
        for i in 0..chans.size {
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            ui_detach_channel(*chans.items.add(i), &raw mut err);
            if error_set(&raw const err) {
                api_clear_error(&raw mut err);
            } else {
                detached += 1;
            }
        }

        chans.destroy();
        detached
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_free_all_mem() {
    unsafe {
        for i in 0..connected_uis.set.h.n_keys as usize {
            let ui = (*connected_uis.values.add(i)).cast::<RemoteUI>();
            if !ui.is_null() {
                remote_ui_destroy(ui);
            }
        }
        xfree(connected_uis.set.keys.cast());
        xfree(connected_uis.set.h.hash.cast());
        connected_uis.set = Set {
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
        xfree(connected_uis.values.cast());
        connected_uis.values = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_wait_for_attach() {
    unsafe {
        while ui_active() == 0 {
            let events = (*loop_queues((&raw mut main_loop).cast())).events;
            if !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events((&raw mut main_loop).cast(), -1);
                multiqueue_process_events(events);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    options: Dict,
    err: *mut Error,
) {
    unsafe {
        if mh_get_uint64_t(&raw mut connected_uis.set, channel_id) != MH_TOMBSTONE {
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

        let ui = xcalloc(1, size_of::<RemoteUI>()).cast::<RemoteUI>();
        (*ui).channel_id = channel_id;
        (*ui).width = width as c_int;
        (*ui).height = height as c_int;
        (*ui).pum_row = -1.0;
        (*ui).pum_col = -1.0;
        (*ui).rgb = true;

        for i in 0..options.size {
            let item = *options.items.add(i);
            ui_set_option(ui, true, item.key, item.value, err);
            if error_set(err) {
                xfree(ui.cast());
                return;
            }
        }

        if (*ui).ui_ext[kUIHlState] || (*ui).ui_ext[kUIMultigrid] {
            (*ui).ui_ext[kUILinegrid] = true;
        }
        if (*ui).ui_ext[kUIMessages] {
            (*ui).ui_ext[kUILinegrid] = true;
            (*ui).ui_ext[kUICmdline] = true;
        }

        (*ui).cur_event = ptr::null();
        (*ui).hl_id = 0;
        (*ui).client_col = -1;
        (*ui).nevents_pos = ptr::null_mut();
        (*ui).nevents = 0;
        (*ui).flushed_events = false;
        (*ui).incomplete_event = false;
        (*ui).ncalls_pos = ptr::null_mut();
        (*ui).ncalls = 0;
        (*ui).ncells_pending = 0;
        (*ui).packer = PackerBuffer {
            startptr: ptr::null_mut(),
            ptr: ptr::null_mut(),
            endptr: ptr::null_mut(),
            packer_flush: Some(ui_flush_callback),
            anydata: ui.cast(),
            anyint: 0,
        };
        (*ui).wildmenu_active = false;

        let slot = map_put_ref_uint64_t_ptr_t(
            &raw mut connected_uis,
            channel_id,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        *slot = ui.cast();
        current_ui = channel_id;
        ui_attach_impl(ui.cast(), channel_id);

        let chan = find_channel(channel_id);
        if !chan.is_null() {
            (*channel_tail(chan)).rpc.ui = ui;
        }

        may_trigger_vim_suspend_resume(false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_attach(
    channel_id: u64,
    width: Integer,
    height: Integer,
    enable_rgb: Boolean,
    err: *mut Error,
) {
    unsafe {
        let mut opts_items: [KeyValuePair; 1] = [KeyValuePair {
            key: cstr_as_string(c"rgb".as_ptr()),
            value: bool_obj(enable_rgb),
        }];
        let opts = Dict {
            items: opts_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        nvim_ui_attach(channel_id, width, height, opts, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_set_focus(channel_id: u64, gained: Boolean, error: *mut Error) {
    unsafe {
        if get_ui_or_err(channel_id, error).is_null() {
            return;
        }
        if gained {
            current_ui = channel_id;
            may_trigger_vim_suspend_resume(false);
        }
        do_autocmd_focusgained(gained);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_detach(channel_id: u64, err: *mut Error) {
    unsafe {
        remote_ui_disconnect(channel_id, err, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_connect(
    channel_id: u64,
    server_addr: *mut c_char,
    err: *mut Error,
) {
    unsafe {
        let ui = get_ui_or_err(channel_id, err);
        if ui.is_null() {
            return;
        }
        let mut args_items: [Object; 1] = [str_obj(cstr_as_string(server_addr))];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"connect".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_restart(listen_addr: NvimString) {
    unsafe {
        for i in 0..connected_uis.set.h.n_keys as usize {
            let ui = (*connected_uis.values.add(i)).cast::<RemoteUI>();
            if ui.is_null() {
                continue;
            }
            let chan = find_channel((*ui).channel_id);
            let detach_val = !chan.is_null() && (*channel_tail(chan)).detach;
            let mut cfg_items: [KeyValuePair; 1] = [KeyValuePair {
                key: cstr_as_string(c"detach".as_ptr()),
                value: bool_obj(detach_val),
            }];
            let chan_config = Dict {
                items: cfg_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };
            let mut args_items: [Object; 2] = [str_obj(listen_addr), dict_obj(chan_config)];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            push_call(ui, c"restart".as_ptr(), args);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_try_resize(
    channel_id: u64,
    width: Integer,
    height: Integer,
    err: *mut Error,
) {
    unsafe {
        let ui = get_ui_or_err(channel_id, err);
        if ui.is_null() {
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
        (*ui).width = width as c_int;
        (*ui).height = height as c_int;
        ui_refresh();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_set_option(
    channel_id: u64,
    name: NvimString,
    value: Object,
    error: *mut Error,
) {
    unsafe {
        let ui = get_ui_or_err(channel_id, error);
        if ui.is_null() {
            return;
        }
        ui_set_option(ui, false, name, value, error);
    }
}

unsafe fn ui_set_option(
    ui: *mut RemoteUI,
    init: bool,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        if strequal(name.data, c"override".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"override".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            (*ui).r#override = value.data.boolean;
            return;
        }

        if strequal(name.data, c"rgb".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"rgb".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            (*ui).rgb = value.data.boolean;
            if !init && !(*ui).ui_ext[kUILinegrid] {
                ui_refresh();
            }
            return;
        }

        if strequal(name.data, c"term_name".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeString {
                api_err_exp(
                    err,
                    c"term_name".as_ptr(),
                    api_typename(ObjectType::kObjectTypeString),
                    api_typename(value.obj_type),
                );
                return;
            }
            set_tty_option(c"term".as_ptr(), string_to_cstr(value.data.string));
            (*ui).term_name = string_to_cstr(value.data.string);
            return;
        }

        if strequal(name.data, c"term_colors".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeInteger {
                api_err_exp(
                    err,
                    c"term_colors".as_ptr(),
                    api_typename(ObjectType::kObjectTypeInteger),
                    api_typename(value.obj_type),
                );
                return;
            }
            t_colors = value.data.integer as c_int;
            (*ui).term_colors = value.data.integer as c_int;
            return;
        }

        if strequal(name.data, c"stdin_fd".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeInteger {
                api_err_exp(
                    err,
                    c"stdin_fd".as_ptr(),
                    api_typename(ObjectType::kObjectTypeInteger),
                    api_typename(value.obj_type),
                );
                return;
            }
            if value.data.integer < 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid stdin_fd: %lld".as_ptr(),
                    value.data.integer,
                );
                return;
            }
            if starting != NO_SCREEN {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"stdin_fd can only be used with first attached UI".as_ptr(),
                );
                return;
            }
            stdin_fd = value.data.integer as c_int;
            return;
        }

        if strequal(name.data, c"stdin_tty".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"stdin_tty".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            if (*ui).channel_id == CHAN_STDIO {
                stdin_isatty = value.data.boolean;
            }
            (*ui).stdin_tty = value.data.boolean;
            return;
        }

        if strequal(name.data, c"stdout_tty".as_ptr()) {
            if value.obj_type != ObjectType::kObjectTypeBoolean {
                api_err_exp(
                    err,
                    c"stdout_tty".as_ptr(),
                    api_typename(ObjectType::kObjectTypeBoolean),
                    api_typename(value.obj_type),
                );
                return;
            }
            if (*ui).channel_id == CHAN_STDIO {
                stdout_isatty = value.data.boolean;
            }
            (*ui).stdout_tty = value.data.boolean;
            return;
        }

        let is_popupmenu = strequal(name.data, c"popupmenu_external".as_ptr());
        let ext_names_ptr = (&raw const ui_ext_names).cast::<*const c_char>();
        for i in 0..kUIExtCount {
            if strequal(name.data, *ext_names_ptr.add(i)) || (i == kUIPopupmenu && is_popupmenu) {
                if value.obj_type != ObjectType::kObjectTypeBoolean {
                    api_err_exp(
                        err,
                        name.data,
                        c"Boolean".as_ptr(),
                        api_typename(value.obj_type),
                    );
                    return;
                }
                let boolval = value.data.boolean;
                if !init && i == kUILinegrid && boolval != (*ui).ui_ext[i] {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"ext_linegrid option cannot be changed".as_ptr(),
                    );
                }
                (*ui).ui_ext[i] = boolval;
                if !init {
                    ui_set_ext_option(ui.cast(), i as UIExtension, boolval);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_try_resize_grid(
    channel_id: u64,
    grid: Integer,
    width: Integer,
    height: Integer,
    err: *mut Error,
) {
    unsafe {
        if get_ui_or_err(channel_id, err).is_null() {
            return;
        }
        if grid == DEFAULT_GRID_HANDLE {
            nvim_ui_try_resize(channel_id, width, height, err);
        } else {
            ui_grid_resize(grid as handle_T, width as c_int, height as c_int, err);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_pum_set_height(channel_id: u64, height: Integer, err: *mut Error) {
    unsafe {
        let ui = get_ui_or_err(channel_id, err);
        if ui.is_null() {
            return;
        }
        if height <= 0 {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected pum height > 0".as_ptr(),
            );
            return;
        }
        if !(*ui).ui_ext[kUIPopupmenu] {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"UI must support the ext_popupmenu option".as_ptr(),
            );
            return;
        }
        (*ui).pum_nlines = height as c_int;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_pum_set_bounds(
    channel_id: u64,
    width: Float,
    height: Float,
    row: Float,
    col: Float,
    err: *mut Error,
) {
    unsafe {
        let ui = get_ui_or_err(channel_id, err);
        if ui.is_null() {
            return;
        }
        if !(*ui).ui_ext[kUIPopupmenu] {
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
        (*ui).pum_row = row;
        (*ui).pum_col = col;
        (*ui).pum_width = width;
        (*ui).pum_height = height;
        (*ui).pum_pos = true;
    }
}

unsafe fn flush_event(ui: *mut RemoteUI) {
    unsafe {
        if !(*ui).cur_event.is_null() {
            mpack_w2(&raw mut (*ui).ncalls_pos, 1 + (*ui).ncalls);
            (*ui).cur_event = ptr::null();
            (*ui).ncalls_pos = ptr::null_mut();
            (*ui).ncalls = 0;
        }
    }
}

unsafe fn ui_alloc_buf(ui: *mut RemoteUI) {
    unsafe {
        (*ui).packer.startptr = alloc_block();
        (*ui).packer.ptr = (*ui).packer.startptr;
        (*ui).packer.endptr = (*ui).packer.startptr.add(UI_BUF_SIZE);
    }
}

unsafe fn prepare_call(ui: *mut RemoteUI, name: *const c_char) {
    unsafe {
        if !(*ui).packer.startptr.is_null()
            && (buf_pos(ui) > UI_BUF_SIZE - EVENT_BUF_SIZE || (*ui).ncells_pending >= 500)
        {
            ui_flush_buf(ui, false);
        }

        if (*ui).packer.startptr.is_null() {
            ui_alloc_buf(ui);
        }

        if (*ui).cur_event.is_null() || !strequal((*ui).cur_event, name) {
            let buf = &raw mut (*ui).packer.ptr;
            if (*ui).nevents_pos.is_null() {
                mpack_array(buf, 3);
                mpack_uint(buf, 2);
                mpack_str_small(buf, c"redraw".as_ptr(), 6);
                (*ui).nevents_pos = mpack_array_dyn16(buf);
                debug_assert!((*ui).cur_event.is_null());
            }
            flush_event(ui);
            (*ui).cur_event = name;
            (*ui).ncalls_pos = mpack_array_dyn16(buf);
            mpack_str_small(buf, name, CStr::from_ptr(name).to_bytes().len());
            (*ui).nevents += 1;
            (*ui).ncalls = 1;
        } else {
            (*ui).ncalls += 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn push_call(ui: *mut RemoteUI, name: *const c_char, args: Array) {
    unsafe {
        prepare_call(ui, name);
        mpack_object_array(args, &raw mut (*ui).packer);
    }
}

unsafe extern "C" fn ui_flush_callback(packer: *mut PackerBuffer) {
    unsafe {
        let ui = (*packer).anydata.cast::<RemoteUI>();
        ui_flush_buf(ui, true);
        ui_alloc_buf(ui);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_grid_clear(ui: *mut RemoteUI, grid: Integer) {
    unsafe {
        let mut args_items: [Object; 1] = [int_obj(grid)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: usize::from((*ui).ui_ext[kUILinegrid]),
            capacity: 1,
        };
        let name = if (*ui).ui_ext[kUILinegrid] {
            c"grid_clear".as_ptr()
        } else {
            c"clear".as_ptr()
        };
        push_call(ui, name, args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_grid_resize(
    ui: *mut RemoteUI,
    grid: Integer,
    width: Integer,
    height: Integer,
) {
    unsafe {
        let mut args_items: [Object; 3] = zeroed();
        let mut args = Array {
            items: args_items.as_mut_ptr(),
            size: 0,
            capacity: 3,
        };
        if (*ui).ui_ext[kUILinegrid] {
            args_items[args.size] = int_obj(grid);
            args.size += 1;
        } else {
            (*ui).client_col = -1;
        }
        args_items[args.size] = int_obj(width);
        args.size += 1;
        args_items[args.size] = int_obj(height);
        args.size += 1;
        let name = if (*ui).ui_ext[kUILinegrid] {
            c"grid_resize".as_ptr()
        } else {
            c"resize".as_ptr()
        };
        push_call(ui, name, args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_grid_scroll(
    ui: *mut RemoteUI,
    grid: Integer,
    top: Integer,
    bot: Integer,
    left: Integer,
    right: Integer,
    rows: Integer,
    cols: Integer,
) {
    unsafe {
        if (*ui).ui_ext[kUILinegrid] {
            let mut args_items: [Object; 7] = [
                int_obj(grid),
                int_obj(top),
                int_obj(bot),
                int_obj(left),
                int_obj(right),
                int_obj(rows),
                int_obj(cols),
            ];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 7,
                capacity: 7,
            };
            push_call(ui, c"grid_scroll".as_ptr(), args);
        } else {
            let mut args_items: [Object; 4] = [
                int_obj(top),
                int_obj(bot - 1),
                int_obj(left),
                int_obj(right - 1),
            ];
            let mut args = Array {
                items: args_items.as_mut_ptr(),
                size: 4,
                capacity: 4,
            };
            push_call(ui, c"set_scroll_region".as_ptr(), args);

            args_items[0] = int_obj(rows);
            args.size = 1;
            push_call(ui, c"scroll".as_ptr(), args);

            args_items[0] = int_obj(0);
            args_items[1] = int_obj(Integer::from((*ui).height - 1));
            args_items[2] = int_obj(0);
            args_items[3] = int_obj(Integer::from((*ui).width - 1));
            args.size = 4;
            push_call(ui, c"set_scroll_region".as_ptr(), args);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_default_colors_set(
    ui: *mut RemoteUI,
    mut rgb_fg: Integer,
    mut rgb_bg: Integer,
    mut rgb_sp: Integer,
    cterm_fg: Integer,
    cterm_bg: Integer,
) {
    unsafe {
        if !(*ui).ui_ext[kUITermColors] {
            let dark = *p_bg == b'd' as c_char;
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
            int_obj(rgb_fg),
            int_obj(rgb_bg),
            int_obj(rgb_sp),
            int_obj(cterm_fg),
            int_obj(cterm_bg),
        ];
        let mut args = Array {
            items: args_items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        };
        push_call(ui, c"default_colors_set".as_ptr(), args);

        if !(*ui).ui_ext[kUILinegrid] {
            args_items[0] = int_obj(if (*ui).rgb { rgb_fg } else { cterm_fg - 1 });
            args.size = 1;
            push_call(ui, c"update_fg".as_ptr(), args);

            args_items[0] = int_obj(if (*ui).rgb { rgb_bg } else { cterm_bg - 1 });
            args.size = 1;
            push_call(ui, c"update_bg".as_ptr(), args);

            args_items[0] = int_obj(if (*ui).rgb { rgb_sp } else { -1 });
            args.size = 1;
            push_call(ui, c"update_sp".as_ptr(), args);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_hl_attr_define(
    ui: *mut RemoteUI,
    id: Integer,
    rgb_attrs: HlAttrs,
    _cterm_attrs: HlAttrs,
    info: Array,
) {
    unsafe {
        if !(*ui).ui_ext[kUILinegrid] {
            return;
        }

        let mut rgb_items: [KeyValuePair; HLATTRS_DICT_SIZE] = zeroed();
        let mut rgb = Dict {
            items: rgb_items.as_mut_ptr(),
            size: 0,
            capacity: HLATTRS_DICT_SIZE,
        };
        let mut cterm_items: [KeyValuePair; HLATTRS_DICT_SIZE] = zeroed();
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
                value: str_obj(cstr_as_string(url)),
            };
            rgb.size += 1;
        }

        let mut args_items: [Object; 4] = [
            int_obj(id),
            dict_obj(rgb),
            dict_obj(cterm),
            arr_obj(if (*ui).ui_ext[kUIHlState] {
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
        push_call(ui, c"hl_attr_define".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_highlight_set(ui: *mut RemoteUI, id: c_int) {
    unsafe {
        if (*ui).hl_id == id {
            return;
        }
        (*ui).hl_id = id;
        let mut dict_items: [KeyValuePair; HLATTRS_DICT_SIZE] = zeroed();
        let mut dict = Dict {
            items: dict_items.as_mut_ptr(),
            size: 0,
            capacity: HLATTRS_DICT_SIZE,
        };
        hlattrs2dict(
            &raw mut dict,
            ptr::null_mut(),
            syn_attr2entry(id),
            (*ui).rgb,
            false,
        );
        let mut args_items: [Object; 1] = [dict_obj(dict)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"highlight_set".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_grid_cursor_goto(
    ui: *mut RemoteUI,
    grid: Integer,
    row: Integer,
    col: Integer,
) {
    unsafe {
        if (*ui).ui_ext[kUILinegrid] {
            let mut args_items: [Object; 3] = [int_obj(grid), int_obj(row), int_obj(col)];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 3,
                capacity: 3,
            };
            push_call(ui, c"grid_cursor_goto".as_ptr(), args);
        } else {
            (*ui).cursor_row = row;
            (*ui).cursor_col = col;
            remote_ui_cursor_goto(ui, row, col);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_cursor_goto(ui: *mut RemoteUI, row: Integer, col: Integer) {
    unsafe {
        if (*ui).client_row == row && (*ui).client_col == col {
            return;
        }
        (*ui).client_row = row;
        (*ui).client_col = col;
        let mut args_items: [Object; 2] = [int_obj(row), int_obj(col)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"cursor_goto".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_put(ui: *mut RemoteUI, cell: *const c_char) {
    unsafe {
        (*ui).client_col += 1;
        let mut args_items: [Object; 1] = [str_obj(cstr_as_string(cell))];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"put".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_raw_line(
    ui: *mut RemoteUI,
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
        if (*ui).ui_ext[kUILinegrid] {
            prepare_call(ui, c"grid_line".as_ptr());

            let buf = &raw mut (*ui).packer.ptr;
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
                        || (*ui).ncells_pending >= 500
                    {
                        if was_space {
                            nelem += 1;
                            (*ui).ncells_pending += 1;
                            mpack_array(buf, 3);
                            mpack_str_small(buf, c" ".as_ptr(), 1);
                            mpack_uint(buf, clearattr as u32);
                            mpack_uint(buf, 0);
                        }
                        mpack_w2(&raw mut lenpos, nelem);
                        mpack_bool(buf, false);
                        ui_flush_buf(ui, false);

                        prepare_call(ui, c"grid_line".as_ptr());
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
                    (*ui).ncells_pending += repeat.min(2) as usize;
                    last_hl = c_int::from(*attrs.add(i));
                    repeat = 0;
                    was_space = *chunk.add(i) == schar_from_ascii(b' ');
                }
            }

            if endcol < clearcol || was_space {
                nelem += 1;
                (*ui).ncells_pending += 1;
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
                    (*ui).client_col = -1;
                }
            }
            if endcol < clearcol {
                remote_ui_cursor_goto(ui, row, endcol);
                remote_ui_highlight_set(ui, clearattr as c_int);
                if clearattr == 0 && clearcol == Integer::from(Columns) {
                    push_call(ui, c"eol_clear".as_ptr(), ARRAY_INIT);
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

unsafe fn ui_flush_buf(ui: *mut RemoteUI, incomplete_event: bool) {
    unsafe {
        if (*ui).packer.startptr.is_null() || buf_pos(ui) == 0 {
            return;
        }
        (*ui).incomplete_event = incomplete_event;

        flush_event(ui);
        if !(*ui).nevents_pos.is_null() {
            mpack_w2(&raw mut (*ui).nevents_pos, (*ui).nevents);
            (*ui).nevents = 0;
            (*ui).nevents_pos = ptr::null_mut();
        }

        let buf = wstream_new_buffer((*ui).packer.startptr, buf_pos(ui), 1, Some(free_block_cb));
        rpc_write_raw((*ui).channel_id, buf);

        (*ui).packer.startptr = ptr::null_mut();
        (*ui).packer.ptr = ptr::null_mut();

        (*ui).flushed_events = true;
        (*ui).ncells_pending = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_flush(ui: *mut RemoteUI) {
    unsafe {
        if (*ui).nevents > 0 || (*ui).flushed_events {
            if !(*ui).ui_ext[kUILinegrid] {
                remote_ui_cursor_goto(ui, (*ui).cursor_row, (*ui).cursor_col);
            }
            push_call(ui, c"flush".as_ptr(), ARRAY_INIT);
            ui_flush_buf(ui, false);
            (*ui).flushed_events = false;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_ui_send(ui: *mut RemoteUI, content: NvimString) {
    unsafe {
        if !(*ui).stdout_tty {
            return;
        }
        let mut args_items: [Object; 1] = [str_obj(content)];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"ui_send".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_flush_pending_data(ui: *mut RemoteUI) {
    unsafe {
        ui_flush_buf(ui, false);
    }
}

unsafe fn translate_contents(ui: *mut RemoteUI, contents: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut new_contents = arena_array(arena, contents.size);
        for i in 0..contents.size {
            let item = (*contents.items.add(i)).data.array;
            let mut new_item = arena_array(arena, 2);
            let attr = (*item.items).data.integer as c_int;
            if attr != 0 {
                let mut rgb_attrs = arena_dict(arena, HLATTRS_DICT_SIZE);
                hlattrs2dict(
                    &raw mut rgb_attrs,
                    ptr::null_mut(),
                    syn_attr2entry(attr),
                    (*ui).rgb,
                    false,
                );
                *new_item.items.add(0) = dict_obj(rgb_attrs);
            } else {
                *new_item.items.add(0) = dict_obj(DICT_INIT);
            }
            *new_item.items.add(1) = *item.items.add(1);
            new_item.size = 2;
            *new_contents.items.add(i) = arr_obj(new_item);
        }
        new_contents.size = contents.size;
        new_contents
    }
}

unsafe fn translate_firstarg(ui: *mut RemoteUI, args: Array, arena: *mut Arena) -> Array {
    unsafe {
        let mut new_args = arena_array(arena, args.size);
        let contents = (*args.items).data.array;
        *new_args.items.add(0) = arr_obj(translate_contents(ui, contents, arena));
        for i in 1..args.size {
            *new_args.items.add(i) = *args.items.add(i);
        }
        new_args.size = args.size;
        new_args
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_event(ui: *mut RemoteUI, mut name: *mut c_char, args: Array) {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        if !(*ui).ui_ext[kUILinegrid] {
            if strequal(name, c"cmdline_show".as_ptr()) {
                let new_args = translate_firstarg(ui, args, &raw mut arena);
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            } else if strequal(name, c"cmdline_block_show".as_ptr()) {
                let block = (*args.items).data.array;
                let mut new_block = arena_array(&raw mut arena, block.size);
                for i in 0..block.size {
                    *new_block.items.add(i) = arr_obj(translate_contents(
                        ui,
                        (*block.items.add(i)).data.array,
                        &raw mut arena,
                    ));
                }
                new_block.size = block.size;
                let mut new_args_items: [Object; 1] = [arr_obj(new_block)];
                let new_args = Array {
                    items: new_args_items.as_mut_ptr(),
                    size: 1,
                    capacity: 1,
                };
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            } else if strequal(name, c"cmdline_block_append".as_ptr()) {
                let new_args = translate_firstarg(ui, args, &raw mut arena);
                push_call(ui, name, new_args);
                arena_mem_free(arena_finish(&raw mut arena));
                return;
            }
        }

        if (*ui).ui_ext[kUIWildmenu] {
            if strequal(name, c"popupmenu_show".as_ptr()) {
                (*ui).wildmenu_active =
                    (*args.items.add(4)).data.integer == -1 || !(*ui).ui_ext[kUIPopupmenu];
                if (*ui).wildmenu_active {
                    let items = (*args.items).data.array;
                    let mut new_items = arena_array(&raw mut arena, items.size);
                    for i in 0..items.size {
                        *new_items.items.add(i) = *(*items.items.add(i)).data.array.items;
                    }
                    new_items.size = items.size;
                    let mut new_args_items: [Object; 1] = [arr_obj(new_items)];
                    let new_args = Array {
                        items: new_args_items.as_mut_ptr(),
                        size: 1,
                        capacity: 1,
                    };
                    push_call(ui, c"wildmenu_show".as_ptr(), new_args);
                    if (*args.items.add(1)).data.integer != -1 {
                        new_args_items[0] = *args.items.add(1);
                        push_call(ui, c"wildmenu_select".as_ptr(), new_args);
                    }
                    arena_mem_free(arena_finish(&raw mut arena));
                    return;
                }
            } else if strequal(name, c"popupmenu_select".as_ptr()) {
                if (*ui).wildmenu_active {
                    name = c"wildmenu_select".as_ptr().cast_mut();
                }
            } else if strequal(name, c"popupmenu_hide".as_ptr()) && (*ui).wildmenu_active {
                name = c"wildmenu_hide".as_ptr().cast_mut();
            }
        }

        push_call(ui, name, args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_ui_send(_channel_id: u64, content: NvimString, _err: *mut Error) {
    unsafe {
        ui_call_ui_send(content);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_mode_info_set(
    ui: *mut RemoteUI,
    enabled: Boolean,
    cursor_styles: Array,
) {
    unsafe {
        let mut items = [bool_obj(enabled), arr_obj(cursor_styles)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"mode_info_set".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_update_menu(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"update_menu".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_busy_start(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"busy_start".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_busy_stop(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"busy_stop".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_mouse_on(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"mouse_on".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_mouse_off(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"mouse_off".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_mode_change(
    ui: *mut RemoteUI,
    mode: NvimString,
    mode_idx: Integer,
) {
    unsafe {
        let mut items = [str_obj(mode), int_obj(mode_idx)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"mode_change".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_bell(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"bell".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_visual_bell(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"visual_bell".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_suspend(ui: *mut RemoteUI) {
    unsafe {
        push_call(ui, c"suspend".as_ptr(), ARRAY_INIT);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_set_title(ui: *mut RemoteUI, title: NvimString) {
    unsafe {
        let mut items = [str_obj(title)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"set_title".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_set_icon(ui: *mut RemoteUI, icon: NvimString) {
    unsafe {
        let mut items = [str_obj(icon)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"set_icon".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_screenshot(ui: *mut RemoteUI, path: NvimString) {
    unsafe {
        let mut items = [str_obj(path)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"screenshot".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_option_set(ui: *mut RemoteUI, name: NvimString, value: Object) {
    unsafe {
        let mut items = [str_obj(name), value];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"option_set".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_chdir(ui: *mut RemoteUI, path: NvimString) {
    unsafe {
        let mut items = [str_obj(path)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"chdir".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_hl_group_set(ui: *mut RemoteUI, name: NvimString, id: Integer) {
    unsafe {
        let mut items = [str_obj(name), int_obj(id)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"hl_group_set".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_msg_set_pos(
    ui: *mut RemoteUI,
    grid: Integer,
    row: Integer,
    scrolled: Boolean,
    sep_char: NvimString,
    zindex: Integer,
    compindex: Integer,
) {
    unsafe {
        let mut items = [
            int_obj(grid),
            int_obj(row),
            bool_obj(scrolled),
            str_obj(sep_char),
            int_obj(zindex),
            int_obj(compindex),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 6,
            capacity: 6,
        };
        push_call(ui, c"msg_set_pos".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_win_viewport(
    ui: *mut RemoteUI,
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
            int_obj(grid),
            win_obj(win),
            int_obj(topline),
            int_obj(botline),
            int_obj(curline),
            int_obj(curcol),
            int_obj(line_count),
            int_obj(scroll_delta),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 8,
            capacity: 8,
        };
        push_call(ui, c"win_viewport".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_win_viewport_margins(
    ui: *mut RemoteUI,
    grid: Integer,
    win: handle_T,
    top: Integer,
    bottom: Integer,
    left: Integer,
    right: Integer,
) {
    unsafe {
        let mut items = [
            int_obj(grid),
            win_obj(win),
            int_obj(top),
            int_obj(bottom),
            int_obj(left),
            int_obj(right),
        ];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 6,
            capacity: 6,
        };
        push_call(ui, c"win_viewport_margins".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui_error_exit(ui: *mut RemoteUI, status: Integer) {
    unsafe {
        let mut items = [int_obj(status)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        push_call(ui, c"error_exit".as_ptr(), args);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn remote_ui__set_restart_on_crash_exit(
    ui: *mut RemoteUI,
    progpath: NvimString,
    argv: Array,
) {
    unsafe {
        let mut items = [str_obj(progpath), arr_obj(argv)];
        let args = Array {
            items: items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        push_call(ui, c"_set_restart_on_crash_exit".as_ptr(), args);
    }
}
