#![deny(unsafe_code)]

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
use crate::strings::{format_to_buf, strcoll, strtod, tr};
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

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct BlobId(Option<std::rc::Rc<TvBlob>>);

impl PartialEq for BlobId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for BlobId {}

impl std::fmt::Debug for BlobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BlobId")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl BlobId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&TvBlob> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<TvBlob>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<TvBlob> {
        self.0.clone().expect("null BlobId")
    }
}

impl From<std::rc::Rc<TvBlob>> for BlobId {
    #[inline(always)]
    fn from(b: std::rc::Rc<TvBlob>) -> Self {
        Self(Some(b))
    }
}

impl From<Option<std::rc::Rc<TvBlob>>> for BlobId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<TvBlob>>) -> Self {
        Self(opt)
    }
}

impl std::ops::Deref for BlobId {
    type Target = TvBlob;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.opt().expect("null BlobId")
    }
}

pub struct TvBlob {
    pub bv_ga: std::cell::RefCell<Vec<u8>>,
    pub bv_refcount: std::cell::Cell<i32>,
    pub bv_lock: std::cell::Cell<i32>,
}

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct PartialId(Option<std::rc::Rc<TvPartial>>);

impl PartialEq for PartialId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for PartialId {}

impl std::fmt::Debug for PartialId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PartialId")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl PartialId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&TvPartial> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<TvPartial>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<TvPartial> {
        self.0.clone().expect("null PartialId")
    }
}

impl From<std::rc::Rc<TvPartial>> for PartialId {
    #[inline(always)]
    fn from(pt: std::rc::Rc<TvPartial>) -> Self {
        Self(Some(pt))
    }
}

impl From<&std::rc::Rc<TvPartial>> for PartialId {
    #[inline(always)]
    fn from(pt: &std::rc::Rc<TvPartial>) -> Self {
        Self(Some(pt.clone()))
    }
}

impl From<Option<std::rc::Rc<TvPartial>>> for PartialId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<TvPartial>>) -> Self {
        Self(opt)
    }
}

impl std::ops::Deref for PartialId {
    type Target = TvPartial;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.opt().expect("null PartialId")
    }
}

pub struct TvPartial {
    pub pt_refcount: std::cell::Cell<i32>,
    pub pt_copy_id: std::cell::Cell<i32>,
    pub pt_name: std::cell::OnceCell<std::ffi::CString>,
    pub pt_func: std::cell::OnceCell<std::rc::Rc<crate::eval::userfunc::UserFunc>>,
    pub pt_auto: std::cell::Cell<bool>,
    pub pt_argv: std::cell::RefCell<Vec<TypVal>>,
    pub pt_dict: crate::api::private::validate::CloneCell<DictId>,
}

impl TvPartial {
    #[inline]
    pub fn name_cstr(&self) -> Option<&std::ffi::CStr> {
        self.pt_name.get().map(std::ffi::CString::as_c_str)
    }

    #[inline]
    pub fn set_name(&self, name: Option<std::ffi::CString>) {
        if let Some(name) = name {
            let _ = self.pt_name.set(name);
        }
    }

    #[inline]
    pub fn func_opt(&self) -> Option<&crate::eval::userfunc::UserFunc> {
        self.pt_func.get().map(std::rc::Rc::as_ref)
    }

    #[inline]
    pub fn func_rc(&self) -> Option<std::rc::Rc<crate::eval::userfunc::UserFunc>> {
        self.pt_func.get().cloned()
    }

    #[inline]
    pub fn set_func(&self, fp: Option<std::rc::Rc<crate::eval::userfunc::UserFunc>>) {
        if let Some(fp) = fp {
            let _ = self.pt_func.set(fp);
        }
    }
}

impl Default for TvPartial {
    fn default() -> Self {
        Self {
            pt_refcount: std::cell::Cell::new(0),
            pt_copy_id: std::cell::Cell::new(0),
            pt_name: std::cell::OnceCell::new(),
            pt_func: std::cell::OnceCell::new(),
            pt_auto: std::cell::Cell::new(false),
            pt_argv: std::cell::RefCell::new(Vec::new()),
            pt_dict: crate::api::private::validate::CloneCell::new(DictId::NULL),
        }
    }
}

impl Drop for TvPartial {
    fn drop(&mut self) {
        if crate::globals::g_st().exiting.get() {
            return;
        }
        for tv in self.pt_argv.get_mut() {
            tv_clear(tv);
        }
        tv_dict_unref(self.pt_dict.get().opt());
        if let Some(name) = self.pt_name.get() {
            func_unref(Some(name.as_c_str()));
        } else {
            crate::eval::userfunc::func_ptr_unref(self.func_opt());
        }
    }
}

#[derive(Clone, Default)]
pub enum CallbackData {
    #[default]
    None,
    Funcref(&'static std::ffi::CStr),
    Partial(PartialId),
    Lua(LuaRef),
    Expr(&'static std::ffi::CStr),
}

impl CallbackData {
    pub const NONE: Self = Self::None;

    #[inline]
    pub fn funcref(funcref: impl AsRef<std::ffi::CStr>) -> Self {
        Self::Funcref(crate::option_vars::intern_opt_cstr(funcref.as_ref()))
    }

    #[inline]
    pub fn expr(expr: impl AsRef<std::ffi::CStr>) -> Self {
        Self::Expr(crate::option_vars::intern_opt_cstr(expr.as_ref()))
    }

    #[inline]
    pub fn partial(partial: impl Into<PartialId>) -> Self {
        Self::Partial(partial.into())
    }

    #[inline]
    pub const fn lua(luaref: LuaRef) -> Self {
        Self::Lua(luaref)
    }

    #[inline]
    pub const fn as_funcref(&self) -> Option<&'static std::ffi::CStr> {
        match self {
            Self::Funcref(s) | Self::Expr(s) => Some(*s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_partial(&self) -> PartialId {
        match self {
            Self::Partial(p) => p.clone(),
            _ => PartialId::NULL,
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

impl crate::globals::CloneCell<CallbackData> {
    #[inline]
    pub fn as_funcref(&self) -> Option<&'static std::ffi::CStr> {
        self.get().as_funcref()
    }

    #[inline]
    pub fn as_partial(&self) -> PartialId {
        self.get().as_partial()
    }

    #[inline]
    pub fn as_luaref(&self) -> LuaRef {
        self.get().as_luaref()
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct Callback {
    pub data: crate::globals::CloneCell<CallbackData>,
}

impl Callback {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const NONE: Self = Self {
        data: crate::globals::CloneCell::new(CallbackData::NONE),
    };

    #[inline]
    pub const fn new() -> Self {
        Self {
            data: crate::globals::CloneCell::new(CallbackData::NONE),
        }
    }

    #[inline]
    pub fn cb_type(&self) -> i32 {
        match self.data.get() {
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
#[derive(Clone, Default)]
pub enum TypvalVval {
    #[default]
    None,
    Number(i64),
    Bool(i32),
    Special(i32),
    Float(f64),
    String(Option<std::ffi::CString>),
    List(ListId),
    Dict(DictId),
    Blob(BlobId),
    Partial(PartialId),
}

impl TypvalVval {
    pub const NULL_STRING: Self = Self::String(None);

    #[inline]
    pub fn from_cstr(s: &CStr) -> Self {
        Self::String(Some(s.to_owned()))
    }

    #[inline]
    pub fn from_opt_cstring(s: Option<std::ffi::CString>) -> Self {
        Self::String(s)
    }

    #[inline]
    pub fn from_opt_cstr(s: Option<&CStr>) -> Self {
        Self::String(s.map(CStr::to_owned))
    }

    #[inline]
    pub fn from_cstring(s: std::ffi::CString) -> Self {
        Self::String(Some(s))
    }

    #[inline]
    pub const fn v_number(&self) -> i64 {
        match *self {
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
    pub const fn v_bool(&self) -> i32 {
        match *self {
            Self::Bool(b) | Self::Special(b) => b,
            Self::Number(n) => n as i32,
            _ => 0,
        }
    }

    #[inline]
    pub const fn v_special(&self) -> i32 {
        match *self {
            Self::Special(s) | Self::Bool(s) => s,
            Self::Number(n) => n as i32,
            _ => 0,
        }
    }

    #[inline]
    pub const fn v_float(&self) -> f64 {
        match *self {
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
    pub const fn v_string_is_null(&self) -> bool {
        !matches!(self, Self::String(Some(_)))
    }

    #[inline]
    pub fn v_string_cstr(&self) -> Option<&CStr> {
        match self {
            Self::String(Some(s)) => Some(s.as_c_str()),
            _ => None,
        }
    }

    #[inline]
    pub fn v_string_cstr_or_empty(&self) -> &CStr {
        self.v_string_cstr().unwrap_or(c"")
    }

    #[inline]
    pub fn v_string_bytes(&self) -> &[u8] {
        self.v_string_cstr_or_empty().to_bytes()
    }

    #[inline]
    pub fn v_string_opt_bytes(&self) -> Option<&[u8]> {
        self.v_string_cstr().map(CStr::to_bytes)
    }

    #[inline]
    pub fn v_string_is_empty(&self) -> bool {
        self.v_string_bytes().is_empty()
    }

    #[inline]
    pub fn v_string_first_byte(&self) -> u8 {
        self.v_string_bytes().first().copied().unwrap_or(0)
    }

    #[inline]
    pub fn v_string_owned(&self) -> std::ffi::CString {
        self.v_string_cstr_or_empty().to_owned()
    }

    #[inline(always)]
    pub fn v_list(&self) -> ListId {
        match self {
            Self::List(p) => p.clone(),
            _ => ListId::NULL,
        }
    }

    #[inline]
    pub fn v_list_ref(&self) -> &TvList {
        match self {
            Self::List(p) => p.opt().expect("null list"),
            _ => panic!("null list"),
        }
    }

    #[inline]
    pub fn v_list_opt(&self) -> Option<&TvList> {
        match self {
            Self::List(p) => p.opt(),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn v_dict(&self) -> DictId {
        match self {
            Self::Dict(p) => p.clone(),
            _ => DictId::NULL,
        }
    }

    #[inline]
    pub fn v_dict_ref(&self) -> &TvDict {
        match self {
            Self::Dict(p) => p.borrow(),
            _ => panic!("null dict"),
        }
    }

    #[inline]
    pub fn v_dict_opt(&self) -> Option<&TvDict> {
        match self {
            Self::Dict(p) => p.opt(),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn v_blob(&self) -> BlobId {
        match self {
            Self::Blob(p) => p.clone(),
            _ => BlobId::NULL,
        }
    }

    #[inline]
    pub fn v_blob_ref(&self) -> &TvBlob {
        match self {
            Self::Blob(p) => p.opt().expect("null blob"),
            _ => panic!("null blob"),
        }
    }

    #[inline]
    pub fn v_blob_opt(&self) -> Option<&TvBlob> {
        match self {
            Self::Blob(p) => p.opt(),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn v_partial(&self) -> PartialId {
        match self {
            Self::Partial(p) => p.clone(),
            _ => PartialId::NULL,
        }
    }

    #[inline]
    pub fn v_partial_opt(&self) -> Option<&TvPartial> {
        match self {
            Self::Partial(p) => p.opt(),
            _ => None,
        }
    }

    #[inline]
    pub fn v_any_addr(&self) -> usize {
        match self {
            Self::String(Some(s)) => s.as_ptr() as usize,
            Self::List(p) => p.as_ptr() as usize,
            Self::Dict(p) => p.as_ptr() as usize,
            Self::Blob(p) => p.opt().map_or(0, |b| ptr::from_ref(b) as usize),
            Self::Partial(p) => p.opt().map_or(0, |pt| ptr::from_ref(pt) as usize),
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone)]
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

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct ListitemId(Option<std::rc::Rc<Listitem>>);

impl PartialEq for ListitemId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for ListitemId {}

impl std::fmt::Debug for ListitemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ListitemId")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl ListitemId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub fn from_ref(li: &Listitem) -> Self {
        Self::from(li)
    }

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&Listitem> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &Listitem {
        self.opt().expect("null ListitemId")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<Listitem>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<Listitem> {
        self.0.clone().expect("null ListitemId")
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const Listitem {
        self.opt().map_or(ptr::null(), ptr::from_ref)
    }
}

impl From<std::rc::Rc<Listitem>> for ListitemId {
    #[inline(always)]
    fn from(rc: std::rc::Rc<Listitem>) -> Self {
        Self(Some(rc))
    }
}

impl From<&std::rc::Rc<Listitem>> for ListitemId {
    #[inline(always)]
    fn from(rc: &std::rc::Rc<Listitem>) -> Self {
        Self(Some(rc.clone()))
    }
}

impl From<Option<std::rc::Rc<Listitem>>> for ListitemId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<Listitem>>) -> Self {
        Self(opt)
    }
}

impl From<&ListitemId> for ListitemId {
    #[inline(always)]
    fn from(id: &ListitemId) -> Self {
        id.clone()
    }
}

impl From<Option<ListitemId>> for ListitemId {
    #[inline(always)]
    fn from(opt: Option<ListitemId>) -> Self {
        opt.unwrap_or(Self::NULL)
    }
}

impl From<Option<&ListitemId>> for ListitemId {
    #[inline(always)]
    fn from(opt: Option<&ListitemId>) -> Self {
        opt.cloned().unwrap_or(Self::NULL)
    }
}

impl From<&Listitem> for ListitemId {
    #[inline(always)]
    fn from(li: &Listitem) -> Self {
        Self(li.self_weak.upgrade())
    }
}

impl From<&mut Listitem> for ListitemId {
    #[inline(always)]
    fn from(li: &mut Listitem) -> Self {
        Self(li.self_weak.upgrade())
    }
}

impl From<Option<&Listitem>> for ListitemId {
    #[inline(always)]
    fn from(opt: Option<&Listitem>) -> Self {
        Self(opt.and_then(|li| li.self_weak.upgrade()))
    }
}

impl From<Option<&mut Listitem>> for ListitemId {
    #[inline(always)]
    fn from(opt: Option<&mut Listitem>) -> Self {
        Self(opt.and_then(|li| li.self_weak.upgrade()))
    }
}

impl PartialEq<Listitem> for ListitemId {
    #[inline(always)]
    fn eq(&self, other: &Listitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), other))
    }
}

impl PartialEq<&Listitem> for ListitemId {
    #[inline(always)]
    fn eq(&self, other: &&Listitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<&mut Listitem> for ListitemId {
    #[inline(always)]
    fn eq(&self, other: &&mut Listitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<*const Listitem> for ListitemId {
    #[inline(always)]
    fn eq(&self, other: &*const Listitem) -> bool {
        ptr::eq(self.as_ptr(), *other)
    }
}

impl std::ops::Deref for ListitemId {
    type Target = Listitem;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

#[repr(C)]
pub struct Listitem {
    pub self_weak: std::rc::Weak<Self>,
    pub li_next: crate::api::private::validate::CloneCell<ListitemId>,
    pub li_prev: crate::api::private::validate::CloneCell<ListitemId>,
    pub li_tv: std::cell::RefCell<TypVal>,
}

impl Listitem {
    pub fn new_rc() -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            li_next: crate::api::private::validate::CloneCell::new(ListitemId::NULL),
            li_prev: crate::api::private::validate::CloneCell::new(ListitemId::NULL),
            li_tv: std::cell::RefCell::new(tv_initial_value()),
        })
    }

    #[inline]
    pub fn tv(&self) -> std::cell::Ref<'_, TypVal> {
        self.li_tv.borrow()
    }

    #[inline]
    pub fn tv_mut(&self) -> std::cell::RefMut<'_, TypVal> {
        self.li_tv.borrow_mut()
    }

    #[inline]
    pub fn set_tv(&self, tv: TypVal) {
        *self.li_tv.borrow_mut() = tv;
    }

    #[inline]
    pub fn clear_tv(&self) {
        let mut tv = std::mem::replace(
            &mut *self.li_tv.borrow_mut(),
            TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            },
        );
        tv_clear(&mut tv);
        *self.li_tv.borrow_mut() = tv;
    }

    #[inline]
    pub fn next_opt(&self) -> Option<ListitemId> {
        let next = self.li_next.get();
        if next.is_null() { None } else { Some(next) }
    }
}

impl Drop for Listitem {
    fn drop(&mut self) {
        tv_clear(self.li_tv.get_mut());
    }
}

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct ListId(Option<std::rc::Rc<TvList>>);

impl PartialEq for ListId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for ListId {}

impl std::fmt::Debug for ListId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ListId")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl ListId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub fn from_ref(l: &TvList) -> Self {
        Self::from(l)
    }

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&TvList> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &TvList {
        self.opt().expect("null ListId")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<TvList>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<TvList> {
        self.0.clone().expect("null ListId")
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const TvList {
        self.opt().map_or(ptr::null(), ptr::from_ref)
    }
}

impl From<std::rc::Rc<TvList>> for ListId {
    #[inline(always)]
    fn from(rc: std::rc::Rc<TvList>) -> Self {
        Self(Some(rc))
    }
}

impl From<&std::rc::Rc<TvList>> for ListId {
    #[inline(always)]
    fn from(rc: &std::rc::Rc<TvList>) -> Self {
        Self(Some(rc.clone()))
    }
}

impl From<Option<std::rc::Rc<TvList>>> for ListId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<TvList>>) -> Self {
        Self(opt)
    }
}

impl From<&ListId> for ListId {
    #[inline(always)]
    fn from(id: &ListId) -> Self {
        id.clone()
    }
}

impl From<Option<ListId>> for ListId {
    #[inline(always)]
    fn from(opt: Option<ListId>) -> Self {
        opt.unwrap_or(Self::NULL)
    }
}

impl From<&TvList> for ListId {
    #[inline(always)]
    fn from(l: &TvList) -> Self {
        Self(l.self_weak.upgrade())
    }
}

impl From<&mut TvList> for ListId {
    #[inline(always)]
    fn from(l: &mut TvList) -> Self {
        Self(l.self_weak.upgrade())
    }
}

impl From<Option<&TvList>> for ListId {
    #[inline(always)]
    fn from(opt: Option<&TvList>) -> Self {
        Self(opt.and_then(|l| l.self_weak.upgrade()))
    }
}

impl From<Option<&mut TvList>> for ListId {
    #[inline(always)]
    fn from(opt: Option<&mut TvList>) -> Self {
        Self(opt.and_then(|l| l.self_weak.upgrade()))
    }
}

impl PartialEq<TvList> for ListId {
    #[inline(always)]
    fn eq(&self, other: &TvList) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), other))
    }
}

impl PartialEq<&TvList> for ListId {
    #[inline(always)]
    fn eq(&self, other: &&TvList) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<&mut TvList> for ListId {
    #[inline(always)]
    fn eq(&self, other: &&mut TvList) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<ListId> for &TvList {
    #[inline(always)]
    fn eq(&self, other: &ListId) -> bool {
        other == self
    }
}

impl PartialEq<ListId> for &mut TvList {
    #[inline(always)]
    fn eq(&self, other: &ListId) -> bool {
        other == self
    }
}

impl std::ops::Deref for ListId {
    type Target = TvList;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

#[repr(C)]
pub struct TvList {
    pub self_weak: std::rc::Weak<Self>,
    pub lv_first: crate::api::private::validate::CloneCell<ListitemId>,
    pub lv_last: crate::api::private::validate::CloneCell<ListitemId>,
    pub lv_watch: std::cell::RefCell<Vec<std::rc::Rc<Listwatch>>>,
    pub lv_idx_item: crate::api::private::validate::CloneCell<ListitemId>,
    pub lv_copylist: crate::api::private::validate::CloneCell<ListId>,
    pub lv_gc_idx: std::cell::Cell<Option<usize>>,
    pub lv_refcount: std::cell::Cell<i32>,
    pub lv_len: std::cell::Cell<i32>,
    pub lv_idx: std::cell::Cell<i32>,
    pub lv_copy_id: std::cell::Cell<i32>,
    pub lv_lock: std::cell::Cell<i32>,
    pub lua_table_ref: std::cell::Cell<i32>,
}

impl TvList {
    pub fn new_rc(refcount: i32) -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            lv_first: crate::api::private::validate::CloneCell::new(ListitemId::NULL),
            lv_last: crate::api::private::validate::CloneCell::new(ListitemId::NULL),
            lv_watch: std::cell::RefCell::new(Vec::new()),
            lv_idx_item: crate::api::private::validate::CloneCell::new(ListitemId::NULL),
            lv_copylist: crate::api::private::validate::CloneCell::new(ListId::NULL),
            lv_gc_idx: std::cell::Cell::new(None),
            lv_refcount: std::cell::Cell::new(refcount),
            lv_len: std::cell::Cell::new(0),
            lv_idx: std::cell::Cell::new(0),
            lv_copy_id: std::cell::Cell::new(0),
            lv_lock: std::cell::Cell::new(0),
            lua_table_ref: std::cell::Cell::new(LUA_NOREF),
        })
    }

    #[inline]
    pub fn first_opt(&self) -> Option<ListitemId> {
        let first = self.lv_first.get();
        if first.is_null() { None } else { Some(first) }
    }

    #[inline]
    pub fn last_opt(&self) -> Option<ListitemId> {
        let last = self.lv_last.get();
        if last.is_null() { None } else { Some(last) }
    }
}

impl Drop for TvList {
    fn drop(&mut self) {
        if let Some(idx) = self.lv_gc_idx.take() {
            gc_unregister_list(idx);
        }
        let mut r = self.lua_table_ref.get();
        nlua_clear_ref(&mut r);
        self.lua_table_ref.set(r);
        let mut cur = self.lv_first.get();
        self.lv_first.set(ListitemId::NULL);
        self.lv_last.set(ListitemId::NULL);
        self.lv_idx_item.set(ListitemId::NULL);
        while let Some(li) = cur.opt() {
            let next = li.li_next.get();
            li.li_next.set(ListitemId::NULL);
            li.li_prev.set(ListitemId::NULL);
            cur = next;
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct DictitemId(Option<std::rc::Rc<Dictitem>>);

impl PartialEq for DictitemId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for DictitemId {}

impl std::fmt::Debug for DictitemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DictitemId")
            .field(&self.0.as_ref().map(std::rc::Rc::as_ptr))
            .finish()
    }
}

impl DictitemId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub fn from_ref(di: &Dictitem) -> Self {
        Self::from(di)
    }

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&Dictitem> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &Dictitem {
        self.opt().expect("null DictitemId")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<Dictitem>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<Dictitem> {
        self.0.clone().expect("null DictitemId")
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const Dictitem {
        self.opt().map_or(ptr::null(), ptr::from_ref)
    }
}

impl From<std::rc::Rc<Dictitem>> for DictitemId {
    #[inline(always)]
    fn from(rc: std::rc::Rc<Dictitem>) -> Self {
        Self(Some(rc))
    }
}

impl From<&std::rc::Rc<Dictitem>> for DictitemId {
    #[inline(always)]
    fn from(rc: &std::rc::Rc<Dictitem>) -> Self {
        Self(Some(rc.clone()))
    }
}

impl From<Option<std::rc::Rc<Dictitem>>> for DictitemId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<Dictitem>>) -> Self {
        Self(opt)
    }
}

impl From<&DictitemId> for DictitemId {
    #[inline(always)]
    fn from(id: &DictitemId) -> Self {
        id.clone()
    }
}

impl From<Option<DictitemId>> for DictitemId {
    #[inline(always)]
    fn from(opt: Option<DictitemId>) -> Self {
        opt.unwrap_or(Self::NULL)
    }
}

impl From<&Dictitem> for DictitemId {
    #[inline(always)]
    fn from(di: &Dictitem) -> Self {
        Self(di.self_weak.upgrade())
    }
}

impl From<&mut Dictitem> for DictitemId {
    #[inline(always)]
    fn from(di: &mut Dictitem) -> Self {
        Self(di.self_weak.upgrade())
    }
}

impl From<Option<&Dictitem>> for DictitemId {
    #[inline(always)]
    fn from(opt: Option<&Dictitem>) -> Self {
        Self(opt.and_then(|di| di.self_weak.upgrade()))
    }
}

impl From<Option<&mut Dictitem>> for DictitemId {
    #[inline(always)]
    fn from(opt: Option<&mut Dictitem>) -> Self {
        Self(opt.and_then(|di| di.self_weak.upgrade()))
    }
}

impl PartialEq<Dictitem> for DictitemId {
    #[inline(always)]
    fn eq(&self, other: &Dictitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), other))
    }
}

impl PartialEq<&Dictitem> for DictitemId {
    #[inline(always)]
    fn eq(&self, other: &&Dictitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<&mut Dictitem> for DictitemId {
    #[inline(always)]
    fn eq(&self, other: &&mut Dictitem) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(rc.as_ref(), *other))
    }
}

impl PartialEq<*const Dictitem> for DictitemId {
    #[inline(always)]
    fn eq(&self, other: &*const Dictitem) -> bool {
        ptr::eq(self.as_ptr(), *other)
    }
}

impl std::ops::Deref for DictitemId {
    type Target = Dictitem;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

#[repr(C)]
pub struct Dictitem {
    pub self_weak: std::rc::Weak<Self>,
    pub di_tv: std::cell::RefCell<TypVal>,
    pub di_flags: std::cell::Cell<u8>,
    pub di_key: Vec<u8>,
}

impl Dictitem {
    pub fn new_rc(key_vec: Vec<u8>, flags: u8) -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            di_tv: std::cell::RefCell::new(tv_initial_value()),
            di_flags: std::cell::Cell::new(flags),
            di_key: key_vec,
        })
    }

    #[inline]
    pub fn tv(&self) -> std::cell::Ref<'_, TypVal> {
        self.di_tv.borrow()
    }

    #[inline]
    pub fn tv_mut(&self) -> std::cell::RefMut<'_, TypVal> {
        self.di_tv.borrow_mut()
    }

    #[inline]
    pub fn set_tv(&self, tv: TypVal) {
        *self.di_tv.borrow_mut() = tv;
    }

    #[inline]
    pub fn clear_tv(&self) {
        let mut tv = std::mem::replace(
            &mut *self.di_tv.borrow_mut(),
            TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            },
        );
        tv_clear(&mut tv);
        *self.di_tv.borrow_mut() = tv;
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
        if (self.di_flags.get() & DI_FLAGS_ALLOC) != 0 {
            tv_clear(self.di_tv.get_mut());
        }
    }
}

pub type ScopeDictDictItem = Dictitem;

#[repr(transparent)]
#[derive(Clone, Default)]
pub struct DictId(Option<std::rc::Rc<TvDict>>);

impl DictId {
    pub const NULL: Self = Self(None);

    #[inline(always)]
    pub fn from_ref(d: &TvDict) -> Self {
        Self(d.self_weak.upgrade())
    }

    #[inline(always)]
    pub const fn is_null(&self) -> bool {
        self.0.is_none()
    }

    #[inline(always)]
    pub fn opt(&self) -> Option<&TvDict> {
        self.0.as_deref()
    }

    #[inline(always)]
    pub fn borrow(&self) -> &TvDict {
        self.opt().expect("null DictId")
    }

    #[inline(always)]
    pub fn rc(&self) -> Option<std::rc::Rc<TvDict>> {
        self.0.clone()
    }

    #[inline(always)]
    pub fn unwrap_rc(&self) -> std::rc::Rc<TvDict> {
        self.0.clone().expect("null DictId")
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *mut TvDict {
        self.0
            .as_ref()
            .map_or(ptr::null_mut(), |rc| std::rc::Rc::as_ptr(rc).cast_mut())
    }
}

impl std::fmt::Debug for DictId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DictId").field(&self.as_ptr()).finish()
    }
}

impl PartialEq for DictId {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for DictId {}

impl From<std::rc::Rc<TvDict>> for DictId {
    #[inline(always)]
    fn from(rc: std::rc::Rc<TvDict>) -> Self {
        Self(Some(rc))
    }
}

impl From<Option<std::rc::Rc<TvDict>>> for DictId {
    #[inline(always)]
    fn from(opt: Option<std::rc::Rc<TvDict>>) -> Self {
        Self(opt)
    }
}

impl From<&TvDict> for DictId {
    #[inline(always)]
    fn from(d: &TvDict) -> Self {
        Self(d.self_weak.upgrade())
    }
}

impl From<&mut TvDict> for DictId {
    #[inline(always)]
    fn from(d: &mut TvDict) -> Self {
        Self(d.self_weak.upgrade())
    }
}

impl From<Option<&TvDict>> for DictId {
    #[inline(always)]
    fn from(opt: Option<&TvDict>) -> Self {
        opt.map_or(Self::NULL, Self::from)
    }
}

impl From<Option<&mut TvDict>> for DictId {
    #[inline(always)]
    fn from(opt: Option<&mut TvDict>) -> Self {
        opt.map_or(Self::NULL, Self::from)
    }
}

impl PartialEq<TvDict> for DictId {
    #[inline(always)]
    fn eq(&self, other: &TvDict) -> bool {
        self.0
            .as_ref()
            .is_some_and(|rc| ptr::eq(std::rc::Rc::as_ptr(rc), other))
    }
}

impl PartialEq<&TvDict> for DictId {
    #[inline(always)]
    fn eq(&self, other: &&TvDict) -> bool {
        *self == **other
    }
}

impl PartialEq<&mut TvDict> for DictId {
    #[inline(always)]
    fn eq(&self, other: &&mut TvDict) -> bool {
        *self == **other
    }
}

impl PartialEq<*const TvDict> for DictId {
    #[inline(always)]
    fn eq(&self, other: &*const TvDict) -> bool {
        ptr::eq(self.as_ptr().cast_const(), *other)
    }
}

impl PartialEq<*mut TvDict> for DictId {
    #[inline(always)]
    fn eq(&self, other: &*mut TvDict) -> bool {
        ptr::eq(self.as_ptr(), *other)
    }
}

impl std::ops::Deref for DictId {
    type Target = TvDict;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

#[repr(C)]
pub struct TvDict {
    pub self_weak: std::rc::Weak<Self>,
    pub dv_lock: std::cell::Cell<i32>,
    pub dv_scope: std::cell::Cell<i32>,
    pub dv_refcount: std::cell::Cell<i32>,
    pub dv_copy_id: std::cell::Cell<i32>,
    pub dv_hashtab: crate::hashtab::Hashtab,
    pub dv_copydict: crate::api::private::validate::CloneCell<DictId>,
    pub dv_gc_idx: std::cell::Cell<Option<usize>>,
    pub watchers: std::cell::RefCell<Vec<DictWatcher>>,
    pub lua_table_ref: std::cell::Cell<i32>,
}

impl TvDict {
    pub fn new() -> Self {
        Self {
            self_weak: std::rc::Weak::new(),
            dv_lock: std::cell::Cell::new(0),
            dv_scope: std::cell::Cell::new(0),
            dv_refcount: std::cell::Cell::new(0),
            dv_copy_id: std::cell::Cell::new(0),
            dv_hashtab: crate::hashtab::Hashtab::new(),
            dv_copydict: crate::api::private::validate::CloneCell::new(DictId::NULL),
            dv_gc_idx: std::cell::Cell::new(None),
            watchers: std::cell::RefCell::new(Vec::new()),
            lua_table_ref: std::cell::Cell::new(LUA_NOREF),
        }
    }

    pub fn new_rc() -> std::rc::Rc<Self> {
        std::rc::Rc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            dv_lock: std::cell::Cell::new(VAR_UNLOCKED),
            dv_scope: std::cell::Cell::new(VAR_NO_SCOPE),
            dv_refcount: std::cell::Cell::new(0),
            dv_copy_id: std::cell::Cell::new(0),
            dv_hashtab: crate::hashtab::Hashtab::new(),
            dv_copydict: crate::api::private::validate::CloneCell::new(DictId::NULL),
            dv_gc_idx: std::cell::Cell::new(None),
            watchers: std::cell::RefCell::new(Vec::new()),
            lua_table_ref: std::cell::Cell::new(LUA_NOREF),
        })
    }

    #[inline(always)]
    pub fn id(&self) -> DictId {
        DictId(self.self_weak.upgrade())
    }
}

impl Drop for TvDict {
    fn drop(&mut self) {
        if let Some(idx) = self.dv_gc_idx.take() {
            gc_unregister_dict(idx);
        }
        let mut r = self.lua_table_ref.get();
        nlua_clear_ref(&mut r);
        self.lua_table_ref.set(r);
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
    pub lw_item: crate::api::private::validate::CloneCell<ListitemId>,
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
        callback_free(&self.callback);
    }
}

struct Sortinfo {
    item_compare_ic: bool,
    item_compare_lc: bool,
    item_compare_numeric: bool,
    item_compare_numbers: bool,
    item_compare_float: bool,
    item_compare_func: Option<std::ffi::CString>,
    item_compare_partial: PartialId,
    item_compare_selfdict: DictId,
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
            item_compare_partial: PartialId::NULL,
            item_compare_selfdict: DictId::NULL,
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
    item: ListitemId,
    idx: i32,
    key: ListSortKey,
}

impl ListSortItem {
    #[inline]
    fn item(&self) -> &Listitem {
        &self.item
    }
}

struct TypvalState {
    tv_in_free_unref_items: std::cell::Cell<bool>,
    tv_equal_recurse_limit: std::cell::Cell<i32>,
}

impl TypvalState {
    const fn new() -> Self {
        Self {
            tv_in_free_unref_items: std::cell::Cell::new(false),
            tv_equal_recurse_limit: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static TV_STATE: TypvalState = const { TypvalState::new() };
}

#[inline]
pub(crate) fn set_tv_in_free_unref_items(val: bool) {
    TV_STATE.with(|s| s.tv_in_free_unref_items.set(val));
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
pub fn tv_dict_hi2di(hi: &Hashitem) -> &Dictitem {
    hi.hi_di.borrow()
}

#[inline]
pub fn tv_list_set_ret(tv: &mut TypVal, l: Option<&TvList>) {
    tv_list_set_ret_inline(tv, l)
}

#[inline]
pub fn tv_take_string(tv: &mut TypVal) -> Option<std::ffi::CString> {
    match std::mem::replace(&mut tv.vval, crate::eval::typval::TypvalVval::NULL_STRING) {
        crate::eval::typval::TypvalVval::String(s) => s,
        other => {
            tv.vval = other;
            None
        }
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
fn tv_list_ref_inline(l: Option<&TvList>) {
    if let Some(l) = l {
        l.lv_refcount.set(l.lv_refcount.get() + 1);
    }
}

#[inline]
fn tv_list_set_ret_inline(tv: &mut TypVal, l: Option<&TvList>) {
    let l_id = l.map_or(ListId::NULL, |l_ref| {
        l_ref.lv_refcount.set(l_ref.lv_refcount.get() + 1);
        ListId::from(l_ref)
    });
    tv.v_type = VarType::VarList;
    tv.vval = crate::eval::typval::TypvalVval::List(l_id);
}

#[inline]
fn tv_list_locked_inline(l: Option<&TvList>) -> VarLockStatus {
    l.map_or(VAR_FIXED, |l| l.lv_lock.get())
}

#[inline]
pub fn tv_list_ref(l: Option<&TvList>) {
    if let Some(l) = l {
        l.lv_refcount.set(l.lv_refcount.get() + 1);
    }
}

pub struct ListMutIter(ListitemId);

impl Iterator for ListMutIter {
    type Item = ListitemId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.li_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

pub struct ListPtrIter(ListitemId);

impl Iterator for ListPtrIter {
    type Item = ListitemId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        let next = self.0.li_next.get();
        Some(std::mem::replace(&mut self.0, next))
    }
}

#[inline]
pub fn list_iter_mut(start: impl Into<ListitemId>) -> ListMutIter {
    ListMutIter(start.into())
}

#[inline]
pub fn list_iter_ptr(start: impl Into<ListitemId>) -> ListPtrIter {
    ListPtrIter(start.into())
}

#[inline]
pub fn tv_list_set_lock(l: Option<&TvList>, lock: VarLockStatus) {
    tv_list_set_lock_inline(l, lock)
}

#[inline]
pub fn tv_blob_len(b: Option<&TvBlob>) -> i32 {
    b.map_or(0, |b| b.bv_ga.borrow().len() as i32)
}

#[inline]
fn tv_list_set_lock_inline(l: Option<&TvList>, lock: VarLockStatus) {
    if l.is_none() {
        debug_assert_eq!(lock, VAR_FIXED);
        return;
    }
    l.unwrap().lv_lock.set(lock);
}

#[inline]
pub fn tv_list_len(l: Option<&TvList>) -> i32 {
    tv_list_len_inline(l)
}

#[inline]
pub fn tv_dict_len(d: Option<&TvDict>) -> i64 {
    d.map_or(0, |d| d.dv_hashtab.ht_used.get() as i64)
}

#[inline]
pub fn tv_list_first(l: Option<&TvList>) -> Option<ListitemId> {
    tv_list_first_inline(l)
}

#[inline]
pub fn tv_list_last(l: Option<&TvList>) -> Option<ListitemId> {
    tv_list_last_inline(l)
}

pub struct TvListIter {
    cur: ListitemId,
    last: ListitemId,
}

impl Iterator for TvListIter {
    type Item = ListitemId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.cur.is_null() {
            return None;
        }
        let next = if self.cur == self.last {
            ListitemId::NULL
        } else {
            self.cur.li_next.get()
        };
        Some(std::mem::replace(&mut self.cur, next))
    }
}

#[inline]
pub fn tv_list_iter(l: Option<&TvList>) -> TvListIter {
    let cur = l.map_or(ListitemId::NULL, |l| l.lv_first.get());
    let last = l.map_or(ListitemId::NULL, |l| l.lv_last.get());
    TvListIter { cur, last }
}

#[inline]
fn tv_list_len_inline(l: Option<&TvList>) -> i32 {
    l.map_or(0, |l| l.lv_len.get())
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
fn tv_list_first_inline(l: Option<&TvList>) -> Option<ListitemId> {
    l.and_then(|l| l.first_opt())
}

#[inline]
fn tv_list_last_inline(l: Option<&TvList>) -> Option<ListitemId> {
    l.and_then(|l| l.last_opt())
}

#[inline]
fn tv_dict_set_ret_inline(tv: &mut TypVal, d: &TvDict) {
    d.dv_refcount.set(d.dv_refcount.get() + 1);
    tv.v_type = VarType::VarDict;
    tv.vval = crate::eval::typval::TypvalVval::Dict(d.id());
}

#[inline]
fn tv_dict_len_inline(d: Option<&TvDict>) -> isize {
    d.map_or(0, |d| d.dv_hashtab.ht_used.get() as isize)
}

#[inline]
fn tv_dict_is_watched_inline(d: Option<&TvDict>) -> bool {
    d.is_some_and(|d| !d.watchers.borrow().is_empty())
}

#[inline]
fn tv_blob_set_ret_inline(tv: &mut TypVal, b: std::rc::Rc<TvBlob>) {
    b.bv_refcount.set(b.bv_refcount.get() + 1);
    tv.v_type = VarType::VarBlob;
    tv.vval = crate::eval::typval::TypvalVval::Blob(BlobId::from(b));
}

#[inline]
fn tv_blob_len_inline(b: Option<&TvBlob>) -> i32 {
    b.map_or(0, |b| b.bv_ga.borrow().len() as i32)
}

#[inline]
fn tv_blob_get_inline(b: &TvBlob, idx: i32) -> u8 {
    b.bv_ga.borrow()[idx as usize]
}

#[inline]
fn tv_blob_set_inline(blob: &TvBlob, idx: i32, c: u8) {
    blob.bv_ga.borrow_mut()[idx as usize] = c;
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
pub(crate) fn tv_list_item_alloc() -> ListitemId {
    ListitemId::from(Listitem::new_rc())
}

#[inline]
fn free_listitem_slot(item: &Listitem) {
    item.li_next.set(ListitemId::NULL);
    item.li_prev.set(ListitemId::NULL);
    item.clear_tv();
}

/// Remove a list item from a List and free it.
pub fn tv_list_item_remove(l: &TvList, item_in: &Listitem) -> Option<ListitemId> {
    let item = ListitemId::from(item_in);
    let next_item = item.li_next.get();

    tv_list_drop_items(l, &item, &item);
    free_listitem_slot(&item);

    if next_item.is_null() {
        None
    } else {
        Some(next_item)
    }
}

/// Add a watcher to a list.
pub fn tv_list_watch_add(l: &TvList, lw: std::rc::Rc<Listwatch>) {
    l.lv_watch.borrow_mut().push(lw);
}

/// Remove a watcher from a list.
pub fn tv_list_watch_remove(l: &TvList, lwrem: &std::rc::Rc<Listwatch>) {
    let mut watch = l.lv_watch.borrow_mut();
    if let Some(pos) = watch.iter().position(|lw| std::rc::Rc::ptr_eq(lw, lwrem)) {
        watch.remove(pos);
    }
}

/// Advance watchers to the next item.
fn tv_list_watch_fix(l: &TvList, item: &Listitem) {
    for lw in l.lv_watch.borrow().iter() {
        if lw.lw_item.get() == *item {
            lw.lw_item.set(item.li_next.get());
        }
    }
}

/// Allocate an empty list.
pub fn tv_list_alloc(_len: isize) -> ListId {
    let list = ListId::from(TvList::new_rc(0));
    gc_register_list(&list);
    list
}

#[allow(unsafe_code)]
#[unsafe(export_name = "tv_list_alloc")]
pub extern "C" fn tv_list_alloc_ffi(len: isize) -> *const TvList {
    let list = tv_list_alloc(len);
    ptr::from_ref(&*list)
}

/// Allocate an empty static list (`lv_refcount = DO_NOT_FREE_CNT`, not registered in GC).
pub fn tv_list_alloc_static() -> ListId {
    ListId::from(TvList::new_rc(DO_NOT_FREE_CNT))
}

/// Unreference a static list initialized with `DO_NOT_FREE_CNT`.
pub fn unref_var_list(l: &TvList) {
    l.lv_refcount
        .set(l.lv_refcount.get() - (DO_NOT_FREE_CNT - 1));
    tv_list_unref(Some(l));
}

/// Free a heap-allocated `TvList` slot.
pub fn tv_list_free_slot(l: &TvList) {
    if let Some(idx) = l.lv_gc_idx.take() {
        gc_unregister_list(idx);
    }
    l.lv_copylist.set(ListId::NULL);
    let mut r = l.lua_table_ref.get();
    nlua_clear_ref(&mut r);
    l.lua_table_ref.set(r);
}

/// Initialize a static list with 10 items.
pub fn tv_list_init_static10(l: &TvList) {
    tv_list_free_contents(l);
    l.lv_refcount.set(DO_NOT_FREE_CNT);
    for _ in 0..10 {
        let li = tv_list_item_alloc();
        tv_list_append(l, &li);
    }
    tv_list_set_lock_inline(Some(l), VAR_FIXED);
}

/// Free items contained in a list.
pub fn tv_list_free_contents(l: &TvList) {
    let mut item = l.lv_first.get();
    while !item.is_null() {
        let next = item.li_next.get();
        l.lv_first.set(next.clone());
        free_listitem_slot(&item);
        item = next;
    }
    l.lv_len.set(0);
    l.lv_idx_item.set(ListitemId::NULL);
    l.lv_last.set(ListitemId::NULL);
    debug_assert!(l.lv_watch.borrow().is_empty());
}

/// Free a list, including all items it points to.
pub fn tv_list_free(l: Option<&TvList>) {
    let Some(l_ref) = l else {
        return;
    };
    let _hold = ListId::from(l_ref);
    if TV_STATE.with(|s| s.tv_in_free_unref_items.get()) && l_ref.lv_gc_idx.get().is_some() {
        return;
    }

    tv_list_free_contents(l_ref);
    tv_list_free_slot(l_ref);
}

#[allow(unsafe_code)]
#[unsafe(export_name = "tv_list_free")]
pub unsafe extern "C" fn tv_list_free_ffi(l: *const TvList) {
    if !l.is_null() {
        tv_list_free(Some(unsafe { &*l }));
    }
}

/// Unreference a list.
pub fn tv_list_unref(l: Option<&TvList>) {
    if let Some(l_ref) = l {
        l_ref.lv_refcount.set(l_ref.lv_refcount.get() - 1);
        if l_ref.lv_refcount.get() <= 0 {
            tv_list_free(Some(l_ref));
        }
    }
}

/// Remove items "item" to "item2" from list "l".
pub fn tv_list_drop_items(l: &TvList, item_in: &Listitem, item2_in: &Listitem) {
    let item = ListitemId::from_ref(item_in);
    let item2 = ListitemId::from_ref(item2_in);
    let end_next = item2.li_next.get();
    let mut ip = item.clone();
    while ip != end_next {
        l.lv_len.set(l.lv_len.get() - 1);

        tv_list_watch_fix(l, &ip);

        ip = ip.li_next.get();
    }

    let item_prev = item.li_prev.get();
    if end_next.is_null() {
        l.lv_last.set(item_prev.clone());
    } else {
        end_next.li_prev.set(item_prev.clone());
    }
    if item_prev.is_null() {
        l.lv_first.set(end_next);
    } else {
        item_prev.li_next.set(end_next);
    }
    l.lv_idx_item.set(ListitemId::NULL);
}

/// Like tv_list_drop_items, but also frees all removed items.
pub fn tv_list_remove_items(l: &TvList, item_in: &Listitem, item2_in: &Listitem) {
    let item = ListitemId::from(item_in);
    let item2 = ListitemId::from(item2_in);
    tv_list_drop_items(l, &item, &item2);
    let mut li = item;
    loop {
        let nli = li.li_next.get();
        let is_end = li == item2;
        free_listitem_slot(&li);
        if is_end {
            break;
        }
        li = nli;
    }
}

/// Move items "item" to "item2" from list "l" to the end of the list "tgt_l".
pub fn tv_list_move_items(
    l: &TvList,
    item_in: &Listitem,
    item2_in: &Listitem,
    tgt_l: &TvList,
    cnt: i32,
) {
    let item = ListitemId::from_ref(item_in);
    let item2 = ListitemId::from_ref(item2_in);
    tv_list_drop_items(l, &item, &item2);
    item.li_prev.set(tgt_l.lv_last.get());
    item2.li_next.set(ListitemId::NULL);
    if let Some(last_ref) = tgt_l.lv_last.get().opt() {
        last_ref.li_next.set(item);
    } else {
        tgt_l.lv_first.set(item);
    }
    tgt_l.lv_last.set(item2);
    tgt_l.lv_len.set(tgt_l.lv_len.get() + cnt);
}

/// Insert list item.
pub fn tv_list_insert(l: &TvList, ni: &Listitem, item_opt: Option<&Listitem>) {
    let Some(item_ref) = item_opt else {
        tv_list_append(l, ni);
        return;
    };
    let item = ListitemId::from_ref(item_ref);
    let ni_ptr = ListitemId::from(ni);
    ni.li_prev.set(item.li_prev.get());
    ni.li_next.set(item.clone());
    if let Some(prev_ref) = item.li_prev.get().opt() {
        prev_ref.li_next.set(ni_ptr.clone());
        l.lv_idx_item.set(ListitemId::NULL);
    } else {
        l.lv_first.set(ni_ptr.clone());
        l.lv_idx.set(l.lv_idx.get() + 1);
    }

    item.li_prev.set(ni_ptr);
    l.lv_len.set(l.lv_len.get() + 1);
}

/// Insert Vimscript value into a list.
pub fn tv_list_insert_tv(l: &TvList, tv: &mut TypVal, item: Option<&Listitem>) {
    let ni = tv_list_item_alloc();
    tv_copy(&*tv, &mut ni.tv_mut());
    tv_list_insert(l, &ni, item);
}

/// Append item to the end of list.
pub fn tv_list_append(l: &TvList, item: &Listitem) {
    let item_ptr = ListitemId::from(item);
    if let Some(last_ref) = l.lv_last.get().opt() {
        last_ref.li_next.set(item_ptr.clone());
        item.li_prev.set(l.lv_last.get());
        l.lv_last.set(item_ptr);
    } else {
        l.lv_first.set(item_ptr.clone());
        l.lv_last.set(item_ptr);
        item.li_prev.set(ListitemId::NULL);
    }
    l.lv_len.set(l.lv_len.get() + 1);
    item.li_next.set(ListitemId::NULL);
}

/// Append Vimscript value to the end of list.
pub fn tv_list_append_tv(l: &TvList, tv: &TypVal) {
    let li = tv_list_item_alloc();
    tv_copy(tv, &mut li.tv_mut());
    tv_list_append(l, &li);
}

/// Like tv_list_append_tv(), but tv is moved to a list.
pub fn tv_list_append_owned_tv(l: &TvList, tv: TypVal) -> ListitemId {
    let li = tv_list_item_alloc();
    li.set_tv(tv);
    tv_list_append(l, &li);
    li
}

/// Append a list to a list as one item.
pub fn tv_list_append_list(l: &TvList, itemlist: Option<&TvList>) {
    let itemlist_id = itemlist.map_or(ListId::NULL, |il| {
        il.lv_refcount.set(il.lv_refcount.get() + 1);
        ListId::from(il)
    });
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarList,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::List(itemlist_id),
        },
    );
}

/// Append a dictionary to a list.
pub fn tv_list_append_dict(l: &TvList, dict: Option<&TvDict>) {
    let dict_id = dict.map_or(DictId::NULL, |d| {
        d.dv_refcount.set(d.dv_refcount.get() + 1);
        d.id()
    });
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarDict,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Dict(dict_id),
        },
    );
}

/// Make a copy of "str" and append it as an item to list "l".
#[inline]
pub fn tv_list_append_bytes(l: &TvList, bytes: &[u8]) {
    tv_list_append_owned_tv(
        l,
        TypVal {
            v_type: VarType::VarString,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::from_cstring(crate::memory::bytes_to_cstring(bytes)),
        },
    );
}

pub fn tv_list_append_string(l: &TvList, str_: Option<&CStr>, len: isize) {
    let v_string = match str_ {
        None => None,
        Some(s) if len >= 0 => {
            let bytes = s.to_bytes();
            Some(crate::memory::bytes_to_cstring(
                &bytes[..(len as usize).min(bytes.len())],
            ))
        }
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
pub fn tv_list_append_number(l: &TvList, n: Varnumber) {
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
    orig: Option<&TvList>,
    deep: bool,
    copy_id: i32,
) -> Option<ListId> {
    let orig = orig?;

    let copy = tv_list_alloc(tv_list_len_inline(Some(orig)) as isize);
    tv_list_ref_inline(Some(&copy));
    if copy_id != 0 {
        orig.lv_copy_id.set(copy_id);
        orig.lv_copylist.set(copy.clone());
    }
    for item in crate::eval::typval::list_iter_mut(orig.first_opt()) {
        if g_st().got_int.get() {
            break;
        }
        let ni = tv_list_item_alloc();
        if deep {
            let mut src_tv = item.tv().clone();
            let mut dst_tv = tv_initial_value();
            if !var_item_copy(conv, &mut src_tv, &mut dst_tv, deep, copy_id) {
                free_listitem_slot(&ni);
                tv_list_unref(Some(&copy));
                return None;
            }
            ni.set_tv(dst_tv);
        } else {
            tv_copy(&item.tv(), &mut ni.tv_mut());
        }
        tv_list_append(&copy, &ni);
    }

    Some(copy)
}

/// Get the list item in "l" with index "n1".
pub fn tv_list_check_range_index_one(
    l: Option<&TvList>,
    n1: &mut i32,
    quiet: bool,
) -> Option<ListitemId> {
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
    l: Option<&TvList>,
    n1: &mut i32,
    li1: Option<&Listitem>,
    n2: &mut i32,
    quiet: bool,
) -> bool {
    if *n2 < 0 {
        let Some(ni) = tv_list_find(l, *n2) else {
            if !quiet {
                semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(*n2));
            }
            return false;
        };
        *n2 = tv_list_idx_of_item(l, Some(&ni));
    }

    if *n1 < 0 {
        *n1 = tv_list_idx_of_item(l, li1);
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
    dest: &TvList,
    src: Option<&TvList>,
    idx1_arg: i32,
    idx2: i32,
    empty_idx2: bool,
    op_opt: Option<&CStr>,
    varname: &CStr,
) -> bool {
    let mut idx1 = idx1_arg;
    let first_li = ListitemId::from(tv_list_find_index(Some(dest), &mut idx1));
    let mut src_li: ListitemId;

    let mut idx = idx1;
    let mut dest_li = first_li.clone();
    src_li = ListitemId::from(tv_list_first_inline(src));
    while !src_li.is_null() && !dest_li.is_null() {
        if value_check_lock(dest_li.tv().v_lock, Some(varname), TV_CSTRING) {
            return false;
        }
        src_li = src_li.li_next.get();
        if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
            break;
        }
        dest_li = dest_li.li_next.get();
        idx += 1;
    }

    idx = idx1;
    dest_li = first_li;
    src_li = ListitemId::from(tv_list_first_inline(src));
    while !src_li.is_null() {
        debug_assert!(!dest_li.is_null());
        let mut src_tv = tv_initial_value();
        tv_copy(&src_li.tv(), &mut src_tv);
        if let Some(op) = op_opt
            && op.to_bytes().first() != Some(&b'=')
        {
            eexe_mod_op(&mut dest_li.tv_mut(), &src_tv, op);
            tv_clear(&mut src_tv);
        } else {
            dest_li.clear_tv();
            dest_li.set_tv(src_tv);
        }
        src_li = src_li.li_next.get();
        if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
            break;
        }
        if dest_li.li_next.get().is_null() {
            tv_list_append_number(dest, 0);
            dest_li = ListitemId::from(tv_list_last_inline(Some(dest)));
        } else {
            dest_li = dest_li.li_next.get();
        }
        idx += 1;
    }
    if !src_li.is_null() {
        emsg(tr(c"E710: List value has more items than target"));
        return false;
    }
    if if empty_idx2 {
        !dest_li.is_null() && !dest_li.li_next.get().is_null()
    } else {
        idx != idx2
    } {
        emsg(tr(c"E711: List value has not enough items"));
        return false;
    }
    true
}

/// Flatten up to "maxitems" in "list", starting at "first" to depth "maxdepth".
pub fn tv_list_flatten(list: &TvList, first_opt: Option<&Listitem>, maxitems: i64, maxdepth: i64) {
    if maxdepth == 0 {
        return;
    }

    let first = ListitemId::from(first_opt);
    let mut item = if first.is_null() {
        list.lv_first.get()
    } else {
        first
    };
    let mut done: i64 = 0;

    while !item.is_null() && done < maxitems {
        let next = item.li_next.get();

        fast_breakcheck();
        if g_st().got_int.get() {
            return;
        }
        if item.tv().v_type == VarType::VarList {
            let itemlist = item.tv().vval.v_list();
            let item_prev = item.li_prev.get();

            tv_list_drop_items(list, &item, &item);
            tv_list_extend(list, itemlist.opt(), next.opt());

            if maxdepth > 0 {
                let sub_first = item_prev
                    .opt()
                    .map_or_else(|| list.lv_first.get(), |prev| prev.li_next.get());
                let sub_len = itemlist.opt().map_or(0, |l| i64::from(l.lv_len.get()));
                tv_list_flatten(list, sub_first.opt(), sub_len, maxdepth - 1);
            }

            free_listitem_slot(&item);
        }

        done += 1;
        item = next;
    }
}

/// "items(blob)" function.
fn tv_blob2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let blob = argvars[0].vval.v_blob_opt();

    tv_list_alloc_ret(&mut *rettv, tv_blob_len_inline(blob) as isize);
    let Some(blob) = blob else {
        return;
    };

    for i in 0..tv_blob_len_inline(Some(blob)) {
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&l2));
        tv_list_append_number(&l2, i64::from(i));
        tv_list_append_number(&l2, i64::from(tv_blob_get_inline(blob, i)));
    }
}

/// "items(dict)" function.
fn tv_dict2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    tv_dict2list(argvars, rettv, DictListType::Items);
}

/// "items(list)" function.
fn tv_list2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    let l = argvars[0].vval.v_list_opt();

    tv_list_alloc_ret(&mut *rettv, tv_list_len_inline(l) as isize);
    let Some(l) = l else {
        return;
    };

    for (idx, li) in (0 as Varnumber..).zip(crate::eval::typval::list_iter_mut(l.first_opt())) {
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&l2));
        tv_list_append_number(&l2, idx);
        tv_list_append_tv(&l2, &li.tv());
    }
}

/// "items(string)" function.
fn tv_string2items(argvars: &mut [TypVal], rettv: &mut TypVal) {
    tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_MAY_KNOW_CONST);
    let Some(mut p) = argvars[0].vval.v_string_cstr() else {
        return;
    };

    let mut idx: Varnumber = 0;
    while !p.is_empty() {
        let len = utfc_ptr2len(p.to_bytes_with_nul());
        if len == 0 {
            break;
        }
        let l2 = tv_list_alloc(2);
        tv_list_append_list(rettv.vval.v_list_ref(), Some(&l2));
        tv_list_append_number(&l2, idx);
        tv_list_append_string(&l2, Some(p), len as isize);
        p = crate::memory::CStrSubslice::advance(p, len as usize);
        idx += 1;
    }
}

/// Extend first list with the second.
pub fn tv_list_extend(l1: &TvList, l2: Option<&TvList>, bef_opt: Option<&Listitem>) {
    let mut todo = tv_list_len_inline(l2);
    if todo == 0 {
        return;
    }

    let bef = ListitemId::from(bef_opt);
    let befbef = bef.opt().map_or(ListitemId::NULL, |b| b.li_prev.get());
    let saved_next = befbef.opt().map_or(ListitemId::NULL, |b| b.li_next.get());
    let mut item = ListitemId::from(tv_list_first_inline(l2));
    while let Some(item_ref) = item.opt()
        && todo > 0
    {
        todo -= 1;
        let mut item_tv = item_ref.tv().clone();
        tv_list_insert_tv(l1, &mut item_tv, bef.opt());
        item = if item == befbef {
            saved_next.clone()
        } else {
            item_ref.li_next.get()
        };
    }
}

/// Concatenate lists into a new list.
pub fn tv_list_concat(l1: Option<&TvList>, l2: Option<&TvList>, tv: &mut TypVal) -> bool {
    tv.v_type = VarType::VarList;
    tv.v_lock = VAR_UNLOCKED;
    let l = if l1.is_none() && l2.is_none() {
        None
    } else if l1.is_none() {
        tv_list_copy(None, l2, false, 0)
    } else {
        let copy = tv_list_copy(None, l1, false, 0);
        if let Some(ref copy_ref) = copy
            && l2.is_some()
        {
            tv_list_extend(copy_ref, l2, None);
        }
        copy
    };
    if l.is_none() && !(l1.is_none() && l2.is_none()) {
        return false;
    }

    tv.vval = crate::eval::typval::TypvalVval::List(ListId::from(l));
    true
}

fn tv_list_slice(ol: Option<&TvList>, mut n1: Varnumber, n2: Varnumber) -> ListId {
    let l = tv_list_alloc((n2 - n1 + 1) as isize);
    let mut item = tv_list_find_opt(ol, n1 as i32);
    while n1 <= n2 {
        let cur = item.unwrap();
        tv_list_append_tv(&l, &cur.tv());
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
    let len = i64::from(tv_list_len_inline(rettv.vval.v_list_opt()));
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
        tv_list_set_ret_inline(rettv, Some(&l));
    } else {
        let found = tv_list_find_opt(rettv.vval.v_list_opt(), n1 as i32).unwrap();
        let mut var1 = tv_initial_value();
        tv_copy(&found.tv(), &mut var1);
        tv_clear(&mut *rettv);
        *rettv = var1;
    }
    true
}

fn list_join_inner(
    gap: &mut Vec<u8>,
    l: &TvList,
    sep: &CStr,
    join_vec: &mut Vec<std::ffi::CString>,
) -> bool {
    let mut sumlen: usize = 0;
    let mut first = true;

    for item in crate::eval::typval::list_iter_mut(l.first_opt()) {
        if g_st().got_int.get() {
            break;
        }
        let mut size: usize = 0;
        let mut item_tv = item.tv().clone();
        let s = encode_tv2echo(&mut item_tv, Some(&mut size));

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
        if g_st().got_int.get() {
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
pub fn tv_list_join(gap: &mut Vec<u8>, l: Option<&TvList>, sep: &CStr) -> bool {
    let len = tv_list_len_inline(l);
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
    let sep_cow;
    let sep = if argvars[1].v_type == VarType::VarUnknown {
        Some(c" ")
    } else {
        sep_cow = tv_get_string_chk(&argvars[1]);
        sep_cow.as_deref()
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
        let n = tv_get_number(&li.tv());
        let buflen = utf_char2bytes(n as i32, &mut buf) as usize;
        buf[buflen] = 0;
        out.extend_from_slice(&buf[..buflen]);
    }

    tv_set_string_len(rettv, Some(&out));
}

/// "remove({list})" function.
pub fn tv_list_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    let l = argvars[0].vval.v_list_opt();
    if value_check_lock(tv_list_locked_inline(l), Some(arg_errmsg), TV_TRANSLATE) {
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
    let Some(item) = tv_list_find_opt(Some(l), idx as i32) else {
        semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), idx);
        return;
    };
    if argvars[2].v_type == VarType::VarUnknown {
        tv_list_drop_items(l, &item, &item);
        *rettv = std::mem::replace(&mut *item.tv_mut(), tv_initial_value());
        free_listitem_slot(&item);
    } else {
        let end = tv_get_number_chk(&argvars[2], Some(&mut error));
        if error {
            return;
        }
        let Some(item2) = tv_list_find_opt(Some(l), end as i32) else {
            semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), end);
            return;
        };
        let mut cnt: i32 = 0;
        let mut found_end = false;
        for li in crate::eval::typval::list_iter_ptr(item.clone()) {
            cnt += 1;
            if item2 == li {
                found_end = true;
                break;
            }
        }
        if !found_end {
            emsg(tr(E_INVRANGE));
        } else {
            let tgt = tv_list_alloc_ret(rettv, cnt as isize);
            tv_list_move_items(l, &item, &item2, &tgt, cnt);
        }
    }
}

/// Compare functions for f_sort() and f_uniq().
fn item_compare(si1: &ListSortItem, si2: &ListSortItem, keep_zero: bool, info: &Sortinfo) -> i32 {
    let mut tv1_val = si1.item().tv().clone();
    let mut tv2_val = si2.item().tv().clone();
    let tv1 = &mut tv1_val;
    let tv2 = &mut tv2_val;
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

    let s1;
    let s2;
    let p1: &CStr = if tv1.v_type == VarType::VarString {
        if tv2.v_type != VarType::VarString || info.item_compare_numeric {
            c"'"
        } else {
            tv1.vval.v_string_cstr_or_empty()
        }
    } else {
        s1 = encode_tv2string(tv1, None);
        s1.as_c_str()
    };
    let p2: &CStr = if tv2.v_type == VarType::VarString {
        if tv1.v_type != VarType::VarString || info.item_compare_numeric {
            c"'"
        } else {
            tv2.vval.v_string_cstr_or_empty()
        }
    } else {
        s2 = encode_tv2string(tv2, None);
        s2.as_c_str()
    };
    if !info.item_compare_numeric {
        if info.item_compare_lc {
            res = strcoll(p1, p2);
        } else if info.item_compare_ic {
            res = p1
                .to_bytes()
                .iter()
                .map(u8::to_ascii_lowercase)
                .cmp(p2.to_bytes().iter().map(u8::to_ascii_lowercase)) as i32;
        } else {
            res = p1.to_bytes().cmp(p2.to_bytes()) as i32;
        }
    } else {
        let (n1, _) = strtod(p1);
        let (n2, _) = strtod(p2);
        res = if n1 == n2 {
            0
        } else if n1 > n2 {
            1
        } else {
            -1
        };
    }

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

    let partial = info.item_compare_partial.clone();
    let func_name = if partial.is_null() {
        info.item_compare_func.as_deref().unwrap_or(c"")
    } else {
        partial_name(partial.opt())
    };

    let mut argv: [TypVal; 3] = [const { tv_initial_value() }; 3];
    tv_copy(&si1.item().tv(), &mut argv[0]);
    tv_copy(&si2.item().tv(), &mut argv[1]);

    let mut rettv = tv_initial_value();
    let mut funcexe: Funcexe = Funcexe::new();
    funcexe.fe_evaluate = true;
    funcexe.fe_partial = partial.clone();
    funcexe.fe_selfdict = info.item_compare_selfdict.clone();
    let ok = call_func(func_name, -1, &mut rettv, 2, &mut argv, &mut funcexe);
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
            let n = tv_get_number(&si.item().tv());
            si.key = ListSortKey::Int(n);
        }
    } else if info.item_compare_float {
        for si in ptrs.iter_mut() {
            let f = tv_get_float(&si.item().tv());
            si.key = ListSortKey::Float(f);
        }
    } else {
        for si in ptrs.iter_mut() {
            let mut tv = si.item().tv().clone();
            if tv.v_type == VarType::VarString {
                si.key = ListSortKey::Float(0.0);
            } else {
                let p = encode_tv2string(&mut tv, None);
                let (f, _) = strtod(&p);
                si.key = ListSortKey::Float(f);
            }
        }
    }
    info.item_compare_keys_ready = true;
}

fn do_sort(l: &TvList, info: &mut Sortinfo) {
    let len = tv_list_len_inline(Some(l));
    let mut ptrs = Vec::with_capacity(len.max(0) as usize);

    for (i, li) in (0_i32..).zip(crate::eval::typval::list_iter_ptr(l.first_opt())) {
        ptrs.push(ListSortItem {
            item: li,
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
        l.lv_first.set(ListitemId::NULL);
        l.lv_last.set(ListitemId::NULL);
        l.lv_idx_item.set(ListitemId::NULL);
        l.lv_len.set(0);
        for si in &ptrs {
            tv_list_append(l, si.item());
        }
    }
    if info.item_compare_func_err {
        emsg(tr(c"E702: Sort compare function failed"));
    }
}

fn do_uniq(l: Option<&TvList>, info: &mut Sortinfo) {
    info.item_compare_func_err = false;
    info.item_compare_keys_ready = false;
    let use_custom = info.item_compare_func.is_some() || !info.item_compare_partial.is_null();

    let mut li = l
        .and_then(|l| l.first_opt())
        .map_or(ListitemId::NULL, |first| first.li_next.get());
    while let Some(cur_li) = li.clone().opt() {
        let prev_li = cur_li.li_prev.get();
        let prev_si = ListSortItem {
            item: prev_li,
            idx: 0,
            key: ListSortKey::Int(0),
        };
        let cur_si = ListSortItem {
            item: li.clone(),
            idx: 0,
            key: ListSortKey::Int(0),
        };
        let cmp = if !use_custom {
            item_compare(&prev_si, &cur_si, true, info)
        } else {
            item_compare2(&prev_si, &cur_si, true, info)
        };
        if cmp == 0 {
            li = ListitemId::from(tv_list_item_remove(l.unwrap(), &li));
        } else {
            li = cur_li.li_next.get();
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
    info.item_compare_partial = PartialId::NULL;
    info.item_compare_selfdict = DictId::NULL;
    info.item_compare_keys_ready = false;

    if argvars[1].v_type == VarType::VarUnknown {
        return true;
    }

    if argvars[1].v_type == VarType::VarFunc {
        if let Some(s) = argvars[1].vval.v_string_cstr() {
            info.item_compare_func = Some(s.to_owned());
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
            info.item_compare_func = Some(s.into_owned());
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
        semsg!(tr(E_LISTARG), if sort { c"sort()" } else { c"uniq()" });
        return;
    }

    let mut info = Sortinfo::new();

    let arg_errmsg = if sort {
        c"sort() argument"
    } else {
        c"uniq() argument"
    };
    let l = argvars[0].vval.v_list();
    if !value_check_lock(
        tv_list_locked_inline(l.opt()),
        Some(arg_errmsg),
        TV_TRANSLATE,
    ) {
        tv_list_set_ret_inline(rettv, l.opt());

        let len = tv_list_len_inline(l.opt());
        if len > 1 && parse_sort_uniq_args(argvars, &mut info) {
            if sort {
                do_sort(l.borrow(), &mut info);
            } else {
                do_uniq(l.opt(), &mut info);
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
        if !tv_equal(&i1.tv(), &i2.tv(), ic) {
            return false;
        }
        item1 = i1.next_opt();
        item2 = i2.next_opt();
    }
    debug_assert!(item1.is_none() && item2.is_none());
    true
}

/// Reverse list in-place.
pub fn tv_list_reverse(l: &TvList) {
    if tv_list_len_inline(Some(l)) <= 1 {
        return;
    }
    let old_first = l.lv_first.get();
    l.lv_first.set(l.lv_last.get());
    l.lv_last.set(old_first);
    let mut li = l.first_opt();
    while let Some(cur) = li {
        let next = cur.li_next.get();
        let prev = cur.li_prev.get();
        cur.li_next.set(prev);
        cur.li_prev.set(next);
        li = cur.next_opt();
    }
    l.lv_idx.set(l.lv_len.get() - l.lv_idx.get() - 1);
}

/// Locate item with a given index in a list and return it.
pub fn tv_list_find(l: Option<&TvList>, mut n: i32) -> Option<ListitemId> {
    let l = l?;

    n = tv_list_uidx_inline(Some(l), n);
    if n == -1 {
        return None;
    }

    let mut idx: i32;
    let mut item: ListitemId;

    if !l.lv_idx_item.get().is_null() {
        if n < l.lv_idx.get() / 2 {
            item = l.lv_first.get();
            idx = 0;
        } else if n > (l.lv_idx.get() + l.lv_len.get()) / 2 {
            item = l.lv_last.get();
            idx = l.lv_len.get() - 1;
        } else {
            item = l.lv_idx_item.get();
            idx = l.lv_idx.get();
        }
    } else if n < l.lv_len.get() / 2 {
        item = l.lv_first.get();
        idx = 0;
    } else {
        item = l.lv_last.get();
        idx = l.lv_len.get() - 1;
    }

    while n > idx {
        item = item.li_next.get();
        idx += 1;
    }
    while n < idx {
        item = item.li_prev.get();
        idx -= 1;
    }

    debug_assert_eq!(idx, n);
    l.lv_idx.set(idx);
    l.lv_idx_item.set(item.clone());

    if item.is_null() { None } else { Some(item) }
}

/// Locate item with a given index in a list and return it if present.
pub fn tv_list_find_opt(l: Option<&TvList>, n: i32) -> Option<ListitemId> {
    tv_list_find(l, n)
}

/// Get list item l[n] as a number.
pub fn tv_list_find_nr(l: Option<&TvList>, n: i32, mut ret_error: Option<&mut bool>) -> Varnumber {
    let Some(li) = tv_list_find_opt(l, n) else {
        if let Some(ref mut p) = ret_error {
            **p = true;
        }
        return -1;
    };
    tv_get_number_chk(
        &li.tv(),
        (ret_error.is_some()).then(|| &mut *ret_error.unwrap()),
    )
}

/// Get list item l[n] as a string.
pub fn tv_list_find_str(
    l: Option<&TvList>,
    n: i32,
) -> Option<std::borrow::Cow<'static, std::ffi::CStr>> {
    let Some(li) = tv_list_find_opt(l, n) else {
        semsg!(tr(E_LIST_INDEX_OUT_OF_RANGE_NR), i64::from(n));
        return None;
    };
    let s = tv_get_string(&li.tv()).into_owned();
    Some(std::borrow::Cow::Owned(s))
}

fn tv_list_find_index(l: Option<&TvList>, idx: &mut i32) -> Option<ListitemId> {
    let mut li = tv_list_find(l, *idx);
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
    let item_id = ListitemId::from(item_ref);
    for (idx, li) in (0_i32..).zip(crate::eval::typval::list_iter_ptr(l.first_opt())) {
        if li == item_id {
            return idx;
        }
    }
    -1
}

/// Add watcher to a dictionary.
pub fn tv_dict_watcher_add(dict: Option<&TvDict>, key_pattern: &[u8], callback: Callback) {
    let Some(dict) = dict else {
        return;
    };
    let key_pattern_len = key_pattern.len();
    dict.watchers.borrow_mut().push(DictWatcher {
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
pub fn callback_free(callback: &Callback) {
    match callback.cb_type() {
        K_CALLBACK_FUNCREF => {
            func_unref(callback.data.as_funcref());
        }
        K_CALLBACK_PARTIAL => {
            partial_unref(callback.data.as_partial().opt());
        }
        K_CALLBACK_LUA => {
            let mut luaref = callback.data.as_luaref();
            nlua_clear_ref(&mut luaref);
        }
        _ => {}
    }
    callback.data.set(CallbackData::NONE);
}

/// Copy a callback into a TypVal.
pub fn callback_put(cb: &Callback, tv: &mut TypVal) {
    match cb.cb_type() {
        K_CALLBACK_PARTIAL => {
            let pt = cb.data.as_partial();
            pt.pt_refcount.set(pt.pt_refcount.get() + 1);
            tv.v_type = VarType::VarPartial;
            tv.vval = crate::eval::typval::TypvalVval::Partial(pt);
        }
        K_CALLBACK_FUNCREF => {
            let funcref = cb.data.as_funcref();
            tv_set_string(&mut *tv, funcref);
            tv.v_type = VarType::VarFunc;
            func_ref(tv.vval.v_string_cstr());
        }
        _ => {
            tv.v_type = VarType::VarSpecial;
            tv.vval = crate::eval::typval::TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST);
        }
    }
}

/// Copy callback from "src" to "dest", incrementing the refcounts.
pub fn callback_copy(dest: &Callback, src: &Callback) {
    match src.cb_type() {
        K_CALLBACK_PARTIAL => {
            let pt = src.data.as_partial();
            pt.pt_refcount.set(pt.pt_refcount.get() + 1);
            dest.data.set(CallbackData::partial(pt));
        }
        K_CALLBACK_FUNCREF => {
            let funcref = src.data.as_funcref();
            dest.data.set(CallbackData::funcref(funcref.unwrap_or(c"")));
            func_ref(funcref);
        }
        K_CALLBACK_LUA => {
            let new_ref = api_new_luaref(src.data.as_luaref());
            dest.data.set(CallbackData::lua(new_ref));
        }
        K_CALLBACK_EXPR => {
            dest.data
                .set(CallbackData::expr(src.data.as_funcref().unwrap_or(c"")));
        }
        _ => {
            dest.data.set(CallbackData::NONE);
        }
    }
}

/// Generate a string description of a callback.
pub fn callback_to_string(cb: &Callback) -> std::ffi::CString {
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
            let s = pt.name_cstr().map_or(&[][..], CStr::to_bytes);
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
    dict: Option<&TvDict>,
    key_pattern: &[u8],
    callback: &Callback,
) -> bool {
    let Some(dict) = dict else {
        return false;
    };
    let key_pattern_len = key_pattern.len();

    let mut matched_idx: Option<usize> = None;
    let mut queue_is_busy = false;
    for (idx, w) in dict.watchers.borrow().iter().enumerate() {
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
        dict.watchers.borrow_mut()[idx].needs_free = true;
    } else {
        dict.watchers.borrow_mut().remove(idx);
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
    dict_in: &TvDict,
    key: &CStr,
    newtv: Option<&TypVal>,
    oldtv: Option<&TypVal>,
) {
    let dict = DictId::from(dict_in);
    let mut argv: [TypVal; 3] = [const { tv_initial_value() }; 3];

    argv[0].v_type = VarType::VarDict;
    argv[0].v_lock = VAR_UNLOCKED;
    argv[0].vval = crate::eval::typval::TypvalVval::Dict(dict.clone());
    argv[1].v_lock = VAR_UNLOCKED;
    tv_set_string(&mut argv[1], Some(key));
    argv[2].v_type = VarType::VarDict;
    argv[2].v_lock = VAR_UNLOCKED;
    let changes_dict = tv_dict_alloc();
    changes_dict
        .dv_refcount
        .set(changes_dict.dv_refcount.get() + 1);
    argv[2].vval = crate::eval::typval::TypvalVval::Dict(changes_dict.clone());

    if let Some(newtv) = newtv {
        let v = tv_dict_item_alloc_len(b"new");
        tv_copy(newtv, &mut v.tv_mut());
        tv_dict_add(&changes_dict, &v);
    }

    if let Some(oldtv) = oldtv
        && oldtv.v_type != VarType::VarUnknown
    {
        let v = tv_dict_item_alloc_len(b"old");
        tv_copy(oldtv, &mut v.tv_mut());
        tv_dict_add(&changes_dict, &v);
    }

    let mut any_needs_free = false;
    dict.dv_refcount.set(dict.dv_refcount.get() + 1);
    let key_bytes = key.to_bytes();
    let len = dict.watchers.borrow().len();
    for i in 0..len {
        let cb_opt = {
            let mut watchers = dict.watchers.borrow_mut();
            let w = &mut watchers[i];
            if !w.busy && tv_dict_watcher_matches(w, key_bytes) {
                w.busy = true;
                Some(w.callback.clone())
            } else {
                None
            }
        };
        if let Some(cb) = cb_opt {
            let mut rettv = tv_initial_value();
            callback_call(&cb, 3, &mut argv, &mut rettv);
            {
                let mut watchers = dict.watchers.borrow_mut();
                let w = &mut watchers[i];
                w.callback = cb;
                w.busy = false;
                if w.needs_free {
                    any_needs_free = true;
                }
            }
            tv_clear(&mut rettv);
        }
    }
    if any_needs_free {
        dict.watchers
            .borrow_mut()
            .retain(|watcher| !watcher.needs_free);
    }
    tv_dict_unref(dict.opt());

    for item in argv.iter_mut().skip(1) {
        tv_clear(item);
    }
}

fn tv_dict_item_alloc_slot(key_vec: Vec<u8>, flags: u8) -> DictitemId {
    DictitemId::from(Dictitem::new_rc(key_vec, flags))
}

/// Allocate a dictionary item.
pub fn tv_dict_item_alloc_len(key: &[u8]) -> DictitemId {
    let mut key_vec = Vec::with_capacity(key.len() + 1);
    key_vec.extend_from_slice(key);
    key_vec.push(0);
    tv_dict_item_alloc_slot(key_vec, DI_FLAGS_ALLOC)
}

/// Allocate a static dictionary item (without `DI_FLAGS_ALLOC`).
pub fn tv_dict_item_alloc_static() -> DictitemId {
    tv_dict_item_alloc_slot(Vec::new(), 0)
}

/// Allocate a static dictionary item with key (without `DI_FLAGS_ALLOC`).
pub fn tv_dict_item_alloc_static_key(key_vec: Vec<u8>) -> DictitemId {
    tv_dict_item_alloc_slot(key_vec, 0)
}

/// Allocate a dictionary item.
pub fn tv_dict_item_alloc(key: &CStr) -> DictitemId {
    tv_dict_item_alloc_len(key.to_bytes())
}

/// Free a dictionary item slot.
pub fn tv_dict_item_free_slot(item_in: &Dictitem) {
    *item_in.tv_mut() = TypVal::new();
}

/// Free a dictionary item, also clearing the value.
pub fn tv_dict_item_free(item_in: &Dictitem) {
    item_in.clear_tv();
}

/// Make a copy of a dictionary item.
pub fn tv_dict_item_copy(di: &Dictitem) -> DictitemId {
    let new_di = tv_dict_item_alloc(di.key_cstr());
    tv_copy(&di.tv(), &mut new_di.tv_mut());
    new_di
}

/// Remove item from dictionary and free it.
pub fn tv_dict_item_remove(dict: &TvDict, item: &Dictitem) {
    if let Some(idx) = dict.dv_hashtab.find_hi_idx(item.key_cstr()) {
        hash_remove(&dict.dv_hashtab, idx);
    } else {
        semsg!(tr(E_INTERN2), c"tv_dict_item_remove()");
    }
    tv_dict_item_free(item);
}

fn tv_dict_alloc_slot() -> DictId {
    DictId::from(TvDict::new_rc())
}

/// Allocate an empty dictionary.
pub fn tv_dict_alloc() -> DictId {
    let d = tv_dict_alloc_slot();
    gc_register_dict(&d);
    d
}

/// Allocate an empty static dictionary (not registered in GC).
pub fn tv_dict_alloc_static() -> DictId {
    tv_dict_alloc_slot()
}

/// Free a heap-allocated `TvDict` slot.
pub fn tv_dict_free_slot(d: &TvDict) {
    if let Some(idx) = d.dv_gc_idx.take() {
        gc_unregister_dict(idx);
    }
    let mut r = d.lua_table_ref.get();
    nlua_clear_ref(&mut r);
    d.lua_table_ref.set(r);
}

/// Free items contained in a dictionary.
pub fn tv_dict_free_contents(d: &TvDict) {
    hash_lock(&d.dv_hashtab);
    debug_assert!(d.dv_hashtab.ht_locked.get() > 0);
    for i in 0..d.dv_hashtab.array_len() {
        if d.dv_hashtab.ht_used.get() == 0 {
            break;
        }
        let di_id = d.dv_hashtab.slot(i).hi_di;
        if let Some(di) = di_id.opt() {
            d.dv_hashtab.remove_at(i);
            tv_dict_item_free(di);
        }
    }

    d.watchers.borrow_mut().clear();
    d.dv_copydict.set(DictId::NULL);

    hash_clear(&d.dv_hashtab);
}

/// Free a dictionary, including all items it contains.
pub fn tv_dict_free(d_in: &TvDict) {
    if TV_STATE.with(|s| s.tv_in_free_unref_items.get()) && d_in.dv_gc_idx.get().is_some() {
        return;
    }
    tv_list_dict_free_dict_inner(d_in);
    tv_dict_free_slot(d_in);
}

#[inline]
fn tv_list_dict_free_dict_inner(d: &TvDict) {
    tv_dict_free_contents(d);
}

/// Unreference a dictionary.
pub fn tv_dict_unref(d: Option<&TvDict>) {
    if let Some(d_ref) = d {
        d_ref.dv_refcount.set(d_ref.dv_refcount.get() - 1);
        if d_ref.dv_refcount.get() <= 0 {
            tv_dict_free(d_ref);
        }
    }
}

/// Find item in dictionary.
pub fn tv_dict_find(d: Option<&TvDict>, key: &CStr, len: isize) -> Option<DictitemId> {
    let d = d?;
    let hi = if len < 0 {
        d.dv_hashtab.find_hi(key)
    } else {
        let bytes = key.to_bytes();
        d.dv_hashtab
            .find_hi_len(&bytes[..(len as usize).min(bytes.len())])
    };
    hi.and_then(|hi| {
        if hi.hi_di.is_null() {
            None
        } else {
            Some(hi.hi_di)
        }
    })
}

/// Find item in dictionary and return it if present.
pub fn tv_dict_find_opt(d: Option<&TvDict>, key: &CStr, len: isize) -> Option<DictitemId> {
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

    tv_copy(&di.tv(), rettv);
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
    tv_get_number(&di.tv())
}

/// Gets a bool item from a dictionary, or a given default value.
pub fn tv_dict_get_bool(d: Option<&TvDict>, key: &std::ffi::CStr, def: i32) -> Varnumber {
    let Some(di) = tv_dict_find_opt(d, key, -1) else {
        return i64::from(def);
    };
    tv_get_bool(&di.tv())
}

/// Converts a dict to an environment.
pub fn tv_dict_to_env(denv: Option<&TvDict>) -> Vec<std::ffi::CString> {
    let env_size = tv_dict_len_inline(denv) as usize;
    let mut env = Vec::with_capacity(env_size);

    if let Some(denv) = denv {
        for (_, var) in denv.dv_hashtab.iter_items() {
            let var_tv = var.tv();
            let str_ = tv_get_string(&var_tv);
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
    tv_dict_get_string(d, key).map(std::borrow::Cow::into_owned)
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string<'a>(
    d: Option<&'a TvDict>,
    key: &CStr,
) -> Option<std::borrow::Cow<'a, std::ffi::CStr>> {
    let di = tv_dict_find_opt(d, key, -1)?;
    let s = tv_get_string(&di.tv()).into_owned();
    Some(std::borrow::Cow::Owned(s))
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string_buf<'a>(
    d: Option<&'a TvDict>,
    key: &CStr,
    numbuf: &'a mut [u8; NUMBUFLEN],
) -> Option<std::borrow::Cow<'a, std::ffi::CStr>> {
    tv_dict_get_string_buf_chk(d, key, -1, numbuf, None)
}

/// Get a string item from a dictionary.
pub fn tv_dict_get_string_buf_chk<'a>(
    d: Option<&'a TvDict>,
    key: &std::ffi::CStr,
    key_len: isize,
    numbuf: &'a mut [u8; NUMBUFLEN],
    def: Option<&'a std::ffi::CStr>,
) -> Option<std::borrow::Cow<'a, std::ffi::CStr>> {
    let Some(di) = tv_dict_find_opt(d, key, key_len) else {
        return def.map(std::borrow::Cow::Borrowed);
    };
    let di_tv = di.tv();
    if di_tv.v_type == VarType::VarString {
        return Some(std::borrow::Cow::Owned(di_tv.vval.v_string_owned()));
    }
    tv_get_string_buf_chk(&di_tv, numbuf)?;
    Some(std::borrow::Cow::Borrowed(crate::memory::bytes_as_cstr(
        numbuf,
    )))
}

/// Get a function from a dictionary.
pub fn tv_dict_get_callback(
    d: Option<&TvDict>,
    key: &std::ffi::CStr,
    key_len: isize,
    result: &Callback,
) -> bool {
    result.data.set(CallbackData::NONE);

    let Some(di) = tv_dict_find_opt(d, key, key_len) else {
        return true;
    };

    if !tv_is_func_inline(&di.tv()) && di.tv().v_type != VarType::VarString {
        emsg(tr(c"E6000: Argument is not a function or function name"));
        return false;
    }

    let mut tv = tv_initial_value();
    tv_copy(&di.tv(), &mut tv);
    set_selfdict(&mut tv, d.unwrap());
    let res = callback_from_typval(result, &tv);
    tv_clear(&mut tv);
    res
}

/// Check for adding a function to g: or l:.
pub fn tv_dict_wrong_func_name(d: &TvDict, tv: &TypVal, name: &CStr) -> i32 {
    i32::from(
        (ptr::eq(d, &*get_globvar_dict())
            || get_funccal_local_ht().is_some_and(|lht| ptr::eq(&raw const d.dv_hashtab, lht)))
            && tv_is_func_inline(tv)
            && var_wrong_func_name(name, true),
    )
}

/// Add item to dictionary.
pub fn tv_dict_add(d: &TvDict, item: &Dictitem) -> bool {
    let key = crate::memory::bytes_as_cstr(&item.di_key);
    if tv_dict_wrong_func_name(d, &item.tv(), key) != 0 {
        return false;
    }
    hash_add(&d.dv_hashtab, item)
}

/// Add a list entry to dictionary.
pub fn tv_dict_add_list(d: &TvDict, key: &CStr, list: Option<&TvList>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarList,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::List(ListId::from(list)),
    });
    if !tv_dict_add(d, &item) {
        item.tv_mut().vval = crate::eval::typval::TypvalVval::List(ListId::NULL);
        tv_dict_item_free(&item);
        return false;
    }
    tv_list_ref_inline(list);
    true
}

/// Add a typval entry to dictionary.
pub fn tv_dict_add_tv(d: &TvDict, key: &[u8], tv: &mut TypVal) -> bool {
    let item = tv_dict_item_alloc_len(key);
    tv_copy(&*tv, &mut item.tv_mut());
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Add a dictionary entry to dictionary.
pub fn tv_dict_add_dict(d: &TvDict, key: &CStr, dict: Option<&TvDict>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarDict,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::Dict(DictId::from(dict)),
    });
    if !tv_dict_add(d, &item) {
        item.tv_mut().vval = crate::eval::typval::TypvalVval::Dict(DictId::NULL);
        tv_dict_item_free(&item);
        return false;
    }

    if let Some(dict_ref) = dict {
        dict_ref.dv_refcount.set(dict_ref.dv_refcount.get() + 1);
    }

    true
}

/// Add a number entry to dictionary.
pub fn tv_dict_add_nr(d: &TvDict, key: &std::ffi::CStr, nr: Varnumber) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarNumber,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::Number(nr),
    });
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Add a floating point number entry to dictionary.
pub fn tv_dict_add_float(d: &TvDict, key: &std::ffi::CStr, nr: TvFloat) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarFloat,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::Float(nr),
    });
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Add a boolean entry to dictionary.
pub fn tv_dict_add_bool(d: &TvDict, key: &std::ffi::CStr, val: BoolVarValue) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarBool,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::Bool(val),
    });
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Add a string entry to dictionary.
pub fn tv_dict_add_str(d: &TvDict, key: &CStr, val: Option<&CStr>) -> bool {
    tv_dict_add_str_len(d, key, val.map(CStr::to_bytes))
}

/// Add a string entry to dictionary.
pub fn tv_dict_add_str_len(d: &TvDict, key: &CStr, val: Option<&[u8]>) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    item.set_tv(TypVal {
        v_type: VarType::VarString,
        v_lock: VAR_UNLOCKED,
        vval: crate::eval::typval::TypvalVval::from_opt_cstring(
            val.map(crate::memory::bytes_to_cstring),
        ),
    });
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Add a function entry to dictionary.
pub fn tv_dict_add_func(d: &TvDict, key: &std::ffi::CStr, fp: &UserFunc) -> bool {
    let item = tv_dict_item_alloc_len(key.to_bytes());
    {
        let mut item_tv = item.tv_mut();
        tv_set_string_len(&mut item_tv, Some(&fp.uf_name.as_bytes()[..fp.uf_namelen]));
        item_tv.v_type = VarType::VarFunc;
        func_ref(item_tv.vval.v_string_cstr());
    }
    if !tv_dict_add(d, &item) {
        tv_dict_item_free(&item);
        return false;
    }
    true
}

/// Clear all the keys of a Dictionary.
#[cfg(test)]
pub fn tv_dict_clear(d: &TvDict) {
    hash_lock(&d.dv_hashtab);
    debug_assert!(d.dv_hashtab.ht_locked.get() > 0);

    for i in 0..d.dv_hashtab.array_len() {
        if d.dv_hashtab.ht_used.get() == 0 {
            break;
        }
        let di_id = d.dv_hashtab.slot(i).hi_di;
        if let Some(di) = di_id.opt() {
            tv_dict_item_free(di);
            d.dv_hashtab.remove_at(i);
        }
    }

    hash_unlock(&d.dv_hashtab);
}

/// Extend dictionary with items from another dictionary.
pub fn tv_dict_extend(d1: &TvDict, d2: &TvDict, action: &[u8]) {
    let first_action = action.first().copied().unwrap_or(0);
    let watched = tv_dict_is_watched_inline(Some(d1));
    let arg_errmsg = tr(c"extend() argument");
    let arg_errmsg_len = arg_errmsg.to_bytes().len();

    if first_action == b'm' {
        hash_lock(&d2.dv_hashtab);
    }

    let mut todo = d2.dv_hashtab.ht_used.get();
    for i in 0..d2.dv_hashtab.array_len() {
        if todo == 0 {
            break;
        }
        let di2_id = d2.dv_hashtab.slot(i).hi_di;
        if let Some(di2) = di2_id.opt() {
            todo -= 1;
            let key2 = crate::memory::bytes_as_cstr(&di2.di_key);
            let di1_opt = tv_dict_find_opt(Some(d1), key2, -1);
            if d1.dv_scope.get() != VAR_NO_SCOPE && !valid_varname(di2.key_cstr()) {
                break;
            }
            if let Some(di1) = di1_opt {
                if first_action == b'e' {
                    semsg!(tr(c"E737: Key already exists: %s"), key2);
                    break;
                } else if first_action == b'f' && !ptr::eq(di2, &*di1) {
                    if value_check_lock(di1.tv().v_lock, Some(arg_errmsg), arg_errmsg_len)
                        || var_check_ro(i32::from(di1.di_flags.get()), arg_errmsg, arg_errmsg_len)
                    {
                        break;
                    }
                    if tv_dict_wrong_func_name(d1, &di2.tv(), key2) != 0 {
                        break;
                    }

                    let mut oldtv = tv_initial_value();
                    if watched {
                        tv_copy(&di1.tv(), &mut oldtv);
                    }

                    let mut new_tv = tv_initial_value();
                    tv_copy(&di2.tv(), &mut new_tv);
                    di1.clear_tv();
                    di1.set_tv(new_tv.clone());

                    if watched {
                        tv_dict_watcher_notify(d1, di1.key_cstr(), Some(&new_tv), Some(&oldtv));
                        tv_clear(&mut oldtv);
                    }
                }
            } else if first_action == b'm' {
                if tv_dict_add(d1, di2) {
                    d2.dv_hashtab.remove_at(i);
                    let new_tv = di2.tv().clone();
                    tv_dict_watcher_notify(d1, di2.key_cstr(), Some(&new_tv), None);
                }
            } else {
                let new_di = tv_dict_item_copy(di2);
                if !tv_dict_add(d1, &new_di) {
                    tv_dict_item_free(&new_di);
                } else if watched {
                    let new_tv = new_di.tv().clone();
                    tv_dict_watcher_notify(d1, new_di.key_cstr(), Some(&new_tv), None);
                }
            }
        }
    }

    if first_action == b'm' {
        hash_unlock(&d2.dv_hashtab);
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
        if !tv_equal(&di1.tv(), &di2.tv(), ic) {
            return false;
        }
    }
    true
}

/// Make a copy of dictionary.
pub fn tv_dict_copy(
    conv: Option<&Vimconv>,
    orig: Option<&TvDict>,
    deep: bool,
    copy_id: i32,
) -> Option<DictId> {
    let orig = orig?;

    let copy = tv_dict_alloc();
    if copy_id != 0 {
        orig.dv_copy_id.set(copy_id);
        orig.dv_copydict.set(copy.clone());
    }
    let items: Vec<DictitemId> = orig.dv_hashtab.iter_items().map(|(_, di)| di).collect();
    for di in items {
        if g_st().got_int.get() {
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
            let mut src_tv = di.tv().clone();
            let mut dst_tv = tv_initial_value();
            if !var_item_copy(conv, &mut src_tv, &mut dst_tv, deep, copy_id) {
                tv_dict_item_free(&new_di);
                break;
            }
            new_di.set_tv(dst_tv);
        } else {
            tv_copy(&di.tv(), &mut new_di.tv_mut());
        }
        if !tv_dict_add(&copy, &new_di) {
            tv_dict_item_free(&new_di);
            break;
        }
    }

    copy.dv_refcount.set(copy.dv_refcount.get() + 1);
    if g_st().got_int.get() {
        tv_dict_unref(copy.opt());
        return None;
    }

    Some(copy)
}

/// Set all existing keys in "dict" as read-only.
pub fn tv_dict_set_keys_readonly(dict: &TvDict) {
    for (_, di) in dict.dv_hashtab.iter_items() {
        di.di_flags
            .set(di.di_flags.get() | DI_FLAGS_RO | DI_FLAGS_FIX);
    }
}

/// Allocate an empty blob.
pub fn tv_blob_alloc() -> std::rc::Rc<TvBlob> {
    std::rc::Rc::new(TvBlob {
        bv_ga: std::cell::RefCell::new(Vec::new()),
        bv_refcount: std::cell::Cell::new(0),
        bv_lock: std::cell::Cell::new(0),
    })
}

/// Unreference a blob.
pub fn tv_blob_unref(b: Option<&TvBlob>) {
    if let Some(b_ref) = b {
        b_ref.bv_refcount.set(b_ref.bv_refcount.get() - 1);
    }
}

/// Allocate a `TvPartial`.
pub fn tv_partial_alloc() -> std::rc::Rc<TvPartial> {
    std::rc::Rc::new(TvPartial::default())
}

/// Clear and release references inside a `TvPartial` when its refcount reaches zero.
pub fn tv_partial_free(pt: &TvPartial) {
    for mut tv in pt.pt_argv.borrow_mut().drain(..) {
        tv_clear(&mut tv);
    }
    let dict = pt.pt_dict.replace(DictId::NULL);
    tv_dict_unref(dict.opt());
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
        (Some(b1), Some(b2)) => ptr::eq(b1, b2) || *b1.bv_ga.borrow() == *b2.bv_ga.borrow(),
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
        rettv.vval = crate::eval::typval::TypvalVval::Blob(BlobId::NULL);
    } else {
        let new_blob = tv_blob_alloc();
        {
            let src_blob = rettv.vval.v_blob_ref();
            new_blob
                .bv_ga
                .borrow_mut()
                .extend_from_slice(&src_blob.bv_ga.borrow()[(n1 as usize)..=(n2 as usize)]);
        }
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
    let len = tv_blob_len_inline(rettv.vval.v_blob_opt());
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
pub fn tv_blob_set_range(dest: &TvBlob, n1: Varnumber, n2: Varnumber, src: &mut TypVal) -> bool {
    if n2 - n1 + 1 != i64::from(tv_blob_len_inline(src.vval.v_blob_opt())) {
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
pub fn tv_blob_set_append(blob: &TvBlob, idx: i32, byte: u8) {
    let mut bv_guard = blob.bv_ga.borrow_mut();
    let bv = &mut *bv_guard;
    let len = bv.len() as i32;
    if idx < len {
        bv[idx as usize] = byte;
    } else if idx == len {
        bv.push(byte);
    }
}

/// "remove({blob})" function.
pub fn tv_blob_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    let b = argvars[0].vval.v_blob().rc();
    if let Some(ref b_ref) = b
        && value_check_lock(b_ref.bv_lock.get(), Some(arg_errmsg), TV_TRANSLATE)
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
            let removed = b_ref.bv_ga.borrow_mut().remove(idx as usize);
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
            blob.bv_ga.borrow_mut().extend(
                b_ref
                    .bv_ga
                    .borrow_mut()
                    .drain((idx as usize)..=(end as usize)),
            );
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

    let list_len = tv_list_len_inline(Some(l));
    if list_len > 0 {
        blob.bv_ga.borrow_mut().reserve(list_len as usize);
    }

    for li in crate::eval::typval::list_iter_mut(l.first_opt()) {
        let mut error = false;
        let n = tv_get_number_chk(&li.tv(), Some(&mut error));
        if error || !(0..=255).contains(&n) {
            if !error {
                semsg!(tr(E_INVALID_VALUE_FOR_BLOB_NR), n as i32);
            }
            blob.bv_ga.borrow_mut().clear();
            return;
        }
        blob.bv_ga.borrow_mut().push(n as u8);
    }
}

/// Allocate an empty list for a return value.
pub fn tv_list_alloc_ret(ret_tv: &mut TypVal, len: isize) -> ListId {
    let l = tv_list_alloc(len);
    tv_list_set_ret_inline(ret_tv, Some(&l));
    ret_tv.v_lock = VAR_UNLOCKED;
    l
}

pub fn tv_dict_alloc_lock(lock: VarLockStatus) -> DictId {
    let d = tv_dict_alloc();
    d.dv_lock.set(lock);
    d
}

/// Allocate an empty dictionary for a return value.
pub fn tv_dict_alloc_ret(ret_tv: &mut TypVal) -> DictId {
    let d = tv_dict_alloc_lock(VAR_UNLOCKED);
    tv_dict_set_ret_inline(ret_tv, &d);
    d
}

fn tv_dict2list(argvars: &mut [TypVal], rettv: &mut TypVal, what: DictListType) {
    if !tv_check_for_dict_arg(argvars, 0) {
        tv_list_alloc_ret(&mut *rettv, 0);
        return;
    }

    let d = argvars[0].vval.v_dict_opt();
    tv_list_alloc_ret(&mut *rettv, tv_dict_len_inline(d));
    let Some(d) = d else {
        return;
    };

    for (_, di) in d.dv_hashtab.iter_items() {
        let key_cstr = di.key_cstr();
        let mut tv_item = tv_initial_value();

        match what {
            DictListType::Keys => {
                tv_set_string(&mut tv_item, Some(key_cstr));
            }
            DictListType::Values => {
                tv_copy(&di.tv(), &mut tv_item);
            }
            DictListType::Items => {
                let sub_l = tv_list_alloc(2);
                tv_item.v_type = VarType::VarList;
                tv_item.vval = crate::eval::typval::TypvalVval::List(sub_l.clone());
                tv_list_ref_inline(Some(&sub_l));
                tv_list_append_string(&sub_l, Some(key_cstr), -1);
                tv_list_append_tv(&sub_l, &di.tv());
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
        tv_dict_find(Some(d), &tv_get_string(&argvars[1]), -1).is_some(),
    ));
}

/// "remove({dict})" function.
pub fn tv_dict_remove(argvars: &mut [TypVal], rettv: &mut TypVal, arg_errmsg: &CStr) {
    if argvars[2].v_type != VarType::VarUnknown {
        semsg!(tr(E_TOOMANYARG), c"remove()");
    } else if let Some(d) = argvars[0].vval.v_dict_opt()
        && !value_check_lock(d.dv_lock.get(), Some(arg_errmsg), TV_TRANSLATE)
        && let Some(key) = tv_get_string_chk(&argvars[1])
    {
        let Some(di) = tv_dict_find_opt(Some(d), &key, -1) else {
            semsg!(tr(E_DICTKEY), key);
            return;
        };
        if !var_check_fixed(i32::from(di.di_flags.get()), arg_errmsg, TV_TRANSLATE)
            && !var_check_ro(i32::from(di.di_flags.get()), arg_errmsg, TV_TRANSLATE)
        {
            *rettv = std::mem::replace(&mut *di.tv_mut(), tv_initial_value());
            tv_dict_item_remove(d, &di);
            if tv_dict_is_watched_inline(Some(d)) {
                tv_dict_watcher_notify(d, &key, None, Some(&*rettv));
            }
        }
    }
}

/// Allocate an empty blob for a return value.
pub fn tv_blob_alloc_ret(ret_tv: &mut TypVal) -> std::rc::Rc<TvBlob> {
    let b = tv_blob_alloc();
    tv_blob_set_ret_inline(ret_tv, b.clone());
    b
}

/// Copy a blob typval to a different typval.
pub fn tv_blob_copy(from: Option<&TvBlob>, to: &mut TypVal) {
    to.v_type = VarType::VarBlob;
    to.v_lock = VAR_UNLOCKED;
    if let Some(from) = from {
        let dst_blob = tv_blob_alloc_ret(to);
        dst_blob
            .bv_ga
            .borrow_mut()
            .extend_from_slice(&from.bv_ga.borrow());
    } else {
        to.vval = crate::eval::typval::TypvalVval::Blob(BlobId::NULL);
    }
}

#[inline]
fn nothing_conv_func_start(tv: &mut TypVal) -> bool {
    tv.v_lock = VAR_UNLOCKED;
    if tv.v_type == VarType::VarPartial {
        if let Some(pt) = tv.vval.v_partial_opt()
            && pt.pt_refcount.get() > 1
        {
            pt.pt_refcount.set(pt.pt_refcount.get() - (1));
            tv.vval = crate::eval::typval::TypvalVval::Partial(PartialId::NULL);
            return true;
        }
    } else {
        let fun = tv_take_string(tv);
        func_unref(fun.as_deref());
    }
    false
}

#[inline]
fn nothing_conv_func_end(pt: PartialId, copy_id: i32) {
    let Some(pt) = pt.opt() else {
        return;
    };
    let dict = pt.pt_dict.get();
    debug_assert!(dict.is_null() || dict.dv_copy_id.get() == copy_id);
    pt.pt_dict.set(DictId::NULL);
    pt.pt_argv.borrow_mut().clear();
    debug_assert!(pt.pt_refcount.get() <= 1);
    partial_unref(Some(pt));
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
            tv.vval = crate::eval::typval::TypvalVval::Blob(BlobId::NULL);
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
            tv.vval = crate::eval::typval::TypvalVval::Partial(PartialId::NULL);
            mpstack.push(MPConvStackVal {
                saved_copy_id: copy_id - 1,
                data: MPConvStackData::Partial {
                    stage: MPConvPartialStage::Args,
                    pt,
                },
            });
        }
        VarType::VarList => {
            let list = tv.vval.v_list();
            if list.is_null() || tv_list_len_inline(list.opt()) == 0 {
                tv_list_unref(list.opt());
                tv.vval = crate::eval::typval::TypvalVval::List(ListId::NULL);
                tv.v_lock = VAR_UNLOCKED;
                return true;
            }
            let saved_copy_id = list.lv_copy_id.get();
            if saved_copy_id == copy_id {
                return true;
            }

            tv.v_lock = VAR_UNLOCKED;
            tv.vval = crate::eval::typval::TypvalVval::List(ListId::NULL);
            if list.lv_refcount.get() > 1 {
                list.lv_refcount.set(list.lv_refcount.get() - 1);
                return true;
            }

            list.lv_copy_id.set(copy_id);

            let first_li = list.lv_first.get();
            let mpsv = MPConvStackVal {
                saved_copy_id,
                data: MPConvStackData::List { list, li: first_li },
            };
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
            let dict = tv.vval.v_dict();
            if dict.is_null() || dict.dv_hashtab.ht_used.get() == 0 {
                tv_dict_unref(dict.opt());
                tv.vval = crate::eval::typval::TypvalVval::Dict(DictId::NULL);
                tv.v_lock = VAR_UNLOCKED;
                return true;
            }
            let saved_copy_id = dict.dv_copy_id.get();
            if saved_copy_id == copy_id {
                return true;
            }

            tv.v_lock = VAR_UNLOCKED;
            tv.vval = crate::eval::typval::TypvalVval::Dict(DictId::NULL);
            if dict.dv_refcount.get() > 1 {
                dict.dv_refcount.set(dict.dv_refcount.get() - 1);
                return true;
            }

            dict.dv_copy_id.set(copy_id);

            let todo = dict.dv_hashtab.ht_used.get();
            let mpsv = MPConvStackVal {
                saved_copy_id,
                data: MPConvStackData::Dict {
                    dict,
                    hi_idx: 0,
                    todo,
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
        match mpstack[last_idx].data {
            MPConvStackData::Dict {
                ref dict,
                ref mut hi_idx,
                ref mut todo,
            } => {
                if *todo == 0 {
                    let dict_cl = dict.clone();
                    mpstack.pop();

                    dict_cl.dv_copy_id.set(saved_copy_id);
                    tv_dict_unref(dict_cl.opt());
                    continue;
                }
                while hashitem_empty(&dict.dv_hashtab.slot(*hi_idx)) {
                    *hi_idx += 1;
                }
                let hi = dict.dv_hashtab.slot(*hi_idx);
                let di = hi.hi_di;
                *todo -= 1;
                *hi_idx += 1;
                let mut item_tv = std::mem::replace(
                    &mut *di.tv_mut(),
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    },
                );
                let ok = nothing_convert_one_value(&mut mpstack, &mut item_tv, copy_id);
                di.set_tv(item_tv);
                if !ok {
                    return false;
                }
            }
            MPConvStackData::List {
                ref list,
                ref mut li,
            } => {
                let cur_li = li.clone();
                if cur_li.is_null() {
                    let list_cl = list.clone();
                    mpstack.pop();

                    list_cl.lv_copy_id.set(saved_copy_id);
                    tv_list_unref(list_cl.opt());

                    continue;
                }
                *li = cur_li.li_next.get();
                let mut item_tv = std::mem::replace(
                    &mut *cur_li.tv_mut(),
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    },
                );
                let ok = nothing_convert_one_value(&mut mpstack, &mut item_tv, copy_id);
                cur_li.set_tv(item_tv);
                if !ok {
                    return false;
                }
            }
            MPConvStackData::Partial { stage, ref pt } => {
                let pt = pt.clone();
                match stage {
                    MPConvPartialStage::Args => {
                        if let MPConvStackData::Partial {
                            stage: ref mut stage_ref,
                            ..
                        } = mpstack[last_idx].data
                        {
                            *stage_ref = MPConvPartialStage::SelfDict;
                        }
                        if !pt.is_null() && !pt.pt_argv.borrow().is_empty() {
                            let todo = pt.pt_argv.borrow().len();
                            mpstack.push(MPConvStackVal {
                                saved_copy_id: copy_id - 1,
                                data: MPConvStackData::PartialList { pt, idx: 0, todo },
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
                            DictId::NULL
                        } else {
                            pt.pt_dict.get()
                        };
                        if !dict.is_null() {
                            if dict.dv_hashtab.ht_used.get() == 0 {
                                pt.pt_dict.set(DictId::NULL);
                                tv_dict_unref(dict.opt());
                                continue;
                            }
                            let saved_copy_id = dict.dv_copy_id.get();
                            if saved_copy_id == copy_id {
                                continue;
                            }
                            pt.pt_dict.set(DictId::NULL);
                            if dict.dv_refcount.get() > 1 {
                                dict.dv_refcount.set(dict.dv_refcount.get() - 1);
                                continue;
                            }

                            dict.dv_copy_id.set(copy_id);

                            let todo = dict.dv_hashtab.ht_used.get();
                            let mpsv = MPConvStackVal {
                                saved_copy_id,
                                data: MPConvStackData::Dict {
                                    dict,
                                    hi_idx: 0,
                                    todo,
                                },
                            };
                            mpstack.push(mpsv);
                        }
                    }
                    MPConvPartialStage::End => {
                        nothing_conv_func_end(pt, copy_id);

                        mpstack.pop();
                    }
                }
            }
            MPConvStackData::PartialList {
                ref pt,
                ref mut idx,
                ref mut todo,
            } => {
                if *todo == 0 {
                    mpstack.pop();
                    continue;
                }
                let cur_idx = *idx;
                *idx += 1;
                *todo -= 1;
                let mut arg_tv = std::mem::take(&mut pt.pt_argv.borrow_mut()[cur_idx]);
                if !nothing_convert_one_value(&mut mpstack, &mut arg_tv, copy_id) {
                    return false;
                }
            }
            MPConvStackData::Pairs { .. } => unreachable!(),
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
    to.vval = from.vval.clone();
    match from.v_type {
        VarType::VarNumber | VarType::VarFloat | VarType::VarBool | VarType::VarSpecial => {}
        VarType::VarString | VarType::VarFunc => {
            if !from.vval.v_string_is_null() && from.v_type == VarType::VarFunc {
                func_ref(to.vval.v_string_cstr());
            }
        }
        VarType::VarPartial => {
            if let Some(pt) = to.vval.v_partial_opt() {
                pt.pt_refcount.set(pt.pt_refcount.get() + (1));
            }
        }
        VarType::VarBlob => {
            if let Some(b) = from.vval.v_blob_opt() {
                b.bv_refcount.set(b.bv_refcount.get() + 1);
            }
        }
        VarType::VarList => {
            tv_list_ref_inline(to.vval.v_list_opt());
        }
        VarType::VarDict => {
            if let Some(d) = from.vval.v_dict_opt() {
                d.dv_refcount.set(d.dv_refcount.get() + 1);
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
                && !(check_refcount && b.bv_refcount.get() > 1)
            {
                b.bv_lock.set(change_lock(lock, b.bv_lock.get()));
            }
        }
        VarType::VarList => {
            if let Some(l) = tv.vval.v_list_opt()
                && !(check_refcount && l.lv_refcount.get() > 1)
            {
                l.lv_lock.set(change_lock(lock, l.lv_lock.get()));
                if !(0..=1).contains(&deep) {
                    for li in crate::eval::typval::list_iter_mut(l.first_opt()) {
                        let mut item_tv = li.tv().clone();
                        tv_item_lock(&mut item_tv, deep - 1, lock, check_refcount);
                        li.tv_mut().v_lock = item_tv.v_lock;
                    }
                }
            }
        }
        VarType::VarDict => {
            if let Some(d) = tv.vval.v_dict_opt()
                && !(check_refcount && d.dv_refcount.get() > 1)
            {
                d.dv_lock.set(change_lock(lock, d.dv_lock.get()));
                if !(0..=1).contains(&deep) {
                    let items: Vec<DictitemId> =
                        d.dv_hashtab.iter_items().map(|(_, di)| di).collect();
                    for di in items {
                        let mut item_tv = di.tv().clone();
                        tv_item_lock(&mut item_tv, deep - 1, lock, check_refcount);
                        di.tv_mut().v_lock = item_tv.v_lock;
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
            && tv_list_locked_inline(tv.vval.v_list_opt()) == VAR_LOCKED)
        || (tv.v_type == VarType::VarDict
            && tv
                .vval
                .v_dict_opt()
                .is_some_and(|d| d.dv_lock.get() == VAR_LOCKED))
}

/// Return true if typval is locked.
pub fn tv_check_lock(tv: &TypVal, name: Option<&CStr>, name_len: usize) -> bool {
    let mut lock = VAR_UNLOCKED;

    match tv.v_type {
        VarType::VarBlob => {
            if let Some(b) = tv.vval.v_blob_opt() {
                lock = b.bv_lock.get();
            }
        }
        VarType::VarList => {
            if let Some(l) = tv.vval.v_list_opt() {
                lock = l.lv_lock.get();
            }
        }
        VarType::VarDict => {
            if let Some(d) = tv.vval.v_dict_opt() {
                lock = d.dv_lock.get();
            }
        }
        _ => {}
    }
    value_check_lock(tv.v_lock, name, name_len)
        || (lock != VAR_UNLOCKED && value_check_lock(lock, name, name_len))
}

/// Return true if variable "name" has a locked (immutable) value.
pub fn value_check_lock(lock: VarLockStatus, name_opt: Option<&CStr>, mut name_len: usize) -> bool {
    let name_opt = name_opt.filter(|s| !s.is_empty());
    match lock {
        VAR_UNLOCKED => return false,
        VAR_LOCKED => {
            if let Some(mut name) = name_opt {
                if name_len == TV_TRANSLATE {
                    name = tr(name);
                    name_len = name.to_bytes().len();
                } else if name_len == TV_CSTRING {
                    name_len = name.to_bytes().len();
                }
                semsg!(tr(E_VALUE_IS_LOCKED_STR), name_len as i32, name);
            } else {
                emsg(tr(E_VALUE_IS_LOCKED));
            }
        }
        VAR_FIXED => {
            if let Some(mut name) = name_opt {
                if name_len == TV_TRANSLATE {
                    name = tr(name);
                    name_len = name.to_bytes().len();
                } else if name_len == TV_CSTRING {
                    name_len = name.to_bytes().len();
                }
                semsg!(tr(E_CANNOT_CHANGE_VALUE_OF_STR), name_len as i32, name);
            } else {
                emsg(tr(E_CANNOT_CHANGE_VALUE));
            }
        }
        _ => {
            let _ = &mut name_len;
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
        TV_STATE.with(|s| s.tv_equal_recurse_limit.set(1000));
    }
    if cur_cnt >= TV_STATE.with(|s| s.tv_equal_recurse_limit.get()) {
        TV_STATE.with(|s| {
            s.tv_equal_recurse_limit
                .set(s.tv_equal_recurse_limit.get() - (1))
        });
        return true;
    }

    match tv1.v_type {
        VarType::VarList => {
            RECURSIVE_CNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = tv_list_equal(tv1.vval.v_list_opt(), tv2.vval.v_list_opt(), ic);
            RECURSIVE_CNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            r
        }
        VarType::VarDict => {
            RECURSIVE_CNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = tv_dict_equal(tv1.vval.v_dict_opt(), tv2.vval.v_dict_opt(), ic);
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
        VarType::VarBlob => tv_blob_equal(tv1.vval.v_blob_opt(), tv2.vval.v_blob_opt()),
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
            if !tv.vval.v_string_is_null() {
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
    let did_emsg_before = g_st().did_emsg.get();
    let mut lnum = tv_get_number_chk(tv, None) as Linenr;
    if lnum <= 0 && did_emsg_before == g_st().did_emsg.get() && tv.v_type != VarType::VarNumber {
        let mut fnum: i32 = 0;
        if let Some(fp) = var2fpos(tv, true, &mut fnum, false, &curwin_mut()) {
            lnum = fp.lnum;
        }
    }
    lnum
}

/// Get the line number from Vimscript object.
pub fn tv_get_lnum_buf(tv: &TypVal, buf: Option<&FileBuf>) -> Linenr {
    if tv.v_type == VarType::VarString
        && !tv.vval.v_string_is_null()
        && tv.vval.v_string_bytes() == b"$"
        && let Some(buf) = buf
    {
        return buf.b_ml.ml_line_count.get();
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
    if args[idx as usize].vval.v_string_is_empty() {
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
        VarType::VarString => Some(tv.vval.v_string_cstr_or_empty()),
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
pub fn tv_get_string_chk(tv: &TypVal) -> Option<std::borrow::Cow<'_, std::ffi::CStr>> {
    match tv.v_type {
        VarType::VarString => Some(std::borrow::Cow::Borrowed(tv.vval.v_string_cstr_or_empty())),
        VarType::VarBool => Some(std::borrow::Cow::Borrowed(
            ENCODE_BOOL_VAR_NAMES[tv.vval.v_bool() as usize],
        )),
        VarType::VarSpecial => Some(std::borrow::Cow::Borrowed(
            ENCODE_SPECIAL_VAR_NAMES[tv.vval.v_special() as usize],
        )),
        _ => {
            let mut buf = [0u8; NUMBUFLEN];
            tv_get_string_buf_chk(tv, &mut buf).map(|s| std::borrow::Cow::Owned(s.to_owned()))
        }
    }
}

/// Get the string value of a "stringish" Vimscript object.
pub fn tv_get_string(tv: &TypVal) -> std::borrow::Cow<'_, std::ffi::CStr> {
    tv_get_string_chk(tv).unwrap_or(std::borrow::Cow::Borrowed(c""))
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
        VarType::VarFunc | VarType::VarString => !tv.vval.v_string_is_empty(),
        VarType::VarList => tv.vval.v_list_opt().is_some_and(|l| l.lv_len.get() > 0),
        VarType::VarDict => tv
            .vval
            .v_dict_opt()
            .is_some_and(|d| d.dv_hashtab.ht_used.get() > 0),
        VarType::VarBool => tv.vval.v_bool() == K_BOOL_VAR_TRUE_CONST,
        VarType::VarSpecial => tv.vval.v_special() != K_SPECIAL_VAR_NULL_CONST,
        VarType::VarBlob => tv
            .vval
            .v_blob_opt()
            .is_some_and(|b| !b.bv_ga.borrow().is_empty()),
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
    semsg!(tr(E_VAL_TOO_LARGE), crate::memory::bytes_as_cstr(&buf));
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
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() + (1));

            // Basic alloc, append_number, find, reverse
            let l = tv_list_alloc(0);
            l.lv_refcount.set(1);
            tv_list_append_number(&l, 10);
            tv_list_append_number(&l, 20);
            tv_list_append_number(&l, 30);
            assert_eq!(tv_list_len_inline(Some(&l)), 3);

            let item1 = tv_list_find(Some(&l), 1).unwrap();
            assert_eq!(item1.tv().vval.v_number(), 20);
            assert_eq!(tv_list_idx_of_item(Some(&l), Some(&item1)), 1);
            assert_eq!(tv_list_idx_of_item(Some(&l), None), -1);

            let mut err = false;
            assert_eq!(tv_list_find_nr(Some(&l), 0, Some(&mut err)), 10);
            assert!(!err);
            assert_eq!(tv_list_find_nr(Some(&l), -1, Some(&mut err)), 30);
            assert!(!err);
            assert_eq!(tv_list_find_nr(Some(&l), 10, Some(&mut err)), -1);
            assert!(err);

            tv_list_reverse(&l);
            let first = tv_list_find(Some(&l), 0).unwrap();
            assert_eq!(first.tv().vval.v_number(), 30);

            // Watchers + item_remove + drop_items + remove_items
            let lw1 = std::rc::Rc::new(Listwatch {
                lw_item: crate::api::private::validate::CloneCell::new(l.lv_first.get()),
            });
            let lw2 = std::rc::Rc::new(Listwatch {
                lw_item: crate::api::private::validate::CloneCell::new(l.lv_last.get()),
            });
            tv_list_watch_add(&l, lw1.clone());
            tv_list_watch_add(&l, lw2.clone());

            let first = l.lv_first.get();
            let second = first.li_next.get();
            let next_after_first = tv_list_item_remove(&l, &first);
            assert_eq!(ListitemId::from(next_after_first), second);
            assert_eq!(lw1.lw_item.get(), second);
            assert_eq!(tv_list_len_inline(Some(&l)), 2);

            tv_list_watch_remove(&l, &lw1);
            tv_list_watch_remove(&l, &lw2);
            tv_list_unref(Some(&l));

            // insert, insert_tv, append_string, append_allocated_string, append_tv, append_owned_tv
            let l2 = tv_list_alloc(0);
            l2.lv_refcount.set(1);
            tv_list_append_string(&l2, Some(c"hello"), 5);
            tv_list_append_string(&l2, Some(c"world"), -1);
            let mut tv_ins = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(99),
            };
            let first2 = l2.first_opt();
            tv_list_insert_tv(&l2, &mut tv_ins, first2.as_deref());
            assert_eq!(tv_list_len_inline(Some(&l2)), 3);
            assert_eq!(tv_list_find_nr(Some(&l2), 0, None), 99);
            assert_eq!(tv_list_find_str(Some(&l2), 1).as_deref(), Some(c"hello"));
            assert_eq!(tv_list_find_str(Some(&l2), 2).as_deref(), Some(c"world"));

            // copy, extend, concat, join, equal
            let l2_copy = tv_list_copy(None, Some(&l2), true, 0);
            l2_copy.as_ref().unwrap().lv_refcount.set(1);
            assert!(tv_list_equal(Some(&l2), l2_copy.as_deref(), false));
            assert!(tv_list_equal(None, None, false));
            assert!(!tv_list_equal(Some(&l2), None, false));

            let mut joined: Vec<u8> = Vec::new();
            assert!(tv_list_join(&mut joined, Some(&l2), c", "));
            assert_eq!(joined.as_slice(), b"99, hello, world");

            let mut cat_tv = tv_initial_value();
            assert!(tv_list_concat(Some(&l2), l2_copy.as_deref(), &mut cat_tv));
            assert_eq!(tv_list_len_inline(cat_tv.vval.v_list_opt()), 6);
            tv_clear(&mut cat_tv);

            tv_list_extend(&l2, l2_copy.as_deref(), None);
            assert_eq!(tv_list_len_inline(Some(&l2)), 6);

            // remove_items range
            let first_item = l2.lv_first.get();
            let second_item = first_item.li_next.get();
            tv_list_remove_items(&l2, &first_item, &second_item);
            assert_eq!(tv_list_len_inline(Some(&l2)), 4);

            tv_list_unref(l2_copy.as_deref());
            tv_list_unref(Some(&l2));
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() - (1));
        }
    }

    #[test]
    fn test_tv_dict_operations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() + (1));

            let d = tv_dict_alloc();
            d.dv_refcount.set(1);
            assert!(tv_dict_add_nr(&d, c"foo", 42));
            assert!(tv_dict_add_float(&d, c"flt", 3.5));
            assert!(tv_dict_add_str(&d, c"str", Some(c"bar")));
            assert!(tv_dict_add_str_len(&d, c"alloc", Some(b"baz")));
            assert!(tv_dict_has_key(Some(&d), c"foo"));
            assert!(!tv_dict_has_key(Some(&d), c"missing"));

            assert_eq!(tv_dict_get_number(Some(&d), c"foo"), 42);
            assert_eq!(tv_dict_get_number(Some(&d), c"missing"), 0);

            let mut numbuf = [0u8; NUMBUFLEN];
            assert_eq!(
                tv_dict_get_string(Some(&d), c"str").as_deref(),
                Some(c"bar")
            );
            let saved = tv_dict_get_cstring(Some(&d), c"foo").unwrap();
            assert_eq!(saved.as_bytes(), b"42");

            assert_eq!(
                tv_dict_get_string_buf_chk(Some(&d), c"missing", 7, &mut numbuf, Some(c"def"))
                    .as_deref(),
                Some(c"def")
            );

            // Copy, equal, extend, readonly, clear
            let d_copy = tv_dict_copy(None, Some(&d), true, 0);
            let d_copy_ref = d_copy.unwrap();
            d_copy_ref.dv_refcount.set(1);
            assert!(tv_dict_equal(Some(&d), Some(&d_copy_ref), false));
            assert!(tv_dict_equal(None, None, false));
            assert!(!tv_dict_equal(Some(&d), None, false));

            let d2 = tv_dict_alloc();
            d2.dv_refcount.set(1);
            assert!(tv_dict_add_nr(&d2, c"extra", 7));
            tv_dict_extend(&d, &d2, b"force");
            assert_eq!(tv_dict_get_number(Some(&d), c"extra"), 7);
            assert!(!tv_dict_equal(Some(&d), Some(&d_copy_ref), false));

            tv_dict_set_keys_readonly(&d2);
            let di_extra = tv_dict_find_opt(Some(&d2), c"extra", -1).unwrap();
            assert_ne!(di_extra.di_flags.get() & (DI_FLAGS_RO | DI_FLAGS_FIX), 0);

            tv_dict_clear(&d_copy_ref);
            assert_eq!(tv_dict_len_inline(Some(&d_copy_ref)), 0);

            tv_dict_unref(Some(&d2));
            tv_dict_unref(Some(&d_copy_ref));
            tv_dict_unref(Some(&d));
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() - (1));
        }
    }

    #[test]
    fn test_tv2bool_and_typval_getters_locks_and_checks() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() + (1));

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
            crate::globals::g_st()
                .emsg_skip
                .set(g_st().emsg_skip.get() - (1));
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
            tv_list_append_number(&inner, 1);
            let outer = tv_list_alloc(3);
            outer.lv_refcount.set(1);
            for _ in 0..3 {
                tv_list_append_list(&outer, Some(&inner));
            }
            assert_eq!(inner.lv_refcount.get(), 3);
            let mut tv = TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(outer),
            };
            tv_clear(&mut tv);
            assert!(tv.vval.v_list().is_null());

            // [&l [], *l, *l]
            let empty_inner = tv_list_alloc(0);
            let outer2 = tv_list_alloc(3);
            outer2.lv_refcount.set(1);
            for _ in 0..3 {
                tv_list_append_list(&outer2, Some(&empty_inner));
            }
            assert_eq!(empty_inner.lv_refcount.get(), 3);
            let mut tv2 = TypVal {
                v_type: VarType::VarList,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::List(outer2),
            };
            tv_clear(&mut tv2);
            assert!(tv2.vval.v_list().is_null());

            // [&d {}, *d] and [&d {a: 1}, *d]
            for with_item in [false, true] {
                let d_inner = tv_dict_alloc();
                if with_item {
                    assert!(tv_dict_add_nr(&d_inner, c"a", 1));
                }
                let outer_d = tv_list_alloc(2);
                outer_d.lv_refcount.set(1);
                tv_list_append_dict(&outer_d, Some(&d_inner));
                tv_list_append_dict(&outer_d, Some(&d_inner));
                assert_eq!(d_inner.dv_refcount.get(), 2);
                let mut tv_d = TypVal {
                    v_type: VarType::VarList,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::List(outer_d),
                };
                tv_clear(&mut tv_d);
                assert!(tv_d.vval.v_list().is_null());
            }
        }
    }
}
