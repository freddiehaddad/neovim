#![deny(unsafe_code)]

//! Expression evaluation (`src/nvim/eval.c`).

use crate::api::private::helpers::{HLF_E, cstr_as_string, cstr_to_string, tv_dict_is_watched};
use crate::api::private::validate::Array;
use crate::charset::{
    NUL, VARNUMBER_MIN, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_islower, ascii_isupper,
    ascii_iswhite, ascii_iswhite_or_nul, hex2nr, skipdigits_len,
};
use crate::eval::typval::{
    Callback, Dictitem, EvalFuncData, K_CALLBACK_EXPR, K_CALLBACK_FUNCREF, K_CALLBACK_LUA,
    K_CALLBACK_NONE_CONST, K_CALLBACK_PARTIAL, Listitem, Listwatch, TV_CSTRING, TvBlob, TvDict,
    TvFloat, TvList, TvPartial, TypVal, TypvalVval, VAR_UNLOCKED, VarType, Varnumber,
    callback_copy, callback_free, callback_put, tv_blob_alloc, tv_blob_alloc_ret,
    tv_blob_check_index, tv_blob_check_range, tv_blob_copy, tv_blob_equal, tv_blob_len,
    tv_blob_set_append, tv_blob_set_range, tv_blob_slice_or_index, tv_check_lock, tv_check_num,
    tv_check_str, tv_clear, tv_copy, tv_dict_add, tv_dict_add_nr, tv_dict_alloc, tv_dict_copy,
    tv_dict_equal, tv_dict_find, tv_dict_find_opt, tv_dict_free, tv_dict_free_contents,
    tv_dict_get_callback, tv_dict_get_number, tv_dict_item_alloc, tv_dict_item_free, tv_dict_unref,
    tv_dict_watcher_notify, tv_dict_wrong_func_name, tv_equal, tv_get_float, tv_get_number,
    tv_get_number_chk, tv_get_string, tv_get_string_buf, tv_get_string_buf_chk, tv_get_string_chk,
    tv_initial_value, tv_is_func, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_owned_tv, tv_list_append_string, tv_list_assign_range,
    tv_list_check_range_index_one, tv_list_check_range_index_two, tv_list_concat, tv_list_copy,
    tv_list_equal, tv_list_find, tv_list_find_nr, tv_list_free, tv_list_free_contents,
    tv_list_join, tv_list_last, tv_list_len, tv_list_ref, tv_list_set_lock, tv_list_set_ret,
    tv_list_slice_or_index, tv_list_unref, tv_list_watch_add, tv_list_watch_remove, tv2bool,
    value_check_lock,
};
use crate::eval::vars::{
    Evalarg, Lval, VV_ARGV, VV_EVENT, VV_LUA, VV_SHELL_ERROR, check_vars, eval_one_expr_in_str,
    eval_variable, evalvars_clear, evalvars_init, ex_let_vars, find_var, garbage_collect_globvars,
    garbage_collect_scriptvars, garbage_collect_vimvars, get_vim_var_dict, get_vim_var_partial,
    get_vimvar_dict, opt_to_tv, set_var, set_var_const, set_vim_var_list, set_vim_var_nr,
    skip_var_list, valid_varname, var_check_lock, var_check_ro, var_wrong_func_name,
};
use crate::event::r#loop::loop_queues;
use crate::ex_docmd::{
    Exarg, check_nextcmd, cmd_has_expr_args, do_cmdline, ends_excmd, set_ref_in_findfunc,
};
use crate::strings::{concat_str, format_to_buf, gettext, vim_strchr};
pub mod buffer;
pub mod decode;
pub mod deprecated;
pub mod encode;
pub mod executor;
pub mod fs;
pub mod funcs;
pub mod gc;
pub mod list;
pub mod typval;
pub mod userfunc;
pub mod vars;
pub mod window;

use crate::buffer::{FileBuf, K_UIMESSAGES, VAR_FIXED, bt_prompt, buflist_findnr};
use crate::channel::{CallbackReader, Channel, K_CHANNEL_STREAM_PROC};
use crate::errors::*;
use crate::eval::funcs::{E_STRING_LIST_OR_BLOB_REQUIRED, K_OPT_CPOPTIONS};
use crate::eval::userfunc::{
    Funcexe, call_func, call_simple_func, call_simple_luafunc, deref_func_name, eval_fname_script,
    find_func, free_all_functions, free_unref_funccal, func_ref, get_current_funccal, get_func_tv,
    get_funccal_args_ht, get_lambda_tv, get_scriptlocal_funcname, make_partial, restore_funccal,
    save_funccal, set_ref_in_call_stack, set_ref_in_func, set_ref_in_func_args,
    set_ref_in_functions, set_ref_in_previous_funccal,
};
use crate::ex_cmds::check_secure;
use crate::globals::*;
use crate::lua::executor::{
    K_RET_OBJECT, nlua_call_ref, nlua_is_deferred_safe, nlua_is_table_from_lua,
    nlua_register_table_as_callable,
};
use crate::message::{
    emsg, emsg_multiline, iemsg, internal_error, msg, msg_clr_eos, msg_end, msg_ext_no_fast,
    msg_ext_set_append, msg_ext_set_kind, msg_multiline, msg_outnum, msg_puts, msg_puts_hl,
    msg_puts_len, msg_sb_eol, msg_start, semsg, semsg_multiline, smsg, verb_msg, verbose_enter,
    verbose_enter_scroll, verbose_leave, verbose_leave_scroll,
};
use crate::r#move::{update_topline, validate_botline_win};
use crate::option::{
    K_OPT_FOLDEXPR, K_OPT_FOLDTEXT, K_OPT_INVALID, OPT_GLOBAL, OPT_LOCAL, find_option_end,
    get_option_value, get_tty_option, is_option_hidden, is_tty_option, optval_free,
    set_option_value_give_err, was_set_insecurely,
};
use crate::option_vars::*;
use crate::quickfix::set_ref_in_quickfix;
use crate::regexp::{
    NUMBUFLEN, RE_MAGIC, RE_STRING, REGSUB_COPY, REGSUB_MAGIC, Regmatch, vim_regcomp,
    vim_regexec_nl, vim_regsub,
};
use crate::register::{K_GREG_EXPR_SRC, get_reg_contents_str};
use crate::runtime::{
    Sctx, free_autoload_scriptnames, free_scriptnames, get_scriptname, script_autoload,
    script_is_lua,
};
use crate::ui::ui_has;
use std::ffi::CStr;
use std::ptr;

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::validate::{NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, PROF_YES};
use crate::cmdexpand::{
    Cmdidx, EXPAND_COMMANDS, EXPAND_ENV_VARS, EXPAND_EXPRESSION, EXPAND_FUNCTIONS, EXPAND_NOTHING,
    EXPAND_SETTINGS, EXPAND_USER_VARS,
};
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::eval::encode::{encode_list_write, encode_tv2echo, encode_tv2string};
use crate::eval::executor::eexe_mod_op;
use crate::eval::gc::{
    gc_dict_at, gc_dicts_len, gc_free_dict_at, gc_free_list_at, gc_list_at, gc_lists_len,
};
use crate::event::multiqueue::{multiqueue_free, multiqueue_new_child};
use crate::event::time::{
    TimeWatcher, TimeWatcherData, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop,
};
use crate::ex_eval::{aborting, discard_current_exception};
use crate::hashtab::Hashtab;
use crate::highlight_group::syn_name2id;
use crate::insexpand::{set_ref_in_cpt_callbacks, set_ref_in_insexpand_funcs};
use crate::keycodes::{
    FSK_IN_STRING, FSK_KEYCODE, FSK_SIMPLIFY, K_SPECIAL, KE_SNR, KS_EXTRA, find_special_key,
    trans_special,
};
use crate::mark::mark_get;
use crate::mbyte::{
    Vimconv, mb_charlen, mb_copy_char, mb_strcmp_ic, string_convert, utf_char2bytes, utfc_ptr2len,
};
use crate::memline::ml_get_buf_len;
use crate::memory::bytes_to_cstring;
use crate::msgpack_rpc::channel::find_channel;
use crate::ops::set_ref_in_opfunc;
use crate::optionstr::OptIndex;
use crate::os::env::{ExpandCtx, env_name_len, expand_env_save, vim_getenv};
use crate::os::fs::os_can_exe;
use crate::os::lang::free_locales;
use crate::os::shell::{os_system, shell_argv_to_str, shell_build_argv};
use crate::profile::{Proftime, prof_child_enter, prof_child_exit};
use crate::state::SaveVEvent;
use crate::tag::set_ref_in_tagfunc;
use crate::window::WinInfoInfo;

use crate::change::TAB;
use crate::globals::IOSIZE;
use crate::viml::parser::parser::CONV_NONE;
pub type VarFlavour = i32;
pub type Exprtype = i32;
pub type GlvStatus = i32;

pub const VAR_FLAVOUR_DEFAULT: VarFlavour = 1;
pub const VAR_FLAVOUR_SESSION: VarFlavour = 2;
pub const VAR_FLAVOUR_SHADA: VarFlavour = 4;

pub const EXPR_UNKNOWN: Exprtype = 0;
pub const EXPR_EQUAL: Exprtype = 1;
pub const EXPR_NEQUAL: Exprtype = 2;
pub const EXPR_GREATER: Exprtype = 3;
pub const EXPR_GEQUAL: Exprtype = 4;
pub const EXPR_SMALLER: Exprtype = 5;
pub const EXPR_SEQUAL: Exprtype = 6;
pub const EXPR_MATCH: Exprtype = 7;
pub const EXPR_NOMATCH: Exprtype = 8;
pub const EXPR_IS: Exprtype = 9;
pub const EXPR_ISNOT: Exprtype = 10;

pub const EVAL_EVALUATE: i32 = 1;
pub const COPYID_INC: i32 = 2;
pub const COPYID_MASK: i32 = !1;

const GLV_FAIL: GlvStatus = 0;
const GLV_OK: GlvStatus = 1;
const GLV_STOP: GlvStatus = 2;

pub(crate) const TFN_QUIET: i32 = 2;
pub(crate) const TFN_NO_AUTOLOAD: i32 = 4;
pub(crate) const TFN_READ_ONLY: i32 = 16;

pub(crate) const GLV_QUIET: i32 = TFN_QUIET;
pub(crate) const GLV_NO_AUTOLOAD: i32 = TFN_NO_AUTOLOAD;
pub(crate) const GLV_READ_ONLY: i32 = TFN_READ_ONLY;

pub(crate) const FNE_INCL_BR: i32 = 1;
pub(crate) const FNE_CHECK_START: i32 = 2;

const BS: u8 = 0x08;
const FF: u8 = 0x0c;
pub(crate) const ESC: u8 = 0x1b;

pub(crate) const DICT_MAXNEST: i32 = 100;

pub(crate) const VARNUMBER_MAX: Varnumber = i64::MAX;

pub(crate) const VAR_DEF_SCOPE: i32 = 2;

pub(crate) const K_BOOL_VAR_FALSE_CONST: Varnumber = 0;
pub(crate) const K_BOOL_VAR_TRUE_CONST: Varnumber = 1;

pub(crate) const K_LIST_LEN_UNKNOWN_CONST: isize = -1;
pub(crate) const K_LIST_LEN_SHOULD_KNOW: isize = -2;
pub(crate) const K_LIST_LEN_MAY_KNOW_CONST: isize = -3;

pub(crate) const K_MARK_ALL: i32 = 1;

const K_WIN_OPT_FOLDEXPR: usize = 15;

pub(crate) const STR2NR_ALL: i32 = 0x7f;

pub(crate) const AUTOLOAD_CHAR: u8 = b'#';

#[cfg(target_env = "msvc")]
const MAX_EVAL7_RECURSE: i32 = 300;
#[cfg(not(target_env = "msvc"))]
const MAX_EVAL7_RECURSE: i32 = 1000;

const E_MISSBRAC: &CStr = c"E111: Missing ']'";
const E_LIST_END: &CStr = c"E697: Missing end of List ']': %s";
const E_CANNOT_SLICE_DICTIONARY: &CStr = c"E719: Cannot slice a Dictionary";
const E_CANNOT_INDEX_SPECIAL_VARIABLE: &CStr = c"E909: Cannot index a special variable";
const E_NOWHITESPACE: &CStr = c"E274: No white space allowed before parenthesis";
const E_CANNOT_INDEX_A_FUNCREF: &CStr = c"E695: Cannot index a Funcref";
const E_VARIABLE_NESTED_TOO_DEEP_FOR_MAKING_COPY: &CStr =
    c"E698: Variable nested too deep for making a copy";
const E_EXPRESSION_TOO_RECURSIVE_STR: &CStr = c"E1169: Expression too recursive: %s";
const E_DOT_CAN_ONLY_BE_USED_ON_DICTIONARY_STR: &CStr =
    c"E1203: Dot can only be used on a dictionary: %s";
const E_EMPTY_FUNCTION_NAME: &CStr = c"E1192: Empty function name";
const E_CANNOT_USE_PARTIAL_HERE: &CStr = c"E1265: Cannot use a partial here";
const NAMESPACE_CHAR: &CStr = c"abglstvw";

pub(crate) struct EvalModuleState {
    pub(crate) eval_lavars_used: std::cell::Cell<Option<bool>>,
    echo_hl_id: std::cell::Cell<i32>,
    last_timer_id: std::cell::Cell<u64>,
    callback_depth: std::cell::Cell<i32>,
    current_copy_id: std::cell::Cell<i32>,
    eval7_recurse: std::cell::Cell<i32>,
    var_item_copy_recurse: std::cell::Cell<i32>,
}

impl EvalModuleState {
    const fn new() -> Self {
        Self {
            eval_lavars_used: std::cell::Cell::new(None),
            echo_hl_id: std::cell::Cell::new(0),
            last_timer_id: std::cell::Cell::new(1),
            callback_depth: std::cell::Cell::new(0),
            current_copy_id: std::cell::Cell::new(0),
            eval7_recurse: std::cell::Cell::new(0),
            var_item_copy_recurse: std::cell::Cell::new(0),
        }
    }
}

#[inline]
pub(crate) fn eval_lavars_used() -> Option<bool> {
    EVAL_STATE.with(|s| s.eval_lavars_used.get())
}

#[inline]
pub(crate) fn set_eval_lavars_used(val: Option<bool>) {
    EVAL_STATE.with(|s| s.eval_lavars_used.set(val));
}

thread_local! {
    static EVAL_STATE: EvalModuleState = const { EvalModuleState::new() };
    static EVAL_TIMERS: std::cell::RefCell<std::collections::BTreeMap<u64, std::rc::Rc<VimTimer>>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

#[inline]
fn with_timers<R>(
    f: impl FnOnce(&std::collections::BTreeMap<u64, std::rc::Rc<VimTimer>>) -> R,
) -> R {
    EVAL_TIMERS.with(|t| f(&t.borrow()))
}

#[inline]
fn with_timers_mut<R>(
    f: impl FnOnce(&mut std::collections::BTreeMap<u64, std::rc::Rc<VimTimer>>) -> R,
) -> R {
    EVAL_TIMERS.with(|t| f(&mut t.borrow_mut()))
}

enum ForSource {
    None,
    List {
        list: crate::eval::typval::ListId,
        lw: std::rc::Rc<Listwatch>,
    },
    Blob {
        bytes: Vec<u8>,
        idx: usize,
    },
    Str {
        text: std::ffi::CString,
        byte_idx: usize,
    },
}

/// Info used by a `:for` loop.
pub struct Forinfo {
    fi_semicolon: i32,
    fi_varcount: i32,
    fi_source: ForSource,
}

pub type HtStack = Vec<crate::eval::typval::DictId>;
pub type ListStack = Vec<crate::eval::typval::ListId>;

pub struct VimTimer {
    pub watcher: std::cell::RefCell<TimeWatcher>,
    pub timer_id: i32,
    pub repeat_count: std::cell::Cell<i32>,
    pub active_calls: std::cell::Cell<i32>,
    pub emsg_count: std::cell::Cell<i32>,
    pub timeout: i64,
    pub stopped: std::cell::Cell<bool>,
    pub paused: std::cell::Cell<bool>,
    pub callback: Callback,
}

impl Drop for VimTimer {
    fn drop(&mut self) {
        if crate::globals::g_st().exiting.get() {
            return;
        }
        if let Some(events) = self.watcher.get_mut().events.opt() {
            multiqueue_free(events);
        }
        callback_free(&self.callback);
    }
}

#[derive(Clone)]
pub struct CallerScope {
    pub script_ctx: Sctx,
    pub es_entry: crate::profile::Estack,
    pub autocmd_fname: Option<std::ffi::CString>,
    pub autocmd_match: Option<std::ffi::CString>,
    pub autocmd_fname_full: bool,
    pub autocmd_bufnr: i32,
    pub funccalp: crate::eval::userfunc::FunccalId,
}

#[inline]
fn tv_partial(tv: &TypVal) -> Option<&TvPartial> {
    tv.vval.v_partial_opt()
}

#[inline]
fn tv_set_partial(tv: &mut TypVal, pt: Option<std::rc::Rc<TvPartial>>) {
    tv.vval = crate::eval::typval::TypvalVval::Partial(crate::eval::typval::PartialId::from(pt));
}

#[inline]
pub fn tv_blob_set_ret(tv: &mut TypVal, b: Option<std::rc::Rc<TvBlob>>) {
    tv.v_type = VarType::VarBlob;
    if let Some(ref b_ref) = b {
        b_ref.bv_refcount.set(b_ref.bv_refcount.get() + 1);
    }
    tv.vval = crate::eval::typval::TypvalVval::Blob(crate::eval::typval::BlobId::from(b));
}

#[inline]
pub fn tv_dict_set_ret(tv: &mut TypVal, d: Option<&TvDict>) {
    tv.v_type = VarType::VarDict;
    tv.vval = crate::eval::typval::TypvalVval::Dict(crate::eval::typval::DictId::from(d));
    if let Some(d_ref) = tv.vval.v_dict_opt() {
        d_ref.dv_refcount.set(d_ref.dv_refcount.get() + 1);
    }
}

#[inline]
fn tv_list_copyid(l: &TvList) -> i32 {
    l.lv_copy_id.get()
}

#[inline]
fn tv_list_latest_copy(l: &TvList) -> Option<crate::eval::typval::ListId> {
    let c = l.lv_copylist.get();
    if c.is_null() { None } else { Some(c) }
}

#[inline]
fn tv_list_has_watchers(l: &TvList) -> bool {
    !l.lv_watch.borrow().is_empty()
}

#[inline]
fn tv_list_item_next(li: &Listitem) -> Option<crate::eval::typval::ListitemId> {
    li.next_opt()
}

pub fn get_v_event(sve: &mut SaveVEvent) -> crate::eval::typval::DictId {
    let v_event = get_vim_var_dict(VV_EVENT);

    if v_event.dv_hashtab.ht_used.get() > 0 {
        sve.sve_did_save = true;
        sve.sve_hashtab = v_event.dv_hashtab.take();
    } else {
        sve.sve_did_save = false;
    }
    v_event
}

pub fn restore_v_event(v_event: &TvDict, sve: &mut SaveVEvent) {
    tv_dict_free_contents(v_event);
    if sve.sve_did_save {
        v_event
            .dv_hashtab
            .replace(std::mem::take(&mut sve.sve_hashtab));
    }
}

/// @return  "n1" divided by "n2", taking care of dividing by zero.
pub fn num_divide(n1: Varnumber, n2: Varnumber) -> Varnumber {
    if n2 == 0 {
        if n1 == 0 {
            VARNUMBER_MIN
        } else if n1 < 0 {
            -VARNUMBER_MAX
        } else {
            VARNUMBER_MAX
        }
    } else if n1 == VARNUMBER_MIN && n2 == -1 {
        VARNUMBER_MAX
    } else {
        n1 / n2
    }
}

/// @return  "n1" modulus "n2", taking care of dividing by zero.
pub fn num_modulus(n1: Varnumber, n2: Varnumber) -> Varnumber {
    if n2 == 0 { 0 } else { n1.wrapping_rem(n2) }
}

/// Initialize the global and v: variables.
pub fn eval_init() {
    evalvars_init();
}

pub fn eval_clear() {
    evalvars_clear();
    free_scriptnames();
    free_locales();
    free_autoload_scriptnames();
    garbage_collect(false);
    free_all_functions();
}

pub fn fill_evalarg_from_eap(evalarg: &mut Evalarg, skip: bool) {
    *evalarg = Evalarg {
        eval_flags: if skip { 0 } else { EVAL_EVALUATE },
    };
}

/// Top level evaluation function, returning a boolean.
/// Sets "error" to true if there was an error.
pub fn eval_to_bool(
    arg: &CStr,
    error: &mut bool,
    eap: &mut Exarg,
    skip: bool,
    use_simple_function: bool,
) -> bool {
    let mut tv = tv_initial_value();
    let mut retval = false;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, skip);

    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    let r = if use_simple_function {
        eval0_simple_funccal(arg, &mut tv, Some(&mut *eap), &mut evalarg)
    } else {
        eval0(arg, &mut tv, Some(&mut *eap), &mut evalarg)
    };
    if !r {
        *error = true;
    } else {
        *error = false;
        if !skip {
            retval = tv_get_number_chk(&tv, Some(&mut *error)) != 0;
            tv_clear(&mut tv);
        }
    }
    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }

    retval
}

/// Call eval1() and give an error message if not done at a lower level.
fn eval1_emsg(arg: &mut &CStr, rettv: &mut TypVal, eap: Option<&mut Exarg>) -> bool {
    let start = *arg;
    let did_emsg_before = g_st().did_emsg.get();
    let called_emsg_before = g_st().called_emsg.get();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.is_some() && eap.unwrap().skip);

    let ret = eval1(arg, &mut *rettv, Some(&mut evalarg));
    if !ret
        && !aborting()
        && g_st().did_emsg.get() == did_emsg_before
        && g_st().called_emsg.get() == called_emsg_before
    {
        semsg!(gettext(E_INVEXPR2), start);
    }
    ret
}

/// @return  whether a typval is a valid expression to pass to eval_expr_typval()
///          or eval_expr_to_bool(). An empty string returns false.
pub fn eval_expr_valid_arg(tv: &TypVal) -> bool {
    tv.v_type != VarType::VarUnknown
        && (tv.v_type != VarType::VarString || !tv.vval.v_string_is_empty())
}

/// Evaluate a partial.
fn eval_expr_partial(expr: &TypVal, argv: &mut [TypVal], argc: i32, rettv: &mut TypVal) -> bool {
    let partial_ptr = expr.vval.v_partial();
    let Some(partial) = expr.vval.v_partial_opt() else {
        return false;
    };

    let s = partial_name(Some(partial));
    if s.to_bytes().is_empty() {
        return false;
    }

    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: 0,
        fe_lastline: 0,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: partial_ptr,
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: None,
        fe_found_var: false,
    };
    if !call_func(s, -1, rettv, argc, argv, &mut funcexe) {
        return false;
    }

    true
}

/// Evaluate an expression which is a function.
fn eval_expr_func(expr: &TypVal, argv: &mut [TypVal], argc: i32, rettv: &mut TypVal) -> bool {
    let mut buf = [0u8; NUMBUFLEN];
    let s = if expr.v_type == VarType::VarFunc {
        expr.vval.v_string_cstr()
    } else {
        tv_get_string_buf_chk(expr, &mut buf)
    };
    let Some(s) = s.filter(|cs| !cs.is_empty()) else {
        return false;
    };

    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: 0,
        fe_lastline: 0,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: crate::eval::typval::PartialId::NULL,
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: None,
        fe_found_var: false,
    };
    if !call_func(s, -1, rettv, argc, argv, &mut funcexe) {
        return false;
    }

    true
}

/// Evaluate an expression, which is a string.
fn eval_expr_string(expr: &TypVal, rettv: &mut TypVal) -> bool {
    let mut buf = [0u8; NUMBUFLEN];
    let Some(s_cstr) = tv_get_string_buf_chk(expr, &mut buf) else {
        return false;
    };
    let mut s = crate::charset::skipwhite(s_cstr);
    if !eval1_emsg(&mut s, rettv, None) {
        return false;
    }

    let rem = crate::charset::skipwhite(s);
    if !rem.is_empty() {
        tv_clear(rettv);
        semsg!(gettext(E_INVEXPR2), rem);
        return false;
    }

    true
}

/// Evaluate an expression, which can be a function, partial or string.
pub fn eval_expr_typval(
    expr: &TypVal,
    want_func: bool,
    argv: &mut [TypVal],
    argc: i32,
    rettv: &mut TypVal,
) -> bool {
    if expr.v_type == VarType::VarPartial {
        return eval_expr_partial(expr, argv, argc, rettv);
    }
    if expr.v_type == VarType::VarFunc || want_func {
        return eval_expr_func(expr, argv, argc, rettv);
    }

    eval_expr_string(expr, rettv)
}

/// Like eval_to_bool() but using a TypVal instead of a string.
pub fn eval_expr_to_bool(expr: &TypVal, error: &mut bool) -> bool {
    let mut argv = [tv_initial_value()];
    let mut rettv = tv_initial_value();

    if !eval_expr_typval(expr, false, &mut argv, 0, &mut rettv) {
        *error = true;
        return false;
    }
    let res = tv_get_number_chk(&rettv, Some(&mut *error)) != 0;
    tv_clear(&mut rettv);
    res
}

/// Top level evaluation function, returning a string.
pub fn eval_to_string_skip(arg: &CStr, eap: &mut Exarg, skip: bool) -> Option<std::ffi::CString> {
    let mut tv = tv_initial_value();
    let retval: Option<std::ffi::CString>;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, skip);
    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    if !eval0(arg, &mut tv, Some(&mut *eap), &mut evalarg) || skip {
        retval = None;
    } else {
        retval = Some(tv_get_string(&tv).into_owned());
        tv_clear(&mut tv);
    }
    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }

    retval
}

/// Skip over an expression at "*pp".
pub fn skip_expr(pp: &mut &CStr, mut evalarg: Option<&mut Evalarg>) -> bool {
    let save_flags = if evalarg.is_none() {
        0
    } else {
        evalarg.as_deref_mut().unwrap().eval_flags
    };

    if evalarg.is_some() {
        evalarg.as_deref_mut().unwrap().eval_flags &= !EVAL_EVALUATE;
    }

    *pp = crate::charset::skipwhite(pp);

    let mut rettv = tv_initial_value();
    let res = eval1(pp, &mut rettv, None);

    if let Some(evalarg) = evalarg {
        evalarg.eval_flags = save_flags;
    }

    res
}

/// Convert "tv" to a string.
fn typval2string(tv: &mut TypVal, join_list: bool) -> Option<std::ffi::CString> {
    if join_list && tv.v_type == VarType::VarList {
        let mut out: Vec<u8> = Vec::with_capacity(80);
        if let Some(l) = tv.vval.v_list_opt() {
            let has_items = tv_list_len(Some(l)) > 0;
            tv_list_join(&mut out, Some(l), c"\n");
            if has_items {
                out.push(b'\n');
            }
        }
        return Some(bytes_to_cstring(&out));
    } else if tv.v_type == VarType::VarList || tv.v_type == VarType::VarDict {
        return Some(encode_tv2string(&mut *tv, None));
    }
    Some(tv_get_string(&*tv).into_owned())
}

/// Top level evaluation function, returning a string.
pub fn eval_to_string_eap(
    arg: &CStr,
    join_list: bool,
    eap: Option<&mut Exarg>,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    let mut tv = tv_initial_value();
    let retval: Option<std::ffi::CString>;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.is_some() && eap.unwrap().skip);
    let r = if use_simple_function {
        eval0_simple_funccal(arg, &mut tv, None, &mut evalarg)
    } else {
        eval0(arg, &mut tv, None, &mut evalarg)
    };
    if !r {
        retval = None;
    } else {
        retval = typval2string(&mut tv, join_list);
        tv_clear(&mut tv);
    }

    retval
}

pub fn eval_to_string(
    arg: &CStr,
    join_list: bool,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    eval_to_string_eap(arg, join_list, None, use_simple_function)
}

/// Call eval_to_string() without using current local variables and using textlock.
pub fn eval_to_string_safe(
    arg: &CStr,
    use_sandbox: bool,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    save_funccal();
    {
        if use_sandbox {
            g_st().sandbox.set(g_st().sandbox.get() + (1));
        }
        g_st().textlock.set(g_st().textlock.get() + (1));
    }
    let retval = eval_to_string(arg, false, use_simple_function);
    {
        if use_sandbox {
            g_st().sandbox.set(g_st().sandbox.get() - (1));
        }
        g_st().textlock.set(g_st().textlock.get() - (1));
    }
    restore_funccal();
    retval
}

/// Top level evaluation function, returning a number.
pub fn eval_to_number(expr: &CStr, use_simple_function: bool) -> Varnumber {
    let mut rettv = tv_initial_value();
    let retval: Varnumber;
    let mut p = crate::charset::skipwhite(expr);
    let mut r: Option<bool> = None;

    {
        g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    }

    if use_simple_function {
        r = may_call_simple_func(expr, &mut rettv);
    }
    let ok = r.unwrap_or_else(|| eval1(&mut p, &mut rettv, Some(&mut Evalarg { eval_flags: 1 })));
    if !ok {
        retval = -1;
    } else {
        retval = tv_get_number_chk(&rettv, None);
        tv_clear(&mut rettv);
    }
    {
        g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    }

    retval
}

/// Top level evaluation function.
pub fn eval_expr(arg: &CStr, eap: Option<&mut Exarg>) -> Option<TypVal> {
    eval_expr_ext(arg, eap, false)
}

pub fn eval_expr_ext(
    arg: &CStr,
    mut eap: Option<&mut Exarg>,
    use_simple_function: bool,
) -> Option<TypVal> {
    let mut tv = tv_initial_value();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(
        &mut evalarg,
        eap.is_some() && eap.as_deref_mut().unwrap().skip,
    );

    let ok = if use_simple_function {
        eval0_simple_funccal(arg, &mut tv, eap, &mut evalarg)
    } else {
        eval0(arg, &mut tv, eap, &mut evalarg)
    };
    ok.then_some(tv)
}

/// Call some Vim script function and return the result in "*rettv".
pub fn call_vim_function(
    mut func: &CStr,
    argc: i32,
    argv: &mut [TypVal],
    rettv: &mut TypVal,
) -> bool {
    let mut len = func.to_bytes().len() as i32;
    let mut pt = crate::eval::typval::PartialId::NULL;

    if len >= 6 && func.to_bytes().starts_with(b"v:lua.") {
        func = crate::memory::CStrSubslice::advance(func, 6);
        len = check_luafunc_name(func, false);
        if len == 0 {
            tv_clear(&mut *rettv);
            return false;
        }
        pt = crate::eval::typval::PartialId::from(get_vim_var_partial(VV_LUA));
    }

    rettv.v_type = VarType::VarUnknown;
    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: pt,
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: None,
        fe_found_var: false,
    };
    let ret = call_func(func, len, &mut *rettv, argc, argv, &mut funcexe);

    if !ret {
        tv_clear(&mut *rettv);
    }

    ret
}

/// Call Vim script function and return the result as a string.
pub fn call_func_retstr(func: &CStr, argc: i32, argv: &mut [TypVal]) -> Option<std::ffi::CString> {
    let mut rettv = tv_initial_value();
    if !call_vim_function(func, argc, argv, &mut rettv) {
        return None;
    }

    let retval = tv_get_string(&rettv).into_owned();
    tv_clear(&mut rettv);
    Some(retval)
}

/// Call Vim script function and return the result as a List.
pub fn call_func_retlist(
    func: &CStr,
    argc: i32,
    argv: &mut [TypVal],
) -> Option<crate::eval::typval::ListId> {
    let mut rettv = tv_initial_value();

    if !call_vim_function(func, argc, argv, &mut rettv) {
        return None;
    }

    if rettv.v_type != VarType::VarList {
        tv_clear(&mut rettv);
        return None;
    }

    let l = rettv.vval.v_list();
    if l.is_null() { None } else { Some(l) }
}

/// Evaluate a number-valued "expr" option ('indentexpr'/'formatexpr') via its callback.
pub fn eval_expr_option_number(cb: &Callback, use_sandbox: bool) -> i32 {
    let copy = Callback::NONE;
    callback_copy(&copy, cb);
    if use_sandbox {
        g_st().sandbox.set(g_st().sandbox.get() + (1));
    }
    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    let mut tv = tv_initial_value();
    let mut argv = [tv_initial_value()];
    let mut retval = -1;
    if callback_call(&copy, 0, &mut argv, &mut tv) {
        retval = tv_get_number_chk(&tv, None) as i32;
        tv_clear(&mut tv);
    }
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    if use_sandbox {
        g_st().sandbox.set(g_st().sandbox.get() - (1));
    }
    callback_free(&copy);
    retval
}

/// Evaluate a "expr" option ('foldtext'/'includeexpr') via its callback, under |textlock| and an
/// isolated funccal stack; evaluates in the |sandbox| when `use_sandbox`.
pub fn eval_expr_option_tv(cb: &Callback, use_sandbox: bool, ret_tv: &mut TypVal) -> bool {
    save_funccal();
    {
        if use_sandbox {
            g_st().sandbox.set(g_st().sandbox.get() + (1));
        }
        g_st().textlock.set(g_st().textlock.get() + (1));
    }
    let mut argv = [tv_initial_value()];
    let ok = callback_call(cb, 0, &mut argv, &mut *ret_tv);
    {
        g_st().textlock.set(g_st().textlock.get() - (1));
        if use_sandbox {
            g_st().sandbox.set(g_st().sandbox.get() - (1));
        }
    }
    restore_funccal();
    ok
}

/// Evaluate 'foldexpr'. Returns the foldlevel, and any character preceding
/// it in "*cp". Doesn't give error messages.
pub fn eval_foldexpr(wp: &WinInfoInfo, cp: &mut i32) -> i32 {
    let saved_sctx = g_st().current_sctx.get();
    let use_sandbox = was_set_insecurely(Some(wp), K_OPT_FOLDEXPR, OPT_LOCAL);

    g_st()
        .current_sctx
        .set(wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_FOLDEXPR].get());

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    if use_sandbox {
        g_st().sandbox.set(g_st().sandbox.get() + (1));
    }
    g_st().textlock.set(g_st().textlock.get() + (1));
    *cp = NUL as i32;

    let mut tv = tv_initial_value();
    let mut argv = [tv_initial_value()];
    let retval: Varnumber;
    if !callback_call(&wp.w_onebuf_opt.wo_fde, 0, &mut argv, &mut tv) {
        retval = 0;
    } else {
        if tv.v_type == VarType::VarNumber {
            retval = tv.vval.v_number();
        } else if tv.v_type != VarType::VarString || tv.vval.v_string_is_null() {
            retval = 0;
        } else {
            let mut s = tv.vval.v_string_bytes();
            if let Some(&b0) = s.first()
                && !ascii_isdigit(b0)
                && b0 != b'-'
            {
                *cp = i32::from(b0);
                s = &s[1..];
            }
            retval = Varnumber::from(crate::charset::parse_i64_prefix(s));
        }
        tv_clear(&mut tv);
    }

    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    if use_sandbox {
        g_st().sandbox.set(g_st().sandbox.get() - (1));
    }
    g_st().textlock.set(g_st().textlock.get() - (1));
    g_st().current_sctx.set(saved_sctx);

    retval as i32
}

/// Evaluate 'foldtext', returning an Array or a String (NULL_STRING on failure).
pub fn eval_foldtext(wp: &WinInfoInfo) -> Object {
    let use_sandbox = was_set_insecurely(Some(wp), K_OPT_FOLDTEXT, OPT_LOCAL);
    let mut tv = tv_initial_value();
    let retval: Object;
    if !eval_expr_option_tv(&wp.w_onebuf_opt.wo_fdt, use_sandbox, &mut tv) {
        retval = Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(NvimString::new()),
        };
    } else {
        if tv.v_type == VarType::VarList {
            retval = vim_to_object(&tv);
        } else {
            retval = Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_to_string(Some(&tv_get_string(&tv)))),
            };
        }
        tv_clear(&mut tv);
    }
    retval
}

/// Find the end of a variable or function name. Unlike find_name_end() this
/// does not recognize magic braces.
fn to_name_end(arg: &CStr, use_namespace: bool) -> &CStr {
    let bytes = arg.to_bytes();
    let Some(&first) = bytes.first() else {
        return arg;
    };
    if !eval_isnamec1(i32::from(first)) {
        return arg;
    }

    let mut i = 1usize;
    while i < bytes.len() && eval_isnamec(i32::from(bytes[i])) {
        if bytes[i] == b':'
            && (i != 1
                || !use_namespace
                || !crate::strings::vim_haschr(c"bgstvw", i32::from(first)))
        {
            break;
        }
        i += crate::mbyte::utfc_slice2len(&bytes[i..]).max(1);
    }
    &arg[i.min(bytes.len())..]
}

fn get_lval_dict_item<'a>(
    lp: &mut Lval,
    name: &CStr,
    key_in: &'a CStr,
    len: i32,
    key_end: &mut &'a CStr,
    var1: &mut TypVal,
    flags: i32,
    unlet: bool,
    rettv: Option<&mut TypVal>,
) -> GlvStatus {
    let quiet = (flags & GLV_QUIET) != 0;
    let p_cstr = *key_end;
    let p0 = p_cstr.to_bytes().first().copied().unwrap_or(0);

    let key: &CStr = if len == -1 {
        &tv_get_string(&*var1)
    } else {
        key_in
    };
    lp.ll_list = crate::eval::typval::ListId::NULL;

    if lp.tv_mut().vval.v_dict().is_null() {
        let d = tv_dict_alloc();
        d.dv_refcount.set(d.dv_refcount.get() + 1);
        lp.tv_mut().vval = crate::eval::typval::TypvalVval::Dict(d);
    }
    let dict = lp.tv_mut().vval.v_dict();
    lp.ll_dict = dict;

    lp.ll_di = tv_dict_find(lp.ll_dict.opt(), key, len as isize).map_or(
        crate::eval::typval::DictitemId::NULL,
        crate::eval::typval::DictitemId::from,
    );

    if let Some(rettv) = rettv.as_deref()
        && lp.ll_dict.dv_scope.get() != 0
    {
        let key_cstr = (len != -1).then(|| bytes_to_cstring(&key.to_bytes()[..len as usize]));
        let key_ref = key_cstr.as_deref().unwrap_or(key);
        let wrong = (lp.ll_dict.dv_scope.get() == VAR_DEF_SCOPE
            && tv_is_func(rettv)
            && var_wrong_func_name(key_ref, lp.ll_di.is_null()))
            || !valid_varname(key_ref);
        if wrong {
            return GLV_FAIL;
        }
    }

    if !lp.ll_di.is_null() && tv_is_luafunc(&lp.ll_di.tv()) && len == -1 && rettv.is_none() {
        semsg!(E_ILLVAR, c"v:['lua']");
        return GLV_FAIL;
    }

    if lp.ll_di.is_null() {
        if lp.ll_dict == get_vimvar_dict()
            || get_funccal_args_ht()
                .is_some_and(|aht| ptr::eq(&raw const lp.ll_dict.dv_hashtab, aht))
        {
            semsg!(gettext(E_ILLVAR), name);
            return GLV_FAIL;
        }

        if p0 == b'[' || p0 == b'.' || unlet {
            if !quiet {
                semsg!(gettext(E_DICTKEY), key);
            }
            return GLV_FAIL;
        }
        if len == -1 {
            lp.ll_newkey = Some(key.to_owned());
        } else {
            let slice = &key.to_bytes()[..len as usize];
            lp.ll_newkey = Some(std::ffi::CString::new(slice).unwrap());
        }
        *key_end = p_cstr;
        return GLV_STOP;
    } else if (flags & GLV_READ_ONLY) == 0
        && (var_check_ro(
            i32::from(lp.ll_di.di_flags.get()),
            name,
            (p_cstr.as_ptr() as usize).wrapping_sub(name.as_ptr() as usize),
        ) || var_check_lock(
            i32::from(lp.ll_di.di_flags.get()),
            name,
            (p_cstr.as_ptr() as usize).wrapping_sub(name.as_ptr() as usize),
        ))
    {
        return GLV_FAIL;
    }

    lp.ll_tv = crate::eval::vars::LvalTv::DictItem(lp.ll_di.clone());

    GLV_OK
}

fn get_lval_blob(
    lp: &mut Lval,
    var1: &mut TypVal,
    var2: &mut TypVal,
    empty1: bool,
    quiet: bool,
) -> bool {
    let bloblen = tv_blob_len(lp.tv_mut().vval.v_blob_opt());

    if empty1 {
        lp.ll_n1 = 0;
    } else {
        lp.ll_n1 = tv_get_number(&*var1) as i32;
    }

    if !tv_blob_check_index(bloblen, Varnumber::from(lp.ll_n1), quiet) {
        return false;
    }
    if lp.ll_range && !lp.ll_empty2 {
        lp.ll_n2 = tv_get_number(&*var2) as i32;
        if !tv_blob_check_range(
            bloblen,
            Varnumber::from(lp.ll_n1),
            Varnumber::from(lp.ll_n2),
            quiet,
        ) {
            return false;
        }
    }

    let blob = lp.tv_mut().vval.v_blob();
    lp.ll_blob = blob;
    lp.ll_tv = crate::eval::vars::LvalTv::None;

    true
}

fn get_lval_list(
    lp: &mut Lval,
    var1: &mut TypVal,
    var2: &mut TypVal,
    empty1: bool,
    _flags: i32,
    quiet: bool,
) -> bool {
    if empty1 {
        lp.ll_n1 = 0;
    } else {
        lp.ll_n1 = tv_get_number(&*var1) as i32;
    }

    lp.ll_dict = crate::eval::typval::DictId::NULL;
    let ll_list = lp.tv_mut().vval.v_list();
    lp.ll_list = ll_list.clone();
    lp.ll_li = tv_list_check_range_index_one(ll_list.opt(), &mut lp.ll_n1, quiet).map_or(
        crate::eval::typval::ListitemId::NULL,
        crate::eval::typval::ListitemId::from,
    );
    if lp.ll_li.is_null() {
        return false;
    }

    if lp.ll_range && !lp.ll_empty2 {
        lp.ll_n2 = tv_get_number(&*var2) as i32;
        if !tv_list_check_range_index_two(
            ll_list.opt(),
            &mut lp.ll_n1,
            lp.ll_li.opt(),
            &mut lp.ll_n2,
            quiet,
        ) {
            return false;
        }
    }

    lp.ll_tv = crate::eval::vars::LvalTv::ListItem(lp.ll_li.clone());

    true
}

fn get_lval_subscript<'a>(
    lp: &mut Lval,
    mut p_cstr: &'a CStr,
    name: &CStr,
    mut rettv: Option<&mut TypVal>,
    unlet: bool,
    flags: i32,
) -> Option<&'a CStr> {
    let quiet = (flags & GLV_QUIET) != 0;
    let mut var1 = tv_initial_value();
    let mut var2 = tv_initial_value();
    let mut empty1 = false;

    loop {
        let p_bytes = p_cstr.to_bytes();
        let Some(&p0) = p_bytes.first() else {
            break;
        };
        if p0 != b'[' && !(p0 == b'.' && !matches!(p_bytes.get(1), Some(b'=' | b'.'))) {
            break;
        }
        if p0 == b'.' && lp.tv_mut().v_type != VarType::VarDict {
            if !quiet {
                semsg!(gettext(E_DOT_CAN_ONLY_BE_USED_ON_DICTIONARY_STR), name,);
            }
            return None;
        }
        if lp.tv_mut().v_type != VarType::VarList
            && lp.tv_mut().v_type != VarType::VarDict
            && lp.tv_mut().v_type != VarType::VarBlob
        {
            if !quiet {
                emsg(gettext(c"E689: Can only index a List, Dictionary or Blob"));
            }
            return None;
        }

        if lp.tv_mut().v_type == VarType::VarList && lp.tv_mut().vval.v_list().is_null() {
            tv_list_alloc_ret(&mut lp.tv_mut(), K_LIST_LEN_UNKNOWN_CONST);
        } else if lp.tv_mut().v_type == VarType::VarBlob && lp.tv_mut().vval.v_blob().is_null() {
            tv_blob_alloc_ret(&mut lp.tv_mut());
        }

        if lp.ll_range {
            if !quiet {
                emsg(gettext(c"E708: [:] must come last"));
            }
            tv_clear(&mut var1);
            tv_clear(&mut var2);
            return None;
        }

        let mut len: i32 = -1;
        let mut key_cstr: &'a CStr = c"";
        if p0 == b'.' {
            key_cstr = &p_cstr[1..];
            len = p_bytes[1..]
                .iter()
                .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
                .count() as i32;
            if len == 0 {
                if !quiet {
                    emsg(gettext(c"E713: Cannot use empty key after ."));
                }
                return None;
            }
            p_cstr = &key_cstr[len as usize..];
        } else {
            p_cstr = crate::charset::skipwhite_at(p_cstr, 1);
            if p_cstr.to_bytes().first().copied().unwrap_or(0) == b':' {
                empty1 = true;
            } else {
                empty1 = false;
                if !eval1(&mut p_cstr, &mut var1, Some(&mut Evalarg { eval_flags: 1 })) {
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return None;
                }
                if !tv_check_str(&var1) {
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return None;
                }
                p_cstr = crate::charset::skipwhite(p_cstr);
            }

            if p_cstr.to_bytes().first().copied().unwrap_or(0) == b':' {
                if lp.tv_mut().v_type == VarType::VarDict {
                    if !quiet {
                        emsg(gettext(E_CANNOT_SLICE_DICTIONARY));
                    }
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return None;
                }
                if rettv.is_some()
                    && !(rettv.as_deref_mut().unwrap().v_type == VarType::VarList
                        && !rettv.as_deref_mut().unwrap().vval.v_list().is_null())
                    && !(rettv.as_deref_mut().unwrap().v_type == VarType::VarBlob
                        && !rettv.as_deref_mut().unwrap().vval.v_blob().is_null())
                {
                    if !quiet {
                        emsg(gettext(c"E709: [:] requires a List or Blob value"));
                    }
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return None;
                }
                p_cstr = crate::charset::skipwhite_at(p_cstr, 1);
                if p_cstr.to_bytes().first().copied().unwrap_or(0) == b']' {
                    lp.ll_empty2 = true;
                } else {
                    lp.ll_empty2 = false;
                    if !eval1(&mut p_cstr, &mut var2, Some(&mut Evalarg { eval_flags: 1 }))
                        || !tv_check_str(&var2)
                    {
                        tv_clear(&mut var1);
                        tv_clear(&mut var2);
                        return None;
                    }
                }
                lp.ll_range = true;
            } else {
                lp.ll_range = false;
            }

            if p_cstr.to_bytes().first().copied().unwrap_or(0) != b']' {
                if !quiet {
                    emsg(gettext(E_MISSBRAC));
                }
                tv_clear(&mut var1);
                tv_clear(&mut var2);
                return None;
            }

            p_cstr = crate::memory::CStrSubslice::advance(p_cstr, 1);
        }

        if lp.tv_mut().v_type == VarType::VarDict {
            let glv_status = get_lval_dict_item(
                &mut *lp,
                name,
                key_cstr,
                len,
                &mut p_cstr,
                &mut var1,
                flags,
                unlet,
                (rettv.is_some()).then(|| rettv.as_deref_mut().unwrap()),
            );
            if glv_status == GLV_FAIL {
                tv_clear(&mut var1);
                tv_clear(&mut var2);
                return None;
            }
            if glv_status == GLV_STOP {
                break;
            }
        } else if lp.tv_mut().v_type == VarType::VarBlob {
            if !get_lval_blob(&mut *lp, &mut var1, &mut var2, empty1, quiet) {
                tv_clear(&mut var1);
                tv_clear(&mut var2);
                return None;
            }
            break;
        } else if !get_lval_list(&mut *lp, &mut var1, &mut var2, empty1, flags, quiet) {
            tv_clear(&mut var1);
            tv_clear(&mut var2);
            return None;
        }

        tv_clear(&mut var1);
        tv_clear(&mut var2);
        var1.v_type = VarType::VarUnknown;
        var2.v_type = VarType::VarUnknown;
    }

    tv_clear(&mut var1);
    tv_clear(&mut var2);
    Some(p_cstr)
}

/// Get an lvalue
pub fn get_lval<'a>(
    name_in: &'a CStr,
    rettv: Option<&mut TypVal>,
    lp: &mut Lval,
    unlet: bool,
    skip: bool,
    flags: i32,
    fne_flags: i32,
) -> Option<&'a CStr> {
    let name = name_in.as_ptr().cast::<u8>();
    let quiet = (flags & GLV_QUIET) != 0;

    *lp = Lval::new();

    if skip {
        lp.set_ll_name(name_in);
        return Some(find_name_end(name_in, None, FNE_INCL_BR | fne_flags));
    }

    let mut expr_range: Option<(usize, usize)> = None;
    let mut p_cstr = find_name_end(name_in, Some(&mut expr_range), fne_flags);
    if let Some((expr_start_off, expr_end_off)) = expr_range {
        let p0 = p_cstr.to_bytes().first().copied().unwrap_or(0);
        if unlet && !ascii_iswhite(p0) && ends_excmd(i32::from(p0)) == 0 && p0 != b'[' && p0 != b'.'
        {
            semsg!(gettext(E_TRAILING_ARG), p_cstr);
            return None;
        }

        let in_len = (p_cstr.as_ptr() as usize).wrapping_sub(name as usize);
        if let Some(cstr) =
            make_expanded_name(&name_in.to_bytes()[..in_len], expr_start_off, expr_end_off)
        {
            lp.ll_name_len = cstr.as_bytes().len();
            lp.set_ll_name(&cstr);
            lp.ll_exp_name = Some(cstr);
        } else {
            lp.ll_exp_name = None;
            lp.ll_name = None;
            lp.ll_name_off = 0;
            if !aborting() && !quiet {
                g_st().emsg_severe.set(true);
                semsg!(gettext(E_INVARG2), name_in);
                return None;
            }
            lp.ll_name_len = 0;
        }
    } else {
        lp.set_ll_name(name_in);
        lp.ll_name_len = (p_cstr.as_ptr() as usize).wrapping_sub(name as usize);
    }

    let p0 = p_cstr.to_bytes().first().copied().unwrap_or(0);
    if (p0 != b'[' && p0 != b'.') || lp.ll_name.is_none() {
        return Some(p_cstr);
    }

    let ll_name = lp.ll_name_cstr().unwrap_or(c"");

    let v = find_var(
        ll_name,
        lp.ll_name_len,
        (flags & GLV_READ_ONLY) == 0,
        flags & GLV_NO_AUTOLOAD,
    );
    if v.is_none() && !quiet {
        semsg!(
            gettext(c"E121: Undefined variable: %.*s"),
            lp.ll_name_len as i32,
            ll_name,
        );
    }
    let v = v?;

    lp.ll_tv = crate::eval::vars::LvalTv::DictItem(v);

    if tv_is_luafunc(&lp.ll_tv.borrow()) {
        return Some(p_cstr);
    }

    p_cstr = get_lval_subscript(
        &mut *lp,
        p_cstr,
        name_in,
        (rettv.is_some()).then(|| rettv.unwrap()),
        unlet,
        flags,
    )?;

    if lp.ll_exp_name.is_none() {
        lp.ll_name_len = (p_cstr.as_ptr() as usize).wrapping_sub(name as usize);
    }
    Some(p_cstr)
}

/// Set a variable that was parsed by get_lval() to "rettv".
pub fn set_var_lval(
    lp: &mut Lval,
    endp_cstr: &CStr,
    rettv: &mut TypVal,
    copy: bool,
    is_const: bool,
    op_opt: Option<&CStr>,
) {
    let ll_name_owned = lp.ll_name_cstr().map(CStr::to_owned);
    let ll_name = ll_name_owned.as_deref().unwrap_or(c"");

    if lp.ll_tv.is_null() {
        let name_cstr = if lp.ll_exp_name.is_some() {
            ll_name.to_owned()
        } else {
            let full_len = ll_name
                .to_bytes()
                .len()
                .saturating_sub(endp_cstr.to_bytes().len());
            bytes_to_cstring(&ll_name.to_bytes()[..full_len])
        };
        if let Some(blob) = lp.ll_blob.rc() {
            if let Some(op) = op_opt
                && !op.is_empty()
                && op.to_bytes().first() != Some(&b'=')
            {
                semsg!(gettext(E_LETWRONG), op);
                return;
            }
            if value_check_lock(blob.bv_lock.get(), Some(&name_cstr), TV_CSTRING) {
                return;
            }

            if lp.ll_range && rettv.v_type == VarType::VarBlob {
                if lp.ll_empty2 {
                    lp.ll_n2 = tv_blob_len(Some(&blob)) - 1;
                }

                let _ = tv_blob_set_range(
                    &blob,
                    Varnumber::from(lp.ll_n1),
                    Varnumber::from(lp.ll_n2),
                    rettv,
                );
            } else {
                let mut error = false;
                let val = tv_get_number_chk(&*rettv, Some(&mut error));
                if !error {
                    if !(0..=255).contains(&val) {
                        semsg!(gettext(E_INVALID_VALUE_FOR_BLOB_NR), val,);
                    } else {
                        tv_blob_set_append(&blob, lp.ll_n1, val as u8);
                    }
                }
            }
        } else if let Some(op) = op_opt
            && !op.is_empty()
            && op.to_bytes().first() != Some(&b'=')
        {
            let mut tv = tv_initial_value();

            if is_const {
                emsg(gettext(E_CANNOT_MOD));
                return;
            }

            let mut di: Option<crate::eval::typval::DictitemId> = None;
            if eval_variable(
                &name_cstr,
                lp.ll_name_len as i32,
                Some(&mut tv),
                Some(&mut di),
                true,
                false,
            ) {
                if di.is_none_or(|di| {
                    !var_check_ro(i32::from(di.di_flags.get()), &name_cstr, TV_CSTRING)
                        && !tv_check_lock(&di.tv(), Some(&name_cstr), TV_CSTRING)
                }) && eexe_mod_op(&mut tv, &*rettv, op)
                {
                    set_var(name_cstr.as_bytes(), &mut tv, false);
                }
                tv_clear(&mut tv);
            }
        } else {
            set_var_const(name_cstr.as_bytes(), rettv, copy, is_const);
        }
    } else if value_check_lock(
        if lp.ll_newkey.is_none() {
            lp.tv_mut().v_lock
        } else {
            lp.tv_mut().vval.v_dict().dv_lock.get()
        },
        lp.ll_name_cstr(),
        TV_CSTRING,
    ) {
        // Skip
    } else if lp.ll_range {
        if is_const {
            emsg(gettext(c"E996: Cannot lock a range"));
            return;
        }

        tv_list_assign_range(
            &lp.ll_list,
            rettv.vval.v_list().opt(),
            lp.ll_n1,
            lp.ll_n2,
            lp.ll_empty2,
            op_opt,
            ll_name,
        );
    } else {
        let mut oldtv = tv_initial_value();
        let dict = lp.ll_dict.clone();
        let watched = tv_dict_is_watched(dict.opt());

        if is_const {
            emsg(gettext(c"E996: Cannot lock a list or dict"));
            return;
        }

        let mut skip_assign = false;
        if let Some(ref newkey) = lp.ll_newkey {
            let newkey_cstr = newkey.as_c_str();
            if let Some(op) = op_opt
                && !op.is_empty()
                && op.to_bytes().first() != Some(&b'=')
            {
                semsg!(gettext(E_DICTKEY), newkey_cstr);
                return;
            }
            if tv_dict_wrong_func_name(&lp.tv_mut().vval.v_dict(), rettv, newkey_cstr) {
                return;
            }

            let di = tv_dict_item_alloc(newkey_cstr);
            if !tv_dict_add(&lp.tv_mut().vval.v_dict(), &di) {
                tv_dict_item_free(&di);
                return;
            }
            lp.ll_tv = crate::eval::vars::LvalTv::DictItem(di);
        } else {
            if watched {
                tv_copy(&lp.ll_tv.borrow(), &mut oldtv);
            }

            if let Some(op) = op_opt
                && !op.is_empty()
                && op.to_bytes().first() != Some(&b'=')
            {
                let mut cur_tv = lp.ll_tv.borrow().clone();
                eexe_mod_op(&mut cur_tv, &*rettv, op);
                *lp.ll_tv.borrow_mut() = cur_tv;
                skip_assign = true;
            } else {
                let mut prev = std::mem::replace(
                    &mut *lp.ll_tv.borrow_mut(),
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    },
                );
                tv_clear(&mut prev);
                *lp.ll_tv.borrow_mut() = prev;
            }
        }

        if !skip_assign {
            if copy {
                tv_copy(&*rettv, &mut lp.ll_tv.borrow_mut());
            } else {
                *lp.ll_tv.borrow_mut() = std::mem::take(rettv);
                lp.tv_mut().v_lock = VAR_UNLOCKED;
            }
        }

        if watched {
            let new_tv = lp.ll_tv.opt().map(|r| r.clone());
            if oldtv.v_type == VarType::VarUnknown {
                let newkey_cstr = lp.ll_newkey.as_ref().unwrap().as_c_str();
                tv_dict_watcher_notify(&dict, newkey_cstr, new_tv.as_ref(), None);
            } else {
                let di_ = &lp.ll_di;
                let di_key_cstr = di_.key_cstr();
                tv_dict_watcher_notify(&dict, di_key_cstr, new_tv.as_ref(), Some(&oldtv));
                tv_clear(&mut oldtv);
            }
        }
    }
}

/// Evaluate the expression used in a ":for var in expr" command.
pub fn eval_for_line(
    arg: &CStr,
    errp: &mut bool,
    eap: &mut Exarg,
    evalarg: &mut Evalarg,
) -> Box<Forinfo> {
    let mut fi = Box::new(Forinfo {
        fi_semicolon: 0,
        fi_varcount: 0,
        fi_source: ForSource::None,
    });
    let mut tv = tv_initial_value();
    let skip = (evalarg.eval_flags & EVAL_EVALUATE) == 0;

    *errp = true;

    let Some(expr_cstr) = skip_var_list(arg, &mut fi.fi_varcount, &mut fi.fi_semicolon, false)
    else {
        return fi;
    };

    let mut expr = crate::charset::skipwhite(expr_cstr);
    let expr_bytes = expr.to_bytes();
    if !expr_bytes.starts_with(b"in")
        || expr_bytes
            .get(2)
            .is_some_and(|&b| !ascii_iswhite(i32::from(b)))
    {
        emsg(gettext(c"E690: Missing \"in\" after :for"));
        return fi;
    }

    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    expr = crate::charset::skipwhite(&expr[2..]);
    if eval0(expr, &mut tv, Some(&mut *eap), &mut *evalarg) {
        *errp = false;
        if !skip {
            if tv.v_type == VarType::VarList {
                let Some(l) = tv.vval.v_list_opt() else {
                    tv_clear(&mut tv);
                    if skip {
                        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
                    }
                    return fi;
                };
                let lw = std::rc::Rc::new(Listwatch {
                    lw_item: crate::globals::CloneCell::new(l.lv_first.get()),
                });
                tv_list_watch_add(l, lw.clone());
                fi.fi_source = ForSource::List { list: l.into(), lw };
            } else if tv.v_type == VarType::VarBlob {
                let bytes = tv
                    .vval
                    .v_blob_opt()
                    .map_or_else(Vec::new, |b| b.bv_ga.borrow().clone());
                fi.fi_source = ForSource::Blob { bytes, idx: 0 };
                tv_clear(&mut tv);
            } else if tv.v_type == VarType::VarString {
                let text = tv.vval.v_string_owned();
                fi.fi_source = ForSource::Str { text, byte_idx: 0 };
                tv_clear(&mut tv);
            } else {
                emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED));
                tv_clear(&mut tv);
            }
        }
    }
    if skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }

    fi
}

/// Use the first item in a ":for" list. Advance to the next.
pub fn next_for_item(fi: &mut Forinfo, arg: &CStr) -> bool {
    match &mut fi.fi_source {
        ForSource::None => false,
        ForSource::Blob { bytes, idx } => {
            if *idx >= bytes.len() {
                return false;
            }
            let mut tv = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_FIXED,
                vval: TypvalVval::Number(Varnumber::from(bytes[*idx])),
            };
            *idx += 1;
            ex_let_vars(arg, &mut tv, 1, fi.fi_semicolon, fi.fi_varcount, 0, None)
        }
        ForSource::Str { text, byte_idx } => {
            let text_bytes = text.as_bytes();
            if *byte_idx >= text_bytes.len() {
                return false;
            }
            let len = crate::mbyte::utfc_slice2len(&text_bytes[*byte_idx..]);
            if len == 0 {
                return false;
            }
            let mut tv = TypVal::new();
            crate::eval::typval::tv_set_string_len(
                &mut tv,
                Some(&text_bytes[*byte_idx..*byte_idx + len]),
            );
            tv.v_lock = VAR_FIXED;
            *byte_idx += len;
            let result = ex_let_vars(arg, &mut tv, 1, fi.fi_semicolon, fi.fi_varcount, 0, None);
            tv_clear(&mut tv);
            result
        }
        ForSource::List { list: _, lw } => {
            let item = lw.lw_item.get();
            let Some(item_ref) = item.opt() else {
                return false;
            };
            lw.lw_item.set(item_ref.li_next.get());
            let mut item_tv = item_ref.tv().clone();
            ex_let_vars(
                arg,
                &mut item_tv,
                1,
                fi.fi_semicolon,
                fi.fi_varcount,
                0,
                None,
            )
        }
    }
}

impl Drop for Forinfo {
    fn drop(&mut self) {
        if let ForSource::List { ref list, ref lw } = self.fi_source
            && let Some(list_ref) = list.opt()
        {
            tv_list_watch_remove(list_ref, lw);
            tv_list_unref(Some(list_ref));
        }
    }
}

pub fn set_context_for_expression(xp: &mut ExpandCtx, arg_in: &CStr, cmdidx: Cmdidx) {
    let mut arg = crate::strings::OwnedSlice::from_cstr(arg_in);
    let mut got_eq = false;

    if cmdidx == Cmdidx::CmdLet || cmdidx == Cmdidx::CmdConst {
        xp.xp_context = EXPAND_USER_VARS;
        let arg_bytes = arg.as_bytes();
        if !arg_bytes
            .iter()
            .any(|b| b"\"'+-*/%.=!?~|&$([<>,#".contains(b))
        {
            let mut p_idx = arg_bytes.len();
            loop {
                xp.xp_pattern = arg.wrapping_add(p_idx);
                if p_idx == 0 {
                    break;
                }
                p_idx -= (crate::mbyte::utf_head_off_slice(arg_bytes, p_idx - 1) as usize + 1)
                    .min(p_idx);
                if ascii_iswhite(arg_bytes[p_idx]) {
                    break;
                }
            }
            return;
        }
    } else {
        xp.xp_context = if cmdidx == Cmdidx::CmdCall {
            EXPAND_FUNCTIONS
        } else {
            EXPAND_EXPRESSION
        };
    }
    while {
        xp.xp_pattern = arg
            .as_bytes()
            .iter()
            .position(|b| b"\"'+-*/%.=!?~|&$([<>,#".contains(b))
            .map_or(crate::strings::OwnedSlice::NULL, |i| arg.wrapping_add(i));
        !xp.xp_pattern.is_null()
    } {
        let pat_bytes = xp.xp_pattern.as_bytes();
        let mut c = i32::from(pat_bytes.first().copied().unwrap_or(0));
        if c == b'&' as i32 {
            c = i32::from(pat_bytes.get(1).copied().unwrap_or(0));
            if c == b'&' as i32 {
                xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                xp.xp_context = if cmdidx != Cmdidx::CmdLet || got_eq {
                    EXPAND_EXPRESSION
                } else {
                    EXPAND_NOTHING
                };
            } else if c != b' ' as i32 {
                xp.xp_context = EXPAND_SETTINGS;
                if (c == b'l' as i32 || c == b'g' as i32) && pat_bytes.get(2) == Some(&b':') {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(2);
                }
            }
        } else if c == b'$' as i32 {
            xp.xp_context = EXPAND_ENV_VARS;
        } else if c == b'=' as i32 {
            got_eq = true;
            xp.xp_context = EXPAND_EXPRESSION;
        } else if (c == b'#' as i32 && xp.xp_context == EXPAND_EXPRESSION)
            || ((c == b'<' as i32 || c == b'#' as i32)
                && xp.xp_context == EXPAND_FUNCTIONS
                && vim_strchr(xp.xp_pattern.as_cstr(), b'(' as i32).is_none())
        {
            break;
        } else if cmdidx != Cmdidx::CmdLet || got_eq {
            if c == b'"' as i32 {
                loop {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    let p_bytes = xp.xp_pattern.as_bytes();
                    c = i32::from(p_bytes.first().copied().unwrap_or(0));
                    if c == NUL as i32 || c == b'"' as i32 {
                        break;
                    }
                    if c == b'\\' as i32 && p_bytes.len() > 1 {
                        xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    }
                }
                xp.xp_context = EXPAND_NOTHING;
            } else if c == b'\'' as i32 {
                loop {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    c = i32::from(xp.xp_pattern.first_byte());
                    if c == NUL as i32 || c == b'\'' as i32 {
                        break;
                    }
                }
                xp.xp_context = EXPAND_NOTHING;
            } else if c == b'|' as i32 {
                if pat_bytes.get(1) == Some(&b'|') {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    xp.xp_context = EXPAND_EXPRESSION;
                } else {
                    xp.xp_context = EXPAND_COMMANDS;
                }
            } else {
                xp.xp_context = EXPAND_EXPRESSION;
            }
        } else {
            xp.xp_context = EXPAND_EXPRESSION;
        }
        arg = arg.wrapping_add(xp.xp_pattern.offset_from(&arg));
        if !arg.is_empty() {
            loop {
                arg = arg.wrapping_add(1);
                c = i32::from(arg.first_byte());
                if c == NUL as i32 || (c != b' ' as i32 && c != b'\t' as i32) {
                    break;
                }
            }
        }
    }

    if cmd_has_expr_args(cmdidx) && xp.xp_context == EXPAND_EXPRESSION {
        loop {
            let n = arg.skiptowhite();
            if n.addr() == arg.addr()
                || ascii_iswhite_or_nul(n.skipwhite().first_byte() as std::ffi::c_char)
            {
                break;
            }
            arg = n.skipwhite();
        }
    }

    xp.xp_pattern = arg;
}

/// Does not use 'cpo' and always uses 'magic'.
pub fn pattern_match(pat: &CStr, text: Option<&CStr>, ic: bool) -> bool {
    let mut matches = false;
    let mut regmatch: Regmatch = Regmatch::new();

    let save_cpo = opt_vars().p_cpo.take();
    regmatch.set_prog(vim_regcomp(pat, RE_MAGIC + RE_STRING));
    if !regmatch.regprog.is_null() {
        regmatch.rm_ic = ic;
        matches = vim_regexec_nl(&mut regmatch, text, 0);
        regmatch.free_prog();
    }
    opt_vars().p_cpo.set(save_cpo);
    matches
}

/// Handle a name followed by "(". Both for just "name(arg)" and for
/// "expr->name(arg)".
fn eval_func(
    arg: &mut &CStr,
    evalarg: Option<&mut Evalarg>,
    name: &CStr,
    name_len: i32,
    rettv: &mut TypVal,
    flags: i32,
    basetv: Option<&mut TypVal>,
) -> bool {
    let evaluate = (flags & EVAL_EVALUATE) != 0;
    let mut len = name_len;
    let mut found_var = false;

    if !evaluate {
        let name_bytes = name.to_bytes();
        check_vars(&name_bytes[..(len as usize).min(name_bytes.len())]);
    }

    let mut partial_opt: Option<std::rc::Rc<TvPartial>> = None;
    let deref = deref_func_name(
        name,
        &mut len,
        Some(&mut partial_opt),
        !evaluate,
        Some(&mut found_var),
    );
    let partial = crate::eval::typval::PartialId::from(partial_opt);
    let deref_bytes = deref.to_bytes();

    let s_cstr = bytes_to_cstring(&deref_bytes[..(len as usize).min(deref_bytes.len())]);

    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: evaluate,
        fe_partial: partial,
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: basetv.map(std::mem::take),
        fe_found_var: found_var,
    };
    let mut ret = get_func_tv(&s_cstr, len, &mut *rettv, arg, evalarg, &mut funcexe);

    if rettv.v_type == VarType::VarUnknown && !evaluate && arg.to_bytes().first() == Some(&b'(') {
        rettv.vval = crate::eval::typval::TypvalVval::from_cstr(c"");
        rettv.v_type = VarType::VarFunc;
    }

    if evaluate && aborting() {
        if ret {
            tv_clear(rettv);
        }
        ret = false;
    }
    ret
}

/// After using "evalarg" filled from "eap": free the memory.
/// Handle zero level expression.
pub fn eval0(
    arg: &CStr,
    rettv: &mut TypVal,
    mut eap: Option<&mut Exarg>,
    evalarg: &mut Evalarg,
) -> bool {
    let did_emsg_before = g_st().did_emsg.get();
    let called_emsg_before = g_st().called_emsg.get();
    let mut end_error = false;

    let mut p = crate::charset::skipwhite(arg);
    let ret = eval1(&mut p, rettv, Some(evalarg));

    if ret {
        end_error = ends_excmd(i32::from(p.to_bytes().first().copied().unwrap_or(0))) == 0;
    }
    if !ret || end_error {
        if ret {
            tv_clear(rettv);
        }
        if !aborting()
            && g_st().did_emsg.get() == did_emsg_before
            && g_st().called_emsg.get() == called_emsg_before
        {
            if end_error {
                semsg!(gettext(E_TRAILING_ARG), p);
            } else {
                semsg!(gettext(E_INVEXPR2), arg);
            }
        }

        if eap.is_some()
            && let Some(nextcmd) = check_nextcmd(p)
            && nextcmd.to_bytes().first() != Some(&b'|')
        {
            eap.as_deref_mut().unwrap().nextcmd = crate::strings::OwnedSlice::from_cstr(nextcmd);
        }
        return false;
    }

    if let Some(eap) = eap {
        eap.nextcmd = crate::strings::OwnedSlice::from_opt_cstr(check_nextcmd(p));
    }

    ret
}

/// If "arg" is a simple function call without arguments then call it and return
/// the result. Otherwise return NOTDONE.
pub fn may_call_simple_func(arg_cstr: &CStr, rettv: &mut TypVal) -> Option<bool> {
    let arg_bytes = arg_cstr.to_bytes();
    let parens_idx = arg_bytes.windows(2).position(|w| w == b"()");
    let mut r = None;

    if let Some(parens_off) = parens_idx
        && crate::charset::skipwhite_slice(&arg_bytes[parens_off + 2..]).is_empty()
    {
        let parens_ptr = arg_cstr[parens_off..].as_ptr();
        if arg_bytes.starts_with(b"v:lua.") {
            if parens_off != 6 && skip_luafunc_name(&arg_cstr[6..]).as_ptr() == parens_ptr {
                r = call_simple_luafunc(&arg_bytes[6..parens_off], rettv);
            }
        } else {
            let p_off = if arg_bytes.starts_with(b"<SNR>") {
                5 + skipdigits_len(&arg_bytes[5..])
            } else {
                0
            };
            if to_name_end(&arg_cstr[p_off..], true).as_ptr() == parens_ptr {
                r = call_simple_func(&arg_bytes[..parens_off], rettv);
            }
        }
    }
    r
}

fn eval0_simple_funccal(
    arg: &CStr,
    rettv: &mut TypVal,
    eap: Option<&mut Exarg>,
    evalarg: &mut Evalarg,
) -> bool {
    may_call_simple_func(arg, rettv).unwrap_or_else(|| eval0(arg, rettv, eap, evalarg))
}

/// Handle top level expression:
///      expr2 ? expr1 : expr1
///      expr2 ?? expr1
pub fn eval1(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    *rettv = TypVal::new();

    if !eval2(arg, rettv, evalarg.as_deref_mut()) {
        return false;
    }

    let mut p = *arg;
    let p_bytes = p.to_bytes();
    if p_bytes.first() == Some(&b'?') {
        let op_falsy = p_bytes.get(1) == Some(&b'?');
        let mut local_evalarg = Evalarg::new();
        let evalarg_used: &mut Evalarg = evalarg.unwrap_or(&mut local_evalarg);
        let orig_flags = evalarg_used.eval_flags;
        let evaluate = (evalarg_used.eval_flags & EVAL_EVALUATE) != 0;

        let mut result = false;
        if evaluate {
            let mut error = false;

            if op_falsy {
                result = tv2bool(&*rettv);
            } else if tv_get_number_chk(&*rettv, Some(&mut error)) != 0 {
                result = true;
            }
            if error || !op_falsy || !result {
                tv_clear(rettv);
            }
            if error {
                return false;
            }
        }

        let adv = if op_falsy { 2 } else { 1 };
        *arg = crate::charset::skipwhite_at(arg, adv);
        evalarg_used.eval_flags = if if op_falsy { !result } else { result } {
            orig_flags
        } else {
            orig_flags & !EVAL_EVALUATE
        };
        let mut var2 = tv_initial_value();
        if !eval1(arg, &mut var2, Some(evalarg_used)) {
            evalarg_used.eval_flags = orig_flags;
            return false;
        }
        if !op_falsy || !result {
            *rettv = std::mem::take(&mut var2);
        }

        if !op_falsy {
            p = *arg;
            if p.to_bytes().first().copied().unwrap_or(0) != b':' {
                emsg(gettext(c"E109: Missing ':' after '?'"));
                if evaluate && result {
                    tv_clear(rettv);
                }
                evalarg_used.eval_flags = orig_flags;
                return false;
            }

            *arg = crate::charset::skipwhite_at(arg, 1);
            evalarg_used.eval_flags = if !result {
                orig_flags
            } else {
                orig_flags & !EVAL_EVALUATE
            };
            if !eval1(arg, &mut var2, Some(evalarg_used)) {
                if evaluate && result {
                    tv_clear(rettv);
                }
                evalarg_used.eval_flags = orig_flags;
                return false;
            }
            if evaluate && !result {
                *rettv = var2;
            }
        }

        evalarg_used.eval_flags = orig_flags;
    }

    true
}

/// Handle first level expression: `expr2 || expr2 || expr2`
fn eval2(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    if !eval3(arg, rettv, evalarg.as_deref_mut()) {
        return false;
    }

    let mut p = *arg;
    if p.to_bytes().starts_with(b"||") {
        let mut local_evalarg = Evalarg::new();
        let evalarg_used: &mut Evalarg = evalarg.unwrap_or(&mut local_evalarg);
        let orig_flags = evalarg_used.eval_flags;
        let evaluate = (evalarg_used.eval_flags & EVAL_EVALUATE) != 0;

        let mut result = false;

        if evaluate {
            let mut error = false;
            if tv_get_number_chk(&*rettv, Some(&mut error)) != 0 {
                result = true;
            }
            tv_clear(rettv);
            if error {
                return false;
            }
        }

        while p.to_bytes().starts_with(b"||") {
            *arg = crate::charset::skipwhite_at(arg, 2);
            evalarg_used.eval_flags = if !result {
                orig_flags
            } else {
                orig_flags & !EVAL_EVALUATE
            };
            let mut var2 = tv_initial_value();
            if !eval3(arg, &mut var2, Some(evalarg_used)) {
                return false;
            }

            if evaluate && !result {
                let mut error = false;
                if tv_get_number_chk(&var2, Some(&mut error)) != 0 {
                    result = true;
                }
                tv_clear(&mut var2);
                if error {
                    return false;
                }
            }
            if evaluate {
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(result));
            }

            p = *arg;
        }

        evalarg_used.eval_flags = orig_flags;
    }

    true
}

/// Handle second level expression: `expr3 && expr3 && expr3`
fn eval3(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    if !eval4(arg, rettv, evalarg.as_deref_mut()) {
        return false;
    }

    let mut p = *arg;
    if p.to_bytes().starts_with(b"&&") {
        let mut local_evalarg = Evalarg::new();
        let evalarg_used: &mut Evalarg = evalarg.unwrap_or(&mut local_evalarg);
        let orig_flags = evalarg_used.eval_flags;
        let evaluate = (evalarg_used.eval_flags & EVAL_EVALUATE) != 0;

        let mut result = true;

        if evaluate {
            let mut error = false;
            if tv_get_number_chk(&*rettv, Some(&mut error)) == 0 {
                result = false;
            }
            tv_clear(rettv);
            if error {
                return false;
            }
        }

        while p.to_bytes().starts_with(b"&&") {
            *arg = crate::charset::skipwhite_at(arg, 2);
            evalarg_used.eval_flags = if result {
                orig_flags
            } else {
                orig_flags & !EVAL_EVALUATE
            };
            let mut var2 = tv_initial_value();
            if !eval4(arg, &mut var2, Some(evalarg_used)) {
                return false;
            }

            if evaluate && result {
                let mut error = false;
                if tv_get_number_chk(&var2, Some(&mut error)) == 0 {
                    result = false;
                }
                tv_clear(&mut var2);
                if error {
                    return false;
                }
            }
            if evaluate {
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(result));
            }

            p = *arg;
        }

        evalarg_used.eval_flags = orig_flags;
    }

    true
}

fn get_compare_type(p_bytes: &[u8], len: &mut usize) -> Exprtype {
    let mut expr_type: Exprtype = EXPR_UNKNOWN;
    match p_bytes.first().copied().unwrap_or(0) {
        b'=' => {
            if p_bytes.get(1) == Some(&b'=') {
                expr_type = EXPR_EQUAL;
            } else if p_bytes.get(1) == Some(&b'~') {
                expr_type = EXPR_MATCH;
            }
        }
        b'!' => {
            if p_bytes.get(1) == Some(&b'=') {
                expr_type = EXPR_NEQUAL;
            } else if p_bytes.get(1) == Some(&b'~') {
                expr_type = EXPR_NOMATCH;
            }
        }
        b'>' => {
            if p_bytes.get(1) != Some(&b'=') {
                expr_type = EXPR_GREATER;
                *len = 1;
            } else {
                expr_type = EXPR_GEQUAL;
            }
        }
        b'<' => {
            if p_bytes.get(1) != Some(&b'=') {
                expr_type = EXPR_SMALLER;
                *len = 1;
            } else {
                expr_type = EXPR_SEQUAL;
            }
        }
        b'i' if p_bytes.get(1) == Some(&b's') => {
            if p_bytes.starts_with(b"isnot") {
                *len = 5;
            }
            if !p_bytes
                .get(*len)
                .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
            {
                expr_type = if *len == 2 { EXPR_IS } else { EXPR_ISNOT };
            }
        }
        _ => {}
    }
    expr_type
}

/// Handle third level expression (comparisons).
fn eval4(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    if !eval5(arg, rettv, evalarg.as_deref_mut()) {
        return false;
    }

    let p = *arg;
    let p_bytes = p.to_bytes();
    let mut len: usize = 2;
    let expr_type = get_compare_type(p_bytes, &mut len);

    if expr_type != EXPR_UNKNOWN {
        let mut var2 = tv_initial_value();
        let ic: bool;
        if p_bytes.get(len) == Some(&b'?') {
            ic = true;
            len += 1;
        } else if p_bytes.get(len) == Some(&b'#') {
            ic = false;
            len += 1;
        } else {
            ic = opt_vars().p_ic.get();
        }

        *arg = crate::charset::skipwhite_at(p, len);

        if !eval5(arg, &mut var2, evalarg.as_deref_mut()) {
            tv_clear(rettv);
            return false;
        }
        if evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0)
        {
            let ret = typval_compare(rettv, &mut var2, expr_type, ic);
            tv_clear(&mut var2);
            return ret;
        }
    }

    true
}

/// Make a copy of blob "tv1" and append blob "tv2".
fn eval_addblob(tv1: &mut TypVal, tv2: &mut TypVal) {
    let b = tv_blob_alloc();
    let b1 = tv1.vval.v_blob();
    let b2 = tv2.vval.v_blob();

    let len1 = i64::from(tv_blob_len(b1.opt()));
    let len2 = i64::from(tv_blob_len(b2.opt()));
    let totallen = len1 + len2;

    if totallen >= 0 && totallen <= i64::from(i32::MAX) {
        let mut bv = b.bv_ga.borrow_mut();
        bv.reserve(totallen as usize);
        if let Some(b1) = b1.opt() {
            bv.extend_from_slice(&b1.bv_ga.borrow());
        }
        if let Some(b2) = b2.opt() {
            bv.extend_from_slice(&b2.bv_ga.borrow());
        }
    }

    tv_clear(tv1);
    tv_blob_set_ret(tv1, Some(b));
}

/// Make a copy of list "tv1" and append list "tv2".
fn eval_addlist(tv1: &mut TypVal, tv2: &mut TypVal) -> bool {
    let mut var3 = tv_initial_value();
    if !tv_list_concat(tv1.vval.v_list_opt(), tv2.vval.v_list_opt(), &mut var3) {
        tv_clear(tv1);
        tv_clear(tv2);
        return false;
    }
    tv_clear(tv1);
    *tv1 = var3;
    true
}

/// Append string "s2" to the string in "tv1".
pub fn grow_string_tv(tv1: &mut TypVal, s2: &[u8]) -> bool {
    if tv1.v_type != VarType::VarString || tv1.vval.v_string_is_null() {
        return false;
    }

    let joined = concat_str(tv1.vval.v_string_bytes(), s2);
    crate::eval::typval::tv_set_string(tv1, Some(joined.as_c_str()));
    true
}

/// Concatenate strings "tv1" and "tv2" and store the result in "tv1".
fn eval_concat_str(tv1: &mut TypVal, tv2: &mut TypVal) -> bool {
    let mut buf2 = [0u8; NUMBUFLEN];
    let Some(s2_cstr) = tv_get_string_buf_chk(&*tv2, &mut buf2) else {
        tv_clear(tv1);
        tv_clear(tv2);
        return false;
    };

    if grow_string_tv(tv1, s2_cstr.to_bytes()) {
        return true;
    }

    let mut buf1 = [0u8; NUMBUFLEN];
    let s1 = tv_get_string_buf(&*tv1, &mut buf1);
    let p = concat_str(s1.to_bytes(), s2_cstr.to_bytes());
    crate::eval::typval::tv_set_string(tv1, Some(p.as_c_str()));

    true
}

/// Add or subtract numbers "tv1" and "tv2" and store the result in "tv1".
fn eval_addsub_number(tv1: &mut TypVal, tv2: &mut TypVal, op: i32) -> bool {
    let mut error = false;
    let mut n1: Varnumber;
    let n2: Varnumber;
    let mut f1: TvFloat = 0.0;
    let mut f2: TvFloat = 0.0;

    if tv1.v_type == VarType::VarFloat {
        f1 = tv1.vval.v_float();
        n1 = 0;
    } else {
        n1 = tv_get_number_chk(&*tv1, Some(&mut error));
        if error {
            tv_clear(tv1);
            tv_clear(tv2);
            return false;
        }
        if tv2.v_type == VarType::VarFloat {
            f1 = n1 as TvFloat;
        }
    }
    if tv2.v_type == VarType::VarFloat {
        f2 = tv2.vval.v_float();
        n2 = 0;
    } else {
        n2 = tv_get_number_chk(&*tv2, Some(&mut error));
        if error {
            tv_clear(tv1);
            tv_clear(tv2);
            return false;
        }
        if tv1.v_type == VarType::VarFloat {
            f2 = n2 as TvFloat;
        }
    }
    tv_clear(tv1);

    if tv1.v_type == VarType::VarFloat || tv2.v_type == VarType::VarFloat {
        if op == b'+' as i32 {
            f1 += f2;
        } else {
            f1 -= f2;
        }
        tv1.v_type = VarType::VarFloat;
        tv1.vval = crate::eval::typval::TypvalVval::Float(f1);
    } else {
        if op == b'+' as i32 {
            n1 = n1.wrapping_add(n2);
        } else {
            n1 = n1.wrapping_sub(n2);
        }
        tv1.v_type = VarType::VarNumber;
        tv1.vval = crate::eval::typval::TypvalVval::Number(n1);
    }

    true
}

/// Handle fourth level expression: `+`, `-`, `.`, `..`
fn eval5(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    if !eval6(arg, rettv, evalarg.as_deref_mut(), false) {
        return false;
    }

    loop {
        let op = i32::from(arg.to_bytes_with_nul()[0]);
        let concat = op == b'.' as i32;
        if op != b'+' as i32 && op != b'-' as i32 && !concat {
            break;
        }

        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        if (op != b'+' as i32
            || (rettv.v_type != VarType::VarList && rettv.v_type != VarType::VarBlob))
            && (op == b'.' as i32 || rettv.v_type != VarType::VarFloat)
            && evaluate
            && ((op == b'.' as i32 && !tv_check_str(&*rettv))
                || (op != b'.' as i32 && !tv_check_num(&*rettv)))
        {
            tv_clear(rettv);
            return false;
        }

        let adv = if op == b'.' as i32 && arg.to_bytes().get(1) == Some(&b'.') {
            2
        } else {
            1
        };
        *arg = crate::charset::skipwhite_at(arg, adv);
        let mut var2 = tv_initial_value();
        if !eval6(arg, &mut var2, evalarg.as_deref_mut(), op == b'.' as i32) {
            tv_clear(rettv);
            return false;
        }

        if evaluate {
            if op == b'.' as i32 {
                if !eval_concat_str(rettv, &mut var2) {
                    return false;
                }
            } else if op == b'+' as i32
                && rettv.v_type == VarType::VarBlob
                && var2.v_type == VarType::VarBlob
            {
                eval_addblob(rettv, &mut var2);
            } else if op == b'+' as i32
                && rettv.v_type == VarType::VarList
                && var2.v_type == VarType::VarList
            {
                if !eval_addlist(rettv, &mut var2) {
                    return false;
                }
            } else if !eval_addsub_number(rettv, &mut var2, op) {
                return false;
            }
            tv_clear(&mut var2);
        }
    }
    true
}

/// Multiply or divide or compute the modulo of numbers "tv1" and "tv2".
fn eval_multdiv_number(tv1: &mut TypVal, tv2: &mut TypVal, op: i32) -> bool {
    let mut n1: Varnumber;
    let n2: Varnumber;
    let mut use_float = false;

    let mut f1: TvFloat = 0.0;
    let mut f2: TvFloat = 0.0;
    let mut error = false;
    if tv1.v_type == VarType::VarFloat {
        f1 = tv1.vval.v_float();
        use_float = true;
        n1 = 0;
    } else {
        n1 = tv_get_number_chk(&*tv1, Some(&mut error));
    }
    tv_clear(tv1);
    if error {
        tv_clear(tv2);
        return false;
    }

    if tv2.v_type == VarType::VarFloat {
        if !use_float {
            f1 = n1 as TvFloat;
            use_float = true;
        }
        f2 = tv2.vval.v_float();
        n2 = 0;
    } else {
        n2 = tv_get_number_chk(&*tv2, Some(&mut error));
        tv_clear(tv2);
        if error {
            return false;
        }
        if use_float {
            f2 = n2 as TvFloat;
        }
    }

    if use_float {
        if op == b'*' as i32 {
            f1 *= f2;
        } else if op == b'/' as i32 {
            f1 = if f2 == 0.0 {
                if f1 == 0.0 {
                    f64::NAN
                } else if f1 > 0.0 {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                }
            } else {
                f1 / f2
            };
        } else {
            emsg(gettext(c"E804: Cannot use '%' with Float"));
            return false;
        }
        tv1.v_type = VarType::VarFloat;
        tv1.vval = crate::eval::typval::TypvalVval::Float(f1);
    } else {
        if op == b'*' as i32 {
            n1 = n1.wrapping_mul(n2);
        } else if op == b'/' as i32 {
            n1 = num_divide(n1, n2);
        } else {
            n1 = num_modulus(n1, n2);
        }
        tv1.v_type = VarType::VarNumber;
        tv1.vval = crate::eval::typval::TypvalVval::Number(n1);
    }

    true
}

/// Handle fifth level expression: `*`, `/`, `%`
fn eval6(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    want_string: bool,
) -> bool {
    if !eval7(arg, rettv, evalarg.as_deref_mut(), want_string) {
        return false;
    }

    loop {
        let op = i32::from(arg.to_bytes_with_nul()[0]);
        if op != b'*' as i32 && op != b'/' as i32 && op != b'%' as i32 {
            break;
        }

        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

        *arg = crate::charset::skipwhite_at(arg, 1);
        let mut var2 = tv_initial_value();
        if !eval7(arg, &mut var2, evalarg.as_deref_mut(), false) {
            return false;
        }

        if evaluate && !eval_multdiv_number(rettv, &mut var2, op) {
            return false;
        }
    }

    true
}

/// Handle sixth level expression.
fn eval7(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    want_string: bool,
) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

    rettv.v_type = VarType::VarUnknown;

    let start_arg = *arg;
    let start_leader = start_arg.as_ptr().cast::<u8>();
    while matches!(arg.to_bytes_with_nul()[0], b'!' | b'-' | b'+') {
        *arg = crate::charset::skipwhite_at(arg, 1);
    }
    let mut leader_len = (arg.as_ptr() as usize).wrapping_sub(start_leader as usize);
    let leader_slice = &start_arg.to_bytes()[..leader_len];

    if EVAL_STATE.with(|s| s.eval7_recurse.get()) == MAX_EVAL7_RECURSE {
        semsg!(gettext(E_EXPRESSION_TOO_RECURSIVE_STR), *arg);
        return false;
    }
    EVAL_STATE.with(|s| s.eval7_recurse.set(s.eval7_recurse.get() + (1)));

    let ret: Option<bool> = match arg.to_bytes_with_nul()[0] {
        b'0'..=b'9' => {
            let mut ok = eval_number(arg, &mut *rettv, evaluate, want_string);

            if ok && evaluate && leader_len > 0 {
                ok = eval7_leader(&mut *rettv, true, leader_slice, &mut leader_len);
            }
            Some(ok)
        }
        b'"' => Some(eval_string(arg, &mut *rettv, evaluate, false)),
        b'\'' => Some(eval_lit_string(arg, &mut *rettv, evaluate, false)),
        b'[' => Some(eval_list(arg, &mut *rettv, evalarg.as_deref_mut())),
        b'#' => eval_lit_dict(arg, &mut *rettv, evalarg.as_deref_mut()),
        b'{' => get_lambda_tv(arg, &mut *rettv, evalarg.as_deref_mut())
            .or_else(|| eval_dict(arg, &mut *rettv, evalarg.as_deref_mut(), false)),
        b'&' => Some(eval_option(arg, Some(&mut *rettv), evaluate)),
        b'$' => {
            let next_c = arg.to_bytes().get(1).copied().unwrap_or(0);
            let ok = if next_c == b'"' || next_c == b'\'' {
                eval_interp_string(arg, &mut *rettv, evaluate)
            } else {
                eval_env_var(arg, &mut *rettv, i32::from(evaluate))
            };
            Some(ok)
        }
        b'@' => {
            *arg = crate::memory::CStrSubslice::advance(*arg, 1);
            if evaluate {
                let reg_str =
                    get_reg_contents_str(i32::from(arg.to_bytes_with_nul()[0]), K_GREG_EXPR_SRC);
                crate::eval::typval::tv_set_string(rettv, reg_str.as_deref());
            }
            if !arg.to_bytes().is_empty() {
                *arg = crate::memory::CStrSubslice::advance(*arg, 1);
            }
            Some(true)
        }
        b'(' => {
            *arg = crate::charset::skipwhite_at(arg, 1);

            let mut ok = eval1(arg, &mut *rettv, evalarg.as_deref_mut());
            if arg.to_bytes_with_nul()[0] == b')' {
                *arg = crate::memory::CStrSubslice::advance(*arg, 1);
            } else if ok {
                if arg.to_bytes().is_empty() {
                    emsg(gettext(E_MISSING_CLOSING_PAREN));
                } else {
                    semsg!(gettext(E_MISSING_CLOSING_PAREN_STR), *arg);
                }
                tv_clear(rettv);
                ok = false;
            }
            Some(ok)
        }
        _ => None,
    };

    let mut ok = if let Some(r) = ret {
        r
    } else {
        let mut s = *arg;
        let mut alias: Option<std::ffi::CString> = None;
        let len = get_name_len(arg, &mut alias, evaluate, true);
        if let Some(ref a) = alias {
            s = a.as_c_str();
        }

        let res = if len <= 0 {
            false
        } else {
            let flags = if evalarg.is_none() {
                0
            } else {
                evalarg.as_deref_mut().unwrap().eval_flags
            };
            let next_nonwhite = crate::charset::skipwhite(arg);
            if next_nonwhite.to_bytes().first() == Some(&b'(') {
                *arg = next_nonwhite;
                eval_func(
                    arg,
                    evalarg.as_deref_mut(),
                    s,
                    len,
                    &mut *rettv,
                    flags,
                    None,
                )
            } else if evaluate {
                eval_variable(s, len, Some(&mut *rettv), None, true, false)
            } else {
                let s_bytes = s.to_bytes();
                check_vars(&s_bytes[..(len as usize).min(s_bytes.len())]);
                if rettv.v_type == VarType::VarUnknown
                    && !evaluate
                    && s.to_bytes().starts_with(b"v:lua.")
                {
                    rettv.v_type = VarType::VarPartial;
                    let lua_pt = get_vim_var_partial(VV_LUA).unwrap();
                    lua_pt.pt_refcount.set(lua_pt.pt_refcount.get() + (1));
                    tv_set_partial(rettv, Some(lua_pt));
                }
                true
            }
        };
        drop(alias);
        res
    };

    *arg = crate::charset::skipwhite(arg);

    if ok {
        ok = handle_subscript(arg, start_arg, &mut *rettv, evalarg, true);
    }

    if ok && evaluate && leader_len > 0 {
        ok = eval7_leader(&mut *rettv, false, leader_slice, &mut leader_len);
    }

    EVAL_STATE.with(|s| s.eval7_recurse.set(s.eval7_recurse.get() - (1)));
    ok
}

/// Apply the leading "!" and "-" before an eval7 expression to "rettv".
fn eval7_leader(
    rettv: &mut TypVal,
    numeric_only: bool,
    leader_bytes: &[u8],
    end_off: &mut usize,
) -> bool {
    let mut off = *end_off;
    let mut ret = true;
    let mut error = false;
    let mut val: Varnumber = 0;
    let mut f: TvFloat = 0.0;

    if rettv.v_type == VarType::VarFloat {
        f = rettv.vval.v_float();
    } else {
        val = tv_get_number_chk(&*rettv, Some(&mut error));
    }
    if error {
        tv_clear(rettv);
        ret = false;
    } else {
        while off > 0 {
            off -= 1;
            if leader_bytes[off] == b'!' {
                if numeric_only {
                    off += 1;
                    break;
                }
                if rettv.v_type == VarType::VarFloat {
                    rettv.v_type = VarType::VarBool;
                    val = if f == 0.0 {
                        K_BOOL_VAR_TRUE_CONST
                    } else {
                        K_BOOL_VAR_FALSE_CONST
                    };
                } else {
                    val = Varnumber::from(val == 0);
                }
            } else if leader_bytes[off] == b'-' {
                if rettv.v_type == VarType::VarFloat {
                    f = -f;
                } else {
                    val = val.wrapping_neg();
                }
            }
        }
        if rettv.v_type == VarType::VarFloat {
            tv_clear(rettv);
            rettv.vval = crate::eval::typval::TypvalVval::Float(f);
        } else {
            tv_clear(rettv);
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(val);
        }
    }

    *end_off = off;
    ret
}

/// Call the function referred to in "rettv".
fn call_func_rettv(
    arg: &mut &CStr,
    evalarg: Option<&mut Evalarg>,
    rettv: &mut TypVal,
    evaluate: bool,
    selfdict: Option<&TvDict>,
    basetv: Option<&mut TypVal>,
    lua_funcname: Option<&CStr>,
) -> bool {
    let mut pt = crate::eval::typval::PartialId::NULL;
    let mut functv = tv_initial_value();
    let funcname: &CStr;
    let mut is_lua = false;

    if evaluate {
        functv = std::mem::take(rettv);

        if functv.v_type == VarType::VarPartial {
            pt = functv.vval.v_partial();
            is_lua = is_luafunc(pt.opt());
            funcname = if is_lua {
                lua_funcname.unwrap_or(c"")
            } else {
                partial_name(pt.opt())
            };
        } else {
            if functv.vval.v_string_is_empty() {
                emsg(gettext(E_EMPTY_FUNCTION_NAME));
                tv_clear(&mut functv);
                return false;
            }
            funcname = functv.vval.v_string_cstr_or_empty();
        }
    } else {
        funcname = c"";
    }

    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: evaluate,
        fe_partial: pt.clone(),
        fe_selfdict: crate::eval::typval::DictId::from(selfdict),
        fe_basetv: basetv.map(std::mem::take),
        fe_found_var: false,
    };
    let ret = get_func_tv(
        funcname,
        if is_lua {
            ((arg.as_ptr() as usize).wrapping_sub(funcname.as_ptr() as usize)) as i32
        } else {
            -1
        },
        rettv,
        arg,
        evalarg,
        &mut funcexe,
    );

    if evaluate {
        tv_clear(&mut functv);
    }

    ret
}

/// Evaluate "->method()".
fn eval_lambda(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    *arg = crate::memory::CStrSubslice::advance(*arg, 2);
    let mut base = std::mem::take(rettv);

    let lambda_rc = get_lambda_tv(arg, &mut *rettv, evalarg.as_deref_mut());
    let ret = if lambda_rc != Some(true) {
        return false;
    } else if arg.to_bytes_with_nul()[0] != b'(' {
        if verbose {
            if crate::charset::skipwhite_slice(arg.to_bytes()).first() == Some(&b'(') {
                emsg(gettext(E_NOWHITESPACE));
            } else {
                semsg!(gettext(E_MISSINGPAREN), c"lambda",);
            }
        }
        tv_clear(rettv);
        false
    } else {
        call_func_rettv(
            arg,
            evalarg,
            &mut *rettv,
            evaluate,
            None,
            Some(&mut base),
            None,
        )
    };

    if evaluate {
        tv_clear(&mut base);
    }

    ret
}

/// Evaluate "->method()" or "->v:lua.method()".
fn eval_method<'a>(
    arg: &mut &'a CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

    let orig_arg = *arg;
    *arg = crate::memory::CStrSubslice::advance(*arg, 2);
    let mut base = std::mem::take(rettv);

    let mut len: i32;
    let mut name = *arg;
    let mut lua_funcname: Option<&'a CStr> = None;
    let mut alias: Option<std::ffi::CString> = None;

    if name.to_bytes().starts_with(b"v:lua.") {
        let lf = crate::memory::CStrSubslice::advance(*arg, 6);
        lua_funcname = Some(lf);
        let after_lf = skip_luafunc_name(lf);
        *arg = crate::charset::skipwhite(after_lf);
        len = ((arg.as_ptr() as usize).wrapping_sub(lf.as_ptr() as usize)) as i32;
    } else {
        len = get_name_len(arg, &mut alias, evaluate, true);
        if let Some(ref a) = alias {
            name = a.as_c_str();
        }
    }

    let mut tofree: Option<std::ffi::CString> = None;
    let mut ret = true;

    if len <= 0 {
        if verbose {
            if lua_funcname.is_none() {
                emsg(gettext(c"E260: Missing name after ->"));
            } else {
                semsg!(gettext(E_INVEXPR2), name);
            }
        }
        ret = false;
    } else {
        *arg = crate::charset::skipwhite(arg);

        let paren = vim_strchr(arg, b'(' as i32);
        if arg.to_bytes_with_nul()[0] != b'('
            && lua_funcname.is_none()
            && alias.is_none()
            && let Some(paren_cstr) = paren
        {
            let expr_len = (paren_cstr.as_ptr() as usize).wrapping_sub(name.as_ptr() as usize);
            let expr_cstr = bytes_to_cstring(&name.to_bytes()[..expr_len]);
            let expr_base = expr_cstr.as_ptr();
            let mut expr_cursor: &CStr = &expr_cstr;
            let mut ref_tv = tv_initial_value();
            if !eval7(&mut expr_cursor, &mut ref_tv, evalarg.as_deref_mut(), false) {
                *arg = crate::memory::CStrSubslice::advance(*arg, len as usize);
                ret = false;
            } else {
                let consumed = (expr_cursor.as_ptr() as usize).wrapping_sub(expr_base as usize);
                *arg = crate::memory::CStrSubslice::advance(*arg, consumed);
                if !crate::charset::skipwhite_slice(expr_cursor.to_bytes()).is_empty() {
                    if verbose {
                        semsg!(gettext(E_TRAILING_ARG), *arg);
                    }
                    ret = false;
                } else if ref_tv.v_type == VarType::VarFunc && !ref_tv.vval.v_string_is_null() {
                    let tf = crate::eval::typval::tv_take_string(&mut ref_tv).unwrap_or_default();
                    len = tf.as_bytes().len() as i32;
                    tofree = Some(tf);
                    name = tofree.as_deref().unwrap();
                } else if ref_tv.v_type == VarType::VarPartial
                    && let Some(ref_pt) = tv_partial(&ref_tv)
                {
                    if !ref_pt.pt_argv.borrow().is_empty() || !ref_pt.pt_dict.get().is_null() {
                        if verbose {
                            emsg(gettext(E_CANNOT_USE_PARTIAL_HERE));
                        }
                        ret = false;
                    } else {
                        let tf = partial_name(Some(ref_pt)).to_owned();
                        len = tf.as_bytes().len() as i32;
                        tofree = Some(tf);
                        name = tofree.as_deref().unwrap();
                    }
                } else {
                    if verbose {
                        semsg!(gettext(E_NOT_CALLABLE_TYPE_STR), &expr_cstr);
                    }
                    ret = false;
                }
            }
            tv_clear(&mut ref_tv);
        }

        if ret {
            if arg.to_bytes_with_nul()[0] != b'(' {
                if verbose {
                    semsg!(gettext(E_MISSINGPAREN), name);
                }
                ret = false;
            } else if ascii_iswhite(i32::from(
                orig_arg.to_bytes()[orig_arg.to_bytes().len() - arg.to_bytes().len() - 1],
            )) {
                if verbose {
                    emsg(gettext(E_NOWHITESPACE));
                }
                ret = false;
            } else if lua_funcname.is_some() {
                if evaluate {
                    rettv.v_type = VarType::VarPartial;
                    let lua_pt = get_vim_var_partial(VV_LUA).unwrap();
                    lua_pt.pt_refcount.set(lua_pt.pt_refcount.get() + (1));
                    tv_set_partial(rettv, Some(lua_pt));
                }
                ret = call_func_rettv(
                    arg,
                    evalarg.as_deref_mut(),
                    &mut *rettv,
                    evaluate,
                    None,
                    Some(&mut base),
                    lua_funcname,
                );
            } else {
                ret = eval_func(
                    arg,
                    evalarg,
                    name,
                    len,
                    &mut *rettv,
                    if evaluate { EVAL_EVALUATE } else { 0 },
                    Some(&mut base),
                );
            }
        }
    }

    if evaluate {
        tv_clear(&mut base);
    }
    drop(tofree);
    drop(alias);

    ret
}

/// Evaluate an "[expr]" or "[expr:expr]" index. Also "dict.key".
fn eval_index<'a>(
    arg: &mut &'a CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    let mut empty1 = false;
    let mut empty2 = false;
    let mut range = false;
    let mut key: Option<&'a CStr> = None;
    let mut keylen: isize = -1;

    if !check_can_index(rettv, evaluate, verbose) {
        return false;
    }

    let mut var1 = tv_initial_value();
    let mut var2 = tv_initial_value();
    if arg.to_bytes_with_nul()[0] == b'.' {
        let key_cstr = crate::memory::CStrSubslice::advance(*arg, 1);
        key = Some(key_cstr);
        keylen = key_cstr
            .to_bytes()
            .iter()
            .take_while(|&&b| eval_isdictc(i32::from(b)))
            .count() as isize;
        if keylen == 0 {
            return false;
        }
        *arg = crate::charset::skipwhite_at(key_cstr, keylen as usize);
    } else {
        *arg = crate::charset::skipwhite_at(arg, 1);
        if arg.to_bytes_with_nul()[0] == b':' {
            empty1 = true;
        } else if !eval1(arg, &mut var1, evalarg.as_deref_mut()) {
            return false;
        } else if evaluate && !tv_check_str(&var1) {
            tv_clear(&mut var1);
            return false;
        }

        if arg.to_bytes_with_nul()[0] == b':' {
            range = true;
            *arg = crate::charset::skipwhite_at(arg, 1);
            if arg.to_bytes_with_nul()[0] == b']' {
                empty2 = true;
            } else if !eval1(arg, &mut var2, evalarg) {
                if !empty1 {
                    tv_clear(&mut var1);
                }
                return false;
            } else if evaluate && !tv_check_str(&var2) {
                if !empty1 {
                    tv_clear(&mut var1);
                }
                tv_clear(&mut var2);
                return false;
            }
        }

        if arg.to_bytes_with_nul()[0] != b']' {
            if verbose {
                emsg(gettext(E_MISSBRAC));
            }
            tv_clear(&mut var1);
            if range {
                tv_clear(&mut var2);
            }
            return false;
        }
        *arg = crate::charset::skipwhite_at(arg, 1);
    }

    if evaluate {
        let res = eval_index_inner(
            rettv,
            range,
            (!empty1).then_some(&var1),
            (!empty2).then_some(&var2),
            false,
            key,
            keylen,
            verbose,
        );
        if !empty1 {
            tv_clear(&mut var1);
        }
        if range {
            tv_clear(&mut var2);
        }
        return res;
    }
    true
}

/// Check if "rettv" can have an [index] or [sli:ce]
fn check_can_index(rettv: &mut TypVal, evaluate: bool, verbose: bool) -> bool {
    match rettv.v_type {
        VarType::VarFunc | VarType::VarPartial => {
            if verbose {
                emsg(gettext(E_CANNOT_INDEX_A_FUNCREF));
            }
            return false;
        }
        VarType::VarFloat => {
            if verbose {
                emsg(gettext(E_USING_FLOAT_AS_STRING));
            }
            return false;
        }
        VarType::VarBool | VarType::VarSpecial => {
            if verbose {
                emsg(gettext(E_CANNOT_INDEX_SPECIAL_VARIABLE));
            }
            return false;
        }
        VarType::VarUnknown => {
            if evaluate {
                emsg(gettext(E_CANNOT_INDEX_SPECIAL_VARIABLE));
                return false;
            }
        }
        VarType::VarString
        | VarType::VarNumber
        | VarType::VarList
        | VarType::VarDict
        | VarType::VarBlob => {}
    }
    true
}

/// slice() function
pub fn f_slice(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };
    if !check_can_index(arg0, true, false) {
        return;
    }

    tv_copy(arg0, rettv);
    eval_index_inner(
        &mut *rettv,
        true,
        Some(arg1),
        if arg2.v_type == VarType::VarUnknown {
            None
        } else {
            Some(&*arg2)
        },
        true,
        None,
        0,
        false,
    );
}

/// Apply index or range to "rettv".
fn eval_index_inner(
    rettv: &mut TypVal,
    is_range: bool,
    var1: Option<&TypVal>,
    var2: Option<&TypVal>,
    exclusive: bool,
    key_opt: Option<&CStr>,
    keylen: isize,
    verbose: bool,
) -> bool {
    let mut n1: Varnumber = 0;
    let mut n2: Varnumber = 0;
    if let Some(var1) = var1
        && rettv.v_type != VarType::VarDict
    {
        n1 = tv_get_number(var1);
    }

    if is_range {
        if rettv.v_type == VarType::VarDict {
            if verbose {
                emsg(gettext(E_CANNOT_SLICE_DICTIONARY));
            }
            return false;
        }
        if let Some(var2) = var2 {
            n2 = tv_get_number(var2);
        } else {
            n2 = VARNUMBER_MAX;
        }
    }

    match rettv.v_type {
        VarType::VarBool
        | VarType::VarSpecial
        | VarType::VarFunc
        | VarType::VarFloat
        | VarType::VarPartial
        | VarType::VarUnknown => {}
        VarType::VarNumber | VarType::VarString => {
            let s = tv_get_string(&*rettv);
            let s_bytes = s.to_bytes();
            let len = s_bytes.len() as Varnumber;
            let v: Option<std::ffi::CString> = if exclusive {
                if is_range {
                    string_slice(s_bytes, n1, n2, exclusive)
                } else {
                    char_from_string(s_bytes, n1)
                }
            } else if is_range {
                if n1 < 0 {
                    n1 += len;
                    if n1 < 0 {
                        n1 = 0;
                    }
                }
                if n2 < 0 {
                    n2 += len;
                } else if n2 >= len {
                    n2 = len - 1;
                }
                if n1 >= len || n2 < 0 || n1 > n2 {
                    None
                } else {
                    Some(bytes_to_cstring(&s_bytes[n1 as usize..=n2 as usize]))
                }
            } else if n1 >= len || n1 < 0 {
                None
            } else {
                Some(bytes_to_cstring(&s_bytes[n1 as usize..=n1 as usize]))
            };
            tv_clear(rettv);
            if let Some(ref c) = v {
                crate::eval::typval::tv_set_string(rettv, Some(c.as_c_str()));
            } else {
                rettv.v_type = VarType::VarString;
                rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
            }
        }
        VarType::VarBlob => {
            tv_blob_slice_or_index(is_range, n1, n2, exclusive, rettv);
        }
        VarType::VarList => {
            if var1.is_none() {
                n1 = 0;
            }
            if var2.is_none() {
                n2 = VARNUMBER_MAX;
            }
            if !tv_list_slice_or_index(is_range, n1, n2, exclusive, rettv, verbose) {
                return false;
            }
        }
        VarType::VarDict => {
            let k_cow;
            let key: &CStr = match key_opt {
                Some(k) => k,
                None => {
                    let Some(k) = tv_get_string_chk(var1.unwrap()) else {
                        return false;
                    };
                    k_cow = k;
                    &k_cow
                }
            };

            let item = tv_dict_find_opt(rettv.vval.v_dict_opt(), key, keylen);

            if item.is_none() && verbose {
                if keylen > 0 {
                    semsg!(gettext(E_DICTKEY_LEN), keylen, key);
                } else {
                    semsg!(gettext(E_DICTKEY), key);
                }
            }
            let Some(item) = item else {
                return false;
            };
            if tv_is_luafunc(&item.tv()) {
                return false;
            }

            let mut tmp = tv_initial_value();
            tv_copy(&item.tv(), &mut tmp);
            tv_clear(rettv);
            *rettv = tmp;
        }
    }
    true
}

/// Get an option value
pub fn eval_option(arg: &mut &CStr, rettv: Option<&mut TypVal>, evaluate: bool) -> bool {
    let start_arg = *arg;
    let working = start_arg.to_bytes().first() == Some(&b'+');
    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;

    let option_end_opt = find_option_var_end(arg, &mut opt_idx, &mut opt_flags);
    let cur_arg = *arg;

    let Some(option_end) = option_end_opt else {
        if rettv.is_some() {
            semsg!(gettext(c"E112: Option name missing: %s"), cur_arg);
        }
        return false;
    };

    if !evaluate {
        *arg = option_end;

        return true;
    }

    let opt_len = (option_end.as_ptr() as usize).wrapping_sub(cur_arg.as_ptr() as usize);
    let opt_bytes = &cur_arg.to_bytes()[..opt_len];
    let is_tty_opt = is_tty_option(opt_bytes);

    let mut ret = true;
    if opt_idx == K_OPT_INVALID && !is_tty_opt {
        if rettv.is_some() {
            let opt_cstr = bytes_to_cstring(opt_bytes);
            semsg!(gettext(c"E113: Unknown option: %s"), &opt_cstr);
        }

        ret = false;
    } else if let Some(rettv) = rettv {
        let value = if is_tty_opt {
            get_tty_option(opt_bytes)
        } else {
            get_option_value(opt_idx, opt_flags)
        };
        debug_assert!(value.obj_type != ObjectType::ObjectTypeNil);

        *rettv = opt_to_tv(value.clone(), true);
        optval_free(value);
    } else if working && !is_tty_opt && is_option_hidden(opt_idx) {
        ret = false;
    }

    *arg = option_end;

    ret
}

/// Allocate a variable for a number constant. Also deals with "0z" for blob.
fn eval_number(arg: &mut &CStr, rettv: &mut TypVal, evaluate: bool, want_string: bool) -> bool {
    let arg_cstr = *arg;
    let arg_bytes = arg_cstr.to_bytes();
    let mut get_float = false;

    if !want_string && !arg_bytes.is_empty() {
        let mut rem = crate::charset::skipdigits_slice(&arg_bytes[1..]);
        if rem.first() == Some(&b'.') && rem.get(1).is_some_and(u8::is_ascii_digit) {
            get_float = true;
            rem = crate::charset::skipdigits_slice(&rem[2..]);
            if matches!(rem.first(), Some(&b'e' | &b'E')) {
                rem = &rem[1..];
                if matches!(rem.first(), Some(&b'-' | &b'+')) {
                    rem = &rem[1..];
                }
                if !rem.first().is_some_and(u8::is_ascii_digit) {
                    get_float = false;
                } else {
                    rem = crate::charset::skipdigits_slice(&rem[1..]);
                }
            }
            if rem
                .first()
                .is_some_and(|&b| b.is_ascii_alphabetic() || b == b'.')
            {
                get_float = false;
            }
        }
    }
    if get_float {
        let mut f: TvFloat = 0.0;
        let consumed = string2float(arg_cstr, &mut f);
        *arg = &arg_cstr[consumed.min(arg_bytes.len())..];
        if evaluate {
            rettv.v_type = VarType::VarFloat;
            rettv.vval = crate::eval::typval::TypvalVval::Float(f);
        }
    } else if arg_bytes.starts_with(b"0z") || arg_bytes.starts_with(b"0Z") {
        let mut bytes: Vec<u8> = Vec::new();
        let mut rem = &arg_bytes[2..];
        while rem.first().is_some_and(u8::is_ascii_hexdigit) {
            if !rem.get(1).is_some_and(u8::is_ascii_hexdigit) {
                if evaluate {
                    emsg(gettext(
                        c"E973: Blob literal should have an even number of hex characters",
                    ));
                }
                return false;
            }
            if evaluate {
                let byte = ((hex2nr(i32::from(rem[0])) << 4) + hex2nr(i32::from(rem[1]))) as u8;
                bytes.push(byte);
            }
            let step = if rem.get(2) == Some(&b'.') && rem.get(3).is_some_and(u8::is_ascii_hexdigit)
            {
                3
            } else {
                2
            };
            rem = &rem[step..];
        }
        if evaluate {
            let blob = tv_blob_alloc();
            *blob.bv_ga.borrow_mut() = bytes;
            tv_blob_set_ret(rettv, Some(blob));
        }
        *arg = &arg_cstr[arg_bytes.len() - rem.len()..];
    } else {
        let mut len: i32 = 0;
        let mut n: Varnumber = 0;
        crate::charset::vim_slice2nr(
            arg_bytes,
            None,
            Some(&mut len),
            STR2NR_ALL,
            Some(&mut n),
            None,
            true,
            None,
        );
        if len == 0 {
            if evaluate {
                semsg!(gettext(E_INVEXPR2), arg_cstr);
            }
            return false;
        }
        *arg = &arg_cstr[(len as usize).min(arg_bytes.len())..];
        if evaluate {
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(n);
        }
    }
    true
}

/// Evaluate a string constant and put the result in "rettv".
fn eval_string(arg: &mut &CStr, rettv: &mut TypVal, evaluate: bool, interpolate: bool) -> bool {
    let arg_bytes = arg.to_bytes();
    let mut extra: isize = if interpolate { 1 } else { 0 };
    let off: usize = if interpolate { 0 } else { 1 };

    let mut idx = off.min(arg_bytes.len());
    while idx < arg_bytes.len() && arg_bytes[idx] != b'"' {
        if arg_bytes[idx] == b'\\' && idx + 1 < arg_bytes.len() {
            idx += 1;
            if arg_bytes[idx] == b'<' {
                let mut modifiers: i32 = 0;
                let mut flags = FSK_KEYCODE | FSK_IN_STRING;

                extra += 5;

                if arg_bytes.get(idx + 1) != Some(&b'*') {
                    flags |= FSK_SIMPLIFY;
                }
                let mut p_slice = &arg_bytes[idx..];
                let rem_len = p_slice.len();
                if find_special_key(&mut p_slice, &mut modifiers, flags, None) != 0 {
                    idx = (idx + (rem_len - p_slice.len())).saturating_sub(1);
                }
            }
        } else if interpolate && (arg_bytes[idx] == b'{' || arg_bytes[idx] == b'}') {
            if arg_bytes[idx] == b'{' && arg_bytes.get(idx + 1) != Some(&b'{') {
                break;
            }
            idx += 1;
            if arg_bytes[idx - 1] == b'}' && arg_bytes.get(idx) != Some(&b'}') {
                semsg!(gettext(E_STRAY_CLOSING_CURLY_STR), *arg);
                return false;
            }
            extra -= 1;
        }
        idx += utfc_ptr2len(&arg[idx.min(arg_bytes.len())..]) as usize;
    }

    let cur_b = arg_bytes.get(idx).copied().unwrap_or(0);
    if cur_b != b'"' && !(interpolate && cur_b == b'{') {
        semsg!(gettext(c"E114: Missing quote: %s"), *arg);
        return false;
    }

    if !evaluate {
        let end_off = (idx + off).min(arg_bytes.len());
        *arg = &arg[end_off..];
        return true;
    }

    let len = ((idx as isize) + extra) as usize;
    let mut buf_vec = Vec::with_capacity(len);
    let mut rem = &arg_bytes[off.min(arg_bytes.len())..];

    while !rem.is_empty() && rem[0] != b'"' {
        if rem[0] == b'\\' {
            rem = &rem[1..];
            match rem.first().copied().unwrap_or(0) {
                b'b' => {
                    buf_vec.push(BS);
                    rem = &rem[1..];
                }
                b'e' => {
                    buf_vec.push(ESC);
                    rem = &rem[1..];
                }
                b'f' => {
                    buf_vec.push(FF);
                    rem = &rem[1..];
                }
                b'n' => {
                    buf_vec.push(b'\n');
                    rem = &rem[1..];
                }
                b'r' => {
                    buf_vec.push(b'\r');
                    rem = &rem[1..];
                }
                b't' => {
                    buf_vec.push(TAB);
                    rem = &rem[1..];
                }
                b'X' | b'x' | b'u' | b'U' => {
                    if rem.get(1).is_some_and(u8::is_ascii_hexdigit) {
                        let c = rem[0].to_ascii_uppercase();
                        let mut n = if c == b'X' {
                            2
                        } else if rem[0] == b'u' {
                            4
                        } else {
                            8
                        };
                        let mut nr: i32 = 0;
                        while n > 0 && rem.get(1).is_some_and(u8::is_ascii_hexdigit) {
                            n -= 1;
                            rem = &rem[1..];
                            nr = (nr << 4) + hex2nr(i32::from(rem[0]));
                        }
                        rem = &rem[1..];
                        if c != b'X' {
                            let mut utf_buf = [0u8; crate::cmdexpand::MB_MAXBYTES];
                            let ulen = utf_char2bytes(nr, &mut utf_buf) as usize;
                            buf_vec.extend_from_slice(&utf_buf[..ulen]);
                        } else {
                            buf_vec.push(nr as u8);
                        }
                    }
                }
                b'0'..=b'7' => {
                    let mut val = rem[0] - b'0';
                    rem = &rem[1..];
                    if let Some(&b1 @ b'0'..=b'7') = rem.first() {
                        val = (val << 3).wrapping_add(b1 - b'0');
                        rem = &rem[1..];
                        if let Some(&b2 @ b'0'..=b'7') = rem.first() {
                            val = (val << 3).wrapping_add(b2 - b'0');
                            rem = &rem[1..];
                        }
                    }
                    buf_vec.push(val);
                }
                b'<' => {
                    let mut flags = FSK_KEYCODE | FSK_IN_STRING;
                    if rem.get(1) != Some(&b'*') {
                        flags |= FSK_SIMPLIFY;
                    }
                    let mut spec_buf = [0u8; 64];
                    let spec_len = trans_special(&mut rem, &mut spec_buf, flags, false, None);
                    if spec_len != 0 {
                        buf_vec.extend_from_slice(&spec_buf[..spec_len as usize]);
                        if buf_vec.len() >= len {
                            iemsg(c"eval_string() used more space than allocated");
                        }
                    } else {
                        mb_copy_char(&mut rem, &mut buf_vec);
                    }
                }
                _ => {
                    mb_copy_char(&mut rem, &mut buf_vec);
                }
            }
        } else {
            if interpolate && (rem[0] == b'{' || rem[0] == b'}') {
                if rem[0] == b'{' && rem.get(1) != Some(&b'{') {
                    break;
                }
                rem = &rem[1..];
            }
            mb_copy_char(&mut rem, &mut buf_vec);
        }
    }
    buf_vec.push(0);
    crate::eval::typval::tv_set_string(rettv, Some(crate::memory::bytes_as_cstr(&buf_vec)));
    if rem.first() == Some(&b'"') && !interpolate {
        rem = &rem[1..];
    }
    let arg_len = arg.to_bytes().len();
    *arg = &arg[arg_len.saturating_sub(rem.len())..];

    true
}

/// Allocate a variable for a 'str''ing' constant.
fn eval_lit_string(arg: &mut &CStr, rettv: &mut TypVal, evaluate: bool, interpolate: bool) -> bool {
    let arg_bytes = arg.to_bytes();
    let mut reduce: isize = if interpolate { -1 } else { 0 };
    let off: usize = if interpolate { 0 } else { 1 };

    let mut idx = off.min(arg_bytes.len());
    while idx < arg_bytes.len() {
        if arg_bytes[idx] == b'\'' {
            if arg_bytes.get(idx + 1) != Some(&b'\'') {
                break;
            }
            reduce += 1;
            idx += 1;
        } else if interpolate {
            if arg_bytes[idx] == b'{' {
                if arg_bytes.get(idx + 1) != Some(&b'{') {
                    break;
                }
                idx += 1;
                reduce += 1;
            } else if arg_bytes[idx] == b'}' {
                idx += 1;
                if arg_bytes.get(idx) != Some(&b'}') {
                    semsg!(gettext(E_STRAY_CLOSING_CURLY_STR), *arg);
                    return false;
                }
                reduce += 1;
            }
        }
        idx += utfc_ptr2len(&arg[idx.min(arg_bytes.len())..]) as usize;
    }

    let cur_b = arg_bytes.get(idx).copied().unwrap_or(0);
    if cur_b != b'\'' && !(interpolate && cur_b == b'{') {
        semsg!(gettext(c"E115: Missing quote: %s"), *arg);
        return false;
    }

    let end_off = (idx + off).min(arg_bytes.len());
    if !evaluate {
        *arg = &arg[end_off..];
        return true;
    }

    let alloc_len = ((idx as isize) - reduce) as usize;
    let mut buf_vec = Vec::with_capacity(alloc_len);
    let mut rem = &arg_bytes[off.min(arg_bytes.len())..];

    while !rem.is_empty() {
        if rem[0] == b'\'' {
            if rem.get(1) != Some(&b'\'') {
                break;
            }
            rem = &rem[1..];
        } else if interpolate && (rem[0] == b'{' || rem[0] == b'}') {
            if rem[0] == b'{' && rem.get(1) != Some(&b'{') {
                break;
            }
            rem = &rem[1..];
        }
        mb_copy_char(&mut rem, &mut buf_vec);
    }
    buf_vec.push(0);
    crate::eval::typval::tv_set_string(rettv, Some(crate::memory::bytes_as_cstr(&buf_vec)));
    *arg = &arg[end_off..];

    true
}

/// Evaluate a single or double quoted string possibly containing expressions.
pub fn eval_interp_string(arg: &mut &CStr, rettv: &mut TypVal, evaluate: bool) -> bool {
    let mut ret: bool;
    let mut buf: Vec<u8> = if evaluate {
        Vec::with_capacity(80)
    } else {
        Vec::new()
    };

    *arg = crate::memory::CStrSubslice::advance(*arg, 1);
    let quote = i32::from(arg.to_bytes_with_nul()[0]);
    *arg = crate::memory::CStrSubslice::advance(*arg, 1);

    loop {
        let mut tv = tv_initial_value();
        if quote == b'"' as i32 {
            ret = eval_string(arg, &mut tv, evaluate, true);
        } else {
            ret = eval_lit_string(arg, &mut tv, evaluate, true);
        }
        if !ret {
            break;
        }
        if evaluate {
            let s_bytes = tv.vval.v_string_bytes();
            if !s_bytes.is_empty() {
                buf.extend_from_slice(s_bytes);
            }
            tv_clear(&mut tv);
        }

        if arg.to_bytes_with_nul()[0] != b'{' {
            *arg = crate::memory::CStrSubslice::advance(*arg, 1);
            break;
        }
        let Some(p) = eval_one_expr_in_str(arg, &mut buf, evaluate) else {
            ret = false;
            break;
        };
        *arg = p;
    }

    if ret && evaluate {
        crate::eval::typval::tv_set_string_len(rettv, Some(&buf));
    } else {
        rettv.v_type = VarType::VarString;
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
    true
}

/// @return  the function name of the partial.
pub fn partial_name(pt: Option<&TvPartial>) -> &CStr {
    if let Some(pt) = pt {
        if let Some(name) = pt.name_cstr() {
            return name;
        }
        if let Some(uf) = pt.func_opt() {
            return uf.uf_name.as_c_str();
        }
    }
    c""
}

/// Unreference a closure: decrement the reference count and free it when it
/// becomes zero.
pub fn partial_unref(pt: Option<&TvPartial>) {
    let Some(pt_ref) = pt else {
        return;
    };
    pt_ref.pt_refcount.set(pt_ref.pt_refcount.get() - (1));
    if pt_ref.pt_refcount.get() <= 0 {
        crate::eval::typval::tv_partial_free(pt_ref);
    }
}

/// Allocate a variable for a List and fill it from "*arg".
fn eval_list(arg: &mut &CStr, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    let mut l = crate::eval::typval::ListId::NULL;

    if evaluate {
        l = tv_list_alloc(K_LIST_LEN_SHOULD_KNOW);
    }

    *arg = crate::charset::skipwhite_at(arg, 1);
    while arg.to_bytes_with_nul()[0] != b']' && arg.to_bytes_with_nul()[0] != 0 {
        let mut tv = tv_initial_value();
        if !eval1(arg, &mut tv, evalarg.as_deref_mut()) {
            if evaluate {
                tv_list_free(l.opt());
            }
            return false;
        }
        if evaluate {
            tv.v_lock = VAR_UNLOCKED;
            tv_list_append_owned_tv(&l, tv);
        }

        let had_comma = arg.to_bytes_with_nul()[0] == b',';
        if had_comma {
            *arg = crate::charset::skipwhite_at(arg, 1);
        }

        if arg.to_bytes_with_nul()[0] == b']' {
            break;
        }

        if !had_comma {
            semsg!(gettext(c"E696: Missing comma in List: %s"), *arg);
            if evaluate {
                tv_list_free(l.opt());
            }
            return false;
        }
    }

    if arg.to_bytes_with_nul()[0] != b']' {
        semsg!(gettext(E_LIST_END), *arg);
        if evaluate {
            tv_list_free(l.opt());
        }
        return false;
    }

    *arg = crate::charset::skipwhite_at(arg, 1);
    if evaluate {
        tv_list_set_ret(rettv, l.opt());
    }

    true
}

pub fn func_equal(tv1: &TypVal, tv2: &TypVal, ic: bool) -> bool {
    let pt1 = tv_partial(tv1);
    let mut s1 = if tv1.v_type == VarType::VarFunc {
        tv1.vval.v_string_cstr()
    } else {
        Some(partial_name(pt1))
    };
    if s1.is_some_and(CStr::is_empty) {
        s1 = None;
    }
    let pt2 = tv_partial(tv2);
    let mut s2 = if tv2.v_type == VarType::VarFunc {
        tv2.vval.v_string_cstr()
    } else {
        Some(partial_name(pt2))
    };
    if s2.is_some_and(CStr::is_empty) {
        s2 = None;
    }
    if s1 != s2 {
        return false;
    }

    let d1 = if tv1.v_type == VarType::VarFunc {
        crate::eval::typval::DictId::NULL
    } else {
        tv_partial(tv1).map_or(crate::eval::typval::DictId::NULL, |pt| pt.pt_dict.get())
    };
    let d2 = if tv2.v_type == VarType::VarFunc {
        crate::eval::typval::DictId::NULL
    } else {
        tv_partial(tv2).map_or(crate::eval::typval::DictId::NULL, |pt| pt.pt_dict.get())
    };
    if d1.is_null() || d2.is_null() {
        if d1 != d2 {
            return false;
        }
    } else if !tv_dict_equal(d1.opt(), d2.opt(), ic) {
        return false;
    }

    let a1 = if tv1.v_type == VarType::VarFunc {
        0
    } else {
        tv_partial(tv1).map_or(0, |pt| pt.pt_argv.borrow().len() as i32)
    };
    let a2 = if tv2.v_type == VarType::VarFunc {
        0
    } else {
        tv_partial(tv2).map_or(0, |pt| pt.pt_argv.borrow().len() as i32)
    };
    if a1 != a2 {
        return false;
    }
    for i in 0..(a1 as usize) {
        let arg1 = tv_partial(tv1).unwrap().pt_argv.borrow()[i].clone();
        let arg2 = tv_partial(tv2).unwrap().pt_argv.borrow()[i].clone();
        if !tv_equal(&arg1, &arg2, ic) {
            return false;
        }
    }
    true
}

/// Get next (unique) copy ID.
pub fn get_copy_id() -> i32 {
    let next_copy_id = EVAL_STATE
        .with(|s| s.current_copy_id.get())
        .wrapping_add(COPYID_INC);
    EVAL_STATE.with(|s| s.current_copy_id.set(next_copy_id));
    next_copy_id
}

/// Do garbage collection for lists and dicts.
pub fn garbage_collect(testing: bool) -> bool {
    let mut abort = false;

    if !testing {
        g_st().want_garbage_collect.set(false);
        g_st().may_garbage_collect.set(false);
        g_st().garbage_collect_at_exit.set(false);
    }

    crate::runtime::estack_cleanup();

    let copy_id = get_copy_id();

    abort = abort || set_ref_in_previous_funccal(copy_id);
    abort = abort || garbage_collect_scriptvars(copy_id);

    {
        for buf in for_all_buffers_ptr() {
            let mut b_tv = buf.b_bufvar.tv().clone();
            abort = abort || set_ref_in_item(&mut b_tv, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_prompt_callback, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_prompt_interrupt, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_p_cfu, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_p_ofu, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_p_tsrfu, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_p_tfu, copy_id, None, None);
            abort = abort || set_ref_in_callback(&buf.b_p_ffu, copy_id, None, None);
            if !abort && !buf.b_p_cpt_cb.borrow().is_empty() {
                abort =
                    abort || set_ref_in_cpt_callbacks(&mut buf.b_p_cpt_cb.borrow_mut(), copy_id);
            }
        }
    }

    abort = abort || set_ref_in_insexpand_funcs(copy_id);
    abort = abort || set_ref_in_opfunc(copy_id);
    abort = abort || set_ref_in_tagfunc(copy_id);
    {
        abort = abort || set_ref_in_findfunc(copy_id);
    }

    {
        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_ptr(&tp) {
                let mut w_tv = wp.w_winvar.tv().clone();
                abort = abort || set_ref_in_item(&mut w_tv, copy_id, None, None);
            }
        }

        for cw in &*g_st().ctx_win_vec.borrow() {
            if let Some(cw_win) = cw.cw_win.opt() {
                let mut w_tv = cw_win.w_winvar.tv().clone();
                abort = abort || set_ref_in_item(&mut w_tv, copy_id, None, None);
            }
        }
    }

    {
        for tp in for_all_tabpages_ptr() {
            let mut tp_tv = tp.tp_winvar.get().tv().clone();
            abort = abort || set_ref_in_item(&mut tp_tv, copy_id, None, None);
        }
    }

    abort = abort || garbage_collect_globvars(copy_id);
    abort = abort || set_ref_in_call_stack(copy_id);
    abort = abort || set_ref_in_functions(copy_id);

    {
        for ct in crate::event::r#loop::main_loop_ptr().channels.values_mut() {
            set_ref_in_callback_reader(&mut ct.on_data, copy_id, None, None);
            set_ref_in_callback_reader(&mut ct.on_stderr, copy_id, None, None);
            set_ref_in_callback(&ct.on_exit, copy_id, None, None);
        }
    }

    let timers: Vec<std::rc::Rc<VimTimer>> = with_timers(|t| t.values().cloned().collect());
    for timer in timers {
        set_ref_in_callback(&timer.callback, copy_id, None, None);
    }

    abort = abort || set_ref_in_func_args(copy_id);
    abort = abort || garbage_collect_vimvars(copy_id);
    {
        abort = abort || set_ref_in_quickfix(copy_id);
    }

    let mut did_free = false;
    if !abort {
        did_free = free_unref_items(copy_id);
        did_free = free_unref_funccal(copy_id, i32::from(testing)) || did_free;
    } else {
        if opt_vars().p_verbose.get() > 0 {
            verb_msg(gettext(
                c"Not enough memory to set references, garbage collection aborted!",
            ));
        }
    }
    did_free
}

/// Free lists and dictionaries that are no longer referenced.
fn free_unref_items(copy_id: i32) -> bool {
    let mut did_free = false;

    crate::eval::typval::set_tv_in_free_unref_items(true);

    for idx in (0..gc_dicts_len()).rev() {
        let dd_ref = gc_dict_at(idx);
        if (dd_ref.dv_copy_id.get() & COPYID_MASK) != (copy_id & COPYID_MASK) {
            tv_dict_free_contents(&dd_ref);
            did_free = true;
        }
    }

    for idx in (0..gc_lists_len()).rev() {
        let ll_ref = gc_list_at(idx);
        if (tv_list_copyid(&ll_ref) & COPYID_MASK) != (copy_id & COPYID_MASK)
            && !tv_list_has_watchers(&ll_ref)
        {
            tv_list_free_contents(&ll_ref);
            did_free = true;
        }
    }

    let mut i = gc_dicts_len();
    while i > 0 {
        i -= 1;
        if (gc_dict_at(i).dv_copy_id.get() & COPYID_MASK) != (copy_id & COPYID_MASK) {
            gc_free_dict_at(i);
        }
    }

    let mut j = gc_lists_len();
    while j > 0 {
        j -= 1;
        let ll_ref = gc_list_at(j);
        if (ll_ref.lv_copy_id.get() & COPYID_MASK) != (copy_id & COPYID_MASK)
            && !tv_list_has_watchers(&ll_ref)
        {
            gc_free_list_at(j);
        }
    }

    crate::eval::typval::set_tv_in_free_unref_items(false);
    did_free
}

/// Mark all lists and dicts referenced through hashtab "ht" with "copyID".
pub fn set_ref_in_ht(
    ht: Option<&Hashtab>,
    copy_id: i32,
    mut list_stack: Option<&mut ListStack>,
) -> bool {
    let Some(ht_ref) = ht else {
        return false;
    };
    let mut abort = false;
    let mut ht_stack: HtStack = Vec::new();

    for (_, di) in ht_ref.iter_items() {
        let mut item_tv = di.tv().clone();
        abort = abort
            || set_ref_in_item(
                &mut item_tv,
                copy_id,
                Some(&mut ht_stack),
                list_stack.as_deref_mut(),
            );
    }

    while let Some(next_d) = ht_stack.pop() {
        if !abort && let Some(d_ref) = next_d.opt() {
            for (_, di) in d_ref.dv_hashtab.iter_items() {
                let mut item_tv = di.tv().clone();
                abort = abort
                    || set_ref_in_item(
                        &mut item_tv,
                        copy_id,
                        Some(&mut ht_stack),
                        list_stack.as_deref_mut(),
                    );
            }
        }
    }

    abort
}

/// Mark all lists and dicts referenced through list "l" with "copyID".
pub fn set_ref_in_list_items(
    l: Option<&TvList>,
    copy_id: i32,
    mut ht_stack: Option<&mut HtStack>,
) -> bool {
    let mut abort = false;
    let mut list_stack: ListStack = Vec::new();

    let mut cur_l = crate::eval::typval::ListId::from(l);
    loop {
        if let Some(cur_l_ref) = cur_l.opt() {
            for li in crate::eval::typval::list_iter_ptr(cur_l_ref.first_opt()) {
                if abort {
                    break;
                }
                let mut item_tv = li.tv().clone();
                abort = set_ref_in_item(
                    &mut item_tv,
                    copy_id,
                    ht_stack.as_deref_mut(),
                    Some(&mut list_stack),
                );
            }
        }

        let Some(next_l) = list_stack.pop() else {
            break;
        };
        cur_l = next_l;
    }

    abort
}

fn set_ref_in_item_dict(
    dd: Option<&TvDict>,
    copy_id: i32,
    mut ht_stack: Option<&mut HtStack>,
    mut list_stack: Option<&mut ListStack>,
) -> bool {
    let Some(dd) = dd else {
        return false;
    };
    if dd.dv_copy_id.get() == copy_id {
        return false;
    }

    dd.dv_copy_id.set(copy_id);
    let Some(ref mut ht_stack_ref) = ht_stack else {
        return set_ref_in_ht(Some(&dd.dv_hashtab), copy_id, list_stack);
    };

    ht_stack_ref.push(dd.id());

    for watcher in dd.watchers.borrow().iter() {
        set_ref_in_callback(
            &watcher.callback,
            copy_id,
            Some(&mut **ht_stack_ref),
            list_stack.as_deref_mut(),
        );
    }

    false
}

fn set_ref_in_item_list(
    ll: Option<&TvList>,
    copy_id: i32,
    ht_stack: Option<&mut HtStack>,
    list_stack: Option<&mut ListStack>,
) -> bool {
    let Some(ll_ref) = ll else {
        return false;
    };
    if ll_ref.lv_copy_id.get() == copy_id {
        return false;
    }

    ll_ref.lv_copy_id.set(copy_id);
    if let Some(list_stack) = list_stack {
        list_stack.push(ll_ref.into());
        false
    } else {
        set_ref_in_list_items(Some(ll_ref), copy_id, ht_stack)
    }
}

fn set_ref_in_item_partial(
    pt: Option<&TvPartial>,
    copy_id: i32,
    mut ht_stack: Option<&mut HtStack>,
    mut list_stack: Option<&mut ListStack>,
) -> bool {
    let Some(pt) = pt else {
        return false;
    };
    if pt.pt_copy_id.get() == copy_id {
        return false;
    }

    pt.pt_copy_id.set(copy_id);

    let mut abort = set_ref_in_func(pt.name_cstr(), pt.func_opt(), copy_id);

    if !pt.pt_dict.get().is_null() {
        let mut dtv = TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(pt.pt_dict.get()),
        };
        abort = abort
            || set_ref_in_item(
                &mut dtv,
                copy_id,
                ht_stack.as_deref_mut(),
                list_stack.as_deref_mut(),
            );
    }

    for arg in &mut *pt.pt_argv.borrow_mut() {
        abort = abort
            || set_ref_in_item(
                arg,
                copy_id,
                ht_stack.as_deref_mut(),
                list_stack.as_deref_mut(),
            );
    }

    abort
}

/// Mark all lists and dicts referenced through typval "tv" with "copyID".
pub fn set_ref_in_item(
    tv: &mut TypVal,
    copy_id: i32,
    ht_stack: Option<&mut HtStack>,
    list_stack: Option<&mut ListStack>,
) -> bool {
    match tv.v_type {
        VarType::VarDict => {
            set_ref_in_item_dict(tv.vval.v_dict_opt(), copy_id, ht_stack, list_stack)
        }
        VarType::VarList => {
            set_ref_in_item_list(tv.vval.v_list_opt(), copy_id, ht_stack, list_stack)
        }
        VarType::VarFunc => set_ref_in_func(tv.vval.v_string_cstr(), None, copy_id),
        VarType::VarPartial => {
            set_ref_in_item_partial(tv_partial(&*tv), copy_id, ht_stack, list_stack)
        }
        VarType::VarUnknown
        | VarType::VarBool
        | VarType::VarSpecial
        | VarType::VarFloat
        | VarType::VarNumber
        | VarType::VarString
        | VarType::VarBlob => false,
    }
}

/// Get the key for #{key: val} into "tv" and advance "arg".
fn get_literal_key(arg: &mut &CStr, tv: &mut TypVal) -> bool {
    let bytes = arg.to_bytes();
    let Some(&b0) = bytes.first() else {
        return false;
    };
    if !ascii_isalnum(i32::from(b0)) && b0 != b'_' && b0 != b'-' {
        return false;
    }
    let len = bytes
        .iter()
        .take_while(|&&b| ascii_isalnum(i32::from(b)) || b == b'_' || b == b'-')
        .count();
    crate::eval::typval::tv_set_string_len(&mut *tv, Some(&bytes[..len]));

    *arg = crate::charset::skipwhite(&arg[len..]);
    true
}

/// Allocate a variable for a Dictionary and fill it from "*arg".
fn eval_dict(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    literal: bool,
) -> Option<bool> {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    let mut tv = tv_initial_value();
    let mut key: Option<std::ffi::CString> = None;
    let mut curly_expr = crate::charset::skipwhite_at(arg, 1);
    let mut buf = [0u8; NUMBUFLEN];

    if curly_expr.to_bytes_with_nul()[0] != b'}'
        && !literal
        && eval1(&mut curly_expr, &mut tv, None)
        && crate::charset::skipwhite_slice(curly_expr.to_bytes()).first() == Some(&b'}')
    {
        return None;
    }

    let mut d = crate::eval::typval::DictId::NULL;
    if evaluate {
        d = tv_dict_alloc();
    }
    let mut tvkey = tv_initial_value();
    tv.v_type = VarType::VarUnknown;

    *arg = crate::charset::skipwhite_at(arg, 1);
    while arg.to_bytes_with_nul()[0] != b'}' && arg.to_bytes_with_nul()[0] != 0 {
        let key_res = if literal {
            get_literal_key(arg, &mut tvkey)
        } else {
            eval1(arg, &mut tvkey, evalarg.as_deref_mut())
        };
        if !key_res {
            if let Some(d_ref) = d.opt() {
                tv_dict_free(d_ref);
            }
            return Some(false);
        }
        if arg.to_bytes_with_nul()[0] != b':' {
            semsg!(gettext(c"E720: Missing colon in Dictionary: %s"), *arg,);
            tv_clear(&mut tvkey);
            if let Some(d_ref) = d.opt() {
                tv_dict_free(d_ref);
            }
            return Some(false);
        }
        if evaluate {
            let Some(key_cstr) = tv_get_string_buf_chk(&tvkey, &mut buf) else {
                tv_clear(&mut tvkey);
                if let Some(d_ref) = d.opt() {
                    tv_dict_free(d_ref);
                }
                return Some(false);
            };
            key = Some(key_cstr.to_owned());
        }

        *arg = crate::charset::skipwhite_at(arg, 1);
        if !eval1(arg, &mut tv, evalarg.as_deref_mut()) {
            tv_clear(&mut tvkey);
            if let Some(d_ref) = d.opt() {
                tv_dict_free(d_ref);
            }
            return Some(false);
        }
        if evaluate {
            let key_cstr = key.as_deref().unwrap_or(c"");
            if tv_dict_find(d.opt(), key_cstr, -1).is_some() {
                semsg!(
                    gettext(c"E721: Duplicate key in Dictionary: \"%s\""),
                    key_cstr,
                );
                tv_clear(&mut tvkey);
                tv_clear(&mut tv);
                if let Some(d_ref) = d.opt() {
                    tv_dict_free(d_ref);
                }
                return Some(false);
            }
            let item = tv_dict_item_alloc(key_cstr);
            tv.v_lock = VAR_UNLOCKED;
            item.set_tv(std::mem::take(&mut tv));
            if !tv_dict_add(&d, &item) {
                tv_dict_item_free(&item);
            }
        }
        tv_clear(&mut tvkey);

        let had_comma = arg.to_bytes_with_nul()[0] == b',';
        if had_comma {
            *arg = crate::charset::skipwhite_at(arg, 1);
        }

        if arg.to_bytes_with_nul()[0] == b'}' {
            break;
        }
        if !had_comma {
            semsg!(gettext(c"E722: Missing comma in Dictionary: %s"), *arg,);
            if let Some(d_ref) = d.opt() {
                tv_dict_free(d_ref);
            }
            return Some(false);
        }
    }

    if arg.to_bytes_with_nul()[0] != b'}' {
        semsg!(gettext(E_723_END), *arg,);
        if let Some(d_ref) = d.opt() {
            tv_dict_free(d_ref);
        }
        return Some(false);
    }

    *arg = crate::charset::skipwhite_at(arg, 1);
    if evaluate {
        tv_dict_set_ret(rettv, d.opt());
    }

    Some(true)
}

const E_723_END: &CStr = c"E723: Missing end of Dictionary '}': %s";

/// Evaluate a literal dictionary: #{key: val, key: val}
fn eval_lit_dict(
    arg: &mut &CStr,
    rettv: &mut TypVal,
    evalarg: Option<&mut Evalarg>,
) -> Option<bool> {
    if arg.to_bytes().get(1) == Some(&b'{') {
        *arg = crate::memory::CStrSubslice::advance(*arg, 1);
        eval_dict(arg, rettv, evalarg, true)
    } else {
        None
    }
}

/// Convert the string to a floating point number.
pub fn string2float(text: &CStr, ret_value: &mut TvFloat) -> usize {
    let bytes = text.to_bytes();
    if bytes.len() >= 3 && bytes[..3].eq_ignore_ascii_case(b"inf") {
        *ret_value = f64::INFINITY;
        return 3;
    }
    if bytes.len() >= 4 && bytes[..4].eq_ignore_ascii_case(b"-inf") {
        *ret_value = f64::NEG_INFINITY;
        return 4;
    }
    if bytes.len() >= 3 && bytes[..3].eq_ignore_ascii_case(b"nan") {
        *ret_value = f64::NAN;
        return 3;
    }
    let (val, consumed) = crate::strings::strtod_slice(bytes);
    *ret_value = val;
    consumed
}

/// Get the value of an environment variable.
fn eval_env_var(arg: &mut &CStr, rettv: &mut TypVal, evaluate: i32) -> bool {
    let orig_arg = *arg;
    *arg = crate::memory::CStrSubslice::advance(*arg, 1);
    let len = get_env_len(arg);

    if evaluate != 0 {
        if len == 0 {
            return false;
        }
        let orig_bytes = orig_arg.to_bytes();
        let env_with_dollar =
            bytes_to_cstring(&orig_bytes[..(len as usize + 1).min(orig_bytes.len())]);
        let env_name =
            CStr::from_bytes_until_nul(&env_with_dollar.to_bytes_with_nul()[1..]).unwrap_or(c"");
        let mut string = vim_getenv(env_name).filter(|s| !s.as_bytes().is_empty());
        if string.is_none() {
            let exp = expand_env_save(&env_with_dollar);
            if exp.to_bytes().first().copied().unwrap_or(0) != b'$' {
                string = Some(exp);
            }
        }
        rettv.v_lock = VAR_UNLOCKED;
        if let Some(ref s) = string {
            crate::eval::typval::tv_set_string(rettv, Some(s.as_c_str()));
        } else {
            rettv.v_type = VarType::VarString;
            rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
        }
    }

    true
}

/// Builds a process argument vector from a Vimscript object (TypVal).
pub fn tv_to_argv(
    cmd_tv: &mut TypVal,
    cmd: Option<&mut std::ffi::CString>,
    executable: Option<&mut bool>,
) -> Vec<std::ffi::CString> {
    if cmd_tv.v_type == VarType::VarString {
        let cmd_str = tv_get_string(&*cmd_tv);
        if let Some(cmd_out) = cmd {
            *cmd_out = (*cmd_str).to_owned();
        }
        return shell_build_argv(Some(&cmd_str), None);
    }

    if cmd_tv.v_type != VarType::VarList {
        semsg!(gettext(E_INVARG2), c"expected String or List",);
        return Vec::new();
    }

    let argl = cmd_tv.vval.v_list_opt();
    let argc = tv_list_len(argl);
    if argc == 0 {
        emsg(gettext(E_INVARG));
        return Vec::new();
    }
    let argl = argl.unwrap();

    let arg0 = tv_get_string_chk(&argl.first_opt().unwrap().tv()).map(std::borrow::Cow::into_owned);
    let mut exe_resolved: Option<std::ffi::CString> = None;
    let Some(arg0) = arg0 else {
        return Vec::new();
    };
    if !os_can_exe(&arg0, Some(&mut exe_resolved), true) {
        if let Some(executable) = executable {
            let mut buf = [0u8; IOSIZE];
            format_to_buf!(&mut buf, c"'%s' is not executable", &arg0);
            semsg!(gettext(E_INVARG_NVAL), c"cmd", &buf,);
            *executable = false;
        }
        return Vec::new();
    }
    let exe_cstr = exe_resolved.unwrap();

    let mut argv = Vec::with_capacity(argc as usize);
    for (i, arg) in crate::eval::typval::list_iter_ptr(argl.first_opt()).enumerate() {
        let Some(a) = tv_get_string_chk(&arg.tv()).map(std::borrow::Cow::into_owned) else {
            return Vec::new();
        };
        argv.push(if i == 0 { exe_cstr.clone() } else { a });
    }
    if let Some(cmd_out) = cmd {
        *cmd_out = exe_cstr;
    }

    argv
}

fn string_to_list(mut bytes: &[u8], keepempty: bool) -> crate::eval::typval::ListId {
    if !keepempty && bytes.last() == Some(&b'\n') {
        bytes = &bytes[..bytes.len() - 1];
    }
    let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    encode_list_write(Some(&list), bytes);
    list
}

fn get_system_output_as_rettv(argvars: &mut [TypVal], rettv: &mut TypVal, retlist: bool) {
    let mut wait_time: Proftime = 0;
    let profiling = g_st().do_profiling.get() == PROF_YES;

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    if check_secure() {
        return;
    }

    let Some(input) = save_tv_as_string(&mut argvars[1], false, false) else {
        return;
    };

    let mut executable = true;
    let argv = tv_to_argv(&mut argvars[0], None, Some(&mut executable));
    if argv.is_empty() {
        if !executable {
            set_vim_var_nr(VV_SHELL_ERROR, -1);
        }
        return;
    }

    if opt_vars().p_verbose.get() > 3 {
        let cmdstr = shell_argv_to_str(&argv);
        verbose_enter_scroll();
        smsg!(0, gettext(c"Executing command: \"%s\""), &cmdstr);
        msg_puts(c"\n\n");
        verbose_leave_scroll();
    }

    if profiling {
        prof_child_enter(&mut wait_time);
    }

    let mut res: Vec<u8> = Vec::new();
    let status = os_system(argv, &input, Some(&mut res));

    if profiling {
        prof_child_exit(&mut wait_time);
    }

    drop(input);

    set_vim_var_nr(VV_SHELL_ERROR, Varnumber::from(status));

    if res.is_empty() {
        if retlist {
            tv_list_alloc_ret(rettv, 0);
        } else {
            crate::eval::typval::tv_set_string(rettv, Some(c""));
        }
        return;
    }

    if retlist {
        let mut keepempty: i32 = 0;
        if argvars[1].v_type != VarType::VarUnknown && argvars[2].v_type != VarType::VarUnknown {
            keepempty = tv_get_number(&argvars[2]) as i32;
        }
        rettv.vval = crate::eval::typval::TypvalVval::List(string_to_list(&res, keepempty != 0));
        tv_list_ref(rettv.vval.v_list().opt());
        rettv.v_type = VarType::VarList;
    } else {
        for b in &mut res {
            if *b == 0 {
                *b = 1;
            }
        }
        #[cfg(windows)]
        {
            let nread = res.len();
            let mut d = 0usize;
            let mut s = 0usize;
            while s < nread {
                if res[s] == b'\r' && s + 1 < nread && res[s + 1] == b'\n' {
                    s += 1;
                }
                res[d] = res[s];
                d += 1;
                s += 1;
            }
            res.truncate(d);
        }
        crate::eval::typval::tv_set_string_len(rettv, Some(&res));
    }
}

/// f_system - the Vimscript system() function
pub fn f_system(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_system_output_as_rettv(argvars, rettv, false);
}

pub fn f_systemlist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    get_system_output_as_rettv(argvars, rettv, true);
}

/// Get a callback from "arg". It can be a Funcref or a function name.
pub fn callback_from_typval(callback: &Callback, arg: &TypVal) -> bool {
    let mut r = true;

    let is_lua_tbl = nlua_is_table_from_lua(arg);

    if arg.v_type == VarType::VarPartial
        && let Some(pt) = arg.vval.v_partial().rc()
    {
        pt.pt_refcount.set(pt.pt_refcount.get() + (1));
        callback
            .data
            .set(crate::eval::typval::CallbackData::partial(pt));
    } else if arg.v_type == VarType::VarString
        && !arg.vval.v_string_is_null()
        && ascii_isdigit(arg.vval.v_string_first_byte())
    {
        r = false;
    } else if arg.v_type == VarType::VarFunc || arg.v_type == VarType::VarString {
        if let Some(name) = arg.vval.v_string_cstr() {
            if name.is_empty() {
                callback.data.set(crate::eval::typval::CallbackData::NONE);
            } else {
                let funcref = if arg.v_type == VarType::VarString
                    && let Some(sl) = get_scriptlocal_funcname(Some(name))
                {
                    sl
                } else {
                    name.to_owned()
                };
                func_ref(Some(funcref.as_c_str()));
                callback
                    .data
                    .set(crate::eval::typval::CallbackData::funcref(funcref));
            }
        } else {
            r = false;
        }
    } else if is_lua_tbl {
        if let Some(name) = nlua_register_table_as_callable(arg) {
            callback
                .data
                .set(crate::eval::typval::CallbackData::funcref(name));
        } else {
            r = false;
        }
    } else if arg.v_type == VarType::VarSpecial
        || (arg.v_type == VarType::VarNumber && arg.vval.v_number() == 0)
    {
        callback.data.set(crate::eval::typval::CallbackData::NONE);
    } else {
        r = false;
    }

    if !r {
        emsg(gettext(c"E921: Invalid callback argument"));
        return false;
    }
    true
}

pub fn get_callback_depth() -> i32 {
    EVAL_STATE.with(|s| s.callback_depth.get())
}

/// @return  whether the callback could be called.
pub fn callback_call(
    callback: &Callback,
    argcount_in: i32,
    argvars_in: &mut [TypVal],
    rettv: &mut TypVal,
) -> bool {
    if i64::from(EVAL_STATE.with(|s| s.callback_depth.get())) > opt_vars().p_mfd.get() {
        emsg(gettext(E_COMMAND_TOO_RECURSIVE));
        return false;
    }

    let partial: crate::eval::typval::PartialId;
    let funcref_rc: Option<std::rc::Rc<CStr>>;
    let name: &CStr;
    match callback.cb_type() {
        K_CALLBACK_FUNCREF => {
            funcref_rc = callback.data.as_funcref();
            let funcref = funcref_rc.as_deref().unwrap_or(c"");
            if funcref.to_bytes().starts_with(b"v:lua.") {
                name = &funcref[6..];
                if check_luafunc_name(name, false) == 0 {
                    return false;
                }
                partial = crate::eval::typval::PartialId::from(get_vim_var_partial(VV_LUA));
            } else {
                name = funcref;
                partial = crate::eval::typval::PartialId::NULL;
            }
        }
        K_CALLBACK_PARTIAL => {
            partial = callback.data.as_partial().unwrap_or_default();
            name = partial_name(partial.opt());
        }
        K_CALLBACK_LUA => {
            let mut luaargs = Array::with_capacity(argcount_in as usize);
            for arg_tv in argvars_in.iter_mut().take(argcount_in as usize) {
                *luaargs.next_slot_mut() = vim_to_object(arg_tv);
                luaargs.size += 1;
            }
            let luaref = callback.data.as_luaref();
            EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() + (1)));
            let res = nlua_call_ref(luaref, None, luaargs, K_RET_OBJECT);
            EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() - (1)));
            return match res {
                Ok(result) => {
                    object_to_vim(result, rettv);
                    true
                }
                Err(err) => {
                    semsg_multiline!(c"emsg", c"E5108: %s", err.msg_cstr());
                    false
                }
            };
        }
        K_CALLBACK_EXPR => {
            EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() + (1)));
            let funcref_rc = callback.data.as_funcref();
            let expr = crate::charset::skipwhite(funcref_rc.as_deref().unwrap_or(c""));
            let r = eval0_simple_funccal(expr, rettv, None, &mut Evalarg { eval_flags: 1 });
            EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() - (1)));
            return r;
        }
        _ => {
            return false;
        }
    }

    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: partial.clone(),
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: None,
        fe_found_var: false,
    };

    EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() + (1)));
    let ret = call_func(name, -1, rettv, argcount_in, argvars_in, &mut funcexe);
    EVAL_STATE.with(|s| s.callback_depth.set(s.callback_depth.get() - (1)));
    ret
}

pub fn set_ref_in_callback(
    callback: &Callback,
    copy_id: i32,
    ht_stack: Option<&mut HtStack>,
    list_stack: Option<&mut ListStack>,
) -> bool {
    if callback.cb_type() == K_CALLBACK_PARTIAL
        && let Some(pt) = callback.data.as_partial()
    {
        let mut tv = TypVal {
            v_type: VarType::VarPartial,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Partial(pt),
        };
        return set_ref_in_item(&mut tv, copy_id, ht_stack, list_stack);
    }
    false
}

fn set_ref_in_callback_reader(
    reader: &mut CallbackReader,
    copy_id: i32,
    mut ht_stack: Option<&mut HtStack>,
    mut list_stack: Option<&mut ListStack>,
) -> bool {
    if set_ref_in_callback(
        &reader.cb,
        copy_id,
        ht_stack.as_deref_mut(),
        list_stack.as_deref_mut(),
    ) {
        return true;
    }

    if !reader.self_dict.is_null() {
        let mut tv = TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(reader.self_dict.clone()),
        };
        return set_ref_in_item(&mut tv, copy_id, ht_stack, list_stack);
    }
    false
}

pub fn find_timer_by_nr(xx: Varnumber) -> Option<std::rc::Rc<VimTimer>> {
    with_timers(|t| t.get(&(xx as u64)).cloned())
}

pub fn add_timer_info(rettv: &mut TypVal, timer: &VimTimer) {
    let list = rettv.vval.v_list_ref();
    let dict = tv_dict_alloc();

    tv_list_append_dict(list, Some(&dict));
    tv_dict_add_nr(&dict, c"id", Varnumber::from(timer.timer_id));
    tv_dict_add_nr(&dict, c"time", timer.timeout);
    tv_dict_add_nr(&dict, c"paused", Varnumber::from(timer.paused.get()));
    tv_dict_add_nr(
        &dict,
        c"repeat",
        if timer.repeat_count.get() < 0 {
            -1
        } else {
            Varnumber::from(timer.repeat_count.get())
        },
    );

    let di = tv_dict_item_alloc(c"callback");
    if !tv_dict_add(&dict, &di) {
        tv_dict_item_free(&di);
    } else {
        callback_put(&timer.callback, &mut di.tv_mut());
    }
}

pub fn add_timer_info_all(rettv: &mut TypVal) {
    let all_timers: Vec<std::rc::Rc<VimTimer>> = with_timers(|t| t.values().cloned().collect());
    tv_list_alloc_ret(rettv, all_timers.len() as isize);
    for timer in all_timers {
        if !timer.stopped.get() || timer.active_calls.get() > 0 {
            add_timer_info(rettv, &timer);
        }
    }
}

/// Invoked on the main loop.
pub fn timer_due_cb(_tw: &mut TimeWatcher, timer_id: u64) {
    let Some(timer) = with_timers(|t| t.get(&timer_id).cloned()) else {
        return;
    };
    let save_did_emsg = g_st().did_emsg.get();
    let called_emsg_before = g_st().called_emsg.get();

    if timer.stopped.get() || timer.paused.get() {
        return;
    }

    timer.active_calls.set(timer.active_calls.get() + 1);

    if timer.repeat_count.get() >= 0 {
        timer.repeat_count.set(timer.repeat_count.get() - 1);

        if timer.repeat_count.get() == 0 {
            timer_stop(&timer);
        }
    }

    let mut argv = [tv_initial_value(), tv_initial_value()];
    argv[0].v_type = VarType::VarNumber;

    argv[0].vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(timer.timer_id));

    let mut rettv = tv_initial_value();

    callback_call(&timer.callback, 1, &mut argv, &mut rettv);

    if g_st().called_emsg.get() > called_emsg_before && g_st().did_emsg.get() != 0 {
        timer.emsg_count.set(timer.emsg_count.get() + 1);

        if g_st().did_throw.get() {
            discard_current_exception();
        }
    }
    g_st().did_emsg.set(save_did_emsg);

    if timer.emsg_count.get() >= 3 {
        timer_stop(&timer);
    }

    tv_clear(&mut rettv);

    if !timer.stopped.get() && timer.timeout == 0 {
        time_watcher_start(&mut timer.watcher.borrow_mut(), 0, 0);
    }
    timer.active_calls.set(timer.active_calls.get() - 1);
}
pub fn timer_start(timeout: i64, repeat_count: i32, callback: Callback) -> u64 {
    let id = EVAL_STATE.with(|s| s.last_timer_id.get());
    EVAL_STATE.with(|s| s.last_timer_id.set(s.last_timer_id.get() + (1)));
    let timer = std::rc::Rc::new(VimTimer {
        watcher: std::cell::RefCell::new(TimeWatcher::new()),
        timer_id: id as i32,
        repeat_count: std::cell::Cell::new(repeat_count),
        active_calls: std::cell::Cell::new(0),
        emsg_count: std::cell::Cell::new(0),
        timeout,
        stopped: std::cell::Cell::new(false),
        paused: std::cell::Cell::new(false),
        callback,
    });

    let timer_id = timer.timer_id as u64;
    {
        let mut watcher = timer.watcher.borrow_mut();
        time_watcher_init(
            crate::event::r#loop::main_loop_ptr(),
            &mut watcher,
            TimeWatcherData::EvalTimer(timer_id),
        );
        watcher.events =
            multiqueue_new_child(loop_queues(crate::event::r#loop::main_loop_ptr()).events);
        watcher.blockable = true;
        time_watcher_start(&mut watcher, timeout as u64, timeout as u64);
    }

    with_timers_mut(|t| {
        t.insert(timer_id, timer);
    });
    timer_id
}

pub fn timer_stop(timer: &VimTimer) {
    if timer.stopped.get() {
        return;
    }
    timer.stopped.set(true);
    {
        let mut watcher = timer.watcher.borrow_mut();
        time_watcher_stop(&mut watcher);
        time_watcher_close(&mut watcher);
    }
}

pub(crate) fn timer_close_cb(_tw: &mut TimeWatcher, timer_id: u64) {
    let _ = with_timers_mut(|t| t.remove(&timer_id));
}

pub fn timer_stop_all() {
    let all_timers: Vec<std::rc::Rc<VimTimer>> = with_timers(|t| t.values().cloned().collect());
    for timer in all_timers {
        timer_stop(&timer);
    }
}

pub fn timer_teardown() {
    timer_stop_all();
}

/// Saves a TypVal as a string buffer (`None` on error, `Some(vec)` on success).
pub fn save_tv_as_string(tv: &mut TypVal, endnl: bool, crlf: bool) -> Option<Vec<u8>> {
    if tv.v_type == VarType::VarUnknown {
        return Some(Vec::new());
    }

    if tv.v_type != VarType::VarList && tv.v_type != VarType::VarNumber {
        return tv_get_string_chk(&*tv).map(|ret| ret.to_bytes().to_vec());
    }

    if tv.v_type == VarType::VarNumber {
        let Some(buf) = buflist_findnr(tv.vval.v_number() as i32) else {
            semsg!(gettext(E_NOBUFNR), tv.vval.v_number());
            return None;
        };
        let mut total_len: usize = 0;
        for lnum in 1..=buf.b_ml.ml_line_count.get() {
            total_len += crate::memline::ml_get_buf_len(&buf, lnum) as usize + 1;
        }

        if total_len == 0 {
            return Some(Vec::new());
        }

        let mut ret = Vec::with_capacity(total_len);
        for lnum in 1..=buf.b_ml.ml_line_count.get() {
            for &p_byte in &crate::memline::ml_get_buf_slice(&buf, lnum) {
                ret.push(if p_byte == b'\n' { 0 } else { p_byte });
            }
            ret.push(b'\n');
        }
        return Some(ret);
    }

    debug_assert!(tv.v_type == VarType::VarList);
    let Some(list) = tv.vval.v_list_opt() else {
        return Some(Vec::new());
    };
    let sep_len = if crlf { 2usize } else { 1usize };
    let mut total_len: usize = 0;
    for li in crate::eval::typval::list_iter_ptr(list.first_opt()) {
        total_len += tv_get_string(&li.tv()).to_bytes().len() + sep_len;
    }

    if total_len == 0 {
        return Some(Vec::new());
    }

    let mut ret = Vec::with_capacity(total_len);
    for li in crate::eval::typval::list_iter_ptr(list.first_opt()) {
        {
            let li_tv = li.tv();
            let s = tv_get_string(&li_tv);
            for &s_byte in s.to_bytes() {
                ret.push(if s_byte == b'\n' { 0 } else { s_byte });
            }
        }
        if endnl || tv_list_item_next(&li).is_some() {
            if crlf {
                ret.push(b'\r');
            }
            ret.push(b'\n');
        }
    }
    Some(ret)
}

/// Convert the specified byte index of line 'lnum' in buffer 'buf' to a
/// character index.
pub fn buf_byteidx_to_charidx(buf: Option<&FileBuf>, mut lnum: Linenr, byteidx: i32) -> i32 {
    let Some(buf) = buf else {
        return -1;
    };
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return -1;
    }

    if lnum > buf.b_ml.ml_line_count.get() {
        lnum = buf.b_ml.ml_line_count.get();
    }

    let str_bytes = crate::memline::ml_get_buf_slice(buf, lnum);

    if str_bytes.is_empty() {
        return 0;
    }

    let mut off: usize = 0;
    let mut count: i32 = 0;
    while off < str_bytes.len() && (off as isize) <= (byteidx as isize) {
        off += crate::mbyte::utfc_slice2len(&str_bytes[off..]);
        count += 1;
    }

    if off >= str_bytes.len() && byteidx != 0 && (off as isize) == (byteidx as isize) {
        count += 1;
    }

    count - 1
}

/// Convert the specified character index of line 'lnum' in buffer 'buf' to a
/// byte index.
pub fn buf_charidx_to_byteidx(buf: Option<&FileBuf>, mut lnum: Linenr, mut charidx: i32) -> i32 {
    let Some(buf) = buf else {
        return -1;
    };
    if buf.b_ml.ml_mfp.borrow().is_none() {
        return -1;
    }

    if lnum > buf.b_ml.ml_line_count.get() {
        lnum = buf.b_ml.ml_line_count.get();
    }

    let str_bytes = crate::memline::ml_get_buf_slice(buf, lnum);

    let mut off: usize = 0;
    while off < str_bytes.len() {
        charidx -= 1;
        if charidx <= 0 {
            break;
        }
        off += crate::mbyte::utfc_slice2len(&str_bytes[off..]);
    }

    off as i32
}

/// Translate a Vimscript object into a position.
pub fn var2fpos(
    tv: &TypVal,
    dollar_lnum: bool,
    ret_fnum: &mut i32,
    charcol: bool,
    wp: &WinInfoInfo,
) -> Option<CursorPos> {
    let bp = wp.w_buffer.get();
    let mut pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };

    if tv.v_type == VarType::VarList {
        let mut error = false;

        let l = tv.vval.v_list();
        if l.is_null() {
            return None;
        }

        pos.lnum = tv_list_find_nr(l.opt(), 0, Some(&mut error)) as Linenr;

        if error || pos.lnum <= 0 || pos.lnum > bp.b_ml.ml_line_count.get() {
            return None;
        }

        pos.col = tv_list_find_nr(l.opt(), 1, Some(&mut error)) as Colnr;

        if error {
            return None;
        }
        let len = if charcol {
            mb_charlen(&crate::memline::ml_get_buf_slice(bp.borrow(), pos.lnum))
        } else {
            ml_get_buf_len(bp.borrow(), pos.lnum)
        };

        if let Some(li) = tv_list_find(l.opt(), 1)
            && li.tv().v_type == VarType::VarString
            && !li.tv().vval.v_string_is_null()
            && li.tv().vval.v_string_bytes() == b"$"
        {
            pos.col = len + 1;
        }

        if pos.col == 0 || pos.col > len + 1 {
            return None;
        }
        pos.col -= 1;

        pos.coladd = tv_list_find_nr(l.opt(), 2, Some(&mut error)) as Colnr;

        if error {
            pos.coladd = 0;
        }

        return Some(pos);
    }

    let name_cstr = tv_get_string_chk(tv)?;
    let name_bytes = name_cstr.to_bytes();

    if name_bytes.first() == Some(&b'.') {
        pos = CursorPos {
            lnum: wp.w_cursor.get().lnum,
            col: wp.w_cursor.get().col,
            coladd: wp.w_cursor.get().coladd,
        };
    } else if name_bytes == b"v" {
        if g_st().visual_global.borrow().active && wp == curwin() {
            pos = CursorPos {
                lnum: g_st().visual_global.borrow().start.lnum,
                col: g_st().visual_global.borrow().start.col,
                coladd: g_st().visual_global.borrow().start.coladd,
            };
        } else {
            pos = CursorPos {
                lnum: wp.w_cursor.get().lnum,
                col: wp.w_cursor.get().col,
                coladd: wp.w_cursor.get().coladd,
            };
        }
    } else if name_bytes.first() == Some(&b'\'') {
        let mname_b = name_bytes.get(1).copied().unwrap_or(0);
        let mname = i32::from(mname_b);
        let fm = mark_get(bp.borrow(), wp, None, K_MARK_ALL, mname)?;
        if fm.mark.lnum <= 0 {
            return None;
        }

        pos = fm.mark;

        if mname_b.is_ascii_uppercase() || mname_b.is_ascii_digit() {
            *ret_fnum = fm.fnum;
        }
    }
    if pos.lnum != 0 {
        if charcol {
            pos.col = buf_byteidx_to_charidx(bp.opt(), pos.lnum, pos.col);
        }
        return Some(pos);
    }

    pos.coladd = 0;

    if name_bytes.first() == Some(&b'w') && dollar_lnum {
        // The "w_valid" flags are not reset when moving the cursor, so update
        // the view before reading either endpoint.
        update_topline(wp);
        pos.col = 0;
        if name_bytes.get(1) == Some(&b'0') {
            pos.lnum = if wp.w_topline.get() > 0 {
                wp.w_topline.get()
            } else {
                1
            };
            return Some(pos);
        } else if name_bytes.get(1) == Some(&b'$') {
            validate_botline_win(wp);
            pos.lnum = if wp.w_botline.get() > 0 {
                wp.w_botline.get() - 1
            } else {
                0
            };
            return Some(pos);
        }
    } else if name_bytes.first() == Some(&b'$') {
        if dollar_lnum {
            pos.lnum = bp.b_ml.ml_line_count.get();
            pos.col = 0;
        } else {
            pos.lnum = wp.w_cursor.get().lnum;
            pos.col = if charcol {
                mb_charlen(&crate::memline::ml_get_buf_slice(
                    bp.borrow(),
                    wp.w_cursor.get().lnum,
                ))
            } else {
                ml_get_buf_len(bp.borrow(), wp.w_cursor.get().lnum)
            };
        }
        return Some(pos);
    }
    None
}

/// Convert list in "arg" into position "posp" and optional file number "fnump".
pub fn list2fpos(
    arg: &mut TypVal,
    posp: &mut CursorPos,
    mut fnump: Option<&mut i32>,
    mut curswantp: Option<&mut Colnr>,
    charcol: bool,
) -> bool {
    if arg.v_type != VarType::VarList {
        return false;
    }
    let l = arg.vval.v_list();
    if l.is_null()
        || tv_list_len(l.opt()) < if fnump.is_none() { 2 } else { 3 }
        || tv_list_len(l.opt()) > if fnump.is_none() { 4 } else { 5 }
    {
        return false;
    }

    let mut i: i32 = 0;
    let mut n: i32;
    if fnump.is_some() {
        n = tv_list_find_nr(l.opt(), i, None) as i32;

        i += 1;
        if n < 0 {
            return false;
        }
        if n == 0 {
            n = curbuf().b_fnum.get();
        }
        *fnump.as_deref_mut().unwrap() = n;
    }

    n = tv_list_find_nr(l.opt(), i, None) as i32;

    i += 1;
    if n < 0 {
        return false;
    }
    posp.lnum = n;

    n = tv_list_find_nr(l.opt(), i, None) as i32;

    i += 1;
    if n < 0 {
        return false;
    }
    if charcol {
        let Some(buf) = buflist_findnr(if let Some(fnump) = fnump {
            *fnump
        } else {
            curbuf().b_fnum.get()
        }) else {
            return false;
        };
        if buf.b_ml.ml_mfp.borrow().is_none() {
            return false;
        }
        let cur_lnum = curwin().w_cursor.get().lnum;

        n = buf_charidx_to_byteidx(
            Some(&buf),
            if posp.lnum == 0 { cur_lnum } else { posp.lnum },
            n,
        ) + 1;
    }
    posp.col = n;

    n = tv_list_find_nr(l.opt(), i, None) as i32;

    if n < 0 {
        posp.coladd = 0;
    } else {
        posp.coladd = n;
    }

    if let Some(ref mut p) = curswantp {
        **p = tv_list_find_nr(l.opt(), i + 1, None) as Colnr;
    }

    true
}

/// Get the length of an environment variable name.
pub fn get_env_len(arg: &mut &CStr) -> i32 {
    let len = env_name_len(arg.to_bytes());
    *arg = crate::memory::CStrSubslice::advance(*arg, len);
    len as i32
}

/// Get the length of the name of a function or internal variable.
pub fn get_id_len(arg: &mut &CStr) -> i32 {
    let bytes = arg.to_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() && eval_isnamec(i32::from(bytes[idx])) {
        if bytes[idx] == b':'
            && (idx > 1 || (idx == 1 && vim_strchr(NAMESPACE_CHAR, i32::from(bytes[0])).is_none()))
        {
            break;
        }
        idx += 1;
    }
    if idx == 0 {
        return 0;
    }

    *arg = crate::charset::skipwhite(&arg[idx..]);
    idx as i32
}

/// Get the length of the name of a variable or function.
pub fn get_name_len(
    arg: &mut &CStr,
    alias: &mut Option<std::ffi::CString>,
    evaluate: bool,
    verbose: bool,
) -> i32 {
    *alias = None;
    let start_arg = *arg;

    if arg.to_bytes().starts_with(&[K_SPECIAL, KS_EXTRA, KE_SNR]) {
        *arg = crate::memory::CStrSubslice::advance(*arg, 3);
        return get_id_len(arg) + 3;
    }
    let mut len = eval_fname_script(arg.to_bytes()) as i32;
    if len > 0 {
        *arg = crate::memory::CStrSubslice::advance(*arg, len as usize);
    }

    let mut expr_range: Option<(usize, usize)> = None;
    let p_cstr = find_name_end(
        arg,
        Some(&mut expr_range),
        if len > 0 { 0 } else { FNE_CHECK_START },
    );
    if let Some((expr_start_off, expr_end_off)) = expr_range {
        if !evaluate {
            len += ((p_cstr.as_ptr() as usize).wrapping_sub(arg.as_ptr() as usize)) as i32;
            *arg = crate::charset::skipwhite(p_cstr);
            return len;
        }

        let prefix_len = len as usize;
        let full_len = (p_cstr.as_ptr() as usize).wrapping_sub(start_arg.as_ptr() as usize);
        let Some(temp_string) = make_expanded_name(
            &start_arg.to_bytes()[..full_len],
            prefix_len + expr_start_off,
            prefix_len + expr_end_off,
        ) else {
            return -1;
        };
        let ret_len = temp_string.as_bytes().len() as i32;
        *alias = Some(temp_string);
        *arg = crate::charset::skipwhite(p_cstr);
        return ret_len;
    }

    len += get_id_len(arg);
    if len == 0 && verbose && !arg.to_bytes().is_empty() {
        semsg!(gettext(E_INVEXPR2), *arg);
    }

    len
}

/// Find the end of a variable or function name, taking care of magic braces.
pub fn find_name_end<'a>(
    arg: &'a CStr,
    mut expr_range: Option<&mut Option<(usize, usize)>>,
    flags: i32,
) -> &'a CStr {
    if let Some(ref mut er) = expr_range {
        **er = None;
    }

    let bytes = arg.to_bytes();
    if (flags & FNE_CHECK_START) != 0
        && !bytes
            .first()
            .is_some_and(|&b0| eval_isnamec1(i32::from(b0)) || b0 == b'{')
    {
        return arg;
    }

    let mut mb_nest: i32 = 0;
    let mut br_nest: i32 = 0;
    let mut expr_start_off: Option<usize> = None;
    let mut expr_end_off: Option<usize> = None;
    let mut idx = 0usize;

    while idx < bytes.len() {
        let b0 = bytes[idx];
        if !(eval_isnamec(i32::from(b0))
            || b0 == b'{'
            || ((flags & FNE_INCL_BR) != 0
                && (b0 == b'['
                    || (b0 == b'.'
                        && bytes
                            .get(idx + 1)
                            .is_some_and(|&b1| eval_isdictc(i32::from(b1))))))
            || mb_nest != 0
            || br_nest != 0)
        {
            break;
        }

        if b0 == b'\'' {
            idx += 1;
            while idx < bytes.len() && bytes[idx] != b'\'' {
                idx += crate::mbyte::utfc_slice2len(&bytes[idx..]).max(1);
            }
            if idx >= bytes.len() {
                break;
            }
        } else if b0 == b'"' {
            idx += 1;
            while idx < bytes.len() && bytes[idx] != b'"' {
                if bytes[idx] == b'\\' && idx + 1 < bytes.len() {
                    idx += 1;
                }
                idx += crate::mbyte::utfc_slice2len(&bytes[idx..]).max(1);
            }
            if idx >= bytes.len() {
                break;
            }
        } else if br_nest == 0
            && mb_nest == 0
            && b0 == b':'
            && ((idx > 1 && bytes[idx - 1] != b'}')
                || (idx == 1 && !NAMESPACE_CHAR.to_bytes().contains(&bytes[0])))
        {
            break;
        }

        let cur_b = bytes[idx];
        if mb_nest == 0 {
            if cur_b == b'[' {
                br_nest += 1;
            } else if cur_b == b']' {
                br_nest -= 1;
            }
        }

        if br_nest == 0 {
            if cur_b == b'{' {
                mb_nest += 1;
                if expr_start_off.is_none() {
                    expr_start_off = Some(idx);
                }
            } else if cur_b == b'}' {
                mb_nest -= 1;
                if mb_nest == 0 && expr_end_off.is_none() {
                    expr_end_off = Some(idx);
                }
            }
        }

        idx += crate::mbyte::utfc_slice2len(&bytes[idx..]).max(1);
    }

    if let (Some(er), Some(s)) = (expr_range, expr_start_off) {
        *er = Some((s, expr_end_off.unwrap_or(usize::MAX)));
    }

    crate::memory::CStrSubslice::advance(arg, idx.min(bytes.len()))
}

/// Expands out the 'magic' {}'s in a variable/function name.
fn make_expanded_name(
    in_bytes: &[u8],
    expr_start_off: usize,
    expr_end_off: usize,
) -> Option<std::ffi::CString> {
    if expr_end_off >= in_bytes.len() || expr_start_off >= expr_end_off {
        return None;
    }

    let expr_cstr = bytes_to_cstring(&in_bytes[expr_start_off + 1..expr_end_off]);
    let temp_result = eval_to_string(&expr_cstr, false, false)?;

    let prefix = &in_bytes[..expr_start_off];
    let suffix = &in_bytes[expr_end_off + 1..];
    let mut buf = Vec::with_capacity(prefix.len() + temp_result.as_bytes().len() + suffix.len());
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(temp_result.as_bytes());
    buf.extend_from_slice(suffix);
    let mut retval = Some(bytes_to_cstring(&buf));

    if let Some(ref rv) = retval {
        let mut next_range: Option<(usize, usize)> = None;
        let end_cstr = find_name_end(rv, Some(&mut next_range), 0);
        if let Some((next_start, next_end)) = next_range {
            let end_len = (end_cstr.as_ptr() as usize).wrapping_sub(rv.as_ptr() as usize);
            retval = make_expanded_name(&rv.as_bytes()[..end_len], next_start, next_end);
        }
    }

    retval
}

pub fn eval_isnamec<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int() as u8;
    ascii_isalnum(i32::from(ch)) || ch == b'_' || ch == b':' || ch == AUTOLOAD_CHAR
}

pub fn eval_isnamec1<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int() as u8;
    ascii_isalpha(i32::from(ch)) || ch == b'_'
}

pub fn eval_isdictc<C: crate::charset::AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int() as u8;
    ascii_isalnum(i32::from(ch)) || ch == b'_'
}

/// Set the v:argv list.
pub fn set_argv_var(argv: &[std::ffi::CString]) {
    let l = tv_list_alloc(argv.len() as isize);

    tv_list_set_lock(Some(&l), VAR_FIXED);
    for arg in argv {
        tv_list_append_string(&l, Some(arg.as_c_str()), -1);
        tv_list_last(Some(&l)).unwrap().tv_mut().v_lock = VAR_FIXED;
    }
    set_vim_var_list(VV_ARGV, Some(&l));
}

/// Check if special v:lua value for calling lua functions.
pub fn is_luafunc(partial: Option<&TvPartial>) -> bool {
    let Some(partial) = partial else {
        return false;
    };
    get_vim_var_partial(VV_LUA).is_some_and(|p| ptr::eq(partial, &*p))
}

fn tv_is_luafunc(tv: &TypVal) -> bool {
    tv.v_type == VarType::VarPartial && is_luafunc(tv_partial(tv))
}

#[inline]
fn luafunc_name_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'\''))
        .count()
}

/// Skips one character past the end of the name of a v:lua function.
pub fn skip_luafunc_name(p: &CStr) -> &CStr {
    let len = luafunc_name_len(p.to_bytes());
    crate::memory::CStrSubslice::advance(p, len)
}

/// Check the function name after "v:lua."
pub fn check_luafunc_name(str_: &CStr, paren: bool) -> i32 {
    let bytes = str_.to_bytes();
    let len = luafunc_name_len(bytes);
    if bytes.get(len).copied().unwrap_or(0) != if paren { b'(' } else { 0 } {
        return 0;
    }
    len as i32
}

/// Return the character "str[index]" where "index" is the character index,
/// including composing characters.
pub fn char_from_string(s: &[u8], index: Varnumber) -> Option<std::ffi::CString> {
    let mut nchar = index;
    let slen = s.len();

    if index < 0 {
        let mut clen: Varnumber = 0;
        let mut nbyte: usize = 0;
        while nbyte < slen {
            nbyte += crate::mbyte::utfc_slice2len(&s[nbyte..]).max(1);
            clen += 1;
        }
        nchar = clen + index;
        if nchar < 0 {
            return None;
        }
    }

    let mut nbyte: usize = 0;
    while nchar > 0 && nbyte < slen {
        nbyte += crate::mbyte::utfc_slice2len(&s[nbyte..]).max(1);
        nchar -= 1;
    }
    if nbyte >= slen {
        return None;
    }
    let clen = crate::mbyte::utfc_slice2len(&s[nbyte..])
        .max(1)
        .min(slen - nbyte);
    Some(bytes_to_cstring(&s[nbyte..nbyte + clen]))
}

fn char_idx2byte(s: &[u8], idx: Varnumber) -> Option<usize> {
    let str_len = s.len();
    let mut nchar = idx;
    let mut nbyte: usize = 0;

    if nchar >= 0 {
        while nchar > 0 && nbyte < str_len {
            nbyte += crate::mbyte::utfc_slice2len(&s[nbyte..]).max(1);
            nchar -= 1;
        }
        nbyte = nbyte.min(str_len);
    } else {
        nbyte = str_len;
        while nchar < 0 && nbyte > 0 {
            nbyte -= 1;
            nbyte -= crate::mbyte::utf_head_off_slice(s, nbyte) as usize;
            nchar += 1;
        }
        if nchar < 0 {
            return None;
        }
    }
    Some(nbyte)
}

/// Return the slice "str[first : last]" using character indexes.
pub fn string_slice(
    s: &[u8],
    first: Varnumber,
    last: Varnumber,
    exclusive: bool,
) -> Option<std::ffi::CString> {
    let slen = s.len();
    let start_byte = char_idx2byte(s, first).unwrap_or(0);
    let end_byte = if (last == -1 && !exclusive) || last == VARNUMBER_MAX {
        slen
    } else {
        let mut eb = char_idx2byte(s, last)?;
        if !exclusive && eb < slen {
            eb += crate::mbyte::utfc_slice2len(&s[eb..]);
        }
        eb
    };

    if start_byte >= slen || end_byte <= start_byte {
        return None;
    }
    Some(bytes_to_cstring(&s[start_byte..end_byte.min(slen)]))
}

/// Handle subscripts, `.name`, funcref calls, and `->method()`.
pub fn handle_subscript<'a>(
    arg: &mut &'a CStr,
    start_arg: &'a CStr,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let evaluate = evalarg
        .as_ref()
        .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
    let mut ret = true;
    let mut selfdict = crate::eval::typval::DictId::NULL;
    let mut lua_funcname: Option<&'a CStr> = None;

    if tv_is_luafunc(rettv) {
        if !evaluate {
            tv_clear(rettv);
        }

        if arg.to_bytes_with_nul()[0] != b'.' {
            tv_clear(rettv);
            ret = false;
        } else {
            *arg = crate::memory::CStrSubslice::advance(*arg, 1);

            lua_funcname = Some(*arg);
            let len = check_luafunc_name(arg, true);
            if len == 0 {
                tv_clear(rettv);
                ret = false;
            }
            *arg = crate::memory::CStrSubslice::advance(*arg, len as usize);
        }
    }

    while ret
        && (((arg.to_bytes_with_nul()[0] == b'['
            || (arg.to_bytes_with_nul()[0] == b'.' && rettv.v_type == VarType::VarDict)
            || (arg.to_bytes_with_nul()[0] == b'(' && (!evaluate || tv_is_func(rettv))))
            && !ascii_iswhite(i32::from(
                start_arg
                    .to_bytes()
                    .get(
                        start_arg
                            .to_bytes()
                            .len()
                            .saturating_sub(arg.to_bytes().len() + 1),
                    )
                    .copied()
                    .unwrap_or(0),
            )))
            || arg.to_bytes().starts_with(b"->"))
    {
        if arg.to_bytes_with_nul()[0] == b'(' {
            ret = call_func_rettv(
                arg,
                evalarg.as_deref_mut(),
                rettv,
                evaluate,
                selfdict.opt(),
                None,
                lua_funcname,
            );

            if aborting() {
                if ret {
                    tv_clear(rettv);
                }
                ret = false;
            }
            tv_dict_unref(selfdict.opt());
            selfdict = crate::eval::typval::DictId::NULL;
        } else if arg.to_bytes_with_nul()[0] == b'-' {
            if arg.to_bytes().get(2) == Some(&b'{') {
                ret = eval_lambda(arg, rettv, evalarg.as_deref_mut(), verbose);
            } else {
                ret = eval_method(arg, rettv, evalarg.as_deref_mut(), verbose);
            }
        } else {
            tv_dict_unref(selfdict.opt());
            if rettv.v_type == VarType::VarDict {
                selfdict = rettv.vval.v_dict();
                if !selfdict.is_null() {
                    selfdict.dv_refcount.set(selfdict.dv_refcount.get() + 1);
                }
            } else {
                selfdict = crate::eval::typval::DictId::NULL;
            }
            if !eval_index(arg, rettv, evalarg.as_deref_mut(), verbose) {
                tv_clear(rettv);
                ret = false;
            }
        }
    }

    if let Some(selfdict_ref) = selfdict.opt()
        && tv_is_func(rettv)
    {
        set_selfdict(rettv, selfdict_ref);
    }

    tv_dict_unref(selfdict.opt());
    ret
}

pub fn set_selfdict(rettv: &mut TypVal, selfdict: &TvDict) {
    if rettv.v_type == VarType::VarPartial
        && let Some(pt) = tv_partial(&*rettv)
        && !pt.pt_auto.get()
        && !pt.pt_dict.get().is_null()
    {
        return;
    }
    make_partial(selfdict, rettv);
}

/// Make a copy of an item.
pub fn var_item_copy(
    conv: Option<&Vimconv>,
    from: &mut TypVal,
    to: &mut TypVal,
    deep: bool,
    copy_id: i32,
) -> bool {
    let mut ret = true;

    if EVAL_STATE.with(|s| s.var_item_copy_recurse.get()) >= DICT_MAXNEST {
        emsg(gettext(E_VARIABLE_NESTED_TOO_DEEP_FOR_MAKING_COPY));
        return false;
    }
    EVAL_STATE.with(|s| {
        s.var_item_copy_recurse
            .set(s.var_item_copy_recurse.get() + (1))
    });

    match from.v_type {
        VarType::VarNumber
        | VarType::VarFloat
        | VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarBool
        | VarType::VarSpecial => {
            tv_copy(&*from, to);
        }
        VarType::VarString => {
            let Some(conv) = conv.filter(|c| c.vc_type != CONV_NONE) else {
                tv_copy(&*from, to);
                EVAL_STATE.with(|s| {
                    s.var_item_copy_recurse
                        .set(s.var_item_copy_recurse.get() - (1))
                });
                return ret;
            };
            if from.vval.v_string_is_null() {
                tv_copy(&*from, to);
            } else {
                if let Some(conv_bytes) = string_convert(conv, from.vval.v_string_bytes(), None) {
                    crate::eval::typval::tv_set_string(
                        to,
                        Some(crate::memory::bytes_as_cstr(&conv_bytes)),
                    );
                } else {
                    crate::eval::typval::tv_set_string(to, from.vval.v_string_cstr());
                }
                to.v_lock = VAR_UNLOCKED;
            }
        }
        VarType::VarList => {
            to.v_type = VarType::VarList;
            to.v_lock = VAR_UNLOCKED;
            let Some(from_list) = from.vval.v_list_opt() else {
                to.vval = crate::eval::typval::TypvalVval::List(crate::eval::typval::ListId::NULL);
                EVAL_STATE.with(|s| {
                    s.var_item_copy_recurse
                        .set(s.var_item_copy_recurse.get() - (1))
                });
                return ret;
            };
            if copy_id != 0 && tv_list_copyid(from_list) == copy_id {
                let latest = crate::eval::typval::ListId::from(tv_list_latest_copy(from_list));
                to.vval = crate::eval::typval::TypvalVval::List(latest);
                tv_list_ref(to.vval.v_list_opt());
            } else {
                to.vval = crate::eval::typval::TypvalVval::List(crate::eval::typval::ListId::from(
                    tv_list_copy(conv, Some(from_list), deep, copy_id),
                ));
            }
            if to.vval.v_list().is_null() {
                ret = false;
            }
        }
        VarType::VarBlob => {
            tv_blob_copy(from.vval.v_blob_opt(), to);
        }
        VarType::VarDict => {
            to.v_type = VarType::VarDict;
            to.v_lock = VAR_UNLOCKED;
            let Some(from_dict) = from.vval.v_dict_opt() else {
                to.vval = crate::eval::typval::TypvalVval::Dict(crate::eval::typval::DictId::NULL);
                EVAL_STATE.with(|s| {
                    s.var_item_copy_recurse
                        .set(s.var_item_copy_recurse.get() - (1))
                });
                return ret;
            };
            if copy_id != 0 && from_dict.dv_copy_id.get() == copy_id {
                let cd = from_dict.dv_copydict.get();
                cd.dv_refcount.set(cd.dv_refcount.get() + 1);
                to.vval = crate::eval::typval::TypvalVval::Dict(cd);
            } else {
                to.vval = crate::eval::typval::TypvalVval::Dict(
                    tv_dict_copy(conv, Some(from_dict), deep, copy_id)
                        .unwrap_or(crate::eval::typval::DictId::NULL),
                );
            }
            if to.vval.v_dict().is_null() {
                ret = false;
            }
        }
        VarType::VarUnknown => {
            internal_error(c"var_item_copy(UNKNOWN)");
            ret = false;
        }
    }
    EVAL_STATE.with(|s| {
        s.var_item_copy_recurse
            .set(s.var_item_copy_recurse.get() - (1))
    });
    ret
}

/// ":echo expr1 ..." and ":echon expr1 ..."
pub fn ex_echo(eap: &mut Exarg) {
    let arg_slice = eap.arg.clone();
    let mut arg = arg_slice.as_cstr();
    let mut rettv = tv_initial_value();
    let mut atstart = true;
    let mut need_clear = true;
    let did_emsg_before = g_st().did_emsg.get();
    let called_emsg_before = g_st().called_emsg.get();
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip);

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    while !arg.to_bytes().is_empty()
        && arg.to_bytes_with_nul()[0] != b'|'
        && arg.to_bytes_with_nul()[0] != b'\n'
        && !g_st().got_int.get()
    {
        g_st().need_clr_eos.set(true);

        {
            let p = arg;
            if !eval1(&mut arg, &mut rettv, Some(&mut evalarg)) {
                if !aborting()
                    && g_st().did_emsg.get() == did_emsg_before
                    && g_st().called_emsg.get() == called_emsg_before
                {
                    semsg!(gettext(E_INVEXPR2), p);
                }
                g_st().need_clr_eos.set(false);
                break;
            }
            g_st().need_clr_eos.set(false);
        }

        if !eap.skip {
            if atstart {
                atstart = false;
                msg_ext_set_append(eap.cmdidx == Cmdidx::CmdEchon);
                msg_ext_no_fast();
                msg_ext_set_kind(c"echo");
                if eap.cmdidx == Cmdidx::CmdEcho {
                    if !g_st().msg_didout.get() {
                        msg_sb_eol();
                    }
                    msg_start();
                }
            } else if eap.cmdidx == Cmdidx::CmdEcho {
                msg_puts_hl(c" ", EVAL_STATE.with(|s| s.echo_hl_id.get()), false);
            }
            let tofree = encode_tv2echo(&mut rettv, None);
            msg_multiline(
                cstr_as_string(Some(&*tofree)),
                EVAL_STATE.with(|s| s.echo_hl_id.get()),
                true,
                false,
                &mut need_clear,
            );
        }
        tv_clear(&mut rettv);
        arg = crate::charset::skipwhite(arg);
    }
    eap.nextcmd = arg_slice.advance_to_opt_cstr(check_nextcmd(arg));
    msg_ext_set_append(false);

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    } else {
        if ui_has(K_UIMESSAGES)
            && (eap.arg.is_empty() || eap.arg.first_byte() == b'|' || eap.arg.first_byte() == b'\n')
        {
            msg_puts_len(b"", 0, 0, false);
        } else if need_clear {
            msg_clr_eos();
        }
        if eap.cmdidx == Cmdidx::CmdEcho {
            msg_end();
        }
    }
}

/// ":echohl {name}".
pub fn ex_echohl(eap: &mut Exarg) {
    EVAL_STATE.with(|s| s.echo_hl_id.set(syn_name2id(eap.arg.as_cstr())));
}

/// Returns the :echo highlight id.
pub fn get_echo_hl_id() -> i32 {
    EVAL_STATE.with(|s| s.echo_hl_id.get())
}

/// ":execute", ":echomsg", ":echoerr"
pub fn ex_execute(eap: &mut Exarg) {
    let arg_slice = eap.arg.clone();
    let mut arg = arg_slice.as_cstr();
    let mut rettv = tv_initial_value();
    let mut ret = true;
    let mut buf: Vec<u8> = Vec::with_capacity(80);
    let mut has_arg = false;

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    while !arg.to_bytes().is_empty()
        && arg.to_bytes_with_nul()[0] != b'|'
        && arg.to_bytes_with_nul()[0] != b'\n'
    {
        ret = eval1_emsg(&mut arg, &mut rettv, Some(eap));
        if !ret {
            break;
        }

        if !eap.skip {
            let mut argstr_owned: Option<std::ffi::CString> = None;
            let argstr_cow;
            let argstr: &CStr = if eap.cmdidx == Cmdidx::CmdExecute {
                argstr_cow = tv_get_string(&rettv);
                &argstr_cow
            } else {
                argstr_owned = Some(if rettv.v_type == VarType::VarString {
                    encode_tv2echo(&mut rettv, None)
                } else {
                    encode_tv2string(&mut rettv, None)
                });
                argstr_owned.as_deref().unwrap_or(c"")
            };
            if has_arg {
                buf.push(b' ');
            }
            has_arg = true;
            let bytes = argstr.to_bytes();
            if !bytes.is_empty() {
                buf.extend_from_slice(bytes);
            }
            drop(argstr_owned);
        }

        tv_clear(&mut rettv);
        arg = crate::charset::skipwhite(arg);
    }

    if ret && has_arg {
        buf.push(0);
        if eap.cmdidx == Cmdidx::CmdEchomsg {
            msg_ext_no_fast();
            msg_ext_set_kind(c"echomsg");
            msg(
                crate::memory::bytes_as_cstr(&buf),
                EVAL_STATE.with(|s| s.echo_hl_id.get()),
            );
        } else if eap.cmdidx == Cmdidx::CmdEchoerr {
            let save_did_emsg = g_st().did_emsg.get();
            msg_ext_no_fast();
            emsg_multiline(crate::memory::bytes_as_cstr(&buf), c"echoerr", HLF_E, true);
            if !g_st().force_abort.get() {
                g_st().did_emsg.set(save_did_emsg);
            }
        } else if eap.cmdidx == Cmdidx::CmdExecute {
            do_cmdline(
                Some(crate::memory::bytes_as_cstr(&buf)),
                eap.ea_getline.clone(),
                DOCMD_NOWAIT | DOCMD_VERBOSE,
            );
        }
    }

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }

    eap.nextcmd = arg_slice.advance_to_opt_cstr(check_nextcmd(arg));
}

/// Skip over the name of an option variable: "&option", "&g:option" or "&l:option".
pub fn find_option_var_end<'a>(
    arg: &mut &'a CStr,
    opt_idxp: &mut OptIndex,
    opt_flags: &mut i32,
) -> Option<&'a CStr> {
    let bytes = arg.to_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut off = 1usize;
    let p_bytes = &bytes[off..];
    if p_bytes.starts_with(b"g:") {
        *opt_flags = OPT_GLOBAL;
        off += 2;
    } else if p_bytes.starts_with(b"l:") {
        *opt_flags = OPT_LOCAL;
        off += 2;
    } else {
        *opt_flags = 0;
    }

    if let Some(end_off) = find_option_end(&bytes[off..], opt_idxp) {
        let p_cstr = &arg[off..];
        *arg = p_cstr;
        Some(&p_cstr[end_off..])
    } else {
        None
    }
}

pub fn var_flavour(varname: &[u8]) -> VarFlavour {
    if varname
        .first()
        .is_some_and(|&b| ascii_isupper(i32::from(b)))
    {
        if varname[1..].iter().any(|&b| ascii_islower(i32::from(b))) {
            return VAR_FLAVOUR_SESSION;
        }
        return VAR_FLAVOUR_SHADA;
    }
    VAR_FLAVOUR_DEFAULT
}

pub fn var_set_global(name: &[u8], mut vartv: TypVal) {
    save_funccal();
    set_var(name, &mut vartv, false);
    restore_funccal();
}

/// Display script name where an item was last set.
pub fn last_set_msg(script_ctx: Sctx) {
    if script_ctx.sc_sid == 0 {
        return;
    }

    let p = get_scriptname(script_ctx, true);
    g_st().msg_ext_skip_verbose.set(true);

    verbose_enter();
    msg_puts(gettext(c"\n\tLast set from "));
    msg_puts(&p);
    if script_ctx.sc_lnum > 0 {
        msg_puts(gettext(LINE_MSG));
        msg_outnum(script_ctx.sc_lnum);
    } else if script_is_lua(script_ctx.sc_sid) {
        msg_puts(gettext(c" (run Nvim with -V1 for more details)"));
    }
    verbose_leave();
}

/// Perform a substitution on "str" with pattern "pat" and substitute "sub".
pub fn do_string_sub(
    str_cstr: &CStr,
    len: usize,
    pat: &CStr,
    sub_cstr: &CStr,
    expr: Option<&TypVal>,
    flags: &CStr,
) -> std::ffi::CString {
    let str_bytes = str_cstr.to_bytes();
    let len = len.min(str_bytes.len());
    let mut regmatch: Regmatch = Regmatch::new();
    let mut out: Option<Vec<u8>> = None;

    let save_cpo = opt_vars().p_cpo.take();

    regmatch.rm_ic = opt_vars().p_ic.get();
    regmatch.set_prog(vim_regcomp(pat, RE_MAGIC + RE_STRING));
    if !regmatch.regprog.is_null() {
        let mut tail_off: usize = 0;
        let do_all = flags.to_bytes().first() == Some(&b'g');
        let mut zero_width: Option<usize> = None;
        let str_opt = Some(str_cstr);

        while vim_regexec_nl(&mut regmatch, str_opt, tail_off as Colnr) {
            let start_off = regmatch.start_offset(0, str_cstr);
            let end_off = regmatch.end_offset(0, str_cstr);
            if start_off == end_off {
                if zero_width == Some(start_off) {
                    let i = crate::mbyte::utfc_slice2len(&str_bytes[tail_off..]);
                    if let Some(buf) = &mut out {
                        buf.extend_from_slice(&str_bytes[tail_off..(tail_off + i).min(len)]);
                    }
                    tail_off = (tail_off + i).min(len);
                    continue;
                }
                zero_width = Some(start_off);
            }

            let sublen = vim_regsub(&mut regmatch, sub_cstr, expr, &mut [], REGSUB_MAGIC);
            if sublen <= 0 {
                out = None;
                break;
            }
            let buf = out.get_or_insert_with(|| Vec::with_capacity(200));

            let i = start_off.saturating_sub(tail_off);
            if i > 0 {
                buf.extend_from_slice(&str_bytes[tail_off..tail_off + i]);
            }
            let cur_len = buf.len();
            buf.resize(cur_len + (sublen as usize), 0);
            vim_regsub(
                &mut regmatch,
                sub_cstr,
                expr,
                &mut buf[cur_len..cur_len + (sublen as usize)],
                REGSUB_COPY | REGSUB_MAGIC,
            );
            buf.truncate(cur_len + (sublen as usize) - 1);
            tail_off = end_off.min(len);
            if tail_off >= len {
                break;
            }
            if !do_all {
                break;
            }
        }

        if let Some(buf) = &mut out
            && tail_off < len
        {
            buf.extend_from_slice(&str_bytes[tail_off..len]);
        }

        regmatch.free_prog();
    }

    let ret = if let Some(buf) = &out {
        bytes_to_cstring(buf)
    } else {
        bytes_to_cstring(&str_bytes[..len])
    };
    drop(out);
    if opt_vars().p_cpo.is_null() {
        opt_vars().p_cpo.set(save_cpo);
    } else if opt_vars().p_cpo.is_empty() {
        let obj = Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(cstr_as_string(Some(&save_cpo.as_cstr()))),
        };
        set_option_value_give_err(K_OPT_CPOPTIONS, obj, 0);
    }

    ret
}

/// Common code for getting job callbacks for `jobstart`.
pub fn common_job_callbacks(
    vopts: &TvDict,
    on_stdout: &mut CallbackReader,
    on_stderr: &mut CallbackReader,
    on_exit: &mut Callback,
) -> bool {
    if tv_dict_get_callback(Some(vopts), c"on_stdout", 9, &on_stdout.cb)
        && tv_dict_get_callback(Some(vopts), c"on_stderr", 9, &on_stderr.cb)
        && tv_dict_get_callback(Some(vopts), c"on_exit", 7, on_exit)
    {
        on_stdout.buffered = tv_dict_get_number(Some(vopts), c"stdout_buffered") != 0;
        on_stderr.buffered = tv_dict_get_number(Some(vopts), c"stderr_buffered") != 0;
        if on_stdout.buffered && on_stdout.cb.cb_type() == K_CALLBACK_NONE_CONST {
            on_stdout.self_dict = vopts.id();
        }
        if on_stderr.buffered && on_stderr.cb.cb_type() == K_CALLBACK_NONE_CONST {
            on_stderr.self_dict = vopts.id();
        }
        vopts.dv_refcount.set(vopts.dv_refcount.get() + 1);
        return true;
    }

    *on_stdout = CallbackReader::none();
    *on_stderr = CallbackReader::none();
    callback_free(on_exit);
    false
}

pub fn find_job(id: u64, show_error: bool) -> Option<&'static mut Channel> {
    let data = find_channel(id);
    let is_stopped = if let Some(ref d) = data
        && d.streamtype == K_CHANNEL_STREAM_PROC
    {
        let hdr = &d.uv.proc_;
        hdr.status >= 0 || hdr.stopped_time != 0
    } else {
        true
    };
    if data
        .as_ref()
        .is_none_or(|d| d.streamtype != K_CHANNEL_STREAM_PROC)
        || is_stopped
    {
        if show_error {
            if data
                .as_ref()
                .is_some_and(|d| d.streamtype != K_CHANNEL_STREAM_PROC)
            {
                emsg(gettext(E_INVCHANJOB));
            } else {
                emsg(gettext(E_INVCHAN));
            }
        }
        return None;
    }
    data
}

pub fn script_host_eval(name: &std::ffi::CStr, argvars: &mut [TypVal], rettv: &mut TypVal) {
    if check_secure() {
        return;
    }

    if argvars[0].v_type != VarType::VarString {
        emsg(gettext(E_INVARG));
        return;
    }

    let args = tv_list_alloc(1);
    tv_list_append_string(&args, argvars[0].vval.v_string_cstr(), -1);
    *rettv = eval_call_provider(name, c"eval", &args, false);
}

pub fn eval_call_provider(
    provider: &CStr,
    method: &std::ffi::CStr,
    arguments: &TvList,
    discard: bool,
) -> TypVal {
    if !eval_has_provider(provider.to_bytes(), false) {
        semsg!(
            c"E319: No \"%s\" provider found. Run \":checkhealth vim.provider\"",
            provider,
        );
        return TypVal {
            v_type: VarType::VarNumber,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
    }

    let mut func = [0u8; 256];
    let name_len = format_to_buf!(&mut func, c"provider#%s#Call", provider);

    let new_scope = CallerScope {
        script_ctx: g_st().current_sctx.get(),
        es_entry: crate::runtime::estack_top_clone(),
        autocmd_fname: g_st().autocmd_fname.get().clone(),
        autocmd_match: g_st().autocmd_match.get().clone(),
        autocmd_fname_full: g_st().autocmd_fname_full.get(),
        autocmd_bufnr: g_st().autocmd_bufnr.get(),
        funccalp: get_current_funccal().into(),
    };
    let saved_provider_caller_scope = g_st().provider_caller_scope.replace(new_scope);
    save_funccal();
    {
        g_st()
            .provider_call_nesting
            .set(g_st().provider_call_nesting.get() + (1));
    }

    let mut argvars = [
        TypVal {
            v_type: VarType::VarString,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::from_cstr(method),
        },
        TypVal {
            v_type: VarType::VarList,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::List(crate::eval::typval::ListId::from(arguments)),
        },
        tv_initial_value(),
    ];
    let mut rettv = tv_initial_value();
    tv_list_ref(Some(arguments));

    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: crate::eval::typval::PartialId::NULL,
        fe_selfdict: crate::eval::typval::DictId::NULL,
        fe_basetv: None,
        fe_found_var: false,
    };
    call_func(
        crate::memory::bytes_as_cstr(&func),
        name_len,
        &mut rettv,
        2,
        &mut argvars,
        &mut funcexe,
    );

    tv_list_unref(Some(arguments));
    restore_funccal();
    {
        *g_st().provider_caller_scope.borrow_mut() = saved_provider_caller_scope;
        g_st()
            .provider_call_nesting
            .set(g_st().provider_call_nesting.get() - (1));
        debug_assert!(g_st().provider_call_nesting.get() >= 0);
    }

    if discard {
        tv_clear(&mut rettv);
    }

    rettv
}

/// Checks if provider for feature `feat` is enabled.
pub fn eval_has_provider(feat_bytes: &[u8], throw_if_fast: bool) -> bool {
    if !matches!(
        feat_bytes,
        b"clipboard"
            | b"python3"
            | b"python3_compiled"
            | b"python3_dynamic"
            | b"perl"
            | b"ruby"
            | b"node"
    ) {
        return false;
    }

    if throw_if_fast && !nlua_is_deferred_safe() {
        semsg!(E_FAST_API_DISABLED, c"Vimscript function",);
        return false;
    }

    let mut name = [0u8; 32];
    let prefix_len = feat_bytes
        .iter()
        .position(|&b| b == b'_')
        .unwrap_or(feat_bytes.len())
        .min(name.len() - 1);
    name[..prefix_len].copy_from_slice(&feat_bytes[..prefix_len]);
    name[prefix_len] = 0;

    let mut buf = [0u8; 256];
    let mut tv = tv_initial_value();
    let mut len = format_to_buf!(&mut buf, c"g:loaded_%s_provider", &name);
    if !eval_variable(
        crate::memory::bytes_as_cstr(&buf),
        len,
        Some(&mut tv),
        None,
        false,
        true,
    ) {
        len = format_to_buf!(&mut buf, c"provider#%s#bogus", &name);
        script_autoload(&buf[..len as usize], false);

        len = format_to_buf!(&mut buf, c"g:loaded_%s_provider", &name);
        if !eval_variable(
            crate::memory::bytes_as_cstr(&buf),
            len,
            Some(&mut tv),
            None,
            false,
            true,
        ) {
            format_to_buf!(&mut buf, c"provider#%s#Call", &name);
            if find_func(Some(crate::memory::bytes_as_cstr(&buf))).is_some()
                && opt_vars().p_lpl.get()
            {
                semsg!(
                    c"provider: %s: missing required variable g:loaded_%s_provider",
                    &name,
                    &name,
                );
            }
            return false;
        }
    }

    let mut ok = tv.v_type == VarType::VarNumber && tv.vval.v_number() == 2;

    if ok {
        format_to_buf!(&mut buf, c"provider#%s#Call", &name);
        if find_func(Some(crate::memory::bytes_as_cstr(&buf))).is_none() {
            semsg!(
                c"provider: %s: g:loaded_%s_provider=2 but %s is not defined",
                &name,
                &name,
                &buf,
            );
            ok = false;
        }
    }

    ok
}

/// Gets the current user-input in prompt buffer `buf`, or `None` if buffer is not a prompt buffer.
pub fn prompt_get_input(buf: &FileBuf) -> Option<std::ffi::CString> {
    if !bt_prompt(Some(buf)) {
        return None;
    }
    let lnum_start = buf.b_prompt_start.borrow().mark.lnum;
    let mark_col = buf.b_prompt_start.borrow().mark.col;
    let lnum_last = buf.b_ml.ml_line_count.get();

    let text_rc = crate::memline::ml_get_buf_slice(buf, lnum_start);
    let mut text = text_rc.as_slice();
    if mark_col >= 0 && (text.len() as i32) >= mark_col {
        text = &text[mark_col as usize..];
    }

    let mut bytes = text.to_vec();
    for i in (lnum_start + 1)..=lnum_last {
        bytes.push(b'\n');
        bytes.extend_from_slice(&crate::memline::ml_get_buf_slice(buf, i));
    }
    Some(std::ffi::CString::new(bytes).unwrap_or_default())
}

/// Compare "typ1" and "typ2". Put the result in "typ1".
pub fn typval_compare(typ1: &mut TypVal, typ2: &mut TypVal, expr_type: Exprtype, ic: bool) -> bool {
    let n1: Varnumber;
    let type_is = expr_type == EXPR_IS || expr_type == EXPR_ISNOT;

    if type_is && typ1.v_type != typ2.v_type {
        n1 = Varnumber::from(expr_type == EXPR_ISNOT);
    } else if typ1.v_type == VarType::VarBlob || typ2.v_type == VarType::VarBlob {
        if type_is {
            let mut eq = typ1.v_type == typ2.v_type && typ1.vval.v_blob() == typ2.vval.v_blob();
            if expr_type == EXPR_ISNOT {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        } else if typ1.v_type != typ2.v_type
            || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
        {
            if typ1.v_type != typ2.v_type {
                emsg(gettext(c"E977: Can only compare Blob with Blob"));
            } else {
                emsg(gettext(E_INVALBLOB));
            }
            tv_clear(typ1);
            return false;
        } else {
            let mut eq = tv_blob_equal(typ1.vval.v_blob_opt(), typ2.vval.v_blob_opt());
            if expr_type == EXPR_NEQUAL {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        }
    } else if typ1.v_type == VarType::VarList || typ2.v_type == VarType::VarList {
        if type_is {
            let mut eq = typ1.v_type == typ2.v_type && typ1.vval.v_list() == typ2.vval.v_list();
            if expr_type == EXPR_ISNOT {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        } else if typ1.v_type != typ2.v_type
            || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
        {
            if typ1.v_type != typ2.v_type {
                emsg(gettext(c"E691: Can only compare List with List"));
            } else {
                emsg(gettext(c"E692: Invalid operation for List"));
            }
            tv_clear(typ1);
            return false;
        } else {
            let mut eq = tv_list_equal(typ1.vval.v_list_opt(), typ2.vval.v_list_opt(), ic);
            if expr_type == EXPR_NEQUAL {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        }
    } else if typ1.v_type == VarType::VarDict || typ2.v_type == VarType::VarDict {
        if type_is {
            let mut eq = typ1.v_type == typ2.v_type && typ1.vval.v_dict() == typ2.vval.v_dict();
            if expr_type == EXPR_ISNOT {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        } else if typ1.v_type != typ2.v_type
            || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
        {
            if typ1.v_type != typ2.v_type {
                emsg(gettext(
                    c"E735: Can only compare Dictionary with Dictionary",
                ));
            } else {
                emsg(gettext(c"E736: Invalid operation for Dictionary"));
            }
            tv_clear(typ1);
            return false;
        } else {
            let mut eq = tv_dict_equal(typ1.vval.v_dict_opt(), typ2.vval.v_dict_opt(), ic);
            if expr_type == EXPR_NEQUAL {
                eq = !eq;
            }
            n1 = Varnumber::from(eq);
        }
    } else if tv_is_func(typ1) || tv_is_func(typ2) {
        if expr_type != EXPR_EQUAL
            && expr_type != EXPR_NEQUAL
            && expr_type != EXPR_IS
            && expr_type != EXPR_ISNOT
        {
            emsg(gettext(c"E694: Invalid operation for Funcrefs"));
            tv_clear(typ1);
            return false;
        }
        let pt1_ptr = typ1.vval.v_partial();
        let pt2_ptr = typ2.vval.v_partial();
        let mut eq = if (typ1.v_type == VarType::VarPartial && pt1_ptr.is_null())
            || (typ2.v_type == VarType::VarPartial && pt2_ptr.is_null())
        {
            pt1_ptr == pt2_ptr
        } else if type_is {
            if typ1.v_type == VarType::VarFunc && typ2.v_type == VarType::VarFunc {
                tv_equal(&*typ1, &*typ2, ic)
            } else if typ1.v_type == VarType::VarPartial && typ2.v_type == VarType::VarPartial {
                pt1_ptr == pt2_ptr
            } else {
                false
            }
        } else {
            tv_equal(&*typ1, &*typ2, ic)
        };
        if expr_type == EXPR_NEQUAL || expr_type == EXPR_ISNOT {
            eq = !eq;
        }
        n1 = Varnumber::from(eq);
    } else if (typ1.v_type == VarType::VarFloat || typ2.v_type == VarType::VarFloat)
        && expr_type != EXPR_MATCH
        && expr_type != EXPR_NOMATCH
    {
        let f1 = tv_get_float(&*typ1);
        let f2 = tv_get_float(&*typ2);
        let cmp = match expr_type {
            EXPR_IS | EXPR_EQUAL => f1 == f2,
            EXPR_ISNOT | EXPR_NEQUAL => f1 != f2,
            EXPR_GREATER => f1 > f2,
            EXPR_GEQUAL => f1 >= f2,
            EXPR_SMALLER => f1 < f2,
            EXPR_SEQUAL => f1 <= f2,
            _ => false,
        };
        n1 = Varnumber::from(cmp);
    } else if (typ1.v_type == VarType::VarNumber || typ2.v_type == VarType::VarNumber)
        && expr_type != EXPR_MATCH
        && expr_type != EXPR_NOMATCH
    {
        let v1 = tv_get_number(&*typ1);
        let v2 = tv_get_number(&*typ2);
        let cmp = match expr_type {
            EXPR_IS | EXPR_EQUAL => v1 == v2,
            EXPR_ISNOT | EXPR_NEQUAL => v1 != v2,
            EXPR_GREATER => v1 > v2,
            EXPR_GEQUAL => v1 >= v2,
            EXPR_SMALLER => v1 < v2,
            EXPR_SEQUAL => v1 <= v2,
            _ => false,
        };
        n1 = Varnumber::from(cmp);
    } else {
        let mut buf1 = [0u8; NUMBUFLEN];
        let mut buf2 = [0u8; NUMBUFLEN];
        let s1 = tv_get_string_buf(&*typ1, &mut buf1);
        let s2 = tv_get_string_buf(&*typ2, &mut buf2);
        let i = if expr_type != EXPR_MATCH && expr_type != EXPR_NOMATCH {
            mb_strcmp_ic(ic, s1, s2)
        } else {
            0
        };
        let cmp = match expr_type {
            EXPR_IS | EXPR_EQUAL => i == 0,
            EXPR_ISNOT | EXPR_NEQUAL => i != 0,
            EXPR_GREATER => i > 0,
            EXPR_GEQUAL => i >= 0,
            EXPR_SMALLER => i < 0,
            EXPR_SEQUAL => i <= 0,
            EXPR_MATCH => pattern_match(s2, Some(s1), ic),
            EXPR_NOMATCH => !pattern_match(s2, Some(s1), ic),
            _ => false,
        };
        n1 = Varnumber::from(cmp);
    }
    tv_clear(typ1);
    typ1.v_type = VarType::VarNumber;
    typ1.vval = crate::eval::typval::TypvalVval::Number(n1);
    true
}

/// Convert any type to a string, never give an error.
pub fn typval_tostring(arg: Option<&mut TypVal>, quotes: bool) -> std::ffi::CString {
    let Some(arg) = arg else {
        return c"(does not exist)".to_owned();
    };
    if !quotes && arg.v_type == VarType::VarString {
        return arg.vval.v_string_owned();
    }
    encode_tv2string(&mut *arg, None)
}

impl Lval {
    #[inline]
    pub fn tv_mut(&self) -> crate::eval::vars::LvalTvMut<'_> {
        self.ll_tv.borrow_mut()
    }

    #[inline]
    pub fn list_opt(&self) -> Option<&TvList> {
        self.ll_list.opt()
    }

    #[inline]
    pub fn dict_opt(&self) -> Option<&TvDict> {
        self.ll_dict.opt()
    }

    #[inline]
    pub fn li(&self) -> &Listitem {
        &self.ll_li
    }

    #[inline]
    pub fn di(&self) -> &Dictitem {
        &self.ll_di
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_num_divide_and_modulus() {
        assert_eq!(num_divide(10, 2), 5);
        assert_eq!(num_divide(0, 0), VARNUMBER_MIN);
        assert_eq!(num_divide(5, 0), VARNUMBER_MAX);
        assert_eq!(num_divide(-5, 0), -VARNUMBER_MAX);
        assert_eq!(num_divide(VARNUMBER_MIN, -1), VARNUMBER_MAX);

        assert_eq!(num_modulus(10, 3), 1);
        assert_eq!(num_modulus(10, 0), 0);
        assert_eq!(num_modulus(VARNUMBER_MIN, -1), 0);
    }

    #[test]
    fn test_eval_isnamec_helpers() {
        assert!(eval_isnamec1(b'a' as i32));
        assert!(eval_isnamec1(b'_' as i32));
        assert!(!eval_isnamec1(b'1' as i32));
        assert!(!eval_isnamec1(b':' as i32));

        assert!(eval_isnamec(b'a' as i32));
        assert!(eval_isnamec(b'9' as i32));
        assert!(eval_isnamec(b':' as i32));
        assert!(eval_isnamec(b'#' as i32));
        assert!(!eval_isnamec(b'-' as i32));

        assert!(eval_isdictc(b'a' as i32));
        assert!(eval_isdictc(b'9' as i32));
        assert!(eval_isdictc(b'_' as i32));
        assert!(!eval_isdictc(b':' as i32));
    }

    #[test]
    fn test_var_flavour_and_luafunc_name() {
        assert_eq!(var_flavour(b"foo"), VAR_FLAVOUR_DEFAULT);
        assert_eq!(var_flavour(b"Foo"), VAR_FLAVOUR_SESSION);
        assert_eq!(var_flavour(b"FOO"), VAR_FLAVOUR_SHADA);

        assert_eq!(check_luafunc_name(c"my_mod.fn(", true), 9);
        assert_eq!(check_luafunc_name(c"my_mod.fn", false), 9);
        assert_eq!(check_luafunc_name(c"my_mod.fn(", false), 0);
    }

    #[test]
    fn test_string2float() {
        let mut val: TvFloat = 0.0;
        assert_eq!(string2float(c"1.5", &mut val), 3);
        assert_eq!(val, 1.5);
        assert_eq!(string2float(c"inf", &mut val), 3);
        assert!(val.is_infinite() && val.is_sign_positive());
        assert_eq!(string2float(c"-inf", &mut val), 4);
        assert!(val.is_infinite() && val.is_sign_negative());
        assert_eq!(string2float(c"nan", &mut val), 3);
        assert!(val.is_nan());
    }

    #[test]
    fn test_null_typval_from_unexistent_env_and_null_vimvars() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::main_init::ensure_test_editor_init();

        let mut rettv = tv_initial_value();
        let mut evalarg = Evalarg {
            eval_flags: EVAL_EVALUATE,
        };
        assert!(eval0(
            c"$XXX_UNEXISTENT_VAR_XXX",
            &mut rettv,
            None,
            &mut evalarg
        ));
        assert_eq!(rettv.v_type, VarType::VarString);
        assert!(rettv.vval.v_string_is_null());
        tv_clear(&mut rettv);

        assert!(eval0(c"v:_null_list", &mut rettv, None, &mut evalarg));
        assert_eq!(rettv.v_type, VarType::VarList);
        assert!(rettv.vval.v_list().is_null());
        tv_clear(&mut rettv);

        assert!(eval0(c"v:_null_dict", &mut rettv, None, &mut evalarg));
        assert_eq!(rettv.v_type, VarType::VarDict);
        assert!(rettv.vval.v_dict().is_null());
        tv_clear(&mut rettv);
    }
}
