//! Script, function, and startup profiling (`src/nvim/profile.c`).

use crate::charset::{skiptowhite, skipwhite};
use crate::debugger::ex_breakadd;
use crate::errors::e_notopen;
use crate::eval::typval::{dict_T, dictitem_T, list_T, listitem_T, typval_T};
use crate::eval::userfunc::{
    defer_T, func_tbl_get, funcline, get_current_funccal, ufunc_T,
};
use crate::eval::vars::{VV_PROFILING, set_vim_var_nr};
use crate::event::r#loop::uv_err_name;
use crate::ex_docmd::exarg_T;
use crate::fileio::{FILE, fclose, fopen, fprintf, vim_fgets};
use crate::globals::*;
use crate::memory::{xfree, xmalloc};
use crate::message::{emsg, semsg};
use crate::os::env::{expand_T, expand_env_save_opt};
use crate::os::fs::os_fopen;
use crate::os::time::os_hrtime;
use crate::runtime::{get_scriptname, script_id_valid, script_item, sourcing_lnum};
use crate::strings::sort_slice_by;
use std::ffi::{CStr, c_char, c_int, c_void};

pub type proftime_T = u64;

const NS_PER_SEC: f64 = 1_000_000_000.0;
const PROF_PAUSED: c_int = 2;

const IOFBF: c_int = 0;

const PEXP_SUBCMD: i32 = 0;

static mut PROF_WAIT_TIME: proftime_T = 0;
static mut WAIT_TIME: proftime_T = 0;
static mut PAUSE_TIME: proftime_T = 0;
static mut G_START_TIME: proftime_T = 0;
static mut G_PREV_TIME: proftime_T = 0;
static mut PEXPAND_WHAT: i32 = PEXP_SUBCMD;

static mut STARTUPTIME_BUF: *mut c_char = std::ptr::null_mut();
static mut PROFILE_FNAME: *mut c_char = std::ptr::null_mut();
static mut PROFILE_MSG_BUF: [c_char; 50] = [0; 50];

const PEXPAND_CMDS: [&CStr; 7] = [
    c"continue",
    c"dump",
    c"file",
    c"func",
    c"pause",
    c"start",
    c"stop",
];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct sn_prl_T {
    pub snp_count: c_int,
    pub sn_prl_total: proftime_T,
    pub sn_prl_self: proftime_T,
}

#[repr(C)]
pub struct scriptitem_T {
    pub sn_vars: *mut crate::runtime::scriptvar_T,
    pub sn_name: *mut c_char,
    pub sn_lua: bool,
    pub sn_prof_on: bool,
    pub sn_pr_force: bool,
    pub sn_pr_child: proftime_T,
    pub sn_pr_nest: c_int,
    pub sn_pr_count: c_int,
    pub sn_pr_total: proftime_T,
    pub sn_pr_self: proftime_T,
    pub sn_pr_start: proftime_T,
    pub sn_pr_children: proftime_T,
    pub sn_prl_ga: Vec<sn_prl_T>,
    pub sn_prl_start: proftime_T,
    pub sn_prl_children: proftime_T,
    pub sn_prl_wait: proftime_T,
    pub sn_prl_idx: i32,
    pub sn_prl_execed: c_int,
}

use crate::autocmd::PROF_YES;
use crate::charset::NL;
use crate::cmdexpand::{EXPAND_FILES, EXPAND_NOTHING, EXPAND_PROFILE, EXPAND_USER_FUNC};
use crate::eval::funcs::MAX_FUNC_ARGS;
use crate::globals::{IOSIZE, PROF_NONE};
use crate::keycodes::K_SPECIAL;

#[derive(Clone, Copy)]
pub enum EStackInfo {
    None,
    Ufunc(*mut ufunc_T),
    AuCmd(*mut crate::autocmd::AutoPatCmd),
}

impl EStackInfo {
    #[inline]
    pub const fn ufunc(self) -> *mut ufunc_T {
        match self {
            Self::Ufunc(p) => p,
            _ => std::ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn aucmd(self) -> *mut crate::autocmd::AutoPatCmd {
        match self {
            Self::AuCmd(p) => p,
            _ => std::ptr::null_mut(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct estack_T {
    pub es_lnum: i32,
    pub es_name: *mut c_char,
    pub es_type: c_int,
    pub es_info: EStackInfo,
}

#[repr(C)]
pub struct funccall_T {
    pub fc_func: *mut ufunc_T,
    pub fc_linenr: c_int,
    pub fc_returned: c_int,
    pub fc_l_vars: dict_T,
    pub fc_l_vars_var: dictitem_T,
    pub fc_l_avars: dict_T,
    pub fc_l_avars_var: dictitem_T,
    pub fc_l_varlist: list_T,
    pub fc_l_listitems: [listitem_T; MAX_FUNC_ARGS],
    pub fc_rettv: *mut typval_T,
    pub fc_breakpoint: i32,
    pub fc_dbg_tick: c_int,
    pub fc_level: c_int,
    pub fc_defer: Vec<defer_T>,
    pub fc_prof_child: proftime_T,
    pub fc_caller: *mut funccall_T,
    pub fc_refcount: c_int,
    pub fc_copyID: c_int,
    pub fc_ufuncs: Vec<*mut ufunc_T>,
}

impl funccall_T {
    pub const fn new() -> Self {
        Self {
            fc_func: std::ptr::null_mut(),
            fc_linenr: 0,
            fc_returned: 0,
            fc_l_vars: dict_T::new(),
            fc_l_vars_var: dictitem_T::new(),
            fc_l_avars: dict_T::new(),
            fc_l_avars_var: dictitem_T::new(),
            fc_l_varlist: list_T::new(),
            fc_l_listitems: [listitem_T {
                li_next: std::ptr::null_mut(),
                li_prev: std::ptr::null_mut(),
                li_tv: crate::eval::typval::tv_initial_value(),
            }; MAX_FUNC_ARGS],
            fc_rettv: std::ptr::null_mut(),
            fc_breakpoint: 0,
            fc_dbg_tick: 0,
            fc_level: 0,
            fc_defer: Vec::new(),
            fc_prof_child: 0,
            fc_caller: std::ptr::null_mut(),
            fc_refcount: 0,
            fc_copyID: 0,
            fc_ufuncs: Vec::new(),
        }
    }
}

use crate::fileio::setvbuf;

#[inline]
unsafe fn prl_item(si: *mut scriptitem_T, idx: c_int) -> *mut sn_prl_T {
    unsafe { (*si).sn_prl_ga.as_mut_ptr().add(idx as usize) }
}

/// Gets the current time (`profile_start`).
pub fn profile_start() -> proftime_T {
    os_hrtime()
}

/// Computes the time elapsed (`profile_end`).
pub fn profile_end(tm: proftime_T) -> proftime_T {
    profile_sub(os_hrtime(), tm)
}

/// Gets a static string representing time `tm` (`profile_msg`).
pub unsafe fn profile_msg(tm: proftime_T) -> *const c_char {
    let secs = (profile_signed(tm) as f64) / NS_PER_SEC;
    let formatted = format!("{secs:10.6}");
    let bytes = formatted.as_bytes();
    unsafe {
        let buf = &mut *std::ptr::addr_of_mut!(PROFILE_MSG_BUF);
        let copy_len = bytes.len().min(buf.len() - 1);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().cast::<u8>(), copy_len);
        buf[copy_len] = 0;
        buf.as_ptr()
    }
}

/// Gets the time `msec` into the future (`profile_setlimit`).
pub fn profile_setlimit(msec: i64) -> proftime_T {
    if msec <= 0 {
        return profile_zero();
    }
    assert!(msec < (i64::MAX / 1_000_000));
    let nsec = (msec as proftime_T) * 1_000_000;
    os_hrtime().wrapping_add(nsec)
}

/// Checks if current time has passed `tm` (`profile_passed_limit`).
pub fn profile_passed_limit(tm: proftime_T) -> bool {
    if tm == 0 {
        return false;
    }
    profile_cmp(os_hrtime(), tm) < 0
}

/// Gets the zero time (`profile_zero`).
pub const fn profile_zero() -> proftime_T {
    0
}

/// Divides time `tm` by `count` (`profile_divide`).
pub fn profile_divide(tm: proftime_T, count: c_int) -> proftime_T {
    if count <= 0 {
        return profile_zero();
    }
    ((tm as f64) / (count as f64)).round() as proftime_T
}

/// Adds time `tm2` to `tm1` (`profile_add`).
pub const fn profile_add(tm1: proftime_T, tm2: proftime_T) -> proftime_T {
    tm1.wrapping_add(tm2)
}

/// Subtracts time `tm2` from `tm1` (`profile_sub`).
pub const fn profile_sub(tm1: proftime_T, tm2: proftime_T) -> proftime_T {
    tm1.wrapping_sub(tm2)
}

/// Adds the `self` time from the total time and the `children` time (`profile_self`).
pub const fn profile_self(
    self_time: proftime_T,
    total: proftime_T,
    children: proftime_T,
) -> proftime_T {
    if total <= children {
        return self_time;
    }
    profile_sub(profile_add(self_time, total), children)
}

#[inline]
fn profile_get_wait() -> proftime_T {
    unsafe { PROF_WAIT_TIME }
}

/// Sets the current waittime (`profile_set_wait`).
pub fn profile_set_wait(wait: proftime_T) {
    unsafe {
        PROF_WAIT_TIME = wait;
    }
}

/// Subtracts the passed waittime since `tm` (`profile_sub_wait`).
pub fn profile_sub_wait(tm: proftime_T, tma: proftime_T) -> proftime_T {
    let tm3 = profile_sub(profile_get_wait(), tm);
    profile_sub(tma, tm3)
}

#[inline]
const fn profile_equal(tm1: proftime_T, tm2: proftime_T) -> bool {
    tm1 == tm2
}

/// Converts time duration `tm` to a signed integer (`profile_signed`).
pub const fn profile_signed(tm: proftime_T) -> i64 {
    if tm <= (i64::MAX as u64) {
        tm as i64
    } else {
        -((u64::MAX - tm) as i64)
    }
}

/// Compares profiling times (`profile_cmp`).
pub const fn profile_cmp(tm1: proftime_T, tm2: proftime_T) -> c_int {
    if tm1 == tm2 {
        return 0;
    }
    if profile_signed(tm2.wrapping_sub(tm1)) < 0 {
        -1
    } else {
        1
    }
}

/// Reset all profiling information (`profile_reset`).
pub unsafe fn profile_reset() {
    unsafe {
        for id in 1..=crate::runtime::script_items_len() {
            let si = &mut *script_item(id);
            if si.sn_prof_on {
                si.sn_prof_on = false;
                si.sn_pr_force = false;
                si.sn_pr_child = profile_zero();
                si.sn_pr_nest = 0;
                si.sn_pr_count = 0;
                si.sn_pr_total = profile_zero();
                si.sn_pr_self = profile_zero();
                si.sn_pr_start = profile_zero();
                si.sn_pr_children = profile_zero();
                si.sn_prl_ga = Vec::new();
                si.sn_prl_start = profile_zero();
                si.sn_prl_children = profile_zero();
                si.sn_prl_wait = profile_zero();
                si.sn_prl_idx = -1;
                si.sn_prl_execed = 0;
            }
        }

        let functbl = func_tbl_get();
        for &fp in (*functbl).values() {
            let uf = &mut *fp;
            if uf.uf_prof_initialized != 0 {
                uf.uf_profiling = 0;
                uf.uf_tm_count = 0;
                uf.uf_tm_total = profile_zero();
                uf.uf_tm_self = profile_zero();
                uf.uf_tm_children = profile_zero();

                uf.uf_tml_count.fill(0);
                uf.uf_tml_total.fill(0);
                uf.uf_tml_self.fill(0);

                uf.uf_tml_start = profile_zero();
                uf.uf_tml_children = profile_zero();
                uf.uf_tml_wait = profile_zero();
                uf.uf_tml_idx = -1;
                uf.uf_tml_execed = 0;
            }
        }

        xfree(PROFILE_FNAME.cast::<c_void>());
        PROFILE_FNAME = std::ptr::null_mut();
    }
}

/// `":profile cmd args"` (`ex_profile`).
pub unsafe fn ex_profile(eap: *mut exarg_T) {
    unsafe {
        let arg = (*eap).arg;
        let mut e = skiptowhite(arg);
        let len = (e as usize) - (arg as usize);
        e = skipwhite(e);

        let arg_bytes = CStr::from_ptr(arg).to_bytes();
        if len == 5 && &arg_bytes[..5] == b"start" && *e != 0 {
            xfree(PROFILE_FNAME.cast::<c_void>());
            PROFILE_FNAME = expand_env_save_opt(e, true, std::ptr::null_mut());
            do_profiling = PROF_YES;
            profile_set_wait(profile_zero());
            set_vim_var_nr(VV_PROFILING, 1);
        } else if do_profiling == PROF_NONE {
            emsg(c"E750: First use \":profile start {fname}\"".as_ptr());
        } else if arg_bytes == b"stop" {
            profile_dump();
            do_profiling = PROF_NONE;
            set_vim_var_nr(VV_PROFILING, 0);
            profile_reset();
        } else if arg_bytes == b"pause" {
            if do_profiling == PROF_YES {
                PAUSE_TIME = profile_start();
            }
            do_profiling = PROF_PAUSED;
        } else if arg_bytes == b"continue" {
            if do_profiling == PROF_PAUSED {
                let elapsed = profile_end(PAUSE_TIME);
                PAUSE_TIME = elapsed;
                profile_set_wait(profile_add(profile_get_wait(), elapsed));
            }
            do_profiling = PROF_YES;
        } else if arg_bytes == b"dump" {
            profile_dump();
        } else {
            ex_breakadd(eap.cast::<exarg_T>());
        }
    }
}

/// Command-line completion item getter for `:profile` (`get_profile_name`).
pub unsafe fn get_profile_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if unsafe { PEXPAND_WHAT } == PEXP_SUBCMD && idx >= 0 && (idx as usize) < PEXPAND_CMDS.len() {
        PEXPAND_CMDS[idx as usize].as_ptr().cast_mut()
    } else {
        std::ptr::null_mut()
    }
}

/// Set command-line completion context for `:profile` (`set_context_in_profile_cmd`).
pub unsafe fn set_context_in_profile_cmd(xp: *mut expand_T, arg: *const c_char) {
    let xp = unsafe { &mut *xp };
    xp.xp_context = EXPAND_PROFILE;
    unsafe {
        PEXPAND_WHAT = PEXP_SUBCMD;
    }
    xp.xp_pattern = arg.cast_mut();

    let end_subcmd = unsafe { skiptowhite(arg) };
    if unsafe { *end_subcmd } == 0 {
        return;
    }

    let len = (end_subcmd as usize) - (arg as usize);
    let sub_bytes = unsafe { std::slice::from_raw_parts(arg.cast::<u8>(), len) };
    if sub_bytes == b"start" || sub_bytes == b"file" {
        xp.xp_context = EXPAND_FILES;
        xp.xp_pattern = unsafe { skipwhite(end_subcmd) };
    } else if sub_bytes == b"func" {
        xp.xp_context = EXPAND_USER_FUNC;
        xp.xp_pattern = unsafe { skipwhite(end_subcmd) };
    } else {
        xp.xp_context = EXPAND_NOTHING;
    }
}

/// Called when starting to wait for user input (`prof_input_start`).
pub fn prof_input_start() {
    unsafe {
        WAIT_TIME = profile_start();
    }
}

/// Called when finished waiting for user input (`prof_input_end`).
pub fn prof_input_end() {
    let elapsed = profile_end(unsafe { WAIT_TIME });
    unsafe {
        WAIT_TIME = elapsed;
    }
    profile_set_wait(profile_add(profile_get_wait(), elapsed));
}

/// Returns `true` when a function defined in the current script should be profiled (`prof_def_func`).
pub unsafe fn prof_def_func() -> bool {
    unsafe {
        if current_sctx.sc_sid > 0 {
            (*script_item(current_sctx.sc_sid)).sn_pr_force
        } else {
            false
        }
    }
}

unsafe fn prof_func_line(
    fd: *mut FILE,
    count: c_int,
    total: &proftime_T,
    self_time: &proftime_T,
    prefer_self: bool,
) {
    unsafe {
        if count > 0 {
            fprintf!(fd, c"%5d ".as_ptr(), count);
            if prefer_self && profile_equal(*total, *self_time) {
                fprintf!(fd, c"           ".as_ptr());
            } else {
                fprintf!(fd, c"%s ".as_ptr(), profile_msg(*total));
            }
            if !prefer_self && profile_equal(*total, *self_time) {
                fprintf!(fd, c"           ".as_ptr());
            } else {
                fprintf!(fd, c"%s ".as_ptr(), profile_msg(*self_time));
            }
        } else {
            fprintf!(fd, c"                            ".as_ptr());
        }
    }
}

unsafe fn prof_sort_list(
    fd: *mut FILE,
    sorttab: *mut *mut ufunc_T,
    st_len: c_int,
    title: *const c_char,
    prefer_self: bool,
) {
    unsafe {
        fprintf!(fd, c"FUNCTIONS SORTED ON %s TIME\n".as_ptr(), title);
        fprintf!(fd, c"count  total (s)   self (s)  function\n".as_ptr());
        let limit = st_len.min(20);
        for i in 0..limit {
            let fp = &**sorttab.add(i as usize);
            prof_func_line(
                fd,
                fp.uf_tm_count,
                &fp.uf_tm_total,
                &fp.uf_tm_self,
                prefer_self,
            );
            let name_ptr = fp.uf_name.as_ptr();
            if (*name_ptr as u8) == K_SPECIAL {
                fprintf!(fd, c" <SNR>%s()\n".as_ptr(), name_ptr.add(3));
            } else {
                fprintf!(fd, c" %s()\n".as_ptr(), name_ptr);
            }
        }
        fprintf!(fd, c"\n".as_ptr());
    }
}

unsafe fn prof_total_cmp(&s1: &*mut ufunc_T, &s2: &*mut ufunc_T) -> c_int {
    unsafe { profile_cmp((*s1).uf_tm_total, (*s2).uf_tm_total) }
}

unsafe fn prof_self_cmp(&s1: &*mut ufunc_T, &s2: &*mut ufunc_T) -> c_int {
    unsafe { profile_cmp((*s1).uf_tm_self, (*s2).uf_tm_self) }
}

/// Start profiling function `fp` (`func_do_profile`).
pub unsafe fn func_do_profile(fp: *mut ufunc_T) {
    let fp = unsafe { &mut *fp };
    let mut len = fp.uf_lines.len() as c_int;

    if fp.uf_prof_initialized == 0 {
        if len == 0 {
            len = 1;
        }
        fp.uf_tm_count = 0;
        fp.uf_tm_self = profile_zero();
        fp.uf_tm_total = profile_zero();

        let count_len = len as usize;
        if fp.uf_tml_count.is_empty() {
            fp.uf_tml_count.resize(count_len, 0);
        }
        if fp.uf_tml_total.is_empty() {
            fp.uf_tml_total.resize(count_len, 0);
        }
        if fp.uf_tml_self.is_empty() {
            fp.uf_tml_self.resize(count_len, 0);
        }

        fp.uf_tml_idx = -1;
        fp.uf_prof_initialized = 1;
    }

    fp.uf_profiling = 1;
}

/// Prepare profiling for entering a child (`prof_child_enter`).
pub unsafe fn prof_child_enter(tm: *mut proftime_T) {
    unsafe {
        let fc = get_current_funccal();
        if !fc.is_null() && (*(*fc).fc_func).uf_profiling != 0 {
            (*fc).fc_prof_child = profile_start();
        }
    }
    unsafe {
        script_prof_save(tm);
    }
}

/// Take care of time spent in a child (`prof_child_exit`).
pub unsafe fn prof_child_exit(tm: *mut proftime_T) {
    unsafe {
        let fc = get_current_funccal();
        if !fc.is_null() && (*(*fc).fc_func).uf_profiling != 0 {
            let child_elapsed = profile_sub_wait(*tm, profile_end((*fc).fc_prof_child));
            (*fc).fc_prof_child = child_elapsed;
            let func = &mut *(*fc).fc_func;
            func.uf_tm_children = profile_add(func.uf_tm_children, child_elapsed);
            func.uf_tml_children = profile_add(func.uf_tml_children, child_elapsed);
        }
    }
    unsafe {
        script_prof_restore(tm);
    }
}

/// Called when starting to read a function line (`func_line_start`).
pub unsafe fn func_line_start(fcp: *mut funccall_T) {
    unsafe {
        let fp = &mut *(*fcp).fc_func;
        let slnum = sourcing_lnum();

        if fp.uf_profiling != 0 && slnum >= 1 && slnum <= fp.uf_lines.len() as c_int {
            fp.uf_tml_idx = slnum - 1;
            while fp.uf_tml_idx > 0 && funcline(fp, fp.uf_tml_idx).is_null() {
                fp.uf_tml_idx -= 1;
            }
            fp.uf_tml_execed = 0;
            fp.uf_tml_start = profile_start();
            fp.uf_tml_children = profile_zero();
            fp.uf_tml_wait = profile_get_wait();
        }
    }
}

/// Called when actually executing a function line (`func_line_exec`).
pub unsafe fn func_line_exec(fcp: *mut funccall_T) {
    let fp = unsafe { &mut *(*fcp).fc_func };
    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        fp.uf_tml_execed = 1;
    }
}

/// Called when done with a function line (`func_line_end`).
pub unsafe fn func_line_end(fcp: *mut funccall_T) {
    let fp = unsafe { &mut *(*fcp).fc_func };

    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        if fp.uf_tml_execed != 0 {
            let idx = fp.uf_tml_idx as usize;
            fp.uf_tml_count[idx] += 1;
            fp.uf_tml_start = profile_sub_wait(fp.uf_tml_wait, profile_end(fp.uf_tml_start));
            fp.uf_tml_total[idx] = profile_add(fp.uf_tml_total[idx], fp.uf_tml_start);
            fp.uf_tml_self[idx] = profile_self(
                fp.uf_tml_self[idx],
                fp.uf_tml_start,
                fp.uf_tml_children,
            );
        }
        fp.uf_tml_idx = -1;
    }
}

unsafe fn func_dump_profile(fd: *mut FILE) {
    unsafe {
        let functbl = func_tbl_get();
        let todo = (*functbl).len();
        if todo == 0 {
            return;
        }

        let mut sorttab: Vec<*mut ufunc_T> = Vec::with_capacity(todo);

        for &fp in (*functbl).values() {
            if (*fp).uf_prof_initialized != 0 {
                sorttab.push(fp);

                let name_ptr = (*fp).uf_name.as_ptr();
                if (*name_ptr as u8) == K_SPECIAL {
                    fprintf!(fd, c"FUNCTION  <SNR>%s()\n".as_ptr(), name_ptr.add(3));
                } else {
                    fprintf!(fd, c"FUNCTION  %s()\n".as_ptr(), name_ptr);
                }
                    if (*fp).uf_script_ctx.sc_sid != 0 {
                        let mut should_free = false;
                        let p = get_scriptname((*fp).uf_script_ctx, &mut should_free);
                        fprintf!(
                            fd,
                            c"    Defined: %s:%d\n".as_ptr(),
                            p,
                            (*fp).uf_script_ctx.sc_lnum,
                        );
                        if should_free {
                            xfree(p.cast::<c_void>());
                        }
                    }
                    if (*fp).uf_tm_count == 1 {
                        fprintf!(fd, c"Called 1 time\n".as_ptr());
                    } else {
                        fprintf!(fd, c"Called %d times\n".as_ptr(), (*fp).uf_tm_count);
                    }
                    fprintf!(
                        fd,
                        c"Total time: %s\n".as_ptr(),
                        profile_msg((*fp).uf_tm_total),
                    );
                    fprintf!(
                        fd,
                        c" Self time: %s\n".as_ptr(),
                        profile_msg((*fp).uf_tm_self),
                    );
                    fprintf!(fd, c"\n".as_ptr());
                    fprintf!(fd, c"count  total (s)   self (s)\n".as_ptr());

                    for i in 0..(*fp).uf_lines.len() as c_int {
                        let line = funcline(fp, i);
                        if line.is_null() {
                            continue;
                        }
                        let idx = i as usize;
                        prof_func_line(
                            fd,
                            (*fp).uf_tml_count.as_slice()[idx],
                            &(*fp).uf_tml_total.as_slice()[idx],
                            &(*fp).uf_tml_self.as_slice()[idx],
                            true,
                        );
                        fprintf!(fd, c"%s\n".as_ptr(), line);
                    }
                    fprintf!(fd, c"\n".as_ptr());
            }
        }

        if !sorttab.is_empty() {
            let st_len = sorttab.len() as c_int;
            sort_slice_by(&mut sorttab, |a, b| prof_total_cmp(a, b));
            prof_sort_list(fd, sorttab.as_mut_ptr(), st_len, c"TOTAL".as_ptr(), false);
            sort_slice_by(&mut sorttab, |a, b| prof_self_cmp(a, b));
            prof_sort_list(fd, sorttab.as_mut_ptr(), st_len, c"SELF".as_ptr(), true);
        }
    }
}

/// Start profiling a script (`profile_init`).
pub unsafe fn profile_init(si: *mut scriptitem_T) {
    let si = unsafe { &mut *si };
    si.sn_pr_count = 0;
    si.sn_pr_total = profile_zero();
    si.sn_pr_self = profile_zero();
    si.sn_prl_ga.clear();
    si.sn_prl_ga.reserve(100);
    si.sn_prl_idx = -1;
    si.sn_prof_on = true;
    si.sn_pr_nest = 0;
}

/// Save time when starting to invoke another script or function (`script_prof_save`).
pub unsafe fn script_prof_save(tm: *mut proftime_T) {
    unsafe {
        if script_id_valid(current_sctx.sc_sid) {
            let si = &mut *script_item(current_sctx.sc_sid);
            if si.sn_prof_on {
                let prev_nest = si.sn_pr_nest;
                si.sn_pr_nest += 1;
                if prev_nest == 0 {
                    si.sn_pr_child = profile_start();
                }
            }
        }
    }
    unsafe {
        *tm = profile_get_wait();
    }
}

/// Count time spent in children after invoking another script or function (`script_prof_restore`).
pub unsafe fn script_prof_restore(tm: *const proftime_T) {
    unsafe {
        if !script_id_valid(current_sctx.sc_sid) {
            return;
        }

        let si = &mut *script_item(current_sctx.sc_sid);
        if si.sn_prof_on {
            si.sn_pr_nest -= 1;
            if si.sn_pr_nest == 0 {
                let elapsed = profile_sub_wait(*tm, profile_end(si.sn_pr_child));
                si.sn_pr_child = elapsed;
                si.sn_pr_children = profile_add(si.sn_pr_children, elapsed);
                si.sn_prl_children = profile_add(si.sn_prl_children, elapsed);
            }
        }
    }
}

unsafe fn script_dump_profile(fd: *mut FILE) {
    unsafe {
        let mut iobuf = [0 as c_char; IOSIZE];
        for id in 1..=crate::runtime::script_items_len() {
            let si = script_item(id);
            if (*si).sn_prof_on {
                fprintf!(fd, c"SCRIPT  %s\n".as_ptr(), (*si).sn_name);
                if (*si).sn_pr_count == 1 {
                    fprintf!(fd, c"Sourced 1 time\n".as_ptr());
                } else {
                    fprintf!(fd, c"Sourced %d times\n".as_ptr(), (*si).sn_pr_count);
                }
                fprintf!(
                    fd,
                    c"Total time: %s\n".as_ptr(),
                    profile_msg((*si).sn_pr_total),
                );
                fprintf!(
                    fd,
                    c" Self time: %s\n".as_ptr(),
                    profile_msg((*si).sn_pr_self),
                );
                fprintf!(fd, c"\n".as_ptr());
                fprintf!(fd, c"count  total (s)   self (s)\n".as_ptr());

                let sfd = os_fopen((*si).sn_name, c"r".as_ptr());
                if sfd.is_null() {
                    fprintf!(fd, c"Cannot open file!\n".as_ptr());
                } else {
                    let mut i: c_int = 0;
                    loop {
                        if vim_fgets(iobuf.as_mut_ptr(), IOSIZE as c_int, sfd) {
                            break;
                        }
                        if iobuf[IOSIZE - 2] != 0 && iobuf[IOSIZE - 2] != NL {
                            let mut n = IOSIZE - 2;
                            while n > 0 && ((iobuf[n] as u8) & 0xc0) == 0x80 {
                                n -= 1;
                            }
                            iobuf[n] = NL;
                            iobuf[n + 1] = 0;
                        }
                        if (i as usize) < (*si).sn_prl_ga.len() {
                            let pp = &*prl_item(si, i);
                            if pp.snp_count > 0 {
                                fprintf!(fd, c"%5d ".as_ptr(), pp.snp_count);
                                if profile_equal(pp.sn_prl_total, pp.sn_prl_self) {
                                    fprintf!(fd, c"           ".as_ptr());
                                } else {
                                    fprintf!(fd, c"%s ".as_ptr(), profile_msg(pp.sn_prl_total));
                                }
                                fprintf!(fd, c"%s ".as_ptr(), profile_msg(pp.sn_prl_self));
                            } else {
                                fprintf!(fd, c"                            ".as_ptr());
                            }
                        } else {
                            fprintf!(fd, c"                            ".as_ptr());
                        }
                        fprintf!(fd, c"%s".as_ptr(), iobuf.as_ptr());
                        i += 1;
                    }
                    fclose(sfd);
                }
                fprintf!(fd, c"\n".as_ptr());
            }
        }
    }
}

/// Dump the profiling info (`profile_dump`).
pub unsafe fn profile_dump() {
    unsafe {
        if PROFILE_FNAME.is_null() {
            return;
        }

        let fd = os_fopen(PROFILE_FNAME, c"w".as_ptr());
        if fd.is_null() {
            semsg!(e_notopen.as_ptr(), PROFILE_FNAME);
        } else {
            script_dump_profile(fd);
            func_dump_profile(fd);
            fclose(fd);
        }
    }
}

/// Called when starting to read a script line (`script_line_start`).
pub unsafe fn script_line_start() {
    unsafe {
        if !script_id_valid(current_sctx.sc_sid) {
            return;
        }
        let si = script_item(current_sctx.sc_sid);
        let slnum = sourcing_lnum();
        if (*si).sn_prof_on && slnum >= 1 {
            (*si).sn_prl_idx = slnum - 1;
            if (*si).sn_prl_ga.len() < slnum as usize {
                (*si).sn_prl_ga.resize(
                    slnum as usize,
                    sn_prl_T {
                        snp_count: 0,
                        sn_prl_total: profile_zero(),
                        sn_prl_self: profile_zero(),
                    },
                );
            }
            (*si).sn_prl_execed = 0;
            (*si).sn_prl_start = profile_start();
            (*si).sn_prl_children = profile_zero();
            (*si).sn_prl_wait = profile_get_wait();
        }
    }
}

/// Called when actually executing a script line (`script_line_exec`).
pub unsafe fn script_line_exec() {
    unsafe {
        if !script_id_valid(current_sctx.sc_sid) {
            return;
        }
        let si = &mut *script_item(current_sctx.sc_sid);
        if si.sn_prof_on && si.sn_prl_idx >= 0 {
            si.sn_prl_execed = 1;
        }
    }
}

/// Called when done with a script line (`script_line_end`).
pub unsafe fn script_line_end() {
    unsafe {
        if !script_id_valid(current_sctx.sc_sid) {
            return;
        }
        let si = script_item(current_sctx.sc_sid);
        if (*si).sn_prof_on
            && (*si).sn_prl_idx >= 0
            && ((*si).sn_prl_idx as usize) < (*si).sn_prl_ga.len()
        {
            if (*si).sn_prl_execed != 0 {
                let pp = &mut *prl_item(si, (*si).sn_prl_idx);
                pp.snp_count += 1;
                (*si).sn_prl_start =
                    profile_sub_wait((*si).sn_prl_wait, profile_end((*si).sn_prl_start));
                pp.sn_prl_total = profile_add(pp.sn_prl_total, (*si).sn_prl_start);
                pp.sn_prl_self =
                    profile_self(pp.sn_prl_self, (*si).sn_prl_start, (*si).sn_prl_children);
            }
            (*si).sn_prl_idx = -1;
        }
    }
}

/// Saves the previous time before doing something that could nest (`time_push`).
pub unsafe fn time_push(rel: *mut proftime_T, start: *mut proftime_T) {
    let now = profile_start();
    unsafe {
        let prev = G_PREV_TIME;
        G_PREV_TIME = now;
        *rel = profile_sub(now, prev);
        *start = now;
    }
}

/// Computes the prev time after doing something that could nest (`time_pop`).
pub fn time_pop(tp: proftime_T) {
    unsafe {
        G_PREV_TIME = G_PREV_TIME.wrapping_sub(tp);
    }
}

unsafe fn time_diff(then: proftime_T, now: proftime_T) {
    let diff = profile_sub(now, then);
    let s = format!("{:07.3}\0", (diff as f64) / 1.0e6);
    unsafe {
        fprintf!(time_fd, c"%s".as_ptr(), s.as_ptr().cast::<c_char>());
    }
}

/// Initializes the startuptime code (`time_start`).
pub unsafe fn time_start(message: *const c_char) {
    unsafe {
        if time_fd.is_null() {
            return;
        }

        let now = profile_start();
        G_START_TIME = now;
        G_PREV_TIME = now;

        fprintf!(time_fd, c"\ntimes in msec\n".as_ptr());
        fprintf!(
            time_fd,
            c" clock   self+sourced   self:  sourced script\n".as_ptr(),
        );
        fprintf!(
            time_fd,
            c" clock   elapsed:              other lines\n\n".as_ptr(),
        );

        time_msg(message, std::ptr::null());
    }
}

/// Prints out timing info (`time_msg`).
pub unsafe fn time_msg(mesg: *const c_char, start: *const proftime_T) {
    unsafe {
        if time_fd.is_null() {
            return;
        }

        let now = profile_start();
        time_diff(G_START_TIME, now);

        if !start.is_null() {
            fprintf!(time_fd, c"  ".as_ptr());
            time_diff(*start, now);
        }

        fprintf!(time_fd, c"  ".as_ptr());
        let prev = G_PREV_TIME;
        G_PREV_TIME = now;
        time_diff(prev, now);

        fprintf!(time_fd, c": %s\n".as_ptr(), mesg);
    }
}

/// Initializes the `time_fd` stream for `--startuptime` (`time_init`).
pub unsafe fn time_init(fname: *const c_char, proc_name: *const c_char) {
    unsafe {
        const BUFSIZE: usize = 8192;
        time_fd = fopen(fname, c"a".as_ptr());
        if time_fd.is_null() {
            let fname_str = CStr::from_ptr(fname).to_string_lossy();
            eprint!("E484: Can't open file {fname_str}");
            return;
        }
        STARTUPTIME_BUF = xmalloc(BUFSIZE + 1).cast::<c_char>();
        let r = setvbuf(time_fd, STARTUPTIME_BUF, IOFBF, BUFSIZE + 1);
        if r != 0 {
            xfree(STARTUPTIME_BUF.cast::<c_void>());
            STARTUPTIME_BUF = std::ptr::null_mut();
            fclose(time_fd);
            time_fd = std::ptr::null_mut();
            let err_str = CStr::from_ptr(uv_err_name(r)).to_string_lossy();
            eprint!("time_init: setvbuf failed: {r} {err_str}");
            return;
        }
        fprintf!(
            time_fd,
            c"--- Startup times for process: %s ---\n".as_ptr(),
            proc_name,
        );
    }
}

/// Flushes the startuptimes to disk for the current process (`time_finish`).
pub unsafe fn time_finish() {
    unsafe {
        if time_fd.is_null() {
            return;
        }
        assert!(!STARTUPTIME_BUF.is_null());
        time_msg(c"--- NVIM STARTED ---\n".as_ptr(), std::ptr::null());

        fclose(time_fd);
        time_fd = std::ptr::null_mut();

        xfree(STARTUPTIME_BUF.cast::<c_void>());
        STARTUPTIME_BUF = std::ptr::null_mut();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_math_and_formatting() {
        assert_eq!(profile_zero(), 0);
        assert_eq!(profile_add(100, 250), 350);
        assert_eq!(profile_sub(250, 100), 150);
        assert_eq!(profile_divide(105, 10), 11);
        assert_eq!(profile_self(10, 50, 20), 40);
        assert_eq!(profile_self(10, 20, 50), 10);
        assert_eq!(profile_cmp(10, 20), 1);
        assert_eq!(profile_cmp(20, 10), -1);
        assert_eq!(profile_cmp(10, 10), 0);
        assert_eq!(profile_cmp(profile_zero(), profile_zero()), 0);
        unsafe {
            let msg = CStr::from_ptr(profile_msg(0)).to_str().unwrap();
            assert_eq!(msg.trim(), "0.000000");
        }
    }

    #[test]
    fn profile_timers_limits_and_division() {
        let mut last = profile_start();
        for _ in 0..100 {
            let curr = profile_start();
            assert!(curr >= last);
            last = curr;
        }

        let start_outer = profile_start();
        let start_inner = profile_start();
        let elapsed_inner = profile_end(start_inner);
        let elapsed_outer = profile_end(start_outer);
        assert!(elapsed_outer >= elapsed_inner);

        let divisor = 10;
        let start = profile_start();
        let divided = profile_divide(start, divisor);
        assert!(profile_cmp(start, divided) <= 0);
        let mut res = divided;
        for _ in 1..divisor {
            res = profile_add(res, divided);
        }
        let start_min = profile_sub(start, divisor as u64);
        let start_max = profile_add(start, divisor as u64);
        assert!(profile_cmp(start_min, res) >= 0);
        assert!(profile_cmp(start_max, res) <= 0);

        assert_eq!(profile_setlimit(0), profile_zero());
        let future = profile_setlimit(1000);
        let now = profile_start();
        assert!(profile_cmp(future, now) < 0);
        assert!(profile_passed_limit(now));
        assert!(!profile_passed_limit(profile_add(now, now)));

        unsafe {
            let endt = profile_end(profile_start());
            let s = CStr::from_ptr(profile_msg(endt)).to_str().unwrap().trim();
            let (secs, _us) = s.split_once('.').unwrap();
            assert_eq!(secs, "0");
        }
    }
}
