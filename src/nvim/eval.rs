//! Expression evaluation (`src/nvim/eval.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::private::helpers::{
    HLF_E, api_clear_error, arena_array, cstr_as_string, cstr_to_string, tv_dict_is_watched,
};
use crate::charset::{
    VARNUMBER_MIN, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_islower, ascii_isupper,
    ascii_iswhite, ascii_iswhite_or_nul, ascii_isxdigit, hex2nr, skipdigits, skiptowhite,
    skipwhite, vim_str2nr,
};
use crate::eval::typval::{
    Callback, EvalFuncData, TV_CSTRING, TypvalVval, VAR_UNLOCKED, VarType, blob_T, callback_copy,
    callback_free, callback_put, dict_T, dict_watchers_queue, dictitem_T, float_T, kCallbackExpr,
    kCallbackFuncref, kCallbackLua, kCallbackNone, kCallbackPartial, list_T, listitem_T,
    listwatch_T, partial_T, tv_blob_alloc, tv_blob_alloc_ret, tv_blob_check_index,
    tv_blob_check_range, tv_blob_copy, tv_blob_equal, tv_blob_get, tv_blob_len, tv_blob_set_append,
    tv_blob_set_range, tv_blob_slice_or_index, tv_blob_unref, tv_check_lock, tv_check_num,
    tv_check_str, tv_clear, tv_copy, tv_dict_add, tv_dict_add_nr, tv_dict_alloc, tv_dict_copy,
    tv_dict_equal, tv_dict_find, tv_dict_free, tv_dict_free_contents, tv_dict_free_dict,
    tv_dict_get_callback, tv_dict_get_number, tv_dict_hi2di, tv_dict_item_alloc, tv_dict_item_free,
    tv_dict_unref, tv_dict_watcher_node_data, tv_dict_watcher_notify, tv_dict_wrong_func_name,
    tv_empty_string, tv_equal, tv_get_float, tv_get_number, tv_get_number_chk, tv_get_string,
    tv_get_string_buf, tv_get_string_buf_chk, tv_get_string_chk, tv_in_free_unref_items,
    tv_initial_value, tv_is_func, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_owned_tv, tv_list_append_string, tv_list_assign_range,
    tv_list_check_range_index_one, tv_list_check_range_index_two, tv_list_concat, tv_list_copy,
    tv_list_equal, tv_list_find, tv_list_find_nr, tv_list_first, tv_list_free,
    tv_list_free_contents, tv_list_free_list, tv_list_join, tv_list_last, tv_list_len, tv_list_ref,
    tv_list_set_lock, tv_list_set_ret, tv_list_slice_or_index, tv_list_unref, tv_list_watch_add,
    tv_list_watch_remove, tv2bool, typval_T, value_check_lock, varnumber_T,
};
use crate::eval::vars::{
    VV_ARGV, VV_EVENT, VV_LUA, VV_SHELL_ERROR, check_vars, eval_one_expr_in_str, eval_variable,
    evalarg_T, evalvars_clear, evalvars_init, ex_let_vars, find_var, garbage_collect_globvars,
    garbage_collect_scriptvars, garbage_collect_vimvars, get_vim_var_dict, get_vim_var_partial,
    get_vimvar_dict, lval_T, opt_to_tv, set_var, set_var_const, set_vim_var_list, set_vim_var_nr,
    skip_var_list, tv_init, valid_varname, var_check_lock, var_check_ro, var_wrong_func_name,
};
use crate::event::r#loop::{loop_queues, uv_handle_size};
use crate::strings::{
    atol, concat_str, gettext, strcmp, strlen, strncmp, strpbrk, strstr, strtod, vim_snprintf,
    vim_strchr,
};
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

use crate::buffer::{NOTDONE, VAR_FIXED, bt_prompt, buf_T, buflist_findnr, kUIMessages};
use crate::channel::{CallbackReader, Channel, callback_reader_free, kChannelStreamProc};
use crate::errors::*;
use crate::eval::funcs::{E_STRING_LIST_OR_BLOB_REQUIRED, UV_TIMER, get_list_line, kOptCpoptions};
use crate::eval::userfunc::{
    call_func, call_simple_func, call_simple_luafunc, deref_func_name, eval_fname_script,
    find_func, free_all_functions, free_unref_funccal, func_init, func_ptr_unref, func_ref,
    func_unref, funcexe_T, get_current_funccal, get_func_tv, get_funccal_args_ht, get_lambda_tv,
    get_scriptlocal_funcname, make_partial, restore_funccal, save_funccal, set_ref_in_call_stack,
    set_ref_in_func, set_ref_in_func_args, set_ref_in_functions, set_ref_in_previous_funccal,
    ufunc_T,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{
    check_nextcmd, cmd_has_expr_args, do_cmdline, ends_excmd, exarg_T, set_ref_in_findfunc,
};
use crate::globals::*;
use crate::lua::executor::{
    kRetObject, nlua_call_ref, nlua_is_deferred_safe, nlua_is_table_from_lua,
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
    find_option_end, get_option_value, get_tty_option, is_option_hidden, is_tty_option,
    kOptFoldexpr, kOptFoldtext, optval_free, set_option_value_give_err, was_set_insecurely,
};
use crate::option_vars::*;
use crate::quickfix::set_ref_in_quickfix;
use crate::regexp::{
    RE_MAGIC, RE_STRING, REGSUB_COPY, REGSUB_MAGIC, regmatch_T, vim_regcomp, vim_regexec_nl,
    vim_regfree, vim_regsub,
};
use crate::register::{get_reg_contents, kGRegExprSrc, op_global_reg_iter, yankreg_T};
use crate::runtime::{
    exestack, free_autoload_scriptnames, free_scriptnames, get_scriptname, script_autoload,
    script_is_lua, sctx_T, sourcing_a_script,
};
use crate::ui::ui_has;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::validate::{Error, ErrorType, NvimString, Object, ObjectData, ObjectType};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, PROF_YES, funccal_entry_T};
use crate::cmdexpand::{
    EXPAND_COMMANDS, EXPAND_ENV_VARS, EXPAND_EXPRESSION, EXPAND_FUNCTIONS, EXPAND_NOTHING,
    EXPAND_SETTINGS, EXPAND_USER_VARS, cmdidx_T,
};
use crate::cursor::{colnr_T, linenr_T, pos_T};
use crate::eval::encode::{encode_list_write, encode_tv2echo, encode_tv2string};
use crate::eval::executor::eexe_mod_op;
use crate::eval::gc::{gc_first_dict, gc_first_list};
use crate::event::multiqueue::{multiqueue_free, multiqueue_new_child};
use crate::event::proc::{Proc, proc_header};
use crate::event::time::{
    TimeWatcher, TimeWatcherFields, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop, watcher_fields,
};
use crate::ex_eval::{aborting, discard_current_exception};
use crate::garray::{ga_append, ga_clear, ga_concat, ga_grow, ga_init, garray_T};
use crate::hashtab::{hash_init, hashitem_empty, hashtab_T};
use crate::highlight_group::syn_name2id;
use crate::insexpand::{set_ref_in_cpt_callbacks, set_ref_in_insexpand_funcs};
use crate::keycodes::{
    FSK_IN_STRING, FSK_KEYCODE, FSK_SIMPLIFY, K_SPECIAL, KE_SNR, KS_EXTRA, find_special_key,
    trans_special,
};
use crate::map::{
    Map, MapHash, Set, map_del_uint64_t_ptr_t, map_put_ref_uint64_t_ptr_t, map_ref_uint64_t_ptr_t,
    ptr_t,
};
use crate::mark::{mark_get, mark_global_iter, xfmark_T};
use crate::math::{FAIL, OK};
use crate::mbyte::{
    mb_charlen, mb_copy_char, mb_strcmp_ic, string_convert, utf_char2bytes, utf_head_off,
    utfc_ptr2len, vimconv_T,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{
    Arena, arena_finish, arena_mem_free, memchrsub, strchrsub, strequal, strnequal, xcalloc, xfree,
    xmalloc, xmemdupz, xrealloc, xstrdup, xstrndup,
};
use crate::msgpack_rpc::channel::{channel_prefix, channel_stream_ptr, channel_tail, find_channel};
use crate::ops::set_ref_in_opfunc;
use crate::optionstr::{OptIndex, free_string_option};
use crate::os::env::{env_name_len, expand_T, expand_env_save, vim_getenv};
use crate::os::fs::os_can_exe;
use crate::os::lang::free_locales;
use crate::os::shell::{os_system, shell_argv_to_str, shell_build_argv, shell_free_argv};
use crate::profile::{estack_T, prof_child_enter, prof_child_exit, proftime_T};
use crate::state::save_v_event_T;
use crate::tag::set_ref_in_tagfunc;
use crate::window::win_T;

use crate::change::TAB;
use crate::charset::{NL, NUL};
use crate::globals::IOSIZE;
use crate::option::{OPT_GLOBAL, OPT_LOCAL, kOptInvalid};
use crate::regexp::NUMBUFLEN;
use crate::viml::parser::parser::CONV_NONE;
pub type timer_T = c_void;
pub type var_flavour_T = c_int;
pub type exprtype_T = c_int;
pub type glv_status_T = c_int;

pub const VAR_FLAVOUR_DEFAULT: var_flavour_T = 1;
pub const VAR_FLAVOUR_SESSION: var_flavour_T = 2;
pub const VAR_FLAVOUR_SHADA: var_flavour_T = 4;

pub const EXPR_UNKNOWN: exprtype_T = 0;
pub const EXPR_EQUAL: exprtype_T = 1;
pub const EXPR_NEQUAL: exprtype_T = 2;
pub const EXPR_GREATER: exprtype_T = 3;
pub const EXPR_GEQUAL: exprtype_T = 4;
pub const EXPR_SMALLER: exprtype_T = 5;
pub const EXPR_SEQUAL: exprtype_T = 6;
pub const EXPR_MATCH: exprtype_T = 7;
pub const EXPR_NOMATCH: exprtype_T = 8;
pub const EXPR_IS: exprtype_T = 9;
pub const EXPR_ISNOT: exprtype_T = 10;

pub const EVAL_EVALUATE: c_int = 1;
pub const COPYID_INC: c_int = 2;
pub const COPYID_MASK: c_int = !1;

const GLV_FAIL: glv_status_T = 0;
const GLV_OK: glv_status_T = 1;
const GLV_STOP: glv_status_T = 2;

pub(crate) const TFN_QUIET: c_int = 2;
pub(crate) const TFN_NO_AUTOLOAD: c_int = 4;
pub(crate) const TFN_READ_ONLY: c_int = 16;

pub(crate) const GLV_QUIET: c_int = TFN_QUIET;
pub(crate) const GLV_NO_AUTOLOAD: c_int = TFN_NO_AUTOLOAD;
pub(crate) const GLV_READ_ONLY: c_int = TFN_READ_ONLY;

pub(crate) const FNE_INCL_BR: c_int = 1;
pub(crate) const FNE_CHECK_START: c_int = 2;

const BS: c_char = 0x08;
const FF: c_char = 0x0c;
const CAR: c_char = 0x0d;
const ESC: c_char = 0x1b;

pub(crate) const DICT_MAXNEST: c_int = 100;

const VARNUMBER_MAX: varnumber_T = i64::MAX;

pub(crate) const VAR_DEF_SCOPE: c_int = 2;

const kBoolVarFalse: varnumber_T = 0;
const kBoolVarTrue: varnumber_T = 1;

pub(crate) const kListLenUnknown: isize = -1;
pub(crate) const kListLenShouldKnow: isize = -2;
pub(crate) const kListLenMayKnow: isize = -3;

pub(crate) const kMarkAll: c_int = 1;

const kWinOptFoldexpr: usize = 15;

pub(crate) const STR2NR_ALL: c_int = 0x7f;

pub(crate) const AUTOLOAD_CHAR: c_char = b'#' as c_char;

#[cfg(target_env = "msvc")]
const MAX_EVAL7_RECURSE: c_int = 300;
#[cfg(not(target_env = "msvc"))]
const MAX_EVAL7_RECURSE: c_int = 1000;

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

/// Used for checking if local variables or arguments used in a lambda.
#[unsafe(no_mangle)]
pub static mut eval_lavars_used: *mut bool = ptr::null_mut();

static mut echo_hl_id: c_int = 0;
static mut last_timer_id: u64 = 1;
static mut callback_depth: c_int = 0;
static mut CURRENT_COPY_ID: c_int = 0;
static mut EVAL7_RECURSE: c_int = 0;
static mut VAR_ITEM_COPY_RECURSE: c_int = 0;
static mut VAR2FPOS_POS: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

static mut timers: Map<u64, ptr_t> = Map {
    set: Set {
        h: MapHash {
            n_buckets: 0,
            size: 0,
            n_occupied: 0,
            upper_bound: 0,
            n_keys: 0,
            keys_capacity: 0,
            hash: ptr::null_mut(),
        },
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

/// Info used by a `:for` loop.
#[repr(C)]
struct forinfo_T {
    fi_semicolon: c_int,
    fi_varcount: c_int,
    fi_lw: listwatch_T,
    fi_list: *mut list_T,
    fi_bi: c_int,
    fi_blob: *mut blob_T,
    fi_string: *mut c_char,
    fi_byte_idx: c_int,
}

#[repr(C)]
pub struct ht_stack_T {
    pub ht: *mut hashtab_T,
    pub prev: *mut ht_stack_T,
}

#[repr(C)]
pub struct list_stack_T {
    pub list: *mut list_T,
    pub prev: *mut list_stack_T,
}

#[repr(C)]
pub struct TimerTail {
    pub timer_id: c_int,
    pub repeat_count: c_int,
    pub refcount: c_int,
    pub emsg_count: c_int,
    pub timeout: i64,
    pub stopped: bool,
    pub paused: bool,
    pub callback: Callback,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EstackEntryCopy {
    pub es_lnum: i32,
    pub es_name: *mut c_char,
    pub es_type: c_int,
    pub es_info: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct caller_scope {
    pub script_ctx: sctx_T,
    pub es_entry: EstackEntryCopy,
    pub autocmd_fname: *mut c_char,
    pub autocmd_match: *mut c_char,
    pub autocmd_fname_full: bool,
    pub autocmd_bufnr: c_int,
    pub funccalp: *mut c_void,
}

#[inline]
unsafe fn tv_partial(tv: *const typval_T) -> *mut partial_T {
    unsafe { (*tv).vval.v_ptr.cast::<partial_T>() }
}

#[inline]
unsafe fn tv_set_partial(tv: *mut typval_T, pt: *mut partial_T) {
    unsafe {
        (*tv).vval.v_ptr = pt.cast();
    }
}

#[inline]
pub unsafe fn tv_blob_set_ret(tv: *mut typval_T, b: *mut blob_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_BLOB;
        (*tv).vval.v_blob = b;
        if !b.is_null() {
            (*b).bv_refcount += 1;
        }
    }
}

#[inline]
pub unsafe fn tv_dict_set_ret(tv: *mut typval_T, d: *mut dict_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval.v_dict = d;
        if !d.is_null() {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_list_copyid(l: *const list_T) -> c_int {
    unsafe { (*l).lv_copyID }
}

#[inline]
unsafe fn tv_list_latest_copy(l: *const list_T) -> *mut list_T {
    unsafe { (*l).lv_copylist }
}

#[inline]
unsafe fn tv_list_has_watchers(l: *const list_T) -> bool {
    unsafe { !(*l).lv_watch.is_null() }
}

#[inline]
unsafe fn tv_list_item_tv(li: *mut listitem_T) -> *mut typval_T {
    unsafe { &raw mut (*li).li_tv }
}

#[inline]
unsafe fn tv_list_item_next(_l: *const list_T, li: *const listitem_T) -> *mut listitem_T {
    unsafe { (*li).li_next }
}

#[inline]
pub(crate) unsafe fn timer_tail(timer: *mut timer_T) -> *mut TimerTail {
    unsafe {
        let uv_sz = uv_handle_size(UV_TIMER);
        timer
            .cast::<u8>()
            .add(uv_sz + size_of::<TimeWatcherFields>())
            .cast::<TimerTail>()
    }
}

#[inline]
unsafe fn timer_size() -> usize {
    let uv_sz = unsafe { uv_handle_size(UV_TIMER) };
    uv_sz + size_of::<TimeWatcherFields>() + size_of::<TimerTail>()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_v_event(sve: *mut save_v_event_T) -> *mut dict_T {
    unsafe {
        let v_event = get_vim_var_dict(VV_EVENT);

        if (*v_event).dv_hashtab.ht_used > 0 {
            (*sve).sve_did_save = true;
            ptr::copy_nonoverlapping(
                &raw const (*v_event).dv_hashtab,
                &raw mut (*sve).sve_hashtab,
                1,
            );
            hash_init(&raw mut (*v_event).dv_hashtab);
        } else {
            (*sve).sve_did_save = false;
        }
        v_event
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn restore_v_event(v_event: *mut dict_T, sve: *mut save_v_event_T) {
    unsafe {
        tv_dict_free_contents(v_event);
        if (*sve).sve_did_save {
            ptr::copy_nonoverlapping(
                &raw const (*sve).sve_hashtab,
                &raw mut (*v_event).dv_hashtab,
                1,
            );
        } else {
            hash_init(&raw mut (*v_event).dv_hashtab);
        }
    }
}

/// @return  "n1" divided by "n2", taking care of dividing by zero.
#[unsafe(no_mangle)]
pub extern "C" fn num_divide(n1: varnumber_T, n2: varnumber_T) -> varnumber_T {
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
#[unsafe(no_mangle)]
pub extern "C" fn num_modulus(n1: varnumber_T, n2: varnumber_T) -> varnumber_T {
    if n2 == 0 { 0 } else { n1.wrapping_rem(n2) }
}

/// Initialize the global and v: variables.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_init() {
    unsafe {
        evalvars_init();
        func_init();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_clear() {
    unsafe {
        evalvars_clear();
        free_scriptnames();
        free_locales();
        free_autoload_scriptnames();
        garbage_collect(false);
        free_all_functions();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fill_evalarg_from_eap(
    evalarg: *mut evalarg_T,
    eap: *mut exarg_T,
    skip: bool,
) {
    unsafe {
        *evalarg = evalarg_T {
            eval_flags: if skip { 0 } else { EVAL_EVALUATE },
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        if eap.is_null() {
            return;
        }

        let is_list_line = (*eap).ea_getline.is_some_and(|f| {
            ptr::fn_addr_eq(
                f,
                get_list_line
                    as unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char,
            )
        });

        if sourcing_a_script(eap) != 0 || is_list_line {
            (*evalarg).eval_getline = (*eap).ea_getline;
            (*evalarg).eval_cookie = (*eap).cookie;
        }
    }
}

/// Top level evaluation function, returning a boolean.
/// Sets "error" to true if there was an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_bool(
    arg: *mut c_char,
    error: *mut bool,
    eap: *mut exarg_T,
    skip: bool,
    use_simple_function: bool,
) -> bool {
    unsafe {
        let mut tv = tv_initial_value();
        let mut retval = false;
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, skip);

        if skip {
            emsg_skip += 1;
        }
        let r = if use_simple_function {
            eval0_simple_funccal(arg, &raw mut tv, eap, &raw mut evalarg)
        } else {
            eval0(arg, &raw mut tv, eap, &raw mut evalarg)
        };
        if r == FAIL {
            *error = true;
        } else {
            *error = false;
            if !skip {
                retval = tv_get_number_chk(&raw const tv, error) != 0;
                tv_clear(&raw mut tv);
            }
        }
        if skip {
            emsg_skip -= 1;
        }
        clear_evalarg(&raw mut evalarg, eap);

        retval
    }
}

/// Call eval1() and give an error message if not done at a lower level.
unsafe fn eval1_emsg(arg: *mut *mut c_char, rettv: *mut typval_T, eap: *mut exarg_T) -> c_int {
    unsafe {
        let start = *arg;
        let did_emsg_before = did_emsg;
        let called_emsg_before = called_emsg;
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, !eap.is_null() && (*eap).skip != 0);

        let ret = eval1(arg, rettv, &raw mut evalarg);
        if ret == FAIL
            && !aborting()
            && did_emsg == did_emsg_before
            && called_emsg == called_emsg_before
        {
            semsg!(gettext((&raw const e_invexpr2).cast()), start);
        }
        clear_evalarg(&raw mut evalarg, eap);
        ret
    }
}

/// @return  whether a typval is a valid expression to pass to eval_expr_typval()
///          or eval_expr_to_bool(). An empty string returns false.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_valid_arg(tv: *const typval_T) -> bool {
    unsafe {
        (*tv).v_type != VarType::VAR_UNKNOWN
            && ((*tv).v_type != VarType::VAR_STRING
                || (!(*tv).vval.v_string.is_null() && *(*tv).vval.v_string != NUL))
    }
}

/// Evaluate a partial.
unsafe fn eval_expr_partial(
    expr: *const typval_T,
    argv: *mut typval_T,
    argc: c_int,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let partial = tv_partial(expr);
        if partial.is_null() {
            return FAIL;
        }

        let s = partial_name(partial);
        if s.is_null() || *s == NUL {
            return FAIL;
        }

        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: 0,
            fe_lastline: 0,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: true,
            fe_partial: partial.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        if call_func(s, -1, rettv, argc, argv, &raw mut funcexe) == FAIL {
            return FAIL;
        }

        OK
    }
}

/// Evaluate an expression which is a function.
unsafe fn eval_expr_func(
    expr: *const typval_T,
    argv: *mut typval_T,
    argc: c_int,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let s = if (*expr).v_type == VarType::VAR_FUNC {
            (*expr).vval.v_string.cast_const()
        } else {
            tv_get_string_buf_chk(expr, buf.as_mut_ptr())
        };
        if s.is_null() || *s == NUL {
            return FAIL;
        }

        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: 0,
            fe_lastline: 0,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: true,
            fe_partial: ptr::null_mut(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        if call_func(s, -1, rettv, argc, argv, &raw mut funcexe) == FAIL {
            return FAIL;
        }

        OK
    }
}

/// Evaluate an expression, which is a string.
unsafe fn eval_expr_string(expr: *const typval_T, rettv: *mut typval_T) -> c_int {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut s = tv_get_string_buf_chk(expr, buf.as_mut_ptr()).cast_mut();
        if s.is_null() {
            return FAIL;
        }

        s = skipwhite(s);
        if eval1_emsg(&raw mut s, rettv, ptr::null_mut()) == FAIL {
            return FAIL;
        }

        if *skipwhite(s) != NUL {
            tv_clear(rettv);
            semsg!(gettext((&raw const e_invexpr2).cast()), s);
            return FAIL;
        }

        OK
    }
}

/// Evaluate an expression, which can be a function, partial or string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_typval(
    expr: *const typval_T,
    want_func: bool,
    argv: *mut typval_T,
    argc: c_int,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        if (*expr).v_type == VarType::VAR_PARTIAL {
            return eval_expr_partial(expr, argv, argc, rettv);
        }
        if (*expr).v_type == VarType::VAR_FUNC || want_func {
            return eval_expr_func(expr, argv, argc, rettv);
        }

        eval_expr_string(expr, rettv)
    }
}

/// Like eval_to_bool() but using a typval_T instead of a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_to_bool(expr: *const typval_T, error: *mut bool) -> bool {
    unsafe {
        let mut argv = tv_initial_value();
        let mut rettv = tv_initial_value();

        if eval_expr_typval(expr, false, &raw mut argv, 0, &raw mut rettv) == FAIL {
            *error = true;
            return false;
        }
        let res = tv_get_number_chk(&raw const rettv, error) != 0;
        tv_clear(&raw mut rettv);
        res
    }
}

/// Top level evaluation function, returning a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_string_skip(
    arg: *mut c_char,
    eap: *mut exarg_T,
    skip: bool,
) -> *mut c_char {
    unsafe {
        let mut tv = tv_initial_value();
        let retval: *mut c_char;
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, skip);
        if skip {
            emsg_skip += 1;
        }
        if eval0(arg, &raw mut tv, eap, &raw mut evalarg) == FAIL || skip {
            retval = ptr::null_mut();
        } else {
            retval = xstrdup(tv_get_string(&raw const tv));
            tv_clear(&raw mut tv);
        }
        if skip {
            emsg_skip -= 1;
        }
        clear_evalarg(&raw mut evalarg, eap);

        retval
    }
}

/// Skip over an expression at "*pp".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn skip_expr(pp: *mut *mut c_char, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        let save_flags = if evalarg.is_null() {
            0
        } else {
            (*evalarg).eval_flags
        };

        if !evalarg.is_null() {
            (*evalarg).eval_flags &= !EVAL_EVALUATE;
        }

        *pp = skipwhite(*pp);
        let mut rettv = tv_initial_value();
        let res = eval1(pp, &raw mut rettv, ptr::null_mut());

        if !evalarg.is_null() {
            (*evalarg).eval_flags = save_flags;
        }

        res
    }
}

/// Convert "tv" to a string.
unsafe fn typval2string(tv: *mut typval_T, join_list: bool) -> *mut c_char {
    unsafe {
        if join_list && (*tv).v_type == VarType::VAR_LIST {
            let mut ga = garray_T {
                ga_len: 0,
                ga_maxlen: 0,
                ga_itemsize: 0,
                ga_growsize: 0,
                ga_data: ptr::null_mut(),
            };
            ga_init(&raw mut ga, size_of::<c_char>() as c_int, 80);
            if !(*tv).vval.v_list.is_null() {
                tv_list_join(&raw mut ga, (*tv).vval.v_list, c"\n".as_ptr());
                if tv_list_len((*tv).vval.v_list) > 0 {
                    ga_append(&raw mut ga, NL as u8);
                }
            }
            ga_append(&raw mut ga, NUL as u8);
            return ga.ga_data.cast::<c_char>();
        } else if (*tv).v_type == VarType::VAR_LIST || (*tv).v_type == VarType::VAR_DICT {
            return encode_tv2string(tv, ptr::null_mut());
        }
        xstrdup(tv_get_string(tv))
    }
}

/// Top level evaluation function, returning a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_string_eap(
    arg: *mut c_char,
    join_list: bool,
    eap: *mut exarg_T,
    use_simple_function: bool,
) -> *mut c_char {
    unsafe {
        let mut tv = tv_initial_value();
        let retval: *mut c_char;
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, !eap.is_null() && (*eap).skip != 0);
        let r = if use_simple_function {
            eval0_simple_funccal(arg, &raw mut tv, ptr::null_mut(), &raw mut evalarg)
        } else {
            eval0(arg, &raw mut tv, ptr::null_mut(), &raw mut evalarg)
        };
        if r == FAIL {
            retval = ptr::null_mut();
        } else {
            retval = typval2string(&raw mut tv, join_list);
            tv_clear(&raw mut tv);
        }
        clear_evalarg(&raw mut evalarg, ptr::null_mut());

        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_string(
    arg: *mut c_char,
    join_list: bool,
    use_simple_function: bool,
) -> *mut c_char {
    unsafe { eval_to_string_eap(arg, join_list, ptr::null_mut(), use_simple_function) }
}

/// Call eval_to_string() without using current local variables and using textlock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_string_safe(
    arg: *mut c_char,
    use_sandbox: bool,
    use_simple_function: bool,
) -> *mut c_char {
    unsafe {
        let mut funccal_entry: funccal_entry_T = std::mem::zeroed();

        save_funccal(&raw mut funccal_entry);
        {
            if use_sandbox {
                sandbox += 1;
            }
            textlock += 1;
        }
        let retval = eval_to_string(arg, false, use_simple_function);
        {
            if use_sandbox {
                sandbox -= 1;
            }
            textlock -= 1;
        }
        restore_funccal();
        retval
    }
}

/// Top level evaluation function, returning a number.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_to_number(
    expr: *mut c_char,
    use_simple_function: bool,
) -> varnumber_T {
    unsafe {
        let mut rettv = tv_initial_value();
        let retval: varnumber_T;
        let mut p = skipwhite(expr);
        let mut r = NOTDONE;

        {
            emsg_off += 1;
        }

        if use_simple_function {
            r = may_call_simple_func(expr, &raw mut rettv);
        }
        if r == NOTDONE {
            let evalarg_ptr = &raw mut EVALARG_EVALUATE;
            r = eval1(&raw mut p, &raw mut rettv, evalarg_ptr);
        }
        if r == FAIL {
            retval = -1;
        } else {
            retval = tv_get_number_chk(&raw const rettv, ptr::null_mut());
            tv_clear(&raw mut rettv);
        }
        {
            emsg_off -= 1;
        }

        retval
    }
}

/// Top level evaluation function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr(arg: *mut c_char, eap: *mut exarg_T) -> *mut typval_T {
    unsafe { eval_expr_ext(arg, eap, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_ext(
    arg: *mut c_char,
    eap: *mut exarg_T,
    use_simple_function: bool,
) -> *mut typval_T {
    unsafe {
        let mut tv = xmalloc(size_of::<typval_T>()).cast::<typval_T>();
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, !eap.is_null() && (*eap).skip != 0);

        let mut r = NOTDONE;

        if use_simple_function {
            r = eval0_simple_funccal(arg, tv, eap, &raw mut evalarg);
        }
        if r == NOTDONE {
            r = eval0(arg, tv, eap, &raw mut evalarg);
        }

        if r == FAIL {
            xfree(tv.cast());
            tv = ptr::null_mut();
        }

        clear_evalarg(&raw mut evalarg, eap);
        tv
    }
}

/// Call some Vim script function and return the result in "*rettv".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn call_vim_function(
    mut func: *const c_char,
    argc: c_int,
    argv: *mut typval_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let mut len = strlen(func) as c_int;
        let mut pt: *mut partial_T = ptr::null_mut();

        if len >= 6 && std::slice::from_raw_parts(func.cast::<u8>(), 6) == b"v:lua." {
            func = func.add(6);
            len = check_luafunc_name(func, false);
            if len == 0 {
                tv_clear(rettv);
                return FAIL;
            }
            pt = get_vim_var_partial(VV_LUA);
        }

        (*rettv).v_type = VarType::VAR_UNKNOWN;
        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: true,
            fe_partial: pt.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        let ret = call_func(func, len, rettv, argc, argv, &raw mut funcexe);

        if ret == FAIL {
            tv_clear(rettv);
        }

        ret
    }
}

/// Call Vim script function and return the result as a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn call_func_retstr(
    func: *const c_char,
    argc: c_int,
    argv: *mut typval_T,
) -> *mut c_void {
    unsafe {
        let mut rettv = tv_initial_value();
        if call_vim_function(func, argc, argv, &raw mut rettv) == FAIL {
            return ptr::null_mut();
        }

        let retval = xstrdup(tv_get_string(&raw const rettv));
        tv_clear(&raw mut rettv);
        retval.cast()
    }
}

/// Call Vim script function and return the result as a List.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn call_func_retlist(
    func: *const c_char,
    argc: c_int,
    argv: *mut typval_T,
) -> *mut c_void {
    unsafe {
        let mut rettv = tv_initial_value();

        if call_vim_function(func, argc, argv, &raw mut rettv) == FAIL {
            return ptr::null_mut();
        }

        if rettv.v_type != VarType::VAR_LIST {
            tv_clear(&raw mut rettv);
            return ptr::null_mut();
        }

        rettv.vval.v_list.cast()
    }
}

/// Evaluate a number-valued "expr" option ('indentexpr'/'formatexpr') via its callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_option_number(cb: *mut Callback, use_sandbox: bool) -> c_int {
    unsafe {
        let mut copy = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        callback_copy(&raw mut copy, cb);
        if use_sandbox {
            sandbox += 1;
        }
        emsg_off += 1;
        let mut tv = tv_initial_value();
        let mut argv = [tv_initial_value()];
        let mut retval = -1;
        if callback_call(&raw mut copy, 0, argv.as_mut_ptr(), &raw mut tv) {
            retval = tv_get_number_chk(&raw const tv, ptr::null_mut()) as c_int;
            tv_clear(&raw mut tv);
        }
        emsg_off -= 1;
        if use_sandbox {
            sandbox -= 1;
        }
        callback_free(&raw mut copy);
        retval
    }
}

/// Evaluate a "expr" option ('foldtext'/'includeexpr') via its callback, under |textlock| and an
/// isolated funccal stack; evaluates in the |sandbox| when `use_sandbox`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_expr_option_tv(
    cb: *mut Callback,
    use_sandbox: bool,
    ret_tv: *mut typval_T,
) -> bool {
    unsafe {
        let mut funccal_entry: funccal_entry_T = std::mem::zeroed();
        save_funccal(&raw mut funccal_entry);
        {
            if use_sandbox {
                sandbox += 1;
            }
            textlock += 1;
        }
        let mut argv = [tv_initial_value()];
        let ok = callback_call(cb, 0, argv.as_mut_ptr(), ret_tv);
        {
            textlock -= 1;
            if use_sandbox {
                sandbox -= 1;
            }
        }
        restore_funccal();
        ok
    }
}

/// Evaluate 'foldexpr'. Returns the foldlevel, and any character preceding
/// it in "*cp". Doesn't give error messages.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_foldexpr(wp: *mut win_T, cp: *mut c_int) -> c_int {
    unsafe {
        let saved_sctx = current_sctx;
        let use_sandbox = was_set_insecurely(wp, kOptFoldexpr, OPT_LOCAL) != 0;

        current_sctx = (*wp).w_onebuf_opt.wo_script_ctx[kWinOptFoldexpr];

        emsg_off += 1;
        if use_sandbox {
            sandbox += 1;
        }
        textlock += 1;
        *cp = NUL as c_int;

        let mut tv = tv_initial_value();
        let mut argv = [tv_initial_value()];
        let retval: varnumber_T;
        if !callback_call(
            (&raw mut (*wp).w_onebuf_opt.wo_fde).cast(),
            0,
            argv.as_mut_ptr(),
            &raw mut tv,
        ) {
            retval = 0;
        } else {
            if tv.v_type == VarType::VAR_NUMBER {
                retval = tv.vval.v_number;
            } else if tv.v_type != VarType::VAR_STRING || tv.vval.v_string.is_null() {
                retval = 0;
            } else {
                let mut s = tv.vval.v_string;
                if *s != NUL && !ascii_isdigit(*s) && *s != b'-' as c_char {
                    *cp = c_int::from(*s as u8);
                    s = s.add(1);
                }
                retval = varnumber_T::from(atol(s));
            }
            tv_clear(&raw mut tv);
        }

        emsg_off -= 1;
        if use_sandbox {
            sandbox -= 1;
        }
        textlock -= 1;
        current_sctx = saved_sctx;

        retval as c_int
    }
}

/// Evaluate 'foldtext', returning an Array or a String (NULL_STRING on failure).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_foldtext(wp: *mut win_T) -> Object {
    unsafe {
        let use_sandbox = was_set_insecurely(wp, kOptFoldtext, OPT_LOCAL) != 0;
        let mut tv = tv_initial_value();
        let retval: Object;
        if !eval_expr_option_tv(
            (&raw mut (*wp).w_onebuf_opt.wo_fdt).cast(),
            use_sandbox,
            &raw mut tv,
        ) {
            retval = Object {
                obj_type: ObjectType::kObjectTypeString,
                data: ObjectData {
                    string: NvimString {
                        data: ptr::null_mut(),
                        size: 0,
                    },
                },
            };
        } else {
            if tv.v_type == VarType::VAR_LIST {
                retval = vim_to_object(&raw mut tv, ptr::null_mut(), false);
            } else {
                retval = Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_to_string(tv_get_string(&raw const tv)),
                    },
                };
            }
            tv_clear(&raw mut tv);
        }
        retval
    }
}

/// Find the end of a variable or function name. Unlike find_name_end() this
/// does not recognize magic braces.
unsafe fn to_name_end(arg: *const c_char, use_namespace: bool) -> *const c_char {
    unsafe {
        if !eval_isnamec1(c_int::from(*arg as u8)) {
            return arg;
        }

        let mut p = arg.add(1);
        while *p != NUL && eval_isnamec(c_int::from(*p as u8)) {
            if *p == b':' as c_char
                && (p != arg.add(1)
                    || !use_namespace
                    || vim_strchr(c"bgstvw".as_ptr(), c_int::from(*arg as u8)).is_null())
            {
                break;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        p
    }
}

unsafe fn get_lval_dict_item(
    lp: *mut lval_T,
    name: *mut c_char,
    mut key: *mut c_char,
    len: c_int,
    key_end: *mut *mut c_char,
    var1: *mut typval_T,
    flags: c_int,
    unlet: bool,
    rettv: *mut typval_T,
) -> glv_status_T {
    unsafe {
        let quiet = (flags & GLV_QUIET) != 0;
        let p = *key_end;

        if len == -1 {
            key = tv_get_string(var1).cast_mut();
        }
        (*lp).ll_list = ptr::null_mut();

        if (*(*lp).ll_tv).vval.v_dict.is_null() {
            (*(*lp).ll_tv).vval.v_dict = tv_dict_alloc();
            (*(*(*lp).ll_tv).vval.v_dict).dv_refcount += 1;
        }
        (*lp).ll_dict = (*(*lp).ll_tv).vval.v_dict;

        (*lp).ll_di = tv_dict_find((*lp).ll_dict, key, len as isize);

        if !rettv.is_null() && (*(*lp).ll_dict).dv_scope != 0 {
            let prevval = if len != -1 {
                let pv = *key.add(len as usize);
                *key.add(len as usize) = NUL;
                pv
            } else {
                0
            };
            let wrong = ((*(*lp).ll_dict).dv_scope == VAR_DEF_SCOPE
                && tv_is_func(*rettv)
                && var_wrong_func_name(key, (*lp).ll_di.is_null()))
                || !valid_varname(key);
            if len != -1 {
                *key.add(len as usize) = prevval;
            }
            if wrong {
                return GLV_FAIL;
            }
        }

        if !(*lp).ll_di.is_null()
            && tv_is_luafunc(&raw mut (*(*lp).ll_di).di_tv)
            && len == -1
            && rettv.is_null()
        {
            semsg!((&raw const e_illvar).cast(), c"v:['lua']".as_ptr());
            return GLV_FAIL;
        }

        if (*lp).ll_di.is_null() {
            if (*lp).ll_dict == get_vimvar_dict()
                || ptr::eq(&raw mut (*(*lp).ll_dict).dv_hashtab, get_funccal_args_ht())
            {
                semsg!(gettext((&raw const e_illvar).cast()), name);
                return GLV_FAIL;
            }

            if *p == b'[' as c_char || *p == b'.' as c_char || unlet {
                if !quiet {
                    semsg!(gettext((&raw const e_dictkey).cast()), key);
                }
                return GLV_FAIL;
            }
            if len == -1 {
                (*lp).ll_newkey = xstrdup(key);
            } else {
                (*lp).ll_newkey = xmemdupz(key.cast(), len as usize).cast();
            }
            *key_end = p;
            return GLV_STOP;
        } else if (flags & GLV_READ_ONLY) == 0
            && (var_check_ro(
                c_int::from((*(*lp).ll_di).di_flags),
                name,
                (p as usize).wrapping_sub(name as usize),
            ) || var_check_lock(
                c_int::from((*(*lp).ll_di).di_flags),
                name,
                (p as usize).wrapping_sub(name as usize),
            ))
        {
            return GLV_FAIL;
        }

        (*lp).ll_tv = &raw mut (*(*lp).ll_di).di_tv;

        GLV_OK
    }
}

unsafe fn get_lval_blob(
    lp: *mut lval_T,
    var1: *mut typval_T,
    var2: *mut typval_T,
    empty1: bool,
    quiet: bool,
) -> c_int {
    unsafe {
        let bloblen = tv_blob_len((*(*lp).ll_tv).vval.v_blob);

        if empty1 {
            (*lp).ll_n1 = 0;
        } else {
            (*lp).ll_n1 = tv_get_number(var1) as c_int;
        }

        if tv_blob_check_index(bloblen, varnumber_T::from((*lp).ll_n1), quiet) == FAIL {
            return FAIL;
        }
        if (*lp).ll_range && !(*lp).ll_empty2 {
            (*lp).ll_n2 = tv_get_number(var2) as c_int;
            if tv_blob_check_range(
                bloblen,
                varnumber_T::from((*lp).ll_n1),
                varnumber_T::from((*lp).ll_n2),
                quiet,
            ) == FAIL
            {
                return FAIL;
            }
        }

        (*lp).ll_blob = (*(*lp).ll_tv).vval.v_blob;
        (*lp).ll_tv = ptr::null_mut();

        OK
    }
}

unsafe fn get_lval_list(
    lp: *mut lval_T,
    var1: *mut typval_T,
    var2: *mut typval_T,
    empty1: bool,
    _flags: c_int,
    quiet: bool,
) -> c_int {
    unsafe {
        if empty1 {
            (*lp).ll_n1 = 0;
        } else {
            (*lp).ll_n1 = tv_get_number(var1) as c_int;
        }

        (*lp).ll_dict = ptr::null_mut();
        (*lp).ll_list = (*(*lp).ll_tv).vval.v_list;
        (*lp).ll_li = tv_list_check_range_index_one((*lp).ll_list, &raw mut (*lp).ll_n1, quiet);
        if (*lp).ll_li.is_null() {
            return FAIL;
        }

        if (*lp).ll_range && !(*lp).ll_empty2 {
            (*lp).ll_n2 = tv_get_number(var2) as c_int;
            if tv_list_check_range_index_two(
                (*lp).ll_list,
                &raw mut (*lp).ll_n1,
                (*lp).ll_li,
                &raw mut (*lp).ll_n2,
                quiet,
            ) == FAIL
            {
                return FAIL;
            }
        }

        (*lp).ll_tv = tv_list_item_tv((*lp).ll_li);

        OK
    }
}

unsafe fn get_lval_subscript(
    lp: *mut lval_T,
    mut p: *mut c_char,
    name: *mut c_char,
    rettv: *mut typval_T,
    _ht: *mut hashtab_T,
    _v: *mut dictitem_T,
    unlet: bool,
    flags: c_int,
) -> *mut c_char {
    unsafe {
        let quiet = (flags & GLV_QUIET) != 0;
        let mut var1 = tv_initial_value();
        let mut var2 = tv_initial_value();
        let mut empty1 = false;

        while *p == b'[' as c_char
            || (*p == b'.' as c_char && *p.add(1) != b'=' as c_char && *p.add(1) != b'.' as c_char)
        {
            if *p == b'.' as c_char && (*(*lp).ll_tv).v_type != VarType::VAR_DICT {
                if !quiet {
                    semsg!(
                        gettext(E_DOT_CAN_ONLY_BE_USED_ON_DICTIONARY_STR.as_ptr()),
                        name,
                    );
                }
                return ptr::null_mut();
            }
            if (*(*lp).ll_tv).v_type != VarType::VAR_LIST
                && (*(*lp).ll_tv).v_type != VarType::VAR_DICT
                && (*(*lp).ll_tv).v_type != VarType::VAR_BLOB
            {
                if !quiet {
                    emsg(gettext(
                        c"E689: Can only index a List, Dictionary or Blob".as_ptr(),
                    ));
                }
                return ptr::null_mut();
            }

            if (*(*lp).ll_tv).v_type == VarType::VAR_LIST && (*(*lp).ll_tv).vval.v_list.is_null() {
                tv_list_alloc_ret((*lp).ll_tv, kListLenUnknown);
            } else if (*(*lp).ll_tv).v_type == VarType::VAR_BLOB
                && (*(*lp).ll_tv).vval.v_blob.is_null()
            {
                tv_blob_alloc_ret((*lp).ll_tv);
            }

            if (*lp).ll_range {
                if !quiet {
                    emsg(gettext(c"E708: [:] must come last".as_ptr()));
                }
                tv_clear(&raw mut var1);
                tv_clear(&raw mut var2);
                return ptr::null_mut();
            }

            let mut len: c_int = -1;
            let mut key: *mut c_char = ptr::null_mut();
            if *p == b'.' as c_char {
                key = p.add(1);
                len = 0;
                while ascii_isalnum(*key.add(len as usize))
                    || *key.add(len as usize) == b'_' as c_char
                {
                    len += 1;
                }
                if len == 0 {
                    if !quiet {
                        emsg(gettext(c"E713: Cannot use empty key after .".as_ptr()));
                    }
                    return ptr::null_mut();
                }
                p = key.add(len as usize);
            } else {
                p = skipwhite(p.add(1));
                if *p == b':' as c_char {
                    empty1 = true;
                } else {
                    empty1 = false;
                    let evalarg_ptr = &raw mut EVALARG_EVALUATE;
                    if eval1(&raw mut p, &raw mut var1, evalarg_ptr) == FAIL {
                        tv_clear(&raw mut var1);
                        tv_clear(&raw mut var2);
                        return ptr::null_mut();
                    }
                    if !tv_check_str(&raw const var1) {
                        tv_clear(&raw mut var1);
                        tv_clear(&raw mut var2);
                        return ptr::null_mut();
                    }
                    p = skipwhite(p);
                }

                if *p == b':' as c_char {
                    if (*(*lp).ll_tv).v_type == VarType::VAR_DICT {
                        if !quiet {
                            emsg(gettext(E_CANNOT_SLICE_DICTIONARY.as_ptr()));
                        }
                        tv_clear(&raw mut var1);
                        tv_clear(&raw mut var2);
                        return ptr::null_mut();
                    }
                    if !rettv.is_null()
                        && !((*rettv).v_type == VarType::VAR_LIST
                            && !(*rettv).vval.v_list.is_null())
                        && !((*rettv).v_type == VarType::VAR_BLOB
                            && !(*rettv).vval.v_blob.is_null())
                    {
                        if !quiet {
                            emsg(gettext(c"E709: [:] requires a List or Blob value".as_ptr()));
                        }
                        tv_clear(&raw mut var1);
                        tv_clear(&raw mut var2);
                        return ptr::null_mut();
                    }
                    p = skipwhite(p.add(1));
                    if *p == b']' as c_char {
                        (*lp).ll_empty2 = true;
                    } else {
                        (*lp).ll_empty2 = false;
                        let evalarg_ptr = &raw mut EVALARG_EVALUATE;
                        if eval1(&raw mut p, &raw mut var2, evalarg_ptr) == FAIL
                            || !tv_check_str(&raw const var2)
                        {
                            tv_clear(&raw mut var1);
                            tv_clear(&raw mut var2);
                            return ptr::null_mut();
                        }
                    }
                    (*lp).ll_range = true;
                } else {
                    (*lp).ll_range = false;
                }

                if *p != b']' as c_char {
                    if !quiet {
                        emsg(gettext(E_MISSBRAC.as_ptr()));
                    }
                    tv_clear(&raw mut var1);
                    tv_clear(&raw mut var2);
                    return ptr::null_mut();
                }

                p = p.add(1);
            }

            if (*(*lp).ll_tv).v_type == VarType::VAR_DICT {
                let glv_status = get_lval_dict_item(
                    lp,
                    name,
                    key,
                    len,
                    &raw mut p,
                    &raw mut var1,
                    flags,
                    unlet,
                    rettv,
                );
                if glv_status == GLV_FAIL {
                    tv_clear(&raw mut var1);
                    tv_clear(&raw mut var2);
                    return ptr::null_mut();
                }
                if glv_status == GLV_STOP {
                    break;
                }
            } else if (*(*lp).ll_tv).v_type == VarType::VAR_BLOB {
                if get_lval_blob(lp, &raw mut var1, &raw mut var2, empty1, quiet) == FAIL {
                    tv_clear(&raw mut var1);
                    tv_clear(&raw mut var2);
                    return ptr::null_mut();
                }
                break;
            } else if get_lval_list(lp, &raw mut var1, &raw mut var2, empty1, flags, quiet) == FAIL
            {
                tv_clear(&raw mut var1);
                tv_clear(&raw mut var2);
                return ptr::null_mut();
            }

            tv_clear(&raw mut var1);
            tv_clear(&raw mut var2);
            var1.v_type = VarType::VAR_UNKNOWN;
            var2.v_type = VarType::VAR_UNKNOWN;
        }

        tv_clear(&raw mut var1);
        tv_clear(&raw mut var2);
        p
    }
}

/// Get an lvalue
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_lval(
    name: *mut c_char,
    rettv: *mut typval_T,
    lp: *mut lval_T,
    unlet: bool,
    skip: bool,
    flags: c_int,
    fne_flags: c_int,
) -> *mut c_char {
    unsafe {
        let quiet = (flags & GLV_QUIET) != 0;

        ptr::write_bytes(lp, 0, 1);

        if skip {
            (*lp).ll_name = name;
            return find_name_end(
                name,
                ptr::null_mut(),
                ptr::null_mut(),
                FNE_INCL_BR | fne_flags,
            )
            .cast_mut();
        }

        let mut expr_start: *const c_char = ptr::null();
        let mut expr_end: *const c_char = ptr::null();
        let mut p =
            find_name_end(name, &raw mut expr_start, &raw mut expr_end, fne_flags).cast_mut();
        if !expr_start.is_null() {
            if unlet
                && !ascii_iswhite(*p)
                && ends_excmd(c_int::from(*p as u8)) == 0
                && *p != b'[' as c_char
                && *p != b'.' as c_char
            {
                semsg!(gettext((&raw const e_trailing_arg).cast()), p);
                return ptr::null_mut();
            }

            (*lp).ll_exp_name =
                make_expanded_name(name, expr_start.cast_mut(), expr_end.cast_mut(), p);
            (*lp).ll_name = (*lp).ll_exp_name;
            if (*lp).ll_exp_name.is_null() {
                if !aborting() && !quiet {
                    {
                        emsg_severe = true;
                        semsg!(gettext((&raw const e_invarg2).cast()), name);
                    }
                    return ptr::null_mut();
                }
                (*lp).ll_name_len = 0;
            } else {
                (*lp).ll_name_len = strlen((*lp).ll_name);
            }
        } else {
            (*lp).ll_name = name;
            (*lp).ll_name_len = (p as usize).wrapping_sub((*lp).ll_name as usize);
        }

        if (*p != b'[' as c_char && *p != b'.' as c_char) || (*lp).ll_name.is_null() {
            return p;
        }

        let mut ht: *mut hashtab_T = ptr::null_mut();

        let v = find_var(
            (*lp).ll_name,
            (*lp).ll_name_len,
            if (flags & GLV_READ_ONLY) != 0 {
                ptr::null_mut()
            } else {
                &raw mut ht
            },
            flags & GLV_NO_AUTOLOAD,
        );
        if v.is_null() && !quiet {
            semsg!(
                gettext(c"E121: Undefined variable: %.*s".as_ptr()),
                (*lp).ll_name_len as c_int,
                (*lp).ll_name,
            );
        }
        if v.is_null() {
            return ptr::null_mut();
        }

        (*lp).ll_tv = &raw mut (*v).di_tv;

        if tv_is_luafunc((*lp).ll_tv) {
            return p;
        }

        p = get_lval_subscript(lp, p, name, rettv, ht, v, unlet, flags);
        if p.is_null() {
            return ptr::null_mut();
        }

        (*lp).ll_name_len = (p as usize).wrapping_sub((*lp).ll_name as usize);
        p
    }
}

/// Clear lval "lp" that was filled by get_lval().
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clear_lval(lp: *mut lval_T) {
    unsafe {
        xfree((*lp).ll_exp_name.cast());
        xfree((*lp).ll_newkey.cast());
    }
}

/// Set a variable that was parsed by get_lval() to "rettv".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_var_lval(
    lp: *mut lval_T,
    endp: *mut c_char,
    rettv: *mut typval_T,
    copy: bool,
    is_const: bool,
    op: *const c_char,
) {
    unsafe {
        if (*lp).ll_tv.is_null() {
            let cc = *endp;
            *endp = NUL;
            if !(*lp).ll_blob.is_null() {
                if !op.is_null() && *op != b'=' as c_char {
                    semsg!(gettext((&raw const e_letwrong).cast()), op);
                    *endp = cc;
                    return;
                }
                if value_check_lock((*(*lp).ll_blob).bv_lock, (*lp).ll_name, TV_CSTRING) {
                    *endp = cc;
                    return;
                }

                if (*lp).ll_range && (*rettv).v_type == VarType::VAR_BLOB {
                    if (*lp).ll_empty2 {
                        (*lp).ll_n2 = tv_blob_len((*lp).ll_blob) - 1;
                    }

                    if tv_blob_set_range(
                        (*lp).ll_blob,
                        varnumber_T::from((*lp).ll_n1),
                        varnumber_T::from((*lp).ll_n2),
                        rettv,
                    ) == FAIL
                    {
                        *endp = cc;
                        return;
                    }
                } else {
                    let mut error = false;
                    let val = tv_get_number_chk(rettv, &raw mut error);
                    if !error {
                        if !(0..=255).contains(&val) {
                            semsg!(
                                gettext((&raw const e_invalid_value_for_blob_nr).cast()),
                                val,
                            );
                        } else {
                            tv_blob_set_append((*lp).ll_blob, (*lp).ll_n1, val as u8);
                        }
                    }
                }
            } else if !op.is_null() && *op != b'=' as c_char {
                let mut tv = tv_initial_value();

                if is_const {
                    emsg(gettext((&raw const e_cannot_mod).cast()));
                    *endp = cc;
                    return;
                }

                let mut di: *mut dictitem_T = ptr::null_mut();
                if eval_variable(
                    (*lp).ll_name,
                    (*lp).ll_name_len as c_int,
                    &raw mut tv,
                    &raw mut di,
                    true,
                    false,
                ) == OK
                {
                    if (di.is_null()
                        || (!var_check_ro(c_int::from((*di).di_flags), (*lp).ll_name, TV_CSTRING)
                            && !tv_check_lock(&raw const (*di).di_tv, (*lp).ll_name, TV_CSTRING)))
                        && eexe_mod_op(&raw mut tv, rettv, op) == OK
                    {
                        set_var((*lp).ll_name, (*lp).ll_name_len, &raw mut tv, false);
                    }
                    tv_clear(&raw mut tv);
                }
            } else {
                set_var_const((*lp).ll_name, (*lp).ll_name_len, rettv, copy, is_const);
            }
            *endp = cc;
        } else if value_check_lock(
            if (*lp).ll_newkey.is_null() {
                (*(*lp).ll_tv).v_lock
            } else {
                (*(*(*lp).ll_tv).vval.v_dict).dv_lock
            },
            (*lp).ll_name,
            TV_CSTRING,
        ) {
            // Skip
        } else if (*lp).ll_range {
            if is_const {
                emsg(gettext(c"E996: Cannot lock a range".as_ptr()));
                return;
            }

            tv_list_assign_range(
                (*lp).ll_list,
                (*rettv).vval.v_list,
                (*lp).ll_n1,
                (*lp).ll_n2,
                (*lp).ll_empty2,
                op,
                (*lp).ll_name,
            );
        } else {
            let mut oldtv = tv_initial_value();
            let dict = (*lp).ll_dict;
            let watched = tv_dict_is_watched(dict);

            if is_const {
                emsg(gettext(c"E996: Cannot lock a list or dict".as_ptr()));
                return;
            }

            let mut skip_assign = false;
            if !(*lp).ll_newkey.is_null() {
                if !op.is_null() && *op != b'=' as c_char {
                    semsg!(gettext((&raw const e_dictkey).cast()), (*lp).ll_newkey);
                    return;
                }
                if tv_dict_wrong_func_name((*(*lp).ll_tv).vval.v_dict, rettv, (*lp).ll_newkey) != 0
                {
                    return;
                }

                let di = tv_dict_item_alloc((*lp).ll_newkey);
                if tv_dict_add((*(*lp).ll_tv).vval.v_dict, di) == FAIL {
                    xfree(di.cast());
                    return;
                }
                (*lp).ll_tv = &raw mut (*di).di_tv;
            } else {
                if watched {
                    tv_copy((*lp).ll_tv, &raw mut oldtv);
                }

                if !op.is_null() && *op != b'=' as c_char {
                    eexe_mod_op((*lp).ll_tv, rettv, op);
                    skip_assign = true;
                } else {
                    tv_clear((*lp).ll_tv);
                }
            }

            if !skip_assign {
                if copy {
                    tv_copy(rettv, (*lp).ll_tv);
                } else {
                    *(*lp).ll_tv = *rettv;
                    (*(*lp).ll_tv).v_lock = VAR_UNLOCKED;
                    tv_init(rettv);
                }
            }

            if watched {
                if oldtv.v_type == VarType::VAR_UNKNOWN {
                    debug_assert!(!(*lp).ll_newkey.is_null());
                    tv_dict_watcher_notify(dict, (*lp).ll_newkey, (*lp).ll_tv, ptr::null_mut());
                } else {
                    let di_ = (*lp).ll_di;
                    let key_ptr = (&raw const (*di_).di_key).cast::<c_char>();
                    tv_dict_watcher_notify(dict, key_ptr, (*lp).ll_tv, &raw mut oldtv);
                    tv_clear(&raw mut oldtv);
                }
            }
        }
    }
}

/// Evaluate the expression used in a ":for var in expr" command.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_for_line(
    arg: *const c_char,
    errp: *mut bool,
    eap: *mut exarg_T,
    evalarg: *mut evalarg_T,
) -> *mut c_void {
    unsafe {
        let fi = xcalloc(1, size_of::<forinfo_T>()).cast::<forinfo_T>();
        let mut tv = tv_initial_value();
        let skip = ((*evalarg).eval_flags & EVAL_EVALUATE) == 0;

        *errp = true;

        let mut expr = skip_var_list(
            arg,
            &raw mut (*fi).fi_varcount,
            &raw mut (*fi).fi_semicolon,
            false,
        );
        if expr.is_null() {
            return fi.cast();
        }

        expr = skipwhite(expr);
        if *expr != b'i' as c_char
            || *expr.add(1) != b'n' as c_char
            || !(*expr.add(2) == NUL || ascii_iswhite(*expr.add(2)))
        {
            emsg(gettext(c"E690: Missing \"in\" after :for".as_ptr()));
            return fi.cast();
        }

        if skip {
            emsg_skip += 1;
        }
        expr = skipwhite(expr.add(2));
        if eval0(expr.cast_mut(), &raw mut tv, eap, evalarg) == OK {
            *errp = false;
            if !skip {
                if tv.v_type == VarType::VAR_LIST {
                    let l = tv.vval.v_list;
                    if l.is_null() {
                        tv_clear(&raw mut tv);
                    } else {
                        (*fi).fi_list = l;
                        tv_list_watch_add(l, &raw mut (*fi).fi_lw);
                        (*fi).fi_lw.lw_item = tv_list_first(l);
                    }
                } else if tv.v_type == VarType::VAR_BLOB {
                    (*fi).fi_bi = 0;
                    if !tv.vval.v_blob.is_null() {
                        let mut btv = tv_initial_value();
                        tv_blob_copy(tv.vval.v_blob, &raw mut btv);
                        (*fi).fi_blob = btv.vval.v_blob;
                    }
                    tv_clear(&raw mut tv);
                } else if tv.v_type == VarType::VAR_STRING {
                    (*fi).fi_byte_idx = 0;
                    (*fi).fi_string = tv.vval.v_string;
                    if (*fi).fi_string.is_null() {
                        (*fi).fi_string = xstrdup(c"".as_ptr());
                    }
                } else {
                    emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED.as_ptr()));
                    tv_clear(&raw mut tv);
                }
            }
        }
        if skip {
            emsg_skip -= 1;
        }

        fi.cast()
    }
}

/// Use the first item in a ":for" list. Advance to the next.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn next_for_item(fi_void: *mut c_void, arg: *mut c_char) -> bool {
    unsafe {
        let fi = fi_void.cast::<forinfo_T>();

        if !(*fi).fi_blob.is_null() {
            if (*fi).fi_bi >= tv_blob_len((*fi).fi_blob) {
                return false;
            }
            let mut tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_number: varnumber_T::from(tv_blob_get((*fi).fi_blob, (*fi).fi_bi)),
                },
            };
            (*fi).fi_bi += 1;
            return ex_let_vars(
                arg,
                &raw mut tv,
                1,
                (*fi).fi_semicolon,
                (*fi).fi_varcount,
                0,
                ptr::null_mut(),
            ) == OK;
        }

        if !(*fi).fi_string.is_null() {
            let len = utfc_ptr2len((*fi).fi_string.add((*fi).fi_byte_idx as usize));
            if len == 0 {
                return false;
            }
            let mut tv = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_string: xmemdupz(
                        (*fi).fi_string.add((*fi).fi_byte_idx as usize).cast(),
                        len as usize,
                    )
                    .cast(),
                },
            };
            (*fi).fi_byte_idx += len;
            let result = ex_let_vars(
                arg,
                &raw mut tv,
                1,
                (*fi).fi_semicolon,
                (*fi).fi_varcount,
                0,
                ptr::null_mut(),
            ) == OK;
            xfree(tv.vval.v_string.cast());
            return result;
        }

        let item = (*fi).fi_lw.lw_item;
        if item.is_null() {
            return false;
        }
        (*fi).fi_lw.lw_item = tv_list_item_next((*fi).fi_list, item);
        ex_let_vars(
            arg,
            tv_list_item_tv(item),
            1,
            (*fi).fi_semicolon,
            (*fi).fi_varcount,
            0,
            ptr::null_mut(),
        ) == OK
    }
}

/// Free the structure used to store info used by ":for".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_for_info(fi_void: *mut c_void) {
    unsafe {
        let fi = fi_void.cast::<forinfo_T>();

        if fi.is_null() {
            return;
        }
        if !(*fi).fi_list.is_null() {
            tv_list_watch_remove((*fi).fi_list, &raw mut (*fi).fi_lw);
            tv_list_unref((*fi).fi_list);
        } else if !(*fi).fi_blob.is_null() {
            tv_blob_unref((*fi).fi_blob);
        } else {
            xfree((*fi).fi_string.cast());
        }
        xfree(fi.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_for_expression(
    xp: *mut expand_T,
    mut arg: *mut c_char,
    cmdidx: cmdidx_T,
) {
    unsafe {
        let mut got_eq = false;

        if cmdidx == cmdidx_T::CMD_let || cmdidx == cmdidx_T::CMD_const {
            (*xp).xp_context = EXPAND_USER_VARS;
            if strpbrk(arg, c"\"'+-*/%.=!?~|&$([<>,#".as_ptr()).is_null() {
                let mut p = arg.add(strlen(arg));
                while (p as usize) >= (arg as usize) {
                    (*xp).xp_pattern = p;
                    p = p.offset(-((utf_head_off(arg, p.sub(1)) + 1) as isize));
                    if (p as usize) >= (arg as usize) && ascii_iswhite(*p) {
                        break;
                    }
                }
                return;
            }
        } else {
            (*xp).xp_context = if cmdidx == cmdidx_T::CMD_call {
                EXPAND_FUNCTIONS
            } else {
                EXPAND_EXPRESSION
            };
        }
        while {
            (*xp).xp_pattern = strpbrk(arg, c"\"'+-*/%.=!?~|&$([<>,#".as_ptr());
            !(*xp).xp_pattern.is_null()
        } {
            let mut c = c_int::from(*(*xp).xp_pattern as u8);
            if c == b'&' as c_int {
                c = c_int::from(*(*xp).xp_pattern.add(1) as u8);
                if c == b'&' as c_int {
                    (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                    (*xp).xp_context = if cmdidx != cmdidx_T::CMD_let || got_eq {
                        EXPAND_EXPRESSION
                    } else {
                        EXPAND_NOTHING
                    };
                } else if c != b' ' as c_int {
                    (*xp).xp_context = EXPAND_SETTINGS;
                    if (c == b'l' as c_int || c == b'g' as c_int)
                        && *(*xp).xp_pattern.add(2) == b':' as c_char
                    {
                        (*xp).xp_pattern = (*xp).xp_pattern.add(2);
                    }
                }
            } else if c == b'$' as c_int {
                (*xp).xp_context = EXPAND_ENV_VARS;
            } else if c == b'=' as c_int {
                got_eq = true;
                (*xp).xp_context = EXPAND_EXPRESSION;
            } else if (c == b'#' as c_int && (*xp).xp_context == EXPAND_EXPRESSION)
                || ((c == b'<' as c_int || c == b'#' as c_int)
                    && (*xp).xp_context == EXPAND_FUNCTIONS
                    && vim_strchr((*xp).xp_pattern, b'(' as c_int).is_null())
            {
                break;
            } else if cmdidx != cmdidx_T::CMD_let || got_eq {
                if c == b'"' as c_int {
                    loop {
                        (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                        c = c_int::from(*(*xp).xp_pattern as u8);
                        if c == NUL as c_int || c == b'"' as c_int {
                            break;
                        }
                        if c == b'\\' as c_int && *(*xp).xp_pattern.add(1) != NUL {
                            (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                        }
                    }
                    (*xp).xp_context = EXPAND_NOTHING;
                } else if c == b'\'' as c_int {
                    loop {
                        (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                        c = c_int::from(*(*xp).xp_pattern as u8);
                        if c == NUL as c_int || c == b'\'' as c_int {
                            break;
                        }
                    }
                    (*xp).xp_context = EXPAND_NOTHING;
                } else if c == b'|' as c_int {
                    if *(*xp).xp_pattern.add(1) == b'|' as c_char {
                        (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                        (*xp).xp_context = EXPAND_EXPRESSION;
                    } else {
                        (*xp).xp_context = EXPAND_COMMANDS;
                    }
                } else {
                    (*xp).xp_context = EXPAND_EXPRESSION;
                }
            } else {
                (*xp).xp_context = EXPAND_EXPRESSION;
            }
            arg = (*xp).xp_pattern;
            if *arg != NUL {
                loop {
                    arg = arg.add(1);
                    c = c_int::from(*arg as u8);
                    if c == NUL as c_int || (c != b' ' as c_int && c != b'\t' as c_int) {
                        break;
                    }
                }
            }
        }

        if cmd_has_expr_args(cmdidx) && (*xp).xp_context == EXPAND_EXPRESSION {
            loop {
                let n = skiptowhite(arg);
                if n == arg || ascii_iswhite_or_nul(*skipwhite(n)) {
                    break;
                }
                arg = skipwhite(n);
            }
        }

        (*xp).xp_pattern = arg;
    }
}

/// Does not use 'cpo' and always uses 'magic'.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pattern_match(pat: *const c_char, text: *const c_char, ic: bool) -> c_int {
    unsafe {
        let mut matches: c_int = 0;
        let mut regmatch: regmatch_T = std::mem::zeroed();

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = ic;
            matches = c_int::from(vim_regexec_nl(&raw mut regmatch, text, 0));
            vim_regfree(regmatch.regprog);
        }
        p_cpo = save_cpo;
        matches
    }
}

/// Handle a name followed by "(". Both for just "name(arg)" and for
/// "expr->name(arg)".
unsafe fn eval_func(
    arg: *mut *mut c_char,
    evalarg: *mut evalarg_T,
    name: *mut c_char,
    name_len: c_int,
    rettv: *mut typval_T,
    flags: c_int,
    basetv: *mut typval_T,
) -> c_int {
    unsafe {
        let evaluate = (flags & EVAL_EVALUATE) != 0;
        let mut s = name;
        let mut len = name_len;
        let mut found_var = false;

        if !evaluate {
            check_vars(s, len as usize);
        }

        let mut partial: *mut partial_T = ptr::null_mut();
        s = deref_func_name(
            s,
            &raw mut len,
            &raw mut partial,
            !evaluate,
            &raw mut found_var,
        );

        s = xmemdupz(s.cast(), len as usize).cast();

        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: evaluate,
            fe_partial: partial.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: basetv,
            fe_found_var: found_var,
        };
        let mut ret = get_func_tv(s, len, rettv, arg, evalarg, &raw mut funcexe);

        xfree(s.cast());

        if (*rettv).v_type == VarType::VAR_UNKNOWN && !evaluate && **arg == b'(' as c_char {
            (*rettv).vval.v_string = tv_empty_string.0.cast_mut();
            (*rettv).v_type = VarType::VAR_FUNC;
        }

        if evaluate && aborting() {
            if ret == OK {
                tv_clear(rettv);
            }
            ret = FAIL;
        }
        ret
    }
}

/// After using "evalarg" filled from "eap": free the memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clear_evalarg(evalarg: *mut evalarg_T, eap: *mut exarg_T) {
    unsafe {
        if evalarg.is_null() {
            return;
        }

        if !(*evalarg).eval_tofree.is_null() {
            if !eap.is_null() {
                xfree((*eap).cmdline_tofree.cast());
                (*eap).cmdline_tofree = *(*eap).cmdlinep;
                *(*eap).cmdlinep = (*evalarg).eval_tofree;
            } else {
                xfree((*evalarg).eval_tofree.cast());
            }
            (*evalarg).eval_tofree = ptr::null_mut();
        }
    }
}

/// Handle zero level expression.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval0(
    arg: *mut c_char,
    rettv: *mut typval_T,
    eap: *mut exarg_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        let did_emsg_before = did_emsg;
        let called_emsg_before = called_emsg;
        let mut end_error = false;

        let mut p = skipwhite(arg);
        let ret = eval1(&raw mut p, rettv, evalarg);

        if ret != FAIL {
            end_error = ends_excmd(c_int::from(*p as u8)) == 0;
        }
        if ret == FAIL || end_error {
            if ret != FAIL {
                tv_clear(rettv);
            }
            if !aborting() && did_emsg == did_emsg_before && called_emsg == called_emsg_before {
                if end_error {
                    semsg!(gettext((&raw const e_trailing_arg).cast()), p);
                } else {
                    semsg!(gettext((&raw const e_invexpr2).cast()), arg);
                }
            }

            if !eap.is_null() && !p.is_null() {
                let nextcmd = check_nextcmd(p);
                if !nextcmd.is_null() && *nextcmd != b'|' as c_char {
                    (*eap).nextcmd = nextcmd;
                }
            }
            return FAIL;
        }

        if !eap.is_null() {
            (*eap).nextcmd = check_nextcmd(p);
        }

        ret
    }
}

/// If "arg" is a simple function call without arguments then call it and return
/// the result. Otherwise return NOTDONE.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn may_call_simple_func(arg: *const c_char, rettv: *mut typval_T) -> c_int {
    unsafe {
        let parens = strstr(arg, c"()".as_ptr()).cast_const();
        let mut r = NOTDONE;

        if !parens.is_null() && *skipwhite(parens.add(2)) == NUL {
            if strnequal(arg, c"v:lua.".as_ptr(), 6) {
                let p = arg.add(6);
                if p != parens && skip_luafunc_name(p) == parens {
                    r = call_simple_luafunc(p, (parens as usize).wrapping_sub(p as usize), rettv);
                }
            } else {
                let p = if strncmp(arg, c"<SNR>".as_ptr(), 5) == 0 {
                    skipdigits(arg.add(5))
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

unsafe fn eval0_simple_funccal(
    arg: *mut c_char,
    rettv: *mut typval_T,
    eap: *mut exarg_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        let mut r = may_call_simple_func(arg, rettv);

        if r == NOTDONE {
            r = eval0(arg, rettv, eap, evalarg);
        }
        r
    }
}

/// Handle top level expression:
///      expr2 ? expr1 : expr1
///      expr2 ?? expr1
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval1(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        ptr::write_bytes(rettv, 0, 1);

        if eval2(arg, rettv, evalarg) == FAIL {
            return FAIL;
        }

        let mut p = *arg;
        if *p == b'?' as c_char {
            let op_falsy = *p.add(1) == b'?' as c_char;
            let mut local_evalarg = evalarg_T {
                eval_flags: 0,
                eval_getline: None,
                eval_cookie: ptr::null_mut(),
                eval_tofree: ptr::null_mut(),
            };
            let evalarg_used = if evalarg.is_null() {
                &raw mut local_evalarg
            } else {
                evalarg
            };
            let orig_flags = (*evalarg_used).eval_flags;
            let evaluate = ((*evalarg_used).eval_flags & EVAL_EVALUATE) != 0;

            let mut result = false;
            if evaluate {
                let mut error = false;

                if op_falsy {
                    result = tv2bool(rettv);
                } else if tv_get_number_chk(rettv, &raw mut error) != 0 {
                    result = true;
                }
                if error || !op_falsy || !result {
                    tv_clear(rettv);
                }
                if error {
                    return FAIL;
                }
            }

            if op_falsy {
                *arg = (*arg).add(1);
            }
            *arg = skipwhite((*arg).add(1));
            (*evalarg_used).eval_flags = if if op_falsy { !result } else { result } {
                orig_flags
            } else {
                orig_flags & !EVAL_EVALUATE
            };
            let mut var2 = tv_initial_value();
            if eval1(arg, &raw mut var2, evalarg_used) == FAIL {
                (*evalarg_used).eval_flags = orig_flags;
                return FAIL;
            }
            if !op_falsy || !result {
                *rettv = var2;
            }

            if !op_falsy {
                p = *arg;
                if *p != b':' as c_char {
                    emsg(gettext(c"E109: Missing ':' after '?'".as_ptr()));
                    if evaluate && result {
                        tv_clear(rettv);
                    }
                    (*evalarg_used).eval_flags = orig_flags;
                    return FAIL;
                }

                *arg = skipwhite((*arg).add(1));
                (*evalarg_used).eval_flags = if !result {
                    orig_flags
                } else {
                    orig_flags & !EVAL_EVALUATE
                };
                if eval1(arg, &raw mut var2, evalarg_used) == FAIL {
                    if evaluate && result {
                        tv_clear(rettv);
                    }
                    (*evalarg_used).eval_flags = orig_flags;
                    return FAIL;
                }
                if evaluate && !result {
                    *rettv = var2;
                }
            }

            if evalarg.is_null() {
                clear_evalarg(&raw mut local_evalarg, ptr::null_mut());
            } else {
                (*evalarg).eval_flags = orig_flags;
            }
        }

        OK
    }
}

/// Handle first level expression: `expr2 || expr2 || expr2`
unsafe fn eval2(arg: *mut *mut c_char, rettv: *mut typval_T, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        if eval3(arg, rettv, evalarg) == FAIL {
            return FAIL;
        }

        let mut p = *arg;
        if *p == b'|' as c_char && *p.add(1) == b'|' as c_char {
            let mut local_evalarg = evalarg_T {
                eval_flags: 0,
                eval_getline: None,
                eval_cookie: ptr::null_mut(),
                eval_tofree: ptr::null_mut(),
            };
            let evalarg_used = if evalarg.is_null() {
                &raw mut local_evalarg
            } else {
                evalarg
            };
            let orig_flags = (*evalarg_used).eval_flags;
            let evaluate = ((*evalarg_used).eval_flags & EVAL_EVALUATE) != 0;

            let mut result = false;

            if evaluate {
                let mut error = false;
                if tv_get_number_chk(rettv, &raw mut error) != 0 {
                    result = true;
                }
                tv_clear(rettv);
                if error {
                    return FAIL;
                }
            }

            while *p == b'|' as c_char && *p.add(1) == b'|' as c_char {
                *arg = skipwhite((*arg).add(2));
                (*evalarg_used).eval_flags = if !result {
                    orig_flags
                } else {
                    orig_flags & !EVAL_EVALUATE
                };
                let mut var2 = tv_initial_value();
                if eval3(arg, &raw mut var2, evalarg_used) == FAIL {
                    return FAIL;
                }

                if evaluate && !result {
                    let mut error = false;
                    if tv_get_number_chk(&raw const var2, &raw mut error) != 0 {
                        result = true;
                    }
                    tv_clear(&raw mut var2);
                    if error {
                        return FAIL;
                    }
                }
                if evaluate {
                    (*rettv).v_type = VarType::VAR_NUMBER;
                    (*rettv).vval.v_number = varnumber_T::from(result);
                }

                p = *arg;
            }

            if evalarg.is_null() {
                clear_evalarg(&raw mut local_evalarg, ptr::null_mut());
            } else {
                (*evalarg).eval_flags = orig_flags;
            }
        }

        OK
    }
}

/// Handle second level expression: `expr3 && expr3 && expr3`
unsafe fn eval3(arg: *mut *mut c_char, rettv: *mut typval_T, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        if eval4(arg, rettv, evalarg) == FAIL {
            return FAIL;
        }

        let mut p = *arg;
        if *p == b'&' as c_char && *p.add(1) == b'&' as c_char {
            let mut local_evalarg = evalarg_T {
                eval_flags: 0,
                eval_getline: None,
                eval_cookie: ptr::null_mut(),
                eval_tofree: ptr::null_mut(),
            };
            let evalarg_used = if evalarg.is_null() {
                &raw mut local_evalarg
            } else {
                evalarg
            };
            let orig_flags = (*evalarg_used).eval_flags;
            let evaluate = ((*evalarg_used).eval_flags & EVAL_EVALUATE) != 0;

            let mut result = true;

            if evaluate {
                let mut error = false;
                if tv_get_number_chk(rettv, &raw mut error) == 0 {
                    result = false;
                }
                tv_clear(rettv);
                if error {
                    return FAIL;
                }
            }

            while *p == b'&' as c_char && *p.add(1) == b'&' as c_char {
                *arg = skipwhite((*arg).add(2));
                (*evalarg_used).eval_flags = if result {
                    orig_flags
                } else {
                    orig_flags & !EVAL_EVALUATE
                };
                let mut var2 = tv_initial_value();
                if eval4(arg, &raw mut var2, evalarg_used) == FAIL {
                    return FAIL;
                }

                if evaluate && result {
                    let mut error = false;
                    if tv_get_number_chk(&raw const var2, &raw mut error) == 0 {
                        result = false;
                    }
                    tv_clear(&raw mut var2);
                    if error {
                        return FAIL;
                    }
                }
                if evaluate {
                    (*rettv).v_type = VarType::VAR_NUMBER;
                    (*rettv).vval.v_number = varnumber_T::from(result);
                }

                p = *arg;
            }

            if evalarg.is_null() {
                clear_evalarg(&raw mut local_evalarg, ptr::null_mut());
            } else {
                (*evalarg).eval_flags = orig_flags;
            }
        }

        OK
    }
}

/// Handle third level expression (comparisons).
unsafe fn eval4(arg: *mut *mut c_char, rettv: *mut typval_T, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        let mut var2 = tv_initial_value();
        let mut expr_type: exprtype_T = EXPR_UNKNOWN;
        let mut len: usize = 2;

        if eval5(arg, rettv, evalarg) == FAIL {
            return FAIL;
        }

        let p = *arg;
        match *p as u8 {
            b'=' => {
                if *p.add(1) == b'=' as c_char {
                    expr_type = EXPR_EQUAL;
                } else if *p.add(1) == b'~' as c_char {
                    expr_type = EXPR_MATCH;
                }
            }
            b'!' => {
                if *p.add(1) == b'=' as c_char {
                    expr_type = EXPR_NEQUAL;
                } else if *p.add(1) == b'~' as c_char {
                    expr_type = EXPR_NOMATCH;
                }
            }
            b'>' => {
                if *p.add(1) != b'=' as c_char {
                    expr_type = EXPR_GREATER;
                    len = 1;
                } else {
                    expr_type = EXPR_GEQUAL;
                }
            }
            b'<' => {
                if *p.add(1) != b'=' as c_char {
                    expr_type = EXPR_SMALLER;
                    len = 1;
                } else {
                    expr_type = EXPR_SEQUAL;
                }
            }
            b'i' if *p.add(1) == b's' as c_char => {
                if *p.add(2) == b'n' as c_char
                    && *p.add(3) == b'o' as c_char
                    && *p.add(4) == b't' as c_char
                {
                    len = 5;
                }
                let next_c = *p.add(len);
                if !ascii_isalnum(next_c) && next_c != b'_' as c_char {
                    expr_type = if len == 2 { EXPR_IS } else { EXPR_ISNOT };
                }
            }
            _ => {}
        }

        if expr_type != EXPR_UNKNOWN {
            let ic: bool;
            if *p.add(len) == b'?' as c_char {
                ic = true;
                len += 1;
            } else if *p.add(len) == b'#' as c_char {
                ic = false;
                len += 1;
            } else {
                {
                    ic = p_ic != 0;
                }
            }

            *arg = skipwhite(p.add(len));
            if eval5(arg, &raw mut var2, evalarg) == FAIL {
                tv_clear(rettv);
                return FAIL;
            }
            if !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0 {
                let ret = typval_compare(rettv, &raw mut var2, expr_type, ic);
                tv_clear(&raw mut var2);
                return ret;
            }
        }

        OK
    }
}

/// Make a copy of blob "tv1" and append blob "tv2".
unsafe fn eval_addblob(tv1: *mut typval_T, tv2: *mut typval_T) {
    unsafe {
        let b1 = (*tv1).vval.v_blob;
        let b2 = (*tv2).vval.v_blob;
        let b = tv_blob_alloc();

        let len1 = i64::from(tv_blob_len(b1));
        let len2 = i64::from(tv_blob_len(b2));
        let totallen = len1 + len2;

        if totallen >= 0 && totallen <= i64::from(c_int::MAX) {
            ga_grow(&raw mut (*b).bv_ga, totallen as c_int);
            if len1 > 0 {
                ptr::copy(
                    (*b1).bv_ga.ga_data.cast::<u8>(),
                    (*b).bv_ga.ga_data.cast::<u8>(),
                    len1 as usize,
                );
            }
            if len2 > 0 {
                ptr::copy(
                    (*b2).bv_ga.ga_data.cast::<u8>(),
                    (*b).bv_ga.ga_data.cast::<u8>().add(len1 as usize),
                    len2 as usize,
                );
            }
            (*b).bv_ga.ga_len = totallen as c_int;
        }

        tv_clear(tv1);
        tv_blob_set_ret(tv1, b);
    }
}

/// Make a copy of list "tv1" and append list "tv2".
unsafe fn eval_addlist(tv1: *mut typval_T, tv2: *mut typval_T) -> c_int {
    unsafe {
        let mut var3 = tv_initial_value();
        if tv_list_concat((*tv1).vval.v_list, (*tv2).vval.v_list, &raw mut var3) == FAIL {
            tv_clear(tv1);
            tv_clear(tv2);
            return FAIL;
        }
        tv_clear(tv1);
        *tv1 = var3;
        OK
    }
}

/// Append string "s2" to the string in "tv1".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn grow_string_tv(tv1: *mut typval_T, s2: *const c_char) -> c_int {
    unsafe {
        if (*tv1).v_type != VarType::VAR_STRING || (*tv1).vval.v_string.is_null() {
            return FAIL;
        }

        let len1 = strlen((*tv1).vval.v_string);
        let len2 = strlen(s2);
        let p = xrealloc((*tv1).vval.v_string.cast(), len1 + len2 + 1).cast::<c_char>();

        ptr::copy(s2, p.add(len1), len2 + 1);
        (*tv1).vval.v_string = p;
        OK
    }
}

/// Concatenate strings "tv1" and "tv2" and store the result in "tv1".
unsafe fn eval_concat_str(tv1: *mut typval_T, tv2: *mut typval_T) -> c_int {
    unsafe {
        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let s1 = tv_get_string_buf(tv1, buf1.as_mut_ptr());
        let s2 = tv_get_string_buf_chk(tv2, buf2.as_mut_ptr());
        if s2.is_null() {
            tv_clear(tv1);
            tv_clear(tv2);
            return FAIL;
        }

        if grow_string_tv(tv1, s2) == OK {
            return OK;
        }

        let p = concat_str(s1, s2);
        tv_clear(tv1);
        (*tv1).v_type = VarType::VAR_STRING;
        (*tv1).vval.v_string = p;

        OK
    }
}

/// Add or subtract numbers "tv1" and "tv2" and store the result in "tv1".
unsafe fn eval_addsub_number(tv1: *mut typval_T, tv2: *mut typval_T, op: c_int) -> c_int {
    unsafe {
        let mut error = false;
        let mut n1: varnumber_T;
        let n2: varnumber_T;
        let mut f1: float_T = 0.0;
        let mut f2: float_T = 0.0;

        if (*tv1).v_type == VarType::VAR_FLOAT {
            f1 = (*tv1).vval.v_float;
            n1 = 0;
        } else {
            n1 = tv_get_number_chk(tv1, &raw mut error);
            if error {
                tv_clear(tv1);
                tv_clear(tv2);
                return FAIL;
            }
            if (*tv2).v_type == VarType::VAR_FLOAT {
                f1 = n1 as float_T;
            }
        }
        if (*tv2).v_type == VarType::VAR_FLOAT {
            f2 = (*tv2).vval.v_float;
            n2 = 0;
        } else {
            n2 = tv_get_number_chk(tv2, &raw mut error);
            if error {
                tv_clear(tv1);
                tv_clear(tv2);
                return FAIL;
            }
            if (*tv1).v_type == VarType::VAR_FLOAT {
                f2 = n2 as float_T;
            }
        }
        tv_clear(tv1);

        if (*tv1).v_type == VarType::VAR_FLOAT || (*tv2).v_type == VarType::VAR_FLOAT {
            if op == b'+' as c_int {
                f1 += f2;
            } else {
                f1 -= f2;
            }
            (*tv1).v_type = VarType::VAR_FLOAT;
            (*tv1).vval.v_float = f1;
        } else {
            if op == b'+' as c_int {
                n1 = n1.wrapping_add(n2);
            } else {
                n1 = n1.wrapping_sub(n2);
            }
            (*tv1).v_type = VarType::VAR_NUMBER;
            (*tv1).vval.v_number = n1;
        }

        OK
    }
}

/// Handle fourth level expression: `+`, `-`, `.`, `..`
unsafe fn eval5(arg: *mut *mut c_char, rettv: *mut typval_T, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        if eval6(arg, rettv, evalarg, false) == FAIL {
            return FAIL;
        }

        loop {
            let op = c_int::from(**arg as u8);
            let concat = op == b'.' as c_int;
            if op != b'+' as c_int && op != b'-' as c_int && !concat {
                break;
            }

            let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
            if (op != b'+' as c_int
                || ((*rettv).v_type != VarType::VAR_LIST && (*rettv).v_type != VarType::VAR_BLOB))
                && (op == b'.' as c_int || (*rettv).v_type != VarType::VAR_FLOAT)
                && evaluate
                && ((op == b'.' as c_int && !tv_check_str(rettv))
                    || (op != b'.' as c_int && !tv_check_num(rettv)))
            {
                tv_clear(rettv);
                return FAIL;
            }

            if op == b'.' as c_int && *(*arg).add(1) == b'.' as c_char {
                *arg = (*arg).add(1);
            }
            *arg = skipwhite((*arg).add(1));
            let mut var2 = tv_initial_value();
            if eval6(arg, &raw mut var2, evalarg, op == b'.' as c_int) == FAIL {
                tv_clear(rettv);
                return FAIL;
            }

            if evaluate {
                if op == b'.' as c_int {
                    if eval_concat_str(rettv, &raw mut var2) == FAIL {
                        return FAIL;
                    }
                } else if op == b'+' as c_int
                    && (*rettv).v_type == VarType::VAR_BLOB
                    && var2.v_type == VarType::VAR_BLOB
                {
                    eval_addblob(rettv, &raw mut var2);
                } else if op == b'+' as c_int
                    && (*rettv).v_type == VarType::VAR_LIST
                    && var2.v_type == VarType::VAR_LIST
                {
                    if eval_addlist(rettv, &raw mut var2) == FAIL {
                        return FAIL;
                    }
                } else if eval_addsub_number(rettv, &raw mut var2, op) == FAIL {
                    return FAIL;
                }
                tv_clear(&raw mut var2);
            }
        }
        OK
    }
}

/// Multiply or divide or compute the modulo of numbers "tv1" and "tv2".
unsafe fn eval_multdiv_number(tv1: *mut typval_T, tv2: *mut typval_T, op: c_int) -> c_int {
    unsafe {
        let mut n1: varnumber_T;
        let n2: varnumber_T;
        let mut use_float = false;

        let mut f1: float_T = 0.0;
        let mut f2: float_T = 0.0;
        let mut error = false;
        if (*tv1).v_type == VarType::VAR_FLOAT {
            f1 = (*tv1).vval.v_float;
            use_float = true;
            n1 = 0;
        } else {
            n1 = tv_get_number_chk(tv1, &raw mut error);
        }
        tv_clear(tv1);
        if error {
            tv_clear(tv2);
            return FAIL;
        }

        if (*tv2).v_type == VarType::VAR_FLOAT {
            if !use_float {
                f1 = n1 as float_T;
                use_float = true;
            }
            f2 = (*tv2).vval.v_float;
            n2 = 0;
        } else {
            n2 = tv_get_number_chk(tv2, &raw mut error);
            tv_clear(tv2);
            if error {
                return FAIL;
            }
            if use_float {
                f2 = n2 as float_T;
            }
        }

        if use_float {
            if op == b'*' as c_int {
                f1 *= f2;
            } else if op == b'/' as c_int {
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
                emsg(gettext(c"E804: Cannot use '%' with Float".as_ptr()));
                return FAIL;
            }
            (*tv1).v_type = VarType::VAR_FLOAT;
            (*tv1).vval.v_float = f1;
        } else {
            if op == b'*' as c_int {
                n1 = n1.wrapping_mul(n2);
            } else if op == b'/' as c_int {
                n1 = num_divide(n1, n2);
            } else {
                n1 = num_modulus(n1, n2);
            }
            (*tv1).v_type = VarType::VAR_NUMBER;
            (*tv1).vval.v_number = n1;
        }

        OK
    }
}

/// Handle fifth level expression: `*`, `/`, `%`
unsafe fn eval6(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    want_string: bool,
) -> c_int {
    unsafe {
        if eval7(arg, rettv, evalarg, want_string) == FAIL {
            return FAIL;
        }

        loop {
            let op = c_int::from(**arg as u8);
            if op != b'*' as c_int && op != b'/' as c_int && op != b'%' as c_int {
                break;
            }

            let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;

            *arg = skipwhite((*arg).add(1));
            let mut var2 = tv_initial_value();
            if eval7(arg, &raw mut var2, evalarg, false) == FAIL {
                return FAIL;
            }

            if evaluate && eval_multdiv_number(rettv, &raw mut var2, op) == FAIL {
                return FAIL;
            }
        }

        OK
    }
}

/// Handle sixth level expression.
unsafe fn eval7(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    want_string: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut ret: c_int;

        (*rettv).v_type = VarType::VAR_UNKNOWN;

        let start_leader: *const c_char = *arg;
        while **arg == b'!' as c_char || **arg == b'-' as c_char || **arg == b'+' as c_char {
            *arg = skipwhite((*arg).add(1));
        }
        let mut end_leader: *const c_char = *arg;

        if EVAL7_RECURSE == MAX_EVAL7_RECURSE {
            semsg!(gettext(E_EXPRESSION_TOO_RECURSIVE_STR.as_ptr()), *arg);
            return FAIL;
        }
        EVAL7_RECURSE += 1;

        match **arg as u8 {
            b'0'..=b'9' => {
                ret = eval_number(arg, rettv, evaluate, want_string);

                if ret == OK && evaluate && (end_leader as usize) > (start_leader as usize) {
                    ret = eval7_leader(rettv, true, start_leader, &raw mut end_leader);
                }
            }
            b'"' => {
                ret = eval_string(arg, rettv, evaluate, false);
            }
            b'\'' => {
                ret = eval_lit_string(arg, rettv, evaluate, false);
            }
            b'[' => {
                ret = eval_list(arg, rettv, evalarg);
            }
            b'#' => {
                ret = eval_lit_dict(arg, rettv, evalarg);
            }
            b'{' => {
                ret = get_lambda_tv(arg, rettv, evalarg);
                if ret == NOTDONE {
                    ret = eval_dict(arg, rettv, evalarg, false);
                }
            }
            b'&' => {
                ret = eval_option(arg.cast(), rettv, evaluate);
            }
            b'$' => {
                let next_c = *(*arg).add(1);
                if next_c == b'"' as c_char || next_c == b'\'' as c_char {
                    ret = eval_interp_string(arg, rettv, evaluate);
                } else {
                    ret = eval_env_var(arg, rettv, c_int::from(evaluate));
                }
            }
            b'@' => {
                *arg = (*arg).add(1);
                if evaluate {
                    (*rettv).v_type = VarType::VAR_STRING;
                    (*rettv).vval.v_string =
                        get_reg_contents(c_int::from(**arg as u8), kGRegExprSrc).cast();
                }
                if **arg != NUL {
                    *arg = (*arg).add(1);
                }
                ret = OK;
            }
            b'(' => {
                *arg = skipwhite((*arg).add(1));

                ret = eval1(arg, rettv, evalarg);
                if **arg == b')' as c_char {
                    *arg = (*arg).add(1);
                } else if ret == OK {
                    emsg(gettext(c"E110: Missing ')'".as_ptr()));
                    tv_clear(rettv);
                    ret = FAIL;
                }
            }
            _ => {
                ret = NOTDONE;
            }
        }

        if ret == NOTDONE {
            let mut s = *arg;
            let mut alias: *mut c_char = ptr::null_mut();
            let len = get_name_len(arg.cast(), &raw mut alias, evaluate, true);
            if !alias.is_null() {
                s = alias;
            }

            if len <= 0 {
                ret = FAIL;
            } else {
                let flags = if evalarg.is_null() {
                    0
                } else {
                    (*evalarg).eval_flags
                };
                if *skipwhite(*arg) == b'(' as c_char {
                    *arg = skipwhite(*arg);
                    ret = eval_func(arg, evalarg, s, len, rettv, flags, ptr::null_mut());
                } else if evaluate {
                    ret = eval_variable(s, len, rettv, ptr::null_mut(), true, false);
                } else {
                    check_vars(s, len as usize);
                    if (*rettv).v_type == VarType::VAR_UNKNOWN
                        && !evaluate
                        && strnequal(s, c"v:lua.".as_ptr(), 6)
                    {
                        (*rettv).v_type = VarType::VAR_PARTIAL;
                        let lua_pt = get_vim_var_partial(VV_LUA);
                        tv_set_partial(rettv, lua_pt);
                        (*lua_pt).pt_refcount += 1;
                    }
                    ret = OK;
                }
            }
            xfree(alias.cast());
        }

        *arg = skipwhite(*arg);

        if ret == OK {
            ret = handle_subscript(arg.cast(), rettv, evalarg, true);
        }

        if ret == OK && evaluate && (end_leader as usize) > (start_leader as usize) {
            ret = eval7_leader(rettv, false, start_leader, &raw mut end_leader);
        }

        EVAL7_RECURSE -= 1;
        ret
    }
}

/// Apply the leading "!" and "-" before an eval7 expression to "rettv".
unsafe fn eval7_leader(
    rettv: *mut typval_T,
    numeric_only: bool,
    start_leader: *const c_char,
    end_leaderp: *mut *const c_char,
) -> c_int {
    unsafe {
        let mut end_leader = *end_leaderp;
        let mut ret = OK;
        let mut error = false;
        let mut val: varnumber_T = 0;
        let mut f: float_T = 0.0;

        if (*rettv).v_type == VarType::VAR_FLOAT {
            f = (*rettv).vval.v_float;
        } else {
            val = tv_get_number_chk(rettv, &raw mut error);
        }
        if error {
            tv_clear(rettv);
            ret = FAIL;
        } else {
            while (end_leader as usize) > (start_leader as usize) {
                end_leader = end_leader.sub(1);
                if *end_leader == b'!' as c_char {
                    if numeric_only {
                        end_leader = end_leader.add(1);
                        break;
                    }
                    if (*rettv).v_type == VarType::VAR_FLOAT {
                        (*rettv).v_type = VarType::VAR_BOOL;
                        val = if f == 0.0 {
                            kBoolVarTrue
                        } else {
                            kBoolVarFalse
                        };
                    } else {
                        val = varnumber_T::from(val == 0);
                    }
                } else if *end_leader == b'-' as c_char {
                    if (*rettv).v_type == VarType::VAR_FLOAT {
                        f = -f;
                    } else {
                        val = val.wrapping_neg();
                    }
                }
            }
            if (*rettv).v_type == VarType::VAR_FLOAT {
                tv_clear(rettv);
                (*rettv).vval.v_float = f;
            } else {
                tv_clear(rettv);
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = val;
            }
        }

        *end_leaderp = end_leader;
        ret
    }
}

/// Call the function referred to in "rettv".
unsafe fn call_func_rettv(
    arg: *mut *mut c_char,
    evalarg: *mut evalarg_T,
    rettv: *mut typval_T,
    evaluate: bool,
    selfdict: *mut dict_T,
    basetv: *mut typval_T,
    lua_funcname: *const c_char,
) -> c_int {
    unsafe {
        let mut pt: *mut partial_T = ptr::null_mut();
        let mut functv = tv_initial_value();
        let funcname: *const c_char;
        let mut is_lua = false;

        if evaluate {
            functv = *rettv;
            (*rettv).v_type = VarType::VAR_UNKNOWN;

            if functv.v_type == VarType::VAR_PARTIAL {
                pt = tv_partial(&raw const functv);
                is_lua = is_luafunc(pt);
                funcname = if is_lua {
                    lua_funcname
                } else {
                    partial_name(pt)
                };
            } else {
                funcname = functv.vval.v_string;
                if funcname.is_null() || *funcname == NUL {
                    emsg(gettext(E_EMPTY_FUNCTION_NAME.as_ptr()));
                    tv_clear(&raw mut functv);
                    return FAIL;
                }
            }
        } else {
            funcname = c"".as_ptr();
        }

        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: evaluate,
            fe_partial: pt.cast(),
            fe_selfdict: selfdict,
            fe_basetv: basetv,
            fe_found_var: false,
        };
        let ret = get_func_tv(
            funcname,
            if is_lua {
                ((*arg as usize).wrapping_sub(funcname as usize)) as c_int
            } else {
                -1
            },
            rettv,
            arg,
            evalarg,
            &raw mut funcexe,
        );

        if evaluate {
            tv_clear(&raw mut functv);
        }

        ret
    }
}

/// Evaluate "->method()".
unsafe fn eval_lambda(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        *arg = (*arg).add(2);
        let mut base = *rettv;
        (*rettv).v_type = VarType::VAR_UNKNOWN;

        let mut ret = get_lambda_tv(arg, rettv, evalarg);
        if ret != OK {
            return FAIL;
        } else if **arg != b'(' as c_char {
            if verbose {
                if *skipwhite(*arg) == b'(' as c_char {
                    emsg(gettext(E_NOWHITESPACE.as_ptr()));
                } else {
                    semsg!(
                        gettext((&raw const e_missingparen).cast()),
                        c"lambda".as_ptr(),
                    );
                }
            }
            tv_clear(rettv);
            ret = FAIL;
        } else {
            ret = call_func_rettv(
                arg,
                evalarg,
                rettv,
                evaluate,
                ptr::null_mut(),
                &raw mut base,
                ptr::null(),
            );
        }

        if evaluate {
            tv_clear(&raw mut base);
        }

        ret
    }
}

/// Evaluate "->method()" or "->v:lua.method()".
unsafe fn eval_method(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;

        *arg = (*arg).add(2);
        let mut base = *rettv;
        (*rettv).v_type = VarType::VAR_UNKNOWN;

        let mut len: c_int;
        let mut name = *arg;
        let mut lua_funcname: *mut c_char = ptr::null_mut();
        let mut alias: *mut c_char = ptr::null_mut();
        if strnequal(name, c"v:lua.".as_ptr(), 6) {
            lua_funcname = name.add(6);
            *arg = skip_luafunc_name(lua_funcname).cast_mut();
            *arg = skipwhite(*arg);
            len = ((*arg as usize).wrapping_sub(lua_funcname as usize)) as c_int;
        } else {
            len = get_name_len(arg.cast(), &raw mut alias, evaluate, true);
            if !alias.is_null() {
                name = alias;
            }
        }

        let mut tofree: *mut c_char = ptr::null_mut();
        let mut ret = OK;

        if len <= 0 {
            if verbose {
                if lua_funcname.is_null() {
                    emsg(gettext(c"E260: Missing name after ->".as_ptr()));
                } else {
                    semsg!(gettext((&raw const e_invexpr2).cast()), name);
                }
            }
            ret = FAIL;
        } else {
            *arg = skipwhite(*arg);

            let paren: *mut c_char;
            if **arg != b'(' as c_char && lua_funcname.is_null() && alias.is_null() && {
                paren = vim_strchr(*arg, b'(' as c_int);
                !paren.is_null()
            } {
                *arg = name;
                *paren = NUL;
                let mut ref_tv = tv_initial_value();
                if eval7(arg, &raw mut ref_tv, evalarg, false) == FAIL {
                    *arg = name.add(len as usize);
                    ret = FAIL;
                } else if *skipwhite(*arg) != NUL {
                    if verbose {
                        semsg!(gettext((&raw const e_trailing_arg).cast()), *arg);
                    }
                    ret = FAIL;
                } else if ref_tv.v_type == VarType::VAR_FUNC && !ref_tv.vval.v_string.is_null() {
                    name = ref_tv.vval.v_string;
                    ref_tv.vval.v_string = ptr::null_mut();
                    tofree = name;
                    len = strlen(name) as c_int;
                } else if ref_tv.v_type == VarType::VAR_PARTIAL
                    && !tv_partial(&raw const ref_tv).is_null()
                {
                    let ref_pt = tv_partial(&raw const ref_tv);
                    if (*ref_pt).pt_argc > 0 || !(*ref_pt).pt_dict.is_null() {
                        if verbose {
                            emsg(gettext(E_CANNOT_USE_PARTIAL_HERE.as_ptr()));
                        }
                        ret = FAIL;
                    } else {
                        name = xstrdup(partial_name(ref_pt));
                        tofree = name;
                        if name.is_null() {
                            ret = FAIL;
                            name = *arg;
                        } else {
                            len = strlen(name) as c_int;
                        }
                    }
                } else {
                    if verbose {
                        semsg!(gettext((&raw const e_not_callable_type_str).cast()), name);
                    }
                    ret = FAIL;
                }
                tv_clear(&raw mut ref_tv);
                *paren = b'(' as c_char;
            }

            if ret == OK {
                if **arg != b'(' as c_char {
                    if verbose {
                        semsg!(gettext((&raw const e_missingparen).cast()), name);
                    }
                    ret = FAIL;
                } else if ascii_iswhite(*(*arg).sub(1)) {
                    if verbose {
                        emsg(gettext(E_NOWHITESPACE.as_ptr()));
                    }
                    ret = FAIL;
                } else if !lua_funcname.is_null() {
                    if evaluate {
                        (*rettv).v_type = VarType::VAR_PARTIAL;
                        let lua_pt = get_vim_var_partial(VV_LUA);
                        tv_set_partial(rettv, lua_pt);
                        (*lua_pt).pt_refcount += 1;
                    }
                    ret = call_func_rettv(
                        arg,
                        evalarg,
                        rettv,
                        evaluate,
                        ptr::null_mut(),
                        &raw mut base,
                        lua_funcname,
                    );
                } else {
                    ret = eval_func(
                        arg,
                        evalarg,
                        name,
                        len,
                        rettv,
                        if evaluate { EVAL_EVALUATE } else { 0 },
                        &raw mut base,
                    );
                }
            }
        }

        if evaluate {
            tv_clear(&raw mut base);
        }
        xfree(tofree.cast());

        if !alias.is_null() {
            xfree(alias.cast());
        }

        ret
    }
}

/// Evaluate an "[expr]" or "[expr:expr]" index. Also "dict.key".
unsafe fn eval_index(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut empty1 = false;
        let mut empty2 = false;
        let mut range = false;
        let mut key: *const c_char = ptr::null();
        let mut keylen: isize = -1;

        if check_can_index(rettv, evaluate, verbose) == FAIL {
            return FAIL;
        }

        let mut var1 = tv_initial_value();
        let mut var2 = tv_initial_value();
        if **arg == b'.' as c_char {
            key = (*arg).add(1);
            keylen = 0;
            while eval_isdictc(c_int::from(*key.add(keylen as usize) as u8)) {
                keylen += 1;
            }
            if keylen == 0 {
                return FAIL;
            }
            *arg = skipwhite(key.add(keylen as usize));
        } else {
            *arg = skipwhite((*arg).add(1));
            if **arg == b':' as c_char {
                empty1 = true;
            } else if eval1(arg, &raw mut var1, evalarg) == FAIL {
                return FAIL;
            } else if evaluate && !tv_check_str(&raw const var1) {
                tv_clear(&raw mut var1);
                return FAIL;
            }

            if **arg == b':' as c_char {
                range = true;
                *arg = skipwhite((*arg).add(1));
                if **arg == b']' as c_char {
                    empty2 = true;
                } else if eval1(arg, &raw mut var2, evalarg) == FAIL {
                    if !empty1 {
                        tv_clear(&raw mut var1);
                    }
                    return FAIL;
                } else if evaluate && !tv_check_str(&raw const var2) {
                    if !empty1 {
                        tv_clear(&raw mut var1);
                    }
                    tv_clear(&raw mut var2);
                    return FAIL;
                }
            }

            if **arg != b']' as c_char {
                if verbose {
                    emsg(gettext(E_MISSBRAC.as_ptr()));
                }
                tv_clear(&raw mut var1);
                if range {
                    tv_clear(&raw mut var2);
                }
                return FAIL;
            }
            *arg = skipwhite((*arg).add(1));
        }

        if evaluate {
            let res = eval_index_inner(
                rettv,
                range,
                if empty1 {
                    ptr::null_mut()
                } else {
                    &raw mut var1
                },
                if empty2 {
                    ptr::null_mut()
                } else {
                    &raw mut var2
                },
                false,
                key,
                keylen,
                verbose,
            );
            if !empty1 {
                tv_clear(&raw mut var1);
            }
            if range {
                tv_clear(&raw mut var2);
            }
            return res;
        }
        OK
    }
}

/// Check if "rettv" can have an [index] or [sli:ce]
unsafe fn check_can_index(rettv: *mut typval_T, evaluate: bool, verbose: bool) -> c_int {
    unsafe {
        match (*rettv).v_type {
            VarType::VAR_FUNC | VarType::VAR_PARTIAL => {
                if verbose {
                    emsg(gettext(E_CANNOT_INDEX_A_FUNCREF.as_ptr()));
                }
                return FAIL;
            }
            VarType::VAR_FLOAT => {
                if verbose {
                    emsg(gettext((&raw const e_using_float_as_string).cast()));
                }
                return FAIL;
            }
            VarType::VAR_BOOL | VarType::VAR_SPECIAL => {
                if verbose {
                    emsg(gettext(E_CANNOT_INDEX_SPECIAL_VARIABLE.as_ptr()));
                }
                return FAIL;
            }
            VarType::VAR_UNKNOWN => {
                if evaluate {
                    emsg(gettext(E_CANNOT_INDEX_SPECIAL_VARIABLE.as_ptr()));
                    return FAIL;
                }
            }
            VarType::VAR_STRING
            | VarType::VAR_NUMBER
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB => {}
        }
        OK
    }
}

/// slice() function
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_slice(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_can_index(argvars, true, false) != OK {
            return;
        }

        tv_copy(argvars, rettv);
        eval_index_inner(
            rettv,
            true,
            argvars.add(1),
            if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
                ptr::null_mut()
            } else {
                argvars.add(2)
            },
            true,
            ptr::null(),
            0,
            false,
        );
    }
}

/// Apply index or range to "rettv".
unsafe fn eval_index_inner(
    rettv: *mut typval_T,
    is_range: bool,
    var1: *mut typval_T,
    var2: *mut typval_T,
    exclusive: bool,
    mut key: *const c_char,
    keylen: isize,
    verbose: bool,
) -> c_int {
    unsafe {
        let mut n1: varnumber_T = 0;
        let mut n2: varnumber_T = 0;
        if !var1.is_null() && (*rettv).v_type != VarType::VAR_DICT {
            n1 = tv_get_number(var1);
        }

        if is_range {
            if (*rettv).v_type == VarType::VAR_DICT {
                if verbose {
                    emsg(gettext(E_CANNOT_SLICE_DICTIONARY.as_ptr()));
                }
                return FAIL;
            }
            if !var2.is_null() {
                n2 = tv_get_number(var2);
            } else {
                n2 = VARNUMBER_MAX;
            }
        }

        match (*rettv).v_type {
            VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_FUNC
            | VarType::VAR_FLOAT
            | VarType::VAR_PARTIAL
            | VarType::VAR_UNKNOWN => {}
            VarType::VAR_NUMBER | VarType::VAR_STRING => {
                let s = tv_get_string(rettv);
                let len = strlen(s) as varnumber_T;
                let v = if exclusive {
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
                        n2 = len;
                    }
                    if n1 >= len || n2 < 0 || n1 > n2 {
                        ptr::null_mut()
                    } else {
                        xmemdupz(s.add(n1 as usize).cast(), (n2 - n1 + 1) as usize).cast()
                    }
                } else if n1 >= len || n1 < 0 {
                    ptr::null_mut()
                } else {
                    xmemdupz(s.add(n1 as usize).cast(), 1).cast()
                };
                tv_clear(rettv);
                (*rettv).v_type = VarType::VAR_STRING;
                (*rettv).vval.v_string = v;
            }
            VarType::VAR_BLOB => {
                tv_blob_slice_or_index((*rettv).vval.v_blob, is_range, n1, n2, exclusive, rettv);
            }
            VarType::VAR_LIST => {
                if var1.is_null() {
                    n1 = 0;
                }
                if var2.is_null() {
                    n2 = VARNUMBER_MAX;
                }
                if tv_list_slice_or_index(
                    (*rettv).vval.v_list,
                    is_range,
                    n1,
                    n2,
                    exclusive,
                    rettv,
                    verbose,
                ) == FAIL
                {
                    return FAIL;
                }
            }
            VarType::VAR_DICT => {
                if key.is_null() {
                    key = tv_get_string_chk(var1);
                    if key.is_null() {
                        return FAIL;
                    }
                }

                let item = tv_dict_find((*rettv).vval.v_dict, key, keylen);

                if item.is_null() && verbose {
                    if keylen > 0 {
                        semsg!(gettext((&raw const e_dictkey_len).cast()), keylen, key);
                    } else {
                        semsg!(gettext((&raw const e_dictkey).cast()), key);
                    }
                }
                if item.is_null() || tv_is_luafunc(&raw mut (*item).di_tv) {
                    return FAIL;
                }

                let mut tmp = tv_initial_value();
                tv_copy(&raw const (*item).di_tv, &raw mut tmp);
                tv_clear(rettv);
                *rettv = tmp;
            }
        }
        OK
    }
}

/// Get an option value
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_option(
    arg: *mut *const c_char,
    rettv: *mut typval_T,
    evaluate: bool,
) -> c_int {
    unsafe {
        let working = **arg == b'+' as c_char;
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;

        let option_end = find_option_var_end(arg, &raw mut opt_idx, &raw mut opt_flags).cast_mut();

        if option_end.is_null() {
            if !rettv.is_null() {
                semsg!(gettext(c"E112: Option name missing: %s".as_ptr()), *arg);
            }
            return FAIL;
        }

        if !evaluate {
            *arg = option_end;
            return OK;
        }

        let c = *option_end;
        *option_end = NUL;

        let mut ret = OK;
        {
            let is_tty_opt = is_tty_option(*arg);

            if opt_idx == kOptInvalid && !is_tty_opt {
                if !rettv.is_null() {
                    semsg!(gettext(c"E113: Unknown option: %s".as_ptr()), *arg);
                }

                ret = FAIL;
            } else if !rettv.is_null() {
                let value = if is_tty_opt {
                    get_tty_option(*arg)
                } else {
                    get_option_value(opt_idx, opt_flags)
                };
                debug_assert!(value.obj_type != ObjectType::kObjectTypeNil);

                *rettv = opt_to_tv(value, true);
                optval_free(value);
            } else if working && !is_tty_opt && is_option_hidden(opt_idx) {
                ret = FAIL;
            }
        }

        *option_end = c;
        *arg = option_end;

        ret
    }
}

/// Allocate a variable for a number constant. Also deals with "0z" for blob.
unsafe fn eval_number(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evaluate: bool,
    want_string: bool,
) -> c_int {
    unsafe {
        let mut p = skipdigits((*arg).add(1));
        let mut get_float = false;

        if !want_string && *p == b'.' as c_char && ascii_isdigit(*p.add(1)) {
            get_float = true;
            p = skipdigits(p.add(2));
            if *p == b'e' as c_char || *p == b'E' as c_char {
                p = p.add(1);
                if *p == b'-' as c_char || *p == b'+' as c_char {
                    p = p.add(1);
                }
                if !ascii_isdigit(*p) {
                    get_float = false;
                } else {
                    p = skipdigits(p.add(1));
                }
            }
            if ascii_isalpha(*p) || *p == b'.' as c_char {
                get_float = false;
            }
        }
        if get_float {
            let mut f: float_T = 0.0;
            *arg = (*arg).add(string2float(*arg, &raw mut f));
            if evaluate {
                (*rettv).v_type = VarType::VAR_FLOAT;
                (*rettv).vval.v_float = f;
            }
        } else if **arg == b'0' as c_char
            && (*(*arg).add(1) == b'z' as c_char || *(*arg).add(1) == b'Z' as c_char)
        {
            let mut blob: *mut blob_T = ptr::null_mut();
            if evaluate {
                blob = tv_blob_alloc();
            }
            let mut bp = (*arg).add(2);
            while ascii_isxdigit(*bp) {
                if !ascii_isxdigit(*bp.add(1)) {
                    if !blob.is_null() {
                        emsg(gettext(
                            c"E973: Blob literal should have an even number of hex characters"
                                .as_ptr(),
                        ));
                        ga_clear(&raw mut (*blob).bv_ga);
                        xfree(blob.cast());
                    }
                    return FAIL;
                }
                if !blob.is_null() {
                    let byte = ((hex2nr(c_int::from(*bp as u8)) << 4)
                        + hex2nr(c_int::from(*bp.add(1) as u8)))
                        as u8;
                    ga_append(&raw mut (*blob).bv_ga, byte);
                }
                if *bp.add(2) == b'.' as c_char && ascii_isxdigit(*bp.add(3)) {
                    bp = bp.add(1);
                }
                bp = bp.add(2);
            }
            if !blob.is_null() {
                tv_blob_set_ret(rettv, blob);
            }
            *arg = bp;
        } else {
            let mut len: c_int = 0;
            let mut n: varnumber_T = 0;
            vim_str2nr(
                *arg,
                ptr::null_mut(),
                &raw mut len,
                STR2NR_ALL,
                &raw mut n,
                ptr::null_mut(),
                0,
                true,
                ptr::null_mut(),
            );
            if len == 0 {
                if evaluate {
                    semsg!(gettext((&raw const e_invexpr2).cast()), *arg);
                }
                return FAIL;
            }
            *arg = (*arg).add(len as usize);
            if evaluate {
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = n;
            }
        }
        OK
    }
}

/// Evaluate a string constant and put the result in "rettv".
unsafe fn eval_string(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evaluate: bool,
    interpolate: bool,
) -> c_int {
    unsafe {
        let arg_end = (*arg).add(strlen(*arg));
        let mut extra: isize = if interpolate { 1 } else { 0 };
        let off: usize = if interpolate { 0 } else { 1 };

        let mut p = (*arg).add(off);
        while *p != NUL && *p != b'"' as c_char {
            if *p == b'\\' as c_char && *p.add(1) != NUL {
                p = p.add(1);
                if *p == b'<' as c_char {
                    let mut modifiers: c_int = 0;
                    let mut flags = FSK_KEYCODE | FSK_IN_STRING;

                    extra += 5;

                    if *p.add(1) != b'*' as c_char {
                        flags |= FSK_SIMPLIFY;
                    }
                    let mut p_const: *const c_char = p;
                    if find_special_key(
                        &raw mut p_const,
                        (arg_end as usize).wrapping_sub(p as usize),
                        &raw mut modifiers,
                        flags,
                        ptr::null_mut(),
                    ) != 0
                    {
                        p = p_const.cast_mut().sub(1);
                    } else {
                        p = p_const.cast_mut();
                    }
                }
            } else if interpolate && (*p == b'{' as c_char || *p == b'}' as c_char) {
                if *p == b'{' as c_char && *p.add(1) != b'{' as c_char {
                    break;
                }
                p = p.add(1);
                if *p.sub(1) == b'}' as c_char && *p != b'}' as c_char {
                    semsg!(gettext((&raw const e_stray_closing_curly_str).cast()), *arg);
                    return FAIL;
                }
                extra -= 1;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        if *p != b'"' as c_char && !(interpolate && *p == b'{' as c_char) {
            semsg!(gettext(c"E114: Missing quote: %s".as_ptr()), *arg);
            return FAIL;
        }

        if !evaluate {
            *arg = p.add(off);
            return OK;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        let len = ((p as isize) - (*arg as isize) + extra) as usize;
        (*rettv).vval.v_string = xmalloc(len).cast::<c_char>();
        let mut end = (*rettv).vval.v_string;

        p = (*arg).add(off);
        while *p != NUL && *p != b'"' as c_char {
            if *p == b'\\' as c_char {
                p = p.add(1);
                match *p as u8 {
                    b'b' => {
                        *end = BS;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b'e' => {
                        *end = ESC;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b'f' => {
                        *end = FF;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b'n' => {
                        *end = NL;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b'r' => {
                        *end = CAR;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b't' => {
                        *end = TAB;
                        end = end.add(1);
                        p = p.add(1);
                    }
                    b'X' | b'x' | b'u' | b'U' => {
                        if ascii_isxdigit(*p.add(1)) {
                            let c = (*p as u8).to_ascii_uppercase();
                            let mut n = if c == b'X' {
                                2
                            } else if *p == b'u' as c_char {
                                4
                            } else {
                                8
                            };
                            let mut nr: c_int = 0;
                            while n > 0 && ascii_isxdigit(*p.add(1)) {
                                n -= 1;
                                p = p.add(1);
                                nr = (nr << 4) + hex2nr(c_int::from(*p as u8));
                            }
                            p = p.add(1);
                            if c != b'X' {
                                end = end.add(utf_char2bytes(nr, end) as usize);
                            } else {
                                *end = nr as c_char;
                                end = end.add(1);
                            }
                        }
                    }
                    b'0'..=b'7' => {
                        *end = (*p as u8 - b'0') as c_char;
                        p = p.add(1);
                        if *p >= b'0' as c_char && *p <= b'7' as c_char {
                            *end = (((*end as u8) << 3).wrapping_add(*p as u8 - b'0')) as c_char;
                            p = p.add(1);
                            if *p >= b'0' as c_char && *p <= b'7' as c_char {
                                *end =
                                    (((*end as u8) << 3).wrapping_add(*p as u8 - b'0')) as c_char;
                                p = p.add(1);
                            }
                        }
                        end = end.add(1);
                    }
                    b'<' => {
                        let mut flags = FSK_KEYCODE | FSK_IN_STRING;
                        if *p.add(1) != b'*' as c_char {
                            flags |= FSK_SIMPLIFY;
                        }
                        let mut p_const: *const c_char = p;
                        let spec_len = trans_special(
                            &raw mut p_const,
                            (arg_end as usize).wrapping_sub(p as usize),
                            end,
                            flags,
                            false,
                            ptr::null_mut(),
                        );
                        p = p_const.cast_mut();
                        if spec_len != 0 {
                            end = end.add(spec_len as usize);
                            if (end as usize) >= ((*rettv).vval.v_string as usize + len) {
                                iemsg(c"eval_string() used more space than allocated".as_ptr());
                            }
                        } else {
                            let mut p_c: *const c_char = p;
                            mb_copy_char(&raw mut p_c, &raw mut end);
                            p = p_c.cast_mut();
                        }
                    }
                    _ => {
                        let mut p_const: *const c_char = p;
                        mb_copy_char(&raw mut p_const, &raw mut end);
                        p = p_const.cast_mut();
                    }
                }
            } else {
                if interpolate && (*p == b'{' as c_char || *p == b'}' as c_char) {
                    if *p == b'{' as c_char && *p.add(1) != b'{' as c_char {
                        break;
                    }
                    p = p.add(1);
                }
                let mut p_const: *const c_char = p;
                mb_copy_char(&raw mut p_const, &raw mut end);
                p = p_const.cast_mut();
            }
        }
        *end = NUL;
        if *p == b'"' as c_char && !interpolate {
            p = p.add(1);
        }
        *arg = p;

        OK
    }
}

/// Allocate a variable for a 'str''ing' constant.
unsafe fn eval_lit_string(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evaluate: bool,
    interpolate: bool,
) -> c_int {
    unsafe {
        let mut reduce: isize = if interpolate { -1 } else { 0 };
        let off: usize = if interpolate { 0 } else { 1 };

        let mut p = (*arg).add(off);
        while *p != NUL {
            if *p == b'\'' as c_char {
                if *p.add(1) != b'\'' as c_char {
                    break;
                }
                reduce += 1;
                p = p.add(1);
            } else if interpolate {
                if *p == b'{' as c_char {
                    if *p.add(1) != b'{' as c_char {
                        break;
                    }
                    p = p.add(1);
                    reduce += 1;
                } else if *p == b'}' as c_char {
                    p = p.add(1);
                    if *p != b'}' as c_char {
                        semsg!(gettext((&raw const e_stray_closing_curly_str).cast()), *arg);
                        return FAIL;
                    }
                    reduce += 1;
                }
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        if *p != b'\'' as c_char && !(interpolate && *p == b'{' as c_char) {
            semsg!(gettext(c"E115: Missing quote: %s".as_ptr()), *arg);
            return FAIL;
        }

        if !evaluate {
            *arg = p.add(off);
            return OK;
        }

        let alloc_len = ((p as isize) - (*arg as isize) - reduce) as usize;
        let mut str_ptr = xmalloc(alloc_len).cast::<c_char>();
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = str_ptr;

        p = (*arg).add(off);
        while *p != NUL {
            if *p == b'\'' as c_char {
                if *p.add(1) != b'\'' as c_char {
                    break;
                }
                p = p.add(1);
            } else if interpolate && (*p == b'{' as c_char || *p == b'}' as c_char) {
                if *p == b'{' as c_char && *p.add(1) != b'{' as c_char {
                    break;
                }
                p = p.add(1);
            }
            let mut p_const: *const c_char = p;
            mb_copy_char(&raw mut p_const, &raw mut str_ptr);
            p = p_const.cast_mut();
        }
        *str_ptr = NUL;
        *arg = p.add(off);

        OK
    }
}

/// Evaluate a single or double quoted string possibly containing expressions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_interp_string(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evaluate: bool,
) -> c_int {
    unsafe {
        let mut ret: c_int;

        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut ga, 1, 80);

        *arg = (*arg).add(1);
        let quote = c_int::from(**arg as u8);
        *arg = (*arg).add(1);

        loop {
            let mut tv = tv_initial_value();
            if quote == b'"' as c_int {
                ret = eval_string(arg, &raw mut tv, evaluate, true);
            } else {
                ret = eval_lit_string(arg, &raw mut tv, evaluate, true);
            }
            if ret == FAIL {
                break;
            }
            if evaluate {
                ga_concat(&raw mut ga, tv.vval.v_string);
                tv_clear(&raw mut tv);
            }

            if **arg != b'{' as c_char {
                *arg = (*arg).add(1);
                break;
            }
            let p = eval_one_expr_in_str(*arg, &raw mut ga, evaluate);
            if p.is_null() {
                ret = FAIL;
                break;
            }
            *arg = p;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        if ret != FAIL && evaluate {
            ga_append(&raw mut ga, NUL as u8);
        }
        (*rettv).vval.v_string = ga.ga_data.cast::<c_char>();
        OK
    }
}

/// @return  the function name of the partial.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn partial_name(pt: *mut partial_T) -> *mut c_char {
    unsafe {
        if !pt.is_null() {
            if !(*pt).pt_name.is_null() {
                return (*pt).pt_name;
            }
            if !(*pt).pt_func.is_null() {
                let uf = (*pt).pt_func.cast::<ufunc_T>();
                return (&raw mut (*uf).uf_name).cast::<c_char>();
            }
        }
        c"".as_ptr().cast_mut()
    }
}

unsafe fn partial_free(pt: *mut partial_T) {
    unsafe {
        for i in 0..((*pt).pt_argc as usize) {
            tv_clear((*pt).pt_argv.add(i));
        }
        xfree((*pt).pt_argv.cast());
        tv_dict_unref((*pt).pt_dict);
        if !(*pt).pt_name.is_null() {
            func_unref((*pt).pt_name);
            xfree((*pt).pt_name.cast());
        } else {
            func_ptr_unref((*pt).pt_func.cast());
        }
        xfree(pt.cast());
    }
}

/// Unreference a closure: decrement the reference count and free it when it
/// becomes zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn partial_unref(pt: *mut partial_T) {
    unsafe {
        if pt.is_null() {
            return;
        }

        (*pt).pt_refcount -= 1;
        if (*pt).pt_refcount <= 0 {
            partial_free(pt);
        }
    }
}

/// Allocate a variable for a List and fill it from "*arg".
unsafe fn eval_list(arg: *mut *mut c_char, rettv: *mut typval_T, evalarg: *mut evalarg_T) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut l: *mut list_T = ptr::null_mut();

        if evaluate {
            l = tv_list_alloc(kListLenShouldKnow);
        }

        *arg = skipwhite((*arg).add(1));
        while **arg != b']' as c_char && **arg != NUL {
            let mut tv = tv_initial_value();
            if eval1(arg, &raw mut tv, evalarg) == FAIL {
                if evaluate {
                    tv_list_free(l);
                }
                return FAIL;
            }
            if evaluate {
                tv.v_lock = VAR_UNLOCKED;
                tv_list_append_owned_tv(l, tv);
            }

            let had_comma = **arg == b',' as c_char;
            if had_comma {
                *arg = skipwhite((*arg).add(1));
            }

            if **arg == b']' as c_char {
                break;
            }

            if !had_comma {
                semsg!(gettext(c"E696: Missing comma in List: %s".as_ptr()), *arg);
                if evaluate {
                    tv_list_free(l);
                }
                return FAIL;
            }
        }

        if **arg != b']' as c_char {
            semsg!(gettext(E_LIST_END.as_ptr()), *arg);
            if evaluate {
                tv_list_free(l);
            }
            return FAIL;
        }

        *arg = skipwhite((*arg).add(1));
        if evaluate {
            tv_list_set_ret(rettv, l);
        }

        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn func_equal(tv1: *mut typval_T, tv2: *mut typval_T, ic: bool) -> bool {
    unsafe {
        let mut s1 = if (*tv1).v_type == VarType::VAR_FUNC {
            (*tv1).vval.v_string
        } else {
            partial_name(tv_partial(tv1))
        };
        if !s1.is_null() && *s1 == NUL {
            s1 = ptr::null_mut();
        }
        let mut s2 = if (*tv2).v_type == VarType::VAR_FUNC {
            (*tv2).vval.v_string
        } else {
            partial_name(tv_partial(tv2))
        };
        if !s2.is_null() && *s2 == NUL {
            s2 = ptr::null_mut();
        }
        if s1.is_null() || s2.is_null() {
            if s1 != s2 {
                return false;
            }
        } else if strcmp(s1, s2) != 0 {
            return false;
        }

        let d1 = if (*tv1).v_type == VarType::VAR_FUNC {
            ptr::null_mut()
        } else {
            (*tv_partial(tv1)).pt_dict
        };
        let d2 = if (*tv2).v_type == VarType::VAR_FUNC {
            ptr::null_mut()
        } else {
            (*tv_partial(tv2)).pt_dict
        };
        if d1.is_null() || d2.is_null() {
            if d1 != d2 {
                return false;
            }
        } else if !tv_dict_equal(d1, d2, ic) {
            return false;
        }

        let a1 = if (*tv1).v_type == VarType::VAR_FUNC {
            0
        } else {
            (*tv_partial(tv1)).pt_argc
        };
        let a2 = if (*tv2).v_type == VarType::VAR_FUNC {
            0
        } else {
            (*tv_partial(tv2)).pt_argc
        };
        if a1 != a2 {
            return false;
        }
        for i in 0..(a1 as usize) {
            if !tv_equal(
                (*tv_partial(tv1)).pt_argv.add(i),
                (*tv_partial(tv2)).pt_argv.add(i),
                ic,
            ) {
                return false;
            }
        }
        true
    }
}

/// Get next (unique) copy ID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_copyID() -> c_int {
    unsafe {
        CURRENT_COPY_ID = CURRENT_COPY_ID.wrapping_add(COPYID_INC);
        CURRENT_COPY_ID
    }
}

/// Do garbage collection for lists and dicts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn garbage_collect(testing: bool) -> bool {
    unsafe {
        let mut abort = false;

        if !testing {
            want_garbage_collect = false;
            may_garbage_collect = false;
            garbage_collect_at_exit = false;
        }

        if exestack.ga_maxlen - exestack.ga_len > 500 {
            let mut n = exestack.ga_len / 2;
            if n < exestack.ga_growsize {
                n = exestack.ga_growsize;
            }
            if exestack.ga_len + n < exestack.ga_maxlen {
                let new_len = (exestack.ga_itemsize as usize) * ((exestack.ga_len + n) as usize);
                let pp = xrealloc(exestack.ga_data, new_len);
                exestack.ga_maxlen = exestack.ga_len + n;
                exestack.ga_data = pp;
            }
        }

        let copyID = get_copyID();

        abort = abort || set_ref_in_previous_funccal(copyID);
        abort = abort || garbage_collect_scriptvars(copyID);

        {
            let mut buf = firstbuf;
            while !buf.is_null() {
                abort = abort
                    || set_ref_in_item(
                        &raw mut (*buf).b_bufvar.di_tv,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_prompt_callback,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_prompt_interrupt,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_p_cfu,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_p_ofu,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_p_tsrfu,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_p_tfu,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                abort = abort
                    || set_ref_in_callback(
                        &raw mut (*buf).b_p_ffu,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                if !abort && !(*buf).b_p_cpt_cb.is_null() {
                    abort = abort
                        || set_ref_in_cpt_callbacks(
                            (*buf).b_p_cpt_cb.cast(),
                            (*buf).b_p_cpt_count,
                            copyID,
                        );
                }
                buf = (*buf).b_next;
            }
        }

        abort = abort || set_ref_in_insexpand_funcs(copyID);
        abort = abort || set_ref_in_opfunc(copyID);
        abort = abort || set_ref_in_tagfunc(copyID);
        {
            abort = abort || set_ref_in_findfunc(copyID);
        }

        {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    abort = abort
                        || set_ref_in_item(
                            &raw mut (*wp).w_winvar.di_tv,
                            copyID,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        );
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }

            for i in 0..ctx_win_vec.size {
                let cw_win = (*ctx_win_vec.items.add(i)).cw_win;
                if !cw_win.is_null() {
                    abort = abort
                        || set_ref_in_item(
                            &raw mut (*cw_win).w_winvar.di_tv,
                            copyID,
                            ptr::null_mut(),
                            ptr::null_mut(),
                        );
                }
            }
        }

        {
            let mut reg_iter: *const c_void = ptr::null();
            loop {
                let mut reg: yankreg_T = std::mem::zeroed();
                let mut name: c_char = NUL;
                let mut is_unnamed = false;
                reg_iter =
                    op_global_reg_iter(reg_iter, &raw mut name, &raw mut reg, &raw mut is_unnamed);
                if reg_iter.is_null() {
                    break;
                }
            }
        }

        {
            let mut mark_iter: *const c_void = ptr::null();
            loop {
                let mut fm: xfmark_T = std::mem::zeroed();
                let mut name: c_char = NUL;
                mark_iter = mark_global_iter(mark_iter, &raw mut name, &raw mut fm);
                if mark_iter.is_null() {
                    break;
                }
            }
        }

        {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                abort = abort
                    || set_ref_in_item(
                        &raw mut (*tp).tp_winvar.di_tv,
                        copyID,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                tp = (*tp).tp_next;
            }
        }

        abort = abort || garbage_collect_globvars(copyID) != 0;
        abort = abort || set_ref_in_call_stack(copyID);
        abort = abort || set_ref_in_functions(copyID);

        {
            for i in 0..(channels.set.h.n_keys as usize) {
                let data = (*channels.values.add(i)).cast::<Channel>();
                let ct = channel_tail(data);
                set_ref_in_callback_reader(
                    &raw mut (*ct).on_data,
                    copyID,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                set_ref_in_callback_reader(
                    &raw mut (*ct).on_stderr,
                    copyID,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                set_ref_in_callback(
                    &raw mut (*ct).on_exit,
                    copyID,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            }
        }

        for i in 0..(timers.set.h.n_keys as usize) {
            let timer = (*timers.values.add(i)).cast::<timer_T>();
            let tt = timer_tail(timer);
            set_ref_in_callback(
                &raw mut (*tt).callback,
                copyID,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }

        abort = abort || set_ref_in_func_args(copyID);
        abort = abort || garbage_collect_vimvars(copyID);
        {
            abort = abort || set_ref_in_quickfix(copyID);
        }

        let mut did_free = false;
        if !abort {
            did_free = free_unref_items(copyID) != 0;
            did_free = free_unref_funccal(copyID, c_int::from(testing)) || did_free;
        } else {
            if p_verbose > 0 {
                verb_msg(gettext(
                    c"Not enough memory to set references, garbage collection aborted!".as_ptr(),
                ));
            }
        }
        did_free
    }
}

/// Free lists and dictionaries that are no longer referenced.
unsafe fn free_unref_items(copyID: c_int) -> c_int {
    unsafe {
        let mut did_free = false;

        tv_in_free_unref_items = true;

        let mut dd = gc_first_dict.cast::<dict_T>();
        while !dd.is_null() {
            if ((*dd).dv_copyID & COPYID_MASK) != (copyID & COPYID_MASK) {
                tv_dict_free_contents(dd);
                did_free = true;
            }
            dd = (*dd).dv_used_next;
        }

        let mut ll = gc_first_list.cast::<list_T>();
        while !ll.is_null() {
            if (tv_list_copyid(ll) & COPYID_MASK) != (copyID & COPYID_MASK)
                && !tv_list_has_watchers(ll)
            {
                tv_list_free_contents(ll);
                did_free = true;
            }
            ll = (*ll).lv_used_next;
        }

        let mut dd_cur = gc_first_dict.cast::<dict_T>();
        while !dd_cur.is_null() {
            let dd_next = (*dd_cur).dv_used_next;
            if ((*dd_cur).dv_copyID & COPYID_MASK) != (copyID & COPYID_MASK) {
                tv_dict_free_dict(dd_cur);
            }
            dd_cur = dd_next;
        }

        let mut ll_cur = gc_first_list.cast::<list_T>();
        while !ll_cur.is_null() {
            let ll_next = (*ll_cur).lv_used_next;
            if ((*ll_cur).lv_copyID & COPYID_MASK) != (copyID & COPYID_MASK)
                && !tv_list_has_watchers(ll_cur)
            {
                tv_list_free_list(ll_cur);
            }
            ll_cur = ll_next;
        }

        tv_in_free_unref_items = false;
        c_int::from(did_free)
    }
}

/// Mark all lists and dicts referenced through hashtab "ht" with "copyID".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_ref_in_ht(
    ht: *mut hashtab_T,
    copyID: c_int,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        let mut abort = false;
        let mut ht_stack: *mut ht_stack_T = ptr::null_mut();

        let mut cur_ht = ht;
        loop {
            if !abort {
                let mut todo = (*cur_ht).ht_used;
                let mut hi = (*cur_ht).ht_array;
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        todo -= 1;
                        let di = tv_dict_hi2di(hi);
                        abort = abort
                            || set_ref_in_item(
                                &raw mut (*di).di_tv,
                                copyID,
                                &raw mut ht_stack,
                                list_stack,
                            );
                    }
                    hi = hi.add(1);
                }
            }

            if ht_stack.is_null() {
                break;
            }

            cur_ht = (*ht_stack).ht;
            let tempitem = ht_stack;
            ht_stack = (*ht_stack).prev;
            xfree(tempitem.cast());
        }

        abort
    }
}

/// Mark all lists and dicts referenced through list "l" with "copyID".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_ref_in_list_items(
    l: *mut list_T,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
) -> bool {
    unsafe {
        let mut abort = false;
        let mut list_stack: *mut list_stack_T = ptr::null_mut();

        let mut cur_l = l;
        loop {
            if !cur_l.is_null() {
                let mut li = (*cur_l).lv_first;
                while !li.is_null() {
                    if abort {
                        break;
                    }
                    abort =
                        set_ref_in_item(tv_list_item_tv(li), copyID, ht_stack, &raw mut list_stack);
                    li = (*li).li_next;
                }
            }

            if list_stack.is_null() {
                break;
            }

            cur_l = (*list_stack).list;
            let tempitem = list_stack;
            list_stack = (*list_stack).prev;
            xfree(tempitem.cast());
        }

        abort
    }
}

unsafe fn set_ref_in_item_dict(
    dd: *mut dict_T,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        if dd.is_null() || (*dd).dv_copyID == copyID {
            return false;
        }

        (*dd).dv_copyID = copyID;
        if ht_stack.is_null() {
            return set_ref_in_ht(&raw mut (*dd).dv_hashtab, copyID, list_stack);
        }

        let newitem = xmalloc(size_of::<ht_stack_T>()).cast::<ht_stack_T>();
        (*newitem).ht = &raw mut (*dd).dv_hashtab;
        (*newitem).prev = *ht_stack;
        *ht_stack = newitem;

        let head = dict_watchers_queue(dd);
        let mut w = (*head).next;
        while w != head {
            let watcher = tv_dict_watcher_node_data(w);
            set_ref_in_callback(&raw mut (*watcher).callback, copyID, ht_stack, list_stack);
            w = (*w).next;
        }

        false
    }
}

unsafe fn set_ref_in_item_list(
    ll: *mut list_T,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        if ll.is_null() || (*ll).lv_copyID == copyID {
            return false;
        }

        (*ll).lv_copyID = copyID;
        if list_stack.is_null() {
            return set_ref_in_list_items(ll, copyID, ht_stack);
        }

        let newitem = xmalloc(size_of::<list_stack_T>()).cast::<list_stack_T>();
        (*newitem).list = ll;
        (*newitem).prev = *list_stack;
        *list_stack = newitem;

        false
    }
}

unsafe fn set_ref_in_item_partial(
    pt: *mut partial_T,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        if pt.is_null() || (*pt).pt_copyID == copyID {
            return false;
        }

        (*pt).pt_copyID = copyID;

        let mut abort = set_ref_in_func((*pt).pt_name, (*pt).pt_func.cast(), copyID);

        if !(*pt).pt_dict.is_null() {
            let mut dtv = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_dict: (*pt).pt_dict,
                },
            };
            abort = abort || set_ref_in_item(&raw mut dtv, copyID, ht_stack, list_stack);
        }

        for i in 0..((*pt).pt_argc as usize) {
            abort = abort || set_ref_in_item((*pt).pt_argv.add(i), copyID, ht_stack, list_stack);
        }

        abort
    }
}

/// Mark all lists and dicts referenced through typval "tv" with "copyID".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_ref_in_item(
    tv: *mut typval_T,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_DICT => {
                set_ref_in_item_dict((*tv).vval.v_dict, copyID, ht_stack, list_stack)
            }
            VarType::VAR_LIST => {
                set_ref_in_item_list((*tv).vval.v_list, copyID, ht_stack, list_stack)
            }
            VarType::VAR_FUNC => set_ref_in_func((*tv).vval.v_string, ptr::null_mut(), copyID),
            VarType::VAR_PARTIAL => {
                set_ref_in_item_partial(tv_partial(tv), copyID, ht_stack, list_stack)
            }
            VarType::VAR_UNKNOWN
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_FLOAT
            | VarType::VAR_NUMBER
            | VarType::VAR_STRING
            | VarType::VAR_BLOB => false,
        }
    }
}

/// Get the key for #{key: val} into "tv" and advance "arg".
unsafe fn get_literal_key(arg: *mut *mut c_char, tv: *mut typval_T) -> c_int {
    unsafe {
        if !ascii_isalnum(**arg) && **arg != b'_' as c_char && **arg != b'-' as c_char {
            return FAIL;
        }
        let mut p = *arg;
        while ascii_isalnum(*p) || *p == b'_' as c_char || *p == b'-' as c_char {
            p = p.add(1);
        }
        (*tv).v_type = VarType::VAR_STRING;
        (*tv).vval.v_string =
            xmemdupz((*arg).cast(), (p as usize).wrapping_sub(*arg as usize)).cast();

        *arg = skipwhite(p);
        OK
    }
}

/// Allocate a variable for a Dictionary and fill it from "*arg".
unsafe fn eval_dict(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    literal: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut tv = tv_initial_value();
        let mut key: *mut c_char = ptr::null_mut();
        let mut curly_expr = skipwhite((*arg).add(1));
        let mut buf = [0 as c_char; NUMBUFLEN];

        if *curly_expr != b'}' as c_char
            && !literal
            && eval1(&raw mut curly_expr, &raw mut tv, ptr::null_mut()) == OK
            && *skipwhite(curly_expr) == b'}' as c_char
        {
            return NOTDONE;
        }

        let mut d: *mut dict_T = ptr::null_mut();
        if evaluate {
            d = tv_dict_alloc();
        }
        let mut tvkey = tv_initial_value();
        tv.v_type = VarType::VAR_UNKNOWN;

        *arg = skipwhite((*arg).add(1));
        while **arg != b'}' as c_char && **arg != NUL {
            let key_res = if literal {
                get_literal_key(arg, &raw mut tvkey)
            } else {
                eval1(arg, &raw mut tvkey, evalarg)
            };
            if key_res == FAIL {
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if **arg != b':' as c_char {
                semsg!(
                    gettext(c"E720: Missing colon in Dictionary: %s".as_ptr()),
                    *arg,
                );
                tv_clear(&raw mut tvkey);
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if evaluate {
                key = tv_get_string_buf_chk(&raw const tvkey, buf.as_mut_ptr()).cast_mut();
                if key.is_null() {
                    tv_clear(&raw mut tvkey);
                    if !d.is_null() {
                        tv_dict_free(d);
                    }
                    return FAIL;
                }
            }

            *arg = skipwhite((*arg).add(1));
            if eval1(arg, &raw mut tv, evalarg) == FAIL {
                tv_clear(&raw mut tvkey);
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
            if evaluate {
                let mut item = tv_dict_find(d, key, -1);
                if !item.is_null() {
                    semsg!(
                        gettext(c"E721: Duplicate key in Dictionary: \"%s\"".as_ptr()),
                        key,
                    );
                    tv_clear(&raw mut tvkey);
                    tv_clear(&raw mut tv);
                    if !d.is_null() {
                        tv_dict_free(d);
                    }
                    return FAIL;
                }
                item = tv_dict_item_alloc(key);
                (*item).di_tv = tv;
                (*item).di_tv.v_lock = VAR_UNLOCKED;
                if tv_dict_add(d, item) == FAIL {
                    tv_dict_item_free(item);
                }
            }
            tv_clear(&raw mut tvkey);

            let had_comma = **arg == b',' as c_char;
            if had_comma {
                *arg = skipwhite((*arg).add(1));
            }

            if **arg == b'}' as c_char {
                break;
            }
            if !had_comma {
                semsg!(
                    gettext(c"E722: Missing comma in Dictionary: %s".as_ptr()),
                    *arg,
                );
                if !d.is_null() {
                    tv_dict_free(d);
                }
                return FAIL;
            }
        }

        if **arg != b'}' as c_char {
            semsg!(
                gettext(c"E723: Missing end of Dictionary '}': %s".as_ptr()),
                *arg,
            );
            if !d.is_null() {
                tv_dict_free(d);
            }
            return FAIL;
        }

        *arg = skipwhite((*arg).add(1));
        if evaluate {
            tv_dict_set_ret(rettv, d);
        }

        OK
    }
}

/// Evaluate a literal dictionary: #{key: val, key: val}
unsafe fn eval_lit_dict(
    arg: *mut *mut c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
) -> c_int {
    unsafe {
        if *(*arg).add(1) == b'{' as c_char {
            *arg = (*arg).add(1);
            eval_dict(arg, rettv, evalarg, true)
        } else {
            NOTDONE
        }
    }
}

/// Convert the string to a floating point number.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn string2float(text: *const c_char, ret_value: *mut float_T) -> usize {
    unsafe {
        let b0 = (*text as u8).to_ascii_lowercase();
        let b1 = if b0 != 0 {
            (*text.add(1) as u8).to_ascii_lowercase()
        } else {
            0
        };
        let b2 = if b1 != 0 {
            (*text.add(2) as u8).to_ascii_lowercase()
        } else {
            0
        };
        if b0 == b'i' && b1 == b'n' && b2 == b'f' {
            *ret_value = f64::INFINITY;
            return 3;
        }
        if b0 == b'-' && b1 == b'i' && b2 == b'n' {
            let b3 = (*text.add(3) as u8).to_ascii_lowercase();
            if b3 == b'f' {
                *ret_value = f64::NEG_INFINITY;
                return 4;
            }
        }
        if b0 == b'n' && b1 == b'a' && b2 == b'n' {
            *ret_value = f64::NAN;
            return 3;
        }
        let mut s: *mut c_char = ptr::null_mut();
        *ret_value = strtod(text, &raw mut s);
        (s as usize).wrapping_sub(text as usize)
    }
}

/// Get the value of an environment variable.
unsafe fn eval_env_var(arg: *mut *mut c_char, rettv: *mut typval_T, evaluate: c_int) -> c_int {
    unsafe {
        *arg = (*arg).add(1);
        let name = *arg;
        let len = get_env_len(arg.cast());

        if evaluate != 0 {
            if len == 0 {
                return FAIL;
            }
            let cc = *name.add(len as usize);
            *name.add(len as usize) = NUL;
            let mut string = vim_getenv(name);
            if string.is_null() || *string == NUL {
                xfree(string.cast());
                string = expand_env_save(name.sub(1));
                if !string.is_null() && *string == b'$' as c_char {
                    xfree(string.cast());
                    string = ptr::null_mut();
                }
            }
            *name.add(len as usize) = cc;
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string = string;
            (*rettv).v_lock = VAR_UNLOCKED;
        }

        OK
    }
}

/// Builds a process argument vector from a Vimscript object (typval_T).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tv_to_argv(
    cmd_tv: *mut typval_T,
    cmd: *mut *const c_char,
    executable: *mut bool,
) -> *mut *mut c_char {
    unsafe {
        if (*cmd_tv).v_type == VarType::VAR_STRING {
            let cmd_str = tv_get_string(cmd_tv);
            if !cmd.is_null() {
                *cmd = cmd_str;
            }
            return shell_build_argv(cmd_str, ptr::null());
        }

        if (*cmd_tv).v_type != VarType::VAR_LIST {
            semsg!(
                gettext((&raw const e_invarg2).cast()),
                c"expected String or List".as_ptr(),
            );
            return ptr::null_mut();
        }

        let argl = (*cmd_tv).vval.v_list;
        let argc = tv_list_len(argl);
        if argc == 0 {
            emsg(gettext((&raw const e_invarg).cast()));
            return ptr::null_mut();
        }

        let arg0 = tv_get_string_chk(tv_list_item_tv(tv_list_first(argl)));
        let mut exe_resolved: *mut c_char = ptr::null_mut();
        if arg0.is_null() || !os_can_exe(arg0, &raw mut exe_resolved, true) {
            if !arg0.is_null() && !executable.is_null() {
                let mut buf = [0 as c_char; IOSIZE];
                vim_snprintf!(
                    buf.as_mut_ptr(),
                    buf.len(),
                    c"'%s' is not executable".as_ptr(),
                    arg0,
                );
                semsg!(
                    gettext((&raw const e_invargNval).cast()),
                    c"cmd".as_ptr(),
                    buf.as_ptr(),
                );
                *executable = false;
            }
            return ptr::null_mut();
        }

        if !cmd.is_null() {
            *cmd = exe_resolved;
        }

        let mut i: usize = 0;
        let argv = xcalloc((argc as usize) + 1, size_of::<*mut c_char>()).cast::<*mut c_char>();
        let mut arg = if argl.is_null() {
            ptr::null_mut()
        } else {
            (*argl).lv_first
        };
        while !arg.is_null() {
            let a = tv_get_string_chk(tv_list_item_tv(arg));
            if a.is_null() {
                shell_free_argv(argv);
                xfree(exe_resolved.cast());
                return ptr::null_mut();
            }
            *argv.add(i) = xstrdup(a);
            i += 1;
            arg = (*arg).li_next;
        }
        xfree((*argv).cast());
        *argv = exe_resolved;

        argv
    }
}

unsafe fn string_to_list(str_: *const c_char, mut len: usize, keepempty: bool) -> *mut list_T {
    unsafe {
        if !keepempty && len > 0 && *str_.add(len - 1) == NL {
            len -= 1;
        }
        let list = tv_list_alloc(kListLenMayKnow);
        encode_list_write(list.cast(), str_, len);
        list
    }
}

unsafe fn get_system_output_as_rettv(argvars: *mut typval_T, rettv: *mut typval_T, retlist: bool) {
    unsafe {
        let mut wait_time: proftime_T = 0;
        let profiling = do_profiling == PROF_YES;

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if check_secure() {
            return;
        }

        let mut input_len: isize = 0;
        let input = save_tv_as_string(argvars.add(1), &raw mut input_len, false, false);
        if input_len < 0 {
            debug_assert!(input.is_null());
            return;
        }

        let mut executable = true;
        let argv = tv_to_argv(argvars, ptr::null_mut(), &raw mut executable);
        if argv.is_null() {
            if !executable {
                set_vim_var_nr(VV_SHELL_ERROR, -1);
            }
            xfree(input.cast());
            return;
        }

        if p_verbose > 3 {
            let cmdstr = shell_argv_to_str(argv);
            verbose_enter_scroll();
            smsg!(0, gettext(c"Executing command: \"%s\"".as_ptr()), cmdstr);
            msg_puts(c"\n\n".as_ptr());
            verbose_leave_scroll();
            xfree(cmdstr.cast());
        }

        if profiling {
            prof_child_enter(&raw mut wait_time);
        }

        let mut nread: usize = 0;
        let mut res: *mut c_char = ptr::null_mut();
        let status = os_system(
            argv,
            input,
            input_len as usize,
            &raw mut res,
            &raw mut nread,
        );

        if profiling {
            prof_child_exit(&raw mut wait_time);
        }

        xfree(input.cast());

        set_vim_var_nr(VV_SHELL_ERROR, varnumber_T::from(status));

        if res.is_null() {
            if retlist {
                tv_list_alloc_ret(rettv, 0);
            } else {
                (*rettv).vval.v_string = xstrdup(c"".as_ptr());
            }
            return;
        }

        if retlist {
            let mut keepempty: c_int = 0;
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            {
                keepempty = tv_get_number(argvars.add(2)) as c_int;
            }
            (*rettv).vval.v_list = string_to_list(res, nread, keepempty != 0);
            tv_list_ref((*rettv).vval.v_list);
            (*rettv).v_type = VarType::VAR_LIST;

            xfree(res.cast());
        } else {
            memchrsub(res.cast(), NUL, 1, nread);
            #[cfg(windows)]
            {
                let mut d = res;
                let mut s = res;
                while *s != NUL {
                    if *s == CAR && *s.add(1) == NL {
                        s = s.add(1);
                    }
                    *d = *s;
                    d = d.add(1);
                    s = s.add(1);
                }
                *d = NUL;
            }
            (*rettv).vval.v_string = res;
        }
    }
}

/// f_system - the Vimscript system() function
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_system(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_system_output_as_rettv(argvars, rettv, false);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_systemlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_system_output_as_rettv(argvars, rettv, true);
    }
}

/// Get a callback from "arg". It can be a Funcref or a function name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn callback_from_typval(
    callback: *mut Callback,
    arg: *const typval_T,
) -> bool {
    unsafe {
        let mut r = OK;

        let is_lua_tbl = nlua_is_table_from_lua(arg);

        if (*arg).v_type == VarType::VAR_PARTIAL && !tv_partial(arg).is_null() {
            let pt = tv_partial(arg);
            (*callback).data = pt.cast();
            (*pt).pt_refcount += 1;
            (*callback).cb_type = kCallbackPartial;
        } else if (*arg).v_type == VarType::VAR_STRING
            && !(*arg).vval.v_string.is_null()
            && ascii_isdigit(*(*arg).vval.v_string)
        {
            r = FAIL;
        } else if (*arg).v_type == VarType::VAR_FUNC || (*arg).v_type == VarType::VAR_STRING {
            let name = (*arg).vval.v_string;
            if name.is_null() {
                r = FAIL;
            } else if *name == NUL {
                (*callback).cb_type = kCallbackNone;
                (*callback).data = ptr::null_mut();
            } else {
                (*callback).data = ptr::null_mut();
                if (*arg).v_type == VarType::VAR_STRING {
                    (*callback).data = get_scriptlocal_funcname(name).cast();
                }
                if (*callback).data.is_null() {
                    (*callback).data = xstrdup(name).cast();
                }
                func_ref((*callback).data.cast());
                (*callback).cb_type = kCallbackFuncref;
            }
        } else if is_lua_tbl {
            {
                let name = nlua_register_table_as_callable(arg);
                if !name.is_null() {
                    (*callback).data = xstrdup(name).cast();
                    (*callback).cb_type = kCallbackFuncref;
                } else {
                    r = FAIL;
                }
            }
        } else if (*arg).v_type == VarType::VAR_SPECIAL
            || ((*arg).v_type == VarType::VAR_NUMBER && (*arg).vval.v_number == 0)
        {
            (*callback).cb_type = kCallbackNone;
            (*callback).data = ptr::null_mut();
        } else {
            r = FAIL;
        }

        if r == FAIL {
            emsg(gettext(c"E921: Invalid callback argument".as_ptr()));
            return false;
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_callback_depth() -> c_int {
    unsafe { callback_depth }
}

/// @return  whether the callback could be called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn callback_call(
    callback: *mut Callback,
    argcount_in: c_int,
    argvars_in: *mut typval_T,
    rettv: *mut typval_T,
) -> bool {
    unsafe {
        if i64::from(callback_depth) > p_mfd {
            emsg(gettext((&raw const e_command_too_recursive).cast()));
            return false;
        }

        let partial: *mut partial_T;
        let mut name: *mut c_char;
        match (*callback).cb_type {
            kCallbackFuncref => {
                name = (*callback).data.cast::<c_char>();
                let len = strlen(name) as c_int;
                if len >= 6 && std::slice::from_raw_parts(name.cast::<u8>(), 6) == b"v:lua." {
                    name = name.add(6);
                    if check_luafunc_name(name, false) == 0 {
                        return false;
                    }
                    partial = get_vim_var_partial(VV_LUA);
                } else {
                    partial = ptr::null_mut();
                }
            }
            kCallbackPartial => {
                partial = (*callback).data.cast::<partial_T>();
                name = partial_name(partial);
            }
            kCallbackLua => {
                let mut arena = Arena {
                    cur_blk: ptr::null_mut(),
                    pos: 0,
                    size: 0,
                };
                let mut luaargs = arena_array(&raw mut arena, argcount_in as usize);
                for i in 0..(argcount_in as usize) {
                    *luaargs.items.add(luaargs.size) =
                        vim_to_object(argvars_in.add(i), &raw mut arena, false);
                    luaargs.size += 1;
                }
                let mut err = Error {
                    err_type: ErrorType::kErrorTypeNone,
                    msg: ptr::null_mut(),
                };
                let luaref = *(&raw const (*callback).data).cast::<c_int>();
                callback_depth += 1;
                let result = nlua_call_ref(
                    luaref,
                    ptr::null(),
                    luaargs,
                    kRetObject,
                    &raw mut arena,
                    &raw mut err,
                );
                callback_depth -= 1;
                let ok = err.err_type == ErrorType::kErrorTypeNone;
                if !ok {
                    semsg_multiline!(c"emsg".as_ptr(), c"E5108: %s".as_ptr(), err.msg);
                    api_clear_error(&raw mut err);
                } else {
                    object_to_vim(result, rettv, ptr::null_mut());
                }
                arena_mem_free(arena_finish(&raw mut arena));
                return ok;
            }
            kCallbackExpr => {
                callback_depth += 1;
                let evalarg_ptr = &raw mut EVALARG_EVALUATE;
                let r = eval0_simple_funccal(
                    skipwhite((*callback).data.cast::<c_char>()),
                    rettv,
                    ptr::null_mut(),
                    evalarg_ptr,
                );
                callback_depth -= 1;
                clear_evalarg(evalarg_ptr, ptr::null_mut());
                return r == OK;
            }
            _ => {
                return false;
            }
        }

        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: true,
            fe_partial: partial.cast(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };

        callback_depth += 1;
        let ret = call_func(name, -1, rettv, argcount_in, argvars_in, &raw mut funcexe);
        callback_depth -= 1;
        ret != 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_ref_in_callback(
    callback: *mut Callback,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        if (*callback).cb_type == kCallbackPartial {
            let mut tv = typval_T {
                v_type: VarType::VAR_PARTIAL,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_ptr: (*callback).data,
                },
            };
            return set_ref_in_item(&raw mut tv, copyID, ht_stack, list_stack);
        }
        false
    }
}

unsafe fn set_ref_in_callback_reader(
    reader: *mut CallbackReader,
    copyID: c_int,
    ht_stack: *mut *mut ht_stack_T,
    list_stack: *mut *mut list_stack_T,
) -> bool {
    unsafe {
        if set_ref_in_callback(&raw mut (*reader).cb, copyID, ht_stack, list_stack) {
            return true;
        }

        if !(*reader).self_dict.is_null() {
            let mut tv = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_dict: (*reader).self_dict,
                },
            };
            return set_ref_in_item(&raw mut tv, copyID, ht_stack, list_stack);
        }
        false
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn find_timer_by_nr(xx: varnumber_T) -> *mut timer_T {
    unsafe {
        let ptr = map_ref_uint64_t_ptr_t(&raw mut timers, xx as u64, ptr::null_mut());
        if ptr.is_null() {
            ptr::null_mut()
        } else {
            (*ptr).cast()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_timer_info(rettv: *mut typval_T, timer: *mut timer_T) {
    unsafe {
        let list = (*rettv).vval.v_list;
        let dict = tv_dict_alloc();
        let tt = timer_tail(timer);

        tv_list_append_dict(list, dict);
        tv_dict_add_nr(dict, c"id".as_ptr(), 2, varnumber_T::from((*tt).timer_id));
        tv_dict_add_nr(dict, c"time".as_ptr(), 4, (*tt).timeout);
        tv_dict_add_nr(dict, c"paused".as_ptr(), 6, varnumber_T::from((*tt).paused));
        tv_dict_add_nr(
            dict,
            c"repeat".as_ptr(),
            6,
            if (*tt).repeat_count < 0 {
                -1
            } else {
                varnumber_T::from((*tt).repeat_count)
            },
        );

        let di = tv_dict_item_alloc(c"callback".as_ptr());
        if tv_dict_add(dict, di) == FAIL {
            xfree(di.cast());
        } else {
            callback_put(&raw mut (*tt).callback, &raw mut (*di).di_tv);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_timer_info_all(rettv: *mut typval_T) {
    unsafe {
        tv_list_alloc_ret(rettv, timers.set.h.size as isize);
        for i in 0..(timers.set.h.n_keys as usize) {
            let timer = (*timers.values.add(i)).cast::<timer_T>();
            let tt = timer_tail(timer);
            if !(*tt).stopped || (*tt).refcount > 1 {
                add_timer_info(rettv, timer);
            }
        }
    }
}

/// Invoked on the main loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_due_cb(_tw: *mut TimeWatcher, data: *mut c_void) {
    unsafe {
        let timer = data.cast::<timer_T>();
        let tt = timer_tail(timer);
        let save_did_emsg = did_emsg;
        let called_emsg_before = called_emsg;

        if (*tt).stopped || (*tt).paused {
            return;
        }

        (*tt).refcount += 1;
        if (*tt).repeat_count >= 0 {
            (*tt).repeat_count -= 1;
            if (*tt).repeat_count == 0 {
                timer_stop(timer);
            }
        }

        let mut argv = [tv_initial_value(), tv_initial_value()];
        argv[0].v_type = VarType::VAR_NUMBER;
        argv[0].vval.v_number = varnumber_T::from((*tt).timer_id);
        let mut rettv = tv_initial_value();

        callback_call(
            &raw mut (*tt).callback,
            1,
            argv.as_mut_ptr(),
            &raw mut rettv,
        );

        {
            if called_emsg > called_emsg_before && did_emsg != 0 {
                (*tt).emsg_count += 1;
                if did_throw {
                    discard_current_exception();
                }
            }
            did_emsg = save_did_emsg;
        }

        if (*tt).emsg_count >= 3 {
            timer_stop(timer);
        }

        tv_clear(&raw mut rettv);

        if !(*tt).stopped && (*tt).timeout == 0 {
            time_watcher_start(timer.cast(), Some(timer_due_cb), 0, 0);
        }
        timer_decref(timer);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_start(
    timeout: i64,
    repeat_count: c_int,
    callback: *const Callback,
) -> u64 {
    unsafe {
        let timer = xmalloc(timer_size()).cast::<timer_T>();
        let tt = timer_tail(timer);
        (*tt).refcount = 1;
        (*tt).stopped = false;
        (*tt).paused = false;
        (*tt).emsg_count = 0;
        (*tt).repeat_count = repeat_count;
        (*tt).timeout = timeout;
        let id = last_timer_id;
        last_timer_id += 1;
        (*tt).timer_id = id as c_int;
        (*tt).callback = *callback;

        {
            let main_loop_ptr = (&raw mut main_loop).cast();
            time_watcher_init(main_loop_ptr, timer.cast(), timer);
            let wf = watcher_fields(timer.cast());
            (*wf).events = multiqueue_new_child((*loop_queues(main_loop_ptr)).events);
            (*wf).blockable = true;
            time_watcher_start(
                timer.cast(),
                Some(timer_due_cb),
                timeout as u64,
                timeout as u64,
            );
        }

        let slot = map_put_ref_uint64_t_ptr_t(
            &raw mut timers,
            (*tt).timer_id as u64,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        *slot = timer.cast();
        (*tt).timer_id as u64
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_stop(timer: *mut timer_T) {
    unsafe {
        let tt = timer_tail(timer);
        if (*tt).stopped {
            return;
        }
        (*tt).stopped = true;
        {
            time_watcher_stop(timer.cast());
            time_watcher_close(timer.cast(), Some(timer_close_cb));
        }
    }
}

unsafe extern "C" fn timer_close_cb(_tw: *mut TimeWatcher, data: *mut c_void) {
    unsafe {
        let timer = data.cast::<timer_T>();
        let wf = watcher_fields(timer.cast());
        let tt = timer_tail(timer);
        multiqueue_free((*wf).events);
        callback_free(&raw mut (*tt).callback);
        let _ = map_del_uint64_t_ptr_t(&raw mut timers, (*tt).timer_id as u64, ptr::null_mut());
        timer_decref(timer);
    }
}

unsafe fn timer_decref(timer: *mut timer_T) {
    unsafe {
        let tt = timer_tail(timer);
        (*tt).refcount -= 1;
        if (*tt).refcount == 0 {
            xfree(timer.cast());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_stop_all() {
    unsafe {
        for i in 0..(timers.set.h.n_keys as usize) {
            let timer = (*timers.values.add(i)).cast::<timer_T>();
            timer_stop(timer);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_teardown() {
    unsafe {
        timer_stop_all();
    }
}

/// Saves a typval_T as a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn save_tv_as_string(
    tv: *mut typval_T,
    len: *mut isize,
    endnl: bool,
    crlf: bool,
) -> *mut c_char {
    unsafe {
        *len = 0;
        if (*tv).v_type == VarType::VAR_UNKNOWN {
            return ptr::null_mut();
        }

        if (*tv).v_type != VarType::VAR_LIST && (*tv).v_type != VarType::VAR_NUMBER {
            let ret = tv_get_string_chk(tv);
            if !ret.is_null() {
                *len = strlen(ret) as isize;
                return xmemdupz(ret.cast(), *len as usize).cast();
            } else {
                *len = -1;
                return ptr::null_mut();
            }
        }

        if (*tv).v_type == VarType::VAR_NUMBER {
            let buf = buflist_findnr((*tv).vval.v_number as c_int);
            if !buf.is_null() {
                for lnum in 1..=(*buf).b_ml.ml_line_count {
                    let mut p = ml_get_buf(buf, lnum);
                    while *p != NUL {
                        *len += 1;
                        p = p.add(1);
                    }
                    *len += 1;
                }
            } else {
                semsg!(gettext((&raw const e_nobufnr).cast()), (*tv).vval.v_number);
                *len = -1;
                return ptr::null_mut();
            }

            if *len == 0 {
                return ptr::null_mut();
            }

            let ret = xmalloc((*len as usize) + 1).cast::<c_char>();
            let mut end = ret;
            for lnum in 1..=(*buf).b_ml.ml_line_count {
                let mut p = ml_get_buf(buf, lnum);
                while *p != NUL {
                    *end = if *p == b'\n' as c_char { NUL } else { *p };
                    end = end.add(1);
                    p = p.add(1);
                }
                *end = b'\n' as c_char;
                end = end.add(1);
            }
            *end = NUL;
            *len = (end as isize) - (ret as isize);
            return ret;
        }

        debug_assert!((*tv).v_type == VarType::VAR_LIST);
        let list = (*tv).vval.v_list;
        let sep_len = if crlf { 2isize } else { 1isize };
        let mut li = if list.is_null() {
            ptr::null_mut()
        } else {
            (*list).lv_first
        };
        while !li.is_null() {
            *len += (strlen(tv_get_string(tv_list_item_tv(li))) as isize) + sep_len;
            li = (*li).li_next;
        }

        if *len == 0 {
            return ptr::null_mut();
        }

        let extra = if endnl {
            if crlf { 2usize } else { 1usize }
        } else {
            0usize
        };
        let ret = xmalloc((*len as usize) + extra).cast::<c_char>();
        let mut end = ret;
        li = if list.is_null() {
            ptr::null_mut()
        } else {
            (*list).lv_first
        };
        while !li.is_null() {
            let mut s = tv_get_string(tv_list_item_tv(li));
            while *s != NUL {
                *end = if *s == b'\n' as c_char { NUL } else { *s };
                end = end.add(1);
                s = s.add(1);
            }
            if endnl || !tv_list_item_next(list, li).is_null() {
                if crlf {
                    *end = b'\r' as c_char;
                    end = end.add(1);
                }
                *end = b'\n' as c_char;
                end = end.add(1);
            }
            li = (*li).li_next;
        }
        *end = NUL;
        *len = (end as isize) - (ret as isize);
        ret
    }
}

/// Convert the specified byte index of line 'lnum' in buffer 'buf' to a
/// character index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn buf_byteidx_to_charidx(
    buf: *mut buf_T,
    mut lnum: linenr_T,
    byteidx: c_int,
) -> c_int {
    unsafe {
        if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
            return -1;
        }

        if lnum > (*buf).b_ml.ml_line_count {
            lnum = (*buf).b_ml.ml_line_count;
        }

        let str_ptr = ml_get_buf(buf, lnum);

        if *str_ptr == NUL {
            return 0;
        }

        let mut t = str_ptr;
        let mut count: c_int = 0;
        let limit = (str_ptr as isize) + (byteidx as isize);
        while *t != NUL && (t as isize) <= limit {
            t = t.add(utfc_ptr2len(t) as usize);
            count += 1;
        }

        if *t == NUL && byteidx != 0 && (t as isize) == limit {
            count += 1;
        }

        count - 1
    }
}

/// Convert the specified character index of line 'lnum' in buffer 'buf' to a
/// byte index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn buf_charidx_to_byteidx(
    buf: *mut buf_T,
    mut lnum: linenr_T,
    mut charidx: c_int,
) -> c_int {
    unsafe {
        if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
            return -1;
        }

        if lnum > (*buf).b_ml.ml_line_count {
            lnum = (*buf).b_ml.ml_line_count;
        }

        let str_ptr = ml_get_buf(buf, lnum);

        let mut t = str_ptr;
        while *t != NUL {
            charidx -= 1;
            if charidx <= 0 {
                break;
            }
            t = t.add(utfc_ptr2len(t) as usize);
        }

        ((t as usize).wrapping_sub(str_ptr as usize)) as c_int
    }
}

/// Translate a Vimscript object into a position.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn var2fpos(
    tv: *const typval_T,
    dollar_lnum: bool,
    ret_fnum: *mut c_int,
    charcol: bool,
    wp: *mut win_T,
) -> *mut pos_T {
    unsafe {
        let bp = (*wp).w_buffer;

        if (*tv).v_type == VarType::VAR_LIST {
            let mut error = false;

            let l = (*tv).vval.v_list;
            if l.is_null() {
                return ptr::null_mut();
            }

            VAR2FPOS_POS.lnum = tv_list_find_nr(l, 0, &raw mut error) as linenr_T;
            if error || VAR2FPOS_POS.lnum <= 0 || VAR2FPOS_POS.lnum > (*bp).b_ml.ml_line_count {
                return ptr::null_mut();
            }

            VAR2FPOS_POS.col = tv_list_find_nr(l, 1, &raw mut error) as colnr_T;
            if error {
                return ptr::null_mut();
            }
            let len = if charcol {
                mb_charlen(ml_get_buf(bp, VAR2FPOS_POS.lnum))
            } else {
                ml_get_buf_len(bp, VAR2FPOS_POS.lnum)
            };

            let li = tv_list_find(l, 1);
            if !li.is_null()
                && (*tv_list_item_tv(li)).v_type == VarType::VAR_STRING
                && !(*tv_list_item_tv(li)).vval.v_string.is_null()
                && strcmp((*tv_list_item_tv(li)).vval.v_string, c"$".as_ptr()) == 0
            {
                VAR2FPOS_POS.col = len + 1;
            }

            if VAR2FPOS_POS.col == 0 || VAR2FPOS_POS.col > len + 1 {
                return ptr::null_mut();
            }
            VAR2FPOS_POS.col -= 1;

            VAR2FPOS_POS.coladd = tv_list_find_nr(l, 2, &raw mut error) as colnr_T;
            if error {
                VAR2FPOS_POS.coladd = 0;
            }

            return &raw mut VAR2FPOS_POS;
        }

        let name = tv_get_string_chk(tv);
        if name.is_null() {
            return ptr::null_mut();
        }

        VAR2FPOS_POS.lnum = 0;
        if *name == b'.' as c_char {
            VAR2FPOS_POS = pos_T {
                lnum: (*wp).w_cursor.lnum,
                col: (*wp).w_cursor.col,
                coladd: (*wp).w_cursor.coladd,
            };
        } else if *name == b'v' as c_char && *name.add(1) == NUL {
            if Visual.active && wp == curwin {
                VAR2FPOS_POS = pos_T {
                    lnum: Visual.start.lnum,
                    col: Visual.start.col,
                    coladd: Visual.start.coladd,
                };
            } else {
                VAR2FPOS_POS = pos_T {
                    lnum: (*wp).w_cursor.lnum,
                    col: (*wp).w_cursor.col,
                    coladd: (*wp).w_cursor.coladd,
                };
            }
        } else if *name == b'\'' as c_char {
            let mname = c_int::from(*name.add(1) as u8);
            let fm = mark_get(bp, wp, ptr::null_mut(), kMarkAll, mname);
            if fm.is_null() || (*fm).mark.lnum <= 0 {
                return ptr::null_mut();
            }
            VAR2FPOS_POS = (*fm).mark;
            if ascii_isupper(*name.add(1)) || ascii_isdigit(*name.add(1)) {
                *ret_fnum = (*fm).fnum;
            }
        }
        if VAR2FPOS_POS.lnum != 0 {
            if charcol {
                VAR2FPOS_POS.col = buf_byteidx_to_charidx(bp, VAR2FPOS_POS.lnum, VAR2FPOS_POS.col);
            }
            return &raw mut VAR2FPOS_POS;
        }

        VAR2FPOS_POS.coladd = 0;

        if *name == b'w' as c_char && dollar_lnum {
            check_cursor_moved(wp);

            VAR2FPOS_POS.col = 0;
            if *name.add(1) == b'0' as c_char {
                update_topline(wp);
                VAR2FPOS_POS.lnum = if (*wp).w_topline > 0 {
                    (*wp).w_topline
                } else {
                    1
                };
                return &raw mut VAR2FPOS_POS;
            } else if *name.add(1) == b'$' as c_char {
                validate_botline_win(wp);
                VAR2FPOS_POS.lnum = if (*wp).w_botline > 0 {
                    (*wp).w_botline - 1
                } else {
                    0
                };
                return &raw mut VAR2FPOS_POS;
            }
        } else if *name == b'$' as c_char {
            if dollar_lnum {
                VAR2FPOS_POS.lnum = (*bp).b_ml.ml_line_count;
                VAR2FPOS_POS.col = 0;
            } else {
                VAR2FPOS_POS.lnum = (*wp).w_cursor.lnum;
                VAR2FPOS_POS.col = if charcol {
                    mb_charlen(ml_get_buf(bp, (*wp).w_cursor.lnum))
                } else {
                    ml_get_buf_len(bp, (*wp).w_cursor.lnum)
                };
            }
            return &raw mut VAR2FPOS_POS;
        }
        ptr::null_mut()
    }
}

/// Convert list in "arg" into position "posp" and optional file number "fnump".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn list2fpos(
    arg: *mut typval_T,
    posp: *mut pos_T,
    fnump: *mut c_int,
    curswantp: *mut colnr_T,
    charcol: bool,
) -> c_int {
    unsafe {
        if (*arg).v_type != VarType::VAR_LIST {
            return FAIL;
        }
        let l = (*arg).vval.v_list;
        if l.is_null()
            || tv_list_len(l) < if fnump.is_null() { 2 } else { 3 }
            || tv_list_len(l) > if fnump.is_null() { 4 } else { 5 }
        {
            return FAIL;
        }

        let mut i: c_int = 0;
        let mut n: c_int;
        if !fnump.is_null() {
            n = tv_list_find_nr(l, i, ptr::null_mut()) as c_int;
            i += 1;
            if n < 0 {
                return FAIL;
            }
            if n == 0 {
                {
                    n = (*curbuf).b_fnum;
                }
            }
            *fnump = n;
        }

        n = tv_list_find_nr(l, i, ptr::null_mut()) as c_int;
        i += 1;
        if n < 0 {
            return FAIL;
        }
        (*posp).lnum = n;

        n = tv_list_find_nr(l, i, ptr::null_mut()) as c_int;
        i += 1;
        if n < 0 {
            return FAIL;
        }
        if charcol {
            let buf = buflist_findnr(if fnump.is_null() {
                (*curbuf).b_fnum
            } else {
                *fnump
            });
            if buf.is_null() || (*buf).b_ml.ml_mfp.is_null() {
                return FAIL;
            }
            let cur_lnum = (*curwin).w_cursor.lnum;
            n = buf_charidx_to_byteidx(
                buf,
                if (*posp).lnum == 0 {
                    cur_lnum
                } else {
                    (*posp).lnum
                },
                n,
            ) + 1;
        }
        (*posp).col = n;

        n = tv_list_find_nr(l, i, ptr::null_mut()) as c_int;
        if n < 0 {
            (*posp).coladd = 0;
        } else {
            (*posp).coladd = n;
        }

        if !curswantp.is_null() {
            *curswantp = tv_list_find_nr(l, i + 1, ptr::null_mut()) as colnr_T;
        }

        OK
    }
}

/// Get the length of an environment variable name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_env_len(arg: *mut *const c_char) -> c_int {
    unsafe {
        let len = env_name_len(*arg);
        *arg = (*arg).add(len);
        len as c_int
    }
}

/// Get the length of the name of a function or internal variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_id_len(arg: *mut *const c_char) -> c_int {
    unsafe {
        let mut p = *arg;
        while eval_isnamec(c_int::from(*p as u8)) {
            if *p == b':' as c_char {
                let len = ((p as usize).wrapping_sub(*arg as usize)) as c_int;
                if len > 1
                    || (len == 1
                        && vim_strchr(NAMESPACE_CHAR.as_ptr(), c_int::from(**arg as u8)).is_null())
                {
                    break;
                }
            }
            p = p.add(1);
        }
        if p == *arg {
            return 0;
        }

        let len = ((p as usize).wrapping_sub(*arg as usize)) as c_int;
        *arg = skipwhite(p);

        len
    }
}

/// Get the length of the name of a variable or function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_name_len(
    arg: *mut *const c_char,
    alias: *mut *mut c_char,
    evaluate: bool,
    verbose: bool,
) -> c_int {
    unsafe {
        *alias = ptr::null_mut();

        if **arg == K_SPECIAL as c_char
            && *(*arg).add(1) == KS_EXTRA as c_char
            && *(*arg).add(2) == KE_SNR as c_char
        {
            *arg = (*arg).add(3);
            return get_id_len(arg) + 3;
        }
        let mut len = eval_fname_script(*arg);
        if len > 0 {
            *arg = (*arg).add(len as usize);
        }

        let mut expr_start: *const c_char = ptr::null();
        let mut expr_end: *const c_char = ptr::null();
        let p = find_name_end(
            *arg,
            &raw mut expr_start,
            &raw mut expr_end,
            if len > 0 { 0 } else { FNE_CHECK_START },
        );
        if !expr_start.is_null() {
            if !evaluate {
                len += ((p as usize).wrapping_sub(*arg as usize)) as c_int;
                *arg = skipwhite(p);
                return len;
            }

            let temp_string = make_expanded_name(
                (*arg).sub(len as usize),
                expr_start.cast_mut(),
                expr_end.cast_mut(),
                p.cast_mut(),
            );
            if temp_string.is_null() {
                return -1;
            }
            *alias = temp_string;
            *arg = skipwhite(p);
            return strlen(temp_string) as c_int;
        }

        len += get_id_len(arg);
        if len == 0 && verbose && **arg != NUL {
            semsg!(gettext((&raw const e_invexpr2).cast()), *arg);
        }

        len
    }
}

/// Find the end of a variable or function name, taking care of magic braces.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn find_name_end(
    arg: *const c_char,
    expr_start: *mut *const c_char,
    expr_end: *mut *const c_char,
    flags: c_int,
) -> *const c_char {
    unsafe {
        if !expr_start.is_null() {
            *expr_start = ptr::null();
            *expr_end = ptr::null();
        }

        if (flags & FNE_CHECK_START) != 0
            && !eval_isnamec1(c_int::from(*arg as u8))
            && *arg != b'{' as c_char
        {
            return arg;
        }

        let mut mb_nest: c_int = 0;
        let mut br_nest: c_int = 0;

        let mut p = arg;
        while *p != NUL
            && (eval_isnamec(c_int::from(*p as u8))
                || *p == b'{' as c_char
                || ((flags & FNE_INCL_BR) != 0
                    && (*p == b'[' as c_char
                        || (*p == b'.' as c_char && eval_isdictc(c_int::from(*p.add(1) as u8)))))
                || mb_nest != 0
                || br_nest != 0)
        {
            if *p == b'\'' as c_char {
                p = p.add(1);
                while *p != NUL && *p != b'\'' as c_char {
                    p = p.add(utfc_ptr2len(p) as usize);
                }
                if *p == NUL {
                    break;
                }
            } else if *p == b'"' as c_char {
                p = p.add(1);
                while *p != NUL && *p != b'"' as c_char {
                    if *p == b'\\' as c_char && *p.add(1) != NUL {
                        p = p.add(1);
                    }
                    p = p.add(utfc_ptr2len(p) as usize);
                }
                if *p == NUL {
                    break;
                }
            } else if br_nest == 0 && mb_nest == 0 && *p == b':' as c_char {
                let len = ((p as usize).wrapping_sub(arg as usize)) as c_int;
                if (len > 1 && *p.sub(1) != b'}' as c_char)
                    || (len == 1
                        && vim_strchr(NAMESPACE_CHAR.as_ptr(), c_int::from(*arg as u8)).is_null())
                {
                    break;
                }
            }

            if mb_nest == 0 {
                if *p == b'[' as c_char {
                    br_nest += 1;
                } else if *p == b']' as c_char {
                    br_nest -= 1;
                }
            }

            if br_nest == 0 {
                if *p == b'{' as c_char {
                    mb_nest += 1;
                    if !expr_start.is_null() && (*expr_start).is_null() {
                        *expr_start = p;
                    }
                } else if *p == b'}' as c_char {
                    mb_nest -= 1;
                    if !expr_start.is_null() && mb_nest == 0 && (*expr_end).is_null() {
                        *expr_end = p;
                    }
                }
            }

            p = p.add(utfc_ptr2len(p) as usize);
        }

        p
    }
}

/// Expands out the 'magic' {}'s in a variable/function name.
unsafe fn make_expanded_name(
    in_start: *const c_char,
    mut expr_start: *mut c_char,
    mut expr_end: *mut c_char,
    in_end: *mut c_char,
) -> *mut c_char {
    unsafe {
        if expr_end.is_null() || in_end.is_null() {
            return ptr::null_mut();
        }

        let mut retval: *mut c_char = ptr::null_mut();

        *expr_start = NUL;
        *expr_end = NUL;
        let c1 = *in_end;
        *in_end = NUL;

        let mut temp_result = eval_to_string(expr_start.add(1), false, false);
        if !temp_result.is_null() {
            let retvalsize = (expr_start as usize).wrapping_sub(in_start as usize)
                + strlen(temp_result)
                + (in_end as usize).wrapping_sub(expr_end as usize)
                + 1;
            retval = xmalloc(retvalsize).cast::<c_char>();
            vim_snprintf!(
                retval,
                retvalsize,
                c"%s%s%s".as_ptr(),
                in_start,
                temp_result,
                expr_end.add(1),
            );
        }
        xfree(temp_result.cast());

        *in_end = c1;
        *expr_start = b'{' as c_char;
        *expr_end = b'}' as c_char;

        if !retval.is_null() {
            temp_result = find_name_end(
                retval,
                (&raw mut expr_start).cast(),
                (&raw mut expr_end).cast(),
                0,
            )
            .cast_mut();
            if !expr_start.is_null() {
                temp_result = make_expanded_name(retval, expr_start, expr_end, temp_result);
                xfree(retval.cast());
                retval = temp_result;
            }
        }

        retval
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn eval_isnamec(c: c_int) -> bool {
    let ch = c as u8 as c_char;
    ascii_isalnum(ch) || ch == b'_' as c_char || ch == b':' as c_char || ch == AUTOLOAD_CHAR
}

#[unsafe(no_mangle)]
pub extern "C" fn eval_isnamec1(c: c_int) -> bool {
    let ch = c as u8 as c_char;
    ascii_isalpha(ch) || ch == b'_' as c_char
}

#[unsafe(no_mangle)]
pub extern "C" fn eval_isdictc(c: c_int) -> bool {
    let ch = c as u8 as c_char;
    ascii_isalnum(ch) || ch == b'_' as c_char
}

/// Set the v:argv list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_argv_var(argv: *mut *mut c_char, argc: c_int) {
    unsafe {
        let l = tv_list_alloc(argc as isize);

        tv_list_set_lock(l, VAR_FIXED);
        for i in 0..(argc as usize) {
            tv_list_append_string(l, (*argv.add(i)).cast_const(), -1);
            (*tv_list_item_tv(tv_list_last(l))).v_lock = VAR_FIXED;
        }
        set_vim_var_list(VV_ARGV, l);
    }
}

/// Check if special v:lua value for calling lua functions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_luafunc(partial: *mut partial_T) -> bool {
    unsafe { partial == get_vim_var_partial(VV_LUA) }
}

unsafe fn tv_is_luafunc(tv: *mut typval_T) -> bool {
    unsafe { (*tv).v_type == VarType::VAR_PARTIAL && is_luafunc(tv_partial(tv)) }
}

/// Skips one character past the end of the name of a v:lua function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn skip_luafunc_name(mut p: *const c_char) -> *const c_char {
    unsafe {
        while ascii_isalnum(*p)
            || *p == b'_' as c_char
            || *p == b'-' as c_char
            || *p == b'.' as c_char
            || *p == b'\'' as c_char
        {
            p = p.add(1);
        }
        p
    }
}

/// Check the function name after "v:lua."
#[unsafe(no_mangle)]
pub unsafe extern "C" fn check_luafunc_name(str_: *const c_char, paren: bool) -> c_int {
    unsafe {
        let p = skip_luafunc_name(str_);
        if *p != if paren { b'(' as c_char } else { NUL } {
            return 0;
        }
        ((p as usize).wrapping_sub(str_ as usize)) as c_int
    }
}

/// Return the character "str[index]" where "index" is the character index,
/// including composing characters.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn char_from_string(str_: *const c_char, index: varnumber_T) -> *mut c_char {
    unsafe {
        let mut nchar = index;

        if str_.is_null() {
            return ptr::null_mut();
        }
        let slen = strlen(str_);

        if index < 0 {
            let mut clen: varnumber_T = 0;
            let mut nbyte: usize = 0;
            while nbyte < slen {
                nbyte += utfc_ptr2len(str_.add(nbyte)) as usize;
                clen += 1;
            }
            nchar = clen + index;
            if nchar < 0 {
                return ptr::null_mut();
            }
        }

        let mut nbyte: usize = 0;
        while nchar > 0 && nbyte < slen {
            nbyte += utfc_ptr2len(str_.add(nbyte)) as usize;
            nchar -= 1;
        }
        if nbyte >= slen {
            return ptr::null_mut();
        }
        xmemdupz(
            str_.add(nbyte).cast(),
            utfc_ptr2len(str_.add(nbyte)) as usize,
        )
        .cast()
    }
}

unsafe fn char_idx2byte(str_: *const c_char, str_len: usize, idx: varnumber_T) -> isize {
    unsafe {
        let mut nchar = idx;
        let mut nbyte: usize = 0;

        if nchar >= 0 {
            while nchar > 0 && nbyte < str_len {
                nbyte += utfc_ptr2len(str_.add(nbyte)) as usize;
                nchar -= 1;
            }
        } else {
            nbyte = str_len;
            while nchar < 0 && nbyte > 0 {
                nbyte -= 1;
                nbyte -= utf_head_off(str_, str_.add(nbyte)) as usize;
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn string_slice(
    str_: *const c_char,
    first: varnumber_T,
    last: varnumber_T,
    exclusive: bool,
) -> *mut c_char {
    unsafe {
        if str_.is_null() {
            return ptr::null_mut();
        }
        let slen = strlen(str_);
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
                end_byte += utfc_ptr2len(str_.add(end_byte as usize)) as isize;
            }
        }

        if start_byte >= (slen as isize) || end_byte <= start_byte {
            return ptr::null_mut();
        }
        xmemdupz(
            str_.add(start_byte as usize).cast(),
            (end_byte - start_byte) as usize,
        )
        .cast()
    }
}

/// Handle subscripts, `.name`, funcref calls, and `->method()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn handle_subscript(
    arg: *mut *const c_char,
    rettv: *mut typval_T,
    evalarg: *mut evalarg_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let evaluate = !evalarg.is_null() && ((*evalarg).eval_flags & EVAL_EVALUATE) != 0;
        let mut ret = OK;
        let mut selfdict: *mut dict_T = ptr::null_mut();
        let mut lua_funcname: *const c_char = ptr::null();

        if tv_is_luafunc(rettv) {
            if !evaluate {
                tv_clear(rettv);
            }

            if **arg != b'.' as c_char {
                tv_clear(rettv);
                ret = FAIL;
            } else {
                *arg = (*arg).add(1);

                lua_funcname = *arg;
                let len = check_luafunc_name(*arg, true);
                if len == 0 {
                    tv_clear(rettv);
                    ret = FAIL;
                }
                *arg = (*arg).add(len as usize);
            }
        }

        while ret == OK
            && (((**arg == b'[' as c_char
                || (**arg == b'.' as c_char && (*rettv).v_type == VarType::VAR_DICT)
                || (**arg == b'(' as c_char && (!evaluate || tv_is_func(*rettv))))
                && !ascii_iswhite(*(*arg).sub(1)))
                || (**arg == b'-' as c_char && *(*arg).add(1) == b'>' as c_char))
        {
            if **arg == b'(' as c_char {
                ret = call_func_rettv(
                    arg.cast(),
                    evalarg,
                    rettv,
                    evaluate,
                    selfdict,
                    ptr::null_mut(),
                    lua_funcname,
                );

                if aborting() {
                    if ret == OK {
                        tv_clear(rettv);
                    }
                    ret = FAIL;
                }
                tv_dict_unref(selfdict);
                selfdict = ptr::null_mut();
            } else if **arg == b'-' as c_char {
                if *(*arg).add(2) == b'{' as c_char {
                    ret = eval_lambda(arg.cast(), rettv, evalarg, verbose);
                } else {
                    ret = eval_method(arg.cast(), rettv, evalarg, verbose);
                }
            } else {
                tv_dict_unref(selfdict);
                if (*rettv).v_type == VarType::VAR_DICT {
                    selfdict = (*rettv).vval.v_dict;
                    if !selfdict.is_null() {
                        (*selfdict).dv_refcount += 1;
                    }
                } else {
                    selfdict = ptr::null_mut();
                }
                if eval_index(arg.cast(), rettv, evalarg, verbose) == FAIL {
                    tv_clear(rettv);
                    ret = FAIL;
                }
            }
        }

        if !selfdict.is_null() && tv_is_func(*rettv) {
            set_selfdict(rettv, selfdict);
        }

        tv_dict_unref(selfdict);
        ret
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_selfdict(rettv: *mut typval_T, selfdict: *mut dict_T) {
    unsafe {
        if (*rettv).v_type == VarType::VAR_PARTIAL
            && !(*tv_partial(rettv)).pt_auto
            && !(*tv_partial(rettv)).pt_dict.is_null()
        {
            return;
        }
        make_partial(selfdict, rettv);
    }
}

/// Make a copy of an item.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn var_item_copy(
    conv: *const vimconv_T,
    from: *mut typval_T,
    to: *mut typval_T,
    deep: bool,
    copyID: c_int,
) -> c_int {
    unsafe {
        let mut ret = OK;

        if VAR_ITEM_COPY_RECURSE >= DICT_MAXNEST {
            emsg(gettext(E_VARIABLE_NESTED_TOO_DEEP_FOR_MAKING_COPY.as_ptr()));
            return FAIL;
        }
        VAR_ITEM_COPY_RECURSE += 1;

        match (*from).v_type {
            VarType::VAR_NUMBER
            | VarType::VAR_FLOAT
            | VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL => {
                tv_copy(from, to);
            }
            VarType::VAR_STRING => {
                if conv.is_null() || (*conv).vc_type == CONV_NONE || (*from).vval.v_string.is_null()
                {
                    tv_copy(from, to);
                } else {
                    (*to).v_type = VarType::VAR_STRING;
                    (*to).v_lock = VAR_UNLOCKED;
                    (*to).vval.v_string =
                        string_convert(conv, (*from).vval.v_string, ptr::null_mut());
                    if (*to).vval.v_string.is_null() {
                        (*to).vval.v_string = xstrdup((*from).vval.v_string);
                    }
                }
            }
            VarType::VAR_LIST => {
                (*to).v_type = VarType::VAR_LIST;
                (*to).v_lock = VAR_UNLOCKED;
                if (*from).vval.v_list.is_null() {
                    (*to).vval.v_list = ptr::null_mut();
                } else if copyID != 0 && tv_list_copyid((*from).vval.v_list) == copyID {
                    (*to).vval.v_list = tv_list_latest_copy((*from).vval.v_list);
                    tv_list_ref((*to).vval.v_list);
                } else {
                    (*to).vval.v_list = tv_list_copy(conv, (*from).vval.v_list, deep, copyID);
                }
                if (*to).vval.v_list.is_null() && !(*from).vval.v_list.is_null() {
                    ret = FAIL;
                }
            }
            VarType::VAR_BLOB => {
                tv_blob_copy((*from).vval.v_blob, to);
            }
            VarType::VAR_DICT => {
                (*to).v_type = VarType::VAR_DICT;
                (*to).v_lock = VAR_UNLOCKED;
                if (*from).vval.v_dict.is_null() {
                    (*to).vval.v_dict = ptr::null_mut();
                } else if copyID != 0 && (*(*from).vval.v_dict).dv_copyID == copyID {
                    (*to).vval.v_dict = (*(*from).vval.v_dict).dv_copydict;
                    (*(*to).vval.v_dict).dv_refcount += 1;
                } else {
                    (*to).vval.v_dict = tv_dict_copy(conv, (*from).vval.v_dict, deep, copyID);
                }
                if (*to).vval.v_dict.is_null() && !(*from).vval.v_dict.is_null() {
                    ret = FAIL;
                }
            }
            VarType::VAR_UNKNOWN => {
                internal_error(c"var_item_copy(UNKNOWN)".as_ptr());
                ret = FAIL;
            }
        }
        VAR_ITEM_COPY_RECURSE -= 1;
        ret
    }
}

/// ":echo expr1 ..." and ":echon expr1 ..."
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_echo(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut rettv = tv_initial_value();
        let mut atstart = true;
        let mut need_clear = true;
        let did_emsg_before = did_emsg;
        let called_emsg_before = called_emsg;
        let mut evalarg = evalarg_T {
            eval_flags: 0,
            eval_getline: None,
            eval_cookie: ptr::null_mut(),
            eval_tofree: ptr::null_mut(),
        };

        fill_evalarg_from_eap(&raw mut evalarg, eap, (*eap).skip != 0);

        if (*eap).skip != 0 {
            emsg_skip += 1;
        }
        while *arg != NUL && *arg != b'|' as c_char && *arg != b'\n' as c_char && !got_int {
            need_clr_eos = true;

            {
                let p = arg;
                if eval1(&raw mut arg, &raw mut rettv, &raw mut evalarg) == FAIL {
                    if !aborting()
                        && did_emsg == did_emsg_before
                        && called_emsg == called_emsg_before
                    {
                        semsg!(gettext((&raw const e_invexpr2).cast()), p);
                    }
                    need_clr_eos = false;
                    break;
                }
                need_clr_eos = false;
            }

            if (*eap).skip == 0 {
                if atstart {
                    atstart = false;
                    msg_ext_set_append((*eap).cmdidx == cmdidx_T::CMD_echon as c_int);
                    msg_ext_no_fast();
                    msg_ext_set_kind(c"echo".as_ptr());
                    if (*eap).cmdidx == cmdidx_T::CMD_echo as c_int {
                        if !msg_didout {
                            msg_sb_eol();
                        }
                        msg_start();
                    }
                } else if (*eap).cmdidx == cmdidx_T::CMD_echo as c_int {
                    msg_puts_hl(c" ".as_ptr(), echo_hl_id, false);
                }
                let tofree = encode_tv2echo(&raw mut rettv, ptr::null_mut());
                msg_multiline(
                    cstr_as_string(tofree),
                    echo_hl_id,
                    true,
                    false,
                    &raw mut need_clear,
                );
                xfree(tofree.cast());
            }
            tv_clear(&raw mut rettv);
            arg = skipwhite(arg);
        }
        (*eap).nextcmd = check_nextcmd(arg);
        clear_evalarg(&raw mut evalarg, eap);
        msg_ext_set_append(false);

        if (*eap).skip != 0 {
            emsg_skip -= 1;
        } else {
            if ui_has(kUIMessages)
                && (*(*eap).arg == NUL
                    || *(*eap).arg == b'|' as c_char
                    || *(*eap).arg == b'\n' as c_char)
            {
                msg_puts_len(c"".as_ptr(), 0, 0, false);
            } else if need_clear {
                msg_clr_eos();
            }
            if (*eap).cmdidx == cmdidx_T::CMD_echo as c_int {
                msg_end();
            }
        }
    }
}

/// ":echohl {name}".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_echohl(eap: *mut exarg_T) {
    unsafe {
        echo_hl_id = syn_name2id((*eap).arg);
    }
}

/// Returns the :echo highlight id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_echo_hl_id() -> c_int {
    unsafe { echo_hl_id }
}

/// ":execute", ":echomsg", ":echoerr"
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_execute(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut rettv = tv_initial_value();
        let mut ret = OK;
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };

        ga_init(&raw mut ga, 1, 80);

        if (*eap).skip != 0 {
            emsg_skip += 1;
        }
        while *arg != NUL && *arg != b'|' as c_char && *arg != b'\n' as c_char {
            ret = eval1_emsg(&raw mut arg, &raw mut rettv, eap);
            if ret == FAIL {
                break;
            }

            if (*eap).skip == 0 {
                let argstr = if (*eap).cmdidx == cmdidx_T::CMD_execute as c_int {
                    tv_get_string(&raw const rettv)
                } else if rettv.v_type == VarType::VAR_STRING {
                    encode_tv2echo(&raw mut rettv, ptr::null_mut()).cast_const()
                } else {
                    encode_tv2string(&raw mut rettv, ptr::null_mut()).cast_const()
                };
                let len = strlen(argstr);
                ga_grow(&raw mut ga, (len as c_int) + 2);
                if ga.ga_len > 0 {
                    *ga.ga_data.cast::<c_char>().add(ga.ga_len as usize) = b' ' as c_char;
                    ga.ga_len += 1;
                }
                ptr::copy_nonoverlapping(
                    argstr,
                    ga.ga_data.cast::<c_char>().add(ga.ga_len as usize),
                    len + 1,
                );
                if (*eap).cmdidx != cmdidx_T::CMD_execute as c_int {
                    xfree(argstr.cast_mut().cast());
                }
                ga.ga_len += len as c_int;
            }

            tv_clear(&raw mut rettv);
            arg = skipwhite(arg);
        }

        if ret != FAIL && !ga.ga_data.is_null() {
            if (*eap).cmdidx == cmdidx_T::CMD_echomsg as c_int {
                msg_ext_no_fast();
                msg_ext_set_kind(c"echomsg".as_ptr());
                msg(ga.ga_data.cast(), echo_hl_id);
            } else if (*eap).cmdidx == cmdidx_T::CMD_echoerr as c_int {
                let save_did_emsg = did_emsg;
                msg_ext_no_fast();
                emsg_multiline(ga.ga_data.cast(), c"echoerr".as_ptr(), HLF_E, true);
                if !force_abort {
                    did_emsg = save_did_emsg;
                }
            } else if (*eap).cmdidx == cmdidx_T::CMD_execute as c_int {
                do_cmdline(
                    ga.ga_data.cast(),
                    (*eap).ea_getline,
                    (*eap).cookie,
                    DOCMD_NOWAIT | DOCMD_VERBOSE,
                );
            }
        }

        ga_clear(&raw mut ga);

        if (*eap).skip != 0 {
            emsg_skip -= 1;
        }

        (*eap).nextcmd = check_nextcmd(arg);
    }
}

/// Skip over the name of an option variable: "&option", "&g:option" or "&l:option".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn find_option_var_end(
    arg: *mut *const c_char,
    opt_idxp: *mut OptIndex,
    opt_flags: *mut c_int,
) -> *const c_char {
    unsafe {
        let mut p = (*arg).add(1);
        if *p == b'g' as c_char && *p.add(1) == b':' as c_char {
            *opt_flags = OPT_GLOBAL;
            p = p.add(2);
        } else if *p == b'l' as c_char && *p.add(1) == b':' as c_char {
            *opt_flags = OPT_LOCAL;
            p = p.add(2);
        } else {
            *opt_flags = 0;
        }

        let end = find_option_end(p, opt_idxp);
        *arg = if end.is_null() { *arg } else { p };
        end
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn var_flavour(varname: *mut c_char) -> var_flavour_T {
    unsafe {
        let mut p = varname;

        if ascii_isupper(*p) {
            loop {
                p = p.add(1);
                if *p == NUL {
                    break;
                }
                if ascii_islower(*p) {
                    return VAR_FLAVOUR_SESSION;
                }
            }
            return VAR_FLAVOUR_SHADA;
        }
        VAR_FLAVOUR_DEFAULT
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn var_set_global(name: *const c_char, mut vartv: typval_T) {
    unsafe {
        let mut funccall_entry: funccal_entry_T = std::mem::zeroed();

        save_funccal(&raw mut funccall_entry);
        set_var(name, strlen(name), &raw mut vartv, false);
        restore_funccal();
    }
}

/// Display script name where an item was last set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn last_set_msg(script_ctx: sctx_T) {
    unsafe {
        if script_ctx.sc_sid == 0 {
            return;
        }

        let mut should_free = false;
        let p = get_scriptname(script_ctx, &raw mut should_free);
        msg_ext_skip_verbose = true;

        verbose_enter();
        msg_puts(gettext(c"\n\tLast set from ".as_ptr()));
        msg_puts(p);
        if script_ctx.sc_lnum > 0 {
            msg_puts(gettext((&raw const line_msg).cast()));
            msg_outnum(script_ctx.sc_lnum);
        } else if script_is_lua(script_ctx.sc_sid) {
            msg_puts(gettext(c" (run Nvim with -V1 for more details)".as_ptr()));
        }
        if should_free {
            xfree(p.cast());
        }
        verbose_leave();
    }
}

/// Perform a substitution on "str" with pattern "pat" and substitute "sub".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_string_sub(
    mut str_: *mut c_char,
    mut len: usize,
    pat: *mut c_char,
    sub: *mut c_char,
    expr: *mut typval_T,
    flags: *const c_char,
    ret_len: *mut usize,
) -> *mut c_char {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        ga_init(&raw mut ga, 1, 200);

        regmatch.rm_ic = p_ic != 0;
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        if !regmatch.regprog.is_null() {
            let mut tail = str_;
            let end = str_.add(len);
            let do_all = *flags == b'g' as c_char;
            let mut zero_width: *mut c_char = ptr::null_mut();

            while vim_regexec_nl(
                &raw mut regmatch,
                str_,
                ((tail as usize).wrapping_sub(str_ as usize)) as colnr_T,
            ) {
                if regmatch.startp[0] == regmatch.endp[0] {
                    if zero_width == regmatch.startp[0] {
                        let i = utfc_ptr2len(tail);
                        ptr::copy(
                            tail,
                            ga.ga_data.cast::<c_char>().add(ga.ga_len as usize),
                            i as usize,
                        );
                        ga.ga_len += i;
                        tail = tail.add(i as usize);
                        continue;
                    }
                    zero_width = regmatch.startp[0];
                }

                let sublen = vim_regsub(&raw mut regmatch, sub, expr, tail, 0, REGSUB_MAGIC);
                if sublen <= 0 {
                    ga_clear(&raw mut ga);
                    break;
                }
                let grow_amount = ((end as isize) - (tail as isize) + (sublen as isize)
                    - ((regmatch.endp[0] as isize) - (regmatch.startp[0] as isize)))
                    as c_int;
                ga_grow(&raw mut ga, grow_amount);

                let i = ((regmatch.startp[0] as usize).wrapping_sub(tail as usize)) as c_int;
                ptr::copy(
                    tail,
                    ga.ga_data.cast::<c_char>().add(ga.ga_len as usize),
                    i as usize,
                );
                vim_regsub(
                    &raw mut regmatch,
                    sub,
                    expr,
                    ga.ga_data.cast::<c_char>().add((ga.ga_len + i) as usize),
                    sublen,
                    REGSUB_COPY | REGSUB_MAGIC,
                );
                ga.ga_len += i + sublen - 1;
                tail = regmatch.endp[0];
                if *tail == NUL {
                    break;
                }
                if !do_all {
                    break;
                }
            }

            if !ga.ga_data.is_null() {
                let rem = (end as usize).wrapping_sub(tail as usize);
                ptr::copy_nonoverlapping(
                    tail,
                    ga.ga_data.cast::<c_char>().add(ga.ga_len as usize),
                    rem + 1,
                );
                ga.ga_len += rem as c_int;
            }

            vim_regfree(regmatch.regprog);
        }

        if !ga.ga_data.is_null() {
            str_ = ga.ga_data.cast();
            len = ga.ga_len as usize;
        }
        let ret = xstrndup(str_, len);
        ga_clear(&raw mut ga);
        let empty_opt = (&raw mut empty_string_option).cast::<c_char>();
        if p_cpo == empty_opt {
            p_cpo = save_cpo;
        } else {
            if *p_cpo == NUL {
                let obj = Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string(save_cpo),
                    },
                };
                set_option_value_give_err(kOptCpoptions, obj, 0);
            }
            free_string_option(save_cpo);
        }

        if !ret_len.is_null() {
            *ret_len = len;
        }

        ret
    }
}

/// Common code for getting job callbacks for `jobstart`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn common_job_callbacks(
    vopts: *mut dict_T,
    on_stdout: *mut CallbackReader,
    on_stderr: *mut CallbackReader,
    on_exit: *mut Callback,
) -> bool {
    unsafe {
        if tv_dict_get_callback(vopts, c"on_stdout".as_ptr(), 9, &raw mut (*on_stdout).cb)
            && tv_dict_get_callback(vopts, c"on_stderr".as_ptr(), 9, &raw mut (*on_stderr).cb)
            && tv_dict_get_callback(vopts, c"on_exit".as_ptr(), 7, on_exit)
        {
            (*on_stdout).buffered = tv_dict_get_number(vopts, c"stdout_buffered".as_ptr()) != 0;
            (*on_stderr).buffered = tv_dict_get_number(vopts, c"stderr_buffered".as_ptr()) != 0;
            if (*on_stdout).buffered && (*on_stdout).cb.cb_type == kCallbackNone {
                (*on_stdout).self_dict = vopts;
            }
            if (*on_stderr).buffered && (*on_stderr).cb.cb_type == kCallbackNone {
                (*on_stderr).self_dict = vopts;
            }
            (*vopts).dv_refcount += 1;
            return true;
        }

        {
            callback_reader_free(on_stdout);
            callback_reader_free(on_stderr);
            callback_free(on_exit);
        }
        false
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn find_job(id: u64, show_error: bool) -> *mut Channel {
    unsafe {
        let data = find_channel(id);
        let is_stopped =
            if !data.is_null() && (*channel_prefix(data)).streamtype == kChannelStreamProc {
                let hdr = proc_header(channel_stream_ptr(data).cast::<Proc>());
                (*hdr).status >= 0 || (*hdr).stopped_time != 0
            } else {
                true
            };
        if data.is_null() || (*channel_prefix(data)).streamtype != kChannelStreamProc || is_stopped
        {
            if show_error {
                if !data.is_null() && (*channel_prefix(data)).streamtype != kChannelStreamProc {
                    emsg(gettext((&raw const e_invchanjob).cast()));
                } else {
                    emsg(gettext((&raw const e_invchan).cast()));
                }
            }
            return ptr::null_mut();
        }
        data
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn script_host_eval(
    name: *mut c_char,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_STRING {
            emsg(gettext((&raw const e_invarg).cast()));
            return;
        }

        let args = tv_list_alloc(1);
        tv_list_append_string(args, (*argvars).vval.v_string, -1);
        *rettv = eval_call_provider(name, c"eval".as_ptr().cast_mut(), args, false);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_call_provider(
    provider: *mut c_char,
    method: *mut c_char,
    arguments: *mut list_T,
    discard: bool,
) -> typval_T {
    unsafe {
        if !eval_has_provider(provider, false) {
            semsg!(
                c"E319: No \"%s\" provider found. Run \":checkhealth vim.provider\"".as_ptr(),
                provider,
            );
            return typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
        }

        let mut func = [0 as c_char; 256];
        let name_len = vim_snprintf!(
            func.as_mut_ptr(),
            func.len(),
            c"provider#%s#Call".as_ptr(),
            provider,
        );

        let saved_provider_caller_scope = provider_caller_scope;
        {
            let es_ptr = exestack
                .ga_data
                .cast::<EstackEntryCopy>()
                .add((exestack.ga_len - 1) as usize);
            provider_caller_scope = caller_scope {
                script_ctx: current_sctx,
                es_entry: *es_ptr,
                autocmd_fname,
                autocmd_match,
                autocmd_fname_full,
                autocmd_bufnr,
                funccalp: get_current_funccal().cast(),
            };
        }
        let mut funccal_entry: funccal_entry_T = std::mem::zeroed();
        save_funccal(&raw mut funccal_entry);
        {
            provider_call_nesting += 1;
        }

        let mut argvars = [
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_string: method },
            },
            typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_list: arguments },
            },
            tv_initial_value(),
        ];
        let mut rettv = tv_initial_value();
        tv_list_ref(arguments);

        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: ptr::null_mut(),
            fe_evaluate: true,
            fe_partial: ptr::null_mut(),
            fe_selfdict: ptr::null_mut(),
            fe_basetv: ptr::null_mut(),
            fe_found_var: false,
        };
        call_func(
            func.as_ptr(),
            name_len,
            &raw mut rettv,
            2,
            argvars.as_mut_ptr(),
            &raw mut funcexe,
        );

        tv_list_unref(arguments);
        restore_funccal();
        {
            provider_caller_scope = saved_provider_caller_scope;
            provider_call_nesting -= 1;
            debug_assert!(provider_call_nesting >= 0);
        }

        if discard {
            tv_clear(&raw mut rettv);
        }

        rettv
    }
}

/// Checks if provider for feature `feat` is enabled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_has_provider(feat: *const c_char, throw_if_fast: bool) -> bool {
    unsafe {
        if !strequal(feat, c"clipboard".as_ptr())
            && !strequal(feat, c"python3".as_ptr())
            && !strequal(feat, c"python3_compiled".as_ptr())
            && !strequal(feat, c"python3_dynamic".as_ptr())
            && !strequal(feat, c"perl".as_ptr())
            && !strequal(feat, c"ruby".as_ptr())
            && !strequal(feat, c"node".as_ptr())
        {
            return false;
        }

        if throw_if_fast && !nlua_is_deferred_safe() {
            semsg!(
                (&raw const e_fast_api_disabled).cast(),
                c"Vimscript function".as_ptr(),
            );
            return false;
        }

        let mut name = [0 as c_char; 32];
        vim_snprintf!(name.as_mut_ptr(), name.len(), c"%s".as_ptr(), feat);
        strchrsub(name.as_mut_ptr(), b'_' as c_char, NUL);

        let mut buf = [0 as c_char; 256];
        let mut tv = tv_initial_value();
        let mut len = vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"g:loaded_%s_provider".as_ptr(),
            name.as_ptr(),
        );
        if eval_variable(buf.as_ptr(), len, &raw mut tv, ptr::null_mut(), false, true) == FAIL {
            len = vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"provider#%s#bogus".as_ptr(),
                name.as_ptr(),
            );
            script_autoload(buf.as_ptr(), len as usize, false);

            len = vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"g:loaded_%s_provider".as_ptr(),
                name.as_ptr(),
            );
            if eval_variable(buf.as_ptr(), len, &raw mut tv, ptr::null_mut(), false, true) == FAIL {
                vim_snprintf!(
                    buf.as_mut_ptr(),
                    buf.len(),
                    c"provider#%s#Call".as_ptr(),
                    name.as_ptr(),
                );
                if !find_func(buf.as_ptr()).is_null() && p_lpl != 0 {
                    semsg!(
                        c"provider: %s: missing required variable g:loaded_%s_provider".as_ptr(),
                        name.as_ptr(),
                        name.as_ptr(),
                    );
                }
                return false;
            }
        }

        let mut ok = tv.v_type == VarType::VAR_NUMBER && tv.vval.v_number == 2;

        if ok {
            vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"provider#%s#Call".as_ptr(),
                name.as_ptr(),
            );
            if find_func(buf.as_ptr()).is_null() {
                semsg!(
                    c"provider: %s: g:loaded_%s_provider=2 but %s is not defined".as_ptr(),
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

/// Writes "<sourcing_name>:<sourcing_lnum>" to `buf[bufsize]`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval_fmt_source_name_line(buf: *mut c_char, bufsize: usize) {
    unsafe {
        if !exestack.ga_data.is_null() && exestack.ga_len > 0 {
            let es = exestack
                .ga_data
                .cast::<estack_T>()
                .add((exestack.ga_len - 1) as usize);
            if !(*es).es_name.is_null() {
                vim_snprintf!(
                    buf,
                    bufsize,
                    c"%s:%d".as_ptr(),
                    (*es).es_name,
                    (*es).es_lnum,
                );
                return;
            }
        }
        vim_snprintf!(buf, bufsize, c"?".as_ptr());
    }
}

/// Gets the current user-input in prompt buffer `buf`, or NULL if buffer is not a prompt buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn prompt_get_input(buf: *mut buf_T) -> *mut c_char {
    unsafe {
        if !bt_prompt(buf) {
            return ptr::null_mut();
        }
        let lnum_start = (*buf).b_prompt_start.mark.lnum;
        let lnum_last = (*buf).b_ml.ml_line_count;

        let mut text = ml_get_buf(buf, lnum_start);
        if (strlen(text) as c_int) >= (*buf).b_prompt_start.mark.col {
            text = text.add((*buf).b_prompt_start.mark.col as usize);
        }

        let mut full_text = xstrdup(text);
        for i in (lnum_start + 1)..=lnum_last {
            let half_text = concat_str(full_text, c"\n".as_ptr());
            xfree(full_text.cast());
            full_text = concat_str(half_text, ml_get_buf(buf, i));
            xfree(half_text.cast());
        }
        full_text
    }
}

/// Compare "typ1" and "typ2". Put the result in "typ1".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typval_compare(
    typ1: *mut typval_T,
    typ2: *mut typval_T,
    expr_type: exprtype_T,
    ic: bool,
) -> c_int {
    unsafe {
        let n1: varnumber_T;
        let type_is = expr_type == EXPR_IS || expr_type == EXPR_ISNOT;

        if type_is && (*typ1).v_type != (*typ2).v_type {
            n1 = varnumber_T::from(expr_type == EXPR_ISNOT);
        } else if (*typ1).v_type == VarType::VAR_BLOB || (*typ2).v_type == VarType::VAR_BLOB {
            if type_is {
                let mut eq =
                    (*typ1).v_type == (*typ2).v_type && (*typ1).vval.v_blob == (*typ2).vval.v_blob;
                if expr_type == EXPR_ISNOT {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            } else if (*typ1).v_type != (*typ2).v_type
                || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
            {
                if (*typ1).v_type != (*typ2).v_type {
                    emsg(gettext(c"E977: Can only compare Blob with Blob".as_ptr()));
                } else {
                    emsg(gettext((&raw const e_invalblob).cast()));
                }
                tv_clear(typ1);
                return FAIL;
            } else {
                let mut eq = tv_blob_equal((*typ1).vval.v_blob, (*typ2).vval.v_blob);
                if expr_type == EXPR_NEQUAL {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            }
        } else if (*typ1).v_type == VarType::VAR_LIST || (*typ2).v_type == VarType::VAR_LIST {
            if type_is {
                let mut eq =
                    (*typ1).v_type == (*typ2).v_type && (*typ1).vval.v_list == (*typ2).vval.v_list;
                if expr_type == EXPR_ISNOT {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            } else if (*typ1).v_type != (*typ2).v_type
                || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
            {
                if (*typ1).v_type != (*typ2).v_type {
                    emsg(gettext(c"E691: Can only compare List with List".as_ptr()));
                } else {
                    emsg(gettext(c"E692: Invalid operation for List".as_ptr()));
                }
                tv_clear(typ1);
                return FAIL;
            } else {
                let mut eq = tv_list_equal((*typ1).vval.v_list, (*typ2).vval.v_list, ic);
                if expr_type == EXPR_NEQUAL {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            }
        } else if (*typ1).v_type == VarType::VAR_DICT || (*typ2).v_type == VarType::VAR_DICT {
            if type_is {
                let mut eq =
                    (*typ1).v_type == (*typ2).v_type && (*typ1).vval.v_dict == (*typ2).vval.v_dict;
                if expr_type == EXPR_ISNOT {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            } else if (*typ1).v_type != (*typ2).v_type
                || (expr_type != EXPR_EQUAL && expr_type != EXPR_NEQUAL)
            {
                if (*typ1).v_type != (*typ2).v_type {
                    emsg(gettext(
                        c"E735: Can only compare Dictionary with Dictionary".as_ptr(),
                    ));
                } else {
                    emsg(gettext(c"E736: Invalid operation for Dictionary".as_ptr()));
                }
                tv_clear(typ1);
                return FAIL;
            } else {
                let mut eq = tv_dict_equal((*typ1).vval.v_dict, (*typ2).vval.v_dict, ic);
                if expr_type == EXPR_NEQUAL {
                    eq = !eq;
                }
                n1 = varnumber_T::from(eq);
            }
        } else if tv_is_func(*typ1) || tv_is_func(*typ2) {
            if expr_type != EXPR_EQUAL
                && expr_type != EXPR_NEQUAL
                && expr_type != EXPR_IS
                && expr_type != EXPR_ISNOT
            {
                emsg(gettext(c"E694: Invalid operation for Funcrefs".as_ptr()));
                tv_clear(typ1);
                return FAIL;
            }
            let mut eq = if ((*typ1).v_type == VarType::VAR_PARTIAL && tv_partial(typ1).is_null())
                || ((*typ2).v_type == VarType::VAR_PARTIAL && tv_partial(typ2).is_null())
            {
                tv_partial(typ1) == tv_partial(typ2)
            } else if type_is {
                if (*typ1).v_type == VarType::VAR_FUNC && (*typ2).v_type == VarType::VAR_FUNC {
                    tv_equal(typ1, typ2, ic)
                } else if (*typ1).v_type == VarType::VAR_PARTIAL
                    && (*typ2).v_type == VarType::VAR_PARTIAL
                {
                    tv_partial(typ1) == tv_partial(typ2)
                } else {
                    false
                }
            } else {
                tv_equal(typ1, typ2, ic)
            };
            if expr_type == EXPR_NEQUAL || expr_type == EXPR_ISNOT {
                eq = !eq;
            }
            n1 = varnumber_T::from(eq);
        } else if ((*typ1).v_type == VarType::VAR_FLOAT || (*typ2).v_type == VarType::VAR_FLOAT)
            && expr_type != EXPR_MATCH
            && expr_type != EXPR_NOMATCH
        {
            let f1 = tv_get_float(typ1);
            let f2 = tv_get_float(typ2);
            let cmp = match expr_type {
                EXPR_IS | EXPR_EQUAL => f1 == f2,
                EXPR_ISNOT | EXPR_NEQUAL => f1 != f2,
                EXPR_GREATER => f1 > f2,
                EXPR_GEQUAL => f1 >= f2,
                EXPR_SMALLER => f1 < f2,
                EXPR_SEQUAL => f1 <= f2,
                _ => false,
            };
            n1 = varnumber_T::from(cmp);
        } else if ((*typ1).v_type == VarType::VAR_NUMBER || (*typ2).v_type == VarType::VAR_NUMBER)
            && expr_type != EXPR_MATCH
            && expr_type != EXPR_NOMATCH
        {
            let v1 = tv_get_number(typ1);
            let v2 = tv_get_number(typ2);
            let cmp = match expr_type {
                EXPR_IS | EXPR_EQUAL => v1 == v2,
                EXPR_ISNOT | EXPR_NEQUAL => v1 != v2,
                EXPR_GREATER => v1 > v2,
                EXPR_GEQUAL => v1 >= v2,
                EXPR_SMALLER => v1 < v2,
                EXPR_SEQUAL => v1 <= v2,
                _ => false,
            };
            n1 = varnumber_T::from(cmp);
        } else {
            let mut buf1 = [0 as c_char; NUMBUFLEN];
            let mut buf2 = [0 as c_char; NUMBUFLEN];
            let s1 = tv_get_string_buf(typ1, buf1.as_mut_ptr());
            let s2 = tv_get_string_buf(typ2, buf2.as_mut_ptr());
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
            n1 = varnumber_T::from(cmp);
        }
        tv_clear(typ1);
        (*typ1).v_type = VarType::VAR_NUMBER;
        (*typ1).vval.v_number = n1;
        OK
    }
}

/// Convert any type to a string, never give an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn typval_tostring(arg: *mut typval_T, quotes: bool) -> *mut c_char {
    unsafe {
        if arg.is_null() {
            return xstrdup(c"(does not exist)".as_ptr());
        }
        if !quotes && (*arg).v_type == VarType::VAR_STRING {
            return xstrdup(if (*arg).vval.v_string.is_null() {
                c"".as_ptr()
            } else {
                (*arg).vval.v_string
            });
        }
        encode_tv2string(arg, ptr::null_mut())
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
        assert!(eval_isnamec1(b'a' as c_int));
        assert!(eval_isnamec1(b'_' as c_int));
        assert!(!eval_isnamec1(b'1' as c_int));
        assert!(!eval_isnamec1(b':' as c_int));

        assert!(eval_isnamec(b'a' as c_int));
        assert!(eval_isnamec(b'9' as c_int));
        assert!(eval_isnamec(b':' as c_int));
        assert!(eval_isnamec(b'#' as c_int));
        assert!(!eval_isnamec(b'-' as c_int));

        assert!(eval_isdictc(b'a' as c_int));
        assert!(eval_isdictc(b'9' as c_int));
        assert!(eval_isdictc(b'_' as c_int));
        assert!(!eval_isdictc(b':' as c_int));
    }

    #[test]
    fn test_var_flavour_and_luafunc_name() {
        unsafe {
            assert_eq!(var_flavour(c"foo".as_ptr().cast_mut()), VAR_FLAVOUR_DEFAULT);
            assert_eq!(var_flavour(c"Foo".as_ptr().cast_mut()), VAR_FLAVOUR_SESSION);
            assert_eq!(var_flavour(c"FOO".as_ptr().cast_mut()), VAR_FLAVOUR_SHADA);

            assert_eq!(check_luafunc_name(c"my_mod.fn(".as_ptr(), true), 9);
            assert_eq!(check_luafunc_name(c"my_mod.fn".as_ptr(), false), 9);
            assert_eq!(check_luafunc_name(c"my_mod.fn(".as_ptr(), false), 0);
        }
    }

    #[test]
    fn test_string2float() {
        unsafe {
            let mut val: float_T = 0.0;
            assert_eq!(string2float(c"1.5".as_ptr(), &raw mut val), 3);
            assert_eq!(val, 1.5);
            assert_eq!(string2float(c"inf".as_ptr(), &raw mut val), 3);
            assert!(val.is_infinite() && val.is_sign_positive());
            assert_eq!(string2float(c"-inf".as_ptr(), &raw mut val), 4);
            assert!(val.is_infinite() && val.is_sign_negative());
            assert_eq!(string2float(c"nan".as_ptr(), &raw mut val), 3);
            assert!(val.is_nan());
        }
    }
}
