//! Functions for dealing with Vimscript variables (`src/nvim/eval/vars.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use crate::api::options::CtxSwitch;
use crate::api::private::helpers::{
    cstr_as_string, cstr_to_string, nil_obj, string_obj, tv_dict_is_watched,
};
use crate::api::private::validate::{Object, ObjectData, ObjectType};
use crate::api::tabpage::{tabpage_T, win_T};
use crate::charset::{
    ascii_isdigit, ascii_isupper, ascii_iswhite, getdigits_int, skiptowhite, skipwhite,
};
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::cmdexpand::{cmdidx_T, exarg_T};
use crate::context::{ctx_restore, ctx_switch};
use crate::drawscreen::redraw_all_later;
use crate::errors::*;
use crate::eval::buffer::varnumber_T;
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{EvalFuncData, ScopeDictDictItem, buf_T};
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::executor::blob_T;
use crate::eval::funcs::{tv_get_buf, tv_get_buf_from_arg};
use crate::eval::typval::{
    tv_check_str_or_nr, tv_clear, tv_copy, tv_dict_add, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_hi2di, tv_dict_item_alloc, tv_dict_item_remove, tv_dict_set_keys_readonly,
    tv_dict_unref, tv_dict_watcher_notify, tv_get_bool_chk, tv_get_number, tv_get_number_chk,
    tv_get_string, tv_get_string_buf_chk, tv_get_string_chk, tv_is_func, tv_item_lock,
    tv_list_alloc, tv_list_append_allocated_string, tv_list_append_string, tv_list_append_tv,
    tv_list_find_nr, tv_list_find_str, tv_list_first, tv_list_free, tv_list_item_remove,
    tv_list_len, tv_list_ref, tv_list_remove_items, tv_list_set_lock, tv_list_set_ret,
    tv_list_unref, value_check_lock,
};
use crate::eval::userfunc::{
    find_hi_in_scoped_ht, find_var_in_scoped_ht, function_exists, get_current_funccal_dict,
    get_funccal_args_dict, get_funccal_args_ht, get_funccal_args_var, get_funccal_local_dict,
    get_funccal_local_ht, get_funccal_local_var, list_func_vars,
};
use crate::eval::window::find_win_by_nr;
use crate::eval::{
    callback_call, clear_evalarg, clear_lval, eval_isnamec1, eval_lavars_used, eval_option,
    eval_to_string, eval0, eval1, fill_evalarg_from_eap, find_name_end, find_option_var_end,
    get_env_len, get_lval, get_name_len, handle_subscript, may_call_simple_func, num_divide,
    num_modulus, partial_unref, set_ref_in_ht, set_var_lval, skip_expr,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{check_nextcmd, ends_excmd};
use crate::ex_eval::aborting;
use crate::extmark::TriState;
use crate::garray::{ga_append, ga_clear, ga_concat, ga_concat_len, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::hashtab::{
    HT_INIT_SIZE, hash_add, hash_clear, hash_find, hash_find_len, hash_init, hash_lock,
    hash_remove, hash_unlock, hashitem_T, hashitem_empty, hashtab_T,
};
use crate::lua::executor::{nlua_funcref_str, nlua_set_sctx};
use crate::math::{FAIL, OK};
use crate::mbyte::utf_char2bytes;
use crate::memory::{
    strequal, xcalloc, xfree, xmalloc, xmallocz, xmemdupz, xstrdup, xstrlcat, xstrlcpy, xstrndup,
};
use crate::message::{
    emsg, internal_error, message_filtered, msg_advance, msg_clr_eos, msg_ext_set_kind,
    msg_outtrans, msg_putchar, msg_puts, msg_puts_len, msg_start, semsg,
};
use crate::option::{
    find_option, get_option, get_option_sctx, get_option_value, get_tty_option, get_winbuf_options,
    is_option_hidden, is_tty_option, opt_from_tristate, option_has_type, optval_free,
    set_option_value, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::optionstr::CharBuf;
use crate::os::env::{
    expand_T, os_env_exists, os_getenv_noalloc, os_unsetenv, vim_getenv, vim_setenv_ext,
    vim_unsetenv_ext,
};
use crate::register::{get_reg_contents, write_reg_contents};
use crate::runtime::{new_script_item, script_autoload, script_item, script_items, scriptvar_T};
use crate::search::set_search_direction;
use crate::strings::{
    concat_str, gettext, strcmp as c_strcmp, strlen as c_strlen, strncmp as c_strncmp,
    vim_snprintf, vim_strchr,
};
use crate::version::{highest_patch, min_vim_version};
use crate::window::{find_tabpage, goto_tabpage_tp, prevwin_curwin, valid_tabpage};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub type VimVarIndex = c_int;
pub use crate::eval::funcs::SpecialVarValue;
pub use crate::eval::typval::BoolVarValue;
pub use crate::eval::typval::VarLockStatus;
pub type ScopeType = c_int;
pub type scid_T = c_int;
pub use crate::cmdexpand::LineGetter;
pub use crate::lua::converter::LuaRef;
pub use crate::option::OptInt;
pub use crate::optionstr::OptIndex;
type ex_unletlock_callback =
    unsafe extern "C" fn(*mut lval_T, *mut c_char, *mut exarg_T, c_int) -> c_int;

pub use crate::api::private::converter::partial_T;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct evalarg_T {
    pub eval_flags: c_int,
    pub eval_getline: LineGetter,
    pub eval_cookie: *mut c_void,
    pub eval_tofree: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct lval_T {
    pub ll_name: *const c_char,
    pub ll_name_len: usize,
    pub ll_exp_name: *mut c_char,
    pub ll_tv: *mut typval_T,
    pub ll_li: *mut listitem_T,
    pub ll_list: *mut list_T,
    pub ll_range: bool,
    pub ll_empty2: bool,
    pub ll_n1: c_int,
    pub ll_n2: c_int,
    pub ll_dict: *mut dict_T,
    pub ll_di: *mut dictitem_T,
    pub ll_newkey: *mut c_char,
    pub ll_blob: *mut blob_T,
}

const NUL: c_char = 0;
const IOSIZE: usize = 1025;
const NUMBUFLEN: usize = 65;
const MB_MAXCHAR: usize = 6;
const MAXCOL: varnumber_T = 0x7fffffff;
const NOTDONE: c_int = 2;

const DICT_MAXNEST: c_int = 100;
const DO_NOT_FREE_CNT: c_int = c_int::MAX / 2;
const VIMVAR_KEY_LEN: usize = 16;

const VV_COMPAT: c_char = 1;
const VV_RO: c_char = 2;
const VV_RO_SBX: c_char = 4;

const DI_FLAGS_RO: u8 = 1;
const DI_FLAGS_RO_SBX: u8 = 2;
const DI_FLAGS_FIX: u8 = 4;
const DI_FLAGS_LOCK: u8 = 8;
const DI_FLAGS_ALLOC: u8 = 16;

const VAR_UNLOCKED: VarLockStatus = 0;
const VAR_FIXED: VarLockStatus = 2;

const VAR_SCOPE: ScopeType = 1;
const VAR_DEF_SCOPE: ScopeType = 2;

const VAR_TYPE_NUMBER: varnumber_T = 0;
const VAR_TYPE_STRING: varnumber_T = 1;
const VAR_TYPE_FUNC: varnumber_T = 2;
const VAR_TYPE_LIST: varnumber_T = 3;
const VAR_TYPE_DICT: varnumber_T = 4;
const VAR_TYPE_FLOAT: varnumber_T = 5;
const VAR_TYPE_BOOL: varnumber_T = 6;
const VAR_TYPE_BLOB: varnumber_T = 10;

const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;
const kSpecialVarNull: SpecialVarValue = 0;
const kListLenUnknown: isize = -1;

const kNone: TriState = -1;
const kFalse: TriState = 0;
const kTrue: TriState = 1;

const CHAN_STDERR: varnumber_T = 2;
const ENV_STARTREASON: &CStr = c"__NVIM_STARTREASON";
const AUTOLOAD_CHAR: c_char = b'#' as c_char;

const SID_LUA: scid_T = -8;
const SID_STR: scid_T = -10;

const FNE_INCL_BR: c_int = 1;
const FNE_CHECK_START: c_int = 2;
const GLV_QUIET: c_int = 2;

const FORCE_BIN: c_int = 1;
const FORCE_NOBIN: c_int = 2;
const BAD_KEEP: c_int = -1;
const BAD_DROP: c_int = -2;

const TV_TRANSLATE: usize = usize::MAX;
const TV_CSTRING: usize = usize::MAX - 1;

const UPD_SOME_VALID: c_int = 35;

const kCtxNoDisplay: c_int = 4;
const kCtxNoEvents: c_int = 8;

const kGRegExprSrc: c_int = 2;

const OPT_LOCAL: c_int = 0x02;
const kOptInvalid: OptIndex = -1;
const kOptCharconvert: OptIndex = 36;
const kOptDiffexpr: OptIndex = 72;
const kOptPatchexpr: OptIndex = 217;
const kOptSpellsuggest: OptIndex = 292;
const kOptFlagFunc: u32 = 1 << 24;

pub const NUM_MSGPACK_TYPES: usize = 8;

pub const VV_COUNT: VimVarIndex = 0;
pub const VV_COUNT1: VimVarIndex = 1;
pub const VV_PREVCOUNT: VimVarIndex = 2;
pub const VV_ERRMSG: VimVarIndex = 3;
pub const VV_WARNINGMSG: VimVarIndex = 4;
pub const VV_STATUSMSG: VimVarIndex = 5;
pub const VV_SHELL_ERROR: VimVarIndex = 6;
pub const VV_THIS_SESSION: VimVarIndex = 7;
pub const VV_VERSION: VimVarIndex = 8;
pub const VV_LNUM: VimVarIndex = 9;
pub const VV_TERMREQUEST: VimVarIndex = 10;
pub const VV_TERMRESPONSE: VimVarIndex = 11;
pub const VV_FNAME: VimVarIndex = 12;
pub const VV_LANG: VimVarIndex = 13;
pub const VV_LC_TIME: VimVarIndex = 14;
pub const VV_CTYPE: VimVarIndex = 15;
pub const VV_CC_FROM: VimVarIndex = 16;
pub const VV_CC_TO: VimVarIndex = 17;
pub const VV_FNAME_IN: VimVarIndex = 18;
pub const VV_FNAME_OUT: VimVarIndex = 19;
pub const VV_FNAME_NEW: VimVarIndex = 20;
pub const VV_FNAME_DIFF: VimVarIndex = 21;
pub const VV_CMDARG: VimVarIndex = 22;
pub const VV_FOLDSTART: VimVarIndex = 23;
pub const VV_FOLDEND: VimVarIndex = 24;
pub const VV_FOLDDASHES: VimVarIndex = 25;
pub const VV_FOLDLEVEL: VimVarIndex = 26;
pub const VV_PROGNAME: VimVarIndex = 27;
pub const VV_SEND_SERVER: VimVarIndex = 28;
pub const VV_DYING: VimVarIndex = 29;
pub const VV_EXCEPTION: VimVarIndex = 30;
pub const VV_THROWPOINT: VimVarIndex = 31;
pub const VV_REG: VimVarIndex = 32;
pub const VV_CMDBANG: VimVarIndex = 33;
pub const VV_INSERTMODE: VimVarIndex = 34;
pub const VV_VAL: VimVarIndex = 35;
pub const VV_KEY: VimVarIndex = 36;
pub const VV_PROFILING: VimVarIndex = 37;
pub const VV_FCS_REASON: VimVarIndex = 38;
pub const VV_FCS_CHOICE: VimVarIndex = 39;
pub const VV_BEVAL_BUFNR: VimVarIndex = 40;
pub const VV_BEVAL_WINNR: VimVarIndex = 41;
pub const VV_BEVAL_WINID: VimVarIndex = 42;
pub const VV_BEVAL_LNUM: VimVarIndex = 43;
pub const VV_BEVAL_COL: VimVarIndex = 44;
pub const VV_BEVAL_TEXT: VimVarIndex = 45;
pub const VV_SCROLLSTART: VimVarIndex = 46;
pub const VV_SWAPNAME: VimVarIndex = 47;
pub const VV_SWAPCHOICE: VimVarIndex = 48;
pub const VV_SWAPCOMMAND: VimVarIndex = 49;
pub const VV_CHAR: VimVarIndex = 50;
pub const VV_MOUSE_WIN: VimVarIndex = 51;
pub const VV_MOUSE_WINID: VimVarIndex = 52;
pub const VV_MOUSE_LNUM: VimVarIndex = 53;
pub const VV_MOUSE_COL: VimVarIndex = 54;
pub const VV_OP: VimVarIndex = 55;
pub const VV_SEARCHFORWARD: VimVarIndex = 56;
pub const VV_HLSEARCH: VimVarIndex = 57;
pub const VV_OLDFILES: VimVarIndex = 58;
pub const VV_WINDOWID: VimVarIndex = 59;
pub const VV_PROGPATH: VimVarIndex = 60;
pub const VV_COMPLETED_ITEM: VimVarIndex = 61;
pub const VV_OPTION_NEW: VimVarIndex = 62;
pub const VV_OPTION_OLD: VimVarIndex = 63;
pub const VV_OPTION_OLDLOCAL: VimVarIndex = 64;
pub const VV_OPTION_OLDGLOBAL: VimVarIndex = 65;
pub const VV_OPTION_COMMAND: VimVarIndex = 66;
pub const VV_OPTION_TYPE: VimVarIndex = 67;
pub const VV_ERRORS: VimVarIndex = 68;
pub const VV_FALSE: VimVarIndex = 69;
pub const VV_TRUE: VimVarIndex = 70;
pub const VV_NULL: VimVarIndex = 71;
pub const VV_NUMBERMAX: VimVarIndex = 72;
pub const VV_NUMBERMIN: VimVarIndex = 73;
pub const VV_NUMBERSIZE: VimVarIndex = 74;
pub const VV_VIM_DID_ENTER: VimVarIndex = 75;
pub const VV_TESTING: VimVarIndex = 76;
pub const VV_TYPE_NUMBER: VimVarIndex = 77;
pub const VV_TYPE_STRING: VimVarIndex = 78;
pub const VV_TYPE_FUNC: VimVarIndex = 79;
pub const VV_TYPE_LIST: VimVarIndex = 80;
pub const VV_TYPE_DICT: VimVarIndex = 81;
pub const VV_TYPE_FLOAT: VimVarIndex = 82;
pub const VV_TYPE_BOOL: VimVarIndex = 83;
pub const VV_TYPE_BLOB: VimVarIndex = 84;
pub const VV_EVENT: VimVarIndex = 85;
pub const VV_VERSIONLONG: VimVarIndex = 86;
pub const VV_ECHOSPACE: VimVarIndex = 87;
pub const VV_ARGF: VimVarIndex = 88;
pub const VV_ARGV: VimVarIndex = 89;
pub const VV_COLLATE: VimVarIndex = 90;
pub const VV_EXITING: VimVarIndex = 91;
pub const VV_MAXCOL: VimVarIndex = 92;
pub const VV_STACKTRACE: VimVarIndex = 93;
pub const VV_VIM_DID_INIT: VimVarIndex = 94;
pub const VV_STDERR: VimVarIndex = 95;
pub const VV_MSGPACK_TYPES: VimVarIndex = 96;
pub const VV__NULL_STRING: VimVarIndex = 97;
pub const VV__NULL_LIST: VimVarIndex = 98;
pub const VV__NULL_DICT: VimVarIndex = 99;
pub const VV__NULL_BLOB: VimVarIndex = 100;
pub const VV_LUA: VimVarIndex = 101;
pub const VV_RELNUM: VimVarIndex = 102;
pub const VV_VIRTNUM: VimVarIndex = 103;
pub const VV_STARTTIME: VimVarIndex = 104;
pub const VV_EXITREASON: VimVarIndex = 105;
pub const VV_USERACTIVE: VimVarIndex = 106;
pub const VV_STARTREASON: VimVarIndex = 107;
pub const VV_CASCADING: VimVarIndex = 108;
const VV_LEN: usize = 109;

const E_LETUNEXP: &CStr = c"E18: Unexpected characters in :let";
const E_DOUBLE_SEMICOLON_IN_LIST_OF_VARIABLES: &CStr = c"E452: Double ; in list of variables";
const E_LOCK_UNLOCK: &CStr = c"E940: Cannot lock or unlock variable %s";
const E_SETTING_V_STR_TO_VALUE_WITH_WRONG_TYPE: &CStr =
    c"E963: Setting v:%s to value with wrong type";
const E_MISSING_END_MARKER_STR: &CStr = c"E990: Missing end marker '%s'";
const E_CANNOT_USE_HEREDOC_HERE: &CStr = c"E991: Cannot use =<< here";

#[repr(C)]
#[derive(Clone, Copy)]
struct VimVarDictItem {
    di_tv: typval_T,
    di_flags: u8,
    di_key: [c_char; VIMVAR_KEY_LEN + 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VimVar {
    vv_name: *const c_char,
    vv_di: VimVarDictItem,
    vv_flags: c_char,
}

const fn make_vimvar(name: &'static CStr, v_type: VarType, flags: c_char) -> VimVar {
    let bytes = name.to_bytes();
    let mut key = [0 as c_char; VIMVAR_KEY_LEN + 1];
    let mut i = 0;
    while i < bytes.len() && i < VIMVAR_KEY_LEN {
        key[i] = bytes[i] as c_char;
        i += 1;
    }
    VimVar {
        vv_name: name.as_ptr(),
        vv_di: VimVarDictItem {
            di_tv: typval_T {
                v_type,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
            di_flags: 0,
            di_key: key,
        },
        vv_flags: flags,
    }
}

const fn empty_hashtab() -> hashtab_T {
    hashtab_T {
        ht_mask: 0,
        ht_used: 0,
        ht_filled: 0,
        ht_changed: 0,
        ht_locked: 0,
        ht_array: ptr::null_mut(),
        ht_smallarray: [hashitem_T {
            hi_hash: 0,
            hi_key: ptr::null_mut(),
        }; HT_INIT_SIZE],
    }
}

const fn empty_dict() -> dict_T {
    dict_T {
        dv_lock: 0,
        dv_scope: 0,
        dv_refcount: 0,
        dv_copyID: 0,
        dv_hashtab: empty_hashtab(),
        dv_copydict: ptr::null_mut(),
        dv_used_next: ptr::null_mut(),
        dv_used_prev: ptr::null_mut(),
        watchers: [ptr::null_mut(); 2],
        lua_table_ref: 0,
    }
}

const fn empty_scope_var() -> ScopeDictDictItem {
    ScopeDictDictItem {
        di_tv: typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        },
        di_flags: 0,
        di_key: [0],
    }
}

static mut globvars_var: ScopeDictDictItem = empty_scope_var();
static mut globvardict: dict_T = empty_dict();
static mut compat_hashtab: hashtab_T = empty_hashtab();

static mut vimvars_var: ScopeDictDictItem = empty_scope_var();
static mut vimvardict: dict_T = empty_dict();

// Note: indexed by `VV_*` enum values (`VV_TERMREQUEST` = 10, `VV_TERMRESPONSE` = 11).
static mut vimvars: [VimVar; VV_LEN] = [
    make_vimvar(c"count", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"count1", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"prevcount", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"errmsg", VarType::VAR_STRING, 0),
    make_vimvar(c"warningmsg", VarType::VAR_STRING, 0),
    make_vimvar(c"statusmsg", VarType::VAR_STRING, 0),
    make_vimvar(c"shell_error", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"this_session", VarType::VAR_STRING, 0),
    make_vimvar(c"version", VarType::VAR_NUMBER, VV_COMPAT + VV_RO),
    make_vimvar(c"lnum", VarType::VAR_NUMBER, VV_RO_SBX),
    make_vimvar(c"termrequest", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"termresponse", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fname", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"lang", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"lc_time", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"ctype", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"charconvert_from", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"charconvert_to", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fname_in", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fname_out", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fname_new", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fname_diff", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"cmdarg", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"foldstart", VarType::VAR_NUMBER, VV_RO_SBX),
    make_vimvar(c"foldend", VarType::VAR_NUMBER, VV_RO_SBX),
    make_vimvar(c"folddashes", VarType::VAR_STRING, VV_RO_SBX),
    make_vimvar(c"foldlevel", VarType::VAR_NUMBER, VV_RO_SBX),
    make_vimvar(c"progname", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"servername", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"dying", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"exception", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"throwpoint", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"register", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"cmdbang", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"insertmode", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"val", VarType::VAR_UNKNOWN, VV_RO),
    make_vimvar(c"key", VarType::VAR_UNKNOWN, VV_RO),
    make_vimvar(c"profiling", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"fcs_reason", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"fcs_choice", VarType::VAR_STRING, 0),
    make_vimvar(c"beval_bufnr", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"beval_winnr", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"beval_winid", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"beval_lnum", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"beval_col", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"beval_text", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"scrollstart", VarType::VAR_STRING, 0),
    make_vimvar(c"swapname", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"swapchoice", VarType::VAR_STRING, 0),
    make_vimvar(c"swapcommand", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"char", VarType::VAR_STRING, 0),
    make_vimvar(c"mouse_win", VarType::VAR_NUMBER, 0),
    make_vimvar(c"mouse_winid", VarType::VAR_NUMBER, 0),
    make_vimvar(c"mouse_lnum", VarType::VAR_NUMBER, 0),
    make_vimvar(c"mouse_col", VarType::VAR_NUMBER, 0),
    make_vimvar(c"operator", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"searchforward", VarType::VAR_NUMBER, 0),
    make_vimvar(c"hlsearch", VarType::VAR_NUMBER, 0),
    make_vimvar(c"oldfiles", VarType::VAR_LIST, 0),
    make_vimvar(c"windowid", VarType::VAR_NUMBER, VV_RO_SBX),
    make_vimvar(c"progpath", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"completed_item", VarType::VAR_DICT, 0),
    make_vimvar(c"option_new", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"option_old", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"option_oldlocal", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"option_oldglobal", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"option_command", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"option_type", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"errors", VarType::VAR_LIST, 0),
    make_vimvar(c"false", VarType::VAR_BOOL, VV_RO),
    make_vimvar(c"true", VarType::VAR_BOOL, VV_RO),
    make_vimvar(c"null", VarType::VAR_SPECIAL, VV_RO),
    make_vimvar(c"numbermax", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"numbermin", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"numbersize", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"vim_did_enter", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"testing", VarType::VAR_NUMBER, 0),
    make_vimvar(c"t_number", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_string", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_func", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_list", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_dict", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_float", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_bool", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"t_blob", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"event", VarType::VAR_DICT, VV_RO),
    make_vimvar(c"versionlong", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"echospace", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"argf", VarType::VAR_LIST, VV_RO),
    make_vimvar(c"argv", VarType::VAR_LIST, VV_RO),
    make_vimvar(c"collate", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"exiting", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"maxcol", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"stacktrace", VarType::VAR_LIST, VV_RO),
    make_vimvar(c"vim_did_init", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"stderr", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"msgpack_types", VarType::VAR_DICT, VV_RO),
    make_vimvar(c"_null_string", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"_null_list", VarType::VAR_LIST, VV_RO),
    make_vimvar(c"_null_dict", VarType::VAR_DICT, VV_RO),
    make_vimvar(c"_null_blob", VarType::VAR_BLOB, VV_RO),
    make_vimvar(c"lua", VarType::VAR_PARTIAL, VV_RO),
    make_vimvar(c"relnum", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"virtnum", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"starttime", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"exitreason", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"useractive", VarType::VAR_NUMBER, VV_RO),
    make_vimvar(c"startreason", VarType::VAR_STRING, VV_RO),
    make_vimvar(c"cascading", VarType::VAR_BOOL, VV_RO),
];

const MSGPACK_TYPE_NAMES: [&CStr; NUM_MSGPACK_TYPES] = [
    c"nil", c"boolean", c"integer", c"float", c"string", c"array", c"map", c"ext",
];

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut eval_msgpack_type_lists: [*const list_T; NUM_MSGPACK_TYPES] =
    [ptr::null(); NUM_MSGPACK_TYPES];

static mut varnamebuf: *mut c_char = ptr::null_mut();
static mut varnamebuflen: usize = 0;

static mut redir_lval: *mut lval_T = ptr::null_mut();
static mut redir_ga: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};
static mut redir_endp: *mut c_char = ptr::null_mut();
static mut redir_varname: *mut c_char = ptr::null_mut();

#[inline]
unsafe fn c_strcpy(mut dst: *mut c_char, mut src: *const c_char) {
    unsafe {
        loop {
            let c = *src;
            *dst = c;
            if c == NUL {
                break;
            }
            dst = dst.add(1);
            src = src.add(1);
        }
    }
}

#[inline]
unsafe fn c_memchr(s: *const c_char, c: u8, n: usize) -> *const c_char {
    unsafe {
        let p = s.cast::<u8>();
        for i in 0..n {
            if *p.add(i) == c {
                return s.add(i);
            }
        }
        ptr::null()
    }
}

#[inline]
unsafe fn tv_init(tv: *mut typval_T) {
    if !tv.is_null() {
        unsafe {
            ptr::write_bytes(tv, 0, 1);
        }
    }
}

#[inline]
unsafe fn tv_list_locked(l: *const list_T) -> VarLockStatus {
    if l.is_null() {
        VAR_FIXED
    } else {
        unsafe { (*l).lv_lock }
    }
}

#[inline]
unsafe fn tv_dict_set_ret(tv: *mut typval_T, d: *mut dict_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval.v_dict = d;
        if !d.is_null() {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn script_sv(id: scid_T) -> *mut scriptvar_T {
    unsafe { (*script_item(id)).sn_vars.cast::<scriptvar_T>() }
}

#[inline]
unsafe fn script_vars_ht(id: scid_T) -> *mut hashtab_T {
    unsafe { &raw mut (*script_sv(id)).sv_dict.dv_hashtab }
}

#[inline]
fn tristate_from_int(val: OptInt) -> TriState {
    if val == 0 {
        kFalse
    } else if val >= 1 {
        kTrue
    } else {
        kNone
    }
}

#[inline]
fn integer_obj(i: OptInt) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn evalvars_init() {
    unsafe {
        init_var_dict(get_globvar_dict(), &raw mut globvars_var, VAR_DEF_SCOPE);
        init_var_dict(&raw mut vimvardict, &raw mut vimvars_var, VAR_SCOPE);
        vimvardict.dv_lock = VAR_FIXED;
        hash_init(&raw mut compat_hashtab);

        let vimvars_ptr = (&raw mut vimvars).cast::<VimVar>();
        for i in 0..VV_LEN {
            let p = vimvars_ptr.add(i);
            if ((*p).vv_flags & VV_RO) != 0 {
                (*p).vv_di.di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
            } else if ((*p).vv_flags & VV_RO_SBX) != 0 {
                (*p).vv_di.di_flags = DI_FLAGS_RO_SBX | DI_FLAGS_FIX;
            } else {
                (*p).vv_di.di_flags = DI_FLAGS_FIX;
            }

            let key_ptr = (&raw mut (*p).vv_di.di_key).cast::<c_char>();
            if (*p).vv_di.di_tv.v_type != VarType::VAR_UNKNOWN {
                hash_add(&raw mut vimvardict.dv_hashtab, key_ptr);
            }
            if ((*p).vv_flags & VV_COMPAT) != 0 {
                hash_add(&raw mut compat_hashtab, key_ptr);
            }
        }
        let vim_version = varnumber_T::from(min_vim_version());
        set_vim_var_nr(VV_VERSION, vim_version);
        set_vim_var_nr(
            VV_VERSIONLONG,
            vim_version * 10000 + varnumber_T::from(highest_patch()),
        );

        let msgpack_types_dict = tv_dict_alloc();
        for (i, type_name) in MSGPACK_TYPE_NAMES.iter().enumerate() {
            let type_list = tv_list_alloc(0);
            tv_list_set_lock(type_list, VAR_FIXED);
            tv_list_ref(type_list);
            let di = tv_dict_item_alloc(type_name.as_ptr());
            (*di).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            (*di).di_tv = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: 0,
                vval: TypvalVval { v_list: type_list },
            };
            *(&raw mut eval_msgpack_type_lists)
                .cast::<*const list_T>()
                .add(i) = type_list;
            if tv_dict_add(msgpack_types_dict, di) == FAIL {
                std::process::abort();
            }
        }
        (*msgpack_types_dict).dv_lock = VAR_FIXED;

        set_vim_var_dict(VV_MSGPACK_TYPES, msgpack_types_dict);
        set_vim_var_dict(VV_COMPLETED_ITEM, tv_dict_alloc_lock(VAR_FIXED));

        set_vim_var_dict(VV_EVENT, tv_dict_alloc_lock(VAR_FIXED));
        set_vim_var_list(VV_ERRORS, tv_list_alloc(kListLenUnknown));
        set_vim_var_nr(VV_STDERR, CHAN_STDERR);
        set_vim_var_nr(VV_SEARCHFORWARD, 1);
        set_vim_var_nr(VV_HLSEARCH, 1);
        set_vim_var_nr(VV_COUNT1, 1);
        set_vim_var_string(VV_STARTREASON, c"normal".as_ptr(), 6);
        set_vim_var_special(VV_EXITING, kSpecialVarNull);

        set_vim_var_nr(VV_TYPE_NUMBER, VAR_TYPE_NUMBER);
        set_vim_var_nr(VV_TYPE_STRING, VAR_TYPE_STRING);
        set_vim_var_nr(VV_TYPE_FUNC, VAR_TYPE_FUNC);
        set_vim_var_nr(VV_TYPE_LIST, VAR_TYPE_LIST);
        set_vim_var_nr(VV_TYPE_DICT, VAR_TYPE_DICT);
        set_vim_var_nr(VV_TYPE_FLOAT, VAR_TYPE_FLOAT);
        set_vim_var_nr(VV_TYPE_BOOL, VAR_TYPE_BOOL);
        set_vim_var_nr(VV_TYPE_BLOB, VAR_TYPE_BLOB);

        set_vim_var_bool(VV_FALSE, kBoolVarFalse);
        set_vim_var_bool(VV_TRUE, kBoolVarTrue);
        set_vim_var_special(VV_NULL, kSpecialVarNull);
        set_vim_var_nr(VV_NUMBERMAX, varnumber_T::MAX);
        set_vim_var_nr(VV_NUMBERMIN, varnumber_T::MIN);
        set_vim_var_nr(
            VV_NUMBERSIZE,
            (std::mem::size_of::<varnumber_T>() * 8) as varnumber_T,
        );
        set_vim_var_nr(VV_MAXCOL, MAXCOL);

        set_vim_var_nr(VV_ECHOSPACE, varnumber_T::from(sc_col - 1));

        let vvlua_partial = xcalloc(1, std::mem::size_of::<partial_T>()).cast::<partial_T>();
        (*vvlua_partial).pt_name = xmallocz(0).cast::<c_char>();
        (*vvlua_partial).pt_refcount += 1;
        set_vim_var_partial(VV_LUA, vvlua_partial);

        set_reg_var(0);

        let startreason = os_getenv_noalloc(ENV_STARTREASON.as_ptr());
        if strequal(startreason, c"restart!".as_ptr()) || strequal(startreason, c"restart".as_ptr())
        {
            set_vim_var_string(VV_STARTREASON, startreason, -1);
        }
        if os_env_exists(ENV_STARTREASON.as_ptr(), false) {
            os_unsetenv(ENV_STARTREASON.as_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn evalvars_clear() {
    unsafe {
        let vimvars_ptr = (&raw mut vimvars).cast::<VimVar>();
        for i in 0..VV_LEN {
            let p = vimvars_ptr.add(i);
            if (*p).vv_di.di_tv.v_type == VarType::VAR_STRING {
                xfree((*p).vv_di.di_tv.vval.v_string.cast());
                (*p).vv_di.di_tv.vval.v_string = ptr::null_mut();
            } else if (*p).vv_di.di_tv.v_type == VarType::VAR_LIST {
                tv_list_unref((*p).vv_di.di_tv.vval.v_list);
                (*p).vv_di.di_tv.vval.v_list = ptr::null_mut();
            }
        }

        partial_unref(get_vim_var_partial(VV_LUA));
        set_vim_var_partial(VV_LUA, ptr::null_mut());
        hash_clear(&raw mut vimvardict.dv_hashtab);
        hash_init(&raw mut vimvardict.dv_hashtab);
        hash_clear(&raw mut compat_hashtab);

        vars_clear(get_globvar_ht());

        for i in 1..=script_items.ga_len {
            vars_clear(script_vars_ht(i));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn garbage_collect_globvars(copyID: c_int) -> c_int {
    unsafe {
        c_int::from(set_ref_in_ht(
            &raw mut globvardict.dv_hashtab,
            copyID,
            ptr::null_mut(),
        ))
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn garbage_collect_vimvars(copyID: c_int) -> bool {
    unsafe { set_ref_in_ht(&raw mut vimvardict.dv_hashtab, copyID, ptr::null_mut()) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn garbage_collect_scriptvars(copyID: c_int) -> bool {
    unsafe {
        let mut abort = false;

        for i in 1..=script_items.ga_len {
            abort = abort || set_ref_in_ht(script_vars_ht(i), copyID, ptr::null_mut());
        }

        abort
    }
}

/// Set an internal variable to a string value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_internal_string_var(name: *const c_char, value: *mut c_char) {
    unsafe {
        let mut tv = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval { v_string: value },
        };

        set_var(name, c_strlen(name), &raw mut tv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_charconvert(
    enc_from: *const c_char,
    enc_to: *const c_char,
    fname_from: *const c_char,
    fname_to: *const c_char,
) -> c_int {
    unsafe {
        let saved_sctx = current_sctx;

        set_vim_var_string(VV_CC_FROM, enc_from, -1);
        set_vim_var_string(VV_CC_TO, enc_to, -1);
        set_vim_var_string(VV_FNAME_IN, fname_from, -1);
        set_vim_var_string(VV_FNAME_OUT, fname_to, -1);
        let ctx = get_option_sctx(kOptCharconvert);
        if !ctx.is_null() {
            current_sctx = *ctx;
        }

        let mut err = false;
        let mut tv: typval_T = std::mem::zeroed();
        let mut argv: [typval_T; 1] = [std::mem::zeroed()];
        if !callback_call(&raw mut p_ccv, 0, argv.as_mut_ptr(), &raw mut tv) {
            err = true;
        } else {
            if tv_get_number_chk(&raw const tv, ptr::null_mut()) != 0 {
                err = true;
            }
            tv_clear(&raw mut tv);
        }

        set_vim_var_string(VV_CC_FROM, ptr::null(), -1);
        set_vim_var_string(VV_CC_TO, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        current_sctx = saved_sctx;

        if err { FAIL } else { OK }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_diff(
    origfile: *const c_char,
    newfile: *const c_char,
    outfile: *const c_char,
) {
    unsafe {
        let saved_sctx = current_sctx;
        set_vim_var_string(VV_FNAME_IN, origfile, -1);
        set_vim_var_string(VV_FNAME_NEW, newfile, -1);
        set_vim_var_string(VV_FNAME_OUT, outfile, -1);

        let ctx = get_option_sctx(kOptDiffexpr);
        if !ctx.is_null() {
            current_sctx = *ctx;
        }

        let mut tv: typval_T = std::mem::zeroed();
        let mut argv: [typval_T; 1] = [std::mem::zeroed()];
        if callback_call(&raw mut p_dex, 0, argv.as_mut_ptr(), &raw mut tv) {
            tv_clear(&raw mut tv);
        }

        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_NEW, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        current_sctx = saved_sctx;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_patch(
    origfile: *const c_char,
    difffile: *const c_char,
    outfile: *const c_char,
) {
    unsafe {
        let saved_sctx = current_sctx;
        set_vim_var_string(VV_FNAME_IN, origfile, -1);
        set_vim_var_string(VV_FNAME_DIFF, difffile, -1);
        set_vim_var_string(VV_FNAME_OUT, outfile, -1);

        let ctx = get_option_sctx(kOptPatchexpr);
        if !ctx.is_null() {
            current_sctx = *ctx;
        }

        let mut tv: typval_T = std::mem::zeroed();
        let mut argv: [typval_T; 1] = [std::mem::zeroed()];
        if callback_call(&raw mut p_pex, 0, argv.as_mut_ptr(), &raw mut tv) {
            tv_clear(&raw mut tv);
        }

        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_DIFF, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        current_sctx = saved_sctx;
    }
}

/// Evaluate an expression to a list with suggestions.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_spell_expr(badword: *mut c_char, expr: *mut c_char) -> *mut list_T {
    unsafe {
        let mut save_val: typval_T = std::mem::zeroed();
        let mut rettv: typval_T = std::mem::zeroed();
        let mut list: *mut list_T = ptr::null_mut();
        let mut p = skipwhite(expr);
        let saved_sctx = current_sctx;

        prepare_vimvar(VV_VAL, &raw mut save_val);
        set_vim_var_string(VV_VAL, badword, -1);
        if p_verbose == 0 {
            emsg_off += 1;
        }
        let ctx = get_option_sctx(kOptSpellsuggest);
        if !ctx.is_null() {
            current_sctx = *ctx;
        }

        let mut r = may_call_simple_func(p, &raw mut rettv);
        if r == NOTDONE {
            r = eval1(&raw mut p, &raw mut rettv, &raw mut EVALARG_EVALUATE);
        }
        if r == OK {
            if rettv.v_type != VarType::VAR_LIST {
                tv_clear(&raw mut rettv);
            } else {
                list = rettv.vval.v_list;
            }
        }

        if p_verbose == 0 {
            emsg_off -= 1;
        }
        tv_clear(get_vim_var_tv(VV_VAL));
        restore_vimvar(VV_VAL, &raw mut save_val);
        current_sctx = saved_sctx;

        list
    }
}

/// Get spell word from an entry from spellsuggest=expr:
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_spellword(list: *mut list_T, ret_word: *mut *const c_char) -> c_int {
    unsafe {
        if tv_list_len(list) != 2 {
            emsg(gettext(
                c"E5700: Expression from 'spellsuggest' must yield lists with exactly two values"
                    .as_ptr(),
            ));
            return -1;
        }
        *ret_word = tv_list_find_str(list, 0);
        if (*ret_word).is_null() {
            return -1;
        }
        tv_list_find_nr(list, -1, ptr::null_mut()) as c_int
    }
}

/// Prepare v: variable "idx" to be used.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prepare_vimvar(idx: c_int, save_tv: *mut typval_T) {
    unsafe {
        let p = (&raw mut vimvars).cast::<VimVar>().add(idx as usize);
        *save_tv = (*p).vv_di.di_tv;
        (*p).vv_di.di_tv.vval.v_string = ptr::null_mut();
        if (*p).vv_di.di_tv.v_type == VarType::VAR_UNKNOWN {
            hash_add(
                &raw mut vimvardict.dv_hashtab,
                (&raw mut (*p).vv_di.di_key).cast::<c_char>(),
            );
        }
    }
}

/// Restore v: variable "idx" to typeval "save_tv".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn restore_vimvar(idx: c_int, save_tv: *mut typval_T) {
    unsafe {
        let p = (&raw mut vimvars).cast::<VimVar>().add(idx as usize);
        (*p).vv_di.di_tv = *save_tv;
        if (*p).vv_di.di_tv.v_type != VarType::VAR_UNKNOWN {
            return;
        }

        let hi = hash_find(
            &raw const vimvardict.dv_hashtab,
            (&raw const (*p).vv_di.di_key).cast::<c_char>(),
        );
        if hashitem_empty(&*hi) {
            internal_error(c"restore_vimvar()".as_ptr());
        } else {
            hash_remove(&raw mut vimvardict.dv_hashtab, hi);
        }
    }
}

/// List Vim variables.
unsafe fn list_vim_vars(first: *mut c_int) {
    unsafe {
        list_hashtable_vars(
            &raw mut vimvardict.dv_hashtab,
            c"v:".as_ptr(),
            false as c_int,
            first,
        );
    }
}

/// List script-local variables, if there is a script.
unsafe fn list_script_vars(first: *mut c_int) {
    unsafe {
        if current_sctx.sc_sid > 0 && current_sctx.sc_sid <= script_items.ga_len {
            list_hashtable_vars(
                script_vars_ht(current_sctx.sc_sid),
                c"s:".as_ptr(),
                false as c_int,
                first,
            );
        }
    }
}

/// Evaluate one Vim expression {expr} in string "p" and append the resulting string to "gap".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_one_expr_in_str(
    p: *mut c_char,
    gap: *mut garray_T,
    evaluate: bool,
) -> *mut c_char {
    unsafe {
        let block_start = skipwhite(p.add(1));
        let mut block_end = block_start;

        if *block_start == NUL {
            semsg!(gettext(e_missing_close_curly_str.as_ptr()), p);
            return ptr::null_mut();
        }
        if skip_expr(&raw mut block_end, ptr::null_mut()) == FAIL {
            return ptr::null_mut();
        }
        block_end = skipwhite(block_end);
        if *block_end != b'}' as c_char {
            semsg!(gettext(e_missing_close_curly_str.as_ptr()), p);
            return ptr::null_mut();
        }
        if evaluate {
            *block_end = NUL;
            let expr_val = eval_to_string(block_start, false, false);
            *block_end = b'}' as c_char;
            if expr_val.is_null() {
                return ptr::null_mut();
            }
            ga_concat(gap, expr_val);
            xfree(expr_val.cast());
        }

        block_end.add(1)
    }
}

/// Evaluate all the Vim expressions {expr} in "str" and return the resulting string in allocated memory.
unsafe fn eval_all_expr_in_str(str_arg: *mut c_char) -> *mut c_char {
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, 1, 80);
        let mut p = str_arg;

        while *p != NUL {
            let mut escaped_brace = false;

            let lit_start = p;
            while *p != b'{' as c_char && *p != b'}' as c_char && *p != NUL {
                p = p.add(1);
            }

            if *p != NUL && *p == *p.add(1) {
                p = p.add(1);
                escaped_brace = true;
            } else if *p == b'}' as c_char {
                semsg!(gettext(e_stray_closing_curly_str.as_ptr()), str_arg);
                ga_clear(&raw mut ga);
                return ptr::null_mut();
            }

            ga_concat_len(&raw mut ga, lit_start, p.offset_from(lit_start) as usize);

            if *p == NUL {
                break;
            }

            if escaped_brace {
                p = p.add(1);
                continue;
            }

            p = eval_one_expr_in_str(p, &raw mut ga, true);
            if p.is_null() {
                ga_clear(&raw mut ga);
                return ptr::null_mut();
            }
        }
        ga_append(&raw mut ga, 0);

        ga.ga_data.cast::<c_char>()
    }
}

/// Get a list of lines from a HERE document.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn heredoc_get(
    eap: *mut exarg_T,
    mut cmd: *mut c_char,
    script_get: bool,
) -> *mut list_T {
    unsafe {
        let mut marker_indent_len: c_int = 0;
        let mut text_indent_len: c_int = 0;
        let mut text_indent: *mut c_char = ptr::null_mut();
        let mut dot: [c_char; 2] = [b'.' as c_char, NUL];
        let mut heredoc_in_string = false;
        let mut line_arg: *mut c_char = ptr::null_mut();
        let nl_ptr = vim_strchr(cmd, b'\n' as c_int);

        if !nl_ptr.is_null() {
            heredoc_in_string = true;
            line_arg = nl_ptr.add(1);
            *nl_ptr = NUL;
        } else if (*eap).ea_getline.is_none() {
            emsg(gettext(E_CANNOT_USE_HEREDOC_HERE.as_ptr()));
            return ptr::null_mut();
        }

        cmd = skipwhite(cmd);
        let mut evalstr = false;
        let mut eval_failed = false;
        loop {
            if c_strncmp(cmd, c"trim".as_ptr(), 4) == 0
                && (*cmd.add(4) == NUL || ascii_iswhite(*cmd.add(4) as c_int))
            {
                cmd = skipwhite(cmd.add(4));

                let mut p = *(*eap).cmdlinep;
                while ascii_iswhite(*p as c_int) {
                    p = p.add(1);
                    marker_indent_len += 1;
                }
                text_indent_len = -1;
                continue;
            }
            if c_strncmp(cmd, c"eval".as_ptr(), 4) == 0
                && (*cmd.add(4) == NUL || ascii_iswhite(*cmd.add(4) as c_int))
            {
                cmd = skipwhite(cmd.add(4));
                evalstr = true;
                continue;
            }
            break;
        }

        let comment_char = b'"' as c_char;
        let marker: *mut c_char;
        if *cmd != NUL && *cmd != comment_char {
            marker = skipwhite(cmd);
            let p = skiptowhite(marker);
            if *skipwhite(p) != NUL && *skipwhite(p) != comment_char {
                semsg!(gettext(e_trailing_arg.as_ptr()), p);
                return ptr::null_mut();
            }
            *p = NUL;
            if !script_get && (*marker as u8).is_ascii_lowercase() {
                emsg(gettext(
                    c"E221: Marker cannot start with lower case letter".as_ptr(),
                ));
                return ptr::null_mut();
            }
        } else if script_get {
            marker = dot.as_mut_ptr();
        } else {
            emsg(gettext(c"E172: Missing marker".as_ptr()));
            return ptr::null_mut();
        }

        let mut theline: *mut c_char = ptr::null_mut();
        let l = tv_list_alloc(0);
        loop {
            let mut mi: c_int = 0;
            let mut ti: c_int = 0;

            if heredoc_in_string {
                if *line_arg == NUL {
                    if !script_get {
                        semsg!(gettext(E_MISSING_END_MARKER_STR.as_ptr()), marker);
                    }
                    break;
                }

                theline = line_arg;
                let next_line = vim_strchr(theline, b'\n' as c_int);
                if next_line.is_null() {
                    line_arg = line_arg.add(c_strlen(line_arg));
                } else {
                    *next_line = NUL;
                    line_arg = next_line.add(1);
                }
            } else {
                xfree(theline.cast());
                theline = if let Some(getline_fn) = (*eap).ea_getline {
                    getline_fn(0, (*eap).cookie, 0, false)
                } else {
                    ptr::null_mut()
                };
                if theline.is_null() {
                    if !script_get {
                        semsg!(gettext(E_MISSING_END_MARKER_STR.as_ptr()), marker);
                    }
                    break;
                }
            }

            if marker_indent_len > 0
                && c_strncmp(theline, *(*eap).cmdlinep, marker_indent_len as usize) == 0
            {
                mi = marker_indent_len;
            }
            if c_strcmp(marker, theline.add(mi as usize)) == 0 {
                break;
            }

            if eval_failed {
                continue;
            }

            if text_indent_len == -1 && *theline != NUL {
                let mut p = theline;
                text_indent_len = 0;
                while ascii_iswhite(*p as c_int) {
                    p = p.add(1);
                    text_indent_len += 1;
                }
                text_indent = xmemdupz(theline.cast(), text_indent_len as usize).cast::<c_char>();
            }
            if !text_indent.is_null() {
                while ti < text_indent_len {
                    if *theline.add(ti as usize) != *text_indent.add(ti as usize) {
                        break;
                    }
                    ti += 1;
                }
            }

            let mut str_line = theline.add(ti as usize);
            if evalstr && (*eap).skip == 0 {
                str_line = eval_all_expr_in_str(str_line);
                if str_line.is_null() {
                    eval_failed = true;
                    continue;
                }
                tv_list_append_allocated_string(l, str_line);
            } else {
                tv_list_append_string(l, str_line, -1);
            }
        }
        if heredoc_in_string {
            (*eap).nextcmd = line_arg;
        } else {
            xfree(theline.cast());
        }
        xfree(text_indent.cast());

        if eval_failed {
            tv_list_free(l);
            return ptr::null_mut();
        }
        l
    }
}

/// ":let" and ":const" commands.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_let(eap: *mut exarg_T) {
    unsafe {
        let is_const = (*eap).cmdidx == cmdidx_T::CMD_const as c_int;
        let mut arg = (*eap).arg;
        let mut rettv: typval_T = std::mem::zeroed();
        let mut var_count: c_int = 0;
        let mut semicolon: c_int = 0;
        let mut op: [c_char; 2] = [0; 2];
        let mut first: c_int = 1;

        let argend = skip_var_list(arg, &raw mut var_count, &raw mut semicolon, false);
        if argend.is_null() {
            return;
        }
        let mut expr = skipwhite(argend);
        let concat = c_strncmp(expr, c"..=".as_ptr(), 3) == 0;
        let has_assign = *expr == b'=' as c_char
            || (!vim_strchr(c"+-*/%.".as_ptr(), *expr as u8 as c_int).is_null()
                && *expr.add(1) == b'=' as c_char);
        if !has_assign && !concat {
            if *arg == b'[' as c_char {
                emsg(gettext(e_invarg.as_ptr()));
            } else if ends_excmd(*arg as c_int) == 0 {
                arg = list_arg_vars(eap, arg, &raw mut first).cast_mut();
            } else if (*eap).skip == 0 {
                list_glob_vars(&raw mut first);
                list_buf_vars(&raw mut first);
                list_win_vars(&raw mut first);
                list_tab_vars(&raw mut first);
                list_script_vars(&raw mut first);
                list_func_vars(&raw mut first);
                list_vim_vars(&raw mut first);
            }
            (*eap).nextcmd = check_nextcmd(arg);
            return;
        }

        if *expr == b'=' as c_char
            && *expr.add(1) == b'<' as c_char
            && *expr.add(2) == b'<' as c_char
        {
            let l = heredoc_get(eap, expr.add(3), false);
            if !l.is_null() {
                tv_list_set_ret(&raw mut rettv, l);
                if (*eap).skip == 0 {
                    op[0] = b'=' as c_char;
                    op[1] = NUL;
                    ex_let_vars(
                        (*eap).arg,
                        &raw mut rettv,
                        0,
                        semicolon,
                        var_count,
                        c_int::from(is_const),
                        op.as_mut_ptr(),
                    );
                }
                tv_clear(&raw mut rettv);
            }
            return;
        }

        rettv.v_type = VarType::VAR_UNKNOWN;

        op[0] = b'=' as c_char;
        op[1] = NUL;
        if *expr != b'=' as c_char {
            if !vim_strchr(c"+-*/%.".as_ptr(), *expr as u8 as c_int).is_null() {
                op[0] = *expr;
                if *expr == b'.' as c_char && *expr.add(1) == b'.' as c_char {
                    expr = expr.add(1);
                }
            }
            expr = expr.add(2);
        } else {
            expr = expr.add(1);
        }

        expr = skipwhite(expr);

        if (*eap).skip != 0 {
            emsg_skip += 1;
        }
        let mut evalarg: evalarg_T = std::mem::zeroed();
        fill_evalarg_from_eap(&raw mut evalarg, eap, (*eap).skip != 0);
        let eval_res = eval0(expr, &raw mut rettv, eap, &raw mut evalarg);
        if (*eap).skip != 0 {
            emsg_skip -= 1;
        }
        clear_evalarg(&raw mut evalarg, eap);

        if (*eap).skip == 0 && eval_res != FAIL {
            ex_let_vars(
                (*eap).arg,
                &raw mut rettv,
                0,
                semicolon,
                var_count,
                c_int::from(is_const),
                op.as_mut_ptr(),
            );
        }
        if eval_res != FAIL {
            tv_clear(&raw mut rettv);
        }
    }
}

/// Assign the typevalue "tv" to the variable or variables at "arg_start".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_let_vars(
    arg_start: *mut c_char,
    tv: *mut typval_T,
    copy: c_int,
    semicolon: c_int,
    var_count: c_int,
    is_const: c_int,
    op: *mut c_char,
) -> c_int {
    unsafe {
        let mut arg = arg_start;
        let mut ltv: typval_T = std::mem::zeroed();

        if *arg != b'[' as c_char {
            if ex_let_one(arg, tv, copy != 0, is_const != 0, op, op).is_null() {
                return FAIL;
            }
            return OK;
        }

        if (*tv).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq.as_ptr()));
            return FAIL;
        }
        let l = (*tv).vval.v_list;

        let len = tv_list_len(l);
        if semicolon == 0 && var_count < len {
            emsg(gettext(c"E687: Less targets than List items".as_ptr()));
            return FAIL;
        }
        if var_count - semicolon > len {
            emsg(gettext(c"E688: More targets than List items".as_ptr()));
            return FAIL;
        }
        debug_assert!(!l.is_null());

        let mut item = tv_list_first(l);
        let mut rest_len = tv_list_len(l) as usize;
        while *arg != b']' as c_char {
            arg = skipwhite(arg.add(1));
            arg = ex_let_one(
                arg,
                &raw mut (*item).li_tv,
                true,
                is_const != 0,
                c",;]".as_ptr(),
                op,
            );
            if arg.is_null() {
                return FAIL;
            }
            rest_len -= 1;

            item = (*item).li_next;
            arg = skipwhite(arg);
            if *arg == b';' as c_char {
                let rest_list = tv_list_alloc(rest_len as isize);
                while !item.is_null() {
                    tv_list_append_tv(rest_list, &raw mut (*item).li_tv);
                    item = (*item).li_next;
                }

                ltv.v_type = VarType::VAR_LIST;
                ltv.v_lock = VAR_UNLOCKED;
                ltv.vval.v_list = rest_list;
                tv_list_ref(rest_list);

                arg = ex_let_one(
                    skipwhite(arg.add(1)),
                    &raw mut ltv,
                    false,
                    is_const != 0,
                    c"]".as_ptr(),
                    op,
                );
                tv_clear(&raw mut ltv);
                if arg.is_null() {
                    return FAIL;
                }
                break;
            } else if *arg != b',' as c_char && *arg != b']' as c_char {
                internal_error(c"ex_let_vars()".as_ptr());
                return FAIL;
            }
        }

        OK
    }
}

/// Skip over assignable variable "var" or list of variables "[var, var]".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn skip_var_list(
    arg: *const c_char,
    var_count: *mut c_int,
    semicolon: *mut c_int,
    silent: bool,
) -> *const c_char {
    unsafe {
        if *arg == b'[' as c_char {
            let mut p = arg;
            loop {
                p = skipwhite(p.add(1));
                let s = skip_var_one(p);
                if s == p {
                    if !silent {
                        semsg!(gettext(e_invarg2.as_ptr()), p);
                    }
                    return ptr::null();
                }
                *var_count += 1;

                p = skipwhite(s);
                if *p == b']' as c_char {
                    break;
                } else if *p == b';' as c_char {
                    if *semicolon == 1 {
                        if !silent {
                            emsg(gettext(E_DOUBLE_SEMICOLON_IN_LIST_OF_VARIABLES.as_ptr()));
                        }
                        return ptr::null();
                    }
                    *semicolon = 1;
                } else if *p != b',' as c_char {
                    if !silent {
                        semsg!(gettext(e_invarg2.as_ptr()), p);
                    }
                    return ptr::null();
                }
            }
            return p.add(1);
        }
        skip_var_one(arg)
    }
}

/// Skip one (assignable) variable name, including @r, $VAR, &option, d.key, l[idx].
unsafe fn skip_var_one(arg: *const c_char) -> *const c_char {
    unsafe {
        if *arg == b'@' as c_char && *arg.add(1) != NUL {
            return arg.add(2);
        }
        find_name_end(
            if *arg == b'$' as c_char || *arg == b'&' as c_char {
                arg.add(1)
            } else {
                arg
            },
            ptr::null_mut(),
            ptr::null_mut(),
            FNE_INCL_BR | FNE_CHECK_START,
        )
    }
}

/// List variables for hashtab "ht" with prefix "prefix".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn list_hashtable_vars(
    ht: *mut hashtab_T,
    prefix: *const c_char,
    empty: c_int,
    first: *mut c_int,
) {
    unsafe {
        let mut todo = (*ht).ht_used as c_int;
        let mut hi = (*ht).ht_array;
        while todo > 0 && !got_int {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                let mut buf = [0 as c_char; IOSIZE];

                xstrlcpy(buf.as_mut_ptr(), prefix, IOSIZE);
                xstrlcat(buf.as_mut_ptr(), (*di).di_key.as_ptr(), IOSIZE);
                if message_filtered(buf.as_ptr()) {
                    hi = hi.add(1);
                    continue;
                }

                if empty != 0
                    || (*di).di_tv.v_type != VarType::VAR_STRING
                    || !(*di).di_tv.vval.v_string.is_null()
                {
                    list_one_var(di, prefix, first);
                }
            }
            hi = hi.add(1);
        }
    }
}

unsafe fn list_glob_vars(first: *mut c_int) {
    unsafe {
        list_hashtable_vars(
            &raw mut globvardict.dv_hashtab,
            c"".as_ptr(),
            true as c_int,
            first,
        );
    }
}

unsafe fn list_buf_vars(first: *mut c_int) {
    unsafe {
        list_hashtable_vars(
            &raw mut (*(*curbuf).b_vars).dv_hashtab,
            c"b:".as_ptr(),
            true as c_int,
            first,
        );
    }
}

unsafe fn list_win_vars(first: *mut c_int) {
    unsafe {
        list_hashtable_vars(
            &raw mut (*(*curwin).w_vars).dv_hashtab,
            c"w:".as_ptr(),
            true as c_int,
            first,
        );
    }
}

unsafe fn list_tab_vars(first: *mut c_int) {
    unsafe {
        list_hashtable_vars(
            &raw mut (*(*curtab).tp_vars).dv_hashtab,
            c"t:".as_ptr(),
            true as c_int,
            first,
        );
    }
}

/// List variables in "arg".
unsafe fn list_arg_vars(
    eap: *mut exarg_T,
    mut arg: *const c_char,
    first: *mut c_int,
) -> *const c_char {
    unsafe {
        let mut error = false;
        let mut tv: typval_T = std::mem::zeroed();

        while ends_excmd(*arg as c_int) == 0 && !got_int {
            if error || (*eap).skip != 0 {
                arg = find_name_end(
                    arg,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    FNE_INCL_BR | FNE_CHECK_START,
                );
                if !ascii_iswhite(*arg as c_int) && ends_excmd(*arg as c_int) == 0 {
                    emsg_severe = true;
                    semsg!(gettext(e_trailing_arg.as_ptr()), arg);
                    break;
                }
            } else {
                let name_start = arg;
                let mut name = arg;
                let mut tofree: *mut c_char = ptr::null_mut();
                let len = get_name_len(&raw mut arg, &raw mut tofree, true, true);
                if len <= 0 {
                    if len < 0 && !aborting() {
                        emsg_severe = true;
                        semsg!(gettext(e_invarg2.as_ptr()), arg);
                        xfree(tofree.cast());
                        break;
                    }
                    error = true;
                } else {
                    if !tofree.is_null() {
                        name = tofree;
                    }
                    if eval_variable(name, len, &raw mut tv, ptr::null_mut(), true, false) == FAIL {
                        error = true;
                    } else {
                        let arg_subsc = arg;
                        if handle_subscript(
                            &raw mut arg,
                            &raw mut tv,
                            &raw mut EVALARG_EVALUATE,
                            true,
                        ) == FAIL
                        {
                            error = true;
                        } else {
                            if arg == arg_subsc && len == 2 && *name.add(1) == b':' as c_char {
                                match *name as u8 {
                                    b'g' => list_glob_vars(first),
                                    b'b' => list_buf_vars(first),
                                    b'w' => list_win_vars(first),
                                    b't' => list_tab_vars(first),
                                    b'v' => list_vim_vars(first),
                                    b's' => list_script_vars(first),
                                    b'l' => list_func_vars(first),
                                    _ => {
                                        semsg!(
                                            gettext(c"E738: Can't list variables for %s".as_ptr()),
                                            name,
                                        );
                                    }
                                }
                            } else {
                                let s = encode_tv2echo(&raw mut tv, ptr::null_mut());
                                let used_name = if arg == arg_subsc { name } else { name_start };
                                debug_assert!(!used_name.is_null());
                                let name_size = if used_name == tofree.cast_const() {
                                    c_strlen(used_name) as isize
                                } else {
                                    arg.offset_from(used_name)
                                };
                                list_one_var_a(
                                    c"".as_ptr(),
                                    used_name,
                                    name_size,
                                    tv.v_type,
                                    if s.is_null() { c"".as_ptr() } else { s },
                                    first,
                                );
                                xfree(s.cast());
                            }
                            tv_clear(&raw mut tv);
                        }
                    }
                }

                xfree(tofree.cast());
            }

            arg = skipwhite(arg);
        }

        arg
    }
}

/// Set an environment variable, part of ex_let_one().
unsafe fn ex_let_env(
    mut arg: *mut c_char,
    tv: *mut typval_T,
    is_const: bool,
    endchars: *const c_char,
    op: *const c_char,
) -> *mut c_char {
    unsafe {
        if is_const {
            emsg(gettext(
                c"E996: Cannot lock an environment variable".as_ptr(),
            ));
            return ptr::null_mut();
        }

        let mut arg_end: *mut c_char = ptr::null_mut();
        arg = arg.add(1);
        let name = arg;
        let mut arg_const: *const c_char = arg;
        let len = get_env_len(&raw mut arg_const) as usize;
        arg = arg_const.cast_mut();
        if len == 0 {
            semsg!(gettext(e_invarg2.as_ptr()), name.sub(1));
        } else if !op.is_null() && !vim_strchr(c"+-*/%".as_ptr(), *op as u8 as c_int).is_null() {
            semsg!(gettext(e_letwrong.as_ptr()), op);
        } else if !endchars.is_null()
            && vim_strchr(endchars, *skipwhite(arg) as u8 as c_int).is_null()
        {
            emsg(gettext(E_LETUNEXP.as_ptr()));
        } else if !check_secure() {
            let mut tofree: *mut c_char = ptr::null_mut();
            let c1 = *name.add(len);
            *name.add(len) = NUL;
            let mut p = tv_get_string_chk(tv);
            if !p.is_null() && !op.is_null() && *op == b'.' as c_char {
                let s = vim_getenv(name);
                if !s.is_null() {
                    tofree = concat_str(s, p);
                    p = tofree;
                    xfree(s.cast());
                }
            }
            if !p.is_null() {
                vim_setenv_ext(name, p);
                arg_end = arg;
            }
            *name.add(len) = c1;
            xfree(tofree.cast());
        }
        arg_end
    }
}

/// Set an option, part of ex_let_one().
unsafe fn ex_let_option(
    arg: *mut c_char,
    tv: *mut typval_T,
    is_const: bool,
    endchars: *const c_char,
    op: *const c_char,
) -> *mut c_char {
    unsafe {
        if is_const {
            emsg(gettext(c"E996: Cannot lock an option".as_ptr()));
            return ptr::null_mut();
        }

        let mut arg_end: *mut c_char = ptr::null_mut();
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;

        let mut arg_const: *const c_char = arg;
        let p = find_option_var_end(&raw mut arg_const, &raw mut opt_idx, &raw mut opt_flags)
            .cast_mut();
        let arg_name = arg_const.cast_mut();

        if p.is_null()
            || (!endchars.is_null() && vim_strchr(endchars, *skipwhite(p) as u8 as c_int).is_null())
        {
            emsg(gettext(E_LETUNEXP.as_ptr()));
            return ptr::null_mut();
        }

        let c1 = *p;
        *p = NUL;

        let is_tty_opt = is_tty_option(arg_name);
        let hidden = is_option_hidden(opt_idx);
        let curval = if is_tty_opt {
            get_tty_option(arg_name)
        } else {
            get_option_value(opt_idx, opt_flags)
        };
        let mut newval = nil_obj();

        if curval.obj_type == ObjectType::kObjectTypeNil {
            semsg!(gettext(e_unknown_option2.as_ptr()), arg_name);
            *p = c1;
            optval_free(curval);
            optval_free(newval);
            return arg_end;
        }
        if !op.is_null()
            && *op != b'=' as c_char
            && ((curval.obj_type != ObjectType::kObjectTypeString && *op == b'.' as c_char)
                || (curval.obj_type == ObjectType::kObjectTypeString && *op != b'.' as c_char))
        {
            semsg!(gettext(e_letwrong.as_ptr()), op);
            *p = c1;
            optval_free(curval);
            optval_free(newval);
            return arg_end;
        }

        let mut error = false;
        newval = opt_from_tv(tv, opt_idx, arg_name, &raw mut error);
        if error {
            *p = c1;
            optval_free(curval);
            optval_free(newval);
            return arg_end;
        }

        let is_num = (curval.obj_type == ObjectType::kObjectTypeInteger
            || curval.obj_type == ObjectType::kObjectTypeBoolean)
            && (newval.obj_type == ObjectType::kObjectTypeInteger
                || newval.obj_type == ObjectType::kObjectTypeBoolean);
        let is_string = curval.obj_type == ObjectType::kObjectTypeString
            && newval.obj_type == ObjectType::kObjectTypeString;

        if !op.is_null() && *op != b'=' as c_char {
            if !hidden && is_num {
                let cur_n: OptInt = if curval.obj_type == ObjectType::kObjectTypeInteger {
                    curval.data.integer
                } else {
                    OptInt::from(curval.data.boolean)
                };
                let mut new_n: OptInt = if newval.obj_type == ObjectType::kObjectTypeInteger {
                    newval.data.integer
                } else {
                    OptInt::from(newval.data.boolean)
                };

                match *op as u8 {
                    b'+' => new_n = cur_n.wrapping_add(new_n),
                    b'-' => new_n = cur_n.wrapping_sub(new_n),
                    b'*' => new_n = cur_n.wrapping_mul(new_n),
                    b'/' => new_n = num_divide(cur_n, new_n),
                    b'%' => new_n = num_modulus(cur_n, new_n),
                    _ => {}
                }

                if curval.obj_type == ObjectType::kObjectTypeInteger {
                    newval = integer_obj(new_n);
                } else {
                    newval = opt_from_tristate(tristate_from_int(new_n));
                }
            } else if !hidden && is_string {
                let curval_data = curval.data.string.data;
                let newval_data = newval.data.string.data;

                if !curval_data.is_null() && !newval_data.is_null() {
                    let newval_old = newval;
                    newval = string_obj(cstr_as_string(concat_str(curval_data, newval_data)));
                    optval_free(newval_old);
                }
            }
        }

        arg_end = p;
        if !is_tty_opt {
            set_option_value_give_err(opt_idx, newval, opt_flags);
        }

        *p = c1;
        optval_free(curval);
        optval_free(newval);
        arg_end
    }
}

/// Set a register, part of ex_let_one().
unsafe fn ex_let_register(
    mut arg: *mut c_char,
    tv: *mut typval_T,
    is_const: bool,
    endchars: *const c_char,
    op: *const c_char,
) -> *mut c_char {
    unsafe {
        if is_const {
            emsg(gettext(c"E996: Cannot lock a register".as_ptr()));
            return ptr::null_mut();
        }

        let mut arg_end: *mut c_char = ptr::null_mut();
        arg = arg.add(1);
        if !op.is_null() && !vim_strchr(c"+-*/%".as_ptr(), *op as u8 as c_int).is_null() {
            semsg!(gettext(e_letwrong.as_ptr()), op);
        } else if !endchars.is_null()
            && vim_strchr(endchars, *skipwhite(arg.add(1)) as u8 as c_int).is_null()
        {
            emsg(gettext(E_LETUNEXP.as_ptr()));
        } else {
            let mut ptofree: *mut c_char = ptr::null_mut();
            let mut p = tv_get_string_chk(tv);
            if !p.is_null() && !op.is_null() && *op == b'.' as c_char {
                let reg_c = if *arg == b'@' as c_char {
                    b'"' as c_int
                } else {
                    *arg as u8 as c_int
                };
                let s = get_reg_contents(reg_c, kGRegExprSrc).cast::<c_char>();
                if !s.is_null() {
                    ptofree = concat_str(s, p);
                    p = ptofree;
                    xfree(s.cast());
                }
            }
            if !p.is_null() {
                let reg_c = if *arg == b'@' as c_char {
                    b'"' as c_int
                } else {
                    *arg as u8 as c_int
                };
                write_reg_contents(reg_c, p, c_strlen(p) as isize, false as c_int);
                arg_end = arg.add(1);
            }
            xfree(ptofree.cast());
        }
        arg_end
    }
}

/// Set one item of `:let var = expr` or `:let [v1, v2] = list` to its value.
unsafe fn ex_let_one(
    arg: *mut c_char,
    tv: *mut typval_T,
    copy: bool,
    is_const: bool,
    endchars: *const c_char,
    op: *const c_char,
) -> *mut c_char {
    unsafe {
        let mut arg_end: *mut c_char = ptr::null_mut();

        if *arg == b'$' as c_char {
            return ex_let_env(arg, tv, is_const, endchars, op);
        } else if *arg == b'&' as c_char {
            return ex_let_option(arg, tv, is_const, endchars, op);
        } else if *arg == b'@' as c_char {
            return ex_let_register(arg, tv, is_const, endchars, op);
        } else if eval_isnamec1(*arg as u8 as c_int) || *arg == b'{' as c_char {
            let mut lv: lval_T = std::mem::zeroed();
            let p = get_lval(arg, tv, &raw mut lv, false, false, 0, FNE_CHECK_START);
            if !p.is_null() && !lv.ll_name.is_null() {
                if !endchars.is_null()
                    && vim_strchr(endchars, *skipwhite(p) as u8 as c_int).is_null()
                {
                    emsg(gettext(E_LETUNEXP.as_ptr()));
                } else {
                    set_var_lval(&raw mut lv, p, tv, copy, is_const, op);
                    arg_end = p;
                }
            }
            clear_lval(&raw mut lv);
        } else {
            semsg!(gettext(e_invarg2.as_ptr()), arg);
        }

        arg_end
    }
}

/// ":unlet[!] var1 ... " command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_unlet(eap: *mut exarg_T) {
    unsafe {
        ex_unletlock(
            eap,
            (*eap).arg,
            0,
            if (*eap).forceit != 0 { GLV_QUIET } else { 0 },
            do_unlet_var,
        );
    }
}

/// ":lockvar" and ":unlockvar" commands.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_lockvar(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut deep: c_int = 2;

        if (*eap).forceit != 0 {
            deep = -1;
        } else if ascii_isdigit(*arg as c_int) {
            deep = getdigits_int(&raw mut arg, false, -1);
            arg = skipwhite(arg);
        }

        ex_unletlock(eap, arg, deep, 0, do_lock_var);
    }
}

/// Common parsing logic for :unlet, :lockvar and :unlockvar.
unsafe fn ex_unletlock(
    eap: *mut exarg_T,
    argstart: *mut c_char,
    deep: c_int,
    glv_flags: c_int,
    callback: ex_unletlock_callback,
) {
    unsafe {
        let mut arg = argstart;
        let mut name_end: *mut c_char;
        let mut error = false;
        let mut lv: lval_T = std::mem::zeroed();

        loop {
            if *arg == b'$' as c_char {
                lv.ll_name = arg;
                lv.ll_tv = ptr::null_mut();
                arg = arg.add(1);
                let mut arg_const: *const c_char = arg;
                if get_env_len(&raw mut arg_const) == 0 {
                    semsg!(gettext(e_invarg2.as_ptr()), arg.sub(1));
                    return;
                }
                arg = arg_const.cast_mut();
                if !error && (*eap).skip == 0 && callback(&raw mut lv, arg, eap, deep) == FAIL {
                    error = true;
                }
                name_end = arg;
            } else {
                name_end = get_lval(
                    arg,
                    ptr::null_mut(),
                    &raw mut lv,
                    true,
                    (*eap).skip != 0 || error,
                    glv_flags,
                    FNE_CHECK_START,
                );
                if lv.ll_name.is_null() {
                    error = true;
                }
                if name_end.is_null()
                    || (!ascii_iswhite(*name_end as c_int) && ends_excmd(*name_end as c_int) == 0)
                {
                    if !name_end.is_null() {
                        emsg_severe = true;
                        semsg!(gettext(e_trailing_arg.as_ptr()), name_end);
                    }
                    if !((*eap).skip != 0 || error) {
                        clear_lval(&raw mut lv);
                    }
                    break;
                }

                if !error && (*eap).skip == 0 && callback(&raw mut lv, name_end, eap, deep) == FAIL
                {
                    error = true;
                }

                if (*eap).skip == 0 {
                    clear_lval(&raw mut lv);
                }
            }
            arg = skipwhite(name_end);
            if ends_excmd(*arg as c_int) != 0 {
                break;
            }
        }

        (*eap).nextcmd = check_nextcmd(arg);
    }
}

/// Unlet a variable indicated by `lp`.
unsafe extern "C" fn do_unlet_var(
    lp: *mut lval_T,
    name_end: *mut c_char,
    eap: *mut exarg_T,
    _deep: c_int,
) -> c_int {
    unsafe {
        let forceit = (*eap).forceit != 0;
        let mut ret = OK;

        if (*lp).ll_tv.is_null() {
            let cc = *name_end;
            *name_end = NUL;

            if *(*lp).ll_name == b'$' as c_char {
                vim_unsetenv_ext((*lp).ll_name.add(1));
            } else if do_unlet((*lp).ll_name, (*lp).ll_name_len, forceit) == FAIL {
                ret = FAIL;
            }
            *name_end = cc;
        } else if (!(*lp).ll_list.is_null()
            && value_check_lock(
                tv_list_locked((*lp).ll_list),
                (*lp).ll_name,
                (*lp).ll_name_len,
            ))
            || (!(*lp).ll_dict.is_null()
                && value_check_lock((*(*lp).ll_dict).dv_lock, (*lp).ll_name, (*lp).ll_name_len))
        {
            return FAIL;
        } else if (*lp).ll_range {
            tv_list_unlet_range(
                (*lp).ll_list,
                (*lp).ll_li,
                (*lp).ll_n1,
                !(*lp).ll_empty2,
                (*lp).ll_n2,
            );
        } else if !(*lp).ll_list.is_null() {
            tv_list_item_remove((*lp).ll_list, (*lp).ll_li);
        } else {
            let d = (*lp).ll_dict;
            debug_assert!(!d.is_null());
            let di = (*lp).ll_di;
            let watched = tv_dict_is_watched(d);
            let mut key: *mut c_char = ptr::null_mut();
            let mut oldtv: typval_T = std::mem::zeroed();

            if watched {
                tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
                key = xstrdup((*di).di_key.as_ptr());
            }

            tv_dict_item_remove(d, di);

            if watched {
                tv_dict_watcher_notify(d, key, ptr::null_mut(), &raw mut oldtv);
                tv_clear(&raw mut oldtv);
                xfree(key.cast());
            }
        }

        ret
    }
}

/// Unlet one item or a range of items from a list.
unsafe fn tv_list_unlet_range(
    l: *mut list_T,
    li_first: *mut listitem_T,
    n1_arg: c_int,
    has_n2: bool,
    n2: c_int,
) {
    unsafe {
        debug_assert!(!l.is_null());
        let mut li_last = li_first;
        let mut n1 = n1_arg;
        loop {
            let li = (*li_last).li_next;
            n1 += 1;
            if li.is_null() || (has_n2 && n2 < n1) {
                break;
            }
            li_last = li;
        }
        tv_list_remove_items(l, li_first, li_last);
    }
}

/// unlet a variable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_unlet(name: *const c_char, name_len: usize, forceit: bool) -> c_int {
    unsafe {
        let mut varname: *const c_char = ptr::null();
        let mut dict: *mut dict_T = ptr::null_mut();
        let mut ht = find_var_ht_dict(name, name_len, &raw mut varname, &raw mut dict);

        if !ht.is_null() && *varname != NUL {
            let mut d = get_current_funccal_dict(ht);
            if d.is_null() {
                if ptr::eq(ht, &raw mut globvardict.dv_hashtab) {
                    d = &raw mut globvardict;
                } else if ptr::eq(ht, &raw mut compat_hashtab) {
                    d = &raw mut vimvardict;
                } else {
                    let di = find_var_in_ht(ht, *name as u8 as c_int, c"".as_ptr(), 0, 0);
                    d = (*di).di_tv.vval.v_dict;
                }
                if d.is_null() {
                    internal_error(c"do_unlet()".as_ptr());
                    return FAIL;
                }
            }

            let mut hi = hash_find(ht, varname);
            if hashitem_empty(&*hi) {
                hi = find_hi_in_scoped_ht(name, &raw mut ht);
            }
            if !hi.is_null() && !hashitem_empty(&*hi) {
                let di = tv_dict_hi2di(hi);
                if var_check_fixed(c_int::from((*di).di_flags), name, TV_CSTRING)
                    || var_check_ro(c_int::from((*di).di_flags), name, TV_CSTRING)
                    || value_check_lock((*d).dv_lock, name, TV_CSTRING)
                {
                    return FAIL;
                }

                if value_check_lock((*d).dv_lock, name, TV_CSTRING) {
                    return FAIL;
                }

                let mut oldtv: typval_T = std::mem::zeroed();
                let watched = tv_dict_is_watched(dict);

                if watched {
                    tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
                }

                delete_var(ht, hi);

                if watched {
                    tv_dict_watcher_notify(dict, varname, ptr::null_mut(), &raw mut oldtv);
                    tv_clear(&raw mut oldtv);
                }
                return OK;
            }
        }
        if forceit {
            return OK;
        }
        semsg!(gettext(c"E108: No such variable: \"%s\"".as_ptr()), name);
        FAIL
    }
}

/// Lock or unlock variable indicated by `lp`.
unsafe extern "C" fn do_lock_var(
    lp: *mut lval_T,
    _name_end: *mut c_char,
    eap: *mut exarg_T,
    deep: c_int,
) -> c_int {
    unsafe {
        let lock = (*eap).cmdidx == cmdidx_T::CMD_lockvar as c_int;
        let mut ret = OK;

        if (*lp).ll_tv.is_null() {
            if *(*lp).ll_name == b'$' as c_char {
                semsg!(gettext(E_LOCK_UNLOCK.as_ptr()), (*lp).ll_name);
                ret = FAIL;
            } else {
                let di = find_var((*lp).ll_name, (*lp).ll_name_len, ptr::null_mut(), 1);
                if di.is_null() {
                    ret = FAIL;
                } else if ((*di).di_flags & DI_FLAGS_FIX) != 0
                    && (*di).di_tv.v_type != VarType::VAR_DICT
                    && (*di).di_tv.v_type != VarType::VAR_LIST
                {
                    semsg!(gettext(E_LOCK_UNLOCK.as_ptr()), (*lp).ll_name);
                    ret = FAIL;
                } else {
                    if lock {
                        (*di).di_flags |= DI_FLAGS_LOCK;
                    } else {
                        (*di).di_flags &= !DI_FLAGS_LOCK;
                    }
                    if deep != 0 {
                        tv_item_lock(&raw mut (*di).di_tv, deep, lock, false);
                    }
                }
            }
        } else if deep == 0 {
            // nothing to do
        } else if (*lp).ll_range {
            let mut li = (*lp).ll_li;
            while !li.is_null() && ((*lp).ll_empty2 || (*lp).ll_n2 >= (*lp).ll_n1) {
                tv_item_lock(&raw mut (*li).li_tv, deep, lock, false);
                li = (*li).li_next;
                (*lp).ll_n1 += 1;
            }
        } else if !(*lp).ll_list.is_null() {
            tv_item_lock(&raw mut (*(*lp).ll_li).li_tv, deep, lock, false);
        } else {
            tv_item_lock(&raw mut (*(*lp).ll_di).di_tv, deep, lock, false);
        }

        ret
    }
}

/// Delete all "menutrans_" variables.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn del_menutrans_vars() {
    unsafe {
        let ht = &raw mut globvardict.dv_hashtab;
        hash_lock(ht);
        let mut todo = (*ht).ht_used;
        let mut hi = (*ht).ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                if c_strncmp((*hi).hi_key, c"menutrans_".as_ptr(), 10) == 0 {
                    delete_var(ht, hi);
                }
            }
            hi = hi.add(1);
        }
        hash_unlock(ht);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_globvar_dict() -> *mut dict_T {
    &raw mut globvardict
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_globvar_ht() -> *mut hashtab_T {
    unsafe { &raw mut globvardict.dv_hashtab }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vimvar_dict() -> *mut dict_T {
    &raw mut vimvardict
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_tv(idx: VimVarIndex, tv: *mut typval_T) {
    unsafe {
        let tv_out = get_vim_var_tv(idx);
        tv_clear(tv_out);
        tv_copy(tv, tv_out);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_name(idx: VimVarIndex) -> *mut c_char {
    unsafe {
        (*(&raw const vimvars).cast::<VimVar>().add(idx as usize))
            .vv_name
            .cast_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_tv(idx: VimVarIndex) -> *mut typval_T {
    unsafe {
        &raw mut (*(&raw mut vimvars).cast::<VimVar>().add(idx as usize))
            .vv_di
            .di_tv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_nr(idx: VimVarIndex) -> varnumber_T {
    unsafe { (*get_vim_var_tv(idx)).vval.v_number }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_list(idx: VimVarIndex) -> *mut list_T {
    unsafe { (*get_vim_var_tv(idx)).vval.v_list }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_dict(idx: VimVarIndex) -> *mut dict_T {
    unsafe { (*get_vim_var_tv(idx)).vval.v_dict }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_str(idx: VimVarIndex) -> *mut c_char {
    unsafe { tv_get_string(get_vim_var_tv(idx)).cast_mut() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_vim_var_partial(idx: VimVarIndex) -> *mut partial_T {
    unsafe { (*get_vim_var_tv(idx)).vval.v_ptr.cast::<partial_T>() }
}

/// Function to concatenate a prefix and a variable name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cat_prefix_varname(prefix: c_int, name: *const c_char) -> *mut c_char {
    unsafe {
        let mut len = c_strlen(name) + 3;

        if len > varnamebuflen {
            xfree(varnamebuf.cast());
            len += 10;
            varnamebuf = xmalloc(len).cast::<c_char>();
            varnamebuflen = len;
        }
        *varnamebuf = prefix as u8 as c_char;
        *varnamebuf.add(1) = b':' as c_char;
        c_strcpy(varnamebuf.add(2), name);
        varnamebuf
    }
}

/// Function given to ExpandGeneric() to obtain the list of user defined variable names.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_user_var_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    static mut GDONE: usize = 0;
    static mut BDONE: usize = 0;
    static mut WDONE: usize = 0;
    static mut TDONE: usize = 0;
    static mut VIDX: usize = 0;
    static mut HI: *mut hashitem_T = ptr::null_mut();

    unsafe {
        if idx == 0 {
            GDONE = 0;
            BDONE = 0;
            WDONE = 0;
            VIDX = 0;
            TDONE = 0;
        }

        if GDONE < globvardict.dv_hashtab.ht_used {
            if GDONE == 0 {
                HI = globvardict.dv_hashtab.ht_array;
            } else {
                HI = HI.add(1);
            }
            GDONE += 1;
            while hashitem_empty(&*HI) {
                HI = HI.add(1);
            }
            if c_strncmp(c"g:".as_ptr(), (*xp).xp_pattern, 2) == 0 {
                return cat_prefix_varname(b'g' as c_int, (*HI).hi_key);
            }
            return (*HI).hi_key;
        }

        let mut ht: *const hashtab_T =
            &raw const (*(*(*prevwin_curwin()).w_buffer).b_vars).dv_hashtab;
        if BDONE < (*ht).ht_used {
            if BDONE == 0 {
                HI = (*ht).ht_array;
            } else {
                HI = HI.add(1);
            }
            BDONE += 1;
            while hashitem_empty(&*HI) {
                HI = HI.add(1);
            }
            return cat_prefix_varname(b'b' as c_int, (*HI).hi_key);
        }

        ht = &raw const (*(*prevwin_curwin()).w_vars).dv_hashtab;
        if WDONE < (*ht).ht_used {
            if WDONE == 0 {
                HI = (*ht).ht_array;
            } else {
                HI = HI.add(1);
            }
            WDONE += 1;
            while hashitem_empty(&*HI) {
                HI = HI.add(1);
            }
            return cat_prefix_varname(b'w' as c_int, (*HI).hi_key);
        }

        ht = &raw const (*(*curtab).tp_vars).dv_hashtab;
        if TDONE < (*ht).ht_used {
            if TDONE == 0 {
                HI = (*ht).ht_array;
            } else {
                HI = HI.add(1);
            }
            TDONE += 1;
            while hashitem_empty(&*HI) {
                HI = HI.add(1);
            }
            return cat_prefix_varname(b't' as c_int, (*HI).hi_key);
        }

        if VIDX < VV_LEN {
            let v = VIDX as VimVarIndex;
            VIDX += 1;
            return cat_prefix_varname(b'v' as c_int, get_vim_var_name(v));
        }

        xfree(varnamebuf.cast());
        varnamebuf = ptr::null_mut();
        varnamebuflen = 0;
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_type(idx: VimVarIndex, v_type: VarType) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        (*tv).v_type = v_type;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_nr(idx: VimVarIndex, val: varnumber_T) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).vval.v_number = val;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_bool(idx: VimVarIndex, val: BoolVarValue) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).v_type = VarType::VAR_BOOL;
        (*tv).vval.v_bool = val;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_special(idx: VimVarIndex, val: SpecialVarValue) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).v_type = VarType::VAR_SPECIAL;
        (*tv).vval.v_special = val;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_char(c: c_int) {
    unsafe {
        let mut buf = [0 as c_char; MB_MAXCHAR + 1];
        let buflen = utf_char2bytes(c, buf.as_mut_ptr());
        buf[buflen as usize] = NUL;
        set_vim_var_string(VV_CHAR, buf.as_ptr(), buflen as isize);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_string(idx: VimVarIndex, val: *const c_char, len: isize) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).v_type = VarType::VAR_STRING;
        if val.is_null() {
            (*tv).vval.v_string = ptr::null_mut();
        } else if len == -1 {
            (*tv).vval.v_string = xstrdup(val);
        } else {
            (*tv).vval.v_string = xstrndup(val, len as usize);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_list(idx: VimVarIndex, val: *mut list_T) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).vval.v_list = val;
        if !val.is_null() {
            tv_list_ref(val);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_dict(idx: VimVarIndex, val: *mut dict_T) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval.v_dict = val;
        if val.is_null() {
            return;
        }

        (*val).dv_refcount += 1;
        tv_dict_set_keys_readonly(val);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vim_var_partial(idx: VimVarIndex, val: *mut partial_T) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        (*tv).vval.v_ptr = val.cast();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_reg_var(c: c_int) {
    unsafe {
        let mut regname: [c_char; 2] = [0, NUL];
        if c == 0 || c == b' ' as c_int {
            regname[0] = b'"' as c_char;
        } else {
            regname[0] = c as u8 as c_char;
        }
        let tv = get_vim_var_tv(VV_REG);
        if (*tv).vval.v_string.is_null() || *(*tv).vval.v_string as c_int != c {
            set_vim_var_string(VV_REG, regname.as_ptr(), 1);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn v_exception(oldval: *mut c_char) -> *mut c_char {
    unsafe {
        let tv = get_vim_var_tv(VV_EXCEPTION);
        if oldval.is_null() {
            return (*tv).vval.v_string;
        }

        (*tv).vval.v_string = oldval;
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_cmdarg(eap: *mut exarg_T, oldarg: *mut c_char) -> *mut c_char {
    unsafe {
        let tv = get_vim_var_tv(VV_CMDARG);
        let oldval = (*tv).vval.v_string;
        if eap.is_null() {
            xfree(oldval.cast());
            (*tv).vval.v_string = oldarg;
            return ptr::null_mut();
        }

        let mut len: usize = 0;
        if (*eap).force_bin == FORCE_BIN {
            len += 6;
        } else if (*eap).force_bin == FORCE_NOBIN {
            len += 8;
        }

        if (*eap).read_edit != 0 {
            len += 7;
        }

        if (*eap).force_ff != 0 {
            len += 10;
        }
        if (*eap).force_enc != 0 {
            len += c_strlen((*eap).cmd.add((*eap).force_enc as usize)) + 7;
        }
        if (*eap).bad_char != 0 {
            len += 11;
        }
        if (*eap).mkdir_p != 0 {
            len += 4;
        }

        let newval_len = len + 1;
        let newval = xmalloc(newval_len).cast::<c_char>();
        let mut xlen: usize = 0;
        let mut rc: c_int = 0;

        if (*eap).force_bin == FORCE_BIN {
            rc = vim_snprintf!(newval, newval_len, c" ++bin".as_ptr());
        } else if (*eap).force_bin == FORCE_NOBIN {
            rc = vim_snprintf!(newval, newval_len, c" ++nobin".as_ptr());
        } else {
            *newval = NUL;
        }
        if rc < 0 {
            xfree(newval.cast());
            xfree(oldval.cast());
            (*tv).vval.v_string = oldarg;
            return ptr::null_mut();
        }
        xlen += rc as usize;

        if (*eap).read_edit != 0 {
            rc = vim_snprintf!(newval.add(xlen), newval_len - xlen, c" ++edit".as_ptr());
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        }

        if (*eap).force_ff != 0 {
            let ff_str = if (*eap).force_ff == b'u' as c_int {
                c"unix".as_ptr()
            } else if (*eap).force_ff == b'd' as c_int {
                c"dos".as_ptr()
            } else {
                c"mac".as_ptr()
            };
            rc = vim_snprintf!(
                newval.add(xlen),
                newval_len - xlen,
                c" ++ff=%s".as_ptr(),
                ff_str,
            );
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        }
        if (*eap).force_enc != 0 {
            rc = vim_snprintf!(
                newval.add(xlen),
                newval_len - xlen,
                c" ++enc=%s".as_ptr(),
                (*eap).cmd.add((*eap).force_enc as usize),
            );
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        }

        if (*eap).bad_char == BAD_KEEP {
            rc = vim_snprintf!(newval.add(xlen), newval_len - xlen, c" ++bad=keep".as_ptr());
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        } else if (*eap).bad_char == BAD_DROP {
            rc = vim_snprintf!(newval.add(xlen), newval_len - xlen, c" ++bad=drop".as_ptr());
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        } else if (*eap).bad_char != 0 {
            rc = vim_snprintf!(
                newval.add(xlen),
                newval_len - xlen,
                c" ++bad=%c".as_ptr(),
                (*eap).bad_char,
            );
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        }

        if (*eap).mkdir_p != 0 {
            rc = vim_snprintf!(newval.add(xlen), newval_len - xlen, c" ++p".as_ptr());
            if rc < 0 {
                xfree(newval.cast());
                xfree(oldval.cast());
                (*tv).vval.v_string = oldarg;
                return ptr::null_mut();
            }
            xlen += rc as usize;
        }
        debug_assert!(xlen <= newval_len);

        (*tv).vval.v_string = newval;
        oldval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn v_throwpoint(oldval: *mut c_char) -> *mut c_char {
    unsafe {
        let tv = get_vim_var_tv(VV_THROWPOINT);
        if oldval.is_null() {
            return (*tv).vval.v_string;
        }

        (*tv).vval.v_string = oldval;
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_vcount(count: i64, count1: i64, set_prevcount: bool) {
    unsafe {
        if set_prevcount {
            (*get_vim_var_tv(VV_PREVCOUNT)).vval.v_number = get_vim_var_nr(VV_COUNT);
        }
        (*get_vim_var_tv(VV_COUNT)).vval.v_number = count;
        (*get_vim_var_tv(VV_COUNT1)).vval.v_number = count1;
    }
}

/// Get the value of internal variable "name".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_variable(
    name: *const c_char,
    len: c_int,
    rettv: *mut typval_T,
    dip: *mut *mut dictitem_T,
    verbose: bool,
    no_autoload: bool,
) -> c_int {
    unsafe {
        let mut ret = OK;
        let mut tv: *mut typval_T = ptr::null_mut();

        let v = find_var(
            name,
            len as usize,
            ptr::null_mut(),
            c_int::from(no_autoload),
        );
        if !v.is_null() {
            tv = &raw mut (*v).di_tv;
            if !dip.is_null() {
                *dip = v;
            }
        }

        if tv.is_null() {
            if !rettv.is_null() && verbose {
                semsg!(
                    gettext(c"E121: Undefined variable: %.*s".as_ptr()),
                    len,
                    name,
                );
            }
            ret = FAIL;
        } else if !rettv.is_null() {
            tv_copy(tv, rettv);
        }

        ret
    }
}

/// Check if variable "name[len]" is a local variable or an argument.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_vars(name: *const c_char, len: usize) {
    unsafe {
        if eval_lavars_used.is_null() {
            return;
        }

        let mut varname: *const c_char = ptr::null();
        let ht = find_var_ht(name, len, &raw mut varname);

        if (ht == get_funccal_local_ht() || ht == get_funccal_args_ht())
            && !find_var(name, len, ptr::null_mut(), 1).is_null()
        {
            *eval_lavars_used = true;
        }
    }
}

/// Find variable "name" in the list of variables.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_var(
    name: *const c_char,
    name_len: usize,
    htp: *mut *mut hashtab_T,
    no_autoload: c_int,
) -> *mut dictitem_T {
    unsafe {
        let mut varname: *const c_char = ptr::null();
        let ht = find_var_ht(name, name_len, &raw mut varname);
        if !htp.is_null() {
            *htp = ht;
        }
        if ht.is_null() {
            return ptr::null_mut();
        }
        let ret = find_var_in_ht(
            ht,
            *name as u8 as c_int,
            varname,
            name_len - (varname.offset_from(name) as usize),
            c_int::from(no_autoload != 0 || !htp.is_null()),
        );
        if !ret.is_null() {
            return ret;
        }

        find_var_in_scoped_ht(
            name,
            name_len,
            c_int::from(no_autoload != 0 || !htp.is_null()),
        )
    }
}

/// Find variable in hashtab.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_var_in_ht(
    ht: *mut hashtab_T,
    htname: c_int,
    varname: *const c_char,
    varname_len: usize,
    no_autoload: c_int,
) -> *mut dictitem_T {
    unsafe {
        if varname_len == 0 {
            return match htname as u8 {
                b's' => (&raw mut (*script_sv(current_sctx.sc_sid)).sv_var).cast::<dictitem_T>(),
                b'g' => (&raw mut globvars_var).cast::<dictitem_T>(),
                b'v' => (&raw mut vimvars_var).cast::<dictitem_T>(),
                b'b' => (&raw mut (*curbuf).b_bufvar).cast::<dictitem_T>(),
                b'w' => (&raw mut (*curwin).w_winvar).cast::<dictitem_T>(),
                b't' => (&raw mut (*curtab).tp_winvar).cast::<dictitem_T>(),
                b'l' => get_funccal_local_var(),
                b'a' => get_funccal_args_var(),
                _ => ptr::null_mut(),
            };
        }

        let mut hi = hash_find_len(ht, varname, varname_len);
        if hashitem_empty(&*hi) {
            if ht == get_globvar_ht() && no_autoload == 0 {
                if !script_autoload(varname, varname_len, false) || aborting() {
                    return ptr::null_mut();
                }
                hi = hash_find_len(ht, varname, varname_len);
            }
            if hashitem_empty(&*hi) {
                return ptr::null_mut();
            }
        }
        tv_dict_hi2di(hi)
    }
}

/// Finds the dict (g:, l:, s:, …) and hashtable used for a variable.
unsafe fn find_var_ht_dict(
    name: *const c_char,
    name_len: usize,
    varname: *mut *const c_char,
    d: *mut *mut dict_T,
) -> *mut hashtab_T {
    unsafe {
        *d = ptr::null_mut();

        if name_len == 0 {
            return ptr::null_mut();
        }
        if name_len == 1 || *name.add(1) != b':' as c_char {
            if *name == b':' as c_char || *name == AUTOLOAD_CHAR {
                return ptr::null_mut();
            }
            *varname = name;

            let hi = hash_find_len(&raw const compat_hashtab, name, name_len);
            if !hashitem_empty(&*hi) {
                return &raw mut compat_hashtab;
            }

            *d = get_funccal_local_dict();
            if !(*d).is_null() {
                return &raw mut (*(*d)).dv_hashtab;
            }

            *d = get_globvar_dict();
            return &raw mut (*(*d)).dv_hashtab;
        }

        *varname = name.add(2);
        if *name == b'g' as c_char {
            *d = get_globvar_dict();
        } else if name_len > 2
            && (!c_memchr(name.add(2), b':', name_len - 2).is_null()
                || !c_memchr(name.add(2), AUTOLOAD_CHAR as u8, name_len - 2).is_null())
        {
            return ptr::null_mut();
        }

        if *name == b'b' as c_char {
            *d = (*curbuf).b_vars;
        } else if *name == b'w' as c_char {
            *d = (*curwin).w_vars;
        } else if *name == b't' as c_char {
            *d = (*curtab).tp_vars;
        } else if *name == b'v' as c_char {
            *d = get_vimvar_dict();
        } else if *name == b'a' as c_char {
            *d = get_funccal_args_dict();
        } else if *name == b'l' as c_char {
            *d = get_funccal_local_dict();
        } else if *name == b's' as c_char
            && (current_sctx.sc_sid > 0
                || current_sctx.sc_sid == SID_STR
                || current_sctx.sc_sid == SID_LUA)
            && current_sctx.sc_sid <= script_items.ga_len
        {
            nlua_set_sctx(&raw mut current_sctx);
            if current_sctx.sc_sid == SID_STR || current_sctx.sc_sid == SID_LUA {
                new_script_item(ptr::null_mut(), &raw mut current_sctx.sc_sid);
            }
            *d = &raw mut (*script_sv(current_sctx.sc_sid)).sv_dict;
        }

        if !(*d).is_null() {
            &raw mut (*(*d)).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

/// Find the hashtable used for a variable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_var_ht(
    name: *const c_char,
    name_len: usize,
    varname: *mut *const c_char,
) -> *mut hashtab_T {
    unsafe {
        let mut d: *mut dict_T = ptr::null_mut();
        find_var_ht_dict(name, name_len, varname, &raw mut d)
    }
}

/// Return the string value of a (global/local) variable or NULL when it doesn't exist.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_var_value(name: *const c_char) -> *mut c_char {
    unsafe {
        let v = find_var(name, c_strlen(name), ptr::null_mut(), 0);
        if v.is_null() {
            return ptr::null_mut();
        }
        tv_get_string(&raw const (*v).di_tv).cast_mut()
    }
}

/// Allocate a new hashtab for a sourced script.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn new_script_vars(id: scid_T) {
    unsafe {
        let sv = xcalloc(1, std::mem::size_of::<scriptvar_T>()).cast::<scriptvar_T>();
        init_var_dict(&raw mut (*sv).sv_dict, &raw mut (*sv).sv_var, VAR_SCOPE);
        (*script_item(id)).sn_vars = sv.cast();
    }
}

/// Initialize dictionary "dict" as a scope and set variable "dict_var" to point to it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn init_var_dict(
    dict: *mut dict_T,
    dict_var: *mut ScopeDictDictItem,
    scope: ScopeType,
) {
    unsafe {
        hash_init(&raw mut (*dict).dv_hashtab);
        (*dict).dv_lock = VAR_UNLOCKED;
        (*dict).dv_scope = scope;
        (*dict).dv_refcount = DO_NOT_FREE_CNT;
        (*dict).dv_copyID = 0;
        (*dict_var).di_tv.vval.v_dict = dict;
        (*dict_var).di_tv.v_type = VarType::VAR_DICT;
        (*dict_var).di_tv.v_lock = VAR_FIXED;
        (*dict_var).di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
        (*dict_var).di_key[0] = NUL;
        let q = (&raw mut (*dict).watchers).cast::<c_void>();
        (*dict).watchers[0] = q;
        (*dict).watchers[1] = q;
    }
}

/// Unreference a dictionary initialized by init_var_dict().
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn unref_var_dict(dict: *mut dict_T) {
    unsafe {
        (*dict).dv_refcount -= DO_NOT_FREE_CNT - 1;
        tv_dict_unref(dict);
    }
}

/// Clean up a list of internal variables.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vars_clear(ht: *mut hashtab_T) {
    unsafe {
        vars_clear_ext(ht, true);
    }
}

/// Like vars_clear(), but only free the value if "free_val" is true.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vars_clear_ext(ht: *mut hashtab_T, free_val: bool) {
    unsafe {
        hash_lock(ht);
        let mut todo = (*ht).ht_used as c_int;
        let mut hi = (*ht).ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;

                let v = tv_dict_hi2di(hi);
                if free_val {
                    tv_clear(&raw mut (*v).di_tv);
                }
                if ((*v).di_flags & DI_FLAGS_ALLOC) != 0 {
                    xfree(v.cast());
                }
            }
            hi = hi.add(1);
        }
        hash_clear(ht);
        hash_init(ht);
    }
}

/// Delete a variable from hashtab "ht" at item "hi".
unsafe fn delete_var(ht: *mut hashtab_T, hi: *mut hashitem_T) {
    unsafe {
        let di = tv_dict_hi2di(hi);
        hash_remove(ht, hi);
        tv_clear(&raw mut (*di).di_tv);
        xfree(di.cast());
    }
}

/// List the value of one internal variable.
unsafe fn list_one_var(v: *mut dictitem_T, prefix: *const c_char, first: *mut c_int) {
    unsafe {
        let s = encode_tv2echo(&raw mut (*v).di_tv, ptr::null_mut());
        list_one_var_a(
            prefix,
            (*v).di_key.as_ptr(),
            c_strlen((*v).di_key.as_ptr()) as isize,
            (*v).di_tv.v_type,
            if s.is_null() { c"".as_ptr() } else { s },
            first,
        );
        xfree(s.cast());
    }
}

unsafe fn list_one_var_a(
    prefix: *const c_char,
    name: *const c_char,
    name_len: isize,
    v_type: VarType,
    mut string: *const c_char,
    first: *mut c_int,
) {
    unsafe {
        if *first != 0 {
            msg_ext_set_kind(c"list_cmd".as_ptr());
            msg_start();
        } else {
            msg_putchar(b'\n' as c_int);
        }
        if *prefix != NUL {
            msg_puts(prefix);
        }
        if !name.is_null() {
            msg_puts_len(name, name_len, 0, false);
        }
        msg_putchar(b' ' as c_int);
        msg_advance(22);
        if v_type == VarType::VAR_NUMBER {
            msg_putchar(b'#' as c_int);
        } else if v_type == VarType::VAR_FUNC || v_type == VarType::VAR_PARTIAL {
            msg_putchar(b'*' as c_int);
        } else if v_type == VarType::VAR_LIST {
            msg_putchar(b'[' as c_int);
            if *string == b'[' as c_char {
                string = string.add(1);
            }
        } else if v_type == VarType::VAR_DICT {
            msg_putchar(b'{' as c_int);
            if *string == b'{' as c_char {
                string = string.add(1);
            }
        } else {
            msg_putchar(b' ' as c_int);
        }

        msg_outtrans(string, 0, false);

        if v_type == VarType::VAR_FUNC || v_type == VarType::VAR_PARTIAL {
            msg_puts(c"()".as_ptr());
        }
        if *first != 0 {
            msg_clr_eos();
            *first = 0;
        }
    }
}

/// Additional handling for setting a v: variable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn before_set_vvar(
    varname: *const c_char,
    di: *mut dictitem_T,
    tv: *mut typval_T,
    copy: bool,
    watched: bool,
    type_error: *mut bool,
) -> bool {
    unsafe {
        if (*di).di_tv.v_type == VarType::VAR_STRING {
            let mut oldtv: typval_T = std::mem::zeroed();
            if watched {
                tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
            }
            xfree((*di).di_tv.vval.v_string.cast());
            (*di).di_tv.vval.v_string = ptr::null_mut();
            if copy || (*tv).v_type != VarType::VAR_STRING {
                let val = tv_get_string(tv);
                if (*di).di_tv.vval.v_string.is_null() {
                    (*di).di_tv.vval.v_string = xstrdup(val);
                }
            } else {
                (*di).di_tv.vval.v_string = (*tv).vval.v_string;
                (*tv).vval.v_string = ptr::null_mut();
            }
            if watched {
                tv_dict_watcher_notify(
                    &raw mut vimvardict,
                    varname,
                    &raw mut (*di).di_tv,
                    &raw mut oldtv,
                );
                tv_clear(&raw mut oldtv);
            }
            return false;
        } else if (*di).di_tv.v_type == VarType::VAR_NUMBER {
            let mut oldtv: typval_T = std::mem::zeroed();
            if watched {
                tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
            }
            (*di).di_tv.vval.v_number = tv_get_number(tv);
            if c_strcmp(varname, c"searchforward".as_ptr()) == 0 {
                set_search_direction(if (*di).di_tv.vval.v_number != 0 {
                    b'/' as c_int
                } else {
                    b'?' as c_int
                });
            } else if c_strcmp(varname, c"hlsearch".as_ptr()) == 0 {
                Search.no_hlsearch = (*di).di_tv.vval.v_number == 0;
                redraw_all_later(UPD_SOME_VALID);
            }
            if watched {
                tv_dict_watcher_notify(
                    &raw mut vimvardict,
                    varname,
                    &raw mut (*di).di_tv,
                    &raw mut oldtv,
                );
                tv_clear(&raw mut oldtv);
            }
            return false;
        } else if (*di).di_tv.v_type != (*tv).v_type {
            *type_error = true;
            return false;
        }
        true
    }
}

/// Set variable to the given value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_var(
    name: *const c_char,
    name_len: usize,
    tv: *mut typval_T,
    copy: bool,
) {
    unsafe {
        set_var_const(name, name_len, tv, copy, false);
    }
}

/// Set variable to the given value, optionally locking as const.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_var_const(
    name: *const c_char,
    name_len: usize,
    tv: *mut typval_T,
    copy: bool,
    is_const: bool,
) {
    unsafe {
        let mut varname: *const c_char = ptr::null();
        let mut dict: *mut dict_T = ptr::null_mut();
        let ht = find_var_ht_dict(name, name_len, &raw mut varname, &raw mut dict);
        let watched = tv_dict_is_watched(dict);

        if ht.is_null() || *varname == NUL {
            semsg!(gettext(e_illvar.as_ptr()), name);
            return;
        }
        let varname_len = name_len - (varname.offset_from(name) as usize);
        let mut di = find_var_in_ht(ht, 0, varname, varname_len, 1);

        if di.is_null() {
            di = find_var_in_scoped_ht(name, name_len, 1);
        }

        if tv_is_func(*tv) && var_wrong_func_name(name, di.is_null()) {
            return;
        }

        let mut oldtv: typval_T = std::mem::zeroed();
        if !di.is_null() {
            if is_const {
                emsg(gettext(e_cannot_mod.as_ptr()));
                return;
            }

            if var_check_ro(c_int::from((*di).di_flags), name, name_len)
                || value_check_lock((*di).di_tv.v_lock, name, name_len)
                || var_check_lock(c_int::from((*di).di_flags), name, name_len)
            {
                return;
            }

            let mut type_error = false;
            if ptr::eq(ht, &raw mut vimvardict.dv_hashtab)
                && !before_set_vvar(varname, di, tv, copy, watched, &raw mut type_error)
            {
                if type_error {
                    semsg!(
                        gettext(E_SETTING_V_STR_TO_VALUE_WITH_WRONG_TYPE.as_ptr()),
                        varname,
                    );
                }
                return;
            }

            if watched {
                tv_copy(&raw const (*di).di_tv, &raw mut oldtv);
            }
            tv_clear(&raw mut (*di).di_tv);
        } else {
            if ptr::eq(ht, &raw mut vimvardict.dv_hashtab) || ht == get_funccal_args_ht() {
                semsg!(gettext(e_illvar.as_ptr()), name);
                return;
            }

            if !valid_varname(varname) {
                return;
            }

            debug_assert!(!dict.is_null());

            di = xmalloc(std::mem::offset_of!(dictitem_T, di_key) + varname_len + 1)
                .cast::<dictitem_T>();
            ptr::copy_nonoverlapping(varname, (*di).di_key.as_mut_ptr(), varname_len + 1);
            if hash_add(ht, (*di).di_key.as_mut_ptr()) == FAIL {
                xfree(di.cast());
                return;
            }
            (*di).di_flags = DI_FLAGS_ALLOC;
            if is_const {
                (*di).di_flags |= DI_FLAGS_LOCK;
            }
        }

        if copy || (*tv).v_type == VarType::VAR_NUMBER || (*tv).v_type == VarType::VAR_FLOAT {
            tv_copy(tv, &raw mut (*di).di_tv);
        } else {
            (*di).di_tv = *tv;
            (*di).di_tv.v_lock = VAR_UNLOCKED;
            tv_init(tv);
        }

        if watched {
            tv_dict_watcher_notify(
                dict,
                (*di).di_key.as_ptr(),
                &raw mut (*di).di_tv,
                &raw mut oldtv,
            );
            tv_clear(&raw mut oldtv);
        }

        if is_const {
            tv_item_lock(&raw mut (*di).di_tv, DICT_MAXNEST, true, true);
        }
    }
}

/// Check whether variable is read-only (DI_FLAGS_RO, DI_FLAGS_RO_SBX).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_check_ro(
    flags: c_int,
    mut name: *const c_char,
    mut name_len: usize,
) -> bool {
    unsafe {
        let error_message = if (flags as u8 & DI_FLAGS_RO) != 0 {
            e_cannot_change_readonly_variable_str.as_ptr()
        } else if (flags as u8 & DI_FLAGS_RO_SBX) != 0 && sandbox != 0 {
            e_cannot_set_variable_in_sandbox_str.as_ptr()
        } else {
            ptr::null()
        };

        if error_message.is_null() {
            return false;
        }
        if name_len == TV_TRANSLATE {
            name = gettext(name);
            name_len = c_strlen(name);
        } else if name_len == TV_CSTRING {
            name_len = c_strlen(name);
        }

        semsg!(gettext(error_message), name_len as c_int, name);
        true
    }
}

/// Return true if di_flags "flags" indicates variable "name" is locked.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_check_lock(
    flags: c_int,
    mut name: *const c_char,
    mut name_len: usize,
) -> bool {
    unsafe {
        if (flags as u8 & DI_FLAGS_LOCK) == 0 {
            return false;
        }

        if name_len == TV_TRANSLATE {
            name = gettext(name);
            name_len = c_strlen(name);
        } else if name_len == TV_CSTRING {
            name_len = c_strlen(name);
        }

        semsg!(
            gettext(c"E1122: Variable is locked: %.*s".as_ptr()),
            name_len as c_int,
            name,
        );
        true
    }
}

/// Check whether variable is fixed (DI_FLAGS_FIX).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_check_fixed(
    flags: c_int,
    mut name: *const c_char,
    mut name_len: usize,
) -> bool {
    unsafe {
        if (flags as u8 & DI_FLAGS_FIX) != 0 {
            if name_len == TV_TRANSLATE {
                name = gettext(name);
                name_len = c_strlen(name);
            } else if name_len == TV_CSTRING {
                name_len = c_strlen(name);
            }
            semsg!(
                gettext(e_cannot_delete_variable_str.as_ptr()),
                name_len as c_int,
                name,
            );
            return true;
        }
        false
    }
}

/// Check if name is a valid name to assign funcref to.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_wrong_func_name(name: *const c_char, new_var: bool) -> bool {
    unsafe {
        let first_after_scope = if *name != NUL && *name.add(1) == b':' as c_char {
            *name.add(2)
        } else {
            *name
        };
        if !(!vim_strchr(c"wbst".as_ptr(), *name as u8 as c_int).is_null()
            && *name.add(1) == b':' as c_char)
            && !ascii_isupper(first_after_scope)
            && vim_strchr(name, b'#' as c_int).is_null()
        {
            semsg!(
                gettext(c"E704: Funcref variable name must start with a capital: %s".as_ptr()),
                name,
            );
            return true;
        }
        if new_var && function_exists(name, false) {
            semsg!(
                gettext(c"E705: Variable name conflicts with existing function: %s".as_ptr()),
                name,
            );
            return true;
        }
        false
    }
}

/// Check if a variable name is valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_varname(varname: *const c_char) -> bool {
    unsafe {
        let mut p = varname;
        while *p != NUL {
            if !eval_isnamec1(*p as u8 as c_int)
                && (p == varname || !ascii_isdigit(*p as c_int))
                && *p != AUTOLOAD_CHAR
            {
                semsg!(gettext(e_illvar.as_ptr()), varname);
                return false;
            }
            p = p.add(1);
        }
        true
    }
}

/// Implements the logic to retrieve local variable and option values.
unsafe fn get_var_from(
    mut varname: *const c_char,
    rettv: *mut typval_T,
    deftv: *mut typval_T,
    htname: c_int,
    tp: *mut tabpage_T,
    win: *mut win_T,
    buf: *mut buf_T,
) {
    unsafe {
        let mut done = false;
        let do_change_curbuf = !buf.is_null() && htname == b'b' as c_int;

        emsg_off += 1;

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if !varname.is_null()
            && !tp.is_null()
            && !win.is_null()
            && (htname != b'b' as c_int || !buf.is_null())
        {
            let need_switch_win = !(tp == curtab && win == curwin) && !do_change_curbuf;
            let mut switchwin: CtxSwitch = std::mem::zeroed();
            if !need_switch_win
                || ctx_switch(
                    &raw mut switchwin,
                    win,
                    tp,
                    ptr::null_mut(),
                    kCtxNoEvents | kCtxNoDisplay,
                )
            {
                if *varname == b'&' as c_char && htname != b't' as c_int {
                    let save_curbuf = curbuf;

                    if do_change_curbuf {
                        curbuf = buf;
                    }

                    if *varname.add(1) == NUL {
                        let opts = get_winbuf_options(c_int::from(htname == b'b' as c_int));
                        if !opts.is_null() {
                            tv_dict_set_ret(rettv, opts);
                            done = true;
                        }
                    } else if eval_option(&raw mut varname, rettv, true) == OK {
                        done = true;
                    }

                    curbuf = save_curbuf;
                } else if *varname == NUL {
                    let v: *const ScopeDictDictItem = if htname == b'b' as c_int {
                        &raw const (*buf).b_bufvar
                    } else if htname == b'w' as c_int {
                        (&raw const (*win).w_winvar).cast()
                    } else {
                        (&raw const (*tp).tp_winvar).cast()
                    };
                    tv_copy(&raw const (*v).di_tv, rettv);
                    done = true;
                } else {
                    let ht = if htname == b'b' as c_int {
                        &raw mut (*(*buf).b_vars).dv_hashtab
                    } else if htname == b'w' as c_int {
                        &raw mut (*(*win).w_vars).dv_hashtab
                    } else {
                        &raw mut (*(*tp).tp_vars).dv_hashtab
                    };

                    let v = find_var_in_ht(ht, htname, varname, c_strlen(varname), 0);
                    if !v.is_null() {
                        tv_copy(&raw const (*v).di_tv, rettv);
                        done = true;
                    }
                }
            }

            if need_switch_win {
                ctx_restore(&raw mut switchwin);
            }
        }

        if !done && (*deftv).v_type != VarType::VAR_UNKNOWN {
            tv_copy(deftv, rettv);
        }

        emsg_off -= 1;
    }
}

/// getwinvar() and gettabwinvar().
unsafe fn getwinvar(argvars: *mut typval_T, rettv: *mut typval_T, off: usize) {
    unsafe {
        let tp = if off == 1 {
            find_tabpage(tv_get_number_chk(argvars, ptr::null_mut()) as c_int)
        } else {
            curtab
        };
        let win = find_win_by_nr(argvars.add(off), tp);
        let varname = tv_get_string_chk(argvars.add(off + 1));

        get_var_from(
            varname,
            rettv,
            argvars.add(off + 2),
            b'w' as c_int,
            tp,
            win,
            ptr::null_mut(),
        );
    }
}

/// Converts a typval to a structured option value.
unsafe fn opt_from_tv(
    tv: *mut typval_T,
    opt_idx: OptIndex,
    option: *const c_char,
    error: *mut bool,
) -> Object {
    unsafe {
        let mut value = nil_obj();
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut err = false;
        let is_tty_opt = is_tty_option(option);
        let option_has_bool =
            !is_tty_opt && option_has_type(opt_idx, ObjectType::kObjectTypeBoolean);
        let option_has_num =
            !is_tty_opt && option_has_type(opt_idx, ObjectType::kObjectTypeInteger);
        let option_has_str = is_tty_opt || option_has_type(opt_idx, ObjectType::kObjectTypeString);

        if !is_tty_opt && ((*get_option(opt_idx)).flags & kOptFlagFunc) != 0 && tv_is_func(*tv) {
            let strval = encode_tv2string(tv, ptr::null_mut());
            err = strval.is_null();
            value = string_obj(cstr_as_string(strval));
        } else if option_has_bool || option_has_num {
            let n = if option_has_num {
                tv_get_number_chk(tv, &raw mut err)
            } else {
                tv_get_bool_chk(tv, &raw mut err)
            };
            if !err && (*tv).v_type == VarType::VAR_STRING && n == 0 {
                let mut idx: usize = 0;
                while !(*tv).vval.v_string.is_null()
                    && *(*tv).vval.v_string.add(idx) == b'0' as c_char
                {
                    idx += 1;
                }
                if idx == 0 || *(*tv).vval.v_string.add(idx) != NUL {
                    err = true;
                    semsg!(
                        gettext(c"E521: Number required: &%s = '%s'".as_ptr()),
                        option,
                        if (*tv).vval.v_string.is_null() {
                            c"".as_ptr()
                        } else {
                            (*tv).vval.v_string
                        },
                    );
                }
            }
            value = if option_has_num {
                integer_obj(n as OptInt)
            } else {
                opt_from_tristate(tristate_from_int(n as OptInt))
            };
        } else if option_has_str {
            if (*tv).v_type != VarType::VAR_BOOL && (*tv).v_type != VarType::VAR_SPECIAL {
                let strval = tv_get_string_buf_chk(tv, nbuf.as_mut_ptr());
                err = strval.is_null();
                value = string_obj(cstr_to_string(strval));
            } else {
                err = true;
                emsg(gettext(e_string_required.as_ptr()));
            }
        } else {
            std::process::abort();
        }

        if !error.is_null() {
            *error = err;
        }
        value
    }
}

/// Converts an option value to typval.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn opt_to_tv(value: Object, numbool: bool) -> typval_T {
    unsafe {
        let mut rettv = typval_T {
            v_type: VarType::VAR_SPECIAL,
            v_lock: 0,
            vval: TypvalVval {
                v_special: kSpecialVarNull,
            },
        };

        match value.obj_type {
            ObjectType::kObjectTypeUnset => {
                if numbool {
                    rettv.v_type = VarType::VAR_NUMBER;
                    rettv.vval.v_number = varnumber_T::from(kNone);
                }
            }
            ObjectType::kObjectTypeNil => {}
            ObjectType::kObjectTypeBoolean => {
                if numbool {
                    rettv.v_type = VarType::VAR_NUMBER;
                    rettv.vval.v_number = varnumber_T::from(value.data.boolean);
                } else {
                    rettv.v_type = VarType::VAR_BOOL;
                    rettv.vval.v_bool = if value.data.boolean {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    };
                }
            }
            ObjectType::kObjectTypeInteger => {
                rettv.v_type = VarType::VAR_NUMBER;
                rettv.vval.v_number = value.data.integer;
            }
            ObjectType::kObjectTypeString => {
                rettv.v_type = VarType::VAR_STRING;
                rettv.vval.v_string = if !value.data.string.data.is_null() {
                    xstrdup(value.data.string.data)
                } else {
                    ptr::null_mut()
                };
            }
            ObjectType::kObjectTypeLuaRef => {
                rettv.v_type = VarType::VAR_STRING;
                rettv.vval.v_string = nlua_funcref_str(value.data.luaref, ptr::null_mut(), true);
            }
            _ => {
                std::process::abort();
            }
        }

        rettv
    }
}

/// Set option "varname" to the value of "varp" for the current buffer/window.
unsafe fn set_option_from_tv(varname: *const c_char, varp: *mut typval_T) {
    unsafe {
        let opt_idx = find_option(varname);
        if opt_idx == kOptInvalid {
            semsg!(gettext(e_unknown_option2.as_ptr()), varname);
            return;
        }

        let mut error = false;
        let value = opt_from_tv(varp, opt_idx, varname, &raw mut error);

        if !error {
            let mut errbuf_storage = [0 as c_char; IOSIZE];
            let errbuf = CharBuf {
                data: errbuf_storage.as_mut_ptr(),
                size: IOSIZE,
            };
            let errmsg = set_option_value(opt_idx, value, OPT_LOCAL, true, &raw const errbuf);
            if !errmsg.is_null() {
                emsg(errmsg);
            }
        }
        optval_free(value);
    }
}

/// "setwinvar()" and "settabwinvar()" functions.
unsafe fn setwinvar(argvars: *mut typval_T, off: usize) {
    unsafe {
        if check_secure() {
            return;
        }

        let tp = if off == 1 {
            find_tabpage(tv_get_number_chk(argvars, ptr::null_mut()) as c_int)
        } else {
            curtab
        };
        let win = find_win_by_nr(argvars.add(off), tp);
        let varname = tv_get_string_chk(argvars.add(off + 1));
        let varp = argvars.add(off + 2);

        if win.is_null() || varname.is_null() {
            return;
        }

        let need_switch_win = !(tp == curtab && win == curwin);
        let mut switchwin: CtxSwitch = std::mem::zeroed();
        if !need_switch_win
            || ctx_switch(
                &raw mut switchwin,
                win,
                tp,
                ptr::null_mut(),
                kCtxNoEvents | kCtxNoDisplay,
            )
        {
            if *varname == b'&' as c_char {
                set_option_from_tv(varname.add(1), varp);
            } else {
                let varname_len = c_strlen(varname);
                let winvarname = xmalloc(varname_len + 3).cast::<c_char>();
                ptr::copy_nonoverlapping(c"w:".as_ptr(), winvarname, 2);
                ptr::copy_nonoverlapping(varname, winvarname.add(2), varname_len + 1);
                set_var(winvarname, varname_len + 2, varp, true);
                xfree(winvarname.cast());
            }
        }
        if need_switch_win {
            ctx_restore(&raw mut switchwin);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn reset_v_option_vars() {
    unsafe {
        set_vim_var_string(VV_OPTION_NEW, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLD, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLDLOCAL, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLDGLOBAL, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_COMMAND, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_TYPE, ptr::null(), -1);
    }
}

/// Add an assert error to v:errors.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn assert_error(gap: *mut garray_T) {
    unsafe {
        let tv = get_vim_var_tv(VV_ERRORS);

        if (*tv).v_type != VarType::VAR_LIST || (*tv).vval.v_list.is_null() {
            set_vim_var_list(VV_ERRORS, tv_list_alloc(1));
        }
        tv_list_append_string(
            get_vim_var_list(VV_ERRORS),
            (*gap).ga_data.cast::<c_char>(),
            (*gap).ga_len as isize,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_exists(mut var: *const c_char) -> bool {
    unsafe {
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut n = false;

        let mut name = var;
        let len = get_name_len(&raw mut var, &raw mut tofree, true, false);
        if len > 0 {
            let mut tv: typval_T = std::mem::zeroed();

            if !tofree.is_null() {
                name = tofree;
            }
            n = eval_variable(name, len, &raw mut tv, ptr::null_mut(), false, true) == OK;
            if n {
                n = handle_subscript(&raw mut var, &raw mut tv, &raw mut EVALARG_EVALUATE, false)
                    == OK;
                if n {
                    tv_clear(&raw mut tv);
                }
            }
        }
        if *var != NUL {
            n = false;
        }

        xfree(tofree.cast());
        n
    }
}

/// Start recording command output to a variable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_redir_start(name: *mut c_char, append: bool) -> c_int {
    unsafe {
        if !eval_isnamec1(*name as u8 as c_int) {
            emsg(gettext(e_invarg.as_ptr()));
            return FAIL;
        }

        redir_varname = xstrdup(name);
        redir_lval = xcalloc(1, std::mem::size_of::<lval_T>()).cast::<lval_T>();

        ga_init(
            &raw mut redir_ga,
            std::mem::size_of::<c_char>() as c_int,
            500,
        );

        redir_endp = get_lval(
            redir_varname,
            ptr::null_mut(),
            redir_lval,
            false,
            false,
            0,
            FNE_CHECK_START,
        );
        if redir_endp.is_null() || (*redir_lval).ll_name.is_null() || *redir_endp != NUL {
            clear_lval(redir_lval);
            if !redir_endp.is_null() && *redir_endp != NUL {
                semsg!(gettext(e_trailing_arg.as_ptr()), redir_endp);
            } else {
                semsg!(gettext(e_invarg2.as_ptr()), name);
            }
            redir_endp = ptr::null_mut();
            var_redir_stop();
            return FAIL;
        }

        let called_emsg_before = called_emsg;
        did_emsg = 0;
        let mut tv = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval {
                v_string: c"".as_ptr().cast_mut(),
            },
        };
        if append {
            set_var_lval(
                redir_lval,
                redir_endp,
                &raw mut tv,
                true,
                false,
                c".".as_ptr(),
            );
        } else {
            set_var_lval(
                redir_lval,
                redir_endp,
                &raw mut tv,
                true,
                false,
                c"=".as_ptr(),
            );
        }
        clear_lval(redir_lval);
        if called_emsg > called_emsg_before {
            redir_endp = ptr::null_mut();
            var_redir_stop();
            return FAIL;
        }

        OK
    }
}

/// Append "value[value_len]" to the variable set by var_redir_start().
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_redir_str(value: *const c_char, value_len: c_int) {
    unsafe {
        if redir_lval.is_null() {
            return;
        }

        let len = if value_len == -1 {
            c_strlen(value) as c_int
        } else {
            value_len
        };

        ga_grow(&raw mut redir_ga, len);
        ptr::copy(
            value,
            redir_ga
                .ga_data
                .cast::<c_char>()
                .add(redir_ga.ga_len as usize),
            len as usize,
        );
        redir_ga.ga_len += len;
    }
}

/// Stop redirecting command output to a variable.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn var_redir_stop() {
    unsafe {
        if !redir_lval.is_null() {
            if !redir_endp.is_null() {
                ga_append(&raw mut redir_ga, 0);
                let mut tv = typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval {
                        v_string: redir_ga.ga_data.cast::<c_char>(),
                    },
                };
                redir_endp = get_lval(
                    redir_varname,
                    ptr::null_mut(),
                    redir_lval,
                    false,
                    false,
                    0,
                    FNE_CHECK_START,
                );
                if !redir_endp.is_null() && !(*redir_lval).ll_name.is_null() {
                    set_var_lval(
                        redir_lval,
                        redir_endp,
                        &raw mut tv,
                        false,
                        false,
                        c".".as_ptr(),
                    );
                }
                clear_lval(redir_lval);
            }

            xfree(redir_ga.ga_data);
            redir_ga.ga_data = ptr::null_mut();

            xfree(redir_lval.cast());
            redir_lval = ptr::null_mut();
        }
        xfree(redir_varname.cast());
        redir_varname = ptr::null_mut();
    }
}

/// "gettabvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_gettabvar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let varname = tv_get_string_chk(argvars.add(1));
        let tp = find_tabpage(tv_get_number_chk(argvars, ptr::null_mut()) as c_int);
        let mut win: *mut win_T = ptr::null_mut();

        if !tp.is_null() {
            win = if tp == curtab || (*tp).tp_firstwin.is_null() {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
        }

        get_var_from(
            varname,
            rettv,
            argvars.add(2),
            b't' as c_int,
            tp,
            win,
            ptr::null_mut(),
        );
    }
}

/// "gettabwinvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_gettabwinvar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getwinvar(argvars, rettv, 1);
    }
}

/// "getwinvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getwinvar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getwinvar(argvars, rettv, 0);
    }
}

/// "getbufvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getbufvar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let varname = tv_get_string_chk(argvars.add(1));
        let buf = tv_get_buf_from_arg(argvars);

        get_var_from(
            varname,
            rettv,
            argvars.add(2),
            b'b' as c_int,
            curtab,
            curwin,
            buf,
        );
    }
}

/// "settabvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_settabvar(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }

        let tp = find_tabpage(tv_get_number_chk(argvars, ptr::null_mut()) as c_int);
        let varname = tv_get_string_chk(argvars.add(1));
        let varp = argvars.add(2);

        if varname.is_null() || tp.is_null() {
            return;
        }

        let save_curtab = curtab;
        let save_lu_tp = lastused_tabpage;
        goto_tabpage_tp(tp, false, false);

        let varname_len = c_strlen(varname);
        let tabvarname = xmalloc(varname_len + 3).cast::<c_char>();
        ptr::copy_nonoverlapping(c"t:".as_ptr(), tabvarname, 2);
        ptr::copy_nonoverlapping(varname, tabvarname.add(2), varname_len + 1);
        set_var(tabvarname, varname_len + 2, varp, true);
        xfree(tabvarname.cast());

        if valid_tabpage(save_curtab) {
            goto_tabpage_tp(save_curtab, false, false);
            if valid_tabpage(save_lu_tp) {
                lastused_tabpage = save_lu_tp;
            }
        }
    }
}

/// "settabwinvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_settabwinvar(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        setwinvar(argvars, 1);
    }
}

/// "setwinvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setwinvar(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        setwinvar(argvars, 0);
    }
}

/// "setbufvar()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setbufvar(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() || !tv_check_str_or_nr(argvars) {
            return;
        }
        let varname = tv_get_string_chk(argvars.add(1));
        let buf = tv_get_buf(argvars, 0);
        let varp = argvars.add(2);

        if buf.is_null() || varname.is_null() {
            return;
        }

        if *varname == b'&' as c_char {
            let mut aco: CtxSwitch = std::mem::zeroed();
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            set_option_from_tv(varname.add(1), varp);
            ctx_restore(&raw mut aco);
        } else {
            let varname_len = c_strlen(varname);
            let bufvarname = xmalloc(varname_len + 3).cast::<c_char>();
            let save_curbuf = curbuf;
            curbuf = buf;
            ptr::copy_nonoverlapping(c"b:".as_ptr(), bufvarname, 2);
            ptr::copy_nonoverlapping(varname, bufvarname.add(2), varname_len + 1);
            set_var(bufvarname, varname_len + 2, varp, true);
            xfree(bufvarname.cast());
            curbuf = save_curbuf;
        }
    }
}
