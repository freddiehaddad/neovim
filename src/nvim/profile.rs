//! Script, function, and startup profiling (`src/nvim/profile.c`).

#![allow(non_camel_case_types, non_snake_case, clippy::missing_safety_doc)]

use crate::api::private::converter::ufunc_T;
use crate::clipboard::{list_T, listitem_T, typval_T};
#[cfg(not(test))]
use crate::debugger::{ex_breakadd, exarg_Full};
use crate::eval::decode::{dict_T, dictitem_T};
#[cfg(not(test))]
use crate::eval::userfunc::{func_tbl_get, get_current_funccal};
#[cfg(not(test))]
use crate::eval::vars::{VV_PROFILING, set_vim_var_nr};
#[cfg(not(test))]
use crate::event::r#loop::uv_err_name;
#[cfg(not(test))]
use crate::fileio::{fclose, fopen, fprintf, vim_fgets};
#[cfg(not(test))]
use crate::garray::{ga_clear, ga_grow};
use crate::garray::{ga_init, garray_T};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::hashtab::hashitem_empty;
use crate::lua::secure::exarg_T;
use crate::memory::xcalloc;
#[cfg(not(test))]
use crate::memory::{xfree, xmalloc};
#[cfg(not(test))]
use crate::message::{emsg, semsg};
#[cfg(not(test))]
use crate::os::env::expand_env_save_opt;
#[cfg(not(test))]
use crate::os::fs::os_fopen;
use crate::os::time::os_hrtime;
#[cfg(not(test))]
use crate::runtime::{get_scriptname, script_items, sourcing_lnum};
#[cfg(not(test))]
use crate::strings::qsort;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

pub type proftime_T = u64;
pub use crate::fileio::FILE;

const NS_PER_SEC: f64 = 1_000_000_000.0;
#[cfg(not(test))]
const PROF_PAUSED: c_int = 2;

#[cfg(not(test))]
const IOFBF: c_int = 0;

const PEXP_SUBCMD: i32 = 0;

static PROF_WAIT_TIME: AtomicU64 = AtomicU64::new(0);
static WAIT_TIME: AtomicU64 = AtomicU64::new(0);
#[cfg(not(test))]
static PAUSE_TIME: AtomicU64 = AtomicU64::new(0);
#[cfg(not(test))]
static G_START_TIME: AtomicU64 = AtomicU64::new(0);
static G_PREV_TIME: AtomicU64 = AtomicU64::new(0);
static PEXPAND_WHAT: AtomicI32 = AtomicI32::new(PEXP_SUBCMD);

#[cfg(not(test))]
static mut STARTUPTIME_BUF: *mut c_char = std::ptr::null_mut();
#[cfg(not(test))]
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
    pub sn_vars: *mut c_void,
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
    pub sn_prl_ga: garray_T,
    pub sn_prl_start: proftime_T,
    pub sn_prl_children: proftime_T,
    pub sn_prl_wait: proftime_T,
    pub sn_prl_idx: i32,
    pub sn_prl_execed: c_int,
}

#[cfg(not(test))]
use crate::api::buffer::NL;
#[cfg(not(test))]
use crate::api::options::IOSIZE;
#[cfg(not(test))]
use crate::autocmd::PROF_YES;
use crate::cmdexpand::{EXPAND_FILES, EXPAND_NOTHING, EXPAND_PROFILE, EXPAND_USER_FUNC};
use crate::eval::funcs::MAX_FUNC_ARGS;
use crate::eval::userfunc::{FIXVAR_CNT, VAR_SHORT_LEN};
#[cfg(not(test))]
use crate::globals::PROF_NONE;
#[cfg(not(test))]
use crate::keycodes::K_SPECIAL;
pub use crate::os::env::expand_T;

#[repr(C)]
pub struct estack_T {
    pub es_lnum: i32,
    pub es_name: *mut c_char,
    pub es_type: c_int,
    pub es_info: *mut c_void,
}

#[repr(C)]
pub struct FixVarItem {
    pub di_tv: typval_T,
    pub di_flags: u8,
    pub di_key: [c_char; VAR_SHORT_LEN + 1],
}

#[repr(C)]
pub struct funccall_T {
    pub fc_func: *mut ufunc_T,
    pub fc_linenr: c_int,
    pub fc_returned: c_int,
    pub fc_fixvar: [FixVarItem; FIXVAR_CNT],
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
    pub fc_defer: garray_T,
    pub fc_prof_child: proftime_T,
    pub fc_caller: *mut funccall_T,
    pub fc_refcount: c_int,
    pub fc_copyID: c_int,
    pub fc_ufuncs: garray_T,
}

#[cfg(not(test))]
unsafe extern "C" {

    fn setvbuf(stream: *mut FILE, buffer: *mut c_char, mode: c_int, size: usize) -> c_int;
}

#[inline]
unsafe fn skipwhite(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while matches!(*p as u8, b' ' | b'\t') {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

#[inline]
unsafe fn skiptowhite(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while !matches!(*p as u8, b' ' | b'\t' | 0) {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn script_item(id: c_int) -> *mut scriptitem_T {
    unsafe {
        *script_items
            .ga_data
            .cast::<*mut scriptitem_T>()
            .add((id - 1) as usize)
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn script_id_valid(id: c_int) -> bool {
    id > 0 && id <= unsafe { script_items.ga_len }
}

#[cfg(not(test))]
#[inline]
unsafe fn funcline(fp: *const ufunc_T, j: c_int) -> *mut c_char {
    unsafe { *(*fp).uf_lines.ga_data.cast::<*mut c_char>().add(j as usize) }
}

#[cfg(not(test))]
#[inline]
unsafe fn prl_item(si: *mut scriptitem_T, idx: c_int) -> *mut sn_prl_T {
    unsafe { (*si).sn_prl_ga.ga_data.cast::<sn_prl_T>().add(idx as usize) }
}

/// Gets the current time (`profile_start`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_start() -> proftime_T {
    os_hrtime()
}

/// Computes the time elapsed (`profile_end`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_end(tm: proftime_T) -> proftime_T {
    profile_sub(os_hrtime(), tm)
}

/// Gets a static string representing time `tm` (`profile_msg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn profile_msg(tm: proftime_T) -> *const c_char {
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
#[unsafe(no_mangle)]
pub extern "C" fn profile_setlimit(msec: i64) -> proftime_T {
    if msec <= 0 {
        return profile_zero();
    }
    assert!(msec < (i64::MAX / 1_000_000));
    let nsec = (msec as proftime_T) * 1_000_000;
    os_hrtime().wrapping_add(nsec)
}

/// Checks if current time has passed `tm` (`profile_passed_limit`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_passed_limit(tm: proftime_T) -> bool {
    if tm == 0 {
        return false;
    }
    profile_cmp(os_hrtime(), tm) < 0
}

/// Gets the zero time (`profile_zero`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_zero() -> proftime_T {
    0
}

/// Divides time `tm` by `count` (`profile_divide`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_divide(tm: proftime_T, count: c_int) -> proftime_T {
    if count <= 0 {
        return profile_zero();
    }
    ((tm as f64) / (count as f64)).round() as proftime_T
}

/// Adds time `tm2` to `tm1` (`profile_add`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_add(tm1: proftime_T, tm2: proftime_T) -> proftime_T {
    tm1.wrapping_add(tm2)
}

/// Subtracts time `tm2` from `tm1` (`profile_sub`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_sub(tm1: proftime_T, tm2: proftime_T) -> proftime_T {
    tm1.wrapping_sub(tm2)
}

/// Adds the `self` time from the total time and the `children` time (`profile_self`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_self(
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
    PROF_WAIT_TIME.load(Ordering::Relaxed)
}

/// Sets the current waittime (`profile_set_wait`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_set_wait(wait: proftime_T) {
    PROF_WAIT_TIME.store(wait, Ordering::Relaxed);
}

/// Subtracts the passed waittime since `tm` (`profile_sub_wait`).
#[unsafe(no_mangle)]
pub extern "C" fn profile_sub_wait(tm: proftime_T, tma: proftime_T) -> proftime_T {
    let tm3 = profile_sub(profile_get_wait(), tm);
    profile_sub(tma, tm3)
}

#[cfg(not(test))]
#[inline]
const fn profile_equal(tm1: proftime_T, tm2: proftime_T) -> bool {
    tm1 == tm2
}

/// Converts time duration `tm` to a signed integer (`profile_signed`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_signed(tm: proftime_T) -> i64 {
    if tm <= (i64::MAX as u64) {
        tm as i64
    } else {
        -((u64::MAX - tm) as i64)
    }
}

/// Compares profiling times (`profile_cmp`).
#[unsafe(no_mangle)]
pub const extern "C" fn profile_cmp(tm1: proftime_T, tm2: proftime_T) -> c_int {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn profile_reset() {
    #[cfg(not(test))]
    unsafe {
        for id in 1..=script_items.ga_len {
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
                ga_clear(&mut si.sn_prl_ga);
                si.sn_prl_start = profile_zero();
                si.sn_prl_children = profile_zero();
                si.sn_prl_wait = profile_zero();
                si.sn_prl_idx = -1;
                si.sn_prl_execed = 0;
            }
        }

        let functbl = func_tbl_get();
        let mut todo = (*functbl).ht_used;
        let mut hi = (*functbl).ht_array;
        let uf_name_off = std::mem::offset_of!(ufunc_T, uf_name);

        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let uf = &mut *(*hi).hi_key.sub(uf_name_off).cast::<ufunc_T>();
                if uf.uf_prof_initialized != 0 {
                    uf.uf_profiling = 0;
                    uf.uf_tm_count = 0;
                    uf.uf_tm_total = profile_zero();
                    uf.uf_tm_self = profile_zero();
                    uf.uf_tm_children = profile_zero();

                    for i in 0..uf.uf_lines.ga_len {
                        let idx = i as usize;
                        *uf.uf_tml_count.add(idx) = 0;
                        *uf.uf_tml_total.add(idx) = 0;
                        *uf.uf_tml_self.add(idx) = 0;
                    }

                    uf.uf_tml_start = profile_zero();
                    uf.uf_tml_children = profile_zero();
                    uf.uf_tml_wait = profile_zero();
                    uf.uf_tml_idx = -1;
                    uf.uf_tml_execed = 0;
                }
            }
            hi = hi.add(1);
        }

        xfree(PROFILE_FNAME.cast::<c_void>());
        PROFILE_FNAME = std::ptr::null_mut();
    }
}

/// `":profile cmd args"` (`ex_profile`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_profile(eap: *mut exarg_T) {
    #[cfg(test)]
    let _ = eap;

    #[cfg(not(test))]
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
                PAUSE_TIME.store(profile_start(), Ordering::Relaxed);
            }
            do_profiling = PROF_PAUSED;
        } else if arg_bytes == b"continue" {
            if do_profiling == PROF_PAUSED {
                let elapsed = profile_end(PAUSE_TIME.load(Ordering::Relaxed));
                PAUSE_TIME.store(elapsed, Ordering::Relaxed);
                profile_set_wait(profile_add(profile_get_wait(), elapsed));
            }
            do_profiling = PROF_YES;
        } else if arg_bytes == b"dump" {
            profile_dump();
        } else {
            ex_breakadd(eap.cast::<exarg_Full>());
        }
    }
}

/// Command-line completion item getter for `:profile` (`get_profile_name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_profile_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if PEXPAND_WHAT.load(Ordering::Relaxed) == PEXP_SUBCMD
        && idx >= 0
        && (idx as usize) < PEXPAND_CMDS.len()
    {
        PEXPAND_CMDS[idx as usize].as_ptr().cast_mut()
    } else {
        std::ptr::null_mut()
    }
}

/// Set command-line completion context for `:profile` (`set_context_in_profile_cmd`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_in_profile_cmd(xp: *mut expand_T, arg: *const c_char) {
    let xp = unsafe { &mut *xp };
    xp.xp_context = EXPAND_PROFILE;
    PEXPAND_WHAT.store(PEXP_SUBCMD, Ordering::Relaxed);
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
#[unsafe(no_mangle)]
pub extern "C" fn prof_input_start() {
    WAIT_TIME.store(profile_start(), Ordering::Relaxed);
}

/// Called when finished waiting for user input (`prof_input_end`).
#[unsafe(no_mangle)]
pub extern "C" fn prof_input_end() {
    let elapsed = profile_end(WAIT_TIME.load(Ordering::Relaxed));
    WAIT_TIME.store(elapsed, Ordering::Relaxed);
    profile_set_wait(profile_add(profile_get_wait(), elapsed));
}

/// Returns `true` when a function defined in the current script should be profiled (`prof_def_func`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prof_def_func() -> bool {
    #[cfg(test)]
    {
        false
    }
    #[cfg(not(test))]
    unsafe {
        if current_sctx.sc_sid > 0 {
            (*script_item(current_sctx.sc_sid)).sn_pr_force
        } else {
            false
        }
    }
}

#[cfg(not(test))]
unsafe fn prof_func_line(
    fd: *mut FILE,
    count: c_int,
    total: &proftime_T,
    self_time: &proftime_T,
    prefer_self: bool,
) {
    unsafe {
        if count > 0 {
            fprintf(fd, c"%5d ".as_ptr(), count);
            if prefer_self && profile_equal(*total, *self_time) {
                fprintf(fd, c"           ".as_ptr());
            } else {
                fprintf(fd, c"%s ".as_ptr(), profile_msg(*total));
            }
            if !prefer_self && profile_equal(*total, *self_time) {
                fprintf(fd, c"           ".as_ptr());
            } else {
                fprintf(fd, c"%s ".as_ptr(), profile_msg(*self_time));
            }
        } else {
            fprintf(fd, c"                            ".as_ptr());
        }
    }
}

#[cfg(not(test))]
unsafe fn prof_sort_list(
    fd: *mut FILE,
    sorttab: *mut *mut ufunc_T,
    st_len: c_int,
    title: *const c_char,
    prefer_self: bool,
) {
    unsafe {
        fprintf(fd, c"FUNCTIONS SORTED ON %s TIME\n".as_ptr(), title);
        fprintf(fd, c"count  total (s)   self (s)  function\n".as_ptr());
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
                fprintf(fd, c" <SNR>%s()\n".as_ptr(), name_ptr.add(3));
            } else {
                fprintf(fd, c" %s()\n".as_ptr(), name_ptr);
            }
        }
        fprintf(fd, c"\n".as_ptr());
    }
}

#[cfg(not(test))]
unsafe extern "C" fn prof_total_cmp(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let p1 = &**s1.cast::<*const ufunc_T>();
        let p2 = &**s2.cast::<*const ufunc_T>();
        profile_cmp(p1.uf_tm_total, p2.uf_tm_total)
    }
}

#[cfg(not(test))]
unsafe extern "C" fn prof_self_cmp(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let p1 = &**s1.cast::<*const ufunc_T>();
        let p2 = &**s2.cast::<*const ufunc_T>();
        profile_cmp(p1.uf_tm_self, p2.uf_tm_self)
    }
}

/// Start profiling function `fp` (`func_do_profile`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn func_do_profile(fp: *mut ufunc_T) {
    let fp = unsafe { &mut *fp };
    let mut len = fp.uf_lines.ga_len;

    if fp.uf_prof_initialized == 0 {
        if len == 0 {
            len = 1;
        }
        fp.uf_tm_count = 0;
        fp.uf_tm_self = profile_zero();
        fp.uf_tm_total = profile_zero();

        let count_len = len as usize;
        unsafe {
            if fp.uf_tml_count.is_null() {
                fp.uf_tml_count = xcalloc(count_len, std::mem::size_of::<c_int>()).cast::<c_int>();
            }
            if fp.uf_tml_total.is_null() {
                fp.uf_tml_total =
                    xcalloc(count_len, std::mem::size_of::<proftime_T>()).cast::<proftime_T>();
            }
            if fp.uf_tml_self.is_null() {
                fp.uf_tml_self =
                    xcalloc(count_len, std::mem::size_of::<proftime_T>()).cast::<proftime_T>();
            }
        }

        fp.uf_tml_idx = -1;
        fp.uf_prof_initialized = 1;
    }

    fp.uf_profiling = 1;
}

/// Prepare profiling for entering a child (`prof_child_enter`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prof_child_enter(tm: *mut proftime_T) {
    #[cfg(not(test))]
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prof_child_exit(tm: *mut proftime_T) {
    #[cfg(not(test))]
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn func_line_start(cookie: *mut c_void) {
    #[cfg(test)]
    let _ = cookie;

    #[cfg(not(test))]
    unsafe {
        let fcp = cookie.cast::<funccall_T>();
        let fp = &mut *(*fcp).fc_func;
        let slnum = sourcing_lnum();

        if fp.uf_profiling != 0 && slnum >= 1 && slnum <= fp.uf_lines.ga_len {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn func_line_exec(cookie: *mut c_void) {
    let fcp = cookie.cast::<funccall_T>();
    let fp = unsafe { &mut *(*fcp).fc_func };
    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        fp.uf_tml_execed = 1;
    }
}

/// Called when done with a function line (`func_line_end`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn func_line_end(cookie: *mut c_void) {
    let fcp = cookie.cast::<funccall_T>();
    let fp = unsafe { &mut *(*fcp).fc_func };

    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        if fp.uf_tml_execed != 0 {
            let idx = fp.uf_tml_idx as usize;
            unsafe {
                *fp.uf_tml_count.add(idx) += 1;
                fp.uf_tml_start = profile_sub_wait(fp.uf_tml_wait, profile_end(fp.uf_tml_start));
                *fp.uf_tml_total.add(idx) = profile_add(*fp.uf_tml_total.add(idx), fp.uf_tml_start);
                *fp.uf_tml_self.add(idx) = profile_self(
                    *fp.uf_tml_self.add(idx),
                    fp.uf_tml_start,
                    fp.uf_tml_children,
                );
            }
        }
        fp.uf_tml_idx = -1;
    }
}

#[cfg(not(test))]
unsafe fn func_dump_profile(fd: *mut FILE) {
    unsafe {
        let functbl = func_tbl_get();
        let mut st_len: c_int = 0;
        let mut todo = (*functbl).ht_used as c_int;
        if todo == 0 {
            return;
        }

        let sorttab =
            xmalloc(std::mem::size_of::<*mut ufunc_T>() * (todo as usize)).cast::<*mut ufunc_T>();
        let uf_name_off = std::mem::offset_of!(ufunc_T, uf_name);
        let mut hi = (*functbl).ht_array;

        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let fp = (*hi).hi_key.sub(uf_name_off).cast::<ufunc_T>();
                if (*fp).uf_prof_initialized != 0 {
                    *sorttab.add(st_len as usize) = fp;
                    st_len += 1;

                    let name_ptr = (*fp).uf_name.as_ptr();
                    if (*name_ptr as u8) == K_SPECIAL {
                        fprintf(fd, c"FUNCTION  <SNR>%s()\n".as_ptr(), name_ptr.add(3));
                    } else {
                        fprintf(fd, c"FUNCTION  %s()\n".as_ptr(), name_ptr);
                    }
                    if (*fp).uf_script_ctx.sc_sid != 0 {
                        let mut should_free = false;
                        let p = get_scriptname((*fp).uf_script_ctx, &mut should_free);
                        fprintf(
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
                        fprintf(fd, c"Called 1 time\n".as_ptr());
                    } else {
                        fprintf(fd, c"Called %d times\n".as_ptr(), (*fp).uf_tm_count);
                    }
                    fprintf(
                        fd,
                        c"Total time: %s\n".as_ptr(),
                        profile_msg((*fp).uf_tm_total),
                    );
                    fprintf(
                        fd,
                        c" Self time: %s\n".as_ptr(),
                        profile_msg((*fp).uf_tm_self),
                    );
                    fprintf(fd, c"\n".as_ptr());
                    fprintf(fd, c"count  total (s)   self (s)\n".as_ptr());

                    for i in 0..(*fp).uf_lines.ga_len {
                        let line = funcline(fp, i);
                        if line.is_null() {
                            continue;
                        }
                        let idx = i as usize;
                        prof_func_line(
                            fd,
                            *(*fp).uf_tml_count.add(idx),
                            &*(*fp).uf_tml_total.add(idx),
                            &*(*fp).uf_tml_self.add(idx),
                            true,
                        );
                        fprintf(fd, c"%s\n".as_ptr(), line);
                    }
                    fprintf(fd, c"\n".as_ptr());
                }
            }
            hi = hi.add(1);
        }

        if st_len > 0 {
            qsort(
                sorttab.cast::<c_void>(),
                st_len as usize,
                std::mem::size_of::<*mut ufunc_T>(),
                prof_total_cmp,
            );
            prof_sort_list(fd, sorttab, st_len, c"TOTAL".as_ptr(), false);
            qsort(
                sorttab.cast::<c_void>(),
                st_len as usize,
                std::mem::size_of::<*mut ufunc_T>(),
                prof_self_cmp,
            );
            prof_sort_list(fd, sorttab, st_len, c"SELF".as_ptr(), true);
        }

        xfree(sorttab.cast::<c_void>());
    }
}

/// Start profiling a script (`profile_init`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn profile_init(si: *mut scriptitem_T) {
    let si = unsafe { &mut *si };
    si.sn_pr_count = 0;
    si.sn_pr_total = profile_zero();
    si.sn_pr_self = profile_zero();

    unsafe {
        ga_init(
            &mut si.sn_prl_ga,
            std::mem::size_of::<sn_prl_T>() as c_int,
            100,
        );
    }
    si.sn_prl_idx = -1;
    si.sn_prof_on = true;
    si.sn_pr_nest = 0;
}

/// Save time when starting to invoke another script or function (`script_prof_save`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_prof_save(tm: *mut proftime_T) {
    #[cfg(not(test))]
    unsafe {
        if current_sctx.sc_sid > 0 && current_sctx.sc_sid <= script_items.ga_len {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_prof_restore(tm: *const proftime_T) {
    #[cfg(test)]
    let _ = tm;

    #[cfg(not(test))]
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

#[cfg(not(test))]
unsafe fn script_dump_profile(fd: *mut FILE) {
    unsafe {
        let mut iobuf = [0 as c_char; IOSIZE];
        for id in 1..=script_items.ga_len {
            let si = script_item(id);
            if (*si).sn_prof_on {
                fprintf(fd, c"SCRIPT  %s\n".as_ptr(), (*si).sn_name);
                if (*si).sn_pr_count == 1 {
                    fprintf(fd, c"Sourced 1 time\n".as_ptr());
                } else {
                    fprintf(fd, c"Sourced %d times\n".as_ptr(), (*si).sn_pr_count);
                }
                fprintf(
                    fd,
                    c"Total time: %s\n".as_ptr(),
                    profile_msg((*si).sn_pr_total),
                );
                fprintf(
                    fd,
                    c" Self time: %s\n".as_ptr(),
                    profile_msg((*si).sn_pr_self),
                );
                fprintf(fd, c"\n".as_ptr());
                fprintf(fd, c"count  total (s)   self (s)\n".as_ptr());

                let sfd = os_fopen((*si).sn_name, c"r".as_ptr());
                if sfd.is_null() {
                    fprintf(fd, c"Cannot open file!\n".as_ptr());
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
                        if i < (*si).sn_prl_ga.ga_len {
                            let pp = &*prl_item(si, i);
                            if pp.snp_count > 0 {
                                fprintf(fd, c"%5d ".as_ptr(), pp.snp_count);
                                if profile_equal(pp.sn_prl_total, pp.sn_prl_self) {
                                    fprintf(fd, c"           ".as_ptr());
                                } else {
                                    fprintf(fd, c"%s ".as_ptr(), profile_msg(pp.sn_prl_total));
                                }
                                fprintf(fd, c"%s ".as_ptr(), profile_msg(pp.sn_prl_self));
                            } else {
                                fprintf(fd, c"                            ".as_ptr());
                            }
                        } else {
                            fprintf(fd, c"                            ".as_ptr());
                        }
                        fprintf(fd, c"%s".as_ptr(), iobuf.as_ptr());
                        i += 1;
                    }
                    fclose(sfd);
                }
                fprintf(fd, c"\n".as_ptr());
            }
        }
    }
}

/// Dump the profiling info (`profile_dump`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn profile_dump() {
    #[cfg(not(test))]
    unsafe {
        if PROFILE_FNAME.is_null() {
            return;
        }

        let fd = os_fopen(PROFILE_FNAME, c"w".as_ptr());
        if fd.is_null() {
            semsg!(c"E484: Can't open file %s".as_ptr(), PROFILE_FNAME);
        } else {
            script_dump_profile(fd);
            func_dump_profile(fd);
            fclose(fd);
        }
    }
}

/// Called when starting to read a script line (`script_line_start`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_line_start() {
    #[cfg(not(test))]
    unsafe {
        if current_sctx.sc_sid <= 0 || current_sctx.sc_sid > script_items.ga_len {
            return;
        }
        let si = script_item(current_sctx.sc_sid);
        let slnum = sourcing_lnum();
        if (*si).sn_prof_on && slnum >= 1 {
            ga_grow(&mut (*si).sn_prl_ga, slnum - (*si).sn_prl_ga.ga_len);
            (*si).sn_prl_idx = slnum - 1;
            while (*si).sn_prl_ga.ga_len <= (*si).sn_prl_idx
                && (*si).sn_prl_ga.ga_len < (*si).sn_prl_ga.ga_maxlen
            {
                let pp = &mut *prl_item(si, (*si).sn_prl_ga.ga_len);
                pp.snp_count = 0;
                pp.sn_prl_total = profile_zero();
                pp.sn_prl_self = profile_zero();
                (*si).sn_prl_ga.ga_len += 1;
            }
            (*si).sn_prl_execed = 0;
            (*si).sn_prl_start = profile_start();
            (*si).sn_prl_children = profile_zero();
            (*si).sn_prl_wait = profile_get_wait();
        }
    }
}

/// Called when actually executing a script line (`script_line_exec`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_line_exec() {
    #[cfg(not(test))]
    unsafe {
        if current_sctx.sc_sid <= 0 || current_sctx.sc_sid > script_items.ga_len {
            return;
        }
        let si = &mut *script_item(current_sctx.sc_sid);
        if si.sn_prof_on && si.sn_prl_idx >= 0 {
            si.sn_prl_execed = 1;
        }
    }
}

/// Called when done with a script line (`script_line_end`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_line_end() {
    #[cfg(not(test))]
    unsafe {
        if current_sctx.sc_sid <= 0 || current_sctx.sc_sid > script_items.ga_len {
            return;
        }
        let si = script_item(current_sctx.sc_sid);
        if (*si).sn_prof_on && (*si).sn_prl_idx >= 0 && (*si).sn_prl_idx < (*si).sn_prl_ga.ga_len {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_push(rel: *mut proftime_T, start: *mut proftime_T) {
    let now = profile_start();
    let prev = G_PREV_TIME.swap(now, Ordering::Relaxed);
    unsafe {
        *rel = profile_sub(now, prev);
        *start = now;
    }
}

/// Computes the prev time after doing something that could nest (`time_pop`).
#[unsafe(no_mangle)]
pub extern "C" fn time_pop(tp: proftime_T) {
    let prev = G_PREV_TIME.load(Ordering::Relaxed);
    G_PREV_TIME.store(prev.wrapping_sub(tp), Ordering::Relaxed);
}

#[cfg(not(test))]
unsafe fn time_diff(then: proftime_T, now: proftime_T) {
    let diff = profile_sub(now, then);
    let s = format!("{:07.3}\0", (diff as f64) / 1.0e6);
    unsafe {
        fprintf(time_fd, c"%s".as_ptr(), s.as_ptr().cast::<c_char>());
    }
}

/// Initializes the startuptime code (`time_start`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_start(message: *const c_char) {
    #[cfg(test)]
    let _ = message;

    #[cfg(not(test))]
    unsafe {
        if time_fd.is_null() {
            return;
        }

        let now = profile_start();
        G_START_TIME.store(now, Ordering::Relaxed);
        G_PREV_TIME.store(now, Ordering::Relaxed);

        fprintf(time_fd, c"\ntimes in msec\n".as_ptr());
        fprintf(
            time_fd,
            c" clock   self+sourced   self:  sourced script\n".as_ptr(),
        );
        fprintf(
            time_fd,
            c" clock   elapsed:              other lines\n\n".as_ptr(),
        );

        time_msg(message, std::ptr::null());
    }
}

/// Prints out timing info (`time_msg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_msg(mesg: *const c_char, start: *const proftime_T) {
    #[cfg(test)]
    let _ = (mesg, start);

    #[cfg(not(test))]
    unsafe {
        if time_fd.is_null() {
            return;
        }

        let now = profile_start();
        time_diff(G_START_TIME.load(Ordering::Relaxed), now);

        if !start.is_null() {
            fprintf(time_fd, c"  ".as_ptr());
            time_diff(*start, now);
        }

        fprintf(time_fd, c"  ".as_ptr());
        time_diff(G_PREV_TIME.swap(now, Ordering::Relaxed), now);

        fprintf(time_fd, c": %s\n".as_ptr(), mesg);
    }
}

/// Initializes the `time_fd` stream for `--startuptime` (`time_init`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_init(fname: *const c_char, proc_name: *const c_char) {
    #[cfg(test)]
    let _ = (fname, proc_name);

    #[cfg(not(test))]
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
        fprintf(
            time_fd,
            c"--- Startup times for process: %s ---\n".as_ptr(),
            proc_name,
        );
    }
}

/// Flushes the startuptimes to disk for the current process (`time_finish`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time_finish() {
    #[cfg(not(test))]
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
        unsafe {
            let msg = CStr::from_ptr(profile_msg(0)).to_str().unwrap();
            assert_eq!(msg.trim(), "0.000000");
        }
    }
}
