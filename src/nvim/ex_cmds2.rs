//! Functions for command-line commands (`src/nvim/ex_cmds2.c`).
#![forbid(unsafe_code)]

use crate::arglist::{
    CCGD_AW, CCGD_EXCMD, CCGD_FORCEIT, CCGD_MULTWIN, ML_EMPTY, do_argfile, editing_arg_idx, ex_all,
    ex_rewind, set_arglist,
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
use crate::cmdexpand::Cmdidx;
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
use crate::strings::{format_to_buf, gettext};
use crate::ui::ui_has;
use crate::undo::buf_is_changed_fn;
use crate::window::{
    goto_tabpage_tp, goto_tabpage_win, valid_tabpage, win_goto, win_is_focusable, win_split,
    win_valid,
};
use std::ffi::CStr;

use crate::cursor::Linenr;
use crate::eval::funcs::VIM_QUESTION;
use crate::os::fs::MAXPATHL;
use crate::search::FORWARD;

const MAXLNUM: usize = 0x7fffffff;

const CCGD_ALLBUF: i32 = 8;

pub(crate) const BF_SYN_SET: i32 = 0x200;

const E_COMPILER_NOT_SUPPORTED_STR: &CStr = c"E666: Compiler not supported: %s";

#[inline]
fn curwin_argcount() -> i32 {
    crate::arglist::argcount()
}

#[inline]
fn curwin_arglist() -> crate::arglist::Aentry {
    crate::arglist::arg_entry(0)
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
    if (!opt_vars().p_aw.get() && !opt_vars().p_awa.get())
        || !opt_vars().p_write.get()
        || bt_dontwrite(Some(&*buf))
        || (!forceit && buf.b_p_ro.get())
        || buf.b_ffname.is_none()
    {
        return false;
    }
    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(&mut *buf));
    let mut r = buf_write_all(buf, forceit);

    if bufref_valid(Some(&bufref)) && buf_is_changed_fn(buf) {
        r = false;
    }
    r
}

/// Flush all buffers, except the ones that are readonly or are never written (`autowrite_all`).
pub fn autowrite_all() {
    if (!opt_vars().p_aw.get() && !opt_vars().p_awa.get()) || !opt_vars().p_write.get() {
        return;
    }

    let mut buf = g_st().firstbuf.get();
    while let Some(buf_ref) = buf.opt_mut() {
        if buf_is_changed_fn(buf_ref) && !buf_ref.b_p_ro.get() && !bt_dontwrite(Some(&*buf_ref)) {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, Some(&mut *buf_ref));

            buf_write_all(buf_ref, false);

            if !bufref_valid(Some(&bufref)) {
                buf = g_st().firstbuf.get();
                continue;
            }
        }

        buf = buf.b_next;
    }
}

/// Return true if buffer was changed and cannot be abandoned (`check_changed`).
pub fn check_changed(buf: &mut FileBuf, flags: i32) -> bool {
    let forceit = (flags & CCGD_FORCEIT) != 0;
    let mut bufref: Bufref = Bufref::new();
    set_bufref(&mut bufref, Some(&mut *buf));

    if !forceit
        && buf_is_changed_fn(buf)
        && ((flags & CCGD_MULTWIN) != 0 || buf.b_nwindows.get() <= 1)
        && ((flags & CCGD_AW) == 0 || !autowrite(buf, forceit))
    {
        if (opt_vars().p_confirm.get() || (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) != 0)
            && opt_vars().p_write.get()
        {
            let mut count: i32 = 0;

            if (flags & CCGD_ALLBUF) != 0 {
                for buf2 in for_all_buffers_mut() {
                    if buf_is_changed_fn(buf2) && buf2.b_ffname.is_some() {
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

/// Ask the user what to do when abandoning a changed buffer (`dialog_changed`).
pub fn dialog_changed(buf: &mut FileBuf, checkall: bool) {
    let mut buff = [0u8; DIALOG_MSG_SIZE];
    let mut ea: Exarg = Exarg::new();

    let fname = buf.fname_opt().unwrap_or_else(|| gettext(c"Untitled"));
    format_to_buf!(&mut buff, gettext(c"Save changes to \"%s\"?"), fname);
    let buff_cstr = crate::memory::bytes_as_cstr(&buff);
    let ret = if checkall {
        vim_dialog_yesnoallcancel(VIM_QUESTION, None, buff_cstr, 1)
    } else {
        vim_dialog_yesnocancel(VIM_QUESTION, None, buff_cstr, 1)
    };

    if ret == VIM_YES {
        let empty_bufname = buf.fname_opt().is_none();
        if empty_bufname {
            buf_set_name(buf.b_fnum.get(), c"Untitled");
        }

        let b_fname = buf.fname_opt().map(CStr::to_owned);
        let b_ffname = buf.ffname_opt().map(CStr::to_owned);
        if check_overwrite(&mut ea, buf, b_fname.as_deref(), b_ffname.as_deref(), false)
            && buf_write_all(buf, false)
        {
            return;
        }

        if empty_bufname {
            buf.clear_fnames();
        }
    } else if ret == VIM_NO {
        unchanged(buf, true, false);
    } else if ret == VIM_ALL {
        let mut buf2 = g_st().firstbuf.get();
        while let Some(buf2_ref) = buf2.opt_mut() {
            if buf_is_changed_fn(buf2_ref) && buf2_ref.b_ffname.is_some() && !buf2_ref.b_p_ro.get()
            {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, Some(&mut *buf2_ref));

                let b_fname = buf2_ref.fname_opt().map(CStr::to_owned);
                let b_ffname = buf2_ref.ffname_opt().map(CStr::to_owned);
                if b_fname.is_some()
                    && check_overwrite(
                        &mut ea,
                        buf2_ref,
                        b_fname.as_deref(),
                        b_ffname.as_deref(),
                        false,
                    )
                {
                    buf_write_all(buf2.borrow_mut(), false);
                }
                if !bufref_valid(Some(&bufref)) {
                    buf2 = g_st().firstbuf.get();
                    continue;
                }
            }

            buf2 = buf2.b_next;
        }
    } else if ret == VIM_DISCARDALL {
        for buf2 in for_all_buffers_ptr() {
            unchanged(&mut *buf2, true, false);
        }
    }
}

/// Ask the user whether to close the terminal buffer or not (`dialog_close_terminal`).
pub fn dialog_close_terminal(buf: &mut FileBuf) -> bool {
    let mut buff = [0u8; DIALOG_MSG_SIZE];
    let fname = buf.fname_opt().unwrap_or(c"?");
    format_to_buf!(&mut buff, gettext(c"Close \"%s\"?"), fname);
    vim_dialog_yesnocancel(VIM_QUESTION, None, crate::memory::bytes_as_cstr(&buff), 1) == VIM_YES
}

/// Return true if the buffer `buf` can be abandoned (`can_abandon`).
pub fn can_abandon(buf: &mut FileBuf, forceit: bool) -> bool {
    buf_hide(buf)
        || !buf_is_changed_fn(buf)
        || buf.b_nwindows.get() > 1
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
    let mut ret = false;
    let mut bufcount: usize = 0;

    for _b in for_all_buffers_mut() {
        bufcount += 1;
    }
    if bufcount == 0 {
        return false;
    }

    let mut bufnrs = Vec::with_capacity(bufcount);
    bufnrs.push(curbuf_mut().b_fnum.get());

    for wp in for_all_windows_mut() {
        if wp.w_buffer != g_st().curbuf.get() {
            add_bufnum(&mut bufnrs, wp.buf_mut().b_fnum.get());
        }
    }

    for tp in for_all_tabpages_ptr() {
        if g_st().curtab.get() != tp {
            for wp2 in win_iter_mut(first_win_in_tab(&tp)) {
                add_bufnum(&mut bufnrs, wp2.buf_mut().b_fnum.get());
            }
        }
    }

    for b in for_all_buffers_mut() {
        add_bufnum(&mut bufnrs, b.b_fnum.get());
    }

    let mut buf = crate::globals::BufPtr::NULL;
    let mut i: usize = 0;
    while i < bufnrs.len() {
        buf = crate::globals::BufPtr::from(buflist_findnr(bufnrs[i]));
        if let Some(buf_ref) = buf.opt_mut()
            && (!hidden || buf_ref.b_nwindows.get() == 0)
            && buf_is_changed_fn(buf_ref)
        {
            let mut bufref: Bufref = Bufref::new();
            set_bufref(&mut bufref, Some(&mut *buf_ref));

            if check_changed(
                buf_ref,
                (if opt_vars().p_awa.get() { CCGD_AW } else { 0 }) | CCGD_MULTWIN | CCGD_ALLBUF,
            ) && bufref_valid(Some(&bufref))
            {
                break;
            }
        }
        i += 1;
    }

    if i < bufnrs.len() {
        let buf_ref = buf.borrow();
        ret = true;
        g_st().exiting.set(false);
        if !opt_vars().p_confirm.get() && (g_st().cmdmod.cmod_flags & CMOD_CONFIRM) == 0 {
            if g_st().vgetc_busy.get() > 0 {
                g_st().msg_row.set(g_st().cmdline_row.get());
                g_st().msg_col.set(0);
                g_st().msg_didout.set(false);
            }
            let shown = if !buf_ref.terminal.get().is_null()
                && channel_job_running(buf_ref.b_p_channel.get() as u64)
            {
                semsg!(
                    gettext(c"E947: Job still running in buffer \"%s\""),
                    buf_ref.fname_cstr(),
                )
            } else {
                let sp = buf_spname(buf_ref);
                semsg!(
                    gettext(c"E162: No write since last change for buffer \"%s\""),
                    sp.unwrap_or_else(|| buf_ref.fname_cstr()),
                )
            };
            if shown && g_st().msg_didany.get() {
                let save = g_st().no_wait_return.get();
                g_st().no_wait_return.set(0);
                if !ui_has(K_UIMESSAGES) {
                    wait_return(0);
                }
                g_st().no_wait_return.set(save);
            }
        }

        let mut wiped = false;
        if buf != g_st().curbuf.get().as_ptr() {
            let mut found = false;
            let mut tp = g_st().first_tabpage.get().clone();
            while let Some(tp_ref) = tp.opt()
                && !found
            {
                for wp in win_iter_ptr(first_win_in_tab(tp_ref)) {
                    if wp.w_buffer == buf {
                        let mut bufref: Bufref = Bufref::new();
                        set_bufref(&mut bufref, buf.opt_mut());

                        goto_tabpage_win(tp.borrow(), wp);

                        if !bufref_valid(Some(&bufref)) {
                            wiped = true;
                        }
                        found = true;
                        break;
                    }
                }

                tp = tp.tp_next.get();
            }
        }

        if !wiped && buf != g_st().curbuf.get().as_ptr() {
            set_curbuf(
                buf.borrow_mut(),
                if unload { DOBUF_UNLOAD } else { DOBUF_GOTO },
                true,
            );
        }
    }

    ret
}

/// Return `FAIL` if there is no file name, `OK` if there is one (`check_fname`).
pub fn check_fname() -> bool {
    if curbuf_mut().b_ffname.is_none() {
        emsg(gettext(E_NONAME));
        return false;
    }
    true
}

/// Flush the contents of a buffer, unless it has no file name (`buf_write_all`).
pub fn buf_write_all(buf: &mut FileBuf, forceit: bool) -> bool {
    let old_curbuf = g_st().curbuf.get();
    let ffname = buf.ffname_opt().map(CStr::to_owned);
    let fname = buf.fname_opt().map(CStr::to_owned);
    let line_count = buf.b_ml.ml_line_count.get();
    let retval = buf_write(
        buf,
        ffname.as_deref(),
        fname.as_deref(),
        1,
        line_count,
        None,
        false,
        forceit,
        true,
        false,
    );
    if g_st().curbuf.get() != old_curbuf {
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
    if curwin_mut().w_onebuf_opt.wo_wfb.get()
        && eap.cmdidx != Cmdidx::CmdWindo
        && eap.cmdidx != Cmdidx::CmdTabdo
    {
        if (eap.cmdidx == Cmdidx::CmdLdo || eap.cmdidx == Cmdidx::CmdLfdo) && !eap.forceit {
            emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
            return;
        }

        if win_valid(g_st().prevwin.get().opt())
            && !crate::globals::prevwin_mut().w_onebuf_opt.wo_wfb.get()
        {
            win_goto(crate::globals::prevwin_mut());
        }
        if curwin_mut().w_onebuf_opt.wo_wfb.get() {
            win_split(0, 0);
            if curwin_mut().w_onebuf_opt.wo_wfb.get() {
                emsg(gettext(E_WINFIXBUF_CANNOT_GO_TO_BUFFER));
                return;
            }
        }
    }

    let mut save_ei: Option<std::ffi::CString> = None;

    g_st()
        .msg_listdo_overwrite
        .set(g_st().msg_listdo_overwrite.get() + (1));

    if eap.cmdidx != Cmdidx::CmdWindo && eap.cmdidx != Cmdidx::CmdTabdo {
        save_ei = Some(au_event_disable(c",Syntax"));

        for buf in for_all_buffers_mut() {
            buf.b_flags.set(buf.b_flags.get() & (!BF_SYN_SET));
        }
    }

    if eap.cmdidx == Cmdidx::CmdWindo
        || eap.cmdidx == Cmdidx::CmdTabdo
        || buf_hide(curbuf_mut())
        || !check_changed(
            curbuf_mut(),
            CCGD_AW | (if eap.forceit { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
        )
    {
        let mut next_fnum: i32 = 0;
        let mut i: i32 = 0;
        let mut wp = g_st().firstwin.get();
        let mut tp = g_st().first_tabpage.get().clone();
        match eap.cmdidx {
            Cmdidx::CmdWindo => {
                while !wp.is_null() && i + 1 < eap.line1 {
                    i += 1;

                    wp = wp.w_next;
                }
            }
            Cmdidx::CmdTabdo => {
                while !tp.is_null() && i + 1 < eap.line1 {
                    i += 1;

                    tp = tp.tp_next.get();
                }
            }
            Cmdidx::CmdArgdo => {
                i = eap.line1 - 1;
            }
            _ => {}
        }

        let mut buf = g_st().curbuf.get();
        let mut qf_size: usize = 0;

        if eap.cmdidx == Cmdidx::CmdBufdo {
            buf = g_st().firstbuf.get();
            while let Some(buf_ref) = buf.opt()
                && (buf_ref.b_fnum.get() < eap.line1 || !buf_ref.b_p_bl.get())
            {
                if buf_ref.b_fnum.get() > eap.line2 {
                    buf = crate::globals::BufPtr::NULL;
                    break;
                }

                buf = buf_ref.b_next;
            }
            if let Some(buf_ref) = buf.opt() {
                goto_buffer(eap, DOBUF_FIRST, FORWARD, buf_ref.b_fnum.get());
            }
        } else if eap.cmdidx == Cmdidx::CmdCdo
            || eap.cmdidx == Cmdidx::CmdLdo
            || eap.cmdidx == Cmdidx::CmdCfdo
            || eap.cmdidx == Cmdidx::CmdLfdo
        {
            qf_size = qf_get_valid_size(eap);
            debug_assert!(eap.line1 >= 0);
            if qf_size == 0 || (eap.line1 as usize) > qf_size {
                buf = crate::globals::BufPtr::NULL;
            } else {
                ex_cc(eap);

                buf = g_st().curbuf.get();
                i = eap.line1 - 1;
                if eap.addr_count <= 0 {
                    debug_assert!(qf_size < MAXLNUM);
                    eap.line2 = qf_size as Linenr;
                }
            }
        } else {
            setpcmark();
        }
        g_st().listcmd_busy.set(true);

        while !g_st().got_int.get() && !buf.is_null() {
            let mut execute = true;
            if eap.cmdidx == Cmdidx::CmdArgdo {
                if i == curwin_argcount() {
                    break;
                }
                if curwin_mut().w_arg_idx.get() != i || !editing_arg_idx(curwin_mut()) {
                    do_argfile(eap, i);
                }
                if curwin_mut().w_arg_idx.get() != i {
                    break;
                }
            } else if eap.cmdidx == Cmdidx::CmdWindo {
                if !win_valid(wp.opt()) {
                    break;
                }
                debug_assert!(!wp.is_null());

                let wp_ref = wp.borrow();
                execute = !wp_ref.w_floating.get() || win_is_focusable(wp_ref);

                if execute {
                    win_goto(wp.borrow_mut());
                    if curwin_ptr() != wp {
                        break;
                    }
                }

                wp = wp.w_next;
            } else if eap.cmdidx == Cmdidx::CmdTabdo {
                if !valid_tabpage(tp.opt()) {
                    break;
                }
                debug_assert!(!tp.is_null());
                goto_tabpage_tp(tp.opt(), true, true);

                tp = tp.tp_next.get();
            } else if eap.cmdidx == Cmdidx::CmdBufdo {
                next_fnum = -1;
                for bp in buf_iter_mut(curbuf_mut().b_next.as_ptr()) {
                    if bp.b_p_bl.get() {
                        next_fnum = bp.b_fnum.get();
                        break;
                    }
                }
            }

            i += 1;
            if execute {
                do_cmdline(
                    eap.arg.as_opt_cstr(),
                    eap.ea_getline.clone(),
                    DOCMD_VERBOSE + DOCMD_NOWAIT,
                );
            }

            if eap.cmdidx == Cmdidx::CmdBufdo {
                if next_fnum < 0 || next_fnum > eap.line2 {
                    break;
                }

                let mut buf_still_exists = false;
                for bp in for_all_buffers_mut() {
                    if bp.b_fnum.get() == next_fnum {
                        buf_still_exists = true;
                        break;
                    }
                }
                if !buf_still_exists {
                    break;
                }

                goto_buffer(eap, DOBUF_FIRST, FORWARD, next_fnum);

                if curbuf_mut().b_fnum.get() != next_fnum {
                    break;
                }
            }

            if eap.cmdidx == Cmdidx::CmdCdo
                || eap.cmdidx == Cmdidx::CmdLdo
                || eap.cmdidx == Cmdidx::CmdCfdo
                || eap.cmdidx == Cmdidx::CmdLfdo
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

            if eap.cmdidx == Cmdidx::CmdWindo && execute {
                validate_cursor(curwin_mut());
                if curwin_mut().w_onebuf_opt.wo_scb.get() {
                    do_check_scrollbind(true);
                }
            }
            if (eap.cmdidx == Cmdidx::CmdWindo || eap.cmdidx == Cmdidx::CmdTabdo)
                && i + 1 > eap.line2
            {
                break;
            }
            if eap.cmdidx == Cmdidx::CmdArgdo && i >= eap.line2 {
                break;
            }
        }
        g_st().listcmd_busy.set(false);
    }

    g_st()
        .msg_listdo_overwrite
        .set(g_st().msg_listdo_overwrite.get() - (1));
    if save_ei.is_some() {
        let mut aco: CtxSwitch = CtxSwitch::new();

        au_event_restore(save_ei);

        let mut buf = g_st().firstbuf.get();
        while let Some(buf_ref) = buf.opt_mut() {
            let mut bnext = buf_ref.b_next;
            if buf_ref.b_nwindows.get() > 0 && (buf_ref.b_flags.get() & BF_SYN_SET) != 0 {
                buf_ref.b_flags.set(buf_ref.b_flags.get() & (!BF_SYN_SET));

                if buf == g_st().curbuf.get().as_ptr() {
                    let b_p_syn = curbuf_mut().b_p_syn.to_owned();
                    let b_fname = curbuf_mut().fname_opt().map(CStr::to_owned);
                    apply_autocmds(
                        EVENT_SYNTAX,
                        Some(&b_p_syn),
                        b_fname.as_deref(),
                        true,
                        curbuf_opt_mut(),
                    );
                } else {
                    let b_p_syn = buf_ref.b_p_syn.to_owned();
                    let b_fname = buf_ref.fname_opt().map(CStr::to_owned);
                    ctx_switch(&mut aco, None, None, Some(buf_ref), 0);

                    apply_autocmds(
                        EVENT_SYNTAX,
                        Some(&b_p_syn),
                        b_fname.as_deref(),
                        true,
                        buf.opt_mut(),
                    );

                    ctx_restore(&mut aco);
                }

                bnext = g_st().firstbuf.get();
            }
            buf = bnext;
        }
    }
}

/// `:compiler[!] {name}` (`ex_compiler`).
pub fn ex_compiler(eap: &mut Exarg) {
    let mut old_cur_comp: Option<std::ffi::CString> = None;

    if eap.arg.is_empty() {
        do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.vim')");
        do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.lua')");
        return;
    }

    let arg_len = eap.arg.len();
    let bufsize = arg_len + 14;
    let mut buf = vec![0u8; bufsize];

    if eap.forceit {
        do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet set <args>");
    } else {
        if let Some(p) = get_var_value(c"g:current_compiler") {
            old_cur_comp = Some(p.into_owned());
        }
        do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet setlocal <args>");
    }
    do_unlet(b"g:current_compiler", true);
    do_unlet(b"b:current_compiler", true);

    format_to_buf!(&mut buf, c"compiler/%s.*", eap.arg);
    if !source_runtime_vim_lua(crate::memory::bytes_as_cstr(&buf), DIP_ALL) {
        semsg!(gettext(E_COMPILER_NOT_SUPPORTED_STR), eap.arg);
    }
    drop(buf);

    do_cmdline_cmd(c":delcommand CompilerSet");

    if let Some(p) = get_var_value(c"g:current_compiler") {
        set_internal_string_var(c"b:current_compiler", &p);
    }

    if !eap.forceit {
        if let Some(ref old_comp) = old_cur_comp {
            set_internal_string_var(c"g:current_compiler", old_comp);
        } else {
            do_unlet(b"g:current_compiler", true);
        }
    }
}

/// `:checktime [buffer]` (`ex_checktime`).
pub fn ex_checktime(eap: &mut Exarg) {
    let save_no_check_timestamps = g_st().no_check_timestamps.get();

    g_st().no_check_timestamps.set(0);
    if eap.addr_count == 0 {
        check_timestamps(0);
    } else if let Some(buf) = crate::buffer::buflist_findnr_opt(eap.line2) {
        buf_check_timestamp(buf);
    }
    g_st().no_check_timestamps.set(save_no_check_timestamps);
}

fn script_host_execute(name: &std::ffi::CStr, eap: &mut Exarg) {
    let mut len: usize = 0;
    if let Some(script) = script_get(eap, &mut len) {
        let args = tv_list_alloc(3);
        crate::eval::typval::tv_list_append_string(&args, Some(script.as_c_str()), -1);
        tv_list_append_number(&args, eap.line1 as Varnumber);
        tv_list_append_number(&args, eap.line2 as Varnumber);

        eval_call_provider(name, c"execute", &args, true);
    }
}

fn script_host_execute_file(name: &std::ffi::CStr, eap: &mut Exarg) {
    if !eap.skip {
        let mut buffer = [0u8; MAXPATHL];
        vim_full_name(eap.arg.as_opt_cstr(), &mut buffer, false);

        let args = tv_list_alloc(3);
        tv_list_append_string(
            &args,
            Some(CStr::from_bytes_until_nul(&buffer).unwrap_or(c"")),
            -1,
        );
        tv_list_append_number(&args, eap.line1 as Varnumber);
        tv_list_append_number(&args, eap.line2 as Varnumber);
        eval_call_provider(name, c"execute_file", &args, true);
    }
}

fn script_host_do_range(name: &std::ffi::CStr, eap: &mut Exarg) {
    if !eap.skip {
        let args = tv_list_alloc(3);
        tv_list_append_number(&args, eap.line1 as Varnumber);
        tv_list_append_number(&args, eap.line2 as Varnumber);
        tv_list_append_string(&args, eap.arg.as_opt_cstr(), -1);
        eval_call_provider(name, c"do_range", &args, true);
    }
}

/// `:drop` (`ex_drop`).
pub fn ex_drop(eap: &mut Exarg) {
    let mut split = false;

    set_arglist(eap.arg.as_cstr());

    if curwin_argcount() == 0 {
        return;
    }

    if g_st().cmdmod.cmod_tab != 0 {
        ex_all(eap);
        g_st().cmdmod.cmod_tab = 0;
        ex_rewind(eap);
        return;
    }

    let buf = crate::globals::BufPtr::from(buflist_findnr(curwin_arglist().ae_fnum));

    for tp in for_all_tabpages_ptr() {
        for wp in win_iter_ptr(first_win_in_tab(&tp)) {
            if wp.w_buffer == buf {
                goto_tabpage_win(&tp, wp);
                curwin_mut().w_arg_idx.set(0);
                if !buf_is_changed_fn(curbuf_mut()) {
                    let save_ar = curbuf_mut().b_p_ar.get();
                    curbuf_mut().b_p_ar.set(Some(true));
                    buf_check_timestamp(curbuf_mut());
                    curbuf_mut().b_p_ar.set(save_ar);
                }
                if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
                    ex_rewind(eap);
                }

                if let Some(ref cmd) = eap.do_ecmd_cmd {
                    let did_set_swapcommand = set_swapcommand(Some(cmd.as_c_str()), 0);
                    do_cmdline(
                        Some(cmd.as_c_str()),
                        crate::cmdexpand::LineGetter::None,
                        DOCMD_VERBOSE,
                    );
                    if did_set_swapcommand {
                        set_vim_var_string(VV_SWAPCOMMAND, None, -1);
                    }
                }
                return;
            }
        }
    }

    if !buf_hide(curbuf_mut()) {
        g_st().emsg_off.set(g_st().emsg_off.get() + (1));
        split = check_changed(curbuf_mut(), CCGD_AW | CCGD_EXCMD);
        g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    }

    if split {
        eap.cmdidx = Cmdidx::CmdSfirst;
        if let Some(b) = eap.cmd.cstr_buf_mut().first_mut() {
            *b = b's';
        }
    } else {
        eap.cmdidx = Cmdidx::CmdFirst;
    }
    ex_rewind(eap);
}
