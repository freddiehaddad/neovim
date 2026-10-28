//! Ex command-line evaluation, conditionals, loops, and exception handling (`src/nvim/ex_eval.c`).

use crate::charset::{ascii_isdigit, skipwhite};
use crate::cursor::Linenr;
use crate::debugger::dbg_check_skipped;
use crate::errors::*;
use crate::eval::typval::{TvList, TypVal, tv_clear, tv_list_ref, tv_list_unref};
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
use crate::memory::xstrlcpy;
use crate::message::{emsg, internal_error, msg_puts, semsg, smsg, verbose_enter, verbose_leave};
use crate::option_vars::*;
use crate::regexp::{
    RE_MAGIC, RE_STRING, Regmatch, skip_regexp_err, vim_regcomp, vim_regexec_nl, vim_regfree,
};
use crate::runtime::{ESTACK_NONE, do_finish, estack_sfile, sourcing_lnum, stacktrace_create};
use crate::strings::{concat_str, gettext, vim_snprintf, vim_snprintf_safelen};
use std::ffi::{CStr, CString, c_char};
use std::ptr;

const CSTACK_LEN: usize = 50;

const CMD_ELSE: i32 = 142;
const CMD_ELSEIF: i32 = 143;
const CMD_ENDWHILE: i32 = 149;
const CMD_WHILE: i32 = 532;

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
    pub cs_pending: [c_char; CSTACK_LEN],
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
            cs_rettv: [None; CSTACK_LEN],
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
    pub stacktrace: *mut TvList,
}

impl Drop for Except {
    fn drop(&mut self) {
        tv_list_unref(self.stacktrace);
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

use crate::charset::NUL;
use crate::globals::IOSIZE;
use crate::math::FAIL;

struct ExEvalModuleState {
    cause_abort: bool,
}

impl ExEvalModuleState {
    const fn new() -> Self {
        Self { cause_abort: false }
    }
}

static EXEVAL_STATE: crate::globals::GlobalState<ExEvalModuleState> =
    crate::globals::GlobalState::new(ExEvalModuleState::new());

#[inline]
fn exeval_st() -> &'static mut ExEvalModuleState {
    unsafe { &mut *EXEVAL_STATE.as_ptr() }
}

#[inline]
fn check_skip(cstack: &Cstack) -> bool {
    g_st().did_emsg != 0
        || g_st().got_int
        || g_st().did_throw
        || (cstack.cs_idx > 0 && (cstack.cs_flags[(cstack.cs_idx - 1) as usize] & CSF_ACTIVE) == 0)
}

#[inline]
fn discard_pending_return(p: &mut Option<TypVal>) {
    if let Some(ref mut tv) = p.take() {
        tv_clear(tv);
    }
}

pub fn aborting() -> bool {
    (g_st().did_emsg != 0 && g_st().force_abort) || g_st().got_int || g_st().did_throw
}

pub fn update_force_abort() {
    if exeval_st().cause_abort {
        g_st().force_abort = true;
    }
}

pub fn should_abort(retcode: i32) -> bool {
    (retcode == FAIL && g_st().trylevel != 0 && g_st().emsg_silent == 0) || aborting()
}

pub fn aborted_in_try() -> bool {
    g_st().force_abort
}

pub fn cause_errthrow(
    mesg: impl crate::strings::IntoCCharPtr,
    multiline: bool,
    concat: bool,
    severe: bool,
    ignore: &mut bool,
) -> bool {
    let mesg = mesg.into_c_char_ptr();
    unsafe {
        if g_st().suppress_errthrow {
            return false;
        }

        if g_st().did_emsg == 0 {
            exeval_st().cause_abort = g_st().force_abort;
            g_st().force_abort = false;
        }

        if ((g_st().trylevel == 0 && !exeval_st().cause_abort) || g_st().emsg_silent != 0)
            && !g_st().did_throw
        {
            return false;
        }

        if ptr::eq(mesg, gettext(E_INTERR).cast_const()) {
            *ignore = true;
            return true;
        }

        exeval_st().cause_abort = true;

        if g_st().did_throw {
            if g_st()
                .current_exception
                .as_ref()
                .is_some_and(|e| e.r#type == ET_INTERRUPT)
            {
                g_st().got_int = false;
            }
            discard_current_exception();
        }

        let ml = &raw mut g_st().msg_list;
        if let Some(ref mut list) = *ml {
            if concat && let Some(last) = list.last_mut() {
                let mut bytes = last.msg.as_bytes().to_vec();
                bytes.extend_from_slice(crate::memory::cstr_as_bytes(mesg));
                last.msg = CString::new(bytes).unwrap();
                return true;
            }

            let is_first = list.is_empty();
            let tmsg = crate::memory::cstr_to_cstring(mesg);
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
}

pub fn free_global_msglist() {
    unsafe {
        let ml = &raw mut g_st().msg_list;
        if let Some(ref mut list) = *ml {
            list.clear();
        }
    }
}

pub fn do_errthrow(cstack: Option<&mut Cstack>, cmdname: impl crate::strings::IntoCCharPtr) {
    let cmdname = cmdname.into_c_char_ptr();
    unsafe {
        if exeval_st().cause_abort {
            exeval_st().cause_abort = false;
            g_st().force_abort = true;
        }

        let ml = &raw mut g_st().msg_list;
        let Some(ref mut list) = *ml else {
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
                g_st().need_rethrow = true;
            }
        }
    }
}

pub fn do_intthrow(cstack: *mut Cstack) -> bool {
    {
        if !g_st().got_int || (g_st().trylevel == 0 && !g_st().did_throw) {
            return false;
        }

        if g_st().did_throw {
            if g_st()
                .current_exception
                .as_ref()
                .is_some_and(|e| e.r#type == ET_INTERRUPT)
            {
                return false;
            }
            discard_current_exception();
        }
        if throw_exception(
            ExceptionValue::Interrupt(c"Vim:Interrupt".to_owned()),
            ptr::null_mut(),
        ) {
            do_throw(unsafe { &mut *(cstack) });
        }

        true
    }
}

enum ExceptionValue {
    Error(Vec<Msglist>),
    Interrupt(CString),
    User(CString),
}

pub fn get_error_exception_string(
    mesg: impl crate::strings::IntoCCharPtr,
    cmdname: impl crate::strings::IntoCCharPtr,
) -> CString {
    let cmdname = cmdname.into_c_char_ptr();
    let mesg = mesg.into_c_char_ptr();
    unsafe {
        let mut out: Vec<u8> = Vec::new();
        if !crate::memory::cstr_is_empty(cmdname) {
            out.extend_from_slice(b"Vim(");
            out.extend_from_slice(crate::memory::cstr_as_bytes(cmdname));
            out.extend_from_slice(b"):");
        } else {
            out.extend_from_slice(b"Vim:");
        }

        let mut p = mesg;
        loop {
            if crate::memory::cstr_is_empty(p)
                || (crate::memory::cstr_first_byte(p) == b'E'
                    && ascii_isdigit(crate::memory::cstr_char_at(p, 1))
                    && (crate::memory::cstr_byte_at(p, 2) == b':'
                        || (ascii_isdigit(crate::memory::cstr_char_at(p, 2))
                            && (crate::memory::cstr_byte_at(p, 3) == b':'
                                || (ascii_isdigit(crate::memory::cstr_char_at(p, 3))
                                    && crate::memory::cstr_byte_at(p, 4) == b':')))))
            {
                if crate::memory::cstr_is_empty(p) || ptr::eq(p, mesg) {
                    out.extend_from_slice(crate::memory::cstr_as_bytes(mesg));
                } else {
                    if crate::memory::cstr_first_byte(mesg) != b'"'
                        || (p as usize) < (mesg.wrapping_add(3) as usize)
                        || crate::memory::cstr_first_byte(p.wrapping_sub(2)) != b'"'
                        || crate::memory::cstr_first_byte(p.wrapping_sub(1)) != b' '
                    {
                        p = p.wrapping_add(1);
                        continue;
                    }

                    out.extend_from_slice(crate::memory::cstr_as_bytes(p));
                    let quoted_len = (p.wrapping_sub(2) as usize) - (mesg.wrapping_add(1) as usize);
                    out.extend_from_slice(b" (");
                    out.extend_from_slice(std::slice::from_raw_parts(
                        mesg.wrapping_add(1).cast::<u8>(),
                        quoted_len,
                    ));
                    out.push(b')');
                }
                break;
            }
            p = p.wrapping_add(1);
        }
        crate::memory::cstr_from_bytes(&out)
    }
}

fn throw_exception(value: ExceptionValue, cmdname: impl crate::strings::IntoCCharPtr) -> bool {
    let cmdname = cmdname.into_c_char_ptr();
    {
        let (r#type, mut messages, excp_value) = match value {
            ExceptionValue::User(val_str) => {
                let bytes = val_str.as_bytes();
                if bytes.starts_with(b"Vim")
                    && (bytes.len() == 3 || bytes[3] == b':' || bytes[3] == b'(')
                {
                    emsg(gettext(c"E608: Cannot :throw exceptions with 'Vim' prefix"));
                    g_st().current_exception = None;
                    return false;
                }
                (ET_USER, Vec::new(), val_str)
            }
            ExceptionValue::Interrupt(val_str) => (ET_INTERRUPT, Vec::new(), val_str),
            ExceptionValue::Error(msgs) => {
                let throw_ptr = msgs[msgs[0].throw_idx]
                    .msg
                    .as_ptr()
                    .wrapping_add(msgs[0].throw_off);
                let excp_cstr = get_error_exception_string(throw_ptr, cmdname);
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
        tv_list_ref(Some(&mut *stacktrace));

        let excp = std::rc::Rc::new(Except {
            r#type,
            value: excp_value,
            messages,
            throw_name,
            throw_lnum,
            stacktrace,
        });

        if opt_vars().p_verbose >= 13 || g_st().debug_break_level > 0 {
            let save_msg_silent = g_st().msg_silent;

            if g_st().debug_break_level > 0 {
                g_st().msg_silent = 0;
            } else {
                verbose_enter();
            }
            g_st().no_wait_return += 1;
            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().msg_scroll = 1;
            }
            smsg!(0, gettext(c"Exception thrown: %s"), excp.value.as_ptr(),);
            msg_puts(c"\n");

            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().cmdline_row = g_st().msg_row;
            }
            g_st().no_wait_return -= 1;
            if g_st().debug_break_level > 0 {
                g_st().msg_silent = save_msg_silent;
            } else {
                verbose_leave();
            }
        }

        g_st().current_exception = Some(excp);
        true
    }
}

fn discard_exception(excp: Option<std::rc::Rc<Except>>, was_finished: bool) {
    {
        let Some(excp) = excp else {
            internal_error(c"discard_exception()");
            return;
        };
        if g_st()
            .current_exception
            .as_ref()
            .is_some_and(|cur| std::rc::Rc::ptr_eq(cur, &excp))
        {
            g_st().current_exception = None;
        }

        if opt_vars().p_verbose >= 13 || g_st().debug_break_level > 0 {
            let save_msg_silent = g_st().msg_silent;
            let saved_iobuff = crate::memory::cstr_to_cstring(ptr::addr_of!(g_st().iobuff).cast());
            if g_st().debug_break_level > 0 {
                g_st().msg_silent = 0;
            } else {
                verbose_enter();
            }
            g_st().no_wait_return += 1;
            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().msg_scroll = 1;
            }
            let fmt = if was_finished {
                gettext(c"Exception finished: %s")
            } else {
                gettext(c"Exception discarded: %s")
            };
            smsg!(0, fmt, excp.value.as_ptr());
            msg_puts(c"\n");
            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().cmdline_row = g_st().msg_row;
            }
            g_st().no_wait_return -= 1;
            if g_st().debug_break_level > 0 {
                g_st().msg_silent = save_msg_silent;
            } else {
                verbose_leave();
            }
            xstrlcpy(
                ptr::addr_of_mut!(g_st().iobuff).cast(),
                saved_iobuff.as_ptr(),
                IOSIZE,
            );
        }
        drop(excp);
    }
}

pub fn discard_current_exception() {
    {
        if let Some(excp) = g_st().current_exception.take() {
            discard_exception(Some(excp), false);
        }
        g_st().did_throw = false;
        g_st().need_rethrow = false;
    }
}

fn catch_exception(excp: &std::rc::Rc<Except>) {
    {
        g_st().caught_stack.push(excp.clone());
        set_vim_var_string(VV_EXCEPTION, excp.value.as_ptr(), -1);
        set_vim_var_list(VV_STACKTRACE, excp.stacktrace);
        if !excp.throw_name.as_bytes().is_empty() {
            let iobufflen = if excp.throw_lnum != 0 {
                vim_snprintf_safelen!(
                    ptr::addr_of_mut!(g_st().iobuff).cast(),
                    IOSIZE,
                    gettext(c"%s, line %lld"),
                    excp.throw_name.as_ptr(),
                    i64::from(excp.throw_lnum),
                )
            } else {
                vim_snprintf_safelen!(
                    ptr::addr_of_mut!(g_st().iobuff).cast(),
                    IOSIZE,
                    c"%s",
                    excp.throw_name.as_ptr(),
                )
            };
            set_vim_var_string(
                VV_THROWPOINT,
                ptr::addr_of!(g_st().iobuff).cast(),
                iobufflen as isize,
            );
        } else {
            set_vim_var_string(VV_THROWPOINT, ptr::null(), -1);
        }

        if opt_vars().p_verbose >= 13 || g_st().debug_break_level > 0 {
            let save_msg_silent = g_st().msg_silent;

            if g_st().debug_break_level > 0 {
                g_st().msg_silent = 0;
            } else {
                verbose_enter();
            }
            g_st().no_wait_return += 1;
            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().msg_scroll = 1;
            }
            smsg!(0, gettext(c"Exception caught: %s"), excp.value.as_ptr(),);
            msg_puts(c"\n");

            if g_st().debug_break_level > 0 || crate::memory::cstr_is_empty(opt_vars().p_vfile) {
                g_st().cmdline_row = g_st().msg_row;
            }
            g_st().no_wait_return -= 1;
            if g_st().debug_break_level > 0 {
                g_st().msg_silent = save_msg_silent;
            } else {
                verbose_leave();
            }
        }
    }
}

fn finish_exception(excp: Option<std::rc::Rc<Except>>) {
    {
        if match (g_st().caught_stack.last(), excp.as_ref()) {
            (Some(last), Some(e)) => !std::rc::Rc::ptr_eq(last, e),
            _ => true,
        } {
            internal_error(c"finish_exception()");
        }
        g_st().caught_stack.pop();
        if let Some(top) = g_st().caught_stack.last() {
            set_vim_var_string(VV_EXCEPTION, top.value.as_ptr(), -1);
            set_vim_var_list(VV_STACKTRACE, top.stacktrace);
            if !top.throw_name.as_bytes().is_empty() {
                let iobufflen = if top.throw_lnum != 0 {
                    vim_snprintf_safelen!(
                        ptr::addr_of_mut!(g_st().iobuff).cast(),
                        IOSIZE,
                        gettext(c"%s, line %lld"),
                        top.throw_name.as_ptr(),
                        i64::from(top.throw_lnum),
                    )
                } else {
                    vim_snprintf_safelen!(
                        ptr::addr_of_mut!(g_st().iobuff).cast(),
                        IOSIZE,
                        c"%s",
                        top.throw_name.as_ptr(),
                    )
                };
                set_vim_var_string(
                    VV_THROWPOINT,
                    ptr::addr_of!(g_st().iobuff).cast(),
                    iobufflen as isize,
                );
            } else {
                set_vim_var_string(VV_THROWPOINT, ptr::null(), -1);
            }
        } else {
            set_vim_var_string(VV_EXCEPTION, ptr::null(), -1);
            set_vim_var_string(VV_THROWPOINT, ptr::null(), -1);
            set_vim_var_list(VV_STACKTRACE, ptr::null_mut());
        }

        discard_exception(excp, true);
    }
}

pub fn exception_state_save(estate: &mut ExceptionState) {
    {
        estate.estate_current_exception = g_st().current_exception.clone();
        estate.estate_did_throw = g_st().did_throw;
        estate.estate_need_rethrow = g_st().need_rethrow;
        estate.estate_trylevel = g_st().trylevel;
        estate.estate_did_emsg = g_st().did_emsg;
    }
}

pub fn exception_state_restore(estate: &mut ExceptionState) {
    if g_st().did_throw {
        handle_did_throw();
    }
    g_st().current_exception = estate.estate_current_exception.take();
    g_st().did_throw = estate.estate_did_throw;
    g_st().need_rethrow = estate.estate_need_rethrow;
    g_st().trylevel = estate.estate_trylevel;
    g_st().did_emsg = estate.estate_did_emsg;
}

pub fn exception_state_clear() {
    g_st().current_exception = None;
    g_st().did_throw = false;
    g_st().need_rethrow = false;
    g_st().trylevel = 0;
    g_st().did_emsg = 0;
}

fn report_pending(action: i32, pending: i32, value: PendingVal) {
    {
        let mut mesg = match action {
            RP_MAKE => gettext(c"%s made pending"),
            RP_RESUME => gettext(c"%s resumed"),
            _ => gettext(c"%s discarded"),
        };

        let s_owned: Option<CString>;
        let mesg_owned: Option<CString>;
        let excp_hold: Option<std::rc::Rc<Except>>;
        let s: *const c_char = match pending {
            CSTP_NONE => return,
            CSTP_CONTINUE => c":continue".as_ptr(),
            CSTP_BREAK => c":break".as_ptr(),
            CSTP_FINISH => c":finish".as_ptr(),
            CSTP_RETURN => {
                let rettv = match value {
                    PendingVal::Return(tv) => tv,
                    _ => None,
                };
                s_owned = Some(get_return_cmd(rettv));
                s_owned.as_ref().map_or(ptr::null(), |cs| cs.as_ptr())
            }
            _ => {
                if (pending & CSTP_THROW) != 0 {
                    excp_hold = match value {
                        PendingVal::Throw(excp) => excp,
                        _ => None,
                    };
                    let excp = excp_hold.as_ref().unwrap();
                    vim_snprintf!(
                        ptr::addr_of_mut!(g_st().iobuff).cast(),
                        IOSIZE,
                        mesg,
                        gettext(c"Exception"),
                    );
                    mesg_owned = Some(concat_str(ptr::addr_of!(g_st().iobuff).cast(), c": %s"));
                    mesg = mesg_owned
                        .as_ref()
                        .map_or(ptr::null_mut(), |cs| cs.as_ptr().cast_mut());
                    excp.value.as_ptr()
                } else if (pending & CSTP_ERROR) != 0 && (pending & CSTP_INTERRUPT) != 0 {
                    gettext(c"Error and interrupt")
                } else if (pending & CSTP_ERROR) != 0 {
                    gettext(c"Error")
                } else {
                    gettext(c"Interrupt")
                }
            }
        };

        let save_msg_silent = g_st().msg_silent;
        if g_st().debug_break_level > 0 {
            g_st().msg_silent = 0;
        }
        g_st().no_wait_return += 1;
        g_st().msg_scroll = 1;
        smsg!(0, mesg, s);
        msg_puts(c"\n");
        g_st().cmdline_row = g_st().msg_row;
        g_st().no_wait_return -= 1;
        if g_st().debug_break_level > 0 {
            g_st().msg_silent = save_msg_silent;
        }
    }
}

pub fn report_make_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose >= 14 || g_st().debug_break_level > 0 {
        if g_st().debug_break_level <= 0 {
            verbose_enter();
        }
        report_pending(RP_MAKE, pending, value);
        if g_st().debug_break_level <= 0 {
            verbose_leave();
        }
    }
}

fn report_resume_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose >= 14 || g_st().debug_break_level > 0 {
        if g_st().debug_break_level <= 0 {
            verbose_enter();
        }
        report_pending(RP_RESUME, pending, value);
        if g_st().debug_break_level <= 0 {
            verbose_leave();
        }
    }
}

fn report_discard_pending(pending: i32, value: PendingVal) {
    if opt_vars().p_verbose >= 14 || g_st().debug_break_level > 0 {
        if g_st().debug_break_level <= 0 {
            verbose_enter();
        }
        report_pending(RP_DISCARD, pending, value);
        if g_st().debug_break_level <= 0 {
            verbose_leave();
        }
    }
}

pub fn ex_eval(eap: &mut Exarg) {
    let mut tv: TypVal = TypVal::new();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip != 0);

    if eval0(eap.arg, &mut tv, Some(&mut *eap), &mut evalarg) {
        tv_clear(&mut tv);
    }
}

pub fn ex_if(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_idx == (CSTACK_LEN - 1) as i32 {
            eap.errmsg = gettext(c"E579: :if nesting too deep");
        } else {
            cstack.cs_idx += 1;
            let idx = cstack.cs_idx as usize;
            cstack.cs_flags[idx] = 0;

            let skip = check_skip(&*cstack);
            let mut error = false;
            let result = eval_to_bool(eap.arg, &mut error, eap, skip, false);

            if !skip && !error {
                if result {
                    cstack.cs_flags[idx] = CSF_ACTIVE | CSF_TRUE;
                }
            } else {
                cstack.cs_flags[idx] = CSF_TRUE;
            }
        }
    }
}

pub fn ex_endif(eap: &mut Exarg) {
    unsafe {
        g_st().did_endif = true;
        let cstack = &mut *eap.cstack.cast::<Cstack>();
        if cstack.cs_idx < 0
            || (cstack.cs_flags[cstack.cs_idx as usize] & (CSF_WHILE | CSF_FOR | CSF_TRY)) != 0
        {
            eap.errmsg = gettext(c"E580: :endif without :if");
        } else {
            if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRUE) == 0 && dbg_check_skipped(eap) {
                do_intthrow(cstack);
            }
            cstack.cs_idx -= 1;
        }
    }
}

pub fn ex_else(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();
        let mut skip = check_skip(&*cstack);

        if cstack.cs_idx < 0
            || (cstack.cs_flags[cstack.cs_idx as usize] & (CSF_WHILE | CSF_FOR | CSF_TRY)) != 0
        {
            if eap.cmdidx == CMD_ELSE {
                eap.errmsg = gettext(c"E581: :else without :if");
                return;
            }
            eap.errmsg = gettext(c"E582: :elseif without :if");
            skip = true;
        } else if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_ELSE) != 0 {
            if eap.cmdidx == CMD_ELSE {
                eap.errmsg = gettext(E_MULTIPLE_ELSE);
                return;
            }
            eap.errmsg = gettext(c"E584: :elseif after :else");
            skip = true;
        }

        let idx = cstack.cs_idx as usize;
        if skip || (cstack.cs_flags[idx] & CSF_TRUE) != 0 {
            if eap.errmsg.is_null() {
                cstack.cs_flags[idx] = CSF_TRUE;
            }
            skip = true;
        } else {
            cstack.cs_flags[idx] = CSF_ACTIVE;
        }

        if !skip && dbg_check_skipped(eap) && g_st().got_int {
            do_intthrow(cstack);
            skip = true;
        }

        if eap.cmdidx == CMD_ELSEIF {
            let mut result = false;
            let mut error = false;
            if skip
                && crate::memory::cstr_first_byte(eap.arg) != b'"'
                && ends_excmd(i32::from(crate::memory::cstr_first_byte(eap.arg))) != 0
            {
                semsg!(gettext(E_INVEXPR2), eap.arg);
            } else {
                result = eval_to_bool(eap.arg, &mut error, eap, skip, false);
            }

            if !skip && !error {
                if result {
                    cstack.cs_flags[idx] = CSF_ACTIVE | CSF_TRUE;
                } else {
                    cstack.cs_flags[idx] = 0;
                }
            } else if eap.errmsg.is_null() {
                cstack.cs_flags[idx] = CSF_TRUE;
            }
        } else {
            cstack.cs_flags[idx] |= CSF_ELSE;
        }
    }
}

pub fn ex_while(eap: &mut Exarg) {
    unsafe {
        let mut error = false;
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_idx == (CSTACK_LEN - 1) as i32 {
            eap.errmsg = gettext(c"E585: :while/:for nesting too deep");
        } else {
            if (cstack.cs_lflags & CSL_HAD_LOOP) == 0 {
                cstack.cs_idx += 1;
                cstack.cs_looplevel += 1;
                cstack.cs_line[cstack.cs_idx as usize] = -1;
            }
            let idx = cstack.cs_idx as usize;
            cstack.cs_flags[idx] = if eap.cmdidx == CMD_WHILE {
                CSF_WHILE
            } else {
                CSF_FOR
            };

            let skip = check_skip(&*cstack);
            let result = if eap.cmdidx == CMD_WHILE {
                eval_to_bool(eap.arg, &mut error, eap, skip, false)
            } else {
                let mut evalarg = Evalarg::new();
                fill_evalarg_from_eap(&mut evalarg, skip);
                if (cstack.cs_lflags & CSL_HAD_LOOP) != 0 {
                    error = false;
                } else {
                    let fi = eval_for_line(eap.arg, &mut error, eap, &mut evalarg);
                    (&mut cstack.cs_forinfo)[idx] = Some(fi);
                }

                let res = if !error && !skip {
                    if let Some(fi) = (&mut cstack.cs_forinfo)[idx].as_deref_mut() {
                        next_for_item(fi, eap.arg)
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
}

pub fn ex_continue(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_looplevel <= 0 || cstack.cs_idx < 0 {
            eap.errmsg = gettext(c"E586: :continue without :while or :for");
        } else {
            let idx = cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, 0);
            assert!(idx >= 0);
            if (cstack.cs_flags[idx as usize] & (CSF_WHILE | CSF_FOR)) != 0 {
                let mut trylevel = cstack.cs_trylevel;
                rewind_conditionals(cstack, idx, CSF_TRY, &mut trylevel);
                cstack.cs_trylevel = trylevel;
                cstack.cs_lflags |= CSL_HAD_CONT;
            } else {
                cstack.cs_pending[idx as usize] = CSTP_CONTINUE as c_char;
                report_make_pending(CSTP_CONTINUE, PendingVal::None);
            }
        }
    }
}

pub fn ex_break(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_looplevel <= 0 || cstack.cs_idx < 0 {
            eap.errmsg = gettext(c"E587: :break without :while or :for");
        } else {
            let idx = cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, 1);
            if idx >= 0 && (cstack.cs_flags[idx as usize] & (CSF_WHILE | CSF_FOR)) == 0 {
                cstack.cs_pending[idx as usize] = CSTP_BREAK as c_char;
                report_make_pending(CSTP_BREAK, PendingVal::None);
            }
        }
    }
}

pub fn ex_endwhile(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();
        let (err, csf) = if eap.cmdidx == CMD_ENDWHILE {
            (E_WHILE.as_ptr(), CSF_WHILE)
        } else {
            (E_FOR.as_ptr(), CSF_FOR)
        };

        if cstack.cs_looplevel <= 0 || cstack.cs_idx < 0 {
            eap.errmsg = gettext(err);
        } else {
            let mut fl = cstack.cs_flags[cstack.cs_idx as usize];
            if (fl & csf) == 0 {
                if (fl & CSF_WHILE) != 0 {
                    eap.errmsg = gettext(c"E732: Using :endfor with :while");
                } else if (fl & CSF_FOR) != 0 {
                    eap.errmsg = gettext(c"E733: Using :endwhile with :for");
                }
            }
            if (fl & (CSF_WHILE | CSF_FOR)) == 0 {
                if (fl & CSF_TRY) == 0 {
                    eap.errmsg = gettext(E_ENDIF);
                } else if (fl & CSF_FINALLY) != 0 {
                    eap.errmsg = gettext(E_ENDTRY);
                }
                let mut idx = cstack.cs_idx;
                while idx > 0 {
                    fl = cstack.cs_flags[idx as usize];
                    if (fl & CSF_TRY) != 0 && (fl & CSF_FINALLY) == 0 {
                        eap.errmsg = gettext(err);
                        return;
                    }
                    if (fl & csf) != 0 {
                        break;
                    }
                    idx -= 1;
                }
                cleanup_conditionals(&mut *cstack, CSF_WHILE | CSF_FOR, 0);
                let mut trylevel = cstack.cs_trylevel;
                rewind_conditionals(cstack, idx, CSF_TRY, &mut trylevel);
                cstack.cs_trylevel = trylevel;
            } else if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRUE) != 0
                && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_ACTIVE) == 0
                && dbg_check_skipped(eap)
            {
                do_intthrow(cstack);
            }

            cstack.cs_lflags |= CSL_HAD_ENDLOOP;
        }
    }
}

pub fn ex_throw(eap: &mut Exarg) {
    {
        let arg = eap.arg;
        let value = if !crate::memory::cstr_is_empty(arg)
            && crate::memory::cstr_first_byte(arg) != b'|'
            && crate::memory::cstr_first_byte(arg) != b'\n'
        {
            eval_to_string_skip(arg, eap, eap.skip != 0)
        } else {
            emsg(gettext(E_ARGREQ));
            None
        };

        if let Some(val_cstr) = value
            && eap.skip == 0
            && throw_exception(ExceptionValue::User(val_cstr), ptr::null_mut())
        {
            do_throw(unsafe { &mut *(eap.cstack.cast()) });
        }
    }
}

pub fn do_throw(cstack: &mut Cstack) {
    {
        let idx = cleanup_conditionals(&mut *cstack, 0, 0);
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
            (&mut cstack.cs_exception)[u_idx] = g_st().current_exception.clone();
        }

        g_st().did_throw = true;
    }
}

pub fn ex_try(eap: &mut Exarg) {
    unsafe {
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_idx == (CSTACK_LEN - 1) as i32 {
            eap.errmsg = gettext(c"E601: :try nesting too deep");
        } else {
            cstack.cs_idx += 1;
            cstack.cs_trylevel += 1;
            let idx = cstack.cs_idx as usize;
            cstack.cs_flags[idx] = CSF_TRY;
            cstack.cs_pending[idx] = CSTP_NONE as c_char;

            let skip = check_skip(&*cstack);
            if !skip {
                cstack.cs_flags[idx] |= CSF_ACTIVE | CSF_TRUE;

                if g_st().emsg_silent != 0 {
                    cstack.cs_emsg_silent_list.push(g_st().emsg_silent);
                    cstack.cs_flags[idx] |= CSF_SILENT;
                    g_st().emsg_silent = 0;
                }
            }
        }
    }
}

pub fn ex_catch(eap: &mut Exarg) {
    unsafe {
        let mut idx: i32 = 0;
        let mut give_up = false;
        let mut skip = false;
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        if cstack.cs_trylevel <= 0 || cstack.cs_idx < 0 {
            eap.errmsg = gettext(c"E603: :catch without :try");
            give_up = true;
        } else {
            if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRY) == 0 {
                eap.errmsg = get_end_emsg(&*cstack);
                skip = true;
            }
            idx = cstack.cs_idx;
            while idx > 0 {
                if (cstack.cs_flags[idx as usize] & CSF_TRY) != 0 {
                    break;
                }
                idx -= 1;
            }
            if (cstack.cs_flags[idx as usize] & CSF_FINALLY) != 0 {
                eap.errmsg = gettext(c"E604: :catch after :finally");
                give_up = true;
            } else {
                let mut looplevel = cstack.cs_looplevel;
                rewind_conditionals(cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
                cstack.cs_looplevel = looplevel;
            }
        }

        let (pat, end): (*const c_char, *mut c_char) =
            if ends_excmd(i32::from(crate::memory::cstr_first_byte(eap.arg))) != 0 {
                eap.nextcmd = find_nextcmd(eap.arg);
                (c".*".as_ptr(), ptr::null_mut())
            } else {
                let pat = eap.arg.wrapping_add(1);
                let end =
                    skip_regexp_err(pat, i32::from(crate::memory::cstr_first_byte(eap.arg)), 1);
                if end.is_null() {
                    give_up = true;
                }
                (pat, end)
            };

        if !give_up {
            let mut caught = false;
            let u_idx = idx as usize;
            if !g_st().did_throw || (cstack.cs_flags[u_idx] & CSF_TRUE) == 0 {
                skip = true;
            }

            if !skip
                && (cstack.cs_flags[u_idx] & CSF_THROWN) != 0
                && (cstack.cs_flags[u_idx] & CSF_CAUGHT) == 0
            {
                if !crate::memory::cstr_is_empty(end)
                    && ends_excmd(i32::from(*skipwhite(end.wrapping_add(1)) as u8)) == 0
                {
                    semsg!(gettext(E_TRAILING_ARG), end);
                    return;
                }

                if !dbg_check_skipped(eap) || !do_intthrow(cstack) {
                    let mut save_char: c_char = 0;
                    if !end.is_null() {
                        save_char = *end;
                        *end = NUL;
                    }
                    let save_cpo = opt_vars().p_cpo;
                    opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();
                    g_st().emsg_off += 1;
                    let mut regmatch: Regmatch = Regmatch::new();
                    regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
                    g_st().emsg_off -= 1;
                    regmatch.rm_ic = false;
                    if !end.is_null() {
                        *end = save_char;
                    }
                    opt_vars().p_cpo = save_cpo;
                    if regmatch.regprog.is_null() {
                        semsg!(gettext(E_INVARG2), pat);
                    } else {
                        let prev_got_int = g_st().got_int;
                        g_st().got_int = false;
                        let val_ptr = g_st()
                            .current_exception
                            .as_ref()
                            .unwrap()
                            .value
                            .as_ptr()
                            .cast_mut();
                        caught = vim_regexec_nl(&mut regmatch, val_ptr, 0);
                        g_st().got_int |= prev_got_int;
                        vim_regfree(regmatch.regprog);
                    }
                }
            }

            if caught {
                cstack.cs_flags[u_idx] |= CSF_ACTIVE | CSF_CAUGHT;
                g_st().did_emsg = 0;
                g_st().got_int = false;
                g_st().did_throw = false;
                if let Some(ref excp) = cstack.cs_exception[u_idx] {
                    catch_exception(excp);
                } else {
                    internal_error(c"catch_exception()");
                }
                if !match (
                    cstack.cs_exception[cstack.cs_idx as usize].as_ref(),
                    g_st().current_exception.as_ref(),
                ) {
                    (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                } {
                    internal_error(c"ex_catch()");
                }
            } else {
                cleanup_conditionals(&mut *cstack, CSF_TRY, 1);
            }
        }

        if !end.is_null() {
            eap.nextcmd = find_nextcmd(end);
        }
    }
}

pub fn ex_finally(eap: &mut Exarg) {
    unsafe {
        let mut pending = CSTP_NONE;
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        let mut idx = cstack.cs_idx;
        while idx >= 0 {
            if (cstack.cs_flags[idx as usize] & CSF_TRY) != 0 {
                break;
            }
            idx -= 1;
        }
        if cstack.cs_trylevel <= 0 || idx < 0 {
            eap.errmsg = gettext(c"E606: :finally without :try");
            return;
        }

        if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRY) == 0 {
            eap.errmsg = get_end_emsg(&*cstack);
            pending = CSTP_ERROR;
        }

        if (cstack.cs_flags[idx as usize] & CSF_FINALLY) != 0 {
            eap.errmsg = gettext(E_MULTIPLE_FINALLY);
            return;
        }
        let mut looplevel = cstack.cs_looplevel;
        rewind_conditionals(cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
        cstack.cs_looplevel = looplevel;

        let cur_idx = cstack.cs_idx as usize;
        let skip = (cstack.cs_flags[cur_idx] & CSF_TRUE) == 0;

        if !skip {
            if dbg_check_skipped(eap) {
                do_intthrow(cstack);
            }

            cleanup_conditionals(&mut *cstack, CSF_TRY, 0);

            let cur_idx = cstack.cs_idx as usize;
            if pending == CSTP_ERROR || g_st().did_emsg != 0 || g_st().got_int || g_st().did_throw {
                if i32::from(cstack.cs_pending[cur_idx]) == CSTP_RETURN {
                    report_discard_pending(
                        CSTP_RETURN,
                        PendingVal::Return(cstack.cs_rettv[cur_idx]),
                    );
                    discard_pending_return(&mut (&mut cstack.cs_rettv)[cur_idx]);
                }
                if (pending == CSTP_ERROR && g_st().did_emsg == 0) || g_st().did_throw {
                    pending |= CSTP_THROW;
                }
                if g_st().did_emsg != 0 {
                    pending |= CSTP_ERROR;
                }
                if g_st().got_int {
                    pending |= CSTP_INTERRUPT;
                }
                cstack.cs_pending[cur_idx] = pending as c_char;

                if g_st().did_throw
                    && !match (
                        cstack.cs_exception[cur_idx].as_ref(),
                        g_st().current_exception.as_ref(),
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
}

pub fn ex_endtry(eap: &mut Exarg) {
    unsafe {
        let mut rethrow = false;
        let mut pending: i32 = CSTP_NONE;
        let mut rettv: Option<TypVal> = None;
        let cstack = &mut *eap.cstack.cast::<Cstack>();

        let mut idx = cstack.cs_idx;
        while idx >= 0 {
            if (cstack.cs_flags[idx as usize] & CSF_TRY) != 0 {
                break;
            }
            idx -= 1;
        }
        if cstack.cs_trylevel <= 0 || idx < 0 {
            eap.errmsg = gettext(c"E602: :endtry without :try");
            return;
        }

        let mut skip = g_st().did_emsg != 0
            || g_st().got_int
            || g_st().did_throw
            || (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRUE) == 0;

        if (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRY) == 0 {
            eap.errmsg = get_end_emsg(&*cstack);
            let mut looplevel = cstack.cs_looplevel;
            rewind_conditionals(cstack, idx, CSF_WHILE | CSF_FOR, &mut looplevel);
            cstack.cs_looplevel = looplevel;
            skip = true;

            if g_st().did_throw {
                discard_current_exception();
            }

            g_st().did_emsg = 0;
        } else {
            idx = cstack.cs_idx;
            if g_st().did_throw
                && (cstack.cs_flags[idx as usize] & CSF_TRUE) != 0
                && (cstack.cs_flags[idx as usize] & CSF_FINALLY) == 0
            {
                rethrow = true;
            }
        }

        let u_idx = idx as usize;
        if (rethrow
            || (!skip
                && (cstack.cs_flags[u_idx] & CSF_FINALLY) == 0
                && cstack.cs_pending[u_idx] == 0))
            && dbg_check_skipped(eap)
            && g_st().got_int
        {
            skip = true;
            do_intthrow(cstack);
            rethrow = g_st().did_throw && (cstack.cs_flags[u_idx] & CSF_FINALLY) == 0;
        }

        if !skip {
            pending = i32::from(cstack.cs_pending[u_idx]);
            cstack.cs_pending[u_idx] = CSTP_NONE as c_char;
            if pending == CSTP_RETURN {
                rettv = (&mut cstack.cs_rettv)[u_idx].take();
            } else if (pending & CSTP_THROW) != 0 {
                g_st().current_exception = (&mut cstack.cs_exception)[u_idx].take();
            }
        }

        cleanup_conditionals(&mut *cstack, CSF_TRY | CSF_SILENT, 1);

        if cstack.cs_idx >= 0 && (cstack.cs_flags[cstack.cs_idx as usize] & CSF_TRY) != 0 {
            cstack.cs_idx -= 1;
        }
        cstack.cs_trylevel -= 1;

        if !skip {
            let report_val = if pending == CSTP_RETURN {
                PendingVal::Return(rettv)
            } else if (pending & CSTP_THROW) != 0 {
                PendingVal::Throw(g_st().current_exception.clone())
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
                        g_st().did_emsg = 1;
                    }
                    if (pending & CSTP_INTERRUPT) != 0 {
                        g_st().got_int = true;
                    }
                    if (pending & CSTP_THROW) != 0 {
                        rethrow = true;
                    }
                }
            }
        }

        if rethrow {
            do_throw(cstack);
        }
    }
}

pub fn enter_cleanup(csp: &mut ExCleanup) {
    {
        let pending = CSTP_NONE;

        if g_st().did_emsg != 0 || g_st().got_int || g_st().did_throw || g_st().need_rethrow {
            csp.pending = (if g_st().did_emsg != 0 { CSTP_ERROR } else { 0 })
                | (if g_st().got_int { CSTP_INTERRUPT } else { 0 })
                | (if g_st().did_throw || g_st().need_rethrow {
                    CSTP_THROW
                } else {
                    0
                });

            if g_st().did_throw || g_st().need_rethrow {
                csp.exception = g_st().current_exception.take();
            } else {
                csp.exception = None;
                if g_st().did_emsg != 0 {
                    g_st().force_abort |= exeval_st().cause_abort;
                    exeval_st().cause_abort = false;
                }
            }
            g_st().did_emsg = 0;
            g_st().got_int = false;
            g_st().did_throw = false;
            g_st().need_rethrow = false;

            report_make_pending(pending, PendingVal::Throw(csp.exception.clone()));
        } else {
            csp.pending = CSTP_NONE;
            csp.exception = None;
        }
    }
}

pub fn leave_cleanup(csp: &mut ExCleanup) {
    {
        let pending = csp.pending;
        if pending == CSTP_NONE {
            return;
        }

        if aborting() || g_st().need_rethrow {
            if (pending & CSTP_THROW) != 0 {
                discard_exception(csp.exception.take(), false);
            } else {
                report_discard_pending(pending, PendingVal::None);
            }

            if g_st().msg_list.is_some() {
                free_global_msglist();
            }
        } else {
            if (pending & CSTP_THROW) != 0 {
                g_st().current_exception = csp.exception.take();
            } else if (pending & CSTP_ERROR) != 0 {
                exeval_st().cause_abort = g_st().force_abort;
                g_st().force_abort = false;
            }

            if (pending & CSTP_ERROR) != 0 {
                g_st().did_emsg = 1;
            }
            if (pending & CSTP_INTERRUPT) != 0 {
                g_st().got_int = true;
            }
            if (pending & CSTP_THROW) != 0 {
                g_st().need_rethrow = true;
            }

            report_resume_pending(
                pending,
                if (pending & CSTP_THROW) != 0 {
                    PendingVal::Throw(g_st().current_exception.clone())
                } else {
                    PendingVal::None
                },
            );
        }
    }
}

pub fn cleanup_conditionals(cstack: &mut Cstack, searched_cond: i32, inclusive: i32) -> i32 {
    let mut idx = cstack.cs_idx;
    let mut stop = false;

    while idx >= 0 {
        let u_idx = idx as usize;
        if (cstack.cs_flags[u_idx] & CSF_TRY) != 0 {
            if g_st().did_emsg != 0 || g_st().got_int || (cstack.cs_flags[u_idx] & CSF_FINALLY) != 0
            {
                let pend = i32::from(cstack.cs_pending[u_idx]);
                match pend {
                    CSTP_NONE => {}
                    CSTP_CONTINUE | CSTP_BREAK | CSTP_FINISH => {
                        report_discard_pending(pend, PendingVal::None);
                        cstack.cs_pending[u_idx] = CSTP_NONE as c_char;
                    }
                    CSTP_RETURN => {
                        report_discard_pending(
                            CSTP_RETURN,
                            PendingVal::Return(cstack.cs_rettv[u_idx]),
                        );
                        discard_pending_return(&mut (&mut cstack.cs_rettv)[u_idx]);
                        cstack.cs_pending[u_idx] = CSTP_NONE as c_char;
                    }
                    _ => {
                        if (cstack.cs_flags[u_idx] & CSF_FINALLY) != 0 {
                            if (pend & CSTP_THROW) != 0 && cstack.cs_exception[u_idx].is_some() {
                                discard_exception((&mut cstack.cs_exception)[u_idx].take(), false);
                            } else {
                                report_discard_pending(pend, PendingVal::None);
                            }
                            cstack.cs_pending[u_idx] = CSTP_NONE as c_char;
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
                    if searched_cond == 0 && inclusive == 0 {
                        break;
                    }
                    stop = true;
                }
            }
        }

        if (cstack.cs_flags[u_idx] & searched_cond) != 0 {
            if inclusive == 0 {
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
                g_st().emsg_silent = saved;
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

fn get_end_emsg(cstack: &Cstack) -> *mut c_char {
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

pub fn has_loop_cmd(p: impl crate::strings::IntoCCharPtr) -> bool {
    let mut p = p.into_c_char_ptr();
    {
        loop {
            while crate::memory::cstr_first_byte(p) == b' '
                || crate::memory::cstr_first_byte(p) == b'\t'
                || crate::memory::cstr_first_byte(p) == b':'
            {
                p = p.wrapping_add(1);
            }
            let len = modifier_len(p);
            if len == 0 {
                break;
            }
            p = p.wrapping_add(len as usize);
        }
        (crate::memory::cstr_first_byte(p) == b'w' && crate::memory::cstr_byte_at(p, 1) == b'h')
            || (crate::memory::cstr_first_byte(p) == b'f'
                && crate::memory::cstr_byte_at(p, 1) == b'o'
                && crate::memory::cstr_byte_at(p, 2) == b'r')
    }
}
