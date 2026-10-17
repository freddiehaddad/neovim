//! Vimscript debugger (`src/nvim/debugger.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::clipboard::typval_T;
use crate::cursor_shape::pos_T;
use crate::eval::deprecated::{Callback, memline_T};
use crate::garray::{ga_clear, ga_grow, garray_T};
use crate::map::Set;
use crate::math::{FAIL, OK};
use crate::memory::{xfree, xmalloc, xstrdup};
use crate::msgpack_rpc::packer::handle_T;
use crate::profile::estack_T;

const MODE_NORMAL: c_int = 0x01;
const ESTACK_NONE: c_int = 0;
const EXPAND_NOTHING: c_int = 0;
const DOCMD_VERBOSE: c_int = 0x01;
const DOCMD_EXCRESET: c_int = 0x10;
const UPD_NOT_VALID: c_int = 40;

const K_SPECIAL: u8 = 0x80;
const KS_EXTRA: u8 = 253;
const KE_SNR: u8 = 82;

const DBG_FUNC: c_int = 1;
const DBG_FILE: c_int = 2;
const DBG_EXPR: c_int = 3;

const CMD_BREAKDEL: c_int = 37;
const CMD_PROFILE: c_int = 337;
const CMD_PROFDEL: c_int = 338;

const RE_MAGIC: c_int = 1;
const RE_STRING: c_int = 2;
const EXPR_IS: c_int = 9;
const MAXPATHL: usize = 4096;

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

static DEBUG_GREEDY: AtomicBool = AtomicBool::new(false);
static LAST_CMD: AtomicI32 = AtomicI32::new(0);
static LAST_BREAKP: AtomicI32 = AtomicI32::new(0);
static HAS_EXPR_BREAKPOINT: AtomicBool = AtomicBool::new(false);
static DEBUG_SKIPPED: AtomicBool = AtomicBool::new(false);
static DEBUG_BREAKPOINT_LNUM: AtomicI32 = AtomicI32::new(0);

static mut DEBUG_OLDVAL: *mut c_char = std::ptr::null_mut();
static mut DEBUG_NEWVAL: *mut c_char = std::ptr::null_mut();
static mut DEBUG_BREAKPOINT_NAME: *mut c_char = std::ptr::null_mut();
static mut DEBUG_SKIPPED_NAME: *mut c_char = std::ptr::null_mut();

static mut DBG_BREAKP: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: std::mem::size_of::<debuggy>() as c_int,
    ga_growsize: 4,
    ga_data: std::ptr::null_mut(),
};

static mut PROF_GA: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: std::mem::size_of::<debuggy>() as c_int,
    ga_growsize: 4,
    ga_data: std::ptr::null_mut(),
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct debuggy {
    pub dbg_nr: c_int,
    pub dbg_type: c_int,
    pub dbg_name: *mut c_char,
    pub dbg_prog: *mut c_void,
    pub dbg_lnum: i32,
    pub dbg_forceit: c_int,
    pub dbg_val: *mut typval_T,
    pub dbg_level: c_int,
}

#[repr(C)]
pub struct exarg_Full {
    pub arg: *mut c_char,
    pub args: *mut *mut c_char,
    pub arglens: *mut usize,
    pub argc: usize,
    pub nextcmd: *mut c_char,
    pub cmd: *mut c_char,
    pub cmdlinep: *mut *mut c_char,
    pub cmdline_tofree: *mut c_char,
    pub cmdidx: c_int,
    pub argt: u32,
    pub skip: c_int,
    pub forceit: c_int,
    pub addr_count: c_int,
    pub line1: i32,
    pub line2: i32,
}

#[repr(C)]
pub struct buf_Prefix {
    pub b_fnum: handle_T,
    pub b_ml: memline_T,
    pub b_next: *mut c_void,
    pub b_prev: *mut c_void,
    pub b_nwindows: c_int,
    pub b_flags: c_int,
    pub b_locked: c_int,
    pub b_locked_split: c_int,
    pub b_ro_locked: c_int,
    pub b_ffname: *mut c_char,
}

#[repr(C)]
pub struct win_Prefix {
    pub handle: handle_T,
    pub w_buffer: *mut buf_Prefix,
    pub w_s: *mut c_void,
    pub w_ns_hl: c_int,
    pub w_ns_hl_winhl: c_int,
    pub w_ns_hl_active: c_int,
    pub w_ns_hl_attr: *mut c_int,
    pub w_ns_set: Set<u32>,
    pub w_hl_id_normal: c_int,
    pub w_hl_attr_normal: c_int,
    pub w_hl_attr_normalnc: c_int,
    pub w_hl_needs_update: c_int,
    pub w_prev: *mut c_void,
    pub w_next: *mut c_void,
    pub w_locked: c_int,
    pub w_frame: *mut c_void,
    pub w_cursor: pos_T,
}

#[repr(C)]
pub struct tasave_T {
    pub _opaque: [u64; 24],
}

pub type LineGetter = Option<unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>;

#[cfg(not(test))]
unsafe extern "C" {
    static mut msg_scroll: c_int;
    static mut State: c_int;
    static mut did_emsg: c_int;
    static mut cmd_silent: bool;
    static mut msg_silent: c_int;
    static mut emsg_silent: c_int;
    static mut redir_off: bool;
    static mut RedrawingDisabled: c_int;
    static mut no_wait_return: c_int;
    static mut need_wait_return: bool;
    static mut debug_mode: bool;
    static mut debug_did_msg: bool;
    static mut ex_normal_busy: c_int;
    static mut ignore_script: bool;
    static mut debug_break_level: c_int;
    static mut ex_nesting_level: c_int;
    static mut debug_backtrace_level: c_int;
    static mut debug_tick: c_int;
    static mut cmdline_row: c_int;
    static mut msg_row: c_int;
    static mut got_int: bool;
    static mut lines_left: c_int;
    static mut Rows: c_int;
    static mut emsg_off: c_int;
    static mut exestack: garray_T;
    static mut curbuf: *mut buf_Prefix;
    static mut curwin: *mut win_Prefix;

    fn estack_sfile(which: c_int) -> *mut c_char;
    fn save_typeahead(tp: *mut tasave_T);
    fn restore_typeahead(tp: *mut tasave_T);
    fn getcmdline_prompt(
        firstc: c_int,
        prompt: *const c_char,
        hl_id: c_int,
        xp_context: c_int,
        xp_arg: *const c_char,
        highlight_callback: Callback,
        one_key: bool,
        mouse_used: *mut bool,
    ) -> *mut c_char;
    fn getexline(c: c_int, cookie: *mut c_void, indent: c_int, do_concat: bool) -> *mut c_char;
    fn do_cmdline(
        cmdline: *mut c_char,
        fgetline: LineGetter,
        cookie: *mut c_void,
        flags: c_int,
    ) -> c_int;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn redraw_all_later(upd_type: c_int);

    fn eval_expr(arg: *mut c_char, eap: *mut c_void) -> *mut typval_T;
    fn tv_free(tv: *mut typval_T);
    fn typval_tostring(arg: *mut typval_T, quotes: bool) -> *mut c_char;
    fn typval_compare(typ1: *mut typval_T, typ2: *mut typval_T, etype: c_int, ic: bool) -> c_int;

    fn getdigits_int32(pp: *mut *mut c_char, strict: bool, def: i32) -> i32;
    fn expand_env_save(src: *mut c_char) -> *mut c_char;
    fn fix_fname(fname: *const c_char) -> *mut c_char;
    fn file_pat_to_reg_pat(
        pat: *const c_char,
        pat_end: *const c_char,
        allow_dirs: *mut c_char,
        no_bslash: c_int,
    ) -> *mut c_char;
    fn vim_regcomp(expr_arg: *const c_char, re_flags: c_int) -> *mut c_void;
    fn vim_regfree(prog: *mut c_void);
    fn vim_regexec_prog(
        prog: *mut *mut c_void,
        ignore_case: bool,
        line: *const c_char,
        col: c_int,
    ) -> bool;
    fn home_replace(
        buf: *const c_void,
        src: *const c_char,
        dst: *mut c_char,
        dstlen: usize,
        one: bool,
    ) -> usize;

    fn msg(s: *const c_char, hl_id: c_int) -> bool;
    fn msg_starthere();
    fn smsg(hl_id: c_int, s: *const c_char, ...) -> c_int;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
}

#[inline]
unsafe fn skipwhite(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        while matches!(*p as u8, b' ' | b'\t') {
            p = p.add(1);
        }
        p
    }
}

#[inline]
fn atoi_cstr(s: *const c_char) -> c_int {
    let text = unsafe { CStr::from_ptr(s).to_string_lossy() };
    let trimmed = text.trim_start();
    let mut sign: c_int = 1;
    let digits = if let Some(rest) = trimmed.strip_prefix('-') {
        sign = -1;
        rest
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        rest
    } else {
        trimmed
    };
    let mut val: c_int = 0;
    for b in digits.bytes() {
        if b.is_ascii_digit() {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as c_int);
        } else {
            break;
        }
    }
    val.wrapping_mul(sign)
}

#[inline]
unsafe fn debuggy_at(gap: *mut garray_T, idx: c_int) -> *mut debuggy {
    unsafe { (*gap).ga_data.cast::<debuggy>().add(idx as usize) }
}

#[cfg(not(test))]
#[inline]
unsafe fn sourcing_lnum() -> i32 {
    unsafe {
        if exestack.ga_data.is_null() || exestack.ga_len <= 0 {
            0
        } else {
            (*exestack
                .ga_data
                .cast::<estack_T>()
                .add((exestack.ga_len - 1) as usize))
            .es_lnum
        }
    }
}

unsafe fn get_maxbacktrace_level(sname: *const c_char) -> c_int {
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

#[cfg(not(test))]
unsafe fn do_checkbacktracelevel() {
    unsafe {
        if debug_backtrace_level < 0 {
            debug_backtrace_level = 0;
            msg(c"frame is zero".as_ptr(), 0);
        } else {
            let sname = estack_sfile(ESTACK_NONE);
            let max = get_maxbacktrace_level(sname);
            if debug_backtrace_level > max {
                debug_backtrace_level = max;
                smsg(0, c"frame at highest level: %d".as_ptr(), max);
            }
            xfree(sname.cast::<c_void>());
        }
    }
}

#[cfg(not(test))]
unsafe fn do_setdebugtracelevel(arg: *mut c_char) {
    unsafe {
        let level = atoi_cstr(arg);
        if *arg as u8 == b'+' || level < 0 {
            debug_backtrace_level += level;
        } else {
            debug_backtrace_level = level;
        }
        do_checkbacktracelevel();
    }
}

#[cfg(not(test))]
unsafe fn do_showbacktrace(cmd: *const c_char) {
    unsafe {
        let sname = estack_sfile(ESTACK_NONE);
        let max = get_maxbacktrace_level(sname);
        if !sname.is_null() {
            let mut i: c_int = 0;
            let mut cur = sname;
            while !got_int {
                let cur_bytes = CStr::from_ptr(cur).to_bytes();
                let next_off = cur_bytes.windows(2).position(|w| w == b"..");
                if let Some(off) = next_off {
                    *cur.add(off) = 0;
                }
                if i == max - debug_backtrace_level {
                    smsg(0, c"->%d %s".as_ptr(), max - i, cur);
                } else {
                    smsg(0, c"  %d %s".as_ptr(), max - i, cur);
                }
                i += 1;
                if let Some(off) = next_off {
                    *cur.add(off) = b'.' as c_char;
                    cur = cur.add(off + 2);
                } else {
                    break;
                }
            }
            xfree(sname.cast::<c_void>());
        }

        let slnum = sourcing_lnum();
        if slnum != 0 {
            let line_msg = format!("line {slnum}: %s\0");
            smsg(0, line_msg.as_ptr().cast::<c_char>(), cmd);
        } else {
            smsg(0, c"cmd: %s".as_ptr(), cmd);
        }
    }
}

/// Interactive debug mode loop (`do_debug`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_debug(cmd: *mut c_char) {
    #[cfg(test)]
    let _ = cmd;

    #[cfg(not(test))]
    unsafe {
        let save_msg_scroll = msg_scroll;
        let save_state = State;
        let save_did_emsg = did_emsg;
        let save_cmd_silent = cmd_silent;
        let save_msg_silent = msg_silent;
        let save_emsg_silent = emsg_silent;
        let save_redir_off = redir_off;
        let mut typeaheadbuf = tasave_T { _opaque: [0; 24] };
        let mut typeahead_saved = false;
        let mut save_ignore_script = false;
        let mut cmdline: *mut c_char = std::ptr::null_mut();

        RedrawingDisabled += 1;
        no_wait_return += 1;
        did_emsg = 0;
        cmd_silent = false;
        msg_silent = 0;
        emsg_silent = 0;
        redir_off = true;

        State = MODE_NORMAL;
        debug_mode = true;

        if !debug_did_msg {
            msg(
                c"Entering Debug mode.  Type \"cont\" to continue.".as_ptr(),
                0,
            );
        }
        if !DEBUG_OLDVAL.is_null() {
            smsg(0, c"Oldval = \"%s\"".as_ptr(), DEBUG_OLDVAL);
            xfree(DEBUG_OLDVAL.cast::<c_void>());
            DEBUG_OLDVAL = std::ptr::null_mut();
        }
        if !DEBUG_NEWVAL.is_null() {
            smsg(0, c"Newval = \"%s\"".as_ptr(), DEBUG_NEWVAL);
            xfree(DEBUG_NEWVAL.cast::<c_void>());
            DEBUG_NEWVAL = std::ptr::null_mut();
        }
        let sname = estack_sfile(ESTACK_NONE);
        if !sname.is_null() {
            msg(sname, 0);
        }
        xfree(sname.cast::<c_void>());
        let slnum = sourcing_lnum();
        if slnum != 0 {
            let line_msg = format!("line {slnum}: %s\0");
            smsg(0, line_msg.as_ptr().cast::<c_char>(), cmd);
        } else {
            smsg(0, c"cmd: %s".as_ptr(), cmd);
        }

        loop {
            msg_scroll = 1;
            need_wait_return = false;

            let save_ex_normal_busy = ex_normal_busy;
            ex_normal_busy = 0;
            if !DEBUG_GREEDY.load(Ordering::Relaxed) {
                save_typeahead(&mut typeaheadbuf);
                typeahead_saved = true;
                save_ignore_script = ignore_script;
                ignore_script = true;
            }

            let n = debug_break_level;
            debug_break_level = -1;

            xfree(cmdline.cast::<c_void>());
            cmdline = getcmdline_prompt(
                b'>' as c_int,
                std::ptr::null(),
                0,
                EXPAND_NOTHING,
                std::ptr::null(),
                Callback {
                    data: std::ptr::null_mut(),
                    cb_type: 0,
                },
                false,
                std::ptr::null_mut(),
            );

            debug_break_level = n;
            if typeahead_saved {
                restore_typeahead(&mut typeaheadbuf);
                ignore_script = save_ignore_script;
            }
            ex_normal_busy = save_ex_normal_busy;

            cmdline_row = msg_row;
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
                    LAST_CMD.store(lcmd, Ordering::Relaxed);
                }

                let cur_last = LAST_CMD.load(Ordering::Relaxed);
                if cur_last != 0 {
                    match cur_last {
                        CMD_CONT => {
                            debug_break_level = -1;
                        }
                        CMD_NEXT => {
                            debug_break_level = ex_nesting_level;
                        }
                        CMD_STEP => {
                            debug_break_level = 9999;
                        }
                        CMD_FINISH => {
                            debug_break_level = ex_nesting_level - 1;
                        }
                        CMD_QUIT => {
                            got_int = true;
                            debug_break_level = -1;
                        }
                        CMD_INTERRUPT => {
                            got_int = true;
                            debug_break_level = 9999;
                            LAST_CMD.store(CMD_STEP, Ordering::Relaxed);
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
                            debug_backtrace_level += 1;
                            do_checkbacktracelevel();
                            continue;
                        }
                        CMD_DOWN => {
                            debug_backtrace_level -= 1;
                            do_checkbacktracelevel();
                            continue;
                        }
                        _ => {}
                    }
                    debug_backtrace_level = 0;
                    break;
                }

                let n2 = debug_break_level;
                debug_break_level = -1;
                do_cmdline(
                    cmdline,
                    Some(getexline),
                    std::ptr::null_mut(),
                    DOCMD_VERBOSE | DOCMD_EXCRESET,
                );
                debug_break_level = n2;
            }
            lines_left = Rows - 1;
        }
        xfree(cmdline.cast::<c_void>());

        RedrawingDisabled -= 1;
        no_wait_return -= 1;
        redraw_all_later(UPD_NOT_VALID);
        need_wait_return = false;
        msg_scroll = save_msg_scroll;
        lines_left = Rows - 1;
        State = save_state;
        debug_mode = false;
        did_emsg = save_did_emsg;
        cmd_silent = save_cmd_silent;
        msg_silent = save_msg_silent;
        emsg_silent = save_emsg_silent;
        redir_off = save_redir_off;

        debug_did_msg = true;
    }
}

/// `:debug` command (`ex_debug`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_debug(eap: *mut exarg_Full) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        let save = debug_break_level;
        debug_break_level = 9999;
        do_cmdline_cmd((*eap).arg);
        debug_break_level = save;
    }
}

/// Check if a breakpoint or debug break level was hit (`dbg_check_breakpoint`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dbg_check_breakpoint(eap: *mut exarg_Full) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        DEBUG_SKIPPED.store(false, Ordering::Relaxed);
        let eap = &mut *eap;
        if !DEBUG_BREAKPOINT_NAME.is_null() {
            if eap.skip == 0 {
                let bp_name = DEBUG_BREAKPOINT_NAME;
                let is_snr = (*bp_name as u8) == K_SPECIAL
                    && (*bp_name.add(1) as u8) == KS_EXTRA
                    && (*bp_name.add(2) as u8) == KE_SNR;
                let (prefix, rest) = if is_snr {
                    (c"<SNR>".as_ptr(), bp_name.add(3))
                } else {
                    (c"".as_ptr(), bp_name)
                };
                let lnum = DEBUG_BREAKPOINT_LNUM.load(Ordering::Relaxed);
                let fmt_str = format!("Breakpoint in \"%s%s\" line {lnum}\0");
                smsg(0, fmt_str.as_ptr().cast::<c_char>(), prefix, rest);
                DEBUG_BREAKPOINT_NAME = std::ptr::null_mut();
                do_debug(eap.cmd);
            } else {
                DEBUG_SKIPPED.store(true, Ordering::Relaxed);
                DEBUG_SKIPPED_NAME = DEBUG_BREAKPOINT_NAME;
                DEBUG_BREAKPOINT_NAME = std::ptr::null_mut();
            }
        } else if ex_nesting_level <= debug_break_level {
            if eap.skip == 0 {
                do_debug(eap.cmd);
            } else {
                DEBUG_SKIPPED.store(true, Ordering::Relaxed);
                DEBUG_SKIPPED_NAME = std::ptr::null_mut();
            }
        }
    }
}

/// Enter debug mode if previously skipped by `dbg_check_breakpoint()` (`dbg_check_skipped`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dbg_check_skipped(eap: *mut exarg_Full) -> bool {
    #[cfg(test)]
    {
        let _ = eap;
        false
    }
    #[cfg(not(test))]
    unsafe {
        if !DEBUG_SKIPPED.load(Ordering::Relaxed) {
            return false;
        }
        let prev_got_int = got_int;
        got_int = false;
        DEBUG_BREAKPOINT_NAME = DEBUG_SKIPPED_NAME;
        (*eap).skip = 0;
        dbg_check_breakpoint(eap);
        (*eap).skip = 1;
        got_int |= prev_got_int;
        true
    }
}

#[cfg(not(test))]
unsafe fn eval_expr_no_emsg(bp: &debuggy) -> *mut typval_T {
    unsafe {
        emsg_off += 1;
        let tv = eval_expr(bp.dbg_name, std::ptr::null_mut());
        emsg_off -= 1;
        tv
    }
}

#[cfg(not(test))]
unsafe fn dbg_parsearg(arg: *mut c_char, gap: *mut garray_T) -> c_int {
    unsafe {
        let mut p = arg;
        let mut here = false;
        let is_prof = std::ptr::eq(gap, std::ptr::addr_of!(PROF_GA));

        ga_grow(gap, 1);
        let bp = &mut *debuggy_at(gap, (*gap).ga_len);
        *bp = debuggy {
            dbg_nr: 0,
            dbg_type: 0,
            dbg_name: std::ptr::null_mut(),
            dbg_prog: std::ptr::null_mut(),
            dbg_lnum: 0,
            dbg_forceit: 0,
            dbg_val: std::ptr::null_mut(),
            dbg_level: 0,
        };

        let p_bytes = CStr::from_ptr(p).to_bytes();
        if p_bytes.starts_with(b"func") {
            bp.dbg_type = DBG_FUNC;
        } else if p_bytes.starts_with(b"file") {
            bp.dbg_type = DBG_FILE;
        } else if !is_prof && p_bytes.starts_with(b"here") {
            if (*curbuf).b_ffname.is_null() {
                emsg(c"E32: No file name".as_ptr());
                return FAIL;
            }
            bp.dbg_type = DBG_FILE;
            here = true;
        } else if !is_prof && p_bytes.starts_with(b"expr") {
            bp.dbg_type = DBG_EXPR;
        } else {
            semsg(c"E475: Invalid argument: %s".as_ptr(), p);
            return FAIL;
        }
        p = skipwhite(p.add(4));

        if here {
            bp.dbg_lnum = (*curwin).w_cursor.lnum;
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
            semsg(c"E475: Invalid argument: %s".as_ptr(), arg);
            return FAIL;
        }

        if bp.dbg_type == DBG_FUNC {
            let name_src = if rem_bytes.starts_with(b"g:") {
                p.add(2)
            } else {
                p
            };
            bp.dbg_name = xstrdup(name_src);
        } else if here {
            bp.dbg_name = xstrdup((*curbuf).b_ffname);
        } else if bp.dbg_type == DBG_EXPR {
            bp.dbg_name = xstrdup(p);
            bp.dbg_val = eval_expr_no_emsg(bp);
        } else {
            let q = expand_env_save(p);
            if q.is_null() {
                return FAIL;
            }
            let p_exp = expand_env_save(q);
            xfree(q.cast::<c_void>());
            if p_exp.is_null() {
                return FAIL;
            }
            if *p_exp as u8 != b'*' {
                bp.dbg_name = fix_fname(p_exp);
                xfree(p_exp.cast::<c_void>());
            } else {
                bp.dbg_name = p_exp;
            }
        }

        if bp.dbg_name.is_null() {
            return FAIL;
        }
        OK
    }
}

/// `:breakadd` (and `:profile` file/func) command (`ex_breakadd`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_breakadd(eap: *mut exarg_Full) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        let eap = &*eap;
        let gap = if eap.cmdidx == CMD_PROFILE {
            std::ptr::addr_of_mut!(PROF_GA)
        } else {
            std::ptr::addr_of_mut!(DBG_BREAKP)
        };

        if dbg_parsearg(eap.arg, gap) != OK {
            return;
        }

        let bp = &mut *debuggy_at(gap, (*gap).ga_len);
        bp.dbg_forceit = eap.forceit;

        if bp.dbg_type != DBG_EXPR {
            let pat = file_pat_to_reg_pat(bp.dbg_name, std::ptr::null(), std::ptr::null_mut(), 0);
            if !pat.is_null() {
                bp.dbg_prog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
                xfree(pat.cast::<c_void>());
            }
            if pat.is_null() || bp.dbg_prog.is_null() {
                xfree(bp.dbg_name.cast::<c_void>());
            } else {
                if bp.dbg_lnum == 0 {
                    bp.dbg_lnum = 1;
                }
                if eap.cmdidx != CMD_PROFILE {
                    let nr = LAST_BREAKP.fetch_add(1, Ordering::Relaxed) + 1;
                    bp.dbg_nr = nr;
                    debug_tick += 1;
                }
                (*gap).ga_len += 1;
            }
        } else {
            let nr = LAST_BREAKP.fetch_add(1, Ordering::Relaxed) + 1;
            bp.dbg_nr = nr;
            (*gap).ga_len += 1;
            debug_tick += 1;
            if std::ptr::eq(gap, std::ptr::addr_of!(DBG_BREAKP)) {
                HAS_EXPR_BREAKPOINT.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// `:debuggreedy` command (`ex_debuggreedy`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_debuggreedy(eap: *mut exarg_Full) {
    let eap = unsafe { &*eap };
    let greedy = eap.addr_count == 0 || eap.line2 != 0;
    DEBUG_GREEDY.store(greedy, Ordering::Relaxed);
}

unsafe fn update_has_expr_breakpoint() {
    unsafe {
        let gap = std::ptr::addr_of_mut!(DBG_BREAKP);
        let mut found = false;
        for i in 0..(*gap).ga_len {
            if (*debuggy_at(gap, i)).dbg_type == DBG_EXPR {
                found = true;
                break;
            }
        }
        HAS_EXPR_BREAKPOINT.store(found, Ordering::Relaxed);
    }
}

/// `:breakdel` and `:profdel` command (`ex_breakdel`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_breakdel(eap: *mut exarg_Full) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
    unsafe {
        let eap = &*eap;
        let mut todel: c_int = -1;
        let mut del_all = false;
        let mut best_lnum: i32 = 0;
        let gap = if eap.cmdidx == CMD_PROFDEL {
            std::ptr::addr_of_mut!(PROF_GA)
        } else {
            std::ptr::addr_of_mut!(DBG_BREAKP)
        };

        let first_byte = *eap.arg as u8;
        if first_byte.is_ascii_digit() {
            let nr = atoi_cstr(eap.arg);
            for i in 0..(*gap).ga_len {
                if (*debuggy_at(gap, i)).dbg_nr == nr {
                    todel = i;
                    break;
                }
            }
        } else if first_byte == b'*' {
            todel = 0;
            del_all = true;
        } else {
            if dbg_parsearg(eap.arg, gap) == FAIL {
                return;
            }
            let bp = *debuggy_at(gap, (*gap).ga_len);
            let bp_name_bytes = CStr::from_ptr(bp.dbg_name).to_bytes();
            for i in 0..(*gap).ga_len {
                let bpi = &*debuggy_at(gap, i);
                if bp.dbg_type == bpi.dbg_type
                    && CStr::from_ptr(bpi.dbg_name).to_bytes() == bp_name_bytes
                    && (bp.dbg_lnum == bpi.dbg_lnum
                        || (bp.dbg_lnum == 0 && (best_lnum == 0 || bpi.dbg_lnum < best_lnum)))
                {
                    todel = i;
                    best_lnum = bpi.dbg_lnum;
                }
            }
            xfree(bp.dbg_name.cast::<c_void>());
        }

        if todel < 0 {
            semsg(c"E161: Breakpoint not found: %s".as_ptr(), eap.arg);
            return;
        }

        while (*gap).ga_len > 0 {
            let cur = *debuggy_at(gap, todel);
            xfree(cur.dbg_name.cast::<c_void>());
            if cur.dbg_type == DBG_EXPR && !cur.dbg_val.is_null() {
                tv_free(cur.dbg_val);
            }
            vim_regfree(cur.dbg_prog);
            (*gap).ga_len -= 1;
            if todel < (*gap).ga_len {
                std::ptr::copy(
                    debuggy_at(gap, todel + 1),
                    debuggy_at(gap, todel),
                    ((*gap).ga_len - todel) as usize,
                );
            }
            if eap.cmdidx == CMD_BREAKDEL {
                debug_tick += 1;
            }
            if !del_all {
                break;
            }
        }

        if (*gap).ga_len == 0 {
            ga_clear(gap);
        }
        if std::ptr::eq(gap, std::ptr::addr_of!(DBG_BREAKP)) {
            update_has_expr_breakpoint();
        }
    }
}

/// `:breaklist` command (`ex_breaklist`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_breaklist(_eap: *mut exarg_Full) {
    #[cfg(not(test))]
    unsafe {
        let gap = std::ptr::addr_of_mut!(DBG_BREAKP);
        if (*gap).ga_len <= 0 {
            msg(c"No breakpoints defined".as_ptr(), 0);
            return;
        }

        let mut namebuf = [0 as c_char; MAXPATHL];
        for i in 0..(*gap).ga_len {
            let bp = &*debuggy_at(gap, i);
            if bp.dbg_type == DBG_FILE {
                home_replace(
                    std::ptr::null(),
                    bp.dbg_name,
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
                    bp.dbg_name.cast_const()
                } else {
                    namebuf.as_ptr()
                };
                let lnum = bp.dbg_lnum;
                let fmt_str = format!("%3d  %s %s  line {lnum}\0");
                smsg(0, fmt_str.as_ptr().cast::<c_char>(), bp.dbg_nr, kind, name);
            } else {
                smsg(0, c"%3d  expr %s".as_ptr(), bp.dbg_nr, bp.dbg_name);
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn debuggy_find(
    file: bool,
    fname: *mut c_char,
    after: i32,
    gap: *mut garray_T,
    fp: *mut bool,
) -> i32 {
    unsafe {
        if (*gap).ga_len <= 0 {
            return 0;
        }

        let mut lnum: i32 = 0;
        let is_prof = std::ptr::eq(gap, std::ptr::addr_of!(PROF_GA));
        let name = if !file && (*fname as u8) == K_SPECIAL {
            let rest_bytes = CStr::from_ptr(fname.add(3)).to_bytes();
            let buf = xmalloc(5 + rest_bytes.len() + 1).cast::<u8>();
            std::ptr::copy_nonoverlapping(b"<SNR>".as_ptr(), buf, 5);
            std::ptr::copy_nonoverlapping(rest_bytes.as_ptr(), buf.add(5), rest_bytes.len());
            *buf.add(5 + rest_bytes.len()) = 0;
            buf.cast::<c_char>()
        } else {
            fname
        };

        for i in 0..(*gap).ga_len {
            let bp = &mut *debuggy_at(gap, i);
            if ((bp.dbg_type == DBG_FILE) == file)
                && bp.dbg_type != DBG_EXPR
                && (is_prof || (bp.dbg_lnum > after && (lnum == 0 || bp.dbg_lnum < lnum)))
            {
                let prev_got_int = got_int;
                got_int = false;
                if vim_regexec_prog(&mut bp.dbg_prog, false, name, 0) {
                    lnum = bp.dbg_lnum;
                    if !fp.is_null() {
                        *fp = bp.dbg_forceit != 0;
                    }
                }
                got_int |= prev_got_int;
            } else if bp.dbg_type == DBG_EXPR {
                let mut line = false;
                let tv = eval_expr_no_emsg(bp);
                if !tv.is_null() {
                    if bp.dbg_val.is_null() {
                        xfree(DEBUG_OLDVAL.cast::<c_void>());
                        DEBUG_OLDVAL = typval_tostring(std::ptr::null_mut(), true);
                        bp.dbg_val = tv;
                        xfree(DEBUG_NEWVAL.cast::<c_void>());
                        DEBUG_NEWVAL = typval_tostring(bp.dbg_val, true);
                        line = true;
                    } else {
                        if typval_compare(tv, bp.dbg_val, EXPR_IS, false) == OK
                            && (*tv).vval.v_number == 0
                        {
                            line = true;
                            xfree(DEBUG_OLDVAL.cast::<c_void>());
                            DEBUG_OLDVAL = typval_tostring(bp.dbg_val, true);
                            let v = eval_expr_no_emsg(bp);
                            xfree(DEBUG_NEWVAL.cast::<c_void>());
                            DEBUG_NEWVAL = typval_tostring(v, true);
                            tv_free(bp.dbg_val);
                            bp.dbg_val = v;
                        }
                        tv_free(tv);
                    }
                } else if !bp.dbg_val.is_null() {
                    xfree(DEBUG_OLDVAL.cast::<c_void>());
                    DEBUG_OLDVAL = typval_tostring(bp.dbg_val, true);
                    xfree(DEBUG_NEWVAL.cast::<c_void>());
                    DEBUG_NEWVAL = typval_tostring(std::ptr::null_mut(), true);
                    tv_free(bp.dbg_val);
                    bp.dbg_val = std::ptr::null_mut();
                    line = true;
                }

                if line {
                    lnum = if after > 0 { after } else { 1 };
                    break;
                }
            }
        }

        if name != fname {
            xfree(name.cast::<c_void>());
        }

        lnum
    }
}

/// Find a breakpoint for a function or sourced file (`dbg_find_breakpoint`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dbg_find_breakpoint(file: bool, fname: *mut c_char, after: i32) -> i32 {
    #[cfg(test)]
    {
        let _ = (file, fname, after);
        0
    }
    #[cfg(not(test))]
    unsafe {
        debuggy_find(
            file,
            fname,
            after,
            std::ptr::addr_of_mut!(DBG_BREAKP),
            std::ptr::null_mut(),
        )
    }
}

/// Check if profiling is enabled for a function or sourced file (`has_profiling`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn has_profiling(file: bool, fname: *mut c_char, fp: *mut bool) -> bool {
    #[cfg(test)]
    {
        let _ = (file, fname, fp);
        false
    }
    #[cfg(not(test))]
    unsafe {
        debuggy_find(file, fname, 0, std::ptr::addr_of_mut!(PROF_GA), fp) != 0
    }
}

/// Record that a breakpoint was encountered (`dbg_breakpoint`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dbg_breakpoint(name: *mut c_char, lnum: i32) {
    unsafe {
        DEBUG_BREAKPOINT_NAME = name;
    }
    DEBUG_BREAKPOINT_LNUM.store(lnum, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_maxbacktrace_level_counts_dot_dot_separators() {
        unsafe {
            assert_eq!(get_maxbacktrace_level(std::ptr::null()), 0);
            assert_eq!(get_maxbacktrace_level(c"script.vim".as_ptr()), 0);
            assert_eq!(
                get_maxbacktrace_level(c"script.vim..FuncA..FuncB".as_ptr()),
                2
            );
        }
    }
}
