//! Conversions between Lua stack values, Vimscript `typval_T`, and API `Object`
//! (`src/nvim/lua/converter.c`).

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE, K_SPECIAL_VAR_NULL};
use crate::api::private::helpers::{
    api_free_array, api_free_dict, api_free_object, api_set_error, api_typename, arena_array,
    arena_dict, cbuf_to_arena_str, error_set, nil_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::buffer::NOTDONE;
use crate::eval::decode::{
    K_MP_EXT, K_MP_INTEGER, K_MP_MAP, decode_create_map_special_dict, decode_string,
};
use crate::eval::encode::{
    K_MP_ARRAY, K_MP_BOOLEAN, K_MP_FLOAT, K_MP_NIL, K_MP_STRING, MPConvStackData, MPConvStackVal,
    MPConvStackValType, encode_vim_list_to_buf,
};
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, blob_T, dict_T, dictitem_T, partial_T, tv_blob_len,
    tv_clear, tv_copy, tv_dict_add, tv_dict_alloc, tv_dict_find, tv_dict_item_alloc_len,
    tv_list_alloc, tv_list_append_list, tv_list_append_owned_tv, tv_list_first, tv_list_last,
    tv_list_len, tv_list_ref, typval_T,
};
use crate::eval::userfunc::{FC_LUAREF, find_func, register_luafunc};
use crate::eval::vars::{NUM_MSGPACK_TYPES, eval_msgpack_type_lists};
use crate::eval::{get_copyID, partial_name};
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::highlight_group::syn_check_group;

use crate::lua::executor::{api_free_luaref, nlua_pushref, nlua_ref_global};
use crate::math::{FAIL, OK};
use crate::memory::{Arena, arena_memdupz, xfree, xstrdup};
use crate::message::{emsg, internal_error, semsg};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use std::ffi::{CStr, c_char, c_int, c_void};

pub type Boolean = bool;
pub type Float = f64;
pub type LuaRef = c_int;
pub(crate) const LUA_NOREF: LuaRef = -2;
pub type lua_State = c_void;
pub type lua_CFunction = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State) -> c_int>;
pub type lua_Number = f64;
pub type lua_Integer = isize;
pub type lua_Alloc = Option<
    unsafe extern "C-unwind" fn(
        ud: *mut c_void,
        ptr: *mut c_void,
        osize: usize,
        nsize: usize,
    ) -> *mut c_void,
>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

unsafe impl Sync for luaL_Reg {}

#[repr(C)]
pub struct luaL_Buffer {
    pub p: *mut c_char,
    pub lvl: c_int,
    pub L: *mut lua_State,
    pub buffer: [c_char; 8192],
}

#[repr(C)]
pub struct lua_Debug {
    pub event: c_int,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub currentline: c_int,
    pub nups: c_int,
    pub linedefined: c_int,
    pub lastlinedefined: c_int,
    pub short_src: [c_char; 60],
    pub i_ci: c_int,
    pub _pad: [u8; 32],
}

pub(crate) const K_NLUA_PUSH_SPECIAL: c_int = 0x01;
const K_NLUA_PUSH_FREE_REFS: c_int = 0x02;

const TYPE_IDX_VALUE: bool = true;
const VAL_IDX_VALUE: bool = false;

pub const LUA_MULTRET: c_int = -1;
pub const LUA_REGISTRYINDEX: c_int = -10000;
pub const LUA_GLOBALSINDEX: c_int = -10002;
pub(crate) const LUA_MINSTACK: c_int = 20;

#[cfg(feature = "vendor-bit")]
pub(crate) const LUA_TNONE: c_int = -1;
pub(crate) const LUA_TNIL: c_int = 0;
pub(crate) const LUA_TBOOLEAN: c_int = 1;
pub(crate) const LUA_TLIGHTUSERDATA: c_int = 2;
pub(crate) const LUA_TNUMBER: c_int = 3;
pub(crate) const LUA_TSTRING: c_int = 4;
pub(crate) const LUA_TTABLE: c_int = 5;
pub(crate) const LUA_TFUNCTION: c_int = 6;
pub(crate) const LUA_TUSERDATA: c_int = 7;

pub(crate) const LUA_ERRRUN: c_int = 2;
pub(crate) const LUA_ERRMEM: c_int = 4;

pub(crate) const LUAL_BUFFERSIZE: usize = if cfg!(windows) { 512 } else { 8192 };

static mut TYPVAL_CONV_SPECIAL: bool = false;

#[repr(C)]
pub struct nlua_ref_state_t {
    pub nil_ref: LuaRef,
    pub empty_dict_ref: LuaRef,
    pub ref_count: c_int,
}

#[repr(C)]
pub struct OptKeySet {
    pub is_set_: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeySetLink {
    pub str: *mut c_char,
    pub ptr_off: usize,
    pub key_type: c_int,
    pub opt_index: c_int,
    pub is_hlgroup: bool,
}

unsafe impl Sync for KeySetLink {}

pub type FieldHashfn = Option<unsafe fn(str: *const c_char, len: usize) -> *const KeySetLink>;

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
    tv: *mut typval_T,
    list_len: usize,
    container: bool,
    special: bool,
    idx: c_int,
}

#[derive(Clone, Copy)]
struct ObjPopStackItem {
    obj: *mut Object,
    container: bool,
}

unsafe extern "C-unwind" {
    pub fn lua_gettop(L: *mut lua_State) -> c_int;
    pub fn lua_settop(L: *mut lua_State, idx: c_int);
    pub fn lua_pushvalue(L: *mut lua_State, idx: c_int);
    pub fn lua_remove(L: *mut lua_State, idx: c_int);
    pub fn lua_insert(L: *mut lua_State, idx: c_int);
    pub fn lua_replace(L: *mut lua_State, idx: c_int);
    pub fn lua_checkstack(L: *mut lua_State, sz: c_int) -> c_int;
    pub fn lua_type(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_typename(L: *mut lua_State, tp: c_int) -> *const c_char;
    pub fn lua_equal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    pub fn lua_rawequal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    pub fn lua_tonumber(L: *mut lua_State, idx: c_int) -> lua_Number;
    pub fn lua_toboolean(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
    pub fn lua_objlen(L: *mut lua_State, idx: c_int) -> usize;
    pub fn lua_tocfunction(L: *mut lua_State, idx: c_int) -> lua_CFunction;
    pub fn lua_topointer(L: *mut lua_State, idx: c_int) -> *const c_void;
    pub fn lua_pushnil(L: *mut lua_State);
    pub fn lua_pushnumber(L: *mut lua_State, n: lua_Number);
    pub fn lua_pushinteger(L: *mut lua_State, n: lua_Integer);
    pub fn lua_pushlstring(L: *mut lua_State, s: *const c_char, l: usize);
    pub fn lua_pushstring(L: *mut lua_State, s: *const c_char);
    pub fn lua_pushfstring(L: *mut lua_State, fmt: *const c_char, ...) -> *const c_char;
    pub fn lua_pushboolean(L: *mut lua_State, b: c_int);
    pub fn lua_pushlightuserdata(L: *mut lua_State, p: *mut c_void);
    pub fn lua_gettable(L: *mut lua_State, idx: c_int);
    pub fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    pub fn lua_settable(L: *mut lua_State, idx: c_int);
    pub fn lua_setfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    pub fn lua_rawget(L: *mut lua_State, idx: c_int);
    pub fn lua_rawgeti(L: *mut lua_State, idx: c_int, n: c_int);
    pub fn lua_createtable(L: *mut lua_State, narr: c_int, nrec: c_int);
    pub fn lua_newuserdata(L: *mut lua_State, sz: usize) -> *mut c_void;
    pub fn lua_getmetatable(L: *mut lua_State, objindex: c_int) -> c_int;
    pub fn lua_getfenv(L: *mut lua_State, idx: c_int);
    pub fn lua_rawset(L: *mut lua_State, idx: c_int);
    pub fn lua_rawseti(L: *mut lua_State, idx: c_int, n: c_int);
    pub fn lua_setmetatable(L: *mut lua_State, objindex: c_int) -> c_int;
    pub fn lua_setfenv(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_call(L: *mut lua_State, nargs: c_int, nresults: c_int);
    pub fn lua_pcall(L: *mut lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;
    pub fn lua_error(L: *mut lua_State) -> c_int;
    pub fn lua_next(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_concat(L: *mut lua_State, n: c_int);
    pub fn lua_getallocf(L: *mut lua_State, ud: *mut *mut c_void) -> lua_Alloc;
    pub fn lua_pushcclosure(L: *mut lua_State, fn_: lua_CFunction, n: c_int);
    pub fn lua_isnumber(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isstring(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_iscfunction(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isuserdata(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tointeger(L: *mut lua_State, idx: c_int) -> lua_Integer;
    pub fn lua_touserdata(L: *mut lua_State, idx: c_int) -> *mut c_void;
    pub fn luaL_newstate() -> *mut lua_State;
    pub fn luaL_openlibs(L: *mut lua_State);
    pub fn lua_close(L: *mut lua_State);
    pub fn luaL_ref(L: *mut lua_State, t: c_int) -> c_int;
    pub fn luaL_unref(L: *mut lua_State, t: c_int, ref_: c_int);
    pub fn luaL_newmetatable(L: *mut lua_State, tname: *const c_char) -> c_int;
    pub fn luaL_getmetafield(L: *mut lua_State, obj: c_int, e: *const c_char) -> c_int;
    pub fn luaL_checkudata(L: *mut lua_State, ud: c_int, tname: *const c_char) -> *mut c_void;
    pub fn luaL_register(L: *mut lua_State, libname: *const c_char, l: *const luaL_Reg);
    pub fn luaL_checkstack(L: *mut lua_State, sz: c_int, msg: *const c_char);
    pub fn luaL_checktype(L: *mut lua_State, narg: c_int, t: c_int);
    pub fn luaL_checkany(L: *mut lua_State, narg: c_int);
    pub fn luaL_checklstring(L: *mut lua_State, numArg: c_int, l: *mut usize) -> *const c_char;
    pub fn luaL_checknumber(L: *mut lua_State, numArg: c_int) -> lua_Number;
    pub fn luaL_checkinteger(L: *mut lua_State, numArg: c_int) -> lua_Integer;
    pub fn luaL_optinteger(L: *mut lua_State, nArg: c_int, def: lua_Integer) -> lua_Integer;
    #[cfg(feature = "vendor-bit")]
    pub fn luaL_typerror(L: *mut lua_State, narg: c_int, tname: *const c_char) -> c_int;
    pub fn luaL_argerror(L: *mut lua_State, numarg: c_int, extramsg: *const c_char) -> c_int;
    pub fn luaL_where(L: *mut lua_State, lvl: c_int);
    pub fn luaL_error(L: *mut lua_State, fmt: *const c_char, ...) -> c_int;
    pub fn luaL_loadbuffer(
        L: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
    ) -> c_int;
    pub fn luaL_buffinit(L: *mut lua_State, B: *mut luaL_Buffer);
    pub fn luaL_prepbuffer(B: *mut luaL_Buffer) -> *mut c_char;
    pub fn luaL_addlstring(B: *mut luaL_Buffer, s: *const c_char, l: usize);
    pub fn luaL_addvalue(B: *mut luaL_Buffer);
    pub fn luaL_pushresult(B: *mut luaL_Buffer);
    pub fn lua_getstack(L: *mut lua_State, level: c_int, ar: *mut lua_Debug) -> c_int;
    pub fn lua_getinfo(L: *mut lua_State, what: *const c_char, ar: *mut lua_Debug) -> c_int;
}

#[inline]
pub const fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_GLOBALSINDEX - i
}

#[inline]
pub unsafe fn lua_pop(lstate: *mut lua_State, n: c_int) {
    unsafe {
        lua_settop(lstate, -n - 1);
    }
}

#[inline]
pub unsafe fn lua_newtable(lstate: *mut lua_State) {
    unsafe {
        lua_createtable(lstate, 0, 0);
    }
}

#[inline]
pub unsafe fn lua_pushcfunction(lstate: *mut lua_State, f: lua_CFunction) {
    unsafe {
        lua_pushcclosure(lstate, f, 0);
    }
}

#[inline]
pub unsafe fn lua_setglobal(lstate: *mut lua_State, s: *const c_char) {
    unsafe {
        lua_setfield(lstate, LUA_GLOBALSINDEX, s);
    }
}

#[inline]
pub unsafe fn lua_getglobal(lstate: *mut lua_State, s: *const c_char) {
    unsafe {
        lua_getfield(lstate, LUA_GLOBALSINDEX, s);
    }
}

#[inline]
pub unsafe fn lua_tostring(lstate: *mut lua_State, i: c_int) -> *const c_char {
    unsafe { lua_tolstring(lstate, i, std::ptr::null_mut()) }
}

#[inline]
pub unsafe fn lua_isfunction(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TFUNCTION }
}

#[inline]
pub unsafe fn lua_istable(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TTABLE }
}

#[inline]
pub unsafe fn lua_isnil(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TNIL }
}

#[inline]
pub unsafe fn lua_isboolean(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TBOOLEAN }
}

#[inline]
pub unsafe fn lua_isnoneornil(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) <= 0 }
}

#[inline]
pub unsafe fn luaL_typename(lstate: *mut lua_State, i: c_int) -> *const c_char {
    unsafe { lua_typename(lstate, lua_type(lstate, i)) }
}

#[inline]
pub unsafe fn luaL_getmetatable(lstate: *mut lua_State, n: *const c_char) {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, n);
    }
}

#[inline]
pub unsafe fn luaL_argcheck(
    lstate: *mut lua_State,
    cond: bool,
    numarg: c_int,
    extramsg: *const c_char,
) {
    if !cond {
        unsafe {
            luaL_argerror(lstate, numarg, extramsg);
        }
    }
}

#[inline]
pub unsafe fn luaL_checkstring(lstate: *mut lua_State, n: c_int) -> *const c_char {
    unsafe { luaL_checklstring(lstate, n, std::ptr::null_mut()) }
}

#[inline]
pub unsafe fn luaL_checkint(lstate: *mut lua_State, n: c_int) -> c_int {
    unsafe { luaL_checkinteger(lstate, n) as c_int }
}

#[inline]
unsafe fn nlua_push_type_idx(lstate: *mut lua_State) {
    unsafe {
        lua_pushboolean(lstate, c_int::from(TYPE_IDX_VALUE));
    }
}

#[inline]
unsafe fn nlua_push_val_idx(lstate: *mut lua_State) {
    unsafe {
        lua_pushboolean(lstate, c_int::from(VAL_IDX_VALUE));
    }
}

#[inline]
unsafe fn nlua_push_type(lstate: *mut lua_State, obj_type: ObjectType) {
    unsafe {
        lua_pushnumber(lstate, (obj_type as c_int) as lua_Number);
    }
}

#[inline]
unsafe fn nlua_create_typed_table(
    lstate: *mut lua_State,
    narr: usize,
    nrec: usize,
    obj_type: ObjectType,
) {
    unsafe {
        lua_createtable(lstate, narr as c_int, (1 + nrec) as c_int);
        nlua_push_type_idx(lstate);
        nlua_push_type(lstate, obj_type);
        lua_rawset(lstate, -3);
    }
}

unsafe fn nlua_traverse_table(lstate: *mut lua_State) -> LuaTableProps {
    unsafe {
        let mut tsize = 0usize;
        let mut val_type = 0 as c_int;
        let mut has_val_key = false;
        let mut other_keys_num = 0usize;
        let mut ret = LuaTableProps {
            maxidx: 0,
            string_keys_num: 0,
            has_string_with_nul: false,
            table_type: ObjectType::kObjectTypeNil,
            val: 0.0,
            has_type_key: false,
        };

        if lua_checkstack(lstate, lua_gettop(lstate) + 3) == 0 {
            semsg!(
                c"E1502: Lua failed to grow stack to %i".as_ptr(),
                lua_gettop(lstate) + 2,
            );
            ret.table_type = ObjectType::kObjectTypeNil;
            return ret;
        }

        lua_pushnil(lstate);
        while lua_next(lstate, -2) != 0 {
            match lua_type(lstate, -2) {
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -2, &mut len);
                    if len > 0 {
                        let slice = std::slice::from_raw_parts(s.cast::<u8>(), len);
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
                            if n == (ObjectType::kObjectTypeFloat as c_int as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::kObjectTypeFloat;
                            } else if n == (ObjectType::kObjectTypeArray as c_int as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::kObjectTypeArray;
                            } else if n == (ObjectType::kObjectTypeDict as c_int as lua_Number) {
                                ret.has_type_key = true;
                                ret.table_type = ObjectType::kObjectTypeDict;
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
            if ret.table_type == ObjectType::kObjectTypeFloat
                && (!has_val_key || val_type != LUA_TNUMBER)
            {
                ret.table_type = ObjectType::kObjectTypeNil;
            } else if ret.table_type == ObjectType::kObjectTypeArray {
                let expected = tsize
                    - usize::from(ret.has_type_key)
                    - other_keys_num
                    - usize::from(has_val_key)
                    - ret.string_keys_num;
                if ret.maxidx != 0 && ret.maxidx != expected {
                    ret.maxidx = 0;
                    loop {
                        lua_rawgeti(lstate, -1, (ret.maxidx as c_int) + 1);
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
            ret.table_type = ObjectType::kObjectTypeArray;
            if tsize == 0 && lua_getmetatable(lstate, -1) != 0 {
                nlua_pushref(lstate, (*nlua_global_refs).empty_dict_ref);
                if lua_rawequal(lstate, -2, -1) != 0 {
                    ret.table_type = ObjectType::kObjectTypeDict;
                }
                lua_pop(lstate, 2);
            }
        } else if ret.string_keys_num == tsize {
            ret.table_type = ObjectType::kObjectTypeDict;
        } else {
            ret.table_type = ObjectType::kObjectTypeNil;
        }
        ret
    }
}

/// Convert Lua object to Vimscript `typval_T` (`nlua_pop_typval`).
pub unsafe extern "C-unwind" fn nlua_pop_typval(
    lstate: *mut lua_State,
    ret_tv: *mut typval_T,
) -> bool {
    unsafe {
        let mut ret = true;
        let initial_size = lua_gettop(lstate);
        let mut stack: Vec<TVPopStackItem> = Vec::with_capacity(2);
        stack.push(TVPopStackItem {
            tv: ret_tv,
            list_len: 0,
            container: false,
            special: false,
            idx: 0,
        });

        while ret && !stack.is_empty() {
            if lua_checkstack(lstate, lua_gettop(lstate) + 3) == 0 {
                semsg!(
                    c"E1502: Lua failed to grow stack to %i".as_ptr(),
                    lua_gettop(lstate) + 3,
                );
                ret = false;
                break;
            }
            let mut cur = stack.pop().unwrap();
            if cur.container {
                if cur.special || (*cur.tv).v_type == VarType::VAR_DICT {
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
                        if cur.special {
                            let kv_pair = tv_list_alloc(2);
                            let s_tv = decode_string(s, len, true, false);
                            tv_list_append_owned_tv(kv_pair, s_tv);
                            tv_list_append_owned_tv(
                                kv_pair,
                                typval_T {
                                    v_type: VarType::VAR_UNKNOWN,
                                    v_lock: VAR_UNLOCKED,
                                    vval: TypvalVval { v_number: 0 },
                                },
                            );
                            stack.push(cur);
                            tv_list_append_list((*cur.tv).vval.v_list, kv_pair);
                            let last = tv_list_last(kv_pair);
                            cur = TVPopStackItem {
                                tv: &mut (*last).li_tv,
                                list_len: 0,
                                container: false,
                                special: false,
                                idx: 0,
                            };
                        } else {
                            let di = tv_dict_item_alloc_len(s, len);
                            if tv_dict_add((*cur.tv).vval.v_ptr.cast::<dict_T>(), di) == FAIL {
                                std::process::abort();
                            }
                            stack.push(cur);
                            cur = TVPopStackItem {
                                tv: &mut (*di).di_tv,
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
                    let list = (*cur.tv).vval.v_list;
                    if (tv_list_len(list) as usize) == cur.list_len {
                        lua_pop(lstate, 1);
                        continue;
                    }
                    lua_rawgeti(lstate, -1, tv_list_len(list) + 1);
                    tv_list_append_owned_tv(
                        list,
                        typval_T {
                            v_type: VarType::VAR_UNKNOWN,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval { v_number: 0 },
                        },
                    );
                    stack.push(cur);
                    let last = tv_list_last(list);
                    cur = TVPopStackItem {
                        tv: &mut (*last).li_tv,
                        list_len: 0,
                        container: false,
                        special: false,
                        idx: 0,
                    };
                }
            }

            *cur.tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };

            match lua_type(lstate, -1) {
                LUA_TNIL => {
                    (*cur.tv).v_type = VarType::VAR_SPECIAL;
                    (*cur.tv).vval.v_number = K_SPECIAL_VAR_NULL;
                }
                LUA_TBOOLEAN => {
                    (*cur.tv).v_type = VarType::VAR_BOOL;
                    (*cur.tv).vval.v_number = if lua_toboolean(lstate, -1) != 0 {
                        K_BOOL_VAR_TRUE
                    } else {
                        K_BOOL_VAR_FALSE
                    };
                }
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -1, &mut len);
                    *cur.tv = decode_string(s, len, false, false);
                }
                LUA_TNUMBER => {
                    let n = lua_tonumber(lstate, -1);
                    if n > (i64::MAX as lua_Number)
                        || n < (i64::MIN as lua_Number)
                        || ((n as i64) as lua_Number) != n
                    {
                        (*cur.tv).v_type = VarType::VAR_FLOAT;
                        (*cur.tv).vval.v_float = n;
                    } else {
                        (*cur.tv).v_type = VarType::VAR_NUMBER;
                        (*cur.tv).vval.v_number = n as i64;
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
                            tv_copy(item.tv, cur.tv);
                            cur.container = false;
                            found_self_ref = true;
                            break;
                        }
                    }

                    if !found_self_ref {
                        match table_props.table_type {
                            ObjectType::kObjectTypeArray => {
                                let l = tv_list_alloc(table_props.maxidx as isize);
                                (*l).lua_table_ref = table_ref;
                                tv_list_ref(l);
                                (*cur.tv).v_type = VarType::VAR_LIST;
                                (*cur.tv).vval.v_list = l;
                                cur.list_len = table_props.maxidx;
                                if table_props.maxidx != 0 {
                                    cur.container = true;
                                    cur.idx = lua_gettop(lstate);
                                    stack.push(cur);
                                }
                            }
                            ObjectType::kObjectTypeDict => {
                                if table_props.string_keys_num == 0 {
                                    let d = tv_dict_alloc();
                                    (*d).dv_refcount += 1;
                                    (*d).lua_table_ref = table_ref;
                                    (*cur.tv).v_type = VarType::VAR_DICT;
                                    (*cur.tv).vval.v_ptr = d.cast::<c_void>();
                                } else {
                                    cur.special = table_props.has_string_with_nul;
                                    if table_props.has_string_with_nul {
                                        decode_create_map_special_dict(
                                            cur.tv,
                                            table_props.string_keys_num as isize,
                                        );
                                        let val_di = tv_dict_find(
                                            (*cur.tv).vval.v_ptr.cast::<dict_T>(),
                                            c"_VAL".as_ptr(),
                                            4,
                                        );
                                        assert!(!val_di.is_null());
                                        cur.tv = &mut (*val_di).di_tv;
                                        (*(*cur.tv).vval.v_list).lua_table_ref = table_ref;
                                        cur.list_len = table_props.string_keys_num;
                                    } else {
                                        let d = tv_dict_alloc();
                                        (*d).dv_refcount += 1;
                                        (*d).lua_table_ref = table_ref;
                                        (*cur.tv).v_type = VarType::VAR_DICT;
                                        (*cur.tv).vval.v_ptr = d.cast::<c_void>();
                                    }
                                    cur.container = true;
                                    cur.idx = lua_gettop(lstate);
                                    stack.push(cur);
                                    lua_pushnil(lstate);
                                }
                            }
                            ObjectType::kObjectTypeFloat => {
                                (*cur.tv).v_type = VarType::VAR_FLOAT;
                                (*cur.tv).vval.v_float = table_props.val;
                            }
                            ObjectType::kObjectTypeNil => {
                                emsg(
                                    c"E5100: Cannot convert given Lua table: table should contain either only integer keys or only string keys"
                                        .as_ptr(),
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
                    (*cur.tv).v_type = VarType::VAR_FUNC;
                    (*cur.tv).vval.v_string = xstrdup(name);
                }
                LUA_TUSERDATA => {
                    nlua_pushref(lstate, (*nlua_global_refs).nil_ref);
                    let is_nil = lua_rawequal(lstate, -2, -1) != 0;
                    lua_pop(lstate, 1);
                    if is_nil {
                        (*cur.tv).v_type = VarType::VAR_SPECIAL;
                        (*cur.tv).vval.v_number = K_SPECIAL_VAR_NULL;
                    } else {
                        emsg(c"E5101: Cannot convert given Lua type".as_ptr());
                        ret = false;
                    }
                }
                _ => {
                    emsg(c"E5101: Cannot convert given Lua type".as_ptr());
                    ret = false;
                }
            }
            if !cur.container {
                lua_pop(lstate, 1);
            }
        }

        if !ret {
            tv_clear(ret_tv);
            *ret_tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
            lua_pop(lstate, lua_gettop(lstate) - initial_size + 1);
        }
        assert_eq!(lua_gettop(lstate), initial_size - 1);
        ret
    }
}

#[inline]
unsafe fn lua_conv_nil(lstate: *mut lua_State) {
    unsafe {
        if TYPVAL_CONV_SPECIAL {
            lua_pushnil(lstate);
        } else {
            nlua_pushref(lstate, (*nlua_global_refs).nil_ref);
        }
    }
}

#[inline]
unsafe fn lua_conv_empty_dict(lstate: *mut lua_State) {
    unsafe {
        if TYPVAL_CONV_SPECIAL {
            nlua_create_typed_table(lstate, 0, 0, ObjectType::kObjectTypeDict);
        } else {
            lua_createtable(lstate, 0, 0);
            nlua_pushref(lstate, (*nlua_global_refs).empty_dict_ref);
            lua_setmetatable(lstate, -2);
        }
    }
}

#[inline]
unsafe fn lua_conv_list_start(lstate: *mut lua_State, len: c_int) -> c_int {
    unsafe {
        if lua_checkstack(lstate, lua_gettop(lstate) + 3) == 0 {
            semsg!(
                c"E5102: Lua failed to grow stack to %i".as_ptr(),
                lua_gettop(lstate) + 3,
            );
            return FAIL;
        }
        lua_createtable(lstate, len, 0);
        lua_pushnumber(lstate, 1.0);
        OK
    }
}

#[inline]
unsafe fn lua_conv_dict_start(lstate: *mut lua_State, len: usize) -> c_int {
    unsafe {
        if lua_checkstack(lstate, lua_gettop(lstate) + 3) == 0 {
            semsg!(
                c"E5102: Lua failed to grow stack to %i".as_ptr(),
                lua_gettop(lstate) + 3,
            );
            return FAIL;
        }
        lua_createtable(lstate, 0, len as c_int);
        OK
    }
}

unsafe fn lua_check_self_reference(
    lstate: *mut lua_State,
    val: *mut c_void,
    val_copy_id: *mut c_int,
    mpstack: &[MPConvStackVal],
    copy_id: c_int,
    conv_type: MPConvStackValType,
) -> c_int {
    unsafe {
        if *val_copy_id != copy_id {
            *val_copy_id = copy_id;
            return NOTDONE;
        }
        let mut backref = mpstack.len();
        while backref > 0 {
            let mpval = mpstack[backref - 1];
            if mpval.val_type == conv_type {
                let matched = match mpval.data {
                    MPConvStackData::Dict { dict, .. } if conv_type == MPConvStackValType::Dict => {
                        dict.cast::<c_void>() == val
                    }
                    MPConvStackData::List { list, .. } if conv_type != MPConvStackValType::Dict => {
                        list.cast::<c_void>() == val
                    }
                    _ => false,
                };
                if matched {
                    let stack_offset = -(((mpstack.len() - backref + 1) * 2) as c_int);
                    lua_pushvalue(lstate, stack_offset);
                    break;
                }
            }
            backref -= 1;
        }
        OK
    }
}

unsafe fn lua_convert_one_value(
    lstate: *mut lua_State,
    mpstack: &mut Vec<MPConvStackVal>,
    tv: *mut typval_T,
    copy_id: c_int,
) -> c_int {
    unsafe {
        let tv_ref = &mut *tv;
        match tv_ref.v_type {
            VarType::VAR_STRING => {
                let s = tv_ref.vval.v_string;
                let len = if s.is_null() {
                    0
                } else {
                    CStr::from_ptr(s).to_bytes().len()
                };
                lua_pushlstring(lstate, s, len);
            }
            VarType::VAR_NUMBER => {
                lua_pushnumber(lstate, tv_ref.vval.v_number as lua_Number);
            }
            VarType::VAR_FLOAT => {
                lua_pushnumber(lstate, tv_ref.vval.v_float);
            }
            VarType::VAR_BLOB => {
                let blob = tv_ref.vval.v_ptr.cast::<blob_T>();
                let len = tv_blob_len(blob) as usize;
                let data = if !blob.is_null() && !(*blob).bv_ga.is_empty() {
                    (*blob).bv_ga.as_ptr().cast::<c_char>()
                } else {
                    c"".as_ptr()
                };
                lua_pushlstring(lstate, data, len);
            }
            VarType::VAR_FUNC => {
                let fun = tv_ref.vval.v_string;
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                        nlua_pushref(lstate, (*fp).uf_luaref);
                    } else {
                        lua_conv_nil(lstate);
                    }
                } else {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VAR_PARTIAL => {
                let pt = tv_ref.vval.v_ptr.cast::<partial_T>();
                let fun = if pt.is_null() {
                    std::ptr::null_mut()
                } else {
                    partial_name(pt)
                };
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                        nlua_pushref(lstate, (*fp).uf_luaref);
                    } else {
                        lua_conv_nil(lstate);
                    }
                } else {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VAR_LIST => {
                let list = tv_ref.vval.v_list;
                if list.is_null() || tv_list_len(list) == 0 {
                    lua_createtable(lstate, 0, 0);
                } else {
                    let saved_copy_id = (*list).lv_copyID;
                    let csr = lua_check_self_reference(
                        lstate,
                        list.cast::<c_void>(),
                        &mut (*list).lv_copyID,
                        mpstack,
                        copy_id,
                        MPConvStackValType::List,
                    );
                    if csr != NOTDONE {
                        return csr;
                    }
                    if lua_conv_list_start(lstate, tv_list_len(list)) == FAIL {
                        return FAIL;
                    }
                    mpstack.push(MPConvStackVal {
                        val_type: MPConvStackValType::List,
                        tv,
                        saved_copy_id,
                        data: MPConvStackData::List {
                            list,
                            li: tv_list_first(list),
                        },
                    });
                }
            }
            VarType::VAR_BOOL => {
                lua_pushboolean(
                    lstate,
                    c_int::from(tv_ref.vval.v_bool == K_BOOL_VAR_TRUE as c_int),
                );
            }
            VarType::VAR_SPECIAL => {
                if tv_ref.vval.v_special == K_SPECIAL_VAR_NULL as c_int {
                    lua_conv_nil(lstate);
                }
            }
            VarType::VAR_DICT => {
                let dict = tv_ref.vval.v_ptr.cast::<dict_T>();
                if dict.is_null() || (*dict).dv_hashtab.ht_used == 0 {
                    lua_conv_empty_dict(lstate);
                    return OK;
                }
                let mut handled_special = false;
                if (*dict).dv_hashtab.ht_used == 2 {
                    let type_di = tv_dict_find(dict, c"_TYPE".as_ptr(), 5);
                    if !type_di.is_null() && (*type_di).di_tv.v_type == VarType::VAR_LIST {
                        let val_di = tv_dict_find(dict, c"_VAL".as_ptr(), 4);
                        if !val_di.is_null() {
                            let mut mp_idx = NUM_MSGPACK_TYPES;
                            for (i, tl) in { eval_msgpack_type_lists }.into_iter().enumerate() {
                                if (*type_di).di_tv.vval.v_list.cast_const() == tl {
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
                                        if (*val_di).di_tv.v_type == VarType::VAR_NUMBER {
                                            lua_pushboolean(
                                                lstate,
                                                c_int::from((*val_di).di_tv.vval.v_number != 0),
                                            );
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_INTEGER => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            if tv_list_len(val_list) == 4 {
                                                let sign_li = tv_list_first(val_list);
                                                let highest_li = (*sign_li).li_next;
                                                let high_li = (*highest_li).li_next;
                                                let low_li = tv_list_last(val_list);
                                                if (*sign_li).li_tv.v_type == VarType::VAR_NUMBER
                                                    && (*sign_li).li_tv.vval.v_number != 0
                                                    && (*highest_li).li_tv.v_type
                                                        == VarType::VAR_NUMBER
                                                    && (*highest_li).li_tv.vval.v_number >= 0
                                                    && (*high_li).li_tv.v_type
                                                        == VarType::VAR_NUMBER
                                                    && (*high_li).li_tv.vval.v_number >= 0
                                                    && (*low_li).li_tv.v_type == VarType::VAR_NUMBER
                                                    && (*low_li).li_tv.vval.v_number >= 0
                                                {
                                                    let sign = (*sign_li).li_tv.vval.v_number;
                                                    let highest =
                                                        (*highest_li).li_tv.vval.v_number as u64;
                                                    let high =
                                                        (*high_li).li_tv.vval.v_number as u64;
                                                    let low = (*low_li).li_tv.vval.v_number as u64;
                                                    let number =
                                                        (highest << 62) | (high << 31) | low;
                                                    if sign > 0 {
                                                        lua_pushnumber(
                                                            lstate,
                                                            number as lua_Number,
                                                        );
                                                    } else {
                                                        lua_pushnumber(
                                                            lstate,
                                                            (number.wrapping_neg() as i64)
                                                                as lua_Number,
                                                        );
                                                    }
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    K_MP_FLOAT => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_FLOAT {
                                            lua_pushnumber(lstate, (*val_di).di_tv.vval.v_float);
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_STRING => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let mut len = 0usize;
                                            let mut buf: *mut c_char = std::ptr::null_mut();
                                            if encode_vim_list_to_buf(
                                                (*val_di).di_tv.vval.v_list,
                                                &mut len,
                                                &mut buf,
                                            ) {
                                                lua_pushlstring(lstate, buf, len);
                                                xfree(buf.cast::<c_void>());
                                                handled_special = true;
                                            }
                                        }
                                    }
                                    K_MP_ARRAY => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            let saved_copy_id = (*val_list).lv_copyID;
                                            let csr = lua_check_self_reference(
                                                lstate,
                                                val_list.cast::<c_void>(),
                                                &mut (*val_list).lv_copyID,
                                                mpstack,
                                                copy_id,
                                                MPConvStackValType::List,
                                            );
                                            if csr != NOTDONE {
                                                return csr;
                                            }
                                            if lua_conv_list_start(lstate, tv_list_len(val_list))
                                                == FAIL
                                            {
                                                return FAIL;
                                            }
                                            mpstack.push(MPConvStackVal {
                                                val_type: MPConvStackValType::List,
                                                tv,
                                                saved_copy_id,
                                                data: MPConvStackData::List {
                                                    list: val_list,
                                                    li: tv_list_first(val_list),
                                                },
                                            });
                                            handled_special = true;
                                        }
                                    }
                                    K_MP_MAP => {
                                        if (*val_di).di_tv.v_type == VarType::VAR_LIST {
                                            let val_list = (*val_di).di_tv.vval.v_list;
                                            if val_list.is_null() || tv_list_len(val_list) == 0 {
                                                lua_conv_empty_dict(lstate);
                                                handled_special = true;
                                            } else {
                                                let mut all_pairs = true;
                                                let mut li = tv_list_first(val_list);
                                                while !li.is_null() {
                                                    if (*li).li_tv.v_type != VarType::VAR_LIST
                                                        || tv_list_len((*li).li_tv.vval.v_list) != 2
                                                    {
                                                        all_pairs = false;
                                                        break;
                                                    }
                                                    li = (*li).li_next;
                                                }
                                                if all_pairs {
                                                    let saved_copy_id = (*val_list).lv_copyID;
                                                    let csr = lua_check_self_reference(
                                                        lstate,
                                                        val_list.cast::<c_void>(),
                                                        &mut (*val_list).lv_copyID,
                                                        mpstack,
                                                        copy_id,
                                                        MPConvStackValType::Pairs,
                                                    );
                                                    if csr != NOTDONE {
                                                        return csr;
                                                    }
                                                    if lua_conv_dict_start(
                                                        lstate,
                                                        tv_list_len(val_list) as usize,
                                                    ) == FAIL
                                                    {
                                                        return FAIL;
                                                    }
                                                    mpstack.push(MPConvStackVal {
                                                        val_type: MPConvStackValType::Pairs,
                                                        tv,
                                                        saved_copy_id,
                                                        data: MPConvStackData::List {
                                                            list: val_list,
                                                            li: tv_list_first(val_list),
                                                        },
                                                    });
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    K_MP_EXT if (*val_di).di_tv.v_type == VarType::VAR_LIST => {
                                        let val_list = (*val_di).di_tv.vval.v_list;
                                        if tv_list_len(val_list) == 2 {
                                            let first = tv_list_first(val_list);
                                            let last = tv_list_last(val_list);
                                            if (*first).li_tv.v_type == VarType::VAR_NUMBER
                                                && (i8::MIN as i64..=i8::MAX as i64)
                                                    .contains(&(*first).li_tv.vval.v_number)
                                                && (*last).li_tv.v_type == VarType::VAR_LIST
                                            {
                                                let mut len = 0usize;
                                                let mut buf: *mut c_char = std::ptr::null_mut();
                                                if encode_vim_list_to_buf(
                                                    (*last).li_tv.vval.v_list,
                                                    &mut len,
                                                    &mut buf,
                                                ) {
                                                    lua_conv_nil(lstate);
                                                    xfree(buf.cast::<c_void>());
                                                    handled_special = true;
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                if !handled_special {
                    let saved_copy_id = (*dict).dv_copyID;
                    let csr = lua_check_self_reference(
                        lstate,
                        dict.cast::<c_void>(),
                        &mut (*dict).dv_copyID,
                        mpstack,
                        copy_id,
                        MPConvStackValType::Dict,
                    );
                    if csr != NOTDONE {
                        return csr;
                    }
                    if lua_conv_dict_start(lstate, (*dict).dv_hashtab.ht_used) == FAIL {
                        return FAIL;
                    }
                    mpstack.push(MPConvStackVal {
                        val_type: MPConvStackValType::Dict,
                        tv,
                        saved_copy_id,
                        data: MPConvStackData::Dict {
                            dict,
                            dictp: std::ptr::addr_of_mut!(tv_ref.vval.v_ptr).cast::<*mut dict_T>(),
                            hi: (*dict).dv_hashtab.ht_array,
                            todo: (*dict).dv_hashtab.ht_used,
                        },
                    });
                }
            }
            VarType::VAR_UNKNOWN => {
                internal_error(c"_typval_encode_lua_convert_one_value()".as_ptr());
                return FAIL;
            }
        }
        OK
    }
}

unsafe fn encode_vim_to_lua(lstate: *mut lua_State, top_tv: *mut typval_T) -> c_int {
    unsafe {
        let copy_id = get_copyID();
        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);
        let key_off = std::mem::offset_of!(dictitem_T, di_key);

        if lua_convert_one_value(lstate, &mut mpstack, top_tv, copy_id) == FAIL {
            return FAIL;
        }

        while !mpstack.is_empty() {
            let cur_idx = mpstack.len() - 1;
            let cur_type = mpstack[cur_idx].val_type;
            let next_tv: *mut typval_T = match cur_type {
                MPConvStackValType::Dict => {
                    let (dict, mut hi, todo, saved_copy_id) = match mpstack[cur_idx].data {
                        MPConvStackData::Dict { dict, hi, todo, .. } => {
                            (dict, hi, todo, mpstack[cur_idx].saved_copy_id)
                        }
                        _ => unreachable!(),
                    };
                    if todo == 0 {
                        mpstack.pop();
                        (*dict).dv_copyID = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if todo != (*dict).dv_hashtab.ht_used {
                        lua_rawset(lstate, -3);
                    }
                    while hashitem_empty(&*hi) {
                        hi = hi.add(1);
                    }
                    let di = (*hi).hi_key.sub(key_off).cast::<dictitem_T>();
                    let next_hi = hi.add(1);
                    if let MPConvStackData::Dict {
                        hi: ref mut hi_ref,
                        todo: ref mut todo_ref,
                        ..
                    } = mpstack[cur_idx].data
                    {
                        *todo_ref -= 1;
                        *hi_ref = next_hi;
                    }
                    let key_ptr = (*di).di_key.as_ptr();
                    let key_len = CStr::from_ptr(key_ptr).to_bytes().len();
                    lua_pushlstring(lstate, key_ptr, key_len);
                    &mut (*di).di_tv
                }
                MPConvStackValType::List => {
                    let (list, li, saved_copy_id) = match mpstack[cur_idx].data {
                        MPConvStackData::List { list, li } => {
                            (list, li, mpstack[cur_idx].saved_copy_id)
                        }
                        _ => unreachable!(),
                    };
                    if li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if li != tv_list_first(list) {
                        let idx = lua_tonumber(lstate, -2);
                        lua_rawset(lstate, -3);
                        lua_pushnumber(lstate, idx + 1.0);
                    }
                    let item_tv = &mut (*li).li_tv as *mut typval_T;
                    let next_li = (*li).li_next;
                    if let MPConvStackData::List {
                        li: ref mut li_ref, ..
                    } = mpstack[cur_idx].data
                    {
                        *li_ref = next_li;
                    }
                    item_tv
                }
                MPConvStackValType::Pairs => {
                    let (list, li, saved_copy_id) = match mpstack[cur_idx].data {
                        MPConvStackData::List { list, li } => {
                            (list, li, mpstack[cur_idx].saved_copy_id)
                        }
                        _ => unreachable!(),
                    };
                    if li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        lua_rawset(lstate, -3);
                        continue;
                    } else if li != tv_list_first(list) {
                        lua_rawset(lstate, -3);
                    }
                    let kv_pair = (*li).li_tv.vval.v_list;
                    let first_item = tv_list_first(kv_pair);
                    if lua_convert_one_value(
                        lstate,
                        &mut mpstack,
                        &mut (*first_item).li_tv,
                        copy_id,
                    ) == FAIL
                    {
                        return FAIL;
                    }
                    let last_item = tv_list_last(kv_pair);
                    let next_li = (*li).li_next;
                    if let MPConvStackData::List {
                        li: ref mut li_ref, ..
                    } = mpstack[cur_idx].data
                    {
                        *li_ref = next_li;
                    }
                    &mut (*last_item).li_tv
                }
                MPConvStackValType::Partial | MPConvStackValType::PartialList => unreachable!(),
            };

            if lua_convert_one_value(lstate, &mut mpstack, next_tv, copy_id) == FAIL {
                return FAIL;
            }
        }
        OK
    }
}

/// Convert Vimscript `typval_T` to Lua value (`nlua_push_typval`).
pub unsafe extern "C-unwind" fn nlua_push_typval(
    lstate: *mut lua_State,
    tv: *mut typval_T,
    flags: c_int,
) -> bool {
    unsafe {
        TYPVAL_CONV_SPECIAL = (flags & K_NLUA_PUSH_SPECIAL) != 0;
        let initial_size = lua_gettop(lstate);

        if lua_checkstack(lstate, initial_size + 2) == 0 {
            semsg!(
                c"E1502: Lua failed to grow stack to %i".as_ptr(),
                initial_size + 4,
            );
            return false;
        }
        if encode_vim_to_lua(lstate, tv) == FAIL {
            return false;
        }
        assert_eq!(lua_gettop(lstate), initial_size + 1);
        true
    }
}

/// Convert given `String` to Lua string (`nlua_push_String`).
pub unsafe extern "C-unwind" fn nlua_push_String(
    lstate: *mut lua_State,
    s: NvimString,
    _flags: c_int,
) {
    unsafe {
        let data = if s.size != 0 { s.data } else { c"".as_ptr() };
        lua_pushlstring(lstate, data, s.size);
    }
}

/// Convert given `Integer` to Lua number (`nlua_push_Integer`).
pub unsafe extern "C-unwind" fn nlua_push_Integer(
    lstate: *mut lua_State,
    n: Integer,
    _flags: c_int,
) {
    unsafe {
        lua_pushnumber(lstate, n as lua_Number);
    }
}

/// Convert given `Float` to Lua number or typed table (`nlua_push_Float`).
pub unsafe extern "C-unwind" fn nlua_push_Float(lstate: *mut lua_State, f: Float, flags: c_int) {
    unsafe {
        if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
            nlua_create_typed_table(lstate, 0, 1, ObjectType::kObjectTypeFloat);
            nlua_push_val_idx(lstate);
            lua_pushnumber(lstate, f);
            lua_rawset(lstate, -3);
        } else {
            lua_pushnumber(lstate, f);
        }
    }
}

/// Convert given `Boolean` to Lua boolean (`nlua_push_Boolean`).
pub unsafe extern "C-unwind" fn nlua_push_Boolean(
    lstate: *mut lua_State,
    b: Boolean,
    _flags: c_int,
) {
    unsafe {
        lua_pushboolean(lstate, c_int::from(b));
    }
}

/// Convert given `Dict` to Lua table (`nlua_push_Dict`).
pub unsafe extern "C-unwind" fn nlua_push_Dict(lstate: *mut lua_State, dict: Dict, flags: c_int) {
    unsafe {
        lua_createtable(lstate, 0, dict.size as c_int);
        if dict.size == 0 {
            nlua_pushref(lstate, (*nlua_global_refs).empty_dict_ref);
            lua_setmetatable(lstate, -2);
        }
        for i in 0..dict.size {
            let item = &mut *dict.items.add(i);
            nlua_push_String(lstate, item.key, flags);
            nlua_push_Object(lstate, &mut item.value, flags);
            lua_rawset(lstate, -3);
        }
    }
}

/// Convert given `Array` to Lua table (`nlua_push_Array`).
pub unsafe extern "C-unwind" fn nlua_push_Array(
    lstate: *mut lua_State,
    array: Array,
    flags: c_int,
) {
    unsafe {
        lua_createtable(lstate, array.size as c_int, 0);
        for i in 0..array.size {
            nlua_push_Object(lstate, array.items.add(i), flags);
            lua_rawseti(lstate, -2, (i as c_int) + 1);
        }
    }
}

/// Convert given `handle_T` to Lua number (`nlua_push_handle`).
pub unsafe extern "C-unwind" fn nlua_push_handle(
    lstate: *mut lua_State,
    item: handle_T,
    _flags: c_int,
) {
    unsafe {
        lua_pushnumber(lstate, item as lua_Number);
    }
}

/// Convert given `Object` to Lua value (`nlua_push_Object`).
pub unsafe extern "C-unwind" fn nlua_push_Object(
    lstate: *mut lua_State,
    obj: *mut Object,
    flags: c_int,
) {
    unsafe {
        let obj = &mut *obj;
        match obj.obj_type {
            ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {
                if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
                    lua_pushnil(lstate);
                } else {
                    nlua_pushref(lstate, (*nlua_global_refs).nil_ref);
                }
            }
            ObjectType::kObjectTypeLuaRef => {
                nlua_pushref(lstate, obj.data.luaref);
                if (flags & K_NLUA_PUSH_FREE_REFS) != 0 {
                    api_free_luaref(obj.data.luaref);
                    obj.data.luaref = LUA_NOREF;
                }
            }
            ObjectType::kObjectTypeBoolean => nlua_push_Boolean(lstate, obj.data.boolean, flags),
            ObjectType::kObjectTypeInteger => nlua_push_Integer(lstate, obj.data.integer, flags),
            ObjectType::kObjectTypeFloat => nlua_push_Float(lstate, obj.data.floating, flags),
            ObjectType::kObjectTypeString => nlua_push_String(lstate, obj.data.string, flags),
            ObjectType::kObjectTypeArray => nlua_push_Array(lstate, obj.data.array, flags),
            ObjectType::kObjectTypeDict => nlua_push_Dict(lstate, obj.data.dict, flags),
            ObjectType::kObjectTypeBuffer
            | ObjectType::kObjectTypeWindow
            | ObjectType::kObjectTypeTabpage => {
                nlua_push_handle(lstate, obj.data.integer as handle_T, flags);
            }
        }
    }
}

/// Convert Lua value to `String` (`nlua_pop_String`).
pub unsafe extern "C-unwind" fn nlua_pop_String(
    lstate: *mut lua_State,
    arena: *mut Arena,
    err: *mut Error,
) -> NvimString {
    unsafe {
        if lua_type(lstate, -1) != LUA_TSTRING {
            lua_pop(lstate, 1);
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected Lua string".as_ptr(),
            );
            return NvimString {
                data: std::ptr::null_mut(),
                size: 0,
            };
        }
        let mut size = 0usize;
        let data = lua_tolstring(lstate, -1, &mut size);
        assert!(!data.is_null());
        let dup = arena_memdupz(arena, data, size);
        lua_pop(lstate, 1);
        NvimString { data: dup, size }
    }
}

/// Convert Lua value to `Integer` (`nlua_pop_Integer`).
pub unsafe extern "C-unwind" fn nlua_pop_Integer(
    lstate: *mut lua_State,
    _arena: *mut Arena,
    err: *mut Error,
) -> Integer {
    unsafe {
        if lua_type(lstate, -1) != LUA_TNUMBER {
            lua_pop(lstate, 1);
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected Lua number".as_ptr(),
            );
            return 0;
        }
        let n = lua_tonumber(lstate, -1);
        lua_pop(lstate, 1);
        if n > (Integer::MAX as lua_Number)
            || n < (Integer::MIN as lua_Number)
            || ((n as Integer) as lua_Number) != n
        {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Number is not integral".as_ptr(),
            );
            return 0;
        }
        n as Integer
    }
}

/// Convert Lua value to `Boolean` (`nlua_pop_Boolean`).
pub unsafe extern "C-unwind" fn nlua_pop_Boolean(
    lstate: *mut lua_State,
    _arena: *mut Arena,
    _err: *mut Error,
) -> Boolean {
    unsafe {
        let ret = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 1);
        ret
    }
}

/// Convert Lua value to `Boolean` strictly (`nlua_pop_Boolean_strict`).
pub unsafe extern "C-unwind" fn nlua_pop_Boolean_strict(
    lstate: *mut lua_State,
    err: *mut Error,
) -> Boolean {
    unsafe {
        let mut ret = false;
        match lua_type(lstate, -1) {
            LUA_TBOOLEAN => {
                ret = lua_toboolean(lstate, -1) != 0;
            }
            LUA_TNUMBER => {
                ret = lua_tonumber(lstate, -1) != 0.0;
            }
            LUA_TNIL => {
                ret = false;
            }
            _ => {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"not a boolean".as_ptr(),
                );
            }
        }
        lua_pop(lstate, 1);
        ret
    }
}

unsafe fn nlua_check_type(
    lstate: *mut lua_State,
    err: *mut Error,
    expected_type: ObjectType,
) -> LuaTableProps {
    unsafe {
        if lua_type(lstate, -1) != LUA_TTABLE {
            if !err.is_null() {
                let name = if expected_type == ObjectType::kObjectTypeFloat {
                    c"number".as_ptr()
                } else {
                    c"table".as_ptr()
                };
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Expected Lua %s".as_ptr(),
                    name,
                );
            }
            return LuaTableProps {
                maxidx: 0,
                string_keys_num: 0,
                has_string_with_nul: false,
                table_type: ObjectType::kObjectTypeNil,
                val: 0.0,
                has_type_key: false,
            };
        }
        let mut table_props = nlua_traverse_table(lstate);

        if expected_type == ObjectType::kObjectTypeDict
            && table_props.table_type == ObjectType::kObjectTypeArray
            && table_props.maxidx == 0
            && !table_props.has_type_key
        {
            table_props.table_type = ObjectType::kObjectTypeDict;
        }

        if table_props.table_type != expected_type && !err.is_null() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected %s-like Lua table".as_ptr(),
                api_typename(expected_type),
            );
        }
        table_props
    }
}

/// Convert Lua value or typed table to `Float` (`nlua_pop_Float`).
pub unsafe extern "C-unwind" fn nlua_pop_Float(
    lstate: *mut lua_State,
    _arena: *mut Arena,
    err: *mut Error,
) -> Float {
    unsafe {
        if lua_type(lstate, -1) == LUA_TNUMBER {
            let ret = lua_tonumber(lstate, -1);
            lua_pop(lstate, 1);
            return ret;
        }
        let table_props = nlua_check_type(lstate, err, ObjectType::kObjectTypeFloat);
        lua_pop(lstate, 1);
        if table_props.table_type != ObjectType::kObjectTypeFloat {
            return 0.0;
        }
        table_props.val
    }
}

unsafe fn nlua_pop_Array_unchecked(
    lstate: *mut lua_State,
    table_props: LuaTableProps,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut ret = arena_array(arena, table_props.maxidx);
        if table_props.maxidx == 0 {
            lua_pop(lstate, 1);
            return ret;
        }
        for i in 1..=table_props.maxidx {
            lua_rawgeti(lstate, -1, i as c_int);
            let val = nlua_pop_Object(lstate, false, arena, err);
            if error_set(err) {
                lua_pop(lstate, 1);
                if arena.is_null() {
                    api_free_array(ret);
                }
                return Array {
                    size: 0,
                    capacity: 0,
                    items: std::ptr::null_mut(),
                };
            }
            assert!(ret.size < ret.capacity);
            *ret.items.add(ret.size) = val;
            ret.size += 1;
        }
        lua_pop(lstate, 1);
        ret
    }
}

/// Convert Lua table to `Array` (`nlua_pop_Array`).
pub unsafe extern "C-unwind" fn nlua_pop_Array(
    lstate: *mut lua_State,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let table_props = nlua_check_type(lstate, err, ObjectType::kObjectTypeArray);
        if table_props.table_type != ObjectType::kObjectTypeArray {
            return Array {
                size: 0,
                capacity: 0,
                items: std::ptr::null_mut(),
            };
        }
        nlua_pop_Array_unchecked(lstate, table_props, arena, err)
    }
}

unsafe fn nlua_pop_Dict_unchecked(
    lstate: *mut lua_State,
    table_props: LuaTableProps,
    ref_: bool,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut ret = arena_dict(arena, table_props.string_keys_num);
        if table_props.string_keys_num == 0 {
            lua_pop(lstate, 1);
            return ret;
        }

        lua_pushnil(lstate);
        let mut i = 0usize;
        while lua_next(lstate, -2) != 0 && i < table_props.string_keys_num {
            if lua_type(lstate, -2) == LUA_TSTRING {
                lua_pushvalue(lstate, -2);
                let key = nlua_pop_String(lstate, arena, err);
                if !error_set(err) {
                    let value = nlua_pop_Object(lstate, ref_, arena, err);
                    assert!(ret.size < ret.capacity);
                    *ret.items.add(ret.size) = KeyValuePair { key, value };
                    ret.size += 1;
                } else {
                    lua_pop(lstate, 1);
                }

                if error_set(err) {
                    if arena.is_null() {
                        api_free_dict(ret);
                    }
                    lua_pop(lstate, 2);
                    return Dict {
                        size: 0,
                        capacity: 0,
                        items: std::ptr::null_mut(),
                    };
                }
                i += 1;
            } else {
                lua_pop(lstate, 1);
            }
        }
        lua_pop(lstate, 1);
        ret
    }
}

/// Convert Lua table to `Dict` (`nlua_pop_Dict`).
pub unsafe extern "C-unwind" fn nlua_pop_Dict(
    lstate: *mut lua_State,
    ref_: bool,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let table_props = nlua_check_type(lstate, err, ObjectType::kObjectTypeDict);
        if table_props.table_type != ObjectType::kObjectTypeDict {
            lua_pop(lstate, 1);
            return Dict {
                size: 0,
                capacity: 0,
                items: std::ptr::null_mut(),
            };
        }
        nlua_pop_Dict_unchecked(lstate, table_props, ref_, arena, err)
    }
}

/// Convert Lua value to `Object` (`nlua_pop_Object`).
pub unsafe extern "C-unwind" fn nlua_pop_Object(
    lstate: *mut lua_State,
    ref_: bool,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut ret = nil_obj();
        let initial_size = lua_gettop(lstate);
        let mut stack: Vec<ObjPopStackItem> = Vec::with_capacity(2);
        stack.push(ObjPopStackItem {
            obj: &mut ret,
            container: false,
        });

        while !error_set(err) && !stack.is_empty() {
            let mut cur = stack.pop().unwrap();
            if cur.container {
                if lua_checkstack(lstate, lua_gettop(lstate) + 3) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Lua failed to grow stack".as_ptr(),
                    );
                    break;
                }
                if (*cur.obj).obj_type == ObjectType::kObjectTypeDict {
                    if (*cur.obj).data.dict.size == (*cur.obj).data.dict.capacity {
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
                        let idx = (*cur.obj).data.dict.size;
                        (*cur.obj).data.dict.size += 1;
                        let item = (*cur.obj).data.dict.items.add(idx);
                        (*item).key = cbuf_to_arena_str(arena, s, len);
                        stack.push(cur);
                        cur = ObjPopStackItem {
                            obj: &mut (*item).value,
                            container: false,
                        };
                    } else {
                        lua_pop(lstate, 1);
                        continue;
                    }
                } else {
                    if (*cur.obj).data.array.size == (*cur.obj).data.array.capacity {
                        lua_pop(lstate, 1);
                        continue;
                    }
                    let idx = (*cur.obj).data.array.size;
                    (*cur.obj).data.array.size += 1;
                    lua_rawgeti(lstate, -1, (idx as c_int) + 1);
                    let item_ptr = (*cur.obj).data.array.items.add(idx);
                    stack.push(cur);
                    cur = ObjPopStackItem {
                        obj: item_ptr,
                        container: false,
                    };
                }
            }

            *cur.obj = nil_obj();
            match lua_type(lstate, -1) {
                LUA_TNIL => {}
                LUA_TBOOLEAN => {
                    *cur.obj = Object {
                        obj_type: ObjectType::kObjectTypeBoolean,
                        data: ObjectData {
                            boolean: lua_toboolean(lstate, -1) != 0,
                        },
                    };
                }
                LUA_TSTRING => {
                    let mut len = 0usize;
                    let s = lua_tolstring(lstate, -1, &mut len);
                    *cur.obj = Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: cbuf_to_arena_str(arena, s, len),
                        },
                    };
                }
                LUA_TNUMBER => {
                    let n = lua_tonumber(lstate, -1);
                    if n > (Integer::MAX as lua_Number)
                        || n < (Integer::MIN as lua_Number)
                        || ((n as Integer) as lua_Number) != n
                    {
                        *cur.obj = Object {
                            obj_type: ObjectType::kObjectTypeFloat,
                            data: ObjectData { floating: n },
                        };
                    } else {
                        *cur.obj = Object {
                            obj_type: ObjectType::kObjectTypeInteger,
                            data: ObjectData {
                                integer: n as Integer,
                            },
                        };
                    }
                }
                LUA_TTABLE => {
                    let table_props = nlua_traverse_table(lstate);
                    match table_props.table_type {
                        ObjectType::kObjectTypeArray => {
                            *cur.obj = Object {
                                obj_type: ObjectType::kObjectTypeArray,
                                data: ObjectData {
                                    array: Array {
                                        size: 0,
                                        capacity: 0,
                                        items: std::ptr::null_mut(),
                                    },
                                },
                            };
                            if table_props.maxidx != 0 {
                                (*cur.obj).data.array = arena_array(arena, table_props.maxidx);
                                cur.container = true;
                                stack.push(cur);
                            }
                        }
                        ObjectType::kObjectTypeDict => {
                            *cur.obj = Object {
                                obj_type: ObjectType::kObjectTypeDict,
                                data: ObjectData {
                                    dict: Dict {
                                        size: 0,
                                        capacity: 0,
                                        items: std::ptr::null_mut(),
                                    },
                                },
                            };
                            if table_props.string_keys_num != 0 {
                                (*cur.obj).data.dict =
                                    arena_dict(arena, table_props.string_keys_num);
                                cur.container = true;
                                stack.push(cur);
                                lua_pushnil(lstate);
                            }
                        }
                        ObjectType::kObjectTypeFloat => {
                            *cur.obj = Object {
                                obj_type: ObjectType::kObjectTypeFloat,
                                data: ObjectData {
                                    floating: table_props.val,
                                },
                            };
                        }
                        ObjectType::kObjectTypeNil => {
                            api_set_error!(
                                err,
                                ErrorType::kErrorTypeValidation,
                                c"Cannot convert given Lua table".as_ptr(),
                            );
                        }
                        _ => std::process::abort(),
                    }
                }
                LUA_TFUNCTION => {
                    if ref_ {
                        *cur.obj = Object {
                            obj_type: ObjectType::kObjectTypeLuaRef,
                            data: ObjectData {
                                luaref: nlua_ref_global(lstate, -1),
                            },
                        };
                    } else {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"Cannot convert given Lua type".as_ptr(),
                        );
                    }
                }
                LUA_TUSERDATA => {
                    nlua_pushref(lstate, (*nlua_global_refs).nil_ref);
                    let is_nil = lua_rawequal(lstate, -2, -1) != 0;
                    lua_pop(lstate, 1);
                    if is_nil {
                        *cur.obj = nil_obj();
                    } else {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"Cannot convert userdata".as_ptr(),
                        );
                    }
                }
                _ => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Cannot convert given Lua type".as_ptr(),
                    );
                }
            }
            if !cur.container {
                lua_pop(lstate, 1);
            }
        }

        if error_set(err) {
            if arena.is_null() {
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
pub unsafe extern "C-unwind" fn nlua_pop_LuaRef(
    lstate: *mut lua_State,
    _arena: *mut Arena,
    _err: *mut Error,
) -> LuaRef {
    unsafe {
        let rv = nlua_ref_global(lstate, -1);
        lua_pop(lstate, 1);
        rv
    }
}

/// Convert Lua value to `handle_T` (`nlua_pop_handle`).
pub unsafe extern "C-unwind" fn nlua_pop_handle(
    lstate: *mut lua_State,
    _arena: *mut Arena,
    err: *mut Error,
) -> handle_T {
    unsafe {
        let ret = if lua_type(lstate, -1) != LUA_TNUMBER {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected Lua number".as_ptr(),
            );
            -1
        } else {
            lua_tonumber(lstate, -1) as handle_T
        };
        lua_pop(lstate, 1);
        ret
    }
}

/// Record auxiliary values in `vim` module (`nlua_init_types`).
pub unsafe extern "C-unwind" fn nlua_init_types(lstate: *mut lua_State) {
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
        lua_pushnumber(
            lstate,
            (ObjectType::kObjectTypeFloat as c_int) as lua_Number,
        );
        lua_rawset(lstate, -3);
        lua_pushnumber(
            lstate,
            (ObjectType::kObjectTypeFloat as c_int) as lua_Number,
        );
        lua_pushlstring(lstate, c"float".as_ptr(), 5);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"array".as_ptr(), 5);
        lua_pushnumber(
            lstate,
            (ObjectType::kObjectTypeArray as c_int) as lua_Number,
        );
        lua_rawset(lstate, -3);
        lua_pushnumber(
            lstate,
            (ObjectType::kObjectTypeArray as c_int) as lua_Number,
        );
        lua_pushlstring(lstate, c"array".as_ptr(), 5);
        lua_rawset(lstate, -3);

        lua_pushlstring(lstate, c"dictionary".as_ptr(), 10);
        lua_pushnumber(lstate, (ObjectType::kObjectTypeDict as c_int) as lua_Number);
        lua_rawset(lstate, -3);
        lua_pushnumber(lstate, (ObjectType::kObjectTypeDict as c_int) as lua_Number);
        lua_pushlstring(lstate, c"dictionary".as_ptr(), 10);
        lua_rawset(lstate, -3);

        lua_rawset(lstate, -3);
    }
}

/// Lua-specific variant of `api_dict_to_keydict` (`nlua_pop_keydict`).
pub unsafe fn nlua_pop_keydict(
    L: *mut lua_State,
    retval: *mut c_void,
    hashy: FieldHashfn,
    err_opt: *mut *mut c_char,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        if lua_type(L, -1) != LUA_TTABLE {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Expected Lua table".as_ptr(),
            );
            lua_pop(L, 1);
            return;
        }

        let hashy_fn = hashy.unwrap();
        lua_pushnil(L);
        while lua_next(L, -2) != 0 {
            let mut len = 0usize;
            let s = lua_tolstring(L, -2, &mut len);
            let field_ptr = hashy_fn(s, len);
            if field_ptr.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"invalid key: %.*s".as_ptr(),
                    len as c_int,
                    s,
                );
                lua_pop(L, 3);
                return;
            }
            let field = &*field_ptr;
            if field.opt_index >= 0 {
                let ks = retval.cast::<OptKeySet>();
                (*ks).is_set_ |= 1u64 << (field.opt_index as u32);
            }
            let mem = retval.cast::<u8>().add(field.ptr_off);

            if field.key_type == (ObjectType::kObjectTypeNil as c_int) {
                *mem.cast::<Object>() = nlua_pop_Object(L, true, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeInteger as c_int) {
                if field.is_hlgroup && lua_type(L, -1) == LUA_TSTRING {
                    let mut name_len = 0usize;
                    let name = lua_tolstring(L, -1, &mut name_len);
                    lua_pop(L, 1);
                    *mem.cast::<Integer>() = if name_len > 0 {
                        syn_check_group(name, name_len) as Integer
                    } else {
                        0
                    };
                } else {
                    *mem.cast::<Integer>() = nlua_pop_Integer(L, arena, err);
                }
            } else if field.key_type == (ObjectType::kObjectTypeBoolean as c_int) {
                *mem.cast::<Boolean>() = nlua_pop_Boolean_strict(L, err);
            } else if field.key_type == (ObjectType::kObjectTypeString as c_int) {
                *mem.cast::<NvimString>() = nlua_pop_String(L, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeFloat as c_int) {
                *mem.cast::<Float>() = nlua_pop_Float(L, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeBuffer as c_int)
                || field.key_type == (ObjectType::kObjectTypeWindow as c_int)
                || field.key_type == (ObjectType::kObjectTypeTabpage as c_int)
            {
                *mem.cast::<handle_T>() = nlua_pop_handle(L, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeArray as c_int) {
                *mem.cast::<Array>() = nlua_pop_Array(L, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeDict as c_int) {
                *mem.cast::<Dict>() = nlua_pop_Dict(L, false, arena, err);
            } else if field.key_type == (ObjectType::kObjectTypeLuaRef as c_int) {
                *mem.cast::<LuaRef>() = nlua_pop_LuaRef(L, arena, err);
            } else {
                std::process::abort();
            }

            if error_set(err) {
                *err_opt = field.str;
                break;
            }
        }
        lua_pop(L, 1);
    }
}

/// Push a `KeySet` struct as a Lua table (`nlua_push_keydict`).
pub unsafe extern "C-unwind" fn nlua_push_keydict(
    L: *mut lua_State,
    value: *mut c_void,
    table: *mut KeySetLink,
) {
    unsafe {
        lua_createtable(L, 0, 0);
        let mut i = 0usize;
        while !(*table.add(i)).str.is_null() {
            let field = &*table.add(i);
            let mut is_set = true;
            if field.opt_index >= 0 {
                let ks = value.cast::<OptKeySet>();
                is_set = ((*ks).is_set_ & (1u64 << (field.opt_index as u32))) != 0;
            }
            if !is_set {
                i += 1;
                continue;
            }

            let mem = value.cast::<u8>().add(field.ptr_off);
            lua_pushstring(L, field.str);
            if field.key_type == (ObjectType::kObjectTypeNil as c_int) {
                nlua_push_Object(L, mem.cast::<Object>(), 0);
            } else if field.key_type == (ObjectType::kObjectTypeInteger as c_int) {
                lua_pushinteger(L, *mem.cast::<Integer>() as lua_Integer);
            } else if field.key_type == (ObjectType::kObjectTypeBuffer as c_int)
                || field.key_type == (ObjectType::kObjectTypeWindow as c_int)
                || field.key_type == (ObjectType::kObjectTypeTabpage as c_int)
            {
                lua_pushinteger(L, *mem.cast::<handle_T>() as lua_Integer);
            } else if field.key_type == (ObjectType::kObjectTypeFloat as c_int) {
                lua_pushnumber(L, *mem.cast::<Float>());
            } else if field.key_type == (ObjectType::kObjectTypeBoolean as c_int) {
                lua_pushboolean(L, c_int::from(*mem.cast::<Boolean>()));
            } else if field.key_type == (ObjectType::kObjectTypeString as c_int) {
                nlua_push_String(L, *mem.cast::<NvimString>(), 0);
            } else if field.key_type == (ObjectType::kObjectTypeArray as c_int) {
                nlua_push_Array(L, *mem.cast::<Array>(), 0);
            } else if field.key_type == (ObjectType::kObjectTypeDict as c_int) {
                nlua_push_Dict(L, *mem.cast::<Dict>(), 0);
            } else if field.key_type == (ObjectType::kObjectTypeLuaRef as c_int) {
                nlua_pushref(L, *mem.cast::<LuaRef>());
            } else {
                std::process::abort();
            }

            lua_rawset(L, -3);
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_link_and_nil_obj_defaults() {
        let obj = nil_obj();
        assert_eq!(obj.obj_type, ObjectType::kObjectTypeNil);
        assert_eq!(unsafe { obj.data.integer }, 0);
    }
}
