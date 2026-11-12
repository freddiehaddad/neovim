//! Ex command-line evaluation, conditionals, loops, and exception handling (`src/nvim/ex_eval.c`).
#![forbid(unsafe_code)]

use crate::cmdexpand::Cmdidx;
use crate::cursor::Linenr;
use crate::debugger::dbg_check_skipped;
use crate::errors::*;
use crate::eval::typval::{TypVal, tv_clear, tv_list_ref, tv_list_unref};
use crate::eval::userfunc::{do_return, get_return_cmd};
use crate::eval::vars::{
    Evalarg, VV_EXCEPTION, VV_STACKTRACE, VV_THROWPOINT, set_vim_var_list, set_vim_var_string,
};
use crate::eval::{
    Forinfo, eval_for_line, eval_to_bool, eval_to_string_skip, eval0, fill_evalarg_from_eap,
    next_for_item,
};
use crate::ex_docmd::{Exarg, ends_excmd, find_nextcmd, handle_did_throw, modifier_len};
use crate::globals::*;
use crate::message::{emsg, internal_error, msg_puts, semsg, smsg, verbose_enter, verbose_leave};
use crate::option_vars::*;
use crate::regexp::{RE_MAGIC, RE_STRING, Regmatch, skip_regexp_err, vim_regcomp, vim_regexec_nl};
use crate::runtime::{ESTACK_NONE, do_finish, estack_sfile, sourcing_lnum, stacktrace_create};
use crate::strings::{concat_str, format_to_buf, format_to_buf_safelen, gettext};
use std::ffi::{CStr, CString};
use std::ptr;

const CSTACK_LEN: usize = 50;

pub const CSF_TRUE: i32 = 0x0001;
pub const CSF_ACTIVE: i32 = 0x0002;
pub const CSF_ELSE: i32 = 0x0004;
pub const CSF_WHILE: i32 = 0x0008;
pub const CSF_FOR: i32 = 0x0010;
pub const CSF_TRY: i32 = 0x0100;
pub const CSF_FINALLY: i32 = 0x0200;
pub const CSF_THROWN: i32 = 0x0800;
pub const CSF_CAUGHT: i32 = 0x1000;
pub const CSF_FINISHED: i32 = 0x2000;
pub const CSF_SILENT: i32 = 0x4000;

pub const CSTP_NONE: i32 = 0;
pub const CSTP_ERROR: i32 = 1;
pub const CSTP_INTERRUPT: i32 = 2;
pub const CSTP_THROW: i32 = 4;
pub const CSTP_BREAK: i32 = 8;
pub const CSTP_CONTINUE: i32 = 16;
pub const CSTP_RETURN: i32 = 24;
pub const CSTP_FINISH: i32 = 32;

pub const CSL_HAD_LOOP: i32 = 1;
pub const CSL_HAD_ENDLOOP: i32 = 2;
pub const CSL_HAD_CONT: i32 = 4;
pub const CSL_HAD_FINA: i32 = 8;

pub const ET_USER: i32 = 0;
pub const ET_ERROR: i32 = 1;
pub const ET_INTERRUPT: i32 = 2;
pub type ExceptType = i32;

const RP_MAKE: i32 = 0;
const RP_RESUME: i32 = 1;
const RP_DISCARD: i32 = 2;

use crate::errors::{
    E_ARGREQ, E_ENDFOR, E_ENDIF, E_ENDTRY, E_ENDWHILE, E_FOR, E_INVARG2, E_INVEXPR2,
    E_STR_NOT_INSIDE_FUNCTION, E_TRAILING_ARG, E_WHILE,
};

const E_MULTIPLE_ELSE: &CStr = c"E583: Multiple :else";
const E_MULTIPLE_FINALLY: &CStr = c"E607: Multiple :finally";

#[repr(C)]
pub struct Cstack {
    pub cs_flags: [i32; CSTACK_LEN],
    pub cs_pending: [u8; CSTACK_LEN],
    pub cs_rettv: [Option<TypVal>; CSTACK_LEN],
    pub cs_exception: [Option<std::rc::Rc<Except>>; CSTACK_LEN],
    pub cs_forinfo: [Option<Box<Forinfo>>; CSTACK_LEN],
    pub cs_line: [i32; CSTACK_LEN],
    pub cs_idx: i32,
    pub cs_looplevel: i32,
    pub cs_trylevel: i32,
    pub cs_emsg_silent_list: Vec<i32>,
    pub cs_lflags: i32,
}

impl Default for Cstack {
    fn default() -> Self {
        Self {
            cs_flags: [0; CSTACK_LEN],
            cs_pending: [0; CSTACK_LEN],
            cs_rettv: [const { None }; CSTACK_LEN],
            cs_exception: [const { None }; CSTACK_LEN],
            cs_forinfo: [const { None }; CSTACK_LEN],
            cs_line: [0; CSTACK_LEN],
            cs_idx: -1,
            cs_looplevel: 0,
            cs_trylevel: 0,
            cs_emsg_silent_list: Vec::new(),
            cs_lflags: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Msglist {
    pub msg: CString,
    pub throw_idx: usize,
    pub throw_off: usize,
    pub sfile: Option<CString>,
    pub slnum: Linenr,
    pub multiline: bool,
}

#[repr(C)]
pub struct Except {
    pub r#type: ExceptType,
    pub value: CString,
    pub messages: Vec<Msglist>,
    pub throw_name: CString,
    pub throw_lnum: Linenr,
    pub stacktrace: crate::eval::typval::ListId,
}

impl Drop for Except {
    fn drop(&mut self) {
        tv_list_unref(self.stacktrace.opt());
    }
}

#[derive(Clone)]
pub enum PendingVal {
    None,
    Return(Option<TypVal>),
    Throw(Option<std::rc::Rc<Except>>),
}

#[repr(C)]
pub struct ExCleanup {
    pub pending: i32,
    pub exception: Option<std::rc::Rc<Except>>,
}

impl ExCleanup {
    #[inline]
    pub const fn new() -> Self {
        Self {
            pending: 0,
            exception: None,
        }
    }
}

impl Default for ExCleanup {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct ExceptionState {
    pub estate_current_exception: Option<std::rc::Rc<Except>>,
    pub estate_did_throw: bool,
    pub estate_need_rethrow: bool,
    pub estate_trylevel: i32,
    pub estate_did_emsg: i32,
}

impl ExceptionState {
    pub const fn new() -> Self {
        Self {
            estate_current_exception: None,
            estate_did_throw: false,
            estate_need_rethrow: false,
            estate_trylevel: 0,
            estate_did_emsg: 0,
        }
    }
}

impl Default for ExceptionState {
    fn default() -> Self {
        Self::new()
    }
}

std::thread_local! {
    static CAUSE_ABORT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
fn check_skip(cstack: &Cstack) -> bool {
    g_st().did_emsg.get() != 0
        || g_st().got_int.get()
        || g_st().did_throw.get()
        || (cstack.cs_idx > 0 && (cstack.cs_flags[(cstack.cs_idx - 1) as usize] & CSF_ACTIVE) == 0)
}

#[inline]
fn discard_pending_return(p: &mut Option<TypVal>) {
    if let Some(ref mut tv) = p.take() {
        tv_clear(tv);
    }
}

pub fn aborting() -> bool {
    (g_st().did_emsg.get() != 0 && g_st().force_abort.get())
        || g_st().got_int.get()
        || g_st().did_throw.get()
}

pub fn update_force_abort() {
    if CAUSE_ABORT.get() {
        g_st().force_abort.set(true);
    }
}

pub fn should_abort(ok: bool) -> bool {
    (!ok && g_st().trylevel.get() != 0 && g_st().emsg_silent.get() == 0) || aborting()
}

pub fn aborted_in_try() -> bool {
    g_st().force_abort.get()
}

pub fn cause_errthrow(
    mesg: &CStr,
    multiline: bool,
    concat: bool,
    severe: bool,
    ignore: &mut bool,
) -> bool {
    if g_st().suppress_errthrow.get() {
        return false;
    }

    if g_st().did_emsg.get() == 0 {
        CAUSE_ABORT.set(g_st().force_abort.get());
        g_st().force_abort.set(false);
    }

    if ((g_st().trylevel.get() == 0 && !CAUSE_ABORT.get()) || g_st().emsg_silent.get() != 0)
        && !g_st().did_throw.get()
    {
        return false;
    }

    if ptr::eq(mesg.as_ptr(), gettext(E_INTERR).as_ptr()) {
        *ignore = true;
        return true;
    }

    CAUSE_ABORT.set(true);

    if g_st().did_throw.get() {
        if g_st()
            .current_exception
            .get()
            .as_ref()
            .is_some_and(|e| e.r#type == ET_INTERRUPT)
        {
            g_st().got_int.set(false);
        }
        discard_current_exception();
    }

    if let Some(ref mut list) = *g_st().msg_list.borrow_mut() {
        if concat && let Some(last) = list.last_mut() {
            let mut bytes = last.msg.as_bytes().to_vec();
            bytes.extend_from_slice(mesg.to_bytes());
            last.msg = CString::new(bytes).unwrap();
            return true;
        }

        let is_first = list.is_empty();
        let tmsg = mesg.to_owned();
        let sfile = estack_sfile(ESTACK_NONE);
        let tmsg_bytes = tmsg.as_bytes();
        let has_vim_e_prefix = tmsg_bytes.len() >= 10
            && tmsg_bytes.starts_with(b"Vim E")
            && tmsg_bytes[5].is_ascii_digit()
            && tmsg_bytes[6].is_ascii_digit()
            && tmsg_bytes[7].is_ascii_digit()
            && tmsg_bytes[8] == b':'
            && tmsg_bytes[9] == b' ';
        list.push(Msglist {
            msg: tmsg,
            throw_idx: 0,
            throw_off: 0,
            sfile,
            slnum: sourcing_lnum(),
            multiline,
        });
        if is_first || severe {
            let idx = list.len() - 1;
            list[0].throw_idx = idx;
            list[0].throw_off = if has_vim_e_prefix { 4 } else { 0 };
        }
    }
    true
}

pub fn free_global_msglist() {
    if let Some(ref mut list) = *g_st().msg_list.borrow_mut() {
        list.clear();
    }
}

pub fn do_errthrow(cstack: Option<&mut Cstack>, cmdname: Option<&CStr>) {
    if CAUSE_ABORT.replace(false) {
        g_st().force_abort.set(true);
    }

    let Some(ref mut list) = *g_st().msg_list.borrow_mut() else {
        return;
    };
    if list.is_empty() {
        return;
    }

    let messages = std::mem::take(list);
    if throw_exception(ExceptionValue::Error(messages), cmdname) {
        if let Some(cstack) = cstack {
            do_throw(&mut *cstack);
        } else {
            g_st().need_rethrow.set(true);
        }
    }
}

pub fn do_intthrow(cstack: &mut Cstack) -> bool {
    if !g_st().got_int.get() || (g_st().trylevel.get() == 0 && !g_st().did_throw.get()) {
        return false;
    }

    if g_st().did_throw.get() {
        if g_st()
            .current_exception
            .get()
            .as_ref()
            .is_some_and(|e| e.r#type == ET_INTERRUPT)
        {
            return false;
        }
        discard_current_exception();
    }
    if throw_exception(ExceptionValue::Interrupt(c"Vim:Interrupt".to_owned()), None) {
        do_throw(cstack);
    }

    true
}

enum ExceptionValue {
    Error(Vec<Msglist>),
    Interrupt(CString),
    User(CString),
}

pub fn get_error_exception_string(mesg_bytes: &[u8], cmdname: Option<&CStr>) -> CString {
    let mut out: Vec<u8> = Vec::new();
    if let Some(cmdname) = cmdname.filter(|s| !s.to_bytes().is_empty()) {
        out.extend_from_slice(b"Vim(");
        out.extend_from_slice(cmdname.to_bytes());
        out.extend_from_slice(b"):");
    } else {
        out.extend_from_slice(b"Vim:");
    }

    let mut p_idx = 0usize;
    loop {
        let rem = &mesg_bytes[p_idx..];
        let is_err_code = rem.first() == Some(&b'E')
            && rem.get(1).is_some_and(u8::is_ascii_digit)
            && (rem.get(2) == Some(&b':')
                || (rem.get(2).is_some_and(u8::is_ascii_digit)
                    && (rem.get(3) == Some(&b':')
                        || (rem.get(3).is_some_and(u8::is_ascii_digit)
                            && rem.get(4) == Some(&b':')))));
        if rem.is_empty() || is_err_code {
            if rem.is_empty() || p_idx == 0 {
                out.extend_from_slice(mesg_bytes);
            } else {
                if mesg_bytes[0] != b'"'
                    || p_idx < 3
                    || mesg_bytes[p_idx - 2] != b'"'
                    || mesg_bytes[p_idx - 1] != b' '
                {
                    p_idx += 1;
                    continue;
                }

                out.extend_from_slice(rem);
                out.extend_from_slice(b" (");
                out.extend_from_slice(&mesg_bytes[1..p_idx - 2]);
                out.push(b')');
            }
            break;
        }
        p_idx += 1;
    }
    crate::memory::bytes_to_cstring(&out)
}

fn throw_exception(value: ExceptionValue, cmdname: Option<&CStr>) -> bool {
    let (r#type, mut messages, excp_value) = match value {
        ExceptionValue::User(val_str) => {
            let bytes = val_str.as_bytes();
            if bytes.starts_with(b"Vim")
                && (bytes.len() == 3 || bytes[3] == b':' || bytes[3] == b'(')
            {
                emsg(gettext(c"E608: Cannot :throw exceptions with 'Vim' prefix"));
                g_st().current_exception.set(None);
                return false;
            }
            (ET_USER, Vec::new(), val_str)
        }
        ExceptionValue::Interrupt(val_str) => (ET_INTERRUPT, Vec::new(), val_str),
        ExceptionValue::Error(msgs) => {
            let msg_bytes = msgs[msgs[0].throw_idx].msg.as_bytes();
            let throw_slice = &msg_bytes[msgs[0].throw_off.min(msg_bytes.len())..];
            let excp_cstr = get_error_exception_string(throw_slice, cmdname);
            (ET_ERROR, msgs, excp_cstr)
        }
    };

    let (throw_name, throw_lnum) = if r#type == ET_ERROR && messages[0].sfile.is_some() {
        let sfile = messages[0].sfile.take().unwrap();
        (sfile, messages[0].slnum)
    } else {
        let sfile = estack_sfile(ESTACK_NONE).unwrap_or_default();
        (sfile, sourcing_lnum())
    };

    let stacktrace = stacktrace_create();
    tv_list_ref(Some(&stacktrace));

    let excp = std::rc::Rc::new(Except {
        r#type,
        value: excp_value,
        messages,
        throw_name,
        throw_lnum,
        stacktrace,
    });

    if opt_vars().p_verbose.get() >= 13 || g_st().debug_break_level.get() > 0 {
        let save_msg_silent = g_st().msg_silent.get();

        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(0);
        } else {
            verbose_enter();
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().msg_scroll.set(1);
        }
        smsg!(0, gettext(c"Exception thrown: %s"), &excp.value,);
        msg_puts(c"\n");

        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().cmdline_row.set(g_st().msg_row.get());
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(save_msg_silent);
        } else {
            verbose_leave();
        }
    }

    g_st().current_exception.set(Some(excp));
    true
}

fn discard_exception(excp: Option<std::rc::Rc<Except>>, was_finished: bool) {
    let Some(excp) = excp else {
        internal_error(c"discard_exception()");
        return;
    };
    if g_st()
        .current_exception
        .get()
        .as_ref()
        .is_some_and(|cur| std::rc::Rc::ptr_eq(cur, &excp))
    {
        g_st().current_exception.set(None);
    }

    if opt_vars().p_verbose.get() >= 13 || g_st().debug_break_level.get() > 0 {
        let save_msg_silent = g_st().msg_silent.get();
        let saved_iobuff = g_st().iobuff_cstr().to_owned();
        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(0);
        } else {
            verbose_enter();
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().msg_scroll.set(1);
        }
        let fmt = if was_finished {
            gettext(c"Exception finished: %s")
        } else {
            gettext(c"Exception discarded: %s")
        };
        smsg!(0, fmt, &excp.value);
        msg_puts(c"\n");
        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().cmdline_row.set(g_st().msg_row.get());
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(save_msg_silent);
        } else {
            verbose_leave();
        }
        crate::memory::copy_bytes_to_c_buf(
            &mut *g_st().iobuff_bytes_mut(),
            saved_iobuff.as_bytes(),
        );
    }
    drop(excp);
}

pub fn discard_current_exception() {
    if let Some(excp) = g_st().current_exception.take() {
        discard_exception(Some(excp), false);
    }
    g_st().did_throw.set(false);
    g_st().need_rethrow.set(false);
}

fn catch_exception(excp: &std::rc::Rc<Except>) {
    g_st().caught_stack.borrow_mut().push(excp.clone());
    set_vim_var_string(VV_EXCEPTION, Some(&excp.value), -1);
    set_vim_var_list(VV_STACKTRACE, excp.stacktrace.opt());
    if !excp.throw_name.as_bytes().is_empty() {
        let iobufflen = if excp.throw_lnum != 0 {
            format_to_buf_safelen!(
                &mut *g_st().iobuff.borrow_mut(),
                gettext(c"%s, line %lld"),
                &excp.throw_name,
                i64::from(excp.throw_lnum),
            )
        } else {
            format_to_buf_safelen!(&mut *g_st().iobuff.borrow_mut(), c"%s", &excp.throw_name)
        };
        set_vim_var_string(
            VV_THROWPOINT,
            Some(&g_st().iobuff_cstr()),
            iobufflen as isize,
        );
    } else {
        set_vim_var_string(VV_THROWPOINT, None, -1);
    }

    if opt_vars().p_verbose.get() >= 13 || g_st().debug_break_level.get() > 0 {
        let save_msg_silent = g_st().msg_silent.get();

        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(0);
        } else {
            verbose_enter();
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().msg_scroll.set(1);
        }
        smsg!(0, gettext(c"Exception caught: %s"), &excp.value,);
        msg_puts(c"\n");

        if g_st().debug_break_level.get() > 0 || opt_vars().p_vfile.is_empty() {
            g_st().cmdline_row.set(g_st().msg_row.get());
        }
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
        if g_st().debug_break_level.get() > 0 {
            g_st().msg_silent.set(save_msg_silent);
        } else {
            verbose_leave();
        }
    }
}

fn finish_exception(excp: Option<std::rc::Rc<Except>>) {
    let mismatch = match (g_st().caught_stack.borrow().last(), excp.as_ref()) {
        (Some(last), Some(e)) => !std::rc::Rc::ptr_eq(last, e),
        _ => true,
    };
    if mismatch {
        internal_error(c"finish_exception()");
    }
    g_st().caught_stack.borrow_mut().pop();
    let top = g_st().caught_stack.borrow().last().cloned();
    if let Some(top) = top {
        set_vim_var_string(VV_EXCEPTION, Some(&top.value), -1);
        set_vim_var_list(VV_STACKTRACE, top.stacktrace.opt());
        if !top.throw_name.as_bytes().is_empty() {
            let iobufflen = if top.throw_lnum != 0 {
                format_to_buf_safelen!(
                    &mut *g_st().iobuff.borrow_mut(),
                    gettext(c"%s, line %lld"),
                    &top.throw_name,
                    i64::from(top.throw_lnum),
                )
            } else {
                format_to_buf_safelen!(&mut *g_st().iobuff.borrow_mut(), c"%s", &top.throw_name)
            };
            set_vim_var_string(
                VV_THROWPOINT,
                Some(&g_st().iobuff_cstr()),
                iobufflen as isize,
            );
        } else {
            set_vim_var_string(VV_THROWPOINT, None, -1);
        }
    } else {
        set_vim_var_string(VV_EXCEPTION, None, -1);
        set_vim_var_string(VV_THROWPOINT, None, -1);
        set_vim_var_list(VV_STACKTRACE, None);
    }

    discard_exception(excp, true);
}

pub fn exception_state_save(estate: &mut ExceptionState) {
    estate.estate_current_exception = g_st().current_exception.get().clone();
    estate.estate_did_throw = g_st().did_throw.get();
    estate.estate_need_rethrow = g_st().need_rethrow.get();
    estate.estate_trylevel = g_st().trylevel.get();
    estate.estate_did_emsg = g_st().did_emsg.get();
}

pub fn exception_state_restore(estate: &mut ExceptionState) {
    if g_st().did_throw.get() {
        handle_did_throw();
    }
    g_st()
        .current_exception
        .set(estate.estate_current_exception.take());
    g_st().did_throw.set(estate.estate_did_throw);
    g_st().need_rethrow.set(estate.estate_need_rethrow);
    g_st().trylevel.set(estate.estate_trylevel);
    g_st().did_emsg.set(estate.estate_did_emsg);
}

pub fn exception_state_clear() {
    g_st().current_exception.set(None);
    g_st().did_throw.set(false);
    g_st().need_rethrow.set(false);
    g_st().trylevel.set(0);
    g_st().did_emsg.set(0);
}

fn report_pending(action: i32, pending: i32, value: PendingVal) {
    let mut mesg: &std::ffi::CStr = match action {
        RP_MAKE => gettext(c"%s made pending"),
        RP_RESUME => gettext(c"%s resumed"),
        _ => gettext(c"%s discarded"),
    };

    let s_owned: Option<CString>;
    let mesg_owned: Option<CString>;
    let excp_hold: Option<std::rc::Rc<Except>>;
    let s: &std::ffi::CStr = match pending {
        CSTP_NONE => return,
        CSTP_CONTINUE => c":continue",
        CSTP_BREAK => c":break",
        CSTP_FINISH => c":finish",
        CSTP_RETURN => {
            let rettv = match value {
                PendingVal::Return(tv) => tv,
                _ => None,
            };
            s_owned = Some(get_return_cmd(rettv));
            s_owned.as_deref().unwrap_or(c"")
        }
        _ => {
            if (pending & CSTP_THROW) != 0 {
                excp_hold = match value {
                    PendingVal::Throw(excp) => excp,
                    _ => None,
                };
                let excp = excp_hold.as_ref().unwrap();
                format_to_buf!(
                    &mut *g_st().iobuff.borrow_mut(),
                    mesg,
                    gettext(c"Exception")
                );
                mesg_owned = Some(concat_str(&*g_st().iobuff_bytes(), b": %s"));
                mesg = mesg_owned.as_deref().unwrap_or(c"");
                excp.value.as_c_str()
            } else if (pending & CSTP_ERROR) != 0 && (pending & CSTP_INTERRUPT) != 0 {
                gettext(c"Error and interrupt")
            } else if (pending & CSTP_ERROR) != 0 {
                gettext(c"Error")
            } else {
                gettext(c"Interrupt")
            }
        }
    };

    let save_msg_silent = g_st().msg_silent.get();
    if g_st().debug_break_level.get() > 0 {
        g_st().msg_silent.set(0);
    }
    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
    g_st().msg_scroll.set(1);
    smsg!(0, mesg, s);
    msg_puts(c"\n");
    g_st().cmdline_row.set(g_st().msg_row.get());
    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    if g_st().debug_break_level.get() > 0 {
        g_st().msg_silent.set(save_msg_silent);
    }
}

pub fn report_make_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose.get() >= 14 || g_st().debug_break_level.get() > 0 {
        if g_st().debug_break_level.get() <= 0 {
            verbose_enter();
        }
        report_pending(RP_MAKE, pending, value);
        if g_st().debug_break_level.get() <= 0 {
            verbose_leave();
        }
    }
}

fn report_resume_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose.get() >= 14 || g_st().debug_break_level.get() > 0 {
        if g_st().debug_break_level.get() <= 0 {
            verbose_enter();
        }
        report_pending(RP_RESUME, pending, value);
        if g_st().debug_break_level.get() <= 0 {
            verbose_leave();
        }
    }
}

fn report_discard_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose.get() >= 14 || g_st().debug_break_level.get() > 0 {
        if g_st().debug_break_level.get() <= 0 {
            verbose_enter();
        }
        report_pending(RP_DISCARD, pending, value);
        if g_st().debug_break_level.get() <= 0 {
            verbose_leave();
        }
    }
}

pub fn ex_eval(eap: &mut Exarg) {
    let mut tv: TypVal = TypVal::new();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip);

    let arg = eap.arg.to_cstring();
    if eval0(&arg, &mut tv, Some(&mut *eap), &mut evalarg) {
        tv_clear(&mut tv);
    }
}

pub fn ex_if(eap: &mut Exarg) {
    if eap.cstack_mut().cs_idx == (CSTACK_LEN - 1) as i32 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E579: :if nesting too deep",
        )));
    } else {
        let (idx, skip) = {
            let cstack = eap.cstack_mut();
            cstack.cs_idx += 1;
            let idx = cstack.cs_idx as usize;
            cstack.cs_flags[idx] = 0;
            (idx, check_skip(&*cstack))
        };
        let mut error = false;
        let arg = eap.arg.to_cstring();
        let result = eval_to_bool(&arg, &mut error, eap, skip, false);

        let cstack = eap.cstack_mut();
        if !skip && !error {
            if result {
                cstack.cs_flags[idx] = CSF_ACTIVE | CSF_TRUE;
            }
        } else {
            cstack.cs_flags[idx] = CSF_TRUE;
        }
    }
}

pub fn ex_endif(eap: &mut Exarg) {
    g_st().did_endif.set(true);
    let cstack = eap.cstack_mut();
    if cstack.cs_idx < 0
        || (cstack.cs_flags[cstack.cs_idx as usize] & (CSF_WHILE | CSF_FOR | CSF_TRY)) != 0
    {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E580: :endif without :if",
        )));
    } else {
        if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRUE) == 0 && dbg_check_skipped(eap) {
            do_intthrow(eap.cstack_mut());
        }
        eap.cstack_mut().cs_idx -= 1;
    }
}

pub fn ex_else(eap: &mut Exarg) {
    let mut skip = check_skip(eap.cstack_mut());
    let cs_idx = eap.cstack_mut().cs_idx;
    if cs_idx < 0
        || (eap.cstack_mut().cs_flags[cs_idx as usize] & (CSF_WHILE | CSF_FOR | CSF_TRY)) != 0
    {
        if eap.cmdidx == Cmdidx::CmdElse {
            eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
                c"E581: :else without :if",
            )));
            return;
        }
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E582: :elseif without :if",
        )));
        skip = true;
    } else if (eap.cstack_mut().cs_flags[cs_idx as usize] & CSF_ELSE) != 0 {
        if eap.cmdidx == Cmdidx::CmdElse {
            eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(E_MULTIPLE_ELSE)));
            return;
        }
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E584: :elseif after :else",
        )));
        skip = true;
    }

    let idx = cs_idx as usize;
    if skip || (eap.cstack_mut().cs_flags[idx] & CSF_TRUE) != 0 {
        if eap.errmsg.is_none() {
            eap.cstack_mut().cs_flags[idx] = CSF_TRUE;
        }
        skip = true;
    } else {
        eap.cstack_mut().cs_flags[idx] = CSF_ACTIVE;
    }

    if !skip && dbg_check_skipped(eap) && g_st().got_int.get() {
        do_intthrow(eap.cstack_mut());
        skip = true;
    }

    if eap.cmdidx == Cmdidx::CmdElseif {
        let mut result = false;
        let mut error = false;
        if skip && eap.arg.first_byte() != b'"' && ends_excmd(i32::from(eap.arg.first_byte())) != 0
        {
            semsg!(gettext(E_INVEXPR2), eap.arg);
        } else {
            let arg = eap.arg.to_cstring();
            result = eval_to_bool(&arg, &mut error, eap, skip, false);
        }

        if !skip && !error {
            if result {
                eap.cstack_mut().cs_flags[idx] = CSF_ACTIVE | CSF_TRUE;
            } else {
                eap.cstack_mut().cs_flags[idx] = 0;
            }
        } else if eap.errmsg.is_none() {
            eap.cstack_mut().cs_flags[idx] = CSF_TRUE;
        }
    } else {
        eap.cstack_mut().cs_flags[idx] |= CSF_ELSE;
    }
}

pub fn ex_while(eap: &mut Exarg) {
    let mut error = false;
    let is_while = eap.cmdidx == Cmdidx::CmdWhile;

    if eap.cstack_mut().cs_idx == (CSTACK_LEN - 1) as i32 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E585: :while/:for nesting too deep",
        )));
    } else {
        let (idx, skip, had_loop) = {
            let cstack = eap.cstack_mut();
            if (cstack.cs_lflags & CSL_HAD_LOOP) == 0 {
                cstack.cs_idx += 1;
                cstack.cs_looplevel += 1;
                cstack.cs_line[cstack.cs_idx as usize] = -1;
            }
            let idx = cstack.cs_idx as usize;
            cstack.cs_flags[idx] = if is_while { CSF_WHILE } else { CSF_FOR };
            (
                idx,
                check_skip(&*cstack),
                (cstack.cs_lflags & CSL_HAD_LOOP) != 0,
            )
        };

        let arg = eap.arg.to_cstring();
        let result = if is_while {
            eval_to_bool(&arg, &mut error, eap, skip, false)
        } else {
            let mut evalarg = Evalarg::new();
            fill_evalarg_from_eap(&mut evalarg, skip);
            if had_loop {
                error = false;
            } else {
                let fi = eval_for_line(&arg, &mut error, eap, &mut evalarg);
                (&mut eap.cstack_mut().cs_forinfo)[idx] = Some(fi);
            }

            let cstack = eap.cstack_mut();
            let res = if !error && !skip {
                if let Some(fi) = (&mut cstack.cs_forinfo)[idx].as_deref_mut() {
                    next_for_item(fi, &arg)
                } else {
                    false
                }
            } else {
                false
            };

            if !res {
                (&mut cstack.cs_forinfo)[idx] = None;
            }
            res
        };

        let cstack = eap.cstack_mut();
        if !skip && !error && result {
            cstack.cs_flags[idx] |= CSF_ACTIVE | CSF_TRUE;
            cstack.cs_lflags ^= CSL_HAD_LOOP;
        } else {
            cstack.cs_lflags &= !CSL_HAD_LOOP;
            if !skip && !error {
                cstack.cs_flags[idx] |= CSF_TRUE;
            }
        }
    }
}

pub fn ex_continue(eap: &mut Exarg) {
    if eap.cstack_mut().cs_looplevel <= 0 || eap.cstack_mut().cs_idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E586: :continue without :while or :for",
        )));
    } else {
        let cstack = eap.cstack_mut();
        let idx = cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, false);
        assert!(idx >= 0);
        if (cstack.cs_flags[idx as usize] & (CSF_WHILE | CSF_FOR)) != 0 {
            let mut trylevel = cstack.cs_trylevel;
            rewind_conditionals(cstack, idx, CSF_TRY, &mut trylevel);
            cstack.cs_trylevel = trylevel;
            cstack.cs_lflags |= CSL_HAD_CONT;
        } else {
            cstack.cs_pending[idx as usize] = CSTP_CONTINUE as u8;
            report_make_pending(CSTP_CONTINUE, PendingVal::None);
        }
    }
}

pub fn ex_break(eap: &mut Exarg) {
    if eap.cstack_mut().cs_looplevel <= 0 || eap.cstack_mut().cs_idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E587: :break without :while or :for",
        )));
    } else {
        let cstack = eap.cstack_mut();
        let idx = cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, true);
        if idx >= 0 && (cstack.cs_flags[idx as usize] & (CSF_WHILE | CSF_FOR)) == 0 {
            cstack.cs_pending[idx as usize] = CSTP_BREAK as u8;
            report_make_pending(CSTP_BREAK, PendingVal::None);
        }
    }
}

pub fn ex_endwhile(eap: &mut Exarg) {
    let (err, csf) = if eap.cmdidx == Cmdidx::CmdEndwhile {
        (E_WHILE, CSF_WHILE)
    } else {
        (E_FOR, CSF_FOR)
    };

    if eap.cstack().cs_looplevel <= 0 || eap.cstack().cs_idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(err)));
    } else {
        let mut fl = eap.cstack().cs_flags[eap.cstack().cs_idx as usize];
        if (fl & csf) == 0 {
            if (fl & CSF_WHILE) != 0 {
                eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
                    c"E732: Using :endfor with :while",
                )));
            } else if (fl & CSF_FOR) != 0 {
                eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
                    c"E733: Using :endwhile with :for",
                )));
            }
        }
        if (fl & (CSF_WHILE | CSF_FOR)) == 0 {
            if (fl & CSF_TRY) == 0 {
                eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(E_ENDIF)));
            } else if (fl & CSF_FINALLY) != 0 {
                eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(E_ENDTRY)));
            }
            let mut idx = eap.cstack().cs_idx;
            while idx > 0 {
                fl = eap.cstack().cs_flags[idx as usize];
                if (fl & CSF_TRY) != 0 && (fl & CSF_FINALLY) == 0 {
                    eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(err)));
                    return;
                }
                if (fl & csf) != 0 {
                    break;
                }
                idx -= 1;
            }
            let cstack = eap.cstack_mut();
            cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, false);
            let mut trylevel = cstack.cs_trylevel;
            rewind_conditionals(cstack, idx, CSF_TRY, &mut trylevel);
            cstack.cs_trylevel = trylevel;
        } else if (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_TRUE) != 0
            && (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_ACTIVE) == 0
            && dbg_check_skipped(eap)
        {
            do_intthrow(eap.cstack_mut());
        }

        eap.cstack_mut().cs_lflags |= CSL_HAD_ENDLOOP;
    }
}

pub fn ex_throw(eap: &mut Exarg) {
    let arg = eap.arg.clone();
    let value = if !(arg).is_empty() && (arg).first_byte() != b'|' && (arg).first_byte() != b'\n' {
        eval_to_string_skip((arg).as_cstr(), eap, eap.skip)
    } else {
        emsg(gettext(E_ARGREQ));
        None
    };

    if let Some(val_cstr) = value
        && !eap.skip
        && throw_exception(ExceptionValue::User(val_cstr), None)
    {
        do_throw(eap.cstack_mut());
    }
}

pub fn do_throw(cstack: &mut Cstack) {
    let idx = cleanup_conditionals(&mut *cstack, 0, false);
    if idx >= 0 {
        let u_idx = idx as usize;
        if (cstack.cs_flags[u_idx] & CSF_CAUGHT) == 0 {
            if (cstack.cs_flags[u_idx] & CSF_ACTIVE) != 0 {
                cstack.cs_flags[u_idx] |= CSF_THROWN;
            } else {
                cstack.cs_flags[u_idx] &= !CSF_THROWN;
            }
        }
        cstack.cs_flags[u_idx] &= !CSF_ACTIVE;
        (&mut cstack.cs_exception)[u_idx] = g_st().current_exception.get().clone();
    }

    g_st().did_throw.set(true);
}

pub fn ex_try(eap: &mut Exarg) {
    if eap.cstack_mut().cs_idx == (CSTACK_LEN - 1) as i32 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E601: :try nesting too deep",
        )));
    } else {
        let cstack = eap.cstack_mut();
        cstack.cs_idx += 1;
        cstack.cs_trylevel += 1;
        let idx = cstack.cs_idx as usize;
        cstack.cs_flags[idx] = CSF_TRY;
        cstack.cs_pending[idx] = CSTP_NONE as u8;

        let skip = check_skip(&*cstack);
        if !skip {
            cstack.cs_flags[idx] |= CSF_ACTIVE | CSF_TRUE;

            if g_st().emsg_silent.get() != 0 {
                cstack.cs_emsg_silent_list.push(g_st().emsg_silent.get());
                cstack.cs_flags[idx] |= CSF_SILENT;
                g_st().emsg_silent.set(0);
            }
        }
    }
}

pub fn ex_catch(eap: &mut Exarg) {
    let mut idx: i32 = 0;
    let mut give_up = false;
    let mut skip = false;

    if eap.cstack().cs_trylevel <= 0 || eap.cstack().cs_idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E603: :catch without :try",
        )));
        give_up = true;
    } else {
        if (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_TRY) == 0 {
            let msg = get_end_emsg(eap.cstack());
            eap.errmsg = Some(std::borrow::Cow::Borrowed(msg));
            skip = true;
        }
        idx = eap.cstack().cs_idx;
        while idx > 0 {
            if (eap.cstack().cs_flags[idx as usize] & CSF_TRY) != 0 {
                break;
            }
            idx -= 1;
        }
        if (eap.cstack().cs_flags[idx as usize] & CSF_FINALLY) != 0 {
            eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
                c"E604: :catch after :finally",
            )));
            give_up = true;
        } else {
            let cstack = eap.cstack_mut();
            let mut looplevel = cstack.cs_looplevel;
            rewind_conditionals(cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
            cstack.cs_looplevel = looplevel;
        }
    }

    let (pat, end) = if ends_excmd(i32::from(eap.arg.first_byte())) != 0 {
        eap.nextcmd = eap.arg.advance_to_opt_cstr(find_nextcmd(eap.arg.as_cstr()));
        (crate::strings::OwnedSlice::from_cstr(c".*"), None)
    } else {
        let pat = eap.arg.wrapping_add(1);
        let end_opt = skip_regexp_err(pat.as_cstr(), i32::from(eap.arg.first_byte()), 1)
            .map(|s| pat.advance_to_cstr(s));
        if end_opt.is_none() {
            give_up = true;
        }
        (pat, end_opt)
    };

    if !give_up {
        let mut caught = false;
        let u_idx = idx as usize;
        if !g_st().did_throw.get() || (eap.cstack_mut().cs_flags[u_idx] & CSF_TRUE) == 0 {
            skip = true;
        }

        if !skip
            && (eap.cstack_mut().cs_flags[u_idx] & CSF_THROWN) != 0
            && (eap.cstack_mut().cs_flags[u_idx] & CSF_CAUGHT) == 0
        {
            if let Some(ref end_slice) = end
                && !end_slice.is_empty()
                && ends_excmd(i32::from(
                    crate::charset::skipwhite_slice(&end_slice.as_bytes()[1..])
                        .first()
                        .copied()
                        .unwrap_or(0),
                )) == 0
            {
                semsg!(gettext(E_TRAILING_ARG), end_slice);
                return;
            }

            if !dbg_check_skipped(eap) || !do_intthrow(eap.cstack_mut()) {
                let pat_cstr = if let Some(ref end_slice) = end {
                    let pat_len = end_slice.offset_from(&pat);
                    Some(crate::memory::bytes_to_cstring(pat.slice(pat_len)))
                } else {
                    None
                };
                let pat_ref: &CStr = pat_cstr.as_deref().unwrap_or_else(|| pat.as_cstr());
                let save_cpo = opt_vars().p_cpo.take();
                g_st().emsg_off.set(g_st().emsg_off.get() + (1));
                let mut regmatch: Regmatch = Regmatch::new();
                regmatch.set_prog(vim_regcomp(pat_ref, RE_MAGIC + RE_STRING));
                g_st().emsg_off.set(g_st().emsg_off.get() - (1));
                regmatch.rm_ic = false;
                opt_vars().p_cpo.set(save_cpo);
                if regmatch.regprog.is_null() {
                    semsg!(gettext(E_INVARG2), pat_ref);
                } else {
                    let prev_got_int = g_st().got_int.get();
                    g_st().got_int.set(false);
                    let cur_excp = g_st().current_exception.get().unwrap();
                    let val_cstr = cur_excp.value.as_c_str();
                    caught = vim_regexec_nl(&mut regmatch, Some(val_cstr), 0);
                    g_st().got_int.set(g_st().got_int.get() | (prev_got_int));
                    regmatch.free_prog();
                }
            }
        }

        let cstack = eap.cstack_mut();
        if caught {
            cstack.cs_flags[u_idx] |= CSF_ACTIVE | CSF_CAUGHT;
            g_st().did_emsg.set(0);
            g_st().got_int.set(false);
            g_st().did_throw.set(false);
            if let Some(ref excp) = cstack.cs_exception[u_idx] {
                catch_exception(excp);
            } else {
                internal_error(c"catch_exception()");
            }
            if !match (
                cstack.cs_exception[cstack.cs_idx as usize].as_ref(),
                g_st().current_exception.get().as_ref(),
            ) {
                (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            } {
                internal_error(c"ex_catch()");
            }
        } else {
            cleanup_conditionals(&mut *cstack, CSF_TRY, true);
        }
    }

    if let Some(ref end_slice) = end {
        eap.nextcmd = end_slice.advance_to_opt_cstr(find_nextcmd(end_slice.as_cstr()));
    }
}

pub fn ex_finally(eap: &mut Exarg) {
    let mut pending = CSTP_NONE;

    let mut idx = eap.cstack_mut().cs_idx;
    while idx >= 0 {
        if (eap.cstack_mut().cs_flags[idx as usize] & CSF_TRY) != 0 {
            break;
        }
        idx -= 1;
    }
    if eap.cstack_mut().cs_trylevel <= 0 || idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E606: :finally without :try",
        )));
        return;
    }

    if (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_TRY) == 0 {
        let msg = get_end_emsg(eap.cstack());
        eap.errmsg = Some(std::borrow::Cow::Borrowed(msg));
        pending = CSTP_ERROR;
    }

    if (eap.cstack_mut().cs_flags[idx as usize] & CSF_FINALLY) != 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(E_MULTIPLE_FINALLY)));
        return;
    }
    let mut looplevel = eap.cstack_mut().cs_looplevel;
    rewind_conditionals(eap.cstack_mut(), idx, CSF_WHILE | CSF_FOR, &mut looplevel);
    eap.cstack_mut().cs_looplevel = looplevel;

    let cur_idx = eap.cstack_mut().cs_idx as usize;
    let skip = (eap.cstack_mut().cs_flags[cur_idx] & CSF_TRUE) == 0;

    if !skip {
        if dbg_check_skipped(eap) {
            do_intthrow(eap.cstack_mut());
        }

        let cstack = eap.cstack_mut();
        cleanup_conditionals(&mut *cstack, CSF_TRY, false);

        let cur_idx = cstack.cs_idx as usize;
        if pending == CSTP_ERROR
            || g_st().did_emsg.get() != 0
            || g_st().got_int.get()
            || g_st().did_throw.get()
        {
            if i32::from(cstack.cs_pending[cur_idx]) == CSTP_RETURN {
                report_discard_pending(
                    CSTP_RETURN,
                    PendingVal::Return(cstack.cs_rettv[cur_idx].clone()),
                );
                discard_pending_return(&mut (&mut cstack.cs_rettv)[cur_idx]);
            }
            if (pending == CSTP_ERROR && g_st().did_emsg.get() == 0) || g_st().did_throw.get() {
                pending |= CSTP_THROW;
            }
            if g_st().did_emsg.get() != 0 {
                pending |= CSTP_ERROR;
            }
            if g_st().got_int.get() {
                pending |= CSTP_INTERRUPT;
            }
            cstack.cs_pending[cur_idx] = pending as u8;

            if g_st().did_throw.get()
                && !match (
                    cstack.cs_exception[cur_idx].as_ref(),
                    g_st().current_exception.get().as_ref(),
                ) {
                    (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
            {
                internal_error(c"ex_finally()");
            }
        }

        cstack.cs_lflags |= CSL_HAD_FINA;
    }
}

pub fn ex_endtry(eap: &mut Exarg) {
    let mut rethrow = false;
    let mut pending: i32 = CSTP_NONE;
    let mut rettv: Option<TypVal> = None;

    let mut idx = eap.cstack_mut().cs_idx;
    while idx >= 0 {
        if (eap.cstack_mut().cs_flags[idx as usize] & CSF_TRY) != 0 {
            break;
        }
        idx -= 1;
    }
    if eap.cstack_mut().cs_trylevel <= 0 || idx < 0 {
        eap.errmsg = Some(std::borrow::Cow::Borrowed(gettext(
            c"E602: :endtry without :try",
        )));
        return;
    }

    let mut skip = g_st().did_emsg.get() != 0
        || g_st().got_int.get()
        || g_st().did_throw.get()
        || (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_TRUE) == 0;

    if (eap.cstack().cs_flags[eap.cstack().cs_idx as usize] & CSF_TRY) == 0 {
        let msg = get_end_emsg(eap.cstack());
        eap.errmsg = Some(std::borrow::Cow::Borrowed(msg));
        let cstack = eap.cstack_mut();
        let mut looplevel = cstack.cs_looplevel;
        rewind_conditionals(cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
        cstack.cs_looplevel = looplevel;
        skip = true;

        if g_st().did_throw.get() {
            discard_current_exception();
        }

        g_st().did_emsg.set(0);
    } else {
        idx = eap.cstack_mut().cs_idx;
        if g_st().did_throw.get()
            && (eap.cstack_mut().cs_flags[idx as usize] & CSF_TRUE) != 0
            && (eap.cstack_mut().cs_flags[idx as usize] & CSF_FINALLY) == 0
        {
            rethrow = true;
        }
    }

    let u_idx = idx as usize;
    if (rethrow
        || (!skip
            && (eap.cstack_mut().cs_flags[u_idx] & CSF_FINALLY) == 0
            && eap.cstack_mut().cs_pending[u_idx] == 0))
        && dbg_check_skipped(eap)
        && g_st().got_int.get()
    {
        skip = true;
        let cstack = eap.cstack_mut();
        do_intthrow(cstack);
        rethrow = g_st().did_throw.get() && (cstack.cs_flags[u_idx] & CSF_FINALLY) == 0;
    }

    {
        let cstack = eap.cstack_mut();
        if !skip {
            pending = i32::from(cstack.cs_pending[u_idx]);
            cstack.cs_pending[u_idx] = CSTP_NONE as u8;
            if pending == CSTP_RETURN {
                rettv = (&mut cstack.cs_rettv)[u_idx].take();
            } else if (pending & CSTP_THROW) != 0 {
                g_st()
                    .current_exception
                    .set((&mut cstack.cs_exception)[u_idx].take());
            }
        }

        cleanup_conditionals(&mut *cstack, CSF_TRY | CSF_SILENT, true);

        if cstack.cs_idx >= 0 && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRY) != 0 {
            cstack.cs_idx -= 1;
        }
        cstack.cs_trylevel -= 1;
    }

    if !skip {
        let report_val = if pending == CSTP_RETURN {
            PendingVal::Return(rettv.clone())
        } else if (pending & CSTP_THROW) != 0 {
            PendingVal::Throw(g_st().current_exception.get().clone())
        } else {
            PendingVal::None
        };
        report_resume_pending(pending, report_val);
        match pending {
            CSTP_NONE => {}
            CSTP_CONTINUE => ex_continue(eap),
            CSTP_BREAK => ex_break(eap),
            CSTP_RETURN => {
                do_return(eap, false, false, rettv);
            }
            CSTP_FINISH => do_finish(eap, false),
            _ => {
                if (pending & CSTP_ERROR) != 0 {
                    g_st().did_emsg.set(1);
                }
                if (pending & CSTP_INTERRUPT) != 0 {
                    g_st().got_int.set(true);
                }
                if (pending & CSTP_THROW) != 0 {
                    rethrow = true;
                }
            }
        }
    }

    if rethrow {
        do_throw(eap.cstack_mut());
    }
}

pub fn enter_cleanup(csp: &mut ExCleanup) {
    let pending = CSTP_NONE;

    if g_st().did_emsg.get() != 0
        || g_st().got_int.get()
        || g_st().did_throw.get()
        || g_st().need_rethrow.get()
    {
        csp.pending = (if g_st().did_emsg.get() != 0 {
            CSTP_ERROR
        } else {
            0
        }) | (if g_st().got_int.get() {
            CSTP_INTERRUPT
        } else {
            0
        }) | (if g_st().did_throw.get() || g_st().need_rethrow.get() {
            CSTP_THROW
        } else {
            0
        });

        if g_st().did_throw.get() || g_st().need_rethrow.get() {
            csp.exception = g_st().current_exception.take();
        } else {
            csp.exception = None;
            if g_st().did_emsg.get() != 0 {
                g_st()
                    .force_abort
                    .set(g_st().force_abort.get() | (CAUSE_ABORT.replace(false)));
            }
        }
        g_st().did_emsg.set(0);
        g_st().got_int.set(false);
        g_st().did_throw.set(false);
        g_st().need_rethrow.set(false);

        report_make_pending(pending, PendingVal::Throw(csp.exception.clone()));
    } else {
        csp.pending = CSTP_NONE;
        csp.exception = None;
    }
}

pub fn leave_cleanup(csp: &mut ExCleanup) {
    let pending = csp.pending;
    if pending == CSTP_NONE {
        return;
    }

    if aborting() || g_st().need_rethrow.get() {
        if (pending & CSTP_THROW) != 0 {
            discard_exception(csp.exception.take(), false);
        } else {
            report_discard_pending(pending, PendingVal::None);
        }

        if g_st().msg_list.borrow().is_some() {
            free_global_msglist();
        }
    } else {
        if (pending & CSTP_THROW) != 0 {
            g_st().current_exception.set(csp.exception.take());
        } else if (pending & CSTP_ERROR) != 0 {
            CAUSE_ABORT.set(g_st().force_abort.get());
            g_st().force_abort.set(false);
        }

        if (pending & CSTP_ERROR) != 0 {
            g_st().did_emsg.set(1);
        }
        if (pending & CSTP_INTERRUPT) != 0 {
            g_st().got_int.set(true);
        }
        if (pending & CSTP_THROW) != 0 {
            g_st().need_rethrow.set(true);
        }

        report_resume_pending(
            pending,
            if (pending & CSTP_THROW) != 0 {
                PendingVal::Throw(g_st().current_exception.get().clone())
            } else {
                PendingVal::None
            },
        );
    }
}

pub fn cleanup_conditionals(cstack: &mut Cstack, searched_cond: i32, inclusive: bool) -> i32 {
    let mut idx = cstack.cs_idx;
    let mut stop = false;

    while idx >= 0 {
        let u_idx = idx as usize;
        if (cstack.cs_flags[u_idx] & CSF_TRY) != 0 {
            if g_st().did_emsg.get() != 0
                || g_st().got_int.get()
                || (cstack.cs_flags[u_idx] & CSF_FINALLY) != 0
            {
                let pend = i32::from(cstack.cs_pending[u_idx]);
                match pend {
                    CSTP_NONE => {}
                    CSTP_CONTINUE | CSTP_BREAK | CSTP_FINISH => {
                        report_discard_pending(pend, PendingVal::None);
                        cstack.cs_pending[u_idx] = CSTP_NONE as u8;
                    }
                    CSTP_RETURN => {
                        report_discard_pending(
                            CSTP_RETURN,
                            PendingVal::Return(cstack.cs_rettv[u_idx].clone()),
                        );
                        discard_pending_return(&mut (&mut cstack.cs_rettv)[u_idx]);
                        cstack.cs_pending[u_idx] = CSTP_NONE as u8;
                    }
                    _ => {
                        if (cstack.cs_flags[u_idx] & CSF_FINALLY) != 0 {
                            if (pend & CSTP_THROW) != 0 && cstack.cs_exception[u_idx].is_some() {
                                discard_exception((&mut cstack.cs_exception)[u_idx].take(), false);
                            } else {
                                report_discard_pending(pend, PendingVal::None);
                            }
                            cstack.cs_pending[u_idx] = CSTP_NONE as u8;
                        }
                    }
                }
            }

            if (cstack.cs_flags[u_idx] & CSF_FINALLY) == 0 {
                if (cstack.cs_flags[u_idx] & CSF_ACTIVE) != 0
                    && (cstack.cs_flags[u_idx] & CSF_CAUGHT) != 0
                    && (cstack.cs_flags[u_idx] & CSF_FINISHED) == 0
                {
                    finish_exception((&mut cstack.cs_exception)[u_idx].take());
                    cstack.cs_flags[u_idx] |= CSF_FINISHED;
                }
                if (cstack.cs_flags[u_idx] & CSF_TRUE) != 0 {
                    if searched_cond == 0 && !inclusive {
                        break;
                    }
                    stop = true;
                }
            }
        }

        if (cstack.cs_flags[u_idx] & searched_cond) != 0 {
            if !inclusive {
                break;
            }
            stop = true;
        }
        cstack.cs_flags[u_idx] &= !CSF_ACTIVE;
        if stop && searched_cond != (CSF_TRY | CSF_SILENT) {
            break;
        }

        if (cstack.cs_flags[u_idx] & CSF_TRY) != 0 && (cstack.cs_flags[u_idx] & CSF_SILENT) != 0 {
            if let Some(saved) = cstack.cs_emsg_silent_list.pop() {
                g_st().emsg_silent.set(saved);
            }
            cstack.cs_flags[u_idx] &= !CSF_SILENT;
        }
        if stop {
            break;
        }
        idx -= 1;
    }
    idx
}

fn get_end_emsg(cstack: &Cstack) -> &'static CStr {
    let fl = cstack.cs_flags[cstack.cs_idx as usize];
    if (fl & CSF_WHILE) != 0 {
        return gettext(E_ENDWHILE);
    }
    if (fl & CSF_FOR) != 0 {
        return gettext(E_ENDFOR);
    }
    gettext(E_ENDIF)
}

pub fn rewind_conditionals(cstack: &mut Cstack, idx: i32, cond_type: i32, cond_level: &mut i32) {
    while cstack.cs_idx > idx {
        let cur_idx = cstack.cs_idx as usize;
        if (cstack.cs_flags[cur_idx] & cond_type) != 0 {
            *cond_level -= 1;
        }
        if (cstack.cs_flags[cur_idx] & CSF_FOR) != 0 {
            (&mut cstack.cs_forinfo)[cur_idx] = None;
        }
        cstack.cs_idx -= 1;
    }
}

pub fn ex_endfunction(_eap: &mut Exarg) {
    semsg!(gettext(E_STR_NOT_INSIDE_FUNCTION), c":endfunction",);
}

pub fn has_loop_cmd(mut rem: &[u8]) -> bool {
    loop {
        let skip = rem
            .iter()
            .take_while(|&&b| matches!(b, b' ' | b'\t' | b':'))
            .count();
        rem = &rem[skip..];
        let len = modifier_len(rem);
        if len == 0 {
            break;
        }
        rem = &rem[(len as usize).min(rem.len())..];
    }
    rem.starts_with(b"wh") || rem.starts_with(b"for")
}
