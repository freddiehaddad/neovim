//! Window related builtin functions (`src/nvim/eval/window.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::buffer::{bt_cmdwin, bt_quickfix, bt_terminal, first_win_in_tab};
use crate::charset::getdigits_int;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{check_cursor, colnr_T, linenr_T};
use crate::errors::*;
use crate::eval::funcs::execute_common;
use crate::eval::typval::{
    EvalFuncData, ScopeDictDictItem, VarType, dict_T, list_T, tv_check_for_nonnull_dict_arg,
    tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr, tv_dict_alloc, tv_dict_alloc_ret,
    tv_dict_find, tv_dict_get_bool, tv_dict_get_number, tv_get_bool, tv_get_number,
    tv_get_number_chk, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_list, tv_list_append_number, tv_list_append_string, typval_T,
};
use crate::ex_getln::text_or_buf_locked;
use crate::garray::{ga_append, ga_concat_len, ga_init, garray_T};
use crate::globals::*;
use crate::memory::{xmallocz, xstrdup};
use crate::message::{emsg, semsg};
use crate::r#move::{
    changed_window_setting, check_topfill, restore_viewstate, save_viewstate, set_topline,
    update_curswant, validate_botline_win, validate_cursor, win_col_off,
};
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::end_visual_mode;
use crate::strings::gettext;
use crate::window::{
    LOWEST_WIN_ID, WSP_ABOVE, WSP_BELOW, WSP_VERT, check_split_disallowed, find_tabpage, frame_T,
    goto_tabpage_win, tabpage_T, tabpage_index, valid_tabpage, win_T, win_drag_status_line,
    win_drag_vsep_line, win_get_tabwin, win_goto, win_horz_neighbor, win_splitmove, win_valid,
    win_vert_neighbor,
};
use std::ffi::{CStr, c_char, c_int};
use std::fmt::Write as _;
use std::ptr;

use crate::api::buffer::NUL;
use crate::api::win_config::kCtxNoDisplay;
use crate::api::window::{kCtxKeepCwd, kCtxValidate};
use crate::drawscreen::{FR_LEAF, FR_ROW};
use crate::eval::buffer::varnumber_T;
use crate::eval::{kListLenMayKnow, kListLenUnknown};
use crate::math::{FAIL, OK};
pub type dictitem_T = ScopeDictDictItem;

const E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE: &CStr =
    c"E1308: Cannot resize a window in another tab page";

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct viewstate_T {
    pub vs_curswant: colnr_T,
    pub vs_set_curswant: bool,
    pub vs_leftcol: colnr_T,
    pub vs_skipcol: colnr_T,
    pub vs_topline: linenr_T,
    pub vs_topfill: c_int,
    pub vs_botline: linenr_T,
    pub vs_empty_rows: c_int,
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_has_winnr(wp: *mut win_T, tp: *mut tabpage_T) -> bool {
    unsafe {
        let cur = if tp == curtab {
            curwin
        } else {
            (*tp).tp_curwin
        };
        wp == cur || (!(*wp).w_config.hide && (*wp).w_config.focusable)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_find_nr(mut nr: c_int, mut tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        if tp.is_null() {
            debug_assert!(!curtab.is_null());
            tp = curtab;
        }
        let mut wp = first_win_in_tab(tp);
        while !wp.is_null() {
            nr -= c_int::from(win_has_winnr(wp, tp));
            if nr <= 0 {
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

unsafe fn win_getid(argvars: *mut typval_T) -> c_int {
    unsafe {
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            return (*curwin).handle;
        }
        let mut winnr = tv_get_number(argvars) as c_int;
        if winnr <= 0 {
            return 0;
        }

        let mut tp: *mut tabpage_T = ptr::null_mut();
        let mut wp: *mut win_T;
        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            tp = curtab;
            wp = firstwin;
        } else {
            let mut tabnr = tv_get_number(argvars.add(1)) as c_int;
            let mut tp2 = first_tabpage;
            while !tp2.is_null() {
                tabnr -= 1;
                if tabnr == 0 {
                    tp = tp2;
                    break;
                }
                tp2 = (*tp2).tp_next;
            }
            if tp.is_null() {
                return -1;
            }
            wp = first_win_in_tab(tp);
        }
        while !wp.is_null() {
            winnr -= c_int::from(win_has_winnr(wp, tp));
            if winnr == 0 {
                return (*wp).handle;
            }
            wp = (*wp).w_next;
        }
        0
    }
}

unsafe fn win_id2tabwin(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let id = tv_get_number(argvars) as handle_T;

        let mut winnr: c_int = 1;
        let mut tabnr: c_int = 1;
        win_get_tabwin(id, &raw mut tabnr, &raw mut winnr);

        let list = tv_list_alloc_ret(rettv, 2);
        tv_list_append_number(list, tabnr as varnumber_T);
        tv_list_append_number(list, winnr as varnumber_T);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_id2wp(id: c_int) -> *mut win_T {
    unsafe { win_id2wp_tp(id, ptr::null_mut()) }
}

/// Return the window and tab pointer of window `id` (`win_id2wp_tp`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_id2wp_tp(id: c_int, tpp: *mut *mut tabpage_T) -> *mut win_T {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).handle == id {
                    if !tpp.is_null() {
                        *tpp = tp;
                    }
                    return wp;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        ptr::null_mut()
    }
}

unsafe fn win_id2win(argvars: *mut typval_T) -> c_int {
    unsafe {
        let mut nr: c_int = 1;
        let id = tv_get_number(argvars) as c_int;

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).handle == id {
                return if win_has_winnr(wp, curtab) { nr } else { 0 };
            }
            nr += c_int::from(win_has_winnr(wp, curtab));
            wp = (*wp).w_next;
        }
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_findbuf(argvars: *mut typval_T, list: *mut list_T) {
    unsafe {
        let bufnr = tv_get_number(argvars) as c_int;

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*(*wp).w_buffer).b_fnum == bufnr {
                    tv_list_append_number(list, (*wp).handle as varnumber_T);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// Find window specified by `vp` in tabpage `tp` (`find_win_by_nr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_win_by_nr(vp: *mut typval_T, mut tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        let nr = tv_get_number_chk(vp, ptr::null_mut()) as c_int;

        if nr < 0 {
            return ptr::null_mut();
        }

        if nr == 0 {
            return curwin;
        }

        if tp.is_null() {
            tp = curtab;
        }

        if nr >= LOWEST_WIN_ID {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).handle == nr {
                    return wp;
                }
                wp = (*wp).w_next;
            }
            return ptr::null_mut();
        }

        win_find_nr(nr, tp)
    }
}

/// Find a window: When using a Window ID in any tab page, when using a number
/// in the current tab page (`find_win_by_nr_or_id`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_win_by_nr_or_id(vp: *mut typval_T) -> *mut win_T {
    unsafe {
        let nr = tv_get_number_chk(vp, ptr::null_mut()) as c_int;

        if nr >= LOWEST_WIN_ID {
            return win_id2wp(tv_get_number(vp) as c_int);
        }

        find_win_by_nr(vp, ptr::null_mut())
    }
}

/// Find window specified by `wvp` in tabpage `tvp` (`find_tabwin`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_tabwin(wvp: *mut typval_T, tvp: *mut typval_T) -> *mut win_T {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        let mut tp: *mut tabpage_T = ptr::null_mut();

        if (*wvp).v_type != VarType::VAR_UNKNOWN {
            if (*tvp).v_type != VarType::VAR_UNKNOWN {
                let n = tv_get_number(tvp) as c_int;
                if n >= 0 {
                    tp = find_tabpage(n);
                }
            } else {
                tp = curtab;
            }

            if !tp.is_null() {
                wp = find_win_by_nr(wvp, tp);
            }
        } else {
            wp = curwin;
        }

        wp
    }
}

/// Get the layout of the given tab page for `winlayout()` (`get_framelayout`).
unsafe fn get_framelayout(fr: *const frame_T, l: *mut list_T, outer: bool) {
    unsafe {
        if fr.is_null() {
            return;
        }

        let fr_list = if outer {
            l
        } else {
            let sub = tv_list_alloc(2);
            tv_list_append_list(l, sub);
            sub
        };

        if (*fr).fr_layout == FR_LEAF {
            if !(*fr).fr_win.is_null() {
                tv_list_append_string(fr_list, c"leaf".as_ptr(), 4);
                tv_list_append_number(fr_list, (*(*fr).fr_win).handle as varnumber_T);
            }
        } else {
            if (*fr).fr_layout == FR_ROW {
                tv_list_append_string(fr_list, c"row".as_ptr(), 3);
            } else {
                tv_list_append_string(fr_list, c"col".as_ptr(), 3);
            }

            let win_list = tv_list_alloc(kListLenUnknown);
            tv_list_append_list(fr_list, win_list);
            let mut child: *const frame_T = (*fr).fr_child;
            while !child.is_null() {
                get_framelayout(child, win_list, false);
                child = (*child).fr_next;
            }
        }
    }
}

/// Common code for `tabpagewinnr()` and `winnr()` (`get_winnr`).
unsafe fn get_winnr(tp: *mut tabpage_T, argvar: *mut typval_T) -> c_int {
    unsafe {
        let mut nr: c_int = 1;

        let mut twin = if tp == curtab {
            curwin
        } else {
            (*tp).tp_curwin
        };
        if (*argvar).v_type != VarType::VAR_UNKNOWN {
            let mut invalid_arg = false;
            let arg = tv_get_string_chk(argvar);
            if arg.is_null() {
                nr = 0;
            } else {
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes == b"$" {
                    twin = if tp == curtab {
                        lastwin
                    } else {
                        (*tp).tp_lastwin
                    };
                } else if bytes == b"#" {
                    twin = if tp == curtab {
                        prevwin
                    } else {
                        (*tp).tp_prevwin
                    };
                    if twin.is_null() {
                        nr = 0;
                    }
                } else {
                    let mut endp = arg.cast_mut();
                    let mut count = getdigits_int(&raw mut endp, false, 0);
                    if count <= 0 {
                        count = 1;
                    }
                    if !endp.is_null() && *endp != NUL {
                        let tail = CStr::from_ptr(endp).to_bytes();
                        if tail == b"j" {
                            twin = win_vert_neighbor(tp, twin, false, count);
                        } else if tail == b"k" {
                            twin = win_vert_neighbor(tp, twin, true, count);
                        } else if tail == b"h" {
                            twin = win_horz_neighbor(tp, twin, true, count);
                        } else if tail == b"l" {
                            twin = win_horz_neighbor(tp, twin, false, count);
                        } else {
                            invalid_arg = true;
                        }
                    } else {
                        invalid_arg = true;
                    }
                }
            }

            if invalid_arg {
                semsg!(gettext((&raw const e_invexpr2).cast()), arg);
                nr = 0;
            }
        } else if !win_has_winnr(twin, tp) {
            nr = 0;
        }

        if nr <= 0 {
            return 0;
        }

        nr = 0;
        let mut wp = first_win_in_tab(tp);
        while !wp.is_null() {
            nr += c_int::from(win_has_winnr(wp, tp));
            if wp == twin {
                break;
            }
            wp = (*wp).w_next;
        }
        if wp.is_null() {
            nr = 0;
        }
        nr
    }
}

/// Information about a window as a dictionary (`get_win_info`).
unsafe fn get_win_info(wp: *mut win_T, tpnr: i16, winnr: i16) -> *mut dict_T {
    unsafe {
        let dict = tv_dict_alloc();

        validate_botline_win(wp);

        tv_dict_add_nr(dict, c"tabnr".as_ptr(), 5, tpnr as varnumber_T);
        tv_dict_add_nr(dict, c"winnr".as_ptr(), 5, winnr as varnumber_T);
        tv_dict_add_nr(dict, c"winid".as_ptr(), 5, (*wp).handle as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"height".as_ptr(),
            6,
            (*wp).w_view_height as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"status_height".as_ptr(),
            13,
            (*wp).w_status_height as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"winrow".as_ptr(),
            6,
            ((*wp).w_winrow + 1) as varnumber_T,
        );
        tv_dict_add_nr(dict, c"topline".as_ptr(), 7, (*wp).w_topline as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"botline".as_ptr(),
            7,
            ((*wp).w_botline - 1) as varnumber_T,
        );
        tv_dict_add_nr(dict, c"leftcol".as_ptr(), 7, (*wp).w_leftcol as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"winbar".as_ptr(),
            6,
            (*wp).w_winbar_height as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"width".as_ptr(),
            5,
            (*wp).w_view_width as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"bufnr".as_ptr(),
            5,
            (*(*wp).w_buffer).b_fnum as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"wincol".as_ptr(),
            6,
            ((*wp).w_wincol + 1) as varnumber_T,
        );
        tv_dict_add_nr(dict, c"textoff".as_ptr(), 7, win_col_off(wp) as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"terminal".as_ptr(),
            8,
            varnumber_T::from(bt_terminal((*wp).w_buffer)),
        );
        tv_dict_add_nr(
            dict,
            c"quickfix".as_ptr(),
            8,
            varnumber_T::from(bt_quickfix((*wp).w_buffer)),
        );
        tv_dict_add_nr(
            dict,
            c"loclist".as_ptr(),
            7,
            varnumber_T::from(bt_quickfix((*wp).w_buffer) && !(*wp).w_llist_ref.is_null()),
        );

        tv_dict_add_dict(dict, c"variables".as_ptr(), 9, (*wp).w_vars);

        dict
    }
}

/// Information about a tab page as a dictionary (`get_tabpage_info`).
unsafe fn get_tabpage_info(tp: *mut tabpage_T, tp_idx: c_int) -> *mut dict_T {
    unsafe {
        let dict = tv_dict_alloc();

        tv_dict_add_nr(dict, c"tabnr".as_ptr(), 5, tp_idx as varnumber_T);

        let l = tv_list_alloc(kListLenMayKnow);
        let mut wp = first_win_in_tab(tp);
        while !wp.is_null() {
            tv_list_append_number(l, (*wp).handle as varnumber_T);
            wp = (*wp).w_next;
        }
        tv_dict_add_list(dict, c"windows".as_ptr(), 7, l);

        tv_dict_add_dict(dict, c"variables".as_ptr(), 9, (*tp).tp_vars);

        dict
    }
}

/// `gettabinfo()` function (`f_gettabinfo`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_gettabinfo(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut tparg: *mut tabpage_T = ptr::null_mut();

        tv_list_alloc_ret(
            rettv,
            if (*argvars).v_type == VarType::VAR_UNKNOWN {
                1
            } else {
                kListLenMayKnow
            },
        );

        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            tparg = find_tabpage(tv_get_number_chk(argvars, ptr::null_mut()) as c_int);
            if tparg.is_null() {
                return;
            }
        }

        let mut tpnr: c_int = 0;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            tpnr += 1;
            if !tparg.is_null() && tp != tparg {
                tp = (*tp).tp_next;
                continue;
            }
            let d = get_tabpage_info(tp, tpnr);
            tv_list_append_dict((*rettv).vval.v_list, d);
            if !tparg.is_null() {
                return;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// `getwininfo()` function (`f_getwininfo`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getwininfo(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut wparg: *mut win_T = ptr::null_mut();

        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            wparg = win_id2wp(tv_get_number(argvars) as c_int);
            if wparg.is_null() {
                return;
            }
        }

        let mut tabnr: i16 = 0;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            tabnr = tabnr.wrapping_add(1);
            let mut winnr: i16 = 0;
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                let has_winnr = win_has_winnr(wp, tp);
                winnr = winnr.wrapping_add(i16::from(has_winnr));
                if !wparg.is_null() && wp != wparg {
                    wp = (*wp).w_next;
                    continue;
                }
                let d = get_win_info(wp, tabnr, if has_winnr { winnr } else { 0 });
                tv_list_append_dict((*rettv).vval.v_list, d);
                if !wparg.is_null() {
                    return;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// `getwinpos({timeout})` function (`f_getwinpos`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getwinpos(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);
        tv_list_append_number((*rettv).vval.v_list, -1);
        tv_list_append_number((*rettv).vval.v_list, -1);
    }
}

/// `getwinposx()` function (`f_getwinposx`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getwinposx(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
    }
}

/// `getwinposy()` function (`f_getwinposy`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getwinposy(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
    }
}

/// `tabpagenr()` function (`f_tabpagenr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tabpagenr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut nr: c_int = 0;

        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let arg = tv_get_string_chk(argvars);
            if !arg.is_null() {
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes == b"$" {
                    nr = tabpage_index(ptr::null_mut()) - 1;
                } else if bytes == b"#" {
                    nr = if valid_tabpage(lastused_tabpage) {
                        tabpage_index(lastused_tabpage)
                    } else {
                        0
                    };
                } else {
                    semsg!(gettext((&raw const e_invexpr2).cast()), arg);
                }
            }
        } else {
            nr = tabpage_index(curtab);
        }
        (*rettv).vval.v_number = nr as varnumber_T;
    }
}

/// `tabpagewinnr()` function (`f_tabpagewinnr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tabpagewinnr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let tp = find_tabpage(tv_get_number(argvars) as c_int);
        let nr = if tp.is_null() {
            0
        } else {
            get_winnr(tp, argvars.add(1))
        };
        (*rettv).vval.v_number = nr as varnumber_T;
    }
}

/// `win_execute(win_id, command)` function (`f_win_execute`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_execute(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let id = tv_get_number(argvars) as c_int;
        let mut tp: *mut tabpage_T = ptr::null_mut();
        let wp = win_id2wp_tp(id, &raw mut tp);
        if wp.is_null() || tp.is_null() {
            return;
        }

        let mut cs: CtxSwitch = std::mem::zeroed();
        if ctx_switch(
            &raw mut cs,
            wp,
            tp,
            ptr::null_mut(),
            kCtxNoDisplay | kCtxKeepCwd | kCtxValidate,
        ) {
            execute_common(argvars, rettv, 1);
        }
        ctx_restore(&raw mut cs);
    }
}

/// `win_findbuf()` function (`f_win_findbuf`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_findbuf(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        win_findbuf(argvars, (*rettv).vval.v_list);
    }
}

/// `win_getid()` function (`f_win_getid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_getid(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = win_getid(argvars) as varnumber_T;
    }
}

/// `win_gotoid()` function (`f_win_gotoid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_gotoid(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let id = tv_get_number(argvars) as c_int;
        if (*curwin).handle == id {
            (*rettv).vval.v_number = 1;
            return;
        }

        if text_or_buf_locked() {
            return;
        }
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).handle == id {
                    if Visual.active && (*wp).w_buffer != curbuf {
                        end_visual_mode();
                    }
                    goto_tabpage_win(tp, wp);
                    (*rettv).vval.v_number = 1;
                    return;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// `win_id2tabwin()` function (`f_win_id2tabwin`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_id2tabwin(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        win_id2tabwin(argvars, rettv);
    }
}

/// `win_id2win()` function (`f_win_id2win`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_id2win(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = win_id2win(argvars) as varnumber_T;
    }
}

/// `win_move_separator()` function (`f_win_move_separator`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_move_separator(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = 0;

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() || (*wp).w_floating {
            return;
        }
        if !win_valid(wp) {
            emsg(gettext(E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE.as_ptr()));
            return;
        }

        let offset = tv_get_number(argvars.add(1)) as c_int;
        win_drag_vsep_line(wp, offset);
        (*rettv).vval.v_number = 1;
    }
}

/// `win_move_statusline()` function (`f_win_move_statusline`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_move_statusline(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = 0;

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() || (*wp).w_floating {
            return;
        }
        if !win_valid(wp) {
            emsg(gettext(E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE.as_ptr()));
            return;
        }

        let offset = tv_get_number(argvars.add(1)) as c_int;
        win_drag_status_line(wp, offset);
        (*rettv).vval.v_number = 1;
    }
}

/// `win_screenpos()` function (`f_win_screenpos`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_screenpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);
        let wp = find_win_by_nr_or_id(argvars);
        tv_list_append_number(
            (*rettv).vval.v_list,
            if wp.is_null() {
                0
            } else {
                ((*wp).w_winrow + 1) as varnumber_T
            },
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            if wp.is_null() {
                0
            } else {
                ((*wp).w_wincol + 1) as varnumber_T
            },
        );
    }
}

/// `win_splitmove()` function (`f_win_splitmove`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_splitmove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wp = find_win_by_nr_or_id(argvars);
        let targetwin = find_win_by_nr_or_id(argvars.add(1));
        let oldwin = curwin;

        (*rettv).vval.v_number = -1;

        if wp.is_null()
            || targetwin.is_null()
            || wp == targetwin
            || !win_valid(wp)
            || !win_valid(targetwin)
            || (*targetwin).w_floating
        {
            emsg(gettext((&raw const e_invalwindow).cast()));
            return;
        }

        let mut flags: c_int = 0;
        let mut size: c_int = 0;

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_nonnull_dict_arg(argvars, 2) == FAIL {
                return;
            }

            let d = (*argvars.add(2)).vval.v_dict;
            if tv_dict_get_bool(d, c"vertical".as_ptr(), 0) != 0 {
                flags |= WSP_VERT;
            }
            let di = tv_dict_find(d, c"rightbelow".as_ptr(), -1);
            if !di.is_null() {
                flags |= if tv_get_bool(&raw const (*di).di_tv) != 0 {
                    WSP_BELOW
                } else {
                    WSP_ABOVE
                };
            }
            size = tv_dict_get_number(d, c"size".as_ptr()) as c_int;
        }

        if is_ctx_win(wp) || text_or_buf_locked() || check_split_disallowed(wp) == FAIL {
            return;
        }

        if curwin != targetwin {
            win_goto(targetwin);
        }

        if curwin == targetwin && win_valid(wp) {
            if win_splitmove(wp, size, flags) == OK {
                (*rettv).vval.v_number = 0;
            }
        } else {
            emsg(gettext((&raw const e_auabort).cast()));
        }

        if oldwin != curwin && win_valid(oldwin) {
            win_goto(oldwin);
        }
    }
}

/// `win_gettype(nr)` function (`f_win_gettype`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_win_gettype(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut wp = curwin;

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            wp = find_win_by_nr_or_id(argvars);
            if wp.is_null() {
                (*rettv).vval.v_string = xstrdup(c"unknown".as_ptr());
                return;
            }
        }
        if is_ctx_win(wp) {
            (*rettv).vval.v_string = xstrdup(c"autocmd".as_ptr());
        } else if (*wp).w_onebuf_opt.wo_pvw != 0 {
            (*rettv).vval.v_string = xstrdup(c"preview".as_ptr());
        } else if (*wp).w_floating {
            (*rettv).vval.v_string = xstrdup(c"popup".as_ptr());
        } else if bt_cmdwin((*wp).w_buffer) {
            (*rettv).vval.v_string = xstrdup(c"command".as_ptr());
        } else if bt_quickfix((*wp).w_buffer) {
            (*rettv).vval.v_string = xstrdup(if !(*wp).w_llist_ref.is_null() {
                c"loclist".as_ptr()
            } else {
                c"quickfix".as_ptr()
            });
        }
    }
}

/// `getcmdwintype()` function (`f_getcmdwintype`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcmdwintype(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let s: *mut c_char = xmallocz(1).cast();
        *s = cmdwin_type as c_char;
        (*rettv).vval.v_string = s;
    }
}

/// `winbufnr(nr)` function (`f_winbufnr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winbufnr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wp = find_win_by_nr_or_id(argvars);
        (*rettv).vval.v_number = if wp.is_null() {
            -1
        } else {
            (*(*wp).w_buffer).b_fnum as varnumber_T
        };
    }
}

/// `wincol()` function (`f_wincol`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_wincol(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        validate_cursor(curwin);
        (*rettv).vval.v_number = ((*curwin).w_wcol + 1) as varnumber_T;
    }
}

/// `winheight(nr)` function (`f_winheight`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winheight(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wp = find_win_by_nr_or_id(argvars);
        (*rettv).vval.v_number = if wp.is_null() {
            -1
        } else {
            (*wp).w_view_height as varnumber_T
        };
    }
}

/// `winlayout()` function (`f_winlayout`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winlayout(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);

        let tp = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            curtab
        } else {
            let found = find_tabpage(tv_get_number(argvars) as c_int);
            if found.is_null() {
                return;
            }
            found
        };

        get_framelayout((*tp).tp_topframe.cast(), (*rettv).vval.v_list, true);
    }
}

/// `winline()` function (`f_winline`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winline(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        validate_cursor(curwin);
        (*rettv).vval.v_number = ((*curwin).w_wrow + 1) as varnumber_T;
    }
}

/// `winnr()` function (`f_winnr`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winnr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = get_winnr(curtab, argvars) as varnumber_T;
    }
}

/// `winrestcmd()` function (`f_winrestcmd`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winrestcmd(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut ga, size_of::<c_char>() as c_int, 70);

        let mut buf = String::with_capacity(50);
        for _ in 0..2 {
            let mut winnr: c_int = 1;
            let mut wp = firstwin;
            while !wp.is_null() {
                if !win_has_winnr(wp, curtab) {
                    wp = (*wp).w_next;
                    continue;
                }
                buf.clear();
                let _ = write!(&mut buf, ":{winnr}resize {}|", (*wp).w_height);
                ga_concat_len(&raw mut ga, buf.as_ptr().cast(), buf.len());
                buf.clear();
                let _ = write!(&mut buf, "vert :{winnr}resize {}|", (*wp).w_width);
                ga_concat_len(&raw mut ga, buf.as_ptr().cast(), buf.len());
                winnr += 1;
                wp = (*wp).w_next;
            }
        }
        ga_append(&raw mut ga, NUL as u8);

        (*rettv).vval.v_string = ga.ga_data.cast();
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// `winrestview()` function (`f_winrestview`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winrestview(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_nonnull_dict_arg(argvars, 0) == FAIL {
            return;
        }

        let dict = (*argvars).vval.v_dict;
        let di = tv_dict_find(dict, c"lnum".as_ptr(), 4);
        if !di.is_null() {
            (*curwin).w_cursor.lnum = tv_get_number(&raw const (*di).di_tv) as linenr_T;
        }
        let di = tv_dict_find(dict, c"col".as_ptr(), 3);
        if !di.is_null() {
            (*curwin).w_cursor.col = tv_get_number(&raw const (*di).di_tv) as colnr_T;
        }
        let di = tv_dict_find(dict, c"coladd".as_ptr(), 6);
        if !di.is_null() {
            (*curwin).w_cursor.coladd = tv_get_number(&raw const (*di).di_tv) as colnr_T;
        }
        let di = tv_dict_find(dict, c"topline".as_ptr(), 7);
        if !di.is_null() {
            set_topline(curwin, tv_get_number(&raw const (*di).di_tv) as linenr_T);
        }
        let mut vs = viewstate_T::default();
        save_viewstate(curwin, &raw mut vs);
        let di = tv_dict_find(dict, c"curswant".as_ptr(), 8);
        if !di.is_null() {
            vs.vs_curswant = tv_get_number(&raw const (*di).di_tv) as colnr_T;
            vs.vs_set_curswant = false;
        }
        let di = tv_dict_find(dict, c"topfill".as_ptr(), 7);
        if !di.is_null() {
            vs.vs_topfill = tv_get_number(&raw const (*di).di_tv) as c_int;
        }
        let di = tv_dict_find(dict, c"leftcol".as_ptr(), 7);
        if !di.is_null() {
            vs.vs_leftcol = tv_get_number(&raw const (*di).di_tv) as colnr_T;
        }
        let di = tv_dict_find(dict, c"skipcol".as_ptr(), 7);
        if !di.is_null() {
            vs.vs_skipcol = tv_get_number(&raw const (*di).di_tv) as colnr_T;
        }
        restore_viewstate(curwin, &raw const vs);

        check_cursor(curwin);
        changed_window_setting(curwin);
        if (*curwin).w_topline <= 0 {
            (*curwin).w_topline = 1;
        }
        if (*curwin).w_topline > (*curbuf).b_ml.ml_line_count {
            (*curwin).w_topline = (*curbuf).b_ml.ml_line_count;
        }
        check_topfill(curwin, true);
    }
}

/// `winsaveview()` function (`f_winsaveview`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winsaveview(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict;

        tv_dict_add_nr(
            dict,
            c"lnum".as_ptr(),
            4,
            (*curwin).w_cursor.lnum as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"col".as_ptr(),
            3,
            (*curwin).w_cursor.col as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"coladd".as_ptr(),
            6,
            (*curwin).w_cursor.coladd as varnumber_T,
        );
        update_curswant();
        let mut vs = viewstate_T::default();
        save_viewstate(curwin, &raw mut vs);
        tv_dict_add_nr(dict, c"curswant".as_ptr(), 8, vs.vs_curswant as varnumber_T);

        tv_dict_add_nr(dict, c"topline".as_ptr(), 7, vs.vs_topline as varnumber_T);
        tv_dict_add_nr(dict, c"topfill".as_ptr(), 7, vs.vs_topfill as varnumber_T);
        tv_dict_add_nr(dict, c"leftcol".as_ptr(), 7, vs.vs_leftcol as varnumber_T);
        tv_dict_add_nr(dict, c"skipcol".as_ptr(), 7, vs.vs_skipcol as varnumber_T);
    }
}

/// `winwidth(nr)` function (`f_winwidth`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_winwidth(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wp = find_win_by_nr_or_id(argvars);
        (*rettv).vval.v_number = if wp.is_null() {
            -1
        } else {
            (*wp).w_view_width as varnumber_T
        };
    }
}
