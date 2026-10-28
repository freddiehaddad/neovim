//! Functions for dealing with Vimscript variables (`src/nvim/eval/vars.c`).

use crate::api::private::helpers::{
    DI_FLAGS_FIX, DI_FLAGS_LOCK, DI_FLAGS_RO, SID_LUA, cstr_to_string, nil_obj, string_obj, tv_dict_is_watched,
};
use crate::api::private::validate::{Object, ObjectType};
use crate::buffer::{DO_NOT_FREE_CNT, NOTDONE, VAR_FIXED, VAR_SCOPE, buf_T};
use crate::charset::{
    NUL, ascii_isdigit, ascii_isupper, ascii_iswhite, getdigits_int, skiptowhite, skipwhite,
};
use crate::cmdexpand::cmdidx_T;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::errors::*;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::funcs::{SpecialVarValue, kSpecialVarNull, tv_get_buf, tv_get_buf_from_arg};
use crate::eval::typval::{
    BoolVarValue, DI_FLAGS_ALLOC, EvalFuncData, ScopeDictDictItem, TV_CSTRING, TV_TRANSLATE,
    TypvalVval, VAR_UNLOCKED, VarLockStatus, VarType, blob_T, dict_T, dictitem_T, list_T,
    listitem_T, partial_T, tv_check_str_or_nr, tv_clear, tv_copy, tv_dict_add, tv_dict_alloc,
    tv_dict_alloc_lock, tv_dict_hi2di, tv_dict_item_alloc, tv_dict_item_remove,
    tv_dict_set_keys_readonly, tv_dict_unref, tv_dict_watcher_notify, tv_get_bool_chk,
    tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk,
    tv_is_func, tv_item_lock, tv_list_alloc,
    tv_list_append_string, tv_list_append_tv, tv_list_find_nr, tv_list_find_str, tv_list_first,
    tv_list_free, tv_list_item_remove, tv_list_len, tv_list_ref, tv_list_remove_items,
    tv_list_set_lock, tv_list_set_ret, typval_T, value_check_lock, varnumber_T,
};
use crate::eval::userfunc::{
    find_hi_in_scoped_ht, find_var_in_scoped_ht, function_exists, get_current_funccal_dict,
    get_funccal_args_dict, get_funccal_args_ht, get_funccal_args_var, get_funccal_local_dict,
    get_funccal_local_ht, get_funccal_local_var, list_func_vars,
};
use crate::eval::window::find_win_by_nr;
use crate::eval::{
    AUTOLOAD_CHAR, DICT_MAXNEST, FNE_CHECK_START, FNE_INCL_BR, GLV_QUIET, VAR_DEF_SCOPE,
    callback_call, eval_isnamec1, eval_st, eval_option, eval_to_string,
    eval0, eval1, fill_evalarg_from_eap, find_name_end, find_option_var_end, get_env_len, get_lval,
    get_name_len, handle_subscript, kListLenUnknown, may_call_simple_func, num_divide, num_modulus,
    partial_unref, set_ref_in_ht, set_var_lval, skip_expr, tv_dict_set_ret,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{check_nextcmd, ends_excmd, exarg_T};
use crate::ex_eval::aborting;
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::hashtab::{
    hash_add, hash_clear, hash_find, hash_find_len, hash_lock, hash_remove, hash_unlock,
    hashitem_T, hashitem_empty, hashtab_T,
};
use crate::lua::executor::{nlua_funcref_str, nlua_set_sctx};
use crate::math::{FAIL, OK};
use crate::mbyte::utf_char2bytes;
use crate::memory::{cstr_from_bytes, cstr_to_cstring, xstrlcpy};
use crate::message::{
    emsg, internal_error, message_filtered, msg_advance, msg_clr_eos, msg_ext_set_kind,
    msg_outtrans, msg_putchar, msg_puts, msg_puts_len, msg_start, semsg,
};
use crate::option::{
    OPT_LOCAL, OptInt, find_option, get_option, get_option_sctx, get_option_value, get_tty_option,
    get_winbuf_options, is_option_hidden, is_tty_option, kOptInvalid, opt_from_tristate,
    option_has_type, optval_free, set_option_value, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::optionstr::{CharBuf, OptIndex};
use crate::os::env::{
    expand_T, os_env_exists, os_getenv_buf, os_unsetenv, vim_getenv, vim_setenv_ext,
    vim_unsetenv_ext,
};
use crate::register::{get_reg_contents_str, kGRegExprSrc, write_reg_contents};
use crate::runtime::{new_script_item, script_autoload, script_item, scriptvar_T};
use crate::search::set_search_direction;
use crate::strings::{concat_str, gettext, vim_strchr};
use crate::version::{highest_patch, min_vim_version};
use crate::window::{
    find_tabpage, goto_tabpage_tp, prevwin_curwin, tabpage_T, valid_tabpage, win_T,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type VimVarIndex = c_int;
pub type ScopeType = c_int;
pub type scid_T = c_int;
type ex_unletlock_callback = fn(&mut lval_T, *mut c_char, &mut exarg_T, c_int) -> bool;

use crate::api::win_config::{kCtxNoDisplay, kCtxNoEvents};
use crate::change::MB_MAXCHAR;
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::regexp::NUMBUFLEN;

#[repr(C)]
pub struct evalarg_T {
    pub eval_flags: c_int,
}

impl evalarg_T {
    pub const fn new() -> Self {
        Self { eval_flags: 0 }
    }
}

impl Default for evalarg_T {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct lval_T {
    pub ll_name: *const c_char,
    pub ll_name_len: usize,
    pub ll_exp_name: Option<std::ffi::CString>,
    pub ll_tv: *mut typval_T,
    pub ll_li: *mut listitem_T,
    pub ll_list: *mut list_T,
    pub ll_range: bool,
    pub ll_empty2: bool,
    pub ll_n1: c_int,
    pub ll_n2: c_int,
    pub ll_dict: *mut dict_T,
    pub ll_di: *mut dictitem_T,
    pub ll_newkey: Option<std::ffi::CString>,
    pub ll_blob: *mut blob_T,
}

impl lval_T {
    pub const fn new() -> Self {
        Self {
            ll_name: ptr::null(),
            ll_name_len: 0,
            ll_exp_name: None,
            ll_tv: ptr::null_mut(),
            ll_li: ptr::null_mut(),
            ll_list: ptr::null_mut(),
            ll_range: false,
            ll_empty2: false,
            ll_n1: 0,
            ll_n2: 0,
            ll_dict: ptr::null_mut(),
            ll_di: ptr::null_mut(),
            ll_newkey: None,
            ll_blob: ptr::null_mut(),
        }
    }
}

impl Default for lval_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) const MAXCOL: varnumber_T = 0x7fffffff;

const VV_COMPAT: c_char = 1;
const VV_RO: c_char = 2;
const VV_RO_SBX: c_char = 4;

const DI_FLAGS_RO_SBX: u8 = 2;

pub(crate) const VAR_TYPE_NUMBER: varnumber_T = 0;
pub(crate) const VAR_TYPE_STRING: varnumber_T = 1;
pub(crate) const VAR_TYPE_FUNC: varnumber_T = 2;
pub(crate) const VAR_TYPE_LIST: varnumber_T = 3;
pub(crate) const VAR_TYPE_DICT: varnumber_T = 4;
pub(crate) const VAR_TYPE_FLOAT: varnumber_T = 5;
pub(crate) const VAR_TYPE_BOOL: varnumber_T = 6;
pub(crate) const VAR_TYPE_SPECIAL: varnumber_T = 7;
pub(crate) const VAR_TYPE_BLOB: varnumber_T = 10;

pub(crate) const ENV_STARTREASON: &CStr = c"__NVIM_STARTREASON";

pub(crate) const SID_STR: scid_T = -10;

pub(crate) const FORCE_BIN: c_int = 1;
pub(crate) const FORCE_NOBIN: c_int = 2;
pub(crate) const BAD_KEEP: c_int = -1;
pub(crate) const BAD_DROP: c_int = -2;

const kOptCharconvert: OptIndex = 36;
const kOptDiffexpr: OptIndex = 72;
const kOptPatchexpr: OptIndex = 217;
const kOptSpellsuggest: OptIndex = 292;
pub(crate) const kOptFlagFunc: u32 = 1 << 24;

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
struct VimVar {
    vv_name: &'static CStr,
    vv_di: dictitem_T,
    vv_flags: c_char,
}

const fn make_vimvar(name: &'static CStr, v_type: VarType, flags: c_char) -> VimVar {
    VimVar {
        vv_name: name,
        vv_di: dictitem_T {
            di_tv: typval_T {
                v_type,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
            di_flags: 0,
            di_key: Vec::new(),
        },
        vv_flags: flags,
    }
}

const fn empty_dict() -> dict_T {
    dict_T {
        dv_lock: 0,
        dv_scope: 0,
        dv_refcount: 0,
        dv_copyID: 0,
        dv_hashtab: hashtab_T::new(),
        dv_copydict: ptr::null_mut(),
        dv_gc_idx: None,
        watchers: Vec::new(),
        lua_table_ref: 0,
    }
}

const fn empty_scope_var() -> ScopeDictDictItem {
    ScopeDictDictItem {
        di_tv: typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
        di_flags: 0,
        di_key: Vec::new(),
    }
}

struct VarsModuleState {
    globvars_var: ScopeDictDictItem,
    globvardict: dict_T,
    compat_hashtab: hashtab_T,
    vimvars_var: ScopeDictDictItem,
    vimvardict: dict_T,
    vimvars: [VimVar; VV_LEN],
    varnamebuf: Vec<c_char>,
    redir_lval: Option<lval_T>,
    redir_buf: Vec<c_char>,
    redir_endp: *mut c_char,
    redir_varname: Option<std::ffi::CString>,
    GDONE: usize,
    BDONE: usize,
    WDONE: usize,
    TDONE: usize,
    VIDX: usize,
    HI: *mut hashitem_T,
}

impl VarsModuleState {
    const fn new() -> Self {
        Self {
            globvars_var: empty_scope_var(),
            globvardict: empty_dict(),
            compat_hashtab: hashtab_T::new(),
            vimvars_var: empty_scope_var(),
            vimvardict: empty_dict(),
            vimvars: [
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
],
            varnamebuf: Vec::new(),
            redir_lval: None,
            redir_buf: Vec::new(),
            redir_endp: ptr::null_mut(),
            redir_varname: None,
            GDONE: 0,
            BDONE: 0,
            WDONE: 0,
            TDONE: 0,
            VIDX: 0,
            HI: ptr::null_mut(),
        }
    }
}

static VARS_STATE: GlobalState<VarsModuleState> = GlobalState::new(VarsModuleState::new());

#[inline]
fn vars_st() -> &'static mut VarsModuleState {
    unsafe { &mut *VARS_STATE.as_ptr() }
}


const MSGPACK_TYPE_NAMES: [&CStr; NUM_MSGPACK_TYPES] = [
    c"nil", c"boolean", c"integer", c"float", c"string", c"array", c"map", c"ext",
];

static EVAL_MSGPACK_TYPE_LISTS: GlobalState<[*const list_T; NUM_MSGPACK_TYPES]> =
    GlobalState::new([ptr::null(); NUM_MSGPACK_TYPES]);

#[inline]
pub(crate) fn eval_msgpack_type_lists_mut() -> &'static mut [*const list_T; NUM_MSGPACK_TYPES] {
    unsafe { &mut *EVAL_MSGPACK_TYPE_LISTS.as_ptr() }
}



#[inline]
fn c_memchr(s: impl crate::strings::IntoCCharPtr, c: u8, n: usize) -> *const c_char { let s = s.into_c_char_ptr();
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
pub fn tv_init(tv: Option<&mut typval_T>) {
    if let Some(tv) = tv {
        *tv = typval_T::new();
    }
}

#[inline]
pub fn tv_list_locked(l: Option<&list_T>) -> VarLockStatus {
    l.map_or(VAR_FIXED, |l| l.lv_lock)
}

#[inline]
fn script_sv(id: scid_T) -> *mut scriptvar_T {
    unsafe { &raw mut *(*script_item(id)).sn_vars }
}

#[inline]
fn script_vars_ht(id: scid_T) -> *mut hashtab_T {
    unsafe { &raw mut (*script_sv(id)).sv_dict.dv_hashtab }
}

#[inline]
pub fn tristate_from_int(val: OptInt) -> TriState {
    if val == 0 {
        kFalse
    } else if val >= 1 {
        kTrue
    } else {
        kNone
    }
}

use crate::api::private::helpers::integer_obj;

pub fn evalvars_init() {
    unsafe {
        init_var_dict(&mut *get_globvar_dict(), &mut vars_st().globvars_var, VAR_DEF_SCOPE);
        init_var_dict(&mut vars_st().vimvardict, &mut vars_st().vimvars_var, VAR_SCOPE);
        vars_st().vimvardict.dv_lock = VAR_FIXED;
        hash_clear(&mut vars_st().compat_hashtab);

        let vimvars_ptr = (&raw mut vars_st().vimvars).cast::<VimVar>();
        for i in 0..VV_LEN {
            let p = vimvars_ptr.add(i);
            if ((*p).vv_flags & VV_RO) != 0 {
                (*p).vv_di.di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
            } else if ((*p).vv_flags & VV_RO_SBX) != 0 {
                (*p).vv_di.di_flags = DI_FLAGS_RO_SBX | DI_FLAGS_FIX;
            } else {
                (*p).vv_di.di_flags = DI_FLAGS_FIX;
            }

            let name_bytes = (*p).vv_name.to_bytes_with_nul();
            (*p).vv_di.di_key.clear();
            (*p).vv_di
                .di_key
                .extend(name_bytes.iter().map(|&b| b as c_char));
            if (*p).vv_di.di_tv.v_type != VarType::VAR_UNKNOWN {
                hash_add(&mut vars_st().vimvardict.dv_hashtab, &mut (*p).vv_di);
            }
            if ((*p).vv_flags & VV_COMPAT) != 0 {
                hash_add(&mut vars_st().compat_hashtab, &mut (*p).vv_di);
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
            tv_list_set_lock(type_list.as_mut(), VAR_FIXED);
            tv_list_ref(type_list.as_mut());
            let di = tv_dict_item_alloc(type_name.as_ptr());
            (*di).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            (*di).di_tv = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: 0,
                vval: TypvalVval::List(type_list),
            };
            *(&raw mut (*eval_msgpack_type_lists_mut()))
                .cast::<*const list_T>()
                .add(i) = type_list;
            if !tv_dict_add(&mut *msgpack_types_dict, di) {
                std::process::abort();
            }
        }
        (*msgpack_types_dict).dv_lock = VAR_FIXED;

        set_vim_var_dict(VV_MSGPACK_TYPES, msgpack_types_dict);
        set_vim_var_dict(VV_COMPLETED_ITEM, tv_dict_alloc_lock(VAR_FIXED));

        set_vim_var_dict(VV_EVENT, tv_dict_alloc_lock(VAR_FIXED));
        set_vim_var_list(VV_ERRORS, tv_list_alloc(kListLenUnknown));
        set_vim_var_nr(VV_STDERR, crate::channel::CHAN_STDERR as varnumber_T);
        set_vim_var_nr(VV_SEARCHFORWARD, 1);
        set_vim_var_nr(VV_HLSEARCH, 1);
        set_vim_var_nr(VV_COUNT1, 1);
        set_vim_var_string(VV_STARTREASON, c"normal", 6);
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

        set_vim_var_nr(VV_ECHOSPACE, varnumber_T::from(g_st().sc_col - 1));

        let mut vvlua_pt = partial_T::default();
        vvlua_pt.pt_name = Some(std::ffi::CString::default());
        vvlua_pt.pt_refcount = 1;
        let vvlua_partial = Box::into_raw(Box::new(vvlua_pt));
        set_vim_var_partial(VV_LUA, vvlua_partial);

        set_reg_var(0);

        let mut startreason_buf = [0 as c_char; 32];
        let startreason = os_getenv_buf(
            ENV_STARTREASON,
            startreason_buf.as_mut_ptr(),
            startreason_buf.len(),
        );
        if (crate::memory::cstr_as_opt_bytes(startreason) == Some(b"restart!")) || (crate::memory::cstr_as_opt_bytes(startreason) == Some(b"restart"))
        {
            set_vim_var_string(VV_STARTREASON, startreason, -1);
        }
        if os_env_exists(ENV_STARTREASON, false) {
            os_unsetenv(ENV_STARTREASON);
        }
    }
}

pub fn evalvars_clear() {
    unsafe {
        let vimvars_ptr = (&raw mut vars_st().vimvars).cast::<VimVar>();
        for i in 0..VV_LEN {
            let p = vimvars_ptr.add(i);
            if (*p).vv_di.di_tv.v_type == VarType::VAR_STRING
                || (*p).vv_di.di_tv.v_type == VarType::VAR_LIST
            {
                tv_clear(&mut (*p).vv_di.di_tv);
            }
            (*p).vv_di.di_key = Vec::new();
        }

        partial_unref(get_vim_var_partial(VV_LUA));
        set_vim_var_partial(VV_LUA, ptr::null_mut());
        hash_clear(&mut vars_st().vimvardict.dv_hashtab);
        hash_clear(&mut vars_st().compat_hashtab);

        vars_clear(&mut *get_globvar_ht());

        for i in 1..=crate::runtime::script_items_len() {
            vars_clear(&mut *script_vars_ht(i));
        }
    }
}

pub fn garbage_collect_globvars(copyID: c_int) -> c_int {
    c_int::from(set_ref_in_ht(
            &raw mut vars_st().globvardict.dv_hashtab,
            copyID,
            ptr::null_mut(),
        ))
}

pub fn garbage_collect_vimvars(copyID: c_int) -> bool {
    set_ref_in_ht(&raw mut vars_st().vimvardict.dv_hashtab, copyID, ptr::null_mut())
}

pub fn garbage_collect_scriptvars(copyID: c_int) -> bool {
    let mut abort = false;

        for i in 1..=crate::runtime::script_items_len() {
            abort = abort || set_ref_in_ht(script_vars_ht(i), copyID, ptr::null_mut());
        }

        abort
}

/// Set an internal variable to a string value.
pub fn set_internal_string_var(name: &std::ffi::CStr, value: impl crate::strings::IntoCCharPtr) { let value = value.into_c_char_ptr();
     {
        let mut tv = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval::String(value.cast_mut()),
        };

        set_var(name.as_ptr(), name.to_bytes().len(), &mut tv, true);
    }
}

pub fn eval_charconvert(
    enc_from: impl crate::strings::IntoCCharPtr,
    enc_to: impl crate::strings::IntoCCharPtr,
    fname_from: impl crate::strings::IntoCCharPtr,
    fname_to: impl crate::strings::IntoCCharPtr,
) -> bool { let fname_from = fname_from.into_c_char_ptr(); let fname_to = fname_to.into_c_char_ptr();
    let enc_from = enc_from.into_c_char_ptr();
    let enc_to = enc_to.into_c_char_ptr();
    unsafe {
        let saved_sctx = g_st().current_sctx;

        set_vim_var_string(VV_CC_FROM, enc_from, -1);
        set_vim_var_string(VV_CC_TO, enc_to, -1);
        set_vim_var_string(VV_FNAME_IN, fname_from, -1);
        set_vim_var_string(VV_FNAME_OUT, fname_to, -1);
        let ctx = get_option_sctx(kOptCharconvert);
        if !ctx.is_null() {
            g_st().current_sctx = *ctx;
        }

        let mut err = false;
        let mut tv: typval_T = typval_T::new();
        let mut argv: [typval_T; 1] = [typval_T::new()];
        if !callback_call(&mut opt_vars().p_ccv, 0, argv.as_mut_ptr(), &mut tv) {
            err = true;
        } else {
            if tv_get_number_chk(&tv, None) != 0 {
                err = true;
            }
            tv_clear(&mut tv);
        }

        set_vim_var_string(VV_CC_FROM, ptr::null(), -1);
        set_vim_var_string(VV_CC_TO, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        g_st().current_sctx = saved_sctx;

        !err
    }
}

pub fn eval_diff(origfile: impl crate::strings::IntoCCharPtr, newfile: impl crate::strings::IntoCCharPtr, outfile: impl crate::strings::IntoCCharPtr) { let origfile = origfile.into_c_char_ptr(); let newfile = newfile.into_c_char_ptr(); let outfile = outfile.into_c_char_ptr();
    unsafe {
        let saved_sctx = g_st().current_sctx;
        set_vim_var_string(VV_FNAME_IN, origfile, -1);
        set_vim_var_string(VV_FNAME_NEW, newfile, -1);
        set_vim_var_string(VV_FNAME_OUT, outfile, -1);

        let ctx = get_option_sctx(kOptDiffexpr);
        if !ctx.is_null() {
            g_st().current_sctx = *ctx;
        }

        let mut tv: typval_T = typval_T::new();
        let mut argv: [typval_T; 1] = [typval_T::new()];
        if callback_call(&mut opt_vars().p_dex, 0, argv.as_mut_ptr(), &mut tv) {
            tv_clear(&mut tv);
        }

        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_NEW, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        g_st().current_sctx = saved_sctx;
    }
}

pub fn eval_patch(origfile: impl crate::strings::IntoCCharPtr, difffile: impl crate::strings::IntoCCharPtr, outfile: impl crate::strings::IntoCCharPtr) { let origfile = origfile.into_c_char_ptr(); let difffile = difffile.into_c_char_ptr(); let outfile = outfile.into_c_char_ptr();
    unsafe {
        let saved_sctx = g_st().current_sctx;
        set_vim_var_string(VV_FNAME_IN, origfile, -1);
        set_vim_var_string(VV_FNAME_DIFF, difffile, -1);
        set_vim_var_string(VV_FNAME_OUT, outfile, -1);

        let ctx = get_option_sctx(kOptPatchexpr);
        if !ctx.is_null() {
            g_st().current_sctx = *ctx;
        }

        let mut tv: typval_T = typval_T::new();
        let mut argv: [typval_T; 1] = [typval_T::new()];
        if callback_call(&mut opt_vars().p_pex, 0, argv.as_mut_ptr(), &mut tv) {
            tv_clear(&mut tv);
        }

        set_vim_var_string(VV_FNAME_IN, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_DIFF, ptr::null(), -1);
        set_vim_var_string(VV_FNAME_OUT, ptr::null(), -1);
        g_st().current_sctx = saved_sctx;
    }
}

/// Evaluate an expression to a list with suggestions.
pub fn eval_spell_expr(badword: impl crate::strings::IntoCCharPtr, expr: impl crate::strings::IntoCCharPtr) -> *mut list_T { let badword = badword.into_c_char_ptr(); let expr = expr.into_c_char_ptr();
    unsafe {
        let mut save_val: typval_T = typval_T::new();
        let mut rettv: typval_T = typval_T::new();
        let mut list: *mut list_T = ptr::null_mut();
        let mut p = skipwhite(expr);
        let saved_sctx = g_st().current_sctx;

        prepare_vimvar(VV_VAL, &mut save_val);
        set_vim_var_string(VV_VAL, badword, -1);
        if opt_vars().p_verbose == 0 {
            g_st().emsg_off += 1;
        }
        let ctx = get_option_sctx(kOptSpellsuggest);
        if !ctx.is_null() {
            g_st().current_sctx = *ctx;
        }

        let mut r = may_call_simple_func(p, &mut rettv);
        if r == NOTDONE {
            r = if eval1(&raw mut p, &mut rettv, Some(&mut g_st().EVALARG_EVALUATE)) {
                OK
            } else {
                FAIL
            };
        }
        if r == OK {
            if rettv.v_type != VarType::VAR_LIST {
                tv_clear(&mut rettv);
            } else {
                list = rettv.vval.v_list();
            }
        }

        if opt_vars().p_verbose == 0 {
            g_st().emsg_off -= 1;
        }
        tv_clear(get_vim_var_tv(VV_VAL));
        restore_vimvar(VV_VAL, &mut save_val);
        g_st().current_sctx = saved_sctx;

        list
    }
}

/// Get spell word from an entry from spellsuggest=expr:
pub fn get_spellword(list: &mut list_T, ret_word: *mut *const c_char) -> c_int {
    unsafe {
        if tv_list_len(Some(list)) != 2 {
            emsg(gettext(
                c"E5700: Expression from 'spellsuggest' must yield lists with exactly two values",
            ));
            return -1;
        }
        *ret_word = tv_list_find_str(Some(&mut *list), 0);
        if (*ret_word).is_null() {
            return -1;
        }
        tv_list_find_nr(Some(&mut *list), -1, None) as c_int
    }
}

/// Prepare v: variable "idx" to be used.
pub fn prepare_vimvar(idx: c_int,save_tv: &mut typval_T) {
    unsafe {
        let p = (&raw mut vars_st().vimvars).cast::<VimVar>().add(idx as usize);
        *save_tv = (*p).vv_di.di_tv;
        (*p).vv_di.di_tv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        if (*p).vv_di.di_tv.v_type == VarType::VAR_UNKNOWN {
            hash_add(&mut vars_st().vimvardict.dv_hashtab, &mut (*p).vv_di);
        }
    }
}

/// Restore v: variable "idx" to typeval "save_tv".
pub fn restore_vimvar(idx: c_int,save_tv: &mut typval_T) {
    unsafe {
        let p = (&raw mut vars_st().vimvars).cast::<VimVar>().add(idx as usize);
        (*p).vv_di.di_tv = *save_tv;
        if (*p).vv_di.di_tv.v_type != VarType::VAR_UNKNOWN {
            return;
        }

        let hi = hash_find(&vars_st().vimvardict.dv_hashtab, (*p).vv_di.di_key.as_ptr());
        if hashitem_empty(&*hi) {
            internal_error(c"restore_vimvar()");
        } else {
            hash_remove(&mut vars_st().vimvardict.dv_hashtab,  &mut *hi);
        }
    }
}

/// List Vim variables.
fn list_vim_vars(first: &mut c_int) {
    list_hashtable_vars(
            &mut vars_st().vimvardict.dv_hashtab,
            c"v:",
            false as c_int,
            &mut *first,
        );
}

/// List script-local variables, if there is a script.
fn list_script_vars(first: &mut c_int) {
    unsafe {
        if crate::runtime::script_id_valid(g_st().current_sctx.sc_sid) {
            list_hashtable_vars(
                &mut *script_vars_ht(g_st().current_sctx.sc_sid),
                c"s:",
                false as c_int,
                &mut *first,
            );
        }
    }
}

/// Evaluate one Vim expression {expr} in string "p" and append the resulting string to "gap".
pub fn eval_one_expr_in_str(
    p: *mut c_char,
    gap: &mut Vec<c_char>,
    evaluate: bool,
) -> *mut c_char {
    unsafe {
        let block_start = skipwhite(p.add(1));
        let mut block_end = block_start;

        if crate::memory::cstr_is_empty(block_start) {
            semsg!(gettext(e_missing_close_curly_str), p);
            return ptr::null_mut();
        }
        if !skip_expr(&raw mut block_end, None) {
            return ptr::null_mut();
        }
        block_end = skipwhite(block_end);
        if crate::memory::cstr_first_byte(block_end) != b'}' {
            semsg!(gettext(e_missing_close_curly_str), p);
            return ptr::null_mut();
        }
        if evaluate {
            *block_end = NUL;
            let expr_val = eval_to_string(block_start, false, false);
            *block_end = b'}' as c_char;
            let Some(expr_val) = expr_val else {
                return ptr::null_mut();
            };
            let bytes = expr_val.as_bytes();
            if !bytes.is_empty() {
                gap.extend_from_slice(std::slice::from_raw_parts(
                    bytes.as_ptr().cast::<c_char>(),
                    bytes.len(),
                ));
            }
        }

        block_end.add(1)
    }
}

/// Evaluate all the Vim expressions {expr} in "str" and return the resulting string as an owned `CString`.
fn eval_all_expr_in_str(str_arg: *mut c_char) -> Option<std::ffi::CString> {
    unsafe {
        let mut buf: Vec<c_char> = Vec::with_capacity(80);
        let mut p = str_arg;

        while !crate::memory::cstr_is_empty(p) {
            let mut escaped_brace = false;

            let lit_start = p;
            while crate::memory::cstr_first_byte(p) != b'{' && crate::memory::cstr_first_byte(p) != b'}' && !crate::memory::cstr_is_empty(p) {
                p = p.add(1);
            }

            if !crate::memory::cstr_is_empty(p) && *p == *p.add(1) {
                p = p.add(1);
                escaped_brace = true;
            } else if crate::memory::cstr_first_byte(p) == b'}' {
                semsg!(gettext(e_stray_closing_curly_str), str_arg);
                return None;
            }

            let lit_len = p.offset_from(lit_start) as usize;
            if lit_len > 0 {
                buf.extend_from_slice(std::slice::from_raw_parts(lit_start, lit_len));
            }

            if crate::memory::cstr_is_empty(p) {
                break;
            }

            if escaped_brace {
                p = p.add(1);
                continue;
            }

            p = eval_one_expr_in_str(p, &mut buf, true);
            if p.is_null() {
                return None;
            }
        }

        let bytes = crate::memory::cstr_slice(buf.as_ptr(), buf.len());
        Some(cstr_from_bytes(bytes))
    }
}

/// Get a list of lines from a HERE document.
pub fn heredoc_get(
    eap: &mut exarg_T,
    cmd: impl crate::strings::IntoCCharPtr,
    script_get: bool,
) -> *mut list_T { let mut cmd = cmd.into_c_char_ptr();
    unsafe {
        let mut marker_indent_len: c_int = 0;
        let mut text_indent_len: c_int = 0;
        let mut text_indent: Option<Vec<c_char>> = None;
        let mut dot: [c_char; 2] = [b'.' as c_char, NUL];
        let mut heredoc_in_string = false;
        let mut line_arg: *mut c_char = ptr::null_mut();
        let nl_ptr = vim_strchr(cmd, b'\n' as c_int);

        if !nl_ptr.is_null() {
            heredoc_in_string = true;
            line_arg = nl_ptr.add(1);
            *nl_ptr = NUL;
        } else if eap.ea_getline.is_none() {
            emsg(gettext(E_CANNOT_USE_HEREDOC_HERE));
            return ptr::null_mut();
        }

        cmd = skipwhite(cmd);
        let mut evalstr = false;
        let mut eval_failed = false;
        loop {
            if (crate::memory::cstr_prefix_bytes(cmd, 4) == &b"trim"[..4])
                && (crate::memory::cstr_byte_at(cmd, 4) == 0 || ascii_iswhite(*cmd.add(4) as c_int))
            {
                cmd = skipwhite(cmd.add(4));

                let mut p = *eap.cmdlinep;
                while ascii_iswhite(*p as c_int) {
                    p = p.add(1);
                    marker_indent_len += 1;
                }
                text_indent_len = -1;
                continue;
            }
            if (crate::memory::cstr_prefix_bytes(cmd, 4) == &b"eval"[..4])
                && (crate::memory::cstr_byte_at(cmd, 4) == 0 || ascii_iswhite(*cmd.add(4) as c_int))
            {
                cmd = skipwhite(cmd.add(4));
                evalstr = true;
                continue;
            }
            break;
        }

        let comment_char = b'"' as c_char;
        let marker: *mut c_char;
        if !crate::memory::cstr_is_empty(cmd) && *cmd != comment_char {
            marker = skipwhite(cmd);
            let p = skiptowhite(marker);
            if *skipwhite(p) != NUL && *skipwhite(p) != comment_char {
                semsg!(gettext(e_trailing_arg), p);
                return ptr::null_mut();
            }
            *p = NUL;
            if !script_get && (crate::memory::cstr_first_byte(marker)).is_ascii_lowercase() {
                emsg(gettext(
                    c"E221: Marker cannot start with lower case letter",
                ));
                return ptr::null_mut();
            }
        } else if script_get {
            marker = dot.as_mut_ptr();
        } else {
            emsg(gettext(c"E172: Missing marker"));
            return ptr::null_mut();
        }

        let mut theline_owned: Option<std::ffi::CString> = None;
        let l = tv_list_alloc(0);
        loop {
            let mut mi: c_int = 0;
            let mut ti: c_int = 0;

            let theline: *mut c_char = if heredoc_in_string {
                if crate::memory::cstr_is_empty(line_arg) {
                    if !script_get {
                        semsg!(gettext(E_MISSING_END_MARKER_STR), marker);
                    }
                    break;
                }

                let cur_line = line_arg;
                let next_line = vim_strchr(cur_line, b'\n' as c_int);
                if next_line.is_null() {
                    line_arg = line_arg.add(crate::memory::cstr_len(line_arg));
                } else {
                    *next_line = NUL;
                    line_arg = next_line.add(1);
                }
                cur_line
            } else {
                theline_owned = eap.ea_getline.call(0, 0, false);
                let Some(ref tl) = theline_owned else {
                    if !script_get {
                        semsg!(gettext(E_MISSING_END_MARKER_STR), marker);
                    }
                    break;
                };
                tl.as_ptr().cast_mut()
            };

            if marker_indent_len > 0
                && (crate::memory::cstr_prefix_bytes(theline, marker_indent_len as usize) == crate::memory::cstr_prefix_bytes(*eap.cmdlinep, marker_indent_len as usize))
            {
                mi = marker_indent_len;
            }
            if crate::memory::cstr_as_bytes(marker) == crate::memory::cstr_as_bytes(theline.add(mi as usize)) {
                break;
            }

            if eval_failed {
                continue;
            }

            if text_indent_len == -1 && !crate::memory::cstr_is_empty(theline) {
                let mut p = theline;
                text_indent_len = 0;
                while ascii_iswhite(*p as c_int) {
                    p = p.add(1);
                    text_indent_len += 1;
                }
                text_indent = Some(
                    std::slice::from_raw_parts(theline, text_indent_len as usize).to_vec(),
                );
            }
            if let Some(ref indent_slice) = text_indent {
                while ti < text_indent_len {
                    if *theline.add(ti as usize) != indent_slice[ti as usize] {
                        break;
                    }
                    ti += 1;
                }
            }

            let str_line = theline.add(ti as usize);
            if evalstr && eap.skip == 0 {
                let Some(evaluated) = eval_all_expr_in_str(str_line) else {
                    eval_failed = true;
                    continue;
                };
                tv_list_append_string( &mut *l, evaluated.as_ptr(), -1);
            } else {
                tv_list_append_string( &mut *l, str_line, -1);
            }
        }
        if heredoc_in_string {
            eap.nextcmd = line_arg;
        }
        drop(theline_owned);
        drop(text_indent);

        if eval_failed {
            tv_list_free(l);
            return ptr::null_mut();
        }
        l
    }
}

/// ":let" and ":const" commands.
pub fn ex_let(eap: &mut exarg_T) {
    unsafe {
        let is_const = eap.cmdidx == cmdidx_T::CMD_const as c_int;
        let mut arg = eap.arg;
        let mut rettv: typval_T = typval_T::new();
        let mut var_count: c_int = 0;
        let mut semicolon: c_int = 0;
        let mut op: [c_char; 2] = [0; 2];
        let mut first: c_int = 1;

        let argend = skip_var_list(arg, &mut var_count, &mut semicolon, false);
        if argend.is_null() {
            return;
        }
        let mut expr = skipwhite(argend);
        let concat = crate::memory::cstr_prefix_bytes(expr, 3) == &b"..="[..3];
        let has_assign = crate::memory::cstr_first_byte(expr) == b'='
            || (!vim_strchr(c"+-*/%.", crate::memory::cstr_first_byte(expr) as c_int).is_null()
                && crate::memory::cstr_byte_at(expr, 1) == b'=');
        if !has_assign && !concat {
            if crate::memory::cstr_first_byte(arg) == b'[' {
                emsg(gettext(e_invarg));
            } else if ends_excmd(*arg as c_int) == 0 {
                arg = list_arg_vars(eap, arg, &mut first).cast_mut();
            } else if eap.skip == 0 {
                list_glob_vars(&mut first);
                list_buf_vars(&mut first);
                list_win_vars(&mut first);
                list_tab_vars(&mut first);
                list_script_vars(&mut first);
                list_func_vars(&mut first);
                list_vim_vars(&mut first);
            }
            eap.nextcmd = check_nextcmd(arg);
            return;
        }

        if crate::memory::cstr_first_byte(expr) == b'='
            && crate::memory::cstr_byte_at(expr, 1) == b'<'
            && crate::memory::cstr_byte_at(expr, 2) == b'<'
        {
            let l = heredoc_get(eap, expr.add(3), false);
            if !l.is_null() {
                tv_list_set_ret(&mut rettv, l);
                if eap.skip == 0 {
                    op[0] = b'=' as c_char;
                    op[1] = NUL;
                    ex_let_vars(
                        eap.arg,
                        &mut rettv,
                        0,
                        semicolon,
                        var_count,
                        c_int::from(is_const),
                        op.as_mut_ptr(),
                    );
                }
                tv_clear(&mut rettv);
            }
            return;
        }

        rettv.v_type = VarType::VAR_UNKNOWN;

        op[0] = b'=' as c_char;
        op[1] = NUL;
        if crate::memory::cstr_first_byte(expr) != b'=' {
            if !vim_strchr(c"+-*/%.", crate::memory::cstr_first_byte(expr) as c_int).is_null() {
                op[0] = *expr;
                if crate::memory::cstr_first_byte(expr) == b'.' && crate::memory::cstr_byte_at(expr, 1) == b'.' {
                    expr = expr.add(1);
                }
            }
            expr = expr.add(2);
        } else {
            expr = expr.add(1);
        }

        expr = skipwhite(expr);

        if eap.skip != 0 {
            g_st().emsg_skip += 1;
        }
        let mut evalarg = evalarg_T::new();
        fill_evalarg_from_eap(&mut evalarg, eap.skip != 0);
        let eval_res = eval0(expr, &mut rettv, Some(&mut *eap), &mut evalarg);
        if eap.skip != 0 {
            g_st().emsg_skip -= 1;
        }

        if eap.skip == 0 && eval_res {
            ex_let_vars(
                eap.arg,
                &mut rettv,
                0,
                semicolon,
                var_count,
                c_int::from(is_const),
                op.as_mut_ptr(),
            );
        }
        if eval_res {
            tv_clear(&mut rettv);
        }
    }
}

/// Assign the typevalue "tv" to the variable or variables at "arg_start".
pub fn ex_let_vars(
    arg_start: *mut c_char,
    tv: &mut typval_T,
    copy: c_int,
    semicolon: c_int,
    var_count: c_int,
    is_const: c_int,
    op: *mut c_char,
) -> bool {
    unsafe {
        let mut arg = arg_start;
        let mut ltv: typval_T = typval_T::new();

        if crate::memory::cstr_first_byte(arg) != b'[' {
            if ex_let_one(arg, &mut *tv, copy != 0, is_const != 0, op, op).is_null() {
                return false;
            }
            return true;
        }

        if tv.v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq));
            return false;
        }
        let l = tv.vval.v_list();

        let len = tv_list_len(l.as_ref());
        if semicolon == 0 && var_count < len {
            emsg(gettext(c"E687: Less targets than List items"));
            return false;
        }
        if var_count - semicolon > len {
            emsg(gettext(c"E688: More targets than List items"));
            return false;
        }
        debug_assert!(!l.is_null());

        let mut item = tv_list_first(l.as_ref());
        let mut rest_len = tv_list_len(l.as_ref()) as usize;
        while crate::memory::cstr_first_byte(arg) != b']' {
            arg = skipwhite(arg.add(1));
            arg = ex_let_one(
                arg,
                &mut (*item).li_tv,
                true,
                is_const != 0,
                c",;]",
                op,
            );
            if arg.is_null() {
                return false;
            }
            rest_len -= 1;

            item = (*item).li_next;
            arg = skipwhite(arg);
            if crate::memory::cstr_first_byte(arg) == b';' {
                let rest_list = tv_list_alloc(rest_len as isize);
                while !item.is_null() {
                    tv_list_append_tv( &mut *rest_list, &mut (*item).li_tv);
                    item = (*item).li_next;
                }

                ltv.v_type = VarType::VAR_LIST;
                ltv.v_lock = VAR_UNLOCKED;
                ltv.vval = crate::eval::typval::TypvalVval::List(rest_list);
                tv_list_ref(rest_list.as_mut());

                arg = ex_let_one(
                    skipwhite(arg.add(1)),
                    &mut ltv,
                    false,
                    is_const != 0,
                    c"]",
                    op,
                );
                tv_clear(&mut ltv);
                if arg.is_null() {
                    return false;
                }
                break;
            } else if crate::memory::cstr_first_byte(arg) != b',' && crate::memory::cstr_first_byte(arg) != b']' {
                internal_error(c"ex_let_vars()");
                return false;
            }
        }

        true
    }
}

/// Skip over assignable variable "var" or list of variables "[var, var]".
pub fn skip_var_list(
    arg: impl crate::strings::IntoCCharPtr,var_count: &mut c_int,semicolon: &mut c_int,
    silent: bool,
) -> *const c_char { let arg = arg.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_first_byte(arg) == b'[' {
            let mut p = arg;
            loop {
                p = skipwhite(p.add(1));
                let s = skip_var_one(p);
                if s == p {
                    if !silent {
                        semsg!(gettext(e_invarg2), p);
                    }
                    return ptr::null();
                }
                *var_count += 1;

                p = skipwhite(s);
                if crate::memory::cstr_first_byte(p) == b']' {
                    break;
                } else if crate::memory::cstr_first_byte(p) == b';' {
                    if *semicolon == 1 {
                        if !silent {
                            emsg(gettext(E_DOUBLE_SEMICOLON_IN_LIST_OF_VARIABLES));
                        }
                        return ptr::null();
                    }
                    *semicolon = 1;
                } else if crate::memory::cstr_first_byte(p) != b',' {
                    if !silent {
                        semsg!(gettext(e_invarg2), p);
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
fn skip_var_one(arg: impl crate::strings::IntoCCharPtr) -> *const c_char { let arg = arg.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_first_byte(arg) == b'@' && crate::memory::cstr_byte_at(arg, 1) != 0 {
            return arg.add(2);
        }
        find_name_end(
            if crate::memory::cstr_first_byte(arg) == b'$' || crate::memory::cstr_first_byte(arg) == b'&' {
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
pub fn list_hashtable_vars(
    ht: &mut hashtab_T,
    prefix: &std::ffi::CStr,
    empty: c_int,first: &mut c_int,
) {
    unsafe {
        let mut todo = ht.ht_used as c_int;
        let mut hi = ht.array_ptr();
        while todo > 0 && !g_st().got_int {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                let mut buf = [0 as c_char; IOSIZE];

                xstrlcpy(buf.as_mut_ptr(), prefix.as_ptr(), IOSIZE);
                { let _dst: *mut c_char = buf.as_mut_ptr(); let _dsize: usize = IOSIZE; let _dlen = crate::memory::cstr_len(_dst); crate::memory::xstrlcpy(_dst.add(_dlen), (*di).di_key.as_ptr(), _dsize - _dlen); };
                if message_filtered(buf.as_ptr()) {
                    hi = hi.add(1);
                    continue;
                }

                if empty != 0
                    || (*di).di_tv.v_type != VarType::VAR_STRING
                    || !(*di).di_tv.vval.v_string().is_null()
                {
                    list_one_var(&mut *di, prefix.as_ptr(), first);
                }
            }
            hi = hi.add(1);
        }
    }
}

fn list_glob_vars(first: &mut c_int) {
    list_hashtable_vars(
            &mut vars_st().globvardict.dv_hashtab,
            c"",
            true as c_int,
            &mut *first,
        );
}

fn list_buf_vars(first: &mut c_int) {
    unsafe {
        list_hashtable_vars(
            &mut (*curbuf_mut().b_vars()).dv_hashtab,
            c"b:",
            true as c_int,
            &mut *first,
        );
    }
}

fn list_win_vars(first: &mut c_int) {
    unsafe {
        list_hashtable_vars(
            &mut (*curwin_mut().w_vars()).dv_hashtab,
            c"w:",
            true as c_int,
            &mut *first,
        );
    }
}

fn list_tab_vars(first: &mut c_int) {
    unsafe {
        list_hashtable_vars(
            &mut (*curtab_mut().tp_vars()).dv_hashtab,
            c"t:",
            true as c_int,
            &mut *first,
        );
    }
}

/// List variables in "arg".
fn list_arg_vars(
    eap: &mut exarg_T,
    arg: impl crate::strings::IntoCCharPtr,
    first: &mut c_int,
) -> *const c_char { let mut arg = arg.into_c_char_ptr();
    unsafe {
        let mut error = false;
        let mut tv: typval_T = typval_T::new();

        while ends_excmd(*arg as c_int) == 0 && !g_st().got_int {
            if error || eap.skip != 0 {
                arg = find_name_end(
                    arg,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    FNE_INCL_BR | FNE_CHECK_START,
                );
                if !ascii_iswhite(*arg as c_int) && ends_excmd(*arg as c_int) == 0 {
                    g_st().emsg_severe = true;
                    semsg!(gettext(e_trailing_arg), arg);
                    break;
                }
            } else {
                let name_start = arg;
                let mut name = arg;
                let mut tofree: Option<std::ffi::CString> = None;
                let len = get_name_len((&raw mut arg).cast(), &mut tofree, true, true);
                if len <= 0 {
                    if len < 0 && !aborting() {
                        g_st().emsg_severe = true;
                        semsg!(gettext(e_invarg2), arg);
                        break;
                    }
                    error = true;
                } else {
                    if let Some(ref tf) = tofree {
                        name = tf.as_ptr().cast_mut();
                    }
                    if !eval_variable(name, len, Some(&mut tv), ptr::null_mut(), true, false) {
                        error = true;
                    } else {
                        let arg_subsc = arg;
                        if !handle_subscript(
                            (&raw mut arg).cast(),
                            &mut tv,
                            Some(&mut g_st().EVALARG_EVALUATE),
                            true,
                        )
                        {
                            error = true;
                        } else {
                            if arg == arg_subsc && len == 2 && crate::memory::cstr_byte_at(name, 1) == b':' {
                                match crate::memory::cstr_first_byte(name) {
                                    b'g' => list_glob_vars(first),
                                    b'b' => list_buf_vars(first),
                                    b'w' => list_win_vars(first),
                                    b't' => list_tab_vars(first),
                                    b'v' => list_vim_vars(first),
                                    b's' => list_script_vars(first),
                                    b'l' => list_func_vars(first),
                                    _ => {
                                        semsg!(
                                            gettext(c"E738: Can't list variables for %s"),
                                            name,
                                        );
                                    }
                                }
                            } else {
                                let s = encode_tv2echo(&mut tv, None);
                                let used_name = if arg == arg_subsc { name } else { name_start };
                                debug_assert!(!used_name.is_null());
                                let name_size = if tofree
                                    .as_ref()
                                    .is_some_and(|tf| ptr::eq(used_name, tf.as_ptr()))
                                {
                                    crate::memory::cstr_len(used_name) as isize
                                } else {
                                    arg.offset_from(used_name)
                                };
                                list_one_var_a(
                                    c"",
                                    used_name,
                                    name_size,
                                    tv.v_type,
                                    s.as_ptr(),
                                    first,
                                );
                            }
                            tv_clear(&mut tv);
                        }
                    }
                }

                drop(tofree);
            }

            arg = skipwhite(arg);
        }

        arg
    }
}

/// Set an environment variable, part of ex_let_one().
fn ex_let_env(
    mut arg: *mut c_char,tv: &mut typval_T,
    is_const: bool,
    endchars: impl crate::strings::IntoCCharPtr,
    op: impl crate::strings::IntoCCharPtr,
) -> *mut c_char { let endchars = endchars.into_c_char_ptr(); let op = op.into_c_char_ptr();
    unsafe {
        if is_const {
            emsg(gettext(
                c"E996: Cannot lock an environment variable",
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
            semsg!(gettext(e_invarg2), name.sub(1));
        } else if !op.is_null() && !vim_strchr(c"+-*/%", crate::memory::cstr_first_byte(op) as c_int).is_null() {
            semsg!(gettext(e_letwrong), op);
        } else if !endchars.is_null()
            && vim_strchr(endchars, *skipwhite(arg) as u8 as c_int).is_null()
        {
            emsg(gettext(E_LETUNEXP));
        } else if !check_secure() {
            let mut tofree: Option<std::ffi::CString> = None;
            let c1 = *name.add(len);
            *name.add(len) = NUL;
            let mut p = tv_get_string_chk(&*tv);
            if !p.is_null()
                && !op.is_null()
                && crate::memory::cstr_first_byte(op) == b'.'
                && let Some(s) = vim_getenv(name)
            {
                let mut bytes = Vec::from(s.as_bytes());
                bytes.extend_from_slice(crate::memory::cstr_as_bytes(p));
                let joined = crate::memory::cstr_from_bytes(&bytes);
                p = joined.as_ptr();
                tofree = Some(joined);
            }
            if !p.is_null() {
                vim_setenv_ext(name, p);
                arg_end = arg;
            }
            *name.add(len) = c1;
            drop(tofree);
        }
        arg_end
    }
}

/// Set an option, part of ex_let_one().
fn ex_let_option(
    arg: *mut c_char,tv: &mut typval_T,
    is_const: bool,
    endchars: impl crate::strings::IntoCCharPtr,
    op: impl crate::strings::IntoCCharPtr,
) -> *mut c_char { let endchars = endchars.into_c_char_ptr(); let op = op.into_c_char_ptr();
    unsafe {
        if is_const {
            emsg(gettext(c"E996: Cannot lock an option"));
            return ptr::null_mut();
        }

        let mut arg_end: *mut c_char = ptr::null_mut();
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;

        let mut arg_const: *const c_char = arg;
        let p = find_option_var_end(&raw mut arg_const, &mut opt_idx, &mut opt_flags)
            .cast_mut();
        let arg_name = arg_const.cast_mut();

        if p.is_null()
            || (!endchars.is_null() && vim_strchr(endchars, *skipwhite(p) as u8 as c_int).is_null())
        {
            emsg(gettext(E_LETUNEXP));
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
            semsg!(gettext(e_unknown_option2), arg_name);
            *p = c1;
            optval_free(curval);
            optval_free(newval);
            return arg_end;
        }
        if !op.is_null()
            && crate::memory::cstr_first_byte(op) != b'='
            && ((curval.obj_type != ObjectType::kObjectTypeString && crate::memory::cstr_first_byte(op) == b'.')
                || (curval.obj_type == ObjectType::kObjectTypeString && crate::memory::cstr_first_byte(op) != b'.'))
        {
            semsg!(gettext(e_letwrong), op);
            *p = c1;
            optval_free(curval);
            optval_free(newval);
            return arg_end;
        }

        let mut error = false;
        newval = opt_from_tv(tv, opt_idx, arg_name, &mut error);
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

        if !op.is_null() && crate::memory::cstr_first_byte(op) != b'=' {
            if !hidden && is_num {
                let cur_n: OptInt = if curval.obj_type == ObjectType::kObjectTypeInteger {
                    curval.data.integer()
                } else {
                    OptInt::from(curval.data.boolean())
                };
                let mut new_n: OptInt = if newval.obj_type == ObjectType::kObjectTypeInteger {
                    newval.data.integer()
                } else {
                    OptInt::from(newval.data.boolean())
                };

                match crate::memory::cstr_first_byte(op) {
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
                let curval_data = curval.data.string().data;
                let newval_data = newval.data.string().data;

                if !curval_data.is_null() && !newval_data.is_null() {
                    let newval_old = newval;
                    let joined = concat_str(curval_data, newval_data);
                    newval = crate::api::private::helpers::cstr_to_obj(joined.as_ptr());
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
fn ex_let_register(
    mut arg: *mut c_char,tv: &mut typval_T,
    is_const: bool,
    endchars: impl crate::strings::IntoCCharPtr,
    op: impl crate::strings::IntoCCharPtr,
) -> *mut c_char { let endchars = endchars.into_c_char_ptr(); let op = op.into_c_char_ptr();
    unsafe {
        if is_const {
            emsg(gettext(c"E996: Cannot lock a register"));
            return ptr::null_mut();
        }

        let mut arg_end: *mut c_char = ptr::null_mut();
        arg = arg.add(1);
        if !op.is_null() && !vim_strchr(c"+-*/%", crate::memory::cstr_first_byte(op) as c_int).is_null() {
            semsg!(gettext(e_letwrong), op);
        } else if !endchars.is_null()
            && vim_strchr(endchars, *skipwhite(arg.add(1)) as u8 as c_int).is_null()
        {
            emsg(gettext(E_LETUNEXP));
        } else {
            let mut ptofree: Option<std::ffi::CString> = None;
            let mut p = tv_get_string_chk(&*tv);
            if !p.is_null() && !op.is_null() && crate::memory::cstr_first_byte(op) == b'.' {
                let reg_c = if crate::memory::cstr_first_byte(arg) == b'@' {
                    b'"' as c_int
                } else {
                    crate::memory::cstr_first_byte(arg) as c_int
                };
                if let Some(s) = get_reg_contents_str(reg_c, kGRegExprSrc) {
                    let mut bytes = Vec::from(s.as_bytes());
                    bytes.extend_from_slice(crate::memory::cstr_as_bytes(p));
                    let joined = cstr_from_bytes(&bytes);
                    p = joined.as_ptr();
                    ptofree = Some(joined);
                }
            }
            if !p.is_null() {
                let reg_c = if crate::memory::cstr_first_byte(arg) == b'@' {
                    b'"' as c_int
                } else {
                    crate::memory::cstr_first_byte(arg) as c_int
                };
                write_reg_contents(reg_c, p, crate::memory::cstr_len(p) as isize, false as c_int);
                arg_end = arg.add(1);
            }
            drop(ptofree);
        }
        arg_end
    }
}

/// Set one item of `:let var = expr` or `:let [v1, v2] = list` to its value.
fn ex_let_one(
    arg: *mut c_char,tv: &mut typval_T,
    copy: bool,
    is_const: bool,
    endchars: impl crate::strings::IntoCCharPtr,
    op: impl crate::strings::IntoCCharPtr,
) -> *mut c_char { let op = op.into_c_char_ptr();
    let endchars = endchars.into_c_char_ptr();
    unsafe {
        let mut arg_end: *mut c_char = ptr::null_mut();

        if crate::memory::cstr_first_byte(arg) == b'$' {
            return ex_let_env(arg, tv, is_const, endchars, op);
        } else if crate::memory::cstr_first_byte(arg) == b'&' {
            return ex_let_option(arg, tv, is_const, endchars, op);
        } else if crate::memory::cstr_first_byte(arg) == b'@' {
            return ex_let_register(arg, tv, is_const, endchars, op);
        } else if eval_isnamec1(crate::memory::cstr_first_byte(arg) as c_int) || crate::memory::cstr_first_byte(arg) == b'{' {
            let mut lv: lval_T = lval_T::new();
            let p = get_lval(arg, Some(tv), &mut lv, false, false, 0, FNE_CHECK_START);
            if !p.is_null() && !lv.ll_name.is_null() {
                if !endchars.is_null()
                    && vim_strchr(endchars, *skipwhite(p) as u8 as c_int).is_null()
                {
                    emsg(gettext(E_LETUNEXP));
                } else {
                    set_var_lval(&mut lv, p, tv, copy, is_const, op);
                    arg_end = p;
                }
            }
        } else {
            semsg!(gettext(e_invarg2), arg);
        }

        arg_end
    }
}

/// ":unlet[!] var1 ... " command.
pub fn ex_unlet(eap: &mut exarg_T) {
    let arg = eap.arg;
        let forceit = eap.forceit;
        ex_unletlock(
            eap,
            arg,
            0,
            if forceit != 0 { GLV_QUIET } else { 0 },
            do_unlet_var,
        );
}

/// ":lockvar" and ":unlockvar" commands.
pub fn ex_lockvar(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        let mut deep: c_int = 2;

        if eap.forceit != 0 {
            deep = -1;
        } else if ascii_isdigit(*arg as c_int) {
            deep = getdigits_int(&raw mut arg, false, -1);
            arg = skipwhite(arg);
        }

        ex_unletlock(eap, arg, deep, 0, do_lock_var);
    }
}

/// Common parsing logic for :unlet, :lockvar and :unlockvar.
fn ex_unletlock(
    eap: &mut exarg_T,
    argstart: *mut c_char,
    deep: c_int,
    glv_flags: c_int,
    callback: ex_unletlock_callback,
) {
    unsafe {
        let mut arg = argstart;
        let mut name_end: *mut c_char;
        let mut error = false;
        let mut lv: lval_T = lval_T::new();

        loop {
            if crate::memory::cstr_first_byte(arg) == b'$' {
                lv.ll_name = arg;
                lv.ll_tv = ptr::null_mut();
                arg = arg.add(1);
                let mut arg_const: *const c_char = arg;
                if get_env_len(&raw mut arg_const) == 0 {
                    semsg!(gettext(e_invarg2), arg.sub(1));
                    return;
                }
                arg = arg_const.cast_mut();
                if !error && eap.skip == 0 && !callback(&mut lv, arg, eap, deep) {
                    error = true;
                }
                name_end = arg;
            } else {
                name_end = get_lval(
                    arg,
                    None,
                    &mut lv,
                    true,
                    eap.skip != 0 || error,
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
                        g_st().emsg_severe = true;
                        semsg!(gettext(e_trailing_arg), name_end);
                    }
                    break;
                }

                if !error && eap.skip == 0 && !callback(&mut lv, name_end, eap, deep)
                {
                    error = true;
                }
            }
            arg = skipwhite(name_end);
            if ends_excmd(*arg as c_int) != 0 {
                break;
            }
        }

        eap.nextcmd = check_nextcmd(arg);
    }
}

/// Unlet a variable indicated by `lp`.
fn do_unlet_var(
    lp: &mut lval_T,
    name_end: *mut c_char,
    eap: &mut exarg_T,
    _deep: c_int,
) -> bool {
    unsafe {
        let forceit = eap.forceit != 0;
        let mut ret = true;

        if lp.ll_tv.is_null() {
            let cc = *name_end;
            *name_end = NUL;

            if crate::memory::cstr_first_byte(lp.ll_name) == b'$' {
                vim_unsetenv_ext(lp.ll_name.add(1));
            } else if !do_unlet(lp.ll_name, lp.ll_name_len, forceit) {
                ret = false;
            }
            *name_end = cc;
        } else if (!lp.ll_list.is_null()
            && value_check_lock(
                tv_list_locked(lp.ll_list.as_ref()),
                lp.ll_name,
                lp.ll_name_len,
            ))
            || (!lp.ll_dict.is_null()
                && value_check_lock((*lp.ll_dict).dv_lock, lp.ll_name, lp.ll_name_len))
        {
            return false;
        } else if lp.ll_range {
            tv_list_unlet_range(
                lp.ll_list.as_mut(),
                lp.ll_li,
                lp.ll_n1,
                !lp.ll_empty2,
                lp.ll_n2,
            );
        } else if !lp.ll_list.is_null() {
            tv_list_item_remove(&mut *lp.ll_list, lp.ll_li);
        } else {
            let d = lp.ll_dict;
            debug_assert!(!d.is_null());
            let di = lp.ll_di;
            let watched = tv_dict_is_watched(d.as_ref());
            let mut key: Option<std::ffi::CString> = None;
            let mut oldtv: typval_T = typval_T::new();

            if watched {
                tv_copy(&(*di).di_tv, &mut oldtv);
                key = Some(cstr_to_cstring((*di).di_key.as_ptr()));
            }

            tv_dict_item_remove(&mut *d,  &mut *di);

            if let Some(ref k) = key {
                tv_dict_watcher_notify(&mut *d, k.as_ptr(), None, Some(&oldtv));
                tv_clear(&mut oldtv);
            }
        }

        ret
    }
}

/// Unlet one item or a range of items from a list.
fn tv_list_unlet_range(
    l: Option<&mut list_T>,
    li_first: *mut listitem_T,
    n1_arg: c_int,
    has_n2: bool,
    n2: c_int,
) {
    unsafe {
        debug_assert!(l.is_some());
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
        tv_list_remove_items( l.unwrap(), li_first,  &mut *li_last);
    }
}

/// unlet a variable.
pub fn do_unlet(name: impl crate::strings::IntoCCharPtr, name_len: usize, forceit: bool) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let mut varname: *const c_char = ptr::null();
        let mut dict: *mut dict_T = ptr::null_mut();
        let mut ht = find_var_ht_dict(name, name_len, &raw mut varname, &raw mut dict);

        if !ht.is_null() && !crate::memory::cstr_is_empty(varname) {
            let mut d = get_current_funccal_dict(ht);
            if d.is_null() {
                if ptr::eq(ht, &raw mut vars_st().globvardict.dv_hashtab) {
                    d = &raw mut vars_st().globvardict;
                } else if ptr::eq(ht, &raw mut vars_st().compat_hashtab) {
                    d = &raw mut vars_st().vimvardict;
                } else {
                    let di = find_var_in_ht( &mut *ht, crate::memory::cstr_first_byte(name) as c_int, c"", 0, 0);
                    d = (*di).di_tv.vval.v_dict();
                }
                if d.is_null() {
                    internal_error(c"do_unlet()");
                    return false;
                }
            }

            let mut hi = hash_find(&*ht, varname);
            if hashitem_empty(&*hi) {
                hi = find_hi_in_scoped_ht(name, &raw mut ht);
            }
            if !hi.is_null() && !hashitem_empty(&*hi) {
                let di = tv_dict_hi2di(hi);
                if var_check_fixed(c_int::from((*di).di_flags), name, TV_CSTRING)
                    || var_check_ro(c_int::from((*di).di_flags), name, TV_CSTRING)
                    || value_check_lock((*d).dv_lock, name, TV_CSTRING)
                {
                    return false;
                }

                if value_check_lock((*d).dv_lock, name, TV_CSTRING) {
                    return false;
                }

                let mut oldtv: typval_T = typval_T::new();
                let watched = tv_dict_is_watched(dict.as_ref());

                if watched {
                    tv_copy(&(*di).di_tv, &mut oldtv);
                }

                delete_var(&mut *ht,  &mut *hi);

                if watched {
                    tv_dict_watcher_notify(&mut *dict, varname, None, Some(&oldtv));
                    tv_clear(&mut oldtv);
                }
                return true;
            }
        }
        if forceit {
            return true;
        }
        semsg!(gettext(c"E108: No such variable: \"%s\""), name);
        false
    }
}

/// Lock or unlock variable indicated by `lp`.
fn do_lock_var(
    lp: &mut lval_T,
    _name_end: *mut c_char,
    eap: &mut exarg_T,
    deep: c_int,
) -> bool {
    unsafe {
        let lock = eap.cmdidx == cmdidx_T::CMD_lockvar as c_int;
        let mut ret = true;

        if lp.ll_tv.is_null() {
            if crate::memory::cstr_first_byte(lp.ll_name) == b'$' {
                semsg!(gettext(E_LOCK_UNLOCK), lp.ll_name);
                ret = false;
            } else {
                let di = find_var(lp.ll_name, lp.ll_name_len, ptr::null_mut(), 1);
                if di.is_null() {
                    ret = false;
                } else if ((*di).di_flags & DI_FLAGS_FIX) != 0
                    && (*di).di_tv.v_type != VarType::VAR_DICT
                    && (*di).di_tv.v_type != VarType::VAR_LIST
                {
                    semsg!(gettext(E_LOCK_UNLOCK), lp.ll_name);
                    ret = false;
                } else {
                    if lock {
                        (*di).di_flags |= DI_FLAGS_LOCK;
                    } else {
                        (*di).di_flags &= !DI_FLAGS_LOCK;
                    }
                    if deep != 0 {
                        tv_item_lock(&mut (*di).di_tv, deep, lock, false);
                    }
                }
            }
        } else if deep == 0 {
            // nothing to do
        } else if lp.ll_range {
            let mut li = lp.ll_li;
            while !li.is_null() && (lp.ll_empty2 || lp.ll_n2 >= lp.ll_n1) {
                tv_item_lock(&mut (*li).li_tv, deep, lock, false);
                li = (*li).li_next;
                lp.ll_n1 += 1;
            }
        } else if !lp.ll_list.is_null() {
            tv_item_lock(&mut (*lp.ll_li).li_tv, deep, lock, false);
        } else {
            tv_item_lock(&mut (*lp.ll_di).di_tv, deep, lock, false);
        }

        ret
    }
}

/// Delete all "menutrans_" variables.
pub fn del_menutrans_vars() {
    unsafe {
        hash_lock(&mut vars_st().globvardict.dv_hashtab);
        let mut todo = vars_st().globvardict.dv_hashtab.ht_used;
        let mut hi = vars_st().globvardict.dv_hashtab.array_ptr();
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                if crate::memory::cstr_prefix_bytes((&*hi).key_ptr(), 10) == &b"menutrans_"[..10] {
                    delete_var(&mut vars_st().globvardict.dv_hashtab,  &mut *hi);
                }
            }
            hi = hi.add(1);
        }
        hash_unlock(&mut vars_st().globvardict.dv_hashtab);
    }
}

pub fn get_globvar_dict() -> *mut dict_T {
    &raw mut vars_st().globvardict
}

pub fn get_globvar_ht() -> *mut hashtab_T {
    &raw mut vars_st().globvardict.dv_hashtab
}

pub fn get_vimvar_dict() -> *mut dict_T {
    &raw mut vars_st().vimvardict
}

pub fn set_vim_var_tv(idx: VimVarIndex, tv: &mut typval_T) {
     {
        let tv_out = get_vim_var_tv(idx);
        tv_clear(tv_out);
        tv_copy(&*tv, tv_out);
    }
}

pub fn get_vim_var_name(idx: VimVarIndex) -> *mut c_char {
    vars_st().vimvars[idx as usize]
            .vv_name
            .as_ptr()
            .cast_mut()
}

pub fn get_vim_var_tv(idx: VimVarIndex) -> &'static mut typval_T {
    &mut vars_st().vimvars[idx as usize].vv_di.di_tv
}

pub fn get_vim_var_nr(idx: VimVarIndex) -> varnumber_T {
    get_vim_var_tv(idx).vval.v_number()
}

pub fn get_vim_var_list(idx: VimVarIndex) -> *mut list_T {
    get_vim_var_tv(idx).vval.v_list()
}

pub fn get_vim_var_dict(idx: VimVarIndex) -> *mut dict_T {
    get_vim_var_tv(idx).vval.v_dict()
}

pub fn get_vim_var_str(idx: VimVarIndex) -> *mut c_char {
    tv_get_string(get_vim_var_tv(idx)).cast_mut()
}

pub fn get_vim_var_partial(idx: VimVarIndex) -> *mut partial_T {
    get_vim_var_tv(idx).vval.v_partial()
}

/// Function to concatenate a prefix and a variable name.
pub fn cat_prefix_varname(prefix: c_int, name: impl crate::strings::IntoCCharPtr) -> *mut c_char { let name = name.into_c_char_ptr();
    unsafe {
        let name_len = crate::memory::cstr_len(name);
        vars_st().varnamebuf.clear();
        vars_st().varnamebuf.reserve(name_len + 3);
        vars_st().varnamebuf.push(prefix as u8 as c_char);
        vars_st().varnamebuf.push(b':' as c_char);
        vars_st().varnamebuf.extend_from_slice(std::slice::from_raw_parts(name, name_len + 1));
        vars_st().varnamebuf.as_mut_ptr()
    }
}

/// Function given to ExpandGeneric() to obtain the list of user defined variable names.
pub fn get_user_var_name(xp: &mut expand_T, idx: c_int) -> *mut c_char {

    unsafe {
        if idx == 0 {
            vars_st().GDONE = 0;
            vars_st().BDONE = 0;
            vars_st().WDONE = 0;
            vars_st().VIDX = 0;
            vars_st().TDONE = 0;
        }

        if vars_st().GDONE < vars_st().globvardict.dv_hashtab.ht_used {
            if vars_st().GDONE == 0 {
                vars_st().HI = (&*get_globvar_ht()).array_ptr();
            } else {
                vars_st().HI = vars_st().HI.add(1);
            }
            vars_st().GDONE += 1;
            while hashitem_empty(&*vars_st().HI) {
                vars_st().HI = vars_st().HI.add(1);
            }
            let hi_key = (&*vars_st().HI).key_ptr();
            if b"g:" == crate::memory::cstr_prefix_bytes(xp.xp_pattern, 2) {
                return cat_prefix_varname(b'g' as c_int, hi_key);
            }
            return hi_key;
        }

        let mut ht: *const hashtab_T =
            &raw const (*(*(*prevwin_curwin()).w_buffer).b_vars()).dv_hashtab;
        if vars_st().BDONE < (*ht).ht_used {
            if vars_st().BDONE == 0 {
                vars_st().HI = (&*ht).array_ptr();
            } else {
                vars_st().HI = vars_st().HI.add(1);
            }
            vars_st().BDONE += 1;
            while hashitem_empty(&*vars_st().HI) {
                vars_st().HI = vars_st().HI.add(1);
            }
            return cat_prefix_varname(b'b' as c_int, (&*vars_st().HI).key_ptr());
        }

        ht = &raw const (*(*prevwin_curwin()).w_vars()).dv_hashtab;
        if vars_st().WDONE < (*ht).ht_used {
            if vars_st().WDONE == 0 {
                vars_st().HI = (&*ht).array_ptr();
            } else {
                vars_st().HI = vars_st().HI.add(1);
            }
            vars_st().WDONE += 1;
            while hashitem_empty(&*vars_st().HI) {
                vars_st().HI = vars_st().HI.add(1);
            }
            return cat_prefix_varname(b'w' as c_int, (&*vars_st().HI).key_ptr());
        }

        ht = &raw const (*curtab_mut().tp_vars()).dv_hashtab;
        if vars_st().TDONE < (*ht).ht_used {
            if vars_st().TDONE == 0 {
                vars_st().HI = (&*ht).array_ptr();
            } else {
                vars_st().HI = vars_st().HI.add(1);
            }
            vars_st().TDONE += 1;
            while hashitem_empty(&*vars_st().HI) {
                vars_st().HI = vars_st().HI.add(1);
            }
            return cat_prefix_varname(b't' as c_int, (&*vars_st().HI).key_ptr());
        }

        if vars_st().VIDX < VV_LEN {
            let v = vars_st().VIDX as VimVarIndex;
            vars_st().VIDX += 1;
            return cat_prefix_varname(b'v' as c_int, get_vim_var_name(v));
        }

        vars_st().varnamebuf = Vec::new();
        ptr::null_mut()
    }
}

pub fn set_vim_var_type(idx: VimVarIndex, v_type: VarType) {
     {
        let tv = get_vim_var_tv(idx);
        tv.v_type = v_type;
    }
}

pub fn set_vim_var_nr(idx: VimVarIndex, val: varnumber_T) {
     {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        tv.vval = crate::eval::typval::TypvalVval::Number(val);
    }
}

pub fn set_vim_var_bool(idx: VimVarIndex, val: BoolVarValue) {
     {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        tv.v_type = VarType::VAR_BOOL;
        tv.vval = crate::eval::typval::TypvalVval::Bool(val);
    }
}

pub fn set_vim_var_special(idx: VimVarIndex, val: SpecialVarValue) {
     {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        tv.v_type = VarType::VAR_SPECIAL;
        tv.vval = crate::eval::typval::TypvalVval::Special(val);
    }
}

pub fn set_vim_var_char(c: c_int) {
    let mut buf = [0 as c_char; MB_MAXCHAR + 1];
        let buflen = utf_char2bytes(c, buf.as_mut_ptr());
        buf[buflen as usize] = NUL;
        set_vim_var_string(VV_CHAR, buf.as_ptr(), buflen as isize);
}

pub fn set_vim_var_string(idx: VimVarIndex, val: impl crate::strings::IntoCCharPtr, len: isize) {
    let val = val.into_c_char_ptr();
     {
        let tv = get_vim_var_tv(idx);
        if val.is_null() {
            tv_clear(tv);
            tv.v_type = VarType::VAR_STRING;
            tv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        } else if len == -1 {
            crate::eval::typval::tv_set_string(tv, val);
        } else {
            crate::eval::typval::tv_set_string_len(tv, val, len as usize);
        }
    }
}

pub fn set_vim_var_list(idx: VimVarIndex, val: *mut list_T) {
     {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        tv.v_type = VarType::VAR_LIST;
        tv.vval = crate::eval::typval::TypvalVval::List(val);
        if !val.is_null() {
            tv_list_ref(unsafe { val.as_mut() });
        }
    }
}

pub fn set_vim_var_dict(idx: VimVarIndex, val: *mut dict_T) {
    unsafe {
        let tv = get_vim_var_tv(idx);
        tv_clear(tv);
        tv.v_type = VarType::VAR_DICT;
        tv.vval = crate::eval::typval::TypvalVval::Dict(val);
        if val.is_null() {
            return;
        }

        (*val).dv_refcount += 1;
        tv_dict_set_keys_readonly(&mut *val);
    }
}

pub fn set_vim_var_partial(idx: VimVarIndex, val: *mut partial_T) {
     {
        let tv = get_vim_var_tv(idx);
        tv.vval = crate::eval::typval::TypvalVval::Partial(val);
    }
}

pub fn set_reg_var(c: c_int) {
    unsafe {
        let mut regname: [c_char; 2] = [0, NUL];
        if c == 0 || c == b' ' as c_int {
            regname[0] = b'"' as c_char;
        } else {
            regname[0] = c as u8 as c_char;
        }
        let tv = get_vim_var_tv(VV_REG);
        if tv.vval.v_string().is_null() || *tv.vval.v_string() as c_int != c {
            set_vim_var_string(VV_REG, regname.as_ptr(), 1);
        }
    }
}

pub fn v_exception(oldval: Option<std::ffi::CString>) -> Option<std::ffi::CString> {
     {
        let tv = get_vim_var_tv(VV_EXCEPTION);
        let Some(oldval) = oldval else {
            let s = tv.vval.v_string();
            return if s.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(s))
            };
        };

        set_vim_var_string(VV_EXCEPTION, oldval.as_ptr(), -1);
        None
    }
}

pub fn set_cmdarg(eap: Option<&exarg_T>,
    oldarg: Option<std::ffi::CString>,
) -> Option<std::ffi::CString> {
    unsafe {
        let tv = get_vim_var_tv(VV_CMDARG);
        let oldval = crate::eval::typval::tv_take_string(tv.vval.v_string());
        tv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        if eap.is_none() {
            tv.v_type = VarType::VAR_STRING;
            tv.vval = crate::eval::typval::TypvalVval::String(
                oldarg.map_or(ptr::null_mut(), std::ffi::CString::into_raw),
            );
            return None;
        }

        let mut newval: Vec<u8> = Vec::new();
        if eap.unwrap().force_bin == FORCE_BIN {
            newval.extend_from_slice(b" ++bin");
        } else if eap.unwrap().force_bin == FORCE_NOBIN {
            newval.extend_from_slice(b" ++nobin");
        }

        if eap.unwrap().read_edit != 0 {
            newval.extend_from_slice(b" ++edit");
        }

        if eap.unwrap().force_ff != 0 {
            let ff_str: &[u8] = if eap.unwrap().force_ff == b'u' as c_int {
                b" ++ff=unix"
            } else if eap.unwrap().force_ff == b'd' as c_int {
                b" ++ff=dos"
            } else {
                b" ++ff=mac"
            };
            newval.extend_from_slice(ff_str);
        }
        if eap.unwrap().force_enc != 0 {
            newval.extend_from_slice(b" ++enc=");
            let enc_ptr = eap.unwrap().cmd.add(eap.unwrap().force_enc as usize);
            newval.extend_from_slice(crate::memory::cstr_as_bytes(enc_ptr));
        }

        if eap.unwrap().bad_char == BAD_KEEP {
            newval.extend_from_slice(b" ++bad=keep");
        } else if eap.unwrap().bad_char == BAD_DROP {
            newval.extend_from_slice(b" ++bad=drop");
        } else if eap.unwrap().bad_char != 0 {
            newval.extend_from_slice(b" ++bad=");
            newval.push(eap.unwrap().bad_char as u8);
        }

        if eap.unwrap().mkdir_p != 0 {
            newval.extend_from_slice(b" ++p");
        }

        crate::eval::typval::tv_set_string_len(tv, newval.as_ptr().cast::<c_char>(), newval.len());
        oldval
    }
}

pub fn v_throwpoint(oldval: Option<std::ffi::CString>) -> Option<std::ffi::CString> {
     {
        let tv = get_vim_var_tv(VV_THROWPOINT);
        let Some(oldval) = oldval else {
            let s = tv.vval.v_string();
            return if s.is_null() {
                None
            } else {
                Some(crate::memory::cstr_to_cstring(s))
            };
        };

        set_vim_var_string(VV_THROWPOINT, oldval.as_ptr(), -1);
        None
    }
}

pub fn set_vcount(count: i64, count1: i64, set_prevcount: bool) {
     {
        if set_prevcount {
            get_vim_var_tv(VV_PREVCOUNT).vval = crate::eval::typval::TypvalVval::Number(get_vim_var_nr(VV_COUNT));
        }
        get_vim_var_tv(VV_COUNT).vval = crate::eval::typval::TypvalVval::Number(count);
        get_vim_var_tv(VV_COUNT1).vval = crate::eval::typval::TypvalVval::Number(count1);
    }
}

/// Get the value of internal variable "name".
pub fn eval_variable(
    name: impl crate::strings::IntoCCharPtr,
    len: c_int,rettv: Option<&mut typval_T>,
    dip: *mut *mut dictitem_T,
    verbose: bool,
    no_autoload: bool,
) -> bool { let name = name.into_c_char_ptr();
    unsafe {
        let mut ret = true;
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
            if rettv.is_some() && verbose {
                semsg!(
                    gettext(c"E121: Undefined variable: %.*s"),
                    len,
                    name,
                );
            }
            ret = false;
        } else if rettv.is_some() {
            tv_copy(&*tv, rettv.unwrap());
        }

        ret
    }
}

/// Check if variable "name[len]" is a local variable or an argument.
pub fn check_vars(name: impl crate::strings::IntoCCharPtr, len: usize) { let name = name.into_c_char_ptr();
    unsafe {
        if eval_st().eval_lavars_used.is_null() {
            return;
        }

        let mut varname: *const c_char = ptr::null();
        let ht = find_var_ht(name, len, &raw mut varname);

        if (ht == get_funccal_local_ht() || ht == get_funccal_args_ht())
            && !find_var(name, len, ptr::null_mut(), 1).is_null()
        {
            *eval_st().eval_lavars_used = true;
        }
    }
}

/// Find variable "name" in the list of variables.
pub fn find_var(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,
    htp: *mut *mut hashtab_T,
    no_autoload: c_int,
) -> *mut dictitem_T { let name = name.into_c_char_ptr();
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
             &mut *ht,
            crate::memory::cstr_first_byte(name) as c_int,
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
pub fn find_var_in_ht(
    ht: &mut hashtab_T,
    htname: c_int,
    varname: impl crate::strings::IntoCCharPtr,
    varname_len: usize,
    no_autoload: c_int,
) -> *mut dictitem_T {
    let varname = varname.into_c_char_ptr();
    unsafe {
        if varname_len == 0 {
            return match htname as u8 {
                b's' => (&raw mut (*script_sv(g_st().current_sctx.sc_sid)).sv_var).cast::<dictitem_T>(),
                b'g' => (&raw mut vars_st().globvars_var).cast::<dictitem_T>(),
                b'v' => (&raw mut vars_st().vimvars_var).cast::<dictitem_T>(),
                b'b' => (&raw mut (*g_st().curbuf).b_bufvar).cast::<dictitem_T>(),
                b'w' => (&raw mut (*curwin_ptr()).w_winvar).cast::<dictitem_T>(),
                b't' => (&raw mut (*g_st().curtab).tp_winvar).cast::<dictitem_T>(),
                b'l' => get_funccal_local_var(),
                b'a' => get_funccal_args_var(),
                _ => ptr::null_mut(),
            };
        }

        let mut hi = hash_find_len(ht, varname, varname_len);
        if hashitem_empty(&*hi) {
            if (&raw mut *ht) == get_globvar_ht() && no_autoload == 0 {
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
fn find_var_ht_dict(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,
    varname: *mut *const c_char,
    d: *mut *mut dict_T,
) -> *mut hashtab_T { let name = name.into_c_char_ptr();
    unsafe {
        *d = ptr::null_mut();

        if name_len == 0 {
            return ptr::null_mut();
        }
        if name_len == 1 || crate::memory::cstr_byte_at(name, 1) != b':' {
            if crate::memory::cstr_first_byte(name) == b':' || *name == AUTOLOAD_CHAR {
                return ptr::null_mut();
            }
            *varname = name;

            let hi = hash_find_len(&vars_st().compat_hashtab, name, name_len);
            if !hashitem_empty(&*hi) {
                return &raw mut vars_st().compat_hashtab;
            }

            *d = get_funccal_local_dict();
            if !(*d).is_null() {
                return &raw mut (*(*d)).dv_hashtab;
            }

            *d = get_globvar_dict();
            return &raw mut (*(*d)).dv_hashtab;
        }

        *varname = name.add(2);
        if crate::memory::cstr_first_byte(name) == b'g' {
            *d = get_globvar_dict();
        } else if name_len > 2
            && (!c_memchr(name.add(2), b':', name_len - 2).is_null()
                || !c_memchr(name.add(2), AUTOLOAD_CHAR as u8, name_len - 2).is_null())
        {
            return ptr::null_mut();
        }

        if crate::memory::cstr_first_byte(name) == b'b' {
            *d = curbuf_mut().b_vars();
        } else if crate::memory::cstr_first_byte(name) == b'w' {
            *d = curwin_mut().w_vars();
        } else if crate::memory::cstr_first_byte(name) == b't' {
            *d = curtab_mut().tp_vars();
        } else if crate::memory::cstr_first_byte(name) == b'v' {
            *d = get_vimvar_dict();
        } else if crate::memory::cstr_first_byte(name) == b'a' {
            *d = get_funccal_args_dict();
        } else if crate::memory::cstr_first_byte(name) == b'l' {
            *d = get_funccal_local_dict();
        } else if crate::memory::cstr_first_byte(name) == b's'
            && (g_st().current_sctx.sc_sid > 0
                || g_st().current_sctx.sc_sid == SID_STR
                || g_st().current_sctx.sc_sid == SID_LUA)
            && g_st().current_sctx.sc_sid <= crate::runtime::script_items_len()
        {
            nlua_set_sctx(&mut g_st().current_sctx);
            if g_st().current_sctx.sc_sid == SID_STR || g_st().current_sctx.sc_sid == SID_LUA {
                new_script_item(None, &mut g_st().current_sctx.sc_sid);
            }
            *d = &raw mut (*script_sv(g_st().current_sctx.sc_sid)).sv_dict;
        }

        if !(*d).is_null() {
            &raw mut (*(*d)).dv_hashtab
        } else {
            ptr::null_mut()
        }
    }
}

/// Find the hashtable used for a variable.
pub fn find_var_ht(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,
    varname: *mut *const c_char,
) -> *mut hashtab_T { let name = name.into_c_char_ptr();
    let mut d: *mut dict_T = ptr::null_mut();
        find_var_ht_dict(name, name_len, varname, &raw mut d)
}

/// Return the string value of a (global/local) variable or NULL when it doesn't exist.
pub fn get_var_value(name: &std::ffi::CStr) -> *mut c_char {
    unsafe {
        let v = find_var(name.as_ptr(), name.to_bytes().len(), ptr::null_mut(), 0);
        if v.is_null() {
            return ptr::null_mut();
        }
        tv_get_string(&(*v).di_tv).cast_mut()
    }
}

/// Allocate a new hashtab for a sourced script.
pub fn new_script_vars() -> Box<scriptvar_T> {
    let mut sv = Box::new(scriptvar_T::new());
        init_var_dict(&mut sv.sv_dict, &mut sv.sv_var, VAR_SCOPE);
        sv
}

/// Initialize dictionary "dict" as a scope and set variable "dict_var" to point to it.
pub fn init_var_dict(dict: &mut dict_T, dict_var: &mut ScopeDictDictItem, scope: ScopeType) {
    hash_clear(&mut dict.dv_hashtab);
        dict.dv_lock = VAR_UNLOCKED;
        dict.dv_scope = scope;
        dict.dv_refcount = DO_NOT_FREE_CNT;
        dict.dv_copyID = 0;
        dict_var.di_tv.vval = crate::eval::typval::TypvalVval::Dict(dict);
        dict_var.di_tv.v_type = VarType::VAR_DICT;
        dict_var.di_tv.v_lock = VAR_FIXED;
        dict_var.di_flags = DI_FLAGS_RO | DI_FLAGS_FIX;
        dict_var.di_key.clear();
        dict.watchers.clear();
}

/// Unreference a dictionary initialized by init_var_dict().
pub fn unref_var_dict(dict: &mut dict_T) {
    dict.dv_refcount -= DO_NOT_FREE_CNT - 1;
        tv_dict_unref(&raw mut *dict);
}

/// Clean up a list of internal variables.
pub fn vars_clear(ht: &mut hashtab_T) {
    vars_clear_ext(ht, true);
}

/// Like vars_clear(), but only free the value if "free_val" is true.
pub fn vars_clear_ext(ht: &mut hashtab_T, free_val: bool) {
    unsafe {
        hash_lock(ht);
        let mut todo = ht.ht_used as c_int;
        let mut hi = ht.array_ptr();
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;

                let v = tv_dict_hi2di(hi);
                if !free_val {
                    (*v).di_tv.v_type = VarType::VAR_UNKNOWN;
                }
                if ((*v).di_flags & DI_FLAGS_ALLOC) != 0 {
                    drop(Box::from_raw(v));
                } else if free_val {
                    tv_clear(&mut (*v).di_tv);
                }
            }
            hi = hi.add(1);
        }
        hash_clear(ht);
    }
}

/// Delete a variable from hashtab "ht" at item "hi".
fn delete_var(ht: &mut hashtab_T, hi: &mut hashitem_T) {
    let di = tv_dict_hi2di(&raw mut *hi);
        hash_remove(ht,  hi);
        crate::eval::typval::tv_dict_item_free(di);
}

/// List the value of one internal variable.
fn list_one_var(v: &mut dictitem_T, prefix: impl crate::strings::IntoCCharPtr,first: &mut c_int) { let prefix = prefix.into_c_char_ptr();
     {
        let s = encode_tv2echo(&mut v.di_tv, None);
        list_one_var_a(
            prefix,
            v.di_key.as_ptr(),
            crate::memory::cstr_len(v.di_key.as_ptr()) as isize,
            v.di_tv.v_type,
            s.as_ptr(),
            first,
        );
    }
}

fn list_one_var_a(
    prefix: impl crate::strings::IntoCCharPtr,
    name: impl crate::strings::IntoCCharPtr,
    name_len: isize,
    v_type: VarType,
    string: impl crate::strings::IntoCCharPtr,
    first: &mut c_int,
) { let name = name.into_c_char_ptr(); let mut string = string.into_c_char_ptr();
    let prefix = prefix.into_c_char_ptr();
    unsafe {
        if *first != 0 {
            msg_ext_set_kind(c"list_cmd");
            msg_start();
        } else {
            msg_putchar(b'\n' as c_int);
        }
        if !crate::memory::cstr_is_empty(prefix) {
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
            if crate::memory::cstr_first_byte(string) == b'[' {
                string = string.add(1);
            }
        } else if v_type == VarType::VAR_DICT {
            msg_putchar(b'{' as c_int);
            if crate::memory::cstr_first_byte(string) == b'{' {
                string = string.add(1);
            }
        } else {
            msg_putchar(b' ' as c_int);
        }

        msg_outtrans(string, 0, false);

        if v_type == VarType::VAR_FUNC || v_type == VarType::VAR_PARTIAL {
            msg_puts(c"()");
        }
        if *first != 0 {
            msg_clr_eos();
            *first = 0;
        }
    }
}

/// Additional handling for setting a v: variable.
pub fn before_set_vvar(
    varname: impl crate::strings::IntoCCharPtr,
    di: &mut dictitem_T,tv: &mut typval_T,
    copy: bool,
    watched: bool,
    type_error: &mut bool,
) -> bool { let varname = varname.into_c_char_ptr();
     {
        if di.di_tv.v_type == VarType::VAR_STRING {
            let mut oldtv: typval_T = typval_T::new();
            if watched {
                tv_copy(&di.di_tv, &mut oldtv);
            }
            tv_clear(&mut di.di_tv);
            if copy || tv.v_type != VarType::VAR_STRING {
                let val = tv_get_string(&*tv);
                crate::eval::typval::tv_set_string(&mut di.di_tv, val);
            } else {
                di.di_tv.v_type = VarType::VAR_STRING;
                di.di_tv.vval = crate::eval::typval::TypvalVval::String(tv.vval.v_string());
                tv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
            }
            if watched {
                tv_dict_watcher_notify(
                    &mut vars_st().vimvardict,
                    varname,
                    Some(&di.di_tv),
                    Some(&oldtv),
                );
                tv_clear(&mut oldtv);
            }
            return false;
        } else if di.di_tv.v_type == VarType::VAR_NUMBER {
            let mut oldtv: typval_T = typval_T::new();
            if watched {
                tv_copy(&di.di_tv, &mut oldtv);
            }
            di.di_tv.vval = crate::eval::typval::TypvalVval::Number(tv_get_number(&*tv));
            if crate::memory::cstr_as_bytes(varname) == b"searchforward" {
                set_search_direction(if di.di_tv.vval.v_number() != 0 {
                    b'/' as c_int
                } else {
                    b'?' as c_int
                });
            } else if crate::memory::cstr_as_bytes(varname) == b"hlsearch" {
                g_st().Search.no_hlsearch = di.di_tv.vval.v_number() == 0;
                redraw_all_later(UPD_SOME_VALID);
            }
            if watched {
                tv_dict_watcher_notify(
                    &mut vars_st().vimvardict,
                    varname,
                    Some(&di.di_tv),
                    Some(&oldtv),
                );
                tv_clear(&mut oldtv);
            }
            return false;
        } else if di.di_tv.v_type != tv.v_type {
            *type_error = true;
            return false;
        }
        true
    }
}

/// Set variable to the given value.
pub fn set_var(name: impl crate::strings::IntoCCharPtr, name_len: usize,tv: &mut typval_T, copy: bool) {
    let name = name.into_c_char_ptr();
    set_var_const(name, name_len, tv, copy, false);
}

/// Set variable to the given value, optionally locking as const.
pub fn set_var_const(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,tv: &mut typval_T,
    copy: bool,
    is_const: bool,
) { let name = name.into_c_char_ptr();
    unsafe {
        let mut varname: *const c_char = ptr::null();
        let mut dict: *mut dict_T = ptr::null_mut();
        let ht = find_var_ht_dict(name, name_len, &raw mut varname, &raw mut dict);
        let watched = tv_dict_is_watched(dict.as_ref());

        if ht.is_null() || crate::memory::cstr_is_empty(varname) {
            semsg!(gettext(e_illvar), name);
            return;
        }
        let varname_len = name_len - (varname.offset_from(name) as usize);
        let mut di = find_var_in_ht( &mut *ht, 0, varname, varname_len, 1);

        if di.is_null() {
            di = find_var_in_scoped_ht(name, name_len, 1);
        }

        if tv_is_func(tv) && var_wrong_func_name(name, di.is_null()) {
            return;
        }

        let mut oldtv: typval_T = typval_T::new();
        if !di.is_null() {
            if is_const {
                emsg(gettext(e_cannot_mod));
                return;
            }

            if var_check_ro(c_int::from((*di).di_flags), name, name_len)
                || value_check_lock((*di).di_tv.v_lock, name, name_len)
                || var_check_lock(c_int::from((*di).di_flags), name, name_len)
            {
                return;
            }

            let mut type_error = false;
            if ptr::eq(ht, &raw mut vars_st().vimvardict.dv_hashtab)
                && !before_set_vvar(varname, &mut *di, tv, copy, watched, &mut type_error)
            {
                if type_error {
                    semsg!(
                        gettext(E_SETTING_V_STR_TO_VALUE_WITH_WRONG_TYPE),
                        varname,
                    );
                }
                return;
            }

            if watched {
                tv_copy(&(*di).di_tv, &mut oldtv);
            }
            tv_clear(&mut (*di).di_tv);
        } else {
            if ptr::eq(ht, &raw mut vars_st().vimvardict.dv_hashtab) || ht == get_funccal_args_ht() {
                semsg!(gettext(e_illvar), name);
                return;
            }

            if !valid_varname(varname) {
                return;
            }

            debug_assert!(!dict.is_null());

            di = crate::eval::typval::tv_dict_item_alloc_len(varname, varname_len);
            if !hash_add(&mut *ht, &mut *di) {
                crate::eval::typval::tv_dict_item_free(di);
                return;
            }
            if is_const {
                (*di).di_flags |= DI_FLAGS_LOCK;
            }
        }

        if copy || tv.v_type == VarType::VAR_NUMBER || tv.v_type == VarType::VAR_FLOAT {
            tv_copy(&*tv, &mut (*di).di_tv);
        } else {
            (*di).di_tv = *tv;
            (*di).di_tv.v_lock = VAR_UNLOCKED;
            tv_init(Some(tv));
        }

        if watched {
            tv_dict_watcher_notify(
                &mut *dict,
                (*di).di_key.as_ptr(),
                Some(&(*di).di_tv),
                Some(&oldtv),
            );
            tv_clear(&mut oldtv);
        }

        if is_const {
            tv_item_lock(&mut (*di).di_tv, DICT_MAXNEST, true, true);
        }
    }
}

/// Check whether variable is read-only (DI_FLAGS_RO, DI_FLAGS_RO_SBX).
pub fn var_check_ro(flags: c_int, name: impl crate::strings::IntoCCharPtr, mut name_len: usize) -> bool { let mut name = name.into_c_char_ptr();
     {
        let error_message = if (flags as u8 & DI_FLAGS_RO) != 0 {
            e_cannot_change_readonly_variable_str.as_ptr()
        } else if (flags as u8 & DI_FLAGS_RO_SBX) != 0 && g_st().sandbox != 0 {
            e_cannot_set_variable_in_sandbox_str.as_ptr()
        } else {
            ptr::null()
        };

        if error_message.is_null() {
            return false;
        }
        if name_len == TV_TRANSLATE {
            name = gettext(name);
            name_len = crate::memory::cstr_len(name);
        } else if name_len == TV_CSTRING {
            name_len = crate::memory::cstr_len(name);
        }

        semsg!(gettext(error_message), name_len as c_int, name);
        true
    }
}

/// Return true if di_flags "flags" indicates variable "name" is locked.
pub fn var_check_lock(flags: c_int, name: impl crate::strings::IntoCCharPtr, mut name_len: usize) -> bool { let mut name = name.into_c_char_ptr();
     {
        if (flags as u8 & DI_FLAGS_LOCK) == 0 {
            return false;
        }

        if name_len == TV_TRANSLATE {
            name = gettext(name);
            name_len = crate::memory::cstr_len(name);
        } else if name_len == TV_CSTRING {
            name_len = crate::memory::cstr_len(name);
        }

        semsg!(
            gettext(c"E1122: Variable is locked: %.*s"),
            name_len as c_int,
            name,
        );
        true
    }
}

/// Check whether variable is fixed (DI_FLAGS_FIX).
pub fn var_check_fixed(flags: c_int, name: impl crate::strings::IntoCCharPtr, mut name_len: usize) -> bool { let mut name = name.into_c_char_ptr();
     {
        if (flags as u8 & DI_FLAGS_FIX) != 0 {
            if name_len == TV_TRANSLATE {
                name = gettext(name);
                name_len = crate::memory::cstr_len(name);
            } else if name_len == TV_CSTRING {
                name_len = crate::memory::cstr_len(name);
            }
            semsg!(
                gettext(e_cannot_delete_variable_str),
                name_len as c_int,
                name,
            );
            return true;
        }
        false
    }
}

/// Check if name is a valid name to assign funcref to.
pub fn var_wrong_func_name(name: impl crate::strings::IntoCCharPtr, new_var: bool) -> bool { let name = name.into_c_char_ptr();
    unsafe {
        let first_after_scope = if !crate::memory::cstr_is_empty(name) && crate::memory::cstr_byte_at(name, 1) == b':' {
            *name.add(2)
        } else {
            *name
        };
        if !(!vim_strchr(c"wbst", crate::memory::cstr_first_byte(name) as c_int).is_null()
            && crate::memory::cstr_byte_at(name, 1) == b':')
            && !ascii_isupper(first_after_scope)
            && vim_strchr(name, b'#' as c_int).is_null()
        {
            semsg!(
                gettext(c"E704: Funcref variable name must start with a capital: %s"),
                name,
            );
            return true;
        }
        if new_var && function_exists(name, false) {
            semsg!(
                gettext(c"E705: Variable name conflicts with existing function: %s"),
                name,
            );
            return true;
        }
        false
    }
}

/// Check if a variable name is valid.
pub fn valid_varname(varname: impl crate::strings::IntoCCharPtr) -> bool { let varname = varname.into_c_char_ptr();
    unsafe {
        let mut p = varname;
        while !crate::memory::cstr_is_empty(p) {
            if !eval_isnamec1(crate::memory::cstr_first_byte(p) as c_int)
                && (p == varname || !ascii_isdigit(*p as c_int))
                && *p != AUTOLOAD_CHAR
            {
                semsg!(gettext(e_illvar), varname);
                return false;
            }
            p = p.add(1);
        }
        true
    }
}

/// Implements the logic to retrieve local variable and option values.
fn get_var_from(
    varname: impl crate::strings::IntoCCharPtr,rettv: &mut typval_T,deftv: &mut typval_T,
    htname: c_int,
    tp: *mut tabpage_T,
    win: *mut win_T,
    buf: *mut buf_T,
) { let mut varname = varname.into_c_char_ptr();
    unsafe {
        let mut done = false;
        let do_change_curbuf = !buf.is_null() && htname == b'b' as c_int;

        g_st().emsg_off += 1;

        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if !varname.is_null()
            && !tp.is_null()
            && !win.is_null()
            && (htname != b'b' as c_int || !buf.is_null())
        {
            let need_switch_win = !(tp == g_st().curtab && win == curwin_ptr()) && !do_change_curbuf;
            let mut switchwin: CtxSwitch = CtxSwitch::new();
            if !need_switch_win
                || ctx_switch(
                    &mut switchwin,
                    win,
                    tp,
                    ptr::null_mut(),
                    kCtxNoEvents | kCtxNoDisplay,
                )
            {
                if crate::memory::cstr_first_byte(varname) == b'&' && htname != b't' as c_int {
                    let save_curbuf = g_st().curbuf;

                    if do_change_curbuf {
                        g_st().curbuf = buf;
                    }

                    if crate::memory::cstr_byte_at(varname, 1) == 0 {
                        let opts = get_winbuf_options(c_int::from(htname == b'b' as c_int));
                        if !opts.is_null() {
                            tv_dict_set_ret(rettv, opts);
                            done = true;
                        }
                    } else if eval_option(&raw mut varname, Some(rettv), true) {
                        done = true;
                    }

                    g_st().curbuf = save_curbuf;
                } else if crate::memory::cstr_is_empty(varname) {
                    let v: *const ScopeDictDictItem = if htname == b'b' as c_int {
                        &raw const (*buf).b_bufvar
                    } else if htname == b'w' as c_int {
                        (&raw const (*win).w_winvar).cast()
                    } else {
                        (&raw const (*tp).tp_winvar).cast()
                    };
                    tv_copy(&(*v).di_tv, rettv);
                    done = true;
                } else {
                    let ht = if htname == b'b' as c_int {
                        &raw mut (*(*buf).b_vars()).dv_hashtab
                    } else if htname == b'w' as c_int {
                        &raw mut (*(*win).w_vars()).dv_hashtab
                    } else {
                        &raw mut (*(*tp).tp_vars()).dv_hashtab
                    };

                    let v = find_var_in_ht( &mut *ht, htname, varname, crate::memory::cstr_len(varname), 0);
                    if !v.is_null() {
                        tv_copy(&(*v).di_tv, rettv);
                        done = true;
                    }
                }
            }

            if need_switch_win {
                ctx_restore(&mut switchwin);
            }
        }

        if !done && deftv.v_type != VarType::VAR_UNKNOWN {
            tv_copy(&*deftv, rettv);
        }

        g_st().emsg_off -= 1;
    }
}

/// getwinvar() and gettabwinvar().
fn getwinvar(argvars: &mut [typval_T],rettv: &mut typval_T, off: usize) {
     {
        let tp = if off == 1 {
            find_tabpage(tv_get_number_chk(&argvars[0], None) as c_int)
        } else {
            g_st().curtab
        };
        let win = find_win_by_nr(&argvars[off], tp);
        let varname = tv_get_string_chk(&argvars[off + 1]);

        get_var_from(
            varname,
            rettv,
            &mut argvars[off + 2],
            b'w' as c_int,
            tp,
            win,
            ptr::null_mut(),
        );
    }
}

/// Converts a typval to a structured option value.
fn opt_from_tv(tv: &mut typval_T,
    opt_idx: OptIndex,
    option: impl crate::strings::IntoCCharPtr,
    error: &mut bool,
) -> Object { let option = option.into_c_char_ptr();
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

        if !is_tty_opt && ((*get_option(opt_idx)).flags & kOptFlagFunc) != 0 && tv_is_func(tv) {
            let strval = encode_tv2string(tv, None);
            value = crate::api::private::helpers::cstr_to_obj(strval.as_ptr());
        } else if option_has_bool || option_has_num {
            let n = if option_has_num {
                tv_get_number_chk(&*tv, Some(&mut err))
            } else {
                tv_get_bool_chk(&*tv, Some(&mut err))
            };
            if !err && tv.v_type == VarType::VAR_STRING && n == 0 {
                let mut idx: usize = 0;
                while !tv.vval.v_string().is_null()
                    && crate::memory::cstr_byte_at(tv.vval.v_string(), idx) == b'0'
                {
                    idx += 1;
                }
                if idx == 0 || crate::memory::cstr_byte_at(tv.vval.v_string(), idx) != 0 {
                    err = true;
                    semsg!(
                        gettext(c"E521: Number required: &%s = '%s'"),
                        option,
                        if tv.vval.v_string().is_null() {
                            c"".as_ptr()
                        } else {
                            tv.vval.v_string()
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
            if tv.v_type != VarType::VAR_BOOL && tv.v_type != VarType::VAR_SPECIAL {
                let strval = tv_get_string_buf_chk(&*tv, nbuf.as_mut_ptr());
                err = strval.is_null();
                value = string_obj(cstr_to_string(strval));
            } else {
                err = true;
                emsg(gettext(e_string_required));
            }
        } else {
            std::process::abort();
        }

        {
            *error = err;
        }
        value
    }
}

/// Converts an option value to typval.
pub fn opt_to_tv(value: Object, numbool: bool) -> typval_T {
    let mut rettv = typval_T {
            v_type: VarType::VAR_SPECIAL,
            v_lock: 0,
            vval: TypvalVval::Special(kSpecialVarNull),
        };

        match value.obj_type {
            ObjectType::kObjectTypeUnset => {
                if numbool {
                    rettv.v_type = VarType::VAR_NUMBER;
                    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(kNone));
                }
            }
            ObjectType::kObjectTypeNil => {}
            ObjectType::kObjectTypeBoolean => {
                if numbool {
                    rettv.v_type = VarType::VAR_NUMBER;
                    rettv.vval = crate::eval::typval::TypvalVval::Number(varnumber_T::from(value.data.boolean()));
                } else {
                    rettv.v_type = VarType::VAR_BOOL;
                    rettv.vval = crate::eval::typval::TypvalVval::Bool(if value.data.boolean() {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    });
                }
            }
            ObjectType::kObjectTypeInteger => {
                rettv.v_type = VarType::VAR_NUMBER;
                rettv.vval = crate::eval::typval::TypvalVval::Number(value.data.integer());
            }
            ObjectType::kObjectTypeString => {
                if !value.data.string().data.is_null() {
                    crate::eval::typval::tv_set_string_len(
                        &mut rettv,
                        value.data.string().data,
                        value.data.string().size,
                    );
                } else {
                    rettv.v_type = VarType::VAR_STRING;
                    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                }
            }
            ObjectType::kObjectTypeLuaRef => {
                let s = nlua_funcref_str(value.data.luaref(), true);
                crate::eval::typval::tv_set_string_len(
                    &mut rettv,
                    s.as_ptr(),
                    s.as_bytes().len(),
                );
            }
            _ => {
                std::process::abort();
            }
        }

        rettv
}

/// Set option "varname" to the value of "varp" for the current buffer/window.
fn set_option_from_tv(varname: impl crate::strings::IntoCCharPtr,varp: &mut typval_T) { let varname = varname.into_c_char_ptr();
    let opt_idx = find_option(varname);
        if opt_idx == kOptInvalid {
            semsg!(gettext(e_unknown_option2), varname);
            return;
        }

        let mut error = false;
        let value = opt_from_tv(varp, opt_idx, varname, &mut error);

        if !error {
            let mut errbuf_storage = [0 as c_char; IOSIZE];
            let errbuf = CharBuf {
                data: errbuf_storage.as_mut_ptr(),
                size: IOSIZE,
            };
            let errmsg = set_option_value(opt_idx, value, OPT_LOCAL, true, Some(errbuf));
            if !errmsg.is_null() {
                emsg(errmsg);
            }
        }
        optval_free(value);
}

/// "setwinvar()" and "settabwinvar()" functions.
fn setwinvar(argvars: &mut [typval_T], off: usize) {
    unsafe {
        if check_secure() {
            return;
        }

        let tp = if off == 1 {
            find_tabpage(tv_get_number_chk(&argvars[0], None) as c_int)
        } else {
            g_st().curtab
        };
        let win = find_win_by_nr(&argvars[off], tp);
        let varname = tv_get_string_chk(&argvars[off + 1]);
        let varp = (&raw mut argvars[off + 2]);

        if win.is_null() || varname.is_null() {
            return;
        }

        let need_switch_win = !(tp == g_st().curtab && win == curwin_ptr());
        let mut switchwin: CtxSwitch = CtxSwitch::new();
        if !need_switch_win
            || ctx_switch(
                &mut switchwin,
                win,
                tp,
                ptr::null_mut(),
                kCtxNoEvents | kCtxNoDisplay,
            )
        {
            if crate::memory::cstr_first_byte(varname) == b'&' {
                set_option_from_tv(varname.add(1), &mut *varp);
            } else {
                let varname_len = crate::memory::cstr_len(varname);
                let mut winvarname = Vec::with_capacity(varname_len + 3);
                winvarname.push(b'w' as c_char);
                winvarname.push(b':' as c_char);
                winvarname.extend_from_slice(std::slice::from_raw_parts(varname, varname_len + 1));
                set_var(winvarname.as_ptr(), varname_len + 2, &mut *varp, true);
            }
        }
        if need_switch_win {
            ctx_restore(&mut switchwin);
        }
    }
}

pub fn reset_v_option_vars() {
    set_vim_var_string(VV_OPTION_NEW, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLD, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLDLOCAL, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_OLDGLOBAL, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_COMMAND, ptr::null(), -1);
        set_vim_var_string(VV_OPTION_TYPE, ptr::null(), -1);
}

/// Add an assert error to v:errors.
pub fn assert_error(buf: &[u8]) {
    unsafe {
        let tv = get_vim_var_tv(VV_ERRORS);

        if tv.v_type != VarType::VAR_LIST || tv.vval.v_list().is_null() {
            set_vim_var_list(VV_ERRORS, tv_list_alloc(1));
        }
        tv_list_append_string(
             &mut *get_vim_var_list(VV_ERRORS),
            buf.as_ptr().cast::<c_char>(),
            buf.len() as isize,
        );
    }
}

pub fn var_exists(var: impl crate::strings::IntoCCharPtr) -> bool { let mut var = var.into_c_char_ptr();
     {
        let mut tofree: Option<std::ffi::CString> = None;
        let mut n = false;

        let mut name = var;
        let len = get_name_len(&raw mut var, &mut tofree, true, false);
        if len > 0 {
            let mut tv: typval_T = typval_T::new();

            if let Some(ref tf) = tofree {
                name = tf.as_ptr();
            }
            n = eval_variable(name, len, Some(&mut tv), ptr::null_mut(), false, true);
            if n {
                n = handle_subscript(&raw mut var, &mut tv, Some(&mut g_st().EVALARG_EVALUATE), false);
                if n {
                    tv_clear(&mut tv);
                }
            }
        }
        if !crate::memory::cstr_is_empty(var) {
            n = false;
        }

        drop(tofree);
        n
    }
}

/// Start recording command output to a variable.
pub fn var_redir_start(name: impl crate::strings::IntoCCharPtr, append: bool) -> bool { let name = name.into_c_char_ptr();
    unsafe {
        if !eval_isnamec1(crate::memory::cstr_first_byte(name) as c_int) {
            emsg(gettext(e_invarg));
            return false;
        }

        let name_cstr = cstr_to_cstring(name);
        let name_ptr = name_cstr.as_ptr().cast_mut();
        vars_st().redir_varname = Some(name_cstr);
        vars_st().redir_lval = Some(lval_T::new());
        let lval_ptr: *mut lval_T = vars_st().redir_lval.as_mut().unwrap();

        vars_st().redir_buf.clear();

        vars_st().redir_endp = get_lval(
            name_ptr,
            None,
            &mut *lval_ptr,
            false,
            false,
            0,
            FNE_CHECK_START,
        );
        if vars_st().redir_endp.is_null() || (*lval_ptr).ll_name.is_null() || !crate::memory::cstr_is_empty(vars_st().redir_endp) {
            if !vars_st().redir_endp.is_null() && !crate::memory::cstr_is_empty(vars_st().redir_endp) {
                semsg!(gettext(e_trailing_arg), vars_st().redir_endp);
            } else {
                semsg!(gettext(e_invarg2), name);
            }
            vars_st().redir_endp = ptr::null_mut();
            var_redir_stop();
            return false;
        }

        let called_emsg_before = g_st().called_emsg;
        g_st().did_emsg = 0;
        let mut tv = typval_T {
            v_type: VarType::VAR_STRING,
            v_lock: 0,
            vval: TypvalVval::String(c"".as_ptr().cast_mut()),
        };
        if append {
            set_var_lval(
                &mut *lval_ptr,
                vars_st().redir_endp,
                &mut tv,
                true,
                false,
                c".",
            );
        } else {
            set_var_lval(
                &mut *lval_ptr,
                vars_st().redir_endp,
                &mut tv,
                true,
                false,
                c"=",
            );
        }
        *lval_ptr = lval_T::new();
        if g_st().called_emsg > called_emsg_before {
            vars_st().redir_endp = ptr::null_mut();
            var_redir_stop();
            return false;
        }

        true
    }
}

/// Append "value[value_len]" to the variable set by var_redir_start().
pub fn var_redir_str(value: impl crate::strings::IntoCCharPtr, value_len: c_int) {
    let value = value.into_c_char_ptr();
    unsafe {
        if vars_st().redir_lval.is_none() {
            return;
        }

        let len = if value_len == -1 {
            crate::memory::cstr_len(value)
        } else {
            value_len.max(0) as usize
        };

        if len > 0 {
            let slice = std::slice::from_raw_parts(value, len);
            vars_st().redir_buf.extend_from_slice(slice);
        }
    }
}

/// Stop redirecting command output to a variable.
pub fn var_redir_stop() {
    unsafe {
        let rl = &raw mut vars_st().redir_lval;
        if let Some(ref mut lval) = *rl {
            let lval_ptr: *mut lval_T = lval;
            if !vars_st().redir_endp.is_null() {
                vars_st().redir_buf.push(0);
                let mut tv = typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval::String(vars_st().redir_buf.as_mut_ptr()),
                };
                let vn_ptr = (&raw const vars_st().redir_varname)
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                vars_st().redir_endp = get_lval(
                    vn_ptr,
                    None,
                    &mut *lval_ptr,
                    false,
                    false,
                    0,
                    FNE_CHECK_START,
                );
                if !vars_st().redir_endp.is_null() && !(*lval_ptr).ll_name.is_null() {
                    set_var_lval(
                        &mut *lval_ptr,
                        vars_st().redir_endp,
                        &mut tv,
                        false,
                        false,
                        c".",
                    );
                }
            }

            vars_st().redir_buf = Vec::new();
            *rl = None;
        }
        vars_st().redir_varname = None;
    }
}

/// "gettabvar()" function.
pub fn f_gettabvar(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let varname = tv_get_string_chk(&argvars[1]);
        let tp = find_tabpage(tv_get_number_chk(&argvars[0], None) as c_int);
        let mut win: *mut win_T = ptr::null_mut();

        if !tp.is_null() {
            win = if tp == g_st().curtab || (*tp).tp_firstwin.is_null() {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
        }

        get_var_from(
            varname,
            &mut *rettv,
            &mut argvars[2],
            b't' as c_int,
            tp,
            win,
            ptr::null_mut(),
        );
    }
}

/// "gettabwinvar()" function.
pub fn f_gettabwinvar(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    getwinvar(argvars, rettv, 1);
}

/// "getwinvar()" function.
pub fn f_getwinvar(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    getwinvar(argvars, rettv, 0);
}

/// "getbufvar()" function.
pub fn f_getbufvar(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let varname = tv_get_string_chk(&argvars[1]);
        let buf = tv_get_buf_from_arg(&mut argvars[0]);

        get_var_from(
            varname,
            &mut *rettv,
            &mut argvars[2],
            b'b' as c_int,
            g_st().curtab,
            curwin_ptr(),
            buf,
        );
    }
}

/// "settabvar()" function.
pub fn f_settabvar(argvars: &mut [typval_T], _rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        if check_secure() {
            return;
        }

        let tp = find_tabpage(tv_get_number_chk(&argvars[0], None) as c_int);
        let varname = tv_get_string_chk(&argvars[1]);
        let varp = (&raw mut argvars[2]);

        if varname.is_null() || tp.is_null() {
            return;
        }

        let save_curtab = g_st().curtab;
        let save_lu_tp = g_st().lastused_tabpage;
        goto_tabpage_tp(tp, false, false);

        let varname_len = crate::memory::cstr_len(varname);
        let mut tabvarname = Vec::with_capacity(varname_len + 3);
        tabvarname.push(b't' as c_char);
        tabvarname.push(b':' as c_char);
        tabvarname.extend_from_slice(std::slice::from_raw_parts(varname, varname_len + 1));
        set_var(tabvarname.as_ptr(), varname_len + 2, &mut *varp, true);

        if valid_tabpage(save_curtab) {
            goto_tabpage_tp(save_curtab, false, false);
            if valid_tabpage(save_lu_tp) {
                g_st().lastused_tabpage = save_lu_tp;
            }
        }
    }
}

/// "settabwinvar()" function.
pub fn f_settabwinvar(argvars: &mut [typval_T], _rettv: &mut typval_T, _fptr: EvalFuncData) {
    setwinvar(argvars, 1);
}

/// "setwinvar()" function.
pub fn f_setwinvar(argvars: &mut [typval_T], _rettv: &mut typval_T, _fptr: EvalFuncData) {
    setwinvar(argvars, 0);
}

/// "setbufvar()" function.
pub fn f_setbufvar(argvars: &mut [typval_T], _rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        if check_secure() || !tv_check_str_or_nr(&argvars[0]) {
            return;
        }
        let varname = tv_get_string_chk(&argvars[1]);
        let buf = tv_get_buf(&mut argvars[0], 0);
        let varp = (&raw mut argvars[2]);

        if buf.is_null() || varname.is_null() {
            return;
        }

        if crate::memory::cstr_first_byte(varname) == b'&' {
            let mut aco: CtxSwitch = CtxSwitch::new();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            set_option_from_tv(varname.add(1), &mut *varp);
            ctx_restore(&mut aco);
        } else {
            let varname_len = crate::memory::cstr_len(varname);
            let mut bufvarname = Vec::with_capacity(varname_len + 3);
            let save_curbuf = g_st().curbuf;
            g_st().curbuf = buf;
            bufvarname.push(b'b' as c_char);
            bufvarname.push(b':' as c_char);
            bufvarname.extend_from_slice(std::slice::from_raw_parts(varname, varname_len + 1));
            set_var(bufvarname.as_ptr(), varname_len + 2, &mut *varp, true);
            g_st().curbuf = save_curbuf;
        }
    }
}
