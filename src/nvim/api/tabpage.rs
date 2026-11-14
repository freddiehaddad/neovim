//! Tabpage API functions (`src/nvim/api/tabpage.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    TryState, api_error, dict_get_value, dict_set_var, find_tab_by_handle, find_window_by_handle,
    has_key, nil_obj, try_enter, try_leave, window_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString, Object};
use crate::api::vim::nvim_get_current_win;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::Boolean;
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::window::{
    tabpage_index, tabpage_win_valid, valid_tabpage, win_goto, win_new_tabpage, win_set_buf,
};

pub type Buffer = ObjHandle;
pub type Window = ObjHandle;
pub type Tabpage = ObjHandle;

const KEYSET_OPTIDX_TABPAGE_CONFIG_AFTER: u64 = 1;

#[derive(Default)]
pub struct KeyDictTabpageConfig {
    pub is_set_tabpage_config: OptionalKeys,
    pub after: Integer,
}

/// Gets the windows in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return List of windows in `tabpage`
// DLLEXPORT ArrayOf(Window) nvim_tabpage_list_wins(Tabpage tabpage, Arena *arena,
//                                                  Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_list_wins(tabpage: Tabpage) -> Result<Array, Error> {
    let mut rv = Array::new();
    let tab = find_tab_by_handle(tabpage)?;
    if !valid_tabpage(Some(&tab)) {
        return Ok(rv);
    }

    let first = if g_st().curtab.get() == tab {
        g_st().firstwin.get()
    } else {
        tab.tp_firstwin.get()
    };
    let n = win_iter_ptr(&first).count();

    rv = Array::with_capacity(n);
    for wp in win_iter_ptr(first) {
        *rv.next_slot_mut() = window_obj(wp.handle);
        rv.size += 1;
    }

    Ok(rv)
}

/// Gets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
/// @return Variable value
// DLLEXPORT Object nvim_tabpage_get_var(Tabpage tabpage, String name, Arena *arena,
//                                       Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_get_var(tabpage: Tabpage, name: NvimString) -> Result<Object, Error> {
    let tab = crate::api::private::helpers::find_tab_by_handle(tabpage)?;
    dict_get_value(Some(&*tab.tp_vars()), name)
}

/// Sets a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param value    Variable value
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_var(Tabpage tabpage, String name, Object value,
//                                     Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
) -> Result<(), Error> {
    let tab = crate::api::private::helpers::find_tab_by_handle(tabpage)?;
    dict_set_var(&tab.tp_vars(), name, value, false, false)?;
    Ok(())
}

/// Removes a tab-scoped (t:) variable
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param name     Variable name
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_del_var(Tabpage tabpage, String name, Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_del_var(tabpage: Tabpage, name: NvimString) -> Result<(), Error> {
    let tab = crate::api::private::helpers::find_tab_by_handle(tabpage)?;
    dict_set_var(&tab.tp_vars(), name, nil_obj(), true, false)?;
    Ok(())
}

/// Gets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return |window-ID|
// DLLEXPORT Window nvim_tabpage_get_win(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_get_win(tabpage: Tabpage) -> Result<Window, Error> {
    let tab = find_tab_by_handle(tabpage)?;
    if !valid_tabpage(Some(&tab)) {
        return Ok(0);
    }
    if g_st().curtab.get() == tab {
        return Ok(nvim_get_current_win());
    }
    for wp in win_iter_ptr(tab.tp_firstwin.get()) {
        if tab.tp_curwin.get() == wp {
            return Ok(wp.handle);
        }
    }
    std::process::abort();
}

/// Sets the current window in a tabpage
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param win |window-ID|, must already belong to {tabpage}
/// @param[out] err Error details, if any
// DLLEXPORT void nvim_tabpage_set_win(Tabpage tabpage, Window win, Error *err) FUNC_API_SINCE(12);
pub fn nvim_tabpage_set_win(tabpage: Tabpage, win: Window) -> Result<(), Error> {
    let tp = crate::api::private::helpers::find_tab_by_handle(tabpage)?;
    let wp = find_window_by_handle(win)?;
    if !tabpage_win_valid(Some(&tp), wp.opt()) {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Window does not belong to tabpage %d",
            tp.handle,
        ));
    }
    if tp == g_st().curtab.get() {
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        win_goto(wp.borrow());
        try_leave(&tstate)?;
    } else if tp.tp_curwin.get() != wp {
        tp.tp_prevwin.set(tp.tp_curwin.get());
        tp.tp_curwin.set(wp);
    }
    Ok(())
}

/// Gets the tabpage number
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @param[out] err Error details, if any
/// @return Tabpage number
// DLLEXPORT Integer nvim_tabpage_get_number(Tabpage tabpage, Error *err) FUNC_API_SINCE(1);
pub fn nvim_tabpage_get_number(tabpage: Tabpage) -> Result<Integer, Error> {
    let tab = find_tab_by_handle(tabpage)?;
    Ok(i64::from(tabpage_index(Some(&tab))))
}

/// Checks if a tabpage is valid
///
/// @param tabpage  |tab-ID|, or 0 for current tabpage
/// @return true if the tabpage is valid, false otherwise
// DLLEXPORT Boolean nvim_tabpage_is_valid(Tabpage tabpage) FUNC_API_SINCE(1);
pub fn nvim_tabpage_is_valid(tabpage: Tabpage) -> Boolean {
    find_tab_by_handle(tabpage).is_ok()
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
pub fn nvim_open_tabpage(
    buf: Buffer,
    enter: Boolean,
    config: &mut KeyDictTabpageConfig,
) -> Result<Tabpage, Error> {
    let b = crate::api::private::helpers::find_buffer_by_handle(buf)?;

    let after: i32 = if has_key(
        config.is_set_tabpage_config,
        KEYSET_OPTIDX_TABPAGE_CONFIG_AFTER,
    ) {
        config.after as i32
    } else {
        -1
    };

    let mut wp_opt: Option<crate::globals::WinPtr> = None;
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let tp = crate::globals::TabpagePtr::from(win_new_tabpage(
        after + 1,
        None,
        enter,
        Some(&mut wp_opt),
    ));
    let mut res = try_leave(&tstate);
    let wp = crate::globals::WinPtr::from(wp_opt);

    if tp.is_null() {
        return Err(match res {
            Err(err) => err,
            Ok(()) => api_error!(
                ErrorType::ErrorTypeException,
                c"Failed to create new tabpage",
            ),
        });
    }
    if !valid_tabpage(tp.opt()) {
        return Err(api_error!(
            ErrorType::ErrorTypeException,
            c"Tabpage was closed immediately",
        ));
    }

    if tabpage_win_valid(tp.opt(), wp.opt()) && wp.w_buffer.get() != b {
        let au_no_enter_leave = curwin() != wp;
        if au_no_enter_leave {
            g_st()
                .autocmd_no_enter
                .set(g_st().autocmd_no_enter.get() + (1));
            g_st()
                .autocmd_no_leave
                .set(g_st().autocmd_no_leave.get() + (1));
        }
        if let Err(err) = win_set_buf(wp.borrow(), &b) {
            res = Err(err);
        }
        if au_no_enter_leave {
            g_st()
                .autocmd_no_enter
                .set(g_st().autocmd_no_enter.get() - (1));
            g_st()
                .autocmd_no_leave
                .set(g_st().autocmd_no_leave.get() - (1));
        }
        if !valid_tabpage(tp.opt()) {
            return Err(api_error!(
                ErrorType::ErrorTypeException,
                c"Tabpage was closed immediately",
            ));
        }
    }

    res?;
    Ok(tp.handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::validate::ObjectType;

    #[test]
    fn window_obj_creates_window_object() {
        let o = window_obj(1002);
        assert_eq!(o.obj_type, ObjectType::ObjectTypeWindow);
        assert_eq!(o.data.integer(), 1002);
    }
}
