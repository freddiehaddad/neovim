#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep

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

/// @deprecated Use nvim_exec2() instead.
/// @see nvim_exec2
DLLEXPORT String nvim_exec(uint64_t channel_id, String src, Boolean output, Error *err)
  FUNC_API_SINCE(7) FUNC_API_DEPRECATED_SINCE(11)
  FUNC_API_RET_ALLOC;

/// @deprecated
/// @see nvim_exec2
DLLEXPORT String nvim_command_output(uint64_t channel_id, String command, Error *err)
  FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(7)
  FUNC_API_RET_ALLOC;

/// @deprecated Use nvim_exec_lua() instead.
/// @see nvim_exec_lua
DLLEXPORT Object nvim_execute_lua(String code, Array args, Arena *arena, Error *err)
  FUNC_API_SINCE(3)
  FUNC_API_DEPRECATED_SINCE(7)
  FUNC_API_REMOTE_ONLY;

/// Gets the buffer number
///
/// @deprecated The buffer number now is equal to the object id
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param[out] err   Error details, if any
/// @return Buffer number
DLLEXPORT Integer nvim_buf_get_number(Buffer buffer, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(2);

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
DLLEXPORT void nvim_buf_clear_highlight(Buffer buffer, Integer ns_id, Integer line_start,
                                        Integer line_end, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(7);

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
DLLEXPORT Integer nvim_buf_add_highlight(Buffer buffer, Integer ns_id, String hl_group,
                                         Integer line, Integer col_start, Integer col_end,
                                         Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(13);

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
DLLEXPORT Integer nvim_buf_set_virtual_text(Buffer buffer, Integer src_id, Integer line,
                                            Array chunks, Dict(empty) *opts, Error *err)
  FUNC_API_SINCE(5)
  FUNC_API_DEPRECATED_SINCE(8);

/// Gets a highlight definition by id. |hlID()|
///
/// @deprecated use |nvim_get_hl()| instead
///
/// @param hl_id Highlight id as returned by |hlID()|
/// @param rgb Export RGB colors
/// @param[out] err Error details, if any
/// @return Highlight definition map
/// @see nvim_get_hl_by_name
DLLEXPORT Dict nvim_get_hl_by_id(Integer hl_id, Boolean rgb, Arena *arena, Error *err)
  FUNC_API_SINCE(3)
  FUNC_API_DEPRECATED_SINCE(9);

/// Gets a highlight definition by name.
///
/// @deprecated use |nvim_get_hl()| instead
///
/// @param name Highlight group name
/// @param rgb Export RGB colors
/// @param[out] err Error details, if any
/// @return Highlight definition map
/// @see nvim_get_hl_by_id
DLLEXPORT Dict nvim_get_hl_by_name(String name, Boolean rgb, Arena *arena, Error *err)
  FUNC_API_SINCE(3)
  FUNC_API_DEPRECATED_SINCE(9);

/// Inserts a sequence of lines to a buffer at a certain index
///
/// @deprecated use nvim_buf_set_lines(buffer, lnum, lnum, true, lines)
///
/// @param buffer     Buffer id
/// @param lnum       Insert the lines after `lnum`. If negative, appends to
///                   the end of the buffer.
/// @param lines      Array of lines
/// @param[out] err   Error details, if any
DLLEXPORT void buffer_insert(Buffer buffer, Integer lnum, ArrayOf(String) lines, Arena *arena,
                             Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Gets a buffer line
///
/// @deprecated use nvim_buf_get_lines instead.
///
/// @param buffer   Buffer id
/// @param index    Line index
/// @param[out] err Error details, if any
/// @return Line string
DLLEXPORT String buffer_get_line(Buffer buffer, Integer index, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Sets a buffer line
///
/// @deprecated use nvim_buf_set_lines instead.
///
/// @param buffer   Buffer id
/// @param index    Line index
/// @param line     Contents of the new line
/// @param[out] err Error details, if any
DLLEXPORT void buffer_set_line(Buffer buffer, Integer index, String line, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Deletes a buffer line
///
/// @deprecated use nvim_buf_set_lines instead.
///
/// @param buffer   buffer id
/// @param index    line index
/// @param[out] err Error details, if any
DLLEXPORT void buffer_del_line(Buffer buffer, Integer index, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

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
DLLEXPORT ArrayOf(String) buffer_get_line_slice(Buffer buffer, Integer start, Integer end,
                                                Boolean include_start, Boolean include_end,
                                                Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

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
DLLEXPORT void buffer_set_line_slice(Buffer buffer, Integer start, Integer end,
                                     Boolean include_start, Boolean include_end,
                                     ArrayOf(String) replacement, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Sets a buffer-scoped (b:) variable
///
/// @deprecated
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param value      Variable value
/// @param[out] err   Error details, if any
/// @return Old value or nil if there was no previous value.
DLLEXPORT Object buffer_set_var(Buffer buffer, String name, Object value, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Removes a buffer-scoped (b:) variable
///
/// @deprecated
///
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Variable name
/// @param[out] err   Error details, if any
/// @return Old value
DLLEXPORT Object buffer_del_var(Buffer buffer, String name, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Sets a window-scoped (w:) variable
///
/// @deprecated
///
/// @param window   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
/// @return Old value or nil if there was no previous value.
DLLEXPORT Object window_set_var(Window window, String name, Object value, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Removes a window-scoped (w:) variable
///
/// @deprecated
///
/// @param window   |window-ID|, or 0 for current window
/// @param name     variable name
/// @param[out] err Error details, if any
/// @return Old value
DLLEXPORT Object window_del_var(Window window, String name, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Sets a tab-scoped (t:) variable
///
/// @deprecated
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
/// @return Old value or nil if there was no previous value.
DLLEXPORT Object tabpage_set_var(Tabpage tabpage, String name, Object value, Arena *arena,
                                 Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Removes a tab-scoped (t:) variable
///
/// @deprecated
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Old value
DLLEXPORT Object tabpage_del_var(Tabpage tabpage, String name, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// @deprecated
/// @see nvim_set_var
/// @return Old value or nil if there was no previous value.
DLLEXPORT Object vim_set_var(String name, Object value, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// @deprecated
/// @see nvim_del_var
DLLEXPORT Object vim_del_var(String name, Arena *arena, Error *err)
  FUNC_API_DEPRECATED_SINCE(1);

/// Gets the option information for one option
///
/// @deprecated Use @ref nvim_get_option_info2 instead.
///
/// @param          name Option name
/// @param[out] err Error details, if any
/// @return         Option Information
DLLEXPORT DictAs(get_option_info) nvim_get_option_info(String name, Arena *arena, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_DEPRECATED_SINCE(11);

/// Sets the global value of an option.
///
/// @deprecated
/// @param channel_id
/// @param name     Option name
/// @param value    New option value
/// @param[out] err Error details, if any
DLLEXPORT void nvim_set_option(uint64_t channel_id, String name, Object value, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11);

/// Gets the global value of an option.
///
/// @deprecated
/// @param name     Option name
/// @param[out] err Error details, if any
/// @return         Option value (global)
DLLEXPORT Object nvim_get_option(String name, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11)
  FUNC_API_RET_ALLOC;

/// Gets a buffer option value
///
/// @deprecated
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Option name
/// @param[out] err   Error details, if any
/// @return Option value
DLLEXPORT Object nvim_buf_get_option(Buffer buffer, String name, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11)
  FUNC_API_RET_ALLOC;

/// Sets a buffer option value. Passing `nil` as value deletes the option (only
/// works if there's a global fallback)
///
/// @deprecated
/// @param channel_id
/// @param buffer     Buffer id, or 0 for current buffer
/// @param name       Option name
/// @param value      Option value
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_set_option(uint64_t channel_id, Buffer buffer, String name, Object value,
                                   Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11);

/// Gets a window option value
///
/// @deprecated
/// @param window   |window-ID|, or 0 for current window
/// @param name     Option name
/// @param[out] err Error details, if any
/// @return Option value
DLLEXPORT Object nvim_win_get_option(Window window, String name, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11)
  FUNC_API_RET_ALLOC;

/// Sets a window option value. Passing `nil` as value deletes the option (only
/// works if there's a global fallback)
///
/// @deprecated
/// @param channel_id
/// @param window   |window-ID|, or 0 for current window
/// @param name     Option name
/// @param value    Option value
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_option(uint64_t channel_id, Window window, String name, Object value,
                                   Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(11);

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
DLLEXPORT Array nvim_call_atomic(uint64_t channel_id, Array calls, Arena *arena, Error *err)
  FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(12) FUNC_API_REMOTE_ONLY;

/// @deprecated
///
/// @param channel_id Channel id (passed automatically by the dispatcher)
/// @param event      Event type string
DLLEXPORT void nvim_subscribe(uint64_t channel_id, String event)
// XXX: c_grammar.lua is order-sensitive.
  FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13) FUNC_API_REMOTE_ONLY;

/// @deprecated
///
/// @param channel_id Channel id (passed automatically by the dispatcher)
/// @param event      Event type string
DLLEXPORT void nvim_unsubscribe(uint64_t channel_id, String event)
// XXX: c_grammar.lua is order-sensitive.
  FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13) FUNC_API_REMOTE_ONLY;

/// @deprecated
///
/// @param str Message
DLLEXPORT void nvim_out_write(String str)
  FUNC_API_SINCE(1) FUNC_API_DEPRECATED_SINCE(13);

/// @deprecated
///
/// @param str Message
DLLEXPORT void nvim_err_write(String str)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(13);

/// @deprecated
///
/// @param str Message
DLLEXPORT void nvim_err_writeln(String str)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(13);

/// @deprecated
///
/// Use `nvim_echo` or `nvim_exec_lua("vim.notify(...)", ...)` instead.
///
/// @param msg        Message to display to the user
/// @param log_level  The log level
/// @param opts       Reserved for future use.
/// @param[out] err   Error details, if any
DLLEXPORT Object nvim_notify(String msg, Integer log_level, Dict opts, Arena *arena, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_DEPRECATED_SINCE(13);

/// @deprecated
///
/// Use |nvim_win_resize()| instead, e.g., nvim_win_resize(win, -1, height)
///
/// Sets the window height.
///
/// @param win   |window-ID|, or 0 for current window
/// @param height   Height as a count of rows
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_height(Window win, Integer height, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(15);

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
DLLEXPORT void nvim_win_set_width(Window win, Integer width, Error *err)
  FUNC_API_SINCE(1)
  FUNC_API_DEPRECATED_SINCE(15);

#include "nvim/func_attr.h"
