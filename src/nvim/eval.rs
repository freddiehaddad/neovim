//! Expression evaluation (`src/nvim/eval.c`).

use crate::api::private::helpers::{
    HLF_E, api_clear_error, arena_array, cstr_as_string, cstr_to_string, tv_dict_is_watched,
};
use crate::charset::{
    NUL, VARNUMBER_MIN, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_islower, ascii_isupper,
    ascii_iswhite, ascii_iswhite_or_nul, ascii_isxdigit, hex2nr, skipdigits, skiptowhite,
    skipwhite, vim_str2nr,
};
use crate::eval::typval::{
    Callback, Dictitem, EvalFuncData, K_CALLBACK_EXPR, K_CALLBACK_FUNCREF, K_CALLBACK_LUA,
    K_CALLBACK_NONE_CONST, K_CALLBACK_PARTIAL, Listitem, Listwatch, TV_CSTRING, TV_EMPTY_STRING,
    TvBlob, TvDict, TvFloat, TvList, TvPartial, TypVal, TypvalVval, VAR_UNLOCKED, VarType,
    Varnumber, callback_copy, callback_free, callback_put, tv_blob_alloc, tv_blob_alloc_ret,
    tv_blob_check_index, tv_blob_check_range, tv_blob_copy, tv_blob_equal, tv_blob_len,
    tv_blob_set_append, tv_blob_set_range, tv_blob_slice_or_index, tv_check_lock, tv_check_num,
    tv_check_str, tv_clear, tv_copy, tv_dict_add, tv_dict_add_nr, tv_dict_alloc, tv_dict_copy,
    tv_dict_equal, tv_dict_find, tv_dict_free, tv_dict_free_contents, tv_dict_get_callback,
    tv_dict_get_number, tv_dict_hi2di, tv_dict_item_alloc, tv_dict_item_free, tv_dict_unref,
    tv_dict_watcher_notify, tv_dict_wrong_func_name, tv_equal, tv_get_float, tv_get_number,
    tv_get_number_chk, tv_get_string, tv_get_string_buf, tv_get_string_buf_chk, tv_get_string_chk,
    tv_initial_value, tv_is_func, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_owned_tv, tv_list_append_string, tv_list_assign_range,
    tv_list_check_range_index_one, tv_list_check_range_index_two, tv_list_concat, tv_list_copy,
    tv_list_equal, tv_list_find, tv_list_find_nr, tv_list_first, tv_list_free,
    tv_list_free_contents, tv_list_join, tv_list_last, tv_list_len, tv_list_ref, tv_list_set_lock,
    tv_list_set_ret, tv_list_slice_or_index, tv_list_unref, tv_list_watch_add,
    tv_list_watch_remove, tv_st, tv2bool, value_check_lock,
};
use crate::eval::vars::{
    Evalarg, Lval, VV_ARGV, VV_EVENT, VV_LUA, VV_SHELL_ERROR, check_vars, eval_one_expr_in_str,
    eval_variable, evalvars_clear, evalvars_init, ex_let_vars, find_var, garbage_collect_globvars,
    garbage_collect_scriptvars, garbage_collect_vimvars, get_vim_var_dict, get_vim_var_partial,
    get_vimvar_dict, opt_to_tv, set_var, set_var_const, set_vim_var_list, set_vim_var_nr,
    skip_var_list, tv_init, valid_varname, var_check_lock, var_check_ro, var_wrong_func_name,
};
use crate::event::r#loop::loop_queues;
use crate::ex_docmd::{
    Exarg, check_nextcmd, cmd_has_expr_args, do_cmdline, ends_excmd, set_ref_in_findfunc,
};
use crate::strings::{concat_str, gettext, strtod, vim_snprintf, vim_strchr};
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

use crate::buffer::{FileBuf, K_UIMESSAGES, NOTDONE, VAR_FIXED, bt_prompt, buflist_findnr};
use crate::channel::{CallbackReader, Channel, K_CHANNEL_STREAM_PROC};
use crate::errors::*;
use crate::eval::funcs::{E_STRING_LIST_OR_BLOB_REQUIRED, K_OPT_CPOPTIONS};
use crate::eval::userfunc::{
    Funcexe, UserFunc, call_func, call_simple_func, call_simple_luafunc, deref_func_name,
    eval_fname_script, find_func, free_all_functions, free_unref_funccal, func_ref,
    get_current_funccal, get_func_tv, get_funccal_args_ht, get_lambda_tv, get_scriptlocal_funcname,
    make_partial, restore_funccal, save_funccal, set_ref_in_call_stack, set_ref_in_func,
    set_ref_in_func_args, set_ref_in_functions, set_ref_in_previous_funccal,
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
use crate::r#move::{check_cursor_moved, update_topline, validate_botline_win};
use crate::option::{
    K_OPT_FOLDEXPR, K_OPT_FOLDTEXT, K_OPT_INVALID, OPT_GLOBAL, OPT_LOCAL, find_option_end,
    get_option_value, get_tty_option, is_option_hidden, is_tty_option, optval_free,
    set_option_value_give_err, was_set_insecurely,
};
use crate::option_vars::*;
use crate::quickfix::set_ref_in_quickfix;
use crate::regexp::{
    NUMBUFLEN, RE_MAGIC, RE_STRING, REGSUB_COPY, REGSUB_MAGIC, Regmatch, vim_regcomp,
    vim_regexec_nl, vim_regfree, vim_regsub,
};
use crate::register::{K_GREG_EXPR_SRC, get_reg_contents_str};
use crate::runtime::{
    Sctx, free_autoload_scriptnames, free_scriptnames, get_scriptname, rt_st, script_autoload,
    script_is_lua,
};
use crate::ui::ui_has;
use std::ffi::CStr;
use std::ptr;

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::validate::{Error, ErrorType, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, PROF_YES};
use crate::cmdexpand::{
    Cmdidx, EXPAND_COMMANDS, EXPAND_ENV_VARS, EXPAND_EXPRESSION, EXPAND_FUNCTIONS, EXPAND_NOTHING,
    EXPAND_SETTINGS, EXPAND_USER_VARS,
};
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::eval::encode::{encode_list_write, encode_tv2echo, encode_tv2string};
use crate::eval::executor::eexe_mod_op;
use crate::eval::gc::{gc_dicts_slice, gc_lists_slice};
use crate::event::multiqueue::{multiqueue_free, multiqueue_new_child};
use crate::event::time::{
    TimeWatcher, TimeWatcherData, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop,
};
use crate::ex_eval::{aborting, discard_current_exception};
use crate::hashtab::{Hashtab, hashitem_empty};
use crate::highlight_group::syn_name2id;
use crate::insexpand::{set_ref_in_cpt_callbacks, set_ref_in_insexpand_funcs};
use crate::keycodes::{
    FSK_IN_STRING, FSK_KEYCODE, FSK_SIMPLIFY, K_SPECIAL, KE_SNR, KS_EXTRA, find_special_key,
    trans_special,
};
use crate::mark::mark_get;
use crate::math::{FAIL, OK};
use crate::mbyte::{
    Vimconv, mb_charlen, mb_copy_char, mb_strcmp_ic, string_convert, utf_char2bytes, utf_head_off,
    utfc_ptr2len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{Arena, arena_finish, arena_mem_free, cstr_from_bytes, cstr_to_cstring};
use crate::msgpack_rpc::channel::find_channel;
use crate::ops::set_ref_in_opfunc;
use crate::optionstr::{OptIndex, free_string_option};
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
    pub(crate) eval_lavars_used: *mut bool,
    echo_hl_id: i32,
    last_timer_id: u64,
    callback_depth: i32,
    current_copy_id: i32,
    eval7_recurse: i32,
    var_item_copy_recurse: i32,
    var2_fpos_pos: CursorPos,
    timers: std::collections::BTreeMap<u64, *mut VimTimer>,
}

impl EvalModuleState {
    const fn new() -> Self {
        Self {
            eval_lavars_used: ptr::null_mut(),
            echo_hl_id: 0,
            last_timer_id: 1,
            callback_depth: 0,
            current_copy_id: 0,
            eval7_recurse: 0,
            var_item_copy_recurse: 0,
            var2_fpos_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            timers: std::collections::BTreeMap::new(),
        }
    }
}

static EVAL_STATE: GlobalState<EvalModuleState> = GlobalState::new(EvalModuleState::new());

#[inline]
pub(crate) fn eval_st() -> &'static mut EvalModuleState {
    unsafe { &mut *EVAL_STATE.as_ptr() }
}

enum ForSource {
    None,
    List {
        list: *mut TvList,
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

pub type HtStack = Vec<*mut Hashtab>;
pub type ListStack = Vec<*mut TvList>;

#[repr(C)]
pub struct VimTimer {
    pub watcher: TimeWatcher,
    pub timer_id: i32,
    pub repeat_count: i32,
    pub refcount: i32,
    pub emsg_count: i32,
    pub timeout: i64,
    pub stopped: bool,
    pub paused: bool,
    pub callback: Callback,
}

impl Drop for VimTimer {
    fn drop(&mut self) {
        if !self.watcher.events.is_null() {
            multiqueue_free(self.watcher.events);
        }
        callback_free(&mut self.callback);
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct CallerScope {
    pub script_ctx: Sctx,
    pub es_entry: crate::profile::Estack,
    pub autocmd_fname: Option<std::ffi::CString>,
    pub autocmd_match: Option<std::ffi::CString>,
    pub autocmd_fname_full: bool,
    pub autocmd_bufnr: i32,
    pub funccalp: *mut crate::profile::Funccall,
}

#[inline]
fn tv_partial(tv: &TypVal) -> *mut TvPartial {
    tv.vval.v_partial()
}

#[inline]
fn tv_set_partial(tv: &mut TypVal, pt: *mut TvPartial) {
    tv.vval = crate::eval::typval::TypvalVval::Partial(pt);
}

#[inline]
pub fn tv_blob_set_ret(tv: &mut TypVal, b: *mut TvBlob) {
    tv.v_type = VarType::VarBlob;
    tv.vval = crate::eval::typval::TypvalVval::Blob(b);
    if !b.is_null() {
        unsafe {
            (*b).bv_refcount += 1;
        }
    }
}

#[inline]
pub fn tv_dict_set_ret(tv: &mut TypVal, d: *mut TvDict) {
    tv.v_type = VarType::VarDict;
    tv.vval = crate::eval::typval::TypvalVval::Dict(d);
    if !d.is_null() {
        unsafe {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
fn tv_list_copyid(l: &TvList) -> i32 {
    l.lv_copy_id
}

#[inline]
fn tv_list_latest_copy(l: &TvList) -> *mut TvList {
    l.lv_copylist
}

#[inline]
fn tv_list_has_watchers(l: &TvList) -> bool {
    !l.lv_watch.is_empty()
}

#[inline]
fn tv_list_item_tv(li: &mut Listitem) -> &mut TypVal {
    &mut li.li_tv
}

#[inline]
fn tv_list_item_next(li: &Listitem) -> *mut Listitem {
    li.li_next
}

pub fn get_v_event(sve: &mut SaveVEvent) -> &'static mut TvDict {
    let v_event = unsafe { &mut *get_vim_var_dict(VV_EVENT) };

    if v_event.dv_hashtab.ht_used > 0 {
        sve.sve_did_save = true;
        sve.sve_hashtab = std::mem::take(&mut v_event.dv_hashtab);
    } else {
        sve.sve_did_save = false;
    }
    v_event
}

pub fn restore_v_event(v_event: &mut TvDict, sve: &mut SaveVEvent) {
    tv_dict_free_contents(&mut *v_event);
    if sve.sve_did_save {
        v_event.dv_hashtab = std::mem::take(&mut sve.sve_hashtab);
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
    arg: impl crate::strings::IntoCCharPtr,
    error: &mut bool,
    eap: &mut Exarg,
    skip: bool,
    use_simple_function: bool,
) -> bool {
    let arg = arg.into_c_char_ptr();
    let mut tv = tv_initial_value();
    let mut retval = false;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, skip);

    if skip {
        g_st().emsg_skip += 1;
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
        g_st().emsg_skip -= 1;
    }

    retval
}

/// Call eval1() and give an error message if not done at a lower level.
fn eval1_emsg<T>(arg: *mut *mut T, rettv: &mut TypVal, eap: Option<&mut Exarg>) -> bool {
    let start = unsafe { *arg };
    let did_emsg_before = g_st().did_emsg;
    let called_emsg_before = g_st().called_emsg;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.is_some() && eap.unwrap().skip != 0);

    let ret = eval1(arg, &mut *rettv, Some(&mut evalarg));
    if !ret
        && !aborting()
        && g_st().did_emsg == did_emsg_before
        && g_st().called_emsg == called_emsg_before
    {
        semsg!(gettext(E_INVEXPR2), start.cast::<std::ffi::c_void>());
    }
    ret
}

/// @return  whether a typval is a valid expression to pass to eval_expr_typval()
///          or eval_expr_to_bool(). An empty string returns false.
pub fn eval_expr_valid_arg(tv: &TypVal) -> bool {
    tv.v_type != VarType::VarUnknown
        && (tv.v_type != VarType::VarString
            || (!tv.vval.v_string().is_null() && !crate::memory::cstr_is_empty(tv.vval.v_string())))
}

/// Evaluate a partial.
fn eval_expr_partial(expr: &TypVal, argv: *mut TypVal, argc: i32, rettv: &mut TypVal) -> bool {
    let partial = tv_partial(expr);
    if partial.is_null() {
        return false;
    }

    let s = partial_name(unsafe { partial.as_ref() });
    if crate::memory::cstr_is_empty(s) {
        return false;
    }

    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: 0,
        fe_lastline: 0,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: partial.cast(),
        fe_selfdict: ptr::null_mut(),
        fe_basetv: ptr::null_mut(),
        fe_found_var: false,
    };
    if !call_func(s, -1, rettv, argc, argv, &mut funcexe) {
        return false;
    }

    true
}

/// Evaluate an expression which is a function.
fn eval_expr_func(expr: &TypVal, argv: *mut TypVal, argc: i32, rettv: &mut TypVal) -> bool {
    let mut buf = [0u8; NUMBUFLEN];
    let s = if expr.v_type == VarType::VarFunc {
        expr.vval.v_string().cast_const()
    } else {
        tv_get_string_buf_chk(expr, &mut buf).map_or(ptr::null(), std::ffi::CStr::as_ptr)
    };
    if crate::memory::cstr_is_empty(s) {
        return false;
    }

    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: 0,
        fe_lastline: 0,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: ptr::null_mut(),
        fe_selfdict: ptr::null_mut(),
        fe_basetv: ptr::null_mut(),
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
    let mut s = skipwhite(s_cstr.as_ptr().cast_mut());
    if !eval1_emsg(&raw mut s, rettv, None) {
        return false;
    }

    if !crate::memory::cstr_is_empty(skipwhite(s)) {
        tv_clear(rettv);
        semsg!(gettext(E_INVEXPR2), s);
        return false;
    }

    true
}

/// Evaluate an expression, which can be a function, partial or string.
pub fn eval_expr_typval(
    expr: &TypVal,
    want_func: bool,
    argv: *mut TypVal,
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
    let mut argv = tv_initial_value();
    let mut rettv = tv_initial_value();

    if !eval_expr_typval(expr, false, &raw mut argv, 0, &mut rettv) {
        *error = true;
        return false;
    }
    let res = tv_get_number_chk(&rettv, Some(&mut *error)) != 0;
    tv_clear(&mut rettv);
    res
}

/// Top level evaluation function, returning a string.
pub fn eval_to_string_skip(
    arg: impl crate::strings::IntoCCharPtr,
    eap: &mut Exarg,
    skip: bool,
) -> Option<std::ffi::CString> {
    let arg = arg.into_c_char_ptr();
    let mut tv = tv_initial_value();
    let retval: Option<std::ffi::CString>;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, skip);
    if skip {
        g_st().emsg_skip += 1;
    }
    if !eval0(arg, &mut tv, Some(&mut *eap), &mut evalarg) || skip {
        retval = None;
    } else {
        retval = Some(tv_get_string(&tv).to_owned());
        tv_clear(&mut tv);
    }
    if skip {
        g_st().emsg_skip -= 1;
    }

    retval
}

/// Skip over an expression at "*pp".
pub fn skip_expr<T>(pp: *mut *mut T, mut evalarg: Option<&mut Evalarg>) -> bool {
    let save_flags = if evalarg.is_none() {
        0
    } else {
        evalarg.as_deref_mut().unwrap().eval_flags
    };

    if evalarg.is_some() {
        evalarg.as_deref_mut().unwrap().eval_flags &= !EVAL_EVALUATE;
    }

    unsafe {
        *pp = skipwhite((*pp).cast()).cast();
    }
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
        if !tv.vval.v_list().is_null() {
            tv_list_join(&mut out, unsafe { tv.vval.v_list().as_mut() }, c"\n");
            if tv_list_len(unsafe { tv.vval.v_list().as_ref() }) > 0 {
                out.push(b'\n');
            }
        }
        return Some(cstr_from_bytes(&out));
    } else if tv.v_type == VarType::VarList || tv.v_type == VarType::VarDict {
        return Some(encode_tv2string(&mut *tv, None));
    }
    Some(tv_get_string(&*tv).to_owned())
}

/// Top level evaluation function, returning a string.
pub fn eval_to_string_eap(
    arg: impl crate::strings::IntoCCharPtr,
    join_list: bool,
    eap: Option<&mut Exarg>,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    let arg = arg.into_c_char_ptr();
    {
        let mut tv = tv_initial_value();
        let retval: Option<std::ffi::CString>;
        let mut evalarg = Evalarg::new();

        fill_evalarg_from_eap(&mut evalarg, eap.is_some() && eap.unwrap().skip != 0);
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
}

pub fn eval_to_string(
    arg: impl crate::strings::IntoCCharPtr,
    join_list: bool,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    let arg = arg.into_c_char_ptr().cast_mut();
    eval_to_string_eap(arg, join_list, None, use_simple_function)
}

/// Call eval_to_string() without using current local variables and using textlock.
pub fn eval_to_string_safe(
    arg: impl crate::strings::IntoCCharPtr,
    use_sandbox: bool,
    use_simple_function: bool,
) -> Option<std::ffi::CString> {
    let arg = arg.into_c_char_ptr();
    save_funccal();
    {
        if use_sandbox {
            g_st().sandbox += 1;
        }
        g_st().textlock += 1;
    }
    let retval = eval_to_string(arg, false, use_simple_function);
    {
        if use_sandbox {
            g_st().sandbox -= 1;
        }
        g_st().textlock -= 1;
    }
    restore_funccal();
    retval
}

/// Top level evaluation function, returning a number.
pub fn eval_to_number(
    expr: impl crate::strings::IntoCCharPtr,
    use_simple_function: bool,
) -> Varnumber {
    let expr = expr.into_c_char_ptr();
    let mut rettv = tv_initial_value();
    let retval: Varnumber;
    let mut p = skipwhite(expr);
    let mut r = NOTDONE;

    {
        g_st().emsg_off += 1;
    }

    if use_simple_function {
        r = may_call_simple_func(expr, &mut rettv);
    }
    if r == NOTDONE {
        r = if eval1(&raw mut p, &mut rettv, Some(&mut g_st().evalarg_evaluate)) {
            OK
        } else {
            FAIL
        };
    }
    if r == FAIL {
        retval = -1;
    } else {
        retval = tv_get_number_chk(&rettv, None);
        tv_clear(&mut rettv);
    }
    {
        g_st().emsg_off -= 1;
    }

    retval
}

/// Top level evaluation function.
pub fn eval_expr(
    arg: impl crate::strings::IntoCCharPtr,
    eap: Option<&mut Exarg>,
) -> Option<TypVal> {
    let arg = arg.into_c_char_ptr();
    eval_expr_ext(arg, eap, false)
}

pub fn eval_expr_ext(
    arg: impl crate::strings::IntoCCharPtr,
    mut eap: Option<&mut Exarg>,
    use_simple_function: bool,
) -> Option<TypVal> {
    let arg = arg.into_c_char_ptr();
    {
        let mut tv = tv_initial_value();
        let mut evalarg = Evalarg::new();

        fill_evalarg_from_eap(
            &mut evalarg,
            eap.is_some() && eap.as_deref_mut().unwrap().skip != 0,
        );

        let ok = if use_simple_function {
            eval0_simple_funccal(arg, &mut tv, eap, &mut evalarg)
        } else {
            eval0(arg, &mut tv, eap, &mut evalarg)
        };
        ok.then_some(tv)
    }
}

/// Call some Vim script function and return the result in "*rettv".
pub fn call_vim_function(
    func: impl crate::strings::IntoCCharPtr,
    argc: i32,
    argv: *mut TypVal,
    rettv: &mut TypVal,
) -> bool {
    let mut func = func.into_c_char_ptr();
    {
        let mut len = crate::memory::cstr_len(func) as i32;
        let mut pt: *mut TvPartial = ptr::null_mut();

        if len >= 6 && crate::memory::cstr_slice(func, 6) == b"v:lua." {
            func = func.wrapping_add(6);
            len = check_luafunc_name(func, false);
            if len == 0 {
                tv_clear(&mut *rettv);
                return false;
            }
            pt = get_vim_var_partial(VV_LUA);
        }

        rettv.v_type = VarType::VarUnknown;
        let lnum = curwin_mut().w_cursor.lnum;
        let mut funcexe = Funcexe {
            fe_fill_submatch: false,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: false,
            fe_evaluate: true,
            fe_partial: pt.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        let ret = call_func(func, len, &mut *rettv, argc, argv, &mut funcexe);

        if !ret {
            tv_clear(&mut *rettv);
        }

        ret
    }
}

/// Call Vim script function and return the result as a string.
pub fn call_func_retstr(
    func: impl crate::strings::IntoCCharPtr,
    argc: i32,
    argv: *mut TypVal,
) -> Option<std::ffi::CString> {
    let func = func.into_c_char_ptr();
    let mut rettv = tv_initial_value();
    if !call_vim_function(func, argc, argv, &mut rettv) {
        return None;
    }

    let retval = tv_get_string(&rettv).to_owned();
    tv_clear(&mut rettv);
    Some(retval)
}

/// Call Vim script function and return the result as a List.
pub fn call_func_retlist(
    func: impl crate::strings::IntoCCharPtr,
    argc: i32,
    argv: *mut TypVal,
) -> *mut TvList {
    let func = func.into_c_char_ptr();
    let mut rettv = tv_initial_value();

    if !call_vim_function(func, argc, argv, &mut rettv) {
        return ptr::null_mut();
    }

    if rettv.v_type != VarType::VarList {
        tv_clear(&mut rettv);
        return ptr::null_mut();
    }

    rettv.vval.v_list()
}

/// Evaluate a number-valued "expr" option ('indentexpr'/'formatexpr') via its callback.
pub fn eval_expr_option_number(cb: &mut Callback, use_sandbox: bool) -> i32 {
    let mut copy = Callback::NONE;
    callback_copy(&mut copy, cb);
    if use_sandbox {
        g_st().sandbox += 1;
    }
    g_st().emsg_off += 1;
    let mut tv = tv_initial_value();
    let mut argv = [tv_initial_value()];
    let mut retval = -1;
    if callback_call(&mut copy, 0, argv.as_mut_ptr(), &mut tv) {
        retval = tv_get_number_chk(&tv, None) as i32;
        tv_clear(&mut tv);
    }
    g_st().emsg_off -= 1;
    if use_sandbox {
        g_st().sandbox -= 1;
    }
    callback_free(&mut copy);
    retval
}

/// Evaluate a "expr" option ('foldtext'/'includeexpr') via its callback, under |textlock| and an
/// isolated funccal stack; evaluates in the |sandbox| when `use_sandbox`.
pub fn eval_expr_option_tv(cb: &mut Callback, use_sandbox: bool, ret_tv: &mut TypVal) -> bool {
    save_funccal();
    {
        if use_sandbox {
            g_st().sandbox += 1;
        }
        g_st().textlock += 1;
    }
    let mut argv = [tv_initial_value()];
    let ok = callback_call(cb, 0, argv.as_mut_ptr(), &mut *ret_tv);
    {
        g_st().textlock -= 1;
        if use_sandbox {
            g_st().sandbox -= 1;
        }
    }
    restore_funccal();
    ok
}

/// Evaluate 'foldexpr'. Returns the foldlevel, and any character preceding
/// it in "*cp". Doesn't give error messages.
pub fn eval_foldexpr(wp: &mut WinInfoInfo, cp: &mut i32) -> i32 {
    let saved_sctx = g_st().current_sctx;
    let use_sandbox = was_set_insecurely(Some(&mut *wp), K_OPT_FOLDEXPR, OPT_LOCAL) != 0;

    g_st().current_sctx = wp.w_onebuf_opt.wo_script_ctx[K_WIN_OPT_FOLDEXPR];

    g_st().emsg_off += 1;
    if use_sandbox {
        g_st().sandbox += 1;
    }
    g_st().textlock += 1;
    *cp = NUL as i32;

    let mut tv = tv_initial_value();
    let mut argv = [tv_initial_value()];
    let retval: Varnumber;
    if !callback_call(
        unsafe { &mut *(&raw mut wp.w_onebuf_opt.wo_fde).cast() },
        0,
        argv.as_mut_ptr(),
        &mut tv,
    ) {
        retval = 0;
    } else {
        if tv.v_type == VarType::VarNumber {
            retval = tv.vval.v_number();
        } else if tv.v_type != VarType::VarString || tv.vval.v_string().is_null() {
            retval = 0;
        } else {
            let mut s = tv.vval.v_string();
            if !crate::memory::cstr_is_empty(s)
                && !ascii_isdigit(crate::memory::cstr_first_char(s))
                && crate::memory::cstr_first_byte(s) != b'-'
            {
                *cp = i32::from(crate::memory::cstr_first_byte(s));
                s = s.wrapping_add(1);
            }
            retval = Varnumber::from(crate::charset::parse_i64_prefix(
                crate::memory::cstr_as_bytes(s),
            ));
        }
        tv_clear(&mut tv);
    }

    g_st().emsg_off -= 1;
    if use_sandbox {
        g_st().sandbox -= 1;
    }
    g_st().textlock -= 1;
    g_st().current_sctx = saved_sctx;

    retval as i32
}

/// Evaluate 'foldtext', returning an Array or a String (NULL_STRING on failure).
pub fn eval_foldtext(wp: &mut WinInfoInfo) -> Object {
    let use_sandbox = was_set_insecurely(Some(&mut *wp), K_OPT_FOLDTEXT, OPT_LOCAL) != 0;
    let mut tv = tv_initial_value();
    let retval: Object;
    if !eval_expr_option_tv(
        unsafe { &mut *(&raw mut wp.w_onebuf_opt.wo_fdt).cast() },
        use_sandbox,
        &mut tv,
    ) {
        retval = Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(NvimString {
                data: ptr::null_mut(),
                size: 0,
            }),
        };
    } else {
        if tv.v_type == VarType::VarList {
            retval = vim_to_object(&raw mut tv, None, false);
        } else {
            retval = Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_to_string(tv_get_string(&tv))),
            };
        }
        tv_clear(&mut tv);
    }
    retval
}

/// Find the end of a variable or function name. Unlike find_name_end() this
/// does not recognize magic braces.
fn to_name_end<T>(arg: *const T, use_namespace: bool) -> *const T {
    let arg: *const u8 = arg.cast();
    if !eval_isnamec1(i32::from(crate::memory::cstr_first_byte(arg.cast()))) {
        return arg.cast();
    }

    let mut p = arg.wrapping_add(1);
    while !crate::memory::cstr_is_empty(p.cast())
        && eval_isnamec(i32::from(crate::memory::cstr_first_byte(p.cast())))
    {
        if crate::memory::cstr_first_byte(p.cast()) == b':'
            && (p != arg.wrapping_add(1)
                || !use_namespace
                || !crate::strings::vim_haschr(
                    c"bgstvw",
                    i32::from(crate::memory::cstr_first_byte(arg.cast())),
                ))
        {
            break;
        }
        p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
    }
    p.cast()
}

fn get_lval_dict_item<T, U>(
    lp: &mut Lval,
    name: impl crate::strings::IntoCCharPtr,
    key: *mut T,
    len: i32,
    key_end: *mut *mut U,
    var1: &mut TypVal,
    flags: i32,
    unlet: bool,
    rettv: Option<&mut TypVal>,
) -> GlvStatus {
    let name = name.into_c_char_ptr();
    let mut key = key.cast();
    unsafe {
        let quiet = (flags & GLV_QUIET) != 0;
        let p = (*key_end).cast();

        if len == -1 {
            key = tv_get_string(&*var1).as_ptr().cast_mut();
        }
        lp.ll_list = ptr::null_mut();

        if lp.tv_mut().vval.v_dict().is_null() {
            lp.tv_mut().vval = crate::eval::typval::TypvalVval::Dict(tv_dict_alloc());
            (*lp.tv_mut().vval.v_dict()).dv_refcount += 1;
        }
        lp.ll_dict = lp.tv_mut().vval.v_dict();

        lp.ll_di = tv_dict_find(lp.ll_dict.as_ref(), key, len as isize);

        if let Some(rettv) = rettv.as_deref()
            && (*lp.ll_dict).dv_scope != 0
        {
            let prevval = if len != -1 {
                let pv = *key.wrapping_add(len as usize);
                *key.wrapping_add(len as usize) = NUL;
                pv
            } else {
                0
            };
            let wrong = ((*lp.ll_dict).dv_scope == VAR_DEF_SCOPE
                && tv_is_func(rettv)
                && var_wrong_func_name(key, lp.ll_di.is_null()))
                || !valid_varname(key);
            if len != -1 {
                *key.wrapping_add(len as usize) = prevval;
            }
            if wrong {
                return GLV_FAIL;
            }
        }

        if !lp.ll_di.is_null()
            && tv_is_luafunc(&mut (*lp.ll_di).di_tv)
            && len == -1
            && rettv.is_none()
        {
            semsg!(E_ILLVAR, c"v:['lua']");
            return GLV_FAIL;
        }

        if lp.ll_di.is_null() {
            if lp.ll_dict == get_vimvar_dict()
                || ptr::eq(&raw mut (*lp.ll_dict).dv_hashtab, get_funccal_args_ht())
            {
                semsg!(gettext(E_ILLVAR), name);
                return GLV_FAIL;
            }

            if crate::memory::cstr_first_byte(p) == b'['
                || crate::memory::cstr_first_byte(p) == b'.'
                || unlet
            {
                if !quiet {
                    semsg!(gettext(E_DICTKEY), key);
                }
                return GLV_FAIL;
            }
            if len == -1 {
                lp.ll_newkey = Some(crate::memory::cstr_to_cstring(key));
            } else {
                let slice = crate::memory::cstr_slice(key, len as usize);
                lp.ll_newkey = Some(std::ffi::CString::new(slice).unwrap());
            }
            *key_end = p.cast();
            return GLV_STOP;
        } else if (flags & GLV_READ_ONLY) == 0
            && (var_check_ro(
                i32::from((*lp.ll_di).di_flags),
                name,
                (p as usize).wrapping_sub(name as usize),
            ) || var_check_lock(
                i32::from((*lp.ll_di).di_flags),
                name,
                (p as usize).wrapping_sub(name as usize),
            ))
        {
            return GLV_FAIL;
        }

        lp.ll_tv = &raw mut (*lp.ll_di).di_tv;

        GLV_OK
    }
}

fn get_lval_blob(
    lp: &mut Lval,
    var1: &mut TypVal,
    var2: &mut TypVal,
    empty1: bool,
    quiet: bool,
) -> bool {
    let bloblen = tv_blob_len(unsafe { (lp.tv_mut().vval.v_blob()).as_ref() });

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

    lp.ll_blob = lp.tv_mut().vval.v_blob();
    lp.ll_tv = ptr::null_mut();

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

    lp.ll_dict = ptr::null_mut();
    lp.ll_list = lp.tv_mut().vval.v_list();
    lp.ll_li = tv_list_check_range_index_one(unsafe { lp.ll_list.as_mut() }, &mut lp.ll_n1, quiet);
    if lp.ll_li.is_null() {
        return false;
    }

    if lp.ll_range && !lp.ll_empty2 {
        lp.ll_n2 = tv_get_number(&*var2) as i32;
        if !tv_list_check_range_index_two(
            unsafe { lp.ll_list.as_mut() },
            &mut lp.ll_n1,
            lp.ll_li,
            &mut lp.ll_n2,
            quiet,
        ) {
            return false;
        }
    }

    lp.ll_tv = tv_list_item_tv(unsafe { &mut *lp.ll_li });

    true
}

fn get_lval_subscript<T>(
    lp: &mut Lval,
    p: *mut T,
    name: impl crate::strings::IntoCCharPtr,
    mut rettv: Option<&mut TypVal>,
    unlet: bool,
    flags: i32,
) -> *mut T {
    let mut p = p.cast();
    let name = name.into_c_char_ptr();
    {
        let quiet = (flags & GLV_QUIET) != 0;
        let mut var1 = tv_initial_value();
        let mut var2 = tv_initial_value();
        let mut empty1 = false;

        while crate::memory::cstr_first_byte(p) == b'['
            || (crate::memory::cstr_first_byte(p) == b'.'
                && crate::memory::cstr_byte_at(p, 1) != b'='
                && crate::memory::cstr_byte_at(p, 1) != b'.')
        {
            if crate::memory::cstr_first_byte(p) == b'.' && lp.tv_mut().v_type != VarType::VarDict {
                if !quiet {
                    semsg!(gettext(E_DOT_CAN_ONLY_BE_USED_ON_DICTIONARY_STR), name,);
                }
                return ptr::null_mut();
            }
            if lp.tv_mut().v_type != VarType::VarList
                && lp.tv_mut().v_type != VarType::VarDict
                && lp.tv_mut().v_type != VarType::VarBlob
            {
                if !quiet {
                    emsg(gettext(c"E689: Can only index a List, Dictionary or Blob"));
                }
                return ptr::null_mut();
            }

            if lp.tv_mut().v_type == VarType::VarList && lp.tv_mut().vval.v_list().is_null() {
                tv_list_alloc_ret(unsafe { &mut *lp.ll_tv }, K_LIST_LEN_UNKNOWN_CONST);
            } else if lp.tv_mut().v_type == VarType::VarBlob && lp.tv_mut().vval.v_blob().is_null()
            {
                tv_blob_alloc_ret(unsafe { &mut *lp.ll_tv });
            }

            if lp.ll_range {
                if !quiet {
                    emsg(gettext(c"E708: [:] must come last"));
                }
                tv_clear(&mut var1);
                tv_clear(&mut var2);
                return ptr::null_mut();
            }

            let mut len: i32 = -1;
            let mut key = ptr::null_mut();
            if crate::memory::cstr_first_byte(p) == b'.' {
                key = p.wrapping_add(1);
                len = 0;
                while ascii_isalnum(crate::memory::cstr_char_at(key, len as usize))
                    || crate::memory::cstr_byte_at(key, len as usize) == b'_'
                {
                    len += 1;
                }
                if len == 0 {
                    if !quiet {
                        emsg(gettext(c"E713: Cannot use empty key after ."));
                    }
                    return ptr::null_mut();
                }
                p = key.wrapping_add(len as usize);
            } else {
                p = skipwhite(p.wrapping_add(1));
                if crate::memory::cstr_first_byte(p) == b':' {
                    empty1 = true;
                } else {
                    empty1 = false;
                    let evalarg_ptr = &raw mut g_st().evalarg_evaluate;
                    if !eval1(
                        &raw mut p,
                        &mut var1,
                        (!evalarg_ptr.is_null()).then(|| unsafe { &mut *evalarg_ptr }),
                    ) {
                        tv_clear(&mut var1);
                        tv_clear(&mut var2);
                        return ptr::null_mut();
                    }
                    if !tv_check_str(&var1) {
                        tv_clear(&mut var1);
                        tv_clear(&mut var2);
                        return ptr::null_mut();
                    }
                    p = skipwhite(p);
                }

                if crate::memory::cstr_first_byte(p) == b':' {
                    if lp.tv_mut().v_type == VarType::VarDict {
                        if !quiet {
                            emsg(gettext(E_CANNOT_SLICE_DICTIONARY));
                        }
                        tv_clear(&mut var1);
                        tv_clear(&mut var2);
                        return ptr::null_mut();
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
                        return ptr::null_mut();
                    }
                    p = skipwhite(p.wrapping_add(1));
                    if crate::memory::cstr_first_byte(p) == b']' {
                        lp.ll_empty2 = true;
                    } else {
                        lp.ll_empty2 = false;
                        let evalarg_ptr = &raw mut g_st().evalarg_evaluate;
                        if !eval1(
                            &raw mut p,
                            &mut var2,
                            (!evalarg_ptr.is_null()).then(|| unsafe { &mut *evalarg_ptr }),
                        ) || !tv_check_str(&var2)
                        {
                            tv_clear(&mut var1);
                            tv_clear(&mut var2);
                            return ptr::null_mut();
                        }
                    }
                    lp.ll_range = true;
                } else {
                    lp.ll_range = false;
                }

                if crate::memory::cstr_first_byte(p) != b']' {
                    if !quiet {
                        emsg(gettext(E_MISSBRAC));
                    }
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return ptr::null_mut();
                }

                p = p.wrapping_add(1);
            }

            if lp.tv_mut().v_type == VarType::VarDict {
                let glv_status = get_lval_dict_item(
                    &mut *lp,
                    name,
                    key,
                    len,
                    &raw mut p,
                    &mut var1,
                    flags,
                    unlet,
                    (rettv.is_some()).then(|| rettv.as_deref_mut().unwrap()),
                );
                if glv_status == GLV_FAIL {
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return ptr::null_mut();
                }
                if glv_status == GLV_STOP {
                    break;
                }
            } else if lp.tv_mut().v_type == VarType::VarBlob {
                if !get_lval_blob(&mut *lp, &mut var1, &mut var2, empty1, quiet) {
                    tv_clear(&mut var1);
                    tv_clear(&mut var2);
                    return ptr::null_mut();
                }
                break;
            } else if !get_lval_list(&mut *lp, &mut var1, &mut var2, empty1, flags, quiet) {
                tv_clear(&mut var1);
                tv_clear(&mut var2);
                return ptr::null_mut();
            }

            tv_clear(&mut var1);
            tv_clear(&mut var2);
            var1.v_type = VarType::VarUnknown;
            var2.v_type = VarType::VarUnknown;
        }

        tv_clear(&mut var1);
        tv_clear(&mut var2);
        p.cast()
    }
}

/// Get an lvalue
pub fn get_lval<T>(
    name: *mut T,
    rettv: Option<&mut TypVal>,
    lp: &mut Lval,
    unlet: bool,
    skip: bool,
    flags: i32,
    fne_flags: i32,
) -> *mut T {
    let name: *const u8 = name.cast();
    unsafe {
        let quiet = (flags & GLV_QUIET) != 0;

        *lp = Lval::new();

        if skip {
            lp.ll_name = name.cast();
            return find_name_end(
                name,
                ptr::null_mut(),
                ptr::null_mut(),
                FNE_INCL_BR | fne_flags,
            )
            .cast_mut()
            .cast();
        }

        let mut expr_start: *const u8 = ptr::null();
        let mut expr_end: *const u8 = ptr::null();
        let mut p =
            find_name_end(name, &raw mut expr_start, &raw mut expr_end, fne_flags).cast_mut();
        if !expr_start.is_null() {
            if unlet
                && !ascii_iswhite(crate::memory::cstr_first_char(p.cast()))
                && ends_excmd(i32::from(crate::memory::cstr_first_byte(p.cast()))) == 0
                && crate::memory::cstr_first_byte(p.cast()) != b'['
                && crate::memory::cstr_first_byte(p.cast()) != b'.'
            {
                semsg!(gettext(E_TRAILING_ARG), p);
                return ptr::null_mut();
            }

            if let Some(cstr) = make_expanded_name(
                name.cast(),
                expr_start.cast_mut(),
                expr_end.cast_mut(),
                p.cast(),
            ) {
                lp.ll_name_len = cstr.as_bytes().len();
                lp.ll_exp_name = Some(cstr);
                lp.ll_name = lp.ll_exp_name.as_ref().unwrap().as_ptr().cast();
            } else {
                lp.ll_exp_name = None;
                lp.ll_name = ptr::null();
                if !aborting() && !quiet {
                    {
                        g_st().emsg_severe = true;
                        semsg!(gettext(E_INVARG2), name);
                    }
                    return ptr::null_mut();
                }
                lp.ll_name_len = 0;
            }
        } else {
            lp.ll_name = name.cast();
            lp.ll_name_len = (p as usize).wrapping_sub(lp.ll_name as usize);
        }

        if (crate::memory::cstr_first_byte(p.cast()) != b'['
            && crate::memory::cstr_first_byte(p.cast()) != b'.')
            || lp.ll_name.is_null()
        {
            return p.cast();
        }

        let mut ht: *mut Hashtab = ptr::null_mut();

        let v = find_var(
            lp.ll_name.cast(),
            lp.ll_name_len,
            if (flags & GLV_READ_ONLY) != 0 {
                ptr::null_mut()
            } else {
                &raw mut ht
            },
            flags & GLV_NO_AUTOLOAD,
        );
        if v.is_null() && !quiet {
            semsg!(
                gettext(c"E121: Undefined variable: %.*s"),
                lp.ll_name_len as i32,
                lp.ll_name,
            );
        }
        if v.is_null() {
            return ptr::null_mut();
        }

        lp.ll_tv = &raw mut (*v).di_tv;

        if tv_is_luafunc(&mut *lp.ll_tv) {
            return p.cast();
        }

        p = get_lval_subscript(
            &mut *lp,
            p,
            name.cast(),
            (rettv.is_some()).then(|| rettv.unwrap()),
            unlet,
            flags,
        );
        if p.is_null() {
            return ptr::null_mut();
        }

        lp.ll_name_len = (p as usize).wrapping_sub(lp.ll_name as usize);
        p.cast()
    }
}

/// Set a variable that was parsed by get_lval() to "rettv".
pub fn set_var_lval<T>(
    lp: &mut Lval,
    endp: *mut T,
    rettv: &mut TypVal,
    copy: bool,
    is_const: bool,
    op: impl crate::strings::IntoCCharPtr,
) {
    let endp: *mut _ = endp.cast();
    let op = op.into_c_char_ptr();
    unsafe {
        if lp.ll_tv.is_null() {
            let cc = *endp;
            *endp = NUL;
            if !lp.ll_blob.is_null() {
                if !op.is_null() && crate::memory::cstr_first_byte(op) != b'=' {
                    semsg!(gettext(E_LETWRONG), op);
                    *endp = cc;
                    return;
                }
                if value_check_lock((*lp.ll_blob).bv_lock, lp.ll_name.cast(), TV_CSTRING) {
                    *endp = cc;
                    return;
                }

                if lp.ll_range && rettv.v_type == VarType::VarBlob {
                    if lp.ll_empty2 {
                        lp.ll_n2 = tv_blob_len(lp.ll_blob.as_ref()) - 1;
                    }

                    if !tv_blob_set_range(
                        &mut *lp.ll_blob,
                        Varnumber::from(lp.ll_n1),
                        Varnumber::from(lp.ll_n2),
                        rettv,
                    ) {
                        *endp = cc;
                        return;
                    }
                } else {
                    let mut error = false;
                    let val = tv_get_number_chk(&*rettv, Some(&mut error));
                    if !error {
                        if !(0..=255).contains(&val) {
                            semsg!(gettext(E_INVALID_VALUE_FOR_BLOB_NR), val,);
                        } else {
                            tv_blob_set_append(&mut *lp.ll_blob, lp.ll_n1, val as u8);
                        }
                    }
                }
            } else if !op.is_null() && crate::memory::cstr_first_byte(op) != b'=' {
                let mut tv = tv_initial_value();

                if is_const {
                    emsg(gettext(E_CANNOT_MOD));
                    *endp = cc;
                    return;
                }

                let mut di: *mut Dictitem = ptr::null_mut();
                if eval_variable(
                    lp.ll_name.cast(),
                    lp.ll_name_len as i32,
                    Some(&mut tv),
                    &raw mut di,
                    true,
                    false,
                ) {
                    if (di.is_null()
                        || (!var_check_ro(
                            i32::from((*di).di_flags),
                            lp.ll_name.cast(),
                            TV_CSTRING,
                        ) && !tv_check_lock(&(*di).di_tv, lp.ll_name.cast(), TV_CSTRING)))
                        && eexe_mod_op(&mut tv, &*rettv, crate::memory::cstr_from_ptr(op))
                    {
                        set_var(lp.ll_name.cast(), lp.ll_name_len, &mut tv, false);
                    }
                    tv_clear(&mut tv);
                }
            } else {
                set_var_const(lp.ll_name.cast(), lp.ll_name_len, rettv, copy, is_const);
            }
            *endp = cc;
        } else if value_check_lock(
            if lp.ll_newkey.is_none() {
                lp.tv_mut().v_lock
            } else {
                (*lp.tv_mut().vval.v_dict()).dv_lock
            },
            lp.ll_name.cast(),
            TV_CSTRING,
        ) {
            // Skip
        } else if lp.ll_range {
            if is_const {
                emsg(gettext(c"E996: Cannot lock a range"));
                return;
            }

            tv_list_assign_range(
                &mut *lp.ll_list,
                rettv.vval.v_list().as_ref(),
                lp.ll_n1,
                lp.ll_n2,
                lp.ll_empty2,
                op,
                lp.ll_name.cast(),
            );
        } else {
            let mut oldtv = tv_initial_value();
            let dict = lp.ll_dict;
            let watched = tv_dict_is_watched(dict.as_ref());

            if is_const {
                emsg(gettext(c"E996: Cannot lock a list or dict"));
                return;
            }

            let mut skip_assign = false;
            if let Some(ref newkey) = lp.ll_newkey {
                let newkey_ptr = newkey.as_ptr();
                if !op.is_null() && crate::memory::cstr_first_byte(op) != b'=' {
                    semsg!(gettext(E_DICTKEY), newkey_ptr);
                    return;
                }
                if tv_dict_wrong_func_name(&mut *lp.tv_mut().vval.v_dict(), rettv, newkey_ptr) != 0
                {
                    return;
                }

                let di = tv_dict_item_alloc(newkey_ptr);
                if !tv_dict_add(&mut *lp.tv_mut().vval.v_dict(), di) {
                    tv_dict_item_free(di);
                    return;
                }
                lp.ll_tv = &raw mut di.di_tv;
            } else {
                if watched {
                    tv_copy(&*lp.ll_tv, &mut oldtv);
                }

                if !op.is_null() && crate::memory::cstr_first_byte(op) != b'=' {
                    eexe_mod_op(&mut *lp.ll_tv, &*rettv, crate::memory::cstr_from_ptr(op));
                    skip_assign = true;
                } else {
                    tv_clear(&mut *lp.ll_tv);
                }
            }

            if !skip_assign {
                if copy {
                    tv_copy(&*rettv, &mut *lp.ll_tv);
                } else {
                    *lp.ll_tv = *rettv;
                    lp.tv_mut().v_lock = VAR_UNLOCKED;
                    tv_init(Some(rettv));
                }
            }

            if watched {
                if oldtv.v_type == VarType::VarUnknown {
                    let newkey_ptr = lp.ll_newkey.as_ref().unwrap().as_ptr();
                    tv_dict_watcher_notify(&mut *dict, newkey_ptr, lp.ll_tv.as_ref(), None);
                } else {
                    let di_ = lp.ll_di;
                    tv_dict_watcher_notify(
                        &mut *dict,
                        &(*di_).di_key,
                        lp.ll_tv.as_ref(),
                        Some(&oldtv),
                    );
                    tv_clear(&mut oldtv);
                }
            }
        }
    }
}

/// Evaluate the expression used in a ":for var in expr" command.
pub fn eval_for_line(
    arg: impl crate::strings::IntoCCharPtr,
    errp: &mut bool,
    eap: &mut Exarg,
    evalarg: &mut Evalarg,
) -> Box<Forinfo> {
    let arg = arg.into_c_char_ptr();
    unsafe {
        let mut fi = Box::new(Forinfo {
            fi_semicolon: 0,
            fi_varcount: 0,
            fi_source: ForSource::None,
        });
        let mut tv = tv_initial_value();
        let skip = (evalarg.eval_flags & EVAL_EVALUATE) == 0;

        *errp = true;

        let mut expr = skip_var_list(arg, &mut fi.fi_varcount, &mut fi.fi_semicolon, false);
        if expr.is_null() {
            return fi;
        }

        expr = skipwhite(expr);
        if crate::memory::cstr_first_byte(expr) != b'i'
            || crate::memory::cstr_byte_at(expr, 1) != b'n'
            || !(crate::memory::cstr_byte_at(expr, 2) == 0
                || ascii_iswhite(crate::memory::cstr_char_at(expr, 2)))
        {
            emsg(gettext(c"E690: Missing \"in\" after :for"));
            return fi;
        }

        if skip {
            g_st().emsg_skip += 1;
        }
        expr = skipwhite(expr.wrapping_add(2));
        if eval0(expr.cast_mut(), &mut tv, Some(&mut *eap), &mut *evalarg) {
            *errp = false;
            if !skip {
                if tv.v_type == VarType::VarList {
                    let l = tv.vval.v_list();
                    if l.is_null() {
                        tv_clear(&mut tv);
                    } else {
                        let lw = std::rc::Rc::new(Listwatch {
                            lw_item: std::cell::Cell::new(tv_list_first(l.as_ref())),
                        });
                        tv_list_watch_add(&mut *l, lw.clone());
                        fi.fi_source = ForSource::List { list: l, lw };
                    }
                } else if tv.v_type == VarType::VarBlob {
                    let bytes = if tv.vval.v_blob().is_null() {
                        Vec::new()
                    } else {
                        (*tv.vval.v_blob()).bv_ga.clone()
                    };
                    fi.fi_source = ForSource::Blob { bytes, idx: 0 };
                    tv_clear(&mut tv);
                } else if tv.v_type == VarType::VarString {
                    let text = if tv.vval.v_string().is_null() {
                        std::ffi::CString::default()
                    } else {
                        crate::memory::cstr_to_cstring(tv.vval.v_string())
                    };
                    fi.fi_source = ForSource::Str { text, byte_idx: 0 };
                    tv_clear(&mut tv);
                } else {
                    emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED));
                    tv_clear(&mut tv);
                }
            }
        }
        if skip {
            g_st().emsg_skip -= 1;
        }

        fi
    }
}

/// Use the first item in a ":for" list. Advance to the next.
pub fn next_for_item(fi: &mut Forinfo, arg: impl crate::strings::IntoCCharPtr) -> bool {
    let arg = arg.into_c_char_ptr();
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
            ex_let_vars(
                arg,
                &mut tv,
                1,
                fi.fi_semicolon,
                fi.fi_varcount,
                0,
                ptr::null_mut(),
            )
        }
        ForSource::Str { text, byte_idx } => {
            let str_ptr = text.as_ptr();
            let len = utfc_ptr2len(str_ptr.wrapping_add(*byte_idx));
            if len == 0 {
                return false;
            }
            let mut tv = TypVal::new();
            crate::eval::typval::tv_set_string_len(
                &mut tv,
                str_ptr.wrapping_add(*byte_idx),
                len as usize,
            );
            tv.v_lock = VAR_FIXED;
            *byte_idx += len as usize;
            let result = ex_let_vars(
                arg,
                &mut tv,
                1,
                fi.fi_semicolon,
                fi.fi_varcount,
                0,
                ptr::null_mut(),
            );
            tv_clear(&mut tv);
            result
        }
        ForSource::List { list: _, lw } => {
            let item = lw.lw_item.get();
            if item.is_null() {
                return false;
            }
            lw.lw_item.set(tv_list_item_next(unsafe { &*item }));
            ex_let_vars(
                arg,
                &mut *tv_list_item_tv(unsafe { &mut *item }),
                1,
                fi.fi_semicolon,
                fi.fi_varcount,
                0,
                ptr::null_mut(),
            )
        }
    }
}

impl Drop for Forinfo {
    fn drop(&mut self) {
        if let ForSource::List { list, ref lw } = self.fi_source
            && !list.is_null()
        {
            tv_list_watch_remove(unsafe { &mut *list }, lw);
            tv_list_unref(list);
        }
    }
}

pub fn set_context_for_expression(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
    cmdidx: Cmdidx,
) {
    let mut arg: *mut u8 = arg.into_c_char_ptr().cast_mut().cast();
    let mut got_eq = false;

    if cmdidx == Cmdidx::CmdLet || cmdidx == Cmdidx::CmdConst {
        xp.xp_context = EXPAND_USER_VARS;
        if !crate::memory::cstr_as_bytes(arg.cast())
            .iter()
            .any(|b| b"\"'+-*/%.=!?~|&$([<>,#".contains(b))
        {
            let mut p = arg.wrapping_add(crate::memory::cstr_len(arg.cast()));
            while (p as usize) >= (arg as usize) {
                xp.xp_pattern = p;
                p = p.wrapping_offset(
                    -((utf_head_off(arg.cast(), p.wrapping_sub(1).cast()) + 1) as isize),
                );
                if (p as usize) >= (arg as usize)
                    && ascii_iswhite(crate::memory::cstr_first_char(p.cast()))
                {
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
        xp.xp_pattern = crate::memory::cstr_as_bytes(arg.cast())
            .iter()
            .position(|b| b"\"'+-*/%.=!?~|&$([<>,#".contains(b))
            .map_or(ptr::null_mut(), |i| arg.wrapping_add(i));
        !xp.xp_pattern.is_null()
    } {
        let mut c = i32::from(crate::memory::cstr_first_byte(xp.xp_pattern.cast()));
        if c == b'&' as i32 {
            c = i32::from(crate::memory::cstr_byte_at(xp.xp_pattern.cast(), 1));
            if c == b'&' as i32 {
                xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                xp.xp_context = if cmdidx != Cmdidx::CmdLet || got_eq {
                    EXPAND_EXPRESSION
                } else {
                    EXPAND_NOTHING
                };
            } else if c != b' ' as i32 {
                xp.xp_context = EXPAND_SETTINGS;
                if (c == b'l' as i32 || c == b'g' as i32)
                    && crate::memory::cstr_byte_at(xp.xp_pattern.cast(), 2) == b':'
                {
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
                && vim_strchr(xp.xp_pattern.cast(), b'(' as i32).is_null())
        {
            break;
        } else if cmdidx != Cmdidx::CmdLet || got_eq {
            if c == b'"' as i32 {
                loop {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    c = i32::from(crate::memory::cstr_first_byte(xp.xp_pattern.cast()));
                    if c == NUL as i32 || c == b'"' as i32 {
                        break;
                    }
                    if c == b'\\' as i32
                        && crate::memory::cstr_byte_at(xp.xp_pattern.cast(), 1) != 0
                    {
                        xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    }
                }
                xp.xp_context = EXPAND_NOTHING;
            } else if c == b'\'' as i32 {
                loop {
                    xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                    c = i32::from(crate::memory::cstr_first_byte(xp.xp_pattern.cast()));
                    if c == NUL as i32 || c == b'\'' as i32 {
                        break;
                    }
                }
                xp.xp_context = EXPAND_NOTHING;
            } else if c == b'|' as i32 {
                if crate::memory::cstr_byte_at(xp.xp_pattern.cast(), 1) == b'|' {
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
        arg = xp.xp_pattern;
        if !crate::memory::cstr_is_empty(arg.cast()) {
            loop {
                arg = arg.wrapping_add(1);
                c = i32::from(crate::memory::cstr_first_byte(arg.cast()));
                if c == NUL as i32 || (c != b' ' as i32 && c != b'\t' as i32) {
                    break;
                }
            }
        }
    }

    if cmd_has_expr_args(cmdidx) && xp.xp_context == EXPAND_EXPRESSION {
        loop {
            let n: *mut u8 = skiptowhite(arg.cast()).cast();
            if n == arg || ascii_iswhite_or_nul(unsafe { *skipwhite(n.cast()) }) {
                break;
            }
            arg = skipwhite(n.cast()).cast();
        }
    }

    xp.xp_pattern = arg;
}

/// Does not use 'cpo' and always uses 'magic'.
pub fn pattern_match(
    pat: impl crate::strings::IntoCCharPtr,
    text: impl crate::strings::IntoCCharPtr,
    ic: bool,
) -> i32 {
    let pat = pat.into_c_char_ptr();
    let text = text.into_c_char_ptr();
    let mut matches: i32 = 0;
    let mut regmatch: Regmatch = Regmatch::new();

    let save_cpo = opt_vars().p_cpo;
    opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();
    regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
    if !regmatch.regprog.is_null() {
        regmatch.rm_ic = ic;
        matches = i32::from(vim_regexec_nl(&mut regmatch, text, 0));
        vim_regfree(regmatch.regprog);
    }
    opt_vars().p_cpo = save_cpo;
    matches
}

/// Handle a name followed by "(". Both for just "name(arg)" and for
/// "expr->name(arg)".
fn eval_func<T>(
    arg: *mut *mut T,
    evalarg: Option<&mut Evalarg>,
    name: impl crate::strings::IntoCCharPtr,
    name_len: i32,
    rettv: &mut TypVal,
    flags: i32,
    basetv: Option<&mut TypVal>,
) -> bool {
    let name = name.into_c_char_ptr();
    {
        let evaluate = (flags & EVAL_EVALUATE) != 0;
        let mut s = name;
        let mut len = name_len;
        let mut found_var = false;

        if !evaluate {
            check_vars(s, len as usize);
        }

        let mut partial: *mut TvPartial = ptr::null_mut();
        s = deref_func_name(
            s,
            &mut len,
            &raw mut partial,
            !evaluate,
            Some(&mut found_var),
        );

        let s_cstr = cstr_from_bytes(crate::memory::cstr_slice(s, len as usize));
        let mut s_vec: Vec<u8> = s_cstr.into_bytes_with_nul();
        s = s_vec.as_mut_ptr().cast();

        let lnum = curwin_mut().w_cursor.lnum;
        let mut funcexe = Funcexe {
            fe_fill_submatch: false,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: false,
            fe_evaluate: evaluate,
            fe_partial: partial.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: basetv.map_or(ptr::null_mut(), |p| &raw mut *p),
            fe_found_var: found_var,
        };
        let mut ret = get_func_tv(s, len, &mut *rettv, arg, evalarg, &mut funcexe);

        drop(s_vec);

        if rettv.v_type == VarType::VarUnknown
            && !evaluate
            && unsafe { *(*arg).cast::<u8>() } == b'('
        {
            rettv.vval = crate::eval::typval::TypvalVval::String(TV_EMPTY_STRING.cast_mut());
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
}

/// After using "evalarg" filled from "eap": free the memory.
/// Handle zero level expression.
pub fn eval0(
    arg: impl crate::strings::IntoCCharPtr,
    rettv: &mut TypVal,
    mut eap: Option<&mut Exarg>,
    evalarg: &mut Evalarg,
) -> bool {
    let arg = arg.into_c_char_ptr();
    {
        let did_emsg_before = g_st().did_emsg;
        let called_emsg_before = g_st().called_emsg;
        let mut end_error = false;

        let mut p = skipwhite(arg);
        let ret = eval1(&raw mut p, rettv, Some(evalarg));

        if ret {
            end_error = ends_excmd(i32::from(crate::memory::cstr_first_byte(p))) == 0;
        }
        if !ret || end_error {
            if ret {
                tv_clear(rettv);
            }
            if !aborting()
                && g_st().did_emsg == did_emsg_before
                && g_st().called_emsg == called_emsg_before
            {
                if end_error {
                    semsg!(gettext(E_TRAILING_ARG), p);
                } else {
                    semsg!(gettext(E_INVEXPR2), arg);
                }
            }

            if eap.is_some() && !p.is_null() {
                let nextcmd = check_nextcmd(p);
                if !nextcmd.is_null() && crate::memory::cstr_first_byte(nextcmd) != b'|' {
                    eap.as_deref_mut().unwrap().nextcmd = nextcmd;
                }
            }
            return false;
        }

        if let Some(eap) = eap {
            eap.nextcmd = check_nextcmd(p);
        }

        ret
    }
}

/// If "arg" is a simple function call without arguments then call it and return
/// the result. Otherwise return NOTDONE.
pub fn may_call_simple_func(arg: impl crate::strings::IntoCCharPtr, rettv: &mut TypVal) -> i32 {
    let arg = arg.into_c_char_ptr();
    {
        let parens = crate::memory::cstr_as_bytes(arg)
            .windows(2)
            .position(|w| w == b"()")
            .map_or(ptr::null(), |i| arg.wrapping_add(i));
        let mut r = NOTDONE;

        if !parens.is_null() && unsafe { *skipwhite(parens.wrapping_add(2)) } == NUL {
            if crate::memory::cstr_as_bytes(arg).starts_with(b"v:lua.") {
                let p = arg.wrapping_add(6);
                if p != parens && skip_luafunc_name(p) == parens.cast() {
                    r = call_simple_luafunc(p, (parens as usize).wrapping_sub(p as usize), rettv);
                }
            } else {
                let p = if crate::memory::cstr_prefix_bytes(arg, 5) == &b"<SNR>"[..5] {
                    skipdigits(arg.wrapping_add(5))
                } else {
                    arg
                };
                if to_name_end(p, true) == parens {
                    r = call_simple_func(arg, (parens as usize).wrapping_sub(arg as usize), rettv);
                }
            }
        }
        r
    }
}

fn eval0_simple_funccal(
    arg: impl crate::strings::IntoCCharPtr,
    rettv: &mut TypVal,
    eap: Option<&mut Exarg>,
    evalarg: &mut Evalarg,
) -> bool {
    let arg = arg.into_c_char_ptr();
    let r = may_call_simple_func(arg, rettv);

    if r == NOTDONE {
        eval0(arg, rettv, eap, evalarg)
    } else {
        r == OK
    }
}

/// Handle top level expression:
///      expr2 ? expr1 : expr1
///      expr2 ?? expr1
pub fn eval1<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg = arg.cast();
    unsafe {
        *rettv = TypVal::new();

        if !eval2(arg, rettv, evalarg.as_deref_mut()) {
            return false;
        }

        let mut p = *arg;
        if crate::memory::cstr_first_byte(p) == b'?' {
            let op_falsy = crate::memory::cstr_byte_at(p, 1) == b'?';
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

            if op_falsy {
                *arg = (*arg).wrapping_add(1);
            }
            *arg = skipwhite((*arg).wrapping_add(1));
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
                *rettv = var2;
            }

            if !op_falsy {
                p = *arg;
                if crate::memory::cstr_first_byte(p) != b':' {
                    emsg(gettext(c"E109: Missing ':' after '?'"));
                    if evaluate && result {
                        tv_clear(rettv);
                    }
                    evalarg_used.eval_flags = orig_flags;
                    return false;
                }

                *arg = skipwhite((*arg).wrapping_add(1));
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
}

/// Handle first level expression: `expr2 || expr2 || expr2`
fn eval2<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg = arg.cast();
    unsafe {
        if !eval3(arg, rettv, evalarg.as_deref_mut()) {
            return false;
        }

        let mut p = *arg;
        if crate::memory::cstr_first_byte(p) == b'|' && crate::memory::cstr_byte_at(p, 1) == b'|' {
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

            while crate::memory::cstr_first_byte(p) == b'|'
                && crate::memory::cstr_byte_at(p, 1) == b'|'
            {
                *arg = skipwhite((*arg).wrapping_add(2));
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
}

/// Handle second level expression: `expr3 && expr3 && expr3`
fn eval3<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg = arg.cast();
    unsafe {
        if !eval4(arg, rettv, evalarg.as_deref_mut()) {
            return false;
        }

        let mut p = *arg;
        if crate::memory::cstr_first_byte(p) == b'&' && crate::memory::cstr_byte_at(p, 1) == b'&' {
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

            while crate::memory::cstr_first_byte(p) == b'&'
                && crate::memory::cstr_byte_at(p, 1) == b'&'
            {
                *arg = skipwhite((*arg).wrapping_add(2));
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
}

/// Handle third level expression (comparisons).
fn eval4<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg = arg.cast();
    let mut var2 = tv_initial_value();
    let mut expr_type: Exprtype = EXPR_UNKNOWN;
    let mut len: usize = 2;

    if !eval5(arg, rettv, evalarg.as_deref_mut()) {
        return false;
    }

    let p = unsafe { *arg };
    match crate::memory::cstr_first_byte(p) {
        b'=' => {
            if crate::memory::cstr_byte_at(p, 1) == b'=' {
                expr_type = EXPR_EQUAL;
            } else if crate::memory::cstr_byte_at(p, 1) == b'~' {
                expr_type = EXPR_MATCH;
            }
        }
        b'!' => {
            if crate::memory::cstr_byte_at(p, 1) == b'=' {
                expr_type = EXPR_NEQUAL;
            } else if crate::memory::cstr_byte_at(p, 1) == b'~' {
                expr_type = EXPR_NOMATCH;
            }
        }
        b'>' => {
            if crate::memory::cstr_byte_at(p, 1) != b'=' {
                expr_type = EXPR_GREATER;
                len = 1;
            } else {
                expr_type = EXPR_GEQUAL;
            }
        }
        b'<' => {
            if crate::memory::cstr_byte_at(p, 1) != b'=' {
                expr_type = EXPR_SMALLER;
                len = 1;
            } else {
                expr_type = EXPR_SEQUAL;
            }
        }
        b'i' if crate::memory::cstr_byte_at(p, 1) == b's' => {
            if crate::memory::cstr_byte_at(p, 2) == b'n'
                && crate::memory::cstr_byte_at(p, 3) == b'o'
                && crate::memory::cstr_byte_at(p, 4) == b't'
            {
                len = 5;
            }
            let next_c = unsafe { *p.wrapping_add(len).cast::<u8>() };
            if !ascii_isalnum(i32::from(next_c)) && next_c != b'_' {
                expr_type = if len == 2 { EXPR_IS } else { EXPR_ISNOT };
            }
        }
        _ => {}
    }

    if expr_type != EXPR_UNKNOWN {
        let ic: bool;
        if crate::memory::cstr_byte_at(p, len) == b'?' {
            ic = true;
            len += 1;
        } else if crate::memory::cstr_byte_at(p, len) == b'#' {
            ic = false;
            len += 1;
        } else {
            ic = opt_vars().p_ic != 0;
        }

        unsafe {
            *arg = skipwhite(p.wrapping_add(len));
        }
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
    let b1 = tv1.vval.v_blob();
    let b2 = tv2.vval.v_blob();
    let b = tv_blob_alloc();

    let len1 = i64::from(tv_blob_len(unsafe { b1.as_ref() }));
    let len2 = i64::from(tv_blob_len(unsafe { b2.as_ref() }));
    let totallen = len1 + len2;

    if totallen >= 0 && totallen <= i64::from(i32::MAX) {
        let bv = &mut b.bv_ga;
        bv.reserve(totallen as usize);
        if !b1.is_null() {
            bv.extend_from_slice(unsafe { &(*b1).bv_ga });
        }
        if !b2.is_null() {
            bv.extend_from_slice(unsafe { &(*b2).bv_ga });
        }
    }

    tv_clear(tv1);
    tv_blob_set_ret(tv1, b);
}

/// Make a copy of list "tv1" and append list "tv2".
fn eval_addlist(tv1: &mut TypVal, tv2: &mut TypVal) -> bool {
    let mut var3 = tv_initial_value();
    if !tv_list_concat(
        unsafe { tv1.vval.v_list().as_mut() },
        unsafe { tv2.vval.v_list().as_mut() },
        &mut var3,
    ) {
        tv_clear(tv1);
        tv_clear(tv2);
        return false;
    }
    tv_clear(tv1);
    *tv1 = var3;
    true
}

/// Append string "s2" to the string in "tv1".
pub fn grow_string_tv(tv1: &mut TypVal, s2: impl crate::strings::IntoCCharPtr) -> bool {
    let s2 = s2.into_c_char_ptr();
    {
        if tv1.v_type != VarType::VarString || tv1.vval.v_string().is_null() {
            return false;
        }

        let joined = concat_str(tv1.vval.v_string(), s2);
        crate::eval::typval::tv_set_string(tv1, joined.as_ptr());
        true
    }
}

/// Concatenate strings "tv1" and "tv2" and store the result in "tv1".
fn eval_concat_str(tv1: &mut TypVal, tv2: &mut TypVal) -> bool {
    let mut buf1 = [0u8; NUMBUFLEN];
    let mut buf2 = [0u8; NUMBUFLEN];
    let s1 = tv_get_string_buf(&*tv1, &mut buf1).as_ptr();
    let Some(s2_cstr) = tv_get_string_buf_chk(&*tv2, &mut buf2) else {
        tv_clear(tv1);
        tv_clear(tv2);
        return false;
    };
    let s2 = s2_cstr.as_ptr();

    if grow_string_tv(tv1, s2) {
        return true;
    }

    let p = concat_str(s1, s2);
    crate::eval::typval::tv_set_string(tv1, p.as_ptr());

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
fn eval5<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg = arg.cast();
    unsafe {
        if !eval6(arg, rettv, evalarg.as_deref_mut(), false) {
            return false;
        }

        loop {
            let op = i32::from(**arg as u8);
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

            if op == b'.' as i32 && crate::memory::cstr_byte_at(*arg, 1) == b'.' {
                *arg = (*arg).wrapping_add(1);
            }
            *arg = skipwhite((*arg).wrapping_add(1));
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
fn eval6<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    want_string: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        if !eval7(arg, rettv, evalarg.as_deref_mut(), want_string) {
            return false;
        }

        loop {
            let op = i32::from(**arg);
            if op != b'*' as i32 && op != b'/' as i32 && op != b'%' as i32 {
                break;
            }

            let evaluate = evalarg
                .as_ref()
                .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

            *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
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
}

/// Handle sixth level expression.
fn eval7<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    want_string: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut ret: i32;

        rettv.v_type = VarType::VarUnknown;

        let start_leader: *const u8 = *arg;
        while matches!(**arg, b'!' | b'-' | b'+') {
            *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        }
        let mut end_leader: *const u8 = *arg;

        if eval_st().eval7_recurse == MAX_EVAL7_RECURSE {
            semsg!(gettext(E_EXPRESSION_TOO_RECURSIVE_STR), *arg);
            return false;
        }
        eval_st().eval7_recurse += 1;

        match **arg {
            b'0'..=b'9' => {
                let mut ok = eval_number(arg, &mut *rettv, evaluate, want_string);

                if ok && evaluate && (end_leader as usize) > (start_leader as usize) {
                    ok = eval7_leader(&mut *rettv, true, start_leader.cast(), &raw mut end_leader);
                }
                ret = if ok { OK } else { FAIL };
            }
            b'"' => {
                ret = if eval_string(arg, &mut *rettv, evaluate, false) {
                    OK
                } else {
                    FAIL
                };
            }
            b'\'' => {
                ret = if eval_lit_string(arg, &mut *rettv, evaluate, false) {
                    OK
                } else {
                    FAIL
                };
            }
            b'[' => {
                ret = if eval_list(arg, &mut *rettv, evalarg.as_deref_mut()) {
                    OK
                } else {
                    FAIL
                };
            }
            b'#' => {
                ret = eval_lit_dict(arg, &mut *rettv, evalarg.as_deref_mut());
            }
            b'{' => {
                ret = get_lambda_tv(arg, &mut *rettv, evalarg.as_deref_mut());
                if ret == NOTDONE {
                    ret = eval_dict(arg, &mut *rettv, evalarg.as_deref_mut(), false);
                }
            }
            b'&' => {
                ret = if eval_option(arg.cast::<*const u8>(), Some(&mut *rettv), evaluate) {
                    OK
                } else {
                    FAIL
                };
            }
            b'$' => {
                let next_c = *(*arg).wrapping_add(1);
                let ok = if next_c == b'"' || next_c == b'\'' {
                    eval_interp_string(arg, &mut *rettv, evaluate)
                } else {
                    eval_env_var(arg, &mut *rettv, i32::from(evaluate))
                };
                ret = if ok { OK } else { FAIL };
            }
            b'@' => {
                *arg = (*arg).wrapping_add(1);
                if evaluate {
                    let reg_str = get_reg_contents_str(i32::from(**arg), K_GREG_EXPR_SRC);
                    crate::eval::typval::tv_set_string(
                        rettv,
                        reg_str.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                    );
                }
                if **arg != 0 {
                    *arg = (*arg).wrapping_add(1);
                }
                ret = OK;
            }
            b'(' => {
                *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();

                let mut ok = eval1(arg, &mut *rettv, evalarg.as_deref_mut());
                if **arg == b')' {
                    *arg = (*arg).wrapping_add(1);
                } else if ok {
                    emsg(gettext(c"E110: Missing ')'"));
                    tv_clear(rettv);
                    ok = false;
                }
                ret = if ok { OK } else { FAIL };
            }
            _ => {
                ret = NOTDONE;
            }
        }

        let mut ok = if ret == NOTDONE {
            let mut s = *arg;
            let mut alias: Option<std::ffi::CString> = None;
            let len = get_name_len(arg.cast::<*const u8>(), &mut alias, evaluate, true);
            if let Some(ref a) = alias {
                s = a.as_ptr().cast_mut().cast();
            }

            let res = if len <= 0 {
                false
            } else {
                let flags = if evalarg.is_none() {
                    0
                } else {
                    evalarg.as_deref_mut().unwrap().eval_flags
                };
                if crate::memory::cstr_first_byte(skipwhite((*arg).cast())) == b'(' {
                    *arg = skipwhite((*arg).cast()).cast();
                    eval_func(
                        arg,
                        evalarg.as_deref_mut(),
                        s.cast(),
                        len,
                        &mut *rettv,
                        flags,
                        None,
                    )
                } else if evaluate {
                    eval_variable(
                        s.cast(),
                        len,
                        Some(&mut *rettv),
                        ptr::null_mut(),
                        true,
                        false,
                    )
                } else {
                    check_vars(s.cast(), len as usize);
                    if rettv.v_type == VarType::VarUnknown
                        && !evaluate
                        && crate::memory::cstr_as_bytes(s.cast()).starts_with(b"v:lua.")
                    {
                        rettv.v_type = VarType::VarPartial;
                        let lua_pt = get_vim_var_partial(VV_LUA);
                        tv_set_partial(rettv, lua_pt);
                        (*lua_pt).pt_refcount += 1;
                    }
                    true
                }
            };
            drop(alias);
            res
        } else {
            ret == OK
        };

        *arg = skipwhite((*arg).cast()).cast();

        if ok {
            ok = handle_subscript(arg.cast::<*const u8>(), &mut *rettv, evalarg, true);
        }

        if ok && evaluate && (end_leader as usize) > (start_leader as usize) {
            ok = eval7_leader(&mut *rettv, false, start_leader.cast(), &raw mut end_leader);
        }

        eval_st().eval7_recurse -= 1;
        ok
    }
}

/// Apply the leading "!" and "-" before an eval7 expression to "rettv".
fn eval7_leader<T>(
    rettv: &mut TypVal,
    numeric_only: bool,
    start_leader: impl crate::strings::IntoCCharPtr,
    end_leaderp: *mut *const T,
) -> bool {
    let start_leader = start_leader.into_c_char_ptr();
    {
        let mut end_leader: *const u8 = unsafe { (*end_leaderp).cast() };
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
            while (end_leader as usize) > (start_leader as usize) {
                end_leader = end_leader.wrapping_sub(1);
                if crate::memory::cstr_first_byte(end_leader.cast()) == b'!' {
                    if numeric_only {
                        end_leader = end_leader.wrapping_add(1);
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
                } else if crate::memory::cstr_first_byte(end_leader.cast()) == b'-' {
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

        unsafe {
            *end_leaderp = end_leader.cast();
        }
        ret
    }
}

/// Call the function referred to in "rettv".
fn call_func_rettv<T>(
    arg: *mut *mut T,
    evalarg: Option<&mut Evalarg>,
    rettv: &mut TypVal,
    evaluate: bool,
    selfdict: *mut TvDict,
    basetv: Option<&mut TypVal>,
    lua_funcname: impl crate::strings::IntoCCharPtr,
) -> bool {
    let lua_funcname = lua_funcname.into_c_char_ptr();
    {
        let mut pt: *mut TvPartial = ptr::null_mut();
        let mut functv = tv_initial_value();
        let funcname;
        let mut is_lua = false;

        if evaluate {
            functv = *rettv;
            rettv.v_type = VarType::VarUnknown;

            if functv.v_type == VarType::VarPartial {
                pt = tv_partial(&functv);
                is_lua = is_luafunc(pt);
                funcname = if is_lua {
                    lua_funcname
                } else {
                    partial_name(unsafe { pt.as_ref() }).as_ptr()
                };
            } else {
                funcname = functv.vval.v_string();
                if crate::memory::cstr_is_empty(funcname) {
                    emsg(gettext(E_EMPTY_FUNCTION_NAME));
                    tv_clear(&mut functv);
                    return false;
                }
            }
        } else {
            funcname = c"".as_ptr();
        }

        let lnum = curwin_mut().w_cursor.lnum;
        let mut funcexe = Funcexe {
            fe_fill_submatch: false,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: false,
            fe_evaluate: evaluate,
            fe_partial: pt.cast(),
            fe_selfdict: selfdict,
            fe_basetv: basetv.map_or(ptr::null_mut(), |p| &raw mut *p),
            fe_found_var: false,
        };
        let ret = get_func_tv(
            funcname,
            if is_lua {
                ((unsafe { *arg } as usize).wrapping_sub(funcname as usize)) as i32
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
}

/// Evaluate "->method()".
fn eval_lambda<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        *arg = (*arg).wrapping_add(2);
        let mut base = *rettv;
        rettv.v_type = VarType::VarUnknown;

        let ret = if get_lambda_tv(arg, &mut *rettv, evalarg.as_deref_mut()) != OK {
            return false;
        } else if **arg != b'(' {
            if verbose {
                if crate::memory::cstr_first_byte(skipwhite((*arg).cast())) == b'(' {
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
                ptr::null_mut(),
                Some(&mut base),
                ptr::null(),
            )
        };

        if evaluate {
            tv_clear(&mut base);
        }

        ret
    }
}

/// Evaluate "->method()" or "->v:lua.method()".
fn eval_method<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);

        *arg = (*arg).wrapping_add(2);
        let mut base = *rettv;
        rettv.v_type = VarType::VarUnknown;

        let mut len: i32;
        let mut name = *arg;
        let mut lua_funcname: *mut u8 = ptr::null_mut();
        let mut alias: Option<std::ffi::CString> = None;

        if crate::memory::cstr_as_bytes(name.cast()).starts_with(b"v:lua.") {
            lua_funcname = name.wrapping_add(6);
            *arg = skip_luafunc_name(lua_funcname.cast()).cast_mut();
            *arg = skipwhite((*arg).cast()).cast();
            len = ((*arg as usize).wrapping_sub(lua_funcname as usize)) as i32;
        } else {
            len = get_name_len(arg.cast::<*const u8>(), &mut alias, evaluate, true);
            if let Some(ref a) = alias {
                name = a.as_ptr().cast_mut().cast();
            }
        }

        let mut tofree: Option<std::ffi::CString> = None;
        let mut ret = true;

        if len <= 0 {
            if verbose {
                if lua_funcname.is_null() {
                    emsg(gettext(c"E260: Missing name after ->"));
                } else {
                    semsg!(gettext(E_INVEXPR2), name);
                }
            }
            ret = false;
        } else {
            *arg = skipwhite((*arg).cast()).cast();

            let paren: *mut u8;
            if **arg != b'(' && lua_funcname.is_null() && alias.is_none() && {
                paren = vim_strchr((*arg).cast(), b'(' as i32).cast();
                !paren.is_null()
            } {
                *arg = name;
                *paren = 0;
                let mut ref_tv = tv_initial_value();
                if !eval7(arg, &mut ref_tv, evalarg.as_deref_mut(), false) {
                    *arg = name.wrapping_add(len as usize);
                    ret = false;
                } else if !crate::memory::cstr_is_empty(skipwhite((*arg).cast())) {
                    if verbose {
                        semsg!(gettext(E_TRAILING_ARG), *arg);
                    }
                    ret = false;
                } else if ref_tv.v_type == VarType::VarFunc && !ref_tv.vval.v_string().is_null() {
                    let tf = crate::eval::typval::tv_take_string(ref_tv.vval.v_string())
                        .unwrap_or_default();
                    ref_tv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                    len = tf.as_bytes().len() as i32;
                    name = tf.as_ptr().cast_mut().cast();
                    tofree = Some(tf);
                } else if ref_tv.v_type == VarType::VarPartial && !tv_partial(&ref_tv).is_null() {
                    let ref_pt = tv_partial(&ref_tv);
                    if !(*ref_pt).pt_argv.is_empty() || !(*ref_pt).pt_dict.is_null() {
                        if verbose {
                            emsg(gettext(E_CANNOT_USE_PARTIAL_HERE));
                        }
                        ret = false;
                    } else {
                        let tf = partial_name(ref_pt.as_ref()).to_owned();
                        len = tf.as_bytes().len() as i32;
                        name = tf.as_ptr().cast_mut().cast();
                        tofree = Some(tf);
                    }
                } else {
                    if verbose {
                        semsg!(gettext(E_NOT_CALLABLE_TYPE_STR), name);
                    }
                    ret = false;
                }
                tv_clear(&mut ref_tv);
                *paren = b'(';
            }

            if ret {
                if **arg != b'(' {
                    if verbose {
                        semsg!(gettext(E_MISSINGPAREN), name);
                    }
                    ret = false;
                } else if ascii_iswhite(i32::from(*(*arg).wrapping_sub(1))) {
                    if verbose {
                        emsg(gettext(E_NOWHITESPACE));
                    }
                    ret = false;
                } else if !lua_funcname.is_null() {
                    if evaluate {
                        rettv.v_type = VarType::VarPartial;
                        let lua_pt = get_vim_var_partial(VV_LUA);
                        tv_set_partial(rettv, lua_pt);
                        (*lua_pt).pt_refcount += 1;
                    }
                    ret = call_func_rettv(
                        arg,
                        evalarg.as_deref_mut(),
                        &mut *rettv,
                        evaluate,
                        ptr::null_mut(),
                        Some(&mut base),
                        lua_funcname.cast(),
                    );
                } else {
                    ret = eval_func(
                        arg,
                        evalarg,
                        name.cast(),
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
}

/// Evaluate an "[expr]" or "[expr:expr]" index. Also "dict.key".
fn eval_index<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut empty1 = false;
        let mut empty2 = false;
        let mut range = false;
        let mut key: *const u8 = ptr::null();
        let mut keylen: isize = -1;

        if !check_can_index(rettv, evaluate, verbose) {
            return false;
        }

        let mut var1 = tv_initial_value();
        let mut var2 = tv_initial_value();
        if **arg == b'.' {
            key = (*arg).wrapping_add(1);
            keylen = 0;
            while eval_isdictc(i32::from(crate::memory::cstr_byte_at(
                key.cast(),
                keylen as usize,
            ))) {
                keylen += 1;
            }
            if keylen == 0 {
                return false;
            }
            *arg = skipwhite(key.wrapping_add(keylen as usize).cast()).cast();
        } else {
            *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
            if **arg == b':' {
                empty1 = true;
            } else if !eval1(arg, &mut var1, evalarg.as_deref_mut()) {
                return false;
            } else if evaluate && !tv_check_str(&var1) {
                tv_clear(&mut var1);
                return false;
            }

            if **arg == b':' {
                range = true;
                *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
                if **arg == b']' {
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

            if **arg != b']' {
                if verbose {
                    emsg(gettext(E_MISSBRAC));
                }
                tv_clear(&mut var1);
                if range {
                    tv_clear(&mut var2);
                }
                return false;
            }
            *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        }

        if evaluate {
            let res = eval_index_inner(
                rettv,
                range,
                (if empty1 {
                    ptr::null_mut()
                } else {
                    &raw mut var1
                })
                .as_ref(),
                (if empty2 {
                    ptr::null_mut()
                } else {
                    &raw mut var2
                })
                .as_ref(),
                false,
                key.cast(),
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
        ptr::null(),
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
    key: impl crate::strings::IntoCCharPtr,
    keylen: isize,
    verbose: bool,
) -> bool {
    let mut key = key.into_c_char_ptr();
    {
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
                        string_slice(s, n1, n2, exclusive)
                    } else {
                        char_from_string(s, n1)
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
                        Some(cstr_from_bytes(&s_bytes[n1 as usize..=n2 as usize]))
                    }
                } else if n1 >= len || n1 < 0 {
                    None
                } else {
                    Some(cstr_from_bytes(&s_bytes[n1 as usize..=n1 as usize]))
                };
                tv_clear(rettv);
                if let Some(ref c) = v {
                    crate::eval::typval::tv_set_string(rettv, c.as_ptr());
                } else {
                    rettv.v_type = VarType::VarString;
                    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
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
                if key.is_null() {
                    let Some(k) = tv_get_string_chk(var1.unwrap()) else {
                        return false;
                    };
                    key = k.as_ptr();
                }

                let item = tv_dict_find(unsafe { rettv.vval.v_dict().as_ref() }, key, keylen);

                if item.is_null() && verbose {
                    if keylen > 0 {
                        semsg!(gettext(E_DICTKEY_LEN), keylen, key);
                    } else {
                        semsg!(gettext(E_DICTKEY), key);
                    }
                }
                if item.is_null() || tv_is_luafunc(unsafe { &mut (*item).di_tv }) {
                    return false;
                }

                let mut tmp = tv_initial_value();
                tv_copy(unsafe { &(*item).di_tv }, &mut tmp);
                tv_clear(rettv);
                *rettv = tmp;
            }
        }
        true
    }
}

/// Get an option value
pub fn eval_option<T>(arg: *mut *const T, rettv: Option<&mut TypVal>, evaluate: bool) -> bool {
    let arg: *mut *const u8 = arg.cast();
    let working = unsafe { **arg } == b'+';
    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;

    let option_end = find_option_var_end(arg, &mut opt_idx, &mut opt_flags).cast_mut();

    if option_end.is_null() {
        if rettv.is_some() {
            semsg!(gettext(c"E112: Option name missing: %s"), unsafe { *arg });
        }
        return false;
    }

    if !evaluate {
        unsafe {
            *arg = option_end;
        }
        return true;
    }

    let c = unsafe { *option_end };
    unsafe {
        *option_end = 0;
    }

    let mut ret = true;
    {
        let is_tty_opt = is_tty_option(unsafe { (*arg).cast() });

        if opt_idx == K_OPT_INVALID && !is_tty_opt {
            if rettv.is_some() {
                semsg!(gettext(c"E113: Unknown option: %s"), unsafe { *arg });
            }

            ret = false;
        } else if let Some(rettv) = rettv {
            let value = if is_tty_opt {
                get_tty_option(unsafe { (*arg).cast() })
            } else {
                get_option_value(opt_idx, opt_flags)
            };
            debug_assert!(value.obj_type != ObjectType::ObjectTypeNil);

            *rettv = opt_to_tv(value, true);
            optval_free(value);
        } else if working && !is_tty_opt && is_option_hidden(opt_idx) {
            ret = false;
        }
    }

    unsafe {
        *option_end = c;
    }
    unsafe {
        *arg = option_end;
    }

    ret
}

/// Allocate a variable for a number constant. Also deals with "0z" for blob.
fn eval_number<T>(arg: *mut *mut T, rettv: &mut TypVal, evaluate: bool, want_string: bool) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let mut p = skipdigits((*arg).wrapping_add(1).cast());
        let mut get_float = false;

        if !want_string
            && crate::memory::cstr_first_byte(p) == b'.'
            && ascii_isdigit(crate::memory::cstr_char_at(p, 1))
        {
            get_float = true;
            p = skipdigits(p.wrapping_add(2));
            if crate::memory::cstr_first_byte(p) == b'e'
                || crate::memory::cstr_first_byte(p) == b'E'
            {
                p = p.wrapping_add(1);
                if crate::memory::cstr_first_byte(p) == b'-'
                    || crate::memory::cstr_first_byte(p) == b'+'
                {
                    p = p.wrapping_add(1);
                }
                if !ascii_isdigit(crate::memory::cstr_first_char(p)) {
                    get_float = false;
                } else {
                    p = skipdigits(p.wrapping_add(1));
                }
            }
            if ascii_isalpha(crate::memory::cstr_first_char(p))
                || crate::memory::cstr_first_byte(p) == b'.'
            {
                get_float = false;
            }
        }
        if get_float {
            let mut f: TvFloat = 0.0;
            *arg = (*arg).wrapping_add(string2float((*arg).cast(), &mut f));
            if evaluate {
                rettv.v_type = VarType::VarFloat;
                rettv.vval = crate::eval::typval::TypvalVval::Float(f);
            }
        } else if **arg == b'0'
            && (crate::memory::cstr_byte_at((*arg).cast(), 1) == b'z'
                || crate::memory::cstr_byte_at((*arg).cast(), 1) == b'Z')
        {
            let mut bytes: Vec<u8> = Vec::new();
            let mut bp = (*arg).wrapping_add(2);
            while ascii_isxdigit(crate::memory::cstr_first_char(bp.cast())) {
                if !ascii_isxdigit(crate::memory::cstr_char_at(bp.cast(), 1)) {
                    if evaluate {
                        emsg(gettext(
                            c"E973: Blob literal should have an even number of hex characters",
                        ));
                    }
                    return false;
                }
                if evaluate {
                    let byte = ((hex2nr(i32::from(crate::memory::cstr_first_byte(bp.cast()))) << 4)
                        + hex2nr(i32::from(crate::memory::cstr_byte_at(bp.cast(), 1))))
                        as u8;
                    bytes.push(byte);
                }
                if crate::memory::cstr_byte_at(bp.cast(), 2) == b'.'
                    && ascii_isxdigit(crate::memory::cstr_char_at(bp.cast(), 3))
                {
                    bp = bp.wrapping_add(1);
                }
                bp = bp.wrapping_add(2);
            }
            if evaluate {
                let blob = tv_blob_alloc();
                blob.bv_ga = bytes;
                tv_blob_set_ret(rettv, blob);
            }
            *arg = bp;
        } else {
            let mut len: i32 = 0;
            let mut n: Varnumber = 0;
            vim_str2nr(
                (*arg).cast(),
                None,
                Some(&mut len),
                STR2NR_ALL,
                Some(&mut n),
                None,
                0,
                true,
                None,
            );
            if len == 0 {
                if evaluate {
                    semsg!(gettext(E_INVEXPR2), *arg);
                }
                return false;
            }
            *arg = (*arg).wrapping_add(len as usize);
            if evaluate {
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(n);
            }
        }
        true
    }
}

/// Evaluate a string constant and put the result in "rettv".
fn eval_string<T>(arg: *mut *mut T, rettv: &mut TypVal, evaluate: bool, interpolate: bool) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let arg_end = (*arg).wrapping_add(crate::memory::cstr_len((*arg).cast()));
        let mut extra: isize = if interpolate { 1 } else { 0 };
        let off: usize = if interpolate { 0 } else { 1 };

        let mut p = (*arg).wrapping_add(off);
        while !crate::memory::cstr_is_empty(p.cast())
            && crate::memory::cstr_first_byte(p.cast()) != b'"'
        {
            if crate::memory::cstr_first_byte(p.cast()) == b'\\'
                && crate::memory::cstr_byte_at(p.cast(), 1) != 0
            {
                p = p.wrapping_add(1);
                if crate::memory::cstr_first_byte(p.cast()) == b'<' {
                    let mut modifiers: i32 = 0;
                    let mut flags = FSK_KEYCODE | FSK_IN_STRING;

                    extra += 5;

                    if crate::memory::cstr_byte_at(p.cast(), 1) != b'*' {
                        flags |= FSK_SIMPLIFY;
                    }
                    let mut p_const: *const u8 = p;
                    if find_special_key(
                        &raw mut p_const,
                        (arg_end as usize).wrapping_sub(p as usize),
                        &mut modifiers,
                        flags,
                        None,
                    ) != 0
                    {
                        p = p_const.cast_mut().wrapping_sub(1);
                    } else {
                        p = p_const.cast_mut();
                    }
                }
            } else if interpolate
                && (crate::memory::cstr_first_byte(p.cast()) == b'{'
                    || crate::memory::cstr_first_byte(p.cast()) == b'}')
            {
                if crate::memory::cstr_first_byte(p.cast()) == b'{'
                    && crate::memory::cstr_byte_at(p.cast(), 1) != b'{'
                {
                    break;
                }
                p = p.wrapping_add(1);
                if crate::memory::cstr_first_byte(p.wrapping_sub(1).cast()) == b'}'
                    && crate::memory::cstr_first_byte(p.cast()) != b'}'
                {
                    semsg!(gettext(E_STRAY_CLOSING_CURLY_STR), *arg);
                    return false;
                }
                extra -= 1;
            }
            p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
        }

        if crate::memory::cstr_first_byte(p.cast()) != b'"'
            && !(interpolate && crate::memory::cstr_first_byte(p.cast()) == b'{')
        {
            semsg!(gettext(c"E114: Missing quote: %s"), *arg);
            return false;
        }

        if !evaluate {
            *arg = p.wrapping_add(off);
            return true;
        }

        let len = ((p as isize) - (*arg as isize) + extra) as usize;
        let mut buf_vec = vec![0u8; len];
        let buf_ptr = buf_vec.as_mut_ptr();
        let mut end = buf_ptr;

        p = (*arg).wrapping_add(off);
        while !crate::memory::cstr_is_empty(p.cast())
            && crate::memory::cstr_first_byte(p.cast()) != b'"'
        {
            if crate::memory::cstr_first_byte(p.cast()) == b'\\' {
                p = p.wrapping_add(1);
                match crate::memory::cstr_first_byte(p.cast()) {
                    b'b' => {
                        *end = BS;
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b'e' => {
                        *end = ESC;
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b'f' => {
                        *end = FF;
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b'n' => {
                        *end = b'\n';
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b'r' => {
                        *end = b'\r';
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b't' => {
                        *end = TAB;
                        end = end.wrapping_add(1);
                        p = p.wrapping_add(1);
                    }
                    b'X' | b'x' | b'u' | b'U' => {
                        if ascii_isxdigit(crate::memory::cstr_char_at(p.cast(), 1)) {
                            let c = (crate::memory::cstr_first_byte(p.cast())).to_ascii_uppercase();
                            let mut n = if c == b'X' {
                                2
                            } else if crate::memory::cstr_first_byte(p.cast()) == b'u' {
                                4
                            } else {
                                8
                            };
                            let mut nr: i32 = 0;
                            while n > 0 && ascii_isxdigit(crate::memory::cstr_char_at(p.cast(), 1))
                            {
                                n -= 1;
                                p = p.wrapping_add(1);
                                nr = (nr << 4)
                                    + hex2nr(i32::from(crate::memory::cstr_first_byte(p.cast())));
                            }
                            p = p.wrapping_add(1);
                            if c != b'X' {
                                end = end.wrapping_add(utf_char2bytes(nr, end.cast()) as usize);
                            } else {
                                *end = nr as u8;
                                end = end.wrapping_add(1);
                            }
                        }
                    }
                    b'0'..=b'7' => {
                        *end = crate::memory::cstr_first_byte(p.cast()) - b'0';
                        p = p.wrapping_add(1);
                        if matches!(*p, b'0'..=b'7') {
                            *end = ((*end) << 3)
                                .wrapping_add(crate::memory::cstr_first_byte(p.cast()) - b'0');
                            p = p.wrapping_add(1);
                            if matches!(*p, b'0'..=b'7') {
                                *end = ((*end) << 3)
                                    .wrapping_add(crate::memory::cstr_first_byte(p.cast()) - b'0');
                                p = p.wrapping_add(1);
                            }
                        }
                        end = end.wrapping_add(1);
                    }
                    b'<' => {
                        let mut flags = FSK_KEYCODE | FSK_IN_STRING;
                        if crate::memory::cstr_byte_at(p.cast(), 1) != b'*' {
                            flags |= FSK_SIMPLIFY;
                        }
                        let mut p_const: *const u8 = p;
                        let spec_len = trans_special(
                            &raw mut p_const,
                            (arg_end as usize).wrapping_sub(p as usize),
                            end,
                            flags,
                            false,
                            None,
                        );
                        p = p_const.cast_mut();
                        if spec_len != 0 {
                            end = end.wrapping_add(spec_len as usize);
                            if (end as usize) >= (buf_ptr as usize + len) {
                                iemsg(c"eval_string() used more space than allocated");
                            }
                        } else {
                            let mut p_c = p.cast_const();
                            mb_copy_char(&raw mut p_c, &raw mut end);
                            p = p_c.cast_mut();
                        }
                    }
                    _ => {
                        let mut p_const = p.cast_const();
                        mb_copy_char(&raw mut p_const, &raw mut end);
                        p = p_const.cast_mut();
                    }
                }
            } else {
                if interpolate
                    && (crate::memory::cstr_first_byte(p.cast()) == b'{'
                        || crate::memory::cstr_first_byte(p.cast()) == b'}')
                {
                    if crate::memory::cstr_first_byte(p.cast()) == b'{'
                        && crate::memory::cstr_byte_at(p.cast(), 1) != b'{'
                    {
                        break;
                    }
                    p = p.wrapping_add(1);
                }
                let mut p_const = p.cast_const();
                mb_copy_char(&raw mut p_const, &raw mut end);
                p = p_const.cast_mut();
            }
        }
        *end = 0;
        crate::eval::typval::tv_set_string(rettv, buf_ptr.cast());
        if crate::memory::cstr_first_byte(p.cast()) == b'"' && !interpolate {
            p = p.wrapping_add(1);
        }
        *arg = p;

        true
    }
}

/// Allocate a variable for a 'str''ing' constant.
fn eval_lit_string<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    evaluate: bool,
    interpolate: bool,
) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let mut reduce: isize = if interpolate { -1 } else { 0 };
        let off: usize = if interpolate { 0 } else { 1 };

        let mut p = (*arg).wrapping_add(off);
        while !crate::memory::cstr_is_empty(p.cast()) {
            if crate::memory::cstr_first_byte(p.cast()) == b'\'' {
                if crate::memory::cstr_byte_at(p.cast(), 1) != b'\'' {
                    break;
                }
                reduce += 1;
                p = p.wrapping_add(1);
            } else if interpolate {
                if crate::memory::cstr_first_byte(p.cast()) == b'{' {
                    if crate::memory::cstr_byte_at(p.cast(), 1) != b'{' {
                        break;
                    }
                    p = p.wrapping_add(1);
                    reduce += 1;
                } else if crate::memory::cstr_first_byte(p.cast()) == b'}' {
                    p = p.wrapping_add(1);
                    if crate::memory::cstr_first_byte(p.cast()) != b'}' {
                        semsg!(gettext(E_STRAY_CLOSING_CURLY_STR), *arg);
                        return false;
                    }
                    reduce += 1;
                }
            }
            p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
        }

        if crate::memory::cstr_first_byte(p.cast()) != b'\''
            && !(interpolate && crate::memory::cstr_first_byte(p.cast()) == b'{')
        {
            semsg!(gettext(c"E115: Missing quote: %s"), *arg);
            return false;
        }

        if !evaluate {
            *arg = p.wrapping_add(off);
            return true;
        }

        let alloc_len = ((p as isize) - (*arg as isize) - reduce) as usize;
        let mut buf_vec = vec![0u8; alloc_len];
        let mut str_ptr: *mut _ = buf_vec.as_mut_ptr().cast();

        p = (*arg).wrapping_add(off);
        while !crate::memory::cstr_is_empty(p.cast()) {
            if crate::memory::cstr_first_byte(p.cast()) == b'\'' {
                if crate::memory::cstr_byte_at(p.cast(), 1) != b'\'' {
                    break;
                }
                p = p.wrapping_add(1);
            } else if interpolate
                && (crate::memory::cstr_first_byte(p.cast()) == b'{'
                    || crate::memory::cstr_first_byte(p.cast()) == b'}')
            {
                if crate::memory::cstr_first_byte(p.cast()) == b'{'
                    && crate::memory::cstr_byte_at(p.cast(), 1) != b'{'
                {
                    break;
                }
                p = p.wrapping_add(1);
            }
            let mut p_const = p.cast_const();
            mb_copy_char(&raw mut p_const, &raw mut str_ptr);
            p = p_const.cast_mut();
        }
        *str_ptr = NUL;
        crate::eval::typval::tv_set_string(rettv, buf_vec.as_ptr().cast());
        *arg = p.wrapping_add(off);

        true
    }
}

/// Evaluate a single or double quoted string possibly containing expressions.
pub fn eval_interp_string<T>(arg: *mut *mut T, rettv: &mut TypVal, evaluate: bool) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let mut ret: bool;
        let mut buf: Vec<u8> = if evaluate {
            Vec::with_capacity(80)
        } else {
            Vec::new()
        };

        *arg = (*arg).wrapping_add(1);
        let quote = i32::from(**arg);
        *arg = (*arg).wrapping_add(1);

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
                if !tv.vval.v_string().is_null() {
                    let len = crate::memory::cstr_len(tv.vval.v_string());
                    if len > 0 {
                        buf.extend_from_slice(std::slice::from_raw_parts(
                            tv.vval.v_string().cast::<u8>(),
                            len,
                        ));
                    }
                }
                tv_clear(&mut tv);
            }

            if **arg != b'{' {
                *arg = (*arg).wrapping_add(1);
                break;
            }
            let p = eval_one_expr_in_str(*arg, &mut buf, evaluate);
            if p.is_null() {
                ret = false;
                break;
            }
            *arg = p;
        }

        if ret && evaluate {
            crate::eval::typval::tv_set_string_len(rettv, buf.as_ptr().cast(), buf.len());
        } else {
            rettv.v_type = VarType::VarString;
            rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
        true
    }
}

/// @return  the function name of the partial.
pub fn partial_name(pt: Option<&TvPartial>) -> &CStr {
    unsafe {
        if let Some(pt) = pt {
            if let Some(ref name) = pt.pt_name {
                return name.as_c_str();
            }
            if !pt.pt_func.is_null() {
                let uf = pt.pt_func.cast::<UserFunc>();
                return (*uf).uf_name.as_c_str();
            }
        }
        c""
    }
}

/// Unreference a closure: decrement the reference count and free it when it
/// becomes zero.
pub fn partial_unref(pt: *mut TvPartial) {
    unsafe {
        if pt.is_null() {
            return;
        }

        (*pt).pt_refcount -= 1;
        if (*pt).pt_refcount <= 0 {
            drop(Box::from_raw(pt));
        }
    }
}

/// Allocate a variable for a List and fill it from "*arg".
fn eval_list<T>(arg: *mut *mut T, rettv: &mut TypVal, mut evalarg: Option<&mut Evalarg>) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut l: *mut TvList = ptr::null_mut();

        if evaluate {
            l = tv_list_alloc(K_LIST_LEN_SHOULD_KNOW);
        }

        *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        while **arg != b']' && **arg != 0 {
            let mut tv = tv_initial_value();
            if !eval1(arg, &mut tv, evalarg.as_deref_mut()) {
                if evaluate {
                    tv_list_free(l);
                }
                return false;
            }
            if evaluate {
                tv.v_lock = VAR_UNLOCKED;
                tv_list_append_owned_tv(&mut *l, tv);
            }

            let had_comma = **arg == b',';
            if had_comma {
                *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
            }

            if **arg == b']' {
                break;
            }

            if !had_comma {
                semsg!(gettext(c"E696: Missing comma in List: %s"), *arg);
                if evaluate {
                    tv_list_free(l);
                }
                return false;
            }
        }

        if **arg != b']' {
            semsg!(gettext(E_LIST_END), *arg);
            if evaluate {
                tv_list_free(l);
            }
            return false;
        }

        *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        if evaluate {
            tv_list_set_ret(rettv, l);
        }

        true
    }
}

pub fn func_equal(tv1: &TypVal, tv2: &TypVal, ic: bool) -> bool {
    unsafe {
        let mut s1 = if tv1.v_type == VarType::VarFunc {
            tv1.vval.v_string().cast_const()
        } else {
            partial_name(tv_partial(tv1).as_ref()).as_ptr()
        };
        if !s1.is_null() && crate::memory::cstr_is_empty(s1) {
            s1 = ptr::null();
        }
        let mut s2 = if tv2.v_type == VarType::VarFunc {
            tv2.vval.v_string().cast_const()
        } else {
            partial_name(tv_partial(tv2).as_ref()).as_ptr()
        };
        if !s2.is_null() && crate::memory::cstr_is_empty(s2) {
            s2 = ptr::null();
        }
        if s1.is_null() || s2.is_null() {
            if s1 != s2 {
                return false;
            }
        } else if crate::memory::cstr_as_bytes(s1) != crate::memory::cstr_as_bytes(s2) {
            return false;
        }

        let d1 = if tv1.v_type == VarType::VarFunc {
            ptr::null_mut()
        } else {
            (*tv_partial(tv1)).pt_dict
        };
        let d2 = if tv2.v_type == VarType::VarFunc {
            ptr::null_mut()
        } else {
            (*tv_partial(tv2)).pt_dict
        };
        if d1.is_null() || d2.is_null() {
            if d1 != d2 {
                return false;
            }
        } else if !tv_dict_equal(d1.as_ref(), d2.as_ref(), ic) {
            return false;
        }

        let a1 = if tv1.v_type == VarType::VarFunc {
            0
        } else {
            (*tv_partial(tv1)).pt_argv.len() as i32
        };
        let a2 = if tv2.v_type == VarType::VarFunc {
            0
        } else {
            (*tv_partial(tv2)).pt_argv.len() as i32
        };
        if a1 != a2 {
            return false;
        }
        for i in 0..(a1 as usize) {
            if !tv_equal(
                &(&(*tv_partial(tv1)).pt_argv)[i],
                &(&(*tv_partial(tv2)).pt_argv)[i],
                ic,
            ) {
                return false;
            }
        }
        true
    }
}

/// Get next (unique) copy ID.
pub fn get_copy_id() -> i32 {
    eval_st().current_copy_id = eval_st().current_copy_id.wrapping_add(COPYID_INC);
    eval_st().current_copy_id
}

/// Do garbage collection for lists and dicts.
pub fn garbage_collect(testing: bool) -> bool {
    let mut abort = false;

    if !testing {
        g_st().want_garbage_collect = false;
        g_st().may_garbage_collect = false;
        g_st().garbage_collect_at_exit = false;
    }

    if rt_st().exestack.capacity() - rt_st().exestack.len() > 500 {
        let keep = rt_st().exestack.len() + (rt_st().exestack.len() / 2).max(50);
        rt_st().exestack.shrink_to(keep);
    }

    let copy_id = get_copy_id();

    abort = abort || set_ref_in_previous_funccal(copy_id);
    abort = abort || garbage_collect_scriptvars(copy_id);

    {
        for buf in for_all_buffers_mut() {
            abort = abort
                || set_ref_in_item(
                    &mut buf.b_bufvar.di_tv,
                    copy_id,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            abort = abort
                || set_ref_in_callback(
                    &mut buf.b_prompt_callback,
                    copy_id,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            abort = abort
                || set_ref_in_callback(
                    &mut buf.b_prompt_interrupt,
                    copy_id,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            abort = abort
                || set_ref_in_callback(&mut buf.b_p_cfu, copy_id, ptr::null_mut(), ptr::null_mut());
            abort = abort
                || set_ref_in_callback(&mut buf.b_p_ofu, copy_id, ptr::null_mut(), ptr::null_mut());
            abort = abort
                || set_ref_in_callback(
                    &mut buf.b_p_tsrfu,
                    copy_id,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            abort = abort
                || set_ref_in_callback(&mut buf.b_p_tfu, copy_id, ptr::null_mut(), ptr::null_mut());
            abort = abort
                || set_ref_in_callback(&mut buf.b_p_ffu, copy_id, ptr::null_mut(), ptr::null_mut());
            if !abort && !buf.b_p_cpt_cb.is_empty() {
                abort = abort || set_ref_in_cpt_callbacks(&mut buf.b_p_cpt_cb, copy_id);
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
            for wp in for_all_windows_in_tab_mut(&*tp) {
                abort = abort
                    || set_ref_in_item(
                        &mut wp.w_winvar.di_tv,
                        copy_id,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
            }
        }

        for cw in &g_st().ctx_win_vec {
            let cw_win = cw.cw_win;
            if !cw_win.is_null() {
                abort = abort
                    || set_ref_in_item(
                        unsafe { &mut (*cw_win).w_winvar.di_tv },
                        copy_id,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
            }
        }
    }

    {
        for tp in for_all_tabpages_mut() {
            abort = abort
                || set_ref_in_item(
                    &mut tp.tp_winvar.di_tv,
                    copy_id,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
        }
    }

    abort = abort || garbage_collect_globvars(copy_id) != 0;
    abort = abort || set_ref_in_call_stack(copy_id);
    abort = abort || set_ref_in_functions(copy_id);

    {
        for &data in g_st().channels.values() {
            let ct = data;
            set_ref_in_callback_reader(unsafe { &mut (*ct).on_data }, copy_id, None, None);
            set_ref_in_callback_reader(unsafe { &mut (*ct).on_stderr }, copy_id, None, None);
            set_ref_in_callback(
                unsafe { &mut (*ct).on_exit },
                copy_id,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
    }

    for &timer in eval_st().timers.values() {
        set_ref_in_callback(
            unsafe { &mut (*timer).callback },
            copy_id,
            ptr::null_mut(),
            ptr::null_mut(),
        );
    }

    abort = abort || set_ref_in_func_args(copy_id);
    abort = abort || garbage_collect_vimvars(copy_id);
    {
        abort = abort || set_ref_in_quickfix(copy_id);
    }

    let mut did_free = false;
    if !abort {
        did_free = free_unref_items(copy_id) != 0;
        did_free = free_unref_funccal(copy_id, i32::from(testing)) || did_free;
    } else {
        if opt_vars().p_verbose > 0 {
            verb_msg(gettext(
                c"Not enough memory to set references, garbage collection aborted!",
            ));
        }
    }
    did_free
}

/// Free lists and dictionaries that are no longer referenced.
fn free_unref_items(copy_id: i32) -> i32 {
    let mut did_free = false;

    tv_st().tv_in_free_unref_items = true;

    for &dd in gc_dicts_slice().iter().rev() {
        if (unsafe { (*dd).dv_copy_id } & COPYID_MASK) != (copy_id & COPYID_MASK) {
            tv_dict_free_contents(unsafe { &mut *dd });
            did_free = true;
        }
    }

    for &ll in gc_lists_slice().iter().rev() {
        if (tv_list_copyid(unsafe { &*ll }) & COPYID_MASK) != (copy_id & COPYID_MASK)
            && !tv_list_has_watchers(unsafe { &*ll })
        {
            tv_list_free_contents(unsafe { &mut *ll });
            did_free = true;
        }
    }

    let mut i = gc_dicts_slice().len();
    while i > 0 {
        i -= 1;
        let dd_cur = gc_dicts_slice()[i];
        if (unsafe { (*dd_cur).dv_copy_id } & COPYID_MASK) != (copy_id & COPYID_MASK) {
            drop(unsafe { Box::from_raw(dd_cur) });
        }
    }

    let mut j = gc_lists_slice().len();
    while j > 0 {
        j -= 1;
        let ll_cur = gc_lists_slice()[j];
        if (unsafe { (*ll_cur).lv_copy_id } & COPYID_MASK) != (copy_id & COPYID_MASK)
            && !tv_list_has_watchers(unsafe { &*ll_cur })
        {
            drop(unsafe { Box::from_raw(ll_cur) });
        }
    }

    tv_st().tv_in_free_unref_items = false;
    i32::from(did_free)
}

/// Mark all lists and dicts referenced through hashtab "ht" with "copyID".
pub fn set_ref_in_ht(ht: *mut Hashtab, copy_id: i32, list_stack: *mut ListStack) -> bool {
    let mut abort = false;
    let mut ht_stack: HtStack = Vec::new();

    let mut cur_ht = ht;
    loop {
        if !abort {
            let mut todo = unsafe { (*cur_ht).ht_used };
            let mut hi = (unsafe { &*cur_ht }).array_ptr();
            while todo > 0 {
                if !hashitem_empty(unsafe { &*hi }) {
                    todo -= 1;
                    let di = tv_dict_hi2di(hi);
                    abort = abort
                        || set_ref_in_item(&mut di.di_tv, copy_id, &raw mut ht_stack, list_stack);
                }
                hi = hi.wrapping_add(1);
            }
        }

        let Some(next_ht) = ht_stack.pop() else {
            break;
        };
        cur_ht = next_ht;
    }

    abort
}

/// Mark all lists and dicts referenced through list "l" with "copyID".
pub fn set_ref_in_list_items(l: *mut TvList, copy_id: i32, ht_stack: *mut HtStack) -> bool {
    let mut abort = false;
    let mut list_stack: ListStack = Vec::new();

    let mut cur_l = l;
    loop {
        if !cur_l.is_null() {
            for li in crate::eval::typval::list_iter_mut(unsafe { (*cur_l).lv_first }) {
                if abort {
                    break;
                }
                abort = set_ref_in_item(
                    &mut *tv_list_item_tv(li),
                    copy_id,
                    ht_stack,
                    &raw mut list_stack,
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
    mut dd: Option<&mut TvDict>,
    copy_id: i32,
    ht_stack: *mut HtStack,
    list_stack: *mut ListStack,
) -> bool {
    unsafe {
        if dd.is_none() || dd.as_deref_mut().unwrap().dv_copy_id == copy_id {
            return false;
        }

        dd.as_deref_mut().unwrap().dv_copy_id = copy_id;
        if ht_stack.is_null() {
            return set_ref_in_ht(
                &raw mut dd.as_deref_mut().unwrap().dv_hashtab,
                copy_id,
                list_stack,
            );
        }

        (*ht_stack).push(&raw mut dd.as_deref_mut().unwrap().dv_hashtab);

        for watcher in &mut dd.unwrap().watchers {
            set_ref_in_callback(&mut watcher.callback, copy_id, ht_stack, list_stack);
        }

        false
    }
}

fn set_ref_in_item_list(
    ll: *mut TvList,
    copy_id: i32,
    ht_stack: *mut HtStack,
    list_stack: Option<&mut ListStack>,
) -> bool {
    if ll.is_null() || unsafe { (*ll).lv_copy_id } == copy_id {
        return false;
    }

    unsafe {
        (*ll).lv_copy_id = copy_id;
    }
    if let Some(list_stack) = list_stack {
        list_stack.push(ll);
        false
    } else {
        set_ref_in_list_items(ll, copy_id, ht_stack)
    }
}

fn set_ref_in_item_partial(
    pt: *mut TvPartial,
    copy_id: i32,
    ht_stack: *mut HtStack,
    list_stack: *mut ListStack,
) -> bool {
    unsafe {
        if pt.is_null() || (*pt).pt_copy_id == copy_id {
            return false;
        }

        (*pt).pt_copy_id = copy_id;

        let mut abort = set_ref_in_func(
            (*pt)
                .pt_name
                .as_ref()
                .map_or(ptr::null_mut(), |n| n.as_ptr().cast_mut()),
            (*pt).pt_func.cast(),
            copy_id,
        );

        if !(*pt).pt_dict.is_null() {
            let mut dtv = TypVal {
                v_type: VarType::VarDict,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Dict((*pt).pt_dict),
            };
            abort = abort || set_ref_in_item(&mut dtv, copy_id, ht_stack, list_stack);
        }

        for arg in &mut (*pt).pt_argv {
            abort = abort || set_ref_in_item(arg, copy_id, ht_stack, list_stack);
        }

        abort
    }
}

/// Mark all lists and dicts referenced through typval "tv" with "copyID".
pub fn set_ref_in_item(
    tv: &mut TypVal,
    copy_id: i32,
    ht_stack: *mut HtStack,
    list_stack: *mut ListStack,
) -> bool {
    match tv.v_type {
        VarType::VarDict => set_ref_in_item_dict(
            unsafe { tv.vval.v_dict().as_mut() },
            copy_id,
            ht_stack,
            list_stack,
        ),
        VarType::VarList => set_ref_in_item_list(tv.vval.v_list(), copy_id, ht_stack, unsafe {
            list_stack.as_mut()
        }),
        VarType::VarFunc => set_ref_in_func(tv.vval.v_string(), ptr::null_mut(), copy_id),
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
fn get_literal_key<T>(arg: *mut *mut T, tv: &mut TypVal) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        if !ascii_isalnum(i32::from(**arg)) && **arg != b'_' && **arg != b'-' {
            return false;
        }
        let mut p = *arg;
        while ascii_isalnum(crate::memory::cstr_first_char(p.cast()))
            || crate::memory::cstr_first_byte(p.cast()) == b'_'
            || crate::memory::cstr_first_byte(p.cast()) == b'-'
        {
            p = p.wrapping_add(1);
        }
        crate::eval::typval::tv_set_string_len(
            &mut *tv,
            (*arg).cast(),
            (p as usize).wrapping_sub(*arg as usize),
        );

        *arg = skipwhite(p.cast()).cast();
        true
    }
}

/// Allocate a variable for a Dictionary and fill it from "*arg".
fn eval_dict<T>(
    arg: *mut *mut T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    literal: bool,
) -> i32 {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut tv = tv_initial_value();
        let mut key = ptr::null_mut();
        let mut curly_expr = skipwhite((*arg).wrapping_add(1).cast());
        let mut buf = [0u8; NUMBUFLEN];

        if crate::memory::cstr_first_byte(curly_expr) != b'}'
            && !literal
            && eval1(&raw mut curly_expr, &mut tv, None)
            && *skipwhite(curly_expr).cast::<u8>() == b'}'
        {
            return NOTDONE;
        }

        let mut d: *mut TvDict = ptr::null_mut();
        if evaluate {
            d = tv_dict_alloc();
        }
        let mut tvkey = tv_initial_value();
        tv.v_type = VarType::VarUnknown;

        *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        while **arg != b'}' && **arg != 0 {
            let key_res = if literal {
                get_literal_key(arg, &mut tvkey)
            } else {
                eval1(arg, &mut tvkey, evalarg.as_deref_mut())
            };
            if !key_res {
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if **arg != b':' {
                semsg!(gettext(c"E720: Missing colon in Dictionary: %s"), *arg,);
                tv_clear(&mut tvkey);
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if evaluate {
                let Some(key_cstr) = tv_get_string_buf_chk(&tvkey, &mut buf) else {
                    tv_clear(&mut tvkey);
                    if !d.is_null() {
                        tv_dict_free(d);
                    }
                    return FAIL;
                };
                key = key_cstr.as_ptr().cast_mut();
            }

            *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
            if !eval1(arg, &mut tv, evalarg.as_deref_mut()) {
                tv_clear(&mut tvkey);
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if evaluate {
                let mut item = tv_dict_find(d.as_ref(), key, -1);
                if !item.is_null() {
                    semsg!(gettext(c"E721: Duplicate key in Dictionary: \"%s\""), key,);
                    tv_clear(&mut tvkey);
                    tv_clear(&mut tv);
                    if !d.is_null() {
                        tv_dict_free(d);
                    }
                    return FAIL;
                }
                item = tv_dict_item_alloc(key);
                (*item).di_tv = tv;
                (*item).di_tv.v_lock = VAR_UNLOCKED;
                if !tv_dict_add(&mut *d, item) {
                    tv_dict_item_free(item);
                }
            }
            tv_clear(&mut tvkey);

            let had_comma = **arg == b',';
            if had_comma {
                *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
            }

            if **arg == b'}' {
                break;
            }
            if !had_comma {
                semsg!(gettext(c"E722: Missing comma in Dictionary: %s"), *arg,);
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
        }

        if **arg != b'}' {
            semsg!(gettext(E_723_END), *arg,);
            if !d.is_null() {
                tv_dict_free(d);
            }
            return FAIL;
        }

        *arg = skipwhite((*arg).wrapping_add(1).cast()).cast();
        if evaluate {
            tv_dict_set_ret(rettv, d);
        }

        OK
    }
}

const E_723_END: &CStr = c"E723: Missing end of Dictionary '}': %s";

/// Evaluate a literal dictionary: #{key: val, key: val}
fn eval_lit_dict<T>(arg: *mut *mut T, rettv: &mut TypVal, evalarg: Option<&mut Evalarg>) -> i32 {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        if crate::memory::cstr_byte_at((*arg).cast(), 1) == b'{' {
            *arg = (*arg).wrapping_add(1);
            eval_dict(arg, rettv, evalarg, true)
        } else {
            NOTDONE
        }
    }
}

/// Convert the string to a floating point number.
pub fn string2float(text: impl crate::strings::IntoCCharPtr, ret_value: &mut TvFloat) -> usize {
    let text = text.into_c_char_ptr();
    {
        let b0 = (crate::memory::cstr_first_byte(text)).to_ascii_lowercase();
        let b1 = if b0 != 0 {
            (crate::memory::cstr_byte_at(text, 1)).to_ascii_lowercase()
        } else {
            0
        };
        let b2 = if b1 != 0 {
            (crate::memory::cstr_byte_at(text, 2)).to_ascii_lowercase()
        } else {
            0
        };
        if b0 == b'i' && b1 == b'n' && b2 == b'f' {
            *ret_value = f64::INFINITY;
            return 3;
        }
        if b0 == b'-' && b1 == b'i' && b2 == b'n' {
            let b3 = (crate::memory::cstr_byte_at(text, 3)).to_ascii_lowercase();
            if b3 == b'f' {
                *ret_value = f64::NEG_INFINITY;
                return 4;
            }
        }
        if b0 == b'n' && b1 == b'a' && b2 == b'n' {
            *ret_value = f64::NAN;
            return 3;
        }
        let mut s = ptr::null_mut();
        *ret_value = strtod(text, &raw mut s);
        (s as usize).wrapping_sub(text as usize)
    }
}

/// Get the value of an environment variable.
fn eval_env_var<T>(arg: *mut *mut T, rettv: &mut TypVal, evaluate: i32) -> bool {
    let arg: *mut *mut u8 = arg.cast();
    unsafe {
        *arg = (*arg).wrapping_add(1);
        let name = *arg;
        let len = get_env_len(arg.cast::<*const u8>());

        if evaluate != 0 {
            if len == 0 {
                return false;
            }
            let cc = *name.wrapping_add(len as usize);
            *name.wrapping_add(len as usize) = 0;
            let mut string = vim_getenv(name.cast()).filter(|s| !s.as_bytes().is_empty());
            if string.is_none() {
                let exp = expand_env_save(name.wrapping_sub(1).cast());
                if crate::memory::cstr_first_byte(exp.as_ptr()) != b'$' {
                    string = Some(exp);
                }
            }
            *name.wrapping_add(len as usize) = cc;
            rettv.v_lock = VAR_UNLOCKED;
            if let Some(ref s) = string {
                crate::eval::typval::tv_set_string(rettv, s.as_ptr());
            } else {
                rettv.v_type = VarType::VarString;
                rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
        }

        true
    }
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
            *cmd_out = crate::memory::cstr_to_cstring(cmd_str);
        }
        return shell_build_argv(cmd_str, ptr::null());
    }

    if cmd_tv.v_type != VarType::VarList {
        semsg!(gettext(E_INVARG2), c"expected String or List",);
        return Vec::new();
    }

    let argl = cmd_tv.vval.v_list();
    let argc = tv_list_len(unsafe { argl.as_ref() });
    if argc == 0 {
        emsg(gettext(E_INVARG));
        return Vec::new();
    }

    let arg0 = tv_get_string_chk(&*tv_list_item_tv(unsafe {
        &mut *tv_list_first(argl.as_ref())
    }));
    let mut exe_resolved: Option<std::ffi::CString> = None;
    let Some(arg0) = arg0 else {
        return Vec::new();
    };
    if !os_can_exe(arg0, Some(&mut exe_resolved), true) {
        if let Some(executable) = executable {
            let mut buf = [0u8; IOSIZE];
            vim_snprintf!(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"'%s' is not executable",
                arg0,
            );
            semsg!(gettext(E_INVARG_NVAL), c"cmd", buf.as_ptr(),);
            *executable = false;
        }
        return Vec::new();
    }
    let exe_cstr = exe_resolved.unwrap();

    let mut argv = Vec::with_capacity(argc as usize);
    for (i, arg) in crate::eval::typval::list_iter_mut(if argl.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*argl).lv_first }
    })
    .enumerate()
    {
        let Some(a) = tv_get_string_chk(&*tv_list_item_tv(arg)) else {
            return Vec::new();
        };
        argv.push(if i == 0 {
            exe_cstr.clone()
        } else {
            a.to_owned()
        });
    }
    if let Some(cmd_out) = cmd {
        *cmd_out = exe_cstr;
    }

    argv
}

fn string_to_list(mut bytes: &[u8], keepempty: bool) -> &'static mut TvList {
    if !keepempty && bytes.last() == Some(&b'\n') {
        bytes = &bytes[..bytes.len() - 1];
    }
    let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    encode_list_write(Some(&mut *list), bytes);
    list
}

fn get_system_output_as_rettv(argvars: &mut [TypVal], rettv: &mut TypVal, retlist: bool) {
    let mut wait_time: Proftime = 0;
    let profiling = g_st().do_profiling == PROF_YES;

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

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

    if opt_vars().p_verbose > 3 {
        let cmdstr = shell_argv_to_str(&argv);
        verbose_enter_scroll();
        smsg!(0, gettext(c"Executing command: \"%s\""), cmdstr.as_ptr());
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
            crate::eval::typval::tv_set_string(rettv, c"");
        }
        return;
    }

    if retlist {
        let mut keepempty: i32 = 0;
        if argvars[1].v_type != VarType::VarUnknown && argvars[2].v_type != VarType::VarUnknown {
            keepempty = tv_get_number(&argvars[2]) as i32;
        }
        rettv.vval = crate::eval::typval::TypvalVval::List(string_to_list(&res, keepempty != 0));
        tv_list_ref(unsafe { rettv.vval.v_list().as_mut() });
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
        crate::eval::typval::tv_set_string_len(rettv, res.as_ptr().cast(), res.len());
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
pub fn callback_from_typval(callback: &mut Callback, arg: &TypVal) -> bool {
    let mut r = OK;

    let is_lua_tbl = nlua_is_table_from_lua(arg);

    if arg.v_type == VarType::VarPartial && !tv_partial(arg).is_null() {
        let pt = tv_partial(arg);
        callback.data = crate::eval::typval::CallbackData::partial(pt);
        unsafe {
            (*pt).pt_refcount += 1;
        }
    } else if arg.v_type == VarType::VarString
        && !arg.vval.v_string().is_null()
        && ascii_isdigit(crate::memory::cstr_first_char(arg.vval.v_string()))
    {
        r = FAIL;
    } else if arg.v_type == VarType::VarFunc || arg.v_type == VarType::VarString {
        let name = arg.vval.v_string();
        if name.is_null() {
            r = FAIL;
        } else if crate::memory::cstr_is_empty(name) {
            *callback = Callback::NONE;
        } else {
            let funcref = if arg.v_type == VarType::VarString
                && let Some(sl) = get_scriptlocal_funcname(name)
            {
                sl
            } else {
                crate::memory::cstr_to_cstring(name)
            };
            func_ref(funcref.as_ptr().cast_mut());
            callback.data = crate::eval::typval::CallbackData::funcref(funcref);
        }
    } else if is_lua_tbl {
        if let Some(name) = nlua_register_table_as_callable(arg) {
            callback.data = crate::eval::typval::CallbackData::funcref(name.to_owned());
        } else {
            r = FAIL;
        }
    } else if arg.v_type == VarType::VarSpecial
        || (arg.v_type == VarType::VarNumber && arg.vval.v_number() == 0)
    {
        *callback = Callback::NONE;
    } else {
        r = FAIL;
    }

    if r == FAIL {
        emsg(gettext(c"E921: Invalid callback argument"));
        return false;
    }
    true
}

pub fn get_callback_depth() -> i32 {
    eval_st().callback_depth
}

/// @return  whether the callback could be called.
pub fn callback_call(
    callback: &mut Callback,
    argcount_in: i32,
    argvars_in: *mut TypVal,
    rettv: &mut TypVal,
) -> bool {
    if i64::from(eval_st().callback_depth) > opt_vars().p_mfd {
        emsg(gettext(E_COMMAND_TOO_RECURSIVE));
        return false;
    }

    let partial: *mut TvPartial;
    let mut name;
    match callback.cb_type() {
        K_CALLBACK_FUNCREF => {
            let funcref = callback.data.as_funcref().unwrap_or(c"");
            name = funcref.as_ptr().cast_mut();
            if funcref.to_bytes().starts_with(b"v:lua.") {
                name = name.wrapping_add(6);
                if check_luafunc_name(name, false) == 0 {
                    return false;
                }
                partial = get_vim_var_partial(VV_LUA);
            } else {
                partial = ptr::null_mut();
            }
        }
        K_CALLBACK_PARTIAL => {
            partial = callback.data.as_partial();
            name = partial_name(unsafe { partial.as_ref() })
                .as_ptr()
                .cast_mut();
        }
        K_CALLBACK_LUA => {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let mut luaargs = arena_array(&mut arena, argcount_in as usize);
            for i in 0..(argcount_in as usize) {
                *luaargs.next_slot_mut() =
                    vim_to_object(argvars_in.wrapping_add(i), Some(&mut arena), false);
                luaargs.size += 1;
            }
            let mut err = Error::new();
            let luaref = callback.data.as_luaref();
            eval_st().callback_depth += 1;
            let result = nlua_call_ref(
                luaref,
                ptr::null(),
                luaargs,
                K_RET_OBJECT,
                Some(&mut arena),
                Some(&mut err),
            );
            eval_st().callback_depth -= 1;
            let ok = err.err_type == ErrorType::ErrorTypeNone;
            if !ok {
                semsg_multiline!(c"emsg", c"E5108: %s", err.msg_cstr());
                api_clear_error(&mut err);
            } else {
                object_to_vim(
                    result,
                    rettv,
                    &mut crate::api::private::validate::Error::new(),
                );
            }
            arena_mem_free(arena_finish(&mut arena));
            return ok;
        }
        K_CALLBACK_EXPR => {
            eval_st().callback_depth += 1;
            let r = eval0_simple_funccal(
                skipwhite(callback.data.as_funcref()),
                rettv,
                None,
                &mut g_st().evalarg_evaluate,
            );
            eval_st().callback_depth -= 1;
            return r;
        }
        _ => {
            return false;
        }
    }

    let lnum = curwin_mut().w_cursor.lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: partial.cast(),
        fe_selfdict: ptr::null_mut(),
        fe_basetv: ptr::null_mut(),
        fe_found_var: false,
    };

    eval_st().callback_depth += 1;
    let ret = call_func(name, -1, rettv, argcount_in, argvars_in, &mut funcexe);
    eval_st().callback_depth -= 1;
    ret
}

pub fn set_ref_in_callback(
    callback: &mut Callback,
    copy_id: i32,
    ht_stack: *mut HtStack,
    list_stack: *mut ListStack,
) -> bool {
    if callback.cb_type() == K_CALLBACK_PARTIAL {
        let mut tv = TypVal {
            v_type: VarType::VarPartial,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Partial(callback.data.as_partial()),
        };
        return set_ref_in_item(&mut tv, copy_id, ht_stack, list_stack);
    }
    false
}

fn set_ref_in_callback_reader(
    reader: &mut CallbackReader,
    copy_id: i32,
    ht_stack: Option<&mut HtStack>,
    list_stack: Option<&mut ListStack>,
) -> bool {
    let ht_ptr = ht_stack.map_or(ptr::null_mut(), |p| &raw mut *p);
    let list_ptr = list_stack.map_or(ptr::null_mut(), |p| &raw mut *p);
    if set_ref_in_callback(&mut reader.cb, copy_id, ht_ptr, list_ptr) {
        return true;
    }

    if !reader.self_dict.is_null() {
        let mut tv = TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(reader.self_dict),
        };
        return set_ref_in_item(&mut tv, copy_id, ht_ptr, list_ptr);
    }
    false
}

pub fn find_timer_by_nr(xx: Varnumber) -> *mut VimTimer {
    eval_st()
        .timers
        .get(&(xx as u64))
        .copied()
        .unwrap_or(ptr::null_mut())
}

pub fn add_timer_info(rettv: &mut TypVal, timer: &mut VimTimer) {
    let list = rettv.vval.v_list();
    let dict = tv_dict_alloc();

    tv_list_append_dict(unsafe { &mut *list }, dict);
    tv_dict_add_nr(&mut *dict, c"id", Varnumber::from(timer.timer_id));
    tv_dict_add_nr(&mut *dict, c"time", timer.timeout);
    tv_dict_add_nr(&mut *dict, c"paused", Varnumber::from(timer.paused));
    tv_dict_add_nr(
        &mut *dict,
        c"repeat",
        if timer.repeat_count < 0 {
            -1
        } else {
            Varnumber::from(timer.repeat_count)
        },
    );

    let di = tv_dict_item_alloc(c"callback");
    if !tv_dict_add(&mut *dict, di) {
        tv_dict_item_free(di);
    } else {
        callback_put(&mut timer.callback, &mut di.di_tv);
    }
}

pub fn add_timer_info_all(rettv: &mut TypVal) {
    unsafe {
        tv_list_alloc_ret(rettv, eval_st().timers.len() as isize);
        for &timer in eval_st().timers.values() {
            if !(*timer).stopped || (*timer).refcount > 1 {
                add_timer_info(rettv, &mut *timer);
            }
        }
    }
}

/// Invoked on the main loop.
pub fn timer_due_cb(_tw: *mut TimeWatcher, timer: *mut VimTimer) {
    unsafe {
        let save_did_emsg = g_st().did_emsg;
        let called_emsg_before = g_st().called_emsg;

        if (*timer).stopped || (*timer).paused {
            return;
        }

        (*timer).refcount += 1;
        if (*timer).repeat_count >= 0 {
            (*timer).repeat_count -= 1;
            if (*timer).repeat_count == 0 {
                timer_stop(&mut *timer);
            }
        }

        let mut argv = [tv_initial_value(), tv_initial_value()];
        argv[0].v_type = VarType::VarNumber;
        argv[0].vval = crate::eval::typval::TypvalVval::Number(Varnumber::from((*timer).timer_id));
        let mut rettv = tv_initial_value();

        callback_call(&mut (*timer).callback, 1, argv.as_mut_ptr(), &mut rettv);

        if g_st().called_emsg > called_emsg_before && g_st().did_emsg != 0 {
            (*timer).emsg_count += 1;
            if g_st().did_throw {
                discard_current_exception();
            }
        }
        g_st().did_emsg = save_did_emsg;

        if (*timer).emsg_count >= 3 {
            timer_stop(&mut *timer);
        }

        tv_clear(&mut rettv);

        if !(*timer).stopped && (*timer).timeout == 0 {
            time_watcher_start(&mut (*timer).watcher, 0, 0);
        }
        timer_decref(timer);
    }
}

pub fn timer_start(timeout: i64, repeat_count: i32, callback: Callback) -> u64 {
    let id = eval_st().last_timer_id;
    eval_st().last_timer_id += 1;
    let timer = Box::leak(Box::new(VimTimer {
        watcher: TimeWatcher::new(),
        timer_id: id as i32,
        repeat_count,
        refcount: 1,
        emsg_count: 0,
        timeout,
        stopped: false,
        paused: false,
        callback,
    }));

    {
        let main_loop_ptr = (&raw mut g_st().main_loop).cast();
        let timer_ptr: *mut VimTimer = timer;
        time_watcher_init(
            unsafe { &mut *main_loop_ptr },
            &mut timer.watcher,
            TimeWatcherData::EvalTimer(timer_ptr),
        );
        timer.watcher.events =
            multiqueue_new_child(loop_queues(unsafe { &mut *main_loop_ptr }).events);
        timer.watcher.blockable = true;
        time_watcher_start(&mut timer.watcher, timeout as u64, timeout as u64);
    }

    eval_st().timers.insert(timer.timer_id as u64, timer);
    timer.timer_id as u64
}

pub fn timer_stop(timer: &mut VimTimer) {
    if timer.stopped {
        return;
    }
    timer.stopped = true;
    {
        time_watcher_stop(&mut timer.watcher);
        time_watcher_close(&mut timer.watcher);
    }
}

pub(crate) fn timer_close_cb(_tw: *mut TimeWatcher, timer: *mut VimTimer) {
    unsafe {
        let _ = eval_st().timers.remove(&((*timer).timer_id as u64));
        timer_decref(timer);
    }
}

fn timer_decref(timer: *mut VimTimer) {
    unsafe {
        (*timer).refcount -= 1;
        if (*timer).refcount == 0 {
            drop(Box::from_raw(timer));
        }
    }
}

pub fn timer_stop_all() {
    let all_timers: Vec<*mut VimTimer> = eval_st().timers.values().copied().collect();
    for timer in all_timers {
        timer_stop(unsafe { &mut *timer });
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
        let buf = buflist_findnr(tv.vval.v_number() as i32);
        let mut total_len: usize = 0;
        if !buf.is_null() {
            for lnum in 1..=unsafe { (*buf).b_ml.ml_line_count } {
                let p = ml_get_buf(unsafe { &mut *buf }, lnum);
                total_len += crate::memory::cstr_len(p) + 1;
            }
        } else {
            semsg!(gettext(E_NOBUFNR), tv.vval.v_number());
            return None;
        }

        if total_len == 0 {
            return Some(Vec::new());
        }

        let mut ret = Vec::with_capacity(total_len);
        for lnum in 1..=unsafe { (*buf).b_ml.ml_line_count } {
            let p = ml_get_buf(unsafe { &mut *buf }, lnum);
            for &p_byte in crate::memory::cstr_as_bytes(p) {
                ret.push(if p_byte == b'\n' { 0 } else { p_byte });
            }
            ret.push(b'\n');
        }
        return Some(ret);
    }

    debug_assert!(tv.v_type == VarType::VarList);
    let list = tv.vval.v_list();
    let sep_len = if crlf { 2usize } else { 1usize };
    let mut total_len: usize = 0;
    let mut li = if list.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*list).lv_first }
    };
    while !li.is_null() {
        total_len += tv_get_string(&*tv_list_item_tv(unsafe { &mut *li }))
            .to_bytes()
            .len()
            + sep_len;
        li = unsafe { (*li).li_next };
    }

    if total_len == 0 {
        return Some(Vec::new());
    }

    let mut ret = Vec::with_capacity(total_len);
    li = if list.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*list).lv_first }
    };
    while !li.is_null() {
        let s = tv_get_string(&*tv_list_item_tv(unsafe { &mut *li }));
        for &s_byte in s.to_bytes() {
            ret.push(if s_byte == b'\n' { 0 } else { s_byte });
        }
        if endnl || !tv_list_item_next(unsafe { &*li }).is_null() {
            if crlf {
                ret.push(b'\r');
            }
            ret.push(b'\n');
        }
        li = unsafe { (*li).li_next };
    }
    Some(ret)
}

/// Convert the specified byte index of line 'lnum' in buffer 'buf' to a
/// character index.
pub fn buf_byteidx_to_charidx(
    mut buf: Option<&mut FileBuf>,
    mut lnum: Linenr,
    byteidx: i32,
) -> i32 {
    if buf.is_none() || buf.as_deref_mut().unwrap().b_ml.ml_mfp.is_none() {
        return -1;
    }

    if lnum > buf.as_deref_mut().unwrap().b_ml.ml_line_count {
        lnum = buf.as_deref_mut().unwrap().b_ml.ml_line_count;
    }

    let str_ptr = ml_get_buf(buf.unwrap(), lnum);

    if crate::memory::cstr_is_empty(str_ptr) {
        return 0;
    }

    let mut t = str_ptr;
    let mut count: i32 = 0;
    let limit = (str_ptr as isize) + (byteidx as isize);
    while !crate::memory::cstr_is_empty(t) && (t as isize) <= limit {
        t = t.wrapping_add(utfc_ptr2len(t) as usize);
        count += 1;
    }

    if crate::memory::cstr_is_empty(t) && byteidx != 0 && (t as isize) == limit {
        count += 1;
    }

    count - 1
}

/// Convert the specified character index of line 'lnum' in buffer 'buf' to a
/// byte index.
pub fn buf_charidx_to_byteidx(
    mut buf: Option<&mut FileBuf>,
    mut lnum: Linenr,
    mut charidx: i32,
) -> i32 {
    if buf.is_none() || buf.as_deref_mut().unwrap().b_ml.ml_mfp.is_none() {
        return -1;
    }

    if lnum > buf.as_deref_mut().unwrap().b_ml.ml_line_count {
        lnum = buf.as_deref_mut().unwrap().b_ml.ml_line_count;
    }

    let str_ptr = ml_get_buf(buf.unwrap(), lnum);

    let mut t = str_ptr;
    while !crate::memory::cstr_is_empty(t) {
        charidx -= 1;
        if charidx <= 0 {
            break;
        }
        t = t.wrapping_add(utfc_ptr2len(t) as usize);
    }

    ((t as usize).wrapping_sub(str_ptr as usize)) as i32
}

/// Translate a Vimscript object into a position.
pub fn var2fpos(
    tv: &TypVal,
    dollar_lnum: bool,
    ret_fnum: &mut i32,
    charcol: bool,
    wp: &mut WinInfoInfo,
) -> *mut CursorPos {
    unsafe {
        let bp = wp.w_buffer;

        if tv.v_type == VarType::VarList {
            let mut error = false;

            let l = tv.vval.v_list();
            if l.is_null() {
                return ptr::null_mut();
            }

            eval_st().var2_fpos_pos.lnum =
                tv_list_find_nr(l.as_mut(), 0, Some(&mut error)) as Linenr;
            if error
                || eval_st().var2_fpos_pos.lnum <= 0
                || eval_st().var2_fpos_pos.lnum > (*bp).b_ml.ml_line_count
            {
                return ptr::null_mut();
            }

            eval_st().var2_fpos_pos.col = tv_list_find_nr(l.as_mut(), 1, Some(&mut error)) as Colnr;
            if error {
                return ptr::null_mut();
            }
            let len = if charcol {
                mb_charlen(ml_get_buf(&mut *bp, eval_st().var2_fpos_pos.lnum))
            } else {
                ml_get_buf_len(&mut *bp, eval_st().var2_fpos_pos.lnum)
            };

            let li = tv_list_find(l.as_mut(), 1);
            if !li.is_null()
                && tv_list_item_tv(&mut *li).v_type == VarType::VarString
                && !tv_list_item_tv(&mut *li).vval.v_string().is_null()
                && (crate::memory::cstr_as_bytes(tv_list_item_tv(&mut *li).vval.v_string()) == b"$")
            {
                eval_st().var2_fpos_pos.col = len + 1;
            }

            if eval_st().var2_fpos_pos.col == 0 || eval_st().var2_fpos_pos.col > len + 1 {
                return ptr::null_mut();
            }
            eval_st().var2_fpos_pos.col -= 1;

            eval_st().var2_fpos_pos.coladd =
                tv_list_find_nr(l.as_mut(), 2, Some(&mut error)) as Colnr;
            if error {
                eval_st().var2_fpos_pos.coladd = 0;
            }

            return &raw mut eval_st().var2_fpos_pos;
        }

        let Some(name_cstr) = tv_get_string_chk(tv) else {
            return ptr::null_mut();
        };
        let name = name_cstr.as_ptr();

        eval_st().var2_fpos_pos.lnum = 0;
        if crate::memory::cstr_first_byte(name) == b'.' {
            eval_st().var2_fpos_pos = CursorPos {
                lnum: wp.w_cursor.lnum,
                col: wp.w_cursor.col,
                coladd: wp.w_cursor.coladd,
            };
        } else if crate::memory::cstr_first_byte(name) == b'v'
            && crate::memory::cstr_byte_at(name, 1) == 0
        {
            if g_st().visual_global.active && ptr::eq(wp, curwin_ptr()) {
                eval_st().var2_fpos_pos = CursorPos {
                    lnum: g_st().visual_global.start.lnum,
                    col: g_st().visual_global.start.col,
                    coladd: g_st().visual_global.start.coladd,
                };
            } else {
                eval_st().var2_fpos_pos = CursorPos {
                    lnum: wp.w_cursor.lnum,
                    col: wp.w_cursor.col,
                    coladd: wp.w_cursor.coladd,
                };
            }
        } else if crate::memory::cstr_first_byte(name) == b'\'' {
            let mname = i32::from(crate::memory::cstr_byte_at(name, 1));
            let fm = mark_get(&mut *bp, wp, None, K_MARK_ALL, mname);
            if fm.is_null() || (*fm).mark.lnum <= 0 {
                return ptr::null_mut();
            }
            eval_st().var2_fpos_pos = (*fm).mark;
            if ascii_isupper(crate::memory::cstr_char_at(name, 1))
                || ascii_isdigit(crate::memory::cstr_char_at(name, 1))
            {
                *ret_fnum = (*fm).fnum;
            }
        }
        if eval_st().var2_fpos_pos.lnum != 0 {
            if charcol {
                eval_st().var2_fpos_pos.col = buf_byteidx_to_charidx(
                    bp.as_mut(),
                    eval_st().var2_fpos_pos.lnum,
                    eval_st().var2_fpos_pos.col,
                );
            }
            return &raw mut eval_st().var2_fpos_pos;
        }

        eval_st().var2_fpos_pos.coladd = 0;

        if crate::memory::cstr_first_byte(name) == b'w' && dollar_lnum {
            check_cursor_moved(wp);

            eval_st().var2_fpos_pos.col = 0;
            if crate::memory::cstr_byte_at(name, 1) == b'0' {
                update_topline(wp);
                eval_st().var2_fpos_pos.lnum = if wp.w_topline > 0 { wp.w_topline } else { 1 };
                return &raw mut eval_st().var2_fpos_pos;
            } else if crate::memory::cstr_byte_at(name, 1) == b'$' {
                validate_botline_win(&mut *wp);
                eval_st().var2_fpos_pos.lnum = if wp.w_botline > 0 {
                    wp.w_botline - 1
                } else {
                    0
                };
                return &raw mut eval_st().var2_fpos_pos;
            }
        } else if crate::memory::cstr_first_byte(name) == b'$' {
            if dollar_lnum {
                eval_st().var2_fpos_pos.lnum = (*bp).b_ml.ml_line_count;
                eval_st().var2_fpos_pos.col = 0;
            } else {
                eval_st().var2_fpos_pos.lnum = wp.w_cursor.lnum;
                eval_st().var2_fpos_pos.col = if charcol {
                    mb_charlen(ml_get_buf(&mut *bp, wp.w_cursor.lnum))
                } else {
                    ml_get_buf_len(&mut *bp, wp.w_cursor.lnum)
                };
            }
            return &raw mut eval_st().var2_fpos_pos;
        }
        ptr::null_mut()
    }
}

/// Convert list in "arg" into position "posp" and optional file number "fnump".
pub fn list2fpos(
    arg: &mut TypVal,
    posp: &mut CursorPos,
    mut fnump: Option<&mut i32>,
    mut curswantp: Option<&mut Colnr>,
    charcol: bool,
) -> bool {
    unsafe {
        if arg.v_type != VarType::VarList {
            return false;
        }
        let l = arg.vval.v_list();
        if l.is_null()
            || tv_list_len(l.as_ref()) < if fnump.is_none() { 2 } else { 3 }
            || tv_list_len(l.as_ref()) > if fnump.is_none() { 4 } else { 5 }
        {
            return false;
        }

        let mut i: i32 = 0;
        let mut n: i32;
        if fnump.is_some() {
            n = tv_list_find_nr(l.as_mut(), i, None) as i32;
            i += 1;
            if n < 0 {
                return false;
            }
            if n == 0 {
                n = curbuf_mut().b_fnum;
            }
            *fnump.as_deref_mut().unwrap() = n;
        }

        n = tv_list_find_nr(l.as_mut(), i, None) as i32;
        i += 1;
        if n < 0 {
            return false;
        }
        posp.lnum = n;

        n = tv_list_find_nr(l.as_mut(), i, None) as i32;
        i += 1;
        if n < 0 {
            return false;
        }
        if charcol {
            let buf = buflist_findnr(if let Some(fnump) = fnump {
                *fnump
            } else {
                curbuf_mut().b_fnum
            });
            if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
                return false;
            }
            let cur_lnum = curwin_mut().w_cursor.lnum;
            n = buf_charidx_to_byteidx(
                buf.as_mut(),
                if posp.lnum == 0 { cur_lnum } else { posp.lnum },
                n,
            ) + 1;
        }
        posp.col = n;

        n = tv_list_find_nr(l.as_mut(), i, None) as i32;
        if n < 0 {
            posp.coladd = 0;
        } else {
            posp.coladd = n;
        }

        if let Some(ref mut p) = curswantp {
            **p = tv_list_find_nr(l.as_mut(), i + 1, None) as Colnr;
        }

        true
    }
}

/// Get the length of an environment variable name.
pub fn get_env_len<T>(arg: *mut *const T) -> i32 {
    unsafe {
        let len = env_name_len((*arg).cast());
        *arg = (*arg).wrapping_add(len);
        len as i32
    }
}

/// Get the length of the name of a function or internal variable.
pub fn get_id_len<T>(arg: *mut *const T) -> i32 {
    let arg: *mut *const u8 = arg.cast();
    unsafe {
        let mut p = *arg;
        while eval_isnamec(i32::from(crate::memory::cstr_first_byte(p.cast()))) {
            if crate::memory::cstr_first_byte(p.cast()) == b':' {
                let len = ((p as usize).wrapping_sub(*arg as usize)) as i32;
                if len > 1 || (len == 1 && vim_strchr(NAMESPACE_CHAR, i32::from(**arg)).is_null()) {
                    break;
                }
            }
            p = p.wrapping_add(1);
        }
        if p == *arg {
            return 0;
        }

        let len = ((p as usize).wrapping_sub(*arg as usize)) as i32;
        *arg = skipwhite(p.cast()).cast();

        len
    }
}

/// Get the length of the name of a variable or function.
pub fn get_name_len<T>(
    arg: *mut *const T,
    alias: &mut Option<std::ffi::CString>,
    evaluate: bool,
    verbose: bool,
) -> i32 {
    let arg: *mut *const u8 = arg.cast();
    unsafe {
        *alias = None;

        if **arg == K_SPECIAL
            && *(*arg).wrapping_add(1) == KS_EXTRA
            && *(*arg).wrapping_add(2) == KE_SNR
        {
            *arg = (*arg).wrapping_add(3);
            return get_id_len(arg) + 3;
        }
        let mut len = eval_fname_script((*arg).cast());
        if len > 0 {
            *arg = (*arg).wrapping_add(len as usize);
        }

        let mut expr_start: *const u8 = ptr::null();
        let mut expr_end: *const u8 = ptr::null();
        let p = find_name_end(
            *arg,
            &raw mut expr_start,
            &raw mut expr_end,
            if len > 0 { 0 } else { FNE_CHECK_START },
        );
        if !expr_start.is_null() {
            if !evaluate {
                len += ((p as usize).wrapping_sub(*arg as usize)) as i32;
                *arg = skipwhite(p.cast()).cast();
                return len;
            }

            let Some(temp_string) = make_expanded_name(
                (*arg).wrapping_sub(len as usize).cast(),
                expr_start.cast_mut(),
                expr_end.cast_mut(),
                p.cast_mut(),
            ) else {
                return -1;
            };
            let ret_len = temp_string.as_bytes().len() as i32;
            *alias = Some(temp_string);
            *arg = skipwhite(p.cast()).cast();
            return ret_len;
        }

        len += get_id_len(arg);
        if len == 0 && verbose && **arg != 0 {
            semsg!(gettext(E_INVEXPR2), *arg);
        }

        len
    }
}

/// Find the end of a variable or function name, taking care of magic braces.
pub fn find_name_end<T>(
    arg: *const T,
    expr_start: *mut *const T,
    expr_end: *mut *const T,
    flags: i32,
) -> *const T {
    let arg: *const u8 = arg.cast();
    let expr_start: *mut *const u8 = expr_start.cast();
    let expr_end: *mut *const u8 = expr_end.cast();
    unsafe {
        if !expr_start.is_null() {
            *expr_start = ptr::null();
            *expr_end = ptr::null();
        }

        if (flags & FNE_CHECK_START) != 0
            && !eval_isnamec1(i32::from(crate::memory::cstr_first_byte(arg.cast())))
            && crate::memory::cstr_first_byte(arg.cast()) != b'{'
        {
            return arg.cast();
        }

        let mut mb_nest: i32 = 0;
        let mut br_nest: i32 = 0;

        let mut p = arg;
        while !crate::memory::cstr_is_empty(p.cast())
            && (eval_isnamec(i32::from(crate::memory::cstr_first_byte(p.cast())))
                || crate::memory::cstr_first_byte(p.cast()) == b'{'
                || ((flags & FNE_INCL_BR) != 0
                    && (crate::memory::cstr_first_byte(p.cast()) == b'['
                        || (crate::memory::cstr_first_byte(p.cast()) == b'.'
                            && eval_isdictc(i32::from(crate::memory::cstr_byte_at(p.cast(), 1))))))
                || mb_nest != 0
                || br_nest != 0)
        {
            if crate::memory::cstr_first_byte(p.cast()) == b'\'' {
                p = p.wrapping_add(1);
                while !crate::memory::cstr_is_empty(p.cast())
                    && crate::memory::cstr_first_byte(p.cast()) != b'\''
                {
                    p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
                }
                if crate::memory::cstr_is_empty(p.cast()) {
                    break;
                }
            } else if crate::memory::cstr_first_byte(p.cast()) == b'"' {
                p = p.wrapping_add(1);
                while !crate::memory::cstr_is_empty(p.cast())
                    && crate::memory::cstr_first_byte(p.cast()) != b'"'
                {
                    if crate::memory::cstr_first_byte(p.cast()) == b'\\'
                        && crate::memory::cstr_byte_at(p.cast(), 1) != 0
                    {
                        p = p.wrapping_add(1);
                    }
                    p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
                }
                if crate::memory::cstr_is_empty(p.cast()) {
                    break;
                }
            } else if br_nest == 0
                && mb_nest == 0
                && crate::memory::cstr_first_byte(p.cast()) == b':'
            {
                let len = ((p as usize).wrapping_sub(arg as usize)) as i32;
                if (len > 1 && *p.wrapping_sub(1) != b'}')
                    || (len == 1
                        && vim_strchr(
                            NAMESPACE_CHAR,
                            i32::from(crate::memory::cstr_first_byte(arg.cast())),
                        )
                        .is_null())
                {
                    break;
                }
            }

            if mb_nest == 0 {
                if crate::memory::cstr_first_byte(p.cast()) == b'[' {
                    br_nest += 1;
                } else if crate::memory::cstr_first_byte(p.cast()) == b']' {
                    br_nest -= 1;
                }
            }

            if br_nest == 0 {
                if crate::memory::cstr_first_byte(p.cast()) == b'{' {
                    mb_nest += 1;
                    if !expr_start.is_null() && (*expr_start).is_null() {
                        *expr_start = p;
                    }
                } else if crate::memory::cstr_first_byte(p.cast()) == b'}' {
                    mb_nest -= 1;
                    if !expr_start.is_null() && mb_nest == 0 && (*expr_end).is_null() {
                        *expr_end = p;
                    }
                }
            }

            p = p.wrapping_add(utfc_ptr2len(p.cast()) as usize);
        }

        p.cast()
    }
}

/// Expands out the 'magic' {}'s in a variable/function name.
fn make_expanded_name(
    in_start: impl crate::strings::IntoCCharPtr,
    mut expr_start: *mut u8,
    mut expr_end: *mut u8,
    in_end: *mut u8,
) -> Option<std::ffi::CString> {
    let in_start = in_start.into_c_char_ptr();
    {
        if expr_end.is_null() || in_end.is_null() {
            return None;
        }

        let mut retval: Option<std::ffi::CString> = None;

        unsafe {
            *expr_start = 0;
            *expr_end = 0;
        }
        let c1 = unsafe { *in_end };
        unsafe {
            *in_end = 0;
        }

        if let Some(temp_result) = eval_to_string(expr_start.wrapping_add(1).cast(), false, false) {
            let retvalsize = (expr_start as usize).wrapping_sub(in_start as usize)
                + temp_result.as_bytes().len()
                + (in_end as usize).wrapping_sub(expr_end as usize)
                + 1;
            let mut buf = vec![0u8; retvalsize];
            vim_snprintf!(
                buf.as_mut_ptr().cast(),
                retvalsize,
                c"%s%s%s",
                in_start,
                temp_result.as_ptr(),
                expr_end.wrapping_add(1),
            );
            retval = Some(cstr_to_cstring(buf.as_ptr().cast()));
        }

        unsafe {
            *in_end = c1;
            *expr_start = b'{';
            *expr_end = b'}';
        }

        if let Some(ref rv) = retval {
            let mut rv_vec: Vec<u8> = rv.as_bytes_with_nul().to_vec();
            let rv_ptr = rv_vec.as_mut_ptr();
            let end_ptr = find_name_end(
                rv_ptr,
                (&raw mut expr_start).cast::<*const u8>(),
                (&raw mut expr_end).cast::<*const u8>(),
                0,
            )
            .cast_mut();
            if !expr_start.is_null() {
                retval = make_expanded_name(rv_ptr.cast(), expr_start, expr_end, end_ptr);
            }
        }

        retval
    }
}

pub fn eval_isnamec(c: i32) -> bool {
    let ch = c as u8;
    ascii_isalnum(i32::from(ch)) || ch == b'_' || ch == b':' || ch == AUTOLOAD_CHAR
}

pub fn eval_isnamec1(c: i32) -> bool {
    let ch = c as u8;
    ascii_isalpha(i32::from(ch)) || ch == b'_'
}

pub fn eval_isdictc(c: i32) -> bool {
    let ch = c as u8;
    ascii_isalnum(i32::from(ch)) || ch == b'_'
}

/// Set the v:argv list.
pub fn set_argv_var<T>(argv: *mut *mut T, argc: i32) {
    unsafe {
        let l = tv_list_alloc(argc as isize);

        tv_list_set_lock(Some(&mut *l), VAR_FIXED);
        for i in 0..(argc as usize) {
            tv_list_append_string(&mut *l, (*argv.wrapping_add(i)).cast(), -1);
            tv_list_item_tv(&mut *tv_list_last(Some(&*l))).v_lock = VAR_FIXED;
        }
        set_vim_var_list(VV_ARGV, l);
    }
}

/// Check if special v:lua value for calling lua functions.
pub fn is_luafunc(partial: *mut TvPartial) -> bool {
    partial == get_vim_var_partial(VV_LUA)
}

fn tv_is_luafunc(tv: &mut TypVal) -> bool {
    tv.v_type == VarType::VarPartial && is_luafunc(tv_partial(tv))
}

/// Skips one character past the end of the name of a v:lua function.
pub fn skip_luafunc_name(p: impl crate::strings::IntoCCharPtr) -> *const u8 {
    let mut p: *const u8 = p.into_c_char_ptr().cast();
    {
        while ascii_isalnum(crate::memory::cstr_first_char(p.cast()))
            || crate::memory::cstr_first_byte(p.cast()) == b'_'
            || crate::memory::cstr_first_byte(p.cast()) == b'-'
            || crate::memory::cstr_first_byte(p.cast()) == b'.'
            || crate::memory::cstr_first_byte(p.cast()) == b'\''
        {
            p = p.wrapping_add(1);
        }
        p
    }
}

/// Check the function name after "v:lua."
pub fn check_luafunc_name(str_: impl crate::strings::IntoCCharPtr, paren: bool) -> i32 {
    let str_: *const u8 = str_.into_c_char_ptr().cast();
    {
        let p = skip_luafunc_name(str_.cast());
        if unsafe { *p } != if paren { b'(' } else { 0 } {
            return 0;
        }
        ((p as usize).wrapping_sub(str_ as usize)) as i32
    }
}

/// Return the character "str[index]" where "index" is the character index,
/// including composing characters.
pub fn char_from_string(
    str_: impl crate::strings::IntoCCharPtr,
    index: Varnumber,
) -> Option<std::ffi::CString> {
    let str_ = str_.into_c_char_ptr();
    {
        let mut nchar = index;

        if str_.is_null() {
            return None;
        }
        let slen = crate::memory::cstr_len(str_);

        if index < 0 {
            let mut clen: Varnumber = 0;
            let mut nbyte: usize = 0;
            while nbyte < slen {
                nbyte += utfc_ptr2len(str_.wrapping_add(nbyte)) as usize;
                clen += 1;
            }
            nchar = clen + index;
            if nchar < 0 {
                return None;
            }
        }

        let mut nbyte: usize = 0;
        while nchar > 0 && nbyte < slen {
            nbyte += utfc_ptr2len(str_.wrapping_add(nbyte)) as usize;
            nchar -= 1;
        }
        if nbyte >= slen {
            return None;
        }
        Some(cstr_from_bytes(unsafe {
            std::slice::from_raw_parts(
                str_.wrapping_add(nbyte).cast::<u8>(),
                utfc_ptr2len(str_.wrapping_add(nbyte)) as usize,
            )
        }))
    }
}

fn char_idx2byte(str_: impl crate::strings::IntoCCharPtr, str_len: usize, idx: Varnumber) -> isize {
    let str_ = str_.into_c_char_ptr();
    {
        let mut nchar = idx;
        let mut nbyte: usize = 0;

        if nchar >= 0 {
            while nchar > 0 && nbyte < str_len {
                nbyte += utfc_ptr2len(str_.wrapping_add(nbyte)) as usize;
                nchar -= 1;
            }
        } else {
            nbyte = str_len;
            while nchar < 0 && nbyte > 0 {
                nbyte -= 1;
                nbyte -= utf_head_off(str_, str_.wrapping_add(nbyte)) as usize;
                nchar += 1;
            }
            if nchar < 0 {
                return -1;
            }
        }
        nbyte as isize
    }
}

/// Return the slice "str[first : last]" using character indexes.
pub fn string_slice(
    str_: impl crate::strings::IntoCCharPtr,
    first: Varnumber,
    last: Varnumber,
    exclusive: bool,
) -> Option<std::ffi::CString> {
    let str_ = str_.into_c_char_ptr();
    {
        if str_.is_null() {
            return None;
        }
        let slen = crate::memory::cstr_len(str_);
        let mut start_byte = char_idx2byte(str_, slen, first);
        if start_byte < 0 {
            start_byte = 0;
        }
        let mut end_byte: isize;
        if (last == -1 && !exclusive) || last == VARNUMBER_MAX {
            end_byte = slen as isize;
        } else {
            end_byte = char_idx2byte(str_, slen, last);
            if !exclusive && end_byte >= 0 && end_byte < (slen as isize) {
                end_byte += utfc_ptr2len(str_.wrapping_add(end_byte as usize)) as isize;
            }
        }

        if start_byte >= (slen as isize) || end_byte <= start_byte {
            return None;
        }
        Some(cstr_from_bytes(unsafe {
            std::slice::from_raw_parts(
                str_.wrapping_add(start_byte as usize).cast::<u8>(),
                (end_byte - start_byte) as usize,
            )
        }))
    }
}

/// Handle subscripts, `.name`, funcref calls, and `->method()`.
pub fn handle_subscript<T>(
    arg: *mut *const T,
    rettv: &mut TypVal,
    mut evalarg: Option<&mut Evalarg>,
    verbose: bool,
) -> bool {
    let arg: *mut *const u8 = arg.cast();
    unsafe {
        let evaluate = evalarg
            .as_ref()
            .is_some_and(|p| (p.eval_flags & EVAL_EVALUATE) != 0);
        let mut ret = true;
        let mut selfdict: *mut TvDict = ptr::null_mut();
        let mut lua_funcname: *const u8 = ptr::null();

        if tv_is_luafunc(rettv) {
            if !evaluate {
                tv_clear(rettv);
            }

            if **arg != b'.' {
                tv_clear(rettv);
                ret = false;
            } else {
                *arg = (*arg).wrapping_add(1);

                lua_funcname = *arg;
                let len = check_luafunc_name((*arg).cast(), true);
                if len == 0 {
                    tv_clear(rettv);
                    ret = false;
                }
                *arg = (*arg).wrapping_add(len as usize);
            }
        }

        while ret
            && (((**arg == b'['
                || (**arg == b'.' && rettv.v_type == VarType::VarDict)
                || (**arg == b'(' && (!evaluate || tv_is_func(rettv))))
                && !ascii_iswhite(i32::from(*(*arg).wrapping_sub(1))))
                || (**arg == b'-' && crate::memory::cstr_byte_at((*arg).cast(), 1) == b'>'))
        {
            if **arg == b'(' {
                ret = call_func_rettv(
                    arg.cast::<*mut u8>(),
                    evalarg.as_deref_mut(),
                    rettv,
                    evaluate,
                    selfdict,
                    None,
                    lua_funcname.cast(),
                );

                if aborting() {
                    if ret {
                        tv_clear(rettv);
                    }
                    ret = false;
                }
                tv_dict_unref(selfdict);
                selfdict = ptr::null_mut();
            } else if **arg == b'-' {
                if crate::memory::cstr_byte_at((*arg).cast(), 2) == b'{' {
                    ret = eval_lambda(
                        arg.cast::<*mut u8>(),
                        rettv,
                        evalarg.as_deref_mut(),
                        verbose,
                    );
                } else {
                    ret = eval_method(
                        arg.cast::<*mut u8>(),
                        rettv,
                        evalarg.as_deref_mut(),
                        verbose,
                    );
                }
            } else {
                tv_dict_unref(selfdict);
                if rettv.v_type == VarType::VarDict {
                    selfdict = rettv.vval.v_dict();
                    if !selfdict.is_null() {
                        (*selfdict).dv_refcount += 1;
                    }
                } else {
                    selfdict = ptr::null_mut();
                }
                if !eval_index(
                    arg.cast::<*mut u8>(),
                    rettv,
                    evalarg.as_deref_mut(),
                    verbose,
                ) {
                    tv_clear(rettv);
                    ret = false;
                }
            }
        }

        if !selfdict.is_null() && tv_is_func(rettv) {
            set_selfdict(rettv, &mut *selfdict);
        }

        tv_dict_unref(selfdict);
        ret
    }
}

pub fn set_selfdict(rettv: &mut TypVal, selfdict: &mut TvDict) {
    unsafe {
        if rettv.v_type == VarType::VarPartial
            && !(*tv_partial(&*rettv)).pt_auto
            && !(*tv_partial(&*rettv)).pt_dict.is_null()
        {
            return;
        }
        make_partial(selfdict, rettv);
    }
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

    if eval_st().var_item_copy_recurse >= DICT_MAXNEST {
        emsg(gettext(E_VARIABLE_NESTED_TOO_DEEP_FOR_MAKING_COPY));
        return false;
    }
    eval_st().var_item_copy_recurse += 1;

    match from.v_type {
        VarType::VarNumber
        | VarType::VarFloat
        | VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarBool
        | VarType::VarSpecial => {
            tv_copy(&*from, &mut *to);
        }
        VarType::VarString => {
            let Some(conv) = conv.filter(|c| c.vc_type != CONV_NONE) else {
                tv_copy(&*from, &mut *to);
                eval_st().var_item_copy_recurse -= 1;
                return ret;
            };
            if from.vval.v_string().is_null() {
                tv_copy(&*from, &mut *to);
            } else {
                if let Some(conv_bytes) = string_convert(conv, from.vval.v_string(), None) {
                    crate::eval::typval::tv_set_string(&mut *to, conv_bytes.as_ptr().cast());
                } else {
                    crate::eval::typval::tv_set_string(&mut *to, from.vval.v_string());
                }
                to.v_lock = VAR_UNLOCKED;
            }
        }
        VarType::VarList => {
            to.v_type = VarType::VarList;
            to.v_lock = VAR_UNLOCKED;
            if from.vval.v_list().is_null() {
                to.vval = crate::eval::typval::TypvalVval::List(ptr::null_mut());
            } else if copy_id != 0 && tv_list_copyid(unsafe { &*from.vval.v_list() }) == copy_id {
                to.vval = crate::eval::typval::TypvalVval::List(tv_list_latest_copy(unsafe {
                    &*from.vval.v_list()
                }));
                tv_list_ref(unsafe { to.vval.v_list().as_mut() });
            } else {
                to.vval = crate::eval::typval::TypvalVval::List(tv_list_copy(
                    conv,
                    unsafe { from.vval.v_list().as_mut() },
                    deep,
                    copy_id,
                ));
            }
            if to.vval.v_list().is_null() && !from.vval.v_list().is_null() {
                ret = false;
            }
        }
        VarType::VarBlob => {
            tv_blob_copy(unsafe { from.vval.v_blob().as_ref() }, to);
        }
        VarType::VarDict => {
            to.v_type = VarType::VarDict;
            to.v_lock = VAR_UNLOCKED;
            if from.vval.v_dict().is_null() {
                to.vval = crate::eval::typval::TypvalVval::Dict(ptr::null_mut());
            } else if copy_id != 0 && unsafe { (*from.vval.v_dict()).dv_copy_id } == copy_id {
                to.vval = crate::eval::typval::TypvalVval::Dict(unsafe {
                    (*from.vval.v_dict()).dv_copydict
                });
                unsafe {
                    (*to.vval.v_dict()).dv_refcount += 1;
                }
            } else {
                to.vval = crate::eval::typval::TypvalVval::Dict(tv_dict_copy(
                    conv,
                    unsafe { from.vval.v_dict().as_mut() },
                    deep,
                    copy_id,
                ));
            }
            if to.vval.v_dict().is_null() && !from.vval.v_dict().is_null() {
                ret = false;
            }
        }
        VarType::VarUnknown => {
            internal_error(c"var_item_copy(UNKNOWN)");
            ret = false;
        }
    }
    eval_st().var_item_copy_recurse -= 1;
    ret
}

/// ":echo expr1 ..." and ":echon expr1 ..."
pub fn ex_echo(eap: &mut Exarg) {
    let mut arg = eap.arg;
    let mut rettv = tv_initial_value();
    let mut atstart = true;
    let mut need_clear = true;
    let did_emsg_before = g_st().did_emsg;
    let called_emsg_before = g_st().called_emsg;
    let mut evalarg = Evalarg::new();

    fill_evalarg_from_eap(&mut evalarg, eap.skip != 0);

    if eap.skip != 0 {
        g_st().emsg_skip += 1;
    }
    while !crate::memory::cstr_is_empty(arg)
        && crate::memory::cstr_first_byte(arg) != b'|'
        && crate::memory::cstr_first_byte(arg) != b'\n'
        && !g_st().got_int
    {
        g_st().need_clr_eos = true;

        {
            let p = arg;
            if !eval1(&raw mut arg, &mut rettv, Some(&mut evalarg)) {
                if !aborting()
                    && g_st().did_emsg == did_emsg_before
                    && g_st().called_emsg == called_emsg_before
                {
                    semsg!(gettext(E_INVEXPR2), p);
                }
                g_st().need_clr_eos = false;
                break;
            }
            g_st().need_clr_eos = false;
        }

        if eap.skip == 0 {
            if atstart {
                atstart = false;
                msg_ext_set_append(eap.cmdidx == Cmdidx::CmdEchon as i32);
                msg_ext_no_fast();
                msg_ext_set_kind(c"echo");
                if eap.cmdidx == Cmdidx::CmdEcho as i32 {
                    if !g_st().msg_didout {
                        msg_sb_eol();
                    }
                    msg_start();
                }
            } else if eap.cmdidx == Cmdidx::CmdEcho as i32 {
                msg_puts_hl(c" ", eval_st().echo_hl_id, false);
            }
            let tofree = encode_tv2echo(&mut rettv, None);
            msg_multiline(
                cstr_as_string(tofree.as_ptr()),
                eval_st().echo_hl_id,
                true,
                false,
                &mut need_clear,
            );
        }
        tv_clear(&mut rettv);
        arg = skipwhite(arg);
    }
    eap.nextcmd = check_nextcmd(arg);
    msg_ext_set_append(false);

    if eap.skip != 0 {
        g_st().emsg_skip -= 1;
    } else {
        if ui_has(K_UIMESSAGES)
            && (crate::memory::cstr_is_empty(eap.arg)
                || crate::memory::cstr_first_byte(eap.arg) == b'|'
                || crate::memory::cstr_first_byte(eap.arg) == b'\n')
        {
            msg_puts_len(c"", 0, 0, false);
        } else if need_clear {
            msg_clr_eos();
        }
        if eap.cmdidx == Cmdidx::CmdEcho as i32 {
            msg_end();
        }
    }
}

/// ":echohl {name}".
pub fn ex_echohl(eap: &mut Exarg) {
    eval_st().echo_hl_id = syn_name2id(eap.arg);
}

/// Returns the :echo highlight id.
pub fn get_echo_hl_id() -> i32 {
    eval_st().echo_hl_id
}

/// ":execute", ":echomsg", ":echoerr"
pub fn ex_execute(eap: &mut Exarg) {
    let mut arg = eap.arg;
    let mut rettv = tv_initial_value();
    let mut ret = true;
    let mut buf: Vec<u8> = Vec::with_capacity(80);
    let mut has_arg = false;

    if eap.skip != 0 {
        g_st().emsg_skip += 1;
    }
    while !crate::memory::cstr_is_empty(arg)
        && crate::memory::cstr_first_byte(arg) != b'|'
        && crate::memory::cstr_first_byte(arg) != b'\n'
    {
        ret = eval1_emsg(&raw mut arg, &mut rettv, Some(eap));
        if !ret {
            break;
        }

        if eap.skip == 0 {
            let mut argstr_owned: Option<std::ffi::CString> = None;
            let argstr = if eap.cmdidx == Cmdidx::CmdExecute as i32 {
                tv_get_string(&rettv)
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
        arg = skipwhite(arg);
    }

    if ret && has_arg {
        buf.push(0);
        if eap.cmdidx == Cmdidx::CmdEchomsg as i32 {
            msg_ext_no_fast();
            msg_ext_set_kind(c"echomsg");
            msg(buf.as_ptr().cast(), eval_st().echo_hl_id);
        } else if eap.cmdidx == Cmdidx::CmdEchoerr as i32 {
            let save_did_emsg = g_st().did_emsg;
            msg_ext_no_fast();
            emsg_multiline(buf.as_ptr().cast(), c"echoerr", HLF_E, true);
            if !g_st().force_abort {
                g_st().did_emsg = save_did_emsg;
            }
        } else if eap.cmdidx == Cmdidx::CmdExecute as i32 {
            do_cmdline(
                buf.as_mut_ptr().cast(),
                eap.ea_getline,
                DOCMD_NOWAIT | DOCMD_VERBOSE,
            );
        }
    }

    if eap.skip != 0 {
        g_st().emsg_skip -= 1;
    }

    eap.nextcmd = check_nextcmd(arg);
}

/// Skip over the name of an option variable: "&option", "&g:option" or "&l:option".
pub fn find_option_var_end<T>(
    arg: *mut *const T,
    opt_idxp: &mut OptIndex,
    opt_flags: &mut i32,
) -> *const T {
    let arg: *mut *const u8 = arg.cast();
    unsafe {
        let mut p = (*arg).wrapping_add(1);
        if crate::memory::cstr_first_byte(p.cast()) == b'g'
            && crate::memory::cstr_byte_at(p.cast(), 1) == b':'
        {
            *opt_flags = OPT_GLOBAL;
            p = p.wrapping_add(2);
        } else if crate::memory::cstr_first_byte(p.cast()) == b'l'
            && crate::memory::cstr_byte_at(p.cast(), 1) == b':'
        {
            *opt_flags = OPT_LOCAL;
            p = p.wrapping_add(2);
        } else {
            *opt_flags = 0;
        }

        let end = find_option_end(p.cast(), opt_idxp);
        *arg = if end.is_null() { *arg } else { p };
        end.cast()
    }
}

pub fn var_flavour(varname: impl crate::strings::IntoCCharPtr) -> VarFlavour {
    let varname = varname.into_c_char_ptr().cast_mut();
    {
        let mut p = varname;

        if ascii_isupper(crate::memory::cstr_first_char(p)) {
            loop {
                p = p.wrapping_add(1);
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
                if ascii_islower(crate::memory::cstr_first_char(p)) {
                    return VAR_FLAVOUR_SESSION;
                }
            }
            return VAR_FLAVOUR_SHADA;
        }
        VAR_FLAVOUR_DEFAULT
    }
}

pub fn var_set_global(name: impl crate::strings::IntoCCharPtr, mut vartv: TypVal) {
    let name = name.into_c_char_ptr();
    {
        save_funccal();
        set_var(name, crate::memory::cstr_len(name), &mut vartv, false);
        restore_funccal();
    }
}

/// Display script name where an item was last set.
pub fn last_set_msg(script_ctx: Sctx) {
    if script_ctx.sc_sid == 0 {
        return;
    }

    let p = get_scriptname(script_ctx, true);
    g_st().msg_ext_skip_verbose = true;

    verbose_enter();
    msg_puts(gettext(c"\n\tLast set from "));
    msg_puts(p.as_ptr());
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
    str_: impl crate::strings::IntoCCharPtr,
    len: usize,
    pat: impl crate::strings::IntoCCharPtr,
    sub: impl crate::strings::IntoCCharPtr,
    expr: Option<&TypVal>,
    flags: impl crate::strings::IntoCCharPtr,
) -> std::ffi::CString {
    let str_ = str_.into_c_char_ptr().cast_mut();
    let pat = pat.into_c_char_ptr().cast_mut();
    let sub = sub.into_c_char_ptr().cast_mut();
    let flags = flags.into_c_char_ptr();
    {
        let mut regmatch: Regmatch = Regmatch::new();
        let mut out: Option<Vec<u8>> = None;

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = EMPTY_STRING_OPTION.as_ptr().cast();

        regmatch.rm_ic = opt_vars().p_ic != 0;
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        if !regmatch.regprog.is_null() {
            let mut tail = str_;
            let end = str_.wrapping_add(len);
            let do_all = crate::memory::cstr_first_byte(flags) == b'g';
            let mut zero_width = ptr::null_mut();

            while vim_regexec_nl(
                &mut regmatch,
                str_,
                ((tail as usize).wrapping_sub(str_ as usize)) as Colnr,
            ) {
                if regmatch.startp[0] == regmatch.endp[0] {
                    if zero_width == regmatch.startp[0] {
                        let i = utfc_ptr2len(tail) as usize;
                        if let Some(buf) = &mut out {
                            buf.extend_from_slice(unsafe {
                                std::slice::from_raw_parts(tail.cast::<u8>(), i)
                            });
                        }
                        tail = tail.wrapping_add(i);
                        continue;
                    }
                    zero_width = regmatch.startp[0];
                }

                let sublen = vim_regsub(&mut regmatch, sub, expr, tail, 0, REGSUB_MAGIC);
                if sublen <= 0 {
                    out = None;
                    break;
                }
                let buf = out.get_or_insert_with(|| Vec::with_capacity(200));

                let i = (regmatch.startp[0] as usize).wrapping_sub(tail as usize);
                if i > 0 {
                    buf.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(tail.cast::<u8>(), i)
                    });
                }
                let cur_len = buf.len();
                buf.resize(cur_len + (sublen as usize), 0);
                vim_regsub(
                    &mut regmatch,
                    sub,
                    expr,
                    buf.as_mut_ptr().wrapping_add(cur_len),
                    sublen,
                    REGSUB_COPY | REGSUB_MAGIC,
                );
                buf.truncate(cur_len + (sublen as usize) - 1);
                tail = regmatch.endp[0].cast();
                if crate::memory::cstr_is_empty(tail) {
                    break;
                }
                if !do_all {
                    break;
                }
            }

            if let Some(buf) = &mut out {
                let rem = (end as usize).wrapping_sub(tail as usize);
                if rem > 0 {
                    buf.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(tail.cast::<u8>(), rem)
                    });
                }
            }

            vim_regfree(regmatch.regprog);
        }

        let ret = if let Some(buf) = &out {
            cstr_from_bytes(buf)
        } else {
            cstr_from_bytes(crate::memory::cstr_slice(str_, len))
        };
        drop(out);
        let empty_opt = EMPTY_STRING_OPTION.as_ptr().cast();
        if opt_vars().p_cpo == empty_opt {
            opt_vars().p_cpo = save_cpo;
        } else {
            if crate::memory::cstr_is_empty(opt_vars().p_cpo) {
                let obj = Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(save_cpo)),
                };
                set_option_value_give_err(K_OPT_CPOPTIONS, obj, 0);
            }
            free_string_option(save_cpo);
        }

        ret
    }
}

/// Common code for getting job callbacks for `jobstart`.
pub fn common_job_callbacks(
    vopts: &mut TvDict,
    on_stdout: &mut CallbackReader,
    on_stderr: &mut CallbackReader,
    on_exit: &mut Callback,
) -> bool {
    if tv_dict_get_callback(Some(&mut *vopts), c"on_stdout", 9, &mut on_stdout.cb)
        && tv_dict_get_callback(Some(&mut *vopts), c"on_stderr", 9, &mut on_stderr.cb)
        && tv_dict_get_callback(Some(&mut *vopts), c"on_exit", 7, on_exit)
    {
        on_stdout.buffered = tv_dict_get_number(Some(vopts), c"stdout_buffered") != 0;
        on_stderr.buffered = tv_dict_get_number(Some(vopts), c"stderr_buffered") != 0;
        if on_stdout.buffered && on_stdout.cb.cb_type() == K_CALLBACK_NONE_CONST {
            on_stdout.self_dict = (&raw mut *vopts);
        }
        if on_stderr.buffered && on_stderr.cb.cb_type() == K_CALLBACK_NONE_CONST {
            on_stderr.self_dict = (&raw mut *vopts);
        }
        vopts.dv_refcount += 1;
        return true;
    }

    *on_stdout = CallbackReader::none();
    *on_stderr = CallbackReader::none();
    callback_free(on_exit);
    false
}

pub fn find_job(id: u64, show_error: bool) -> *mut Channel {
    let data = find_channel(id);
    let is_stopped = if !data.is_null() && unsafe { (*data).streamtype } == K_CHANNEL_STREAM_PROC {
        let hdr = unsafe { &(*data).uv.proc_ };
        hdr.status >= 0 || hdr.stopped_time != 0
    } else {
        true
    };
    if data.is_null() || unsafe { (*data).streamtype } != K_CHANNEL_STREAM_PROC || is_stopped {
        if show_error {
            if !data.is_null() && unsafe { (*data).streamtype } != K_CHANNEL_STREAM_PROC {
                emsg(gettext(E_INVCHANJOB));
            } else {
                emsg(gettext(E_INVCHAN));
            }
        }
        return ptr::null_mut();
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
    tv_list_append_string(&mut *args, argvars[0].vval.v_string(), -1);
    *rettv = eval_call_provider(name.as_ptr().cast_mut(), c"eval", &mut *args, false);
}

pub fn eval_call_provider(
    provider: impl crate::strings::IntoCCharPtr,
    method: &std::ffi::CStr,
    arguments: &mut TvList,
    discard: bool,
) -> TypVal {
    let provider = provider.into_c_char_ptr().cast_mut();
    {
        if !eval_has_provider(provider, false) {
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
        let name_len = vim_snprintf!(
            func.as_mut_ptr().cast(),
            func.len(),
            c"provider#%s#Call",
            provider,
        );

        let new_scope = CallerScope {
            script_ctx: g_st().current_sctx,
            es_entry: rt_st().exestack.last().unwrap().clone(),
            autocmd_fname: g_st().autocmd_fname.clone(),
            autocmd_match: g_st().autocmd_match.clone(),
            autocmd_fname_full: g_st().autocmd_fname_full,
            autocmd_bufnr: g_st().autocmd_bufnr,
            funccalp: get_current_funccal().cast(),
        };
        let saved_provider_caller_scope =
            std::mem::replace(&mut g_st().provider_caller_scope, new_scope);
        save_funccal();
        {
            g_st().provider_call_nesting += 1;
        }

        let mut argvars = [
            TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(method.as_ptr().cast_mut()),
            },
            TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(arguments),
            },
            tv_initial_value(),
        ];
        let mut rettv = tv_initial_value();
        tv_list_ref(Some(&mut *arguments));

        let lnum = curwin_mut().w_cursor.lnum;
        let mut funcexe = Funcexe {
            fe_fill_submatch: false,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: false,
            fe_evaluate: true,
            fe_partial: ptr::null_mut(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        call_func(
            func.as_ptr().cast(),
            name_len,
            &mut rettv,
            2,
            argvars.as_mut_ptr(),
            &mut funcexe,
        );

        tv_list_unref(arguments);
        restore_funccal();
        {
            g_st().provider_caller_scope = saved_provider_caller_scope;
            g_st().provider_call_nesting -= 1;
            debug_assert!(g_st().provider_call_nesting >= 0);
        }

        if discard {
            tv_clear(&mut rettv);
        }

        rettv
    }
}

/// Checks if provider for feature `feat` is enabled.
pub fn eval_has_provider(feat: impl crate::strings::IntoCCharPtr, throw_if_fast: bool) -> bool {
    let feat = feat.into_c_char_ptr();
    {
        if !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"clipboard"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"python3"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"python3_compiled"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"python3_dynamic"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"perl"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"ruby"))
            && !(crate::memory::cstr_as_opt_bytes(feat) == Some(b"node"))
        {
            return false;
        }

        if throw_if_fast && !nlua_is_deferred_safe() {
            semsg!(E_FAST_API_DISABLED, c"Vimscript function",);
            return false;
        }

        let mut name = [0u8; 32];
        vim_snprintf!(name.as_mut_ptr().cast(), name.len(), c"%s", feat);
        {
            let _len = crate::memory::cstr_len(name.as_ptr().cast());
            for _b in &mut name[.._len] {
                if *_b == b'_' {
                    *_b = 0;
                }
            }
        };

        let mut buf = [0u8; 256];
        let mut tv = tv_initial_value();
        let mut len = vim_snprintf!(
            buf.as_mut_ptr().cast(),
            buf.len(),
            c"g:loaded_%s_provider",
            name.as_ptr(),
        );
        if !eval_variable(
            buf.as_ptr().cast(),
            len,
            Some(&mut tv),
            ptr::null_mut(),
            false,
            true,
        ) {
            len = vim_snprintf!(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"provider#%s#bogus",
                name.as_ptr(),
            );
            script_autoload(buf.as_ptr().cast(), len as usize, false);

            len = vim_snprintf!(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"g:loaded_%s_provider",
                name.as_ptr(),
            );
            if !eval_variable(
                buf.as_ptr().cast(),
                len,
                Some(&mut tv),
                ptr::null_mut(),
                false,
                true,
            ) {
                vim_snprintf!(
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    c"provider#%s#Call",
                    name.as_ptr(),
                );
                if !find_func(buf.as_ptr().cast()).is_null() && opt_vars().p_lpl != 0 {
                    semsg!(
                        c"provider: %s: missing required variable g:loaded_%s_provider",
                        name.as_ptr(),
                        name.as_ptr(),
                    );
                }
                return false;
            }
        }

        let mut ok = tv.v_type == VarType::VarNumber && tv.vval.v_number() == 2;

        if ok {
            vim_snprintf!(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"provider#%s#Call",
                name.as_ptr(),
            );
            if find_func(buf.as_ptr().cast()).is_null() {
                semsg!(
                    c"provider: %s: g:loaded_%s_provider=2 but %s is not defined",
                    name.as_ptr(),
                    name.as_ptr(),
                    buf.as_ptr(),
                );
                ok = false;
            }
        }

        ok
    }
}

/// Gets the current user-input in prompt buffer `buf`, or `None` if buffer is not a prompt buffer.
pub fn prompt_get_input(buf: &mut FileBuf) -> Option<std::ffi::CString> {
    if !bt_prompt(Some(&*buf)) {
        return None;
    }
    let lnum_start = buf.b_prompt_start.mark.lnum;
    let lnum_last = buf.b_ml.ml_line_count;

    let mut text = ml_get_buf(buf, lnum_start);
    if (crate::memory::cstr_len(text) as i32) >= buf.b_prompt_start.mark.col {
        text = text.wrapping_add(buf.b_prompt_start.mark.col as usize);
    }

    let mut bytes = crate::memory::cstr_as_bytes(text).to_vec();
    for i in (lnum_start + 1)..=lnum_last {
        bytes.push(b'\n');
        bytes.extend_from_slice(crate::memory::cstr_as_bytes(ml_get_buf(buf, i)));
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
            let mut eq = tv_blob_equal(unsafe { typ1.vval.v_blob().as_ref() }, unsafe {
                typ2.vval.v_blob().as_ref()
            });
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
            let mut eq = tv_list_equal(
                unsafe { typ1.vval.v_list().as_ref() },
                unsafe { typ2.vval.v_list().as_ref() },
                ic,
            );
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
            let mut eq = tv_dict_equal(
                unsafe { typ1.vval.v_dict().as_ref() },
                unsafe { typ2.vval.v_dict().as_ref() },
                ic,
            );
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
        let mut eq = if (typ1.v_type == VarType::VarPartial && tv_partial(&*typ1).is_null())
            || (typ2.v_type == VarType::VarPartial && tv_partial(&*typ2).is_null())
        {
            tv_partial(&*typ1) == tv_partial(&*typ2)
        } else if type_is {
            if typ1.v_type == VarType::VarFunc && typ2.v_type == VarType::VarFunc {
                tv_equal(&*typ1, &*typ2, ic)
            } else if typ1.v_type == VarType::VarPartial && typ2.v_type == VarType::VarPartial {
                tv_partial(&*typ1) == tv_partial(&*typ2)
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
            EXPR_MATCH => pattern_match(s2, s1, ic) != 0,
            EXPR_NOMATCH => pattern_match(s2, s1, ic) == 0,
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
        return cstr_to_cstring(arg.vval.v_string());
    }
    encode_tv2string(&mut *arg, None)
}

impl Lval {
    #[inline]
    pub fn tv_mut(&self) -> &'static mut TypVal {
        unsafe { &mut *self.ll_tv }
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
        assert_eq!(var_flavour(c"foo"), VAR_FLAVOUR_DEFAULT);
        assert_eq!(var_flavour(c"Foo"), VAR_FLAVOUR_SESSION);
        assert_eq!(var_flavour(c"FOO"), VAR_FLAVOUR_SHADA);

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

        let mut expr1 = *b"$XXX_UNEXISTENT_VAR_XXX\0";
        let mut rettv = tv_initial_value();
        let mut evalarg = Evalarg {
            eval_flags: EVAL_EVALUATE,
        };
        assert!(eval0(
            expr1.as_mut_ptr().cast(),
            &mut rettv,
            None,
            &mut evalarg
        ));
        assert_eq!(rettv.v_type, VarType::VarString);
        assert!(rettv.vval.v_string().is_null());
        tv_clear(&mut rettv);

        let mut expr2 = *b"v:_null_list\0";
        assert!(eval0(
            expr2.as_mut_ptr().cast(),
            &mut rettv,
            None,
            &mut evalarg
        ));
        assert_eq!(rettv.v_type, VarType::VarList);
        assert!(rettv.vval.v_list().is_null());
        tv_clear(&mut rettv);

        let mut expr3 = *b"v:_null_dict\0";
        assert!(eval0(
            expr3.as_mut_ptr().cast(),
            &mut rettv,
            None,
            &mut evalarg
        ));
        assert_eq!(rettv.v_type, VarType::VarDict);
        assert!(rettv.vval.v_dict().is_null());
        tv_clear(&mut rettv);
    }
}
