//! User defined function support (`src/nvim/eval/userfunc.c`).

use crate::api::private::validate::NvimString;
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_FUNCUNDEFINED, PROF_YES, apply_autocmds,
};
use crate::charset::{
    NUL, ascii_isalnum, ascii_islower, ascii_iswhite, getdigits, skiptowhite, skipwhite,
    vim_strsize,
};
use crate::cmdexpand::{Cmdidx, EXPAND_USER_FUNC, K_UICMDLINE};
use crate::cursor::Linenr;
use crate::debugger::{dbg_breakpoint, dbg_find_breakpoint, has_profiling};
use crate::errors::*;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::funcs::{
    MAX_FUNC_ARGS, TFN_INT, TFN_NO_DEREF, call_internal_func, call_internal_method,
    check_internal_func, find_internal_func,
};
use crate::eval::typval::{
    Callback, Dictitem, EvalFuncData, Listitem, TV_CSTRING, TvDict, TvPartial, TypVal,
    VAR_UNLOCKED, VarType, Varnumber, tv_clear, tv_copy, tv_dict_add, tv_dict_hi2di,
    tv_dict_item_alloc, tv_dict_item_alloc_len, tv_dict_item_remove, tv_dict_unref,
    tv_get_number_chk, tv_is_func, tv_list_append, tv_list_init_static, tv_list_set_lock,
    value_check_lock,
};
use crate::eval::vars::{
    Evalarg, Lval, VV_TESTING, find_var, find_var_ht, find_var_in_ht, get_vim_var_nr,
    init_var_dict, list_hashtable_vars, skip_var_list, vars_clear, vars_clear_ext,
};
use crate::eval::{
    AUTOLOAD_CHAR, EVAL_EVALUATE, FNE_CHECK_START, FNE_INCL_BR, GLV_READ_ONLY, TFN_NO_AUTOLOAD,
    TFN_QUIET, VAR_DEF_SCOPE, callback_call, check_luafunc_name, eval_isnamec, eval_isnamec1,
    eval_st, eval0, eval1, fill_evalarg_from_eap, find_name_end, garbage_collect, get_id_len,
    get_lval, handle_subscript, is_luafunc, last_set_msg, partial_name, partial_unref,
    set_ref_in_ht, set_ref_in_item, set_ref_in_list_items, skip_expr,
};
use crate::ex_docmd::{Exarg, check_nextcmd, checkforcmd, do_cmdline, ends_excmd, skip_range};
use crate::ex_eval::{
    CSTP_RETURN, Cstack, ExceptionState, PendingVal, aborted_in_try, aborting,
    cleanup_conditionals, exception_state_clear, exception_state_restore, exception_state_save,
    report_make_pending, update_force_abort,
};
use crate::ex_getln::{getcmdline, ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave};
use crate::globals::*;
use crate::hashtab::{Hashitem, Hashtab, hash_add, hash_find_len, hashitem_empty};
use crate::input::{RedoState, restore_redobuff, save_redobuff};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_SPECIAL, KE_SNR, KS_EXTRA};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{
    api_free_luaref, nlua_call_vlua, nlua_exec_typval_callable, nlua_set_sctx,
};
use crate::math::{FAIL, OK};
use crate::mbyte::mb_strnicmp;
use crate::memory::{cstr_from_bytes, cstr_to_cstring, mem_st, xstrlcpy};
use crate::message::{
    emsg, iemsg, internal_error, message_filtered, msg_clr_eos, msg_ext_set_kind, msg_outnum,
    msg_prt_line, msg_putchar, msg_puts, msg_start, semsg, smsg, swmsg, trunc_string,
    verbose_enter_scroll, verbose_leave_scroll,
};
use crate::option_vars::*;
use crate::os::env::ExpandCtx;
use crate::os::input::line_breakcheck;
use crate::path::{K_PATH_CMP_LITERAL, path_equal};
use crate::profile::{
    Funccall, Proftime, func_do_profile, func_line_end, func_line_start, prof_def_func,
    profile_add, profile_end, profile_self, profile_start, profile_sub_wait, profile_zero,
    script_prof_restore, script_prof_save,
};
use crate::regexp::{
    NUMBUFLEN, RE_MAGIC, Regmatch, skip_regexp, vim_regcomp, vim_regexec, vim_regfree,
};
use crate::runtime::{
    autoload_name, estack_pop, estack_push_ufunc, get_sourced_lnum, rt_st, script_autoload,
    set_sourcing_lnum, sourcing_lnum, sourcing_name,
};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::ui::ui_has;
use std::ffi::CStr;
use std::ptr;

pub type VimLFunc = Option<fn(&mut [TypVal], &mut TypVal, EvalFuncData)>;

#[repr(C)]
pub struct UserFunc {
    pub uf_varargs: i32,
    pub uf_flags: i32,
    pub uf_calls: i32,
    pub uf_cleared: bool,
    pub uf_args: Vec<std::ffi::CString>,
    pub uf_def_args: Vec<std::ffi::CString>,
    pub uf_lines: Vec<Option<std::ffi::CString>>,
    pub uf_profiling: i32,
    pub uf_prof_initialized: i32,
    pub uf_luaref: i32,
    pub uf_tm_count: i32,
    pub uf_tm_total: u64,
    pub uf_tm_self: u64,
    pub uf_tm_children: u64,
    pub uf_tml_count: Vec<i32>,
    pub uf_tml_total: Vec<u64>,
    pub uf_tml_self: Vec<u64>,
    pub uf_tml_start: u64,
    pub uf_tml_children: u64,
    pub uf_tml_wait: u64,
    pub uf_tml_idx: i32,
    pub uf_tml_execed: i32,
    pub uf_script_ctx: crate::runtime::Sctx,
    pub uf_refcount: i32,
    pub uf_scoped: *mut crate::profile::Funccall,
    pub uf_name_exp: Option<std::ffi::CString>,
    pub uf_namelen: usize,
    pub uf_name: std::ffi::CString,
}
use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO};
use crate::buffer::{DO_NOT_FREE_CNT, NOTDONE, VAR_FIXED, VAR_SCOPE};
use crate::globals::IOSIZE;

pub const FC_ABORT: i32 = 0x01;
pub const FC_RANGE: i32 = 0x02;
pub const FC_DICT: i32 = 0x04;
pub const FC_CLOSURE: i32 = 0x08;
pub const FC_DELETED: i32 = 0x10;
pub const FC_REMOVED: i32 = 0x20;
pub const FC_SANDBOX: i32 = 0x40;
pub const FC_NOARGS: i32 = 0x200;
pub const FC_LUAREF: i32 = 0x800;

pub const FCERR_UNKNOWN: i32 = 0;
pub const FCERR_TOOMANY: i32 = 1;
pub const FCERR_TOOFEW: i32 = 2;
pub const FCERR_SCRIPT: i32 = 3;
pub const FCERR_DICT: i32 = 4;
pub const FCERR_NONE: i32 = 5;
pub const FCERR_OTHER: i32 = 6;
pub const FCERR_DELETED: i32 = 7;
pub const FCERR_NOTMETHOD: i32 = 8;

const FLEN_FIXED: usize = 40;
const MAX_FUNC_NESTING: i32 = 50;

pub(crate) const MSG_BUF_LEN: usize = 480;
const MSG_BUF_CLEN: i32 = (MSG_BUF_LEN / 6) as i32;

#[repr(C)]
#[derive(Clone)]
pub struct Funcdict {
    pub fd_dict: *mut TvDict,
    pub fd_newkey: Option<std::ffi::CString>,
    pub fd_di: *mut Dictitem,
}

impl Funcdict {
    pub const fn new() -> Self {
        Self {
            fd_dict: ptr::null_mut(),
            fd_newkey: None,
            fd_di: ptr::null_mut(),
        }
    }
}

impl Default for Funcdict {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Funcexe {
    pub fe_fill_submatch: bool,
    pub fe_firstline: Linenr,
    pub fe_lastline: Linenr,
    pub fe_doesrange: bool,
    pub fe_evaluate: bool,
    pub fe_partial: *mut TvPartial,
    pub fe_selfdict: *mut TvDict,
    pub fe_basetv: *mut TypVal,
    pub fe_found_var: bool,
}

impl Funcexe {
    pub const fn new() -> Self {
        Self {
            fe_fill_submatch: false,
            fe_firstline: 0,
            fe_lastline: 0,
            fe_doesrange: false,
            fe_evaluate: false,
            fe_partial: ptr::null_mut(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        }
    }
}

impl Default for Funcexe {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct EvalFuncDef {
    pub name: Option<&'static std::ffi::CStr>,
    pub min_argc: u8,
    pub max_argc: u8,
    pub base_arg: u8,
    pub fast: bool,
    pub func: VimLFunc,
    pub data: EvalFuncData,
}

#[repr(C)]
pub(crate) struct DeferCall {
    pub(crate) dr_name: Option<std::ffi::CString>,
    pub(crate) dr_argvars: Vec<TypVal>,
}

impl Drop for DeferCall {
    fn drop(&mut self) {
        for tv in self.dr_argvars.iter_mut().rev() {
            tv_clear(tv);
        }
    }
}

struct RetainedFunccall(Box<Funccall>);

struct UserfuncState {
    func_hashtab: std::collections::BTreeMap<Vec<u8>, *mut UserFunc>,
    func_hashtab_changed: i32,
    funcargs: Vec<*mut TypVal>,
    current_funccal: *mut Funccall,
    previous_funccal: Vec<RetainedFunccall>,
    funccal_stack: Vec<*mut Funccall>,
    lambda_name: [u8; 8 + NUMBUFLEN],
    lambda_no: i32,
    made_copy: i32,
    call_depth: i32,
    func_nr: i32,
    get_uf_done: usize,
    get_uf_changed: i32,
    get_uf_items: Vec<*mut UserFunc>,
}

impl UserfuncState {
    const fn new() -> Self {
        Self {
            func_hashtab: std::collections::BTreeMap::new(),
            func_hashtab_changed: 0,
            funcargs: Vec::new(),
            current_funccal: ptr::null_mut(),
            previous_funccal: Vec::new(),
            funccal_stack: Vec::new(),
            lambda_name: [0; 8 + NUMBUFLEN],
            lambda_no: 0,
            made_copy: 0,
            call_depth: 0,
            func_nr: 0,
            get_uf_done: 0,
            get_uf_changed: 0,
            get_uf_items: Vec::new(),
        }
    }
}

static UF_STATE: GlobalState<UserfuncState> = GlobalState::new(UserfuncState::new());

#[inline]
fn uf_st() -> &'static mut UserfuncState {
    unsafe { &mut *UF_STATE.as_ptr() }
}

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
pub const fn ascii_iswhite_nl_or_nul(c: i32) -> bool {
    matches!(c as u8, b' ' | b'\t' | b'\n' | b'\r' | 0)
}

use crate::charset::toupper_asc;

#[inline]
fn uf2hikey(fp: &UserFunc) -> &CStr {
    fp.uf_name.as_c_str()
}

fn func_hashtab_add(fp: &mut UserFunc) -> bool {
    let key_cstr = uf2hikey(fp);
    let key = key_cstr.to_bytes();
    if uf_st().func_hashtab.contains_key(key) {
        crate::siemsg!(c"Duplicate key in hash table: \"%s\"", key_cstr.as_ptr());
        return false;
    }
    uf_st().func_hashtab.insert(key.to_vec(), fp);
    uf_st().func_hashtab_changed = uf_st().func_hashtab_changed.wrapping_add(1);
    true
}

#[inline]
fn funcarg(fp: &UserFunc, j: i32) -> &std::ffi::CStr {
    (&fp.uf_args)[j as usize].as_c_str()
}

/// Return the function hash table.
pub fn func_tbl_get() -> &'static mut std::collections::BTreeMap<Vec<u8>, *mut UserFunc> {
    &mut uf_st().func_hashtab
}

/// Get one function argument.
/// Return a pointer to after the type.
/// When something is wrong return "arg".
fn one_function_arg<T>(
    arg: *mut T,
    newargs: Option<&mut Vec<std::ffi::CString>>,
    skip: bool,
) -> *mut T {
    let arg = arg.cast::<u8>();
    {
        let mut p = arg;

        while ascii_isalnum(crate::memory::cstr_first_char(p.cast()))
            || crate::memory::cstr_first_byte(p.cast()) == b'_'
        {
            p = p.wrapping_add(1);
        }
        let len = crate::memory::ptr_offset_from(p, arg);
        if arg == p
            || (crate::memory::cstr_first_byte(arg.cast())).is_ascii_digit()
            || (len == 9
                && (crate::memory::cstr_prefix_bytes(arg.cast(), 9) == &b"firstline"[..9]))
            || (len == 8 && (crate::memory::cstr_prefix_bytes(arg.cast(), 8) == &b"lastline"[..8]))
        {
            if !skip {
                semsg!(
                    gettext(c"E125: Illegal argument: %s"),
                    arg.cast::<std::ffi::c_void>()
                );
            }
            return arg.cast();
        }

        if let Some(newargs) = newargs {
            let arg_slice = crate::memory::cstr_slice(arg.cast(), len as usize);
            let arg_copy = std::ffi::CString::new(arg_slice).unwrap();

            // Check for duplicate argument name.
            for existing in newargs.iter() {
                if existing.as_bytes() == arg_copy.as_bytes() {
                    semsg!(
                        gettext(c"E853: Duplicate argument name: %s"),
                        arg_copy.as_ptr(),
                    );
                    return arg.cast();
                }
            }
            newargs.push(arg_copy);
        }

        p.cast()
    }
}

/// Get function arguments.
fn get_function_args<T>(
    argp: *mut *mut T,
    endchar: u8,
    mut newargs: Option<&mut Vec<std::ffi::CString>>,
    mut varargs: Option<&mut i32>,
    mut default_args: Option<&mut Vec<std::ffi::CString>>,
    skip: bool,
) -> bool {
    let mut mustend = false;
    let mut arg = unsafe { (*argp).cast::<u8>() };
    let mut p = arg;

    if let Some(ref mut p) = varargs {
        **p = 0;
    }

    let mut any_default = false;
    while unsafe { *p } != endchar {
        if crate::memory::cstr_first_byte(p.cast()) == b'.'
            && crate::memory::cstr_byte_at(p.cast(), 1) == b'.'
            && crate::memory::cstr_byte_at(p.cast(), 2) == b'.'
        {
            if let Some(ref mut p) = varargs {
                **p = 1;
            }
            p = p.wrapping_add(3);
            mustend = true;
        } else {
            arg = p;
            p = one_function_arg(p, newargs.as_deref_mut(), skip);
            if p == arg {
                break;
            }

            if crate::memory::cstr_first_byte(skipwhite(p.cast())) == b'='
                && default_args.is_some()
            {
                let mut rettv: TypVal = TypVal::new();

                any_default = true;
                p = skipwhite(p.cast()).wrapping_add(1).cast();
                p = skipwhite(p.cast()).cast();
                let expr = p;
                if eval1((&raw mut p).cast(), &mut rettv, None) {
                    while p > expr && ascii_iswhite(unsafe { *p.wrapping_sub(1) } as i32) {
                        p = p.wrapping_sub(1);
                    }
                    if let Some(default_args) = default_args.as_deref_mut() {
                        let expr_slice = crate::memory::cstr_slice(
                            expr.cast(),
                            crate::memory::ptr_offset_from(p, expr) as usize,
                        );
                        default_args.push(std::ffi::CString::new(expr_slice).unwrap());
                    }
                } else {
                    mustend = true;
                }
            } else if any_default {
                emsg(gettext(
                    c"E989: Non-default argument follows default argument",
                ));
                mustend = true;
            }

            if ascii_iswhite(crate::memory::cstr_first_char(p.cast()))
                && crate::memory::cstr_first_byte(skipwhite(p.cast())) == b','
            {
                if !skip {
                    semsg!(
                        gettext(E_NO_WHITE_SPACE_ALLOWED_BEFORE_STR_STR),
                        c",",
                        p.cast::<std::ffi::c_void>(),
                    );
                    if let Some(newargs) = newargs.as_deref_mut() {
                        newargs.clear();
                    }
                    if let Some(default_args) = default_args.as_deref_mut() {
                        default_args.clear();
                    }
                    return false;
                }
                p = skipwhite(p.cast()).cast();
            }
            if crate::memory::cstr_first_byte(p.cast()) == b',' {
                p = p.wrapping_add(1);
            } else {
                mustend = true;
            }
        }
        p = skipwhite(p.cast()).cast();
        if mustend && unsafe { *p } != endchar {
            if !skip {
                semsg!(gettext(E_INVARG2), unsafe { *argp });
            }
            break;
        }
    }
    if unsafe { *p } != endchar {
        if let Some(newargs) = newargs {
            newargs.clear();
        }
        if let Some(default_args) = default_args {
            default_args.clear();
        }
        return false;
    }
    p = p.wrapping_add(1); // skip "endchar"

    unsafe {
        *argp = p.cast();
    }
    true
}

/// Register function "fp" as using "current_funccal" as its scope.
fn register_closure(fp: &mut UserFunc) {
    if fp.uf_scoped == uf_st().current_funccal {
        return;
    }
    funccal_unref(fp.uf_scoped, fp, false);
    fp.uf_scoped = uf_st().current_funccal;
    cur_funccal_mut().fc_refcount += 1;
    cur_funccal_mut().fc_ufuncs.push(fp);
}

/// Return a name for a lambda in static memory.
fn get_lambda_name() -> NvimString {
    uf_st().lambda_no += 1;
    let buf_ptr = uf_st().lambda_name.as_mut_ptr();
    let buf_size = uf_st().lambda_name.len();
    let n = vim_snprintf!(buf_ptr.cast(), buf_size, c"<lambda>%d", uf_st().lambda_no);
    let size = if n < 1 {
        0
    } else {
        (n as usize).min(buf_size - 1)
    };
    NvimString {
        data: buf_ptr.cast(),
        size,
    }
}

/// Allocate a `UserFunc` for a function called `name`.
fn alloc_ufunc(name: impl crate::strings::IntoCCharPtr, namelen: usize) -> &'static mut UserFunc {
    let name = name.into_c_char_ptr();
    {
        let name_cstr = cstr_from_bytes(crate::memory::cstr_slice(name, namelen));
        let fp = Box::into_raw(Box::new(UserFunc {
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
            uf_script_ctx: crate::runtime::Sctx::new(),
            uf_refcount: 0,
            uf_scoped: ptr::null_mut(),
            uf_name_exp: None,
            uf_namelen: namelen,
            uf_name: name_cstr,
        }));

        if crate::memory::cstr_first_byte(name) == K_SPECIAL {
            let mut bytes = Vec::with_capacity(namelen + 2);
            bytes.extend_from_slice(b"<SNR>");
            bytes.extend_from_slice(&uf2hikey(unsafe { &*fp }).to_bytes()[3..]);
            unsafe {
                (*fp).uf_name_exp = Some(std::ffi::CString::new(bytes).unwrap());
            }
        }

        unsafe { &mut *fp }
    }
}

/// Parse a lambda expression and get a Funcref from `*arg`.
pub fn get_lambda_tv<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    evalarg: Option<&mut Evalarg>,
) -> i32 {
    unsafe {
        let arg_u8 = arg.cast::<*mut u8>();
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut newargs: Vec<std::ffi::CString> = Vec::new();
        let mut varargs: i32 = 0;
        let old_eval_lavars = eval_st().eval_lavars_used;
        let mut eval_lavars = false;

        let mut s: *mut u8 = skipwhite((*arg_u8).wrapping_add(1).cast()).cast();
        let mut ret = get_function_args(&raw mut s, b'-', None, None, None, true);
        if !ret || crate::memory::cstr_first_byte(s.cast()) != b'>' {
            return NOTDONE;
        }

        *arg_u8 = skipwhite((*arg_u8).wrapping_add(1).cast()).cast();
        ret = get_function_args(
            arg_u8,
            b'-',
            evaluate.then_some(&mut newargs),
            Some(&mut varargs),
            None,
            false,
        );
        if !ret || **arg_u8 != b'>' {
            eval_st().eval_lavars_used = old_eval_lavars;
            return FAIL;
        }

        if evaluate {
            eval_st().eval_lavars_used = &raw mut eval_lavars;
        }

        *arg_u8 = skipwhite((*arg_u8).wrapping_add(1).cast()).cast();
        let start = *arg_u8;
        ret = skip_expr(arg.cast(), evalarg);
        let end = *arg_u8;
        if !ret {
            eval_st().eval_lavars_used = old_eval_lavars;
            return FAIL;
        }

        *arg_u8 = skipwhite((*arg_u8).cast()).cast();
        if **arg_u8 != b'}' {
            semsg!(gettext(c"E451: Expected }: %s"), *arg_u8);
            eval_st().eval_lavars_used = old_eval_lavars;
            return FAIL;
        }
        *arg_u8 = (*arg_u8).wrapping_add(1);

        if evaluate {
            let mut flags = 0;

            let name = get_lambda_name();
            let fp = alloc_ufunc(name.data, name.size);
            let pt = Box::leak(Box::new(TvPartial::default()));

            let expr_len = crate::memory::ptr_offset_from(end, start) as usize;
            let mut line_bytes = Vec::with_capacity(7 + expr_len);
            line_bytes.extend_from_slice(b"return ");
            line_bytes.extend_from_slice(crate::memory::cstr_slice(start.cast(), expr_len));
            let line_cstr = std::ffi::CString::new(line_bytes).unwrap();
            if !line_cstr.to_bytes()[7..].windows(2).any(|w| w == b"a:") {
                flags |= FC_NOARGS;
            }
            let newlines = vec![Some(line_cstr)];

            let newlines_len = newlines.len() as i32;
            fp.uf_refcount = 1;
            func_hashtab_add(&mut *fp);
            fp.uf_args = newargs;
            fp.uf_def_args = Vec::new();
            fp.uf_lines = newlines;
            if !uf_st().current_funccal.is_null() && eval_lavars {
                flags |= FC_CLOSURE;
                register_closure(&mut *fp);
            } else {
                fp.uf_scoped = ptr::null_mut();
            }

            if prof_def_func() {
                func_do_profile(fp);
            }
            if g_st().sandbox != 0 {
                flags |= FC_SANDBOX;
            }
            fp.uf_varargs = 1;
            fp.uf_flags = flags;
            fp.uf_calls = 0;
            fp.uf_script_ctx = g_st().current_sctx;
            fp.uf_script_ctx.sc_lnum += sourcing_lnum() - newlines_len;

            pt.pt_func = fp;
            pt.pt_refcount = 1;
            rettv.vval = crate::eval::typval::TypvalVval::Partial(pt);
            rettv.v_type = VarType::VarPartial;
        }

        eval_st().eval_lavars_used = old_eval_lavars;
        OK
    }
}

/// Return name of the function corresponding to `name`.
pub fn deref_func_name<T>(
    name: *const T,
    lenp: &mut i32,
    partialp: *mut *mut TvPartial,
    no_autoload: bool,
    found_var: Option<&mut bool>,
) -> *mut T {
    let name: *const u8 = name.cast();
    {
        if !partialp.is_null() {
            unsafe {
                *partialp = ptr::null_mut();
            }
        }

        let v = find_var(name.cast(), *lenp as usize, ptr::null_mut(), no_autoload as i32);
        if v.is_null() {
            return name.cast_mut().cast();
        }
        let tv = unsafe { &mut (*v).di_tv };
        if let Some(found_var) = found_var {
            *found_var = true;
        }

        if tv.v_type == VarType::VarFunc {
            if tv.vval.v_string().is_null() {
                *lenp = 0;
                return c"".as_ptr().cast_mut().cast();
            }
            *lenp = crate::memory::cstr_len(tv.vval.v_string()) as i32;
            return tv.vval.v_string().cast();
        }

        if tv.v_type == VarType::VarPartial {
            let pt = tv.vval.v_partial();
            if pt.is_null() {
                *lenp = 0;
                return c"".as_ptr().cast_mut().cast();
            }
            if !partialp.is_null() {
                unsafe {
                    *partialp = pt;
                }
            }
            let s = partial_name(unsafe { pt.as_ref() });
            *lenp = crate::memory::cstr_len(s) as i32;
            return s.cast();
        }

        name.cast_mut().cast()
    }
}

/// Give an error message with a function name. Handle `<SNR>` things.
pub fn emsg_funcname<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    errmsg: S1,
    name: S2,
) {
    let errmsg = errmsg.into_c_char_ptr();
    let name = name.into_c_char_ptr();
    {
        let mut p_owned: Option<std::ffi::CString> = None;
        let mut p = name;

        if crate::memory::cstr_first_byte(name) == K_SPECIAL
            && crate::memory::cstr_byte_at(name, 1) != 0
            && crate::memory::cstr_byte_at(name, 2) != 0
        {
            let mut bytes = Vec::from(&b"<SNR>"[..]);
            bytes.extend_from_slice(crate::memory::cstr_as_bytes(name.wrapping_add(3)));
            let joined = cstr_from_bytes(&bytes);
            p = joined.as_ptr();
            p_owned = Some(joined);
        }

        semsg!(gettext(errmsg), p);
        drop(p_owned);
    }
}

/// Get function arguments at `*arg` and advance it.
fn get_func_arguments<T>(
    arg: *mut *mut T,
    mut evalarg: Option<&mut Evalarg>,
    partial_argc: i32,
    argvars: *mut TypVal,
    argcount: &mut i32,
) -> bool {
    let mut argp = unsafe { (*arg).cast::<u8>() };
    let mut ret = true;

    while *argcount < MAX_FUNC_ARGS as i32 - partial_argc {
        argp = skipwhite(argp.wrapping_add(1).cast()).cast();

        if crate::memory::cstr_first_byte(argp.cast()) == b')'
            || crate::memory::cstr_first_byte(argp.cast()) == b','
            || crate::memory::cstr_is_empty(argp.cast())
        {
            break;
        }
        if !eval1(
            (&raw mut argp).cast(),
            unsafe { &mut *argvars.wrapping_add(*argcount as usize) },
            evalarg.as_deref_mut(),
        ) {
            ret = false;
            break;
        }
        *argcount += 1;
        if crate::memory::cstr_first_byte(argp.cast()) != b',' {
            break;
        }
    }

    argp = skipwhite(argp.cast()).cast();
    if crate::memory::cstr_first_byte(argp.cast()) == b')' {
        argp = argp.wrapping_add(1);
    } else {
        ret = false;
    }
    unsafe {
        *arg = argp.cast();
    }
    ret
}

/// Call a function and put the result in `rettv`.
pub fn get_func_tv<T>(
    name: impl crate::strings::IntoCCharPtr,
    len: i32,
    rettv: &mut TypVal,
    arg: *mut *mut T,
    evalarg: Option<&mut Evalarg>,
    funcexe: &mut Funcexe,
) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let mut argvars: [TypVal; MAX_FUNC_ARGS + 1] = [TypVal::new(); MAX_FUNC_ARGS + 1];
        let mut argcount: i32 = 0;
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

        let mut argp = *arg;
        let partial_argc = if funcexe.fe_partial.is_null() {
            0
        } else {
            (*funcexe.fe_partial).pt_argv.len() as i32
        };
        let mut ret = get_func_arguments(
            &raw mut argp,
            evalarg,
            partial_argc,
            argvars.as_mut_ptr(),
            &mut argcount,
        );

        if ret {
            let mut i = 0usize;

            if get_vim_var_nr(VV_TESTING) != 0 {
                while (i as i32) < argcount {
                    uf_st().funcargs.push(&raw mut argvars[i]);
                    i += 1;
                }
            }
            ret = call_func(name, len, rettv, argcount, argvars.as_mut_ptr(), funcexe);

            if i > 0 {
                let new_len = uf_st().funcargs.len().saturating_sub(i);
                uf_st().funcargs.truncate(new_len);
            }
        } else if !aborting() && evaluate {
            if argcount == MAX_FUNC_ARGS as i32 {
                emsg_funcname(c"E740: Too many arguments for function %s", name);
            } else {
                emsg_funcname(c"E116: Invalid arguments for function %s", name);
            }
        }

        while argcount > 0 {
            argcount -= 1;
            tv_clear(&mut argvars[argcount as usize]);
        }

        *arg = skipwhite(argp.cast()).cast();
        ret
    }
}

/// Check whether function name starts with `<SID>` or `s:`.
#[inline]
fn eval_fname_sid(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    crate::memory::cstr_first_byte(name) == b's'
        || toupper_asc(crate::memory::cstr_char_at(name, 2)) == b'I' as i32
}

/// In a script transform script-local names into actually used names.
fn fname_trans_sid(
    name: impl crate::strings::IntoCCharPtr,
    fname_buf: *mut u8,
    tofree: &mut Option<std::ffi::CString>,
    error: &mut i32,
) -> *mut u8 {
    let name: *const u8 = name.into_c_char_ptr().cast();
    {
        let script_name = name.wrapping_add(eval_fname_script(name.cast()) as usize);
        if script_name == name {
            return name.cast_mut();
        }

        unsafe {
            *fname_buf.wrapping_add(0) = K_SPECIAL;
        }
        unsafe {
            *fname_buf.wrapping_add(1) = KS_EXTRA;
        }
        unsafe {
            *fname_buf.wrapping_add(2) = KE_SNR;
        }
        let mut fname_buflen = 3usize;
        unsafe {
            *fname_buf.wrapping_add(fname_buflen) = 0;
        }
        if eval_fname_sid(name.cast()) {
            if g_st().current_sctx.sc_sid <= 0 {
                *error = FCERR_SCRIPT;
            } else {
                fname_buflen += vim_snprintf!(
                    fname_buf.wrapping_add(fname_buflen).cast(),
                    FLEN_FIXED + 1 - fname_buflen,
                    c"%d_",
                    g_st().current_sctx.sc_sid,
                ) as usize;
            }
        }
        let fnamelen = fname_buflen + crate::memory::cstr_len(script_name.cast());
        if fnamelen < FLEN_FIXED {
            crate::memory::cstr_cpy(
                fname_buf.wrapping_add(fname_buflen).cast(),
                script_name.cast(),
            );
            fname_buf
        } else {
            let mut fname_vec = vec![0u8; fnamelen + 1];
            vim_snprintf!(
                fname_vec.as_mut_ptr().cast(),
                fnamelen + 1,
                c"%s%s",
                fname_buf,
                script_name,
            );
            let cstr = cstr_to_cstring(fname_vec.as_ptr().cast());
            let ptr = cstr.as_ptr().cast_mut().cast();
            *tofree = Some(cstr);
            ptr
        }
    }
}

pub fn get_func_arity(
    name: impl crate::strings::IntoCCharPtr,
    required: &mut i32,
    optional: &mut i32,
    varargs: &mut bool,
) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let argcount: i32;
        let min_argcount: i32;

        let fdef = find_internal_func(name);
        if !fdef.is_null() {
            let fdef = &*fdef;
            argcount = fdef.max_argc as i32;
            min_argcount = fdef.min_argc as i32;
            *varargs = false;
        } else {
            let mut fname_buf = [0u8; FLEN_FIXED + 1];
            let mut tofree: Option<std::ffi::CString> = None;
            let mut error = FCERR_NONE;

            let fname = fname_trans_sid(name, fname_buf.as_mut_ptr(), &mut tofree, &mut error);
            let mut ufunc: *mut UserFunc = ptr::null_mut();
            if error == FCERR_NONE {
                ufunc = find_func(fname.cast());
            }
            drop(tofree);

            if ufunc.is_null() {
                return false;
            }

            argcount = (*ufunc).uf_args.len() as i32;
            min_argcount = argcount - (*ufunc).uf_def_args.len() as i32;
            *varargs = (*ufunc).uf_varargs != 0;
        }

        *required = min_argcount;
        *optional = argcount - min_argcount;

        true
    }
}

/// Find a function by name, return pointer to it in ufuncs.
pub fn find_func(name: impl crate::strings::IntoCCharPtr) -> *mut UserFunc {
    let name = name.into_c_char_ptr();
    if name.is_null() {
        return ptr::null_mut();
    }
    {
        uf_st()
            .func_hashtab
            .get(crate::memory::cstr_as_bytes(name))
            .copied()
            .unwrap_or(ptr::null_mut())
    }
}

#[inline]
fn func_is_global(ufunc: &UserFunc) -> bool {
    ufunc.uf_name.as_bytes_with_nul()[0] != K_SPECIAL
}

fn cat_func_name<T>(buf: *mut T, bufsize: usize, fp: &UserFunc) -> i32 {
    let buf = buf.cast();
    let uflen = fp.uf_namelen;
    debug_assert!(uflen > 0);

    let len = if !func_is_global(fp) && uflen > 3 {
        vim_snprintf!(buf, bufsize, c"<SNR>%s", uf2hikey(fp).as_ptr().wrapping_add(3))
    } else {
        vim_snprintf!(buf, bufsize, c"%s", uf2hikey(fp))
    };

    debug_assert!(len > 0);
    if len >= bufsize as i32 {
        bufsize as i32 - 1
    } else {
        len
    }
}

/// Add a number variable `name` to dict `dp` with value `nr`.
fn add_nr_var(dp: &mut TvDict, name: &std::ffi::CStr, nr: Varnumber) {
    let v = tv_dict_item_alloc(name.as_ptr());
    v.di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
    v.di_tv.v_type = VarType::VarNumber;
    v.di_tv.v_lock = VAR_FIXED;
    v.di_tv.vval = crate::eval::typval::TypvalVval::Number(nr);
    hash_add(&mut dp.dv_hashtab, &mut *v);
}

impl Drop for Funccall {
    fn drop(&mut self) {
        vars_clear(&mut self.fc_l_vars.dv_hashtab);
        vars_clear(&mut self.fc_l_avars.dv_hashtab);
        let self_ptr = std::ptr::from_mut(self).cast();
        for &fp in &self.fc_ufuncs {
            if !fp.is_null() && unsafe { (*fp).uf_scoped } == self_ptr {
                unsafe {
                    (*fp).uf_scoped = ptr::null_mut();
                }
            }
        }
        func_ptr_unref(self.fc_func);
    }
}

/// Handle the last part of returning from a function: free the local hashtable.
/// Unless it is still in use by a closure.
fn cleanup_function_call(fc_ptr: *mut Funccall) {
    let fc = unsafe { &mut *fc_ptr };
    let may_free_fc = fc.fc_refcount <= 0;
    let mut free_fc = true;

    uf_st().current_funccal = fc.fc_caller;

    if may_free_fc && fc.fc_l_vars.dv_refcount == DO_NOT_FREE_CNT {
        vars_clear(&mut fc.fc_l_vars.dv_hashtab);
    } else {
        free_fc = false;
    }

    if may_free_fc && fc.fc_l_avars.dv_refcount == DO_NOT_FREE_CNT {
        vars_clear_ext(&mut fc.fc_l_avars.dv_hashtab, false);
    } else {
        free_fc = false;

        {
            let mut todo = fc.fc_l_avars.dv_hashtab.ht_used;
            let mut hi = fc.fc_l_avars.dv_hashtab.array_ptr();
            while todo > 0 {
                if !hashitem_empty(unsafe { &*hi }) {
                    todo -= 1;
                    let di = tv_dict_hi2di(hi);
                    let tv_val = di.di_tv;
                    tv_copy(&tv_val, &mut di.di_tv);
                }
                hi = hi.wrapping_add(1);
            }
        }
    }

    if may_free_fc && fc.fc_l_varlist.lv_refcount == DO_NOT_FREE_CNT {
        for li in crate::eval::typval::list_iter_mut(fc.fc_l_varlist.lv_first) {
            li.li_tv.v_type = VarType::VarUnknown;
        }
        fc.fc_l_varlist.lv_first = ptr::null_mut();
    } else {
        free_fc = false;

        {
            for li in crate::eval::typval::list_iter_mut(fc.fc_l_varlist.lv_first) {
                let src_tv = li.li_tv;
                tv_copy(&src_tv, &mut li.li_tv);
            }
        }
    }

    if free_fc {
        drop(unsafe { Box::from_raw(fc_ptr) });
    } else {
        fc.fc_caller = ptr::null_mut();
        uf_st()
            .previous_funccal
            .push(RetainedFunccall(unsafe { Box::from_raw(fc_ptr) }));

        if g_st().want_garbage_collect {
            uf_st().made_copy = 0;
        } else {
            uf_st().made_copy += 1;
            if uf_st().made_copy >= ((4096 * 1024) / std::mem::size_of::<Funccall>()) as i32 {
                uf_st().made_copy = 0;
                g_st().want_garbage_collect = true;
            }
        }
    }
}

/// Unreference `fc`: decrement the reference count and free it when it
/// becomes zero. `fp` is detached from `fc`.
fn funccal_unref(fc: *mut Funccall, fp: &mut UserFunc, force: bool) {
    unsafe {
        if fc.is_null() {
            return;
        }

        (*fc).fc_refcount -= 1;
        if if force {
            (*fc).fc_refcount <= 0
        } else {
            !fc_referenced(&*fc)
        } && let Some(pos) = uf_st()
            .previous_funccal
            .iter()
            .position(|item| ptr::eq(&raw const *item.0, fc))
        {
            uf_st().previous_funccal.remove(pos);
            return;
        }
        for slot in &mut (*fc).fc_ufuncs {
            if std::ptr::eq(*slot, fp) {
                *slot = ptr::null_mut();
            }
        }
    }
}

/// Remove the function from the function hashtable.
fn func_remove(fp: &mut UserFunc) -> bool {
    let key = crate::memory::cstr_as_bytes(uf2hikey(fp));
    if uf_st().func_hashtab.remove(key).is_some() {
        uf_st().func_hashtab_changed = uf_st().func_hashtab_changed.wrapping_add(1);
        true
    } else {
        false
    }
}

fn func_clear_items(fp: &mut UserFunc) {
    fp.uf_args.clear();
    fp.uf_def_args.clear();
    fp.uf_lines.clear();

    if (fp.uf_flags & FC_LUAREF) != 0 {
        api_free_luaref(fp.uf_luaref);
        fp.uf_luaref = LUA_NOREF;
    }

    fp.uf_tml_count.clear();
    fp.uf_tml_total.clear();
    fp.uf_tml_self.clear();
}

fn func_clear(fp: &mut UserFunc, force: bool) {
    if fp.uf_cleared {
        return;
    }
    fp.uf_cleared = true;

    func_clear_items(fp);
    funccal_unref(fp.uf_scoped, fp, force);
}

impl Drop for UserFunc {
    fn drop(&mut self) {
        if !self.uf_cleared {
            self.uf_cleared = true;
            func_clear_items(self);
        }
        if (self.uf_flags & (FC_DELETED | FC_REMOVED)) == 0 {
            func_remove(self);
        }
    }
}

fn func_clear_free(fp: *mut UserFunc, force: bool) {
    unsafe {
        func_clear(&mut *fp, force);
        drop(Box::from_raw(fp));
    }
}

/// Allocate a `Funccall`, link it in `uf_st().current_funccal` and fill in `fp` and `rettv`.
pub fn create_funccal(fp: *mut UserFunc, rettv: &mut TypVal) -> &'static mut Funccall {
    let fc = Box::leak(Box::new(Funccall::new()));
    fc.fc_caller = uf_st().current_funccal;
    uf_st().current_funccal = fc;
    fc.fc_func = fp;
    func_ptr_ref(unsafe { fp.as_mut() });
    fc.fc_rettv = rettv;
    &mut *fc
}

/// Call a user function.
pub fn call_user_func(
    fp: &mut UserFunc,
    argcount: i32,
    argvars: *mut TypVal,
    rettv: &mut TypVal,
    firstline: Linenr,
    lastline: Linenr,
    selfdict: *mut TvDict,
) {
    unsafe {
        let mut using_sandbox = false;
        let mut islambda = false;
        let mut numbuf = [0u8; NUMBUFLEN];
        let mut name: *const u8;
        let mut namelen: usize;
        let mut tv_to_free: [*mut TypVal; MAX_FUNC_ARGS] = [ptr::null_mut(); MAX_FUNC_ARGS];
        let mut tv_to_free_len = 0usize;
        let mut wait_start: Proftime = 0;
        let mut call_start: Proftime = 0;
        let mut started_profiling = false;
        let mut did_save_redo = false;
        let mut save_redo = RedoState::new();

        if i64::from(uf_st().call_depth) >= opt_vars().p_mfd {
            emsg(gettext(
                c"E132: Function call depth is higher than 'maxfuncdepth'",
            ));
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
            return;
        }
        uf_st().call_depth += 1;
        save_search_patterns();
        if !ins_compl_active() {
            save_redobuff(&mut save_redo);
            did_save_redo = true;
        }
        fp.uf_calls += 1;
        line_breakcheck();

        let fc = create_funccal(fp, rettv);
        fc.fc_level = g_st().ex_nesting_level;
        fc.fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(&*fp), 0);
        fc.fc_dbg_tick = g_st().debug_tick;

        if uf2hikey(&*fp).to_bytes().starts_with(b"<lambda>") {
            islambda = true;
        }

        init_var_dict(
            &mut fc.fc_l_vars,
            &mut *(&raw mut fc.fc_l_vars_var).cast(),
            VAR_DEF_SCOPE,
        );
        if !selfdict.is_null() {
            let v = tv_dict_item_alloc_len(c"self", 4);
            v.di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            v.di_tv.v_type = VarType::VarDict;
            v.di_tv.v_lock = VAR_UNLOCKED;
            v.di_tv.vval = crate::eval::typval::TypvalVval::Dict(selfdict);
            (*selfdict).dv_refcount += 1;
            hash_add(&mut fc.fc_l_vars.dv_hashtab, &mut *v);
        }

        init_var_dict(
            &mut fc.fc_l_avars,
            &mut *(&raw mut fc.fc_l_avars_var).cast(),
            VAR_SCOPE,
        );
        let uf_args_len = fp.uf_args.len() as i32;
        let uf_def_args_len = fp.uf_def_args.len() as i32;
        if (fp.uf_flags & FC_NOARGS) == 0 {
            add_nr_var(
                &mut fc.fc_l_avars,
                c"0",
                if argcount >= uf_args_len {
                    Varnumber::from(argcount - uf_args_len)
                } else {
                    0
                },
            );
        }
        fc.fc_l_avars.dv_lock = VAR_FIXED;
        if (fp.uf_flags & FC_NOARGS) == 0 {
            let v = tv_dict_item_alloc_len(c"000", 3);
            v.di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            v.di_tv.v_type = VarType::VarList;
            v.di_tv.v_lock = VAR_FIXED;
            v.di_tv.vval = crate::eval::typval::TypvalVval::List(&raw mut fc.fc_l_varlist);
            hash_add(&mut fc.fc_l_avars.dv_hashtab, &mut *v);
        }
        tv_list_init_static(&mut fc.fc_l_varlist);
        tv_list_set_lock(Some(&mut fc.fc_l_varlist), VAR_FIXED);

        if (fp.uf_flags & FC_NOARGS) == 0 {
            add_nr_var(&mut fc.fc_l_avars, c"firstline", Varnumber::from(firstline));
            add_nr_var(&mut fc.fc_l_avars, c"lastline", Varnumber::from(lastline));
        }

        let mut default_arg_err = false;
        let max_i = argcount.max(uf_args_len);
        let listitems_ptr = (&raw mut fc.fc_l_listitems).cast::<Listitem>();
        for i in 0..max_i {
            let mut addlocal = false;
            let mut isdefault = false;
            let mut def_rettv: TypVal = TypVal::new();

            let ai = i - uf_args_len;
            if ai < 0 {
                let fa = funcarg(&*fp, i);
                name = fa.as_ptr().cast();
                if islambda {
                    addlocal = true;
                }

                isdefault = ai + uf_def_args_len >= 0 && i >= argcount;
                if isdefault {
                    def_rettv.v_type = VarType::VarNumber;
                    def_rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

                    let mut default_expr = (&fp.uf_def_args)[(ai + uf_def_args_len) as usize]
                        .as_ptr()
                        .cast_mut();
                    if !eval1(
                        &raw mut default_expr,
                        &mut def_rettv,
                        Some(&mut g_st().evalarg_evaluate),
                    ) {
                        default_arg_err = true;
                        break;
                    }
                }

                namelen = fa.to_bytes().len();
            } else {
                if (fp.uf_flags & FC_NOARGS) != 0 {
                    break;
                }
                namelen =
                    vim_snprintf!(numbuf.as_mut_ptr().cast(), NUMBUFLEN, c"%d", ai + 1) as usize;
                name = numbuf.as_ptr();
            }

            let v = tv_dict_item_alloc_len(name.cast(), namelen);
            v.di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;

            v.di_tv = if isdefault {
                def_rettv
            } else {
                *argvars.wrapping_add(i as usize)
            };
            v.di_tv.v_lock = VAR_FIXED;

            if isdefault {
                tv_to_free[tv_to_free_len] = &raw mut v.di_tv;
                tv_to_free_len += 1;
            }

            if addlocal {
                let tv_val = v.di_tv;
                tv_copy(&tv_val, &mut v.di_tv);
                hash_add(&mut fc.fc_l_vars.dv_hashtab, &mut *v);
            } else {
                hash_add(&mut fc.fc_l_avars.dv_hashtab, &mut *v);
            }

            if ai >= 0 && (ai as usize) < MAX_FUNC_ARGS {
                let li = listitems_ptr.wrapping_add(ai as usize);
                (*li).li_tv = *argvars.wrapping_add(i as usize);
                (*li).li_tv.v_lock = VAR_FIXED;
                tv_list_append(&mut fc.fc_l_varlist, &mut *li);
            }
        }

        g_st().redrawing_disabled += 1;

        if (fp.uf_flags & FC_SANDBOX) != 0 {
            using_sandbox = true;
            g_st().sandbox += 1;
        }

        estack_push_ufunc(&mut *fp, 1);
        let estack_len_before = rt_st().exestack.len() as i32;
        if opt_vars().p_verbose >= 12 {
            g_st().no_wait_return += 1;
            verbose_enter_scroll();

            smsg!(0, gettext(c"calling %s"), sourcing_name());
            if opt_vars().p_verbose >= 14 {
                msg_puts(c"(");
                for i in 0..argcount {
                    if i > 0 {
                        msg_puts(c", ");
                    }
                    let arg_tv = argvars.wrapping_add(i as usize);
                    if (*arg_tv).v_type == VarType::VarNumber {
                        msg_outnum((*arg_tv).vval.v_number() as i32);
                    } else {
                        g_st().emsg_off += 1;
                        let tf = encode_tv2string(&mut *arg_tv, None);
                        g_st().emsg_off -= 1;
                        let mut s = tf.as_ptr().cast_mut();
                        let mut buf = [0u8; MSG_BUF_LEN];
                        if vim_strsize(s) > MSG_BUF_CLEN {
                            trunc_string(
                                s,
                                buf.as_mut_ptr().cast(),
                                MSG_BUF_CLEN,
                                MSG_BUF_LEN as i32,
                            );
                            s = buf.as_mut_ptr().cast();
                        }
                        msg_puts(s);
                    }
                }
                msg_puts(c")");
            }
            msg_puts(c"\n");

            verbose_leave_scroll();
            g_st().no_wait_return -= 1;
        }

        let do_profiling_yes = g_st().do_profiling == PROF_YES;
        let func_not_yet_profiling_but_should =
            do_profiling_yes && fp.uf_profiling == 0 && has_profiling(false, uf2hikey(&*fp), None);

        if func_not_yet_profiling_but_should {
            started_profiling = true;
            func_do_profile(fp);
        }

        let func_or_func_caller_profiling = do_profiling_yes
            && (fp.uf_profiling != 0
                || (!fc.fc_caller.is_null() && (*(*fc.fc_caller).fc_func).uf_profiling != 0));

        if func_or_func_caller_profiling {
            fp.uf_tm_count += 1;
            call_start = profile_start();
            fp.uf_tm_children = profile_zero();
        }

        if do_profiling_yes {
            script_prof_save(&mut wait_start);
        }

        let save_current_sctx = g_st().current_sctx;
        g_st().current_sctx = fp.uf_script_ctx;
        let save_did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;

        if default_arg_err && ((fp.uf_flags & FC_ABORT) != 0 || g_st().trylevel > 0) {
            g_st().did_emsg = 1;
        } else if islambda {
            let mut p = (&fp.uf_lines)[0]
                .as_ref()
                .unwrap()
                .as_ptr()
                .wrapping_add(7)
                .cast_mut();
            g_st().ex_nesting_level += 1;
            eval1(&raw mut p, rettv, Some(&mut g_st().evalarg_evaluate));
            g_st().ex_nesting_level -= 1;
        } else {
            do_cmdline(
                ptr::null_mut(),
                crate::cmdexpand::LineGetter::Func(fc),
                DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
            );
        }

        handle_defer_one(&mut *uf_st().current_funccal);

        g_st().redrawing_disabled -= 1;

        if (g_st().did_emsg != 0 && (fp.uf_flags & FC_ABORT) != 0)
            || rettv.v_type == VarType::VarUnknown
        {
            tv_clear(rettv);
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }

        if func_or_func_caller_profiling {
            call_start = profile_end(call_start);
            call_start = profile_sub_wait(wait_start, call_start);
            fp.uf_tm_total = profile_add(fp.uf_tm_total, call_start);
            fp.uf_tm_self = profile_self(fp.uf_tm_self, call_start, fp.uf_tm_children);
            if !fc.fc_caller.is_null() && (*(*fc.fc_caller).fc_func).uf_profiling != 0 {
                let caller_fp = (*fc.fc_caller).fc_func;
                let caller_fp = &mut *caller_fp;
                caller_fp.uf_tm_children = profile_add(caller_fp.uf_tm_children, call_start);
                caller_fp.uf_tml_children = profile_add(caller_fp.uf_tml_children, call_start);
            }
            if started_profiling {
                fp.uf_profiling = 0;
            }
        }

        if opt_vars().p_verbose >= 12 {
            g_st().no_wait_return += 1;
            verbose_enter_scroll();

            if aborting() {
                smsg!(0, gettext(c"%s aborted"), sourcing_name());
            } else if (*fc.fc_rettv).v_type == VarType::VarNumber {
                smsg!(
                    0,
                    gettext(c"%s returning #%lld"),
                    sourcing_name(),
                    (*fc.fc_rettv).vval.v_number(),
                );
            } else {
                let mut buf = [0u8; MSG_BUF_LEN];
                g_st().emsg_off += 1;
                let tf = encode_tv2string(&mut *fc.fc_rettv, None);
                g_st().emsg_off -= 1;
                let mut s = tf.as_ptr().cast_mut();
                if vim_strsize(s) > MSG_BUF_CLEN {
                    trunc_string(s, buf.as_mut_ptr().cast(), MSG_BUF_CLEN, MSG_BUF_LEN as i32);
                    s = buf.as_mut_ptr().cast();
                }
                smsg!(0, gettext(c"%s returning %s"), sourcing_name(), s);
            }
            msg_puts(c"\n");

            verbose_leave_scroll();
            g_st().no_wait_return -= 1;
        }

        let _ = estack_len_before;
        estack_pop();
        g_st().current_sctx = save_current_sctx;
        if do_profiling_yes {
            script_prof_restore(&wait_start);
        }
        if using_sandbox {
            g_st().sandbox -= 1;
        }

        if opt_vars().p_verbose >= 12 && sourcing_name().is_some() {
            g_st().no_wait_return += 1;
            verbose_enter_scroll();

            smsg!(0, gettext(c"continuing in %s"), sourcing_name());
            msg_puts(c"\n");

            verbose_leave_scroll();
            g_st().no_wait_return -= 1;
        }

        g_st().did_emsg |= save_did_emsg;
        uf_st().call_depth -= 1;
        for &tv in tv_to_free.iter().take(tv_to_free_len) {
            tv_clear(&mut *tv);
        }
        cleanup_function_call(fc);

        fp.uf_calls -= 1;
        if fp.uf_calls <= 0 && fp.uf_refcount <= 0 {
            func_clear_free(fp, false);
        }
        if did_save_redo {
            restore_redobuff(&mut save_redo);
        }
        restore_search_patterns();
    }
}

#[inline]
fn func_name_refcount(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    {
        (crate::memory::cstr_first_byte(name)).is_ascii_digit()
            || (crate::memory::cstr_first_byte(name) == b'<'
                && crate::memory::cstr_byte_at(name, 1) == b'l')
    }
}

fn check_user_func_argcount(fp: &mut UserFunc, argcount: i32) -> i32 {
    let regular_args = fp.uf_args.len() as i32;
    if argcount < regular_args - fp.uf_def_args.len() as i32 {
        FCERR_TOOFEW
    } else if fp.uf_varargs == 0 && argcount > regular_args {
        FCERR_TOOMANY
    } else {
        FCERR_UNKNOWN
    }
}

fn call_user_func_check(
    fp: &mut UserFunc,
    argcount: i32,
    argvars: *mut TypVal,
    rettv: &mut TypVal,
    funcexe: &mut Funcexe,
    selfdict: *mut TvDict,
) -> i32 {
    if (fp.uf_flags & FC_LUAREF) != 0 {
        return nlua_exec_typval_callable(fp.uf_luaref, argcount, argvars, rettv);
    }

    if (fp.uf_flags & FC_RANGE) != 0 {
        funcexe.fe_doesrange = true;
    }
    let mut error = check_user_func_argcount(fp, argcount);
    if error != FCERR_UNKNOWN {
        return error;
    }
    if (fp.uf_flags & FC_DICT) != 0 && selfdict.is_null() {
        error = FCERR_DICT;
    } else {
        call_user_func(
            fp,
            argcount,
            argvars,
            rettv,
            funcexe.fe_firstline,
            funcexe.fe_lastline,
            if (fp.uf_flags & FC_DICT) != 0 {
                selfdict
            } else {
                ptr::null_mut()
            },
        );
        error = FCERR_NONE;
    }
    error
}

/// Save the current function call pointer, and set it to NULL.
pub fn save_funccal() {
    uf_st().funccal_stack.push(uf_st().current_funccal);
    uf_st().current_funccal = ptr::null_mut();
}

pub fn restore_funccal() {
    if let Some(top) = uf_st().funccal_stack.pop() {
        uf_st().current_funccal = top;
    } else {
        iemsg(c"INTERNAL: restore_funccal()");
    }
}

pub fn get_current_funccal() -> *mut Funccall {
    uf_st().current_funccal
}

pub fn set_current_funccal(fc: *mut Funccall) {
    uf_st().current_funccal = fc;
}

pub fn free_all_functions() {
    while !uf_st().current_funccal.is_null() {
        tv_clear(unsafe { &mut *cur_funccal_mut().fc_rettv });
        cleanup_function_call(uf_st().current_funccal);
        if uf_st().current_funccal.is_null() && !uf_st().funccal_stack.is_empty() {
            restore_funccal();
        }
    }
    uf_st().funccal_stack = Vec::new();

    loop {
        let mut restarted = false;
        let funcs: Vec<*mut UserFunc> = uf_st().func_hashtab.values().copied().collect();
        for fp in funcs {
            if !func_name_refcount(uf2hikey(unsafe { &*fp })) {
                let changed = uf_st().func_hashtab_changed;
                func_clear(unsafe { &mut *fp }, true);
                if changed != uf_st().func_hashtab_changed {
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
        if uf_st().func_hashtab.len() <= skipped {
            break;
        }
        let funcs: Vec<*mut UserFunc> = uf_st().func_hashtab.values().copied().collect();
        for fp in funcs {
            if func_name_refcount(uf2hikey(unsafe { &*fp })) {
                skipped += 1;
            } else {
                drop(unsafe { Box::from_raw(fp) });
                skipped = 0;
                break;
            }
        }
    }
    if skipped == 0 {
        uf_st().func_hashtab = std::collections::BTreeMap::new();
    }
    uf_st().get_uf_items = Vec::new();
}

/// Checks if a builtin function with the given name exists.
fn builtin_function(name: impl crate::strings::IntoCCharPtr, len: i32) -> bool {
    let name = name.into_c_char_ptr();
    {
        if !ascii_islower(crate::memory::cstr_first_char(name))
            || crate::memory::cstr_byte_at(name, 1) == b':'
        {
            return false;
        }

        let bytes = if len == -1 {
            crate::memory::cstr_as_bytes(name)
        } else {
            crate::memory::cstr_slice(name, len as usize)
        };
        !bytes.contains(&(AUTOLOAD_CHAR as u8))
    }
}

pub fn func_call(
    name: impl crate::strings::IntoCCharPtr,
    args: &mut TypVal,
    partial: *mut TvPartial,
    selfdict: *mut TvDict,
    rettv: &mut TypVal,
) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let mut argv: [TypVal; MAX_FUNC_ARGS + 1] = [TypVal::new(); MAX_FUNC_ARGS + 1];
        let mut argc = 0;
        let mut r = false;

        let list = args.vval.v_list();
        let max_argc = MAX_FUNC_ARGS as i32
            - if partial.is_null() {
                0
            } else {
                (*partial).pt_argv.len() as i32
            };
        let mut skip_call = false;
        if !list.is_null() {
            for item in crate::eval::typval::list_iter_mut((*list).lv_first) {
                if argc == max_argc {
                    emsg(gettext(c"E699: Too many arguments"));
                    skip_call = true;
                    break;
                }
                tv_copy(&item.li_tv, &mut argv[argc as usize]);
                argc += 1;
            }
        }

        if !skip_call {
            let mut funcexe = Funcexe::new();
            funcexe.fe_firstline = curwin_mut().w_cursor.lnum;
            funcexe.fe_lastline = curwin_mut().w_cursor.lnum;
            funcexe.fe_evaluate = true;
            funcexe.fe_partial = partial;
            funcexe.fe_selfdict = selfdict;
            r = call_func(name, -1, rettv, argc, argv.as_mut_ptr(), &mut funcexe);
        }

        while argc > 0 {
            argc -= 1;
            tv_clear(&mut argv[argc as usize]);
        }

        r
    }
}

pub fn callback_call_retnr(
    callback: &mut Callback,
    argcount: i32,
    argvars: *mut TypVal,
) -> Varnumber {
    let mut rettv: TypVal = TypVal::new();
    if !callback_call(callback, argcount, argvars, &mut rettv) {
        return -2;
    }

    let retval = tv_get_number_chk(&rettv, None);
    tv_clear(&mut rettv);
    retval
}

fn user_func_error(error: i32, name: impl crate::strings::IntoCCharPtr, found_var: bool) {
    let name = name.into_c_char_ptr();
    match error {
        FCERR_UNKNOWN => {
            if found_var {
                semsg!(gettext(E_NOT_CALLABLE_TYPE_STR), name);
            } else {
                emsg_funcname(E_UNKNOWN_FUNCTION_STR, name);
            }
        }
        FCERR_NOTMETHOD => {
            emsg_funcname(c"E276: Cannot use function as a method: %s", name);
        }
        FCERR_DELETED => {
            emsg_funcname(c"E933: Function was deleted: %s", name);
        }
        FCERR_TOOMANY => {
            emsg_funcname(gettext(E_TOOMANYARG), name);
        }
        FCERR_TOOFEW => {
            emsg_funcname(gettext(E_TOOFEWARG), name);
        }
        FCERR_SCRIPT => {
            emsg_funcname(c"E120: Using <SID> not in a script context: %s", name);
        }
        FCERR_DICT => {
            emsg_funcname(c"E725: Calling dict function without Dictionary: %s", name);
        }
        _ => {}
    }
}

fn argv_add_base(
    basetv: *mut TypVal,
    argvars: *mut *mut TypVal,
    argcount: &mut i32,
    new_argvars: *mut TypVal,
    argv_base: &mut i32,
) {
    unsafe {
        if !basetv.is_null() {
            ptr::copy(*argvars, new_argvars.wrapping_add(1), *argcount as usize);
            *new_argvars = *basetv;
            *argcount += 1;
            *argvars = new_argvars;
            *argv_base = 1;
        }
    }
}

/// Call a function with its resolved parameters.
pub fn call_func(
    funcname: impl crate::strings::IntoCCharPtr,
    mut len: i32,
    rettv: &mut TypVal,
    argcount_in: i32,
    argvars_in: *mut TypVal,
    funcexe: &mut Funcexe,
) -> bool {
    let mut funcname = funcname.into_c_char_ptr();
    {
        let mut ret = false;
        let mut error = FCERR_NONE;
        let mut fp: *mut UserFunc = ptr::null_mut();
        let mut fname_buf = [0u8; FLEN_FIXED + 1];
        let mut tofree: Option<std::ffi::CString> = None;
        let mut fname: *mut u8 = ptr::null_mut();
        let mut name_owned: Option<std::ffi::CString> = None;
        let mut argcount = argcount_in;
        let mut argvars = argvars_in;
        let mut selfdict = funcexe.fe_selfdict;
        let mut argv: [TypVal; MAX_FUNC_ARGS + 1] = [TypVal::new(); MAX_FUNC_ARGS + 1];
        let mut argv_clear = 0;
        let mut argv_base = 0;
        let partial = funcexe.fe_partial;

        rettv.v_type = VarType::VarUnknown;

        if len <= 0 {
            len = crate::memory::cstr_len(funcname) as i32;
        }
        if !partial.is_null() {
            fp = unsafe { (*partial).pt_func };
        }
        if fp.is_null() {
            let nm = cstr_from_bytes(unsafe {
                std::slice::from_raw_parts(funcname.cast::<u8>(), len as usize)
            });
            fname = fname_trans_sid(nm.as_ptr(), fname_buf.as_mut_ptr(), &mut tofree, &mut error);
            name_owned = Some(nm);
        }

        funcexe.fe_doesrange = false;

        let mut skip_eval = false;
        if !partial.is_null() {
            let partial = unsafe { &mut *partial };
            if !partial.pt_dict.is_null() && (selfdict.is_null() || !partial.pt_auto) {
                selfdict = partial.pt_dict;
            }
            let pt_argc = partial.pt_argv.len() as i32;
            if error == FCERR_NONE && pt_argc > 0 {
                while argv_clear < pt_argc {
                    if argv_clear + argcount_in >= MAX_FUNC_ARGS as i32 {
                        error = FCERR_TOOMANY;
                        skip_eval = true;
                        break;
                    }
                    tv_copy(
                        &(&partial.pt_argv)[argv_clear as usize],
                        &mut argv[argv_clear as usize],
                    );
                    argv_clear += 1;
                }
                if !skip_eval {
                    for i in 0..argcount_in {
                        argv[(i + argv_clear) as usize] =
                            unsafe { *argvars_in.wrapping_add(i as usize) };
                    }
                    argvars = argv.as_mut_ptr();
                    argcount = pt_argc + argcount_in;
                }
            }
        }

        if !skip_eval && error == FCERR_NONE && funcexe.fe_evaluate {
            let is_global = fp.is_null()
                && crate::memory::cstr_first_byte(fname.cast()) == b'g'
                && crate::memory::cstr_byte_at(fname.cast(), 1) == b':';
            let rfname = if is_global {
                fname.wrapping_add(2)
            } else {
                fname
            };

            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
            error = FCERR_UNKNOWN;

            if is_luafunc(partial) {
                if len > 0 {
                    error = FCERR_NONE;
                    argv_add_base(
                        funcexe.fe_basetv,
                        &raw mut argvars,
                        &mut argcount,
                        argv.as_mut_ptr(),
                        &mut argv_base,
                    );
                    nlua_call_vlua(funcname, len as usize, argvars, argcount, rettv);
                } else {
                    name_owned = None;
                    funcname = c"v:lua".as_ptr();
                }
            } else if !fp.is_null() || !builtin_function(rfname.cast(), -1) {
                if fp.is_null() {
                    fp = find_func(rfname.cast());
                }

                if fp.is_null()
                    && apply_autocmds(
                        EVENT_FUNCUNDEFINED,
                        rfname.cast(),
                        rfname.cast(),
                        true,
                        ptr::null_mut(),
                    )
                    && !aborting()
                {
                    fp = find_func(rfname.cast());
                }
                if fp.is_null()
                    && script_autoload(
                        rfname.cast(),
                        crate::memory::cstr_len(rfname.cast()),
                        true,
                    )
                    && !aborting()
                {
                    fp = find_func(rfname.cast());
                }

                if !fp.is_null() && (unsafe { (*fp).uf_flags } & FC_DELETED) != 0 {
                    error = FCERR_DELETED;
                } else if !fp.is_null() {
                    if funcexe.fe_fill_submatch {
                        argcount = crate::regexp::fill_submatch_list(
                            argcount,
                            argvars,
                            argv_clear,
                            unsafe { &mut *fp },
                        );
                    }

                    argv_add_base(
                        funcexe.fe_basetv,
                        &raw mut argvars,
                        &mut argcount,
                        argv.as_mut_ptr(),
                        &mut argv_base,
                    );

                    error = call_user_func_check(
                        unsafe { &mut *fp },
                        argcount,
                        argvars,
                        rettv,
                        &mut *funcexe,
                        selfdict,
                    );
                }
            } else if !funcexe.fe_basetv.is_null() {
                error = call_internal_method(
                    fname.cast(),
                    argcount,
                    argvars,
                    rettv,
                    funcexe.fe_basetv,
                );
            } else {
                error = call_internal_func(fname.cast(), argcount, argvars, rettv);
            }
            update_force_abort();
        }
        if !skip_eval && error == FCERR_NONE {
            ret = true;
        }

        if !aborting() {
            user_func_error(
                error,
                name_owned.as_ref().map_or(funcname, |nm| nm.as_ptr()),
                funcexe.fe_found_var,
            );
        }

        while argv_clear > 0 {
            argv_clear -= 1;
            tv_clear(&mut argv[(argv_clear + argv_base) as usize]);
        }

        drop(tofree);
        drop(name_owned);

        ret
    }
}

pub fn call_simple_luafunc(
    funcname: impl crate::strings::IntoCCharPtr,
    len: usize,
    rettv: &mut TypVal,
) -> i32 {
    let funcname = funcname.into_c_char_ptr();
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    let mut argvars: [TypVal; 1] = [TypVal::new(); 1];
    argvars[0].v_type = VarType::VarUnknown;
    nlua_call_vlua(funcname, len, argvars.as_mut_ptr(), 0, &mut *rettv);
    OK
}

pub fn call_simple_func(
    funcname: impl crate::strings::IntoCCharPtr,
    len: usize,
    rettv: &mut TypVal,
) -> i32 {
    let funcname = funcname.into_c_char_ptr();
    {
        let mut ret = FAIL;

        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);

        let name = cstr_from_bytes(crate::memory::cstr_slice(funcname, len));

        let mut error = FCERR_NONE;
        let mut tofree: Option<std::ffi::CString> = None;
        let mut fname_buf = [0u8; FLEN_FIXED + 1];
        let fname = fname_trans_sid(
            name.as_ptr(),
            fname_buf.as_mut_ptr(),
            &mut tofree,
            &mut error,
        );

        let is_global = crate::memory::cstr_first_byte(fname.cast()) == b'g'
            && crate::memory::cstr_byte_at(fname.cast(), 1) == b':';
        let rfname = if is_global {
            fname.wrapping_add(2)
        } else {
            fname
        };

        let fp = find_func(rfname.cast());
        if fp.is_null() {
            ret = NOTDONE;
        } else if (unsafe { (*fp).uf_flags } & FC_DELETED) != 0 {
            error = FCERR_DELETED;
        } else {
            let mut argvars: [TypVal; 1] = [TypVal::new(); 1];
            argvars[0].v_type = VarType::VarUnknown;
            let mut funcexe = Funcexe::new();
            funcexe.fe_evaluate = true;

            error = call_user_func_check(
                unsafe { &mut *fp },
                0,
                argvars.as_mut_ptr(),
                &mut *rettv,
                &mut funcexe,
                ptr::null_mut(),
            );
            if error == FCERR_NONE {
                ret = OK;
            }
        }

        user_func_error(error, name.as_ptr(), false);
        drop(tofree);

        ret
    }
}

pub fn printable_func_name(fp: &mut UserFunc) -> &CStr {
    if let Some(ref exp) = fp.uf_name_exp {
        exp.as_c_str()
    } else {
        uf2hikey(fp)
    }
}

fn function_list_modified(prev_ht_changed: i32) -> bool {
    if prev_ht_changed != uf_st().func_hashtab_changed {
        emsg(gettext(E_FUNCTION_LIST_WAS_MODIFIED));
        return true;
    }
    false
}

fn list_func_head(fp: &mut UserFunc, indent: bool, force: bool) -> bool {
    let prev_ht_changed = uf_st().func_hashtab_changed;

    msg_start();

    if function_list_modified(prev_ht_changed) {
        return false;
    }

    if indent {
        msg_puts(c"   ");
    }
    msg_puts(if force {
        c"function! ".as_ptr()
    } else {
        c"function ".as_ptr()
    });
    if let Some(ref exp) = fp.uf_name_exp {
        msg_puts(exp.as_ptr());
    } else {
        msg_puts(uf2hikey(fp));
    }
    msg_putchar(b'(' as i32);
    let uf_args_len = fp.uf_args.len() as i32;
    let uf_def_args_len = fp.uf_def_args.len() as i32;
    let mut j = 0;
    while j < uf_args_len {
        if j != 0 {
            msg_puts(c", ");
        }
        msg_puts(funcarg(fp, j));
        if j >= uf_args_len - uf_def_args_len {
            msg_puts(c" = ");
            msg_puts((&fp.uf_def_args)[(j - uf_args_len + uf_def_args_len) as usize].as_ptr());
        }
        j += 1;
    }
    if fp.uf_varargs != 0 {
        if j != 0 {
            msg_puts(c", ");
        }
        msg_puts(c"...");
    }
    msg_putchar(b')' as i32);
    if (fp.uf_flags & FC_ABORT) != 0 {
        msg_puts(c" abort");
    }
    if (fp.uf_flags & FC_RANGE) != 0 {
        msg_puts(c" range");
    }
    if (fp.uf_flags & FC_DICT) != 0 {
        msg_puts(c" dict");
    }
    if (fp.uf_flags & FC_CLOSURE) != 0 {
        msg_puts(c" closure");
    }
    msg_clr_eos();
    if opt_vars().p_verbose > 0 {
        last_set_msg(fp.uf_script_ctx);
    }

    true
}

/// Get a function name, translating `<SID>` and `<SNR>`.
pub fn trans_function_name<T>(
    pp: *mut *mut T,
    skip: bool,
    flags: i32,
    mut fdp: Option<&mut Funcdict>,
    partial: *mut *mut TvPartial,
) -> Option<std::ffi::CString> {
    unsafe {
        let mut name: *mut u8 = ptr::null_mut();
        let mut len: i32;
        let mut lv: Lval = Lval::new();

        if fdp.is_some() {
            *fdp.as_deref_mut().unwrap() = Funcdict::new();
        }
        let mut start: *const u8 = (*pp).cast();

        if crate::memory::cstr_first_byte((*pp).cast()) == K_SPECIAL
            && crate::memory::cstr_byte_at((*pp).cast(), 1) == KS_EXTRA
            && *((*pp).cast::<u8>()).wrapping_add(2) == KE_SNR
        {
            *pp = ((*pp).cast::<u8>()).wrapping_add(3).cast();
            len = get_id_len(pp.cast()) + 3;
            return Some(cstr_from_bytes(std::slice::from_raw_parts(
                start,
                len as usize,
            )));
        }

        let mut lead = eval_fname_script(start.cast());
        if lead > 2 {
            start = start.wrapping_add(lead as usize);
        }

        let end: *const u8 = get_lval(
            start.cast_mut().cast(),
            None,
            &mut lv,
            false,
            skip,
            flags | GLV_READ_ONLY,
            if lead > 2 { 0 } else { FNE_CHECK_START },
        )
        .cast_const()
        .cast();
        if end == start {
            if !skip {
                emsg(gettext(c"E129: Function name required"));
            }
            return None;
        }
        if end.is_null() || (!lv.ll_tv.is_null() && (lead > 2 || lv.ll_range)) {
            if !aborting() {
                if !end.is_null() {
                    semsg!(gettext(E_INVARG2), start);
                }
            } else {
                *pp = find_name_end(start.cast(), ptr::null_mut(), ptr::null_mut(), FNE_INCL_BR)
                    .cast_mut()
                    .cast();
            }
            return None;
        }

        if !lv.ll_tv.is_null() {
            if fdp.is_some() {
                fdp.as_deref_mut().unwrap().fd_dict = lv.ll_dict;
                fdp.as_deref_mut().unwrap().fd_newkey = lv.ll_newkey.take();
                fdp.as_deref_mut().unwrap().fd_di = lv.ll_di;
            }
            let mut res: Option<std::ffi::CString> = None;
            if (*lv.ll_tv).v_type == VarType::VarFunc && !(*lv.ll_tv).vval.v_string().is_null() {
                res = Some(cstr_to_cstring((*lv.ll_tv).vval.v_string()));
                *pp = end.cast_mut().cast();
            } else if (*lv.ll_tv).v_type == VarType::VarPartial
                && !(*lv.ll_tv).vval.v_partial().is_null()
            {
                let pt = (*lv.ll_tv).vval.v_partial();
                if is_luafunc(pt) && crate::memory::cstr_first_byte(end.cast()) == b'.' {
                    len = check_luafunc_name(end.wrapping_add(1).cast(), true);
                    if len == 0 {
                        semsg!(E_INVEXPR2, c"v:lua");
                        return None;
                    }
                    res = Some(cstr_from_bytes(std::slice::from_raw_parts(
                        end.wrapping_add(1),
                        len as usize,
                    )));
                    *pp = end.cast_mut().wrapping_add(1 + len as usize).cast();
                } else {
                    res = Some(cstr_to_cstring(partial_name(pt.as_ref())));
                    *pp = end.cast_mut().cast();
                }
                if !partial.is_null() {
                    *partial = pt;
                }
            } else if !skip
                && (flags & TFN_QUIET) == 0
                && (fdp.is_none() || lv.ll_dict.is_null() || fdp.unwrap().fd_newkey.is_none())
            {
                emsg(gettext(E_FUNCREF));
            } else {
                *pp = end.cast_mut().cast();
            }
            return res;
        }

        if lv.ll_name.is_null() {
            *pp = end.cast_mut().cast();
            return None;
        }

        if let Some(ref exp_name) = lv.ll_exp_name {
            let exp_ptr: *const u8 = exp_name.as_ptr().cast();
            len = exp_name.as_bytes().len() as i32;
            name = deref_func_name(
                exp_ptr,
                &mut len,
                partial,
                (flags & TFN_NO_AUTOLOAD) != 0,
                None,
            );
            if ptr::eq(name, exp_ptr) {
                name = ptr::null_mut();
            }
        } else if (flags & TFN_NO_DEREF) == 0 {
            len = crate::memory::ptr_offset_from(end, (*pp).cast()) as i32;
            name = deref_func_name(
                (*pp).cast::<u8>(),
                &mut len,
                partial,
                (flags & TFN_NO_AUTOLOAD) != 0,
                None,
            );
            if ptr::eq(name, (*pp).cast()) {
                name = ptr::null_mut();
            }
        }
        if !name.is_null() {
            *pp = end.cast_mut().cast();
            if crate::memory::cstr_prefix_bytes(name.cast(), 5) == &b"<SNR>"[..5] {
                let tail = crate::memory::cstr_as_bytes(name.wrapping_add(5).cast());
                let mut bytes = Vec::with_capacity(3 + tail.len());
                bytes.extend_from_slice(&[K_SPECIAL, KS_EXTRA, KE_SNR]);
                bytes.extend_from_slice(tail);
                return Some(cstr_from_bytes(&bytes));
            }
            return Some(cstr_to_cstring(name.cast()));
        }

        if let Some(ref exp_name) = lv.ll_exp_name {
            len = exp_name.as_bytes().len() as i32;
            if lead <= 2
                && lv.ll_name == exp_name.as_ptr().cast()
                && lv.ll_name_len >= 2
                && (crate::memory::cstr_prefix_bytes(lv.ll_name.cast(), 2) == &b"s:"[..2])
            {
                lv.ll_name = lv.ll_name.wrapping_add(2);
                lv.ll_name_len -= 2;
                len -= 2;
                lead = 2;
            }
        } else {
            if lead == 2
                || (crate::memory::cstr_first_byte(lv.ll_name.cast()) == b'g'
                    && crate::memory::cstr_byte_at(lv.ll_name.cast(), 1) == b':')
            {
                lv.ll_name = lv.ll_name.wrapping_add(2);
                lv.ll_name_len = lv.ll_name_len.wrapping_sub(2);
            }
            len = crate::memory::ptr_offset_from(end.cast(), lv.ll_name) as i32;
        }

        let mut sid_buflen = 0usize;
        let mut sid_buf = [0u8; 20];

        if skip {
            lead = 0;
        } else if lead > 0 {
            lead = 3;
            if lv
                .ll_exp_name
                .as_ref()
                .is_some_and(|s| eval_fname_sid(s.as_ptr()))
                || eval_fname_sid((*pp).cast())
            {
                if g_st().current_sctx.sc_sid <= 0 {
                    emsg(gettext(E_USINGSID));
                    return None;
                }
                sid_buflen = vim_snprintf!(
                    sid_buf.as_mut_ptr().cast(),
                    sid_buf.len(),
                    c"%d_",
                    g_st().current_sctx.sc_sid,
                ) as usize;
                lead += sid_buflen as i32;
            }
        } else if (flags & TFN_INT) == 0
            && builtin_function(lv.ll_name.cast(), lv.ll_name_len as i32)
        {
            semsg!(
                gettext(c"E128: Function name must start with a capital or \"s:\": %s"),
                start,
            );
            return None;
        }

        if !skip && (flags & TFN_QUIET) == 0 && (flags & TFN_NO_DEREF) == 0 {
            let cp: *const u8 = {
                let _s = (lv.ll_name).cast::<u8>();
                let _sz = lv.ll_name_len;
                if _sz == 0 {
                    std::ptr::null()
                } else {
                    std::slice::from_raw_parts(_s, _sz)
                        .iter()
                        .rposition(|&_b| _b == b':')
                        .map_or(std::ptr::null(), |_i| _s.wrapping_add(_i))
                }
            };
            if !cp.is_null() && cp < end {
                semsg!(
                    gettext(c"E884: Function name cannot contain a colon: %s"),
                    start,
                );
                return None;
            }
        }

        let mut bytes = Vec::with_capacity(len as usize + lead as usize);
        if !skip && lead > 0 {
            bytes.extend_from_slice(&[K_SPECIAL, KS_EXTRA, KE_SNR]);
            if sid_buflen > 0 {
                bytes.extend_from_slice(&sid_buf[..sid_buflen]);
            }
        }
        bytes.extend_from_slice(std::slice::from_raw_parts(
            lv.ll_name.cast::<u8>(),
            len as usize,
        ));
        *pp = end.cast_mut().cast();

        Some(cstr_from_bytes(&bytes))
    }
}

pub fn get_scriptlocal_funcname(
    funcname: impl crate::strings::IntoCCharPtr,
) -> Option<std::ffi::CString> {
    let funcname = funcname.into_c_char_ptr();
    {
        if funcname.is_null() {
            return None;
        }

        if !(crate::memory::cstr_prefix_bytes(funcname, 2) == &b"s:"[..2])
            && !(crate::memory::cstr_prefix_bytes(funcname, 5) == &b"<SID>"[..5])
        {
            return None;
        }

        if !crate::runtime::script_id_valid(g_st().current_sctx.sc_sid) {
            emsg(gettext(E_USINGSID));
            return None;
        }

        let mut sid_buf = [0u8; 25];
        let sid_buflen = vim_snprintf!(
            sid_buf.as_mut_ptr().cast(),
            sid_buf.len(),
            c"<SNR>%d_",
            g_st().current_sctx.sc_sid,
        ) as usize;
        let off = if crate::memory::cstr_first_byte(funcname) == b's' {
            2
        } else {
            5
        };
        let newnamesize = sid_buflen + crate::memory::cstr_len(funcname.wrapping_add(off)) + 1;
        let mut newname = vec![0u8; newnamesize];
        vim_snprintf!(
            newname.as_mut_ptr().cast(),
            newnamesize,
            c"%s%s",
            sid_buf.as_ptr(),
            funcname.wrapping_add(off),
        );

        Some(cstr_to_cstring(newname.as_ptr().cast()))
    }
}

/// Call `trans_function_name()`, except that a lambda is returned as-is.
pub fn save_function_name<T>(
    name: *mut *mut T,
    skip: bool,
    flags: i32,
    mut fudi: Option<&mut Funcdict>,
) -> Option<std::ffi::CString> {
    let mut p = unsafe { (*name).cast::<u8>() };
    let saved: Option<std::ffi::CString>;

    if crate::memory::cstr_prefix_bytes(p.cast(), 8) == &b"<lambda>"[..8] {
        p = p.wrapping_add(8);
        getdigits((&raw mut p).cast(), false, 0);
        saved = Some(cstr_from_bytes(crate::memory::cstr_slice(
            unsafe { (*name).cast() },
            crate::memory::ptr_offset_from(p, unsafe { (*name).cast() }) as usize,
        )));
        if fudi.is_some() {
            *fudi.as_deref_mut().unwrap() = Funcdict::new();
        }
    } else {
        saved = trans_function_name(
            &raw mut p,
            skip,
            flags,
            (fudi.is_some()).then(|| fudi.unwrap()),
            ptr::null_mut(),
        );
    }
    unsafe {
        *name = p.cast();
    }
    saved
}

fn list_functions(mut regmatch: Option<&mut Regmatch>) {
    let prev_ht_changed = uf_st().func_hashtab_changed;
    let funcs: Vec<*mut UserFunc> = uf_st().func_hashtab.values().copied().collect();

    msg_ext_set_kind(c"list_cmd");
    for fp in funcs {
        if g_st().got_int {
            break;
        }
        let matches = if let Some(ref mut regmatch) = regmatch {
            !uf2hikey(unsafe { &*fp }).to_bytes_with_nul()[0].is_ascii_digit()
                && vim_regexec(regmatch, uf2hikey(unsafe { &*fp }).as_ptr(), 0)
        } else {
            !message_filtered(uf2hikey(unsafe { &*fp }))
                && !func_name_refcount(uf2hikey(unsafe { &*fp }))
        };
        if matches {
            if !list_func_head(unsafe { &mut *fp }, false, false) {
                return;
            }
            if function_list_modified(prev_ht_changed) {
                return;
            }
        }
    }
}

fn list_functions_matching_pat(eap: &mut Exarg) -> *mut u8 {
    let mut p = skip_regexp(eap.arg.wrapping_add(1), b'/' as i32, 1);
    if eap.skip == 0 {
        let mut regmatch: Regmatch = Regmatch::new();

        let c = unsafe { *p };
        unsafe {
            *p = NUL;
        }
        regmatch.regprog = vim_regcomp(eap.arg.wrapping_add(1), RE_MAGIC);
        unsafe {
            *p = c;
        }
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = opt_vars().p_ic != 0;
            list_functions(Some(&mut regmatch));
            vim_regfree(regmatch.regprog);
        }
    }
    if crate::memory::cstr_first_byte(p) == b'/' {
        p = p.wrapping_add(1);
    }

    p.cast()
}

fn list_one_function<T>(
    eap: &mut Exarg,
    name: impl crate::strings::IntoCCharPtr,
    p: *mut T,
) -> *mut UserFunc {
    let p = p.cast::<u8>();
    let name = name.into_c_char_ptr();
    {
        if ends_excmd(crate::memory::cstr_first_char(skipwhite(p.cast()))) == 0 {
            semsg!(
                gettext(E_TRAILING_ARG),
                p.cast::<std::ffi::c_void>()
            );
            return ptr::null_mut();
        }

        eap.nextcmd = check_nextcmd(p.cast());

        if !eap.nextcmd.is_null() {
            unsafe {
                *p = 0;
            }
        }

        if eap.skip != 0 || g_st().got_int {
            return ptr::null_mut();
        }

        let fp = find_func(name);
        if fp.is_null() {
            emsg_funcname(c"E123: Undefined function: %s", name);
            return ptr::null_mut();
        }
        let fp = unsafe { &mut *fp };

        let prev_ht_changed = uf_st().func_hashtab_changed;

        msg_ext_set_kind(c"list_cmd");
        if !list_func_head(&mut *fp, eap.forceit == 0, eap.forceit != 0) {
            return fp;
        }

        let mut j = 0;
        while j < fp.uf_lines.len() as i32 && !g_st().got_int {
            let Some(ref line_cstr) = (&fp.uf_lines)[j as usize] else {
                j += 1;
                continue;
            };
            let line = line_cstr.as_ptr();
            msg_putchar(b'\n' as i32);
            if eap.forceit == 0 {
                msg_outnum(j + 1);
                if j < 9 {
                    msg_putchar(b' ' as i32);
                }
                if j < 99 {
                    msg_putchar(b' ' as i32);
                }
                if function_list_modified(prev_ht_changed) {
                    break;
                }
            }
            msg_prt_line(line, false);
            line_breakcheck();
            j += 1;
        }

        if !g_st().got_int {
            msg_putchar(b'\n' as i32);
            if !function_list_modified(prev_ht_changed) {
                msg_puts(if eap.forceit != 0 {
                    c"endfunction".as_ptr()
                } else {
                    c"   endfunction".as_ptr()
                });
            }
        }

        fp
    }
}

fn get_function_body(
    eap: &mut Exarg,
    newlines: &mut Vec<Option<std::ffi::CString>>,
    line_arg_in: impl crate::strings::IntoCCharPtr,
    show_block: bool,
) -> bool {
    let line_arg_in = line_arg_in.into_c_char_ptr().cast_mut();
    unsafe {
        let mut saved_wait_return = g_st().need_wait_return;
        let mut line_arg = line_arg_in;
        let mut line_to_free: Option<std::ffi::CString> = None;
        let mut indent: i32 = 2;
        let mut nesting: i32 = 0;
        let mut skip_until: Option<std::ffi::CString> = None;
        let mut ret = false;
        let mut is_heredoc = false;
        let mut heredoc_trimmed: Option<std::ffi::CString> = None;
        let mut heredoc_trimmedlen: usize = 0;
        let mut do_concat = true;

        loop {
            if g_st().key_typed {
                g_st().msg_scroll = 1;
                saved_wait_return = false;
            }
            g_st().need_wait_return = false;

            let theline;
            let mut p;
            let mut arg;

            if !line_arg.is_null() {
                theline = line_arg;
                p = vim_strchr(theline, b'\n' as i32);
                if p.is_null() {
                    line_arg = line_arg.wrapping_add(crate::memory::cstr_len(line_arg));
                } else {
                    *p = NUL;
                    line_arg = p.wrapping_add(1);
                }
            } else {
                line_to_free = if eap.ea_getline.is_none() {
                    getcmdline(b':' as i32, 0, indent, do_concat)
                } else {
                    eap.ea_getline.call(b':' as i32, indent, do_concat)
                };
                theline = line_to_free
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            }
            if g_st().key_typed {
                g_st().lines_left = g_st().rows_global - 1;
            }
            if theline.is_null() {
                if let Some(ref marker) = skip_until {
                    semsg!(gettext(E_MISSING_HEREDOC_END_MARKER_STR), marker.as_ptr(),);
                } else {
                    emsg(gettext(c"E126: Missing :endfunction"));
                }
                g_st().need_wait_return |= saved_wait_return;
                return ret;
            }
            if show_block {
                debug_assert!(indent >= 0);
                ui_ext_cmdline_block_append(indent as usize, theline);
            }

            let mut sourcing_lnum_off = get_sourced_lnum(eap.ea_getline);
            let cur_sourcing_lnum = sourcing_lnum();
            if cur_sourcing_lnum < sourcing_lnum_off {
                sourcing_lnum_off -= cur_sourcing_lnum;
            } else {
                sourcing_lnum_off = 0;
            }

            if let Some(ref marker) = skip_until {
                if heredoc_trimmed.is_none()
                    || (is_heredoc && skipwhite(theline) == theline)
                    || heredoc_trimmed.as_ref().is_some_and(|ht| {
                        crate::memory::cstr_prefix_bytes(theline, heredoc_trimmedlen)
                            == crate::memory::cstr_prefix_bytes(ht.as_ptr(), heredoc_trimmedlen)
                    })
                {
                    p = if heredoc_trimmed.is_none() {
                        theline
                    } else if is_heredoc {
                        if skipwhite(theline) == theline {
                            theline
                        } else {
                            theline.wrapping_add(heredoc_trimmedlen)
                        }
                    } else {
                        theline.wrapping_add(heredoc_trimmedlen)
                    };
                    if crate::memory::cstr_as_bytes(p)
                        == crate::memory::cstr_as_bytes(marker.as_ptr())
                    {
                        skip_until = None;
                        heredoc_trimmed = None;
                        heredoc_trimmedlen = 0;
                        do_concat = true;
                        is_heredoc = false;
                    }
                }
            } else {
                p = theline;
                while ascii_iswhite(crate::memory::cstr_first_char(p))
                    || crate::memory::cstr_first_byte(p) == b':'
                {
                    p = p.wrapping_add(1);
                }

                if checkforcmd(&raw mut p, c"endfunction", 4) {
                    let prev_nesting = nesting;
                    nesting -= 1;
                    if prev_nesting == 0 {
                        if crate::memory::cstr_first_byte(p) == b'!' {
                            p = p.wrapping_add(1);
                        }
                        let mut nextcmd = ptr::null_mut();
                        if crate::memory::cstr_first_byte(p) == b'|' {
                            nextcmd = p.wrapping_add(1);
                        } else if !line_arg.is_null() && *skipwhite(line_arg) != NUL {
                            nextcmd = line_arg;
                        } else if !crate::memory::cstr_is_empty(p)
                            && crate::memory::cstr_first_byte(p) != b'"'
                            && opt_vars().p_verbose > 0
                        {
                            swmsg!(true, gettext(c"W22: Text found after :endfunction: %s"), p,);
                        }
                        if !nextcmd.is_null() {
                            if let Some(ref ltf) = line_to_free {
                                let off = (nextcmd as usize) - (ltf.as_ptr() as usize);
                                let new_buf: Vec<u8> = ltf.as_bytes_with_nul().to_vec();
                                eap.set_cmdline_vec(eap.cmdlinep, new_buf);
                                eap.nextcmd = (*eap.cmdlinep).wrapping_add(off);
                            } else {
                                eap.nextcmd = nextcmd;
                            }
                        }
                        break;
                    }
                }

                if indent > 2 && (crate::memory::cstr_prefix_bytes(p, 3) == &b"end"[..3]) {
                    indent -= 2;
                } else if (crate::memory::cstr_prefix_bytes(p, 2) == &b"if"[..2])
                    || (crate::memory::cstr_prefix_bytes(p, 2) == &b"wh"[..2])
                    || (crate::memory::cstr_prefix_bytes(p, 3) == &b"for"[..3])
                    || (crate::memory::cstr_prefix_bytes(p, 3) == &b"try"[..3])
                {
                    indent += 2;
                }

                if checkforcmd(&raw mut p, c"function", 2) {
                    if crate::memory::cstr_first_byte(p) == b'!' {
                        p = skipwhite(p.wrapping_add(1));
                    }
                    p = p.wrapping_add(eval_fname_script(p) as usize);
                    let _ = trans_function_name(&raw mut p, true, 0, None, ptr::null_mut());
                    if crate::memory::cstr_first_byte(skipwhite(p)) == b'(' {
                        if nesting == MAX_FUNC_NESTING - 1 {
                            emsg(gettext(E_FUNCTION_NESTING_TOO_DEEP));
                        } else {
                            nesting += 1;
                            indent += 2;
                        }
                    }
                }

                p = skip_range(p, None);
                let tp = p;
                if (checkforcmd(&raw mut p, c"append", 1)
                    || checkforcmd(&raw mut p, c"change", 1)
                    || checkforcmd(&raw mut p, c"insert", 1))
                    && (crate::memory::cstr_first_byte(p) == b'!'
                        || crate::memory::cstr_first_byte(p) == b'|'
                        || ascii_iswhite_nl_or_nul(crate::memory::cstr_first_char(p)))
                {
                    skip_until = Some(c".".to_owned());
                } else {
                    p = tp;
                }

                arg = skipwhite(skiptowhite(p));
                let p0 = crate::memory::cstr_first_byte(p);
                let p1 = if p0 != 0 {
                    crate::memory::cstr_byte_at(p, 1)
                } else {
                    0
                };
                let p2 = if p1 != 0 {
                    crate::memory::cstr_byte_at(p, 2)
                } else {
                    0
                };
                let p3 = if p2 != 0 {
                    crate::memory::cstr_byte_at(p, 3)
                } else {
                    0
                };
                let is_script_heredoc = crate::memory::cstr_first_byte(arg) == b'<'
                    && crate::memory::cstr_byte_at(arg, 1) == b'<'
                    && ((p0 == b'p'
                        && p1 == b'y'
                        && (!ascii_isalnum(p2 as i32)
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
                    p = skipwhite(arg.wrapping_add(2));
                    if (crate::memory::cstr_prefix_bytes(p, 4) == &b"trim"[..4])
                        && (crate::memory::cstr_byte_at(p, 4) == 0
                            || ascii_iswhite(crate::memory::cstr_char_at(p, 4)))
                    {
                        p = skipwhite(p.wrapping_add(4));
                        heredoc_trimmedlen =
                            crate::memory::ptr_offset_from(skipwhite(theline), theline) as usize;
                        let slice = crate::memory::cstr_slice(theline, heredoc_trimmedlen);
                        heredoc_trimmed = Some(std::ffi::CString::new(slice).unwrap());
                    }
                    if crate::memory::cstr_is_empty(p) {
                        skip_until = Some(c".".to_owned());
                    } else {
                        let slice = crate::memory::cstr_slice(
                            p,
                            crate::memory::ptr_offset_from(skiptowhite(p), p) as usize,
                        );
                        skip_until = Some(std::ffi::CString::new(slice).unwrap());
                    }
                    do_concat = false;
                    is_heredoc = true;
                }

                if !is_heredoc {
                    arg = p;
                    if checkforcmd(&raw mut arg, c"let", 2) || checkforcmd(&raw mut p, c"const", 5)
                    {
                        let mut var_count = 0;
                        let mut semicolon = 0;
                        arg = skip_var_list(arg, &mut var_count, &mut semicolon, true).cast_mut();
                        if !arg.is_null() {
                            arg = skipwhite(arg);
                        }
                        if !arg.is_null()
                            && (crate::memory::cstr_prefix_bytes(arg, 3) == &b"=<<"[..3])
                        {
                            p = skipwhite(arg.wrapping_add(3));
                            let mut has_trim = false;
                            loop {
                                if (crate::memory::cstr_prefix_bytes(p, 4) == &b"trim"[..4])
                                    && (crate::memory::cstr_byte_at(p, 4) == 0
                                        || ascii_iswhite(crate::memory::cstr_char_at(p, 4)))
                                {
                                    p = skipwhite(p.wrapping_add(4));
                                    has_trim = true;
                                    continue;
                                }
                                if (crate::memory::cstr_prefix_bytes(p, 4) == &b"eval"[..4])
                                    && (crate::memory::cstr_byte_at(p, 4) == 0
                                        || ascii_iswhite(crate::memory::cstr_char_at(p, 4)))
                                {
                                    p = skipwhite(p.wrapping_add(4));
                                    continue;
                                }
                                break;
                            }
                            if has_trim {
                                heredoc_trimmedlen =
                                    crate::memory::ptr_offset_from(skipwhite(theline), theline)
                                        as usize;
                                let slice = std::slice::from_raw_parts(
                                    theline.cast::<u8>(),
                                    heredoc_trimmedlen,
                                );
                                heredoc_trimmed = Some(std::ffi::CString::new(slice).unwrap());
                            }
                            let slice = crate::memory::cstr_slice(
                                p,
                                crate::memory::ptr_offset_from(skiptowhite(p), p) as usize,
                            );
                            skip_until = Some(std::ffi::CString::new(slice).unwrap());
                            do_concat = false;
                            is_heredoc = true;
                        }
                    }
                }
            }

            newlines.push(Some(crate::memory::cstr_to_cstring(theline)));

            while sourcing_lnum_off > 0 {
                sourcing_lnum_off -= 1;
                newlines.push(None);
            }

            if !line_arg.is_null() && crate::memory::cstr_is_empty(line_arg) {
                line_arg = ptr::null_mut();
            }
        }

        if g_st().did_emsg == 0 {
            ret = true;
        }

        g_st().need_wait_return |= saved_wait_return;
        ret
    }
}

/// `:function`
pub fn ex_function(eap: &mut Exarg) {
    unsafe {
        let mut line_arg = ptr::null_mut();
        let mut newargs: Vec<std::ffi::CString> = Vec::new();
        let mut default_args: Vec<std::ffi::CString> = Vec::new();
        let mut newlines: Vec<Option<std::ffi::CString>> = Vec::new();
        let mut varargs: i32 = 0;
        let mut flags: i32 = 0;
        let mut fp: *mut UserFunc = ptr::null_mut();
        let mut free_fp = false;
        let mut overwrite = false;
        let mut fudi = Funcdict::new();
        let mut ht: *mut Hashtab = ptr::null_mut();
        let mut show_block = false;

        if ends_excmd(crate::memory::cstr_first_char(eap.arg)) != 0 {
            if eap.skip == 0 {
                list_functions(None);
            }
            eap.nextcmd = check_nextcmd(eap.arg);
            return;
        }

        if crate::memory::cstr_first_byte(eap.arg) == b'/' {
            let p = list_functions_matching_pat(eap);
            eap.nextcmd = check_nextcmd(p.cast());
            return;
        }

        let mut p = eap.arg;
        let mut name_owned =
            save_function_name(&raw mut p, eap.skip != 0, TFN_NO_AUTOLOAD, Some(&mut fudi));
        let mut name = name_owned
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let paren = !vim_strchr(p, b'(' as i32).is_null();
        if name.is_null() && (fudi.fd_dict.is_null() || !paren) && eap.skip == 0 {
            if !aborting() {
                if let Some(ref newkey) = fudi.fd_newkey {
                    semsg!(gettext(E_DICTKEY), newkey.as_ptr());
                }
                return;
            }
            eap.skip = 1;
        }

        let saved_did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;

        macro_rules! ret_free {
            () => {{
                drop(name_owned);
                g_st().did_emsg |= saved_did_emsg;
                if show_block {
                    ui_ext_cmdline_block_leave();
                }
                return;
            }};
        }

        macro_rules! errret_keep {
            () => {{
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
        if crate::memory::cstr_first_byte(p) != b'(' {
            if eap.skip == 0 {
                semsg!(gettext(c"E124: Missing '(': %s"), eap.arg);
                ret_free!();
            }
            let open_paren = vim_strchr(p, b'(' as i32);
            if !open_paren.is_null() {
                p = open_paren;
            }
        }
        p = skipwhite(p.wrapping_add(1));

        if eap.skip == 0 {
            let fudi_newkey_ptr = fudi
                .fd_newkey
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            let arg = if !name.is_null() {
                name
            } else {
                fudi_newkey_ptr
            };
            if !arg.is_null() && (fudi.fd_di.is_null() || !tv_is_func(&(*fudi.fd_di).di_tv)) {
                let mut name_base = arg;
                if arg != fudi_newkey_ptr {
                    if crate::memory::cstr_first_byte(arg) == K_SPECIAL {
                        name_base = vim_strchr(arg, b'_' as i32);
                        if name_base.is_null() {
                            name_base = arg.wrapping_add(3);
                        } else {
                            name_base = name_base.wrapping_add(1);
                        }
                    }
                    let mut i = 0usize;
                    while crate::memory::cstr_byte_at(name_base, i) != 0
                        && if i == 0 {
                            eval_isnamec1(crate::memory::cstr_char_at(name_base, i))
                        } else {
                            eval_isnamec(crate::memory::cstr_char_at(name_base, i))
                        }
                    {
                        i += 1;
                    }
                    if crate::memory::cstr_byte_at(name_base, i) != 0 {
                        emsg_funcname(E_INVARG2, arg);
                        ret_free!();
                    }
                }
            }
            if !fudi.fd_dict.is_null() && (*fudi.fd_dict).dv_scope == VAR_DEF_SCOPE {
                emsg(gettext(c"E862: Cannot use g: here"));
                ret_free!();
            }
        }

        if !get_function_args(
            &raw mut p,
            b')',
            Some(&mut newargs),
            Some(&mut varargs),
            Some(&mut default_args),
            eap.skip != 0,
        ) {
            errret_2!();
        }

        if g_st().key_typed && ui_has(K_UICMDLINE) {
            show_block = true;
            ui_ext_cmdline_block_append(0, eap.cmd);
        }

        loop {
            p = skipwhite(p);
            if crate::memory::cstr_prefix_bytes(p, 5) == &b"range"[..5] {
                flags |= FC_RANGE;
                p = p.wrapping_add(5);
            } else if crate::memory::cstr_prefix_bytes(p, 4) == &b"dict"[..4] {
                flags |= FC_DICT;
                p = p.wrapping_add(4);
            } else if crate::memory::cstr_prefix_bytes(p, 5) == &b"abort"[..5] {
                flags |= FC_ABORT;
                p = p.wrapping_add(5);
            } else if crate::memory::cstr_prefix_bytes(p, 7) == &b"closure"[..7] {
                flags |= FC_CLOSURE;
                p = p.wrapping_add(7);
                if uf_st().current_funccal.is_null() {
                    emsg_funcname(
                        c"E932: Closure function should not be at top level: %s",
                        if name.is_null() { c"".as_ptr() } else { name },
                    );
                    erret!();
                }
            } else {
                break;
            }
        }

        if crate::memory::cstr_first_byte(p) == b'\n' {
            line_arg = p.wrapping_add(1);
        } else if !crate::memory::cstr_is_empty(p)
            && crate::memory::cstr_first_byte(p) != b'"'
            && eap.skip == 0
            && g_st().did_emsg == 0
        {
            semsg!(gettext(E_TRAILING_ARG), p);
        }

        if g_st().key_typed {
            if eap.skip == 0 && eap.forceit == 0 {
                if !fudi.fd_dict.is_null() && fudi.fd_newkey.is_none() {
                    emsg(gettext(E_FUNCDICT));
                } else if !name.is_null() && !find_func(name).is_null() {
                    emsg_funcname(E_FUNCEXTS, name);
                }
            }

            if eap.skip == 0 && g_st().did_emsg != 0 {
                erret!();
            }

            if !ui_has(K_UICMDLINE) {
                msg_putchar(b'\n' as i32);
            }
            g_st().cmdline_row = g_st().msg_row;
        }

        let sourcing_lnum_top = sourcing_lnum();

        if !get_function_body(eap, &mut newlines, line_arg, show_block) || eap.skip != 0 {
            erret!();
        }

        let mut namelen = 0usize;
        if fudi.fd_dict.is_null() {
            let v = find_var(name, crate::memory::cstr_len(name), &raw mut ht, 0);
            if !v.is_null() && (*v).di_tv.v_type == VarType::VarFunc {
                emsg_funcname(c"E707: Function name conflicts with variable: %s", name);
                erret!();
            }

            fp = find_func(name);
            if !fp.is_null() {
                if eap.forceit == 0
                    && ((*fp).uf_script_ctx.sc_sid != g_st().current_sctx.sc_sid
                        || (*fp).uf_script_ctx.sc_seq == g_st().current_sctx.sc_seq)
                {
                    emsg_funcname(E_FUNCEXTS, name);
                    errret_keep!();
                }
                if (*fp).uf_calls > 0 {
                    emsg_funcname(c"E127: Cannot redefine function %s: It is in use", name);
                    errret_keep!();
                }
                if (*fp).uf_refcount > 1 {
                    (*fp).uf_refcount -= 1;
                    (*fp).uf_flags |= FC_REMOVED;
                    fp = ptr::null_mut();
                    overwrite = true;
                } else {
                    let exp_name = (*fp).uf_name_exp.take();
                    name_owned = None;
                    name = ptr::null_mut();
                    func_clear_items(&mut *fp);
                    (*fp).uf_name_exp = exp_name;
                    (*fp).uf_profiling = 0;
                    (*fp).uf_prof_initialized = 0;
                }
            }
        } else {
            let mut numbuf = [0u8; NUMBUFLEN];

            fp = ptr::null_mut();
            if fudi.fd_newkey.is_none() && eap.forceit == 0 {
                emsg(gettext(E_FUNCDICT));
                erret!();
            }
            if fudi.fd_di.is_null() {
                if value_check_lock((*fudi.fd_dict).dv_lock, eap.arg, TV_CSTRING) {
                    erret!();
                }
            } else if value_check_lock((*fudi.fd_di).di_tv.v_lock, eap.arg, TV_CSTRING) {
                erret!();
            }

            uf_st().func_nr += 1;
            namelen =
                vim_snprintf!(numbuf.as_mut_ptr().cast(), NUMBUFLEN, c"%d", uf_st().func_nr)
                    as usize;
            let nm = cstr_from_bytes(&numbuf[..namelen]);
            name = nm.as_ptr().cast_mut();
            name_owned = Some(nm);
        }

        if fp.is_null() {
            if fudi.fd_dict.is_null() && !vim_strchr(name, AUTOLOAD_CHAR as i32).is_null() {
                let mut j = FAIL;
                if let Some(sname) = sourcing_name() {
                    let sn = autoload_name(name, crate::memory::cstr_len(name));
                    p = vim_strchr(sn.as_ptr(), b'/' as i32);
                    let plen = crate::memory::cstr_len(p) as i32;
                    let slen = sname.to_bytes().len() as i32;
                    if slen > plen
                        && path_equal(
                            p,
                            sname.as_ptr().wrapping_add((slen - plen) as usize),
                            K_PATH_CMP_LITERAL,
                        )
                    {
                        j = OK;
                    }
                }
                if j == FAIL {
                    semsg!(
                        gettext(c"E746: Function name does not match script file name: %s",),
                        name,
                    );
                    erret!();
                }
            }

            if namelen == 0 {
                namelen = crate::memory::cstr_len(name);
            }
            fp = alloc_ufunc(name, namelen);

            if !fudi.fd_dict.is_null() {
                if fudi.fd_di.is_null() {
                    fudi.fd_di = tv_dict_item_alloc(fudi.fd_newkey.as_ref().unwrap().as_ptr());
                    if !tv_dict_add(&mut *fudi.fd_dict, fudi.fd_di) {
                        crate::eval::typval::tv_dict_item_free(fudi.fd_di);
                        erret!();
                    }
                } else {
                    tv_clear(&mut (*fudi.fd_di).di_tv);
                }
                crate::eval::typval::tv_set_string_len(&mut (*fudi.fd_di).di_tv, name, namelen);
                (*fudi.fd_di).di_tv.v_type = VarType::VarFunc;

                flags |= FC_DICT;
            }

            if overwrite {
                uf_st()
                    .func_hashtab
                    .insert(crate::memory::cstr_as_bytes(name).to_vec(), fp);
            } else if !func_hashtab_add(&mut *fp) {
                free_fp = true;
                erret!();
            }
            (*fp).uf_refcount = 1;
        }
        let fp = &mut *fp;
        fp.uf_args = newargs;
        fp.uf_def_args = default_args;
        fp.uf_lines = newlines;
        if (flags & FC_CLOSURE) != 0 {
            register_closure(&mut *fp);
        } else {
            fp.uf_scoped = ptr::null_mut();
        }
        if prof_def_func() {
            func_do_profile(fp);
        }
        fp.uf_varargs = varargs;
        if g_st().sandbox != 0 {
            flags |= FC_SANDBOX;
        }
        fp.uf_flags = flags;
        fp.uf_calls = 0;
        fp.uf_script_ctx = g_st().current_sctx;
        fp.uf_script_ctx.sc_lnum += sourcing_lnum_top;
        nlua_set_sctx(&mut fp.uf_script_ctx);

        ret_free!();
    }
}

/// Return 5 if `p` starts with `<SID>` or `<SNR>` (ignoring case),
/// 2 if `p` starts with `s:`, 0 otherwise.
pub fn eval_fname_script(p: impl crate::strings::IntoCCharPtr) -> i32 {
    let p = p.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(p) == b'<'
            && (mb_strnicmp(p.wrapping_add(1), c"SID>", 4) == 0
                || mb_strnicmp(p.wrapping_add(1), c"SNR>", 4) == 0)
        {
            return 5;
        }
        if crate::memory::cstr_first_byte(p) == b's' && crate::memory::cstr_byte_at(p, 1) == b':' {
            return 2;
        }
        0
    }
}

pub fn translated_function_exists(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    if builtin_function(name, -1) {
        return !find_internal_func(name).is_null();
    }
    !find_func(name).is_null()
}

pub fn function_exists(name: impl crate::strings::IntoCCharPtr, no_deref: bool) -> bool {
    let name = name.into_c_char_ptr();
    {
        let mut nm = name.cast_mut();
        let mut n = false;
        let mut flag = TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD;

        if no_deref {
            flag |= TFN_NO_DEREF;
        }
        let p = trans_function_name(&raw mut nm, false, flag, None, ptr::null_mut());
        nm = skipwhite(nm);

        if let Some(ref p) = p
            && (crate::memory::cstr_is_empty(nm) || crate::memory::cstr_first_byte(nm) == b'(')
        {
            n = translated_function_exists(p.as_ptr());
        }
        n
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user defined
/// function names.
pub fn get_user_func_name(xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx == 0 {
        uf_st().get_uf_done = 0;
        uf_st().get_uf_items = uf_st().func_hashtab.values().copied().collect();
        uf_st().get_uf_changed = uf_st().func_hashtab_changed;
    }
    if uf_st().get_uf_changed == uf_st().func_hashtab_changed
        && uf_st().get_uf_done < uf_st().get_uf_items.len()
    {
        let fp = uf_st().get_uf_items.as_slice()[uf_st().get_uf_done];
        let fp = unsafe { &mut *fp };
        uf_st().get_uf_done += 1;

        if (fp.uf_flags & FC_DICT) != 0 || uf2hikey(&*fp).to_bytes().starts_with(b"<lambda>") {
            return Some(c"");
        }

        if fp.uf_namelen + 4 >= IOSIZE {
            return Some(uf2hikey(&*fp));
        }

        let iobuff_ptr = g_st().iobuff.as_mut_ptr();
        let mut len = cat_func_name(iobuff_ptr, IOSIZE, &*fp);
        if xp.xp_context != EXPAND_USER_FUNC {
            xstrlcpy(
                iobuff_ptr.wrapping_add(len as usize),
                c"(",
                IOSIZE - len as usize,
            );
            if fp.uf_varargs == 0 && fp.uf_args.is_empty() {
                len += 1;
                xstrlcpy(
                    iobuff_ptr.wrapping_add(len as usize),
                    c")",
                    IOSIZE - len as usize,
                );
            }
        }
        return Some(crate::memory::cstr_from_ptr(iobuff_ptr));
    }
    None
}

/// `:delfunction {name}`
pub fn ex_delfunction(eap: &mut Exarg) {
    let mut fp: *mut UserFunc = ptr::null_mut();
    let mut fudi = Funcdict::new();

    let mut p = eap.arg;
    let name_owned = trans_function_name(
        &raw mut p,
        eap.skip != 0,
        0,
        Some(&mut fudi),
        ptr::null_mut(),
    );
    let Some(ref name_cstr) = name_owned else {
        if !fudi.fd_dict.is_null() && eap.skip == 0 {
            emsg(gettext(E_FUNCREF));
        }
        return;
    };
    let name = name_cstr.as_ptr();
    if ends_excmd(unsafe { *skipwhite(p) } as i32) == 0 {
        semsg!(gettext(E_TRAILING_ARG), p);
        return;
    }
    eap.nextcmd = check_nextcmd(p);
    if !eap.nextcmd.is_null() {
        unsafe {
            *p = NUL;
        }
    }

    if (crate::memory::cstr_first_byte(name)).is_ascii_digit() && fudi.fd_dict.is_null() {
        if eap.skip == 0 {
            semsg!(gettext(E_INVARG2), eap.arg);
        }
        return;
    }
    if eap.skip == 0 {
        fp = find_func(name);
    }
    drop(name_owned);

    if eap.skip == 0 {
        if fp.is_null() {
            if eap.forceit == 0 {
                semsg!(gettext(E_NOFUNC), eap.arg);
            }
            return;
        }
        if unsafe { (*fp).uf_calls } > 0 {
            semsg!(
                gettext(c"E131: Cannot delete function %s: It is in use"),
                eap.arg,
            );
            return;
        }
        if unsafe { (*fp).uf_refcount } > 2 {
            semsg!(
                gettext(c"Cannot delete function %s: It is being used internally"),
                eap.arg,
            );
            return;
        }

        if !fudi.fd_dict.is_null() {
            tv_dict_item_remove(unsafe { &mut *fudi.fd_dict }, unsafe { &mut *fudi.fd_di });
        } else {
            let threshold = if func_name_refcount(uf2hikey(unsafe { &*fp })) {
                0
            } else {
                1
            };
            if unsafe { (*fp).uf_refcount } > threshold {
                if func_remove(unsafe { &mut *fp }) {
                    unsafe {
                        (*fp).uf_refcount -= 1;
                    }
                }
                unsafe {
                    (*fp).uf_flags |= FC_DELETED;
                }
            } else {
                func_clear_free(fp, false);
            }
        }
    }
}

/// Unreference a Function: decrement the reference count and free it when it
/// becomes zero.
pub fn func_unref(name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    {
        if name.is_null() || !func_name_refcount(name) {
            return;
        }

        let fp = find_func(name);
        if fp.is_null()
            && (crate::memory::cstr_first_byte(name)).is_ascii_digit()
            && !mem_st().entered_free_all_mem
        {
            internal_error(c"func_unref()");
            std::process::abort();
        }
        func_ptr_unref(fp);
    }
}

pub fn func_ptr_unref(fp: *mut UserFunc) {
    if !fp.is_null() {
        let fp = unsafe { &mut *fp };
        fp.uf_refcount -= 1;
        if fp.uf_refcount <= 0 && fp.uf_calls == 0 {
            func_clear_free(fp, false);
        }
    }
}

pub fn func_ref(name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    {
        if name.is_null() || !func_name_refcount(name) {
            return;
        }
        let fp = find_func(name);
        if !fp.is_null() {
            unsafe {
                (*fp).uf_refcount += 1;
            }
        } else if (crate::memory::cstr_first_byte(name)).is_ascii_digit() {
            internal_error(c"func_ref()");
        }
    }
}

pub fn func_ptr_ref(fp: Option<&mut UserFunc>) {
    if let Some(fp) = fp {
        fp.uf_refcount += 1;
    }
}

#[inline]
fn fc_referenced(fc: &Funccall) -> bool {
    fc.fc_l_varlist.lv_refcount != DO_NOT_FREE_CNT
        || fc.fc_l_vars.dv_refcount != DO_NOT_FREE_CNT
        || fc.fc_l_avars.dv_refcount != DO_NOT_FREE_CNT
        || fc.fc_refcount > 0
}

fn can_free_funccal(fc: &Funccall, copy_id: i32) -> bool {
    fc.fc_l_varlist.lv_copy_id != copy_id
        && fc.fc_l_vars.dv_copy_id != copy_id
        && fc.fc_l_avars.dv_copy_id != copy_id
        && fc.fc_copy_id != copy_id
}

/// `:return [expr]`
pub fn ex_return(eap: &mut Exarg) {
    let arg = eap.arg;
    let mut rettv: TypVal = TypVal::new();
    let mut returning = false;

    if uf_st().current_funccal.is_null() {
        emsg(gettext(c"E133: :return not inside a function"));
        return;
    }

    let mut evalarg = Evalarg {
        eval_flags: if eap.skip != 0 { 0 } else { EVAL_EVALUATE },
    };

    if eap.skip != 0 {
        g_st().emsg_skip += 1;
    }

    eap.nextcmd = ptr::null_mut();
    if (!crate::memory::cstr_is_empty(arg)
        && crate::memory::cstr_first_byte(arg) != b'|'
        && crate::memory::cstr_first_byte(arg) != b'\n')
        && eval0(arg, &mut rettv, Some(&mut *eap), &mut evalarg)
    {
        if eap.skip == 0 {
            returning = do_return(eap, false, true, Some(rettv));
        } else {
            tv_clear(&mut rettv);
        }
    } else if eap.skip == 0 {
        update_force_abort();

        if !aborting() {
            returning = do_return(eap, false, true, None);
        }
    }

    if returning {
        eap.nextcmd = ptr::null_mut();
    } else if eap.nextcmd.is_null() {
        eap.nextcmd = check_nextcmd(arg);
    }

    if eap.skip != 0 {
        g_st().emsg_skip -= 1;
    }
}

fn ex_call_inner<T>(
    eap: &mut Exarg,
    name: impl crate::strings::IntoCCharPtr,
    arg: *mut *mut T,
    startarg: impl crate::strings::IntoCCharPtr,
    funcexe_init: &Funcexe,
    evalarg: &mut Evalarg,
) -> bool {
    let startarg = startarg.into_c_char_ptr().cast_mut();
    let name = name.into_c_char_ptr();
    {
        let mut failed = false;

        for lnum in eap.line1..=eap.line2 {
            if eap.addr_count > 0 {
                if lnum > curbuf_mut().b_ml.ml_line_count {
                    emsg(gettext(E_INVRANGE));
                    break;
                }
                curwin_mut().w_cursor.lnum = lnum;
                curwin_mut().w_cursor.col = 0;
                curwin_mut().w_cursor.coladd = 0;
            }
            unsafe {
                *arg = startarg.cast();
            }

            let mut funcexe = *funcexe_init;
            funcexe.fe_doesrange = false;
            let mut rettv: TypVal = TypVal::new();
            rettv.v_type = VarType::VarUnknown;
            if !get_func_tv(name, -1, &mut rettv, arg, Some(evalarg), &mut funcexe) {
                failed = true;
                break;
            }

            if !handle_subscript(
                arg.cast(),
                &mut rettv,
                Some(&mut g_st().evalarg_evaluate),
                true,
            ) {
                failed = true;
                break;
            }

            tv_clear(&mut rettv);
            if funcexe.fe_doesrange {
                break;
            }

            if aborting() {
                break;
            }
        }

        failed
    }
}

fn ex_defer_inner<T>(
    name: impl crate::strings::IntoCCharPtr,
    arg: *mut *mut T,
    partial: Option<&TvPartial>,
    evalarg: &mut Evalarg,
) -> bool {
    let name = name.into_c_char_ptr();
    {
        let mut argvars: [TypVal; MAX_FUNC_ARGS + 1] = [TypVal::new(); MAX_FUNC_ARGS + 1];
        let mut partial_argc = 0;
        let mut argcount = 0;

        if uf_st().current_funccal.is_null() {
            semsg!(gettext(E_STR_NOT_INSIDE_FUNCTION), c"defer",);
            return false;
        }
        if let Some(partial) = partial {
            if !partial.pt_dict.is_null() {
                emsg(gettext(E_CANNOT_USE_PARTIAL_WITH_DICTIONARY_FOR_DEFER));
                return false;
            }
            if !partial.pt_argv.is_empty() {
                partial_argc = partial.pt_argv.len() as i32;
                for i in 0..partial_argc {
                    tv_copy(&(&partial.pt_argv)[i as usize], &mut argvars[i as usize]);
                }
            }
        }
        let mut r = get_func_arguments(
            arg,
            Some(evalarg),
            0,
            argvars.as_mut_ptr().wrapping_add(partial_argc as usize),
            &mut argcount,
        );
        argcount += partial_argc;

        if r {
            if builtin_function(name, -1) {
                let fdef = find_internal_func(name);
                if fdef.is_null() {
                    emsg_funcname(E_UNKNOWN_FUNCTION_STR, name);
                    r = false;
                } else if check_internal_func(unsafe { &*fdef }, argcount) == -1 {
                    r = false;
                }
            } else {
                let ufunc = find_func(name);
                if !ufunc.is_null() {
                    let error = check_user_func_argcount(unsafe { &mut *ufunc }, argcount);
                    if error != FCERR_UNKNOWN {
                        user_func_error(error, name, false);
                        r = false;
                    }
                }
            }
        }

        if !r {
            while argcount > 0 {
                argcount -= 1;
                tv_clear(&mut argvars[argcount as usize]);
            }
            return false;
        }
        add_defer(name, argcount, argvars.as_mut_ptr());
        true
    }
}

pub fn can_add_defer() -> bool {
    if get_current_funccal().is_null() {
        semsg!(gettext(E_STR_NOT_INSIDE_FUNCTION), c"defer",);
        return false;
    }
    true
}

pub fn add_defer(name: impl crate::strings::IntoCCharPtr, argcount_arg: i32, argvars: *mut TypVal) {
    let name = name.into_c_char_ptr().cast_mut();
    {
        let dr_name_cstr = cstr_to_cstring(name);
        let argcount = argcount_arg as usize;
        let mut dr_argvars = vec![TypVal::new(); argcount + 1];
        for (i, slot) in dr_argvars.iter_mut().enumerate().take(argcount) {
            *slot = unsafe { *argvars.wrapping_add(i) };
        }
        cur_funccal_mut().fc_defer.push(DeferCall {
            dr_name: Some(dr_name_cstr),
            dr_argvars,
        });
    }
}

fn handle_defer_one(funccal: &mut Funccall) {
    let mut idx = funccal.fc_defer.len() as isize - 1;
    while idx >= 0 {
        let dr = &mut (&mut funccal.fc_defer)[idx as usize];

        let Some(name_cstr) = dr.dr_name.take() else {
            idx -= 1;
            continue;
        };

        let mut funcexe = Funcexe::new();
        funcexe.fe_evaluate = true;

        let mut rettv: TypVal = TypVal::new();
        rettv.v_type = VarType::VarUnknown;

        let argcount = (dr.dr_argvars.len() - 1) as i32;

        let mut estate = ExceptionState::new();
        exception_state_save(&mut estate);
        exception_state_clear();

        call_func(
            name_cstr.as_ptr(),
            -1,
            &mut rettv,
            argcount,
            dr.dr_argvars.as_mut_ptr(),
            &mut funcexe,
        );

        exception_state_restore(&mut estate);

        tv_clear(&mut rettv);
        drop(name_cstr);
        let mut i = argcount - 1;
        while i >= 0 {
            let dr = &mut (&mut funccal.fc_defer)[idx as usize];
            tv_clear(&mut dr.dr_argvars[i as usize]);
            i -= 1;
        }
        (&mut funccal.fc_defer)[idx as usize].dr_argvars.clear();
        idx -= 1;
    }
    funccal.fc_defer.clear();
}

pub struct FunccalCallerMutIter(*mut Funccall);

impl Iterator for FunccalCallerMutIter {
    type Item = &'static mut Funccall;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0;
        if cur.is_null() {
            None
        } else {
            unsafe {
                self.0 = (*cur).fc_caller;
                Some(&mut *cur)
            }
        }
    }
}

pub struct FunccalScopedMutIter(*mut Funccall);

impl Iterator for FunccalScopedMutIter {
    type Item = &'static mut Funccall;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0;
        if cur.is_null() {
            None
        } else {
            unsafe {
                self.0 = (*(*cur).fc_func).uf_scoped;
                Some(&mut *cur)
            }
        }
    }
}

pub fn invoke_all_defer() {
    for fc in FunccalCallerMutIter(uf_st().current_funccal) {
        handle_defer_one(fc);
    }

    for &top in uf_st().funccal_stack.iter().rev() {
        for fc2 in FunccalCallerMutIter(top) {
            handle_defer_one(fc2);
        }
    }
}

/// `:1,25call func(arg1, arg2)` or `:defer func(arg1, arg2)`.
pub fn ex_call(eap: &mut Exarg) {
    let mut arg = eap.arg;
    let mut fudi = Funcdict::new();
    let mut partial: *mut TvPartial = ptr::null_mut();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip != 0);
    if eap.skip != 0 {
        let mut rettv: TypVal = TypVal::new();
        g_st().emsg_skip += 1;
        if eval0(eap.arg, &mut rettv, Some(&mut *eap), &mut evalarg) {
            tv_clear(&mut rettv);
        }
        g_st().emsg_skip -= 1;
        return;
    }

    let tofree_owned = trans_function_name(
        &raw mut arg,
        false,
        TFN_INT,
        Some(&mut fudi),
        &raw mut partial,
    );
    if let Some(ref newkey) = fudi.fd_newkey {
        semsg!(gettext(E_DICTKEY), newkey.as_ptr());
    }
    let Some(ref tofree_cstr) = tofree_owned else {
        return;
    };
    let tofree = tofree_cstr.as_ptr();

    if !fudi.fd_dict.is_null() {
        unsafe {
            (*fudi.fd_dict).dv_refcount += 1;
        }
    }

    let mut len = tofree_cstr.as_bytes().len() as i32;
    let mut found_var = false;
    let name = deref_func_name(
        tofree,
        &mut len,
        if !partial.is_null() {
            ptr::null_mut()
        } else {
            &raw mut partial
        },
        false,
        Some(&mut found_var),
    );

    let startarg = skipwhite(arg);

    if crate::memory::cstr_first_byte(startarg) != b'(' {
        semsg!(gettext(E_MISSINGPAREN), eap.arg);
        tv_dict_unref(fudi.fd_dict);
        return;
    }

    let failed = if eap.cmdidx == Cmdidx::CmdDefer as i32 {
        arg = startarg;
        !ex_defer_inner(
            name,
            &raw mut arg,
            unsafe { partial.as_ref() },
            &mut evalarg,
        )
    } else {
        let mut funcexe = Funcexe::new();
        funcexe.fe_partial = partial;
        funcexe.fe_selfdict = fudi.fd_dict;
        funcexe.fe_firstline = eap.line1;
        funcexe.fe_lastline = eap.line2;
        funcexe.fe_found_var = found_var;
        funcexe.fe_evaluate = true;
        ex_call_inner(eap, name, &raw mut arg, startarg, &funcexe, &mut evalarg)
    };

    if (!aborting() || g_st().did_throw)
        && (!failed || unsafe { (*eap.cstack.cast::<Cstack>()).cs_trylevel } > 0)
    {
        if ends_excmd(crate::memory::cstr_first_char(arg)) == 0 {
            if !failed && !aborting() {
                g_st().emsg_severe = true;
                semsg!(gettext(E_TRAILING_ARG), arg);
            }
        } else {
            eap.nextcmd = check_nextcmd(arg);
        }
    }

    tv_dict_unref(fudi.fd_dict);
    drop(tofree_owned);
}

/// Return from a function. Possibly makes the return pending.
pub fn do_return(
    eap: &mut Exarg,
    reanimate: bool,
    _is_cmd: bool,
    mut rettv: Option<TypVal>,
) -> bool {
    let cstack = eap.cstack_mut();

    if reanimate {
        cur_funccal_mut().fc_returned = 0;
    }

    let idx = cleanup_conditionals(&mut *cstack, 0, 1);
    if idx >= 0 {
        cstack.cs_pending[idx as usize] = CSTP_RETURN as u8;

        if reanimate {
            debug_assert!(!cur_funccal_mut().fc_rettv.is_null());
            rettv = Some(unsafe { *cur_funccal_mut().fc_rettv });
            unsafe {
                (*cur_funccal_mut().fc_rettv).v_type = VarType::VarNumber;
            }
            unsafe {
                (*cur_funccal_mut().fc_rettv).vval = crate::eval::typval::TypvalVval::Number(0);
            }
        }

        cstack.cs_rettv[idx as usize] = rettv;
        report_make_pending(CSTP_RETURN, PendingVal::Return(rettv));
    } else {
        cur_funccal_mut().fc_returned = 1;

        if !reanimate && let Some(tv) = rettv {
            tv_clear(unsafe { &mut *cur_funccal_mut().fc_rettv });
            unsafe {
                *cur_funccal_mut().fc_rettv = tv;
            }
        }
    }

    idx < 0
}

/// Generate a return command for producing the value of `rettv`.
pub fn get_return_cmd(rettv: Option<TypVal>) -> std::ffi::CString {
    let mut tofree: Option<std::ffi::CString> = None;
    if let Some(mut tv) = rettv {
        tofree = Some(encode_tv2echo(&mut tv, None));
    }
    let (s, slen) = if let Some(ref tf) = tofree {
        (tf.as_ptr(), tf.as_bytes().len())
    } else {
        (c"".as_ptr(), 0)
    };

    let iobuff_ptr = g_st().iobuff.as_mut_ptr();
    xstrlcpy(iobuff_ptr, c":return ", IOSIZE);
    xstrlcpy(iobuff_ptr.wrapping_add(8), s, IOSIZE - 8);
    let mut iobuff_len = 8 + slen;
    if iobuff_len >= IOSIZE {
        crate::memory::cstr_cpy(iobuff_ptr.wrapping_add(IOSIZE - 4), c"...");
        iobuff_len = IOSIZE - 1;
    }
    drop(tofree);
    cstr_from_bytes(unsafe { std::slice::from_raw_parts(iobuff_ptr.cast::<u8>(), iobuff_len) })
}

/// Get next function line.
pub fn get_func_line(
    _c: i32,
    fcp: &mut Funccall,
    _indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    let fp = fcp.fc_func;
    let retval: Option<std::ffi::CString>;

    if fcp.fc_dbg_tick != g_st().debug_tick {
        fcp.fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(unsafe { &*fp }), sourcing_lnum());
        fcp.fc_dbg_tick = g_st().debug_tick;
    }
    if g_st().do_profiling == PROF_YES {
        func_line_end(fcp);
    }

    let lines = unsafe { &(*fp).uf_lines };
    let lines_len = lines.len() as i32;
    if ((unsafe { (*fp).uf_flags } & FC_ABORT) != 0 && g_st().did_emsg != 0 && !aborted_in_try())
        || fcp.fc_returned != 0
    {
        retval = None;
    } else {
        while fcp.fc_linenr < lines_len && lines[fcp.fc_linenr as usize].is_none() {
            fcp.fc_linenr += 1;
        }
        if fcp.fc_linenr >= lines_len {
            retval = None;
        } else {
            let line = lines[fcp.fc_linenr as usize].clone();
            fcp.fc_linenr += 1;
            retval = line;
            set_sourcing_lnum(fcp.fc_linenr);
            if g_st().do_profiling == PROF_YES {
                func_line_start(fcp);
            }
        }
    }

    let cur_lnum = sourcing_lnum();
    if fcp.fc_breakpoint != 0 && fcp.fc_breakpoint <= cur_lnum {
        dbg_breakpoint(uf2hikey(unsafe { &*fp }), cur_lnum);
        fcp.fc_breakpoint = dbg_find_breakpoint(false, uf2hikey(unsafe { &*fp }), sourcing_lnum());
        fcp.fc_dbg_tick = g_st().debug_tick;
    }

    retval
}

pub fn func_has_ended(fcp: &mut Funccall) -> i32 {
    if ((unsafe { (*fcp.fc_func).uf_flags } & FC_ABORT) != 0
        && g_st().did_emsg != 0
        && !aborted_in_try())
        || fcp.fc_returned != 0
    {
        1
    } else {
        0
    }
}

pub fn func_has_abort(fcp: &mut Funccall) -> i32 {
    unsafe { (*fcp.fc_func).uf_flags & FC_ABORT }
}

/// Turn `dict.Func` into a partial for `Func` bound to `dict`.
pub fn make_partial(selfdict: &mut TvDict, rettv: &mut TypVal) {
    unsafe {
        let mut fp: *mut UserFunc = ptr::null_mut();
        let mut fname_buf = [0u8; FLEN_FIXED + 1];
        let mut error = 0;

        if rettv.v_type == VarType::VarPartial
            && !rettv.vval.v_partial().is_null()
            && !(*rettv.vval.v_partial()).pt_func.is_null()
        {
            fp = (*rettv.vval.v_partial()).pt_func;
        } else {
            let mut fname: *mut u8 =
                if rettv.v_type == VarType::VarFunc || rettv.v_type == VarType::VarString {
                    rettv.vval.v_string().cast()
                } else if rettv.vval.v_partial().is_null() {
                    ptr::null_mut()
                } else {
                    (*rettv.vval.v_partial())
                        .pt_name
                        .as_ref()
                        .map_or(ptr::null_mut(), |n| n.as_ptr().cast_mut().cast())
                };
            if fname.is_null() {
                rettv.v_type = VarType::VarFunc;
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            } else {
                let mut tofree: Option<std::ffi::CString> = None;
                fname = fname_trans_sid(
                    fname.cast(),
                    fname_buf.as_mut_ptr(),
                    &mut tofree,
                    &mut error,
                );
                fp = find_func(fname.cast());
            }
        }

        if !fp.is_null() && ((*fp).uf_flags & FC_DICT) != 0 {
            let pt = Box::leak(Box::new(TvPartial::default()));
            pt.pt_refcount = 1;
            pt.pt_dict = (&raw mut *selfdict);
            selfdict.dv_refcount += 1;
            pt.pt_auto = true;
            if rettv.v_type == VarType::VarFunc || rettv.v_type == VarType::VarString {
                pt.pt_name = crate::eval::typval::tv_take_string(rettv.vval.v_string());
            } else {
                let ret_pt = rettv.vval.v_partial();
                let ret_pt = &mut *ret_pt;
                if let Some(ref ret_name) = ret_pt.pt_name {
                    func_ref(ret_name.as_ptr().cast_mut());
                    pt.pt_name = Some(ret_name.clone());
                } else {
                    pt.pt_func = ret_pt.pt_func;
                    func_ptr_ref((pt.pt_func).as_mut());
                }
                if !ret_pt.pt_argv.is_empty() {
                    let pt_argc = ret_pt.pt_argv.len();
                    pt.pt_argv.resize(pt_argc, TypVal::new());
                    for i in 0..pt_argc {
                        tv_copy(&(&ret_pt.pt_argv)[i], &mut (&mut pt.pt_argv)[i]);
                    }
                }
                partial_unref(ret_pt);
            }
            rettv.v_type = VarType::VarPartial;
            rettv.vval = crate::eval::typval::TypvalVval::Partial(pt);
        }
    }
}

pub fn func_name(fcp: &mut Funccall) -> &'static CStr {
    unsafe { uf2hikey(&*fcp.fc_func) }
}

pub fn func_breakpoint(fcp: &mut Funccall) -> *mut Linenr {
    &raw mut fcp.fc_breakpoint
}

pub fn func_dbg_tick(fcp: &mut Funccall) -> *mut i32 {
    &raw mut fcp.fc_dbg_tick
}

pub fn func_level(fcp: &mut Funccall) -> i32 {
    fcp.fc_level
}

pub fn current_func_returned() -> i32 {
    cur_funccal_mut().fc_returned
}

pub fn free_unref_funccal(copy_id: i32, testing: i32) -> bool {
    let mut did_free = false;
    let mut did_free_funccal = false;

    let mut i = 0usize;
    while i < uf_st().previous_funccal.len() {
        let fc = &*uf_st().previous_funccal.as_slice()[i].0;
        if can_free_funccal(fc, copy_id) {
            uf_st().previous_funccal.remove(i);
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

pub fn get_funccal() -> *mut Funccall {
    let mut funccal = uf_st().current_funccal;
    if g_st().debug_backtrace_level > 0 {
        for i in 0..g_st().debug_backtrace_level {
            let temp_funccal = unsafe { (*funccal).fc_caller };
            if !temp_funccal.is_null() {
                funccal = temp_funccal;
            } else {
                g_st().debug_backtrace_level = i;
                break;
            }
        }
    }
    funccal
}

fn get_funccal_for_vars() -> *mut Funccall {
    if uf_st().current_funccal.is_null() {
        return ptr::null_mut();
    }
    let funccal = get_funccal();
    if funccal.is_null() || unsafe { (*funccal).fc_l_vars.dv_refcount } == 0 {
        return ptr::null_mut();
    }
    funccal
}

pub fn get_funccal_local_dict() -> *mut TvDict {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            &raw mut (*funccal).fc_l_vars
        }
    }
}

pub fn get_funccal_local_ht() -> *mut Hashtab {
    unsafe {
        let d = get_funccal_local_dict();
        if !d.is_null() {
            &raw mut (*d).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

pub fn get_funccal_local_var() -> *mut Dictitem {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            (&raw mut (*funccal).fc_l_vars_var).cast()
        }
    }
}

pub fn get_funccal_args_dict() -> *mut TvDict {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            &raw mut (*funccal).fc_l_avars
        }
    }
}

pub fn get_funccal_args_ht() -> *mut Hashtab {
    unsafe {
        let d = get_funccal_args_dict();
        if !d.is_null() {
            &raw mut (*d).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

pub fn get_funccal_args_var() -> *mut Dictitem {
    unsafe {
        let funccal = get_funccal_for_vars();
        if funccal.is_null() {
            ptr::null_mut()
        } else {
            (&raw mut (*funccal).fc_l_avars_var).cast()
        }
    }
}

pub fn list_func_vars(first: &mut i32) {
    if !uf_st().current_funccal.is_null() && cur_funccal_mut().fc_l_vars.dv_refcount > 0 {
        list_hashtable_vars(
            &mut cur_funccal_mut().fc_l_vars.dv_hashtab,
            c"l:",
            0,
            &mut *first,
        );
    }
}

pub fn get_current_funccal_dict(ht: *mut Hashtab) -> *mut TvDict {
    if !uf_st().current_funccal.is_null() && ht == &raw mut cur_funccal_mut().fc_l_vars.dv_hashtab {
        return &raw mut cur_funccal_mut().fc_l_vars;
    }
    ptr::null_mut()
}

pub fn find_hi_in_scoped_ht(
    name: impl crate::strings::IntoCCharPtr,
    pht: *mut *mut Hashtab,
) -> *mut Hashitem {
    let name = name.into_c_char_ptr();
    unsafe {
        if uf_st().current_funccal.is_null() || (*cur_funccal_mut().fc_func).uf_scoped.is_null() {
            return ptr::null_mut();
        }

        let old_current_funccal = uf_st().current_funccal;
        let mut hi: *mut Hashitem = ptr::null_mut();
        let namelen = crate::memory::cstr_len(name);
        let mut varname = ptr::null();

        uf_st().current_funccal = (*cur_funccal_mut().fc_func).uf_scoped;
        while !uf_st().current_funccal.is_null() {
            let ht = find_var_ht(name, namelen, &raw mut varname);
            if !ht.is_null() && !crate::memory::cstr_is_empty(varname) {
                hi = hash_find_len(
                    &*ht,
                    varname,
                    namelen - crate::memory::ptr_offset_from(varname, name) as usize,
                );
                if !hashitem_empty(&*hi) {
                    *pht = ht;
                    break;
                }
            }
            if uf_st().current_funccal == (*cur_funccal_mut().fc_func).uf_scoped {
                break;
            }
            uf_st().current_funccal = (*cur_funccal_mut().fc_func).uf_scoped;
        }
        uf_st().current_funccal = old_current_funccal;

        hi
    }
}

pub fn find_var_in_scoped_ht(
    name: impl crate::strings::IntoCCharPtr,
    namelen: usize,
    no_autoload: i32,
) -> *mut Dictitem {
    let name = name.into_c_char_ptr();
    unsafe {
        if uf_st().current_funccal.is_null() || (*cur_funccal_mut().fc_func).uf_scoped.is_null() {
            return ptr::null_mut();
        }

        let mut v: *mut Dictitem = ptr::null_mut();
        let old_current_funccal = uf_st().current_funccal;
        let mut varname = ptr::null();

        uf_st().current_funccal = (*cur_funccal_mut().fc_func).uf_scoped;
        while !uf_st().current_funccal.is_null() {
            let ht = find_var_ht(name, namelen, &raw mut varname);
            if !ht.is_null() && !crate::memory::cstr_is_empty(varname) {
                v = find_var_in_ht(
                    &mut *ht,
                    crate::memory::cstr_first_char(name),
                    varname,
                    namelen - crate::memory::ptr_offset_from(varname, name) as usize,
                    no_autoload,
                );
                if !v.is_null() {
                    break;
                }
            }
            if uf_st().current_funccal == (*cur_funccal_mut().fc_func).uf_scoped {
                break;
            }
            uf_st().current_funccal = (*cur_funccal_mut().fc_func).uf_scoped;
        }
        uf_st().current_funccal = old_current_funccal;

        v
    }
}

pub fn set_ref_in_previous_funccal(copy_id: i32) -> bool {
    for item in uf_st().previous_funccal.as_mut_slice() {
        let fc = &mut *item.0;
        fc.fc_copy_id = copy_id + 1;
        if set_ref_in_ht(
            &raw mut fc.fc_l_vars.dv_hashtab,
            copy_id + 1,
            ptr::null_mut(),
        ) || set_ref_in_ht(
            &raw mut fc.fc_l_avars.dv_hashtab,
            copy_id + 1,
            ptr::null_mut(),
        ) || set_ref_in_list_items(&raw mut fc.fc_l_varlist, copy_id + 1, ptr::null_mut())
        {
            return true;
        }
    }
    false
}

/// Set `copy_id` in the arguments of the deferred functions of `fc`.
/// Returns `true` if setting references failed somehow.
fn set_ref_in_defer(fc: &mut Funccall, copy_id: i32) -> bool {
    for dr in &mut fc.fc_defer {
        let argcount = dr.dr_argvars.len().saturating_sub(1);
        for tv in dr.dr_argvars.iter_mut().take(argcount) {
            if set_ref_in_item(tv, copy_id, ptr::null_mut(), ptr::null_mut()) {
                return true;
            }
        }
    }
    false
}

fn set_ref_in_funccal(fc: &mut Funccall, copy_id: i32) -> bool {
    if fc.fc_copy_id != copy_id {
        fc.fc_copy_id = copy_id;
        if set_ref_in_ht(&raw mut fc.fc_l_vars.dv_hashtab, copy_id, ptr::null_mut())
            || set_ref_in_ht(&raw mut fc.fc_l_avars.dv_hashtab, copy_id, ptr::null_mut())
            || set_ref_in_list_items(&raw mut fc.fc_l_varlist, copy_id, ptr::null_mut())
            || set_ref_in_defer(fc, copy_id)
            || set_ref_in_func(ptr::null_mut(), fc.fc_func, copy_id)
        {
            return true;
        }
    }
    false
}

pub fn set_ref_in_call_stack(copy_id: i32) -> bool {
    for fc in FunccalCallerMutIter(uf_st().current_funccal) {
        if set_ref_in_funccal(fc, copy_id) {
            return true;
        }
    }

    for &top in uf_st().funccal_stack.iter().rev() {
        for fc2 in FunccalCallerMutIter(top) {
            if set_ref_in_funccal(fc2, copy_id) {
                return true;
            }
        }
    }

    false
}

pub fn set_ref_in_functions(copy_id: i32) -> bool {
    for &fp in uf_st().func_hashtab.values() {
        if g_st().got_int {
            break;
        }
        if !func_name_refcount(uf2hikey(unsafe { &*fp }))
            && set_ref_in_func(ptr::null_mut(), fp, copy_id)
        {
            return true;
        }
    }
    false
}

pub fn set_ref_in_func_args(copy_id: i32) -> bool {
    for &tv in &uf_st().funcargs {
        if set_ref_in_item(
            unsafe { &mut *tv },
            copy_id,
            ptr::null_mut(),
            ptr::null_mut(),
        ) {
            return true;
        }
    }
    false
}

pub fn set_ref_in_func(
    name: impl crate::strings::IntoCCharPtr,
    fp_in: *mut UserFunc,
    copy_id: i32,
) -> bool {
    let name = name.into_c_char_ptr();
    {
        let mut fp = fp_in;
        let mut error = FCERR_NONE;
        let mut fname_buf = [0u8; FLEN_FIXED + 1];
        let mut tofree: Option<std::ffi::CString> = None;
        let mut abort = false;
        if name.is_null() && fp_in.is_null() {
            return false;
        }

        if fp_in.is_null() {
            let fname = fname_trans_sid(name, fname_buf.as_mut_ptr(), &mut tofree, &mut error);
            fp = find_func(fname.cast());
        }
        if !fp.is_null() {
            for fc in FunccalScopedMutIter(unsafe { (*fp).uf_scoped }) {
                abort = abort || set_ref_in_funccal(fc, copy_id);
            }
        }
        drop(tofree);
        abort
    }
}

/// Registers a luaref as a lambda.
pub fn register_luafunc(ref_: LuaRef) -> &'static std::ffi::CStr {
    let name = get_lambda_name();
    let fp = alloc_ufunc(name.data, name.size);

    fp.uf_refcount = 1;
    fp.uf_varargs = 1;
    fp.uf_flags = FC_LUAREF;
    fp.uf_calls = 0;
    fp.uf_script_ctx = g_st().current_sctx;
    fp.uf_luaref = ref_;

    func_hashtab_add(&mut *fp);

    uf2hikey(&*fp)
}

#[inline]
pub fn cur_funccal_mut() -> &'static mut Funccall {
    unsafe { &mut *uf_st().current_funccal }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_function_and_func_name_helpers_work() {
        assert!(eval_fname_sid(c"s:Foo"));
        assert!(eval_fname_sid(c"<SID>Foo"));
        assert!(eval_fname_sid(c"<sid>Foo"));
        assert!(!eval_fname_sid(c"<SNR>12_Foo"));
        assert!(func_name_refcount(c"123"));
        assert!(func_name_refcount(c"<lambda>1"));
        assert!(!func_name_refcount(c"MyFunc"));
        assert!(builtin_function(c"abs", -1));
        assert!(!builtin_function(c"s:abs", -1));
        assert!(!builtin_function(c"foo#bar", -1));
        assert!(!builtin_function(c"Foo", -1));
    }
}
