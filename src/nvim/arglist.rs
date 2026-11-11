//! Functions for dealing with the argument list (`src/nvim/arglist.c`).
#![forbid(unsafe_code)]

use crate::buffer::{
    BLN_CURBUF, BLN_LISTED, Bufref, buf_hide, buf_is_empty, buflist_add, buflist_findnr_opt,
    bufref_valid, curbuf_reusable, first_win_in_tab, maketitle, otherfile, set_bufref,
};
use crate::charset::{ascii_isspace, rem_backslash};
use crate::cmdexpand::Cmdidx;
use crate::context::is_ctx_win;
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, TypVal, VarType, Varnumber, tv_get_number, tv_get_number_chk, tv_list_alloc_ret,
    tv_list_append_string, tv_set_string,
};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id};
use crate::ex_cmds::{ECMD_FORCEIT, ECMD_HIDE, ECMD_LAST, ECMD_OLDBUF, ECMD_ONE, do_ecmd};
use crate::ex_cmds2::{autowrite, check_changed};
use crate::ex_docmd::Exarg;
use crate::ex_getln::gotocmdline;
use crate::fileio::file_pat_to_reg_pat;
use crate::globals::*;
use crate::mark::{setmark, setpcmark};

use crate::message::{emsg, semsg};
use crate::normal::reset_visual_and_resel;
use crate::option_vars::*;
use crate::os::input::os_breakcheck;
use crate::path::{
    EW_ADDSLASH, EW_DIR, EW_FILE, EW_NOTFOUND, EW_NOTWILD, FullName_save, K_PATH_CMP_EXPAND,
    K_PATH_CMP_FULL, K_PATH_CMP_LITERAL, expand_wildcards_vec, fix_fname, gen_expand_wildcards_vec,
    path_equal,
};
use crate::regexp::{RE_MAGIC, Regmatch, vim_regcomp, vim_regexec};
use crate::strings::gettext;
use crate::undo::buf_is_changed_fn;
use crate::version::list_in_columns;
use crate::window::{
    WSP_BELOW, WSP_ROOM, WinInfoInfo, check_can_set_curbuf_forceit, goto_tabpage_tp,
    lastwin_nofloating, tabpage_index, valid_tabpage, win_close, win_enter, win_move_after,
    win_split, win_valid,
};
use std::ffi::{CStr, CString};

use crate::option::OptInt;
use crate::os::env::ExpandCtx;

const AL_SET: i32 = 1;
const AL_ADD: i32 = 2;
const AL_DEL: i32 = 3;

pub(crate) const CCGD_AW: i32 = 1;
pub(crate) const CCGD_MULTWIN: i32 = 2;
pub(crate) const CCGD_FORCEIT: i32 = 4;
pub(crate) const CCGD_EXCMD: i32 = 16;

pub(crate) const ML_EMPTY: i32 = 0x01;

const E_WINDOW_LAYOUT_CHANGED_UNEXPECTEDLY: &CStr = c"E249: Window layout changed unexpectedly";

#[repr(C)]
pub struct Alist {
    pub al_ga: std::cell::RefCell<Vec<Aentry>>,
    pub id: std::cell::Cell<i32>,
}

impl Alist {
    pub const fn new() -> Self {
        Self {
            al_ga: std::cell::RefCell::new(Vec::new()),
            id: std::cell::Cell::new(0),
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Aentry {
    pub ae_fname: Option<std::ffi::CString>,
    pub ae_fnum: i32,
}

impl Aentry {
    #[inline]
    pub fn fname_cstr(&self) -> Option<&CStr> {
        self.ae_fname.as_deref()
    }
}

struct ArgAllState {
    alist: std::rc::Rc<Alist>,
    local_alist: Option<std::rc::Rc<Alist>>,
    had_tab: i32,
    keep_tabs: bool,
    forceit: bool,
    use_firstwin: bool,
    opened: Vec<u8>,
    new_curwin: crate::globals::WinPtr,
    new_curtab: crate::globals::TabpagePtr,
}

std::thread_local! {
    static ARGLIST_LOCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static MAX_ALIST_ID: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static GLOBAL_ALIST: std::rc::Rc<Alist> = std::rc::Rc::new(Alist::new());
}

#[inline]
pub fn global_alist_rc() -> std::rc::Rc<Alist> {
    GLOBAL_ALIST.with(std::rc::Rc::clone)
}

#[inline]
pub fn win_alist_rc(wp: &WinInfoInfo) -> std::rc::Rc<Alist> {
    wp.w_alist.clone().unwrap_or_else(global_alist_rc)
}

#[inline]
pub fn is_global_alist(al: &std::rc::Rc<Alist>) -> bool {
    GLOBAL_ALIST.with(|g| std::rc::Rc::ptr_eq(al, g))
}

#[inline]
pub fn wargcount(wp: &WinInfoInfo) -> i32 {
    win_alist_rc(wp).al_ga.borrow().len() as i32
}

#[inline]
pub fn argcount() -> i32 {
    wargcount(curwin_mut())
}

#[inline]
pub fn gargcount() -> i32 {
    GLOBAL_ALIST.with(|g| g.al_ga.borrow().len() as i32)
}

#[inline]
pub fn garg_entry(idx: usize) -> Aentry {
    GLOBAL_ALIST.with(|g| g.al_ga.borrow()[idx].clone())
}

#[inline]
pub fn arg_entry(idx: usize) -> Aentry {
    win_alist_rc(curwin_mut()).al_ga.borrow()[idx].clone()
}

#[inline]
pub fn warg_entry(wp: &WinInfoInfo, idx: usize) -> Aentry {
    win_alist_rc(wp).al_ga.borrow()[idx].clone()
}

fn check_arglist_locked() -> bool {
    if ARGLIST_LOCKED.get() {
        emsg(gettext(E_CANNOT_CHANGE_ARGLIST_RECURSIVELY));
        return false;
    }
    true
}

/// Clear an argument list: free all file names and reset it to zero entries (`alist_clear`).
pub fn alist_clear(al: &Alist) {
    if !check_arglist_locked() {
        return;
    }
    *al.al_ga.borrow_mut() = Vec::new();
}

/// Create a new argument list and use it for the current window (`alist_new`).
pub fn alist_new() {
    let next_id = MAX_ALIST_ID.with(|id| {
        let next = id.get() + 1;
        id.set(next);
        next
    });
    curwin_mut().w_alist = Some(std::rc::Rc::new(Alist {
        al_ga: std::cell::RefCell::new(Vec::new()),
        id: std::cell::Cell::new(next_id),
    }));
}

/// Set the argument list for the current window (`alist_set`).
fn alist_set(al: &std::rc::Rc<Alist>, files: Vec<CString>, use_curbuf: i32) {
    if !check_arglist_locked() {
        return;
    }

    alist_clear(al);

    al.al_ga.borrow_mut().reserve(files.len());

    for fname in files {
        if g_st().got_int {
            break;
        }

        if !fname.as_bytes().is_empty() {
            alist_add(al, fname, if use_curbuf != 0 { 2 } else { 1 });
        }
        os_breakcheck();
    }

    if is_global_alist(al) {
        g_st().arg_had_last = false;
    }
}

/// Add file `fname` to argument list `al` (`alist_add`).
pub fn alist_add(al: &Alist, ae_fname: CString, set_fnum: i32) {
    let mut wp = curwin_ptr();
    if !check_arglist_locked() {
        return;
    }
    ARGLIST_LOCKED.set(true);

    wp.w_locked += 1;

    let ae_fnum = if set_fnum > 0 {
        buflist_add(
            Some(ae_fname.as_c_str()),
            BLN_LISTED | if set_fnum == 2 { BLN_CURBUF } else { 0 },
        )
    } else {
        0
    };
    al.al_ga.borrow_mut().push(Aentry {
        ae_fname: Some(ae_fname),
        ae_fnum,
    });

    ARGLIST_LOCKED.set(false);

    wp.w_locked -= 1;
}

/// Isolate one argument, taking backticks (`do_one_arg`).
fn do_one_arg(str_: &[u8]) -> (CString, &[u8]) {
    let bytes = crate::memory::bytes_until_nul(str_);
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    let mut inbacktick = false;
    while idx < bytes.len() {
        if rem_backslash(&bytes[idx..]) && idx + 1 < bytes.len() {
            out.push(bytes[idx]);
            idx += 1;
            out.push(bytes[idx]);
            idx += 1;
        } else {
            if !inbacktick && ascii_isspace(i32::from(bytes[idx])) {
                break;
            }
            if bytes[idx] == b'`' {
                inbacktick = !inbacktick;
            }
            out.push(bytes[idx]);
            idx += 1;
        }
    }
    let rest = crate::charset::skipwhite_slice(&bytes[idx..]);
    (CString::new(out).unwrap_or_default(), rest)
}

/// Separate the arguments in `str` and return a list of strings in `gap` (`get_arglist`).
fn get_arglist(gap: &mut Vec<CString>, str_: &[u8], escaped: bool) {
    let mut rem = crate::memory::bytes_until_nul(str_);
    gap.clear();
    gap.reserve(20);
    while !rem.is_empty() {
        if !escaped {
            gap.push(CString::new(rem).unwrap_or_default());
            return;
        }
        let (arg, next_rem) = do_one_arg(rem);
        gap.push(arg);
        rem = next_rem;
    }
}

/// Parse a list of arguments (file names), expand them and return in `fnames` (`get_arglist_exp`).
pub fn get_arglist_exp(str_: &[u8], fnames: &mut Vec<CString>, wig: bool) -> bool {
    let mut ga: Vec<CString> = Vec::new();
    get_arglist(&mut ga, str_, true);
    let ptrs: Vec<&CStr> = ga.iter().map(CString::as_c_str).collect();

    if wig {
        expand_wildcards_vec(&ptrs, fnames, EW_FILE | EW_NOTFOUND | EW_NOTWILD)
    } else {
        gen_expand_wildcards_vec(&ptrs, fnames, EW_FILE | EW_NOTFOUND | EW_NOTWILD)
    }
}

/// Check the validity of the `arg_idx` for each other window (`alist_check_arg_idx`).
fn alist_check_arg_idx() {
    let cur_al = win_alist_rc(curwin_mut());
    for tp in for_all_tabpages_ptr() {
        for win in win_iter_mut(first_win_in_tab(&tp)) {
            if std::rc::Rc::ptr_eq(&win_alist_rc(win), &cur_al) {
                check_arg_idx(win);
            }
        }
    }
}

/// Add `files[count]` to the arglist of the current window after arg `after` (`alist_add_list`).
fn alist_add_list(files: Vec<CString>, mut after: i32, will_edit: bool) {
    let old_argcount = argcount();
    if check_arglist_locked() {
        let mut wp = curwin_ptr();
        let al = win_alist_rc(wp.borrow());
        after = after.clamp(0, old_argcount);
        ARGLIST_LOCKED.set(true);

        wp.w_locked += 1;

        let count = files.len() as i32;
        let mut new_entries = Vec::with_capacity(files.len());
        for f in files {
            let flags = BLN_LISTED | if will_edit { BLN_CURBUF } else { 0 };
            let ae_fname = (!f.as_bytes().is_empty()).then_some(f);
            let ae_fnum = buflist_add(ae_fname.as_deref(), flags);
            new_entries.push(Aentry { ae_fname, ae_fnum });
        }
        let pos = after as usize;

        al.al_ga.borrow_mut().splice(pos..pos, new_entries);

        ARGLIST_LOCKED.set(false);
        {
            wp.w_locked -= 1;
            if old_argcount > 0 && wp.w_arg_idx >= after {
                wp.w_arg_idx += count;
            }
        }
    }
}

/// Delete the file names in `alist_ga` from the argument list (`arglist_del_files`).
fn arglist_del_files(alist_ga: &[CString]) {
    let mut regmatch: Regmatch = Regmatch::new();
    regmatch.rm_ic = opt_vars().p_fic.get();

    for orig_p in alist_ga {
        if g_st().got_int {
            break;
        }
        let Some(p) = file_pat_to_reg_pat(orig_p.to_bytes(), None, 0) else {
            break;
        };
        regmatch.set_prog(vim_regcomp(
            &p,
            if opt_vars().p_magic.get() {
                RE_MAGIC
            } else {
                0
            },
        ));
        if regmatch.regprog.is_null() {
            break;
        }

        let mut didone = false;
        let mut match_idx: i32 = 0;
        while match_idx < argcount() {
            let entry = arg_entry(match_idx as usize);
            if vim_regexec(&mut regmatch, alist_name(&entry).unwrap_or(c""), 0) {
                didone = true;
                win_alist_rc(curwin_mut())
                    .al_ga
                    .borrow_mut()
                    .remove(match_idx as usize);
                if curwin_mut().w_arg_idx > match_idx {
                    curwin_mut().w_arg_idx -= 1;
                }
                match_idx -= 1;
            }
            match_idx += 1;
        }

        regmatch.free_prog();
        if !didone {
            semsg!(gettext(E_NOMATCH2), orig_p);
        }
    }
}

fn do_arglist(str_cstr: &CStr, what: i32, after: i32, will_edit: bool) -> bool {
    let mut str_bytes = str_cstr.to_bytes();
    let mut new_ga: Vec<CString> = Vec::new();
    let mut exp_files: Vec<CString> = Vec::new();
    let mut arg_escaped = true;

    if !check_arglist_locked() {
        return false;
    }

    if what == AL_ADD && str_bytes.is_empty() {
        if curbuf_mut().b_ffname.is_none() {
            return false;
        }
        str_bytes = curbuf_mut().fname_bytes();
        arg_escaped = false;
    }

    get_arglist(&mut new_ga, str_bytes, arg_escaped);

    if what == AL_DEL {
        arglist_del_files(&new_ga);
    } else {
        let ptrs: Vec<&CStr> = new_ga.iter().map(CString::as_c_str).collect();
        let i = expand_wildcards_vec(
            &ptrs,
            &mut exp_files,
            EW_DIR | EW_FILE | EW_ADDSLASH | EW_NOTFOUND,
        );
        drop(ptrs);
        drop(new_ga);
        if !i || exp_files.is_empty() {
            emsg(gettext(E_NOMATCH));
            return false;
        }

        if what == AL_ADD {
            alist_add_list(exp_files, after, will_edit);
        } else {
            debug_assert_eq!(what, AL_SET);
            alist_set(&win_alist_rc(curwin_mut()), exp_files, i32::from(will_edit));
        }
    }

    alist_check_arg_idx();
    true
}

/// Redefine the argument list (`set_arglist`).
pub fn set_arglist(str_: &CStr) {
    do_arglist(str_, AL_SET, 0, true);
}

/// Return true if window `win` is editing the file at the current argument index (`editing_arg_idx`).
pub fn editing_arg_idx(win: &mut WinInfoInfo) -> bool {
    let idx = win.w_arg_idx;
    if idx >= wargcount(&*win) {
        return false;
    }
    let entry = warg_entry(&*win, idx as usize);
    if win.buf_mut().b_fnum == entry.ae_fnum {
        return true;
    }
    win.buf_mut().ffname_opt().is_some_and(|ffname| {
        path_equal(
            alist_name(&entry).unwrap_or(c""),
            ffname,
            K_PATH_CMP_EXPAND | K_PATH_CMP_FULL,
        )
    })
}

/// Check if window `win` is editing the `w_arg_idx` file in its argument list (`check_arg_idx`).
pub fn check_arg_idx(win: &mut WinInfoInfo) {
    let wcount = wargcount(&*win);
    if wcount > 1 && !editing_arg_idx(win) {
        win.w_arg_idx_invalid = true;
        let gcount = gargcount();
        if win.w_arg_idx != wcount - 1
            && !g_st().arg_had_last
            && is_global_alist(&win_alist_rc(&*win))
            && gcount > 0
            && win.w_arg_idx < gcount
        {
            let last_entry = garg_entry((gcount - 1) as usize);
            let buf = win.buf_mut();
            if buf.b_fnum == last_entry.ae_fnum
                || (buf.b_ffname.is_some()
                    && path_equal(
                        alist_name(&last_entry).unwrap_or(c""),
                        buf.ffname_cstr(),
                        K_PATH_CMP_EXPAND | K_PATH_CMP_FULL,
                    ))
            {
                g_st().arg_had_last = true;
            }
        }
    } else {
        win.w_arg_idx_invalid = false;
        if win.w_arg_idx == wcount - 1 && is_global_alist(&win_alist_rc(&*win)) {
            g_st().arg_had_last = true;
        }
    }
}

/// `:args`, `:arglocal` and `:argglobal` (`ex_args`).
pub fn ex_args(eap: &mut Exarg) {
    if eap.cmdidx != Cmdidx::CmdArgs {
        if !check_arglist_locked() {
            return;
        }
        if eap.cmdidx == Cmdidx::CmdArgglobal {
            curwin_mut().w_alist = None;
        } else {
            alist_new();
        }
    }

    if !eap.arg.is_empty() {
        if !check_arglist_locked() {
            return;
        }
        ex_next(eap);
        return;
    }

    if eap.cmdidx == Cmdidx::CmdArgs {
        let count = argcount();
        if count <= 0 {
            return;
        }

        gotocmdline(true);

        let entries = win_alist_rc(curwin_mut()).al_ga.borrow().clone();
        let items: Vec<&CStr> = entries[..(count as usize)]
            .iter()
            .map(|e| alist_name(e).unwrap_or(c""))
            .collect();
        list_in_columns(&items, curwin_mut().w_arg_idx);
        return;
    }

    if eap.cmdidx == Cmdidx::CmdArglocal {
        let al = win_alist_rc(curwin_mut());
        let g_entries = global_alist_rc().al_ga.borrow().clone();
        let mut al_ga = al.al_ga.borrow_mut();
        al_ga.reserve(g_entries.len());

        for ge in &g_entries {
            if ge.ae_fname.is_some() {
                al_ga.push(Aentry {
                    ae_fname: ge.ae_fname.clone(),
                    ae_fnum: ge.ae_fnum,
                });
            }
        }
    }
}

/// `:previous`, `:sprevious`, `:Next` and `:sNext` (`ex_previous`).
pub fn ex_previous(eap: &mut Exarg) {
    let count = argcount();
    let target = if curwin_mut().w_arg_idx - eap.line2 >= count {
        count - 1
    } else {
        curwin_mut().w_arg_idx - eap.line2
    };
    do_argfile(eap, target);
}

/// `:rewind`, `:first`, `:sfirst` and `:srewind` (`ex_rewind`).
pub fn ex_rewind(eap: &mut Exarg) {
    do_argfile(eap, 0);
}

/// `:last` and `:slast` (`ex_last`).
pub fn ex_last(eap: &mut Exarg) {
    do_argfile(eap, argcount() - 1);
}

/// `:argument` and `:sargument` (`ex_argument`).
pub fn ex_argument(eap: &mut Exarg) {
    let i = if eap.addr_count > 0 {
        eap.line2 - 1
    } else {
        curwin_mut().w_arg_idx
    };
    do_argfile(eap, i);
}

/// Edit file `argn` of the argument lists (`do_argfile`).
pub fn do_argfile(eap: &mut Exarg, argn: i32) {
    let is_split_cmd = eap.cmd.first_byte() == b's';
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
        && arg_entry(argn as usize).ae_fnum != curbuf_mut().b_fnum
        && !check_can_set_curbuf_forceit(eap.forceit)
    {
        return;
    }

    setpcmark();

    if is_split_cmd || g_st().cmdmod.cmod_tab != 0 {
        if !win_split(0, 0) {
            return;
        }
        curwin_mut().w_onebuf_opt.wo_scb = false;
        curwin_mut().w_onebuf_opt.wo_crb = false;
    } else {
        let mut other = true;
        if buf_hide(curbuf_mut()) {
            let entry = arg_entry(argn as usize);
            let p = fix_fname(alist_name(&entry));
            other = otherfile(p.as_deref());
        }
        if (!buf_hide(curbuf_mut()) || !other)
            && check_changed(
                curbuf_mut(),
                CCGD_AW
                    | if other { 0 } else { CCGD_MULTWIN }
                    | if eap.forceit { CCGD_FORCEIT } else { 0 }
                    | CCGD_EXCMD,
            )
        {
            return;
        }
    }

    curwin_mut().w_arg_idx = argn;
    if argn == argcount() - 1 && is_global_alist(&win_alist_rc(curwin_mut())) {
        g_st().arg_had_last = true;
    }

    let cur_entry = arg_entry(curwin_mut().w_arg_idx as usize);
    if !do_ecmd(
        0,
        alist_name(&cur_entry),
        None,
        Some(&*eap),
        ECMD_LAST,
        (if buf_hide(curwin_mut().buf_mut()) {
            ECMD_HIDE
        } else {
            0
        }) + (if eap.forceit { ECMD_FORCEIT } else { 0 }),
        Some(curwin_mut()),
    ) {
        curwin_mut().w_arg_idx = old_arg_idx;
    } else if eap.cmdidx != Cmdidx::CmdArgdo {
        setmark(b'\'' as i32);
    }
}

/// `:next`, and commands that behave like it (`ex_next`).
pub fn ex_next(eap: &mut Exarg) {
    if buf_hide(curbuf_mut())
        || eap.cmdidx == Cmdidx::CmdSnextVariant
        || !check_changed(
            curbuf_mut(),
            CCGD_AW | (if eap.forceit { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
        )
    {
        let i = if !eap.arg.is_empty() {
            if !do_arglist(eap.arg.as_cstr(), AL_SET, 0, true) {
                return;
            }
            0
        } else {
            curwin_mut().w_arg_idx + eap.line2
        };
        do_argfile(eap, i);
    }
}

/// `:argdedupe` (`ex_argdedupe`).
pub fn ex_argdedupe(_eap: &mut Exarg) {
    let mut i: i32 = 0;
    while i < argcount() {
        let first_entry = arg_entry(i as usize);
        let first_fullname = FullName_save(first_entry.fname_cstr(), false);
        let mut j = i + 1;
        while j < argcount() {
            let second_entry = arg_entry(j as usize);
            let second_fullname = FullName_save(second_entry.fname_cstr(), false);
            let dup = path_equal(
                first_fullname.as_deref().unwrap_or(c""),
                second_fullname.as_deref().unwrap_or(c""),
                K_PATH_CMP_LITERAL,
            );

            if dup {
                win_alist_rc(curwin_mut())
                    .al_ga
                    .borrow_mut()
                    .remove(j as usize);

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

/// `:argedit` (`ex_argedit`).
pub fn ex_argedit(eap: &mut Exarg) {
    let mut i = if eap.addr_count != 0 {
        eap.line2
    } else {
        curwin_mut().w_arg_idx + 1
    };
    let curbuf_is_reusable = curbuf_reusable();

    if !do_arglist(eap.arg.as_cstr(), AL_ADD, i, true) {
        return;
    }
    maketitle();

    if curwin_mut().w_arg_idx == 0
        && (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0
        && (curbuf_mut().b_ffname.is_none() || curbuf_is_reusable)
    {
        i = 0;
    }
    if i < argcount() {
        do_argfile(eap, i);
    }
}

/// `:argadd` (`ex_argadd`).
pub fn ex_argadd(eap: &mut Exarg) {
    do_arglist(
        eap.arg.as_cstr(),
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

/// `:argdelete` (`ex_argdelete`).
pub fn ex_argdelete(eap: &mut Exarg) {
    if !check_arglist_locked() {
        return;
    }

    if eap.addr_count > 0 || eap.arg.is_empty() {
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
        if !eap.arg.is_empty() {
            emsg(gettext(E_INVARG));
            return;
        }
        if n <= 0 {
            if eap.line1 != 1 || eap.line2 != 0 {
                emsg(gettext(E_INVRANGE));
            }
            return;
        }

        win_alist_rc(curwin_mut())
            .al_ga
            .borrow_mut()
            .drain((eap.line1 as usize - 1)..(eap.line2 as usize));
        if curwin_mut().w_arg_idx >= eap.line2 {
            curwin_mut().w_arg_idx -= n;
        } else if curwin_mut().w_arg_idx > eap.line1 {
            curwin_mut().w_arg_idx = eap.line1;
        }
    } else {
        do_arglist(eap.arg.as_cstr(), AL_DEL, 0, false);
    }

    let new_len = argcount();
    if new_len == 0 {
        curwin_mut().w_arg_idx = 0;
    } else if curwin_mut().w_arg_idx >= new_len {
        curwin_mut().w_arg_idx = new_len - 1;
    }

    maketitle();
}

/// Obtain the possible arguments of the `argedit` and `argdelete` commands (`get_arglist_name`).
pub fn get_arglist_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if idx < 0 || idx >= argcount() {
        return None;
    }
    let entry = arg_entry(idx as usize);
    xp.xp_item = alist_name(&entry).map(CStr::to_owned);
    xp.xp_item.as_deref()
}

/// Get the file name for an argument list entry (`alist_name`).
pub fn alist_name(aep: &Aentry) -> Option<&CStr> {
    if let Some(bp) = buflist_findnr_opt(aep.ae_fnum)
        && bp.fname_opt().is_some()
    {
        Some(bp.fname_cstr())
    } else {
        aep.ae_fname.as_deref()
    }
}

/// Close all the windows containing files which are not in the argument list (`arg_all_close_unused_windows`).
fn arg_all_close_unused_windows(aall: &mut ArgAllState) {
    let old_curwin = curwin_ptr();
    let old_curtab = g_st().curtab.clone();

    if aall.had_tab > 0 {
        let first_tp = g_st().first_tabpage.clone();
        goto_tabpage_tp(first_tp.opt(), true, true);
    }

    g_st().tabpage_move_disallowed += 1;
    loop {
        let mut tpnext = curtab_mut().tp_next.get();
        let mut wp = if lastwin_mut().w_floating {
            g_st().lastwin
        } else {
            g_st().firstwin
        };
        while !wp.is_null() {
            let mut wpnext = if wp.w_floating {
                if !wp.w_prev.is_null() && wp.w_prev.w_floating {
                    wp.w_prev
                } else {
                    g_st().firstwin
                }
            } else if wp.w_next.is_null() || wp.w_next.w_floating {
                crate::globals::WinPtr::NULL
            } else {
                wp.w_next
            };
            let buf = wp.w_buffer.borrow_mut();
            let aall_ref = &mut *aall;
            let opened_len = aall_ref.opened.len() as i32;
            let mut i: i32;
            if buf.b_ffname.is_none()
                || (!aall_ref.keep_tabs
                    && (buf.b_nwindows > 1
                        || wp.w_width != g_st().columns_global
                        || (wp.w_floating && !is_ctx_win(&wp))))
            {
                i = opened_len;
            } else {
                i = 0;
                while i < opened_len {
                    let al = &aall_ref.alist;
                    let entry_opt = al.al_ga.borrow().get(i as usize).cloned();
                    if let Some(ref entry) = entry_opt
                        && (entry.ae_fnum == buf.b_fnum
                            || path_equal(
                                alist_name(entry).unwrap_or(c""),
                                buf.ffname_cstr(),
                                K_PATH_CMP_EXPAND | K_PATH_CMP_FULL,
                            ))
                    {
                        let mut weight: i32 = 1;
                        if old_curtab == g_st().curtab {
                            weight += 1;
                            if old_curwin == wp {
                                weight += 1;
                            }
                        }

                        if weight > i32::from(aall_ref.opened[i as usize]) {
                            aall_ref.opened[i as usize] = weight as u8;
                            if i == 0 {
                                if !aall_ref.new_curwin.is_null() {
                                    aall_ref.new_curwin.w_arg_idx = opened_len;
                                }
                                aall_ref.new_curwin = wp;
                                aall_ref.new_curtab = g_st().curtab.clone();
                            }
                        } else if aall_ref.keep_tabs {
                            i = opened_len;
                        }

                        if !std::rc::Rc::ptr_eq(&win_alist_rc(&wp), al) {
                            wp.w_alist = aall_ref.local_alist.clone();
                        }
                        break;
                    }
                    i += 1;
                }
            }

            wp.w_arg_idx = i;

            if i == opened_len
                && !aall_ref.keep_tabs
                && (buf_hide(&*buf)
                    || aall_ref.forceit
                    || buf.b_nwindows > 1
                    || !buf_is_changed_fn(&mut *buf))
            {
                if !buf_hide(&*buf) && buf.b_nwindows <= 1 && buf_is_changed_fn(&mut *buf) {
                    let mut bufref: Bufref = Bufref::new();
                    set_bufref(&mut bufref, Some(&mut *buf));
                    autowrite(&mut *buf, false);
                    if !win_valid(wp.opt()) || !bufref_valid(Some(&bufref)) {
                        wp = if lastwin_mut().w_floating {
                            g_st().lastwin
                        } else {
                            g_st().firstwin
                        };
                        continue;
                    }
                }
                if g_st().firstwin == g_st().lastwin
                    && (crate::globals::first_tabpage_mut().tp_next.get().is_null()
                        || aall.had_tab == 0)
                {
                    aall.use_firstwin = true;
                } else {
                    let close_b = !buf_hide(&*buf) && !buf_is_changed_fn(&mut *buf);
                    win_close(wp.borrow_mut(), close_b, false);
                    if !win_valid(wpnext.opt()) {
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

        if !valid_tabpage(tpnext.opt()) {
            tpnext = g_st().first_tabpage.clone();
        }
        goto_tabpage_tp(tpnext.opt(), true, true);
    }
    g_st().tabpage_move_disallowed -= 1;
}

/// Open up to `count` windows for the files in the argument list `aall->alist` (`arg_all_open_windows`).
fn arg_all_open_windows(aall: &mut ArgAllState, count: i32) {
    let mut tab_drop_empty_window = false;

    if aall.keep_tabs
        && buf_is_empty(curbuf_mut())
        && curbuf_mut().b_nwindows == 1
        && curbuf_mut().b_ffname.is_none()
        && curbuf_mut().b_changed == 0
    {
        aall.use_firstwin = true;
        tab_drop_empty_window = true;
    }

    let mut split_ret = true;
    let mut i: i32 = 0;
    while i < count && !g_st().got_int {
        if is_global_alist(&aall.alist) && i == gargcount() - 1 {
            g_st().arg_had_last = true;
        }
        if aall.opened[i as usize] > 0 {
            if curwin_mut().w_arg_idx != i {
                for wp in for_all_windows_ptr() {
                    if wp.w_arg_idx == i {
                        if aall.keep_tabs {
                            aall.new_curwin = crate::globals::WinPtr::new(wp);
                            aall.new_curtab = g_st().curtab.clone();
                        } else if wp.w_floating {
                            break;
                        } else if wp.frame_mut().fr_parent.get()
                            != curwin_mut().frame_mut().fr_parent.get()
                        {
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
        } else if split_ret {
            if tab_drop_empty_window && i == count - 1 {
                g_st().autocmd_no_enter -= 1;
            }
            if !aall.use_firstwin {
                let p_ea_save = opt_vars().p_ea.get();
                opt_vars().p_ea.set(true);
                let ok = win_split(0, WSP_ROOM | WSP_BELOW);
                split_ret = ok;
                opt_vars().p_ea.set(p_ea_save);
                if !ok {
                    i += 1;
                    continue;
                }
            } else {
                g_st().autocmd_no_leave -= 1;
            }

            curwin_mut().w_arg_idx = i;
            if i == 0 {
                aall.new_curwin = crate::globals::curwin_ptr();
                aall.new_curtab = g_st().curtab.clone();
            }
            let entry = aall.alist.al_ga.borrow()[i as usize].clone();
            do_ecmd(
                0,
                alist_name(&entry),
                None,
                None,
                ECMD_ONE,
                (if buf_hide(curwin_mut().buf_mut()) || buf_is_changed_fn(curwin_mut().buf_mut()) {
                    ECMD_HIDE
                } else {
                    0
                }) + ECMD_OLDBUF,
                Some(curwin_mut()),
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

        if aall.had_tab > 0 && (tabpage_index(None) as OptInt) <= opt_vars().p_tpm.get() {
            g_st().cmdmod.cmod_tab = 9999;
        }
        i += 1;
    }
}

/// Open up to `count` windows, one for each argument (`do_arg_all`).
fn do_arg_all(mut count: i32, forceit: bool, keep_tabs: bool) {
    let prev_arglist_locked = ARGLIST_LOCKED.get();
    debug_assert!(!g_st().firstwin.is_null());

    let acount = argcount();
    if acount <= 0 {
        return;
    }
    setpcmark();

    let mut aall = ArgAllState {
        alist: win_alist_rc(curwin_mut()),
        local_alist: curwin_mut().w_alist.clone(),
        had_tab: g_st().cmdmod.cmod_tab,
        keep_tabs,
        forceit,
        use_firstwin: false,
        opened: vec![0u8; acount as usize],
        new_curwin: crate::globals::WinPtr::NULL,
        new_curtab: crate::globals::TabpagePtr::NULL,
    };

    ARGLIST_LOCKED.set(true);

    let new_lu_tp = g_st().curtab.clone();

    reset_visual_and_resel();

    arg_all_close_unused_windows(&mut aall);

    let opened_len = aall.opened.len() as i32;
    if count > opened_len || count <= 0 {
        count = opened_len;
    }

    g_st().autocmd_no_enter += 1;
    g_st().autocmd_no_leave += 1;
    let last_curwin = curwin_ptr();
    let last_curtab = g_st().curtab.clone();
    win_enter(lastwin_nofloating(None), false);

    arg_all_open_windows(&mut aall, count);

    drop(aall.local_alist.take());
    ARGLIST_LOCKED.set(prev_arglist_locked);

    g_st().autocmd_no_enter -= 1;

    if aall.new_curtab != last_curtab {
        if valid_tabpage(last_curtab.opt()) {
            goto_tabpage_tp(last_curtab.opt(), true, true);
        }
        if win_valid(last_curwin.opt()) {
            win_enter(last_curwin.borrow_mut(), false);
        }
    }
    if valid_tabpage(aall.new_curtab.opt()) {
        goto_tabpage_tp(aall.new_curtab.opt(), true, true);
    }

    if valid_tabpage(new_lu_tp.opt()) {
        g_st().lastused_tabpage = new_lu_tp;
    }

    if win_valid(aall.new_curwin.opt()) {
        win_enter(aall.new_curwin.borrow_mut(), false);
    }

    g_st().autocmd_no_leave -= 1;
}

/// `:all` and `:sall` (`ex_all`).
pub fn ex_all(eap: &mut Exarg) {
    if eap.addr_count == 0 {
        eap.line2 = 9999;
    }
    do_arg_all(eap.line2, eap.forceit, eap.cmdidx == Cmdidx::CmdDrop);
}

/// Concatenate all files in the argument list, separated by spaces (`arg_all`).
pub fn arg_all() -> CString {
    let mut bytes: Vec<u8> = Vec::new();
    let entries = win_alist_rc(curwin_mut()).al_ga.borrow().clone();
    for entry in &entries[..(argcount() as usize).min(entries.len())] {
        let Some(p) = alist_name(entry) else {
            continue;
        };
        if !bytes.is_empty() {
            bytes.push(b' ');
        }
        for &p_byte in p.to_bytes() {
            let ch = p_byte;
            let escape = ch == b' ' || ch == b'`' || (cfg!(not(windows)) && ch == b'\\');
            if escape {
                bytes.push(b'\\');
            }
            bytes.push(ch);
        }
    }
    CString::new(bytes).unwrap_or_default()
}

/// `argc([window id])` function (`f_argc`).
pub fn f_argc(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarUnknown {
        rettv.vval = crate::eval::typval::TypvalVval::Number(argcount() as Varnumber);
    } else if argvars[0].v_type == VarType::VarNumber && tv_get_number(&argvars[0]) == -1 {
        rettv.vval = crate::eval::typval::TypvalVval::Number(gargcount() as Varnumber);
    } else {
        let wp = find_win_by_nr_or_id(&argvars[0]);
        rettv.vval = crate::eval::typval::TypvalVval::Number(if let Some(wp_ref) = wp {
            wargcount(wp_ref) as Varnumber
        } else {
            -1
        });
    }
}

/// `argidx()` function (`f_argidx`).
pub fn f_argidx(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(curwin_mut().w_arg_idx as Varnumber);
}

/// `arglistid()` function (`f_arglistid`).
pub fn f_arglistid(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    if let Some(wp_ref) = find_tabwin(&argvars[0], &argvars[1]) {
        rettv.vval =
            crate::eval::typval::TypvalVval::Number(win_alist_rc(wp_ref).id.get() as Varnumber);
    }
}

fn get_arglist_as_rettv(entries: Option<&[Aentry]>, rettv: &mut TypVal) {
    let count = entries.map_or(0, <[Aentry]>::len);
    tv_list_alloc_ret(&mut *rettv, count as isize);
    if let Some(entries) = entries {
        for entry in entries {
            tv_list_append_string(rettv.vval.v_list_ref(), alist_name(entry), -1);
        }
    }
}

/// `argv(nr)` function (`f_argv`).
pub fn f_argv(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarUnknown {
        let entries = win_alist_rc(curwin_mut()).al_ga.borrow().clone();
        get_arglist_as_rettv(Some(&entries), rettv);
        return;
    }

    let entries: Option<Vec<Aentry>> = if argvars[1].v_type == VarType::VarUnknown {
        Some(win_alist_rc(curwin_mut()).al_ga.borrow().clone())
    } else if argvars[1].v_type == VarType::VarNumber && tv_get_number(&argvars[1]) == -1 {
        Some(global_alist_rc().al_ga.borrow().clone())
    } else {
        find_win_by_nr_or_id(&argvars[1]).map(|wp_ref| win_alist_rc(wp_ref).al_ga.borrow().clone())
    };

    tv_set_string(rettv, None);
    let idx = tv_get_number_chk(&argvars[0], None) as i32;
    if let Some(ref entries) = entries
        && idx >= 0
        && (idx as usize) < entries.len()
    {
        tv_set_string(rettv, alist_name(&entries[idx as usize]));
    } else if idx == -1 {
        get_arglist_as_rettv(entries.as_deref(), rettv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alist_init_and_clear_work() {
        let al = Alist::new();
        assert!(al.al_ga.borrow().is_empty());
        alist_clear(&al);
        assert!(al.al_ga.borrow().is_empty());
    }
}
