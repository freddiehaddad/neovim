//! User defined function support (`src/nvim/eval/userfunc.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_FUNCUNDEFINED, PROF_YES, apply_autocmds,
};
use crate::charset::{
    NUL, ascii_isalnum, ascii_islower, ascii_iswhite, getdigits, skiptowhite, skipwhite,
    vim_strsize,
};
use crate::cmdexpand::{EXPAND_USER_FUNC, cmdidx_T, kUICmdline};
use crate::cursor::linenr_T;
use crate::debugger::{dbg_breakpoint, dbg_find_breakpoint, has_profiling};
use crate::errors::*;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::funcs::{
    MAX_FUNC_ARGS, TFN_INT, TFN_NO_DEREF, call_internal_func, call_internal_method,
    check_internal_func, find_internal_func,
};
use crate::eval::typval::{
    Callback, EvalFuncData, TV_CSTRING, VAR_UNLOCKED, VarType, dict_T, dictitem_T, listitem_T,
    partial_T, tv_clear, tv_copy, tv_dict_add, tv_dict_hi2di, tv_dict_item_alloc,
    tv_dict_item_alloc_len, tv_dict_item_remove, tv_dict_unref, tv_get_number_chk, tv_is_func,
    tv_list_append, tv_list_init_static, tv_list_set_lock, typval_T, value_check_lock, varnumber_T,
};
use crate::eval::vars::{
    VV_TESTING, evalarg_T, find_var, find_var_ht, find_var_in_ht, get_vim_var_nr, init_var_dict,
    list_hashtable_vars, lval_T, skip_var_list, vars_clear, vars_clear_ext,
};
use crate::eval::{
    AUTOLOAD_CHAR, EVAL_EVALUATE, FNE_CHECK_START, FNE_INCL_BR, GLV_READ_ONLY, TFN_NO_AUTOLOAD,
    TFN_QUIET, VAR_DEF_SCOPE, callback_call, check_luafunc_name, clear_evalarg, eval_isnamec,
    eval_isnamec1, eval_lavars_used, eval0, eval1, fill_evalarg_from_eap, find_name_end,
    garbage_collect, get_id_len, get_lval, handle_subscript, is_luafunc, last_set_msg,
    partial_name, partial_unref, set_ref_in_ht, set_ref_in_item, set_ref_in_list_items, skip_expr,
};
use crate::ex_docmd::{check_nextcmd, checkforcmd, do_cmdline, ends_excmd, exarg_T, skip_range};
use crate::ex_eval::{
    CSTP_RETURN, PendingVal, aborted_in_try, aborting, cleanup_conditionals, cstack_T,
    exception_state_T, exception_state_clear, exception_state_restore, exception_state_save,
    report_make_pending, update_force_abort,
};
use crate::ex_getln::{getcmdline, ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave};
use crate::globals::*;
use crate::hashtab::{hash_add, hash_find_len, hashitem_T, hashitem_empty, hashtab_T};
use crate::input::{RedoState, restore_redobuff, save_redobuff};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_SPECIAL, KE_SNR, KS_EXTRA};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{
    api_free_luaref, nlua_call_vlua, nlua_exec_typval_callable, nlua_set_sctx,
};
use crate::math::{FAIL, OK};
use crate::mbyte::mb_strnicmp;
use crate::memory::{
    entered_free_all_mem, xfree, xmalloc, xmallocz, xmemcpyz, xmemdupz, xmemrchr, xstrdup, xstrlcpy,
};
use crate::message::{
    emsg, iemsg, internal_error, message_filtered, msg_clr_eos, msg_ext_set_kind, msg_outnum,
    msg_prt_line, msg_putchar, msg_puts, msg_start, semsg, smsg, swmsg, trunc_string,
    verbose_enter_scroll, verbose_leave_scroll,
};
use crate::option_vars::*;
use crate::os::env::expand_T;
use crate::os::input::line_breakcheck;
use crate::path::{kPathCmpLiteral, path_equal};
use crate::profile::{
    func_do_profile, func_line_end, func_line_start, funccall_T, prof_def_func, profile_add,
    profile_end, profile_self, profile_start, profile_sub_wait, profile_zero, proftime_T,
    script_prof_restore, script_prof_save,
};
use crate::regexp::{
    NUMBUFLEN, RE_MAGIC, regmatch_T, skip_regexp, vim_regcomp, vim_regexec, vim_regfree,
};
use crate::runtime::{
    autoload_name, estack_pop, estack_push_ufunc, exestack, get_sourced_lnum, script_autoload,
    set_sourcing_lnum, sourcing_lnum, sourcing_name,
};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::strings::{
    concat_str, gettext, memchr, strchr, strcmp, strcpy, strlen, strncmp, strstr, vim_snprintf,
    vim_strchr, xstrnsave,
};
use crate::ui::ui_has;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type VimLFunc = Option<unsafe fn(*mut typval_T, *mut typval_T, EvalFuncData)>;
pub type ArgvFunc = Option<unsafe fn(c_int, *mut typval_T, c_int, *mut ufunc_T) -> c_int>;

#[repr(C)]
pub struct ufunc_T {
    pub uf_varargs: c_int,
    pub uf_flags: c_int,
    pub uf_calls: c_int,
    pub uf_cleared: bool,
    pub uf_args: Vec<*mut c_char>,
    pub uf_def_args: Vec<*mut c_char>,
    pub uf_lines: Vec<*mut c_char>,
    pub uf_profiling: c_int,
    pub uf_prof_initialized: c_int,
    pub uf_luaref: c_int,
    pub uf_tm_count: c_int,
    pub uf_tm_total: u64,
    pub uf_tm_self: u64,
    pub uf_tm_children: u64,
    pub uf_tml_count: Vec<c_int>,
    pub uf_tml_total: Vec<u64>,
    pub uf_tml_self: Vec<u64>,
    pub uf_tml_start: u64,
    pub uf_tml_children: u64,
    pub uf_tml_wait: u64,
    pub uf_tml_idx: c_int,
    pub uf_tml_execed: c_int,
    pub uf_script_ctx: crate::runtime::sctx_T,
    pub uf_refcount: c_int,
    pub uf_scoped: *mut crate::profile::funccall_T,
    pub uf_name_exp: Option<std::ffi::CString>,
    pub uf_namelen: usize,
    pub uf_name: Vec<c_char>,
}
use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO};
use crate::buffer::{DO_NOT_FREE_CNT, NOTDONE, VAR_FIXED, VAR_SCOPE};
use crate::globals::IOSIZE;

pub const FC_ABORT: c_int = 0x01;
pub const FC_RANGE: c_int = 0x02;
pub const FC_DICT: c_int = 0x04;
pub const FC_CLOSURE: c_int = 0x08;
pub const FC_DELETED: c_int = 0x10;
pub const FC_REMOVED: c_int = 0x20;
pub const FC_SANDBOX: c_int = 0x40;
pub const FC_NOARGS: c_int = 0x200;
pub const FC_LUAREF: c_int = 0x800;

pub const FCERR_UNKNOWN: c_int = 0;
pub const FCERR_TOOMANY: c_int = 1;
pub const FCERR_TOOFEW: c_int = 2;
pub const FCERR_SCRIPT: c_int = 3;
pub const FCERR_DICT: c_int = 4;
pub const FCERR_NONE: c_int = 5;
pub const FCERR_OTHER: c_int = 6;
pub const FCERR_DELETED: c_int = 7;
pub const FCERR_NOTMETHOD: c_int = 8;

const FLEN_FIXED: usize = 40;
const MAX_FUNC_NESTING: c_int = 50;

pub(crate) const MSG_BUF_LEN: usize = 480;
const MSG_BUF_CLEN: c_int = (MSG_BUF_LEN / 6) as c_int;

#[repr(C)]
#[derive(Clone)]
pub struct funcdict_T {
    pub fd_dict: *mut dict_T,
    pub fd_newkey: Option<std::ffi::CString>,
    pub fd_di: *mut dictitem_T,
}

impl funcdict_T {
    pub const fn new() -> Self {
        Self {
            fd_dict: ptr::null_mut(),
            fd_newkey: None,
            fd_di: ptr::null_mut(),
        }
    }
}

impl Default for funcdict_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct funcexe_T {
    pub fe_argv_func: ArgvFunc,
    pub fe_firstline: linenr_T,
    pub fe_lastline: linenr_T,
    pub fe_doesrange: *mut bool,
    pub fe_evaluate: bool,
    pub fe_partial: *mut partial_T,
    pub fe_selfdict: *mut dict_T,
    pub fe_basetv: *mut typval_T,
    pub fe_found_var: bool,
}

impl funcexe_T {
    pub const fn new() -> Self {
        Self {
            fe_argv_func: None,
            fe_firstline: 0,
            fe_lastline: 0,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: false,
            fe_partial: ptr::null_mut(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        }
    }
}

impl Default for funcexe_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct EvalFuncDef {
    pub name: *mut c_char,
    pub min_argc: u8,
    pub max_argc: u8,
    pub base_arg: u8,
    pub fast: bool,
    pub func: VimLFunc,
    pub data: EvalFuncData,
}

unsafe impl Sync for EvalFuncDef {}

#[repr(C)]
pub(crate) struct defer_T {
    pub(crate) dr_name: Option<std::ffi::CString>,
    pub(crate) dr_argvars: Vec<typval_T>,
}

impl Drop for defer_T {
    fn drop(&mut self) {
        unsafe {
            for tv in self.dr_argvars.iter_mut().rev() {
                tv_clear(tv);
            }
        }
    }
}

static mut func_hashtab: std::collections::BTreeMap<Vec<u8>, *mut ufunc_T> =
    std::collections::BTreeMap::new();
static mut func_hashtab_changed: c_int = 0;
static mut funcargs: Vec<*mut typval_T> = Vec::new();
struct OwnedFunccall(*mut funccall_T);

impl Drop for OwnedFunccall {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.0));
        }
    }
}

static mut current_funccal: *mut funccall_T = ptr::null_mut();
static mut previous_funccal: Vec<OwnedFunccall> = Vec::new();
static mut funccal_stack: Vec<*mut funccall_T> = Vec::new();
static mut lambda_name: [c_char; 8 + NUMBUFLEN] = [0; 8 + NUMBUFLEN];
static mut lambda_no: c_int = 0;
static mut made_copy: c_int = 0;
static mut call_depth: c_int = 0;
static mut func_nr: c_int = 0;
static mut get_uf_done: usize = 0;
static mut get_uf_changed: c_int = 0;
static mut get_uf_items: Vec<*mut ufunc_T> = Vec::new();

const E_FUNCEXTS: &CStr = c"E122: Function %s already exists, add ! to replace it";
const E_FUNCDICT: &CStr = c"E717: Dictionary entry already exists";
const E_FUNCREF: &CStr = c"E718: Funcref required";
const E_NOFUNC: &CStr = c"E130: Unknown function: %s";
const E_FUNCTION_LIST_WAS_MODIFIED: &CStr = c"E454: Function list was modified";
const E_FUNCTION_NESTING_TOO_DEEP: &CStr = c"E1058: Function nesting too deep";
const E_NO_WHITE_SPACE_ALLOWED_BEFORE_STR_STR: &CStr =
    c"E1068: No white space allowed before '%s': %s";
const E_MISSING_HEREDOC_END_MARKER_STR: &CStr = c"E1145: Missing heredoc end marker: %s";
const E_CANNOT_USE_PARTIAL_WITH_DICTIONARY_FOR_DEFER: &CStr =
    c"E1300: Cannot use a partial with dictionary for :defer";

#[inline]
pub const fn ascii_iswhite_nl_or_nul(c: c_int) -> bool {
    matches!(c as u8, b' ' | b'\t' | b'\n' | b'\r' | 0)
}

use crate::charset::toupper_asc;

#[inline]
unsafe fn uf2hikey(fp: *const ufunc_T) -> *mut c_char {
    unsafe { (*fp).uf_name.as_ptr().cast_mut() }
}

unsafe fn func_hashtab_add(fp: *mut ufunc_T) -> c_int {
    unsafe {
        let key_ptr = uf2hikey(fp);
        let key = CStr::from_ptr(key_ptr).to_bytes();
        let ht = &raw mut func_hashtab;
        if (*ht).contains_key(key) {
            crate::siemsg!(c"Duplicate key in hash table: \"%s\"".as_ptr(), key_ptr);
            return FAIL;
        }
        (*ht).insert(key.to_vec(), fp);
        func_hashtab_changed = func_hashtab_changed.wrapping_add(1);
        OK
    }
}

#[inline]
unsafe fn funcarg(fp: *const ufunc_T, j: c_int) -> *mut c_char {
    unsafe { (&(*fp).uf_args)[j as usize] }
}

/// Return the function hash table.
pub unsafe fn func_tbl_get() -> &'static mut std::collections::BTreeMap<Vec<u8>, *mut ufunc_T> {
    unsafe {
        let tbl = &raw mut func_hashtab;
        &mut *tbl
    }
}

/// Get one function argument.
/// Return a pointer to after the type.
/// When something is wrong return "arg".
unsafe fn one_function_arg(
    arg: *mut c_char,
    newargs: Option<&mut Vec<*mut c_char>>,
    skip: bool,
) -> *mut c_char {
    unsafe {
        let mut p = arg;

        while ascii_isalnum(*p as c_int) || *p == b'_' as c_char {
            p = p.add(1);
        }
        let len = p.offset_from(arg);
        if arg == p
            || (*arg as u8).is_ascii_digit()
            || (len == 9 && strncmp(arg, c"firstline".as_ptr(), 9) == 0)
            || (len == 8 && strncmp(arg, c"lastline".as_ptr(), 8) == 0)
        {
            if !skip {
                semsg!(gettext(c"E125: Illegal argument: %s".as_ptr()), arg);
            }
            return arg;
        }

        if let Some(newargs) = newargs {
            let c = *p;
            *p = NUL;
            let arg_copy = xstrdup(arg);

            // Check for duplicate argument name.
            for &existing in newargs.iter() {
                if strcmp(existing, arg_copy) == 0 {
                    semsg!(
                        gettext(c"E853: Duplicate argument name: %s".as_ptr()),
                        arg_copy,
                    );
                    xfree(arg_copy.cast());
                    return arg;
                }
            }
            newargs.push(arg_copy);

            *p = c;
        }

        p
    }
}

/// Get function arguments.
unsafe fn get_function_args(
    argp: *mut *mut c_char,
    endchar: c_char,
    mut newargs: Option<&mut Vec<*mut c_char>>,
    varargs: *mut c_int,
    mut default_args: Option<&mut Vec<*mut c_char>>,
    skip: bool,
) -> c_int {
    unsafe {
        let mut mustend = false;
        let mut arg = *argp;
        let mut p = arg;

        if !varargs.is_null() {
            *varargs = 0;
        }

        let mut any_default = false;
        while *p != endchar {
            if *p == b'.' as c_char && *p.add(1) == b'.' as c_char && *p.add(2) == b'.' as c_char {
                if !varargs.is_null() {
                    *varargs = 1;
                }
                p = p.add(3);
                mustend = true;
            } else {
                arg = p;
                p = one_function_arg(p, newargs.as_deref_mut(), skip);
                if p == arg {
                    break;
                }

                if *skipwhite(p) == b'=' as c_char && default_args.is_some() {
                    let mut rettv: typval_T = typval_T::new();

                    any_default = true;
                    p = skipwhite(p).add(1);
                    p = skipwhite(p);
                    let mut expr = p;
                    if eval1(&raw mut p, &raw mut rettv, ptr::null_mut()) != FAIL {
                        while p > expr && ascii_iswhite(*p.sub(1) as c_int) {
                            p = p.sub(1);
                        }
                        let c = *p;
                        *p = NUL;
                        expr = xstrdup(expr);
                        if let Some(default_args) = default_args.as_deref_mut() {
                            default_args.push(expr);
                        }
                        *p = c;
                    } else {
                        mustend = true;
                    }
                } else if any_default {
                    emsg(gettext(
                        c"E989: Non-default argument follows default argument".as_ptr(),
                    ));
                    mustend = true;
                }

                if ascii_iswhite(*p as c_int) && *skipwhite(p) == b',' as c_char {
                    if !skip {
                        semsg!(
                            gettext(E_NO_WHITE_SPACE_ALLOWED_BEFORE_STR_STR.as_ptr()),
                            c",".as_ptr(),
                            p,
                        );
                        if let Some(newargs) = newargs.as_deref_mut() {
                            crate::path::clear_cstrings(newargs);
                        }
                        if let Some(default_args) = default_args.as_deref_mut() {
                            crate::path::clear_cstrings(default_args);
                        }
                        return FAIL;
                    }
                    p = skipwhite(p);
                }
                if *p == b',' as c_char {
                    p = p.add(1);
                } else {
                    mustend = true;
                }
            }
            p = skipwhite(p);
            if mustend && *p != endchar {
                if !skip {
                    semsg!(gettext(e_invarg2.as_ptr()), *argp);
                }
                break;
            }
        }
        if *p != endchar {
            if let Some(newargs) = newargs {
                crate::path::clear_cstrings(newargs);
            }
            if let Some(default_args) = default_args {
                crate::path::clear_cstrings(default_args);
            }
            return FAIL;
        }
        p = p.add(1); // skip "endchar"

        *argp = p;
        OK
    }
}

/// Register function "fp" as using "current_funccal" as its scope.
unsafe fn register_closure(fp: *mut ufunc_T) {
    unsafe {
        if (*fp).uf_scoped == current_funccal {
            return;
        }
        funccal_unref((*fp).uf_scoped, fp, false);
        (*fp).uf_scoped = current_funccal;
        (*current_funccal).fc_refcount += 1;
        (*current_funccal).fc_ufuncs.push(fp);
    }
}

/// Return a name for a lambda in static memory.
unsafe fn get_lambda_name() -> NvimString {
    unsafe {
        lambda_no += 1;
        let buf_ptr = (&raw mut lambda_name).cast::<c_char>();
        let buf_size = std::mem::size_of::<[c_char; 8 + NUMBUFLEN]>();
        let n = vim_snprintf!(buf_ptr, buf_size, c"<lambda>%d".as_ptr(), lambda_no);
        let size = if n < 1 {
            0
        } else {
            (n as usize).min(buf_size - 1)
        };
        NvimString {
            data: buf_ptr,
            size,
        }
    }
}

/// Allocate a `ufunc_T` for a function called `name`.
unsafe fn alloc_ufunc(name: *const c_char, namelen: usize) -> *mut ufunc_T {
    unsafe {
        let mut name_vec = Vec::with_capacity(namelen + 1);
        name_vec.extend_from_slice(std::slice::from_raw_parts(name, namelen));
        name_vec.push(NUL);
        let fp = Box::into_raw(Box::new(ufunc_T {
            uf_varargs: 0,
            uf_flags: 0,
            uf_calls: 0,
            uf_cleared: false,
            uf_args: Vec::new(),
            uf_def_args: Vec::new(),
            uf_lines: Vec::new(),
            uf_profiling: 0,
            uf_prof_initialized: 0,
            uf_luaref: 0,
            uf_tm_count: 0,
            uf_tm_total: 0,
            uf_tm_self: 0,
            uf_tm_children: 0,
            uf_tml_count: Vec::new(),
            uf_tml_total: Vec::new(),
            uf_tml_self: Vec::new(),
            uf_tml_start: 0,
            uf_tml_children: 0,
            uf_tml_wait: 0,
            uf_tml_idx: 0,
            uf_tml_execed: 0,
            uf_script_ctx: crate::runtime::sctx_T::new(),
            uf_refcount: 0,
            uf_scoped: ptr::null_mut(),
            uf_name_exp: None,
            uf_namelen: namelen,
            uf_name: name_vec,
        }));

        if *name as u8 == K_SPECIAL {
            let mut bytes = Vec::with_capacity(namelen + 2);
            bytes.extend_from_slice(b"<SNR>");
            bytes.extend_from_slice(CStr::from_ptr(uf2hikey(fp).add(3)).to_bytes());
            (*fp).uf_name_exp = Some(std::ffi::CString::new(bytes).unwrap());
        }

        fp
    }
}

/// Parse a lambda expression and get a Funcref from `*arg`.
pub unsafe fn get_lambda_tv(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut newargs: Vec<*mut c_char> = Vec::new();
        let mut pt: *mut partial_T = ptr::null_mut();
        let mut varargs: c_int = 0;
        let old_eval_lavars = eval_lavars_used;
        let mut eval_lavars = false;
        let mut tofree: *mut c_char = ptr::null_mut();

        let mut s = skipwhite((*arg).add(1));
        let mut ret = get_function_args(
            &raw mut s,
            b'-' as c_char,
            None,
            ptr::null_mut(),
            None,
            true,
        );
        if ret == FAIL || *s != b'>' as c_char {
            return NOTDONE;
        }

        *arg = skipwhite((*arg).add(1));
        ret = get_function_args(
            arg,
            b'-' as c_char,
            evaluate.then_some(&mut newargs),
            &raw mut varargs,
            None,
            false,
        );
        if ret == FAIL || **arg != b'>' as c_char {
            crate::path::clear_cstrings(&mut newargs);
            xfree(pt.cast());
            if !evalarg.is_null() && (*evalarg).eval_tofree.is_null() {
                (*evalarg).eval_tofree = tofree;
            } else {
                xfree(tofree.cast());
            }
            eval_lavars_used = old_eval_lavars;
            return FAIL;
        }

        if evaluate {
            eval_lavars_used = &raw mut eval_lavars;
        }

        *arg = skipwhite((*arg).add(1));
        let start = *arg;
        ret = skip_expr(arg, evalarg);
        let end = *arg;
        if ret == FAIL {
            crate::path::clear_cstrings(&mut newargs);
            xfree(pt.cast());
            if !evalarg.is_null() && (*evalarg).eval_tofree.is_null() {
                (*evalarg).eval_tofree = tofree;
            } else {
                xfree(tofree.cast());
            }
            eval_lavars_used = old_eval_lavars;
            return FAIL;
        }
        if !evalarg.is_null() {
            tofree = (*evalarg).eval_tofree;
            (*evalarg).eval_tofree = ptr::null_mut();
        }

        *arg = skipwhite(*arg);
        if **arg != b'}' as c_char {
            semsg!(gettext(c"E451: Expected }: %s".as_ptr()), *arg);
            crate::path::clear_cstrings(&mut newargs);
            xfree(pt.cast());
            if !evalarg.is_null() && (*evalarg).eval_tofree.is_null() {
                (*evalarg).eval_tofree = tofree;
            } else {
                xfree(tofree.cast());
            }
            eval_lavars_used = old_eval_lavars;
            return FAIL;
        }
        *arg = (*arg).add(1);

        if evaluate {
            let mut flags = 0;

            let name = get_lambda_name();
            let fp = alloc_ufunc(name.data, name.size);
            pt = Box::into_raw(Box::new(partial_T::default()));

            let expr_len = end.offset_from(start) as usize;
            let len = 7 + expr_len + 1;
            let p = xmalloc(len).cast::<c_char>();
            let newlines = vec![p];
            strcpy(p, c"return ".as_ptr());
            xmemcpyz(p.add(7).cast(), start.cast(), expr_len);
            if strstr(p.add(7), c"a:".as_ptr()).is_null() {
                flags |= FC_NOARGS;
            }

            let newlines_len = newlines.len() as c_int;
            (*fp).uf_refcount = 1;
            func_hashtab_add(fp);
            (*fp).uf_args = newargs;
            (*fp).uf_def_args = Vec::new();
            (*fp).uf_lines = newlines;
            if !current_funccal.is_null() && eval_lavars {
                flags |= FC_CLOSURE;
                register_closure(fp);
            } else {
                (*fp).uf_scoped = ptr::null_mut();
            }

            if prof_def_func() {
                func_do_profile(fp);
            }
            if sandbox != 0 {
                flags |= FC_SANDBOX;
            }
            (*fp).uf_varargs = 1;
            (*fp).uf_flags = flags;
            (*fp).uf_calls = 0;
            (*fp).uf_script_ctx = current_sctx;
            (*fp).uf_script_ctx.sc_lnum += sourcing_lnum() - newlines_len;

            (*pt).pt_func = fp;
            (*pt).pt_refcount = 1;
            (*rettv).vval.v_partial = pt;
            (*rettv).v_type = VarType::VAR_PARTIAL;
        }

        eval_lavars_used = old_eval_lavars;
        if !evalarg.is_null() && (*evalarg).eval_tofree.is_null() {
            (*evalarg).eval_tofree = tofree;
        } else {
            xfree(tofree.cast());
        }
        OK
    }
}

/// Return name of the function corresponding to `name`.
pub unsafe fn deref_func_name(
    name: *const c_char,
    lenp: *mut c_int,
    partialp: *mut *mut partial_T,
    no_autoload: bool,
    found_var: *mut bool,
) -> *mut c_char {
    unsafe {
        if !partialp.is_null() {
            *partialp = ptr::null_mut();
        }

        let v = find_var(name, *lenp as usize, ptr::null_mut(), no_autoload as c_int);
        if v.is_null() {
            return name.cast_mut();
        }
        let tv = &raw mut (*v).di_tv;
        if !found_var.is_null() {
            *found_var = true;
        }

        if (*tv).v_type == VarType::VAR_FUNC {
            if (*tv).vval.v_string.is_null() {
                *lenp = 0;
                return c"".as_ptr().cast_mut();
            }
            *lenp = strlen((*tv).vval.v_string) as c_int;
            return (*tv).vval.v_string;
        }

        if (*tv).v_type == VarType::VAR_PARTIAL {
            let pt = (*tv).vval.v_partial;
            if pt.is_null() {
                *lenp = 0;
                return c"".as_ptr().cast_mut();
            }
            if !partialp.is_null() {
                *partialp = pt;
            }
            let s = partial_name(pt);
            *lenp = strlen(s) as c_int;
            return s;
        }

        name.cast_mut()
    }
}

/// Give an error message with a function name. Handle `<SNR>` things.
pub unsafe fn emsg_funcname(errmsg: *const c_char, name: *const c_char) {
    unsafe {
        let mut p = name.cast_mut();

        if *name as u8 == K_SPECIAL && *name.add(1) != NUL && *name.add(2) != NUL {
            p = concat_str(c"<SNR>".as_ptr(), name.add(3));
        }

        semsg!(gettext(errmsg), p);

        if p.cast_const() != name {
            xfree(p.cast());
        }
    }
}

/// Get function arguments at `*arg` and advance it.
unsafe fn get_func_arguments(
    arg: *mut *mut c_char,
    evalarg: *mut evalarg_T,
    partial_argc: c_int,
    argvars: *mut typval_T,
    argcount: *mut c_int,
) -> c_int {
    unsafe {
        let mut argp = *arg;
        let mut ret = OK;

        while *argcount < MAX_FUNC_ARGS as c_int - partial_argc {
            argp = skipwhite(argp.add(1));

            if *argp == b')' as c_char || *argp == b',' as c_char || *argp == NUL {
                break;
            }
            if eval1(&raw mut argp, argvars.add(*argcount as usize), evalarg) == FAIL {
                ret = FAIL;
                break;
            }
            *argcount += 1;
            if *argp != b',' as c_char {
                break;
            }
        }

        argp = skipwhite(argp);
        if *argp == b')' as c_char {
            argp = argp.add(1);
        } else {
            ret = FAIL;
        }
        *arg = argp;
        ret
    }
}

/// Call a function and put the result in `rettv`.
pub unsafe fn get_func_tv(
    name: *const c_char,
    len: c_int,
    rettv: *mut typval_T,
    arg: *mut *mut c_char,
    evalarg: *mut evalarg_T,
    funcexe: *mut funcexe_T,
) -> c_int {
    unsafe {
        let mut argvars: [typval_T; MAX_FUNC_ARGS + 1] = [typval_T::new(); MAX_FUNC_ARGS + 1];
        let mut argcount: c_int = 0;
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;

        let mut argp = *arg;
        let partial_argc = if (*funcexe).fe_partial.is_null() {
            0
        } else {
            (*(*funcexe).fe_partial).pt_argv.len() as c_int
        };
        let mut ret = get_func_arguments(
            &raw mut argp,
            evalarg,
            partial_argc,
            argvars.as_mut_ptr(),
            &raw mut argcount,
        );

        if ret == OK {
            let mut i = 0usize;

            if get_vim_var_nr(VV_TESTING) != 0 {
                let fa = &raw mut funcargs;
                while (i as c_int) < argcount {
                    (*fa).push(&raw mut argvars[i]);
                    i += 1;
                }
            }
            ret = call_func(name, len, rettv, argcount, argvars.as_mut_ptr(), funcexe);

            if i > 0 {
                let fa = &raw mut funcargs;
                let new_len = (*fa).len().saturating_sub(i);
                (*fa).truncate(new_len);
            }
        } else if !aborting() && evaluate {
            if argcount == MAX_FUNC_ARGS as c_int {
                emsg_funcname(c"E740: Too many arguments for function %s".as_ptr(), name);
            } else {
                emsg_funcname(c"E116: Invalid arguments for function %s".as_ptr(), name);
            }
        }

        while argcount > 0 {
            argcount -= 1;
            tv_clear(&raw mut argvars[argcount as usize]);
        }

        *arg = skipwhite(argp);
        ret
    }
}

/// Check whether function name starts with `<SID>` or `s:`.
#[inline]
unsafe fn eval_fname_sid(name: *const c_char) -> bool {
    unsafe { *name == b's' as c_char || toupper_asc(*name.add(2) as c_int) == b'I' as c_int }
}

/// In a script transform script-local names into actually used names.
unsafe fn fname_trans_sid(
    name: *const c_char,
    fname_buf: *mut c_char,
    tofree: *mut *mut c_char,
    error: *mut c_int,
) -> *mut c_char {
    unsafe {
        let script_name = name.add(eval_fname_script(name) as usize);
        if script_name == name {
            return name.cast_mut();
        }

        *fname_buf.add(0) = K_SPECIAL as c_char;
        *fname_buf.add(1) = KS_EXTRA as c_char;
        *fname_buf.add(2) = KE_SNR as c_char;
        let mut fname_buflen = 3usize;
        *fname_buf.add(fname_buflen) = NUL;
        if eval_fname_sid(name) {
            if current_sctx.sc_sid <= 0 {
                *error = FCERR_SCRIPT;
            } else {
                fname_buflen += vim_snprintf!(
                    fname_buf.add(fname_buflen),
                    FLEN_FIXED + 1 - fname_buflen,
                    c"%d_".as_ptr(),
                    current_sctx.sc_sid,
                ) as usize;
            }
        }
        let fnamelen = fname_buflen + strlen(script_name);
        if fnamelen < FLEN_FIXED {
            strcpy(fname_buf.add(fname_buflen), script_name);
            fname_buf
        } else {
            let fname = xmalloc(fnamelen + 1).cast::<c_char>();
            *tofree = fname;
            vim_snprintf!(
                fname,
                fnamelen + 1,
                c"%s%s".as_ptr(),
                fname_buf,
                script_name,
            );
            fname
        }
    }
}

pub unsafe fn get_func_arity(
    name: *const c_char,
    required: *mut c_int,
    optional: *mut c_int,
    varargs: *mut bool,
) -> c_int {
    unsafe {
        let argcount: c_int;
        let min_argcount: c_int;

        let fdef = find_internal_func(name);
        if !fdef.is_null() {
            argcount = (*fdef).max_argc as c_int;
            min_argcount = (*fdef).min_argc as c_int;
            *varargs = false;
        } else {
            let mut fname_buf = [0 as c_char; FLEN_FIXED + 1];
            let mut tofree: *mut c_char = ptr::null_mut();
            let mut error = FCERR_NONE;

            let fname = fname_trans_sid(
                name,
                fname_buf.as_mut_ptr(),
                &raw mut tofree,
                &raw mut error,
            );
            let mut ufunc: *mut ufunc_T = ptr::null_mut();
            if error == FCERR_NONE {
                ufunc = find_func(fname);
            }
            xfree(tofree.cast());

            if ufunc.is_null() {
                return FAIL;
            }

            argcount = (*ufunc).uf_args.len() as c_int;
            min_argcount = argcount - (*ufunc).uf_def_args.len() as c_int;
            *varargs = (*ufunc).uf_varargs != 0;
        }

        *required = min_argcount;
        *optional = argcount - min_argcount;

        OK
    }
}

/// Find a function by name, return pointer to it in ufuncs.
pub unsafe fn find_func(name: *const c_char) -> *mut ufunc_T {
    if name.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let ht = &raw const func_hashtab;
        (*ht)
            .get(CStr::from_ptr(name).to_bytes())
            .copied()
            .unwrap_or(ptr::null_mut())
    }
}

#[inline]
unsafe fn func_is_global(ufunc: *const ufunc_T) -> bool {
    unsafe { (*ufunc).uf_name.as_slice()[0] as u8 != K_SPECIAL }
}

unsafe fn cat_func_name(buf: *mut c_char, bufsize: usize, fp: *const ufunc_T) -> c_int {
    unsafe {
        let uflen = (*fp).uf_namelen;
        debug_assert!(uflen > 0);

        let len = if !func_is_global(fp) && uflen > 3 {
            vim_snprintf!(buf, bufsize, c"<SNR>%s".as_ptr(), uf2hikey(fp).add(3))
        } else {
            vim_snprintf!(buf, bufsize, c"%s".as_ptr(), uf2hikey(fp))
        };

        debug_assert!(len > 0);
        if len >= bufsize as c_int {
            bufsize as c_int - 1
        } else {
            len
        }
    }
}

/// Add a number variable `name` to dict `dp` with value `nr`.
unsafe fn add_nr_var(dp: *mut dict_T, name: *const c_char, nr: varnumber_T) {
    unsafe {
        let v = tv_dict_item_alloc(name);
        (*v).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
        (*v).di_tv.v_type = VarType::VAR_NUMBER;
        (*v).di_tv.v_lock = VAR_FIXED;
        (*v).di_tv.vval.v_number = nr;
        hash_add(&raw mut (*dp).dv_hashtab, v);
    }
}

impl Drop for funccall_T {
    fn drop(&mut self) {
        unsafe {
            vars_clear(&raw mut self.fc_l_vars.dv_hashtab);
            vars_clear(&raw mut self.fc_l_avars.dv_hashtab);
            let self_ptr = std::ptr::from_mut(self).cast();
            for &fp in &self.fc_ufuncs {
                if !fp.is_null() && (*fp).uf_scoped == self_ptr {
                    (*fp).uf_scoped = ptr::null_mut();
                }
            }
            func_ptr_unref(self.fc_func);
        }
    }
}

/// Handle the last part of returning from a function: free the local hashtable.
/// Unless it is still in use by a closure.
unsafe fn cleanup_function_call(fc: *mut funccall_T) {
    unsafe {
        let may_free_fc = (*fc).fc_refcount <= 0;
        let mut free_fc = true;

        current_funccal = (*fc).fc_caller;

        if may_free_fc && (*fc).fc_l_vars.dv_refcount == DO_NOT_FREE_CNT {
            vars_clear(&raw mut (*fc).fc_l_vars.dv_hashtab);
        } else {
            free_fc = false;
        }

        if may_free_fc && (*fc).fc_l_avars.dv_refcount == DO_NOT_FREE_CNT {
            vars_clear_ext(&raw mut (*fc).fc_l_avars.dv_hashtab, false);
        } else {
            free_fc = false;

            {
                let mut todo = (*fc).fc_l_avars.dv_hashtab.ht_used;
                let mut hi = (*fc).fc_l_avars.dv_hashtab.ht_array;
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        todo -= 1;
                        let di = tv_dict_hi2di(hi);
                        tv_copy(&raw const (*di).di_tv, &raw mut (*di).di_tv);
                    }
                    hi = hi.add(1);
                }
            }
        }

        if may_free_fc && (*fc).fc_l_varlist.lv_refcount == DO_NOT_FREE_CNT {
            let mut li = (*fc).fc_l_varlist.lv_first;
            while !li.is_null() {
                (*li).li_tv.v_type = VarType::VAR_UNKNOWN;
                li = (*li).li_next;
            }
            (*fc).fc_l_varlist.lv_first = ptr::null_mut();
        } else {
            free_fc = false;

            {
                let mut li = (*fc).fc_l_varlist.lv_first;
                while !li.is_null() {
                    tv_copy(&raw const (*li).li_tv, &raw mut (*li).li_tv);
                    li = (*li).li_next;
                }
            }
        }

        if free_fc {
            drop(Box::from_raw(fc));
        } else {
            (*fc).fc_caller = ptr::null_mut();
            let pfc = &raw mut previous_funccal;
            (*pfc).push(OwnedFunccall(fc));

            if want_garbage_collect {
                made_copy = 0;
            } else {
                made_copy += 1;
                if made_copy >= ((4096 * 1024) / std::mem::size_of::<funccall_T>()) as c_int {
                    made_copy = 0;
                    want_garbage_collect = true;
                }
            }
        }
    }
}

/// Unreference `fc`: decrement the reference count and free it when it
/// becomes zero. `fp` is detached from `fc`.
unsafe fn funccal_unref(fc: *mut funccall_T, fp: *mut ufunc_T, force: bool) {
    unsafe {
        if fc.is_null() {
            return;
        }

        (*fc).fc_refcount -= 1;
        if if force {
            (*fc).fc_refcount <= 0
        } else {
            !fc_referenced(fc)
        } {
            let pfc = &raw mut previous_funccal;
            if let Some(pos) = (*pfc).iter().position(|item| item.0 == fc) {
                (*pfc).remove(pos);
                return;
            }
        }
        for slot in &mut (*fc).fc_ufuncs {
            if *slot == fp {
                *slot = ptr::null_mut();
            }
        }
    }
}

/// Remove the function from the function hashtable.
unsafe fn func_remove(fp: *mut ufunc_T) -> bool {
    unsafe {
        let ht = &raw mut func_hashtab;
        let key = CStr::from_ptr(uf2hikey(fp)).to_bytes();
        if (*ht).remove(key).is_some() {
            func_hashtab_changed = func_hashtab_changed.wrapping_add(1);
            true
        } else {
            false
        }
    }
}

unsafe fn func_clear_items(fp: *mut ufunc_T) {
    unsafe {
        crate::path::clear_cstrings(&mut (*fp).uf_args);
        crate::path::clear_cstrings(&mut (*fp).uf_def_args);
        crate::path::clear_cstrings(&mut (*fp).uf_lines);

        if ((*fp).uf_flags & FC_LUAREF) != 0 {
            api_free_luaref((*fp).uf_luaref);
            (*fp).uf_luaref = LUA_NOREF;
        }

        (*fp).uf_tml_count.clear();
        (*fp).uf_tml_total.clear();
        (*fp).uf_tml_self.clear();
    }
}

unsafe fn func_clear(fp: *mut ufunc_T, force: bool) {
    unsafe {
        if (*fp).uf_cleared {
            return;
        }
        (*fp).uf_cleared = true;

        func_clear_items(fp);
        funccal_unref((*fp).uf_scoped, fp, force);
    }
}

impl Drop for ufunc_T {
    fn drop(&mut self) {
        unsafe {
            if !self.uf_cleared {
                self.uf_cleared = true;
                func_clear_items(self);
            }
            if (self.uf_flags & (FC_DELETED | FC_REMOVED)) == 0 {
                func_remove(self);
            }
        }
    }
}

unsafe fn func_clear_free(fp: *mut ufunc_T, force: bool) {
    unsafe {
        func_clear(fp, force);
        drop(Box::from_raw(fp));
    }
}

/// Allocate a `funccall_T`, link it in `current_funccal` and fill in `fp` and `rettv`.
pub unsafe fn create_funccal(fp: *mut ufunc_T, rettv: *mut typval_T) -> *mut funccall_T {
    unsafe {
        let fc = Box::into_raw(Box::new(funccall_T::new()));
        (*fc).fc_caller = current_funccal;
        current_funccal = fc;
        (*fc).fc_func = fp;
        func_ptr_ref(fp);
        (*fc).fc_rettv = rettv;
        fc
    }
}

/// Call a user function.
pub unsafe fn call_user_func(
    fp: *mut ufunc_T,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    firstline: linenr_T,
    lastline: linenr_T,
    selfdict: *mut dict_T,
) {
    unsafe {
        let mut using_sandbox = false;
        let mut islambda = false;
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let mut name: *mut c_char;
        let mut namelen: usize;
        let mut tv_to_free: [*mut typval_T; MAX_FUNC_ARGS] = [ptr::null_mut(); MAX_FUNC_ARGS];
        let mut tv_to_free_len = 0usize;
        let mut wait_start: proftime_T = 0;
        let mut call_start: proftime_T = 0;
        let mut started_profiling = false;
        let mut did_save_redo = false;
        let mut save_redo = RedoState::new();

        if i64::from(call_depth) >= p_mfd {
            emsg(gettext(
                c"E132: Function call depth is higher than 'maxfuncdepth'".as_ptr(),
            ));
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = -1;
            return;
        }
        call_depth += 1;
        save_search_patterns();
        if !ins_compl_active() {
            save_redobuff(&raw mut save_redo);
            did_save_redo = true;
        }
        (*fp).uf_calls += 1;
        line_breakcheck();

        let fc = create_funccal(fp, rettv);
        (*fc).fc_level = ex_nesting_level;
        (*fc).fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(fp), 0);
        (*fc).fc_dbg_tick = debug_tick;

        if strncmp(uf2hikey(fp), c"<lambda>".as_ptr(), 8) == 0 {
            islambda = true;
        }

        init_var_dict(
            &raw mut (*fc).fc_l_vars,
            (&raw mut (*fc).fc_l_vars_var).cast(),
            VAR_DEF_SCOPE,
        );
        if !selfdict.is_null() {
            let v = tv_dict_item_alloc_len(c"self".as_ptr(), 4);
            (*v).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            (*v).di_tv.v_type = VarType::VAR_DICT;
            (*v).di_tv.v_lock = VAR_UNLOCKED;
            (*v).di_tv.vval.v_dict = selfdict;
            (*selfdict).dv_refcount += 1;
            hash_add(&raw mut (*fc).fc_l_vars.dv_hashtab, v);
        }

        init_var_dict(
            &raw mut (*fc).fc_l_avars,
            (&raw mut (*fc).fc_l_avars_var).cast(),
            VAR_SCOPE,
        );
        let uf_args_len = (*fp).uf_args.len() as c_int;
        let uf_def_args_len = (*fp).uf_def_args.len() as c_int;
        if ((*fp).uf_flags & FC_NOARGS) == 0 {
            add_nr_var(
                &raw mut (*fc).fc_l_avars,
                c"0".as_ptr(),
                if argcount >= uf_args_len {
                    varnumber_T::from(argcount - uf_args_len)
                } else {
                    0
                },
            );
        }
        (*fc).fc_l_avars.dv_lock = VAR_FIXED;
        if ((*fp).uf_flags & FC_NOARGS) == 0 {
            let v = tv_dict_item_alloc_len(c"000".as_ptr(), 3);
            (*v).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            (*v).di_tv.v_type = VarType::VAR_LIST;
            (*v).di_tv.v_lock = VAR_FIXED;
            (*v).di_tv.vval.v_list = &raw mut (*fc).fc_l_varlist;
            hash_add(&raw mut (*fc).fc_l_avars.dv_hashtab, v);
        }
        tv_list_init_static(&raw mut (*fc).fc_l_varlist);
        tv_list_set_lock(&raw mut (*fc).fc_l_varlist, VAR_FIXED);

        if ((*fp).uf_flags & FC_NOARGS) == 0 {
            add_nr_var(
                &raw mut (*fc).fc_l_avars,
                c"firstline".as_ptr(),
                varnumber_T::from(firstline),
            );
            add_nr_var(
                &raw mut (*fc).fc_l_avars,
                c"lastline".as_ptr(),
                varnumber_T::from(lastline),
            );
        }

        let mut default_arg_err = false;
        let max_i = argcount.max(uf_args_len);
        let listitems_ptr = (&raw mut (*fc).fc_l_listitems).cast::<listitem_T>();
        for i in 0..max_i {
            let mut addlocal = false;
            let mut isdefault = false;
            let mut def_rettv: typval_T = typval_T::new();

            let ai = i - uf_args_len;
            if ai < 0 {
                name = funcarg(fp, i);
                if islambda {
                    addlocal = true;
                }

                isdefault = ai + uf_def_args_len >= 0 && i >= argcount;
                if isdefault {
                    def_rettv.v_type = VarType::VAR_NUMBER;
                    def_rettv.vval.v_number = -1;

                    let mut default_expr = (&(*fp).uf_def_args)[(ai + uf_def_args_len) as usize];
                    if eval1(
                        &raw mut default_expr,
                        &raw mut def_rettv,
                        &raw mut EVALARG_EVALUATE,
                    ) == FAIL
                    {
                        default_arg_err = true;
                        break;
                    }
                }

                namelen = strlen(name);
            } else {
                if ((*fp).uf_flags & FC_NOARGS) != 0 {
                    break;
                }
                namelen =
                    vim_snprintf!(numbuf.as_mut_ptr(), NUMBUFLEN, c"%d".as_ptr(), ai + 1) as usize;
                name = numbuf.as_mut_ptr();
            }

            let v = tv_dict_item_alloc_len(name, namelen);
            (*v).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;

            (*v).di_tv = if isdefault {
                def_rettv
            } else {
                *argvars.add(i as usize)
            };
            (*v).di_tv.v_lock = VAR_FIXED;

            if isdefault {
                tv_to_free[tv_to_free_len] = &raw mut (*v).di_tv;
                tv_to_free_len += 1;
            }

            if addlocal {
                tv_copy(&raw const (*v).di_tv, &raw mut (*v).di_tv);
                hash_add(&raw mut (*fc).fc_l_vars.dv_hashtab, v);
            } else {
                hash_add(&raw mut (*fc).fc_l_avars.dv_hashtab, v);
            }

            if ai >= 0 && (ai as usize) < MAX_FUNC_ARGS {
                let li = listitems_ptr.add(ai as usize);
                (*li).li_tv = *argvars.add(i as usize);
                (*li).li_tv.v_lock = VAR_FIXED;
                tv_list_append(&raw mut (*fc).fc_l_varlist, li);
            }
        }

        RedrawingDisabled += 1;

        if ((*fp).uf_flags & FC_SANDBOX) != 0 {
            using_sandbox = true;
            sandbox += 1;
        }

        estack_push_ufunc(fp, 1);
        let es = &raw const exestack;
        let estack_len_before = (*es).len() as c_int;
        if p_verbose >= 12 {
            no_wait_return += 1;
            verbose_enter_scroll();

            smsg!(0, gettext(c"calling %s".as_ptr()), sourcing_name());
            if p_verbose >= 14 {
                msg_puts(c"(".as_ptr());
                for i in 0..argcount {
                    if i > 0 {
                        msg_puts(c", ".as_ptr());
                    }
                    let arg_tv = argvars.add(i as usize);
                    if (*arg_tv).v_type == VarType::VAR_NUMBER {
                        msg_outnum((*arg_tv).vval.v_number as c_int);
                    } else {
                        emsg_off += 1;
                        let tofree = encode_tv2string(arg_tv, ptr::null_mut());
                        emsg_off -= 1;
                        if !tofree.is_null() {
                            let mut s = tofree;
                            let mut buf = [0 as c_char; MSG_BUF_LEN];
                            if vim_strsize(s) > MSG_BUF_CLEN {
                                trunc_string(
                                    s,
                                    buf.as_mut_ptr(),
                                    MSG_BUF_CLEN,
                                    MSG_BUF_LEN as c_int,
                                );
                                s = buf.as_mut_ptr();
                            }
                            msg_puts(s);
                            xfree(tofree.cast());
                        }
                    }
                }
                msg_puts(c")".as_ptr());
            }
            msg_puts(c"\n".as_ptr());

            verbose_leave_scroll();
            no_wait_return -= 1;
        }

        let do_profiling_yes = do_profiling == PROF_YES;
        let func_not_yet_profiling_but_should = do_profiling_yes
            && (*fp).uf_profiling == 0
            && has_profiling(false, uf2hikey(fp), ptr::null_mut());

        if func_not_yet_profiling_but_should {
            started_profiling = true;
            func_do_profile(fp);
        }

        let func_or_func_caller_profiling = do_profiling_yes
            && ((*fp).uf_profiling != 0
                || (!(*fc).fc_caller.is_null() && (*(*(*fc).fc_caller).fc_func).uf_profiling != 0));

        if func_or_func_caller_profiling {
            (*fp).uf_tm_count += 1;
            call_start = profile_start();
            (*fp).uf_tm_children = profile_zero();
        }

        if do_profiling_yes {
            script_prof_save(&raw mut wait_start);
        }

        let save_current_sctx = current_sctx;
        current_sctx = (*fp).uf_script_ctx;
        let save_did_emsg = did_emsg;
        did_emsg = 0;

        if default_arg_err && (((*fp).uf_flags & FC_ABORT) != 0 || trylevel > 0) {
            did_emsg = 1;
        } else if islambda {
            let mut p = (&(*fp).uf_lines)[0].add(7);
            ex_nesting_level += 1;
            eval1(&raw mut p, rettv, &raw mut EVALARG_EVALUATE);
            ex_nesting_level -= 1;
        } else {
            do_cmdline(
                ptr::null_mut(),
                crate::cmdexpand::LineGetter::Func(fc),
                DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
            );
        }

        handle_defer_one(current_funccal);

        RedrawingDisabled -= 1;

        if (did_emsg != 0 && ((*fp).uf_flags & FC_ABORT) != 0)
            || (*rettv).v_type == VarType::VAR_UNKNOWN
        {
            tv_clear(rettv);
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = -1;
        }

        if func_or_func_caller_profiling {
            call_start = profile_end(call_start);
            call_start = profile_sub_wait(wait_start, call_start);
            (*fp).uf_tm_total = profile_add((*fp).uf_tm_total, call_start);
            (*fp).uf_tm_self = profile_self((*fp).uf_tm_self, call_start, (*fp).uf_tm_children);
            if !(*fc).fc_caller.is_null() && (*(*(*fc).fc_caller).fc_func).uf_profiling != 0 {
                let caller_fp = (*(*fc).fc_caller).fc_func;
                (*caller_fp).uf_tm_children = profile_add((*caller_fp).uf_tm_children, call_start);
                (*caller_fp).uf_tml_children =
                    profile_add((*caller_fp).uf_tml_children, call_start);
            }
            if started_profiling {
                (*fp).uf_profiling = 0;
            }
        }

        if p_verbose >= 12 {
            no_wait_return += 1;
            verbose_enter_scroll();

            if aborting() {
                smsg!(0, gettext(c"%s aborted".as_ptr()), sourcing_name());
            } else if (*(*fc).fc_rettv).v_type == VarType::VAR_NUMBER {
                smsg!(
                    0,
                    gettext(c"%s returning #%lld".as_ptr()),
                    sourcing_name(),
                    (*(*fc).fc_rettv).vval.v_number,
                );
            } else {
                let mut buf = [0 as c_char; MSG_BUF_LEN];
                emsg_off += 1;
                let mut s = encode_tv2string((*fc).fc_rettv, ptr::null_mut());
                let tofree = s;
                emsg_off -= 1;
                if !s.is_null() {
                    if vim_strsize(s) > MSG_BUF_CLEN {
                        trunc_string(s, buf.as_mut_ptr(), MSG_BUF_CLEN, MSG_BUF_LEN as c_int);
                        s = buf.as_mut_ptr();
                    }
                    smsg!(0, gettext(c"%s returning %s".as_ptr()), sourcing_name(), s);
                    xfree(tofree.cast());
                }
            }
            msg_puts(c"\n".as_ptr());

            verbose_leave_scroll();
            no_wait_return -= 1;
        }

        let _ = estack_len_before;
        estack_pop();
        current_sctx = save_current_sctx;
        if do_profiling_yes {
            script_prof_restore(&raw const wait_start);
        }
        if using_sandbox {
            sandbox -= 1;
        }

        if p_verbose >= 12 && !sourcing_name().is_null() {
            no_wait_return += 1;
            verbose_enter_scroll();

            smsg!(0, gettext(c"continuing in %s".as_ptr()), sourcing_name());
            msg_puts(c"\n".as_ptr());

            verbose_leave_scroll();
            no_wait_return -= 1;
        }

        did_emsg |= save_did_emsg;
        call_depth -= 1;
        for &tv in tv_to_free.iter().take(tv_to_free_len) {
            tv_clear(tv);
        }
        cleanup_function_call(fc);

        (*fp).uf_calls -= 1;
        if (*fp).uf_calls <= 0 && (*fp).uf_refcount <= 0 {
            func_clear_free(fp, false);
        }
        if did_save_redo {
            restore_redobuff(&raw mut save_redo);
        }
        restore_search_patterns();
    }
}

#[inline]
unsafe fn func_name_refcount(name: *const c_char) -> bool {
    unsafe {
        (*name as u8).is_ascii_digit()
            || (*name == b'<' as c_char && *name.add(1) == b'l' as c_char)
    }
}

unsafe fn check_user_func_argcount(fp: *mut ufunc_T, argcount: c_int) -> c_int {
    unsafe {
        let regular_args = (*fp).uf_args.len() as c_int;
        if argcount < regular_args - (*fp).uf_def_args.len() as c_int {
            FCERR_TOOFEW
        } else if (*fp).uf_varargs == 0 && argcount > regular_args {
            FCERR_TOOMANY
        } else {
            FCERR_UNKNOWN
        }
    }
}

unsafe fn call_user_func_check(
    fp: *mut ufunc_T,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    funcexe: *mut funcexe_T,
    selfdict: *mut dict_T,
) -> c_int {
    unsafe {
        if ((*fp).uf_flags & FC_LUAREF) != 0 {
            return nlua_exec_typval_callable((*fp).uf_luaref, argcount, argvars, rettv);
        }

        if ((*fp).uf_flags & FC_RANGE) != 0 && !(*funcexe).fe_doesrange.is_null() {
            *(*funcexe).fe_doesrange = true;
        }
        let mut error = check_user_func_argcount(fp, argcount);
        if error != FCERR_UNKNOWN {
            return error;
        }
        if ((*fp).uf_flags & FC_DICT) != 0 && selfdict.is_null() {
            error = FCERR_DICT;
        } else {
            call_user_func(
                fp,
                argcount,
                argvars,
                rettv,
                (*funcexe).fe_firstline,
                (*funcexe).fe_lastline,
                if ((*fp).uf_flags & FC_DICT) != 0 {
                    selfdict
                } else {
                    ptr::null_mut()
                },
            );
            error = FCERR_NONE;
        }
        error
    }
}

/// Save the current function call pointer, and set it to NULL.
pub unsafe fn save_funccal() {
    unsafe {
        let stack = &raw mut funccal_stack;
        (*stack).push(current_funccal);
        current_funccal = ptr::null_mut();
    }
}

pub unsafe fn restore_funccal() {
    unsafe {
        let stack = &raw mut funccal_stack;
        if let Some(top) = (*stack).pop() {
            current_funccal = top;
        } else {
            iemsg(c"INTERNAL: restore_funccal()".as_ptr());
        }
    }
}

pub unsafe fn get_current_funccal() -> *mut funccall_T {
    unsafe { current_funccal }
}

pub unsafe fn set_current_funccal(fc: *mut funccall_T) {
    unsafe {
        current_funccal = fc;
    }
}

pub unsafe fn free_all_functions() {
    unsafe {
        while !current_funccal.is_null() {
            tv_clear((*current_funccal).fc_rettv);
            cleanup_function_call(current_funccal);
            let stack = &raw const funccal_stack;
            if current_funccal.is_null() && !(*stack).is_empty() {
                restore_funccal();
            }
        }
        funccal_stack = Vec::new();

        loop {
            let mut restarted = false;
            let ht = &raw const func_hashtab;
            let funcs: Vec<*mut ufunc_T> = (*ht).values().copied().collect();
            for fp in funcs {
                if !func_name_refcount(uf2hikey(fp)) {
                    let changed = func_hashtab_changed;
                    func_clear(fp, true);
                    if changed != func_hashtab_changed {
                        restarted = true;
                        break;
                    }
                }
            }
            if !restarted {
                break;
            }
        }

        let mut skipped = 0usize;
        loop {
            let ht = &raw const func_hashtab;
            if (*ht).len() <= skipped {
                break;
            }
            let funcs: Vec<*mut ufunc_T> = (*ht).values().copied().collect();
            for fp in funcs {
                if func_name_refcount(uf2hikey(fp)) {
                    skipped += 1;
                } else {
                    drop(Box::from_raw(fp));
                    skipped = 0;
                    break;
                }
            }
        }
        if skipped == 0 {
            let ht = &raw mut func_hashtab;
            *ht = std::collections::BTreeMap::new();
        }
        let items_ptr = &raw mut get_uf_items;
        *items_ptr = Vec::new();
    }
}

/// Checks if a builtin function with the given name exists.
unsafe fn builtin_function(name: *const c_char, len: c_int) -> bool {
    unsafe {
        if !ascii_islower(*name as c_int) || *name.add(1) == b':' as c_char {
            return false;
        }

        let p = if len == -1 {
            strchr(name, AUTOLOAD_CHAR as c_int)
        } else {
            memchr(name.cast(), AUTOLOAD_CHAR as c_int, len as usize).cast::<c_char>()
        };

        p.is_null()
    }
}

pub unsafe fn func_call(
    name: *mut c_char,
    args: *mut typval_T,
    partial: *mut partial_T,
    selfdict: *mut dict_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let mut argv: [typval_T; MAX_FUNC_ARGS + 1] = [typval_T::new(); MAX_FUNC_ARGS + 1];
        let mut argc = 0;
        let mut r = 0;

        let list = (*args).vval.v_list;
        let max_argc = MAX_FUNC_ARGS as c_int
            - if partial.is_null() {
                0
            } else {
                (*partial).pt_argv.len() as c_int
            };
        let mut skip_call = false;
        if !list.is_null() {
            let mut item = (*list).lv_first;
            while !item.is_null() {
                if argc == max_argc {
                    emsg(gettext(c"E699: Too many arguments".as_ptr()));
                    skip_call = true;
                    break;
                }
                tv_copy(&raw const (*item).li_tv, &raw mut argv[argc as usize]);
                argc += 1;
                item = (*item).li_next;
            }
        }

        if !skip_call {
            let mut funcexe = funcexe_T::new();
            funcexe.fe_firstline = (*curwin).w_cursor.lnum;
            funcexe.fe_lastline = (*curwin).w_cursor.lnum;
            funcexe.fe_evaluate = true;
            funcexe.fe_partial = partial;
            funcexe.fe_selfdict = selfdict;
            r = call_func(name, -1, rettv, argc, argv.as_mut_ptr(), &raw mut funcexe);
        }

        while argc > 0 {
            argc -= 1;
            tv_clear(&raw mut argv[argc as usize]);
        }

        r
    }
}

pub unsafe fn callback_call_retnr(
    callback: *mut Callback,
    argcount: c_int,
    argvars: *mut typval_T,
) -> varnumber_T {
    unsafe {
        let mut rettv: typval_T = typval_T::new();
        if !callback_call(callback, argcount, argvars, &raw mut rettv) {
            return -2;
        }

        let retval = tv_get_number_chk(&raw const rettv, ptr::null_mut());
        tv_clear(&raw mut rettv);
        retval
    }
}

unsafe fn user_func_error(error: c_int, name: *const c_char, found_var: bool) {
    unsafe {
        match error {
            FCERR_UNKNOWN => {
                if found_var {
                    semsg!(gettext(e_not_callable_type_str.as_ptr()), name);
                } else {
                    emsg_funcname(e_unknown_function_str.as_ptr(), name);
                }
            }
            FCERR_NOTMETHOD => {
                emsg_funcname(c"E276: Cannot use function as a method: %s".as_ptr(), name);
            }
            FCERR_DELETED => {
                emsg_funcname(c"E933: Function was deleted: %s".as_ptr(), name);
            }
            FCERR_TOOMANY => {
                emsg_funcname(gettext(e_toomanyarg.as_ptr()), name);
            }
            FCERR_TOOFEW => {
                emsg_funcname(gettext(e_toofewarg.as_ptr()), name);
            }
            FCERR_SCRIPT => {
                emsg_funcname(
                    c"E120: Using <SID> not in a script context: %s".as_ptr(),
                    name,
                );
            }
            FCERR_DICT => {
                emsg_funcname(
                    c"E725: Calling dict function without Dictionary: %s".as_ptr(),
                    name,
                );
            }
            _ => {}
        }
    }
}

unsafe fn argv_add_base(
    basetv: *mut typval_T,
    argvars: *mut *mut typval_T,
    argcount: *mut c_int,
    new_argvars: *mut typval_T,
    argv_base: *mut c_int,
) {
    unsafe {
        if !basetv.is_null() {
            ptr::copy(*argvars, new_argvars.add(1), *argcount as usize);
            *new_argvars = *basetv;
            *argcount += 1;
            *argvars = new_argvars;
            *argv_base = 1;
        }
    }
}

/// Call a function with its resolved parameters.
pub unsafe fn call_func(
    mut funcname: *const c_char,
    mut len: c_int,
    rettv: *mut typval_T,
    argcount_in: c_int,
    argvars_in: *mut typval_T,
    funcexe: *mut funcexe_T,
) -> c_int {
    unsafe {
        let mut ret = FAIL;
        let mut error = FCERR_NONE;
        let mut fp: *mut ufunc_T = ptr::null_mut();
        let mut fname_buf = [0 as c_char; FLEN_FIXED + 1];
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut fname: *mut c_char = ptr::null_mut();
        let mut name: *mut c_char = ptr::null_mut();
        let mut argcount = argcount_in;
        let mut argvars = argvars_in;
        let mut selfdict = (*funcexe).fe_selfdict;
        let mut argv: [typval_T; MAX_FUNC_ARGS + 1] = [typval_T::new(); MAX_FUNC_ARGS + 1];
        let mut argv_clear = 0;
        let mut argv_base = 0;
        let partial = (*funcexe).fe_partial;

        (*rettv).v_type = VarType::VAR_UNKNOWN;

        if len <= 0 {
            len = strlen(funcname) as c_int;
        }
        if !partial.is_null() {
            fp = (*partial).pt_func;
        }
        if fp.is_null() {
            name = xmemdupz(funcname.cast(), len as usize).cast();
            fname = fname_trans_sid(
                name,
                fname_buf.as_mut_ptr(),
                &raw mut tofree,
                &raw mut error,
            );
        }

        if !(*funcexe).fe_doesrange.is_null() {
            *(*funcexe).fe_doesrange = false;
        }

        let mut skip_eval = false;
        if !partial.is_null() {
            if !(*partial).pt_dict.is_null() && (selfdict.is_null() || !(*partial).pt_auto) {
                selfdict = (*partial).pt_dict;
            }
            let pt_argc = (*partial).pt_argv.len() as c_int;
            if error == FCERR_NONE && pt_argc > 0 {
                while argv_clear < pt_argc {
                    if argv_clear + argcount_in >= MAX_FUNC_ARGS as c_int {
                        error = FCERR_TOOMANY;
                        skip_eval = true;
                        break;
                    }
                    tv_copy(
                        &(&(*partial).pt_argv)[argv_clear as usize],
                        &raw mut argv[argv_clear as usize],
                    );
                    argv_clear += 1;
                }
                if !skip_eval {
                    for i in 0..argcount_in {
                        argv[(i + argv_clear) as usize] = *argvars_in.add(i as usize);
                    }
                    argvars = argv.as_mut_ptr();
                    argcount = pt_argc + argcount_in;
                }
            }
        }

        if !skip_eval && error == FCERR_NONE && (*funcexe).fe_evaluate {
            let is_global =
                fp.is_null() && *fname == b'g' as c_char && *fname.add(1) == b':' as c_char;
            let rfname = if is_global { fname.add(2) } else { fname };

            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = 0;
            error = FCERR_UNKNOWN;

            if is_luafunc(partial) {
                if len > 0 {
                    error = FCERR_NONE;
                    argv_add_base(
                        (*funcexe).fe_basetv,
                        &raw mut argvars,
                        &raw mut argcount,
                        argv.as_mut_ptr(),
                        &raw mut argv_base,
                    );
                    nlua_call_vlua(funcname, len as usize, argvars, argcount, rettv);
                } else {
                    xfree(name.cast());
                    name = ptr::null_mut();
                    funcname = c"v:lua".as_ptr();
                }
            } else if !fp.is_null() || !builtin_function(rfname, -1) {
                if fp.is_null() {
                    fp = find_func(rfname);
                }

                if fp.is_null()
                    && apply_autocmds(EVENT_FUNCUNDEFINED, rfname, rfname, true, ptr::null_mut())
                    && !aborting()
                {
                    fp = find_func(rfname);
                }
                if fp.is_null() && script_autoload(rfname, strlen(rfname), true) && !aborting() {
                    fp = find_func(rfname);
                }

                if !fp.is_null() && ((*fp).uf_flags & FC_DELETED) != 0 {
                    error = FCERR_DELETED;
                } else if !fp.is_null() {
                    if let Some(argv_fn) = (*funcexe).fe_argv_func {
                        argcount = argv_fn(argcount, argvars, argv_clear, fp);
                    }

                    argv_add_base(
                        (*funcexe).fe_basetv,
                        &raw mut argvars,
                        &raw mut argcount,
                        argv.as_mut_ptr(),
                        &raw mut argv_base,
                    );

                    error = call_user_func_check(fp, argcount, argvars, rettv, funcexe, selfdict);
                }
            } else if !(*funcexe).fe_basetv.is_null() {
                error = call_internal_method(fname, argcount, argvars, rettv, (*funcexe).fe_basetv);
            } else {
                error = call_internal_func(fname, argcount, argvars, rettv);
            }
            update_force_abort();
        }
        if !skip_eval && error == FCERR_NONE {
            ret = OK;
        }

        if !aborting() {
            user_func_error(
                error,
                if !name.is_null() { name } else { funcname },
                (*funcexe).fe_found_var,
            );
        }

        while argv_clear > 0 {
            argv_clear -= 1;
            tv_clear(&raw mut argv[(argv_clear + argv_base) as usize]);
        }

        xfree(tofree.cast());
        xfree(name.cast());

        ret
    }
}

pub unsafe fn call_simple_luafunc(
    funcname: *const c_char,
    len: usize,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        let mut argvars: [typval_T; 1] = [typval_T::new(); 1];
        argvars[0].v_type = VarType::VAR_UNKNOWN;
        nlua_call_vlua(funcname, len, argvars.as_mut_ptr(), 0, rettv);
        OK
    }
}

pub unsafe fn call_simple_func(funcname: *const c_char, len: usize, rettv: *mut typval_T) -> c_int {
    unsafe {
        let mut ret = FAIL;

        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        let name = xstrnsave(funcname, len);

        let mut error = FCERR_NONE;
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut fname_buf = [0 as c_char; FLEN_FIXED + 1];
        let fname = fname_trans_sid(
            name,
            fname_buf.as_mut_ptr(),
            &raw mut tofree,
            &raw mut error,
        );

        let is_global = *fname == b'g' as c_char && *fname.add(1) == b':' as c_char;
        let rfname = if is_global { fname.add(2) } else { fname };

        let fp = find_func(rfname);
        if fp.is_null() {
            ret = NOTDONE;
        } else if ((*fp).uf_flags & FC_DELETED) != 0 {
            error = FCERR_DELETED;
        } else {
            let mut argvars: [typval_T; 1] = [typval_T::new(); 1];
            argvars[0].v_type = VarType::VAR_UNKNOWN;
            let mut funcexe = funcexe_T::new();
            funcexe.fe_evaluate = true;

            error = call_user_func_check(
                fp,
                0,
                argvars.as_mut_ptr(),
                rettv,
                &raw mut funcexe,
                ptr::null_mut(),
            );
            if error == FCERR_NONE {
                ret = OK;
            }
        }

        user_func_error(error, name, false);
        xfree(tofree.cast());
        xfree(name.cast());

        ret
    }
}

pub unsafe fn printable_func_name(fp: *mut ufunc_T) -> *mut c_char {
    unsafe {
        if let Some(ref exp) = (*fp).uf_name_exp {
            exp.as_ptr().cast_mut()
        } else {
            uf2hikey(fp)
        }
    }
}

unsafe fn function_list_modified(prev_ht_changed: c_int) -> bool {
    unsafe {
        if prev_ht_changed != func_hashtab_changed {
            emsg(gettext(E_FUNCTION_LIST_WAS_MODIFIED.as_ptr()));
            return true;
        }
        false
    }
}

unsafe fn list_func_head(fp: *mut ufunc_T, indent: bool, force: bool) -> c_int {
    unsafe {
        let prev_ht_changed = func_hashtab_changed;

        msg_start();

        if function_list_modified(prev_ht_changed) {
            return FAIL;
        }

        if indent {
            msg_puts(c"   ".as_ptr());
        }
        msg_puts(if force {
            c"function! ".as_ptr()
        } else {
            c"function ".as_ptr()
        });
        if let Some(ref exp) = (*fp).uf_name_exp {
            msg_puts(exp.as_ptr());
        } else {
            msg_puts(uf2hikey(fp));
        }
        msg_putchar(b'(' as c_int);
        let uf_args_len = (*fp).uf_args.len() as c_int;
        let uf_def_args_len = (*fp).uf_def_args.len() as c_int;
        let mut j = 0;
        while j < uf_args_len {
            if j != 0 {
                msg_puts(c", ".as_ptr());
            }
            msg_puts(funcarg(fp, j));
            if j >= uf_args_len - uf_def_args_len {
                msg_puts(c" = ".as_ptr());
                msg_puts((&(*fp).uf_def_args)[(j - uf_args_len + uf_def_args_len) as usize]);
            }
            j += 1;
        }
        if (*fp).uf_varargs != 0 {
            if j != 0 {
                msg_puts(c", ".as_ptr());
            }
            msg_puts(c"...".as_ptr());
        }
        msg_putchar(b')' as c_int);
        if ((*fp).uf_flags & FC_ABORT) != 0 {
            msg_puts(c" abort".as_ptr());
        }
        if ((*fp).uf_flags & FC_RANGE) != 0 {
            msg_puts(c" range".as_ptr());
        }
        if ((*fp).uf_flags & FC_DICT) != 0 {
            msg_puts(c" dict".as_ptr());
        }
        if ((*fp).uf_flags & FC_CLOSURE) != 0 {
            msg_puts(c" closure".as_ptr());
        }
        msg_clr_eos();
        if p_verbose > 0 {
            last_set_msg((*fp).uf_script_ctx);
        }

        OK
    }
}

/// Get a function name, translating `<SID>` and `<SNR>`.
pub unsafe fn trans_function_name(
    pp: *mut *mut c_char,
    skip: bool,
    flags: c_int,
    fdp: *mut funcdict_T,
    partial: *mut *mut partial_T,
) -> *mut c_char {
    unsafe {
        let mut name: *mut c_char = ptr::null_mut();
        let mut len: c_int;
        let mut lv: lval_T = lval_T::new();

        if !fdp.is_null() {
            *fdp = funcdict_T::new();
        }
        let mut start: *const c_char = *pp;

        if *(*pp) as u8 == K_SPECIAL
            && *(*pp).add(1) as u8 == KS_EXTRA
            && *(*pp).add(2) == KE_SNR as c_char
        {
            *pp = (*pp).add(3);
            len = get_id_len(pp.cast::<*const c_char>()) + 3;
            return xmemdupz(start.cast(), len as usize).cast();
        }

        let mut lead = eval_fname_script(start);
        if lead > 2 {
            start = start.add(lead as usize);
        }

        let end = get_lval(
            start.cast_mut(),
            ptr::null_mut(),
            &raw mut lv,
            false,
            skip,
            flags | GLV_READ_ONLY,
            if lead > 2 { 0 } else { FNE_CHECK_START },
        )
        .cast_const();
        if end == start {
            if !skip {
                emsg(gettext(c"E129: Function name required".as_ptr()));
            }
            return name;
        }
        if end.is_null() || (!lv.ll_tv.is_null() && (lead > 2 || lv.ll_range)) {
            if !aborting() {
                if !end.is_null() {
                    semsg!(gettext(e_invarg2.as_ptr()), start);
                }
            } else {
                *pp =
                    find_name_end(start, ptr::null_mut(), ptr::null_mut(), FNE_INCL_BR).cast_mut();
            }
            return name;
        }

        if !lv.ll_tv.is_null() {
            if !fdp.is_null() {
                (*fdp).fd_dict = lv.ll_dict;
                (*fdp).fd_newkey = lv.ll_newkey.take();
                (*fdp).fd_di = lv.ll_di;
            }
            if (*lv.ll_tv).v_type == VarType::VAR_FUNC && !(*lv.ll_tv).vval.v_string.is_null() {
                name = xstrdup((*lv.ll_tv).vval.v_string);
                *pp = end.cast_mut();
            } else if (*lv.ll_tv).v_type == VarType::VAR_PARTIAL
                && !(*lv.ll_tv).vval.v_partial.is_null()
            {
                let pt = (*lv.ll_tv).vval.v_partial;
                if is_luafunc(pt) && *end == b'.' as c_char {
                    len = check_luafunc_name(end.add(1), true);
                    if len == 0 {
                        semsg!(e_invexpr2.as_ptr(), c"v:lua".as_ptr());
                        return name;
                    }
                    name = xmallocz(len as usize).cast();
                    ptr::copy_nonoverlapping(end.add(1), name, len as usize);
                    *pp = end.cast_mut().add(1 + len as usize);
                } else {
                    name = xstrdup(partial_name(pt));
                    *pp = end.cast_mut();
                }
                if !partial.is_null() {
                    *partial = pt;
                }
            } else {
                if !skip
                    && (flags & TFN_QUIET) == 0
                    && (fdp.is_null() || lv.ll_dict.is_null() || (*fdp).fd_newkey.is_none())
                {
                    emsg(gettext(E_FUNCREF.as_ptr()));
                } else {
                    *pp = end.cast_mut();
                }
                name = ptr::null_mut();
            }
            return name;
        }

        if lv.ll_name.is_null() {
            *pp = end.cast_mut();
            return name;
        }

        if let Some(ref exp_name) = lv.ll_exp_name {
            let exp_ptr = exp_name.as_ptr().cast_mut();
            len = exp_name.as_bytes().len() as c_int;
            name = deref_func_name(
                exp_ptr,
                &raw mut len,
                partial,
                (flags & TFN_NO_AUTOLOAD) != 0,
                ptr::null_mut(),
            );
            if name == exp_ptr {
                name = ptr::null_mut();
            }
        } else if (flags & TFN_NO_DEREF) == 0 {
            len = end.offset_from(*pp) as c_int;
            name = deref_func_name(
                *pp,
                &raw mut len,
                partial,
                (flags & TFN_NO_AUTOLOAD) != 0,
                ptr::null_mut(),
            );
            if name == *pp {
                name = ptr::null_mut();
            }
        }
        if !name.is_null() {
            name = xstrdup(name);
            *pp = end.cast_mut();
            if strncmp(name, c"<SNR>".as_ptr(), 5) == 0 {
                *name.add(0) = K_SPECIAL as c_char;
                *name.add(1) = KS_EXTRA as c_char;
                *name.add(2) = KE_SNR as c_char;
                let tail_len = strlen(name.add(5)) + 1;
                ptr::copy(name.add(5), name.add(3), tail_len);
            }
            return name;
        }

        if let Some(ref exp_name) = lv.ll_exp_name {
            len = exp_name.as_bytes().len() as c_int;
            if lead <= 2
                && lv.ll_name == exp_name.as_ptr()
                && lv.ll_name_len >= 2
                && strncmp(lv.ll_name, c"s:".as_ptr(), 2) == 0
            {
                lv.ll_name = lv.ll_name.add(2);
                lv.ll_name_len -= 2;
                len -= 2;
                lead = 2;
            }
        } else {
            if lead == 2 || (*lv.ll_name == b'g' as c_char && *lv.ll_name.add(1) == b':' as c_char)
            {
                lv.ll_name = lv.ll_name.add(2);
                lv.ll_name_len = lv.ll_name_len.wrapping_sub(2);
            }
            len = end.offset_from(lv.ll_name) as c_int;
        }

        let mut sid_buflen = 0usize;
        let mut sid_buf = [0 as c_char; 20];

        if skip {
            lead = 0;
        } else if lead > 0 {
            lead = 3;
            if lv
                .ll_exp_name
                .as_ref()
                .is_some_and(|s| eval_fname_sid(s.as_ptr()))
                || eval_fname_sid(*pp)
            {
                if current_sctx.sc_sid <= 0 {
                    emsg(gettext(e_usingsid.as_ptr()));
                    return name;
                }
                sid_buflen = vim_snprintf!(
                    sid_buf.as_mut_ptr(),
                    sid_buf.len(),
                    c"%d_".as_ptr(),
                    current_sctx.sc_sid,
                ) as usize;
                lead += sid_buflen as c_int;
            }
        } else if (flags & TFN_INT) == 0 && builtin_function(lv.ll_name, lv.ll_name_len as c_int) {
            semsg!(
                gettext(c"E128: Function name must start with a capital or \"s:\": %s".as_ptr()),
                start,
            );
            return name;
        }

        if !skip && (flags & TFN_QUIET) == 0 && (flags & TFN_NO_DEREF) == 0 {
            let cp = xmemrchr(lv.ll_name.cast(), b':', lv.ll_name_len).cast::<c_char>();
            if !cp.is_null() && cp.cast_const() < end {
                semsg!(
                    gettext(c"E884: Function name cannot contain a colon: %s".as_ptr()),
                    start,
                );
                return name;
            }
        }

        name = xmalloc(len as usize + lead as usize + 1).cast();
        if !skip && lead > 0 {
            *name.add(0) = K_SPECIAL as c_char;
            *name.add(1) = KS_EXTRA as c_char;
            *name.add(2) = KE_SNR as c_char;
            if sid_buflen > 0 {
                ptr::copy_nonoverlapping(sid_buf.as_ptr(), name.add(3), sid_buflen);
            }
        }
        ptr::copy(lv.ll_name, name.add(lead as usize), len as usize);
        *name.add((lead + len) as usize) = NUL;
        *pp = end.cast_mut();

        name
    }
}

pub unsafe fn get_scriptlocal_funcname(funcname: *mut c_char) -> *mut c_char {
    unsafe {
        if funcname.is_null() {
            return ptr::null_mut();
        }

        if strncmp(funcname, c"s:".as_ptr(), 2) != 0 && strncmp(funcname, c"<SID>".as_ptr(), 5) != 0
        {
            return ptr::null_mut();
        }

        if !crate::runtime::script_id_valid(current_sctx.sc_sid) {
            emsg(gettext(e_usingsid.as_ptr()));
            return ptr::null_mut();
        }

        let mut sid_buf = [0 as c_char; 25];
        let sid_buflen = vim_snprintf!(
            sid_buf.as_mut_ptr(),
            sid_buf.len(),
            c"<SNR>%d_".as_ptr(),
            current_sctx.sc_sid,
        ) as usize;
        let off = if *funcname == b's' as c_char { 2 } else { 5 };
        let newnamesize = sid_buflen + strlen(funcname.add(off)) + 1;
        let newname = xmalloc(newnamesize).cast::<c_char>();
        vim_snprintf!(
            newname,
            newnamesize,
            c"%s%s".as_ptr(),
            sid_buf.as_ptr(),
            funcname.add(off),
        );

        newname
    }
}

/// Call `trans_function_name()`, except that a lambda is returned as-is.
pub unsafe fn save_function_name(
    name: *mut *mut c_char,
    skip: bool,
    flags: c_int,
    fudi: *mut funcdict_T,
) -> *mut c_char {
    unsafe {
        let mut p = *name;
        let saved: *mut c_char;

        if strncmp(p, c"<lambda>".as_ptr(), 8) == 0 {
            p = p.add(8);
            getdigits(&raw mut p, false, 0);
            saved = xmemdupz((*name).cast(), p.offset_from(*name) as usize).cast();
            if !fudi.is_null() {
                *fudi = funcdict_T::new();
            }
        } else {
            saved = trans_function_name(&raw mut p, skip, flags, fudi, ptr::null_mut());
        }
        *name = p;
        saved
    }
}

unsafe fn list_functions(regmatch: *mut regmatch_T) {
    unsafe {
        let prev_ht_changed = func_hashtab_changed;
        let ht = &raw const func_hashtab;
        let funcs: Vec<*mut ufunc_T> = (*ht).values().copied().collect();

        msg_ext_set_kind(c"list_cmd".as_ptr());
        for fp in funcs {
            if got_int {
                break;
            }
            let matches = if regmatch.is_null() {
                !message_filtered(uf2hikey(fp)) && !func_name_refcount(uf2hikey(fp))
            } else {
                !(*uf2hikey(fp) as u8).is_ascii_digit() && vim_regexec(regmatch, uf2hikey(fp), 0)
            };
            if matches {
                if list_func_head(fp, false, false) == FAIL {
                    return;
                }
                if function_list_modified(prev_ht_changed) {
                    return;
                }
            }
        }
    }
}

unsafe fn list_functions_matching_pat(eap: *mut exarg_T) -> *mut c_char {
    unsafe {
        let mut p = skip_regexp((*eap).arg.add(1), b'/' as c_int, 1);
        if (*eap).skip == 0 {
            let mut regmatch: regmatch_T = regmatch_T::new();

            let c = *p;
            *p = NUL;
            regmatch.regprog = vim_regcomp((*eap).arg.add(1), RE_MAGIC);
            *p = c;
            if !regmatch.regprog.is_null() {
                regmatch.rm_ic = p_ic != 0;
                list_functions(&raw mut regmatch);
                vim_regfree(regmatch.regprog);
            }
        }
        if *p == b'/' as c_char {
            p = p.add(1);
        }

        p
    }
}

unsafe fn list_one_function(eap: *mut exarg_T, name: *mut c_char, p: *mut c_char) -> *mut ufunc_T {
    unsafe {
        if ends_excmd(*skipwhite(p) as c_int) == 0 {
            semsg!(gettext(e_trailing_arg.as_ptr()), p);
            return ptr::null_mut();
        }

        (*eap).nextcmd = check_nextcmd(p);

        if !(*eap).nextcmd.is_null() {
            *p = NUL;
        }

        if (*eap).skip != 0 || got_int {
            return ptr::null_mut();
        }

        let fp = find_func(name);
        if fp.is_null() {
            emsg_funcname(c"E123: Undefined function: %s".as_ptr(), name);
            return ptr::null_mut();
        }

        let prev_ht_changed = func_hashtab_changed;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if list_func_head(fp, (*eap).forceit == 0, (*eap).forceit != 0) != OK {
            return fp;
        }

        let mut j = 0;
        while j < (*fp).uf_lines.len() as c_int && !got_int {
            let line = (&(*fp).uf_lines)[j as usize];
            if line.is_null() {
                j += 1;
                continue;
            }
            msg_putchar(b'\n' as c_int);
            if (*eap).forceit == 0 {
                msg_outnum(j + 1);
                if j < 9 {
                    msg_putchar(b' ' as c_int);
                }
                if j < 99 {
                    msg_putchar(b' ' as c_int);
                }
                if function_list_modified(prev_ht_changed) {
                    break;
                }
            }
            msg_prt_line(line, false);
            line_breakcheck();
            j += 1;
        }

        if !got_int {
            msg_putchar(b'\n' as c_int);
            if !function_list_modified(prev_ht_changed) {
                msg_puts(if (*eap).forceit != 0 {
                    c"endfunction".as_ptr()
                } else {
                    c"   endfunction".as_ptr()
                });
            }
        }

        fp
    }
}

unsafe fn get_function_body(
    eap: *mut exarg_T,
    newlines: &mut Vec<*mut c_char>,
    line_arg_in: *mut c_char,
    line_to_free: *mut *mut c_char,
    show_block: bool,
) -> c_int {
    unsafe {
        let mut saved_wait_return = need_wait_return;
        let mut line_arg = line_arg_in;
        let mut indent: c_int = 2;
        let mut nesting: c_int = 0;
        let mut skip_until: *mut c_char = ptr::null_mut();
        let mut ret = FAIL;
        let mut is_heredoc = false;
        let mut heredoc_trimmed: *mut c_char = ptr::null_mut();
        let mut heredoc_trimmedlen: usize = 0;
        let mut do_concat = true;

        loop {
            if KeyTyped {
                msg_scroll = 1;
                saved_wait_return = false;
            }
            need_wait_return = false;

            let theline: *mut c_char;
            let mut p: *mut c_char;
            let mut arg: *mut c_char;

            if !line_arg.is_null() {
                theline = line_arg;
                p = vim_strchr(theline, b'\n' as c_int);
                if p.is_null() {
                    line_arg = line_arg.add(strlen(line_arg));
                } else {
                    *p = NUL;
                    line_arg = p.add(1);
                }
            } else {
                xfree((*line_to_free).cast());
                theline = if (*eap).ea_getline.is_none() {
                    getcmdline(b':' as c_int, 0, indent, do_concat)
                } else {
                    (*eap).ea_getline.call(b':' as c_int, indent, do_concat)
                };
                *line_to_free = theline;
            }
            if KeyTyped {
                lines_left = Rows - 1;
            }
            if theline.is_null() {
                if !skip_until.is_null() {
                    semsg!(
                        gettext(E_MISSING_HEREDOC_END_MARKER_STR.as_ptr()),
                        skip_until,
                    );
                } else {
                    emsg(gettext(c"E126: Missing :endfunction".as_ptr()));
                }
                xfree(skip_until.cast());
                xfree(heredoc_trimmed.cast());
                need_wait_return |= saved_wait_return;
                return ret;
            }
            if show_block {
                debug_assert!(indent >= 0);
                ui_ext_cmdline_block_append(indent as usize, theline);
            }

            let mut sourcing_lnum_off = get_sourced_lnum((*eap).ea_getline);
            let cur_sourcing_lnum = sourcing_lnum();
            if cur_sourcing_lnum < sourcing_lnum_off {
                sourcing_lnum_off -= cur_sourcing_lnum;
            } else {
                sourcing_lnum_off = 0;
            }

            if !skip_until.is_null() {
                if heredoc_trimmed.is_null()
                    || (is_heredoc && skipwhite(theline) == theline)
                    || strncmp(theline, heredoc_trimmed, heredoc_trimmedlen) == 0
                {
                    p = if heredoc_trimmed.is_null() {
                        theline
                    } else if is_heredoc {
                        if skipwhite(theline) == theline {
                            theline
                        } else {
                            theline.add(heredoc_trimmedlen)
                        }
                    } else {
                        theline.add(heredoc_trimmedlen)
                    };
                    if strcmp(p, skip_until) == 0 {
                        xfree(skip_until.cast());
                        skip_until = ptr::null_mut();
                        xfree(heredoc_trimmed.cast());
                        heredoc_trimmed = ptr::null_mut();
                        heredoc_trimmedlen = 0;
                        do_concat = true;
                        is_heredoc = false;
                    }
                }
            } else {
                p = theline;
                while ascii_iswhite(*p as c_int) || *p == b':' as c_char {
                    p = p.add(1);
                }

                if checkforcmd(&raw mut p, c"endfunction".as_ptr(), 4) {
                    let prev_nesting = nesting;
                    nesting -= 1;
                    if prev_nesting == 0 {
                        if *p == b'!' as c_char {
                            p = p.add(1);
                        }
                        let mut nextcmd: *mut c_char = ptr::null_mut();
                        if *p == b'|' as c_char {
                            nextcmd = p.add(1);
                        } else if !line_arg.is_null() && *skipwhite(line_arg) != NUL {
                            nextcmd = line_arg;
                        } else if *p != NUL && *p != b'"' as c_char && p_verbose > 0 {
                            swmsg!(
                                true,
                                gettext(c"W22: Text found after :endfunction: %s".as_ptr()),
                                p,
                            );
                        }
                        if !nextcmd.is_null() {
                            (*eap).nextcmd = nextcmd;
                            if !(*line_to_free).is_null() {
                                xfree((*(*eap).cmdlinep).cast());
                                *(*eap).cmdlinep = *line_to_free;
                                *line_to_free = ptr::null_mut();
                            }
                        }
                        break;
                    }
                }

                if indent > 2 && strncmp(p, c"end".as_ptr(), 3) == 0 {
                    indent -= 2;
                } else if strncmp(p, c"if".as_ptr(), 2) == 0
                    || strncmp(p, c"wh".as_ptr(), 2) == 0
                    || strncmp(p, c"for".as_ptr(), 3) == 0
                    || strncmp(p, c"try".as_ptr(), 3) == 0
                {
                    indent += 2;
                }

                if checkforcmd(&raw mut p, c"function".as_ptr(), 2) {
                    if *p == b'!' as c_char {
                        p = skipwhite(p.add(1));
                    }
                    p = p.add(eval_fname_script(p) as usize);
                    xfree(
                        trans_function_name(&raw mut p, true, 0, ptr::null_mut(), ptr::null_mut())
                            .cast(),
                    );
                    if *skipwhite(p) == b'(' as c_char {
                        if nesting == MAX_FUNC_NESTING - 1 {
                            emsg(gettext(E_FUNCTION_NESTING_TOO_DEEP.as_ptr()));
                        } else {
                            nesting += 1;
                            indent += 2;
                        }
                    }
                }

                p = skip_range(p, ptr::null_mut());
                let tp = p;
                if (checkforcmd(&raw mut p, c"append".as_ptr(), 1)
                    || checkforcmd(&raw mut p, c"change".as_ptr(), 1)
                    || checkforcmd(&raw mut p, c"insert".as_ptr(), 1))
                    && (*p == b'!' as c_char
                        || *p == b'|' as c_char
                        || ascii_iswhite_nl_or_nul(*p as c_int))
                {
                    skip_until = xmemdupz(c".".as_ptr().cast(), 1).cast();
                } else {
                    p = tp;
                }

                arg = skipwhite(skiptowhite(p));
                let p0 = *p as u8;
                let p1 = if p0 != 0 { *p.add(1) as u8 } else { 0 };
                let p2 = if p1 != 0 { *p.add(2) as u8 } else { 0 };
                let p3 = if p2 != 0 { *p.add(3) as u8 } else { 0 };
                let is_script_heredoc = *arg == b'<' as c_char
                    && *arg.add(1) == b'<' as c_char
                    && ((p0 == b'p'
                        && p1 == b'y'
                        && (!ascii_isalnum(p2 as c_int)
                            || p2 == b't'
                            || ((p2 == b'3' || p2 == b'x') && !p3.is_ascii_alphabetic())))
                        || (p0 == b'p' && p1 == b'e' && (!p2.is_ascii_alphabetic() || p2 == b'r'))
                        || (p0 == b't' && p1 == b'c' && (!p2.is_ascii_alphabetic() || p2 == b'l'))
                        || (p0 == b'l' && p1 == b'u' && p2 == b'a' && !p3.is_ascii_alphabetic())
                        || (p0 == b'r'
                            && p1 == b'u'
                            && p2 == b'b'
                            && (!p3.is_ascii_alphabetic() || p3 == b'y'))
                        || (p0 == b'm' && p1 == b'z' && (!p2.is_ascii_alphabetic() || p2 == b's')));
                if is_script_heredoc {
                    p = skipwhite(arg.add(2));
                    if strncmp(p, c"trim".as_ptr(), 4) == 0
                        && (*p.add(4) == NUL || ascii_iswhite(*p.add(4) as c_int))
                    {
                        p = skipwhite(p.add(4));
                        heredoc_trimmedlen = skipwhite(theline).offset_from(theline) as usize;
                        heredoc_trimmed = xmemdupz(theline.cast(), heredoc_trimmedlen).cast();
                    }
                    if *p == NUL {
                        skip_until = xmemdupz(c".".as_ptr().cast(), 1).cast();
                    } else {
                        skip_until =
                            xmemdupz(p.cast(), skiptowhite(p).offset_from(p) as usize).cast();
                    }
                    do_concat = false;
                    is_heredoc = true;
                }

                if !is_heredoc {
                    arg = p;
                    if checkforcmd(&raw mut arg, c"let".as_ptr(), 2)
                        || checkforcmd(&raw mut p, c"const".as_ptr(), 5)
                    {
                        let mut var_count = 0;
                        let mut semicolon = 0;
                        arg = skip_var_list(arg, &raw mut var_count, &raw mut semicolon, true)
                            .cast_mut();
                        if !arg.is_null() {
                            arg = skipwhite(arg);
                        }
                        if !arg.is_null() && strncmp(arg, c"=<<".as_ptr(), 3) == 0 {
                            p = skipwhite(arg.add(3));
                            let mut has_trim = false;
                            loop {
                                if strncmp(p, c"trim".as_ptr(), 4) == 0
                                    && (*p.add(4) == NUL || ascii_iswhite(*p.add(4) as c_int))
                                {
                                    p = skipwhite(p.add(4));
                                    has_trim = true;
                                    continue;
                                }
                                if strncmp(p, c"eval".as_ptr(), 4) == 0
                                    && (*p.add(4) == NUL || ascii_iswhite(*p.add(4) as c_int))
                                {
                                    p = skipwhite(p.add(4));
                                    continue;
                                }
                                break;
                            }
                            if has_trim {
                                heredoc_trimmedlen =
                                    skipwhite(theline).offset_from(theline) as usize;
                                heredoc_trimmed =
                                    xmemdupz(theline.cast(), heredoc_trimmedlen).cast();
                            }
                            xfree(skip_until.cast());
                            skip_until =
                                xmemdupz(p.cast(), skiptowhite(p).offset_from(p) as usize).cast();
                            do_concat = false;
                            is_heredoc = true;
                        }
                    }
                }
            }

            p = xstrdup(theline);
            newlines.push(p);

            while sourcing_lnum_off > 0 {
                sourcing_lnum_off -= 1;
                newlines.push(ptr::null_mut());
            }

            if !line_arg.is_null() && *line_arg == NUL {
                line_arg = ptr::null_mut();
            }
        }

        if did_emsg == 0 {
            ret = OK;
        }

        xfree(skip_until.cast());
        xfree(heredoc_trimmed.cast());
        need_wait_return |= saved_wait_return;
        ret
    }
}

/// `:function`
pub unsafe fn ex_function(eap: *mut exarg_T) {
    unsafe {
        let mut line_to_free: *mut c_char = ptr::null_mut();
        let mut line_arg: *mut c_char = ptr::null_mut();
        let mut newargs: Vec<*mut c_char> = Vec::new();
        let mut default_args: Vec<*mut c_char> = Vec::new();
        let mut newlines: Vec<*mut c_char> = Vec::new();
        let mut varargs: c_int = 0;
        let mut flags: c_int = 0;
        let mut fp: *mut ufunc_T = ptr::null_mut();
        let mut free_fp = false;
        let mut overwrite = false;
        let mut fudi = funcdict_T::new();
        let mut ht: *mut hashtab_T = ptr::null_mut();
        let mut show_block = false;

        if ends_excmd(*(*eap).arg as c_int) != 0 {
            if (*eap).skip == 0 {
                list_functions(ptr::null_mut());
            }
            (*eap).nextcmd = check_nextcmd((*eap).arg);
            return;
        }

        if *(*eap).arg == b'/' as c_char {
            let p = list_functions_matching_pat(eap);
            (*eap).nextcmd = check_nextcmd(p);
            return;
        }

        let mut p = (*eap).arg;
        let mut name =
            save_function_name(&raw mut p, (*eap).skip != 0, TFN_NO_AUTOLOAD, &raw mut fudi);
        let paren = !vim_strchr(p, b'(' as c_int).is_null();
        if name.is_null() && (fudi.fd_dict.is_null() || !paren) && (*eap).skip == 0 {
            if !aborting() {
                if let Some(ref newkey) = fudi.fd_newkey {
                    semsg!(gettext(e_dictkey.as_ptr()), newkey.as_ptr());
                }
                return;
            }
            (*eap).skip = 1;
        }

        let saved_did_emsg = did_emsg;
        did_emsg = 0;

        macro_rules! ret_free {
            () => {{
                xfree(line_to_free.cast());
                xfree(name.cast());
                did_emsg |= saved_did_emsg;
                if show_block {
                    ui_ext_cmdline_block_leave();
                }
                return;
            }};
        }

        macro_rules! errret_keep {
            () => {{
                crate::path::clear_cstrings(&mut newargs);
                crate::path::clear_cstrings(&mut default_args);
                crate::path::clear_cstrings(&mut newlines);
                ret_free!();
            }};
        }

        macro_rules! errret_2 {
            () => {{
                if free_fp && !fp.is_null() {
                    func_clear_free(fp, false);
                } else if !fp.is_null() {
                    (*fp).uf_name_exp = None;
                }
                errret_keep!();
            }};
        }

        macro_rules! erret {
            () => {{
                errret_2!();
            }};
        }

        if !paren {
            let _ = list_one_function(eap, name, p);
            ret_free!();
        }

        p = skipwhite(p);
        if *p != b'(' as c_char {
            if (*eap).skip == 0 {
                semsg!(gettext(c"E124: Missing '(': %s".as_ptr()), (*eap).arg);
                ret_free!();
            }
            let open_paren = vim_strchr(p, b'(' as c_int);
            if !open_paren.is_null() {
                p = open_paren;
            }
        }
        p = skipwhite(p.add(1));

        if (*eap).skip == 0 {
            let fudi_newkey_ptr = fudi
                .fd_newkey
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            let arg = if !name.is_null() {
                name
            } else {
                fudi_newkey_ptr
            };
            if !arg.is_null() && (fudi.fd_di.is_null() || !tv_is_func((*fudi.fd_di).di_tv)) {
                let mut name_base = arg;
                if arg != fudi_newkey_ptr {
                    if *arg as u8 == K_SPECIAL {
                        name_base = vim_strchr(arg, b'_' as c_int);
                        if name_base.is_null() {
                            name_base = arg.add(3);
                        } else {
                            name_base = name_base.add(1);
                        }
                    }
                    let mut i = 0usize;
                    while *name_base.add(i) != NUL
                        && if i == 0 {
                            eval_isnamec1(*name_base.add(i) as c_int)
                        } else {
                            eval_isnamec(*name_base.add(i) as c_int)
                        }
                    {
                        i += 1;
                    }
                    if *name_base.add(i) != NUL {
                        emsg_funcname(e_invarg2.as_ptr(), arg);
                        ret_free!();
                    }
                }
            }
            if !fudi.fd_dict.is_null() && (*fudi.fd_dict).dv_scope == VAR_DEF_SCOPE {
                emsg(gettext(c"E862: Cannot use g: here".as_ptr()));
                ret_free!();
            }
        }

        if get_function_args(
            &raw mut p,
            b')' as c_char,
            Some(&mut newargs),
            &raw mut varargs,
            Some(&mut default_args),
            (*eap).skip != 0,
        ) == FAIL
        {
            errret_2!();
        }

        if KeyTyped && ui_has(kUICmdline) {
            show_block = true;
            ui_ext_cmdline_block_append(0, (*eap).cmd);
        }

        loop {
            p = skipwhite(p);
            if strncmp(p, c"range".as_ptr(), 5) == 0 {
                flags |= FC_RANGE;
                p = p.add(5);
            } else if strncmp(p, c"dict".as_ptr(), 4) == 0 {
                flags |= FC_DICT;
                p = p.add(4);
            } else if strncmp(p, c"abort".as_ptr(), 5) == 0 {
                flags |= FC_ABORT;
                p = p.add(5);
            } else if strncmp(p, c"closure".as_ptr(), 7) == 0 {
                flags |= FC_CLOSURE;
                p = p.add(7);
                if current_funccal.is_null() {
                    emsg_funcname(
                        c"E932: Closure function should not be at top level: %s".as_ptr(),
                        if name.is_null() { c"".as_ptr() } else { name },
                    );
                    erret!();
                }
            } else {
                break;
            }
        }

        if *p == b'\n' as c_char {
            line_arg = p.add(1);
        } else if *p != NUL && *p != b'"' as c_char && (*eap).skip == 0 && did_emsg == 0 {
            semsg!(gettext(e_trailing_arg.as_ptr()), p);
        }

        if KeyTyped {
            if (*eap).skip == 0 && (*eap).forceit == 0 {
                if !fudi.fd_dict.is_null() && fudi.fd_newkey.is_none() {
                    emsg(gettext(E_FUNCDICT.as_ptr()));
                } else if !name.is_null() && !find_func(name).is_null() {
                    emsg_funcname(E_FUNCEXTS.as_ptr(), name);
                }
            }

            if (*eap).skip == 0 && did_emsg != 0 {
                erret!();
            }

            if !ui_has(kUICmdline) {
                msg_putchar(b'\n' as c_int);
            }
            cmdline_row = msg_row;
        }

        let sourcing_lnum_top = sourcing_lnum();

        if get_function_body(
            eap,
            &mut newlines,
            line_arg,
            &raw mut line_to_free,
            show_block,
        ) == FAIL
            || (*eap).skip != 0
        {
            erret!();
        }

        let mut namelen = 0usize;
        if fudi.fd_dict.is_null() {
            let v = find_var(name, strlen(name), &raw mut ht, 0);
            if !v.is_null() && (*v).di_tv.v_type == VarType::VAR_FUNC {
                emsg_funcname(
                    c"E707: Function name conflicts with variable: %s".as_ptr(),
                    name,
                );
                erret!();
            }

            fp = find_func(name);
            if !fp.is_null() {
                if (*eap).forceit == 0
                    && ((*fp).uf_script_ctx.sc_sid != current_sctx.sc_sid
                        || (*fp).uf_script_ctx.sc_seq == current_sctx.sc_seq)
                {
                    emsg_funcname(E_FUNCEXTS.as_ptr(), name);
                    errret_keep!();
                }
                if (*fp).uf_calls > 0 {
                    emsg_funcname(
                        c"E127: Cannot redefine function %s: It is in use".as_ptr(),
                        name,
                    );
                    errret_keep!();
                }
                if (*fp).uf_refcount > 1 {
                    (*fp).uf_refcount -= 1;
                    (*fp).uf_flags |= FC_REMOVED;
                    fp = ptr::null_mut();
                    overwrite = true;
                } else {
                    let exp_name = (*fp).uf_name_exp.take();
                    xfree(name.cast());
                    name = ptr::null_mut();
                    func_clear_items(fp);
                    (*fp).uf_name_exp = exp_name;
                    (*fp).uf_profiling = 0;
                    (*fp).uf_prof_initialized = 0;
                }
            }
        } else {
            let mut numbuf = [0 as c_char; NUMBUFLEN];

            fp = ptr::null_mut();
            if fudi.fd_newkey.is_none() && (*eap).forceit == 0 {
                emsg(gettext(E_FUNCDICT.as_ptr()));
                erret!();
            }
            if fudi.fd_di.is_null() {
                if value_check_lock((*fudi.fd_dict).dv_lock, (*eap).arg, TV_CSTRING) {
                    erret!();
                }
            } else if value_check_lock((*fudi.fd_di).di_tv.v_lock, (*eap).arg, TV_CSTRING) {
                erret!();
            }

            xfree(name.cast());
            func_nr += 1;
            namelen =
                vim_snprintf!(numbuf.as_mut_ptr(), NUMBUFLEN, c"%d".as_ptr(), func_nr) as usize;
            name = xmemdupz(numbuf.as_ptr().cast(), namelen).cast();
        }

        if fp.is_null() {
            if fudi.fd_dict.is_null() && !vim_strchr(name, AUTOLOAD_CHAR as c_int).is_null() {
                let mut j = FAIL;
                let sname = sourcing_name();
                if !sname.is_null() {
                    let scriptname = autoload_name(name, strlen(name));
                    p = vim_strchr(scriptname, b'/' as c_int);
                    let plen = strlen(p) as c_int;
                    let slen = strlen(sname) as c_int;
                    if slen > plen
                        && path_equal(p, sname.add((slen - plen) as usize), kPathCmpLiteral)
                    {
                        j = OK;
                    }
                    xfree(scriptname.cast());
                }
                if j == FAIL {
                    semsg!(
                        gettext(
                            c"E746: Function name does not match script file name: %s".as_ptr(),
                        ),
                        name,
                    );
                    erret!();
                }
            }

            if namelen == 0 {
                namelen = strlen(name);
            }
            fp = alloc_ufunc(name, namelen);

            if !fudi.fd_dict.is_null() {
                let func_name = xmemdupz(name.cast(), namelen).cast::<c_char>();

                if fudi.fd_di.is_null() {
                    fudi.fd_di = tv_dict_item_alloc(fudi.fd_newkey.as_ref().unwrap().as_ptr());
                    if tv_dict_add(fudi.fd_dict, fudi.fd_di) == FAIL {
                        crate::eval::typval::tv_dict_item_free(fudi.fd_di);
                        xfree(func_name.cast());
                        erret!();
                    }
                } else {
                    tv_clear(&raw mut (*fudi.fd_di).di_tv);
                }
                (*fudi.fd_di).di_tv.v_type = VarType::VAR_FUNC;
                (*fudi.fd_di).di_tv.vval.v_string = func_name;

                flags |= FC_DICT;
            }

            if overwrite {
                let ht = &raw mut func_hashtab;
                (*ht).insert(CStr::from_ptr(name).to_bytes().to_vec(), fp);
            } else if func_hashtab_add(fp) == FAIL {
                free_fp = true;
                erret!();
            }
            (*fp).uf_refcount = 1;
        }
        (*fp).uf_args = newargs;
        (*fp).uf_def_args = default_args;
        (*fp).uf_lines = newlines;
        if (flags & FC_CLOSURE) != 0 {
            register_closure(fp);
        } else {
            (*fp).uf_scoped = ptr::null_mut();
        }
        if prof_def_func() {
            func_do_profile(fp);
        }
        (*fp).uf_varargs = varargs;
        if sandbox != 0 {
            flags |= FC_SANDBOX;
        }
        (*fp).uf_flags = flags;
        (*fp).uf_calls = 0;
        (*fp).uf_script_ctx = current_sctx;
        (*fp).uf_script_ctx.sc_lnum += sourcing_lnum_top;
        nlua_set_sctx(&raw mut (*fp).uf_script_ctx);

        ret_free!();
    }
}

/// Return 5 if `p` starts with `<SID>` or `<SNR>` (ignoring case),
/// 2 if `p` starts with `s:`, 0 otherwise.
pub unsafe fn eval_fname_script(p: *const c_char) -> c_int {
    unsafe {
        if *p == b'<' as c_char
            && (mb_strnicmp(p.add(1), c"SID>".as_ptr(), 4) == 0
                || mb_strnicmp(p.add(1), c"SNR>".as_ptr(), 4) == 0)
        {
            return 5;
        }
        if *p == b's' as c_char && *p.add(1) == b':' as c_char {
            return 2;
        }
        0
    }
}

pub unsafe fn translated_function_exists(name: *const c_char) -> bool {
    unsafe {
        if builtin_function(name, -1) {
            return !find_internal_func(name).is_null();
        }
        !find_func(name).is_null()
    }
}

pub unsafe fn function_exists(name: *const c_char, no_deref: bool) -> bool {
    unsafe {
        let mut nm = name.cast_mut();
        let mut n = false;
        let mut flag = TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD;

        if no_deref {
            flag |= TFN_NO_DEREF;
        }
        let p = trans_function_name(&raw mut nm, false, flag, ptr::null_mut(), ptr::null_mut());
        nm = skipwhite(nm);

        if !p.is_null() && (*nm == NUL || *nm == b'(' as c_char) {
            n = translated_function_exists(p);
        }
        xfree(p.cast());
        n
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user defined
/// function names.
pub unsafe fn get_user_func_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            get_uf_done = 0;
            let ht = &raw const func_hashtab;
            let items_ptr = &raw mut get_uf_items;
            *items_ptr = (*ht).values().copied().collect();
            get_uf_changed = func_hashtab_changed;
        }
        let items_ptr = &raw const get_uf_items;
        if get_uf_changed == func_hashtab_changed && get_uf_done < (*items_ptr).len() {
            let fp = (*items_ptr).as_slice()[get_uf_done];
            get_uf_done += 1;

            if ((*fp).uf_flags & FC_DICT) != 0
                || strncmp(uf2hikey(fp), c"<lambda>".as_ptr(), 8) == 0
            {
                return c"".as_ptr().cast_mut();
            }

            if (*fp).uf_namelen + 4 >= IOSIZE {
                return uf2hikey(fp);
            }

            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            let mut len = cat_func_name(iobuff_ptr, IOSIZE, fp);
            if (*xp).xp_context != EXPAND_USER_FUNC {
                xstrlcpy(
                    iobuff_ptr.add(len as usize),
                    c"(".as_ptr(),
                    IOSIZE - len as usize,
                );
                if (*fp).uf_varargs == 0 && (*fp).uf_args.is_empty() {
                    len += 1;
                    xstrlcpy(
                        iobuff_ptr.add(len as usize),
                        c")".as_ptr(),
                        IOSIZE - len as usize,
                    );
                }
            }
            return iobuff_ptr;
        }
        ptr::null_mut()
    }
}

/// `:delfunction {name}`
pub unsafe fn ex_delfunction(eap: *mut exarg_T) {
    unsafe {
        let mut fp: *mut ufunc_T = ptr::null_mut();
        let mut fudi = funcdict_T::new();

        let mut p = (*eap).arg;
        let name = trans_function_name(
            &raw mut p,
            (*eap).skip != 0,
            0,
            &raw mut fudi,
            ptr::null_mut(),
        );
        if name.is_null() {
            if !fudi.fd_dict.is_null() && (*eap).skip == 0 {
                emsg(gettext(E_FUNCREF.as_ptr()));
            }
            return;
        }
        if ends_excmd(*skipwhite(p) as c_int) == 0 {
            xfree(name.cast());
            semsg!(gettext(e_trailing_arg.as_ptr()), p);
            return;
        }
        (*eap).nextcmd = check_nextcmd(p);
        if !(*eap).nextcmd.is_null() {
            *p = NUL;
        }

        if (*name as u8).is_ascii_digit() && fudi.fd_dict.is_null() {
            if (*eap).skip == 0 {
                semsg!(gettext(e_invarg2.as_ptr()), (*eap).arg);
            }
            xfree(name.cast());
            return;
        }
        if (*eap).skip == 0 {
            fp = find_func(name);
        }
        xfree(name.cast());

        if (*eap).skip == 0 {
            if fp.is_null() {
                if (*eap).forceit == 0 {
                    semsg!(gettext(E_NOFUNC.as_ptr()), (*eap).arg);
                }
                return;
            }
            if (*fp).uf_calls > 0 {
                semsg!(
                    gettext(c"E131: Cannot delete function %s: It is in use".as_ptr()),
                    (*eap).arg,
                );
                return;
            }
            if (*fp).uf_refcount > 2 {
                semsg!(
                    gettext(c"Cannot delete function %s: It is being used internally".as_ptr()),
                    (*eap).arg,
                );
                return;
            }

            if !fudi.fd_dict.is_null() {
                tv_dict_item_remove(fudi.fd_dict, fudi.fd_di);
            } else {
                let threshold = if func_name_refcount(uf2hikey(fp)) {
                    0
                } else {
                    1
                };
                if (*fp).uf_refcount > threshold {
                    if func_remove(fp) {
                        (*fp).uf_refcount -= 1;
                    }
                    (*fp).uf_flags |= FC_DELETED;
                } else {
                    func_clear_free(fp, false);
                }
            }
        }
    }
}

/// Unreference a Function: decrement the reference count and free it when it
/// becomes zero.
pub unsafe fn func_unref(name: *mut c_char) {
    unsafe {
        if name.is_null() || !func_name_refcount(name) {
            return;
        }

        let fp = find_func(name);
        if fp.is_null() && (*name as u8).is_ascii_digit() && !entered_free_all_mem {
            internal_error(c"func_unref()".as_ptr());
            std::process::abort();
        }
        func_ptr_unref(fp);
    }
}

pub unsafe fn func_ptr_unref(fp: *mut ufunc_T) {
    unsafe {
        if !fp.is_null() {
            (*fp).uf_refcount -= 1;
            if (*fp).uf_refcount <= 0 && (*fp).uf_calls == 0 {
                func_clear_free(fp, false);
            }
        }
    }
}

pub unsafe fn func_ref(name: *mut c_char) {
    unsafe {
        if name.is_null() || !func_name_refcount(name) {
            return;
        }
        let fp = find_func(name);
        if !fp.is_null() {
            (*fp).uf_refcount += 1;
        } else if (*name as u8).is_ascii_digit() {
            internal_error(c"func_ref()".as_ptr());
        }
    }
}

pub unsafe fn func_ptr_ref(fp: *mut ufunc_T) {
    unsafe {
        if !fp.is_null() {
            (*fp).uf_refcount += 1;
        }
    }
}

#[inline]
unsafe fn fc_referenced(fc: *const funccall_T) -> bool {
    unsafe {
        (*fc).fc_l_varlist.lv_refcount != DO_NOT_FREE_CNT
            || (*fc).fc_l_vars.dv_refcount != DO_NOT_FREE_CNT
            || (*fc).fc_l_avars.dv_refcount != DO_NOT_FREE_CNT
            || (*fc).fc_refcount > 0
    }
}

unsafe fn can_free_funccal(fc: *const funccall_T, copyID: c_int) -> bool {
    unsafe {
        (*fc).fc_l_varlist.lv_copyID != copyID
            && (*fc).fc_l_vars.dv_copyID != copyID
            && (*fc).fc_l_avars.dv_copyID != copyID
            && (*fc).fc_copyID != copyID
    }
}

/// `:return [expr]`
pub unsafe fn ex_return(eap: *mut exarg_T) {
    unsafe {
        let arg = (*eap).arg;
        let mut rettv: typval_T = typval_T::new();
        let mut returning = false;

        if current_funccal.is_null() {
            emsg(gettext(c"E133: :return not inside a function".as_ptr()));
            return;
        }

        let mut evalarg = evalarg_T {
            eval_flags: if (*eap).skip != 0 { 0 } else { EVAL_EVALUATE },
            eval_tofree: ptr::null_mut(),
        };

        if (*eap).skip != 0 {
            emsg_skip += 1;
        }

        (*eap).nextcmd = ptr::null_mut();
        if (*arg != NUL && *arg != b'|' as c_char && *arg != b'\n' as c_char)
            && eval0(arg, &raw mut rettv, eap, &raw mut evalarg) != FAIL
        {
            if (*eap).skip == 0 {
                returning = do_return(eap, false, true, Some(rettv));
            } else {
                tv_clear(&raw mut rettv);
            }
        } else if (*eap).skip == 0 {
            update_force_abort();

            if !aborting() {
                returning = do_return(eap, false, true, None);
            }
        }

        if returning {
            (*eap).nextcmd = ptr::null_mut();
        } else if (*eap).nextcmd.is_null() {
            (*eap).nextcmd = check_nextcmd(arg);
        }

        if (*eap).skip != 0 {
            emsg_skip -= 1;
        }
        clear_evalarg(&raw mut evalarg, eap);
    }
}

unsafe fn ex_call_inner(
    eap: *mut exarg_T,
    name: *mut c_char,
    arg: *mut *mut c_char,
    startarg: *mut c_char,
    funcexe_init: *const funcexe_T,
    evalarg: *mut evalarg_T,
) -> bool {
    unsafe {
        let mut doesrange = false;
        let mut failed = false;

        for lnum in (*eap).line1..=(*eap).line2 {
            if (*eap).addr_count > 0 {
                if lnum > (*curbuf).b_ml.ml_line_count {
                    emsg(gettext(e_invrange.as_ptr()));
                    break;
                }
                (*curwin).w_cursor.lnum = lnum;
                (*curwin).w_cursor.col = 0;
                (*curwin).w_cursor.coladd = 0;
            }
            *arg = startarg;

            let mut funcexe = *funcexe_init;
            funcexe.fe_doesrange = &raw mut doesrange;
            let mut rettv: typval_T = typval_T::new();
            rettv.v_type = VarType::VAR_UNKNOWN;
            if get_func_tv(name, -1, &raw mut rettv, arg, evalarg, &raw mut funcexe) == FAIL {
                failed = true;
                break;
            }

            if handle_subscript(
                arg.cast::<*const c_char>(),
                &raw mut rettv,
                &raw mut EVALARG_EVALUATE,
                true,
            ) == FAIL
            {
                failed = true;
                break;
            }

            tv_clear(&raw mut rettv);
            if doesrange {
                break;
            }

            if aborting() {
                break;
            }
        }

        failed
    }
}

unsafe fn ex_defer_inner(
    name: *mut c_char,
    arg: *mut *mut c_char,
    partial: *const partial_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        let mut argvars: [typval_T; MAX_FUNC_ARGS + 1] = [typval_T::new(); MAX_FUNC_ARGS + 1];
        let mut partial_argc = 0;
        let mut argcount = 0;

        if current_funccal.is_null() {
            semsg!(
                gettext(e_str_not_inside_function.as_ptr()),
                c"defer".as_ptr(),
            );
            return FAIL;
        }
        if !partial.is_null() {
            if !(*partial).pt_dict.is_null() {
                emsg(gettext(
                    E_CANNOT_USE_PARTIAL_WITH_DICTIONARY_FOR_DEFER.as_ptr(),
                ));
                return FAIL;
            }
            if !(*partial).pt_argv.is_empty() {
                partial_argc = (*partial).pt_argv.len() as c_int;
                for i in 0..partial_argc {
                    tv_copy(
                        &(&(*partial).pt_argv)[i as usize],
                        &raw mut argvars[i as usize],
                    );
                }
            }
        }
        let mut r = get_func_arguments(
            arg,
            evalarg,
            0,
            argvars.as_mut_ptr().add(partial_argc as usize),
            &raw mut argcount,
        );
        argcount += partial_argc;

        if r == OK {
            if builtin_function(name, -1) {
                let fdef = find_internal_func(name);
                if fdef.is_null() {
                    emsg_funcname(e_unknown_function_str.as_ptr(), name);
                    r = FAIL;
                } else if check_internal_func(fdef, argcount) == -1 {
                    r = FAIL;
                }
            } else {
                let ufunc = find_func(name);
                if !ufunc.is_null() {
                    let error = check_user_func_argcount(ufunc, argcount);
                    if error != FCERR_UNKNOWN {
                        user_func_error(error, name, false);
                        r = FAIL;
                    }
                }
            }
        }

        if r == FAIL {
            while argcount > 0 {
                argcount -= 1;
                tv_clear(&raw mut argvars[argcount as usize]);
            }
            return FAIL;
        }
        add_defer(name, argcount, argvars.as_mut_ptr());
        OK
    }
}

pub unsafe fn can_add_defer() -> bool {
    unsafe {
        if get_current_funccal().is_null() {
            semsg!(
                gettext(e_str_not_inside_function.as_ptr()),
                c"defer".as_ptr(),
            );
            return false;
        }
        true
    }
}

pub unsafe fn add_defer(name: *mut c_char, argcount_arg: c_int, argvars: *mut typval_T) {
    unsafe {
        let saved_name = xstrdup(name);
        let argcount = argcount_arg as usize;
        let mut dr_argvars = vec![typval_T::new(); argcount + 1];
        for (i, slot) in dr_argvars.iter_mut().enumerate().take(argcount) {
            *slot = *argvars.add(i);
        }
        let dr_name_cstr = CStr::from_ptr(saved_name).to_owned();
        xfree(saved_name.cast());
        (*current_funccal).fc_defer.push(defer_T {
            dr_name: Some(dr_name_cstr),
            dr_argvars,
        });
    }
}

unsafe fn handle_defer_one(funccal: *mut funccall_T) {
    unsafe {
        let mut idx = (*funccal).fc_defer.len() as isize - 1;
        while idx >= 0 {
            let dr = &mut (&mut (*funccal).fc_defer)[idx as usize];

            let Some(name_cstr) = dr.dr_name.take() else {
                idx -= 1;
                continue;
            };

            let mut funcexe = funcexe_T::new();
            funcexe.fe_evaluate = true;

            let mut rettv: typval_T = typval_T::new();
            rettv.v_type = VarType::VAR_UNKNOWN;

            let argcount = (dr.dr_argvars.len() - 1) as c_int;

            let mut estate = exception_state_T::new();
            exception_state_save(&raw mut estate);
            exception_state_clear();

            call_func(
                name_cstr.as_ptr(),
                -1,
                &raw mut rettv,
                argcount,
                dr.dr_argvars.as_mut_ptr(),
                &raw mut funcexe,
            );

            exception_state_restore(&raw mut estate);

            tv_clear(&raw mut rettv);
            drop(name_cstr);
            let mut i = argcount - 1;
            while i >= 0 {
                let dr = &mut (&mut (*funccal).fc_defer)[idx as usize];
                tv_clear(&raw mut dr.dr_argvars[i as usize]);
                i -= 1;
            }
            (&mut (*funccal).fc_defer)[idx as usize].dr_argvars.clear();
            idx -= 1;
        }
        (*funccal).fc_defer.clear();
    }
}

pub unsafe fn invoke_all_defer() {
    unsafe {
        let mut fc = current_funccal;
        while !fc.is_null() {
            handle_defer_one(fc);
            fc = (*fc).fc_caller;
        }

        let stack = &raw const funccal_stack;
        for &top in (*stack).iter().rev() {
            let mut fc2 = top;
            while !fc2.is_null() {
                handle_defer_one(fc2);
                fc2 = (*fc2).fc_caller;
            }
        }
    }
}

/// `:1,25call func(arg1, arg2)` or `:defer func(arg1, arg2)`.
pub unsafe fn ex_call(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut fudi = funcdict_T::new();
        let mut partial: *mut partial_T = ptr::null_mut();
        let mut evalarg = evalarg_T::new();

        fill_evalarg_from_eap(&raw mut evalarg, eap, (*eap).skip != 0);
        if (*eap).skip != 0 {
            let mut rettv: typval_T = typval_T::new();
            emsg_skip += 1;
            if eval0((*eap).arg, &raw mut rettv, eap, &raw mut evalarg) != FAIL {
                tv_clear(&raw mut rettv);
            }
            emsg_skip -= 1;
            clear_evalarg(&raw mut evalarg, eap);
            return;
        }

        let tofree = trans_function_name(
            &raw mut arg,
            false,
            TFN_INT,
            &raw mut fudi,
            &raw mut partial,
        );
        if let Some(ref newkey) = fudi.fd_newkey {
            semsg!(gettext(e_dictkey.as_ptr()), newkey.as_ptr());
        }
        if tofree.is_null() {
            return;
        }

        if !fudi.fd_dict.is_null() {
            (*fudi.fd_dict).dv_refcount += 1;
        }

        let mut len = strlen(tofree) as c_int;
        let mut found_var = false;
        let name = deref_func_name(
            tofree,
            &raw mut len,
            if !partial.is_null() {
                ptr::null_mut()
            } else {
                &raw mut partial
            },
            false,
            &raw mut found_var,
        );

        let startarg = skipwhite(arg);

        if *startarg != b'(' as c_char {
            semsg!(gettext(e_missingparen.as_ptr()), (*eap).arg);
            tv_dict_unref(fudi.fd_dict);
            xfree(tofree.cast());
            return;
        }

        let failed = if (*eap).cmdidx == cmdidx_T::CMD_defer as c_int {
            arg = startarg;
            ex_defer_inner(name, &raw mut arg, partial, &raw mut evalarg) == FAIL
        } else {
            let mut funcexe = funcexe_T::new();
            funcexe.fe_partial = partial;
            funcexe.fe_selfdict = fudi.fd_dict;
            funcexe.fe_firstline = (*eap).line1;
            funcexe.fe_lastline = (*eap).line2;
            funcexe.fe_found_var = found_var;
            funcexe.fe_evaluate = true;
            ex_call_inner(
                eap,
                name,
                &raw mut arg,
                startarg,
                &raw const funcexe,
                &raw mut evalarg,
            )
        };

        if (!aborting() || did_throw)
            && (!failed || (*(*eap).cstack.cast::<cstack_T>()).cs_trylevel > 0)
        {
            if ends_excmd(*arg as c_int) == 0 {
                if !failed && !aborting() {
                    emsg_severe = true;
                    semsg!(gettext(e_trailing_arg.as_ptr()), arg);
                }
            } else {
                (*eap).nextcmd = check_nextcmd(arg);
            }
        }
        clear_evalarg(&raw mut evalarg, eap);

        tv_dict_unref(fudi.fd_dict);
        xfree(tofree.cast());
    }
}

/// Return from a function. Possibly makes the return pending.
pub unsafe fn do_return(
    eap: *mut exarg_T,
    reanimate: bool,
    _is_cmd: bool,
    mut rettv: Option<typval_T>,
) -> bool {
    unsafe {
        let cstack = (*eap).cstack.cast::<cstack_T>();

        if reanimate {
            (*current_funccal).fc_returned = 0;
        }

        let idx = cleanup_conditionals(cstack, 0, 1);
        if idx >= 0 {
            (*cstack).cs_pending[idx as usize] = CSTP_RETURN as c_char;

            if reanimate {
                debug_assert!(!(*current_funccal).fc_rettv.is_null());
                rettv = Some(*(*current_funccal).fc_rettv);
                (*(*current_funccal).fc_rettv).v_type = VarType::VAR_NUMBER;
                (*(*current_funccal).fc_rettv).vval.v_number = 0;
            }

            (*cstack).cs_rettv[idx as usize] = rettv;
            report_make_pending(CSTP_RETURN, PendingVal::Return(rettv));
        } else {
            (*current_funccal).fc_returned = 1;

            if !reanimate && let Some(tv) = rettv {
                tv_clear((*current_funccal).fc_rettv);
                *(*current_funccal).fc_rettv = tv;
            }
        }

        idx < 0
    }
}

/// Generate a return command for producing the value of `rettv`.
pub unsafe fn get_return_cmd(rettv: Option<typval_T>) -> *mut c_char {
    unsafe {
        let mut s: *mut c_char = ptr::null_mut();
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut slen = 0usize;

        if let Some(mut tv) = rettv {
            s = encode_tv2echo(&raw mut tv, ptr::null_mut());
            tofree = s;
        }
        if s.is_null() {
            s = c"".as_ptr().cast_mut();
        } else {
            slen = strlen(s);
        }

        let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
        xstrlcpy(iobuff_ptr, c":return ".as_ptr(), IOSIZE);
        xstrlcpy(iobuff_ptr.add(8), s, IOSIZE - 8);
        let mut iobuff_len = 8 + slen;
        if iobuff_len >= IOSIZE {
            strcpy(iobuff_ptr.add(IOSIZE - 4), c"...".as_ptr());
            iobuff_len = IOSIZE - 1;
        }
        xfree(tofree.cast());
        xstrnsave(iobuff_ptr, iobuff_len)
    }
}

/// Get next function line.
pub unsafe fn get_func_line(
    _c: c_int,
    fcp: *mut funccall_T,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
        let fp = (*fcp).fc_func;
        let retval: *mut c_char;

        if (*fcp).fc_dbg_tick != debug_tick {
            (*fcp).fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(fp), sourcing_lnum());
            (*fcp).fc_dbg_tick = debug_tick;
        }
        if do_profiling == PROF_YES {
            func_line_end(fcp);
        }

        let lines = &(*fp).uf_lines;
        let lines_len = lines.len() as c_int;
        if (((*fp).uf_flags & FC_ABORT) != 0 && did_emsg != 0 && !aborted_in_try())
            || (*fcp).fc_returned != 0
        {
            retval = ptr::null_mut();
        } else {
            while (*fcp).fc_linenr < lines_len && lines[(*fcp).fc_linenr as usize].is_null() {
                (*fcp).fc_linenr += 1;
            }
            if (*fcp).fc_linenr >= lines_len {
                retval = ptr::null_mut();
            } else {
                let line = lines[(*fcp).fc_linenr as usize];
                (*fcp).fc_linenr += 1;
                retval = xstrdup(line);
                set_sourcing_lnum((*fcp).fc_linenr);
                if do_profiling == PROF_YES {
                    func_line_start(fcp);
                }
            }
        }

        let cur_lnum = sourcing_lnum();
        if (*fcp).fc_breakpoint != 0 && (*fcp).fc_breakpoint <= cur_lnum {
            dbg_breakpoint(uf2hikey(fp), cur_lnum);
            (*fcp).fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(fp), sourcing_lnum());
            (*fcp).fc_dbg_tick = debug_tick;
        }

        retval
    }
}

pub unsafe fn func_has_ended(fcp: *mut funccall_T) -> c_int {
    unsafe {
        if (((*(*fcp).fc_func).uf_flags & FC_ABORT) != 0 && did_emsg != 0 && !aborted_in_try())
            || (*fcp).fc_returned != 0
        {
            1
        } else {
            0
        }
    }
}

pub unsafe fn func_has_abort(fcp: *mut funccall_T) -> c_int {
    unsafe { (*(*fcp).fc_func).uf_flags & FC_ABORT }
}

/// Turn `dict.Func` into a partial for `Func` bound to `dict`.
pub unsafe fn make_partial(selfdict: *mut dict_T, rettv: *mut typval_T) {
    unsafe {
        let mut fp: *mut ufunc_T = ptr::null_mut();
        let mut fname_buf = [0 as c_char; FLEN_FIXED + 1];
        let mut error = 0;

        if (*rettv).v_type == VarType::VAR_PARTIAL
            && !(*rettv).vval.v_partial.is_null()
            && !(*(*rettv).vval.v_partial).pt_func.is_null()
        {
            fp = (*(*rettv).vval.v_partial).pt_func;
        } else {
            let mut fname =
                if (*rettv).v_type == VarType::VAR_FUNC || (*rettv).v_type == VarType::VAR_STRING {
                    (*rettv).vval.v_string
                } else if (*rettv).vval.v_partial.is_null() {
                    ptr::null_mut()
                } else {
                    (*(*rettv).vval.v_partial).pt_name
                };
            if fname.is_null() {
                (*rettv).v_type = VarType::VAR_FUNC;
                (*rettv).vval.v_string = ptr::null_mut();
            } else {
                let mut tofree: *mut c_char = ptr::null_mut();
                fname = fname_trans_sid(
                    fname,
                    fname_buf.as_mut_ptr(),
                    &raw mut tofree,
                    &raw mut error,
                );
                fp = find_func(fname);
                xfree(tofree.cast());
            }
        }

        if !fp.is_null() && ((*fp).uf_flags & FC_DICT) != 0 {
            let pt = Box::into_raw(Box::new(partial_T::default()));
            (*pt).pt_refcount = 1;
            (*pt).pt_dict = selfdict;
            (*selfdict).dv_refcount += 1;
            (*pt).pt_auto = true;
            if (*rettv).v_type == VarType::VAR_FUNC || (*rettv).v_type == VarType::VAR_STRING {
                (*pt).pt_name = (*rettv).vval.v_string;
            } else {
                let ret_pt = (*rettv).vval.v_partial;
                if !(*ret_pt).pt_name.is_null() {
                    (*pt).pt_name = xstrdup((*ret_pt).pt_name);
                    func_ref((*pt).pt_name);
                } else {
                    (*pt).pt_func = (*ret_pt).pt_func;
                    func_ptr_ref((*pt).pt_func);
                }
                if !(*ret_pt).pt_argv.is_empty() {
                    let pt_argc = (*ret_pt).pt_argv.len();
                    (*pt).pt_argv.resize(pt_argc, typval_T::new());
                    for i in 0..pt_argc {
                        tv_copy(&(&(*ret_pt).pt_argv)[i], &raw mut (&mut (*pt).pt_argv)[i]);
                    }
                }
                partial_unref(ret_pt);
            }
            (*rettv).v_type = VarType::VAR_PARTIAL;
            (*rettv).vval.v_partial = pt;
        }
    }
}

pub unsafe fn func_name(fcp: *mut funccall_T) -> *mut c_char {
    unsafe { uf2hikey((*fcp).fc_func) }
}

pub unsafe fn func_breakpoint(fcp: *mut funccall_T) -> *mut linenr_T {
    unsafe { &raw mut (*fcp).fc_breakpoint }
}

pub unsafe fn func_dbg_tick(fcp: *mut funccall_T) -> *mut c_int {
    unsafe { &raw mut (*fcp).fc_dbg_tick }
}

pub unsafe fn func_level(fcp: *mut funccall_T) -> c_int {
    unsafe { (*fcp).fc_level }
}

pub unsafe fn current_func_returned() -> c_int {
    unsafe { (*current_funccal).fc_returned }
}

pub unsafe fn free_unref_funccal(copyID: c_int, testing: c_int) -> bool {
    unsafe {
        let mut did_free = false;
        let mut did_free_funccal = false;

        let pfc = &raw mut previous_funccal;
        let mut i = 0usize;
        while i < (*pfc).len() {
            let fc = (*pfc).as_slice()[i].0;
            if can_free_funccal(fc, copyID) {
                (*pfc).remove(i);
                did_free = true;
                did_free_funccal = true;
            } else {
                i += 1;
            }
        }
        if did_free_funccal {
            garbage_collect(testing != 0);
        }
        did_free
    }
}

pub unsafe fn get_funccal() -> *mut funccall_T {
    unsafe {
        let mut funccal = current_funccal;
        if debug_backtrace_level > 0 {
            for i in 0..debug_backtrace_level {
                let temp_funccal = (*funccal).fc_caller;
                if !temp_funccal.is_null() {
                    funccal = temp_funccal;
                } else {
                    debug_backtrace_level = i;
                    break;
                }
            }
        }
        funccal
    }
}

unsafe fn get_funccal_for_vars() -> *mut funccall_T {
    unsafe {
        if current_funccal.is_null() {
            return ptr::null_mut();
        }
        let funccal = get_funccal();
        if funccal.is_null() || (*funccal).fc_l_vars.dv_refcount == 0 {
            return ptr::null_mut();
        }
        funccal
    }
}

pub unsafe fn get_funccal_local_dict() -> *mut dict_T {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            &raw mut (*funccal).fc_l_vars
        }
    }
}

pub unsafe fn get_funccal_local_ht() -> *mut hashtab_T {
    unsafe {
        let d = get_funccal_local_dict();
        if !d.is_null() {
            &raw mut (*d).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

pub unsafe fn get_funccal_local_var() -> *mut dictitem_T {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            (&raw mut (*funccal).fc_l_vars_var).cast()
        }
    }
}

pub unsafe fn get_funccal_args_dict() -> *mut dict_T {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            &raw mut (*funccal).fc_l_avars
        }
    }
}

pub unsafe fn get_funccal_args_ht() -> *mut hashtab_T {
    unsafe {
        let d = get_funccal_args_dict();
        if !d.is_null() {
            &raw mut (*d).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

pub unsafe fn get_funccal_args_var() -> *mut dictitem_T {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            (&raw mut (*funccal).fc_l_avars_var).cast()
        }
    }
}

pub unsafe fn list_func_vars(first: *mut c_int) {
    unsafe {
        if !current_funccal.is_null() && (*current_funccal).fc_l_vars.dv_refcount > 0 {
            list_hashtable_vars(
                &raw mut (*current_funccal).fc_l_vars.dv_hashtab,
                c"l:".as_ptr(),
                0,
                first,
            );
        }
    }
}

pub unsafe fn get_current_funccal_dict(ht: *mut hashtab_T) -> *mut dict_T {
    unsafe {
        if !current_funccal.is_null() && ht == &raw mut (*current_funccal).fc_l_vars.dv_hashtab {
            return &raw mut (*current_funccal).fc_l_vars;
        }
        ptr::null_mut()
    }
}

pub unsafe fn find_hi_in_scoped_ht(
    name: *const c_char,
    pht: *mut *mut hashtab_T,
) -> *mut hashitem_T {
    unsafe {
        if current_funccal.is_null() || (*(*current_funccal).fc_func).uf_scoped.is_null() {
            return ptr::null_mut();
        }

        let old_current_funccal = current_funccal;
        let mut hi: *mut hashitem_T = ptr::null_mut();
        let namelen = strlen(name);
        let mut varname: *const c_char = ptr::null();

        current_funccal = (*(*current_funccal).fc_func).uf_scoped;
        while !current_funccal.is_null() {
            let ht = find_var_ht(name, namelen, &raw mut varname);
            if !ht.is_null() && *varname != NUL {
                hi = hash_find_len(ht, varname, namelen - varname.offset_from(name) as usize);
                if !hashitem_empty(&*hi) {
                    *pht = ht;
                    break;
                }
            }
            if current_funccal == (*(*current_funccal).fc_func).uf_scoped {
                break;
            }
            current_funccal = (*(*current_funccal).fc_func).uf_scoped;
        }
        current_funccal = old_current_funccal;

        hi
    }
}

pub unsafe fn find_var_in_scoped_ht(
    name: *const c_char,
    namelen: usize,
    no_autoload: c_int,
) -> *mut dictitem_T {
    unsafe {
        if current_funccal.is_null() || (*(*current_funccal).fc_func).uf_scoped.is_null() {
            return ptr::null_mut();
        }

        let mut v: *mut dictitem_T = ptr::null_mut();
        let old_current_funccal = current_funccal;
        let mut varname: *const c_char = ptr::null();

        current_funccal = (*(*current_funccal).fc_func).uf_scoped;
        while !current_funccal.is_null() {
            let ht = find_var_ht(name, namelen, &raw mut varname);
            if !ht.is_null() && *varname != NUL {
                v = find_var_in_ht(
                    ht,
                    *name as c_int,
                    varname,
                    namelen - varname.offset_from(name) as usize,
                    no_autoload,
                );
                if !v.is_null() {
                    break;
                }
            }
            if current_funccal == (*(*current_funccal).fc_func).uf_scoped {
                break;
            }
            current_funccal = (*(*current_funccal).fc_func).uf_scoped;
        }
        current_funccal = old_current_funccal;

        v
    }
}

pub unsafe fn set_ref_in_previous_funccal(copyID: c_int) -> bool {
    unsafe {
        let pfc = &raw const previous_funccal;
        for item in (*pfc).as_slice() {
            let fc = item.0;
            (*fc).fc_copyID = copyID + 1;
            if set_ref_in_ht(
                &raw mut (*fc).fc_l_vars.dv_hashtab,
                copyID + 1,
                ptr::null_mut(),
            ) || set_ref_in_ht(
                &raw mut (*fc).fc_l_avars.dv_hashtab,
                copyID + 1,
                ptr::null_mut(),
            ) || set_ref_in_list_items(&raw mut (*fc).fc_l_varlist, copyID + 1, ptr::null_mut())
            {
                return true;
            }
        }
        false
    }
}

unsafe fn set_ref_in_funccal(fc: *mut funccall_T, copyID: c_int) -> bool {
    unsafe {
        if (*fc).fc_copyID != copyID {
            (*fc).fc_copyID = copyID;
            if set_ref_in_ht(&raw mut (*fc).fc_l_vars.dv_hashtab, copyID, ptr::null_mut())
                || set_ref_in_ht(
                    &raw mut (*fc).fc_l_avars.dv_hashtab,
                    copyID,
                    ptr::null_mut(),
                )
                || set_ref_in_list_items(&raw mut (*fc).fc_l_varlist, copyID, ptr::null_mut())
                || set_ref_in_func(ptr::null_mut(), (*fc).fc_func, copyID)
            {
                return true;
            }
        }
        false
    }
}

pub unsafe fn set_ref_in_call_stack(copyID: c_int) -> bool {
    unsafe {
        let mut fc = current_funccal;
        while !fc.is_null() {
            if set_ref_in_funccal(fc, copyID) {
                return true;
            }
            fc = (*fc).fc_caller;
        }

        let stack = &raw const funccal_stack;
        for &top in (*stack).iter().rev() {
            let mut fc2 = top;
            while !fc2.is_null() {
                if set_ref_in_funccal(fc2, copyID) {
                    return true;
                }
                fc2 = (*fc2).fc_caller;
            }
        }

        false
    }
}

pub unsafe fn set_ref_in_functions(copyID: c_int) -> bool {
    unsafe {
        let ht = &raw const func_hashtab;
        for &fp in (*ht).values() {
            if got_int {
                break;
            }
            if !func_name_refcount(uf2hikey(fp)) && set_ref_in_func(ptr::null_mut(), fp, copyID) {
                return true;
            }
        }
        false
    }
}

pub unsafe fn set_ref_in_func_args(copyID: c_int) -> bool {
    unsafe {
        let fa = &raw const funcargs;
        for &tv in &*fa {
            if set_ref_in_item(tv, copyID, ptr::null_mut(), ptr::null_mut()) {
                return true;
            }
        }
        false
    }
}

pub unsafe fn set_ref_in_func(name: *mut c_char, fp_in: *mut ufunc_T, copyID: c_int) -> bool {
    unsafe {
        let mut fp = fp_in;
        let mut error = FCERR_NONE;
        let mut fname_buf = [0 as c_char; FLEN_FIXED + 1];
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut abort = false;
        if name.is_null() && fp_in.is_null() {
            return false;
        }

        if fp_in.is_null() {
            let fname = fname_trans_sid(
                name,
                fname_buf.as_mut_ptr(),
                &raw mut tofree,
                &raw mut error,
            );
            fp = find_func(fname);
        }
        if !fp.is_null() {
            let mut fc = (*fp).uf_scoped;
            while !fc.is_null() {
                abort = abort || set_ref_in_funccal(fc, copyID);
                fc = (*(*fc).fc_func).uf_scoped;
            }
        }
        xfree(tofree.cast());
        abort
    }
}

/// Registers a luaref as a lambda.
pub unsafe fn register_luafunc(ref_: LuaRef) -> *mut c_char {
    unsafe {
        let name = get_lambda_name();
        let fp = alloc_ufunc(name.data, name.size);

        (*fp).uf_refcount = 1;
        (*fp).uf_varargs = 1;
        (*fp).uf_flags = FC_LUAREF;
        (*fp).uf_calls = 0;
        (*fp).uf_script_ctx = current_sctx;
        (*fp).uf_luaref = ref_;

        func_hashtab_add(fp);

        uf2hikey(fp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_function_and_func_name_helpers_work() {
        unsafe {
            assert!(eval_fname_sid(c"s:Foo".as_ptr()));
            assert!(eval_fname_sid(c"<SID>Foo".as_ptr()));
            assert!(eval_fname_sid(c"<sid>Foo".as_ptr()));
            assert!(!eval_fname_sid(c"<SNR>12_Foo".as_ptr()));
            assert!(func_name_refcount(c"123".as_ptr()));
            assert!(func_name_refcount(c"<lambda>1".as_ptr()));
            assert!(!func_name_refcount(c"MyFunc".as_ptr()));
            assert!(builtin_function(c"abs".as_ptr(), -1));
            assert!(!builtin_function(c"s:abs".as_ptr(), -1));
            assert!(!builtin_function(c"foo#bar".as_ptr(), -1));
            assert!(!builtin_function(c"Foo".as_ptr(), -1));
        }
    }
}
