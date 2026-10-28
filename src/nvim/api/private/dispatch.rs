//! API Msgpack-RPC request handler lookup and dispatch wrappers (`src/nvim/api/private/dispatch.c`).

use crate::api::autocmd::{
    KeyDict_clear_autocmds, KeyDict_create_augroup, KeyDict_create_autocmd, KeyDict_exec_autocmds,
    KeyDict_get_autocmds, nvim_clear_autocmds, nvim_create_augroup, nvim_create_autocmd,
    nvim_del_augroup_by_id, nvim_del_augroup_by_name, nvim_del_autocmd, nvim_exec_autocmds,
    nvim_get_autocmds,
};
use crate::api::buffer::{
    KeyDict_buf_attach, KeyDict_buf_delete, KeyDict_keymap, KeyDict_keymap_del, LUA_INTERNAL_CALL,
    nvim_buf_attach, nvim_buf_call, nvim_buf_del_keymap, nvim_buf_del_mark, nvim_buf_del_var,
    nvim_buf_delete, nvim_buf_detach, nvim_buf_get_changedtick, nvim_buf_get_keymap,
    nvim_buf_get_lines, nvim_buf_get_mark, nvim_buf_get_name, nvim_buf_get_offset,
    nvim_buf_get_text, nvim_buf_get_var, nvim_buf_is_loaded, nvim_buf_is_valid,
    nvim_buf_line_count, nvim_buf_set_keymap, nvim_buf_set_lines, nvim_buf_set_mark,
    nvim_buf_set_name, nvim_buf_set_text, nvim_buf_set_var, nvim_buf_stats,
};
use crate::api::command::{
    KeyDict_cmd, KeyDict_cmd_magic, KeyDict_cmd_mods, KeyDict_cmd_mods_filter, KeyDict_cmd_opts,
    KeyDict_get_commands, KeyDict_user_command, nvim_buf_create_user_command,
    nvim_buf_del_user_command, nvim_buf_get_commands, nvim_cmd, nvim_create_user_command,
    nvim_del_user_command, nvim_get_commands, nvim_parse_cmd,
};
use crate::api::deprecated::{
    KeyDict_empty, buffer_del_line, buffer_del_var, buffer_get_line, buffer_get_line_slice,
    buffer_insert, buffer_set_line, buffer_set_line_slice, buffer_set_var, nvim_buf_add_highlight,
    nvim_buf_clear_highlight, nvim_buf_get_number, nvim_buf_get_option, nvim_buf_set_option,
    nvim_buf_set_virtual_text, nvim_call_atomic, nvim_command_output, nvim_err_write,
    nvim_err_writeln, nvim_exec, nvim_execute_lua, nvim_get_hl_by_id, nvim_get_hl_by_name,
    nvim_get_option, nvim_get_option_info, nvim_notify, nvim_out_write, nvim_set_option,
    nvim_subscribe, nvim_unsubscribe, nvim_win_get_option, nvim_win_set_height,
    nvim_win_set_option, nvim_win_set_width, tabpage_del_var, tabpage_set_var, vim_del_var,
    vim_set_var, window_del_var, window_set_var,
};
use crate::api::events::{nvim_error_event, nvim_ui_term_event};
use crate::api::extmark::{
    KeyDict_get_extmark, KeyDict_get_extmarks, KeyDict_ns_opts, KeyDict_set_decoration_provider,
    KeyDict_set_extmark, nvim_buf_clear_namespace, nvim_buf_debug_extmarks, nvim_buf_del_extmark,
    nvim_buf_get_extmark_by_id, nvim_buf_get_extmarks, nvim_buf_set_extmark, nvim_create_namespace,
    nvim_get_namespaces, nvim_ns_get, nvim_ns_set, nvim_set_decoration_provider,
};
use crate::api::options::{
    KeyDict_option, nvim_get_all_options_info, nvim_get_option_info2, nvim_get_option_value,
    nvim_option_to_object, nvim_set_option_value,
};
use crate::api::private::helpers::{
    OBJECT_NIL, api_clear_error, api_dict_to_keydict, api_free_dict, api_free_object,
    api_free_string, api_keydict_to_dict, api_luarefs_free_keydict, api_luarefs_free_object,
    api_set_error, api_set_error_str, array_obj, boolean_obj, buffer_obj, dict_obj, float_obj,
    integer_obj, string_obj, tabpage_obj, window_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType,
};
use crate::api::tabpage::{
    Buffer, KeyDict_tabpage_config, Tabpage, Window, nvim_open_tabpage, nvim_tabpage_del_var,
    nvim_tabpage_get_number, nvim_tabpage_get_var, nvim_tabpage_get_win, nvim_tabpage_is_valid,
    nvim_tabpage_list_wins, nvim_tabpage_set_var, nvim_tabpage_set_win,
};
use crate::api::ui::{
    nvim_ui_attach, nvim_ui_detach, nvim_ui_pum_set_bounds, nvim_ui_pum_set_height, nvim_ui_send,
    nvim_ui_set_focus, nvim_ui_set_option, nvim_ui_try_resize, nvim_ui_try_resize_grid, ui_attach,
};
use crate::api::vim::{
    KeyDict_chan_set, KeyDict_complete_set, KeyDict_context, KeyDict_echo_opts,
    KeyDict_eval_statusline, KeyDict_get_ns, KeyDict_open_term, KeyDict_redraw, KeyDict_runtime,
    nvim__id_float, nvim_chan_send, nvim_chan_set, nvim_cmdwin_set, nvim_complete_set,
    nvim_create_buf, nvim_del_current_line, nvim_del_keymap, nvim_del_mark, nvim_del_var,
    nvim_echo, nvim_eval_statusline, nvim_exec_lua, nvim_exec_lua_fast, nvim_feedkeys,
    nvim_get_api_info, nvim_get_chan_info, nvim_get_color_by_name, nvim_get_color_map,
    nvim_get_context, nvim_get_current_buf, nvim_get_current_line, nvim_get_current_tabpage,
    nvim_get_current_win, nvim_get_hl, nvim_get_hl_id_by_name, nvim_get_hl_ns, nvim_get_keymap,
    nvim_get_lib_dir, nvim_get_mark, nvim_get_mode, nvim_get_proc, nvim_get_proc_children,
    nvim_get_runtime, nvim_get_runtime_file, nvim_get_var, nvim_get_vvar, nvim_id, nvim_id_array,
    nvim_id_dict, nvim_input, nvim_input_mouse, nvim_inspect_cell, nvim_invalidate_glyph_cache,
    nvim_list_bufs, nvim_list_chans, nvim_list_runtime_paths, nvim_list_tabpages, nvim_list_uis,
    nvim_list_wins, nvim_load_context, nvim_open_term, nvim_paste, nvim_put, nvim_redraw,
    nvim_replace_termcodes, nvim_runtime_inspect, nvim_screenshot, nvim_select_popupmenu_item,
    nvim_set_client_info, nvim_set_current_buf, nvim_set_current_dir, nvim_set_current_line,
    nvim_set_current_tabpage, nvim_set_current_win, nvim_set_hl, nvim_set_hl_ns,
    nvim_set_hl_ns_fast, nvim_set_keymap, nvim_set_restart_on_crash, nvim_set_var, nvim_set_vvar,
    nvim_stats, nvim_strwidth, nvim_unpack,
};
use crate::api::vimscript::{
    KeyDict_exec_opts, nvim_call_dict_function, nvim_call_function, nvim_command, nvim_eval,
    nvim_exec2, nvim_parse_expression,
};
use crate::api::win_config::{
    KeyDict_win_config, nvim_open_win, nvim_win_get_config, nvim_win_set_config,
};
use crate::api::window::{
    KeyDict_win_resize, KeyDict_win_text_height, nvim_win_call, nvim_win_close, nvim_win_del_var,
    nvim_win_get_buf, nvim_win_get_cursor, nvim_win_get_height, nvim_win_get_number,
    nvim_win_get_position, nvim_win_get_tabpage, nvim_win_get_var, nvim_win_get_width,
    nvim_win_hide, nvim_win_is_valid, nvim_win_resize, nvim_win_set_buf, nvim_win_set_cursor,
    nvim_win_set_hl_ns, nvim_win_set_var, nvim_win_text_height,
};
use crate::ex_getln::{get_text_locked_msg, text_locked};
use crate::highlight::{KeyDict_highlight, KeyDict_highlight_cterm};
use crate::highlight_group::KeyDict_get_highlight;
use crate::lua::converter::{
    Boolean, FieldHashfn, Float, KeySetLink, LuaRef, lua_concat, lua_createtable, lua_error,
    lua_gettop, lua_pushcclosure, lua_pushstring, lua_setfield, luaL_error, luaL_where,
    nlua_pop_array, nlua_pop_boolean, nlua_pop_dict, nlua_pop_float,
    nlua_pop_handle as nlua_pop_Buffer, nlua_pop_handle as nlua_pop_Tabpage,
    nlua_pop_handle as nlua_pop_Window, nlua_pop_integer, nlua_pop_keydict, nlua_pop_lua_ref,
    nlua_pop_object, nlua_pop_string, nlua_push_array, nlua_push_boolean, nlua_push_dict,
    nlua_push_float, nlua_push_handle as nlua_push_Buffer, nlua_push_handle as nlua_push_Tabpage,
    nlua_push_handle as nlua_push_Window, nlua_push_integer, nlua_push_keydict, nlua_push_object,
    nlua_push_string,
};
use crate::lua::xdiff::KeyDict_xdl_diff;
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::shada::{
    KeyDict__shada_buflist_item, KeyDict__shada_mark, KeyDict__shada_register,
    KeyDict__shada_search_pat,
};
use crate::ui_client::handle_ui_client_redraw;
use std::mem::offset_of;
use std::ptr;

const K_UNPACK_TYPE_STRING_ARRAY_CONST: i32 = -1;

pub type ApiDispatchWrapper =
    Option<fn(channel_id: u64, args: Array, arena: &mut Arena, error: &mut Error) -> Object>;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct MsgpackRpcRequestHandler {
    pub name: &'static std::ffi::CStr,
    pub r#fn: ApiDispatchWrapper,
    pub fast: bool,
    pub ret_alloc: bool,
}

#[inline]
fn dlog_internal_call(channel_id: u64, func_name: &std::ffi::CStr, msg: &std::ffi::CStr) {
    #[cfg(debug_assertions)]
    {
        if (channel_id & (1u64 << 63)) != 0 {
            let _ = crate::log::logmsg_str(
                crate::log::LOGLVL_DBG,
                ptr::null(),
                func_name.as_ptr(),
                0,
                true,
                msg.as_ptr(),
            );
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (channel_id, func_name.as_ptr(), msg.as_ptr());
}

#[inline]
fn extract_keydict<T: Default>(
    item: Object,
    out: &mut T,
    get_field: FieldHashfn,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeDict {
        *out = T::default();
        api_dict_to_keydict((out as *mut T).cast(), get_field, item.data.dict(), error)
    } else if item.obj_type == ObjectType::ObjectTypeArray && item.data.array().size == 0 {
        *out = T::default();
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_handle(
    item: Object,
    expected: ObjectType,
    out: &mut ObjHandle,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if (item.obj_type == expected || item.obj_type == ObjectType::ObjectTypeInteger)
        && item.data.integer() >= 0
    {
        *out = item.data.integer() as ObjHandle;
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_boolean(
    item: Object,
    out: &mut Boolean,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeBoolean {
        *out = item.data.boolean();
        true
    } else if item.obj_type == ObjectType::ObjectTypeInteger && item.data.integer() >= 0 {
        *out = (item.data.integer() as ObjHandle) != 0;
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_float(
    item: Object,
    out: &mut Float,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeFloat {
        *out = item.data.floating();
        true
    } else if item.obj_type == ObjectType::ObjectTypeInteger {
        *out = item.data.integer() as Float;
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_dict(
    item: Object,
    out: &mut Dict,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeDict {
        *out = item.data.dict();
        true
    } else if item.obj_type == ObjectType::ObjectTypeArray && item.data.array().size == 0 {
        *out = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_integer(
    item: Object,
    out: &mut Integer,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeInteger {
        *out = item.data.integer();
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_string(
    item: Object,
    out: &mut NvimString,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeString {
        *out = item.data.string();
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

#[inline]
fn extract_array(
    item: Object,
    out: &mut Array,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeArray {
        *out = item.data.array();
        true
    } else {
        api_set_error_str(
            error,
            ErrorType::ErrorTypeException,
            wrong_type_msg.as_ptr(),
        );
        false
    }
}

pub static _SHADA_BUFLIST_ITEM_TABLE: [KeySetLink; 4] = [
    KeySetLink {
        str: Some(c"c"),
        ptr_off: offset_of!(KeyDict__shada_buflist_item, c),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"f"),
        ptr_off: offset_of!(KeyDict__shada_buflist_item, f),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"l"),
        ptr_off: offset_of!(KeyDict__shada_buflist_item, l),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn _shada_buflist_item_hash(key: &[u8]) -> i32 {
    match key {
        b"c" => 0,
        b"f" => 1,
        b"l" => 2,
        _ => -1,
    }
}

pub fn key_dict_shada_buflist_item_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = _shada_buflist_item_hash(key);
    (hash >= 0).then(|| &_SHADA_BUFLIST_ITEM_TABLE[hash as usize])
}

pub static _SHADA_MARK_TABLE: [KeySetLink; 5] = [
    KeySetLink {
        str: Some(c"c"),
        ptr_off: offset_of!(KeyDict__shada_mark, c),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"f"),
        ptr_off: offset_of!(KeyDict__shada_mark, f),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"l"),
        ptr_off: offset_of!(KeyDict__shada_mark, l),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"n"),
        ptr_off: offset_of!(KeyDict__shada_mark, n),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn _shada_mark_hash(key: &[u8]) -> i32 {
    match key {
        b"c" => 0,
        b"f" => 1,
        b"l" => 2,
        b"n" => 3,
        _ => -1,
    }
}

pub fn key_dict_shada_mark_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = _shada_mark_hash(key);
    (hash >= 0).then(|| &_SHADA_MARK_TABLE[hash as usize])
}

pub static _SHADA_REGISTER_TABLE: [KeySetLink; 6] = [
    KeySetLink {
        str: Some(c"n"),
        ptr_off: offset_of!(KeyDict__shada_register, n),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rc"),
        ptr_off: offset_of!(KeyDict__shada_register, rc),
        key_type: K_UNPACK_TYPE_STRING_ARRAY_CONST,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rt"),
        ptr_off: offset_of!(KeyDict__shada_register, rt),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ru"),
        ptr_off: offset_of!(KeyDict__shada_register, ru),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rw"),
        ptr_off: offset_of!(KeyDict__shada_register, rw),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn _shada_register_hash(key: &[u8]) -> i32 {
    match key {
        b"n" => 0,
        b"rc" => 1,
        b"rt" => 2,
        b"ru" => 3,
        b"rw" => 4,
        _ => -1,
    }
}

pub fn key_dict_shada_register_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = _shada_register_hash(key);
    (hash >= 0).then(|| &_SHADA_REGISTER_TABLE[hash as usize])
}

pub static _SHADA_SEARCH_PAT_TABLE: [KeySetLink; 11] = [
    KeySetLink {
        str: Some(c"sb"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, search_backward),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sc"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, smartcase),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"se"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, place_cursor_at_end),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sh"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, highlighted),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sl"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, has_line_offset),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sm"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, magic),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"so"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, offset),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sp"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, pat),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ss"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, is_substitute_pattern),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"su"),
        ptr_off: offset_of!(KeyDict__shada_search_pat, is_last_used),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn _shada_search_pat_hash(key: &[u8]) -> i32 {
    match key {
        b"sb" => 0,
        b"sc" => 1,
        b"se" => 2,
        b"sh" => 3,
        b"sl" => 4,
        b"sm" => 5,
        b"so" => 6,
        b"sp" => 7,
        b"ss" => 8,
        b"su" => 9,
        _ => -1,
    }
}

pub fn key_dict_shada_search_pat_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = _shada_search_pat_hash(key);
    (hash >= 0).then(|| &_SHADA_SEARCH_PAT_TABLE[hash as usize])
}

pub static BUF_ATTACH_TABLE: [KeySetLink; 8] = [
    KeySetLink {
        str: Some(c"preview"),
        ptr_off: offset_of!(KeyDict_buf_attach, preview),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_bytes"),
        ptr_off: offset_of!(KeyDict_buf_attach, on_bytes),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_lines"),
        ptr_off: offset_of!(KeyDict_buf_attach, on_lines),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"utf_sizes"),
        ptr_off: offset_of!(KeyDict_buf_attach, utf_sizes),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_detach"),
        ptr_off: offset_of!(KeyDict_buf_attach, on_detach),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_reload"),
        ptr_off: offset_of!(KeyDict_buf_attach, on_reload),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_changedtick"),
        ptr_off: offset_of!(KeyDict_buf_attach, on_changedtick),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn buf_attach_hash(key: &[u8]) -> i32 {
    match key {
        b"preview" => 0,
        b"on_bytes" => 1,
        b"on_lines" => 2,
        b"utf_sizes" => 3,
        b"on_detach" => 4,
        b"on_reload" => 5,
        b"on_changedtick" => 6,
        _ => -1,
    }
}

pub fn key_dict_buf_attach_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = buf_attach_hash(key);
    (hash >= 0).then(|| &BUF_ATTACH_TABLE[hash as usize])
}

pub static BUF_DELETE_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"force"),
        ptr_off: offset_of!(KeyDict_buf_delete, force),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unload"),
        ptr_off: offset_of!(KeyDict_buf_delete, unload),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn buf_delete_hash(key: &[u8]) -> i32 {
    match key {
        b"force" => 0,
        b"unload" => 1,
        _ => -1,
    }
}

pub fn key_dict_buf_delete_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = buf_delete_hash(key);
    (hash >= 0).then(|| &BUF_DELETE_TABLE[hash as usize])
}

pub static CHAN_SET_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"detach"),
        ptr_off: offset_of!(KeyDict_chan_set, detach),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn chan_set_hash(key: &[u8]) -> i32 {
    match key {
        b"detach" => 0,
        _ => -1,
    }
}

pub fn key_dict_chan_set_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = chan_set_hash(key);
    (hash >= 0).then(|| &CHAN_SET_TABLE[hash as usize])
}

pub static CLEAR_AUTOCMDS_TABLE: [KeySetLink; 6] = [
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_clear_autocmds, buf),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"event"),
        ptr_off: offset_of!(KeyDict_clear_autocmds, event),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        ptr_off: offset_of!(KeyDict_clear_autocmds, group),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        ptr_off: offset_of!(KeyDict_clear_autocmds, buffer),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        ptr_off: offset_of!(KeyDict_clear_autocmds, pattern),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn clear_autocmds_hash(key: &[u8]) -> i32 {
    match key {
        b"buf" => 0,
        b"event" => 1,
        b"group" => 2,
        b"buffer" => 3,
        b"pattern" => 4,
        _ => -1,
    }
}

pub fn key_dict_clear_autocmds_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = clear_autocmds_hash(key);
    (hash >= 0).then(|| &CLEAR_AUTOCMDS_TABLE[hash as usize])
}

pub static CMD_TABLE: [KeySetLink; 12] = [
    KeySetLink {
        str: Some(c"cmd"),
        ptr_off: offset_of!(KeyDict_cmd, cmd),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reg"),
        ptr_off: offset_of!(KeyDict_cmd, reg),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bang"),
        ptr_off: offset_of!(KeyDict_cmd, bang),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"addr"),
        ptr_off: offset_of!(KeyDict_cmd, addr),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"mods"),
        ptr_off: offset_of!(KeyDict_cmd, mods),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"args"),
        ptr_off: offset_of!(KeyDict_cmd, args),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"count"),
        ptr_off: offset_of!(KeyDict_cmd, count),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"magic"),
        ptr_off: offset_of!(KeyDict_cmd, magic),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nargs"),
        ptr_off: offset_of!(KeyDict_cmd, nargs),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        ptr_off: offset_of!(KeyDict_cmd, range),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nextcmd"),
        ptr_off: offset_of!(KeyDict_cmd, nextcmd),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn cmd_hash(key: &[u8]) -> i32 {
    match key {
        b"cmd" => 0,
        b"reg" => 1,
        b"bang" => 2,
        b"addr" => 3,
        b"mods" => 4,
        b"args" => 5,
        b"count" => 6,
        b"magic" => 7,
        b"nargs" => 8,
        b"range" => 9,
        b"nextcmd" => 10,
        _ => -1,
    }
}

pub fn key_dict_cmd_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = cmd_hash(key);
    (hash >= 0).then(|| &CMD_TABLE[hash as usize])
}

pub static CMD_MAGIC_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"bar"),
        ptr_off: offset_of!(KeyDict_cmd_magic, bar),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"file"),
        ptr_off: offset_of!(KeyDict_cmd_magic, file),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn cmd_magic_hash(key: &[u8]) -> i32 {
    match key {
        b"bar" => 0,
        b"file" => 1,
        _ => -1,
    }
}

pub fn key_dict_cmd_magic_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = cmd_magic_hash(key);
    (hash >= 0).then(|| &CMD_MAGIC_TABLE[hash as usize])
}

pub static CMD_MODS_TABLE: [KeySetLink; 21] = [
    KeySetLink {
        str: Some(c"tab"),
        ptr_off: offset_of!(KeyDict_cmd_mods, tab),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hide"),
        ptr_off: offset_of!(KeyDict_cmd_mods, hide),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"split"),
        ptr_off: offset_of!(KeyDict_cmd_mods, split),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"browse"),
        ptr_off: offset_of!(KeyDict_cmd_mods, browse),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filter"),
        ptr_off: offset_of!(KeyDict_cmd_mods, filter),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"silent"),
        ptr_off: offset_of!(KeyDict_cmd_mods, silent),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"confirm"),
        ptr_off: offset_of!(KeyDict_cmd_mods, confirm),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepalt"),
        ptr_off: offset_of!(KeyDict_cmd_mods, keepalt),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sandbox"),
        ptr_off: offset_of!(KeyDict_cmd_mods, sandbox),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"verbose"),
        ptr_off: offset_of!(KeyDict_cmd_mods, verbose),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unsilent"),
        ptr_off: offset_of!(KeyDict_cmd_mods, unsilent),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        ptr_off: offset_of!(KeyDict_cmd_mods, vertical),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepjumps"),
        ptr_off: offset_of!(KeyDict_cmd_mods, keepjumps),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepmarks"),
        ptr_off: offset_of!(KeyDict_cmd_mods, keepmarks),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"lockmarks"),
        ptr_off: offset_of!(KeyDict_cmd_mods, lockmarks),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noautocmd"),
        ptr_off: offset_of!(KeyDict_cmd_mods, noautocmd),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"horizontal"),
        ptr_off: offset_of!(KeyDict_cmd_mods, horizontal),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noswapfile"),
        ptr_off: offset_of!(KeyDict_cmd_mods, noswapfile),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"emsg_silent"),
        ptr_off: offset_of!(KeyDict_cmd_mods, emsg_silent),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keeppatterns"),
        ptr_off: offset_of!(KeyDict_cmd_mods, keeppatterns),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn cmd_mods_hash(key: &[u8]) -> i32 {
    match key {
        b"tab" => 0,
        b"hide" => 1,
        b"split" => 2,
        b"browse" => 3,
        b"filter" => 4,
        b"silent" => 5,
        b"confirm" => 6,
        b"keepalt" => 7,
        b"sandbox" => 8,
        b"verbose" => 9,
        b"unsilent" => 10,
        b"vertical" => 11,
        b"keepjumps" => 12,
        b"keepmarks" => 13,
        b"lockmarks" => 14,
        b"noautocmd" => 15,
        b"horizontal" => 16,
        b"noswapfile" => 17,
        b"emsg_silent" => 18,
        b"keeppatterns" => 19,
        _ => -1,
    }
}

pub fn key_dict_cmd_mods_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = cmd_mods_hash(key);
    (hash >= 0).then(|| &CMD_MODS_TABLE[hash as usize])
}

pub static CMD_MODS_FILTER_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"force"),
        ptr_off: offset_of!(KeyDict_cmd_mods_filter, force),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        ptr_off: offset_of!(KeyDict_cmd_mods_filter, pattern),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn cmd_mods_filter_hash(key: &[u8]) -> i32 {
    match key {
        b"force" => 0,
        b"pattern" => 1,
        _ => -1,
    }
}

pub fn key_dict_cmd_mods_filter_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = cmd_mods_filter_hash(key);
    (hash >= 0).then(|| &CMD_MODS_FILTER_TABLE[hash as usize])
}

pub static CMD_OPTS_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"output"),
        ptr_off: offset_of!(KeyDict_cmd_opts, output),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn cmd_opts_hash(key: &[u8]) -> i32 {
    match key {
        b"output" => 0,
        _ => -1,
    }
}

pub fn key_dict_cmd_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = cmd_opts_hash(key);
    (hash >= 0).then(|| &CMD_OPTS_TABLE[hash as usize])
}

pub static COMPLETE_SET_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"info"),
        ptr_off: offset_of!(KeyDict_complete_set, info),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn complete_set_hash(key: &[u8]) -> i32 {
    match key {
        b"info" => 0,
        _ => -1,
    }
}

pub fn key_dict_complete_set_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = complete_set_hash(key);
    (hash >= 0).then(|| &COMPLETE_SET_TABLE[hash as usize])
}

pub static CONTEXT_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"types"),
        ptr_off: offset_of!(KeyDict_context, types),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn context_hash(key: &[u8]) -> i32 {
    match key {
        b"types" => 0,
        _ => -1,
    }
}

pub fn key_dict_context_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = context_hash(key);
    (hash >= 0).then(|| &CONTEXT_TABLE[hash as usize])
}

pub static CREATE_AUGROUP_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"clear"),
        ptr_off: offset_of!(KeyDict_create_augroup, clear),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn create_augroup_hash(key: &[u8]) -> i32 {
    match key {
        b"clear" => 0,
        _ => -1,
    }
}

pub fn key_dict_create_augroup_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = create_augroup_hash(key);
    (hash >= 0).then(|| &CREATE_AUGROUP_TABLE[hash as usize])
}

pub static CREATE_AUTOCMD_TABLE: [KeySetLink; 10] = [
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_create_autocmd, buf),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"desc"),
        ptr_off: offset_of!(KeyDict_create_autocmd, desc),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"once"),
        ptr_off: offset_of!(KeyDict_create_autocmd, once),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        ptr_off: offset_of!(KeyDict_create_autocmd, group),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        ptr_off: offset_of!(KeyDict_create_autocmd, buffer),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nested"),
        ptr_off: offset_of!(KeyDict_create_autocmd, nested),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"command"),
        ptr_off: offset_of!(KeyDict_create_autocmd, command),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        ptr_off: offset_of!(KeyDict_create_autocmd, pattern),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"callback"),
        ptr_off: offset_of!(KeyDict_create_autocmd, callback),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn create_autocmd_hash(key: &[u8]) -> i32 {
    match key {
        b"buf" => 0,
        b"desc" => 1,
        b"once" => 2,
        b"group" => 3,
        b"buffer" => 4,
        b"nested" => 5,
        b"command" => 6,
        b"pattern" => 7,
        b"callback" => 8,
        _ => -1,
    }
}

pub fn key_dict_create_autocmd_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = create_autocmd_hash(key);
    (hash >= 0).then(|| &CREATE_AUTOCMD_TABLE[hash as usize])
}

pub static ECHO_OPTS_TABLE: [KeySetLink; 11] = [
    KeySetLink {
        str: Some(c"id"),
        ptr_off: offset_of!(KeyDict_echo_opts, id),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"err"),
        ptr_off: offset_of!(KeyDict_echo_opts, err),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"data"),
        ptr_off: offset_of!(KeyDict_echo_opts, data),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"kind"),
        ptr_off: offset_of!(KeyDict_echo_opts, kind),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title"),
        ptr_off: offset_of!(KeyDict_echo_opts, title),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"source"),
        ptr_off: offset_of!(KeyDict_echo_opts, source),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"status"),
        ptr_off: offset_of!(KeyDict_echo_opts, status),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"percent"),
        ptr_off: offset_of!(KeyDict_echo_opts, percent),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"verbose"),
        ptr_off: offset_of!(KeyDict_echo_opts, verbose),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_truncate"),
        ptr_off: offset_of!(KeyDict_echo_opts, _truncate),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn echo_opts_hash(key: &[u8]) -> i32 {
    match key {
        b"id" => 0,
        b"err" => 1,
        b"data" => 2,
        b"kind" => 3,
        b"title" => 4,
        b"source" => 5,
        b"status" => 6,
        b"percent" => 7,
        b"verbose" => 8,
        b"_truncate" => 9,
        _ => -1,
    }
}

pub fn key_dict_echo_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = echo_opts_hash(key);
    (hash >= 0).then(|| &ECHO_OPTS_TABLE[hash as usize])
}

pub static EMPTY_TABLE: [KeySetLink; 1] = [KeySetLink {
    str: None,
    ptr_off: 0,
    key_type: ObjectType::ObjectTypeNil as i32,
    opt_index: -1,
    is_hlgroup: false,
}];

pub fn empty_hash(key: &[u8]) -> i32 {
    let _ = key;
    -1
}

pub fn key_dict_empty_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = empty_hash(key);
    (hash >= 0).then(|| &EMPTY_TABLE[hash as usize])
}

pub static EVAL_STATUSLINE_TABLE: [KeySetLink; 8] = [
    KeySetLink {
        str: Some(c"winid"),
        ptr_off: offset_of!(KeyDict_eval_statusline, winid),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fillchar"),
        ptr_off: offset_of!(KeyDict_eval_statusline, fillchar),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"maxwidth"),
        ptr_off: offset_of!(KeyDict_eval_statusline, maxwidth),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"highlights"),
        ptr_off: offset_of!(KeyDict_eval_statusline, highlights),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_winbar"),
        ptr_off: offset_of!(KeyDict_eval_statusline, use_winbar),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_tabline"),
        ptr_off: offset_of!(KeyDict_eval_statusline, use_tabline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_statuscol_lnum"),
        ptr_off: offset_of!(KeyDict_eval_statusline, use_statuscol_lnum),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn eval_statusline_hash(key: &[u8]) -> i32 {
    match key {
        b"winid" => 0,
        b"fillchar" => 1,
        b"maxwidth" => 2,
        b"highlights" => 3,
        b"use_winbar" => 4,
        b"use_tabline" => 5,
        b"use_statuscol_lnum" => 6,
        _ => -1,
    }
}

pub fn key_dict_eval_statusline_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = eval_statusline_hash(key);
    (hash >= 0).then(|| &EVAL_STATUSLINE_TABLE[hash as usize])
}

pub static EXEC_AUTOCMDS_TABLE: [KeySetLink; 7] = [
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, buf),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"data"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, data),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, group),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, buffer),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, pattern),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"modeline"),
        ptr_off: offset_of!(KeyDict_exec_autocmds, modeline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn exec_autocmds_hash(key: &[u8]) -> i32 {
    match key {
        b"buf" => 0,
        b"data" => 1,
        b"group" => 2,
        b"buffer" => 3,
        b"pattern" => 4,
        b"modeline" => 5,
        _ => -1,
    }
}

pub fn key_dict_exec_autocmds_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = exec_autocmds_hash(key);
    (hash >= 0).then(|| &EXEC_AUTOCMDS_TABLE[hash as usize])
}

pub static EXEC_OPTS_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"output"),
        ptr_off: offset_of!(KeyDict_exec_opts, output),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn exec_opts_hash(key: &[u8]) -> i32 {
    match key {
        b"output" => 0,
        _ => -1,
    }
}

pub fn key_dict_exec_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = exec_opts_hash(key);
    (hash >= 0).then(|| &EXEC_OPTS_TABLE[hash as usize])
}

pub static GET_AUTOCMDS_TABLE: [KeySetLink; 7] = [
    KeySetLink {
        str: Some(c"id"),
        ptr_off: offset_of!(KeyDict_get_autocmds, id),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_get_autocmds, buf),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"event"),
        ptr_off: offset_of!(KeyDict_get_autocmds, event),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        ptr_off: offset_of!(KeyDict_get_autocmds, group),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        ptr_off: offset_of!(KeyDict_get_autocmds, buffer),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        ptr_off: offset_of!(KeyDict_get_autocmds, pattern),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_autocmds_hash(key: &[u8]) -> i32 {
    match key {
        b"id" => 0,
        b"buf" => 1,
        b"event" => 2,
        b"group" => 3,
        b"buffer" => 4,
        b"pattern" => 5,
        _ => -1,
    }
}

pub fn key_dict_get_autocmds_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_autocmds_hash(key);
    (hash >= 0).then(|| &GET_AUTOCMDS_TABLE[hash as usize])
}

pub static GET_COMMANDS_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"builtin"),
        ptr_off: offset_of!(KeyDict_get_commands, builtin),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_commands_hash(key: &[u8]) -> i32 {
    match key {
        b"builtin" => 0,
        _ => -1,
    }
}

pub fn key_dict_get_commands_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_commands_hash(key);
    (hash >= 0).then(|| &GET_COMMANDS_TABLE[hash as usize])
}

pub static GET_EXTMARK_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"details"),
        ptr_off: offset_of!(KeyDict_get_extmark, details),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_name"),
        ptr_off: offset_of!(KeyDict_get_extmark, hl_name),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_extmark_hash(key: &[u8]) -> i32 {
    match key {
        b"details" => 0,
        b"hl_name" => 1,
        _ => -1,
    }
}

pub fn key_dict_get_extmark_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_extmark_hash(key);
    (hash >= 0).then(|| &GET_EXTMARK_TABLE[hash as usize])
}

pub static GET_EXTMARKS_TABLE: [KeySetLink; 6] = [
    KeySetLink {
        str: Some(c"type"),
        ptr_off: offset_of!(KeyDict_get_extmarks, type_),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"limit"),
        ptr_off: offset_of!(KeyDict_get_extmarks, limit),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"details"),
        ptr_off: offset_of!(KeyDict_get_extmarks, details),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_name"),
        ptr_off: offset_of!(KeyDict_get_extmarks, hl_name),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overlap"),
        ptr_off: offset_of!(KeyDict_get_extmarks, overlap),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_extmarks_hash(key: &[u8]) -> i32 {
    match key {
        b"type" => 0,
        b"limit" => 1,
        b"details" => 2,
        b"hl_name" => 3,
        b"overlap" => 4,
        _ => -1,
    }
}

pub fn key_dict_get_extmarks_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_extmarks_hash(key);
    (hash >= 0).then(|| &GET_EXTMARKS_TABLE[hash as usize])
}

pub static GET_HIGHLIGHT_TABLE: [KeySetLink; 5] = [
    KeySetLink {
        str: Some(c"id"),
        ptr_off: offset_of!(KeyDict_get_highlight, id),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link"),
        ptr_off: offset_of!(KeyDict_get_highlight, link),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"name"),
        ptr_off: offset_of!(KeyDict_get_highlight, name),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"create"),
        ptr_off: offset_of!(KeyDict_get_highlight, create),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_highlight_hash(key: &[u8]) -> i32 {
    match key {
        b"id" => 0,
        b"link" => 1,
        b"name" => 2,
        b"create" => 3,
        _ => -1,
    }
}

pub fn key_dict_get_highlight_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_highlight_hash(key);
    (hash >= 0).then(|| &GET_HIGHLIGHT_TABLE[hash as usize])
}

pub static GET_NS_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"winid"),
        ptr_off: offset_of!(KeyDict_get_ns, winid),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn get_ns_hash(key: &[u8]) -> i32 {
    match key {
        b"winid" => 0,
        _ => -1,
    }
}

pub fn key_dict_get_ns_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = get_ns_hash(key);
    (hash >= 0).then(|| &GET_NS_TABLE[hash as usize])
}

pub static HIGHLIGHT_TABLE: [KeySetLink; 37] = [
    KeySetLink {
        str: Some(c"bg"),
        ptr_off: offset_of!(KeyDict_highlight, bg),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fg"),
        ptr_off: offset_of!(KeyDict_highlight, fg),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sp"),
        ptr_off: offset_of!(KeyDict_highlight, sp),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"dim"),
        ptr_off: offset_of!(KeyDict_highlight, dim),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"url"),
        ptr_off: offset_of!(KeyDict_highlight, url),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bold"),
        ptr_off: offset_of!(KeyDict_highlight, bold),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"font"),
        ptr_off: offset_of!(KeyDict_highlight, font),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link"),
        ptr_off: offset_of!(KeyDict_highlight, link),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 8,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"blend"),
        ptr_off: offset_of!(KeyDict_highlight, blend),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force"),
        ptr_off: offset_of!(KeyDict_highlight, force),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"blink"),
        ptr_off: offset_of!(KeyDict_highlight, blink),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cterm"),
        ptr_off: offset_of!(KeyDict_highlight, cterm),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"italic"),
        ptr_off: offset_of!(KeyDict_highlight, italic),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"update"),
        ptr_off: offset_of!(KeyDict_highlight, update),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reverse"),
        ptr_off: offset_of!(KeyDict_highlight, reverse),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"default"),
        ptr_off: offset_of!(KeyDict_highlight, default_),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"altfont"),
        ptr_off: offset_of!(KeyDict_highlight, altfont),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        ptr_off: offset_of!(KeyDict_highlight, conceal),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"special"),
        ptr_off: offset_of!(KeyDict_highlight, special),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ctermfg"),
        ptr_off: offset_of!(KeyDict_highlight, ctermfg),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ctermbg"),
        ptr_off: offset_of!(KeyDict_highlight, ctermbg),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fallback"),
        ptr_off: offset_of!(KeyDict_highlight, fallback),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overline"),
        ptr_off: offset_of!(KeyDict_highlight, overline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 23,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"standout"),
        ptr_off: offset_of!(KeyDict_highlight, standout),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nocombine"),
        ptr_off: offset_of!(KeyDict_highlight, nocombine),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 25,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undercurl"),
        ptr_off: offset_of!(KeyDict_highlight, undercurl),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 26,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underline"),
        ptr_off: offset_of!(KeyDict_highlight, underline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 27,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"background"),
        ptr_off: offset_of!(KeyDict_highlight, background),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 28,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bg_indexed"),
        ptr_off: offset_of!(KeyDict_highlight, bg_indexed),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 29,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"foreground"),
        ptr_off: offset_of!(KeyDict_highlight, foreground),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 30,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fg_indexed"),
        ptr_off: offset_of!(KeyDict_highlight, fg_indexed),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 31,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link_global"),
        ptr_off: offset_of!(KeyDict_highlight, link_global),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 32,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"underdashed"),
        ptr_off: offset_of!(KeyDict_highlight, underdashed),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 33,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdotted"),
        ptr_off: offset_of!(KeyDict_highlight, underdotted),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 34,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdouble"),
        ptr_off: offset_of!(KeyDict_highlight, underdouble),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 35,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strikethrough"),
        ptr_off: offset_of!(KeyDict_highlight, strikethrough),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 36,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn highlight_hash(key: &[u8]) -> i32 {
    match key {
        b"bg" => 0,
        b"fg" => 1,
        b"sp" => 2,
        b"dim" => 3,
        b"url" => 4,
        b"bold" => 5,
        b"font" => 6,
        b"link" => 7,
        b"blend" => 8,
        b"force" => 9,
        b"blink" => 10,
        b"cterm" => 11,
        b"italic" => 12,
        b"update" => 13,
        b"reverse" => 14,
        b"default" => 15,
        b"altfont" => 16,
        b"conceal" => 17,
        b"special" => 18,
        b"ctermfg" => 19,
        b"ctermbg" => 20,
        b"fallback" => 21,
        b"overline" => 22,
        b"standout" => 23,
        b"nocombine" => 24,
        b"undercurl" => 25,
        b"underline" => 26,
        b"background" => 27,
        b"bg_indexed" => 28,
        b"foreground" => 29,
        b"fg_indexed" => 30,
        b"link_global" => 31,
        b"underdashed" => 32,
        b"underdotted" => 33,
        b"underdouble" => 34,
        b"strikethrough" => 35,
        _ => -1,
    }
}

pub fn key_dict_highlight_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = highlight_hash(key);
    (hash >= 0).then(|| &HIGHLIGHT_TABLE[hash as usize])
}

pub static HIGHLIGHT_CTERM_TABLE: [KeySetLink; 17] = [
    KeySetLink {
        str: Some(c"dim"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, dim),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bold"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, bold),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"blink"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, blink),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"italic"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, italic),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"altfont"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, altfont),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, conceal),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reverse"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, reverse),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overline"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, overline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"standout"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, standout),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nocombine"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, nocombine),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undercurl"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, undercurl),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underline"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, underline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdashed"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, underdashed),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdotted"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, underdotted),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdouble"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, underdouble),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strikethrough"),
        ptr_off: offset_of!(KeyDict_highlight_cterm, strikethrough),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn highlight_cterm_hash(key: &[u8]) -> i32 {
    match key {
        b"dim" => 0,
        b"bold" => 1,
        b"blink" => 2,
        b"italic" => 3,
        b"altfont" => 4,
        b"conceal" => 5,
        b"reverse" => 6,
        b"overline" => 7,
        b"standout" => 8,
        b"nocombine" => 9,
        b"undercurl" => 10,
        b"underline" => 11,
        b"underdashed" => 12,
        b"underdotted" => 13,
        b"underdouble" => 14,
        b"strikethrough" => 15,
        _ => -1,
    }
}

pub fn key_dict_highlight_cterm_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = highlight_cterm_hash(key);
    (hash >= 0).then(|| &HIGHLIGHT_CTERM_TABLE[hash as usize])
}

pub static KEYMAP_TABLE: [KeySetLink; 10] = [
    KeySetLink {
        str: Some(c"desc"),
        ptr_off: offset_of!(KeyDict_keymap, desc),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"expr"),
        ptr_off: offset_of!(KeyDict_keymap, expr),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"script"),
        ptr_off: offset_of!(KeyDict_keymap, script),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"silent"),
        ptr_off: offset_of!(KeyDict_keymap, silent),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unique"),
        ptr_off: offset_of!(KeyDict_keymap, unique),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nowait"),
        ptr_off: offset_of!(KeyDict_keymap, nowait),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noremap"),
        ptr_off: offset_of!(KeyDict_keymap, noremap),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"callback"),
        ptr_off: offset_of!(KeyDict_keymap, callback),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"replace_keycodes"),
        ptr_off: offset_of!(KeyDict_keymap, replace_keycodes),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn keymap_hash(key: &[u8]) -> i32 {
    match key {
        b"desc" => 0,
        b"expr" => 1,
        b"script" => 2,
        b"silent" => 3,
        b"unique" => 4,
        b"nowait" => 5,
        b"noremap" => 6,
        b"callback" => 7,
        b"replace_keycodes" => 8,
        _ => -1,
    }
}

pub fn key_dict_keymap_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = keymap_hash(key);
    (hash >= 0).then(|| &KEYMAP_TABLE[hash as usize])
}

pub static KEYMAP_DEL_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"lhs"),
        ptr_off: offset_of!(KeyDict_keymap_del, lhs),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn keymap_del_hash(key: &[u8]) -> i32 {
    match key {
        b"lhs" => 0,
        _ => -1,
    }
}

pub fn key_dict_keymap_del_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = keymap_del_hash(key);
    (hash >= 0).then(|| &KEYMAP_DEL_TABLE[hash as usize])
}

pub static NS_OPTS_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"wins"),
        ptr_off: offset_of!(KeyDict_ns_opts, wins),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn ns_opts_hash(key: &[u8]) -> i32 {
    match key {
        b"wins" => 0,
        _ => -1,
    }
}

pub fn key_dict_ns_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = ns_opts_hash(key);
    (hash >= 0).then(|| &NS_OPTS_TABLE[hash as usize])
}

pub static OPEN_TERM_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"on_input"),
        ptr_off: offset_of!(KeyDict_open_term, on_input),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force_crlf"),
        ptr_off: offset_of!(KeyDict_open_term, force_crlf),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn open_term_hash(key: &[u8]) -> i32 {
    match key {
        b"on_input" => 0,
        b"force_crlf" => 1,
        _ => -1,
    }
}

pub fn key_dict_open_term_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = open_term_hash(key);
    (hash >= 0).then(|| &OPEN_TERM_TABLE[hash as usize])
}

pub static OPTION_TABLE: [KeySetLink; 8] = [
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_option, buf),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"tab"),
        ptr_off: offset_of!(KeyDict_option, tab),
        key_type: ObjectType::ObjectTypeTabpage as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        ptr_off: offset_of!(KeyDict_option, win),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"scope"),
        ptr_off: offset_of!(KeyDict_option, scope),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"dry_run"),
        ptr_off: offset_of!(KeyDict_option, dry_run),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filetype"),
        ptr_off: offset_of!(KeyDict_option, filetype),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"operation"),
        ptr_off: offset_of!(KeyDict_option, operation),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn option_hash(key: &[u8]) -> i32 {
    match key {
        b"buf" => 0,
        b"tab" => 1,
        b"win" => 2,
        b"scope" => 3,
        b"dry_run" => 4,
        b"filetype" => 5,
        b"operation" => 6,
        _ => -1,
    }
}

pub fn key_dict_option_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = option_hash(key);
    (hash >= 0).then(|| &OPTION_TABLE[hash as usize])
}

pub static REDRAW_TABLE: [KeySetLink; 11] = [
    KeySetLink {
        str: Some(c"buf"),
        ptr_off: offset_of!(KeyDict_redraw, buf),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        ptr_off: offset_of!(KeyDict_redraw, win),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"flush"),
        ptr_off: offset_of!(KeyDict_redraw, flush),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        ptr_off: offset_of!(KeyDict_redraw, range),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"valid"),
        ptr_off: offset_of!(KeyDict_redraw, valid),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cursor"),
        ptr_off: offset_of!(KeyDict_redraw, cursor),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"winbar"),
        ptr_off: offset_of!(KeyDict_redraw, winbar),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"tabline"),
        ptr_off: offset_of!(KeyDict_redraw, tabline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"statusline"),
        ptr_off: offset_of!(KeyDict_redraw, statusline),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"statuscolumn"),
        ptr_off: offset_of!(KeyDict_redraw, statuscolumn),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn redraw_hash(key: &[u8]) -> i32 {
    match key {
        b"buf" => 0,
        b"win" => 1,
        b"flush" => 2,
        b"range" => 3,
        b"valid" => 4,
        b"cursor" => 5,
        b"winbar" => 6,
        b"tabline" => 7,
        b"statusline" => 8,
        b"statuscolumn" => 9,
        _ => -1,
    }
}

pub fn key_dict_redraw_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = redraw_hash(key);
    (hash >= 0).then(|| &REDRAW_TABLE[hash as usize])
}

pub static RUNTIME_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"is_lua"),
        ptr_off: offset_of!(KeyDict_runtime, is_lua),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"do_source"),
        ptr_off: offset_of!(KeyDict_runtime, do_source),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn runtime_hash(key: &[u8]) -> i32 {
    match key {
        b"is_lua" => 0,
        b"do_source" => 1,
        _ => -1,
    }
}

pub fn key_dict_runtime_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = runtime_hash(key);
    (hash >= 0).then(|| &RUNTIME_TABLE[hash as usize])
}

pub static SET_DECORATION_PROVIDER_TABLE: [KeySetLink; 10] = [
    KeySetLink {
        str: Some(c"on_buf"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_buf),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_end"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_end),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_win"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_win),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_line"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_line),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_range"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_range),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_start"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, on_start),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_hl_def"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, _on_hl_def),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_spell_nav"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, _on_spell_nav),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_conceal_line"),
        ptr_off: offset_of!(KeyDict_set_decoration_provider, _on_conceal_line),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn set_decoration_provider_hash(key: &[u8]) -> i32 {
    match key {
        b"on_buf" => 0,
        b"on_end" => 1,
        b"on_win" => 2,
        b"on_line" => 3,
        b"on_range" => 4,
        b"on_start" => 5,
        b"_on_hl_def" => 6,
        b"_on_spell_nav" => 7,
        b"_on_conceal_line" => 8,
        _ => -1,
    }
}

pub fn key_dict_set_decoration_provider_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = set_decoration_provider_hash(key);
    (hash >= 0).then(|| &SET_DECORATION_PROVIDER_TABLE[hash as usize])
}

pub static SET_EXTMARK_TABLE: [KeySetLink; 36] = [
    KeySetLink {
        str: Some(c"id"),
        ptr_off: offset_of!(KeyDict_set_extmark, id),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"url"),
        ptr_off: offset_of!(KeyDict_set_extmark, url),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"spell"),
        ptr_off: offset_of!(KeyDict_set_extmark, spell),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"scoped"),
        ptr_off: offset_of!(KeyDict_set_extmark, scoped),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_eol"),
        ptr_off: offset_of!(KeyDict_set_extmark, hl_eol),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strict"),
        ptr_off: offset_of!(KeyDict_set_extmark, strict),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_col"),
        ptr_off: offset_of!(KeyDict_set_extmark, end_col),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        ptr_off: offset_of!(KeyDict_set_extmark, conceal),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_mode"),
        ptr_off: offset_of!(KeyDict_set_extmark, hl_mode),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_row"),
        ptr_off: offset_of!(KeyDict_set_extmark, end_row),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_line"),
        ptr_off: offset_of!(KeyDict_set_extmark, end_line),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_group"),
        ptr_off: offset_of!(KeyDict_set_extmark, hl_group),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"priority"),
        ptr_off: offset_of!(KeyDict_set_extmark, priority),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ephemeral"),
        ptr_off: offset_of!(KeyDict_set_extmark, ephemeral),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sign_text"),
        ptr_off: offset_of!(KeyDict_set_extmark, sign_text),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_text),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"invalidate"),
        ptr_off: offset_of!(KeyDict_set_extmark, invalidate),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ui_watched"),
        ptr_off: offset_of!(KeyDict_set_extmark, ui_watched),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_lines"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_lines),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_subpriority"),
        ptr_off: offset_of!(KeyDict_set_extmark, _subpriority),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undo_restore"),
        ptr_off: offset_of!(KeyDict_set_extmark, undo_restore),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal_lines"),
        ptr_off: offset_of!(KeyDict_set_extmark, conceal_lines),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"line_hl_group"),
        ptr_off: offset_of!(KeyDict_set_extmark, line_hl_group),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 23,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"right_gravity"),
        ptr_off: offset_of!(KeyDict_set_extmark, right_gravity),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sign_hl_group"),
        ptr_off: offset_of!(KeyDict_set_extmark, sign_hl_group),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 25,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_text_pos"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_text_pos),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 26,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_hide"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_text_hide),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 27,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"number_hl_group"),
        ptr_off: offset_of!(KeyDict_set_extmark, number_hl_group),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 28,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_lines_above"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_lines_above),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 29,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_right_gravity"),
        ptr_off: offset_of!(KeyDict_set_extmark, end_right_gravity),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 30,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_win_col"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_text_win_col),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 31,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_lines_leftcol"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_lines_leftcol),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 32,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cursorline_hl_group"),
        ptr_off: offset_of!(KeyDict_set_extmark, cursorline_hl_group),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 33,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_lines_overflow"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_lines_overflow),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 34,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_repeat_linebreak"),
        ptr_off: offset_of!(KeyDict_set_extmark, virt_text_repeat_linebreak),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 35,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn set_extmark_hash(key: &[u8]) -> i32 {
    match key {
        b"id" => 0,
        b"url" => 1,
        b"spell" => 2,
        b"scoped" => 3,
        b"hl_eol" => 4,
        b"strict" => 5,
        b"end_col" => 6,
        b"conceal" => 7,
        b"hl_mode" => 8,
        b"end_row" => 9,
        b"end_line" => 10,
        b"hl_group" => 11,
        b"priority" => 12,
        b"ephemeral" => 13,
        b"sign_text" => 14,
        b"virt_text" => 15,
        b"invalidate" => 16,
        b"ui_watched" => 17,
        b"virt_lines" => 18,
        b"_subpriority" => 19,
        b"undo_restore" => 20,
        b"conceal_lines" => 21,
        b"line_hl_group" => 22,
        b"right_gravity" => 23,
        b"sign_hl_group" => 24,
        b"virt_text_pos" => 25,
        b"virt_text_hide" => 26,
        b"number_hl_group" => 27,
        b"virt_lines_above" => 28,
        b"end_right_gravity" => 29,
        b"virt_text_win_col" => 30,
        b"virt_lines_leftcol" => 31,
        b"cursorline_hl_group" => 32,
        b"virt_lines_overflow" => 33,
        b"virt_text_repeat_linebreak" => 34,
        _ => -1,
    }
}

pub fn key_dict_set_extmark_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = set_extmark_hash(key);
    (hash >= 0).then(|| &SET_EXTMARK_TABLE[hash as usize])
}

pub static TABPAGE_CONFIG_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"after"),
        ptr_off: offset_of!(KeyDict_tabpage_config, after),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn tabpage_config_hash(key: &[u8]) -> i32 {
    match key {
        b"after" => 0,
        _ => -1,
    }
}

pub fn key_dict_tabpage_config_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = tabpage_config_hash(key);
    (hash >= 0).then(|| &TABPAGE_CONFIG_TABLE[hash as usize])
}

pub static USER_COMMAND_TABLE: [KeySetLink; 13] = [
    KeySetLink {
        str: Some(c"bar"),
        ptr_off: offset_of!(KeyDict_user_command, bar),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"addr"),
        ptr_off: offset_of!(KeyDict_user_command, addr),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bang"),
        ptr_off: offset_of!(KeyDict_user_command, bang),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"desc"),
        ptr_off: offset_of!(KeyDict_user_command, desc),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"count"),
        ptr_off: offset_of!(KeyDict_user_command, count),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force"),
        ptr_off: offset_of!(KeyDict_user_command, force),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nargs"),
        ptr_off: offset_of!(KeyDict_user_command, nargs),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        ptr_off: offset_of!(KeyDict_user_command, range),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"preview"),
        ptr_off: offset_of!(KeyDict_user_command, preview),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"complete"),
        ptr_off: offset_of!(KeyDict_user_command, complete),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"register"),
        ptr_off: offset_of!(KeyDict_user_command, register_),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepscript"),
        ptr_off: offset_of!(KeyDict_user_command, keepscript),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn user_command_hash(key: &[u8]) -> i32 {
    match key {
        b"bar" => 0,
        b"addr" => 1,
        b"bang" => 2,
        b"desc" => 3,
        b"count" => 4,
        b"force" => 5,
        b"nargs" => 6,
        b"range" => 7,
        b"preview" => 8,
        b"complete" => 9,
        b"register" => 10,
        b"keepscript" => 11,
        _ => -1,
    }
}

pub fn key_dict_user_command_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = user_command_hash(key);
    (hash >= 0).then(|| &USER_COMMAND_TABLE[hash as usize])
}

pub static WIN_CONFIG_TABLE: [KeySetLink; 25] = [
    KeySetLink {
        str: Some(c"col"),
        ptr_off: offset_of!(KeyDict_win_config, col),
        key_type: ObjectType::ObjectTypeFloat as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"row"),
        ptr_off: offset_of!(KeyDict_win_config, row),
        key_type: ObjectType::ObjectTypeFloat as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        ptr_off: offset_of!(KeyDict_win_config, win),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hide"),
        ptr_off: offset_of!(KeyDict_win_config, hide),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"width"),
        ptr_off: offset_of!(KeyDict_win_config, width),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"split"),
        ptr_off: offset_of!(KeyDict_win_config, split),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title"),
        ptr_off: offset_of!(KeyDict_win_config, title),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"mouse"),
        ptr_off: offset_of!(KeyDict_win_config, mouse),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fixed"),
        ptr_off: offset_of!(KeyDict_win_config, fixed),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"style"),
        ptr_off: offset_of!(KeyDict_win_config, style),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"anchor"),
        ptr_off: offset_of!(KeyDict_win_config, anchor),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bufpos"),
        ptr_off: offset_of!(KeyDict_win_config, bufpos),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"height"),
        ptr_off: offset_of!(KeyDict_win_config, height),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"zindex"),
        ptr_off: offset_of!(KeyDict_win_config, zindex),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"footer"),
        ptr_off: offset_of!(KeyDict_win_config, footer),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"border"),
        ptr_off: offset_of!(KeyDict_win_config, border),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"external"),
        ptr_off: offset_of!(KeyDict_win_config, external),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"relative"),
        ptr_off: offset_of!(KeyDict_win_config, relative),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        ptr_off: offset_of!(KeyDict_win_config, vertical),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"focusable"),
        ptr_off: offset_of!(KeyDict_win_config, focusable),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noautocmd"),
        ptr_off: offset_of!(KeyDict_win_config, noautocmd),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title_pos"),
        ptr_off: offset_of!(KeyDict_win_config, title_pos),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"footer_pos"),
        ptr_off: offset_of!(KeyDict_win_config, footer_pos),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 23,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_cmdline_offset"),
        ptr_off: offset_of!(KeyDict_win_config, _cmdline_offset),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn win_config_hash(key: &[u8]) -> i32 {
    match key {
        b"col" => 0,
        b"row" => 1,
        b"win" => 2,
        b"hide" => 3,
        b"width" => 4,
        b"split" => 5,
        b"title" => 6,
        b"mouse" => 7,
        b"fixed" => 8,
        b"style" => 9,
        b"anchor" => 10,
        b"bufpos" => 11,
        b"height" => 12,
        b"zindex" => 13,
        b"footer" => 14,
        b"border" => 15,
        b"external" => 16,
        b"relative" => 17,
        b"vertical" => 18,
        b"focusable" => 19,
        b"noautocmd" => 20,
        b"title_pos" => 21,
        b"footer_pos" => 22,
        b"_cmdline_offset" => 23,
        _ => -1,
    }
}

pub fn key_dict_win_config_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = win_config_hash(key);
    (hash >= 0).then(|| &WIN_CONFIG_TABLE[hash as usize])
}

pub static WIN_RESIZE_TABLE: [KeySetLink; 2] = [
    KeySetLink {
        str: Some(c"anchor"),
        ptr_off: offset_of!(KeyDict_win_resize, anchor),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn win_resize_hash(key: &[u8]) -> i32 {
    match key {
        b"anchor" => 0,
        _ => -1,
    }
}

pub fn key_dict_win_resize_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = win_resize_hash(key);
    (hash >= 0).then(|| &WIN_RESIZE_TABLE[hash as usize])
}

pub static WIN_TEXT_HEIGHT_TABLE: [KeySetLink; 6] = [
    KeySetLink {
        str: Some(c"end_row"),
        ptr_off: offset_of!(KeyDict_win_text_height, end_row),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_vcol"),
        ptr_off: offset_of!(KeyDict_win_text_height, end_vcol),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"start_row"),
        ptr_off: offset_of!(KeyDict_win_text_height, start_row),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"max_height"),
        ptr_off: offset_of!(KeyDict_win_text_height, max_height),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"start_vcol"),
        ptr_off: offset_of!(KeyDict_win_text_height, start_vcol),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn win_text_height_hash(key: &[u8]) -> i32 {
    match key {
        b"end_row" => 0,
        b"end_vcol" => 1,
        b"start_row" => 2,
        b"max_height" => 3,
        b"start_vcol" => 4,
        _ => -1,
    }
}

pub fn key_dict_win_text_height_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = win_text_height_hash(key);
    (hash >= 0).then(|| &WIN_TEXT_HEIGHT_TABLE[hash as usize])
}

pub static XDL_DIFF_TABLE: [KeySetLink; 13] = [
    KeySetLink {
        str: Some(c"ctxlen"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ctxlen),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_hunk"),
        ptr_off: offset_of!(KeyDict_xdl_diff, on_hunk),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"algorithm"),
        ptr_off: offset_of!(KeyDict_xdl_diff, algorithm),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"linematch"),
        ptr_off: offset_of!(KeyDict_xdl_diff, linematch),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"result_type"),
        ptr_off: offset_of!(KeyDict_xdl_diff, result_type),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"interhunkctxlen"),
        ptr_off: offset_of!(KeyDict_xdl_diff, interhunkctxlen),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_cr_at_eol"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ignore_cr_at_eol),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"indent_heuristic"),
        ptr_off: offset_of!(KeyDict_xdl_diff, indent_heuristic),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ignore_whitespace),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_blank_lines"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ignore_blank_lines),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace_change"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ignore_whitespace_change),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace_change_at_eol"),
        ptr_off: offset_of!(KeyDict_xdl_diff, ignore_whitespace_change_at_eol),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn xdl_diff_hash(key: &[u8]) -> i32 {
    match key {
        b"ctxlen" => 0,
        b"on_hunk" => 1,
        b"algorithm" => 2,
        b"linematch" => 3,
        b"result_type" => 4,
        b"interhunkctxlen" => 5,
        b"ignore_cr_at_eol" => 6,
        b"indent_heuristic" => 7,
        b"ignore_whitespace" => 8,
        b"ignore_blank_lines" => 9,
        b"ignore_whitespace_change" => 10,
        b"ignore_whitespace_change_at_eol" => 11,
        _ => -1,
    }
}

pub fn key_dict_xdl_diff_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = xdl_diff_hash(key);
    (hash >= 0).then(|| &XDL_DIFF_TABLE[hash as usize])
}

pub fn handle_nvim_get_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_autocmds",
            c"API: vim -> nvim_get_autocmds",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: KeyDict_get_autocmds = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(args[0], &mut arg_1, Some(key_dict_get_autocmds_get_field), c"Wrong type for argument 1 when calling nvim_get_autocmds, expecting Dict(get_autocmds) *", error) {
                return ret;
            }
        let rv = nvim_get_autocmds(&mut arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_create_autocmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_create_autocmd",
            c"API: vim -> nvim_create_autocmd",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_2: KeyDict_create_autocmd = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        let arg_1: Object = args[0];
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_create_autocmd_get_field), c"Wrong type for argument 2 when calling nvim_create_autocmd, expecting Dict(create_autocmd) *", error) {
                return ret;
            }
        let rv = nvim_create_autocmd(channel_id, arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_del_autocmd(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_autocmd",
            c"API: vim -> nvim_del_autocmd",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_autocmd, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_del_autocmd(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_clear_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_clear_autocmds",
            c"API: vim -> nvim_clear_autocmds",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: KeyDict_clear_autocmds = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(args[0], &mut arg_1, Some(key_dict_clear_autocmds_get_field), c"Wrong type for argument 1 when calling nvim_clear_autocmds, expecting Dict(clear_autocmds) *", error) {
                return ret;
            }
        nvim_clear_autocmds(&mut arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_create_augroup(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_create_augroup",
            c"API: vim -> nvim_create_augroup",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_create_augroup = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_create_augroup, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_create_augroup_get_field), c"Wrong type for argument 2 when calling nvim_create_augroup, expecting Dict(create_augroup) *", error) {
                return ret;
            }
        let rv = nvim_create_augroup(channel_id, arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_del_augroup_by_id(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_augroup_by_id",
            c"API: vim -> nvim_del_augroup_by_id",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_augroup_by_id, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_del_augroup_by_id(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_del_augroup_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_augroup_by_name",
            c"API: vim -> nvim_del_augroup_by_name",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_augroup_by_name, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_del_augroup_by_name(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_exec_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_exec_autocmds",
            c"API: vim -> nvim_exec_autocmds",
        );
        let ret = OBJECT_NIL;
        let mut arg_2: KeyDict_exec_autocmds = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        let arg_1: Object = args[0];
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_exec_autocmds_get_field), c"Wrong type for argument 2 when calling nvim_exec_autocmds, expecting Dict(exec_autocmds) *", error) {
                return ret;
            }
        nvim_exec_autocmds(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_line_count(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_line_count",
            c"API: vim -> nvim_buf_line_count",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_line_count, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_line_count(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_attach",
            c"API: vim -> nvim_buf_attach",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: KeyDict_buf_attach = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_attach, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_attach, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if args.size >= 3
            && !extract_keydict(args[2], &mut arg_3, Some(key_dict_buf_attach_get_field), c"Wrong type for argument 3 when calling nvim_buf_attach, expecting Dict(buf_attach) *", error) {
                return ret;
            }
        let rv = nvim_buf_attach(channel_id, arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_detach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_detach",
            c"API: vim -> nvim_buf_detach",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_detach, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_detach(channel_id, arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_lines(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_lines",
            c"API: vim -> nvim_buf_get_lines",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Boolean = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_lines, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_lines, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_get_lines, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_get_lines, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_lines(
            channel_id,
            arg_1,
            arg_2,
            arg_3,
            arg_4,
            arena,
            ptr::null_mut(),
            error,
        );
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_lines(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_lines",
            c"API: vim -> nvim_buf_set_lines",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Boolean = Default::default();
        let mut arg_5: Array = Default::default();
        if args.size != 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_lines, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_lines, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_lines, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_lines, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_buf_set_lines, expecting ArrayOf(String)",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_buf_set_lines(channel_id, arg_1, arg_2, arg_3, arg_4, arg_5, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_set_text(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_text",
            c"API: vim -> nvim_buf_set_text",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: Integer = Default::default();
        let mut arg_6: Array = Default::default();
        if args.size != 6 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 6 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_text, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_buf_set_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[5],
            &mut arg_6,
            c"Wrong type for argument 6 when calling nvim_buf_set_text, expecting ArrayOf(String)",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_buf_set_text(
            channel_id, arg_1, arg_2, arg_3, arg_4, arg_5, arg_6, arena, error,
        );
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_get_text(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_text",
            c"API: vim -> nvim_buf_get_text",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: Integer = Default::default();
        let mut arg_6: KeyDict_empty = Default::default();
        if args.size < 5 || args.size > 6 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 5 to 6 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_text, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_get_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_get_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_buf_get_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 6
            && !extract_keydict(args[5], &mut arg_6, Some(key_dict_empty_get_field), c"Wrong type for argument 6 when calling nvim_buf_get_text, expecting Dict(empty) *", error) {
                return ret;
            }
        let rv = nvim_buf_get_text(
            channel_id,
            arg_1,
            arg_2,
            arg_3,
            arg_4,
            arg_5,
            &mut arg_6,
            arena,
            ptr::null_mut(),
            error,
        );
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_offset(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_offset",
            c"API: vim -> nvim_buf_get_offset",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_offset, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_offset, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_offset(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_var",
            c"API: vim -> nvim_buf_get_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_var, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_buf_get_changedtick(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_changedtick",
            c"API: vim -> nvim_buf_get_changedtick",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_changedtick, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_changedtick(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_keymap(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_keymap",
            c"API: vim -> nvim_buf_get_keymap",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_keymap, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_keymap(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_keymap",
            c"API: vim -> nvim_buf_set_keymap",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: NvimString = Default::default();
        let mut arg_5: KeyDict_keymap = Default::default();
        if args.size < 4 || args.size > 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 to 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_keymap, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 5
            && !extract_keydict(args[4], &mut arg_5, Some(key_dict_keymap_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_keymap, expecting Dict(keymap) *", error) {
                return ret;
            }
        nvim_buf_set_keymap(channel_id, arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_del_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_del_keymap",
            c"API: vim -> nvim_buf_del_keymap",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: KeyDict_keymap_del = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_del_keymap, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_del_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_del_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 4
            && !extract_keydict(args[3], &mut arg_4, Some(key_dict_keymap_del_get_field), c"Wrong type for argument 4 when calling nvim_buf_del_keymap, expecting Dict(keymap_del) *", error) {
                return ret;
            }
        nvim_buf_del_keymap(channel_id, arg_1, arg_2, arg_3, &mut arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_var",
            c"API: vim -> nvim_buf_set_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_var, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        nvim_buf_set_var(arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_del_var",
            c"API: vim -> nvim_buf_del_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_del_var, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_buf_del_var(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_get_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_name",
            c"API: vim -> nvim_buf_get_name",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_name, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_name(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_name",
            c"API: vim -> nvim_buf_set_name",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_name, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_name, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_buf_set_name(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_is_loaded(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_is_loaded",
            c"API: vim -> nvim_buf_is_loaded",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_is_loaded, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_is_loaded(arg_1);
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_delete(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_delete",
            c"API: vim -> nvim_buf_delete",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: KeyDict_buf_delete = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_delete, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_buf_delete_get_field), c"Wrong type for argument 2 when calling nvim_buf_delete, expecting Dict(buf_delete) *", error) {
                return ret;
            }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_buf_delete(arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_is_valid",
            c"API: vim -> nvim_buf_is_valid",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_is_valid, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_is_valid(arg_1);
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_del_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_del_mark",
            c"API: vim -> nvim_buf_del_mark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_del_mark, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_del_mark, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_del_mark(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_mark",
            c"API: vim -> nvim_buf_set_mark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: KeyDict_empty = Default::default();
        if args.size < 4 || args.size > 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 to 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_mark, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_mark, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_mark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_mark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 5
            && !extract_keydict(args[4], &mut arg_5, Some(key_dict_empty_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_mark, expecting Dict(empty) *", error) {
                return ret;
            }
        let rv = nvim_buf_set_mark(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_mark(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_mark",
            c"API: vim -> nvim_buf_get_mark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_mark, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_mark, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_mark(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_stats(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__buf_stats",
            c"API: vim -> nvim__buf_stats",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__buf_stats, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_stats(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_parse_cmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_parse_cmd",
            c"API: vim -> nvim_parse_cmd",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_empty = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_parse_cmd, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(
                args[1],
                &mut arg_2,
                Some(key_dict_empty_get_field),
                c"Wrong type for argument 2 when calling nvim_parse_cmd, expecting Dict(empty) *",
                error,
            )
        {
            return ret;
        }
        let mut rv = nvim_parse_cmd(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(api_keydict_to_dict(
            (&raw mut rv).cast(),
            (&raw const CMD_TABLE).cast_mut().cast(),
            CMD_TABLE.len(),
            arena,
        ));
        ret
    }
}

pub fn handle_nvim_cmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_cmd", c"API: vim -> nvim_cmd");
        let mut ret = OBJECT_NIL;
        let mut arg_1: KeyDict_cmd = Default::default();
        let mut arg_2: KeyDict_cmd_opts = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_keydict(
            args[0],
            &mut arg_1,
            Some(key_dict_cmd_get_field),
            c"Wrong type for argument 1 when calling nvim_cmd, expecting Dict(cmd) *",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(
                args[1],
                &mut arg_2,
                Some(key_dict_cmd_opts_get_field),
                c"Wrong type for argument 2 when calling nvim_cmd, expecting Dict(cmd_opts) *",
                error,
            )
        {
            return ret;
        }
        let rv = nvim_cmd(channel_id, &mut arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_create_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_create_user_command",
            c"API: vim -> nvim_create_user_command",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_3: KeyDict_user_command = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_create_user_command, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        if args.size >= 3
            && !extract_keydict(args[2], &mut arg_3, Some(key_dict_user_command_get_field), c"Wrong type for argument 3 when calling nvim_create_user_command, expecting Dict(user_command) *", error) {
                return ret;
            }
        nvim_create_user_command(channel_id, arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_del_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_user_command",
            c"API: vim -> nvim_del_user_command",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_user_command, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_del_user_command(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_create_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_create_user_command",
            c"API: vim -> nvim_buf_create_user_command",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_4: KeyDict_user_command = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(args[0], ObjectType::ObjectTypeBuffer, &mut arg_1, c"Wrong type for argument 1 when calling nvim_buf_create_user_command, expecting Buffer", error) {
            return ret;
        }
        if !extract_string(args[1], &mut arg_2, c"Wrong type for argument 2 when calling nvim_buf_create_user_command, expecting String", error) {
            return ret;
        }
        let arg_3: Object = args[2];
        if args.size >= 4
            && !extract_keydict(args[3], &mut arg_4, Some(key_dict_user_command_get_field), c"Wrong type for argument 4 when calling nvim_buf_create_user_command, expecting Dict(user_command) *", error) {
                return ret;
            }
        nvim_buf_create_user_command(channel_id, arg_1, arg_2, arg_3, &mut arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_del_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_del_user_command",
            c"API: vim -> nvim_buf_del_user_command",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_del_user_command, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_del_user_command, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_buf_del_user_command(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_commands(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_commands",
            c"API: vim -> nvim_get_commands",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: KeyDict_get_commands = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(args[0], &mut arg_1, Some(key_dict_get_commands_get_field), c"Wrong type for argument 1 when calling nvim_get_commands, expecting Dict(get_commands) *", error) {
                return ret;
            }
        let rv = nvim_get_commands(&mut arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_commands(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_commands",
            c"API: vim -> nvim_buf_get_commands",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: KeyDict_get_commands = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_commands, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_get_commands_get_field), c"Wrong type for argument 2 when calling nvim_buf_get_commands, expecting Dict(get_commands) *", error) {
                return ret;
            }
        let rv = nvim_buf_get_commands(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_exec(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_exec", c"API: vim -> nvim_exec");
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_exec, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_exec, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_exec(channel_id, arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_command_output(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_command_output",
            c"API: vim -> nvim_command_output",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_command_output, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_command_output(channel_id, arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_execute_lua(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_execute_lua",
            c"API: vim -> nvim_execute_lua",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_execute_lua, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_execute_lua, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_execute_lua(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_buf_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_number",
            c"API: vim -> nvim_buf_get_number",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_number, expecting Buffer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_number(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_clear_highlight(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_clear_highlight",
            c"API: vim -> nvim_buf_clear_highlight",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_clear_highlight, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_clear_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_clear_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_clear_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_buf_clear_highlight(arg_1, arg_2, arg_3, arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_add_highlight(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_add_highlight",
            c"API: vim -> nvim_buf_add_highlight",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: Integer = Default::default();
        let mut arg_6: Integer = Default::default();
        if args.size != 6 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 6 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_add_highlight, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_add_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_add_highlight, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_add_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_buf_add_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[5],
            &mut arg_6,
            c"Wrong type for argument 6 when calling nvim_buf_add_highlight, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_add_highlight(arg_1, arg_2, arg_3, arg_4, arg_5, arg_6, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_virtual_text(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_virtual_text",
            c"API: vim -> nvim_buf_set_virtual_text",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Array = Default::default();
        let mut arg_5: KeyDict_empty = Default::default();
        if args.size < 4 || args.size > 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 to 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_virtual_text, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_virtual_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_virtual_text, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_virtual_text, expecting Array",
            error,
        ) {
            return ret;
        }
        if args.size >= 5
            && !extract_keydict(args[4], &mut arg_5, Some(key_dict_empty_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_virtual_text, expecting Dict(empty) *", error) {
                return ret;
            }
        let rv = nvim_buf_set_virtual_text(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_hl_by_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_hl_by_id",
            c"API: vim -> nvim_get_hl_by_id",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_hl_by_id, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_get_hl_by_id, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_hl_by_id(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_hl_by_name(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_hl_by_name",
            c"API: vim -> nvim_get_hl_by_name",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_hl_by_name, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_get_hl_by_name, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_hl_by_name(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_buffer_insert(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_insert",
            c"API: vim -> buffer_insert",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Array = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_insert, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_insert, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling buffer_insert, expecting ArrayOf(String)",
            error,
        ) {
            return ret;
        }
        buffer_insert(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_buffer_get_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_get_line",
            c"API: vim -> buffer_get_line",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_get_line, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_get_line, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = buffer_get_line(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_buffer_set_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_set_line",
            c"API: vim -> buffer_set_line",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_set_line, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_set_line, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling buffer_set_line, expecting String",
            error,
        ) {
            return ret;
        }
        buffer_set_line(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_buffer_del_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_del_line",
            c"API: vim -> buffer_del_line",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_del_line, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_del_line, expecting Integer",
            error,
        ) {
            return ret;
        }
        buffer_del_line(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_buffer_get_line_slice(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_get_line_slice",
            c"API: vim -> buffer_get_line_slice",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Boolean = Default::default();
        let mut arg_5: Boolean = Default::default();
        if args.size != 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_get_line_slice, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_get_line_slice, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling buffer_get_line_slice, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling buffer_get_line_slice, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling buffer_get_line_slice, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = buffer_get_line_slice(arg_1, arg_2, arg_3, arg_4, arg_5, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_buffer_set_line_slice(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_set_line_slice",
            c"API: vim -> buffer_set_line_slice",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Boolean = Default::default();
        let mut arg_5: Boolean = Default::default();
        let mut arg_6: Array = Default::default();
        if args.size != 6 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 6 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_set_line_slice, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_set_line_slice, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling buffer_set_line_slice, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling buffer_set_line_slice, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling buffer_set_line_slice, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_array(args[5], &mut arg_6, c"Wrong type for argument 6 when calling buffer_set_line_slice, expecting ArrayOf(String)", error) {
            return ret;
        }
        buffer_set_line_slice(arg_1, arg_2, arg_3, arg_4, arg_5, arg_6, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_buffer_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_set_var",
            c"API: vim -> buffer_set_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_set_var, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        let rv = buffer_set_var(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_buffer_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_buffer_del_var",
            c"API: vim -> buffer_del_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling buffer_del_var, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling buffer_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = buffer_del_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_window_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_window_set_var",
            c"API: vim -> window_set_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling window_set_var, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling window_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        let rv = window_set_var(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_window_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_window_del_var",
            c"API: vim -> window_del_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling window_del_var, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling window_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = window_del_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_tabpage_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_tabpage_set_var",
            c"API: vim -> tabpage_set_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling tabpage_set_var, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling tabpage_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        let rv = tabpage_set_var(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_tabpage_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_tabpage_del_var",
            c"API: vim -> tabpage_del_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling tabpage_del_var, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling tabpage_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = tabpage_del_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_vim_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_vim_set_var",
            c"API: vim -> vim_set_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling vim_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        let rv = vim_set_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_vim_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_vim_del_var",
            c"API: vim -> vim_del_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling vim_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = vim_del_var(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_get_option_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_option_info",
            c"API: vim -> nvim_get_option_info",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_option_info, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_option_info(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_option",
            c"API: vim -> nvim_set_option",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_option, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        nvim_set_option(channel_id, arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_option",
            c"API: vim -> nvim_get_option",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_option, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_option(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_buf_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_option",
            c"API: vim -> nvim_buf_get_option",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_option, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_option, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_get_option(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_buf_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_option",
            c"API: vim -> nvim_buf_set_option",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_option, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_option, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        nvim_buf_set_option(channel_id, arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_option",
            c"API: vim -> nvim_win_get_option",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_option, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_get_option, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_option(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_win_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_option",
            c"API: vim -> nvim_win_set_option",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_option, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_option, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        nvim_win_set_option(channel_id, arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_call_atomic(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_call_atomic",
            c"API: vim -> nvim_call_atomic",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Array = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_array(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_call_atomic, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_call_atomic(channel_id, arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_subscribe(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_subscribe",
            c"API: vim -> nvim_subscribe",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_subscribe, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_subscribe(channel_id, arg_1);
        ret
    }
}

pub fn handle_nvim_unsubscribe(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_unsubscribe",
            c"API: vim -> nvim_unsubscribe",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_unsubscribe, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_unsubscribe(channel_id, arg_1);
        ret
    }
}

pub fn handle_nvim_out_write(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_out_write",
            c"API: vim -> nvim_out_write",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_out_write, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_out_write(arg_1);
        ret
    }
}

pub fn handle_nvim_err_write(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_err_write",
            c"API: vim -> nvim_err_write",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_err_write, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_err_write(arg_1);
        ret
    }
}

pub fn handle_nvim_err_writeln(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_err_writeln",
            c"API: vim -> nvim_err_writeln",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_err_writeln, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_err_writeln(arg_1);
        ret
    }
}

pub fn handle_nvim_notify(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_notify",
            c"API: vim -> nvim_notify",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Dict = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_notify, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_notify, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_dict(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_notify, expecting Dict",
            error,
        ) {
            return ret;
        }
        let rv = nvim_notify(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_win_set_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_height",
            c"API: vim -> nvim_win_set_height",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_height, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_height, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_win_set_height(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_set_width(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_width",
            c"API: vim -> nvim_win_set_width",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_width, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_width, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_win_set_width(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_error_event(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_error_event",
            c"API: vim -> nvim_error_event",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_error_event, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_error_event, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_error_event(channel_id, arg_1, arg_2);
        ret
    }
}

pub fn handle_nvim_ui_term_event(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_term_event",
            c"API: vim -> nvim_ui_term_event",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_term_event, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        nvim_ui_term_event(channel_id, arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_create_namespace(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_create_namespace",
            c"API: vim -> nvim_create_namespace",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_create_namespace, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_create_namespace(arg_1);
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_namespaces(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_namespaces",
            c"API: vim -> nvim_get_namespaces",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_namespaces(arena);
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_extmark_by_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_extmark_by_id",
            c"API: vim -> nvim_buf_get_extmark_by_id",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: KeyDict_get_extmark = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_extmark_by_id, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_extmark_by_id, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_get_extmark_by_id, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 4
            && !extract_keydict(args[3], &mut arg_4, Some(key_dict_get_extmark_get_field), c"Wrong type for argument 4 when calling nvim_buf_get_extmark_by_id, expecting Dict(get_extmark) *", error) {
                return ret;
            }
        let rv = nvim_buf_get_extmark_by_id(arg_1, arg_2, arg_3, &mut arg_4, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_get_extmarks(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_get_extmarks",
            c"API: vim -> nvim_buf_get_extmarks",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_5: KeyDict_get_extmarks = Default::default();
        if args.size < 4 || args.size > 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 to 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_get_extmarks, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_get_extmarks, expecting Integer",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        let arg_4: Object = args[3];
        if args.size >= 5
            && !extract_keydict(args[4], &mut arg_5, Some(key_dict_get_extmarks_get_field), c"Wrong type for argument 5 when calling nvim_buf_get_extmarks, expecting Dict(get_extmarks) *", error) {
                return ret;
            }
        let rv = nvim_buf_get_extmarks(arg_1, arg_2, arg_3, arg_4, &mut arg_5, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_set_extmark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_set_extmark",
            c"API: vim -> nvim_buf_set_extmark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: KeyDict_set_extmark = Default::default();
        if args.size < 4 || args.size > 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 to 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_set_extmark, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_set_extmark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_set_extmark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_set_extmark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 5
            && !extract_keydict(args[4], &mut arg_5, Some(key_dict_set_extmark_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_extmark, expecting Dict(set_extmark) *", error) {
                return ret;
            }
        let rv = nvim_buf_set_extmark(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_del_extmark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_del_extmark",
            c"API: vim -> nvim_buf_del_extmark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_del_extmark, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_del_extmark, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_del_extmark, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_del_extmark(arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_buf_clear_namespace(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_buf_clear_namespace",
            c"API: vim -> nvim_buf_clear_namespace",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: Integer = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_buf_clear_namespace, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_buf_clear_namespace, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_buf_clear_namespace, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_buf_clear_namespace, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_buf_clear_namespace(arg_1, arg_2, arg_3, arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_buf_debug_extmarks(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__buf_debug_extmarks",
            c"API: vim -> nvim__buf_debug_extmarks",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: Boolean = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__buf_debug_extmarks, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__buf_debug_extmarks, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim__buf_debug_extmarks, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_buf_debug_extmarks(arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_ns_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__ns_set",
            c"API: vim -> nvim__ns_set",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: KeyDict_ns_opts = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__ns_set, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(
                args[1],
                &mut arg_2,
                Some(key_dict_ns_opts_get_field),
                c"Wrong type for argument 2 when calling nvim__ns_set, expecting Dict(ns_opts) *",
                error,
            )
        {
            return ret;
        }
        nvim_ns_set(arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ns_get(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__ns_get",
            c"API: vim -> nvim__ns_get",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__ns_get, expecting Integer",
            error,
        ) {
            return ret;
        }
        let mut rv = nvim_ns_get(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(api_keydict_to_dict(
            (&raw mut rv).cast(),
            (&raw const NS_OPTS_TABLE).cast_mut().cast(),
            NS_OPTS_TABLE.len(),
            arena,
        ));
        ret
    }
}

pub fn handle_nvim_get_option_value(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_option_value",
            c"API: vim -> nvim_get_option_value",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_option = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_option_value, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_option_get_field), c"Wrong type for argument 2 when calling nvim_get_option_value, expecting Dict(option) *", error) {
                return ret;
            }
        let rv = nvim_get_option_value(arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_set_option_value(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_option_value",
            c"API: vim -> nvim_set_option_value",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_3: KeyDict_option = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_option_value, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        if args.size >= 3
            && !extract_keydict(args[2], &mut arg_3, Some(key_dict_option_get_field), c"Wrong type for argument 3 when calling nvim_set_option_value, expecting Dict(option) *", error) {
                return ret;
            }
        let rv = nvim_set_option_value(channel_id, arg_1, arg_2, &mut arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_get_all_options_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_all_options_info",
            c"API: vim -> nvim_get_all_options_info",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_all_options_info(arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_option_info2(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_option_info2",
            c"API: vim -> nvim_get_option_info2",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_option = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_option_info2, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_option_get_field), c"Wrong type for argument 2 when calling nvim_get_option_info2, expecting Dict(option) *", error) {
                return ret;
            }
        let rv = nvim_get_option_info2(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_tabpage_list_wins(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_list_wins",
            c"API: vim -> nvim_tabpage_list_wins",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_list_wins, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        let rv = nvim_tabpage_list_wins(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_tabpage_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_get_var",
            c"API: vim -> nvim_tabpage_get_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_get_var, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_tabpage_get_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_tabpage_get_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_tabpage_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_set_var",
            c"API: vim -> nvim_tabpage_set_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_set_var, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_tabpage_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        nvim_tabpage_set_var(arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_tabpage_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_del_var",
            c"API: vim -> nvim_tabpage_del_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_del_var, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_tabpage_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_tabpage_del_var(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_tabpage_get_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_get_win",
            c"API: vim -> nvim_tabpage_get_win",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_get_win, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        let rv = nvim_tabpage_get_win(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = window_obj(rv);
        ret
    }
}

pub fn handle_nvim_tabpage_set_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_set_win",
            c"API: vim -> nvim_tabpage_set_win",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        let mut arg_2: Window = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_set_win, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if !extract_handle(
            args[1],
            ObjectType::ObjectTypeWindow,
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_tabpage_set_win, expecting Window",
            error,
        ) {
            return ret;
        }
        nvim_tabpage_set_win(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_tabpage_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_get_number",
            c"API: vim -> nvim_tabpage_get_number",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_get_number, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        let rv = nvim_tabpage_get_number(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_tabpage_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_tabpage_is_valid",
            c"API: vim -> nvim_tabpage_is_valid",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_tabpage_is_valid, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        let rv = nvim_tabpage_is_valid(arg_1);
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_open_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_open_tabpage",
            c"API: vim -> nvim_open_tabpage",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: KeyDict_tabpage_config = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_open_tabpage, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_open_tabpage, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_keydict(args[2], &mut arg_3, Some(key_dict_tabpage_config_get_field), c"Wrong type for argument 3 when calling nvim_open_tabpage, expecting Dict(tabpage_config) *", error) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        let rv = nvim_open_tabpage(arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = tabpage_obj(rv);
        ret
    }
}

pub fn handle_nvim_ui_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_attach",
            c"API: vim -> nvim_ui_attach",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Dict = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_attach, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_ui_attach, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_dict(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_ui_attach, expecting Dict",
            error,
        ) {
            return ret;
        }
        nvim_ui_attach(channel_id, arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_ui_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_ui_attach", c"API: vim -> ui_attach");
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Boolean = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling ui_attach, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling ui_attach, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling ui_attach, expecting Boolean",
            error,
        ) {
            return ret;
        }
        ui_attach(channel_id, arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_set_focus(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_set_focus",
            c"API: vim -> nvim_ui_set_focus",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Boolean = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_boolean(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_set_focus, expecting Boolean",
            error,
        ) {
            return ret;
        }
        nvim_ui_set_focus(channel_id, arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_detach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_detach",
            c"API: vim -> nvim_ui_detach",
        );
        let ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        nvim_ui_detach(channel_id, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_try_resize(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_try_resize",
            c"API: vim -> nvim_ui_try_resize",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_try_resize, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_ui_try_resize, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_ui_try_resize(channel_id, arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_set_option",
            c"API: vim -> nvim_ui_set_option",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_set_option, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        nvim_ui_set_option(channel_id, arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_try_resize_grid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_try_resize_grid",
            c"API: vim -> nvim_ui_try_resize_grid",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_try_resize_grid, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_ui_try_resize_grid, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_ui_try_resize_grid, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_ui_try_resize_grid(channel_id, arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_pum_set_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_pum_set_height",
            c"API: vim -> nvim_ui_pum_set_height",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_pum_set_height, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_ui_pum_set_height(channel_id, arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_pum_set_bounds(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_pum_set_bounds",
            c"API: vim -> nvim_ui_pum_set_bounds",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Float = Default::default();
        let mut arg_2: Float = Default::default();
        let mut arg_3: Float = Default::default();
        let mut arg_4: Float = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_float(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_pum_set_bounds, expecting Float",
            error,
        ) {
            return ret;
        }
        if !extract_float(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_ui_pum_set_bounds, expecting Float",
            error,
        ) {
            return ret;
        }
        if !extract_float(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_ui_pum_set_bounds, expecting Float",
            error,
        ) {
            return ret;
        }
        if !extract_float(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_ui_pum_set_bounds, expecting Float",
            error,
        ) {
            return ret;
        }
        nvim_ui_pum_set_bounds(channel_id, arg_1, arg_2, arg_3, arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_ui_send(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_ui_send",
            c"API: vim -> nvim_ui_send",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_ui_send, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_ui_send(channel_id, arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_hl_id_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_hl_id_by_name",
            c"API: vim -> nvim_get_hl_id_by_name",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_hl_id_by_name, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_hl_id_by_name(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_hl(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_hl",
            c"API: vim -> nvim_get_hl",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: KeyDict_get_highlight = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_hl, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_get_highlight_get_field), c"Wrong type for argument 2 when calling nvim_get_hl, expecting Dict(get_highlight) *", error) {
                return ret;
            }
        let rv = nvim_get_hl(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_hl(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_hl",
            c"API: vim -> nvim_set_hl",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: KeyDict_highlight = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_hl, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_set_hl, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_keydict(
            args[2],
            &mut arg_3,
            Some(key_dict_highlight_get_field),
            c"Wrong type for argument 3 when calling nvim_set_hl, expecting Dict(highlight) *",
            error,
        ) {
            return ret;
        }
        nvim_set_hl(channel_id, arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_hl_ns",
            c"API: vim -> nvim_get_hl_ns",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: KeyDict_get_ns = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(
                args[0],
                &mut arg_1,
                Some(key_dict_get_ns_get_field),
                c"Wrong type for argument 1 when calling nvim_get_hl_ns, expecting Dict(get_ns) *",
                error,
            )
        {
            return ret;
        }
        let rv = nvim_get_hl_ns(&mut arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_hl_ns",
            c"API: vim -> nvim_set_hl_ns",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_hl_ns, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_set_hl_ns(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_set_hl_ns_fast(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_hl_ns_fast",
            c"API: vim -> nvim_set_hl_ns_fast",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_hl_ns_fast, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_set_hl_ns_fast(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_feedkeys(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_feedkeys",
            c"API: vim -> nvim_feedkeys",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: Boolean = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_feedkeys, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_feedkeys, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_feedkeys, expecting Boolean",
            error,
        ) {
            return ret;
        }
        nvim_feedkeys(arg_1, arg_2, arg_3);
        ret
    }
}

pub fn handle_nvim_input(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_input", c"API: vim -> nvim_input");
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_input, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_input(channel_id, arg_1);
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_input_mouse(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_input_mouse",
            c"API: vim -> nvim_input_mouse",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: Integer = Default::default();
        let mut arg_5: Integer = Default::default();
        let mut arg_6: Integer = Default::default();
        if args.size != 6 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 6 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_input_mouse, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_input_mouse, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_input_mouse, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_input_mouse, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_input_mouse, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[5],
            &mut arg_6,
            c"Wrong type for argument 6 when calling nvim_input_mouse, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_input_mouse(arg_1, arg_2, arg_3, arg_4, arg_5, arg_6, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_replace_termcodes(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_replace_termcodes",
            c"API: vim -> nvim_replace_termcodes",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: Boolean = Default::default();
        let mut arg_4: Boolean = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_replace_termcodes, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_replace_termcodes, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_replace_termcodes, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_replace_termcodes, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_replace_termcodes(arg_1, arg_2, arg_3, arg_4);
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_exec_lua(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_exec_lua",
            c"API: vim -> nvim_exec_lua",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_exec_lua, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_exec_lua, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_exec_lua(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_exec_lua_fast(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__exec_lua_fast",
            c"API: vim -> nvim__exec_lua_fast",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__exec_lua_fast, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__exec_lua_fast, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_exec_lua_fast(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_strwidth(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_strwidth",
            c"API: vim -> nvim_strwidth",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_strwidth, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_strwidth(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_list_runtime_paths(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_runtime_paths",
            c"API: vim -> nvim_list_runtime_paths",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_runtime_paths(arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_runtime_inspect(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__runtime_inspect",
            c"API: vim -> nvim__runtime_inspect",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_runtime_inspect(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_runtime_file(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_runtime_file",
            c"API: vim -> nvim_get_runtime_file",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_runtime_file, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_get_runtime_file, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_runtime_file(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_lib_dir(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__get_lib_dir",
            c"API: vim -> nvim__get_lib_dir",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_lib_dir();
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_runtime(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__get_runtime",
            c"API: vim -> nvim__get_runtime",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Array = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: KeyDict_runtime = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_array(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__get_runtime, expecting ArrayOf(String)",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__get_runtime, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if args.size >= 3
            && !extract_keydict(args[2], &mut arg_3, Some(key_dict_runtime_get_field), c"Wrong type for argument 3 when calling nvim__get_runtime, expecting Dict(runtime) *", error) {
                return ret;
            }
        let rv = nvim_get_runtime(arg_1, arg_2, &mut arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_current_dir(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_current_dir",
            c"API: vim -> nvim_set_current_dir",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_current_dir, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_set_current_dir(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_current_line",
            c"API: vim -> nvim_get_current_line",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_current_line(arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = string_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_current_line",
            c"API: vim -> nvim_set_current_line",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_current_line, expecting String",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_set_current_line(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_del_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_current_line",
            c"API: vim -> nvim_del_current_line",
        );
        let ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_del_current_line(arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_var",
            c"API: vim -> nvim_get_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_var(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_var",
            c"API: vim -> nvim_set_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        nvim_set_var(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_var",
            c"API: vim -> nvim_del_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_del_var(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_vvar(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_vvar",
            c"API: vim -> nvim_get_vvar",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_vvar, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_vvar(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_set_vvar(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_vvar",
            c"API: vim -> nvim_set_vvar",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_vvar, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_2: Object = args[1];
        nvim_set_vvar(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_echo(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_echo", c"API: vim -> nvim_echo");
        let mut ret = OBJECT_NIL;
        let mut arg_1: Array = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: KeyDict_echo_opts = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_array(args[0], &mut arg_1, c"Wrong type for argument 1 when calling nvim_echo, expecting ArrayOf(Tuple(String, *HLGroupID))", error) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_echo, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if args.size >= 3
            && !extract_keydict(
                args[2],
                &mut arg_3,
                Some(key_dict_echo_opts_get_field),
                c"Wrong type for argument 3 when calling nvim_echo, expecting Dict(echo_opts) *",
                error,
            )
        {
            return ret;
        }
        let rv = nvim_echo(arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_list_bufs(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_bufs",
            c"API: vim -> nvim_list_bufs",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_bufs(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_current_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_current_buf",
            c"API: vim -> nvim_get_current_buf",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_current_buf();
        ret = buffer_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_current_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_current_buf",
            c"API: vim -> nvim_set_current_buf",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_current_buf, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_set_current_buf(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_list_wins(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_wins",
            c"API: vim -> nvim_list_wins",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_wins(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_current_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_current_win",
            c"API: vim -> nvim_get_current_win",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_current_win();
        ret = window_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_current_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_current_win",
            c"API: vim -> nvim_set_current_win",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_current_win, expecting Window",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_set_current_win(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_create_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_create_buf",
            c"API: vim -> nvim_create_buf",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Boolean = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_boolean(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_create_buf, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_create_buf, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_create_buf(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = buffer_obj(rv);
        ret
    }
}

pub fn handle_nvim_open_term(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_open_term",
            c"API: vim -> nvim_open_term",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: KeyDict_open_term = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_open_term, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_open_term_get_field), c"Wrong type for argument 2 when calling nvim_open_term, expecting Dict(open_term) *", error) {
                return ret;
            }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        let rv = nvim_open_term(arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_chan_send(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_chan_send",
            c"API: vim -> nvim_chan_send",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_chan_send, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_chan_send, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_chan_send(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_list_tabpages(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_tabpages",
            c"API: vim -> nvim_list_tabpages",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_tabpages(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_current_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_current_tabpage",
            c"API: vim -> nvim_get_current_tabpage",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_current_tabpage();
        ret = tabpage_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_current_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_current_tabpage",
            c"API: vim -> nvim_set_current_tabpage",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Tabpage = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeTabpage,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_current_tabpage, expecting Tabpage",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_set_current_tabpage(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_paste(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_paste", c"API: vim -> nvim_paste");
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: Integer = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_paste, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_paste, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_paste, expecting Integer",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        let rv = nvim_paste(channel_id, arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_put(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_put", c"API: vim -> nvim_put");
        let ret = OBJECT_NIL;
        let mut arg_1: Array = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: Boolean = Default::default();
        let mut arg_4: Boolean = Default::default();
        if args.size != 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_array(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_put, expecting ArrayOf(String)",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_put, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_put, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_put, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_put(arg_1, arg_2, arg_3, arg_4, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_color_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_color_by_name",
            c"API: vim -> nvim_get_color_by_name",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_color_by_name, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_color_by_name(arg_1);
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_color_map(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_color_map",
            c"API: vim -> nvim_get_color_map",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_color_map(arena);
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_context(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_context",
            c"API: vim -> nvim_get_context",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: KeyDict_context = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(args[0], &mut arg_1, Some(key_dict_context_get_field), c"Wrong type for argument 1 when calling nvim_get_context, expecting Dict(context) *", error) {
                return ret;
            }
        let rv = nvim_get_context(&mut arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_load_context(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_load_context",
            c"API: vim -> nvim_load_context",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Dict = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_dict(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_load_context, expecting Dict",
            error,
        ) {
            return ret;
        }
        let rv = nvim_load_context(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_get_mode(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_mode",
            c"API: vim -> nvim_get_mode",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_mode(arena);
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_keymap(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_keymap",
            c"API: vim -> nvim_get_keymap",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_keymap(arg_1, arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_keymap",
            c"API: vim -> nvim_set_keymap",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: KeyDict_keymap = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_set_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 4
            && !extract_keydict(
                args[3],
                &mut arg_4,
                Some(key_dict_keymap_get_field),
                c"Wrong type for argument 4 when calling nvim_set_keymap, expecting Dict(keymap) *",
                error,
            )
        {
            return ret;
        }
        nvim_set_keymap(channel_id, arg_1, arg_2, arg_3, &mut arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_del_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_keymap",
            c"API: vim -> nvim_del_keymap",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: KeyDict_keymap_del = Default::default();
        if args.size < 2 || args.size > 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 to 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_del_keymap, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 3
            && !extract_keydict(args[2], &mut arg_3, Some(key_dict_keymap_del_get_field), c"Wrong type for argument 3 when calling nvim_del_keymap, expecting Dict(keymap_del) *", error) {
                return ret;
            }
        nvim_del_keymap(channel_id, arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_api_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_api_info",
            c"API: vim -> nvim_get_api_info",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_get_api_info(channel_id, arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_set_client_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_set_client_info",
            c"API: vim -> nvim_set_client_info",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Dict = Default::default();
        let mut arg_3: NvimString = Default::default();
        let mut arg_4: Dict = Default::default();
        let mut arg_5: Dict = Default::default();
        if args.size != 5 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 5 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_set_client_info, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_dict(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_set_client_info, expecting Dict",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_set_client_info, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_dict(
            args[3],
            &mut arg_4,
            c"Wrong type for argument 4 when calling nvim_set_client_info, expecting Dict",
            error,
        ) {
            return ret;
        }
        if !extract_dict(
            args[4],
            &mut arg_5,
            c"Wrong type for argument 5 when calling nvim_set_client_info, expecting Dict",
            error,
        ) {
            return ret;
        }
        nvim_set_client_info(channel_id, arg_1, arg_2, arg_3, arg_4, arg_5, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_chan_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_chan_set",
            c"API: vim -> nvim_chan_set",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: KeyDict_chan_set = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_chan_set, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_keydict(
            args[1],
            &mut arg_2,
            Some(key_dict_chan_set_get_field),
            c"Wrong type for argument 2 when calling nvim_chan_set, expecting Dict(chan_set) *",
            error,
        ) {
            return ret;
        }
        nvim_chan_set(channel_id, arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_cmdwin_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__cmdwin_set",
            c"API: vim -> nvim__cmdwin_set",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Buffer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__cmdwin_set, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_handle(
            args[1],
            ObjectType::ObjectTypeBuffer,
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__cmdwin_set, expecting Buffer",
            error,
        ) {
            return ret;
        }
        nvim_cmdwin_set(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_get_chan_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_chan_info",
            c"API: vim -> nvim_get_chan_info",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_chan_info, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_chan_info(channel_id, arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_list_chans(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_chans",
            c"API: vim -> nvim_list_chans",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_chans(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim__id", c"API: vim -> nvim__id");
        let mut ret = OBJECT_NIL;
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        let arg_1: Object = args[0];
        let rv = nvim_id(arg_1, arena);
        ret = rv;
        ret
    }
}

pub fn handle_nvim_id_array(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__id_array",
            c"API: vim -> nvim__id_array",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Array = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_array(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__id_array, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_id_array(arg_1, arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_id_dict(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__id_dict",
            c"API: vim -> nvim__id_dict",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Dict = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_dict(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__id_dict, expecting Dict",
            error,
        ) {
            return ret;
        }
        let rv = nvim_id_dict(arg_1, arena);
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_id_float(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__id_float",
            c"API: vim -> nvim__id_float",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Float = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_float(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__id_float, expecting Float",
            error,
        ) {
            return ret;
        }
        let rv = nvim__id_float(arg_1);
        ret = float_obj(rv);
        ret
    }
}

pub fn handle_nvim_stats(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__stats",
            c"API: vim -> nvim__stats",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_stats(arena);
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_list_uis(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_list_uis",
            c"API: vim -> nvim_list_uis",
        );
        let mut ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        let rv = nvim_list_uis(arena);
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_proc_children(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_proc_children",
            c"API: vim -> nvim_get_proc_children",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_proc_children, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_proc_children(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_proc(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_proc",
            c"API: vim -> nvim_get_proc",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_proc, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_get_proc(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_select_popupmenu_item(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_select_popupmenu_item",
            c"API: vim -> nvim_select_popupmenu_item",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: Boolean = Default::default();
        let mut arg_4: KeyDict_empty = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_select_popupmenu_item, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_select_popupmenu_item, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_select_popupmenu_item, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if args.size >= 4
            && !extract_keydict(args[3], &mut arg_4, Some(key_dict_empty_get_field), c"Wrong type for argument 4 when calling nvim_select_popupmenu_item, expecting Dict(empty) *", error) {
                return ret;
            }
        nvim_select_popupmenu_item(arg_1, arg_2, arg_3, &mut arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_inspect_cell(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__inspect_cell",
            c"API: vim -> nvim__inspect_cell",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__inspect_cell, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__inspect_cell, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim__inspect_cell, expecting Integer",
            error,
        ) {
            return ret;
        }
        let rv = nvim_inspect_cell(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_screenshot(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__screenshot",
            c"API: vim -> nvim__screenshot",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__screenshot, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_screenshot(arg_1);
        ret
    }
}

pub fn handle_nvim_invalidate_glyph_cache(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__invalidate_glyph_cache",
            c"API: vim -> nvim__invalidate_glyph_cache",
        );
        let ret = OBJECT_NIL;
        if args.size != 0 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 0 but got %zu",
                args.size
            );
            return ret;
        }
        nvim_invalidate_glyph_cache();
        ret
    }
}

pub fn handle_nvim_unpack(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__unpack",
            c"API: vim -> nvim__unpack",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__unpack, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_unpack(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_del_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_del_mark",
            c"API: vim -> nvim_del_mark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_del_mark, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_del_mark(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_get_mark(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_get_mark",
            c"API: vim -> nvim_get_mark",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_empty = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_get_mark, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(
                args[1],
                &mut arg_2,
                Some(key_dict_empty_get_field),
                c"Wrong type for argument 2 when calling nvim_get_mark, expecting Dict(empty) *",
                error,
            )
        {
            return ret;
        }
        let rv = nvim_get_mark(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_eval_statusline(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_eval_statusline",
            c"API: vim -> nvim_eval_statusline",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_eval_statusline = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_eval_statusline, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_eval_statusline_get_field), c"Wrong type for argument 2 when calling nvim_eval_statusline, expecting Dict(eval_statusline) *", error) {
                return ret;
            }
        let rv = nvim_eval_statusline(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_complete_set(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__complete_set",
            c"API: vim -> nvim__complete_set",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Integer = Default::default();
        let mut arg_2: KeyDict_complete_set = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_integer(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__complete_set, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_complete_set_get_field), c"Wrong type for argument 2 when calling nvim__complete_set, expecting Dict(complete_set) *", error) {
                return ret;
            }
        let rv = nvim_complete_set(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_redraw(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__redraw",
            c"API: vim -> nvim__redraw",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: KeyDict_redraw = Default::default();
        if args.size > 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting at most 1 but got %zu",
                args.size
            );
            return ret;
        }
        if args.size >= 1
            && !extract_keydict(
                args[0],
                &mut arg_1,
                Some(key_dict_redraw_get_field),
                c"Wrong type for argument 1 when calling nvim__redraw, expecting Dict(redraw) *",
                error,
            )
        {
            return ret;
        }
        nvim_redraw(&mut arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_set_restart_on_crash(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim__set_restart_on_crash",
            c"API: vim -> nvim__set_restart_on_crash",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim__set_restart_on_crash, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim__set_restart_on_crash, expecting Array",
            error,
        ) {
            return ret;
        }
        nvim_set_restart_on_crash(arg_1, arg_2);
        ret
    }
}

pub fn handle_nvim_exec2(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_exec2", c"API: vim -> nvim_exec2");
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: KeyDict_exec_opts = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_exec2, expecting String",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(
                args[1],
                &mut arg_2,
                Some(key_dict_exec_opts_get_field),
                c"Wrong type for argument 2 when calling nvim_exec2, expecting Dict(exec_opts) *",
                error,
            )
        {
            return ret;
        }
        let rv = nvim_exec2(channel_id, arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_command",
            c"API: vim -> nvim_command",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_command, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_command(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_eval(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(channel_id, c"handle_nvim_eval", c"API: vim -> nvim_eval");
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_eval, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_eval(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_call_function(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_call_function",
            c"API: vim -> nvim_call_function",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_call_function, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_call_function, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_call_function(channel_id, arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_call_dict_function(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_call_dict_function",
            c"API: vim -> nvim_call_dict_function",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: Array = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        let arg_1: Object = args[0];
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_call_dict_function, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_array(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_call_dict_function, expecting Array",
            error,
        ) {
            return ret;
        }
        let rv = nvim_call_dict_function(channel_id, arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_parse_expression(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_parse_expression",
            c"API: vim -> nvim_parse_expression",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: NvimString = Default::default();
        let mut arg_2: NvimString = Default::default();
        let mut arg_3: Boolean = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_string(
            args[0],
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_parse_expression, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_parse_expression, expecting String",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_parse_expression, expecting Boolean",
            error,
        ) {
            return ret;
        }
        let rv = nvim_parse_expression(arg_1, arg_2, arg_3, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_open_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_open_win",
            c"API: vim -> nvim_open_win",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Buffer = Default::default();
        let mut arg_2: Boolean = Default::default();
        let mut arg_3: KeyDict_win_config = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeBuffer,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_open_win, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_open_win, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if !extract_keydict(
            args[2],
            &mut arg_3,
            Some(key_dict_win_config_get_field),
            c"Wrong type for argument 3 when calling nvim_open_win, expecting Dict(win_config) *",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        let rv = nvim_open_win(arg_1, arg_2, &mut arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = window_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_set_config(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_config",
            c"API: vim -> nvim_win_set_config",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: KeyDict_win_config = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_config, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_keydict(args[1], &mut arg_2, Some(key_dict_win_config_get_field), c"Wrong type for argument 2 when calling nvim_win_set_config, expecting Dict(win_config) *", error) {
            return ret;
        }
        nvim_win_set_config(arg_1, &mut arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_get_config(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_config",
            c"API: vim -> nvim_win_get_config",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_config, expecting Window",
            error,
        ) {
            return ret;
        }
        let mut rv = nvim_win_get_config(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(api_keydict_to_dict(
            (&raw mut rv).cast(),
            (&raw const WIN_CONFIG_TABLE).cast_mut().cast(),
            WIN_CONFIG_TABLE.len(),
            arena,
        ));
        ret
    }
}

pub fn handle_nvim_win_get_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_buf",
            c"API: vim -> nvim_win_get_buf",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_buf, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_buf(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = buffer_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_set_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_buf",
            c"API: vim -> nvim_win_set_buf",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Buffer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_buf, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_handle(
            args[1],
            ObjectType::ObjectTypeBuffer,
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_buf, expecting Buffer",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_win_set_buf(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_get_cursor(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_cursor",
            c"API: vim -> nvim_win_get_cursor",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_cursor, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_cursor(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_set_cursor(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_cursor",
            c"API: vim -> nvim_win_set_cursor",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Array = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_cursor, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_array(args[1], &mut arg_2, c"Wrong type for argument 2 when calling nvim_win_set_cursor, expecting ArrayOf(Integer, 2)", error) {
            return ret;
        }
        nvim_win_set_cursor(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_get_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_height",
            c"API: vim -> nvim_win_get_height",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_height, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_height(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_get_width(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_width",
            c"API: vim -> nvim_win_get_width",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_width, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_width(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_var",
            c"API: vim -> nvim_win_get_var",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_var, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_get_var, expecting String",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_var(arg_1, arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = rv;
        ret
    }
}

pub fn handle_nvim_win_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_var",
            c"API: vim -> nvim_win_set_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 3 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_var, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_var, expecting String",
            error,
        ) {
            return ret;
        }
        let arg_3: Object = args[2];
        nvim_win_set_var(arg_1, arg_2, arg_3, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_del_var",
            c"API: vim -> nvim_win_del_var",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: NvimString = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_del_var, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_string(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_del_var, expecting String",
            error,
        ) {
            return ret;
        }
        nvim_win_del_var(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_get_position(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_position",
            c"API: vim -> nvim_win_get_position",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_position, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_position(arg_1, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = array_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_get_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_tabpage",
            c"API: vim -> nvim_win_get_tabpage",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_tabpage, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_tabpage(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = tabpage_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_get_number",
            c"API: vim -> nvim_win_get_number",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_get_number, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_get_number(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = integer_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_is_valid",
            c"API: vim -> nvim_win_is_valid",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_is_valid, expecting Window",
            error,
        ) {
            return ret;
        }
        let rv = nvim_win_is_valid(arg_1);
        ret = boolean_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_hide(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_hide",
            c"API: vim -> nvim_win_hide",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        if args.size != 1 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_hide, expecting Window",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_win_hide(arg_1, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_close(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_close",
            c"API: vim -> nvim_win_close",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Boolean = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_close, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_boolean(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_close, expecting Boolean",
            error,
        ) {
            return ret;
        }
        if text_locked() {
            api_set_error_str(error, ErrorType::ErrorTypeException, get_text_locked_msg());
            return ret;
        }
        nvim_win_close(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_set_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_set_hl_ns",
            c"API: vim -> nvim_win_set_hl_ns",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Integer = Default::default();
        if args.size != 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_set_hl_ns, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_set_hl_ns, expecting Integer",
            error,
        ) {
            return ret;
        }
        nvim_win_set_hl_ns(arg_1, arg_2, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

pub fn handle_nvim_win_text_height(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_text_height",
            c"API: vim -> nvim_win_text_height",
        );
        let mut ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: KeyDict_win_text_height = Default::default();
        if args.size < 1 || args.size > 2 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 1 to 2 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_text_height, expecting Window",
            error,
        ) {
            return ret;
        }
        if args.size >= 2
            && !extract_keydict(args[1], &mut arg_2, Some(key_dict_win_text_height_get_field), c"Wrong type for argument 2 when calling nvim_win_text_height, expecting Dict(win_text_height) *", error) {
                return ret;
            }
        let rv = nvim_win_text_height(arg_1, &mut arg_2, arena, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret = dict_obj(rv);
        ret
    }
}

pub fn handle_nvim_win_resize(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    unsafe {
        dlog_internal_call(
            channel_id,
            c"handle_nvim_win_resize",
            c"API: vim -> nvim_win_resize",
        );
        let ret = OBJECT_NIL;
        let mut arg_1: Window = Default::default();
        let mut arg_2: Integer = Default::default();
        let mut arg_3: Integer = Default::default();
        let mut arg_4: KeyDict_win_resize = Default::default();
        if args.size < 3 || args.size > 4 {
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Wrong number of arguments: expecting 3 to 4 but got %zu",
                args.size
            );
            return ret;
        }
        if !extract_handle(
            args[0],
            ObjectType::ObjectTypeWindow,
            &mut arg_1,
            c"Wrong type for argument 1 when calling nvim_win_resize, expecting Window",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[1],
            &mut arg_2,
            c"Wrong type for argument 2 when calling nvim_win_resize, expecting Integer",
            error,
        ) {
            return ret;
        }
        if !extract_integer(
            args[2],
            &mut arg_3,
            c"Wrong type for argument 3 when calling nvim_win_resize, expecting Integer",
            error,
        ) {
            return ret;
        }
        if args.size >= 4
            && !extract_keydict(args[3], &mut arg_4, Some(key_dict_win_resize_get_field), c"Wrong type for argument 4 when calling nvim_win_resize, expecting Dict(win_resize) *", error) {
                return ret;
            }
        nvim_win_resize(arg_1, arg_2, arg_3, &mut arg_4, error);
        if error.err_type != ErrorType::ErrorTypeNone {
            return ret;
        }
        ret
    }
}

macro_rules! rpc_fn {
    ($f:ident) => {{ { Some($f) } }};
}

pub static METHOD_HANDLERS: [MsgpackRpcRequestHandler; 283] = [
    MsgpackRpcRequestHandler {
        name: c"redraw",
        r#fn: rpc_fn!(handle_ui_client_redraw),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__id",
        r#fn: rpc_fn!(handle_nvim_id),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_cmd",
        r#fn: rpc_fn!(handle_nvim_cmd),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_put",
        r#fn: rpc_fn!(handle_nvim_put),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_eval",
        r#fn: rpc_fn!(handle_nvim_eval),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"ui_attach",
        r#fn: rpc_fn!(handle_ui_attach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"ui_detach",
        r#fn: rpc_fn!(handle_nvim_ui_detach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_echo",
        r#fn: rpc_fn!(handle_nvim_echo),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_input",
        r#fn: rpc_fn!(handle_nvim_input),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_eval",
        r#fn: rpc_fn!(handle_nvim_eval),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_exec",
        r#fn: rpc_fn!(handle_nvim_exec),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_exec2",
        r#fn: rpc_fn!(handle_nvim_exec2),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_input",
        r#fn: rpc_fn!(handle_nvim_input),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_paste",
        r#fn: rpc_fn!(handle_nvim_paste),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_hl",
        r#fn: rpc_fn!(handle_nvim_get_hl),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_hl",
        r#fn: rpc_fn!(handle_nvim_set_hl),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_del_var",
        r#fn: rpc_fn!(handle_vim_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_command",
        r#fn: rpc_fn!(handle_nvim_command),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_notify",
        r#fn: rpc_fn!(handle_nvim_notify),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__stats",
        r#fn: rpc_fn!(handle_nvim_stats),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_var",
        r#fn: rpc_fn!(handle_nvim_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_var",
        r#fn: rpc_fn!(handle_vim_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_send",
        r#fn: rpc_fn!(handle_nvim_ui_send),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_vvar",
        r#fn: rpc_fn!(handle_nvim_get_vvar),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_feedkeys",
        r#fn: rpc_fn!(handle_nvim_feedkeys),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__redraw",
        r#fn: rpc_fn!(handle_nvim_redraw),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_var",
        r#fn: rpc_fn!(handle_nvim_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_command",
        r#fn: rpc_fn!(handle_nvim_command),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__unpack",
        r#fn: rpc_fn!(handle_nvim_unpack),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__ns_get",
        r#fn: rpc_fn!(handle_nvim_ns_get),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__ns_set",
        r#fn: rpc_fn!(handle_nvim_ns_set),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_var",
        r#fn: rpc_fn!(handle_nvim_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_var",
        r#fn: rpc_fn!(handle_nvim_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_strwidth",
        r#fn: rpc_fn!(handle_nvim_strwidth),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_vvar",
        r#fn: rpc_fn!(handle_nvim_get_vvar),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_vvar",
        r#fn: rpc_fn!(handle_nvim_set_vvar),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_subscribe",
        r#fn: rpc_fn!(handle_nvim_subscribe),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__id_dict",
        r#fn: rpc_fn!(handle_nvim_id_dict),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_mode",
        r#fn: rpc_fn!(handle_nvim_get_mode),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_hide",
        r#fn: rpc_fn!(handle_nvim_win_hide),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_chan_set",
        r#fn: rpc_fn!(handle_nvim_chan_set),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_uis",
        r#fn: rpc_fn!(handle_nvim_list_uis),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_open_win",
        r#fn: rpc_fn!(handle_nvim_open_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_proc",
        r#fn: rpc_fn!(handle_nvim_get_proc),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_insert",
        r#fn: rpc_fn!(handle_buffer_insert),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_mark",
        r#fn: rpc_fn!(handle_nvim_del_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_mark",
        r#fn: rpc_fn!(handle_nvim_get_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_strwidth",
        r#fn: rpc_fn!(handle_nvim_strwidth),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_err_write",
        r#fn: rpc_fn!(handle_nvim_err_write),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_out_write",
        r#fn: rpc_fn!(handle_nvim_out_write),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_exec_lua",
        r#fn: rpc_fn!(handle_nvim_exec_lua),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_feedkeys",
        r#fn: rpc_fn!(handle_nvim_feedkeys),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"ui_try_resize",
        r#fn: rpc_fn!(handle_nvim_ui_try_resize),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__id_array",
        r#fn: rpc_fn!(handle_nvim_id_array),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__id_float",
        r#fn: rpc_fn!(handle_nvim_id_float),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_chan_send",
        r#fn: rpc_fn!(handle_nvim_chan_send),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_err_write",
        r#fn: rpc_fn!(handle_nvim_err_write),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_option",
        r#fn: rpc_fn!(handle_nvim_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_option",
        r#fn: rpc_fn!(handle_nvim_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_hl_ns",
        r#fn: rpc_fn!(handle_nvim_get_hl_ns),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_bufs",
        r#fn: rpc_fn!(handle_nvim_list_bufs),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_wins",
        r#fn: rpc_fn!(handle_nvim_list_wins),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_open_term",
        r#fn: rpc_fn!(handle_nvim_open_term),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_out_write",
        r#fn: rpc_fn!(handle_nvim_out_write),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_parse_cmd",
        r#fn: rpc_fn!(handle_nvim_parse_cmd),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_del_var",
        r#fn: rpc_fn!(handle_buffer_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_var",
        r#fn: rpc_fn!(handle_nvim_buf_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_var",
        r#fn: rpc_fn!(handle_buffer_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_hl_ns",
        r#fn: rpc_fn!(handle_nvim_set_hl_ns),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_subscribe",
        r#fn: rpc_fn!(handle_nvim_subscribe),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_attach",
        r#fn: rpc_fn!(handle_nvim_ui_attach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_detach",
        r#fn: rpc_fn!(handle_nvim_ui_detach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_close",
        r#fn: rpc_fn!(handle_nvim_win_close),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_del_var",
        r#fn: rpc_fn!(handle_window_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_var",
        r#fn: rpc_fn!(handle_nvim_win_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_set_var",
        r#fn: rpc_fn!(handle_window_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_del_var",
        r#fn: rpc_fn!(handle_tabpage_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_get_var",
        r#fn: rpc_fn!(handle_nvim_tabpage_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_set_var",
        r#fn: rpc_fn!(handle_tabpage_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_buffers",
        r#fn: rpc_fn!(handle_nvim_list_bufs),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_windows",
        r#fn: rpc_fn!(handle_nvim_list_wins),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_del_line",
        r#fn: rpc_fn!(handle_buffer_del_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_create_buf",
        r#fn: rpc_fn!(handle_nvim_create_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_attach",
        r#fn: rpc_fn!(handle_nvim_buf_attach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_delete",
        r#fn: rpc_fn!(handle_nvim_buf_delete),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_detach",
        r#fn: rpc_fn!(handle_nvim_buf_detach),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_line",
        r#fn: rpc_fn!(handle_buffer_get_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_mark",
        r#fn: rpc_fn!(handle_nvim_buf_get_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_name",
        r#fn: rpc_fn!(handle_nvim_buf_get_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_is_valid",
        r#fn: rpc_fn!(handle_nvim_buf_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_is_valid",
        r#fn: rpc_fn!(handle_nvim_win_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_keymap",
        r#fn: rpc_fn!(handle_nvim_del_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_resize",
        r#fn: rpc_fn!(handle_nvim_win_resize),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_line",
        r#fn: rpc_fn!(handle_buffer_set_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_name",
        r#fn: rpc_fn!(handle_nvim_buf_set_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_chans",
        r#fn: rpc_fn!(handle_nvim_list_chans),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_keymap",
        r#fn: rpc_fn!(handle_nvim_get_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_option",
        r#fn: rpc_fn!(handle_nvim_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_keymap",
        r#fn: rpc_fn!(handle_nvim_set_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_option",
        r#fn: rpc_fn!(handle_nvim_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__buf_stats",
        r#fn: rpc_fn!(handle_nvim_buf_stats),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_unsubscribe",
        r#fn: rpc_fn!(handle_nvim_unsubscribe),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_lines",
        r#fn: rpc_fn!(handle_nvim_buf_get_lines),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_lines",
        r#fn: rpc_fn!(handle_nvim_buf_set_lines),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_width",
        r#fn: rpc_fn!(handle_nvim_win_get_width),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_set_width",
        r#fn: rpc_fn!(handle_nvim_win_set_width),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_call_atomic",
        r#fn: rpc_fn!(handle_nvim_call_atomic),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__cmdwin_set",
        r#fn: rpc_fn!(handle_nvim_cmdwin_set),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_autocmd",
        r#fn: rpc_fn!(handle_nvim_del_autocmd),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_context",
        r#fn: rpc_fn!(handle_nvim_get_context),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_is_valid",
        r#fn: rpc_fn!(handle_nvim_tabpage_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_del_var",
        r#fn: rpc_fn!(handle_nvim_win_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_buf",
        r#fn: rpc_fn!(handle_nvim_win_get_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_var",
        r#fn: rpc_fn!(handle_nvim_win_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_buf",
        r#fn: rpc_fn!(handle_nvim_win_set_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_var",
        r#fn: rpc_fn!(handle_nvim_win_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_input_mouse",
        r#fn: rpc_fn!(handle_nvim_input_mouse),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_unsubscribe",
        r#fn: rpc_fn!(handle_nvim_unsubscribe),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_report_error",
        r#fn: rpc_fn!(handle_nvim_err_writeln),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_err_writeln",
        r#fn: rpc_fn!(handle_nvim_err_writeln),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_error_event",
        r#fn: rpc_fn!(handle_nvim_error_event),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__screenshot",
        r#fn: rpc_fn!(handle_nvim_screenshot),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_api_info",
        r#fn: rpc_fn!(handle_nvim_get_api_info),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_tabpages",
        r#fn: rpc_fn!(handle_nvim_list_tabpages),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_del_var",
        r#fn: rpc_fn!(handle_nvim_buf_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_var",
        r#fn: rpc_fn!(handle_nvim_buf_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_var",
        r#fn: rpc_fn!(handle_nvim_buf_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_execute_lua",
        r#fn: rpc_fn!(handle_nvim_execute_lua),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_hl_by_id",
        r#fn: rpc_fn!(handle_nvim_get_hl_by_id),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_del_mark",
        r#fn: rpc_fn!(handle_nvim_buf_del_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_mark",
        r#fn: rpc_fn!(handle_nvim_buf_get_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_name",
        r#fn: rpc_fn!(handle_nvim_buf_get_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_mark",
        r#fn: rpc_fn!(handle_nvim_buf_set_mark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_name",
        r#fn: rpc_fn!(handle_nvim_buf_set_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_open_tabpage",
        r#fn: rpc_fn!(handle_nvim_open_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_number",
        r#fn: rpc_fn!(handle_nvim_buf_get_number),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_set_focus",
        r#fn: rpc_fn!(handle_nvim_ui_set_focus),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__get_lib_dir",
        r#fn: rpc_fn!(handle_nvim_get_lib_dir),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_text",
        r#fn: rpc_fn!(handle_nvim_buf_get_text),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_text",
        r#fn: rpc_fn!(handle_nvim_buf_set_text),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_load_context",
        r#fn: rpc_fn!(handle_nvim_load_context),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_buffer",
        r#fn: rpc_fn!(handle_nvim_win_get_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_height",
        r#fn: rpc_fn!(handle_nvim_win_get_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_set_height",
        r#fn: rpc_fn!(handle_nvim_win_set_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_option",
        r#fn: rpc_fn!(handle_nvim_buf_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_option",
        r#fn: rpc_fn!(handle_nvim_buf_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__get_runtime",
        r#fn: rpc_fn!(handle_nvim_get_runtime),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_call_function",
        r#fn: rpc_fn!(handle_nvim_call_function),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_option",
        r#fn: rpc_fn!(handle_nvim_win_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"window_set_option",
        r#fn: rpc_fn!(handle_nvim_win_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_is_valid",
        r#fn: rpc_fn!(handle_nvim_buf_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_is_valid",
        r#fn: rpc_fn!(handle_nvim_win_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_name_to_color",
        r#fn: rpc_fn!(handle_nvim_get_color_by_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_autocmds",
        r#fn: rpc_fn!(handle_nvim_get_autocmds),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_color_map",
        r#fn: rpc_fn!(handle_nvim_get_color_map),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_api_info",
        r#fn: rpc_fn!(handle_nvim_get_api_info),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_commands",
        r#fn: rpc_fn!(handle_nvim_get_commands),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_cursor",
        r#fn: rpc_fn!(handle_nvim_win_get_cursor),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_set_cursor",
        r#fn: rpc_fn!(handle_nvim_win_set_cursor),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_line_count",
        r#fn: rpc_fn!(handle_nvim_buf_line_count),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_call_function",
        r#fn: rpc_fn!(handle_nvim_call_function),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_exec_autocmds",
        r#fn: rpc_fn!(handle_nvim_exec_autocmds),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_tabpages",
        r#fn: rpc_fn!(handle_nvim_list_tabpages),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_chan_info",
        r#fn: rpc_fn!(handle_nvim_get_chan_info),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_color_map",
        r#fn: rpc_fn!(handle_nvim_get_color_map),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_set_option",
        r#fn: rpc_fn!(handle_nvim_ui_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_term_event",
        r#fn: rpc_fn!(handle_nvim_ui_term_event),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_get_window",
        r#fn: rpc_fn!(handle_nvim_tabpage_get_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_lines",
        r#fn: rpc_fn!(handle_nvim_buf_get_lines),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_width",
        r#fn: rpc_fn!(handle_nvim_win_get_width),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_is_loaded",
        r#fn: rpc_fn!(handle_nvim_buf_is_loaded),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_command_output",
        r#fn: rpc_fn!(handle_nvim_command_output),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__complete_set",
        r#fn: rpc_fn!(handle_nvim_complete_set),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__inspect_cell",
        r#fn: rpc_fn!(handle_nvim_inspect_cell),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_try_resize",
        r#fn: rpc_fn!(handle_nvim_ui_try_resize),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_lines",
        r#fn: rpc_fn!(handle_nvim_buf_set_lines),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_hl_ns",
        r#fn: rpc_fn!(handle_nvim_win_set_hl_ns),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_width",
        r#fn: rpc_fn!(handle_nvim_win_set_width),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_tabpage",
        r#fn: rpc_fn!(handle_nvim_win_get_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__exec_lua_fast",
        r#fn: rpc_fn!(handle_nvim_exec_lua_fast),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_hl_by_name",
        r#fn: rpc_fn!(handle_nvim_get_hl_by_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_hl_ns_fast",
        r#fn: rpc_fn!(handle_nvim_set_hl_ns_fast),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_line_count",
        r#fn: rpc_fn!(handle_nvim_buf_line_count),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_del_keymap",
        r#fn: rpc_fn!(handle_nvim_buf_del_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_keymap",
        r#fn: rpc_fn!(handle_nvim_buf_get_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_keymap",
        r#fn: rpc_fn!(handle_nvim_buf_set_keymap),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_height",
        r#fn: rpc_fn!(handle_nvim_win_get_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_height",
        r#fn: rpc_fn!(handle_nvim_win_set_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_offset",
        r#fn: rpc_fn!(handle_nvim_buf_get_offset),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_create_augroup",
        r#fn: rpc_fn!(handle_nvim_create_augroup),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"window_get_position",
        r#fn: rpc_fn!(handle_nvim_win_get_position),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"tabpage_get_windows",
        r#fn: rpc_fn!(handle_nvim_tabpage_list_wins),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_clear_autocmds",
        r#fn: rpc_fn!(handle_nvim_clear_autocmds),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_config",
        r#fn: rpc_fn!(handle_nvim_win_get_config),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_config",
        r#fn: rpc_fn!(handle_nvim_win_set_config),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_option",
        r#fn: rpc_fn!(handle_nvim_buf_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_option",
        r#fn: rpc_fn!(handle_nvim_buf_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_namespaces",
        r#fn: rpc_fn!(handle_nvim_get_namespaces),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_option",
        r#fn: rpc_fn!(handle_nvim_win_get_option),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_option",
        r#fn: rpc_fn!(handle_nvim_win_set_option),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_create_autocmd",
        r#fn: rpc_fn!(handle_nvim_create_autocmd),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_number",
        r#fn: rpc_fn!(handle_nvim_buf_get_number),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_command_output",
        r#fn: rpc_fn!(handle_nvim_command_output),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_cursor",
        r#fn: rpc_fn!(handle_nvim_win_get_cursor),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_number",
        r#fn: rpc_fn!(handle_nvim_win_get_number),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_set_cursor",
        r#fn: rpc_fn!(handle_nvim_win_set_cursor),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_del_extmark",
        r#fn: rpc_fn!(handle_nvim_buf_del_extmark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_extmark",
        r#fn: rpc_fn!(handle_nvim_buf_set_extmark),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_tabpage",
        r#fn: rpc_fn!(handle_nvim_win_get_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_current_buf",
        r#fn: rpc_fn!(handle_nvim_get_current_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_current_buf",
        r#fn: rpc_fn!(handle_nvim_set_current_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_current_dir",
        r#fn: rpc_fn!(handle_nvim_set_current_dir),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_add_highlight",
        r#fn: rpc_fn!(handle_nvim_buf_add_highlight),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_text_height",
        r#fn: rpc_fn!(handle_nvim_win_text_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_eval_statusline",
        r#fn: rpc_fn!(handle_nvim_eval_statusline),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_del_current_line",
        r#fn: rpc_fn!(handle_nvim_del_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_current_line",
        r#fn: rpc_fn!(handle_nvim_get_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_current_line",
        r#fn: rpc_fn!(handle_nvim_set_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_option_info",
        r#fn: rpc_fn!(handle_nvim_get_option_info),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_client_info",
        r#fn: rpc_fn!(handle_nvim_set_client_info),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_change_directory",
        r#fn: rpc_fn!(handle_nvim_set_current_dir),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_del_var",
        r#fn: rpc_fn!(handle_nvim_tabpage_del_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_get_var",
        r#fn: rpc_fn!(handle_nvim_tabpage_get_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_set_var",
        r#fn: rpc_fn!(handle_nvim_tabpage_set_var),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_current_win",
        r#fn: rpc_fn!(handle_nvim_get_current_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_current_win",
        r#fn: rpc_fn!(handle_nvim_set_current_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_get_win",
        r#fn: rpc_fn!(handle_nvim_tabpage_get_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_set_win",
        r#fn: rpc_fn!(handle_nvim_tabpage_set_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__runtime_inspect",
        r#fn: rpc_fn!(handle_nvim_runtime_inspect),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_commands",
        r#fn: rpc_fn!(handle_nvim_buf_get_commands),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_extmarks",
        r#fn: rpc_fn!(handle_nvim_buf_get_extmarks),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_create_namespace",
        r#fn: rpc_fn!(handle_nvim_create_namespace),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_current_line",
        r#fn: rpc_fn!(handle_nvim_del_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_user_command",
        r#fn: rpc_fn!(handle_nvim_del_user_command),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_replace_termcodes",
        r#fn: rpc_fn!(handle_nvim_replace_termcodes),
        fast: true,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_current_line",
        r#fn: rpc_fn!(handle_nvim_get_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_option_info2",
        r#fn: rpc_fn!(handle_nvim_get_option_info2),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_option_value",
        r#fn: rpc_fn!(handle_nvim_get_option_value),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_runtime_file",
        r#fn: rpc_fn!(handle_nvim_get_runtime_file),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_parse_expression",
        r#fn: rpc_fn!(handle_nvim_parse_expression),
        fast: true,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_get_line_slice",
        r#fn: rpc_fn!(handle_buffer_get_line_slice),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_set_line_slice",
        r#fn: rpc_fn!(handle_buffer_set_line_slice),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_current_line",
        r#fn: rpc_fn!(handle_nvim_set_current_line),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_option_value",
        r#fn: rpc_fn!(handle_nvim_set_option_value),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_is_valid",
        r#fn: rpc_fn!(handle_nvim_tabpage_is_valid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_win_get_position",
        r#fn: rpc_fn!(handle_nvim_win_get_position),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_augroup_by_id",
        r#fn: rpc_fn!(handle_nvim_del_augroup_by_id),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_list_runtime_paths",
        r#fn: rpc_fn!(handle_nvim_list_runtime_paths),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_pum_set_bounds",
        r#fn: rpc_fn!(handle_nvim_ui_pum_set_bounds),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_current_buffer",
        r#fn: rpc_fn!(handle_nvim_get_current_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_current_buffer",
        r#fn: rpc_fn!(handle_nvim_set_current_buf),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"buffer_clear_highlight",
        r#fn: rpc_fn!(handle_nvim_buf_clear_highlight),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_add_highlight",
        r#fn: rpc_fn!(handle_nvim_buf_add_highlight),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_pum_set_height",
        r#fn: rpc_fn!(handle_nvim_ui_pum_set_height),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_proc_children",
        r#fn: rpc_fn!(handle_nvim_get_proc_children),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_replace_termcodes",
        r#fn: rpc_fn!(handle_nvim_replace_termcodes),
        fast: true,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_list_wins",
        r#fn: rpc_fn!(handle_nvim_tabpage_list_wins),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_current_window",
        r#fn: rpc_fn!(handle_nvim_get_current_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_current_window",
        r#fn: rpc_fn!(handle_nvim_set_current_win),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_color_by_name",
        r#fn: rpc_fn!(handle_nvim_get_color_by_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_hl_id_by_name",
        r#fn: rpc_fn!(handle_nvim_get_hl_id_by_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_call_dict_function",
        r#fn: rpc_fn!(handle_nvim_call_dict_function),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_get_current_tabpage",
        r#fn: rpc_fn!(handle_nvim_get_current_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"vim_set_current_tabpage",
        r#fn: rpc_fn!(handle_nvim_set_current_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_list_runtime_paths",
        r#fn: rpc_fn!(handle_nvim_list_runtime_paths),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_tabpage_get_number",
        r#fn: rpc_fn!(handle_nvim_tabpage_get_number),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_ui_try_resize_grid",
        r#fn: rpc_fn!(handle_nvim_ui_try_resize_grid),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_changedtick",
        r#fn: rpc_fn!(handle_nvim_buf_get_changedtick),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_current_tabpage",
        r#fn: rpc_fn!(handle_nvim_get_current_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_set_current_tabpage",
        r#fn: rpc_fn!(handle_nvim_set_current_tabpage),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_del_augroup_by_name",
        r#fn: rpc_fn!(handle_nvim_del_augroup_by_name),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_clear_highlight",
        r#fn: rpc_fn!(handle_nvim_buf_clear_highlight),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_clear_namespace",
        r#fn: rpc_fn!(handle_nvim_buf_clear_namespace),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_create_user_command",
        r#fn: rpc_fn!(handle_nvim_create_user_command),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__buf_debug_extmarks",
        r#fn: rpc_fn!(handle_nvim_buf_debug_extmarks),
        fast: false,
        ret_alloc: true,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_get_all_options_info",
        r#fn: rpc_fn!(handle_nvim_get_all_options_info),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_del_user_command",
        r#fn: rpc_fn!(handle_nvim_buf_del_user_command),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_set_virtual_text",
        r#fn: rpc_fn!(handle_nvim_buf_set_virtual_text),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__set_restart_on_crash",
        r#fn: rpc_fn!(handle_nvim_set_restart_on_crash),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_get_extmark_by_id",
        r#fn: rpc_fn!(handle_nvim_buf_get_extmark_by_id),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_select_popupmenu_item",
        r#fn: rpc_fn!(handle_nvim_select_popupmenu_item),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim__invalidate_glyph_cache",
        r#fn: rpc_fn!(handle_nvim_invalidate_glyph_cache),
        fast: false,
        ret_alloc: false,
    },
    MsgpackRpcRequestHandler {
        name: c"nvim_buf_create_user_command",
        r#fn: rpc_fn!(handle_nvim_buf_create_user_command),
        fast: false,
        ret_alloc: false,
    },
];

pub fn msgpack_rpc_get_handler_for_hash(key: &[u8]) -> i32 {
    match key {
        b"redraw" => 0,
        b"nvim__id" => 1,
        b"nvim_cmd" => 2,
        b"nvim_put" => 3,
        b"vim_eval" => 4,
        b"ui_attach" => 5,
        b"ui_detach" => 6,
        b"nvim_echo" => 7,
        b"vim_input" => 8,
        b"nvim_eval" => 9,
        b"nvim_exec" => 10,
        b"nvim_exec2" => 11,
        b"nvim_input" => 12,
        b"nvim_paste" => 13,
        b"nvim_get_hl" => 14,
        b"nvim_set_hl" => 15,
        b"vim_del_var" => 16,
        b"vim_command" => 17,
        b"nvim_notify" => 18,
        b"nvim__stats" => 19,
        b"vim_get_var" => 20,
        b"vim_set_var" => 21,
        b"nvim_ui_send" => 22,
        b"vim_get_vvar" => 23,
        b"vim_feedkeys" => 24,
        b"nvim__redraw" => 25,
        b"nvim_del_var" => 26,
        b"nvim_command" => 27,
        b"nvim__unpack" => 28,
        b"nvim__ns_get" => 29,
        b"nvim__ns_set" => 30,
        b"nvim_get_var" => 31,
        b"nvim_set_var" => 32,
        b"vim_strwidth" => 33,
        b"nvim_get_vvar" => 34,
        b"nvim_set_vvar" => 35,
        b"vim_subscribe" => 36,
        b"nvim__id_dict" => 37,
        b"nvim_get_mode" => 38,
        b"nvim_win_hide" => 39,
        b"nvim_chan_set" => 40,
        b"nvim_list_uis" => 41,
        b"nvim_open_win" => 42,
        b"nvim_get_proc" => 43,
        b"buffer_insert" => 44,
        b"nvim_del_mark" => 45,
        b"nvim_get_mark" => 46,
        b"nvim_strwidth" => 47,
        b"vim_err_write" => 48,
        b"vim_out_write" => 49,
        b"nvim_exec_lua" => 50,
        b"nvim_feedkeys" => 51,
        b"ui_try_resize" => 52,
        b"nvim__id_array" => 53,
        b"nvim__id_float" => 54,
        b"nvim_chan_send" => 55,
        b"nvim_err_write" => 56,
        b"vim_get_option" => 57,
        b"vim_set_option" => 58,
        b"nvim_get_hl_ns" => 59,
        b"nvim_list_bufs" => 60,
        b"nvim_list_wins" => 61,
        b"nvim_open_term" => 62,
        b"nvim_out_write" => 63,
        b"nvim_parse_cmd" => 64,
        b"buffer_del_var" => 65,
        b"buffer_get_var" => 66,
        b"buffer_set_var" => 67,
        b"nvim_set_hl_ns" => 68,
        b"nvim_subscribe" => 69,
        b"nvim_ui_attach" => 70,
        b"nvim_ui_detach" => 71,
        b"nvim_win_close" => 72,
        b"window_del_var" => 73,
        b"window_get_var" => 74,
        b"window_set_var" => 75,
        b"tabpage_del_var" => 76,
        b"tabpage_get_var" => 77,
        b"tabpage_set_var" => 78,
        b"vim_get_buffers" => 79,
        b"vim_get_windows" => 80,
        b"buffer_del_line" => 81,
        b"nvim_create_buf" => 82,
        b"nvim_buf_attach" => 83,
        b"nvim_buf_delete" => 84,
        b"nvim_buf_detach" => 85,
        b"buffer_get_line" => 86,
        b"buffer_get_mark" => 87,
        b"buffer_get_name" => 88,
        b"buffer_is_valid" => 89,
        b"window_is_valid" => 90,
        b"nvim_del_keymap" => 91,
        b"nvim_win_resize" => 92,
        b"buffer_set_line" => 93,
        b"buffer_set_name" => 94,
        b"nvim_list_chans" => 95,
        b"nvim_get_keymap" => 96,
        b"nvim_get_option" => 97,
        b"nvim_set_keymap" => 98,
        b"nvim_set_option" => 99,
        b"nvim__buf_stats" => 100,
        b"vim_unsubscribe" => 101,
        b"buffer_get_lines" => 102,
        b"buffer_set_lines" => 103,
        b"window_get_width" => 104,
        b"window_set_width" => 105,
        b"nvim_call_atomic" => 106,
        b"nvim__cmdwin_set" => 107,
        b"nvim_del_autocmd" => 108,
        b"nvim_get_context" => 109,
        b"tabpage_is_valid" => 110,
        b"nvim_win_del_var" => 111,
        b"nvim_win_get_buf" => 112,
        b"nvim_win_get_var" => 113,
        b"nvim_win_set_buf" => 114,
        b"nvim_win_set_var" => 115,
        b"nvim_input_mouse" => 116,
        b"nvim_unsubscribe" => 117,
        b"vim_report_error" => 118,
        b"nvim_err_writeln" => 119,
        b"nvim_error_event" => 120,
        b"nvim__screenshot" => 121,
        b"vim_get_api_info" => 122,
        b"vim_get_tabpages" => 123,
        b"nvim_buf_del_var" => 124,
        b"nvim_buf_get_var" => 125,
        b"nvim_buf_set_var" => 126,
        b"nvim_execute_lua" => 127,
        b"nvim_get_hl_by_id" => 128,
        b"nvim_buf_del_mark" => 129,
        b"nvim_buf_get_mark" => 130,
        b"nvim_buf_get_name" => 131,
        b"nvim_buf_set_mark" => 132,
        b"nvim_buf_set_name" => 133,
        b"nvim_open_tabpage" => 134,
        b"buffer_get_number" => 135,
        b"nvim_ui_set_focus" => 136,
        b"nvim__get_lib_dir" => 137,
        b"nvim_buf_get_text" => 138,
        b"nvim_buf_set_text" => 139,
        b"nvim_load_context" => 140,
        b"window_get_buffer" => 141,
        b"window_get_height" => 142,
        b"window_set_height" => 143,
        b"buffer_get_option" => 144,
        b"buffer_set_option" => 145,
        b"nvim__get_runtime" => 146,
        b"vim_call_function" => 147,
        b"window_get_option" => 148,
        b"window_set_option" => 149,
        b"nvim_buf_is_valid" => 150,
        b"nvim_win_is_valid" => 151,
        b"vim_name_to_color" => 152,
        b"nvim_get_autocmds" => 153,
        b"vim_get_color_map" => 154,
        b"nvim_get_api_info" => 155,
        b"nvim_get_commands" => 156,
        b"window_get_cursor" => 157,
        b"window_set_cursor" => 158,
        b"buffer_line_count" => 159,
        b"nvim_call_function" => 160,
        b"nvim_exec_autocmds" => 161,
        b"nvim_list_tabpages" => 162,
        b"nvim_get_chan_info" => 163,
        b"nvim_get_color_map" => 164,
        b"nvim_ui_set_option" => 165,
        b"nvim_ui_term_event" => 166,
        b"tabpage_get_window" => 167,
        b"nvim_buf_get_lines" => 168,
        b"nvim_win_get_width" => 169,
        b"nvim_buf_is_loaded" => 170,
        b"vim_command_output" => 171,
        b"nvim__complete_set" => 172,
        b"nvim__inspect_cell" => 173,
        b"nvim_ui_try_resize" => 174,
        b"nvim_buf_set_lines" => 175,
        b"nvim_win_set_hl_ns" => 176,
        b"nvim_win_set_width" => 177,
        b"window_get_tabpage" => 178,
        b"nvim__exec_lua_fast" => 179,
        b"nvim_get_hl_by_name" => 180,
        b"nvim_set_hl_ns_fast" => 181,
        b"nvim_buf_line_count" => 182,
        b"nvim_buf_del_keymap" => 183,
        b"nvim_buf_get_keymap" => 184,
        b"nvim_buf_set_keymap" => 185,
        b"nvim_win_get_height" => 186,
        b"nvim_win_set_height" => 187,
        b"nvim_buf_get_offset" => 188,
        b"nvim_create_augroup" => 189,
        b"window_get_position" => 190,
        b"tabpage_get_windows" => 191,
        b"nvim_clear_autocmds" => 192,
        b"nvim_win_get_config" => 193,
        b"nvim_win_set_config" => 194,
        b"nvim_buf_get_option" => 195,
        b"nvim_buf_set_option" => 196,
        b"nvim_get_namespaces" => 197,
        b"nvim_win_get_option" => 198,
        b"nvim_win_set_option" => 199,
        b"nvim_create_autocmd" => 200,
        b"nvim_buf_get_number" => 201,
        b"nvim_command_output" => 202,
        b"nvim_win_get_cursor" => 203,
        b"nvim_win_get_number" => 204,
        b"nvim_win_set_cursor" => 205,
        b"nvim_buf_del_extmark" => 206,
        b"nvim_buf_set_extmark" => 207,
        b"nvim_win_get_tabpage" => 208,
        b"nvim_get_current_buf" => 209,
        b"nvim_set_current_buf" => 210,
        b"nvim_set_current_dir" => 211,
        b"buffer_add_highlight" => 212,
        b"nvim_win_text_height" => 213,
        b"nvim_eval_statusline" => 214,
        b"vim_del_current_line" => 215,
        b"vim_get_current_line" => 216,
        b"vim_set_current_line" => 217,
        b"nvim_get_option_info" => 218,
        b"nvim_set_client_info" => 219,
        b"vim_change_directory" => 220,
        b"nvim_tabpage_del_var" => 221,
        b"nvim_tabpage_get_var" => 222,
        b"nvim_tabpage_set_var" => 223,
        b"nvim_get_current_win" => 224,
        b"nvim_set_current_win" => 225,
        b"nvim_tabpage_get_win" => 226,
        b"nvim_tabpage_set_win" => 227,
        b"nvim__runtime_inspect" => 228,
        b"nvim_buf_get_commands" => 229,
        b"nvim_buf_get_extmarks" => 230,
        b"nvim_create_namespace" => 231,
        b"nvim_del_current_line" => 232,
        b"nvim_del_user_command" => 233,
        b"vim_replace_termcodes" => 234,
        b"nvim_get_current_line" => 235,
        b"nvim_get_option_info2" => 236,
        b"nvim_get_option_value" => 237,
        b"nvim_get_runtime_file" => 238,
        b"nvim_parse_expression" => 239,
        b"buffer_get_line_slice" => 240,
        b"buffer_set_line_slice" => 241,
        b"nvim_set_current_line" => 242,
        b"nvim_set_option_value" => 243,
        b"nvim_tabpage_is_valid" => 244,
        b"nvim_win_get_position" => 245,
        b"nvim_del_augroup_by_id" => 246,
        b"vim_list_runtime_paths" => 247,
        b"nvim_ui_pum_set_bounds" => 248,
        b"vim_get_current_buffer" => 249,
        b"vim_set_current_buffer" => 250,
        b"buffer_clear_highlight" => 251,
        b"nvim_buf_add_highlight" => 252,
        b"nvim_ui_pum_set_height" => 253,
        b"nvim_get_proc_children" => 254,
        b"nvim_replace_termcodes" => 255,
        b"nvim_tabpage_list_wins" => 256,
        b"vim_get_current_window" => 257,
        b"vim_set_current_window" => 258,
        b"nvim_get_color_by_name" => 259,
        b"nvim_get_hl_id_by_name" => 260,
        b"nvim_call_dict_function" => 261,
        b"vim_get_current_tabpage" => 262,
        b"vim_set_current_tabpage" => 263,
        b"nvim_list_runtime_paths" => 264,
        b"nvim_tabpage_get_number" => 265,
        b"nvim_ui_try_resize_grid" => 266,
        b"nvim_buf_get_changedtick" => 267,
        b"nvim_get_current_tabpage" => 268,
        b"nvim_set_current_tabpage" => 269,
        b"nvim_del_augroup_by_name" => 270,
        b"nvim_buf_clear_highlight" => 271,
        b"nvim_buf_clear_namespace" => 272,
        b"nvim_create_user_command" => 273,
        b"nvim__buf_debug_extmarks" => 274,
        b"nvim_get_all_options_info" => 275,
        b"nvim_buf_del_user_command" => 276,
        b"nvim_buf_set_virtual_text" => 277,
        b"nvim__set_restart_on_crash" => 278,
        b"nvim_buf_get_extmark_by_id" => 279,
        b"nvim_select_popupmenu_item" => 280,
        b"nvim__invalidate_glyph_cache" => 281,
        b"nvim_buf_create_user_command" => 282,
        _ => -1,
    }
}

pub fn msgpack_rpc_get_handler_at(hash: i32) -> MsgpackRpcRequestHandler {
    METHOD_HANDLERS[hash as usize]
}

pub fn msgpack_rpc_get_handler_for(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,
    error: &mut Error,
) -> MsgpackRpcRequestHandler {
    let name = name.into_c_char_ptr();
    unsafe {
        let key_bytes = if name.is_null() || name_len == 0 {
            &[][..]
        } else {
            crate::memory::cstr_slice(name, name_len)
        };
        let hash = msgpack_rpc_get_handler_for_hash(key_bytes);
        if hash < 0 {
            let empty = c"<empty>";
            api_set_error!(
                error,
                ErrorType::ErrorTypeException,
                c"Invalid method: %.*s",
                if name_len > 0 {
                    name_len as i32
                } else {
                    (empty.to_bytes_with_nul().len()) as i32
                },
                if name_len > 0 { name } else { empty.as_ptr() },
            );
            return MsgpackRpcRequestHandler {
                name: c"",
                r#fn: None,
                fast: false,
                ret_alloc: false,
            };
        }
        msgpack_rpc_get_handler_at(hash)
    }
}

use crate::errors::E_FAST_API_DISABLED;
use crate::lua::converter::lua_State;
use crate::lua::executor::{api_free_luaref, nlua_is_deferred_safe, nlua_st};

const K_NLUA_PUSH_SPECIAL_CONST: i32 = 0x01;
const K_NLUA_PUSH_FREE_REFS_CONST: i32 = 0x02;

#[inline]
unsafe fn nlua_fast_api_disabled_error(lstate: *mut lua_State, fn_name: &std::ffi::CStr) -> i32 {
    unsafe { luaL_error(lstate, E_FAST_API_DISABLED.as_ptr(), fn_name.as_ptr()) }
}

#[inline]
fn nlua_finish_api_call(
    lstate: *mut lua_State,
    arena: &mut Arena,
    err: &mut Error,
    err_param: Option<&'static std::ffi::CStr>,
    nret: i32,
) -> i32 {
    unsafe {
        arena_mem_free(arena_finish(arena));
        if err.err_type != ErrorType::ErrorTypeNone {
            luaL_where(lstate, 1);
            if let Some(err_param) = err_param {
                lua_pushstring(lstate, c"Invalid '");
                lua_pushstring(lstate, err_param);
                lua_pushstring(lstate, c"': ");
            }
            lua_pushstring(lstate, err.msg_ptr());
            api_clear_error(err);
            lua_concat(lstate, if err_param.is_some() { 5 } else { 2 });
            return lua_error(lstate);
        }
        if nret < 0 { lua_gettop(lstate) } else { nret }
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_autocmds(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_get_autocmds = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_autocmds");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_get_autocmds_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_autocmds(&mut arg1, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const GET_AUTOCMDS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_create_autocmd(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_create_autocmd = Default::default();
        let arg1: Object;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_create_autocmd_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"event");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret =
                    nvim_create_autocmd(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_integer(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
                api_luarefs_free_object(arg1);
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const CREATE_AUTOCMD_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_autocmd(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_autocmd");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"id");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_autocmd(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_clear_autocmds(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_clear_autocmds = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_clear_autocmds");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_clear_autocmds_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_clear_autocmds(&mut arg1, &mut arena, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const CLEAR_AUTOCMDS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_create_augroup(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_create_augroup = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_create_augroup");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_create_augroup_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_create_augroup(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
                nlua_push_integer(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const CREATE_AUGROUP_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_augroup_by_id(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_augroup_by_id");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"id");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_augroup_by_id(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_augroup_by_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_augroup_by_name");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_augroup_by_name(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_exec_autocmds(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_exec_autocmds = Default::default();
        let arg1: Object;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_exec_autocmds");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_exec_autocmds_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"event");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_exec_autocmds(arg1, &mut arg2, &mut arena, &mut err);
                nlua_st().active_lstate = save_active_lstate;
                api_luarefs_free_object(arg1);
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const EXEC_AUTOCMDS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_line_count(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_line_count");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_line_count(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_attach(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_buf_attach = Default::default();
        let arg2: Boolean;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_attach");
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_buf_attach_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"send_buffer");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_buf_attach(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                    nlua_push_boolean(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const BUF_ATTACH_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_lines(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg4: Boolean;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) != 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_lines");
                        }
                        arg4 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"strict_indexing");
                            break 'exit_0;
                        }
                        arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"end");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"start");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buf");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        let pretop = lua_gettop(lstate);
                        let ret = nvim_buf_get_lines(
                            LUA_INTERNAL_CALL,
                            arg1,
                            arg2,
                            arg3,
                            arg4,
                            &mut arena,
                            lstate,
                            &mut err,
                        );
                        debug_assert!(
                            err.err_type != ErrorType::ErrorTypeNone
                                || lua_gettop(lstate) == pretop + 1
                        );
                        let _ = ret;
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_lines(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg5: Array;
        let arg4: Boolean;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) != 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_lines");
                            }
                            if text_locked() {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeException,
                                    get_text_locked_msg(),
                                );
                                break 'exit_0;
                            }
                            arg5 = nlua_pop_array(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"replacement");
                                break 'exit_0;
                            }
                            arg4 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"strict_indexing");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"end");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"start");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buf");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            nvim_buf_set_lines(
                                LUA_INTERNAL_CALL,
                                arg1,
                                arg2,
                                arg3,
                                arg4,
                                arg5,
                                &mut arena,
                                &mut err,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_text(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg6: Array;
        let arg5: Integer;
        let arg4: Integer;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            'exit_5: {
                                if lua_gettop(lstate) != 6 {
                                    api_set_error_str(
                                        &mut err,
                                        ErrorType::ErrorTypeValidation,
                                        c"Expected 6 arguments",
                                    );
                                    break 'exit_0;
                                }
                                if !nlua_is_deferred_safe() {
                                    return nlua_fast_api_disabled_error(
                                        lstate,
                                        c"nvim_buf_set_text",
                                    );
                                }
                                if text_locked() {
                                    api_set_error_str(
                                        &mut err,
                                        ErrorType::ErrorTypeException,
                                        get_text_locked_msg(),
                                    );
                                    break 'exit_0;
                                }
                                arg6 = nlua_pop_array(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"replacement");
                                    break 'exit_0;
                                }
                                arg5 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"end_col");
                                    break 'exit_1;
                                }
                                arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"end_row");
                                    break 'exit_2;
                                }
                                arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"start_col");
                                    break 'exit_3;
                                }
                                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"start_row");
                                    break 'exit_4;
                                }
                                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"buf");
                                    break 'exit_5;
                                }
                                let save_active_lstate = nlua_st().active_lstate;
                                nlua_st().active_lstate = lstate;
                                nvim_buf_set_text(
                                    LUA_INTERNAL_CALL,
                                    arg1,
                                    arg2,
                                    arg3,
                                    arg4,
                                    arg5,
                                    arg6,
                                    &mut arena,
                                    &mut err,
                                );
                                nlua_st().active_lstate = save_active_lstate;
                            }
                        }
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_text(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg6: KeyDict_empty = Default::default();
        let arg5: Integer;
        let arg4: Integer;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            'exit_5: {
                                if lua_gettop(lstate) < 5 || lua_gettop(lstate) > 6 {
                                    api_set_error_str(
                                        &mut err,
                                        ErrorType::ErrorTypeValidation,
                                        c"Expected 5 to 6 arguments",
                                    );
                                    break 'exit_0;
                                }
                                if !nlua_is_deferred_safe() {
                                    return nlua_fast_api_disabled_error(
                                        lstate,
                                        c"nvim_buf_get_text",
                                    );
                                }
                                if lua_gettop(lstate) == 6 {
                                    nlua_pop_keydict(
                                        lstate,
                                        (&raw mut arg6).cast(),
                                        Some(key_dict_empty_get_field),
                                        &mut err_param,
                                        &mut arena,
                                        &mut err,
                                    );
                                    if err.err_type != ErrorType::ErrorTypeNone {
                                        break 'exit_1;
                                    }
                                }
                                arg5 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"end_col");
                                    break 'exit_1;
                                }
                                arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"end_row");
                                    break 'exit_2;
                                }
                                arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"start_col");
                                    break 'exit_3;
                                }
                                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"start_row");
                                    break 'exit_4;
                                }
                                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"buf");
                                    break 'exit_5;
                                }
                                let save_active_lstate = nlua_st().active_lstate;
                                nlua_st().active_lstate = lstate;
                                let pretop = lua_gettop(lstate);
                                let ret = nvim_buf_get_text(
                                    LUA_INTERNAL_CALL,
                                    arg1,
                                    arg2,
                                    arg3,
                                    arg4,
                                    arg5,
                                    &mut arg6,
                                    &mut arena,
                                    lstate,
                                    &mut err,
                                );
                                debug_assert!(
                                    err.err_type != ErrorType::ErrorTypeNone
                                        || lua_gettop(lstate) == pretop + 1
                                );
                                let _ = ret;
                                nlua_st().active_lstate = save_active_lstate;
                            }
                        }
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg6).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_offset(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_offset");
                }
                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"index");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_get_offset(arg1, arg2, &mut err);
                nlua_push_integer(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_buf_get_var(arg1, arg2, &mut arena, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_changedtick(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_changedtick");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_get_changedtick(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_keymap");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"mode");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_get_keymap(arg1, arg2, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg5: KeyDict_keymap = Default::default();
        let arg4: NvimString;
        let arg3: NvimString;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) < 4 || lua_gettop(lstate) > 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 4 to 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(
                                    lstate,
                                    c"nvim_buf_set_keymap",
                                );
                            }
                            if lua_gettop(lstate) == 5 {
                                nlua_pop_keydict(
                                    lstate,
                                    (&raw mut arg5).cast(),
                                    Some(key_dict_keymap_get_field),
                                    &mut err_param,
                                    &mut arena,
                                    &mut err,
                                );
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    break 'exit_1;
                                }
                            }
                            arg4 = nlua_pop_string(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"rhs");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_string(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"lhs");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"mode");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buf");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            nvim_buf_set_keymap(
                                LUA_INTERNAL_CALL,
                                arg1,
                                arg2,
                                arg3,
                                arg4,
                                &mut arg5,
                                &mut err,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg5).cast(),
                (&raw const KEYMAP_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_del_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_keymap_del = Default::default();
        let arg3: NvimString;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_buf_del_keymap");
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_keymap_del_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"lhs");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"mode");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buf");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_buf_del_keymap(
                            LUA_INTERNAL_CALL,
                            arg1,
                            arg2,
                            arg3,
                            &mut arg4,
                            &mut err,
                        );
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const KEYMAP_DEL_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_var");
                    }
                    arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_buf_set_var(arg1, arg2, arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_object(arg3);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_del_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_del_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_buf_del_var(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_name");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_get_name(arg1, &mut err);
            nlua_push_string(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_name");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_buf_set_name(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_is_loaded(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_is_loaded");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_is_loaded(arg1);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_delete(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_buf_delete = Default::default();
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_delete");
                }
                if text_locked() {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeException,
                        get_text_locked_msg(),
                    );
                    break 'exit_0;
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_buf_delete_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_buf_delete(arg1, &mut arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const BUF_DELETE_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_is_valid(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_is_valid");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_is_valid(arg1);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_del_mark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_del_mark");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_del_mark(arg1, arg2, &mut err);
                nlua_push_boolean(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_mark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg5: KeyDict_empty = Default::default();
        let arg4: Integer;
        let arg3: Integer;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) < 4 || lua_gettop(lstate) > 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 4 to 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_mark");
                            }
                            if lua_gettop(lstate) == 5 {
                                nlua_pop_keydict(
                                    lstate,
                                    (&raw mut arg5).cast(),
                                    Some(key_dict_empty_get_field),
                                    &mut err_param,
                                    &mut arena,
                                    &mut err,
                                );
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    break 'exit_1;
                                }
                            }
                            arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"col");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"line");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"name");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buf");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            let ret =
                                nvim_buf_set_mark(arg1, arg2, arg3, arg4, &mut arg5, &mut err);
                            nlua_push_boolean(
                                lstate,
                                ret,
                                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg5).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_mark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_mark");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_get_mark(arg1, arg2, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_call(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: LuaRef;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_call");
                }
                arg2 = nlua_pop_lua_ref(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"fn");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_call(arg1, arg2, lstate, &mut err);
                let _ = ret;
                nlua_st().active_lstate = save_active_lstate;
            }
            api_free_luaref(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_stats(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__buf_stats");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_stats(arg1, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_parse_cmd(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_empty = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_empty_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"str");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_parse_cmd(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_keydict(
                    lstate,
                    (&raw mut ret).cast(),
                    (&raw const CMD_TABLE).cast_mut().cast(),
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_cmd(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_cmd_opts = Default::default();
        let mut arg1: KeyDict_cmd = Default::default();
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 1 to 2 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_cmd");
                    }
                    if lua_gettop(lstate) == 2 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg2).cast(),
                            Some(key_dict_cmd_opts_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_cmd_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_cmd(
                        LUA_INTERNAL_CALL,
                        &mut arg1,
                        &mut arg2,
                        &mut arena,
                        &mut err,
                    );
                    nlua_push_string(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
                api_luarefs_free_keydict(
                    (&raw mut arg1).cast(),
                    (&raw const CMD_TABLE).cast_mut().cast(),
                );
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const CMD_OPTS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_create_user_command(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_user_command = Default::default();
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_create_user_command");
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_user_command_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"cmd");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_create_user_command(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
                api_luarefs_free_object(arg2);
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const USER_COMMAND_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_user_command(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_user_command");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_user_command(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_create_user_command(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_user_command = Default::default();
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(
                                lstate,
                                c"nvim_buf_create_user_command",
                            );
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_user_command_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"cmd");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"name");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buf");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_buf_create_user_command(
                            LUA_INTERNAL_CALL,
                            arg1,
                            arg2,
                            arg3,
                            &mut arg4,
                            &mut err,
                        );
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
                api_luarefs_free_object(arg3);
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const USER_COMMAND_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_del_user_command(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_del_user_command");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_buf_del_user_command(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_commands(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_get_commands = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_commands");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_get_commands_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_commands(&mut arg1, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const GET_COMMANDS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_commands(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_get_commands = Default::default();
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_commands");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_get_commands_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_buf_get_commands(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const GET_COMMANDS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_exec(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_exec");
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"output");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"src");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_exec(LUA_INTERNAL_CALL, arg1, arg2, &mut err);
                nlua_push_string(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
                api_free_string(ret);
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_command_output(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_command_output");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"command");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_command_output(LUA_INTERNAL_CALL, arg1, &mut err);
            nlua_push_string(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
            api_free_string(ret);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_number(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_number");
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buffer");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_buf_get_number(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_clear_highlight(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg4: Integer;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) != 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(
                                lstate,
                                c"nvim_buf_clear_highlight",
                            );
                        }
                        arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"line_end");
                            break 'exit_0;
                        }
                        arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"line_start");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"ns_id");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buffer");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_buf_clear_highlight(arg1, arg2, arg3, arg4, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_add_highlight(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg6: Integer;
        let arg5: Integer;
        let arg4: Integer;
        let arg3: NvimString;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            'exit_5: {
                                if lua_gettop(lstate) != 6 {
                                    api_set_error_str(
                                        &mut err,
                                        ErrorType::ErrorTypeValidation,
                                        c"Expected 6 arguments",
                                    );
                                    break 'exit_0;
                                }
                                if !nlua_is_deferred_safe() {
                                    return nlua_fast_api_disabled_error(
                                        lstate,
                                        c"nvim_buf_add_highlight",
                                    );
                                }
                                arg6 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"col_end");
                                    break 'exit_0;
                                }
                                arg5 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"col_start");
                                    break 'exit_1;
                                }
                                arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"line");
                                    break 'exit_2;
                                }
                                arg3 = nlua_pop_string(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"hl_group");
                                    break 'exit_3;
                                }
                                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"ns_id");
                                    break 'exit_4;
                                }
                                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"buffer");
                                    break 'exit_5;
                                }
                                let save_active_lstate = nlua_st().active_lstate;
                                nlua_st().active_lstate = lstate;
                                let ret = nvim_buf_add_highlight(
                                    arg1, arg2, arg3, arg4, arg5, arg6, &mut err,
                                );
                                nlua_push_integer(
                                    lstate,
                                    ret,
                                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                                );
                                nlua_st().active_lstate = save_active_lstate;
                            }
                        }
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_virtual_text(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg5: KeyDict_empty = Default::default();
        let arg4: Array;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) < 4 || lua_gettop(lstate) > 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 4 to 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(
                                    lstate,
                                    c"nvim_buf_set_virtual_text",
                                );
                            }
                            if lua_gettop(lstate) == 5 {
                                nlua_pop_keydict(
                                    lstate,
                                    (&raw mut arg5).cast(),
                                    Some(key_dict_empty_get_field),
                                    &mut err_param,
                                    &mut arena,
                                    &mut err,
                                );
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    break 'exit_1;
                                }
                            }
                            arg4 = nlua_pop_array(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"chunks");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"line");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"src_id");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buffer");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            let ret = nvim_buf_set_virtual_text(
                                arg1, arg2, arg3, arg4, &mut arg5, &mut err,
                            );
                            nlua_push_integer(
                                lstate,
                                ret,
                                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg5).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_hl_by_id(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_hl_by_id");
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"rgb");
                    break 'exit_0;
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"hl_id");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_hl_by_id(arg1, arg2, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_hl_by_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_hl_by_name");
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"rgb");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_hl_by_name(arg1, arg2, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_option_info(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_option_info");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_option_info(arg1, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_set_option");
                }
                arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"value");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_set_option(LUA_INTERNAL_CALL, arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_object(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_option");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_get_option(arg1, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
            api_free_object(ret);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_option");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buffer");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_buf_get_option(arg1, arg2, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
                api_free_object(ret);
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_option");
                    }
                    arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buffer");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_buf_set_option(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_object(arg3);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_option");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"window");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_win_get_option(arg1, arg2, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
                api_free_object(ret);
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_option(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_option");
                    }
                    arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"window");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_win_set_option(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_object(arg3);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_out_write(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_out_write");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"str");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_out_write(arg1);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_err_write(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_err_write");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"str");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_err_write(arg1);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_err_writeln(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_err_writeln");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"str");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_err_writeln(arg1);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_notify(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Dict;
        let arg2: Integer;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_notify");
                    }
                    arg3 = nlua_pop_dict(lstate, false, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"opts");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"log_level");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"msg");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let mut ret = nvim_notify(arg1, arg2, arg3, &mut arena, &mut err);
                    nlua_push_object(
                        lstate,
                        &raw mut ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_height(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Integer;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_height");
                }
                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"height");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_height(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_width(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Integer;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_width");
                }
                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"width");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_width(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_create_namespace(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_create_namespace");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_create_namespace(arg1);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_namespaces(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_namespaces");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_namespaces(&mut arena);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_extmark_by_id(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_get_extmark = Default::default();
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(
                                lstate,
                                c"nvim_buf_get_extmark_by_id",
                            );
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_get_extmark_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"id");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"ns_id");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buf");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        let ret = nvim_buf_get_extmark_by_id(
                            arg1, arg2, arg3, &mut arg4, &mut arena, &mut err,
                        );
                        nlua_push_array(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const GET_EXTMARK_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_get_extmarks(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg5: KeyDict_get_extmarks = Default::default();
        let arg4: Object;
        let arg3: Object;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) < 4 || lua_gettop(lstate) > 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 4 to 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(
                                    lstate,
                                    c"nvim_buf_get_extmarks",
                                );
                            }
                            if lua_gettop(lstate) == 5 {
                                nlua_pop_keydict(
                                    lstate,
                                    (&raw mut arg5).cast(),
                                    Some(key_dict_get_extmarks_get_field),
                                    &mut err_param,
                                    &mut arena,
                                    &mut err,
                                );
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    break 'exit_1;
                                }
                            }
                            arg4 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"end");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"start");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"ns_id");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buf");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            let ret = nvim_buf_get_extmarks(
                                arg1, arg2, arg3, arg4, &mut arg5, &mut arena, &mut err,
                            );
                            nlua_push_array(
                                lstate,
                                ret,
                                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                    api_luarefs_free_object(arg3);
                }
                api_luarefs_free_object(arg4);
            }
            api_luarefs_free_keydict(
                (&raw mut arg5).cast(),
                (&raw const GET_EXTMARKS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_set_extmark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg5: KeyDict_set_extmark = Default::default();
        let arg4: Integer;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            if lua_gettop(lstate) < 4 || lua_gettop(lstate) > 5 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 4 to 5 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(
                                    lstate,
                                    c"nvim_buf_set_extmark",
                                );
                            }
                            if lua_gettop(lstate) == 5 {
                                nlua_pop_keydict(
                                    lstate,
                                    (&raw mut arg5).cast(),
                                    Some(key_dict_set_extmark_get_field),
                                    &mut err_param,
                                    &mut arena,
                                    &mut err,
                                );
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    break 'exit_1;
                                }
                            }
                            arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"col");
                                break 'exit_1;
                            }
                            arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"line");
                                break 'exit_2;
                            }
                            arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"ns_id");
                                break 'exit_3;
                            }
                            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                            if err.err_type != ErrorType::ErrorTypeNone {
                                err_param = Some(c"buf");
                                break 'exit_4;
                            }
                            let save_active_lstate = nlua_st().active_lstate;
                            nlua_st().active_lstate = lstate;
                            let ret =
                                nvim_buf_set_extmark(arg1, arg2, arg3, arg4, &mut arg5, &mut err);
                            nlua_push_integer(
                                lstate,
                                ret,
                                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                            );
                            nlua_st().active_lstate = save_active_lstate;
                        }
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg5).cast(),
                (&raw const SET_EXTMARK_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_del_extmark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_del_extmark");
                    }
                    arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"id");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"ns_id");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_buf_del_extmark(arg1, arg2, arg3, &mut err);
                    nlua_push_boolean(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_clear_namespace(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg4: Integer;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) != 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(
                                lstate,
                                c"nvim_buf_clear_namespace",
                            );
                        }
                        arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"line_end");
                            break 'exit_0;
                        }
                        arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"line_start");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"ns_id");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"buf");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_buf_clear_namespace(arg1, arg2, arg3, arg4, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_decoration_provider(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_set_decoration_provider = Default::default();
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_set_decoration_provider");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_set_decoration_provider_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"ns_id");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_set_decoration_provider(arg1, &mut arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const SET_DECORATION_PROVIDER_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_buf_debug_extmarks(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Boolean;
        let arg2: Boolean;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim__buf_debug_extmarks");
                    }
                    arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"dot");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"keys");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_buf_debug_extmarks(arg1, arg2, arg3, &mut err);
                    nlua_push_string(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                    api_free_string(ret);
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_ns_set(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_ns_opts = Default::default();
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__ns_set");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_ns_opts_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"ns_id");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_ns_set(arg1, &mut arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const NS_OPTS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_ns_get(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__ns_get");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"ns_id");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_ns_get(arg1, &mut arena, &mut err);
            nlua_push_keydict(
                lstate,
                (&raw mut ret).cast(),
                (&raw const NS_OPTS_TABLE).cast_mut().cast(),
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_option_to_object(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__option_to_object");
                }
                arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"value");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_option_to_object(arg1, arg2, &mut arena, &mut err);
                nlua_push_object(lstate, &raw mut ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_object(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_option_value(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_option = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_option_value");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_option_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_get_option_value(arg1, &mut arg2, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
                api_free_object(ret);
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const OPTION_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_option_value(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_option = Default::default();
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_set_option_value");
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_option_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let mut ret = nvim_set_option_value(
                        LUA_INTERNAL_CALL,
                        arg1,
                        arg2,
                        &mut arg3,
                        &mut arena,
                        &mut err,
                    );
                    nlua_push_object(
                        lstate,
                        &raw mut ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
                api_luarefs_free_object(arg2);
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const OPTION_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_all_options_info(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_all_options_info");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_all_options_info(&mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_option_info2(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_option = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_option_info2");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_option_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_option_info2(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const OPTION_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_list_wins(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Tabpage;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_list_wins");
            }
            arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"tabpage");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_tabpage_list_wins(arg1, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_get_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Tabpage;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_get_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"tabpage");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_tabpage_get_var(arg1, arg2, &mut arena, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_set_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Tabpage;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_set_var");
                    }
                    arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"tabpage");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_tabpage_set_var(arg1, arg2, arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_object(arg3);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_del_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Tabpage;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_del_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"tabpage");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_tabpage_del_var(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_get_win(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Tabpage;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_get_win");
            }
            arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"tabpage");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_tabpage_get_win(arg1, &mut err);
            nlua_push_Window(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_set_win(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Window;
        let arg1: Tabpage;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_set_win");
                }
                arg2 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"tabpage");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_tabpage_set_win(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_get_number(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Tabpage;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_get_number");
            }
            arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"tabpage");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_tabpage_get_number(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_tabpage_is_valid(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Tabpage;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_tabpage_is_valid");
            }
            arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"tabpage");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_tabpage_is_valid(arg1);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_open_tabpage(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_tabpage_config = Default::default();
        let arg2: Boolean;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_open_tabpage");
                    }
                    if text_locked() {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeException,
                            get_text_locked_msg(),
                        );
                        break 'exit_0;
                    }
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg3).cast(),
                        Some(key_dict_tabpage_config_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"enter");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_open_tabpage(arg1, arg2, &mut arg3, &mut err);
                    nlua_push_Tabpage(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const TABPAGE_CONFIG_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_ui_send(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_ui_send");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"content");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_ui_send(LUA_INTERNAL_CALL, arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_hl_id_by_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_hl_id_by_name");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_hl_id_by_name(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_hl(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_get_highlight = Default::default();
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_hl");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_get_highlight_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"ns_id");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_hl(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const GET_HIGHLIGHT_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_hl(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_highlight = Default::default();
        let arg2: NvimString;
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_set_hl");
                    }
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg3).cast(),
                        Some(key_dict_highlight_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"ns_id");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_set_hl(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const HIGHLIGHT_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_hl_ns(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_get_ns = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_hl_ns");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_get_ns_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_hl_ns(&mut arg1, &mut err);
                nlua_push_integer(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const GET_NS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_hl_ns(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_hl_ns");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"ns_id");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_hl_ns(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_hl_ns_fast(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"ns_id");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_hl_ns_fast(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_feedkeys(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Boolean;
        let arg2: NvimString;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_feedkeys");
                    }
                    arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"escape_ks");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"mode");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"keys");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_feedkeys(arg1, arg2, arg3);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_input(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"keys");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_input(LUA_INTERNAL_CALL, arg1);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_input_mouse(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg6: Integer;
        let arg5: Integer;
        let arg4: Integer;
        let arg3: NvimString;
        let arg2: NvimString;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        'exit_4: {
                            'exit_5: {
                                if lua_gettop(lstate) != 6 {
                                    api_set_error_str(
                                        &mut err,
                                        ErrorType::ErrorTypeValidation,
                                        c"Expected 6 arguments",
                                    );
                                    break 'exit_0;
                                }
                                arg6 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"col");
                                    break 'exit_0;
                                }
                                arg5 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"row");
                                    break 'exit_1;
                                }
                                arg4 = nlua_pop_integer(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"grid");
                                    break 'exit_2;
                                }
                                arg3 = nlua_pop_string(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"modifier");
                                    break 'exit_3;
                                }
                                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"action");
                                    break 'exit_4;
                                }
                                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                                if err.err_type != ErrorType::ErrorTypeNone {
                                    err_param = Some(c"button");
                                    break 'exit_5;
                                }
                                let save_active_lstate = nlua_st().active_lstate;
                                nlua_st().active_lstate = lstate;
                                nvim_input_mouse(arg1, arg2, arg3, arg4, arg5, arg6, &mut err);
                                nlua_st().active_lstate = save_active_lstate;
                            }
                        }
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_replace_termcodes(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg4: Boolean;
        let arg3: Boolean;
        let arg2: Boolean;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) != 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 arguments",
                            );
                            break 'exit_0;
                        }
                        arg4 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"special");
                            break 'exit_0;
                        }
                        arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"do_lt");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"from_part");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"str");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        let ret = nvim_replace_termcodes(arg1, arg2, arg3, arg4);
                        nlua_push_string(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        nlua_st().active_lstate = save_active_lstate;
                        api_free_string(ret);
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_strwidth(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_strwidth");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"text");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_strwidth(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_runtime_paths(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_runtime_paths");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_runtime_paths(&mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_runtime_inspect(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__runtime_inspect");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_runtime_inspect(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_runtime_file(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"all");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_runtime_file(arg1, arg2, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_lib_dir(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__get_lib_dir");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_lib_dir();
            nlua_push_string(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
            api_free_string(ret);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_runtime(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_runtime = Default::default();
        let arg2: Boolean;
        let arg1: Array;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_runtime_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"all");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_array(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"pat");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_get_runtime(arg1, arg2, &mut arg3, &mut arena, &mut err);
                    nlua_push_array(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const RUNTIME_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_current_dir(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_current_dir");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"dir");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_current_dir(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_current_line(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_current_line");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_current_line(&mut arena, &mut err);
            nlua_push_string(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_current_line(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_current_line");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"line");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_current_line(arg1, &mut arena, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_current_line(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_current_line");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_current_line(&mut arena, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_var");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_get_var(arg1, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_set_var");
                }
                arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"value");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_set_var(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_object(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_var");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_del_var(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_vvar(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_vvar");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_get_vvar(arg1, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_vvar(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Object;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_set_vvar");
                }
                arg2 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"value");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_set_vvar(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_object(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_echo(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_echo_opts = Default::default();
        let arg2: Boolean;
        let arg1: Array;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_echo");
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_echo_opts_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"history");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_array(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"chunks");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let mut ret = nvim_echo(arg1, arg2, &mut arg3, &mut err);
                    nlua_push_object(
                        lstate,
                        &raw mut ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const ECHO_OPTS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_bufs(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_bufs");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_bufs(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_current_buf(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_current_buf");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_current_buf();
            nlua_push_Buffer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_current_buf(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Buffer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_current_buf");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"buf");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_current_buf(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_wins(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_wins");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_wins(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_current_win(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_current_win");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_current_win();
            nlua_push_Window(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_current_win(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_current_win");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_current_win(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_create_buf(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: Boolean;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_create_buf");
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"scratch");
                    break 'exit_0;
                }
                arg1 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"listed");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_create_buf(arg1, arg2, &mut err);
                nlua_push_Buffer(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_open_term(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_open_term = Default::default();
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_open_term");
                }
                if text_locked() {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeException,
                        get_text_locked_msg(),
                    );
                    break 'exit_0;
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_open_term_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_open_term(arg1, &mut arg2, &mut err);
                nlua_push_integer(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const OPEN_TERM_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_chan_send(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_chan_send");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"data");
                    break 'exit_0;
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"chan");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_chan_send(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_tabpages(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_tabpages");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_tabpages(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_current_tabpage(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_current_tabpage");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_current_tabpage();
            nlua_push_Tabpage(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_current_tabpage(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Tabpage;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_set_current_tabpage");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_Tabpage(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"tabpage");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_set_current_tabpage(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_paste(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Integer;
        let arg2: Boolean;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_paste");
                    }
                    if text_locked() {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeException,
                            get_text_locked_msg(),
                        );
                        break 'exit_0;
                    }
                    arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"phase");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"crlf");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"data");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_paste(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut arena, &mut err);
                    nlua_push_boolean(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_put(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg4: Boolean;
        let arg3: Boolean;
        let arg2: NvimString;
        let arg1: Array;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) != 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_put");
                        }
                        if text_locked() {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeException,
                                get_text_locked_msg(),
                            );
                            break 'exit_0;
                        }
                        arg4 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"follow");
                            break 'exit_0;
                        }
                        arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"after");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"type");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_array(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"lines");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_put(arg1, arg2, arg3, arg4, &mut arena, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_color_by_name(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_color_by_name");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_color_by_name(arg1);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_color_map(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_color_map");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_color_map(&mut arena);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_context(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_context = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_context");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_context_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_context(&mut arg1, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const CONTEXT_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_load_context(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Dict;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_load_context");
            }
            arg1 = nlua_pop_dict(lstate, false, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"dict");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_load_context(arg1, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_mode(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_mode(&mut arena);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_keymap");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"mode");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_keymap(arg1, &mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_keymap = Default::default();
        let arg3: NvimString;
        let arg2: NvimString;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_set_keymap");
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_keymap_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"rhs");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"lhs");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"mode");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_set_keymap(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut arg4, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const KEYMAP_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_keymap(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_keymap_del = Default::default();
        let arg2: NvimString;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) < 2 || lua_gettop(lstate) > 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 2 to 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_del_keymap");
                    }
                    if lua_gettop(lstate) == 3 {
                        nlua_pop_keydict(
                            lstate,
                            (&raw mut arg3).cast(),
                            Some(key_dict_keymap_del_get_field),
                            &mut err_param,
                            &mut arena,
                            &mut err,
                        );
                        if err.err_type != ErrorType::ErrorTypeNone {
                            break 'exit_1;
                        }
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"lhs");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"mode");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_del_keymap(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const KEYMAP_DEL_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_chan_set(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_chan_set = Default::default();
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                nlua_pop_keydict(
                    lstate,
                    (&raw mut arg2).cast(),
                    Some(key_dict_chan_set_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"chan");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_chan_set(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const CHAN_SET_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_cmdwin_set(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Buffer;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__cmdwin_set");
                }
                arg2 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"type");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_cmdwin_set(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_chan_info(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_chan_info");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"chan");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_chan_info(LUA_INTERNAL_CALL, arg1, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_chans(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_chans");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_chans(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_id(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Object;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__id");
            }
            arg1 = nlua_pop_object(lstate, true, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"obj");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_id(arg1, &mut arena);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
            api_luarefs_free_object(arg1);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_id_array(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Array;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__id_array");
            }
            arg1 = nlua_pop_array(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"arr");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_id_array(arg1, &mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_id_dict(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Dict;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__id_dict");
            }
            arg1 = nlua_pop_dict(lstate, false, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"dct");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_id_dict(arg1, &mut arena);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_id_float(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Float;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__id_float");
            }
            arg1 = nlua_pop_float(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"flt");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim__id_float(arg1);
            nlua_push_float(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_stats(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__stats");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_stats(&mut arena);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_list_uis(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_list_uis");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_list_uis(&mut arena);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_proc_children(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_proc_children");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"pid");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_get_proc_children(arg1, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_proc(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Integer;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_get_proc");
            }
            arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"pid");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_get_proc(arg1, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_select_popupmenu_item(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_empty = Default::default();
        let arg3: Boolean;
        let arg2: Boolean;
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(
                                lstate,
                                c"nvim_select_popupmenu_item",
                            );
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_empty_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"finish");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"insert");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"item");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_select_popupmenu_item(arg1, arg2, arg3, &mut arg4, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_inspect_cell(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim__inspect_cell");
                    }
                    arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"col");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"row");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"grid");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_inspect_cell(arg1, arg2, arg3, &mut arena, &mut err);
                    nlua_push_array(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_screenshot(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"path");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_screenshot(arg1);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_invalidate_glyph_cache(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let err_param: Option<&'static std::ffi::CStr> = None;
        'exit_0: {
            if lua_gettop(lstate) != 0 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 0 arguments",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim__invalidate_glyph_cache");
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_invalidate_glyph_cache();
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_unpack(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"str");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_unpack(arg1, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_del_mark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_del_mark");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"name");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_del_mark(arg1, &mut err);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_get_mark(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_empty = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_get_mark");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_empty_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_get_mark(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const EMPTY_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_eval_statusline(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_eval_statusline = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_eval_statusline_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"str");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_eval_statusline(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const EVAL_STATUSLINE_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_complete_set(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_complete_set = Default::default();
        let arg1: Integer;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__complete_set");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_complete_set_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"index");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_complete_set(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const COMPLETE_SET_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_redraw(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg1: KeyDict_redraw = Default::default();
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) > 1 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected at most 1 argument",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__redraw");
                }
                if lua_gettop(lstate) == 1 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg1).cast(),
                        Some(key_dict_redraw_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_redraw(&mut arg1, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg1).cast(),
                (&raw const REDRAW_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_set_restart_on_crash(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Array;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim__set_restart_on_crash");
                }
                arg2 = nlua_pop_array(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"argv");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"progpath");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_set_restart_on_crash(arg1, arg2);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_exec2(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_exec_opts = Default::default();
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_exec2");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_exec_opts_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"src");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_exec2(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
                nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
                api_free_dict(ret);
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const EXEC_OPTS_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_command(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_command");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"cmd");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_command(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_eval(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: NvimString;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_eval");
            }
            arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"expr");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_eval(arg1, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &raw mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_call_function(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Array;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_call_function");
                }
                arg2 = nlua_pop_array(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"args");
                    break 'exit_0;
                }
                arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"fn");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret =
                    nvim_call_function(LUA_INTERNAL_CALL, arg1, arg2, &mut arena, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_call_dict_function(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Array;
        let arg2: NvimString;
        let arg1: Object;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_call_dict_function");
                    }
                    arg3 = nlua_pop_array(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"args");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"fn");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"dict");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let mut ret = nvim_call_dict_function(
                        LUA_INTERNAL_CALL,
                        arg1,
                        arg2,
                        arg3,
                        &mut arena,
                        &mut err,
                    );
                    nlua_push_object(
                        lstate,
                        &raw mut ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                    api_luarefs_free_object(arg1);
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_parse_expression(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Boolean;
        let arg2: NvimString;
        let arg1: NvimString;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    arg3 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"hl");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"flags");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"expr");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_parse_expression(arg1, arg2, arg3, &mut arena, &mut err);
                    nlua_push_dict(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_open_win(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg3: KeyDict_win_config = Default::default();
        let arg2: Boolean;
        let arg1: Buffer;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_open_win");
                    }
                    if text_locked() {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeException,
                            get_text_locked_msg(),
                        );
                        break 'exit_0;
                    }
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg3).cast(),
                        Some(key_dict_win_config_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                    arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"enter");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"buf");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    let ret = nvim_open_win(arg1, arg2, &mut arg3, &mut err);
                    nlua_push_Window(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg3).cast(),
                (&raw const WIN_CONFIG_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_config(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_win_config = Default::default();
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_config");
                }
                nlua_pop_keydict(
                    lstate,
                    (&raw mut arg2).cast(),
                    Some(key_dict_win_config_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_config(arg1, &mut arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const WIN_CONFIG_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_config(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_config");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let mut ret = nvim_win_get_config(arg1, &mut arena, &mut err);
            nlua_push_keydict(
                lstate,
                (&raw mut ret).cast(),
                (&raw const WIN_CONFIG_TABLE).cast_mut().cast(),
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_buf(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_buf");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_buf(arg1, &mut err);
            nlua_push_Buffer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_buf(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Buffer;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_buf");
                }
                if text_locked() {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeException,
                        get_text_locked_msg(),
                    );
                    break 'exit_0;
                }
                arg2 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"buf");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_buf(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_cursor(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_cursor");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_cursor(arg1, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_cursor(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Array;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_cursor");
                }
                arg2 = nlua_pop_array(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"pos");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_cursor(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_height(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_height");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_height(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_width(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_width");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_width(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let mut ret = nvim_win_get_var(arg1, arg2, &mut arena, &mut err);
                nlua_push_object(
                    lstate,
                    &raw mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg3: Object;
        let arg2: NvimString;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    if lua_gettop(lstate) != 3 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_var");
                    }
                    arg3 = nlua_pop_object(lstate, true, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"value");
                        break 'exit_0;
                    }
                    arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"name");
                        break 'exit_1;
                    }
                    arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        err_param = Some(c"win");
                        break 'exit_2;
                    }
                    let save_active_lstate = nlua_st().active_lstate;
                    nlua_st().active_lstate = lstate;
                    nvim_win_set_var(arg1, arg2, arg3, &mut err);
                    nlua_st().active_lstate = save_active_lstate;
                }
            }
            api_luarefs_free_object(arg3);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_del_var(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: NvimString;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_del_var");
                }
                arg2 = nlua_pop_string(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"name");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_del_var(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_position(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_position");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_position(arg1, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_tabpage(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_tabpage");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_tabpage(arg1, &mut err);
            nlua_push_Tabpage(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_get_number(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_get_number");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_get_number(arg1, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_is_valid(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_is_valid");
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            let ret = nvim_win_is_valid(arg1);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_hide(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg1: Window;
        'exit_0: {
            if lua_gettop(lstate) != 1 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 argument",
                );
                break 'exit_0;
            }
            if !nlua_is_deferred_safe() {
                return nlua_fast_api_disabled_error(lstate, c"nvim_win_hide");
            }
            if text_locked() {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeException,
                    get_text_locked_msg(),
                );
                break 'exit_0;
            }
            arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
            if err.err_type != ErrorType::ErrorTypeNone {
                err_param = Some(c"win");
                break 'exit_0;
            }
            let save_active_lstate = nlua_st().active_lstate;
            nlua_st().active_lstate = lstate;
            nvim_win_hide(arg1, &mut err);
            nlua_st().active_lstate = save_active_lstate;
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_close(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Boolean;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_close");
                }
                if text_locked() {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeException,
                        get_text_locked_msg(),
                    );
                    break 'exit_0;
                }
                arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"force");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_close(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_call(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: LuaRef;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_call");
                }
                arg2 = nlua_pop_lua_ref(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"fn");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_win_call(arg1, arg2, lstate, &mut err);
                let _ = ret;
                nlua_st().active_lstate = save_active_lstate;
            }
            api_free_luaref(arg2);
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_set_hl_ns(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let arg2: Integer;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) != 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_set_hl_ns");
                }
                arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"ns_id");
                    break 'exit_0;
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                nvim_win_set_hl_ns(arg1, arg2, &mut err);
                nlua_st().active_lstate = save_active_lstate;
            }
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_text_height(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg2: KeyDict_win_text_height = Default::default();
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                if lua_gettop(lstate) < 1 || lua_gettop(lstate) > 2 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 1 to 2 arguments",
                    );
                    break 'exit_0;
                }
                if !nlua_is_deferred_safe() {
                    return nlua_fast_api_disabled_error(lstate, c"nvim_win_text_height");
                }
                if lua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        (&raw mut arg2).cast(),
                        Some(key_dict_win_text_height_get_field),
                        &mut err_param,
                        &mut arena,
                        &mut err,
                    );
                    if err.err_type != ErrorType::ErrorTypeNone {
                        break 'exit_1;
                    }
                }
                arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    err_param = Some(c"win");
                    break 'exit_1;
                }
                let save_active_lstate = nlua_st().active_lstate;
                nlua_st().active_lstate = lstate;
                let ret = nvim_win_text_height(arg1, &mut arg2, &mut arena, &mut err);
                nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                nlua_st().active_lstate = save_active_lstate;
            }
            api_luarefs_free_keydict(
                (&raw mut arg2).cast(),
                (&raw const WIN_TEXT_HEIGHT_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
    }
}

unsafe extern "C-unwind" fn nlua_api_nvim_win_resize(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let mut arena: Arena = Default::default();
        let mut err_param: Option<&'static std::ffi::CStr> = None;
        let mut arg4: KeyDict_win_resize = Default::default();
        let arg3: Integer;
        let arg2: Integer;
        let arg1: Window;
        'exit_0: {
            'exit_1: {
                'exit_2: {
                    'exit_3: {
                        if lua_gettop(lstate) < 3 || lua_gettop(lstate) > 4 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 3 to 4 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_win_resize");
                        }
                        if lua_gettop(lstate) == 4 {
                            nlua_pop_keydict(
                                lstate,
                                (&raw mut arg4).cast(),
                                Some(key_dict_win_resize_get_field),
                                &mut err_param,
                                &mut arena,
                                &mut err,
                            );
                            if err.err_type != ErrorType::ErrorTypeNone {
                                break 'exit_1;
                            }
                        }
                        arg3 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"height");
                            break 'exit_1;
                        }
                        arg2 = nlua_pop_integer(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"width");
                            break 'exit_2;
                        }
                        arg1 = nlua_pop_Window(lstate, &mut arena, &mut err);
                        if err.err_type != ErrorType::ErrorTypeNone {
                            err_param = Some(c"win");
                            break 'exit_3;
                        }
                        let save_active_lstate = nlua_st().active_lstate;
                        nlua_st().active_lstate = lstate;
                        nvim_win_resize(arg1, arg2, arg3, &mut arg4, &mut err);
                        nlua_st().active_lstate = save_active_lstate;
                    }
                }
            }
            api_luarefs_free_keydict(
                (&raw mut arg4).cast(),
                (&raw const WIN_RESIZE_TABLE).cast_mut().cast(),
            );
        }
        nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
    }
}

pub unsafe extern "C-unwind" fn nlua_add_api_functions(lstate: *mut lua_State) {
    unsafe {
        lua_createtable(lstate, 0, 185);
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_autocmds), 0);
        lua_setfield(lstate, -2, c"nvim_get_autocmds");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_create_autocmd), 0);
        lua_setfield(lstate, -2, c"nvim_create_autocmd");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_autocmd), 0);
        lua_setfield(lstate, -2, c"nvim_del_autocmd");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_clear_autocmds), 0);
        lua_setfield(lstate, -2, c"nvim_clear_autocmds");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_create_augroup), 0);
        lua_setfield(lstate, -2, c"nvim_create_augroup");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_augroup_by_id), 0);
        lua_setfield(lstate, -2, c"nvim_del_augroup_by_id");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_augroup_by_name), 0);
        lua_setfield(lstate, -2, c"nvim_del_augroup_by_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_exec_autocmds), 0);
        lua_setfield(lstate, -2, c"nvim_exec_autocmds");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_line_count), 0);
        lua_setfield(lstate, -2, c"nvim_buf_line_count");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_attach), 0);
        lua_setfield(lstate, -2, c"nvim_buf_attach");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_lines), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_lines");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_lines), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_lines");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_text), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_text");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_text), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_text");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_offset), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_offset");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_var), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_changedtick), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_changedtick");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_del_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_buf_del_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_var), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_del_var), 0);
        lua_setfield(lstate, -2, c"nvim_buf_del_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_name), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_name), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_is_loaded), 0);
        lua_setfield(lstate, -2, c"nvim_buf_is_loaded");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_delete), 0);
        lua_setfield(lstate, -2, c"nvim_buf_delete");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_is_valid), 0);
        lua_setfield(lstate, -2, c"nvim_buf_is_valid");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_del_mark), 0);
        lua_setfield(lstate, -2, c"nvim_buf_del_mark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_mark), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_mark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_mark), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_mark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_call), 0);
        lua_setfield(lstate, -2, c"nvim_buf_call");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_stats), 0);
        lua_setfield(lstate, -2, c"nvim__buf_stats");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_parse_cmd), 0);
        lua_setfield(lstate, -2, c"nvim_parse_cmd");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_cmd), 0);
        lua_setfield(lstate, -2, c"nvim_cmd");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_create_user_command), 0);
        lua_setfield(lstate, -2, c"nvim_create_user_command");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_user_command), 0);
        lua_setfield(lstate, -2, c"nvim_del_user_command");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_create_user_command), 0);
        lua_setfield(lstate, -2, c"nvim_buf_create_user_command");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_del_user_command), 0);
        lua_setfield(lstate, -2, c"nvim_buf_del_user_command");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_commands), 0);
        lua_setfield(lstate, -2, c"nvim_get_commands");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_commands), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_commands");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_exec), 0);
        lua_setfield(lstate, -2, c"nvim_exec");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_command_output), 0);
        lua_setfield(lstate, -2, c"nvim_command_output");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_number), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_number");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_clear_highlight), 0);
        lua_setfield(lstate, -2, c"nvim_buf_clear_highlight");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_add_highlight), 0);
        lua_setfield(lstate, -2, c"nvim_buf_add_highlight");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_virtual_text), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_virtual_text");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_hl_by_id), 0);
        lua_setfield(lstate, -2, c"nvim_get_hl_by_id");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_hl_by_name), 0);
        lua_setfield(lstate, -2, c"nvim_get_hl_by_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_option_info), 0);
        lua_setfield(lstate, -2, c"nvim_get_option_info");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_option), 0);
        lua_setfield(lstate, -2, c"nvim_set_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_option), 0);
        lua_setfield(lstate, -2, c"nvim_get_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_option), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_option), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_option), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_option), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_option");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_out_write), 0);
        lua_setfield(lstate, -2, c"nvim_out_write");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_err_write), 0);
        lua_setfield(lstate, -2, c"nvim_err_write");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_err_writeln), 0);
        lua_setfield(lstate, -2, c"nvim_err_writeln");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_notify), 0);
        lua_setfield(lstate, -2, c"nvim_notify");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_height), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_height");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_width), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_width");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_create_namespace), 0);
        lua_setfield(lstate, -2, c"nvim_create_namespace");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_namespaces), 0);
        lua_setfield(lstate, -2, c"nvim_get_namespaces");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_extmark_by_id), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_extmark_by_id");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_get_extmarks), 0);
        lua_setfield(lstate, -2, c"nvim_buf_get_extmarks");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_set_extmark), 0);
        lua_setfield(lstate, -2, c"nvim_buf_set_extmark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_del_extmark), 0);
        lua_setfield(lstate, -2, c"nvim_buf_del_extmark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_clear_namespace), 0);
        lua_setfield(lstate, -2, c"nvim_buf_clear_namespace");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_decoration_provider), 0);
        lua_setfield(lstate, -2, c"nvim_set_decoration_provider");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_buf_debug_extmarks), 0);
        lua_setfield(lstate, -2, c"nvim__buf_debug_extmarks");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_ns_set), 0);
        lua_setfield(lstate, -2, c"nvim__ns_set");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_ns_get), 0);
        lua_setfield(lstate, -2, c"nvim__ns_get");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_option_value), 0);
        lua_setfield(lstate, -2, c"nvim_get_option_value");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_option_to_object), 0);
        lua_setfield(lstate, -2, c"nvim__option_to_object");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_option_value), 0);
        lua_setfield(lstate, -2, c"nvim_set_option_value");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_all_options_info), 0);
        lua_setfield(lstate, -2, c"nvim_get_all_options_info");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_option_info2), 0);
        lua_setfield(lstate, -2, c"nvim_get_option_info2");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_list_wins), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_list_wins");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_get_var), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_get_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_set_var), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_set_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_del_var), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_del_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_get_win), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_get_win");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_set_win), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_set_win");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_get_number), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_get_number");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_tabpage_is_valid), 0);
        lua_setfield(lstate, -2, c"nvim_tabpage_is_valid");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_open_tabpage), 0);
        lua_setfield(lstate, -2, c"nvim_open_tabpage");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_ui_send), 0);
        lua_setfield(lstate, -2, c"nvim_ui_send");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_hl_id_by_name), 0);
        lua_setfield(lstate, -2, c"nvim_get_hl_id_by_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_hl), 0);
        lua_setfield(lstate, -2, c"nvim_get_hl");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_hl), 0);
        lua_setfield(lstate, -2, c"nvim_set_hl");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_hl_ns), 0);
        lua_setfield(lstate, -2, c"nvim_get_hl_ns");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_hl_ns), 0);
        lua_setfield(lstate, -2, c"nvim_set_hl_ns");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_hl_ns_fast), 0);
        lua_setfield(lstate, -2, c"nvim_set_hl_ns_fast");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_feedkeys), 0);
        lua_setfield(lstate, -2, c"nvim_feedkeys");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_input), 0);
        lua_setfield(lstate, -2, c"nvim_input");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_input_mouse), 0);
        lua_setfield(lstate, -2, c"nvim_input_mouse");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_replace_termcodes), 0);
        lua_setfield(lstate, -2, c"nvim_replace_termcodes");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_strwidth), 0);
        lua_setfield(lstate, -2, c"nvim_strwidth");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_runtime_paths), 0);
        lua_setfield(lstate, -2, c"nvim_list_runtime_paths");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_runtime_inspect), 0);
        lua_setfield(lstate, -2, c"nvim__runtime_inspect");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_runtime_file), 0);
        lua_setfield(lstate, -2, c"nvim_get_runtime_file");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_lib_dir), 0);
        lua_setfield(lstate, -2, c"nvim__get_lib_dir");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_runtime), 0);
        lua_setfield(lstate, -2, c"nvim__get_runtime");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_current_dir), 0);
        lua_setfield(lstate, -2, c"nvim_set_current_dir");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_current_line), 0);
        lua_setfield(lstate, -2, c"nvim_get_current_line");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_current_line), 0);
        lua_setfield(lstate, -2, c"nvim_set_current_line");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_current_line), 0);
        lua_setfield(lstate, -2, c"nvim_del_current_line");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_var), 0);
        lua_setfield(lstate, -2, c"nvim_get_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_var), 0);
        lua_setfield(lstate, -2, c"nvim_set_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_var), 0);
        lua_setfield(lstate, -2, c"nvim_del_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_vvar), 0);
        lua_setfield(lstate, -2, c"nvim_get_vvar");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_vvar), 0);
        lua_setfield(lstate, -2, c"nvim_set_vvar");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_echo), 0);
        lua_setfield(lstate, -2, c"nvim_echo");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_bufs), 0);
        lua_setfield(lstate, -2, c"nvim_list_bufs");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_current_buf), 0);
        lua_setfield(lstate, -2, c"nvim_get_current_buf");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_current_buf), 0);
        lua_setfield(lstate, -2, c"nvim_set_current_buf");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_wins), 0);
        lua_setfield(lstate, -2, c"nvim_list_wins");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_current_win), 0);
        lua_setfield(lstate, -2, c"nvim_get_current_win");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_current_win), 0);
        lua_setfield(lstate, -2, c"nvim_set_current_win");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_create_buf), 0);
        lua_setfield(lstate, -2, c"nvim_create_buf");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_open_term), 0);
        lua_setfield(lstate, -2, c"nvim_open_term");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_chan_send), 0);
        lua_setfield(lstate, -2, c"nvim_chan_send");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_tabpages), 0);
        lua_setfield(lstate, -2, c"nvim_list_tabpages");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_current_tabpage), 0);
        lua_setfield(lstate, -2, c"nvim_get_current_tabpage");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_current_tabpage), 0);
        lua_setfield(lstate, -2, c"nvim_set_current_tabpage");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_paste), 0);
        lua_setfield(lstate, -2, c"nvim_paste");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_put), 0);
        lua_setfield(lstate, -2, c"nvim_put");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_color_by_name), 0);
        lua_setfield(lstate, -2, c"nvim_get_color_by_name");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_color_map), 0);
        lua_setfield(lstate, -2, c"nvim_get_color_map");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_context), 0);
        lua_setfield(lstate, -2, c"nvim_get_context");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_load_context), 0);
        lua_setfield(lstate, -2, c"nvim_load_context");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_mode), 0);
        lua_setfield(lstate, -2, c"nvim_get_mode");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_get_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_set_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_keymap), 0);
        lua_setfield(lstate, -2, c"nvim_del_keymap");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_chan_set), 0);
        lua_setfield(lstate, -2, c"nvim_chan_set");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_cmdwin_set), 0);
        lua_setfield(lstate, -2, c"nvim__cmdwin_set");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_chan_info), 0);
        lua_setfield(lstate, -2, c"nvim_get_chan_info");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_chans), 0);
        lua_setfield(lstate, -2, c"nvim_list_chans");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_id), 0);
        lua_setfield(lstate, -2, c"nvim__id");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_id_array), 0);
        lua_setfield(lstate, -2, c"nvim__id_array");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_id_dict), 0);
        lua_setfield(lstate, -2, c"nvim__id_dict");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_id_float), 0);
        lua_setfield(lstate, -2, c"nvim__id_float");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_stats), 0);
        lua_setfield(lstate, -2, c"nvim__stats");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_list_uis), 0);
        lua_setfield(lstate, -2, c"nvim_list_uis");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_proc_children), 0);
        lua_setfield(lstate, -2, c"nvim_get_proc_children");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_proc), 0);
        lua_setfield(lstate, -2, c"nvim_get_proc");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_select_popupmenu_item), 0);
        lua_setfield(lstate, -2, c"nvim_select_popupmenu_item");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_inspect_cell), 0);
        lua_setfield(lstate, -2, c"nvim__inspect_cell");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_screenshot), 0);
        lua_setfield(lstate, -2, c"nvim__screenshot");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_invalidate_glyph_cache), 0);
        lua_setfield(lstate, -2, c"nvim__invalidate_glyph_cache");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_unpack), 0);
        lua_setfield(lstate, -2, c"nvim__unpack");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_del_mark), 0);
        lua_setfield(lstate, -2, c"nvim_del_mark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_get_mark), 0);
        lua_setfield(lstate, -2, c"nvim_get_mark");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_eval_statusline), 0);
        lua_setfield(lstate, -2, c"nvim_eval_statusline");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_complete_set), 0);
        lua_setfield(lstate, -2, c"nvim__complete_set");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_redraw), 0);
        lua_setfield(lstate, -2, c"nvim__redraw");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_set_restart_on_crash), 0);
        lua_setfield(lstate, -2, c"nvim__set_restart_on_crash");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_exec2), 0);
        lua_setfield(lstate, -2, c"nvim_exec2");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_command), 0);
        lua_setfield(lstate, -2, c"nvim_command");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_eval), 0);
        lua_setfield(lstate, -2, c"nvim_eval");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_call_function), 0);
        lua_setfield(lstate, -2, c"nvim_call_function");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_call_dict_function), 0);
        lua_setfield(lstate, -2, c"nvim_call_dict_function");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_parse_expression), 0);
        lua_setfield(lstate, -2, c"nvim_parse_expression");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_open_win), 0);
        lua_setfield(lstate, -2, c"nvim_open_win");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_config), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_config");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_config), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_config");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_buf), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_buf");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_buf), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_buf");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_cursor), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_cursor");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_cursor), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_cursor");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_height), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_height");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_width), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_width");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_var), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_var), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_del_var), 0);
        lua_setfield(lstate, -2, c"nvim_win_del_var");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_position), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_position");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_tabpage), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_tabpage");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_get_number), 0);
        lua_setfield(lstate, -2, c"nvim_win_get_number");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_is_valid), 0);
        lua_setfield(lstate, -2, c"nvim_win_is_valid");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_hide), 0);
        lua_setfield(lstate, -2, c"nvim_win_hide");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_close), 0);
        lua_setfield(lstate, -2, c"nvim_win_close");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_call), 0);
        lua_setfield(lstate, -2, c"nvim_win_call");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_set_hl_ns), 0);
        lua_setfield(lstate, -2, c"nvim_win_set_hl_ns");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_text_height), 0);
        lua_setfield(lstate, -2, c"nvim_win_text_height");
        lua_pushcclosure(lstate, Some(nlua_api_nvim_win_resize), 0);
        lua_setfield(lstate, -2, c"nvim_win_resize");
        lua_setfield(lstate, -2, c"api");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_handler_lookup_resolves_all_methods() {
        for (idx, handler) in METHOD_HANDLERS.iter().enumerate() {
            let name = handler.name.to_bytes();
            assert_eq!(msgpack_rpc_get_handler_for_hash(name), idx as i32);
            let got = msgpack_rpc_get_handler_at(idx as i32);
            assert_eq!(got.name, handler.name);
            assert_eq!(got.fast, handler.fast);
            assert_eq!(got.ret_alloc, handler.ret_alloc);
        }
        assert_eq!(msgpack_rpc_get_handler_for_hash(b"not_a_method"), -1);
        assert_eq!(msgpack_rpc_get_handler_for_hash(b""), -1);
    }

    #[test]
    fn keyset_lookup_resolves_known_and_unknown_fields() {
        let link = key_dict_get_autocmds_get_field(b"event").unwrap();
        assert_eq!(link.str.unwrap().to_bytes(), b"event");
        assert!(key_dict_get_autocmds_get_field(b"nonexistent").is_none());
    }
}
