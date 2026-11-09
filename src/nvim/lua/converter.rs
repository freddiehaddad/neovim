//! Conversions between Lua stack values, Vimscript `TypVal`, and API `Object`
//! (`src/nvim/lua/converter.c`).

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::api::private::helpers::{
    api_free_array, api_free_dict, api_free_object, api_set_error, api_typename, arena_array_opt,
    arena_dict_opt, cbuf_to_arena_str_opt, error_set, nil_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::eval::decode::{
    K_MP_EXT, K_MP_INTEGER, K_MP_MAP, decode_create_map_special_dict, decode_string,
};
use crate::eval::encode::{
    K_MP_ARRAY, K_MP_BOOLEAN, K_MP_FLOAT, K_MP_NIL, K_MP_STRING, MPContainerRef, MPConvStackData,
    MPConvStackVal, encode_vim_list_to_buf,
};
use crate::eval::typval::{
    TypVal, TypvalVval, VAR_UNLOCKED, VarType, tv_blob_len, tv_clear, tv_copy, tv_dict_add,
    tv_dict_alloc, tv_dict_find, tv_dict_item_alloc_len, tv_list_alloc, tv_list_append_list,
    tv_list_append_owned_tv, tv_list_first, tv_list_last, tv_list_len, tv_list_ref,
};
use crate::eval::userfunc::{FC_LUAREF, find_func, register_luafunc};
use crate::eval::vars::{NUM_MSGPACK_TYPES, eval_msgpack_type_list};
use crate::eval::{get_copy_id, partial_name};
use crate::hashtab::hashitem_empty;
use crate::highlight_group::syn_check_group;

use crate::lua::executor::{api_free_luaref, nlua_pushref, nlua_ref_global};
pub use crate::lua::ffi::*;
use crate::memory::{Arena, arena_memdupz_opt};
use crate::message::{emsg, internal_error, semsg};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use std::ffi::{CStr, c_char, c_void};

pub(crate) const K_NLUA_PUSH_SPECIAL: i32 = 0x01;
const K_NLUA_PUSH_FREE_REFS: i32 = 0x02;

const TYPE_IDX_VALUE: bool = true;
const VAL_IDX_VALUE: bool = false;

struct LuaConverterState {
    typval_conv_special: bool,
}

impl LuaConverterState {
    const fn new() -> Self {
        Self {
            typval_conv_special: false,
        }
    }
}

static LUACONV_STATE: crate::globals::GlobalState<LuaConverterState> =
    crate::globals::GlobalState::new(LuaConverterState::new());

#[inline]
fn luaconv_st() -> crate::globals::GraphPtr<LuaConverterState> {
    LUACONV_STATE.handle()
}

#[repr(C)]
pub struct NluaRefState {
    pub nil_ref: LuaRef,
    pub empty_dict_ref: LuaRef,
    pub ref_count: i32,
}

#[repr(C)]
pub struct OptKeySet {
    pub is_set_: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeySetLink {
    pub str: Option<&'static CStr>,
    pub ptr_off: usize,
    pub key_type: i32,
    pub opt_index: i32,
    pub is_hlgroup: bool,
}

pub type FieldHashfn = Option<fn(key: &[u8]) -> Option<&'static KeySetLink>>;

#[derive(Clone, Copy)]
struct LuaTableProps {
    maxidx: usize,
    string_keys_num: usize,
    has_string_with_nul: bool,
    table_type: ObjectType,
    val: lua_Number,
    has_type_key: bool,
}

#[derive(Clone, Copy)]
struct TVPopStackItem {
    tv: crate::globals::GraphPtr<TypVal>,
    list_len: usize,
    container: bool,
    special: bool,
    idx: i32,
}

#[derive(Clone, Copy)]
struct ObjPopStackItem {
    obj: crate::globals::GraphPtr<Object>,
    container: bool,
}

#[inline]
pub fn lua_push_bytes(lstate: *mut lua_State, bytes: &[u8]) {
    unsafe {
        lua_pushlstring(
            lstate,
            if bytes.is_empty() {
                c"".as_ptr()
            } else {
                bytes.as_ptr().cast::<c_char>()
            },
            bytes.len(),
        )
    };
}

#[inline]
pub fn lua_pushstring(lstate: *mut lua_State, s: &CStr) {
    unsafe { c_lua_pushstring(lstate, s.as_ptr()) }
}

#[inline]
pub fn lua_pushstring_opt(lstate: *mut lua_State, s: Option<&CStr>) {
    unsafe { c_lua_pushstring(lstate, s.map_or(std::ptr::null(), CStr::as_ptr)) }
}

#[inline]
pub fn lua_getfield(lstate: *mut lua_State, idx: i32, k: &CStr) {
    unsafe { c_lua_getfield(lstate, idx, k.as_ptr()) }
}

#[inline]
pub fn lua_setfield(lstate: *mut lua_State, idx: i32, k: &CStr) {
    unsafe { c_lua_setfield(lstate, idx, k.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_newmetatable(lstate: *mut lua_State, tname: &CStr) -> i32 {
    unsafe { c_luaL_newmetatable(lstate, tname.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_getmetafield(lstate: *mut lua_State, obj: i32, e: &CStr) -> i32 {
    unsafe { c_luaL_getmetafield(lstate, obj, e.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata(lstate: *mut lua_State, ud: i32, tname: &CStr) -> *mut c_void {
    unsafe { c_luaL_checkudata(lstate, ud, tname.as_ptr()) }
}

#[inline]
pub fn lua_createtable_safe(lstate: *mut lua_State, narr: i32, nrec: i32) {
    unsafe { lua_createtable(lstate, narr, nrec) };
}

#[inline]
pub fn lua_settable_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_settable(lstate, idx) };
}

#[inline]
pub fn lua_rawseti_safe(lstate: *mut lua_State, idx: i32, n: i32) {
    unsafe { lua_rawseti(lstate, idx, n) };
}

#[inline]
pub fn lua_setmetatable_safe(lstate: *mut lua_State, objindex: i32) -> i32 {
    unsafe { lua_setmetatable(lstate, objindex) }
}

#[inline]
pub fn lua_settop_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_settop(lstate, idx) };
}

#[inline]
pub fn lua_type_safe(lstate: *mut lua_State, idx: i32) -> i32 {
    unsafe { lua_type(lstate, idx) }
}

#[inline]
pub fn lua_isnumber_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_isnumber(lstate, idx) != 0 }
}

#[inline]
pub fn lua_isstring_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_isstring(lstate, idx) != 0 }
}

#[inline]
pub fn lua_toboolean_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_toboolean(lstate, idx) != 0 }
}

#[inline]
pub fn lua_tonumber_safe(lstate: *mut lua_State, idx: i32) -> lua_Number {
    unsafe { lua_tonumber(lstate, idx) }
}

#[inline]
pub fn lua_objlen_safe(lstate: *mut lua_State, idx: i32) -> usize {
    unsafe { lua_objlen(lstate, idx) }
}

#[inline]
pub fn lua_next_safe(lstate: *mut lua_State, idx: i32) -> i32 {
    unsafe { lua_next(lstate, idx) }
}

#[inline]
pub fn lua_pushnil_safe(lstate: *mut lua_State) {
    unsafe { lua_pushnil(lstate) };
}

#[inline]
pub fn lua_pushboolean_safe(lstate: *mut lua_State, b: bool) {
    unsafe { lua_pushboolean(lstate, i32::from(b)) };
}

#[inline]
pub fn lua_pushinteger_safe(lstate: *mut lua_State, n: lua_Integer) {
    unsafe { lua_pushinteger(lstate, n) };
}

#[inline]
pub fn lua_pushnumber_safe(lstate: *mut lua_State, n: lua_Number) {
    unsafe { lua_pushnumber(lstate, n) };
}

#[inline]
pub fn lua_pushvalue_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_pushvalue(lstate, idx) };
}

#[inline]
pub fn lua_pushcfunction_safe(
    lstate: *mut lua_State,
    f: extern "C-unwind" fn(*mut lua_State) -> i32,
) {
    unsafe { lua_pushcclosure(lstate, Some(f), 0) };
}

#[inline]
pub fn lua_register_funcs(
    lstate: *mut lua_State,
    funcs: &[(&'static CStr, extern "C-unwind" fn(*mut lua_State) -> i32)],
) {
    for &(name, func) in funcs {
        lua_pushcfunction_safe(lstate, func);
        lua_setfield(lstate, -2, name);
    }
}

#[inline]
pub fn lua_concat_safe(lstate: *mut lua_State, n: i32) {
    unsafe { lua_concat(lstate, n) };
}

#[inline]
pub fn lua_pcall_safe(lstate: *mut lua_State, nargs: i32, nresults: i32, errfunc: i32) -> i32 {
    unsafe { lua_pcall(lstate, nargs, nresults, errfunc) }
}

#[inline]
pub fn lua_error_safe(lstate: *mut lua_State) -> i32 {
    unsafe { lua_error(lstate) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_where_safe(lstate: *mut lua_State, lvl: i32) {
    unsafe { luaL_where(lstate, lvl) };
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_cstr(lstate: *mut lua_State, msg: &CStr) -> i32 {
    unsafe { luaL_error(lstate, c"%s".as_ptr(), msg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_cstr_arg(lstate: *mut lua_State, fmt: &CStr, arg: &CStr) -> i32 {
    unsafe { luaL_error(lstate, fmt.as_ptr(), arg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkinteger_safe(lstate: *mut lua_State, numarg: i32) -> lua_Integer {
    unsafe { luaL_checkinteger(lstate, numarg) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkbytes(lstate: *mut lua_State, numarg: i32) -> &'static [u8] {
    let mut len = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkbytes_with_nul(
    lstate: *mut lua_State,
    numarg: i32,
) -> (&'static [u8], &'static [u8]) {
    let mut len = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut len) };
    if ptr.is_null() {
        (&[], &[0])
    } else {
        let with_nul = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len + 1) };
        (&with_nul[..len], with_nul)
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_check_nvim_string(lstate: *mut lua_State, numarg: i32) -> NvimString {
    let mut size = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut size) };
    NvimString {
        data: crate::globals::GraphPtr::new(ptr.cast_mut().cast()),
        size,
    }
}

#[inline]
pub fn lua_tolstring_bytes(lstate: *mut lua_State, idx: i32) -> &'static [u8] {
    let mut len = 0usize;
    let ptr = unsafe { lua_tolstring(lstate, idx, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
}

#[inline]
pub fn lua_tolstring_opt_cstr(lstate: *mut lua_State, idx: i32) -> Option<&'static CStr> {
    let ptr = unsafe { lua_tolstring(lstate, idx, std::ptr::null_mut()) };
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) })
}

#[inline]
pub fn lua_newuserdata_typed<T>(lstate: *mut lua_State, val: T) -> &'static mut T {
    unsafe {
        let p = lua_newuserdata(lstate, size_of::<T>()).cast::<T>();
        p.write(val);
        &mut *p
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata_typed<T>(lstate: *mut lua_State, ud: i32, tname: &CStr) -> &'static mut T {
    unsafe { &mut *luaL_checkudata(lstate, ud, tname).cast::<T>() }
}

#[inline]
pub fn lua_tolstring_cstr(lstate: *mut lua_State, idx: i32) -> &'static CStr {
    let ptr = unsafe { lua_tolstring(lstate, idx, std::ptr::null_mut()) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { CStr::from_ptr(ptr) }
    }
}

#[inline]
pub fn nlua_gettop(lstate: *mut lua_State) -> i32 {
    unsafe { lua_gettop(lstate) }
}

#[inline]
pub fn nlua_fast_api_disabled_error(lstate: *mut lua_State, fn_name: &CStr) -> i32 {
    unsafe {
        luaL_error(
            lstate,
            crate::errors::E_FAST_API_DISABLED.as_ptr(),
            fn_name.as_ptr(),
        )
    }
}

#[inline]
pub fn nlua_finish_api_call(
    lstate: *mut lua_State,
    arena: &mut Arena,
    err: &mut Error,
    err_param: Option<&'static CStr>,
    nret: i32,
) -> i32 {
    crate::memory::arena_mem_free(crate::memory::arena_finish(arena));
    if err.err_type != ErrorType::ErrorTypeNone {
        unsafe { luaL_where(lstate, 1) };

        if let Some(err_param) = err_param {
            lua_pushstring(lstate, c"Invalid '");
            lua_pushstring(lstate, err_param);
            lua_pushstring(lstate, c"': ");
        }
        lua_pushstring_opt(lstate, err.msg_cstr());
        crate::api::private::helpers::api_clear_error(err);

        unsafe { lua_concat(lstate, if err_param.is_some() { 5 } else { 2 }) };

        return unsafe { lua_error(lstate) };
    }
    if nret < 0 {
        unsafe { lua_gettop(lstate) }
    } else {
        nret
    }
}

#[inline]
pub fn nlua_register_api_table(
    lstate: *mut lua_State,
    api_functions: &[(&'static CStr, extern "C-unwind" fn(*mut lua_State) -> i32)],
) {
    unsafe {
        lua_createtable(lstate, 0, api_functions.len() as i32);
        for &(name, func) in api_functions {
            lua_pushcclosure(lstate, Some(func), 0);
            lua_setfield(lstate, -2, name);
        }
        lua_setfield(lstate, -2, c"api");
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_register(lstate: *mut lua_State, libname: Option<&CStr>, l: &[luaL_Reg]) {
    unsafe {
        c_luaL_register(
            lstate,
            libname.map_or(std::ptr::null(), CStr::as_ptr),
            l.as_ptr(),
        )
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkstack(lstate: *mut lua_State, sz: i32, msg: &CStr) {
    unsafe { c_luaL_checkstack(lstate, sz, msg.as_ptr()) }
}

#[cfg(feature = "vendor-bit")]
#[inline]
#[allow(non_snake_case)]
pub fn luaL_typerror(lstate: *mut lua_State, narg: i32, tname: &CStr) -> i32 {
    unsafe { c_luaL_typerror(lstate, narg, tname.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_argerror(lstate: *mut lua_State, numarg: i32, extramsg: &CStr) -> i32 {
    unsafe { c_luaL_argerror(lstate, numarg, extramsg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_loadbuffer(lstate: *mut lua_State, buff: &[u8], name: &CStr) -> i32 {
    unsafe {
        c_luaL_loadbuffer(
            lstate,
            if buff.is_empty() {
                c"".as_ptr()
            } else {
                buff.as_ptr().cast()
            },
            buff.len(),
            name.as_ptr(),
        )
    }
}

#[inline]
pub fn lua_getinfo(lstate: *mut lua_State, what: &CStr, ar: &mut lua_Debug) -> i32 {
    unsafe { c_lua_getinfo(lstate, what.as_ptr(), ar) }
}

#[inline]
pub const fn lua_upvalueindex(i: i32) -> i32 {
    LUA_GLOBALSINDEX - i
}

#[inline]
pub fn lua_pop(lstate: *mut lua_State, n: i32) {
    unsafe {
        lua_settop(lstate, -n - 1);
    }
}

#[inline]
pub fn lua_newtable(lstate: *mut lua_State) {
    unsafe {
        lua_createtable(lstate, 0, 0);
    }
}

#[inline]
pub fn lua_pushcfunction(lstate: *mut lua_State, f: lua_CFunction) {
    unsafe {
        lua_pushcclosure(lstate, f, 0);
    }
}

#[inline]
pub fn lua_setglobal(lstate: *mut lua_State, s: &std::ffi::CStr) {
    lua_setfield(lstate, LUA_GLOBALSINDEX, s);
}

#[inline]
pub fn lua_getglobal(lstate: *mut lua_State, s: &std::ffi::CStr) {
    lua_getfield(lstate, LUA_GLOBALSINDEX, s);
}

#[inline]
pub fn lua_tostring(lstate: *mut lua_State, i: i32) -> Option<&'static CStr> {
    let ptr = unsafe { lua_tolstring(lstate, i, std::ptr::null_mut()) };
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) })
}

#[inline]
pub fn lua_isfunction(lstate: *mut lua_State, n: i32) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TFUNCTION }
}

#[inline]
pub fn lua_istable(lstate: *mut lua_State, n: i32) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TTABLE }
}

#[inline]
pub fn lua_isnil(lstate: *mut lua_State, n: i32) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TNIL }
}

#[inline]
pub fn lua_isboolean(lstate: *mut lua_State, n: i32) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TBOOLEAN }
}

#[inline]
pub fn lua_isnoneornil(lstate: *mut lua_State, n: i32) -> bool {
    unsafe { lua_type(lstate, n) <= 0 }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_typename(lstate: *mut lua_State, i: i32) -> &'static CStr {
    let ptr = unsafe { lua_typename(lstate, lua_type(lstate, i)) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { CStr::from_ptr(ptr) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_getmetatable(lstate: *mut lua_State, n: &CStr) {
    lua_getfield(lstate, LUA_REGISTRYINDEX, n);
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_argcheck(lstate: *mut lua_State, cond: bool, numarg: i32, extramsg: &std::ffi::CStr) {
    if !cond {
        luaL_argerror(lstate, numarg, extramsg);
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkstring(lstate: *mut lua_State, n: i32) -> &'static CStr {
    let ptr = unsafe { luaL_checklstring(lstate, n, std::ptr::null_mut()) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { CStr::from_ptr(ptr) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkint(lstate: *mut lua_State, n: i32) -> i32 {
    unsafe { luaL_checkinteger(lstate, n) as i32 }
}

#[inline]
fn nlua_push_type_idx(lstate: *mut lua_State) {
    unsafe {
        lua_pushboolean(lstate, i32::from(TYPE_IDX_VALUE));
    }
}

#[inline]
fn nlua_push_val_idx(lstate: *mut lua_State) {
    unsafe {
        lua_pushboolean(lstate, i32::from(VAL_IDX_VALUE));
    }
}

#[inline]
fn nlua_push_type(lstate: *mut lua_State, obj_type: ObjectType) {
    unsafe {
        lua_pushnumber(lstate, (obj_type as i32) as lua_Number);
    }
}

#[inline]
fn nlua_create_typed_table(lstate: *mut lua_State, narr: usize, nrec: usize, obj_type: ObjectType) {
    unsafe {
        lua_createtable(lstate, narr as i32, (1 + nrec) as i32);
        nlua_push_type_idx(lstate);
        nlua_push_type(lstate, obj_type);
        lua_rawset(lstate, -3);
    }
}

fn nlua_traverse_table(lstate: *mut lua_State) -> LuaTableProps {
    unsafe {
        let mut tsize = 0usize;
        let mut val_type = 0_i32;
        let mut has_val_key = false;
        let mut other_keys_num = 0usize;
        let mut ret = LuaTableProps {
            maxidx: 0,
            string_keys_num: 0,
            has_string_with_nul: false,
            table_type: ObjectType::ObjectTypeNil,
            val: 0.0,
            has_type_key: false,
        };

        if lua_checkstack(lstate, 3) == 0 {
            semsg!(
                c"E1502: Lua failed to grow stack to %i",
                lua_gettop(lstate) + 3,
            );
            ret.table_type = ObjectType::ObjectTypeNil;
            return ret;
        }

        lua_pushnil(lstate);
        while lua_next(lstate, -2) != 0 {
            match lua_type(lstate, -2) {
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -2, &mut len);
                    if len > 0 {
                        let slice = std::slice::from_raw_parts((s).cast::<u8>(), len);
                        if slice.contains(&0) {
                            ret.has_string_with_nul = true;
                        }
                    }
                    ret.string_keys_num += 1;
                }
                LUA_TNUMBER => {
                    let n = lua_tonumber(lstate, -2);
                    if n > (usize::MAX as lua_Number)
                        || n <= 0.0
                        || ((n as usize) as lua_Number) != n
                    {
                        other_keys_num += 1;
                    } else {
                        let idx = n as usize;
                        if idx > ret.maxidx {
                            ret.maxidx = idx;
                        }
                    }
                }
                LUA_TBOOLEAN => {
                    let b = lua_toboolean(lstate, -2) != 0;
                    if b == TYPE_IDX_VALUE {
                        if lua_type(lstate, -1) == LUA_TNUMBER {
                            let n = lua_tonumber(lstate, -1);
                            if n == (ObjectType::ObjectTypeFloat as i32 as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::ObjectTypeFloat;
                            } else if n == (ObjectType::ObjectTypeArray as i32 as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::ObjectTypeArray;
                            } else if n == (ObjectType::ObjectTypeDict as i32 as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::ObjectTypeDict;
                            } else {
                                other_keys_num += 1;
                            }
                        } else {
                            other_keys_num += 1;
                        }
                    } else {
                        has_val_key = true;
                        val_type = lua_type(lstate, -1);
                        if val_type == LUA_TNUMBER {
                            ret.val = lua_tonumber(lstate, -1);
                        }
                    }
                }
                _ => {
                    other_keys_num += 1;
                }
            }
            tsize += 1;
            lua_pop(lstate, 1);
        }

        if ret.has_type_key {
            assert!(tsize > 0);
            if ret.table_type == ObjectType::ObjectTypeFloat
                && (!has_val_key || val_type != LUA_TNUMBER)
            {
                ret.table_type = ObjectType::ObjectTypeNil;
            } else if ret.table_type == ObjectType::ObjectTypeArray {
                let expected = tsize
                    - usize::from(ret.has_type_key)
                    - other_keys_num
                    - usize::from(has_val_key)
                    - ret.string_keys_num;
                if ret.maxidx != 0 && ret.maxidx != expected {
                    ret.maxidx = 0;
                    loop {
                        lua_rawgeti(lstate, -1, (ret.maxidx as i32) + 1);
                        if lua_type(lstate, -1) == LUA_TNIL {
                            lua_pop(lstate, 1);
                            break;
                        }
                        lua_pop(lstate, 1);
                        ret.maxidx += 1;
                    }
                }
            }
        } else if tsize == 0
            || (tsize <= ret.maxidx && other_keys_num == 0 && ret.string_keys_num == 0)
        {
            ret.table_type = ObjectType::ObjectTypeArray;
            if tsize == 0 && lua_getmetatable(lstate, -1) != 0 {
                nlua_pushref(
                    lstate,
                    crate::globals::nlua_global_refs_mut().empty_dict_ref,
                );
                if lua_rawequal(lstate, -2, -1) != 0 {
                    ret.table_type = ObjectType::ObjectTypeDict;
                }
                lua_pop(lstate, 2);
            }
        } else if ret.string_keys_num == tsize {
            ret.table_type = ObjectType::ObjectTypeDict;
        } else {
            ret.table_type = ObjectType::ObjectTypeNil;
        }
        ret
    }
}

/// Convert Lua object to Vimscript `TypVal` (`nlua_pop_typval`).
pub fn nlua_pop_typval(lstate: *mut lua_State, ret_tv: &mut TypVal) -> bool {
    unsafe {
        let mut ret = true;
        let initial_size = lua_gettop(lstate);
        let mut stack: Vec<TVPopStackItem> = Vec::with_capacity(2);
        stack.push(TVPopStackItem {
            tv: crate::globals::GraphPtr::from(&mut *ret_tv),
            list_len: 0,
            container: false,
            special: false,
            idx: 0,
        });

        while ret && !stack.is_empty() {
            if lua_checkstack(lstate, 3) == 0 {
                semsg!(
                    c"E1502: Lua failed to grow stack to %i",
                    lua_gettop(lstate) + 3,
                );
                ret = false;
                break;
            }
            let mut cur = stack.pop().unwrap();
            if cur.container {
                if cur.special || cur.tv_mut().v_type == VarType::VarDict {
                    let mut next_key_found = false;
                    while lua_next(lstate, -2) != 0 {
                        if lua_type(lstate, -2) == LUA_TSTRING {
                            next_key_found = true;
                            break;
                        }
                        lua_pop(lstate, 1);
                    }
                    if next_key_found {
                        let mut len = 0usize;
                        let s = lua_tolstring(lstate, -2, &mut len);
                        let slice = if s.is_null() || len == 0 {
                            &[]
                        } else {
                            std::slice::from_raw_parts(s.cast::<u8>(), len)
                        };
                        if cur.special {
                            let kv_pair = tv_list_alloc(2);
                            let s_tv = decode_string(slice, true);
                            tv_list_append_owned_tv(&mut *kv_pair, s_tv);
                            tv_list_append_owned_tv(
                                &mut *kv_pair,
                                TypVal {
                                    v_type: VarType::VarUnknown,
                                    v_lock: VAR_UNLOCKED,
                                    vval: TypvalVval::Number(0),
                                },
                            );
                            stack.push(cur);
                            tv_list_append_list(
                                cur.tv_mut().vval.v_list().borrow_mut(),
                                Some(kv_pair),
                            );
                            let last = tv_list_last(Some(&*kv_pair)).unwrap();
                            cur = TVPopStackItem {
                                tv: crate::globals::GraphPtr::from(&mut last.li_tv),
                                list_len: 0,
                                container: false,
                                special: false,
                                idx: 0,
                            };
                        } else {
                            let di = tv_dict_item_alloc_len(std::slice::from_raw_parts(
                                (s).cast::<u8>(),
                                len,
                            ));
                            if !tv_dict_add(cur.tv_mut().vval.v_dict().borrow_mut(), di) {
                                std::process::abort();
                            }
                            stack.push(cur);
                            cur = TVPopStackItem {
                                tv: crate::globals::GraphPtr::from(&mut di.di_tv),
                                list_len: 0,
                                container: false,
                                special: false,
                                idx: 0,
                            };
                        }
                    } else {
                        lua_pop(lstate, 1);
                        continue;
                    }
                } else {
                    let list = cur.tv_mut().vval.v_list();
                    if (tv_list_len(list.opt()) as usize) == cur.list_len {
                        lua_pop(lstate, 1);
                        continue;
                    }
                    lua_rawgeti(lstate, -1, tv_list_len(list.as_ref()) + 1);
                    tv_list_append_owned_tv(
                        list.borrow_mut(),
                        TypVal {
                            v_type: VarType::VarUnknown,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval::Number(0),
                        },
                    );
                    stack.push(cur);
                    let last = tv_list_last(list.as_ref()).unwrap();
                    cur = TVPopStackItem {
                        tv: crate::globals::GraphPtr::from(&mut last.li_tv),
                        list_len: 0,
                        container: false,
                        special: false,
                        idx: 0,
                    };
                }
            }

            *cur.tv = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };

            match lua_type(lstate, -1) {
                LUA_TNIL => {
                    cur.tv_mut().v_type = VarType::VarSpecial;
                    cur.tv_mut().vval = crate::eval::typval::TypvalVval::Number(K_SPECIAL_VAR_NULL);
                }
                LUA_TBOOLEAN => {
                    cur.tv_mut().v_type = VarType::VarBool;
                    cur.tv_mut().vval = crate::eval::typval::TypvalVval::Number(
                        if lua_toboolean(lstate, -1) != 0 {
                            K_BOOL_VAR_TRUE
                        } else {
                            K_BOOL_VAR_FALSE
                        },
                    );
                }
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -1, &mut len);
                    let slice = if s.is_null() || len == 0 {
                        &[]
                    } else {
                        std::slice::from_raw_parts(s.cast::<u8>(), len)
                    };
                    *cur.tv = decode_string(slice, false);
                }
                LUA_TNUMBER => {
                    let n = lua_tonumber(lstate, -1);
                    if n > (i64::MAX as lua_Number)
                        || n < (i64::MIN as lua_Number)
                        || ((n as i64) as lua_Number) != n
                    {
                        cur.tv_mut().v_type = VarType::VarFloat;
                        cur.tv_mut().vval = crate::eval::typval::TypvalVval::Float(n);
                    } else {
                        cur.tv_mut().v_type = VarType::VarNumber;
                        cur.tv_mut().vval = crate::eval::typval::TypvalVval::Number(n as i64);
                    }
                }
                LUA_TTABLE => {
                    let mut table_ref = LUA_NOREF;
                    if lua_getmetatable(lstate, -1) != 0 {
                        lua_pop(lstate, 1);
                        table_ref = nlua_ref_global(lstate, -1);
                    }

                    let table_props = nlua_traverse_table(lstate);

                    let mut found_self_ref = false;
                    for item in &stack {
                        if item.container && lua_rawequal(lstate, -1, item.idx) != 0 {
                            tv_copy(&item.tv, &mut cur.tv);
                            cur.container = false;
                            found_self_ref = true;
                            break;
                        }
                    }

                    if !found_self_ref {
                        match table_props.table_type {
                            ObjectType::ObjectTypeArray => {
                                let l = tv_list_alloc(table_props.maxidx as isize);
                                l.lua_table_ref = table_ref;
                                tv_list_ref(Some(&mut *l));
                                cur.tv_mut().v_type = VarType::VarList;
                                cur.tv_mut().vval = crate::eval::typval::TypvalVval::List(
                                    crate::globals::GraphPtr::new(l),
                                );
                                cur.list_len = table_props.maxidx;
                                if table_props.maxidx != 0 {
                                    cur.container = true;
                                    cur.idx = lua_gettop(lstate);
                                    stack.push(cur);
                                }
                            }
                            ObjectType::ObjectTypeDict => {
                                if table_props.string_keys_num == 0 {
                                    let d = tv_dict_alloc();
                                    d.dv_refcount += 1;
                                    d.lua_table_ref = table_ref;
                                    cur.tv_mut().v_type = VarType::VarDict;
                                    cur.tv_mut().vval = crate::eval::typval::TypvalVval::Dict(
                                        crate::globals::GraphPtr::new(d),
                                    );
                                } else {
                                    cur.special = table_props.has_string_with_nul;
                                    if table_props.has_string_with_nul {
                                        decode_create_map_special_dict(
                                            &mut cur.tv,
                                            table_props.string_keys_num as isize,
                                        );
                                        let val_di = tv_dict_find(
                                            (cur.tv_mut().vval.v_dict()).as_ref(),
                                            c"_VAL",
                                            4,
                                        )
                                        .unwrap();
                                        cur.tv = crate::globals::GraphPtr::from(&mut val_di.di_tv);
                                        cur.tv_mut().vval.v_list().lua_table_ref = table_ref;
                                        cur.list_len = table_props.string_keys_num;
                                    } else {
                                        let d = tv_dict_alloc();
                                        d.dv_refcount += 1;
                                        d.lua_table_ref = table_ref;
                                        cur.tv_mut().v_type = VarType::VarDict;
                                        cur.tv_mut().vval = crate::eval::typval::TypvalVval::Dict(
                                            crate::globals::GraphPtr::new(d),
                                        );
                                    }
                                    cur.container = true;
                                    cur.idx = lua_gettop(lstate);
                                    stack.push(cur);
                                    lua_pushnil(lstate);
                                }
                            }
                            ObjectType::ObjectTypeFloat => {
                                cur.tv_mut().v_type = VarType::VarFloat;
                                cur.tv_mut().vval =
                                    crate::eval::typval::TypvalVval::Float(table_props.val);
                            }
                            ObjectType::ObjectTypeNil => {
                                emsg(
                                    c"E5100: Cannot convert given Lua table: table should contain either only integer keys or only string keys",
                                );
                                ret = false;
                            }
                            _ => std::process::abort(),
                        }
                    }
                }
                LUA_TFUNCTION => {
                    let func = nlua_ref_global(lstate, -1);
                    let name = register_luafunc(func);
                    crate::eval::typval::tv_set_string(&mut cur.tv, Some(name));
                    cur.tv_mut().v_type = VarType::VarFunc;
                }
                LUA_TUSERDATA => {
                    nlua_pushref(lstate, crate::globals::nlua_global_refs_mut().nil_ref);
                    let is_nil = lua_rawequal(lstate, -2, -1) != 0;
                    lua_pop(lstate, 1);
                    if is_nil {
                        cur.tv_mut().v_type = VarType::VarSpecial;
                        cur.tv_mut().vval =
                            crate::eval::typval::TypvalVval::Number(K_SPECIAL_VAR_NULL);
                    } else {
                        emsg(c"E5101: Cannot convert given Lua type");
                        ret = false;
                    }
                }
                _ => {
                    emsg(c"E5101: Cannot convert given Lua type");
                    ret = false;
                }
            }
            if !cur.container {
                lua_pop(lstate, 1);
            }
        }

        if !ret {
            tv_clear(&mut *ret_tv);
            *ret_tv = TypVal {
                v_type: VarType::VarNumber,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            lua_pop(lstate, lua_gettop(lstate) - initial_size + 1);
        }
        assert_eq!(lua_gettop(lstate), initial_size - 1);
        ret
    }
}

#[inline]
fn lua_conv_nil(lstate: *mut lua_State) {
    if luaconv_st().typval_conv_special {
        unsafe { lua_pushnil(lstate) };
    } else {
        nlua_pushref(lstate, crate::globals::nlua_global_refs_mut().nil_ref);
    }
}

#[inline]
fn lua_conv_empty_dict(lstate: *mut lua_State) {
    if luaconv_st().typval_conv_special {
        nlua_create_typed_table(lstate, 0, 0, ObjectType::ObjectTypeDict);
    } else {
        unsafe { lua_createtable(lstate, 0, 0) };
        nlua_pushref(
            lstate,
            crate::globals::nlua_global_refs_mut().empty_dict_ref,
        );
        unsafe { lua_setmetatable(lstate, -2) };
    }
}

#[inline]
fn lua_conv_list_start(lstate: *mut lua_State, len: i32) {
    unsafe {
        lua_createtable(lstate, len, 0);
        lua_pushnumber(lstate, 1.0);
    }
}

#[inline]
fn lua_conv_dict_start(lstate: *mut lua_State, len: usize) {
    unsafe {
        lua_createtable(lstate, 0, len as i32);
    }
}

fn lua_check_self_reference(
    lstate: *mut lua_State,
    container: MPContainerRef,
    val_copy_id: &mut i32,
    mpstack: &[MPConvStackVal],
    copy_id: i32,
) -> bool {
    if *val_copy_id != copy_id {
        *val_copy_id = copy_id;
        return false;
    }
    let mut backref = mpstack.len();
    while backref > 0 {
        let mpval = mpstack[backref - 1];
        let matched = match (container, mpval.data) {
            (MPContainerRef::Dict(d1), MPConvStackData::Dict { dict: d2, .. }) => d1 == d2,
            (MPContainerRef::List(l1), MPConvStackData::List { list: l2, .. }) => l1 == l2,
            (MPContainerRef::Pairs(l1), MPConvStackData::Pairs { list: l2, .. }) => l1 == l2,
            _ => false,
        };
        if matched {
            let stack_offset = -(((mpstack.len() - backref + 1) * 2) as i32);
            unsafe { lua_pushvalue(lstate, stack_offset) };
            break;
        }
        backref -= 1;
    }
    true
}

fn lua_convert_one_value(
    lstate: *mut lua_State,
    mpstack: &mut Vec<MPConvStackVal>,
    tv: &mut TypVal,
    copy_id: i32,
) -> bool {
    let tv_ptr = crate::globals::GraphPtr::<TypVal>::from_ref(&*tv);
    unsafe {
        if lua_checkstack(lstate, 3) == 0 {
            semsg!(
                c"E5102: Lua failed to grow stack to %i",
                lua_gettop(lstate) + 3,
            );
            return false;
        }
        let tv_ref = tv;
        match tv_ref.v_type {
            VarType::VarString => {
                let s_bytes = tv_ref.vval.v_string_bytes();
                lua_pushlstring(
                    lstate,
                    tv_ref.vval.v_string_cstr_or_empty().as_ptr(),
                    s_bytes.len(),
                );
            }
            VarType::VarNumber => {
                lua_pushnumber(lstate, tv_ref.vval.v_number() as lua_Number);
            }
            VarType::VarFloat => {
                lua_pushnumber(lstate, tv_ref.vval.v_float());
            }
            VarType::VarBlob => {
                let blob = tv_ref.vval.v_blob();
                let len = tv_blob_len(blob.opt()) as usize;
                let data = if !blob.is_null() && !blob.bv_ga.is_empty() {
                    blob.bv_ga.as_ptr().cast::<c_char>()
                } else {
                    c"".as_ptr()
                };
                lua_pushlstring(lstate, data, len);
            }
            VarType::VarFunc => {
                if let Some(fun) = tv_ref.vval.v_string_cstr() {
                    if let Some(fp) = find_func(Some(fun))
                        && (fp.uf_flags & FC_LUAREF) != 0
                    {
                        nlua_pushref(lstate, fp.uf_luaref);
                    } else {
                        lua_conv_nil(lstate);
                    }
                } else {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VarPartial => {
                let pt = tv_ref.vval.v_partial();
                if !pt.is_null() {
                    let fun = partial_name(pt.opt());
                    if let Some(fp) = find_func(Some(fun))
                        && (fp.uf_flags & FC_LUAREF) != 0
                    {
                        nlua_pushref(lstate, fp.uf_luaref);
                    } else {
                        lua_conv_nil(lstate);
                    }
                } else {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VarList => {
                let mut list = tv_ref.vval.v_list();
                if list.is_null() || tv_list_len(list.opt()) == 0 {
                    lua_createtable(lstate, 0, 0);
                } else {
                    let saved_copy_id = list.lv_copy_id;
                    if lua_check_self_reference(
                        lstate,
                        MPContainerRef::List(list),
                        &mut list.lv_copy_id,
                        mpstack,
                        copy_id,
                    ) {
                        return true;
                    }
                    lua_conv_list_start(lstate, tv_list_len(list.as_ref()));
                    mpstack.push(MPConvStackVal {
                        tv: tv_ptr,
                        saved_copy_id,
                        data: MPConvStackData::List {
                            list,
                            li: tv_list_first(list.as_ref()).into(),
                        },
                    });
                }
            }
            VarType::VarBool => {
                lua_pushboolean(
                    lstate,
                    i32::from(tv_ref.vval.v_bool() == K_BOOL_VAR_TRUE as i32),
                );
            }
            VarType::VarSpecial => {
                if tv_ref.vval.v_special() == K_SPECIAL_VAR_NULL as i32 {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VarDict => {
                let mut dict = tv_ref.vval.v_dict();
                if dict.is_null() || dict.dv_hashtab.ht_used == 0 {
                    lua_conv_empty_dict(lstate);
                    return true;
                }
                let mut handled_special = false;
                if dict.dv_hashtab.ht_used == 2
                    && let Some(type_di) = tv_dict_find(dict.as_ref(), c"_TYPE", 5)
                    && type_di.di_tv.v_type == VarType::VarList
                    && let Some(val_di) = tv_dict_find(dict.as_ref(), c"_VAL", 4)
                {
                    let mut mp_idx = NUM_MSGPACK_TYPES;
                    for i in 0..NUM_MSGPACK_TYPES {
                        if eval_msgpack_type_list(i)
                            .is_some_and(|tl| type_di.di_tv.vval.v_list() == tl)
                        {
                            mp_idx = i;
                            break;
                        }
                    }
                    if mp_idx < NUM_MSGPACK_TYPES {
                        match mp_idx {
                            K_MP_NIL => {
                                lua_conv_nil(lstate);
                                handled_special = true;
                            }
                            K_MP_BOOLEAN => {
                                if val_di.di_tv.v_type == VarType::VarNumber {
                                    lua_pushboolean(
                                        lstate,
                                        i32::from(val_di.di_tv.vval.v_number() != 0),
                                    );
                                    handled_special = true;
                                }
                            }
                            K_MP_INTEGER => {
                                if val_di.di_tv.v_type == VarType::VarList {
                                    let val_list = val_di.di_tv.vval.v_list();
                                    if tv_list_len(val_list.opt()) == 4 {
                                        let sign_li = tv_list_first(val_list.opt()).unwrap();
                                        let highest_li = sign_li.li_next.borrow_mut();
                                        let high_li = highest_li.li_next.borrow_mut();
                                        let low_li = tv_list_last(val_list.opt()).unwrap();
                                        if sign_li.li_tv.v_type == VarType::VarNumber
                                            && sign_li.li_tv.vval.v_number() != 0
                                            && highest_li.li_tv.v_type == VarType::VarNumber
                                            && highest_li.li_tv.vval.v_number() >= 0
                                            && high_li.li_tv.v_type == VarType::VarNumber
                                            && high_li.li_tv.vval.v_number() >= 0
                                            && low_li.li_tv.v_type == VarType::VarNumber
                                            && low_li.li_tv.vval.v_number() >= 0
                                        {
                                            let sign = sign_li.li_tv.vval.v_number();
                                            let highest = highest_li.li_tv.vval.v_number() as u64;
                                            let high = high_li.li_tv.vval.v_number() as u64;
                                            let low = low_li.li_tv.vval.v_number() as u64;
                                            let number = (highest << 62) | (high << 31) | low;
                                            if sign > 0 {
                                                lua_pushnumber(lstate, number as lua_Number);
                                            } else {
                                                lua_pushnumber(
                                                    lstate,
                                                    (number.wrapping_neg() as i64) as lua_Number,
                                                );
                                            }
                                            handled_special = true;
                                        }
                                    }
                                }
                            }
                            K_MP_FLOAT => {
                                if val_di.di_tv.v_type == VarType::VarFloat {
                                    lua_pushnumber(lstate, val_di.di_tv.vval.v_float());
                                    handled_special = true;
                                }
                            }
                            K_MP_STRING => {
                                if val_di.di_tv.v_type == VarType::VarList
                                    && let Some(buf) =
                                        encode_vim_list_to_buf(val_di.di_tv.vval.v_list().opt())
                                {
                                    lua_pushlstring(
                                        lstate,
                                        buf.as_ptr().cast::<c_char>(),
                                        buf.len(),
                                    );
                                    handled_special = true;
                                }
                            }
                            K_MP_ARRAY => {
                                if val_di.di_tv.v_type == VarType::VarList {
                                    let mut val_list = val_di.di_tv.vval.v_list();
                                    let saved_copy_id = val_list.lv_copy_id;
                                    if lua_check_self_reference(
                                        lstate,
                                        MPContainerRef::List(val_list),
                                        &mut val_list.lv_copy_id,
                                        mpstack,
                                        copy_id,
                                    ) {
                                        return true;
                                    }
                                    lua_conv_list_start(lstate, tv_list_len(val_list.as_ref()));
                                    mpstack.push(MPConvStackVal {
                                        tv: tv_ptr,
                                        saved_copy_id,
                                        data: MPConvStackData::List {
                                            list: val_list,
                                            li: tv_list_first(val_list.as_ref()).into(),
                                        },
                                    });
                                    handled_special = true;
                                }
                            }
                            K_MP_MAP => {
                                if val_di.di_tv.v_type == VarType::VarList {
                                    let mut val_list = val_di.di_tv.vval.v_list();
                                    if val_list.is_null() || tv_list_len(val_list.opt()) == 0 {
                                        lua_conv_empty_dict(lstate);
                                        handled_special = true;
                                    } else {
                                        let mut all_pairs = true;
                                        let mut li = crate::globals::GraphPtr::from(tv_list_first(
                                            val_list.as_ref(),
                                        ));
                                        while !li.is_null() {
                                            if li.li_tv.v_type != VarType::VarList
                                                || tv_list_len((li.li_tv.vval.v_list()).as_ref())
                                                    != 2
                                            {
                                                all_pairs = false;
                                                break;
                                            }
                                            li = li.li_next;
                                        }
                                        if all_pairs {
                                            let saved_copy_id = val_list.lv_copy_id;
                                            if lua_check_self_reference(
                                                lstate,
                                                MPContainerRef::Pairs(val_list),
                                                &mut val_list.lv_copy_id,
                                                mpstack,
                                                copy_id,
                                            ) {
                                                return true;
                                            }
                                            lua_conv_dict_start(
                                                lstate,
                                                tv_list_len(val_list.as_ref()) as usize,
                                            );
                                            mpstack.push(MPConvStackVal {
                                                tv: tv_ptr,
                                                saved_copy_id,
                                                data: MPConvStackData::Pairs {
                                                    list: val_list,
                                                    li: tv_list_first(val_list.as_ref()).into(),
                                                },
                                            });
                                            handled_special = true;
                                        }
                                    }
                                }
                            }
                            K_MP_EXT if val_di.di_tv.v_type == VarType::VarList => {
                                let val_list = val_di.di_tv.vval.v_list();
                                if tv_list_len(val_list.opt()) == 2 {
                                    let first = tv_list_first(val_list.opt()).unwrap();
                                    let last = tv_list_last(val_list.opt()).unwrap();
                                    if first.li_tv.v_type == VarType::VarNumber
                                        && (i8::MIN as i64..=i8::MAX as i64)
                                            .contains(&first.li_tv.vval.v_number())
                                        && last.li_tv.v_type == VarType::VarList
                                        && encode_vim_list_to_buf(last.li_tv.vval.v_list().opt())
                                            .is_some()
                                    {
                                        lua_conv_nil(lstate);
                                        handled_special = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if !handled_special {
                    let saved_copy_id = dict.dv_copy_id;
                    if lua_check_self_reference(
                        lstate,
                        MPContainerRef::Dict(dict),
                        &mut dict.dv_copy_id,
                        mpstack,
                        copy_id,
                    ) {
                        return true;
                    }
                    lua_conv_dict_start(lstate, dict.dv_hashtab.ht_used);
                    mpstack.push(MPConvStackVal {
                        tv: tv_ptr,
                        saved_copy_id,
                        data: MPConvStackData::Dict {
                            dict,
                            hi: dict.dv_hashtab.array_ptr().into(),
                            todo: dict.dv_hashtab.ht_used,
                        },
                    });
                }
            }
            VarType::VarUnknown => {
                internal_error(c"_typval_encode_lua_convert_one_value()");
                return false;
            }
        }
        true
    }
}

fn encode_vim_to_lua(lstate: *mut lua_State, top_tv: &mut TypVal) -> bool {
    unsafe {
        let copy_id = get_copy_id();
        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);

        if !lua_convert_one_value(lstate, &mut mpstack, top_tv, copy_id) {
            return false;
        }

        while !mpstack.is_empty() {
            let cur_idx = mpstack.len() - 1;
            let saved_copy_id = mpstack[cur_idx].saved_copy_id;
            let next_tv = crate::globals::GraphPtr::<TypVal>::of(match mpstack[cur_idx].data {
                MPConvStackData::Dict {
                    mut dict,
                    ref mut hi,
                    ref mut todo,
                    ..
                } => {
                    if *todo == 0 {
                        mpstack.pop();
                        dict.dv_copy_id = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if *todo != dict.dv_hashtab.ht_used {
                        lua_rawset(lstate, -3);
                    }
                    while hashitem_empty(hi.borrow()) {
                        *hi = (*hi).wrapping_add(1);
                    }
                    let di = crate::eval::typval::tv_dict_hi2di(hi.borrow());
                    *todo -= 1;
                    *hi = (*hi).wrapping_add(1);
                    let key_bytes = di.key_bytes();
                    lua_pushlstring(lstate, key_bytes.as_ptr().cast(), key_bytes.len());
                    &mut di.di_tv
                }
                MPConvStackData::List {
                    mut list,
                    ref mut li,
                } => {
                    let cur_li = *li;
                    if cur_li.is_null() {
                        mpstack.pop();
                        list.lv_copy_id = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if cur_li != crate::globals::GraphPtr::from(tv_list_first(list.opt())) {
                        let idx = lua_tonumber(lstate, -2);
                        lua_rawset(lstate, -3);
                        lua_pushnumber(lstate, idx + 1.0);
                    }
                    let cur_li = cur_li.borrow_mut();
                    *li = cur_li.li_next;
                    &mut cur_li.li_tv
                }
                MPConvStackData::Pairs { mut list, li } => {
                    if li.is_null() {
                        mpstack.pop();
                        list.lv_copy_id = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if li != crate::globals::GraphPtr::from(tv_list_first(list.opt())) {
                        lua_rawset(lstate, -3);
                    }
                    let kv_pair = li.li_tv.vval.v_list();
                    let first_item = tv_list_first(kv_pair.opt()).unwrap();
                    if !lua_convert_one_value(lstate, &mut mpstack, &mut first_item.li_tv, copy_id)
                    {
                        return false;
                    }
                    let last_item = tv_list_last(kv_pair.as_ref()).unwrap();
                    let next_li = li.li_next;
                    if let MPConvStackData::Pairs {
                        li: ref mut li_ref, ..
                    } = mpstack[cur_idx].data
                    {
                        *li_ref = next_li;
                    }
                    &mut last_item.li_tv
                }
                MPConvStackData::Partial { .. } | MPConvStackData::PartialList { .. } => {
                    unreachable!()
                }
            })
            .as_ptr();

            if !lua_convert_one_value(lstate, &mut mpstack, &mut *next_tv, copy_id) {
                return false;
            }
        }
        true
    }
}

/// Convert Vimscript `TypVal` to Lua value (`nlua_push_typval`).
pub fn nlua_push_typval(lstate: *mut lua_State, tv: &mut TypVal, flags: i32) -> bool {
    unsafe {
        luaconv_st().typval_conv_special = (flags & K_NLUA_PUSH_SPECIAL) != 0;
        let initial_size = lua_gettop(lstate);

        if !encode_vim_to_lua(lstate, tv) {
            lua_settop(lstate, initial_size);
            lua_pushnil(lstate);
            return false;
        }
        assert_eq!(lua_gettop(lstate), initial_size + 1);
        true
    }
}

/// Convert given `String` to Lua string (`nlua_push_String`).
pub fn nlua_push_string(lstate: *mut lua_State, s: NvimString, _flags: i32) {
    unsafe {
        let data = if s.size != 0 {
            s.data.as_ptr().cast::<std::ffi::c_char>().cast_const()
        } else {
            c"".as_ptr()
        };
        lua_pushlstring(lstate, data, s.size);
    }
}

/// Convert given `Integer` to Lua number (`nlua_push_Integer`).
pub fn nlua_push_integer(lstate: *mut lua_State, n: Integer, _flags: i32) {
    unsafe {
        lua_pushnumber(lstate, n as lua_Number);
    }
}

/// Convert given `Float` to Lua number or typed table (`nlua_push_Float`).
pub fn nlua_push_float(lstate: *mut lua_State, f: Float, flags: i32) {
    unsafe {
        if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
            nlua_create_typed_table(lstate, 0, 1, ObjectType::ObjectTypeFloat);
            nlua_push_val_idx(lstate);
            lua_pushnumber(lstate, f);
            lua_rawset(lstate, -3);
        } else {
            lua_pushnumber(lstate, f);
        }
    }
}

/// Convert given `Boolean` to Lua boolean (`nlua_push_Boolean`).
pub fn nlua_push_boolean(lstate: *mut lua_State, b: Boolean, _flags: i32) {
    unsafe {
        lua_pushboolean(lstate, i32::from(b));
    }
}

/// Convert given `Dict` to Lua table (`nlua_push_Dict`).
pub fn nlua_push_dict(lstate: *mut lua_State, mut dict: Dict, flags: i32) {
    unsafe { lua_createtable(lstate, 0, dict.size as i32) };
    if dict.size == 0 {
        nlua_pushref(
            lstate,
            crate::globals::nlua_global_refs_mut().empty_dict_ref,
        );
        unsafe { lua_setmetatable(lstate, -2) };
    }
    for item in dict.as_mut_slice() {
        nlua_push_string(lstate, item.key, flags);
        nlua_push_object(lstate, &mut item.value, flags);
        unsafe { lua_rawset(lstate, -3) };
    }
}

/// Convert given `Array` to Lua table (`nlua_push_Array`).
pub fn nlua_push_array(lstate: *mut lua_State, array: Array, flags: i32) {
    unsafe {
        lua_createtable(lstate, array.size as i32, 0);
        for i in 0..array.size {
            nlua_push_object(lstate, array.items.wrapping_add(i).borrow_mut(), flags);
            lua_rawseti(lstate, -2, (i as i32) + 1);
        }
    }
}

/// Convert given `ObjHandle` to Lua number (`nlua_push_handle`).
pub fn nlua_push_handle(lstate: *mut lua_State, item: ObjHandle, _flags: i32) {
    unsafe {
        lua_pushnumber(lstate, item as lua_Number);
    }
}

/// Convert given `Object` to Lua value (`nlua_push_Object`).
pub fn nlua_push_object(lstate: *mut lua_State, obj: &mut Object, flags: i32) {
    match obj.obj_type {
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {
            if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
                unsafe { lua_pushnil(lstate) };
            } else {
                nlua_pushref(lstate, crate::globals::nlua_global_refs_mut().nil_ref);
            }
        }
        ObjectType::ObjectTypeLuaRef => {
            nlua_pushref(lstate, obj.data.luaref());
            if (flags & K_NLUA_PUSH_FREE_REFS) != 0 {
                api_free_luaref(obj.data.luaref());
                obj.data = ObjectData::LuaRef(LUA_NOREF);
            }
        }
        ObjectType::ObjectTypeBoolean => nlua_push_boolean(lstate, obj.data.boolean(), flags),
        ObjectType::ObjectTypeInteger => nlua_push_integer(lstate, obj.data.integer(), flags),
        ObjectType::ObjectTypeFloat => nlua_push_float(lstate, obj.data.floating(), flags),
        ObjectType::ObjectTypeString => nlua_push_string(lstate, obj.data.string(), flags),
        ObjectType::ObjectTypeArray => nlua_push_array(lstate, obj.data.array(), flags),
        ObjectType::ObjectTypeDict => nlua_push_dict(lstate, obj.data.dict(), flags),
        ObjectType::ObjectTypeBuffer
        | ObjectType::ObjectTypeWindow
        | ObjectType::ObjectTypeTabpage => {
            nlua_push_handle(lstate, obj.data.integer() as ObjHandle, flags);
        }
    }
}

/// Convert Lua value to `String` (`nlua_pop_String`).
pub fn nlua_pop_string(lstate: *mut lua_State, arena: &mut Arena, err: &mut Error) -> NvimString {
    nlua_pop_string_opt(lstate, Some(arena), err)
}

pub fn nlua_pop_string_opt(
    lstate: *mut lua_State,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> NvimString {
    if unsafe { lua_type(lstate, -1) } != LUA_TSTRING {
        lua_pop(lstate, 1);
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua string",);
        return NvimString::new();
    }
    let mut size = 0usize;
    let data = unsafe { lua_tolstring(lstate, -1, &mut size) };
    assert!(!data.is_null());
    let dup = arena_memdupz_opt(
        arena,
        if size == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }
        },
    );
    lua_pop(lstate, 1);
    NvimString { data: dup, size }
}

/// Convert Lua value to `Integer` (`nlua_pop_Integer`).
pub fn nlua_pop_integer(lstate: *mut lua_State, _arena: &mut Arena, err: &mut Error) -> Integer {
    if unsafe { lua_type(lstate, -1) } != LUA_TNUMBER {
        lua_pop(lstate, 1);
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua number",);
        return 0;
    }
    let n = unsafe { lua_tonumber(lstate, -1) };
    lua_pop(lstate, 1);
    if n > (Integer::MAX as lua_Number)
        || n < (Integer::MIN as lua_Number)
        || ((n as Integer) as lua_Number) != n
    {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Number is not integral",
        );
        return 0;
    }
    n as Integer
}

/// Convert Lua value to `Boolean` (`nlua_pop_Boolean`).
pub fn nlua_pop_boolean(lstate: *mut lua_State, _arena: &mut Arena, _err: &mut Error) -> Boolean {
    let ret = unsafe { lua_toboolean(lstate, -1) } != 0;
    lua_pop(lstate, 1);
    ret
}

/// Convert Lua value to `Boolean` strictly (`nlua_pop_Boolean_strict`).
pub fn nlua_pop_boolean_strict(lstate: *mut lua_State, err: &mut Error) -> Boolean {
    let mut ret = false;
    match unsafe { lua_type(lstate, -1) } {
        LUA_TBOOLEAN => {
            ret = unsafe { lua_toboolean(lstate, -1) } != 0;
        }
        LUA_TNUMBER => {
            ret = unsafe { lua_tonumber(lstate, -1) } != 0.0;
        }
        LUA_TNIL => {
            ret = false;
        }
        _ => {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"not a boolean",);
        }
    }
    lua_pop(lstate, 1);
    ret
}

fn nlua_check_type(
    lstate: *mut lua_State,
    err: &mut Error,
    expected_type: ObjectType,
) -> LuaTableProps {
    if unsafe { lua_type(lstate, -1) } != LUA_TTABLE {
        let name = if expected_type == ObjectType::ObjectTypeFloat {
            c"number"
        } else {
            c"table"
        };
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Expected Lua %s",
            name,
        );
        return LuaTableProps {
            maxidx: 0,
            string_keys_num: 0,
            has_string_with_nul: false,
            table_type: ObjectType::ObjectTypeNil,
            val: 0.0,
            has_type_key: false,
        };
    }
    let mut table_props = nlua_traverse_table(lstate);

    if expected_type == ObjectType::ObjectTypeDict
        && table_props.table_type == ObjectType::ObjectTypeArray
        && table_props.maxidx == 0
        && !table_props.has_type_key
    {
        table_props.table_type = ObjectType::ObjectTypeDict;
    }

    if table_props.table_type != expected_type {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Expected %s-like Lua table",
            api_typename(expected_type),
        );
    }
    table_props
}

/// Convert Lua value or typed table to `Float` (`nlua_pop_Float`).
pub fn nlua_pop_float(lstate: *mut lua_State, _arena: &mut Arena, err: &mut Error) -> Float {
    unsafe {
        if lua_type(lstate, -1) == LUA_TNUMBER {
            let ret = lua_tonumber(lstate, -1);
            lua_pop(lstate, 1);
            return ret;
        }
    }
    let table_props = nlua_check_type(lstate, err, ObjectType::ObjectTypeFloat);
    lua_pop(lstate, 1);
    if table_props.table_type != ObjectType::ObjectTypeFloat {
        return 0.0;
    }
    table_props.val
}

fn nlua_pop_array_unchecked(
    lstate: *mut lua_State,
    table_props: LuaTableProps,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) -> Array {
    let mut ret = arena_array_opt(arena.as_deref_mut(), table_props.maxidx);
    if table_props.maxidx == 0 {
        lua_pop(lstate, 1);
        return ret;
    }
    for i in 1..=table_props.maxidx {
        unsafe { lua_rawgeti(lstate, -1, i as i32) };
        let val = nlua_pop_object_opt(lstate, false, arena.as_deref_mut(), err);
        if error_set(err) {
            lua_pop(lstate, 1);
            if arena.is_none() {
                api_free_array(ret);
            }
            return Array::new();
        }
        assert!(ret.size < ret.capacity);
        *ret.next_slot_mut() = val;
        ret.size += 1;
    }
    lua_pop(lstate, 1);
    ret
}

/// Convert Lua table to `Array` (`nlua_pop_Array`).
pub fn nlua_pop_array(lstate: *mut lua_State, arena: &mut Arena, err: &mut Error) -> Array {
    nlua_pop_array_opt(lstate, Some(arena), err)
}

pub fn nlua_pop_array_opt(
    lstate: *mut lua_State,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Array {
    let table_props = nlua_check_type(lstate, err, ObjectType::ObjectTypeArray);
    if table_props.table_type != ObjectType::ObjectTypeArray {
        return Array::new();
    }
    nlua_pop_array_unchecked(lstate, table_props, arena, err)
}

fn nlua_pop_dict_unchecked(
    lstate: *mut lua_State,
    table_props: LuaTableProps,
    ref_: bool,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) -> Dict {
    let mut ret = arena_dict_opt(arena.as_deref_mut(), table_props.string_keys_num);
    if table_props.string_keys_num == 0 {
        lua_pop(lstate, 1);
        return ret;
    }

    unsafe { lua_pushnil(lstate) };
    let mut i = 0usize;
    while unsafe { lua_next(lstate, -2) } != 0 && i < table_props.string_keys_num {
        if unsafe { lua_type(lstate, -2) } == LUA_TSTRING {
            unsafe { lua_pushvalue(lstate, -2) };
            let key = nlua_pop_string_opt(lstate, arena.as_deref_mut(), err);
            if !error_set(err) {
                let value = nlua_pop_object_opt(lstate, ref_, arena.as_deref_mut(), err);
                assert!(ret.size < ret.capacity);
                *ret.next_slot_mut() = KeyValuePair { key, value };
                ret.size += 1;
            } else {
                lua_pop(lstate, 1);
            }

            if error_set(err) {
                if arena.is_none() {
                    api_free_dict(ret);
                }
                lua_pop(lstate, 2);
                return Dict::new();
            }
            i += 1;
        } else {
            lua_pop(lstate, 1);
        }
    }
    lua_pop(lstate, 1);
    ret
}

/// Convert Lua table to `Dict` (`nlua_pop_Dict`).
pub fn nlua_pop_dict(
    lstate: *mut lua_State,
    ref_: bool,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
    nlua_pop_dict_opt(lstate, ref_, Some(arena), err)
}

pub fn nlua_pop_dict_opt(
    lstate: *mut lua_State,
    ref_: bool,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Dict {
    let table_props = nlua_check_type(lstate, err, ObjectType::ObjectTypeDict);
    if table_props.table_type != ObjectType::ObjectTypeDict {
        lua_pop(lstate, 1);
        return Dict::new();
    }
    nlua_pop_dict_unchecked(lstate, table_props, ref_, arena, err)
}

/// Convert Lua value to `Object` (`nlua_pop_Object`).
pub fn nlua_pop_object(
    lstate: *mut lua_State,
    ref_: bool,
    arena: &mut Arena,
    err: &mut Error,
) -> Object {
    nlua_pop_object_opt(lstate, ref_, Some(arena), err)
}

pub fn nlua_pop_object_opt(
    lstate: *mut lua_State,
    ref_: bool,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    unsafe {
        let mut ret = nil_obj();
        let initial_size = lua_gettop(lstate);
        let mut stack: Vec<ObjPopStackItem> = Vec::with_capacity(2);
        stack.push(ObjPopStackItem {
            obj: crate::globals::GraphPtr::from(&mut ret),
            container: false,
        });

        while !error_set(err) && !stack.is_empty() {
            let mut cur = stack.pop().unwrap();
            if cur.container {
                if lua_checkstack(lstate, 3) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Lua failed to grow stack",
                    );
                    break;
                }
                if cur.obj.obj_type == ObjectType::ObjectTypeDict {
                    if cur.obj.data.dict().size == cur.obj.data.dict().capacity {
                        lua_pop(lstate, 2);
                        continue;
                    }
                    let mut next_key_found = false;
                    while lua_next(lstate, -2) != 0 {
                        if lua_type(lstate, -2) == LUA_TSTRING {
                            next_key_found = true;
                            break;
                        }
                        lua_pop(lstate, 1);
                    }
                    if next_key_found {
                        let mut len = 0usize;
                        let s = lua_tolstring(lstate, -2, &mut len);
                        let idx = cur.obj.data.dict().size;
                        cur.obj.data.dict_mut().size += 1;
                        let item = &mut cur.obj.data.dict_mut()[idx];
                        item.key = cbuf_to_arena_str_opt(
                            arena.as_deref_mut(),
                            std::slice::from_raw_parts((s).cast::<u8>(), len),
                        );
                        let item_val_ptr = &raw mut item.value;
                        stack.push(cur);
                        cur = ObjPopStackItem {
                            obj: crate::globals::GraphPtr::new(item_val_ptr),
                            container: false,
                        };
                    } else {
                        lua_pop(lstate, 1);
                        continue;
                    }
                } else {
                    if cur.obj.data.array().size == cur.obj.data.array().capacity {
                        lua_pop(lstate, 1);
                        continue;
                    }
                    let idx = cur.obj.data.array().size;
                    cur.obj.data.array_mut().size += 1;
                    lua_rawgeti(lstate, -1, (idx as i32) + 1);
                    let item_ptr = &raw mut cur.obj.data.array_mut()[idx];
                    stack.push(cur);
                    cur = ObjPopStackItem {
                        obj: crate::globals::GraphPtr::new(item_ptr),
                        container: false,
                    };
                }
            }

            *cur.obj = nil_obj();
            match lua_type(lstate, -1) {
                LUA_TNIL => {}
                LUA_TBOOLEAN => {
                    *cur.obj = Object {
                        obj_type: ObjectType::ObjectTypeBoolean,
                        data: ObjectData::Boolean(lua_toboolean(lstate, -1) != 0),
                    };
                }
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -1, &mut len);
                    *cur.obj = Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(cbuf_to_arena_str_opt(
                            arena.as_deref_mut(),
                            std::slice::from_raw_parts((s).cast::<u8>(), len),
                        )),
                    };
                }
                LUA_TNUMBER => {
                    let n = lua_tonumber(lstate, -1);
                    if n > (Integer::MAX as lua_Number)
                        || n < (Integer::MIN as lua_Number)
                        || ((n as Integer) as lua_Number) != n
                    {
                        *cur.obj = Object {
                            obj_type: ObjectType::ObjectTypeFloat,
                            data: ObjectData::Floating(n),
                        };
                    } else {
                        *cur.obj = Object {
                            obj_type: ObjectType::ObjectTypeInteger,
                            data: ObjectData::Integer(n as Integer),
                        };
                    }
                }
                LUA_TTABLE => {
                    let table_props = nlua_traverse_table(lstate);
                    match table_props.table_type {
                        ObjectType::ObjectTypeArray => {
                            *cur.obj = Object {
                                obj_type: ObjectType::ObjectTypeArray,
                                data: ObjectData::Array(Array::new()),
                            };
                            if table_props.maxidx != 0 {
                                cur.obj.data = ObjectData::Array(arena_array_opt(
                                    arena.as_deref_mut(),
                                    table_props.maxidx,
                                ));
                                cur.container = true;
                                stack.push(cur);
                            }
                        }
                        ObjectType::ObjectTypeDict => {
                            *cur.obj = Object {
                                obj_type: ObjectType::ObjectTypeDict,
                                data: ObjectData::Dict(Dict::new()),
                            };
                            if table_props.string_keys_num != 0 {
                                cur.obj.data = ObjectData::Dict(arena_dict_opt(
                                    arena.as_deref_mut(),
                                    table_props.string_keys_num,
                                ));
                                cur.container = true;
                                stack.push(cur);
                                lua_pushnil(lstate);
                            }
                        }
                        ObjectType::ObjectTypeFloat => {
                            *cur.obj = Object {
                                obj_type: ObjectType::ObjectTypeFloat,
                                data: ObjectData::Floating(table_props.val),
                            };
                        }
                        ObjectType::ObjectTypeNil => {
                            api_set_error!(
                                err,
                                ErrorType::ErrorTypeValidation,
                                c"Cannot convert given Lua table",
                            );
                        }
                        _ => std::process::abort(),
                    }
                }
                LUA_TFUNCTION => {
                    if ref_ {
                        *cur.obj = Object {
                            obj_type: ObjectType::ObjectTypeLuaRef,
                            data: ObjectData::LuaRef(nlua_ref_global(lstate, -1)),
                        };
                    } else {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeValidation,
                            c"Cannot convert given Lua type",
                        );
                    }
                }
                LUA_TUSERDATA => {
                    nlua_pushref(lstate, crate::globals::nlua_global_refs_mut().nil_ref);
                    let is_nil = lua_rawequal(lstate, -2, -1) != 0;
                    lua_pop(lstate, 1);
                    if is_nil {
                        *cur.obj = nil_obj();
                    } else {
                        api_set_error!(
                            err,
                            ErrorType::ErrorTypeValidation,
                            c"Cannot convert userdata",
                        );
                    }
                }
                _ => {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeValidation,
                        c"Cannot convert given Lua type",
                    );
                }
            }
            if !cur.container {
                lua_pop(lstate, 1);
            }
        }

        if error_set(err) {
            if arena.is_none() {
                api_free_object(ret);
            }
            ret = nil_obj();
            lua_pop(lstate, lua_gettop(lstate) - initial_size + 1);
        }
        assert_eq!(lua_gettop(lstate), initial_size - 1);
        ret
    }
}

/// Convert Lua value to `LuaRef` (`nlua_pop_LuaRef`).
pub fn nlua_pop_lua_ref(lstate: *mut lua_State, _arena: &mut Arena, _err: &mut Error) -> LuaRef {
    let rv = nlua_ref_global(lstate, -1);
    lua_pop(lstate, 1);
    rv
}

/// Convert Lua value to `ObjHandle` (`nlua_pop_handle`).
pub fn nlua_pop_handle(lstate: *mut lua_State, _arena: &mut Arena, err: &mut Error) -> ObjHandle {
    unsafe {
        let ret = if lua_type(lstate, -1) != LUA_TNUMBER {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua number",);
            -1
        } else {
            lua_tonumber(lstate, -1) as ObjHandle
        };
        lua_pop(lstate, 1);
        ret
    }
}

/// Record auxiliary values in `vim` module (`nlua_init_types`).
pub fn nlua_init_types(lstate: *mut lua_State) {
    unsafe {
        lua_pushlstring(lstate, c"type_idx".as_ptr(), 8);
        nlua_push_type_idx(lstate);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"val_idx".as_ptr(), 7);
        nlua_push_val_idx(lstate);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"types".as_ptr(), 5);
        lua_createtable(lstate, 0, 3);

        lua_pushlstring(lstate, c"float".as_ptr(), 5);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeFloat as i32) as lua_Number);
        lua_rawset(lstate, -3);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeFloat as i32) as lua_Number);
        lua_pushlstring(lstate, c"float".as_ptr(), 5);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"array".as_ptr(), 5);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeArray as i32) as lua_Number);
        lua_rawset(lstate, -3);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeArray as i32) as lua_Number);
        lua_pushlstring(lstate, c"array".as_ptr(), 5);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"dictionary".as_ptr(), 10);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeDict as i32) as lua_Number);
        lua_rawset(lstate, -3);
        lua_pushnumber(lstate, (ObjectType::ObjectTypeDict as i32) as lua_Number);
        lua_pushlstring(lstate, c"dictionary".as_ptr(), 10);
        lua_rawset(lstate, -3);

        lua_rawset(lstate, -3);
    }
}

/// Lua-specific variant of `api_dict_to_keydict` (`nlua_pop_keydict`).
pub fn nlua_pop_keydict<T>(
    lstate: *mut lua_State,
    retval: &mut T,
    hashy: FieldHashfn,
    err_opt: &mut Option<&'static std::ffi::CStr>,
    arena: &mut Arena,
    err: &mut Error,
) {
    nlua_pop_keydict_opt(lstate, retval, hashy, err_opt, Some(arena), err);
}

pub fn nlua_pop_keydict_opt<T>(
    lstate: *mut lua_State,
    retval: &mut T,
    hashy: FieldHashfn,
    err_opt: &mut Option<&'static std::ffi::CStr>,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) {
    let retval = std::ptr::from_mut(retval).cast::<OptKeySet>();
    unsafe {
        if lua_type(lstate, -1) != LUA_TTABLE {
            api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua table",);
            lua_pop(lstate, 1);
            return;
        }

        let mut dummy_arena = Arena::default();
        let hashy_fn = hashy.unwrap();
        lua_pushnil(lstate);
        while lua_next(lstate, -2) != 0 {
            let mut len = 0usize;
            let s = lua_tolstring(lstate, -2, &mut len);
            let key_bytes = if s.is_null() || len == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts((s).cast::<u8>(), len)
            };
            let Some(field) = hashy_fn(key_bytes) else {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"invalid key: %.*s",
                    len as i32,
                    (!s.is_null()).then(|| CStr::from_ptr(s)),
                );
                lua_pop(lstate, 3);
                return;
            };
            if field.opt_index >= 0 {
                (*retval).is_set_ |= 1u64 << (field.opt_index as u32);
            }
            let mem = retval.cast::<u8>().wrapping_add(field.ptr_off);

            if field.key_type == (ObjectType::ObjectTypeNil as i32) {
                *mem.cast::<Object>() =
                    nlua_pop_object_opt(lstate, true, arena.as_deref_mut(), err);
            } else if field.key_type == (ObjectType::ObjectTypeInteger as i32) {
                if field.is_hlgroup && lua_type(lstate, -1) == LUA_TSTRING {
                    let mut name_len = 0usize;
                    let name = lua_tolstring(lstate, -1, &mut name_len);
                    lua_pop(lstate, 1);
                    *mem.cast::<Integer>() = if name_len > 0 {
                        syn_check_group(std::slice::from_raw_parts((name).cast::<u8>(), name_len))
                            as Integer
                    } else {
                        0
                    };
                } else {
                    *mem.cast::<Integer>() = nlua_pop_integer(lstate, &mut dummy_arena, err);
                }
            } else if field.key_type == (ObjectType::ObjectTypeBoolean as i32) {
                *mem.cast::<Boolean>() = nlua_pop_boolean_strict(lstate, err);
            } else if field.key_type == (ObjectType::ObjectTypeString as i32) {
                *mem.cast::<NvimString>() = nlua_pop_string_opt(lstate, arena.as_deref_mut(), err);
            } else if field.key_type == (ObjectType::ObjectTypeFloat as i32) {
                *mem.cast::<Float>() = nlua_pop_float(lstate, &mut dummy_arena, err);
            } else if field.key_type == (ObjectType::ObjectTypeBuffer as i32)
                || field.key_type == (ObjectType::ObjectTypeWindow as i32)
                || field.key_type == (ObjectType::ObjectTypeTabpage as i32)
            {
                *mem.cast::<ObjHandle>() = nlua_pop_handle(lstate, &mut dummy_arena, err);
            } else if field.key_type == (ObjectType::ObjectTypeArray as i32) {
                *mem.cast::<Array>() = nlua_pop_array_opt(lstate, arena.as_deref_mut(), err);
            } else if field.key_type == (ObjectType::ObjectTypeDict as i32) {
                *mem.cast::<Dict>() = nlua_pop_dict_opt(lstate, false, arena.as_deref_mut(), err);
            } else if field.key_type == (ObjectType::ObjectTypeLuaRef as i32) {
                *mem.cast::<LuaRef>() = nlua_pop_lua_ref(lstate, &mut dummy_arena, err);
            } else {
                std::process::abort();
            }

            if error_set(err) {
                *err_opt = field.str;
                break;
            }
        }
        lua_pop(lstate, 1);
    }
}

/// Push a `KeySet` struct as a Lua table (`nlua_push_keydict`).
pub fn nlua_push_keydict<T>(lstate: *mut lua_State, value: &T, table: &[KeySetLink]) {
    let value = std::ptr::from_ref(value).cast::<OptKeySet>();
    unsafe {
        lua_createtable(lstate, 0, 0);
        for field in table {
            let Some(field_str) = field.str else {
                break;
            };
            let mut is_set = true;
            if field.opt_index >= 0 {
                is_set = ((*value).is_set_ & (1u64 << (field.opt_index as u32))) != 0;
            }
            if !is_set {
                continue;
            }

            let mem = value.cast::<u8>().wrapping_add(field.ptr_off).cast_mut();
            lua_pushstring(lstate, field_str);
            if field.key_type == (ObjectType::ObjectTypeNil as i32) {
                nlua_push_object(lstate, &mut *mem.cast::<Object>(), 0);
            } else if field.key_type == (ObjectType::ObjectTypeInteger as i32) {
                lua_pushinteger(lstate, *mem.cast::<Integer>() as lua_Integer);
            } else if field.key_type == (ObjectType::ObjectTypeBuffer as i32)
                || field.key_type == (ObjectType::ObjectTypeWindow as i32)
                || field.key_type == (ObjectType::ObjectTypeTabpage as i32)
            {
                lua_pushinteger(lstate, *mem.cast::<ObjHandle>() as lua_Integer);
            } else if field.key_type == (ObjectType::ObjectTypeFloat as i32) {
                lua_pushnumber(lstate, *mem.cast::<Float>());
            } else if field.key_type == (ObjectType::ObjectTypeBoolean as i32) {
                lua_pushboolean(lstate, i32::from(*mem.cast::<Boolean>()));
            } else if field.key_type == (ObjectType::ObjectTypeString as i32) {
                nlua_push_string(lstate, *mem.cast::<NvimString>(), 0);
            } else if field.key_type == (ObjectType::ObjectTypeArray as i32) {
                nlua_push_array(lstate, *mem.cast::<Array>(), 0);
            } else if field.key_type == (ObjectType::ObjectTypeDict as i32) {
                nlua_push_dict(lstate, *mem.cast::<Dict>(), 0);
            } else if field.key_type == (ObjectType::ObjectTypeLuaRef as i32) {
                nlua_pushref(lstate, *mem.cast::<LuaRef>());
            } else {
                std::process::abort();
            }

            lua_rawset(lstate, -3);
        }
    }
}

impl TVPopStackItem {
    #[inline]
    fn tv_mut(&self) -> &'static mut TypVal {
        self.tv.borrow_mut()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_link_and_nil_obj_defaults() {
        let obj = nil_obj();
        assert_eq!(obj.obj_type, ObjectType::ObjectTypeNil);
        assert_eq!(obj.data.integer(), 0);
    }
}
