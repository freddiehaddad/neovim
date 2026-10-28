//! Functions for command-line commands (`src/nvim/ex_cmds2.c`).

use crate::arglist::{
    Aentry, CCGD_AW, CCGD_EXCMD, CCGD_FORCEIT, CCGD_MULTWIN, CMD_ARGDO, ML_EMPTY, do_argfile,
    editing_arg_idx, ex_all, ex_rewind, set_arglist,
};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_VERBOSE, EVENT_SYNTAX, apply_autocmds, au_event_disable, au_event_restore,
};
use crate::buffer::{
    Bufref, DOBUF_FIRST, DOBUF_GOTO, DOBUF_UNLOAD, FileBuf, K_UIMESSAGES, bt_dontwrite, buf_hide,
    buf_set_name, buf_spname, buflist_findnr, bufref_valid, first_win_in_tab, goto_buffer,
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
use crate::eval::typval::{Varnumber, tv_list_alloc, tv_list_append_number, tv_list_append_string};
use crate::eval::vars::{
    VV_SWAPCOMMAND, do_unlet, get_var_value, set_internal_string_var, set_vim_var_string,
};
use crate::ex_cmds::{DIALOG_MSG_SIZE, check_overwrite, set_swapcommand};
use crate::ex_docmd::{CMOD_CONFIRM, Exarg, do_cmdline, do_cmdline_cmd};
use crate::ex_getln::script_get;
use crate::fileio::{buf_check_timestamp, check_timestamps};
use crate::globals::*;
use crate::mark::setpcmark;
use crate::message::{emsg, msg, msg_source, semsg, wait_return};
use crate::r#move::validate_cursor;
use crate::normal::do_check_scrollbind;
use crate::option_vars::*;
use crate::path::vim_full_name;
use crate::quickfix::{ex_cc, ex_cnext, qf_get_cur_idx, qf_get_valid_size};
use crate::runtime::{DIP_ALL, source_runtime_vim_lua};
use crate::strings::{gettext, vim_snprintf};
use crate::ui::ui_has;
use crate::undo::buf_is_changed_fn;
use crate::window::{
    goto_tabpage_tp, goto_tabpage_win, valid_tabpage, win_goto, win_split, win_valid,
};
use std::ffi::{CStr, c_char};
use std::ptr;

use crate::cursor::Linenr;
use crate::eval::funcs::VIM_QUESTION;
use crate::math::OK;
use crate::os::fs::MAXPATHL;
use crate::search::FORWARD;

const MAXLNUM: usize = 0x7fffffff;

const CCGD_ALLBUF: i32 = 8;

const CMD_BUFDO: i32 = 42;
pub(crate) const CMD_CDO: i32 = 64;
pub(crate) const CMD_CFDO: i32 = 68;
const CMD_FIRST: i32 = 164;
pub(crate) const CMD_LDO: i32 = 231;
pub(crate) const CMD_LFDO: i32 = 237;
const CMD_SFIRST: i32 = 410;
const CMD_TABDO: i32 = 461;
const CMD_WINDO: i32 = 535;

pub(crate) const BF_SYN_SET: i32 = 0x200;

const E_COMPILER_NOT_SUPPORTED_STR: &CStr = c"E666: Compiler not supported: %s";

#[inline]
fn curwin_argcount() -> i32 {
    crate::arglist::argcount()
}

#[inline]
fn curwin_arglist() -> *mut Aentry {
    crate::arglist::arglist()
}

pub fn ex_ruby(eap: &mut Exarg) {
    script_host_execute(c"ruby", eap);
}

pub fn ex_rubyfile(eap: &mut Exarg) {
    script_host_execute_file(c"ruby", eap);
}

pub fn ex_rubydo(eap: &mut Exarg) {
    script_host_do_range(c"ruby", eap);
}

pub fn ex_python3(eap: &mut Exarg) {
    script_host_execute(c"python3", eap);
}

pub fn ex_py3file(eap: &mut Exarg) {
    script_host_execute_file(c"python3", eap);
}

pub fn ex_pydo3(eap: &mut Exarg) {
    script_host_do_range(c"python3", eap);
}

pub fn ex_perl(eap: &mut Exarg) {
    script_host_execute(c"perl", eap);
}

pub fn ex_perlfile(eap: &mut Exarg) {
    script_host_execute_file(c"perl", eap);
}

pub fn ex_perldo(eap: &mut Exarg) {
    script_host_do_range(c"perl", eap);
}

/// If `'autowrite'` option set, try to write the file (`autowrite`).
pub fn autowrite(buf: &mut FileBuf, forceit: bool) -> bool {
    if (opt_vars().p_aw == 0 && opt_vars().p_awa == 0)
        || opt_vars().p_write == 0
        || bt_dontwrite(Some(&*buf))
        || (!forceit && buf.b_p_ro != 0)
        || buf.b_ffname.is_null()
    {
        return false;
    }
    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, &raw mut *buf);
    let mut r = buf_write_all(buf, forceit) == OK;

    if bufref_valid(Some(&bufref)) && buf_is_changed_fn(buf) {
        r = false;
    }
    r
}

/// Flush all buffers, except the ones that are readonly or are never written (`autowrite_all`).
pub fn autowrite_all() {
    unsafe {
        if (opt_vars().p_aw == 0 && opt_vars().p_awa == 0) || opt_vars().p_write == 0 {
            return;
        }

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if buf_is_changed_fn(&mut *buf) && (*buf).b_p_ro == 0 && !bt_dontwrite(buf.as_ref()) {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, buf);
                buf_write_all(&mut *buf, false);
                if !bufref_valid(Some(&bufref)) {
                    buf = g_st().firstbuf;
                    continue;
                }
            }
            buf = (*buf).b_next;
        }
    }
}

/// Return true if buffer was changed and cannot be abandoned (`check_changed`).
pub fn check_changed(buf: &mut FileBuf, flags: i32) -> bool {
    {
        let forceit = (flags & CCGD_FORCEIT) != 0;
        let mut bufref: Bufref = Bufref::new();
        set_bufref(&mut bufref, &raw mut *buf);

        if !forceit
            && buf_is_changed_fn(buf)
            && ((flags & CCGD_MULTWIN) != 0 || buf.b_nwindows <= 1)
            && ((flags & CCGD_AW) == 0 || !autowrite(buf, forceit))
        {
            if (opt_vars().p_confirm != 0 || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
                && opt_vars().p_write != 0
            {
                let mut count: i32 = 0;

                if (flags & CCGD_ALLBUF) != 0 {
                    for buf2 in for_all_buffers_mut() {
                        if buf_is_changed_fn(buf2) && !buf2.b_ffname.is_null() {
                            count += 1;
                        }
                    }
                }
                if !bufref_valid(Some(&bufref)) {
                    return false;
                }
                dialog_changed(buf, count > 1);
                if !bufref_valid(Some(&bufref)) {
                    return false;
                }
                return buf_is_changed_fn(buf);
            }
            if (flags & CCGD_EXCMD) != 0 {
                no_write_message();
            } else {
                no_write_message_nobang(curbuf_mut());
            }
            return true;
        }
        false
    }
}

/// Ask the user what to do when abandoning a changed buffer (`dialog_changed`).
pub fn dialog_changed(buf: &mut FileBuf, checkall: bool) {
    unsafe {
        let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
        let mut ea: Exarg = Exarg::new();

        let fname = if !buf.b_fname.is_null() {
            buf.b_fname.cast_const()
        } else {
            gettext(c"Untitled")
        };
        vim_snprintf!(
            buff.as_mut_ptr(),
            DIALOG_MSG_SIZE,
            gettext(c"Save changes to \"%s\"?"),
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
                buf_set_name(buf.b_fnum, c"Untitled");
            }

            if check_overwrite(&mut ea, buf, buf.b_fname, buf.b_ffname, false)
                && buf_write_all(buf, false) == OK
            {
                return;
            }

            if empty_bufname {
                buf.b_fname = ptr::null_mut();
                buf.clear_fnames();
            }
        } else if ret == VIM_NO {
            unchanged(buf, true, false);
        } else if ret == VIM_ALL {
            let mut buf2 = g_st().firstbuf;
            while !buf2.is_null() {
                if buf_is_changed_fn(&mut *buf2)
                    && !(*buf2).b_ffname.is_null()
                    && (*buf2).b_p_ro == 0
                {
                    let mut bufref: Bufref = Bufref::new();
                    set_bufref(&mut bufref, buf2);

                    if !(*buf2).b_fname.is_null()
                        && check_overwrite(
                            &mut ea,
                            &mut *buf2,
                            (*buf2).b_fname,
                            (*buf2).b_ffname,
                            false,
                        )
                    {
                        buf_write_all(&mut *buf2, false);
                    }
                    if !bufref_valid(Some(&bufref)) {
                        buf2 = g_st().firstbuf;
                        continue;
                    }
                }
                buf2 = (*buf2).b_next;
            }
        } else if ret == VIM_DISCARDALL {
            for buf2 in for_all_buffers_ptr() {
                unchanged(&mut *buf2, true, false);
            }
        }
    }
}

/// Ask the user whether to close the terminal buffer or not (`dialog_close_terminal`).
pub fn dialog_close_terminal(buf: &mut FileBuf) -> bool {
    let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
    let fname = if !buf.b_fname.is_null() {
        buf.b_fname.cast_const()
    } else {
        c"?".as_ptr()
    };
    vim_snprintf!(
        buff.as_mut_ptr(),
        DIALOG_MSG_SIZE,
        gettext(c"Close \"%s\"?"),
        fname,
    );
    vim_dialog_yesnocancel(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1) == VIM_YES
}

/// Return true if the buffer `buf` can be abandoned (`can_abandon`).
pub fn can_abandon(buf: &mut FileBuf, forceit: bool) -> bool {
    buf_hide(buf)
        || !buf_is_changed_fn(buf)
        || buf.b_nwindows > 1
        || autowrite(buf, forceit)
        || forceit
}

fn add_bufnum(bufnrs: &mut Vec<i32>, nr: i32) {
    if !bufnrs.contains(&nr) {
        bufnrs.push(nr);
    }
}

/// Check if any buffer was changed and cannot be abandoned (`check_changed_any`).
pub fn check_changed_any(hidden: bool, unload: bool) -> bool {
    unsafe {
        let mut ret = false;
        let mut bufcount: usize = 0;

        for _b in for_all_buffers_mut() {
            bufcount += 1;
        }
        if bufcount == 0 {
            return false;
        }

        let mut bufnrs = Vec::with_capacity(bufcount);
        bufnrs.push(curbuf_mut().b_fnum);

        for wp in for_all_windows_mut() {
            if wp.w_buffer != g_st().curbuf {
                add_bufnum(&mut bufnrs, wp.buf_mut().b_fnum);
            }
        }

        for tp in for_all_tabpages_ptr() {
            if !std::ptr::eq(tp, g_st().curtab) {
                for wp2 in win_iter_mut(first_win_in_tab(&mut *tp)) {
                    add_bufnum(&mut bufnrs, wp2.buf_mut().b_fnum);
                }
            }
        }

        for b in for_all_buffers_mut() {
            add_bufnum(&mut bufnrs, b.b_fnum);
        }

        let mut buf: *mut FileBuf = ptr::null_mut();
        let mut i: usize = 0;
        while i < bufnrs.len() {
            buf = buflist_findnr(bufnrs[i]);
            if !buf.is_null() && (!hidden || (*buf).b_nwindows == 0) && buf_is_changed_fn(&mut *buf)
            {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, buf);

                if check_changed(
                    &mut *buf,
                    (if opt_vars().p_awa != 0 { CCGD_AW } else { 0 }) | CCGD_MULTWIN | CCGD_ALLBUF,
                ) && bufref_valid(Some(&bufref))
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
                        gettext(c"E947: Job still running in buffer \"%s\""),
                        (*buf).b_fname,
                    )
                } else {
                    let sp = buf_spname(&*(buf));
                    semsg!(
                        gettext(c"E162: No write since last change for buffer \"%s\""),
                        if !sp.is_null() { sp } else { (*buf).b_fname },
                    )
                };
                if shown && g_st().msg_didany {
                    let save = g_st().no_wait_return;
                    g_st().no_wait_return = 0;
                    if !ui_has(K_UIMESSAGES) {
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
                    for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
                        if wp.w_buffer == buf {
                            let mut bufref: Bufref = Bufref::new();
                            set_bufref(&mut bufref, buf);
                            goto_tabpage_win(&mut *tp, wp);
                            if !bufref_valid(Some(&bufref)) {
                                wiped = true;
                            }
                            found = true;
                            break;
                        }
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
pub fn check_fname() -> bool {
    {
        if curbuf_mut().b_ffname.is_null() {
            emsg(gettext(E_NONAME));
            return false;
        }
        true
    }
}

/// Flush the contents of a buffer, unless it has no file name (`buf_write_all`).
pub fn buf_write_all(buf: &mut FileBuf, forceit: bool) -> i32 {
    let old_curbuf = g_st().curbuf;
    let retval = buf_write(
        &raw mut *buf,
        buf.b_ffname,
        buf.b_fname,
        1,
        buf.b_ml.ml_line_count,
        None,
        false,
        forceit,
        true,
        false,
    );
    if g_st().curbuf != old_curbuf {
        msg_source(HLF_W);
        msg(
            gettext(c"Warning: Entered other buffer unexpectedly (check autocommands)"),
            0,
        );
    }
    retval
}

/// `:argdo`, `:windo`, `:bufdo`, `:tabdo`, `:cdo`, `:ldo`, `:cfdo` and `:lfdo` (`ex_listdo`).
pub fn ex_listdo(eap: &mut Exarg) {
    unsafe {
        if curwin_mut().w_onebuf_opt.wo_wfb != 0
            && eap.cmdidx != CMD_WINDO
            && eap.cmdidx != CMD_TABDO
        {
            if (eap.cmdidx == CMD_LDO || eap.cmdidx == CMD_LFDO) && eap.forceit == 0 {
                emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                return;
            }

            if win_valid(g_st().prevwin) && crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb == 0 {
                win_goto(g_st().prevwin);
            }
            if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                win_split(0, 0);
                if curwin_mut().w_onebuf_opt.wo_wfb != 0 {
                    emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                    return;
                }
            }
        }

        let mut save_ei: Option<std::ffi::CString> = None;

        g_st().msg_listdo_overwrite += 1;

        if eap.cmdidx != CMD_WINDO && eap.cmdidx != CMD_TABDO {
            save_ei = Some(au_event_disable(c",Syntax"));

            for buf in for_all_buffers_mut() {
                buf.b_flags &= !BF_SYN_SET;
            }
        }

        if eap.cmdidx == CMD_WINDO
            || eap.cmdidx == CMD_TABDO
            || buf_hide(curbuf_mut())
            || !check_changed(
                curbuf_mut(),
                CCGD_AW | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
            )
        {
            let mut next_fnum: i32 = 0;
            let mut i: i32 = 0;
            let mut wp = g_st().firstwin;
            let mut tp = g_st().first_tabpage;
            match eap.cmdidx {
                CMD_WINDO => {
                    while !wp.is_null() && i + 1 < eap.line1 {
                        i += 1;
                        wp = (*wp).w_next;
                    }
                }
                CMD_TABDO => {
                    while !tp.is_null() && i + 1 < eap.line1 {
                        i += 1;
                        tp = (*tp).tp_next;
                    }
                }
                CMD_ARGDO => {
                    i = eap.line1 - 1;
                }
                _ => {}
            }

            let mut buf = g_st().curbuf;
            let mut qf_size: usize = 0;

            if eap.cmdidx == CMD_BUFDO {
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
            } else if eap.cmdidx == CMD_CDO
                || eap.cmdidx == CMD_LDO
                || eap.cmdidx == CMD_CFDO
                || eap.cmdidx == CMD_LFDO
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
                        eap.line2 = qf_size as Linenr;
                    }
                }
            } else {
                setpcmark();
            }
            g_st().listcmd_busy = true;

            while !g_st().got_int && !buf.is_null() {
                let mut execute = true;
                if eap.cmdidx == CMD_ARGDO {
                    if i == curwin_argcount() {
                        break;
                    }
                    if curwin_mut().w_arg_idx != i || !editing_arg_idx(curwin_mut()) {
                        do_argfile(eap, i);
                    }
                    if curwin_mut().w_arg_idx != i {
                        break;
                    }
                } else if eap.cmdidx == CMD_WINDO {
                    if !win_valid(wp) {
                        break;
                    }
                    debug_assert!(!wp.is_null());
                    execute =
                        !(*wp).w_floating || (!(*wp).w_config.hide && (*wp).w_config.focusable);
                    if execute {
                        win_goto(wp);
                        if curwin_ptr() != wp {
                            break;
                        }
                    }
                    wp = (*wp).w_next;
                } else if eap.cmdidx == CMD_TABDO {
                    if !valid_tabpage(tp) {
                        break;
                    }
                    debug_assert!(!tp.is_null());
                    goto_tabpage_tp(tp, true, true);
                    tp = (*tp).tp_next;
                } else if eap.cmdidx == CMD_BUFDO {
                    next_fnum = -1;
                    for bp in buf_iter_mut(curbuf_mut().b_next) {
                        if bp.b_p_bl != 0 {
                            next_fnum = bp.b_fnum;
                            break;
                        }
                    }
                }

                i += 1;
                if execute {
                    do_cmdline(eap.arg, eap.ea_getline, DOCMD_VERBOSE + DOCMD_NOWAIT);
                }

                if eap.cmdidx == CMD_BUFDO {
                    if next_fnum < 0 || next_fnum > eap.line2 {
                        break;
                    }

                    let mut buf_still_exists = false;
                    for bp in for_all_buffers_mut() {
                        if bp.b_fnum == next_fnum {
                            buf_still_exists = true;
                            break;
                        }
                    }
                    if !buf_still_exists {
                        break;
                    }

                    goto_buffer(eap, DOBUF_FIRST, FORWARD, next_fnum);

                    if curbuf_mut().b_fnum != next_fnum {
                        break;
                    }
                }

                if eap.cmdidx == CMD_CDO
                    || eap.cmdidx == CMD_LDO
                    || eap.cmdidx == CMD_CFDO
                    || eap.cmdidx == CMD_LFDO
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

                if eap.cmdidx == CMD_WINDO && execute {
                    validate_cursor(curwin_mut());
                    if curwin_mut().w_onebuf_opt.wo_scb != 0 {
                        do_check_scrollbind(true);
                    }
                }
                if (eap.cmdidx == CMD_WINDO || eap.cmdidx == CMD_TABDO) && i + 1 > eap.line2 {
                    break;
                }
                if eap.cmdidx == CMD_ARGDO && i >= eap.line2 {
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
                            curbuf_mut().b_p_syn,
                            curbuf_mut().b_fname,
                            true,
                            g_st().curbuf,
                        );
                    } else {
                        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
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
pub fn ex_compiler(eap: &mut Exarg) {
    {
        let mut old_cur_comp: Option<std::ffi::CString> = None;

        if crate::memory::cstr_is_empty(eap.arg) {
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.vim')");
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.lua')");
            return;
        }

        let arg_len = crate::memory::cstr_len(eap.arg);
        let bufsize = arg_len + 14;
        let mut buf = vec![0 as c_char; bufsize];

        if eap.forceit != 0 {
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet set <args>");
        } else {
            let p = get_var_value(c"g:current_compiler");
            if !p.is_null() {
                old_cur_comp = Some(crate::memory::cstr_to_cstring(p));
            }
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet setlocal <args>");
        }
        do_unlet(c"g:current_compiler", 18, true);
        do_unlet(c"b:current_compiler", 18, true);

        vim_snprintf!(buf.as_mut_ptr(), bufsize, c"compiler/%s.*", eap.arg,);
        if !source_runtime_vim_lua(buf.as_mut_ptr(), DIP_ALL) {
            semsg!(gettext(E_COMPILER_NOT_SUPPORTED_STR), eap.arg);
        }
        drop(buf);

        do_cmdline_cmd(c":delcommand CompilerSet");

        let p = get_var_value(c"g:current_compiler");
        if !p.is_null() {
            set_internal_string_var(c"b:current_compiler", p);
        }

        if eap.forceit == 0 {
            if let Some(ref old_comp) = old_cur_comp {
                set_internal_string_var(c"g:current_compiler", old_comp.as_ptr());
            } else {
                do_unlet(c"g:current_compiler", 18, true);
            }
        }
    }
}

/// `:checktime [buffer]` (`ex_checktime`).
pub fn ex_checktime(eap: &mut Exarg) {
    {
        let save_no_check_timestamps = g_st().no_check_timestamps;

        g_st().no_check_timestamps = 0;
        if eap.addr_count == 0 {
            check_timestamps(0);
        } else {
            let buf = buflist_findnr(eap.line2);
            if !buf.is_null() {
                buf_check_timestamp(unsafe { &mut *buf });
            }
        }
        g_st().no_check_timestamps = save_no_check_timestamps;
    }
}

fn script_host_execute(name: &std::ffi::CStr, eap: &mut Exarg) {
    {
        let mut len: usize = 0;
        if let Some(script) = script_get(eap, &mut len) {
            let args = tv_list_alloc(3);
            crate::eval::typval::tv_list_append_string(&mut *args, script.as_ptr(), -1);
            tv_list_append_number(&mut *args, eap.line1 as Varnumber);
            tv_list_append_number(&mut *args, eap.line2 as Varnumber);

            eval_call_provider(name.as_ptr().cast_mut(), c"execute", &mut *args, true);
        }
    }
}

fn script_host_execute_file(name: &std::ffi::CStr, eap: &mut Exarg) {
    {
        if eap.skip == 0 {
            let mut buffer = [0 as c_char; MAXPATHL];
            vim_full_name(eap.arg, buffer.as_mut_ptr(), MAXPATHL, false);

            let args = tv_list_alloc(3);
            tv_list_append_string(&mut *args, buffer.as_ptr(), -1);
            tv_list_append_number(&mut *args, eap.line1 as Varnumber);
            tv_list_append_number(&mut *args, eap.line2 as Varnumber);
            eval_call_provider(name.as_ptr().cast_mut(), c"execute_file", &mut *args, true);
        }
    }
}

fn script_host_do_range(name: &std::ffi::CStr, eap: &mut Exarg) {
    {
        if eap.skip == 0 {
            let args = tv_list_alloc(3);
            tv_list_append_number(&mut *args, eap.line1 as Varnumber);
            tv_list_append_number(&mut *args, eap.line2 as Varnumber);
            tv_list_append_string(&mut *args, eap.arg, -1);
            eval_call_provider(name.as_ptr().cast_mut(), c"do_range", &mut *args, true);
        }
    }
}

/// `:drop` (`ex_drop`).
pub fn ex_drop(eap: &mut Exarg) {
    {
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

        let buf = buflist_findnr(unsafe { (*curwin_arglist()).ae_fnum });

        for tp in for_all_tabpages_ptr() {
            for wp in win_iter_ptr(first_win_in_tab(&mut *tp)) {
                if wp.w_buffer == buf {
                    goto_tabpage_win(&mut *tp, wp);
                    curwin_mut().w_arg_idx = 0;
                    if !buf_is_changed_fn(curbuf_mut()) {
                        let save_ar = curbuf_mut().b_p_ar;
                        curbuf_mut().b_p_ar = 1;
                        buf_check_timestamp(curbuf_mut());
                        curbuf_mut().b_p_ar = save_ar;
                    }
                    if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
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
            }
        }

        if !buf_hide(curbuf_mut()) {
            g_st().emsg_off += 1;
            split = check_changed(curbuf_mut(), CCGD_AW | CCGD_EXCMD);
            g_st().emsg_off -= 1;
        }

        if split {
            eap.cmdidx = CMD_SFIRST;
            unsafe {
                *eap.cmd = b's' as c_char;
            }
        } else {
            eap.cmdidx = CMD_FIRST;
        }
        ex_rewind(eap);
    }
}
