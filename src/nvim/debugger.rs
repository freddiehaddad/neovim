//! Vimscript debugger (`src/nvim/debugger.c`).

use crate::charset::{getdigits_int32, skipwhite};
use crate::drawscreen::{UPD_NOT_VALID, redraw_all_later};
use crate::errors::{e_invarg2, e_noname};
use crate::eval::typval::{Callback, tv_clear, typval_T};
use crate::eval::{EXPR_IS, eval_expr, typval_compare, typval_tostring};
use crate::ex_docmd::{DOCMD_EXCRESET, do_cmdline, do_cmdline_cmd, exarg_T};
use crate::ex_getln::getcmdline_prompt;
use crate::fileio::file_pat_to_reg_pat;
use crate::globals::*;
use crate::input::{restore_typeahead, save_typeahead, tasave_T};
use crate::math::OK;

use crate::message::{emsg, msg, msg_starthere, semsg, smsg};
use crate::os::env::{expand_env_save, home_replace};
use crate::path::fix_fname;
use crate::regexp::{RE_MAGIC, RE_STRING, vim_regcomp_box, vim_regexec_prog};
use crate::runtime::{ESTACK_NONE, estack_sfile, sourcing_lnum};
use std::ffi::{CStr, CString, c_char, c_int};

const DBG_FUNC: c_int = 1;
const DBG_FILE: c_int = 2;
const DBG_EXPR: c_int = 3;

const CMD_BREAKDEL: c_int = 38;
const CMD_PROFILE: c_int = 338;
const CMD_PROFDEL: c_int = 339;

const CMD_CONT: c_int = 1;
const CMD_NEXT: c_int = 2;
const CMD_STEP: c_int = 3;
const CMD_FINISH: c_int = 4;
const CMD_QUIT: c_int = 5;
const CMD_INTERRUPT: c_int = 6;
const CMD_BACKTRACE: c_int = 7;
const CMD_FRAME: c_int = 8;
const CMD_UP: c_int = 9;
const CMD_DOWN: c_int = 10;

struct DebuggerState {
    DEBUG_GREEDY: bool,
    LAST_CMD: c_int,
    LAST_BREAKP: c_int,
    HAS_EXPR_BREAKPOINT: bool,
    DEBUG_SKIPPED: bool,
    DEBUG_BREAKPOINT_LNUM: c_int,
    DEBUG_OLDVAL: Option<CString>,
    DEBUG_NEWVAL: Option<CString>,
    DEBUG_BREAKPOINT_NAME: *mut c_char,
    DEBUG_SKIPPED_NAME: *mut c_char,
    DBG_BREAKP: Vec<debuggy>,
    PROF_GA: Vec<debuggy>,
}

impl DebuggerState {
    const fn new() -> Self {
        Self {
            DEBUG_GREEDY: false,
            LAST_CMD: 0,
            LAST_BREAKP: 0,
            HAS_EXPR_BREAKPOINT: false,
            DEBUG_SKIPPED: false,
            DEBUG_BREAKPOINT_LNUM: 0,
            DEBUG_OLDVAL: None,
            DEBUG_NEWVAL: None,
            DEBUG_BREAKPOINT_NAME: std::ptr::null_mut(),
            DEBUG_SKIPPED_NAME: std::ptr::null_mut(),
            DBG_BREAKP: Vec::new(),
            PROF_GA: Vec::new(),
        }
    }
}

static DBG_STATE: GlobalState<DebuggerState> = GlobalState::new(DebuggerState::new());

#[inline]
fn dbg_st() -> &'static mut DebuggerState {
    unsafe { &mut *DBG_STATE.as_ptr() }
}

#[inline]
fn dbg_vec_mut(is_prof: bool) -> &'static mut Vec<debuggy> {
    if is_prof {
            &mut dbg_st().PROF_GA
        } else {
            &mut dbg_st().DBG_BREAKP
        }
}

#[repr(C)]
pub struct debuggy {
    pub dbg_nr: c_int,
    pub dbg_type: c_int,
    pub dbg_name: std::ffi::CString,
    pub dbg_prog: Option<Box<crate::regexp::regprog_T>>,
    pub dbg_lnum: i32,
    pub dbg_forceit: c_int,
    pub dbg_val: Option<typval_T>,
    pub dbg_level: c_int,
}

impl Drop for debuggy {
    fn drop(&mut self) {
        if let Some(ref mut val) = self.dbg_val {
                tv_clear(val);
            }
    }
}

use crate::autocmd::DOCMD_VERBOSE;
use crate::cmdexpand::EXPAND_NOTHING;
use crate::keycodes::{K_SPECIAL, KE_SNR, KS_EXTRA};
use crate::os::fs::MAXPATHL;
use crate::state::MODE_NORMAL;

fn get_maxbacktrace_level(sname: *const c_char) -> c_int {
    if sname.is_null() {
        return 0;
    }
    let bytes = unsafe { CStr::from_ptr(sname).to_bytes() };
    let mut count: c_int = 0;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'.' && bytes[i + 1] == b'.' {
            count += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    count
}

fn do_checkbacktracelevel() {
    if g_st().debug_backtrace_level < 0 {
            g_st().debug_backtrace_level = 0;
            msg(c"frame is zero".as_ptr(), 0);
        } else {
            let sname = estack_sfile(ESTACK_NONE);
            let max =
                get_maxbacktrace_level(sname.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()));
            if g_st().debug_backtrace_level > max {
                g_st().debug_backtrace_level = max;
                smsg!(0, c"frame at highest level: %d".as_ptr(), max);
            }
        }
}

fn do_setdebugtracelevel(arg: *mut c_char) {
    unsafe {
        let level = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(arg).to_bytes()) as c_int;
        if *arg as u8 == b'+' || level < 0 {
            g_st().debug_backtrace_level += level;
        } else {
            g_st().debug_backtrace_level = level;
        }
        do_checkbacktracelevel();
    }
}

fn do_showbacktrace(cmd: *const c_char) {
    unsafe {
        let sname_cstr = estack_sfile(ESTACK_NONE);
        let max =
            get_maxbacktrace_level(sname_cstr.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()));
        if let Some(sname_cstr) = sname_cstr {
            let mut sname_vec: Vec<c_char> = sname_cstr
                .as_bytes_with_nul()
                .iter()
                .map(|&b| b as c_char)
                .collect();
            let mut i: c_int = 0;
            let mut cur = sname_vec.as_mut_ptr();
            while !g_st().got_int {
                let cur_bytes = CStr::from_ptr(cur).to_bytes();
                let next_off = cur_bytes.windows(2).position(|w| w == b"..");
                if let Some(off) = next_off {
                    *cur.add(off) = 0;
                }
                if i == max - g_st().debug_backtrace_level {
                    smsg!(0, c"->%d %s".as_ptr(), max - i, cur);
                } else {
                    smsg!(0, c"  %d %s".as_ptr(), max - i, cur);
                }
                i += 1;
                if let Some(off) = next_off {
                    *cur.add(off) = b'.' as c_char;
                    cur = cur.add(off + 2);
                } else {
                    break;
                }
            }
        }

        let slnum = sourcing_lnum();
        if slnum != 0 {
            let line_msg = format!("line {slnum}: %s\0");
            smsg!(0, line_msg.as_ptr().cast::<c_char>(), cmd);
        } else {
            smsg!(0, c"cmd: %s".as_ptr(), cmd);
        }
    }
}

/// Interactive debug mode loop (`do_debug`).
pub fn do_debug(cmd: *mut c_char) {
    unsafe {
        let save_msg_scroll = g_st().msg_scroll;
        let save_state = g_st().State;
        let save_did_emsg = g_st().did_emsg;
        let save_cmd_silent = g_st().cmd_silent;
        let save_msg_silent = g_st().msg_silent;
        let save_emsg_silent = g_st().emsg_silent;
        let save_redir_off = g_st().redir_off;
        let mut typeaheadbuf = tasave_T::new();
        let mut typeahead_saved = false;
        let mut save_ignore_script = false;
        let mut cmdline_cstr: Option<CString>;

        g_st().RedrawingDisabled += 1;
        g_st().no_wait_return += 1;
        g_st().did_emsg = 0;
        g_st().cmd_silent = false;
        g_st().msg_silent = 0;
        g_st().emsg_silent = 0;
        g_st().redir_off = true;

        g_st().State = MODE_NORMAL;
        g_st().debug_mode = true;

        if !g_st().debug_did_msg {
            msg(
                c"Entering Debug mode.  Type \"cont\" to continue.".as_ptr(),
                0,
            );
        }
        if let Some(oldval) = dbg_st().DEBUG_OLDVAL.take() {
            smsg!(0, c"Oldval = \"%s\"".as_ptr(), oldval.as_ptr());
        }
        if let Some(newval) = dbg_st().DEBUG_NEWVAL.take() {
            smsg!(0, c"Newval = \"%s\"".as_ptr(), newval.as_ptr());
        }
        if let Some(sname) = estack_sfile(ESTACK_NONE) {
            msg(sname.as_ptr(), 0);
        }
        let slnum = sourcing_lnum();
        if slnum != 0 {
            let line_msg = format!("line {slnum}: %s\0");
            smsg!(0, line_msg.as_ptr().cast::<c_char>(), cmd);
        } else {
            smsg!(0, c"cmd: %s".as_ptr(), cmd);
        }

        loop {
            g_st().msg_scroll = 1;
            g_st().need_wait_return = false;

            let save_ex_normal_busy = g_st().ex_normal_busy;
            g_st().ex_normal_busy = 0;
            if !dbg_st().DEBUG_GREEDY {
                save_typeahead(&mut typeaheadbuf);
                typeahead_saved = true;
                save_ignore_script = g_st().ignore_script;
                g_st().ignore_script = true;
            }

            let n = g_st().debug_break_level;
            g_st().debug_break_level = -1;

            cmdline_cstr = getcmdline_prompt(
                b'>' as c_int,
                std::ptr::null(),
                0,
                EXPAND_NOTHING,
                std::ptr::null(),
                Callback::NONE,
                false,
                std::ptr::null_mut(),
            );
            let cmdline = cmdline_cstr
                .as_ref()
                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut());

            g_st().debug_break_level = n;
            if typeahead_saved {
                restore_typeahead(&mut typeaheadbuf);
                g_st().ignore_script = save_ignore_script;
            }
            g_st().ex_normal_busy = save_ex_normal_busy;

            g_st().cmdline_row = g_st().msg_row;
            msg_starthere();
            if !cmdline.is_null() {
                let mut p = skipwhite(cmdline);
                if *p != 0 {
                    let b0 = *p as u8;
                    let b1 = *p.add(1) as u8;
                    let (mut lcmd, mut tail): (c_int, &[u8]) = match b0 {
                        b'c' => (CMD_CONT, b"ont"),
                        b'n' => (CMD_NEXT, b"ext"),
                        b's' => (CMD_STEP, b"tep"),
                        b'f' => {
                            if b1 == b'r' {
                                (CMD_FRAME, b"rame")
                            } else {
                                (CMD_FINISH, b"inish")
                            }
                        }
                        b'q' => (CMD_QUIT, b"uit"),
                        b'i' => (CMD_INTERRUPT, b"nterrupt"),
                        b'b' => {
                            if b1 == b't' {
                                (CMD_BACKTRACE, b"t")
                            } else {
                                (CMD_BACKTRACE, b"acktrace")
                            }
                        }
                        b'w' => (CMD_BACKTRACE, b"here"),
                        b'u' => (CMD_UP, b"p"),
                        b'd' => (CMD_DOWN, b"own"),
                        _ => (0, b""),
                    };

                    if lcmd != 0 {
                        p = p.add(1);
                        while *p != 0 && !tail.is_empty() && (*p as u8) == tail[0] {
                            p = p.add(1);
                            tail = &tail[1..];
                        }
                        if (*p as u8).is_ascii_alphabetic() && lcmd != CMD_FRAME {
                            lcmd = 0;
                        }
                    }
                    dbg_st().LAST_CMD = lcmd;
                }

                let cur_last = dbg_st().LAST_CMD;
                if cur_last != 0 {
                    match cur_last {
                        CMD_CONT => {
                            g_st().debug_break_level = -1;
                        }
                        CMD_NEXT => {
                            g_st().debug_break_level = g_st().ex_nesting_level;
                        }
                        CMD_STEP => {
                            g_st().debug_break_level = 9999;
                        }
                        CMD_FINISH => {
                            g_st().debug_break_level = g_st().ex_nesting_level - 1;
                        }
                        CMD_QUIT => {
                            g_st().got_int = true;
                            g_st().debug_break_level = -1;
                        }
                        CMD_INTERRUPT => {
                            g_st().got_int = true;
                            g_st().debug_break_level = 9999;
                            dbg_st().LAST_CMD = CMD_STEP;
                        }
                        CMD_BACKTRACE => {
                            do_showbacktrace(cmd);
                            continue;
                        }
                        CMD_FRAME => {
                            if *p == 0 {
                                do_showbacktrace(cmd);
                            } else {
                                p = skipwhite(p);
                                do_setdebugtracelevel(p);
                            }
                            continue;
                        }
                        CMD_UP => {
                            g_st().debug_backtrace_level += 1;
                            do_checkbacktracelevel();
                            continue;
                        }
                        CMD_DOWN => {
                            g_st().debug_backtrace_level -= 1;
                            do_checkbacktracelevel();
                            continue;
                        }
                        _ => {}
                    }
                    g_st().debug_backtrace_level = 0;
                    break;
                }

                let n2 = g_st().debug_break_level;
                g_st().debug_break_level = -1;
                do_cmdline(
                    cmdline,
                    crate::cmdexpand::LineGetter::ExLine,
                    DOCMD_VERBOSE | DOCMD_EXCRESET,
                );
                g_st().debug_break_level = n2;
            }
            g_st().lines_left = g_st().Rows - 1;
        }

        g_st().RedrawingDisabled -= 1;
        g_st().no_wait_return -= 1;
        redraw_all_later(UPD_NOT_VALID);
        g_st().need_wait_return = false;
        g_st().msg_scroll = save_msg_scroll;
        g_st().lines_left = g_st().Rows - 1;
        g_st().State = save_state;
        g_st().debug_mode = false;
        g_st().did_emsg = save_did_emsg;
        g_st().cmd_silent = save_cmd_silent;
        g_st().msg_silent = save_msg_silent;
        g_st().emsg_silent = save_emsg_silent;
        g_st().redir_off = save_redir_off;

        g_st().debug_did_msg = true;
    }
}

/// `:debug` command (`ex_debug`).
pub fn ex_debug(eap: &mut exarg_T) {
    let save = g_st().debug_break_level;
        g_st().debug_break_level = 9999;
        do_cmdline_cmd(eap.arg);
        g_st().debug_break_level = save;
}

/// Check if a breakpoint or debug break level was hit (`dbg_check_breakpoint`).
pub fn dbg_check_breakpoint(eap: &mut exarg_T) {
    unsafe {
        dbg_st().DEBUG_SKIPPED = false;
        if !dbg_st().DEBUG_BREAKPOINT_NAME.is_null() {
            if eap.skip == 0 {
                let bp_name = dbg_st().DEBUG_BREAKPOINT_NAME;
                let is_snr = (*bp_name as u8) == K_SPECIAL
                    && (*bp_name.add(1) as u8) == KS_EXTRA
                    && (*bp_name.add(2) as u8) == KE_SNR;
                let (prefix, rest) = if is_snr {
                    (c"<SNR>".as_ptr(), bp_name.add(3))
                } else {
                    (c"".as_ptr(), bp_name)
                };
                let lnum = dbg_st().DEBUG_BREAKPOINT_LNUM;
                let fmt_str = format!("Breakpoint in \"%s%s\" line {lnum}\0");
                smsg!(0, fmt_str.as_ptr().cast::<c_char>(), prefix, rest);
                dbg_st().DEBUG_BREAKPOINT_NAME = std::ptr::null_mut();
                do_debug(eap.cmd);
            } else {
                dbg_st().DEBUG_SKIPPED = true;
                dbg_st().DEBUG_SKIPPED_NAME = dbg_st().DEBUG_BREAKPOINT_NAME;
                dbg_st().DEBUG_BREAKPOINT_NAME = std::ptr::null_mut();
            }
        } else if g_st().ex_nesting_level <= g_st().debug_break_level {
            if eap.skip == 0 {
                do_debug(eap.cmd);
            } else {
                dbg_st().DEBUG_SKIPPED = true;
                dbg_st().DEBUG_SKIPPED_NAME = std::ptr::null_mut();
            }
        }
    }
}

/// Enter debug mode if previously skipped by `dbg_check_breakpoint()` (`dbg_check_skipped`).
pub fn dbg_check_skipped(eap: &mut exarg_T) -> bool {
    if !dbg_st().DEBUG_SKIPPED {
            return false;
        }
        let prev_got_int = g_st().got_int;
        g_st().got_int = false;
        dbg_st().DEBUG_BREAKPOINT_NAME = dbg_st().DEBUG_SKIPPED_NAME;
        eap.skip = 0;
        dbg_check_breakpoint(eap);
        eap.skip = 1;
        g_st().got_int |= prev_got_int;
        true
}

fn eval_expr_no_emsg(name: *mut c_char) -> Option<typval_T> {
    g_st().emsg_off += 1;
        let tv = eval_expr(name, std::ptr::null_mut());
        g_st().emsg_off -= 1;
        tv
}

fn dbg_parsearg(arg: *mut c_char, is_prof: bool) -> Option<debuggy> {
    unsafe {
        let mut p = arg;
        let mut here = false;
        let mut bp = debuggy {
            dbg_nr: 0,
            dbg_type: 0,
            dbg_name: std::ffi::CString::default(),
            dbg_prog: None,
            dbg_lnum: 0,
            dbg_forceit: 0,
            dbg_val: None,
            dbg_level: 0,
        };

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"func") {
            bp.dbg_type = DBG_FUNC;
        } else if p_bytes.starts_with(b"file") {
            bp.dbg_type = DBG_FILE;
        } else if !is_prof && p_bytes.starts_with(b"here") {
            if (*g_st().curbuf).b_ffname.is_null() {
                emsg(e_noname.as_ptr());
                return None;
            }
            bp.dbg_type = DBG_FILE;
            here = true;
        } else if !is_prof && p_bytes.starts_with(b"expr") {
            bp.dbg_type = DBG_EXPR;
        } else {
            semsg!(e_invarg2.as_ptr(), p);
            return None;
        }
        p = skipwhite(p.add(4));

        if here {
            bp.dbg_lnum = (**curwin.as_ptr()).w_cursor.lnum;
        } else if !is_prof && (*p as u8).is_ascii_digit() {
            bp.dbg_lnum = getdigits_int32(&mut p, true, 0);
            p = skipwhite(p);
        } else {
            bp.dbg_lnum = 0;
        }

        let rem_bytes = CStr::from_ptr(p).to_bytes();
        if (!here && rem_bytes.is_empty())
            || (here && !rem_bytes.is_empty())
            || (bp.dbg_type == DBG_FUNC && rem_bytes.windows(2).any(|w| w == b"()"))
        {
            semsg!(e_invarg2.as_ptr(), arg);
            return None;
        }

        if bp.dbg_type == DBG_FUNC {
            let name_src = if rem_bytes.starts_with(b"g:") {
                p.add(2)
            } else {
                p
            };
            bp.dbg_name = CStr::from_ptr(name_src).to_owned();
        } else if here {
            if (*g_st().curbuf).b_ffname.is_null() {
                return None;
            }
            bp.dbg_name = CStr::from_ptr((*g_st().curbuf).b_ffname).to_owned();
        } else if bp.dbg_type == DBG_EXPR {
            bp.dbg_name = CStr::from_ptr(p).to_owned();
            bp.dbg_val = eval_expr_no_emsg(bp.dbg_name.as_ptr().cast_mut());
        } else {
            let q = expand_env_save(p);
            let p_exp = expand_env_save(q.as_ptr().cast_mut());
            let raw_name = if *p_exp.as_ptr() as u8 != b'*' {
                fix_fname(p_exp.as_ptr())?
            } else {
                p_exp
            };
            bp.dbg_name = raw_name;
        }

        Some(bp)
    }
}

/// `:breakadd` (and `:profile` file/func) command (`ex_breakadd`).
pub fn ex_breakadd(eap: &mut exarg_T) {
    let eap = &*eap;
        let is_prof = eap.cmdidx == CMD_PROFILE;
        let gap = dbg_vec_mut(is_prof);

        let Some(mut bp) = dbg_parsearg(eap.arg, is_prof) else {
            return;
        };

        bp.dbg_forceit = eap.forceit;

        if bp.dbg_type != DBG_EXPR {
            let pat = file_pat_to_reg_pat(
                bp.dbg_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );
            if let Some(ref pat) = pat {
                bp.dbg_prog = vim_regcomp_box(pat.as_ptr(), RE_MAGIC + RE_STRING);
            }
            if pat.is_some() && bp.dbg_prog.is_some() {
                if bp.dbg_lnum == 0 {
                    bp.dbg_lnum = 1;
                }
                if eap.cmdidx != CMD_PROFILE {
                    dbg_st().LAST_BREAKP += 1;
                    bp.dbg_nr = dbg_st().LAST_BREAKP;
                    g_st().debug_tick += 1;
                }
                gap.push(bp);
            }
        } else {
            dbg_st().LAST_BREAKP += 1;
            bp.dbg_nr = dbg_st().LAST_BREAKP;
            gap.push(bp);
            g_st().debug_tick += 1;
            if !is_prof {
                dbg_st().HAS_EXPR_BREAKPOINT = true;
            }
        }
}

/// `:debuggreedy` command (`ex_debuggreedy`).
pub fn ex_debuggreedy(eap: &mut exarg_T) {
    let eap = &*eap;
    let greedy = eap.addr_count == 0 || eap.line2 != 0;
    dbg_st().DEBUG_GREEDY = greedy;
}

fn update_has_expr_breakpoint() {
    let bps = dbg_vec_mut(false);
        dbg_st().HAS_EXPR_BREAKPOINT = bps.iter().any(|bp| bp.dbg_type == DBG_EXPR);
}

/// `:breakdel` and `:profdel` command (`ex_breakdel`).
pub fn ex_breakdel(eap: &mut exarg_T) {
    unsafe {
        let eap = &*eap;
        let mut todel: isize = -1;
        let mut del_all = false;
        let mut best_lnum: i32 = 0;
        let is_prof = eap.cmdidx == CMD_PROFDEL;
        let gap = dbg_vec_mut(is_prof);

        let first_byte = *eap.arg as u8;
        if first_byte.is_ascii_digit() {
            let nr = crate::charset::parse_i64_prefix(std::ffi::CStr::from_ptr(eap.arg).to_bytes()) as c_int;
            if let Some(idx) = gap.iter().position(|bp| bp.dbg_nr == nr) {
                todel = idx as isize;
            }
        } else if first_byte == b'*' {
            todel = 0;
            del_all = true;
        } else {
            let Some(bp) = dbg_parsearg(eap.arg, is_prof) else {
                return;
            };
            for (i, bpi) in gap.iter().enumerate() {
                if bp.dbg_type == bpi.dbg_type
                    && bpi.dbg_name == bp.dbg_name
                    && (bp.dbg_lnum == bpi.dbg_lnum
                        || (bp.dbg_lnum == 0 && (best_lnum == 0 || bpi.dbg_lnum < best_lnum)))
                {
                    todel = i as isize;
                    best_lnum = bpi.dbg_lnum;
                }
            }
        }

        if todel < 0 || (todel as usize) >= gap.len() {
            semsg!(c"E161: Breakpoint not found: %s".as_ptr(), eap.arg);
            return;
        }

        let idx = todel as usize;
        while idx < gap.len() {
            let _ = gap.remove(idx);
            if eap.cmdidx == CMD_BREAKDEL {
                g_st().debug_tick += 1;
            }
            if !del_all {
                break;
            }
        }

        if !is_prof {
            update_has_expr_breakpoint();
        }
    }
}

/// `:breaklist` command (`ex_breaklist`).
pub fn ex_breaklist(_eap: &mut exarg_T) {
    let gap = dbg_vec_mut(false);
        if gap.is_empty() {
            msg(c"No breakpoints defined".as_ptr(), 0);
            return;
        }

        let mut namebuf = [0 as c_char; MAXPATHL];
        for bp in gap {
            if bp.dbg_type == DBG_FILE {
                home_replace(
                    std::ptr::null(),
                    bp.dbg_name.as_ptr(),
                    namebuf.as_mut_ptr(),
                    MAXPATHL,
                    true,
                );
            }
            if bp.dbg_type != DBG_EXPR {
                let kind = if bp.dbg_type == DBG_FUNC {
                    c"func".as_ptr()
                } else {
                    c"file".as_ptr()
                };
                let name = if bp.dbg_type == DBG_FUNC {
                    bp.dbg_name.as_ptr()
                } else {
                    namebuf.as_ptr()
                };
                let lnum = bp.dbg_lnum;
                let fmt_str = format!("%3d  %s %s  line {lnum}\0");
                smsg!(0, fmt_str.as_ptr().cast::<c_char>(), bp.dbg_nr, kind, name);
            } else {
                smsg!(0, c"%3d  expr %s".as_ptr(), bp.dbg_nr, bp.dbg_name.as_ptr());
            }
        }
}

fn debuggy_find(
    file: bool,
    fname: *mut c_char,
    after: i32,
    gap: &mut [debuggy],
    is_prof: bool,
    fp: *mut bool,
) -> i32 {
    unsafe {
        if gap.is_empty() {
            return 0;
        }

        let mut lnum: i32 = 0;
        let name_owned: Option<CString>;
        let name = if !file && (*fname as u8) == K_SPECIAL {
            let rest_bytes = CStr::from_ptr(fname.add(3)).to_bytes();
            let mut bytes = Vec::with_capacity(5 + rest_bytes.len());
            bytes.extend_from_slice(b"<SNR>");
            bytes.extend_from_slice(rest_bytes);
            name_owned = Some(CString::new(bytes).unwrap_or_default());
            name_owned.as_ref().unwrap().as_ptr().cast_mut()
        } else {
            fname
        };

        for bp in gap.iter_mut() {
            if ((bp.dbg_type == DBG_FILE) == file)
                && bp.dbg_type != DBG_EXPR
                && (is_prof || (bp.dbg_lnum > after && (lnum == 0 || bp.dbg_lnum < lnum)))
            {
                let prev_got_int = g_st().got_int;
                g_st().got_int = false;
                if vim_regexec_prog(bp.dbg_prog.as_deref(), false, name, 0) {
                    lnum = bp.dbg_lnum;
                    if !fp.is_null() {
                        *fp = bp.dbg_forceit != 0;
                    }
                }
                g_st().got_int |= prev_got_int;
            } else if bp.dbg_type == DBG_EXPR {
                let mut line = false;
                if let Some(mut tv) = eval_expr_no_emsg(bp.dbg_name.as_ptr().cast_mut()) {
                    if let Some(ref mut old_val) = bp.dbg_val {
                        if typval_compare(&mut tv, &mut *old_val, EXPR_IS, false) == OK
                            && tv.vval.v_number() == 0
                        {
                            line = true;
                            dbg_st().DEBUG_OLDVAL = Some(typval_tostring(Some(old_val), true));
                            let mut v = eval_expr_no_emsg(bp.dbg_name.as_ptr().cast_mut());
                            dbg_st().DEBUG_NEWVAL = Some(typval_tostring(v.as_mut(), true));
                            tv_clear(old_val);
                            bp.dbg_val = v;
                        }
                        tv_clear(&raw mut tv);
                    } else {
                        dbg_st().DEBUG_OLDVAL = Some(typval_tostring(None, true));
                        bp.dbg_val = Some(tv);
                        dbg_st().DEBUG_NEWVAL = Some(typval_tostring(bp.dbg_val.as_mut(), true));
                        line = true;
                    }
                } else if let Some(ref mut old_val) = bp.dbg_val {
                    dbg_st().DEBUG_OLDVAL = Some(typval_tostring(Some(old_val), true));
                    dbg_st().DEBUG_NEWVAL = Some(typval_tostring(None, true));
                    tv_clear(old_val);
                    bp.dbg_val = None;
                    line = true;
                }

                if line {
                    lnum = if after > 0 { after } else { 1 };
                    break;
                }
            }
        }

        lnum
    }
}

/// Find a breakpoint for a function or sourced file (`dbg_find_breakpoint`).
pub fn dbg_find_breakpoint(file: bool, fname: *mut c_char, after: i32) -> i32 {
    let gap = dbg_vec_mut(false);
        debuggy_find(file, fname, after, gap, false, std::ptr::null_mut())
}

/// Check if profiling is enabled for a function or sourced file (`has_profiling`).
pub fn has_profiling(file: bool, fname: *mut c_char, fp: *mut bool) -> bool {
    let gap = dbg_vec_mut(true);
        debuggy_find(file, fname, 0, gap, true, fp) != 0
}

/// Record that a breakpoint was encountered (`dbg_breakpoint`).
pub fn dbg_breakpoint(name: *mut c_char, lnum: i32) {
    dbg_st().DEBUG_BREAKPOINT_NAME = name;
    dbg_st().DEBUG_BREAKPOINT_LNUM = lnum;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_maxbacktrace_level_counts_dot_dot_separators() {
        assert_eq!(get_maxbacktrace_level(std::ptr::null()), 0);
            assert_eq!(get_maxbacktrace_level(c"script.vim".as_ptr()), 0);
            assert_eq!(
                get_maxbacktrace_level(c"script.vim..FuncA..FuncB".as_ptr()),
                2
            );
    }
}
