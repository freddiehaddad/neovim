//! Buffer related builtin functions (`src/nvim/eval/buffer.c`).

use crate::buffer::{
    FileBuf, K_EXTMARK_NO_UNDO, K_MARK_ADJUST_NORMAL, MAXLNUM, SEA_NONE, bt_cmdwin, bt_nofilename,
    bt_prompt, buf_ensure_loaded, buf_get_changedtick, buflist_add, buflist_findlnum,
    buflist_findname_exp, buflist_findnr, buflist_new, first_win_in_tab,
};
use crate::change::{
    ML_DEL_MESSAGE, appended_lines_mark, changed_lines, deleted_lines_buf, deleted_lines_mark,
    inserted_bytes,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor::{Colnr, Linenr, check_cursor_col};
use crate::eval::funcs::{get_buf_arg, tv_get_buf, tv_get_buf_from_arg};
use crate::eval::typval::{
    Callback, EvalFuncData, K_CALLBACK_NONE_CONST, Listitem, TvDict, TvList, TypVal, TypvalVval,
    VarType, Varnumber, callback_free, tv_check_str_or_nr, tv_clear, tv_dict_add_dict,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_alloc, tv_dict_get_bool,
    tv_get_bool_chk, tv_get_lnum, tv_get_lnum_buf, tv_get_string, tv_get_string_chk, tv_list_alloc,
    tv_list_alloc_ret, tv_list_append_dict, tv_list_append_number, tv_list_append_string,
    tv_list_first, tv_list_item_remove, tv_list_last, tv_list_len, tv_set_string,
};
use crate::eval::window::win_has_winnr;
use crate::eval::{
    K_LIST_LEN_MAY_KNOW_CONST, callback_call, callback_from_typval, prompt_get_input,
    typval_tostring,
};
use crate::ex_cmds::check_secure;
use crate::extmark::{K_EXTMARK_UNDO, extmark_splice_cols};
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
use crate::undo::{buf_is_changed_fn, u_clearallandblockfree, u_save, u_savesub, u_sync};
use crate::window::WinInfoInfo;
use std::ffi::c_int;
use std::ptr;

pub(crate) const SEA_READONLY: c_int = 4;

#[repr(C)]
struct Cob {
    cob_curwin_save: *mut WinInfoInfo,
    cob_aco: CtxSwitch,
    cob_using_aco: c_int,
    cob_save_visual_active: c_int,
}

impl Cob {
    const fn new() -> Self {
        Self {
            cob_curwin_save: ptr::null_mut(),
            cob_aco: CtxSwitch::new(),
            cob_using_aco: 0,
            cob_save_visual_active: 0,
        }
    }
}

impl Default for Cob {
    fn default() -> Self {
        Self::new()
    }
}

/// Find a buffer by number or exact name (`find_buffer`).
pub fn find_buffer(avar: &mut TypVal) -> *mut FileBuf {
    {
        let mut buf: *mut FileBuf = ptr::null_mut();

        if avar.v_type == VarType::VarNumber {
            buf = buflist_findnr(avar.vval.v_number() as c_int);
        } else if avar.v_type == VarType::VarString && !avar.vval.v_string().is_null() {
            buf = buflist_findname_exp(avar.vval.v_string());
            if buf.is_null() {
                let target = crate::memory::cstr_as_bytes(avar.vval.v_string());
                for bp in for_all_buffers_ptr() {
                    if !bp.b_fname.is_null()
                        && (path_with_url(bp.b_fname) != 0 || bt_nofilename(Some(&*bp)))
                        && crate::memory::cstr_as_bytes(bp.b_fname) == target
                    {
                        buf = bp;
                        break;
                    }
                }
            }
        }
        buf
    }
}

/// If there is a window for `curbuf`, make it the current window (`find_win_for_curbuf`).
fn find_win_for_curbuf() {
    unsafe {
        for wip in &curbuf_mut().b_wininfo {
            if !wip.wi_win.is_null() && (*wip.wi_win).w_buffer == g_st().curbuf {
                set_curwin(wip.wi_win);
                break;
            }
        }
    }
}

/// Used before making a change in `buf`, which is not the current one (`change_other_buffer_prepare`).
fn change_other_buffer_prepare(cob: &mut Cob, buf: *mut FileBuf) {
    {
        *cob = Cob::new();

        cob.cob_save_visual_active = c_int::from(g_st().visual_global.active);
        g_st().visual_global.active = false;
        cob.cob_curwin_save = curwin_ptr();
        g_st().curbuf = buf;
        find_win_for_curbuf();

        if curwin_mut().w_buffer != buf {
            g_st().curbuf = curwin_mut().w_buffer;
            ctx_switch(&mut cob.cob_aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            cob.cob_using_aco = 1;
        }
    }
}

fn change_other_buffer_restore(cob: &mut Cob) {
    {
        if cob.cob_using_aco != 0 {
            ctx_restore(&mut cob.cob_aco);
        } else {
            set_curwin(cob.cob_curwin_save);
            g_st().curbuf = curwin_mut().w_buffer;
        }
        g_st().visual_global.active = cob.cob_save_visual_active != 0;
    }
}

/// Set line or list of lines in buffer `buf` to `lines` (`set_buffer_lines`).
fn set_buffer_lines(
    buf: *mut FileBuf,
    lnum_arg: Linenr,
    append: bool,
    lines: &mut TypVal,
    rettv: &mut TypVal,
) {
    unsafe {
        let mut lnum = lnum_arg + if append { 1 } else { 0 };
        let mut added: c_int = 0;

        let is_curbuf = buf == g_st().curbuf;
        if buf.is_null() || (!is_curbuf && (*buf).b_ml.ml_mfp.is_none()) || lnum < 1 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
            return;
        }

        let mut cob = Cob::new();
        if !is_curbuf {
            change_other_buffer_prepare(&mut cob, buf);
        }

        let append_lnum = if append {
            lnum - 1
        } else {
            curbuf_mut().b_ml.ml_line_count
        };

        let mut l: *mut TvList = ptr::null_mut();
        let mut li: *mut Listitem = ptr::null_mut();
        let mut line: Option<std::ffi::CString> = None;
        if lines.v_type == VarType::VarList {
            l = lines.vval.v_list();
            if l.is_null() || tv_list_len(l.as_ref()) == 0 {
                if !is_curbuf {
                    change_other_buffer_restore(&mut cob);
                }
                return;
            }
            li = tv_list_first(l.as_ref());
        } else {
            line = Some(typval_tostring(Some(lines), false));
        }

        loop {
            if lines.v_type == VarType::VarList {
                if li.is_null() {
                    break;
                }
                line = Some(typval_tostring(Some(&mut (*li).li_tv), false));
                li = (*li).li_next;
            }

            rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
            let Some(ref line_cstr) = line else {
                break;
            };
            if lnum > curbuf_mut().b_ml.ml_line_count + 1 {
                break;
            }

            if g_st().u_sync_once == 2 {
                g_st().u_sync_once = 1;
                u_sync(true);
            }

            if !append && lnum <= curbuf_mut().b_ml.ml_line_count {
                let old_len = crate::memory::cstr_len(ml_get(lnum)) as c_int;
                if u_savesub(lnum) && ml_replace(lnum, line_cstr.as_ptr().cast_mut(), true) {
                    let new_len = line_cstr.to_bytes().len() as c_int;
                    inserted_bytes(lnum, 0, old_len, new_len);
                    if is_curbuf && lnum == curwin_mut().w_cursor.lnum {
                        check_cursor_col(curwin_mut());
                    }
                    rettv.vval = crate::eval::typval::TypvalVval::Number(0); // OK
                }
            } else if added > 0 || u_save(lnum - 1, lnum) {
                added += 1;
                if ml_append(lnum - 1, line_cstr.as_ptr().cast_mut(), 0, false) {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(0); // OK
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

            for tp in for_all_tabpages_ptr() {
                for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
                    if wp.w_buffer == buf
                        && (wp.w_buffer != g_st().curbuf || std::ptr::eq(wp, curwin_ptr()))
                        && wp.w_cursor.lnum > append_lnum
                    {
                        wp.w_cursor.lnum += added as Linenr;
                    }
                }
            }
            check_cursor_col(curwin_mut());
            update_topline(curwin_mut());
        }

        if !is_curbuf {
            change_other_buffer_restore(&mut cob);
        }
    }
}

/// `append(lnum, string/list)` function (`f_append`).
pub fn f_append(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let did_emsg_before = g_st().did_emsg;
    let lnum = tv_get_lnum(&argvars[0]);
    if g_st().did_emsg == did_emsg_before {
        set_buffer_lines(g_st().curbuf, lnum, true, &mut argvars[1], rettv);
    }
}

/// Set or append lines to a buffer (`buf_set_append_line`).
fn buf_set_append_line(argvars: &mut [TypVal], rettv: &mut TypVal, append: bool) {
    let did_emsg_before = g_st().did_emsg;
    let buf = tv_get_buf(&mut argvars[0], 0);
    if buf.is_null() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL
    } else {
        let lnum = tv_get_lnum_buf(&argvars[1], unsafe { buf.as_ref() });
        if g_st().did_emsg == did_emsg_before {
            set_buffer_lines(buf, lnum, append, &mut argvars[2], rettv);
        }
    }
}

/// `appendbufline(buf, lnum, string/list)` function (`f_appendbufline`).
pub fn f_appendbufline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    buf_set_append_line(argvars, rettv, true);
}

/// Trim lines above the prompt to enforce `'scrollback'` limit (`prompt_trim_scrollback`).
fn prompt_trim_scrollback(buf: &mut FileBuf) {
    {
        if buf.b_p_scbk <= 0 {
            return;
        }

        let prompt_line = buf.b_prompt_start.mark.lnum;
        let above_prompt = prompt_line - 1;
        if above_prompt <= buf.b_p_scbk as Linenr {
            return;
        }

        let to_delete = above_prompt - buf.b_p_scbk as Linenr;
        for _ in 0..to_delete {
            ml_delete_buf(buf, 1, false);
        }
        mark_adjust_buf(
            &mut *buf,
            1,
            to_delete,
            MAXLNUM,
            -to_delete,
            true,
            K_MARK_ADJUST_NORMAL,
            K_EXTMARK_UNDO,
        );
        deleted_lines_buf(buf, 1, to_delete);

        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_mut(first_win_in_tab(&mut *tp)) {
                if wp.w_buffer == buf {
                    wp.w_cursor.lnum = if wp.w_cursor.lnum <= to_delete {
                        1
                    } else {
                        wp.w_cursor.lnum - to_delete
                    };
                    if wp.w_cursor.lnum > wp.buf_mut().b_ml.ml_line_count {
                        wp.w_cursor.lnum = wp.buf_mut().b_ml.ml_line_count;
                    }
                }
            }
        }
        check_cursor_col(curwin_mut());
    }
}

/// Invokes the user-defined callback defined for the current prompt-buffer (`prompt_invoke_callback`).
pub fn prompt_invoke_callback() {
    unsafe {
        let lnum = curbuf_mut().b_ml.ml_line_count;
        let Some(user_input) = prompt_get_input(curbuf_mut()) else {
            return;
        };

        ml_append(lnum, c"", 0, false);
        appended_lines_mark(lnum, 1);
        curwin_mut().w_cursor.lnum = lnum + 1;
        curwin_mut().w_cursor.col = 0;
        curbuf_mut().b_prompt_start.mark.lnum = lnum + 1;

        if curbuf_mut().b_prompt_callback.cb_type() != K_CALLBACK_NONE_CONST {
            let mut rettv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            };
            let mut argv = [
                TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
                TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: 0,
                    vval: TypvalVval::Number(0),
                },
            ];
            tv_set_string(&mut argv[0], user_input.as_ptr());

            callback_call(
                &mut (*g_st().curbuf).b_prompt_callback,
                1,
                argv.as_mut_ptr(),
                &mut rettv,
            );
            tv_clear(&mut argv[0]);
            tv_clear(&mut rettv);
        }

        u_clearallandblockfree(curbuf_mut());

        curbuf_mut().b_prompt_start.mark.lnum = curbuf_mut().b_ml.ml_line_count;
        curbuf_mut().b_prompt_append_new_line = true;

        prompt_trim_scrollback(curbuf_mut());
    }
}

/// Return true when the interrupt callback was invoked (`invoke_prompt_interrupt`).
pub fn invoke_prompt_interrupt() -> bool {
    unsafe {
        if curbuf_mut().b_prompt_interrupt.cb_type() == K_CALLBACK_NONE_CONST {
            return false;
        }
        let mut rettv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        let mut argv = [TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        }];

        g_st().got_int = false;
        let ret = callback_call(
            &mut (*g_st().curbuf).b_prompt_interrupt,
            0,
            argv.as_mut_ptr(),
            &mut rettv,
        );
        tv_clear(&mut rettv);
        ret
    }
}

/// `prompt_appendbuf({buffer}, string/list)` function (`f_prompt_appendbuf`).
pub fn f_prompt_appendbuf(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;

        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);

        let buf = tv_get_buf_from_arg(&mut argvars[0]);
        if buf.is_null() || !bt_prompt(buf.as_ref()) {
            return;
        }

        let lnum = ((*buf).b_prompt_start.mark.lnum - 1).max(0);
        let lines = (&raw mut argvars[1]);
        let mut did_concat = false;

        if !(*buf).b_prompt_append_new_line {
            let text = if lnum > 0 {
                ml_get_buf(&mut *buf, lnum).cast_const()
            } else {
                c"".as_ptr()
            };
            if (*lines).v_type == VarType::VarList {
                let l = (*lines).vval.v_list();
                if !l.is_null() && tv_list_len(l.as_ref()) > 0 {
                    let li = tv_list_first(l.as_ref());
                    let str_ = tv_get_string(&(*li).li_tv);
                    let new_str = concat_str(text, str_);
                    crate::eval::typval::tv_set_string(&mut (*li).li_tv, new_str.as_ptr());
                    did_concat = true;
                }
            } else if (*lines).v_type == VarType::VarString {
                let str_ = tv_get_string(&*lines);
                let new_str = concat_str(text, str_);
                crate::eval::typval::tv_set_string(&mut *lines, new_str.as_ptr());
            }
        }

        if g_st().did_emsg == did_emsg_before {
            if did_concat && tv_list_len(((*lines).vval.v_list()).as_ref()) > 1 {
                let l = (*lines).vval.v_list();
                let li = tv_list_first(l.as_ref());
                set_buffer_lines(buf, lnum, false, &mut (*li).li_tv, &mut *rettv);

                if rettv.vval.v_number() == 0 {
                    tv_list_item_remove(&mut *l, li);
                    set_buffer_lines(buf, lnum, true, &mut *lines, &mut *rettv);
                }
            } else {
                set_buffer_lines(
                    buf,
                    lnum,
                    (*buf).b_prompt_append_new_line,
                    &mut *lines,
                    &mut *rettv,
                );
            }
        }

        if rettv.vval.v_number() == 0 {
            (*buf).b_prompt_append_new_line = false;
            if (*lines).v_type == VarType::VarList {
                let l = (*lines).vval.v_list();
                if !l.is_null() && tv_list_len(l.as_ref()) > 0 {
                    let li = tv_list_last(l.as_ref());
                    let str_ = tv_get_string(&(*li).li_tv);
                    let bytes = crate::memory::cstr_as_bytes(str_);
                    if bytes.last() == Some(&b'\n') {
                        (*buf).b_prompt_append_new_line = true;
                    }
                }
            } else if (*lines).v_type == VarType::VarString {
                let str_ = tv_get_string(&*lines);
                let bytes = crate::memory::cstr_as_bytes(str_);
                if bytes.last() == Some(&b'\n') {
                    (*buf).b_prompt_append_new_line = true;
                }
            }

            prompt_trim_scrollback(&mut *buf);
        }
    }
}

/// `bufadd(expr)` function (`f_bufadd`).
pub fn f_bufadd(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let name = tv_get_string(&argvars[0]).cast_mut();
        rettv.vval = crate::eval::typval::TypvalVval::Number(buflist_add(
            if crate::memory::cstr_is_empty(name) {
                ptr::null_mut()
            } else {
                name
            },
            0,
        ) as Varnumber);
    }
}

/// `bufexists(expr)` function (`f_bufexists`).
pub fn f_bufexists(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
        !find_buffer(&mut argvars[0]).is_null(),
    ));
}

/// `buflisted(expr)` function (`f_buflisted`).
pub fn f_buflisted(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let buf = find_buffer(&mut argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
            !buf.is_null() && (*buf).b_p_bl != 0,
        ));
    }
}

/// `bufload(expr)` function (`f_bufload`).
pub fn f_bufload(argvars: &mut [TypVal], _unused: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let buf = get_buf_arg(&mut argvars[0]);
        if !buf.is_null() {
            if g_st().swap_exists_action != SEA_READONLY {
                g_st().swap_exists_action = SEA_NONE;
            }
            buf_ensure_loaded(&mut *buf);
        }
    }
}

/// `bufloaded(expr)` function (`f_bufloaded`).
pub fn f_bufloaded(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let buf = find_buffer(&mut argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
            !buf.is_null() && (*buf).b_ml.ml_mfp.is_some(),
        ));
    }
}

/// `bufname(expr)` function (`f_bufname`).
pub fn f_bufname(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        tv_set_string(rettv, ptr::null());
        let buf: *const FileBuf = if argvars[0].v_type == VarType::VarUnknown {
            g_st().curbuf
        } else {
            tv_get_buf_from_arg(&mut argvars[0])
        };
        if !buf.is_null() && !(*buf).b_fname.is_null() {
            tv_set_string(rettv, (*buf).b_fname);
        }
    }
}

/// `bufnr(expr)` function (`f_bufnr`).
pub fn f_bufnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut error = false;

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let mut buf: *const FileBuf = if argvars[0].v_type == VarType::VarUnknown {
            g_st().curbuf
        } else {
            if !tv_check_str_or_nr(&argvars[0]) {
                return;
            }
            g_st().emsg_off += 1;
            let b = tv_get_buf(&mut argvars[0], 0);
            g_st().emsg_off -= 1;
            b
        };

        if buf.is_null()
            && argvars[1].v_type != VarType::VarUnknown
            && tv_get_bool_chk(&argvars[1], Some(&mut error)) != 0
            && !error
        {
            let name = tv_get_string_chk(&argvars[0]);
            if !name.is_null() {
                buf = buflist_new(name.cast_mut(), ptr::null_mut(), 1, 0);
            }
        }

        if !buf.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number((*buf).b_fnum as Varnumber);
        }
    }
}

fn buf_win_common(argvars: &mut [TypVal], rettv: &mut TypVal, get_nr: bool) {
    {
        let buf: *const FileBuf = tv_get_buf_from_arg(&mut argvars[0]);
        if buf.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
            return;
        }

        let mut winnr: c_int = 0;
        let mut winid: c_int = 0;
        let mut found_buf = false;
        for wp in for_all_windows_mut() {
            winnr += c_int::from(win_has_winnr(wp, curtab_mut()));
            if ptr::eq(wp.w_buffer, buf) && (!get_nr || win_has_winnr(wp, curtab_mut())) {
                found_buf = true;
                winid = wp.handle;
                break;
            }
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(if found_buf {
            (if get_nr { winnr } else { winid }) as Varnumber
        } else {
            -1
        });
    }
}

/// `bufwinid(nr)` function (`f_bufwinid`).
pub fn f_bufwinid(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    buf_win_common(argvars, rettv, false);
}

/// `bufwinnr(nr)` function (`f_bufwinnr`).
pub fn f_bufwinnr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    buf_win_common(argvars, rettv, true);
}

/// `deletebufline()` function (`f_deletebufline`).
pub fn f_deletebufline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let did_emsg_before = g_st().did_emsg;
        rettv.vval = crate::eval::typval::TypvalVval::Number(1); // FAIL by default
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            return;
        }

        let first = tv_get_lnum_buf(&argvars[1], buf.as_ref());
        if g_st().did_emsg > did_emsg_before {
            return;
        }
        let mut last = if argvars[2].v_type != VarType::VarUnknown {
            tv_get_lnum_buf(&argvars[2], buf.as_ref())
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
        let mut cob = Cob::new();
        if !is_curbuf {
            change_other_buffer_prepare(&mut cob, buf);
        }

        if last > curbuf_mut().b_ml.ml_line_count {
            last = curbuf_mut().b_ml.ml_line_count;
        }
        let count: c_int = last - first + 1;

        if g_st().u_sync_once == 2 {
            g_st().u_sync_once = 1;
            u_sync(true);
        }

        if u_save(first - 1, last + 1) {
            for _ in first..=last {
                ml_delete_flags(first, ML_DEL_MESSAGE);
            }

            for tp in for_all_tabpages_ptr() {
                for wp in win_iter_mut(first_win_in_tab(&mut *tp)) {
                    if wp.w_buffer == buf {
                        if wp.w_cursor.lnum > last {
                            wp.w_cursor.lnum -= count as Linenr;
                        } else if wp.w_cursor.lnum > first {
                            wp.w_cursor.lnum = first;
                        }
                        if wp.w_cursor.lnum > wp.buf_mut().b_ml.ml_line_count {
                            wp.w_cursor.lnum = wp.buf_mut().b_ml.ml_line_count;
                        }
                    }
                }
            }
            check_cursor_col(curwin_mut());
            deleted_lines_mark(first, count);
            rettv.vval = crate::eval::typval::TypvalVval::Number(0); // OK
        }

        if !is_curbuf {
            change_other_buffer_restore(&mut cob);
        }
    }
}

/// Buffer options, variables and other attributes in a dictionary (`get_buffer_info`).
fn get_buffer_info(buf: &mut FileBuf) -> &'static mut TvDict {
    {
        let dict = tv_dict_alloc();

        tv_dict_add_nr(&mut *dict, c"bufnr", buf.b_fnum as Varnumber);
        tv_dict_add_str(
            &mut *dict,
            c"name",
            if !buf.b_ffname.is_null() {
                buf.b_ffname
            } else {
                c"".as_ptr()
            },
        );
        let lnum = if ptr::eq(buf, g_st().curbuf) {
            curwin_mut().w_cursor.lnum
        } else {
            buflist_findlnum(buf)
        };
        tv_dict_add_nr(&mut *dict, c"lnum", lnum as Varnumber);
        tv_dict_add_nr(
            &mut *dict,
            c"linecount",
            buf.b_ml.ml_line_count as Varnumber,
        );
        tv_dict_add_nr(
            &mut *dict,
            c"loaded",
            Varnumber::from(buf.b_ml.ml_mfp.is_some()),
        );
        tv_dict_add_nr(&mut *dict, c"listed", buf.b_p_bl as Varnumber);
        tv_dict_add_nr(
            &mut *dict,
            c"changed",
            Varnumber::from(buf_is_changed_fn(buf)),
        );
        tv_dict_add_nr(&mut *dict, c"changedtick", buf_get_changedtick(&*buf));
        tv_dict_add_nr(
            &mut *dict,
            c"hidden",
            Varnumber::from(buf.b_ml.ml_mfp.is_some() && buf.b_nwindows == 0),
        );
        tv_dict_add_nr(
            &mut *dict,
            c"command",
            Varnumber::from(bt_cmdwin(&raw mut *buf)),
        );

        tv_dict_add_dict(&mut *dict, c"variables", buf.b_vars());

        let windows = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_mut(first_win_in_tab(&mut *tp)) {
                if wp.w_buffer == buf {
                    tv_list_append_number(&mut *windows, wp.handle as Varnumber);
                }
            }
        }
        tv_dict_add_list(&mut *dict, c"windows", windows);

        if buf_has_signs(&*buf) {
            tv_dict_add_list(&mut *dict, c"signs", get_buffer_signs(buf));
        }

        tv_dict_add_nr(&mut *dict, c"lastused", buf.b_last_used);

        dict
    }
}

/// `getbufinfo()` function (`f_getbufinfo`).
pub fn f_getbufinfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut argbuf: *mut FileBuf = ptr::null_mut();
        let mut filtered = false;
        let mut sel_buflisted = false;
        let mut sel_bufloaded = false;
        let mut sel_bufmodified = false;

        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

        if argvars[0].v_type == VarType::VarDict {
            let sel_d = argvars[0].vval.v_dict();
            if !sel_d.is_null() {
                filtered = true;
                sel_buflisted = tv_dict_get_bool(sel_d.as_ref(), c"buflisted", 0) != 0;
                sel_bufloaded = tv_dict_get_bool(sel_d.as_ref(), c"bufloaded", 0) != 0;
                sel_bufmodified = tv_dict_get_bool(sel_d.as_ref(), c"bufmodified", 0) != 0;
            }
        } else if argvars[0].v_type != VarType::VarUnknown {
            argbuf = tv_get_buf_from_arg(&mut argvars[0]);
            if argbuf.is_null() {
                return;
            }
        }

        for buf in for_all_buffers_ptr() {
            if !argbuf.is_null() && argbuf != buf {
                continue;
            }
            if filtered
                && ((sel_bufloaded && buf.b_ml.ml_mfp.is_none())
                    || (sel_buflisted && buf.b_p_bl == 0)
                    || (sel_bufmodified && buf.b_changed == 0))
            {
                continue;
            }

            let d = get_buffer_info(&mut *buf);
            tv_list_append_dict(&mut *rettv.vval.v_list(), d);
            if !argbuf.is_null() {
                return;
            }
        }
    }
}

/// Get line or list of lines from buffer `buf` into `rettv` (`get_buffer_lines`).
fn get_buffer_lines(
    mut buf: Option<&mut FileBuf>,
    mut start: Linenr,
    mut end: Linenr,
    retlist: bool,
    rettv: &mut TypVal,
) {
    unsafe {
        rettv.v_type = if retlist {
            VarType::VarList
        } else {
            VarType::VarString
        };
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if buf.is_none()
            || buf.as_deref_mut().unwrap().b_ml.ml_mfp.is_none()
            || start < 0
            || end < start
        {
            if retlist {
                tv_list_alloc_ret(&mut *rettv, 0);
            }
            return;
        }

        if retlist {
            if start < 1 {
                start = 1;
            }
            if end > buf.as_deref_mut().unwrap().b_ml.ml_line_count {
                end = buf.as_deref_mut().unwrap().b_ml.ml_line_count;
            }
            tv_list_alloc_ret(&mut *rettv, (end - start + 1) as isize);
            while start <= end {
                tv_list_append_string(
                    &mut *rettv.vval.v_list(),
                    ml_get_buf(buf.as_deref_mut().unwrap(), start),
                    ml_get_buf_len(buf.as_deref_mut().unwrap(), start) as isize,
                );
                start += 1;
            }
        } else if start >= 1 && start <= buf.as_deref_mut().unwrap().b_ml.ml_line_count {
            crate::eval::typval::tv_set_string_len(
                &mut *rettv,
                ml_get_buf(buf.as_deref_mut().unwrap(), start),
                ml_get_buf_len(buf.unwrap(), start) as usize,
            );
        } else {
            rettv.v_type = VarType::VarString;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

fn getbufline(argvars: &mut [TypVal], rettv: &mut TypVal, retlist: bool) {
    {
        let mut lnum: Linenr = 1;
        let mut end: Linenr = 1;
        let did_emsg_before = g_st().did_emsg;
        let buf = tv_get_buf_from_arg(&mut argvars[0]);
        if !buf.is_null() {
            lnum = tv_get_lnum_buf(&argvars[1], unsafe { buf.as_ref() });
            if g_st().did_emsg > did_emsg_before {
                return;
            }
            end = if argvars[2].v_type == VarType::VarUnknown {
                lnum
            } else {
                tv_get_lnum_buf(&argvars[2], unsafe { buf.as_ref() })
            };
        }

        get_buffer_lines(unsafe { buf.as_mut() }, lnum, end, retlist, &mut *rettv);
    }
}

/// `getbufline()` function (`f_getbufline`).
pub fn f_getbufline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getbufline(argvars, &mut *rettv, true);
}

/// `getbufoneline()` function (`f_getbufoneline`).
pub fn f_getbufoneline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getbufline(argvars, &mut *rettv, false);
}

/// `getline(lnum, [end])` function (`f_getline`).
pub fn f_getline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let lnum = tv_get_lnum(&argvars[0]);
        let (end, retlist) = if argvars[1].v_type == VarType::VarUnknown {
            (lnum, false)
        } else {
            (tv_get_lnum(&argvars[1]), true)
        };

        get_buffer_lines(
            Some(crate::globals::curbuf_mut()),
            lnum,
            end,
            retlist,
            &mut *rettv,
        );
    }
}

/// `setbufline()` function (`f_setbufline`).
pub fn f_setbufline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    buf_set_append_line(argvars, rettv, false);
}

/// `setline()` function (`f_setline`).
pub fn f_setline(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let did_emsg_before = g_st().did_emsg;
    let lnum = tv_get_lnum(&argvars[0]);
    if g_st().did_emsg == did_emsg_before {
        set_buffer_lines(g_st().curbuf, lnum, false, &mut argvars[1], rettv);
    }
}

/// `prompt_setcallback({buffer}, {callback})` function (`f_prompt_setcallback`).
pub fn f_prompt_setcallback(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut prompt_callback = Callback::NONE;

        if check_secure() {
            return;
        }
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            return;
        }

        if !callback_from_typval(&mut prompt_callback, &argvars[1]) {
            return;
        }

        callback_free(&mut (*buf).b_prompt_callback);
        (*buf).b_prompt_callback = prompt_callback;
    }
}

/// `prompt_setinterrupt({buffer}, {callback})` function (`f_prompt_setinterrupt`).
pub fn f_prompt_setinterrupt(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let mut interrupt_callback = Callback::NONE;

        if check_secure() {
            return;
        }
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            return;
        }

        if !callback_from_typval(&mut interrupt_callback, &argvars[1]) {
            return;
        }

        callback_free(&mut (*buf).b_prompt_interrupt);
        (*buf).b_prompt_interrupt = interrupt_callback;
    }
}

/// `prompt_getprompt({buffer})` function (`f_prompt_getprompt`).
pub fn f_prompt_getprompt(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        tv_set_string(rettv, ptr::null());

        let buf = tv_get_buf_from_arg(&mut argvars[0]);
        if buf.is_null() || !bt_prompt(buf.as_ref()) {
            return;
        }

        tv_set_string(rettv, buf_prompt_text(&*buf));
    }
}

/// `prompt_getinput({buffer})` function (`f_prompt_getinput`).
pub fn f_prompt_getinput(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        tv_set_string(rettv, ptr::null());

        let buf = tv_get_buf_from_arg(&mut argvars[0]);
        if buf.is_null() || !bt_prompt(buf.as_ref()) {
            return;
        }

        tv_set_string(
            rettv,
            prompt_get_input(&mut *buf)
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr()),
        );
    }
}

/// `prompt_setprompt({buffer}, {text})` function (`f_prompt_setprompt`).
pub fn f_prompt_setprompt(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        if check_secure() {
            return;
        }
        let buf = tv_get_buf(&mut argvars[0], 0);
        if buf.is_null() {
            return;
        }

        let new_prompt = tv_get_string(&argvars[1]);
        let new_prompt_len = crate::memory::cstr_len(new_prompt) as c_int;

        if bt_prompt(buf.as_ref()) && (*buf).b_ml.ml_mfp.is_some() {
            if (*buf).b_prompt_start.mark.lnum < 1
                || (*buf).b_prompt_start.mark.lnum > curbuf_mut().b_ml.ml_line_count
            {
                (*buf).b_prompt_start.mark.lnum = (*buf)
                    .b_prompt_start
                    .mark
                    .lnum
                    .min((*buf).b_ml.ml_line_count)
                    .max(1);
                curbuf_mut().b_prompt_append_new_line = true;
            }

            let prompt_lno = (*buf).b_prompt_start.mark.lnum;
            let old_prompt = buf_prompt_text(&*buf);
            let old_line = ml_get_buf(&mut *buf, prompt_lno);
            let old_line_len = ml_get_buf_len(&mut *buf, prompt_lno);

            let old_prompt_bytes = crate::memory::cstr_as_bytes(old_prompt);
            let old_prompt_len = old_prompt_bytes.len() as c_int;
            let mut cursor_col = curwin_mut().w_cursor.col;

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
                ml_replace_buf(&mut *buf, prompt_lno, new_prompt.cast_mut(), true, false);
                extmark_splice_cols(
                    &mut *buf,
                    prompt_lno - 1,
                    0,
                    old_line_len,
                    new_prompt_len,
                    K_EXTMARK_NO_UNDO,
                );
                cursor_col = new_prompt_len;
            } else {
                let new_line = concat_str(new_prompt, old_line.add(mark_col as usize));
                let _ = ml_replace_buf(
                    &mut *buf,
                    prompt_lno,
                    new_line.as_ptr().cast_mut(),
                    true,
                    false,
                );
                extmark_splice_cols(
                    &mut *buf,
                    prompt_lno - 1,
                    0,
                    mark_col,
                    new_prompt_len,
                    K_EXTMARK_NO_UNDO,
                );
                cursor_col += new_prompt_len - mark_col;
            }

            if curwin_mut().w_buffer == buf && curwin_mut().w_cursor.lnum == prompt_lno {
                curwin_mut().w_cursor.col = cursor_col;
                check_cursor_col(curwin_mut());
            }
            changed_lines(&mut *buf, prompt_lno, 0, prompt_lno + 1, 0, true);
            u_clearallandblockfree(&mut *buf);
        }

        (*buf).b_prompt_text = Some(crate::memory::cstr_to_cstring(new_prompt));
        (*buf).b_prompt_start.mark.col = new_prompt_len as Colnr;
    }
}
