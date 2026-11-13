//! Autocommand API functions (`src/nvim/api/autocmd.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    ArrayBuilder, TryState, add_c, api_set_error, api_set_sctx, api_typename, boolean_obj,
    cstr_as_string, dict_obj, error_set, find_buffer_by_handle, has_key, integer_obj, luaref_obj,
    put_c, string_obj, try_enter, try_leave,
};
use crate::api::private::validate::Dict;
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectType, api_err_conflict, api_err_exp,
    api_err_invalid, api_err_required, check_string_array,
};
use crate::autocmd::{
    AUGROUP_ALL, AUGROUP_DEFAULT, AUGROUP_ERROR, AuEvent, BUFLOCAL_PAT_LEN, NUM_EVENTS,
    apply_autocmds_group, aucmd_del_for_event_and_group, aucmd_span_pattern_slice, augroup_add,
    augroup_del, augroup_del_by_id, augroup_exists, augroup_find, augroup_name,
    aupat_get_buflocal_nr, aupat_is_buflocal, aupat_normalize_buflocal_pat, autocmd_delete_id,
    autocmd_register, do_autocmd_event, event_name2nr_str, event_nr2name, with_autocmds_for_event,
};
use crate::buffer::do_modelines;
use crate::eval::typval::{
    Callback, K_CALLBACK_FUNCREF, K_CALLBACK_LUA, K_CALLBACK_PARTIAL, callback_free,
    callback_to_string,
};
use crate::globals::*;
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::{api_new_luaref, nlua_ref_is_function};
use crate::memory::bytes_to_cstring;
use crate::msgpack_rpc::packer::Integer;
use std::ffi::{CStr, CString};

use crate::api::tabpage::Buffer;
use crate::highlight::OptionalKeys;

const AUCMD_MAX_PATTERNS: usize = 256;

const KEYSET_OPTIDX_GET_AUTOCMDS_ID: u64 = 1;
const KEYSET_OPTIDX_GET_AUTOCMDS_BUF: u64 = 2;
const KEYSET_OPTIDX_GET_AUTOCMDS_EVENT: u64 = 3;
const KEYSET_OPTIDX_GET_AUTOCMDS_BUFFER: u64 = 5;
const KEYSET_OPTIDX_GET_AUTOCMDS_PATTERN: u64 = 6;

const KEYSET_OPTIDX_CREATE_AUTOCMD_BUF: u64 = 1;
const KEYSET_OPTIDX_CREATE_AUTOCMD_DESC: u64 = 2;
const KEYSET_OPTIDX_CREATE_AUTOCMD_BUFFER: u64 = 5;
const KEYSET_OPTIDX_CREATE_AUTOCMD_COMMAND: u64 = 7;
const KEYSET_OPTIDX_CREATE_AUTOCMD_PATTERN: u64 = 8;
const KEYSET_OPTIDX_CREATE_AUTOCMD_CALLBACK: u64 = 9;

const KEYSET_OPTIDX_CLEAR_AUTOCMDS_BUF: u64 = 1;
const KEYSET_OPTIDX_CLEAR_AUTOCMDS_BUFFER: u64 = 4;
const KEYSET_OPTIDX_CLEAR_AUTOCMDS_PATTERN: u64 = 5;

const KEYSET_OPTIDX_CREATE_AUGROUP_CLEAR: u64 = 1;

const KEYSET_OPTIDX_EXEC_AUTOCMDS_BUF: u64 = 1;
const KEYSET_OPTIDX_EXEC_AUTOCMDS_DATA: u64 = 2;
const KEYSET_OPTIDX_EXEC_AUTOCMDS_BUFFER: u64 = 4;
const KEYSET_OPTIDX_EXEC_AUTOCMDS_PATTERN: u64 = 5;
const KEYSET_OPTIDX_EXEC_AUTOCMDS_MODELINE: u64 = 6;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_get_autocmds {
    pub is_set_get_autocmds: OptionalKeys,
    pub event: Object,
    pub group: Object,
    pub pattern: Object,
    pub buffer: Object,
    pub buf: Object,
    pub id: Integer,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_create_autocmd {
    pub is_set_create_autocmd: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub callback: Object,
    pub command: NvimString,
    pub desc: NvimString,
    pub group: Object,
    pub nested: bool,
    pub once: bool,
    pub pattern: Object,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_clear_autocmds {
    pub is_set_clear_autocmds: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub event: Object,
    pub group: Object,
    pub pattern: Object,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_create_augroup {
    pub is_set_create_augroup: OptionalKeys,
    pub clear: bool,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_exec_autocmds {
    pub is_set_exec_autocmds: OptionalKeys,
    pub buffer: Buffer,
    pub buf: Buffer,
    pub group: Object,
    pub modeline: bool,
    pub pattern: Object,
    pub data: Object,
}

std::thread_local! {
    static NEXT_AUTOCMD_ID: std::cell::Cell<i64> = const { std::cell::Cell::new(1) };
}

#[inline]
fn api_err_con(err: &mut Error, name: &std::ffi::CStr, name2: &std::ffi::CStr) {
    api_err_conflict(err, name, name2);
}

#[inline]
fn api_err_inv(err: &mut Error, name: &std::ffi::CStr, val_s: &std::ffi::CStr) {
    api_err_invalid(err, name, Some(val_s), 0, true);
}

#[inline]
fn api_err_int(err: &mut Error, name: &std::ffi::CStr, val_n: i64) {
    api_err_invalid(err, name, None, val_n, false);
}

#[inline]
fn api_err_req(err: &mut Error, name: &std::ffi::CStr) {
    api_err_required(err, name);
}

#[inline]
fn format_buflocal_arena(handle: i32) -> NvimString {
    let s = format!("<buffer={handle}>");
    NvimString::from_bytes(s.as_bytes())
}

fn unpack_string_or_array(v: Object, k: &std::ffi::CStr, required: bool, err: &mut Error) -> Array {
    if v.obj_type == ObjectType::ObjectTypeString {
        let mut arr = Array::with_capacity(1);
        add_c(&mut arr, v);
        arr
    } else if v.obj_type == ObjectType::ObjectTypeArray {
        if !check_string_array(v.data.array(), k, true, err) {
            return Array::new();
        }
        v.data.array()
    } else {
        if !(!required && v.obj_type == ObjectType::ObjectTypeNil) {
            api_err_exp(err, k, c"Array or String", Some(api_typename(v.obj_type)));
        }
        Array::new()
    }
}

fn get_augroup_from_object(group: Object, err: &mut Error) -> i32 {
    match group.obj_type {
        ObjectType::ObjectTypeNil => AUGROUP_DEFAULT,
        ObjectType::ObjectTypeString => {
            let group_cstr = group.data.as_cstr();
            let au_group = augroup_find(group_cstr);
            if au_group == AUGROUP_ERROR {
                api_err_inv(err, c"group", group_cstr);
                return AUGROUP_ERROR;
            }
            au_group
        }
        ObjectType::ObjectTypeInteger => {
            let au_group = group.data.integer() as i32;
            let name = if au_group == 0 {
                None
            } else {
                augroup_name(au_group)
            };
            if !augroup_exists(name.as_deref()) {
                api_err_int(err, c"group", i64::from(au_group));
                return AUGROUP_ERROR;
            }
            au_group
        }
        _ => {
            api_err_exp(
                err,
                c"group",
                c"String or Integer",
                Some(api_typename(group.obj_type)),
            );
            AUGROUP_ERROR
        }
    }
}

fn get_patterns_from_pattern_or_buf(
    pattern: Object,
    has_buf: bool,
    buf: Buffer,
    fallback: &std::ffi::CStr,
    err: &mut Error,
) -> Array {
    let mut patterns = ArrayBuilder::new();

    if pattern.obj_type != ObjectType::ObjectTypeNil {
        if pattern.obj_type == ObjectType::ObjectTypeString {
            let pat_str = pattern.data.string();
            let mut rem = pat_str.cstr_bytes();
            while let Some((span, rest)) = aucmd_span_pattern_slice(rem) {
                patterns.push(string_obj(NvimString::from_bytes(span)));
                rem = rest;
            }
        } else if pattern.obj_type == ObjectType::ObjectTypeArray {
            if !check_string_array(pattern.data.array(), c"pattern", true, err) {
                return Array::new();
            }
            let array = pattern.data.array();
            for i in 0..array.size {
                let entry_str = array[i].data.string();
                let mut rem = entry_str.cstr_bytes();
                while let Some((span, rest)) = aucmd_span_pattern_slice(rem) {
                    patterns.push(string_obj(NvimString::from_bytes(span)));
                    rem = rest;
                }
            }
        } else {
            api_err_exp(
                err,
                c"pattern",
                c"String or Table",
                Some(api_typename(pattern.obj_type)),
            );
            return Array::new();
        }
    } else if has_buf {
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle(buf, err) else {
            return Array::new();
        };
        patterns.push(string_obj(format_buflocal_arena(b.b_fnum.get())));
    } else {
        patterns.push(string_obj(cstr_as_string(Some(fallback))));
    }

    crate::api::private::helpers::take_arraybuilder(&mut patterns)
}

fn clear_autocmd(event: AuEvent, pat: &std::ffi::CStr, au_group: i32, err: &mut Error) -> bool {
    if !do_autocmd_event(event, pat, false, 0, c"", true, au_group) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Failed to clear autocmd",
        );
        return false;
    }
    true
}

/// Gets all autocommands matching ALL criteria in the {opts} query.
///
/// Note: When multiple patterns or events are provided they have "OR" semantics (any combination
/// is matched).
///
/// Examples:
///
/// ```lua
/// -- Matches all criteria
/// autocommands = vim.api.nvim_get_autocmds({
///   group = 'MyGroup',
///   event = {'BufEnter', 'BufWinEnter'},
///   pattern = {'*.c', '*.h'}
/// })
///
/// -- All commands from one group
/// autocommands = vim.api.nvim_get_autocmds({
///   group = 'MyGroup',
/// })
/// ```
///
/// @param opts Dict with at least one of these keys:
///        - buf: (`integer[]|integer?`) Buffer id or list of buffer ids, for buffer-local autocommands
///          |autocmd-buflocal|. Not allowed with {pattern}.
///        - event: (`vim.api.keyset.events|vim.api.keyset.events[]?`) Event(s) to match |autocmd-events|.
///        - group: (`string|table?`) Group name or id to match.
///        - id: (`integer?`) Autocommand ID to match.
///        - pattern: (`string|table?`) Pattern(s) to match |autocmd-pattern|. Not allowed with {buf}.
/// @return Array of matching autocommands, where each item has:
///         - buf (`integer?`): Buffer id (only for |autocmd-buffer-local|).
///         - buflocal (`boolean?`): true if the autocommand is buffer-local |autocmd-buffer-local|.
///         - callback: (`function|string?`): Event handler: a Lua function or Vimscript function name.
///         - command: (`string`) Event handler: an Ex-command. Empty if a `callback` is set.
///         - desc: (`string`) Description.
///         - event: (`vim.api.keyset.events`) Event name(s).
///         - group: (`integer`) Group id.
///         - group_name: (`string`) Group name.
///         - id: (`integer`) Autocommand id (only when defined with the API).
///         - once: (`boolean`) true if |autocmd-once| was set.
///         - pattern: (`string`) Autocommand pattern.
// DLLEXPORT ArrayOf(DictAs(get_autocmds__ret)) nvim_get_autocmds(Dict(get_autocmds) *opts,
//                                                                Arena *arena,
//                                                                Error *err) FUNC_API_SINCE(9);
pub fn nvim_get_autocmds(opts: &mut KeyDict_get_autocmds, err: &mut Error) -> Array {
    let mut autocmd_list = ArrayBuilder::new();
    let mut pattern_filters: [Option<&CStr>; AUCMD_MAX_PATTERNS] = [None; AUCMD_MAX_PATTERNS];
    let mut buffers = Array::new();
    let mut event_set = [false; NUM_EVENTS];
    let mut check_event = false;
    let mut group: i32 = 0;

    'body: {
        match opts.group.obj_type {
            ObjectType::ObjectTypeNil => {}
            ObjectType::ObjectTypeString => {
                let group_cstr = opts.group.data.as_cstr();
                group = augroup_find(group_cstr);
                if group < 0 {
                    api_err_inv(err, c"group", group_cstr);
                    break 'body;
                }
            }
            ObjectType::ObjectTypeInteger => {
                group = opts.group.data.integer() as i32;
                let name = if group == 0 {
                    None
                } else {
                    augroup_name(group)
                };
                if !augroup_exists(name.as_deref()) {
                    api_err_int(err, c"group", opts.group.data.integer());
                    break 'body;
                }
            }
            _ => {
                api_err_exp(
                    err,
                    c"group",
                    c"String or Integer",
                    Some(api_typename(opts.group.obj_type)),
                );
                break 'body;
            }
        }

        let mask = opts.is_set_get_autocmds;
        let id: i32 = if has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_ID) {
            opts.id as i32
        } else {
            -1
        };

        if has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_EVENT) {
            check_event = true;
            let v = &opts.event;
            if v.obj_type == ObjectType::ObjectTypeString {
                let event_nr = event_name2nr_str(v.data.string());
                if (event_nr as usize) >= NUM_EVENTS {
                    api_err_inv(err, c"event", v.data.as_cstr());
                    break 'body;
                }
                event_set[event_nr as usize] = true;
            } else if v.obj_type == ObjectType::ObjectTypeArray {
                for event_v in v.data.array_slice() {
                    if event_v.obj_type != ObjectType::ObjectTypeString {
                        api_err_exp(
                            err,
                            c"event item",
                            api_typename(ObjectType::ObjectTypeString),
                            Some(api_typename(event_v.obj_type)),
                        );
                        break 'body;
                    }
                    let event_nr = event_name2nr_str(event_v.data.string());
                    if (event_nr as usize) >= NUM_EVENTS {
                        api_err_inv(err, c"event", event_v.data.as_cstr());
                        break 'body;
                    }
                    event_set[event_nr as usize] = true;
                }
            } else {
                api_err_exp(err, c"event", c"String or Array", None);
                break 'body;
            }
        }

        let has_buf_key = has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_BUF);
        let has_buffer_key = has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_BUFFER);
        let has_buf = has_buf_key || has_buffer_key;
        let buf = if has_buf_key { &opts.buf } else { &opts.buffer };

        if has_buf_key && has_buffer_key {
            api_err_con(err, c"buf", c"buffer");
            break 'body;
        }
        if has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_PATTERN) && has_buf {
            api_err_con(err, c"pattern", c"buf");
            break 'body;
        }

        let mut pattern_filter_count = 0usize;
        if has_key(mask, KEYSET_OPTIDX_GET_AUTOCMDS_PATTERN) {
            let v = &opts.pattern;
            if v.obj_type == ObjectType::ObjectTypeString {
                pattern_filters[pattern_filter_count] = v.data.as_opt_cstr();
                pattern_filter_count += 1;
            } else if v.obj_type == ObjectType::ObjectTypeArray {
                let arr_slice = v.data.array_slice();
                if arr_slice.len() > AUCMD_MAX_PATTERNS {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"Too many patterns (maximum of %d)",
                        AUCMD_MAX_PATTERNS as i32,
                    );
                    break 'body;
                }
                for item in arr_slice {
                    if item.obj_type != ObjectType::ObjectTypeString {
                        api_err_exp(
                            err,
                            c"pattern",
                            api_typename(ObjectType::ObjectTypeString),
                            Some(api_typename(item.obj_type)),
                        );
                        break 'body;
                    }
                    pattern_filters[pattern_filter_count] = item.data.as_opt_cstr();
                    pattern_filter_count += 1;
                }
            } else {
                api_err_exp(
                    err,
                    c"pattern",
                    c"String or Array",
                    Some(api_typename(v.obj_type)),
                );
                break 'body;
            }
        }

        if buf.obj_type == ObjectType::ObjectTypeInteger
            || buf.obj_type == ObjectType::ObjectTypeBuffer
        {
            let Some(b) = crate::api::private::helpers::find_buffer_by_handle(
                buf.data.integer() as Buffer,
                err,
            ) else {
                break 'body;
            };
            let pat = format_buflocal_arena(b.b_fnum.get());
            buffers = Array::with_capacity(1);
            add_c(&mut buffers, string_obj(pat));
        } else if buf.obj_type == ObjectType::ObjectTypeArray {
            let buf_slice = buf.data.array_slice();
            if buf_slice.len() > AUCMD_MAX_PATTERNS {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Too many buffers (maximum of %d)",
                    AUCMD_MAX_PATTERNS as i32,
                );
                break 'body;
            }
            buffers = Array::with_capacity(buf_slice.len());
            for bufnr in buf_slice {
                if bufnr.obj_type != ObjectType::ObjectTypeInteger
                    && bufnr.obj_type != ObjectType::ObjectTypeBuffer
                {
                    api_err_exp(
                        err,
                        c"buffer",
                        c"Integer",
                        Some(api_typename(bufnr.obj_type)),
                    );
                    break 'body;
                }
                let Some(b) = crate::api::private::helpers::find_buffer_by_handle(
                    bufnr.data.integer() as Buffer,
                    err,
                ) else {
                    break 'body;
                };
                add_c(
                    &mut buffers,
                    string_obj(format_buflocal_arena(b.b_fnum.get())),
                );
            }
        } else if has_buf {
            api_err_exp(
                err,
                c"buffer",
                c"Integer or Array",
                Some(api_typename(buf.obj_type)),
            );
            break 'body;
        }

        for bufnr in buffers.as_slice() {
            pattern_filters[pattern_filter_count] = bufnr.data.as_opt_cstr();
            pattern_filter_count += 1;
        }

        for (event, &has_ev) in event_set.iter().enumerate().take(NUM_EVENTS) {
            if check_event && !has_ev {
                continue;
            }

            with_autocmds_for_event(event as AuEvent, |acs| {
                for ac in acs {
                    let Some(ref ap) = ac.pat else {
                        continue;
                    };
                    if id != -1 && ac.id != i64::from(id) {
                        continue;
                    }
                    if group != 0 && ap.group != group {
                        continue;
                    }

                    if pattern_filter_count > 0 {
                        let mut passed = false;
                        for &raw_pat in &pattern_filters[..pattern_filter_count] {
                            let Some(raw_cstr) = raw_pat else {
                                continue;
                            };
                            let mut pat_cstr = raw_cstr;
                            let mut pattern_buflocal = [0u8; BUFLOCAL_PAT_LEN];
                            let pat_bytes = pat_cstr.to_bytes();
                            if aupat_is_buflocal(pat_bytes) {
                                aupat_normalize_buflocal_pat(
                                    &mut pattern_buflocal,
                                    pat_bytes,
                                    aupat_get_buflocal_nr(pat_bytes),
                                );
                                pat_cstr =
                                    CStr::from_bytes_until_nul(&pattern_buflocal).unwrap_or(c"");
                            }
                            if ap.pat.as_bytes() == pat_cstr.to_bytes() {
                                passed = true;
                                break;
                            }
                        }
                        if !passed {
                            continue;
                        }
                    }

                    let mut autocmd_info = Dict::with_capacity(12);

                    if ap.group != AUGROUP_DEFAULT {
                        put_c(
                            &mut autocmd_info,
                            c"group",
                            integer_obj(i64::from(ap.group)),
                        );
                        let gname = augroup_name(ap.group);
                        put_c(
                            &mut autocmd_info,
                            c"group_name",
                            crate::api::private::helpers::cstr_to_nonnull_obj(gname.as_deref()),
                        );
                    }

                    if ac.id > 0 {
                        put_c(&mut autocmd_info, c"id", integer_obj(ac.id));
                    }

                    if let Some(desc) = &ac.desc {
                        put_c(
                            &mut autocmd_info,
                            c"desc",
                            crate::api::private::helpers::cstr_to_nonnull_obj(Some(desc)),
                        );
                    }

                    match &mut ac.handler {
                        crate::autocmd::AutoCmdHandler::Cmd(cmd) => {
                            put_c(
                                &mut autocmd_info,
                                c"command",
                                crate::api::private::helpers::cstr_to_nonnull_obj(Some(
                                    cmd.as_c_str(),
                                )),
                            );
                        }
                        crate::autocmd::AutoCmdHandler::Fn(cb) => {
                            put_c(&mut autocmd_info, c"command", string_obj(NvimString::new()));

                            match cb.cb_type() {
                                K_CALLBACK_LUA => {
                                    let luaref = cb.data.as_luaref();
                                    if nlua_ref_is_function(luaref) {
                                        put_c(
                                            &mut autocmd_info,
                                            c"callback",
                                            luaref_obj(api_new_luaref(luaref)),
                                        );
                                    }
                                }
                                K_CALLBACK_FUNCREF | K_CALLBACK_PARTIAL => {
                                    let cb_str = callback_to_string(cb);
                                    put_c(
                                        &mut autocmd_info,
                                        c"callback",
                                        string_obj(
                                            crate::api::private::helpers::ensure_nul_string(
                                                cstr_as_string(Some(&*cb_str)),
                                            ),
                                        ),
                                    );
                                }
                                _ => {
                                    std::process::abort();
                                }
                            }
                        }
                        crate::autocmd::AutoCmdHandler::None => {
                            std::process::abort();
                        }
                    }

                    put_c(
                        &mut autocmd_info,
                        c"pattern",
                        crate::api::private::helpers::cstr_to_nonnull_obj(Some(&*ap.pat)),
                    );
                    put_c(
                        &mut autocmd_info,
                        c"event",
                        crate::api::private::helpers::cstr_to_nonnull_obj(Some(event_nr2name(
                            event as AuEvent,
                        ))),
                    );
                    put_c(&mut autocmd_info, c"once", boolean_obj(ac.once));

                    if ap.buflocal_nr != 0 {
                        put_c(&mut autocmd_info, c"buflocal", boolean_obj(true));
                        put_c(
                            &mut autocmd_info,
                            c"buf",
                            integer_obj(i64::from(ap.buflocal_nr)),
                        );
                        put_c(
                            &mut autocmd_info,
                            c"buffer",
                            integer_obj(i64::from(ap.buflocal_nr)),
                        );
                    } else {
                        put_c(&mut autocmd_info, c"buflocal", boolean_obj(false));
                    }

                    autocmd_list.push(dict_obj(autocmd_info));
                }
            });
        }
    }

    crate::api::private::helpers::take_arraybuilder(&mut autocmd_list)
}

/// Creates an |autocommand| event handler, defined by `callback` (Lua function or Vimscript
/// function _name_ string) or `command` (Ex command string).
///
/// Example using Lua callback:
///
/// ```lua
/// vim.api.nvim_create_autocmd({'BufEnter', 'BufWinEnter'}, {
///   pattern = {'*.c', '*.h'},
///   callback = function(ev)
///     print(string.format('event fired: %s', vim.inspect(ev)))
///   end
/// })
/// ```
///
/// Example using an Ex command as the handler:
///
/// ```lua
/// vim.api.nvim_create_autocmd({'BufEnter', 'BufWinEnter'}, {
///   pattern = {'*.c', '*.h'},
///   command = "echo 'Entering a C or C++ file'",
/// })
/// ```
///
/// Note: `pattern` is NOT automatically expanded (unlike with |:autocmd|), thus names like "$HOME"
/// and "~" must be expanded explicitly:
///
/// ```lua
/// pattern = vim.fn.expand('~') .. '/some/path/*.py'
/// ```
///
/// @param event Event(s) that will trigger the handler (`callback` or `command`): one handler is
///        created for each event name.
/// @param opts Options dict:
///        - buf (`integer?`) Buffer id for buffer-local autocommands |autocmd-buflocal|.
///          Not allowed with {pattern}.
///        - callback (`function|string?`) Lua function (or Vimscript function name, if string)
///          called when the event(s) is triggered. Lua callback can return |lua-truthy| to delete
///          the autocommand. Callback receives one argument, a table with keys: [event-args]()
///            - buf: (`integer`) [<abuf>]
///            - data: (`any`) Arbitrary data passed from [nvim_exec_autocmds()] [event-data]()
///            - event: (`vim.api.keyset.events`) Name of the triggered event |autocmd-events|
///            - file: (`string`) [<afile>] (not expanded to a full path)
///            - group: (`integer?`) Group id, if any
///            - id: (`integer`) Autocommand id
///            - match: (`string`) [<amatch>] (expanded to a full path)
///            - win: (`integer`) id of the window for the event, see |window-ID|
///        - command (string?) Vim command executed on event. Not allowed with {callback}.
///        - desc (`string?`) Description (for documentation and troubleshooting).
///        - group (`string|integer?`) Group name or id to match against.
///        - nested (`boolean?`, default: false) Run nested autocommands |autocmd-nested|.
///        - once (`boolean?`, default: false) Handle the event only once |autocmd-once|. If {event}
///          is a list, each handler will fire once.
///        - pattern (`string|array?`) Pattern(s) to match literally |autocmd-pattern|.
///
/// @return Autocommand id (number)
/// @see |autocommand|
/// @see |nvim_del_autocmd()|
// DLLEXPORT Integer nvim_create_autocmd(uint64_t channel_id, Object event,
//                                       Dict(create_autocmd) *opts, Arena *arena,
//                                       Error *err) FUNC_API_SINCE(9) FUNC_API_FAST;
pub fn nvim_create_autocmd(
    channel_id: u64,
    event: Object,
    opts: &mut KeyDict_create_autocmd,
    err: &mut Error,
) -> Integer {
    let mut autocmd_id: i64 = -1;
    let mut desc: Option<&CStr> = None;
    let mut handler_cmd: Option<CString> = None;
    let mut handler_fn = Callback::NONE;

    'body: {
        let event_array = unpack_string_or_array(event, c"event", true, err);
        if error_set(err) {
            break 'body;
        }

        let mask = opts.is_set_create_autocmd;
        let has_callback = has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_CALLBACK);
        let has_command = has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_COMMAND);

        if has_callback && has_command {
            api_err_con(err, c"callback", c"command");
            break 'body;
        }

        if has_callback {
            let callback = &mut opts.callback;
            match callback.obj_type {
                ObjectType::ObjectTypeLuaRef => {
                    if callback.data.luaref() == LUA_NOREF {
                        api_err_inv(err, c"callback", c"<no value>");
                        break 'body;
                    }
                    if !nlua_ref_is_function(callback.data.luaref()) {
                        api_err_inv(err, c"callback", c"<not a function>");
                        break 'body;
                    }
                    handler_fn.data.set(crate::eval::typval::CallbackData::lua(
                        callback.data.luaref(),
                    ));
                    callback.data = crate::api::private::validate::ObjectData::LuaRef(LUA_NOREF);
                }
                ObjectType::ObjectTypeString => {
                    let s = bytes_to_cstring(callback.data.string().as_bytes());
                    handler_fn
                        .data
                        .set(crate::eval::typval::CallbackData::funcref(s));
                }
                _ => {
                    api_err_exp(
                        err,
                        c"callback",
                        c"Lua function or Vim function name",
                        Some(api_typename(callback.obj_type)),
                    );
                    break 'body;
                }
            }
        } else if has_command {
            handler_cmd = Some(bytes_to_cstring(opts.command.as_bytes()));
        } else {
            api_err_req(err, c"'command' or 'callback'");
            break 'body;
        }

        let au_group = get_augroup_from_object(opts.group.clone(), err);
        if au_group == AUGROUP_ERROR {
            break 'body;
        }

        let has_buf_key = has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_BUF);
        let has_buffer_key = has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_BUFFER);
        let has_buf = has_buf_key || has_buffer_key;
        let buf = if has_buf_key { opts.buf } else { opts.buffer };

        if has_buf_key && has_buffer_key {
            api_err_con(err, c"buf", c"buffer");
            break 'body;
        }
        if has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_PATTERN) && has_buf {
            api_err_con(err, c"pattern", c"buf");
            break 'body;
        }

        let patterns =
            get_patterns_from_pattern_or_buf(opts.pattern.clone(), has_buf, buf, c"*", err);
        if error_set(err) {
            break 'body;
        }
        if patterns.size == 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"No non-empty patterns specified",
            );
            break 'body;
        }

        if has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_DESC) {
            desc = opts.desc.as_opt_cstr();
        }

        if event_array.size == 0 {
            api_err_req(err, c"event");
            break 'body;
        }

        autocmd_id = NEXT_AUTOCMD_ID.get();
        NEXT_AUTOCMD_ID.set(autocmd_id + 1);

        for event_str in event_array.as_slice() {
            let event_nr = event_name2nr_str(event_str.data.string());
            if (event_nr as usize) >= NUM_EVENTS {
                api_err_inv(err, c"event", event_str.data.as_cstr());
                break 'body;
            }

            for pat in patterns.as_slice() {
                let save_current_sctx = api_set_sctx(channel_id);
                let retval = autocmd_register(
                    autocmd_id,
                    event_nr,
                    pat.data.as_bytes(),
                    au_group,
                    opts.once,
                    opts.nested,
                    desc,
                    handler_cmd.as_deref(),
                    &mut handler_fn,
                );
                g_st().current_sctx.set(save_current_sctx);

                if !retval {
                    api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to set autocmd",);
                    break 'body;
                }
            }
        }
    }

    if handler_fn.cb_type() == K_CALLBACK_LUA {
        callback_free(&handler_fn);
    }

    autocmd_id
}

/// Deletes an autocommand by id.
///
/// @param id Autocommand id returned by |nvim_create_autocmd()|
// DLLEXPORT void nvim_del_autocmd(Integer id, Error *err) FUNC_API_SINCE(9);
pub fn nvim_del_autocmd(id: Integer, err: &mut Error) {
    if id <= 0 {
        api_err_int(err, c"autocmd id", id);
        return;
    }
    if !autocmd_delete_id(id) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Failed to delete autocmd",
        );
    }
}

/// Clears all autocommands matching the {opts} query. To delete autocmds see |nvim_del_autocmd()|.
///
/// @param opts Optional parameters:
///        - buf: (`integer?`) Select |autocmd-buflocal| autocommands. Not allowed with {pattern}.
///        - event: (`vim.api.keyset.events|vim.api.keyset.events[]?`)
///          Examples:
///          - event: "pat1"
///          - event: { "pat1" }
///          - event: { "pat1", "pat2", "pat3" }
///        - group: (`string|int?`) Group name or id.
///          - NOTE: If not given, matches autocmds *not* in any group.
///        - pattern: (`string|table?`) Filter by patterns (exact match). Not allowed with {buf}.
///          - Example: if you have `*.py` as that pattern for the autocmd, you must pass `*.py`
///            exactly to clear it. `test.py` will not match the pattern.
///
// DLLEXPORT void nvim_clear_autocmds(Dict(clear_autocmds) *opts, Arena *arena,
//                                    Error *err) FUNC_API_SINCE(9);
pub fn nvim_clear_autocmds(opts: &mut KeyDict_clear_autocmds, err: &mut Error) {
    let event_array = unpack_string_or_array(opts.event.clone(), c"event", false, err);
    if error_set(err) {
        return;
    }

    let mask = opts.is_set_clear_autocmds;
    let has_buf_key = has_key(mask, KEYSET_OPTIDX_CLEAR_AUTOCMDS_BUF);
    let has_buffer_key = has_key(mask, KEYSET_OPTIDX_CLEAR_AUTOCMDS_BUFFER);
    let has_buf = has_buf_key || has_buffer_key;
    let buf = if has_buf_key { opts.buf } else { opts.buffer };

    if has_buf_key && has_buffer_key {
        api_err_con(err, c"buf", c"buffer");
        return;
    }
    if has_key(mask, KEYSET_OPTIDX_CLEAR_AUTOCMDS_PATTERN) && has_buf {
        api_err_con(err, c"pattern", c"buf");
        return;
    }

    let au_group = get_augroup_from_object(opts.group.clone(), err);
    if au_group == AUGROUP_ERROR {
        return;
    }

    let patterns = get_patterns_from_pattern_or_buf(opts.pattern.clone(), has_buf, buf, c"", err);
    if error_set(err) {
        return;
    }

    if opts.event.obj_type == ObjectType::ObjectTypeNil {
        for event in 0..NUM_EVENTS {
            for pat_object in patterns.as_slice() {
                if !clear_autocmd(event as AuEvent, pat_object.data.as_cstr(), au_group, err) {
                    return;
                }
            }
        }
    } else {
        for event_str in event_array.as_slice() {
            let event_nr = event_name2nr_str(event_str.data.string());
            if (event_nr as usize) >= NUM_EVENTS {
                api_err_inv(err, c"event", event_str.data.as_cstr());
                return;
            }
            for pat_object in patterns.as_slice() {
                if !clear_autocmd(event_nr, pat_object.data.as_cstr(), au_group, err) {
                    return;
                }
            }
        }
    }
}

/// Create or get an autocommand group |autocmd-groups|.
///
/// To get an existing group id, do:
///
/// ```lua
/// local id = vim.api.nvim_create_augroup('my.lsp.config', {
///   clear = false
/// })
/// ```
///
/// @param name Group name
/// @param opts Optional parameters:
///        - clear (`boolean?`, default: true) Clear existing commands in the group |autocmd-groups|.
/// @return Group id.
/// @see |autocmd-groups|
// DLLEXPORT Integer nvim_create_augroup(uint64_t channel_id, String name,
//                                       Dict(create_augroup) *opts, Error *err) FUNC_API_SINCE(9);
pub fn nvim_create_augroup(
    channel_id: u64,
    name: NvimString,
    opts: &mut KeyDict_create_augroup,
    err: &mut Error,
) -> Integer {
    let augroup_name = name;
    let clear_autocmds = if has_key(
        opts.is_set_create_augroup,
        KEYSET_OPTIDX_CREATE_AUGROUP_CLEAR,
    ) {
        opts.clear
    } else {
        true
    };

    let save_current_sctx = api_set_sctx(channel_id);
    let augroup = augroup_add(augroup_name.as_cstr());
    if augroup == AUGROUP_ERROR {
        api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to set augroup",);
        g_st().current_sctx.set(save_current_sctx);
        return -1;
    }

    if clear_autocmds {
        for event in 0..NUM_EVENTS {
            aucmd_del_for_event_and_group(event as AuEvent, augroup);
        }
    }
    g_st().current_sctx.set(save_current_sctx);

    i64::from(augroup)
}

/// Delete an autocommand group by id.
///
/// To get a group id one can use |nvim_get_autocmds()|.
///
/// NOTE: behavior differs from |:augroup-delete|. When deleting a group, autocommands contained in
/// this group will also be deleted and cleared. This group will no longer exist.
/// @param id Group id.
/// @see |nvim_del_augroup_by_name()|
/// @see |nvim_create_augroup()|
// DLLEXPORT void nvim_del_augroup_by_id(Integer id, Error *err) FUNC_API_SINCE(9);
pub fn nvim_del_augroup_by_id(id: Integer, err: &mut Error) {
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    augroup_del_by_id(id as i32);
    try_leave(&tstate, err);
}

/// Delete an autocommand group by name.
///
/// NOTE: behavior differs from |:augroup-delete|. When deleting a group, autocommands contained in
/// this group will also be deleted and cleared. This group will no longer exist.
/// @param name Group name.
/// @see |autocmd-groups|
// DLLEXPORT void nvim_del_augroup_by_name(String name, Error *err) FUNC_API_SINCE(9);
pub fn nvim_del_augroup_by_name(name: NvimString, err: &mut Error) {
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    augroup_del(name.as_cstr(), -1, false);
    try_leave(&tstate, err);
}

/// Executes {event} handlers matching the {opts} query, in the context of {buf} (if given). |autocmd-execute|
///
/// @param event Event(s) to execute.
/// @param opts Optional filters:
///        - buf (`integer?`) Buffer where the event is applied. |autocmd-buflocal| Not allowed with {pattern}.
///        - data (`any`): Arbitrary data passed to the callback. See |nvim_create_autocmd()|.
///        - group (`string|integer?`) Group name or id to match against. |autocmd-groups|.
///        - modeline (`boolean?`, default: true) Process the modeline after the autocommands
///          [<nomodeline>]. Ignored if `buf` is given.
///        - pattern (`string|array?`, default: current file name) |autocmd-pattern|. Not allowed with {buf}.
/// @see |:doautocmd|
// DLLEXPORT void nvim_exec_autocmds(Object event, Dict(exec_autocmds) *opts, Arena *arena,
//                                   Error *err) FUNC_API_SINCE(9);
pub fn nvim_exec_autocmds(event: Object, opts: &mut KeyDict_exec_autocmds, err: &mut Error) {
    let mut au_group = AUGROUP_ALL;
    let mut b = g_st().curbuf.get();

    let event_array = unpack_string_or_array(event, c"event", true, err);
    if error_set(err) {
        return;
    }

    match opts.group.obj_type {
        ObjectType::ObjectTypeNil => {}
        ObjectType::ObjectTypeString => {
            let group_cstr = opts.group.data.as_cstr();
            au_group = augroup_find(group_cstr);
            if au_group == AUGROUP_ERROR {
                api_err_inv(err, c"group", group_cstr);
                return;
            }
        }
        ObjectType::ObjectTypeInteger => {
            au_group = opts.group.data.integer() as i32;
            let name = if au_group == 0 {
                None
            } else {
                augroup_name(au_group)
            };
            if !augroup_exists(name.as_deref()) {
                api_err_int(err, c"group", i64::from(au_group));
                return;
            }
        }
        _ => {
            api_err_exp(
                err,
                c"group",
                c"String or Integer",
                Some(api_typename(opts.group.obj_type)),
            );
            return;
        }
    }

    let mask = opts.is_set_exec_autocmds;
    let has_buf_key = has_key(mask, KEYSET_OPTIDX_EXEC_AUTOCMDS_BUF);
    let has_buffer_key = has_key(mask, KEYSET_OPTIDX_EXEC_AUTOCMDS_BUFFER);
    let has_buf = has_buf_key || has_buffer_key;
    let buf = if has_buf_key { opts.buf } else { opts.buffer };

    if has_buf_key && has_buffer_key {
        api_err_con(err, c"buf", c"buffer");
        return;
    }

    if has_buf {
        if has_key(mask, KEYSET_OPTIDX_EXEC_AUTOCMDS_PATTERN) {
            api_err_con(err, c"pattern", c"buf");
            return;
        }
        b = crate::globals::BufPtr::from(find_buffer_by_handle(buf, err));
        if error_set(err) {
            return;
        }
    }

    let patterns = get_patterns_from_pattern_or_buf(opts.pattern.clone(), has_buf, buf, c"", err);
    if error_set(err) {
        return;
    }

    let data = has_key(mask, KEYSET_OPTIDX_EXEC_AUTOCMDS_DATA).then_some(&opts.data);

    let modeline = if has_key(mask, KEYSET_OPTIDX_EXEC_AUTOCMDS_MODELINE) {
        opts.modeline
    } else {
        true
    };

    let mut did_aucmd = false;
    for event_str in event_array.as_slice() {
        let event_nr = event_name2nr_str(event_str.data.string());
        if (event_nr as usize) >= NUM_EVENTS {
            api_err_inv(err, c"event", event_str.data.as_cstr());
            return;
        }
        for pat in patterns.as_slice() {
            let fname = (!has_buf).then(|| pat.data.as_opt_cstr()).flatten();
            did_aucmd |= apply_autocmds_group(
                event_nr,
                fname,
                None,
                true,
                au_group,
                b.opt(),
                Some(&crate::globals::curwin()),
                None,
                data,
                has_buf,
            );
        }
    }

    if did_aucmd && modeline && !has_buf {
        do_modelines(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYSET_OPTIDX_CREATE_AUTOCMD_ONCE: u64 = 12;

    #[test]
    fn has_key_checks_bitmask() {
        let mask = (1u64 << KEYSET_OPTIDX_CREATE_AUTOCMD_ONCE)
            | (1u64 << KEYSET_OPTIDX_CREATE_AUTOCMD_CALLBACK);
        assert!(has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_ONCE));
        assert!(has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_CALLBACK));
        assert!(!has_key(mask, KEYSET_OPTIDX_CREATE_AUTOCMD_COMMAND));
    }
}
