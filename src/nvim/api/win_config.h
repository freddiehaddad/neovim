#pragma once

#include <stdint.h>  // IWYU pragma: keep

#include "nvim/api/keysets_defs.h"  // IWYU pragma: keep
#include "nvim/api/private/defs.h"  // IWYU pragma: keep
#include "nvim/buffer_defs.h"  // IWYU pragma: keep

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

/// Opens a new split window, floating window, or external window.
///
/// - Specify `relative` to create a floating window. Floats are drawn over the split layout,
///   relative to a position in some other window. See |api-floatwin|.
///   - Floats must specify `width` and `height`.
/// - Specify `external` to create an external window. External windows are displayed as separate
///   top-level windows managed by the |ui-multigrid| UI (not Nvim).
/// - If `relative` and `external` are omitted, a normal "split" window is created.
///   - The `win` key decides which window to split. If nil or 0, the split will be adjacent to
///     the current window. If -1, a top-level split will be created.
///   - Use `vertical` and `split` to control split direction. For `vertical`, the exact direction
///     is determined by 'splitright' and 'splitbelow'.
///   - Split windows cannot have `bufpos`, `row`, `col`, `border`, `title`, `footer`.
///
/// With relative=editor (row=0,col=0) refers to the top-left corner of the
/// screen-grid and (row=Lines-1,col=Columns-1) refers to the bottom-right
/// corner. Fractional values are allowed, but the builtin implementation
/// (used by non-multigrid UIs) will always round down to nearest integer.
///
/// Out-of-bounds values, and configurations that make the float not fit inside
/// the main editor, are allowed. The builtin implementation truncates values
/// so floats are fully within the main screen grid. External GUIs
/// could let floats hover outside of the main window like a tooltip, but
/// this should not be used to specify arbitrary WM screen positions.
///
/// Examples:
///
/// ```lua
/// -- Window-relative float with 'statusline' enabled:
/// local w1 = vim.api.nvim_open_win(0, false,
///   {relative='win', row=3, col=3, width=40, height=4})
/// vim.wo[w1].statusline = vim.o.statusline
///
/// -- Buffer-relative float (travels as buffer is scrolled):
/// vim.api.nvim_open_win(0, false,
///   {relative='win', width=40, height=4, bufpos={100,10}})
///
/// -- Vertical split left of the current window:
/// vim.api.nvim_open_win(0, false, { split = 'left', win = 0, })
/// ```
///
/// @param buf Buffer to display, or 0 for current buffer
/// @param enter  Enter the window (make it the current window)
/// @param config Map defining the window configuration. Keys:
///   - anchor: Decides which corner of the float to place at (row,col):
///      - "NW" northwest (default)
///      - "NE" northeast
///      - "SW" southwest
///      - "SE" southeast
///   - border: (`string|string[]`, default: 'winborder' option) Window border. The string form
///     accepts the same values as the 'winborder' option. The array form must have a length of
///     eight or any divisor of eight, specifying the chars that form the border in a clockwise
///     fashion starting from the top-left corner. For example, the double-box style can be
///     specified as:
///     ```
///     [ "╔", "═" ,"╗", "║", "╝", "═", "╚", "║" ].
///     ```
///     If fewer than eight chars are given, they will be repeated. An ASCII border could be
///     specified as:
///     ```
///     [ "/", "-", \"\\\\\", "|" ],
///     ```
///     Or one char for all sides:
///     ```
///     [ "x" ].
///     ```
///     Empty string can be used to hide a specific border. This example will show only vertical
///     borders, not horizontal:
///     ```
///     [ "", "", "", ">", "", "", "", "<" ]
///     ```
///     By default, |hl-FloatBorder| highlight is used, which links to |hl-WinSeparator| when not
///     defined. Each border side can specify an optional highlight:
///     ```
///     [ ["+", "MyCorner"], ["x", "MyBorder"] ].
///     ```
///   - bufpos: Places float relative to buffer text (only when
///       relative="win"). Takes a tuple of zero-indexed `[line, column]`.
///       `row` and `col` if given are applied relative to this
///       position, else they default to:
///       - `row=1` and `col=0` if `anchor` is "NW" or "NE"
///       - `row=0` and `col=0` if `anchor` is "SW" or "SE"
///         (thus like a tooltip near the buffer text).
///   - col: Column position in units of screen cell width, may be fractional.
///   - external: GUI should display the window as an external
///       top-level window. Currently accepts no other positioning
///       configuration together with this.
///   - fixed: If true when anchor is NW or SW, the float window
///            would be kept fixed even if the window would be truncated.
///   - focusable: (default: true) Enable focus by user actions (wincmds, mouse events).
///     Non-|focusable| windows can be entered by |nvim_set_current_win()|, or by mouse events if
///     `mouse=true`.
///   - footer: (`string|string[]?`) Footer in window border: string or list of `[text, highlight]`
///     tuples. Default highlight group is `FloatFooter`.
///   - footer_pos: (default: "left") Footer position. One of: "left", "center", "right". The
///    `footer` field must be set also.
///   - height: Window height (in character cells). Minimum of 1.
///   - hide: Hides the floating window. |window-hidden|
///   - mouse: (default: `focusable` value) If true, mouse events interact with the window
///     normally; if false, they pass through to the window behind it.
///   - noautocmd: Block all autocommands for the duration of the call. Cannot be changed by
///     |nvim_win_set_config()|.
///   - relative: Sets the window layout to "floating", placed at (row,col) coordinates relative to:
///      - "cursor"     Cursor position in current window.
///      - "editor"     The global editor grid.
///      - "laststatus" 'laststatus' if present, or last row.
///      - "mouse"      Mouse position.
///      - "tabline"    Tabline if present, or first row.
///      - "win"        Window given by the `win` field, or current window.
///   - row: Row position in units of "screen cell height", may be fractional.
///   - split: Split direction: "left", "right", "above", "below".
///   - style: (optional) Configure the appearance of the window:
///       - ""         No special style.
///       - "minimal"  Nvim will display the window with many UI options
///                    disabled. This is useful when displaying a temporary
///                    float where the text should not be edited. Disables
///                    'number', 'relativenumber', 'cursorline', 'cursorcolumn',
///                    'foldcolumn', 'spell' and 'list' options. 'signcolumn'
///                    is changed to `auto` and 'colorcolumn' is cleared.
///                    'statuscolumn' is changed to empty. The end-of-buffer
///                     region is hidden by setting `eob` flag of
///                    'fillchars' to a space char, and clearing the
///                    |hl-EndOfBuffer| region in 'winhighlight'.
///   - title: (optional) Title in window border, string or list.
///       List should consist of `[text, highlight]` tuples.
///       If string, or a tuple lacks a highlight, the default highlight group is `FloatTitle`.
///   - title_pos: (default: "left") Title position. One of: "left", "center", "right". The `title`
///     field must be set also.
///   - vertical: Split vertically |:vertical|.
///   - width: Window width (in character cells). Minimum of 1.
///   - win: |window-ID| target window. Can be in a different tab page. Determines the window to
///       split (negative values act like |:topleft|, |:botright|), the relative window for a
///       `relative="win"` float, or just the target tab page (inferred from the window) for others.
///   - zindex: (positive integer, default: 50) Stacking order. Floats with higher `zindex` overlay
///     floats with lower indices. Below 100 is recommended, unless there is a good reason to
///     overshadow builtin elements. The cursor is dimmed if an unfocused float above the cursor
///     exceeds the zindex of the current window by 50. These screen elements have hard-coded
///     z-indices:
///       - 100: |ins-completion-menu| popupmenu
///       - 200: message scrollback (|pager|)
///       - 250: |cmdline-completion| popupmenu (wildoptions=pum)
///   - _cmdline_offset: (EXPERIMENTAL) When provided, anchor the |cmdline-completion|
///     popupmenu to this window, with an offset in screen cell width.
///
/// @param[out] err Error details, if any
///
/// @return |window-ID|, or 0 on error
DLLEXPORT Window nvim_open_win(Buffer buf, Boolean enter, Dict(win_config) *config, Error *err)
  FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;

/// Reconfigures the layout and properties of a window.
///
/// - Updates only the given keys; unspecified (`nil`) keys will not be changed.
/// - Can move a window to another tabpage.
/// - Can transform a window to/from a float.
/// - Keys `row` / `col` / `relative` must be specified together.
/// - Cannot move the last window in a tabpage to a different one.
///
/// Example: to convert a floating window to a "normal" split window, specify the `win` field:
///
/// ```lua
/// vim.api.nvim_win_set_config(0, { split = 'above', win = vim.fn.win_getid(1), })
/// ```
///
/// @see |nvim_open_win()|
///
/// @param      win  |window-ID|, or 0 for current window
/// @param      config  Map defining the window configuration, see [nvim_open_win()]
/// @param[out] err     Error details, if any
DLLEXPORT void nvim_win_set_config(Window win, Dict(win_config) *config, Error *err)
  FUNC_API_SINCE(6);

/// Gets window config as a dict which can be passed to |nvim_open_win()| as the `config` parameter.
///
/// For non-floating windows, `relative` is empty, thus you can check that field to detect if
/// a window is a floatwin:
/// ```lua
/// vim.print(vim.api.nvim_win_get_config(0).relative == '' and 'non-float' or 'float')
/// -- Or use win_gettype().
/// vim.print(vim.fn.win_gettype())
/// ```
///
/// @param      win |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return     Map defining the window configuration, see |nvim_open_win()|
DLLEXPORT Dict(win_config) nvim_win_get_config(Window win, Arena *arena, Error *err)
  FUNC_API_SINCE(6);

DLLEXPORT void parse_bordertext(Object bordertext, BorderTextType bordertext_type,
                                WinConfig *fconfig, Error *err);
DLLEXPORT void parse_border_style(Object style, WinConfig *fconfig, Error *err);

/// Parses a border style name or custom (comma-separated) style.
DLLEXPORT bool parse_winborder(WinConfig *fconfig, char *border_opt, Error *err);

#include "nvim/func_attr.h"
