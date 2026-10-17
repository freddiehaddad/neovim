//! Window API functions (`src/nvim/api/window.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::options::CtxSwitch;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_conflict,
    api_err_exp, api_err_invalid, api_err_required,
};
use crate::api::tabpage::{Buffer, Tabpage, Window, tabpage_T, win_T};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::lua::base64::lua_State;
use crate::lua::converter::{Boolean, LuaRef};
use crate::map_glyph_cache::NvimString;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::Integer;

pub type linenr_T = i32;
pub type colnr_T = c_int;
pub type LuaRetMode = c_int;

const MAXCOL: i64 = 0x7fffffff;
const UPD_VALID: c_int = 10;
const UPD_NOT_VALID: c_int = 40;

const kCtxKeepCwd: c_int = 1;
const kCtxNoDisplay: c_int = 4;
const kCtxNoEvents: c_int = 8;
const kCtxValidate: c_int = 16;

const kRetMultiStack: LuaRetMode = 4;

const KEYSET_OPTIDX_win_text_height__end_row: u64 = 1;
const KEYSET_OPTIDX_win_text_height__end_vcol: u64 = 2;
const KEYSET_OPTIDX_win_text_height__start_row: u64 = 3;
const KEYSET_OPTIDX_win_text_height__max_height: u64 = 4;
const KEYSET_OPTIDX_win_text_height__start_vcol: u64 = 5;

const KEYSET_OPTIDX_win_resize__anchor: u64 = 1;

#[repr(C)]
pub struct KeyDict_win_text_height {
    pub is_set__win_text_height_: OptionalKeys,
    pub start_row: Integer,
    pub end_row: Integer,
    pub start_vcol: Integer,
    pub end_vcol: Integer,
    pub max_height: Integer,
}

#[repr(C)]
pub struct KeyDict_win_resize {
    pub is_set__win_resize_: OptionalKeys,
    pub anchor: NvimString,
}

unsafe extern "C" {
    static mut curwin: *mut win_T;
    static mut curtab: *mut tabpage_T;
    static e_autocmd_close: [c_char; 0];

    fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T;
    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn win_set_buf(win: *mut win_T, buf: *mut buf_T, err: *mut Error);
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict;
    fn check_cursor_col(win: *mut win_T);
    fn ctx_switch(
        cs: *mut CtxSwitch,
        win: *mut win_T,
        tabpage: *mut tabpage_T,
        buf: *mut c_void,
        flags: c_int,
    ) -> bool;
    fn ctx_restore(cs: *mut CtxSwitch);
    fn update_topline(wp: *mut win_T);
    fn validate_cursor(wp: *mut win_T);
    fn redraw_later(wp: *mut win_T, type_: c_int);
    fn dict_get_value(
        d: *mut dict_T,
        name: NvimString,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn dict_set_var(
        dict: *mut dict_T,
        name: NvimString,
        value: Object,
        del: bool,
        retval: bool,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn win_find_tabpage(win: *mut win_T) -> *mut tabpage_T;
    fn win_get_tabwin(id: handle_T, tabnr: *mut c_int, winnr: *mut c_int);
    fn api_clear_error(value: *mut Error);
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
    fn is_ctx_win(win: *mut win_T) -> bool;
    fn emsg(s: *const c_char) -> bool;
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn win_close(win: *mut win_T, free_buf: bool, force: bool) -> c_int;
    fn win_close_othertab(win: *mut win_T, free_buf: c_int, tp: *mut tabpage_T, force: bool);
    fn ex_win_close(forceit: bool, win: *mut win_T, tp: *mut tabpage_T);
    fn nlua_call_ref(
        ref_id: LuaRef,
        name: *const c_char,
        args: Array,
        mode: LuaRetMode,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn normalize_index(buf: *mut buf_T, index: i64, end_exclusive: bool, oob: *mut bool) -> i64;
    fn win_text_height(
        wp: *mut win_T,
        start_lnum: linenr_T,
        start_vcol: i64,
        end_lnum: *mut linenr_T,
        end_vcol: *mut i64,
        fill: *mut i64,
        max: i64,
    ) -> i64;
    fn win_get_fill(wp: *mut win_T, lnum: linenr_T) -> c_int;
    fn win_setheight_win(height: c_int, win: *mut win_T, from_top: bool);
    fn win_setwidth_win(width: c_int, wp: *mut win_T, from_left: bool);
}

use crate::msgpack_rpc::packer::handle_T;

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn cstr_as_string(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
unsafe fn add_c(arr: &mut Array, item: Object) {
    unsafe {
        *arr.items.add(arr.size) = item;
        arr.size += 1;
    }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_as_string(key),
            value,
        };
        dict.size += 1;
    }
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn strequal(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

#[inline]
unsafe fn api_err_range(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_buf(win: Window, err: *mut Error) -> Buffer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        (*(*w).w_buffer).b_fnum
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_set_cursor(win: Window, pos: Array, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        if pos.size != 2
            || (*pos.items.add(0)).obj_type != ObjectType::kObjectTypeInteger
            || (*pos.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
        {
            api_err_exp(
                err,
                c"pos".as_ptr(),
                c"[row, col] array".as_ptr(),
                ptr::null(),
            );
            return;
        }

        let row = (*pos.items.add(0)).data.integer;
        let col = (*pos.items.add(1)).data.integer;

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

        let mut switchwin: CtxSwitch = std::mem::zeroed();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_height(win: Window, err: *mut Error) -> Integer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        i64::from((*w).w_height)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_width(win: Window, err: *mut Error) -> Integer {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return 0;
        }
        i64::from((*w).w_width)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_var(
    win: Window,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return NIL;
        }
        dict_get_value((*w).w_vars, name, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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
        dict_set_var((*w).w_vars, name, value, false, false, ptr::null_mut(), err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_del_var(win: Window, name: NvimString, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        dict_set_var((*w).w_vars, name, NIL, true, false, ptr::null_mut(), err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_is_valid(win: Window) -> Boolean {
    unsafe {
        let mut stub = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let ret = !find_window_by_handle(win, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_close(win: Window, force: Boolean, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }

        let tabpage = win_find_tabpage(w);
        try_wrap(err, || {
            ex_win_close(
                force,
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_call(
    win: Window,
    fn_ref: LuaRef,
    _lstate: *mut lua_State,
    err: *mut Error,
) -> Object {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return NIL;
        }
        let tabpage = win_find_tabpage(w);

        try_wrap(err, || {
            let mut cs: CtxSwitch = std::mem::zeroed();
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
        NIL
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"Line index out of bounds".as_ptr(),
            );
            return rv;
        }
        if start_lnum > end_lnum {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"%s".as_ptr(),
                c"'start_row' is higher than 'end_row'".as_ptr(),
            );
            return rv;
        }

        if has_key(mask, KEYSET_OPTIDX_win_text_height__start_vcol) {
            if !has_key(mask, KEYSET_OPTIDX_win_text_height__start_row) {
                api_set_error(
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
                api_set_error(
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
            api_set_error(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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
        assert_eq!(unsafe { obj.data.integer }, 42);
        assert!(has_key(
            1u64 << KEYSET_OPTIDX_win_resize__anchor,
            KEYSET_OPTIDX_win_resize__anchor
        ));
    }
}
