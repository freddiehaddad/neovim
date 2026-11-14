//! Window API functions (`src/nvim/api/window.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    OBJECT_NIL, add_c, api_error, dict_get_value, dict_set_var, find_window_by_handle, has_key,
    integer_obj, normalize_index, put_c, try_wrap, try_wrap_err,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType, api_err_conflict, api_err_exp,
    api_err_invalid, api_err_range, api_err_required,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{Colnr, Linenr, check_cursor_col};
use crate::drawscreen::{UPD_NOT_VALID, UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::vars::MAXCOL;
use crate::ex_docmd::ex_win_close;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{Boolean, LuaRef};
use crate::lua::executor::{K_RET_MULTI_STACK, nlua_call_ref};
use crate::message::emsg;
use crate::r#move::{update_topline, validate_cursor};
use crate::msgpack_rpc::packer::Integer;
use crate::plines::{win_get_fill, win_text_height};
use crate::strings::gettext;
use crate::window::{
    win_close, win_close_othertab, win_find_tabpage, win_get_tabwin, win_set_buf,
    win_setheight_win, win_setwidth_win,
};

use crate::api::win_config::{K_CTX_NO_DISPLAY, K_CTX_NO_EVENTS};

pub(crate) const K_CTX_KEEP_CWD: i32 = 1;
pub(crate) const K_CTX_VALIDATE: i32 = 16;

const KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_ROW: u64 = 1;
const KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_VCOL: u64 = 2;
const KEYSET_OPTIDX_WIN_TEXT_HEIGHT_START_ROW: u64 = 3;
const KEYSET_OPTIDX_WIN_TEXT_HEIGHT_MAX_HEIGHT: u64 = 4;
const KEYSET_OPTIDX_WIN_TEXT_HEIGHT_START_VCOL: u64 = 5;

const KEYSET_OPTIDX_WIN_RESIZE_ANCHOR: u64 = 1;

#[derive(Default)]
pub struct KeyDictWinTextHeight {
    pub is_set_win_text_height: OptionalKeys,
    pub start_row: Integer,
    pub end_row: Integer,
    pub start_vcol: Integer,
    pub end_vcol: Integer,
    pub max_height: Integer,
}

#[derive(Default)]
pub struct KeyDictWinResize {
    pub is_set_win_resize: OptionalKeys,
    pub anchor: NvimString,
}

/// Gets the current buffer in a window
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Buffer id
// DLLEXPORT Buffer nvim_win_get_buf(Window win, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_buf(win: Window) -> Result<Buffer, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    Ok(w.buf().b_fnum.get())
}

/// Sets the current buffer in a window.
///
/// Note: As a side-effect, this executes |BufEnter| and |BufLeave| autocommands.
/// @param win   |window-ID|, or 0 for current window
/// @param buf   Buffer id
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_buf(Window win, Buffer buf, Error *err)
//   FUNC_API_SINCE(5)
//   FUNC_API_TEXTLOCK;
pub fn nvim_win_set_buf(win: Window, buf: Buffer) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    let b = crate::api::private::helpers::find_buffer_by_handle(buf)?;
    win_set_buf(&w, &b)
}

/// Gets the (1,0)-indexed, buffer-relative cursor position for a given window
/// (different windows showing the same buffer have independent cursor
/// positions). |api-indexing|
///
/// @see |getcurpos()|
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return (row, col) tuple
// DLLEXPORT ArrayOf(Integer, 2) nvim_win_get_cursor(Window win, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_cursor(win: Window) -> Result<Array, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    let mut rv = Array::with_capacity(2);
    add_c(&mut rv, integer_obj(i64::from(w.w_cursor.get().lnum)));
    add_c(&mut rv, integer_obj(i64::from(w.w_cursor.get().col)));
    Ok(rv)
}

/// Sets the (1,0)-indexed cursor position (byte offset) in the window. |api-indexing|
/// This scrolls the window even if it is not the current one.
///
/// @param win   |window-ID|, or 0 for current window
/// @param pos      (row, col) tuple representing the new position
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_cursor(Window win, ArrayOf(Integer, 2) pos, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_set_cursor(win: Window, pos: Array) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;

    if pos.size != 2
        || pos[0].obj_type != ObjectType::ObjectTypeInteger
        || pos[1].obj_type != ObjectType::ObjectTypeInteger
    {
        return Err(api_err_exp(c"pos", c"[row, col] array", None));
    }

    let row = pos[0].data.integer();
    let col = pos[1].data.integer();

    if row <= 0 || row > i64::from(w.buf().b_ml.ml_line_count.get()) {
        return Err(api_err_range(c"cursor line"));
    }

    if !(0..=MAXCOL).contains(&col) {
        return Err(api_err_range(c"cursor column"));
    }

    w.w_cursor.set_lnum(row as Linenr);
    w.w_cursor.set_col(col as Colnr);
    w.w_cursor.set_coladd(0);
    check_cursor_col(&w);

    w.w_set_curswant.set(true);

    let mut switchwin: CtxSwitch = CtxSwitch::new();
    if ctx_switch(
        &mut switchwin,
        Some(&w),
        None,
        None,
        K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
    ) {
        update_topline(&curwin());
        validate_cursor(&curwin());
    }
    ctx_restore(&mut switchwin);

    redraw_later(Some(&w), UPD_VALID);
    w.w_redr_status.set(true);
    Ok(())
}

/// Gets the window height
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Height as a count of rows
// DLLEXPORT Integer nvim_win_get_height(Window win, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_height(win: Window) -> Result<Integer, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    Ok(i64::from(w.w_height.get()))
}

/// Gets the window width
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Width as a count of columns
// DLLEXPORT Integer nvim_win_get_width(Window win, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_width(win: Window) -> Result<Integer, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    Ok(i64::from(w.w_width.get()))
}

/// Gets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_win_get_var(Window win, String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_var(win: Window, name: NvimString) -> Result<Object, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    dict_get_value(Some(&*w.w_vars()), name)
}

/// Sets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_var(Window win, String name, Object value, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_set_var(win: Window, name: NvimString, value: Object) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    dict_set_var(&w.w_vars(), name, value, false, false)?;
    Ok(())
}

/// Removes a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_del_var(Window win, String name, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_del_var(win: Window, name: NvimString) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    dict_set_var(&w.w_vars(), name, OBJECT_NIL, true, false)?;
    Ok(())
}

/// Gets the window position in display cells. First position is zero.
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return (row, col) tuple with the window position
// DLLEXPORT ArrayOf(Integer, 2) nvim_win_get_position(Window win, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_position(win: Window) -> Result<Array, Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    let mut rv = Array::with_capacity(2);
    add_c(&mut rv, integer_obj(i64::from(w.w_winrow.get())));
    add_c(&mut rv, integer_obj(i64::from(w.w_wincol.get())));
    Ok(rv)
}

/// Gets the window tabpage
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Tabpage that contains the window
// DLLEXPORT Tabpage nvim_win_get_tabpage(Window win, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_tabpage(win: Window) -> Result<Tabpage, Error> {
    let w = find_window_by_handle(win)?;
    Ok(win_find_tabpage(w.borrow()).unwrap().handle)
}

/// Gets the window number
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Window number
// DLLEXPORT Integer nvim_win_get_number(Window win, Error *err)
//   FUNC_API_SINCE(1);
pub fn nvim_win_get_number(win: Window) -> Result<Integer, Error> {
    let w = find_window_by_handle(win)?;
    let mut tabnr: i32 = 0;
    let mut rv: i32 = 0;
    win_get_tabwin(w.handle, &mut tabnr, &mut rv);
    Ok(i64::from(rv))
}

/// Checks if a window is valid
///
/// @param win |window-ID|, or 0 for current window
/// @return true if the window is valid, false otherwise
// DLLEXPORT Boolean nvim_win_is_valid(Window win)
//   FUNC_API_SINCE(1);
pub fn nvim_win_is_valid(win: Window) -> Boolean {
    find_window_by_handle(win).is_ok()
}

/// Closes the window and hides the buffer it contains (like |:hide| with a |window-ID|; unrelated
/// to the `hide` flag of |nvim_open_win()|, |nvim_win_get_config()|).
///
/// Compare |:close| and |nvim_win_close()|, which close the buffer instead of hiding it.
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_hide(Window win, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_TEXTLOCK;
pub fn nvim_win_hide(win: Window) -> Result<(), Error> {
    let w = find_window_by_handle(win)?;

    let tabpage = crate::globals::TabpagePtr::from(win_find_tabpage(w.borrow()));
    try_wrap(|| {
        if is_ctx_win(w.borrow()) {
            emsg(gettext(E_AUTOCMD_CLOSE));
        } else if tabpage == g_st().curtab.get() {
            win_close(w.borrow(), false, false);
        } else {
            win_close_othertab(w.borrow(), 0, tabpage.borrow(), false);
        }
    })
}

/// Closes the window (like |:close| with a |window-ID|).
///
/// @param win   |window-ID|, or 0 for current window
/// @param force    Behave like `:close!` The last window of a buffer with
///                 unwritten changes can be closed. The buffer will become
///                 hidden, even if 'hidden' is not set.
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_close(Window win, Boolean force, Error *err)
//   FUNC_API_SINCE(6)
//   FUNC_API_TEXTLOCK;
pub fn nvim_win_close(win: Window, force: Boolean) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;

    let tabpage = crate::globals::TabpagePtr::from(win_find_tabpage(&w));
    try_wrap(|| {
        ex_win_close(
            force,
            &w,
            if tabpage == g_st().curtab.get() {
                None
            } else {
                tabpage.opt()
            },
        );
    })
}

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
// DLLEXPORT Object nvim_win_call(Window win, LuaRef fn, lua_State *lstate, Error *err)
//   FUNC_API_SINCE(7)
//   FUNC_API_LUA_ONLY;
pub fn nvim_win_call(win: Window, fn_ref: LuaRef) -> Result<Object, Error> {
    let w = find_window_by_handle(win)?;
    let tabpage = crate::globals::TabpagePtr::from(win_find_tabpage(w.borrow()));

    try_wrap_err(|| {
        let mut cs: CtxSwitch = CtxSwitch::new();
        let res = if ctx_switch(
            &mut cs,
            w.opt(),
            tabpage.opt(),
            None,
            K_CTX_NO_DISPLAY | K_CTX_KEEP_CWD | K_CTX_VALIDATE,
        ) {
            let args = Array::new();
            nlua_call_ref(fn_ref, None, args, K_RET_MULTI_STACK).map(|_| ())
        } else {
            Ok(())
        };
        ctx_restore(&mut cs);
        res
    })?;
    Ok(OBJECT_NIL)
}

/// Set highlight namespace for a window. This will use highlights defined with
/// |nvim_set_hl()| for this namespace, but fall back to global highlights (ns=0) when
/// missing.
///
/// This takes precedence over the 'winhighlight' option.
///
/// @param win
/// @param ns_id the namespace to use
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_hl_ns(Window win, Integer ns_id, Error *err)
//   FUNC_API_SINCE(10);
pub fn nvim_win_set_hl_ns(win: Window, ns_id: Integer) -> Result<(), Error> {
    let w = crate::api::private::helpers::find_window_by_handle(win)?;

    if ns_id < -1 {
        return Err(api_err_invalid(c"namespace", Some(c""), 0, true));
    }

    w.w_ns_hl.set(ns_id as i32);
    w.w_hl_needs_update.set(1);
    redraw_later(Some(&w), UPD_NOT_VALID);
    Ok(())
}

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
// DLLEXPORT DictAs(win_text_height_ret) nvim_win_text_height(Window win, Dict(win_text_height) *opts,
//                                                            Arena *arena, Error *err)
//   FUNC_API_SINCE(12);
pub fn nvim_win_text_height(win: Window, opts: &mut KeyDictWinTextHeight) -> Result<Dict, Error> {
    let mut rv = Dict::with_capacity(4);

    let w = crate::api::private::helpers::find_window_by_handle(win)?;
    let buf = w.buf();
    let line_count = buf.b_ml.ml_line_count.get();

    let mut start_lnum: Linenr = 1;
    let mut end_lnum: Linenr = line_count;
    let mut start_vcol: i64 = -1;
    let mut end_vcol: i64 = -1;

    let mut oob = false;
    let mask = opts.is_set_win_text_height;

    if has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_START_ROW) {
        start_lnum = normalize_index(&buf, opts.start_row, false, &mut oob) as Linenr;
    }

    if has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_ROW) {
        end_lnum = normalize_index(&buf, opts.end_row, false, &mut oob) as Linenr;
    }

    if oob {
        return Err(api_error!(
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"Line index out of bounds",
        ));
    }
    if start_lnum > end_lnum {
        return Err(api_error!(
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"'start_row' is higher than 'end_row'",
        ));
    }

    if has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_START_VCOL) {
        if !has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_START_ROW) {
            return Err(api_error!(
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"'start_vcol' specified without 'start_row'",
            ));
        }
        start_vcol = opts.start_vcol;
        if !(0..=MAXCOL).contains(&start_vcol) {
            return Err(api_err_range(c"start_vcol"));
        }
    }

    if has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_VCOL) {
        if !has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_ROW) {
            return Err(api_error!(
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"'end_vcol' specified without 'end_row'",
            ));
        }
        end_vcol = opts.end_vcol;
        if !(0..=MAXCOL).contains(&end_vcol) {
            return Err(api_err_range(c"end_vcol"));
        }
    }

    let mut max: i64 = i64::MAX;
    if has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_MAX_HEIGHT) {
        if opts.max_height <= 0 {
            return Err(api_err_range(c"max_height"));
        }
        max = opts.max_height;
    }

    if start_lnum == end_lnum && start_vcol >= 0 && end_vcol >= 0 && start_vcol > end_vcol {
        return Err(api_error!(
            ErrorType::ErrorTypeValidation,
            c"%s",
            c"'start_vcol' is higher than 'end_vcol'",
        ));
    }

    let mut fill: i64 = 0;
    let mut all = win_text_height(
        &w,
        start_lnum,
        start_vcol,
        &mut end_lnum,
        &mut end_vcol,
        Some(&mut fill),
        max,
        false,
    );
    if !has_key(mask, KEYSET_OPTIDX_WIN_TEXT_HEIGHT_END_ROW) {
        let end_fill = i64::from(win_get_fill(&w, line_count + 1));
        fill += end_fill;
        all += end_fill;
    }
    put_c(&mut rv, c"all", integer_obj(all));
    put_c(&mut rv, c"fill", integer_obj(fill));
    put_c(&mut rv, c"end_row", integer_obj(i64::from(end_lnum - 1)));
    put_c(&mut rv, c"end_vcol", integer_obj(end_vcol));
    Ok(rv)
}

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
// DLLEXPORT void nvim_win_resize(Window win, Integer width, Integer height, Dict(win_resize) *opts,
//                                Error *err)
//   FUNC_API_SINCE(15);
pub fn nvim_win_resize(
    win: Window,
    width: Integer,
    height: Integer,
    opts: &mut KeyDictWinResize,
) -> Result<(), Error> {
    let w = find_window_by_handle(win)?;

    if !(height >= 0 || height == -1) {
        return Err(api_err_exp(c"height", c"non-negative number or -1", None));
    }
    if !(width >= 0 || width == -1) {
        return Err(api_err_exp(c"width", c"non-negative number or -1", None));
    }

    if !(height >= 0 || width >= 0) {
        return Err(api_err_required(c"'height' or 'width'"));
    }

    let mut from_top = true;
    let mut from_left = true;

    if has_key(opts.is_set_win_resize, KEYSET_OPTIDX_WIN_RESIZE_ANCHOR) {
        let anchor = &opts.anchor;
        let anchor_bytes = opts.anchor.as_bytes();
        let is_height = anchor_bytes == b"top" || anchor_bytes == b"bottom";
        let is_width = anchor_bytes == b"left" || anchor_bytes == b"right";

        if !(is_height || is_width) {
            return Err(api_err_exp(
                c"anchor",
                c"\"top\", \"bottom\", \"left\" or \"right\"",
                anchor.as_opt_cstr(),
            ));
        }
        if (is_height && height == -1) || (is_width && width == -1) {
            return Err(api_err_conflict(
                anchor.as_cstr(),
                if is_width { c"width" } else { c"height" },
            ));
        }

        from_top = anchor_bytes != b"bottom";
        from_left = anchor_bytes != b"right";
    }
    if height >= 0 {
        try_wrap(|| {
            win_setheight_win(height as i32, w.borrow(), from_top);
        })?;
    }
    if width >= 0 {
        try_wrap(|| {
            win_setwidth_win(width as i32, w.borrow(), from_left);
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_and_integer_obj_helpers() {
        let obj = integer_obj(42);
        assert_eq!(obj.obj_type, ObjectType::ObjectTypeInteger);
        assert_eq!(obj.data.integer(), 42);
        assert!(has_key(
            1u64 << KEYSET_OPTIDX_WIN_RESIZE_ANCHOR,
            KEYSET_OPTIDX_WIN_RESIZE_ANCHOR
        ));
    }
}
