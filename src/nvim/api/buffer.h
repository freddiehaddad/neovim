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
/// @param channel_id
/// @param buf Buffer id, or 0 for current buffer
/// @param send_buffer True if the initial notification should contain the
///        whole buffer: first notification will be `nvim_buf_lines_event`.
/// @param  opts  Optional parameters.
/// @param[out] err Error details, if any
/// @return False if attach failed (invalid parameter, or buffer isn't loaded);
///         otherwise True.
DLLEXPORT Boolean nvim_buf_attach(uint64_t channel_id, Buffer buf, Boolean send_buffer,
                                  Dict(buf_attach) *opts, Error *err)
  FUNC_API_SINCE(4);

/// Deactivates buffer-update events on the channel.
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
/// @return `b:changedtick` value.
DLLEXPORT Integer nvim_buf_get_changedtick(Buffer buf, Error *err)
  FUNC_API_SINCE(2);

/// Gets a list of buffer-local |mapping| definitions.
///
/// @param  buf     Buffer id, or 0 for current buffer
/// @param  mode       Mode short-name ("n", "i", "v", ...)
/// @param[out]  err   Error details, if any
/// @returns Array of |maparg()|-like dictionaries describing mappings.
DLLEXPORT ArrayOf(DictAs(get_keymap)) nvim_buf_get_keymap(Buffer buf, String mode, Arena *arena,
                                                          Error *err)
  FUNC_API_SINCE(3);

/// Sets a buffer-local |mapping| for the given mode.
///
/// @param  buf  Buffer id, or 0 for current buffer
DLLEXPORT void nvim_buf_set_keymap(uint64_t channel_id, Buffer buf, String mode, String lhs,
                                   String rhs, Dict(keymap) *opts, Error *err)
  FUNC_API_SINCE(6);

/// Unmaps a buffer-local |mapping| for the given mode.
///
/// @param  buf   Buffer id, or 0 for current buffer
/// @param  mode  Mode short-name ("n", "i", "v", ...)
/// @param  lhs   Left-hand-side |{lhs}| of the mapping.
/// @param  opts  Optional parameters.
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

/// Checks if a buffer is valid and loaded.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid and loaded, false otherwise.
DLLEXPORT Boolean nvim_buf_is_loaded(Buffer buf)
  FUNC_API_SINCE(5);

/// Deletes a buffer and its metadata (like |:bwipeout|).
///
/// @param buf Buffer id, or 0 for current buffer
/// @param opts  Optional parameters.
DLLEXPORT void nvim_buf_delete(Buffer buf, Dict(buf_delete) *opts, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_TEXTLOCK;

/// Checks if a buffer is valid.
///
/// @param buf Buffer id, or 0 for current buffer
/// @return true if the buffer is valid, false otherwise.
DLLEXPORT Boolean nvim_buf_is_valid(Buffer buf)
  FUNC_API_SINCE(1);

/// Deletes a named mark in the buffer.
///
/// @param buf     Buffer to set the mark on
/// @param name       Mark name
/// @return true if the mark was deleted, else false.
DLLEXPORT Boolean nvim_buf_del_mark(Buffer buf, String name, Error *err)
  FUNC_API_SINCE(8);

/// Sets a named mark in the given buffer.
///
/// @param buf     Buffer to set the mark on
/// @param name       Mark name
/// @param line       Line number
/// @param col        Column/row number
/// @param opts       Optional parameters. Reserved for future use.
/// @return true if the mark was set, else false.
DLLEXPORT Boolean nvim_buf_set_mark(Buffer buf, String name, Integer line, Integer col,
                                    Dict(empty) *opts, Error *err)
  FUNC_API_SINCE(8);

/// Returns a `(row,col)` tuple representing the position of the named mark.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param name       Mark name
/// @param[out] err   Error details, if any
/// @return (row, col) tuple
DLLEXPORT ArrayOf(Integer, 2) nvim_buf_get_mark(Buffer buf, String name, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

/// Calls function `fn` in the context of buffer `buf` and returns its result.
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

DLLEXPORT void buf_collect_lines(buf_T *buf, size_t n, linenr_T start, int start_idx,
                                 bool replace_nl, Array *l, lua_State *lstate, Arena *arena);

#include "nvim/func_attr.h"
