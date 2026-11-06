//! Script, function, and startup profiling (`src/nvim/profile.c`).

use crate::debugger::ex_breakadd;
use crate::errors::E_NOTOPEN;
use crate::eval::typval::{Dictitem, Listitem, TvDict, TvList, TypVal};
use crate::eval::userfunc::{DeferCall, UserFunc, func_tbl_get, get_current_funccal};
use crate::eval::vars::{VV_PROFILING, set_vim_var_nr};
use crate::event::r#loop::uv_err_name;
use crate::ex_docmd::Exarg;
use crate::fileio::{fprintf, vim_fgets};
use crate::globals::*;
use crate::message::{emsg, semsg};
use crate::os::env::{ExpandCtx, expand_env_save_opt};
use crate::os::fs::CFile;
use crate::os::time::os_hrtime;
use crate::runtime::{get_scriptname, script_id_valid, script_item, sourcing_lnum};
use std::ffi::{CStr, CString};

pub type Proftime = u64;

const NS_PER_SEC: f64 = 1_000_000_000.0;
const PROF_PAUSED: i32 = 2;

const IOFBF: i32 = 0;

const PEXP_SUBCMD: i32 = 0;

struct ProfileModuleState {
    prof_wait_time: Proftime,
    wait_time: Proftime,
    pause_time: Proftime,
    g_start_time: Proftime,
    g_prev_time: Proftime,
    pexpand_what: i32,
    startuptime_buf: Vec<u8>,
    profile_fname: Option<CString>,
    profile_msg_buf: [u8; 50],
}

impl ProfileModuleState {
    const fn new() -> Self {
        Self {
            prof_wait_time: 0,
            wait_time: 0,
            pause_time: 0,
            g_start_time: 0,
            g_prev_time: 0,
            pexpand_what: PEXP_SUBCMD,
            startuptime_buf: Vec::new(),
            profile_fname: None,
            profile_msg_buf: [0; 50],
        }
    }
}

static PROF_STATE: GlobalState<ProfileModuleState> = GlobalState::new(ProfileModuleState::new());

#[inline]
fn prof_st() -> crate::globals::GraphPtr<ProfileModuleState> {
    PROF_STATE.handle()
}

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
pub struct SnPrl {
    pub snp_count: i32,
    pub sn_prl_total: Proftime,
    pub sn_prl_self: Proftime,
}

#[repr(C)]
pub struct Scriptitem {
    pub sn_vars: Box<crate::runtime::Scriptvar>,
    pub sn_name: Option<std::ffi::CString>,
    pub sn_lua: bool,
    pub sn_prof_on: bool,
    pub sn_pr_force: bool,
    pub sn_pr_child: Proftime,
    pub sn_pr_nest: i32,
    pub sn_pr_count: i32,
    pub sn_pr_total: Proftime,
    pub sn_pr_self: Proftime,
    pub sn_pr_start: Proftime,
    pub sn_pr_children: Proftime,
    pub sn_prl_ga: Vec<SnPrl>,
    pub sn_prl_start: Proftime,
    pub sn_prl_children: Proftime,
    pub sn_prl_wait: Proftime,
    pub sn_prl_idx: i32,
    pub sn_prl_execed: i32,
}

impl Scriptitem {
    #[inline]
    pub fn name_cstr(&self) -> Option<&CStr> {
        self.sn_name.as_deref()
    }
}

use crate::autocmd::PROF_YES;
use crate::charset::NL;
use crate::cmdexpand::{EXPAND_FILES, EXPAND_NOTHING, EXPAND_PROFILE, EXPAND_USER_FUNC};
use crate::eval::funcs::MAX_FUNC_ARGS;
use crate::globals::{IOSIZE, PROF_NONE};
use crate::keycodes::K_SPECIAL;

#[derive(Clone)]
pub enum EStackInfo {
    None,
    Ufunc(crate::globals::GraphPtr<UserFunc>),
    AuCmd(std::rc::Rc<std::cell::Cell<crate::runtime::Sctx>>),
}

impl EStackInfo {
    #[inline]
    pub fn ufunc(&self) -> Option<&'static mut UserFunc> {
        match *self {
            Self::Ufunc(p) => p.opt_mut(),
            _ => None,
        }
    }

    #[inline]
    pub fn aucmd_sctx(&self) -> crate::runtime::Sctx {
        match self {
            Self::AuCmd(sctx) => sctx.get(),
            _ => crate::runtime::Sctx::new(),
        }
    }
}

#[derive(Clone)]
pub struct Estack {
    pub es_lnum: i32,
    pub es_name: Option<std::ffi::CString>,
    pub es_type: i32,
    pub es_info: EStackInfo,
}

impl Estack {
    #[inline]
    pub fn name_cstr(&self) -> Option<&CStr> {
        self.es_name.as_deref()
    }
}

#[repr(C)]
pub struct Funccall {
    pub fc_func: crate::globals::GraphPtr<UserFunc>,
    pub fc_linenr: i32,
    pub fc_returned: i32,
    pub fc_l_vars: TvDict,
    pub fc_l_vars_var: Dictitem,
    pub fc_l_avars: TvDict,
    pub fc_l_avars_var: Dictitem,
    pub fc_l_varlist: TvList,
    pub fc_l_listitems: [Listitem; MAX_FUNC_ARGS],
    pub fc_rettv: crate::globals::GraphPtr<TypVal>,
    pub fc_breakpoint: i32,
    pub fc_dbg_tick: i32,
    pub fc_level: i32,
    pub fc_defer: Vec<DeferCall>,
    pub fc_prof_child: Proftime,
    pub fc_caller: crate::globals::GraphPtr<Funccall>,
    pub fc_refcount: i32,
    pub fc_copy_id: i32,
    pub fc_ufuncs: Vec<crate::globals::GraphPtr<UserFunc>>,
}

impl Funccall {
    pub const fn new() -> Self {
        Self {
            fc_func: crate::globals::GraphPtr::NULL,
            fc_linenr: 0,
            fc_returned: 0,
            fc_l_vars: TvDict::new(),
            fc_l_vars_var: Dictitem::new(),
            fc_l_avars: TvDict::new(),
            fc_l_avars_var: Dictitem::new(),
            fc_l_varlist: TvList::new(),
            fc_l_listitems: [const { Listitem::new() }; MAX_FUNC_ARGS],
            fc_rettv: crate::globals::GraphPtr::NULL,
            fc_breakpoint: 0,
            fc_dbg_tick: 0,
            fc_level: 0,
            fc_defer: Vec::new(),
            fc_prof_child: 0,
            fc_caller: crate::globals::GraphPtr::NULL,
            fc_refcount: 0,
            fc_copy_id: 0,
            fc_ufuncs: Vec::new(),
        }
    }

    #[inline]
    pub fn func_mut(&self) -> &'static mut UserFunc {
        self.fc_func.borrow_mut()
    }

    #[inline]
    pub fn rettv_mut(&self) -> &'static mut TypVal {
        self.fc_rettv.borrow_mut()
    }

    #[inline]
    pub fn caller_opt(&self) -> Option<&'static mut Funccall> {
        self.fc_caller.opt_mut()
    }
}

/// Gets the current time (`profile_start`).
pub fn profile_start() -> Proftime {
    os_hrtime()
}

/// Computes the time elapsed (`profile_end`).
pub fn profile_end(tm: Proftime) -> Proftime {
    profile_sub(os_hrtime(), tm)
}

/// Gets a static string representing time `tm` (`profile_msg`).
pub fn profile_msg(tm: Proftime) -> &'static CStr {
    let secs = (profile_signed(tm) as f64) / NS_PER_SEC;
    let formatted = format!("{secs:10.6}");
    let bytes = formatted.as_bytes();
    {
        let buf = &mut prof_st().borrow_mut().profile_msg_buf;
        let copy_len = bytes.len().min(buf.len() - 1);
        buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
        buf[copy_len] = 0;
        CStr::from_bytes_until_nul(buf).unwrap_or(c"")
    }
}

/// Gets the time `msec` into the future (`profile_setlimit`).
pub fn profile_setlimit(msec: i64) -> Proftime {
    if msec <= 0 {
        return profile_zero();
    }
    assert!(msec < (i64::MAX / 1_000_000));
    let nsec = (msec as Proftime) * 1_000_000;
    os_hrtime().wrapping_add(nsec)
}

/// Checks if current time has passed `tm` (`profile_passed_limit`).
pub fn profile_passed_limit(tm: Proftime) -> bool {
    if tm == 0 {
        return false;
    }
    profile_cmp(os_hrtime(), tm) < 0
}

/// Gets the zero time (`profile_zero`).
pub const fn profile_zero() -> Proftime {
    0
}

/// Divides time `tm` by `count` (`profile_divide`).
pub fn profile_divide(tm: Proftime, count: i32) -> Proftime {
    if count <= 0 {
        return profile_zero();
    }
    ((tm as f64) / (count as f64)).round() as Proftime
}

/// Adds time `tm2` to `tm1` (`profile_add`).
pub const fn profile_add(tm1: Proftime, tm2: Proftime) -> Proftime {
    tm1.wrapping_add(tm2)
}

/// Subtracts time `tm2` from `tm1` (`profile_sub`).
pub const fn profile_sub(tm1: Proftime, tm2: Proftime) -> Proftime {
    tm1.wrapping_sub(tm2)
}

/// Adds the `self` time from the total time and the `children` time (`profile_self`).
pub const fn profile_self(self_time: Proftime, total: Proftime, children: Proftime) -> Proftime {
    if total <= children {
        return self_time;
    }
    profile_sub(profile_add(self_time, total), children)
}

#[inline]
fn profile_get_wait() -> Proftime {
    prof_st().prof_wait_time
}

/// Sets the current waittime (`profile_set_wait`).
pub fn profile_set_wait(wait: Proftime) {
    prof_st().prof_wait_time = wait;
}

/// Subtracts the passed waittime since `tm` (`profile_sub_wait`).
pub fn profile_sub_wait(tm: Proftime, tma: Proftime) -> Proftime {
    let tm3 = profile_sub(profile_get_wait(), tm);
    profile_sub(tma, tm3)
}

#[inline]
const fn profile_equal(tm1: Proftime, tm2: Proftime) -> bool {
    tm1 == tm2
}

/// Converts time duration `tm` to a signed integer (`profile_signed`).
pub const fn profile_signed(tm: Proftime) -> i64 {
    tm as i64
}

/// Compares profiling times (`profile_cmp`).
pub const fn profile_cmp(tm1: Proftime, tm2: Proftime) -> i32 {
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
pub fn profile_reset() {
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
    for &fp in functbl.values() {
        let uf = crate::globals::GraphPtr::of(fp).borrow_mut();
        if uf.uf_prof_initialized {
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

    prof_st().profile_fname = None;
}

/// `":profile cmd args"` (`ex_profile`).
pub fn ex_profile(eap: &mut Exarg) {
    let arg = eap.arg;
    let mut e = arg.skiptowhite();
    let len = e.offset_from(arg);
    e = e.skipwhite();

    let arg_bytes = arg.as_bytes();
    if len == 5 && &arg_bytes[..5] == b"start" && !e.is_empty() {
        prof_st().profile_fname = Some(expand_env_save_opt(e.as_cstr(), true, None));
        g_st().do_profiling = PROF_YES;
        profile_set_wait(profile_zero());
        set_vim_var_nr(VV_PROFILING, 1);
    } else if g_st().do_profiling == PROF_NONE {
        emsg(c"E750: First use \":profile start {fname}\"");
    } else if arg_bytes == b"stop" {
        profile_dump();
        g_st().do_profiling = PROF_NONE;
        set_vim_var_nr(VV_PROFILING, 0);
        profile_reset();
    } else if arg_bytes == b"pause" {
        if g_st().do_profiling == PROF_YES {
            prof_st().pause_time = profile_start();
        }
        g_st().do_profiling = PROF_PAUSED;
    } else if arg_bytes == b"continue" {
        if g_st().do_profiling == PROF_PAUSED {
            let elapsed = profile_end(prof_st().pause_time);
            prof_st().pause_time = elapsed;
            profile_set_wait(profile_add(profile_get_wait(), elapsed));
        }
        g_st().do_profiling = PROF_YES;
    } else if arg_bytes == b"dump" {
        profile_dump();
    } else {
        ex_breakadd(eap);
    }
}

/// Command-line completion item getter for `:profile` (`get_profile_name`).
pub fn get_profile_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if prof_st().pexpand_what == PEXP_SUBCMD {
        usize::try_from(idx)
            .ok()
            .and_then(|i| PEXPAND_CMDS.get(i).copied())
    } else {
        None
    }
}

/// Set command-line completion context for `:profile` (`set_context_in_profile_cmd`).
pub fn set_context_in_profile_cmd(xp: &mut ExpandCtx, arg_slice: &[u8]) {
    let arg = crate::strings::ByteSlice::from_bytes(arg_slice);
    xp.xp_context = EXPAND_PROFILE;
    prof_st().pexpand_what = PEXP_SUBCMD;
    xp.xp_pattern = arg;

    let end_subcmd = arg.skiptowhite();
    if end_subcmd.is_empty() {
        return;
    }

    let len = end_subcmd.offset_from(arg);
    let sub_bytes = &arg_slice[..len.min(arg_slice.len())];
    if sub_bytes == b"start" || sub_bytes == b"file" {
        xp.xp_context = EXPAND_FILES;
        xp.xp_pattern = end_subcmd.skipwhite();
    } else if sub_bytes == b"func" {
        xp.xp_context = EXPAND_USER_FUNC;
        xp.xp_pattern = end_subcmd.skipwhite();
    } else {
        xp.xp_context = EXPAND_NOTHING;
    }
}

/// Called when starting to wait for user input (`prof_input_start`).
pub fn prof_input_start() {
    prof_st().wait_time = profile_start();
}

/// Called when finished waiting for user input (`prof_input_end`).
pub fn prof_input_end() {
    let elapsed = profile_end(prof_st().wait_time);
    prof_st().wait_time = elapsed;
    profile_set_wait(profile_add(profile_get_wait(), elapsed));
}

/// Returns `true` when a function defined in the current script should be profiled (`prof_def_func`).
pub fn prof_def_func() -> bool {
    if g_st().current_sctx.sc_sid > 0 {
        script_item(g_st().current_sctx.sc_sid).sn_pr_force
    } else {
        false
    }
}

fn prof_func_line(
    fd: &CFile,
    count: i32,
    total: &Proftime,
    self_time: &Proftime,
    prefer_self: bool,
) {
    if count > 0 {
        fprintf!(fd, c"%5d ", count);
        if prefer_self && profile_equal(*total, *self_time) {
            fprintf!(fd, c"           ");
        } else {
            fprintf!(
                fd,
                c"%s ",
                crate::strings::ByteSlice::from_ptr(profile_msg(*total).as_ptr())
            );
        }
        if !prefer_self && profile_equal(*total, *self_time) {
            fprintf!(fd, c"           ");
        } else {
            fprintf!(
                fd,
                c"%s ",
                crate::strings::ByteSlice::from_ptr(profile_msg(*self_time).as_ptr())
            );
        }
    } else {
        fprintf!(fd, c"                            ");
    }
}

fn prof_sort_list(fd: &CFile, sorttab: &[&UserFunc], title: &std::ffi::CStr, prefer_self: bool) {
    fprintf!(fd, c"FUNCTIONS SORTED ON %s TIME\n", title);
    fprintf!(fd, c"count  total (s)   self (s)  function\n");
    for &fp in sorttab.iter().take(20) {
        prof_func_line(
            fd,
            fp.uf_tm_count,
            &fp.uf_tm_total,
            &fp.uf_tm_self,
            prefer_self,
        );
        let name_ptr = crate::strings::ByteSlice::from_cstr(&fp.uf_name);
        if (name_ptr.first_byte()) == K_SPECIAL {
            fprintf!(fd, c" <SNR>%s()\n", name_ptr.wrapping_add(3));
        } else {
            fprintf!(fd, c" %s()\n", name_ptr);
        }
    }
    fprintf!(fd, c"\n");
}

fn prof_total_cmp(s1: &&UserFunc, s2: &&UserFunc) -> std::cmp::Ordering {
    s2.uf_tm_total.cmp(&s1.uf_tm_total)
}

fn prof_self_cmp(s1: &&UserFunc, s2: &&UserFunc) -> std::cmp::Ordering {
    s2.uf_tm_self.cmp(&s1.uf_tm_self)
}

/// Start profiling function `fp` (`func_do_profile`).
pub fn func_do_profile(fp: &mut UserFunc) {
    let mut len = fp.uf_lines.len() as i32;

    if !fp.uf_prof_initialized {
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
        fp.uf_prof_initialized = true;
    }

    fp.uf_profiling = 1;
}

/// Prepare profiling for entering a child (`prof_child_enter`).
pub fn prof_child_enter(tm: &mut Proftime) {
    if let Some(fc) = get_current_funccal()
        && fc.func_mut().uf_profiling != 0
    {
        fc.fc_prof_child = profile_start();
    }
    script_prof_save(tm);
}

/// Take care of time spent in a child (`prof_child_exit`).
pub fn prof_child_exit(tm: &mut Proftime) {
    if let Some(fc) = get_current_funccal()
        && fc.func_mut().uf_profiling != 0
    {
        let child_elapsed = profile_sub_wait(*tm, profile_end(fc.fc_prof_child));
        fc.fc_prof_child = child_elapsed;
        let func = fc.func_mut();
        func.uf_tm_children = profile_add(func.uf_tm_children, child_elapsed);
        func.uf_tml_children = profile_add(func.uf_tml_children, child_elapsed);
    }
    script_prof_restore(tm);
}

/// Called when starting to read a function line (`func_line_start`).
pub fn func_line_start(fcp: &mut Funccall) {
    let fp = fcp.func_mut();
    let slnum = sourcing_lnum();

    if fp.uf_profiling != 0 && slnum >= 1 && slnum <= fp.uf_lines.len() as i32 {
        fp.uf_tml_idx = slnum - 1;
        while fp.uf_tml_idx > 0 && fp.uf_lines[fp.uf_tml_idx as usize].is_none() {
            fp.uf_tml_idx -= 1;
        }
        fp.uf_tml_execed = 0;
        fp.uf_tml_start = profile_start();
        fp.uf_tml_children = profile_zero();
        fp.uf_tml_wait = profile_get_wait();
    }
}

/// Called when actually executing a function line (`func_line_exec`).
pub fn func_line_exec(fcp: &mut Funccall) {
    let fp = fcp.func_mut();
    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        fp.uf_tml_execed = 1;
    }
}

/// Called when done with a function line (`func_line_end`).
pub fn func_line_end(fcp: &mut Funccall) {
    let fp = fcp.func_mut();

    if fp.uf_profiling != 0 && fp.uf_tml_idx >= 0 {
        if fp.uf_tml_execed != 0 {
            let idx = fp.uf_tml_idx as usize;
            fp.uf_tml_count[idx] += 1;
            fp.uf_tml_start = profile_sub_wait(fp.uf_tml_wait, profile_end(fp.uf_tml_start));
            fp.uf_tml_total[idx] = profile_add(fp.uf_tml_total[idx], fp.uf_tml_start);
            fp.uf_tml_self[idx] =
                profile_self(fp.uf_tml_self[idx], fp.uf_tml_start, fp.uf_tml_children);
        }
        fp.uf_tml_idx = -1;
    }
}

fn func_dump_profile(fd: &CFile) {
    let functbl = func_tbl_get();
    let todo = functbl.len();
    if todo == 0 {
        return;
    }

    let mut sorttab: Vec<&UserFunc> = Vec::with_capacity(todo);

    for &fp_ptr in functbl.values() {
        let fp = crate::globals::GraphPtr::of(fp_ptr).borrow();
        if fp.uf_prof_initialized {
            sorttab.push(fp);

            let name_ptr = crate::strings::ByteSlice::from_cstr(&fp.uf_name);
            if (name_ptr.first_byte()) == K_SPECIAL {
                fprintf!(fd, c"FUNCTION  <SNR>%s()\n", name_ptr.wrapping_add(3));
            } else {
                fprintf!(fd, c"FUNCTION  %s()\n", name_ptr);
            }
            if fp.uf_script_ctx.sc_sid != 0 {
                let p = get_scriptname(fp.uf_script_ctx, true);

                fprintf!(fd, c"    Defined: %s:%d\n", &p, fp.uf_script_ctx.sc_lnum,);
            }
            if fp.uf_tm_count == 1 {
                fprintf!(fd, c"Called 1 time\n");
            } else {
                fprintf!(fd, c"Called %d times\n", fp.uf_tm_count);
            }

            fprintf!(fd, c"Total time: %s\n", profile_msg(fp.uf_tm_total));

            fprintf!(fd, c" Self time: %s\n", profile_msg(fp.uf_tm_self));

            fprintf!(fd, c"\n");
            fprintf!(fd, c"count  total (s)   self (s)\n");

            for (idx, line_opt) in fp.uf_lines.iter().enumerate() {
                let Some(ref line) = *line_opt else {
                    continue;
                };

                prof_func_line(
                    fd,
                    fp.uf_tml_count.as_slice()[idx],
                    &fp.uf_tml_total.as_slice()[idx],
                    &fp.uf_tml_self.as_slice()[idx],
                    true,
                );

                fprintf!(fd, c"%s\n", line);
            }
            fprintf!(fd, c"\n");
        }
    }

    if !sorttab.is_empty() {
        sorttab.sort_unstable_by(prof_total_cmp);
        prof_sort_list(fd, &sorttab, c"TOTAL", false);
        sorttab.sort_unstable_by(prof_self_cmp);
        prof_sort_list(fd, &sorttab, c"SELF", true);
    }
}

/// Start profiling a script (`profile_init`).
pub fn profile_init(si: &mut Scriptitem) {
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
pub fn script_prof_save(tm: &mut Proftime) {
    {
        if script_id_valid(g_st().current_sctx.sc_sid) {
            let si = &mut *script_item(g_st().current_sctx.sc_sid);
            if si.sn_prof_on {
                let prev_nest = si.sn_pr_nest;
                si.sn_pr_nest += 1;
                if prev_nest == 0 {
                    si.sn_pr_child = profile_start();
                }
            }
        }
    }
    {
        *tm = profile_get_wait();
    }
}

/// Count time spent in children after invoking another script or function (`script_prof_restore`).
pub fn script_prof_restore(tm: &Proftime) {
    if !script_id_valid(g_st().current_sctx.sc_sid) {
        return;
    }

    let si = &mut *script_item(g_st().current_sctx.sc_sid);
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

fn script_dump_profile(fd: &CFile) {
    let mut iobuf = [0u8; IOSIZE];
    for id in 1..=crate::runtime::script_items_len() {
        let si = script_item(id);
        if si.sn_prof_on {
            let si_name = crate::strings::ByteSlice::from_opt_cstr(si.name_cstr());
            fprintf!(fd, c"SCRIPT  %s\n", si_name);
            if si.sn_pr_count == 1 {
                fprintf!(fd, c"Sourced 1 time\n");
            } else {
                fprintf!(fd, c"Sourced %d times\n", si.sn_pr_count);
            }
            fprintf!(fd, c"Total time: %s\n", profile_msg(si.sn_pr_total));
            fprintf!(fd, c" Self time: %s\n", profile_msg(si.sn_pr_self));
            fprintf!(fd, c"\n");
            fprintf!(fd, c"count  total (s)   self (s)\n");

            if let Some(sfd) = CFile::open(si_name.as_cstr(), c"r") {
                let mut i: i32 = 0;
                loop {
                    if vim_fgets(&mut iobuf, &sfd) {
                        break;
                    }
                    if iobuf[IOSIZE - 2] != 0 && iobuf[IOSIZE - 2] != NL as u8 {
                        let mut n = IOSIZE - 2;
                        while n > 0 && (iobuf[n] & 0xc0) == 0x80 {
                            n -= 1;
                        }
                        iobuf[n] = NL as u8;
                        iobuf[n + 1] = 0;
                    }
                    if (i as usize) < si.sn_prl_ga.len() {
                        let pp = &(&si.sn_prl_ga)[i as usize];
                        if pp.snp_count > 0 {
                            fprintf!(fd, c"%5d ", pp.snp_count);
                            if profile_equal(pp.sn_prl_total, pp.sn_prl_self) {
                                fprintf!(fd, c"           ");
                            } else {
                                fprintf!(fd, c"%s ", profile_msg(pp.sn_prl_total));
                            }
                            fprintf!(fd, c"%s ", profile_msg(pp.sn_prl_self));
                        } else {
                            fprintf!(fd, c"                            ");
                        }
                    } else {
                        fprintf!(fd, c"                            ");
                    }
                    if let Ok(line_cstr) = CStr::from_bytes_until_nul(&iobuf) {
                        fprintf!(fd, c"%s", line_cstr);
                    }
                    i += 1;
                }
            } else {
                fprintf!(fd, c"Cannot open file!\n");
            }
            fprintf!(fd, c"\n");
        }
    }
}

/// Dump the profiling info (`profile_dump`).
pub fn profile_dump() {
    let Some(ref profile_fname) = prof_st().profile_fname else {
        return;
    };

    if let Some(fd) = CFile::open(profile_fname, c"w") {
        script_dump_profile(&fd);
        func_dump_profile(&fd);
    } else {
        semsg!(E_NOTOPEN, profile_fname);
    }
}

/// Called when starting to read a script line (`script_line_start`).
pub fn script_line_start() {
    if !script_id_valid(g_st().current_sctx.sc_sid) {
        return;
    }
    let si = script_item(g_st().current_sctx.sc_sid);
    let slnum = sourcing_lnum();
    if si.sn_prof_on && slnum >= 1 {
        si.sn_prl_idx = slnum - 1;
        if si.sn_prl_ga.len() < slnum as usize {
            si.sn_prl_ga.resize(
                slnum as usize,
                SnPrl {
                    snp_count: 0,
                    sn_prl_total: profile_zero(),
                    sn_prl_self: profile_zero(),
                },
            );
        }
        si.sn_prl_execed = 0;
        si.sn_prl_start = profile_start();
        si.sn_prl_children = profile_zero();
        si.sn_prl_wait = profile_get_wait();
    }
}

/// Called when actually executing a script line (`script_line_exec`).
pub fn script_line_exec() {
    if !script_id_valid(g_st().current_sctx.sc_sid) {
        return;
    }
    let si = &mut *script_item(g_st().current_sctx.sc_sid);
    if si.sn_prof_on && si.sn_prl_idx >= 0 {
        si.sn_prl_execed = 1;
    }
}

/// Called when done with a script line (`script_line_end`).
pub fn script_line_end() {
    if !script_id_valid(g_st().current_sctx.sc_sid) {
        return;
    }
    let si = script_item(g_st().current_sctx.sc_sid);
    if si.sn_prof_on && si.sn_prl_idx >= 0 && (si.sn_prl_idx as usize) < si.sn_prl_ga.len() {
        if si.sn_prl_execed != 0 {
            let prl_idx = si.sn_prl_idx as usize;
            let pp = &mut (&mut si.sn_prl_ga)[prl_idx];
            pp.snp_count += 1;
            si.sn_prl_start = profile_sub_wait(si.sn_prl_wait, profile_end(si.sn_prl_start));
            pp.sn_prl_total = profile_add(pp.sn_prl_total, si.sn_prl_start);
            pp.sn_prl_self = profile_self(pp.sn_prl_self, si.sn_prl_start, si.sn_prl_children);
        }
        si.sn_prl_idx = -1;
    }
}

/// Saves the previous time before doing something that could nest (`time_push`).
pub fn time_push(rel: &mut Proftime, start: &mut Proftime) {
    let now = profile_start();
    {
        let prev = prof_st().g_prev_time;
        prof_st().g_prev_time = now;
        *rel = profile_sub(now, prev);
        *start = now;
    }
}

/// Computes the prev time after doing something that could nest (`time_pop`).
pub fn time_pop(tp: Proftime) {
    prof_st().g_prev_time = prof_st().g_prev_time.wrapping_sub(tp);
}

fn time_diff(then: Proftime, now: Proftime) {
    let diff = profile_sub(now, then);
    let s = CString::new(format!("{:07.3}", (diff as f64) / 1.0e6)).unwrap();
    if let Some(ref tf) = g_st().time_fd {
        fprintf!(tf, c"%s", &s);
    }
}

/// Initializes the startuptime code (`time_start`).
pub fn time_start(message: &std::ffi::CStr) {
    let Some(ref tf) = g_st().time_fd else {
        return;
    };

    let now = profile_start();
    prof_st().g_start_time = now;
    prof_st().g_prev_time = now;

    fprintf!(tf, c"\ntimes in msec\n");
    fprintf!(tf, c" clock   self+sourced   self:  sourced script\n",);
    fprintf!(tf, c" clock   elapsed:              other lines\n\n",);

    time_msg(message, None);
}

/// Prints out timing info (`time_msg`).
pub fn time_msg(mesg: &CStr, start: Option<&Proftime>) {
    let Some(ref tf) = g_st().time_fd else {
        return;
    };

    let now = profile_start();
    time_diff(prof_st().g_start_time, now);

    if let Some(start) = start {
        fprintf!(tf, c"  ");
        time_diff(*start, now);
    }

    fprintf!(tf, c"  ");
    let prev = prof_st().g_prev_time;
    prof_st().g_prev_time = now;
    time_diff(prev, now);

    fprintf!(tf, c": %s\n", mesg);
}

/// Initializes the `time_fd` stream for `--startuptime` (`time_init`).
pub fn time_init(fname: &CStr, proc_name: &CStr) {
    const BUFSIZE: usize = 8192;
    g_st().time_fd = CFile::open_libc(fname, c"a");
    let Some(ref tf) = g_st().time_fd else {
        let fname_str = fname.to_string_lossy();
        eprint!("E484: Can't open file {fname_str}");
        return;
    };
    prof_st().startuptime_buf = vec![0; BUFSIZE + 1];
    let r = tf.setvbuf(&mut prof_st().startuptime_buf, IOFBF);
    if r != 0 {
        prof_st().startuptime_buf = Vec::new();
        g_st().time_fd = None;
        let err_str = crate::strings::ByteSlice::from_ptr(uv_err_name(r))
            .as_cstr()
            .to_string_lossy();
        eprint!("time_init: setvbuf failed: {r} {err_str}");
        return;
    }
    fprintf!(tf, c"--- Startup times for process: %s ---\n", proc_name,);
}

/// Flushes the startuptimes to disk for the current process (`time_finish`).
pub fn time_finish() {
    if !has_time_fd() {
        return;
    }
    assert!(!prof_st().startuptime_buf.is_empty());
    time_msg(c"--- NVIM STARTED ---\n", None);

    g_st().time_fd = None;

    prof_st().startuptime_buf = Vec::new();
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
        assert_eq!(profile_cmp(10, 11), 1);
        assert_eq!(profile_cmp(11, 10), -1);
        assert_eq!(profile_cmp(10, 10), 0);
        assert_eq!(profile_cmp(profile_zero(), profile_zero()), 0);
        {
            let msg = profile_msg(0).to_str().unwrap();
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
        assert!(profile_passed_limit(now.saturating_sub(2)));
        assert!(!profile_passed_limit(profile_add(now, now)));

        {
            let endt = profile_end(profile_start());
            let s = profile_msg(endt).to_str().unwrap().trim();
            let (secs, _us) = s.split_once('.').unwrap();
            assert_eq!(secs, "0");
        }
    }
}
