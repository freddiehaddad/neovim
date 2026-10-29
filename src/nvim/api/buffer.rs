//! Buffer API functions (`src/nvim/api/buffer.c`).

use crate::api::deprecated::KeyDict_empty;
use crate::api::private::helpers::{
    MAXLNUM, OBJECT_NIL, add_c, api_clear_error, api_set_error, arena_array, arena_bytes,
    arena_dict, boolean_obj, buf_get_text, cstr_as_string, dict_get_value, dict_set_var,
    error_set, find_buffer_by_handle, has_key, integer_obj, normalize_index, put_c, set_mark,
    static_cstr_as_obj, string_obj, try_wrap, try_wrap_err,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, api_err_invalid, api_err_range,
    check_string_array,
};
use crate::api::tabpage::Buffer;
use crate::buffer::{
    BufUpdateCallbacks, DOBUF_FIRST, DOBUF_UNLOAD, DOBUF_WIPE, FileBuf, buf_ensure_loaded,
    do_buffer,
};
use crate::buffer_updates::{buf_updates_register, buf_updates_unregister};
use crate::change::changed_lines;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    Colnr, CursorPos, Linenr, check_cursor_col, check_cursor_lnum, check_visual_pos,
};
use crate::eval::vars::MAXCOL;
use crate::ex_cmds::rename_buffer;
use crate::extmark::{Bcount, K_EXTMARK_NOOP, K_EXTMARK_UNDO, extmark_splice};
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{
    Boolean, LUA_NOREF, LuaRef, lua_State, lua_createtable, lua_push_bytes, lua_rawseti,
};
use crate::lua::executor::{K_RET_MULTI_STACK, nlua_call_ref};
use crate::mapping::{MAPTYPE_MAP, MAPTYPE_UNMAP, MAPTYPE_UNMAP_LHS, keymap_array, modify_keymap};
use crate::mark::{K_MARK_ADJUST_API, K_MARK_ALL_NO_RESOLVE, mark_adjust_buf, mark_get};
use crate::marktree::K_MTMETA_LINES;
use crate::memline::{
    ML_LINE_DIRTY, ml_append_buf, ml_delete_buf, ml_find_line_or_offset, ml_get_buf,
    ml_get_buf_len, ml_replace_buf,
};
use crate::memory::{Arena, arena_allocz, arena_memdupz};
use crate::r#move::{changed_cline_bef_curs, invalidate_botline_win, update_topline};
use crate::msgpack_rpc::packer::Integer;
use crate::ops::get_region_bytecount;
use crate::option_vars::*;
use crate::undo::{u_save_buf, uhp_ref};
use crate::window::{VALID_BOTLINE_AP, WinInfoInfo};
use std::ptr;

use crate::search::FORWARD;

pub(crate) const INTERNAL_CALL_MASK: u64 = 1u64 << 63;
pub(crate) const VIML_INTERNAL_CALL: u64 = INTERNAL_CALL_MASK;
pub(crate) const LUA_INTERNAL_CALL: u64 = VIML_INTERNAL_CALL + 1;

const KEYSET_OPTIDX_BUF_ATTACH_ON_BYTES: u64 = 2;
const KEYSET_OPTIDX_BUF_ATTACH_ON_LINES: u64 = 3;
const KEYSET_OPTIDX_BUF_ATTACH_ON_DETACH: u64 = 5;
const KEYSET_OPTIDX_BUF_ATTACH_ON_RELOAD: u64 = 6;
const KEYSET_OPTIDX_BUF_ATTACH_ON_CHANGEDTICK: u64 = 7;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_buf_attach {
    pub is_set_buf_attach: OptionalKeys,
    pub on_lines: LuaRef,
    pub on_bytes: LuaRef,
    pub on_changedtick: LuaRef,
    pub on_detach: LuaRef,
    pub on_reload: LuaRef,
    pub utf_sizes: Boolean,
    pub preview: Boolean,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_keymap {
    pub is_set_keymap: OptionalKeys,
    pub noremap: Boolean,
    pub nowait: Boolean,
    pub silent: Boolean,
    pub script: Boolean,
    pub expr: Boolean,
    pub unique: Boolean,
    pub callback: LuaRef,
    pub desc: NvimString,
    pub replace_keycodes: Boolean,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_keymap_del {
    pub is_set_keymap_del: OptionalKeys,
    pub lhs: Boolean,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_buf_delete {
    pub is_set_buf_delete: OptionalKeys,
    pub force: Boolean,
    pub unload: Boolean,
}

use crate::math::{FAIL, OK};
use crate::regexp::CTRL_V_CONST;
use crate::state::MODE_INSERT;

// @brief <pre>help
// For more information on buffers, see |buffers|.
//
// Unloaded Buffers: ~
//
// Buffers may be unloaded by the |:bunload| command or the buffer's
// 'bufhidden' option. When a buffer is unloaded its file contents are freed
// from memory and vim cannot operate on the buffer lines until it is reloaded
// (usually by opening the buffer again in a new window). API methods such as
// |nvim_buf_get_lines()| and |nvim_buf_line_count()| will be affected.
//
// You can use |nvim_buf_is_loaded()| or |nvim_buf_line_count()| to check
// whether a buffer is loaded.
// </pre>

/// Ensures that a buffer is loaded.
// DLLEXPORT FileBuf *api_buf_ensure_loaded(Buffer buf, Error *err);
pub fn api_buf_ensure_loaded(buf: Buffer, err: &mut Error) -> *mut FileBuf {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return ptr::null_mut();
    };

    if b.b_ml.ml_mfp.is_none() && !buf_ensure_loaded(b) {
        api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to load buffer",);
        return ptr::null_mut();
    }

    (&raw mut *b)
}

/// Returns the number of lines in the given buffer.
///
/// @param buf   Buffer id, or 0 for current buffer
/// @param[out] err Error details, if any
/// @return Line count, or 0 for unloaded buffer. |api-buffer|
// DLLEXPORT Integer nvim_buf_line_count(Buffer buf, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_line_count(buf: Buffer, err: &mut Error) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() || (*b).b_ml.ml_mfp.is_none() {
            return 0;
        }
        i64::from((*b).b_ml.ml_line_count)
    }
}

/// Activates |api-buffer-updates| events on a channel, or as Lua callbacks.
///
/// Example (Lua): capture buffer updates in a global `events` variable
/// (use "vim.print(events)" to see its contents):
///
/// ```lua
/// events = {}
/// vim.api.nvim_buf_attach(0, false, {
///   on_lines = function(...)
///     table.insert(events, {...})
///   end,
/// })
/// ```
///
/// @see |nvim_buf_detach()|
/// @see |api-buffer-updates-lua|
///
/// @param channel_id
/// @param buf Buffer id, or 0 for current buffer
/// @param send_buffer True if the initial notification should contain the
///        whole buffer: first notification will be `nvim_buf_lines_event`.
///        Else the first notification will be `nvim_buf_changedtick_event`.
///        Not for Lua callbacks.
/// @param  opts  Optional parameters.
///             - on_bytes: Called on granular changes (compared to on_lines). Not called on buffer
///               reload (`:checktime`, `:edit`, …), see `on_reload`. Returning [lua-truthy] deletes
///               the callback. Args:
///               - the string "bytes"
///               - buffer id
///               - b:changedtick
///               - start row of the changed text (zero-indexed)
///               - start column of the changed text
///               - byte offset of the changed text (from the start of
///                   the buffer)
///               - old end row of the changed text (offset from start row)
///               - old end column of the changed text
///                 (if old end row = 0, offset from start column)
///               - old end byte length of the changed text
///               - new end row of the changed text (offset from start row)
///               - new end column of the changed text
///                 (if new end row = 0, offset from start column)
///               - new end byte length of the changed text
///             - on_changedtick: Called on [changetick] increment without text change. Args:
///               - the string "changedtick"
///               - buffer id
///               - b:changedtick
///             - on_detach: Called on detach, or when the buffer is unloaded or deleted. Not called
///               when a callback returns [lua-truthy] to delete itself. Args:
///               - the string "detach"
///               - buffer id
///             - on_lines: Called on linewise changes. Not called on buffer reload (`:checktime`,
///               `:edit`, …), see `on_reload`. Returning [lua-truthy] deletes the callback. Args:
///               - the string "lines"
///               - buffer id
///               - b:changedtick
///               - first line that changed (zero-indexed)
///               - last line that was changed
///               - last line in the updated range
///               - byte count of previous contents
///               - deleted_codepoints (if `utf_sizes` is true)
///               - deleted_codeunits (if `utf_sizes` is true)
///             - on_reload: Called on whole-buffer load (`:checktime`, `:edit`, …). Clients should
///               typically re-fetch the entire buffer contents. Args:
///               - the string "reload"
///               - buffer id
///             - preview: also attach to command preview (i.e. 'inccommand')
///               events.
///             - utf_sizes: include UTF-32 and UTF-16 size of the replaced
///               region, as args to `on_lines`.
/// @param[out] err Error details, if any
/// @return False if attach failed (invalid parameter, or buffer isn't loaded);
///         otherwise True.
// DLLEXPORT Boolean nvim_buf_attach(uint64_t channel_id, Buffer buf, Boolean send_buffer,
//                                   Dict(buf_attach) *opts, Error *err)
//   FUNC_API_SINCE(4);
pub fn nvim_buf_attach(
    channel_id: u64,
    buf: Buffer,
    send_buffer: Boolean,
    opts: &mut KeyDict_buf_attach,
    err: &mut Error,
) -> Boolean {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return false;
    };

    let mut cb = BufUpdateCallbacks {
        on_lines: LUA_NOREF,
        on_bytes: LUA_NOREF,
        on_changedtick: LUA_NOREF,
        on_detach: LUA_NOREF,
        on_reload: LUA_NOREF,
        utf_sizes: false,
        preview: false,
    };

    if channel_id == LUA_INTERNAL_CALL {
        let mask = opts.is_set_buf_attach;
        if has_key(mask, KEYSET_OPTIDX_BUF_ATTACH_ON_LINES) {
            cb.on_lines = opts.on_lines;
            opts.on_lines = LUA_NOREF;
        }
        if has_key(mask, KEYSET_OPTIDX_BUF_ATTACH_ON_BYTES) {
            cb.on_bytes = opts.on_bytes;
            opts.on_bytes = LUA_NOREF;
        }
        if has_key(mask, KEYSET_OPTIDX_BUF_ATTACH_ON_CHANGEDTICK) {
            cb.on_changedtick = opts.on_changedtick;
            opts.on_changedtick = LUA_NOREF;
        }
        if has_key(mask, KEYSET_OPTIDX_BUF_ATTACH_ON_DETACH) {
            cb.on_detach = opts.on_detach;
            opts.on_detach = LUA_NOREF;
        }
        if has_key(mask, KEYSET_OPTIDX_BUF_ATTACH_ON_RELOAD) {
            cb.on_reload = opts.on_reload;
            opts.on_reload = LUA_NOREF;
        }
        cb.utf_sizes = opts.utf_sizes;
        cb.preview = opts.preview;
    }

    buf_updates_register(b, channel_id, cb, send_buffer)
}

/// Deactivates buffer-update events on the channel.
///
/// @see |nvim_buf_attach()|
/// @see |api-lua-detach| for detaching Lua callbacks
///
/// @param channel_id
/// @param buf Buffer id, or 0 for current buffer
/// @param[out] err Error details, if any
/// @return False if detach failed (because the buffer isn't loaded);
///         otherwise True.
// DLLEXPORT Boolean nvim_buf_detach(uint64_t channel_id, Buffer buf, Error *err)
//   FUNC_API_SINCE(4) FUNC_API_REMOTE_ONLY;
pub fn nvim_buf_detach(channel_id: u64, buf: Buffer, err: &mut Error) -> Boolean {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return false;
    };
    buf_updates_unregister(b, channel_id);
    true
}

/// Gets a line-range from the buffer.
///
/// Indexing is zero-based, end-exclusive. Negative indices are interpreted
/// as length+1+index: -1 refers to the index past the end. So to get the
/// last element use start=-2 and end=-1.
///
/// Out-of-bounds indices are clamped to the nearest valid value, unless
/// `strict_indexing` is set.
///
/// @see |nvim_buf_get_text()|
///
/// @param channel_id
/// @param buf           Buffer id, or 0 for current buffer
/// @param start            First line index
/// @param end              Last line index, exclusive
/// @param strict_indexing  Whether out-of-bounds should be an error.
/// @param[out] err         Error details, if any
/// @return Array of lines, or empty array for unloaded buffer.
// DLLEXPORT ArrayOf(String) nvim_buf_get_lines(uint64_t channel_id, Buffer buf, Integer start,
//                                              Integer end, Boolean strict_indexing, Arena *arena,
//                                              lua_State *lstate, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_get_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    arena: &mut Arena,
    lstate: *mut lua_State,
    err: &mut Error,
) -> Array {
    let mut rv = Array::new();
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return rv;
    };

    if b.b_ml.ml_mfp.is_none() {
        init_line_array(lstate, &mut rv, 0, arena);
        return rv;
    }

    let mut oob = false;
    start = normalize_index(b, start, true, &mut oob);
    end = normalize_index(b, end, true, &mut oob);

    if strict_indexing && oob {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"Index out of bounds",
        );
        return rv;
    }

    let size = if end >= start {
        (end - start) as usize
    } else {
        0
    };

    init_line_array(lstate, &mut rv, size, arena);

    buf_collect_lines(
        &mut *b,
        size,
        start as Linenr,
        0,
        channel_id != VIML_INTERNAL_CALL,
        &mut rv,
        lstate,
        arena,
    );

    rv
}

/// Sets (replaces) a line-range in the buffer.
///
/// Indexing is zero-based, end-exclusive. Negative indices are interpreted
/// as length+1+index: -1 refers to the index past the end. So to change
/// or delete the last line use start=-2 and end=-1.
///
/// To insert lines at a given index, set `start` and `end` to the same index.
/// To delete a range of lines, set `replacement` to an empty array.
///
/// Out-of-bounds indices are clamped to the nearest valid value, unless
/// `strict_indexing` is set.
///
/// @see |nvim_buf_set_text()|
///
/// @param channel_id
/// @param buf           Buffer id, or 0 for current buffer
/// @param start            First line index
/// @param end              Last line index, exclusive
/// @param strict_indexing  Whether out-of-bounds should be an error.
/// @param replacement      Array of lines to use as replacement
/// @param[out] err         Error details, if any
// DLLEXPORT void nvim_buf_set_lines(uint64_t channel_id, Buffer buf, Integer start, Integer end,
//                                   Boolean strict_indexing, ArrayOf(String) replacement,
//                                   Arena *arena, Error *err)
//   FUNC_API_SINCE(1)
//   FUNC_API_TEXTLOCK;
pub fn nvim_buf_set_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    replacement: Array,
    arena: &mut Arena,
    err: &mut Error,
) {
    let b = api_buf_ensure_loaded(buf, err);
    if b.is_null() {
        return;
    }

    let mut oob = false;
    start = normalize_index(unsafe { &mut *b }, start, true, &mut oob);
    end = normalize_index(unsafe { &mut *b }, end, true, &mut oob);

    if strict_indexing && oob {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"Index out of bounds",
        );
        return;
    }
    if start > end {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"'start' is higher than 'end'",
        );
        return;
    }

    let disallow_nl = channel_id != VIML_INTERNAL_CALL;
    if !check_string_array(replacement, c"replacement string", disallow_nl, err) {
        return;
    }

    let new_len = replacement.size;
    let old_len = (end - start) as usize;
    let mut extra: isize = 0;
    let mut lines: Vec<*mut u8> = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let l = replacement[i].data.string();
        let p = arena_memdupz(arena, l.data, l.size).cast::<u8>();
        if l.size > 0 {
            for b in unsafe { std::slice::from_raw_parts_mut(p, l.size) } {
                if *b == 0 {
                    *b = b'\n';
                }
            }
        }
        lines.push(p);
    }

    try_wrap_err(err, |err| 'end: {
        if unsafe { (*b).b_p_ma } == 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Buffer is not 'modifiable'",
            );
            break 'end;
        }

        if !u_save_buf(unsafe { &mut *b }, (start - 1) as Linenr, end as Linenr) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Failed to save undo information",
            );
            break 'end;
        }

        let deleted_bytes =
            get_region_bytecount(unsafe { &mut *b }, start as Linenr, end as Linenr, 0, 0);

        let to_delete = old_len.saturating_sub(new_len);
        for _ in 0..to_delete {
            if !ml_delete_buf(unsafe { &mut *b }, start as Linenr, false) {
                api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to delete line",);
                break 'end;
            }
        }

        if to_delete > 0 {
            extra -= to_delete as isize;
        }

        let to_replace = old_len.min(new_len);
        let mut inserted_bytes: Bcount = 0;
        for (i, &line) in lines.iter().enumerate().take(to_replace) {
            let lnum = start + (i as i64);
            if lnum >= MAXLNUM {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Index out of bounds",
                );
                break 'end;
            }

            let line_ptr = line.cast();
            if !ml_replace_buf(unsafe { &mut *b }, lnum as Linenr, line_ptr, false, true) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Failed to replace line",
                );
                break 'end;
            }

            inserted_bytes += crate::memory::cstr_len(line_ptr) as Bcount + 1;
        }

        for (i, &line) in lines.iter().enumerate().take(new_len).skip(to_replace) {
            let lnum = start + (i as i64) - 1;
            if lnum >= MAXLNUM {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Index out of bounds",
                );
                break 'end;
            }

            let line_ptr = line.cast();
            if !ml_append_buf(unsafe { &mut *b }, lnum as Linenr, line_ptr, 0, false) {
                api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to insert line",);
                break 'end;
            }

            inserted_bytes += crate::memory::cstr_len(line_ptr) as Bcount + 1;
            extra += 1;
        }

        let adjust = if end > start { MAXLNUM as Linenr } else { 0 };
        mark_adjust_buf(
            unsafe { &mut *b },
            start as Linenr,
            (end - 1) as Linenr,
            adjust,
            extra as Linenr,
            true,
            K_MARK_ADJUST_API,
            K_EXTMARK_NOOP,
        );

        if g_st().visual_global.active
            && b == g_st().curbuf
            && g_st().visual_global.start.lnum >= (start as Linenr)
        {
            if g_st().visual_global.start.lnum >= (end as Linenr) {
                g_st().visual_global.start.lnum += extra as Linenr;
            }
            check_visual_pos();
        }

        extmark_splice(
            unsafe { &mut *b },
            (start as i32) - 1,
            0,
            (end - start) as i32,
            0,
            deleted_bytes,
            new_len as i32,
            0,
            inserted_bytes,
            K_EXTMARK_UNDO,
        );

        changed_lines(
            unsafe { &mut *b },
            start as Linenr,
            0,
            end as Linenr,
            extra as Linenr,
            true,
        );

        for tp in for_all_tabpages_ptr() {
            for win in for_all_windows_in_tab_mut(&*tp) {
                if win.w_buffer == b {
                    fix_cursor(win, start as Linenr, end as Linenr, extra as Linenr);
                }
            }
        }
    });
}

/// Sets (replaces) a range in the buffer
///
/// This is recommended over |nvim_buf_set_lines()| when only modifying parts of
/// a line, as extmarks will be preserved on non-modified parts of the touched
/// lines.
///
/// Indexing is zero-based. Row indices are end-inclusive, and column indices
/// are end-exclusive.
///
/// To insert text at a given `(row, column)` location, use `start_row = end_row
/// = row` and `start_col = end_col = col`. To delete the text in a range, use
/// `replacement = {}`.
///
/// @note Prefer |nvim_buf_set_lines()| (for performance) to add or delete entire lines.
/// @note Prefer |nvim_paste()| or |nvim_put()| to insert (instead of replace) text at cursor.
///
/// @param channel_id
/// @param buf           Buffer id, or 0 for current buffer
/// @param start_row        First line index
/// @param start_col        Starting column (byte offset) on first line
/// @param end_row          Last line index, inclusive
/// @param end_col          Ending column (byte offset) on last line, exclusive
/// @param replacement      Array of lines to use as replacement
/// @param[out] err         Error details, if any
// DLLEXPORT void nvim_buf_set_text(uint64_t channel_id, Buffer buf, Integer start_row,
//                                  Integer start_col, Integer end_row, Integer end_col,
//                                  ArrayOf(String) replacement, Arena *arena, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_TEXTLOCK;
pub fn nvim_buf_set_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    mut start_col: Integer,
    mut end_row: Integer,
    mut end_col: Integer,
    mut replacement: Array,
    arena: &mut Arena,
    err: &mut Error,
) {
    unsafe {
        let mut scratch_items = [static_cstr_as_obj(c"")];
        if replacement.size == 0 {
            replacement = Array {
                size: 1,
                capacity: 1,
                items: scratch_items.as_mut_ptr(),
            };
        }

        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return;
        }

        let mut oob = false;
        start_row = normalize_index(&mut *b, start_row, false, &mut oob);
        if oob {
            api_err_range(err, c"start_row");
            return;
        }

        end_row = normalize_index(&mut *b, end_row, false, &mut oob);
        if oob {
            api_err_range(err, c"end_row");
            return;
        }

        let str_at_start_raw = ml_get_buf(&mut *b, start_row as Linenr);
        let len_at_start = ml_get_buf_len(&mut *b, start_row as Linenr);
        let str_at_start = arena_memdupz(arena, str_at_start_raw, len_at_start as usize);
        if start_col < 0 {
            start_col = i64::from(len_at_start) + start_col + 1;
        }
        if start_col < 0 || start_col > i64::from(len_at_start) {
            api_err_range(err, c"start_col");
            return;
        }

        let str_at_end_raw = ml_get_buf(&mut *b, end_row as Linenr);
        let len_at_end = ml_get_buf_len(&mut *b, end_row as Linenr);
        let str_at_end = arena_memdupz(arena, str_at_end_raw, len_at_end as usize);
        if end_col < 0 {
            end_col = i64::from(len_at_end) + end_col + 1;
        }
        if end_col < 0 || end_col > i64::from(len_at_end) {
            api_err_range(err, c"end_col");
            return;
        }

        if start_row > end_row || (end_row == start_row && start_col > end_col) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"'start' is higher than 'end'",
            );
            return;
        }

        let disallow_nl = channel_id != VIML_INTERNAL_CALL;
        if !check_string_array(replacement, c"replacement string", disallow_nl, err) {
            return;
        }

        let new_len = replacement.size;
        let mut new_byte: Bcount = 0;
        let mut old_byte: Bcount = 0;

        if start_row == end_row {
            old_byte = (end_col - start_col) as Bcount;
        } else {
            old_byte += (i64::from(len_at_start) - start_col) as Bcount;
            for i in 1..(end_row - start_row) {
                let lnum = start_row + i;
                old_byte += (ml_get_buf_len(&mut *b, lnum as Linenr) as Bcount) + 1;
            }
            old_byte += (end_col as Bcount) + 1;
        }

        let first_item = replacement[0].data.string();
        let last_item = replacement[replacement.size - 1].data.string();

        let mut firstlen = (start_col as usize) + first_item.size;
        let last_part_len = (len_at_end as usize) - (end_col as usize);
        if replacement.size == 1 {
            firstlen += last_part_len;
        }
        let first = arena_allocz(arena, firstlen).cast::<u8>();
        let mut last: *mut u8 = ptr::null_mut();
        ptr::copy_nonoverlapping(str_at_start.cast::<u8>(), first, start_col as usize);
        if first_item.size > 0 {
            ptr::copy_nonoverlapping(
                first_item.data.cast::<u8>(),
                first.wrapping_add(start_col as usize),
                first_item.size,
            );
            for b in std::slice::from_raw_parts_mut(first.add(start_col as usize), first_item.size)
            {
                if *b == 0 {
                    *b = b'\n';
                }
            }
        }
        if replacement.size == 1 {
            ptr::copy_nonoverlapping(
                str_at_end.wrapping_add(end_col as usize).cast::<u8>(),
                first.wrapping_add((start_col as usize) + first_item.size),
                last_part_len,
            );
        } else {
            last = arena_allocz(arena, last_item.size + last_part_len).cast::<u8>();
            if last_item.size > 0 {
                ptr::copy_nonoverlapping(last_item.data.cast::<u8>(), last, last_item.size);
                for b in std::slice::from_raw_parts_mut(last, last_item.size) {
                    if *b == 0 {
                        *b = b'\n';
                    }
                }
            }
            ptr::copy_nonoverlapping(
                str_at_end.wrapping_add(end_col as usize).cast::<u8>(),
                last.wrapping_add(last_item.size),
                last_part_len,
            );
        }

        let mut lines: Vec<*mut u8> = Vec::with_capacity(new_len);
        lines.push(first);
        new_byte += first_item.size as Bcount;
        for i in 1..new_len.saturating_sub(1) {
            let l = replacement[i].data.string();
            let p = arena_memdupz(arena, l.data, l.size).cast::<u8>();
            if l.size > 0 {
                for b in std::slice::from_raw_parts_mut(p, l.size) {
                    if *b == 0 {
                        *b = b'\n';
                    }
                }
            }
            lines.push(p);
            new_byte += (l.size as Bcount) + 1;
        }
        if replacement.size > 1 {
            lines.push(last);
            new_byte += (last_item.size as Bcount) + 1;
        }

        try_wrap_err(err, |err| 'end: {
            if (*b).b_p_ma == 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Buffer is not 'modifiable'",
                );
                break 'end;
            }

            if !u_save_buf(&mut *b, (start_row as Linenr) - 1, (end_row as Linenr) + 1) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Failed to save undo information",
                );
                break 'end;
            }

            let mut extra: isize = 0;
            let old_len = (end_row - start_row + 1) as usize;

            let to_delete = old_len.saturating_sub(new_len);
            for _ in 0..to_delete {
                if !ml_delete_buf(&mut *b, start_row as Linenr, false) {
                    api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to delete line",);
                    break 'end;
                }
            }

            if to_delete > 0 {
                extra -= to_delete as isize;
            }

            let to_replace = old_len.min(new_len);
            for (i, &line) in lines.iter().enumerate().take(to_replace) {
                let lnum = start_row + (i as i64);
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"%s",
                        c"Index out of bounds",
                    );
                    break 'end;
                }

                if !ml_replace_buf(&mut *b, lnum as Linenr, line.cast(), false, true) {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Failed to replace line",
                    );
                    break 'end;
                }
            }

            for (i, &line) in lines.iter().enumerate().take(new_len).skip(to_replace) {
                let lnum = start_row + (i as i64) - 1;
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"%s",
                        c"Index out of bounds",
                    );
                    break 'end;
                }

                if !ml_append_buf(&mut *b, lnum as Linenr, line.cast(), 0, false) {
                    api_set_error!(err, ErrorType::ErrorTypeException, c"Failed to insert line",);
                    break 'end;
                }

                extra += 1;
            }

            let col_extent = (end_col - if end_row == start_row { start_col } else { 0 }) as Colnr;

            let adjust = if end_row >= start_row {
                MAXLNUM as Linenr
            } else {
                0
            };
            mark_adjust_buf(
                &mut *b,
                start_row as Linenr,
                (end_row as Linenr) - 1,
                adjust,
                extra as Linenr,
                true,
                K_MARK_ADJUST_API,
                K_EXTMARK_NOOP,
            );

            if g_st().visual_global.active
                && b == g_st().curbuf
                && g_st().visual_global.mode != CTRL_V_CONST
            {
                fix_pos_col(
                    &mut *b,
                    &mut g_st().visual_global.start,
                    start_row as Linenr,
                    start_col as Colnr,
                    end_row as Linenr,
                    end_col as Colnr,
                    new_len as Linenr,
                    last_item.size as Colnr,
                    1,
                );
                check_visual_pos();
            }

            extmark_splice(
                &mut *b,
                (start_row as i32) - 1,
                start_col as Colnr,
                (end_row - start_row) as i32,
                col_extent,
                old_byte,
                (new_len as i32) - 1,
                last_item.size as Colnr,
                new_byte,
                K_EXTMARK_UNDO,
            );

            changed_lines(
                &mut *b,
                start_row as Linenr,
                start_col as Colnr,
                (end_row as Linenr) + 1,
                extra as Linenr,
                true,
            );

            for tp in for_all_tabpages_ptr() {
                for win in for_all_windows_in_tab_mut(&*tp) {
                    if win.w_buffer == b {
                        if i64::from(win.w_cursor.lnum) >= start_row
                            && i64::from(win.w_cursor.lnum) <= end_row
                        {
                            fix_cursor_cols(
                                win,
                                start_row as Linenr,
                                start_col as Colnr,
                                end_row as Linenr,
                                end_col as Colnr,
                                new_len as Linenr,
                                last_item.size as Colnr,
                            );
                        } else {
                            fix_cursor(
                                win,
                                start_row as Linenr,
                                end_row as Linenr,
                                extra as Linenr,
                            );
                        }
                    }
                }
            }
        });
    }
}

/// Gets a range from the buffer (may be partial lines, unlike |nvim_buf_get_lines()|).
///
/// Indexing is zero-based. Row indices are end-inclusive, and column indices
/// are end-exclusive.
///
/// Prefer |nvim_buf_get_lines()| when retrieving entire lines.
///
/// @param channel_id
/// @param buf     Buffer id, or 0 for current buffer
/// @param start_row  First line index
/// @param start_col  Starting column (byte offset) on first line
/// @param end_row    Last line index, inclusive
/// @param end_col    Ending column (byte offset) on last line, exclusive
/// @param opts       Optional parameters. Currently unused.
/// @param[out] err   Error details, if any
/// @return Array of lines, or empty array for unloaded buffer.
// DLLEXPORT ArrayOf(String) nvim_buf_get_text(uint64_t channel_id, Buffer buf, Integer start_row,
//                                             Integer start_col, Integer end_row, Integer end_col,
//                                             Dict(empty) *opts, Arena *arena, lua_State *lstate,
//                                             Error *err)
//   FUNC_API_SINCE(9);
pub fn nvim_buf_get_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    start_col: Integer,
    mut end_row: Integer,
    end_col: Integer,
    _opts: &mut KeyDict_empty,
    arena: &mut Arena,
    lstate: *mut lua_State,
    err: &mut Error,
) -> Array {
    let mut rv = Array::new();
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return rv;
    };

    if b.b_ml.ml_mfp.is_none() {
        init_line_array(lstate, &mut rv, 0, arena);
        return rv;
    }

    let mut oob = false;
    start_row = normalize_index(b, start_row, false, &mut oob);
    end_row = normalize_index(b, end_row, false, &mut oob);

    if oob {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"Index out of bounds",
        );
        return rv;
    }

    if start_row > end_row {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"'start' is higher than 'end'",
        );
        return rv;
    }

    let replace_nl = channel_id != VIML_INTERNAL_CALL;
    let size = ((end_row - start_row) as usize) + 1;

    init_line_array(lstate, &mut rv, size, arena);

    if start_row == end_row {
        let line = buf_get_text(&mut *b, start_row, start_col, end_col, err);
        if error_set(err) {
            return Array::new();
        }
        push_linestr(lstate, &mut rv, line.as_bytes(), 0, replace_nl, arena);
        return rv;
    }

    let mut str_val = buf_get_text(&mut *b, start_row, start_col, MAXCOL - 1, err);
    if error_set(err) {
        return Array::new();
    }

    push_linestr(lstate, &mut rv, str_val.as_bytes(), 0, replace_nl, arena);

    if size > 2 {
        buf_collect_lines(
            &mut *b,
            size - 2,
            (start_row as Linenr) + 1,
            1,
            replace_nl,
            &mut rv,
            lstate,
            arena,
        );
    }

    str_val = buf_get_text(&mut *b, end_row, 0, end_col, err);
    if error_set(err) {
        return Array::new();
    }

    push_linestr(
        lstate,
        &mut rv,
        str_val.as_bytes(),
        (size - 1) as i32,
        replace_nl,
        arena,
    );

    if error_set(err) {
        return Array::new();
    }

    rv
}

/// Returns the byte offset of a line (0-indexed). |api-indexing|
///
/// Line 1 (index=0) has offset 0. UTF-8 bytes are counted. EOL is one byte.
/// 'fileformat' and 'fileencoding' are ignored. The line index just after the
/// last line gives the total byte-count of the buffer. A final EOL byte is
/// counted if it would be written, see 'eol'.
///
/// Unlike |line2byte()|, throws error for out-of-bounds indexing.
/// Returns -1 for unloaded buffer.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param index      Line index
/// @param[out] err   Error details, if any
/// @return Integer byte offset, or -1 for unloaded buffer.
// DLLEXPORT Integer nvim_buf_get_offset(Buffer buf, Integer index, Error *err)
//   FUNC_API_SINCE(5);
pub fn nvim_buf_get_offset(buf: Buffer, index: Integer, err: &mut Error) -> Integer {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return 0;
    };

    if b.b_ml.ml_mfp.is_none() {
        return -1;
    }

    if index < 0 || index > i64::from(b.b_ml.ml_line_count) {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"Index out of bounds",
        );
        return 0;
    }

    i64::from(ml_find_line_or_offset(b, (index as Linenr) + 1, None, true))
}

/// Gets a buffer-scoped (b:) variable.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_buf_get_var(Buffer buf, String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_get_var(
    buf: Buffer,
    name: NvimString,
    arena: &mut Arena,
    err: &mut Error,
) -> Object {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return OBJECT_NIL;
    };
    dict_get_value(Some(&*b.b_vars()), name, Some(arena), err)
}

/// Gets a changed tick of a buffer
///
/// @param[in]  buf  Buffer id, or 0 for current buffer
/// @param[out] err     Error details, if any
///
/// @return `b:changedtick` value.
// DLLEXPORT Integer nvim_buf_get_changedtick(Buffer buf, Error *err)
//   FUNC_API_SINCE(2);
pub fn nvim_buf_get_changedtick(buf: Buffer, err: &mut Error) -> Integer {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return -1;
    };
    b.changedtick_di.di_tv.vval.v_number()
}

/// Gets a list of buffer-local |mapping| definitions.
///
/// @param  buf     Buffer id, or 0 for current buffer
/// @param  mode       Mode short-name ("n", "i", "v", ...)
/// @param[out]  err   Error details, if any
/// @returns Array of |maparg()|-like dictionaries describing mappings.
///          The "buf" key holds the associated buffer id.
// DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_buf_get_keymap(Buffer buf, String mode, Arena *arena,
//                                                           Error *err)
//   FUNC_API_SINCE(3);
pub fn nvim_buf_get_keymap(
    buf: Buffer,
    mode: NvimString,
    arena: &mut Arena,
    err: &mut Error,
) -> Array {
    let b = find_buffer_by_handle(buf, err);
    if b.is_null() {
        return Array::new();
    }
    let b = unsafe { &mut *b };
    keymap_array(mode, b, arena)
}

/// Sets a buffer-local |mapping| for the given mode.
///
/// @see |nvim_set_keymap()|
///
/// @param  buf  Buffer id, or 0 for current buffer
// DLLEXPORT void nvim_buf_set_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
//                                    String rhs, Dict(keymap) *opts, Error *err)
//   FUNC_API_SINCE(6);
pub fn nvim_buf_set_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: &mut KeyDict_keymap,
    err: &mut Error,
) {
    modify_keymap(
        channel_id,
        buf,
        MAPTYPE_MAP,
        mode,
        lhs,
        rhs,
        Some(opts),
        err,
    );
}

/// Unmaps a buffer-local |mapping| for the given mode.
///
/// @see |nvim_del_keymap()|
///
/// @param  buf   Buffer id, or 0 for current buffer
/// @param  mode  Mode short-name ("n", "i", "v", ...)
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  opts  Optional parameters.
///               - lhs: When true, only match {lhs}, not {rhs}.
// DLLEXPORT void nvim_buf_del_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
//                                    Dict(keymap_del) *opts, Error *err)
//   FUNC_API_SINCE(6);
pub fn nvim_buf_del_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    opts: &mut KeyDict_keymap_del,
    err: &mut Error,
) {
    let rhs = NvimString::from_bytes(b"");
    let maptype = if opts.lhs {
        MAPTYPE_UNMAP_LHS
    } else {
        MAPTYPE_UNMAP
    };
    modify_keymap(channel_id, buf, maptype, mode, lhs, rhs, None, err);
}

/// Sets a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param value      Variable value
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_set_var(Buffer buf, String name, Object value, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_set_var(buf: Buffer, name: NvimString, value: Object, err: &mut Error) {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return;
    };
    dict_set_var(&mut *b.b_vars(), name, value, false, false, None, err);
}

/// Removes a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_del_var(Buffer buf, String name, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_del_var(buf: Buffer, name: NvimString, err: &mut Error) {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return;
    };
    dict_set_var(&mut *b.b_vars(), name, OBJECT_NIL, true, false, None, err);
}

/// Gets the full/absolute filepath of the buffer, or the buffer name for non-file buffers.
///
/// If the buffer represents a directory, the name ends with a path separator,
/// unless it was changed by |:file| or |nvim_buf_set_name()|.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param[out] err   Error details, if any
/// @return Buffer name
// DLLEXPORT String nvim_buf_get_name(Buffer buf, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_get_name(buf: Buffer, err: &mut Error) -> NvimString {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() || (*b).b_ffname.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        cstr_as_string((*b).b_ffname)
    }
}

/// Sets the full file name for a buffer, like |:file_f|
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Buffer name
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_set_name(Buffer buf, String name, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_set_name(buf: Buffer, name: NvimString, err: &mut Error) {
    let b = find_buffer_by_handle(buf, err);
    if b.is_null() {
        return;
    }

    let mut ren_ret = OK;
    try_wrap(err, || {
        let is_curbuf = b == g_st().curbuf;
        let save_acd = opt_vars().p_acd;
        if !is_curbuf {
            g_st().redrawing_disabled += 1;
            opt_vars().p_acd = 0;
        }

        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), b.cast(), 0);
        ren_ret = if rename_buffer(name.data) { OK } else { FAIL };
        ctx_restore(&mut aco);

        if !is_curbuf {
            g_st().redrawing_disabled -= 1;
            opt_vars().p_acd = save_acd;
        }
    });

    if error_set(err) {
        return;
    }

    if ren_ret == FAIL {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Failed to rename buffer",
        );
    }
}

/// Checks if a buffer is valid and loaded. See |api-buffer| for more info
/// about unloaded buffers.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid and loaded, false otherwise.
// DLLEXPORT Boolean nvim_buf_is_loaded(Buffer buf)
//   FUNC_API_SINCE(5);
pub fn nvim_buf_is_loaded(buf: Buffer) -> Boolean {
    unsafe {
        let mut stub = Error::new();
        let b = find_buffer_by_handle(buf, &mut stub);
        api_clear_error(&mut stub);
        !b.is_null() && (*b).b_ml.ml_mfp.is_some()
    }
}

/// Deletes a buffer and its metadata (like |:bwipeout|).
///
/// To get |:bdelete| behavior, reset 'buflisted' and pass `unload=true`:
/// ```lua
/// vim.bo.buflisted = false
/// vim.api.nvim_buf_delete(0, { unload = true })
/// ```
///
/// @param buf Buffer id, or 0 for current buffer
/// @param opts  Optional parameters. Keys:
///          - force:  Force deletion, ignore unsaved changes.
///          - unload: Unloaded only (|:bunload|), do not delete.
// DLLEXPORT void nvim_buf_delete(Buffer buf, Dict(buf_delete) *opts, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_TEXTLOCK;
pub fn nvim_buf_delete(buf: Buffer, opts: &mut KeyDict_buf_delete, err: &mut Error) {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return;
    };

    let force = opts.force;
    let unload = opts.unload;

    let result = do_buffer(
        if unload { DOBUF_UNLOAD } else { DOBUF_WIPE },
        DOBUF_FIRST,
        FORWARD,
        b.b_fnum,
        i32::from(force),
    );

    if !result {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Failed to unload buffer.",
        );
    }
}

/// Checks if a buffer is valid.
///
/// @note Even if a buffer is valid it may have been unloaded. See |api-buffer|
/// for more info about unloaded buffers.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid, false otherwise.
// DLLEXPORT Boolean nvim_buf_is_valid(Buffer buf)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_is_valid(buf: Buffer) -> Boolean {
    let mut stub = Error::new();
    let ret = !find_buffer_by_handle(buf, &mut stub).is_null();
    api_clear_error(&mut stub);
    ret
}

/// Deletes a named mark in the buffer. See |mark-motions|.
///
/// @note only deletes marks set in the buffer, if the mark is not set
/// in the buffer it will return false.
/// @param buf     Buffer to set the mark on
/// @param name       Mark name
/// @return true if the mark was deleted, else false.
/// @see |nvim_buf_set_mark()|
/// @see |nvim_del_mark()|
// DLLEXPORT Boolean nvim_buf_del_mark(Buffer buf, String name, Error *err)
//   FUNC_API_SINCE(8);
pub fn nvim_buf_del_mark(buf: Buffer, name: NvimString, err: &mut Error) -> Boolean {
    let mut res = false;
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return res;
    };

    if name.size != 1 {
        api_err_invalid(
            err,
            c"mark name (must be a single char)",
            name.data,
            0,
            true,
        );
        return res;
    }

    let fm = mark_get(
        b,
        curwin_mut(),
        None,
        K_MARK_ALL_NO_RESOLVE,
        i32::from(unsafe { *name.data.cast::<u8>() }),
    );
    if fm.is_null() {
        api_err_invalid(err, c"mark name", name.data, 0, true);
        return res;
    }
    let fm = unsafe { &mut *fm };

    if fm.mark.lnum != 0 && fm.fnum == b.b_fnum {
        res = set_mark((&raw mut *b), name, 0, 0, err);
    }

    res
}

/// Sets a named mark in the given buffer, all marks are allowed
/// file/uppercase, visual, last change, etc. See |mark-motions|.
///
/// Marks are (1,0)-indexed. |api-indexing|
///
/// @note Passing 0 as line deletes the mark
///
/// @param buf     Buffer to set the mark on
/// @param name       Mark name
/// @param line       Line number
/// @param col        Column/row number
/// @param opts       Optional parameters. Reserved for future use.
/// @return true if the mark was set, else false.
/// @see |nvim_buf_del_mark()|
/// @see |nvim_buf_get_mark()|
// DLLEXPORT Boolean nvim_buf_set_mark(Buffer buf, String name, Integer line, Integer col,
//                                     Dict(empty) *opts, Error *err)
//   FUNC_API_SINCE(8);
pub fn nvim_buf_set_mark(
    buf: Buffer,
    name: NvimString,
    line: Integer,
    col: Integer,
    _opts: &mut KeyDict_empty,
    err: &mut Error,
) -> Boolean {
    let b = api_buf_ensure_loaded(buf, err);
    if b.is_null() {
        return false;
    }

    if name.size != 1 {
        api_err_invalid(
            err,
            c"mark name (must be a single char)",
            name.data,
            0,
            true,
        );
        return false;
    }

    set_mark(b, name, line, col, err)
}

/// Returns a `(row,col)` tuple representing the position of the named mark.
/// "End of line" column position is returned as |v:maxcol| (big number).
/// See |mark-motions|.
///
/// Marks are (1,0)-indexed. |api-indexing|
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Mark name
/// @param[out] err   Error details, if any
/// @return (row, col) tuple, (0, 0) if the mark is not set, or is an
/// uppercase/file mark set in another buffer.
/// @see |nvim_buf_set_mark()|
/// @see |nvim_buf_del_mark()|
// DLLEXPORT ArrayOf(Integer, 2) nvim_buf_get_mark(Buffer buf, String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_buf_get_mark(
    buf: Buffer,
    name: NvimString,
    arena: &mut Arena,
    err: &mut Error,
) -> Array {
    let mut rv = Array::new();
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return rv;
    };

    if name.size != 1 {
        api_err_invalid(
            err,
            c"mark name (must be a single char)",
            name.data,
            0,
            true,
        );
        return rv;
    }

    let mark = i32::from(unsafe { *name.data.cast::<u8>() });
    let fm = mark_get(b, curwin_mut(), None, K_MARK_ALL_NO_RESOLVE, mark);
    if fm.is_null() {
        api_err_invalid(err, c"mark name", name.data, 0, true);
        return rv;
    }

    let (lnum, col) = if unsafe { (*fm).fnum } != b.b_fnum {
        (0, 0)
    } else {
        (unsafe { (*fm).mark.lnum }, unsafe { (*fm).mark.col })
    };

    rv = arena_array(arena, 2);
    add_c(&mut rv, integer_obj(i64::from(lnum)));
    add_c(&mut rv, integer_obj(i64::from(col)));
    rv
}

/// Calls function `fn` in the context of buffer `buf` and returns its result (may be multiple
/// values).
///
/// This temporarily switches current buffer to `buf`.
/// If the current window already shows `buf`, the window is not switched.
/// If a window inside the current tabpage (including a float) already shows the
/// buffer, then one of those windows will be set as current window temporarily.
/// Otherwise a temporary scratch window (called the "autocmd window" for
/// historical reasons) will be used.
///
/// This is useful e.g. to call Vimscript functions that only work with the
/// current buffer/window currently, like `jobstart(…, {'term': v:true})`.
///
/// @param buf  Buffer id, or 0 for current buffer.
/// @param fn   Lua function to call inside the buffer.
/// @param err  Error details, if any.
/// @return     Value(s) returned by `fn()`.
// DLLEXPORT Object nvim_buf_call(Buffer buf, LuaRef fn, lua_State *lstate, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_LUA_ONLY;
pub fn nvim_buf_call(
    buf: Buffer,
    fn_ref: LuaRef,
    _lstate: *mut lua_State,
    err: &mut Error,
) -> Object {
    let b = find_buffer_by_handle(buf, err);
    if b.is_null() {
        return OBJECT_NIL;
    }

    try_wrap_err(err, |err| {
        let mut cs: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut cs, ptr::null_mut(), ptr::null_mut(), b.cast(), 0);

        let args = Array::new();
        nlua_call_ref(
            fn_ref,
            ptr::null(),
            args,
            K_RET_MULTI_STACK,
            None,
            Some(err),
        );

        ctx_restore(&mut cs);
    });

    OBJECT_NIL
}

/// @nodoc
// DLLEXPORT Dict nvim__buf_stats(Buffer buf, Arena *arena, Error *err);
pub fn nvim_buf_stats(buf: Buffer, arena: &mut Arena, err: &mut Error) -> Dict {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
        return Dict::new();
    };

    let mut rv = arena_dict(arena, 7);
    put_c(
        &mut rv,
        c"flush_count",
        integer_obj(i64::from(b.flush_count)),
    );
    put_c(
        &mut rv,
        c"current_lnum",
        integer_obj(i64::from(b.b_ml.ml_line_lnum)),
    );
    put_c(
        &mut rv,
        c"line_dirty",
        boolean_obj((b.b_ml.ml_flags & ML_LINE_DIRTY) != 0),
    );
    put_c(
        &mut rv,
        c"dirty_bytes",
        integer_obj(b.deleted_bytes as Integer),
    );
    put_c(
        &mut rv,
        c"dirty_bytes2",
        integer_obj(b.deleted_bytes2 as Integer),
    );
    put_c(
        &mut rv,
        c"virt_blocks",
        integer_obj(b.b_marktree.meta_root[K_MTMETA_LINES] as Integer),
    );

    let uhp = b.b_u_curhead.or(b.b_u_newhead);
    if let Some(uh) = uhp {
        put_c(
            &mut rv,
            c"uhp_extmark_size",
            integer_obj(uhp_ref(b, uh).uh_extmark.len() as Integer),
        );
    }

    rv
}

fn fix_cursor(win: &mut WinInfoInfo, lo: Linenr, hi: Linenr, extra: Linenr) {
    if win.w_cursor.lnum >= lo {
        if win.w_cursor.lnum >= hi {
            win.w_cursor.lnum += extra;
        } else if extra < 0 {
            check_cursor_lnum(&mut *win);
        }
        check_cursor_col(&mut *win);
        changed_cline_bef_curs(win);
        win.w_valid &= !VALID_BOTLINE_AP;
        update_topline(win);
    } else {
        invalidate_botline_win(&mut *win);
    }
}

fn fix_pos_col(
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    start_row: Linenr,
    start_col: Colnr,
    end_row: Linenr,
    end_col: Colnr,
    new_rows: Linenr,
    new_cols_at_end_row: Colnr,
    mode_col_adj: Colnr,
) {
    if pos.lnum < start_row {
        return;
    }

    let old_rows = end_row - start_row + 1;
    let lnum_shift = new_rows - old_rows;

    if pos.lnum > end_row {
        pos.lnum += lnum_shift;
        return;
    }

    let end_row_change_start = if new_rows == 1 { start_col } else { 0 };
    let end_row_change_end = end_row_change_start + new_cols_at_end_row;

    if pos.lnum == end_row && pos.col + mode_col_adj > end_col {
        pos.lnum += lnum_shift;
        pos.col += end_row_change_end - end_col;
        return;
    }

    let old_coladd = pos.coladd;
    pos.col += pos.coladd;
    pos.coladd = 0;

    let new_end_row = start_row + new_rows - 1;

    if pos.lnum > new_end_row {
        pos.lnum = new_end_row;
        let len = ml_get_buf_len(buf, new_end_row);
        if pos.col < len {
            pos.col = len;
        }
    }

    if pos.lnum == new_end_row && pos.col > end_row_change_end && old_coladd == 0 {
        pos.col = end_row_change_end;
        if pos.col - mode_col_adj >= end_row_change_start {
            pos.col -= mode_col_adj;
        }
    }
}

fn fix_cursor_cols(
    win: &mut WinInfoInfo,
    start_row: Linenr,
    start_col: Colnr,
    end_row: Linenr,
    end_col: Colnr,
    new_rows: Linenr,
    new_cols_at_end_row: Colnr,
) {
    let mode_col_adj = if ptr::eq(win, curwin_ptr()) && (g_st().state_global & MODE_INSERT) != 0 {
        0
    } else {
        1
    };
    fix_pos_col(
        win.buf_mut(),
        &mut win.w_cursor,
        start_row,
        start_col,
        end_row,
        end_col,
        new_rows,
        new_cols_at_end_row,
        mode_col_adj,
    );

    check_cursor_col(&mut *win);
    changed_cline_bef_curs(win);
    invalidate_botline_win(&mut *win);
}

#[inline]
fn init_line_array(lstate: *mut lua_State, a: &mut Array, size: usize, arena: &mut Arena) {
    if !lstate.is_null() {
        unsafe { lua_createtable(lstate, size as i32, 0) };
    } else {
        *a = arena_array(arena, size);
    }
}

fn push_linestr(
    lstate: *mut lua_State,
    a: &mut Array,
    s: &[u8],
    idx: i32,
    replace_nl: bool,
    arena: &mut Arena,
) {
    if !lstate.is_null() {
        if replace_nl && s.contains(&b'\n') {
            let mut tmp: Vec<u8> = s.to_vec();
            for b in &mut tmp {
                if *b == b'\n' {
                    *b = 0;
                }
            }
            lua_push_bytes(lstate, &tmp);
        } else {
            lua_push_bytes(lstate, s);
        }
        unsafe { lua_rawseti(lstate, -2, idx + 1) };
    } else {
        let mut str_val = NvimString::new();
        if !s.is_empty() {
            str_val = arena_bytes(arena, s);
            if replace_nl {
                let slice = unsafe { std::slice::from_raw_parts_mut(str_val.data.cast::<u8>(), s.len()) };
                for b in slice {
                    if *b == b'\n' {
                        *b = 0;
                    }
                }
            }
        }
        add_c(a, string_obj(str_val));
    }
}

/// Collects `n` buffer lines into array `l` and/or lua_State `lstate`, optionally replacing
/// newlines with NUL.
///
/// @param buf Buffer to get lines from
/// @param n Number of lines to collect
/// @param replace_nl Replace newlines ("\n") with NUL
/// @param start Line number to start from
/// @param start_idx First index to push to (only used for Lua)
/// @param[out] l If not NULL, Lines are copied here
/// @param[out] lstate If not NULL, Lines are pushed into a table onto the stack
/// @param err[out] Error, if any
/// @return true unless `err` was set
// DLLEXPORT void buf_collect_lines(FileBuf *buf, size_t n, Linenr start, int start_idx,
//                                  bool replace_nl, Array *l, lua_State *lstate, Arena *arena);
pub fn buf_collect_lines(
    buf: &mut FileBuf,
    n: usize,
    start: Linenr,
    start_idx: i32,
    replace_nl: bool,
    l: &mut Array,
    lstate: *mut lua_State,
    arena: &mut Arena,
) {
    for i in 0..n {
        let lnum = start + (i as Linenr);
        let bufstr = ml_get_buf(buf, lnum);
        let bufstrlen = ml_get_buf_len(buf, lnum) as usize;
        let line_bytes = if bufstrlen == 0 || bufstr.is_null() {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(bufstr.cast::<u8>(), bufstrlen) }
        };
        push_linestr(
            lstate,
            l,
            line_bytes,
            start_idx + (i as i32),
            replace_nl,
            arena,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::validate::ObjectType;

    #[test]
    fn internal_call_constants_and_boolean_obj() {
        assert_eq!(VIML_INTERNAL_CALL, 1u64 << 63);
        assert_eq!(LUA_INTERNAL_CALL, (1u64 << 63) + 1);
        let obj = boolean_obj(true);
        assert_eq!(obj.obj_type, ObjectType::ObjectTypeBoolean);
        assert!(obj.data.boolean());
    }
}
