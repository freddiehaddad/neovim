//! Window configuration API functions (`src/nvim/api/win_config.c`).
#![forbid(unsafe_code)]

use crate::api::extmark::{parse_virt_text, virt_text_to_array};
use crate::api::private::helpers::{
    add_c, api_error, api_free_array, api_free_object, api_typename, array_obj, bytes_to_string,
    cstr_as_string, find_window_by_handle, has_key, integer_obj, object_to_hl_id, string_obj,
    try_wrap,
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
use crate::r#move::changed_window_setting;
use crate::msgpack_rpc::packer::Integer;
use crate::option::didset_window_options;
use crate::option_vars::*;
use crate::ui::ui_has;
use crate::ui_compositor::ui_comp_remove_grid;
use crate::window::{
    TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_NOENTER, WSP_TOP, WSP_VERT, WinConfig,
    WinInfoInfo, check_split_disallowed_err, clear_float_config, goto_tabpage_win, last_status,
    lastwin_nofloating, merge_win_config, one_window, win_append, win_comp_pos, win_find_tabpage,
    win_goto, win_locked, win_remove, win_set_buf, win_setheight_win, win_setwidth_win,
    win_split_ins, win_valid, win_valid_any_tab, window_layout_locked_err, winframe_find_altwin,
    winframe_remove, winframe_restore,
};
use crate::winfloat::{
    win_config_float, win_float_find_altwin, win_new_float, win_set_minimal_style,
};
use std::ffi::CStr;

pub type FloatAnchor = i32;
pub type FloatRelative = i32;
pub type WinSplit = i32;
pub type WinStyle = i32;
pub type AlignTextPos = i32;
pub type BorderTextType = i32;
pub type UIExtension = i32;

pub(crate) const K_UIMULTIGRID: UIExtension = 6;
pub(crate) const K_ZINDEX_FLOAT_DEFAULT: i32 = 50;

pub(crate) const FR_COL: u8 = 2;

pub(crate) const K_CTX_NO_DISPLAY: i32 = 4;
pub(crate) const K_CTX_NO_EVENTS: i32 = 8;

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

#[derive(Clone, Default)]
pub struct KeyDictWinConfig {
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
    _cmdline_offset: i32::MAX,
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
    config: &mut KeyDictWinConfig,
) -> Result<Window, Error> {
    let b = crate::api::private::helpers::find_buffer_by_handle(buf)?;

    let mask = config.is_set_win_config;
    let is_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT)
        || has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
    let mut fconfig = WIN_CONFIG_INIT;
    parse_win_config(None, &mut *config, &mut fconfig, false, !is_split)?;

    if fconfig.noautocmd {
        block_autocmds();
    }

    let res = (|| -> Result<Window, Error> {
        let mut wp = crate::globals::WinPtr::NULL;
        let mut tp = g_st().curtab.get().clone();
        assert!(!(curwin()).is_null());
        let mut parent = if config.win == 0 {
            curwin()
        } else {
            crate::globals::WinPtr::NULL
        };
        if config.win > 0 {
            parent = find_window_by_handle(fconfig.window)?;
            if is_split && parent.w_floating.get() {
                return Err(api_error!(
                    ErrorType::ErrorTypeException,
                    c"Cannot split a floating window",
                ));
            }
            tp = crate::globals::TabpagePtr::from(win_find_tabpage(parent.borrow()));
        }
        let mut split_res = Ok(());
        if is_split {
            let check_win = if !parent.is_null() {
                parent.clone()
            } else {
                curwin()
            };
            check_split_disallowed_err(check_win.opt())?;

            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL)
                && !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT)
            {
                if config.vertical {
                    fconfig.split = if opt_vars().p_spr.get() {
                        K_WIN_SPLIT_RIGHT
                    } else {
                        K_WIN_SPLIT_LEFT
                    };
                } else {
                    fconfig.split = if opt_vars().p_sb.get() {
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

            split_res = try_wrap(|| {
                if parent.is_null() || parent == curwin() {
                    wp = crate::globals::WinPtr::from(win_split_ins(size, flags, None, 0, None));
                } else {
                    let mut switchwin: CtxSwitch = CtxSwitch::new();
                    let result = ctx_switch(
                        &mut switchwin,
                        parent.opt(),
                        tp.opt(),
                        None,
                        K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
                    );
                    assert!(result);
                    wp = crate::globals::WinPtr::from(win_split_ins(size, flags, None, 0, None));
                    ctx_restore(&mut switchwin);
                }
            });
            if !wp.is_null() {
                *wp.w_config.borrow_mut() = fconfig.clone();
                if size > 0 {
                    if (flags & WSP_VERT) != 0 && wp.w_width.get() != size {
                        win_setwidth_win(size, wp.borrow(), true);
                    } else if (flags & WSP_VERT) == 0 && wp.w_height.get() != size {
                        win_setheight_win(size, wp.borrow(), true);
                    }
                }
            }
        } else {
            if curwin().buf().b_locked_split.get() != 0 {
                return Err(api_error!(
                    ErrorType::ErrorTypeException,
                    c"E1159: Cannot open a float when closing the buffer",
                ));
            }
            wp = win_new_float(None, false, fconfig.clone())?;
        }
        if wp.is_null() {
            split_res?;
            return Err(api_error!(
                ErrorType::ErrorTypeException,
                c"Failed to create window",
            ));
        }

        if fconfig._cmdline_offset < i32::MAX {
            g_st().cmdline_win.set(wp.clone());
        }

        let mut bufref = Bufref {
            br_buf: crate::globals::BufPtr::NULL,
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, Some(&b));
        if !fconfig.noautocmd {
            let mut switchwin: CtxSwitch = CtxSwitch::new();
            let result = ctx_switch(&mut switchwin, wp.opt(), tp.opt(), None, K_CTX_NO_DISPLAY);
            assert!(result);
            if apply_autocmds_win(
                EVENT_WINNEW,
                None,
                None,
                false,
                curbuf_opt().as_deref(),
                wp.opt(),
            ) {
                tp = crate::globals::TabpagePtr::from(win_find_tabpage(wp.borrow()));
            }
            ctx_restore(&mut switchwin);
        }
        if !tp.is_null() && enter {
            goto_tabpage_win(tp.borrow(), wp.borrow());
            tp = crate::globals::TabpagePtr::from(win_find_tabpage(wp.borrow()));
        }
        let mut set_buf_res = Ok(());
        if !tp.is_null() && bufref_valid(Some(&bufref)) && b != wp.w_buffer.get() {
            let au_no_enter_leave = curwin() != wp && !fconfig.noautocmd;
            if au_no_enter_leave {
                g_st()
                    .autocmd_no_enter
                    .set(g_st().autocmd_no_enter.get() + (1));
                g_st()
                    .autocmd_no_leave
                    .set(g_st().autocmd_no_leave.get() + (1));
            }
            set_buf_res = win_set_buf(wp.borrow(), &b);
            if !fconfig.noautocmd {
                tp = crate::globals::TabpagePtr::from(win_find_tabpage(wp.borrow()));
            }
            if au_no_enter_leave {
                g_st()
                    .autocmd_no_enter
                    .set(g_st().autocmd_no_enter.get() - (1));
                g_st()
                    .autocmd_no_leave
                    .set(g_st().autocmd_no_leave.get() - (1));
            }
        }
        if tp.is_null() {
            return Err(api_error!(
                ErrorType::ErrorTypeException,
                c"Window was closed immediately",
            ));
        }

        if fconfig.style == K_WIN_STYLE_MINIMAL {
            win_set_minimal_style(wp.borrow());
            didset_window_options(wp.borrow(), true);
            changed_window_setting(wp.borrow());
        }
        set_buf_res?;
        split_res?;
        Ok(wp.handle)
    })();

    if fconfig.noautocmd {
        unblock_autocmds();
    }
    res
}

fn win_split_dir(win: &WinInfoInfo) -> WinSplit {
    if win.w_frame.get().is_null() || win.frame().fr_parent.get().is_null() {
        return K_WIN_SPLIT_LEFT;
    }

    let layout = win.frame().parent_opt().unwrap().fr_layout.get();
    if layout == FR_COL {
        if !win.frame().fr_next.get().is_null() {
            K_WIN_SPLIT_ABOVE
        } else {
            K_WIN_SPLIT_BELOW
        }
    } else if !win.frame().fr_next.get().is_null() {
        K_WIN_SPLIT_LEFT
    } else {
        K_WIN_SPLIT_RIGHT
    }
}

const fn win_split_flags(split: WinSplit, toplevel: bool) -> i32 {
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

fn win_can_move_tp(wp: &WinInfoInfo, tp: &TabPageInfo) -> Result<(), Error> {
    if one_window(
        wp,
        if g_st().curtab.get() == tp {
            None
        } else {
            Some(tp)
        },
    ) {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Cannot move last non-floating window",
        ));
    }
    if win_locked(wp) != 0 {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Cannot move window to another tabpage whilst in use",
        ));
    }
    window_layout_locked_err(crate::cmdexpand::Cmdidx::CmdSize)?;
    if g_st().textlock.get() != 0 || expr_map_locked() {
        return Err(api_error!(ErrorType::ErrorTypeException, c"%s", E_TEXTLOCK));
    }
    if is_ctx_win(wp) {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Cannot move autocmd window to another tabpage",
        ));
    }
    Ok(())
}

fn win_find_altwin(win: &WinInfoInfo, tp: &TabPageInfo) -> Option<crate::globals::WinPtr> {
    let tp_arg = if g_st().curtab.get() == tp {
        crate::globals::TabpagePtr::NULL
    } else {
        crate::globals::TabpagePtr::from(tp)
    };
    if win.w_floating.get() {
        Some(win_float_find_altwin(win, tp_arg.opt()))
    } else {
        let mut dir: i32 = 0;
        winframe_find_altwin(win, &mut dir, tp_arg.opt(), None)
    }
}

fn win_config_split(
    win: &WinInfoInfo,
    config: &KeyDictWinConfig,
    fconfig: &mut WinConfig,
) -> Result<(), Error> {
    let was_split = !win.w_floating.get();
    let mask = config.is_set_win_config;
    let has_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT);
    let has_vertical = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
    let old_split = win_split_dir(win);
    if has_vertical && !has_split {
        if config.vertical {
            fconfig.split = if old_split == K_WIN_SPLIT_RIGHT || opt_vars().p_spr.get() {
                K_WIN_SPLIT_RIGHT
            } else {
                K_WIN_SPLIT_LEFT
            };
        } else {
            fconfig.split = if old_split == K_WIN_SPLIT_BELOW || opt_vars().p_sb.get() {
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

        let mut parent = crate::globals::WinPtr::NULL;
        let mut parent_tp = crate::globals::TabpagePtr::NULL;
        if config.win == 0 {
            parent = curwin();
            parent_tp = g_st().curtab.get().clone();
        } else if config.win > 0 {
            parent = find_window_by_handle(fconfig.window)?;
            parent_tp = crate::globals::TabpagePtr::from(win_find_tabpage(parent.borrow()));
        }

        let mut win_tp = crate::globals::TabpagePtr::from(win_find_tabpage(win));
        if !parent.is_null() {
            if parent.w_floating.get() {
                return Err(api_error!(
                    ErrorType::ErrorTypeException,
                    c"Cannot split a floating window",
                ));
            }
            if win_tp != parent_tp {
                win_can_move_tp(win, win_tp.borrow())?;
            }
        }

        check_split_disallowed_err(Some(win))?;

        let mut to_split_ok = false;
        let curwin_moving_tp = (std::ptr::from_ref(win).cast_mut()) == curwin()
            && !parent.is_null()
            && win_tp != parent_tp;

        let err: Error = 'restore_curwin: {
            if curwin_moving_tp {
                let altwin = crate::globals::WinPtr::from(win_find_altwin(win, win_tp.borrow()));
                assert!(!altwin.is_null());
                win_goto(altwin.borrow());

                if curwin() == (std::ptr::from_ref(win).cast_mut()) {
                    return Err(api_error!(
                        ErrorType::ErrorTypeException,
                        c"Failed to switch away from window %d",
                        win.handle,
                    ));
                }
                win_tp = win_find_tabpage(win).into();
                if win_tp.is_null() || !win_valid_any_tab(parent.opt()) {
                    break 'restore_curwin api_error!(
                        ErrorType::ErrorTypeException,
                        c"Windows to split were closed",
                    );
                }
                if was_split == win.w_floating.get() || parent.w_floating.get() {
                    break 'restore_curwin api_error!(
                        ErrorType::ErrorTypeException,
                        c"Floating state of windows to split changed",
                    );
                }
            }

            let mut dir: i32 = 0;
            let mut unflat_altfr = crate::globals::FramePtr::NULL;
            let altwin: crate::globals::WinPtr;

            if was_split {
                if win.frame().fr_parent.get().is_null() {
                    break 'restore_curwin api_error!(
                        ErrorType::ErrorTypeException,
                        c"Cannot move last non-floating window",
                    );
                } else if !parent.is_null() && parent.handle == win.handle {
                    let mut n_frames = 0;
                    for _fr in frame_iter_ptr(win.frame().fr_parent.get().fr_child.get()) {
                        n_frames += 1;
                    }

                    let mut neighbor = crate::globals::WinPtr::NULL;

                    if n_frames > 2 {
                        let frame = win.frame().fr_parent.get();
                        if !frame.fr_parent.get().is_null() {
                            neighbor = if fconfig.split == K_WIN_SPLIT_ABOVE
                                || fconfig.split == K_WIN_SPLIT_LEFT
                            {
                                win.w_next.get()
                            } else {
                                win.w_prev.get()
                            };
                        }
                        altwin = crate::globals::WinPtr::from(winframe_remove(
                            win,
                            &mut dir,
                            if win_tp == g_st().curtab.get() {
                                None
                            } else {
                                win_tp.opt()
                            },
                            Some(&mut unflat_altfr),
                        ));
                    } else if n_frames == 2 {
                        altwin = crate::globals::WinPtr::from(winframe_remove(
                            win,
                            &mut dir,
                            if win_tp == g_st().curtab.get() {
                                None
                            } else {
                                win_tp.opt()
                            },
                            Some(&mut unflat_altfr),
                        ));
                        neighbor = altwin.clone();
                    } else {
                        break 'restore_curwin api_error!(
                            ErrorType::ErrorTypeException,
                            c"Cannot split window into itself",
                        );
                    }
                    parent = neighbor;
                } else {
                    altwin = crate::globals::WinPtr::from(winframe_remove(
                        win,
                        &mut dir,
                        if win_tp == g_st().curtab.get() {
                            None
                        } else {
                            win_tp.opt()
                        },
                        Some(&mut unflat_altfr),
                    ));
                }
            } else {
                altwin = win_float_find_altwin(
                    win,
                    (win_tp != g_st().curtab.get()).then(|| win_tp.borrow()),
                );
            }

            win_remove(
                win,
                if win_tp == g_st().curtab.get() {
                    None
                } else {
                    win_tp.opt()
                },
            );
            if win_tp == g_st().curtab.get() {
                last_status(false);
                win_comp_pos();
            }

            let flags = win_split_flags(fconfig.split, parent.is_null()) | WSP_NOENTER;
            parent_tp = if !parent.is_null() {
                crate::globals::TabpagePtr::from(win_find_tabpage(parent.borrow()))
            } else {
                g_st().curtab.get().clone()
            };

            let win_ptr = crate::globals::WinPtr::from(win);
            let split_res = try_wrap(|| {
                let need_switch = !parent.is_null() && parent != curwin();
                let mut switchwin: CtxSwitch = CtxSwitch::new();
                if need_switch {
                    let result = ctx_switch(
                        &mut switchwin,
                        parent.opt(),
                        parent_tp.opt(),
                        None,
                        K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
                    );
                    assert!(result);
                }
                to_split_ok =
                    win_split_ins(0, flags, win_ptr.opt(), 0, unflat_altfr.opt()).is_some();
                if !to_split_ok {
                    win_append(
                        win_ptr.w_prev.get().opt(),
                        win_ptr.borrow(),
                        if win_tp == g_st().curtab.get() {
                            None
                        } else {
                            win_tp.opt()
                        },
                    );
                }
                if need_switch {
                    ctx_restore(&mut switchwin);
                }
            });
            if !to_split_ok {
                if was_split {
                    winframe_restore(win, dir, unflat_altfr.borrow());
                }
                break 'restore_curwin match split_res {
                    Err(e) => e,
                    Ok(()) => api_error!(
                        ErrorType::ErrorTypeException,
                        c"Failed to move window %d into split",
                        win.handle,
                    ),
                };
            }
            split_res?;

            if win_tp != parent_tp && win_tp.tp_curwin.get() == win {
                win_tp.tp_curwin.set(altwin);
            }
            break 'resize;
        };

        if curwin_moving_tp && win_valid(Some(win)) {
            win_goto(win);
        }
        return Err(err);
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
    merge_win_config(&mut win.w_config.borrow_mut(), fconfig.clone());
    Ok(())
}

fn win_config_float_tp(
    win: &WinInfoInfo,
    config: &KeyDictWinConfig,
    fconfig: &WinConfig,
) -> Result<(), Error> {
    let mut win_tp = crate::globals::TabpagePtr::from(win_find_tabpage(win));
    let mut parent = crate::globals::WinPtr::from(win);
    let mut parent_tp = win_tp.clone();
    if has_key(config.is_set_win_config, KEYSET_OPTIDX_WIN_CONFIG_WIN) {
        parent = find_window_by_handle(fconfig.window)?;
        parent_tp = win_find_tabpage(parent.borrow()).into();
    }

    let mut curwin_moving_tp = false;
    let mut altwin = crate::globals::WinPtr::NULL;

    let err: Error = 'restore_curwin: {
        if win_tp != parent_tp {
            win_can_move_tp(win, win_tp.borrow())?;
            altwin = crate::globals::WinPtr::from(win_find_altwin(win, win_tp.borrow()));
            assert!(!altwin.is_null());

            if curwin() == (std::ptr::from_ref(win).cast_mut()) {
                curwin_moving_tp = true;
                win_goto(altwin.borrow());

                if curwin() == (std::ptr::from_ref(win).cast_mut()) {
                    return Err(api_error!(
                        ErrorType::ErrorTypeException,
                        c"Failed to switch away from window %d",
                        win.handle,
                    ));
                }
                win_tp = win_find_tabpage(win).into();
                parent_tp = win_find_tabpage(parent.borrow()).into();

                if win_tp.is_null() || parent_tp.is_null() {
                    break 'restore_curwin api_error!(
                        ErrorType::ErrorTypeException,
                        c"Target windows were closed",
                    );
                }
                if win_tp != parent_tp
                    && let Err(e) = win_can_move_tp(win, win_tp.borrow())
                {
                    break 'restore_curwin e;
                }
                altwin = crate::globals::WinPtr::from(win_find_altwin(win, win_tp.borrow()));
                assert!(!altwin.is_null());
            }
        }

        if !win.w_floating.get() {
            if let Err(e) = win_new_float(Some(win), false, fconfig.clone()) {
                break 'restore_curwin e;
            }
            redraw_later(Some(win), UPD_NOT_VALID);
        }

        if win_tp != parent_tp {
            win_remove(
                win,
                if win_tp == g_st().curtab.get() {
                    None
                } else {
                    win_tp.opt()
                },
            );
            let append_tp = if parent_tp == g_st().curtab.get() {
                crate::globals::TabpagePtr::NULL
            } else {
                parent_tp
            };
            win_append(
                Some(&lastwin_nofloating(append_tp.opt())),
                win,
                append_tp.opt(),
            );

            if win_tp != g_st().curtab.get() && win_tp.tp_curwin.get() == win {
                win_tp.tp_curwin.set(altwin);
            }

            ui_comp_remove_grid(&win.w_grid_alloc);

            redraw_later(Some(win), UPD_NOT_VALID);
            set_must_redraw(UPD_NOT_VALID);
        }

        win_config_float(win, fconfig.clone());
        return Ok(());
    };

    if curwin_moving_tp && win_valid(Some(win)) {
        win_goto(win);
    }
    Err(err)
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
pub fn nvim_win_set_config(win: Window, config: &mut KeyDictWinConfig) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;

    let was_split = !w.w_floating.get();
    let mask = config.is_set_win_config;
    let has_split = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT);
    let has_vertical = has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL);
    let old_style = w.w_config.borrow().style;
    let mut fconfig = w.w_config.borrow().clone();

    let to_split = config.relative.size == 0
        && !(has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL) && config.external)
        && (has_split || has_vertical || was_split);
    let will_float = (!was_split && !to_split) || config.relative.size > 0 || config.external;

    parse_win_config(
        Some(&w),
        &mut *config,
        &mut fconfig,
        !was_split || to_split,
        will_float,
    )?;

    if to_split {
        win_config_split(&w, &*config, &mut fconfig)?;
    } else {
        win_config_float_tp(&w, &*config, &fconfig)?;
    }

    if fconfig.style == K_WIN_STYLE_MINIMAL && old_style != fconfig.style {
        win_set_minimal_style(&w);
        didset_window_options(&w, true);
        changed_window_setting(&w);
    }
    if fconfig._cmdline_offset < i32::MAX {
        g_st().cmdline_win.set(w);
    } else if g_st().cmdline_win.get() == w && fconfig._cmdline_offset == i32::MAX {
        g_st().cmdline_win.set(crate::globals::WinPtr::NULL);
    }
    Ok(())
}

fn config_put_bordertext(
    config: &mut KeyDictWinConfig,
    fconfig: &WinConfig,
    bordertext_type: BorderTextType,
) {
    let (vt, align) = match bordertext_type {
        K_BORDER_TEXT_TITLE => (&fconfig.title_chunks, fconfig.title_pos),
        _ => (&fconfig.footer_chunks, fconfig.footer_pos),
    };

    let bordertext = virt_text_to_array(vt, true);

    let pos = match align {
        K_ALIGN_LEFT => c"left",
        K_ALIGN_CENTER => c"center",
        _ => c"right",
    };

    if bordertext_type == K_BORDER_TEXT_TITLE {
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_TITLE;
        config.title = array_obj(bordertext);
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_TITLE_POS;
        config.title_pos = cstr_as_string(Some(pos));
    } else {
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOOTER;
        config.footer = array_obj(bordertext);
        config.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOOTER_POS;
        config.footer_pos = cstr_as_string(Some(pos));
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
pub fn nvim_win_get_config(win: Window) -> Result<KeyDictWinConfig, Error> {
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

    let mut rv = KeyDictWinConfig::default();

    let wp = crate::api::private::helpers::find_window_by_handle(win)?;

    let config = &wp.w_config.borrow();

    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_FOCUSABLE;
    rv.focusable = config.focusable;
    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL;
    rv.external = config.external;
    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HIDE;
    rv.hide = config.hide;
    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_MOUSE;
    rv.mouse = config.mouse;
    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_STYLE;
    rv.style = cstr_as_string(Some(WIN_STYLE_STR[config.style as usize]));

    if wp.w_floating.get() {
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIDTH;
        rv.width = i64::from(config.width);
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HEIGHT;
        rv.height = i64::from(config.height);
        if !config.external {
            if config.relative == K_FLOAT_RELATIVE_WINDOW {
                rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIN;
                rv.win = config.window;
                if config.bufpos.lnum >= 0 {
                    let mut pos = Array::with_capacity(2);
                    add_c(&mut pos, integer_obj(i64::from(config.bufpos.lnum)));
                    add_c(&mut pos, integer_obj(i64::from(config.bufpos.col)));
                    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BUFPOS;
                    rv.bufpos = pos;
                }
            }
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ANCHOR;
            rv.anchor = cstr_as_string(Some(FLOAT_ANCHOR_STR[config.anchor as usize]));
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ROW;
            rv.row = config.row;
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_COL;
            rv.col = config.col;
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_ZINDEX;
            rv.zindex = i64::from(config.zindex);
        }
        if config.border {
            let mut border = Array::with_capacity(8);
            for i in 0..8 {
                let bchars = &config.border_chars[i];
                let blen = bchars
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(MAX_SCHAR_SIZE);
                let s = NvimString::from_bytes(&bchars[..blen]);
                let hi_id = config.border_hl_ids[i];
                let hi_name = syn_id2name(hi_id);
                if !hi_name.to_bytes().is_empty() {
                    let mut tuple = Array::with_capacity(2);
                    add_c(&mut tuple, string_obj(s));
                    add_c(
                        &mut tuple,
                        crate::api::private::helpers::cstr_to_nonnull_obj(Some(&hi_name)),
                    );
                    add_c(&mut border, array_obj(tuple));
                } else {
                    add_c(&mut border, string_obj(s));
                }
            }
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BORDER;
            rv.border = array_obj(border);
            if config.title {
                config_put_bordertext(&mut rv, config, K_BORDER_TEXT_TITLE);
            }
            if config.footer {
                config_put_bordertext(&mut rv, config, K_BORDER_TEXT_FOOTER);
            }
        } else {
            rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_BORDER;
            rv.border = string_obj(cstr_as_string(Some(c"none")));
        }
    } else if !config.external {
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_WIDTH;
        rv.width = i64::from(wp.w_width.get());
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_HEIGHT;
        rv.height = i64::from(wp.w_height.get());
        let split = win_split_dir(&wp);
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_SPLIT;
        rv.split = cstr_as_string(Some(WIN_SPLIT_STR[split as usize]));
    }

    let rel = if wp.w_floating.get() && !config.external {
        FLOAT_RELATIVE_STR[config.relative as usize]
    } else {
        c""
    };
    rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_RELATIVE;
    rv.relative = cstr_as_string(Some(rel));
    if config._cmdline_offset < i32::MAX {
        rv.is_set_win_config |= 1u64 << KEYSET_OPTIDX_WIN_CONFIG_CMDLINE_OFFSET;
        rv._cmdline_offset = i64::from(config._cmdline_offset);
    }

    Ok(rv)
}

fn parse_float_anchor(anchor: NvimString) -> Option<FloatAnchor> {
    let bytes = anchor.cstr_bytes();
    if bytes.eq_ignore_ascii_case(b"NW") {
        Some(0)
    } else if bytes.eq_ignore_ascii_case(b"NE") {
        Some(K_FLOAT_ANCHOR_EAST)
    } else if bytes.eq_ignore_ascii_case(b"SW") {
        Some(K_FLOAT_ANCHOR_SOUTH)
    } else if bytes.eq_ignore_ascii_case(b"SE") {
        Some(K_FLOAT_ANCHOR_SOUTH | K_FLOAT_ANCHOR_EAST)
    } else {
        None
    }
}

fn parse_float_relative(relative: NvimString) -> Option<FloatRelative> {
    let bytes = relative.cstr_bytes();
    if bytes.eq_ignore_ascii_case(b"editor") {
        Some(K_FLOAT_RELATIVE_EDITOR)
    } else if bytes.eq_ignore_ascii_case(b"win") {
        Some(K_FLOAT_RELATIVE_WINDOW)
    } else if bytes.eq_ignore_ascii_case(b"cursor") {
        Some(K_FLOAT_RELATIVE_CURSOR)
    } else if bytes.eq_ignore_ascii_case(b"mouse") {
        Some(K_FLOAT_RELATIVE_MOUSE)
    } else if bytes.eq_ignore_ascii_case(b"tabline") {
        Some(K_FLOAT_RELATIVE_TABLINE)
    } else if bytes.eq_ignore_ascii_case(b"laststatus") {
        Some(K_FLOAT_RELATIVE_LASTSTATUS)
    } else {
        None
    }
}

fn parse_config_split(split: NvimString) -> Option<WinSplit> {
    let bytes = split.cstr_bytes();
    if bytes.eq_ignore_ascii_case(b"left") {
        Some(K_WIN_SPLIT_LEFT)
    } else if bytes.eq_ignore_ascii_case(b"right") {
        Some(K_WIN_SPLIT_RIGHT)
    } else if bytes.eq_ignore_ascii_case(b"above") {
        Some(K_WIN_SPLIT_ABOVE)
    } else if bytes.eq_ignore_ascii_case(b"below") {
        Some(K_WIN_SPLIT_BELOW)
    } else {
        None
    }
}

fn parse_float_bufpos(bufpos: Array) -> Option<Lpos> {
    if bufpos.size != 2
        || bufpos[0].obj_type != ObjectType::ObjectTypeInteger
        || bufpos[1].obj_type != ObjectType::ObjectTypeInteger
    {
        return None;
    }
    Some(Lpos {
        lnum: bufpos[0].data.integer() as i32,
        col: bufpos[1].data.integer() as i32,
    })
}

// DLLEXPORT void parse_bordertext(Object bordertext, BorderTextType bordertext_type,
//                                 WinConfig *fconfig, Error *err);
pub fn parse_bordertext(
    bordertext: Object,
    bordertext_type: BorderTextType,
    fconfig: &mut WinConfig,
) -> Result<(), Error> {
    if bordertext.obj_type != ObjectType::ObjectTypeString
        && bordertext.obj_type != ObjectType::ObjectTypeArray
    {
        return Err(api_err_exp(
            c"title/footer",
            c"String or Array",
            Some(api_typename(bordertext.obj_type)),
        ));
    }

    if bordertext.obj_type == ObjectType::ObjectTypeArray && bordertext.data.array().size == 0 {
        return Err(api_err_exp(c"title/footer", c"non-empty Array", None));
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
            return Ok(());
        }
        let text_cstr = transstr(bordertext.data.string().cstr_bytes(), true);
        *width = mb_string2cells(text_cstr.as_bytes()) as i32;
        chunks.clear();
        chunks.push(VirtTextChunk {
            text: Some(crate::option_vars::RcCStr::from_cstring(text_cstr)),
            hl_id: -1,
        });
    } else {
        *chunks = parse_virt_text(bordertext.data.array(), Some(width), true)?;
    }

    *is_present = true;
    Ok(())
}

fn parse_bordertext_pos(
    wp: Option<&WinInfoInfo>,
    bordertext_pos: NvimString,
    bordertext_type: BorderTextType,
    fconfig: &mut WinConfig,
) -> Result<(), Error> {
    let align = if bordertext_type == K_BORDER_TEXT_TITLE {
        &mut fconfig.title_pos
    } else {
        &mut fconfig.footer_pos
    };

    if bordertext_pos.size == 0 {
        if wp.is_none() {
            *align = K_ALIGN_LEFT;
        }
        return Ok(());
    }

    match bordertext_pos.cstr_bytes() {
        b"left" => *align = K_ALIGN_LEFT,
        b"center" => *align = K_ALIGN_CENTER,
        b"right" => *align = K_ALIGN_RIGHT,
        _ => {
            return Err(api_err_invalid(
                if bordertext_type == K_BORDER_TEXT_TITLE {
                    c"title_pos"
                } else {
                    c"footer_pos"
                },
                bordertext_pos.as_opt_cstr(),
                0,
                true,
            ));
        }
    }
    Ok(())
}

// DLLEXPORT void parse_border_style(Object style, WinConfig *fconfig, Error *err);
pub fn parse_border_style(style: Object, fconfig: Option<&mut WinConfig>) -> Result<(), Error> {
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
            return Err(api_err_exp(c"border", c"1, 2, 4, or 8 chars", None));
        }
        for i in 0..size {
            let iytem = &arr[i];
            let string: NvimString;
            let mut hl_id: i32 = 0;
            if iytem.obj_type == ObjectType::ObjectTypeArray {
                let iarr = iytem.data.array();
                if iarr.size == 0 || iarr.size > 2 {
                    return Err(api_err_exp(c"border", c"1 or 2-item Array", None));
                }
                let first = &iarr[0];
                if first.obj_type != ObjectType::ObjectTypeString {
                    return Err(api_err_exp(c"border", c"Array of Strings", None));
                }
                string = first.data.string();
                if iarr.size == 2 {
                    hl_id = object_to_hl_id(iarr[1].clone(), c"border char highlight")?;
                }
            } else if iytem.obj_type == ObjectType::ObjectTypeString {
                string = iytem.data.string();
            } else {
                return Err(api_err_exp(
                    c"border",
                    c"String or Array",
                    Some(api_typename(iytem.obj_type)),
                ));
            }
            if string.size != 0 && mb_string2cells_len(string.as_bytes()) > 1 {
                return Err(api_err_exp(c"border", c"only one-cell chars", None));
            }
            let len = string.size.min(MAX_SCHAR_SIZE - 1);
            if len > 0 {
                chars[i][..len].copy_from_slice(&string.as_bytes()[..len]);
            }
            chars[i][len] = 0;
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
            return Err(api_err_exp(
                c"border",
                c"corner char between edge chars",
                None,
            ));
        }
    } else if style.obj_type == ObjectType::ObjectTypeString {
        let str_val = style.data.string();
        let str_bytes = str_val.cstr_bytes();
        if str_val.size == 0 || str_bytes == b"none" {
            fconfig.border = false;
            fconfig.title = false;
            fconfig.footer = false;
            return Ok(());
        }
        for i in 0..6 {
            let def_name = OPT_WINBORDER_VALUES[i + 1];
            if !str_val.data.is_null() && str_bytes == def_name.to_bytes() {
                *chars = [[0; MAX_SCHAR_SIZE]; 8];
                for (slot, piece) in chars.iter_mut().zip(DEFAULTS_CHARS[i].iter()) {
                    let bytes = piece.as_bytes();
                    slot[..bytes.len()].copy_from_slice(bytes);
                }
                *hl_ids = [0; 8];
                if DEFAULTS_SHADOW[i] {
                    let hl_blend = syn_check_group(b"FloatShadow");
                    let hl_through = syn_check_group(b"FloatShadowThrough");
                    hl_ids[2] = hl_through;
                    hl_ids[3] = hl_blend;
                    hl_ids[4] = hl_blend;
                    hl_ids[5] = hl_blend;
                    hl_ids[6] = hl_through;
                }
                return Ok(());
            }
        }
        return Err(api_err_invalid(c"border", str_val.as_opt_cstr(), 0, true));
    }
    Ok(())
}

fn generate_api_error(wp: Option<&WinInfoInfo>, attribute: &std::ffi::CStr) -> Error {
    if let Some(wp) = wp
        && wp.w_floating.get()
    {
        api_error!(
            ErrorType::ErrorTypeValidation,
            c"Required: 'relative' when reconfiguring floating window %d",
            wp.handle,
        )
    } else {
        api_err_conflict(attribute, c"non-float window")
    }
}

/// Parses a border style name or custom (comma-separated) style.
// DLLEXPORT bool parse_winborder(WinConfig *fconfig, char *border_opt, Error *err);
pub fn parse_winborder(fconfig: Option<&mut WinConfig>, opt_bytes: &[u8]) -> Result<(), Error> {
    if fconfig.is_none() {
        return Err(Error::new());
    }
    let style = if opt_bytes.contains(&b',') {
        let mut border_chars = Array::new();
        for part in opt_bytes.split(|&b| b == b',') {
            if border_chars.size >= 8 {
                api_free_array(border_chars);
                return Err(Error::new());
            }
            border_chars.push(string_obj(bytes_to_string(part)));
        }
        if border_chars.size != 8 {
            api_free_array(border_chars);
            return Err(Error::new());
        }
        array_obj(border_chars)
    } else {
        string_obj(bytes_to_string(opt_bytes))
    };

    let res = parse_border_style(style.clone(), (fconfig.is_some()).then(|| fconfig.unwrap()));
    api_free_object(style);
    res
}

fn parse_win_config(
    wp: Option<&WinInfoInfo>,
    config: &mut KeyDictWinConfig,
    fconfig: &mut WinConfig,
    reconf: bool,
    will_float: bool,
) -> Result<(), Error> {
    let mut has_relative = false;
    let mut relative_is_win = false;
    let mut is_split = false;
    let mask = config.is_set_win_config;

    let err: Error = 'fail: {
        if config.relative.size > 0 {
            let Some(rel) = parse_float_relative(config.relative.clone()) else {
                break 'fail api_err_invalid(c"relative", config.relative.as_opt_cstr(), 0, true);
            };
            fconfig.relative = rel;

            if !(has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ROW)
                && has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_COL))
                && !has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BUFPOS)
            {
                break 'fail api_err_required(c"'relative' requires 'row'/'col' or 'bufpos'");
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
            } else if wp.is_none() {
                break 'fail api_err_required(c"'relative' or 'external' when creating a float");
            }
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_VERTICAL) && !is_split {
            break 'fail api_err_conflict(c"vertical", c"floating windows");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT) && !is_split {
            break 'fail api_err_conflict(c"split", c"floating windows");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_SPLIT) {
            if !is_split {
                break 'fail api_err_conflict(c"split", c"floating windows");
            }
            let Some(split) = parse_config_split(config.split.clone()) else {
                break 'fail api_err_invalid(c"split", config.split.as_opt_cstr(), 0, true);
            };
            fconfig.split = split;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ANCHOR) {
            let Some(anchor) = parse_float_anchor(config.anchor.clone()) else {
                break 'fail api_err_invalid(c"anchor", config.anchor.as_opt_cstr(), 0, true);
            };
            fconfig.anchor = anchor;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_ROW) {
            if !has_relative || is_split {
                break 'fail generate_api_error(wp, c"row");
            }
            fconfig.row = config.row;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_COL) {
            if !has_relative || is_split {
                break 'fail generate_api_error(wp, c"col");
            }
            fconfig.col = config.col;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BUFPOS) {
            if !has_relative || is_split {
                break 'fail generate_api_error(wp, c"bufpos");
            } else {
                let Some(bufpos) = parse_float_bufpos(config.bufpos.clone()) else {
                    break 'fail api_err_exp(c"bufpos", c"[row, col] array", None);
                };
                fconfig.bufpos = bufpos;

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
                break 'fail api_err_exp(c"width", c"positive Integer", None);
            }
            fconfig.width = config.width as i32;
        } else if !reconf && !is_split {
            break 'fail api_err_required(c"width");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_HEIGHT) {
            if config.height <= 0 {
                break 'fail api_err_exp(c"height", c"positive Integer", None);
            }
            fconfig.height = config.height as i32;
        } else if !reconf && !is_split {
            break 'fail api_err_required(c"height");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_EXTERNAL) {
            fconfig.external = config.external;
            if has_relative && fconfig.external {
                break 'fail api_err_conflict(c"relative", c"external");
            }
            if fconfig.external && !ui_has(K_UIMULTIGRID) {
                break 'fail api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"UI doesn't support external windows",
                );
            }
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN) && fconfig.external {
            break 'fail api_err_conflict(c"win", c"external window");
        }

        if relative_is_win
            || (has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN)
                && !is_split
                && wp.as_ref().is_some_and(|w| w.w_floating.get())
                && fconfig.relative == K_FLOAT_RELATIVE_WINDOW)
        {
            let target_win = match crate::api::private::helpers::find_window_by_handle(config.win) {
                Ok(w) => w,
                Err(e) => break 'fail e,
            };
            if wp.is_some_and(|w| target_win == w) {
                break 'fail api_error!(
                    ErrorType::ErrorTypeException,
                    c"floating window cannot be relative to itself",
                );
            }
            fconfig.window = target_win.handle;
        } else {
            if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_WIN) {
                if !is_split && !has_relative && wp.as_ref().is_none_or(|w| !w.w_floating.get()) {
                    break 'fail api_err_required(
                        c"non-float with 'win' requires 'split' or 'vertical'",
                    );
                }
                fconfig.window = config.win;
            }
            if fconfig.window == 0 {
                fconfig.window = curwin().handle;
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
                break 'fail api_err_conflict(c"zindex", c"non-float window");
            }
            if config.zindex <= 0 {
                break 'fail api_err_exp(c"zindex", c"positive Integer", None);
            }
            fconfig.zindex = config.zindex as i32;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_TITLE) {
            if !will_float {
                break 'fail api_err_conflict(c"title", c"non-float window");
            }

            if let Err(e) = parse_bordertext(config.title.clone(), K_BORDER_TEXT_TITLE, fconfig) {
                break 'fail e;
            }

            if let Err(e) =
                parse_bordertext_pos(wp, config.title_pos.clone(), K_BORDER_TEXT_TITLE, fconfig)
            {
                break 'fail e;
            }
        } else if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_TITLE_POS) {
            break 'fail api_err_required(c"'title' requires 'title_pos'");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FOOTER) {
            if !will_float {
                break 'fail api_err_conflict(c"footer", c"non-float window");
            }

            if let Err(e) = parse_bordertext(config.footer.clone(), K_BORDER_TEXT_FOOTER, fconfig) {
                break 'fail e;
            }

            if let Err(e) =
                parse_bordertext_pos(wp, config.footer_pos.clone(), K_BORDER_TEXT_FOOTER, fconfig)
            {
                break 'fail e;
            }
        } else if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FOOTER_POS) {
            break 'fail api_err_required(c"'footer' requires 'footer_pos'");
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_BORDER) {
            if !will_float {
                break 'fail api_err_conflict(c"border", c"non-float window");
            }
            let border_style = config.border.clone();
            if border_style.obj_type != ObjectType::ObjectTypeNil
                && let Err(e) = parse_border_style(border_style, Some(&mut *fconfig))
            {
                break 'fail e;
            }
        } else if !opt_vars().p_winborder.is_empty()
            && wp.as_ref().is_none_or(|w| !w.w_floating.get())
            && let Err(e) = parse_winborder(Some(&mut *fconfig), &opt_vars().p_winborder.as_bytes())
        {
            break 'fail e;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_STYLE) {
            let style_bytes = config.style.cstr_bytes();
            if style_bytes.is_empty() {
                fconfig.style = K_WIN_STYLE_UNUSED;
            } else if style_bytes.eq_ignore_ascii_case(b"minimal") {
                fconfig.style = K_WIN_STYLE_MINIMAL;
            } else {
                break 'fail api_err_invalid(c"style", config.style.as_opt_cstr(), 0, true);
            }
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_NOAUTOCMD) {
            if wp.is_some() && config.noautocmd != fconfig.noautocmd {
                break 'fail api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"'noautocmd' cannot be changed on existing window",
                );
            }
            fconfig.noautocmd = config.noautocmd;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_FIXED) {
            fconfig.fixed = config.fixed;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_HIDE) {
            if !will_float && config.hide {
                break 'fail api_err_conflict(c"hide", c"non-float window");
            }
            if fconfig.hide && !config.hide {
                redraw_later(wp, UPD_NOT_VALID);
            }
            fconfig.hide = config.hide;
        }

        if has_key(mask, KEYSET_OPTIDX_WIN_CONFIG_CMDLINE_OFFSET) {
            fconfig._cmdline_offset = config._cmdline_offset as i32;
        }

        return Ok(());
    };

    merge_win_config(
        fconfig,
        if let Some(wp) = wp {
            wp.w_config.borrow().clone()
        } else {
            WIN_CONFIG_INIT
        },
    );
    Err(err)
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
