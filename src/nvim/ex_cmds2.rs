//! Functions for command-line commands (`src/nvim/ex_cmds2.c`).

use crate::arglist::{
    CCGD_AW, CCGD_EXCMD, CCGD_FORCEIT, CCGD_MULTWIN, CMD_argdo, ML_EMPTY, aentry_T, do_argfile,
    editing_arg_idx, ex_all, ex_rewind, set_arglist,
};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_VERBOSE, EVENT_SYNTAX, apply_autocmds, au_event_disable, au_event_restore,
};
use crate::buffer::{
    DOBUF_FIRST, DOBUF_GOTO, DOBUF_UNLOAD, bt_dontwrite, buf_T, buf_hide, buf_set_name, buf_spname,
    buflist_findnr, bufref_T, bufref_valid, first_win_in_tab, goto_buffer, kUIMessages,
    no_write_message, no_write_message_nobang, set_bufref, set_curbuf,
};
use crate::bufwrite::buf_write;
use crate::change::{HLF_W, unchanged};
use crate::channel::channel_job_running;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::dialog::{
    VIM_ALL, VIM_DISCARDALL, VIM_NO, VIM_YES, vim_dialog_yesnoallcancel, vim_dialog_yesnocancel,
};
use crate::errors::*;
use crate::eval::eval_call_provider;
use crate::eval::typval::{
    tv_list_alloc, tv_list_append_number, tv_list_append_string,
    varnumber_T,
};
use crate::eval::vars::{
    VV_SWAPCOMMAND, do_unlet, get_var_value, set_internal_string_var, set_vim_var_string,
};
use crate::ex_cmds::{DIALOG_MSG_SIZE, check_overwrite, set_swapcommand};
use crate::ex_docmd::{CMOD_CONFIRM, do_cmdline, do_cmdline_cmd, exarg_T};
use crate::ex_getln::script_get;
use crate::fileio::{buf_check_timestamp, check_timestamps};
use crate::globals::*;
use crate::mark::setpcmark;
use crate::message::{emsg, msg, msg_source, semsg, wait_return};
use crate::r#move::validate_cursor;
use crate::normal::do_check_scrollbind;
use crate::option_vars::*;
use crate::path::vim_FullName;
use crate::quickfix::{ex_cc, ex_cnext, qf_get_cur_idx, qf_get_valid_size};
use crate::runtime::{DIP_ALL, source_runtime_vim_lua};
use crate::strings::{gettext, vim_snprintf};
use crate::ui::ui_has;
use crate::undo::bufIsChanged;
use crate::window::{
    goto_tabpage_tp, goto_tabpage_win, valid_tabpage, win_goto, win_split, win_valid,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::charset::NUL;
use crate::cursor::linenr_T;
use crate::eval::funcs::VIM_QUESTION;
use crate::math::{FAIL, OK};
use crate::os::fs::MAXPATHL;
use crate::search::FORWARD;

const MAXLNUM: usize = 0x7fffffff;

const CCGD_ALLBUF: c_int = 8;

const CMD_bufdo: c_int = 42;
pub(crate) const CMD_cdo: c_int = 64;
pub(crate) const CMD_cfdo: c_int = 68;
const CMD_first: c_int = 164;
pub(crate) const CMD_ldo: c_int = 231;
pub(crate) const CMD_lfdo: c_int = 237;
const CMD_sfirst: c_int = 410;
const CMD_tabdo: c_int = 461;
const CMD_windo: c_int = 535;

pub(crate) const BF_SYN_SET: c_int = 0x200;

const E_COMPILER_NOT_SUPPORTED_STR: &CStr = c"E666: Compiler not supported: %s";

#[inline]
unsafe fn curwin_argcount() -> c_int {
    unsafe { crate::arglist::argcount() }
}

#[inline]
unsafe fn curwin_arglist() -> *mut aentry_T {
    unsafe { crate::arglist::arglist() }
}

pub unsafe fn ex_ruby(eap: &mut exarg_T) {
    unsafe {
        script_host_execute(c"ruby".as_ptr(), eap);
    }
}

pub unsafe fn ex_rubyfile(eap: &mut exarg_T) {
    unsafe {
        script_host_execute_file(c"ruby".as_ptr(), eap);
    }
}

pub unsafe fn ex_rubydo(eap: &mut exarg_T) {
    unsafe {
        script_host_do_range(c"ruby".as_ptr(), eap);
    }
}

pub unsafe fn ex_python3(eap: &mut exarg_T) {
    unsafe {
        script_host_execute(c"python3".as_ptr(), eap);
    }
}

pub unsafe fn ex_py3file(eap: &mut exarg_T) {
    unsafe {
        script_host_execute_file(c"python3".as_ptr(), eap);
    }
}

pub unsafe fn ex_pydo3(eap: &mut exarg_T) {
    unsafe {
        script_host_do_range(c"python3".as_ptr(), eap);
    }
}

pub unsafe fn ex_perl(eap: &mut exarg_T) {
    unsafe {
        script_host_execute(c"perl".as_ptr(), eap);
    }
}

pub unsafe fn ex_perlfile(eap: &mut exarg_T) {
    unsafe {
        script_host_execute_file(c"perl".as_ptr(), eap);
    }
}

pub unsafe fn ex_perldo(eap: &mut exarg_T) {
    unsafe {
        script_host_do_range(c"perl".as_ptr(), eap);
    }
}

/// If `'autowrite'` option set, try to write the file (`autowrite`).
pub unsafe fn autowrite(buf: &mut buf_T, forceit: bool) -> c_int {
    unsafe {
        if (opt_vars().p_aw == 0 && opt_vars().p_awa == 0)
            || opt_vars().p_write == 0
            || bt_dontwrite(&raw mut *buf)
            || (!forceit && buf.b_p_ro != 0)
            || buf.b_ffname.is_null()
        {
            return FAIL;
        }
        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, &raw mut *buf);
        let mut r = buf_write_all(buf, forceit);

        if bufref_valid(&raw mut bufref) && bufIsChanged(buf) {
            r = FAIL;
        }
        r
    }
}

/// Flush all buffers, except the ones that are readonly or are never written (`autowrite_all`).
pub unsafe fn autowrite_all() {
    unsafe {
        if (opt_vars().p_aw == 0 && opt_vars().p_awa == 0) || opt_vars().p_write == 0 {
            return;
        }

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if bufIsChanged(&mut *buf) && (*buf).b_p_ro == 0 && !bt_dontwrite(buf) {
                let mut bufref: bufref_T = bufref_T::new();
                set_bufref(&mut bufref, buf);
                buf_write_all(&mut *buf, false);
                if !bufref_valid(&raw mut bufref) {
                    buf = g_st().firstbuf;
                    continue;
                }
            }
            buf = (*buf).b_next;
        }
    }
}

/// Return true if buffer was changed and cannot be abandoned (`check_changed`).
pub unsafe fn check_changed(buf: &mut buf_T, flags: c_int) -> bool {
    unsafe {
        let forceit = (flags & CCGD_FORCEIT) != 0;
        let mut bufref: bufref_T = bufref_T::new();
        set_bufref(&mut bufref, &raw mut *buf);

        if !forceit
            && bufIsChanged(buf)
            && ((flags & CCGD_MULTWIN) != 0 || buf.b_nwindows <= 1)
            && ((flags & CCGD_AW) == 0 || autowrite(buf, forceit) == FAIL)
        {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && opt_vars().p_write != 0 {
                let mut count: c_int = 0;

                if (flags & CCGD_ALLBUF) != 0 {
                    let mut buf2 = g_st().firstbuf;
                    while !buf2.is_null() {
                        if bufIsChanged(&mut *buf2) && !(*buf2).b_ffname.is_null() {
                            count += 1;
                        }
                        buf2 = (*buf2).b_next;
                    }
                }
                if !bufref_valid(&raw mut bufref) {
                    return false;
                }
                dialog_changed(buf, count > 1);
                if !bufref_valid(&raw mut bufref) {
                    return false;
                }
                return bufIsChanged(buf);
            }
            if (flags & CCGD_EXCMD) != 0 {
                no_write_message();
            } else {
                no_write_message_nobang(&*g_st().curbuf);
            }
            return true;
        }
        false
    }
}

/// Ask the user what to do when abandoning a changed buffer (`dialog_changed`).
pub unsafe fn dialog_changed(buf: &mut buf_T, checkall: bool) {
    unsafe {
        let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
        let mut ea: exarg_T = exarg_T::new();

        let fname = if !buf.b_fname.is_null() {
            buf.b_fname.cast_const()
        } else {
            gettext(c"Untitled".as_ptr())
        };
        vim_snprintf!(
            buff.as_mut_ptr(),
            DIALOG_MSG_SIZE,
            gettext(c"Save changes to \"%s\"?".as_ptr()),
            fname,
        );
        let ret = if checkall {
            vim_dialog_yesnoallcancel(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1)
        } else {
            vim_dialog_yesnocancel(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1)
        };

        if ret == VIM_YES {
            let empty_bufname = buf.b_fname.is_null();
            if empty_bufname {
                buf_set_name(buf.b_fnum, c"Untitled".as_ptr().cast_mut());
            }

            if check_overwrite(&mut ea, buf, buf.b_fname, buf.b_ffname, false) == OK
                && buf_write_all(buf, false) == OK
            {
                return;
            }

            if empty_bufname {
                buf.b_fname = ptr::null_mut();
                buf.clear_fnames();
            }
        } else if ret == VIM_NO {
            unchanged(&raw mut *buf, true, false);
        } else if ret == VIM_ALL {
            let mut buf2 = g_st().firstbuf;
            while !buf2.is_null() {
                if bufIsChanged(&mut *buf2) && !(*buf2).b_ffname.is_null() && (*buf2).b_p_ro == 0 {
                    let mut bufref: bufref_T = bufref_T::new();
                    set_bufref(&mut bufref, buf2);

                    if !(*buf2).b_fname.is_null()
                        && check_overwrite(
                            &mut ea,
                            &mut *buf2,
                            (*buf2).b_fname,
                            (*buf2).b_ffname,
                            false,
                        ) == OK
                    {
                        buf_write_all(&mut *buf2, false);
                    }
                    if !bufref_valid(&raw mut bufref) {
                        buf2 = g_st().firstbuf;
                        continue;
                    }
                }
                buf2 = (*buf2).b_next;
            }
        } else if ret == VIM_DISCARDALL {
            let mut buf2 = g_st().firstbuf;
            while !buf2.is_null() {
                unchanged(buf2, true, false);
                buf2 = (*buf2).b_next;
            }
        }
    }
}

/// Ask the user whether to close the terminal buffer or not (`dialog_close_terminal`).
pub unsafe fn dialog_close_terminal(buf: &mut buf_T) -> bool {
    unsafe {
        let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
        let fname = if !buf.b_fname.is_null() {
            buf.b_fname.cast_const()
        } else {
            c"?".as_ptr()
        };
        vim_snprintf!(
            buff.as_mut_ptr(),
            DIALOG_MSG_SIZE,
            gettext(c"Close \"%s\"?".as_ptr()),
            fname,
        );
        vim_dialog_yesnocancel(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1) == VIM_YES
    }
}

/// Return true if the buffer `buf` can be abandoned (`can_abandon`).
pub unsafe fn can_abandon(buf: &mut buf_T, forceit: bool) -> bool {
    unsafe {
        buf_hide(buf)
            || !bufIsChanged(buf)
            || buf.b_nwindows > 1
            || autowrite(buf, forceit) == OK
            || forceit
    }
}

fn add_bufnum(bufnrs: &mut Vec<c_int>, nr: c_int) {
    if !bufnrs.contains(&nr) {
        bufnrs.push(nr);
    }
}

/// Check if any buffer was changed and cannot be abandoned (`check_changed_any`).
pub unsafe fn check_changed_any(hidden: bool, unload: bool) -> bool {
    unsafe {
        let mut ret = false;
        let mut bufcount: usize = 0;

        let mut b = g_st().firstbuf;
        while !b.is_null() {
            bufcount += 1;
            b = (*b).b_next;
        }
        if bufcount == 0 {
            return false;
        }

        let mut bufnrs = Vec::with_capacity(bufcount);
        bufnrs.push((*g_st().curbuf).b_fnum);

        let mut wp = g_st().firstwin;
        while !wp.is_null() {
            if (*wp).w_buffer != g_st().curbuf {
                add_bufnum(&mut bufnrs, (*(*wp).w_buffer).b_fnum);
            }
            wp = (*wp).w_next;
        }

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            if tp != g_st().curtab {
                let mut wp2 = first_win_in_tab(tp);
                while !wp2.is_null() {
                    add_bufnum(&mut bufnrs, (*(*wp2).w_buffer).b_fnum);
                    wp2 = (*wp2).w_next;
                }
            }
            tp = (*tp).tp_next;
        }

        let mut b = g_st().firstbuf;
        while !b.is_null() {
            add_bufnum(&mut bufnrs, (*b).b_fnum);
            b = (*b).b_next;
        }

        let mut buf: *mut buf_T = ptr::null_mut();
        let mut i: usize = 0;
        while i < bufnrs.len() {
            buf = buflist_findnr(bufnrs[i]);
            if !buf.is_null() && (!hidden || (*buf).b_nwindows == 0) && bufIsChanged(&mut *buf) {
                let mut bufref: bufref_T = bufref_T::new();
                set_bufref(&mut bufref, buf);

                if check_changed(
                    &mut *buf,
                    (if opt_vars().p_awa != 0 { CCGD_AW } else { 0 }) | CCGD_MULTWIN | CCGD_ALLBUF,
                ) && bufref_valid(&raw mut bufref)
                {
                    break;
                }
            }
            i += 1;
        }

        if i < bufnrs.len() {
            ret = true;
            g_st().exiting = false;
            if opt_vars().p_confirm == 0 && (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) == 0 {
                if g_st().vgetc_busy > 0 {
                    g_st().msg_row = g_st().cmdline_row;
                    g_st().msg_col = 0;
                    g_st().msg_didout = false;
                }
                let shown = if !(*buf).terminal.is_null()
                    && channel_job_running((*buf).b_p_channel as u64)
                {
                    semsg!(
                        gettext(c"E947: Job still running in buffer \"%s\"".as_ptr()),
                        (*buf).b_fname,
                    )
                } else {
                    let sp = buf_spname(buf);
                    semsg!(
                        gettext(c"E162: No write since last change for buffer \"%s\"".as_ptr()),
                        if !sp.is_null() { sp } else { (*buf).b_fname },
                    )
                };
                if shown && g_st().msg_didany {
                    let save = g_st().no_wait_return;
                    g_st().no_wait_return = 0;
                    if !ui_has(kUIMessages) {
                        wait_return(0);
                    }
                    g_st().no_wait_return = save;
                }
            }

            let mut wiped = false;
            if buf != g_st().curbuf {
                let mut found = false;
                let mut tp = g_st().first_tabpage;
                while !tp.is_null() && !found {
                    let mut wp = first_win_in_tab(tp);
                    while !wp.is_null() {
                        if (*wp).w_buffer == buf {
                            let mut bufref: bufref_T = bufref_T::new();
                            set_bufref(&mut bufref, buf);
                            goto_tabpage_win(&mut *tp, wp);
                            if !bufref_valid(&raw mut bufref) {
                                wiped = true;
                            }
                            found = true;
                            break;
                        }
                        wp = (*wp).w_next;
                    }
                    tp = (*tp).tp_next;
                }
            }

            if !wiped && buf != g_st().curbuf {
                set_curbuf(buf, if unload { DOBUF_UNLOAD } else { DOBUF_GOTO }, true);
            }
        }

        ret
    }
}

/// Return `FAIL` if there is no file name, `OK` if there is one (`check_fname`).
pub unsafe fn check_fname() -> c_int {
    unsafe {
        if (*g_st().curbuf).b_ffname.is_null() {
            emsg(gettext((&raw const e_noname).cast()));
            return FAIL;
        }
        OK
    }
}

/// Flush the contents of a buffer, unless it has no file name (`buf_write_all`).
pub unsafe fn buf_write_all(buf: &mut buf_T, forceit: bool) -> c_int {
    unsafe {
        let old_curbuf = g_st().curbuf;
        let retval = buf_write(
            &raw mut *buf,
            buf.b_ffname,
            buf.b_fname,
            1,
            buf.b_ml.ml_line_count,
            ptr::null_mut(),
            false,
            forceit,
            true,
            false,
        );
        if g_st().curbuf != old_curbuf {
            msg_source(HLF_W);
            msg(
                gettext(
                    c"Warning: Entered other buffer unexpectedly (check autocommands)".as_ptr(),
                ),
                0,
            );
        }
        retval
    }
}

/// `:argdo`, `:windo`, `:bufdo`, `:tabdo`, `:cdo`, `:ldo`, `:cfdo` and `:lfdo` (`ex_listdo`).
pub unsafe fn ex_listdo(eap: &mut exarg_T) {
    unsafe {
        if (**curwin.as_ptr()).w_onebuf_opt.wo_wfb != 0
            && eap.cmdidx != CMD_windo
            && eap.cmdidx != CMD_tabdo
        {
            if (eap.cmdidx == CMD_ldo || eap.cmdidx == CMD_lfdo) && eap.forceit == 0 {
                emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                return;
            }

            if win_valid(g_st().prevwin) && (*g_st().prevwin).w_onebuf_opt.wo_wfb == 0 {
                win_goto(g_st().prevwin);
            }
            if (**curwin.as_ptr()).w_onebuf_opt.wo_wfb != 0 {
                win_split(0, 0);
                if (**curwin.as_ptr()).w_onebuf_opt.wo_wfb != 0 {
                    emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                    return;
                }
            }
        }

        let mut save_ei: Option<std::ffi::CString> = None;

        g_st().msg_listdo_overwrite += 1;

        if eap.cmdidx != CMD_windo && eap.cmdidx != CMD_tabdo {
            save_ei = Some(au_event_disable(c",Syntax".as_ptr().cast_mut()));

            let mut buf = g_st().firstbuf;
            while !buf.is_null() {
                (*buf).b_flags &= !BF_SYN_SET;
                buf = (*buf).b_next;
            }
        }

        if eap.cmdidx == CMD_windo
            || eap.cmdidx == CMD_tabdo
            || buf_hide(&*g_st().curbuf)
            || !check_changed(
                &mut *g_st().curbuf,
                CCGD_AW | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
            )
        {
            let mut next_fnum: c_int = 0;
            let mut i: c_int = 0;
            let mut wp = g_st().firstwin;
            let mut tp = g_st().first_tabpage;
            match eap.cmdidx {
                CMD_windo => {
                    while !wp.is_null() && i + 1 < eap.line1 {
                        i += 1;
                        wp = (*wp).w_next;
                    }
                }
                CMD_tabdo => {
                    while !tp.is_null() && i + 1 < eap.line1 {
                        i += 1;
                        tp = (*tp).tp_next;
                    }
                }
                CMD_argdo => {
                    i = eap.line1 - 1;
                }
                _ => {}
            }

            let mut buf = g_st().curbuf;
            let mut qf_size: usize = 0;

            if eap.cmdidx == CMD_bufdo {
                buf = g_st().firstbuf;
                while !buf.is_null() && ((*buf).b_fnum < eap.line1 || (*buf).b_p_bl == 0) {
                    if (*buf).b_fnum > eap.line2 {
                        buf = ptr::null_mut();
                        break;
                    }
                    buf = (*buf).b_next;
                }
                if !buf.is_null() {
                    goto_buffer(eap, DOBUF_FIRST, FORWARD, (*buf).b_fnum);
                }
            } else if eap.cmdidx == CMD_cdo
                || eap.cmdidx == CMD_ldo
                || eap.cmdidx == CMD_cfdo
                || eap.cmdidx == CMD_lfdo
            {
                qf_size = qf_get_valid_size(eap);
                debug_assert!(eap.line1 >= 0);
                if qf_size == 0 || (eap.line1 as usize) > qf_size {
                    buf = ptr::null_mut();
                } else {
                    ex_cc(eap);

                    buf = g_st().curbuf;
                    i = eap.line1 - 1;
                    if eap.addr_count <= 0 {
                        debug_assert!(qf_size < MAXLNUM);
                        eap.line2 = qf_size as linenr_T;
                    }
                }
            } else {
                setpcmark();
            }
            g_st().listcmd_busy = true;

            while !g_st().got_int && !buf.is_null() {
                let mut execute = true;
                if eap.cmdidx == CMD_argdo {
                    if i == curwin_argcount() {
                        break;
                    }
                    if (**curwin.as_ptr()).w_arg_idx != i || !editing_arg_idx(&mut **curwin.as_ptr()) {
                        do_argfile(eap, i);
                    }
                    if (**curwin.as_ptr()).w_arg_idx != i {
                        break;
                    }
                } else if eap.cmdidx == CMD_windo {
                    if !win_valid(wp) {
                        break;
                    }
                    debug_assert!(!wp.is_null());
                    execute =
                        !(*wp).w_floating || (!(*wp).w_config.hide && (*wp).w_config.focusable);
                    if execute {
                        win_goto(wp);
                        if *curwin.as_ptr() != wp {
                            break;
                        }
                    }
                    wp = (*wp).w_next;
                } else if eap.cmdidx == CMD_tabdo {
                    if !valid_tabpage(tp) {
                        break;
                    }
                    debug_assert!(!tp.is_null());
                    goto_tabpage_tp(tp, true, true);
                    tp = (*tp).tp_next;
                } else if eap.cmdidx == CMD_bufdo {
                    next_fnum = -1;
                    let mut bp = (*g_st().curbuf).b_next;
                    while !bp.is_null() {
                        if (*bp).b_p_bl != 0 {
                            next_fnum = (*bp).b_fnum;
                            break;
                        }
                        bp = (*bp).b_next;
                    }
                }

                i += 1;
                if execute {
                    do_cmdline(eap.arg, eap.ea_getline, DOCMD_VERBOSE + DOCMD_NOWAIT);
                }

                if eap.cmdidx == CMD_bufdo {
                    if next_fnum < 0 || next_fnum > eap.line2 {
                        break;
                    }

                    let mut buf_still_exists = false;
                    let mut bp = g_st().firstbuf;
                    while !bp.is_null() {
                        if (*bp).b_fnum == next_fnum {
                            buf_still_exists = true;
                            break;
                        }
                        bp = (*bp).b_next;
                    }
                    if !buf_still_exists {
                        break;
                    }

                    goto_buffer(eap, DOBUF_FIRST, FORWARD, next_fnum);

                    if (*g_st().curbuf).b_fnum != next_fnum {
                        break;
                    }
                }

                if eap.cmdidx == CMD_cdo
                    || eap.cmdidx == CMD_ldo
                    || eap.cmdidx == CMD_cfdo
                    || eap.cmdidx == CMD_lfdo
                {
                    debug_assert!(i >= 0);
                    if (i as usize) >= qf_size || i >= eap.line2 {
                        break;
                    }

                    let qf_idx = qf_get_cur_idx(eap);
                    ex_cnext(eap);
                    if qf_get_cur_idx(eap) == qf_idx {
                        break;
                    }
                }

                if eap.cmdidx == CMD_windo && execute {
                    validate_cursor(*curwin.as_ptr());
                    if (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
                        do_check_scrollbind(true);
                    }
                }
                if (eap.cmdidx == CMD_windo || eap.cmdidx == CMD_tabdo)
                    && i + 1 > eap.line2
                {
                    break;
                }
                if eap.cmdidx == CMD_argdo && i >= eap.line2 {
                    break;
                }
            }
            g_st().listcmd_busy = false;
        }

        g_st().msg_listdo_overwrite -= 1;
        if save_ei.is_some() {
            let mut aco: CtxSwitch = CtxSwitch::new();

            au_event_restore(save_ei);

            let mut buf = g_st().firstbuf;
            while !buf.is_null() {
                let mut bnext = (*buf).b_next;
                if (*buf).b_nwindows > 0 && ((*buf).b_flags & BF_SYN_SET) != 0 {
                    (*buf).b_flags &= !BF_SYN_SET;

                    if buf == g_st().curbuf {
                        apply_autocmds(
                            EVENT_SYNTAX,
                            (*g_st().curbuf).b_p_syn,
                            (*g_st().curbuf).b_fname,
                            true,
                            g_st().curbuf,
                        );
                    } else {
                        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
                        apply_autocmds(EVENT_SYNTAX, (*buf).b_p_syn, (*buf).b_fname, true, buf);
                        ctx_restore(&mut aco);
                    }

                    bnext = g_st().firstbuf;
                }
                buf = bnext;
            }
        }
    }
}

/// `:compiler[!] {name}` (`ex_compiler`).
pub unsafe fn ex_compiler(eap: &mut exarg_T) {
    unsafe {
        let mut old_cur_comp: Option<std::ffi::CString> = None;

        if *eap.arg == NUL {
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.vim')".as_ptr());
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.lua')".as_ptr());
            return;
        }

        let arg_len = CStr::from_ptr(eap.arg).to_bytes().len();
        let bufsize = arg_len + 14;
        let mut buf = vec![0 as c_char; bufsize];

        if eap.forceit != 0 {
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet set <args>".as_ptr());
        } else {
            let p = get_var_value(c"g:current_compiler".as_ptr());
            if !p.is_null() {
                old_cur_comp = Some(CStr::from_ptr(p).to_owned());
            }
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet setlocal <args>".as_ptr());
        }
        do_unlet(c"g:current_compiler".as_ptr(), 18, true);
        do_unlet(c"b:current_compiler".as_ptr(), 18, true);

        vim_snprintf!(
            buf.as_mut_ptr(),
            bufsize,
            c"compiler/%s.*".as_ptr(),
            eap.arg,
        );
        if source_runtime_vim_lua(buf.as_mut_ptr(), DIP_ALL) == FAIL {
            semsg!(gettext(E_COMPILER_NOT_SUPPORTED_STR.as_ptr()), eap.arg);
        }
        drop(buf);

        do_cmdline_cmd(c":delcommand CompilerSet".as_ptr());

        let p = get_var_value(c"g:current_compiler".as_ptr());
        if !p.is_null() {
            set_internal_string_var(c"b:current_compiler".as_ptr(), p);
        }

        if eap.forceit == 0 {
            if let Some(ref old_comp) = old_cur_comp {
                set_internal_string_var(c"g:current_compiler".as_ptr(), old_comp.as_ptr());
            } else {
                do_unlet(c"g:current_compiler".as_ptr(), 18, true);
            }
        }
    }
}

/// `:checktime [buffer]` (`ex_checktime`).
pub unsafe fn ex_checktime(eap: &mut exarg_T) {
    unsafe {
        let save_no_check_timestamps = g_st().no_check_timestamps;

        g_st().no_check_timestamps = 0;
        if eap.addr_count == 0 {
            check_timestamps(0);
        } else {
            let buf = buflist_findnr(eap.line2 as c_int);
            if !buf.is_null() {
                buf_check_timestamp(&mut *buf);
            }
        }
        g_st().no_check_timestamps = save_no_check_timestamps;
    }
}

unsafe fn script_host_execute(name: *const c_char, eap: &mut exarg_T) {
    unsafe {
        let mut len: usize = 0;
        if let Some(script) = script_get(eap, &mut len) {
            let args = tv_list_alloc(3);
            crate::eval::typval::tv_list_append_string(args, script.as_ptr(), -1);
            tv_list_append_number(args, eap.line1 as varnumber_T);
            tv_list_append_number(args, eap.line2 as varnumber_T);

            eval_call_provider(name.cast_mut(), c"execute".as_ptr().cast_mut(), args, true);
        }
    }
}

unsafe fn script_host_execute_file(name: *const c_char, eap: &mut exarg_T) {
    unsafe {
        if eap.skip == 0 {
            let mut buffer = [0 as c_char; MAXPATHL];
            vim_FullName(eap.arg, buffer.as_mut_ptr(), MAXPATHL, false);

            let args = tv_list_alloc(3);
            tv_list_append_string(args, buffer.as_ptr(), -1);
            tv_list_append_number(args, eap.line1 as varnumber_T);
            tv_list_append_number(args, eap.line2 as varnumber_T);
            eval_call_provider(
                name.cast_mut(),
                c"execute_file".as_ptr().cast_mut(),
                args,
                true,
            );
        }
    }
}

unsafe fn script_host_do_range(name: *const c_char, eap: &mut exarg_T) {
    unsafe {
        if eap.skip == 0 {
            let args = tv_list_alloc(3);
            tv_list_append_number(args, eap.line1 as varnumber_T);
            tv_list_append_number(args, eap.line2 as varnumber_T);
            tv_list_append_string(args, eap.arg, -1);
            eval_call_provider(name.cast_mut(), c"do_range".as_ptr().cast_mut(), args, true);
        }
    }
}

/// `:drop` (`ex_drop`).
pub unsafe fn ex_drop(eap: &mut exarg_T) {
    unsafe {
        let mut split = false;

        set_arglist(eap.arg);

        if curwin_argcount() == 0 {
            return;
        }

        if g_st().cmdmod.cmod_tab != 0 {
            ex_all(eap);
            g_st().cmdmod.cmod_tab = 0;
            ex_rewind(eap);
            return;
        }

        let buf = buflist_findnr((*curwin_arglist()).ae_fnum);

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    goto_tabpage_win(&mut *tp, wp);
                    (**curwin.as_ptr()).w_arg_idx = 0;
                    if !bufIsChanged(&mut *g_st().curbuf) {
                        let save_ar = (*g_st().curbuf).b_p_ar;
                        (*g_st().curbuf).b_p_ar = 1;
                        buf_check_timestamp(&mut *g_st().curbuf);
                        (*g_st().curbuf).b_p_ar = save_ar;
                    }
                    if ((*g_st().curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
                        ex_rewind(eap);
                    }

                    if !eap.do_ecmd_cmd.is_null() {
                        let did_set_swapcommand = set_swapcommand(eap.do_ecmd_cmd, 0);
                        do_cmdline(
                            eap.do_ecmd_cmd,
                            crate::cmdexpand::LineGetter::None,
                            DOCMD_VERBOSE,
                        );
                        if did_set_swapcommand {
                            set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);
                        }
                    }
                    return;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        if !buf_hide(&*g_st().curbuf) {
            g_st().emsg_off += 1;
            split = check_changed(&mut *g_st().curbuf, CCGD_AW | CCGD_EXCMD);
            g_st().emsg_off -= 1;
        }

        if split {
            eap.cmdidx = CMD_sfirst;
            *eap.cmd = b's' as c_char;
        } else {
            eap.cmdidx = CMD_first;
        }
        ex_rewind(eap);
    }
}
