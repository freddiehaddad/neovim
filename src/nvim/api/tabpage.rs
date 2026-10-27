//! Tabpage API functions (`src/nvim/api/tabpage.c`).

use crate::api::private::helpers::{
    TryState, api_clear_error, api_set_error, arena_array, dict_get_value, dict_set_var, error_set,
    find_buffer_by_handle, find_tab_by_handle, find_window_by_handle, has_key, nil_obj, try_enter,
    try_leave, window_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString, Object};
use crate::api::vim::nvim_get_current_win;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::Boolean;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::window::{
    tabpage_index, tabpage_win_valid, valid_tabpage, win_T, win_goto, win_new_tabpage, win_set_buf,
};
use std::ffi::c_int;

pub type Buffer = handle_T;
pub type Window = handle_T;
pub type Tabpage = handle_T;

const KEYSET_OPTIDX_tabpage_config__after: u64 = 1;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_tabpage_config {
    pub is_set__tabpage_config_: OptionalKeys,
    pub after: Integer,
}

/// Gets the windows in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return List of windows in `tabpage`
// DLLEXPORT ArrayOf(Window) nvim_tabpage_list_wins(Tabpage tabpage, Arena *arena,
//                                                  Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_list_wins(
    tabpage: Tabpage,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
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

/// Gets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_tabpage_get_var(Tabpage tabpage, String name, Arena *arena,
//                                       Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_var(
    tabpage: Tabpage,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return nil_obj();
        }
        dict_get_value((*tab).tp_vars(), name, arena, err)
    }
}

/// Sets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_var(Tabpage tabpage, String name, Object value,
//                                     Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars(),
            name,
            value,
            false,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Removes a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_del_var(Tabpage tabpage, String name, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_del_var(tabpage: Tabpage, name: NvimString, err: *mut Error) {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return;
        }
        dict_set_var(
            (*tab).tp_vars(),
            name,
            nil_obj(),
            true,
            false,
            std::ptr::null_mut(),
            err,
        );
    }
}

/// Gets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return |window-ID|
// DLLEXPORT Window nvim_tabpage_get_win(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_win(tabpage: Tabpage, err: *mut Error) -> Window {
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

/// Sets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param win |window-ID|, must already belong to {tabpage}
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_win(Tabpage tabpage, Window win, Error *err) FUNC_API_SINCE(12);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_set_win(tabpage: Tabpage, win: Window, err: *mut Error) {
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
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Window does not belong to tabpage %d".as_ptr(),
                (*tp).handle,
            );
            return;
        }
        if tp == curtab {
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            win_goto(wp);
            try_leave(&tstate, err);
        } else if (*tp).tp_curwin != wp {
            (*tp).tp_prevwin = (*tp).tp_curwin;
            (*tp).tp_curwin = wp;
        }
    }
}

/// Gets the tabpage number
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return Tabpage number
// DLLEXPORT Integer nvim_tabpage_get_number(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_get_number(tabpage: Tabpage, err: *mut Error) -> Integer {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return 0;
        }
        i64::from(tabpage_index(tab))
    }
}

/// Checks if a tabpage is valid
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @return true if the tabpage is valid, false otherwise
// DLLEXPORT Boolean nvim_tabpage_is_valid(Tabpage tabpage) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_tabpage_is_valid(tabpage: Tabpage) -> Boolean {
    unsafe {
        let mut stub = Error::new();
        let ret = !find_tab_by_handle(tabpage, &mut stub).is_null();
        api_clear_error(&mut stub);
        ret
    }
}

/// Opens a new tabpage.
///
/// @param buf Buffer to open in the first window of the new tabpage.
///               Use 0 for current buffer.
/// @param enter  Enter the tabpage (make it the current tabpage).
/// @param config Configuration for the new tabpage. Keys:
///   - after: Position to insert tabpage (default: -1; after current).
///            0 = first, N = after Nth.
/// @param[out] err Error details, if any
/// @return |tab-ID| of the new tabpage
// DLLEXPORT Tabpage nvim_open_tabpage(Buffer buf, Boolean enter, Dict(tabpage_config) *config,
//                                     Error *err) FUNC_API_SINCE(14) FUNC_API_TEXTLOCK;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_open_tabpage(
    buf: Buffer,
    enter: Boolean,
    config: *mut KeyDict_tabpage_config,
    err: *mut Error,
) -> Tabpage {
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
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let tp = win_new_tabpage(after + 1, std::ptr::null_mut(), enter, &mut wp);
        try_leave(&tstate, err);

        if tp.is_null() {
            if !error_set(err) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to create new tabpage".as_ptr(),
                );
            }
            return 0;
        }
        if !valid_tabpage(tp) {
            api_clear_error(err);
            api_set_error!(
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
                api_set_error!(
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
    use crate::api::private::validate::ObjectType;

    #[test]
    fn window_obj_creates_window_object() {
        let o = window_obj(1002);
        assert_eq!(o.obj_type, ObjectType::kObjectTypeWindow);
        assert_eq!(o.data.integer(), 1002);
    }
}
