//! Functions for dealing with the argument list (`src/nvim/arglist.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::errors::*;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::command::{cmdidx_T, cmdmod_T, exarg_T};
use crate::api::tabpage::{tabpage_T, win_T};
use crate::api::win_config::bufref_T;
use crate::clipboard::{VarType, list_T, typval_T};
use crate::cmdhist::regmatch_T;
use crate::context::is_ctx_win;
use crate::eval::deprecated::{EvalFuncData, buf_T};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id};
use crate::ex_cmds2::{autowrite, check_changed};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::memory::{xcalloc, xfree, xmalloc, xstrdup};

pub type colnr_T = i32;
pub type linenr_T = i32;
pub type varnumber_T = i64;
pub type OptInt = i64;
pub type regprog_T = c_void;
pub type expand_T = c_void;
pub type PathCmpFlags = c_int;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const NSUBEXP: usize = 10;

const AL_SET: c_int = 1;
const AL_ADD: c_int = 2;
const AL_DEL: c_int = 3;

const EW_DIR: c_int = 0x01;
const EW_FILE: c_int = 0x02;
const EW_NOTFOUND: c_int = 0x04;
const EW_ADDSLASH: c_int = 0x08;
const EW_NOERROR: c_int = 0x200;
const EW_NOTWILD: c_int = 0x400;

const BLN_CURBUF: c_int = 1;
const BLN_LISTED: c_int = 2;

const RE_MAGIC: c_int = 1;

const kPathCmpLiteral: PathCmpFlags = 1 << 0;
const kPathCmpExpand: PathCmpFlags = 1 << 1;
const kPathCmpFull: PathCmpFlags = 1 << 2;

const CMD_args: cmdidx_T = 7;
const CMD_argdo: cmdidx_T = 10;
const CMD_argglobal: cmdidx_T = 13;
const CMD_arglocal: cmdidx_T = 14;
const CMD_drop: cmdidx_T = 132;
const CMD_snext: cmdidx_T = 420;

const ECMD_LAST: linenr_T = -1;
const ECMD_ONE: linenr_T = 1;
const ECMD_HIDE: c_int = 0x01;
const ECMD_OLDBUF: c_int = 0x04;
const ECMD_FORCEIT: c_int = 0x08;

const CCGD_AW: c_int = 1;
const CCGD_MULTWIN: c_int = 2;
const CCGD_FORCEIT: c_int = 4;
const CCGD_EXCMD: c_int = 16;

const WSP_ROOM: c_int = 0x01;
const WSP_BELOW: c_int = 0x40;
const ML_EMPTY: c_int = 0x01;

const E_WINDOW_LAYOUT_CHANGED_UNEXPECTEDLY: &CStr = c"E249: Window layout changed unexpectedly";

#[repr(C)]
pub struct alist_T {
    pub al_ga: garray_T,
    pub al_refcount: c_int,
    pub id: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct aentry_T {
    pub ae_fname: *mut c_char,
    pub ae_fnum: c_int,
}

struct arg_all_state_T {
    alist: *mut alist_T,
    had_tab: c_int,
    keep_tabs: bool,
    forceit: bool,
    use_firstwin: bool,
    opened: *mut u8,
    opened_len: c_int,
    new_curwin: *mut win_T,
    new_curtab: *mut tabpage_T,
}

static mut arglist_locked: bool = false;

unsafe extern "C" {
    static mut global_alist: alist_T;
    static mut max_alist_id: c_int;
    static mut arg_had_last: bool;
    static mut curwin: *mut win_T;
    static mut firstwin: *mut win_T;
    static mut lastwin: *mut win_T;
    static mut curtab: *mut tabpage_T;
    static mut first_tabpage: *mut tabpage_T;
    static mut lastused_tabpage: *mut tabpage_T;
    static mut curbuf: *mut buf_T;
    static mut p_su: *mut c_char;
    static mut empty_string_option: [c_char; 1];
    static mut got_int: bool;
    static mut p_fic: c_int;
    static mut p_magic: c_int;
    static mut p_ea: c_int;
    static mut p_tpm: OptInt;
    static mut cmdmod: cmdmod_T;
    static mut Columns: c_int;
    static mut tabpage_move_disallowed: c_int;
    static mut autocmd_no_enter: c_int;
    static mut autocmd_no_leave: c_int;

    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn expand_wildcards(
        num_pat: c_int,
        pat: *mut *mut c_char,
        num_file: *mut c_int,
        file: *mut *mut *mut c_char,
        flags: c_int,
    ) -> c_int;
    fn gen_expand_wildcards(
        num_pat: c_int,
        pat: *mut *mut c_char,
        num_file: *mut c_int,
        file: *mut *mut *mut c_char,
        flags: c_int,
    ) -> c_int;
    fn FreeWild(count: c_int, files: *mut *mut c_char);
    fn os_breakcheck();
    fn buf_set_name(fnum: c_int, name: *const c_char);
    fn buflist_add(fname: *mut c_char, flags: c_int) -> c_int;
    fn rem_backslash(str_: *const c_char) -> bool;
    fn skipwhite(q: *const c_char) -> *mut c_char;
    fn file_pat_to_reg_pat(
        pat: *const c_char,
        pat_end: *const c_char,
        allow_dirs: *mut c_char,
        no_bslash: c_int,
    ) -> *mut c_char;
    fn vim_regcomp(expr_arg: *const c_char, re_flags: c_int) -> *mut regprog_T;
    fn vim_regexec(rmp: *mut regmatch_T, line: *const c_char, col: colnr_T) -> bool;
    fn vim_regfree(prog: *mut regprog_T);
    fn path_equal(s1: *const c_char, s2: *const c_char, flags: PathCmpFlags) -> bool;
    fn gotocmdline(clr: bool);
    fn list_in_columns(items: *mut *mut c_char, count: c_int, curr: c_int);
    fn check_can_set_curbuf_forceit(forceit: c_int) -> bool;
    fn setpcmark();
    fn win_split(size: c_int, flags: c_int) -> c_int;
    fn buf_hide(buf: *const buf_T) -> bool;
    fn fix_fname(fname: *const c_char) -> *mut c_char;
    fn otherfile(ffname: *mut c_char) -> bool;
    fn do_ecmd(
        fnum: c_int,
        ffname: *mut c_char,
        sfname: *mut c_char,
        eap: *mut exarg_T,
        newlnum: linenr_T,
        flags: c_int,
        oldwin: *mut win_T,
    ) -> c_int;
    fn setmark(c: c_int) -> c_int;
    fn FullName_save(fname: *const c_char, force: bool) -> *mut c_char;
    fn curbuf_reusable() -> bool;
    fn maketitle();
    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn goto_tabpage_tp(
        tp: *mut tabpage_T,
        trigger_enter_autocmds: bool,
        trigger_leave_autocmds: bool,
    );
    fn bufIsChanged(buf: *mut buf_T) -> bool;
    fn set_bufref(bufref: *mut bufref_T, buf: *mut buf_T);
    fn bufref_valid(bufref: *mut bufref_T) -> bool;
    fn win_valid(win: *const win_T) -> bool;
    fn win_close(win: *mut win_T, free_buf: bool, force: bool) -> c_int;
    fn valid_tabpage(tpc: *mut tabpage_T) -> bool;
    fn buf_is_empty(buf: *mut buf_T) -> bool;
    fn win_move_after(wp1: *mut win_T, wp2: *mut win_T);
    fn tabpage_index(ftp: *mut tabpage_T) -> c_int;
    fn reset_VIsual_and_resel();
    fn win_enter(wp: *mut win_T, undo_sync: bool);
    fn lastwin_nofloating(tp: *mut tabpage_T) -> *mut win_T;
    fn tv_get_number(tv: *const typval_T) -> varnumber_T;
    fn tv_get_number_chk(tv: *const typval_T, ret_error: *mut bool) -> varnumber_T;
    fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T;
    fn tv_list_append_string(l: *mut list_T, str_: *const c_char, len: isize);
}

#[inline]
const fn ascii_isspace(c: c_char) -> bool {
    matches!(c as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[inline]
unsafe fn first_win_in_tab(tp: *mut tabpage_T) -> *mut win_T {
    unsafe {
        if tp == curtab {
            firstwin
        } else {
            (*tp).tp_firstwin
        }
    }
}

#[inline]
unsafe fn win_alist(wp: *const win_T) -> *mut alist_T {
    unsafe { (*wp).w_alist.cast::<alist_T>() }
}

#[inline]
unsafe fn aarglist(al: *const alist_T) -> *mut aentry_T {
    unsafe { (*al).al_ga.ga_data.cast::<aentry_T>() }
}

#[inline]
unsafe fn warglist(wp: *const win_T) -> *mut aentry_T {
    unsafe { aarglist(win_alist(wp)) }
}

#[inline]
unsafe fn wargcount(wp: *const win_T) -> c_int {
    unsafe { (*win_alist(wp)).al_ga.ga_len }
}

#[inline]
unsafe fn arglist() -> *mut aentry_T {
    unsafe { warglist(curwin) }
}

#[inline]
unsafe fn argcount() -> c_int {
    unsafe { wargcount(curwin) }
}

#[inline]
unsafe fn garglist() -> *mut aentry_T {
    unsafe { aarglist(&raw const global_alist) }
}

#[inline]
unsafe fn gargcount() -> c_int {
    unsafe { global_alist.al_ga.ga_len }
}

unsafe fn check_arglist_locked() -> c_int {
    unsafe {
        if arglist_locked {
            emsg(gettext(
                (&raw const e_cannot_change_arglist_recursively).cast(),
            ));
            return FAIL;
        }
        OK
    }
}

/// Clear an argument list: free all file names and reset it to zero entries (`alist_clear`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_clear(al: *mut alist_T) {
    unsafe {
        if check_arglist_locked() == FAIL {
            return;
        }
        let entries = aarglist(al);
        for i in 0..((*al).al_ga.ga_len as usize) {
            xfree((*entries.add(i)).ae_fname.cast());
        }
        ga_clear(&raw mut (*al).al_ga);
    }
}

/// Init an argument list (`alist_init`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_init(al: *mut alist_T) {
    unsafe {
        ga_init(&raw mut (*al).al_ga, size_of::<aentry_T>() as c_int, 5);
    }
}

/// Remove a reference from an argument list (`alist_unlink`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_unlink(al: *mut alist_T) {
    unsafe {
        if !ptr::eq(al, &raw const global_alist) {
            (*al).al_refcount -= 1;
            if (*al).al_refcount <= 0 {
                alist_clear(al);
                xfree(al.cast());
            }
        }
    }
}

/// Create a new argument list and use it for the current window (`alist_new`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_new() {
    unsafe {
        let al: *mut alist_T = xmalloc(size_of::<alist_T>()).cast();
        (*al).al_refcount = 1;
        max_alist_id += 1;
        (*al).id = max_alist_id;
        alist_init(al);
        (*curwin).w_alist = al.cast();
    }
}

/// Expand the file names in the global argument list (`alist_expand`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_expand(fnum_list: *mut c_int, fnum_len: c_int) {
    unsafe {
        let save_p_su = p_su;
        let old_arg_count = gargcount();
        let old_arg_files: *mut *mut c_char =
            xmalloc(size_of::<*mut c_char>() * (old_arg_count as usize)).cast();

        p_su = (&raw mut empty_string_option).cast();
        for i in 0..(old_arg_count as usize) {
            *old_arg_files.add(i) = xstrdup((*garglist().add(i)).ae_fname);
        }
        let mut new_arg_files: *mut *mut c_char = ptr::null_mut();
        let mut new_arg_file_count: c_int = 0;
        if expand_wildcards(
            old_arg_count,
            old_arg_files,
            &raw mut new_arg_file_count,
            &raw mut new_arg_files,
            EW_FILE | EW_NOTFOUND | EW_ADDSLASH | EW_NOERROR,
        ) == OK
            && new_arg_file_count > 0
        {
            alist_set(
                &raw mut global_alist,
                new_arg_file_count,
                new_arg_files,
                1,
                fnum_list,
                fnum_len,
            );
            FreeWild(old_arg_count, old_arg_files);
        }
        p_su = save_p_su;
    }
}

/// Set the argument list for the current window (`alist_set`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_set(
    al: *mut alist_T,
    count: c_int,
    files: *mut *mut c_char,
    use_curbuf: c_int,
    fnum_list: *mut c_int,
    fnum_len: c_int,
) {
    unsafe {
        if check_arglist_locked() == FAIL {
            return;
        }

        alist_clear(al);
        ga_grow(&raw mut (*al).al_ga, count);
        let mut i: c_int = 0;
        while i < count {
            if got_int {
                while i < count {
                    xfree((*files.add(i as usize)).cast());
                    i += 1;
                }
                break;
            }

            if !fnum_list.is_null() && i < fnum_len {
                arglist_locked = true;
                buf_set_name(*fnum_list.add(i as usize), *files.add(i as usize));
                arglist_locked = false;
            }

            alist_add(
                al,
                *files.add(i as usize),
                if use_curbuf != 0 { 2 } else { 1 },
            );
            os_breakcheck();
            i += 1;
        }
        xfree(files.cast());

        if ptr::eq(al, &raw const global_alist) {
            arg_had_last = false;
        }
    }
}

/// Add file `fname` to argument list `al` (`alist_add`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_add(al: *mut alist_T, fname: *mut c_char, set_fnum: c_int) {
    unsafe {
        let wp = curwin;
        if fname.is_null() {
            return;
        }
        if check_arglist_locked() == FAIL {
            return;
        }
        arglist_locked = true;
        (*wp).w_locked += 1;

        let idx = (*al).al_ga.ga_len as usize;
        (*aarglist(al).add(idx)).ae_fname = fname;
        if set_fnum > 0 {
            (*aarglist(al).add(idx)).ae_fnum = buflist_add(
                fname,
                BLN_LISTED | if set_fnum == 2 { BLN_CURBUF } else { 0 },
            );
        }
        (*al).al_ga.ga_len += 1;

        arglist_locked = false;
        (*wp).w_locked -= 1;
    }
}

/// Isolate one argument, taking backticks (`do_one_arg`).
unsafe fn do_one_arg(mut str_: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = str_;
        let mut inbacktick = false;
        while *str_ != NUL {
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
                if *str_ == b'`' as c_char {
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
unsafe fn get_arglist(gap: *mut garray_T, mut str_: *mut c_char, escaped: bool) {
    unsafe {
        ga_init(gap, size_of::<*mut c_char>() as c_int, 20);
        while *str_ != NUL {
            ga_grow(gap, 1);
            *((*gap).ga_data.cast::<*mut c_char>()).add((*gap).ga_len as usize) = str_;
            (*gap).ga_len += 1;

            if !escaped {
                return;
            }
            str_ = do_one_arg(str_);
        }
    }
}

/// Parse a list of arguments (file names), expand them and return in `fnames[fcountp]` (`get_arglist_exp`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_arglist_exp(
    str_: *mut c_char,
    fcountp: *mut c_int,
    fnamesp: *mut *mut *mut c_char,
    wig: bool,
) -> c_int {
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();
        get_arglist(&raw mut ga, str_, true);

        let i = if wig {
            expand_wildcards(
                ga.ga_len,
                ga.ga_data.cast(),
                fcountp,
                fnamesp,
                EW_FILE | EW_NOTFOUND | EW_NOTWILD,
            )
        } else {
            gen_expand_wildcards(
                ga.ga_len,
                ga.ga_data.cast(),
                fcountp,
                fnamesp,
                EW_FILE | EW_NOTFOUND | EW_NOTWILD,
            )
        };

        ga_clear(&raw mut ga);
        i
    }
}

/// Check the validity of the `arg_idx` for each other window (`alist_check_arg_idx`).
unsafe fn alist_check_arg_idx() {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut win = first_win_in_tab(tp);
            while !win.is_null() {
                if (*win).w_alist == (*curwin).w_alist {
                    check_arg_idx(win);
                }
                win = (*win).w_next;
            }
            tp = (*tp).tp_next;
        }
    }
}

/// Add `files[count]` to the arglist of the current window after arg `after` (`alist_add_list`).
unsafe fn alist_add_list(count: c_int, files: *mut *mut c_char, mut after: c_int, will_edit: bool) {
    unsafe {
        let old_argcount = argcount();
        if check_arglist_locked() != FAIL {
            let wp = curwin;
            let al = win_alist(wp);
            ga_grow(&raw mut (*al).al_ga, count);
            after = after.clamp(0, old_argcount);
            if after < old_argcount {
                let entries = aarglist(al);
                ptr::copy(
                    entries.add(after as usize),
                    entries.add((after + count) as usize),
                    (old_argcount - after) as usize,
                );
            }
            arglist_locked = true;
            (*wp).w_locked += 1;
            let entries = aarglist(al);
            for i in 0..(count as usize) {
                let flags = BLN_LISTED | if will_edit { BLN_CURBUF } else { 0 };
                let f = *files.add(i);
                (*entries.add(after as usize + i)).ae_fname = f;
                (*entries.add(after as usize + i)).ae_fnum = buflist_add(f, flags);
            }
            arglist_locked = false;
            (*wp).w_locked -= 1;
            (*al).al_ga.ga_len += count;
            if old_argcount > 0 && (*wp).w_arg_idx >= after {
                (*wp).w_arg_idx += count;
            }
        }
    }
}

/// Delete the file names in `alist_ga` from the argument list (`arglist_del_files`).
unsafe fn arglist_del_files(alist_ga: *mut garray_T) {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.rm_ic = p_fic != 0;

        let mut i: c_int = 0;
        while i < (*alist_ga).ga_len && !got_int {
            let orig_p = *((*alist_ga).ga_data.cast::<*mut c_char>()).add(i as usize);
            let p = file_pat_to_reg_pat(orig_p, ptr::null(), ptr::null_mut(), 0);
            if p.is_null() {
                break;
            }
            regmatch.regprog = vim_regcomp(p, if p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                xfree(p.cast());
                break;
            }

            let mut didone = false;
            let mut match_idx: c_int = 0;
            while match_idx < argcount() {
                let entries = arglist();
                if vim_regexec(
                    &raw mut regmatch,
                    alist_name(entries.add(match_idx as usize)),
                    0,
                ) {
                    didone = true;
                    xfree((*entries.add(match_idx as usize)).ae_fname.cast());
                    let cur_len = argcount();
                    ptr::copy(
                        entries.add((match_idx + 1) as usize),
                        entries.add(match_idx as usize),
                        (cur_len - match_idx - 1) as usize,
                    );
                    (*win_alist(curwin)).al_ga.ga_len -= 1;
                    if (*curwin).w_arg_idx > match_idx {
                        (*curwin).w_arg_idx -= 1;
                    }
                    match_idx -= 1;
                }
                match_idx += 1;
            }

            vim_regfree(regmatch.regprog);
            xfree(p.cast());
            if !didone {
                semsg(gettext((&raw const e_nomatch2).cast()), orig_p);
            }
            i += 1;
        }
        ga_clear(alist_ga);
    }
}

unsafe fn do_arglist(mut str_: *mut c_char, what: c_int, after: c_int, will_edit: bool) -> c_int {
    unsafe {
        let mut new_ga: garray_T = std::mem::zeroed();
        let mut exp_count: c_int = 0;
        let mut exp_files: *mut *mut c_char = ptr::null_mut();
        let mut arg_escaped = true;

        if check_arglist_locked() == FAIL {
            return FAIL;
        }

        if what == AL_ADD && *str_ == NUL {
            if (*curbuf).b_ffname.is_null() {
                return FAIL;
            }
            str_ = (*curbuf).b_fname;
            arg_escaped = false;
        }

        get_arglist(&raw mut new_ga, str_, arg_escaped);

        if what == AL_DEL {
            arglist_del_files(&raw mut new_ga);
        } else {
            let i = expand_wildcards(
                new_ga.ga_len,
                new_ga.ga_data.cast(),
                &raw mut exp_count,
                &raw mut exp_files,
                EW_DIR | EW_FILE | EW_ADDSLASH | EW_NOTFOUND,
            );
            ga_clear(&raw mut new_ga);
            if i == FAIL || exp_count == 0 {
                emsg(gettext((&raw const e_nomatch).cast()));
                return FAIL;
            }

            if what == AL_ADD {
                alist_add_list(exp_count, exp_files, after, will_edit);
                xfree(exp_files.cast());
            } else {
                debug_assert_eq!(what, AL_SET);
                alist_set(
                    win_alist(curwin),
                    exp_count,
                    exp_files,
                    c_int::from(will_edit),
                    ptr::null_mut(),
                    0,
                );
            }
        }

        alist_check_arg_idx();
        OK
    }
}

/// Redefine the argument list (`set_arglist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_arglist(str_: *mut c_char) {
    unsafe {
        do_arglist(str_, AL_SET, 0, true);
    }
}

/// Return true if window `win` is editing the file at the current argument index (`editing_arg_idx`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn editing_arg_idx(win: *mut win_T) -> bool {
    unsafe {
        let idx = (*win).w_arg_idx;
        if idx >= wargcount(win) {
            return false;
        }
        let entry = warglist(win).add(idx as usize);
        if (*(*win).w_buffer).b_fnum == (*entry).ae_fnum {
            return true;
        }
        let ffname = (*(*win).w_buffer).b_ffname;
        !ffname.is_null() && path_equal(alist_name(entry), ffname, kPathCmpExpand | kPathCmpFull)
    }
}

/// Check if window `win` is editing the `w_arg_idx` file in its argument list (`check_arg_idx`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_arg_idx(win: *mut win_T) {
    unsafe {
        let wcount = wargcount(win);
        if wcount > 1 && !editing_arg_idx(win) {
            (*win).w_arg_idx_invalid = true;
            let gcount = gargcount();
            if (*win).w_arg_idx != wcount - 1
                && !arg_had_last
                && ptr::eq(win_alist(win), &raw const global_alist)
                && gcount > 0
                && (*win).w_arg_idx < gcount
            {
                let last_entry = garglist().add((gcount - 1) as usize);
                let buf = (*win).w_buffer;
                if (*buf).b_fnum == (*last_entry).ae_fnum
                    || (!(*buf).b_ffname.is_null()
                        && path_equal(
                            alist_name(last_entry),
                            (*buf).b_ffname,
                            kPathCmpExpand | kPathCmpFull,
                        ))
                {
                    arg_had_last = true;
                }
            }
        } else {
            (*win).w_arg_idx_invalid = false;
            if (*win).w_arg_idx == wcount - 1 && ptr::eq(win_alist(win), &raw const global_alist) {
                arg_had_last = true;
            }
        }
    }
}

/// `:args`, `:arglocal` and `:argglobal` (`ex_args`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_args(eap: *mut exarg_T) {
    unsafe {
        if (*eap).cmdidx != CMD_args {
            if check_arglist_locked() == FAIL {
                return;
            }
            alist_unlink(win_alist(curwin));
            if (*eap).cmdidx == CMD_argglobal {
                (*curwin).w_alist = (&raw mut global_alist).cast();
            } else {
                alist_new();
            }
        }

        if *(*eap).arg != NUL {
            if check_arglist_locked() == FAIL {
                return;
            }
            ex_next(eap);
            return;
        }

        if (*eap).cmdidx == CMD_args {
            let count = argcount();
            if count <= 0 {
                return;
            }

            let items: *mut *mut c_char =
                xmalloc(size_of::<*mut c_char>() * (count as usize)).cast();
            gotocmdline(true);

            let entries = arglist();
            for i in 0..(count as usize) {
                *items.add(i) = alist_name(entries.add(i));
            }
            list_in_columns(items, count, (*curwin).w_arg_idx);
            xfree(items.cast());
            return;
        }

        if (*eap).cmdidx == CMD_arglocal {
            let al = win_alist(curwin);
            let gap = &raw mut (*al).al_ga;
            let gcount = gargcount();
            ga_grow(gap, gcount);

            for i in 0..(gcount as usize) {
                let ge = *garglist().add(i);
                if !ge.ae_fname.is_null() {
                    let dst = aarglist(al).add((*gap).ga_len as usize);
                    (*dst).ae_fname = xstrdup(ge.ae_fname);
                    (*dst).ae_fnum = ge.ae_fnum;
                    (*gap).ga_len += 1;
                }
            }
        }
    }
}

/// `:previous`, `:sprevious`, `:Next` and `:sNext` (`ex_previous`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_previous(eap: *mut exarg_T) {
    unsafe {
        let count = argcount();
        let target = if (*curwin).w_arg_idx - (*eap).line2 >= count {
            count - 1
        } else {
            (*curwin).w_arg_idx - (*eap).line2
        };
        do_argfile(eap, target);
    }
}

/// `:rewind`, `:first`, `:sfirst` and `:srewind` (`ex_rewind`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_rewind(eap: *mut exarg_T) {
    unsafe {
        do_argfile(eap, 0);
    }
}

/// `:last` and `:slast` (`ex_last`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_last(eap: *mut exarg_T) {
    unsafe {
        do_argfile(eap, argcount() - 1);
    }
}

/// `:argument` and `:sargument` (`ex_argument`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_argument(eap: *mut exarg_T) {
    unsafe {
        let i = if (*eap).addr_count > 0 {
            (*eap).line2 - 1
        } else {
            (*curwin).w_arg_idx
        };
        do_argfile(eap, i);
    }
}

/// Edit file `argn` of the argument lists (`do_argfile`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_argfile(eap: *mut exarg_T, argn: c_int) {
    unsafe {
        let is_split_cmd = *(*eap).cmd == b's' as c_char;
        let old_arg_idx = (*curwin).w_arg_idx;
        let count = argcount();

        if argn < 0 || argn >= count {
            if count <= 1 {
                emsg(gettext(c"E163: There is only one file to edit".as_ptr()));
            } else if argn < 0 {
                emsg(gettext(c"E164: Cannot go before first file".as_ptr()));
            } else {
                emsg(gettext(c"E165: Cannot go beyond last file".as_ptr()));
            }
            return;
        }

        if !is_split_cmd
            && (*arglist().add(argn as usize)).ae_fnum != (*curbuf).b_fnum
            && !check_can_set_curbuf_forceit((*eap).forceit)
        {
            return;
        }

        setpcmark();

        if is_split_cmd || cmdmod.cmod_tab != 0 {
            if win_split(0, 0) == FAIL {
                return;
            }
            (*curwin).w_onebuf_opt.wo_scb = 0;
            (*curwin).w_onebuf_opt.wo_crb = 0;
        } else {
            let mut other = true;
            if buf_hide(curbuf) {
                let p = fix_fname(alist_name(arglist().add(argn as usize)));
                other = otherfile(p);
                xfree(p.cast());
            }
            if (!buf_hide(curbuf) || !other)
                && check_changed(
                    curbuf,
                    CCGD_AW
                        | if other { 0 } else { CCGD_MULTWIN }
                        | if (*eap).forceit != 0 { CCGD_FORCEIT } else { 0 }
                        | CCGD_EXCMD,
                )
            {
                return;
            }
        }

        (*curwin).w_arg_idx = argn;
        if argn == argcount() - 1 && ptr::eq(win_alist(curwin), &raw const global_alist) {
            arg_had_last = true;
        }

        if do_ecmd(
            0,
            alist_name(arglist().add((*curwin).w_arg_idx as usize)),
            ptr::null_mut(),
            eap,
            ECMD_LAST,
            (if buf_hide((*curwin).w_buffer) {
                ECMD_HIDE
            } else {
                0
            }) + (if (*eap).forceit != 0 { ECMD_FORCEIT } else { 0 }),
            curwin,
        ) == FAIL
        {
            (*curwin).w_arg_idx = old_arg_idx;
        } else if (*eap).cmdidx != CMD_argdo {
            setmark(b'\'' as c_int);
        }
    }
}

/// `:next`, and commands that behave like it (`ex_next`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_next(eap: *mut exarg_T) {
    unsafe {
        if buf_hide(curbuf)
            || (*eap).cmdidx == CMD_snext
            || !check_changed(
                curbuf,
                CCGD_AW | (if (*eap).forceit != 0 { CCGD_FORCEIT } else { 0 }) | CCGD_EXCMD,
            )
        {
            let i = if *(*eap).arg != NUL {
                if do_arglist((*eap).arg, AL_SET, 0, true) == FAIL {
                    return;
                }
                0
            } else {
                (*curwin).w_arg_idx + (*eap).line2
            };
            do_argfile(eap, i);
        }
    }
}

/// `:argdedupe` (`ex_argdedupe`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_argdedupe(_eap: *mut exarg_T) {
    unsafe {
        let mut i: c_int = 0;
        while i < argcount() {
            let first_fullname = FullName_save((*arglist().add(i as usize)).ae_fname, false);

            let mut j = i + 1;
            while j < argcount() {
                let second_fullname = FullName_save((*arglist().add(j as usize)).ae_fname, false);
                let dup = path_equal(first_fullname, second_fullname, kPathCmpLiteral);
                xfree(second_fullname.cast());

                if dup {
                    let entries = arglist();
                    xfree((*entries.add(j as usize)).ae_fname.cast());
                    let cur_len = argcount();
                    ptr::copy(
                        entries.add((j + 1) as usize),
                        entries.add(j as usize),
                        (cur_len - j - 1) as usize,
                    );
                    (*win_alist(curwin)).al_ga.ga_len -= 1;

                    if (*curwin).w_arg_idx == j {
                        (*curwin).w_arg_idx = i;
                    } else if (*curwin).w_arg_idx > j {
                        (*curwin).w_arg_idx -= 1;
                    }

                    j -= 1;
                }
                j += 1;
            }

            xfree(first_fullname.cast());
            i += 1;
        }
    }
}

/// `:argedit` (`ex_argedit`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_argedit(eap: *mut exarg_T) {
    unsafe {
        let mut i = if (*eap).addr_count != 0 {
            (*eap).line2
        } else {
            (*curwin).w_arg_idx + 1
        };
        let curbuf_is_reusable = curbuf_reusable();

        if do_arglist((*eap).arg, AL_ADD, i, true) == FAIL {
            return;
        }
        maketitle();

        if (*curwin).w_arg_idx == 0
            && ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0
            && ((*curbuf).b_ffname.is_null() || curbuf_is_reusable)
        {
            i = 0;
        }
        if i < argcount() {
            do_argfile(eap, i);
        }
    }
}

/// `:argadd` (`ex_argadd`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_argadd(eap: *mut exarg_T) {
    unsafe {
        do_arglist(
            (*eap).arg,
            AL_ADD,
            if (*eap).addr_count > 0 {
                (*eap).line2
            } else {
                (*curwin).w_arg_idx + 1
            },
            false,
        );
        maketitle();
    }
}

/// `:argdelete` (`ex_argdelete`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_argdelete(eap: *mut exarg_T) {
    unsafe {
        if check_arglist_locked() == FAIL {
            return;
        }

        if (*eap).addr_count > 0 || *(*eap).arg == NUL {
            if (*eap).addr_count == 0 {
                if (*curwin).w_arg_idx >= argcount() {
                    emsg(gettext(c"E610: No argument to delete".as_ptr()));
                    return;
                }
                (*eap).line1 = (*curwin).w_arg_idx + 1;
                (*eap).line2 = (*eap).line1;
            } else if (*eap).line2 > argcount() {
                (*eap).line2 = argcount();
            }
            let n = (*eap).line2 - (*eap).line1 + 1;
            if *(*eap).arg != NUL {
                emsg(gettext((&raw const e_invarg).cast()));
                return;
            }
            if n <= 0 {
                if (*eap).line1 != 1 || (*eap).line2 != 0 {
                    emsg(gettext((&raw const e_invrange).cast()));
                }
                return;
            }

            let entries = arglist();
            for i in (*eap).line1..=(*eap).line2 {
                xfree((*entries.add((i - 1) as usize)).ae_fname.cast());
            }
            let cur_len = argcount();
            ptr::copy(
                entries.add((*eap).line2 as usize),
                entries.add(((*eap).line1 - 1) as usize),
                (cur_len - (*eap).line2) as usize,
            );
            (*win_alist(curwin)).al_ga.ga_len -= n;
            if (*curwin).w_arg_idx >= (*eap).line2 {
                (*curwin).w_arg_idx -= n;
            } else if (*curwin).w_arg_idx > (*eap).line1 {
                (*curwin).w_arg_idx = (*eap).line1;
            }
        } else {
            do_arglist((*eap).arg, AL_DEL, 0, false);
        }

        let new_len = argcount();
        if new_len == 0 {
            (*curwin).w_arg_idx = 0;
        } else if (*curwin).w_arg_idx >= new_len {
            (*curwin).w_arg_idx = new_len - 1;
        }

        maketitle();
    }
}

/// Obtain the possible arguments of the `argedit` and `argdelete` commands (`get_arglist_name`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_arglist_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx >= argcount() {
            return ptr::null_mut();
        }
        alist_name(arglist().add(idx as usize))
    }
}

/// Get the file name for an argument list entry (`alist_name`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn alist_name(aep: *mut aentry_T) -> *mut c_char {
    unsafe {
        let bp = buflist_findnr((*aep).ae_fnum);
        if bp.is_null() || (*bp).b_fname.is_null() {
            (*aep).ae_fname
        } else {
            (*bp).b_fname
        }
    }
}

/// Close all the windows containing files which are not in the argument list (`arg_all_close_unused_windows`).
unsafe fn arg_all_close_unused_windows(aall: *mut arg_all_state_T) {
    unsafe {
        let old_curwin = curwin;
        let old_curtab = curtab;

        if (*aall).had_tab > 0 {
            goto_tabpage_tp(first_tabpage, true, true);
        }

        tabpage_move_disallowed += 1;
        loop {
            let mut tpnext = (*curtab).tp_next;
            let mut wp = if (*lastwin).w_floating {
                lastwin
            } else {
                firstwin
            };
            while !wp.is_null() {
                let mut wpnext = if (*wp).w_floating {
                    if !(*wp).w_prev.is_null() && (*(*wp).w_prev).w_floating {
                        (*wp).w_prev
                    } else {
                        firstwin
                    }
                } else if (*wp).w_next.is_null() || (*(*wp).w_next).w_floating {
                    ptr::null_mut()
                } else {
                    (*wp).w_next
                };
                let buf = (*wp).w_buffer;
                let mut i: c_int;
                if (*buf).b_ffname.is_null()
                    || (!(*aall).keep_tabs
                        && ((*buf).b_nwindows > 1
                            || (*wp).w_width != Columns
                            || ((*wp).w_floating && !is_ctx_win(wp))))
                {
                    i = (*aall).opened_len;
                } else {
                    i = 0;
                    while i < (*aall).opened_len {
                        let al = (*aall).alist;
                        if i < (*al).al_ga.ga_len {
                            let entry = aarglist(al).add(i as usize);
                            if (*entry).ae_fnum == (*buf).b_fnum
                                || path_equal(
                                    alist_name(entry),
                                    (*buf).b_ffname,
                                    kPathCmpExpand | kPathCmpFull,
                                )
                            {
                                let mut weight: c_int = 1;
                                if old_curtab == curtab {
                                    weight += 1;
                                    if old_curwin == wp {
                                        weight += 1;
                                    }
                                }

                                if weight > c_int::from(*(*aall).opened.add(i as usize)) {
                                    *(*aall).opened.add(i as usize) = weight as u8;
                                    if i == 0 {
                                        if !(*aall).new_curwin.is_null() {
                                            (*(*aall).new_curwin).w_arg_idx = (*aall).opened_len;
                                        }
                                        (*aall).new_curwin = wp;
                                        (*aall).new_curtab = curtab;
                                    }
                                } else if (*aall).keep_tabs {
                                    i = (*aall).opened_len;
                                }

                                if win_alist(wp) != al {
                                    alist_unlink(win_alist(wp));
                                    (*wp).w_alist = al.cast();
                                    (*al).al_refcount += 1;
                                }
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                (*wp).w_arg_idx = i;

                if i == (*aall).opened_len
                    && !(*aall).keep_tabs
                    && (buf_hide(buf)
                        || (*aall).forceit
                        || (*buf).b_nwindows > 1
                        || !bufIsChanged(buf))
                {
                    if !buf_hide(buf) && (*buf).b_nwindows <= 1 && bufIsChanged(buf) {
                        let mut bufref: bufref_T = std::mem::zeroed();
                        set_bufref(&raw mut bufref, buf);
                        autowrite(buf, false);
                        if !win_valid(wp) || !bufref_valid(&raw mut bufref) {
                            wp = if (*lastwin).w_floating {
                                lastwin
                            } else {
                                firstwin
                            };
                            continue;
                        }
                    }
                    if firstwin == lastwin
                        && ((*first_tabpage).tp_next.is_null() || (*aall).had_tab == 0)
                    {
                        (*aall).use_firstwin = true;
                    } else {
                        win_close(wp, !buf_hide(buf) && !bufIsChanged(buf), false);
                        if !win_valid(wpnext) {
                            wpnext = if (*lastwin).w_floating {
                                lastwin
                            } else {
                                firstwin
                            };
                        }
                    }
                }
                wp = wpnext;
            }

            if (*aall).had_tab == 0 || tpnext.is_null() {
                break;
            }

            if !valid_tabpage(tpnext) {
                tpnext = first_tabpage;
            }
            goto_tabpage_tp(tpnext, true, true);
        }
        tabpage_move_disallowed -= 1;
    }
}

/// Open up to `count` windows for the files in the argument list `aall->alist` (`arg_all_open_windows`).
unsafe fn arg_all_open_windows(aall: *mut arg_all_state_T, count: c_int) {
    unsafe {
        let mut tab_drop_empty_window = false;

        if (*aall).keep_tabs
            && buf_is_empty(curbuf)
            && (*curbuf).b_nwindows == 1
            && (*curbuf).b_ffname.is_null()
            && (*curbuf).b_changed == 0
        {
            (*aall).use_firstwin = true;
            tab_drop_empty_window = true;
        }

        let mut split_ret = OK;
        let mut i: c_int = 0;
        while i < count && !got_int {
            if ptr::eq((*aall).alist, &raw const global_alist) && i == global_alist.al_ga.ga_len - 1
            {
                arg_had_last = true;
            }
            if *(*aall).opened.add(i as usize) > 0 {
                if (*curwin).w_arg_idx != i {
                    let mut wp = firstwin;
                    while !wp.is_null() {
                        if (*wp).w_arg_idx == i {
                            if (*aall).keep_tabs {
                                (*aall).new_curwin = wp;
                                (*aall).new_curtab = curtab;
                            } else if (*wp).w_floating {
                                break;
                            } else if (*(*wp).w_frame).fr_parent != (*(*curwin).w_frame).fr_parent {
                                emsg(gettext(E_WINDOW_LAYOUT_CHANGED_UNEXPECTEDLY.as_ptr()));
                                i = count;
                                break;
                            } else {
                                win_move_after(wp, curwin);
                            }
                            break;
                        }
                        wp = (*wp).w_next;
                    }
                }
            } else if split_ret == OK {
                if tab_drop_empty_window && i == count - 1 {
                    autocmd_no_enter -= 1;
                }
                if !(*aall).use_firstwin {
                    let p_ea_save = p_ea;
                    p_ea = 1;
                    split_ret = win_split(0, WSP_ROOM | WSP_BELOW);
                    p_ea = p_ea_save;
                    if split_ret == FAIL {
                        i += 1;
                        continue;
                    }
                } else {
                    autocmd_no_leave -= 1;
                }

                (*curwin).w_arg_idx = i;
                if i == 0 {
                    (*aall).new_curwin = curwin;
                    (*aall).new_curtab = curtab;
                }
                do_ecmd(
                    0,
                    alist_name(aarglist((*aall).alist).add(i as usize)),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ECMD_ONE,
                    (if buf_hide((*curwin).w_buffer) || bufIsChanged((*curwin).w_buffer) {
                        ECMD_HIDE
                    } else {
                        0
                    }) + ECMD_OLDBUF,
                    curwin,
                );
                if tab_drop_empty_window && i == count - 1 {
                    autocmd_no_enter += 1;
                }
                if (*aall).use_firstwin {
                    autocmd_no_leave += 1;
                }
                (*aall).use_firstwin = false;
            }
            os_breakcheck();

            if (*aall).had_tab > 0 && (tabpage_index(ptr::null_mut()) as OptInt) <= p_tpm {
                cmdmod.cmod_tab = 9999;
            }
            i += 1;
        }
    }
}

/// Open up to `count` windows, one for each argument (`do_arg_all`).
unsafe fn do_arg_all(mut count: c_int, forceit: c_int, keep_tabs: bool) {
    unsafe {
        let prev_arglist_locked = arglist_locked;
        debug_assert!(!firstwin.is_null());

        let acount = argcount();
        if acount <= 0 {
            return;
        }
        setpcmark();

        let mut aall = arg_all_state_T {
            alist: win_alist(curwin),
            had_tab: cmdmod.cmod_tab,
            keep_tabs,
            forceit: forceit != 0,
            use_firstwin: false,
            opened: xcalloc(acount as usize, 1).cast(),
            opened_len: acount,
            new_curwin: ptr::null_mut(),
            new_curtab: ptr::null_mut(),
        };

        (*aall.alist).al_refcount += 1;
        arglist_locked = true;

        let new_lu_tp = curtab;

        reset_VIsual_and_resel();

        arg_all_close_unused_windows(&raw mut aall);

        if count > aall.opened_len || count <= 0 {
            count = aall.opened_len;
        }

        autocmd_no_enter += 1;
        autocmd_no_leave += 1;
        let last_curwin = curwin;
        let last_curtab = curtab;
        win_enter(lastwin_nofloating(ptr::null_mut()), false);

        arg_all_open_windows(&raw mut aall, count);

        alist_unlink(aall.alist);
        arglist_locked = prev_arglist_locked;

        autocmd_no_enter -= 1;

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
            lastused_tabpage = new_lu_tp;
        }

        if win_valid(aall.new_curwin) {
            win_enter(aall.new_curwin, false);
        }

        autocmd_no_leave -= 1;
        xfree(aall.opened.cast());
    }
}

/// `:all` and `:sall` (`ex_all`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_all(eap: *mut exarg_T) {
    unsafe {
        if (*eap).addr_count == 0 {
            (*eap).line2 = 9999;
        }
        do_arg_all((*eap).line2, (*eap).forceit, (*eap).cmdidx == CMD_drop);
    }
}

/// Concatenate all files in the argument list, separated by spaces (`arg_all`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn arg_all() -> *mut c_char {
    unsafe {
        let mut retval: *mut c_char = ptr::null_mut();

        loop {
            let mut len: usize = 0;
            let count = argcount();
            for idx in 0..(count as usize) {
                let mut p = alist_name(arglist().add(idx));
                if p.is_null() {
                    continue;
                }
                if len > 0 {
                    if !retval.is_null() {
                        *retval.add(len) = b' ' as c_char;
                    }
                    len += 1;
                }
                while *p != NUL {
                    let ch = *p as u8;
                    let escape = ch == b' ' || ch == b'`' || (cfg!(not(windows)) && ch == b'\\');
                    if escape {
                        if !retval.is_null() {
                            *retval.add(len) = b'\\' as c_char;
                        }
                        len += 1;
                    }
                    if !retval.is_null() {
                        *retval.add(len) = *p;
                    }
                    len += 1;
                    p = p.add(1);
                }
            }

            if !retval.is_null() {
                *retval.add(len) = NUL;
                break;
            }

            retval = xmalloc(len + 1).cast();
        }

        retval
    }
}

/// `argc([window id])` function (`f_argc`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_argc(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            (*rettv).vval.v_number = argcount() as varnumber_T;
        } else if (*argvars).v_type == VarType::VAR_NUMBER && tv_get_number(argvars) == -1 {
            (*rettv).vval.v_number = gargcount() as varnumber_T;
        } else {
            let wp = find_win_by_nr_or_id(argvars);
            (*rettv).vval.v_number = if !wp.is_null() {
                wargcount(wp) as varnumber_T
            } else {
                -1
            };
        }
    }
}

/// `argidx()` function (`f_argidx`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_argidx(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = (*curwin).w_arg_idx as varnumber_T;
    }
}

/// `arglistid()` function (`f_arglistid`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_arglistid(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
        let wp = find_tabwin(argvars, argvars.add(1));
        if !wp.is_null() {
            (*rettv).vval.v_number = (*win_alist(wp)).id as varnumber_T;
        }
    }
}

unsafe fn get_arglist_as_rettv(arglist_ptr: *mut aentry_T, count: c_int, rettv: *mut typval_T) {
    unsafe {
        tv_list_alloc_ret(rettv, count as isize);
        if !arglist_ptr.is_null() {
            for idx in 0..(count as usize) {
                tv_list_append_string((*rettv).vval.v_list, alist_name(arglist_ptr.add(idx)), -1);
            }
        }
    }
}

/// `argv(nr)` function (`f_argv`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_argv(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut al_ptr: *mut aentry_T = ptr::null_mut();
        let mut count: c_int = -1;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            get_arglist_as_rettv(arglist(), argcount(), rettv);
            return;
        }

        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            al_ptr = arglist();
            count = argcount();
        } else if (*argvars.add(1)).v_type == VarType::VAR_NUMBER
            && tv_get_number(argvars.add(1)) == -1
        {
            al_ptr = garglist();
            count = gargcount();
        } else {
            let wp = find_win_by_nr_or_id(argvars.add(1));
            if !wp.is_null() {
                al_ptr = warglist(wp);
                count = wargcount(wp);
            }
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        let idx = tv_get_number_chk(argvars, ptr::null_mut()) as c_int;
        if !al_ptr.is_null() && idx >= 0 && idx < count {
            (*rettv).vval.v_string = xstrdup(alist_name(al_ptr.add(idx as usize)));
        } else if idx == -1 {
            get_arglist_as_rettv(al_ptr, count, rettv);
        }
    }
}
