#pragma once

#include <lua.h>  // IWYU pragma: keep
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/pos_defs.h"  // IWYU pragma: keep
#include "nvim/types_defs.h"  // IWYU pragma: keep

#define DEFINE_FUNC_ATTRIBUTES
#include "nvim/func_attr.h"
#undef DEFINE_FUNC_ATTRIBUTES
#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

/// @brief <pre>help
/// For more information on buffers, see |buffers|.
///
/// Unloaded Buffers: ~
///
/// Buffers may be unloaded by the |:bunload| command or the buffer's
/// 'bufhidden' option. When a buffer is unloaded its file contents are freed
/// from memory and vim cannot operate on the buffer lines until it is reloaded
/// (usually by opening the buffer again in a new window). API methods such as
/// |nvim_buf_get_lines()| and |nvim_buf_line_count()| will be affected.
///
/// You can use |nvim_buf_is_loaded()| or |nvim_buf_line_count()| to check
/// whether a buffer is loaded.
/// </pre>

/// Ensures that a buffer is loaded.
DLLEXPORT buf_T *api_buf_ensure_loaded(Buffer buf, Error *err);

/// Returns the number of lines in the given buffer.
///
/// @param buf   Buffer id, or 0 for current buffer
/// @param[out] err Error details, if any
/// @return Line count, or 0 for unloaded buffer. |api-buffer|
DLLEXPORT Integer nvim_buf_line_count(Buffer buf, Error *err)
  FUNC_API_SINCE(1);

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
DLLEXPORT Boolean nvim_buf_attach(uint64_t channel_id, Buffer buf, Boolean send_buffer,
                                  Dict(buf_attach) *opts, Error *err)
  FUNC_API_SINCE(4);

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
DLLEXPORT Boolean nvim_buf_detach(uint64_t channel_id, Buffer buf, Error *err)
  FUNC_API_SINCE(4) FUNC_API_REMOTE_ONLY;

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
DLLEXPORT ArrayOf(String) nvim_buf_get_lines(uint64_t channel_id, Buffer buf, Integer start,
                                             Integer end, Boolean strict_indexing, Arena *arena,
                                             lua_State *lstate, Error *err)
  FUNC_API_SINCE(1);

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
DLLEXPORT void nvim_buf_set_lines(uint64_t channel_id, Buffer buf, Integer start, Integer end,
                                  Boolean strict_indexing, ArrayOf(String) replacement,
                                  Arena *arena, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_TEXTLOCK;

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
DLLEXPORT void nvim_buf_set_text(uint64_t channel_id, Buffer buf, Integer start_row,
                                 Integer start_col, Integer end_row, Integer end_col,
                                 ArrayOf(String) replacement, Arena *arena, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_TEXTLOCK;

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
DLLEXPORT ArrayOf(String) nvim_buf_get_text(uint64_t channel_id, Buffer buf, Integer start_row,
                                            Integer start_col, Integer end_row, Integer end_col,
                                            Dict(empty) *opts, Arena *arena, lua_State *lstate,
                                            Error *err)
  FUNC_API_SINCE(9);

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
DLLEXPORT Integer nvim_buf_get_offset(Buffer buf, Integer index, Error *err)
  FUNC_API_SINCE(5);

/// Gets a buffer-scoped (b:) variable.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
/// @return Variable value
DLLEXPORT Object nvim_buf_get_var(Buffer buf, String name, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

/// Gets a changed tick of a buffer
///
/// @param[in]  buf  Buffer id, or 0 for current buffer
/// @param[out] err     Error details, if any
///
/// @return `b:changedtick` value.
DLLEXPORT Integer nvim_buf_get_changedtick(Buffer buf, Error *err)
  FUNC_API_SINCE(2);

/// Gets a list of buffer-local |mapping| definitions.
///
/// @param  buf     Buffer id, or 0 for current buffer
/// @param  mode       Mode short-name ("n", "i", "v", ...)
/// @param[out]  err   Error details, if any
/// @returns Array of |maparg()|-like dictionaries describing mappings.
///          The "buf" key holds the associated buffer id.
DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_buf_get_keymap(Buffer buf, String mode, Arena *arena,
                                                          Error *err)
  FUNC_API_SINCE(3);

/// Sets a buffer-local |mapping| for the given mode.
///
/// @see |nvim_set_keymap()|
///
/// @param  buf  Buffer id, or 0 for current buffer
DLLEXPORT void nvim_buf_set_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
                                   String rhs, Dict(keymap) *opts, Error *err)
  FUNC_API_SINCE(6);

/// Unmaps a buffer-local |mapping| for the given mode.
///
/// @see |nvim_del_keymap()|
///
/// @param  buf   Buffer id, or 0 for current buffer
/// @param  mode  Mode short-name ("n", "i", "v", ...)
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  opts  Optional parameters.
///               - lhs: When true, only match {lhs}, not {rhs}.
DLLEXPORT void nvim_buf_del_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
                                   Dict(keymap_del) *opts, Error *err)
  FUNC_API_SINCE(6);

/// Sets a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param value      Variable value
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_set_var(Buffer buf, String name, Object value, Error *err)
  FUNC_API_SINCE(1);

/// Removes a buffer-scoped (b:) variable
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_del_var(Buffer buf, String name, Error *err)
  FUNC_API_SINCE(1);

/// Gets the full/absolute filepath of the buffer, or the buffer name for non-file buffers.
///
/// If the buffer represents a directory, the name ends with a path separator,
/// unless it was changed by |:file| or |nvim_buf_set_name()|.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param[out] err   Error details, if any
/// @return Buffer name
DLLEXPORT String nvim_buf_get_name(Buffer buf, Error *err)
  FUNC_API_SINCE(1);

/// Sets the full file name for a buffer, like |:file_f|
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Buffer name
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_set_name(Buffer buf, String name, Error *err)
  FUNC_API_SINCE(1);

/// Checks if a buffer is valid and loaded. See |api-buffer| for more info
/// about unloaded buffers.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid and loaded, false otherwise.
DLLEXPORT Boolean nvim_buf_is_loaded(Buffer buf)
  FUNC_API_SINCE(5);

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
DLLEXPORT void nvim_buf_delete(Buffer buf, Dict(buf_delete) *opts, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_TEXTLOCK;

/// Checks if a buffer is valid.
///
/// @note Even if a buffer is valid it may have been unloaded. See |api-buffer|
/// for more info about unloaded buffers.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid, false otherwise.
DLLEXPORT Boolean nvim_buf_is_valid(Buffer buf)
  FUNC_API_SINCE(1);

/// Deletes a named mark in the buffer. See |mark-motions|.
///
/// @note only deletes marks set in the buffer, if the mark is not set
/// in the buffer it will return false.
/// @param buf     Buffer to set the mark on
/// @param name       Mark name
/// @return true if the mark was deleted, else false.
/// @see |nvim_buf_set_mark()|
/// @see |nvim_del_mark()|
DLLEXPORT Boolean nvim_buf_del_mark(Buffer buf, String name, Error *err)
  FUNC_API_SINCE(8);

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
DLLEXPORT Boolean nvim_buf_set_mark(Buffer buf, String name, Integer line, Integer col,
                                    Dict(empty) *opts, Error *err)
  FUNC_API_SINCE(8);

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
DLLEXPORT ArrayOf(Integer, 2) nvim_buf_get_mark(Buffer buf, String name, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

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
DLLEXPORT Object nvim_buf_call(Buffer buf, LuaRef fn, lua_State *lstate, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_LUA_ONLY;

/// @nodoc
DLLEXPORT Dict nvim__buf_stats(Buffer buf, Arena *arena, Error *err);

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
DLLEXPORT void buf_collect_lines(buf_T *buf, size_t n, linenr_T start, int start_idx,
                                 bool replace_nl, Array *l, lua_State *lstate, Arena *arena);

#include "nvim/func_attr.h"
