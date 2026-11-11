#![forbid(unsafe_code)]

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
    nvim_buf_get_extmark_by_id, nvim_buf_get_extmarks, nvim_buf_set_conceal_provider,
    nvim_buf_set_extmark, nvim_create_namespace, nvim_get_namespaces, nvim_ns_get, nvim_ns_set,
    nvim_set_decoration_provider,
};
use crate::api::options::{
    KeyDict_option, nvim_get_all_options_info, nvim_get_option_info2, nvim_get_option_value,
    nvim_option_to_object, nvim_set_option_value,
};
use crate::api::private::helpers::{
    OBJECT_NIL, api_dict_to_keydict, api_free_dict, api_free_object, api_free_string,
    api_keydict_to_dict, api_luarefs_free_keydict, api_luarefs_free_object, api_set_error,
    api_set_error_str, array_obj, boolean_obj, buffer_obj, dict_obj, float_obj, integer_obj,
    string_obj, tabpage_obj, window_obj,
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
    Boolean, FieldHashfn, Float, KeyDict, KeyField, KeySetLink, LuaRef, nlua_pop_array,
    nlua_pop_boolean, nlua_pop_dict, nlua_pop_float, nlua_pop_handle as nlua_pop_Buffer,
    nlua_pop_handle as nlua_pop_Tabpage, nlua_pop_handle as nlua_pop_Window, nlua_pop_integer,
    nlua_pop_keydict, nlua_pop_lua_ref, nlua_pop_object, nlua_pop_string, nlua_push_array,
    nlua_push_boolean, nlua_push_dict, nlua_push_float, nlua_push_handle as nlua_push_Buffer,
    nlua_push_handle as nlua_push_Tabpage, nlua_push_handle as nlua_push_Window, nlua_push_integer,
    nlua_push_keydict, nlua_push_object, nlua_push_string,
};
use crate::lua::xdiff::KeyDict_xdl_diff;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::shada::{
    KeyDict__shada_buflist_item, KeyDict__shada_mark, KeyDict__shada_register,
    KeyDict__shada_search_pat,
};
use crate::ui_client::handle_ui_client_redraw;

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
            let _ =
                crate::log::logmsg_str(crate::log::LOGLVL_DBG, None, Some(func_name), 0, true, msg);
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (channel_id, func_name.as_ptr(), msg.as_ptr());
}

#[inline]
fn extract_keydict<T: Default + KeyDict>(
    item: Object,
    out: &mut T,
    get_field: FieldHashfn<T>,
    wrong_type_msg: &std::ffi::CStr,
    error: &mut Error,
) -> bool {
    if item.obj_type == ObjectType::ObjectTypeDict {
        *out = T::default();
        api_dict_to_keydict(out, get_field, item.data.dict(), error)
    } else if item.obj_type == ObjectType::ObjectTypeArray && item.data.array().size == 0 {
        *out = T::default();
        true
    } else {
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        *out = Dict::new();
        true
    } else {
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
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
        api_set_error_str(error, ErrorType::ErrorTypeException, wrong_type_msg);
        false
    }
}

macro_rules! impl_keydict {
    ($ty:ty, $field:ident) => {
        impl KeyDict for $ty {
            #[inline]
            fn is_set_mask(&self) -> u64 {
                self.$field
            }
            #[inline]
            fn mark_set_bit(&mut self, bit: u32) {
                self.$field |= 1u64 << bit;
            }
        }
    };
    ($ty:ty) => {
        impl KeyDict for $ty {
            #[inline]
            fn is_set_mask(&self) -> u64 {
                u64::MAX
            }
            #[inline]
            fn mark_set_bit(&mut self, _bit: u32) {}
        }
    };
}
impl_keydict!(KeyDict__shada_buflist_item, is_set_shada_buflist_item);
impl_keydict!(KeyDict__shada_mark, is_set_shada_mark);
impl_keydict!(KeyDict__shada_register, is_set_shada_register);
impl_keydict!(KeyDict__shada_search_pat, is_set_shada_search_pat);
impl_keydict!(KeyDict_buf_attach, is_set_buf_attach);
impl_keydict!(KeyDict_buf_delete, is_set_buf_delete);
impl_keydict!(KeyDict_chan_set, is_set_chan_set);
impl_keydict!(KeyDict_clear_autocmds, is_set_clear_autocmds);
impl_keydict!(KeyDict_cmd, is_set_cmd);
impl_keydict!(KeyDict_cmd_magic, is_set_cmd_magic);
impl_keydict!(KeyDict_cmd_mods, is_set_cmd_mods);
impl_keydict!(KeyDict_cmd_mods_filter, is_set_cmd_mods_filter);
impl_keydict!(KeyDict_cmd_opts);
impl_keydict!(KeyDict_complete_set, is_set_complete_set);
impl_keydict!(KeyDict_context, is_set_context);
impl_keydict!(KeyDict_create_augroup, is_set_create_augroup);
impl_keydict!(KeyDict_create_autocmd, is_set_create_autocmd);
impl_keydict!(KeyDict_echo_opts, is_set_echo_opts);
impl_keydict!(KeyDict_eval_statusline, is_set_eval_statusline);
impl_keydict!(KeyDict_exec_autocmds, is_set_exec_autocmds);
impl_keydict!(KeyDict_exec_opts);
impl_keydict!(KeyDict_get_autocmds, is_set_get_autocmds);
impl_keydict!(KeyDict_get_commands);
impl_keydict!(KeyDict_get_extmark, is_set_get_extmark);
impl_keydict!(KeyDict_get_extmarks, is_set_get_extmarks);
impl_keydict!(KeyDict_get_highlight, is_set_get_highlight);
impl_keydict!(KeyDict_get_ns, is_set_get_ns);
impl_keydict!(KeyDict_highlight, is_set_highlight);
impl_keydict!(KeyDict_highlight_cterm);
impl_keydict!(KeyDict_keymap, is_set_keymap);
impl_keydict!(KeyDict_keymap_del, is_set_keymap_del);
impl_keydict!(KeyDict_ns_opts, is_set_ns_opts);
impl_keydict!(KeyDict_open_term, is_set_open_term);
impl_keydict!(KeyDict_option, is_set_option);
impl_keydict!(KeyDict_redraw, is_set_redraw);
impl_keydict!(KeyDict_runtime);
impl_keydict!(
    KeyDict_set_decoration_provider,
    is_set_set_decoration_provider
);
impl_keydict!(KeyDict_set_extmark, is_set_set_extmark);
impl_keydict!(KeyDict_tabpage_config, is_set_tabpage_config);
impl_keydict!(KeyDict_user_command, is_set_user_command);
impl_keydict!(KeyDict_win_config, is_set_win_config);
impl_keydict!(KeyDict_win_resize, is_set_win_resize);
impl_keydict!(KeyDict_win_text_height, is_set_win_text_height);
impl_keydict!(KeyDict_xdl_diff, is_set_xdl_diff);
impl_keydict!(KeyDict_empty, is_set_empty);

pub static _SHADA_BUFLIST_ITEM_TABLE: [KeySetLink<KeyDict__shada_buflist_item>; 4] = [
    KeySetLink {
        str: Some(c"c"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_buflist_item| &d.c,
            |d: &mut KeyDict__shada_buflist_item| &mut d.c,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"f"),
        field: KeyField::String(
            |d: &KeyDict__shada_buflist_item| &d.f,
            |d: &mut KeyDict__shada_buflist_item| &mut d.f,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"l"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_buflist_item| &d.l,
            |d: &mut KeyDict__shada_buflist_item| &mut d.l,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_shada_buflist_item_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict__shada_buflist_item>> {
    let hash = _shada_buflist_item_hash(key);
    (hash >= 0).then(|| &_SHADA_BUFLIST_ITEM_TABLE[hash as usize])
}

pub static _SHADA_MARK_TABLE: [KeySetLink<KeyDict__shada_mark>; 5] = [
    KeySetLink {
        str: Some(c"c"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_mark| &d.c,
            |d: &mut KeyDict__shada_mark| &mut d.c,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"f"),
        field: KeyField::String(
            |d: &KeyDict__shada_mark| &d.f,
            |d: &mut KeyDict__shada_mark| &mut d.f,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"l"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_mark| &d.l,
            |d: &mut KeyDict__shada_mark| &mut d.l,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"n"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_mark| &d.n,
            |d: &mut KeyDict__shada_mark| &mut d.n,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_shada_mark_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict__shada_mark>> {
    let hash = _shada_mark_hash(key);
    (hash >= 0).then(|| &_SHADA_MARK_TABLE[hash as usize])
}

pub static _SHADA_REGISTER_TABLE: [KeySetLink<KeyDict__shada_register>; 6] = [
    KeySetLink {
        str: Some(c"n"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_register| &d.n,
            |d: &mut KeyDict__shada_register| &mut d.n,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rc"),
        field: KeyField::StringArray(
            |d: &KeyDict__shada_register| &d.rc,
            |d: &mut KeyDict__shada_register| &mut d.rc,
        ),
        key_type: K_UNPACK_TYPE_STRING_ARRAY_CONST,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rt"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_register| &d.rt,
            |d: &mut KeyDict__shada_register| &mut d.rt,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ru"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_register| &d.ru,
            |d: &mut KeyDict__shada_register| &mut d.ru,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"rw"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_register| &d.rw,
            |d: &mut KeyDict__shada_register| &mut d.rw,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_shada_register_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict__shada_register>> {
    let hash = _shada_register_hash(key);
    (hash >= 0).then(|| &_SHADA_REGISTER_TABLE[hash as usize])
}

pub static _SHADA_SEARCH_PAT_TABLE: [KeySetLink<KeyDict__shada_search_pat>; 11] = [
    KeySetLink {
        str: Some(c"sb"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.search_backward,
            |d: &mut KeyDict__shada_search_pat| &mut d.search_backward,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sc"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.smartcase,
            |d: &mut KeyDict__shada_search_pat| &mut d.smartcase,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"se"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.place_cursor_at_end,
            |d: &mut KeyDict__shada_search_pat| &mut d.place_cursor_at_end,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sh"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.highlighted,
            |d: &mut KeyDict__shada_search_pat| &mut d.highlighted,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sl"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.has_line_offset,
            |d: &mut KeyDict__shada_search_pat| &mut d.has_line_offset,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sm"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.magic,
            |d: &mut KeyDict__shada_search_pat| &mut d.magic,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"so"),
        field: KeyField::Integer(
            |d: &KeyDict__shada_search_pat| &d.offset,
            |d: &mut KeyDict__shada_search_pat| &mut d.offset,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sp"),
        field: KeyField::String(
            |d: &KeyDict__shada_search_pat| &d.pat,
            |d: &mut KeyDict__shada_search_pat| &mut d.pat,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ss"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.is_substitute_pattern,
            |d: &mut KeyDict__shada_search_pat| &mut d.is_substitute_pattern,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"su"),
        field: KeyField::Boolean(
            |d: &KeyDict__shada_search_pat| &d.is_last_used,
            |d: &mut KeyDict__shada_search_pat| &mut d.is_last_used,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_shada_search_pat_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict__shada_search_pat>> {
    let hash = _shada_search_pat_hash(key);
    (hash >= 0).then(|| &_SHADA_SEARCH_PAT_TABLE[hash as usize])
}

pub static BUF_ATTACH_TABLE: [KeySetLink<KeyDict_buf_attach>; 8] = [
    KeySetLink {
        str: Some(c"preview"),
        field: KeyField::Boolean(
            |d: &KeyDict_buf_attach| &d.preview,
            |d: &mut KeyDict_buf_attach| &mut d.preview,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_bytes"),
        field: KeyField::I32(
            |d: &KeyDict_buf_attach| &d.on_bytes,
            |d: &mut KeyDict_buf_attach| &mut d.on_bytes,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_lines"),
        field: KeyField::I32(
            |d: &KeyDict_buf_attach| &d.on_lines,
            |d: &mut KeyDict_buf_attach| &mut d.on_lines,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"utf_sizes"),
        field: KeyField::Boolean(
            |d: &KeyDict_buf_attach| &d.utf_sizes,
            |d: &mut KeyDict_buf_attach| &mut d.utf_sizes,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_detach"),
        field: KeyField::I32(
            |d: &KeyDict_buf_attach| &d.on_detach,
            |d: &mut KeyDict_buf_attach| &mut d.on_detach,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_reload"),
        field: KeyField::I32(
            |d: &KeyDict_buf_attach| &d.on_reload,
            |d: &mut KeyDict_buf_attach| &mut d.on_reload,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_changedtick"),
        field: KeyField::I32(
            |d: &KeyDict_buf_attach| &d.on_changedtick,
            |d: &mut KeyDict_buf_attach| &mut d.on_changedtick,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_buf_attach_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_buf_attach>> {
    let hash = buf_attach_hash(key);
    (hash >= 0).then(|| &BUF_ATTACH_TABLE[hash as usize])
}

pub static BUF_DELETE_TABLE: [KeySetLink<KeyDict_buf_delete>; 3] = [
    KeySetLink {
        str: Some(c"force"),
        field: KeyField::Boolean(
            |d: &KeyDict_buf_delete| &d.force,
            |d: &mut KeyDict_buf_delete| &mut d.force,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unload"),
        field: KeyField::Boolean(
            |d: &KeyDict_buf_delete| &d.unload,
            |d: &mut KeyDict_buf_delete| &mut d.unload,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_buf_delete_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_buf_delete>> {
    let hash = buf_delete_hash(key);
    (hash >= 0).then(|| &BUF_DELETE_TABLE[hash as usize])
}

pub static CHAN_SET_TABLE: [KeySetLink<KeyDict_chan_set>; 4] = [
    KeySetLink {
        str: Some(c"addr"),
        field: KeyField::String(
            |d: &KeyDict_chan_set| &d.addr,
            |d: &mut KeyDict_chan_set| &mut d.addr,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"detach"),
        field: KeyField::Boolean(
            |d: &KeyDict_chan_set| &d.detach,
            |d: &mut KeyDict_chan_set| &mut d.detach,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"trust"),
        field: KeyField::Boolean(
            |d: &KeyDict_chan_set| &d.trust,
            |d: &mut KeyDict_chan_set| &mut d.trust,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

pub fn chan_set_hash(key: &[u8]) -> i32 {
    match key {
        b"addr" => 0,
        b"detach" => 1,
        b"trust" => 2,
        _ => -1,
    }
}

pub fn key_dict_chan_set_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_chan_set>> {
    let hash = chan_set_hash(key);
    (hash >= 0).then(|| &CHAN_SET_TABLE[hash as usize])
}

pub static CLEAR_AUTOCMDS_TABLE: [KeySetLink<KeyDict_clear_autocmds>; 6] = [
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::I32(
            |d: &KeyDict_clear_autocmds| &d.buf,
            |d: &mut KeyDict_clear_autocmds| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"event"),
        field: KeyField::Object(
            |d: &KeyDict_clear_autocmds| &d.event,
            |d: &mut KeyDict_clear_autocmds| &mut d.event,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        field: KeyField::Object(
            |d: &KeyDict_clear_autocmds| &d.group,
            |d: &mut KeyDict_clear_autocmds| &mut d.group,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        field: KeyField::I32(
            |d: &KeyDict_clear_autocmds| &d.buffer,
            |d: &mut KeyDict_clear_autocmds| &mut d.buffer,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        field: KeyField::Object(
            |d: &KeyDict_clear_autocmds| &d.pattern,
            |d: &mut KeyDict_clear_autocmds| &mut d.pattern,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_clear_autocmds_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_clear_autocmds>> {
    let hash = clear_autocmds_hash(key);
    (hash >= 0).then(|| &CLEAR_AUTOCMDS_TABLE[hash as usize])
}

pub static CMD_TABLE: [KeySetLink<KeyDict_cmd>; 12] = [
    KeySetLink {
        str: Some(c"cmd"),
        field: KeyField::String(|d: &KeyDict_cmd| &d.cmd, |d: &mut KeyDict_cmd| &mut d.cmd),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reg"),
        field: KeyField::String(|d: &KeyDict_cmd| &d.reg, |d: &mut KeyDict_cmd| &mut d.reg),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bang"),
        field: KeyField::Boolean(|d: &KeyDict_cmd| &d.bang, |d: &mut KeyDict_cmd| &mut d.bang),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"addr"),
        field: KeyField::String(|d: &KeyDict_cmd| &d.addr, |d: &mut KeyDict_cmd| &mut d.addr),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"mods"),
        field: KeyField::Dict(|d: &KeyDict_cmd| &d.mods, |d: &mut KeyDict_cmd| &mut d.mods),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"args"),
        field: KeyField::Array(|d: &KeyDict_cmd| &d.args, |d: &mut KeyDict_cmd| &mut d.args),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"count"),
        field: KeyField::Integer(
            |d: &KeyDict_cmd| &d.count,
            |d: &mut KeyDict_cmd| &mut d.count,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"magic"),
        field: KeyField::Dict(
            |d: &KeyDict_cmd| &d.magic,
            |d: &mut KeyDict_cmd| &mut d.magic,
        ),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nargs"),
        field: KeyField::Object(
            |d: &KeyDict_cmd| &d.nargs,
            |d: &mut KeyDict_cmd| &mut d.nargs,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        field: KeyField::Array(
            |d: &KeyDict_cmd| &d.range,
            |d: &mut KeyDict_cmd| &mut d.range,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nextcmd"),
        field: KeyField::String(
            |d: &KeyDict_cmd| &d.nextcmd,
            |d: &mut KeyDict_cmd| &mut d.nextcmd,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_cmd_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_cmd>> {
    let hash = cmd_hash(key);
    (hash >= 0).then(|| &CMD_TABLE[hash as usize])
}

pub static CMD_MAGIC_TABLE: [KeySetLink<KeyDict_cmd_magic>; 3] = [
    KeySetLink {
        str: Some(c"bar"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_magic| &d.bar,
            |d: &mut KeyDict_cmd_magic| &mut d.bar,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"file"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_magic| &d.file,
            |d: &mut KeyDict_cmd_magic| &mut d.file,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_cmd_magic_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_cmd_magic>> {
    let hash = cmd_magic_hash(key);
    (hash >= 0).then(|| &CMD_MAGIC_TABLE[hash as usize])
}

pub static CMD_MODS_TABLE: [KeySetLink<KeyDict_cmd_mods>; 21] = [
    KeySetLink {
        str: Some(c"tab"),
        field: KeyField::Integer(
            |d: &KeyDict_cmd_mods| &d.tab,
            |d: &mut KeyDict_cmd_mods| &mut d.tab,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hide"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.hide,
            |d: &mut KeyDict_cmd_mods| &mut d.hide,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"split"),
        field: KeyField::String(
            |d: &KeyDict_cmd_mods| &d.split,
            |d: &mut KeyDict_cmd_mods| &mut d.split,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"browse"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.browse,
            |d: &mut KeyDict_cmd_mods| &mut d.browse,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filter"),
        field: KeyField::Dict(
            |d: &KeyDict_cmd_mods| &d.filter,
            |d: &mut KeyDict_cmd_mods| &mut d.filter,
        ),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"silent"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.silent,
            |d: &mut KeyDict_cmd_mods| &mut d.silent,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"confirm"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.confirm,
            |d: &mut KeyDict_cmd_mods| &mut d.confirm,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepalt"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.keepalt,
            |d: &mut KeyDict_cmd_mods| &mut d.keepalt,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sandbox"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.sandbox,
            |d: &mut KeyDict_cmd_mods| &mut d.sandbox,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"verbose"),
        field: KeyField::Integer(
            |d: &KeyDict_cmd_mods| &d.verbose,
            |d: &mut KeyDict_cmd_mods| &mut d.verbose,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unsilent"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.unsilent,
            |d: &mut KeyDict_cmd_mods| &mut d.unsilent,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.vertical,
            |d: &mut KeyDict_cmd_mods| &mut d.vertical,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepjumps"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.keepjumps,
            |d: &mut KeyDict_cmd_mods| &mut d.keepjumps,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepmarks"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.keepmarks,
            |d: &mut KeyDict_cmd_mods| &mut d.keepmarks,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"lockmarks"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.lockmarks,
            |d: &mut KeyDict_cmd_mods| &mut d.lockmarks,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noautocmd"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.noautocmd,
            |d: &mut KeyDict_cmd_mods| &mut d.noautocmd,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"horizontal"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.horizontal,
            |d: &mut KeyDict_cmd_mods| &mut d.horizontal,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noswapfile"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.noswapfile,
            |d: &mut KeyDict_cmd_mods| &mut d.noswapfile,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"emsg_silent"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.emsg_silent,
            |d: &mut KeyDict_cmd_mods| &mut d.emsg_silent,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keeppatterns"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods| &d.keeppatterns,
            |d: &mut KeyDict_cmd_mods| &mut d.keeppatterns,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_cmd_mods_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_cmd_mods>> {
    let hash = cmd_mods_hash(key);
    (hash >= 0).then(|| &CMD_MODS_TABLE[hash as usize])
}

pub static CMD_MODS_FILTER_TABLE: [KeySetLink<KeyDict_cmd_mods_filter>; 3] = [
    KeySetLink {
        str: Some(c"force"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_mods_filter| &d.force,
            |d: &mut KeyDict_cmd_mods_filter| &mut d.force,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        field: KeyField::String(
            |d: &KeyDict_cmd_mods_filter| &d.pattern,
            |d: &mut KeyDict_cmd_mods_filter| &mut d.pattern,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_cmd_mods_filter_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_cmd_mods_filter>> {
    let hash = cmd_mods_filter_hash(key);
    (hash >= 0).then(|| &CMD_MODS_FILTER_TABLE[hash as usize])
}

pub static CMD_OPTS_TABLE: [KeySetLink<KeyDict_cmd_opts>; 2] = [
    KeySetLink {
        str: Some(c"output"),
        field: KeyField::Boolean(
            |d: &KeyDict_cmd_opts| &d.output,
            |d: &mut KeyDict_cmd_opts| &mut d.output,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_cmd_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_cmd_opts>> {
    let hash = cmd_opts_hash(key);
    (hash >= 0).then(|| &CMD_OPTS_TABLE[hash as usize])
}

pub static COMPLETE_SET_TABLE: [KeySetLink<KeyDict_complete_set>; 2] = [
    KeySetLink {
        str: Some(c"info"),
        field: KeyField::String(
            |d: &KeyDict_complete_set| &d.info,
            |d: &mut KeyDict_complete_set| &mut d.info,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_complete_set_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_complete_set>> {
    let hash = complete_set_hash(key);
    (hash >= 0).then(|| &COMPLETE_SET_TABLE[hash as usize])
}

pub static CONTEXT_TABLE: [KeySetLink<KeyDict_context>; 2] = [
    KeySetLink {
        str: Some(c"types"),
        field: KeyField::Array(
            |d: &KeyDict_context| &d.types,
            |d: &mut KeyDict_context| &mut d.types,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_context_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_context>> {
    let hash = context_hash(key);
    (hash >= 0).then(|| &CONTEXT_TABLE[hash as usize])
}

pub static CREATE_AUGROUP_TABLE: [KeySetLink<KeyDict_create_augroup>; 2] = [
    KeySetLink {
        str: Some(c"clear"),
        field: KeyField::Boolean(
            |d: &KeyDict_create_augroup| &d.clear,
            |d: &mut KeyDict_create_augroup| &mut d.clear,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_create_augroup_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_create_augroup>> {
    let hash = create_augroup_hash(key);
    (hash >= 0).then(|| &CREATE_AUGROUP_TABLE[hash as usize])
}

pub static CREATE_AUTOCMD_TABLE: [KeySetLink<KeyDict_create_autocmd>; 10] = [
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::I32(
            |d: &KeyDict_create_autocmd| &d.buf,
            |d: &mut KeyDict_create_autocmd| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"desc"),
        field: KeyField::String(
            |d: &KeyDict_create_autocmd| &d.desc,
            |d: &mut KeyDict_create_autocmd| &mut d.desc,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"once"),
        field: KeyField::Boolean(
            |d: &KeyDict_create_autocmd| &d.once,
            |d: &mut KeyDict_create_autocmd| &mut d.once,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        field: KeyField::Object(
            |d: &KeyDict_create_autocmd| &d.group,
            |d: &mut KeyDict_create_autocmd| &mut d.group,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        field: KeyField::I32(
            |d: &KeyDict_create_autocmd| &d.buffer,
            |d: &mut KeyDict_create_autocmd| &mut d.buffer,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nested"),
        field: KeyField::Boolean(
            |d: &KeyDict_create_autocmd| &d.nested,
            |d: &mut KeyDict_create_autocmd| &mut d.nested,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"command"),
        field: KeyField::String(
            |d: &KeyDict_create_autocmd| &d.command,
            |d: &mut KeyDict_create_autocmd| &mut d.command,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        field: KeyField::Object(
            |d: &KeyDict_create_autocmd| &d.pattern,
            |d: &mut KeyDict_create_autocmd| &mut d.pattern,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"callback"),
        field: KeyField::Object(
            |d: &KeyDict_create_autocmd| &d.callback,
            |d: &mut KeyDict_create_autocmd| &mut d.callback,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_create_autocmd_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_create_autocmd>> {
    let hash = create_autocmd_hash(key);
    (hash >= 0).then(|| &CREATE_AUTOCMD_TABLE[hash as usize])
}

pub static ECHO_OPTS_TABLE: [KeySetLink<KeyDict_echo_opts>; 11] = [
    KeySetLink {
        str: Some(c"id"),
        field: KeyField::Object(
            |d: &KeyDict_echo_opts| &d.id,
            |d: &mut KeyDict_echo_opts| &mut d.id,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"err"),
        field: KeyField::Boolean(
            |d: &KeyDict_echo_opts| &d.err,
            |d: &mut KeyDict_echo_opts| &mut d.err,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"data"),
        field: KeyField::Dict(
            |d: &KeyDict_echo_opts| &d.data,
            |d: &mut KeyDict_echo_opts| &mut d.data,
        ),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"kind"),
        field: KeyField::String(
            |d: &KeyDict_echo_opts| &d.kind,
            |d: &mut KeyDict_echo_opts| &mut d.kind,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title"),
        field: KeyField::String(
            |d: &KeyDict_echo_opts| &d.title,
            |d: &mut KeyDict_echo_opts| &mut d.title,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"source"),
        field: KeyField::String(
            |d: &KeyDict_echo_opts| &d.source,
            |d: &mut KeyDict_echo_opts| &mut d.source,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"status"),
        field: KeyField::String(
            |d: &KeyDict_echo_opts| &d.status,
            |d: &mut KeyDict_echo_opts| &mut d.status,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"percent"),
        field: KeyField::Integer(
            |d: &KeyDict_echo_opts| &d.percent,
            |d: &mut KeyDict_echo_opts| &mut d.percent,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"verbose"),
        field: KeyField::Boolean(
            |d: &KeyDict_echo_opts| &d.verbose,
            |d: &mut KeyDict_echo_opts| &mut d.verbose,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_truncate"),
        field: KeyField::Boolean(
            |d: &KeyDict_echo_opts| &d._truncate,
            |d: &mut KeyDict_echo_opts| &mut d._truncate,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_echo_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_echo_opts>> {
    let hash = echo_opts_hash(key);
    (hash >= 0).then(|| &ECHO_OPTS_TABLE[hash as usize])
}

pub static EMPTY_TABLE: [KeySetLink<KeyDict_empty>; 1] = [KeySetLink {
    str: None,
    field: KeyField::None,
    key_type: ObjectType::ObjectTypeNil as i32,
    opt_index: -1,
    is_hlgroup: false,
}];

pub fn empty_hash(key: &[u8]) -> i32 {
    let _ = key;
    -1
}

pub fn key_dict_empty_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_empty>> {
    let hash = empty_hash(key);
    (hash >= 0).then(|| &EMPTY_TABLE[hash as usize])
}

pub static EVAL_STATUSLINE_TABLE: [KeySetLink<KeyDict_eval_statusline>; 8] = [
    KeySetLink {
        str: Some(c"winid"),
        field: KeyField::I32(
            |d: &KeyDict_eval_statusline| &d.winid,
            |d: &mut KeyDict_eval_statusline| &mut d.winid,
        ),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fillchar"),
        field: KeyField::String(
            |d: &KeyDict_eval_statusline| &d.fillchar,
            |d: &mut KeyDict_eval_statusline| &mut d.fillchar,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"maxwidth"),
        field: KeyField::Integer(
            |d: &KeyDict_eval_statusline| &d.maxwidth,
            |d: &mut KeyDict_eval_statusline| &mut d.maxwidth,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"highlights"),
        field: KeyField::Boolean(
            |d: &KeyDict_eval_statusline| &d.highlights,
            |d: &mut KeyDict_eval_statusline| &mut d.highlights,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_winbar"),
        field: KeyField::Boolean(
            |d: &KeyDict_eval_statusline| &d.use_winbar,
            |d: &mut KeyDict_eval_statusline| &mut d.use_winbar,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_tabline"),
        field: KeyField::Boolean(
            |d: &KeyDict_eval_statusline| &d.use_tabline,
            |d: &mut KeyDict_eval_statusline| &mut d.use_tabline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"use_statuscol_lnum"),
        field: KeyField::Integer(
            |d: &KeyDict_eval_statusline| &d.use_statuscol_lnum,
            |d: &mut KeyDict_eval_statusline| &mut d.use_statuscol_lnum,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_eval_statusline_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_eval_statusline>> {
    let hash = eval_statusline_hash(key);
    (hash >= 0).then(|| &EVAL_STATUSLINE_TABLE[hash as usize])
}

pub static EXEC_AUTOCMDS_TABLE: [KeySetLink<KeyDict_exec_autocmds>; 7] = [
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::I32(
            |d: &KeyDict_exec_autocmds| &d.buf,
            |d: &mut KeyDict_exec_autocmds| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"data"),
        field: KeyField::Object(
            |d: &KeyDict_exec_autocmds| &d.data,
            |d: &mut KeyDict_exec_autocmds| &mut d.data,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        field: KeyField::Object(
            |d: &KeyDict_exec_autocmds| &d.group,
            |d: &mut KeyDict_exec_autocmds| &mut d.group,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        field: KeyField::I32(
            |d: &KeyDict_exec_autocmds| &d.buffer,
            |d: &mut KeyDict_exec_autocmds| &mut d.buffer,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        field: KeyField::Object(
            |d: &KeyDict_exec_autocmds| &d.pattern,
            |d: &mut KeyDict_exec_autocmds| &mut d.pattern,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"modeline"),
        field: KeyField::Boolean(
            |d: &KeyDict_exec_autocmds| &d.modeline,
            |d: &mut KeyDict_exec_autocmds| &mut d.modeline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_exec_autocmds_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_exec_autocmds>> {
    let hash = exec_autocmds_hash(key);
    (hash >= 0).then(|| &EXEC_AUTOCMDS_TABLE[hash as usize])
}

pub static EXEC_OPTS_TABLE: [KeySetLink<KeyDict_exec_opts>; 2] = [
    KeySetLink {
        str: Some(c"output"),
        field: KeyField::Boolean(
            |d: &KeyDict_exec_opts| &d.output,
            |d: &mut KeyDict_exec_opts| &mut d.output,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_exec_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_exec_opts>> {
    let hash = exec_opts_hash(key);
    (hash >= 0).then(|| &EXEC_OPTS_TABLE[hash as usize])
}

pub static GET_AUTOCMDS_TABLE: [KeySetLink<KeyDict_get_autocmds>; 7] = [
    KeySetLink {
        str: Some(c"id"),
        field: KeyField::Integer(
            |d: &KeyDict_get_autocmds| &d.id,
            |d: &mut KeyDict_get_autocmds| &mut d.id,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::Object(
            |d: &KeyDict_get_autocmds| &d.buf,
            |d: &mut KeyDict_get_autocmds| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"event"),
        field: KeyField::Object(
            |d: &KeyDict_get_autocmds| &d.event,
            |d: &mut KeyDict_get_autocmds| &mut d.event,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"group"),
        field: KeyField::Object(
            |d: &KeyDict_get_autocmds| &d.group,
            |d: &mut KeyDict_get_autocmds| &mut d.group,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"buffer"),
        field: KeyField::Object(
            |d: &KeyDict_get_autocmds| &d.buffer,
            |d: &mut KeyDict_get_autocmds| &mut d.buffer,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pattern"),
        field: KeyField::Object(
            |d: &KeyDict_get_autocmds| &d.pattern,
            |d: &mut KeyDict_get_autocmds| &mut d.pattern,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_autocmds_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_get_autocmds>> {
    let hash = get_autocmds_hash(key);
    (hash >= 0).then(|| &GET_AUTOCMDS_TABLE[hash as usize])
}

pub static GET_COMMANDS_TABLE: [KeySetLink<KeyDict_get_commands>; 2] = [
    KeySetLink {
        str: Some(c"builtin"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_commands| &d.builtin,
            |d: &mut KeyDict_get_commands| &mut d.builtin,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_commands_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_get_commands>> {
    let hash = get_commands_hash(key);
    (hash >= 0).then(|| &GET_COMMANDS_TABLE[hash as usize])
}

pub static GET_EXTMARK_TABLE: [KeySetLink<KeyDict_get_extmark>; 3] = [
    KeySetLink {
        str: Some(c"details"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_extmark| &d.details,
            |d: &mut KeyDict_get_extmark| &mut d.details,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_name"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_extmark| &d.hl_name,
            |d: &mut KeyDict_get_extmark| &mut d.hl_name,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_extmark_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_get_extmark>> {
    let hash = get_extmark_hash(key);
    (hash >= 0).then(|| &GET_EXTMARK_TABLE[hash as usize])
}

pub static GET_EXTMARKS_TABLE: [KeySetLink<KeyDict_get_extmarks>; 6] = [
    KeySetLink {
        str: Some(c"type"),
        field: KeyField::String(
            |d: &KeyDict_get_extmarks| &d.type_,
            |d: &mut KeyDict_get_extmarks| &mut d.type_,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"limit"),
        field: KeyField::Integer(
            |d: &KeyDict_get_extmarks| &d.limit,
            |d: &mut KeyDict_get_extmarks| &mut d.limit,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"details"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_extmarks| &d.details,
            |d: &mut KeyDict_get_extmarks| &mut d.details,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_name"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_extmarks| &d.hl_name,
            |d: &mut KeyDict_get_extmarks| &mut d.hl_name,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overlap"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_extmarks| &d.overlap,
            |d: &mut KeyDict_get_extmarks| &mut d.overlap,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_extmarks_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_get_extmarks>> {
    let hash = get_extmarks_hash(key);
    (hash >= 0).then(|| &GET_EXTMARKS_TABLE[hash as usize])
}

pub static GET_HIGHLIGHT_TABLE: [KeySetLink<KeyDict_get_highlight>; 5] = [
    KeySetLink {
        str: Some(c"id"),
        field: KeyField::Integer(
            |d: &KeyDict_get_highlight| &d.id,
            |d: &mut KeyDict_get_highlight| &mut d.id,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_highlight| &d.link,
            |d: &mut KeyDict_get_highlight| &mut d.link,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"name"),
        field: KeyField::String(
            |d: &KeyDict_get_highlight| &d.name,
            |d: &mut KeyDict_get_highlight| &mut d.name,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"create"),
        field: KeyField::Boolean(
            |d: &KeyDict_get_highlight| &d.create,
            |d: &mut KeyDict_get_highlight| &mut d.create,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_highlight_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_get_highlight>> {
    let hash = get_highlight_hash(key);
    (hash >= 0).then(|| &GET_HIGHLIGHT_TABLE[hash as usize])
}

pub static GET_NS_TABLE: [KeySetLink<KeyDict_get_ns>; 2] = [
    KeySetLink {
        str: Some(c"winid"),
        field: KeyField::I32(
            |d: &KeyDict_get_ns| &d.winid,
            |d: &mut KeyDict_get_ns| &mut d.winid,
        ),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_get_ns_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_get_ns>> {
    let hash = get_ns_hash(key);
    (hash >= 0).then(|| &GET_NS_TABLE[hash as usize])
}

pub static HIGHLIGHT_TABLE: [KeySetLink<KeyDict_highlight>; 37] = [
    KeySetLink {
        str: Some(c"bg"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.bg,
            |d: &mut KeyDict_highlight| &mut d.bg,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fg"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.fg,
            |d: &mut KeyDict_highlight| &mut d.fg,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sp"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.sp,
            |d: &mut KeyDict_highlight| &mut d.sp,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"dim"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.dim,
            |d: &mut KeyDict_highlight| &mut d.dim,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"url"),
        field: KeyField::String(
            |d: &KeyDict_highlight| &d.url,
            |d: &mut KeyDict_highlight| &mut d.url,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bold"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.bold,
            |d: &mut KeyDict_highlight| &mut d.bold,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"font"),
        field: KeyField::String(
            |d: &KeyDict_highlight| &d.font,
            |d: &mut KeyDict_highlight| &mut d.font,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link"),
        field: KeyField::Integer(
            |d: &KeyDict_highlight| &d.link,
            |d: &mut KeyDict_highlight| &mut d.link,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 8,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"blend"),
        field: KeyField::Integer(
            |d: &KeyDict_highlight| &d.blend,
            |d: &mut KeyDict_highlight| &mut d.blend,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.force,
            |d: &mut KeyDict_highlight| &mut d.force,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"blink"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.blink,
            |d: &mut KeyDict_highlight| &mut d.blink,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cterm"),
        field: KeyField::Dict(
            |d: &KeyDict_highlight| &d.cterm,
            |d: &mut KeyDict_highlight| &mut d.cterm,
        ),
        key_type: ObjectType::ObjectTypeDict as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"italic"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.italic,
            |d: &mut KeyDict_highlight| &mut d.italic,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"update"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.update,
            |d: &mut KeyDict_highlight| &mut d.update,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reverse"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.reverse,
            |d: &mut KeyDict_highlight| &mut d.reverse,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"default"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.default_,
            |d: &mut KeyDict_highlight| &mut d.default_,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"altfont"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.altfont,
            |d: &mut KeyDict_highlight| &mut d.altfont,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.conceal,
            |d: &mut KeyDict_highlight| &mut d.conceal,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"special"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.special,
            |d: &mut KeyDict_highlight| &mut d.special,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ctermfg"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.ctermfg,
            |d: &mut KeyDict_highlight| &mut d.ctermfg,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ctermbg"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.ctermbg,
            |d: &mut KeyDict_highlight| &mut d.ctermbg,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fallback"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.fallback,
            |d: &mut KeyDict_highlight| &mut d.fallback,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overline"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.overline,
            |d: &mut KeyDict_highlight| &mut d.overline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 23,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"standout"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.standout,
            |d: &mut KeyDict_highlight| &mut d.standout,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nocombine"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.nocombine,
            |d: &mut KeyDict_highlight| &mut d.nocombine,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 25,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undercurl"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.undercurl,
            |d: &mut KeyDict_highlight| &mut d.undercurl,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 26,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underline"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.underline,
            |d: &mut KeyDict_highlight| &mut d.underline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 27,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"background"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.background,
            |d: &mut KeyDict_highlight| &mut d.background,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 28,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bg_indexed"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.bg_indexed,
            |d: &mut KeyDict_highlight| &mut d.bg_indexed,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 29,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"foreground"),
        field: KeyField::Object(
            |d: &KeyDict_highlight| &d.foreground,
            |d: &mut KeyDict_highlight| &mut d.foreground,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 30,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fg_indexed"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.fg_indexed,
            |d: &mut KeyDict_highlight| &mut d.fg_indexed,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 31,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"link_global"),
        field: KeyField::Integer(
            |d: &KeyDict_highlight| &d.link_global,
            |d: &mut KeyDict_highlight| &mut d.link_global,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 32,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"underdashed"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.underdashed,
            |d: &mut KeyDict_highlight| &mut d.underdashed,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 33,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdotted"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.underdotted,
            |d: &mut KeyDict_highlight| &mut d.underdotted,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 34,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdouble"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.underdouble,
            |d: &mut KeyDict_highlight| &mut d.underdouble,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 35,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strikethrough"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight| &d.strikethrough,
            |d: &mut KeyDict_highlight| &mut d.strikethrough,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 36,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_highlight_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_highlight>> {
    let hash = highlight_hash(key);
    (hash >= 0).then(|| &HIGHLIGHT_TABLE[hash as usize])
}

pub static HIGHLIGHT_CTERM_TABLE: [KeySetLink<KeyDict_highlight_cterm>; 17] = [
    KeySetLink {
        str: Some(c"dim"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.dim,
            |d: &mut KeyDict_highlight_cterm| &mut d.dim,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bold"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.bold,
            |d: &mut KeyDict_highlight_cterm| &mut d.bold,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"blink"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.blink,
            |d: &mut KeyDict_highlight_cterm| &mut d.blink,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"italic"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.italic,
            |d: &mut KeyDict_highlight_cterm| &mut d.italic,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"altfont"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.altfont,
            |d: &mut KeyDict_highlight_cterm| &mut d.altfont,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.conceal,
            |d: &mut KeyDict_highlight_cterm| &mut d.conceal,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"reverse"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.reverse,
            |d: &mut KeyDict_highlight_cterm| &mut d.reverse,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"overline"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.overline,
            |d: &mut KeyDict_highlight_cterm| &mut d.overline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"standout"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.standout,
            |d: &mut KeyDict_highlight_cterm| &mut d.standout,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nocombine"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.nocombine,
            |d: &mut KeyDict_highlight_cterm| &mut d.nocombine,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undercurl"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.undercurl,
            |d: &mut KeyDict_highlight_cterm| &mut d.undercurl,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underline"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.underline,
            |d: &mut KeyDict_highlight_cterm| &mut d.underline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdashed"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.underdashed,
            |d: &mut KeyDict_highlight_cterm| &mut d.underdashed,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdotted"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.underdotted,
            |d: &mut KeyDict_highlight_cterm| &mut d.underdotted,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"underdouble"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.underdouble,
            |d: &mut KeyDict_highlight_cterm| &mut d.underdouble,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strikethrough"),
        field: KeyField::Boolean(
            |d: &KeyDict_highlight_cterm| &d.strikethrough,
            |d: &mut KeyDict_highlight_cterm| &mut d.strikethrough,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_highlight_cterm_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_highlight_cterm>> {
    let hash = highlight_cterm_hash(key);
    (hash >= 0).then(|| &HIGHLIGHT_CTERM_TABLE[hash as usize])
}

pub static KEYMAP_TABLE: [KeySetLink<KeyDict_keymap>; 10] = [
    KeySetLink {
        str: Some(c"desc"),
        field: KeyField::String(
            |d: &KeyDict_keymap| &d.desc,
            |d: &mut KeyDict_keymap| &mut d.desc,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"expr"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.expr,
            |d: &mut KeyDict_keymap| &mut d.expr,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"script"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.script,
            |d: &mut KeyDict_keymap| &mut d.script,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"silent"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.silent,
            |d: &mut KeyDict_keymap| &mut d.silent,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"unique"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.unique,
            |d: &mut KeyDict_keymap| &mut d.unique,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nowait"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.nowait,
            |d: &mut KeyDict_keymap| &mut d.nowait,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noremap"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.noremap,
            |d: &mut KeyDict_keymap| &mut d.noremap,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"callback"),
        field: KeyField::I32(
            |d: &KeyDict_keymap| &d.callback,
            |d: &mut KeyDict_keymap| &mut d.callback,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"replace_keycodes"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap| &d.replace_keycodes,
            |d: &mut KeyDict_keymap| &mut d.replace_keycodes,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_keymap_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_keymap>> {
    let hash = keymap_hash(key);
    (hash >= 0).then(|| &KEYMAP_TABLE[hash as usize])
}

pub static KEYMAP_DEL_TABLE: [KeySetLink<KeyDict_keymap_del>; 2] = [
    KeySetLink {
        str: Some(c"lhs"),
        field: KeyField::Boolean(
            |d: &KeyDict_keymap_del| &d.lhs,
            |d: &mut KeyDict_keymap_del| &mut d.lhs,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_keymap_del_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_keymap_del>> {
    let hash = keymap_del_hash(key);
    (hash >= 0).then(|| &KEYMAP_DEL_TABLE[hash as usize])
}

pub static NS_OPTS_TABLE: [KeySetLink<KeyDict_ns_opts>; 2] = [
    KeySetLink {
        str: Some(c"wins"),
        field: KeyField::Array(
            |d: &KeyDict_ns_opts| &d.wins,
            |d: &mut KeyDict_ns_opts| &mut d.wins,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_ns_opts_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_ns_opts>> {
    let hash = ns_opts_hash(key);
    (hash >= 0).then(|| &NS_OPTS_TABLE[hash as usize])
}

pub static OPEN_TERM_TABLE: [KeySetLink<KeyDict_open_term>; 3] = [
    KeySetLink {
        str: Some(c"on_input"),
        field: KeyField::I32(
            |d: &KeyDict_open_term| &d.on_input,
            |d: &mut KeyDict_open_term| &mut d.on_input,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force_crlf"),
        field: KeyField::Boolean(
            |d: &KeyDict_open_term| &d.force_crlf,
            |d: &mut KeyDict_open_term| &mut d.force_crlf,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_open_term_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_open_term>> {
    let hash = open_term_hash(key);
    (hash >= 0).then(|| &OPEN_TERM_TABLE[hash as usize])
}

pub static OPTION_TABLE: [KeySetLink<KeyDict_option>; 8] = [
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::I32(
            |d: &KeyDict_option| &d.buf,
            |d: &mut KeyDict_option| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"tab"),
        field: KeyField::I32(
            |d: &KeyDict_option| &d.tab,
            |d: &mut KeyDict_option| &mut d.tab,
        ),
        key_type: ObjectType::ObjectTypeTabpage as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        field: KeyField::I32(
            |d: &KeyDict_option| &d.win,
            |d: &mut KeyDict_option| &mut d.win,
        ),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"scope"),
        field: KeyField::String(
            |d: &KeyDict_option| &d.scope,
            |d: &mut KeyDict_option| &mut d.scope,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"dry_run"),
        field: KeyField::Boolean(
            |d: &KeyDict_option| &d.dry_run,
            |d: &mut KeyDict_option| &mut d.dry_run,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filetype"),
        field: KeyField::String(
            |d: &KeyDict_option| &d.filetype,
            |d: &mut KeyDict_option| &mut d.filetype,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"operation"),
        field: KeyField::String(
            |d: &KeyDict_option| &d.operation,
            |d: &mut KeyDict_option| &mut d.operation,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_option_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_option>> {
    let hash = option_hash(key);
    (hash >= 0).then(|| &OPTION_TABLE[hash as usize])
}

pub static REDRAW_TABLE: [KeySetLink<KeyDict_redraw>; 11] = [
    KeySetLink {
        str: Some(c"buf"),
        field: KeyField::I32(
            |d: &KeyDict_redraw| &d.buf,
            |d: &mut KeyDict_redraw| &mut d.buf,
        ),
        key_type: ObjectType::ObjectTypeBuffer as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        field: KeyField::I32(
            |d: &KeyDict_redraw| &d.win,
            |d: &mut KeyDict_redraw| &mut d.win,
        ),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"flush"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.flush,
            |d: &mut KeyDict_redraw| &mut d.flush,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        field: KeyField::Array(
            |d: &KeyDict_redraw| &d.range,
            |d: &mut KeyDict_redraw| &mut d.range,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"valid"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.valid,
            |d: &mut KeyDict_redraw| &mut d.valid,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cursor"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.cursor,
            |d: &mut KeyDict_redraw| &mut d.cursor,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"winbar"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.winbar,
            |d: &mut KeyDict_redraw| &mut d.winbar,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"tabline"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.tabline,
            |d: &mut KeyDict_redraw| &mut d.tabline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"statusline"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.statusline,
            |d: &mut KeyDict_redraw| &mut d.statusline,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"statuscolumn"),
        field: KeyField::Boolean(
            |d: &KeyDict_redraw| &d.statuscolumn,
            |d: &mut KeyDict_redraw| &mut d.statuscolumn,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_redraw_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_redraw>> {
    let hash = redraw_hash(key);
    (hash >= 0).then(|| &REDRAW_TABLE[hash as usize])
}

pub static RUNTIME_TABLE: [KeySetLink<KeyDict_runtime>; 3] = [
    KeySetLink {
        str: Some(c"is_lua"),
        field: KeyField::Boolean(
            |d: &KeyDict_runtime| &d.is_lua,
            |d: &mut KeyDict_runtime| &mut d.is_lua,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"do_source"),
        field: KeyField::Boolean(
            |d: &KeyDict_runtime| &d.do_source,
            |d: &mut KeyDict_runtime| &mut d.do_source,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_runtime_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_runtime>> {
    let hash = runtime_hash(key);
    (hash >= 0).then(|| &RUNTIME_TABLE[hash as usize])
}

pub static SET_DECORATION_PROVIDER_TABLE: [KeySetLink<KeyDict_set_decoration_provider>; 11] = [
    KeySetLink {
        str: Some(c"on_buf"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_buf,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_buf,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_end"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_end,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_end,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_win"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_win,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_win,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_line"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_line,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_line,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_range"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_range,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_range,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_start"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d.on_start,
            |d: &mut KeyDict_set_decoration_provider| &mut d.on_start,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_hl_def"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d._on_hl_def,
            |d: &mut KeyDict_set_decoration_provider| &mut d._on_hl_def,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_spell_nav"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d._on_spell_nav,
            |d: &mut KeyDict_set_decoration_provider| &mut d._on_spell_nav,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_conceal_line"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d._on_conceal_line,
            |d: &mut KeyDict_set_decoration_provider| &mut d._on_conceal_line,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_on_conceal"),
        field: KeyField::I32(
            |d: &KeyDict_set_decoration_provider| &d._on_conceal,
            |d: &mut KeyDict_set_decoration_provider| &mut d._on_conceal,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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
        b"_on_conceal" => 9,
        _ => -1,
    }
}

pub fn key_dict_set_decoration_provider_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_set_decoration_provider>> {
    let hash = set_decoration_provider_hash(key);
    (hash >= 0).then(|| &SET_DECORATION_PROVIDER_TABLE[hash as usize])
}

pub static SET_EXTMARK_TABLE: [KeySetLink<KeyDict_set_extmark>; 37] = [
    KeySetLink {
        str: Some(c"id"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.id,
            |d: &mut KeyDict_set_extmark| &mut d.id,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"url"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.url,
            |d: &mut KeyDict_set_extmark| &mut d.url,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"spell"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.spell,
            |d: &mut KeyDict_set_extmark| &mut d.spell,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"scoped"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.scoped,
            |d: &mut KeyDict_set_extmark| &mut d.scoped,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_eol"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.hl_eol,
            |d: &mut KeyDict_set_extmark| &mut d.hl_eol,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"strict"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.strict,
            |d: &mut KeyDict_set_extmark| &mut d.strict,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_col"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.end_col,
            |d: &mut KeyDict_set_extmark| &mut d.end_col,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal"),
        field: KeyField::Object(
            |d: &KeyDict_set_extmark| &d.conceal,
            |d: &mut KeyDict_set_extmark| &mut d.conceal,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_mode"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.hl_mode,
            |d: &mut KeyDict_set_extmark| &mut d.hl_mode,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_row"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.end_row,
            |d: &mut KeyDict_set_extmark| &mut d.end_row,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_line"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.end_line,
            |d: &mut KeyDict_set_extmark| &mut d.end_line,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hl_group"),
        field: KeyField::Object(
            |d: &KeyDict_set_extmark| &d.hl_group,
            |d: &mut KeyDict_set_extmark| &mut d.hl_group,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"priority"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.priority,
            |d: &mut KeyDict_set_extmark| &mut d.priority,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ephemeral"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.ephemeral,
            |d: &mut KeyDict_set_extmark| &mut d.ephemeral,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sign_text"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.sign_text,
            |d: &mut KeyDict_set_extmark| &mut d.sign_text,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text"),
        field: KeyField::Array(
            |d: &KeyDict_set_extmark| &d.virt_text,
            |d: &mut KeyDict_set_extmark| &mut d.virt_text,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"invalidate"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.invalidate,
            |d: &mut KeyDict_set_extmark| &mut d.invalidate,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ui_watched"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.ui_watched,
            |d: &mut KeyDict_set_extmark| &mut d.ui_watched,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_lines"),
        field: KeyField::Array(
            |d: &KeyDict_set_extmark| &d.virt_lines,
            |d: &mut KeyDict_set_extmark| &mut d.virt_lines,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_subpriority"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d._subpriority,
            |d: &mut KeyDict_set_extmark| &mut d._subpriority,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"undo_restore"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.undo_restore,
            |d: &mut KeyDict_set_extmark| &mut d.undo_restore,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"conceal_lines"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.conceal_lines,
            |d: &mut KeyDict_set_extmark| &mut d.conceal_lines,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"line_hl_group"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.line_hl_group,
            |d: &mut KeyDict_set_extmark| &mut d.line_hl_group,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 23,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"right_gravity"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.right_gravity,
            |d: &mut KeyDict_set_extmark| &mut d.right_gravity,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sign_hl_group"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.sign_hl_group,
            |d: &mut KeyDict_set_extmark| &mut d.sign_hl_group,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 25,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_text_pos"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.virt_text_pos,
            |d: &mut KeyDict_set_extmark| &mut d.virt_text_pos,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 26,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_hide"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.virt_text_hide,
            |d: &mut KeyDict_set_extmark| &mut d.virt_text_hide,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 27,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"number_hl_group"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.number_hl_group,
            |d: &mut KeyDict_set_extmark| &mut d.number_hl_group,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 28,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_lines_above"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.virt_lines_above,
            |d: &mut KeyDict_set_extmark| &mut d.virt_lines_above,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 29,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_right_gravity"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.end_right_gravity,
            |d: &mut KeyDict_set_extmark| &mut d.end_right_gravity,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 30,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_win_col"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.virt_text_win_col,
            |d: &mut KeyDict_set_extmark| &mut d.virt_text_win_col,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 31,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_lines_leftcol"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.virt_lines_leftcol,
            |d: &mut KeyDict_set_extmark| &mut d.virt_lines_leftcol,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 32,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"cursorline_hl_group"),
        field: KeyField::Integer(
            |d: &KeyDict_set_extmark| &d.cursorline_hl_group,
            |d: &mut KeyDict_set_extmark| &mut d.cursorline_hl_group,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 33,
        is_hlgroup: true,
    },
    KeySetLink {
        str: Some(c"virt_lines_overflow"),
        field: KeyField::String(
            |d: &KeyDict_set_extmark| &d.virt_lines_overflow,
            |d: &mut KeyDict_set_extmark| &mut d.virt_lines_overflow,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 34,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"virt_text_repeat_linebreak"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d.virt_text_repeat_linebreak,
            |d: &mut KeyDict_set_extmark| &mut d.virt_text_repeat_linebreak,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 35,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_conceal_continuation"),
        field: KeyField::Boolean(
            |d: &KeyDict_set_extmark| &d._conceal_continuation,
            |d: &mut KeyDict_set_extmark| &mut d._conceal_continuation,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 36,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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
        b"_conceal_continuation" => 35,
        _ => -1,
    }
}

pub fn key_dict_set_extmark_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_set_extmark>> {
    let hash = set_extmark_hash(key);
    (hash >= 0).then(|| &SET_EXTMARK_TABLE[hash as usize])
}

pub static TABPAGE_CONFIG_TABLE: [KeySetLink<KeyDict_tabpage_config>; 2] = [
    KeySetLink {
        str: Some(c"after"),
        field: KeyField::Integer(
            |d: &KeyDict_tabpage_config| &d.after,
            |d: &mut KeyDict_tabpage_config| &mut d.after,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_tabpage_config_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_tabpage_config>> {
    let hash = tabpage_config_hash(key);
    (hash >= 0).then(|| &TABPAGE_CONFIG_TABLE[hash as usize])
}

pub static USER_COMMAND_TABLE: [KeySetLink<KeyDict_user_command>; 13] = [
    KeySetLink {
        str: Some(c"bar"),
        field: KeyField::Boolean(
            |d: &KeyDict_user_command| &d.bar,
            |d: &mut KeyDict_user_command| &mut d.bar,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"addr"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.addr,
            |d: &mut KeyDict_user_command| &mut d.addr,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bang"),
        field: KeyField::Boolean(
            |d: &KeyDict_user_command| &d.bang,
            |d: &mut KeyDict_user_command| &mut d.bang,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"desc"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.desc,
            |d: &mut KeyDict_user_command| &mut d.desc,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"count"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.count,
            |d: &mut KeyDict_user_command| &mut d.count,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"force"),
        field: KeyField::Boolean(
            |d: &KeyDict_user_command| &d.force,
            |d: &mut KeyDict_user_command| &mut d.force,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"nargs"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.nargs,
            |d: &mut KeyDict_user_command| &mut d.nargs,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"range"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.range,
            |d: &mut KeyDict_user_command| &mut d.range,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"preview"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.preview,
            |d: &mut KeyDict_user_command| &mut d.preview,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"complete"),
        field: KeyField::Object(
            |d: &KeyDict_user_command| &d.complete,
            |d: &mut KeyDict_user_command| &mut d.complete,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"register"),
        field: KeyField::Boolean(
            |d: &KeyDict_user_command| &d.register_,
            |d: &mut KeyDict_user_command| &mut d.register_,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"keepscript"),
        field: KeyField::Boolean(
            |d: &KeyDict_user_command| &d.keepscript,
            |d: &mut KeyDict_user_command| &mut d.keepscript,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_user_command_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_user_command>> {
    let hash = user_command_hash(key);
    (hash >= 0).then(|| &USER_COMMAND_TABLE[hash as usize])
}

pub static WIN_CONFIG_TABLE: [KeySetLink<KeyDict_win_config>; 25] = [
    KeySetLink {
        str: Some(c"col"),
        field: KeyField::Float(
            |d: &KeyDict_win_config| &d.col,
            |d: &mut KeyDict_win_config| &mut d.col,
        ),
        key_type: ObjectType::ObjectTypeFloat as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"row"),
        field: KeyField::Float(
            |d: &KeyDict_win_config| &d.row,
            |d: &mut KeyDict_win_config| &mut d.row,
        ),
        key_type: ObjectType::ObjectTypeFloat as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"win"),
        field: KeyField::I32(
            |d: &KeyDict_win_config| &d.win,
            |d: &mut KeyDict_win_config| &mut d.win,
        ),
        key_type: ObjectType::ObjectTypeWindow as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hide"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.hide,
            |d: &mut KeyDict_win_config| &mut d.hide,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"width"),
        field: KeyField::Integer(
            |d: &KeyDict_win_config| &d.width,
            |d: &mut KeyDict_win_config| &mut d.width,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"split"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.split,
            |d: &mut KeyDict_win_config| &mut d.split,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title"),
        field: KeyField::Object(
            |d: &KeyDict_win_config| &d.title,
            |d: &mut KeyDict_win_config| &mut d.title,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"mouse"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.mouse,
            |d: &mut KeyDict_win_config| &mut d.mouse,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"fixed"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.fixed,
            |d: &mut KeyDict_win_config| &mut d.fixed,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"style"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.style,
            |d: &mut KeyDict_win_config| &mut d.style,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"anchor"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.anchor,
            |d: &mut KeyDict_win_config| &mut d.anchor,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"bufpos"),
        field: KeyField::Array(
            |d: &KeyDict_win_config| &d.bufpos,
            |d: &mut KeyDict_win_config| &mut d.bufpos,
        ),
        key_type: ObjectType::ObjectTypeArray as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"height"),
        field: KeyField::Integer(
            |d: &KeyDict_win_config| &d.height,
            |d: &mut KeyDict_win_config| &mut d.height,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"zindex"),
        field: KeyField::Integer(
            |d: &KeyDict_win_config| &d.zindex,
            |d: &mut KeyDict_win_config| &mut d.zindex,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"footer"),
        field: KeyField::Object(
            |d: &KeyDict_win_config| &d.footer,
            |d: &mut KeyDict_win_config| &mut d.footer,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"border"),
        field: KeyField::Object(
            |d: &KeyDict_win_config| &d.border,
            |d: &mut KeyDict_win_config| &mut d.border,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"external"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.external,
            |d: &mut KeyDict_win_config| &mut d.external,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"relative"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.relative,
            |d: &mut KeyDict_win_config| &mut d.relative,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.vertical,
            |d: &mut KeyDict_win_config| &mut d.vertical,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"focusable"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.focusable,
            |d: &mut KeyDict_win_config| &mut d.focusable,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 20,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"noautocmd"),
        field: KeyField::Boolean(
            |d: &KeyDict_win_config| &d.noautocmd,
            |d: &mut KeyDict_win_config| &mut d.noautocmd,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 21,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"title_pos"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.title_pos,
            |d: &mut KeyDict_win_config| &mut d.title_pos,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 22,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"footer_pos"),
        field: KeyField::String(
            |d: &KeyDict_win_config| &d.footer_pos,
            |d: &mut KeyDict_win_config| &mut d.footer_pos,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 23,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"_cmdline_offset"),
        field: KeyField::Integer(
            |d: &KeyDict_win_config| &d._cmdline_offset,
            |d: &mut KeyDict_win_config| &mut d._cmdline_offset,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 24,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_win_config_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_win_config>> {
    let hash = win_config_hash(key);
    (hash >= 0).then(|| &WIN_CONFIG_TABLE[hash as usize])
}

pub static WIN_RESIZE_TABLE: [KeySetLink<KeyDict_win_resize>; 2] = [
    KeySetLink {
        str: Some(c"anchor"),
        field: KeyField::String(
            |d: &KeyDict_win_resize| &d.anchor,
            |d: &mut KeyDict_win_resize| &mut d.anchor,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_win_resize_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_win_resize>> {
    let hash = win_resize_hash(key);
    (hash >= 0).then(|| &WIN_RESIZE_TABLE[hash as usize])
}

pub static WIN_TEXT_HEIGHT_TABLE: [KeySetLink<KeyDict_win_text_height>; 6] = [
    KeySetLink {
        str: Some(c"end_row"),
        field: KeyField::Integer(
            |d: &KeyDict_win_text_height| &d.end_row,
            |d: &mut KeyDict_win_text_height| &mut d.end_row,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"end_vcol"),
        field: KeyField::Integer(
            |d: &KeyDict_win_text_height| &d.end_vcol,
            |d: &mut KeyDict_win_text_height| &mut d.end_vcol,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"start_row"),
        field: KeyField::Integer(
            |d: &KeyDict_win_text_height| &d.start_row,
            |d: &mut KeyDict_win_text_height| &mut d.start_row,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"max_height"),
        field: KeyField::Integer(
            |d: &KeyDict_win_text_height| &d.max_height,
            |d: &mut KeyDict_win_text_height| &mut d.max_height,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"start_vcol"),
        field: KeyField::Integer(
            |d: &KeyDict_win_text_height| &d.start_vcol,
            |d: &mut KeyDict_win_text_height| &mut d.start_vcol,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_win_text_height_get_field(
    key: &[u8],
) -> Option<&'static KeySetLink<KeyDict_win_text_height>> {
    let hash = win_text_height_hash(key);
    (hash >= 0).then(|| &WIN_TEXT_HEIGHT_TABLE[hash as usize])
}

pub static XDL_DIFF_TABLE: [KeySetLink<KeyDict_xdl_diff>; 13] = [
    KeySetLink {
        str: Some(c"ctxlen"),
        field: KeyField::Integer(
            |d: &KeyDict_xdl_diff| &d.ctxlen,
            |d: &mut KeyDict_xdl_diff| &mut d.ctxlen,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"on_hunk"),
        field: KeyField::I32(
            |d: &KeyDict_xdl_diff| &d.on_hunk,
            |d: &mut KeyDict_xdl_diff| &mut d.on_hunk,
        ),
        key_type: ObjectType::ObjectTypeLuaRef as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"algorithm"),
        field: KeyField::String(
            |d: &KeyDict_xdl_diff| &d.algorithm,
            |d: &mut KeyDict_xdl_diff| &mut d.algorithm,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"linematch"),
        field: KeyField::Object(
            |d: &KeyDict_xdl_diff| &d.linematch,
            |d: &mut KeyDict_xdl_diff| &mut d.linematch,
        ),
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"result_type"),
        field: KeyField::String(
            |d: &KeyDict_xdl_diff| &d.result_type,
            |d: &mut KeyDict_xdl_diff| &mut d.result_type,
        ),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"interhunkctxlen"),
        field: KeyField::Integer(
            |d: &KeyDict_xdl_diff| &d.interhunkctxlen,
            |d: &mut KeyDict_xdl_diff| &mut d.interhunkctxlen,
        ),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_cr_at_eol"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.ignore_cr_at_eol,
            |d: &mut KeyDict_xdl_diff| &mut d.ignore_cr_at_eol,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"indent_heuristic"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.indent_heuristic,
            |d: &mut KeyDict_xdl_diff| &mut d.indent_heuristic,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.ignore_whitespace,
            |d: &mut KeyDict_xdl_diff| &mut d.ignore_whitespace,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_blank_lines"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.ignore_blank_lines,
            |d: &mut KeyDict_xdl_diff| &mut d.ignore_blank_lines,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace_change"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.ignore_whitespace_change,
            |d: &mut KeyDict_xdl_diff| &mut d.ignore_whitespace_change,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ignore_whitespace_change_at_eol"),
        field: KeyField::Boolean(
            |d: &KeyDict_xdl_diff| &d.ignore_whitespace_change_at_eol,
            |d: &mut KeyDict_xdl_diff| &mut d.ignore_whitespace_change_at_eol,
        ),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        field: KeyField::None,
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

pub fn key_dict_xdl_diff_get_field(key: &[u8]) -> Option<&'static KeySetLink<KeyDict_xdl_diff>> {
    let hash = xdl_diff_hash(key);
    (hash >= 0).then(|| &XDL_DIFF_TABLE[hash as usize])
}

pub fn handle_nvim_get_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
            && !extract_keydict(args[0].clone(), &mut arg_1, Some(key_dict_get_autocmds_get_field), c"Wrong type for argument 1 when calling nvim_get_autocmds, expecting Dict(get_autocmds) *", error) {
                return ret;
            }
    let rv = nvim_get_autocmds(&mut arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_create_autocmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    let arg_1: Object = args[0].clone();
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_create_autocmd_get_field), c"Wrong type for argument 2 when calling nvim_create_autocmd, expecting Dict(create_autocmd) *", error) {
                return ret;
            }
    let rv = nvim_create_autocmd(channel_id, arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = integer_obj(rv);
    ret
}

pub fn handle_nvim_del_autocmd(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_clear_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
            && !extract_keydict(args[0].clone(), &mut arg_1, Some(key_dict_clear_autocmds_get_field), c"Wrong type for argument 1 when calling nvim_clear_autocmds, expecting Dict(clear_autocmds) *", error) {
                return ret;
            }
    nvim_clear_autocmds(&mut arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_create_augroup(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_create_augroup, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_create_augroup_get_field), c"Wrong type for argument 2 when calling nvim_create_augroup, expecting Dict(create_augroup) *", error) {
                return ret;
            }
    let rv = nvim_create_augroup(channel_id, arg_1, &mut arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = integer_obj(rv);
    ret
}

pub fn handle_nvim_del_augroup_by_id(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_del_augroup_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_exec_autocmds(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    let arg_1: Object = args[0].clone();
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_exec_autocmds_get_field), c"Wrong type for argument 2 when calling nvim_exec_autocmds, expecting Dict(exec_autocmds) *", error) {
                return ret;
            }
    nvim_exec_autocmds(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_buf_line_count(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_attach, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_attach, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if args.size >= 3
        && !extract_keydict(
            args[2].clone(),
            &mut arg_3,
            Some(key_dict_buf_attach_get_field),
            c"Wrong type for argument 3 when calling nvim_buf_attach, expecting Dict(buf_attach) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_buf_attach(channel_id, arg_1, arg_2, &mut arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = boolean_obj(rv);
    ret
}

pub fn handle_nvim_buf_detach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_get_lines(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_lines, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_get_lines, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_get_lines, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
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
        std::ptr::null_mut(),
        error,
    );
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_buf_set_lines(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_lines, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_lines, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_lines, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_lines, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[4].clone(),
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

pub fn handle_nvim_buf_set_text(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_text, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[4].clone(),
        &mut arg_5,
        c"Wrong type for argument 5 when calling nvim_buf_set_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[5].clone(),
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

pub fn handle_nvim_buf_get_text(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_text, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_get_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_get_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_get_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[4].clone(),
        &mut arg_5,
        c"Wrong type for argument 5 when calling nvim_buf_get_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 6
        && !extract_keydict(
            args[5].clone(),
            &mut arg_6,
            Some(key_dict_empty_get_field),
            c"Wrong type for argument 6 when calling nvim_buf_get_text, expecting Dict(empty) *",
            error,
        )
    {
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
        std::ptr::null_mut(),
        error,
    );
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_buf_get_offset(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_offset, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_nvim_buf_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_var, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_get_changedtick(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_get_keymap(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_keymap, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_set_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_keymap, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 5
        && !extract_keydict(
            args[4].clone(),
            &mut arg_5,
            Some(key_dict_keymap_get_field),
            c"Wrong type for argument 5 when calling nvim_buf_set_keymap, expecting Dict(keymap) *",
            error,
        )
    {
        return ret;
    }
    nvim_buf_set_keymap(channel_id, arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_buf_del_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_del_keymap, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_del_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_del_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 4
            && !extract_keydict(args[3].clone(), &mut arg_4, Some(key_dict_keymap_del_get_field), c"Wrong type for argument 4 when calling nvim_buf_del_keymap, expecting Dict(keymap_del) *", error) {
                return ret;
            }
    nvim_buf_del_keymap(channel_id, arg_1, arg_2, arg_3, &mut arg_4, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_buf_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_var, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    nvim_buf_set_var(arg_1, arg_2, arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_buf_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_del_var, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_get_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_set_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_name, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_is_loaded(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_delete(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_delete, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
            &mut arg_2,
            Some(key_dict_buf_delete_get_field),
            c"Wrong type for argument 2 when calling nvim_buf_delete, expecting Dict(buf_delete) *",
            error,
        )
    {
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

pub fn handle_nvim_buf_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_del_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_del_mark, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_set_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_mark, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_mark, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_mark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_mark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 5
        && !extract_keydict(
            args[4].clone(),
            &mut arg_5,
            Some(key_dict_empty_get_field),
            c"Wrong type for argument 5 when calling nvim_buf_set_mark, expecting Dict(empty) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_buf_set_mark(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = boolean_obj(rv);
    ret
}

pub fn handle_nvim_buf_get_mark(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_mark, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_stats(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_parse_cmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_parse_cmd, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
            &mut arg_2,
            Some(key_dict_empty_get_field),
            c"Wrong type for argument 2 when calling nvim_parse_cmd, expecting Dict(empty) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_parse_cmd(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(api_keydict_to_dict(&rv, &CMD_TABLE, CMD_TABLE.len(), arena));
    ret
}

pub fn handle_nvim_cmd(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        Some(key_dict_cmd_get_field),
        c"Wrong type for argument 1 when calling nvim_cmd, expecting Dict(cmd) *",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
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

pub fn handle_nvim_create_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_create_user_command, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    if args.size >= 3
            && !extract_keydict(args[2].clone(), &mut arg_3, Some(key_dict_user_command_get_field), c"Wrong type for argument 3 when calling nvim_create_user_command, expecting Dict(user_command) *", error) {
                return ret;
            }
    nvim_create_user_command(channel_id, arg_1, arg_2, &mut arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_del_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_create_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    if !extract_handle(
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_create_user_command, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_create_user_command, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    if args.size >= 4
            && !extract_keydict(args[3].clone(), &mut arg_4, Some(key_dict_user_command_get_field), c"Wrong type for argument 4 when calling nvim_buf_create_user_command, expecting Dict(user_command) *", error) {
                return ret;
            }
    nvim_buf_create_user_command(channel_id, arg_1, arg_2, arg_3, &mut arg_4, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_buf_del_user_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_del_user_command, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_get_commands(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
            && !extract_keydict(args[0].clone(), &mut arg_1, Some(key_dict_get_commands_get_field), c"Wrong type for argument 1 when calling nvim_get_commands, expecting Dict(get_commands) *", error) {
                return ret;
            }
    let rv = nvim_get_commands(&mut arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_buf_get_commands(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_commands, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_get_commands_get_field), c"Wrong type for argument 2 when calling nvim_buf_get_commands, expecting Dict(get_commands) *", error) {
                return ret;
            }
    let rv = nvim_buf_get_commands(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_exec(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_exec, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_nvim_command_output(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_execute_lua(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_execute_lua, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[1].clone(),
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

pub fn handle_nvim_buf_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_clear_highlight(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_clear_highlight, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_clear_highlight, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_clear_highlight, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
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

pub fn handle_nvim_buf_add_highlight(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_add_highlight, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_add_highlight, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_add_highlight, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_add_highlight, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[4].clone(),
        &mut arg_5,
        c"Wrong type for argument 5 when calling nvim_buf_add_highlight, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[5].clone(),
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

pub fn handle_nvim_buf_set_virtual_text(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_virtual_text, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_virtual_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_virtual_text, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_virtual_text, expecting Array",
        error,
    ) {
        return ret;
    }
    if args.size >= 5
            && !extract_keydict(args[4].clone(), &mut arg_5, Some(key_dict_empty_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_virtual_text, expecting Dict(empty) *", error) {
                return ret;
            }
    let rv = nvim_buf_set_virtual_text(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = integer_obj(rv);
    ret
}

pub fn handle_nvim_get_hl_by_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_hl_by_id, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_nvim_get_hl_by_name(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_hl_by_name, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_buffer_insert(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_insert, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling buffer_insert, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[2].clone(),
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

pub fn handle_buffer_get_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_get_line, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_buffer_set_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_set_line, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling buffer_set_line, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
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

pub fn handle_buffer_del_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_del_line, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_buffer_get_line_slice(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_get_line_slice, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling buffer_get_line_slice, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling buffer_get_line_slice, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling buffer_get_line_slice, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[4].clone(),
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

pub fn handle_buffer_set_line_slice(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_set_line_slice, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling buffer_set_line_slice, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling buffer_set_line_slice, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling buffer_set_line_slice, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[4].clone(),
        &mut arg_5,
        c"Wrong type for argument 5 when calling buffer_set_line_slice, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[5].clone(),
        &mut arg_6,
        c"Wrong type for argument 6 when calling buffer_set_line_slice, expecting ArrayOf(String)",
        error,
    ) {
        return ret;
    }
    buffer_set_line_slice(arg_1, arg_2, arg_3, arg_4, arg_5, arg_6, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_buffer_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_set_var, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling buffer_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    let rv = buffer_set_var(arg_1, arg_2, arg_3, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_buffer_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling buffer_del_var, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_window_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling window_set_var, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling window_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    let rv = window_set_var(arg_1, arg_2, arg_3, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_window_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling window_del_var, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_tabpage_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling tabpage_set_var, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling tabpage_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    let rv = tabpage_set_var(arg_1, arg_2, arg_3, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_tabpage_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling tabpage_del_var, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_vim_set_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling vim_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    let rv = vim_set_var(arg_1, arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_vim_del_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_option_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_option, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    nvim_set_option(channel_id, arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_buf_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_option, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_buf_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_option, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_option, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    nvim_buf_set_option(channel_id, arg_1, arg_2, arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_win_get_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_get_option, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_win_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_option, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_win_set_option, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    nvim_win_set_option(channel_id, arg_1, arg_2, arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_call_atomic(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_subscribe(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_subscribe, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_subscribe(channel_id, arg_1);
    ret
}

pub fn handle_nvim_unsubscribe(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_unsubscribe, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_unsubscribe(channel_id, arg_1);
    ret
}

pub fn handle_nvim_out_write(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_out_write, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_out_write(arg_1);
    ret
}

pub fn handle_nvim_err_write(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_err_write, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_err_write(arg_1);
    ret
}

pub fn handle_nvim_err_writeln(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_err_writeln, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_err_writeln(arg_1);
    ret
}

pub fn handle_nvim_notify(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_notify, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_notify, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_dict(
        args[2].clone(),
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

pub fn handle_nvim_win_set_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_height, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_nvim_win_set_width(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_width, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_nvim_error_event(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_error_event, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_error_event, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_error_event(channel_id, arg_1, arg_2);
    ret
}

pub fn handle_nvim_ui_term_event(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_term_event, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    nvim_ui_term_event(channel_id, arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_create_namespace(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_namespaces(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_buf_get_extmark_by_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_extmark_by_id, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_get_extmark_by_id, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_get_extmark_by_id, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 4
            && !extract_keydict(args[3].clone(), &mut arg_4, Some(key_dict_get_extmark_get_field), c"Wrong type for argument 4 when calling nvim_buf_get_extmark_by_id, expecting Dict(get_extmark) *", error) {
                return ret;
            }
    let rv = nvim_buf_get_extmark_by_id(arg_1, arg_2, arg_3, &mut arg_4, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_buf_get_extmarks(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_get_extmarks, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_get_extmarks, expecting Integer",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    let arg_4: Object = args[3].clone();
    if args.size >= 5
            && !extract_keydict(args[4].clone(), &mut arg_5, Some(key_dict_get_extmarks_get_field), c"Wrong type for argument 5 when calling nvim_buf_get_extmarks, expecting Dict(get_extmarks) *", error) {
                return ret;
            }
    let rv = nvim_buf_get_extmarks(arg_1, arg_2, arg_3, arg_4, &mut arg_5, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_buf_set_extmark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_set_extmark, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_set_extmark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_set_extmark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_buf_set_extmark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 5
            && !extract_keydict(args[4].clone(), &mut arg_5, Some(key_dict_set_extmark_get_field), c"Wrong type for argument 5 when calling nvim_buf_set_extmark, expecting Dict(set_extmark) *", error) {
                return ret;
            }
    let rv = nvim_buf_set_extmark(arg_1, arg_2, arg_3, arg_4, &mut arg_5, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = integer_obj(rv);
    ret
}

pub fn handle_nvim_buf_del_extmark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_del_extmark, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_del_extmark, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
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

pub fn handle_nvim_buf_clear_namespace(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_buf_clear_namespace, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_buf_clear_namespace, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_buf_clear_namespace, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
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

pub fn handle_nvim_buf_debug_extmarks(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__buf_debug_extmarks, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim__buf_debug_extmarks, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
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

pub fn handle_nvim_ns_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__ns_set, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
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

pub fn handle_nvim_ns_get(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__ns_get, expecting Integer",
        error,
    ) {
        return ret;
    }
    let rv = nvim_ns_get(arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(api_keydict_to_dict(
        &rv,
        &NS_OPTS_TABLE,
        NS_OPTS_TABLE.len(),
        arena,
    ));
    ret
}

pub fn handle_nvim_get_option_value(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_option_value, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_option_get_field), c"Wrong type for argument 2 when calling nvim_get_option_value, expecting Dict(option) *", error) {
                return ret;
            }
    let rv = nvim_get_option_value(arg_1, &mut arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_nvim_set_option_value(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_option_value, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    if args.size >= 3
            && !extract_keydict(args[2].clone(), &mut arg_3, Some(key_dict_option_get_field), c"Wrong type for argument 3 when calling nvim_set_option_value, expecting Dict(option) *", error) {
                return ret;
            }
    let rv = nvim_set_option_value(channel_id, arg_1, arg_2, &mut arg_3, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = rv;
    ret
}

pub fn handle_nvim_get_all_options_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_option_info2(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_option_info2, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_option_get_field), c"Wrong type for argument 2 when calling nvim_get_option_info2, expecting Dict(option) *", error) {
                return ret;
            }
    let rv = nvim_get_option_info2(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_tabpage_list_wins(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_tabpage_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_tabpage_get_var, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_tabpage_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_tabpage_set_var, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_tabpage_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    nvim_tabpage_set_var(arg_1, arg_2, arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_tabpage_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_tabpage_del_var, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_tabpage_get_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_tabpage_set_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeTabpage,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_tabpage_set_win, expecting Tabpage",
        error,
    ) {
        return ret;
    }
    if !extract_handle(
        args[1].clone(),
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

pub fn handle_nvim_tabpage_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_tabpage_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_open_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_open_tabpage, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_open_tabpage, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_keydict(args[2].clone(), &mut arg_3, Some(key_dict_tabpage_config_get_field), c"Wrong type for argument 3 when calling nvim_open_tabpage, expecting Dict(tabpage_config) *", error) {
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

pub fn handle_nvim_ui_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_attach, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_ui_attach, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_dict(
        args[2].clone(),
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

pub fn handle_ui_attach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling ui_attach, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling ui_attach, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
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

pub fn handle_nvim_ui_set_focus(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_ui_detach(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_ui_try_resize(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_try_resize, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_nvim_ui_set_option(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_set_option, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    nvim_ui_set_option(channel_id, arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_ui_try_resize_grid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_try_resize_grid, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_ui_try_resize_grid, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
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

pub fn handle_nvim_ui_pum_set_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_ui_pum_set_bounds(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_ui_pum_set_bounds, expecting Float",
        error,
    ) {
        return ret;
    }
    if !extract_float(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_ui_pum_set_bounds, expecting Float",
        error,
    ) {
        return ret;
    }
    if !extract_float(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_ui_pum_set_bounds, expecting Float",
        error,
    ) {
        return ret;
    }
    if !extract_float(
        args[3].clone(),
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

pub fn handle_nvim_ui_send(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_hl_id_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_hl(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_hl, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
            &mut arg_2,
            Some(key_dict_get_highlight_get_field),
            c"Wrong type for argument 2 when calling nvim_get_hl, expecting Dict(get_highlight) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_get_hl(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_set_hl(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_hl, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_set_hl, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_keydict(
        args[2].clone(),
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

pub fn handle_nvim_get_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
            args[0].clone(),
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

pub fn handle_nvim_set_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_set_hl_ns_fast(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_feedkeys(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_feedkeys, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_feedkeys, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_feedkeys, expecting Boolean",
        error,
    ) {
        return ret;
    }
    nvim_feedkeys(arg_1, arg_2, arg_3);
    ret
}

pub fn handle_nvim_input(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_input_mouse(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_input_mouse, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_input_mouse, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_input_mouse, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_input_mouse, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[4].clone(),
        &mut arg_5,
        c"Wrong type for argument 5 when calling nvim_input_mouse, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[5].clone(),
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

pub fn handle_nvim_replace_termcodes(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_replace_termcodes, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_replace_termcodes, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_replace_termcodes, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
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

pub fn handle_nvim_exec_lua(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_exec_lua, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[1].clone(),
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

pub fn handle_nvim_exec_lua_fast(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__exec_lua_fast, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[1].clone(),
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

pub fn handle_nvim_strwidth(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_list_runtime_paths(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_runtime_inspect(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_runtime_file(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_runtime_file, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_nvim_get_lib_dir(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_runtime(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__get_runtime, expecting ArrayOf(String)",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim__get_runtime, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if args.size >= 3
        && !extract_keydict(
            args[2].clone(),
            &mut arg_3,
            Some(key_dict_runtime_get_field),
            c"Wrong type for argument 3 when calling nvim__get_runtime, expecting Dict(runtime) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_get_runtime(arg_1, arg_2, &mut arg_3, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = array_obj(rv);
    ret
}

pub fn handle_nvim_set_current_dir(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_set_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_del_current_line(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    nvim_set_var(arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_vvar(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_set_vvar(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_vvar, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_2: Object = args[1].clone();
    nvim_set_vvar(arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_echo(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    if !extract_array(args[0].clone(), &mut arg_1, c"Wrong type for argument 1 when calling nvim_echo, expecting ArrayOf(Tuple(String, *HLGroupID))", error) {
            return ret;
        }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_echo, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if args.size >= 3
        && !extract_keydict(
            args[2].clone(),
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

pub fn handle_nvim_list_bufs(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_current_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_set_current_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_list_wins(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_current_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_set_current_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_create_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_create_buf, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_nvim_open_term(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_open_term, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
            &mut arg_2,
            Some(key_dict_open_term_get_field),
            c"Wrong type for argument 2 when calling nvim_open_term, expecting Dict(open_term) *",
            error,
        )
    {
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

pub fn handle_nvim_chan_send(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_chan_send, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_list_tabpages(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_current_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_set_current_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_paste(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_paste, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_paste, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
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

pub fn handle_nvim_put(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_put, expecting ArrayOf(String)",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_put, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_put, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[3].clone(),
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

pub fn handle_nvim_get_color_by_name(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_color_map(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_context(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        && !extract_keydict(
            args[0].clone(),
            &mut arg_1,
            Some(key_dict_context_get_field),
            c"Wrong type for argument 1 when calling nvim_get_context, expecting Dict(context) *",
            error,
        )
    {
        return ret;
    }
    let rv = nvim_get_context(&mut arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_load_context(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_mode(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_keymap(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_set_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_set_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 4
        && !extract_keydict(
            args[3].clone(),
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

pub fn handle_nvim_del_keymap(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_del_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_del_keymap, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 3
        && !extract_keydict(
            args[2].clone(),
            &mut arg_3,
            Some(key_dict_keymap_del_get_field),
            c"Wrong type for argument 3 when calling nvim_del_keymap, expecting Dict(keymap_del) *",
            error,
        )
    {
        return ret;
    }
    nvim_del_keymap(channel_id, arg_1, arg_2, &mut arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_get_api_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_set_client_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_set_client_info, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_dict(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_set_client_info, expecting Dict",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_set_client_info, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_dict(
        args[3].clone(),
        &mut arg_4,
        c"Wrong type for argument 4 when calling nvim_set_client_info, expecting Dict",
        error,
    ) {
        return ret;
    }
    if !extract_dict(
        args[4].clone(),
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

pub fn handle_nvim_chan_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_chan_set, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_keydict(
        args[1].clone(),
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

pub fn handle_nvim_cmdwin_set(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__cmdwin_set, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_handle(
        args[1].clone(),
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

pub fn handle_nvim_get_chan_info(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_list_chans(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_id(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    let arg_1: Object = args[0].clone();
    let rv = nvim_id(arg_1, arena);
    ret = rv;
    ret
}

pub fn handle_nvim_id_array(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_id_dict(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_id_float(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_stats(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_list_uis(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_get_proc_children(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_proc(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_select_popupmenu_item(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_select_popupmenu_item, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_select_popupmenu_item, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_select_popupmenu_item, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if args.size >= 4
            && !extract_keydict(args[3].clone(), &mut arg_4, Some(key_dict_empty_get_field), c"Wrong type for argument 4 when calling nvim_select_popupmenu_item, expecting Dict(empty) *", error) {
                return ret;
            }
    nvim_select_popupmenu_item(arg_1, arg_2, arg_3, &mut arg_4, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_inspect_cell(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__inspect_cell, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim__inspect_cell, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
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

pub fn handle_nvim_screenshot(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__screenshot, expecting String",
        error,
    ) {
        return ret;
    }
    nvim_screenshot(arg_1);
    ret
}

pub fn handle_nvim_invalidate_glyph_cache(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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

pub fn handle_nvim_unpack(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_del_mark(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_get_mark(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_get_mark, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
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

pub fn handle_nvim_eval_statusline(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_eval_statusline, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_eval_statusline_get_field), c"Wrong type for argument 2 when calling nvim_eval_statusline, expecting Dict(eval_statusline) *", error) {
                return ret;
            }
    let rv = nvim_eval_statusline(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_complete_set(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__complete_set, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_complete_set_get_field), c"Wrong type for argument 2 when calling nvim__complete_set, expecting Dict(complete_set) *", error) {
                return ret;
            }
    let rv = nvim_complete_set(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_redraw(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
            args[0].clone(),
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

pub fn handle_nvim_set_restart_on_crash(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim__set_restart_on_crash, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim__set_restart_on_crash, expecting Array",
        error,
    ) {
        return ret;
    }
    nvim_set_restart_on_crash(arg_1, arg_2);
    ret
}

pub fn handle_nvim_exec2(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_exec2, expecting String",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
        && !extract_keydict(
            args[1].clone(),
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

pub fn handle_nvim_command(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_eval(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_call_function(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_call_function, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[1].clone(),
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

pub fn handle_nvim_call_dict_function(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
    let arg_1: Object = args[0].clone();
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_call_dict_function, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_array(
        args[2].clone(),
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

pub fn handle_nvim_parse_expression(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_parse_expression, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_parse_expression, expecting String",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[2].clone(),
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

pub fn handle_nvim_open_win(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeBuffer,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_open_win, expecting Buffer",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_open_win, expecting Boolean",
        error,
    ) {
        return ret;
    }
    if !extract_keydict(
        args[2].clone(),
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

pub fn handle_nvim_win_set_config(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_config, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_keydict(
        args[1].clone(),
        &mut arg_2,
        Some(key_dict_win_config_get_field),
        c"Wrong type for argument 2 when calling nvim_win_set_config, expecting Dict(win_config) *",
        error,
    ) {
        return ret;
    }
    nvim_win_set_config(arg_1, &mut arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_win_get_config(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_get_config, expecting Window",
        error,
    ) {
        return ret;
    }
    let rv = nvim_win_get_config(arg_1, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(api_keydict_to_dict(
        &rv,
        &WIN_CONFIG_TABLE,
        WIN_CONFIG_TABLE.len(),
        arena,
    ));
    ret
}

pub fn handle_nvim_win_get_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_set_buf(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_buf, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_handle(
        args[1].clone(),
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

pub fn handle_nvim_win_get_cursor(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_set_cursor(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_cursor, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_array(args[1].clone(), &mut arg_2, c"Wrong type for argument 2 when calling nvim_win_set_cursor, expecting ArrayOf(Integer, 2)", error) {
            return ret;
        }
    nvim_win_set_cursor(arg_1, arg_2, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_win_get_height(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_get_width(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_get_var(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_get_var, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_win_set_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_var, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_win_set_var, expecting String",
        error,
    ) {
        return ret;
    }
    let arg_3: Object = args[2].clone();
    nvim_win_set_var(arg_1, arg_2, arg_3, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

pub fn handle_nvim_win_del_var(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_del_var, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_string(
        args[1].clone(),
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

pub fn handle_nvim_win_get_position(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_get_tabpage(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_get_number(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_is_valid(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_hide(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
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

pub fn handle_nvim_win_close(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_close, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_boolean(
        args[1].clone(),
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

pub fn handle_nvim_win_set_hl_ns(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_set_hl_ns, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
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

pub fn handle_nvim_win_text_height(
    channel_id: u64,
    args: Array,
    arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_text_height, expecting Window",
        error,
    ) {
        return ret;
    }
    if args.size >= 2
            && !extract_keydict(args[1].clone(), &mut arg_2, Some(key_dict_win_text_height_get_field), c"Wrong type for argument 2 when calling nvim_win_text_height, expecting Dict(win_text_height) *", error) {
                return ret;
            }
    let rv = nvim_win_text_height(arg_1, &mut arg_2, arena, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret = dict_obj(rv);
    ret
}

pub fn handle_nvim_win_resize(
    channel_id: u64,
    args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
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
        args[0].clone(),
        ObjectType::ObjectTypeWindow,
        &mut arg_1,
        c"Wrong type for argument 1 when calling nvim_win_resize, expecting Window",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[1].clone(),
        &mut arg_2,
        c"Wrong type for argument 2 when calling nvim_win_resize, expecting Integer",
        error,
    ) {
        return ret;
    }
    if !extract_integer(
        args[2].clone(),
        &mut arg_3,
        c"Wrong type for argument 3 when calling nvim_win_resize, expecting Integer",
        error,
    ) {
        return ret;
    }
    if args.size >= 4
        && !extract_keydict(
            args[3].clone(),
            &mut arg_4,
            Some(key_dict_win_resize_get_field),
            c"Wrong type for argument 4 when calling nvim_win_resize, expecting Dict(win_resize) *",
            error,
        )
    {
        return ret;
    }
    nvim_win_resize(arg_1, arg_2, arg_3, &mut arg_4, error);
    if error.err_type != ErrorType::ErrorTypeNone {
        return ret;
    }
    ret
}

macro_rules! rpc_fn {
    ($f:ident) => {{ Some($f) }};
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
    key_bytes: &[u8],
    error: &mut Error,
) -> MsgpackRpcRequestHandler {
    let hash = msgpack_rpc_get_handler_for_hash(key_bytes);
    if hash < 0 {
        let mut msg = b"Invalid method: ".to_vec();
        if key_bytes.is_empty() {
            msg.extend_from_slice(b"<empty>");
        } else {
            msg.extend_from_slice(key_bytes);
        }
        error.msg = Some(crate::memory::bytes_to_cstring(&msg));
        error.err_type = ErrorType::ErrorTypeException;
        return MsgpackRpcRequestHandler {
            name: c"",
            r#fn: None,
            fast: false,
            ret_alloc: false,
        };
    }
    msgpack_rpc_get_handler_at(hash)
}

use crate::lua::converter::{
    lua_State, nlua_fast_api_disabled_error, nlua_finish_api_call, nlua_gettop,
    nlua_register_api_table,
};
use crate::lua::executor::{
    active_lstate, api_free_luaref, nlua_is_deferred_safe, set_active_lstate,
};

const K_NLUA_PUSH_SPECIAL_CONST: i32 = 0x01;
const K_NLUA_PUSH_FREE_REFS_CONST: i32 = 0x02;

extern "C-unwind" fn nlua_api_nvim_get_autocmds(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_get_autocmds = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_get_autocmds_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_autocmds(&mut arg1, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &GET_AUTOCMDS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_create_autocmd(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_create_autocmd = Default::default();
    let arg1: Object;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 to 2 arguments",
                );
                break 'exit_0;
            }
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_create_autocmd(
                LUA_INTERNAL_CALL,
                arg1.clone(),
                &mut arg2,
                &mut arena,
                &mut err,
            );
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
            api_luarefs_free_object(arg1);
        }
        api_luarefs_free_keydict(&arg2, &CREATE_AUTOCMD_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_del_autocmd(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_autocmd(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_clear_autocmds(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_clear_autocmds = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_clear_autocmds_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_clear_autocmds(&mut arg1, &mut arena, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &CLEAR_AUTOCMDS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_create_augroup(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_create_augroup = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_create_augroup(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &CREATE_AUGROUP_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_del_augroup_by_id(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_augroup_by_id(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_del_augroup_by_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_augroup_by_name(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_exec_autocmds(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_exec_autocmds = Default::default();
    let arg1: Object;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_exec_autocmds(arg1.clone(), &mut arg2, &mut arena, &mut err);
            set_active_lstate(save_active_lstate);
            api_luarefs_free_object(arg1);
        }
        api_luarefs_free_keydict(&arg2, &EXEC_AUTOCMDS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_line_count(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_line_count(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_attach(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_buf_attach = Default::default();
    let arg2: Boolean;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
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
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_buf_attach(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                nlua_push_boolean(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &BUF_ATTACH_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_lines(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) != 4 {
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    let pretop = nlua_gettop(lstate);
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
                            || nlua_gettop(lstate) == pretop + 1
                    );
                    let _ = ret;
                    set_active_lstate(save_active_lstate);
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_lines(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) != 5 {
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
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
                        set_active_lstate(save_active_lstate);
                    }
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_text(lstate: *mut lua_State) -> i32 {
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
                            if nlua_gettop(lstate) != 6 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 6 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_text");
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
                            let save_active_lstate = active_lstate();
                            set_active_lstate(lstate);
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
                            set_active_lstate(save_active_lstate);
                        }
                    }
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_text(lstate: *mut lua_State) -> i32 {
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
                            if nlua_gettop(lstate) < 5 || nlua_gettop(lstate) > 6 {
                                api_set_error_str(
                                    &mut err,
                                    ErrorType::ErrorTypeValidation,
                                    c"Expected 5 to 6 arguments",
                                );
                                break 'exit_0;
                            }
                            if !nlua_is_deferred_safe() {
                                return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_text");
                            }
                            if nlua_gettop(lstate) == 6 {
                                nlua_pop_keydict(
                                    lstate,
                                    &mut arg6,
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
                            let save_active_lstate = active_lstate();
                            set_active_lstate(lstate);
                            let pretop = nlua_gettop(lstate);
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
                                    || nlua_gettop(lstate) == pretop + 1
                            );
                            let _ = ret;
                            set_active_lstate(save_active_lstate);
                        }
                    }
                }
            }
        }
        api_luarefs_free_keydict(&arg6, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_offset(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Integer;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_get_offset(arg1, arg2, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_buf_get_var(arg1, arg2, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_changedtick(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_get_changedtick(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_keymap(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_get_keymap(arg1, arg2, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_keymap(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) < 4 || nlua_gettop(lstate) > 5 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 to 5 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_keymap");
                        }
                        if nlua_gettop(lstate) == 5 {
                            nlua_pop_keydict(
                                lstate,
                                &mut arg5,
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
                        nvim_buf_set_keymap(
                            LUA_INTERNAL_CALL,
                            arg1,
                            arg2,
                            arg3,
                            arg4,
                            &mut arg5,
                            &mut err,
                        );
                        set_active_lstate(save_active_lstate);
                    }
                }
            }
        }
        api_luarefs_free_keydict(&arg5, &KEYMAP_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_del_keymap(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
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
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_buf_del_keymap(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
        api_luarefs_free_keydict(&arg4, &KEYMAP_DEL_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Object;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_buf_set_var(arg1, arg2, arg3.clone(), &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_object(arg3);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_del_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_buf_del_var(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_get_name(arg1, &mut err);
        nlua_push_string(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_buf_set_name(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_is_loaded(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_is_loaded(arg1);
        nlua_push_boolean(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_delete(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_buf_delete = Default::default();
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_buf_delete(arg1, &mut arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &BUF_DELETE_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_is_valid(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_is_valid(arg1);
        nlua_push_boolean(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_del_mark(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_del_mark(arg1, arg2, &mut err);
            nlua_push_boolean(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_mark(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) < 4 || nlua_gettop(lstate) > 5 {
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
                        if nlua_gettop(lstate) == 5 {
                            nlua_pop_keydict(
                                lstate,
                                &mut arg5,
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
                        let ret = nvim_buf_set_mark(arg1, arg2, arg3, arg4, &mut arg5, &mut err);
                        nlua_push_boolean(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        set_active_lstate(save_active_lstate);
                    }
                }
            }
        }
        api_luarefs_free_keydict(&arg5, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_mark(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_get_mark(arg1, arg2, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_call(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: LuaRef;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_call(arg1, arg2, &mut err);
            let _ = ret;
            set_active_lstate(save_active_lstate);
        }
        api_free_luaref(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
}

extern "C-unwind" fn nlua_api_nvim_buf_stats(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_stats(arg1, &mut arena, &mut err);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_parse_cmd(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_empty = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 to 2 arguments",
                );
                break 'exit_0;
            }
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_parse_cmd(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_keydict(lstate, &ret, &CMD_TABLE);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_cmd(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_cmd_opts = Default::default();
    let mut arg1: KeyDict_cmd = Default::default();
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
                if nlua_gettop(lstate) == 2 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg2,
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
                    &mut arg1,
                    Some(key_dict_cmd_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_2;
                }
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
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
                set_active_lstate(save_active_lstate);
            }
            api_luarefs_free_keydict(&arg1, &CMD_TABLE);
        }
        api_luarefs_free_keydict(&arg2, &CMD_OPTS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_create_user_command(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_user_command = Default::default();
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
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
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_create_user_command(
                    LUA_INTERNAL_CALL,
                    arg1,
                    arg2.clone(),
                    &mut arg3,
                    &mut err,
                );
                set_active_lstate(save_active_lstate);
            }
            api_luarefs_free_object(arg2);
        }
        api_luarefs_free_keydict(&arg3, &USER_COMMAND_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_del_user_command(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_user_command(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_create_user_command(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
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
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_buf_create_user_command(
                        LUA_INTERNAL_CALL,
                        arg1,
                        arg2,
                        arg3.clone(),
                        &mut arg4,
                        &mut err,
                    );
                    set_active_lstate(save_active_lstate);
                }
            }
            api_luarefs_free_object(arg3);
        }
        api_luarefs_free_keydict(&arg4, &USER_COMMAND_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_del_user_command(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_buf_del_user_command(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_commands(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_get_commands = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_get_commands_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_commands(&mut arg1, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &GET_COMMANDS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_commands(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_get_commands = Default::default();
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_buf_get_commands(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &GET_COMMANDS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_exec(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_exec(LUA_INTERNAL_CALL, arg1, arg2, &mut err);
            nlua_push_string(
                lstate,
                ret.clone(),
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
            api_free_string(ret);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_command_output(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_command_output(LUA_INTERNAL_CALL, arg1, &mut err);
        nlua_push_string(
            lstate,
            ret.clone(),
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
        api_free_string(ret);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_number(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_buf_get_number(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_clear_highlight(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) != 4 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 4 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_clear_highlight");
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_buf_clear_highlight(arg1, arg2, arg3, arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_add_highlight(lstate: *mut lua_State) -> i32 {
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
                            if nlua_gettop(lstate) != 6 {
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
                            let save_active_lstate = active_lstate();
                            set_active_lstate(lstate);
                            let ret = nvim_buf_add_highlight(
                                arg1, arg2, arg3, arg4, arg5, arg6, &mut err,
                            );
                            nlua_push_integer(
                                lstate,
                                ret,
                                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                            );
                            set_active_lstate(save_active_lstate);
                        }
                    }
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_virtual_text(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) < 4 || nlua_gettop(lstate) > 5 {
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
                        if nlua_gettop(lstate) == 5 {
                            nlua_pop_keydict(
                                lstate,
                                &mut arg5,
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
                        let ret =
                            nvim_buf_set_virtual_text(arg1, arg2, arg3, arg4, &mut arg5, &mut err);
                        nlua_push_integer(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        set_active_lstate(save_active_lstate);
                    }
                }
            }
        }
        api_luarefs_free_keydict(&arg5, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_hl_by_id(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_hl_by_id(arg1, arg2, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_hl_by_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_hl_by_name(arg1, arg2, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_option_info(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_option_info(arg1, &mut arena, &mut err);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_set_option(LUA_INTERNAL_CALL, arg1, arg2.clone(), &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_object(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_get_option(arg1, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
        api_free_object(ret);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_buf_get_option(arg1, arg2, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
            api_free_object(ret);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Object;
    let arg2: NvimString;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_buf_set_option(LUA_INTERNAL_CALL, arg1, arg2, arg3.clone(), &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_object(arg3);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_get_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_win_get_option(arg1, arg2, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
            api_free_object(ret);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_option(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Object;
    let arg2: NvimString;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_win_set_option(LUA_INTERNAL_CALL, arg1, arg2, arg3.clone(), &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_object(arg3);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_out_write(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_out_write(arg1);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_err_write(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_err_write(arg1);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_err_writeln(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_err_writeln(arg1);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_notify(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Dict;
    let arg2: Integer;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let mut ret = nvim_notify(arg1, arg2, arg3, &mut arena, &mut err);
                nlua_push_object(
                    lstate,
                    &mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_height(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Integer;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_height(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_set_width(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Integer;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_width(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_create_namespace(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_create_namespace(arg1);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_namespaces(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_namespaces(&mut arena);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_extmark_by_id(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 to 4 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_extmark_by_id");
                    }
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    let ret = nvim_buf_get_extmark_by_id(
                        arg1, arg2, arg3, &mut arg4, &mut arena, &mut err,
                    );
                    nlua_push_array(
                        lstate,
                        ret,
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    set_active_lstate(save_active_lstate);
                }
            }
        }
        api_luarefs_free_keydict(&arg4, &GET_EXTMARK_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_get_extmarks(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) < 4 || nlua_gettop(lstate) > 5 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 to 5 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_buf_get_extmarks");
                        }
                        if nlua_gettop(lstate) == 5 {
                            nlua_pop_keydict(
                                lstate,
                                &mut arg5,
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
                        let ret = nvim_buf_get_extmarks(
                            arg1,
                            arg2,
                            arg3.clone(),
                            arg4.clone(),
                            &mut arg5,
                            &mut arena,
                            &mut err,
                        );
                        nlua_push_array(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        set_active_lstate(save_active_lstate);
                    }
                }
                api_luarefs_free_object(arg3);
            }
            api_luarefs_free_object(arg4);
        }
        api_luarefs_free_keydict(&arg5, &GET_EXTMARKS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_extmark(lstate: *mut lua_State) -> i32 {
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
                        if nlua_gettop(lstate) < 4 || nlua_gettop(lstate) > 5 {
                            api_set_error_str(
                                &mut err,
                                ErrorType::ErrorTypeValidation,
                                c"Expected 4 to 5 arguments",
                            );
                            break 'exit_0;
                        }
                        if !nlua_is_deferred_safe() {
                            return nlua_fast_api_disabled_error(lstate, c"nvim_buf_set_extmark");
                        }
                        if nlua_gettop(lstate) == 5 {
                            nlua_pop_keydict(
                                lstate,
                                &mut arg5,
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
                        let save_active_lstate = active_lstate();
                        set_active_lstate(lstate);
                        let ret = nvim_buf_set_extmark(arg1, arg2, arg3, arg4, &mut arg5, &mut err);
                        nlua_push_integer(
                            lstate,
                            ret,
                            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                        );
                        set_active_lstate(save_active_lstate);
                    }
                }
            }
        }
        api_luarefs_free_keydict(&arg5, &SET_EXTMARK_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_del_extmark(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Integer;
    let arg2: Integer;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_buf_del_extmark(arg1, arg2, arg3, &mut err);
                nlua_push_boolean(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_buf_clear_namespace(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) != 4 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 4 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_buf_clear_namespace");
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_buf_clear_namespace(arg1, arg2, arg3, arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_set_decoration_provider(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_set_decoration_provider = Default::default();
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_set_decoration_provider(arg1, &mut arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &SET_DECORATION_PROVIDER_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_set_conceal_provider(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 2 {
            api_set_error_str(
                &mut err,
                ErrorType::ErrorTypeValidation,
                c"Expected 2 arguments",
            );
            break 'exit_0;
        }
        if !nlua_is_deferred_safe() {
            return nlua_fast_api_disabled_error(lstate, c"nvim__buf_set_conceal_provider");
        }
        arg2 = nlua_pop_boolean(lstate, &mut arena, &mut err);
        if err.err_type != ErrorType::ErrorTypeNone {
            err_param = Some(c"enabled");
            break 'exit_0;
        }
        arg1 = nlua_pop_Buffer(lstate, &mut arena, &mut err);
        if err.err_type != ErrorType::ErrorTypeNone {
            err_param = Some(c"buf");
            break 'exit_0;
        }
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_buf_set_conceal_provider(arg1, arg2, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_buf_debug_extmarks(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Boolean;
    let arg2: Boolean;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_buf_debug_extmarks(arg1, arg2, arg3, &mut err);
                nlua_push_string(
                    lstate,
                    ret.clone(),
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
                api_free_string(ret);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_ns_set(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_ns_opts = Default::default();
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_ns_set(arg1, &mut arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &NS_OPTS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_ns_get(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_ns_get(arg1, &mut arena, &mut err);
        nlua_push_keydict(lstate, &ret, &NS_OPTS_TABLE);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_option_to_object(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_option_to_object(arg1, arg2.clone(), &mut arena, &mut err);
            nlua_push_object(lstate, &mut ret, K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_object(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_option_value(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_option = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_get_option_value(arg1, &mut arg2, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
            api_free_object(ret);
        }
        api_luarefs_free_keydict(&arg2, &OPTION_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_option_value(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_option = Default::default();
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
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
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let mut ret = nvim_set_option_value(
                    LUA_INTERNAL_CALL,
                    arg1,
                    arg2.clone(),
                    &mut arg3,
                    &mut arena,
                    &mut err,
                );
                nlua_push_object(
                    lstate,
                    &mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
            api_luarefs_free_object(arg2);
        }
        api_luarefs_free_keydict(&arg3, &OPTION_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_all_options_info(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_all_options_info(&mut arena, &mut err);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_option_info2(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_option = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_option_info2(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &OPTION_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_list_wins(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Tabpage;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_tabpage_list_wins(arg1, &mut arena, &mut err);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_get_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Tabpage;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_tabpage_get_var(arg1, arg2, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_set_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Object;
    let arg2: NvimString;
    let arg1: Tabpage;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_tabpage_set_var(arg1, arg2, arg3.clone(), &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_object(arg3);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_del_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Tabpage;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_tabpage_del_var(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_get_win(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Tabpage;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_tabpage_get_win(arg1, &mut err);
        nlua_push_Window(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_set_win(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Window;
    let arg1: Tabpage;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_tabpage_set_win(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_get_number(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Tabpage;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_tabpage_get_number(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_tabpage_is_valid(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Tabpage;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_tabpage_is_valid(arg1);
        nlua_push_boolean(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_open_tabpage(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_tabpage_config = Default::default();
    let arg2: Boolean;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                    &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_open_tabpage(arg1, arg2, &mut arg3, &mut err);
                nlua_push_Tabpage(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &TABPAGE_CONFIG_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_ui_send(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_ui_send(LUA_INTERNAL_CALL, arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_hl_id_by_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_hl_id_by_name(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_hl(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_get_highlight = Default::default();
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_hl(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &GET_HIGHLIGHT_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_hl(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_highlight = Default::default();
    let arg2: NvimString;
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                    &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_set_hl(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &HIGHLIGHT_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_hl_ns(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_get_ns = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_get_ns_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_hl_ns(&mut arg1, &mut err);
            nlua_push_integer(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &GET_NS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_hl_ns(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_hl_ns(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_set_hl_ns_fast(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_hl_ns_fast(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_feedkeys(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Boolean;
    let arg2: NvimString;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_feedkeys(arg1, arg2, arg3);
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_input(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_input(LUA_INTERNAL_CALL, arg1);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_input_mouse(lstate: *mut lua_State) -> i32 {
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
                            if nlua_gettop(lstate) != 6 {
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
                            let save_active_lstate = active_lstate();
                            set_active_lstate(lstate);
                            nvim_input_mouse(arg1, arg2, arg3, arg4, arg5, arg6, &mut err);
                            set_active_lstate(save_active_lstate);
                        }
                    }
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_replace_termcodes(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) != 4 {
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    let ret = nvim_replace_termcodes(arg1, arg2, arg3, arg4);
                    nlua_push_string(
                        lstate,
                        ret.clone(),
                        K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                    );
                    set_active_lstate(save_active_lstate);
                    api_free_string(ret);
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_strwidth(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_strwidth(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_list_runtime_paths(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_runtime_paths(&mut arena, &mut err);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_runtime_inspect(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_runtime_inspect(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_runtime_file(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_runtime_file(arg1, arg2, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_lib_dir(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_lib_dir();
        nlua_push_string(
            lstate,
            ret.clone(),
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
        api_free_string(ret);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_runtime(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_runtime = Default::default();
    let arg2: Boolean;
    let arg1: Array;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
                    api_set_error_str(
                        &mut err,
                        ErrorType::ErrorTypeValidation,
                        c"Expected 2 to 3 arguments",
                    );
                    break 'exit_0;
                }
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_get_runtime(arg1, arg2, &mut arg3, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &RUNTIME_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_current_dir(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_current_dir(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_current_line(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_current_line(&mut arena, &mut err);
        nlua_push_string(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_current_line(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_current_line(arg1, &mut arena, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_del_current_line(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_current_line(&mut arena, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_get_var(arg1, &mut arena, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_set_var(arg1, arg2.clone(), &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_object(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_del_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_del_var(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_vvar(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_get_vvar(arg1, &mut arena, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_vvar(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Object;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_set_vvar(arg1, arg2.clone(), &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_object(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_echo(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_echo_opts = Default::default();
    let arg2: Boolean;
    let arg1: Array;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
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
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let mut ret = nvim_echo(arg1, arg2, &mut arg3, &mut err);
                nlua_push_object(
                    lstate,
                    &mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &ECHO_OPTS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_list_bufs(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_bufs(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_current_buf(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_current_buf();
        nlua_push_Buffer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_current_buf(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Buffer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_current_buf(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_list_wins(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_wins(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_current_win(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_current_win();
        nlua_push_Window(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_current_win(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_current_win(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_create_buf(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: Boolean;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_create_buf(arg1, arg2, &mut err);
            nlua_push_Buffer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_open_term(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_open_term = Default::default();
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_open_term(arg1, &mut arg2, &mut err);
            nlua_push_integer(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &OPEN_TERM_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_chan_send(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_chan_send(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_list_tabpages(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_tabpages(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_current_tabpage(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_current_tabpage();
        nlua_push_Tabpage(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_current_tabpage(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Tabpage;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_set_current_tabpage(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_paste(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Integer;
    let arg2: Boolean;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_paste(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut arena, &mut err);
                nlua_push_boolean(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_put(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) != 4 {
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_put(arg1, arg2, arg3, arg4, &mut arena, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_color_by_name(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_color_by_name(arg1);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_color_map(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_color_map(&mut arena);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_context(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_context = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_context_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_context(&mut arg1, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &CONTEXT_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_load_context(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Dict;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_load_context(arg1, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_mode(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
            api_set_error_str(
                &mut err,
                ErrorType::ErrorTypeValidation,
                c"Expected 0 arguments",
            );
            break 'exit_0;
        }
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_mode(&mut arena);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_keymap(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_keymap(arg1, &mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_set_keymap(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
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
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_set_keymap(LUA_INTERNAL_CALL, arg1, arg2, arg3, &mut arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
        api_luarefs_free_keydict(&arg4, &KEYMAP_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_del_keymap(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_keymap_del = Default::default();
    let arg2: NvimString;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) < 2 || nlua_gettop(lstate) > 3 {
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
                if nlua_gettop(lstate) == 3 {
                    nlua_pop_keydict(
                        lstate,
                        &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_del_keymap(LUA_INTERNAL_CALL, arg1, arg2, &mut arg3, &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &KEYMAP_DEL_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_chan_set(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_chan_set = Default::default();
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 2 arguments",
                );
                break 'exit_0;
            }
            nlua_pop_keydict(
                lstate,
                &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_chan_set(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &CHAN_SET_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_cmdwin_set(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Buffer;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_cmdwin_set(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_get_chan_info(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_chan_info(LUA_INTERNAL_CALL, arg1, &mut arena, &mut err);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_list_chans(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_chans(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_id(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Object;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_id(arg1.clone(), &mut arena);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
        api_luarefs_free_object(arg1);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_id_array(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Array;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_id_array(arg1, &mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_id_dict(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Dict;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_id_dict(arg1, &mut arena);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_id_float(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Float;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim__id_float(arg1);
        nlua_push_float(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_stats(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_stats(&mut arena);
        nlua_push_dict(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_list_uis(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_list_uis(&mut arena);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_proc_children(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_get_proc_children(arg1, &mut arena, &mut err);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_proc(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Integer;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_get_proc(arg1, &mut arena, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_select_popupmenu_item(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
                        api_set_error_str(
                            &mut err,
                            ErrorType::ErrorTypeValidation,
                            c"Expected 3 to 4 arguments",
                        );
                        break 'exit_0;
                    }
                    if !nlua_is_deferred_safe() {
                        return nlua_fast_api_disabled_error(lstate, c"nvim_select_popupmenu_item");
                    }
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_select_popupmenu_item(arg1, arg2, arg3, &mut arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
        api_luarefs_free_keydict(&arg4, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_inspect_cell(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Integer;
    let arg2: Integer;
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_inspect_cell(arg1, arg2, arg3, &mut arena, &mut err);
                nlua_push_array(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_screenshot(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_screenshot(arg1);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_invalidate_glyph_cache(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let err_param: Option<&'static std::ffi::CStr> = None;
    'exit_0: {
        if nlua_gettop(lstate) != 0 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_invalidate_glyph_cache();
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_unpack(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_unpack(arg1, &mut arena, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_del_mark(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_del_mark(arg1, &mut err);
        nlua_push_boolean(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_get_mark(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_empty = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_get_mark(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_array(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &EMPTY_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_eval_statusline(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_eval_statusline = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
                api_set_error_str(
                    &mut err,
                    ErrorType::ErrorTypeValidation,
                    c"Expected 1 to 2 arguments",
                );
                break 'exit_0;
            }
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_eval_statusline(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &EVAL_STATUSLINE_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_complete_set(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_complete_set = Default::default();
    let arg1: Integer;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_complete_set(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(
                lstate,
                ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &COMPLETE_SET_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_redraw(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg1: KeyDict_redraw = Default::default();
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) > 1 {
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
            if nlua_gettop(lstate) == 1 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg1,
                    Some(key_dict_redraw_get_field),
                    &mut err_param,
                    &mut arena,
                    &mut err,
                );
                if err.err_type != ErrorType::ErrorTypeNone {
                    break 'exit_1;
                }
            }
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_redraw(&mut arg1, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg1, &REDRAW_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_set_restart_on_crash(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Array;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_set_restart_on_crash(arg1, arg2);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_exec2(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_exec_opts = Default::default();
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_exec2(LUA_INTERNAL_CALL, arg1, &mut arg2, &mut err);
            nlua_push_dict(lstate, ret.clone(), K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
            api_free_dict(ret);
        }
        api_luarefs_free_keydict(&arg2, &EXEC_OPTS_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_command(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_command(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_eval(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: NvimString;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let mut ret = nvim_eval(arg1, &mut arena, &mut err);
        nlua_push_object(
            lstate,
            &mut ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_call_function(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Array;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_call_function(LUA_INTERNAL_CALL, arg1, arg2, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_call_dict_function(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Array;
    let arg2: NvimString;
    let arg1: Object;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let mut ret = nvim_call_dict_function(
                    LUA_INTERNAL_CALL,
                    arg1.clone(),
                    arg2,
                    arg3,
                    &mut arena,
                    &mut err,
                );
                nlua_push_object(
                    lstate,
                    &mut ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
                api_luarefs_free_object(arg1);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_parse_expression(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Boolean;
    let arg2: NvimString;
    let arg1: NvimString;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_parse_expression(arg1, arg2, arg3, &mut arena, &mut err);
                nlua_push_dict(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_open_win(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg3: KeyDict_win_config = Default::default();
    let arg2: Boolean;
    let arg1: Buffer;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                    &mut arg3,
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                let ret = nvim_open_win(arg1, arg2, &mut arg3, &mut err);
                nlua_push_Window(
                    lstate,
                    ret,
                    K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
                );
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_keydict(&arg3, &WIN_CONFIG_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_config(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_win_config = Default::default();
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
                &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_config(arg1, &mut arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &WIN_CONFIG_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_get_config(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_config(arg1, &mut arena, &mut err);
        nlua_push_keydict(lstate, &ret, &WIN_CONFIG_TABLE);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_get_buf(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_buf(arg1, &mut err);
        nlua_push_Buffer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_buf(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Buffer;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_buf(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_get_cursor(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_cursor(arg1, &mut arena, &mut err);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_cursor(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Array;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_cursor(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_get_height(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_height(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_get_width(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_width(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_get_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let mut ret = nvim_win_get_var(arg1, arg2, &mut arena, &mut err);
            nlua_push_object(
                lstate,
                &mut ret,
                K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
            );
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg3: Object;
    let arg2: NvimString;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            'exit_2: {
                if nlua_gettop(lstate) != 3 {
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
                let save_active_lstate = active_lstate();
                set_active_lstate(lstate);
                nvim_win_set_var(arg1, arg2, arg3.clone(), &mut err);
                set_active_lstate(save_active_lstate);
            }
        }
        api_luarefs_free_object(arg3);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_del_var(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: NvimString;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_del_var(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_get_position(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_position(arg1, &mut arena, &mut err);
        nlua_push_array(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_get_tabpage(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_tabpage(arg1, &mut err);
        nlua_push_Tabpage(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_get_number(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_get_number(arg1, &mut err);
        nlua_push_integer(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_is_valid(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        let ret = nvim_win_is_valid(arg1);
        nlua_push_boolean(
            lstate,
            ret,
            K_NLUA_PUSH_SPECIAL_CONST | K_NLUA_PUSH_FREE_REFS_CONST,
        );
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_hide(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg1: Window;
    'exit_0: {
        if nlua_gettop(lstate) != 1 {
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
        let save_active_lstate = active_lstate();
        set_active_lstate(lstate);
        nvim_win_hide(arg1, &mut err);
        set_active_lstate(save_active_lstate);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_close(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Boolean;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_close(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_call(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: LuaRef;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_win_call(arg1, arg2, &mut err);
            let _ = ret;
            set_active_lstate(save_active_lstate);
        }
        api_free_luaref(arg2);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, -1)
}

extern "C-unwind" fn nlua_api_nvim_win_set_hl_ns(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let arg2: Integer;
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) != 2 {
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            nvim_win_set_hl_ns(arg1, arg2, &mut err);
            set_active_lstate(save_active_lstate);
        }
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

extern "C-unwind" fn nlua_api_nvim_win_text_height(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut arena: Arena = Default::default();
    let mut err_param: Option<&'static std::ffi::CStr> = None;
    let mut arg2: KeyDict_win_text_height = Default::default();
    let arg1: Window;
    'exit_0: {
        'exit_1: {
            if nlua_gettop(lstate) < 1 || nlua_gettop(lstate) > 2 {
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
            if nlua_gettop(lstate) == 2 {
                nlua_pop_keydict(
                    lstate,
                    &mut arg2,
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
            let save_active_lstate = active_lstate();
            set_active_lstate(lstate);
            let ret = nvim_win_text_height(arg1, &mut arg2, &mut arena, &mut err);
            nlua_push_dict(lstate, ret, K_NLUA_PUSH_FREE_REFS_CONST);
            set_active_lstate(save_active_lstate);
        }
        api_luarefs_free_keydict(&arg2, &WIN_TEXT_HEIGHT_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 1)
}

extern "C-unwind" fn nlua_api_nvim_win_resize(lstate: *mut lua_State) -> i32 {
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
                    if nlua_gettop(lstate) < 3 || nlua_gettop(lstate) > 4 {
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
                    if nlua_gettop(lstate) == 4 {
                        nlua_pop_keydict(
                            lstate,
                            &mut arg4,
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
                    let save_active_lstate = active_lstate();
                    set_active_lstate(lstate);
                    nvim_win_resize(arg1, arg2, arg3, &mut arg4, &mut err);
                    set_active_lstate(save_active_lstate);
                }
            }
        }
        api_luarefs_free_keydict(&arg4, &WIN_RESIZE_TABLE);
    }
    nlua_finish_api_call(lstate, &mut arena, &mut err, err_param, 0)
}

pub extern "C-unwind" fn nlua_add_api_functions(lstate: *mut lua_State) {
    type ApiLuaFn = extern "C-unwind" fn(*mut lua_State) -> i32;
    const API_FUNCTIONS: &[(&std::ffi::CStr, ApiLuaFn)] = &[
        (c"nvim_get_autocmds", nlua_api_nvim_get_autocmds),
        (c"nvim_create_autocmd", nlua_api_nvim_create_autocmd),
        (c"nvim_del_autocmd", nlua_api_nvim_del_autocmd),
        (c"nvim_clear_autocmds", nlua_api_nvim_clear_autocmds),
        (c"nvim_create_augroup", nlua_api_nvim_create_augroup),
        (c"nvim_del_augroup_by_id", nlua_api_nvim_del_augroup_by_id),
        (
            c"nvim_del_augroup_by_name",
            nlua_api_nvim_del_augroup_by_name,
        ),
        (c"nvim_exec_autocmds", nlua_api_nvim_exec_autocmds),
        (c"nvim_buf_line_count", nlua_api_nvim_buf_line_count),
        (c"nvim_buf_attach", nlua_api_nvim_buf_attach),
        (c"nvim_buf_get_lines", nlua_api_nvim_buf_get_lines),
        (c"nvim_buf_set_lines", nlua_api_nvim_buf_set_lines),
        (c"nvim_buf_set_text", nlua_api_nvim_buf_set_text),
        (c"nvim_buf_get_text", nlua_api_nvim_buf_get_text),
        (c"nvim_buf_get_offset", nlua_api_nvim_buf_get_offset),
        (c"nvim_buf_get_var", nlua_api_nvim_buf_get_var),
        (
            c"nvim_buf_get_changedtick",
            nlua_api_nvim_buf_get_changedtick,
        ),
        (c"nvim_buf_get_keymap", nlua_api_nvim_buf_get_keymap),
        (c"nvim_buf_set_keymap", nlua_api_nvim_buf_set_keymap),
        (c"nvim_buf_del_keymap", nlua_api_nvim_buf_del_keymap),
        (c"nvim_buf_set_var", nlua_api_nvim_buf_set_var),
        (c"nvim_buf_del_var", nlua_api_nvim_buf_del_var),
        (c"nvim_buf_get_name", nlua_api_nvim_buf_get_name),
        (c"nvim_buf_set_name", nlua_api_nvim_buf_set_name),
        (c"nvim_buf_is_loaded", nlua_api_nvim_buf_is_loaded),
        (c"nvim_buf_delete", nlua_api_nvim_buf_delete),
        (c"nvim_buf_is_valid", nlua_api_nvim_buf_is_valid),
        (c"nvim_buf_del_mark", nlua_api_nvim_buf_del_mark),
        (c"nvim_buf_set_mark", nlua_api_nvim_buf_set_mark),
        (c"nvim_buf_get_mark", nlua_api_nvim_buf_get_mark),
        (c"nvim_buf_call", nlua_api_nvim_buf_call),
        (c"nvim__buf_stats", nlua_api_nvim_buf_stats),
        (c"nvim_parse_cmd", nlua_api_nvim_parse_cmd),
        (c"nvim_cmd", nlua_api_nvim_cmd),
        (
            c"nvim_create_user_command",
            nlua_api_nvim_create_user_command,
        ),
        (c"nvim_del_user_command", nlua_api_nvim_del_user_command),
        (
            c"nvim_buf_create_user_command",
            nlua_api_nvim_buf_create_user_command,
        ),
        (
            c"nvim_buf_del_user_command",
            nlua_api_nvim_buf_del_user_command,
        ),
        (c"nvim_get_commands", nlua_api_nvim_get_commands),
        (c"nvim_buf_get_commands", nlua_api_nvim_buf_get_commands),
        (c"nvim_exec", nlua_api_nvim_exec),
        (c"nvim_command_output", nlua_api_nvim_command_output),
        (c"nvim_buf_get_number", nlua_api_nvim_buf_get_number),
        (
            c"nvim_buf_clear_highlight",
            nlua_api_nvim_buf_clear_highlight,
        ),
        (c"nvim_buf_add_highlight", nlua_api_nvim_buf_add_highlight),
        (
            c"nvim_buf_set_virtual_text",
            nlua_api_nvim_buf_set_virtual_text,
        ),
        (c"nvim_get_hl_by_id", nlua_api_nvim_get_hl_by_id),
        (c"nvim_get_hl_by_name", nlua_api_nvim_get_hl_by_name),
        (c"nvim_get_option_info", nlua_api_nvim_get_option_info),
        (c"nvim_set_option", nlua_api_nvim_set_option),
        (c"nvim_get_option", nlua_api_nvim_get_option),
        (c"nvim_buf_get_option", nlua_api_nvim_buf_get_option),
        (c"nvim_buf_set_option", nlua_api_nvim_buf_set_option),
        (c"nvim_win_get_option", nlua_api_nvim_win_get_option),
        (c"nvim_win_set_option", nlua_api_nvim_win_set_option),
        (c"nvim_out_write", nlua_api_nvim_out_write),
        (c"nvim_err_write", nlua_api_nvim_err_write),
        (c"nvim_err_writeln", nlua_api_nvim_err_writeln),
        (c"nvim_notify", nlua_api_nvim_notify),
        (c"nvim_win_set_height", nlua_api_nvim_win_set_height),
        (c"nvim_win_set_width", nlua_api_nvim_win_set_width),
        (c"nvim_create_namespace", nlua_api_nvim_create_namespace),
        (c"nvim_get_namespaces", nlua_api_nvim_get_namespaces),
        (
            c"nvim_buf_get_extmark_by_id",
            nlua_api_nvim_buf_get_extmark_by_id,
        ),
        (c"nvim_buf_get_extmarks", nlua_api_nvim_buf_get_extmarks),
        (c"nvim_buf_set_extmark", nlua_api_nvim_buf_set_extmark),
        (c"nvim_buf_del_extmark", nlua_api_nvim_buf_del_extmark),
        (
            c"nvim_buf_clear_namespace",
            nlua_api_nvim_buf_clear_namespace,
        ),
        (
            c"nvim_set_decoration_provider",
            nlua_api_nvim_set_decoration_provider,
        ),
        (
            c"nvim__buf_set_conceal_provider",
            nlua_api_nvim_buf_set_conceal_provider,
        ),
        (
            c"nvim__buf_debug_extmarks",
            nlua_api_nvim_buf_debug_extmarks,
        ),
        (c"nvim__ns_set", nlua_api_nvim_ns_set),
        (c"nvim__ns_get", nlua_api_nvim_ns_get),
        (c"nvim_get_option_value", nlua_api_nvim_get_option_value),
        (c"nvim__option_to_object", nlua_api_nvim_option_to_object),
        (c"nvim_set_option_value", nlua_api_nvim_set_option_value),
        (
            c"nvim_get_all_options_info",
            nlua_api_nvim_get_all_options_info,
        ),
        (c"nvim_get_option_info2", nlua_api_nvim_get_option_info2),
        (c"nvim_tabpage_list_wins", nlua_api_nvim_tabpage_list_wins),
        (c"nvim_tabpage_get_var", nlua_api_nvim_tabpage_get_var),
        (c"nvim_tabpage_set_var", nlua_api_nvim_tabpage_set_var),
        (c"nvim_tabpage_del_var", nlua_api_nvim_tabpage_del_var),
        (c"nvim_tabpage_get_win", nlua_api_nvim_tabpage_get_win),
        (c"nvim_tabpage_set_win", nlua_api_nvim_tabpage_set_win),
        (c"nvim_tabpage_get_number", nlua_api_nvim_tabpage_get_number),
        (c"nvim_tabpage_is_valid", nlua_api_nvim_tabpage_is_valid),
        (c"nvim_open_tabpage", nlua_api_nvim_open_tabpage),
        (c"nvim_ui_send", nlua_api_nvim_ui_send),
        (c"nvim_get_hl_id_by_name", nlua_api_nvim_get_hl_id_by_name),
        (c"nvim_get_hl", nlua_api_nvim_get_hl),
        (c"nvim_set_hl", nlua_api_nvim_set_hl),
        (c"nvim_get_hl_ns", nlua_api_nvim_get_hl_ns),
        (c"nvim_set_hl_ns", nlua_api_nvim_set_hl_ns),
        (c"nvim_set_hl_ns_fast", nlua_api_nvim_set_hl_ns_fast),
        (c"nvim_feedkeys", nlua_api_nvim_feedkeys),
        (c"nvim_input", nlua_api_nvim_input),
        (c"nvim_input_mouse", nlua_api_nvim_input_mouse),
        (c"nvim_replace_termcodes", nlua_api_nvim_replace_termcodes),
        (c"nvim_strwidth", nlua_api_nvim_strwidth),
        (c"nvim_list_runtime_paths", nlua_api_nvim_list_runtime_paths),
        (c"nvim__runtime_inspect", nlua_api_nvim_runtime_inspect),
        (c"nvim_get_runtime_file", nlua_api_nvim_get_runtime_file),
        (c"nvim__get_lib_dir", nlua_api_nvim_get_lib_dir),
        (c"nvim__get_runtime", nlua_api_nvim_get_runtime),
        (c"nvim_set_current_dir", nlua_api_nvim_set_current_dir),
        (c"nvim_get_current_line", nlua_api_nvim_get_current_line),
        (c"nvim_set_current_line", nlua_api_nvim_set_current_line),
        (c"nvim_del_current_line", nlua_api_nvim_del_current_line),
        (c"nvim_get_var", nlua_api_nvim_get_var),
        (c"nvim_set_var", nlua_api_nvim_set_var),
        (c"nvim_del_var", nlua_api_nvim_del_var),
        (c"nvim_get_vvar", nlua_api_nvim_get_vvar),
        (c"nvim_set_vvar", nlua_api_nvim_set_vvar),
        (c"nvim_echo", nlua_api_nvim_echo),
        (c"nvim_list_bufs", nlua_api_nvim_list_bufs),
        (c"nvim_get_current_buf", nlua_api_nvim_get_current_buf),
        (c"nvim_set_current_buf", nlua_api_nvim_set_current_buf),
        (c"nvim_list_wins", nlua_api_nvim_list_wins),
        (c"nvim_get_current_win", nlua_api_nvim_get_current_win),
        (c"nvim_set_current_win", nlua_api_nvim_set_current_win),
        (c"nvim_create_buf", nlua_api_nvim_create_buf),
        (c"nvim_open_term", nlua_api_nvim_open_term),
        (c"nvim_chan_send", nlua_api_nvim_chan_send),
        (c"nvim_list_tabpages", nlua_api_nvim_list_tabpages),
        (
            c"nvim_get_current_tabpage",
            nlua_api_nvim_get_current_tabpage,
        ),
        (
            c"nvim_set_current_tabpage",
            nlua_api_nvim_set_current_tabpage,
        ),
        (c"nvim_paste", nlua_api_nvim_paste),
        (c"nvim_put", nlua_api_nvim_put),
        (c"nvim_get_color_by_name", nlua_api_nvim_get_color_by_name),
        (c"nvim_get_color_map", nlua_api_nvim_get_color_map),
        (c"nvim_get_context", nlua_api_nvim_get_context),
        (c"nvim_load_context", nlua_api_nvim_load_context),
        (c"nvim_get_mode", nlua_api_nvim_get_mode),
        (c"nvim_get_keymap", nlua_api_nvim_get_keymap),
        (c"nvim_set_keymap", nlua_api_nvim_set_keymap),
        (c"nvim_del_keymap", nlua_api_nvim_del_keymap),
        (c"nvim_chan_set", nlua_api_nvim_chan_set),
        (c"nvim__cmdwin_set", nlua_api_nvim_cmdwin_set),
        (c"nvim_get_chan_info", nlua_api_nvim_get_chan_info),
        (c"nvim_list_chans", nlua_api_nvim_list_chans),
        (c"nvim__id", nlua_api_nvim_id),
        (c"nvim__id_array", nlua_api_nvim_id_array),
        (c"nvim__id_dict", nlua_api_nvim_id_dict),
        (c"nvim__id_float", nlua_api_nvim_id_float),
        (c"nvim__stats", nlua_api_nvim_stats),
        (c"nvim_list_uis", nlua_api_nvim_list_uis),
        (c"nvim_get_proc_children", nlua_api_nvim_get_proc_children),
        (c"nvim_get_proc", nlua_api_nvim_get_proc),
        (
            c"nvim_select_popupmenu_item",
            nlua_api_nvim_select_popupmenu_item,
        ),
        (c"nvim__inspect_cell", nlua_api_nvim_inspect_cell),
        (c"nvim__screenshot", nlua_api_nvim_screenshot),
        (
            c"nvim__invalidate_glyph_cache",
            nlua_api_nvim_invalidate_glyph_cache,
        ),
        (c"nvim__unpack", nlua_api_nvim_unpack),
        (c"nvim_del_mark", nlua_api_nvim_del_mark),
        (c"nvim_get_mark", nlua_api_nvim_get_mark),
        (c"nvim_eval_statusline", nlua_api_nvim_eval_statusline),
        (c"nvim__complete_set", nlua_api_nvim_complete_set),
        (c"nvim__redraw", nlua_api_nvim_redraw),
        (
            c"nvim__set_restart_on_crash",
            nlua_api_nvim_set_restart_on_crash,
        ),
        (c"nvim_exec2", nlua_api_nvim_exec2),
        (c"nvim_command", nlua_api_nvim_command),
        (c"nvim_eval", nlua_api_nvim_eval),
        (c"nvim_call_function", nlua_api_nvim_call_function),
        (c"nvim_call_dict_function", nlua_api_nvim_call_dict_function),
        (c"nvim_parse_expression", nlua_api_nvim_parse_expression),
        (c"nvim_open_win", nlua_api_nvim_open_win),
        (c"nvim_win_set_config", nlua_api_nvim_win_set_config),
        (c"nvim_win_get_config", nlua_api_nvim_win_get_config),
        (c"nvim_win_get_buf", nlua_api_nvim_win_get_buf),
        (c"nvim_win_set_buf", nlua_api_nvim_win_set_buf),
        (c"nvim_win_get_cursor", nlua_api_nvim_win_get_cursor),
        (c"nvim_win_set_cursor", nlua_api_nvim_win_set_cursor),
        (c"nvim_win_get_height", nlua_api_nvim_win_get_height),
        (c"nvim_win_get_width", nlua_api_nvim_win_get_width),
        (c"nvim_win_get_var", nlua_api_nvim_win_get_var),
        (c"nvim_win_set_var", nlua_api_nvim_win_set_var),
        (c"nvim_win_del_var", nlua_api_nvim_win_del_var),
        (c"nvim_win_get_position", nlua_api_nvim_win_get_position),
        (c"nvim_win_get_tabpage", nlua_api_nvim_win_get_tabpage),
        (c"nvim_win_get_number", nlua_api_nvim_win_get_number),
        (c"nvim_win_is_valid", nlua_api_nvim_win_is_valid),
        (c"nvim_win_hide", nlua_api_nvim_win_hide),
        (c"nvim_win_close", nlua_api_nvim_win_close),
        (c"nvim_win_call", nlua_api_nvim_win_call),
        (c"nvim_win_set_hl_ns", nlua_api_nvim_win_set_hl_ns),
        (c"nvim_win_text_height", nlua_api_nvim_win_text_height),
        (c"nvim_win_resize", nlua_api_nvim_win_resize),
    ];
    nlua_register_api_table(lstate, API_FUNCTIONS);
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
