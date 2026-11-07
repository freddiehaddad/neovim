//! Vimscript `TypVal`, `TvList`, `TvDict`, `TvBlob`, and `Callback` operations (`src/nvim/eval/typval.c`).

use crate::charset::vim_str2nr;
use crate::eval::userfunc::{
    Funcexe, UserFunc, call_func, func_ref, func_unref, get_funccal_local_ht,
};

use crate::cursor::Linenr;
use crate::errors::*;

use crate::buffer::{DO_NOT_FREE_CNT, FileBuf, VAR_FIXED};
use crate::eval::encode::{
    ENCODE_BOOL_VAR_NAMES, ENCODE_SPECIAL_VAR_NAMES, MPConvPartialStage, MPConvStackData,
    MPConvStackVal, encode_tv2echo, encode_tv2string,
};
use crate::eval::executor::eexe_mod_op;
use crate::eval::gc::{gc_register_dict, gc_register_list, gc_unregister_dict, gc_unregister_list};
use crate::eval::vars::{
    get_globvar_dict, valid_varname, var_check_fixed, var_check_ro, var_wrong_func_name,
};
use crate::eval::{
    DICT_MAXNEST, K_LIST_LEN_MAY_KNOW_CONST, STR2NR_ALL, callback_call, callback_from_typval,
    func_equal, get_copy_id, partial_name, partial_unref, set_selfdict, var_item_copy, var2fpos,
};
use crate::globals::*;
use crate::hashtab::{
    Hashitem, hash_add, hash_clear, hash_lock, hash_remove, hash_unlock, hashitem_empty,
};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_free_luaref, api_new_luaref, nlua_funcref_str};
use crate::mbyte::{Vimconv, mb_strcmp_ic, string_convert, utf_char2bytes, utfc_ptr2len};
use crate::memory::bytes_to_cstring;
use crate::message::{emsg, internal_error, semsg};
use crate::os::input::{fast_breakcheck, line_breakcheck};
use crate::strings::{RawCCharPtrMut, format_to_buf, strcoll, strtod, tr};
use std::ffi::CStr;
use std::ptr;

use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO};
use crate::cmdexpand::MB_MAXBYTES;
use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};

use crate::eval::funcs::K_SPECIAL_VAR_NULL_CONST;
use crate::regexp::NUMBUFLEN;
use crate::viml::parser::parser::CONV_NONE;
pub type TvFloat = f64;
pub type Varnumber = i64;
pub type VarLockStatus = i32;
pub type BoolVarValue = i32;

#[derive(Clone, Copy, Default)]
pub enum EvalFuncData {
    #[default]
    None,
    FloatOp(fn(TvFloat) -> TvFloat),
    ApiHandler(&'static crate::api::private::dispatch::MsgpackRpcRequestHandler),
    LuaFunc(&'static CStr),
}

pub(crate) const VAR_UNLOCKED: i32 = 0;
pub const K_CALLBACK_NONE_CONST: i32 = 0;
pub const K_CALLBACK_FUNCREF: i32 = 1;
pub const K_CALLBACK_PARTIAL: i32 = 2;
pub const K_CALLBACK_LUA: i32 = 3;
pub const K_CALLBACK_EXPR: i32 = 4;

#[repr(C)]
pub struct TvBlob {
    pub bv_ga: Vec<u8>,
    pub bv_refcount: i32,
    pub bv_lock: i32,
}

#[repr(C)]
pub struct TvPartial {
    pub pt_refcount: i32,
    pub pt_copy_id: i32,
    pub pt_name: Option<std::ffi::CString>,
    pub pt_func: crate::globals::GraphPtr<UserFunc>,
    pub pt_auto: bool,
    pub pt_argv: Vec<TypVal>,
    pub pt_dict: crate::globals::GraphPtr<TvDict>,
}

impl Default for TvPartial {
    fn default() -> Self {
        Self {
            pt_refcount: 0,
            pt_copy_id: 0,
            pt_name: None,
            pt_func: crate::globals::GraphPtr::NULL,
            pt_auto: false,
            pt_argv: Vec::new(),
            pt_dict: crate::globals::GraphPtr::NULL,
        }
    }
}

impl Drop for TvPartial {
    fn drop(&mut self) {
        for tv in &mut self.pt_argv {
            tv_clear(tv);
        }
        tv_dict_unref(self.pt_dict.opt_mut());
        if let Some(ref name) = self.pt_name {
            func_unref(Some(name.as_c_str()));
        } else {
            crate::eval::userfunc::func_ptr_unref(self.pt_func.opt_mut());
        }
    }
}

#[derive(Clone)]
pub enum CallbackData {
    None,
    Funcref(std::ffi::CString),
    Partial(crate::globals::GraphPtr<TvPartial>),
    Lua(LuaRef),
    Expr(std::ffi::CString),
}

impl CallbackData {
    pub const NONE: Self = Self::None;

    #[inline]
    pub const fn funcref(funcref: std::ffi::CString) -> Self {
        Self::Funcref(funcref)
    }

    #[inline]
    pub const fn expr(expr: std::ffi::CString) -> Self {
        Self::Expr(expr)
    }

    #[inline]
    pub fn partial(partial: impl Into<crate::globals::GraphPtr<TvPartial>>) -> Self {
        Self::Partial(partial.into())
    }

    #[inline]
    pub const fn lua(luaref: LuaRef) -> Self {
        Self::Lua(luaref)
    }

    #[inline]
    pub fn as_funcref(&self) -> Option<&std::ffi::CStr> {
        match self {
            Self::Funcref(s) | Self::Expr(s) => Some(s.as_c_str()),
            _ => None,
        }
    }

    #[inline]
    pub const fn as_partial(&self) -> crate::globals::GraphPtr<TvPartial> {
        match self {
            Self::Partial(p) => *p,
            _ => crate::globals::GraphPtr::NULL,
        }
    }

    #[inline]
    pub const fn as_luaref(&self) -> LuaRef {
        match self {
            Self::Lua(r) => *r,
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Callback {
    pub data: CallbackData,
}

impl Callback {
    pub const NONE: Self = Self {
        data: CallbackData::NONE,
    };

    #[inline]
    pub const fn new() -> Self {
        Self::NONE
    }

    #[inline]
    pub const fn cb_type(&self) -> i32 {
        match self.data {
            CallbackData::None => K_CALLBACK_NONE_CONST,
            CallbackData::Funcref(_) => K_CALLBACK_FUNCREF,
            CallbackData::Partial(_) => K_CALLBACK_PARTIAL,
            CallbackData::Lua(_) => K_CALLBACK_LUA,
            CallbackData::Expr(_) => K_CALLBACK_EXPR,
        }
    }

    #[inline]
    pub fn raw_cb_type(cb: &Self) -> i32 {
        cb.cb_type()
    }
}

impl Default for Callback {
    #[inline]
    fn default() -> Self {
        Self::NONE
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarType {
    VarUnknown = 0,
    VarNumber = 1,
    VarString = 2,
    VarFunc = 3,
    VarList = 4,
    VarDict = 5,
    VarFloat = 6,
    VarBool = 7,
    VarSpecial = 8,
    VarPartial = 9,
    VarBlob = 10,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub enum TypvalVval {
    #[default]
    None,
    Number(i64),
    Bool(i32),
    Special(i32),
    Float(f64),
    String(crate::strings::ByteSlice),
    List(crate::globals::TvListPtr),
    Dict(crate::globals::GraphPtr<TvDict>),
    Blob(crate::globals::GraphPtr<TvBlob>),
    Partial(crate::globals::GraphPtr<TvPartial>),
}

impl TypvalVval {
    pub const NULL_STRING: Self = Self::String(crate::strings::BYTE_SLICE_NULL);

    #[inline]
    pub fn from_cstr(s: &CStr) -> Self {
        Self::String(crate::strings::ByteSlice::from_cstr(s))
    }

    #[inline]
    pub fn from_opt_cstring(s: Option<std::ffi::CString>) -> Self {
        match s {
            Some(cs) => Self::from_cstring(cs),
            None => Self::NULL_STRING,
        }
    }

    #[inline]
    pub fn from_opt_cstr(s: Option<&CStr>) -> Self {
        Self::String(crate::strings::ByteSlice::from_opt_cstr(s))
    }

    #[inline]
    pub fn from_cstring(s: std::ffi::CString) -> Self {
        let len = s.as_bytes_with_nul().len();
        let ptr = s.into_raw().cast::<u8>().cast_const();
        Self::String(Some(crate::globals::GraphPtr::raw_slice(ptr, len)))
    }

    #[inline]
    pub const fn v_number(self) -> i64 {
        match self {
            Self::Number(n) => n,
            Self::Bool(b) | Self::Special(b) => b as i64,
            _ => 0,
        }
    }

    #[inline]
    pub fn v_number_mut(&mut self) -> &mut i64 {
        if !matches!(self, Self::Number(_)) {
            *self = Self::Number(self.v_number());
        }
        match self {
            Self::Number(n) => n,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn v_bool(self) -> i32 {
        match self {
            Self::Bool(b) | Self::Special(b) => b,
            Self::Number(n) => n as i32,
            _ => 0,
        }
    }

    #[inline]
    pub const fn v_special(self) -> i32 {
        match self {
            Self::Special(s) | Self::Bool(s) => s,
            Self::Number(n) => n as i32,
            _ => 0,
        }
    }

    #[inline]
    pub const fn v_float(self) -> f64 {
        match self {
            Self::Float(f) => f,
            Self::Number(n) => n as f64,
            _ => 0.0,
        }
    }

    #[inline]
    pub fn v_float_mut(&mut self) -> &mut f64 {
        if !matches!(self, Self::Float(_)) {
            *self = Self::Float(self.v_float());
        }
        match self {
            Self::Float(f) => f,
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    pub const fn v_string(self) -> crate::strings::ByteSlice {
        match self {
            Self::String(p) => p,
            _ => crate::strings::BYTE_SLICE_NULL,
        }
    }

    #[inline]
    pub fn v_string_cstr(self) -> Option<&'static CStr> {
        self.v_string().as_opt_cstr()
    }

    #[inline]
    pub fn v_string_cstr_or_empty(self) -> &'static CStr {
        self.v_string().as_cstr()
    }

    #[inline]
    pub fn v_string_bytes(self) -> &'static [u8] {
        self.v_string().as_bytes()
    }

    #[inline]
    pub fn v_string_opt_bytes(self) -> Option<&'static [u8]> {
        self.v_string().map(crate::memory::bytes_until_nul)
    }

    #[inline]
    pub fn v_string_is_empty(self) -> bool {
        self.v_string_bytes().is_empty()
    }

    #[inline]
    pub fn v_string_first_byte(self) -> u8 {
        self.v_string_bytes().first().copied().unwrap_or(0)
    }

    #[inline]
    pub fn v_string_owned(self) -> std::ffi::CString {
        self.v_string_cstr_or_empty().to_owned()
    }

    #[inline(always)]
    pub const fn v_list(self) -> crate::globals::GraphPtr<TvList> {
        match self {
            Self::List(p) => p,
            _ => crate::globals::GraphPtr::NULL,
        }
    }

    #[inline]
    pub fn v_list_ref(self) -> &'static mut TvList {
        match self {
            Self::List(p) => p.borrow_mut(),
            _ => panic!("null list"),
        }
    }

    #[inline]
    pub fn v_list_opt(self) -> Option<&'static mut TvList> {
        match self {
            Self::List(p) => p.opt_mut(),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn v_dict(self) -> crate::globals::GraphPtr<TvDict> {
        match self {
            Self::Dict(p) => p,
            _ => crate::globals::GraphPtr::NULL,
        }
    }

    #[inline]
    pub fn v_dict_ref(self) -> &'static mut TvDict {
        match self {
            Self::Dict(p) => p.borrow_mut(),
            _ => panic!("null dict"),
        }
    }

    #[inline]
    pub fn v_dict_opt(self) -> Option<&'static mut TvDict> {
        match self {
            Self::Dict(p) => p.opt_mut(),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn v_blob(self) -> crate::globals::GraphPtr<TvBlob> {
        match self {
            Self::Blob(p) => p,
            _ => crate::globals::GraphPtr::NULL,
        }
    }

    #[inline]
    pub fn v_blob_ref(self) -> &'static mut TvBlob {
        match self {
            Self::Blob(p) => p.borrow_mut(),
            _ => panic!("null blob"),
        }
    }

    #[inline]
    pub fn v_blob_opt(self) -> Option<&'static mut TvBlob> {
        match self {
            Self::Blob(p) => p.opt_mut(),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn v_partial(self) -> crate::globals::GraphPtr<TvPartial> {
        match self {
            Self::Partial(p) => p,
            _ => crate::globals::GraphPtr::NULL,
        }
    }

    #[inline]
    pub fn v_partial_opt(self) -> Option<&'static mut TvPartial> {
        match self {
            Self::Partial(p) => p.opt_mut(),
            _ => None,
        }
    }

    #[inline]
    pub fn v_any_addr(self) -> usize {
        match self {
            Self::String(p) => p.as_ptr() as usize,
            Self::List(p) => p.as_ptr() as usize,
            Self::Dict(p) => p.as_ptr() as usize,
            Self::Blob(p) => p.as_ptr() as usize,
            Self::Partial(p) => p.as_ptr() as usize,
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypVal {
    pub v_type: VarType,
    pub v_lock: i32,
    pub vval: TypvalVval,
}

impl TypVal {
    #[inline]
    pub const fn new() -> Self {
        tv_initial_value()
    }
}

impl Default for TypVal {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct Listitem {
    pub li_next: crate::globals::ListitemPtr,
    pub li_prev: crate::globals::ListitemPtr,
    pub li_tv: TypVal,
}

impl Listitem {
    pub const fn new() -> Self {
        Self {
            li_next: crate::globals::ListitemPtr::NULL,
            li_prev: crate::globals::ListitemPtr::NULL,
            li_tv: tv_initial_value(),
        }
    }

    #[inline]
    pub fn next_opt(&self) -> Option<&'static mut Listitem> {
        self.li_next.opt_mut()
    }
}

impl Default for Listitem {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Listitem {
    fn drop(&mut self) {
        tv_clear(&mut self.li_tv);
    }
}

#[repr(C)]
pub struct TvList {
    pub lv_first: crate::globals::ListitemPtr,
    pub lv_last: crate::globals::ListitemPtr,
    pub lv_watch: Vec<std::rc::Rc<Listwatch>>,
    pub lv_idx_item: crate::globals::ListitemPtr,
    pub lv_copylist: crate::globals::TvListPtr,
    pub lv_gc_idx: Option<usize>,
    pub lv_refcount: i32,
    pub lv_len: i32,
    pub lv_idx: i32,
    pub lv_copy_id: i32,
    pub lv_lock: i32,
    pub lua_table_ref: i32,
}

impl TvList {
    pub const fn new() -> Self {
        Self {
            lv_first: crate::globals::ListitemPtr::NULL,
            lv_last: crate::globals::ListitemPtr::NULL,
            lv_watch: Vec::new(),
            lv_idx_item: crate::globals::ListitemPtr::NULL,
            lv_copylist: crate::globals::TvListPtr::NULL,
            lv_gc_idx: None,
            lv_refcount: 0,
            lv_len: 0,
            lv_idx: 0,
            lv_copy_id: 0,
            lv_lock: 0,
            lua_table_ref: LUA_NOREF,
        }
    }

    #[inline]
    pub fn first_opt(&self) -> Option<&'static mut Listitem> {
        self.lv_first.opt_mut()
    }

    #[inline]
    pub fn last_opt(&self) -> Option<&'static mut Listitem> {
        self.lv_last.opt_mut()
    }
}

impl Drop for TvList {
    fn drop(&mut self) {
        if let Some(idx) = self.lv_gc_idx.take() {
            gc_unregister_list(idx);
        }
        nlua_clear_ref(&mut self.lua_table_ref);
    }
}

#[repr(C)]
pub struct Dictitem {
    pub di_tv: TypVal,
    pub di_flags: u8,
    pub di_key: Vec<u8>,
}

impl Dictitem {
    pub const fn new() -> Self {
        Self {
            di_tv: tv_initial_value(),
            di_flags: 0,
            di_key: Vec::new(),
        }
    }

    #[inline]
    pub fn key_cstr(&self) -> &CStr {
        CStr::from_bytes_until_nul(&self.di_key).unwrap_or(c"")
    }

    #[inline]
    pub fn key_bytes(&self) -> &[u8] {
        self.key_cstr().to_bytes()
    }
}

impl Drop for Dictitem {
    fn drop(&mut self) {
        if (self.di_flags & DI_FLAGS_ALLOC) != 0 {
            tv_clear(&mut self.di_tv);
        }
    }
}

pub type ScopeDictDictItem = Dictitem;

#[repr(C)]
pub struct TvDict {
    pub dv_lock: i32,
    pub dv_scope: i32,
    pub dv_refcount: i32,
    pub dv_copy_id: i32,
    pub dv_hashtab: crate::hashtab::Hashtab,
    pub dv_copydict: crate::globals::GraphPtr<TvDict>,
    pub dv_gc_idx: Option<usize>,
    pub watchers: Vec<DictWatcher>,
    pub lua_table_ref: i32,
}

impl TvDict {
    pub const fn new() -> Self {
        Self {
            dv_lock: 0,
            dv_scope: 0,
            dv_refcount: 0,
            dv_copy_id: 0,
            dv_hashtab: crate::hashtab::Hashtab::new(),
            dv_copydict: crate::globals::GraphPtr::NULL,
            dv_gc_idx: None,
            watchers: Vec::new(),
            lua_table_ref: LUA_NOREF,
        }
    }
}

impl Drop for TvDict {
    fn drop(&mut self) {
        if let Some(idx) = self.dv_gc_idx.take() {
            gc_unregister_dict(idx);
        }
        nlua_clear_ref(&mut self.lua_table_ref);
    }
}

const ITEM_COMPARE_FAIL: i32 = 999;

pub(crate) const VAR_LOCKED: VarLockStatus = 1;

const VAR_NO_SCOPE: i32 = 0;

pub(crate) const DI_FLAGS_ALLOC: u8 = 16;

pub const TV_TRANSLATE: usize = usize::MAX;
pub const TV_CSTRING: usize = usize::MAX - 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DictListType {
    Keys,
    Values,
    Items,
}

#[repr(C)]
pub struct Listwatch {
    pub lw_item: std::cell::Cell<crate::globals::GraphPtr<Listitem>>,
}

#[repr(C)]
pub struct StaticList10 {
    pub sl_list: TvList,
    pub sl_items: [Listitem; 10],
}

impl StaticList10 {
    pub const fn new() -> Self {
        Self {
            sl_list: TvList::new(),
            sl_items: [const { Listitem::new() }; 10],
        }
    }
}

#[repr(C)]
pub struct DictWatcher {
    pub callback: Callback,
    pub key_pattern: std::ffi::CString,
    pub key_pattern_len: usize,
    pub busy: bool,
    pub needs_free: bool,
}

impl Drop for DictWatcher {
    fn drop(&mut self) {
        callback_free(&mut self.callback);
    }
}

struct Sortinfo {
    item_compare_ic: bool,
    item_compare_lc: bool,
    item_compare_numeric: bool,
    item_compare_numbers: bool,
    item_compare_float: bool,
    item_compare_func: Option<std::ffi::CString>,
    item_compare_partial: crate::globals::GraphPtr<TvPartial>,
    item_compare_selfdict: crate::globals::GraphPtr<TvDict>,
    item_compare_func_err: bool,
    item_compare_keys_ready: bool,
}

impl Sortinfo {
    const fn new() -> Self {
        Self {
            item_compare_ic: false,
            item_compare_lc: false,
            item_compare_numeric: false,
            item_compare_numbers: false,
            item_compare_float: false,
            item_compare_func: None,
            item_compare_partial: crate::globals::GraphPtr::NULL,
            item_compare_selfdict: crate::globals::GraphPtr::NULL,
            item_compare_func_err: false,
            item_compare_keys_ready: false,
        }
    }
}

impl Default for Sortinfo {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum ListSortKey {
    Int(Varnumber),
    Float(f64),
}

impl ListSortKey {
    #[inline]
    fn as_int(self) -> Varnumber {
        match self {
            Self::Int(v) => v,
            Self::Float(_) => 0,
        }
    }

    #[inline]
    fn as_float(self) -> f64 {
        match self {
            Self::Float(v) => v,
            Self::Int(_) => 0.0,
        }
    }
}

#[repr(C)]
struct ListSortItem {
    item: crate::globals::GraphPtr<Listitem>,
    idx: i32,
    key: ListSortKey,
}

impl ListSortItem {
    #[inline]
    fn item_mut(&self) -> &'static mut Listitem {
        self.item.borrow_mut()
    }
}

pub(crate) struct TypvalModuleState {
    pub(crate) tv_in_free_unref_items: bool,
    tv_equal_recurse_limit: i32,
    dict_numbuf: [u8; NUMBUFLEN],
    mybuf_chk: [u8; NUMBUFLEN],
    mybuf: [u8; NUMBUFLEN],
}

impl TypvalModuleState {
    const fn new() -> Self {
        Self {
            tv_in_free_unref_items: false,
            tv_equal_recurse_limit: 0,
            dict_numbuf: [0; NUMBUFLEN],
            mybuf_chk: [0; NUMBUFLEN],
            mybuf: [0; NUMBUFLEN],
        }
    }
}

static TV_STATE: crate::globals::GlobalState<TypvalModuleState> =
    crate::globals::GlobalState::new(TypvalModuleState::new());

#[inline]
pub(crate) fn tv_st() -> crate::globals::GraphPtr<TypvalModuleState> {
    TV_STATE.handle()
}

static E_VARIABLE_NESTED_TOO_DEEP_FOR_UNLOCK: &CStr =
    c"E743: Variable nested too deep for (un)lock";
static E_USING_INVALID_VALUE_AS_STRING: &CStr = c"E908: Using an invalid value as a String";
static E_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1174: String required for argument %d";
static E_NON_EMPTY_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1175: Non-empty string required for argument %d";
static E_DICT_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1206: Dictionary required for argument %d";
static E_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1210: Number required for argument %d";
static E_LIST_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1211: List required for argument %d";
static E_BOOL_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1212: Bool required for argument %d";
static E_FLOAT_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1219: Float or Number required for argument %d";
static E_STRING_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1220: String or Number required for argument %d";
static E_STRING_OR_LIST_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1222: String or List required for argument %d";
static E_LIST_DICT_BLOB_OR_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1225: List, Dictionary, Blob or String required for argument %d";
static E_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1226: List or Blob required for argument %d";
static E_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1238: Blob required for argument %d";
static E_STRING_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1252: String, List or Blob required for argument %d";
static E_STRING_OR_FUNCTION_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1256: String or function required for argument %d";
static E_NON_NULL_DICT_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1297: Non-NULL Dictionary required for argument %d";

#[inline]
pub fn nlua_clear_ref(r: &mut LuaRef) {
    if *r != LUA_NOREF && *r != 0 {
        api_free_luaref(*r);
        *r = LUA_NOREF;
    }
}

#[inline]
pub fn tv_dict_hi2di(hi: &Hashitem) -> &'static mut Dictitem {
    hi.hi_di.borrow_mut()
}

#[inline]
pub fn tv_list_set_ret(tv: &mut TypVal, l: Option<&mut TvList>) {
    tv_list_set_ret_inline(tv, l)
}

#[inline]
pub fn tv_take_string(tv: &mut TypVal) -> Option<std::ffi::CString> {
    let p = tv.vval.v_string();
    tv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    if p.is_null() {
        None
    } else {
        Some(p.to_cstring())
    }
}

#[inline]
pub fn tv_set_string(tv: &mut TypVal, s: Option<&CStr>) {
    tv.v_type = VarType::VarString;
    tv.vval = crate::eval::typval::TypvalVval::from_opt_cstring(s.map(CStr::to_owned));
}

#[inline]
pub fn tv_set_bytes(tv: &mut TypVal, bytes: &[u8]) {
    tv.v_type = VarType::VarString;
    tv.vval = crate::eval::typval::TypvalVval::from_cstring(crate::memory::bytes_to_cstring(bytes));
}

#[inline]
pub fn tv_set_string_len(tv: &mut TypVal, s: Option<&[u8]>) {
    tv.v_type = VarType::VarString;
    tv.vval =
        crate::eval::typval::TypvalVval::from_opt_cstring(s.map(crate::memory::bytes_to_cstring));
}

#[inline]
pub fn tv_blob_get(b: &TvBlob, idx: i32) -> u8 {
    tv_blob_get_inline(b, idx)
}

#[inline]
pub const fn tv_is_func(tv: &TypVal) -> bool {
    tv_is_func_inline(tv)
}

#[inline]
fn tv_list_ref_inline(l: Option<&mut TvList>) {
    if let Some(l) = l {
        l.lv_refcount += 1;
    }
}

#[inline]
fn tv_list_set_ret_inline(tv: &mut TypVal, l: Option<&mut TvList>) {
    let l_ptr = l.map_or(ptr::null_mut(), |l_ref| {
        l_ref.lv_refcount += 1;
        ptr::from_mut(l_ref)
    });
    tv.v_type = VarType::VarList;
    tv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::new(l_ptr));
}

#[inline]
fn tv_list_locked_inline(l: Option<&TvList>) -> VarLockStatus {
    l.map_or(VAR_FIXED, |l| l.lv_lock)
}

#[inline]
pub fn tv_list_ref(l: Option<&mut TvList>) {
    if let Some(l) = l {
        l.lv_refcount += 1;
    }
}

pub struct ListMutIter(crate::globals::GraphPtr<Listitem>);

impl Iterator for ListMutIter {
    type Item = &'static mut Listitem;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.li_next;
        Some(cur)
    }
}

pub struct ListPtrIter(crate::globals::GraphPtr<Listitem>);

impl Iterator for ListPtrIter {
    type Item = &'static mut Listitem;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cur = self.0.opt_mut()?;
        self.0 = cur.li_next;
        Some(cur)
    }
}

#[inline]
pub fn list_iter_mut(start: Option<&mut Listitem>) -> ListMutIter {
    ListMutIter(start.into())
}

#[inline]
pub fn list_iter_ptr(start: Option<&mut Listitem>) -> ListPtrIter {
    ListPtrIter(start.into())
}

#[inline]
pub fn tv_list_set_lock(l: Option<&mut TvList>, lock: VarLockStatus) {
    tv_list_set_lock_inline(l, lock)
}

#[inline]
pub fn tv_blob_len(b: Option<&TvBlob>) -> i32 {
    b.map_or(0, |b| b.bv_ga.len() as i32)
}

#[inline]
fn tv_list_set_lock_inline(l: Option<&mut TvList>, lock: VarLockStatus) {
    if l.is_none() {
        debug_assert_eq!(lock, VAR_FIXED);
        return;
    }
    l.unwrap().lv_lock = lock;
}

#[inline]
pub fn tv_list_len(l: Option<&TvList>) -> i32 {
    tv_list_len_inline(l)
}

#[inline]
pub fn tv_dict_len(d: Option<&TvDict>) -> i64 {
    d.map_or(0, |d| d.dv_hashtab.ht_used as i64)
}

#[inline]
pub fn tv_list_first(l: Option<&TvList>) -> Option<&'static mut Listitem> {
    tv_list_first_inline(l)
}

#[inline]
pub fn tv_list_last(l: Option<&TvList>) -> Option<&'static mut Listitem> {
    tv_list_last_inline(l)
}

pub struct TvListIter<'a> {
    cur: crate::globals::ListitemPtr,
    last: crate::globals::ListitemPtr,
    _marker: std::marker::PhantomData<&'a Listitem>,
}

impl<'a> Iterator for TvListIter<'a> {
    type Item = &'a Listitem;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.cur.opt()?;
        if self.cur == self.last {
            self.cur = crate::globals::ListitemPtr::NULL;
        } else {
            self.cur = item.li_next;
        }
        Some(item)
    }
}

#[inline]
pub fn tv_list_iter(l: Option<&TvList>) -> TvListIter<'_> {
    let cur = l.map_or(crate::globals::ListitemPtr::NULL, |l| l.lv_first);
    let last = l.map_or(crate::globals::ListitemPtr::NULL, |l| l.lv_last);
    TvListIter {
        cur,
        last,
        _marker: std::marker::PhantomData,
    }
}

#[inline]
fn tv_list_len_inline(l: Option<&TvList>) -> i32 {
    l.map_or(0, |l| l.lv_len)
}

#[inline]
fn tv_list_uidx_inline(l: Option<&TvList>, mut n: i32) -> i32 {
    let len = tv_list_len_inline(l);
    if n < 0 {
        n += len;
    }
    if n < 0 || n >= len { -1 } else { n }
}

#[inline]
fn tv_list_first_inline(l: Option<&TvList>) -> Option<&'static mut Listitem> {
    l.and_then(|l| l.lv_first.opt_mut())
}

#[inline]
fn tv_list_last_inline(l: Option<&TvList>) -> Option<&'static mut Listitem> {
    l.and_then(|l| l.lv_last.opt_mut())
}

#[inline]
fn tv_dict_set_ret_inline(tv: &mut TypVal, d: &mut TvDict) {
    d.dv_refcount += 1;
    tv.v_type = VarType::VarDict;
    tv.vval = crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::new(d));
}

#[inline]
fn tv_dict_len_inline(d: Option<&TvDict>) -> isize {
    d.map_or(0, |d| d.dv_hashtab.ht_used as isize)
}

#[inline]
fn tv_dict_is_watched_inline(d: Option<&TvDict>) -> bool {
    d.is_some() && !d.unwrap().watchers.is_empty()
}

#[inline]
fn tv_blob_set_ret_inline(tv: &mut TypVal, b: &mut TvBlob) {
    b.bv_refcount += 1;
    tv.v_type = VarType::VarBlob;
    tv.vval = crate::eval::typval::TypvalVval::Blob(crate::globals::GraphPtr::new(b));
}

#[inline]
fn tv_blob_len_inline(b: Option<&TvBlob>) -> i32 {
    b.map_or(0, |b| b.bv_ga.len() as i32)
}

#[inline]
fn tv_blob_get_inline(b: &TvBlob, idx: i32) -> u8 {
    (&b.bv_ga)[idx as usize]
}

#[inline]
fn tv_blob_set_inline(blob: &mut TvBlob, idx: i32, c: u8) {
    (&mut blob.bv_ga)[idx as usize] = c;
}

#[inline]
const fn tv_is_func_inline(tv: &TypVal) -> bool {
    matches!(tv.v_type, VarType::VarFunc | VarType::VarPartial)
}

#[inline]
pub const fn tv_initial_value() -> TypVal {
    TypVal {
        v_type: VarType::VarUnknown,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval::Number(0),
    }
}

/// Allocate a list item.
fn tv_list_item_alloc() -> &'static mut Listitem {
    Box::leak(Box::new(Listitem {
        li_next: crate::globals::ListitemPtr::NULL,
        li_prev: crate::globals::ListitemPtr::NULL,
        li_tv: TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        },
    }))
}

/// Remove a list item from a List and free it.
pub fn tv_list_item_remove(
    l: &mut TvList,
    item_in: &mut Listitem,
) -> Option<&'static mut Listitem> {
    let item = crate::globals::GraphPtr::from(item_in);
    let next_item = item.li_next;

    tv_list_drop_items(l, item.borrow_mut(), item.borrow_mut());

    crate::globals::GraphPtr::of(item.as_ptr()).drop_box();

    next_item.opt_mut()
}

/// Add a watcher to a list.
pub fn tv_list_watch_add(l: &mut TvList, lw: std::rc::Rc<Listwatch>) {
    l.lv_watch.push(lw);
}

/// Remove a watcher from a list.
pub fn tv_list_watch_remove(l: &mut TvList, lwrem: &std::rc::Rc<Listwatch>) {
    if let Some(pos) = l
        .lv_watch
        .iter()
        .position(|lw| std::rc::Rc::ptr_eq(lw, lwrem))
    {
        l.lv_watch.remove(pos);
    }
}

/// Advance watchers to the next item.
fn tv_list_watch_fix(l: &mut TvList, item: &Listitem) {
    for lw in &l.lv_watch {
        if lw.lw_item.get() == item {
            lw.lw_item.set(item.li_next);
        }
    }
}

/// Allocate an empty list.
#[unsafe(no_mangle)]
pub extern "C" fn tv_list_alloc(_len: isize) -> &'static mut TvList {
    let list = Box::leak(Box::new(TvList {
        lv_first: crate::globals::ListitemPtr::NULL,
        lv_last: crate::globals::ListitemPtr::NULL,
        lv_watch: Vec::new(),
        lv_idx_item: crate::globals::ListitemPtr::NULL,
        lv_copylist: crate::globals::TvListPtr::NULL,
        lv_gc_idx: None,
        lv_refcount: 0,
        lv_len: 0,
        lv_idx: 0,
        lv_copy_id: 0,
        lv_lock: 0,
        lua_table_ref: LUA_NOREF,
    }));

    gc_register_list(list);
    list
}

/// Initialize a static list with 10 items.
pub fn tv_list_init_static10(sl: &mut StaticList10) {
    const SL_SIZE: usize = 10;
    *sl = StaticList10::new();
    let l = &mut sl.sl_list;
    let items = (&raw mut sl.sl_items).cast::<Listitem>();

    l.lv_first = crate::globals::ListitemPtr::new(items);
    l.lv_last = crate::globals::ListitemPtr::new(items.wrapping_add(SL_SIZE - 1));
    l.lv_refcount = DO_NOT_FREE_CNT;
    tv_list_set_lock_inline(Some(&mut *l), VAR_FIXED);
    l.lv_len = 10;

    let items_slice = crate::globals::GraphPtr::raw_slice_mut(items, SL_SIZE);
    items_slice[0].li_prev = crate::globals::ListitemPtr::NULL;
    items_slice[0].li_next = crate::globals::ListitemPtr::new(items.wrapping_add(1));
    items_slice[SL_SIZE - 1].li_prev =
        crate::globals::ListitemPtr::new(items.wrapping_add(SL_SIZE - 2));
    items_slice[SL_SIZE - 1].li_next = crate::globals::ListitemPtr::NULL;

    for (i, li_ref) in items_slice.iter_mut().enumerate().take(SL_SIZE - 1).skip(1) {
        let li = items.wrapping_add(i);
        li_ref.li_prev = crate::globals::ListitemPtr::new(li.wrapping_sub(1));
        li_ref.li_next = crate::globals::ListitemPtr::new(li.wrapping_add(1));
    }
}

/// Initialize static list with undefined number of elements.
pub fn tv_list_init_static(l: &mut TvList) {
    *l = TvList::new();
    l.lv_refcount = DO_NOT_FREE_CNT;
}

/// Free items contained in a list.
pub fn tv_list_free_contents(l: &mut TvList) {
    let mut item = l.lv_first;
    while !item.is_null() {
        l.lv_first = item.li_next;
        crate::globals::GraphPtr::of(item.as_ptr()).drop_box();
        item = l.lv_first;
    }
    l.lv_len = 0;
    l.lv_idx_item = crate::globals::ListitemPtr::NULL;
    l.lv_last = crate::globals::ListitemPtr::NULL;
    debug_assert!(l.lv_watch.is_empty());
}

/// Free a list, including all items it points to.
#[unsafe(no_mangle)]
pub extern "C" fn tv_list_free(l: Option<&mut TvList>) {
    if tv_st().tv_in_free_unref_items {
        return;
    }
    let Some(l_ref) = l else {
        return;
    };
    let l_ptr = crate::globals::GraphPtr::from(l_ref);

    tv_list_free_contents(l_ptr.borrow_mut());

    crate::globals::GraphPtr::of(l_ptr.as_ptr()).drop_box();
}

/// Unreference a list.
pub fn tv_list_unref(l: Option<&mut TvList>) {
    if let Some(l_ref) = l {
        l_ref.lv_refcount -= 1;
        if l_ref.lv_refcount <= 0 {
            tv_list_free(Some(l_ref));
        }
    }
}

/// Remove items "item" to "item2" from list "l".
pub fn tv_list_drop_items(l: &mut TvList, item_in: &mut Listitem, item2_in: &mut Listitem) {
    let item = crate::globals::GraphPtr::<Listitem>::from_ref(item_in);
    let item2 = crate::globals::GraphPtr::<Listitem>::from_ref(item2_in);
    let mut end_next = item2.li_next;
    let mut ip = item;
    while ip != end_next {
        l.lv_len -= 1;

        tv_list_watch_fix(l, &ip);

        ip = ip.li_next;
    }

    let mut item_prev = item.li_prev;
    if end_next.is_null() {
        l.lv_last = item_prev;
    } else {
        end_next.li_prev = item_prev;
    }
    if item_prev.is_null() {
        l.lv_first = end_next;
    } else {
        item_prev.li_next = end_next;
    }
    l.lv_idx_item = crate::globals::ListitemPtr::NULL;
}

/// Like tv_list_drop_items, but also frees all removed items.
pub fn tv_list_remove_items(l: &mut TvList, item_in: &mut Listitem, item2_in: &mut Listitem) {
    let item = crate::globals::GraphPtr::from(item_in);
    let item2 = crate::globals::GraphPtr::from(item2_in);
    tv_list_drop_items(l, item.borrow_mut(), item2.borrow_mut());
    let mut li = item;
    loop {
        let nli = li.li_next;
        crate::globals::GraphPtr::of(li.as_ptr()).drop_box();
        if li == item2 {
            break;
        }
        li = nli;
    }
}

/// Move items "item" to "item2" from list "l" to the end of the list "tgt_l".
pub fn tv_list_move_items(
    l: &mut TvList,
    item_in: &mut Listitem,
    item2_in: &mut Listitem,
    tgt_l: &mut TvList,
    cnt: i32,
) {
    let mut item = crate::globals::GraphPtr::<Listitem>::from_ref(item_in);
    let mut item2 = crate::globals::GraphPtr::<Listitem>::from_ref(item2_in);
    tv_list_drop_items(&mut *l, item.borrow_mut(), item2.borrow_mut());
    item.li_prev = tgt_l.lv_last;
    item2.li_next = crate::globals::ListitemPtr::NULL;
    if let Some(last_ref) = tgt_l.lv_last.opt_mut() {
        last_ref.li_next = item;
    } else {
        tgt_l.lv_first = item;
    }
    tgt_l.lv_last = item2;
    tgt_l.lv_len += cnt;
}

/// Insert list item.
pub fn tv_list_insert(l: &mut TvList, ni: &mut Listitem, item_opt: Option<&mut Listitem>) {
    let Some(item_ref) = item_opt else {
        tv_list_append(l, ni);
        return;
    };
    let mut item = crate::globals::GraphPtr::<Listitem>::from_ref(item_ref);
    let ni_ptr = crate::globals::ListitemPtr::from(&mut *ni);
    ni.li_prev = item.li_prev;
    ni.li_next = item;
    if let Some(prev_ref) = item.li_prev.opt_mut() {
        prev_ref.li_next = ni_ptr;
        l.lv_idx_item = crate::globals::ListitemPtr::NULL;
    } else {
        l.lv_first = ni_ptr;
        l.lv_idx += 1;
    }

    item.li_prev = ni_ptr;
    l.lv_len += 1;
}

/// Insert Vimscript value into a list.
pub fn tv_list_insert_tv(l: &mut TvList, tv: &mut TypVal, item: Option<&mut Listitem>) {
    let ni = tv_list_item_alloc();
    tv_copy(&*tv, &mut ni.li_tv);
    tv_list_insert(l, &mut *ni, item);
}

/// Append item to the end of list.
pub fn tv_list_append(l: &mut TvList, item: &mut Listitem) {
    let item_ptr = crate::globals::ListitemPtr::from(&mut *item);
    if let Some(last_ref) = l.lv_last.opt_mut() {
        last_ref.li_next = item_ptr;
        item.li_prev = l.lv_last;
        l.lv_last = item_ptr;
    } else {
        l.lv_first = item_ptr;
        l.lv_last = item_ptr;
        item.li_prev = crate::globals::ListitemPtr::NULL;
    }
    l.lv_len += 1;
    item.li_next = crate::globals::ListitemPtr::NULL;
}

/// Append Vimscript value to the end of list.
pub fn tv_list_append_tv(l: &mut TvList, tv: &mut TypVal) {
    let li = tv_list_item_alloc();
    tv_copy(&*tv, &mut li.li_tv);
    tv_list_append(l, &mut *li);
}

/// Like tv_list_append_tv(), but tv is moved to a list.
pub fn tv_list_append_owned_tv(l: &mut TvList, tv: TypVal) -> &'static mut TypVal {
    let li = tv_list_item_alloc();
    li.li_tv = tv;
    tv_list_append(l, li);
    &mut li.li_tv
}

/// Append a list to a list as one item.
pub fn tv_list_append_list(l: &mut TvList, itemlist: Option<&mut TvList>) {
    let itemlist_ptr = itemlist.map_or(ptr::null_mut(), |il| {
        il.lv_refcount += 1;
        ptr::from_mut(il)
    });
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarList,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::List(crate::globals::GraphPtr::new(itemlist_ptr)),
        },
    );
}

/// Append a dictionary to a list.
pub fn tv_list_append_dict(l: &mut TvList, dict: Option<&mut TvDict>) {
    let dict_ptr = dict.map_or(ptr::null_mut(), |d| {
        d.dv_refcount += 1;
        ptr::from_mut(d)
    });
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(crate::globals::GraphPtr::new(dict_ptr)),
        },
    );
}

/// Make a copy of "str" and append it as an item to list "l".
#[inline]
pub fn tv_list_append_bytes(l: &mut TvList, bytes: &[u8]) {
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarString,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::from_cstring(crate::memory::bytes_to_cstring(bytes)),
        },
    );
}

pub fn tv_list_append_string(l: &mut TvList, str_: Option<&CStr>, len: isize) {
    let v_string = match str_ {
        None => None,
        Some(s) if len >= 0 => Some(crate::memory::bytes_to_cstring(
            crate::strings::AsByteSlice::slice_len(s.as_ptr(), len as usize),
        )),
        Some(s) => Some(s.to_owned()),
    };
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarString,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::from_opt_cstring(v_string),
        },
    );
}

/// Append number to the list.
pub fn tv_list_append_number(l: &mut TvList, n: Varnumber) {
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarNumber,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(n),
        },
    );
}

/// Make a copy of list.
pub fn tv_list_copy(
    conv: Option<&Vimconv>,
    mut orig: Option<&mut TvList>,
    deep: bool,
    copy_id: i32,
) -> Option<&'static mut TvList> {
    orig.as_ref()?;

    let copy = tv_list_alloc(tv_list_len_inline(orig.as_deref()) as isize);
    tv_list_ref_inline(Some(&mut *copy));
    if copy_id != 0 {
        orig.as_deref_mut().unwrap().lv_copy_id = copy_id;
        orig.as_deref_mut().unwrap().lv_copylist = crate::globals::TvListPtr::from(&mut *copy);
    }
    for item in crate::eval::typval::list_iter_mut(orig.unwrap().first_opt()) {
        if g_st().got_int {
            break;
        }
        let ni = tv_list_item_alloc();
        if deep {
            if !var_item_copy(conv, &mut item.li_tv, &mut ni.li_tv, deep, copy_id) {
                crate::globals::GraphPtr::of(ni).drop_box();
                tv_list_unref(Some(copy));
                return None;
            }
        } else {
            tv_copy(&item.li_tv, &mut ni.li_tv);
        }
        tv_list_append(&mut *copy, &mut *ni);
    }

    Some(copy)
}

/// Get the list item in "l" with index "n1".
pub fn tv_list_check_range_index_one(
    l: Option<&mut TvList>,
    n1: &mut i32,
    quiet: bool,
) -> Option<&'static mut Listitem> {
    let li = tv_list_find_index(l, n1);
    if li.is_some() {
        return li;
    }

    if !quiet {
        semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(*n1));
    }
    None
}

/// Check that "n2" can be used as the second index in a range of list "l".
pub fn tv_list_check_range_index_two(
    mut l: Option<&mut TvList>,
    n1: &mut i32,
    li1: Option<&Listitem>,
    n2: &mut i32,
    quiet: bool,
) -> bool {
    if *n2 < 0 {
        let Some(ni) = tv_list_find(l.as_deref_mut(), *n2) else {
            if !quiet {
                semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(*n2));
            }
            return false;
        };
        *n2 = tv_list_idx_of_item(l.as_deref(), Some(ni));
    }

    if *n1 < 0 {
        *n1 = tv_list_idx_of_item(l.as_deref(), li1);
    }
    if *n2 < *n1 {
        if !quiet {
            semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(*n2));
        }
        return false;
    }
    true
}

/// Assign values from list "src" into a range of "dest".
pub fn tv_list_assign_range(
    dest: &mut TvList,
    src: Option<&TvList>,
    idx1_arg: i32,
    idx2: i32,
    empty_idx2: bool,
    op_opt: Option<&CStr>,
    varname: &CStr,
) -> bool {
    let op = crate::strings::ByteSlice::from_opt_cstr(op_opt);
    let mut idx1 = idx1_arg;
    let first_li = crate::globals::GraphPtr::from(tv_list_find_index(Some(&mut *dest), &mut idx1));
    let mut src_li: crate::globals::GraphPtr<Listitem>;

    let mut idx = idx1;
    let mut dest_li = first_li;
    src_li = crate::globals::GraphPtr::from(tv_list_first_inline(src));
    while !src_li.is_null() && !dest_li.is_null() {
        if value_check_lock(dest_li.li_tv.v_lock, Some(varname), TV_CSTRING) {
            return false;
        }
        src_li = crate::globals::GraphPtr::<Listitem>::of(src_li.li_next);
        if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
            break;
        }
        dest_li = dest_li.li_next;
        idx += 1;
    }

    idx = idx1;
    dest_li = crate::globals::GraphPtr::of(first_li);
    src_li = crate::globals::GraphPtr::from(tv_list_first_inline(src));
    while !src_li.is_null() {
        debug_assert!(!dest_li.is_null());
        if !op.is_null() && op.first_byte() != b'=' {
            eexe_mod_op(
                &mut (dest_li.borrow_mut().li_tv),
                &(src_li.borrow().li_tv),
                op.as_cstr(),
            );
        } else {
            tv_clear(&mut (dest_li.borrow_mut().li_tv));
            tv_copy(&(src_li.borrow().li_tv), &mut (dest_li.borrow_mut().li_tv));
        }
        src_li = crate::globals::GraphPtr::<Listitem>::of(src_li.li_next);
        if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
            break;
        }
        if dest_li.li_next.is_null() {
            tv_list_append_number(dest, 0);
            dest_li = crate::globals::GraphPtr::of(
                tv_list_last_inline(Some(dest)).map_or(ptr::null_mut(), ptr::from_mut),
            );
        } else {
            dest_li = dest_li.li_next;
        }
        idx += 1;
    }
    if !src_li.is_null() {
        emsg(tr(c"E710: List value has more items than target"));
        return false;
    }
    if if empty_idx2 {
        !dest_li.is_null() && !dest_li.li_next.is_null()
    } else {
        idx != idx2
    } {
        emsg(tr(c"E711: List value has not enough items"));
        return false;
    }
    true
}

/// Flatten up to "maxitems" in "list", starting at "first" to depth "maxdepth".
pub fn tv_list_flatten(
    list: &mut TvList,
    first_opt: Option<&mut Listitem>,
    maxitems: i64,
    maxdepth: i64,
) {
    if maxdepth == 0 {
        return;
    }

    let first = crate::globals::GraphPtr::from(first_opt);
    let mut item = if first.is_null() {
        list.lv_first
    } else {
        first
    };
    let mut done: i64 = 0;

    while !item.is_null() && done < maxitems {
        let next = item.borrow().li_next;

        fast_breakcheck();
        if g_st().got_int {
            return;
        }
        if item.borrow().li_tv.v_type == VarType::VarList {
            let itemlist = item.borrow().li_tv.vval.v_list();
            let item_ptr = item.as_ptr();

            tv_list_drop_items(
                list,
                crate::globals::GraphPtr::of(item_ptr).borrow_mut(),
                crate::globals::GraphPtr::of(item_ptr).borrow_mut(),
            );
            tv_list_extend(list, itemlist.opt(), next.opt_mut());

            if maxdepth > 0 {
                let sub_first = item
                    .borrow()
                    .li_prev
                    .opt()
                    .map_or_else(|| list.lv_first, |prev| prev.li_next);
                let sub_len = itemlist.opt().map_or(0, |l| i64::from(l.lv_len));
                tv_list_flatten(list, sub_first.opt_mut(), sub_len, maxdepth - 1);
            }

            crate::globals::GraphPtr::of(item_ptr).drop_box();
        }

        done += 1;
        item = next;
    }
}

/// "items(blob)" function.
fn tv_blob2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let blob = argvars[0].vval.v_blob_opt();

    tv_list_alloc_ret(&mut *rettv, tv_blob_len_inline(blob.as_deref()) as isize);
    let Some(blob) = blob else {
        return;
    };

    for i in 0..tv_blob_len_inline(Some(blob)) {
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&mut *l2));
        tv_list_append_number(&mut *l2, i64::from(i));
        tv_list_append_number(&mut *l2, i64::from(tv_blob_get_inline(blob, i)));
    }
}

/// "items(dict)" function.
fn tv_dict2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    tv_dict2list(argvars, rettv, DictListType::Items);
}

/// "items(list)" function.
fn tv_list2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let l = argvars[0].vval.v_list_opt();

    tv_list_alloc_ret(&mut *rettv, tv_list_len_inline(l.as_deref()) as isize);
    let Some(l) = l else {
        return;
    };

    for (idx, li) in (0 as Varnumber..).zip(crate::eval::typval::list_iter_mut(l.first_opt())) {
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&mut *l2));
        tv_list_append_number(&mut *l2, idx);
        tv_list_append_tv(&mut *l2, &mut li.li_tv);
    }
}

/// "items(string)" function.
fn tv_string2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let mut p = argvars[0].vval.v_string();

    tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_MAY_KNOW_CONST);
    if p.is_null() {
        return;
    }

    let mut idx: Varnumber = 0;
    while !p.is_empty() {
        let len = utfc_ptr2len(p);
        if len == 0 {
            break;
        }
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&mut *l2));
        tv_list_append_number(&mut *l2, idx);
        tv_list_append_string(&mut *l2, Some(p.as_cstr()), len as isize);
        p = p.wrapping_add(len as usize);
        idx += 1;
    }
}

/// Extend first list with the second.
pub fn tv_list_extend(l1: &mut TvList, l2: Option<&TvList>, bef_opt: Option<&mut Listitem>) {
    let mut todo = tv_list_len_inline(l2);
    if todo == 0 {
        return;
    }

    let bef = crate::globals::GraphPtr::from(bef_opt);
    let befbef = if bef.is_null() {
        ptr::null_mut()
    } else {
        bef.li_prev.as_ptr()
    };
    let saved_next = if befbef.is_null() {
        crate::globals::ListitemPtr::NULL
    } else {
        crate::globals::GraphPtr::of(befbef).borrow().li_next
    };
    let mut item = crate::globals::GraphPtr::from(tv_list_first_inline(l2));
    while let Some(item_ref) = item.opt_mut()
        && todo > 0
    {
        todo -= 1;
        tv_list_insert_tv(l1, &mut item_ref.li_tv, bef.opt_mut());
        item = if item == befbef {
            saved_next
        } else {
            item_ref.li_next
        };
    }
}

/// Concatenate lists into a new list.
pub fn tv_list_concat(
    mut l1: Option<&mut TvList>,
    mut l2: Option<&mut TvList>,
    tv: &mut TypVal,
) -> bool {
    tv.v_type = VarType::VarList;
    tv.v_lock = VAR_UNLOCKED;
    let l = if l1.is_none() && l2.is_none() {
        None
    } else if l1.is_none() {
        tv_list_copy(None, l2.as_deref_mut(), false, 0)
    } else {
        let mut copy = tv_list_copy(None, l1.as_deref_mut(), false, 0);
        if let Some(ref mut copy_ref) = copy
            && l2.is_some()
        {
            tv_list_extend(copy_ref, l2.as_deref(), None);
        }
        copy
    };
    if l.is_none() && !(l1.is_none() && l2.is_none()) {
        return false;
    }

    tv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::from(l));
    true
}

fn tv_list_slice(ol: Option<&mut TvList>, mut n1: Varnumber, n2: Varnumber) -> &'static mut TvList {
    let l = tv_list_alloc((n2 - n1 + 1) as isize);
    let mut item = tv_list_find_opt(ol, n1 as i32);
    while n1 <= n2 {
        let cur = item.unwrap();
        tv_list_append_tv(&mut *l, &mut cur.li_tv);
        item = cur.next_opt();
        n1 += 1;
    }
    l
}

pub fn tv_list_slice_or_index(
    range: bool,
    n1_arg: Varnumber,
    n2_arg: Varnumber,
    exclusive: bool,
    rettv: &mut TypVal,
    verbose: bool,
) -> bool {
    let len = i64::from(tv_list_len_inline(rettv.vval.v_list_opt().as_deref()));
    let mut n1 = n1_arg;
    let mut n2 = n2_arg;

    if n1 < 0 {
        n1 += len;
    }
    if n1 < 0 || n1 >= len {
        if !range {
            if verbose {
                semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), n1_arg);
            }
            return false;
        }
        n1 = len;
    }
    if range {
        if n2 < 0 {
            n2 += len;
        } else if n2 >= len {
            n2 = len - if exclusive { 0 } else { 1 };
        }
        if exclusive {
            n2 -= 1;
        }
        if n2 < 0 || n2 + 1 < n1 {
            n2 = -1;
        }
        let l = tv_list_slice(rettv.vval.v_list_opt(), n1, n2);
        tv_clear(&mut *rettv);
        tv_list_set_ret_inline(rettv, Some(l));
    } else {
        let found = tv_list_find_opt(rettv.vval.v_list_opt(), n1 as i32).unwrap();
        let mut var1 = tv_initial_value();
        tv_copy(&found.li_tv, &mut var1);
        tv_clear(&mut *rettv);
        *rettv = var1;
    }
    true
}

fn list_join_inner(
    gap: &mut Vec<u8>,
    l: &mut TvList,
    sep: &CStr,
    join_vec: &mut Vec<std::ffi::CString>,
) -> bool {
    let mut sumlen: usize = 0;
    let mut first = true;

    for item in crate::eval::typval::list_iter_mut(l.first_opt()) {
        if g_st().got_int {
            break;
        }
        let mut size: usize = 0;
        let s = encode_tv2echo(&mut item.li_tv, Some(&mut size));

        sumlen += size;
        join_vec.push(s);

        line_breakcheck();
    }

    let sep_bytes = sep.to_bytes();
    let seplen = sep_bytes.len();
    if join_vec.len() >= 2 {
        sumlen += seplen * (join_vec.len() - 1);
    }
    gap.reserve(sumlen + 2);

    for s in join_vec.iter() {
        if g_st().got_int {
            break;
        }
        if first {
            first = false;
        } else if seplen > 0 {
            gap.extend_from_slice(sep_bytes);
        }
        let bytes = s.as_bytes();
        if !bytes.is_empty() {
            gap.extend_from_slice(bytes);
        }
        line_breakcheck();
    }

    true
}

/// Join list into a string using given separator.
pub fn tv_list_join(gap: &mut Vec<u8>, l: Option<&mut TvList>, sep: &CStr) -> bool {
    let len = tv_list_len_inline(l.as_deref());
    if len == 0 {
        return true;
    }

    let mut join_vec: Vec<std::ffi::CString> = Vec::with_capacity(len as usize);
    list_join_inner(gap, l.unwrap(), sep, &mut join_vec)
}

/// "join()" function.
pub fn f_join(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type != VarType::VarList {
        emsg(tr(E_LISTREQ));
        return;
    }
    let sep = if argvars[1].v_type == VarType::VarUnknown {
        Some(c" ")
    } else {
        tv_get_string_chk(&argvars[1])
    };

    rettv.v_type = VarType::VarString;

    if let Some(sep) = sep {
        let mut out: Vec<u8> = Vec::with_capacity(80);
        tv_list_join(&mut out, argvars[0].vval.v_list_opt(), sep);
        tv_set_string_len(rettv, Some(&out));
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    }
}

/// "list2str()" function.
pub fn f_list2str(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;
    if argvars[0].v_type != VarType::VarList {
        emsg(tr(E_INVARG));
        return;
    }

    let Some(l) = argvars[0].vval.v_list_opt() else {
        return;
    };

    let mut out: Vec<u8> = Vec::with_capacity(80);
    let mut buf: [u8; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];

    for li in crate::eval::typval::list_iter_mut(l.first_opt()) {
        let n = tv_get_number(&li.li_tv);
        let buflen = utf_char2bytes(n as i32, &mut buf) as usize;
        buf[buflen] = 0;
        out.extend_from_slice(&buf[..buflen]);
    }

    tv_set_string_len(rettv, Some(&out));
}

/// "remove({list})" function.
pub fn tv_list_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    let l = argvars[0].vval.v_list_opt();
    if value_check_lock(
        tv_list_locked_inline(l.as_deref()),
        Some(arg_errmsg),
        TV_TRANSLATE,
    ) {
        return;
    }
    let Some(l) = l else {
        return;
    };

    let mut error = false;
    let idx = tv_get_number_chk(&argvars[1], Some(&mut error));

    if error {
        return;
    }
    let Some(item) = tv_list_find_opt(Some(&mut *l), idx as i32) else {
        semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), idx);
        return;
    };
    if argvars[2].v_type == VarType::VarUnknown {
        let mut item_ptr = crate::globals::GraphPtr::from(item);
        tv_list_drop_items(l, item_ptr.borrow_mut(), item_ptr.borrow_mut());
        *rettv = std::mem::replace(&mut item_ptr.li_tv, tv_initial_value());
        crate::globals::GraphPtr::of(item_ptr.as_ptr()).drop_box();
    } else {
        let end = tv_get_number_chk(&argvars[2], Some(&mut error));
        if error {
            return;
        }
        let Some(item2) = tv_list_find_opt(Some(&mut *l), end as i32) else {
            semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), end);
            return;
        };
        let item_ptr = crate::globals::GraphPtr::<Listitem>::from_ref(item);
        let item2_ptr = crate::globals::GraphPtr::<Listitem>::from_ref(item2);
        let mut cnt: i32 = 0;
        let mut found_end = false;
        for li in crate::eval::typval::list_iter_ptr(item_ptr.opt_mut()) {
            cnt += 1;
            if item2_ptr == li {
                found_end = true;
                break;
            }
        }
        if !found_end {
            emsg(tr(E_INVRANGE));
        } else {
            let tgt = tv_list_alloc_ret(rettv, cnt as isize);
            tv_list_move_items(
                l,
                item_ptr.borrow_mut(),
                item2_ptr.borrow_mut(),
                &mut *tgt,
                cnt,
            );
        }
    }
}

/// Compare functions for f_sort() and f_uniq().
fn item_compare(si1: &ListSortItem, si2: &ListSortItem, keep_zero: bool, info: &Sortinfo) -> i32 {
    let tv1 = &mut si1.item_mut().li_tv;
    let tv2 = &mut si2.item_mut().li_tv;
    let mut res: i32;

    if info.item_compare_numbers {
        let v1 = if info.item_compare_keys_ready {
            si1.key.as_int()
        } else {
            tv_get_number(tv1)
        };
        let v2 = if info.item_compare_keys_ready {
            si2.key.as_int()
        } else {
            tv_get_number(tv2)
        };

        res = if v1 == v2 {
            0
        } else if v1 > v2 {
            1
        } else {
            -1
        };
        if res == 0 && !keep_zero {
            res = if si1.idx > si2.idx { 1 } else { -1 };
        }
        return res;
    }

    if info.item_compare_float {
        let v1 = if info.item_compare_keys_ready {
            si1.key.as_float()
        } else {
            tv_get_float(tv1)
        };
        let v2 = if info.item_compare_keys_ready {
            si2.key.as_float()
        } else {
            tv_get_float(tv2)
        };

        res = if v1 == v2 {
            0
        } else if v1 > v2 {
            1
        } else {
            -1
        };
        if res == 0 && !keep_zero {
            res = if si1.idx > si2.idx { 1 } else { -1 };
        }
        return res;
    }

    if info.item_compare_numeric && info.item_compare_keys_ready {
        let n1 = si1.key.as_float();
        let n2 = si2.key.as_float();

        res = if n1 == n2 {
            0
        } else if n1 > n2 {
            1
        } else {
            -1
        };
        if res == 0 && !keep_zero {
            res = if si1.idx > si2.idx { 1 } else { -1 };
        }
        return res;
    }

    let mut tofree1: Option<std::ffi::CString> = None;
    let mut tofree2: Option<std::ffi::CString> = None;
    let mut p1: RawCCharPtrMut;
    let mut p2: RawCCharPtrMut;

    if tv1.v_type == VarType::VarString {
        if tv2.v_type != VarType::VarString || info.item_compare_numeric {
            p1 = c"'".as_ptr().cast_mut();
        } else {
            p1 = tv1.vval.v_string().as_ptr();
        }
    } else {
        let s1 = encode_tv2string(tv1, None);
        p1 = s1.as_ptr().cast_mut();
        tofree1 = Some(s1);
    }
    if tv2.v_type == VarType::VarString {
        if tv1.v_type != VarType::VarString || info.item_compare_numeric {
            p2 = c"'".as_ptr().cast_mut();
        } else {
            p2 = tv2.vval.v_string().as_ptr();
        }
    } else {
        let s2 = encode_tv2string(tv2, None);
        p2 = s2.as_ptr().cast_mut();
        tofree2 = Some(s2);
    }
    if p1.is_null() {
        p1 = c"".as_ptr().cast_mut();
    }
    if p2.is_null() {
        p2 = c"".as_ptr().cast_mut();
    }
    if !info.item_compare_numeric {
        if info.item_compare_lc {
            res = unsafe { strcoll(p1, p2) };
        } else if info.item_compare_ic {
            res = crate::strings::ByteSlice::from_ptr(p1)
                .as_cstr()
                .to_bytes()
                .iter()
                .map(u8::to_ascii_lowercase)
                .cmp(
                    crate::strings::ByteSlice::from_ptr(p2)
                        .as_cstr()
                        .to_bytes()
                        .iter()
                        .map(u8::to_ascii_lowercase),
                ) as i32;
        } else {
            res = crate::strings::ByteSlice::from_ptr(p1)
                .as_cstr()
                .to_bytes()
                .cmp(crate::strings::ByteSlice::from_ptr(p2).as_bytes()) as i32;
        }
    } else {
        let (n1, _) = strtod(crate::strings::ByteSlice::from_ptr(p1).as_cstr());
        let (n2, _) = strtod(crate::strings::ByteSlice::from_ptr(p2).as_cstr());
        res = if n1 == n2 {
            0
        } else if n1 > n2 {
            1
        } else {
            -1
        };
    }

    drop(tofree1);
    drop(tofree2);

    if res == 0 && !keep_zero {
        res = if si1.idx > si2.idx { 1 } else { -1 };
    }
    res
}

fn item_compare2(
    si1: &ListSortItem,
    si2: &ListSortItem,
    keep_zero: bool,
    info: &mut Sortinfo,
) -> i32 {
    if info.item_compare_func_err {
        return 0;
    }

    let partial = info.item_compare_partial;
    let func_name = if partial.is_null() {
        info.item_compare_func
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr())
    } else {
        partial_name(partial.opt()).as_ptr()
    };

    let mut argv: [TypVal; 3] = [tv_initial_value(); 3];
    tv_copy(&si1.item_mut().li_tv, &mut argv[0]);
    tv_copy(&si2.item_mut().li_tv, &mut argv[1]);

    let mut rettv = tv_initial_value();
    let mut funcexe: Funcexe = Funcexe::new();
    funcexe.fe_evaluate = true;
    funcexe.fe_partial = partial;
    funcexe.fe_selfdict = info.item_compare_selfdict;
    let ok = call_func(
        crate::strings::ByteSlice::from_ptr(func_name).as_cstr(),
        -1,
        &mut rettv,
        2,
        &mut argv,
        &mut funcexe,
    );
    tv_clear(&mut argv[0]);
    tv_clear(&mut argv[1]);

    let mut res = if !ok {
        info.item_compare_func_err = true;
        ITEM_COMPARE_FAIL
    } else {
        let n = tv_get_number_chk(&rettv, Some(&mut info.item_compare_func_err));
        if n > 0 {
            1
        } else if n < 0 {
            -1
        } else {
            0
        }
    };
    if info.item_compare_func_err {
        res = ITEM_COMPARE_FAIL;
    }
    tv_clear(&mut rettv);

    if res == 0 && !keep_zero {
        res = if si1.idx > si2.idx { 1 } else { -1 };
    }

    res
}

fn sort_compute_keys(ptrs: &mut [ListSortItem], info: &mut Sortinfo) {
    if info.item_compare_numbers {
        for si in ptrs.iter_mut() {
            si.key = ListSortKey::Int(tv_get_number(&si.item_mut().li_tv));
        }
    } else if info.item_compare_float {
        for si in ptrs.iter_mut() {
            si.key = ListSortKey::Float(tv_get_float(&si.item_mut().li_tv));
        }
    } else {
        for si in ptrs.iter_mut() {
            let tv = &mut si.item_mut().li_tv;
            if tv.v_type == VarType::VarString {
                si.key = ListSortKey::Float(0.0);
            } else {
                let p = encode_tv2string(tv, None);
                let (f, _) = strtod(&p);
                si.key = ListSortKey::Float(f);
            }
        }
    }
    info.item_compare_keys_ready = true;
}

fn do_sort(l: &mut TvList, info: &mut Sortinfo) {
    let len = tv_list_len_inline(Some(&*l));
    let mut ptrs = Vec::with_capacity(len.max(0) as usize);

    for (i, li) in (0_i32..).zip(crate::eval::typval::list_iter_ptr(l.first_opt())) {
        ptrs.push(ListSortItem {
            item: crate::globals::GraphPtr::new(li),
            idx: i,
            key: ListSortKey::Int(0),
        });
    }

    info.item_compare_func_err = false;
    info.item_compare_keys_ready = false;

    let use_custom = info.item_compare_func.is_some() || !info.item_compare_partial.is_null();
    if !use_custom
        && (info.item_compare_numbers || info.item_compare_float || info.item_compare_numeric)
    {
        sort_compute_keys(&mut ptrs, info);
    }

    if !use_custom {
        ptrs.sort_by(|a, b| item_compare(a, b, false, info).cmp(&0));
    } else {
        ptrs.sort_by(|a, b| item_compare2(a, b, false, info).cmp(&0));
    }
    if !info.item_compare_func_err {
        l.lv_first = crate::globals::ListitemPtr::NULL;
        l.lv_last = crate::globals::ListitemPtr::NULL;
        l.lv_idx_item = crate::globals::ListitemPtr::NULL;
        l.lv_len = 0;
        for si in &ptrs {
            tv_list_append(l, si.item_mut());
        }
    }
    if info.item_compare_func_err {
        emsg(tr(c"E702: Sort compare function failed"));
    }
}

fn do_uniq(mut l: Option<&mut TvList>, info: &mut Sortinfo) {
    info.item_compare_func_err = false;
    info.item_compare_keys_ready = false;
    let use_custom = info.item_compare_func.is_some() || !info.item_compare_partial.is_null();

    let mut li = l
        .as_deref()
        .and_then(|l| l.first_opt())
        .map_or(ptr::null_mut(), |first| first.li_next.as_ptr());
    while let Some(cur_li) = crate::globals::GraphPtr::of(li).opt_mut() {
        let prev_li = cur_li.li_prev;
        let prev_si = ListSortItem {
            item: prev_li,
            idx: 0,
            key: ListSortKey::Int(0),
        };
        let cur_si = ListSortItem {
            item: crate::globals::GraphPtr::new(li),
            idx: 0,
            key: ListSortKey::Int(0),
        };
        let cmp = if !use_custom {
            item_compare(&prev_si, &cur_si, true, info)
        } else {
            item_compare2(&prev_si, &cur_si, true, info)
        };
        if cmp == 0 {
            li = tv_list_item_remove(
                l.as_deref_mut().unwrap(),
                crate::globals::GraphPtr::of(li).borrow_mut(),
            )
            .map_or(ptr::null_mut(), ptr::from_mut);
        } else {
            li = cur_li.li_next.as_ptr();
        }
        if info.item_compare_func_err {
            emsg(tr(c"E882: Uniq compare function failed"));
            break;
        }
    }
}

fn parse_sort_uniq_args(argvars: &mut [TypVal], info: &mut Sortinfo) -> bool {
    info.item_compare_ic = false;
    info.item_compare_lc = false;
    info.item_compare_numeric = false;
    info.item_compare_numbers = false;
    info.item_compare_float = false;
    info.item_compare_func = None;
    info.item_compare_partial = crate::globals::GraphPtr::NULL;
    info.item_compare_selfdict = crate::globals::GraphPtr::NULL;
    info.item_compare_keys_ready = false;

    if argvars[1].v_type == VarType::VarUnknown {
        return true;
    }

    if argvars[1].v_type == VarType::VarFunc {
        let s = argvars[1].vval.v_string();
        if !s.is_null() {
            info.item_compare_func = Some(s.to_cstring());
        }
    } else if argvars[1].v_type == VarType::VarPartial {
        info.item_compare_partial = argvars[1].vval.v_partial();
    } else {
        let mut error = false;
        let nr = tv_get_number_chk(&argvars[1], Some(&mut error)) as i32;
        if error {
            return false;
        }
        if nr == 1 {
            info.item_compare_ic = true;
        } else if argvars[1].v_type != VarType::VarNumber {
            let s = tv_get_string(&argvars[1]);
            info.item_compare_func = Some(s.to_owned());
        } else if nr != 0 {
            emsg(tr(E_INVARG));
            return false;
        }
        if let Some(ref func_cstr) = info.item_compare_func {
            match func_cstr.to_bytes() {
                b"" => info.item_compare_func = None,
                b"n" => {
                    info.item_compare_func = None;
                    info.item_compare_numeric = true;
                }
                b"N" => {
                    info.item_compare_func = None;
                    info.item_compare_numbers = true;
                }
                b"f" => {
                    info.item_compare_func = None;
                    info.item_compare_float = true;
                }
                b"i" => {
                    info.item_compare_func = None;
                    info.item_compare_ic = true;
                }
                b"l" => {
                    info.item_compare_func = None;
                    info.item_compare_lc = true;
                }
                _ => {}
            }
        }
    }

    if argvars[2].v_type != VarType::VarUnknown {
        if !tv_check_for_dict_arg(argvars, 2) {
            return false;
        }
        info.item_compare_selfdict = argvars[2].vval.v_dict();
    }

    true
}

fn do_sort_uniq(argvars: &mut [TypVal], rettv: &mut TypVal, sort: bool) {
    if argvars[0].v_type != VarType::VarList {
        semsg!(
            tr(E_LISTARG),
            crate::strings::ByteSlice::from_ptr(if sort {
                c"sort()".as_ptr()
            } else {
                c"uniq()".as_ptr()
            }),
        );
        return;
    }

    let mut info = Sortinfo::new();

    let arg_errmsg = if sort {
        c"sort() argument"
    } else {
        c"uniq() argument"
    };
    let mut l = argvars[0].vval.v_list_opt();
    if !value_check_lock(
        tv_list_locked_inline(l.as_deref()),
        Some(arg_errmsg),
        TV_TRANSLATE,
    ) {
        tv_list_set_ret_inline(rettv, l.as_deref_mut());

        let len = tv_list_len_inline(l.as_deref());
        if len > 1 && parse_sort_uniq_args(argvars, &mut info) {
            if sort {
                do_sort(l.unwrap(), &mut info);
            } else {
                do_uniq(l, &mut info);
            }
        }
    }
}

/// "sort({list})" function.
pub fn f_sort(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    do_sort_uniq(argvars, rettv, true);
}

/// "uniq({list})" function.
pub fn f_uniq(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    do_sort_uniq(argvars, rettv, false);
}

/// Check whether two lists are equal.
pub fn tv_list_equal(l1: Option<&TvList>, l2: Option<&TvList>, ic: bool) -> bool {
    if tv_list_len_inline(l1) != tv_list_len_inline(l2) {
        return false;
    }
    if tv_list_len_inline(l1) == 0 {
        return true;
    }
    let (Some(l1), Some(l2)) = (l1, l2) else {
        return false;
    };
    if ptr::eq(l1, l2) {
        return true;
    }

    let mut item1 = l1.first_opt();
    let mut item2 = l2.first_opt();
    while let (Some(i1), Some(i2)) = (item1.take(), item2.take()) {
        if !tv_equal(&i1.li_tv, &i2.li_tv, ic) {
            return false;
        }
        item1 = i1.next_opt();
        item2 = i2.next_opt();
    }
    debug_assert!(item1.is_none() && item2.is_none());
    true
}

/// Reverse list in-place.
pub fn tv_list_reverse(l: &mut TvList) {
    if tv_list_len_inline(Some(&*l)) <= 1 {
        return;
    }
    std::mem::swap(&mut l.lv_first, &mut l.lv_last);
    let mut li = l.first_opt();
    while let Some(cur) = li {
        std::mem::swap(&mut cur.li_next, &mut cur.li_prev);
        li = cur.next_opt();
    }
    l.lv_idx = l.lv_len - l.lv_idx - 1;
}

/// Locate item with a given index in a list and return it.
pub fn tv_list_find(mut l: Option<&mut TvList>, mut n: i32) -> Option<&'static mut Listitem> {
    l.as_ref()?;

    n = tv_list_uidx_inline(l.as_deref(), n);
    if n == -1 {
        return None;
    }

    let mut idx: i32;
    let mut item: crate::globals::ListitemPtr;

    if !l.as_deref_mut().unwrap().lv_idx_item.is_null() {
        if n < l.as_deref_mut().unwrap().lv_idx / 2 {
            item = l.as_deref_mut().unwrap().lv_first;
            idx = 0;
        } else if n > (l.as_deref_mut().unwrap().lv_idx + l.as_deref_mut().unwrap().lv_len) / 2 {
            item = l.as_deref_mut().unwrap().lv_last;
            idx = l.as_deref_mut().unwrap().lv_len - 1;
        } else {
            item = l.as_deref_mut().unwrap().lv_idx_item;
            idx = l.as_deref_mut().unwrap().lv_idx;
        }
    } else if n < l.as_deref_mut().unwrap().lv_len / 2 {
        item = l.as_deref_mut().unwrap().lv_first;
        idx = 0;
    } else {
        item = l.as_deref_mut().unwrap().lv_last;
        idx = l.as_deref_mut().unwrap().lv_len - 1;
    }

    while n > idx {
        item = item.li_next;
        idx += 1;
    }
    while n < idx {
        item = item.li_prev;
        idx -= 1;
    }

    debug_assert_eq!(idx, n);
    l.as_deref_mut().unwrap().lv_idx = idx;
    l.unwrap().lv_idx_item = item;

    item.opt_mut()
}

/// Locate item with a given index in a list and return a mutable reference if present.
pub fn tv_list_find_opt(l: Option<&mut TvList>, n: i32) -> Option<&'static mut Listitem> {
    tv_list_find(l, n)
}

/// Get list item l[n] as a number.
pub fn tv_list_find_nr(
    l: Option<&mut TvList>,
    n: i32,
    mut ret_error: Option<&mut bool>,
) -> Varnumber {
    let Some(li) = tv_list_find_opt(l, n) else {
        if let Some(ref mut p) = ret_error {
            **p = true;
        }
        return -1;
    };
    tv_get_number_chk(
        &li.li_tv,
        (ret_error.is_some()).then(|| &mut *ret_error.unwrap()),
    )
}

/// Get list item l[n] as a string.
pub fn tv_list_find_str(l: Option<&mut TvList>, n: i32) -> Option<&'static std::ffi::CStr> {
    let Some(li) = tv_list_find_opt(l, n) else {
        semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(n));
        return None;
    };
    Some(tv_get_string(&li.li_tv))
}

fn tv_list_find_index(mut l: Option<&mut TvList>, idx: &mut i32) -> Option<&'static mut Listitem> {
    let mut li = tv_list_find(l.as_deref_mut(), *idx);
    if li.is_some() {
        return li;
    }

    if *idx < 0 {
        *idx = 0;
        li = tv_list_find(l, *idx);
    }
    li
}

/// Locate item in a list and return its index.
pub fn tv_list_idx_of_item(l: Option<&TvList>, item: Option<&Listitem>) -> i32 {
    let (Some(l), Some(item_ref)) = (l, item) else {
        return -1;
    };
    let item_ptr = crate::globals::GraphPtr::<Listitem>::of(item_ref).as_ptr();
    for (idx, li) in (0_i32..).zip(crate::eval::typval::list_iter_ptr(l.first_opt())) {
        if ptr::eq(li, item_ptr) {
            return idx;
        }
    }
    -1
}

/// Add watcher to a dictionary.
pub fn tv_dict_watcher_add(dict: Option<&mut TvDict>, key_pattern: &[u8], callback: Callback) {
    let Some(dict) = dict else {
        return;
    };
    let key_pattern_len = key_pattern.len();
    dict.watchers.push(DictWatcher {
        key_pattern: crate::memory::bytes_to_cstring(key_pattern),
        key_pattern_len,
        callback,
        busy: false,
        needs_free: false,
    });
}

/// Check whether two callbacks are equal.
pub fn tv_callback_equal(cb1: &Callback, cb2: &Callback) -> bool {
    if cb1.cb_type() != cb2.cb_type() {
        return false;
    }
    match cb1.cb_type() {
        K_CALLBACK_FUNCREF | K_CALLBACK_EXPR => cb1.data.as_funcref() == cb2.data.as_funcref(),
        K_CALLBACK_PARTIAL => cb1.data.as_partial() == cb2.data.as_partial(),
        K_CALLBACK_LUA => cb1.data.as_luaref() == cb2.data.as_luaref(),
        K_CALLBACK_NONE_CONST => true,
        _ => std::process::abort(),
    }
}

/// Unref/free callback.
pub fn callback_free(callback: &mut Callback) {
    match callback.cb_type() {
        K_CALLBACK_FUNCREF => {
            func_unref(callback.data.as_funcref());
        }
        K_CALLBACK_PARTIAL => {
            partial_unref(callback.data.as_partial().opt_mut());
        }
        K_CALLBACK_LUA => {
            let mut luaref = callback.data.as_luaref();
            nlua_clear_ref(&mut luaref);
        }
        _ => {}
    }
    *callback = Callback::NONE;
}

/// Copy a callback into a TypVal.
pub fn callback_put(cb: &mut Callback, tv: &mut TypVal) {
    match cb.cb_type() {
        K_CALLBACK_PARTIAL => {
            let pt = cb.data.as_partial();
            tv.v_type = VarType::VarPartial;
            tv.vval = crate::eval::typval::TypvalVval::Partial(pt);

            pt.borrow_mut().pt_refcount += 1;
        }
        K_CALLBACK_FUNCREF => {
            let funcref = cb.data.as_funcref();
            tv_set_string(&mut *tv, funcref);
            tv.v_type = VarType::VarFunc;
            let s = tv.vval.v_string();
            func_ref(s.as_opt_cstr());
        }
        _ => {
            tv.v_type = VarType::VarSpecial;
            tv.vval = crate::eval::typval::TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST);
        }
    }
}

/// Copy callback from "src" to "dest", incrementing the refcounts.
pub fn callback_copy(dest: &mut Callback, src: &Callback) {
    match src.cb_type() {
        K_CALLBACK_PARTIAL => {
            let pt = src.data.as_partial();
            dest.data = CallbackData::partial(pt);

            pt.borrow_mut().pt_refcount += 1;
        }
        K_CALLBACK_FUNCREF => {
            let funcref = src.data.as_funcref();
            dest.data = CallbackData::funcref(funcref.unwrap_or(c"").to_owned());
            func_ref(funcref);
        }
        K_CALLBACK_LUA => {
            let new_ref = api_new_luaref(src.data.as_luaref());
            dest.data = CallbackData::lua(new_ref);
        }
        K_CALLBACK_EXPR => {
            dest.data = CallbackData::expr(src.data.as_funcref().unwrap_or(c"").to_owned());
        }
        _ => {
            dest.data = CallbackData::NONE;
        }
    }
}

/// Generate a string description of a callback.
pub fn callback_to_string(cb: &mut Callback) -> std::ffi::CString {
    if cb.cb_type() == K_CALLBACK_LUA {
        return nlua_funcref_str(cb.data.as_luaref(), true);
    }

    const MSGLEN: usize = 100;
    let mut msg: Vec<u8> = Vec::with_capacity(MSGLEN);

    match cb.cb_type() {
        K_CALLBACK_FUNCREF => {
            let s = cb.data.as_funcref().unwrap_or(c"").to_bytes();
            let prefix = b"<vim function: ";
            let suffix = b">";
            let copy_len = s.len().min(MSGLEN - prefix.len() - suffix.len());
            msg.extend_from_slice(prefix);
            msg.extend_from_slice(&s[..copy_len]);
            if msg.len() < MSGLEN {
                msg.push(b'>');
            }
        }
        K_CALLBACK_PARTIAL => {
            let pt = cb.data.as_partial();
            let s = pt.pt_name.as_ref().map_or(&[][..], |n| n.as_bytes());
            let prefix = b"<vim partial: ";
            let suffix = b">";
            let copy_len = s.len().min(MSGLEN - prefix.len() - suffix.len());
            msg.extend_from_slice(prefix);
            msg.extend_from_slice(&s[..copy_len]);
            if msg.len() < MSGLEN {
                msg.push(b'>');
            }
        }
        _ => {}
    }
    bytes_to_cstring(&msg)
}

/// Remove watcher from a dictionary.
pub fn tv_dict_watcher_remove(
    mut dict: Option<&mut TvDict>,
    key_pattern: &[u8],
    callback: &Callback,
) -> bool {
    if dict.is_none() {
        return false;
    }
    let key_pattern_len = key_pattern.len();

    let mut matched_idx: Option<usize> = None;
    let mut queue_is_busy = false;
    for (idx, w) in dict.as_deref_mut().unwrap().watchers.iter().enumerate() {
        if w.busy {
            queue_is_busy = true;
        }
        if matched_idx.is_none()
            && tv_callback_equal(&w.callback, callback)
            && w.key_pattern_len == key_pattern_len
            && w.key_pattern.as_bytes() == key_pattern
        {
            matched_idx = Some(idx);
            if queue_is_busy {
                break;
            }
        }
    }

    let Some(idx) = matched_idx else {
        return false;
    };

    if queue_is_busy {
        (&mut dict.as_deref_mut().unwrap().watchers)[idx].needs_free = true;
    } else {
        dict.unwrap().watchers.remove(idx);
    }
    true
}

/// Test if `key` matches with `watcher->key_pattern`.
fn tv_dict_watcher_matches(watcher: &DictWatcher, key_bytes: &[u8]) -> bool {
    let pat_bytes = watcher.key_pattern.as_bytes();
    if let Some(prefix) = pat_bytes.strip_suffix(b"*") {
        key_bytes.starts_with(prefix)
    } else {
        key_bytes == pat_bytes
    }
}

/// Send a change notification to all dictionary watchers that match given key.
pub fn tv_dict_watcher_notify(
    dict_in: &mut TvDict,
    key: &CStr,
    newtv: Option<&TypVal>,
    oldtv: Option<&TypVal>,
) {
    let mut dict = crate::globals::GraphPtr::from(dict_in);
    let mut argv: [TypVal; 3] = [tv_initial_value(); 3];

    argv[0].v_type = VarType::VarDict;
    argv[0].v_lock = VAR_UNLOCKED;
    argv[0].vval = crate::eval::typval::TypvalVval::Dict(dict);
    argv[1].v_lock = VAR_UNLOCKED;
    tv_set_string(&mut argv[1], Some(key));
    argv[2].v_type = VarType::VarDict;
    argv[2].v_lock = VAR_UNLOCKED;
    let changes_dict = tv_dict_alloc();
    argv[2].vval =
        crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::new(changes_dict));
    changes_dict.dv_refcount += 1;

    if let Some(newtv) = newtv {
        let v = tv_dict_item_alloc_len(b"new");
        tv_copy(newtv, &mut v.di_tv);
        tv_dict_add(&mut *changes_dict, v);
    }

    if let Some(oldtv) = oldtv
        && oldtv.v_type != VarType::VarUnknown
    {
        let v = tv_dict_item_alloc_len(b"old");
        tv_copy(oldtv, &mut v.di_tv);
        tv_dict_add(&mut *changes_dict, v);
    }

    let mut any_needs_free = false;
    dict.dv_refcount += 1;
    let key_bytes = key.to_bytes();
    let len = dict.watchers.len();
    for i in 0..len {
        let matches = {
            let w = &dict.watchers[i];
            !w.busy && tv_dict_watcher_matches(w, key_bytes)
        };
        if matches {
            let mut rettv = tv_initial_value();
            dict.watchers[i].busy = true;
            let mut cb = dict.watchers[i].callback.clone();
            callback_call(&mut cb, 3, &mut argv, &mut rettv);
            let w = &mut dict.watchers[i];
            w.callback = cb;
            w.busy = false;
            tv_clear(&mut rettv);
            if w.needs_free {
                any_needs_free = true;
            }
        }
    }
    if any_needs_free {
        dict.watchers.retain(|watcher| !watcher.needs_free);
    }
    tv_dict_unref(dict.opt_mut());

    for item in argv.iter_mut().skip(1) {
        tv_clear(item);
    }
}

/// Allocate a dictionary item.
pub fn tv_dict_item_alloc_len(key: &[u8]) -> &'static mut Dictitem {
    let mut key_vec = Vec::with_capacity(key.len() + 1);
    key_vec.extend_from_slice(key);
    key_vec.push(0);
    Box::leak(Box::new(Dictitem {
        di_tv: TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        },
        di_flags: DI_FLAGS_ALLOC,
        di_key: key_vec,
    }))
}

/// Allocate a dictionary item.
pub fn tv_dict_item_alloc(key: &CStr) -> &'static mut Dictitem {
    tv_dict_item_alloc_len(key.to_bytes())
}

/// Free a dictionary item, also clearing the value.
pub fn tv_dict_item_free(item_in: &mut Dictitem) {
    let mut item = crate::globals::GraphPtr::from(item_in);
    if (item.di_flags & DI_FLAGS_ALLOC) != 0 {
        crate::globals::GraphPtr::of(item.as_ptr()).drop_box();
    } else {
        tv_clear(&mut item.di_tv);
    }
}

/// Make a copy of a dictionary item.
pub fn tv_dict_item_copy(di: &mut Dictitem) -> &'static mut Dictitem {
    let new_di = tv_dict_item_alloc(di.key_cstr());
    tv_copy(&di.di_tv, &mut new_di.di_tv);
    new_di
}

/// Remove item from dictionary and free it.
pub fn tv_dict_item_remove(dict: &mut TvDict, item: &mut Dictitem) {
    if let Some(hi) = dict.dv_hashtab.find_hi_mut(item.key_cstr()) {
        hash_remove(&mut dict.dv_hashtab, hi);
    } else {
        semsg!(tr(E_INTERN2), c"tv_dict_item_remove()");
    }
    tv_dict_item_free(item);
}

/// Allocate an empty dictionary.
pub fn tv_dict_alloc() -> &'static mut TvDict {
    let d = Box::leak(Box::new(TvDict {
        dv_lock: VAR_UNLOCKED,
        dv_scope: VAR_NO_SCOPE,
        dv_refcount: 0,
        dv_copy_id: 0,
        dv_hashtab: crate::hashtab::Hashtab::new(),
        dv_copydict: crate::globals::GraphPtr::NULL,
        dv_gc_idx: None,
        watchers: Vec::new(),
        lua_table_ref: LUA_NOREF,
    }));

    gc_register_dict(d);

    d
}

/// Free items contained in a dictionary.
pub fn tv_dict_free_contents(d: &mut TvDict) {
    hash_lock(&mut d.dv_hashtab);
    debug_assert!(d.dv_hashtab.ht_locked > 0);
    for i in 0..d.dv_hashtab.ht_array.len() {
        if d.dv_hashtab.ht_used == 0 {
            break;
        }
        if let Some(di) = d.dv_hashtab.ht_array[i].di_opt() {
            d.dv_hashtab.remove_at(i);
            tv_dict_item_free(di);
        }
    }

    d.watchers.clear();

    hash_clear(&mut d.dv_hashtab);
}

/// Free a dictionary, including all items it contains.
pub fn tv_dict_free(d_in: &mut TvDict) {
    if tv_st().tv_in_free_unref_items {
        return;
    }
    let d = crate::globals::GraphPtr::from(d_in);

    tv_list_dict_free_dict_inner(d.borrow_mut());

    crate::globals::GraphPtr::of(d.as_ptr()).drop_box();
}

#[inline]
fn tv_list_dict_free_dict_inner(d: &mut TvDict) {
    tv_dict_free_contents(d);
}

/// Unreference a dictionary.
pub fn tv_dict_unref(d: Option<&mut TvDict>) {
    if let Some(d_ref) = d {
        d_ref.dv_refcount -= 1;
        if d_ref.dv_refcount <= 0 {
            tv_dict_free(d_ref);
        }
    }
}

/// Find item in dictionary.
pub fn tv_dict_find(d: Option<&TvDict>, key: &CStr, len: isize) -> Option<&'static mut Dictitem> {
    let d = d?;
    let hi = if len < 0 {
        d.dv_hashtab.find_hi_mut(key)
    } else {
        d.dv_hashtab
            .find_hi_len_mut(crate::strings::AsByteSlice::slice_len(
                key.as_ptr(),
                len as usize,
            ))
    };
    hi.and_then(|hi| hi.hi_di.opt_mut())
}

/// Find item in dictionary and return a mutable reference if present.
pub fn tv_dict_find_opt(
    d: Option<&TvDict>,
    key: &CStr,
    len: isize,
) -> Option<&'static mut Dictitem> {
    tv_dict_find(d, key, len)
}

/// Check if a key is present in a dictionary.
pub fn tv_dict_has_key(d: Option<&TvDict>, key: &std::ffi::CStr) -> bool {
    tv_dict_find(d, key, -1).is_some()
}

/// Get a typval item from a dictionary and copy it into "rettv".
pub fn tv_dict_get_tv(d: Option<&TvDict>, key: &std::ffi::CStr, rettv: &mut TypVal) -> bool {
    let Some(di) = tv_dict_find_opt(d, key, -1) else {
        return false;
    };

    tv_copy(&di.di_tv, rettv);
    true
}

/// Gets a number item from a dictionary.
pub fn tv_dict_get_number(d: Option<&TvDict>, key: &std::ffi::CStr) -> Varnumber {
    tv_dict_get_number_def(d, key, 0)
}

/// Gets a number item from a dictionary, or a given default value.
pub fn tv_dict_get_number_def(d: Option<&TvDict>, key: &std::ffi::CStr, def: i32) -> Varnumber {
    let Some(di) = tv_dict_find_opt(d, key, -1) else {
        return i64::from(def);
    };
    tv_get_number(&di.di_tv)
}

/// Gets a bool item from a dictionary, or a given default value.
pub fn tv_dict_get_bool(d: Option<&TvDict>, key: &std::ffi::CStr, def: i32) -> Varnumber {
    let Some(di) = tv_dict_find_opt(d, key, -1) else {
        return i64::from(def);
    };
    tv_get_bool(&di.di_tv)
}

/// Converts a dict to an environment.
pub fn tv_dict_to_env(denv: Option<&TvDict>) -> Vec<std::ffi::CString> {
    let env_size = tv_dict_len_inline(denv) as usize;
    let mut env = Vec::with_capacity(env_size);

    if let Some(denv) = denv {
        for (_, var) in denv.dv_hashtab.iter_items() {
            let str_ = tv_get_string(&var.di_tv);
            let key_bytes = var.key_bytes();
            let val_bytes = str_.to_bytes();
            let mut entry = Vec::with_capacity(key_bytes.len() + 1 + val_bytes.len());
            entry.extend_from_slice(key_bytes);
            entry.push(b'=');
            entry.extend_from_slice(val_bytes);
            env.push(bytes_to_cstring(&entry));
        }
    }

    env
}

/// Get an owned `CString` item from a dictionary.
pub fn tv_dict_get_cstring(d: Option<&TvDict>, key: &std::ffi::CStr) -> Option<std::ffi::CString> {
    let mut numbuf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    tv_dict_get_string_buf(d, key, &mut numbuf).map(std::ffi::CStr::to_owned)
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string<'a>(d: Option<&'a TvDict>, key: &CStr) -> Option<&'a std::ffi::CStr> {
    tv_dict_get_string_buf(d, key, &mut tv_st().borrow_mut().dict_numbuf)
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string_buf<'a>(
    d: Option<&'a TvDict>,
    key: &CStr,
    numbuf: &'a mut [u8; NUMBUFLEN],
) -> Option<&'a std::ffi::CStr> {
    let di = tv_dict_find_opt(d, key, -1)?;
    Some(tv_get_string_buf(&di.di_tv, numbuf))
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string_buf_chk<'a>(
    d: Option<&'a TvDict>,
    key: &std::ffi::CStr,
    key_len: isize,
    numbuf: &'a mut [u8; NUMBUFLEN],
    def: Option<&'a std::ffi::CStr>,
) -> Option<&'a std::ffi::CStr> {
    let Some(di) = tv_dict_find_opt(d, key, key_len) else {
        return def;
    };
    tv_get_string_buf_chk(&di.di_tv, numbuf)
}

/// Get a function from a dictionary.
pub fn tv_dict_get_callback(
    d: Option<&mut TvDict>,
    key: &std::ffi::CStr,
    key_len: isize,
    result: &mut Callback,
) -> bool {
    *result = Callback::NONE;

    let Some(di) = tv_dict_find_opt(d.as_deref(), key, key_len) else {
        return true;
    };

    if !tv_is_func_inline(&di.di_tv) && di.di_tv.v_type != VarType::VarString {
        emsg(tr(c"E6000: Argument is not a function or function name"));
        return false;
    }

    let mut tv = tv_initial_value();
    tv_copy(&di.di_tv, &mut tv);
    set_selfdict(&mut tv, d.unwrap());
    let res = callback_from_typval(result, &tv);
    tv_clear(&mut tv);
    res
}

/// Check for adding a function to g: or l:.
pub fn tv_dict_wrong_func_name(d: &mut TvDict, tv: &mut TypVal, name: &CStr) -> i32 {
    i32::from(
        (ptr::eq(d, get_globvar_dict())
            || get_funccal_local_ht().is_some_and(|lht| ptr::eq(&raw const d.dv_hashtab, lht)))
            && tv_is_func_inline(tv)
            && var_wrong_func_name(name, true),
    )
}

/// Add item to dictionary.
pub fn tv_dict_add(d: &mut TvDict, item: &mut Dictitem) -> bool {
    let key = crate::memory::bytes_as_cstr(&item.di_key);
    if tv_dict_wrong_func_name(d, &mut item.di_tv, key) != 0 {
        return false;
    }
    hash_add(&mut d.dv_hashtab, item)
}

/// Add a list entry to dictionary.
pub fn tv_dict_add_list(d: &mut TvDict, key: &CStr, mut list: Option<&mut TvList>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarList;
    item.di_tv.vval =
        crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::from(list.as_deref_mut()));
    if !tv_dict_add(d, item) {
        item.di_tv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::NULL);
        tv_dict_item_free(item);
        return false;
    }
    tv_list_ref_inline(list);
    true
}

/// Add a typval entry to dictionary.
pub fn tv_dict_add_tv(d: &mut TvDict, key: &[u8], tv: &mut TypVal) -> bool {
    let item = tv_dict_item_alloc_len(key);
    tv_copy(&*tv, &mut item.di_tv);
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Add a dictionary entry to dictionary.
pub fn tv_dict_add_dict(d: &mut TvDict, key: &CStr, mut dict: Option<&mut TvDict>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarDict;
    item.di_tv.vval =
        crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::from(dict.as_deref_mut()));
    if !tv_dict_add(d, item) {
        item.di_tv.vval = crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::NULL);
        tv_dict_item_free(item);
        return false;
    }

    if let Some(dict_ref) = dict {
        dict_ref.dv_refcount += 1;
    }

    true
}

/// Add a number entry to dictionary.
pub fn tv_dict_add_nr(d: &mut TvDict, key: &std::ffi::CStr, nr: Varnumber) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarNumber;
    item.di_tv.vval = crate::eval::typval::TypvalVval::Number(nr);
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Add a floating point number entry to dictionary.
pub fn tv_dict_add_float(d: &mut TvDict, key: &std::ffi::CStr, nr: TvFloat) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarFloat;
    item.di_tv.vval = crate::eval::typval::TypvalVval::Float(nr);
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Add a boolean entry to dictionary.
pub fn tv_dict_add_bool(d: &mut TvDict, key: &std::ffi::CStr, val: BoolVarValue) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarBool;
    item.di_tv.vval = crate::eval::typval::TypvalVval::Bool(val);
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Add a string entry to dictionary.
pub fn tv_dict_add_str(d: &mut TvDict, key: &CStr, val: Option<&CStr>) -> bool {
    tv_dict_add_str_len(d, key, val.map(CStr::to_bytes))
}

/// Add a string entry to dictionary.
pub fn tv_dict_add_str_len(d: &mut TvDict, key: &CStr, val: Option<&[u8]>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.di_tv.v_type = VarType::VarString;
    item.di_tv.vval =
        crate::eval::typval::TypvalVval::from_opt_cstring(val.map(crate::memory::bytes_to_cstring));
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Add a function entry to dictionary.
pub fn tv_dict_add_func(d: &mut TvDict, key: &std::ffi::CStr, fp: &mut UserFunc) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    tv_set_string_len(
        &mut item.di_tv,
        Some(&fp.uf_name.as_bytes()[..fp.uf_namelen]),
    );
    item.di_tv.v_type = VarType::VarFunc;
    let s = item.di_tv.vval.v_string();
    func_ref(s.as_opt_cstr());
    if !tv_dict_add(d, item) {
        tv_dict_item_free(item);
        return false;
    }
    true
}

/// Clear all the keys of a Dictionary.
#[cfg(test)]
pub fn tv_dict_clear(d: &mut TvDict) {
    hash_lock(&mut d.dv_hashtab);
    debug_assert!(d.dv_hashtab.ht_locked > 0);

    for i in 0..d.dv_hashtab.ht_array.len() {
        if d.dv_hashtab.ht_used == 0 {
            break;
        }
        if let Some(di) = d.dv_hashtab.ht_array[i].di_opt() {
            tv_dict_item_free(di);
            d.dv_hashtab.remove_at(i);
        }
    }

    hash_unlock(&mut d.dv_hashtab);
}

/// Extend dictionary with items from another dictionary.
pub fn tv_dict_extend(d1: &mut TvDict, d2: &mut TvDict, action: &[u8]) {
    let first_action = action.first().copied().unwrap_or(0);
    let watched = tv_dict_is_watched_inline(Some(d1));
    let arg_errmsg = tr(c"extend() argument");
    let arg_errmsg_len = arg_errmsg.to_bytes().len();

    if first_action == b'm' {
        hash_lock(&mut d2.dv_hashtab);
    }

    let mut todo = d2.dv_hashtab.ht_used;
    for i in 0..d2.dv_hashtab.ht_array.len() {
        if todo == 0 {
            break;
        }
        if let Some(di2) = d2.dv_hashtab.ht_array[i].di_opt() {
            todo -= 1;
            let key2 = crate::memory::bytes_as_cstr(&di2.di_key);
            let di1_opt = tv_dict_find_opt(Some(d1), key2, -1);
            if d1.dv_scope != VAR_NO_SCOPE && !valid_varname(di2.key_cstr()) {
                break;
            }
            if let Some(di1) = di1_opt {
                if first_action == b'e' {
                    semsg!(tr(c"E737: Key already exists: %s"), key2);
                    break;
                } else if first_action == b'f' && !ptr::eq(di2, di1) {
                    if value_check_lock(di1.di_tv.v_lock, Some(arg_errmsg), arg_errmsg_len)
                        || var_check_ro(i32::from(di1.di_flags), arg_errmsg, arg_errmsg_len)
                    {
                        break;
                    }
                    if tv_dict_wrong_func_name(&mut *d1, &mut di2.di_tv, key2) != 0 {
                        break;
                    }

                    let mut oldtv = tv_initial_value();
                    if watched {
                        tv_copy(&di1.di_tv, &mut oldtv);
                    }

                    tv_clear(&mut di1.di_tv);
                    tv_copy(&di2.di_tv, &mut di1.di_tv);

                    if watched {
                        tv_dict_watcher_notify(d1, di1.key_cstr(), Some(&di1.di_tv), Some(&oldtv));
                        tv_clear(&mut oldtv);
                    }
                }
            } else if first_action == b'm' {
                let new_di = di2;
                if tv_dict_add(d1, new_di) {
                    d2.dv_hashtab.remove_at(i);
                    tv_dict_watcher_notify(d1, new_di.key_cstr(), Some(&new_di.di_tv), None);
                }
            } else {
                let new_di = tv_dict_item_copy(&mut *di2);
                if !tv_dict_add(d1, new_di) {
                    tv_dict_item_free(new_di);
                } else if watched {
                    tv_dict_watcher_notify(d1, new_di.key_cstr(), Some(&new_di.di_tv), None);
                }
            }
        }
    }

    if first_action == b'm' {
        hash_unlock(&mut d2.dv_hashtab);
    }
}

/// Compare two dictionaries.
pub fn tv_dict_equal(d1: Option<&TvDict>, d2: Option<&TvDict>, ic: bool) -> bool {
    if tv_dict_len_inline(d1) != tv_dict_len_inline(d2) {
        return false;
    }
    if tv_dict_len_inline(d1) == 0 {
        return true;
    }
    let (Some(d1), Some(d2)) = (d1, d2) else {
        return false;
    };
    if ptr::eq(d1, d2) {
        return true;
    }

    for (_, di1) in d1.dv_hashtab.iter_items() {
        let Some(di2) = tv_dict_find_opt(Some(d2), di1.key_cstr(), -1) else {
            return false;
        };
        if !tv_equal(&di1.di_tv, &di2.di_tv, ic) {
            return false;
        }
    }
    true
}

/// Make a copy of dictionary.
pub fn tv_dict_copy(
    conv: Option<&Vimconv>,
    mut orig: Option<&mut TvDict>,
    deep: bool,
    copy_id: i32,
) -> Option<&'static mut TvDict> {
    orig.as_ref()?;

    let copy = tv_dict_alloc();
    if copy_id != 0 {
        orig.as_deref_mut().unwrap().dv_copy_id = copy_id;
        orig.as_deref_mut().unwrap().dv_copydict = crate::globals::GraphPtr::from(&mut *copy);
    }
    for (_, di) in orig.unwrap().dv_hashtab.iter_items_mut() {
        if g_st().got_int {
            break;
        }
        let key_cstr = di.key_cstr();
        let new_di = if let Some(conv) = conv.filter(|c| c.vc_type != CONV_NONE) {
            let key_bytes = key_cstr.to_bytes();
            let mut len = key_bytes.len();
            if let Some(key) = string_convert(conv, key_cstr.to_bytes(), Some(&mut len)) {
                tv_dict_item_alloc_len(&key[..len])
            } else {
                tv_dict_item_alloc_len(key_bytes)
            }
        } else {
            tv_dict_item_alloc(key_cstr)
        };
        if deep {
            if !var_item_copy(conv, &mut di.di_tv, &mut new_di.di_tv, deep, copy_id) {
                tv_dict_item_free(new_di);
                break;
            }
        } else {
            tv_copy(&di.di_tv, &mut new_di.di_tv);
        }
        if !tv_dict_add(&mut *copy, new_di) {
            tv_dict_item_free(new_di);
            break;
        }
    }

    copy.dv_refcount += 1;
    let copy_ptr = crate::globals::GraphPtr::<TvDict>::from_ref(copy);
    if g_st().got_int {
        tv_dict_unref(copy_ptr.opt_mut());
        return None;
    }

    copy_ptr.opt_mut()
}

/// Set all existing keys in "dict" as read-only.
pub fn tv_dict_set_keys_readonly(dict: &mut TvDict) {
    for (_, di) in dict.dv_hashtab.iter_items_mut() {
        di.di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
    }
}

/// Allocate an empty blob.
pub fn tv_blob_alloc() -> &'static mut TvBlob {
    Box::leak(Box::new(TvBlob {
        bv_ga: Vec::new(),
        bv_refcount: 0,
        bv_lock: 0,
    }))
}

/// Unreference a blob.
pub fn tv_blob_unref(b: Option<&mut TvBlob>) {
    if let Some(b_ref) = b {
        b_ref.bv_refcount -= 1;
        if b_ref.bv_refcount <= 0 {
            let b_ptr = crate::globals::GraphPtr::from(b_ref);
            crate::globals::GraphPtr::of(b_ptr.as_ptr()).drop_box();
        }
    }
}

/// Check whether two blobs are equal.
pub fn tv_blob_equal(b1: Option<&TvBlob>, b2: Option<&TvBlob>) -> bool {
    let len1 = tv_blob_len_inline(b1);
    let len2 = tv_blob_len_inline(b2);
    if len1 == 0 && len2 == 0 {
        return true;
    }
    if len1 != len2 {
        return false;
    }
    match (b1, b2) {
        (Some(b1), Some(b2)) => ptr::eq(b1, b2) || b1.bv_ga == b2.bv_ga,
        _ => false,
    }
}

fn tv_blob_slice(
    len: i32,
    mut n1: Varnumber,
    mut n2: Varnumber,
    exclusive: bool,
    rettv: &mut TypVal,
) -> bool {
    let len64 = i64::from(len);
    if n1 < 0 {
        n1 += len64;
        if n1 < 0 {
            n1 = 0;
        }
    }
    if n2 < 0 {
        n2 += len64;
    } else if n2 >= len64 {
        n2 = len64 - if exclusive { 0 } else { 1 };
    }
    if exclusive {
        n2 -= 1;
    }
    if n1 >= len64 || n2 < 0 || n1 > n2 {
        tv_clear(&mut *rettv);
        rettv.v_type = VarType::VarBlob;
        rettv.vval = crate::eval::typval::TypvalVval::Blob(crate::globals::GraphPtr::NULL);
    } else {
        let new_blob = tv_blob_alloc();
        let src_blob = rettv.vval.v_blob_ref();
        let new_bv = &mut new_blob.bv_ga;
        new_bv.extend_from_slice(&src_blob.bv_ga[(n1 as usize)..=(n2 as usize)]);
        tv_clear(&mut *rettv);
        tv_blob_set_ret_inline(rettv, new_blob);
    }

    true
}

fn tv_blob_index(len: i32, mut idx: Varnumber, rettv: &mut TypVal) -> bool {
    let len64 = i64::from(len);
    if idx < 0 {
        idx += len64;
    }
    if idx < len64 && idx >= 0 {
        let v = i64::from(tv_blob_get_inline(rettv.vval.v_blob_ref(), idx as i32));
        tv_clear(&mut *rettv);
        rettv.v_type = VarType::VarNumber;
        rettv.vval = crate::eval::typval::TypvalVval::Number(v);
    } else {
        semsg!(tr(E_BLOBIDX), idx);
        return false;
    }

    true
}

pub fn tv_blob_slice_or_index(
    is_range: bool,
    n1: Varnumber,
    n2: Varnumber,
    exclusive: bool,
    rettv: &mut TypVal,
) -> bool {
    let len = tv_blob_len_inline(rettv.vval.v_blob_opt().as_deref());
    if is_range {
        tv_blob_slice(len, n1, n2, exclusive, rettv)
    } else {
        tv_blob_index(len, n1, rettv)
    }
}

/// Check if "n1" is a valid index for a blob with length "bloblen".
pub fn tv_blob_check_index(bloblen: i32, n1: Varnumber, quiet: bool) -> bool {
    if n1 < 0 || n1 > i64::from(bloblen) {
        if !quiet {
            semsg!(tr(E_BLOBIDX), n1);
        }
        return false;
    }
    true
}

/// Check if "n1"-"n2" is a valid range for a blob with length "bloblen".
pub fn tv_blob_check_range(bloblen: i32, n1: Varnumber, n2: Varnumber, quiet: bool) -> bool {
    if n2 < 0 || n2 >= i64::from(bloblen) || n2 < n1 {
        if !quiet {
            semsg!(tr(E_BLOBIDX), n2);
        }
        return false;
    }
    true
}

/// Set bytes "n1" to "n2" (inclusive) in "dest" to the value of "src".
pub fn tv_blob_set_range(
    dest: &mut TvBlob,
    n1: Varnumber,
    n2: Varnumber,
    src: &mut TypVal,
) -> bool {
    if n2 - n1 + 1 != i64::from(tv_blob_len_inline(src.vval.v_blob_opt().as_deref())) {
        emsg(tr(
            c"E972: Blob value does not have the right number of bytes",
        ));
        return false;
    }

    let src_blob = src.vval.v_blob_ref();
    for (ir, il) in (0_i32..).zip((n1 as i32)..=(n2 as i32)) {
        tv_blob_set_inline(dest, il, tv_blob_get_inline(src_blob, ir));
    }
    true
}

/// Store one byte "byte" in blob "blob" at "idx".
pub fn tv_blob_set_append(blob: &mut TvBlob, idx: i32, byte: u8) {
    let bv = &mut blob.bv_ga;
    let len = bv.len() as i32;
    if idx < len {
        bv[idx as usize] = byte;
    } else if idx == len {
        bv.push(byte);
    }
}

/// "remove({blob})" function.
pub fn tv_blob_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    let b = argvars[0].vval.v_blob_opt();
    if let Some(ref b_ref) = b
        && value_check_lock(b_ref.bv_lock, Some(arg_errmsg), TV_TRANSLATE)
    {
        return;
    }

    let mut error = false;
    let mut idx = tv_get_number_chk(&argvars[1], Some(&mut error));

    if !error {
        let len = i64::from(tv_blob_len_inline(b.as_deref()));
        if idx < 0 {
            idx += len;
        }
        if idx < 0 || idx >= len {
            semsg!(tr(E_BLOBIDX), idx);
            return;
        }
        let b_ref = b.unwrap();
        if argvars[2].v_type == VarType::VarUnknown {
            let removed = b_ref.bv_ga.remove(idx as usize);
            rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(removed));
        } else {
            let mut end = tv_get_number_chk(&argvars[2], Some(&mut error));
            if error {
                return;
            }
            if end < 0 {
                end += len;
            }
            if end >= len || idx > end {
                semsg!(tr(E_BLOBIDX), end);
                return;
            }
            let blob = tv_blob_alloc();
            let dst_bv = &mut blob.bv_ga;
            dst_bv.extend(b_ref.bv_ga.drain((idx as usize)..=(end as usize)));
            tv_blob_set_ret_inline(rettv, blob);
        }
    }
}

/// blob2list() function.
pub fn f_blob2list(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

    if !tv_check_for_blob_arg(argvars, 0) {
        return;
    }

    let Some(blob) = argvars[0].vval.v_blob_opt() else {
        return;
    };
    let l = rettv.vval.v_list_ref();
    for i in 0..tv_blob_len_inline(Some(blob)) {
        tv_list_append_number(l, i64::from(tv_blob_get_inline(blob, i)));
    }
}

/// list2blob() function.
pub fn f_list2blob(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let blob = tv_blob_alloc_ret(rettv);

    if !tv_check_for_list_arg(argvars, 0) {
        return;
    }

    let Some(l) = argvars[0].vval.v_list_opt() else {
        return;
    };

    let bv = &mut blob.bv_ga;
    let list_len = tv_list_len_inline(Some(l));
    if list_len > 0 {
        bv.reserve(list_len as usize);
    }

    for li in crate::eval::typval::list_iter_mut(l.first_opt()) {
        let mut error = false;
        let n = tv_get_number_chk(&li.li_tv, Some(&mut error));
        if error || !(0..=255).contains(&n) {
            if !error {
                semsg!(tr(E_INVALID_VALUE_FOR_BLOB_NR), n as i32);
            }
            bv.clear();
            return;
        }
        bv.push(n as u8);
    }
}

/// Allocate an empty list for a return value.
pub fn tv_list_alloc_ret(ret_tv: &mut TypVal, len: isize) -> &'static mut TvList {
    let l = tv_list_alloc(len);
    tv_list_set_ret_inline(ret_tv, Some(l));
    ret_tv.v_lock = VAR_UNLOCKED;
    &mut *l
}

pub fn tv_dict_alloc_lock(lock: VarLockStatus) -> &'static mut TvDict {
    let d = &mut *tv_dict_alloc();
    d.dv_lock = lock;
    d
}

/// Allocate an empty dictionary for a return value.
pub fn tv_dict_alloc_ret(ret_tv: &mut TypVal) -> &'static mut TvDict {
    let d = tv_dict_alloc_lock(VAR_UNLOCKED);
    tv_dict_set_ret_inline(ret_tv, d);
    &mut *d
}

fn tv_dict2list(argvars: &mut [TypVal], rettv: &mut TypVal, what: DictListType) {
    if !tv_check_for_dict_arg(argvars, 0) {
        tv_list_alloc_ret(&mut *rettv, 0);
        return;
    }

    let d = argvars[0].vval.v_dict_opt();
    tv_list_alloc_ret(&mut *rettv, tv_dict_len_inline(d.as_deref()));
    let Some(d) = d else {
        return;
    };

    for (_, di) in d.dv_hashtab.iter_items_mut() {
        let key_cstr = di.key_cstr();
        let mut tv_item = tv_initial_value();

        match what {
            DictListType::Keys => {
                tv_set_string(&mut tv_item, Some(key_cstr));
            }
            DictListType::Values => {
                tv_copy(&di.di_tv, &mut tv_item);
            }
            DictListType::Items => {
                let sub_l = tv_list_alloc(2);
                tv_item.v_type = VarType::VarList;
                tv_item.vval =
                    crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::new(sub_l));
                tv_list_ref_inline(Some(&mut *sub_l));
                tv_list_append_string(&mut *sub_l, Some(key_cstr), -1);
                tv_list_append_tv(&mut *sub_l, &mut di.di_tv);
            }
        }

        tv_list_append_owned_tv(rettv.vval.v_list_ref(), tv_item);
    }
}

/// "items()" function.
pub fn f_items(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    match argvars[0].v_type {
        VarType::VarString => tv_string2items(argvars, rettv),
        VarType::VarList => tv_list2items(argvars, rettv),
        VarType::VarBlob => tv_blob2items(argvars, rettv),
        VarType::VarDict => tv_dict2items(argvars, rettv),
        _ => {
            semsg!(tr(E_LIST_DICT_BLOB_OR_STRING_REQUIRED_FOR_ARGUMENT_NR), 1,);
        }
    }
}

/// "keys()" function.
pub fn f_keys(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_dict2list(argvars, rettv, DictListType::Keys);
}

/// "values(dict)" function.
pub fn f_values(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_dict2list(argvars, rettv, DictListType::Values);
}

/// "has_key()" function.
pub fn f_has_key(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_dict_arg(argvars, 0) {
        return;
    }

    let Some(d) = argvars[0].vval.v_dict_opt() else {
        return;
    };

    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(
        tv_dict_find(Some(d), tv_get_string(&argvars[1]), -1).is_some(),
    ));
}

/// "remove({dict})" function.
pub fn tv_dict_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    if argvars[2].v_type != VarType::VarUnknown {
        semsg!(tr(E_TOOMANYARG), c"remove()");
    } else if let Some(d) = argvars[0].vval.v_dict_opt()
        && !value_check_lock(d.dv_lock, Some(arg_errmsg), TV_TRANSLATE)
        && let Some(key) = tv_get_string_chk(&argvars[1])
    {
        let Some(di) = tv_dict_find_opt(Some(d), key, -1) else {
            semsg!(tr(E_DICTKEY), key);
            return;
        };
        if !var_check_fixed(i32::from(di.di_flags), arg_errmsg, TV_TRANSLATE)
            && !var_check_ro(i32::from(di.di_flags), arg_errmsg, TV_TRANSLATE)
        {
            *rettv = di.di_tv;
            di.di_tv = tv_initial_value();
            tv_dict_item_remove(d, di);
            if tv_dict_is_watched_inline(Some(d)) {
                tv_dict_watcher_notify(d, key, None, Some(&*rettv));
            }
        }
    }
}

/// Allocate an empty blob for a return value.
pub fn tv_blob_alloc_ret(ret_tv: &mut TypVal) -> &'static mut TvBlob {
    let b = tv_blob_alloc();
    tv_blob_set_ret_inline(ret_tv, b);
    b
}

/// Copy a blob typval to a different typval.
pub fn tv_blob_copy(from: Option<&TvBlob>, to: &mut TypVal) {
    to.v_type = VarType::VarBlob;
    to.v_lock = VAR_UNLOCKED;
    if let Some(from) = from {
        let dst_blob = &mut *tv_blob_alloc_ret(to);
        dst_blob.bv_ga.extend_from_slice(&from.bv_ga);
    } else {
        to.vval = crate::eval::typval::TypvalVval::Blob(crate::globals::GraphPtr::NULL);
    }
}

#[inline]
fn nothing_conv_func_start(tv: &mut TypVal) -> bool {
    tv.v_lock = VAR_UNLOCKED;
    if tv.v_type == VarType::VarPartial {
        if let Some(pt) = tv.vval.v_partial_opt()
            && pt.pt_refcount > 1
        {
            pt.pt_refcount -= 1;
            tv.vval = crate::eval::typval::TypvalVval::Partial(crate::globals::GraphPtr::NULL);
            return true;
        }
    } else {
        let fun = tv_take_string(tv);
        func_unref(fun.as_deref());
    }
    false
}

#[inline]
fn nothing_conv_func_end(tv: &mut TypVal, copy_id: i32) {
    if tv.v_type == VarType::VarPartial {
        let Some(pt) = tv.vval.v_partial_opt() else {
            return;
        };
        let dict = pt.pt_dict;
        debug_assert!(dict.is_null() || dict.dv_copy_id == copy_id);
        pt.pt_dict = crate::globals::GraphPtr::NULL;
        pt.pt_argv.clear();
        debug_assert!(pt.pt_refcount <= 1);
        partial_unref(Some(pt));
        tv.vval = crate::eval::typval::TypvalVval::Partial(crate::globals::GraphPtr::NULL);
        debug_assert_eq!(tv.v_lock, VAR_UNLOCKED);
    }
}

#[inline]
fn nothing_conv_real_list_after_start(tv: &mut TypVal, mpsv: &mut MPConvStackVal) {
    tv.v_lock = VAR_UNLOCKED;
    let list = tv.vval.v_list_ref();
    if list.lv_refcount > 1 {
        list.lv_refcount -= 1;
        tv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::NULL);
        if let MPConvStackData::List { ref mut li, .. } = mpsv.data {
            *li = crate::globals::GraphPtr::NULL;
        }
    }
}

#[inline]
fn nothing_conv_list_end(mut tv: Option<&mut TypVal>) {
    if tv.is_none() {
        return;
    }
    debug_assert_eq!(tv.as_deref_mut().unwrap().v_type, VarType::VarList);
    let list = tv.as_deref_mut().unwrap().vval.v_list_opt();
    tv_list_unref(list);
    tv.unwrap().vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::NULL);
}

fn nothing_convert_one_value(
    mpstack: &mut Vec<MPConvStackVal>,
    tv: &mut TypVal,
    copy_id: i32,
) -> bool {
    match tv.v_type {
        VarType::VarString => {
            let _ = tv_take_string(tv);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarNumber => {
            tv.vval = crate::eval::typval::TypvalVval::Number(0);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarFloat => {
            tv.vval = crate::eval::typval::TypvalVval::Float(0.0);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarBlob => {
            tv_blob_unref(tv.vval.v_blob_opt());
            tv.vval = crate::eval::typval::TypvalVval::Blob(crate::globals::GraphPtr::NULL);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarFunc => {
            let _ = nothing_conv_func_start(tv);
        }
        VarType::VarPartial => {
            let pt = tv.vval.v_partial();
            if nothing_conv_func_start(tv) {
                return true;
            }
            mpstack.push(MPConvStackVal {
                tv: crate::globals::GraphPtr::from(&mut *tv),
                saved_copy_id: copy_id - 1,
                data: MPConvStackData::Partial {
                    stage: MPConvPartialStage::Args,
                    pt,
                },
            });
        }
        VarType::VarList => {
            let mut list = tv.vval.v_list();
            if list.is_null() || tv_list_len_inline(list.opt()) == 0 {
                tv_list_unref(list.opt_mut());
                tv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::NULL);
                tv.v_lock = VAR_UNLOCKED;
                return true;
            }
            let saved_copy_id = list.lv_copy_id;
            if saved_copy_id == copy_id {
                return true;
            }

            {
                list.lv_copy_id = copy_id;
            }

            let mut mpsv = MPConvStackVal {
                tv: crate::globals::GraphPtr::from(&mut *tv),
                saved_copy_id,
                data: MPConvStackData::List {
                    list,
                    li: list.lv_first,
                },
            };
            nothing_conv_real_list_after_start(tv, &mut mpsv);
            mpstack.push(mpsv);
        }
        VarType::VarBool => {
            tv.vval = crate::eval::typval::TypvalVval::Bool(K_BOOL_VAR_FALSE_CONST);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarSpecial => {
            tv.vval = crate::eval::typval::TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST);
            tv.v_lock = VAR_UNLOCKED;
        }
        VarType::VarDict => {
            let tv_ptr = &raw mut *tv;
            let mut dict = tv.vval.v_dict();
            if dict.is_null() || dict.dv_hashtab.ht_used == 0 {
                tv_dict_unref(dict.opt_mut());
                tv.vval = crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::NULL);
                tv.v_lock = VAR_UNLOCKED;
                return true;
            }
            let saved_copy_id = dict.dv_copy_id;
            if saved_copy_id == copy_id {
                return true;
            }

            tv.v_lock = VAR_UNLOCKED;
            if dict.dv_refcount > 1 {
                {
                    dict.dv_refcount -= 1;
                }
                tv.vval = crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::NULL);
                return true;
            }

            {
                dict.dv_copy_id = copy_id;
            }

            let mpsv = MPConvStackVal {
                tv: crate::globals::GraphPtr::new(tv_ptr),
                saved_copy_id,
                data: MPConvStackData::Dict {
                    dict,
                    hi: dict.dv_hashtab.array_ptr().into(),
                    todo: dict.dv_hashtab.ht_used,
                },
            };
            mpstack.push(mpsv);
        }
        VarType::VarUnknown => {
            internal_error(c"_typval_encode_nothing_convert_one_value()");
            return false;
        }
    }
    true
}

fn encode_vim_to_nothing(top_tv: &mut TypVal) -> bool {
    let copy_id = get_copy_id();

    let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);
    if !nothing_convert_one_value(&mut mpstack, top_tv, copy_id) {
        return false;
    }

    while !mpstack.is_empty() {
        let last_idx = mpstack.len() - 1;
        let saved_copy_id = mpstack[last_idx].saved_copy_id;
        let cur_tv = mpstack[last_idx].tv;
        let tv = crate::globals::GraphPtr::<TypVal>::of(match mpstack[last_idx].data {
            MPConvStackData::Dict {
                mut dict,
                ref mut hi,
                ref mut todo,
            } => {
                if *todo == 0 {
                    mpstack.pop();

                    dict.dv_copy_id = saved_copy_id;
                    if let Some(tv_ref) = cur_tv.opt_mut() {
                        tv_ref.vval =
                            crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::NULL);
                    }
                    tv_dict_unref(dict.opt_mut());
                    continue;
                }
                while hashitem_empty(hi.borrow()) {
                    *hi = (*hi).wrapping_add(1);
                }
                let di = tv_dict_hi2di(hi.borrow());
                *todo -= 1;
                *hi = (*hi).wrapping_add(1);
                &raw mut di.di_tv
            }
            MPConvStackData::List {
                mut list,
                ref mut li,
            } => {
                let cur_li = *li;
                if cur_li.is_null() {
                    mpstack.pop();

                    list.lv_copy_id = saved_copy_id;

                    nothing_conv_list_end(cur_tv.opt_mut());

                    continue;
                }
                let cur_li = cur_li.borrow_mut();
                *li = cur_li.li_next;
                &raw mut cur_li.li_tv
            }
            MPConvStackData::Partial { stage, mut pt } => {
                match stage {
                    MPConvPartialStage::Args => {
                        if let MPConvStackData::Partial {
                            stage: ref mut stage_ref,
                            ..
                        } = mpstack[last_idx].data
                        {
                            *stage_ref = MPConvPartialStage::SelfDict;
                        }
                        if !pt.is_null() && !pt.pt_argv.is_empty() {
                            mpstack.push(MPConvStackVal {
                                tv: crate::globals::GraphPtr::NULL,
                                saved_copy_id: copy_id - 1,
                                data: MPConvStackData::PartialList {
                                    arg: pt.pt_argv.as_mut_ptr().into(),
                                    argv: pt.pt_argv.as_mut_ptr().into(),
                                    todo: pt.pt_argv.len(),
                                },
                            });
                        }
                    }
                    MPConvPartialStage::SelfDict => {
                        if let MPConvStackData::Partial {
                            stage: ref mut stage_ref,
                            ..
                        } = mpstack[last_idx].data
                        {
                            *stage_ref = MPConvPartialStage::End;
                        }
                        let mut dict = if pt.is_null() {
                            crate::globals::GraphPtr::NULL
                        } else {
                            pt.pt_dict
                        };
                        if !dict.is_null() {
                            if dict.dv_hashtab.ht_used == 0 {
                                pt.pt_dict = crate::globals::GraphPtr::NULL;
                                tv_dict_unref(dict.opt_mut());
                                continue;
                            }
                            let saved_copy_id = dict.dv_copy_id;
                            if saved_copy_id == copy_id {
                                continue;
                            }
                            pt.pt_dict = crate::globals::GraphPtr::NULL;
                            if dict.dv_refcount > 1 {
                                dict.dv_refcount -= 1;
                                continue;
                            }

                            dict.dv_copy_id = copy_id;

                            let mpsv = MPConvStackVal {
                                tv: crate::globals::GraphPtr::NULL,
                                saved_copy_id,
                                data: MPConvStackData::Dict {
                                    dict,
                                    hi: dict.dv_hashtab.array_ptr().into(),
                                    todo: dict.dv_hashtab.ht_used,
                                },
                            };
                            mpstack.push(mpsv);
                        }
                    }
                    MPConvPartialStage::End => {
                        nothing_conv_func_end(cur_tv.borrow_mut(), copy_id);

                        mpstack.pop();
                    }
                }
                continue;
            }
            MPConvStackData::PartialList {
                ref mut arg,
                argv: _argv,
                ref mut todo,
            } => {
                let cur_arg = *arg;
                if *todo == 0 {
                    mpstack.pop();

                    nothing_conv_list_end(cur_tv.opt_mut());

                    continue;
                }
                *arg = cur_arg.wrapping_add(1);
                *todo -= 1;
                cur_arg.as_ptr()
            }
            MPConvStackData::Pairs { .. } => unreachable!(),
        })
        .as_ptr();
        debug_assert!(!tv.is_null());
        if !nothing_convert_one_value(
            &mut mpstack,
            crate::globals::GraphPtr::of(tv).borrow_mut(),
            copy_id,
        ) {
            return false;
        }
    }
    true
}

/// Free memory for a variable value and set the value to NULL or 0.
pub fn tv_clear(tv: &mut TypVal) {
    if tv.v_type == VarType::VarUnknown {
        return;
    }
    let evn_ret = encode_vim_to_nothing(tv);
    debug_assert!(evn_ret);
}

/// Copy typval from one location to another.
pub fn tv_copy(from: &TypVal, to: &mut TypVal) {
    to.v_type = from.v_type;
    to.v_lock = VAR_UNLOCKED;
    to.vval = from.vval;
    match from.v_type {
        VarType::VarNumber | VarType::VarFloat | VarType::VarBool | VarType::VarSpecial => {}
        VarType::VarString | VarType::VarFunc => {
            if !from.vval.v_string().is_null() {
                to.vval = crate::eval::typval::TypvalVval::from_cstring(from.vval.v_string_owned());
                if from.v_type == VarType::VarFunc {
                    let s = to.vval.v_string();
                    func_ref(s.as_opt_cstr());
                }
            }
        }
        VarType::VarPartial => {
            if let Some(pt) = to.vval.v_partial_opt() {
                pt.pt_refcount += 1;
            }
        }
        VarType::VarBlob => {
            if let Some(b) = from.vval.v_blob_opt() {
                b.bv_refcount += 1;
            }
        }
        VarType::VarList => {
            tv_list_ref_inline(to.vval.v_list_opt());
        }
        VarType::VarDict => {
            if let Some(d) = from.vval.v_dict_opt() {
                d.dv_refcount += 1;
            }
        }
        VarType::VarUnknown => {
            semsg!(tr(E_INTERN2), c"tv_copy(UNKNOWN)");
        }
    }
}

#[inline]
const fn change_lock(lock: bool, var: VarLockStatus) -> VarLockStatus {
    if var == VAR_FIXED {
        VAR_FIXED
    } else if lock {
        VAR_LOCKED
    } else {
        VAR_UNLOCKED
    }
}

/// Lock or unlock an item.
pub fn tv_item_lock(tv: &mut TypVal, deep: i32, lock: bool, check_refcount: bool) {
    static RECURSE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    if RECURSE.load(std::sync::atomic::Ordering::Relaxed) >= DICT_MAXNEST {
        emsg(tr(E_VARIABLE_NESTED_TOO_DEEP_FOR_UNLOCK));
        return;
    }
    if deep == 0 {
        return;
    }
    RECURSE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    tv.v_lock = change_lock(lock, tv.v_lock);

    match tv.v_type {
        VarType::VarBlob => {
            if let Some(b) = tv.vval.v_blob_opt()
                && !(check_refcount && b.bv_refcount > 1)
            {
                b.bv_lock = change_lock(lock, b.bv_lock);
            }
        }
        VarType::VarList => {
            if let Some(l) = tv.vval.v_list_opt()
                && !(check_refcount && l.lv_refcount > 1)
            {
                l.lv_lock = change_lock(lock, l.lv_lock);
                if !(0..=1).contains(&deep) {
                    for li in crate::eval::typval::list_iter_mut(l.first_opt()) {
                        tv_item_lock(&mut li.li_tv, deep - 1, lock, check_refcount);
                    }
                }
            }
        }
        VarType::VarDict => {
            if let Some(d) = tv.vval.v_dict_opt()
                && !(check_refcount && d.dv_refcount > 1)
            {
                d.dv_lock = change_lock(lock, d.dv_lock);
                if !(0..=1).contains(&deep) {
                    for (_, di) in d.dv_hashtab.iter_items_mut() {
                        tv_item_lock(&mut di.di_tv, deep - 1, lock, check_refcount);
                    }
                }
            }
        }
        VarType::VarNumber
        | VarType::VarFloat
        | VarType::VarString
        | VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarBool
        | VarType::VarSpecial => {}
        VarType::VarUnknown => std::process::abort(),
    }
    RECURSE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

/// Check whether Vimscript value is locked itself or refers to a locked container.
pub fn tv_islocked(tv: &TypVal) -> bool {
    tv.v_lock == VAR_LOCKED
        || (tv.v_type == VarType::VarList
            && tv_list_locked_inline(tv.vval.v_list_opt().as_deref()) == VAR_LOCKED)
        || (tv.v_type == VarType::VarDict
            && tv
                .vval
                .v_dict_opt()
                .is_some_and(|d| d.dv_lock == VAR_LOCKED))
}

/// Return true if typval is locked.
pub fn tv_check_lock(tv: &TypVal, name: Option<&CStr>, name_len: usize) -> bool {
    let mut lock = VAR_UNLOCKED;

    match tv.v_type {
        VarType::VarBlob => {
            if let Some(b) = tv.vval.v_blob_opt() {
                lock = b.bv_lock;
            }
        }
        VarType::VarList => {
            if let Some(l) = tv.vval.v_list_opt() {
                lock = l.lv_lock;
            }
        }
        VarType::VarDict => {
            if let Some(d) = tv.vval.v_dict_opt() {
                lock = d.dv_lock;
            }
        }
        _ => {}
    }
    value_check_lock(tv.v_lock, name, name_len)
        || (lock != VAR_UNLOCKED && value_check_lock(lock, name, name_len))
}

/// Return true if variable "name" has a locked (immutable) value.
pub fn value_check_lock(lock: VarLockStatus, name_opt: Option<&CStr>, mut name_len: usize) -> bool {
    let mut name = crate::strings::ByteSlice::from_opt_cstr(name_opt);
    match lock {
        VAR_UNLOCKED => return false,
        VAR_LOCKED => {
            let error_message = if name.is_null() {
                E_VALUE_IS_LOCKED
            } else {
                E_VALUE_IS_LOCKED_STR
            };
            if name.is_null() {
                emsg(tr(error_message));
            } else {
                let tr_name;
                if name_len == TV_TRANSLATE {
                    tr_name = tr(name.as_cstr());
                    name = crate::strings::ByteSlice::from_cstr(tr_name);
                    name_len = tr_name.to_bytes().len();
                } else if name_len == TV_CSTRING {
                    name_len = name.len();
                }
                semsg!(tr(error_message), name_len as i32, name);
            }
        }
        VAR_FIXED => {
            let error_message = if name.is_null() {
                E_CANNOT_CHANGE_VALUE
            } else {
                E_CANNOT_CHANGE_VALUE_OF_STR
            };
            if name.is_null() {
                emsg(tr(error_message));
            } else {
                let tr_name;
                if name_len == TV_TRANSLATE {
                    tr_name = tr(name.as_cstr());
                    name = crate::strings::ByteSlice::from_cstr(tr_name);
                    name_len = tr_name.to_bytes().len();
                } else if name_len == TV_CSTRING {
                    name_len = name.len();
                }
                semsg!(tr(error_message), name_len as i32, name);
            }
        }
        _ => {
            let _ = (&mut name, &mut name_len);
        }
    }
    true
}

/// Compare two Vimscript values.
pub fn tv_equal(tv1: &TypVal, tv2: &TypVal, ic: bool) -> bool {
    static RECURSIVE_CNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    if !(tv_is_func_inline(tv1) && tv_is_func_inline(tv2)) && tv1.v_type != tv2.v_type {
        return false;
    }

    let cur_cnt = RECURSIVE_CNT.load(std::sync::atomic::Ordering::Relaxed);
    if cur_cnt == 0 {
        tv_st().tv_equal_recurse_limit = 1000;
    }
    if cur_cnt >= tv_st().tv_equal_recurse_limit {
        tv_st().tv_equal_recurse_limit -= 1;
        return true;
    }

    match tv1.v_type {
        VarType::VarList => {
            RECURSIVE_CNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = tv_list_equal(
                tv1.vval.v_list_opt().as_deref(),
                tv2.vval.v_list_opt().as_deref(),
                ic,
            );
            RECURSIVE_CNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            r
        }
        VarType::VarDict => {
            RECURSIVE_CNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = tv_dict_equal(
                tv1.vval.v_dict_opt().as_deref(),
                tv2.vval.v_dict_opt().as_deref(),
                ic,
            );
            RECURSIVE_CNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            r
        }
        VarType::VarPartial | VarType::VarFunc => {
            if (tv1.v_type == VarType::VarPartial && tv1.vval.v_partial().is_null())
                || (tv2.v_type == VarType::VarPartial && tv2.vval.v_partial().is_null())
            {
                return false;
            }
            RECURSIVE_CNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = func_equal(tv1, tv2, ic);
            RECURSIVE_CNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            r
        }
        VarType::VarBlob => tv_blob_equal(
            tv1.vval.v_blob_opt().as_deref(),
            tv2.vval.v_blob_opt().as_deref(),
        ),
        VarType::VarNumber => tv1.vval.v_number() == tv2.vval.v_number(),
        VarType::VarFloat => tv1.vval.v_float() == tv2.vval.v_float(),
        VarType::VarString => {
            let mut buf1: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
            let mut buf2: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
            let s1 = tv_get_string_buf(tv1, &mut buf1);
            let s2 = tv_get_string_buf(tv2, &mut buf2);
            mb_strcmp_ic(ic, s1, s2) == 0
        }
        VarType::VarBool => tv1.vval.v_bool() == tv2.vval.v_bool(),
        VarType::VarSpecial => tv1.vval.v_special() == tv2.vval.v_special(),
        VarType::VarUnknown => false,
    }
}

/// Check that given value is a number or string.
pub fn tv_check_str_or_nr(tv: &TypVal) -> bool {
    match tv.v_type {
        VarType::VarNumber | VarType::VarString => true,
        VarType::VarFloat => {
            emsg(tr(c"E805: Expected a Number or a String, Float found"));
            false
        }
        VarType::VarPartial | VarType::VarFunc => {
            emsg(tr(c"E703: Expected a Number or a String, Funcref found"));
            false
        }
        VarType::VarList => {
            emsg(tr(c"E745: Expected a Number or a String, List found"));
            false
        }
        VarType::VarDict => {
            emsg(tr(c"E728: Expected a Number or a String, Dictionary found"));
            false
        }
        VarType::VarBlob => {
            emsg(tr(c"E974: Expected a Number or a String, Blob found"));
            false
        }
        VarType::VarBool => {
            emsg(tr(c"E5299: Expected a Number or a String, Boolean found"));
            false
        }
        VarType::VarSpecial => {
            emsg(tr(c"E5300: Expected a Number or a String"));
            false
        }
        VarType::VarUnknown => {
            semsg!(tr(E_INTERN2), c"tv_check_str_or_nr(&*UNKNOWN)",);
            false
        }
    }
}

#[inline]
const fn num_error(vt: VarType) -> &'static std::ffi::CStr {
    match vt {
        VarType::VarPartial | VarType::VarFunc => c"E703: Using a Funcref as a Number",
        VarType::VarList => c"E745: Using a List as a Number",
        VarType::VarDict => c"E728: Using a Dictionary as a Number",
        VarType::VarFloat => c"E805: Using a Float as a Number",
        VarType::VarBlob => c"E974: Using a Blob as a Number",
        VarType::VarUnknown => c"E685: using an invalid value as a Number",
        _ => c"",
    }
}

#[inline]
const fn str_error(vt: VarType) -> &'static std::ffi::CStr {
    match vt {
        VarType::VarPartial | VarType::VarFunc => c"E729: Using a Funcref as a String",
        VarType::VarList => c"E730: Using a List as a String",
        VarType::VarDict => c"E731: Using a Dictionary as a String",
        VarType::VarBlob => c"E976: Using a Blob as a String",
        VarType::VarUnknown => E_USING_INVALID_VALUE_AS_STRING,
        _ => c"",
    }
}

/// Check that given value is a number or can be converted to it.
pub fn tv_check_num(tv: &TypVal) -> bool {
    match tv.v_type {
        VarType::VarNumber | VarType::VarBool | VarType::VarSpecial | VarType::VarString => true,
        VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarList
        | VarType::VarDict
        | VarType::VarFloat
        | VarType::VarBlob
        | VarType::VarUnknown => {
            emsg(tr(num_error(tv.v_type)));
            false
        }
    }
}

/// Check that given value is a Vimscript String or can be "cast" to it.
pub fn tv_check_str(tv: &TypVal) -> bool {
    match tv.v_type {
        VarType::VarNumber
        | VarType::VarBool
        | VarType::VarSpecial
        | VarType::VarString
        | VarType::VarFloat => true,
        VarType::VarPartial
        | VarType::VarFunc
        | VarType::VarList
        | VarType::VarDict
        | VarType::VarBlob
        | VarType::VarUnknown => {
            emsg(tr(str_error(tv.v_type)));
            false
        }
    }
}

/// Get the number value of a Vimscript object.
pub fn tv_get_number(tv: &TypVal) -> Varnumber {
    let mut error = false;
    tv_get_number_chk(tv, Some(&mut error))
}

/// Get the number value of a Vimscript object.
pub fn tv_get_number_chk(tv: &TypVal, mut ret_error: Option<&mut bool>) -> Varnumber {
    match tv.v_type {
        VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarList
        | VarType::VarDict
        | VarType::VarBlob
        | VarType::VarFloat => {
            emsg(tr(num_error(tv.v_type)));
        }
        VarType::VarNumber => return tv.vval.v_number(),
        VarType::VarString => {
            let mut n: Varnumber = 0;
            if !tv.vval.v_string().is_null() {
                vim_str2nr(
                    tv.vval.v_string_bytes(),
                    None,
                    None,
                    STR2NR_ALL,
                    Some(&mut n),
                    None,
                    0,
                    false,
                    None,
                );
            }
            return n;
        }
        VarType::VarBool => return i64::from(tv.vval.v_bool() == K_BOOL_VAR_TRUE_CONST),
        VarType::VarSpecial => return 0,
        VarType::VarUnknown => {
            semsg!(tr(E_INTERN2), c"tv_get_number(&*UNKNOWN)");
        }
    }
    if let Some(ref mut p) = ret_error {
        **p = true;
    }
    if ret_error.is_none() { -1 } else { 0 }
}

/// Get the boolean value of a Vimscript object.
pub fn tv_get_bool(tv: &TypVal) -> Varnumber {
    tv_get_number_chk(tv, None)
}

pub fn tv_get_bool_chk(tv: &TypVal, ret_error: Option<&mut bool>) -> Varnumber {
    tv_get_number_chk(tv, ret_error)
}

/// Get the line number from Vimscript object.
pub fn tv_get_lnum(tv: &TypVal) -> Linenr {
    let did_emsg_before = g_st().did_emsg;
    let mut lnum = tv_get_number_chk(tv, None) as Linenr;
    if lnum <= 0 && did_emsg_before == g_st().did_emsg && tv.v_type != VarType::VarNumber {
        let mut fnum: i32 = 0;
        if let Some(fp) = var2fpos(tv, true, &mut fnum, false, curwin_mut()) {
            lnum = fp.lnum;
        }
    }
    lnum
}

/// Get the line number from Vimscript object.
pub fn tv_get_lnum_buf(tv: &TypVal, buf: Option<&FileBuf>) -> Linenr {
    if tv.v_type == VarType::VarString
        && !tv.vval.v_string().is_null()
        && tv.vval.v_string_bytes() == b"$"
        && let Some(buf) = buf
    {
        return buf.b_ml.ml_line_count;
    }
    tv_get_number_chk(tv, None) as Linenr
}

/// Get the floating-point value of a Vimscript object.
pub fn tv_get_float(tv: &TypVal) -> TvFloat {
    match tv.v_type {
        VarType::VarNumber => tv.vval.v_number() as TvFloat,
        VarType::VarFloat => tv.vval.v_float(),
        VarType::VarPartial | VarType::VarFunc => {
            emsg(tr(c"E891: Using a Funcref as a Float"));
            0.0
        }
        VarType::VarString => {
            emsg(tr(c"E892: Using a String as a Float"));
            0.0
        }
        VarType::VarList => {
            emsg(tr(c"E893: Using a List as a Float"));
            0.0
        }
        VarType::VarDict => {
            emsg(tr(c"E894: Using a Dictionary as a Float"));
            0.0
        }
        VarType::VarBool => {
            emsg(tr(c"E362: Using a boolean value as a Float"));
            0.0
        }
        VarType::VarSpecial => {
            emsg(tr(c"E907: Using a special value as a Float"));
            0.0
        }
        VarType::VarBlob => {
            emsg(tr(c"E975: Using a Blob as a Float"));
            0.0
        }
        VarType::VarUnknown => {
            semsg!(tr(E_INTERN2), c"tv_get_float(&*UNKNOWN)");
            0.0
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string.
pub fn tv_check_for_string_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type != VarType::VarString {
        semsg!(tr(E_STRING_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a non-empty string.
pub fn tv_check_for_nonempty_string_arg(args: &[TypVal], idx: i32) -> bool {
    if !tv_check_for_string_arg(args, idx) {
        return false;
    }
    let s = args[idx as usize].vval.v_string();
    if s.is_null() || { s.is_empty() } {
        semsg!(tr(E_NON_EMPTY_STRING_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Check for an optional string argument at "idx".
pub fn tv_check_for_opt_string_arg(args: &[TypVal], idx: i32) -> bool {
    args[idx as usize].v_type == VarType::VarUnknown || tv_check_for_string_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a number.
pub fn tv_check_for_number_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type != VarType::VarNumber {
        semsg!(tr(E_NUMBER_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Check for an optional number argument at "idx".
pub fn tv_check_for_opt_number_arg(args: &[TypVal], idx: i32) -> bool {
    args[idx as usize].v_type == VarType::VarUnknown || tv_check_for_number_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a float or a number.
pub fn tv_check_for_float_or_nr_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarFloat && vt != VarType::VarNumber {
        semsg!(tr(E_FLOAT_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a bool.
pub fn tv_check_for_bool_arg(args: &[TypVal], idx: i32) -> bool {
    let arg = &args[idx as usize];
    if arg.v_type != VarType::VarBool
        && !(arg.v_type == VarType::VarNumber
            && (arg.vval.v_number() == 0 || arg.vval.v_number() == 1))
    {
        semsg!(tr(E_BOOL_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Check for an optional bool argument at "idx".
pub fn tv_check_for_opt_bool_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type == VarType::VarUnknown {
        return true;
    }
    tv_check_for_bool_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a blob.
pub fn tv_check_for_blob_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type != VarType::VarBlob {
        semsg!(tr(E_BLOB_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a list.
pub fn tv_check_for_list_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type != VarType::VarList {
        semsg!(tr(E_LIST_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a dict.
pub fn tv_check_for_dict_arg(args: &[TypVal], idx: i32) -> bool {
    if args[idx as usize].v_type != VarType::VarDict {
        semsg!(tr(E_DICT_REQUIRED_FOR_ARGUMENT_NR), idx + 1);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a non-NULL dict.
pub fn tv_check_for_nonnull_dict_arg(args: &[TypVal], idx: i32) -> bool {
    if !tv_check_for_dict_arg(args, idx) {
        return false;
    }
    if args[idx as usize].vval.v_dict().is_null() {
        semsg!(tr(E_NON_NULL_DICT_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Check for an optional dict argument at "idx".
pub fn tv_check_for_opt_dict_arg(args: &[TypVal], idx: i32) -> bool {
    args[idx as usize].v_type == VarType::VarUnknown || tv_check_for_dict_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a string or a number.
pub fn tv_check_for_string_or_number_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarString && vt != VarType::VarNumber {
        semsg!(tr(E_STRING_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a buffer number.
pub fn tv_check_for_buffer_arg(args: &[TypVal], idx: i32) -> bool {
    tv_check_for_string_or_number_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a line number.
pub fn tv_check_for_lnum_arg(args: &[TypVal], idx: i32) -> bool {
    tv_check_for_string_or_number_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a string or a list.
pub fn tv_check_for_string_or_list_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarString && vt != VarType::VarList {
        semsg!(tr(E_STRING_OR_LIST_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a string, a list or a blob.
pub fn tv_check_for_string_or_list_or_blob_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarString && vt != VarType::VarList && vt != VarType::VarBlob {
        semsg!(tr(E_STRING_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Check for an optional string or list argument at "idx".
pub fn tv_check_for_opt_string_or_list_arg(args: &[TypVal], idx: i32) -> bool {
    args[idx as usize].v_type == VarType::VarUnknown || tv_check_for_string_or_list_arg(args, idx)
}

/// Give an error and return FAIL unless "args[idx]" is a string or a function reference.
pub fn tv_check_for_string_or_func_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarPartial && vt != VarType::VarFunc && vt != VarType::VarString {
        semsg!(tr(E_STRING_OR_FUNCTION_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Give an error and return FAIL unless "args[idx]" is a list or a blob.
pub fn tv_check_for_list_or_blob_arg(args: &[TypVal], idx: i32) -> bool {
    let vt = args[idx as usize].v_type;
    if vt != VarType::VarList && vt != VarType::VarBlob {
        semsg!(tr(E_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR), idx + 1,);
        return false;
    }
    true
}

/// Get the string value of a "stringish" Vimscript object.
pub fn tv_get_string_buf_chk<'a>(
    tv: &'a TypVal,
    buf: &'a mut [u8; NUMBUFLEN],
) -> Option<&'a std::ffi::CStr> {
    match tv.v_type {
        VarType::VarNumber => {
            use std::io::Write;
            let mut slice = &mut buf[..];
            let _ = write!(slice, "{}\0", tv.vval.v_number());
            Some(crate::memory::bytes_as_cstr(buf))
        }
        VarType::VarFloat => {
            format_to_buf!(buf, c"%g", tv.vval.v_float());
            Some(crate::memory::bytes_as_cstr(buf))
        }
        VarType::VarString => {
            if !tv.vval.v_string().is_null() {
                Some(tv.vval.v_string_cstr_or_empty())
            } else {
                Some(c"")
            }
        }
        VarType::VarBool => {
            crate::memory::copy_bytes_to_c_buf(
                buf,
                ENCODE_BOOL_VAR_NAMES[tv.vval.v_bool() as usize].to_bytes(),
            );
            Some(crate::memory::bytes_as_cstr(buf))
        }
        VarType::VarSpecial => {
            crate::memory::copy_bytes_to_c_buf(
                buf,
                ENCODE_SPECIAL_VAR_NAMES[tv.vval.v_special() as usize].to_bytes(),
            );
            Some(crate::memory::bytes_as_cstr(buf))
        }
        VarType::VarPartial
        | VarType::VarFunc
        | VarType::VarList
        | VarType::VarDict
        | VarType::VarBlob
        | VarType::VarUnknown => {
            emsg(tr(str_error(tv.v_type)));
            None
        }
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub fn tv_get_string_chk(tv: &TypVal) -> Option<&std::ffi::CStr> {
    tv_get_string_buf_chk(tv, &mut tv_st().borrow_mut().mybuf_chk)
}

/// Get the string value of a "stringish" Vimscript object.
pub fn tv_get_string(tv: &TypVal) -> &std::ffi::CStr {
    tv_get_string_buf(tv, &mut tv_st().borrow_mut().mybuf)
}

/// Get the string value of a "stringish" Vimscript object.
pub fn tv_get_string_buf<'a>(tv: &'a TypVal, buf: &'a mut [u8; NUMBUFLEN]) -> &'a std::ffi::CStr {
    tv_get_string_buf_chk(tv, buf).unwrap_or(c"")
}

/// Return true when "tv" is not falsy.
pub fn tv2bool(tv: &TypVal) -> bool {
    match tv.v_type {
        VarType::VarNumber => tv.vval.v_number() != 0,
        VarType::VarFloat => tv.vval.v_float() != 0.0,
        VarType::VarPartial => !tv.vval.v_partial().is_null(),
        VarType::VarFunc | VarType::VarString => {
            !tv.vval.v_string().is_null() && !tv.vval.v_string_is_empty()
        }
        VarType::VarList => tv.vval.v_list_opt().is_some_and(|l| l.lv_len > 0),
        VarType::VarDict => tv
            .vval
            .v_dict_opt()
            .is_some_and(|d| d.dv_hashtab.ht_used > 0),
        VarType::VarBool => tv.vval.v_bool() == K_BOOL_VAR_TRUE_CONST,
        VarType::VarSpecial => tv.vval.v_special() != K_SPECIAL_VAR_NULL_CONST,
        VarType::VarBlob => tv.vval.v_blob_opt().is_some_and(|b| !b.bv_ga.is_empty()),
        VarType::VarUnknown => false,
    }
}

/// Check that repeating "slen" items "n" times does not overflow int.
pub fn check_repeat_count(slen: Varnumber, n: Varnumber) -> bool {
    if n <= i64::from(i32::MAX) / slen {
        return true;
    }

    let mut buf: [u8; NUMBUFLEN] = [0; NUMBUFLEN];
    format_to_buf!(&mut buf, c"%lld", n);
    semsg!(
        tr(E_VAL_TOO_LARGE),
        crate::strings::ByteSlice::from_ptr(buf.as_ptr().cast::<()>())
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tv_list_operations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            crate::globals::g_st().emsg_skip += 1;

            // Basic alloc, append_number, find, reverse
            let l = tv_list_alloc(0);
            l.lv_refcount = 1;
            tv_list_append_number(&mut *l, 10);
            tv_list_append_number(&mut *l, 20);
            tv_list_append_number(&mut *l, 30);
            assert_eq!(tv_list_len_inline(Some(&*l)), 3);

            let item1 = tv_list_find(Some(&mut *l), 1).unwrap();
            assert_eq!(item1.li_tv.vval.v_number(), 20);
            assert_eq!(tv_list_idx_of_item(Some(&*l), Some(item1)), 1);
            assert_eq!(tv_list_idx_of_item(Some(&*l), None), -1);

            let mut err = false;
            assert_eq!(tv_list_find_nr(Some(&mut *l), 0, Some(&mut err)), 10);
            assert!(!err);
            assert_eq!(tv_list_find_nr(Some(&mut *l), -1, Some(&mut err)), 30);
            assert!(!err);
            assert_eq!(tv_list_find_nr(Some(&mut *l), 10, Some(&mut err)), -1);
            assert!(err);

            tv_list_reverse(&mut *l);
            let first = tv_list_find(Some(&mut *l), 0).unwrap();
            assert_eq!(first.li_tv.vval.v_number(), 30);

            // Watchers + item_remove + drop_items + remove_items
            let lw1 = std::rc::Rc::new(Listwatch {
                lw_item: std::cell::Cell::new(l.lv_first),
            });
            let lw2 = std::rc::Rc::new(Listwatch {
                lw_item: std::cell::Cell::new(l.lv_last),
            });
            tv_list_watch_add(&mut *l, lw1.clone());
            tv_list_watch_add(&mut *l, lw2.clone());

            let first = l.lv_first;
            let second = first.li_next.as_ptr();
            let next_after_first = tv_list_item_remove(l, first.borrow_mut());
            assert_eq!(
                next_after_first.map_or(ptr::null_mut(), ptr::from_mut),
                second
            );
            assert_eq!(lw1.lw_item.get(), second);
            assert_eq!(tv_list_len_inline(Some(&*l)), 2);

            tv_list_watch_remove(&mut *l, &lw1);
            tv_list_watch_remove(&mut *l, &lw2);
            tv_list_unref(Some(l));

            // insert, insert_tv, append_string, append_allocated_string, append_tv, append_owned_tv
            let l2 = tv_list_alloc(0);
            l2.lv_refcount = 1;
            tv_list_append_string(&mut *l2, Some(c"hello"), 5);
            tv_list_append_string(&mut *l2, Some(c"world"), -1);
            let mut tv_ins = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(99),
            };
            let first2 = l2.first_opt();
            tv_list_insert_tv(l2, &mut tv_ins, first2);
            assert_eq!(tv_list_len_inline(Some(&*l2)), 3);
            assert_eq!(tv_list_find_nr(Some(&mut *l2), 0, None), 99);
            assert_eq!(tv_list_find_str(Some(&mut *l2), 1), Some(c"hello"));
            assert_eq!(tv_list_find_str(Some(&mut *l2), 2), Some(c"world"));

            // copy, extend, concat, join, equal
            let mut l2_copy = tv_list_copy(None, Some(&mut *l2), true, 0);
            l2_copy.as_deref_mut().unwrap().lv_refcount = 1;
            assert!(tv_list_equal(Some(&*l2), l2_copy.as_deref(), false));
            assert!(tv_list_equal(None, None, false));
            assert!(!tv_list_equal(Some(&*l2), None, false));

            let mut joined: Vec<u8> = Vec::new();
            assert!(tv_list_join(&mut joined, Some(&mut *l2), c", "));
            assert_eq!(joined.as_slice(), b"99, hello, world");

            let mut cat_tv = tv_initial_value();
            assert!(tv_list_concat(
                Some(&mut *l2),
                l2_copy.as_deref_mut(),
                &mut cat_tv
            ));
            assert_eq!(tv_list_len_inline(cat_tv.vval.v_list_opt().as_deref()), 6);
            tv_clear(&mut cat_tv);

            tv_list_extend(&mut *l2, l2_copy.as_deref(), None);
            assert_eq!(tv_list_len_inline(Some(&*l2)), 6);

            // remove_items range
            let first_item = l2.lv_first;
            let second_item = first_item.li_next.as_ptr();
            tv_list_remove_items(
                &mut *l2,
                first_item.borrow_mut(),
                crate::globals::GraphPtr::of(second_item).borrow_mut(),
            );
            assert_eq!(tv_list_len_inline(Some(&*l2)), 4);

            tv_list_unref(l2_copy);
            tv_list_unref(Some(l2));
            crate::globals::g_st().emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv_dict_operations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            crate::globals::g_st().emsg_skip += 1;

            let d = tv_dict_alloc();
            d.dv_refcount = 1;
            assert!(tv_dict_add_nr(&mut *d, c"foo", 42));
            assert!(tv_dict_add_float(&mut *d, c"flt", 3.5));
            assert!(tv_dict_add_str(&mut *d, c"str", Some(c"bar")));
            assert!(tv_dict_add_str_len(&mut *d, c"alloc", Some(b"baz")));
            assert!(tv_dict_has_key(Some(&*d), c"foo"));
            assert!(!tv_dict_has_key(Some(&*d), c"missing"));

            assert_eq!(tv_dict_get_number(Some(&*d), c"foo"), 42);
            assert_eq!(tv_dict_get_number(Some(&*d), c"missing"), 0);

            let mut numbuf = [0u8; NUMBUFLEN];
            assert_eq!(tv_dict_get_string(Some(&*d), c"str"), Some(c"bar"));
            let saved = tv_dict_get_cstring(Some(&*d), c"foo").unwrap();
            assert_eq!(saved.as_bytes(), b"42");

            assert_eq!(
                tv_dict_get_string_buf_chk(Some(&*d), c"missing", 7, &mut numbuf, Some(c"def")),
                Some(c"def")
            );

            // Copy, equal, extend, readonly, clear
            let d_copy = tv_dict_copy(None, Some(&mut *d), true, 0);
            let d_copy_ref = d_copy.unwrap();
            d_copy_ref.dv_refcount = 1;
            assert!(tv_dict_equal(Some(&*d), Some(d_copy_ref), false));
            assert!(tv_dict_equal(None, None, false));
            assert!(!tv_dict_equal(Some(&*d), None, false));

            let d2 = tv_dict_alloc();
            d2.dv_refcount = 1;
            assert!(tv_dict_add_nr(&mut *d2, c"extra", 7));
            tv_dict_extend(&mut *d, &mut *d2, b"force");
            assert_eq!(tv_dict_get_number(Some(&*d), c"extra"), 7);
            assert!(!tv_dict_equal(Some(&*d), Some(d_copy_ref), false));

            tv_dict_set_keys_readonly(&mut *d2);
            let di_extra = tv_dict_find_opt(Some(&*d2), c"extra", -1).unwrap();
            assert_ne!(di_extra.di_flags & (DI_FLAGS_RO | DI_FLAGS_FIX), 0);

            tv_dict_clear(d_copy_ref);
            assert_eq!(tv_dict_len_inline(Some(d_copy_ref)), 0);

            tv_dict_unref(Some(d2));
            tv_dict_unref(Some(d_copy_ref));
            tv_dict_unref(Some(d));
            crate::globals::g_st().emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv2bool_and_typval_getters_locks_and_checks() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            crate::globals::g_st().emsg_skip += 1;

            let mut tv = tv_initial_value();
            tv.v_type = VarType::VarNumber;
            tv.vval = crate::eval::typval::TypvalVval::Number(0);
            assert!(!tv2bool(&tv));
            tv.vval = crate::eval::typval::TypvalVval::Number(5);
            assert!(tv2bool(&tv));

            // tv_get_number, tv_get_float, tv_get_string, tv_equal
            assert_eq!(tv_get_number(&tv), 5);
            assert_eq!(tv_get_float(&tv), 5.0);
            let mut buf = [0u8; NUMBUFLEN];
            assert_eq!(tv_get_string_buf(&tv, &mut buf), c"5");

            let tv_flt = TypVal {
                v_type: VarType::VarFloat,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Float(5.0),
            };
            assert!(!tv_equal(&tv, &tv_flt, false));
            assert!(tv_equal(&tv_flt, &tv_flt, false));
            assert_eq!(tv_get_float(&tv_flt), 5.0);

            let mut tv_str = tv_initial_value();
            tv_set_string(&mut tv_str, Some(c"hello"));
            assert!(tv_check_str_or_nr(&tv_str));
            assert!(tv_check_num(&tv));
            assert!(!tv_check_num(&tv_flt));
            assert!(tv_check_str(&tv_str));

            // tv_list_alloc_ret, tv_dict_alloc_ret, tv_item_lock, tv_islocked, tv_check_lock
            let mut tv_l = tv_initial_value();
            tv_list_alloc_ret(&mut tv_l, 0);
            tv_list_append_number(tv_l.vval.v_list_ref(), 1);
            assert!(!tv_islocked(&tv_l));
            tv_item_lock(&mut tv_l, 2, true, false);
            assert!(tv_islocked(&tv_l));
            assert!(tv_check_lock(&tv_l, Some(c"test"), 4));
            tv_item_lock(&mut tv_l, 2, false, false);
            assert!(!tv_islocked(&tv_l));

            let mut tv_d = tv_initial_value();
            tv_dict_alloc_ret(&mut tv_d);
            assert_eq!(tv_d.v_type, VarType::VarDict);

            tv_clear(&mut tv_str);
            tv_clear(&mut tv_l);
            tv_clear(&mut tv_d);
            crate::globals::g_st().emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv_clear_shared_lists_and_dicts() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            // [&l [1], *l, *l]
            let inner = tv_list_alloc(1);
            tv_list_append_number(&mut *inner, 1);
            let outer = tv_list_alloc(3);
            outer.lv_refcount = 1;
            for _ in 0..3 {
                tv_list_append_list(&mut *outer, Some(&mut *inner));
            }
            assert_eq!(inner.lv_refcount, 3);
            let mut tv = TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(crate::globals::GraphPtr::new(outer)),
            };
            tv_clear(&mut tv);
            assert!(tv.vval.v_list().is_null());

            // [&l [], *l, *l]
            let empty_inner = tv_list_alloc(0);
            let outer2 = tv_list_alloc(3);
            outer2.lv_refcount = 1;
            for _ in 0..3 {
                tv_list_append_list(&mut *outer2, Some(&mut *empty_inner));
            }
            assert_eq!(empty_inner.lv_refcount, 3);
            let mut tv2 = TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(crate::globals::GraphPtr::new(outer2)),
            };
            tv_clear(&mut tv2);
            assert!(tv2.vval.v_list().is_null());

            // [&d {}, *d] and [&d {a: 1}, *d]
            for with_item in [false, true] {
                let d_inner = tv_dict_alloc();
                if with_item {
                    assert!(tv_dict_add_nr(&mut *d_inner, c"a", 1));
                }
                let outer_d = tv_list_alloc(2);
                outer_d.lv_refcount = 1;
                tv_list_append_dict(&mut *outer_d, Some(&mut *d_inner));
                tv_list_append_dict(&mut *outer_d, Some(&mut *d_inner));
                assert_eq!(d_inner.dv_refcount, 2);
                let mut tv_d = TypVal {
                    v_type: VarType::VarList,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(crate::globals::GraphPtr::new(outer_d)),
                };
                tv_clear(&mut tv_d);
                assert!(tv_d.vval.v_list().is_null());
            }
        }
    }
}
