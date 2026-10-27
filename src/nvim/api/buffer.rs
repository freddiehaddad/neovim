//! Buffer API functions (`src/nvim/api/buffer.c`).

use crate::api::deprecated::KeyDict_empty;
use crate::api::private::helpers::{
    MAXLNUM, OBJECT_NIL, add_c, api_clear_error, api_set_error, arena_array, arena_dict,
    boolean_obj, buf_get_text, cstr_as_string, dict_get_value, dict_set_var, error_set,
    find_buffer_by_handle, has_key, integer_obj, normalize_index, put_c, set_mark,
    static_cstr_as_obj, string_obj, try_wrap, try_wrap_err,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, api_err_invalid, api_err_range,
    check_string_array,
};
use crate::api::tabpage::Buffer;
use crate::buffer::{
    BufUpdateCallbacks, DOBUF_FIRST, DOBUF_UNLOAD, DOBUF_WIPE, buf_T, buf_ensure_loaded, do_buffer,
};
use crate::buffer_updates::{buf_updates_register, buf_updates_unregister};
use crate::change::changed_lines;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{
    check_cursor_col, check_cursor_lnum, check_visual_pos, colnr_T, linenr_T, pos_T,
};
use crate::eval::vars::MAXCOL;
use crate::ex_cmds::rename_buffer;
use crate::extmark::{bcount_t, extmark_splice, kExtmarkNOOP, kExtmarkUndo};
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{
    Boolean, LUA_NOREF, LuaRef, lua_State, lua_createtable, lua_pushlstring, lua_rawseti,
};
use crate::lua::executor::{kRetMultiStack, nlua_call_ref};
use crate::mapping::{MAPTYPE_MAP, MAPTYPE_UNMAP, MAPTYPE_UNMAP_LHS, keymap_array, modify_keymap};
use crate::mark::{kMarkAdjustApi, kMarkAllNoResolve, mark_adjust_buf, mark_get};
use crate::marktree::kMTMetaLines;
use crate::memline::{
    ML_LINE_DIRTY, ml_append_buf, ml_delete_buf, ml_find_line_or_offset, ml_get_buf,
    ml_get_buf_len, ml_replace_buf,
};
use crate::memory::{Arena, arena_alloc, arena_allocz, arena_memdupz};
use crate::r#move::{changed_cline_bef_curs, invalidate_botline_win, update_topline};
use crate::msgpack_rpc::packer::Integer;
use crate::ops::get_region_bytecount;
use crate::option_vars::*;
use crate::undo::{u_save_buf, uhp_ref};
use crate::window::{VALID_BOTLINE_AP, win_T};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::charset::{NL, NUL};
use crate::search::FORWARD;

pub(crate) const INTERNAL_CALL_MASK: u64 = 1u64 << 63;
pub(crate) const VIML_INTERNAL_CALL: u64 = INTERNAL_CALL_MASK;
pub(crate) const LUA_INTERNAL_CALL: u64 = VIML_INTERNAL_CALL + 1;

const KEYSET_OPTIDX_buf_attach__on_bytes: u64 = 2;
const KEYSET_OPTIDX_buf_attach__on_lines: u64 = 3;
const KEYSET_OPTIDX_buf_attach__on_detach: u64 = 5;
const KEYSET_OPTIDX_buf_attach__on_reload: u64 = 6;
const KEYSET_OPTIDX_buf_attach__on_changedtick: u64 = 7;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_buf_attach {
    pub is_set__buf_attach_: OptionalKeys,
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
    pub is_set__keymap_: OptionalKeys,
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
    pub is_set__keymap_del_: OptionalKeys,
    pub lhs: Boolean,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_buf_delete {
    pub is_set__buf_delete_: OptionalKeys,
    pub force: Boolean,
    pub unload: Boolean,
}

use crate::math::{FAIL, OK};
use crate::regexp::Ctrl_V;
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
// DLLEXPORT buf_T *api_buf_ensure_loaded(Buffer buf, Error *err);
pub unsafe fn api_buf_ensure_loaded(buf: Buffer, err: &mut Error) -> *mut buf_T {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return ptr::null_mut();
        }

        if (*b).b_ml.ml_mfp.is_none() && !buf_ensure_loaded(&mut *b) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to load buffer".as_ptr(),
            );
            return ptr::null_mut();
        }

        b
    }
}

/// Returns the number of lines in the given buffer.
///
/// @param buf   Buffer id, or 0 for current buffer
/// @param[out] err Error details, if any
/// @return Line count, or 0 for unloaded buffer. |api-buffer|
// DLLEXPORT Integer nvim_buf_line_count(Buffer buf, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_line_count(buf: Buffer, err: &mut Error) -> Integer {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_attach(
    channel_id: u64,
    buf: Buffer,
    send_buffer: Boolean,
    opts: *mut KeyDict_buf_attach,
    err: &mut Error,
) -> Boolean {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return false;
        }

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
            let mask = (*opts).is_set__buf_attach_;
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_lines) {
                cb.on_lines = (*opts).on_lines;
                (*opts).on_lines = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_bytes) {
                cb.on_bytes = (*opts).on_bytes;
                (*opts).on_bytes = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_changedtick) {
                cb.on_changedtick = (*opts).on_changedtick;
                (*opts).on_changedtick = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_detach) {
                cb.on_detach = (*opts).on_detach;
                (*opts).on_detach = LUA_NOREF;
            }
            if has_key(mask, KEYSET_OPTIDX_buf_attach__on_reload) {
                cb.on_reload = (*opts).on_reload;
                (*opts).on_reload = LUA_NOREF;
            }
            cb.utf_sizes = (*opts).utf_sizes;
            cb.preview = (*opts).preview;
        }

        buf_updates_register(&mut *b, channel_id, cb, send_buffer)
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_detach(channel_id: u64, buf: Buffer, err: &mut Error) -> Boolean {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return false;
        }
        buf_updates_unregister(b, channel_id);
        true
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    arena: *mut Arena,
    lstate: *mut lua_State,
    err: &mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if (*b).b_ml.ml_mfp.is_none() {
            init_line_array(lstate, &mut rv, 0, arena);
            return rv;
        }

        let mut oob = false;
        start = normalize_index(&mut *b, start, true, &mut oob);
        end = normalize_index(&mut *b, end, true, &mut oob);

        if strict_indexing && oob {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
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
            b,
            size,
            start as linenr_T,
            0,
            channel_id != VIML_INTERNAL_CALL,
            &mut rv,
            lstate,
            arena,
        );

        rv
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_lines(
    channel_id: u64,
    buf: Buffer,
    mut start: Integer,
    mut end: Integer,
    strict_indexing: Boolean,
    replacement: Array,
    arena: *mut Arena,
    err: &mut Error,
) {
    unsafe {
        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return;
        }

        let mut oob = false;
        start = normalize_index(&mut *b, start, true, &mut oob);
        end = normalize_index(&mut *b, end, true, &mut oob);

        if strict_indexing && oob {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return;
        }
        if start > end {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return;
        }

        let disallow_nl = channel_id != VIML_INTERNAL_CALL;
        if !check_string_array(
            replacement,
            c"replacement string".as_ptr().cast_mut(),
            disallow_nl,
            err,
        ) {
            return;
        }

        let new_len = replacement.size;
        let old_len = (end - start) as usize;
        let mut extra: isize = 0;
        let lines = if new_len != 0 {
            arena_alloc(arena, new_len * std::mem::size_of::<*mut c_char>(), true)
                .cast::<*mut c_char>()
        } else {
            ptr::null_mut()
        };

        for i in 0..new_len {
            let l = replacement[i].data.string();
            let p = arena_memdupz(arena, l.data, l.size);
            { let _d = (p).cast::<u8>(); let _sz = l.size; if _sz > 0 { let _c = (NUL) as u8; let _x = (NL) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
            *lines.add(i) = p;
        }

        try_wrap_err(err, |err| 'end: {
            if (*b).b_p_ma == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Buffer is not 'modifiable'".as_ptr(),
                );
                break 'end;
            }

            if u_save_buf(&mut *b, (start - 1) as linenr_T, end as linenr_T) == FAIL {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to save undo information".as_ptr(),
                );
                break 'end;
            }

            let deleted_bytes = get_region_bytecount(&mut *b, start as linenr_T, end as linenr_T, 0, 0);

            let to_delete = old_len.saturating_sub(new_len);
            for _ in 0..to_delete {
                if ml_delete_buf(b, start as linenr_T, false) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to delete line".as_ptr(),
                    );
                    break 'end;
                }
            }

            if to_delete > 0 {
                extra -= to_delete as isize;
            }

            let to_replace = old_len.min(new_len);
            let mut inserted_bytes: bcount_t = 0;
            for i in 0..to_replace {
                let lnum = start + (i as i64);
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                let line_ptr = *lines.add(i);
                if ml_replace_buf(b, lnum as linenr_T, line_ptr, false, true) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to replace line".as_ptr(),
                    );
                    break 'end;
                }

                inserted_bytes += CStr::from_ptr(line_ptr).to_bytes().len() as bcount_t + 1;
            }

            for i in to_replace..new_len {
                let lnum = start + (i as i64) - 1;
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                let line_ptr = *lines.add(i);
                if ml_append_buf(b, lnum as linenr_T, line_ptr, 0, false) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to insert line".as_ptr(),
                    );
                    break 'end;
                }

                inserted_bytes += CStr::from_ptr(line_ptr).to_bytes().len() as bcount_t + 1;
                extra += 1;
            }

            let adjust = if end > start { MAXLNUM as linenr_T } else { 0 };
            mark_adjust_buf(
                b,
                start as linenr_T,
                (end - 1) as linenr_T,
                adjust,
                extra as linenr_T,
                true,
                kMarkAdjustApi,
                kExtmarkNOOP,
            );

            if g_st().Visual.active && b == g_st().curbuf && g_st().Visual.start.lnum >= (start as linenr_T) {
                if g_st().Visual.start.lnum >= (end as linenr_T) {
                    g_st().Visual.start.lnum += extra as linenr_T;
                }
                check_visual_pos();
            }

            extmark_splice(
                b,
                (start as c_int) - 1,
                0,
                (end - start) as c_int,
                0,
                deleted_bytes,
                new_len as c_int,
                0,
                inserted_bytes,
                kExtmarkUndo,
            );

            changed_lines(
                b,
                start as linenr_T,
                0,
                end as linenr_T,
                extra as linenr_T,
                true,
            );

            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                let mut win = if tp == g_st().curtab {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == b {
                        fix_cursor(&mut *win, start as linenr_T, end as linenr_T, extra as linenr_T);
                    }
                    win = (*win).w_next;
                }
                tp = (*tp).tp_next;
            }
        });
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    mut start_col: Integer,
    mut end_row: Integer,
    mut end_col: Integer,
    mut replacement: Array,
    arena: *mut Arena,
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
            api_err_range(err, c"start_row".as_ptr());
            return;
        }

        end_row = normalize_index(&mut *b, end_row, false, &mut oob);
        if oob {
            api_err_range(err, c"end_row".as_ptr());
            return;
        }

        let str_at_start_raw = ml_get_buf(b, start_row as linenr_T);
        let len_at_start = ml_get_buf_len(b, start_row as linenr_T);
        let str_at_start = arena_memdupz(arena, str_at_start_raw, len_at_start as usize);
        if start_col < 0 {
            start_col = i64::from(len_at_start) + start_col + 1;
        }
        if start_col < 0 || start_col > i64::from(len_at_start) {
            api_err_range(err, c"start_col".as_ptr());
            return;
        }

        let str_at_end_raw = ml_get_buf(b, end_row as linenr_T);
        let len_at_end = ml_get_buf_len(b, end_row as linenr_T);
        let str_at_end = arena_memdupz(arena, str_at_end_raw, len_at_end as usize);
        if end_col < 0 {
            end_col = i64::from(len_at_end) + end_col + 1;
        }
        if end_col < 0 || end_col > i64::from(len_at_end) {
            api_err_range(err, c"end_col".as_ptr());
            return;
        }

        if start_row > end_row || (end_row == start_row && start_col > end_col) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return;
        }

        let disallow_nl = channel_id != VIML_INTERNAL_CALL;
        if !check_string_array(
            replacement,
            c"replacement string".as_ptr().cast_mut(),
            disallow_nl,
            err,
        ) {
            return;
        }

        let new_len = replacement.size;
        let mut new_byte: bcount_t = 0;
        let mut old_byte: bcount_t = 0;

        if start_row == end_row {
            old_byte = (end_col - start_col) as bcount_t;
        } else {
            old_byte += (i64::from(len_at_start) - start_col) as bcount_t;
            for i in 1..(end_row - start_row) {
                let lnum = start_row + i;
                old_byte += (ml_get_buf_len(b, lnum as linenr_T) as bcount_t) + 1;
            }
            old_byte += (end_col as bcount_t) + 1;
        }

        let first_item = replacement[0].data.string();
        let last_item = (*replacement.items.add(replacement.size - 1)).data.string();

        let mut firstlen = (start_col as usize) + first_item.size;
        let last_part_len = (len_at_end as usize) - (end_col as usize);
        if replacement.size == 1 {
            firstlen += last_part_len;
        }
        let first = arena_allocz(arena, firstlen).cast::<c_char>();
        let mut last: *mut c_char = ptr::null_mut();
        ptr::copy_nonoverlapping(str_at_start, first, start_col as usize);
        if first_item.size > 0 {
            ptr::copy_nonoverlapping(
                first_item.data,
                first.add(start_col as usize),
                first_item.size,
            );
        }
        { let _d = (first.add(start_col as usize)).cast::<u8>(); let _sz = first_item.size; if _sz > 0 { let _c = (NUL) as u8; let _x = (NL) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
        if replacement.size == 1 {
            ptr::copy_nonoverlapping(
                str_at_end.add(end_col as usize),
                first.add((start_col as usize) + first_item.size),
                last_part_len,
            );
        } else {
            last = arena_allocz(arena, last_item.size + last_part_len).cast::<c_char>();
            if last_item.size > 0 {
                ptr::copy_nonoverlapping(last_item.data, last, last_item.size);
            }
            { let _d = (last).cast::<u8>(); let _sz = last_item.size; if _sz > 0 { let _c = (NUL) as u8; let _x = (NL) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
            ptr::copy_nonoverlapping(
                str_at_end.add(end_col as usize),
                last.add(last_item.size),
                last_part_len,
            );
        }

        let lines = arena_alloc(arena, new_len * std::mem::size_of::<*mut c_char>(), true)
            .cast::<*mut c_char>();
        *lines.add(0) = first;
        new_byte += first_item.size as bcount_t;
        for i in 1..new_len.saturating_sub(1) {
            let l = replacement[i].data.string();
            let p = arena_memdupz(arena, l.data, l.size);
            { let _d = (p).cast::<u8>(); let _sz = l.size; if _sz > 0 { let _c = (NUL) as u8; let _x = (NL) as u8; for _b in std::slice::from_raw_parts_mut(_d, _sz) { if *_b == _c { *_b = _x; } } } };
            *lines.add(i) = p;
            new_byte += (l.size as bcount_t) + 1;
        }
        if replacement.size > 1 {
            *lines.add(replacement.size - 1) = last;
            new_byte += (last_item.size as bcount_t) + 1;
        }

        try_wrap_err(err, |err| 'end: {
            if (*b).b_p_ma == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Buffer is not 'modifiable'".as_ptr(),
                );
                break 'end;
            }

            if u_save_buf(&mut *b, (start_row as linenr_T) - 1, (end_row as linenr_T) + 1) == FAIL {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to save undo information".as_ptr(),
                );
                break 'end;
            }

            let mut extra: isize = 0;
            let old_len = (end_row - start_row + 1) as usize;

            let to_delete = old_len.saturating_sub(new_len);
            for _ in 0..to_delete {
                if ml_delete_buf(b, start_row as linenr_T, false) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to delete line".as_ptr(),
                    );
                    break 'end;
                }
            }

            if to_delete > 0 {
                extra -= to_delete as isize;
            }

            let to_replace = old_len.min(new_len);
            for i in 0..to_replace {
                let lnum = start_row + (i as i64);
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                if ml_replace_buf(b, lnum as linenr_T, *lines.add(i), false, true) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to replace line".as_ptr(),
                    );
                    break 'end;
                }
            }

            for i in to_replace..new_len {
                let lnum = start_row + (i as i64) - 1;
                if lnum >= MAXLNUM {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Index out of bounds".as_ptr(),
                    );
                    break 'end;
                }

                if ml_append_buf(b, lnum as linenr_T, *lines.add(i), 0, false) == FAIL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to insert line".as_ptr(),
                    );
                    break 'end;
                }

                extra += 1;
            }

            let col_extent =
                (end_col - if end_row == start_row { start_col } else { 0 }) as colnr_T;

            let adjust = if end_row >= start_row {
                MAXLNUM as linenr_T
            } else {
                0
            };
            mark_adjust_buf(
                b,
                start_row as linenr_T,
                (end_row as linenr_T) - 1,
                adjust,
                extra as linenr_T,
                true,
                kMarkAdjustApi,
                kExtmarkNOOP,
            );

            if g_st().Visual.active && b == g_st().curbuf && g_st().Visual.mode != Ctrl_V {
                fix_pos_col(
                    b,
                    &mut g_st().Visual.start,
                    start_row as linenr_T,
                    start_col as colnr_T,
                    end_row as linenr_T,
                    end_col as colnr_T,
                    new_len as linenr_T,
                    last_item.size as colnr_T,
                    1,
                );
                check_visual_pos();
            }

            extmark_splice(
                b,
                (start_row as c_int) - 1,
                start_col as colnr_T,
                (end_row - start_row) as c_int,
                col_extent,
                old_byte,
                (new_len as c_int) - 1,
                last_item.size as colnr_T,
                new_byte,
                kExtmarkUndo,
            );

            changed_lines(
                b,
                start_row as linenr_T,
                start_col as colnr_T,
                (end_row as linenr_T) + 1,
                extra as linenr_T,
                true,
            );

            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                let mut win = if tp == g_st().curtab {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !win.is_null() {
                    if (*win).w_buffer == b {
                        if i64::from((*win).w_cursor.lnum) >= start_row
                            && i64::from((*win).w_cursor.lnum) <= end_row
                        {
                            fix_cursor_cols(
                                &mut *win,
                                start_row as linenr_T,
                                start_col as colnr_T,
                                end_row as linenr_T,
                                end_col as colnr_T,
                                new_len as linenr_T,
                                last_item.size as colnr_T,
                            );
                        } else {
                            fix_cursor(
                                &mut *win,
                                start_row as linenr_T,
                                end_row as linenr_T,
                                extra as linenr_T,
                            );
                        }
                    }
                    win = (*win).w_next;
                }
                tp = (*tp).tp_next;
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_text(
    channel_id: u64,
    buf: Buffer,
    mut start_row: Integer,
    start_col: Integer,
    mut end_row: Integer,
    end_col: Integer,
    _opts: *mut KeyDict_empty,
    arena: *mut Arena,
    lstate: *mut lua_State,
    err: &mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if (*b).b_ml.ml_mfp.is_none() {
            init_line_array(lstate, &mut rv, 0, arena);
            return rv;
        }

        let mut oob = false;
        start_row = normalize_index(&mut *b, start_row, false, &mut oob);
        end_row = normalize_index(&mut *b, end_row, false, &mut oob);

        if oob {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return rv;
        }

        if start_row > end_row {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start' is higher than 'end'".as_ptr(),
            );
            return rv;
        }

        let replace_nl = channel_id != VIML_INTERNAL_CALL;
        let size = ((end_row - start_row) as usize) + 1;

        init_line_array(lstate, &mut rv, size, arena);

        if start_row == end_row {
            let line = buf_get_text(b, start_row, start_col, end_col, err);
            if error_set(err) {
                return Array::new();
            }
            push_linestr(lstate, &mut rv, line.data, line.size, 0, replace_nl, arena);
            return rv;
        }

        let mut str_val = buf_get_text(b, start_row, start_col, MAXCOL - 1, err);
        if error_set(err) {
            return Array::new();
        }

        push_linestr(
            lstate,
            &mut rv,
            str_val.data,
            str_val.size,
            0,
            replace_nl,
            arena,
        );

        if size > 2 {
            buf_collect_lines(
                b,
                size - 2,
                (start_row as linenr_T) + 1,
                1,
                replace_nl,
                &mut rv,
                lstate,
                arena,
            );
        }

        str_val = buf_get_text(b, end_row, 0, end_col, err);
        if error_set(err) {
            return Array::new();
        }

        push_linestr(
            lstate,
            &mut rv,
            str_val.data,
            str_val.size,
            (size - 1) as c_int,
            replace_nl,
            arena,
        );

        if error_set(err) {
            return Array::new();
        }

        rv
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_offset(
    buf: Buffer,
    index: Integer,
    err: &mut Error,
) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return 0;
        }

        if (*b).b_ml.ml_mfp.is_none() {
            return -1;
        }

        if index < 0 || index > i64::from((*b).b_ml.ml_line_count) {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Index out of bounds".as_ptr(),
            );
            return 0;
        }

        i64::from(ml_find_line_or_offset(
            b,
            (index as linenr_T) + 1,
            ptr::null_mut(),
            true,
        ))
    }
}

/// Gets a buffer-scoped (b:) variable.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_buf_get_var(Buffer buf, String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_var(
    buf: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Object {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return OBJECT_NIL;
        }
        dict_get_value((*b).b_vars(), name, arena, err)
    }
}

/// Gets a changed tick of a buffer
///
/// @param[in]  buf  Buffer id, or 0 for current buffer
/// @param[out] err     Error details, if any
///
/// @return `b:changedtick` value.
// DLLEXPORT Integer nvim_buf_get_changedtick(Buffer buf, Error *err)
//   FUNC_API_SINCE(2);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_changedtick(buf: Buffer, err: &mut Error) -> Integer {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return -1;
        }
        (*b).changedtick_di.di_tv.vval.v_number()
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_keymap(
    buf: Buffer,
    mode: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Array {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return Array::new();
        }
        keymap_array(mode, b, arena)
    }
}

/// Sets a buffer-local |mapping| for the given mode.
///
/// @see |nvim_set_keymap()|
///
/// @param  buf  Buffer id, or 0 for current buffer
// DLLEXPORT void nvim_buf_set_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
//                                    String rhs, Dict(keymap) *opts, Error *err)
//   FUNC_API_SINCE(6);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    rhs: NvimString,
    opts: *mut KeyDict_keymap,
    err: &mut Error,
) {
    unsafe {
        modify_keymap(channel_id, buf, MAPTYPE_MAP, mode, lhs, rhs, opts, err);
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_del_keymap(
    channel_id: u64,
    buf: Buffer,
    mode: NvimString,
    lhs: NvimString,
    opts: *mut KeyDict_keymap_del,
    err: &mut Error,
) {
    unsafe {
        let rhs = NvimString {
            data: c"".as_ptr() as *mut c_char,
            size: 0,
        };
        let maptype = if !opts.is_null() && (*opts).lhs {
            MAPTYPE_UNMAP_LHS
        } else {
            MAPTYPE_UNMAP
        };
        modify_keymap(
            channel_id,
            buf,
            maptype,
            mode,
            lhs,
            rhs,
            ptr::null_mut(),
            err,
        );
    }
}

/// Sets a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param value      Variable value
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_set_var(Buffer buf, String name, Object value, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_var(
    buf: Buffer,
    name: NvimString,
    value: Object,
    err: &mut Error,
) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        dict_set_var(&mut *(*b).b_vars(), name, value, false, false, ptr::null_mut(), err);
    }
}

/// Removes a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_del_var(Buffer buf, String name, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_del_var(buf: Buffer, name: NvimString, err: &mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        dict_set_var(
            &mut *(*b).b_vars(),
            name,
            OBJECT_NIL,
            true,
            false,
            ptr::null_mut(),
            err,
        );
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_name(buf: Buffer, err: &mut Error) -> NvimString {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_name(buf: Buffer, name: NvimString, err: &mut Error) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }

        let mut ren_ret = OK;
        try_wrap(err, || {
            let is_curbuf = b == g_st().curbuf;
            let save_acd = opt_vars().p_acd;
            if !is_curbuf {
                g_st().RedrawingDisabled += 1;
                opt_vars().p_acd = 0;
            }

            let mut aco: CtxSwitch = CtxSwitch::new();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), b.cast(), 0);
            ren_ret = rename_buffer(name.data);
            ctx_restore(&mut aco);

            if !is_curbuf {
                g_st().RedrawingDisabled -= 1;
                opt_vars().p_acd = save_acd;
            }
        });

        if error_set(err) {
            return;
        }

        if ren_ret == FAIL {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to rename buffer".as_ptr(),
            );
        }
    }
}

/// Checks if a buffer is valid and loaded. See |api-buffer| for more info
/// about unloaded buffers.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid and loaded, false otherwise.
// DLLEXPORT Boolean nvim_buf_is_loaded(Buffer buf)
//   FUNC_API_SINCE(5);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_is_loaded(buf: Buffer) -> Boolean {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_delete(
    buf: Buffer,
    opts: *mut KeyDict_buf_delete,
    err: &mut Error,
) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if error_set(err) {
            return;
        }

        let force = (*opts).force;
        let unload = (*opts).unload;

        let result = do_buffer(
            if unload { DOBUF_UNLOAD } else { DOBUF_WIPE },
            DOBUF_FIRST,
            FORWARD,
            (*b).b_fnum,
            c_int::from(force),
        );

        if result == FAIL {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Failed to unload buffer.".as_ptr(),
            );
        }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_is_valid(buf: Buffer) -> Boolean {
    unsafe {
        let mut stub = Error::new();
        let ret = !find_buffer_by_handle(buf, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_del_mark(
    buf: Buffer,
    name: NvimString,
    err: &mut Error,
) -> Boolean {
    unsafe {
        let mut res = false;
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return res;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return res;
        }

        let fm = mark_get(
            &mut *b,
            *curwin.as_ptr(),
            ptr::null_mut(),
            kMarkAllNoResolve,
            c_int::from(*name.data.cast::<u8>()),
        );
        if fm.is_null() {
            api_err_invalid(err, c"mark name".as_ptr(), name.data, 0, true);
            return res;
        }

        if (*fm).mark.lnum != 0 && (*fm).fnum == (*b).b_fnum {
            res = set_mark(b, name, 0, 0, err);
        }

        res
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_set_mark(
    buf: Buffer,
    name: NvimString,
    line: Integer,
    col: Integer,
    _opts: *mut KeyDict_empty,
    err: &mut Error,
) -> Boolean {
    unsafe {
        let b = api_buf_ensure_loaded(buf, err);
        if b.is_null() {
            return false;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return false;
        }

        set_mark(b, name, line, col, err)
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_mark(
    buf: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: &mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if name.size != 1 {
            api_err_invalid(
                err,
                c"mark name (must be a single char)".as_ptr(),
                name.data,
                0,
                true,
            );
            return rv;
        }

        let mark = c_int::from(*name.data.cast::<u8>());
        let fm = mark_get(&mut *b, *curwin.as_ptr(), ptr::null_mut(), kMarkAllNoResolve, mark);
        if fm.is_null() {
            api_err_invalid(err, c"mark name".as_ptr(), name.data, 0, true);
            return rv;
        }

        let (lnum, col) = if (*fm).fnum != (*b).b_fnum {
            (0, 0)
        } else {
            ((*fm).mark.lnum, (*fm).mark.col)
        };

        rv = arena_array(arena, 2);
        add_c(&mut rv, integer_obj(i64::from(lnum)));
        add_c(&mut rv, integer_obj(i64::from(col)));
        rv
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_call(
    buf: Buffer,
    fn_ref: LuaRef,
    _lstate: *mut lua_State,
    err: &mut Error,
) -> Object {
    unsafe {
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
                kRetMultiStack,
                ptr::null_mut(),
                err,
            );

            ctx_restore(&mut cs);
        });

        OBJECT_NIL
    }
}

/// @nodoc
// DLLEXPORT Dict nvim__buf_stats(Buffer buf, Arena *arena, Error *err);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim__buf_stats(buf: Buffer, arena: *mut Arena, err: &mut Error) -> Dict {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return Dict::new();
        }

        let mut rv = arena_dict(arena, 7);
        put_c(
            &mut rv,
            c"flush_count",
            integer_obj(i64::from((*b).flush_count)),
        );
        put_c(
            &mut rv,
            c"current_lnum",
            integer_obj(i64::from((*b).b_ml.ml_line_lnum)),
        );
        put_c(
            &mut rv,
            c"line_dirty",
            boolean_obj(((*b).b_ml.ml_flags & ML_LINE_DIRTY) != 0),
        );
        put_c(
            &mut rv,
            c"dirty_bytes",
            integer_obj((*b).deleted_bytes as Integer),
        );
        put_c(
            &mut rv,
            c"dirty_bytes2",
            integer_obj((*b).deleted_bytes2 as Integer),
        );
        put_c(
            &mut rv,
            c"virt_blocks",
            integer_obj((*b).b_marktree.meta_root[kMTMetaLines] as Integer),
        );

        let uhp = (*b).b_u_curhead.or((*b).b_u_newhead);
        if let Some(uh) = uhp {
            put_c(
                &mut rv,
                c"uhp_extmark_size",
                integer_obj(uhp_ref(b, uh).uh_extmark.len() as Integer),
            );
        }

        rv
    }
}

unsafe fn fix_cursor(win: &mut win_T, lo: linenr_T, hi: linenr_T, extra: linenr_T) {
    unsafe {
        if win.w_cursor.lnum >= lo {
            if win.w_cursor.lnum >= hi {
                win.w_cursor.lnum += extra;
            } else if extra < 0 {
                check_cursor_lnum(&raw mut *win);
            }
            check_cursor_col(&raw mut *win);
            changed_cline_bef_curs(win);
            win.w_valid &= !VALID_BOTLINE_AP;
            update_topline(&raw mut *win);
        } else {
            invalidate_botline_win(&raw mut *win);
        }
    }
}

unsafe fn fix_pos_col(
    buf: *mut buf_T,
    pos: &mut pos_T,
    start_row: linenr_T,
    start_col: colnr_T,
    end_row: linenr_T,
    end_col: colnr_T,
    new_rows: linenr_T,
    new_cols_at_end_row: colnr_T,
    mode_col_adj: colnr_T,
) {
    unsafe {
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
}

unsafe fn fix_cursor_cols(
    win: &mut win_T,
    start_row: linenr_T,
    start_col: colnr_T,
    end_row: linenr_T,
    end_col: colnr_T,
    new_rows: linenr_T,
    new_cols_at_end_row: colnr_T,
) {
    unsafe {
        let mode_col_adj = if ptr::eq(win, *curwin.as_ptr()) && (g_st().State & MODE_INSERT) != 0 {
            0
        } else {
            1
        };
        fix_pos_col(
            win.w_buffer,
            &mut win.w_cursor,
            start_row,
            start_col,
            end_row,
            end_col,
            new_rows,
            new_cols_at_end_row,
            mode_col_adj,
        );

        check_cursor_col(&raw mut *win);
        changed_cline_bef_curs(win);
        invalidate_botline_win(&raw mut *win);
    }
}

#[inline]
unsafe fn init_line_array(lstate: *mut lua_State, a: *mut Array, size: usize, arena: *mut Arena) {
    unsafe {
        if !lstate.is_null() {
            lua_createtable(lstate, size as c_int, 0);
        } else {
            *a = arena_array(arena, size);
        }
    }
}

unsafe fn push_linestr(
    lstate: *mut lua_State,
    a: *mut Array,
    s: *const c_char,
    len: usize,
    idx: c_int,
    replace_nl: bool,
    arena: *mut Arena,
) {
    unsafe {
        if !lstate.is_null() {
            if !s.is_null() && replace_nl && CStr::from_ptr(s).to_bytes().contains(&b'\n') {
                let mut tmp: Vec<c_char> = std::slice::from_raw_parts(s, len + 1).to_vec();
                { let _p = tmp.as_mut_ptr(); let _len = std::ffi::CStr::from_ptr(_p).to_bytes().len(); for _b in std::slice::from_raw_parts_mut(_p.cast::<u8>(), _len) { if *_b == (NL) as u8 { *_b = (NUL) as u8; } } };
                lua_pushlstring(lstate, tmp.as_ptr(), len);
            } else {
                lua_pushlstring(lstate, s, len);
            }
            lua_rawseti(lstate, -2, idx + 1);
        } else {
            let mut str_val = NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            if len > 0 {
                let data = arena_memdupz(arena, s, len);
                if replace_nl {
                    { let _p = data; let _len = std::ffi::CStr::from_ptr(_p).to_bytes().len(); for _b in std::slice::from_raw_parts_mut(_p.cast::<u8>(), _len) { if *_b == (NL) as u8 { *_b = (NUL) as u8; } } };
                }
                str_val = NvimString { data, size: len };
            }
            add_c(&mut *a, string_obj(str_val));
        }
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
// DLLEXPORT void buf_collect_lines(buf_T *buf, size_t n, linenr_T start, int start_idx,
//                                  bool replace_nl, Array *l, lua_State *lstate, Arena *arena);
pub unsafe fn buf_collect_lines(
    buf: *mut buf_T,
    n: usize,
    start: linenr_T,
    start_idx: c_int,
    replace_nl: bool,
    l: *mut Array,
    lstate: *mut lua_State,
    arena: *mut Arena,
) {
    unsafe {
        for i in 0..n {
            let lnum = start + (i as linenr_T);
            let bufstr = ml_get_buf(buf, lnum);
            let bufstrlen = ml_get_buf_len(buf, lnum) as usize;
            push_linestr(
                lstate,
                l,
                bufstr,
                bufstrlen,
                start_idx + (i as c_int),
                replace_nl,
                arena,
            );
        }
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
        assert_eq!(obj.obj_type, ObjectType::kObjectTypeBoolean);
        assert!(obj.data.boolean());
    }
}
