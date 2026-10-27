//! Buffer related builtin functions (`src/nvim/eval/buffer.c`).

use crate::buffer::{
    MAXLNUM, SEA_NONE, bt_cmdwin, bt_nofilename, bt_prompt, buf_T, buf_ensure_loaded,
    buf_get_changedtick, buflist_add, buflist_findlnum, buflist_findname_exp, buflist_findnr,
    buflist_new, first_win_in_tab, kExtmarkNoUndo, kMarkAdjustNormal,
};
use crate::change::{
    ML_DEL_MESSAGE, appended_lines_mark, changed_lines, deleted_lines_buf, deleted_lines_mark,
    inserted_bytes,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{check_cursor_col, colnr_T, linenr_T};
use crate::eval::funcs::{get_buf_arg, tv_get_buf, tv_get_buf_from_arg};
use crate::eval::typval::{
    Callback, EvalFuncData, TypvalVval, VarType, callback_free, dict_T, kCallbackNone, list_T,
    listitem_T, tv_check_str_or_nr, tv_clear, tv_dict_add_dict, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_alloc, tv_dict_get_bool, tv_get_bool_chk, tv_get_lnum,
    tv_get_lnum_buf, tv_get_string, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_append_number, tv_list_append_string, tv_list_first,
    tv_list_item_remove, tv_list_last, tv_list_len, tv_set_string, typval_T, varnumber_T,
};
use crate::eval::window::win_has_winnr;
use crate::eval::{
    callback_call, callback_from_typval, kListLenMayKnow, prompt_get_input, typval_tostring,
};
use crate::ex_cmds::check_secure;
use crate::extmark::{extmark_splice_cols, kExtmarkUndo};
use crate::globals::*;
use crate::insert::buf_prompt_text;
use crate::mark::mark_adjust_buf;
use crate::memline::{
    ml_append, ml_delete_buf, ml_delete_flags, ml_get, ml_get_buf, ml_get_buf_len, ml_replace,
    ml_replace_buf,
};
use crate::r#move::update_topline;
use crate::path::path_with_url;
use crate::sign::{buf_has_signs, get_buffer_signs};
use crate::strings::concat_str;
use crate::undo::{bufIsChanged, u_clearallandblockfree, u_save, u_savesub, u_sync};
use crate::window::win_T;
use std::ffi::{CStr, c_int};
use std::ptr;

use crate::charset::NUL;
use crate::math::{FAIL, OK};

pub(crate) const SEA_READONLY: c_int = 4;

#[repr(C)]
struct cob_T {
    cob_curwin_save: *mut win_T,
    cob_aco: CtxSwitch,
    cob_using_aco: c_int,
    cob_save_VIsual_active: c_int,
}

impl cob_T {
    const fn new() -> Self {
        Self {
            cob_curwin_save: ptr::null_mut(),
            cob_aco: CtxSwitch::new(),
            cob_using_aco: 0,
            cob_save_VIsual_active: 0,
        }
    }
}

impl Default for cob_T {
    fn default() -> Self {
        Self::new()
    }
}

/// Find a buffer by number or exact name (`find_buffer`).
pub unsafe fn find_buffer(avar: *mut typval_T) -> *mut buf_T {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();

        if (*avar).v_type == VarType::VAR_NUMBER {
            buf = buflist_findnr((*avar).vval.v_number() as c_int);
        } else if (*avar).v_type == VarType::VAR_STRING && !(*avar).vval.v_string().is_null() {
            buf = buflist_findname_exp((*avar).vval.v_string());
            if buf.is_null() {
                let target = CStr::from_ptr((*avar).vval.v_string()).to_bytes();
                let mut bp = g_st().firstbuf;
                while !bp.is_null() {
                    if !(*bp).b_fname.is_null()
                        && (path_with_url((*bp).b_fname) != 0 || bt_nofilename(bp))
                        && CStr::from_ptr((*bp).b_fname).to_bytes() == target
                    {
                        buf = bp;
                        break;
                    }
                    bp = (*bp).b_next;
                }
            }
        }
        buf
    }
}

/// If there is a window for `curbuf`, make it the current window (`find_win_for_curbuf`).
unsafe fn find_win_for_curbuf() {
    unsafe {
        for wip in &(*g_st().curbuf).b_wininfo {
            if !wip.wi_win.is_null() && (*wip.wi_win).w_buffer == g_st().curbuf {
                *curwin.as_ptr() = wip.wi_win;
                break;
            }
        }
    }
}

/// Used before making a change in `buf`, which is not the current one (`change_other_buffer_prepare`).
unsafe fn change_other_buffer_prepare(cob: *mut cob_T, buf: *mut buf_T) {
    unsafe {
        *cob = cob_T::new();

        (*cob).cob_save_VIsual_active = c_int::from(g_st().Visual.active);
        g_st().Visual.active = false;
        (*cob).cob_curwin_save = *curwin.as_ptr();
        g_st().curbuf = buf;
        find_win_for_curbuf();

        if (**curwin.as_ptr()).w_buffer != buf {
            g_st().curbuf = (**curwin.as_ptr()).w_buffer;
            ctx_switch(
                &raw mut (*cob).cob_aco,
                ptr::null_mut(),
                ptr::null_mut(),
                buf,
                0,
            );
            (*cob).cob_using_aco = 1;
        }
    }
}

unsafe fn change_other_buffer_restore(cob: *mut cob_T) {
    unsafe {
        if (*cob).cob_using_aco != 0 {
            ctx_restore(&mut (*cob).cob_aco);
        } else {
            *curwin.as_ptr() = (*cob).cob_curwin_save;
            g_st().curbuf = (**curwin.as_ptr()).w_buffer;
        }
        g_st().Visual.active = (*cob).cob_save_VIsual_active != 0;
    }
}

/// Set line or list of lines in buffer `buf` to `lines` (`set_buffer_lines`).
unsafe fn set_buffer_lines(
    buf: *mut buf_T,
    lnum_arg: linenr_T,
    append: bool,
    lines: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        let mut lnum = lnum_arg + if append { 1 } else { 0 };
        let mut added: c_int = 0;

        let is_curbuf = buf == g_st().curbuf;
        if buf.is_null() || (!is_curbuf && (*buf).b_ml.ml_mfp.is_none()) || lnum < 1 {
            (*rettv).vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
            return;
        }

        let mut cob = cob_T::new();
        if !is_curbuf {
            change_other_buffer_prepare(&raw mut cob, buf);
        }

        let append_lnum = if append {
            lnum - 1
        } else {
            (*g_st().curbuf).b_ml.ml_line_count
        };

        let mut l: *mut list_T = ptr::null_mut();
        let mut li: *mut listitem_T = ptr::null_mut();
        let mut line: Option<std::ffi::CString> = None;
        if (*lines).v_type == VarType::VAR_LIST {
            l = (*lines).vval.v_list();
            if l.is_null() || tv_list_len(l) == 0 {
                if !is_curbuf {
                    change_other_buffer_restore(&raw mut cob);
                }
                return;
            }
            li = tv_list_first(l);
        } else {
            line = Some(typval_tostring(Some(&mut *lines), false));
        }

        loop {
            if (*lines).v_type == VarType::VAR_LIST {
                if li.is_null() {
                    break;
                }
                line = Some(typval_tostring(Some(&mut (*li).li_tv), false));
                li = (*li).li_next;
            }

            (*rettv).vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
            let Some(ref line_cstr) = line else {
                break;
            };
            if lnum > (*g_st().curbuf).b_ml.ml_line_count + 1 {
                break;
            }

            if g_st().u_sync_once == 2 {
                g_st().u_sync_once = 1;
                u_sync(true);
            }

            if !append && lnum <= (*g_st().curbuf).b_ml.ml_line_count {
                let old_len = CStr::from_ptr(ml_get(lnum)).to_bytes().len() as c_int;
                if u_savesub(lnum) == OK
                    && ml_replace(lnum, line_cstr.as_ptr().cast_mut(), true) == OK
                {
                    let new_len = line_cstr.to_bytes().len() as c_int;
                    inserted_bytes(lnum, 0, old_len, new_len);
                    if is_curbuf && lnum == (**curwin.as_ptr()).w_cursor.lnum {
                        check_cursor_col(*curwin.as_ptr());
                    }
                    (*rettv).vval = crate::eval::typval::TypvalVval::Number(0); // OK
                }
            } else if added > 0 || u_save(lnum - 1, lnum) == OK {
                added += 1;
                if ml_append(lnum - 1, line_cstr.as_ptr().cast_mut(), 0, false) == OK {
                    (*rettv).vval = crate::eval::typval::TypvalVval::Number(0); // OK
                }
            }

            if l.is_null() {
                break;
            }
            lnum += 1;
        }
        drop(line);

        if added > 0 {
            appended_lines_mark(append_lnum, added);

            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                let mut wp = first_win_in_tab(tp);
                while !wp.is_null() {
                    if (*wp).w_buffer == buf
                        && ((*wp).w_buffer != g_st().curbuf || wp == *curwin.as_ptr())
                        && (*wp).w_cursor.lnum > append_lnum
                    {
                        (*wp).w_cursor.lnum += added as linenr_T;
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
            check_cursor_col(*curwin.as_ptr());
            update_topline(*curwin.as_ptr());
        }

        if !is_curbuf {
            change_other_buffer_restore(&raw mut cob);
        }
    }
}

/// `append(lnum, string/list)` function (`f_append`).
pub unsafe fn f_append(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;
        let lnum = tv_get_lnum(&*argvars);
        if g_st().did_emsg == did_emsg_before {
            set_buffer_lines(g_st().curbuf, lnum, true, argvars.add(1), rettv);
        }
    }
}

/// Set or append lines to a buffer (`buf_set_append_line`).
unsafe fn buf_set_append_line(argvars: *mut typval_T, rettv: &mut typval_T, append: bool) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
        } else {
            let lnum = tv_get_lnum_buf(&*argvars.add(1), buf);
            if g_st().did_emsg == did_emsg_before {
                set_buffer_lines(buf, lnum, append, argvars.add(2), &raw mut *rettv);
            }
        }
    }
}

/// `appendbufline(buf, lnum, string/list)` function (`f_appendbufline`).
pub unsafe fn f_appendbufline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        buf_set_append_line(argvars, rettv, true);
    }
}

/// Trim lines above the prompt to enforce `'scrollback'` limit (`prompt_trim_scrollback`).
unsafe fn prompt_trim_scrollback(buf: &mut buf_T) {
    unsafe {
        if buf.b_p_scbk <= 0 {
            return;
        }

        let prompt_line = buf.b_prompt_start.mark.lnum;
        let above_prompt = prompt_line - 1;
        if above_prompt <= buf.b_p_scbk as linenr_T {
            return;
        }

        let to_delete = above_prompt - buf.b_p_scbk as linenr_T;
        for _ in 0..to_delete {
            ml_delete_buf(&raw mut *buf, 1, false);
        }
        mark_adjust_buf(
            &raw mut *buf,
            1,
            to_delete,
            MAXLNUM,
            -to_delete,
            true,
            kMarkAdjustNormal,
            kExtmarkUndo,
        );
        deleted_lines_buf(&raw mut *buf, 1, to_delete);

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    (*wp).w_cursor.lnum = if (*wp).w_cursor.lnum <= to_delete {
                        1
                    } else {
                        (*wp).w_cursor.lnum - to_delete
                    };
                    if (*wp).w_cursor.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
                        (*wp).w_cursor.lnum = (*(*wp).w_buffer).b_ml.ml_line_count;
                    }
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        check_cursor_col(*curwin.as_ptr());
    }
}

/// Invokes the user-defined callback defined for the current prompt-buffer (`prompt_invoke_callback`).
pub unsafe fn prompt_invoke_callback() {
    unsafe {
        let lnum = (*g_st().curbuf).b_ml.ml_line_count;
        let Some(user_input) = prompt_get_input(g_st().curbuf) else {
            return;
        };

        ml_append(lnum, c"".as_ptr().cast_mut(), 0, false);
        appended_lines_mark(lnum, 1);
        (**curwin.as_ptr()).w_cursor.lnum = lnum + 1;
        (**curwin.as_ptr()).w_cursor.col = 0;
        (*g_st().curbuf).b_prompt_start.mark.lnum = lnum + 1;

        if (*g_st().curbuf).b_prompt_callback.cb_type() != kCallbackNone {
            let mut rettv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            };
            let mut argv = [
                typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
                typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
            ];
            tv_set_string(&raw mut argv[0], user_input.as_ptr());

            callback_call(
                &raw mut (*g_st().curbuf).b_prompt_callback,
                1,
                argv.as_mut_ptr(),
                &raw mut rettv,
            );
            tv_clear(&raw mut argv[0]);
            tv_clear(&raw mut rettv);
        }

        u_clearallandblockfree(g_st().curbuf);

        (*g_st().curbuf).b_prompt_start.mark.lnum = (*g_st().curbuf).b_ml.ml_line_count;
        (*g_st().curbuf).b_prompt_append_new_line = true;

        prompt_trim_scrollback(&mut *g_st().curbuf);
    }
}

/// Return true when the interrupt callback was invoked (`invoke_prompt_interrupt`).
pub unsafe fn invoke_prompt_interrupt() -> bool {
    unsafe {
        if (*g_st().curbuf).b_prompt_interrupt.cb_type() == kCallbackNone {
            return false;
        }
        let mut rettv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        let mut argv = [typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        }];

        g_st().got_int = false;
        let ret = callback_call(
            &raw mut (*g_st().curbuf).b_prompt_interrupt,
            0,
            argv.as_mut_ptr(),
            &raw mut rettv,
        );
        tv_clear(&raw mut rettv);
        ret
    }
}

/// `prompt_appendbuf({buffer}, string/list)` function (`f_prompt_appendbuf`).
pub unsafe fn f_prompt_appendbuf(
    argvars: *mut typval_T,
    rettv: &mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;

        rettv.v_type = VarType::VAR_NUMBER;
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);

        let buf = tv_get_buf_from_arg(argvars);
        if buf.is_null() || !bt_prompt(buf) {
            return;
        }

        let lnum = ((*buf).b_prompt_start.mark.lnum - 1).max(0);
        let lines = argvars.add(1);
        let mut did_concat = false;

        if !(*buf).b_prompt_append_new_line {
            let text = if lnum > 0 {
                ml_get_buf(buf, lnum).cast_const()
            } else {
                c"".as_ptr()
            };
            if (*lines).v_type == VarType::VAR_LIST {
                let l = (*lines).vval.v_list();
                if !l.is_null() && tv_list_len(l) > 0 {
                    let li = tv_list_first(l);
                    let str_ = tv_get_string(&raw const (*li).li_tv);
                    let new_str = concat_str(text, str_);
                    crate::eval::typval::tv_set_string(&raw mut (*li).li_tv, new_str.as_ptr());
                    did_concat = true;
                }
            } else if (*lines).v_type == VarType::VAR_STRING {
                let str_ = tv_get_string(lines);
                let new_str = concat_str(text, str_);
                crate::eval::typval::tv_set_string(lines, new_str.as_ptr());
            }
        }

        if g_st().did_emsg == did_emsg_before {
            if did_concat && tv_list_len((*lines).vval.v_list()) > 1 {
                let l = (*lines).vval.v_list();
                let li = tv_list_first(l);
                set_buffer_lines(buf, lnum, false, &raw mut (*li).li_tv, rettv);

                if rettv.vval.v_number() == 0 {
                    tv_list_item_remove(l, li);
                    set_buffer_lines(buf, lnum, true, lines, rettv);
                }
            } else {
                set_buffer_lines(buf, lnum, (*buf).b_prompt_append_new_line, lines, rettv);
            }
        }

        if rettv.vval.v_number() == 0 {
            (*buf).b_prompt_append_new_line = false;
            if (*lines).v_type == VarType::VAR_LIST {
                let l = (*lines).vval.v_list();
                if !l.is_null() && tv_list_len(l) > 0 {
                    let li = tv_list_last(l);
                    let str_ = tv_get_string(&raw const (*li).li_tv);
                    let bytes = CStr::from_ptr(str_).to_bytes();
                    if bytes.last() == Some(&b'\n') {
                        (*buf).b_prompt_append_new_line = true;
                    }
                }
            } else if (*lines).v_type == VarType::VAR_STRING {
                let str_ = tv_get_string(lines);
                let bytes = CStr::from_ptr(str_).to_bytes();
                if bytes.last() == Some(&b'\n') {
                    (*buf).b_prompt_append_new_line = true;
                }
            }

            prompt_trim_scrollback(&mut *buf);
        }
    }
}

/// `bufadd(expr)` function (`f_bufadd`).
pub unsafe fn f_bufadd(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let name = tv_get_string(argvars).cast_mut();
        rettv.vval = crate::eval::typval::TypvalVval::Number(buflist_add(if *name == NUL { ptr::null_mut() } else { name }, 0) as varnumber_T);
    }
}

/// `bufexists(expr)` function (`f_bufexists`).
pub unsafe fn f_bufexists(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(!find_buffer(argvars).is_null()));
    }
}

/// `buflisted(expr)` function (`f_buflisted`).
pub unsafe fn f_buflisted(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let buf = find_buffer(argvars);
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(!buf.is_null() && (*buf).b_p_bl != 0));
    }
}

/// `bufload(expr)` function (`f_bufload`).
pub unsafe fn f_bufload(argvars: *mut typval_T, _unused: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let buf = get_buf_arg(argvars);
        if !buf.is_null() {
            if g_st().swap_exists_action != SEA_READONLY {
                g_st().swap_exists_action = SEA_NONE;
            }
            buf_ensure_loaded(&mut *buf);
        }
    }
}

/// `bufloaded(expr)` function (`f_bufloaded`).
pub unsafe fn f_bufloaded(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let buf = find_buffer(argvars);
        rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(!buf.is_null() && (*buf).b_ml.ml_mfp.is_some()));
    }
}

/// `bufname(expr)` function (`f_bufname`).
pub unsafe fn f_bufname(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_set_string(rettv, ptr::null());
        let buf: *const buf_T = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            g_st().curbuf
        } else {
            tv_get_buf_from_arg(argvars)
        };
        if !buf.is_null() && !(*buf).b_fname.is_null() {
            tv_set_string(rettv, (*buf).b_fname);
        }
    }
}

/// `bufnr(expr)` function (`f_bufnr`).
pub unsafe fn f_bufnr(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut error = false;

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let mut buf: *const buf_T = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            g_st().curbuf
        } else {
            if !tv_check_str_or_nr(argvars) {
                return;
            }
            g_st().emsg_off += 1;
            let b = tv_get_buf(argvars, 0);
            g_st().emsg_off -= 1;
            b
        };

        if buf.is_null()
            && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
            && tv_get_bool_chk(argvars.add(1), &mut error) != 0
            && !error
        {
            let name = tv_get_string_chk(argvars);
            if !name.is_null() {
                buf = buflist_new(name.cast_mut(), ptr::null_mut(), 1, 0);
            }
        }

        if !buf.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number((*buf).b_fnum as varnumber_T);
        }
    }
}

unsafe fn buf_win_common(argvars: *mut typval_T, rettv: &mut typval_T, get_nr: bool) {
    unsafe {
        let buf: *const buf_T = tv_get_buf_from_arg(argvars);
        if buf.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
            return;
        }

        let mut winnr: c_int = 0;
        let mut winid: c_int = 0;
        let mut found_buf = false;
        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            winnr += c_int::from(win_has_winnr(&mut *wp, g_st().curtab));
            if ptr::eq((*wp).w_buffer, buf) && (!get_nr || win_has_winnr(&mut *wp, g_st().curtab)) {
                found_buf = true;
                winid = (*wp).handle;
                break;
            }
            wp = (*wp).w_next;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(if found_buf {
            (if get_nr { winnr } else { winid }) as varnumber_T
        } else {
            -1
        });
    }
}

/// `bufwinid(nr)` function (`f_bufwinid`).
pub unsafe fn f_bufwinid(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        buf_win_common(argvars, rettv, false);
    }
}

/// `bufwinnr(nr)` function (`f_bufwinnr`).
pub unsafe fn f_bufwinnr(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        buf_win_common(argvars, rettv, true);
    }
}

/// `deletebufline()` function (`f_deletebufline`).
pub unsafe fn f_deletebufline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;
        rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL by default
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }

        let first = tv_get_lnum_buf(&*argvars.add(1), buf);
        if g_st().did_emsg > did_emsg_before {
            return;
        }
        let mut last = if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            tv_get_lnum_buf(&*argvars.add(2), buf)
        } else {
            first
        };

        if (*buf).b_ml.ml_mfp.is_none()
            || first < 1
            || first > (*buf).b_ml.ml_line_count
            || last < first
        {
            return;
        }

        let is_curbuf = buf == g_st().curbuf;
        let mut cob = cob_T::new();
        if !is_curbuf {
            change_other_buffer_prepare(&raw mut cob, buf);
        }

        if last > (*g_st().curbuf).b_ml.ml_line_count {
            last = (*g_st().curbuf).b_ml.ml_line_count;
        }
        let count: c_int = last - first + 1;

        if g_st().u_sync_once == 2 {
            g_st().u_sync_once = 1;
            u_sync(true);
        }

        if u_save(first - 1, last + 1) != FAIL {
            for _ in first..=last {
                ml_delete_flags(first, ML_DEL_MESSAGE);
            }

            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                let mut wp = first_win_in_tab(tp);
                while !wp.is_null() {
                    if (*wp).w_buffer == buf {
                        if (*wp).w_cursor.lnum > last {
                            (*wp).w_cursor.lnum -= count as linenr_T;
                        } else if (*wp).w_cursor.lnum > first {
                            (*wp).w_cursor.lnum = first;
                        }
                        if (*wp).w_cursor.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
                            (*wp).w_cursor.lnum = (*(*wp).w_buffer).b_ml.ml_line_count;
                        }
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
            check_cursor_col(*curwin.as_ptr());
            deleted_lines_mark(first, count);
            rettv.vval = crate::eval::typval::TypvalVval::Number(0); // OK
        }

        if !is_curbuf {
            change_other_buffer_restore(&raw mut cob);
        }
    }
}

/// Buffer options, variables and other attributes in a dictionary (`get_buffer_info`).
unsafe fn get_buffer_info(buf: &mut buf_T) -> *mut dict_T {
    unsafe {
        let dict = tv_dict_alloc();

        tv_dict_add_nr(dict, c"bufnr".as_ptr(), 5, buf.b_fnum as varnumber_T);
        tv_dict_add_str(
            dict,
            c"name".as_ptr(),
            4,
            if !buf.b_ffname.is_null() {
                buf.b_ffname
            } else {
                c"".as_ptr()
            },
        );
        let lnum = if ptr::eq(buf, g_st().curbuf) {
            (**curwin.as_ptr()).w_cursor.lnum
        } else {
            buflist_findlnum(&raw mut *buf)
        };
        tv_dict_add_nr(dict, c"lnum".as_ptr(), 4, lnum as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"linecount".as_ptr(),
            9,
            buf.b_ml.ml_line_count as varnumber_T,
        );
        tv_dict_add_nr(
            dict,
            c"loaded".as_ptr(),
            6,
            varnumber_T::from(buf.b_ml.ml_mfp.is_some()),
        );
        tv_dict_add_nr(dict, c"listed".as_ptr(), 6, buf.b_p_bl as varnumber_T);
        tv_dict_add_nr(
            dict,
            c"changed".as_ptr(),
            7,
            varnumber_T::from(bufIsChanged(buf)),
        );
        tv_dict_add_nr(dict, c"changedtick".as_ptr(), 11, buf_get_changedtick(&raw mut *buf));
        tv_dict_add_nr(
            dict,
            c"hidden".as_ptr(),
            6,
            varnumber_T::from(buf.b_ml.ml_mfp.is_some() && buf.b_nwindows == 0),
        );
        tv_dict_add_nr(
            dict,
            c"command".as_ptr(),
            7,
            varnumber_T::from(bt_cmdwin(&raw mut *buf)),
        );

        tv_dict_add_dict(dict, c"variables".as_ptr(), 9, buf.b_vars());

        let windows = tv_list_alloc(kListLenMayKnow);
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    tv_list_append_number(windows, (*wp).handle as varnumber_T);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        tv_dict_add_list(dict, c"windows".as_ptr(), 7, windows);

        if buf_has_signs(&raw mut *buf) {
            tv_dict_add_list(dict, c"signs".as_ptr(), 5, get_buffer_signs(buf));
        }

        tv_dict_add_nr(dict, c"lastused".as_ptr(), 8, buf.b_last_used);

        dict
    }
}

/// `getbufinfo()` function (`f_getbufinfo`).
pub unsafe fn f_getbufinfo(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut argbuf: *mut buf_T = ptr::null_mut();
        let mut filtered = false;
        let mut sel_buflisted = false;
        let mut sel_bufloaded = false;
        let mut sel_bufmodified = false;

        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if (*argvars).v_type == VarType::VAR_DICT {
            let sel_d = (*argvars).vval.v_dict();
            if !sel_d.is_null() {
                filtered = true;
                sel_buflisted = tv_dict_get_bool(sel_d, c"buflisted".as_ptr(), 0) != 0;
                sel_bufloaded = tv_dict_get_bool(sel_d, c"bufloaded".as_ptr(), 0) != 0;
                sel_bufmodified = tv_dict_get_bool(sel_d, c"bufmodified".as_ptr(), 0) != 0;
            }
        } else if (*argvars).v_type != VarType::VAR_UNKNOWN {
            argbuf = tv_get_buf_from_arg(argvars);
            if argbuf.is_null() {
                return;
            }
        }

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if !argbuf.is_null() && argbuf != buf {
                buf = (*buf).b_next;
                continue;
            }
            if filtered
                && ((sel_bufloaded && (*buf).b_ml.ml_mfp.is_none())
                    || (sel_buflisted && (*buf).b_p_bl == 0)
                    || (sel_bufmodified && (*buf).b_changed == 0))
            {
                buf = (*buf).b_next;
                continue;
            }

            let d = get_buffer_info(&mut *buf);
            tv_list_append_dict(rettv.vval.v_list(), d);
            if !argbuf.is_null() {
                return;
            }
            buf = (*buf).b_next;
        }
    }
}

/// Get line or list of lines from buffer `buf` into `rettv` (`get_buffer_lines`).
unsafe fn get_buffer_lines(
    buf: *mut buf_T,
    mut start: linenr_T,
    mut end: linenr_T,
    retlist: bool,
    rettv: *mut typval_T,
) {
    unsafe {
        (*rettv).v_type = if retlist {
            VarType::VAR_LIST
        } else {
            VarType::VAR_STRING
        };
        (*rettv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() || start < 0 || end < start {
            if retlist {
                tv_list_alloc_ret(rettv, 0);
            }
            return;
        }

        if retlist {
            if start < 1 {
                start = 1;
            }
            if end > (*buf).b_ml.ml_line_count {
                end = (*buf).b_ml.ml_line_count;
            }
            tv_list_alloc_ret(rettv, (end - start + 1) as isize);
            while start <= end {
                tv_list_append_string(
                    (*rettv).vval.v_list(),
                    ml_get_buf(buf, start),
                    ml_get_buf_len(buf, start) as isize,
                );
                start += 1;
            }
        } else if start >= 1 && start <= (*buf).b_ml.ml_line_count {
            crate::eval::typval::tv_set_string_len(
                rettv,
                ml_get_buf(buf, start),
                ml_get_buf_len(buf, start) as usize,
            );
        } else {
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

unsafe fn getbufline(argvars: *mut typval_T, rettv: *mut typval_T, retlist: bool) {
    unsafe {
        let mut lnum: linenr_T = 1;
        let mut end: linenr_T = 1;
        let did_emsg_before = g_st().did_emsg;
        let buf = tv_get_buf_from_arg(argvars);
        if !buf.is_null() {
            lnum = tv_get_lnum_buf(&*argvars.add(1), buf);
            if g_st().did_emsg > did_emsg_before {
                return;
            }
            end = if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
                lnum
            } else {
                tv_get_lnum_buf(&*argvars.add(2), buf)
            };
        }

        get_buffer_lines(buf, lnum, end, retlist, rettv);
    }
}

/// `getbufline()` function (`f_getbufline`).
pub unsafe fn f_getbufline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getbufline(argvars, rettv, true);
    }
}

/// `getbufoneline()` function (`f_getbufoneline`).
pub unsafe fn f_getbufoneline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        getbufline(argvars, rettv, false);
    }
}

/// `getline(lnum, [end])` function (`f_getline`).
pub unsafe fn f_getline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let lnum = tv_get_lnum(&*argvars);
        let (end, retlist) = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            (lnum, false)
        } else {
            (tv_get_lnum(&*argvars.add(1)), true)
        };

        get_buffer_lines(g_st().curbuf, lnum, end, retlist, rettv);
    }
}

/// `setbufline()` function (`f_setbufline`).
pub unsafe fn f_setbufline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        buf_set_append_line(argvars, rettv, false);
    }
}

/// `setline()` function (`f_setline`).
pub unsafe fn f_setline(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;
        let lnum = tv_get_lnum(&*argvars);
        if g_st().did_emsg == did_emsg_before {
            set_buffer_lines(g_st().curbuf, lnum, false, argvars.add(1), rettv);
        }
    }
}

/// `prompt_setcallback({buffer}, {callback})` function (`f_prompt_setcallback`).
pub unsafe fn f_prompt_setcallback(
    argvars: *mut typval_T,
    _rettv: &mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut prompt_callback = Callback::NONE;

        if check_secure() {
            return;
        }
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }

        if !callback_from_typval(&mut prompt_callback, &*argvars.add(1)) {
            return;
        }

        callback_free(&mut (*buf).b_prompt_callback);
        (*buf).b_prompt_callback = prompt_callback;
    }
}

/// `prompt_setinterrupt({buffer}, {callback})` function (`f_prompt_setinterrupt`).
pub unsafe fn f_prompt_setinterrupt(
    argvars: *mut typval_T,
    _rettv: &mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut interrupt_callback = Callback::NONE;

        if check_secure() {
            return;
        }
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }

        if !callback_from_typval(&mut interrupt_callback, &*argvars.add(1)) {
            return;
        }

        callback_free(&mut (*buf).b_prompt_interrupt);
        (*buf).b_prompt_interrupt = interrupt_callback;
    }
}

/// `prompt_getprompt({buffer})` function (`f_prompt_getprompt`).
pub unsafe fn f_prompt_getprompt(
    argvars: *mut typval_T,
    rettv: &mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_set_string(rettv, ptr::null());

        let buf = tv_get_buf_from_arg(argvars);
        if buf.is_null() || !bt_prompt(buf) {
            return;
        }

        tv_set_string(rettv, buf_prompt_text(&*buf));
    }
}

/// `prompt_getinput({buffer})` function (`f_prompt_getinput`).
pub unsafe fn f_prompt_getinput(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_set_string(rettv, ptr::null());

        let buf = tv_get_buf_from_arg(argvars);
        if buf.is_null() || !bt_prompt(buf) {
            return;
        }

        tv_set_string(
            rettv,
            prompt_get_input(buf)
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr()),
        );
    }
}

/// `prompt_setprompt({buffer}, {text})` function (`f_prompt_setprompt`).
pub unsafe fn f_prompt_setprompt(
    argvars: *mut typval_T,
    _rettv: &mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }

        let new_prompt = tv_get_string(argvars.add(1));
        let new_prompt_len = CStr::from_ptr(new_prompt).to_bytes().len() as c_int;

        if bt_prompt(buf) && (*buf).b_ml.ml_mfp.is_some() {
            if (*buf).b_prompt_start.mark.lnum < 1
                || (*buf).b_prompt_start.mark.lnum > (*g_st().curbuf).b_ml.ml_line_count
            {
                (*buf).b_prompt_start.mark.lnum = (*buf)
                    .b_prompt_start
                    .mark
                    .lnum
                    .min((*buf).b_ml.ml_line_count)
                    .max(1);
                (*g_st().curbuf).b_prompt_append_new_line = true;
            }

            let prompt_lno = (*buf).b_prompt_start.mark.lnum;
            let old_prompt = buf_prompt_text(&*buf);
            let old_line = ml_get_buf(buf, prompt_lno);
            let old_line_len = ml_get_buf_len(buf, prompt_lno);

            let old_prompt_bytes = CStr::from_ptr(old_prompt).to_bytes();
            let old_prompt_len = old_prompt_bytes.len() as c_int;
            let mut cursor_col = (**curwin.as_ptr()).w_cursor.col;

            let mark_col = (*buf).b_prompt_start.mark.col;
            let prefix_matches = mark_col >= old_prompt_len
                && mark_col <= old_line_len
                && std::slice::from_raw_parts(
                    old_line
                        .add((mark_col - old_prompt_len) as usize)
                        .cast::<u8>(),
                    old_prompt_bytes.len(),
                ) == old_prompt_bytes;

            if !prefix_matches {
                ml_replace_buf(buf, prompt_lno, new_prompt.cast_mut(), true, false);
                extmark_splice_cols(
                    buf,
                    prompt_lno - 1,
                    0,
                    old_line_len,
                    new_prompt_len,
                    kExtmarkNoUndo,
                );
                cursor_col = new_prompt_len;
            } else {
                let new_line = concat_str(new_prompt, old_line.add(mark_col as usize));
                let _ = ml_replace_buf(buf, prompt_lno, new_line.as_ptr().cast_mut(), true, false);
                extmark_splice_cols(
                    buf,
                    prompt_lno - 1,
                    0,
                    mark_col,
                    new_prompt_len,
                    kExtmarkNoUndo,
                );
                cursor_col += new_prompt_len - mark_col;
            }

            if (**curwin.as_ptr()).w_buffer == buf && (**curwin.as_ptr()).w_cursor.lnum == prompt_lno {
                (**curwin.as_ptr()).w_cursor.col = cursor_col;
                check_cursor_col(*curwin.as_ptr());
            }
            changed_lines(buf, prompt_lno, 0, prompt_lno + 1, 0, true);
            u_clearallandblockfree(buf);
        }

        (*buf).b_prompt_text = Some(CStr::from_ptr(new_prompt).to_owned());
        (*buf).b_prompt_start.mark.col = new_prompt_len as colnr_T;
    }
}
