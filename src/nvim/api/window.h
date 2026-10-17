#pragma once

#include <lua.h>  // IWYU pragma: keep

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

/// Gets the current buffer in a window
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Buffer id
DLLEXPORT Buffer nvim_win_get_buf(Window win, Error *err)
  FUNC_API_SINCE(1);

/// Sets the current buffer in a window.
///
/// Note: As a side-effect, this executes |BufEnter| and |BufLeave| autocommands.
/// @param win   |window-ID|, or 0 for current window
/// @param buf   Buffer id
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_buf(Window win, Buffer buf, Error *err)
  FUNC_API_SINCE(5)
  FUNC_API_TEXTLOCK;

/// Gets the (1,0)-indexed, buffer-relative cursor position for a given window
/// (different windows showing the same buffer have independent cursor
/// positions). |api-indexing|
///
/// @see |getcurpos()|
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return (row, col) tuple
DLLEXPORT ArrayOf(Integer, 2) nvim_win_get_cursor(Window win, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

/// Sets the (1,0)-indexed cursor position (byte offset) in the window. |api-indexing|
/// This scrolls the window even if it is not the current one.
///
/// @param win   |window-ID|, or 0 for current window
/// @param pos      (row, col) tuple representing the new position
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_cursor(Window win, ArrayOf(Integer, 2) pos, Error *err)
  FUNC_API_SINCE(1);

/// Gets the window height
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Height as a count of rows
DLLEXPORT Integer nvim_win_get_height(Window win, Error *err)
  FUNC_API_SINCE(1);

/// Gets the window width
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Width as a count of columns
DLLEXPORT Integer nvim_win_get_width(Window win, Error *err)
  FUNC_API_SINCE(1);

/// Gets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
DLLEXPORT Object nvim_win_get_var(Window win, String name, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

/// Sets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_var(Window win, String name, Object value, Error *err)
  FUNC_API_SINCE(1);

/// Removes a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_del_var(Window win, String name, Error *err)
  FUNC_API_SINCE(1);

/// Gets the window position in display cells. First position is zero.
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return (row, col) tuple with the window position
DLLEXPORT ArrayOf(Integer, 2) nvim_win_get_position(Window win, Arena *arena, Error *err)
  FUNC_API_SINCE(1);

/// Gets the window tabpage
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Tabpage that contains the window
DLLEXPORT Tabpage nvim_win_get_tabpage(Window win, Error *err)
  FUNC_API_SINCE(1);

/// Gets the window number
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Window number
DLLEXPORT Integer nvim_win_get_number(Window win, Error *err)
  FUNC_API_SINCE(1);

/// Checks if a window is valid
///
/// @param win |window-ID|, or 0 for current window
/// @return true if the window is valid, false otherwise
DLLEXPORT Boolean nvim_win_is_valid(Window win)
  FUNC_API_SINCE(1);

/// Closes the window and hides the buffer it contains (like |:hide| with a |window-ID|; unrelated
/// to the `hide` flag of |nvim_open_win()|, |nvim_win_get_config()|).
///
/// Compare |:close| and |nvim_win_close()|, which close the buffer instead of hiding it.
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_hide(Window win, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_TEXTLOCK;

/// Closes the window (like |:close| with a |window-ID|).
///
/// @param win   |window-ID|, or 0 for current window
/// @param force    Behave like `:close!` The last window of a buffer with
///                 unwritten changes can be closed. The buffer will become
///                 hidden, even if 'hidden' is not set.
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_close(Window win, Boolean force, Error *err)
  FUNC_API_SINCE(6)
  FUNC_API_TEXTLOCK;

/// Calls function `fn` in the context of window `win` and returns its result (may be multiple
/// values).
///
/// @see |win_execute()|
/// @see |nvim_buf_call()|
///
/// @param win  |window-ID|, or 0 for current window.
/// @param fn   Lua function to call inside the window.
/// @param err  Error details, if any.
/// @return     Value(s) returned by `fn()`.
DLLEXPORT Object nvim_win_call(Window win, LuaRef fn, lua_State *lstate, Error *err)
  FUNC_API_SINCE(7)
  FUNC_API_LUA_ONLY;

/// Set highlight namespace for a window. This will use highlights defined with
/// |nvim_set_hl()| for this namespace, but fall back to global highlights (ns=0) when
/// missing.
///
/// This takes precedence over the 'winhighlight' option.
///
/// @param win
/// @param ns_id the namespace to use
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_set_hl_ns(Window win, Integer ns_id, Error *err)
  FUNC_API_SINCE(10);

/// Computes the number of screen lines occupied by a range of text in a given window.
/// Works for off-screen text and takes folds into account.
///
/// Diff filler or virtual lines above a line are counted as a part of that line,
/// unless the line is on "start_row" and "start_vcol" is specified.
///
/// Diff filler or virtual lines below the last buffer line are counted in the result
/// when "end_row" is omitted.
///
/// Line indexing is similar to |nvim_buf_get_text()|.
///
/// @param win  |window-ID|, or 0 for current window.
/// @param opts    Optional parameters:
///                - end_row: Ending line index, 0-based inclusive.
///                           When omitted end at the very bottom.
///                - end_vcol: Ending virtual column index on "end_row",
///                            0-based exclusive, rounded up to full screen lines.
///                            When 0 only include diff filler and virtual lines above
///                            "end_row". When omitted include the whole line.
///                - max_height: Don't add the height of lines below the row
///                              for which this height is reached. Useful to e.g. limit the
///                              height to the window height, avoiding unnecessary work. Or
///                              to find out how many buffer lines beyond "start_row" take
///                              up a certain number of logical lines (returned in
///                              "end_row" and "end_vcol").
///                - start_row: Starting line index, 0-based inclusive.
///                             When omitted start at the very top.
///                - start_vcol: Starting virtual column index on "start_row",
///                              0-based inclusive, rounded down to full screen lines.
///                              When omitted include the whole line.
/// @return  Dict containing text height information, with these keys:
///          - all: The total number of screen lines occupied by the range.
///          - fill: The number of diff filler or virtual lines among them.
///          - end_row: The row on which the returned height is reached (first row of
///            a closed fold).
///          - end_vcol: Ending virtual column in "end_row" where "max_height" or the returned
///            height is reached. 0 if "end_row" is a closed fold.
///
/// @see |virtcol()| for text width.
DLLEXPORT DictAs(win_text_height_ret) nvim_win_text_height(Window win, Dict(win_text_height) *opts,
                                                           Arena *arena, Error *err)
  FUNC_API_SINCE(12);

/// Resizes a window, anchored to a given edge.
///
/// The window first takes space from the non-anchored side, and only then from the anchored side.
/// The "anchor" decides the fixed edge, so a window can grow or shrink in any direction.
///
/// Can set height, width (only for a vertical split), or both. The "anchor" applies to the matching
/// axis (top/bottom for height, left/right for width); the other axis uses its default anchor.
///
/// @param win        |window-ID|, or 0 for current window
/// @param width      New width (columns), or -1 for "no change".
/// @param height     New height (rows), or -1 for "no change".
/// @param opts       Optional parameters.
///                     - anchor: Edge that stays fixed while the opposite edge moves; the
///                       neighbor on the moving side is resized first. One of:
///                       - "top" (default) or "bottom"
///                       - "left" (default) or "right"
/// @param[out] err Error details, if any
DLLEXPORT void nvim_win_resize(Window win, Integer width, Integer height, Dict(win_resize) *opts,
                               Error *err)
  FUNC_API_SINCE(15);

#include "nvim/func_attr.h"
