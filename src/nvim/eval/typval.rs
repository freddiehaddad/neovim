//! Vimscript `typval_T`, `list_T`, `dict_T`, `blob_T`, and `Callback` operations (`src/nvim/eval/typval.c`).

use crate::charset::{NUL, vim_str2nr};
use crate::eval::userfunc::{
    call_func, func_ref, func_unref, funcexe_T, get_funccal_local_ht, ufunc_T,
};

use crate::cursor::linenr_T;
use crate::errors::*;

use crate::buffer::{DO_NOT_FREE_CNT, NOTDONE, VAR_FIXED, buf_T};
use crate::eval::encode::{
    MPConvPartialStage, MPConvStackData, MPConvStackVal, encode_bool_var_names,
    encode_special_var_names, encode_tv2echo, encode_tv2string,
};
use crate::eval::executor::eexe_mod_op;
use crate::eval::gc::{gc_first_dict, gc_first_list};
use crate::eval::vars::{
    get_globvar_dict, valid_varname, var_check_fixed, var_check_ro, var_wrong_func_name,
};
use crate::eval::{
    DICT_MAXNEST, STR2NR_ALL, callback_call, callback_from_typval, func_equal, get_copyID,
    kListLenMayKnow, partial_name, partial_unref, set_selfdict, var_item_copy, var2fpos,
};
use crate::globals::*;
use crate::hashtab::{
    hash_add, hash_clear, hash_find, hash_find_len, hash_lock, hash_remove, hash_unlock,
    hashitem_T, hashitem_empty,
};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_free_luaref, api_new_luaref, nlua_funcref_str};
use crate::mbyte::{mb_strcmp_ic, string_convert, utf_char2bytes, utfc_ptr2len, vimconv_T};
use crate::memory::{cstr_from_bytes, cstr_to_cstring, strequal, xstrlcpy};
use crate::message::{emsg, internal_error, semsg};
use crate::os::input::{fast_breakcheck, line_breakcheck};
use crate::strings::{
    sort_slice_by, strcmp, strcoll, strlen, strncmp, strtod, tr, vim_snprintf, vim_stricmp,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::api::private::helpers::{DI_FLAGS_FIX, DI_FLAGS_RO};
use crate::cmdexpand::MB_MAXBYTES;
use crate::digraph::{kBoolVarFalse, kBoolVarTrue};

use crate::eval::funcs::kSpecialVarNull;
use crate::math::{FAIL, OK};
use crate::regexp::NUMBUFLEN;
use crate::viml::parser::parser::CONV_NONE;
pub type float_T = f64;
pub type varnumber_T = i64;
pub type VarLockStatus = c_int;
pub type BoolVarValue = c_int;

#[derive(Clone, Copy, Default)]
pub enum EvalFuncData {
    #[default]
    None,
    FloatOp(fn(float_T) -> float_T),
    ApiHandler(&'static crate::api::private::dispatch::MsgpackRpcRequestHandler),
    LuaFunc(&'static CStr),
}

pub(crate) const VAR_UNLOCKED: c_int = 0;
pub const kCallbackNone: c_int = 0;
pub const kCallbackFuncref: c_int = 1;
pub const kCallbackPartial: c_int = 2;
pub const kCallbackLua: c_int = 3;
pub const kCallbackExpr: c_int = 4;

#[repr(C)]
pub struct blob_T {
    pub bv_ga: Vec<u8>,
    pub bv_refcount: c_int,
    pub bv_lock: c_int,
}

#[repr(C)]
pub struct partial_T {
    pub pt_refcount: c_int,
    pub pt_copyID: c_int,
    pub pt_name: Option<std::ffi::CString>,
    pub pt_func: *mut ufunc_T,
    pub pt_auto: bool,
    pub pt_argv: Vec<typval_T>,
    pub pt_dict: *mut dict_T,
}

impl Default for partial_T {
    fn default() -> Self {
        Self {
            pt_refcount: 0,
            pt_copyID: 0,
            pt_name: None,
            pt_func: ptr::null_mut(),
            pt_auto: false,
            pt_argv: Vec::new(),
            pt_dict: ptr::null_mut(),
        }
    }
}

impl Drop for partial_T {
    fn drop(&mut self) {
        unsafe {
            for tv in &mut self.pt_argv {
                tv_clear(tv);
            }
            tv_dict_unref(self.pt_dict);
            if let Some(ref name) = self.pt_name {
                func_unref(name.as_ptr().cast_mut());
            } else {
                crate::eval::userfunc::func_ptr_unref(self.pt_func.cast());
            }
        }
    }
}

#[derive(Clone)]
pub enum CallbackData {
    None,
    Funcref(std::ffi::CString),
    Partial(*mut partial_T),
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
    pub const fn partial(partial: *mut partial_T) -> Self {
        Self::Partial(partial)
    }

    #[inline]
    pub const fn lua(luaref: LuaRef) -> Self {
        Self::Lua(luaref)
    }

    #[inline]
    pub fn as_funcref(&self) -> *mut c_char {
        match self {
            Self::Funcref(s) | Self::Expr(s) => s.as_ptr().cast_mut(),
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn as_partial(&self) -> *mut partial_T {
        match self {
            Self::Partial(p) => *p,
            _ => ptr::null_mut(),
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
    pub const fn cb_type(&self) -> c_int {
        match self.data {
            CallbackData::None => kCallbackNone,
            CallbackData::Funcref(_) => kCallbackFuncref,
            CallbackData::Partial(_) => kCallbackPartial,
            CallbackData::Lua(_) => kCallbackLua,
            CallbackData::Expr(_) => kCallbackExpr,
        }
    }

    #[inline]
    pub unsafe fn raw_cb_type(cb: *const Self) -> c_int {
        unsafe { (*cb).cb_type() }
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
    VAR_UNKNOWN = 0,
    VAR_NUMBER = 1,
    VAR_STRING = 2,
    VAR_FUNC = 3,
    VAR_LIST = 4,
    VAR_DICT = 5,
    VAR_FLOAT = 6,
    VAR_BOOL = 7,
    VAR_SPECIAL = 8,
    VAR_PARTIAL = 9,
    VAR_BLOB = 10,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub enum TypvalVval {
    #[default]
    None,
    Number(i64),
    Bool(c_int),
    Special(c_int),
    Float(f64),
    String(*mut c_char),
    List(*mut list_T),
    Dict(*mut dict_T),
    Blob(*mut blob_T),
    Partial(*mut partial_T),
}

impl TypvalVval {
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
    pub const fn v_bool(self) -> c_int {
        match self {
            Self::Bool(b) | Self::Special(b) => b,
            Self::Number(n) => n as c_int,
            _ => 0,
        }
    }

    #[inline]
    pub const fn v_special(self) -> c_int {
        match self {
            Self::Special(s) | Self::Bool(s) => s,
            Self::Number(n) => n as c_int,
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

    #[inline]
    pub const fn v_string(self) -> *mut c_char {
        match self {
            Self::String(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn v_list(self) -> *mut list_T {
        match self {
            Self::List(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn v_dict(self) -> *mut dict_T {
        match self {
            Self::Dict(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub fn v_dict_mut(&mut self) -> &mut *mut dict_T {
        if !matches!(self, Self::Dict(_)) {
            *self = Self::Dict(ptr::null_mut());
        }
        match self {
            Self::Dict(p) => p,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn v_blob(self) -> *mut blob_T {
        match self {
            Self::Blob(p) => p,
            _ => ptr::null_mut(),
        }
    }

    #[inline]
    pub const fn v_partial(self) -> *mut partial_T {
        match self {
            Self::Partial(p) => p,
            _ => ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct typval_T {
    pub v_type: VarType,
    pub v_lock: c_int,
    pub vval: TypvalVval,
}

impl typval_T {
    #[inline]
    pub const fn new() -> Self {
        tv_initial_value()
    }
}

impl Default for typval_T {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct listitem_T {
    pub li_next: *mut listitem_T,
    pub li_prev: *mut listitem_T,
    pub li_tv: typval_T,
}

impl listitem_T {
    pub const fn new() -> Self {
        Self {
            li_next: ptr::null_mut(),
            li_prev: ptr::null_mut(),
            li_tv: tv_initial_value(),
        }
    }
}

impl Default for listitem_T {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for listitem_T {
    fn drop(&mut self) {
        unsafe {
            tv_clear(&raw mut self.li_tv);
        }
    }
}

#[repr(C)]
pub struct list_T {
    pub lv_first: *mut listitem_T,
    pub lv_last: *mut listitem_T,
    pub lv_watch: Vec<*mut listwatch_T>,
    pub lv_idx_item: *mut listitem_T,
    pub lv_copylist: *mut list_T,
    pub lv_used_next: *mut list_T,
    pub lv_used_prev: *mut list_T,
    pub lv_refcount: c_int,
    pub lv_len: c_int,
    pub lv_idx: c_int,
    pub lv_copyID: c_int,
    pub lv_lock: c_int,
    pub lua_table_ref: c_int,
}

impl list_T {
    pub const fn new() -> Self {
        Self {
            lv_first: ptr::null_mut(),
            lv_last: ptr::null_mut(),
            lv_watch: Vec::new(),
            lv_idx_item: ptr::null_mut(),
            lv_copylist: ptr::null_mut(),
            lv_used_next: ptr::null_mut(),
            lv_used_prev: ptr::null_mut(),
            lv_refcount: 0,
            lv_len: 0,
            lv_idx: 0,
            lv_copyID: 0,
            lv_lock: 0,
            lua_table_ref: LUA_NOREF,
        }
    }
}

impl Drop for list_T {
    fn drop(&mut self) {
        unsafe {
            let self_ptr = std::ptr::from_mut(self);
            if !self.lv_used_prev.is_null() || ptr::eq(gc_first_list, self_ptr) {
                if self.lv_used_prev.is_null() {
                    gc_first_list = self.lv_used_next;
                } else {
                    (*self.lv_used_prev).lv_used_next = self.lv_used_next;
                }
                if !self.lv_used_next.is_null() {
                    (*self.lv_used_next).lv_used_prev = self.lv_used_prev;
                }
            }
            nlua_clear_ref(&mut self.lua_table_ref);
        }
    }
}

#[repr(C)]
pub struct dictitem_T {
    pub di_tv: typval_T,
    pub di_flags: u8,
    pub di_key: Vec<c_char>,
}

impl dictitem_T {
    pub const fn new() -> Self {
        Self {
            di_tv: tv_initial_value(),
            di_flags: 0,
            di_key: Vec::new(),
        }
    }
}

impl Drop for dictitem_T {
    fn drop(&mut self) {
        if (self.di_flags & DI_FLAGS_ALLOC) != 0 {
            unsafe {
                tv_clear(&raw mut self.di_tv);
            }
        }
    }
}

pub type ScopeDictDictItem = dictitem_T;

#[repr(C)]
pub struct dict_T {
    pub dv_lock: c_int,
    pub dv_scope: c_int,
    pub dv_refcount: c_int,
    pub dv_copyID: c_int,
    pub dv_hashtab: crate::hashtab::hashtab_T,
    pub dv_copydict: *mut dict_T,
    pub dv_used_next: *mut dict_T,
    pub dv_used_prev: *mut dict_T,
    pub watchers: Vec<DictWatcher>,
    pub lua_table_ref: c_int,
}

impl dict_T {
    pub const fn new() -> Self {
        Self {
            dv_lock: 0,
            dv_scope: 0,
            dv_refcount: 0,
            dv_copyID: 0,
            dv_hashtab: crate::hashtab::hashtab_T::new(),
            dv_copydict: ptr::null_mut(),
            dv_used_next: ptr::null_mut(),
            dv_used_prev: ptr::null_mut(),
            watchers: Vec::new(),
            lua_table_ref: LUA_NOREF,
        }
    }
}

impl Drop for dict_T {
    fn drop(&mut self) {
        unsafe {
            let self_ptr = std::ptr::from_mut(self);
            if !self.dv_used_prev.is_null() || ptr::eq(gc_first_dict, self_ptr) {
                if self.dv_used_prev.is_null() {
                    gc_first_dict = self.dv_used_next;
                } else {
                    (*self.dv_used_prev).dv_used_next = self.dv_used_next;
                }
                if !self.dv_used_next.is_null() {
                    (*self.dv_used_next).dv_used_prev = self.dv_used_prev;
                }
            }
            nlua_clear_ref(&mut self.lua_table_ref);
        }
    }
}

const ITEM_COMPARE_FAIL: c_int = 999;

pub(crate) const VAR_LOCKED: VarLockStatus = 1;

const VAR_NO_SCOPE: c_int = 0;

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
pub struct listwatch_T {
    pub lw_item: *mut listitem_T,
}

#[repr(C)]
pub struct staticList10_T {
    pub sl_list: list_T,
    pub sl_items: [listitem_T; 10],
}

impl staticList10_T {
    pub const fn new() -> Self {
        Self {
            sl_list: list_T::new(),
            sl_items: [const { listitem_T::new() }; 10],
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
        unsafe {
            callback_free(&raw mut self.callback);
        }
    }
}

#[repr(C)]
struct sortinfo_T {
    item_compare_ic: c_int,
    item_compare_lc: bool,
    item_compare_numeric: bool,
    item_compare_numbers: bool,
    item_compare_float: bool,
    item_compare_func: *const c_char,
    item_compare_partial: *mut partial_T,
    item_compare_selfdict: *mut dict_T,
    item_compare_func_err: bool,
    item_compare_keys_ready: bool,
}

impl sortinfo_T {
    const fn new() -> Self {
        Self {
            item_compare_ic: 0,
            item_compare_lc: false,
            item_compare_numeric: false,
            item_compare_numbers: false,
            item_compare_float: false,
            item_compare_func: ptr::null(),
            item_compare_partial: ptr::null_mut(),
            item_compare_selfdict: ptr::null_mut(),
            item_compare_func_err: false,
            item_compare_keys_ready: false,
        }
    }
}

impl Default for sortinfo_T {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum ListSortKey {
    Int(varnumber_T),
    Float(f64),
}

impl ListSortKey {
    #[inline]
    fn as_int(self) -> varnumber_T {
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
    item: *mut listitem_T,
    idx: c_int,
    key: ListSortKey,
}

type ListSorter = unsafe fn(&ListSortItem, &ListSortItem) -> c_int;

pub static mut tv_in_free_unref_items: bool = false;

static TV_EMPTY_STRING_BYTE: c_char = 0;
pub const tv_empty_string: *const c_char = &raw const TV_EMPTY_STRING_BYTE;

static mut SORTINFO: *mut sortinfo_T = ptr::null_mut();
static mut TV_EQUAL_RECURSE_LIMIT: c_int = 0;

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
pub unsafe fn nlua_clear_ref(r: &mut LuaRef) {
    if *r != LUA_NOREF && *r != 0 {
        unsafe { api_free_luaref(*r) };
        *r = LUA_NOREF;
    }
}

#[inline]
pub unsafe fn tv_dict_hi2di(hi: *const hashitem_T) -> *mut dictitem_T {
    unsafe { (*hi).hi_di }
}

#[inline]
pub unsafe fn tv_list_set_ret(tv: *mut typval_T, l: *mut list_T) {
    unsafe { tv_list_set_ret_inline(tv, l) }
}

#[inline]
pub unsafe fn tv_alloc_string(s: *const c_char) -> *mut c_char {
    unsafe { CStr::from_ptr(s).to_owned().into_raw() }
}

#[inline]
pub fn tv_alloc_string_bytes(bytes: &[u8]) -> *mut c_char {
    crate::memory::cstr_from_bytes(bytes).into_raw()
}

#[inline]
pub unsafe fn tv_free_string(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            drop(std::ffi::CString::from_raw(p));
        }
    }
}

#[inline]
pub unsafe fn tv_take_string(p: *mut c_char) -> Option<std::ffi::CString> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { std::ffi::CString::from_raw(p) })
    }
}

#[inline]
pub unsafe fn tv_set_string(tv: *mut typval_T, s: *const c_char) {
    unsafe {
        (*tv).v_type = VarType::VAR_STRING;
        (*tv).vval = crate::eval::typval::TypvalVval::String(if s.is_null() {
            ptr::null_mut()
        } else {
            tv_alloc_string(s)
        });
    }
}

#[inline]
pub unsafe fn tv_set_string_len(tv: *mut typval_T, s: *const c_char, len: usize) {
    unsafe {
        (*tv).v_type = VarType::VAR_STRING;
        (*tv).vval = crate::eval::typval::TypvalVval::String(if s.is_null() {
            ptr::null_mut()
        } else {
            tv_alloc_string_bytes(std::slice::from_raw_parts(s.cast::<u8>(), len))
        });
    }
}

#[inline]
pub unsafe fn tv_blob_get(b: *const blob_T, idx: c_int) -> u8 {
    unsafe { tv_blob_get_inline(b, idx) }
}

#[inline]
pub const fn tv_is_func(tv: typval_T) -> bool {
    tv_is_func_inline(tv)
}

#[inline]
unsafe fn tv_list_ref_inline(l: *mut list_T) {
    if !l.is_null() {
        unsafe {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_list_set_ret_inline(tv: *mut typval_T, l: *mut list_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).vval = crate::eval::typval::TypvalVval::List(l);
        tv_list_ref_inline(l);
    }
}

#[inline]
unsafe fn tv_list_locked_inline(l: *const list_T) -> VarLockStatus {
    if l.is_null() {
        VAR_FIXED
    } else {
        unsafe { (*l).lv_lock }
    }
}

#[inline]
pub unsafe fn tv_list_ref(l: *mut list_T) {
    if !l.is_null() {
        unsafe {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
pub unsafe fn tv_list_set_lock(l: *mut list_T, lock: VarLockStatus) {
    unsafe { tv_list_set_lock_inline(l, lock) }
}

#[inline]
pub unsafe fn tv_blob_len(b: *const blob_T) -> c_int {
    if b.is_null() {
        0
    } else {
        unsafe { (*b).bv_ga.len() as c_int }
    }
}

#[inline]
unsafe fn tv_list_set_lock_inline(l: *mut list_T, lock: VarLockStatus) {
    if l.is_null() {
        debug_assert_eq!(lock, VAR_FIXED);
        return;
    }
    unsafe {
        (*l).lv_lock = lock;
    }
}

#[inline]
pub unsafe fn tv_list_len(l: *const list_T) -> c_int {
    unsafe { tv_list_len_inline(l) }
}

#[inline]
pub unsafe fn tv_dict_len(d: *const dict_T) -> i64 {
    if d.is_null() {
        0
    } else {
        unsafe { (*d).dv_hashtab.ht_used as i64 }
    }
}

#[inline]
pub unsafe fn tv_list_first(l: *const list_T) -> *mut listitem_T {
    unsafe { tv_list_first_inline(l) }
}

#[inline]
pub unsafe fn tv_list_last(l: *const list_T) -> *mut listitem_T {
    unsafe { tv_list_last_inline(l) }
}

#[inline]
unsafe fn tv_list_len_inline(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_uidx_inline(l: *const list_T, mut n: c_int) -> c_int {
    unsafe {
        let len = tv_list_len_inline(l);
        if n < 0 {
            n += len;
        }
        if n < 0 || n >= len { -1 } else { n }
    }
}

#[inline]
unsafe fn tv_list_first_inline(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

#[inline]
unsafe fn tv_list_last_inline(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_last }
    }
}

#[inline]
unsafe fn tv_dict_set_ret_inline(tv: *mut typval_T, d: *mut dict_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval = crate::eval::typval::TypvalVval::Dict(d);
        if !d.is_null() {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_dict_len_inline(d: *const dict_T) -> isize {
    if d.is_null() {
        0
    } else {
        unsafe { (*d).dv_hashtab.ht_used as isize }
    }
}

#[inline]
unsafe fn tv_dict_is_watched_inline(d: *const dict_T) -> bool {
    !d.is_null() && unsafe { !(*d).watchers.is_empty() }
}

#[inline]
unsafe fn tv_blob_set_ret_inline(tv: *mut typval_T, b: *mut blob_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_BLOB;
        (*tv).vval = crate::eval::typval::TypvalVval::Blob(b);
        if !b.is_null() {
            (*b).bv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_blob_len_inline(b: *const blob_T) -> c_int {
    if b.is_null() {
        0
    } else {
        unsafe { (*b).bv_ga.len() as c_int }
    }
}

#[inline]
unsafe fn tv_blob_get_inline(b: *const blob_T, idx: c_int) -> u8 {
    unsafe { (&(*b).bv_ga)[idx as usize] }
}

#[inline]
unsafe fn tv_blob_set_inline(blob: *mut blob_T, idx: c_int, c: u8) {
    unsafe {
        (&mut (*blob).bv_ga)[idx as usize] = c;
    }
}

#[inline]
const fn tv_is_func_inline(tv: typval_T) -> bool {
    matches!(tv.v_type, VarType::VAR_FUNC | VarType::VAR_PARTIAL)
}

#[inline]
pub const fn tv_initial_value() -> typval_T {
    typval_T {
        v_type: VarType::VAR_UNKNOWN,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval::Number(0),
    }
}

/// Allocate a list item.
unsafe fn tv_list_item_alloc() -> *mut listitem_T {
    Box::into_raw(Box::new(listitem_T {
        li_next: ptr::null_mut(),
        li_prev: ptr::null_mut(),
        li_tv: typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        },
    }))
}

/// Remove a list item from a List and free it.
pub unsafe fn tv_list_item_remove(l: *mut list_T, item: *mut listitem_T) -> *mut listitem_T {
    unsafe {
        let next_item = (*item).li_next;
        tv_list_drop_items(l, item, item);
        drop(Box::from_raw(item));
        next_item
    }
}

/// Add a watcher to a list.
pub unsafe fn tv_list_watch_add(l: *mut list_T, lw: *mut listwatch_T) {
    unsafe {
        (*l).lv_watch.push(lw);
    }
}

/// Remove a watcher from a list.
pub unsafe fn tv_list_watch_remove(l: *mut list_T, lwrem: *mut listwatch_T) {
    unsafe {
        if let Some(pos) = (*l).lv_watch.iter().position(|&lw| lw == lwrem) {
            (*l).lv_watch.remove(pos);
        }
    }
}

/// Advance watchers to the next item.
unsafe fn tv_list_watch_fix(l: *mut list_T, item: *const listitem_T) {
    unsafe {
        for &lw in &(*l).lv_watch {
            if ptr::eq((*lw).lw_item, item) {
                (*lw).lw_item = (*item).li_next;
            }
        }
    }
}

/// Allocate an empty list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tv_list_alloc(_len: isize) -> *mut list_T {
    unsafe {
        let list = Box::into_raw(Box::new(list_T {
            lv_first: ptr::null_mut(),
            lv_last: ptr::null_mut(),
            lv_watch: Vec::new(),
            lv_idx_item: ptr::null_mut(),
            lv_copylist: ptr::null_mut(),
            lv_used_next: gc_first_list,
            lv_used_prev: ptr::null_mut(),
            lv_refcount: 0,
            lv_len: 0,
            lv_idx: 0,
            lv_copyID: 0,
            lv_lock: 0,
            lua_table_ref: LUA_NOREF,
        }));

        if !gc_first_list.is_null() {
            (*gc_first_list).lv_used_prev = list;
        }
        gc_first_list = list;
        list
    }
}

/// Initialize a static list with 10 items.
pub unsafe fn tv_list_init_static10(sl: *mut staticList10_T) {
    unsafe {
        const SL_SIZE: usize = 10;
        *sl = staticList10_T::new();
        let l = &raw mut (*sl).sl_list;
        let items = (&raw mut (*sl).sl_items).cast::<listitem_T>();

        (*l).lv_first = items;
        (*l).lv_last = items.add(SL_SIZE - 1);
        (*l).lv_refcount = DO_NOT_FREE_CNT;
        tv_list_set_lock_inline(l, VAR_FIXED);
        (*l).lv_len = 10;

        (*items).li_prev = ptr::null_mut();
        (*items).li_next = items.add(1);
        (*items.add(SL_SIZE - 1)).li_prev = items.add(SL_SIZE - 2);
        (*items.add(SL_SIZE - 1)).li_next = ptr::null_mut();

        for i in 1..(SL_SIZE - 1) {
            let li = items.add(i);
            (*li).li_prev = li.sub(1);
            (*li).li_next = li.add(1);
        }
    }
}

/// Initialize static list with undefined number of elements.
pub unsafe fn tv_list_init_static(l: *mut list_T) {
    unsafe {
        *l = list_T::new();
        (*l).lv_refcount = DO_NOT_FREE_CNT;
    }
}

/// Free items contained in a list.
pub unsafe fn tv_list_free_contents(l: *mut list_T) {
    unsafe {
        let mut item = (*l).lv_first;
        while !item.is_null() {
            (*l).lv_first = (*item).li_next;
            drop(Box::from_raw(item));
            item = (*l).lv_first;
        }
        (*l).lv_len = 0;
        (*l).lv_idx_item = ptr::null_mut();
        (*l).lv_last = ptr::null_mut();
        debug_assert!((*l).lv_watch.is_empty());
    }
}

/// Free a list, including all items it points to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tv_list_free(l: *mut list_T) {
    unsafe {
        if tv_in_free_unref_items {
            return;
        }

        tv_list_free_contents(l);
        drop(Box::from_raw(l));
    }
}

/// Unreference a list.
pub unsafe fn tv_list_unref(l: *mut list_T) {
    unsafe {
        if !l.is_null() {
            (*l).lv_refcount -= 1;
            if (*l).lv_refcount <= 0 {
                tv_list_free(l);
            }
        }
    }
}

/// Remove items "item" to "item2" from list "l".
pub unsafe fn tv_list_drop_items(l: *mut list_T, item: *mut listitem_T, item2: *mut listitem_T) {
    unsafe {
        let mut ip = item;
        while ip != (*item2).li_next {
            (*l).lv_len -= 1;
            tv_list_watch_fix(l, ip);
            ip = (*ip).li_next;
        }

        if (*item2).li_next.is_null() {
            (*l).lv_last = (*item).li_prev;
        } else {
            (*(*item2).li_next).li_prev = (*item).li_prev;
        }
        if (*item).li_prev.is_null() {
            (*l).lv_first = (*item2).li_next;
        } else {
            (*(*item).li_prev).li_next = (*item2).li_next;
        }
        (*l).lv_idx_item = ptr::null_mut();
    }
}

/// Like tv_list_drop_items, but also frees all removed items.
pub unsafe fn tv_list_remove_items(l: *mut list_T, item: *mut listitem_T, item2: *mut listitem_T) {
    unsafe {
        tv_list_drop_items(l, item, item2);
        let mut li = item;
        loop {
            let nli = (*li).li_next;
            drop(Box::from_raw(li));
            if li == item2 {
                break;
            }
            li = nli;
        }
    }
}

/// Move items "item" to "item2" from list "l" to the end of the list "tgt_l".
pub unsafe fn tv_list_move_items(
    l: *mut list_T,
    item: *mut listitem_T,
    item2: *mut listitem_T,
    tgt_l: *mut list_T,
    cnt: c_int,
) {
    unsafe {
        tv_list_drop_items(l, item, item2);
        (*item).li_prev = (*tgt_l).lv_last;
        (*item2).li_next = ptr::null_mut();
        if (*tgt_l).lv_last.is_null() {
            (*tgt_l).lv_first = item;
        } else {
            (*(*tgt_l).lv_last).li_next = item;
        }
        (*tgt_l).lv_last = item2;
        (*tgt_l).lv_len += cnt;
    }
}

/// Insert list item.
pub unsafe fn tv_list_insert(l: *mut list_T, ni: *mut listitem_T, item: *mut listitem_T) {
    unsafe {
        if item.is_null() {
            tv_list_append(l, ni);
        } else {
            (*ni).li_prev = (*item).li_prev;
            (*ni).li_next = item;
            if (*item).li_prev.is_null() {
                (*l).lv_first = ni;
                (*l).lv_idx += 1;
            } else {
                (*(*item).li_prev).li_next = ni;
                (*l).lv_idx_item = ptr::null_mut();
            }
            (*item).li_prev = ni;
            (*l).lv_len += 1;
        }
    }
}

/// Insert Vimscript value into a list.
pub unsafe fn tv_list_insert_tv(l: *mut list_T, tv: *mut typval_T, item: *mut listitem_T) {
    unsafe {
        let ni = tv_list_item_alloc();
        tv_copy(tv, &raw mut (*ni).li_tv);
        tv_list_insert(l, ni, item);
    }
}

/// Append item to the end of list.
pub unsafe fn tv_list_append(l: *mut list_T, item: *mut listitem_T) {
    unsafe {
        if (*l).lv_last.is_null() {
            (*l).lv_first = item;
            (*l).lv_last = item;
            (*item).li_prev = ptr::null_mut();
        } else {
            (*(*l).lv_last).li_next = item;
            (*item).li_prev = (*l).lv_last;
            (*l).lv_last = item;
        }
        (*l).lv_len += 1;
        (*item).li_next = ptr::null_mut();
    }
}

/// Append Vimscript value to the end of list.
pub unsafe fn tv_list_append_tv(l: *mut list_T, tv: *mut typval_T) {
    unsafe {
        let li = tv_list_item_alloc();
        tv_copy(tv, &raw mut (*li).li_tv);
        tv_list_append(l, li);
    }
}

/// Like tv_list_append_tv(), but tv is moved to a list.
pub unsafe fn tv_list_append_owned_tv(l: *mut list_T, tv: typval_T) -> *mut typval_T {
    unsafe {
        let li = tv_list_item_alloc();
        (*li).li_tv = tv;
        tv_list_append(l, li);
        &raw mut (*li).li_tv
    }
}

/// Append a list to a list as one item.
pub unsafe fn tv_list_append_list(l: *mut list_T, itemlist: *mut list_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(itemlist),
            },
        );
        tv_list_ref_inline(itemlist);
    }
}

/// Append a dictionary to a list.
pub unsafe fn tv_list_append_dict(l: *mut list_T, dict: *mut dict_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Dict(dict),
            },
        );
        if !dict.is_null() {
            (*dict).dv_refcount += 1;
        }
    }
}

/// Make a copy of "str" and append it as an item to list "l".
pub unsafe fn tv_list_append_string(l: *mut list_T, str_: *const c_char, len: isize) {
    unsafe {
        let v_string = if str_.is_null() {
            ptr::null_mut()
        } else if len >= 0 {
            tv_alloc_string_bytes(std::slice::from_raw_parts(str_.cast::<u8>(), len as usize))
        } else {
            tv_alloc_string(str_)
        };
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(v_string),
            },
        );
    }
}

/// Append number to the list.
pub unsafe fn tv_list_append_number(l: *mut list_T, n: varnumber_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(n),
            },
        );
    }
}

/// Make a copy of list.
pub unsafe fn tv_list_copy(
    conv: Option<&vimconv_T>,
    orig: *mut list_T,
    deep: bool,
    copyID: c_int,
) -> *mut list_T {
    unsafe {
        if orig.is_null() {
            return ptr::null_mut();
        }

        let copy = tv_list_alloc(tv_list_len_inline(orig) as isize);
        tv_list_ref_inline(copy);
        if copyID != 0 {
            (*orig).lv_copyID = copyID;
            (*orig).lv_copylist = copy;
        }
        let mut item = (*orig).lv_first;
        while !item.is_null() {
            if got_int {
                break;
            }
            let ni = tv_list_item_alloc();
            if deep {
                if var_item_copy(
                    conv,
                    &raw mut (*item).li_tv,
                    &raw mut (*ni).li_tv,
                    deep,
                    copyID,
                ) == FAIL
                {
                    drop(Box::from_raw(ni));
                    tv_list_unref(copy);
                    return ptr::null_mut();
                }
            } else {
                tv_copy(&raw const (*item).li_tv, &raw mut (*ni).li_tv);
            }
            tv_list_append(copy, ni);
            item = (*item).li_next;
        }

        copy
    }
}

/// Get the list item in "l" with index "n1".
pub unsafe fn tv_list_check_range_index_one(
    l: *mut list_T,
    n1: *mut c_int,
    quiet: bool,
) -> *mut listitem_T {
    unsafe {
        let li = tv_list_find_index(l, n1);
        if !li.is_null() {
            return li;
        }

        if !quiet {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n1));
        }
        ptr::null_mut()
    }
}

/// Check that "n2" can be used as the second index in a range of list "l".
pub unsafe fn tv_list_check_range_index_two(
    l: *mut list_T,
    n1: *mut c_int,
    li1: *const listitem_T,
    n2: *mut c_int,
    quiet: bool,
) -> c_int {
    unsafe {
        if *n2 < 0 {
            let ni = tv_list_find(l, *n2);
            if ni.is_null() {
                if !quiet {
                    semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n2));
                }
                return FAIL;
            }
            *n2 = tv_list_idx_of_item(l, ni);
        }

        if *n1 < 0 {
            *n1 = tv_list_idx_of_item(l, li1);
        }
        if *n2 < *n1 {
            if !quiet {
                semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n2));
            }
            return FAIL;
        }
        OK
    }
}

/// Assign values from list "src" into a range of "dest".
pub unsafe fn tv_list_assign_range(
    dest: *mut list_T,
    src: *mut list_T,
    idx1_arg: c_int,
    idx2: c_int,
    empty_idx2: bool,
    op: *const c_char,
    varname: *const c_char,
) -> c_int {
    unsafe {
        let mut idx1 = idx1_arg;
        let first_li = tv_list_find_index(dest, &raw mut idx1);
        let mut src_li: *mut listitem_T;

        let mut idx = idx1;
        let mut dest_li = first_li;
        src_li = tv_list_first_inline(src);
        while !src_li.is_null() && !dest_li.is_null() {
            if value_check_lock((*dest_li).li_tv.v_lock, varname, TV_CSTRING) {
                return FAIL;
            }
            src_li = (*src_li).li_next;
            if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
                break;
            }
            dest_li = (*dest_li).li_next;
            idx += 1;
        }

        idx = idx1;
        dest_li = first_li;
        src_li = tv_list_first_inline(src);
        while !src_li.is_null() {
            debug_assert!(!dest_li.is_null());
            if !op.is_null() && *op != b'=' as c_char {
                eexe_mod_op(&raw mut (*dest_li).li_tv, &raw mut (*src_li).li_tv, op);
            } else {
                tv_clear(&raw mut (*dest_li).li_tv);
                tv_copy(&raw const (*src_li).li_tv, &raw mut (*dest_li).li_tv);
            }
            src_li = (*src_li).li_next;
            if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
                break;
            }
            if (*dest_li).li_next.is_null() {
                tv_list_append_number(dest, 0);
                dest_li = tv_list_last_inline(dest);
            } else {
                dest_li = (*dest_li).li_next;
            }
            idx += 1;
        }
        if !src_li.is_null() {
            emsg(tr(c"E710: List value has more items than target".as_ptr()));
            return FAIL;
        }
        if if empty_idx2 {
            !dest_li.is_null() && !(*dest_li).li_next.is_null()
        } else {
            idx != idx2
        } {
            emsg(tr(c"E711: List value has not enough items".as_ptr()));
            return FAIL;
        }
        OK
    }
}

/// Flatten up to "maxitems" in "list", starting at "first" to depth "maxdepth".
pub unsafe fn tv_list_flatten(
    list: *mut list_T,
    first: *mut listitem_T,
    maxitems: i64,
    maxdepth: i64,
) {
    unsafe {
        if maxdepth == 0 {
            return;
        }

        let mut item = if first.is_null() {
            (*list).lv_first
        } else {
            first
        };
        let mut done: i64 = 0;

        while !item.is_null() && done < maxitems {
            let next = (*item).li_next;

            fast_breakcheck();
            if got_int {
                return;
            }
            if (*item).li_tv.v_type == VarType::VAR_LIST {
                let itemlist = (*item).li_tv.vval.v_list();

                tv_list_drop_items(list, item, item);
                tv_list_extend(list, itemlist, next);

                if maxdepth > 0 {
                    let sub_first = if (*item).li_prev.is_null() {
                        (*list).lv_first
                    } else {
                        (*(*item).li_prev).li_next
                    };
                    tv_list_flatten(list, sub_first, i64::from((*itemlist).lv_len), maxdepth - 1);
                }
                drop(Box::from_raw(item));
            }

            done += 1;
            item = next;
        }
    }
}

/// "items(blob)" function.
unsafe fn tv_blob2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let blob = (*argvars).vval.v_blob();

        tv_list_alloc_ret(rettv, tv_blob_len_inline(blob) as isize);

        for i in 0..tv_blob_len_inline(blob) {
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list(), l2);
            tv_list_append_number(l2, i64::from(i));
            tv_list_append_number(l2, i64::from(tv_blob_get_inline(blob, i)));
        }
    }
}

/// "items(dict)" function.
unsafe fn tv_dict2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Items);
    }
}

/// "items(list)" function.
unsafe fn tv_list2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let l = (*argvars).vval.v_list();

        tv_list_alloc_ret(rettv, tv_list_len_inline(l) as isize);
        if l.is_null() {
            return;
        }

        let mut idx: varnumber_T = 0;
        let mut li = (*l).lv_first;
        while !li.is_null() {
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list(), l2);
            tv_list_append_number(l2, idx);
            tv_list_append_tv(l2, &raw mut (*li).li_tv);
            idx += 1;
            li = (*li).li_next;
        }
    }
}

/// "items(string)" function.
unsafe fn tv_string2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let mut p: *const c_char = (*argvars).vval.v_string();

        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if p.is_null() {
            return;
        }

        let mut idx: varnumber_T = 0;
        while *p != NUL {
            let len = utfc_ptr2len(p);
            if len == 0 {
                break;
            }
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list(), l2);
            tv_list_append_number(l2, idx);
            tv_list_append_string(l2, p, len as isize);
            p = p.add(len as usize);
            idx += 1;
        }
    }
}

/// Extend first list with the second.
pub unsafe fn tv_list_extend(l1: *mut list_T, l2: *mut list_T, bef: *mut listitem_T) {
    unsafe {
        let mut todo = tv_list_len_inline(l2);
        if todo == 0 {
            return;
        }

        let befbef = if bef.is_null() {
            ptr::null_mut()
        } else {
            (*bef).li_prev
        };
        let saved_next = if befbef.is_null() {
            ptr::null_mut()
        } else {
            (*befbef).li_next
        };
        let mut item = tv_list_first_inline(l2);
        while !item.is_null() && todo > 0 {
            todo -= 1;
            tv_list_insert_tv(l1, &raw mut (*item).li_tv, bef);
            item = if item == befbef {
                saved_next
            } else {
                (*item).li_next
            };
        }
    }
}

/// Concatenate lists into a new list.
pub unsafe fn tv_list_concat(l1: *mut list_T, l2: *mut list_T, tv: *mut typval_T) -> c_int {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).v_lock = VAR_UNLOCKED;
        let l = if l1.is_null() && l2.is_null() {
            ptr::null_mut()
        } else if l1.is_null() {
            tv_list_copy(None, l2, false, 0)
        } else {
            let copy = tv_list_copy(None, l1, false, 0);
            if !copy.is_null() && !l2.is_null() {
                tv_list_extend(copy, l2, ptr::null_mut());
            }
            copy
        };
        if l.is_null() && !(l1.is_null() && l2.is_null()) {
            return FAIL;
        }

        (*tv).vval = crate::eval::typval::TypvalVval::List(l);
        OK
    }
}

unsafe fn tv_list_slice(ol: *mut list_T, mut n1: varnumber_T, n2: varnumber_T) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc((n2 - n1 + 1) as isize);
        let mut item = tv_list_find(ol, n1 as c_int);
        while n1 <= n2 {
            tv_list_append_tv(l, &raw mut (*item).li_tv);
            item = (*item).li_next;
            n1 += 1;
        }
        l
    }
}

pub unsafe fn tv_list_slice_or_index(
    _list: *mut list_T,
    range: bool,
    n1_arg: varnumber_T,
    n2_arg: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let len = i64::from(tv_list_len_inline((*rettv).vval.v_list()));
        let mut n1 = n1_arg;
        let mut n2 = n2_arg;

        if n1 < 0 {
            n1 += len;
        }
        if n1 < 0 || n1 >= len {
            if !range {
                if verbose {
                    semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), n1_arg);
                }
                return FAIL;
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
            let l = tv_list_slice((*rettv).vval.v_list(), n1, n2);
            tv_clear(rettv);
            tv_list_set_ret_inline(rettv, l);
        } else {
            let found = tv_list_find((*rettv).vval.v_list(), n1 as c_int);
            let mut var1 = tv_initial_value();
            tv_copy(&raw const (*found).li_tv, &raw mut var1);
            tv_clear(rettv);
            *rettv = var1;
        }
        OK
    }
}

unsafe fn list_join_inner(
    gap: &mut Vec<c_char>,
    l: *mut list_T,
    sep: *const c_char,
    join_vec: &mut Vec<std::ffi::CString>,
) -> c_int {
    unsafe {
        let mut sumlen: usize = 0;
        let mut first = true;

        let mut item = (*l).lv_first;
        while !item.is_null() {
            if got_int {
                break;
            }
            let mut size: usize = 0;
            let s = encode_tv2echo(&raw mut (*item).li_tv, &raw mut size);

            sumlen += size;
            join_vec.push(s);

            line_breakcheck();
            item = (*item).li_next;
        }

        let seplen = strlen(sep);
        if join_vec.len() >= 2 {
            sumlen += seplen * (join_vec.len() - 1);
        }
        gap.reserve(sumlen + 2);

        for s in join_vec.iter() {
            if got_int {
                break;
            }
            if first {
                first = false;
            } else if seplen > 0 {
                gap.extend_from_slice(std::slice::from_raw_parts(sep, seplen));
            }
            let bytes = s.as_bytes();
            if !bytes.is_empty() {
                gap.extend_from_slice(std::slice::from_raw_parts(
                    bytes.as_ptr().cast::<c_char>(),
                    bytes.len(),
                ));
            }
            line_breakcheck();
        }

        OK
    }
}

/// Join list into a string using given separator.
pub unsafe fn tv_list_join(gap: &mut Vec<c_char>, l: *mut list_T, sep: *const c_char) -> c_int {
    unsafe {
        let len = tv_list_len_inline(l);
        if len == 0 {
            return OK;
        }

        let mut join_vec: Vec<std::ffi::CString> = Vec::with_capacity(len as usize);
        list_join_inner(gap, l, sep, &mut join_vec)
    }
}

/// "join()" function.
pub unsafe fn f_join(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(tr(e_listreq.as_ptr()));
            return;
        }
        let sep = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            c" ".as_ptr()
        } else {
            tv_get_string_chk(argvars.add(1))
        };

        (*rettv).v_type = VarType::VAR_STRING;

        if !sep.is_null() {
            let mut out: Vec<c_char> = Vec::with_capacity(80);
            tv_list_join(&mut out, (*argvars).vval.v_list(), sep);
            tv_set_string_len(rettv, out.as_ptr(), out.len());
        } else {
            (*rettv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
    }
}

/// "list2str()" function.
pub unsafe fn f_list2str(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(tr(e_invarg.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list();
        if l.is_null() {
            return;
        }

        let mut out: Vec<c_char> = Vec::with_capacity(80);
        let mut buf: [c_char; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];

        let mut li = (*l).lv_first;
        while !li.is_null() {
            let n = tv_get_number(&raw const (*li).li_tv);
            let buflen = utf_char2bytes(n as c_int, buf.as_mut_ptr()) as usize;
            buf[buflen] = NUL;
            out.extend_from_slice(&buf[..buflen]);
            li = (*li).li_next;
        }

        tv_set_string_len(rettv, out.as_ptr(), out.len());
    }
}

/// "remove({list})" function.
pub unsafe fn tv_list_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        let l = (*argvars).vval.v_list();
        if value_check_lock(tv_list_locked_inline(l), arg_errmsg, TV_TRANSLATE) {
            return;
        }

        let mut error = false;
        let idx = tv_get_number_chk(argvars.add(1), &raw mut error);

        if error {
            return;
        }
        let item = tv_list_find(l, idx as c_int);
        if item.is_null() {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), idx);
        } else if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            tv_list_drop_items(l, item, item);
            *rettv = std::mem::replace(&mut (*item).li_tv, tv_initial_value());
            drop(Box::from_raw(item));
        } else {
            let end = tv_get_number_chk(argvars.add(2), &raw mut error);
            if error {
                return;
            }
            let item2 = tv_list_find(l, end as c_int);
            if item2.is_null() {
                semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), end);
            } else {
                let mut cnt: c_int = 0;
                let mut li = item;
                while !li.is_null() {
                    cnt += 1;
                    if li == item2 {
                        break;
                    }
                    li = (*li).li_next;
                }
                if li.is_null() {
                    emsg(tr(e_invrange.as_ptr()));
                } else {
                    let tgt = tv_list_alloc_ret(rettv, cnt as isize);
                    tv_list_move_items(l, item, item2, tgt, cnt);
                }
            }
        }
    }
}

/// Compare functions for f_sort() and f_uniq().
unsafe fn item_compare(si1: &ListSortItem, si2: &ListSortItem, keep_zero: bool) -> c_int {
    unsafe {
        let item1 = si1.item;
        let item2 = si2.item;

        let tv1 = &raw mut (*item1).li_tv;
        let tv2 = &raw mut (*item2).li_tv;

        let mut res: c_int;

        if (*SORTINFO).item_compare_numbers {
            let v1 = if (*SORTINFO).item_compare_keys_ready {
                si1.key.as_int()
            } else {
                tv_get_number(tv1)
            };
            let v2 = if (*SORTINFO).item_compare_keys_ready {
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

        if (*SORTINFO).item_compare_float {
            let v1 = if (*SORTINFO).item_compare_keys_ready {
                si1.key.as_float()
            } else {
                tv_get_float(tv1)
            };
            let v2 = if (*SORTINFO).item_compare_keys_ready {
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

        if (*SORTINFO).item_compare_numeric && (*SORTINFO).item_compare_keys_ready {
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
        let mut p1: *mut c_char;
        let mut p2: *mut c_char;

        if (*tv1).v_type == VarType::VAR_STRING {
            if (*tv2).v_type != VarType::VAR_STRING || (*SORTINFO).item_compare_numeric {
                p1 = c"'".as_ptr().cast_mut();
            } else {
                p1 = (*tv1).vval.v_string();
            }
        } else {
            let s1 = encode_tv2string(tv1, ptr::null_mut());
            p1 = s1.as_ptr().cast_mut();
            tofree1 = Some(s1);
        }
        if (*tv2).v_type == VarType::VAR_STRING {
            if (*tv1).v_type != VarType::VAR_STRING || (*SORTINFO).item_compare_numeric {
                p2 = c"'".as_ptr().cast_mut();
            } else {
                p2 = (*tv2).vval.v_string();
            }
        } else {
            let s2 = encode_tv2string(tv2, ptr::null_mut());
            p2 = s2.as_ptr().cast_mut();
            tofree2 = Some(s2);
        }
        if p1.is_null() {
            p1 = c"".as_ptr().cast_mut();
        }
        if p2.is_null() {
            p2 = c"".as_ptr().cast_mut();
        }
        if !(*SORTINFO).item_compare_numeric {
            if (*SORTINFO).item_compare_lc {
                res = strcoll(p1, p2);
            } else if (*SORTINFO).item_compare_ic != 0 {
                res = vim_stricmp(p1, p2);
            } else {
                res = strcmp(p1, p2);
            }
        } else {
            let n1 = strtod(p1, &raw mut p1);
            let n2 = strtod(p2, &raw mut p2);
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
}

unsafe fn item_compare_keeping_zero(s1: &ListSortItem, s2: &ListSortItem) -> c_int {
    unsafe { item_compare(s1, s2, true) }
}

unsafe fn item_compare_not_keeping_zero(s1: &ListSortItem, s2: &ListSortItem) -> c_int {
    unsafe { item_compare(s1, s2, false) }
}

unsafe fn item_compare2(si1: &ListSortItem, si2: &ListSortItem, keep_zero: bool) -> c_int {
    unsafe {
        if (*SORTINFO).item_compare_func_err {
            return 0;
        }

        let item1 = si1.item;
        let item2 = si2.item;

        let partial = (*SORTINFO).item_compare_partial;
        let func_name = if partial.is_null() {
            (*SORTINFO).item_compare_func
        } else {
            partial_name(partial)
        };

        let mut argv: [typval_T; 3] = [tv_initial_value(); 3];
        tv_copy(&raw const (*item1).li_tv, &raw mut argv[0]);
        tv_copy(&raw const (*item2).li_tv, &raw mut argv[1]);

        let mut rettv = tv_initial_value();
        let mut funcexe: funcexe_T = funcexe_T::new();
        funcexe.fe_evaluate = true;
        funcexe.fe_partial = partial;
        funcexe.fe_selfdict = (*SORTINFO).item_compare_selfdict;
        let mut res = call_func(
            func_name,
            -1,
            &raw mut rettv,
            2,
            argv.as_mut_ptr(),
            &raw mut funcexe,
        );
        tv_clear(&raw mut argv[0]);
        tv_clear(&raw mut argv[1]);

        if res == FAIL {
            res = ITEM_COMPARE_FAIL;
            (*SORTINFO).item_compare_func_err = true;
        } else {
            let n = tv_get_number_chk(&raw const rettv, &raw mut (*SORTINFO).item_compare_func_err);
            res = if n > 0 {
                1
            } else if n < 0 {
                -1
            } else {
                0
            };
        }
        if (*SORTINFO).item_compare_func_err {
            res = ITEM_COMPARE_FAIL;
        }
        tv_clear(&raw mut rettv);

        if res == 0 && !keep_zero {
            res = if si1.idx > si2.idx { 1 } else { -1 };
        }

        res
    }
}

unsafe fn item_compare2_keeping_zero(s1: &ListSortItem, s2: &ListSortItem) -> c_int {
    unsafe { item_compare2(s1, s2, true) }
}

unsafe fn item_compare2_not_keeping_zero(s1: &ListSortItem, s2: &ListSortItem) -> c_int {
    unsafe { item_compare2(s1, s2, false) }
}

unsafe fn sort_compute_keys(ptrs: &mut [ListSortItem], info: *mut sortinfo_T) {
    unsafe {
        if (*info).item_compare_numbers {
            for si in ptrs.iter_mut() {
                si.key = ListSortKey::Int(tv_get_number(&raw const (*si.item).li_tv));
            }
        } else if (*info).item_compare_float {
            for si in ptrs.iter_mut() {
                si.key = ListSortKey::Float(tv_get_float(&raw const (*si.item).li_tv));
            }
        } else {
            for si in ptrs.iter_mut() {
                let tv = &raw mut (*si.item).li_tv;
                if (*tv).v_type == VarType::VAR_STRING {
                    si.key = ListSortKey::Float(0.0);
                } else {
                    let p = encode_tv2string(tv, ptr::null_mut());
                    let f = strtod(p.as_ptr(), ptr::null_mut());
                    si.key = ListSortKey::Float(f);
                }
            }
        }
        (*info).item_compare_keys_ready = true;
    }
}

unsafe fn do_sort(l: *mut list_T, info: *mut sortinfo_T) {
    unsafe {
        let len = tv_list_len_inline(l);
        let mut ptrs = Vec::with_capacity(len.max(0) as usize);

        let mut i: c_int = 0;
        let mut li = (*l).lv_first;
        while !li.is_null() {
            ptrs.push(ListSortItem {
                item: li,
                idx: i,
                key: ListSortKey::Int(0),
            });
            i += 1;
            li = (*li).li_next;
        }

        (*info).item_compare_func_err = false;
        (*info).item_compare_keys_ready = false;

        if (*info).item_compare_func.is_null()
            && (*info).item_compare_partial.is_null()
            && ((*info).item_compare_numbers
                || (*info).item_compare_float
                || (*info).item_compare_numeric)
        {
            sort_compute_keys(&mut ptrs, info);
        }
        let item_compare_func: ListSorter =
            if (*info).item_compare_func.is_null() && (*info).item_compare_partial.is_null() {
                item_compare_not_keeping_zero
            } else {
                item_compare2_not_keeping_zero
            };

        sort_slice_by(&mut ptrs, |a, b| item_compare_func(a, b));
        if !(*info).item_compare_func_err {
            (*l).lv_first = ptr::null_mut();
            (*l).lv_last = ptr::null_mut();
            (*l).lv_idx_item = ptr::null_mut();
            (*l).lv_len = 0;
            for si in &ptrs {
                tv_list_append(l, si.item);
            }
        }
        if (*info).item_compare_func_err {
            emsg(tr(c"E702: Sort compare function failed".as_ptr()));
        }
    }
}

unsafe fn do_uniq(l: *mut list_T, info: *mut sortinfo_T) {
    unsafe {
        (*info).item_compare_func_err = false;
        (*info).item_compare_keys_ready = false;
        let item_compare_func: ListSorter =
            if (*info).item_compare_func.is_null() && (*info).item_compare_partial.is_null() {
                item_compare_keeping_zero
            } else {
                item_compare2_keeping_zero
            };

        let first = tv_list_first_inline(l);
        let mut li = if first.is_null() {
            ptr::null_mut()
        } else {
            (*first).li_next
        };
        while !li.is_null() {
            let prev_li = (*li).li_prev;
            let prev_si = ListSortItem {
                item: prev_li,
                idx: 0,
                key: ListSortKey::Int(0),
            };
            let cur_si = ListSortItem {
                item: li,
                idx: 0,
                key: ListSortKey::Int(0),
            };
            if item_compare_func(&prev_si, &cur_si) == 0 {
                li = tv_list_item_remove(l, li);
            } else {
                li = (*li).li_next;
            }
            if (*info).item_compare_func_err {
                emsg(tr(c"E882: Uniq compare function failed".as_ptr()));
                break;
            }
        }
    }
}

unsafe fn parse_sort_uniq_args(argvars: *mut typval_T, info: *mut sortinfo_T) -> c_int {
    unsafe {
        (*info).item_compare_ic = 0;
        (*info).item_compare_lc = false;
        (*info).item_compare_numeric = false;
        (*info).item_compare_numbers = false;
        (*info).item_compare_float = false;
        (*info).item_compare_func = ptr::null();
        (*info).item_compare_partial = ptr::null_mut();
        (*info).item_compare_selfdict = ptr::null_mut();
        (*info).item_compare_keys_ready = false;

        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            return OK;
        }

        if (*argvars.add(1)).v_type == VarType::VAR_FUNC {
            (*info).item_compare_func = (*argvars.add(1)).vval.v_string();
        } else if (*argvars.add(1)).v_type == VarType::VAR_PARTIAL {
            (*info).item_compare_partial = (*argvars.add(1)).vval.v_partial();
        } else {
            let mut error = false;
            let nr = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return FAIL;
            }
            if nr == 1 {
                (*info).item_compare_ic = 1;
            } else if (*argvars.add(1)).v_type != VarType::VAR_NUMBER {
                (*info).item_compare_func = tv_get_string(argvars.add(1));
            } else if nr != 0 {
                emsg(tr(e_invarg.as_ptr()));
                return FAIL;
            }
            if !(*info).item_compare_func.is_null() {
                let fn_cstr = CStr::from_ptr((*info).item_compare_func).to_bytes();
                match fn_cstr {
                    b"" => (*info).item_compare_func = ptr::null(),
                    b"n" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_numeric = true;
                    }
                    b"N" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_numbers = true;
                    }
                    b"f" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_float = true;
                    }
                    b"i" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_ic = 1;
                    }
                    b"l" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_lc = true;
                    }
                    _ => {}
                }
            }
        }

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_dict_arg(argvars, 2) == FAIL {
                return FAIL;
            }
            (*info).item_compare_selfdict = (*argvars.add(2)).vval.v_dict();
        }

        OK
    }
}

unsafe fn do_sort_uniq(argvars: *mut typval_T, rettv: *mut typval_T, sort: bool) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg!(
                tr(e_listarg.as_ptr()),
                if sort {
                    c"sort()".as_ptr()
                } else {
                    c"uniq()".as_ptr()
                },
            );
            return;
        }

        let mut info = sortinfo_T::new();
        let old_sortinfo = SORTINFO;
        SORTINFO = &raw mut info;

        let arg_errmsg = if sort {
            c"sort() argument".as_ptr()
        } else {
            c"uniq() argument".as_ptr()
        };
        let l = (*argvars).vval.v_list();
        if !value_check_lock(tv_list_locked_inline(l), arg_errmsg, TV_TRANSLATE) {
            tv_list_set_ret_inline(rettv, l);

            let len = tv_list_len_inline(l);
            if len > 1 && parse_sort_uniq_args(argvars, &raw mut info) != FAIL {
                if sort {
                    do_sort(l, &raw mut info);
                } else {
                    do_uniq(l, &raw mut info);
                }
            }
        }

        SORTINFO = old_sortinfo;
    }
}

/// "sort({list})" function.
pub unsafe fn f_sort(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        do_sort_uniq(argvars, rettv, true);
    }
}

/// "uniq({list})" function.
pub unsafe fn f_uniq(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        do_sort_uniq(argvars, rettv, false);
    }
}

/// Check whether two lists are equal.
pub unsafe fn tv_list_equal(l1: *mut list_T, l2: *mut list_T, ic: bool) -> bool {
    unsafe {
        if l1 == l2 {
            return true;
        }
        if tv_list_len_inline(l1) != tv_list_len_inline(l2) {
            return false;
        }
        if tv_list_len_inline(l1) == 0 {
            return true;
        }
        if l1.is_null() || l2.is_null() {
            return false;
        }

        let mut item1 = tv_list_first_inline(l1);
        let mut item2 = tv_list_first_inline(l2);
        while !item1.is_null() && !item2.is_null() {
            if !tv_equal(&raw mut (*item1).li_tv, &raw mut (*item2).li_tv, ic) {
                return false;
            }
            item1 = (*item1).li_next;
            item2 = (*item2).li_next;
        }
        debug_assert!(item1.is_null() && item2.is_null());
        true
    }
}

/// Reverse list in-place.
pub unsafe fn tv_list_reverse(l: *mut list_T) {
    unsafe {
        if tv_list_len_inline(l) <= 1 {
            return;
        }
        std::mem::swap(&mut (*l).lv_first, &mut (*l).lv_last);
        let mut li = (*l).lv_first;
        while !li.is_null() {
            std::mem::swap(&mut (*li).li_next, &mut (*li).li_prev);
            li = (*li).li_next;
        }
        (*l).lv_idx = (*l).lv_len - (*l).lv_idx - 1;
    }
}

/// Locate item with a given index in a list and return it.
pub unsafe fn tv_list_find(l: *mut list_T, mut n: c_int) -> *mut listitem_T {
    unsafe {
        if l.is_null() {
            return ptr::null_mut();
        }

        n = tv_list_uidx_inline(l, n);
        if n == -1 {
            return ptr::null_mut();
        }

        let mut idx: c_int;
        let mut item: *mut listitem_T;

        if !(*l).lv_idx_item.is_null() {
            if n < (*l).lv_idx / 2 {
                item = (*l).lv_first;
                idx = 0;
            } else if n > ((*l).lv_idx + (*l).lv_len) / 2 {
                item = (*l).lv_last;
                idx = (*l).lv_len - 1;
            } else {
                item = (*l).lv_idx_item;
                idx = (*l).lv_idx;
            }
        } else if n < (*l).lv_len / 2 {
            item = (*l).lv_first;
            idx = 0;
        } else {
            item = (*l).lv_last;
            idx = (*l).lv_len - 1;
        }

        while n > idx {
            item = (*item).li_next;
            idx += 1;
        }
        while n < idx {
            item = (*item).li_prev;
            idx -= 1;
        }

        debug_assert_eq!(idx, n);
        (*l).lv_idx = idx;
        (*l).lv_idx_item = item;

        item
    }
}

/// Get list item l[n] as a number.
pub unsafe fn tv_list_find_nr(l: *mut list_T, n: c_int, ret_error: *mut bool) -> varnumber_T {
    unsafe {
        let li = tv_list_find(l, n);
        if li.is_null() {
            if !ret_error.is_null() {
                *ret_error = true;
            }
            return -1;
        }
        tv_get_number_chk(&raw const (*li).li_tv, ret_error)
    }
}

/// Get list item l[n] as a string.
pub unsafe fn tv_list_find_str(l: *mut list_T, n: c_int) -> *const c_char {
    unsafe {
        let li = tv_list_find(l, n);
        if li.is_null() {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(n));
            return ptr::null();
        }
        tv_get_string(&raw const (*li).li_tv)
    }
}

unsafe fn tv_list_find_index(l: *mut list_T, idx: *mut c_int) -> *mut listitem_T {
    unsafe {
        let mut li = tv_list_find(l, *idx);
        if !li.is_null() {
            return li;
        }

        if *idx < 0 {
            *idx = 0;
            li = tv_list_find(l, *idx);
        }
        li
    }
}

/// Locate item in a list and return its index.
pub unsafe fn tv_list_idx_of_item(l: *const list_T, item: *const listitem_T) -> c_int {
    unsafe {
        if l.is_null() {
            return -1;
        }
        let mut idx: c_int = 0;
        let mut li: *const listitem_T = (*l).lv_first;
        while !li.is_null() {
            if ptr::eq(li, item) {
                return idx;
            }
            idx += 1;
            li = (*li).li_next;
        }
        -1
    }
}

/// Add watcher to a dictionary.
pub unsafe fn tv_dict_watcher_add(
    dict: *mut dict_T,
    key_pattern: *const c_char,
    key_pattern_len: usize,
    callback: Callback,
) {
    unsafe {
        if dict.is_null() {
            return;
        }
        (*dict).watchers.push(DictWatcher {
            key_pattern: crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                key_pattern.cast::<u8>(),
                key_pattern_len,
            )),
            key_pattern_len,
            callback,
            busy: false,
            needs_free: false,
        });
    }
}

/// Check whether two callbacks are equal.
pub unsafe fn tv_callback_equal(cb1: *const Callback, cb2: *const Callback) -> bool {
    unsafe {
        if (*cb1).cb_type() != (*cb2).cb_type() {
            return false;
        }
        match (*cb1).cb_type() {
            kCallbackFuncref | kCallbackExpr => {
                strequal((*cb1).data.as_funcref(), (*cb2).data.as_funcref())
            }
            kCallbackPartial => (*cb1).data.as_partial() == (*cb2).data.as_partial(),
            kCallbackLua => (*cb1).data.as_luaref() == (*cb2).data.as_luaref(),
            kCallbackNone => true,
            _ => std::process::abort(),
        }
    }
}

/// Unref/free callback.
pub unsafe fn callback_free(callback: *mut Callback) {
    unsafe {
        match (*callback).cb_type() {
            kCallbackFuncref => {
                func_unref((*callback).data.as_funcref());
            }
            kCallbackPartial => {
                partial_unref((*callback).data.as_partial());
            }
            kCallbackLua => {
                let mut luaref = (*callback).data.as_luaref();
                nlua_clear_ref(&mut luaref);
            }
            _ => {}
        }
        *callback = Callback::NONE;
    }
}

/// Copy a callback into a typval_T.
pub unsafe fn callback_put(cb: *mut Callback, tv: *mut typval_T) {
    unsafe {
        match (*cb).cb_type() {
            kCallbackPartial => {
                let pt = (*cb).data.as_partial();
                (*tv).v_type = VarType::VAR_PARTIAL;
                (*tv).vval = crate::eval::typval::TypvalVval::Partial(pt);
                (*pt).pt_refcount += 1;
            }
            kCallbackFuncref => {
                let funcref = (*cb).data.as_funcref();
                tv_set_string(tv, funcref);
                (*tv).v_type = VarType::VAR_FUNC;
                func_ref((*tv).vval.v_string());
            }
            _ => {
                (*tv).v_type = VarType::VAR_SPECIAL;
                (*tv).vval = crate::eval::typval::TypvalVval::Special(kSpecialVarNull);
            }
        }
    }
}

/// Copy callback from "src" to "dest", incrementing the refcounts.
pub unsafe fn callback_copy(dest: *mut Callback, src: *mut Callback) {
    unsafe {
        match (*src).cb_type() {
            kCallbackPartial => {
                let pt = (*src).data.as_partial();
                (*dest).data = CallbackData::partial(pt);
                (*pt).pt_refcount += 1;
            }
            kCallbackFuncref => {
                let funcref = (*src).data.as_funcref();
                (*dest).data = CallbackData::funcref(CStr::from_ptr(funcref).to_owned());
                func_ref(funcref);
            }
            kCallbackLua => {
                let new_ref = api_new_luaref((*src).data.as_luaref());
                (*dest).data = CallbackData::lua(new_ref);
            }
            kCallbackExpr => {
                (*dest).data =
                    CallbackData::expr(CStr::from_ptr((*src).data.as_funcref()).to_owned());
            }
            _ => {
                (*dest).data = CallbackData::NONE;
            }
        }
    }
}

/// Generate a string description of a callback.
pub unsafe fn callback_to_string(cb: *mut Callback) -> std::ffi::CString {
    unsafe {
        if (*cb).cb_type() == kCallbackLua {
            return nlua_funcref_str((*cb).data.as_luaref(), true);
        }

        const MSGLEN: usize = 100;
        let mut msg: Vec<u8> = Vec::with_capacity(MSGLEN);

        match (*cb).cb_type() {
            kCallbackFuncref => {
                let s = CStr::from_ptr((*cb).data.as_funcref()).to_bytes();
                let prefix = b"<vim function: ";
                let suffix = b">";
                let copy_len = s.len().min(MSGLEN - prefix.len() - suffix.len());
                msg.extend_from_slice(prefix);
                msg.extend_from_slice(&s[..copy_len]);
                if msg.len() < MSGLEN {
                    msg.push(b'>');
                }
            }
            kCallbackPartial => {
                let pt = (*cb).data.as_partial();
                let s = (*pt).pt_name.as_ref().map_or(&[][..], |n| n.as_bytes());
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
        cstr_from_bytes(&msg)
    }
}

/// Remove watcher from a dictionary.
pub unsafe fn tv_dict_watcher_remove(
    dict: *mut dict_T,
    key_pattern: *const c_char,
    key_pattern_len: usize,
    callback: &Callback,
) -> bool {
    unsafe {
        if dict.is_null() {
            return false;
        }

        let mut matched_idx: Option<usize> = None;
        let mut queue_is_busy = false;
        for (idx, w) in (*dict).watchers.iter().enumerate() {
            if w.busy {
                queue_is_busy = true;
            }
            if matched_idx.is_none()
                && tv_callback_equal(&raw const w.callback, callback)
                && w.key_pattern_len == key_pattern_len
                && w.key_pattern.as_bytes()
                    == std::slice::from_raw_parts(key_pattern.cast::<u8>(), key_pattern_len)
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
            (&mut (*dict).watchers)[idx].needs_free = true;
        } else {
            (*dict).watchers.remove(idx);
        }
        true
    }
}

/// Test if `key` matches with `watcher->key_pattern`.
unsafe fn tv_dict_watcher_matches(watcher: *const DictWatcher, key: *const c_char) -> bool {
    unsafe {
        let len = (*watcher).key_pattern_len;
        let pat_ptr = (*watcher).key_pattern.as_ptr();
        if len > 0 && *pat_ptr.add(len - 1) == b'*' as c_char {
            strncmp(key, pat_ptr, len - 1) == 0
        } else {
            strcmp(key, pat_ptr) == 0
        }
    }
}

/// Send a change notification to all dictionary watchers that match given key.
pub unsafe fn tv_dict_watcher_notify(
    dict: *mut dict_T,
    key: *const c_char,
    newtv: *mut typval_T,
    oldtv: *mut typval_T,
) {
    unsafe {
        let mut argv: [typval_T; 3] = [tv_initial_value(); 3];

        argv[0].v_type = VarType::VAR_DICT;
        argv[0].v_lock = VAR_UNLOCKED;
        argv[0].vval = crate::eval::typval::TypvalVval::Dict(dict);
        argv[1].v_lock = VAR_UNLOCKED;
        tv_set_string(&raw mut argv[1], key);
        argv[2].v_type = VarType::VAR_DICT;
        argv[2].v_lock = VAR_UNLOCKED;
        let changes_dict = tv_dict_alloc();
        argv[2].vval = crate::eval::typval::TypvalVval::Dict(changes_dict);
        (*changes_dict).dv_refcount += 1;

        if !newtv.is_null() {
            let v = tv_dict_item_alloc_len(c"new".as_ptr(), 3);
            tv_copy(newtv, &raw mut (*v).di_tv);
            tv_dict_add(changes_dict, v);
        }

        if !oldtv.is_null() && (*oldtv).v_type != VarType::VAR_UNKNOWN {
            let v = tv_dict_item_alloc_len(c"old".as_ptr(), 3);
            tv_copy(oldtv, &raw mut (*v).di_tv);
            tv_dict_add(changes_dict, v);
        }

        let mut any_needs_free = false;
        (*dict).dv_refcount += 1;
        let len = (*dict).watchers.len();
        for i in 0..len {
            let matches = {
                let w = &(&(*dict).watchers)[i];
                !w.busy && tv_dict_watcher_matches(w, key)
            };
            if matches {
                let mut rettv = tv_initial_value();
                (&mut (*dict).watchers)[i].busy = true;
                let mut cb = (&(*dict).watchers)[i].callback.clone();
                callback_call(&raw mut cb, 3, argv.as_mut_ptr(), &raw mut rettv);
                let w = &mut (&mut (*dict).watchers)[i];
                w.callback = cb;
                w.busy = false;
                tv_clear(&raw mut rettv);
                if w.needs_free {
                    any_needs_free = true;
                }
            }
        }
        if any_needs_free {
            (*dict).watchers.retain(|watcher| !watcher.needs_free);
        }
        tv_dict_unref(dict);

        for item in argv.iter_mut().skip(1) {
            tv_clear(item);
        }
    }
}

/// Allocate a dictionary item.
pub unsafe fn tv_dict_item_alloc_len(key: *const c_char, key_len: usize) -> *mut dictitem_T {
    unsafe {
        let mut key_vec = Vec::with_capacity(key_len + 1);
        if key_len > 0 {
            key_vec.extend_from_slice(std::slice::from_raw_parts(key, key_len));
        }
        key_vec.push(NUL);
        Box::into_raw(Box::new(dictitem_T {
            di_tv: typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            },
            di_flags: DI_FLAGS_ALLOC,
            di_key: key_vec,
        }))
    }
}

/// Allocate a dictionary item.
pub unsafe fn tv_dict_item_alloc(key: *const c_char) -> *mut dictitem_T {
    unsafe { tv_dict_item_alloc_len(key, strlen(key)) }
}

/// Free a dictionary item, also clearing the value.
pub unsafe fn tv_dict_item_free(item: *mut dictitem_T) {
    unsafe {
        if ((*item).di_flags & DI_FLAGS_ALLOC) != 0 {
            drop(Box::from_raw(item));
        } else {
            tv_clear(&raw mut (*item).di_tv);
        }
    }
}

/// Make a copy of a dictionary item.
pub unsafe fn tv_dict_item_copy(di: *mut dictitem_T) -> *mut dictitem_T {
    unsafe {
        let key_ptr = (*di).di_key.as_ptr();
        let new_di = tv_dict_item_alloc(key_ptr);
        tv_copy(&raw const (*di).di_tv, &raw mut (*new_di).di_tv);
        new_di
    }
}

/// Remove item from dictionary and free it.
pub unsafe fn tv_dict_item_remove(dict: *mut dict_T, item: *mut dictitem_T) {
    unsafe {
        let key_ptr = (*item).di_key.as_ptr();
        let hi = hash_find(&raw const (*dict).dv_hashtab, key_ptr);
        if hashitem_empty(&*hi) {
            semsg!(tr(e_intern2.as_ptr()), c"tv_dict_item_remove()".as_ptr());
        } else {
            hash_remove(&raw mut (*dict).dv_hashtab, hi);
        }
        tv_dict_item_free(item);
    }
}

/// Allocate an empty dictionary.
pub unsafe fn tv_dict_alloc() -> *mut dict_T {
    unsafe {
        let d = Box::into_raw(Box::new(dict_T {
            dv_lock: VAR_UNLOCKED,
            dv_scope: VAR_NO_SCOPE,
            dv_refcount: 0,
            dv_copyID: 0,
            dv_hashtab: crate::hashtab::hashtab_T::new(),
            dv_copydict: ptr::null_mut(),
            dv_used_next: gc_first_dict,
            dv_used_prev: ptr::null_mut(),
            watchers: Vec::new(),
            lua_table_ref: LUA_NOREF,
        }));

        if !gc_first_dict.is_null() {
            (*gc_first_dict).dv_used_prev = d;
        }
        gc_first_dict = d;

        d
    }
}

/// Free items contained in a dictionary.
pub unsafe fn tv_dict_free_contents(d: *mut dict_T) {
    unsafe {
        hash_lock(&raw mut (*d).dv_hashtab);
        debug_assert!((*d).dv_hashtab.ht_locked > 0);
        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                hash_remove(&raw mut (*d).dv_hashtab, hi);
                tv_dict_item_free(di);
            }
            hi = hi.add(1);
        }

        (*d).watchers.clear();

        hash_clear(&raw mut (*d).dv_hashtab);
    }
}

/// Free a dictionary, including all items it contains.
pub unsafe fn tv_dict_free(d: *mut dict_T) {
    unsafe {
        if tv_in_free_unref_items {
            return;
        }

        tv_dict_free_contents(d);
        drop(Box::from_raw(d));
    }
}

/// Unreference a dictionary.
pub unsafe fn tv_dict_unref(d: *mut dict_T) {
    unsafe {
        if !d.is_null() {
            (*d).dv_refcount -= 1;
            if (*d).dv_refcount <= 0 {
                tv_dict_free(d);
            }
        }
    }
}

/// Find item in dictionary.
pub unsafe fn tv_dict_find(d: *const dict_T, key: *const c_char, len: isize) -> *mut dictitem_T {
    unsafe {
        if d.is_null() {
            return ptr::null_mut();
        }
        let hi = if len < 0 {
            hash_find(&raw const (*d).dv_hashtab, key)
        } else {
            hash_find_len(&raw const (*d).dv_hashtab, key, len as usize)
        };
        if hashitem_empty(&*hi) {
            return ptr::null_mut();
        }
        tv_dict_hi2di(hi)
    }
}

/// Check if a key is present in a dictionary.
pub unsafe fn tv_dict_has_key(d: *const dict_T, key: *const c_char) -> bool {
    unsafe { !tv_dict_find(d, key, -1).is_null() }
}

/// Get a typval item from a dictionary and copy it into "rettv".
pub unsafe fn tv_dict_get_tv(d: *mut dict_T, key: *const c_char, rettv: *mut typval_T) -> c_int {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return FAIL;
        }

        tv_copy(&raw const (*di).di_tv, rettv);
        OK
    }
}

/// Gets a number item from a dictionary.
pub unsafe fn tv_dict_get_number(d: *const dict_T, key: *const c_char) -> varnumber_T {
    unsafe { tv_dict_get_number_def(d, key, 0) }
}

/// Gets a number item from a dictionary, or a given default value.
pub unsafe fn tv_dict_get_number_def(
    d: *const dict_T,
    key: *const c_char,
    def: c_int,
) -> varnumber_T {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return i64::from(def);
        }
        tv_get_number(&raw const (*di).di_tv)
    }
}

/// Gets a bool item from a dictionary, or a given default value.
pub unsafe fn tv_dict_get_bool(d: *const dict_T, key: *const c_char, def: c_int) -> varnumber_T {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return i64::from(def);
        }
        tv_get_bool(&raw const (*di).di_tv)
    }
}

/// Converts a dict to an environment.
pub unsafe fn tv_dict_to_env(denv: *mut dict_T) -> Vec<std::ffi::CString> {
    unsafe {
        let env_size = tv_dict_len_inline(denv) as usize;
        let mut env = Vec::with_capacity(env_size);

        if !denv.is_null() {
            let mut todo = (*denv).dv_hashtab.ht_used;
            let mut hi = (*denv).dv_hashtab.ht_array;
            while todo > 0 {
                if !hashitem_empty(&*hi) {
                    todo -= 1;
                    let var = tv_dict_hi2di(hi);
                    let str_ = tv_get_string(&raw const (*var).di_tv);
                    debug_assert!(!str_.is_null());
                    let key_bytes = CStr::from_ptr((*var).di_key.as_ptr()).to_bytes();
                    let val_bytes = CStr::from_ptr(str_).to_bytes();
                    let mut entry = Vec::with_capacity(key_bytes.len() + 1 + val_bytes.len());
                    entry.extend_from_slice(key_bytes);
                    entry.push(b'=');
                    entry.extend_from_slice(val_bytes);
                    env.push(cstr_from_bytes(&entry));
                }
                hi = hi.add(1);
            }
        }

        env
    }
}

/// Get an owned `CString` item from a dictionary.
pub unsafe fn tv_dict_get_cstring(
    d: *const dict_T,
    key: *const c_char,
) -> Option<std::ffi::CString> {
    unsafe {
        static mut NUMBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let s = tv_dict_get_string_buf(d, key, (&raw mut NUMBUF).cast::<c_char>());
        if s.is_null() {
            None
        } else {
            Some(cstr_to_cstring(s))
        }
    }
}

/// Get a string item from a dictionary.
pub unsafe fn tv_dict_get_string(d: *const dict_T, key: *const c_char) -> *mut c_char {
    unsafe {
        static mut NUMBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        tv_dict_get_string_buf(d, key, (&raw mut NUMBUF).cast::<c_char>()).cast_mut()
    }
}

/// Get a string item from a dictionary.
pub unsafe fn tv_dict_get_string_buf(
    d: *const dict_T,
    key: *const c_char,
    numbuf: *mut c_char,
) -> *const c_char {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return ptr::null();
        }
        tv_get_string_buf(&raw const (*di).di_tv, numbuf)
    }
}

/// Get a string item from a dictionary.
pub unsafe fn tv_dict_get_string_buf_chk(
    d: *const dict_T,
    key: *const c_char,
    key_len: isize,
    numbuf: *mut c_char,
    def: *const c_char,
) -> *const c_char {
    unsafe {
        let di = tv_dict_find(d, key, key_len);
        if di.is_null() {
            return def;
        }
        tv_get_string_buf_chk(&raw const (*di).di_tv, numbuf)
    }
}

/// Get a function from a dictionary.
pub unsafe fn tv_dict_get_callback(
    d: *mut dict_T,
    key: *const c_char,
    key_len: isize,
    result: *mut Callback,
) -> bool {
    unsafe {
        *result = Callback::NONE;

        let di = tv_dict_find(d, key, key_len);
        if di.is_null() {
            return true;
        }

        if !tv_is_func_inline((*di).di_tv) && (*di).di_tv.v_type != VarType::VAR_STRING {
            emsg(tr(
                c"E6000: Argument is not a function or function name".as_ptr()
            ));
            return false;
        }

        let mut tv = tv_initial_value();
        tv_copy(&raw const (*di).di_tv, &raw mut tv);
        set_selfdict(&raw mut tv, d);
        let res = callback_from_typval(result, &raw mut tv);
        tv_clear(&raw mut tv);
        res
    }
}

/// Check for adding a function to g: or l:.
pub unsafe fn tv_dict_wrong_func_name(
    d: *mut dict_T,
    tv: *mut typval_T,
    name: *const c_char,
) -> c_int {
    unsafe {
        c_int::from(
            (d == get_globvar_dict()
                || ptr::eq(&raw const (*d).dv_hashtab, get_funccal_local_ht()))
                && tv_is_func_inline(*tv)
                && var_wrong_func_name(name, true),
        )
    }
}

/// Add item to dictionary.
pub unsafe fn tv_dict_add(d: *mut dict_T, item: *mut dictitem_T) -> c_int {
    unsafe {
        let key_ptr = (*item).di_key.as_mut_ptr();
        if tv_dict_wrong_func_name(d, &raw mut (*item).di_tv, key_ptr) != 0 {
            return FAIL;
        }
        hash_add(&raw mut (*d).dv_hashtab, item)
    }
}

/// Add a list entry to dictionary.
pub unsafe fn tv_dict_add_list(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    list: *mut list_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_LIST;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::List(list);
        if tv_dict_add(d, item) == FAIL {
            (*item).di_tv.vval = crate::eval::typval::TypvalVval::List(ptr::null_mut());
            tv_dict_item_free(item);
            return FAIL;
        }
        tv_list_ref_inline(list);
        OK
    }
}

/// Add a typval entry to dictionary.
pub unsafe fn tv_dict_add_tv(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    tv: *mut typval_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        tv_copy(tv, &raw mut (*item).di_tv);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a dictionary entry to dictionary.
pub unsafe fn tv_dict_add_dict(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    dict: *mut dict_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_DICT;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::Dict(dict);
        if tv_dict_add(d, item) == FAIL {
            (*item).di_tv.vval = crate::eval::typval::TypvalVval::Dict(ptr::null_mut());
            tv_dict_item_free(item);
            return FAIL;
        }
        (*dict).dv_refcount += 1;
        OK
    }
}

/// Add a number entry to dictionary.
pub unsafe fn tv_dict_add_nr(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    nr: varnumber_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_NUMBER;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::Number(nr);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a floating point number entry to dictionary.
pub unsafe fn tv_dict_add_float(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    nr: float_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_FLOAT;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::Float(nr);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a boolean entry to dictionary.
pub unsafe fn tv_dict_add_bool(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: BoolVarValue,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_BOOL;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::Bool(val);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a string entry to dictionary.
pub unsafe fn tv_dict_add_str(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: *const c_char,
) -> c_int {
    unsafe { tv_dict_add_str_len(d, key, key_len, val, -1) }
}

/// Add a string entry to dictionary.
pub unsafe fn tv_dict_add_str_len(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: *const c_char,
    len: c_int,
) -> c_int {
    unsafe {
        let s = if val.is_null() {
            ptr::null_mut()
        } else if len < 0 {
            tv_alloc_string(val)
        } else {
            tv_alloc_string_bytes(std::slice::from_raw_parts(val.cast::<u8>(), len as usize))
        };
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_STRING;
        (*item).di_tv.vval = crate::eval::typval::TypvalVval::String(s);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a function entry to dictionary.
pub unsafe fn tv_dict_add_func(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    fp: *mut ufunc_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        let name_ptr = (*fp).uf_name.as_ptr();
        tv_set_string_len(&raw mut (*item).di_tv, name_ptr, (*fp).uf_namelen);
        (*item).di_tv.v_type = VarType::VAR_FUNC;
        func_ref((*item).di_tv.vval.v_string());
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Clear all the keys of a Dictionary.
#[cfg(test)]
pub unsafe fn tv_dict_clear(d: *mut dict_T) {
    unsafe {
        hash_lock(&raw mut (*d).dv_hashtab);
        debug_assert!((*d).dv_hashtab.ht_locked > 0);

        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                tv_dict_item_free(tv_dict_hi2di(hi));
                hash_remove(&raw mut (*d).dv_hashtab, hi);
            }
            hi = hi.add(1);
        }

        hash_unlock(&raw mut (*d).dv_hashtab);
    }
}

/// Extend dictionary with items from another dictionary.
pub unsafe fn tv_dict_extend(d1: *mut dict_T, d2: *mut dict_T, action: *const c_char) {
    unsafe {
        let watched = tv_dict_is_watched_inline(d1);
        let arg_errmsg = tr(c"extend() argument".as_ptr());
        let arg_errmsg_len = strlen(arg_errmsg);

        if *action == b'm' as c_char {
            hash_lock(&raw mut (*d2).dv_hashtab);
        }

        let mut todo = (*d2).dv_hashtab.ht_used;
        let mut hi2 = (*d2).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi2) {
                todo -= 1;
                let di2 = tv_dict_hi2di(hi2);
                let key2 = (*di2).di_key.as_ptr();
                let di1 = tv_dict_find(d1, key2, -1);
                if (*d1).dv_scope != VAR_NO_SCOPE && !valid_varname(key2) {
                    break;
                }
                if di1.is_null() {
                    if *action == b'm' as c_char {
                        let new_di = di2;
                        if tv_dict_add(d1, new_di) == OK {
                            hash_remove(&raw mut (*d2).dv_hashtab, hi2);
                            tv_dict_watcher_notify(
                                d1,
                                (*new_di).di_key.as_ptr(),
                                &raw mut (*new_di).di_tv,
                                ptr::null_mut(),
                            );
                        }
                    } else {
                        let new_di = tv_dict_item_copy(di2);
                        if tv_dict_add(d1, new_di) == FAIL {
                            tv_dict_item_free(new_di);
                        } else if watched {
                            tv_dict_watcher_notify(
                                d1,
                                (*new_di).di_key.as_ptr(),
                                &raw mut (*new_di).di_tv,
                                ptr::null_mut(),
                            );
                        }
                    }
                } else if *action == b'e' as c_char {
                    semsg!(tr(c"E737: Key already exists: %s".as_ptr()), key2);
                    break;
                } else if *action == b'f' as c_char && di2 != di1 {
                    if value_check_lock((*di1).di_tv.v_lock, arg_errmsg, arg_errmsg_len)
                        || var_check_ro(c_int::from((*di1).di_flags), arg_errmsg, arg_errmsg_len)
                    {
                        break;
                    }
                    if tv_dict_wrong_func_name(d1, &raw mut (*di2).di_tv, key2) != 0 {
                        break;
                    }

                    let mut oldtv = tv_initial_value();
                    if watched {
                        tv_copy(&raw const (*di1).di_tv, &raw mut oldtv);
                    }

                    tv_clear(&raw mut (*di1).di_tv);
                    tv_copy(&raw const (*di2).di_tv, &raw mut (*di1).di_tv);

                    if watched {
                        tv_dict_watcher_notify(
                            d1,
                            (*di1).di_key.as_ptr(),
                            &raw mut (*di1).di_tv,
                            &raw mut oldtv,
                        );
                        tv_clear(&raw mut oldtv);
                    }
                }
            }
            hi2 = hi2.add(1);
        }

        if *action == b'm' as c_char {
            hash_unlock(&raw mut (*d2).dv_hashtab);
        }
    }
}

/// Compare two dictionaries.
pub unsafe fn tv_dict_equal(d1: *mut dict_T, d2: *mut dict_T, ic: bool) -> bool {
    unsafe {
        if d1 == d2 {
            return true;
        }
        if tv_dict_len_inline(d1) != tv_dict_len_inline(d2) {
            return false;
        }
        if tv_dict_len_inline(d1) == 0 {
            return true;
        }
        if d1.is_null() || d2.is_null() {
            return false;
        }

        let mut todo = (*d1).dv_hashtab.ht_used;
        let mut hi = (*d1).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di1 = tv_dict_hi2di(hi);
                let key1 = (*di1).di_key.as_ptr();
                let di2 = tv_dict_find(d2, key1, -1);
                if di2.is_null() {
                    return false;
                }
                if !tv_equal(&raw mut (*di1).di_tv, &raw mut (*di2).di_tv, ic) {
                    return false;
                }
            }
            hi = hi.add(1);
        }
        true
    }
}

/// Make a copy of dictionary.
pub unsafe fn tv_dict_copy(
    conv: Option<&vimconv_T>,
    orig: *mut dict_T,
    deep: bool,
    copyID: c_int,
) -> *mut dict_T {
    unsafe {
        if orig.is_null() {
            return ptr::null_mut();
        }

        let mut copy = tv_dict_alloc();
        if copyID != 0 {
            (*orig).dv_copyID = copyID;
            (*orig).dv_copydict = copy;
        }
        let mut todo = (*orig).dv_hashtab.ht_used;
        let mut hi = (*orig).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                if got_int {
                    break;
                }
                let di = tv_dict_hi2di(hi);
                let key_ptr = (*di).di_key.as_mut_ptr();
                let new_di = if let Some(conv) = conv.filter(|c| c.vc_type != CONV_NONE) {
                    let mut len = strlen(key_ptr);
                    if let Some(key) = string_convert(conv, key_ptr, &raw mut len) {
                        tv_dict_item_alloc_len(key.as_ptr().cast(), len)
                    } else {
                        tv_dict_item_alloc_len(key_ptr, len)
                    }
                } else {
                    tv_dict_item_alloc(key_ptr)
                };
                if deep {
                    if var_item_copy(
                        conv,
                        &raw mut (*di).di_tv,
                        &raw mut (*new_di).di_tv,
                        deep,
                        copyID,
                    ) == FAIL
                    {
                        tv_dict_item_free(new_di);
                        break;
                    }
                } else {
                    tv_copy(&raw const (*di).di_tv, &raw mut (*new_di).di_tv);
                }
                if tv_dict_add(copy, new_di) == FAIL {
                    tv_dict_item_free(new_di);
                    break;
                }
            }
            hi = hi.add(1);
        }

        (*copy).dv_refcount += 1;
        if got_int {
            tv_dict_unref(copy);
            copy = ptr::null_mut();
        }

        copy
    }
}

/// Set all existing keys in "dict" as read-only.
pub unsafe fn tv_dict_set_keys_readonly(dict: *mut dict_T) {
    unsafe {
        let mut todo = (*dict).dv_hashtab.ht_used;
        let mut hi = (*dict).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                (*di).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            }
            hi = hi.add(1);
        }
    }
}

/// Allocate an empty blob.
pub unsafe fn tv_blob_alloc() -> *mut blob_T {
    Box::into_raw(Box::new(blob_T {
        bv_ga: Vec::new(),
        bv_refcount: 0,
        bv_lock: 0,
    }))
}

/// Unreference a blob.
pub unsafe fn tv_blob_unref(b: *mut blob_T) {
    unsafe {
        if !b.is_null() {
            (*b).bv_refcount -= 1;
            if (*b).bv_refcount <= 0 {
                drop(Box::from_raw(b));
            }
        }
    }
}

/// Check whether two blobs are equal.
pub unsafe fn tv_blob_equal(b1: *const blob_T, b2: *const blob_T) -> bool {
    unsafe {
        let len1 = tv_blob_len_inline(b1);
        let len2 = tv_blob_len_inline(b2);

        if len1 == 0 && len2 == 0 {
            return true;
        }
        if ptr::eq(b1, b2) {
            return true;
        }
        if len1 != len2 {
            return false;
        }

        let s1 = &(*b1).bv_ga;
        let s2 = &(*b2).bv_ga;
        s1 == s2
    }
}

unsafe fn tv_blob_slice(
    _blob: *const blob_T,
    len: c_int,
    mut n1: varnumber_T,
    mut n2: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
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
            tv_clear(rettv);
            (*rettv).v_type = VarType::VAR_BLOB;
            (*rettv).vval = crate::eval::typval::TypvalVval::Blob(ptr::null_mut());
        } else {
            let new_blob = tv_blob_alloc();
            let src_blob = (*rettv).vval.v_blob();
            let new_bv = &mut (*new_blob).bv_ga;
            new_bv.extend_from_slice(&(&(*src_blob).bv_ga)[(n1 as usize)..=(n2 as usize)]);
            tv_clear(rettv);
            tv_blob_set_ret_inline(rettv, new_blob);
        }

        OK
    }
}

unsafe fn tv_blob_index(
    _blob: *const blob_T,
    len: c_int,
    mut idx: varnumber_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let len64 = i64::from(len);
        if idx < 0 {
            idx += len64;
        }
        if idx < len64 && idx >= 0 {
            let v = i64::from(tv_blob_get_inline((*rettv).vval.v_blob(), idx as c_int));
            tv_clear(rettv);
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval = crate::eval::typval::TypvalVval::Number(v);
        } else {
            semsg!(tr(e_blobidx.as_ptr()), idx);
            return FAIL;
        }

        OK
    }
}

pub unsafe fn tv_blob_slice_or_index(
    blob: *const blob_T,
    is_range: bool,
    n1: varnumber_T,
    n2: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let len = tv_blob_len_inline((*rettv).vval.v_blob());
        if is_range {
            tv_blob_slice(blob, len, n1, n2, exclusive, rettv)
        } else {
            tv_blob_index(blob, len, n1, rettv)
        }
    }
}

/// Check if "n1" is a valid index for a blob with length "bloblen".
pub unsafe fn tv_blob_check_index(bloblen: c_int, n1: varnumber_T, quiet: bool) -> c_int {
    unsafe {
        if n1 < 0 || n1 > i64::from(bloblen) {
            if !quiet {
                semsg!(tr(e_blobidx.as_ptr()), n1);
            }
            return FAIL;
        }
        OK
    }
}

/// Check if "n1"-"n2" is a valid range for a blob with length "bloblen".
pub unsafe fn tv_blob_check_range(
    bloblen: c_int,
    n1: varnumber_T,
    n2: varnumber_T,
    quiet: bool,
) -> c_int {
    unsafe {
        if n2 < 0 || n2 >= i64::from(bloblen) || n2 < n1 {
            if !quiet {
                semsg!(tr(e_blobidx.as_ptr()), n2);
            }
            return FAIL;
        }
        OK
    }
}

/// Set bytes "n1" to "n2" (inclusive) in "dest" to the value of "src".
pub unsafe fn tv_blob_set_range(
    dest: *mut blob_T,
    n1: varnumber_T,
    n2: varnumber_T,
    src: *mut typval_T,
) -> c_int {
    unsafe {
        if n2 - n1 + 1 != i64::from(tv_blob_len_inline((*src).vval.v_blob())) {
            emsg(tr(
                c"E972: Blob value does not have the right number of bytes".as_ptr(),
            ));
            return FAIL;
        }

        for (ir, il) in (0_i32..).zip((n1 as c_int)..=(n2 as c_int)) {
            tv_blob_set_inline(dest, il, tv_blob_get_inline((*src).vval.v_blob(), ir));
        }
        OK
    }
}

/// Store one byte "byte" in blob "blob" at "idx".
pub unsafe fn tv_blob_set_append(blob: *mut blob_T, idx: c_int, byte: u8) {
    unsafe {
        let bv = &mut (*blob).bv_ga;
        let len = bv.len() as c_int;
        if idx < len {
            bv[idx as usize] = byte;
        } else if idx == len {
            bv.push(byte);
        }
    }
}

/// "remove({blob})" function.
pub unsafe fn tv_blob_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        let b = (*argvars).vval.v_blob();
        if !b.is_null() && value_check_lock((*b).bv_lock, arg_errmsg, TV_TRANSLATE) {
            return;
        }

        let mut error = false;
        let mut idx = tv_get_number_chk(argvars.add(1), &raw mut error);

        if !error {
            let len = i64::from(tv_blob_len_inline(b));
            if idx < 0 {
                idx += len;
            }
            if idx < 0 || idx >= len {
                semsg!(tr(e_blobidx.as_ptr()), idx);
                return;
            }
            if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
                let bv = &mut (*b).bv_ga;
                let removed = bv.remove(idx as usize);
                (*rettv).vval = crate::eval::typval::TypvalVval::Number(i64::from(removed));
            } else {
                let mut end = tv_get_number_chk(argvars.add(2), &raw mut error);
                if error {
                    return;
                }
                if end < 0 {
                    end += len;
                }
                if end >= len || idx > end {
                    semsg!(tr(e_blobidx.as_ptr()), end);
                    return;
                }
                let blob = tv_blob_alloc();
                let src_bv = &mut (*b).bv_ga;
                let dst_bv = &mut (*blob).bv_ga;
                dst_bv.extend(src_bv.drain((idx as usize)..=(end as usize)));
                tv_blob_set_ret_inline(rettv, blob);
            }
        }
    }
}

/// blob2list() function.
pub unsafe fn f_blob2list(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if tv_check_for_blob_arg(argvars, 0) == FAIL {
            return;
        }

        let blob = (*argvars).vval.v_blob();
        let l = (*rettv).vval.v_list();
        for i in 0..tv_blob_len_inline(blob) {
            tv_list_append_number(l, i64::from(tv_blob_get_inline(blob, i)));
        }
    }
}

/// list2blob() function.
pub unsafe fn f_list2blob(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let blob = tv_blob_alloc_ret(rettv);

        if tv_check_for_list_arg(argvars, 0) == FAIL {
            return;
        }

        let l = (*argvars).vval.v_list();
        if l.is_null() {
            return;
        }

        let bv = &mut (*blob).bv_ga;
        let list_len = tv_list_len_inline(l);
        if list_len > 0 {
            bv.reserve(list_len as usize);
        }

        let mut li = (*l).lv_first;
        while !li.is_null() {
            let mut error = false;
            let n = tv_get_number_chk(&raw const (*li).li_tv, &raw mut error);
            if error || !(0..=255).contains(&n) {
                if !error {
                    semsg!(tr(e_invalid_value_for_blob_nr.as_ptr()), n as c_int);
                }
                bv.clear();
                return;
            }
            bv.push(n as u8);
            li = (*li).li_next;
        }
    }
}

/// Allocate an empty list for a return value.
pub unsafe fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(len);
        tv_list_set_ret_inline(ret_tv, l);
        (*ret_tv).v_lock = VAR_UNLOCKED;
        l
    }
}

pub unsafe fn tv_dict_alloc_lock(lock: VarLockStatus) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();
        (*d).dv_lock = lock;
        d
    }
}

/// Allocate an empty dictionary for a return value.
pub unsafe fn tv_dict_alloc_ret(ret_tv: *mut typval_T) {
    unsafe {
        let d = tv_dict_alloc_lock(VAR_UNLOCKED);
        tv_dict_set_ret_inline(ret_tv, d);
    }
}

unsafe fn tv_dict2list(argvars: *mut typval_T, rettv: *mut typval_T, what: DictListType) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            tv_list_alloc_ret(rettv, 0);
            return;
        }

        let d = (*argvars).vval.v_dict();
        tv_list_alloc_ret(rettv, tv_dict_len_inline(d));
        if d.is_null() {
            return;
        }

        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                let key_ptr = (*di).di_key.as_ptr();
                let mut tv_item = tv_initial_value();

                match what {
                    DictListType::Keys => {
                        tv_set_string(&raw mut tv_item, key_ptr);
                    }
                    DictListType::Values => {
                        tv_copy(&raw const (*di).di_tv, &raw mut tv_item);
                    }
                    DictListType::Items => {
                        let sub_l = tv_list_alloc(2);
                        tv_item.v_type = VarType::VAR_LIST;
                        tv_item.vval = crate::eval::typval::TypvalVval::List(sub_l);
                        tv_list_ref_inline(sub_l);
                        tv_list_append_string(sub_l, key_ptr, -1);
                        tv_list_append_tv(sub_l, &raw mut (*di).di_tv);
                    }
                }

                tv_list_append_owned_tv((*rettv).vval.v_list(), tv_item);
            }
            hi = hi.add(1);
        }
    }
}

/// "items()" function.
pub unsafe fn f_items(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        match (*argvars).v_type {
            VarType::VAR_STRING => tv_string2items(argvars, rettv),
            VarType::VAR_LIST => tv_list2items(argvars, rettv),
            VarType::VAR_BLOB => tv_blob2items(argvars, rettv),
            VarType::VAR_DICT => tv_dict2items(argvars, rettv),
            _ => {
                semsg!(
                    tr(E_LIST_DICT_BLOB_OR_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                    1,
                );
            }
        }
    }
}

/// "keys()" function.
pub unsafe fn f_keys(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Keys);
    }
}

/// "values(dict)" function.
pub unsafe fn f_values(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Values);
    }
}

/// "has_key()" function.
pub unsafe fn f_has_key(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            return;
        }

        if (*argvars).vval.v_dict().is_null() {
            return;
        }

        (*rettv).vval = crate::eval::typval::TypvalVval::Number(i64::from(
            !tv_dict_find((*argvars).vval.v_dict(), tv_get_string(argvars.add(1)), -1).is_null(),
        ));
    }
}

/// "remove({dict})" function.
pub unsafe fn tv_dict_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            semsg!(tr(e_toomanyarg.as_ptr()), c"remove()".as_ptr());
        } else {
            let d = (*argvars).vval.v_dict();
            if !d.is_null() && !value_check_lock((*d).dv_lock, arg_errmsg, TV_TRANSLATE) {
                let key = tv_get_string_chk(argvars.add(1));
                if !key.is_null() {
                    let di = tv_dict_find(d, key, -1);
                    if di.is_null() {
                        semsg!(tr(e_dictkey.as_ptr()), key);
                    } else if !var_check_fixed(
                        c_int::from((*di).di_flags),
                        arg_errmsg,
                        TV_TRANSLATE,
                    ) && !var_check_ro(
                        c_int::from((*di).di_flags),
                        arg_errmsg,
                        TV_TRANSLATE,
                    ) {
                        *rettv = (*di).di_tv;
                        (*di).di_tv = tv_initial_value();
                        tv_dict_item_remove(d, di);
                        if tv_dict_is_watched_inline(d) {
                            tv_dict_watcher_notify(d, key, ptr::null_mut(), rettv);
                        }
                    }
                }
            }
        }
    }
}

/// Allocate an empty blob for a return value.
pub unsafe fn tv_blob_alloc_ret(ret_tv: *mut typval_T) -> *mut blob_T {
    unsafe {
        let b = tv_blob_alloc();
        tv_blob_set_ret_inline(ret_tv, b);
        b
    }
}

/// Copy a blob typval to a different typval.
pub unsafe fn tv_blob_copy(from: *mut blob_T, to: *mut typval_T) {
    unsafe {
        (*to).v_type = VarType::VAR_BLOB;
        (*to).v_lock = VAR_UNLOCKED;
        if from.is_null() {
            (*to).vval = crate::eval::typval::TypvalVval::Blob(ptr::null_mut());
        } else {
            let dst_blob = &mut *tv_blob_alloc_ret(to);
            dst_blob.bv_ga.extend_from_slice(&(*from).bv_ga);
        }
    }
}

#[inline]
unsafe fn nothing_conv_func_start(tv: *mut typval_T, fun: *mut c_char) -> c_int {
    unsafe {
        (*tv).v_lock = VAR_UNLOCKED;
        if (*tv).v_type == VarType::VAR_PARTIAL {
            let pt = (*tv).vval.v_partial();
            if !pt.is_null() && (*pt).pt_refcount > 1 {
                (*pt).pt_refcount -= 1;
                (*tv).vval = crate::eval::typval::TypvalVval::Partial(ptr::null_mut());
                return OK;
            }
        } else {
            func_unref(fun);
            if !ptr::eq(fun.cast_const(), tv_empty_string) {
                tv_free_string(fun);
            }
            (*tv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_func_end(tv: *mut typval_T, copy_id: c_int) {
    unsafe {
        if (*tv).v_type == VarType::VAR_PARTIAL {
            let pt = (*tv).vval.v_partial();
            if pt.is_null() {
                return;
            }
            let dict = (*pt).pt_dict.cast::<dict_T>();
            debug_assert!(dict.is_null() || (*dict).dv_copyID == copy_id);
            (*pt).pt_dict = ptr::null_mut();
            (*pt).pt_argv.clear();
            debug_assert!((*pt).pt_refcount <= 1);
            partial_unref(pt);
            (*tv).vval = crate::eval::typval::TypvalVval::Partial(ptr::null_mut());
            debug_assert_eq!((*tv).v_lock, VAR_UNLOCKED);
        }
    }
}

#[inline]
unsafe fn nothing_conv_empty_dict(tv: *mut typval_T, dictp: *mut *mut dict_T) {
    unsafe {
        tv_dict_unref(*dictp);
        *dictp = ptr::null_mut();
        if !tv.is_null() {
            (*tv).v_lock = VAR_UNLOCKED;
        }
    }
}

#[inline]
unsafe fn nothing_conv_real_list_after_start(
    tv: *mut typval_T,
    mpsv: &mut MPConvStackVal,
) -> c_int {
    unsafe {
        debug_assert!(!tv.is_null());
        (*tv).v_lock = VAR_UNLOCKED;
        let list = (*tv).vval.v_list();
        if (*list).lv_refcount > 1 {
            (*list).lv_refcount -= 1;
            (*tv).vval = crate::eval::typval::TypvalVval::List(ptr::null_mut());
            if let MPConvStackData::List { ref mut li, .. } = mpsv.data {
                *li = ptr::null_mut();
            }
            return OK;
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_list_end(tv: *mut typval_T) {
    unsafe {
        if tv.is_null() {
            return;
        }
        debug_assert_eq!((*tv).v_type, VarType::VAR_LIST);
        let list = (*tv).vval.v_list();
        tv_list_unref(list);
        (*tv).vval = crate::eval::typval::TypvalVval::List(ptr::null_mut());
    }
}

#[inline]
unsafe fn nothing_conv_real_dict_after_start(
    tv: *mut typval_T,
    dictp: *mut *mut dict_T,
    mpsv: &mut MPConvStackVal,
) -> c_int {
    unsafe {
        if !tv.is_null() {
            (*tv).v_lock = VAR_UNLOCKED;
        }
        if !dictp.is_null() && (**dictp).dv_refcount > 1 {
            (**dictp).dv_refcount -= 1;
            *dictp = ptr::null_mut();
            if let MPConvStackData::Dict { ref mut todo, .. } = mpsv.data {
                *todo = 0;
            }
            return OK;
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_dict_end(dictp: *mut *mut dict_T) {
    unsafe {
        if !dictp.is_null() {
            tv_dict_unref(*dictp);
            *dictp = ptr::null_mut();
        }
    }
}

unsafe fn nothing_convert_one_value(
    mpstack: &mut Vec<MPConvStackVal>,
    tv: *mut typval_T,
    copy_id: c_int,
) -> c_int {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_STRING => {
                tv_free_string((*tv).vval.v_string());
                (*tv).vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_NUMBER => {
                (*tv).vval = crate::eval::typval::TypvalVval::Number(0);
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_FLOAT => {
                (*tv).vval = crate::eval::typval::TypvalVval::Float(0.0);
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_BLOB => {
                tv_blob_unref((*tv).vval.v_blob());
                (*tv).vval = crate::eval::typval::TypvalVval::Blob(ptr::null_mut());
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_FUNC => {
                let _ = nothing_conv_func_start(tv, (*tv).vval.v_string());
            }
            VarType::VAR_PARTIAL => {
                let pt = (*tv).vval.v_partial();
                let fun = if pt.is_null() {
                    ptr::null_mut()
                } else {
                    partial_name(pt)
                };
                if nothing_conv_func_start(tv, fun) != NOTDONE {
                    return OK;
                }
                mpstack.push(MPConvStackVal {
                    tv,
                    saved_copy_id: copy_id - 1,
                    data: MPConvStackData::Partial {
                        stage: MPConvPartialStage::Args,
                        pt,
                    },
                });
            }
            VarType::VAR_LIST => {
                let list = (*tv).vval.v_list();
                if list.is_null() || tv_list_len_inline(list) == 0 {
                    tv_list_unref(list);
                    (*tv).vval = crate::eval::typval::TypvalVval::List(ptr::null_mut());
                    (*tv).v_lock = VAR_UNLOCKED;
                    return OK;
                }
                let saved_copy_id = (*list).lv_copyID;
                if (*list).lv_copyID == copy_id {
                    return OK;
                }
                (*list).lv_copyID = copy_id;
                let mut mpsv = MPConvStackVal {
                    tv,
                    saved_copy_id,
                    data: MPConvStackData::List {
                        list,
                        li: tv_list_first_inline(list),
                    },
                };
                let _ = nothing_conv_real_list_after_start(tv, &mut mpsv);
                mpstack.push(mpsv);
            }
            VarType::VAR_BOOL => {
                (*tv).vval = crate::eval::typval::TypvalVval::Bool(kBoolVarFalse);
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_SPECIAL => {
                (*tv).vval = crate::eval::typval::TypvalVval::Special(kSpecialVarNull);
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_DICT => {
                let dict = (*tv).vval.v_dict();
                let dictp = (*tv).vval.v_dict_mut();
                if dict.is_null() || (*dict).dv_hashtab.ht_used == 0 {
                    nothing_conv_empty_dict(tv, dictp);
                    return OK;
                }
                let saved_copy_id = (*dict).dv_copyID;
                if (*dict).dv_copyID == copy_id {
                    return OK;
                }
                (*dict).dv_copyID = copy_id;
                let mut mpsv = MPConvStackVal {
                    tv,
                    saved_copy_id,
                    data: MPConvStackData::Dict {
                        dict,
                        dictp,
                        hi: (*dict).dv_hashtab.ht_array,
                        todo: (*dict).dv_hashtab.ht_used,
                    },
                };
                let _ = nothing_conv_real_dict_after_start(tv, dictp, &mut mpsv);
                mpstack.push(mpsv);
            }
            VarType::VAR_UNKNOWN => {
                internal_error(c"_typval_encode_nothing_convert_one_value()".as_ptr());
                return FAIL;
            }
        }
        OK
    }
}

unsafe fn encode_vim_to_nothing(top_tv: *mut typval_T) -> c_int {
    unsafe {
        let copy_id = get_copyID();

        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);
        if nothing_convert_one_value(&mut mpstack, top_tv, copy_id) == FAIL {
            return FAIL;
        }

        while !mpstack.is_empty() {
            let last_idx = mpstack.len() - 1;
            let saved_copy_id = mpstack[last_idx].saved_copy_id;
            let cur_tv = mpstack[last_idx].tv;
            let tv: *mut typval_T = match mpstack[last_idx].data {
                MPConvStackData::Dict {
                    dict,
                    dictp,
                    ref mut hi,
                    ref mut todo,
                } => {
                    if *todo == 0 {
                        mpstack.pop();
                        (*dict).dv_copyID = saved_copy_id;
                        nothing_conv_dict_end(dictp);
                        continue;
                    }
                    while hashitem_empty(&**hi) {
                        *hi = (*hi).add(1);
                    }
                    let di = tv_dict_hi2di(*hi);
                    *todo -= 1;
                    *hi = (*hi).add(1);
                    &raw mut (*di).di_tv
                }
                MPConvStackData::List { list, ref mut li } => {
                    let cur_li = *li;
                    if cur_li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        nothing_conv_list_end(cur_tv);
                        continue;
                    }
                    *li = (*cur_li).li_next;
                    &raw mut (*cur_li).li_tv
                }
                MPConvStackData::Partial { stage, pt } => {
                    match stage {
                        MPConvPartialStage::Args => {
                            if let MPConvStackData::Partial {
                                stage: ref mut stage_ref,
                                ..
                            } = mpstack[last_idx].data
                            {
                                *stage_ref = MPConvPartialStage::SelfDict;
                            }
                            if !pt.is_null() && !(*pt).pt_argv.is_empty() {
                                mpstack.push(MPConvStackVal {
                                    tv: ptr::null_mut(),
                                    saved_copy_id: copy_id - 1,
                                    data: MPConvStackData::PartialList {
                                        arg: (*pt).pt_argv.as_mut_ptr(),
                                        argv: (*pt).pt_argv.as_mut_ptr(),
                                        todo: (*pt).pt_argv.len(),
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
                            let dict = if pt.is_null() {
                                ptr::null_mut()
                            } else {
                                (*pt).pt_dict.cast::<dict_T>()
                            };
                            if !dict.is_null() {
                                let dictp = (&raw mut (*pt).pt_dict).cast::<*mut dict_T>();
                                if (*dict).dv_hashtab.ht_used == 0 {
                                    nothing_conv_empty_dict(ptr::null_mut(), dictp);
                                    continue;
                                }
                                let saved_copy_id = (*dict).dv_copyID;
                                if (*dict).dv_copyID == copy_id {
                                    continue;
                                }
                                (*dict).dv_copyID = copy_id;
                                let mut mpsv = MPConvStackVal {
                                    tv: ptr::null_mut(),
                                    saved_copy_id,
                                    data: MPConvStackData::Dict {
                                        dict,
                                        dictp,
                                        hi: (*dict).dv_hashtab.ht_array,
                                        todo: (*dict).dv_hashtab.ht_used,
                                    },
                                };
                                let _ = nothing_conv_real_dict_after_start(
                                    ptr::null_mut(),
                                    dictp,
                                    &mut mpsv,
                                );
                                mpstack.push(mpsv);
                            }
                        }
                        MPConvPartialStage::End => {
                            nothing_conv_func_end(cur_tv, copy_id);
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
                        nothing_conv_list_end(cur_tv);
                        continue;
                    }
                    *arg = cur_arg.add(1);
                    *todo -= 1;
                    cur_arg
                }
                MPConvStackData::Pairs { .. } => unreachable!(),
            };
            debug_assert!(!tv.is_null());
            if nothing_convert_one_value(&mut mpstack, tv, copy_id) == FAIL {
                return FAIL;
            }
        }
        OK
    }
}

/// Free memory for a variable value and set the value to NULL or 0.
pub unsafe fn tv_clear(tv: *mut typval_T) {
    unsafe {
        if tv.is_null() || (*tv).v_type == VarType::VAR_UNKNOWN {
            return;
        }
        let evn_ret = encode_vim_to_nothing(tv);
        debug_assert_eq!(evn_ret, OK);
    }
}

/// Copy typval from one location to another.
pub unsafe fn tv_copy(from: *const typval_T, to: *mut typval_T) {
    unsafe {
        (*to).v_type = (*from).v_type;
        (*to).v_lock = VAR_UNLOCKED;
        (*to).vval = (*from).vval;
        match (*from).v_type {
            VarType::VAR_NUMBER | VarType::VAR_FLOAT | VarType::VAR_BOOL | VarType::VAR_SPECIAL => {
            }
            VarType::VAR_STRING | VarType::VAR_FUNC => {
                if !(*from).vval.v_string().is_null() {
                    (*to).vval =
                        crate::eval::typval::TypvalVval::String(tv_alloc_string((*from).vval.v_string()));
                    if (*from).v_type == VarType::VAR_FUNC {
                        func_ref((*to).vval.v_string());
                    }
                }
            }
            VarType::VAR_PARTIAL => {
                let pt = (*to).vval.v_partial();
                if !pt.is_null() {
                    (*pt).pt_refcount += 1;
                }
            }
            VarType::VAR_BLOB => {
                if !(*from).vval.v_blob().is_null() {
                    (*(*from).vval.v_blob()).bv_refcount += 1;
                }
            }
            VarType::VAR_LIST => {
                tv_list_ref_inline((*to).vval.v_list());
            }
            VarType::VAR_DICT => {
                if !(*from).vval.v_dict().is_null() {
                    (*(*from).vval.v_dict()).dv_refcount += 1;
                }
            }
            VarType::VAR_UNKNOWN => {
                semsg!(tr(e_intern2.as_ptr()), c"tv_copy(UNKNOWN)".as_ptr());
            }
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
pub unsafe fn tv_item_lock(tv: *mut typval_T, deep: c_int, lock: bool, check_refcount: bool) {
    unsafe {
        static mut RECURSE: c_int = 0;

        if RECURSE >= DICT_MAXNEST {
            emsg(tr(E_VARIABLE_NESTED_TOO_DEEP_FOR_UNLOCK.as_ptr()));
            return;
        }
        if deep == 0 {
            return;
        }
        RECURSE += 1;

        (*tv).v_lock = change_lock(lock, (*tv).v_lock);

        match (*tv).v_type {
            VarType::VAR_BLOB => {
                let b = (*tv).vval.v_blob();
                if !b.is_null() && !(check_refcount && (*b).bv_refcount > 1) {
                    (*b).bv_lock = change_lock(lock, (*b).bv_lock);
                }
            }
            VarType::VAR_LIST => {
                let l = (*tv).vval.v_list();
                if !l.is_null() && !(check_refcount && (*l).lv_refcount > 1) {
                    (*l).lv_lock = change_lock(lock, (*l).lv_lock);
                    if !(0..=1).contains(&deep) {
                        let mut li = (*l).lv_first;
                        while !li.is_null() {
                            tv_item_lock(&raw mut (*li).li_tv, deep - 1, lock, check_refcount);
                            li = (*li).li_next;
                        }
                    }
                }
            }
            VarType::VAR_DICT => {
                let d = (*tv).vval.v_dict();
                if !d.is_null() && !(check_refcount && (*d).dv_refcount > 1) {
                    (*d).dv_lock = change_lock(lock, (*d).dv_lock);
                    if !(0..=1).contains(&deep) {
                        let mut todo = (*d).dv_hashtab.ht_used;
                        let mut hi = (*d).dv_hashtab.ht_array;
                        while todo > 0 {
                            if !hashitem_empty(&*hi) {
                                todo -= 1;
                                let di = tv_dict_hi2di(hi);
                                tv_item_lock(&raw mut (*di).di_tv, deep - 1, lock, check_refcount);
                            }
                            hi = hi.add(1);
                        }
                    }
                }
            }
            VarType::VAR_NUMBER
            | VarType::VAR_FLOAT
            | VarType::VAR_STRING
            | VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL => {}
            VarType::VAR_UNKNOWN => std::process::abort(),
        }
        RECURSE -= 1;
    }
}

/// Check whether Vimscript value is locked itself or refers to a locked container.
pub unsafe fn tv_islocked(tv: *const typval_T) -> bool {
    unsafe {
        (*tv).v_lock == VAR_LOCKED
            || ((*tv).v_type == VarType::VAR_LIST
                && tv_list_locked_inline((*tv).vval.v_list()) == VAR_LOCKED)
            || ((*tv).v_type == VarType::VAR_DICT
                && !(*tv).vval.v_dict().is_null()
                && (*(*tv).vval.v_dict()).dv_lock == VAR_LOCKED)
    }
}

/// Return true if typval is locked.
pub unsafe fn tv_check_lock(tv: *const typval_T, name: *const c_char, name_len: usize) -> bool {
    unsafe {
        let mut lock = VAR_UNLOCKED;

        match (*tv).v_type {
            VarType::VAR_BLOB if !(*tv).vval.v_blob().is_null() => {
                lock = (*(*tv).vval.v_blob()).bv_lock;
            }
            VarType::VAR_LIST if !(*tv).vval.v_list().is_null() => {
                lock = (*(*tv).vval.v_list()).lv_lock;
            }
            VarType::VAR_DICT if !(*tv).vval.v_dict().is_null() => {
                lock = (*(*tv).vval.v_dict()).dv_lock;
            }
            _ => {}
        }
        value_check_lock((*tv).v_lock, name, name_len)
            || (lock != VAR_UNLOCKED && value_check_lock(lock, name, name_len))
    }
}

/// Return true if variable "name" has a locked (immutable) value.
pub unsafe fn value_check_lock(
    lock: VarLockStatus,
    mut name: *const c_char,
    mut name_len: usize,
) -> bool {
    unsafe {
        match lock {
            VAR_UNLOCKED => return false,
            VAR_LOCKED => {
                let error_message = if name.is_null() {
                    e_value_is_locked.as_ptr()
                } else {
                    e_value_is_locked_str.as_ptr()
                };
                if name.is_null() {
                    emsg(tr(error_message));
                } else {
                    if name_len == TV_TRANSLATE {
                        name = tr(name);
                        name_len = strlen(name);
                    } else if name_len == TV_CSTRING {
                        name_len = strlen(name);
                    }
                    semsg!(tr(error_message), name_len as c_int, name);
                }
            }
            VAR_FIXED => {
                let error_message = if name.is_null() {
                    e_cannot_change_value.as_ptr()
                } else {
                    e_cannot_change_value_of_str.as_ptr()
                };
                if name.is_null() {
                    emsg(tr(error_message));
                } else {
                    if name_len == TV_TRANSLATE {
                        name = tr(name);
                        name_len = strlen(name);
                    } else if name_len == TV_CSTRING {
                        name_len = strlen(name);
                    }
                    semsg!(tr(error_message), name_len as c_int, name);
                }
            }
            _ => {
                let _ = (&mut name, &mut name_len);
            }
        }
        true
    }
}

/// Compare two Vimscript values.
pub unsafe fn tv_equal(tv1: *mut typval_T, tv2: *mut typval_T, ic: bool) -> bool {
    unsafe {
        static mut RECURSIVE_CNT: c_int = 0;

        if !(tv_is_func_inline(*tv1) && tv_is_func_inline(*tv2)) && (*tv1).v_type != (*tv2).v_type {
            return false;
        }

        if RECURSIVE_CNT == 0 {
            TV_EQUAL_RECURSE_LIMIT = 1000;
        }
        if RECURSIVE_CNT >= TV_EQUAL_RECURSE_LIMIT {
            TV_EQUAL_RECURSE_LIMIT -= 1;
            return true;
        }

        match (*tv1).v_type {
            VarType::VAR_LIST => {
                RECURSIVE_CNT += 1;
                let r = tv_list_equal((*tv1).vval.v_list(), (*tv2).vval.v_list(), ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_DICT => {
                RECURSIVE_CNT += 1;
                let r = tv_dict_equal((*tv1).vval.v_dict(), (*tv2).vval.v_dict(), ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                if ((*tv1).v_type == VarType::VAR_PARTIAL && (*tv1).vval.v_partial().is_null())
                    || ((*tv2).v_type == VarType::VAR_PARTIAL && (*tv2).vval.v_partial().is_null())
                {
                    return false;
                }
                RECURSIVE_CNT += 1;
                let r = func_equal(tv1, tv2, ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_BLOB => tv_blob_equal((*tv1).vval.v_blob(), (*tv2).vval.v_blob()),
            VarType::VAR_NUMBER => (*tv1).vval.v_number() == (*tv2).vval.v_number(),
            VarType::VAR_FLOAT => (*tv1).vval.v_float() == (*tv2).vval.v_float(),
            VarType::VAR_STRING => {
                let mut buf1: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                let mut buf2: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                let s1 = tv_get_string_buf(tv1, buf1.as_mut_ptr());
                let s2 = tv_get_string_buf(tv2, buf2.as_mut_ptr());
                mb_strcmp_ic(ic, s1, s2) == 0
            }
            VarType::VAR_BOOL => (*tv1).vval.v_bool() == (*tv2).vval.v_bool(),
            VarType::VAR_SPECIAL => (*tv1).vval.v_special() == (*tv2).vval.v_special(),
            VarType::VAR_UNKNOWN => false,
        }
    }
}

/// Check that given value is a number or string.
pub unsafe fn tv_check_str_or_nr(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER | VarType::VAR_STRING => true,
            VarType::VAR_FLOAT => {
                emsg(tr(
                    c"E805: Expected a Number or a String, Float found".as_ptr()
                ));
                false
            }
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                emsg(tr(
                    c"E703: Expected a Number or a String, Funcref found".as_ptr()
                ));
                false
            }
            VarType::VAR_LIST => {
                emsg(tr(
                    c"E745: Expected a Number or a String, List found".as_ptr()
                ));
                false
            }
            VarType::VAR_DICT => {
                emsg(tr(
                    c"E728: Expected a Number or a String, Dictionary found".as_ptr()
                ));
                false
            }
            VarType::VAR_BLOB => {
                emsg(tr(
                    c"E974: Expected a Number or a String, Blob found".as_ptr()
                ));
                false
            }
            VarType::VAR_BOOL => {
                emsg(tr(
                    c"E5299: Expected a Number or a String, Boolean found".as_ptr()
                ));
                false
            }
            VarType::VAR_SPECIAL => {
                emsg(tr(c"E5300: Expected a Number or a String".as_ptr()));
                false
            }
            VarType::VAR_UNKNOWN => {
                semsg!(
                    tr(e_intern2.as_ptr()),
                    c"tv_check_str_or_nr(UNKNOWN)".as_ptr(),
                );
                false
            }
        }
    }
}

#[inline]
const fn num_error(vt: VarType) -> *const c_char {
    match vt {
        VarType::VAR_PARTIAL | VarType::VAR_FUNC => c"E703: Using a Funcref as a Number".as_ptr(),
        VarType::VAR_LIST => c"E745: Using a List as a Number".as_ptr(),
        VarType::VAR_DICT => c"E728: Using a Dictionary as a Number".as_ptr(),
        VarType::VAR_FLOAT => c"E805: Using a Float as a Number".as_ptr(),
        VarType::VAR_BLOB => c"E974: Using a Blob as a Number".as_ptr(),
        VarType::VAR_UNKNOWN => c"E685: using an invalid value as a Number".as_ptr(),
        _ => c"".as_ptr(),
    }
}

#[inline]
const fn str_error(vt: VarType) -> *const c_char {
    match vt {
        VarType::VAR_PARTIAL | VarType::VAR_FUNC => c"E729: Using a Funcref as a String".as_ptr(),
        VarType::VAR_LIST => c"E730: Using a List as a String".as_ptr(),
        VarType::VAR_DICT => c"E731: Using a Dictionary as a String".as_ptr(),
        VarType::VAR_BLOB => c"E976: Using a Blob as a String".as_ptr(),
        VarType::VAR_UNKNOWN => E_USING_INVALID_VALUE_AS_STRING.as_ptr(),
        _ => c"".as_ptr(),
    }
}

/// Check that given value is a number or can be converted to it.
pub unsafe fn tv_check_num(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_STRING => true,
            VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_FLOAT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(num_error((*tv).v_type)));
                false
            }
        }
    }
}

/// Check that given value is a Vimscript String or can be "cast" to it.
pub unsafe fn tv_check_str(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_STRING
            | VarType::VAR_FLOAT => true,
            VarType::VAR_PARTIAL
            | VarType::VAR_FUNC
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(str_error((*tv).v_type)));
                false
            }
        }
    }
}

/// Get the number value of a Vimscript object.
pub unsafe fn tv_get_number(tv: *const typval_T) -> varnumber_T {
    unsafe {
        let mut error = false;
        tv_get_number_chk(tv, &raw mut error)
    }
}

/// Get the number value of a Vimscript object.
pub unsafe fn tv_get_number_chk(tv: *const typval_T, ret_error: *mut bool) -> varnumber_T {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_FLOAT => {
                emsg(tr(num_error((*tv).v_type)));
            }
            VarType::VAR_NUMBER => return (*tv).vval.v_number(),
            VarType::VAR_STRING => {
                let mut n: varnumber_T = 0;
                if !(*tv).vval.v_string().is_null() {
                    vim_str2nr(
                        (*tv).vval.v_string(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        STR2NR_ALL,
                        &raw mut n,
                        ptr::null_mut(),
                        0,
                        false,
                        ptr::null_mut(),
                    );
                }
                return n;
            }
            VarType::VAR_BOOL => return i64::from((*tv).vval.v_bool() == kBoolVarTrue),
            VarType::VAR_SPECIAL => return 0,
            VarType::VAR_UNKNOWN => {
                semsg!(tr(e_intern2.as_ptr()), c"tv_get_number(UNKNOWN)".as_ptr());
            }
        }
        if !ret_error.is_null() {
            *ret_error = true;
        }
        if ret_error.is_null() { -1 } else { 0 }
    }
}

/// Get the boolean value of a Vimscript object.
pub unsafe fn tv_get_bool(tv: *const typval_T) -> varnumber_T {
    unsafe { tv_get_number_chk(tv, ptr::null_mut()) }
}

pub unsafe fn tv_get_bool_chk(tv: *const typval_T, ret_error: *mut bool) -> varnumber_T {
    unsafe { tv_get_number_chk(tv, ret_error) }
}

/// Get the line number from Vimscript object.
pub unsafe fn tv_get_lnum(tv: *const typval_T) -> linenr_T {
    unsafe {
        let did_emsg_before = did_emsg;
        let mut lnum = tv_get_number_chk(tv, ptr::null_mut()) as linenr_T;
        if lnum <= 0 && did_emsg_before == did_emsg && (*tv).v_type != VarType::VAR_NUMBER {
            let mut fnum: c_int = 0;
            let fp = var2fpos(tv, true, &raw mut fnum, false, curwin);
            if !fp.is_null() {
                lnum = (*fp).lnum;
            }
        }
        lnum
    }
}

/// Get the line number from Vimscript object.
pub unsafe fn tv_get_lnum_buf(tv: *const typval_T, buf: *const buf_T) -> linenr_T {
    unsafe {
        if (*tv).v_type == VarType::VAR_STRING
            && !(*tv).vval.v_string().is_null()
            && *(*tv).vval.v_string() == b'$' as c_char
            && *(*tv).vval.v_string().add(1) == NUL
            && !buf.is_null()
        {
            return (*buf).b_ml.ml_line_count;
        }
        tv_get_number_chk(tv, ptr::null_mut()) as linenr_T
    }
}

/// Get the floating-point value of a Vimscript object.
pub unsafe fn tv_get_float(tv: *const typval_T) -> float_T {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => (*tv).vval.v_number() as float_T,
            VarType::VAR_FLOAT => (*tv).vval.v_float(),
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                emsg(tr(c"E891: Using a Funcref as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_STRING => {
                emsg(tr(c"E892: Using a String as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_LIST => {
                emsg(tr(c"E893: Using a List as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_DICT => {
                emsg(tr(c"E894: Using a Dictionary as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_BOOL => {
                emsg(tr(c"E362: Using a boolean value as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_SPECIAL => {
                emsg(tr(c"E907: Using a special value as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_BLOB => {
                emsg(tr(c"E975: Using a Blob as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_UNKNOWN => {
                semsg!(tr(e_intern2.as_ptr()), c"tv_get_float(UNKNOWN)".as_ptr());
                0.0
            }
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string.
pub unsafe fn tv_check_for_string_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_STRING {
            semsg!(tr(E_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a non-empty string.
pub unsafe fn tv_check_for_nonempty_string_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if tv_check_for_string_arg(args, idx) == FAIL {
            return FAIL;
        }
        let s = (*args.offset(idx as isize)).vval.v_string();
        if s.is_null() || *s == NUL {
            semsg!(
                tr(E_NON_EMPTY_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional string argument at "idx".
pub unsafe fn tv_check_for_opt_string_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_string_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a number.
pub unsafe fn tv_check_for_number_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_NUMBER {
            semsg!(tr(E_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Check for an optional number argument at "idx".
pub unsafe fn tv_check_for_opt_number_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_number_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a float or a number.
pub unsafe fn tv_check_for_float_or_nr_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_FLOAT && vt != VarType::VAR_NUMBER {
            semsg!(
                tr(E_FLOAT_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a bool.
pub unsafe fn tv_check_for_bool_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let arg = &*args.offset(idx as isize);
        if arg.v_type != VarType::VAR_BOOL
            && !(arg.v_type == VarType::VAR_NUMBER
                && (arg.vval.v_number() == 0 || arg.vval.v_number() == 1))
        {
            semsg!(tr(E_BOOL_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Check for an optional bool argument at "idx".
pub unsafe fn tv_check_for_opt_bool_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN {
            return OK;
        }
        tv_check_for_bool_arg(args, idx)
    }
}

/// Give an error and return FAIL unless "args[idx]" is a blob.
pub unsafe fn tv_check_for_blob_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_BLOB {
            semsg!(tr(E_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a list.
pub unsafe fn tv_check_for_list_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_LIST {
            semsg!(tr(E_LIST_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a dict.
pub unsafe fn tv_check_for_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_DICT {
            semsg!(tr(E_DICT_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a non-NULL dict.
pub unsafe fn tv_check_for_nonnull_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if tv_check_for_dict_arg(args, idx) == FAIL {
            return FAIL;
        }
        if (*args.offset(idx as isize)).vval.v_dict().is_null() {
            semsg!(
                tr(E_NON_NULL_DICT_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional dict argument at "idx".
pub unsafe fn tv_check_for_opt_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_dict_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a number.
pub unsafe fn tv_check_for_string_or_number_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_NUMBER {
            semsg!(
                tr(E_STRING_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a buffer number.
pub unsafe fn tv_check_for_buffer_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe { tv_check_for_string_or_number_arg(args, idx) }
}

/// Give an error and return FAIL unless "args[idx]" is a line number.
pub unsafe fn tv_check_for_lnum_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe { tv_check_for_string_or_number_arg(args, idx) }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a list.
pub unsafe fn tv_check_for_string_or_list_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_LIST {
            semsg!(
                tr(E_STRING_OR_LIST_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string, a list or a blob.
pub unsafe fn tv_check_for_string_or_list_or_blob_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_LIST && vt != VarType::VAR_BLOB {
            semsg!(
                tr(E_STRING_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional string or list argument at "idx".
pub unsafe fn tv_check_for_opt_string_or_list_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_string_or_list_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a function reference.
pub unsafe fn tv_check_for_string_or_func_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_PARTIAL && vt != VarType::VAR_FUNC && vt != VarType::VAR_STRING {
            semsg!(
                tr(E_STRING_OR_FUNCTION_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a list or a blob.
pub unsafe fn tv_check_for_list_or_blob_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_LIST && vt != VarType::VAR_BLOB {
            semsg!(
                tr(E_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub unsafe fn tv_get_string_buf_chk(tv: *const typval_T, buf: *mut c_char) -> *const c_char {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => {
                use std::io::Write;
                let mut slice = std::slice::from_raw_parts_mut(buf.cast::<u8>(), NUMBUFLEN);
                let _ = write!(slice, "{}\0", (*tv).vval.v_number());
                buf
            }
            VarType::VAR_FLOAT => {
                vim_snprintf!(buf, NUMBUFLEN, c"%g".as_ptr(), (*tv).vval.v_float());
                buf
            }
            VarType::VAR_STRING => {
                if !(*tv).vval.v_string().is_null() {
                    (*tv).vval.v_string()
                } else {
                    c"".as_ptr()
                }
            }
            VarType::VAR_BOOL => {
                xstrlcpy(
                    buf,
                    encode_bool_var_names[(*tv).vval.v_bool() as usize],
                    NUMBUFLEN,
                );
                buf
            }
            VarType::VAR_SPECIAL => {
                xstrlcpy(
                    buf,
                    encode_special_var_names[(*tv).vval.v_special() as usize],
                    NUMBUFLEN,
                );
                buf
            }
            VarType::VAR_PARTIAL
            | VarType::VAR_FUNC
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(str_error((*tv).v_type)));
                ptr::null()
            }
        }
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub unsafe fn tv_get_string_chk(tv: *const typval_T) -> *const c_char {
    unsafe {
        static mut MYBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        tv_get_string_buf_chk(tv, (&raw mut MYBUF).cast::<c_char>())
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub unsafe fn tv_get_string(tv: *const typval_T) -> *const c_char {
    unsafe {
        static mut MYBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        tv_get_string_buf(tv, (&raw mut MYBUF).cast::<c_char>())
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub unsafe fn tv_get_string_buf(tv: *const typval_T, buf: *mut c_char) -> *const c_char {
    unsafe {
        let res = tv_get_string_buf_chk(tv, buf);
        if !res.is_null() { res } else { c"".as_ptr() }
    }
}

/// Return true when "tv" is not falsy.
pub unsafe fn tv2bool(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => (*tv).vval.v_number() != 0,
            VarType::VAR_FLOAT => (*tv).vval.v_float() != 0.0,
            VarType::VAR_PARTIAL => !(*tv).vval.v_partial().is_null(),
            VarType::VAR_FUNC | VarType::VAR_STRING => {
                !(*tv).vval.v_string().is_null() && *(*tv).vval.v_string() != NUL
            }
            VarType::VAR_LIST => !(*tv).vval.v_list().is_null() && (*(*tv).vval.v_list()).lv_len > 0,
            VarType::VAR_DICT => {
                !(*tv).vval.v_dict().is_null() && (*(*tv).vval.v_dict()).dv_hashtab.ht_used > 0
            }
            VarType::VAR_BOOL => (*tv).vval.v_bool() == kBoolVarTrue,
            VarType::VAR_SPECIAL => (*tv).vval.v_special() != kSpecialVarNull,
            VarType::VAR_BLOB => {
                !(*tv).vval.v_blob().is_null() && !(*(*tv).vval.v_blob()).bv_ga.is_empty()
            }
            VarType::VAR_UNKNOWN => false,
        }
    }
}

/// Check that repeating "slen" items "n" times does not overflow int.
pub unsafe fn check_repeat_count(slen: varnumber_T, n: varnumber_T) -> c_int {
    unsafe {
        if n <= i64::from(c_int::MAX) / slen {
            return OK;
        }

        let mut buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"%lld".as_ptr(), n);
        semsg!(tr(e_val_too_large.as_ptr()), buf.as_ptr());
        FAIL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tv_list_operations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            crate::globals::emsg_skip += 1;

            // Basic alloc, append_number, find, reverse
            let l = tv_list_alloc(0);
            assert!(!l.is_null());
            (*l).lv_refcount = 1;
            tv_list_append_number(l, 10);
            tv_list_append_number(l, 20);
            tv_list_append_number(l, 30);
            assert_eq!(tv_list_len_inline(l), 3);

            let item1 = tv_list_find(l, 1);
            assert!(!item1.is_null());
            assert_eq!((*item1).li_tv.vval.v_number(), 20);
            assert_eq!(tv_list_idx_of_item(l, item1), 1);
            assert_eq!(tv_list_idx_of_item(l, ptr::null()), -1);

            let mut err = false;
            assert_eq!(tv_list_find_nr(l, 0, &raw mut err), 10);
            assert!(!err);
            assert_eq!(tv_list_find_nr(l, -1, &raw mut err), 30);
            assert!(!err);
            assert_eq!(tv_list_find_nr(l, 10, &raw mut err), -1);
            assert!(err);

            tv_list_reverse(l);
            let first = tv_list_find(l, 0);
            assert_eq!((*first).li_tv.vval.v_number(), 30);

            // Watchers + item_remove + drop_items + remove_items
            let mut lw1 = listwatch_T {
                lw_item: (*l).lv_first,
            };
            let mut lw2 = listwatch_T {
                lw_item: (*l).lv_last,
            };
            tv_list_watch_add(l, &raw mut lw1);
            tv_list_watch_add(l, &raw mut lw2);

            let second = (*(*l).lv_first).li_next;
            let next_after_first = tv_list_item_remove(l, (*l).lv_first);
            assert_eq!(next_after_first, second);
            assert_eq!(lw1.lw_item, second);
            assert_eq!(tv_list_len_inline(l), 2);

            tv_list_watch_remove(l, &raw mut lw1);
            tv_list_watch_remove(l, &raw mut lw2);
            tv_list_unref(l);

            // insert, insert_tv, append_string, append_allocated_string, append_tv, append_owned_tv
            let l2 = tv_list_alloc(0);
            (*l2).lv_refcount = 1;
            tv_list_append_string(l2, c"hello".as_ptr(), 5);
            tv_list_append_string(l2, c"world".as_ptr(), -1);
            let mut tv_ins = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(99),
            };
            tv_list_insert_tv(l2, &raw mut tv_ins, (*l2).lv_first);
            assert_eq!(tv_list_len_inline(l2), 3);
            assert_eq!(tv_list_find_nr(l2, 0, ptr::null_mut()), 99);
            assert_eq!(CStr::from_ptr(tv_list_find_str(l2, 1)).to_bytes(), b"hello");
            assert_eq!(CStr::from_ptr(tv_list_find_str(l2, 2)).to_bytes(), b"world");

            // copy, extend, concat, join, equal
            let l2_copy = tv_list_copy(None, l2, true, 0);
            (*l2_copy).lv_refcount = 1;
            assert!(tv_list_equal(l2, l2_copy, false));
            assert!(tv_list_equal(ptr::null_mut(), ptr::null_mut(), false));
            assert!(!tv_list_equal(l2, ptr::null_mut(), false));

            let mut joined: Vec<c_char> = Vec::new();
            assert_eq!(tv_list_join(&mut joined, l2, c", ".as_ptr()), OK);
            assert_eq!(
                std::slice::from_raw_parts(joined.as_ptr().cast::<u8>(), joined.len()),
                b"99, hello, world"
            );

            let mut cat_tv = tv_initial_value();
            assert_eq!(tv_list_concat(l2, l2_copy, &raw mut cat_tv), OK);
            assert_eq!(tv_list_len_inline(cat_tv.vval.v_list()), 6);
            tv_clear(&raw mut cat_tv);

            tv_list_extend(l2, l2_copy, ptr::null_mut());
            assert_eq!(tv_list_len_inline(l2), 6);

            // remove_items range
            let first_item = (*l2).lv_first;
            let second_item = (*first_item).li_next;
            tv_list_remove_items(l2, first_item, second_item);
            assert_eq!(tv_list_len_inline(l2), 4);

            tv_list_unref(l2_copy);
            tv_list_unref(l2);
            crate::globals::emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv_dict_operations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            crate::globals::emsg_skip += 1;

            let d = tv_dict_alloc();
            assert!(!d.is_null());
            (*d).dv_refcount = 1;
            assert_eq!(tv_dict_add_nr(d, c"foo".as_ptr(), 3, 42), OK);
            assert_eq!(tv_dict_add_float(d, c"flt".as_ptr(), 3, 3.5), OK);
            assert_eq!(tv_dict_add_str(d, c"str".as_ptr(), 3, c"bar".as_ptr()), OK);
            assert_eq!(
                tv_dict_add_str_len(d, c"alloc".as_ptr(), 5, c"baz".as_ptr(), -1),
                OK
            );
            assert!(tv_dict_has_key(d, c"foo".as_ptr()));
            assert!(!tv_dict_has_key(d, c"missing".as_ptr()));

            assert_eq!(tv_dict_get_number(d, c"foo".as_ptr()), 42);
            assert_eq!(tv_dict_get_number(d, c"missing".as_ptr()), 0);

            let mut numbuf = [0 as c_char; NUMBUFLEN];
            assert_eq!(
                CStr::from_ptr(tv_dict_get_string(d, c"str".as_ptr())).to_bytes(),
                b"bar"
            );
            let saved = tv_dict_get_cstring(d, c"foo".as_ptr()).unwrap();
            assert_eq!(saved.as_bytes(), b"42");

            assert_eq!(
                CStr::from_ptr(tv_dict_get_string_buf_chk(
                    d,
                    c"missing".as_ptr(),
                    7,
                    numbuf.as_mut_ptr(),
                    c"def".as_ptr()
                ))
                .to_bytes(),
                b"def"
            );

            // Copy, equal, extend, readonly, clear
            let d_copy = tv_dict_copy(None, d, true, 0);
            (*d_copy).dv_refcount = 1;
            assert!(tv_dict_equal(d, d_copy, false));
            assert!(tv_dict_equal(ptr::null_mut(), ptr::null_mut(), false));
            assert!(!tv_dict_equal(d, ptr::null_mut(), false));

            let d2 = tv_dict_alloc();
            (*d2).dv_refcount = 1;
            assert_eq!(tv_dict_add_nr(d2, c"extra".as_ptr(), 5, 7), OK);
            tv_dict_extend(d, d2, c"force".as_ptr());
            assert_eq!(tv_dict_get_number(d, c"extra".as_ptr()), 7);
            assert!(!tv_dict_equal(d, d_copy, false));

            tv_dict_set_keys_readonly(d2);
            let di_extra = tv_dict_find(d2, c"extra".as_ptr(), -1);
            assert_ne!((*di_extra).di_flags & (DI_FLAGS_RO | DI_FLAGS_FIX), 0);

            tv_dict_clear(d_copy);
            assert_eq!(tv_dict_len_inline(d_copy), 0);

            tv_dict_unref(d2);
            tv_dict_unref(d_copy);
            tv_dict_unref(d);
            crate::globals::emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv2bool_and_typval_getters_locks_and_checks() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            crate::globals::emsg_skip += 1;

            let mut tv = tv_initial_value();
            tv.v_type = VarType::VAR_NUMBER;
            tv.vval = crate::eval::typval::TypvalVval::Number(0);
            assert!(!tv2bool(&raw const tv));
            tv.vval = crate::eval::typval::TypvalVval::Number(5);
            assert!(tv2bool(&raw const tv));

            // tv_get_number, tv_get_float, tv_get_string, tv_equal
            assert_eq!(tv_get_number(&raw const tv), 5);
            assert_eq!(tv_get_float(&raw const tv), 5.0);
            let mut buf = [0 as c_char; NUMBUFLEN];
            assert_eq!(
                CStr::from_ptr(tv_get_string_buf(&raw const tv, buf.as_mut_ptr())).to_bytes(),
                b"5"
            );

            let mut tv_flt = typval_T {
                v_type: VarType::VAR_FLOAT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Float(5.0),
            };
            assert!(!tv_equal(&raw mut tv, &raw mut tv_flt, false));
            assert!(tv_equal(&raw mut tv_flt, &raw mut tv_flt, false));
            assert_eq!(tv_get_float(&raw const tv_flt), 5.0);

            let mut tv_str = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::String(tv_alloc_string(c"hello".as_ptr())),
            };
            assert!(tv_check_str_or_nr(&raw const tv_str));
            assert!(tv_check_num(&raw const tv));
            assert!(!tv_check_num(&raw const tv_flt));
            assert!(tv_check_str(&raw const tv_str));

            // tv_list_alloc_ret, tv_dict_alloc_ret, tv_item_lock, tv_islocked, tv_check_lock
            let mut tv_l = tv_initial_value();
            tv_list_alloc_ret(&raw mut tv_l, 0);
            tv_list_append_number(tv_l.vval.v_list(), 1);
            assert!(!tv_islocked(&raw const tv_l));
            tv_item_lock(&raw mut tv_l, 2, true, false);
            assert!(tv_islocked(&raw const tv_l));
            assert!(tv_check_lock(&raw const tv_l, c"test".as_ptr(), 4));
            tv_item_lock(&raw mut tv_l, 2, false, false);
            assert!(!tv_islocked(&raw const tv_l));

            let mut tv_d = tv_initial_value();
            tv_dict_alloc_ret(&raw mut tv_d);
            assert_eq!(tv_d.v_type, VarType::VAR_DICT);

            tv_clear(&raw mut tv_str);
            tv_clear(&raw mut tv_l);
            tv_clear(&raw mut tv_d);
            crate::globals::emsg_skip -= 1;
        }
    }

    #[test]
    fn test_tv_clear_shared_lists_and_dicts() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            // [&l [1], *l, *l]
            let inner = tv_list_alloc(1);
            tv_list_append_number(inner, 1);
            let outer = tv_list_alloc(3);
            (*outer).lv_refcount = 1;
            for _ in 0..3 {
                tv_list_append_list(outer, inner);
            }
            assert_eq!((*inner).lv_refcount, 3);
            let mut tv = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(outer),
            };
            tv_clear(&mut tv);
            assert!(tv.vval.v_list().is_null());

            // [&l [], *l, *l]
            let empty_inner = tv_list_alloc(0);
            let outer2 = tv_list_alloc(3);
            (*outer2).lv_refcount = 1;
            for _ in 0..3 {
                tv_list_append_list(outer2, empty_inner);
            }
            assert_eq!((*empty_inner).lv_refcount, 3);
            let mut tv2 = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(outer2),
            };
            tv_clear(&mut tv2);
            assert!(tv2.vval.v_list().is_null());

            // [&d {}, *d] and [&d {a: 1}, *d]
            for with_item in [false, true] {
                let d_inner = tv_dict_alloc();
                if with_item {
                    assert_eq!(tv_dict_add_nr(d_inner, c"a".as_ptr(), 1, 1), OK);
                }
                let outer_d = tv_list_alloc(2);
                (*outer_d).lv_refcount = 1;
                tv_list_append_dict(outer_d, d_inner);
                tv_list_append_dict(outer_d, d_inner);
                assert_eq!((*d_inner).dv_refcount, 2);
                let mut tv_d = typval_T {
                    v_type: VarType::VAR_LIST,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(outer_d),
                };
                tv_clear(&mut tv_d);
                assert!(tv_d.vval.v_list().is_null());
            }
        }
    }
}
