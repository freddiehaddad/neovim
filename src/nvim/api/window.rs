//! Window API functions (`src/nvim/api/window.c`).

use crate::api::private::helpers::{
    OBJECT_NIL, add_c, api_clear_error, api_set_error, arena_array, arena_dict, dict_get_value,
    dict_set_var, error_set, find_buffer_by_handle, find_window_by_handle, has_key, integer_obj,
    normalize_index, put_c, try_wrap,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType, api_err_conflict, api_err_exp,
    api_err_invalid, api_err_range, api_err_required,
};
use crate::api::tabpage::{Buffer, Tabpage, Window};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{check_cursor_col, colnr_T, linenr_T};
use crate::drawscreen::{UPD_NOT_VALID, UPD_VALID, redraw_later};
use crate::errors::*;
use crate::eval::vars::MAXCOL;
use crate::ex_docmd::ex_win_close;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{Boolean, LuaRef, lua_State};
use crate::lua::executor::{kRetMultiStack, nlua_call_ref};
use crate::memory::{Arena, strequal_str as strequal};
use crate::message::emsg;
use crate::r#move::{update_topline, validate_cursor};
use crate::msgpack_rpc::packer::Integer;
use crate::plines::{win_get_fill, win_text_height};
use crate::strings::gettext;
use crate::window::{
    win_close, win_close_othertab, win_find_tabpage, win_get_tabwin, win_set_buf,
    win_setheight_win, win_setwidth_win,
};
use std::ffi::c_int;
use std::ptr;

use crate::api::win_config::{kCtxNoDisplay, kCtxNoEvents};

pub(crate) const kCtxKeepCwd: c_int = 1;
pub(crate) const kCtxValidate: c_int = 16;

const KEYSET_OPTIDX_win_text_height__end_row: u64 = 1;
const KEYSET_OPTIDX_win_text_height__end_vcol: u64 = 2;
const KEYSET_OPTIDX_win_text_height__start_row: u64 = 3;
const KEYSET_OPTIDX_win_text_height__max_height: u64 = 4;
const KEYSET_OPTIDX_win_text_height__start_vcol: u64 = 5;

const KEYSET_OPTIDX_win_resize__anchor: u64 = 1;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_win_text_height {
    pub is_set__win_text_height_: OptionalKeys,
    pub start_row: Integer,
    pub end_row: Integer,
    pub start_vcol: Integer,
    pub end_vcol: Integer,
    pub max_height: Integer,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_win_resize {
    pub is_set__win_resize_: OptionalKeys,
    pub anchor: NvimString,
}

/// Gets the current buffer in a window
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Buffer id
// DLLEXPORT Buffer nvim_win_get_buf(Window win, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_buf(win: Window, err: *mut Error) -> Buffer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        (*(*w).w_buffer).b_fnum
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_buf(win: Window, buf: Buffer, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }
        win_set_buf(w, b, err);
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_cursor(
    win: Window,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let w = find_window_by_handle(win, err);
        if !w.is_null() {
            rv = arena_array(arena, 2);
            add_c(&mut rv, integer_obj(i64::from((*w).w_cursor.lnum)));
            add_c(&mut rv, integer_obj(i64::from((*w).w_cursor.col)));
        }
        rv
    }
}

/// Sets the (1,0)-indexed cursor position (byte offset) in the window. |api-indexing|
/// This scrolls the window even if it is not the current one.
///
/// @param win   |window-ID|, or 0 for current window
/// @param pos      (row, col) tuple representing the new position
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_cursor(Window win, ArrayOf(Integer, 2) pos, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_cursor(win: Window, pos: Array, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        if pos.size != 2
            || pos[0].obj_type != ObjectType::kObjectTypeInteger
            || pos[1].obj_type != ObjectType::kObjectTypeInteger
        {
            api_err_exp(
                err,
                c"pos".as_ptr(),
                c"[row, col] array".as_ptr(),
                ptr::null(),
            );
            return;
        }

        let row = pos[0].data.integer();
        let col = pos[1].data.integer();

        if row <= 0 || row > i64::from((*(*w).w_buffer).b_ml.ml_line_count) {
            api_err_range(err, c"cursor line".as_ptr());
            return;
        }

        if !(0..=MAXCOL).contains(&col) {
            api_err_range(err, c"cursor column".as_ptr());
            return;
        }

        (*w).w_cursor.lnum = row as linenr_T;
        (*w).w_cursor.col = col as colnr_T;
        (*w).w_cursor.coladd = 0;
        check_cursor_col(w);

        (*w).w_set_curswant = true;

        let mut switchwin: CtxSwitch = CtxSwitch::new();
        if ctx_switch(
            &mut switchwin,
            w,
            ptr::null_mut(),
            ptr::null_mut(),
            kCtxNoEvents | kCtxNoDisplay,
        ) {
            update_topline(curwin);
            validate_cursor(curwin);
        }
        ctx_restore(&mut switchwin);

        redraw_later(w, UPD_VALID);
        (*w).w_redr_status = true;
    }
}

/// Gets the window height
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Height as a count of rows
// DLLEXPORT Integer nvim_win_get_height(Window win, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_height(win: Window, err: *mut Error) -> Integer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        i64::from((*w).w_height)
    }
}

/// Gets the window width
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Width as a count of columns
// DLLEXPORT Integer nvim_win_get_width(Window win, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_width(win: Window, err: *mut Error) -> Integer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        i64::from((*w).w_width)
    }
}

/// Gets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_win_get_var(Window win, String name, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_var(
    win: Window,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return OBJECT_NIL;
        }
        dict_get_value((*w).w_vars(), name, arena, err)
    }
}

/// Sets a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_set_var(Window win, String name, Object value, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_var(
    win: Window,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        dict_set_var((*w).w_vars(), name, value, false, false, ptr::null_mut(), err);
    }
}

/// Removes a window-scoped (w:) variable
///
/// @param win   |window-ID|, or 0 for current window
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_win_del_var(Window win, String name, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_del_var(win: Window, name: NvimString, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        dict_set_var(
            (*w).w_vars(),
            name,
            OBJECT_NIL,
            true,
            false,
            ptr::null_mut(),
            err,
        );
    }
}

/// Gets the window position in display cells. First position is zero.
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return (row, col) tuple with the window position
// DLLEXPORT ArrayOf(Integer, 2) nvim_win_get_position(Window win, Arena *arena, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_position(
    win: Window,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let w = find_window_by_handle(win, err);
        if !w.is_null() {
            rv = arena_array(arena, 2);
            add_c(&mut rv, integer_obj(i64::from((*w).w_winrow)));
            add_c(&mut rv, integer_obj(i64::from((*w).w_wincol)));
        }
        rv
    }
}

/// Gets the window tabpage
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Tabpage that contains the window
// DLLEXPORT Tabpage nvim_win_get_tabpage(Window win, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_tabpage(win: Window, err: *mut Error) -> Tabpage {
    unsafe {
        let mut rv: Tabpage = 0;
        let w = find_window_by_handle(win, err);
        if !w.is_null() {
            rv = (*win_find_tabpage(w)).handle;
        }
        rv
    }
}

/// Gets the window number
///
/// @param win   |window-ID|, or 0 for current window
/// @param[out] err Error details, if any
/// @return Window number
// DLLEXPORT Integer nvim_win_get_number(Window win, Error *err)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_get_number(win: Window, err: *mut Error) -> Integer {
    unsafe {
        let mut rv: c_int = 0;
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return i64::from(rv);
        }
        let mut tabnr: c_int = 0;
        win_get_tabwin((*w).handle, &mut tabnr, &mut rv);
        i64::from(rv)
    }
}

/// Checks if a window is valid
///
/// @param win |window-ID|, or 0 for current window
/// @return true if the window is valid, false otherwise
// DLLEXPORT Boolean nvim_win_is_valid(Window win)
//   FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_is_valid(win: Window) -> Boolean {
    unsafe {
        let mut stub = Error::new();
        let ret = !find_window_by_handle(win, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_hide(win: Window, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        let tabpage = win_find_tabpage(w);
        try_wrap(err, || {
            if is_ctx_win(w) {
                emsg(gettext(e_autocmd_close.as_ptr()));
            } else if tabpage == curtab {
                win_close(w, false, false);
            } else {
                win_close_othertab(w, 0, tabpage, false);
            }
        });
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_close(win: Window, force: Boolean, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        let tabpage = win_find_tabpage(w);
        try_wrap(err, || {
            ex_win_close(
                c_int::from(force),
                w,
                if tabpage == curtab {
                    ptr::null_mut()
                } else {
                    tabpage
                },
            );
        });
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_call(
    win: Window,
    fn_ref: LuaRef,
    _lstate: *mut lua_State,
    err: *mut Error,
) -> Object {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return OBJECT_NIL;
        }
        let tabpage = win_find_tabpage(w);

        try_wrap(err, || {
            let mut cs: CtxSwitch = CtxSwitch::new();
            if ctx_switch(
                &mut cs,
                w,
                tabpage,
                ptr::null_mut(),
                kCtxNoDisplay | kCtxKeepCwd | kCtxValidate,
            ) {
                let args = Array::new();
                nlua_call_ref(
                    fn_ref,
                    ptr::null(),
                    args,
                    kRetMultiStack,
                    ptr::null_mut(),
                    err,
                );
            }
            ctx_restore(&mut cs);
        });
        OBJECT_NIL
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_set_hl_ns(win: Window, ns_id: Integer, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        if ns_id < -1 {
            api_err_invalid(err, c"namespace".as_ptr(), c"".as_ptr(), 0, true);
            return;
        }

        (*w).w_ns_hl = ns_id as c_int;
        (*w).w_hl_needs_update = 1;
        redraw_later(w, UPD_NOT_VALID);
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_text_height(
    win: Window,
    opts: *mut KeyDict_win_text_height,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut rv = arena_dict(arena, 4);

        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return rv;
        }
        let buf = (*w).w_buffer;
        let line_count = (*buf).b_ml.ml_line_count;

        let mut start_lnum: linenr_T = 1;
        let mut end_lnum: linenr_T = line_count;
        let mut start_vcol: i64 = -1;
        let mut end_vcol: i64 = -1;

        let mut oob = false;
        let mask = (*opts).is_set__win_text_height_;

        if has_key(mask, KEYSET_OPTIDX_win_text_height__start_row) {
            start_lnum = normalize_index(buf, (*opts).start_row, false, &mut oob) as linenr_T;
        }

        if has_key(mask, KEYSET_OPTIDX_win_text_height__end_row) {
            end_lnum = normalize_index(buf, (*opts).end_row, false, &mut oob) as linenr_T;
        }

        if oob {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Line index out of bounds".as_ptr(),
            );
            return rv;
        }
        if start_lnum > end_lnum {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start_row' is higher than 'end_row'".as_ptr(),
            );
            return rv;
        }

        if has_key(mask, KEYSET_OPTIDX_win_text_height__start_vcol) {
            if !has_key(mask, KEYSET_OPTIDX_win_text_height__start_row) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"'start_vcol' specified without 'start_row'".as_ptr(),
                );
                return rv;
            }
            start_vcol = (*opts).start_vcol;
            if !(0..=MAXCOL).contains(&start_vcol) {
                api_err_range(err, c"start_vcol".as_ptr());
                return rv;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_win_text_height__end_vcol) {
            if !has_key(mask, KEYSET_OPTIDX_win_text_height__end_row) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"'end_vcol' specified without 'end_row'".as_ptr(),
                );
                return rv;
            }
            end_vcol = (*opts).end_vcol;
            if !(0..=MAXCOL).contains(&end_vcol) {
                api_err_range(err, c"end_vcol".as_ptr());
                return rv;
            }
        }

        let mut max: i64 = i64::MAX;
        if has_key(mask, KEYSET_OPTIDX_win_text_height__max_height) {
            if (*opts).max_height <= 0 {
                api_err_range(err, c"max_height".as_ptr());
                return rv;
            }
            max = (*opts).max_height;
        }

        if start_lnum == end_lnum && start_vcol >= 0 && end_vcol >= 0 && start_vcol > end_vcol {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start_vcol' is higher than 'end_vcol'".as_ptr(),
            );
            return rv;
        }

        let mut fill: i64 = 0;
        let mut all = win_text_height(
            w,
            start_lnum,
            start_vcol,
            &mut end_lnum,
            &mut end_vcol,
            &mut fill,
            max,
        );
        if !has_key(mask, KEYSET_OPTIDX_win_text_height__end_row) {
            let end_fill = i64::from(win_get_fill(w, line_count + 1));
            fill += end_fill;
            all += end_fill;
        }
        put_c(&mut rv, c"all", integer_obj(all));
        put_c(&mut rv, c"fill", integer_obj(fill));
        put_c(&mut rv, c"end_row", integer_obj(i64::from(end_lnum - 1)));
        put_c(&mut rv, c"end_vcol", integer_obj(end_vcol));
        rv
    }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_win_resize(
    win: Window,
    width: Integer,
    height: Integer,
    opts: *mut KeyDict_win_resize,
    err: *mut Error,
) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        if !(height >= 0 || height == -1) {
            api_err_exp(
                err,
                c"height".as_ptr(),
                c"non-negative number or -1".as_ptr(),
                ptr::null(),
            );
            return;
        }
        if !(width >= 0 || width == -1) {
            api_err_exp(
                err,
                c"width".as_ptr(),
                c"non-negative number or -1".as_ptr(),
                ptr::null(),
            );
            return;
        }

        if !(height >= 0 || width >= 0) {
            api_err_required(err, c"'height' or 'width'".as_ptr());
            return;
        }

        let mut from_top = true;
        let mut from_left = true;

        if has_key(
            (*opts).is_set__win_resize_,
            KEYSET_OPTIDX_win_resize__anchor,
        ) {
            let anchor = (*opts).anchor.data;
            let is_height = strequal(anchor, "top") || strequal(anchor, "bottom");
            let is_width = strequal(anchor, "left") || strequal(anchor, "right");

            if !(is_height || is_width) {
                api_err_exp(
                    err,
                    c"anchor".as_ptr(),
                    c"\"top\", \"bottom\", \"left\" or \"right\"".as_ptr(),
                    anchor,
                );
                return;
            }
            if (is_height && height == -1) || (is_width && width == -1) {
                api_err_conflict(
                    err,
                    anchor,
                    if is_width {
                        c"width".as_ptr()
                    } else {
                        c"height".as_ptr()
                    },
                );
                return;
            }

            from_top = !strequal(anchor, "bottom");
            from_left = !strequal(anchor, "right");
        }
        if height >= 0 {
            try_wrap(err, || {
                win_setheight_win(height as c_int, w, from_top);
            });
        }
        if !error_set(err) && width >= 0 {
            try_wrap(err, || {
                win_setwidth_win(width as c_int, w, from_left);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_and_integer_obj_helpers() {
        let obj = integer_obj(42);
        assert_eq!(obj.obj_type, ObjectType::kObjectTypeInteger);
        assert_eq!(obj.data.integer(), 42);
        assert!(has_key(
            1u64 << KEYSET_OPTIDX_win_resize__anchor,
            KEYSET_OPTIDX_win_resize__anchor
        ));
    }
}
