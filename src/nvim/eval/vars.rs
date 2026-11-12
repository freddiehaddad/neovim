//! Functions for dealing with Vimscript variables (`src/nvim/eval/vars.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    DI_FLAGS_FIX, DI_FLAGS_LOCK, DI_FLAGS_RO, SID_LUA, cstr_to_string, nil_obj, string_obj,
    tv_dict_is_watched,
};
use crate::api::private::validate::{Object, ObjectType};
use crate::buffer::{DO_NOT_FREE_CNT, FileBuf, VAR_FIXED, VAR_SCOPE};
use crate::charset::{ascii_isdigit, ascii_iswhite, getdigits_int};
use crate::cmdexpand::Cmdidx;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::errors::*;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::funcs::{
    K_SPECIAL_VAR_NULL_CONST, SpecialVarValue, tv_get_buf, tv_get_buf_from_arg,
};
use crate::eval::typval::{
    BoolVarValue, DI_FLAGS_ALLOC, Dictitem, EvalFuncData, Listitem, ScopeDictDictItem, TV_CSTRING,
    TV_TRANSLATE, TvDict, TvList, TvPartial, TypVal, TypvalVval, VAR_UNLOCKED, VarLockStatus,
    VarType, Varnumber, tv_check_str_or_nr, tv_clear, tv_copy, tv_dict_add, tv_dict_alloc,
    tv_dict_alloc_lock, tv_dict_hi2di, tv_dict_item_alloc, tv_dict_item_remove,
    tv_dict_set_keys_readonly, tv_dict_unref, tv_dict_watcher_notify, tv_get_bool_chk,
    tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk,
    tv_is_func, tv_item_lock, tv_list_alloc, tv_list_append_string, tv_list_append_tv,
    tv_list_find_nr, tv_list_find_str, tv_list_free, tv_list_item_remove, tv_list_len, tv_list_ref,
    tv_list_remove_items, tv_list_set_lock, tv_list_set_ret, value_check_lock,
};
use crate::eval::userfunc::{
    find_hi_in_scoped_ht, find_var_in_scoped_ht, function_exists, get_current_funccal_dict,
    get_funccal_args_dict, get_funccal_args_var, get_funccal_local_dict, get_funccal_local_var,
    list_func_vars,
};
use crate::eval::window::find_win_by_nr;
use crate::eval::{
    AUTOLOAD_CHAR, DICT_MAXNEST, FNE_CHECK_START, FNE_INCL_BR, GLV_QUIET, K_LIST_LEN_UNKNOWN_CONST,
    VAR_DEF_SCOPE, callback_call, eval_isnamec1, eval_lavars_used, eval_option, eval_to_string,
    eval0, eval1, fill_evalarg_from_eap, find_name_end, find_option_var_end, get_env_len, get_lval,
    get_name_len, handle_subscript, may_call_simple_func, num_divide, num_modulus, partial_unref,
    set_eval_lavars_used, set_ref_in_ht, set_var_lval, skip_expr, tv_dict_set_ret,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{Exarg, check_nextcmd, ends_excmd};
use crate::ex_eval::aborting;
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::hashtab::{
    Hashtab, hash_add, hash_clear, hash_find_idx, hash_find_len, hash_lock, hash_remove,
    hash_unlock, hashitem_empty,
};
use crate::lua::executor::{nlua_funcref_str, nlua_set_sctx};
use crate::mbyte::utf_char2bytes;
use crate::memory::bytes_to_cstring;
use crate::message::{
    emsg, internal_error, message_filtered, msg_advance, msg_clr_eos, msg_ext_set_kind,
    msg_outtrans, msg_putchar, msg_puts, msg_puts_len, msg_start, semsg,
};
use crate::option::{
    K_OPT_INVALID, OPT_LOCAL, OptInt, find_option, get_option, get_option_sctx, get_option_value,
    get_tty_option, get_winbuf_options, is_option_hidden, is_tty_option, opt_from_tristate,
    option_has_type, optval_free, set_option_value, set_option_value_give_err,
};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::env::{
    ExpandCtx, os_env_exists, os_getenv_buf, os_unsetenv, vim_getenv, vim_setenv_ext,
    vim_unsetenv_ext,
};
use crate::register::{K_GREG_EXPR_SRC, get_reg_contents_str, write_reg_contents};
use crate::runtime::{Scriptvar, new_script_item, script_autoload, script_item_vars};
use crate::search::set_search_direction;
use crate::strings::{concat_str, gettext};
use crate::version::{highest_patch, min_vim_version};
use crate::window::{
    TabPageInfo, WinInfoInfo, find_tabpage, goto_tabpage_tp, prevwin_curwin, valid_tabpage,
};
use std::ffi::CStr;
use std::ptr;

pub type VimVarIndex = i32;
pub type ScopeType = i32;
pub type Scid = i32;
type ExUnletlockCallback = fn(&mut Lval, &mut Exarg, i32) -> bool;

use crate::api::win_config::{K_CTX_NO_DISPLAY, K_CTX_NO_EVENTS};
use crate::change::MB_MAXCHAR;
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::regexp::NUMBUFLEN;

#[repr(C)]
pub struct Evalarg {
    pub eval_flags: i32,
}

impl Evalarg {
    pub const fn new() -> Self {
        Self { eval_flags: 0 }
    }
}

impl Default for Evalarg {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub enum LvalTv {
    #[default]
    None,
    DictItem(crate::eval::typval::DictitemId),
    ListItem(crate::eval::typval::ListitemId),
}

pub type LvalTvRef<'a> = std::cell::Ref<'a, TypVal>;
pub type LvalTvMut<'a> = std::cell::RefMut<'a, TypVal>;

impl LvalTv {
    #[inline]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::None)
    }

    #[inline]
    pub fn opt(&self) -> Option<LvalTvRef<'_>> {
        match self {
            Self::None => None,
            Self::DictItem(di) => di.opt().map(|d| d.tv()),
            Self::ListItem(li) => li.opt().map(|l| l.tv()),
        }
    }

    #[inline]
    pub fn opt_mut(&self) -> Option<LvalTvMut<'_>> {
        match self {
            Self::None => None,
            Self::DictItem(di) => di.opt().map(|d| d.tv_mut()),
            Self::ListItem(li) => li.opt().map(|l| l.tv_mut()),
        }
    }

    #[inline]
    pub fn borrow(&self) -> LvalTvRef<'_> {
        self.opt().expect("null LvalTv")
    }

    #[inline]
    pub fn borrow_mut(&self) -> LvalTvMut<'_> {
        self.opt_mut().expect("null LvalTv")
    }
}

#[repr(C)]
pub struct Lval {
    pub ll_name: Option<std::ffi::CString>,
    pub ll_name_off: usize,
    pub ll_name_len: usize,
    pub ll_exp_name: Option<std::ffi::CString>,
    pub ll_tv: LvalTv,
    pub ll_li: crate::eval::typval::ListitemId,
    pub ll_list: crate::eval::typval::ListId,
    pub ll_range: bool,
    pub ll_empty2: bool,
    pub ll_n1: i32,
    pub ll_n2: i32,
    pub ll_dict: crate::eval::typval::DictId,
    pub ll_di: crate::eval::typval::DictitemId,
    pub ll_newkey: Option<std::ffi::CString>,
    pub ll_blob: crate::eval::typval::BlobId,
}

impl Lval {
    pub const fn new() -> Self {
        Self {
            ll_name: None,
            ll_name_off: 0,
            ll_name_len: 0,
            ll_exp_name: None,
            ll_tv: LvalTv::None,
            ll_li: crate::eval::typval::ListitemId::NULL,
            ll_list: crate::eval::typval::ListId::NULL,
            ll_range: false,
            ll_empty2: false,
            ll_n1: 0,
            ll_n2: 0,
            ll_dict: crate::eval::typval::DictId::NULL,
            ll_di: crate::eval::typval::DictitemId::NULL,
            ll_newkey: None,
            ll_blob: crate::eval::typval::BlobId::NULL,
        }
    }

    #[inline]
    pub fn ll_name_cstr(&self) -> Option<&std::ffi::CStr> {
        self.ll_name
            .as_deref()
            .map(|s| crate::memory::CStrSubslice::advance(s, self.ll_name_off))
    }

    #[inline]
    pub fn set_ll_name(&mut self, s: &std::ffi::CStr) {
        self.ll_name = Some(s.to_owned());
        self.ll_name_off = 0;
    }

    #[inline]
    pub fn advance_ll_name(&mut self, n: usize) {
        self.ll_name_off += n;
    }
}

impl Default for Lval {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) const MAXCOL: Varnumber = 0x7fffffff;

const VV_COMPAT: u8 = 1;
const VV_RO: u8 = 2;
const VV_RO_SBX: u8 = 4;

const DI_FLAGS_RO_SBX: u8 = 2;

pub(crate) const VAR_TYPE_NUMBER: Varnumber = 0;
pub(crate) const VAR_TYPE_STRING: Varnumber = 1;
pub(crate) const VAR_TYPE_FUNC: Varnumber = 2;
pub(crate) const VAR_TYPE_LIST: Varnumber = 3;
pub(crate) const VAR_TYPE_DICT: Varnumber = 4;
pub(crate) const VAR_TYPE_FLOAT: Varnumber = 5;
pub(crate) const VAR_TYPE_BOOL: Varnumber = 6;
pub(crate) const VAR_TYPE_SPECIAL: Varnumber = 7;
pub(crate) const VAR_TYPE_BLOB: Varnumber = 10;

pub(crate) const ENV_STARTREASON: &CStr = c"__NVIM_STARTREASON";

pub(crate) const SID_STR: Scid = -10;

pub(crate) const BAD_KEEP: i32 = -1;
pub(crate) const BAD_DROP: i32 = -2;

const K_OPT_CHARCONVERT: OptIndex = 36;
const K_OPT_DIFFEXPR: OptIndex = 72;
const K_OPT_PATCHEXPR: OptIndex = 217;
const K_OPT_SPELLSUGGEST: OptIndex = 292;
pub(crate) const K_OPT_FLAG_FUNC: u32 = 1 << 24;

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

#[derive(Clone, Copy)]
struct VimVarMeta {
    vv_name: &'static CStr,
    vv_type: VarType,
    vv_flags: u8,
}

const fn make_vimvar(name: &'static CStr, v_type: VarType, flags: u8) -> VimVarMeta {
    VimVarMeta {
        vv_name: name,
        vv_type: v_type,
        vv_flags: flags,
    }
}

const VIMVARS_META: [VimVarMeta; VV_LEN] = [
    make_vimvar(c"count", VarType::VarNumber, VV_RO),
    make_vimvar(c"count1", VarType::VarNumber, VV_RO),
    make_vimvar(c"prevcount", VarType::VarNumber, VV_RO),
    make_vimvar(c"errmsg", VarType::VarString, 0),
    make_vimvar(c"warningmsg", VarType::VarString, 0),
    make_vimvar(c"statusmsg", VarType::VarString, 0),
    make_vimvar(c"shell_error", VarType::VarNumber, VV_RO),
    make_vimvar(c"this_session", VarType::VarString, 0),
    make_vimvar(c"version", VarType::VarNumber, VV_COMPAT + VV_RO),
    make_vimvar(c"lnum", VarType::VarNumber, VV_RO_SBX),
    make_vimvar(c"termrequest", VarType::VarString, VV_RO),
    make_vimvar(c"termresponse", VarType::VarString, VV_RO),
    make_vimvar(c"fname", VarType::VarString, VV_RO),
    make_vimvar(c"lang", VarType::VarString, VV_RO),
    make_vimvar(c"lc_time", VarType::VarString, VV_RO),
    make_vimvar(c"ctype", VarType::VarString, VV_RO),
    make_vimvar(c"charconvert_from", VarType::VarString, VV_RO),
    make_vimvar(c"charconvert_to", VarType::VarString, VV_RO),
    make_vimvar(c"fname_in", VarType::VarString, VV_RO),
    make_vimvar(c"fname_out", VarType::VarString, VV_RO),
    make_vimvar(c"fname_new", VarType::VarString, VV_RO),
    make_vimvar(c"fname_diff", VarType::VarString, VV_RO),
    make_vimvar(c"cmdarg", VarType::VarString, VV_RO),
    make_vimvar(c"foldstart", VarType::VarNumber, VV_RO_SBX),
    make_vimvar(c"foldend", VarType::VarNumber, VV_RO_SBX),
    make_vimvar(c"folddashes", VarType::VarString, VV_RO_SBX),
    make_vimvar(c"foldlevel", VarType::VarNumber, VV_RO_SBX),
    make_vimvar(c"progname", VarType::VarString, VV_RO),
    make_vimvar(c"servername", VarType::VarString, VV_RO),
    make_vimvar(c"dying", VarType::VarNumber, VV_RO),
    make_vimvar(c"exception", VarType::VarString, VV_RO),
    make_vimvar(c"throwpoint", VarType::VarString, VV_RO),
    make_vimvar(c"register", VarType::VarString, VV_RO),
    make_vimvar(c"cmdbang", VarType::VarNumber, VV_RO),
    make_vimvar(c"insertmode", VarType::VarString, VV_RO),
    make_vimvar(c"val", VarType::VarUnknown, VV_RO),
    make_vimvar(c"key", VarType::VarUnknown, VV_RO),
    make_vimvar(c"profiling", VarType::VarNumber, VV_RO),
    make_vimvar(c"fcs_reason", VarType::VarString, VV_RO),
    make_vimvar(c"fcs_choice", VarType::VarString, 0),
    make_vimvar(c"beval_bufnr", VarType::VarNumber, VV_RO),
    make_vimvar(c"beval_winnr", VarType::VarNumber, VV_RO),
    make_vimvar(c"beval_winid", VarType::VarNumber, VV_RO),
    make_vimvar(c"beval_lnum", VarType::VarNumber, VV_RO),
    make_vimvar(c"beval_col", VarType::VarNumber, VV_RO),
    make_vimvar(c"beval_text", VarType::VarString, VV_RO),
    make_vimvar(c"scrollstart", VarType::VarString, 0),
    make_vimvar(c"swapname", VarType::VarString, VV_RO),
    make_vimvar(c"swapchoice", VarType::VarString, 0),
    make_vimvar(c"swapcommand", VarType::VarString, VV_RO),
    make_vimvar(c"char", VarType::VarString, 0),
    make_vimvar(c"mouse_win", VarType::VarNumber, 0),
    make_vimvar(c"mouse_winid", VarType::VarNumber, 0),
    make_vimvar(c"mouse_lnum", VarType::VarNumber, 0),
    make_vimvar(c"mouse_col", VarType::VarNumber, 0),
    make_vimvar(c"operator", VarType::VarString, VV_RO),
    make_vimvar(c"searchforward", VarType::VarNumber, 0),
    make_vimvar(c"hlsearch", VarType::VarNumber, 0),
    make_vimvar(c"oldfiles", VarType::VarList, 0),
    make_vimvar(c"windowid", VarType::VarNumber, VV_RO_SBX),
    make_vimvar(c"progpath", VarType::VarString, VV_RO),
    make_vimvar(c"completed_item", VarType::VarDict, 0),
    make_vimvar(c"option_new", VarType::VarString, VV_RO),
    make_vimvar(c"option_old", VarType::VarString, VV_RO),
    make_vimvar(c"option_oldlocal", VarType::VarString, VV_RO),
    make_vimvar(c"option_oldglobal", VarType::VarString, VV_RO),
    make_vimvar(c"option_command", VarType::VarString, VV_RO),
    make_vimvar(c"option_type", VarType::VarString, VV_RO),
    make_vimvar(c"errors", VarType::VarList, 0),
    make_vimvar(c"false", VarType::VarBool, VV_RO),
    make_vimvar(c"true", VarType::VarBool, VV_RO),
    make_vimvar(c"null", VarType::VarSpecial, VV_RO),
    make_vimvar(c"numbermax", VarType::VarNumber, VV_RO),
    make_vimvar(c"numbermin", VarType::VarNumber, VV_RO),
    make_vimvar(c"numbersize", VarType::VarNumber, VV_RO),
    make_vimvar(c"vim_did_enter", VarType::VarNumber, VV_RO),
    make_vimvar(c"testing", VarType::VarNumber, 0),
    make_vimvar(c"t_number", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_string", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_func", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_list", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_dict", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_float", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_bool", VarType::VarNumber, VV_RO),
    make_vimvar(c"t_blob", VarType::VarNumber, VV_RO),
    make_vimvar(c"event", VarType::VarDict, VV_RO),
    make_vimvar(c"versionlong", VarType::VarNumber, VV_RO),
    make_vimvar(c"echospace", VarType::VarNumber, VV_RO),
    make_vimvar(c"argf", VarType::VarList, VV_RO),
    make_vimvar(c"argv", VarType::VarList, VV_RO),
    make_vimvar(c"collate", VarType::VarString, VV_RO),
    make_vimvar(c"exiting", VarType::VarNumber, VV_RO),
    make_vimvar(c"maxcol", VarType::VarNumber, VV_RO),
    make_vimvar(c"stacktrace", VarType::VarList, VV_RO),
    make_vimvar(c"vim_did_init", VarType::VarNumber, VV_RO),
    make_vimvar(c"stderr", VarType::VarNumber, VV_RO),
    make_vimvar(c"msgpack_types", VarType::VarDict, VV_RO),
    make_vimvar(c"_null_string", VarType::VarString, VV_RO),
    make_vimvar(c"_null_list", VarType::VarList, VV_RO),
    make_vimvar(c"_null_dict", VarType::VarDict, VV_RO),
    make_vimvar(c"_null_blob", VarType::VarBlob, VV_RO),
    make_vimvar(c"lua", VarType::VarPartial, VV_RO),
    make_vimvar(c"relnum", VarType::VarNumber, VV_RO),
    make_vimvar(c"virtnum", VarType::VarNumber, VV_RO),
    make_vimvar(c"starttime", VarType::VarNumber, VV_RO),
    make_vimvar(c"exitreason", VarType::VarString, VV_RO),
    make_vimvar(c"useractive", VarType::VarNumber, VV_RO),
    make_vimvar(c"startreason", VarType::VarString, VV_RO),
    make_vimvar(c"cascading", VarType::VarBool, VV_RO),
];

struct VarsState {
    vv_di: [crate::globals::CloneCell<crate::eval::typval::DictitemId>; VV_LEN],
    globvars_var: crate::globals::CloneCell<crate::eval::typval::DictitemId>,
    globvardict: crate::globals::CloneCell<crate::eval::typval::DictId>,
    vimvars_var: crate::globals::CloneCell<crate::eval::typval::DictitemId>,
    vimvardict: crate::globals::CloneCell<crate::eval::typval::DictId>,
    compat_dict: crate::globals::CloneCell<crate::eval::typval::DictId>,
    redir_active: std::cell::Cell<bool>,
    gdone: std::cell::Cell<usize>,
    bdone: std::cell::Cell<usize>,
    wdone: std::cell::Cell<usize>,
    tdone: std::cell::Cell<usize>,
    vidx: std::cell::Cell<usize>,
    hi_idx: std::cell::Cell<usize>,
}

impl VarsState {
    const fn new() -> Self {
        Self {
            vv_di: [const { crate::globals::CloneCell::new(crate::eval::typval::DictitemId::NULL) };
                VV_LEN],
            globvars_var: crate::globals::CloneCell::new(crate::eval::typval::DictitemId::NULL),
            globvardict: crate::globals::CloneCell::new(crate::eval::typval::DictId::NULL),
            vimvars_var: crate::globals::CloneCell::new(crate::eval::typval::DictitemId::NULL),
            vimvardict: crate::globals::CloneCell::new(crate::eval::typval::DictId::NULL),
            compat_dict: crate::globals::CloneCell::new(crate::eval::typval::DictId::NULL),
            redir_active: std::cell::Cell::new(false),
            gdone: std::cell::Cell::new(0),
            bdone: std::cell::Cell::new(0),
            wdone: std::cell::Cell::new(0),
            tdone: std::cell::Cell::new(0),
            vidx: std::cell::Cell::new(0),
            hi_idx: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static VARS_STATE: VarsState = const { VarsState::new() };
    static REDIR_BUF: std::cell::RefCell<Vec<u8>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static REDIR_ENDP: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
    static REDIR_VARNAME: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
}

const MSGPACK_TYPE_NAMES: [&CStr; NUM_MSGPACK_TYPES] = [
    c"nil", c"boolean", c"integer", c"float", c"string", c"array", c"map", c"ext",
];

std::thread_local! {
    static EVAL_MSGPACK_TYPE_LISTS: std::cell::RefCell<[crate::eval::typval::ListId; NUM_MSGPACK_TYPES]> =
        const { std::cell::RefCell::new([crate::eval::typval::ListId::NULL; NUM_MSGPACK_TYPES]) };
}

#[inline]
pub(crate) fn eval_msgpack_type_list(idx: usize) -> crate::eval::typval::ListId {
    EVAL_MSGPACK_TYPE_LISTS.with(|l| l.borrow()[idx].clone())
}

#[inline]
pub fn tv_list_locked(l: Option<&TvList>) -> VarLockStatus {
    l.map_or(VAR_FIXED, |l| l.lv_lock.get())
}

#[inline]
fn script_vars_dict(id: Scid) -> crate::eval::typval::DictId {
    script_item_vars(id).1
}

#[inline]
pub fn tristate_from_int(val: OptInt) -> TriState {
    if val == 0 {
        K_FALSE
    } else if val >= 1 {
        K_TRUE
    } else {
        K_NONE
    }
}

use crate::api::private::helpers::integer_obj;

#[inline]
fn get_globvars_var() -> crate::eval::typval::DictitemId {
    let mut id = VARS_STATE.with(|s| s.globvars_var.get());
    if id.is_null() {
        id = crate::eval::typval::tv_dict_item_alloc_static();
        VARS_STATE.with(|s| s.globvars_var.set(id.clone()));
    }
    id
}

#[inline]
fn get_vimvars_var() -> crate::eval::typval::DictitemId {
    let mut id = VARS_STATE.with(|s| s.vimvars_var.get());
    if id.is_null() {
        id = crate::eval::typval::tv_dict_item_alloc_static();
        VARS_STATE.with(|s| s.vimvars_var.set(id.clone()));
    }
    id
}

#[inline]
fn get_compat_dict() -> crate::eval::typval::DictId {
    let mut id = VARS_STATE.with(|s| s.compat_dict.get());
    if id.is_null() {
        let d = crate::eval::typval::tv_dict_alloc_static();
        d.dv_refcount.set(DO_NOT_FREE_CNT);
        id = d;
        VARS_STATE.with(|s| s.compat_dict.set(id.clone()));
    }
    id
}

#[inline]
fn vimvar_di(idx: usize) -> crate::eval::typval::DictitemId {
    let mut id = VARS_STATE.with(|s| s.vv_di[idx].get());
    if id.is_null() {
        let di = crate::eval::typval::tv_dict_item_alloc_static_key(
            VIMVARS_META[idx].vv_name.to_bytes_with_nul().to_vec(),
        );
        di.tv_mut().v_type = VIMVARS_META[idx].vv_type;
        id = di;
        VARS_STATE.with(|s| s.vv_di[idx].set(id.clone()));
    }
    id
}

pub fn evalvars_init() {
    init_var_dict(&get_globvar_dict(), &get_globvars_var(), VAR_DEF_SCOPE);
    init_var_dict(&get_vimvar_dict(), &get_vimvars_var(), VAR_SCOPE);
    get_vimvar_dict().dv_lock.set(VAR_FIXED);
    hash_clear(&get_compat_dict().dv_hashtab);

    for (i, vv) in VIMVARS_META.iter().enumerate() {
        let vv_flags = vv.vv_flags;
        let di = vimvar_di(i);
        if (vv_flags & VV_RO) != 0 {
            di.di_flags.set(DI_FLAGS_RO | DI_FLAGS_FIX);
        } else if (vv_flags & VV_RO_SBX) != 0 {
            di.di_flags.set(DI_FLAGS_RO_SBX | DI_FLAGS_FIX);
        } else {
            di.di_flags.set(DI_FLAGS_FIX);
        }

        if di.tv().v_type != VarType::VarUnknown {
            hash_add(&get_vimvar_dict().dv_hashtab, &di);
        }
        if (vv_flags & VV_COMPAT) != 0 {
            hash_add(&get_compat_dict().dv_hashtab, &di);
        }
    }
    let vim_version = Varnumber::from(min_vim_version());
    set_vim_var_nr(VV_VERSION, vim_version);
    set_vim_var_nr(
        VV_VERSIONLONG,
        vim_version * 10000 + Varnumber::from(highest_patch()),
    );

    let msgpack_types_dict = tv_dict_alloc();
    for (i, type_name) in MSGPACK_TYPE_NAMES.iter().enumerate() {
        let type_list = tv_list_alloc(0);
        tv_list_set_lock(Some(&type_list), VAR_FIXED);
        tv_list_ref(Some(&type_list));
        let di = tv_dict_item_alloc(type_name);
        di.di_flags
            .set(di.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);
        di.set_tv(TypVal {
            v_type: VarType::VarList,
            v_lock: 0,
            vval: TypvalVval::List(type_list.clone()),
        });
        EVAL_MSGPACK_TYPE_LISTS.with(|l| l.borrow_mut()[i] = type_list);
        if !tv_dict_add(&msgpack_types_dict, &di) {
            std::process::abort();
        }
    }
    msgpack_types_dict.dv_lock.set(VAR_FIXED);

    set_vim_var_dict(VV_MSGPACK_TYPES, Some(&msgpack_types_dict));
    set_vim_var_dict(VV_COMPLETED_ITEM, Some(&tv_dict_alloc_lock(VAR_FIXED)));

    set_vim_var_dict(VV_EVENT, Some(&tv_dict_alloc_lock(VAR_FIXED)));
    set_vim_var_list(VV_ERRORS, Some(&tv_list_alloc(K_LIST_LEN_UNKNOWN_CONST)));
    set_vim_var_nr(VV_STDERR, crate::channel::CHAN_STDERR as Varnumber);
    set_vim_var_nr(VV_SEARCHFORWARD, 1);
    set_vim_var_nr(VV_HLSEARCH, 1);
    set_vim_var_nr(VV_COUNT1, 1);
    set_vim_var_string(VV_STARTREASON, Some(c"normal"), 6);
    set_vim_var_special(VV_EXITING, K_SPECIAL_VAR_NULL_CONST);

    set_vim_var_nr(VV_TYPE_NUMBER, VAR_TYPE_NUMBER);
    set_vim_var_nr(VV_TYPE_STRING, VAR_TYPE_STRING);
    set_vim_var_nr(VV_TYPE_FUNC, VAR_TYPE_FUNC);
    set_vim_var_nr(VV_TYPE_LIST, VAR_TYPE_LIST);
    set_vim_var_nr(VV_TYPE_DICT, VAR_TYPE_DICT);
    set_vim_var_nr(VV_TYPE_FLOAT, VAR_TYPE_FLOAT);
    set_vim_var_nr(VV_TYPE_BOOL, VAR_TYPE_BOOL);
    set_vim_var_nr(VV_TYPE_BLOB, VAR_TYPE_BLOB);

    set_vim_var_bool(VV_FALSE, K_BOOL_VAR_FALSE_CONST);
    set_vim_var_bool(VV_TRUE, K_BOOL_VAR_TRUE_CONST);
    set_vim_var_special(VV_NULL, K_SPECIAL_VAR_NULL_CONST);
    set_vim_var_nr(VV_NUMBERMAX, Varnumber::MAX);
    set_vim_var_nr(VV_NUMBERMIN, Varnumber::MIN);
    set_vim_var_nr(
        VV_NUMBERSIZE,
        (std::mem::size_of::<Varnumber>() * 8) as Varnumber,
    );
    set_vim_var_nr(VV_MAXCOL, MAXCOL);

    set_vim_var_nr(VV_ECHOSPACE, Varnumber::from(g_st().sc_col.get() - 1));

    let vvlua_pt = crate::eval::typval::tv_partial_alloc();
    vvlua_pt.set_name(Some(std::ffi::CString::default()));
    vvlua_pt.pt_refcount.set(1);
    set_vim_var_partial(VV_LUA, Some(vvlua_pt));

    set_reg_var(0);

    let mut startreason_buf = [0u8; 32];
    let startreason = os_getenv_buf(ENV_STARTREASON, &mut startreason_buf);
    if matches!(
        startreason.map(CStr::to_bytes),
        Some(b"restart!" | b"restart" | b"remote-ssh")
    ) {
        set_vim_var_string(VV_STARTREASON, startreason, -1);
    }
    if os_env_exists(ENV_STARTREASON, false) {
        os_unsetenv(ENV_STARTREASON);
    }
}

pub fn evalvars_clear() {
    for i in 0..VV_LEN {
        let di = vimvar_di(i);
        let vt = di.tv().v_type;
        if vt == VarType::VarString || vt == VarType::VarList {
            di.clear_tv();
        }
    }

    partial_unref(get_vim_var_partial(VV_LUA).as_deref());
    set_vim_var_partial(VV_LUA, None);
    hash_clear(&get_vimvar_dict().dv_hashtab);
    hash_clear(&get_compat_dict().dv_hashtab);

    vars_clear(&get_globvar_dict().dv_hashtab);

    for i in 1..=crate::runtime::script_items_len() {
        vars_clear(&script_vars_dict(i).dv_hashtab);
    }
}

pub fn garbage_collect_globvars(copy_id: i32) -> i32 {
    i32::from(set_ref_in_ht(
        Some(&get_globvar_dict().dv_hashtab),
        copy_id,
        None,
    ))
}

pub fn garbage_collect_vimvars(copy_id: i32) -> bool {
    set_ref_in_ht(Some(&get_vimvar_dict().dv_hashtab), copy_id, None)
}

pub fn garbage_collect_scriptvars(copy_id: i32) -> bool {
    let mut abort = false;

    for i in 1..=crate::runtime::script_items_len() {
        abort = abort || set_ref_in_ht(Some(&script_vars_dict(i).dv_hashtab), copy_id, None);
    }

    abort
}

/// Set an internal variable to a string value.
pub fn set_internal_string_var(name: &std::ffi::CStr, value: &CStr) {
    let mut tv = TypVal {
        v_type: VarType::VarString,
        v_lock: 0,
        vval: TypvalVval::from_cstr(value),
    };

    set_var(name.to_bytes(), &mut tv, true);
}

pub fn eval_charconvert(
    enc_from: &CStr,
    enc_to: &CStr,
    fname_from: &CStr,
    fname_to: &CStr,
) -> bool {
    let saved_sctx = g_st().current_sctx.get();

    set_vim_var_string(VV_CC_FROM, Some(enc_from), -1);
    set_vim_var_string(VV_CC_TO, Some(enc_to), -1);
    set_vim_var_string(VV_FNAME_IN, Some(fname_from), -1);
    set_vim_var_string(VV_FNAME_OUT, Some(fname_to), -1);
    let ctx = get_option_sctx(K_OPT_CHARCONVERT);
    g_st().current_sctx.set(ctx);

    let mut err = false;
    let mut tv: TypVal = TypVal::new();
    let mut argv: [TypVal; 1] = [TypVal::new()];
    if !callback_call(&opt_vars().p_ccv, 0, &mut argv, &mut tv) {
        err = true;
    } else {
        if tv_get_number_chk(&tv, None) != 0 {
            err = true;
        }
        tv_clear(&mut tv);
    }

    set_vim_var_string(VV_CC_FROM, None, -1);
    set_vim_var_string(VV_CC_TO, None, -1);
    set_vim_var_string(VV_FNAME_IN, None, -1);
    set_vim_var_string(VV_FNAME_OUT, None, -1);
    g_st().current_sctx.set(saved_sctx);

    !err
}

pub fn eval_diff(origfile: &CStr, newfile: &CStr, outfile: &CStr) {
    let saved_sctx = g_st().current_sctx.get();
    set_vim_var_string(VV_FNAME_IN, Some(origfile), -1);
    set_vim_var_string(VV_FNAME_NEW, Some(newfile), -1);
    set_vim_var_string(VV_FNAME_OUT, Some(outfile), -1);

    let ctx = get_option_sctx(K_OPT_DIFFEXPR);
    g_st().current_sctx.set(ctx);

    let mut tv: TypVal = TypVal::new();
    let mut argv: [TypVal; 1] = [TypVal::new()];
    if callback_call(&opt_vars().p_dex, 0, &mut argv, &mut tv) {
        tv_clear(&mut tv);
    }

    set_vim_var_string(VV_FNAME_IN, None, -1);
    set_vim_var_string(VV_FNAME_NEW, None, -1);
    set_vim_var_string(VV_FNAME_OUT, None, -1);
    g_st().current_sctx.set(saved_sctx);
}

pub fn eval_patch(origfile: &CStr, difffile: &CStr, outfile: &CStr) {
    let saved_sctx = g_st().current_sctx.get();
    set_vim_var_string(VV_FNAME_IN, Some(origfile), -1);
    set_vim_var_string(VV_FNAME_DIFF, Some(difffile), -1);
    set_vim_var_string(VV_FNAME_OUT, Some(outfile), -1);

    let ctx = get_option_sctx(K_OPT_PATCHEXPR);
    g_st().current_sctx.set(ctx);

    let mut tv: TypVal = TypVal::new();
    let mut argv: [TypVal; 1] = [TypVal::new()];
    if callback_call(&opt_vars().p_pex, 0, &mut argv, &mut tv) {
        tv_clear(&mut tv);
    }

    set_vim_var_string(VV_FNAME_IN, None, -1);
    set_vim_var_string(VV_FNAME_DIFF, None, -1);
    set_vim_var_string(VV_FNAME_OUT, None, -1);
    g_st().current_sctx.set(saved_sctx);
}

/// Evaluate an expression to a list with suggestions.
pub fn eval_spell_expr(badword: &CStr, expr: &CStr) -> Option<crate::eval::typval::ListId> {
    let mut save_val: TypVal = TypVal::new();
    let mut rettv: TypVal = TypVal::new();
    let mut list = crate::eval::typval::ListId::NULL;
    let mut p = crate::charset::skipwhite(expr);
    let saved_sctx = g_st().current_sctx.get();

    prepare_vimvar(VV_VAL, &mut save_val);
    set_vim_var_string(VV_VAL, Some(badword), -1);
    if opt_vars().p_verbose.get() == 0 {
        g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    }
    let ctx = get_option_sctx(K_OPT_SPELLSUGGEST);
    g_st().current_sctx.set(ctx);

    let r = may_call_simple_func(p, &mut rettv)
        .unwrap_or_else(|| eval1(&mut p, &mut rettv, Some(&mut Evalarg { eval_flags: 1 })));
    if r {
        if rettv.v_type != VarType::VarList {
            tv_clear(&mut rettv);
        } else {
            list = rettv.vval.v_list();
        }
    }

    if opt_vars().p_verbose.get() == 0 {
        g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    }
    get_vim_var_tv(VV_VAL).clear_tv();
    restore_vimvar(VV_VAL, &mut save_val);
    g_st().current_sctx.set(saved_sctx);

    if list.is_null() { None } else { Some(list) }
}

/// Get spell word from an entry from spellsuggest=expr:
pub fn get_spellword(list: &TvList, ret_word: &mut Option<std::borrow::Cow<'static, CStr>>) -> i32 {
    if tv_list_len(Some(list)) != 2 {
        emsg(gettext(
            c"E5700: Expression from 'spellsuggest' must yield lists with exactly two values",
        ));
        return -1;
    }
    *ret_word = tv_list_find_str(Some(list), 0);
    if ret_word.is_none() {
        return -1;
    }
    tv_list_find_nr(Some(list), -1, None) as i32
}

/// Prepare v: variable "idx" to be used.
pub fn prepare_vimvar(idx: i32, save_tv: &mut TypVal) {
    let di = vimvar_di(idx as usize);
    let was_unknown = {
        let mut tv = di.tv_mut();
        *save_tv = tv.clone();
        tv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
        tv.v_type == VarType::VarUnknown
    };
    if was_unknown {
        hash_add(&get_vimvar_dict().dv_hashtab, &di);
    }
}

/// Restore v: variable "idx" to typeval "save_tv".
pub fn restore_vimvar(idx: i32, save_tv: &mut TypVal) {
    let di = vimvar_di(idx as usize);
    di.set_tv(std::mem::take(save_tv));
    if di.tv().v_type != VarType::VarUnknown {
        return;
    }

    if let Some(idx) = get_vimvar_dict().dv_hashtab.find_hi_idx(di.key_cstr()) {
        hash_remove(&get_vimvar_dict().dv_hashtab, idx);
    } else {
        internal_error(c"restore_vimvar()");
    }
}

/// List Vim variables.
fn list_vim_vars(first: &mut i32) {
    list_hashtable_vars(
        &get_vimvar_dict().dv_hashtab,
        c"v:",
        false as i32,
        &mut *first,
    );
}

/// List script-local variables, if there is a script.
fn list_script_vars(first: &mut i32) {
    if crate::runtime::script_id_valid(g_st().current_sctx.get().sc_sid) {
        list_hashtable_vars(
            &script_vars_dict(g_st().current_sctx.get().sc_sid).dv_hashtab,
            c"s:",
            false as i32,
            &mut *first,
        );
    }
}

/// Evaluate one Vim expression {expr} in string "p" and append the resulting string to "gap".
pub fn eval_one_expr_in_str<'a>(
    p_in: &'a CStr,
    gap: &mut Vec<u8>,
    evaluate: bool,
) -> Option<&'a CStr> {
    let block_start = crate::charset::skipwhite_at(p_in, 1);

    if block_start.is_empty() {
        semsg!(gettext(E_MISSING_CLOSE_CURLY_STR), p_in);
        return None;
    }
    let mut block_end_cstr = block_start;
    if !skip_expr(&mut block_end_cstr, None) {
        return None;
    }
    let block_end = crate::charset::skipwhite(block_end_cstr);
    if block_end.to_bytes().first() != Some(&b'}') {
        semsg!(gettext(E_MISSING_CLOSE_CURLY_STR), p_in);
        return None;
    }
    if evaluate {
        let expr_len = (block_end.as_ptr() as usize).wrapping_sub(block_start.as_ptr() as usize);
        let expr_cstr = bytes_to_cstring(&block_start.to_bytes()[..expr_len]);
        let expr_val = eval_to_string(&expr_cstr, false, false)?;
        let bytes = expr_val.as_bytes();
        if !bytes.is_empty() {
            gap.extend_from_slice(bytes);
        }
    }

    Some(&block_end[1..])
}

/// Evaluate all the Vim expressions {expr} in "str" and return the resulting string as an owned `CString`.
fn eval_all_expr_in_str(str_arg_in: &CStr) -> Option<std::ffi::CString> {
    let str_arg = str_arg_in.as_ptr().cast::<u8>();
    let full_bytes = str_arg_in.to_bytes();
    let mut rem = full_bytes;
    let mut buf: Vec<u8> = Vec::with_capacity(80);

    while !rem.is_empty() {
        let mut escaped_brace = false;
        let lit_len = rem.iter().take_while(|&&b| b != b'{' && b != b'}').count();
        let after_lit = &rem[lit_len..];

        let copy_len = if !after_lit.is_empty() && after_lit.get(1) == Some(&after_lit[0]) {
            escaped_brace = true;
            lit_len + 1
        } else if after_lit.first() == Some(&b'}') {
            semsg!(gettext(E_STRAY_CLOSING_CURLY_STR), str_arg_in);
            return None;
        } else {
            lit_len
        };

        if copy_len > 0 {
            buf.extend_from_slice(&rem[..copy_len]);
        }
        rem = &rem[copy_len..];

        if rem.is_empty() {
            break;
        }

        if escaped_brace {
            rem = &rem[1..];
            continue;
        }

        let p_off = full_bytes.len() - rem.len();
        let next_p = eval_one_expr_in_str(&str_arg_in[p_off..], &mut buf, true)?;
        let consumed = (next_p.as_ptr() as usize).wrapping_sub(str_arg as usize);
        rem = &full_bytes[consumed.min(full_bytes.len())..];
    }

    Some(bytes_to_cstring(&buf))
}

/// Get a list of lines from a HERE document.
pub fn heredoc_get(
    eap: &mut Exarg,
    cmd_in: &CStr,
    script_get: bool,
) -> Option<crate::eval::typval::ListId> {
    let mut marker_indent_len: i32 = 0;
    let mut text_indent_len: i32 = 0;
    let mut text_indent: Option<Vec<u8>> = None;
    let full_cmd_bytes = cmd_in.to_bytes();
    let cmd_line_len = full_cmd_bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(full_cmd_bytes.len());
    let heredoc_in_string = cmd_line_len < full_cmd_bytes.len();
    let mut line_arg_cstr: Option<&CStr> = if heredoc_in_string {
        Some(&cmd_in[cmd_line_len + 1..])
    } else {
        if eap.ea_getline.is_none() {
            emsg(gettext(E_CANNOT_USE_HEREDOC_HERE));
            return None;
        }
        None
    };

    let mut cmd_rem = crate::charset::skipwhite_slice(&full_cmd_bytes[..cmd_line_len]);
    let mut evalstr = false;
    let mut eval_failed = false;
    loop {
        if cmd_rem.starts_with(b"trim") && cmd_rem.get(4).is_none_or(|&b| ascii_iswhite(b as i32)) {
            cmd_rem = crate::charset::skipwhite_slice(&cmd_rem[4..]);

            marker_indent_len = eap
                .cmdline_cstr()
                .to_bytes()
                .iter()
                .take_while(|&&b| ascii_iswhite(b as i32))
                .count() as i32;
            text_indent_len = -1;
            continue;
        }
        if cmd_rem.starts_with(b"eval") && cmd_rem.get(4).is_none_or(|&b| ascii_iswhite(b as i32)) {
            cmd_rem = crate::charset::skipwhite_slice(&cmd_rem[4..]);
            evalstr = true;
            continue;
        }
        break;
    }

    let comment_char = b'"';
    let marker_cstr = if !cmd_rem.is_empty() && cmd_rem[0] != comment_char {
        let marker_start = crate::charset::skipwhite_slice(cmd_rem);
        let after_marker = crate::charset::skiptowhite_slice(marker_start);
        let marker_len = marker_start.len() - after_marker.len();
        let after_ws = crate::charset::skipwhite_slice(after_marker);
        if let Some(&after_b) = after_ws.first()
            && after_b != comment_char
        {
            let p_off = cmd_line_len - after_marker.len();
            let trailing_cstr = bytes_to_cstring(&full_cmd_bytes[p_off..cmd_line_len]);
            semsg!(gettext(E_TRAILING_ARG), &trailing_cstr);
            return None;
        }
        if !script_get && marker_start[0].is_ascii_lowercase() {
            emsg(gettext(c"E221: Marker cannot start with lower case letter"));
            return None;
        }
        bytes_to_cstring(&marker_start[..marker_len])
    } else if script_get {
        bytes_to_cstring(b".")
    } else {
        emsg(gettext(c"E172: Missing marker"));
        return None;
    };

    let mut theline_owned: Option<std::ffi::CString> = None;
    let l = tv_list_alloc(0);
    loop {
        let mut mi: i32 = 0;
        let mut ti: i32 = 0;

        let theline: &std::ffi::CStr = if heredoc_in_string {
            let cur_line_arg = line_arg_cstr.unwrap_or(c"");
            let rem_bytes = cur_line_arg.to_bytes();
            if rem_bytes.is_empty() {
                if !script_get {
                    semsg!(gettext(E_MISSING_END_MARKER_STR), &marker_cstr);
                }
                break;
            }
            let line_len = rem_bytes
                .iter()
                .position(|&b| b == b'\n')
                .unwrap_or(rem_bytes.len());
            theline_owned = Some(bytes_to_cstring(&rem_bytes[..line_len]));
            let step = if line_len < rem_bytes.len() {
                line_len + 1
            } else {
                line_len
            };
            line_arg_cstr = Some(&cur_line_arg[step..]);
            theline_owned.as_deref().unwrap()
        } else {
            theline_owned = eap.ea_getline.call(0, 0, false);
            let Some(ref tl) = theline_owned else {
                if !script_get {
                    semsg!(gettext(E_MISSING_END_MARKER_STR), &marker_cstr);
                }
                break;
            };
            tl.as_c_str()
        };

        let line_bytes = theline.to_bytes();
        if marker_indent_len > 0 {
            let len = marker_indent_len as usize;
            let cmd_b = eap.cmdline_cstr().to_bytes();
            if line_bytes[..len.min(line_bytes.len())] == cmd_b[..len.min(cmd_b.len())] {
                mi = marker_indent_len;
            }
        }
        if marker_cstr.as_bytes() == &line_bytes[(mi as usize).min(line_bytes.len())..] {
            break;
        }

        if eval_failed {
            continue;
        }

        if text_indent_len == -1 && !line_bytes.is_empty() {
            text_indent_len = line_bytes
                .iter()
                .take_while(|&&b| ascii_iswhite(b as i32))
                .count() as i32;
            text_indent = Some(line_bytes[..text_indent_len as usize].to_vec());
        }
        if let Some(ref indent_slice) = text_indent {
            while ti < text_indent_len {
                if line_bytes.get(ti as usize).copied() != Some(indent_slice[ti as usize]) {
                    break;
                }
                ti += 1;
            }
        }

        let str_line = &theline[(ti as usize).min(line_bytes.len())..];
        if evalstr && !eap.skip {
            let Some(evaluated) = eval_all_expr_in_str(str_line) else {
                eval_failed = true;
                continue;
            };
            tv_list_append_string(&l, Some(evaluated.as_c_str()), -1);
        } else {
            tv_list_append_string(&l, Some(str_line), -1);
        }
    }
    if heredoc_in_string {
        eap.nextcmd = crate::strings::OwnedSlice::from_opt_cstr(line_arg_cstr);
    }
    drop(theline_owned);
    drop(text_indent);

    if eval_failed {
        tv_list_free(Some(&l));
        return None;
    }
    Some(l)
}

/// ":let" and ":const" commands.
pub fn ex_let(eap: &mut Exarg) {
    let is_const = eap.cmdidx == Cmdidx::CmdConst;
    let arg_slice = eap.arg.clone();
    let mut arg = arg_slice.as_cstr();
    let mut rettv: TypVal = TypVal::new();
    let mut var_count: i32 = 0;
    let mut semicolon: i32 = 0;
    let mut op: [u8; 2] = [0; 2];
    let mut first: i32 = 1;

    let Some(argend_cstr) = skip_var_list(arg, &mut var_count, &mut semicolon, false) else {
        return;
    };
    let mut expr = crate::charset::skipwhite(argend_cstr);
    let expr_bytes = expr.to_bytes();
    let concat = expr_bytes.starts_with(b"..=");
    let has_assign = expr_bytes.first() == Some(&b'=')
        || (expr_bytes
            .first()
            .is_some_and(|&b| matches!(b, b'+' | b'-' | b'*' | b'/' | b'%' | b'.'))
            && expr_bytes.get(1) == Some(&b'='));
    if !has_assign && !concat {
        let arg_b0 = arg.to_bytes().first().copied().unwrap_or(0);
        if arg_b0 == b'[' {
            emsg(gettext(E_INVARG));
        } else if ends_excmd(i32::from(arg_b0)) == 0 {
            arg = list_arg_vars(eap, arg, &mut first);
        } else if !eap.skip {
            list_glob_vars(&mut first);
            list_buf_vars(&mut first);
            list_win_vars(&mut first);
            list_tab_vars(&mut first);
            list_script_vars(&mut first);
            list_func_vars(&mut first);
            list_vim_vars(&mut first);
        }
        eap.nextcmd = arg_slice.advance_to_opt_cstr(check_nextcmd(arg));
        return;
    }

    if expr_bytes.starts_with(b"=<<") {
        if let Some(l) = heredoc_get(eap, &expr[3..], false) {
            tv_list_set_ret(&mut rettv, Some(&l));
            if !eap.skip {
                op[0] = b'=';
                op[1] = 0;
                ex_let_vars(
                    eap.arg.as_cstr(),
                    &mut rettv,
                    0,
                    semicolon,
                    var_count,
                    i32::from(is_const),
                    Some(crate::memory::bytes_as_cstr(&op)),
                );
            }
            tv_clear(&mut rettv);
        }
        return;
    }

    rettv.v_type = VarType::VarUnknown;

    op[0] = b'=';
    op[1] = 0;
    if expr_bytes.first() != Some(&b'=') {
        let mut step = 2usize;
        if let Some(&b0) = expr_bytes.first()
            && matches!(b0, b'+' | b'-' | b'*' | b'/' | b'%' | b'.')
        {
            op[0] = b0;
            if expr_bytes.starts_with(b"..") {
                step += 1;
            }
        }
        expr = &expr[step..];
    } else {
        expr = &expr[1..];
    }

    expr = crate::charset::skipwhite(expr);

    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() + (1));
    }
    let mut evalarg = Evalarg::new();
    fill_evalarg_from_eap(&mut evalarg, eap.skip);
    let eval_res = eval0(expr, &mut rettv, Some(&mut *eap), &mut evalarg);
    if eap.skip {
        g_st().emsg_skip.set(g_st().emsg_skip.get() - (1));
    }

    if !eap.skip && eval_res {
        ex_let_vars(
            eap.arg.as_cstr(),
            &mut rettv,
            0,
            semicolon,
            var_count,
            i32::from(is_const),
            Some(crate::memory::bytes_as_cstr(&op)),
        );
    }
    if eval_res {
        tv_clear(&mut rettv);
    }
}

/// Assign the typevalue "tv" to the variable or variables at "arg_start".
pub fn ex_let_vars(
    arg_start: &CStr,
    tv: &mut TypVal,
    copy: i32,
    semicolon: i32,
    var_count: i32,
    is_const: i32,
    op: Option<&CStr>,
) -> bool {
    let mut arg = arg_start;
    let mut ltv: TypVal = TypVal::new();

    if arg.to_bytes().first() != Some(&b'[') {
        return ex_let_one(arg, &mut *tv, copy != 0, is_const != 0, op, op).is_some();
    }

    if tv.v_type != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return false;
    }
    let l = tv.vval.v_list_opt();

    let len = tv_list_len(l);
    if semicolon == 0 && var_count < len {
        emsg(gettext(c"E687: Less targets than List items"));
        return false;
    }
    if var_count - semicolon > len {
        emsg(gettext(c"E688: More targets than List items"));
        return false;
    }
    let l = l.unwrap();

    let mut item = l.first_opt();
    let mut rest_len = tv_list_len(Some(l)) as usize;
    while arg.to_bytes().first() != Some(&b']') {
        let cur = item.unwrap();
        let next_arg = crate::charset::skipwhite_at(arg, 1);
        let mut cur_tv = cur.tv().clone();
        let Some(after_one) =
            ex_let_one(next_arg, &mut cur_tv, true, is_const != 0, Some(c",;]"), op)
        else {
            return false;
        };
        rest_len -= 1;

        item = cur.next_opt();
        arg = crate::charset::skipwhite(after_one);
        if arg.to_bytes().first() == Some(&b';') {
            let rest_list = tv_list_alloc(rest_len as isize);
            while let Some(rem) = item {
                tv_list_append_tv(&rest_list, &rem.tv());
                item = rem.next_opt();
            }

            tv_list_ref(Some(&rest_list));
            ltv.v_type = VarType::VarList;
            ltv.v_lock = VAR_UNLOCKED;
            ltv.vval = crate::eval::typval::TypvalVval::List(rest_list);

            let semi_arg = crate::charset::skipwhite_at(arg, 1);
            let res = ex_let_one(semi_arg, &mut ltv, false, is_const != 0, Some(c"]"), op);
            tv_clear(&mut ltv);
            if res.is_none() {
                return false;
            }
            break;
        } else if !matches!(arg.to_bytes().first(), Some(&b',' | &b']')) {
            internal_error(c"ex_let_vars()");
            return false;
        }
    }

    true
}

/// Skip over assignable variable "var" or list of variables "[var, var]".
pub fn skip_var_list<'a>(
    arg_in: &'a CStr,
    var_count: &mut i32,
    semicolon: &mut i32,
    silent: bool,
) -> Option<&'a CStr> {
    if arg_in.to_bytes().first() == Some(&b'[') {
        let mut p_cstr = arg_in;
        loop {
            p_cstr = crate::charset::skipwhite_at(p_cstr, 1);
            let s_cstr = skip_var_one(p_cstr);
            if s_cstr.as_ptr() == p_cstr.as_ptr() {
                if !silent {
                    semsg!(gettext(E_INVARG2), p_cstr);
                }
                return None;
            }
            *var_count += 1;

            p_cstr = crate::charset::skipwhite(s_cstr);
            let p0 = p_cstr.to_bytes().first().copied().unwrap_or(0);
            if p0 == b']' {
                break;
            } else if p0 == b';' {
                if *semicolon == 1 {
                    if !silent {
                        emsg(gettext(E_DOUBLE_SEMICOLON_IN_LIST_OF_VARIABLES));
                    }
                    return None;
                }
                *semicolon = 1;
            } else if p0 != b',' {
                if !silent {
                    semsg!(gettext(E_INVARG2), p_cstr);
                }
                return None;
            }
        }
        return Some(&p_cstr[1..]);
    }
    Some(skip_var_one(arg_in))
}

/// Skip one (assignable) variable name, including @r, $VAR, &option, d.key, l[idx].
fn skip_var_one(arg_in: &CStr) -> &CStr {
    let arg_bytes = arg_in.to_bytes();
    if arg_bytes.first() == Some(&b'@') && arg_bytes.len() > 1 {
        return &arg_in[2..];
    }
    find_name_end(
        if matches!(arg_bytes.first(), Some(&b'$' | &b'&')) {
            &arg_in[1..]
        } else {
            arg_in
        },
        None,
        FNE_INCL_BR | FNE_CHECK_START,
    )
}

/// List variables for hashtab "ht" with prefix "prefix".
pub fn list_hashtable_vars(ht: &Hashtab, prefix: &std::ffi::CStr, empty: i32, first: &mut i32) {
    for (_, di) in ht.iter_items_mut() {
        if g_st().got_int.get() {
            break;
        }
        let mut buf = [0u8; IOSIZE];

        crate::memory::copy_bytes_to_c_buf(&mut buf, prefix.to_bytes());
        crate::memory::append_bytes_to_c_buf(&mut buf, di.key_bytes());
        if message_filtered(crate::memory::bytes_as_cstr(&buf)) {
            continue;
        }

        if empty != 0 || di.tv().v_type != VarType::VarString || !di.tv().vval.v_string_is_null() {
            list_one_var(&di, prefix, first);
        }
    }
}

fn list_glob_vars(first: &mut i32) {
    list_hashtable_vars(
        &get_globvar_dict().dv_hashtab,
        c"",
        true as i32,
        &mut *first,
    );
}

fn list_buf_vars(first: &mut i32) {
    list_hashtable_vars(
        &curbuf_mut().b_vars().dv_hashtab,
        c"b:",
        true as i32,
        &mut *first,
    );
}

fn list_win_vars(first: &mut i32) {
    list_hashtable_vars(
        &curwin_mut().w_vars().dv_hashtab,
        c"w:",
        true as i32,
        &mut *first,
    );
}

fn list_tab_vars(first: &mut i32) {
    list_hashtable_vars(
        &curtab_mut().tp_vars().dv_hashtab,
        c"t:",
        true as i32,
        &mut *first,
    );
}

/// List variables in "arg".
fn list_arg_vars<'a>(eap: &mut Exarg, arg_in: &'a CStr, first: &mut i32) -> &'a CStr {
    let mut arg = arg_in;
    let mut error = false;
    let mut tv: TypVal = TypVal::new();

    while ends_excmd(i32::from(arg.to_bytes().first().copied().unwrap_or(0))) == 0
        && !g_st().got_int.get()
    {
        if error || eap.skip {
            arg = find_name_end(arg, None, FNE_INCL_BR | FNE_CHECK_START);
            let b0 = arg.to_bytes().first().copied().unwrap_or(0);
            if !ascii_iswhite(b0) && ends_excmd(i32::from(b0)) == 0 {
                g_st().emsg_severe.set(true);
                semsg!(gettext(E_TRAILING_ARG), arg);
                break;
            }
        } else {
            let name_start = arg;
            let mut name: &CStr = arg;
            let mut tofree: Option<std::ffi::CString> = None;
            let len = get_name_len(&mut arg, &mut tofree, true, true);
            if len <= 0 {
                if len < 0 && !aborting() {
                    g_st().emsg_severe.set(true);
                    semsg!(gettext(E_INVARG2), arg);
                    break;
                }
                error = true;
            } else {
                if let Some(ref tf) = tofree {
                    name = tf.as_c_str();
                }
                if !eval_variable(name, len, Some(&mut tv), None, true, false) {
                    error = true;
                } else {
                    let arg_subsc = arg;
                    let subsc_ok = handle_subscript(
                        &mut arg,
                        name_start,
                        &mut tv,
                        Some(&mut Evalarg { eval_flags: 1 }),
                        true,
                    );
                    if !subsc_ok {
                        error = true;
                    } else {
                        if arg.as_ptr() == arg_subsc.as_ptr()
                            && len == 2
                            && name.to_bytes().get(1) == Some(&b':')
                        {
                            match name.to_bytes().first().copied().unwrap_or(0) {
                                b'g' => list_glob_vars(first),
                                b'b' => list_buf_vars(first),
                                b'w' => list_win_vars(first),
                                b't' => list_tab_vars(first),
                                b'v' => list_vim_vars(first),
                                b's' => list_script_vars(first),
                                b'l' => list_func_vars(first),
                                _ => {
                                    semsg!(gettext(c"E738: Can't list variables for %s"), name);
                                }
                            }
                        } else {
                            let s = encode_tv2echo(&mut tv, None);
                            let used_name = if arg.as_ptr() == arg_subsc.as_ptr() {
                                name
                            } else {
                                name_start
                            };
                            let name_size = if tofree
                                .as_ref()
                                .is_some_and(|tf| ptr::eq(used_name.as_ptr(), tf.as_ptr()))
                            {
                                used_name.to_bytes().len() as isize
                            } else {
                                (arg.as_ptr() as isize) - (used_name.as_ptr() as isize)
                            };
                            list_one_var_a(c"", Some(used_name), name_size, tv.v_type, &s, first);
                        }
                        tv_clear(&mut tv);
                    }
                }
            }

            drop(tofree);
        }

        arg = crate::charset::skipwhite(arg);
    }

    arg
}

/// Set an environment variable, part of ex_let_one().
fn ex_let_env<'a>(
    arg_in: &'a CStr,
    tv: &mut TypVal,
    is_const: bool,
    endchars: Option<&CStr>,
    op: Option<&CStr>,
) -> Option<&'a CStr> {
    if is_const {
        emsg(gettext(c"E996: Cannot lock an environment variable"));
        return None;
    }

    let mut arg = &arg_in[1..];
    let name = arg;
    let mut arg_end: Option<&'a CStr> = None;
    let len = get_env_len(&mut arg) as usize;
    let op_b = op.and_then(|s| s.to_bytes().first().copied()).unwrap_or(0);
    if len == 0 {
        semsg!(gettext(E_INVARG2), arg_in);
    } else if op_b != 0 && crate::strings::vim_haschr(c"+-*/%", i32::from(op_b)) {
        semsg!(gettext(E_LETWRONG), op.unwrap());
    } else if endchars.is_some_and(|ec| {
        let next_b = crate::charset::skipwhite(arg)
            .to_bytes()
            .first()
            .copied()
            .unwrap_or(0);
        !crate::strings::vim_haschr(ec, i32::from(next_b))
    }) {
        emsg(gettext(E_LETUNEXP));
    } else if !check_secure() {
        let mut tofree: Option<std::ffi::CString> = None;
        let name_cstr = bytes_to_cstring(&name.to_bytes()[..len]);
        let mut p_opt = tv_get_string_chk(&*tv);
        if let Some(ref p_cstr) = p_opt
            && op_b == b'.'
            && let Some(s) = vim_getenv(&name_cstr)
        {
            let mut bytes = Vec::from(s.as_bytes());
            bytes.extend_from_slice(p_cstr.to_bytes());
            let joined = crate::memory::bytes_to_cstring(&bytes);
            tofree = Some(joined);
            p_opt = tofree.as_deref().map(std::borrow::Cow::Borrowed);
        }
        if let Some(ref p_cstr) = p_opt {
            vim_setenv_ext(&name_cstr, p_cstr);
            arg_end = Some(arg);
        }
        drop(tofree);
    }
    arg_end
}

/// Set an option, part of ex_let_one().
fn ex_let_option<'a>(
    arg_in: &'a CStr,
    tv: &mut TypVal,
    is_const: bool,
    endchars: Option<&CStr>,
    op: Option<&CStr>,
) -> Option<&'a CStr> {
    if is_const {
        emsg(gettext(c"E996: Cannot lock an option"));
        return None;
    }

    let mut opt_idx: OptIndex = 0;
    let mut opt_flags: i32 = 0;

    let mut arg_cstr = arg_in;
    let p_opt = find_option_var_end(&mut arg_cstr, &mut opt_idx, &mut opt_flags);

    let Some(p_cstr) = p_opt else {
        emsg(gettext(E_LETUNEXP));
        return None;
    };
    if endchars.is_some_and(|ec| {
        let next_b = crate::charset::skipwhite(p_cstr)
            .to_bytes()
            .first()
            .copied()
            .unwrap_or(0);
        !crate::strings::vim_haschr(ec, i32::from(next_b))
    }) {
        emsg(gettext(E_LETUNEXP));
        return None;
    }

    let opt_len = (p_cstr.as_ptr() as usize).wrapping_sub(arg_cstr.as_ptr() as usize);
    let opt_bytes = &arg_cstr.to_bytes()[..opt_len];
    let opt_cstr = bytes_to_cstring(opt_bytes);

    let is_tty_opt = is_tty_option(opt_bytes);
    let hidden = is_option_hidden(opt_idx);
    let curval = if is_tty_opt {
        get_tty_option(opt_bytes)
    } else {
        get_option_value(opt_idx, opt_flags)
    };
    let mut newval = nil_obj();

    if curval.obj_type == ObjectType::ObjectTypeNil {
        semsg!(gettext(E_UNKNOWN_OPTION2), &opt_cstr);
        optval_free(curval);
        optval_free(newval);
        return None;
    }
    let op_b = op.and_then(|s| s.to_bytes().first().copied()).unwrap_or(0);
    if op_b != 0
        && op_b != b'='
        && ((curval.obj_type != ObjectType::ObjectTypeString && op_b == b'.')
            || (curval.obj_type == ObjectType::ObjectTypeString && op_b != b'.'))
    {
        semsg!(gettext(E_LETWRONG), op.unwrap());
        optval_free(curval);
        optval_free(newval);
        return None;
    }

    let mut error = false;
    newval = opt_from_tv(tv, opt_idx, &opt_cstr, &mut error);
    if error {
        optval_free(curval);
        optval_free(newval);
        return None;
    }

    let is_num = (curval.obj_type == ObjectType::ObjectTypeInteger
        || curval.obj_type == ObjectType::ObjectTypeBoolean)
        && (newval.obj_type == ObjectType::ObjectTypeInteger
            || newval.obj_type == ObjectType::ObjectTypeBoolean);
    let is_string = curval.obj_type == ObjectType::ObjectTypeString
        && newval.obj_type == ObjectType::ObjectTypeString;

    if op_b != 0 && op_b != b'=' {
        if !hidden && is_num {
            let cur_n: OptInt = if curval.obj_type == ObjectType::ObjectTypeInteger {
                curval.data.integer()
            } else {
                OptInt::from(curval.data.boolean())
            };
            let mut new_n: OptInt = if newval.obj_type == ObjectType::ObjectTypeInteger {
                newval.data.integer()
            } else {
                OptInt::from(newval.data.boolean())
            };

            match op_b {
                b'+' => new_n = cur_n.wrapping_add(new_n),
                b'-' => new_n = cur_n.wrapping_sub(new_n),
                b'*' => new_n = cur_n.wrapping_mul(new_n),
                b'/' => new_n = num_divide(cur_n, new_n),
                b'%' => new_n = num_modulus(cur_n, new_n),
                _ => {}
            }

            if curval.obj_type == ObjectType::ObjectTypeInteger {
                newval = integer_obj(new_n);
            } else {
                newval = opt_from_tristate(tristate_from_int(new_n));
            }
        } else if !hidden && is_string {
            let curval_str = curval.data.string();
            let newval_str = newval.data.string();

            if !curval_str.data.is_null() && !newval_str.data.is_null() {
                let newval_old = newval;
                let joined = concat_str(curval_str.cstr_bytes(), newval_str.cstr_bytes());
                newval = crate::api::private::helpers::cstr_to_obj(Some(&*joined));
                optval_free(newval_old);
            }
        }
    }

    if !is_tty_opt {
        set_option_value_give_err(opt_idx, newval.clone(), opt_flags);
    }

    optval_free(curval);
    optval_free(newval);
    Some(p_cstr)
}

/// Set a register, part of ex_let_one().
fn ex_let_register<'a>(
    arg_in: &'a CStr,
    tv: &mut TypVal,
    is_const: bool,
    endchars: Option<&CStr>,
    op: Option<&CStr>,
) -> Option<&'a CStr> {
    if is_const {
        emsg(gettext(c"E996: Cannot lock a register"));
        return None;
    }

    let arg = &arg_in[1..];
    let mut arg_end: Option<&'a CStr> = None;
    let op_b = op.and_then(|s| s.to_bytes().first().copied()).unwrap_or(0);
    let arg_b0 = arg.to_bytes().first().copied().unwrap_or(0);
    let after_reg = if arg.is_empty() { c"" } else { &arg[1..] };
    if op_b != 0 && crate::strings::vim_haschr(c"+-*/%", i32::from(op_b)) {
        semsg!(gettext(E_LETWRONG), op.unwrap());
    } else if endchars.is_some_and(|ec| {
        let next_b = crate::charset::skipwhite(after_reg)
            .to_bytes()
            .first()
            .copied()
            .unwrap_or(0);
        !crate::strings::vim_haschr(ec, i32::from(next_b))
    }) {
        emsg(gettext(E_LETUNEXP));
    } else {
        let mut ptofree: Option<std::ffi::CString> = None;
        let mut p_opt = tv_get_string_chk(&*tv);
        if let Some(ref p_cstr) = p_opt
            && op_b == b'.'
        {
            let reg_c = if arg_b0 == b'@' {
                b'"' as i32
            } else {
                i32::from(arg_b0)
            };
            if let Some(s) = get_reg_contents_str(reg_c, K_GREG_EXPR_SRC) {
                let mut bytes = Vec::from(s.as_bytes());
                bytes.extend_from_slice(p_cstr.to_bytes());
                let joined = bytes_to_cstring(&bytes);
                ptofree = Some(joined);
                p_opt = ptofree.as_deref().map(std::borrow::Cow::Borrowed);
            }
        }
        if let Some(ref p_cstr) = p_opt {
            let reg_c = if arg_b0 == b'@' {
                b'"' as i32
            } else {
                i32::from(arg_b0)
            };
            let p_bytes = p_cstr.to_bytes();
            write_reg_contents(reg_c, p_bytes, p_bytes.len() as isize, false);
            arg_end = Some(after_reg);
        }
        drop(ptofree);
    }
    arg_end
}

/// Set one item of `:let var = expr` or `:let [v1, v2] = list` to its value.
fn ex_let_one<'a>(
    arg_in: &'a CStr,
    tv: &mut TypVal,
    copy: bool,
    is_const: bool,
    endchars: Option<&CStr>,
    op: Option<&CStr>,
) -> Option<&'a CStr> {
    let mut arg_end: Option<&'a CStr> = None;

    match arg_in.to_bytes().first().copied().unwrap_or(0) {
        b'$' => return ex_let_env(arg_in, tv, is_const, endchars, op),
        b'&' => return ex_let_option(arg_in, tv, is_const, endchars, op),
        b'@' => return ex_let_register(arg_in, tv, is_const, endchars, op),
        b0 if eval_isnamec1(i32::from(b0)) || b0 == b'{' => {
            let mut lv: Lval = Lval::new();
            let p_opt = get_lval(arg_in, Some(tv), &mut lv, false, false, 0, FNE_CHECK_START);
            if let Some(p_cstr) = p_opt
                && lv.ll_name.is_some()
            {
                if endchars.is_some_and(|ec| {
                    let next_b = crate::charset::skipwhite(p_cstr)
                        .to_bytes()
                        .first()
                        .copied()
                        .unwrap_or(0);
                    !crate::strings::vim_haschr(ec, i32::from(next_b))
                }) {
                    emsg(gettext(E_LETUNEXP));
                } else {
                    set_var_lval(&mut lv, p_cstr, tv, copy, is_const, op);
                    arg_end = Some(p_cstr);
                }
            }
        }
        _ => {
            semsg!(gettext(E_INVARG2), arg_in);
        }
    }

    arg_end
}

/// ":unlet[!] var1 ... " command.
pub fn ex_unlet(eap: &mut Exarg) {
    let arg = eap.arg.to_cstring();
    let forceit = eap.forceit;
    ex_unletlock(
        eap,
        &arg,
        0,
        if forceit { GLV_QUIET } else { 0 },
        do_unlet_var,
    );
}

/// ":lockvar" and ":unlockvar" commands.
pub fn ex_lockvar(eap: &mut Exarg) {
    let mut arg = eap.arg.clone();
    let mut deep: i32 = 2;

    if eap.forceit {
        deep = -1;
    } else if ascii_isdigit((arg).first_byte()) {
        let step = {
            let arg_bytes = arg.as_bytes();
            let mut rem = arg_bytes;
            deep = getdigits_int(&mut rem, false, -1);
            let rem = crate::charset::skipwhite_slice(rem);
            arg_bytes.len() - rem.len()
        };
        arg = arg.wrapping_add(step);
    }

    ex_unletlock(eap, (arg).as_cstr(), deep, 0, do_lock_var);
}

/// Common parsing logic for :unlet, :lockvar and :unlockvar.
fn ex_unletlock(
    eap: &mut Exarg,
    argstart: &CStr,
    deep: i32,
    glv_flags: i32,
    callback: ExUnletlockCallback,
) {
    let mut arg = argstart;
    let mut name_end: Option<&CStr>;
    let mut error = false;
    let mut lv: Lval = Lval::new();

    loop {
        if arg.to_bytes().first() == Some(&b'$') {
            let name_start = arg;
            lv.set_ll_name(arg);
            lv.ll_tv = LvalTv::None;
            let mut arg_cstr = &arg[1..];
            if get_env_len(&mut arg_cstr) == 0 {
                semsg!(gettext(E_INVARG2), name_start);
                return;
            }
            arg = arg_cstr;
            lv.ll_name_len = (arg.as_ptr() as usize).wrapping_sub(name_start.as_ptr() as usize);
            if !error && !eap.skip && !callback(&mut lv, eap, deep) {
                error = true;
            }
            name_end = Some(arg);
        } else {
            name_end = get_lval(
                arg,
                None,
                &mut lv,
                true,
                eap.skip || error,
                glv_flags,
                FNE_CHECK_START,
            );
            if lv.ll_name.is_none() {
                error = true;
            }
            let Some(ne) = name_end else {
                break;
            };
            let ne_b0 = ne.to_bytes().first().copied().unwrap_or(0);
            if !ascii_iswhite(ne_b0) && ends_excmd(i32::from(ne_b0)) == 0 {
                g_st().emsg_severe.set(true);
                semsg!(gettext(E_TRAILING_ARG), ne);
                break;
            }

            if !error && !eap.skip && !callback(&mut lv, eap, deep) {
                error = true;
            }
        }
        arg = crate::charset::skipwhite(name_end.unwrap_or(c""));
        if ends_excmd(i32::from(arg.to_bytes().first().copied().unwrap_or(0))) != 0 {
            break;
        }
    }

    eap.nextcmd = crate::strings::OwnedSlice::from_opt_cstr(check_nextcmd(arg));
}

/// Unlet a variable indicated by `lp`.
fn do_unlet_var(lp: &mut Lval, eap: &mut Exarg, _deep: i32) -> bool {
    let forceit = eap.forceit;
    let mut ret = true;
    let ll_name = lp.ll_name_cstr().unwrap_or(c"");

    if lp.ll_tv.is_null() {
        let ll_bytes = ll_name.to_bytes();
        let name_slice = &ll_bytes[..lp.ll_name_len.min(ll_bytes.len())];
        if name_slice.first() == Some(&b'$') {
            let env_cstr = bytes_to_cstring(&name_slice[1..]);
            vim_unsetenv_ext(env_cstr.as_c_str());
        } else if !do_unlet(name_slice, forceit) {
            ret = false;
        }
    } else if (!lp.ll_list.is_null()
        && value_check_lock(
            tv_list_locked(lp.list_opt()),
            lp.ll_name_cstr(),
            lp.ll_name_len,
        ))
        || (lp
            .dict_opt()
            .is_some_and(|d| value_check_lock(d.dv_lock.get(), lp.ll_name_cstr(), lp.ll_name_len)))
    {
        return false;
    } else if lp.ll_range {
        let li_first = lp.ll_li.clone();
        let n1 = lp.ll_n1;
        let has_n2 = !lp.ll_empty2;
        let n2 = lp.ll_n2;
        tv_list_unlet_range(lp.list_opt(), &li_first, n1, has_n2, n2);
    } else if let Some(l) = lp.list_opt() {
        tv_list_item_remove(l, &lp.ll_li);
    } else {
        let d = lp.dict_opt().unwrap();
        let di = lp.di_mut();
        let watched = tv_dict_is_watched(Some(d));
        let mut key: Option<std::ffi::CString> = None;
        let mut oldtv: TypVal = TypVal::new();

        if watched {
            tv_copy(&di.tv(), &mut oldtv);
            key = Some(di.key_cstr().to_owned());
        }

        tv_dict_item_remove(d, di);

        if let Some(ref k) = key {
            tv_dict_watcher_notify(d, k.as_c_str(), None, Some(&oldtv));
            tv_clear(&mut oldtv);
        }
    }

    ret
}

/// Unlet one item or a range of items from a list.
fn tv_list_unlet_range(
    l: Option<&TvList>,
    li_first: &Listitem,
    n1_arg: i32,
    has_n2: bool,
    n2: i32,
) {
    debug_assert!(l.is_some());
    let mut li_last = crate::eval::typval::ListitemId::from_ref(li_first);
    let mut n1 = n1_arg;
    loop {
        let li = li_last.li_next.get();
        n1 += 1;
        if li.is_null() || (has_n2 && n2 < n1) {
            break;
        }
        li_last = li;
    }
    tv_list_remove_items(l.unwrap(), li_first, &li_last);
}

/// unlet a variable.
pub fn do_unlet(name_slice: &[u8], forceit: bool) -> bool {
    let name_cstr = bytes_to_cstring(name_slice);
    let name_len = name_slice.len();
    let mut varname_opt: Option<&CStr> = None;
    let mut dict_opt: Option<crate::eval::typval::DictId> = None;
    let ht_dict_opt = find_var_ht_dict(&name_cstr, name_len, &mut varname_opt, &mut dict_opt);

    if let Some(mut ht_dict) = ht_dict_opt
        && let Some(varname_cstr) = varname_opt.filter(|s| !s.to_bytes().is_empty())
    {
        let mut d = get_current_funccal_dict(&ht_dict.dv_hashtab)
            .unwrap_or(crate::eval::typval::DictId::NULL);
        if d.is_null() {
            if ht_dict == get_globvar_dict() {
                d = get_globvar_dict();
            } else if ht_dict == get_compat_dict() {
                d = get_vimvar_dict();
            } else if let Some(di) = find_var_in_ht(
                &ht_dict.dv_hashtab,
                name_slice.first().copied().unwrap_or(0) as i32,
                c"",
                0,
                0,
            ) {
                d = di.tv().vval.v_dict();
            }
            if d.is_null() {
                internal_error(c"do_unlet()");
                return false;
            }
        }

        let mut hi_idx = Some(hash_find_idx(&ht_dict.dv_hashtab, varname_cstr));
        if hashitem_empty(&ht_dict.dv_hashtab.slot(hi_idx.unwrap())) {
            let mut ht_dict_scoped: Option<crate::eval::typval::DictId> = Some(ht_dict);
            hi_idx = find_hi_in_scoped_ht(&name_cstr, &mut ht_dict_scoped);
            if let Some(h) = ht_dict_scoped {
                ht_dict = h;
            } else {
                hi_idx = None;
                ht_dict = get_globvar_dict();
            }
        }
        if let Some(idx) = hi_idx
            && !hashitem_empty(&ht_dict.dv_hashtab.slot(idx))
        {
            let hi = ht_dict.dv_hashtab.slot(idx);
            let di = tv_dict_hi2di(&hi);
            let d_lock = d.dv_lock.get();
            if var_check_fixed(i32::from(di.di_flags.get()), &name_cstr, TV_CSTRING)
                || var_check_ro(i32::from(di.di_flags.get()), &name_cstr, TV_CSTRING)
                || value_check_lock(d_lock, Some(&name_cstr), TV_CSTRING)
            {
                return false;
            }

            if value_check_lock(d_lock, Some(&name_cstr), TV_CSTRING) {
                return false;
            }

            let mut oldtv: TypVal = TypVal::new();
            let watched = tv_dict_is_watched(dict_opt.as_deref());

            if watched {
                tv_copy(&di.tv(), &mut oldtv);
            }

            delete_var(&ht_dict.dv_hashtab, idx);

            if watched {
                tv_dict_watcher_notify(
                    dict_opt.as_deref().unwrap(),
                    varname_cstr,
                    None,
                    Some(&oldtv),
                );
                tv_clear(&mut oldtv);
            }
            return true;
        }
    }
    if forceit {
        return true;
    }
    semsg!(gettext(c"E108: No such variable: \"%s\""), &name_cstr);
    false
}

/// Lock or unlock variable indicated by `lp`.
fn do_lock_var(lp: &mut Lval, eap: &mut Exarg, deep: i32) -> bool {
    let lock = eap.cmdidx == Cmdidx::CmdLockvar;
    let mut ret = true;
    let ll_name = lp.ll_name_cstr().unwrap_or(c"");

    if lp.ll_tv.is_null() {
        if ll_name.to_bytes().first() == Some(&b'$') {
            semsg!(gettext(E_LOCK_UNLOCK), ll_name);
            ret = false;
        } else {
            let Some(di) = find_var_opt(ll_name, lp.ll_name_len, false, 1) else {
                return false;
            };
            if (di.di_flags.get() & DI_FLAGS_FIX) != 0
                && di.tv().v_type != VarType::VarDict
                && di.tv().v_type != VarType::VarList
            {
                semsg!(gettext(E_LOCK_UNLOCK), ll_name);
                ret = false;
            } else {
                if lock {
                    di.di_flags.set(di.di_flags.get() | DI_FLAGS_LOCK);
                } else {
                    di.di_flags.set(di.di_flags.get() & !DI_FLAGS_LOCK);
                }
                if deep != 0 {
                    let mut item_tv = di.tv().clone();
                    tv_item_lock(&mut item_tv, deep, lock, false);
                    di.tv_mut().v_lock = item_tv.v_lock;
                }
            }
        }
    } else if deep == 0 {
        // nothing to do
    } else if lp.ll_range {
        for li in crate::eval::typval::list_iter_ptr(lp.ll_li.clone()) {
            if !lp.ll_empty2 && lp.ll_n2 < lp.ll_n1 {
                break;
            }
            let mut item_tv = li.tv().clone();
            tv_item_lock(&mut item_tv, deep, lock, false);
            li.tv_mut().v_lock = item_tv.v_lock;
            lp.ll_n1 += 1;
        }
    } else if !lp.ll_list.is_null() {
        let mut item_tv = lp.li_mut().tv().clone();
        tv_item_lock(&mut item_tv, deep, lock, false);
        lp.li_mut().tv_mut().v_lock = item_tv.v_lock;
    } else {
        let mut item_tv = lp.di_mut().tv().clone();
        tv_item_lock(&mut item_tv, deep, lock, false);
        lp.di_mut().tv_mut().v_lock = item_tv.v_lock;
    }

    ret
}

/// Delete all "menutrans_" variables.
pub fn del_menutrans_vars() {
    let gd = get_globvar_dict();
    let ght = &gd.dv_hashtab;
    hash_lock(ght);
    for i in 0..ght.array_len() {
        if ght.ht_used.get() == 0 {
            break;
        }
        let hi = ght.slot(i);
        let is_menutrans = hi
            .key_cstr()
            .is_some_and(|k| k.to_bytes().starts_with(b"menutrans_"));
        if is_menutrans {
            let di = hi.hi_di;
            ght.remove_at(i);
            crate::eval::typval::tv_dict_item_free(&di);
        }
    }
    hash_unlock(ght);
}

pub fn get_globvar_dict() -> crate::eval::typval::DictId {
    let mut id = VARS_STATE.with(|s| s.globvardict.get());
    if id.is_null() {
        let d = crate::eval::typval::tv_dict_alloc_static();
        d.dv_refcount.set(DO_NOT_FREE_CNT);
        id = d;
        VARS_STATE.with(|s| s.globvardict.set(id.clone()));
    }
    id
}

pub fn get_vimvar_dict() -> crate::eval::typval::DictId {
    let mut id = VARS_STATE.with(|s| s.vimvardict.get());
    if id.is_null() {
        let d = crate::eval::typval::tv_dict_alloc_static();
        d.dv_refcount.set(DO_NOT_FREE_CNT);
        id = d;
        VARS_STATE.with(|s| s.vimvardict.set(id.clone()));
    }
    id
}

pub fn set_vim_var_tv(idx: VimVarIndex, tv: &mut TypVal) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    tv_copy(&*tv, &mut di.tv_mut());
}

pub fn get_vim_var_name(idx: VimVarIndex) -> &'static std::ffi::CStr {
    VIMVARS_META[idx as usize].vv_name
}

pub fn get_vim_var_tv(idx: VimVarIndex) -> crate::eval::typval::DictitemId {
    vimvar_di(idx as usize)
}

pub fn get_vim_var_nr(idx: VimVarIndex) -> Varnumber {
    get_vim_var_tv(idx).tv().vval.v_number()
}

pub fn get_vim_var_list(idx: VimVarIndex) -> Option<crate::eval::typval::ListId> {
    let l = get_vim_var_tv(idx).tv().vval.v_list();
    (!l.is_null()).then_some(l)
}

pub fn get_vim_var_dict(idx: VimVarIndex) -> crate::eval::typval::DictId {
    get_vim_var_tv(idx).tv().vval.v_dict()
}

pub fn get_vim_var_str(idx: VimVarIndex) -> std::borrow::Cow<'static, CStr> {
    match tv_get_string(&get_vim_var_tv(idx).tv()) {
        std::borrow::Cow::Borrowed(s) if s.is_empty() => std::borrow::Cow::Borrowed(c""),
        s => std::borrow::Cow::Owned(s.into_owned()),
    }
}

pub fn get_vim_var_partial(idx: VimVarIndex) -> Option<std::rc::Rc<TvPartial>> {
    get_vim_var_tv(idx).tv().vval.v_partial().rc()
}

/// Function to concatenate a prefix and a variable name.
pub fn cat_prefix_varname(prefix: i32, name: Option<&CStr>) -> std::ffi::CString {
    let name_bytes = name.map_or(&[][..], CStr::to_bytes);
    let mut buf = Vec::with_capacity(name_bytes.len() + 2);
    buf.push(prefix as u8);
    buf.push(b':');
    buf.extend_from_slice(name_bytes);
    std::ffi::CString::new(buf).unwrap_or_default()
}

/// Function given to ExpandGeneric() to obtain the list of user defined variable names.
pub fn get_user_var_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if idx == 0 {
        VARS_STATE.with(|s| s.gdone.set(0));
        VARS_STATE.with(|s| s.bdone.set(0));
        VARS_STATE.with(|s| s.wdone.set(0));
        VARS_STATE.with(|s| s.vidx.set(0));
        VARS_STATE.with(|s| s.tdone.set(0));
    }

    let gd = get_globvar_dict();
    let ght = &gd.dv_hashtab;
    if VARS_STATE.with(|s| s.gdone.get()) < ght.ht_used.get() {
        if VARS_STATE.with(|s| s.gdone.get()) == 0 {
            VARS_STATE.with(|s| s.hi_idx.set(0));
        } else {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        VARS_STATE.with(|s| s.gdone.set(s.gdone.get() + (1)));
        while hashitem_empty(&ght.slot(VARS_STATE.with(|s| s.hi_idx.get()))) {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        let hi = ght.slot(VARS_STATE.with(|s| s.hi_idx.get()));
        let hi_key = hi.key_cstr();
        if xp.xp_pattern.as_bytes().starts_with(b"g:") {
            return Some(xp.set_item(cat_prefix_varname(b'g' as i32, hi_key)));
        }
        return hi_key.map(|k| xp.set_item(k.to_owned()));
    }

    let ht = &prevwin_curwin().w_buffer.get().b_vars().dv_hashtab;
    if VARS_STATE.with(|s| s.bdone.get()) < ht.ht_used.get() {
        if VARS_STATE.with(|s| s.bdone.get()) == 0 {
            VARS_STATE.with(|s| s.hi_idx.set(0));
        } else {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        VARS_STATE.with(|s| s.bdone.set(s.bdone.get() + (1)));
        while hashitem_empty(&ht.slot(VARS_STATE.with(|s| s.hi_idx.get()))) {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        let hi = ht.slot(VARS_STATE.with(|s| s.hi_idx.get()));
        return Some(xp.set_item(cat_prefix_varname(b'b' as i32, hi.key_cstr())));
    }

    let ht = &prevwin_curwin().w_vars().dv_hashtab;
    if VARS_STATE.with(|s| s.wdone.get()) < ht.ht_used.get() {
        if VARS_STATE.with(|s| s.wdone.get()) == 0 {
            VARS_STATE.with(|s| s.hi_idx.set(0));
        } else {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        VARS_STATE.with(|s| s.wdone.set(s.wdone.get() + (1)));
        while hashitem_empty(&ht.slot(VARS_STATE.with(|s| s.hi_idx.get()))) {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        let hi = ht.slot(VARS_STATE.with(|s| s.hi_idx.get()));
        return Some(xp.set_item(cat_prefix_varname(b'w' as i32, hi.key_cstr())));
    }

    let ht = &curtab_mut().tp_vars().dv_hashtab;
    if VARS_STATE.with(|s| s.tdone.get()) < ht.ht_used.get() {
        if VARS_STATE.with(|s| s.tdone.get()) == 0 {
            VARS_STATE.with(|s| s.hi_idx.set(0));
        } else {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        VARS_STATE.with(|s| s.tdone.set(s.tdone.get() + (1)));
        while hashitem_empty(&ht.slot(VARS_STATE.with(|s| s.hi_idx.get()))) {
            VARS_STATE.with(|s| s.hi_idx.set(s.hi_idx.get() + (1)));
        }
        let hi = ht.slot(VARS_STATE.with(|s| s.hi_idx.get()));
        return Some(xp.set_item(cat_prefix_varname(b't' as i32, hi.key_cstr())));
    }

    if VARS_STATE.with(|s| s.vidx.get()) < VV_LEN {
        let v = VARS_STATE.with(|s| s.vidx.get()) as VimVarIndex;
        VARS_STATE.with(|s| s.vidx.set(s.vidx.get() + (1)));
        return Some(xp.set_item(cat_prefix_varname(b'v' as i32, Some(get_vim_var_name(v)))));
    }

    None
}

pub fn set_vim_var_type(idx: VimVarIndex, v_type: VarType) {
    get_vim_var_tv(idx).tv_mut().v_type = v_type;
}

pub fn set_vim_var_nr(idx: VimVarIndex, val: Varnumber) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    di.tv_mut().vval = crate::eval::typval::TypvalVval::Number(val);
}

pub fn set_vim_var_bool(idx: VimVarIndex, val: BoolVarValue) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    let mut tv = di.tv_mut();
    tv.v_type = VarType::VarBool;
    tv.vval = crate::eval::typval::TypvalVval::Bool(val);
}

pub fn set_vim_var_special(idx: VimVarIndex, val: SpecialVarValue) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    let mut tv = di.tv_mut();
    tv.v_type = VarType::VarSpecial;
    tv.vval = crate::eval::typval::TypvalVval::Special(val);
}

pub fn set_vim_var_char(c: i32) {
    let mut buf = [0u8; MB_MAXCHAR + 1];
    let buflen = utf_char2bytes(c, &mut buf) as usize;
    set_vim_var_bytes(VV_CHAR, &buf[..buflen]);
}

pub fn set_vim_var_bytes(idx: VimVarIndex, val: &[u8]) {
    let di = get_vim_var_tv(idx);
    crate::eval::typval::tv_set_bytes(&mut di.tv_mut(), val);
}

pub fn set_vim_var_string(idx: VimVarIndex, val: Option<&CStr>, len: isize) {
    let di = get_vim_var_tv(idx);
    let Some(val) = val else {
        di.clear_tv();
        let mut tv = di.tv_mut();
        tv.v_type = VarType::VarString;
        tv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
        return;
    };
    if len == -1 {
        crate::eval::typval::tv_set_string(&mut di.tv_mut(), Some(val));
    } else {
        let bytes = val.to_bytes();
        crate::eval::typval::tv_set_string_len(
            &mut di.tv_mut(),
            Some(&bytes[..(len as usize).min(bytes.len())]),
        );
    }
}

pub fn set_vim_var_list(idx: VimVarIndex, val: Option<&TvList>) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    let list_id = crate::eval::typval::ListId::from(val);
    tv_list_ref(list_id.opt());
    let mut tv = di.tv_mut();
    tv.v_type = VarType::VarList;
    tv.vval = crate::eval::typval::TypvalVval::List(list_id);
}

pub fn set_vim_var_dict(idx: VimVarIndex, val: Option<&TvDict>) {
    let di = get_vim_var_tv(idx);
    di.clear_tv();
    let dict_id = crate::eval::typval::DictId::from(val);
    {
        let mut tv = di.tv_mut();
        tv.v_type = VarType::VarDict;
        tv.vval = crate::eval::typval::TypvalVval::Dict(dict_id.clone());
    }
    let Some(val_ref) = dict_id.opt() else {
        return;
    };

    val_ref.dv_refcount.set(val_ref.dv_refcount.get() + 1);
    tv_dict_set_keys_readonly(val_ref);
}

pub fn set_vim_var_partial(idx: VimVarIndex, val: Option<std::rc::Rc<TvPartial>>) {
    let di = get_vim_var_tv(idx);
    di.tv_mut().vval =
        crate::eval::typval::TypvalVval::Partial(crate::eval::typval::PartialId::from(val));
}

pub fn set_reg_var(c: i32) {
    let mut regname: [u8; 2] = [0, 0];
    if c == 0 || c == b' ' as i32 {
        regname[0] = b'"';
    } else {
        regname[0] = c as u8;
    }
    let di = get_vim_var_tv(VV_REG);
    let need_set = {
        let tv = di.tv();
        tv.vval.v_string_is_null() || i32::from(tv.vval.v_string_first_byte()) != c
    };
    if need_set {
        set_vim_var_bytes(VV_REG, &regname[..1]);
    }
}

pub fn v_exception(oldval: Option<std::ffi::CString>) -> Option<std::ffi::CString> {
    let di = get_vim_var_tv(VV_EXCEPTION);
    let Some(oldval) = oldval else {
        return di.tv().vval.v_string_cstr().map(CStr::to_owned);
    };

    set_vim_var_string(VV_EXCEPTION, Some(&oldval), -1);
    None
}

pub fn set_cmdarg(
    eap: Option<&Exarg>,
    oldarg: Option<std::ffi::CString>,
) -> Option<std::ffi::CString> {
    let di = get_vim_var_tv(VV_CMDARG);
    let oldval = crate::eval::typval::tv_take_string(&mut di.tv_mut());
    if eap.is_none() {
        let mut tv = di.tv_mut();
        tv.v_type = VarType::VarString;
        tv.vval = crate::eval::typval::TypvalVval::from_opt_cstring(oldarg);
        return None;
    }

    let mut newval: Vec<u8> = Vec::new();
    match eap.unwrap().force_bin {
        Some(true) => newval.extend_from_slice(b" ++bin"),
        Some(false) => newval.extend_from_slice(b" ++nobin"),
        None => {}
    }

    if eap.unwrap().read_edit {
        newval.extend_from_slice(b" ++edit");
    }

    if eap.unwrap().force_ff != 0 {
        let ff_str: &[u8] = if eap.unwrap().force_ff == b'u' as i32 {
            b" ++ff=unix"
        } else if eap.unwrap().force_ff == b'd' as i32 {
            b" ++ff=dos"
        } else {
            b" ++ff=mac"
        };
        newval.extend_from_slice(ff_str);
    }
    if eap.unwrap().force_enc != 0 {
        newval.extend_from_slice(b" ++enc=");
        let enc_ptr = eap
            .unwrap()
            .cmd
            .wrapping_add(eap.unwrap().force_enc as usize);
        newval.extend_from_slice((enc_ptr).as_bytes());
    }

    if eap.unwrap().bad_char == BAD_KEEP {
        newval.extend_from_slice(b" ++bad=keep");
    } else if eap.unwrap().bad_char == BAD_DROP {
        newval.extend_from_slice(b" ++bad=drop");
    } else if eap.unwrap().bad_char != 0 {
        newval.extend_from_slice(b" ++bad=");
        newval.push(eap.unwrap().bad_char as u8);
    }

    if eap.unwrap().mkdir_p {
        newval.extend_from_slice(b" ++p");
    }

    crate::eval::typval::tv_set_string_len(&mut di.tv_mut(), Some(&newval));
    oldval
}

pub fn v_throwpoint(oldval: Option<std::ffi::CString>) -> Option<std::ffi::CString> {
    let di = get_vim_var_tv(VV_THROWPOINT);
    let Some(oldval) = oldval else {
        return di.tv().vval.v_string_cstr().map(CStr::to_owned);
    };

    set_vim_var_string(VV_THROWPOINT, Some(&oldval), -1);
    None
}

pub fn set_vcount(count: i64, count1: i64, set_prevcount: bool) {
    if set_prevcount {
        let prev = get_vim_var_nr(VV_COUNT);
        get_vim_var_tv(VV_PREVCOUNT).tv_mut().vval = crate::eval::typval::TypvalVval::Number(prev);
    }
    get_vim_var_tv(VV_COUNT).tv_mut().vval = crate::eval::typval::TypvalVval::Number(count);
    get_vim_var_tv(VV_COUNT1).tv_mut().vval = crate::eval::typval::TypvalVval::Number(count1);
}

/// Get the value of internal variable "name".
pub fn eval_variable(
    name: &CStr,
    len: i32,
    rettv: Option<&mut TypVal>,
    dip: Option<&mut Option<crate::eval::typval::DictitemId>>,
    verbose: bool,
    no_autoload: bool,
) -> bool {
    let mut ret = true;
    let mut tv: Option<TypVal> = None;

    if let Some(v) = find_var(name, len as usize, false, i32::from(no_autoload)) {
        tv = Some(v.tv().clone());
        if let Some(dip) = dip {
            *dip = Some(v);
        }
    }

    if let Some(ref tv) = tv {
        if let Some(rettv) = rettv {
            tv_copy(tv, rettv);
        }
    } else {
        if rettv.is_some() && verbose {
            semsg!(gettext(c"E121: Undefined variable: %.*s"), len, name,);
        }
        ret = false;
    }

    ret
}

/// Check if variable `name_slice` is a local variable or an argument.
pub fn check_vars(name_slice: &[u8]) {
    if eval_lavars_used().is_none() {
        return;
    }

    let name_cstr = bytes_to_cstring(name_slice);
    let len = name_slice.len();
    let mut varname_opt: Option<&CStr> = None;
    let ht_dict = find_var_ht(&name_cstr, len, &mut varname_opt);
    let local_dict = get_funccal_local_dict();
    let args_dict = get_funccal_args_dict();

    if ht_dict.is_some()
        && (ht_dict == local_dict || ht_dict == args_dict)
        && find_var(&name_cstr, len, false, 1).is_some()
    {
        set_eval_lavars_used(Some(true));
    }
}

/// Find variable "name" in the list of variables.
pub fn find_var(
    name: &CStr,
    name_len: usize,
    has_htp: bool,
    no_autoload: i32,
) -> Option<crate::eval::typval::DictitemId> {
    let mut varname_opt: Option<&CStr> = None;
    let ht_dict = find_var_ht(name, name_len, &mut varname_opt)?;
    let varname = varname_opt.unwrap_or(c"");
    let varname_len =
        name_len.saturating_sub((varname.as_ptr() as usize).wrapping_sub(name.as_ptr() as usize));
    let ret = find_var_in_ht(
        &ht_dict.dv_hashtab,
        name.to_bytes().first().copied().unwrap_or(0) as i32,
        varname,
        varname_len,
        i32::from(no_autoload != 0 || has_htp),
    );
    if ret.is_some() {
        return ret;
    }

    let name_bytes = name.to_bytes();
    find_var_in_scoped_ht(
        &name_bytes[..name_len.min(name_bytes.len())],
        i32::from(no_autoload != 0 || has_htp),
    )
}

#[inline]
pub fn find_var_opt(
    name: &CStr,
    name_len: usize,
    has_htp: bool,
    no_autoload: i32,
) -> Option<crate::eval::typval::DictitemId> {
    find_var(name, name_len, has_htp, no_autoload)
}

/// Find variable in hashtab.
pub fn find_var_in_ht(
    ht: &Hashtab,
    htname: i32,
    varname: &CStr,
    varname_len: usize,
    no_autoload: i32,
) -> Option<crate::eval::typval::DictitemId> {
    if varname_len == 0 {
        let id = match htname as u8 {
            b's' => script_item_vars(g_st().current_sctx.get().sc_sid).0,
            b'g' => get_globvars_var(),
            b'v' => get_vimvars_var(),
            b'b' => curbuf_mut().b_bufvar.clone(),
            b'w' => curwin_mut().w_winvar.clone(),
            b't' => curtab_mut().tp_winvar.get(),
            b'l' => crate::eval::typval::DictitemId::from(get_funccal_local_var()),
            b'a' => crate::eval::typval::DictitemId::from(get_funccal_args_var()),
            _ => crate::eval::typval::DictitemId::NULL,
        };
        return if id.is_null() { None } else { Some(id) };
    }

    let varname_bytes = varname.to_bytes();
    let varname_slice = &varname_bytes[..varname_len.min(varname_bytes.len())];
    let mut hi = hash_find_len(ht, varname_slice);
    if hashitem_empty(&hi) {
        if std::ptr::eq(ht, &get_globvar_dict().dv_hashtab) && no_autoload == 0 {
            if !script_autoload(varname_slice, false) || aborting() {
                return None;
            }
            hi = hash_find_len(ht, varname_slice);
        }
        if hashitem_empty(&hi) {
            return None;
        }
    }
    Some(hi.hi_di)
}

/// Finds the dict (g:, l:, s:, …) and hashtable used for a variable.
fn find_var_ht_dict<'a>(
    name_in: &'a CStr,
    name_len: usize,
    varname: &mut Option<&'a CStr>,
    d: &mut Option<crate::eval::typval::DictId>,
) -> Option<crate::eval::typval::DictId> {
    *d = None;

    if name_len == 0 {
        return None;
    }
    let name_bytes = name_in.to_bytes();
    let name_slice = &name_bytes[..name_len.min(name_bytes.len())];
    if name_len == 1 || name_slice.get(1) != Some(&b':') {
        if matches!(name_slice[0], b':' | AUTOLOAD_CHAR) {
            return None;
        }
        *varname = Some(name_in);

        let compat_d = get_compat_dict();
        let hi = hash_find_len(&compat_d.dv_hashtab, name_slice);
        if !hashitem_empty(&hi) {
            return Some(compat_d);
        }

        if let Some(local_d) = get_funccal_local_dict() {
            *d = Some(local_d.clone());
            return Some(local_d);
        }

        let glob_d = get_globvar_dict();
        *d = Some(glob_d.clone());
        return Some(glob_d);
    }

    *varname = Some(&name_in[2..]);
    let scope_b = name_slice[0];
    let mut dict_ptr = crate::eval::typval::DictId::NULL;
    if scope_b == b'g' {
        dict_ptr = get_globvar_dict();
    } else if name_slice[2..]
        .iter()
        .any(|&b| b == b':' || b == AUTOLOAD_CHAR)
    {
        return None;
    }

    match scope_b {
        b'b' => dict_ptr = curbuf_mut().b_vars(),
        b'w' => dict_ptr = curwin_mut().w_vars(),
        b't' => dict_ptr = curtab_mut().tp_vars(),
        b'v' => dict_ptr = get_vimvar_dict(),
        b'a' => dict_ptr = get_funccal_args_dict().unwrap_or(crate::eval::typval::DictId::NULL),
        b'l' => dict_ptr = get_funccal_local_dict().unwrap_or(crate::eval::typval::DictId::NULL),
        b's' if (g_st().current_sctx.get().sc_sid > 0
            || g_st().current_sctx.get().sc_sid == SID_STR
            || g_st().current_sctx.get().sc_sid == SID_LUA)
            && g_st().current_sctx.get().sc_sid <= crate::runtime::script_items_len() =>
        {
            let mut sctx = g_st().current_sctx.get();
            nlua_set_sctx(&mut sctx);
            if sctx.sc_sid == SID_STR || sctx.sc_sid == SID_LUA {
                new_script_item(None, &mut sctx.sc_sid);
            }
            g_st().current_sctx.set(sctx);
            dict_ptr = script_item_vars(sctx.sc_sid).1;
        }
        _ => {}
    }

    if !dict_ptr.is_null() {
        *d = Some(dict_ptr.clone());
        Some(dict_ptr)
    } else {
        None
    }
}

/// Find the hashtable used for a variable.
pub fn find_var_ht<'a>(
    name: &'a CStr,
    name_len: usize,
    varname: &mut Option<&'a CStr>,
) -> Option<crate::eval::typval::DictId> {
    let mut d: Option<crate::eval::typval::DictId> = None;
    find_var_ht_dict(name, name_len, varname, &mut d)
}

/// Return the string value of a (global/local) variable or NULL when it doesn't exist.
pub fn get_var_value(name: &std::ffi::CStr) -> Option<std::borrow::Cow<'static, CStr>> {
    let v = find_var_opt(name, name.to_bytes().len(), false, 0)?;
    Some(std::borrow::Cow::Owned(tv_get_string(&v.tv()).into_owned()))
}

/// Allocate a new hashtab for a sourced script.
pub fn new_script_vars() -> Box<Scriptvar> {
    let mut sv = Box::new(Scriptvar::new());
    let dict = crate::eval::typval::tv_dict_alloc_static();
    let var = crate::eval::typval::tv_dict_item_alloc_static();
    init_var_dict(&dict, &var, VAR_SCOPE);
    sv.sv_dict = dict;
    sv.sv_var = var;
    sv
}

/// Initialize dictionary "dict" as a scope and set variable "dict_var" to point to it.
pub fn init_var_dict(dict: &TvDict, dict_var: &ScopeDictDictItem, scope: ScopeType) {
    hash_clear(&dict.dv_hashtab);
    dict.dv_lock.set(VAR_UNLOCKED);
    dict.dv_scope.set(scope);
    dict.dv_refcount.set(DO_NOT_FREE_CNT);
    dict.dv_copy_id.set(0);
    dict_var.set_tv(TypVal {
        v_type: VarType::VarDict,
        v_lock: VAR_FIXED,
        vval: crate::eval::typval::TypvalVval::Dict(dict.id()),
    });
    dict_var.di_flags.set(DI_FLAGS_RO | DI_FLAGS_FIX);
    dict.watchers.borrow_mut().clear();
}

/// Unreference a dictionary initialized by init_var_dict().
pub fn unref_var_dict(dict: &TvDict) {
    dict.dv_refcount
        .set(dict.dv_refcount.get() - (DO_NOT_FREE_CNT - 1));
    tv_dict_unref(Some(dict));
}

/// Clean up a list of internal variables.
pub fn vars_clear(ht: &Hashtab) {
    vars_clear_ext(ht, true);
}

/// Like vars_clear(), but only free the value if "free_val" is true.
pub fn vars_clear_ext(ht: &Hashtab, free_val: bool) {
    hash_lock(ht);
    for (_, v) in ht.iter_items() {
        if !free_val {
            v.set_tv(TypVal::new());
        }
        if (v.di_flags.get() & DI_FLAGS_ALLOC) != 0 {
            crate::eval::typval::tv_dict_item_free(&v);
        } else if free_val {
            v.clear_tv();
        }
    }
    hash_clear(ht);
}

/// Delete a variable from hashtab "ht" at slot "hi_idx".
fn delete_var(ht: &Hashtab, hi_idx: usize) {
    let di = ht.slot(hi_idx).hi_di;
    hash_remove(ht, hi_idx);
    if let Some(di) = di.opt() {
        crate::eval::typval::tv_dict_item_free(di);
    }
}

/// List the value of one internal variable.
fn list_one_var(v: &Dictitem, prefix: &CStr, first: &mut i32) {
    let mut v_tv = v.tv().clone();
    let s = encode_tv2echo(&mut v_tv, None);
    let key_cstr = v.key_cstr();
    list_one_var_a(
        prefix,
        Some(key_cstr),
        key_cstr.to_bytes().len() as isize,
        v_tv.v_type,
        &s,
        first,
    );
}

fn list_one_var_a(
    prefix: &CStr,
    name: Option<&CStr>,
    name_len: isize,
    v_type: VarType,
    string_in: &CStr,
    first: &mut i32,
) {
    let mut string = string_in;
    if *first != 0 {
        msg_ext_set_kind(c"list_cmd");
        msg_start();
    } else {
        msg_putchar(b'\n' as i32);
    }
    if !prefix.to_bytes().is_empty() {
        msg_puts(prefix);
    }
    if let Some(name) = name {
        msg_puts_len(name.to_bytes(), name_len, 0, false);
    }
    msg_putchar(b' ' as i32);
    msg_advance(22);
    if v_type == VarType::VarNumber {
        msg_putchar(b'#' as i32);
    } else if v_type == VarType::VarFunc || v_type == VarType::VarPartial {
        msg_putchar(b'*' as i32);
    } else if v_type == VarType::VarList {
        msg_putchar(b'[' as i32);
        if string.to_bytes().first() == Some(&b'[') {
            string = &string[1..];
        }
    } else if v_type == VarType::VarDict {
        msg_putchar(b'{' as i32);
        if string.to_bytes().first() == Some(&b'{') {
            string = &string[1..];
        }
    } else {
        msg_putchar(b' ' as i32);
    }

    msg_outtrans(string, 0, false);

    if v_type == VarType::VarFunc || v_type == VarType::VarPartial {
        msg_puts(c"()");
    }
    if *first != 0 {
        msg_clr_eos();
        *first = 0;
    }
}

/// Additional handling for setting a v: variable.
pub fn before_set_vvar(
    varname: &CStr,
    di: &Dictitem,
    tv: &mut TypVal,
    copy: bool,
    watched: bool,
    type_error: &mut bool,
) -> bool {
    let di_vtype = di.tv().v_type;
    if di_vtype == VarType::VarString {
        let mut oldtv: TypVal = TypVal::new();
        if watched {
            tv_copy(&di.tv(), &mut oldtv);
        }
        di.clear_tv();
        if copy || tv.v_type != VarType::VarString {
            let val = tv_get_string(&*tv);
            crate::eval::typval::tv_set_string(&mut di.tv_mut(), Some(&val));
        } else {
            let mut di_tv = di.tv_mut();
            di_tv.v_type = VarType::VarString;
            di_tv.vval =
                std::mem::replace(&mut tv.vval, crate::eval::typval::TypvalVval::NULL_STRING);
        }
        if watched {
            let new_tv = di.tv().clone();
            tv_dict_watcher_notify(&get_vimvar_dict(), varname, Some(&new_tv), Some(&oldtv));
            tv_clear(&mut oldtv);
        }
        return false;
    } else if di_vtype == VarType::VarNumber {
        let mut oldtv: TypVal = TypVal::new();
        if watched {
            tv_copy(&di.tv(), &mut oldtv);
        }
        let n = tv_get_number(&*tv);
        di.tv_mut().vval = crate::eval::typval::TypvalVval::Number(n);
        if varname.to_bytes() == b"searchforward" {
            set_search_direction(if n != 0 { b'/' as i32 } else { b'?' as i32 });
        } else if varname.to_bytes() == b"hlsearch" {
            g_st().search_global.no_hlsearch.set(n == 0);
            redraw_all_later(UPD_SOME_VALID);
        }
        if watched {
            let new_tv = di.tv().clone();
            tv_dict_watcher_notify(&get_vimvar_dict(), varname, Some(&new_tv), Some(&oldtv));
            tv_clear(&mut oldtv);
        }
        return false;
    } else if di_vtype != tv.v_type {
        *type_error = true;
        return false;
    }
    true
}

/// Set variable to the given value.
pub fn set_var(name_slice: &[u8], tv: &mut TypVal, copy: bool) {
    set_var_const(name_slice, tv, copy, false);
}

/// Set variable to the given value, optionally locking as const.
pub fn set_var_const(name_slice: &[u8], tv: &mut TypVal, copy: bool, is_const: bool) {
    let name_cstr = bytes_to_cstring(name_slice);
    let name_len = name_slice.len();
    let mut varname_opt: Option<&CStr> = None;
    let mut dict_opt: Option<crate::eval::typval::DictId> = None;
    let ht_dict_opt = find_var_ht_dict(&name_cstr, name_len, &mut varname_opt, &mut dict_opt);
    let watched = tv_dict_is_watched(dict_opt.as_deref());

    let Some(ht_dict) = ht_dict_opt else {
        semsg!(gettext(E_ILLVAR), &name_cstr);
        return;
    };
    let ht_ref = &ht_dict.dv_hashtab;
    let Some(varname_cstr) = varname_opt.filter(|s| !s.to_bytes().is_empty()) else {
        semsg!(gettext(E_ILLVAR), &name_cstr);
        return;
    };
    let varname_len = name_len
        .saturating_sub((varname_cstr.as_ptr() as usize).wrapping_sub(name_cstr.as_ptr() as usize));
    let mut di_opt = find_var_in_ht(ht_ref, 0, varname_cstr, varname_len, 1);

    if di_opt.is_none() {
        di_opt = find_var_in_scoped_ht(&name_slice[..name_len], 1);
    }

    if tv_is_func(tv) && var_wrong_func_name(&name_cstr, di_opt.is_none()) {
        return;
    }

    let is_vimvar_ht = ht_dict == get_vimvar_dict();
    let mut oldtv: TypVal = TypVal::new();
    let di: crate::eval::typval::DictitemId = if let Some(di) = di_opt {
        if is_const {
            emsg(gettext(E_CANNOT_MOD));
            return;
        }

        if var_check_ro(i32::from(di.di_flags.get()), &name_cstr, name_len)
            || value_check_lock(di.tv().v_lock, Some(&name_cstr), name_len)
            || var_check_lock(i32::from(di.di_flags.get()), &name_cstr, name_len)
        {
            return;
        }

        let mut type_error = false;
        if is_vimvar_ht && !before_set_vvar(varname_cstr, &di, tv, copy, watched, &mut type_error) {
            if type_error {
                semsg!(
                    gettext(E_SETTING_V_STR_TO_VALUE_WITH_WRONG_TYPE),
                    varname_cstr,
                );
            }
            return;
        }

        if watched {
            tv_copy(&di.tv(), &mut oldtv);
        }
        di.clear_tv();
        di
    } else {
        if is_vimvar_ht || get_funccal_args_dict().is_some_and(|ad| ad == ht_dict) {
            semsg!(gettext(E_ILLVAR), &name_cstr);
            return;
        }

        if !valid_varname(varname_cstr) {
            return;
        }

        debug_assert!(dict_opt.is_some());

        let varname_bytes = varname_cstr.to_bytes();
        let new_di = crate::eval::typval::tv_dict_item_alloc_len(
            &varname_bytes[..varname_len.min(varname_bytes.len())],
        );
        if !hash_add(ht_ref, &new_di) {
            crate::eval::typval::tv_dict_item_free(&new_di);
            return;
        }
        if is_const {
            new_di.di_flags.set(new_di.di_flags.get() | DI_FLAGS_LOCK);
        }
        new_di
    };

    if copy || tv.v_type == VarType::VarNumber || tv.v_type == VarType::VarFloat {
        tv_copy(&*tv, &mut di.tv_mut());
    } else {
        let mut taken = std::mem::take(tv);
        taken.v_lock = VAR_UNLOCKED;
        di.set_tv(taken);
    }

    if watched {
        let key_cstr = di.key_cstr();
        let new_tv = di.tv().clone();
        tv_dict_watcher_notify(
            dict_opt.as_deref().unwrap(),
            key_cstr,
            Some(&new_tv),
            Some(&oldtv),
        );
        tv_clear(&mut oldtv);
    }

    if is_const {
        let mut item_tv = di.tv().clone();
        tv_item_lock(&mut item_tv, DICT_MAXNEST, true, true);
        di.tv_mut().v_lock = item_tv.v_lock;
    }
}

/// Check whether variable is read-only (DI_FLAGS_RO, DI_FLAGS_RO_SBX).
pub fn var_check_ro(flags: i32, name_in: &CStr, mut name_len: usize) -> bool {
    let error_message = if (flags as u8 & DI_FLAGS_RO) != 0 {
        Some(E_CANNOT_CHANGE_READONLY_VARIABLE_STR)
    } else if (flags as u8 & DI_FLAGS_RO_SBX) != 0 && g_st().sandbox.get() != 0 {
        Some(E_CANNOT_SET_VARIABLE_IN_SANDBOX_STR)
    } else {
        None
    };

    let Some(error_message) = error_message else {
        return false;
    };
    let mut name = name_in;
    if name_len == TV_TRANSLATE {
        name = gettext(name_in);
        name_len = name.to_bytes().len();
    } else if name_len == TV_CSTRING {
        name_len = name_in.to_bytes().len();
    }

    semsg!(gettext(error_message), name_len as i32, name);
    true
}

/// Return true if di_flags "flags" indicates variable "name" is locked.
pub fn var_check_lock(flags: i32, name_in: &CStr, mut name_len: usize) -> bool {
    if (flags as u8 & DI_FLAGS_LOCK) == 0 {
        return false;
    }

    let mut name = name_in;
    if name_len == TV_TRANSLATE {
        name = gettext(name_in);
        name_len = name.to_bytes().len();
    } else if name_len == TV_CSTRING {
        name_len = name_in.to_bytes().len();
    }

    semsg!(
        gettext(c"E1122: Variable is locked: %.*s"),
        name_len as i32,
        name,
    );
    true
}

/// Check whether variable is fixed (DI_FLAGS_FIX).
pub fn var_check_fixed(flags: i32, name_in: &CStr, mut name_len: usize) -> bool {
    if (flags as u8 & DI_FLAGS_FIX) != 0 {
        let mut name = name_in;
        if name_len == TV_TRANSLATE {
            name = gettext(name_in);
            name_len = name.to_bytes().len();
        } else if name_len == TV_CSTRING {
            name_len = name_in.to_bytes().len();
        }
        semsg!(gettext(E_CANNOT_DELETE_VARIABLE_STR), name_len as i32, name,);
        return true;
    }
    false
}

/// Check if name is a valid name to assign funcref to.
pub fn var_wrong_func_name(name: &CStr, new_var: bool) -> bool {
    let bytes = name.to_bytes();
    let has_colon = bytes.get(1) == Some(&b':');
    let first_after_scope = if has_colon {
        bytes.get(2).copied().unwrap_or(0)
    } else {
        bytes.first().copied().unwrap_or(0)
    };
    if !(has_colon && matches!(bytes[0], b'w' | b'b' | b's' | b't'))
        && !first_after_scope.is_ascii_uppercase()
        && !bytes.contains(&b'#')
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

/// Check if a variable name is valid.
pub fn valid_varname(varname: &CStr) -> bool {
    for (i, &b) in varname.to_bytes().iter().enumerate() {
        if !eval_isnamec1(i32::from(b)) && (i == 0 || !ascii_isdigit(b)) && b != AUTOLOAD_CHAR {
            semsg!(gettext(E_ILLVAR), varname);
            return false;
        }
    }
    true
}

/// Implements the logic to retrieve local variable and option values.
fn get_var_from(
    varname_opt: Option<&CStr>,
    rettv: &mut TypVal,
    deftv: &mut TypVal,
    htname: i32,
    tp_in: Option<&TabPageInfo>,
    win_in: Option<&WinInfoInfo>,
    buf_in: Option<&FileBuf>,
) {
    let tp = crate::globals::TabpagePtr::from(tp_in);
    let win = crate::globals::WinPtr::from(win_in);
    let buf = crate::globals::BufPtr::from(buf_in);
    let mut done = false;
    let do_change_curbuf = !buf.is_null() && htname == b'b' as i32;

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    if let Some(varname_cstr) = varname_opt
        && !tp.is_null()
        && !win.is_null()
        && (htname != b'b' as i32 || !buf.is_null())
    {
        let need_switch_win =
            !(tp == g_st().curtab.get() && win == curwin_ptr()) && !do_change_curbuf;
        let mut switchwin: CtxSwitch = CtxSwitch::new();
        if !need_switch_win
            || ctx_switch(
                &mut switchwin,
                win.opt_mut(),
                tp.opt(),
                None,
                K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
            )
        {
            if varname_cstr.to_bytes().first() == Some(&b'&') && htname != b't' as i32 {
                let save_curbuf = g_st().curbuf.get();

                if do_change_curbuf {
                    g_st().curbuf.set(buf);
                }

                if varname_cstr.to_bytes().len() == 1 {
                    let opts = get_winbuf_options(i32::from(htname == b'b' as i32));
                    tv_dict_set_ret(rettv, Some(&opts));
                    done = true;
                } else {
                    let mut varname_c = varname_cstr;
                    if eval_option(&mut varname_c, Some(rettv), true) {
                        done = true;
                    }
                }

                g_st().curbuf.set(save_curbuf);
            } else if varname_cstr.to_bytes().is_empty() {
                let v = if htname == b'b' as i32 {
                    buf.b_bufvar.clone()
                } else if htname == b'w' as i32 {
                    win.w_winvar.clone()
                } else {
                    tp.tp_winvar.get()
                };
                tv_copy(&v.tv(), rettv);
                done = true;
            } else {
                let d_scope = if htname == b'b' as i32 {
                    buf.borrow_mut().b_vars()
                } else if htname == b'w' as i32 {
                    win.borrow_mut().w_vars()
                } else {
                    tp.borrow().tp_vars()
                };

                if let Some(v) = find_var_in_ht(
                    &d_scope.dv_hashtab,
                    htname,
                    varname_cstr,
                    varname_cstr.to_bytes().len(),
                    0,
                ) {
                    tv_copy(&v.tv(), rettv);
                    done = true;
                }
            }
        }

        if need_switch_win {
            ctx_restore(&mut switchwin);
        }
    }

    if !done && deftv.v_type != VarType::VarUnknown {
        tv_copy(&*deftv, rettv);
    }

    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
}

/// getwinvar() and gettabwinvar().
fn getwinvar(argvars: &mut [TypVal], rettv: &mut TypVal, off: usize) {
    let (head, tail) = argvars.split_at_mut(off + 2);
    let tp = if off == 1 {
        crate::globals::TabpagePtr::from(find_tabpage(tv_get_number_chk(&head[0], None) as i32))
    } else {
        g_st().curtab.get().clone()
    };
    let win =
        crate::globals::WinPtr::from(tp.opt().and_then(|tp| find_win_by_nr(&head[off], Some(tp))));
    let varname = tv_get_string_chk(&head[off + 1]);

    get_var_from(
        varname.as_deref(),
        rettv,
        &mut tail[0],
        b'w' as i32,
        tp.opt(),
        win.opt_mut(),
        None,
    );
}

/// Converts a typval to a structured option value.
fn opt_from_tv(tv: &mut TypVal, opt_idx: OptIndex, option: &CStr, error: &mut bool) -> Object {
    let mut value = nil_obj();
    let mut nbuf = [0u8; NUMBUFLEN];
    let mut err = false;
    let is_tty_opt = is_tty_option(option.to_bytes());
    let option_has_bool = !is_tty_opt && option_has_type(opt_idx, ObjectType::ObjectTypeBoolean);
    let option_has_num = !is_tty_opt && option_has_type(opt_idx, ObjectType::ObjectTypeInteger);
    let option_has_str = is_tty_opt || option_has_type(opt_idx, ObjectType::ObjectTypeString);

    if !is_tty_opt && (get_option(opt_idx).flags() & K_OPT_FLAG_FUNC) != 0 && tv_is_func(tv) {
        let strval = encode_tv2string(tv, None);
        value = crate::api::private::helpers::cstr_to_obj(Some(&*strval));
    } else if option_has_bool || option_has_num {
        let n = if option_has_num {
            tv_get_number_chk(&*tv, Some(&mut err))
        } else {
            tv_get_bool_chk(&*tv, Some(&mut err))
        };
        if !err && tv.v_type == VarType::VarString && n == 0 {
            let s_bytes = tv.vval.v_string_bytes();
            if s_bytes.is_empty() || !s_bytes.iter().all(|&b| b == b'0') {
                err = true;
                semsg!(
                    gettext(c"E521: Number required: &%s = '%s'"),
                    option,
                    tv.vval.v_string_cstr_or_empty(),
                );
            }
        }
        value = if option_has_num {
            integer_obj(n as OptInt)
        } else {
            opt_from_tristate(tristate_from_int(n as OptInt))
        };
    } else if option_has_str {
        if tv.v_type != VarType::VarBool && tv.v_type != VarType::VarSpecial {
            let strval = tv_get_string_buf_chk(&*tv, &mut nbuf);
            err = strval.is_none();
            value = string_obj(cstr_to_string(strval));
        } else {
            err = true;
            emsg(gettext(E_STRING_REQUIRED));
        }
    } else {
        std::process::abort();
    }

    *error = err;
    value
}

/// Converts an option value to typval.
pub fn opt_to_tv(value: Object, numbool: bool) -> TypVal {
    let mut rettv = TypVal {
        v_type: VarType::VarSpecial,
        v_lock: 0,
        vval: TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST),
    };

    match value.obj_type {
        ObjectType::ObjectTypeUnset => {
            if numbool {
                rettv.v_type = VarType::VarNumber;
                rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(K_NONE));
            }
        }
        ObjectType::ObjectTypeNil => {}
        ObjectType::ObjectTypeBoolean => {
            if numbool {
                rettv.v_type = VarType::VarNumber;
                rettv.vval =
                    crate::eval::typval::TypvalVval::Number(Varnumber::from(value.data.boolean()));
            } else {
                rettv.v_type = VarType::VarBool;
                rettv.vval = crate::eval::typval::TypvalVval::Bool(if value.data.boolean() {
                    K_BOOL_VAR_TRUE_CONST
                } else {
                    K_BOOL_VAR_FALSE_CONST
                });
            }
        }
        ObjectType::ObjectTypeInteger => {
            rettv.v_type = VarType::VarNumber;
            rettv.vval = crate::eval::typval::TypvalVval::Number(value.data.integer());
        }
        ObjectType::ObjectTypeString => {
            if !value.data.string().data.is_null() {
                crate::eval::typval::tv_set_string_len(
                    &mut rettv,
                    Some(value.data.string().as_bytes()),
                );
            } else {
                rettv.v_type = VarType::VarString;
                rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
            }
        }
        ObjectType::ObjectTypeLuaRef => {
            let s = nlua_funcref_str(value.data.luaref(), true);
            crate::eval::typval::tv_set_string_len(&mut rettv, Some(s.as_bytes()));
        }
        _ => {
            std::process::abort();
        }
    }

    rettv
}

/// Set option "varname" to the value of "varp" for the current buffer/window.
fn set_option_from_tv(varname: &CStr, varp: &mut TypVal) {
    let opt_idx = find_option(varname.to_bytes());
    if opt_idx == K_OPT_INVALID {
        semsg!(gettext(E_UNKNOWN_OPTION2), varname);
        return;
    }

    let mut error = false;
    let value = opt_from_tv(varp, opt_idx, varname, &mut error);

    if !error {
        let mut errbuf_storage = [0u8; IOSIZE];
        let errmsg = set_option_value(
            opt_idx,
            value.clone(),
            OPT_LOCAL,
            true,
            Some(&mut errbuf_storage),
        );
        if let Some(errmsg) = errmsg {
            emsg(errmsg);
        }
    }
    optval_free(value);
}

/// "setwinvar()" and "settabwinvar()" functions.
fn setwinvar(argvars: &mut [TypVal], off: usize) {
    if check_secure() {
        return;
    }

    let (head, tail) = argvars.split_at_mut(off + 2);
    let varp = &mut tail[0];
    let tp = if off == 1 {
        crate::globals::TabpagePtr::from(find_tabpage(tv_get_number_chk(&head[0], None) as i32))
    } else {
        g_st().curtab.get().clone()
    };
    let win =
        crate::globals::WinPtr::from(tp.opt().and_then(|tp| find_win_by_nr(&head[off], Some(tp))));
    let varname_opt = tv_get_string_chk(&head[off + 1]);

    if win.is_null() {
        return;
    }
    let Some(varname) = varname_opt else {
        return;
    };

    let need_switch_win = !(g_st().curtab.get() == tp && win == curwin_ptr());
    let mut switchwin: CtxSwitch = CtxSwitch::new();
    if !need_switch_win
        || ctx_switch(
            &mut switchwin,
            win.opt_mut(),
            tp.opt(),
            None,
            K_CTX_NO_EVENTS | K_CTX_NO_DISPLAY,
        )
    {
        if varname.to_bytes().first() == Some(&b'&') {
            set_option_from_tv(&varname[1..], varp);
        } else {
            let varname_bytes = varname.to_bytes_with_nul();
            let varname_len = varname.to_bytes().len();
            let mut winvarname = Vec::with_capacity(varname_len + 3);
            winvarname.push(b'w');
            winvarname.push(b':');
            winvarname.extend_from_slice(varname_bytes);
            set_var(&winvarname[..varname_len + 2], varp, true);
        }
    }
    if need_switch_win {
        ctx_restore(&mut switchwin);
    }
}

pub fn reset_v_option_vars() {
    set_vim_var_string(VV_OPTION_NEW, None, -1);
    set_vim_var_string(VV_OPTION_OLD, None, -1);
    set_vim_var_string(VV_OPTION_OLDLOCAL, None, -1);
    set_vim_var_string(VV_OPTION_OLDGLOBAL, None, -1);
    set_vim_var_string(VV_OPTION_COMMAND, None, -1);
    set_vim_var_string(VV_OPTION_TYPE, None, -1);
}

/// Add an assert error to v:errors.
pub fn assert_error(buf: &[u8]) {
    let di = get_vim_var_tv(VV_ERRORS);

    if di.tv().v_type != VarType::VarList || di.tv().vval.v_list().is_null() {
        set_vim_var_list(VV_ERRORS, Some(&tv_list_alloc(1)));
    }
    let list = get_vim_var_tv(VV_ERRORS).tv().vval.v_list();
    crate::eval::typval::tv_list_append_bytes(&list, buf);
}

pub fn var_exists(var_in: &CStr) -> bool {
    let mut var = var_in;
    let mut tofree: Option<std::ffi::CString> = None;
    let mut n = false;

    let start_var = var;
    let mut name = var;
    let len = get_name_len(&mut var, &mut tofree, true, false);
    if len > 0 {
        let mut tv: TypVal = TypVal::new();

        if let Some(ref tf) = tofree {
            name = tf.as_c_str();
        }
        n = eval_variable(name, len, Some(&mut tv), None, false, true);
        if n {
            n = handle_subscript(
                &mut var,
                start_var,
                &mut tv,
                Some(&mut Evalarg { eval_flags: 1 }),
                false,
            );
            if n {
                tv_clear(&mut tv);
            }
        }
    }
    if !var.to_bytes().is_empty() {
        n = false;
    }

    drop(tofree);
    n
}

/// Start recording command output to a variable.
pub fn var_redir_start(name: &CStr, append: bool) -> bool {
    if !eval_isnamec1(i32::from(name.to_bytes().first().copied().unwrap_or(0))) {
        emsg(gettext(E_INVARG));
        return false;
    }

    let name_cstr = name.to_owned();
    REDIR_VARNAME.with(|c| *c.borrow_mut() = Some(name_cstr.clone()));
    VARS_STATE.with(|s| s.redir_active.set(true));
    REDIR_BUF.with(|c| c.borrow_mut().clear());

    let mut lval = Lval::new();
    let endp_opt = get_lval(
        &name_cstr,
        None,
        &mut lval,
        false,
        false,
        0,
        FNE_CHECK_START,
    )
    .map(CStr::to_owned);
    REDIR_ENDP.with(|c| *c.borrow_mut() = endp_opt.clone());

    if endp_opt.as_deref().is_none_or(|s| !s.to_bytes().is_empty()) || lval.ll_name.is_none() {
        if let Some(endp) = endp_opt.as_deref().filter(|s| !s.to_bytes().is_empty()) {
            semsg!(gettext(E_TRAILING_ARG), endp);
        } else {
            semsg!(gettext(E_INVARG2), name);
        }
        REDIR_ENDP.with(|c| *c.borrow_mut() = None);
        var_redir_stop();
        return false;
    }

    let called_emsg_before = g_st().called_emsg.get();
    g_st().did_emsg.set(0);
    let mut tv = TypVal {
        v_type: VarType::VarString,
        v_lock: 0,
        vval: TypvalVval::from_cstr(c""),
    };
    let endp_cstr = endp_opt.as_deref().unwrap_or(c"");
    if append {
        set_var_lval(&mut lval, endp_cstr, &mut tv, true, false, Some(c"."));
    } else {
        set_var_lval(&mut lval, endp_cstr, &mut tv, true, false, Some(c"="));
    }
    drop(lval);
    if g_st().called_emsg.get() > called_emsg_before {
        REDIR_ENDP.with(|c| *c.borrow_mut() = None);
        var_redir_stop();
        return false;
    }

    true
}

/// Append `value` to the variable set by var_redir_start().
pub fn var_redir_str(value: &[u8]) {
    if !VARS_STATE.with(|s| s.redir_active.get()) {
        return;
    }

    if !value.is_empty() {
        REDIR_BUF.with(|c| c.borrow_mut().extend_from_slice(value));
    }
}

/// Stop redirecting command output to a variable.
pub fn var_redir_stop() {
    if VARS_STATE.with(|s| s.redir_active.get()) {
        if REDIR_ENDP.with(|c| c.borrow().is_some()) {
            let mut buf = REDIR_BUF.with(|c| std::mem::take(&mut *c.borrow_mut()));
            buf.push(0);
            let mut tv = TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::from_cstr(crate::memory::bytes_as_cstr(&buf)),
            };
            let vn_owned = REDIR_VARNAME.with(|c| c.borrow().clone());
            let vn_cstr = vn_owned.as_deref().unwrap_or(c"");

            let mut lval = Lval::new();
            let endp_owned = get_lval(vn_cstr, None, &mut lval, false, false, 0, FNE_CHECK_START)
                .map(CStr::to_owned);
            REDIR_ENDP.with(|c| *c.borrow_mut() = endp_owned.clone());

            if let Some(ref endp) = endp_owned
                && lval.ll_name.is_some()
            {
                set_var_lval(&mut lval, endp, &mut tv, false, false, Some(c"."));
            }
        }

        REDIR_BUF.with(|c| *c.borrow_mut() = Vec::new());
        VARS_STATE.with(|s| s.redir_active.set(false));
    }
    REDIR_VARNAME.with(|c| *c.borrow_mut() = None);
}

/// "gettabvar()" function.
pub fn f_gettabvar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };
    let varname = tv_get_string_chk(arg1);
    let tp = crate::globals::TabpagePtr::from(find_tabpage(tv_get_number_chk(arg0, None) as i32));
    let mut win = crate::globals::WinPtr::NULL;

    if let Some(tp_ref) = tp.opt() {
        win = if g_st().curtab.get() == tp_ref || tp_ref.tp_firstwin.get().is_null() {
            g_st().firstwin.get()
        } else {
            tp_ref.tp_firstwin.get()
        };
    }

    get_var_from(
        varname.as_deref(),
        &mut *rettv,
        arg2,
        b't' as i32,
        tp.opt(),
        win.opt_mut(),
        None,
    );
}

/// "gettabwinvar()" function.
pub fn f_gettabwinvar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getwinvar(argvars, rettv, 1);
}

/// "getwinvar()" function.
pub fn f_getwinvar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    getwinvar(argvars, rettv, 0);
}

/// "getbufvar()" function.
pub fn f_getbufvar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };
    let varname = tv_get_string_chk(arg1);
    let buf = tv_get_buf_from_arg(arg0);

    get_var_from(
        varname.as_deref(),
        &mut *rettv,
        arg2,
        b'b' as i32,
        Some(&curtab_mut()),
        Some(&curwin_mut()),
        buf.as_deref(),
    );
}

/// "settabvar()" function.
pub fn f_settabvar(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() {
        return;
    }

    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };
    let tp = find_tabpage(tv_get_number_chk(arg0, None) as i32);
    let Some(varname) = tv_get_string_chk(arg1) else {
        return;
    };
    let Some(tp_ref) = tp else {
        return;
    };
    let varname_len = varname.to_bytes().len();
    let mut tabvarname = Vec::with_capacity(varname_len + 3);
    tabvarname.push(b't');
    tabvarname.push(b':');
    tabvarname.extend_from_slice(varname.to_bytes_with_nul());

    let save_curtab = g_st().curtab.get().clone();
    let save_lu_tp = g_st().lastused_tabpage.get().clone();
    goto_tabpage_tp(Some(&tp_ref), false, false);

    set_var(&tabvarname[..varname_len + 2], arg2, true);

    if valid_tabpage(save_curtab.opt()) {
        goto_tabpage_tp(save_curtab.opt(), false, false);
        if valid_tabpage(save_lu_tp.opt()) {
            g_st().lastused_tabpage.set(save_lu_tp);
        }
    }
}

/// "settabwinvar()" function.
pub fn f_settabwinvar(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    setwinvar(argvars, 1);
}

/// "setwinvar()" function.
pub fn f_setwinvar(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    setwinvar(argvars, 0);
}

/// "setbufvar()" function.
pub fn f_setbufvar(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    if check_secure() || !tv_check_str_or_nr(&argvars[0]) {
        return;
    }
    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };
    let varname_owned = tv_get_string_chk(arg1);
    let Some(buf) = tv_get_buf(arg0, 0) else {
        return;
    };

    let Some(varname) = varname_owned else {
        return;
    };

    if varname.to_bytes().first() == Some(&b'&') {
        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, None, None, Some(&buf), 0);
        set_option_from_tv(&varname[1..], arg2);
        ctx_restore(&mut aco);
    } else {
        let varname_len = varname.to_bytes().len();
        let mut bufvarname = Vec::with_capacity(varname_len + 3);
        let save_curbuf = g_st().curbuf.get();
        g_st().curbuf.set(buf);
        bufvarname.push(b'b');
        bufvarname.push(b':');
        bufvarname.extend_from_slice(varname.to_bytes_with_nul());
        set_var(&bufvarname[..varname_len + 2], arg2, true);
        g_st().curbuf.set(save_curbuf);
    }
}
