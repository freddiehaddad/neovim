//! Window configuration API functions (`src/nvim/api/win_config.c`).

use crate::api::extmark::{parse_virt_text, virt_text_to_array};
use crate::api::private::helpers::{
    add_c, api_clear_error, api_free_array, api_free_object, api_set_error, api_typename,
    arena_array, array_obj, cbuf_to_string, cstr_as_string, cstr_to_obj, cstrn_as_string,
    error_set, find_window_by_handle, has_key, integer_obj, object_to_hl_id, string_obj, try_wrap,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectType, api_err_conflict, api_err_exp,
    api_err_invalid, api_err_required,
};
use crate::api::tabpage::{Buffer, Window};
use crate::autocmd::{EVENT_WINNEW, apply_autocmds_win, block_autocmds, unblock_autocmds};
use crate::buffer::{Bufref, bufref_valid, set_bufref};
use crate::charset::transstr;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::Lpos;
use crate::decoration::{VirtText, VirtTextChunk};
use crate::drawscreen::{UPD_NOT_VALID, redraw_later, set_must_redraw};
use crate::errors::*;
use crate::ex_docmd::expr_map_locked;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::highlight_group::{syn_check_group, syn_id2name};
use crate::lua::converter::Boolean;
use crate::mbyte::{mb_string2cells, mb_string2cells_len};
use crate::memory::Arena;
use crate::r#move::changed_window_setting;
use crate::msgpack_rpc::packer::Integer;
use crate::option::didset_window_options;
use crate::option_vars::*;
use crate::ui::ui_has;
use crate::ui_compositor::ui_comp_remove_grid;
use crate::window::{
    Frame, TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_NOENTER, WSP_TOP, WSP_VERT,
    WinConfig, WinInfoInfo, check_split_disallowed_err, clear_float_config, goto_tabpage_win,
    last_status, lastwin_nofloating, merge_win_config, one_window, win_append, win_comp_pos,
    win_find_tabpage, win_goto, win_locked, win_remove, win_set_buf, win_setheight_win,
    win_setwidth_win, win_split_ins, win_valid, win_valid_any_tab, window_layout_locked_err,
    winframe_find_altwin, winframe_remove, winframe_restore,
};
use crate::winfloat::{
    win_config_float, win_float_find_altwin, win_new_float, win_set_minimal_style,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type FloatAnchor = c_int;
pub type FloatRelative = c_int;
pub type WinSplit = c_int;
pub type WinStyle = c_int;
pub type AlignTextPos = c_int;
pub type BorderTextType = c_int;
pub type UIExtension = c_int;

pub(crate) const K_UIMULTIGRID: UIExtension = 6;
pub(crate) const K_ZINDEX_FLOAT_DEFAULT: c_int = 50;

pub(crate) const FR_COL: c_char = 2;

pub(crate) const K_CTX_NO_DISPLAY: c_int = 4;
pub(crate) const K_CTX_NO_EVENTS: c_int = 8;

pub(crate) const K_FLOAT_ANCHOR_EAST: FloatAnchor = 1;
pub(crate) const K_FLOAT_ANCHOR_SOUTH: FloatAnchor = 2;

pub(crate) const K_FLOAT_RELATIVE_EDITOR: FloatRelative = 0;
pub(crate) const K_FLOAT_RELATIVE_WINDOW: FloatRelative = 1;
pub(crate) const K_FLOAT_RELATIVE_CURSOR: FloatRelative = 2;
pub(crate) const K_FLOAT_RELATIVE_MOUSE: FloatRelative = 3;
pub(crate) const K_FLOAT_RELATIVE_TABLINE: FloatRelative = 4;
pub(crate) const K_FLOAT_RELATIVE_LASTSTATUS: FloatRelative = 5;

const K_WIN_SPLIT_LEFT: WinSplit = 0;
const K_WIN_SPLIT_RIGHT: WinSplit = 1;
const K_WIN_SPLIT_ABOVE: WinSplit = 2;
const K_WIN_SPLIT_BELOW: WinSplit = 3;

pub(crate) const K_WIN_STYLE_UNUSED: WinStyle = 0;
pub(crate) const K_WIN_STYLE_MINIMAL: WinStyle = 1;

pub(crate) const K_ALIGN_LEFT: AlignTextPos = 0;
pub(crate) const K_ALIGN_CENTER: AlignTextPos = 1;
pub(crate) const K_ALIGN_RIGHT: AlignTextPos = 2;

pub(crate) const K_BORDER_TEXT_TITLE: BorderTextType = 0;
pub(crate) const K_BORDER_TEXT_FOOTER: BorderTextType = 1;

const KEYSET_OPTIDX_WIN_CONFIG_COL: u64 = 1;
const KEYSET_OPTIDX_WIN_CONFIG_ROW: u64 = 2;
const KEYSET_OPTIDX_WIN_CONFIG_WIN: u64 = 3;
const KEYSET_OPTIDX_WIN_CONFIG_HIDE: u64 = 4;
const KEYSET_OPTIDX_WIN_CONFIG_WIDTH: u64 = 5;
const KEYSET_OPTIDX_WIN_CONFIG_SPLIT: u64 = 6;
const KEYSET_OPTIDX_WIN_CONFIG_TITLE: u64 = 7;
const KEYSET_OPTIDX_WIN_CONFIG_MOUSE: u64 = 8;
const KEYSET_OPTIDX_WIN_CONFIG_FIXED: u64 = 9;
const KEYSET_OPTIDX_WIN_CONFIG_STYLE: u64 = 10;
const KEYSET_OPTIDX_WIN_CONFIG_ANCHOR: u64 = 11;
const KEYSET_OPTIDX_WIN_CONFIG_BUFPOS: u64 = 12;
const KEYSET_OPTIDX_WIN_CONFIG_HEIGHT: u64 = 13;
const KEYSET_OPTIDX_WIN_CONFIG_ZINDEX: u64 = 14;
const KEYSET_OPTIDX_WIN_CONFIG_FOOTER: u64 = 15;
const KEYSET_OPTIDX_WIN_CONFIG_BORDER: u64 = 16;
const KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL: u64 = 17;
const KEYSET_OPTIDX_WIN_CONFIG_RELATIVE: u64 = 18;
const KEYSET_OPTIDX_WIN_CONFIG_VERTICAL: u64 = 19;
const KEYSET_OPTIDX_WIN_CONFIG_FOCUSABLE: u64 = 20;
const KEYSET_OPTIDX_WIN_CONFIG_NOAUTOCMD: u64 = 21;
const KEYSET_OPTIDX_WIN_CONFIG_TITLE_POS: u64 = 22;
const KEYSET_OPTIDX_WIN_CONFIG_FOOTER_POS: u64 = 23;
const KEYSET_OPTIDX_WIN_CONFIG_CMDLINE_OFFSET: u64 = 24;

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::charset::NUL;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_win_config {
    pub is_set_win_config: OptionalKeys,
    pub external: Boolean,
    pub fixed: Boolean,
    pub focusable: Boolean,
    pub footer: Object,
    pub footer_pos: NvimString,
    pub hide: Boolean,
    pub height: Integer,
    pub mouse: Boolean,
    pub relative: NvimString,
    pub row: f64,
    pub style: NvimString,
    pub noautocmd: Boolean,
    pub vertical: Boolean,
    pub win: Window,
    pub width: Integer,
    pub zindex: Integer,
    pub anchor: NvimString,
    pub border: Object,
    pub bufpos: Array,
    pub col: f64,
    pub split: NvimString,
    pub title: Object,
    pub title_pos: NvimString,
    pub _cmdline_offset: Integer,
}

pub const WIN_CONFIG_INIT: WinConfig = WinConfig {
    window: 0,
    bufpos: Lpos { lnum: -1, col: 0 },
    height: 0,
    width: 0,
    row: 0.0,
    col: 0.0,
    anchor: 0,
    relative: 0,
    external: false,
    focusable: true,
    mouse: true,
    split: 0,
    zindex: K_ZINDEX_FLOAT_DEFAULT,
    style: K_WIN_STYLE_UNUSED,
    border: false,
    shadow: false,
    border_chars: [[0; MAX_SCHAR_SIZE]; 8],
    border_hl_ids: [0; 8],
    border_attr: [0; 8],
    title: false,
    title_pos: 0,
    title_chunks: VirtText::new(),
    title_width: 0,
    footer: false,
    footer_pos: 0,
    footer_chunks: VirtText::new(),
    footer_width: 0,
    noautocmd: false,
    fixed: false,
    hide: false,
    _cmdline_offset: c_int::MAX,
};

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
// DLLEXPORT Window nvim_open_win(Buffer buf, Boolean enter, Dict(win_config) *config, Error *err)
//   FUNC_API_SINCE(6) FUNC_API_TEXTLOCK;
pub fn nvim_open_win(
    buf: Buffer,
    enter: Boolean,
    config: &mut KeyDict_win_config,
    err: &mut Error,
) -> Window {
    unsafe {
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return 0;
        };

        let mask = config.is_set_win_config;
        let is_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT)
            || has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
        let mut fconfig = WIN_CONFIG_INIT;
        if !parse_win_config(
            ptr::null_mut(),
            &mut *config,
            &mut fconfig,
            false,
            !is_split,
            err,
        ) {
            return 0;
        }

        let mut rv: Window = 0;
        if fconfig.noautocmd {
            block_autocmds();
        }

        'cleanup: {
            let mut wp: *mut WinInfoInfo = ptr::null_mut();
            let mut tp = g_st().curtab;
            assert!(!(curwin_ptr()).is_null());
            let mut parent = if config.win == 0 {
                curwin_ptr()
            } else {
                ptr::null_mut()
            };
            if config.win > 0 {
                parent = find_window_by_handle(fconfig.window, err);
                if parent.is_null() {
                    break 'cleanup;
                } else if is_split && (*parent).w_floating {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Cannot split a floating window",
                    );
                    break 'cleanup;
                }
                tp = win_find_tabpage(parent);
            }
            if is_split {
                if !check_split_disallowed_err(
                    if !parent.is_null() {
                        parent
                    } else {
                        curwin_ptr()
                    },
                    err,
                ) {
                    break 'cleanup;
                }

                if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL)
                    && !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT)
                {
                    if config.vertical {
                        fconfig.split = if opt_vars().p_spr != 0 {
                            K_WIN_SPLIT_RIGHT
                        } else {
                            K_WIN_SPLIT_LEFT
                        };
                    } else {
                        fconfig.split = if opt_vars().p_sb != 0 {
                            K_WIN_SPLIT_BELOW
                        } else {
                            K_WIN_SPLIT_ABOVE
                        };
                    }
                }
                let flags = win_split_flags(fconfig.split, parent.is_null()) | WSP_NOENTER;
                let size = if (flags & WSP_VERT) != 0 {
                    fconfig.width
                } else {
                    fconfig.height
                };

                try_wrap(err, || {
                    if parent.is_null() || parent == curwin_ptr() {
                        wp = win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut());
                    } else {
                        let mut switchwin: CtxSwitch = CtxSwitch::new();
                        let result = ctx_switch(
                            &mut switchwin,
                            parent,
                            tp,
                            ptr::null_mut(),
                            K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
                        );
                        assert!(result);
                        wp = win_split_ins(size, flags, ptr::null_mut(), 0, ptr::null_mut());
                        ctx_restore(&mut switchwin);
                    }
                });
                if !wp.is_null() {
                    (*wp).w_config = fconfig.clone();
                    if size > 0 {
                        if (flags & WSP_VERT) != 0 && (*wp).w_width != size {
                            win_setwidth_win(size, &mut *wp, true);
                        } else if (flags & WSP_VERT) == 0 && (*wp).w_height != size {
                            win_setheight_win(size, &mut *wp, true);
                        }
                    }
                }
            } else {
                if curwin_mut().buf_mut().b_locked_split != 0 {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"E1159: Cannot open a float when closing the buffer",
                    );
                    break 'cleanup;
                }
                wp = win_new_float(ptr::null_mut(), false, fconfig.clone(), err);
            }
            if wp.is_null() {
                if !error_set(err) {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Failed to create window",
                    );
                }
                break 'cleanup;
            }

            if fconfig._cmdline_offset < c_int::MAX {
                g_st().cmdline_win = wp;
            }

            let mut bufref = Bufref {
                br_buf: ptr::null_mut(),
                br_fnum: 0,
                br_buf_free_count: 0,
            };
            set_bufref(&mut bufref, (&raw mut *b));
            if !fconfig.noautocmd {
                let mut switchwin: CtxSwitch = CtxSwitch::new();
                let result = ctx_switch(&mut switchwin, wp, tp, ptr::null_mut(), K_CTX_NO_DISPLAY);
                assert!(result);
                if apply_autocmds_win(
                    EVENT_WINNEW,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    g_st().curbuf,
                    wp.as_ref(),
                ) {
                    tp = win_find_tabpage(wp);
                }
                ctx_restore(&mut switchwin);
            }
            if !tp.is_null() && enter {
                goto_tabpage_win(&mut *tp, wp);
                tp = win_find_tabpage(wp);
            }
            if !tp.is_null() && bufref_valid(Some(&bufref)) && (&raw mut *b) != (*wp).w_buffer {
                let au_no_enter_leave = curwin_ptr() != wp && !fconfig.noautocmd;
                if au_no_enter_leave {
                    g_st().autocmd_no_enter += 1;
                    g_st().autocmd_no_leave += 1;
                }
                win_set_buf(&mut *wp, b, err);
                if !fconfig.noautocmd {
                    tp = win_find_tabpage(wp);
                }
                if au_no_enter_leave {
                    g_st().autocmd_no_enter -= 1;
                    g_st().autocmd_no_leave -= 1;
                }
            }
            if tp.is_null() {
                api_clear_error(err);
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Window was closed immediately",
                );
                break 'cleanup;
            }

            if fconfig.style == K_WIN_STYLE_MINIMAL {
                win_set_minimal_style(&mut *wp);
                didset_window_options(&mut *wp, true);
                changed_window_setting(&mut *wp);
            }
            rv = (*wp).handle;
        }

        if fconfig.noautocmd {
            unblock_autocmds();
        }
        rv
    }
}

fn win_split_dir(win: &WinInfoInfo) -> WinSplit {
    unsafe {
        if win.w_frame.is_null() || (*win.w_frame).fr_parent.is_null() {
            return K_WIN_SPLIT_LEFT;
        }

        let layout = (*(*win.w_frame).fr_parent).fr_layout;
        if layout == FR_COL {
            if !(*win.w_frame).fr_next.is_null() {
                K_WIN_SPLIT_ABOVE
            } else {
                K_WIN_SPLIT_BELOW
            }
        } else if !(*win.w_frame).fr_next.is_null() {
            K_WIN_SPLIT_LEFT
        } else {
            K_WIN_SPLIT_RIGHT
        }
    }
}

const fn win_split_flags(split: WinSplit, toplevel: bool) -> c_int {
    let mut flags = 0;
    if split == K_WIN_SPLIT_ABOVE || split == K_WIN_SPLIT_BELOW {
        flags |= WSP_HOR;
    } else {
        flags |= WSP_VERT;
    }
    if split == K_WIN_SPLIT_ABOVE || split == K_WIN_SPLIT_LEFT {
        flags |= if toplevel { WSP_TOP } else { WSP_ABOVE };
    } else {
        flags |= if toplevel { WSP_BOT } else { WSP_BELOW };
    }
    flags
}

fn win_can_move_tp(wp: *mut WinInfoInfo, tp: &mut TabPageInfo, err: &mut Error) -> bool {
    unsafe {
        if one_window(
            &mut *wp,
            if &raw mut *tp == g_st().curtab {
                ptr::null_mut()
            } else {
                &raw mut *tp
            },
        ) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Cannot move last non-floating window",
            );
            return false;
        }
        if win_locked(&mut *wp) != 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Cannot move window to another tabpage whilst in use",
            );
            return false;
        }
        if window_layout_locked_err(crate::cmdexpand::Cmdidx::CmdSize, err) {
            return false;
        }
        if g_st().textlock != 0 || expr_map_locked() {
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", E_TEXTLOCK,);
            return false;
        }
        if is_ctx_win(wp) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Cannot move autocmd window to another tabpage",
            );
            return false;
        }
        true
    }
}

fn win_find_altwin(win: *mut WinInfoInfo, tp: &mut TabPageInfo) -> *mut WinInfoInfo {
    unsafe {
        let tp_arg = if ptr::eq(tp, g_st().curtab) {
            ptr::null_mut()
        } else {
            &raw mut *tp
        };
        if (*win).w_floating {
            win_float_find_altwin(&*win, tp_arg.as_ref())
        } else {
            let mut dir: c_int = 0;
            winframe_find_altwin(&mut *win, &mut dir, tp_arg, ptr::null_mut())
        }
    }
}

fn win_config_split(
    win: &mut WinInfoInfo,
    config: &KeyDict_win_config,
    fconfig: &mut WinConfig,
    err: &mut Error,
) -> bool {
    unsafe {
        let was_split = !win.w_floating;
        let mask = config.is_set_win_config;
        let has_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT);
        let has_vertical = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
        let old_split = win_split_dir(win);
        if has_vertical && !has_split {
            if config.vertical {
                fconfig.split = if old_split == K_WIN_SPLIT_RIGHT || opt_vars().p_spr != 0 {
                    K_WIN_SPLIT_RIGHT
                } else {
                    K_WIN_SPLIT_LEFT
                };
            } else {
                fconfig.split = if old_split == K_WIN_SPLIT_BELOW || opt_vars().p_sb != 0 {
                    K_WIN_SPLIT_BELOW
                } else {
                    K_WIN_SPLIT_ABOVE
                };
            }
        }

        'resize: {
            if (!has_vertical && !has_split)
                || (was_split
                    && !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN)
                    && old_split == fconfig.split)
            {
                break 'resize;
            }

            let mut parent: *mut WinInfoInfo = ptr::null_mut();
            let mut parent_tp: *mut TabPageInfo = ptr::null_mut();
            if config.win == 0 {
                parent = curwin_ptr();
                parent_tp = g_st().curtab;
            } else if config.win > 0 {
                parent = find_window_by_handle(fconfig.window, err);
                if parent.is_null() {
                    return false;
                }
                parent_tp = win_find_tabpage(parent);
            }

            let mut win_tp = win_find_tabpage(&raw mut *win);
            if !parent.is_null() {
                if (*parent).w_floating {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Cannot split a floating window",
                    );
                    return false;
                }
                if win_tp != parent_tp && !win_can_move_tp((&raw mut *win), &mut *win_tp, err) {
                    return false;
                }
            }

            if !check_split_disallowed_err((&raw mut *win), err) {
                return false;
            }

            let mut to_split_ok = false;
            let curwin_moving_tp =
                (&raw mut *win) == curwin_ptr() && !parent.is_null() && win_tp != parent_tp;

            'restore_curwin: {
                if curwin_moving_tp {
                    let altwin = win_find_altwin((&raw mut *win), &mut *win_tp);
                    assert!(!altwin.is_null());
                    win_goto(altwin);

                    if curwin_ptr() == (&raw mut *win) {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Failed to switch away from window %d",
                            win.handle,
                        );
                        return false;
                    }
                    win_tp = win_find_tabpage(&raw mut *win);
                    if win_tp.is_null() || !win_valid_any_tab(parent) {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Windows to split were closed",
                        );
                        break 'restore_curwin;
                    }
                    if was_split == win.w_floating || (*parent).w_floating {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Floating state of windows to split changed",
                        );
                        break 'restore_curwin;
                    }
                }

                let mut dir: c_int = 0;
                let mut unflat_altfr: *mut Frame = ptr::null_mut();
                let altwin: *mut WinInfoInfo;

                if was_split {
                    if (*win.w_frame).fr_parent.is_null() {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Cannot move last non-floating window",
                        );
                        break 'restore_curwin;
                    } else if !parent.is_null() && (*parent).handle == win.handle {
                        let mut n_frames = 0;
                        for _fr in frame_iter_mut((*(*win.w_frame).fr_parent).fr_child) {
                            n_frames += 1;
                        }

                        let mut neighbor: *mut WinInfoInfo = ptr::null_mut();

                        if n_frames > 2 {
                            let frame = (*win.w_frame).fr_parent;
                            if !(*frame).fr_parent.is_null() {
                                neighbor = if fconfig.split == K_WIN_SPLIT_ABOVE
                                    || fconfig.split == K_WIN_SPLIT_LEFT
                                {
                                    win.w_next
                                } else {
                                    win.w_prev
                                };
                            }
                            altwin = winframe_remove(
                                win,
                                &mut dir,
                                if win_tp == g_st().curtab {
                                    ptr::null_mut()
                                } else {
                                    win_tp
                                },
                                &mut unflat_altfr,
                            );
                        } else if n_frames == 2 {
                            altwin = winframe_remove(
                                win,
                                &mut dir,
                                if win_tp == g_st().curtab {
                                    ptr::null_mut()
                                } else {
                                    win_tp
                                },
                                &mut unflat_altfr,
                            );
                            neighbor = altwin;
                        } else {
                            api_set_error!(
                                err,
                                ErrorType::ErrorTypeException,
                                c"Cannot split window into itself",
                            );
                            break 'restore_curwin;
                        }
                        parent = neighbor;
                    } else {
                        altwin = winframe_remove(
                            win,
                            &mut dir,
                            if win_tp == g_st().curtab {
                                ptr::null_mut()
                            } else {
                                win_tp
                            },
                            &mut unflat_altfr,
                        );
                    }
                } else {
                    altwin =
                        win_float_find_altwin(win, (win_tp != g_st().curtab).then(|| &*win_tp));
                }

                win_remove(
                    win,
                    if win_tp == g_st().curtab {
                        ptr::null_mut()
                    } else {
                        win_tp
                    },
                );
                if win_tp == g_st().curtab {
                    last_status(false);
                    win_comp_pos();
                }

                let flags = win_split_flags(fconfig.split, parent.is_null()) | WSP_NOENTER;
                parent_tp = if !parent.is_null() {
                    win_find_tabpage(parent)
                } else {
                    g_st().curtab
                };

                try_wrap(err, || {
                    let need_switch = !parent.is_null() && parent != curwin_ptr();
                    let mut switchwin: CtxSwitch = CtxSwitch::new();
                    if need_switch {
                        let result = ctx_switch(
                            &mut switchwin,
                            parent,
                            parent_tp,
                            ptr::null_mut(),
                            K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
                        );
                        assert!(result);
                    }
                    to_split_ok =
                        !win_split_ins(0, flags, (&raw mut *win), 0, unflat_altfr).is_null();
                    if !to_split_ok {
                        win_append(
                            win.w_prev,
                            (&raw mut *win),
                            if win_tp == g_st().curtab {
                                ptr::null_mut()
                            } else {
                                win_tp
                            },
                        );
                    }
                    if need_switch {
                        ctx_restore(&mut switchwin);
                    }
                });
                if !to_split_ok {
                    if was_split {
                        winframe_restore(win, dir, &mut *unflat_altfr);
                    }
                    if !error_set(err) {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Failed to move window %d into split",
                            win.handle,
                        );
                    }
                    break 'restore_curwin;
                }

                if win_tp != parent_tp && (*win_tp).tp_curwin == (&raw mut *win) {
                    (*win_tp).tp_curwin = altwin;
                }
                break 'resize;
            }

            if curwin_moving_tp && win_valid(&raw mut *win) {
                win_goto(&raw mut *win);
            }
            return false;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIDTH) {
            win_setwidth_win(fconfig.width, win, true);
        }
        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_HEIGHT) {
            win_setheight_win(fconfig.height, win, true);
        }

        if !was_split {
            clear_float_config(fconfig, false);
        }
        merge_win_config(&mut win.w_config, fconfig.clone());
        true
    }
}

fn win_config_float_tp(
    win: &mut WinInfoInfo,
    config: &KeyDict_win_config,
    fconfig: &WinConfig,
    err: &mut Error,
) -> bool {
    unsafe {
        let mut win_tp = win_find_tabpage(&raw mut *win);
        let mut parent = (&raw mut *win);
        let mut parent_tp = win_tp;
        if has_key(config.is_set_win_config, KEYSET_OPTIDX_WIN_CONFIG_WIN) {
            parent = find_window_by_handle(fconfig.window, err);
            if parent.is_null() {
                return false;
            }
            parent_tp = win_find_tabpage(parent);
        }

        let mut curwin_moving_tp = false;
        let mut altwin: *mut WinInfoInfo = ptr::null_mut();

        'restore_curwin: {
            if win_tp != parent_tp {
                if !win_can_move_tp((&raw mut *win), &mut *win_tp, err) {
                    return false;
                }
                altwin = win_find_altwin((&raw mut *win), &mut *win_tp);
                assert!(!altwin.is_null());

                if curwin_ptr() == (&raw mut *win) {
                    curwin_moving_tp = true;
                    win_goto(altwin);

                    if curwin_ptr() == (&raw mut *win) {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Failed to switch away from window %d",
                            win.handle,
                        );
                        return false;
                    }
                    win_tp = win_find_tabpage(&raw mut *win);
                    parent_tp = win_find_tabpage(parent);

                    if win_tp.is_null() || parent_tp.is_null() {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeException,
                            c"Target windows were closed",
                        );
                        break 'restore_curwin;
                    }
                    if win_tp != parent_tp && !win_can_move_tp((&raw mut *win), &mut *win_tp, err) {
                        break 'restore_curwin;
                    }
                    altwin = win_find_altwin((&raw mut *win), &mut *win_tp);
                    assert!(!altwin.is_null());
                }
            }

            if !win.w_floating {
                if win_new_float((&raw mut *win), false, fconfig.clone(), err).is_null() {
                    break 'restore_curwin;
                }
                redraw_later(Some(&mut *win), UPD_NOT_VALID);
            }

            if win_tp != parent_tp {
                win_remove(
                    win,
                    if win_tp == g_st().curtab {
                        ptr::null_mut()
                    } else {
                        win_tp
                    },
                );
                let append_tp = if parent_tp == g_st().curtab {
                    ptr::null_mut()
                } else {
                    parent_tp
                };
                win_append(lastwin_nofloating(append_tp), (&raw mut *win), append_tp);

                if win_tp != g_st().curtab && (*win_tp).tp_curwin == (&raw mut *win) {
                    (*win_tp).tp_curwin = altwin;
                }

                ui_comp_remove_grid(&mut win.w_grid_alloc);

                redraw_later(Some(&mut *win), UPD_NOT_VALID);
                set_must_redraw(UPD_NOT_VALID);
            }

            win_config_float(win, fconfig.clone());
            return true;
        }

        if curwin_moving_tp && win_valid(&raw mut *win) {
            win_goto(&raw mut *win);
        }
        false
    }
}

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
// DLLEXPORT void nvim_win_set_config(Window win, Dict(win_config) *config, Error *err)
//   FUNC_API_SINCE(6);
pub fn nvim_win_set_config(win: Window, config: &mut KeyDict_win_config, err: &mut Error) {
    {
        let Some(w) = crate::api::private::helpers::find_window_by_handle_mut(win, err) else {
            return;
        };

        let was_split = !w.w_floating;
        let mask = config.is_set_win_config;
        let has_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT);
        let has_vertical = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
        let old_style = w.w_config.style;
        let mut fconfig = w.w_config.clone();

        let to_split = config.relative.size == 0
            && !(has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL) && config.external)
            && (has_split || has_vertical || was_split);
        let will_float = (!was_split && !to_split) || config.relative.size > 0 || config.external;

        if !parse_win_config(
            (&raw mut *w),
            &mut *config,
            &mut fconfig,
            !was_split || to_split,
            will_float,
            err,
        ) {
            return;
        }

        if to_split {
            if !win_config_split(w, &*config, &mut fconfig, err) {
                return;
            }
        } else if !win_config_float_tp(w, &*config, &fconfig, err) {
            return;
        }

        if fconfig.style == K_WIN_STYLE_MINIMAL && old_style != fconfig.style {
            win_set_minimal_style(w);
            didset_window_options(w, true);
            changed_window_setting(w);
        }
        if fconfig._cmdline_offset < c_int::MAX {
            g_st().cmdline_win = (&raw mut *w);
        } else if (&raw mut *w) == g_st().cmdline_win && fconfig._cmdline_offset == c_int::MAX {
            g_st().cmdline_win = ptr::null_mut();
        }
    }
}

fn config_put_bordertext(
    config: &mut KeyDict_win_config,
    fconfig: &WinConfig,
    bordertext_type: BorderTextType,
    arena: &mut Arena,
) {
    let (vt, align) = match bordertext_type {
        K_BORDER_TEXT_TITLE => (&fconfig.title_chunks, fconfig.title_pos),
        _ => (&fconfig.footer_chunks, fconfig.footer_pos),
    };

    let bordertext = virt_text_to_array(vt, true, arena);

    let pos = match align {
        K_ALIGN_LEFT => c"left".as_ptr(),
        K_ALIGN_CENTER => c"center".as_ptr(),
        _ => c"right".as_ptr(),
    };

    if bordertext_type == K_BORDER_TEXT_TITLE {
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_TITLE;
        config.title = array_obj(bordertext);
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_TITLE_POS;
        config.title_pos = cstr_as_string(pos);
    } else {
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOOTER;
        config.footer = array_obj(bordertext);
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOOTER_POS;
        config.footer_pos = cstr_as_string(pos);
    }
}

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
// DLLEXPORT Dict(win_config) nvim_win_get_config(Window win, Arena *arena, Error *err)
//   FUNC_API_SINCE(6);
pub fn nvim_win_get_config(win: Window, arena: &mut Arena, err: &mut Error) -> KeyDict_win_config {
    unsafe {
        const FLOAT_RELATIVE_STR: [&CStr; 6] = [
            c"editor",
            c"win",
            c"cursor",
            c"mouse",
            c"tabline",
            c"laststatus",
        ];
        const WIN_SPLIT_STR: [&CStr; 4] = [c"left", c"right", c"above", c"below"];
        const WIN_STYLE_STR: [&CStr; 2] = [c"", c"minimal"];

        let mut rv = KeyDict_win_config::default();

        let Some(wp) = crate::api::private::helpers::find_window_by_handle_mut(win, err) else {
            return rv;
        };

        let config = &wp.w_config;

        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOCUSABLE;
        rv.focusable = config.focusable;
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL;
        rv.external = config.external;
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HIDE;
        rv.hide = config.hide;
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_MOUSE;
        rv.mouse = config.mouse;
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_STYLE;
        rv.style = cstr_as_string(WIN_STYLE_STR[config.style as usize].as_ptr());

        if wp.w_floating {
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIDTH;
            rv.width = i64::from(config.width);
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HEIGHT;
            rv.height = i64::from(config.height);
            if !config.external {
                if config.relative == K_FLOAT_RELATIVE_WINDOW {
                    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIN;
                    rv.win = config.window;
                    if config.bufpos.lnum >= 0 {
                        let mut pos = arena_array(arena, 2);
                        add_c(&mut pos, integer_obj(i64::from(config.bufpos.lnum)));
                        add_c(&mut pos, integer_obj(i64::from(config.bufpos.col)));
                        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BUFPOS;
                        rv.bufpos = pos;
                    }
                }
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ANCHOR;
                rv.anchor = cstr_as_string(FLOAT_ANCHOR_STR[config.anchor as usize].as_ptr());
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ROW;
                rv.row = config.row;
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_COL;
                rv.col = config.col;
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ZINDEX;
                rv.zindex = i64::from(config.zindex);
            }
            if config.border {
                let mut border = arena_array(arena, 8);
                for i in 0..8 {
                    let s = cstrn_as_string(config.border_chars[i].as_ptr(), MAX_SCHAR_SIZE);
                    let hi_id = config.border_hl_ids[i];
                    let hi_name = syn_id2name(hi_id);
                    if *hi_name != 0 {
                        let mut tuple = arena_array(arena, 2);
                        add_c(&mut tuple, string_obj(s));
                        add_c(&mut tuple, string_obj(cstr_as_string(hi_name)));
                        add_c(&mut border, array_obj(tuple));
                    } else {
                        add_c(&mut border, string_obj(s));
                    }
                }
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BORDER;
                rv.border = array_obj(border);
                if config.title {
                    config_put_bordertext(&mut rv, config, K_BORDER_TEXT_TITLE, arena);
                }
                if config.footer {
                    config_put_bordertext(&mut rv, config, K_BORDER_TEXT_FOOTER, arena);
                }
            } else {
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BORDER;
                rv.border = string_obj(cstr_as_string(c"none"));
            }
        } else if !config.external {
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIDTH;
            rv.width = i64::from(wp.w_width);
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HEIGHT;
            rv.height = i64::from(wp.w_height);
            let split = win_split_dir(wp);
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_SPLIT;
            rv.split = cstr_as_string(WIN_SPLIT_STR[split as usize].as_ptr());
        }

        let rel = if wp.w_floating && !config.external {
            FLOAT_RELATIVE_STR[config.relative as usize].as_ptr()
        } else {
            c"".as_ptr()
        };
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_RELATIVE;
        rv.relative = cstr_as_string(rel);
        if config._cmdline_offset < c_int::MAX {
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_CMDLINE_OFFSET;
            rv._cmdline_offset = i64::from(config._cmdline_offset);
        }

        rv
    }
}

fn parse_float_anchor(anchor: NvimString, out: &mut FloatAnchor) -> bool {
    {
        if anchor.size == 0 {
            *out = 0;
        }
        let str_ptr = anchor.data;
        if crate::memory::cstr_as_opt_bytes(str_ptr).is_some_and(|b| b.eq_ignore_ascii_case(b"NW"))
        {
            *out = 0;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"NE"))
        {
            *out = K_FLOAT_ANCHOR_EAST;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"SW"))
        {
            *out = K_FLOAT_ANCHOR_SOUTH;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"SE"))
        {
            *out = K_FLOAT_ANCHOR_SOUTH | K_FLOAT_ANCHOR_EAST;
        } else {
            return false;
        }
        true
    }
}

fn parse_float_relative(relative: NvimString, out: &mut FloatRelative) -> bool {
    {
        let str_ptr = relative.data;
        if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"editor"))
        {
            *out = K_FLOAT_RELATIVE_EDITOR;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"win"))
        {
            *out = K_FLOAT_RELATIVE_WINDOW;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"cursor"))
        {
            *out = K_FLOAT_RELATIVE_CURSOR;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"mouse"))
        {
            *out = K_FLOAT_RELATIVE_MOUSE;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"tabline"))
        {
            *out = K_FLOAT_RELATIVE_TABLINE;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"laststatus"))
        {
            *out = K_FLOAT_RELATIVE_LASTSTATUS;
        } else {
            return false;
        }
        true
    }
}

fn parse_config_split(split: NvimString, out: &mut WinSplit) -> bool {
    {
        let str_ptr = split.data;
        if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"left"))
        {
            *out = K_WIN_SPLIT_LEFT;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"right"))
        {
            *out = K_WIN_SPLIT_RIGHT;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"above"))
        {
            *out = K_WIN_SPLIT_ABOVE;
        } else if crate::memory::cstr_as_opt_bytes(str_ptr)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"below"))
        {
            *out = K_WIN_SPLIT_BELOW;
        } else {
            return false;
        }
        true
    }
}

fn parse_float_bufpos(bufpos: Array, out: &mut Lpos) -> bool {
    if bufpos.size != 2
        || bufpos[0].obj_type != ObjectType::ObjectTypeInteger
        || bufpos[1].obj_type != ObjectType::ObjectTypeInteger
    {
        return false;
    }
    out.lnum = bufpos[0].data.integer() as i32;
    out.col = bufpos[1].data.integer() as i32;
    true
}

// DLLEXPORT void parse_bordertext(Object bordertext, BorderTextType bordertext_type,
//                                 WinConfig *fconfig, Error *err);
pub fn parse_bordertext(
    bordertext: Object,
    bordertext_type: BorderTextType,
    fconfig: &mut WinConfig,
    err: &mut Error,
) {
    if bordertext.obj_type != ObjectType::ObjectTypeString
        && bordertext.obj_type != ObjectType::ObjectTypeArray
    {
        api_err_exp(
            err,
            c"title/footer",
            c"String or Array",
            api_typename(bordertext.obj_type),
        );
        return;
    }

    if bordertext.obj_type == ObjectType::ObjectTypeArray && bordertext.data.array().size == 0 {
        api_err_exp(err, c"title/footer", c"non-empty Array", ptr::null());
        return;
    }

    let is_title = bordertext_type == K_BORDER_TEXT_TITLE;
    let is_present = if is_title {
        &mut fconfig.title
    } else {
        &mut fconfig.footer
    };
    let chunks = if is_title {
        &mut fconfig.title_chunks
    } else {
        &mut fconfig.footer_chunks
    };
    let width = if is_title {
        &mut fconfig.title_width
    } else {
        &mut fconfig.footer_width
    };

    if bordertext.obj_type == ObjectType::ObjectTypeString {
        if bordertext.data.string().size == 0 {
            *is_present = false;
            return;
        }
        let text_cstr = transstr(bordertext.data.string().data, true);
        *width = mb_string2cells(text_cstr.as_ptr()) as c_int;
        chunks.clear();
        chunks.push(VirtTextChunk {
            text: Some(text_cstr),
            hl_id: -1,
        });
    } else {
        *chunks = parse_virt_text(bordertext.data.array(), err, width, true);
        if error_set(err) {
            return;
        }
    }

    *is_present = true;
}

fn parse_bordertext_pos(
    wp: Option<&WinInfoInfo>,
    bordertext_pos: NvimString,
    bordertext_type: BorderTextType,
    fconfig: &mut WinConfig,
    err: &mut Error,
) -> bool {
    let align = if bordertext_type == K_BORDER_TEXT_TITLE {
        &mut fconfig.title_pos
    } else {
        &mut fconfig.footer_pos
    };

    if bordertext_pos.size == 0 {
        if wp.is_none() {
            *align = K_ALIGN_LEFT;
        }
        return true;
    }

    let pos = bordertext_pos.data;
    if crate::memory::cstr_as_opt_bytes(pos) == Some(b"left") {
        *align = K_ALIGN_LEFT;
    } else if crate::memory::cstr_as_opt_bytes(pos) == Some(b"center") {
        *align = K_ALIGN_CENTER;
    } else if crate::memory::cstr_as_opt_bytes(pos) == Some(b"right") {
        *align = K_ALIGN_RIGHT;
    } else {
        api_err_invalid(
            err,
            if bordertext_type == K_BORDER_TEXT_TITLE {
                c"title_pos".as_ptr()
            } else {
                c"footer_pos".as_ptr()
            },
            pos,
            0,
            true,
        );
        return false;
    }
    true
}

// DLLEXPORT void parse_border_style(Object style, WinConfig *fconfig, Error *err);
pub fn parse_border_style(style: Object, fconfig: Option<&mut WinConfig>, err: &mut Error) {
    unsafe {
        const DEFAULTS_CHARS: [[&str; 8]; 6] = [
            ["╔", "═", "╗", "║", "╝", "═", "╚", "║"],
            ["┌", "─", "┐", "│", "┘", "─", "└", "│"],
            ["", "", " ", " ", " ", " ", " ", ""],
            ["╭", "─", "╮", "│", "╯", "─", "╰", "│"],
            [" ", " ", " ", " ", " ", " ", " ", " "],
            ["┏", "━", "┓", "┃", "┛", "━", "┗", "┃"],
        ];
        const DEFAULTS_SHADOW: [bool; 6] = [false, false, true, false, false, false];

        let fconfig = fconfig.unwrap();
        fconfig.border = true;
        let chars = &mut fconfig.border_chars;
        let hl_ids = &mut fconfig.border_hl_ids;

        if style.obj_type == ObjectType::ObjectTypeArray {
            let arr = style.data.array();
            let mut size = arr.size;
            if size == 0 || size > 8 || (size & (size - 1)) != 0 {
                api_err_exp(err, c"border", c"1, 2, 4, or 8 chars", ptr::null());
                return;
            }
            for i in 0..size {
                let iytem = arr[i];
                let string: NvimString;
                let mut hl_id: c_int = 0;
                if iytem.obj_type == ObjectType::ObjectTypeArray {
                    let iarr = iytem.data.array();
                    if iarr.size == 0 || iarr.size > 2 {
                        api_err_exp(err, c"border", c"1 or 2-item Array", ptr::null());
                        return;
                    }
                    let first = iarr[0];
                    if first.obj_type != ObjectType::ObjectTypeString {
                        api_err_exp(err, c"border", c"Array of Strings", ptr::null());
                        return;
                    }
                    string = first.data.string();
                    if iarr.size == 2 {
                        hl_id = object_to_hl_id(iarr[1], c"border char highlight", err);
                        if error_set(err) {
                            return;
                        }
                    }
                } else if iytem.obj_type == ObjectType::ObjectTypeString {
                    string = iytem.data.string();
                } else {
                    api_err_exp(
                        err,
                        c"border",
                        c"String or Array",
                        api_typename(iytem.obj_type),
                    );
                    return;
                }
                if string.size != 0 && mb_string2cells_len(string.data, string.size) > 1 {
                    api_err_exp(err, c"border", c"only one-cell chars", ptr::null());
                    return;
                }
                let len = string.size.min(MAX_SCHAR_SIZE - 1);
                if len > 0 {
                    ptr::copy_nonoverlapping(string.data, chars[i].as_mut_ptr(), len);
                }
                chars[i][len] = NUL;
                hl_ids[i] = hl_id;
            }
            while size < 8 {
                for j in 0..size {
                    chars[size + j] = chars[j];
                    hl_ids[size + j] = hl_ids[j];
                }
                size <<= 1;
            }
            if (chars[7][0] != 0 && chars[1][0] != 0 && chars[0][0] == 0)
                || (chars[1][0] != 0 && chars[3][0] != 0 && chars[2][0] == 0)
                || (chars[3][0] != 0 && chars[5][0] != 0 && chars[4][0] == 0)
                || (chars[5][0] != 0 && chars[7][0] != 0 && chars[6][0] == 0)
            {
                api_err_exp(
                    err,
                    c"border",
                    c"corner char between edge chars",
                    ptr::null(),
                );
            }
        } else if style.obj_type == ObjectType::ObjectTypeString {
            let str_val = style.data.string();
            if str_val.size == 0
                || (crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"none"))
            {
                fconfig.border = false;
                fconfig.title = false;
                fconfig.footer = false;
                return;
            }
            for i in 0..6 {
                let def_name = OPT_WINBORDER_VALUES[i + 1].as_ptr();
                if crate::memory::cstr_as_opt_bytes(str_val.data)
                    == crate::memory::cstr_as_opt_bytes(def_name)
                {
                    *chars = [[0; MAX_SCHAR_SIZE]; 8];
                    for (slot, piece) in chars.iter_mut().zip(DEFAULTS_CHARS[i].iter()) {
                        let bytes = piece.as_bytes();
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            slot.as_mut_ptr(),
                            bytes.len(),
                        );
                    }
                    *hl_ids = [0; 8];
                    if DEFAULTS_SHADOW[i] {
                        let hl_blend = syn_check_group(c"FloatShadow", 11);
                        let hl_through = syn_check_group(c"FloatShadowThrough", 18);
                        hl_ids[2] = hl_through;
                        hl_ids[3] = hl_blend;
                        hl_ids[4] = hl_blend;
                        hl_ids[5] = hl_blend;
                        hl_ids[6] = hl_through;
                    }
                    return;
                }
            }
            api_err_invalid(err, c"border", str_val.data, 0, true);
        }
    }
}

fn generate_api_error(wp: Option<&WinInfoInfo>, attribute: &std::ffi::CStr, err: &mut Error) {
    unsafe {
        if let Some(wp) = wp
            && wp.w_floating
        {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Required: 'relative' when reconfiguring floating window %d",
                wp.handle,
            );
        } else {
            api_err_conflict(err, attribute.as_ptr(), c"non-float window");
        }
    }
}

/// Parses a border style name or custom (comma-separated) style.
// DLLEXPORT bool parse_winborder(WinConfig *fconfig, char *border_opt, Error *err);
pub fn parse_winborder(
    fconfig: Option<&mut WinConfig>,
    border_opt: impl crate::strings::IntoCCharPtr,
    err: &mut Error,
) -> bool {
    let border_opt = border_opt.into_c_char_ptr();
    {
        if fconfig.is_none() {
            return false;
        }
        let opt_bytes = crate::memory::cstr_as_bytes(border_opt);
        let style = if opt_bytes.contains(&b',') {
            let mut border_chars = Array::new();
            for part in opt_bytes.split(|&b| b == b',') {
                if border_chars.size >= 8 {
                    api_free_array(border_chars);
                    return false;
                }
                border_chars.push(string_obj(cbuf_to_string(
                    part.as_ptr().cast::<c_char>(),
                    part.len(),
                )));
            }
            if border_chars.size != 8 {
                api_free_array(border_chars);
                return false;
            }
            array_obj(border_chars)
        } else {
            cstr_to_obj(border_opt)
        };

        parse_border_style(style, (fconfig.is_some()).then(|| fconfig.unwrap()), err);
        api_free_object(style);
        !error_set(err)
    }
}

fn parse_win_config(
    wp: *mut WinInfoInfo,
    config: &mut KeyDict_win_config,
    fconfig: &mut WinConfig,
    reconf: bool,
    will_float: bool,
    err: &mut Error,
) -> bool {
    unsafe {
        let mut has_relative = false;
        let mut relative_is_win = false;
        let mut is_split = false;
        let mask = config.is_set_win_config;

        'fail: {
            if config.relative.size > 0 {
                if !parse_float_relative(config.relative, &mut fconfig.relative) {
                    api_err_invalid(err, c"relative", config.relative.data, 0, true);
                    break 'fail;
                }

                if !(has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ROW)
                    && has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_COL))
                    && !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BUFPOS)
                {
                    api_err_required(err, c"'relative' requires 'row'/'col' or 'bufpos'");
                    break 'fail;
                }

                has_relative = true;
                fconfig.external = false;
                if fconfig.relative == K_FLOAT_RELATIVE_WINDOW {
                    relative_is_win = true;
                    fconfig.bufpos.lnum = -1;
                }
            } else if !config.external {
                if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL)
                    || has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT)
                {
                    is_split = true;
                    fconfig.external = false;
                } else if wp.is_null() {
                    api_err_required(err, c"'relative' or 'external' when creating a float");
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL) && !is_split {
                api_err_conflict(err, c"vertical", c"floating windows");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT) && !is_split {
                api_err_conflict(err, c"split", c"floating windows");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT) {
                if !is_split {
                    api_err_conflict(err, c"split", c"floating windows");
                    break 'fail;
                }
                if !parse_config_split(config.split, &mut fconfig.split) {
                    api_err_invalid(err, c"split", config.split.data, 0, true);
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ANCHOR)
                && !parse_float_anchor(config.anchor, &mut fconfig.anchor)
            {
                api_err_invalid(err, c"anchor", config.anchor.data, 0, true);
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ROW) {
                if !has_relative || is_split {
                    generate_api_error(wp.as_ref(), c"row", err);
                    break 'fail;
                }
                fconfig.row = config.row;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_COL) {
                if !has_relative || is_split {
                    generate_api_error(wp.as_ref(), c"col", err);
                    break 'fail;
                }
                fconfig.col = config.col;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BUFPOS) {
                if !has_relative || is_split {
                    generate_api_error(wp.as_ref(), c"bufpos", err);
                    break 'fail;
                } else {
                    if !parse_float_bufpos(config.bufpos, &mut fconfig.bufpos) {
                        api_err_exp(err, c"bufpos", c"[row, col] array", ptr::null());
                        break 'fail;
                    }

                    if !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ROW) {
                        fconfig.row = if (fconfig.anchor & K_FLOAT_ANCHOR_SOUTH) != 0 {
                            0.0
                        } else {
                            1.0
                        };
                    }
                    if !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_COL) {
                        fconfig.col = 0.0;
                    }
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIDTH) {
                if config.width <= 0 {
                    api_err_exp(err, c"width", c"positive Integer", ptr::null());
                    break 'fail;
                }
                fconfig.width = config.width as c_int;
            } else if !reconf && !is_split {
                api_err_required(err, c"width");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_HEIGHT) {
                if config.height <= 0 {
                    api_err_exp(err, c"height", c"positive Integer", ptr::null());
                    break 'fail;
                }
                fconfig.height = config.height as c_int;
            } else if !reconf && !is_split {
                api_err_required(err, c"height");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL) {
                fconfig.external = config.external;
                if has_relative && fconfig.external {
                    api_err_conflict(err, c"relative", c"external");
                    break 'fail;
                }
                if fconfig.external && !ui_has(K_UIMULTIGRID) {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"UI doesn't support external windows",
                    );
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN) && fconfig.external {
                api_err_conflict(err, c"win", c"external window");
                break 'fail;
            }

            if relative_is_win
                || (has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN)
                    && !is_split
                    && !wp.is_null()
                    && (*wp).w_floating
                    && fconfig.relative == K_FLOAT_RELATIVE_WINDOW)
            {
                let Some(target_win) =
                    crate::api::private::helpers::find_window_by_handle_mut(config.win, err)
                else {
                    break 'fail;
                };
                if (&raw mut *target_win) == wp {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"floating window cannot be relative to itself",
                    );
                    break 'fail;
                }
                fconfig.window = target_win.handle;
            } else {
                if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN) {
                    if !is_split && !has_relative && (wp.is_null() || !(*wp).w_floating) {
                        api_err_required(
                            err,
                            c"non-float with 'win' requires 'split' or 'vertical'",
                        );
                        break 'fail;
                    }
                    fconfig.window = config.win;
                }
                if fconfig.window == 0 {
                    fconfig.window = curwin_mut().handle;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FOCUSABLE) {
                fconfig.focusable = config.focusable;
                fconfig.mouse = config.focusable;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_MOUSE) {
                fconfig.mouse = config.mouse;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ZINDEX) {
                if !will_float {
                    api_err_conflict(err, c"zindex", c"non-float window");
                    break 'fail;
                }
                if config.zindex <= 0 {
                    api_err_exp(err, c"zindex", c"positive Integer", ptr::null());
                    break 'fail;
                }
                fconfig.zindex = config.zindex as c_int;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_TITLE) {
                if !will_float {
                    api_err_conflict(err, c"title", c"non-float window");
                    break 'fail;
                }

                parse_bordertext(config.title, K_BORDER_TEXT_TITLE, fconfig, err);
                if error_set(err) {
                    break 'fail;
                }

                if !parse_bordertext_pos(
                    wp.as_ref(),
                    config.title_pos,
                    K_BORDER_TEXT_TITLE,
                    fconfig,
                    err,
                ) {
                    break 'fail;
                }
            } else if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_TITLE_POS) {
                api_err_required(err, c"'title' requires 'title_pos'");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FOOTER) {
                if !will_float {
                    api_err_conflict(err, c"footer", c"non-float window");
                    break 'fail;
                }

                parse_bordertext(config.footer, K_BORDER_TEXT_FOOTER, fconfig, err);
                if error_set(err) {
                    break 'fail;
                }

                if !parse_bordertext_pos(
                    wp.as_ref(),
                    config.footer_pos,
                    K_BORDER_TEXT_FOOTER,
                    fconfig,
                    err,
                ) {
                    break 'fail;
                }
            } else if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FOOTER_POS) {
                api_err_required(err, c"'footer' requires 'footer_pos'");
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BORDER) {
                if !will_float {
                    api_err_conflict(err, c"border", c"non-float window");
                    break 'fail;
                }
                let border_style = config.border;
                if border_style.obj_type != ObjectType::ObjectTypeNil {
                    parse_border_style(border_style, Some(&mut *fconfig), err);
                    if error_set(err) {
                        break 'fail;
                    }
                }
            } else if !crate::memory::cstr_is_empty(opt_vars().p_winborder)
                && (wp.is_null() || !(*wp).w_floating)
                && !parse_winborder(Some(&mut *fconfig), opt_vars().p_winborder, err)
            {
                break 'fail;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_STYLE) {
                if crate::memory::cstr_is_empty(config.style.data) {
                    fconfig.style = K_WIN_STYLE_UNUSED;
                } else if crate::memory::cstr_as_opt_bytes(config.style.data)
                    .is_some_and(|b| b.eq_ignore_ascii_case(b"minimal"))
                {
                    fconfig.style = K_WIN_STYLE_MINIMAL;
                } else {
                    api_err_invalid(err, c"style", config.style.data, 0, true);
                    break 'fail;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_NOAUTOCMD) {
                if !wp.is_null() && config.noautocmd != fconfig.noautocmd {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"'noautocmd' cannot be changed on existing window",
                    );
                    break 'fail;
                }
                fconfig.noautocmd = config.noautocmd;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FIXED) {
                fconfig.fixed = config.fixed;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_HIDE) {
                if !will_float && config.hide {
                    api_err_conflict(err, c"hide", c"non-float window");
                    break 'fail;
                }
                if fconfig.hide && !config.hide {
                    redraw_later(wp.as_mut(), UPD_NOT_VALID);
                }
                fconfig.hide = config.hide;
            }

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_CMDLINE_OFFSET) {
                fconfig._cmdline_offset = config._cmdline_offset as c_int;
            }

            return true;
        }

        merge_win_config(
            fconfig,
            if !wp.is_null() {
                (*wp).w_config.clone()
            } else {
                WIN_CONFIG_INIT
            },
        );
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_split_flags_computes_expected_bits() {
        assert_eq!(
            win_split_flags(K_WIN_SPLIT_ABOVE, false),
            WSP_HOR | WSP_ABOVE
        );
        assert_eq!(win_split_flags(K_WIN_SPLIT_BELOW, true), WSP_HOR | WSP_BOT);
        assert_eq!(win_split_flags(K_WIN_SPLIT_LEFT, true), WSP_VERT | WSP_TOP);
        assert_eq!(
            win_split_flags(K_WIN_SPLIT_RIGHT, false),
            WSP_VERT | WSP_BELOW
        );
    }
}
