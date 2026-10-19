//! Functions for command-line commands (`src/nvim/ex_cmds2.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::command::{cmdidx_T, cmdmod_T, exarg_T};
use crate::api::options::CtxSwitch;
use crate::api::tabpage::{tabpage_T, win_T};
use crate::api::win_config::{UIExtension, bufref_T};
use crate::clipboard::{list_T, typval_T};
use crate::context::{ctx_restore, ctx_switch};
use crate::dialog::{vim_dialog_yesnoallcancel, vim_dialog_yesnocancel};
use crate::eval::deprecated::buf_T;
use crate::memory::{xfree, xmalloc, xstrdup};

pub type linenr_T = i32;
pub type varnumber_T = i64;
pub type event_T = c_int;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const FORWARD: c_int = 1;
const MAXLNUM: usize = 0x7fffffff;
const MAXPATHL: usize = 4096;
const DIALOG_MSG_SIZE: usize = 1000;

const CCGD_AW: c_int = 1;
const CCGD_MULTWIN: c_int = 2;
const CCGD_FORCEIT: c_int = 4;
const CCGD_ALLBUF: c_int = 8;
const CCGD_EXCMD: c_int = 16;

const CMOD_CONFIRM: c_int = 0x0080;

const VIM_QUESTION: c_int = 4;
const VIM_YES: c_int = 2;
const VIM_NO: c_int = 3;
const VIM_ALL: c_int = 5;
const VIM_DISCARDALL: c_int = 6;

const kUIMessages: UIExtension = 4;

const DOBUF_GOTO: c_int = 0;
const DOBUF_FIRST: c_int = 1;
const DOBUF_UNLOAD: c_int = 2;

const HLF_W: c_int = 26;

const CMD_argdo: cmdidx_T = 10;
const CMD_bufdo: cmdidx_T = 42;
const CMD_cdo: cmdidx_T = 64;
const CMD_cfdo: cmdidx_T = 68;
const CMD_first: cmdidx_T = 164;
const CMD_ldo: cmdidx_T = 231;
const CMD_lfdo: cmdidx_T = 237;
const CMD_sfirst: cmdidx_T = 410;
const CMD_tabdo: cmdidx_T = 461;
const CMD_windo: cmdidx_T = 535;

const BF_SYN_SET: c_int = 0x200;
const DOCMD_VERBOSE: c_int = 0x01;
const DOCMD_NOWAIT: c_int = 0x02;
const EVENT_SYNTAX: event_T = 112;
const DIP_ALL: c_int = 0x01;
const ML_EMPTY: c_int = 0x01;
const VV_SWAPCOMMAND: c_int = 49;

const E_COMPILER_NOT_SUPPORTED_STR: &CStr = c"E666: Compiler not supported: %s";

use crate::arglist::{aentry_T, alist_T};

unsafe extern "C" {
    static mut p_aw: c_int;
    static mut p_awa: c_int;
    static mut p_write: c_int;
    static mut p_confirm: c_int;
    static mut firstbuf: *mut buf_T;
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut firstwin: *mut win_T;
    static mut prevwin: *mut win_T;
    static mut curtab: *mut tabpage_T;
    static mut first_tabpage: *mut tabpage_T;
    static mut cmdmod: cmdmod_T;
    static mut exiting: bool;
    static mut vgetc_busy: c_int;
    static mut msg_row: c_int;
    static mut cmdline_row: c_int;
    static mut msg_col: c_int;
    static mut msg_didout: bool;
    static mut msg_didany: bool;
    static mut no_wait_return: c_int;
    static mut msg_listdo_overwrite: c_int;
    static mut listcmd_busy: bool;
    static mut got_int: bool;
    static mut no_check_timestamps: c_int;
    static mut emsg_off: c_int;
    static e_noname: [c_char; 0];
    static e_winfixbuf_cannot_go_to_buffer: [c_char; 0];

    fn bt_dontwrite(buf: *const buf_T) -> bool;
    fn set_bufref(bufref: *mut bufref_T, buf: *mut buf_T);
    fn bufref_valid(bufref: *mut bufref_T) -> bool;
    fn bufIsChanged(buf: *mut buf_T) -> bool;
    fn no_write_message();
    fn no_write_message_nobang(buf: *const buf_T);
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn vim_snprintf(str_: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn buf_set_name(fnum: c_int, name: *const c_char);
    fn check_overwrite(
        eap: *mut exarg_T,
        buf: *mut buf_T,
        fname: *mut c_char,
        ffname: *mut c_char,
        other: bool,
    ) -> c_int;
    fn unchanged(buf: *mut buf_T, ff: bool, always_inc_changedtick: bool);
    fn buf_hide(buf: *const buf_T) -> bool;
    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn channel_job_running(id: u64) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn emsg(s: *const c_char) -> bool;
    fn buf_spname(buf: *mut buf_T) -> *mut c_char;
    fn ui_has(ext: UIExtension) -> bool;
    fn wait_return(redraw: c_int);
    fn goto_tabpage_win(tp: *mut tabpage_T, wp: *mut win_T);
    fn set_curbuf(buf: *mut buf_T, action: c_int, update_jumplist: bool);
    fn buf_write(
        buf: *mut buf_T,
        fname: *mut c_char,
        sfname: *mut c_char,
        start: linenr_T,
        end: linenr_T,
        eap: *mut exarg_T,
        append: bool,
        forceit: bool,
        reset_changed: bool,
        filtering: bool,
    ) -> c_int;
    fn msg_source(hl_id: c_int);
    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn win_valid(win: *const win_T) -> bool;
    fn win_goto(wp: *mut win_T);
    fn win_split(size: c_int, flags: c_int) -> c_int;
    fn au_event_disable(what: *const c_char) -> *mut c_char;
    fn au_event_restore(old_ei: *mut c_char);
    fn goto_buffer(eap: *mut exarg_T, start: c_int, dir: c_int, count: c_int);
    fn qf_get_valid_size(eap: *mut exarg_T) -> usize;
    fn qf_get_cur_idx(eap: *mut exarg_T) -> usize;
    fn ex_cc(eap: *mut exarg_T);
    fn ex_cnext(eap: *mut exarg_T);
    fn setpcmark();
    fn editing_arg_idx(win: *mut win_T) -> bool;
    fn do_argfile(eap: *mut exarg_T, argn: c_int);
    fn valid_tabpage(tpc: *mut tabpage_T) -> bool;
    fn goto_tabpage_tp(
        tp: *mut tabpage_T,
        trigger_enter_autocmds: bool,
        trigger_leave_autocmds: bool,
    );
    fn do_cmdline(
        cmdline: *mut c_char,
        fgetline: *mut c_void,
        cookie: *mut c_void,
        flags: c_int,
    ) -> c_int;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn validate_cursor(wp: *mut win_T);
    fn do_check_scrollbind(check: bool);
    fn apply_autocmds(
        event: event_T,
        fname: *mut c_char,
        fname_io: *mut c_char,
        force: bool,
        buf: *mut buf_T,
    ) -> bool;
    fn get_var_value(name: *const c_char) -> *mut c_char;
    fn do_unlet(name: *const c_char, name_len: usize, forceit: bool) -> c_int;
    fn source_runtime_vim_lua(name: *mut c_char, flags: c_int) -> c_int;
    fn set_internal_string_var(name: *const c_char, value: *mut c_char);
    fn check_timestamps(focus: c_int) -> c_int;
    fn buf_check_timestamp(buf: *mut buf_T) -> c_int;
    fn script_get(eap: *mut exarg_T, lenp: *mut usize) -> *mut c_char;
    fn tv_list_alloc(len: isize) -> *mut list_T;
    fn tv_list_append_allocated_string(l: *mut list_T, str_: *mut c_char);
    fn tv_list_append_string(l: *mut list_T, str_: *const c_char, len: isize);
    fn tv_list_append_number(l: *mut list_T, n: varnumber_T);
    fn eval_call_provider(
        provider: *const c_char,
        method: *const c_char,
        arguments: *mut list_T,
        discard: bool,
    ) -> typval_T;
    fn vim_FullName(fname: *const c_char, buf: *mut c_char, len: usize, force: bool) -> c_int;
    fn set_arglist(str_: *mut c_char);
    fn ex_all(eap: *mut exarg_T);
    fn ex_rewind(eap: *mut exarg_T);
    fn set_swapcommand(command: *mut c_char, newlnum: linenr_T) -> bool;
    fn set_vim_var_string(idx: c_int, val: *const c_char, len: isize);
}

#[inline]
unsafe fn first_win_in_tab(tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        if tp == curtab {
            firstwin
        } else {
            (*tp).tp_firstwin
        }
    }
}

#[inline]
unsafe fn curwin_argcount() -> c_int {
    unsafe { (*((*curwin).w_alist.cast::<alist_T>())).al_ga.ga_len }
}

#[inline]
unsafe fn curwin_arglist() -> *mut aentry_T {
    unsafe {
        (*((*curwin).w_alist.cast::<alist_T>()))
            .al_ga
            .ga_data
            .cast::<aentry_T>()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_ruby(eap: *mut exarg_T) {
    unsafe {
        script_host_execute(c"ruby".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_rubyfile(eap: *mut exarg_T) {
    unsafe {
        script_host_execute_file(c"ruby".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_rubydo(eap: *mut exarg_T) {
    unsafe {
        script_host_do_range(c"ruby".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_python3(eap: *mut exarg_T) {
    unsafe {
        script_host_execute(c"python3".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_py3file(eap: *mut exarg_T) {
    unsafe {
        script_host_execute_file(c"python3".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_pydo3(eap: *mut exarg_T) {
    unsafe {
        script_host_do_range(c"python3".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_perl(eap: *mut exarg_T) {
    unsafe {
        script_host_execute(c"perl".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_perlfile(eap: *mut exarg_T) {
    unsafe {
        script_host_execute_file(c"perl".as_ptr(), eap);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_perldo(eap: *mut exarg_T) {
    unsafe {
        script_host_do_range(c"perl".as_ptr(), eap);
    }
}

/// If `'autowrite'` option set, try to write the file (`autowrite`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn autowrite(buf: *mut buf_T, forceit: bool) -> c_int {
    unsafe {
        if (p_aw == 0 && p_awa == 0)
            || p_write == 0
            || bt_dontwrite(buf)
            || (!forceit && (*buf).b_p_ro != 0)
            || (*buf).b_ffname.is_null()
        {
            return FAIL;
        }
        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);
        let mut r = buf_write_all(buf, forceit);

        if bufref_valid(&raw mut bufref) && bufIsChanged(buf) {
            r = FAIL;
        }
        r
    }
}

/// Flush all buffers, except the ones that are readonly or are never written (`autowrite_all`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn autowrite_all() {
    unsafe {
        if (p_aw == 0 && p_awa == 0) || p_write == 0 {
            return;
        }

        let mut buf = firstbuf;
        while !buf.is_null() {
            if bufIsChanged(buf) && (*buf).b_p_ro == 0 && !bt_dontwrite(buf) {
                let mut bufref: bufref_T = std::mem::zeroed();
                set_bufref(&raw mut bufref, buf);
                buf_write_all(buf, false);
                if !bufref_valid(&raw mut bufref) {
                    buf = firstbuf;
                    continue;
                }
            }
            buf = (*buf).b_next;
        }
    }
}

/// Return true if buffer was changed and cannot be abandoned (`check_changed`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_changed(buf: *mut buf_T, flags: c_int) -> bool {
    unsafe {
        let forceit = (flags & CCGD_FORCEIT) != 0;
        let mut bufref: bufref_T = std::mem::zeroed();
        set_bufref(&raw mut bufref, buf);

        if !forceit
            && bufIsChanged(buf)
            && ((flags & CCGD_MULTWIN) != 0 || (*buf).b_nwindows <= 1)
            && ((flags & CCGD_AW) == 0 || autowrite(buf, forceit) == FAIL)
        {
            if (p_confirm != 0 || (cmdmod.cmod_flags & CMOD_CONFIRM) != 0) && p_write != 0 {
                let mut count: c_int = 0;

                if (flags & CCGD_ALLBUF) != 0 {
                    let mut buf2 = firstbuf;
                    while !buf2.is_null() {
                        if bufIsChanged(buf2) && !(*buf2).b_ffname.is_null() {
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
                no_write_message_nobang(curbuf);
            }
            return true;
        }
        false
    }
}

/// Ask the user what to do when abandoning a changed buffer (`dialog_changed`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dialog_changed(buf: *mut buf_T, checkall: bool) {
    unsafe {
        let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
        let mut ea: exarg_T = std::mem::zeroed();

        let fname = if !(*buf).b_fname.is_null() {
            (*buf).b_fname.cast_const()
        } else {
            gettext(c"Untitled".as_ptr())
        };
        vim_snprintf(
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
            let empty_bufname = (*buf).b_fname.is_null();
            if empty_bufname {
                buf_set_name((*buf).b_fnum, c"Untitled".as_ptr());
            }

            if check_overwrite(&raw mut ea, buf, (*buf).b_fname, (*buf).b_ffname, false) == OK
                && buf_write_all(buf, false) == OK
            {
                return;
            }

            if empty_bufname {
                (*buf).b_fname = ptr::null_mut();
                xfree((*buf).b_ffname.cast());
                (*buf).b_ffname = ptr::null_mut();
                xfree((*buf).b_sfname.cast());
                (*buf).b_sfname = ptr::null_mut();
            }
        } else if ret == VIM_NO {
            unchanged(buf, true, false);
        } else if ret == VIM_ALL {
            let mut buf2 = firstbuf;
            while !buf2.is_null() {
                if bufIsChanged(buf2) && !(*buf2).b_ffname.is_null() && (*buf2).b_p_ro == 0 {
                    let mut bufref: bufref_T = std::mem::zeroed();
                    set_bufref(&raw mut bufref, buf2);

                    if !(*buf2).b_fname.is_null()
                        && check_overwrite(
                            &raw mut ea,
                            buf2,
                            (*buf2).b_fname,
                            (*buf2).b_ffname,
                            false,
                        ) == OK
                    {
                        buf_write_all(buf2, false);
                    }
                    if !bufref_valid(&raw mut bufref) {
                        buf2 = firstbuf;
                        continue;
                    }
                }
                buf2 = (*buf2).b_next;
            }
        } else if ret == VIM_DISCARDALL {
            let mut buf2 = firstbuf;
            while !buf2.is_null() {
                unchanged(buf2, true, false);
                buf2 = (*buf2).b_next;
            }
        }
    }
}

/// Ask the user whether to close the terminal buffer or not (`dialog_close_terminal`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dialog_close_terminal(buf: *mut buf_T) -> bool {
    unsafe {
        let mut buff = [0 as c_char; DIALOG_MSG_SIZE];
        let fname = if !(*buf).b_fname.is_null() {
            (*buf).b_fname.cast_const()
        } else {
            c"?".as_ptr()
        };
        vim_snprintf(
            buff.as_mut_ptr(),
            DIALOG_MSG_SIZE,
            gettext(c"Close \"%s\"?".as_ptr()),
            fname,
        );
        vim_dialog_yesnocancel(VIM_QUESTION, ptr::null_mut(), buff.as_mut_ptr(), 1) == VIM_YES
    }
}

/// Return true if the buffer `buf` can be abandoned (`can_abandon`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn can_abandon(buf: *mut buf_T, forceit: bool) -> bool {
    unsafe {
        buf_hide(buf)
            || !bufIsChanged(buf)
            || (*buf).b_nwindows > 1
            || autowrite(buf, forceit) == OK
            || forceit
    }
}

unsafe fn add_bufnum(bufnrs: *mut c_int, bufnump: *mut c_int, nr: c_int) {
    unsafe {
        for i in 0..(*bufnump as usize) {
            if *bufnrs.add(i) == nr {
                return;
            }
        }
        *bufnrs.add(*bufnump as usize) = nr;
        *bufnump += 1;
    }
}

/// Check if any buffer was changed and cannot be abandoned (`check_changed_any`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_changed_any(hidden: bool, unload: bool) -> bool {
    unsafe {
        let mut ret = false;
        let mut bufcount: usize = 0;

        let mut b = firstbuf;
        while !b.is_null() {
            bufcount += 1;
            b = (*b).b_next;
        }
        if bufcount == 0 {
            return false;
        }

        let bufnrs: *mut c_int = xmalloc(size_of::<c_int>() * bufcount).cast();

        *bufnrs = (*curbuf).b_fnum;
        let mut bufnum: c_int = 1;

        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_buffer != curbuf {
                add_bufnum(bufnrs, &raw mut bufnum, (*(*wp).w_buffer).b_fnum);
            }
            wp = (*wp).w_next;
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            if tp != curtab {
                let mut wp2 = first_win_in_tab(tp);
                while !wp2.is_null() {
                    add_bufnum(bufnrs, &raw mut bufnum, (*(*wp2).w_buffer).b_fnum);
                    wp2 = (*wp2).w_next;
                }
            }
            tp = (*tp).tp_next;
        }

        let mut b = firstbuf;
        while !b.is_null() {
            add_bufnum(bufnrs, &raw mut bufnum, (*b).b_fnum);
            b = (*b).b_next;
        }

        let mut buf: *mut buf_T = ptr::null_mut();
        let mut i: c_int = 0;
        while i < bufnum {
            buf = buflist_findnr(*bufnrs.add(i as usize));
            if !buf.is_null() && (!hidden || (*buf).b_nwindows == 0) && bufIsChanged(buf) {
                let mut bufref: bufref_T = std::mem::zeroed();
                set_bufref(&raw mut bufref, buf);

                if check_changed(
                    buf,
                    (if p_awa != 0 { CCGD_AW } else { 0 }) | CCGD_MULTWIN | CCGD_ALLBUF,
                ) && bufref_valid(&raw mut bufref)
                {
                    break;
                }
            }
            i += 1;
        }

        if i < bufnum {
            ret = true;
            exiting = false;
            if p_confirm == 0 && (cmdmod.cmod_flags & CMOD_CONFIRM) == 0 {
                if vgetc_busy > 0 {
                    msg_row = cmdline_row;
                    msg_col = 0;
                    msg_didout = false;
                }
                let shown = if !(*buf).terminal.is_null()
                    && channel_job_running((*buf).b_p_channel as u64)
                {
                    semsg(
                        gettext(c"E947: Job still running in buffer \"%s\"".as_ptr()),
                        (*buf).b_fname,
                    )
                } else {
                    let sp = buf_spname(buf);
                    semsg(
                        gettext(c"E162: No write since last change for buffer \"%s\"".as_ptr()),
                        if !sp.is_null() { sp } else { (*buf).b_fname },
                    )
                };
                if shown && msg_didany {
                    let save = no_wait_return;
                    no_wait_return = 0;
                    if !ui_has(kUIMessages) {
                        wait_return(0);
                    }
                    no_wait_return = save;
                }
            }

            let mut wiped = false;
            if buf != curbuf {
                let mut found = false;
                let mut tp = first_tabpage;
                while !tp.is_null() && !found {
                    let mut wp = first_win_in_tab(tp);
                    while !wp.is_null() {
                        if (*wp).w_buffer == buf {
                            let mut bufref: bufref_T = std::mem::zeroed();
                            set_bufref(&raw mut bufref, buf);
                            goto_tabpage_win(tp, wp);
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

            if !wiped && buf != curbuf {
                set_curbuf(buf, if unload { DOBUF_UNLOAD } else { DOBUF_GOTO }, true);
            }
        }

        xfree(bufnrs.cast());
        ret
    }
}

/// Return `FAIL` if there is no file name, `OK` if there is one (`check_fname`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_fname() -> c_int {
    unsafe {
        if (*curbuf).b_ffname.is_null() {
            emsg(gettext((&raw const e_noname).cast()));
            return FAIL;
        }
        OK
    }
}

/// Flush the contents of a buffer, unless it has no file name (`buf_write_all`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buf_write_all(buf: *mut buf_T, forceit: bool) -> c_int {
    unsafe {
        let old_curbuf = curbuf;
        let retval = buf_write(
            buf,
            (*buf).b_ffname,
            (*buf).b_fname,
            1,
            (*buf).b_ml.ml_line_count,
            ptr::null_mut(),
            false,
            forceit,
            true,
            false,
        );
        if curbuf != old_curbuf {
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_listdo(eap: *mut exarg_T) {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_wfb != 0
            && (*eap).cmdidx != CMD_windo
            && (*eap).cmdidx != CMD_tabdo
        {
            if ((*eap).cmdidx == CMD_ldo || (*eap).cmdidx == CMD_lfdo) && (*eap).forceit == 0 {
                emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                return;
            }

            if win_valid(prevwin) && (*prevwin).w_onebuf_opt.wo_wfb == 0 {
                win_goto(prevwin);
            }
            if (*curwin).w_onebuf_opt.wo_wfb != 0 {
                win_split(0, 0);
                if (*curwin).w_onebuf_opt.wo_wfb != 0 {
                    emsg(gettext((&raw const e_winfixbuf_cannot_go_to_buffer).cast()));
                    return;
                }
            }
        }

        let mut save_ei: *mut c_char = ptr::null_mut();

        msg_listdo_overwrite += 1;

        if (*eap).cmdidx != CMD_windo && (*eap).cmdidx != CMD_tabdo {
            save_ei = au_event_disable(c",Syntax".as_ptr());

            let mut buf = firstbuf;
            while !buf.is_null() {
                (*buf).b_flags &= !BF_SYN_SET;
                buf = (*buf).b_next;
            }
        }

        if (*eap).cmdidx == CMD_windo
            || (*eap).cmdidx == CMD_tabdo
            || buf_hide(curbuf)
            || !check_changed(
                curbuf,
                CCGD_AW | (if (*eap).forceit != 0 { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
            )
        {
            let mut next_fnum: c_int = 0;
            let mut i: c_int = 0;
            let mut wp = firstwin;
            let mut tp = first_tabpage;
            match (*eap).cmdidx {
                CMD_windo => {
                    while !wp.is_null() && i + 1 < (*eap).line1 {
                        i += 1;
                        wp = (*wp).w_next;
                    }
                }
                CMD_tabdo => {
                    while !tp.is_null() && i + 1 < (*eap).line1 {
                        i += 1;
                        tp = (*tp).tp_next;
                    }
                }
                CMD_argdo => {
                    i = (*eap).line1 - 1;
                }
                _ => {}
            }

            let mut buf = curbuf;
            let mut qf_size: usize = 0;

            if (*eap).cmdidx == CMD_bufdo {
                buf = firstbuf;
                while !buf.is_null() && ((*buf).b_fnum < (*eap).line1 || (*buf).b_p_bl == 0) {
                    if (*buf).b_fnum > (*eap).line2 {
                        buf = ptr::null_mut();
                        break;
                    }
                    buf = (*buf).b_next;
                }
                if !buf.is_null() {
                    goto_buffer(eap, DOBUF_FIRST, FORWARD, (*buf).b_fnum);
                }
            } else if (*eap).cmdidx == CMD_cdo
                || (*eap).cmdidx == CMD_ldo
                || (*eap).cmdidx == CMD_cfdo
                || (*eap).cmdidx == CMD_lfdo
            {
                qf_size = qf_get_valid_size(eap);
                debug_assert!((*eap).line1 >= 0);
                if qf_size == 0 || ((*eap).line1 as usize) > qf_size {
                    buf = ptr::null_mut();
                } else {
                    ex_cc(eap);

                    buf = curbuf;
                    i = (*eap).line1 - 1;
                    if (*eap).addr_count <= 0 {
                        debug_assert!(qf_size < MAXLNUM);
                        (*eap).line2 = qf_size as linenr_T;
                    }
                }
            } else {
                setpcmark();
            }
            listcmd_busy = true;

            while !got_int && !buf.is_null() {
                let mut execute = true;
                if (*eap).cmdidx == CMD_argdo {
                    if i == curwin_argcount() {
                        break;
                    }
                    if (*curwin).w_arg_idx != i || !editing_arg_idx(curwin) {
                        do_argfile(eap, i);
                    }
                    if (*curwin).w_arg_idx != i {
                        break;
                    }
                } else if (*eap).cmdidx == CMD_windo {
                    if !win_valid(wp) {
                        break;
                    }
                    debug_assert!(!wp.is_null());
                    execute =
                        !(*wp).w_floating || (!(*wp).w_config.hide && (*wp).w_config.focusable);
                    if execute {
                        win_goto(wp);
                        if curwin != wp {
                            break;
                        }
                    }
                    wp = (*wp).w_next;
                } else if (*eap).cmdidx == CMD_tabdo {
                    if !valid_tabpage(tp) {
                        break;
                    }
                    debug_assert!(!tp.is_null());
                    goto_tabpage_tp(tp, true, true);
                    tp = (*tp).tp_next;
                } else if (*eap).cmdidx == CMD_bufdo {
                    next_fnum = -1;
                    let mut bp = (*curbuf).b_next;
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
                    do_cmdline(
                        (*eap).arg,
                        (*eap).ea_getline,
                        (*eap).cookie,
                        DOCMD_VERBOSE + DOCMD_NOWAIT,
                    );
                }

                if (*eap).cmdidx == CMD_bufdo {
                    if next_fnum < 0 || next_fnum > (*eap).line2 {
                        break;
                    }

                    let mut buf_still_exists = false;
                    let mut bp = firstbuf;
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

                    if (*curbuf).b_fnum != next_fnum {
                        break;
                    }
                }

                if (*eap).cmdidx == CMD_cdo
                    || (*eap).cmdidx == CMD_ldo
                    || (*eap).cmdidx == CMD_cfdo
                    || (*eap).cmdidx == CMD_lfdo
                {
                    debug_assert!(i >= 0);
                    if (i as usize) >= qf_size || i >= (*eap).line2 {
                        break;
                    }

                    let qf_idx = qf_get_cur_idx(eap);
                    ex_cnext(eap);
                    if qf_get_cur_idx(eap) == qf_idx {
                        break;
                    }
                }

                if (*eap).cmdidx == CMD_windo && execute {
                    validate_cursor(curwin);
                    if (*curwin).w_onebuf_opt.wo_scb != 0 {
                        do_check_scrollbind(true);
                    }
                }
                if ((*eap).cmdidx == CMD_windo || (*eap).cmdidx == CMD_tabdo)
                    && i + 1 > (*eap).line2
                {
                    break;
                }
                if (*eap).cmdidx == CMD_argdo && i >= (*eap).line2 {
                    break;
                }
            }
            listcmd_busy = false;
        }

        msg_listdo_overwrite -= 1;
        if !save_ei.is_null() {
            let mut aco: CtxSwitch = std::mem::zeroed();

            au_event_restore(save_ei);

            let mut buf = firstbuf;
            while !buf.is_null() {
                let mut bnext = (*buf).b_next;
                if (*buf).b_nwindows > 0 && ((*buf).b_flags & BF_SYN_SET) != 0 {
                    (*buf).b_flags &= !BF_SYN_SET;

                    if buf == curbuf {
                        apply_autocmds(
                            EVENT_SYNTAX,
                            (*curbuf).b_p_syn,
                            (*curbuf).b_fname,
                            true,
                            curbuf,
                        );
                    } else {
                        ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
                        apply_autocmds(EVENT_SYNTAX, (*buf).b_p_syn, (*buf).b_fname, true, buf);
                        ctx_restore(&raw mut aco);
                    }

                    bnext = firstbuf;
                }
                buf = bnext;
            }
        }
    }
}

/// `:compiler[!] {name}` (`ex_compiler`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_compiler(eap: *mut exarg_T) {
    unsafe {
        let mut old_cur_comp: *mut c_char = ptr::null_mut();

        if *(*eap).arg == NUL {
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.vim')".as_ptr());
            do_cmdline_cmd(c"echo globpath(&rtp, 'compiler/*.lua')".as_ptr());
            return;
        }

        let arg_len = CStr::from_ptr((*eap).arg).to_bytes().len();
        let bufsize = arg_len + 14;
        let buf: *mut c_char = xmalloc(bufsize).cast();

        if (*eap).forceit != 0 {
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet set <args>".as_ptr());
        } else {
            old_cur_comp = get_var_value(c"g:current_compiler".as_ptr());
            if !old_cur_comp.is_null() {
                old_cur_comp = xstrdup(old_cur_comp);
            }
            do_cmdline_cmd(c"command -nargs=* -keepscript CompilerSet setlocal <args>".as_ptr());
        }
        do_unlet(c"g:current_compiler".as_ptr(), 18, true);
        do_unlet(c"b:current_compiler".as_ptr(), 18, true);

        vim_snprintf(buf, bufsize, c"compiler/%s.*".as_ptr(), (*eap).arg);
        if source_runtime_vim_lua(buf, DIP_ALL) == FAIL {
            semsg(gettext(E_COMPILER_NOT_SUPPORTED_STR.as_ptr()), (*eap).arg);
        }
        xfree(buf.cast());

        do_cmdline_cmd(c":delcommand CompilerSet".as_ptr());

        let p = get_var_value(c"g:current_compiler".as_ptr());
        if !p.is_null() {
            set_internal_string_var(c"b:current_compiler".as_ptr(), p);
        }

        if (*eap).forceit == 0 {
            if !old_cur_comp.is_null() {
                set_internal_string_var(c"g:current_compiler".as_ptr(), old_cur_comp);
                xfree(old_cur_comp.cast());
            } else {
                do_unlet(c"g:current_compiler".as_ptr(), 18, true);
            }
        }
    }
}

/// `:checktime [buffer]` (`ex_checktime`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_checktime(eap: *mut exarg_T) {
    unsafe {
        let save_no_check_timestamps = no_check_timestamps;

        no_check_timestamps = 0;
        if (*eap).addr_count == 0 {
            check_timestamps(0);
        } else {
            let buf = buflist_findnr((*eap).line2 as c_int);
            if !buf.is_null() {
                buf_check_timestamp(buf);
            }
        }
        no_check_timestamps = save_no_check_timestamps;
    }
}

unsafe fn script_host_execute(name: *const c_char, eap: *mut exarg_T) {
    unsafe {
        let mut len: usize = 0;
        let script = script_get(eap, &raw mut len);

        if !script.is_null() {
            let args = tv_list_alloc(3);
            tv_list_append_allocated_string(args, script);
            tv_list_append_number(args, (*eap).line1 as varnumber_T);
            tv_list_append_number(args, (*eap).line2 as varnumber_T);

            eval_call_provider(name, c"execute".as_ptr(), args, true);
        }
    }
}

unsafe fn script_host_execute_file(name: *const c_char, eap: *mut exarg_T) {
    unsafe {
        if (*eap).skip == 0 {
            let mut buffer = [0 as c_char; MAXPATHL];
            vim_FullName((*eap).arg, buffer.as_mut_ptr(), MAXPATHL, false);

            let args = tv_list_alloc(3);
            tv_list_append_string(args, buffer.as_ptr(), -1);
            tv_list_append_number(args, (*eap).line1 as varnumber_T);
            tv_list_append_number(args, (*eap).line2 as varnumber_T);
            eval_call_provider(name, c"execute_file".as_ptr(), args, true);
        }
    }
}

unsafe fn script_host_do_range(name: *const c_char, eap: *mut exarg_T) {
    unsafe {
        if (*eap).skip == 0 {
            let args = tv_list_alloc(3);
            tv_list_append_number(args, (*eap).line1 as varnumber_T);
            tv_list_append_number(args, (*eap).line2 as varnumber_T);
            tv_list_append_string(args, (*eap).arg, -1);
            eval_call_provider(name, c"do_range".as_ptr(), args, true);
        }
    }
}

/// `:drop` (`ex_drop`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_drop(eap: *mut exarg_T) {
    unsafe {
        let mut split = false;

        set_arglist((*eap).arg);

        if curwin_argcount() == 0 {
            return;
        }

        if cmdmod.cmod_tab != 0 {
            ex_all(eap);
            cmdmod.cmod_tab = 0;
            ex_rewind(eap);
            return;
        }

        let buf = buflist_findnr((*curwin_arglist()).ae_fnum);

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = first_win_in_tab(tp);
            while !wp.is_null() {
                if (*wp).w_buffer == buf {
                    goto_tabpage_win(tp, wp);
                    (*curwin).w_arg_idx = 0;
                    if !bufIsChanged(curbuf) {
                        let save_ar = (*curbuf).b_p_ar;
                        (*curbuf).b_p_ar = 1;
                        buf_check_timestamp(curbuf);
                        (*curbuf).b_p_ar = save_ar;
                    }
                    if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
                        ex_rewind(eap);
                    }

                    if !(*eap).do_ecmd_cmd.is_null() {
                        let did_set_swapcommand = set_swapcommand((*eap).do_ecmd_cmd, 0);
                        do_cmdline(
                            (*eap).do_ecmd_cmd,
                            ptr::null_mut(),
                            ptr::null_mut(),
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

        if !buf_hide(curbuf) {
            emsg_off += 1;
            split = check_changed(curbuf, CCGD_AW | CCGD_EXCMD);
            emsg_off -= 1;
        }

        if split {
            (*eap).cmdidx = CMD_sfirst;
            *(*eap).cmd = b's' as c_char;
        } else {
            (*eap).cmdidx = CMD_first;
        }
        ex_rewind(eap);
    }
}
