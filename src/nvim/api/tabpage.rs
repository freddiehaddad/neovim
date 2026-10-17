//! Tabpage API functions (`src/nvim/api/tabpage.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_void};

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::private::validate::{Array, Error, ErrorType, Object, ObjectData, ObjectType};
use crate::clipboard::typval_T;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::lua::converter::Boolean;
use crate::map::Set;
use crate::map_glyph_cache::NvimString;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::{Integer, handle_T};

pub type Buffer = handle_T;
pub type Window = handle_T;
pub type Tabpage = handle_T;

const KEYSET_OPTIDX_tabpage_config__after: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct pos_T {
    pub lnum: i32,
    pub col: i32,
    pub coladd: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScopeDictDictItem {
    pub di_tv: typval_T,
    pub di_flags: u8,
    pub di_key: [c_char; 1],
}

#[repr(C)]
pub struct win_T {
    pub handle: handle_T,
    pub w_buffer: *mut buf_T,
    pub w_s: *mut c_void,
    pub w_ns_hl: c_int,
    pub w_ns_hl_winhl: c_int,
    pub w_ns_hl_active: c_int,
    pub w_ns_hl_attr: *mut c_int,
    pub w_ns_set: Set<u32>,
    pub w_hl_id_normal: c_int,
    pub w_hl_attr_normal: c_int,
    pub w_hl_attr_normalnc: c_int,
    pub w_hl_needs_update: c_int,
    pub w_prev: *mut win_T,
    pub w_next: *mut win_T,
    pub w_locked: c_int,
    pub w_frame: *mut c_void,
    pub w_cursor: pos_T,
    pub w_curswant: i32,
    pub w_set_curswant: bool,
}

#[repr(C)]
pub struct tabpage_T {
    pub handle: handle_T,
    pub tp_next: *mut tabpage_T,
    pub tp_topframe: *mut c_void,
    pub tp_curwin: *mut win_T,
    pub tp_prevwin: *mut win_T,
    pub tp_firstwin: *mut win_T,
    pub tp_lastwin: *mut win_T,
    pub tp_old_Rows_avail: i64,
    pub tp_old_Columns: i64,
    pub tp_ch_used: i64,
    pub tp_did_tabclosedpre: bool,
    pub tp_first_diff: *mut c_void,
    pub tp_diffbuf: [*mut buf_T; 8],
    pub tp_diff_invalid: c_int,
    pub tp_diff_update: c_int,
    pub tp_snapshot: [*mut c_void; 3],
    pub tp_winvar: ScopeDictDictItem,
    pub tp_vars: *mut dict_T,
    pub tp_localdir: *mut c_char,
    pub tp_prevdir: *mut c_char,
}

#[repr(C)]
pub struct KeyDict_tabpage_config {
    pub is_set__tabpage_config_: OptionalKeys,
    pub after: Integer,
}

#[cfg(not(test))]
unsafe extern "C" {
    static mut curtab: *mut tabpage_T;
    static mut curwin: *mut win_T;
    static mut firstwin: *mut win_T;
    static mut autocmd_no_enter: c_int;
    static mut autocmd_no_leave: c_int;

    fn find_tab_by_handle(tabpage: Tabpage, err: *mut Error) -> *mut tabpage_T;
    fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T;
    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn valid_tabpage(tpc: *mut tabpage_T) -> bool;
    fn tabpage_win_valid(tp: *const tabpage_T, wp: *const win_T) -> bool;
    fn tabpage_index(ftp: *mut tabpage_T) -> c_int;
    fn win_goto(wp: *mut win_T);
    fn win_new_tabpage(
        after: c_int,
        filename: *mut c_char,
        enter: bool,
        wp_out: *mut *mut win_T,
    ) -> *mut tabpage_T;
    fn win_set_buf(wp: *mut win_T, b: *mut buf_T, err: *mut Error);
    fn nvim_get_current_win() -> Window;

    fn dict_get_value(
        dict: *mut dict_T,
        key: NvimString,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn dict_set_var(
        dict: *mut dict_T,
        key: NvimString,
        value: Object,
        del: bool,
        retval: bool,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn api_clear_error(value: *mut Error);
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
}

#[inline]
const fn nil_obj() -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeNil,
        data: ObjectData { integer: 0 },
    }
}

#[inline]
const fn window_obj(w: Window) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeWindow,
        data: ObjectData {
            integer: w as Integer,
        },
    }
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

/// Gets the windows in a tabpage (`nvim_tabpage_list_wins`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_list_wins(
    tabpage: Tabpage,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    #[cfg(test)]
    {
        let _ = (tabpage, arena, err);
        Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        }
    }

    #[cfg(not(test))]
    unsafe {
        let mut rv = Array {
            items: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        let tab = find_tab_by_handle(tabpage, err);

        if tab.is_null() || !valid_tabpage(tab) {
            return rv;
        }

        let first = if tab == curtab {
            firstwin
        } else {
            (*tab).tp_firstwin
        };
        let mut n = 0usize;
        let mut wp = first;
        while !wp.is_null() {
            n += 1;
            wp = (*wp).w_next;
        }

        rv = arena_array(arena, n);
        wp = first;
        while !wp.is_null() {
            *rv.items.add(rv.size) = window_obj((*wp).handle);
            rv.size += 1;
            wp = (*wp).w_next;
        }

        rv
    }
}

/// Gets a tab-scoped (`t:`) variable (`nvim_tabpage_get_var`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_var(
    tabpage: Tabpage,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    #[cfg(test)]
    {
        let _ = (tabpage, name, arena, err);
        nil_obj()
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return nil_obj();
        }
        dict_get_value((*tab).tp_vars, name, arena, err)
    }
}

/// Sets a tab-scoped (`t:`) variable (`nvim_tabpage_set_var`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    #[cfg(test)]
    let _ = (tabpage, name, value, err);

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars,
            name,
            value,
            false,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Removes a tab-scoped (`t:`) variable (`nvim_tabpage_del_var`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_del_var(tabpage: Tabpage, name: NvimString, err: *mut Error) {
    #[cfg(test)]
    let _ = (tabpage, name, err);

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars,
            name,
            nil_obj(),
            true,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Gets the current window in a tabpage (`nvim_tabpage_get_win`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_win(tabpage: Tabpage, err: *mut Error) -> Window {
    #[cfg(test)]
    {
        let _ = (tabpage, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() || !valid_tabpage(tab) {
            return 0;
        }
        if tab == curtab {
            return nvim_get_current_win();
        }
        let mut wp = (*tab).tp_firstwin;
        while !wp.is_null() {
            if wp == (*tab).tp_curwin {
                return (*wp).handle;
            }
            wp = (*wp).w_next;
        }
        std::process::abort();
    }
}

/// Sets the current window in a tabpage (`nvim_tabpage_set_win`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_win(tabpage: Tabpage, win: Window, err: *mut Error) {
    #[cfg(test)]
    let _ = (tabpage, win, err);

    #[cfg(not(test))]
    unsafe {
        let tp = find_tab_by_handle(tabpage, err);
        if tp.is_null() {
            return;
        }
        let wp = find_window_by_handle(win, err);
        if wp.is_null() {
            return;
        }
        if !tabpage_win_valid(tp, wp) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Window does not belong to tabpage %d".as_ptr(),
                (*tp).handle,
            );
            return;
        }
        if tp == curtab {
            let mut tstate: TryState = std::mem::zeroed();
            try_enter(&mut tstate);
            win_goto(wp);
            try_leave(&tstate, err);
        } else if (*tp).tp_curwin != wp {
            (*tp).tp_prevwin = (*tp).tp_curwin;
            (*tp).tp_curwin = wp;
        }
    }
}

/// Gets the tabpage number (`nvim_tabpage_get_number`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_number(tabpage: Tabpage, err: *mut Error) -> Integer {
    #[cfg(test)]
    {
        let _ = (tabpage, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return 0;
        }
        i64::from(tabpage_index(tab))
    }
}

/// Checks if a tabpage is valid (`nvim_tabpage_is_valid`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_is_valid(tabpage: Tabpage) -> Boolean {
    #[cfg(test)]
    {
        let _ = tabpage;
        false
    }

    #[cfg(not(test))]
    unsafe {
        let mut stub = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: std::ptr::null_mut(),
        };
        let ret = !find_tab_by_handle(tabpage, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
}

/// Opens a new tabpage (`nvim_open_tabpage`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_open_tabpage(
    buf: Buffer,
    enter: Boolean,
    config: *mut KeyDict_tabpage_config,
    err: *mut Error,
) -> Tabpage {
    #[cfg(test)]
    {
        let _ = (buf, enter, config, err);
        0
    }

    #[cfg(not(test))]
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return 0;
        }

        let after: c_int = if has_key(
            (*config).is_set__tabpage_config_,
            KEYSET_OPTIDX_tabpage_config__after,
        ) {
            (*config).after as c_int
        } else {
            -1
        };

        let mut wp: *mut win_T = std::ptr::null_mut();
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        let tp = win_new_tabpage(after + 1, std::ptr::null_mut(), enter, &mut wp);
        try_leave(&tstate, err);

        if tp.is_null() {
            if !error_set(err) {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to create new tabpage".as_ptr(),
                );
            }
            return 0;
        }
        if !valid_tabpage(tp) {
            api_clear_error(err);
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Tabpage was closed immediately".as_ptr(),
            );
            return 0;
        }

        if tabpage_win_valid(tp, wp) && (*wp).w_buffer != b {
            let au_no_enter_leave = curwin != wp;
            if au_no_enter_leave {
                autocmd_no_enter += 1;
                autocmd_no_leave += 1;
            }
            win_set_buf(wp, b, err);
            if au_no_enter_leave {
                autocmd_no_enter -= 1;
                autocmd_no_leave -= 1;
            }
            if !valid_tabpage(tp) {
                api_clear_error(err);
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Tabpage was closed immediately".as_ptr(),
                );
                return 0;
            }
        }

        (*tp).handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_obj_creates_window_object() {
        let o = window_obj(1002);
        assert_eq!(o.obj_type, ObjectType::kObjectTypeWindow);
        assert_eq!(unsafe { o.data.integer }, 1002);
    }
}
