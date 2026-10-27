//! Deprecated API functions (`src/nvim/api/deprecated.c`).

use crate::api::buffer::{nvim_buf_get_lines, nvim_buf_set_lines};
use crate::api::extmark::{nvim_buf_clear_namespace, nvim_create_namespace, parse_virt_text};
use crate::api::private::dispatch::msgpack_rpc_get_handler_for;
use crate::api::private::helpers::{
    MAXLNUM, OBJECT_NIL, add_c, api_clear_error, api_free_object, api_set_error, api_typename,
    arena_array, array_obj, copy_object, copy_string, cstr_as_string, dict_obj, dict_set_var,
    error_set, find_buffer_by_handle, find_tab_by_handle, find_window_by_handle, integer_obj,
    static_cstr_as_string, string_obj, try_wrap, with_script_context,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType, api_err_exp, api_err_invalid,
    api_err_range,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::api::vimscript::{KeyDict_exec_opts, exec_impl};
use crate::charset::{NL, NUL};
use crate::cursor::colnr_T;
use crate::decoration::{
    DECOR_ID_INVALID, DecorInline, DecorVirtText, DecorVirtTextData, decor_find_virttext,
    kHlModeUnknown, kVPosEndOfLine,
};
use crate::eval::vars::MAXCOL;
use crate::eval::vars::get_globvar_dict;
use crate::extmark::extmark_set;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::highlight_group::{syn_check_group, syn_get_final_id, syn_id2attr, syn_name2id};
use crate::lua::converter::Boolean;
use crate::lua::executor::{kRetObject, nlua_exec};
use crate::marktree::{DECOR_HIGHLIGHT_INLINE_INIT, DecorExt, DecorInlineData, MT_FLAG_DECOR_HL};
use crate::memory::Arena;
use crate::message::{emsg, msg, msg_end, msg_ext_no_fast};
use crate::msgpack_rpc::packer::Integer;
use crate::option::{
    OPT_GLOBAL, OPT_LOCAL, OptScope, OptTarget, find_option, get_option_info, get_option_value_for,
    kOptInvalid, kOptScopeBuf, kOptScopeGlobal, kOptScopeWin, option_has_scope,
    set_option_value_for,
};
use crate::window::{win_setheight_win, win_setwidth_win};
use std::ffi::{c_char, c_int};
use std::ptr;

const LINE_BUFFER_MIN_SIZE: usize = 4096;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_empty {
    pub is_set__empty_: OptionalKeys,
}

struct ApiDeprecatedState {
    OUT_LINE_BUF: Vec<c_char>,
    ERR_LINE_BUF: Vec<c_char>,
}

impl ApiDeprecatedState {
    const fn new() -> Self {
        Self {
            OUT_LINE_BUF: Vec::new(),
            ERR_LINE_BUF: Vec::new(),
        }
    }
}

static API_DEP_STATE: crate::globals::GlobalState<ApiDeprecatedState> = crate::globals::GlobalState::new(ApiDeprecatedState::new());

#[inline]
fn api_dep_st() -> &'static mut ApiDeprecatedState {
    unsafe { &mut *API_DEP_STATE.as_ptr() }
}

/// @deprecated Use nvim_exec2() instead.
/// @see nvim_exec2
// DLLEXPORT String nvim_exec(uint64_t channel_id, String src, Boolean output, Error *err)
//   FUNC_API_SINCE(7) FUNC_API_DEPRECATED_SINCE(11)
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_exec(
    channel_id: u64,
    src: NvimString,
    output: Boolean,
    err: &mut Error,
) -> NvimString {
    let mut opts = KeyDict_exec_opts { output };
        exec_impl(channel_id, src, &mut opts, err)
}

/// @deprecated
/// @see nvim_exec2
// DLLEXPORT String nvim_command_output(uint64_t channel_id, String command, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(7)
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_command_output(
    channel_id: u64,
    command: NvimString,
    err: &mut Error,
) -> NvimString {
    let mut opts = KeyDict_exec_opts { output: true };
        exec_impl(channel_id, command, &mut opts, err)
}

/// @deprecated Use nvim_exec_lua() instead.
/// @see nvim_exec_lua
// DLLEXPORT Object nvim_execute_lua(String code, Array args, Arena *arena, Error *err)
//   FUNC_API_SINCE(3)
//   FUNC_API_DEPRECATED_SINCE(7)
//   FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_execute_lua(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe { nlua_exec(code, ptr::null(), args, kRetObject, arena, err) }
}

/// Gets the buffer number
///
/// @deprecated The buffer number now is equal to the object id
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param[out] err   Error details, if any
/// @return Buffer number
// DLLEXPORT Integer nvim_buf_get_number(Buffer buffer, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(2);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_number(buffer: Buffer, err: &mut Error) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }
        i64::from((*buf).b_fnum)
    }
}

unsafe fn src2ns(src_id: *mut Integer) -> u32 {
    unsafe {
        if *src_id == 0 {
            *src_id = nvim_create_namespace(NvimString {
                data: ptr::null_mut(),
                size: 0,
            });
        }
        if *src_id < 0 {
            return (1u32 << 31) - 1;
        }
        *src_id as u32
    }
}

/// Clears highlights and virtual text from namespace and range of lines
///
/// @deprecated use |nvim_buf_clear_namespace()|.
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param ns_id      Namespace to clear, or -1 to clear all.
/// @param line_start Start of range of lines to clear
/// @param line_end   End of range of lines to clear (exclusive) or -1 to clear
///                   to end of file.
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_clear_highlight(Buffer buffer, Integer ns_id, Integer line_start,
//                                         Integer line_end, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(7);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_clear_highlight(
    buffer: Buffer,
    ns_id: Integer,
    line_start: Integer,
    line_end: Integer,
    err: &mut Error,
) {
    unsafe {
        nvim_buf_clear_namespace(buffer, ns_id, line_start, line_end, err);
    }
}

/// Adds a highlight to buffer.
///
/// @deprecated use |nvim_buf_set_extmark()| or |vim.hl.range()|
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param ns_id      namespace to use or -1 for ungrouped highlight
/// @param hl_group   Name of the highlight group to use
/// @param line       Line to highlight (zero-indexed)
/// @param col_start  Start of (byte-indexed) column range to highlight
/// @param col_end    End of (byte-indexed) column range to highlight,
///                   or -1 to highlight to end of line
/// @param[out] err   Error details, if any
/// @return The ns_id that was used
// DLLEXPORT Integer nvim_buf_add_highlight(Buffer buffer, Integer ns_id, String hl_group,
//                                          Integer line, Integer col_start, Integer col_end,
//                                          Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(13);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_add_highlight(
    buffer: Buffer,
    mut ns_id: Integer,
    hl_group: NvimString,
    line: Integer,
    col_start: Integer,
    mut col_end: Integer,
    err: &mut Error,
) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }

        if !(0..MAXLNUM).contains(&line) {
            api_err_range(err, c"line number".as_ptr());
            return 0;
        }
        if !(0..=MAXCOL).contains(&col_start) {
            api_err_range(err, c"column".as_ptr());
            return 0;
        }

        if !(0..=MAXCOL).contains(&col_end) {
            col_end = MAXCOL;
        }

        let ns = src2ns(&mut ns_id);

        if line >= i64::from((*buf).b_ml.ml_line_count) {
            return ns_id;
        }

        let hl_id = if hl_group.size > 0 {
            syn_check_group(hl_group.data, hl_group.size)
        } else {
            return ns_id;
        };

        let mut end_line = line as c_int;
        if col_end == MAXCOL {
            col_end = 0;
            end_line += 1;
        }

        let mut decor = DecorInline {
            data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
        };
        decor.data.hl_mut().hl_id = hl_id;

        extmark_set(
            &mut *buf,
            ns,
            ptr::null_mut(),
            line as c_int,
            col_start as colnr_T,
            end_line,
            col_end as colnr_T,
            decor,
            MT_FLAG_DECOR_HL,
            true,
            false,
            false,
            false,
            ptr::null_mut(),
        );
        ns_id
    }
}

/// Set the virtual text (annotation) for a buffer line.
///
/// @deprecated use nvim_buf_set_extmark to use full virtual text functionality.
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param src_id     Namespace to use or 0 to create a namespace,
///                   or -1 for a ungrouped annotation
/// @param line       Line to annotate with virtual text (zero-indexed)
/// @param chunks     A list of [text, hl_group] arrays, each representing a
///                   text chunk with specified highlight. `hl_group` element
///                   can be omitted for no highlight.
/// @param opts       Optional parameters. Currently not used.
/// @param[out] err   Error details, if any
/// @return The ns_id that was used
// DLLEXPORT Integer nvim_buf_set_virtual_text(Buffer buffer, Integer src_id, Integer line,
//                                             Array chunks, Dict(empty) *opts, Error *err)
//   FUNC_API_SINCE(5)
//   FUNC_API_DEPRECATED_SINCE(8);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_virtual_text(
    buffer: Buffer,
    mut src_id: Integer,
    line: Integer,
    chunks: Array,
    _opts: *mut KeyDict_empty,
    err: &mut Error,
) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }

        if !(0..MAXLNUM).contains(&line) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Line number outside range".as_ptr(),
            );
            return 0;
        }

        let ns_id = src2ns(&mut src_id);
        let mut width: c_int = 0;

        let virt_text = parse_virt_text(chunks, err, &mut width, false);
        if error_set(err) {
            return 0;
        }

        let existing = decor_find_virttext(&mut *buf, line as c_int, u64::from(ns_id));
        if !existing.is_null() {
            (*existing).data = DecorVirtTextData::VirtText(virt_text);
            (*existing).width = width;
            return src_id;
        }

        let vt = crate::decoration::decor_put_vt(
            DecorVirtText {
                flags: 0,
                hl_mode: kHlModeUnknown,
                priority: 0,
                width,
                col: 0,
                pos: kVPosEndOfLine,
                data: DecorVirtTextData::VirtText(virt_text),
                next: None,
            },
            ptr::null_mut(),
        );

        let decor = DecorInline {
            data: DecorInlineData::Ext(DecorExt {
                sh_idx: DECOR_ID_INVALID,
                vt: vt.cast(),
            }),
        };

        extmark_set(
            &mut *buf,
            ns_id,
            ptr::null_mut(),
            line as c_int,
            0,
            -1,
            -1,
            decor,
            0,
            true,
            false,
            false,
            false,
            ptr::null_mut(),
        );
        src_id
    }
}

/// Gets a highlight definition by id. |hlID()|
///
/// @deprecated use |nvim_get_hl()| instead
///
/// @param hl_id Highlight id as returned by |hlID()|
/// @param rgb Export RGB colors
/// @param[out] err Error details, if any
/// @return Highlight definition map
/// @see nvim_get_hl_by_name
// DLLEXPORT Dict nvim_get_hl_by_id(Integer hl_id, Boolean rgb, Arena *arena, Error *err)
//   FUNC_API_SINCE(3)
//   FUNC_API_DEPRECATED_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_hl_by_id(
    hl_id: Integer,
    rgb: Boolean,
    arena: *mut Arena,
    err: &mut Error,
) -> Dict {
    unsafe {
        let dic = Dict::new();
        if syn_get_final_id(hl_id as c_int) == 0 {
            api_err_invalid(err, c"highlight id".as_ptr(), ptr::null(), hl_id, false);
            return dic;
        }
        let attrcode = syn_id2attr(hl_id as c_int);
        crate::highlight::hl_get_attr_by_id(i64::from(attrcode), rgb, arena, err)
    }
}

/// Gets a highlight definition by name.
///
/// @deprecated use |nvim_get_hl()| instead
///
/// @param name Highlight group name
/// @param rgb Export RGB colors
/// @param[out] err Error details, if any
/// @return Highlight definition map
/// @see nvim_get_hl_by_id
// DLLEXPORT Dict nvim_get_hl_by_name(String name, Boolean rgb, Arena *arena, Error *err)
//   FUNC_API_SINCE(3)
//   FUNC_API_DEPRECATED_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_hl_by_name(
    name: NvimString,
    rgb: Boolean,
    arena: *mut Arena,
    err: &mut Error,
) -> Dict {
    unsafe {
        let result = Dict::new();
        let id = syn_name2id(name.data);
        if id == 0 {
            api_err_invalid(err, c"highlight name".as_ptr(), name.data, 0, true);
            return result;
        }
        nvim_get_hl_by_id(i64::from(id), rgb, arena, err)
    }
}

/// Inserts a sequence of lines to a buffer at a certain index
///
/// @deprecated use nvim_buf_set_lines(buffer, lnum, lnum, true, lines)
///
/// @param buffer     Buffer id
/// @param lnum       Insert the lines after `lnum`. If negative, appends to
///                   the end of the buffer.
/// @param lines      Array of lines
/// @param[out] err   Error details, if any
// DLLEXPORT void buffer_insert(Buffer buffer, Integer lnum, ArrayOf(String) lines, Arena *arena,
//                              Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_insert(
    buffer: Buffer,
    lnum: Integer,
    lines: Array,
    arena: *mut Arena,
    err: &mut Error,
) {
    unsafe {
        nvim_buf_set_lines(0, buffer, lnum, lnum, true, lines, arena, err);
    }
}

/// Gets a buffer line
///
/// @deprecated use nvim_buf_get_lines instead.
///
/// @param buffer   Buffer id
/// @param index    Line index
/// @param[out] err Error details, if any
/// @return Line string
// DLLEXPORT String buffer_get_line(Buffer buffer, Integer index, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_get_line(
    buffer: Buffer,
    mut index: Integer,
    arena: *mut Arena,
    err: &mut Error,
) -> NvimString {
    unsafe {
        let mut rv = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        index = convert_index(index);
        let slice = nvim_buf_get_lines(
            0,
            buffer,
            index,
            index + 1,
            true,
            arena,
            ptr::null_mut(),
            err,
        );
        if !error_set(err) && slice.size > 0 {
            rv = slice[0].data.string();
        }
        rv
    }
}

/// Sets a buffer line
///
/// @deprecated use nvim_buf_set_lines instead.
///
/// @param buffer   Buffer id
/// @param index    Line index
/// @param line     Contents of the new line
/// @param[out] err Error details, if any
// DLLEXPORT void buffer_set_line(Buffer buffer, Integer index, String line, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_set_line(
    buffer: Buffer,
    mut index: Integer,
    line: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) {
    unsafe {
        let mut l = string_obj(line);
        let array = Array {
            items: &mut l,
            size: 1,
            capacity: 0,
        };
        index = convert_index(index);
        nvim_buf_set_lines(0, buffer, index, index + 1, true, array, arena, err);
    }
}

/// Deletes a buffer line
///
/// @deprecated use nvim_buf_set_lines instead.
///
/// @param buffer   buffer id
/// @param index    line index
/// @param[out] err Error details, if any
// DLLEXPORT void buffer_del_line(Buffer buffer, Integer index, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_del_line(
    buffer: Buffer,
    mut index: Integer,
    arena: *mut Arena,
    err: &mut Error,
) {
    unsafe {
        let array = Array::new();
        index = convert_index(index);
        nvim_buf_set_lines(0, buffer, index, index + 1, true, array, arena, err);
    }
}

/// Retrieves a line range from the buffer
///
/// @deprecated use nvim_buf_get_lines(buffer, newstart, newend, false)
///
/// @param buffer         Buffer id
/// @param start          First line index
/// @param end            Last line index
/// @param include_start  True if the slice includes the `start` parameter
/// @param include_end    True if the slice includes the `end` parameter
/// @param[out] err       Error details, if any
/// @return Array of lines
// DLLEXPORT ArrayOf(String) buffer_get_line_slice(Buffer buffer, Integer start, Integer end,
//                                                 Boolean include_start, Boolean include_end,
//                                                 Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_get_line_slice(
    buffer: Buffer,
    mut start: Integer,
    mut end: Integer,
    include_start: Boolean,
    include_end: Boolean,
    arena: *mut Arena,
    err: &mut Error,
) -> Array {
    unsafe {
        start = convert_index(start) + i64::from(!include_start);
        end = convert_index(end) + i64::from(include_end);
        nvim_buf_get_lines(0, buffer, start, end, false, arena, ptr::null_mut(), err)
    }
}

/// Replaces a line range on the buffer
///
/// @deprecated use nvim_buf_set_lines(buffer, newstart, newend, false, lines)
///
/// @param buffer         Buffer id, or 0 for current buffer
/// @param start          First line index
/// @param end            Last line index
/// @param include_start  True if the slice includes the `start` parameter
/// @param include_end    True if the slice includes the `end` parameter
/// @param replacement    Array of lines to use as replacement (0-length
///                       array will delete the line range)
/// @param[out] err       Error details, if any
// DLLEXPORT void buffer_set_line_slice(Buffer buffer, Integer start, Integer end,
//                                      Boolean include_start, Boolean include_end,
//                                      ArrayOf(String) replacement, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_set_line_slice(
    buffer: Buffer,
    mut start: Integer,
    mut end: Integer,
    include_start: Boolean,
    include_end: Boolean,
    replacement: Array,
    arena: *mut Arena,
    err: &mut Error,
) {
    unsafe {
        start = convert_index(start) + i64::from(!include_start);
        end = convert_index(end) + i64::from(include_end);
        nvim_buf_set_lines(0, buffer, start, end, false, replacement, arena, err);
    }
}

/// Sets a buffer-scoped (b:) variable
///
/// @deprecated
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param value      Variable value
/// @param[out] err   Error details, if any
/// @return Old value or nil if there was no previous value.
// DLLEXPORT Object buffer_set_var(Buffer buffer, String name, Object value, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_set_var(
    buffer: Buffer,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*buf).b_vars(), name, value, false, true, arena, err)
    }
}

/// Removes a buffer-scoped (b:) variable
///
/// @deprecated
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
/// @return Old value
// DLLEXPORT Object buffer_del_var(Buffer buffer, String name, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn buffer_del_var(
    buffer: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*buf).b_vars(), name, OBJECT_NIL, true, true, arena, err)
    }
}

/// Sets a window-scoped (w:) variable
///
/// @deprecated
///
/// @param window   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
/// @return Old value or nil if there was no previous value.
// DLLEXPORT Object window_set_var(Window window, String name, Object value, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn window_set_var(
    window: Window,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*win).w_vars(), name, value, false, true, arena, err)
    }
}

/// Removes a window-scoped (w:) variable
///
/// @deprecated
///
/// @param window   |window-ID|, or 0 for current window
/// @param name     variable name
/// @param[out] err Error details, if any
/// @return Old value
// DLLEXPORT Object window_del_var(Window window, String name, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn window_del_var(
    window: Window,
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*win).w_vars(), name, OBJECT_NIL, true, true, arena, err)
    }
}

/// Sets a tab-scoped (t:) variable
///
/// @deprecated
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
/// @return Old value or nil if there was no previous value.
// DLLEXPORT Object tabpage_set_var(Tabpage tabpage, String name, Object value, Arena *arena,
//                                  Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*tab).tp_vars(), name, value, false, true, arena, err)
    }
}

/// Removes a tab-scoped (t:) variable
///
/// @deprecated
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Old value
// DLLEXPORT Object tabpage_del_var(Tabpage tabpage, String name, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn tabpage_del_var(
    tabpage: Tabpage,
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return OBJECT_NIL;
        }
        dict_set_var(&mut *(*tab).tp_vars(), name, OBJECT_NIL, true, true, arena, err)
    }
}

/// @deprecated
/// @see nvim_set_var
/// @return Old value or nil if there was no previous value.
// DLLEXPORT Object vim_set_var(String name, Object value, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn vim_set_var(
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe { dict_set_var(&mut *get_globvar_dict(), name, value, false, true, arena, err) }
}

/// @deprecated
/// @see nvim_del_var
// DLLEXPORT Object vim_del_var(String name, Arena *arena, Error *err)
//   FUNC_API_DEPRECATED_SINCE(1);
pub unsafe fn vim_del_var(name: NvimString, arena: *mut Arena, err: &mut Error) -> Object {
    unsafe { dict_set_var(&mut *get_globvar_dict(), name, OBJECT_NIL, true, true, arena, err) }
}

#[inline]
const fn convert_index(index: i64) -> i64 {
    if index < 0 { index - 1 } else { index }
}

/// Gets the option information for one option
///
/// @deprecated Use @ref nvim_get_option_info2 instead.
///
/// @param          name Option name
/// @param[out] err Error details, if any
/// @return         Option Information
// DLLEXPORT DictAs(get_option_info) nvim_get_option_info(String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_DEPRECATED_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_option_info(
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Dict {
    unsafe { get_option_info(name, OPT_GLOBAL, g_st().curbuf.cast(), *curwin.as_ptr(), arena, err) }
}

/// Sets the global value of an option.
///
/// @deprecated
/// @param channel_id
/// @param name     Option name
/// @param value    New option value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_set_option(uint64_t channel_id, String name, Object value, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_set_option(
    channel_id: u64,
    name: NvimString,
    value: Object,
    err: &mut Error,
) {
    set_option_to(
            channel_id,
            OptTarget::Global,
            kOptScopeGlobal,
            name,
            value,
            err,
        );
}

/// Gets the global value of an option.
///
/// @deprecated
/// @param name     Option name
/// @param[out] err Error details, if any
/// @return         Option value (global)
// DLLEXPORT Object nvim_get_option(String name, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11)
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_option(name: NvimString, err: &mut Error) -> Object {
    get_option_from(OptTarget::Global, kOptScopeGlobal, name, err)
}

/// Gets a buffer option value
///
/// @deprecated
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Option name
/// @param[out] err   Error details, if any
/// @return Option value
// DLLEXPORT Object nvim_buf_get_option(Buffer buffer, String name, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11)
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_option(
    buffer: Buffer,
    name: NvimString,
    err: &mut Error,
) -> Object {
    let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return OBJECT_NIL;
        }
        get_option_from(OptTarget::Buf(buf), kOptScopeBuf, name, err)
}

/// Sets a buffer option value. Passing `nil` as value deletes the option (only
/// works if there's a global fallback)
///
/// @deprecated
/// @param channel_id
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Option name
/// @param value      Option value
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_set_option(uint64_t channel_id, Buffer buffer, String name, Object value,
//                                    Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_option(
    channel_id: u64,
    buffer: Buffer,
    name: NvimString,
    value: Object,
    err: &mut Error,
) {
    let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return;
        }
        set_option_to(
            channel_id,
            OptTarget::Buf(buf),
            kOptScopeBuf,
            name,
            value,
            err,
        );
}

/// Gets a window option value
///
/// @deprecated
/// @param window   |window-ID|, or 0 for current window
/// @param name     Option name
/// @param[out] err Error details, if any
/// @return Option value
// DLLEXPORT Object nvim_win_get_option(Window window, String name, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11)
//   FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_option(
    window: Window,
    name: NvimString,
    err: &mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return OBJECT_NIL;
        }
        get_option_from(OptTarget::Win(win), kOptScopeWin, name, err)
    }
}

/// Sets a window option value. Passing `nil` as value deletes the option (only
/// works if there's a global fallback)
///
/// @deprecated
/// @param channel_id
/// @param window   |window-ID|, or 0 for current window
/// @param name     Option name
/// @param value    Option value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_option(uint64_t channel_id, Window window, String name, Object value,
//                                    Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(11);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_option(
    channel_id: u64,
    window: Window,
    name: NvimString,
    value: Object,
    err: &mut Error,
) {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return;
        }
        set_option_to(
            channel_id,
            OptTarget::Win(win),
            kOptScopeWin,
            name,
            value,
            err,
        );
    }
}

fn get_option_from(
    from: OptTarget,
    scope: OptScope,
    name: NvimString,
    err: &mut Error,
) -> Object {
    unsafe {
        if name.size == 0 {
            api_err_invalid(err, c"option name".as_ptr(), c"<empty>".as_ptr(), 0, true);
            return OBJECT_NIL;
        }

        let opt_idx = find_option(name.data);
        if opt_idx == kOptInvalid {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return OBJECT_NIL;
        }

        if !option_has_scope(opt_idx, scope) {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return OBJECT_NIL;
        }

        let value = get_option_value_for(
            opt_idx,
            if scope == kOptScopeGlobal {
                OPT_GLOBAL
            } else {
                OPT_LOCAL
            },
            scope,
            from,
            err,
        );
        if error_set(err) {
            return OBJECT_NIL;
        }

        value
    }
}

fn set_option_to(
    channel_id: u64,
    to: OptTarget,
    scope: OptScope,
    name: NvimString,
    value: Object,
    err: &mut Error,
) {
    unsafe {
        if name.size == 0 {
            api_err_invalid(err, c"option name".as_ptr(), c"<empty>".as_ptr(), 0, true);
            return;
        }

        let opt_idx = find_option(name.data);
        if opt_idx == kOptInvalid {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return;
        }

        let valid = matches!(
            value.obj_type,
            ObjectType::kObjectTypeNil
                | ObjectType::kObjectTypeBoolean
                | ObjectType::kObjectTypeInteger
                | ObjectType::kObjectTypeString
        );
        if !valid {
            api_err_exp(
                err,
                c"value".as_ptr(),
                c"valid option type".as_ptr(),
                api_typename(value.obj_type),
            );
            return;
        }

        let opt_flags = if scope == kOptScopeWin && !option_has_scope(opt_idx, kOptScopeGlobal) {
            0
        } else if scope == kOptScopeGlobal {
            OPT_GLOBAL
        } else {
            OPT_LOCAL
        };

        with_script_context(channel_id, || {
            set_option_value_for(opt_idx, value, opt_flags, scope, to, true, err);
        });
    }
}

/// @deprecated Use nvim_exec_lua() instead.
///
/// Calls many API methods atomically.
///
/// @param channel_id
/// @param calls an array of calls, where each call is described by an array
///              with two elements: the request name, and an array of arguments.
/// @param[out] err Validation error details (malformed `calls` parameter),
///             if any. Errors from batched calls are given in the return value.
///
/// @return Array of two elements.
// DLLEXPORT Array nvim_call_atomic(uint64_t channel_id, Array calls, Arena *arena, Error *err)
//   FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(12) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_call_atomic(
    channel_id: u64,
    calls: Array,
    arena: *mut Arena,
    err: &mut Error,
) -> Array {
    unsafe {
        let mut rv = arena_array(arena, 2);
        let mut results = arena_array(arena, calls.size);
        let mut nested_error = Error::new();

        let mut i = 0usize;
        'theend: {
            while i < calls.size {
                let item = calls[i];
                if item.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"'calls' item".as_ptr(),
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(item.obj_type),
                    );
                    break 'theend;
                }
                let call = item.data.array();
                if call.size != 2 {
                    api_err_exp(
                        err,
                        c"'calls' item".as_ptr(),
                        c"2-item Array".as_ptr(),
                        ptr::null(),
                    );
                    break 'theend;
                }
                let first = call[0];
                if first.obj_type != ObjectType::kObjectTypeString {
                    api_err_exp(
                        err,
                        c"name".as_ptr(),
                        api_typename(ObjectType::kObjectTypeString),
                        api_typename(first.obj_type),
                    );
                    break 'theend;
                }
                let name = first.data.string();
                let second = call[1];
                if second.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"call args".as_ptr(),
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(second.obj_type),
                    );
                    break 'theend;
                }
                let args = second.data.array();

                let handler = msgpack_rpc_get_handler_for(name.data, name.size, &mut nested_error);
                if error_set(&nested_error) {
                    break;
                }

                let func = handler.r#fn.unwrap();
                let result = func(channel_id, args, &mut *arena, &mut nested_error);
                if error_set(&nested_error) {
                    break;
                }
                add_c(&mut results, copy_object(result, arena));
                if handler.ret_alloc {
                    api_free_object(result);
                }

                i += 1;
            }

            add_c(&mut rv, array_obj(results));
            if error_set(&nested_error) {
                let mut errval = arena_array(arena, 3);
                add_c(&mut errval, integer_obj(i as Integer));
                add_c(&mut errval, integer_obj(nested_error.err_type as Integer));
                add_c(
                    &mut errval,
                    string_obj(copy_string(cstr_as_string(nested_error.msg_ptr()), arena)),
                );
                add_c(&mut rv, array_obj(errval));
            } else {
                add_c(&mut rv, OBJECT_NIL);
            }
        }

        api_clear_error(&mut nested_error);
        rv
    }
}

/// @deprecated
///
/// @param channel_id Channel id (passed automatically by the dispatcher)
/// @param event      Event type string
// DLLEXPORT void nvim_subscribe(uint64_t channel_id, String event)
// // XXX: c_grammar.lua is order-sensitive.
//   FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_subscribe(_channel_id: u64, _event: NvimString) {}

/// @deprecated
///
/// @param channel_id Channel id (passed automatically by the dispatcher)
/// @param event      Event type string
// DLLEXPORT void nvim_unsubscribe(uint64_t channel_id, String event)
// // XXX: c_grammar.lua is order-sensitive.
//   FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13) FUNC_API_REMOTE_ONLY;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_unsubscribe(_channel_id: u64, _event: NvimString) {}

fn push_char(line_buf: &mut Vec<c_char>, c: c_char, to_err: bool) {
    unsafe {
        if line_buf.capacity() == 0 {
            line_buf.reserve(LINE_BUFFER_MIN_SIZE);
        }
        if c == NL {
            line_buf.push(NUL);
            if to_err {
                emsg(line_buf.as_ptr());
            } else {
                msg(line_buf.as_ptr(), 0);
            }
            if g_st().msg_silent == 0 {
                g_st().msg_didout = true;
            }
            line_buf.clear();
            line_buf.shrink_to(LINE_BUFFER_MIN_SIZE);
        } else if c == NUL {
            line_buf.push(NL);
        } else {
            line_buf.push(c);
        }
    }
}

fn write_msg(message: NvimString, to_err: bool, writeln: bool) {
    unsafe {
        let line_buf_ptr = if to_err {
            &raw mut api_dep_st().ERR_LINE_BUF
        } else {
            &raw mut api_dep_st().OUT_LINE_BUF
        };
        let line_buf = &mut *line_buf_ptr;

        msg_ext_no_fast();
        g_st().no_wait_return += 1;
        for i in 0..message.size {
            if g_st().got_int {
                break;
            }
            push_char(line_buf, *message.data.add(i), to_err);
        }
        if writeln {
            push_char(line_buf, NL, to_err);
        }
        g_st().no_wait_return -= 1;
        msg_end();
    }
}

/// @deprecated
///
/// @param str Message
// DLLEXPORT void nvim_out_write(String str)
//   FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_out_write(str: NvimString) {
    write_msg(str, false, false);
}

/// @deprecated
///
/// @param str Message
// DLLEXPORT void nvim_err_write(String str)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(13);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_err_write(str: NvimString) {
    write_msg(str, true, false);
}

/// @deprecated
///
/// @param str Message
// DLLEXPORT void nvim_err_writeln(String str)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(13);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_err_writeln(str: NvimString) {
    write_msg(str, true, true);
}

/// @deprecated
///
/// Use `nvim_echo` or `nvim_exec_lua("vim.notify(...)", ...)` instead.
///
/// @param msg        Message to display to the user
/// @param log_level  The log level
/// @param opts       Reserved for future use.
/// @param[out] err   Error details, if any
// DLLEXPORT Object nvim_notify(String msg, Integer log_level, Dict opts, Arena *arena, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_DEPRECATED_SINCE(13);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_notify(
    msg_str: NvimString,
    log_level: Integer,
    opts: Dict,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let mut items = [string_obj(msg_str), integer_obj(log_level), dict_obj(opts)];
        let args = Array {
            size: 3,
            capacity: 3,
            items: items.as_mut_ptr(),
        };
        nlua_exec(
            static_cstr_as_string(c"return vim.notify(...)"),
            ptr::null(),
            args,
            kRetObject,
            arena,
            err,
        )
    }
}

/// @deprecated
///
/// Use |nvim_win_resize()| instead, e.g., nvim_win_resize(win, -1, height)
///
/// Sets the window height.
///
/// @param win   |window-ID|, or 0 for current window
/// @param height   Height as a count of rows
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_height(Window win, Integer height, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(15);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_height(win: Window, height: Integer, err: &mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        try_wrap(err, || {
            win_setheight_win(height as c_int, w, true);
        });
    }
}

/// @deprecated
///
/// Use |nvim_win_resize()| instead, e.g., nvim_win_resize(win, width, -1)
///
/// Sets the window width. This will only succeed if the screen is split
/// vertically.
///
/// @param win   |window-ID|, or 0 for current window
/// @param width    Width as a count of columns
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_width(Window win, Integer width, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_DEPRECATED_SINCE(15);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_width(win: Window, width: Integer, err: &mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        try_wrap(err, || {
            win_setwidth_win(width as c_int, w, true);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_index_shifts_negative_indices() {
        assert_eq!(convert_index(0), 0);
        assert_eq!(convert_index(5), 5);
        assert_eq!(convert_index(-1), -2);
        assert_eq!(convert_index(-3), -4);
    }
}
