//! Functions for dealing with the argument list (`src/nvim/arglist.c`).

use crate::buffer::{
    BLN_CURBUF, BLN_LISTED, buf_hide, buf_is_empty, buflist_add, buflist_findnr, bufref_T,
    bufref_valid, curbuf_reusable, first_win_in_tab, maketitle, otherfile, set_bufref,
};
use crate::charset::{NUL, ascii_isspace, rem_backslash, skipwhite};
use crate::context::is_ctx_win;
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, VarType, tv_get_number, tv_get_number_chk, tv_list_alloc_ret,
    tv_list_append_string, tv_set_string, typval_T, varnumber_T,
};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id};
use crate::ex_cmds::{ECMD_FORCEIT, ECMD_HIDE, ECMD_LAST, ECMD_OLDBUF, ECMD_ONE, do_ecmd};
use crate::ex_cmds2::{autowrite, check_changed};
use crate::ex_docmd::exarg_T;
use crate::ex_getln::gotocmdline;
use crate::fileio::file_pat_to_reg_pat;
use crate::globals::*;
use crate::mark::{setmark, setpcmark};

use crate::message::{emsg, semsg};
use crate::normal::reset_VIsual_and_resel;
use crate::option_vars::*;
use crate::os::input::os_breakcheck;
use crate::path::{
    EW_ADDSLASH, EW_DIR, EW_FILE, EW_NOTFOUND, EW_NOTWILD, FullName_save, expand_wildcards_vec,
    fix_fname, gen_expand_wildcards_vec, kPathCmpExpand, kPathCmpFull, kPathCmpLiteral, path_equal,
};
use crate::regexp::{RE_MAGIC, regmatch_T, vim_regcomp, vim_regexec, vim_regfree};
use crate::strings::gettext;
use crate::undo::bufIsChanged;
use crate::version::list_in_columns;
use crate::window::{
    WSP_BELOW, WSP_ROOM, check_can_set_curbuf_forceit, goto_tabpage_tp, lastwin_nofloating,
    tabpage_T, tabpage_index, valid_tabpage, win_T, win_close, win_enter, win_move_after,
    win_split, win_valid,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

use crate::math::{FAIL, OK};
use crate::option::OptInt;
use crate::os::env::expand_T;

const AL_SET: c_int = 1;
const AL_ADD: c_int = 2;
const AL_DEL: c_int = 3;

const CMD_args: c_int = 7;
pub(crate) const CMD_argdo: c_int = 10;
const CMD_argglobal: c_int = 13;
const CMD_arglocal: c_int = 14;
const CMD_drop: c_int = 132;
const CMD_snext: c_int = 420;

pub(crate) const CCGD_AW: c_int = 1;
pub(crate) const CCGD_MULTWIN: c_int = 2;
pub(crate) const CCGD_FORCEIT: c_int = 4;
pub(crate) const CCGD_EXCMD: c_int = 16;

pub(crate) const ML_EMPTY: c_int = 0x01;

const E_WINDOW_LAYOUT_CHANGED_UNEXPECTEDLY: &CStr = c"E249: Window layout changed unexpectedly";

#[repr(C)]
pub struct alist_T {
    pub al_ga: Vec<aentry_T>,
    pub id: c_int,
}

impl alist_T {
    pub const fn new() -> Self {
        Self {
            al_ga: Vec::new(),
            id: 0,
        }
    }
}

#[repr(C)]
pub struct aentry_T {
    pub ae_fname: Option<std::ffi::CString>,
    pub ae_fnum: c_int,
}

impl aentry_T {
    #[inline]
    pub fn fname_ptr(&self) -> *mut c_char {
        self.ae_fname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

struct arg_all_state_T {
    alist: *mut alist_T,
    local_alist: Option<std::rc::Rc<std::cell::RefCell<alist_T>>>,
    had_tab: c_int,
    keep_tabs: bool,
    forceit: bool,
    use_firstwin: bool,
    opened: Vec<u8>,
    new_curwin: *mut win_T,
    new_curtab: *mut tabpage_T,
}

struct ArglistModuleState {
    arglist_locked: bool,
}

impl ArglistModuleState {
    const fn new() -> Self {
        Self {
            arglist_locked: false,
        }
    }
}

static ARGLIST_STATE: crate::globals::GlobalState<ArglistModuleState> = crate::globals::GlobalState::new(ArglistModuleState::new());

#[inline]
fn arglist_st() -> &'static mut ArglistModuleState {
    unsafe { &mut *ARGLIST_STATE.as_ptr() }
}

#[inline]
pub fn win_alist(wp: &win_T) -> *mut alist_T {
    wp
            .w_alist
            .as_ref()
            .map_or(&raw mut g_st().global_alist, |rc| rc.as_ptr())
}

#[inline]
fn aarglist(al: &mut alist_T) -> *mut aentry_T {
    al.al_ga.as_mut_ptr()
}

#[inline]
fn warglist(wp: &win_T) -> *mut aentry_T {
    unsafe { aarglist(&mut *win_alist(wp)) }
}

#[inline]
fn wargcount(wp: &win_T) -> c_int {
    unsafe { (*win_alist(wp)).al_ga.len() as c_int }
}

#[inline]
pub fn arglist() -> *mut aentry_T {
     warglist(curwin_mut())
}

#[inline]
pub fn argcount() -> c_int {
     wargcount(curwin_mut())
}

#[inline]
pub fn garglist() -> *mut aentry_T {
    aarglist(&mut g_st().global_alist)
}

#[inline]
pub fn gargcount() -> c_int {
    unsafe {
        let ga = &raw const g_st().global_alist.al_ga;
        (*ga).len() as c_int
    }
}

fn check_arglist_locked() -> bool {
    if arglist_st().arglist_locked {
            emsg(gettext(
                e_cannot_change_arglist_recursively,
            ));
            return false;
        }
        true
}

/// Clear an argument list: free all file names and reset it to zero entries (`alist_clear`).
pub fn alist_clear(al: &mut alist_T) {
     {
        if !check_arglist_locked() {
            return;
        }
        al.al_ga = Vec::new();
    }
}

/// Create a new argument list and use it for the current window (`alist_new`).
pub fn alist_new() {
     {
        g_st().max_alist_id += 1;
        curwin_mut().w_alist = Some(std::rc::Rc::new(std::cell::RefCell::new(alist_T {
            al_ga: Vec::new(),
            id: g_st().max_alist_id,
        })));
    }
}

/// Set the argument list for the current window (`alist_set`).
fn alist_set(al: *mut alist_T, files: Vec<CString>, use_curbuf: c_int) {
    unsafe {
        if !check_arglist_locked() {
            return;
        }

        alist_clear( &mut *al);
        (*al).al_ga.reserve(files.len());
        for fname in files {
            if g_st().got_int {
                break;
            }

            if !fname.as_bytes().is_empty() {
                alist_add( &mut *al, fname, if use_curbuf != 0 { 2 } else { 1 });
            }
            os_breakcheck();
        }

        if ptr::eq(al, &raw const g_st().global_alist) {
            g_st().arg_had_last = false;
        }
    }
}

/// Add file `fname` to argument list `al` (`alist_add`).
pub fn alist_add(al: &mut alist_T, ae_fname: CString, set_fnum: c_int) {
    unsafe {
        let wp = curwin_ptr();
        if !check_arglist_locked() {
            return;
        }
        arglist_st().arglist_locked = true;
        (*wp).w_locked += 1;

        let ae_fnum = if set_fnum > 0 {
            buflist_add(
                ae_fname.as_ptr().cast_mut(),
                BLN_LISTED | if set_fnum == 2 { BLN_CURBUF } else { 0 },
            )
        } else {
            0
        };
        al.al_ga.push(aentry_T {
            ae_fname: Some(ae_fname),
            ae_fnum,
        });

        arglist_st().arglist_locked = false;
        (*wp).w_locked -= 1;
    }
}

/// Isolate one argument, taking backticks (`do_one_arg`).
fn do_one_arg(mut str_: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = str_;
        let mut inbacktick = false;
        while !crate::memory::cstr_is_empty(str_) {
            if rem_backslash(str_) {
                *p = *str_;
                p = p.add(1);
                str_ = str_.add(1);
                *p = *str_;
                p = p.add(1);
            } else {
                if !inbacktick && ascii_isspace(*str_) {
                    break;
                }
                if crate::memory::cstr_first_byte(str_) == b'`' {
                    inbacktick = !inbacktick;
                }
                *p = *str_;
                p = p.add(1);
            }
            str_ = str_.add(1);
        }
        str_ = skipwhite(str_);
        *p = NUL;
        str_
    }
}

/// Separate the arguments in `str` and return a list of pointers in `gap` (`get_arglist`).
fn get_arglist(gap: &mut Vec<*mut c_char>, mut str_: *mut c_char, escaped: bool) {
     {
        gap.clear();
        gap.reserve(20);
        while !crate::memory::cstr_is_empty(str_) {
            gap.push(str_);

            if !escaped {
                return;
            }
            str_ = do_one_arg(str_);
        }
    }
}

/// Parse a list of arguments (file names), expand them and return in `fnames` (`get_arglist_exp`).
pub fn get_arglist_exp(
    str_: *mut c_char,
    fnames: &mut Vec<CString>,
    wig: bool,
) -> bool {
    let mut ga: Vec<*mut c_char> = Vec::new();
        get_arglist(&mut ga, str_, true);

        if wig {
            expand_wildcards_vec(
                ga.len() as c_int,
                ga.as_mut_ptr(),
                fnames,
                EW_FILE | EW_NOTFOUND | EW_NOTWILD,
            )
        } else {
            gen_expand_wildcards_vec(
                ga.len() as c_int,
                ga.as_mut_ptr(),
                fnames,
                EW_FILE | EW_NOTFOUND | EW_NOTWILD,
            )
        }
}

/// Check the validity of the `arg_idx` for each other window (`alist_check_arg_idx`).
fn alist_check_arg_idx() {
     {
        for tp in for_all_tabpages_ptr() {
            for win in win_iter_mut(first_win_in_tab(unsafe { &mut *tp })) {
                if win_alist(win) == win_alist(curwin_mut()) {
                    check_arg_idx(win);
                }
            }
        }
    }
}

/// Add `files[count]` to the arglist of the current window after arg `after` (`alist_add_list`).
fn alist_add_list(files: Vec<CString>, mut after: c_int, will_edit: bool) {
    unsafe {
        let old_argcount = argcount();
        if check_arglist_locked() {
            let wp = curwin_ptr();
            let al = win_alist(&*wp);
            after = after.clamp(0, old_argcount);
            arglist_st().arglist_locked = true;
            (*wp).w_locked += 1;
            let count = files.len() as c_int;
            let mut new_entries = Vec::with_capacity(files.len());
            for f in files {
                let flags = BLN_LISTED | if will_edit { BLN_CURBUF } else { 0 };
                let ae_fname = (!f.as_bytes().is_empty()).then_some(f);
                let ae_fnum = buflist_add(
                    ae_fname
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                    flags,
                );
                new_entries.push(aentry_T { ae_fname, ae_fnum });
            }
            let pos = after as usize;
            (*al).al_ga.splice(pos..pos, new_entries);
            arglist_st().arglist_locked = false;
            (*wp).w_locked -= 1;
            if old_argcount > 0 && (*wp).w_arg_idx >= after {
                (*wp).w_arg_idx += count;
            }
        }
    }
}

/// Delete the file names in `alist_ga` from the argument list (`arglist_del_files`).
fn arglist_del_files(alist_ga: &[*mut c_char]) {
    unsafe {
        let mut regmatch: regmatch_T = regmatch_T::new();
        regmatch.rm_ic = opt_vars().p_fic != 0;

        for &orig_p in alist_ga {
            if g_st().got_int {
                break;
            }
            let Some(p) = file_pat_to_reg_pat(orig_p, ptr::null(), ptr::null_mut(), 0)
            else {
                break;
            };
            regmatch.regprog = vim_regcomp(p.as_ptr(), if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                break;
            }

            let mut didone = false;
            let mut match_idx: c_int = 0;
            while match_idx < argcount() {
                let entries = arglist();
                if vim_regexec(
                    &mut regmatch,
                    alist_name(&*entries.add(match_idx as usize)),
                    0,
                ) {
                    didone = true;
                    (*win_alist(curwin_mut())).al_ga.remove(match_idx as usize);
                    if curwin_mut().w_arg_idx > match_idx {
                        curwin_mut().w_arg_idx -= 1;
                    }
                    match_idx -= 1;
                }
                match_idx += 1;
            }

            vim_regfree(regmatch.regprog);
            if !didone {
                semsg!(gettext(e_nomatch2), orig_p);
            }
        }
    }
}

fn do_arglist(mut str_: *mut c_char, what: c_int, after: c_int, will_edit: bool) -> bool {
     {
        let mut new_ga: Vec<*mut c_char> = Vec::new();
        let mut exp_files: Vec<CString> = Vec::new();
        let mut arg_escaped = true;

        if !check_arglist_locked() {
            return false;
        }

        if what == AL_ADD && crate::memory::cstr_is_empty(str_) {
            if curbuf_mut().b_ffname.is_null() {
                return false;
            }
            str_ = curbuf_mut().b_fname;
            arg_escaped = false;
        }

        get_arglist(&mut new_ga, str_, arg_escaped);

        if what == AL_DEL {
            arglist_del_files(&new_ga);
        } else {
            let i = expand_wildcards_vec(
                new_ga.len() as c_int,
                new_ga.as_mut_ptr(),
                &mut exp_files,
                EW_DIR | EW_FILE | EW_ADDSLASH | EW_NOTFOUND,
            );
            drop(new_ga);
            if !i || exp_files.is_empty() {
                emsg(gettext(e_nomatch));
                return false;
            }

            if what == AL_ADD {
                alist_add_list(exp_files, after, will_edit);
            } else {
                debug_assert_eq!(what, AL_SET);
                alist_set(win_alist(curwin_mut()), exp_files, c_int::from(will_edit));
            }
        }

        alist_check_arg_idx();
        true
    }
}

/// Redefine the argument list (`set_arglist`).
pub fn set_arglist(str_: *mut c_char) {
    do_arglist(str_, AL_SET, 0, true);
}

/// Return true if window `win` is editing the file at the current argument index (`editing_arg_idx`).
pub fn editing_arg_idx(win: &mut win_T) -> bool {
    unsafe {
        let idx = win.w_arg_idx;
        if idx >= wargcount(&*win) {
            return false;
        }
        let entry = warglist(&*win).add(idx as usize);
        if win.buf_mut().b_fnum == (*entry).ae_fnum {
            return true;
        }
        let ffname = win.buf_mut().b_ffname;
        !ffname.is_null() && path_equal(alist_name(&*entry), ffname, kPathCmpExpand | kPathCmpFull)
    }
}

/// Check if window `win` is editing the `w_arg_idx` file in its argument list (`check_arg_idx`).
pub fn check_arg_idx(win: &mut win_T) {
    unsafe {
        let wcount = wargcount(&*win);
        if wcount > 1 && !editing_arg_idx(win) {
            win.w_arg_idx_invalid = true;
            let gcount = gargcount();
            if win.w_arg_idx != wcount - 1
                && !g_st().arg_had_last
                && ptr::eq(win_alist(&*win), &raw const g_st().global_alist)
                && gcount > 0
                && win.w_arg_idx < gcount
            {
                let last_entry = garglist().add((gcount - 1) as usize);
                let buf = win.w_buffer;
                if (*buf).b_fnum == (*last_entry).ae_fnum
                    || (!(*buf).b_ffname.is_null()
                        && path_equal(
                            alist_name(&*last_entry),
                            (*buf).b_ffname,
                            kPathCmpExpand | kPathCmpFull,
                        ))
                {
                    g_st().arg_had_last = true;
                }
            }
        } else {
            win.w_arg_idx_invalid = false;
            if win.w_arg_idx == wcount - 1 && ptr::eq(win_alist(&*win), &raw const g_st().global_alist) {
                g_st().arg_had_last = true;
            }
        }
    }
}

/// `:args`, `:arglocal` and `:argglobal` (`ex_args`).
pub fn ex_args(eap: &mut exarg_T) {
    unsafe {
        if eap.cmdidx != CMD_args {
            if !check_arglist_locked() {
                return;
            }
            if eap.cmdidx == CMD_argglobal {
                curwin_mut().w_alist = None;
            } else {
                alist_new();
            }
        }

        if !crate::memory::cstr_is_empty(eap.arg) {
            if !check_arglist_locked() {
                return;
            }
            ex_next(eap);
            return;
        }

        if eap.cmdidx == CMD_args {
            let count = argcount();
            if count <= 0 {
                return;
            }

            gotocmdline(true);

            let entries = arglist();
            let items: Vec<*mut c_char> = (0..(count as usize))
                .map(|i| alist_name(&*entries.add(i)))
                .collect();
            list_in_columns(&items, curwin_mut().w_arg_idx);
            return;
        }

        if eap.cmdidx == CMD_arglocal {
            let al = win_alist(curwin_mut());
            let gcount = gargcount() as usize;
            (*al).al_ga.reserve(gcount);

            for i in 0..gcount {
                let ge = &*garglist().add(i);
                if ge.ae_fname.is_some() {
                    (*al).al_ga.push(aentry_T {
                        ae_fname: ge.ae_fname.clone(),
                        ae_fnum: ge.ae_fnum,
                    });
                }
            }
        }
    }
}

/// `:previous`, `:sprevious`, `:Next` and `:sNext` (`ex_previous`).
pub fn ex_previous(eap: &mut exarg_T) {
     {
        let count = argcount();
        let target = if curwin_mut().w_arg_idx - eap.line2 >= count {
            count - 1
        } else {
            curwin_mut().w_arg_idx - eap.line2
        };
        do_argfile(eap, target);
    }
}

/// `:rewind`, `:first`, `:sfirst` and `:srewind` (`ex_rewind`).
pub fn ex_rewind(eap: &mut exarg_T) {
    do_argfile(eap, 0);
}

/// `:last` and `:slast` (`ex_last`).
pub fn ex_last(eap: &mut exarg_T) {
    do_argfile(eap, argcount() - 1);
}

/// `:argument` and `:sargument` (`ex_argument`).
pub fn ex_argument(eap: &mut exarg_T) {
     {
        let i = if eap.addr_count > 0 {
            eap.line2 - 1
        } else {
            curwin_mut().w_arg_idx
        };
        do_argfile(eap, i);
    }
}

/// Edit file `argn` of the argument lists (`do_argfile`).
pub fn do_argfile(eap: &mut exarg_T, argn: c_int) {
    unsafe {
        let is_split_cmd = crate::memory::cstr_first_byte(eap.cmd) == b's';
        let old_arg_idx = curwin_mut().w_arg_idx;
        let count = argcount();

        if argn < 0 || argn >= count {
            if count <= 1 {
                emsg(gettext(c"E163: There is only one file to edit"));
            } else if argn < 0 {
                emsg(gettext(c"E164: Cannot go before first file"));
            } else {
                emsg(gettext(c"E165: Cannot go beyond last file"));
            }
            return;
        }

        if !is_split_cmd
            && (*arglist().add(argn as usize)).ae_fnum != curbuf_mut().b_fnum
            && !check_can_set_curbuf_forceit(eap.forceit)
        {
            return;
        }

        setpcmark();

        if is_split_cmd || g_st().cmdmod.cmod_tab != 0 {
            if !win_split(0, 0) {
                return;
            }
            curwin_mut().w_onebuf_opt.wo_scb = 0;
            curwin_mut().w_onebuf_opt.wo_crb = 0;
        } else {
            let mut other = true;
            if buf_hide(curbuf_mut()) {
                let p = fix_fname(alist_name(&*arglist().add(argn as usize)));
                other = otherfile(
                    p.as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                );
            }
            if (!buf_hide(curbuf_mut()) || !other)
                && check_changed(
                    curbuf_mut(),
                    CCGD_AW
                        | if other { 0 } else { CCGD_MULTWIN }
                        | if eap.forceit != 0 { CCGD_FORCEIT } else { 0 }
                        | CCGD_EXCMD,
                )
            {
                return;
            }
        }

        curwin_mut().w_arg_idx = argn;
        if argn == argcount() - 1 && ptr::eq(win_alist(curwin_mut()), &raw const g_st().global_alist) {
            g_st().arg_had_last = true;
        }

        if !do_ecmd(
            0,
            alist_name(&*arglist().add(curwin_mut().w_arg_idx as usize)),
            ptr::null_mut(),
            Some(&*eap),
            ECMD_LAST,
            (if buf_hide(curwin_mut().buf_mut()) {
                ECMD_HIDE
            } else {
                0
            }) + (if eap.forceit != 0 { ECMD_FORCEIT } else { 0 }),
            curwin_ptr(),
        )
        {
            curwin_mut().w_arg_idx = old_arg_idx;
        } else if eap.cmdidx != CMD_argdo {
            setmark(b'\'' as c_int);
        }
    }
}

/// `:next`, and commands that behave like it (`ex_next`).
pub fn ex_next(eap: &mut exarg_T) {
     {
        if buf_hide(curbuf_mut())
            || eap.cmdidx == CMD_snext
            || !check_changed(
                curbuf_mut(),
                CCGD_AW | (if eap.forceit != 0 { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
            )
        {
            let i = if !crate::memory::cstr_is_empty(eap.arg) {
                if !do_arglist(eap.arg, AL_SET, 0, true) {
                    return;
                }
                0
            } else {
                curwin_mut().w_arg_idx + eap.line2
            };
            do_argfile(eap, i);
        }
    }
}

/// `:argdedupe` (`ex_argdedupe`).
pub fn ex_argdedupe(_eap: &mut exarg_T) {
    unsafe {
        let mut i: c_int = 0;
        while i < argcount() {
            let first_fullname = FullName_save((*arglist().add(i as usize)).fname_ptr(), false);
            let first_ptr = first_fullname
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

            let mut j = i + 1;
            while j < argcount() {
                let second_fullname =
                    FullName_save((*arglist().add(j as usize)).fname_ptr(), false);
                let second_ptr = second_fullname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                let dup = path_equal(first_ptr, second_ptr, kPathCmpLiteral);

                if dup {
                    (*win_alist(curwin_mut())).al_ga.remove(j as usize);

                    if curwin_mut().w_arg_idx == j {
                        curwin_mut().w_arg_idx = i;
                    } else if curwin_mut().w_arg_idx > j {
                        curwin_mut().w_arg_idx -= 1;
                    }

                    j -= 1;
                }
                j += 1;
            }

            i += 1;
        }
    }
}

/// `:argedit` (`ex_argedit`).
pub fn ex_argedit(eap: &mut exarg_T) {
     {
        let mut i = if eap.addr_count != 0 {
            eap.line2
        } else {
            curwin_mut().w_arg_idx + 1
        };
        let curbuf_is_reusable = curbuf_reusable();

        if !do_arglist(eap.arg, AL_ADD, i, true) {
            return;
        }
        maketitle();

        if curwin_mut().w_arg_idx == 0
            && (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0
            && (curbuf_mut().b_ffname.is_null() || curbuf_is_reusable)
        {
            i = 0;
        }
        if i < argcount() {
            do_argfile(eap, i);
        }
    }
}

/// `:argadd` (`ex_argadd`).
pub fn ex_argadd(eap: &mut exarg_T) {
     {
        do_arglist(
            eap.arg,
            AL_ADD,
            if eap.addr_count > 0 {
                eap.line2
            } else {
                curwin_mut().w_arg_idx + 1
            },
            false,
        );
        maketitle();
    }
}

/// `:argdelete` (`ex_argdelete`).
pub fn ex_argdelete(eap: &mut exarg_T) {
    unsafe {
        if !check_arglist_locked() {
            return;
        }

        if eap.addr_count > 0 || crate::memory::cstr_is_empty(eap.arg) {
            if eap.addr_count == 0 {
                if curwin_mut().w_arg_idx >= argcount() {
                    emsg(gettext(c"E610: No argument to delete"));
                    return;
                }
                eap.line1 = curwin_mut().w_arg_idx + 1;
                eap.line2 = eap.line1;
            } else if eap.line2 > argcount() {
                eap.line2 = argcount();
            }
            let n = eap.line2 - eap.line1 + 1;
            if !crate::memory::cstr_is_empty(eap.arg) {
                emsg(gettext(e_invarg));
                return;
            }
            if n <= 0 {
                if eap.line1 != 1 || eap.line2 != 0 {
                    emsg(gettext(e_invrange));
                }
                return;
            }

            (*win_alist(curwin_mut()))
                .al_ga
                .drain((eap.line1 as usize - 1)..(eap.line2 as usize));
            if curwin_mut().w_arg_idx >= eap.line2 {
                curwin_mut().w_arg_idx -= n;
            } else if curwin_mut().w_arg_idx > eap.line1 {
                curwin_mut().w_arg_idx = eap.line1;
            }
        } else {
            do_arglist(eap.arg, AL_DEL, 0, false);
        }

        let new_len = argcount();
        if new_len == 0 {
            curwin_mut().w_arg_idx = 0;
        } else if curwin_mut().w_arg_idx >= new_len {
            curwin_mut().w_arg_idx = new_len - 1;
        }

        maketitle();
    }
}

/// Obtain the possible arguments of the `argedit` and `argdelete` commands (`get_arglist_name`).
pub fn get_arglist_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx >= argcount() {
            return ptr::null_mut();
        }
        alist_name(&*arglist().add(idx as usize))
    }
}

/// Get the file name for an argument list entry (`alist_name`).
pub fn alist_name(aep: &aentry_T) -> *mut c_char {
    unsafe {
        let bp = buflist_findnr(aep.ae_fnum);
        if bp.is_null() || (*bp).b_fname.is_null() {
            aep.fname_ptr()
        } else {
            (*bp).b_fname
        }
    }
}

/// Close all the windows containing files which are not in the argument list (`arg_all_close_unused_windows`).
fn arg_all_close_unused_windows(aall: &mut arg_all_state_T) {
    unsafe {
        let old_curwin = curwin_ptr();
        let old_curtab = g_st().curtab;

        if aall.had_tab > 0 {
            goto_tabpage_tp(g_st().first_tabpage, true, true);
        }

        g_st().tabpage_move_disallowed += 1;
        loop {
            let mut tpnext = curtab_mut().tp_next;
            let mut wp = if lastwin_mut().w_floating {
                g_st().lastwin
            } else {
                g_st().firstwin
            };
            while !wp.is_null() {
                let mut wpnext = if (*wp).w_floating {
                    if !(*wp).w_prev.is_null() && (*(*wp).w_prev).w_floating {
                        (*wp).w_prev
                    } else {
                        g_st().firstwin
                    }
                } else if (*wp).w_next.is_null() || (*(*wp).w_next).w_floating {
                    ptr::null_mut()
                } else {
                    (*wp).w_next
                };
                let buf = (*wp).w_buffer;
                let aall_ref = &mut *aall;
                let opened_len = aall_ref.opened.len() as c_int;
                let mut i: c_int;
                if (*buf).b_ffname.is_null()
                    || (!aall_ref.keep_tabs
                        && ((*buf).b_nwindows > 1
                            || (*wp).w_width != g_st().Columns
                            || ((*wp).w_floating && !is_ctx_win(wp))))
                {
                    i = opened_len;
                } else {
                    i = 0;
                    while i < opened_len {
                        let al = aall_ref.alist;
                        if (i as usize) < (*al).al_ga.len() {
                            let entry = aarglist(&mut *al).add(i as usize);
                            if (*entry).ae_fnum == (*buf).b_fnum
                                || path_equal(
                                    alist_name(&*entry),
                                    (*buf).b_ffname,
                                    kPathCmpExpand | kPathCmpFull,
                                )
                            {
                                let mut weight: c_int = 1;
                                if old_curtab == g_st().curtab {
                                    weight += 1;
                                    if old_curwin == wp {
                                        weight += 1;
                                    }
                                }

                                if weight > c_int::from(aall_ref.opened[i as usize]) {
                                    aall_ref.opened[i as usize] = weight as u8;
                                    if i == 0 {
                                        if !aall_ref.new_curwin.is_null() {
                                            (*aall_ref.new_curwin).w_arg_idx = opened_len;
                                        }
                                        aall_ref.new_curwin = wp;
                                        aall_ref.new_curtab = g_st().curtab;
                                    }
                                } else if aall_ref.keep_tabs {
                                    i = opened_len;
                                }

                                if win_alist(&*wp) != al {
                                    (*wp).w_alist = aall_ref.local_alist.clone();
                                }
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                (*wp).w_arg_idx = i;

                if i == opened_len
                    && !aall_ref.keep_tabs
                    && (buf_hide(&*buf)
                        || aall_ref.forceit
                        || (*buf).b_nwindows > 1
                        || !bufIsChanged(&mut *buf))
                {
                    if !buf_hide(&*buf) && (*buf).b_nwindows <= 1 && bufIsChanged(&mut *buf) {
                        let mut bufref: bufref_T = bufref_T::new();
                        set_bufref(&mut bufref, buf);
                        autowrite(&mut *buf, false);
                        if !win_valid(wp) || !bufref_valid(Some(&bufref)) {
                            wp = if lastwin_mut().w_floating {
                                g_st().lastwin
                            } else {
                                g_st().firstwin
                            };
                            continue;
                        }
                    }
                    if g_st().firstwin == g_st().lastwin
                        && ((*g_st().first_tabpage).tp_next.is_null() || aall.had_tab == 0)
                    {
                        aall.use_firstwin = true;
                    } else {
                        win_close(wp, !buf_hide(&*buf) && !bufIsChanged(&mut *buf), false);
                        if !win_valid(wpnext) {
                            wpnext = if lastwin_mut().w_floating {
                                g_st().lastwin
                            } else {
                                g_st().firstwin
                            };
                        }
                    }
                }
                wp = wpnext;
            }

            if aall.had_tab == 0 || tpnext.is_null() {
                break;
            }

            if !valid_tabpage(tpnext) {
                tpnext = g_st().first_tabpage;
            }
            goto_tabpage_tp(tpnext, true, true);
        }
        g_st().tabpage_move_disallowed -= 1;
    }
}

/// Open up to `count` windows for the files in the argument list `aall->alist` (`arg_all_open_windows`).
fn arg_all_open_windows(aall: &mut arg_all_state_T, count: c_int) {
    unsafe {
        let mut tab_drop_empty_window = false;

        if aall.keep_tabs
            && buf_is_empty(curbuf_mut())
            && curbuf_mut().b_nwindows == 1
            && curbuf_mut().b_ffname.is_null()
            && curbuf_mut().b_changed == 0
        {
            aall.use_firstwin = true;
            tab_drop_empty_window = true;
        }

        let mut split_ret = OK;
        let mut i: c_int = 0;
        while i < count && !g_st().got_int {
            if ptr::eq(aall.alist, &raw const g_st().global_alist) && i == gargcount() - 1 {
                g_st().arg_had_last = true;
            }
            if aall.opened[i as usize] > 0 {
                if curwin_mut().w_arg_idx != i {
                    for wp in for_all_windows_ptr() {
                        if (*wp).w_arg_idx == i {
                            if aall.keep_tabs {
                                aall.new_curwin = wp;
                                aall.new_curtab = g_st().curtab;
                            } else if (*wp).w_floating {
                                break;
                            } else if (*(*wp).w_frame).fr_parent != (*curwin_mut().w_frame).fr_parent {
                                emsg(gettext(E_WINDOW_LAYOUT_CHANGED_UNEXPECTEDLY));
                                i = count;
                                break;
                            } else {
                                win_move_after(&mut *wp, curwin_mut());
                            }
                            break;
                        }
                    }
                }
            } else if split_ret == OK {
                if tab_drop_empty_window && i == count - 1 {
                    g_st().autocmd_no_enter -= 1;
                }
                if !aall.use_firstwin {
                    let p_ea_save = opt_vars().p_ea;
                    opt_vars().p_ea = 1;
                    let ok = win_split(0, WSP_ROOM | WSP_BELOW);
                    split_ret = if ok { OK } else { FAIL };
                    opt_vars().p_ea = p_ea_save;
                    if !ok {
                        i += 1;
                        continue;
                    }
                } else {
                    g_st().autocmd_no_leave -= 1;
                }

                curwin_mut().w_arg_idx = i;
                if i == 0 {
                    aall.new_curwin = curwin_ptr();
                    aall.new_curtab = g_st().curtab;
                }
                do_ecmd(
                    0,
                    alist_name(&*aarglist(&mut *aall.alist).add(i as usize)),
                    ptr::null_mut(),
                    None,
                    ECMD_ONE,
                    (if buf_hide(curwin_mut().buf_mut()) || bufIsChanged(curwin_mut().buf_mut()) {
                        ECMD_HIDE
                    } else {
                        0
                    }) + ECMD_OLDBUF,
                    curwin_ptr(),
                );
                if tab_drop_empty_window && i == count - 1 {
                    g_st().autocmd_no_enter += 1;
                }
                if aall.use_firstwin {
                    g_st().autocmd_no_leave += 1;
                }
                aall.use_firstwin = false;
            }
            os_breakcheck();

            if aall.had_tab > 0 && (tabpage_index(ptr::null_mut()) as OptInt) <= opt_vars().p_tpm {
                g_st().cmdmod.cmod_tab = 9999;
            }
            i += 1;
        }
    }
}

/// Open up to `count` windows, one for each argument (`do_arg_all`).
fn do_arg_all(mut count: c_int, forceit: c_int, keep_tabs: bool) {
     {
        let prev_arglist_locked = arglist_st().arglist_locked;
        debug_assert!(!g_st().firstwin.is_null());

        let acount = argcount();
        if acount <= 0 {
            return;
        }
        setpcmark();

        let mut aall = arg_all_state_T {
            alist: win_alist(curwin_mut()),
            local_alist: curwin_mut().w_alist.clone(),
            had_tab: g_st().cmdmod.cmod_tab,
            keep_tabs,
            forceit: forceit != 0,
            use_firstwin: false,
            opened: vec![0u8; acount as usize],
            new_curwin: ptr::null_mut(),
            new_curtab: ptr::null_mut(),
        };

        arglist_st().arglist_locked = true;

        let new_lu_tp = g_st().curtab;

        reset_VIsual_and_resel();

        arg_all_close_unused_windows(&mut aall);

        let opened_len = aall.opened.len() as c_int;
        if count > opened_len || count <= 0 {
            count = opened_len;
        }

        g_st().autocmd_no_enter += 1;
        g_st().autocmd_no_leave += 1;
        let last_curwin = curwin_ptr();
        let last_curtab = g_st().curtab;
        win_enter(lastwin_nofloating(ptr::null_mut()), false);

        arg_all_open_windows(&mut aall, count);

        drop(aall.local_alist.take());
        arglist_st().arglist_locked = prev_arglist_locked;

        g_st().autocmd_no_enter -= 1;

        if last_curtab != aall.new_curtab {
            if valid_tabpage(last_curtab) {
                goto_tabpage_tp(last_curtab, true, true);
            }
            if win_valid(last_curwin) {
                win_enter(last_curwin, false);
            }
        }
        if valid_tabpage(aall.new_curtab) {
            goto_tabpage_tp(aall.new_curtab, true, true);
        }

        if valid_tabpage(new_lu_tp) {
            g_st().lastused_tabpage = new_lu_tp;
        }

        if win_valid(aall.new_curwin) {
            win_enter(aall.new_curwin, false);
        }

        g_st().autocmd_no_leave -= 1;
    }
}

/// `:all` and `:sall` (`ex_all`).
pub fn ex_all(eap: &mut exarg_T) {
    if eap.addr_count == 0 {
            eap.line2 = 9999;
        }
        do_arg_all(eap.line2, eap.forceit, eap.cmdidx == CMD_drop);
}

/// Concatenate all files in the argument list, separated by spaces (`arg_all`).
pub fn arg_all() -> CString {
    unsafe {
        let mut bytes: Vec<u8> = Vec::new();
        let count = argcount();
        for idx in 0..(count as usize) {
            let mut p = alist_name(&*arglist().add(idx));
            if p.is_null() {
                continue;
            }
            if !bytes.is_empty() {
                bytes.push(b' ');
            }
            while !crate::memory::cstr_is_empty(p) {
                let ch = crate::memory::cstr_first_byte(p);
                let escape = ch == b' ' || ch == b'`' || (cfg!(not(windows)) && ch == b'\\');
                if escape {
                    bytes.push(b'\\');
                }
                bytes.push(ch);
                p = p.add(1);
            }
        }
        CString::new(bytes).unwrap_or_default()
    }
}

/// `argc([window id])` function (`f_argc`).
pub fn f_argc(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        if argvars[0].v_type == VarType::VAR_UNKNOWN {
            rettv.vval = crate::eval::typval::TypvalVval::Number(argcount() as varnumber_T);
        } else if argvars[0].v_type == VarType::VAR_NUMBER && tv_get_number(&argvars[0]) == -1 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(gargcount() as varnumber_T);
        } else {
            let wp = find_win_by_nr_or_id(&argvars[0]);
            rettv.vval = crate::eval::typval::TypvalVval::Number(if !wp.is_null() {
                wargcount(unsafe { &*wp }) as varnumber_T
            } else {
                -1
            });
        }
    }
}

/// `argidx()` function (`f_argidx`).
pub fn f_argidx(_argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
     {
        rettv.vval = crate::eval::typval::TypvalVval::Number(curwin_mut().w_arg_idx as varnumber_T);
    }
}

/// `arglistid()` function (`f_arglistid`).
pub fn f_arglistid(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        let wp = find_tabwin(&argvars[0], &argvars[1]);
        if !wp.is_null() {
            rettv.vval = crate::eval::typval::TypvalVval::Number((*win_alist(&*wp)).id as varnumber_T);
        }
    }
}

fn get_arglist_as_rettv(arglist_ptr: *mut aentry_T, count: c_int, rettv: &mut typval_T) {
    unsafe {
        tv_list_alloc_ret(&mut *rettv, count as isize);
        if !arglist_ptr.is_null() {
            for idx in 0..(count as usize) {
                tv_list_append_string( &mut *rettv.vval.v_list(), alist_name(&*arglist_ptr.add(idx)), -1);
            }
        }
    }
}

/// `argv(nr)` function (`f_argv`).
pub fn f_argv(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let mut al_ptr: *mut aentry_T = ptr::null_mut();
        let mut count: c_int = -1;

        if argvars[0].v_type == VarType::VAR_UNKNOWN {
            get_arglist_as_rettv(arglist(), argcount(), rettv);
            return;
        }

        if argvars[1].v_type == VarType::VAR_UNKNOWN {
            al_ptr = arglist();
            count = argcount();
        } else if argvars[1].v_type == VarType::VAR_NUMBER
            && tv_get_number(&argvars[1]) == -1
        {
            al_ptr = garglist();
            count = gargcount();
        } else {
            let wp = find_win_by_nr_or_id(&argvars[1]);
            if !wp.is_null() {
                al_ptr = warglist( &*wp);
                count = wargcount( &*wp);
            }
        }

        tv_set_string(rettv, ptr::null());
        let idx = tv_get_number_chk(&argvars[0], None) as c_int;
        if !al_ptr.is_null() && idx >= 0 && idx < count {
            tv_set_string(rettv, alist_name(&*al_ptr.add(idx as usize)));
        } else if idx == -1 {
            get_arglist_as_rettv(al_ptr, count, rettv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alist_init_and_clear_work() {
        let mut al = alist_T::new();
            assert!(al.al_ga.is_empty());
            alist_clear(&mut al);
            assert!(al.al_ga.is_empty());
    }
}
