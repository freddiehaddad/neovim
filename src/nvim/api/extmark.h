#pragma once

#include <stdbool.h>
#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"
#include "nvim/decoration_defs.h"  // IWYU pragma: keep
#include "nvim/macros_defs.h"
#include "nvim/map_defs.h"
#include "nvim/types_defs.h"

EXTERN Map(String, int) namespace_ids INIT( = MAP_INIT);
EXTERN Set(uint32_t) namespace_localscope INIT( = SET_INIT);
EXTERN handle_T next_namespace_id INIT( = 1);

static inline bool ns_in_win(uint32_t ns_id, win_T *wp)
{
  if (!set_has(uint32_t, &namespace_localscope, ns_id)) {
    return true;
  }

  return set_has(uint32_t, &wp->w_ns_set, ns_id);
}

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

DLLEXPORT void api_extmark_free_all_mem(void);

/// Creates a new namespace or gets an existing one. [namespace]()
///
/// Namespaces are used for buffer highlights and virtual text, see
/// |nvim_buf_set_extmark()|.
///
/// Namespaces can be named or anonymous. If `name` matches an existing
/// namespace, the associated id is returned. If `name` is an empty string
/// a new, anonymous namespace is created.
///
/// @param name Namespace name or empty string
/// @return Namespace id
DLLEXPORT Integer nvim_create_namespace(String name)
  FUNC_API_SINCE(5);

/// Gets existing, non-anonymous |namespace|s.
///
/// @return dict that maps from names to namespace ids.
DLLEXPORT DictOf(Integer) nvim_get_namespaces(Arena *arena)
  FUNC_API_SINCE(5);

DLLEXPORT const char *describe_ns(NS ns_id, const char *unknown);
DLLEXPORT bool ns_initialized(uint32_t ns);
DLLEXPORT Array virt_text_to_array(VirtText vt, bool hl_name, Arena *arena);

/// Gets the position (0-indexed) of an |extmark|.
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param id  Extmark id
/// @param opts  Optional parameters. Keys:
///          - details: Whether to include the details dict
///          - hl_name: Whether to include highlight group name instead of id, true if omitted
/// @param[out] err   Error details, if any
/// @return 0-indexed (row, col, details?) tuple or empty list () if extmark id was absent.  The
/// optional `details` dictionary contains the same keys as `opts` in |nvim_buf_set_extmark()|,
/// except for `id`, `conceal_lines` and `ephemeral`. It also contains the following keys:
///
/// - ns_id: |namespace| id
/// - invalid: boolean that indicates whether the mark is hidden because the entirety of
/// text span range is deleted. See also the key `invalidate` in |nvim_buf_set_extmark()|.
DLLEXPORT Tuple(Integer, Integer, *DictAs(extmark_details))
nvim_buf_get_extmark_by_id(Buffer buf, Integer ns_id, Integer id, Dict(get_extmark) *opts,
                           Arena *arena, Error *err)
  FUNC_API_SINCE(7);

/// Gets |extmarks| in "traversal order" from a |charwise| region defined by buffer positions
/// (inclusive, 0-indexed |api-indexing|).
///
/// Region can be given as (row,col) tuples, or valid extmark ids (whose
/// positions define the bounds). 0 and -1 are understood as (0,0) and (-1,-1)
/// respectively, thus the following are equivalent:
///
/// ```lua
/// vim.api.nvim_buf_get_extmarks(0, my_ns, 0, -1)
/// vim.api.nvim_buf_get_extmarks(0, my_ns, {0,0}, {-1,-1})
/// ```
///
/// If `end` is less than `start`, marks are returned in reverse order.
/// (Useful with `limit`, to get the first marks prior to a given position.)
///
/// Note:
/// - For a reverse range, `limit` does not actually affect the traversed range, just how many marks
///   are returned
/// - When using extmark ranges (marks with a end_row/end_col position) the `overlap` option might
///   be useful. Otherwise only the start position of an extmark will be considered.
/// - The |:marks| command can list extmarks.
/// - Legacy signs placed through the |:sign| commands are implemented as extmarks. Their details
///   array will contain a `sign_name` field.
///
/// Example:
///
/// ```lua
/// local api = vim.api
/// local pos = api.nvim_win_get_cursor(0)
/// local ns  = api.nvim_create_namespace('my-plugin')
/// -- Create new extmark at line 1, column 1.
/// local m1  = api.nvim_buf_set_extmark(0, ns, 0, 0)
/// -- Create new extmark at line 3, column 1.
/// local m2  = api.nvim_buf_set_extmark(0, ns, 2, 0)
/// -- Get extmarks only from line 3.
/// local ms  = api.nvim_buf_get_extmarks(0, ns, {2,0}, {2,0})
/// -- Get all marks in this buffer + namespace.
/// local all = api.nvim_buf_get_extmarks(0, ns, 0, -1)
/// vim.print(ms)
/// ```
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id, or -1 for all namespaces
/// @param start  Start of range: a 0-indexed (row, col) or valid extmark id
/// (whose position defines the bound). |api-indexing|
/// @param end  End of range (inclusive): a 0-indexed (row, col) or valid
/// extmark id (whose position defines the bound). |api-indexing|
/// @param opts  Optional parameters. Keys:
///          - details: Whether to include the details dict
///          - hl_name: Whether to include highlight group name instead of id, true if omitted
///          - limit:  Maximum number of marks to return
///          - overlap: Also include marks which overlap the range, even if
///                     their start position is less than `start`
///          - type: Filter marks by type: "highlight", "sign", "virt_text" and "virt_lines"
/// @param[out] err   Error details, if any
/// @return List of `[extmark_id, row, col, details?]` tuples in "traversal order". For the
/// `details` dictionary, see |nvim_buf_get_extmark_by_id()|.
DLLEXPORT ArrayOf(DictAs(get_extmark_item)) nvim_buf_get_extmarks(Buffer buf, Integer ns_id,
                                                                  Object start, Object end,
                                                                  Dict(get_extmarks) *opts,
                                                                  Arena *arena, Error *err)
  FUNC_API_SINCE(7);

/// Creates or updates an |extmark|.
///
/// By default a new extmark is created when no id is passed in, but it is also
/// possible to create a new mark by passing in a previously unused id or move
/// an existing mark by passing in its id. The caller must then keep track of
/// existing and unused ids itself. (Useful over RPC, to avoid waiting for the
/// return value.)
///
/// Using the optional arguments, it is possible to use this to highlight
/// a range of text, and also to associate virtual text to the mark.
///
/// If present, the position defined by `end_col` and `end_row` should be after
/// the start position in order for the extmark to cover a range.
/// An earlier end position is not an error, but then it behaves like an empty
/// range (no highlighting).
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param line  Line where to place the mark, 0-based. |api-indexing|
/// @param col  Column where to place the mark, 0-based. |api-indexing|
/// @param opts  Optional parameters:
///               - conceal: (boolean|string)
///                   - string: Single char or empty string, enables conceal similar to |:syn-conceal|.
///                     Non-empty char is used as |:syn-cchar|. Highlighted with "hl_group" if
///                     defined, else defaults to |hl-Conceal|.
///                   - boolean: true is equivalent to "", false removes any existing conceal.
///               - conceal_lines: (string) Line-level conceal. When set to an empty string (other
///                 values reserved for future use), the lines in the extmark range are not drawn;
///                 the next non-concealed line is drawn in their place. Requires 'conceallevel' >=
///                 2; subject to 'concealcursor' on the cursor line.
///               - cursorline_hl_group: Highlight group used for the 'signcolumn' text when the
///                 cursor is on the same line as the mark and 'cursorline' is enabled.
///               - end_col: End-column of the mark, 0-based exclusive, or -1 to extend the range to
///                 end of line (if strict=false).
///               - end_right_gravity: (boolean, default: false) Indicates which direction the extmark
///                 end-position (if any) will be shifted when new text is inserted (true=right, false=left).
///               - end_row: End-line of the mark, 0-based inclusive.
///               - ephemeral: For use with |nvim_set_decoration_provider()| callbacks. The mark
///                 will only be used for the current redraw cycle, and not stored in the buffer.
///               - hl_eol: When true, for a multiline highlight covering the EOL of a line,
///                 continue the highlight for the rest of the screen line (just like for diff and
///                 cursorline highlight).
///               - hl_group: Highlight group for the text range. This and below highlight groups
///                 can be given as a string or as an integer (|nvim_get_hl_id_by_name()|). Multiple
///                 highlight groups can be stacked by passing an array (highest-priority last).
///               - hl_mode: Controls how highlights are combined with the highlights of the text.
///                 Only affects virt_text highlights, but might affect `hl_group` in the future.
///                 - "replace": Only show the virt_text color. This is the default.
///                 - "combine": Combine with background text color.
///                 - "blend": Blend with background text color. Not supported for "inline" virt_text.
///               - id: Update this extmark instead of creating a new one.
///               - invalidate: (boolean) Indicates whether to hide the extmark if the entirety of
///                 its range is deleted. For hidden marks, an "invalid" key is added to the
///                 "details" array of |nvim_buf_get_extmarks()| and family. If "undo_restore" is
///                 false, the extmark is deleted instead.
///               - line_hl_group: Highlight group used for the whole line.
///               - number_hl_group: Highlight group used for the number column.
///               - priority: Priority of the highlight group, sign attribute or virtual text. For
///                 virtual text, the highest-priority item is drawn last. For example treesitter
///                 highlighting uses a value of 100.
///               - right_gravity: (boolean, default: true) Indicates which direction the extmark
///                 will be shifted when new text is inserted (true=right, false=left).
///               - sign_hl_group: Highlight group for `sign_text`.
///               - sign_text: (string, length 1-2) Displayed in 'signcolumn'.
///               - spell: (boolean) Indicates that spell checking should be performed within this
///                 extmark.
///               - strict: (boolean, default: true) Indicates extmark should not be placed if the
///                 line or column value is past end-of-buffer or end-of-line respectively.
///               - ui_watched: (boolean) Indicates the mark should be drawn by a UI; activates
///                 `win_extmark` [ui-multigrid] events. Note: the mark is positioned by virt_text
///                 attributes. Can be used with virt_text.
///               - undo_restore: (default: true) Restore the exact position of the mark if text
///                 around the mark was deleted and then restored by undo.
///               - url: URL stored on this extmark. In the TUI, the OSC 8 control
///                   sequence is used to generate a clickable hyperlink to this URL.
///               - virt_lines: List of virtual lines to place next to the mark, where each line is
///                 a list of `[text_chunk, hl]` tuples. Buffer and window options do not affect the
///                 display of the text, except 'tabstop' is used for hard tabs. The 'wrap' and
///                 'linebreak' options do not take effect, so the number of extra screen lines will
///                 match the list-size (but see `virt_lines_overflow`). By default lines are placed
///                 below the marked line. Special case: If the last `text_chunk` is empty,
///                 the corresponding `hl` will extend to the end of the line.
///               - virt_lines_above: Place virtual lines above instead.
///               - virt_lines_leftcol: Place virtual lines in the leftmost column of the window,
///                 bypassing sign and number columns.
///               - virt_lines_overflow: Controls display of virtual lines exceeding the viewport:
///                 - "auto": Decided by the 'wrap' option.
///                 - "scroll": Scroll horizontally with 'nowrap', otherwise the same as "trunc".
///                 - "trunc": Truncate on the right (default).
///                 - "wrap": Always wrap (regardless of the 'wrap' option).
///               - virt_text: [](virtual-text) stored and displayed by this mark. List of `[text, hl]`
///                 tuples, each representing a text chunk with a highlight `hl` which may be
///                 a single highlight group or a list of stacked highlight groups (highest-priority
///                 last).
///               - virt_text_hide: Hide the virtual text when the background text is selected or
///                 hidden because of scrolling with 'nowrap' or 'smoothscroll'. Currently only
///                 affects "overlay" virt_text.
///               - virt_text_pos: Position of virtual text:
///                 - "eol": One cell after the line text or its displayed 'listchars' EOL character
///                   (default).
///                 - "eol_right_align": Display right-aligned in the window unless the virtual text
///                   is longer than the space available. If the virtual text is too long, it is
///                   truncated to fit in the window after the EOL character. If the line is
///                   wrapped, the virtual text is shown after the end of the line rather than the
///                   previous screen line.
///                 - "overlay": Display over the specified column, without shifting the underlying text.
///                 - "right_align": Display right-aligned in the window.
///                 - "inline": Display at the specified column, and right-shift the buffer text as needed.
///               - virt_text_repeat_linebreak: Repeat the virtual text on wrapped lines.
///               - virt_text_win_col: Position the virtual text at a fixed window column (starting
///                 from the first text column of the screen line) instead of "virt_text_pos".
///
/// @param[out]  err   Error details, if any
/// @return Id of the created/updated extmark
DLLEXPORT Integer nvim_buf_set_extmark(Buffer buf, Integer ns_id, Integer line, Integer col,
                                       Dict(set_extmark) *opts, Error *err)
  FUNC_API_SINCE(7);

/// Removes an |extmark|.
///
/// @param buf Buffer id, or 0 for current buffer
/// @param ns_id |namespace| id
/// @param id Extmark id
/// @param[out] err   Error details, if any
/// @return true if the extmark was found, else false
DLLEXPORT Boolean nvim_buf_del_extmark(Buffer buf, Integer ns_id, Integer id, Error *err)
  FUNC_API_SINCE(7);

/// Clears |namespace|d objects (highlights, |extmarks|, virtual text) from
/// a region.
///
/// Lines are 0-indexed. |api-indexing|  To clear the namespace in the entire
/// buffer, specify line_start=0 and line_end=-1.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param ns_id      Namespace to clear, or -1 to clear all namespaces.
/// @param line_start Start of range of lines to clear
/// @param line_end   End of range of lines to clear (exclusive) or -1 to clear
///                   to end of buffer.
/// @param[out] err   Error details, if any
DLLEXPORT void nvim_buf_clear_namespace(Buffer buf, Integer ns_id, Integer line_start,
                                        Integer line_end, Error *err)
  FUNC_API_SINCE(5);

/// Set or change decoration provider for a |namespace|
///
/// This is a very general purpose interface for having Lua callbacks
/// being triggered during the redraw code.
///
/// The expected usage is to set |extmarks| for the currently
/// redrawn buffer. |nvim_buf_set_extmark()| can be called to add marks
/// on a per-window or per-lines basis. Use the `ephemeral` key to only
/// use the mark for the current screen redraw (the callback will be called
/// again for the next redraw).
///
/// Note: this function should not be called often. Rather, the callbacks
/// themselves can be used to throttle unneeded callbacks. the `on_start`
/// callback can return `false` to disable the provider until the next redraw.
/// Similarly, return `false` in `on_win` will skip the `on_line` and `on_range` calls
/// for that window (but any extmarks set in `on_win` will still be used).
/// A plugin managing multiple sources of decoration should ideally only set
/// one provider, and merge the sources internally. You can use multiple `ns_id`
/// for the extmarks set/modified inside the callback anyway.
///
/// Note: doing anything other than setting extmarks is considered experimental.
/// Doing things like changing options are not explicitly forbidden, but is
/// likely to have unexpected consequences (such as 100% CPU consumption).
/// Doing `vim.rpcnotify` should be OK, but `vim.rpcrequest` is quite dubious
/// for the moment.
///
/// Note: It is not allowed to remove or update extmarks in `on_line` or `on_range` callbacks.
///
/// Note: Callbacks will not run for |window-hidden|, as they are not redrawn.
///
/// @param ns_id  |namespace| id
/// @param opts  Table of callbacks:
///             - on_buf: called for each buffer being redrawn (once per edit,
///               before window callbacks)
///               ```
///                 ["buf", bufnr, tick]
///               ```
///             - on_end: called at the end of a redraw cycle
///               ```
///                 ["end", tick]
///               ```
///             - on_line: (deprecated, use on_range instead)
///               ```
///                 ["line", winid, bufnr, row]
///               ```
///             - on_range: called for each buffer range being redrawn.
///               Range is end-exclusive and may span multiple lines. Range
///               bounds point to the first byte of a character. An end position
///               of the form (lnum, 0), including (number of lines, 0), is valid
///               and indicates that EOL of the preceding line is included.
///               ```
///                 ["range", winid, bufnr, begin_row, begin_col, end_row, end_col]
///               ```
///
///               In addition to returning a boolean, it is also allowed to
///               return a `skip_row, skip_col` pair of integers. This implies
///               that this function does not need to be called until a range
///               which continues beyond the skipped position. A single integer
///               return value `skip_row` is short for `skip_row, 0`
///
///             - on_start: called first on each screen redraw
///               ```
///                 ["start", tick]
///               ```
///             - on_win: called when starting to redraw a specific window.
///               ```
///                 ["win", winid, bufnr, toprow, botrow]
///               ```
DLLEXPORT void nvim_set_decoration_provider(Integer ns_id, Dict(set_decoration_provider) *opts,
                                            Error *err)
  FUNC_API_SINCE(7) FUNC_API_LUA_ONLY;

DLLEXPORT VirtText parse_virt_text(Array chunks, Error *err, int *width, bool untab);

/// @nodoc
DLLEXPORT String nvim__buf_debug_extmarks(Buffer buf, Boolean keys, Boolean dot, Error *err)
  FUNC_API_SINCE(7) FUNC_API_RET_ALLOC;

/// Set some properties for namespace
///
/// @param ns_id Namespace
/// @param opts Optional parameters to set:
///           - wins: a list of windows to be scoped in
///
DLLEXPORT void nvim__ns_set(Integer ns_id, Dict(ns_opts) *opts, Error *err);

/// Get the properties for namespace
///
/// @param ns_id Namespace
/// @return  Map defining the namespace properties, see |nvim__ns_set()|
DLLEXPORT Dict(ns_opts) nvim__ns_get(Integer ns_id, Arena *arena, Error *err);

#include "nvim/func_attr.h"
