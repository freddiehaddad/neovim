#![forbid(unsafe_code)]

//! Vimscript debugger (`src/nvim/debugger.c`).

use crate::charset::{getdigits_int32, skipwhite_slice};
use crate::cmdexpand::Cmdidx;
use crate::drawscreen::{UPD_NOT_VALID, redraw_all_later};
use crate::errors::{E_INVARG2, E_NONAME};
use crate::eval::typval::{Callback, TypVal, tv_clear};
use crate::eval::{EXPR_IS, eval_expr, typval_compare, typval_tostring};
use crate::ex_docmd::{DOCMD_EXCRESET, Exarg, do_cmdline, do_cmdline_cmd};
use crate::ex_getln::getcmdline_prompt;
use crate::fileio::file_pat_to_reg_pat;
use crate::globals::*;
use crate::input::{Tasave, restore_typeahead, save_typeahead};

use crate::message::{emsg, msg, msg_starthere, semsg, smsg};
use crate::os::env::{expand_env_save, home_replace};
use crate::path::fix_fname;
use crate::regexp::{RE_MAGIC, RE_STRING, vim_regcomp_box, vim_regexec_prog};
use crate::runtime::{ESTACK_NONE, estack_sfile, sourcing_lnum};
use std::ffi::{CStr, CString};

const DBG_FUNC: i32 = 1;
const DBG_FILE: i32 = 2;
const DBG_EXPR: i32 = 3;

const CMD_CONT: i32 = 1;
const CMD_NEXT: i32 = 2;
const CMD_STEP: i32 = 3;
const CMD_FINISH: i32 = 4;
const CMD_QUIT: i32 = 5;
const CMD_INTERRUPT: i32 = 6;
const CMD_BACKTRACE: i32 = 7;
const CMD_FRAME: i32 = 8;
const CMD_UP: i32 = 9;
const CMD_DOWN: i32 = 10;

struct DebuggerState {
    debug_greedy: bool,
    last_cmd: i32,
    last_breakp: i32,
    has_expr_breakpoint: bool,
    debug_skipped: bool,
    debug_breakpoint_lnum: i32,
    debug_oldval: Option<CString>,
    debug_newval: Option<CString>,
    debug_breakpoint_name: Option<CString>,
    debug_skipped_name: Option<CString>,
    dbg_breakp: Vec<Debuggy>,
    prof_ga: Vec<Debuggy>,
}

impl DebuggerState {
    const fn new() -> Self {
        Self {
            debug_greedy: false,
            last_cmd: 0,
            last_breakp: 0,
            has_expr_breakpoint: false,
            debug_skipped: false,
            debug_breakpoint_lnum: 0,
            debug_oldval: None,
            debug_newval: None,
            debug_breakpoint_name: None,
            debug_skipped_name: None,
            dbg_breakp: Vec::new(),
            prof_ga: Vec::new(),
        }
    }
}

static DBG_STATE: GlobalState<DebuggerState> = GlobalState::new(DebuggerState::new());

#[inline]
fn dbg_st() -> crate::globals::GraphPtr<DebuggerState> {
    DBG_STATE.handle()
}

#[inline]
fn dbg_vec_mut(is_prof: bool) -> &'static mut Vec<Debuggy> {
    if is_prof {
        &mut dbg_st().borrow_mut().prof_ga
    } else {
        &mut dbg_st().borrow_mut().dbg_breakp
    }
}

#[repr(C)]
pub struct Debuggy {
    pub dbg_nr: i32,
    pub dbg_type: i32,
    pub dbg_name: std::ffi::CString,
    pub dbg_prog: Option<Box<crate::regexp::Regprog>>,
    pub dbg_lnum: i32,
    pub dbg_forceit: bool,
    pub dbg_val: Option<TypVal>,
    pub dbg_level: i32,
}

impl Drop for Debuggy {
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

fn get_maxbacktrace_level(sname: Option<&CStr>) -> i32 {
    let Some(sname) = sname else {
        return 0;
    };
    let bytes = sname.to_bytes();
    let mut count: i32 = 0;
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
        msg(c"frame is zero", 0);
    } else {
        let sname = estack_sfile(ESTACK_NONE);
        let max = get_maxbacktrace_level(sname.as_deref());
        if g_st().debug_backtrace_level > max {
            g_st().debug_backtrace_level = max;
            smsg!(0, c"frame at highest level: %d", max);
        }
    }
}

fn do_setdebugtracelevel(arg: &[u8]) {
    let level = crate::charset::parse_i64_prefix(arg) as i32;
    if arg.first() == Some(&b'+') || level < 0 {
        g_st().debug_backtrace_level += level;
    } else {
        g_st().debug_backtrace_level = level;
    }
    do_checkbacktracelevel();
}

fn do_showbacktrace(cmd: &CStr) {
    let sname_cstr = estack_sfile(ESTACK_NONE);
    let max = get_maxbacktrace_level(sname_cstr.as_deref());
    if let Some(sname_cstr) = sname_cstr {
        let mut sname_vec: Vec<u8> = sname_cstr.into_bytes_with_nul();
        let mut i: i32 = 0;
        let mut start = 0usize;
        while !g_st().got_int {
            let rem = &sname_vec[start..];
            let str_len = rem.iter().position(|&b| b == 0).unwrap_or(rem.len());
            let next_off = rem[..str_len].windows(2).position(|w| w == b"..");
            if let Some(off) = next_off {
                sname_vec[start + off] = 0;
            }
            let cur_cstr = CStr::from_bytes_until_nul(&sname_vec[start..]).unwrap_or(c"");
            if i == max - g_st().debug_backtrace_level {
                smsg!(0, c"->%d %s", max - i, cur_cstr);
            } else {
                smsg!(0, c"  %d %s", max - i, cur_cstr);
            }
            i += 1;
            if let Some(off) = next_off {
                sname_vec[start + off] = b'.';
                start += off + 2;
            } else {
                break;
            }
        }
    }

    let slnum = sourcing_lnum();
    if slnum != 0 {
        smsg!(0, c"line %ld: %s", i64::from(slnum), cmd);
    } else {
        smsg!(0, c"cmd: %s", cmd);
    }
}

/// Interactive debug mode loop (`do_debug`).
pub fn do_debug(cmd: &CStr) {
    {
        let save_msg_scroll = g_st().msg_scroll;
        let save_state = g_st().state_global;
        let save_did_emsg = g_st().did_emsg;
        let save_cmd_silent = g_st().cmd_silent;
        let save_msg_silent = g_st().msg_silent;
        let save_emsg_silent = g_st().emsg_silent;
        let save_redir_off = g_st().redir_off;
        let mut typeaheadbuf = Tasave::new();
        let mut typeahead_saved = false;
        let mut save_ignore_script = false;
        let mut cmdline_cstr: Option<CString>;

        g_st().redrawing_disabled += 1;
        g_st().no_wait_return += 1;
        g_st().did_emsg = 0;
        g_st().cmd_silent = false;
        g_st().msg_silent = 0;
        g_st().emsg_silent = 0;
        g_st().redir_off = true;

        g_st().state_global = MODE_NORMAL;
        g_st().debug_mode = true;

        if !g_st().debug_did_msg {
            msg(c"Entering Debug mode.  Type \"cont\" to continue.", 0);
        }
        if let Some(oldval) = dbg_st().debug_oldval.take() {
            smsg!(0, c"Oldval = \"%s\"", &oldval);
        }
        if let Some(newval) = dbg_st().debug_newval.take() {
            smsg!(0, c"Newval = \"%s\"", &newval);
        }
        if let Some(sname) = estack_sfile(ESTACK_NONE) {
            msg(&sname, 0);
        }
        let slnum = sourcing_lnum();
        if slnum != 0 {
            smsg!(0, c"line %ld: %s", i64::from(slnum), cmd);
        } else {
            smsg!(0, c"cmd: %s", cmd);
        }

        loop {
            g_st().msg_scroll = 1;
            g_st().need_wait_return = false;

            let save_ex_normal_busy = g_st().ex_normal_busy;
            g_st().ex_normal_busy = 0;
            if !dbg_st().debug_greedy {
                save_typeahead(&mut typeaheadbuf);
                typeahead_saved = true;
                save_ignore_script = g_st().ignore_script;
                g_st().ignore_script = true;
            }

            let n = g_st().debug_break_level;
            g_st().debug_break_level = -1;

            cmdline_cstr = getcmdline_prompt(
                b'>' as i32,
                None,
                0,
                EXPAND_NOTHING,
                None,
                Callback::NONE,
                false,
                None,
            );

            g_st().debug_break_level = n;
            if typeahead_saved {
                restore_typeahead(&mut typeaheadbuf);
                g_st().ignore_script = save_ignore_script;
            }
            g_st().ex_normal_busy = save_ex_normal_busy;

            g_st().cmdline_row = g_st().msg_row;
            msg_starthere();
            if let Some(cmdline_cstr) = cmdline_cstr.as_deref() {
                let mut p_cstr = crate::charset::skipwhite(cmdline_cstr);
                let p_bytes = p_cstr.to_bytes();
                if let Some(&b0) = p_bytes.first() {
                    let b1 = p_bytes.get(1).copied().unwrap_or(0);
                    let (mut lcmd, mut tail): (i32, &[u8]) = match b0 {
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
                        p_cstr = crate::memory::CStrSubslice::advance(p_cstr, 1);
                        let mut p_bytes = p_cstr.to_bytes();
                        while !p_bytes.is_empty() && !tail.is_empty() && p_bytes[0] == tail[0] {
                            p_cstr = crate::memory::CStrSubslice::advance(p_cstr, 1);
                            p_bytes = &p_bytes[1..];
                            tail = &tail[1..];
                        }
                        if p_bytes.first().is_some_and(u8::is_ascii_alphabetic) && lcmd != CMD_FRAME
                        {
                            lcmd = 0;
                        }
                    }
                    dbg_st().last_cmd = lcmd;
                }

                let cur_last = dbg_st().last_cmd;
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
                            dbg_st().last_cmd = CMD_STEP;
                        }
                        CMD_BACKTRACE => {
                            do_showbacktrace(cmd);
                            continue;
                        }
                        CMD_FRAME => {
                            let p_rem = crate::charset::skipwhite_slice(p_cstr.to_bytes());
                            if p_rem.is_empty() {
                                do_showbacktrace(cmd);
                            } else {
                                do_setdebugtracelevel(p_rem);
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
                    Some(cmdline_cstr),
                    crate::cmdexpand::LineGetter::ExLine,
                    DOCMD_VERBOSE | DOCMD_EXCRESET,
                );
                g_st().debug_break_level = n2;
            }
            g_st().lines_left = g_st().rows_global - 1;
        }

        g_st().redrawing_disabled -= 1;
        g_st().no_wait_return -= 1;
        redraw_all_later(UPD_NOT_VALID);
        g_st().need_wait_return = false;
        g_st().msg_scroll = save_msg_scroll;
        g_st().lines_left = g_st().rows_global - 1;
        g_st().state_global = save_state;
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
pub fn ex_debug(eap: &mut Exarg) {
    let save = g_st().debug_break_level;
    g_st().debug_break_level = 9999;
    do_cmdline_cmd(eap.arg.as_cstr());
    g_st().debug_break_level = save;
}

/// Check if a breakpoint or debug break level was hit (`dbg_check_breakpoint`).
pub fn dbg_check_breakpoint(eap: &mut Exarg) {
    dbg_st().debug_skipped = false;
    if let Some(bp_name_cstr) = dbg_st().debug_breakpoint_name.take() {
        if !eap.skip {
            let bp_bytes = bp_name_cstr.to_bytes();
            let is_snr = bp_bytes.starts_with(&[K_SPECIAL, KS_EXTRA, KE_SNR]);
            let (prefix, rest) = if is_snr {
                (
                    c"<SNR>",
                    crate::memory::CStrSubslice::advance(bp_name_cstr.as_c_str(), 3),
                )
            } else {
                (c"", bp_name_cstr.as_c_str())
            };
            let lnum = dbg_st().debug_breakpoint_lnum;
            smsg!(
                0,
                c"Breakpoint in \"%s%s\" line %ld",
                prefix,
                rest,
                i64::from(lnum)
            );
            do_debug(eap.cmd.as_cstr());
        } else {
            dbg_st().debug_skipped = true;
            dbg_st().debug_skipped_name = Some(bp_name_cstr);
        }
    } else if g_st().ex_nesting_level <= g_st().debug_break_level {
        if !eap.skip {
            do_debug(eap.cmd.as_cstr());
        } else {
            dbg_st().debug_skipped = true;
            dbg_st().debug_skipped_name = None;
        }
    }
}

/// Enter debug mode if previously skipped by `dbg_check_breakpoint()` (`dbg_check_skipped`).
pub fn dbg_check_skipped(eap: &mut Exarg) -> bool {
    if !dbg_st().debug_skipped {
        return false;
    }
    let prev_got_int = g_st().got_int;
    g_st().got_int = false;
    dbg_st().debug_breakpoint_name = dbg_st().debug_skipped_name.take();
    eap.skip = false;
    dbg_check_breakpoint(eap);
    eap.skip = true;
    g_st().got_int |= prev_got_int;
    true
}

fn eval_expr_no_emsg(name: &CStr) -> Option<TypVal> {
    g_st().emsg_off += 1;
    let tv = eval_expr(name, None);
    g_st().emsg_off -= 1;
    tv
}

fn dbg_parsearg(arg: &CStr, is_prof: bool) -> Option<Debuggy> {
    let mut p = arg;
    {
        let mut here = false;
        let mut bp = Debuggy {
            dbg_nr: 0,
            dbg_type: 0,
            dbg_name: std::ffi::CString::default(),
            dbg_prog: None,
            dbg_lnum: 0,
            dbg_forceit: false,
            dbg_level: 0,
            dbg_val: None,
        };

        let p_bytes = p.to_bytes();
        if p_bytes.starts_with(b"func") {
            bp.dbg_type = DBG_FUNC;
        } else if p_bytes.starts_with(b"file") {
            bp.dbg_type = DBG_FILE;
        } else if !is_prof && p_bytes.starts_with(b"here") {
            if curbuf_mut().b_ffname.is_none() {
                emsg(E_NONAME);
                return None;
            }
            bp.dbg_type = DBG_FILE;
            here = true;
        } else if !is_prof && p_bytes.starts_with(b"expr") {
            bp.dbg_type = DBG_EXPR;
        } else {
            semsg!(E_INVARG2, p);
            return None;
        }
        p = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(p, 4));

        if here {
            bp.dbg_lnum = curwin_mut().w_cursor.lnum;
        } else if !is_prof && p.to_bytes().first().is_some_and(u8::is_ascii_digit) {
            let p_bytes = p.to_bytes();
            let mut rem = p_bytes;
            bp.dbg_lnum = getdigits_int32(&mut rem, true, 0);
            let rem = skipwhite_slice(rem);
            p = crate::memory::CStrSubslice::advance(p, p_bytes.len() - rem.len());
        } else {
            bp.dbg_lnum = 0;
        }

        let rem_bytes = p.to_bytes();
        if (!here && rem_bytes.is_empty())
            || (here && !rem_bytes.is_empty())
            || (bp.dbg_type == DBG_FUNC && rem_bytes.windows(2).any(|w| w == b"()"))
        {
            semsg!(E_INVARG2, arg);
            return None;
        }

        if bp.dbg_type == DBG_FUNC {
            let name_src = if rem_bytes.starts_with(b"g:") {
                crate::memory::CStrSubslice::advance(p, 2)
            } else {
                p
            };
            bp.dbg_name = name_src.to_owned();
        } else if here {
            bp.dbg_name = curbuf_mut().b_ffname.clone()?;
        } else if bp.dbg_type == DBG_EXPR {
            bp.dbg_name = p.to_owned();
            bp.dbg_val = eval_expr_no_emsg(&bp.dbg_name);
        } else {
            let q = expand_env_save(p);
            let p_exp = expand_env_save(&q);
            let raw_name = if p_exp.to_bytes().first().copied().unwrap_or(0) != b'*' {
                fix_fname(Some(&p_exp))?
            } else {
                p_exp
            };
            bp.dbg_name = raw_name;
        }

        Some(bp)
    }
}

/// `:breakadd` (and `:profile` file/func) command (`ex_breakadd`).
pub fn ex_breakadd(eap: &mut Exarg) {
    let is_prof = eap.cmdidx == Cmdidx::CmdProfile;
    let gap = dbg_vec_mut(is_prof);

    let Some(mut bp) = dbg_parsearg(eap.arg.as_cstr(), is_prof) else {
        return;
    };

    bp.dbg_forceit = eap.forceit;

    if bp.dbg_type != DBG_EXPR {
        let pat = file_pat_to_reg_pat(bp.dbg_name.as_bytes(), None, 0);
        if let Some(ref pat) = pat {
            bp.dbg_prog = vim_regcomp_box(pat, RE_MAGIC + RE_STRING);
        }
        if pat.is_some() && bp.dbg_prog.is_some() {
            if bp.dbg_lnum == 0 {
                bp.dbg_lnum = 1;
            }
            if eap.cmdidx != Cmdidx::CmdProfile {
                dbg_st().last_breakp += 1;
                bp.dbg_nr = dbg_st().last_breakp;
                g_st().debug_tick += 1;
            }
            gap.push(bp);
        }
    } else {
        dbg_st().last_breakp += 1;
        bp.dbg_nr = dbg_st().last_breakp;
        gap.push(bp);
        g_st().debug_tick += 1;
        if !is_prof {
            dbg_st().has_expr_breakpoint = true;
        }
    }
}

/// `:debuggreedy` command (`ex_debuggreedy`).
pub fn ex_debuggreedy(eap: &mut Exarg) {
    let greedy = eap.addr_count == 0 || eap.line2 != 0;
    dbg_st().debug_greedy = greedy;
}

fn update_has_expr_breakpoint() {
    let bps = dbg_vec_mut(false);
    dbg_st().has_expr_breakpoint = bps.iter().any(|bp| bp.dbg_type == DBG_EXPR);
}

/// `:breakdel` and `:profdel` command (`ex_breakdel`).
pub fn ex_breakdel(eap: &mut Exarg) {
    let mut todel: isize = -1;
    let mut del_all = false;
    let mut best_lnum: i32 = 0;
    let is_prof = eap.cmdidx == Cmdidx::CmdProfdel;
    let gap = dbg_vec_mut(is_prof);

    let first_byte = eap.arg.first_byte();
    if first_byte.is_ascii_digit() {
        let nr = crate::charset::parse_i64_prefix(eap.arg.as_bytes()) as i32;
        if let Some(idx) = gap.iter().position(|bp| bp.dbg_nr == nr) {
            todel = idx as isize;
        }
    } else if first_byte == b'*' {
        todel = 0;
        del_all = true;
    } else {
        let Some(bp) = dbg_parsearg(eap.arg.as_cstr(), is_prof) else {
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
        semsg!(c"E161: Breakpoint not found: %s", eap.arg);
        return;
    }

    let idx = todel as usize;
    while idx < gap.len() {
        let _ = gap.remove(idx);
        if eap.cmdidx == Cmdidx::CmdBreakdel {
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

/// `:breaklist` command (`ex_breaklist`).
pub fn ex_breaklist(_eap: &mut Exarg) {
    let gap = dbg_vec_mut(false);
    if gap.is_empty() {
        msg(c"No breakpoints defined", 0);
        return;
    }

    let mut namebuf = [0u8; MAXPATHL];
    for bp in gap {
        if bp.dbg_type == DBG_FILE {
            home_replace(None, Some(bp.dbg_name.as_c_str()), &mut namebuf, true);
        }
        if bp.dbg_type != DBG_EXPR {
            let kind = if bp.dbg_type == DBG_FUNC {
                c"func"
            } else {
                c"file"
            };
            let name = if bp.dbg_type == DBG_FUNC {
                bp.dbg_name.as_c_str()
            } else {
                crate::memory::bytes_as_cstr(&namebuf)
            };
            let lnum = bp.dbg_lnum;
            smsg!(
                0,
                c"%3d  %s %s  line %ld",
                bp.dbg_nr,
                kind,
                name,
                i64::from(lnum)
            );
        } else {
            smsg!(0, c"%3d  expr %s", bp.dbg_nr, &bp.dbg_name);
        }
    }
}

fn debuggy_find(
    file: bool,
    fname: &CStr,
    after: i32,
    gap: &mut [Debuggy],
    is_prof: bool,
    mut fp: Option<&mut bool>,
) -> i32 {
    if gap.is_empty() {
        return 0;
    }

    let mut lnum: i32 = 0;
    let name_owned: Option<CString>;
    let fname_bytes = fname.to_bytes();
    let name = if !file && fname_bytes.first() == Some(&K_SPECIAL) {
        let rest_bytes = fname_bytes.get(3..).unwrap_or(&[]);
        let mut bytes = Vec::with_capacity(5 + rest_bytes.len());
        bytes.extend_from_slice(b"<SNR>");
        bytes.extend_from_slice(rest_bytes);
        name_owned = Some(CString::new(bytes).unwrap_or_default());
        name_owned.as_deref().unwrap()
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
                if let Some(ref mut p) = fp {
                    **p = bp.dbg_forceit;
                }
            }
            g_st().got_int |= prev_got_int;
        } else if bp.dbg_type == DBG_EXPR {
            let mut line = false;
            if let Some(mut tv) = eval_expr_no_emsg(&bp.dbg_name) {
                if let Some(ref mut old_val) = bp.dbg_val {
                    if typval_compare(&mut tv, &mut *old_val, EXPR_IS, false)
                        && tv.vval.v_number() == 0
                    {
                        line = true;
                        dbg_st().debug_oldval = Some(typval_tostring(Some(old_val), true));
                        let mut v = eval_expr_no_emsg(&bp.dbg_name);
                        dbg_st().debug_newval = Some(typval_tostring(v.as_mut(), true));
                        tv_clear(old_val);
                        bp.dbg_val = v;
                    }
                    tv_clear(&mut tv);
                } else {
                    dbg_st().debug_oldval = Some(typval_tostring(None, true));
                    bp.dbg_val = Some(tv);
                    dbg_st().debug_newval = Some(typval_tostring(bp.dbg_val.as_mut(), true));
                    line = true;
                }
            } else if let Some(ref mut old_val) = bp.dbg_val {
                dbg_st().debug_oldval = Some(typval_tostring(Some(old_val), true));
                dbg_st().debug_newval = Some(typval_tostring(None, true));
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

/// Find a breakpoint for a function or sourced file (`dbg_find_breakpoint`).
pub fn dbg_find_breakpoint(file: bool, fname: &CStr, after: i32) -> i32 {
    let gap = dbg_vec_mut(false);
    debuggy_find(file, fname, after, gap, false, None)
}

/// Check if profiling is enabled for a function or sourced file (`has_profiling`).
pub fn has_profiling(file: bool, fname: &CStr, fp: Option<&mut bool>) -> bool {
    let gap = dbg_vec_mut(true);
    debuggy_find(file, fname, 0, gap, true, fp) != 0
}

/// Record that a breakpoint was encountered (`dbg_breakpoint`).
pub fn dbg_breakpoint(name: Option<&CStr>, lnum: i32) {
    dbg_st().debug_breakpoint_name = name.map(CStr::to_owned);
    dbg_st().debug_breakpoint_lnum = lnum;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_maxbacktrace_level_counts_dot_dot_separators() {
        assert_eq!(get_maxbacktrace_level(None), 0);
        assert_eq!(get_maxbacktrace_level(Some(c"script.vim")), 0);
        assert_eq!(get_maxbacktrace_level(Some(c"script.vim..FuncA..FuncB")), 2);
    }
}
