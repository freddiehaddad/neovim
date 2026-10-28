//! Window related builtin functions (`src/nvim/eval/window.c`).

use crate::buffer::{bt_cmdwin, bt_quickfix, bt_terminal, first_win_in_tab};
use crate::charset::getdigits_int;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::cursor::{Colnr, Linenr, check_cursor};
use crate::errors::*;
use crate::eval::funcs::execute_common;
use crate::eval::typval::{
    EvalFuncData, TvDict, TvList, TypVal, VarType, Varnumber, tv_check_for_nonnull_dict_arg,
    tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr, tv_dict_alloc, tv_dict_alloc_ret,
    tv_dict_find, tv_dict_get_bool, tv_dict_get_number, tv_get_bool, tv_get_number,
    tv_get_number_chk, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_list, tv_list_append_number, tv_list_append_string, tv_set_string,
    tv_set_string_len,
};
use crate::ex_getln::text_or_buf_locked;
use crate::globals::*;
use crate::message::{emsg, semsg};
use crate::r#move::{
    changed_window_setting, check_topfill, restore_viewstate, save_viewstate, set_topline,
    update_curswant, validate_botline_win, validate_cursor, win_col_off,
};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::end_visual_mode;
use crate::strings::gettext;
use crate::window::{
    Frame, LOWEST_WIN_ID, TabPageInfo, WSP_ABOVE, WSP_BELOW, WSP_VERT, WinInfoInfo,
    check_split_disallowed, find_tabpage, goto_tabpage_win, tabpage_index, valid_tabpage,
    win_drag_status_line, win_drag_vsep_line, win_get_tabwin, win_goto, win_horz_neighbor,
    win_splitmove, win_valid, win_vert_neighbor,
};
use std::ffi::{CStr, c_char};
use std::fmt::Write as _;
use std::ptr;

use crate::api::win_config::K_CTX_NO_DISPLAY;
use crate::api::window::{K_CTX_KEEP_CWD, K_CTX_VALIDATE};
use crate::drawscreen::{FR_LEAF, FR_ROW};
use crate::eval::{K_LIST_LEN_MAY_KNOW_CONST, K_LIST_LEN_UNKNOWN_CONST};

const E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE: &CStr =
    c"E1308: Cannot resize a window in another tab page";

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Viewstate {
    pub vs_curswant: Colnr,
    pub vs_set_curswant: bool,
    pub vs_leftcol: Colnr,
    pub vs_skipcol: Colnr,
    pub vs_topline: Linenr,
    pub vs_topfill: i32,
    pub vs_botline: Linenr,
    pub vs_empty_rows: i32,
}

impl Viewstate {
    pub const fn new() -> Self {
        Self {
            vs_curswant: 0,
            vs_set_curswant: false,
            vs_leftcol: 0,
            vs_skipcol: 0,
            vs_topline: 0,
            vs_topfill: 0,
            vs_botline: 0,
            vs_empty_rows: 0,
        }
    }
}

pub fn win_has_winnr(wp: &mut WinInfoInfo, tp: &mut TabPageInfo) -> bool {
    let cur = if ptr::eq((&raw mut *tp), g_st().curtab) {
        curwin_ptr()
    } else {
        tp.tp_curwin
    };
    ptr::eq(wp, cur) || (!wp.w_config.hide && wp.w_config.focusable)
}

pub fn win_find_nr(mut nr: i32, mut tp: *mut TabPageInfo) -> *mut WinInfoInfo {
    if tp.is_null() {
        debug_assert!(!g_st().curtab.is_null());
        tp = g_st().curtab;
    }
    for wp in win_iter_ptr(first_win_in_tab(unsafe { &mut *tp })) {
        nr -= i32::from(win_has_winnr(&mut *wp, unsafe { &mut *tp }));
        if nr <= 0 {
            return wp;
        }
    }
    ptr::null_mut()
}

fn win_getid(argvars: &mut [TypVal]) -> i32 {
    if argvars[0].v_type == VarType::VarUnknown {
        return curwin_mut().handle;
    }
    let mut winnr = tv_get_number(&argvars[0]) as i32;
    if winnr <= 0 {
        return 0;
    }

    let mut tp: *mut TabPageInfo = ptr::null_mut();
    let mut wp: *mut WinInfoInfo;
    if argvars[1].v_type == VarType::VarUnknown {
        tp = g_st().curtab;
        wp = g_st().firstwin;
    } else {
        let mut tabnr = tv_get_number(&argvars[1]) as i32;
        for tp2 in for_all_tabpages_ptr() {
            tabnr -= 1;
            if tabnr == 0 {
                tp = tp2;
                break;
            }
        }
        if tp.is_null() {
            return -1;
        }
        wp = first_win_in_tab(unsafe { &mut *tp });
    }
    while !wp.is_null() {
        winnr -= i32::from(win_has_winnr(unsafe { &mut *wp }, unsafe { &mut *tp }));
        if winnr == 0 {
            return unsafe { (*wp).handle };
        }
        wp = unsafe { (*wp).w_next };
    }
    0
}

fn win_id2tabwin(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let id = tv_get_number(&argvars[0]) as ObjHandle;

    let mut winnr: i32 = 1;
    let mut tabnr: i32 = 1;
    win_get_tabwin(id, &mut tabnr, &mut winnr);

    let list = tv_list_alloc_ret(rettv, 2);
    tv_list_append_number(&mut *list, tabnr as Varnumber);
    tv_list_append_number(&mut *list, winnr as Varnumber);
}

pub fn win_id2wp(id: i32) -> *mut WinInfoInfo {
    win_id2wp_tp(id, ptr::null_mut())
}

/// Return the window and tab pointer of window `id` (`win_id2wp_tp`).
pub fn win_id2wp_tp(id: i32, tpp: *mut *mut TabPageInfo) -> *mut WinInfoInfo {
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
            if wp.handle == id {
                if !tpp.is_null() {
                    unsafe {
                        *tpp = tp;
                    }
                }
                return wp;
            }
        }
    }
    ptr::null_mut()
}

fn win_id2win(argvars: &mut [TypVal]) -> i32 {
    let mut nr: i32 = 1;
    let id = tv_get_number(&argvars[0]) as i32;

    for wp in for_all_windows_mut() {
        if wp.handle == id {
            return if win_has_winnr(wp, curtab_mut()) {
                nr
            } else {
                0
            };
        }
        nr += i32::from(win_has_winnr(wp, curtab_mut()));
    }
    0
}

pub fn win_findbuf(argvars: &mut [TypVal], list: &mut TvList) {
    let bufnr = tv_get_number(&argvars[0]) as i32;

    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_mut(first_win_in_tab(&mut *tp)) {
            if wp.buf_mut().b_fnum == bufnr {
                tv_list_append_number(list, wp.handle as Varnumber);
            }
        }
    }
}

/// Find window specified by `vp` in tabpage `tp` (`find_win_by_nr`).
pub fn find_win_by_nr(vp: &TypVal, mut tp: *mut TabPageInfo) -> *mut WinInfoInfo {
    let nr = tv_get_number_chk(vp, None) as i32;

    if nr < 0 {
        return ptr::null_mut();
    }

    if nr == 0 {
        return curwin_ptr();
    }

    if tp.is_null() {
        tp = g_st().curtab;
    }

    if nr >= LOWEST_WIN_ID {
        for wp in win_iter_ptr(first_win_in_tab(unsafe { &mut *tp })) {
            if wp.handle == nr {
                return wp;
            }
        }
        return ptr::null_mut();
    }

    win_find_nr(nr, tp)
}

/// Find a window: When using a Window ID in any tab page, when using a number
/// in the current tab page (`find_win_by_nr_or_id`).
pub fn find_win_by_nr_or_id(vp: &TypVal) -> *mut WinInfoInfo {
    let nr = tv_get_number_chk(vp, None) as i32;

    if nr >= LOWEST_WIN_ID {
        return win_id2wp(tv_get_number(vp) as i32);
    }

    find_win_by_nr(vp, ptr::null_mut())
}

/// Find window specified by `wvp` in tabpage `tvp` (`find_tabwin`).
pub fn find_tabwin(wvp: &TypVal, tvp: &TypVal) -> *mut WinInfoInfo {
    let mut wp: *mut WinInfoInfo = ptr::null_mut();
    let mut tp: *mut TabPageInfo = ptr::null_mut();

    if wvp.v_type != VarType::VarUnknown {
        if tvp.v_type != VarType::VarUnknown {
            let n = tv_get_number(tvp) as i32;
            if n >= 0 {
                tp = find_tabpage(n);
            }
        } else {
            tp = g_st().curtab;
        }

        if !tp.is_null() {
            wp = find_win_by_nr(wvp, tp);
        }
    } else {
        wp = curwin_ptr();
    }

    wp
}

/// Get the layout of the given tab page for `winlayout()` (`get_framelayout`).
fn get_framelayout(fr: Option<&Frame>, l: &mut TvList, outer: bool) {
    if fr.is_none() {
        return;
    }

    let fr_list = if outer {
        (&raw mut *l)
    } else {
        let sub = tv_list_alloc(2);
        tv_list_append_list(l, sub);
        sub
    };

    if fr.unwrap().fr_layout == FR_LEAF {
        if !fr.unwrap().fr_win.is_null() {
            tv_list_append_string(unsafe { &mut *fr_list }, c"leaf", 4);
            tv_list_append_number(
                unsafe { &mut *fr_list },
                unsafe { (*fr.unwrap().fr_win).handle } as Varnumber,
            );
        }
    } else {
        if fr.unwrap().fr_layout == FR_ROW {
            tv_list_append_string(unsafe { &mut *fr_list }, c"row", 3);
        } else {
            tv_list_append_string(unsafe { &mut *fr_list }, c"col", 3);
        }

        let win_list = tv_list_alloc(K_LIST_LEN_UNKNOWN_CONST);
        tv_list_append_list(unsafe { &mut *fr_list }, win_list);
        for child in frame_iter_ptr(fr.unwrap().fr_child) {
            get_framelayout(Some(&*child), &mut *win_list, false);
        }
    }
}

/// Common code for `tabpagewinnr()` and `winnr()` (`get_winnr`).
fn get_winnr(tp: &mut TabPageInfo, argvar: &mut TypVal) -> i32 {
    let mut nr: i32 = 1;

    let mut twin = if ptr::eq(tp, g_st().curtab) {
        curwin_ptr()
    } else {
        tp.tp_curwin
    };
    if argvar.v_type != VarType::VarUnknown {
        let mut invalid_arg = false;
        let arg = tv_get_string_chk(&*argvar);
        if arg.is_null() {
            nr = 0;
        } else {
            let bytes = crate::memory::cstr_as_bytes(arg);
            if bytes == b"$" {
                twin = if ptr::eq(tp, g_st().curtab) {
                    g_st().lastwin
                } else {
                    tp.tp_lastwin
                };
            } else if bytes == b"#" {
                twin = if ptr::eq(tp, g_st().curtab) {
                    g_st().prevwin
                } else {
                    tp.tp_prevwin
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
                if !crate::memory::cstr_is_empty(endp) {
                    let tail = crate::memory::cstr_as_bytes(endp);
                    if tail == b"j" {
                        twin = win_vert_neighbor(tp, unsafe { &mut *twin }, false, count);
                    } else if tail == b"k" {
                        twin = win_vert_neighbor(tp, unsafe { &mut *twin }, true, count);
                    } else if tail == b"h" {
                        twin = win_horz_neighbor(tp, unsafe { &mut *twin }, true, count);
                    } else if tail == b"l" {
                        twin = win_horz_neighbor(tp, unsafe { &mut *twin }, false, count);
                    } else {
                        invalid_arg = true;
                    }
                } else {
                    invalid_arg = true;
                }
            }
        }

        if invalid_arg {
            semsg!(gettext(E_INVEXPR2), arg);
            nr = 0;
        }
    } else if !win_has_winnr(unsafe { &mut *twin }, tp) {
        nr = 0;
    }

    if nr <= 0 {
        return 0;
    }

    nr = 0;
    let mut wp: *mut WinInfoInfo = first_win_in_tab(tp);
    while !wp.is_null() {
        nr += i32::from(win_has_winnr(unsafe { &mut *wp }, tp));
        if wp == twin {
            break;
        }
        wp = unsafe { (*wp).w_next };
    }
    if wp.is_null() {
        nr = 0;
    }
    nr
}

/// Information about a window as a dictionary (`get_win_info`).
fn get_win_info(wp: &mut WinInfoInfo, tpnr: i16, winnr: i16) -> &'static mut TvDict {
    let dict = tv_dict_alloc();

    validate_botline_win(&mut *wp);

    tv_dict_add_nr(&mut *dict, c"tabnr", tpnr as Varnumber);
    tv_dict_add_nr(&mut *dict, c"winnr", winnr as Varnumber);
    tv_dict_add_nr(&mut *dict, c"winid", wp.handle as Varnumber);
    tv_dict_add_nr(&mut *dict, c"height", wp.w_view_height as Varnumber);
    tv_dict_add_nr(
        &mut *dict,
        c"status_height",
        wp.w_status_height as Varnumber,
    );
    tv_dict_add_nr(&mut *dict, c"winrow", (wp.w_winrow + 1) as Varnumber);
    tv_dict_add_nr(&mut *dict, c"topline", wp.w_topline as Varnumber);
    tv_dict_add_nr(&mut *dict, c"botline", (wp.w_botline - 1) as Varnumber);
    tv_dict_add_nr(&mut *dict, c"leftcol", wp.w_leftcol as Varnumber);
    tv_dict_add_nr(&mut *dict, c"winbar", wp.w_winbar_height as Varnumber);
    tv_dict_add_nr(&mut *dict, c"width", wp.w_view_width as Varnumber);
    tv_dict_add_nr(&mut *dict, c"bufnr", wp.buf_mut().b_fnum as Varnumber);
    tv_dict_add_nr(&mut *dict, c"wincol", (wp.w_wincol + 1) as Varnumber);
    tv_dict_add_nr(
        &mut *dict,
        c"textoff",
        win_col_off(&raw mut *wp) as Varnumber,
    );
    tv_dict_add_nr(
        &mut *dict,
        c"terminal",
        Varnumber::from(bt_terminal(unsafe { wp.w_buffer.as_ref() })),
    );
    tv_dict_add_nr(
        &mut *dict,
        c"quickfix",
        Varnumber::from(bt_quickfix(unsafe { wp.w_buffer.as_ref() })),
    );
    tv_dict_add_nr(
        &mut *dict,
        c"loclist",
        Varnumber::from(bt_quickfix(unsafe { wp.w_buffer.as_ref() }) && !wp.w_llist_ref.is_null()),
    );

    tv_dict_add_dict(&mut *dict, c"variables", wp.w_vars());

    dict
}

/// Information about a tab page as a dictionary (`get_tabpage_info`).
fn get_tabpage_info(tp: &mut TabPageInfo, tp_idx: i32) -> &'static mut TvDict {
    let dict = tv_dict_alloc();

    tv_dict_add_nr(&mut *dict, c"tabnr", tp_idx as Varnumber);

    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    for wp in win_iter_mut(first_win_in_tab(tp)) {
        tv_list_append_number(&mut *l, wp.handle as Varnumber);
    }
    tv_dict_add_list(&mut *dict, c"windows", l);

    tv_dict_add_dict(&mut *dict, c"variables", tp.tp_vars());

    dict
}

/// `gettabinfo()` function (`f_gettabinfo`).
pub fn f_gettabinfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut tparg: *mut TabPageInfo = ptr::null_mut();

    tv_list_alloc_ret(
        rettv,
        if argvars[0].v_type == VarType::VarUnknown {
            1
        } else {
            K_LIST_LEN_MAY_KNOW_CONST
        },
    );

    if argvars[0].v_type != VarType::VarUnknown {
        tparg = find_tabpage(tv_get_number_chk(&argvars[0], None) as i32);
        if tparg.is_null() {
            return;
        }
    }

    let mut tpnr: i32 = 0;
    for tp in for_all_tabpages_ptr() {
        tpnr += 1;
        if !tparg.is_null() && !std::ptr::eq(tp, tparg) {
            continue;
        }
        let d = get_tabpage_info(&mut *tp, tpnr);
        tv_list_append_dict(rettv.vval.v_list_ref(), d);
        if !tparg.is_null() {
            return;
        }
    }
}

/// `getwininfo()` function (`f_getwininfo`).
pub fn f_getwininfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut wparg: *mut WinInfoInfo = ptr::null_mut();

    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

    if argvars[0].v_type != VarType::VarUnknown {
        wparg = win_id2wp(tv_get_number(&argvars[0]) as i32);
        if wparg.is_null() {
            return;
        }
    }

    let mut tabnr: i16 = 0;
    for tp in for_all_tabpages_ptr() {
        tabnr = tabnr.wrapping_add(1);
        let mut winnr: i16 = 0;
        for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
            let has_winnr = win_has_winnr(&mut *wp, &mut *tp);
            winnr = winnr.wrapping_add(i16::from(has_winnr));
            if !wparg.is_null() && !std::ptr::eq(wp, wparg) {
                continue;
            }
            let d = get_win_info(&mut *wp, tabnr, if has_winnr { winnr } else { 0 });
            tv_list_append_dict(rettv.vval.v_list_ref(), d);
            if !wparg.is_null() {
                return;
            }
        }
    }
}

/// `getwinpos({timeout})` function (`f_getwinpos`).
pub fn f_getwinpos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, 2);
    tv_list_append_number(rettv.vval.v_list_ref(), -1);
    tv_list_append_number(rettv.vval.v_list_ref(), -1);
}

/// `getwinposx()` function (`f_getwinposx`).
pub fn f_getwinposx(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
}

/// `getwinposy()` function (`f_getwinposy`).
pub fn f_getwinposy(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
}

/// `tabpagenr()` function (`f_tabpagenr`).
pub fn f_tabpagenr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut nr: i32 = 0;

    if argvars[0].v_type != VarType::VarUnknown {
        let arg = tv_get_string_chk(&argvars[0]);
        if !arg.is_null() {
            let bytes = crate::memory::cstr_as_bytes(arg);
            if bytes == b"$" {
                nr = tabpage_index(ptr::null_mut()) - 1;
            } else if bytes == b"#" {
                nr = if valid_tabpage(g_st().lastused_tabpage) {
                    tabpage_index(g_st().lastused_tabpage)
                } else {
                    0
                };
            } else {
                semsg!(gettext(E_INVEXPR2), arg);
            }
        }
    } else {
        nr = tabpage_index(g_st().curtab);
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(nr as Varnumber);
}

/// `tabpagewinnr()` function (`f_tabpagewinnr`).
pub fn f_tabpagewinnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let tp = find_tabpage(tv_get_number(&argvars[0]) as i32);
    let nr = if tp.is_null() {
        0
    } else {
        get_winnr(unsafe { &mut *tp }, &mut argvars[1])
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(nr as Varnumber);
}

/// `win_execute(win_id, command)` function (`f_win_execute`).
pub fn f_win_execute(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    let id = tv_get_number(&argvars[0]) as i32;
    let mut tp: *mut TabPageInfo = ptr::null_mut();
    let wp = win_id2wp_tp(id, &raw mut tp);
    if wp.is_null() || tp.is_null() {
        return;
    }

    let mut cs: CtxSwitch = CtxSwitch::new();
    if ctx_switch(
        &mut cs,
        wp,
        tp,
        ptr::null_mut(),
        K_CTX_NO_DISPLAY | K_CTX_KEEP_CWD | K_CTX_VALIDATE,
    ) {
        execute_common(argvars, rettv, 1);
    }
    ctx_restore(&mut cs);
}

/// `win_findbuf()` function (`f_win_findbuf`).
pub fn f_win_findbuf(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
    win_findbuf(argvars, rettv.vval.v_list_ref());
}

/// `win_getid()` function (`f_win_getid`).
pub fn f_win_getid(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(win_getid(argvars) as Varnumber);
}

/// `win_gotoid()` function (`f_win_gotoid`).
pub fn f_win_gotoid(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let id = tv_get_number(&argvars[0]) as i32;
    if curwin_mut().handle == id {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        return;
    }

    if text_or_buf_locked() {
        return;
    }
    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
            if wp.handle == id {
                if g_st().visual_global.active && wp.w_buffer != g_st().curbuf {
                    end_visual_mode();
                }
                goto_tabpage_win(&mut *tp, wp);
                rettv.vval = crate::eval::typval::TypvalVval::Number(1);
                return;
            }
        }
    }
}

/// `win_id2tabwin()` function (`f_win_id2tabwin`).
pub fn f_win_id2tabwin(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    win_id2tabwin(argvars, &mut *rettv);
}

/// `win_id2win()` function (`f_win_id2win`).
pub fn f_win_id2win(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(win_id2win(argvars) as Varnumber);
}

/// `win_move_separator()` function (`f_win_move_separator`).
pub fn f_win_move_separator(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    let wp = find_win_by_nr_or_id(&argvars[0]);
    if wp.is_null() || unsafe { (*wp).w_floating } {
        return;
    }
    if !win_valid(wp) {
        emsg(gettext(E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE));
        return;
    }

    let offset = tv_get_number(&argvars[1]) as i32;
    win_drag_vsep_line(unsafe { &mut *wp }, offset);
    rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// `win_move_statusline()` function (`f_win_move_statusline`).
pub fn f_win_move_statusline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    let wp = find_win_by_nr_or_id(&argvars[0]);
    if wp.is_null() || unsafe { (*wp).w_floating } {
        return;
    }
    if !win_valid(wp) {
        emsg(gettext(E_CANNOT_RESIZE_WINDOW_IN_ANOTHER_TAB_PAGE));
        return;
    }

    let offset = tv_get_number(&argvars[1]) as i32;
    win_drag_status_line(unsafe { &mut *wp }, offset);
    rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// `win_screenpos()` function (`f_win_screenpos`).
pub fn f_win_screenpos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, 2);
    let wp = find_win_by_nr_or_id(&argvars[0]);
    tv_list_append_number(
        rettv.vval.v_list_ref(),
        if wp.is_null() {
            0
        } else {
            (unsafe { (*wp).w_winrow } + 1) as Varnumber
        },
    );
    tv_list_append_number(
        rettv.vval.v_list_ref(),
        if wp.is_null() {
            0
        } else {
            (unsafe { (*wp).w_wincol } + 1) as Varnumber
        },
    );
}

/// `win_splitmove()` function (`f_win_splitmove`).
pub fn f_win_splitmove(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let wp = find_win_by_nr_or_id(&argvars[0]);
    let targetwin = find_win_by_nr_or_id(&argvars[1]);
    let oldwin = curwin_ptr();

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if wp.is_null()
        || targetwin.is_null()
        || wp == targetwin
        || !win_valid(wp)
        || !win_valid(targetwin)
        || unsafe { (*targetwin).w_floating }
    {
        emsg(gettext(E_INVALWINDOW));
        return;
    }

    let mut flags: i32 = 0;
    let mut size: i32 = 0;

    if argvars[2].v_type != VarType::VarUnknown {
        if !tv_check_for_nonnull_dict_arg(argvars, 2) {
            return;
        }

        let d = argvars[2].vval.v_dict();
        if tv_dict_get_bool(unsafe { d.as_ref() }, c"vertical", 0) != 0 {
            flags |= WSP_VERT;
        }
        let di = tv_dict_find(unsafe { d.as_ref() }, c"rightbelow", -1);
        if !di.is_null() {
            flags |= if tv_get_bool(unsafe { &(*di).di_tv }) != 0 {
                WSP_BELOW
            } else {
                WSP_ABOVE
            };
        }
        size = tv_dict_get_number(unsafe { d.as_ref() }, c"size") as i32;
    }

    if is_ctx_win(wp) || text_or_buf_locked() || !check_split_disallowed(wp) {
        return;
    }

    if curwin_ptr() != targetwin {
        win_goto(targetwin);
    }

    if curwin_ptr() == targetwin && win_valid(wp) {
        if win_splitmove(wp, size, flags) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    } else {
        emsg(gettext(E_AUABORT));
    }

    if oldwin != curwin_ptr() && win_valid(oldwin) {
        win_goto(oldwin);
    }
}

/// `win_gettype(nr)` function (`f_win_gettype`).
pub fn f_win_gettype(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut wp = curwin_ptr();

        tv_set_string(rettv, ptr::null());
        if argvars[0].v_type != VarType::VarUnknown {
            wp = find_win_by_nr_or_id(&argvars[0]);
            if wp.is_null() {
                tv_set_string(rettv, c"unknown");
                return;
            }
        }
        if is_ctx_win(wp) {
            tv_set_string(rettv, c"autocmd");
        } else if (*wp).w_onebuf_opt.wo_pvw != 0 {
            tv_set_string(rettv, c"preview");
        } else if (*wp).w_floating {
            tv_set_string(rettv, c"popup");
        } else if bt_cmdwin((*wp).w_buffer) {
            tv_set_string(rettv, c"command");
        } else if bt_quickfix(((*wp).w_buffer).as_ref()) {
            tv_set_string(
                rettv,
                if !(*wp).w_llist_ref.is_null() {
                    c"loclist".as_ptr()
                } else {
                    c"quickfix".as_ptr()
                },
            );
        }
    }
}

/// `getcmdwintype()` function (`f_getcmdwintype`).
pub fn f_getcmdwintype(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let buf = [g_st().cmdwin_type as c_char, 0];
    tv_set_string(rettv, buf.as_ptr());
}

/// `winbufnr(nr)` function (`f_winbufnr`).
pub fn f_winbufnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let wp = find_win_by_nr_or_id(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(if wp.is_null() {
            -1
        } else {
            (*wp).buf_mut().b_fnum as Varnumber
        });
    }
}

/// `wincol()` function (`f_wincol`).
pub fn f_wincol(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    validate_cursor(curwin_mut());
    rettv.vval = crate::eval::typval::TypvalVval::Number((curwin_mut().w_wcol + 1) as Varnumber);
}

/// `winheight(nr)` function (`f_winheight`).
pub fn f_winheight(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let wp = find_win_by_nr_or_id(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(if wp.is_null() {
            -1
        } else {
            (*wp).w_view_height as Varnumber
        });
    }
}

/// `winlayout()` function (`f_winlayout`).
pub fn f_winlayout(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);

        let tp = if argvars[0].v_type == VarType::VarUnknown {
            g_st().curtab
        } else {
            let found = find_tabpage(tv_get_number(&argvars[0]) as i32);
            if found.is_null() {
                return;
            }
            found
        };

        get_framelayout((*tp).tp_topframe.as_ref(), rettv.vval.v_list_ref(), true);
    }
}

/// `winline()` function (`f_winline`).
pub fn f_winline(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    validate_cursor(curwin_mut());
    rettv.vval = crate::eval::typval::TypvalVval::Number((curwin_mut().w_wrow + 1) as Varnumber);
}

/// `winnr()` function (`f_winnr`).
pub fn f_winnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(
        get_winnr(curtab_mut(), &mut argvars[0]) as Varnumber
    );
}

/// `winrestcmd()` function (`f_winrestcmd`).
pub fn f_winrestcmd(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = String::with_capacity(70);
    for _ in 0..2 {
        let mut winnr: i32 = 1;
        for wp in for_all_windows_mut() {
            if !win_has_winnr(wp, curtab_mut()) {
                continue;
            }
            let _ = write!(
                &mut buf,
                ":{winnr}resize {}|vert :{winnr}resize {}|",
                wp.w_height, wp.w_width
            );
            winnr += 1;
        }
    }

    tv_set_string_len(rettv, buf.as_ptr().cast::<c_char>(), buf.len());
}

/// `winrestview()` function (`f_winrestview`).
pub fn f_winrestview(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        if !tv_check_for_nonnull_dict_arg(argvars, 0) {
            return;
        }

        let dict = argvars[0].vval.v_dict();
        let di = tv_dict_find(dict.as_ref(), c"lnum", 4);
        if !di.is_null() {
            curwin_mut().w_cursor.lnum = tv_get_number(&(*di).di_tv) as Linenr;
        }
        let di = tv_dict_find(dict.as_ref(), c"col", 3);
        if !di.is_null() {
            curwin_mut().w_cursor.col = tv_get_number(&(*di).di_tv) as Colnr;
        }
        let di = tv_dict_find(dict.as_ref(), c"coladd", 6);
        if !di.is_null() {
            curwin_mut().w_cursor.coladd = tv_get_number(&(*di).di_tv) as Colnr;
        }
        let di = tv_dict_find(dict.as_ref(), c"topline", 7);
        if !di.is_null() {
            set_topline(curwin_mut(), tv_get_number(&(*di).di_tv) as Linenr);
        }
        let mut vs = Viewstate::default();
        save_viewstate(curwin_mut(), &mut vs);
        let di = tv_dict_find(dict.as_ref(), c"curswant", 8);
        if !di.is_null() {
            vs.vs_curswant = tv_get_number(&(*di).di_tv) as Colnr;
            vs.vs_set_curswant = false;
        }
        let di = tv_dict_find(dict.as_ref(), c"topfill", 7);
        if !di.is_null() {
            vs.vs_topfill = tv_get_number(&(*di).di_tv) as i32;
        }
        let di = tv_dict_find(dict.as_ref(), c"leftcol", 7);
        if !di.is_null() {
            vs.vs_leftcol = tv_get_number(&(*di).di_tv) as Colnr;
        }
        let di = tv_dict_find(dict.as_ref(), c"skipcol", 7);
        if !di.is_null() {
            vs.vs_skipcol = tv_get_number(&(*di).di_tv) as Colnr;
        }
        restore_viewstate(curwin_mut(), &vs);

        check_cursor(curwin_mut());
        changed_window_setting(curwin_mut());
        if curwin_mut().w_topline <= 0 {
            curwin_mut().w_topline = 1;
        }
        if curwin_mut().w_topline > curbuf_mut().b_ml.ml_line_count {
            curwin_mut().w_topline = curbuf_mut().b_ml.ml_line_count;
        }
        check_topfill(curwin_mut(), true);
    }
}

/// `winsaveview()` function (`f_winsaveview`).
pub fn f_winsaveview(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let dict = tv_dict_alloc_ret(rettv);

    tv_dict_add_nr(dict, c"lnum", curwin_mut().w_cursor.lnum as Varnumber);
    tv_dict_add_nr(dict, c"col", curwin_mut().w_cursor.col as Varnumber);
    tv_dict_add_nr(dict, c"coladd", curwin_mut().w_cursor.coladd as Varnumber);
    update_curswant();
    let mut vs = Viewstate::default();
    save_viewstate(curwin_mut(), &mut vs);
    tv_dict_add_nr(dict, c"curswant", vs.vs_curswant as Varnumber);

    tv_dict_add_nr(dict, c"topline", vs.vs_topline as Varnumber);
    tv_dict_add_nr(dict, c"topfill", vs.vs_topfill as Varnumber);
    tv_dict_add_nr(dict, c"leftcol", vs.vs_leftcol as Varnumber);
    tv_dict_add_nr(dict, c"skipcol", vs.vs_skipcol as Varnumber);
}

/// `winwidth(nr)` function (`f_winwidth`).
pub fn f_winwidth(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let wp = find_win_by_nr_or_id(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(if wp.is_null() {
            -1
        } else {
            (*wp).w_view_width as Varnumber
        });
    }
}
