//! User defined function support (`src/nvim/eval/userfunc.c`).
#![forbid(unsafe_code)]

use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_FUNCUNDEFINED, PROF_YES, apply_autocmds,
};
use crate::charset::{NUL, ascii_isalnum, ascii_islower, ascii_iswhite, getdigits, vim_strsize};
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
    Callback, DictId, Dictitem, DictitemId, EvalFuncData, TV_CSTRING, TvDict, TvPartial, TypVal,
    VAR_UNLOCKED, VarType, Varnumber, tv_clear, tv_copy, tv_dict_add, tv_dict_alloc_static,
    tv_dict_free_slot, tv_dict_item_alloc, tv_dict_item_alloc_len, tv_dict_item_alloc_static,
    tv_dict_item_free_slot, tv_dict_item_remove, tv_dict_unref, tv_get_number_chk, tv_is_func,
    tv_list_alloc_static, tv_list_append, tv_list_free_contents, tv_list_item_alloc,
    tv_list_set_lock, unref_var_list, value_check_lock,
};
use crate::eval::vars::{
    Evalarg, Lval, VV_TESTING, find_var, find_var_ht, find_var_in_ht, get_vim_var_nr,
    init_var_dict, list_hashtable_vars, skip_var_list, unref_var_dict, vars_clear, vars_clear_ext,
};
use crate::eval::{
    AUTOLOAD_CHAR, EVAL_EVALUATE, FNE_CHECK_START, FNE_INCL_BR, GLV_READ_ONLY, TFN_NO_AUTOLOAD,
    TFN_QUIET, VAR_DEF_SCOPE, callback_call, check_luafunc_name, eval_isnamec, eval_isnamec1,
    eval_lavars_used, eval0, eval1, fill_evalarg_from_eap, find_name_end, garbage_collect,
    get_id_len, get_lval, handle_subscript, is_luafunc, last_set_msg, partial_name, partial_unref,
    set_eval_lavars_used, set_ref_in_ht, set_ref_in_item, set_ref_in_list_items, skip_expr,
};
use crate::ex_docmd::{Exarg, check_nextcmd, checkforcmd, do_cmdline, ends_excmd, skip_range};
use crate::ex_eval::{
    CSTP_RETURN, ExceptionState, PendingVal, aborted_in_try, aborting, cleanup_conditionals,
    exception_state_clear, exception_state_restore, exception_state_save, report_make_pending,
    update_force_abort,
};
use crate::ex_getln::{getcmdline, ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave};
use crate::globals::*;
use crate::hashtab::{Hashtab, hash_add, hash_find_len_idx, hashitem_empty};
use crate::input::{RedoState, restore_redobuff, save_redobuff};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_SPECIAL, KE_SNR, KS_EXTRA};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{
    api_free_luaref, nlua_call_vlua, nlua_exec_typval_callable, nlua_set_sctx,
};
use crate::memory::{bytes_to_cstring, entered_free_all_mem};
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
use crate::regexp::{NUMBUFLEN, RE_MAGIC, Regmatch, skip_regexp, vim_regcomp, vim_regexec};
use crate::runtime::{
    autoload_name, estack_pop, estack_push_ufunc, get_sourced_lnum, script_autoload,
    set_sourcing_lnum, sourcing_lnum, sourcing_name,
};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::strings::{format_to_buf, gettext, vim_strchr};
use crate::ui::ui_has;
use std::ffi::CStr;
use std::ptr;

pub type VimLFunc = Option<fn(&mut [TypVal], &mut TypVal, EvalFuncData)>;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct UfuncId(Option<std::num::NonZeroU32>);

impl UfuncId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn from_slot(idx: usize) -> Self {
        Self(std::num::NonZeroU32::new((idx as u32) + 1))
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn rc(self) -> Option<std::rc::Rc<UserFunc>> {
        let idx = (self.0?.get() - 1) as usize;
        UFUNC_ARENA
            .try_with(|a| a.borrow().get(idx).and_then(|s| s.clone()))
            .ok()
            .flatten()
    }

    #[inline(always)]
    pub fn unwrap_rc(self) -> std::rc::Rc<UserFunc> {
        self.rc().expect("null or freed UfuncId")
    }
}

impl From<&UserFunc> for UfuncId {
    #[inline(always)]
    fn from(fp: &UserFunc) -> Self {
        fp.uf_id
    }
}

impl From<&std::rc::Rc<UserFunc>> for UfuncId {
    #[inline(always)]
    fn from(fp: &std::rc::Rc<UserFunc>) -> Self {
        fp.uf_id
    }
}

impl From<std::rc::Rc<UserFunc>> for UfuncId {
    #[inline(always)]
    fn from(fp: std::rc::Rc<UserFunc>) -> Self {
        fp.uf_id
    }
}

impl From<Option<&UserFunc>> for UfuncId {
    #[inline(always)]
    fn from(opt: Option<&UserFunc>) -> Self {
        opt.map_or(Self::NULL, |fp| fp.uf_id)
    }
}

impl From<Option<std::rc::Rc<UserFunc>>> for UfuncId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<UserFunc>>) -> Self {
        opt.map_or(Self::NULL, |fp| fp.uf_id)
    }
}

impl PartialEq<UserFunc> for UfuncId {
    #[inline(always)]
    fn eq(&self, other: &UserFunc) -> bool {
        *self == other.uf_id
    }
}

impl PartialEq<&UserFunc> for UfuncId {
    #[inline(always)]
    fn eq(&self, other: &&UserFunc) -> bool {
        *self == other.uf_id
    }
}

impl PartialEq<std::rc::Rc<UserFunc>> for UfuncId {
    #[inline(always)]
    fn eq(&self, other: &std::rc::Rc<UserFunc>) -> bool {
        *self == other.uf_id
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FunccalId(Option<std::num::NonZeroU32>);

impl FunccalId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn from_slot(idx: usize) -> Self {
        Self(std::num::NonZeroU32::new((idx as u32) + 1))
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn rc(self) -> Option<std::rc::Rc<Funccall>> {
        let idx = (self.0?.get() - 1) as usize;
        FUNCCAL_ARENA
            .try_with(|a| a.borrow().get(idx).and_then(|s| s.clone()))
            .ok()
            .flatten()
    }

    #[inline(always)]
    pub fn unwrap_rc(self) -> std::rc::Rc<Funccall> {
        self.rc().expect("null or freed FunccalId")
    }
}

impl From<&Funccall> for FunccalId {
    #[inline(always)]
    fn from(fc: &Funccall) -> Self {
        fc.fc_id
    }
}

impl From<&std::rc::Rc<Funccall>> for FunccalId {
    #[inline(always)]
    fn from(fc: &std::rc::Rc<Funccall>) -> Self {
        fc.fc_id
    }
}

impl From<std::rc::Rc<Funccall>> for FunccalId {
    #[inline(always)]
    fn from(fc: std::rc::Rc<Funccall>) -> Self {
        fc.fc_id
    }
}

impl From<Option<&Funccall>> for FunccalId {
    #[inline(always)]
    fn from(opt: Option<&Funccall>) -> Self {
        opt.map_or(Self::NULL, |fc| fc.fc_id)
    }
}

impl From<Option<std::rc::Rc<Funccall>>> for FunccalId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<Funccall>>) -> Self {
        opt.map_or(Self::NULL, |fc| fc.fc_id)
    }
}

pub struct UserFunc {
    pub uf_varargs: std::cell::Cell<i32>,
    pub uf_flags: std::cell::Cell<i32>,
    pub uf_calls: std::cell::Cell<i32>,
    pub uf_cleared: std::cell::Cell<bool>,
    pub uf_args: std::cell::RefCell<Vec<std::ffi::CString>>,
    pub uf_def_args: std::cell::RefCell<Vec<std::ffi::CString>>,
    pub uf_lines: std::cell::RefCell<Vec<Option<std::ffi::CString>>>,
    pub uf_profiling: std::cell::Cell<i32>,
    pub uf_prof_initialized: std::cell::Cell<bool>,
    pub uf_luaref: std::cell::Cell<i32>,
    pub uf_tm_count: std::cell::Cell<i32>,
    pub uf_tm_total: std::cell::Cell<u64>,
    pub uf_tm_self: std::cell::Cell<u64>,
    pub uf_tm_children: std::cell::Cell<u64>,
    pub uf_tml_count: std::cell::RefCell<Vec<i32>>,
    pub uf_tml_total: std::cell::RefCell<Vec<u64>>,
    pub uf_tml_self: std::cell::RefCell<Vec<u64>>,
    pub uf_tml_start: std::cell::Cell<u64>,
    pub uf_tml_children: std::cell::Cell<u64>,
    pub uf_tml_wait: std::cell::Cell<u64>,
    pub uf_tml_idx: std::cell::Cell<i32>,
    pub uf_tml_execed: std::cell::Cell<i32>,
    pub uf_script_ctx: std::cell::Cell<crate::runtime::Sctx>,
    pub uf_refcount: std::cell::Cell<i32>,
    pub uf_scoped: std::cell::Cell<FunccalId>,
    pub uf_name_exp: Option<std::ffi::CString>,
    pub uf_namelen: usize,
    pub uf_name: std::ffi::CString,
    pub uf_id: UfuncId,
}
use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO};
use crate::buffer::{DO_NOT_FREE_CNT, VAR_FIXED, VAR_SCOPE};
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
    pub fd_dict: DictId,
    pub fd_newkey: Option<std::ffi::CString>,
    pub fd_di: DictitemId,
}

impl Funcdict {
    pub const fn new() -> Self {
        Self {
            fd_dict: DictId::NULL,
            fd_newkey: None,
            fd_di: DictitemId::NULL,
        }
    }

    #[inline]
    pub fn dict_opt(&self) -> Option<&TvDict> {
        self.fd_dict.opt()
    }

    #[inline]
    pub fn di(&self) -> &Dictitem {
        &self.fd_di
    }
}

impl Default for Funcdict {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Funcexe {
    pub fe_fill_submatch: bool,
    pub fe_firstline: Linenr,
    pub fe_lastline: Linenr,
    pub fe_doesrange: bool,
    pub fe_evaluate: bool,
    pub fe_partial: crate::eval::typval::PartialId,
    pub fe_selfdict: DictId,
    pub fe_basetv: Option<TypVal>,
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
            fe_partial: crate::eval::typval::PartialId::NULL,
            fe_selfdict: DictId::NULL,
            fe_basetv: None,
            fe_found_var: false,
        }
    }

    #[inline]
    pub fn partial_opt(&self) -> Option<&TvPartial> {
        self.fe_partial.opt()
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
pub struct DeferCall {
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

pub type FuncHashtab = crate::memory::FxHashMap<Vec<u8>, UfuncId>;

struct UserfuncState {
    func_hashtab_changed: std::cell::Cell<i32>,
    current_funccal: std::cell::Cell<FunccalId>,
    lambda_no: std::cell::Cell<i32>,
    made_copy: std::cell::Cell<i32>,
    call_depth: std::cell::Cell<i32>,
    func_nr: std::cell::Cell<i32>,
    get_uf_done: std::cell::Cell<usize>,
    get_uf_changed: std::cell::Cell<i32>,
}

impl UserfuncState {
    const fn new() -> Self {
        Self {
            func_hashtab_changed: std::cell::Cell::new(0),
            current_funccal: std::cell::Cell::new(FunccalId::NULL),
            lambda_no: std::cell::Cell::new(0),
            made_copy: std::cell::Cell::new(0),
            call_depth: std::cell::Cell::new(0),
            func_nr: std::cell::Cell::new(0),
            get_uf_done: std::cell::Cell::new(0),
            get_uf_changed: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static UF_STATE: UserfuncState = const { UserfuncState::new() };
    static UFUNC_ARENA: std::cell::RefCell<Vec<Option<std::rc::Rc<UserFunc>>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FREE_UFUNCS: std::cell::RefCell<Vec<usize>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FUNCCAL_ARENA: std::cell::RefCell<Vec<Option<std::rc::Rc<Funccall>>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FREE_FUNCCALS: std::cell::RefCell<Vec<usize>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FUNC_HASHTAB: std::cell::RefCell<FuncHashtab> = const {
        std::cell::RefCell::new(crate::memory::fx_hashmap())
    };
    static FUNCARGS: std::cell::RefCell<Vec<TypVal>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static PREVIOUS_FUNCCAL: std::cell::RefCell<Vec<FunccalId>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static FUNCCAL_STACK: std::cell::RefCell<Vec<FunccalId>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static GET_UF_ITEMS: std::cell::RefCell<Vec<UfuncId>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

fn free_funccal_slot(id: FunccalId) {
    if let Some(nz) = id.0 {
        let idx = (nz.get() - 1) as usize;
        let dropped = FUNCCAL_ARENA
            .try_with(|a| a.borrow_mut()[idx].take())
            .ok()
            .flatten();
        let _ = FREE_FUNCCALS.try_with(|f| f.borrow_mut().push(idx));
        drop(dropped);
    }
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

#[inline]
fn uf2hikey(fp: &UserFunc) -> &CStr {
    fp.uf_name.as_c_str()
}

fn func_hashtab_add(fp: &UserFunc) -> bool {
    let key_cstr = uf2hikey(fp);
    let key = key_cstr.to_bytes();
    if FUNC_HASHTAB.with(|c| c.borrow().contains_key(key)) {
        crate::siemsg!(c"Duplicate key in hash table: \"%s\"", key_cstr);
        return false;
    }
    FUNC_HASHTAB.with(|c| c.borrow_mut().insert(key.to_vec(), fp.uf_id));
    UF_STATE.with(|s| {
        s.func_hashtab_changed
            .set(s.func_hashtab_changed.get().wrapping_add(1));
    });
    true
}

#[inline]
fn funcarg(fp: &UserFunc, j: i32) -> std::ffi::CString {
    fp.uf_args.borrow()[j as usize].clone()
}

/// Return the keys of the function hash table.
pub fn func_tbl_keys() -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = FUNC_HASHTAB.with(|c| c.borrow().keys().cloned().collect());
    keys.sort_unstable();
    keys
}

/// Return the values of the function hash table.
pub fn func_tbl_values() -> Vec<UfuncId> {
    FUNC_HASHTAB.with(|c| {
        let map = c.borrow();
        let mut pairs: Vec<(&[u8], UfuncId)> =
            map.iter().map(|(k, &id)| (k.as_slice(), id)).collect();
        pairs.sort_unstable_by(|a, b| a.0.cmp(b.0));
        pairs.into_iter().map(|(_, id)| id).collect()
    })
}

/// Get one function argument.
/// Return a pointer to after the type.
/// When something is wrong return "arg".
fn one_function_arg<'a>(
    arg_in: &'a CStr,
    newargs: Option<&mut Vec<std::ffi::CString>>,
    skip: bool,
) -> &'a CStr {
    let bytes = arg_in.to_bytes();
    let len = bytes
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
        .count();
    let arg_slice = &bytes[..len];
    if len == 0
        || arg_slice[0].is_ascii_digit()
        || arg_slice == b"firstline"
        || arg_slice == b"lastline"
    {
        if !skip {
            semsg!(gettext(c"E125: Illegal argument: %s"), arg_in);
        }
        return arg_in;
    }

    if let Some(newargs) = newargs {
        let arg_copy = std::ffi::CString::new(arg_slice).unwrap();

        for existing in newargs.iter() {
            if existing.as_bytes() == arg_copy.as_bytes() {
                semsg!(gettext(c"E853: Duplicate argument name: %s"), &arg_copy,);
                return arg_in;
            }
        }
        newargs.push(arg_copy);
    }

    &arg_in[len..]
}

/// Get function arguments.
fn get_function_args(
    argp: &mut &CStr,
    endchar: u8,
    mut newargs: Option<&mut Vec<std::ffi::CString>>,
    mut varargs: Option<&mut i32>,
    mut default_args: Option<&mut Vec<std::ffi::CString>>,
    skip: bool,
) -> bool {
    let mut mustend = false;
    let mut p_cstr = *argp;

    if let Some(ref mut p) = varargs {
        **p = 0;
    }

    let mut any_default = false;
    while p_cstr.to_bytes_with_nul()[0] != endchar {
        if p_cstr.to_bytes().starts_with(b"...") {
            if let Some(ref mut p) = varargs {
                **p = 1;
            }
            p_cstr = &p_cstr[3..];
            mustend = true;
        } else {
            let arg_cstr = p_cstr;
            p_cstr = one_function_arg(p_cstr, newargs.as_deref_mut(), skip);
            if p_cstr.as_ptr() == arg_cstr.as_ptr() {
                break;
            }

            if crate::charset::skipwhite(p_cstr).to_bytes().first() == Some(&b'=')
                && default_args.is_some()
            {
                let mut rettv: TypVal = TypVal::new();

                any_default = true;
                p_cstr = crate::charset::skipwhite_at(crate::charset::skipwhite(p_cstr), 1);
                let expr_cstr = p_cstr;
                let eval_ok = eval1(&mut p_cstr, &mut rettv, None);
                if eval_ok {
                    let mut expr_len =
                        (p_cstr.as_ptr() as usize).wrapping_sub(expr_cstr.as_ptr() as usize);
                    let expr_bytes = expr_cstr.to_bytes();
                    while expr_len > 0 && ascii_iswhite(expr_bytes[expr_len - 1]) {
                        expr_len -= 1;
                    }
                    p_cstr = &expr_cstr[expr_len..];
                    if let Some(default_args) = default_args.as_deref_mut() {
                        default_args.push(std::ffi::CString::new(&expr_bytes[..expr_len]).unwrap());
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

            if ascii_iswhite(p_cstr.to_bytes_with_nul()[0])
                && crate::charset::skipwhite(p_cstr).to_bytes().first() == Some(&b',')
            {
                if !skip {
                    semsg!(
                        gettext(E_NO_WHITE_SPACE_ALLOWED_BEFORE_STR_STR),
                        c",",
                        p_cstr
                    );
                    if let Some(newargs) = newargs.as_deref_mut() {
                        newargs.clear();
                    }
                    if let Some(default_args) = default_args.as_deref_mut() {
                        default_args.clear();
                    }
                    return false;
                }
                p_cstr = crate::charset::skipwhite(p_cstr);
            }
            if p_cstr.to_bytes_with_nul()[0] == b',' {
                p_cstr = &p_cstr[1..];
            } else {
                mustend = true;
            }
        }
        p_cstr = crate::charset::skipwhite(p_cstr);
        if mustend && p_cstr.to_bytes_with_nul()[0] != endchar {
            if !skip {
                semsg!(gettext(E_INVARG2), *argp);
            }
            break;
        }
    }
    if p_cstr.to_bytes_with_nul()[0] != endchar {
        if let Some(newargs) = newargs {
            newargs.clear();
        }
        if let Some(default_args) = default_args {
            default_args.clear();
        }
        return false;
    }
    if !p_cstr.is_empty() {
        p_cstr = &p_cstr[1..]; // skip "endchar"
    }

    *argp = p_cstr;

    true
}

/// Register function "fp" as using "current_funccal" as its scope.
fn register_closure(fp: &UserFunc) {
    if fp.uf_scoped.get() == UF_STATE.with(|s| s.current_funccal.get()) {
        return;
    }
    funccal_unref(fp.uf_scoped.get().rc().as_deref(), fp, false);
    fp.uf_scoped.set(UF_STATE.with(|s| s.current_funccal.get()));
    let cur_fc = cur_funccal();
    cur_fc.fc_refcount.set(cur_fc.fc_refcount.get() + 1);
    cur_fc.fc_ufuncs.borrow_mut().push(fp.uf_id);
}

/// Return a name for a lambda.
fn get_lambda_name() -> std::ffi::CString {
    let no = UF_STATE.with(|s| {
        let next = s.lambda_no.get() + 1;
        s.lambda_no.set(next);
        next
    });
    std::ffi::CString::new(format!("<lambda>{no}")).unwrap()
}

/// Allocate a `UserFunc` for a function called `name`.
fn alloc_ufunc(name: &[u8]) -> std::rc::Rc<UserFunc> {
    let namelen = name.len();
    let name_cstr = bytes_to_cstring(name);
    let uf_name_exp = if name.first() == Some(&K_SPECIAL) {
        let mut bytes = Vec::with_capacity(namelen + 2);
        bytes.extend_from_slice(b"<SNR>");
        bytes.extend_from_slice(&name_cstr.to_bytes()[3..]);
        Some(std::ffi::CString::new(bytes).unwrap())
    } else {
        None
    };

    let popped = FREE_UFUNCS.with(|f| f.borrow_mut().pop());
    let idx = popped.unwrap_or_else(|| UFUNC_ARENA.with(|a| a.borrow().len()));
    let id = UfuncId::from_slot(idx);
    let fp = std::rc::Rc::new(UserFunc {
        uf_varargs: std::cell::Cell::new(0),
        uf_flags: std::cell::Cell::new(0),
        uf_calls: std::cell::Cell::new(0),
        uf_cleared: std::cell::Cell::new(false),
        uf_args: std::cell::RefCell::new(Vec::new()),
        uf_def_args: std::cell::RefCell::new(Vec::new()),
        uf_lines: std::cell::RefCell::new(Vec::new()),
        uf_profiling: std::cell::Cell::new(0),
        uf_prof_initialized: std::cell::Cell::new(false),
        uf_luaref: std::cell::Cell::new(0),
        uf_tm_count: std::cell::Cell::new(0),
        uf_tm_total: std::cell::Cell::new(0),
        uf_tm_self: std::cell::Cell::new(0),
        uf_tm_children: std::cell::Cell::new(0),
        uf_tml_count: std::cell::RefCell::new(Vec::new()),
        uf_tml_total: std::cell::RefCell::new(Vec::new()),
        uf_tml_self: std::cell::RefCell::new(Vec::new()),
        uf_tml_start: std::cell::Cell::new(0),
        uf_tml_children: std::cell::Cell::new(0),
        uf_tml_wait: std::cell::Cell::new(0),
        uf_tml_idx: std::cell::Cell::new(0),
        uf_tml_execed: std::cell::Cell::new(0),
        uf_script_ctx: std::cell::Cell::new(crate::runtime::Sctx::new()),
        uf_refcount: std::cell::Cell::new(0),
        uf_scoped: std::cell::Cell::new(FunccalId::NULL),
        uf_name_exp,
        uf_namelen: namelen,
        uf_name: name_cstr,
        uf_id: id,
    });
    if popped.is_some() {
        UFUNC_ARENA.with(|a| a.borrow_mut()[idx] = Some(fp.clone()));
    } else {
        UFUNC_ARENA.with(|a| a.borrow_mut().push(Some(fp.clone())));
    }
    fp
}

/// Parse a lambda expression and get a Funcref from `*arg`.
pub fn get_lambda_tv(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    evalarg: Option<&mut Evalarg>,
) -> Option<bool> {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    let mut newargs: Vec<std::ffi::CString> = Vec::new();
    let mut varargs: i32 = 0;
    let old_eval_lavars = eval_lavars_used();

    let mut s = crate::charset::skipwhite_at(arg, 1);
    let mut ret = get_function_args(&mut s, b'-', None, None, None, true);
    if !ret || s.to_bytes().first() != Some(&b'>') {
        return None;
    }

    *arg = crate::charset::skipwhite_at(arg, 1);

    ret = get_function_args(
        arg,
        b'-',
        evaluate.then_some(&mut newargs),
        Some(&mut varargs),
        None,
        false,
    );
    if !ret || arg.to_bytes().first() != Some(&b'>') {
        return Some(false);
    }

    if evaluate {
        set_eval_lavars_used(Some(false));
    }

    let mut arg_cstr = crate::charset::skipwhite_at(arg, 1);
    let start_cstr = arg_cstr;
    ret = skip_expr(&mut arg_cstr, evalarg);
    let end_cstr = arg_cstr;
    *arg = arg_cstr;
    if !ret {
        if evaluate {
            set_eval_lavars_used(old_eval_lavars);
        }
        return Some(false);
    }

    let arg_ws = crate::charset::skipwhite(arg_cstr);
    *arg = arg_ws;

    if arg_ws.to_bytes().first() != Some(&b'}') {
        semsg!(gettext(c"E451: Expected }: %s"), arg_ws);
        if evaluate {
            set_eval_lavars_used(old_eval_lavars);
        }
        return Some(false);
    }

    *arg = &arg_ws[1..];

    if evaluate {
        let mut flags = 0;

        let name = get_lambda_name();
        let fp = alloc_ufunc(name.as_bytes());
        let pt = crate::eval::typval::tv_partial_alloc();

        let expr_len = (end_cstr.as_ptr() as usize).wrapping_sub(start_cstr.as_ptr() as usize);
        let mut line_bytes = Vec::with_capacity(7 + expr_len);
        line_bytes.extend_from_slice(b"return ");
        line_bytes.extend_from_slice(&start_cstr.to_bytes()[..expr_len]);
        let line_cstr = std::ffi::CString::new(line_bytes).unwrap();
        if !line_cstr.to_bytes()[7..].windows(2).any(|w| w == b"a:") {
            flags |= FC_NOARGS;
        }
        let newlines = vec![Some(line_cstr)];

        let newlines_len = newlines.len() as i32;
        fp.uf_refcount.set(1);
        func_hashtab_add(&fp);
        *fp.uf_args.borrow_mut() = newargs;
        *fp.uf_def_args.borrow_mut() = Vec::new();
        *fp.uf_lines.borrow_mut() = newlines;
        if !UF_STATE.with(|s| s.current_funccal.get()).is_null() && eval_lavars_used() == Some(true)
        {
            flags |= FC_CLOSURE;
            register_closure(&fp);
        } else {
            fp.uf_scoped.set(FunccalId::NULL);
        }

        if prof_def_func() {
            func_do_profile(&fp);
        }
        if g_st().sandbox.get() != 0 {
            flags |= FC_SANDBOX;
        }
        fp.uf_varargs.set(1);
        fp.uf_flags.set(flags);
        fp.uf_calls.set(0);
        let mut sctx = g_st().current_sctx.get();
        sctx.sc_lnum += sourcing_lnum() - newlines_len;
        fp.uf_script_ctx.set(sctx);

        pt.set_func(Some(fp));
        pt.pt_refcount.set(1);
        rettv.vval =
            crate::eval::typval::TypvalVval::Partial(crate::eval::typval::PartialId::from(pt));
        rettv.v_type = VarType::VarPartial;
        set_eval_lavars_used(old_eval_lavars);
    }

    Some(true)
}

/// Return name of the function corresponding to `name`.
pub fn deref_func_name<'a>(
    name: &'a CStr,
    lenp: &mut i32,
    mut partialp: Option<&mut Option<std::rc::Rc<TvPartial>>>,
    no_autoload: bool,
    found_var: Option<&mut bool>,
) -> std::borrow::Cow<'a, CStr> {
    if let Some(ref mut partialp) = partialp {
        **partialp = None;
    }

    let Some(v) = crate::eval::vars::find_var_opt(name, *lenp as usize, false, no_autoload as i32)
    else {
        return std::borrow::Cow::Borrowed(name);
    };
    let tv = v.tv();
    if let Some(found_var) = found_var {
        *found_var = true;
    }

    if tv.v_type == VarType::VarFunc {
        if tv.vval.v_string_is_null() {
            *lenp = 0;
            return std::borrow::Cow::Borrowed(c"");
        }
        let s = tv.vval.v_string_owned();
        *lenp = s.as_bytes().len() as i32;
        return std::borrow::Cow::Owned(s);
    }

    if tv.v_type == VarType::VarPartial {
        let Some(pt) = tv.vval.v_partial_opt() else {
            *lenp = 0;
            return std::borrow::Cow::Borrowed(c"");
        };
        let s = partial_name(Some(pt));
        if let Some(partialp) = partialp {
            *partialp = tv.vval.v_partial().rc();
        }
        *lenp = s.to_bytes().len() as i32;
        return std::borrow::Cow::Owned(s.to_owned());
    }

    std::borrow::Cow::Borrowed(name)
}

/// Give an error message with a function name. Handle `<SNR>` things.
pub fn emsg_funcname(errmsg: &CStr, name: &CStr) {
    let mut p_owned: Option<std::ffi::CString> = None;
    let mut p = name;

    let name_bytes = name.to_bytes();
    if name_bytes.first() == Some(&K_SPECIAL) && name_bytes.len() >= 3 {
        let mut bytes = Vec::from(&b"<SNR>"[..]);
        bytes.extend_from_slice(&name_bytes[3..]);
        let joined = bytes_to_cstring(&bytes);
        p_owned = Some(joined);
        p = p_owned.as_deref().unwrap();
    }

    semsg!(gettext(errmsg), p);
    drop(p_owned);
}

/// Get function arguments at `*arg` and advance it.
fn get_func_arguments(
    arg: &mut &CStr,
    mut evalarg: Option<&mut Evalarg>,
    partial_argc: i32,
    argvars: &mut [TypVal],
    argcount: &mut i32,
) -> bool {
    let mut argp = *arg;
    let mut ret = true;

    while *argcount < MAX_FUNC_ARGS as i32 - partial_argc {
        argp = crate::charset::skipwhite_at(argp, 1);

        let b0 = argp.to_bytes().first().copied().unwrap_or(0);
        if b0 == b')' || b0 == b',' || b0 == 0 {
            break;
        }
        if !eval1(
            &mut argp,
            &mut argvars[*argcount as usize],
            evalarg.as_deref_mut(),
        ) {
            ret = false;
            break;
        }
        *argcount += 1;
        if argp.to_bytes().first() != Some(&b',') {
            break;
        }
    }

    argp = crate::charset::skipwhite(argp);
    if argp.to_bytes().first() == Some(&b')') {
        argp = &argp[1..];
    } else {
        ret = false;
    }

    *arg = argp;

    ret
}

/// Call a function and put the result in `rettv`.
pub fn get_func_tv(
    name: &CStr,
    len: i32,
    rettv: &mut TypVal,
    arg: &mut &CStr,
    evalarg: Option<&mut Evalarg>,
    funcexe: &mut Funcexe,
) -> bool {
    let mut argvars: [TypVal; MAX_FUNC_ARGS + 1] = [const { TypVal::new() }; MAX_FUNC_ARGS + 1];
    let mut argcount: i32 = 0;
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

    let mut argp = *arg;
    let partial_argc = if funcexe.fe_partial.is_null() {
        0
    } else {
        funcexe.fe_partial.pt_argv.borrow().len() as i32
    };
    let mut ret = get_func_arguments(
        &mut argp,
        evalarg,
        partial_argc,
        &mut argvars,
        &mut argcount,
    );

    if ret {
        let mut i = 0usize;

        if get_vim_var_nr(VV_TESTING) != 0 {
            FUNCARGS.with(|c| {
                let mut fa = c.borrow_mut();
                while (i as i32) < argcount {
                    fa.push(argvars[i].clone());
                    i += 1;
                }
            });
        }
        ret = call_func(name, len, rettv, argcount, &mut argvars, funcexe);

        if i > 0 {
            FUNCARGS.with(|c| {
                let mut fa = c.borrow_mut();
                let new_len = fa.len().saturating_sub(i);
                fa.truncate(new_len);
            });
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

    *arg = crate::charset::skipwhite(argp);
    ret
}

/// Check whether function name starts with `<SID>` or `s:`.
#[inline]
fn eval_fname_sid(name_bytes: &[u8]) -> bool {
    name_bytes.first() == Some(&b's')
        || name_bytes
            .get(2)
            .is_some_and(|&b| b.eq_ignore_ascii_case(&b'I'))
}

/// In a script transform script-local names into actually used names.
fn fname_trans_sid<'a>(
    name_in: &'a CStr,
    fname_buf: &'a mut [u8; FLEN_FIXED + 1],
    tofree: &'a mut Option<std::ffi::CString>,
    error: &mut i32,
) -> &'a CStr {
    let name_bytes = name_in.to_bytes();
    let script_off = eval_fname_script(name_bytes);
    if script_off == 0 {
        return name_in;
    }
    let script_name = &name_in[script_off..];

    fname_buf[..4].copy_from_slice(&[K_SPECIAL, KS_EXTRA, KE_SNR, 0]);
    let mut fname_buflen = 3usize;
    if eval_fname_sid(name_bytes) {
        if g_st().current_sctx.get().sc_sid <= 0 {
            *error = FCERR_SCRIPT;
        } else {
            fname_buflen += format_to_buf!(
                &mut fname_buf[fname_buflen..FLEN_FIXED + 1],
                c"%d_",
                g_st().current_sctx.get().sc_sid,
            ) as usize;
        }
    }
    let script_bytes = script_name.to_bytes_with_nul();
    let fnamelen = fname_buflen + script_bytes.len() - 1;
    if fnamelen < FLEN_FIXED {
        fname_buf[fname_buflen..fname_buflen + script_bytes.len()].copy_from_slice(script_bytes);
        crate::memory::bytes_as_cstr(fname_buf)
    } else {
        let mut fname_vec = Vec::with_capacity(fnamelen + 1);
        fname_vec.extend_from_slice(&fname_buf[..fname_buflen]);
        fname_vec.extend_from_slice(script_bytes);
        let cstr = crate::memory::cstring_until_nul(&fname_vec);
        *tofree = Some(cstr);
        tofree.as_deref().unwrap()
    }
}

pub fn get_func_arity(
    name: &CStr,
    required: &mut i32,
    optional: &mut i32,
    varargs: &mut bool,
) -> bool {
    let argcount: i32;
    let min_argcount: i32;

    if let Some(fdef) = find_internal_func(name) {
        argcount = fdef.max_argc as i32;
        min_argcount = fdef.min_argc as i32;
        *varargs = false;
    } else {
        let mut fname_buf = [0u8; FLEN_FIXED + 1];
        let mut tofree: Option<std::ffi::CString> = None;
        let mut error = FCERR_NONE;

        let fname = fname_trans_sid(name, &mut fname_buf, &mut tofree, &mut error);
        let mut ufunc: Option<std::rc::Rc<UserFunc>> = None;
        if error == FCERR_NONE {
            ufunc = find_func(Some(fname));
        }
        drop(tofree);

        let Some(ufunc) = ufunc else {
            return false;
        };

        argcount = ufunc.uf_args.borrow().len() as i32;
        min_argcount = argcount - ufunc.uf_def_args.borrow().len() as i32;
        *varargs = ufunc.uf_varargs.get() != 0;
    }

    *required = min_argcount;
    *optional = argcount - min_argcount;

    true
}

/// Find a function by name, return handle to it in ufuncs.
pub fn find_func(name: Option<&CStr>) -> Option<std::rc::Rc<UserFunc>> {
    let name = name?;
    FUNC_HASHTAB
        .try_with(|c| c.borrow().get(name.to_bytes()).copied())
        .ok()
        .flatten()
        .and_then(UfuncId::rc)
}

#[inline]
fn func_is_global(ufunc: &UserFunc) -> bool {
    ufunc.uf_name.as_bytes_with_nul()[0] != K_SPECIAL
}

fn cat_func_name(buf: &mut [u8], fp: &UserFunc) -> usize {
    let bufsize = buf.len();
    let uflen = fp.uf_namelen;
    debug_assert!(uflen > 0);

    let hikey = uf2hikey(fp);
    let len = if !func_is_global(fp) && uflen > 3 {
        format_to_buf!(buf, c"<SNR>%s", &hikey[3..])
    } else {
        format_to_buf!(buf, c"%s", hikey)
    };

    debug_assert!(len > 0);
    if len as usize >= bufsize {
        bufsize.saturating_sub(1)
    } else {
        len as usize
    }
}

/// Add a number variable `name` to dict `dp` with value `nr`.
fn add_nr_var(dp: &TvDict, name: &std::ffi::CStr, nr: Varnumber) {
    let v = tv_dict_item_alloc(name);
    v.di_flags
        .set(v.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);
    v.set_tv(TypVal {
        v_type: VarType::VarNumber,
        v_lock: VAR_FIXED,
        vval: crate::eval::typval::TypvalVval::Number(nr),
    });
    hash_add(&dp.dv_hashtab, &v);
}

impl Drop for Funccall {
    fn drop(&mut self) {
        if crate::globals::g_st().exiting.get() {
            return;
        }
        if let Some(vars) = self.fc_l_vars.get().opt() {
            vars_clear(&vars.dv_hashtab);
            if vars.dv_refcount.get() >= DO_NOT_FREE_CNT / 2 {
                unref_var_dict(vars);
            } else {
                tv_dict_free_slot(vars);
            }
        }
        if let Some(v) = self.fc_l_vars_var.get().opt() {
            tv_dict_item_free_slot(v);
        }
        if let Some(avars) = self.fc_l_avars.get().opt() {
            vars_clear(&avars.dv_hashtab);
            if avars.dv_refcount.get() >= DO_NOT_FREE_CNT / 2 {
                unref_var_dict(avars);
            } else {
                tv_dict_free_slot(avars);
            }
        }
        if let Some(v) = self.fc_l_avars_var.get().opt() {
            tv_dict_item_free_slot(v);
        }
        if let Some(varlist) = self.fc_l_varlist.get().opt() {
            tv_list_free_contents(varlist);
            unref_var_list(varlist);
        }
        let self_id = self.fc_id;
        for fp_id in self.fc_ufuncs.get_mut() {
            if let Some(fp) = fp_id.rc()
                && fp.uf_scoped.get() == self_id
            {
                fp.uf_scoped.set(FunccalId::NULL);
            }
        }
        func_ptr_unref(self.fc_func.get().rc().as_deref());
    }
}

/// Handle the last part of returning from a function: free the local hashtable.
/// Unless it is still in use by a closure.
fn cleanup_function_call(fc: &Funccall) {
    let may_free_fc = fc.fc_refcount.get() <= 0;
    let mut free_fc = true;

    UF_STATE.with(|s| s.current_funccal.set(fc.fc_caller.get()));

    if may_free_fc && fc.fc_l_vars.get().dv_refcount.get() == DO_NOT_FREE_CNT {
        vars_clear(&fc.fc_l_vars.get().dv_hashtab);
    } else {
        free_fc = false;
    }

    if may_free_fc && fc.fc_l_avars.get().dv_refcount.get() == DO_NOT_FREE_CNT {
        vars_clear_ext(&fc.fc_l_avars.get().dv_hashtab, false);
    } else {
        free_fc = false;

        for (_, di) in fc.fc_l_avars.get().dv_hashtab.iter_items() {
            let tv_val = di.tv().clone();
            tv_copy(&tv_val, &mut di.tv_mut());
        }
    }

    if may_free_fc && fc.fc_l_varlist.get().lv_refcount.get() == DO_NOT_FREE_CNT {
        for li in crate::eval::typval::list_iter_ptr(fc.fc_l_varlist.get().first_opt()) {
            li.set_tv(TypVal::new());
        }
        tv_list_free_contents(&fc.fc_l_varlist.get());
    } else {
        free_fc = false;

        {
            for li in crate::eval::typval::list_iter_ptr(fc.fc_l_varlist.get().first_opt()) {
                let src_tv = li.tv().clone();
                tv_copy(&src_tv, &mut li.tv_mut());
            }
        }
    }

    let fc_id = fc.fc_id;
    if free_fc {
        free_funccal_slot(fc_id);
    } else {
        fc.fc_caller.set(FunccalId::NULL);
        PREVIOUS_FUNCCAL.with(|c| c.borrow_mut().push(fc_id));

        if g_st().want_garbage_collect.get() {
            UF_STATE.with(|s| s.made_copy.set(0));
        } else {
            UF_STATE.with(|s| s.made_copy.set(s.made_copy.get() + (1)));
            if UF_STATE.with(|s| s.made_copy.get())
                >= ((4096 * 1024) / std::mem::size_of::<Funccall>()) as i32
            {
                UF_STATE.with(|s| s.made_copy.set(0));
                g_st().want_garbage_collect.set(true);
            }
        }
    }
}

/// Unreference `fc`: decrement the reference count and free it when it
/// becomes zero. `fp` is detached from `fc`.
fn funccal_unref(fc_opt: Option<&Funccall>, fp: &UserFunc, force: bool) {
    let Some(fc) = fc_opt else {
        return;
    };
    let fc_id = fc.fc_id;

    fc.fc_refcount.set(fc.fc_refcount.get() - 1);

    if if force {
        fc.fc_refcount.get() <= 0
    } else {
        !fc_referenced(fc)
    } {
        let removed = PREVIOUS_FUNCCAL
            .try_with(|c| {
                let mut v = c.borrow_mut();
                v.iter()
                    .position(|&item| item == fc_id)
                    .map(|pos| v.remove(pos))
            })
            .ok()
            .flatten();
        if let Some(removed_id) = removed {
            free_funccal_slot(removed_id);
            return;
        }
    }
    for slot in &mut *fc.fc_ufuncs.borrow_mut() {
        if *slot == fp.uf_id {
            *slot = UfuncId::NULL;
        }
    }
}

/// Remove the function from the function hashtable.
fn func_remove(fp: &UserFunc) -> bool {
    let key = uf2hikey(fp).to_bytes();
    if FUNC_HASHTAB
        .try_with(|c| c.borrow_mut().remove(key))
        .ok()
        .flatten()
        .is_some()
    {
        let _ = UF_STATE.try_with(|s| {
            s.func_hashtab_changed
                .set(s.func_hashtab_changed.get().wrapping_add(1))
        });
        true
    } else {
        false
    }
}

fn func_clear_items(fp: &UserFunc) {
    fp.uf_args.borrow_mut().clear();
    fp.uf_def_args.borrow_mut().clear();
    fp.uf_lines.borrow_mut().clear();

    if (fp.uf_flags.get() & FC_LUAREF) != 0 {
        api_free_luaref(fp.uf_luaref.get());
        fp.uf_luaref.set(LUA_NOREF);
    }

    fp.uf_tml_count.borrow_mut().clear();
    fp.uf_tml_total.borrow_mut().clear();
    fp.uf_tml_self.borrow_mut().clear();
}

fn func_clear(fp: &UserFunc, force: bool) {
    if fp.uf_cleared.get() {
        return;
    }
    fp.uf_cleared.set(true);

    func_clear_items(fp);
    funccal_unref(fp.uf_scoped.get().rc().as_deref(), fp, force);
}

impl Drop for UserFunc {
    fn drop(&mut self) {
        if crate::globals::g_st().exiting.get() {
            return;
        }
        if !self.uf_cleared.get() {
            self.uf_cleared.set(true);
            func_clear_items(self);
        }
        if (self.uf_flags.get() & (FC_DELETED | FC_REMOVED)) == 0 {
            func_remove(self);
        }
    }
}

fn free_ufunc_slot(fp: &UserFunc) {
    if let Some(nz) = fp.uf_id.0 {
        let idx = (nz.get() - 1) as usize;
        let dropped = UFUNC_ARENA
            .try_with(|a| a.borrow_mut()[idx].take())
            .ok()
            .flatten();
        let _ = FREE_UFUNCS.try_with(|f| f.borrow_mut().push(idx));
        drop(dropped);
    }
}

fn func_clear_free(fp_in: &UserFunc, force: bool) {
    func_clear(fp_in, force);
    free_ufunc_slot(fp_in);
}

/// Allocate a `Funccall`, link it in `UF_STATE.with(|s| s.current_funccal.get())` and fill in `fp` and `rettv`.
pub fn create_funccal(fp: Option<&UserFunc>, rettv: TypVal) -> std::rc::Rc<Funccall> {
    let fp_ptr = UfuncId::from(fp);
    let popped = FREE_FUNCCALS.with(|f| f.borrow_mut().pop());
    let idx = popped.unwrap_or_else(|| FUNCCAL_ARENA.with(|a| a.borrow().len()));
    let id = FunccalId::from_slot(idx);
    let mut fc_val = Funccall::new();
    fc_val.fc_id = id;
    let fc = std::rc::Rc::new(fc_val);
    if popped.is_some() {
        FUNCCAL_ARENA.with(|a| a.borrow_mut()[idx] = Some(fc.clone()));
    } else {
        FUNCCAL_ARENA.with(|a| a.borrow_mut().push(Some(fc.clone())));
    }
    fc.fc_caller.set(UF_STATE.with(|s| s.current_funccal.get()));
    UF_STATE.with(|s| s.current_funccal.set(id));
    fc.fc_func.set(fp_ptr);
    func_ptr_ref(fp);
    *fc.fc_rettv.borrow_mut() = rettv;
    fc.fc_l_vars.set(tv_dict_alloc_static());
    fc.fc_l_vars_var.set(tv_dict_item_alloc_static());
    fc.fc_l_avars.set(tv_dict_alloc_static());
    fc.fc_l_avars_var.set(tv_dict_item_alloc_static());
    fc
}

/// Call a user function.
pub fn call_user_func(
    fp: &UserFunc,
    argcount: i32,
    argvars: &mut [TypVal],
    rettv: &mut TypVal,
    firstline: Linenr,
    lastline: Linenr,
    selfdict_in: Option<&TvDict>,
) {
    let selfdict = DictId::from(selfdict_in);
    let mut using_sandbox = false;
    let mut islambda = false;
    let mut numbuf = [0u8; NUMBUFLEN];
    let mut tv_to_free: [TypVal; MAX_FUNC_ARGS] = [const { TypVal::new() }; MAX_FUNC_ARGS];
    let mut tv_to_free_len = 0usize;
    let mut wait_start: Proftime = 0;
    let mut call_start: Proftime = 0;
    let mut started_profiling = false;
    let mut did_save_redo = false;
    let mut save_redo = RedoState::new();

    if i64::from(UF_STATE.with(|s| s.call_depth.get())) >= opt_vars().p_mfd.get() {
        emsg(gettext(
            c"E132: Function call depth is higher than 'maxfuncdepth'",
        ));
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        return;
    }
    UF_STATE.with(|s| s.call_depth.set(s.call_depth.get() + (1)));
    save_search_patterns();
    if !ins_compl_active() {
        save_redobuff(&mut save_redo);
        did_save_redo = true;
    }
    fp.uf_calls.set(fp.uf_calls.get() + (1));
    line_breakcheck();

    let fc_rc = create_funccal(Some(fp), std::mem::take(rettv));
    let fc = &*fc_rc;
    fc.fc_level.set(g_st().ex_nesting_level.get());
    fc.fc_breakpoint
        .set(dbg_find_breakpoint(false, uf2hikey(fp), 0));
    fc.fc_dbg_tick.set(g_st().debug_tick.get());

    if uf2hikey(fp).to_bytes().starts_with(b"<lambda>") {
        islambda = true;
    }

    init_var_dict(&fc.fc_l_vars.get(), &fc.fc_l_vars_var.get(), VAR_DEF_SCOPE);

    if !selfdict.is_null() {
        let v = tv_dict_item_alloc_len(b"self");
        v.di_flags
            .set(v.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);
        selfdict.dv_refcount.set(selfdict.dv_refcount.get() + 1);
        v.set_tv(TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: crate::eval::typval::TypvalVval::Dict(selfdict),
        });

        hash_add(&fc.fc_l_vars.get().dv_hashtab, &v);
    }

    init_var_dict(&fc.fc_l_avars.get(), &fc.fc_l_avars_var.get(), VAR_SCOPE);

    let uf_args_len = fp.uf_args.borrow().len() as i32;
    let uf_def_args_len = fp.uf_def_args.borrow().len() as i32;
    if (fp.uf_flags.get() & FC_NOARGS) == 0 {
        add_nr_var(
            &fc.fc_l_avars.get(),
            c"0",
            if argcount >= uf_args_len {
                Varnumber::from(argcount - uf_args_len)
            } else {
                0
            },
        );
    }
    fc.fc_l_avars.get().dv_lock.set(VAR_FIXED);
    let varlist = tv_list_alloc_static();
    tv_list_set_lock(Some(&varlist), VAR_FIXED);
    fc.fc_l_varlist.set(varlist);
    if (fp.uf_flags.get() & FC_NOARGS) == 0 {
        let v = tv_dict_item_alloc_len(b"000");
        v.di_flags
            .set(v.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);
        v.set_tv(TypVal {
            v_type: VarType::VarList,
            v_lock: VAR_FIXED,
            vval: crate::eval::typval::TypvalVval::List(fc.fc_l_varlist.get()),
        });
        hash_add(&fc.fc_l_avars.get().dv_hashtab, &v);
    }

    if (fp.uf_flags.get() & FC_NOARGS) == 0 {
        add_nr_var(
            &fc.fc_l_avars.get(),
            c"firstline",
            Varnumber::from(firstline),
        );
        add_nr_var(&fc.fc_l_avars.get(), c"lastline", Varnumber::from(lastline));
    }

    let mut default_arg_err = false;
    let max_i = argcount.max(uf_args_len);
    for i in 0..max_i {
        let mut addlocal = false;
        let mut isdefault = false;
        let mut def_rettv: TypVal = TypVal::new();

        let ai = i - uf_args_len;
        let fa_owned: std::ffi::CString;
        let name_slice: &[u8] = if ai < 0 {
            fa_owned = funcarg(fp, i);
            if islambda {
                addlocal = true;
            }

            isdefault = ai + uf_def_args_len >= 0 && i >= argcount;
            if isdefault {
                def_rettv.v_type = VarType::VarNumber;
                def_rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

                let default_expr_owned =
                    fp.uf_def_args.borrow()[(ai + uf_def_args_len) as usize].clone();
                let mut default_expr = default_expr_owned.as_c_str();
                if !eval1(
                    &mut default_expr,
                    &mut def_rettv,
                    Some(&mut crate::eval::vars::Evalarg { eval_flags: 1 }),
                ) {
                    default_arg_err = true;
                    break;
                }
            }

            fa_owned.to_bytes()
        } else {
            if (fp.uf_flags.get() & FC_NOARGS) != 0 {
                break;
            }
            let namelen = format_to_buf!(&mut numbuf, c"%d", ai + 1) as usize;
            &numbuf[..namelen]
        };

        let v = tv_dict_item_alloc_len(name_slice);
        v.di_flags
            .set(v.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);

        let mut arg_tv = if isdefault {
            def_rettv.clone()
        } else {
            argvars[i as usize].clone()
        };
        arg_tv.v_lock = VAR_FIXED;
        v.set_tv(arg_tv);

        if isdefault {
            tv_to_free[tv_to_free_len] = def_rettv;
            tv_to_free_len += 1;
        }

        if addlocal {
            let tv_val = v.tv().clone();
            tv_copy(&tv_val, &mut v.tv_mut());
            hash_add(&fc.fc_l_vars.get().dv_hashtab, &v);
        } else {
            hash_add(&fc.fc_l_avars.get().dv_hashtab, &v);
        }

        if ai >= 0 && (ai as usize) < MAX_FUNC_ARGS {
            let li = tv_list_item_alloc();
            let mut tv = argvars[i as usize].clone();
            tv.v_lock = VAR_FIXED;
            li.set_tv(tv);
            tv_list_append(&fc.fc_l_varlist.get(), &li);
        }
    }

    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() + (1));

    if (fp.uf_flags.get() & FC_SANDBOX) != 0 {
        using_sandbox = true;
        g_st().sandbox.set(g_st().sandbox.get() + (1));
    }

    estack_push_ufunc(fp, 1);
    let estack_len_before = crate::runtime::estack_len() as i32;
    if opt_vars().p_verbose.get() >= 12 {
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        verbose_enter_scroll();

        smsg!(0, gettext(c"calling %s"), sourcing_name().as_deref());
        if opt_vars().p_verbose.get() >= 14 {
            msg_puts(c"(");
            for (i, arg_tv) in argvars.iter_mut().take(argcount as usize).enumerate() {
                if i > 0 {
                    msg_puts(c", ");
                }
                if arg_tv.v_type == VarType::VarNumber {
                    msg_outnum(arg_tv.vval.v_number() as i32);
                } else {
                    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
                    let tf = encode_tv2string(arg_tv, None);
                    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
                    let mut buf = [0u8; MSG_BUF_LEN];
                    let s = if vim_strsize(tf.as_c_str()) > MSG_BUF_CLEN {
                        trunc_string(Some(tf.as_bytes()), &mut buf, MSG_BUF_CLEN);
                        crate::memory::bytes_as_cstr(&buf)
                    } else {
                        tf.as_c_str()
                    };
                    msg_puts(s);
                }
            }
            msg_puts(c")");
        }
        msg_puts(c"\n");

        verbose_leave_scroll();
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    }

    let do_profiling_yes = g_st().do_profiling.get() == PROF_YES;
    let func_not_yet_profiling_but_should =
        do_profiling_yes && fp.uf_profiling.get() == 0 && has_profiling(false, uf2hikey(fp), None);

    if func_not_yet_profiling_but_should {
        started_profiling = true;
        func_do_profile(fp);
    }

    let func_or_func_caller_profiling = do_profiling_yes
        && (fp.uf_profiling.get() != 0
            || (!fc.fc_caller.get().is_null()
                && fc
                    .fc_caller
                    .get()
                    .unwrap_rc()
                    .fc_func
                    .get()
                    .unwrap_rc()
                    .uf_profiling
                    .get()
                    != 0));

    if func_or_func_caller_profiling {
        fp.uf_tm_count.set(fp.uf_tm_count.get() + (1));
        call_start = profile_start();
        fp.uf_tm_children.set(profile_zero());
    }

    if do_profiling_yes {
        script_prof_save(&mut wait_start);
    }

    let save_current_sctx = g_st().current_sctx.get();
    g_st().current_sctx.set(fp.uf_script_ctx.get());
    let save_did_emsg = g_st().did_emsg.get();
    g_st().did_emsg.set(0);

    if default_arg_err && ((fp.uf_flags.get() & FC_ABORT) != 0 || g_st().trylevel.get() > 0) {
        g_st().did_emsg.set(1);
    } else if islambda {
        let line0 = fp.uf_lines.borrow()[0].clone().unwrap();
        let mut p = &line0.as_c_str()[7..];
        g_st()
            .ex_nesting_level
            .set(g_st().ex_nesting_level.get() + (1));
        let mut fc_rv = std::mem::take(&mut *fc.fc_rettv.borrow_mut());
        eval1(
            &mut p,
            &mut fc_rv,
            Some(&mut crate::eval::vars::Evalarg { eval_flags: 1 }),
        );
        *fc.fc_rettv.borrow_mut() = fc_rv;
        g_st()
            .ex_nesting_level
            .set(g_st().ex_nesting_level.get() - (1));
    } else {
        do_cmdline(
            None,
            crate::cmdexpand::LineGetter::Func(fc.into()),
            DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
        );
    }

    handle_defer_one(&cur_funccal());
    *rettv = std::mem::take(&mut *fc.fc_rettv.borrow_mut());

    g_st()
        .redrawing_disabled
        .set(g_st().redrawing_disabled.get() - (1));

    if (g_st().did_emsg.get() != 0 && (fp.uf_flags.get() & FC_ABORT) != 0)
        || rettv.v_type == VarType::VarUnknown
    {
        tv_clear(rettv);
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    }

    if func_or_func_caller_profiling {
        call_start = profile_end(call_start);
        call_start = profile_sub_wait(wait_start, call_start);
        fp.uf_tm_total
            .set(profile_add(fp.uf_tm_total.get(), call_start));
        fp.uf_tm_self.set(profile_self(
            fp.uf_tm_self.get(),
            call_start,
            fp.uf_tm_children.get(),
        ));
        if !fc.fc_caller.get().is_null()
            && fc
                .fc_caller
                .get()
                .unwrap_rc()
                .fc_func
                .get()
                .unwrap_rc()
                .uf_profiling
                .get()
                != 0
        {
            let caller_fp = fc.fc_caller.get().unwrap_rc().fc_func.get().unwrap_rc();
            caller_fp
                .uf_tm_children
                .set(profile_add(caller_fp.uf_tm_children.get(), call_start));
            caller_fp
                .uf_tml_children
                .set(profile_add(caller_fp.uf_tml_children.get(), call_start));
        }
        if started_profiling {
            fp.uf_profiling.set(0);
        }
    }

    if opt_vars().p_verbose.get() >= 12 {
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        verbose_enter_scroll();

        if aborting() {
            smsg!(0, gettext(c"%s aborted"), sourcing_name().as_deref());
        } else if rettv.v_type == VarType::VarNumber {
            smsg!(
                0,
                gettext(c"%s returning #%lld"),
                sourcing_name().as_deref(),
                rettv.vval.v_number(),
            );
        } else {
            let mut buf = [0u8; MSG_BUF_LEN];
            g_st().emsg_off.set(g_st().emsg_off.get() + (1));
            let tf = encode_tv2string(rettv, None);
            g_st().emsg_off.set(g_st().emsg_off.get() - (1));
            let s = if vim_strsize(tf.as_c_str()) > MSG_BUF_CLEN {
                trunc_string(Some(tf.as_bytes()), &mut buf, MSG_BUF_CLEN);
                crate::memory::bytes_as_cstr(&buf)
            } else {
                tf.as_c_str()
            };
            smsg!(
                0,
                gettext(c"%s returning %s"),
                sourcing_name().as_deref(),
                s
            );
        }
        msg_puts(c"\n");

        verbose_leave_scroll();
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    }

    let _ = estack_len_before;
    estack_pop();
    g_st().current_sctx.set(save_current_sctx);
    if do_profiling_yes {
        script_prof_restore(&wait_start);
    }
    if using_sandbox {
        g_st().sandbox.set(g_st().sandbox.get() - (1));
    }

    if opt_vars().p_verbose.get() >= 12 && sourcing_name().as_deref().is_some() {
        g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
        verbose_enter_scroll();

        smsg!(0, gettext(c"continuing in %s"), sourcing_name().as_deref());
        msg_puts(c"\n");

        verbose_leave_scroll();
        g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));
    }

    g_st().did_emsg.set(g_st().did_emsg.get() | (save_did_emsg));
    UF_STATE.with(|s| s.call_depth.set(s.call_depth.get() - (1)));
    cleanup_function_call(fc);
    for tv in tv_to_free.iter_mut().take(tv_to_free_len) {
        tv_clear(tv);
    }

    fp.uf_calls.set(fp.uf_calls.get() - (1));
    if fp.uf_calls.get() <= 0 && fp.uf_refcount.get() <= 0 {
        func_clear_free(fp, false);
    }
    if did_save_redo {
        restore_redobuff(&mut save_redo);
    }
    restore_search_patterns();
}

#[inline]
fn func_name_refcount(name: &CStr) -> bool {
    let name_bytes = name.to_bytes();
    name_bytes.first().is_some_and(u8::is_ascii_digit) || name_bytes.starts_with(b"<l")
}

fn check_user_func_argcount(fp: &UserFunc, argcount: i32) -> i32 {
    let regular_args = fp.uf_args.borrow().len() as i32;
    if argcount < regular_args - fp.uf_def_args.borrow().len() as i32 {
        FCERR_TOOFEW
    } else if fp.uf_varargs.get() == 0 && argcount > regular_args {
        FCERR_TOOMANY
    } else {
        FCERR_UNKNOWN
    }
}

fn call_user_func_check(
    fp: &UserFunc,
    argcount: i32,
    argvars: &mut [TypVal],
    rettv: &mut TypVal,
    funcexe: &mut Funcexe,
    selfdict: Option<&TvDict>,
) -> i32 {
    if (fp.uf_flags.get() & FC_LUAREF) != 0 {
        let argslice = if argcount <= 0 {
            &mut []
        } else {
            &mut argvars[..argcount as usize]
        };
        return nlua_exec_typval_callable(fp.uf_luaref.get(), argslice, rettv);
    }

    if (fp.uf_flags.get() & FC_RANGE) != 0 {
        funcexe.fe_doesrange = true;
    }
    let mut error = check_user_func_argcount(fp, argcount);
    if error != FCERR_UNKNOWN {
        return error;
    }
    if (fp.uf_flags.get() & FC_DICT) != 0 && selfdict.is_none() {
        error = FCERR_DICT;
    } else {
        call_user_func(
            fp,
            argcount,
            argvars,
            rettv,
            funcexe.fe_firstline,
            funcexe.fe_lastline,
            if (fp.uf_flags.get() & FC_DICT) != 0 {
                selfdict
            } else {
                None
            },
        );
        error = FCERR_NONE;
    }
    error
}

/// Save the current function call pointer, and set it to NULL.
pub fn save_funccal() {
    FUNCCAL_STACK.with(|c| {
        c.borrow_mut()
            .push(UF_STATE.with(|s| s.current_funccal.get()))
    });
    UF_STATE.with(|s| s.current_funccal.set(FunccalId::NULL));
}

pub fn restore_funccal() {
    if let Some(top) = FUNCCAL_STACK.with(|c| c.borrow_mut().pop()) {
        UF_STATE.with(|s| s.current_funccal.set(top));
    } else {
        iemsg(c"INTERNAL: restore_funccal()");
    }
}

pub fn get_current_funccal() -> Option<std::rc::Rc<Funccall>> {
    UF_STATE.with(|s| s.current_funccal.get()).rc()
}

pub fn set_current_funccal(fc: FunccalId) {
    UF_STATE.with(|s| s.current_funccal.set(fc));
}

pub fn free_all_functions() {
    while !UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        let fc = cur_funccal();
        tv_clear(&mut fc.fc_rettv.borrow_mut());
        cleanup_function_call(&fc);
        if UF_STATE.with(|s| s.current_funccal.get()).is_null()
            && !FUNCCAL_STACK.with(|c| c.borrow().is_empty())
        {
            restore_funccal();
        }
    }
    FUNCCAL_STACK.with(|c| c.borrow_mut().clear());

    loop {
        let mut restarted = false;
        let funcs = func_tbl_values();
        for fp_id in funcs {
            let fp = fp_id.unwrap_rc();
            if !func_name_refcount(uf2hikey(&fp)) {
                let changed = UF_STATE.with(|s| s.func_hashtab_changed.get());
                func_clear(&fp, true);
                if changed != UF_STATE.with(|s| s.func_hashtab_changed.get()) {
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
        if FUNC_HASHTAB.with(|c| c.borrow().len()) <= skipped {
            break;
        }
        let funcs = func_tbl_values();
        for fp_id in funcs {
            let fp = fp_id.unwrap_rc();
            if func_name_refcount(uf2hikey(&fp)) {
                skipped += 1;
            } else {
                free_ufunc_slot(&fp);
                skipped = 0;
                break;
            }
        }
    }
    if skipped == 0 {
        FUNC_HASHTAB.with(|c| c.borrow_mut().clear());
    }
    GET_UF_ITEMS.with(|c| c.borrow_mut().clear());
}

/// Checks if a builtin function with the given name exists.
fn builtin_function(bytes: &[u8]) -> bool {
    if !bytes.first().is_some_and(|&b| ascii_islower(i32::from(b))) || bytes.get(1) == Some(&b':') {
        return false;
    }

    !bytes.contains(&AUTOLOAD_CHAR)
}

pub fn func_call(
    name: &CStr,
    args: &mut TypVal,
    partial: Option<std::rc::Rc<TvPartial>>,
    selfdict: Option<&TvDict>,
    rettv: &mut TypVal,
) -> bool {
    let mut argv: [TypVal; MAX_FUNC_ARGS + 1] = [const { TypVal::new() }; MAX_FUNC_ARGS + 1];
    let mut argc = 0;
    let mut r = false;

    let max_argc = MAX_FUNC_ARGS as i32
        - partial
            .as_deref()
            .map_or(0, |pt| pt.pt_argv.borrow().len() as i32);
    let mut skip_call = false;
    if let Some(list) = args.vval.v_list_opt() {
        for item in crate::eval::typval::list_iter_ptr(list.first_opt()) {
            if argc == max_argc {
                emsg(gettext(c"E699: Too many arguments"));
                skip_call = true;
                break;
            }
            tv_copy(&item.tv(), &mut argv[argc as usize]);
            argc += 1;
        }
    }

    if !skip_call {
        let mut funcexe = Funcexe::new();
        funcexe.fe_firstline = curwin().w_cursor.get().lnum;
        funcexe.fe_lastline = curwin().w_cursor.get().lnum;
        funcexe.fe_evaluate = true;
        funcexe.fe_partial = crate::eval::typval::PartialId::from(partial);
        funcexe.fe_selfdict = DictId::from(selfdict);
        r = call_func(name, -1, rettv, argc, &mut argv, &mut funcexe);
    }

    while argc > 0 {
        argc -= 1;
        tv_clear(&mut argv[argc as usize]);
    }

    r
}

pub fn callback_call_retnr(
    callback: &Callback,
    argcount: i32,
    argvars: &mut [TypVal],
) -> Varnumber {
    let mut rettv: TypVal = TypVal::new();
    if !callback_call(callback, argcount, argvars, &mut rettv) {
        return -2;
    }

    let retval = tv_get_number_chk(&rettv, None);
    tv_clear(&mut rettv);
    retval
}

fn user_func_error(error: i32, name: &CStr, found_var: bool) {
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

/// Call a function with its resolved parameters.
pub fn call_func(
    mut funcname: &CStr,
    mut len: i32,
    rettv: &mut TypVal,
    argcount_in: i32,
    argvars_in: &mut [TypVal],
    funcexe: &mut Funcexe,
) -> bool {
    let mut ret = false;
    let mut error = FCERR_NONE;
    let mut fp: Option<std::rc::Rc<UserFunc>> = None;
    let mut fname_buf = [0u8; FLEN_FIXED + 1];
    let mut tofree: Option<std::ffi::CString> = None;
    let mut fname: &CStr = c"";
    let mut name_owned: Option<std::ffi::CString> = None;
    let mut argcount = argcount_in;
    let mut use_argv = false;
    let mut selfdict = funcexe.fe_selfdict.clone();
    let mut argv: [TypVal; MAX_FUNC_ARGS + 1] = [const { TypVal::new() }; MAX_FUNC_ARGS + 1];
    let mut argv_clear = 0;
    let mut argv_base = 0;
    let partial = funcexe.fe_partial.clone();

    rettv.v_type = VarType::VarUnknown;

    if len <= 0 {
        len = funcname.to_bytes().len() as i32;
    }
    if let Some(partial_ref) = funcexe.partial_opt() {
        fp = partial_ref.func_rc();
    }
    if fp.is_none() {
        let fn_bytes = funcname.to_bytes();
        let nm = bytes_to_cstring(&fn_bytes[..(len as usize).min(fn_bytes.len())]);
        name_owned = Some(nm);
        fname = fname_trans_sid(
            name_owned.as_deref().unwrap(),
            &mut fname_buf,
            &mut tofree,
            &mut error,
        );
    }

    funcexe.fe_doesrange = false;

    let mut skip_eval = false;
    if let Some(partial) = funcexe.partial_opt() {
        if !partial.pt_dict.get().is_null() && (selfdict.is_null() || !partial.pt_auto.get()) {
            selfdict = partial.pt_dict.get();
        }
        let pt_argc = partial.pt_argv.borrow().len() as i32;
        if error == FCERR_NONE && pt_argc > 0 {
            while argv_clear < pt_argc {
                if argv_clear + argcount_in >= MAX_FUNC_ARGS as i32 {
                    error = FCERR_TOOMANY;
                    skip_eval = true;
                    break;
                }
                tv_copy(
                    &partial.pt_argv.borrow()[argv_clear as usize],
                    &mut argv[argv_clear as usize],
                );
                argv_clear += 1;
            }
            if !skip_eval {
                for i in 0..argcount_in {
                    argv[(i + argv_clear) as usize] = argvars_in[i as usize].clone();
                }
                use_argv = true;
                argcount = pt_argc + argcount_in;
            }
        }
    }

    if !skip_eval && error == FCERR_NONE && funcexe.fe_evaluate {
        let is_global = fp.is_none() && fname.to_bytes().starts_with(b"g:");
        let rfname = if is_global {
            crate::memory::CStrSubslice::advance(fname, 2)
        } else {
            fname
        };

        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        error = FCERR_UNKNOWN;

        if is_luafunc(partial.opt()) {
            if len > 0 {
                error = FCERR_NONE;
                if let Some(ref basetv) = funcexe.fe_basetv {
                    if use_argv {
                        for i in (0..argcount as usize).rev() {
                            argv[i + 1] = std::mem::take(&mut argv[i]);
                        }
                    } else {
                        argv[1..1 + argcount as usize]
                            .clone_from_slice(&argvars_in[..argcount as usize]);
                        use_argv = true;
                    }
                    argv[0] = basetv.clone();
                    argcount += 1;
                    argv_base = 1;
                }
                let active_argv: &mut [TypVal] = if use_argv { &mut argv } else { argvars_in };
                let argslice = if argcount <= 0 {
                    &mut []
                } else {
                    &mut active_argv[..argcount as usize]
                };
                let fn_bytes = funcname.to_bytes();
                nlua_call_vlua(
                    &fn_bytes[..(len as usize).min(fn_bytes.len())],
                    argslice,
                    rettv,
                );
            } else {
                name_owned = None;
                funcname = c"v:lua";
            }
        } else if fp.is_some() || !builtin_function(rfname.to_bytes()) {
            if fp.is_none() {
                fp = find_func(Some(rfname));
            }

            let rfname_opt = (!rfname.to_bytes().is_empty()).then_some(rfname);
            if fp.is_none()
                && apply_autocmds(EVENT_FUNCUNDEFINED, rfname_opt, rfname_opt, true, None)
                && !aborting()
            {
                fp = find_func(Some(rfname));
            }
            if fp.is_none() && script_autoload(rfname.to_bytes(), true) && !aborting() {
                fp = find_func(Some(rfname));
            }

            if let Some(fp_ref) = fp {
                if (fp_ref.uf_flags.get() & FC_DELETED) != 0 {
                    error = FCERR_DELETED;
                } else {
                    if funcexe.fe_fill_submatch {
                        let active_argv: &mut [TypVal] =
                            if use_argv { &mut argv } else { argvars_in };
                        argcount = crate::regexp::fill_submatch_list(
                            argcount,
                            active_argv,
                            argv_clear,
                            &fp_ref,
                        );
                    }

                    if let Some(ref basetv) = funcexe.fe_basetv {
                        if use_argv {
                            for i in (0..argcount as usize).rev() {
                                argv[i + 1] = std::mem::take(&mut argv[i]);
                            }
                        } else {
                            argv[1..1 + argcount as usize]
                                .clone_from_slice(&argvars_in[..argcount as usize]);
                            use_argv = true;
                        }
                        argv[0] = basetv.clone();
                        argcount += 1;
                        argv_base = 1;
                    }

                    let active_argv: &mut [TypVal] = if use_argv { &mut argv } else { argvars_in };
                    error = call_user_func_check(
                        &fp_ref,
                        argcount,
                        active_argv,
                        rettv,
                        &mut *funcexe,
                        selfdict.opt(),
                    );
                }
            }
        } else if let Some(ref basetv) = funcexe.fe_basetv {
            let active_argv: &[TypVal] = if use_argv { &argv } else { argvars_in };
            let argvars_slice = if argcount <= 0 {
                &[]
            } else {
                &active_argv[..argcount as usize]
            };
            error = call_internal_method(fname, argcount, argvars_slice, rettv, basetv);
        } else {
            let active_argv: &mut [TypVal] = if use_argv { &mut argv } else { argvars_in };
            error = call_internal_func(fname, argcount, active_argv, rettv);
        }
        update_force_abort();
    }
    if !skip_eval && error == FCERR_NONE {
        ret = true;
    }

    if !aborting() {
        user_func_error(
            error,
            name_owned.as_deref().unwrap_or(funcname),
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

pub fn call_simple_luafunc(funcname: &[u8], rettv: &mut TypVal) -> Option<bool> {
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    nlua_call_vlua(funcname, &mut [], &mut *rettv);
    Some(true)
}

pub fn call_simple_func(funcname: &[u8], rettv: &mut TypVal) -> Option<bool> {
    let mut ret = Some(false);

    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    let name = bytes_to_cstring(funcname);

    let mut error = FCERR_NONE;
    let mut tofree: Option<std::ffi::CString> = None;
    let mut fname_buf = [0u8; FLEN_FIXED + 1];
    let fname = fname_trans_sid(&name, &mut fname_buf, &mut tofree, &mut error);

    let is_global = fname.to_bytes().starts_with(b"g:");
    let rfname = if is_global {
        crate::memory::CStrSubslice::advance(fname, 2)
    } else {
        fname
    };

    let Some(fp_ref) = find_func(Some(rfname)) else {
        user_func_error(error, &name, false);
        drop(tofree);
        return None;
    };
    if (fp_ref.uf_flags.get() & FC_DELETED) != 0 {
        error = FCERR_DELETED;
    } else {
        let mut argvars: [TypVal; 1] = [TypVal::new(); 1];
        argvars[0].v_type = VarType::VarUnknown;
        let mut funcexe = Funcexe::new();
        funcexe.fe_evaluate = true;

        error = call_user_func_check(&fp_ref, 0, &mut argvars, &mut *rettv, &mut funcexe, None);
        if error == FCERR_NONE {
            ret = Some(true);
        }
    }

    user_func_error(error, &name, false);
    drop(tofree);

    ret
}

pub fn printable_func_name(fp: &UserFunc) -> &CStr {
    if let Some(ref exp) = fp.uf_name_exp {
        exp.as_c_str()
    } else {
        uf2hikey(fp)
    }
}

fn function_list_modified(prev_ht_changed: i32) -> bool {
    if prev_ht_changed != UF_STATE.with(|s| s.func_hashtab_changed.get()) {
        emsg(gettext(E_FUNCTION_LIST_WAS_MODIFIED));
        return true;
    }
    false
}

fn list_func_head(fp: &UserFunc, indent: bool, force: bool) -> bool {
    let prev_ht_changed = UF_STATE.with(|s| s.func_hashtab_changed.get());

    msg_start();

    if function_list_modified(prev_ht_changed) {
        return false;
    }

    if indent {
        msg_puts(c"   ");
    }
    msg_puts(if force { c"function! " } else { c"function " });
    if let Some(ref exp) = fp.uf_name_exp {
        msg_puts(exp);
    } else {
        msg_puts(uf2hikey(fp));
    }
    msg_putchar(b'(' as i32);
    let uf_args_len = fp.uf_args.borrow().len() as i32;
    let uf_def_args_len = fp.uf_def_args.borrow().len() as i32;
    let mut j = 0;
    while j < uf_args_len {
        if j != 0 {
            msg_puts(c", ");
        }
        msg_puts(&funcarg(fp, j));
        if j >= uf_args_len - uf_def_args_len {
            msg_puts(c" = ");
            msg_puts(&fp.uf_def_args.borrow()[(j - uf_args_len + uf_def_args_len) as usize]);
        }
        j += 1;
    }
    if fp.uf_varargs.get() != 0 {
        if j != 0 {
            msg_puts(c", ");
        }
        msg_puts(c"...");
    }
    msg_putchar(b')' as i32);
    if (fp.uf_flags.get() & FC_ABORT) != 0 {
        msg_puts(c" abort");
    }
    if (fp.uf_flags.get() & FC_RANGE) != 0 {
        msg_puts(c" range");
    }
    if (fp.uf_flags.get() & FC_DICT) != 0 {
        msg_puts(c" dict");
    }
    if (fp.uf_flags.get() & FC_CLOSURE) != 0 {
        msg_puts(c" closure");
    }
    msg_clr_eos();
    if opt_vars().p_verbose.get() > 0 {
        last_set_msg(fp.uf_script_ctx.get());
    }

    true
}

/// Get a function name, translating `<SID>` and `<SNR>`.
pub fn trans_function_name(
    pp: &mut &CStr,
    skip: bool,
    flags: i32,
    mut fdp: Option<&mut Funcdict>,
    mut partial: Option<&mut Option<std::rc::Rc<TvPartial>>>,
) -> Option<std::ffi::CString> {
    let mut name: Option<std::borrow::Cow<'_, CStr>> = None;
    let mut len: i32;
    let mut lv: Lval = Lval::new();

    if fdp.is_some() {
        *fdp.as_deref_mut().unwrap() = Funcdict::new();
    }
    let orig_pp = *pp;
    let mut start_cstr = orig_pp;

    if pp.to_bytes().starts_with(&[K_SPECIAL, KS_EXTRA, KE_SNR]) {
        *pp = crate::memory::CStrSubslice::advance(*pp, 3);

        len = get_id_len(pp) + 3;
        return Some(bytes_to_cstring(&orig_pp.to_bytes()[..len as usize]));
    }

    let mut lead = eval_fname_script(start_cstr.to_bytes()) as i32;
    if lead > 2 {
        start_cstr = &start_cstr[lead as usize..];
    }

    let end_opt = get_lval(
        start_cstr,
        None,
        &mut lv,
        false,
        skip,
        flags | GLV_READ_ONLY,
        if lead > 2 { 0 } else { FNE_CHECK_START },
    );
    if end_opt.is_some_and(|e| e.as_ptr() == start_cstr.as_ptr()) {
        if !skip {
            emsg(gettext(c"E129: Function name required"));
        }
        return None;
    }
    let Some(end_cstr) = end_opt else {
        if aborting() {
            *pp = find_name_end(start_cstr, None, FNE_INCL_BR);
        }
        return None;
    };
    if !lv.ll_tv.is_null() && (lead > 2 || lv.ll_range) {
        if !aborting() {
            semsg!(gettext(E_INVARG2), start_cstr);
        } else {
            *pp = find_name_end(start_cstr, None, FNE_INCL_BR);
        }
        return None;
    }

    if let Some(ll_tv) = lv.ll_tv.opt() {
        if fdp.is_some() {
            fdp.as_deref_mut().unwrap().fd_dict = lv.ll_dict.clone();
            fdp.as_deref_mut().unwrap().fd_newkey = lv.ll_newkey.take();
            fdp.as_deref_mut().unwrap().fd_di = lv.ll_di;
        }
        let mut res: Option<std::ffi::CString> = None;
        if ll_tv.v_type == VarType::VarFunc && !ll_tv.vval.v_string_is_null() {
            res = Some(ll_tv.vval.v_string_owned());

            *pp = end_cstr;
        } else if ll_tv.v_type == VarType::VarPartial && !ll_tv.vval.v_partial().is_null() {
            let pt = ll_tv.vval.v_partial();
            if is_luafunc(pt.opt()) && end_cstr.to_bytes().first() == Some(&b'.') {
                let after_dot = &end_cstr[1..];
                len = check_luafunc_name(after_dot, true);
                if len == 0 {
                    semsg!(E_INVEXPR2, c"v:lua");
                    return None;
                }

                res = Some(bytes_to_cstring(&after_dot.to_bytes()[..len as usize]));

                *pp = &after_dot[len as usize..];
            } else {
                res = Some(partial_name(pt.opt()).to_owned());

                *pp = end_cstr;
            }
            if let Some(ref mut partial_out) = partial {
                **partial_out = pt.rc();
            }
        } else if !skip
            && (flags & TFN_QUIET) == 0
            && (fdp.is_none() || lv.ll_dict.is_null() || fdp.unwrap().fd_newkey.is_none())
        {
            emsg(gettext(E_FUNCREF));
        } else {
            *pp = end_cstr;
        }
        return res;
    }

    if lv.ll_name.is_none() {
        *pp = end_cstr;

        return None;
    }

    if let Some(ref exp_name) = lv.ll_exp_name {
        let exp_cstr = exp_name.as_c_str();
        len = exp_cstr.to_bytes().len() as i32;
        let deref = deref_func_name(
            exp_cstr,
            &mut len,
            partial.as_deref_mut(),
            (flags & TFN_NO_AUTOLOAD) != 0,
            None,
        );
        if !ptr::eq(deref.as_ptr(), exp_cstr.as_ptr()) {
            name = Some(deref);
        }
    } else if (flags & TFN_NO_DEREF) == 0 {
        len = ((end_cstr.as_ptr() as isize) - (pp.as_ptr() as isize)) as i32;

        let deref = deref_func_name(pp, &mut len, partial, (flags & TFN_NO_AUTOLOAD) != 0, None);

        if !ptr::eq(deref.as_ptr(), pp.as_ptr()) {
            name = Some(deref);
        }
    }
    if let Some(name_cstr) = name {
        *pp = end_cstr;

        let name_bytes = name_cstr.to_bytes();
        if let Some(tail) = name_bytes.strip_prefix(b"<SNR>") {
            let mut bytes = Vec::with_capacity(3 + tail.len());
            bytes.extend_from_slice(&[K_SPECIAL, KS_EXTRA, KE_SNR]);
            bytes.extend_from_slice(tail);
            return Some(bytes_to_cstring(&bytes));
        }
        return Some(name_cstr.into_owned());
    }

    if let Some(ref exp_name) = lv.ll_exp_name {
        len = exp_name.as_bytes().len() as i32;
        if lead <= 2
            && lv.ll_name_off == 0
            && lv.ll_name_len >= 2
            && exp_name.as_bytes().starts_with(b"s:")
        {
            lv.advance_ll_name(2);
            lv.ll_name_len -= 2;
            len -= 2;
            lead = 2;
        }
    } else {
        if lead == 2
            || lv
                .ll_name_cstr()
                .is_some_and(|s| s.to_bytes().starts_with(b"g:"))
        {
            lv.advance_ll_name(2);
            lv.ll_name_len = lv.ll_name_len.wrapping_sub(2);
        }
        let end_len = end_cstr.to_bytes().len();
        len = lv
            .ll_name_cstr()
            .unwrap_or(c"")
            .to_bytes()
            .len()
            .saturating_sub(end_len) as i32;
    }

    let mut sid_buflen = 0usize;
    let mut sid_buf = [0u8; 20];
    let ll_name_bytes = lv.ll_name_cstr().map_or(&[][..], CStr::to_bytes);

    if skip {
        lead = 0;
    } else if lead > 0 {
        lead = 3;
        if lv
            .ll_exp_name
            .as_ref()
            .is_some_and(|s| eval_fname_sid(s.as_bytes()))
            || eval_fname_sid(pp.to_bytes())
        {
            if g_st().current_sctx.get().sc_sid <= 0 {
                emsg(gettext(E_USINGSID));
                return None;
            }
            sid_buflen =
                format_to_buf!(&mut sid_buf, c"%d_", g_st().current_sctx.get().sc_sid) as usize;
            lead += sid_buflen as i32;
        }
    } else if (flags & TFN_INT) == 0
        && builtin_function(&ll_name_bytes[..lv.ll_name_len.min(ll_name_bytes.len())])
    {
        semsg!(
            gettext(c"E128: Function name must start with a capital or \"s:\": %s"),
            start_cstr,
        );
        return None;
    }

    if !skip && (flags & TFN_QUIET) == 0 && (flags & TFN_NO_DEREF) == 0 {
        let check_len = lv
            .ll_name_len
            .min(len.max(0) as usize)
            .min(ll_name_bytes.len());
        if ll_name_bytes[..check_len].contains(&b':') {
            semsg!(
                gettext(c"E884: Function name cannot contain a colon: %s"),
                start_cstr,
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

    bytes.extend_from_slice(&ll_name_bytes[..(len.max(0) as usize).min(ll_name_bytes.len())]);

    *pp = end_cstr;

    Some(bytes_to_cstring(&bytes))
}

pub fn get_scriptlocal_funcname(funcname: Option<&CStr>) -> Option<std::ffi::CString> {
    let funcname = funcname?;

    let bytes = funcname.to_bytes();
    let rest = if let Some(r) = bytes.strip_prefix(b"s:") {
        r
    } else {
        bytes.strip_prefix(b"<SID>")?
    };

    if !crate::runtime::script_id_valid(g_st().current_sctx.get().sc_sid) {
        emsg(gettext(E_USINGSID));
        return None;
    }

    let prefix = format!("<SNR>{}_", g_st().current_sctx.get().sc_sid);
    let mut newname = Vec::with_capacity(prefix.len() + rest.len());
    newname.extend_from_slice(prefix.as_bytes());
    newname.extend_from_slice(rest);
    std::ffi::CString::new(newname).ok()
}

/// Call `trans_function_name()`, except that a lambda is returned as-is.
pub fn save_function_name(
    name: &mut &CStr,
    skip: bool,
    flags: i32,
    mut fudi: Option<&mut Funcdict>,
) -> Option<std::ffi::CString> {
    let start = *name;
    let mut p = start;
    let saved: Option<std::ffi::CString>;

    if let Some(tail) = p.to_bytes().strip_prefix(b"<lambda>") {
        let mut rem = tail;
        getdigits(&mut rem, false, 0);
        let consumed = 8 + tail.len() - rem.len();
        p = crate::memory::CStrSubslice::advance(p, consumed);
        saved = Some(bytes_to_cstring(&start.to_bytes()[..consumed]));
        if fudi.is_some() {
            *fudi.as_deref_mut().unwrap() = Funcdict::new();
        }
    } else {
        saved = trans_function_name(
            &mut p,
            skip,
            flags,
            (fudi.is_some()).then(|| fudi.unwrap()),
            None,
        );
    }

    *name = p;

    saved
}

fn list_functions(mut regmatch: Option<&mut Regmatch>) {
    let prev_ht_changed = UF_STATE.with(|s| s.func_hashtab_changed.get());
    let funcs = func_tbl_values();

    msg_ext_set_kind(c"list_cmd");
    for fp_id in funcs {
        if g_st().got_int.get() {
            break;
        }
        let Some(fp) = fp_id.rc() else {
            continue;
        };
        let key = uf2hikey(&fp);
        let matches = if let Some(ref mut regmatch) = regmatch {
            !key.to_bytes_with_nul()[0].is_ascii_digit() && vim_regexec(regmatch, key, 0)
        } else {
            !message_filtered(key) && !func_name_refcount(key)
        };
        if matches {
            if !list_func_head(&fp, false, false) {
                return;
            }
            if function_list_modified(prev_ht_changed) {
                return;
            }
        }
    }
}

fn list_functions_matching_pat(eap: &mut Exarg) -> crate::strings::OwnedSlice {
    let pat_start = eap.arg.wrapping_add(1);
    let mut p = pat_start.advance_to_cstr(skip_regexp(pat_start.as_cstr(), b'/' as i32, 1));
    if !eap.skip {
        let mut regmatch: Regmatch = Regmatch::new();
        let pat_len = p.offset_from(&pat_start);
        let pat_cstr = bytes_to_cstring(pat_start.slice(pat_len));
        regmatch.set_prog(vim_regcomp(&pat_cstr, RE_MAGIC));
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = opt_vars().p_ic.get();
            list_functions(Some(&mut regmatch));
            regmatch.free_prog();
        }
    }
    if p.first_byte() == b'/' {
        p = p.wrapping_add(1);
    }

    p
}

fn list_one_function(
    eap: &mut Exarg,
    name: Option<&CStr>,
    p_off: usize,
) -> Option<std::rc::Rc<UserFunc>> {
    let p = eap.arg.wrapping_add(p_off);
    if ends_excmd(
        crate::charset::skipwhite_slice(p.as_bytes())
            .first()
            .copied()
            .unwrap_or(0),
    ) == 0
    {
        semsg!(gettext(E_TRAILING_ARG), p);
        return None;
    }

    eap.nextcmd = p.advance_to_opt_cstr(check_nextcmd(p.as_cstr()));

    if !eap.nextcmd.is_null()
        && let Some(b) = eap.arg_cstr_buf_mut().get_mut(p_off)
    {
        *b = 0;
    }

    if eap.skip || g_st().got_int.get() {
        return None;
    }

    let Some(fp) = find_func(name) else {
        emsg_funcname(c"E123: Undefined function: %s", name.unwrap_or(c""));
        return None;
    };

    let prev_ht_changed = UF_STATE.with(|s| s.func_hashtab_changed.get());

    msg_ext_set_kind(c"list_cmd");
    if !list_func_head(&fp, !eap.forceit, eap.forceit) {
        return Some(fp);
    }

    let mut j = 0;
    while j < fp.uf_lines.borrow().len() as i32 && !g_st().got_int.get() {
        let line_opt = fp.uf_lines.borrow()[j as usize].clone();
        let Some(ref line_cstr) = line_opt else {
            j += 1;
            continue;
        };
        msg_putchar(b'\n' as i32);
        if !eap.forceit {
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
        msg_prt_line(line_cstr.as_bytes(), false);
        line_breakcheck();
        j += 1;
    }

    if !g_st().got_int.get() {
        msg_putchar(b'\n' as i32);
        if !function_list_modified(prev_ht_changed) {
            msg_puts(if eap.forceit {
                c"endfunction"
            } else {
                c"   endfunction"
            });
        }
    }

    Some(fp)
}

fn get_function_body(
    eap: &mut Exarg,
    newlines: &mut Vec<Option<std::ffi::CString>>,
    mut line_arg_opt: Option<&CStr>,
    show_block: bool,
) -> bool {
    let mut saved_wait_return = g_st().need_wait_return.get();
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
        if g_st().key_typed.get() {
            g_st().msg_scroll.set(1);
            saved_wait_return = false;
        }
        g_st().need_wait_return.set(false);

        let line_arg_cstr: std::ffi::CString;
        let mut line_arg_base: Option<&CStr> = None;

        let theline_opt: Option<&CStr> = if let Some(la) = line_arg_opt {
            line_arg_base = Some(la);
            let rem_bytes = la.to_bytes();
            let line_len = rem_bytes
                .iter()
                .position(|&b| b == b'\n')
                .unwrap_or(rem_bytes.len());
            line_arg_cstr = bytes_to_cstring(&rem_bytes[..line_len]);
            let step = if line_len < rem_bytes.len() {
                line_len + 1
            } else {
                line_len
            };
            line_arg_opt = Some(&la[step..]);
            Some(line_arg_cstr.as_c_str())
        } else {
            line_to_free = if eap.ea_getline.is_none() {
                getcmdline(b':' as i32, 0, indent, do_concat)
            } else {
                eap.ea_getline.call(b':' as i32, indent, do_concat)
            };
            line_to_free.as_deref()
        };
        if g_st().key_typed.get() {
            g_st().lines_left.set(g_st().rows_global.get() - 1);
        }
        let Some(theline_cstr) = theline_opt else {
            if let Some(ref marker) = skip_until {
                semsg!(gettext(E_MISSING_HEREDOC_END_MARKER_STR), marker,);
            } else {
                emsg(gettext(c"E126: Missing :endfunction"));
            }
            g_st()
                .need_wait_return
                .set(g_st().need_wait_return.get() | (saved_wait_return));
            return ret;
        };
        let theline_bytes = theline_cstr.to_bytes();
        if show_block {
            debug_assert!(indent >= 0);
            ui_ext_cmdline_block_append(indent as usize, theline_bytes);
        }

        let mut sourcing_lnum_off = get_sourced_lnum(&eap.ea_getline);
        let cur_sourcing_lnum = sourcing_lnum();
        if cur_sourcing_lnum < sourcing_lnum_off {
            sourcing_lnum_off -= cur_sourcing_lnum;
        } else {
            sourcing_lnum_off = 0;
        }

        if let Some(ref marker) = skip_until {
            let no_leading_ws = crate::charset::skipwhite_len(theline_bytes) == 0;
            if heredoc_trimmed.is_none()
                || (is_heredoc && no_leading_ws)
                || heredoc_trimmed.as_ref().is_some_and(|ht| {
                    theline_bytes
                        .starts_with(&ht.as_bytes()[..heredoc_trimmedlen.min(ht.as_bytes().len())])
                })
            {
                let p_off = if heredoc_trimmed.is_none() || (is_heredoc && no_leading_ws) {
                    0
                } else {
                    heredoc_trimmedlen.min(theline_bytes.len())
                };
                if &theline_bytes[p_off..] == marker.to_bytes() {
                    skip_until = None;
                    heredoc_trimmed = None;
                    heredoc_trimmedlen = 0;
                    do_concat = true;
                    is_heredoc = false;
                }
            }
        } else {
            let mut p_cstr = theline_cstr;
            while p_cstr
                .to_bytes()
                .first()
                .is_some_and(|&b| ascii_iswhite(b) || b == b':')
            {
                p_cstr = &p_cstr[1..];
            }

            if checkforcmd(p_cstr.to_bytes(), b"endfunction", 4)
                .inspect(|&n| p_cstr = &p_cstr[n..])
                .is_some()
            {
                let prev_nesting = nesting;
                nesting -= 1;
                if prev_nesting == 0 {
                    if p_cstr.to_bytes().first() == Some(&b'!') {
                        p_cstr = &p_cstr[1..];
                    }
                    let mut nextcmd_opt: Option<&CStr> = None;
                    if p_cstr.to_bytes().first() == Some(&b'|') {
                        nextcmd_opt = Some(if let Some(la_base) = line_arg_base {
                            let off =
                                (p_cstr.as_ptr() as usize) - (theline_cstr.as_ptr() as usize) + 1;
                            &la_base[off..]
                        } else {
                            &p_cstr[1..]
                        });
                    } else if let Some(la) = line_arg_opt
                        && !crate::charset::skipwhite_slice(la.to_bytes()).is_empty()
                    {
                        nextcmd_opt = Some(la);
                    } else if !p_cstr.is_empty()
                        && p_cstr.to_bytes().first() != Some(&b'"')
                        && opt_vars().p_verbose.get() > 0
                    {
                        swmsg!(
                            true,
                            gettext(c"W22: Text found after :endfunction: %s"),
                            p_cstr,
                        );
                    }
                    if let Some(nextcmd) = nextcmd_opt {
                        if let Some(ref ltf) = line_to_free {
                            let off = (nextcmd.as_ptr() as usize) - (ltf.as_ptr() as usize);
                            let new_buf: Vec<u8> = ltf.as_bytes_with_nul().to_vec();
                            eap.set_cmdline_vec(new_buf);
                            eap.nextcmd = eap.cmdline_ptr().wrapping_add(off);
                        } else {
                            eap.nextcmd = crate::strings::OwnedSlice::from_cstr(nextcmd);
                        }
                    }
                    break;
                }
            }

            let p_bytes = p_cstr.to_bytes();
            if indent > 2 && p_bytes.starts_with(b"end") {
                indent -= 2;
            } else if p_bytes.starts_with(b"if")
                || p_bytes.starts_with(b"wh")
                || p_bytes.starts_with(b"for")
                || p_bytes.starts_with(b"try")
            {
                indent += 2;
            }

            if checkforcmd(p_cstr.to_bytes(), b"function", 2)
                .inspect(|&n| p_cstr = &p_cstr[n..])
                .is_some()
            {
                if p_cstr.to_bytes().first() == Some(&b'!') {
                    p_cstr = crate::charset::skipwhite_at(p_cstr, 1);
                }
                let script_off = eval_fname_script(p_cstr.to_bytes());
                p_cstr = &p_cstr[script_off..];
                let _ = trans_function_name(&mut p_cstr, true, 0, None, None);
                if crate::charset::skipwhite(p_cstr).to_bytes().first() == Some(&b'(') {
                    if nesting == MAX_FUNC_NESTING - 1 {
                        emsg(gettext(E_FUNCTION_NESTING_TOO_DEEP));
                    } else {
                        nesting += 1;
                        indent += 2;
                    }
                }
            }

            p_cstr = skip_range(p_cstr, None);
            let tp_cstr = p_cstr;
            if (checkforcmd(p_cstr.to_bytes(), b"append", 1)
                .inspect(|&n| p_cstr = &p_cstr[n..])
                .is_some()
                || checkforcmd(p_cstr.to_bytes(), b"change", 1)
                    .inspect(|&n| p_cstr = &p_cstr[n..])
                    .is_some()
                || checkforcmd(p_cstr.to_bytes(), b"insert", 1)
                    .inspect(|&n| p_cstr = &p_cstr[n..])
                    .is_some())
                && {
                    let b0 = p_cstr.to_bytes().first().copied().unwrap_or(0);
                    b0 == b'!' || b0 == b'|' || ascii_iswhite_nl_or_nul(i32::from(b0))
                }
            {
                skip_until = Some(c".".to_owned());
            } else {
                p_cstr = tp_cstr;
            }

            let mut arg_cstr = crate::charset::skipwhite_at(
                p_cstr,
                crate::charset::skiptowhite_len(p_cstr.to_bytes()),
            );
            let p_bytes = p_cstr.to_bytes();
            let p0 = p_bytes.first().copied().unwrap_or(0);
            let p1 = p_bytes.get(1).copied().unwrap_or(0);
            let p2 = p_bytes.get(2).copied().unwrap_or(0);
            let p3 = p_bytes.get(3).copied().unwrap_or(0);
            let is_script_heredoc = arg_cstr.to_bytes().starts_with(b"<<")
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
                p_cstr = crate::charset::skipwhite_at(arg_cstr, 2);
                let p_bytes = p_cstr.to_bytes();
                if p_bytes.starts_with(b"trim")
                    && p_bytes.get(4).is_none_or(|&b| ascii_iswhite(b as i32))
                {
                    p_cstr = crate::charset::skipwhite_at(p_cstr, 4);
                    heredoc_trimmedlen = crate::charset::skipwhite_len(theline_bytes);
                    heredoc_trimmed =
                        Some(std::ffi::CString::new(&theline_bytes[..heredoc_trimmedlen]).unwrap());
                }
                if p_cstr.is_empty() {
                    skip_until = Some(c".".to_owned());
                } else {
                    let p_bytes = p_cstr.to_bytes();
                    let wlen = crate::charset::skiptowhite_len(p_bytes);
                    skip_until = Some(std::ffi::CString::new(&p_bytes[..wlen]).unwrap());
                }
                do_concat = false;
                is_heredoc = true;
            }

            if !is_heredoc {
                arg_cstr = p_cstr;
                if checkforcmd(arg_cstr.to_bytes(), b"let", 2)
                    .inspect(|&n| arg_cstr = &arg_cstr[n..])
                    .is_some()
                    || checkforcmd(p_cstr.to_bytes(), b"const", 5)
                        .inspect(|&n| p_cstr = &p_cstr[n..])
                        .is_some()
                {
                    let mut var_count = 0;
                    let mut semicolon = 0;
                    let arg_after = skip_var_list(arg_cstr, &mut var_count, &mut semicolon, true)
                        .map(crate::charset::skipwhite);
                    if let Some(arg_ws) = arg_after
                        && arg_ws.to_bytes().starts_with(b"=<<")
                    {
                        p_cstr = crate::charset::skipwhite_at(arg_ws, 3);
                        let mut has_trim = false;
                        loop {
                            let p_bytes = p_cstr.to_bytes();
                            if p_bytes.starts_with(b"trim")
                                && p_bytes.get(4).is_none_or(|&b| ascii_iswhite(b as i32))
                            {
                                p_cstr = crate::charset::skipwhite_at(p_cstr, 4);
                                has_trim = true;
                                continue;
                            }
                            if p_bytes.starts_with(b"eval")
                                && p_bytes.get(4).is_none_or(|&b| ascii_iswhite(b as i32))
                            {
                                p_cstr = crate::charset::skipwhite_at(p_cstr, 4);
                                continue;
                            }
                            break;
                        }
                        if has_trim {
                            heredoc_trimmedlen = crate::charset::skipwhite_len(theline_bytes);
                            heredoc_trimmed = Some(
                                std::ffi::CString::new(&theline_bytes[..heredoc_trimmedlen])
                                    .unwrap(),
                            );
                        }
                        let p_bytes = p_cstr.to_bytes();
                        let wlen = crate::charset::skiptowhite_len(p_bytes);
                        skip_until = Some(std::ffi::CString::new(&p_bytes[..wlen]).unwrap());
                        do_concat = false;
                        is_heredoc = true;
                    }
                }
            }
        }

        newlines.push(Some(theline_cstr.to_owned()));

        while sourcing_lnum_off > 0 {
            sourcing_lnum_off -= 1;
            newlines.push(None);
        }

        if line_arg_opt.is_some_and(CStr::is_empty) {
            line_arg_opt = None;
        }
    }

    if g_st().did_emsg.get() == 0 {
        ret = true;
    }

    g_st()
        .need_wait_return
        .set(g_st().need_wait_return.get() | (saved_wait_return));
    ret
}

/// `:function`
pub fn ex_function(eap: &mut Exarg) {
    let mut line_arg: Option<&CStr> = None;
    let mut newargs: Vec<std::ffi::CString> = Vec::new();
    let mut default_args: Vec<std::ffi::CString> = Vec::new();
    let mut newlines: Vec<Option<std::ffi::CString>> = Vec::new();
    let mut varargs: i32 = 0;
    let mut flags: i32 = 0;
    let mut fp = UfuncId::NULL;
    let mut free_fp = false;
    let mut overwrite = false;
    let mut fudi = Funcdict::new();
    let mut show_block = false;

    if ends_excmd(eap.arg.first_byte()) != 0 {
        if !eap.skip {
            list_functions(None);
        }
        eap.nextcmd = eap
            .arg
            .advance_to_opt_cstr(check_nextcmd(eap.arg.as_cstr()));
        return;
    }

    if eap.arg.first_byte() == b'/' {
        let p = list_functions_matching_pat(eap);
        eap.nextcmd = p.advance_to_opt_cstr(check_nextcmd(p.as_cstr()));
        return;
    }

    let arg_slice = eap.arg.clone();
    let mut p_cstr = arg_slice.as_cstr();
    let mut name_owned =
        save_function_name(&mut p_cstr, eap.skip, TFN_NO_AUTOLOAD, Some(&mut fudi));
    let paren = vim_strchr(p_cstr, b'(' as i32).is_some();
    if name_owned.is_none() && (fudi.fd_dict.is_null() || !paren) && !eap.skip {
        if !aborting() {
            if let Some(ref newkey) = fudi.fd_newkey {
                semsg!(gettext(E_DICTKEY), newkey);
            }
            return;
        }
        eap.skip = true;
    }

    let saved_did_emsg = g_st().did_emsg.get();
    g_st().did_emsg.set(0);

    macro_rules! ret_free {
        () => {{
            drop(name_owned);
            g_st()
                .did_emsg
                .set(g_st().did_emsg.get() | (saved_did_emsg));
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
                func_clear_free(&fp.unwrap_rc(), false);
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
        let p_off = (p_cstr.as_ptr() as usize) - (arg_slice.as_cstr().as_ptr() as usize);
        let _ = list_one_function(eap, name_owned.as_deref(), p_off);
        ret_free!();
    }

    p_cstr = crate::charset::skipwhite(p_cstr);
    if p_cstr.to_bytes().first() != Some(&b'(') {
        if !eap.skip {
            semsg!(gettext(c"E124: Missing '(': %s"), eap.arg);
            ret_free!();
        }
        if let Some(open_paren) = vim_strchr(p_cstr, b'(' as i32) {
            p_cstr = open_paren;
        }
    }
    p_cstr = crate::charset::skipwhite_at(p_cstr, 1);

    if !eap.skip {
        let arg_opt = name_owned.as_deref().or(fudi.fd_newkey.as_deref());
        if let Some(arg_cstr) = arg_opt
            && (fudi.fd_di.is_null() || !tv_is_func(&fudi.fd_di.tv()))
            && name_owned.is_some()
        {
            let mut name_base = arg_cstr;
            if arg_cstr.to_bytes().first() == Some(&K_SPECIAL) {
                if let Some(underscore) = vim_strchr(arg_cstr, b'_' as i32) {
                    name_base = &underscore[1..];
                } else {
                    name_base = &arg_cstr[3..];
                }
            }
            let name_bytes = name_base.to_bytes();
            let valid = name_bytes.iter().enumerate().all(|(i, &b)| {
                if i == 0 {
                    eval_isnamec1(b)
                } else {
                    eval_isnamec(b)
                }
            });
            if !valid {
                emsg_funcname(E_INVARG2, arg_cstr);
                ret_free!();
            }
        }
        if !fudi.fd_dict.is_null() && fudi.fd_dict.dv_scope.get() == VAR_DEF_SCOPE {
            emsg(gettext(c"E862: Cannot use g: here"));
            ret_free!();
        }
    }

    if !get_function_args(
        &mut p_cstr,
        b')',
        Some(&mut newargs),
        Some(&mut varargs),
        Some(&mut default_args),
        eap.skip,
    ) {
        errret_2!();
    }

    if g_st().key_typed.get() && ui_has(K_UICMDLINE) {
        show_block = true;
        ui_ext_cmdline_block_append(0, eap.cmd.as_bytes());
    }

    loop {
        p_cstr = crate::charset::skipwhite(p_cstr);
        let p_bytes = p_cstr.to_bytes();
        if p_bytes.starts_with(b"range") {
            flags |= FC_RANGE;
            p_cstr = &p_cstr[5..];
        } else if p_bytes.starts_with(b"dict") {
            flags |= FC_DICT;
            p_cstr = &p_cstr[4..];
        } else if p_bytes.starts_with(b"abort") {
            flags |= FC_ABORT;
            p_cstr = &p_cstr[5..];
        } else if p_bytes.starts_with(b"closure") {
            flags |= FC_CLOSURE;
            p_cstr = &p_cstr[7..];
            if UF_STATE.with(|s| s.current_funccal.get()).is_null() {
                emsg_funcname(
                    c"E932: Closure function should not be at top level: %s",
                    name_owned.as_deref().unwrap_or(c""),
                );
                erret!();
            }
        } else {
            break;
        }
    }

    if p_cstr.to_bytes().first() == Some(&b'\n') {
        line_arg = Some(&p_cstr[1..]);
    } else if !p_cstr.is_empty()
        && p_cstr.to_bytes().first() != Some(&b'"')
        && !eap.skip
        && g_st().did_emsg.get() == 0
    {
        semsg!(gettext(E_TRAILING_ARG), p_cstr);
    }

    if g_st().key_typed.get() {
        if !eap.skip && !eap.forceit {
            if !fudi.fd_dict.is_null() && fudi.fd_newkey.is_none() {
                emsg(gettext(E_FUNCDICT));
            } else if let Some(ref nm) = name_owned
                && find_func(Some(nm.as_c_str())).is_some()
            {
                emsg_funcname(E_FUNCEXTS, nm.as_c_str());
            }
        }

        if !eap.skip && g_st().did_emsg.get() != 0 {
            erret!();
        }

        if !ui_has(K_UICMDLINE) {
            msg_putchar(b'\n' as i32);
        }
        g_st().cmdline_row.set(g_st().msg_row.get());
    }

    let sourcing_lnum_top = sourcing_lnum();

    if !get_function_body(eap, &mut newlines, line_arg, show_block) || eap.skip {
        erret!();
    }

    let mut namelen = 0usize;
    if fudi.fd_dict.is_null() {
        let name_cstr = name_owned.as_deref().unwrap_or(c"");
        let v = find_var(name_cstr, name_cstr.to_bytes().len(), true, 0);
        if v.is_some_and(|v| v.tv().v_type == VarType::VarFunc) {
            emsg_funcname(
                c"E707: Function name conflicts with variable: %s",
                name_cstr,
            );
            erret!();
        }

        fp = UfuncId::from(find_func(Some(name_cstr)));
        if !fp.is_null() {
            let fp_ref = fp.unwrap_rc();
            if !eap.forceit
                && (fp_ref.uf_script_ctx.get().sc_sid != g_st().current_sctx.get().sc_sid
                    || fp_ref.uf_script_ctx.get().sc_seq == g_st().current_sctx.get().sc_seq)
            {
                emsg_funcname(E_FUNCEXTS, name_cstr);
                errret_keep!();
            }
            if fp_ref.uf_calls.get() > 0 {
                emsg_funcname(
                    c"E127: Cannot redefine function %s: It is in use",
                    name_cstr,
                );
                errret_keep!();
            }
            if fp_ref.uf_refcount.get() > 1 {
                fp_ref.uf_refcount.set(fp_ref.uf_refcount.get() - 1);

                fp_ref.uf_flags.set(fp_ref.uf_flags.get() | FC_REMOVED);

                fp = UfuncId::NULL;
                overwrite = true;
            } else {
                name_owned = None;

                func_clear_items(&fp_ref);

                fp_ref.uf_profiling.set(0);

                fp_ref.uf_prof_initialized.set(false);
            }
        }
    } else {
        let mut numbuf = [0u8; NUMBUFLEN];

        fp = UfuncId::NULL;
        if fudi.fd_newkey.is_none() && !eap.forceit {
            emsg(gettext(E_FUNCDICT));
            erret!();
        }
        if fudi.fd_di.is_null() {
            if value_check_lock(
                fudi.fd_dict.dv_lock.get(),
                Some(eap.arg.as_cstr()),
                TV_CSTRING,
            ) {
                erret!();
            }
        } else if value_check_lock(fudi.fd_di.tv().v_lock, Some(eap.arg.as_cstr()), TV_CSTRING) {
            erret!();
        }

        UF_STATE.with(|s| s.func_nr.set(s.func_nr.get() + (1)));
        namelen = format_to_buf!(&mut numbuf, c"%d", UF_STATE.with(|s| s.func_nr.get())) as usize;
        let nm = bytes_to_cstring(&numbuf[..namelen]);
        name_owned = Some(nm);
    }

    if fp.is_null() {
        let name_cstr = name_owned.as_deref().unwrap_or(c"");
        if fudi.fd_dict.is_null() && vim_strchr(name_cstr, AUTOLOAD_CHAR as i32).is_some() {
            let mut j = false;
            if let Some(sname) = sourcing_name() {
                let sn = autoload_name(name_cstr.to_bytes());
                let p_slash = vim_strchr(&sn, b'/' as i32).unwrap_or(c"");
                let plen = p_slash.to_bytes().len() as i32;
                let slen = sname.to_bytes().len() as i32;
                if slen > plen
                    && path_equal(
                        p_slash,
                        &sname.as_c_str()[(slen - plen) as usize..],
                        K_PATH_CMP_LITERAL,
                    )
                {
                    j = true;
                }
            }
            if !j {
                semsg!(
                    gettext(c"E746: Function name does not match script file name: %s",),
                    name_cstr,
                );
                erret!();
            }
        }

        let name_bytes = name_cstr.to_bytes();
        if namelen == 0 {
            namelen = name_bytes.len();
        }
        let name_slice = &name_bytes[..namelen.min(name_bytes.len())];
        fp = alloc_ufunc(name_slice).uf_id;

        if !fudi.fd_dict.is_null() {
            if fudi.fd_di.is_null() {
                let di = tv_dict_item_alloc(fudi.fd_newkey.as_ref().unwrap().as_c_str());
                fudi.fd_di = di;
                if !tv_dict_add(&fudi.fd_dict, &fudi.fd_di) {
                    crate::eval::typval::tv_dict_item_free(&fudi.fd_di);
                    erret!();
                }
            } else {
                fudi.fd_di.clear_tv();
            }

            crate::eval::typval::tv_set_string_len(&mut fudi.fd_di.tv_mut(), Some(name_slice));

            fudi.fd_di.tv_mut().v_type = VarType::VarFunc;

            flags |= FC_DICT;
        }

        if overwrite {
            FUNC_HASHTAB.with(|c| c.borrow_mut().insert(name_bytes.to_vec(), fp));
        } else if !func_hashtab_add(&fp.unwrap_rc()) {
            free_fp = true;
            erret!();
        }

        fp.unwrap_rc().uf_refcount.set(1);
    }
    let fp = fp.unwrap_rc();
    *fp.uf_args.borrow_mut() = newargs;
    *fp.uf_def_args.borrow_mut() = default_args;
    *fp.uf_lines.borrow_mut() = newlines;
    if (flags & FC_CLOSURE) != 0 {
        register_closure(&fp);
    } else {
        fp.uf_scoped.set(FunccalId::NULL);
    }
    if prof_def_func() {
        func_do_profile(&fp);
    }
    fp.uf_varargs.set(varargs);
    if g_st().sandbox.get() != 0 {
        flags |= FC_SANDBOX;
    }
    fp.uf_flags.set(flags);
    fp.uf_calls.set(0);
    let mut sctx = g_st().current_sctx.get();
    sctx.sc_lnum += sourcing_lnum_top;
    nlua_set_sctx(&mut sctx);
    fp.uf_script_ctx.set(sctx);

    ret_free!();
}

/// Return 5 if `p` starts with `<SID>` or `<SNR>` (ignoring case),
/// 2 if `p` starts with `s:`, 0 otherwise.
pub fn eval_fname_script(bytes: &[u8]) -> usize {
    if bytes.len() >= 5
        && bytes[0] == b'<'
        && (bytes[1..5].eq_ignore_ascii_case(b"SID>") || bytes[1..5].eq_ignore_ascii_case(b"SNR>"))
    {
        return 5;
    }
    if bytes.starts_with(b"s:") {
        return 2;
    }
    0
}

pub fn translated_function_exists(name: &CStr) -> bool {
    if builtin_function(name.to_bytes()) {
        return find_internal_func(name).is_some();
    }
    find_func(Some(name)).is_some()
}

pub fn function_exists(name: &CStr, no_deref: bool) -> bool {
    let mut nm = name;
    let mut n = false;
    let mut flag = TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD;

    if no_deref {
        flag |= TFN_NO_DEREF;
    }
    let p = trans_function_name(&mut nm, false, flag, None, None);
    let nm_ws = crate::charset::skipwhite(nm);

    if let Some(ref p) = p
        && (nm_ws.is_empty() || nm_ws.to_bytes().first() == Some(&b'('))
    {
        n = translated_function_exists(p);
    }
    n
}

/// Function given to `ExpandGeneric()` to obtain the list of user defined
/// function names.
pub fn get_user_func_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if idx == 0 {
        UF_STATE.with(|s| s.get_uf_done.set(0));
        GET_UF_ITEMS.with(|c| *c.borrow_mut() = func_tbl_values());
        UF_STATE.with(|s| {
            s.get_uf_changed
                .set(UF_STATE.with(|s| s.func_hashtab_changed.get()))
        });
    }
    if UF_STATE.with(|s| s.get_uf_changed.get()) == UF_STATE.with(|s| s.func_hashtab_changed.get())
        && UF_STATE.with(|s| s.get_uf_done.get()) < GET_UF_ITEMS.with(|c| c.borrow().len())
    {
        let fp = GET_UF_ITEMS
            .with(|c| c.borrow()[UF_STATE.with(|s| s.get_uf_done.get())])
            .unwrap_rc();
        UF_STATE.with(|s| s.get_uf_done.set(s.get_uf_done.get() + 1));

        if (fp.uf_flags.get() & FC_DICT) != 0 || uf2hikey(&fp).to_bytes().starts_with(b"<lambda>") {
            return Some(c"");
        }

        let mut iobuff = [0u8; IOSIZE];
        let len = cat_func_name(&mut iobuff, &fp);
        if xp.xp_context != EXPAND_USER_FUNC && fp.uf_namelen + 4 < IOSIZE {
            crate::memory::copy_bytes_to_c_buf(&mut iobuff[len..], b"(");
            if fp.uf_varargs.get() == 0 && fp.uf_args.borrow().is_empty() {
                crate::memory::copy_bytes_to_c_buf(&mut iobuff[len + 1..], b")");
            }
        }
        let item = crate::memory::cstring_until_nul(&iobuff);
        return Some(xp.set_item(item));
    }
    None
}

/// `:delfunction {name}`
pub fn ex_delfunction(eap: &mut Exarg) {
    let mut fp: Option<std::rc::Rc<UserFunc>> = None;
    let mut fudi = Funcdict::new();

    let arg_slice = eap.arg.clone();
    let mut p_cstr = arg_slice.as_cstr();
    let name_owned = trans_function_name(&mut p_cstr, eap.skip, 0, Some(&mut fudi), None);
    let p_off = (p_cstr.as_ptr() as usize) - (arg_slice.as_cstr().as_ptr() as usize);
    let p = eap.arg.wrapping_add(p_off);
    let Some(ref name_cstr) = name_owned else {
        if !fudi.fd_dict.is_null() && !eap.skip {
            emsg(gettext(E_FUNCREF));
        }
        return;
    };
    if ends_excmd(
        crate::charset::skipwhite_slice(p_cstr.to_bytes())
            .first()
            .copied()
            .unwrap_or(0),
    ) == 0
    {
        semsg!(gettext(E_TRAILING_ARG), p);
        return;
    }
    eap.nextcmd = p.advance_to_opt_cstr(check_nextcmd(p.as_cstr()));
    if !eap.nextcmd.is_null()
        && let Some(b) = eap.arg_cstr_buf_mut().get_mut(p_off)
    {
        *b = NUL as u8;
    }

    if name_cstr.as_bytes().first().is_some_and(u8::is_ascii_digit) && fudi.fd_dict.is_null() {
        if !eap.skip {
            semsg!(gettext(E_INVARG2), eap.arg);
        }
        return;
    }
    if !eap.skip {
        fp = find_func(Some(name_cstr));
    }
    drop(name_owned);

    if !eap.skip {
        let Some(fp_ref) = fp else {
            if !eap.forceit {
                semsg!(gettext(E_NOFUNC), eap.arg);
            }
            return;
        };
        if fp_ref.uf_calls.get() > 0 {
            semsg!(
                gettext(c"E131: Cannot delete function %s: It is in use"),
                eap.arg,
            );
            return;
        }
        if fp_ref.uf_refcount.get() > 2 {
            semsg!(
                gettext(c"Cannot delete function %s: It is being used internally"),
                eap.arg,
            );
            return;
        }

        if let Some(dict) = fudi.dict_opt() {
            tv_dict_item_remove(dict, fudi.di());
        } else {
            let threshold = if func_name_refcount(uf2hikey(&fp_ref)) {
                0
            } else {
                1
            };
            if fp_ref.uf_refcount.get() > threshold {
                if func_remove(&fp_ref) {
                    fp_ref.uf_refcount.set(fp_ref.uf_refcount.get() - 1);
                }
                fp_ref.uf_flags.set(fp_ref.uf_flags.get() | FC_DELETED);
            } else {
                func_clear_free(&fp_ref, false);
            }
        }
    }
}

/// Unreference a Function: decrement the reference count and free it when it
/// becomes zero.
pub fn func_unref(name: Option<&CStr>) {
    let Some(name) = name else {
        return;
    };
    if !func_name_refcount(name) {
        return;
    }

    let Ok(fp) = FUNC_HASHTAB.try_with(|c| c.borrow().get(name.to_bytes()).copied()) else {
        return;
    };
    let fp = fp.and_then(UfuncId::rc);
    if fp.is_none()
        && name.to_bytes().first().is_some_and(u8::is_ascii_digit)
        && !entered_free_all_mem()
        && !g_st().exiting.get()
    {
        internal_error(c"func_unref()");
        std::process::abort();
    }
    func_ptr_unref(fp.as_deref());
}

pub fn func_ptr_unref(fp: Option<&UserFunc>) {
    if let Some(fp) = fp {
        fp.uf_refcount.set(fp.uf_refcount.get() - (1));
        if fp.uf_refcount.get() <= 0 && fp.uf_calls.get() == 0 {
            func_clear_free(fp, false);
        }
    }
}

pub fn func_ref(name: Option<&CStr>) {
    let Some(name) = name else {
        return;
    };
    if !func_name_refcount(name) {
        return;
    }
    if let Some(fp) = find_func(Some(name)) {
        fp.uf_refcount.set(fp.uf_refcount.get() + (1));
    } else if name.to_bytes().first().is_some_and(u8::is_ascii_digit) {
        internal_error(c"func_ref()");
    }
}

pub fn func_ptr_ref(fp: Option<&UserFunc>) {
    if let Some(fp) = fp {
        fp.uf_refcount.set(fp.uf_refcount.get() + (1));
    }
}

#[inline]
fn fc_referenced(fc: &Funccall) -> bool {
    fc.fc_l_varlist.get().lv_refcount.get() != DO_NOT_FREE_CNT
        || fc.fc_l_vars.get().dv_refcount.get() != DO_NOT_FREE_CNT
        || fc.fc_l_avars.get().dv_refcount.get() != DO_NOT_FREE_CNT
        || fc.fc_refcount.get() > 0
}

fn can_free_funccal(fc: &Funccall, copy_id: i32) -> bool {
    fc.fc_l_varlist.get().lv_copy_id.get() != copy_id
        && fc.fc_l_vars.get().dv_copy_id.get() != copy_id
        && fc.fc_l_avars.get().dv_copy_id.get() != copy_id
        && fc.fc_copy_id.get() != copy_id
}

/// `:return [expr]`
pub fn ex_return(eap: &mut Exarg) {
    let arg = eap.arg.clone();
    let mut rettv: TypVal = TypVal::new();
    let mut returning = false;

    if UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        emsg(gettext(c"E133: :return not inside a function"));
        return;
    }

    let mut evalarg = Evalarg {
        eval_flags: if eap.skip { 0 } else { EVAL_EVALUATE },
    };

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }

    eap.nextcmd = crate::strings::OwnedSlice::NULL;
    if (!(arg).is_empty() && (arg).first_byte() != b'|' && (arg).first_byte() != b'\n')
        && eval0((arg).as_cstr(), &mut rettv, Some(&mut *eap), &mut evalarg)
    {
        if !eap.skip {
            returning = do_return(eap, false, true, Some(rettv));
        } else {
            tv_clear(&mut rettv);
        }
    } else if !eap.skip {
        update_force_abort();

        if !aborting() {
            returning = do_return(eap, false, true, None);
        }
    }

    if returning {
        eap.nextcmd = crate::strings::OwnedSlice::NULL;
    } else if eap.nextcmd.is_null() {
        eap.nextcmd = arg.advance_to_opt_cstr(check_nextcmd((arg).as_cstr()));
    }

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }
}

fn ex_call_inner<'a>(
    eap: &mut Exarg,
    name: &CStr,
    arg: &mut &'a CStr,
    startarg: &'a CStr,
    funcexe_init: &Funcexe,
    evalarg: &mut Evalarg,
) -> bool {
    let mut failed = false;

    for lnum in eap.line1..=eap.line2 {
        if eap.addr_count > 0 {
            if lnum > curbuf().b_ml.ml_line_count.get() {
                emsg(gettext(E_INVRANGE));
                break;
            }
            curwin().w_cursor.set_lnum(lnum);
            curwin().w_cursor.set_col(0);
            curwin().w_cursor.set_coladd(0);
        }

        *arg = startarg;

        let mut funcexe = funcexe_init.clone();
        funcexe.fe_doesrange = false;
        let mut rettv: TypVal = TypVal::new();
        rettv.v_type = VarType::VarUnknown;
        if !get_func_tv(name, -1, &mut rettv, arg, Some(evalarg), &mut funcexe) {
            failed = true;
            break;
        }

        if !handle_subscript(
            arg,
            startarg,
            &mut rettv,
            Some(&mut crate::eval::vars::Evalarg { eval_flags: 1 }),
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

fn ex_defer_inner(
    name: &CStr,
    arg: &mut &CStr,
    partial: Option<&TvPartial>,
    evalarg: &mut Evalarg,
) -> bool {
    let mut argvars: [TypVal; MAX_FUNC_ARGS + 1] = [const { TypVal::new() }; MAX_FUNC_ARGS + 1];
    let mut partial_argc = 0;
    let mut argcount = 0;

    if UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        semsg!(gettext(E_STR_NOT_INSIDE_FUNCTION), c"defer",);
        return false;
    }
    if let Some(partial) = partial {
        if !partial.pt_dict.get().is_null() {
            emsg(gettext(E_CANNOT_USE_PARTIAL_WITH_DICTIONARY_FOR_DEFER));
            return false;
        }
        if !partial.pt_argv.borrow().is_empty() {
            partial_argc = partial.pt_argv.borrow().len() as i32;
            for i in 0..partial_argc {
                tv_copy(
                    &partial.pt_argv.borrow()[i as usize],
                    &mut argvars[i as usize],
                );
            }
        }
    }
    let mut r = get_func_arguments(
        arg,
        Some(evalarg),
        0,
        &mut argvars[partial_argc as usize..],
        &mut argcount,
    );
    argcount += partial_argc;

    if r {
        if builtin_function(name.to_bytes()) {
            if let Some(fdef) = find_internal_func(name) {
                if check_internal_func(fdef, argcount) == -1 {
                    r = false;
                }
            } else {
                emsg_funcname(E_UNKNOWN_FUNCTION_STR, name);
                r = false;
            }
        } else if let Some(ufunc) = find_func(Some(name)) {
            let error = check_user_func_argcount(&ufunc, argcount);
            if error != FCERR_UNKNOWN {
                user_func_error(error, name, false);
                r = false;
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
    add_defer(name, &argvars[..argcount as usize]);
    true
}

pub fn can_add_defer() -> bool {
    if get_current_funccal().is_none() {
        semsg!(gettext(E_STR_NOT_INSIDE_FUNCTION), c"defer",);
        return false;
    }
    true
}

pub fn add_defer(name: &CStr, argvars: &[TypVal]) {
    let dr_name_cstr = name.to_owned();
    let argcount = argvars.len();
    let mut dr_argvars = vec![TypVal::new(); argcount + 1];
    dr_argvars[..argcount].clone_from_slice(argvars);
    cur_funccal().fc_defer.borrow_mut().push(DeferCall {
        dr_name: Some(dr_name_cstr),
        dr_argvars,
    });
}

fn handle_defer_one(funccal: &Funccall) {
    let mut idx = funccal.fc_defer.borrow().len() as isize - 1;
    while idx >= 0 {
        let (name_cstr, mut dr_argvars) = {
            let mut defer = funccal.fc_defer.borrow_mut();
            let dr = &mut defer[idx as usize];
            let Some(name_cstr) = dr.dr_name.take() else {
                idx -= 1;
                continue;
            };
            (name_cstr, std::mem::take(&mut dr.dr_argvars))
        };

        let mut funcexe = Funcexe::new();
        funcexe.fe_evaluate = true;

        let mut rettv: TypVal = TypVal::new();
        rettv.v_type = VarType::VarUnknown;

        let argcount = (dr_argvars.len() - 1) as i32;

        let mut estate = ExceptionState::new();
        exception_state_save(&mut estate);
        exception_state_clear();

        call_func(
            &name_cstr,
            -1,
            &mut rettv,
            argcount,
            &mut dr_argvars,
            &mut funcexe,
        );

        exception_state_restore(&mut estate);

        tv_clear(&mut rettv);
        drop(name_cstr);
        let mut i = argcount - 1;
        while i >= 0 {
            tv_clear(&mut dr_argvars[i as usize]);
            i -= 1;
        }
        idx -= 1;
    }
    funccal.fc_defer.borrow_mut().clear();
}

pub struct FunccalCallerIter(FunccalId);

impl Iterator for FunccalCallerIter {
    type Item = std::rc::Rc<Funccall>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.rc()?;
        self.0 = cur.fc_caller.get();
        Some(cur)
    }
}

pub struct FunccalScopedIter(FunccalId);

impl Iterator for FunccalScopedIter {
    type Item = std::rc::Rc<Funccall>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.rc()?;
        self.0 = cur
            .fc_func
            .get()
            .rc()
            .map_or(FunccalId::NULL, |fp| fp.uf_scoped.get());
        Some(cur)
    }
}

pub fn invoke_all_defer() {
    for fc in FunccalCallerIter(UF_STATE.with(|s| s.current_funccal.get())) {
        handle_defer_one(&fc);
    }

    for top in FUNCCAL_STACK.with(|c| c.borrow().iter().rev().copied().collect::<Vec<_>>()) {
        for fc2 in FunccalCallerIter(top) {
            handle_defer_one(&fc2);
        }
    }
}

/// `:1,25call func(arg1, arg2)` or `:defer func(arg1, arg2)`.
pub fn ex_call(eap: &mut Exarg) {
    let arg_slice = eap.arg.clone();
    let mut arg = arg_slice.as_cstr();
    let mut fudi = Funcdict::new();
    let mut partial_opt: Option<std::rc::Rc<TvPartial>> = None;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip);
    if eap.skip {
        let mut rettv: TypVal = TypVal::new();
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
        if eval0(arg, &mut rettv, Some(&mut *eap), &mut evalarg) {
            tv_clear(&mut rettv);
        }
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
        return;
    }

    let tofree_owned = trans_function_name(
        &mut arg,
        false,
        TFN_INT,
        Some(&mut fudi),
        Some(&mut partial_opt),
    );
    if let Some(ref newkey) = fudi.fd_newkey {
        semsg!(gettext(E_DICTKEY), newkey);
    }
    let Some(ref tofree_cstr) = tofree_owned else {
        return;
    };

    if !fudi.fd_dict.is_null() {
        fudi.fd_dict
            .dv_refcount
            .set(fudi.fd_dict.dv_refcount.get() + 1);
    }

    let mut len = tofree_cstr.as_bytes().len() as i32;
    let mut found_var = false;
    let has_partial = partial_opt.is_some();
    let name = deref_func_name(
        tofree_cstr,
        &mut len,
        if has_partial {
            None
        } else {
            Some(&mut partial_opt)
        },
        false,
        Some(&mut found_var),
    );

    let startarg = crate::charset::skipwhite(arg);

    if startarg.to_bytes().first().copied().unwrap_or(0) != b'(' {
        semsg!(gettext(E_MISSINGPAREN), eap.arg);
        tv_dict_unref(fudi.fd_dict.opt());
        return;
    }

    let failed = if eap.cmdidx == Cmdidx::CmdDefer {
        arg = startarg;
        !ex_defer_inner(&name, &mut arg, partial_opt.as_deref(), &mut evalarg)
    } else {
        let mut funcexe = Funcexe::new();
        funcexe.fe_partial = crate::eval::typval::PartialId::from(partial_opt);
        funcexe.fe_selfdict = fudi.fd_dict.clone();
        funcexe.fe_firstline = eap.line1;
        funcexe.fe_lastline = eap.line2;
        funcexe.fe_found_var = found_var;
        funcexe.fe_evaluate = true;
        ex_call_inner(eap, &name, &mut arg, startarg, &funcexe, &mut evalarg)
    };

    if (!aborting() || g_st().did_throw.get()) && (!failed || eap.cstack_mut().cs_trylevel > 0) {
        if ends_excmd(arg.to_bytes().first().copied().unwrap_or(0)) == 0 {
            if !failed && !aborting() {
                g_st().emsg_severe.set(true);
                semsg!(gettext(E_TRAILING_ARG), arg);
            }
        } else {
            eap.nextcmd = arg_slice.advance_to_opt_cstr(check_nextcmd(arg));
        }
    }

    tv_dict_unref(fudi.fd_dict.opt());
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
        cur_funccal().fc_returned.set(0);
    }

    let idx = cleanup_conditionals(&mut *cstack, 0, true);
    if idx >= 0 {
        cstack.cs_pending[idx as usize] = CSTP_RETURN as u8;

        if reanimate {
            let fc = cur_funccal();
            let mut fc_rettv = fc.fc_rettv.borrow_mut();
            rettv = Some(std::mem::replace(
                &mut *fc_rettv,
                TypVal {
                    v_type: VarType::VarNumber,
                    v_lock: VAR_UNLOCKED,
                    vval: crate::eval::typval::TypvalVval::Number(0),
                },
            ));
        }

        cstack.cs_rettv[idx as usize] = rettv.clone();
        report_make_pending(CSTP_RETURN, PendingVal::Return(rettv));
    } else {
        cur_funccal().fc_returned.set(1);

        if !reanimate && let Some(tv) = rettv {
            let fc = cur_funccal();
            let mut fc_rettv = fc.fc_rettv.borrow_mut();
            tv_clear(&mut fc_rettv);
            *fc_rettv = tv;
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
    let s_bytes = tofree.as_ref().map_or(&[][..], |tf| tf.as_bytes());
    let slen = s_bytes.len();

    let mut iobuff = [0u8; IOSIZE];
    crate::memory::copy_bytes_to_c_buf(&mut iobuff, b":return ");
    crate::memory::copy_bytes_to_c_buf(&mut iobuff[8..], s_bytes);
    let mut iobuff_len = 8 + slen;
    if iobuff_len >= IOSIZE {
        crate::memory::copy_bytes_to_c_buf(&mut iobuff[IOSIZE - 4..], b"...");
        iobuff_len = IOSIZE - 1;
    }
    drop(tofree);
    bytes_to_cstring(&iobuff[..iobuff_len])
}

/// Get next function line.
pub fn get_func_line(
    _c: i32,
    fcp: &Funccall,
    _indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    let fp = fcp.func_rc();
    let retval: Option<std::ffi::CString>;

    if fcp.fc_dbg_tick.get() != g_st().debug_tick.get() {
        fcp.fc_breakpoint
            .set(dbg_find_breakpoint(false, uf2hikey(&fp), sourcing_lnum()));
        fcp.fc_dbg_tick.set(g_st().debug_tick.get());
    }
    if g_st().do_profiling.get() == PROF_YES {
        func_line_end(fcp);
    }

    let lines = fp.uf_lines.borrow();
    let lines_len = lines.len() as i32;
    if ((fp.uf_flags.get() & FC_ABORT) != 0 && g_st().did_emsg.get() != 0 && !aborted_in_try())
        || fcp.fc_returned.get() != 0
    {
        retval = None;
    } else {
        while fcp.fc_linenr.get() < lines_len && lines[fcp.fc_linenr.get() as usize].is_none() {
            fcp.fc_linenr.set(fcp.fc_linenr.get() + 1);
        }
        if fcp.fc_linenr.get() >= lines_len {
            retval = None;
        } else {
            let line = lines[fcp.fc_linenr.get() as usize].clone();
            drop(lines);
            fcp.fc_linenr.set(fcp.fc_linenr.get() + 1);
            retval = line;
            set_sourcing_lnum(fcp.fc_linenr.get());
            if g_st().do_profiling.get() == PROF_YES {
                func_line_start(fcp);
            }
        }
    }

    let cur_lnum = sourcing_lnum();
    if fcp.fc_breakpoint.get() != 0 && fcp.fc_breakpoint.get() <= cur_lnum {
        dbg_breakpoint(Some(uf2hikey(&fp)), cur_lnum);
        fcp.fc_breakpoint
            .set(dbg_find_breakpoint(false, uf2hikey(&fp), sourcing_lnum()));
        fcp.fc_dbg_tick.set(g_st().debug_tick.get());
    }

    retval
}

pub fn func_has_ended(fcp: &Funccall) -> i32 {
    if ((fcp.func_rc().uf_flags.get() & FC_ABORT) != 0
        && g_st().did_emsg.get() != 0
        && !aborted_in_try())
        || fcp.fc_returned.get() != 0
    {
        1
    } else {
        0
    }
}

pub fn func_has_abort(fcp: &Funccall) -> i32 {
    fcp.func_rc().uf_flags.get() & FC_ABORT
}

/// Turn `dict.Func` into a partial for `Func` bound to `dict`.
pub fn make_partial(selfdict: &TvDict, rettv: &mut TypVal) {
    let mut fp: Option<std::rc::Rc<UserFunc>> = None;
    let mut fname_buf = [0u8; FLEN_FIXED + 1];
    let mut error = 0;

    if rettv.v_type == VarType::VarPartial
        && let Some(pt_ref) = rettv.vval.v_partial().opt()
        && let Some(pt_fp) = pt_ref.func_rc()
    {
        fp = Some(pt_fp);
    } else {
        let fname_opt = if rettv.v_type == VarType::VarFunc || rettv.v_type == VarType::VarString {
            rettv.vval.v_string_cstr()
        } else if let Some(pt_ref) = rettv.vval.v_partial_opt() {
            pt_ref.name_cstr()
        } else {
            None
        };
        if let Some(fname_cstr) = fname_opt {
            let mut tofree: Option<std::ffi::CString> = None;
            let fname = fname_trans_sid(fname_cstr, &mut fname_buf, &mut tofree, &mut error);
            fp = find_func(Some(fname));
        } else {
            rettv.v_type = VarType::VarFunc;
            rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
        }
    }

    if let Some(fp_ref) = fp
        && (fp_ref.uf_flags.get() & FC_DICT) != 0
    {
        let pt = crate::eval::typval::tv_partial_alloc();
        pt.pt_refcount.set(1);
        pt.pt_dict.set(selfdict.id());
        selfdict.dv_refcount.set(selfdict.dv_refcount.get() + 1);
        pt.pt_auto.set(true);
        if rettv.v_type == VarType::VarFunc || rettv.v_type == VarType::VarString {
            pt.set_name(crate::eval::typval::tv_take_string(rettv));
        } else {
            let ret_pt = rettv.vval.v_partial().unwrap_rc();
            if let Some(ret_name) = ret_pt.pt_name.get() {
                func_ref(Some(ret_name));
                pt.set_name(Some(ret_name.clone()));
            } else if let Some(ret_fp) = ret_pt.func_rc() {
                func_ptr_ref(Some(&ret_fp));
                pt.set_func(Some(ret_fp));
            }
            if !ret_pt.pt_argv.borrow().is_empty() {
                let pt_argc = ret_pt.pt_argv.borrow().len();
                let mut dst_argv = pt.pt_argv.borrow_mut();
                dst_argv.resize(pt_argc, TypVal::new());
                for i in 0..pt_argc {
                    tv_copy(&ret_pt.pt_argv.borrow()[i], &mut dst_argv[i]);
                }
            }
            partial_unref(Some(&ret_pt));
        }
        rettv.v_type = VarType::VarPartial;
        rettv.vval =
            crate::eval::typval::TypvalVval::Partial(crate::eval::typval::PartialId::from(pt));
    }
}

pub fn func_name(fcp: &Funccall) -> std::ffi::CString {
    uf2hikey(&fcp.func_rc()).to_owned()
}

pub fn func_level(fcp: &Funccall) -> i32 {
    fcp.fc_level.get()
}

pub fn current_func_returned() -> i32 {
    cur_funccal().fc_returned.get()
}

pub fn free_unref_funccal(copy_id: i32, testing: i32) -> bool {
    let mut did_free = false;
    let mut did_free_funccal = false;

    let mut i = 0usize;
    loop {
        let removed = PREVIOUS_FUNCCAL.with(|c| {
            let mut v = c.borrow_mut();
            if i >= v.len() {
                return None;
            }
            let fc_id = v[i];
            if fc_id.rc().is_none_or(|fc| can_free_funccal(&fc, copy_id)) {
                Some(Ok(v.remove(i)))
            } else {
                Some(Err(()))
            }
        });
        match removed {
            None => break,
            Some(Ok(fc_id)) => {
                free_funccal_slot(fc_id);
                did_free = true;
                did_free_funccal = true;
            }
            Some(Err(())) => {
                i += 1;
            }
        }
    }
    if did_free_funccal {
        garbage_collect(testing != 0);
    }
    did_free
}

pub fn get_funccal() -> Option<std::rc::Rc<Funccall>> {
    let mut funccal = UF_STATE.with(|s| s.current_funccal.get()).rc()?;
    if g_st().debug_backtrace_level.get() > 0 {
        for i in 0..g_st().debug_backtrace_level.get() {
            if let Some(temp_funccal) = funccal.fc_caller.get().rc() {
                funccal = temp_funccal;
            } else {
                g_st().debug_backtrace_level.set(i);
                break;
            }
        }
    }
    Some(funccal)
}

#[inline]
fn get_funccal_for_vars_opt() -> Option<std::rc::Rc<Funccall>> {
    if UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        return None;
    }
    let funccal = get_funccal()?;
    (funccal.fc_l_vars.get().dv_refcount.get() != 0).then_some(funccal)
}

pub fn get_funccal_local_dict() -> Option<DictId> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let d = fc.fc_l_vars.get();
        (!d.is_null()).then_some(d)
    })
}

pub fn get_funccal_local_ht() -> Option<*const Hashtab> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let d = fc.fc_l_vars.get();
        (!d.is_null()).then(|| &raw const d.dv_hashtab)
    })
}

pub fn get_funccal_local_var() -> Option<DictitemId> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let v = fc.fc_l_vars_var.get();
        if v.is_null() { None } else { Some(v) }
    })
}

pub fn get_funccal_args_dict() -> Option<DictId> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let d = fc.fc_l_avars.get();
        (!d.is_null()).then_some(d)
    })
}

pub fn get_funccal_args_ht() -> Option<*const Hashtab> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let d = fc.fc_l_avars.get();
        (!d.is_null()).then(|| &raw const d.dv_hashtab)
    })
}

pub fn get_funccal_args_var() -> Option<DictitemId> {
    get_funccal_for_vars_opt().and_then(|fc| {
        let v = fc.fc_l_avars_var.get();
        if v.is_null() { None } else { Some(v) }
    })
}

pub fn list_func_vars(first: &mut i32) {
    if !UF_STATE.with(|s| s.current_funccal.get()).is_null()
        && cur_funccal().fc_l_vars.get().dv_refcount.get() > 0
    {
        list_hashtable_vars(
            &cur_funccal().fc_l_vars.get().dv_hashtab,
            c"l:",
            0,
            &mut *first,
        );
    }
}

pub fn get_current_funccal_dict(ht: &Hashtab) -> Option<DictId> {
    if !UF_STATE.with(|s| s.current_funccal.get()).is_null()
        && ptr::eq(ht, &raw const cur_funccal().fc_l_vars.get().dv_hashtab)
    {
        return Some(cur_funccal().fc_l_vars.get());
    }
    None
}

pub fn find_hi_in_scoped_ht(name: &CStr, pdict: &mut Option<DictId>) -> Option<usize> {
    if UF_STATE.with(|s| s.current_funccal.get()).is_null()
        || cur_funccal().func_rc().uf_scoped.get().is_null()
    {
        return None;
    }

    let old_current_funccal = UF_STATE.with(|s| s.current_funccal.get());
    let mut found_idx: Option<usize> = None;
    let namelen = name.to_bytes().len();
    let mut varname_opt: Option<&CStr> = None;

    UF_STATE.with(|s| {
        s.current_funccal
            .set(cur_funccal().func_rc().uf_scoped.get())
    });
    while !UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        let ht_dict = find_var_ht(name, namelen, &mut varname_opt);
        if let Some(ht_dict) = ht_dict
            && let Some(varname) = varname_opt.filter(|s| !s.to_bytes().is_empty())
        {
            let len = namelen
                .saturating_sub((varname.as_ptr() as usize).wrapping_sub(name.as_ptr() as usize));
            let varname_bytes = varname.to_bytes();
            let idx = hash_find_len_idx(
                &ht_dict.dv_hashtab,
                &varname_bytes[..len.min(varname_bytes.len())],
            );
            if !hashitem_empty(&ht_dict.dv_hashtab.slot(idx)) {
                *pdict = Some(ht_dict);
                found_idx = Some(idx);
                break;
            }
        }
        let next_scoped = cur_funccal().func_rc().uf_scoped.get();
        if UF_STATE.with(|s| s.current_funccal.get()) == next_scoped {
            break;
        }
        UF_STATE.with(|s| s.current_funccal.set(next_scoped));
    }
    UF_STATE.with(|s| s.current_funccal.set(old_current_funccal));

    found_idx
}

pub fn find_var_in_scoped_ht(
    name_slice: &[u8],
    no_autoload: i32,
) -> Option<crate::eval::typval::DictitemId> {
    let name_cstr = crate::memory::bytes_to_cstring(name_slice);
    let namelen = name_slice.len();
    if UF_STATE.with(|s| s.current_funccal.get()).is_null()
        || cur_funccal().func_rc().uf_scoped.get().is_null()
    {
        return None;
    }

    let mut v: Option<crate::eval::typval::DictitemId> = None;
    let old_current_funccal = UF_STATE.with(|s| s.current_funccal.get());
    let mut varname_opt: Option<&CStr> = None;

    UF_STATE.with(|s| {
        s.current_funccal
            .set(cur_funccal().func_rc().uf_scoped.get())
    });
    while !UF_STATE.with(|s| s.current_funccal.get()).is_null() {
        let ht_dict = find_var_ht(&name_cstr, namelen, &mut varname_opt);
        if let Some(ht_dict) = ht_dict
            && let Some(varname) = varname_opt.filter(|s| !s.to_bytes().is_empty())
        {
            v = find_var_in_ht(
                &ht_dict.dv_hashtab,
                i32::from(name_slice.first().copied().unwrap_or(0)),
                varname,
                namelen.saturating_sub(
                    (varname.as_ptr() as usize).wrapping_sub(name_cstr.as_ptr() as usize),
                ),
                no_autoload,
            );
            if v.is_some() {
                break;
            }
        }
        let next_scoped = cur_funccal().func_rc().uf_scoped.get();
        if UF_STATE.with(|s| s.current_funccal.get()) == next_scoped {
            break;
        }
        UF_STATE.with(|s| s.current_funccal.set(next_scoped));
    }
    UF_STATE.with(|s| s.current_funccal.set(old_current_funccal));

    v
}

pub fn set_ref_in_previous_funccal(copy_id: i32) -> bool {
    let items = PREVIOUS_FUNCCAL.with(|c| c.borrow().clone());
    for fc_id in items {
        let Some(fc) = fc_id.rc() else {
            continue;
        };
        fc.fc_copy_id.set(copy_id + 1);
        if set_ref_in_ht(Some(&fc.fc_l_vars.get().dv_hashtab), copy_id + 1, None)
            || set_ref_in_ht(Some(&fc.fc_l_avars.get().dv_hashtab), copy_id + 1, None)
            || set_ref_in_list_items(fc.fc_l_varlist.get().opt(), copy_id + 1, None)
        {
            return true;
        }
    }
    false
}

/// Set `copy_id` in the arguments of the deferred functions of `fc`.
/// Returns `true` if setting references failed somehow.
fn set_ref_in_defer(fc: &Funccall, copy_id: i32) -> bool {
    for dr in &mut *fc.fc_defer.borrow_mut() {
        let argcount = dr.dr_argvars.len().saturating_sub(1);
        for tv in dr.dr_argvars.iter_mut().take(argcount) {
            if set_ref_in_item(tv, copy_id, None, None) {
                return true;
            }
        }
    }
    false
}

fn set_ref_in_funccal(fc: &Funccall, copy_id: i32) -> bool {
    if fc.fc_copy_id.get() != copy_id {
        fc.fc_copy_id.set(copy_id);
        if set_ref_in_ht(Some(&fc.fc_l_vars.get().dv_hashtab), copy_id, None)
            || set_ref_in_ht(Some(&fc.fc_l_avars.get().dv_hashtab), copy_id, None)
            || set_ref_in_list_items(fc.fc_l_varlist.get().opt(), copy_id, None)
            || set_ref_in_defer(fc, copy_id)
            || set_ref_in_func(None, fc.fc_func.get().rc().as_deref(), copy_id)
        {
            return true;
        }
    }
    false
}

pub fn set_ref_in_call_stack(copy_id: i32) -> bool {
    for fc in FunccalCallerIter(UF_STATE.with(|s| s.current_funccal.get())) {
        if set_ref_in_funccal(&fc, copy_id) {
            return true;
        }
    }

    for top in FUNCCAL_STACK.with(|c| c.borrow().iter().rev().copied().collect::<Vec<_>>()) {
        for fc2 in FunccalCallerIter(top) {
            if set_ref_in_funccal(&fc2, copy_id) {
                return true;
            }
        }
    }

    false
}

pub fn set_ref_in_functions(copy_id: i32) -> bool {
    for fp_id in func_tbl_values() {
        if g_st().got_int.get() {
            break;
        }
        if let Some(fp) = fp_id.rc()
            && !func_name_refcount(uf2hikey(&fp))
            && set_ref_in_func(None, Some(&fp), copy_id)
        {
            return true;
        }
    }
    false
}

pub fn set_ref_in_func_args(copy_id: i32) -> bool {
    let mut args = FUNCARGS.with(|c| c.borrow().clone());
    for tv in &mut args {
        if set_ref_in_item(tv, copy_id, None, None) {
            return true;
        }
    }
    false
}

pub fn set_ref_in_func(name: Option<&CStr>, fp_in: Option<&UserFunc>, copy_id: i32) -> bool {
    let fp_found: Option<std::rc::Rc<UserFunc>>;
    let mut fp = fp_in;
    let mut error = FCERR_NONE;
    let mut fname_buf = [0u8; FLEN_FIXED + 1];
    let mut tofree: Option<std::ffi::CString> = None;
    let mut abort = false;
    if name.is_none() && fp.is_none() {
        return false;
    }

    if fp.is_none()
        && let Some(name) = name
    {
        let fname = fname_trans_sid(name, &mut fname_buf, &mut tofree, &mut error);
        fp_found = find_func(Some(fname));
        fp = fp_found.as_deref();
    }
    if let Some(fp_ref) = fp {
        for fc in FunccalScopedIter(fp_ref.uf_scoped.get()) {
            abort = abort || set_ref_in_funccal(&fc, copy_id);
        }
    }
    drop(tofree);
    abort
}

/// Registers a luaref as a lambda.
pub fn register_luafunc(ref_: LuaRef) -> std::ffi::CString {
    let name = get_lambda_name();
    let fp = alloc_ufunc(name.as_bytes());

    fp.uf_refcount.set(1);
    fp.uf_varargs.set(1);
    fp.uf_flags.set(FC_LUAREF);
    fp.uf_calls.set(0);
    fp.uf_script_ctx.set(g_st().current_sctx.get());
    fp.uf_luaref.set(ref_);

    func_hashtab_add(&fp);

    uf2hikey(&fp).to_owned()
}

#[inline]
pub fn cur_funccal() -> std::rc::Rc<Funccall> {
    UF_STATE.with(|s| s.current_funccal.get()).unwrap_rc()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_function_and_func_name_helpers_work() {
        assert!(eval_fname_sid(b"s:Foo"));
        assert!(eval_fname_sid(b"<SID>Foo"));
        assert!(eval_fname_sid(b"<sid>Foo"));
        assert!(!eval_fname_sid(b"<SNR>12_Foo"));
        assert!(func_name_refcount(c"123"));
        assert!(func_name_refcount(c"<lambda>1"));
        assert!(!func_name_refcount(c"MyFunc"));
        assert!(builtin_function(b"abs"));
        assert!(!builtin_function(b"s:abs"));
        assert!(!builtin_function(b"foo#bar"));
        assert!(!builtin_function(b"Foo"));
    }
}
