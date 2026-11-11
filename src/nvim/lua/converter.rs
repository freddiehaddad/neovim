#![forbid(unsafe_code)]
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
use crate::memory::Arena;
use crate::message::{emsg, internal_error, semsg};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};

pub(crate) const K_NLUA_PUSH_SPECIAL: i32 = 0x01;
const K_NLUA_PUSH_FREE_REFS: i32 = 0x02;

const TYPE_IDX_VALUE: bool = true;
const VAL_IDX_VALUE: bool = false;

std::thread_local! {
    static TYPVAL_CONV_SPECIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NluaRefState {
    pub nil_ref: LuaRef,
    pub empty_dict_ref: LuaRef,
    pub ref_count: i32,
}

pub use crate::api::private::validate::{KeyDict, KeyField, KeySetLink};
pub type FieldHashfn<T> = Option<fn(key: &[u8]) -> Option<&'static KeySetLink<T>>>;

#[derive(Clone, Copy)]
struct LuaTableProps {
    maxidx: usize,
    string_keys_num: usize,
    has_string_with_nul: bool,
    table_type: ObjectType,
    val: lua_Number,
    has_type_key: bool,
}

#[derive(Clone)]
enum TVPopTarget {
    Root,
    ListItem(crate::eval::typval::ListitemId),
    DictItem(crate::eval::typval::DictitemId),
}

#[derive(Clone)]
struct TVPopStackItem {
    tv: TVPopTarget,
    list_len: usize,
    container: bool,
    special: bool,
    idx: i32,
}

struct ObjPopStackItem {
    obj: Object,
    next_key: Option<NvimString>,
}

#[inline]
fn nlua_push_type_idx(lstate: *mut lua_State) {
    lua_pushboolean_safe(lstate, TYPE_IDX_VALUE);
}

#[inline]
fn nlua_push_val_idx(lstate: *mut lua_State) {
    lua_pushboolean_safe(lstate, VAL_IDX_VALUE);
}

#[inline]
fn nlua_push_type(lstate: *mut lua_State, obj_type: ObjectType) {
    lua_pushnumber_safe(lstate, (obj_type as i32) as lua_Number);
}

#[inline]
fn nlua_create_typed_table(lstate: *mut lua_State, narr: usize, nrec: usize, obj_type: ObjectType) {
    lua_createtable_safe(lstate, narr as i32, (1 + nrec) as i32);
    nlua_push_type_idx(lstate);
    nlua_push_type(lstate, obj_type);
    lua_rawset_safe(lstate, -3);
}

fn nlua_traverse_table(lstate: *mut lua_State) -> LuaTableProps {
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

    if !lua_checkstack_safe(lstate, 3) {
        semsg!(
            c"E1502: Lua failed to grow stack to %i",
            nlua_gettop(lstate) + 3,
        );
        ret.table_type = ObjectType::ObjectTypeNil;
        return ret;
    }

    lua_pushnil_safe(lstate);
    while lua_next_safe(lstate, -2) != 0 {
        match lua_type_safe(lstate, -2) {
            LUA_TSTRING => {
                let slice = lua_tolstring_bytes(lstate, -2);
                if slice.contains(&0) {
                    ret.has_string_with_nul = true;
                }
                ret.string_keys_num += 1;
            }
            LUA_TNUMBER => {
                let n = lua_tonumber_safe(lstate, -2);
                if n > (usize::MAX as lua_Number) || n <= 0.0 || ((n as usize) as lua_Number) != n {
                    other_keys_num += 1;
                } else {
                    let idx = n as usize;
                    if idx > ret.maxidx {
                        ret.maxidx = idx;
                    }
                }
            }
            LUA_TBOOLEAN => {
                let b = lua_toboolean_safe(lstate, -2);
                if b == TYPE_IDX_VALUE {
                    if lua_type_safe(lstate, -1) == LUA_TNUMBER {
                        let n = lua_tonumber_safe(lstate, -1);
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
                    val_type = lua_type_safe(lstate, -1);
                    if val_type == LUA_TNUMBER {
                        ret.val = lua_tonumber_safe(lstate, -1);
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
                    lua_rawgeti_safe(lstate, -1, (ret.maxidx as i32) + 1);
                    if lua_type_safe(lstate, -1) == LUA_TNIL {
                        lua_pop(lstate, 1);
                        break;
                    }
                    lua_pop(lstate, 1);
                    ret.maxidx += 1;
                }
            }
        }
    } else if tsize == 0 || (tsize <= ret.maxidx && other_keys_num == 0 && ret.string_keys_num == 0)
    {
        ret.table_type = ObjectType::ObjectTypeArray;
        if tsize == 0 && lua_getmetatable_safe(lstate, -1) {
            nlua_pushref(lstate, crate::globals::nlua_global_refs().empty_dict_ref);
            if lua_rawequal_safe(lstate, -2, -1) {
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

/// Convert Lua object to Vimscript `TypVal` (`nlua_pop_typval`).
pub fn nlua_pop_typval(lstate: *mut lua_State, ret_tv: &mut TypVal) -> bool {
    let mut ret = true;
    let initial_size = nlua_gettop(lstate);
    let mut stack: Vec<TVPopStackItem> = Vec::with_capacity(2);
    stack.push(TVPopStackItem {
        tv: TVPopTarget::Root,
        list_len: 0,
        container: false,
        special: false,
        idx: 0,
    });

    while ret && !stack.is_empty() {
        if !lua_checkstack_safe(lstate, 3) {
            semsg!(
                c"E1502: Lua failed to grow stack to %i",
                nlua_gettop(lstate) + 3,
            );
            ret = false;
            break;
        }
        let mut cur = stack.pop().unwrap();
        if cur.container {
            if cur.special || cur.tv_ref(ret_tv).v_type == VarType::VarDict {
                let mut next_key_found = false;
                while lua_next_safe(lstate, -2) != 0 {
                    if lua_type_safe(lstate, -2) == LUA_TSTRING {
                        next_key_found = true;
                        break;
                    }
                    lua_pop(lstate, 1);
                }
                if next_key_found {
                    let slice = lua_tolstring_bytes(lstate, -2);
                    if cur.special {
                        let kv_pair = tv_list_alloc(2);
                        let s_tv = decode_string(slice, true);
                        tv_list_append_owned_tv(&kv_pair, s_tv);
                        tv_list_append_owned_tv(
                            &kv_pair,
                            TypVal {
                                v_type: VarType::VarUnknown,
                                v_lock: VAR_UNLOCKED,
                                vval: TypvalVval::Number(0),
                            },
                        );
                        tv_list_append_list(&cur.tv_mut(ret_tv).vval.v_list(), Some(&kv_pair));
                        stack.push(cur);
                        let last = tv_list_last(Some(&kv_pair)).unwrap();
                        cur = TVPopStackItem {
                            tv: TVPopTarget::ListItem(last),
                            list_len: 0,
                            container: false,
                            special: false,
                            idx: 0,
                        };
                    } else {
                        let di = tv_dict_item_alloc_len(slice);
                        let di_id = di.di_id;
                        if !tv_dict_add(cur.tv_mut(ret_tv).vval.v_dict().borrow_mut(), di) {
                            std::process::abort();
                        }
                        stack.push(cur);
                        cur = TVPopStackItem {
                            tv: TVPopTarget::DictItem(di_id),
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
                let list = cur.tv_ref(ret_tv).vval.v_list();
                if (tv_list_len(list.opt()) as usize) == cur.list_len {
                    lua_pop(lstate, 1);
                    continue;
                }
                lua_rawgeti_safe(lstate, -1, tv_list_len(list.opt()) + 1);
                tv_list_append_owned_tv(
                    &list,
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    },
                );
                stack.push(cur);
                let last = tv_list_last(list.opt()).unwrap();
                cur = TVPopStackItem {
                    tv: TVPopTarget::ListItem(last),
                    list_len: 0,
                    container: false,
                    special: false,
                    idx: 0,
                };
            }
        }

        *cur.tv_mut(ret_tv) = TypVal {
            v_type: VarType::VarNumber,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };

        match lua_type_safe(lstate, -1) {
            LUA_TNIL => {
                cur.tv_mut(ret_tv).v_type = VarType::VarSpecial;
                cur.tv_mut(ret_tv).vval =
                    crate::eval::typval::TypvalVval::Number(K_SPECIAL_VAR_NULL);
            }
            LUA_TBOOLEAN => {
                cur.tv_mut(ret_tv).v_type = VarType::VarBool;
                cur.tv_mut(ret_tv).vval =
                    crate::eval::typval::TypvalVval::Number(if lua_toboolean_safe(lstate, -1) {
                        K_BOOL_VAR_TRUE
                    } else {
                        K_BOOL_VAR_FALSE
                    });
            }
            LUA_TSTRING => {
                let slice = lua_tolstring_bytes(lstate, -1);
                *cur.tv_mut(ret_tv) = decode_string(slice, false);
            }
            LUA_TNUMBER => {
                let n = lua_tonumber_safe(lstate, -1);
                if n > (i64::MAX as lua_Number)
                    || n < (i64::MIN as lua_Number)
                    || ((n as i64) as lua_Number) != n
                {
                    cur.tv_mut(ret_tv).v_type = VarType::VarFloat;
                    cur.tv_mut(ret_tv).vval = crate::eval::typval::TypvalVval::Float(n);
                } else {
                    cur.tv_mut(ret_tv).v_type = VarType::VarNumber;
                    cur.tv_mut(ret_tv).vval = crate::eval::typval::TypvalVval::Number(n as i64);
                }
            }
            LUA_TTABLE => {
                let mut table_ref = LUA_NOREF;
                if lua_getmetatable_safe(lstate, -1) {
                    lua_pop(lstate, 1);
                    table_ref = nlua_ref_global(lstate, -1);
                }

                let table_props = nlua_traverse_table(lstate);

                let mut found_self_ref = false;
                for item in &stack {
                    if item.container && lua_rawequal_safe(lstate, -1, item.idx) {
                        let src_tv = item.tv_ref(ret_tv).clone();
                        tv_copy(&src_tv, &mut cur.tv_mut(ret_tv));
                        cur.container = false;
                        found_self_ref = true;
                        break;
                    }
                }

                if !found_self_ref {
                    match table_props.table_type {
                        ObjectType::ObjectTypeArray => {
                            let l = tv_list_alloc(table_props.maxidx as isize);
                            l.lua_table_ref.set(table_ref);
                            tv_list_ref(Some(&l));
                            cur.tv_mut(ret_tv).v_type = VarType::VarList;
                            cur.tv_mut(ret_tv).vval = crate::eval::typval::TypvalVval::List(l);
                            cur.list_len = table_props.maxidx;
                            if table_props.maxidx != 0 {
                                cur.container = true;
                                cur.idx = nlua_gettop(lstate);
                                stack.push(cur.clone());
                            }
                        }
                        ObjectType::ObjectTypeDict => {
                            if table_props.string_keys_num == 0 {
                                let d = tv_dict_alloc();
                                d.dv_refcount += 1;
                                d.lua_table_ref = table_ref;
                                cur.tv_mut(ret_tv).v_type = VarType::VarDict;
                                cur.tv_mut(ret_tv).vval =
                                    crate::eval::typval::TypvalVval::Dict(d.dv_id);
                            } else {
                                cur.special = table_props.has_string_with_nul;
                                if table_props.has_string_with_nul {
                                    decode_create_map_special_dict(
                                        &mut cur.tv_mut(ret_tv),
                                        table_props.string_keys_num as isize,
                                    );
                                    let val_di = tv_dict_find(
                                        cur.tv_ref(ret_tv).vval.v_dict().opt(),
                                        c"_VAL",
                                        4,
                                    )
                                    .unwrap();
                                    cur.tv = TVPopTarget::DictItem(val_di.di_id);
                                    cur.tv_mut(ret_tv)
                                        .vval
                                        .v_list()
                                        .lua_table_ref
                                        .set(table_ref);
                                    cur.list_len = table_props.string_keys_num;
                                } else {
                                    let d = tv_dict_alloc();
                                    d.dv_refcount += 1;
                                    d.lua_table_ref = table_ref;
                                    cur.tv_mut(ret_tv).v_type = VarType::VarDict;
                                    cur.tv_mut(ret_tv).vval =
                                        crate::eval::typval::TypvalVval::Dict(d.dv_id);
                                }
                                cur.container = true;
                                cur.idx = nlua_gettop(lstate);
                                stack.push(cur.clone());
                                lua_pushnil_safe(lstate);
                            }
                        }
                        ObjectType::ObjectTypeFloat => {
                            cur.tv_mut(ret_tv).v_type = VarType::VarFloat;
                            cur.tv_mut(ret_tv).vval =
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
                crate::eval::typval::tv_set_string(&mut cur.tv_mut(ret_tv), Some(&name));
                cur.tv_mut(ret_tv).v_type = VarType::VarFunc;
            }
            LUA_TUSERDATA => {
                nlua_pushref(lstate, crate::globals::nlua_global_refs().nil_ref);
                let is_nil = lua_rawequal_safe(lstate, -2, -1);
                lua_pop(lstate, 1);
                if is_nil {
                    cur.tv_mut(ret_tv).v_type = VarType::VarSpecial;
                    cur.tv_mut(ret_tv).vval =
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
        lua_pop(lstate, nlua_gettop(lstate) - initial_size + 1);
    }
    assert_eq!(nlua_gettop(lstate), initial_size - 1);
    ret
}

#[inline]
fn lua_conv_nil(lstate: *mut lua_State) {
    if TYPVAL_CONV_SPECIAL.get() {
        lua_pushnil_safe(lstate);
    } else {
        nlua_pushref(lstate, crate::globals::nlua_global_refs().nil_ref);
    }
}

#[inline]
fn lua_conv_empty_dict(lstate: *mut lua_State) {
    if TYPVAL_CONV_SPECIAL.get() {
        nlua_create_typed_table(lstate, 0, 0, ObjectType::ObjectTypeDict);
    } else {
        lua_createtable_safe(lstate, 0, 0);
        nlua_pushref(lstate, crate::globals::nlua_global_refs().empty_dict_ref);
        let _ = lua_setmetatable_safe(lstate, -2);
    }
}

#[inline]
fn lua_conv_list_start(lstate: *mut lua_State, len: i32) {
    lua_createtable_safe(lstate, len, 0);
    lua_pushnumber_safe(lstate, 1.0);
}

#[inline]
fn lua_conv_dict_start(lstate: *mut lua_State, len: usize) {
    lua_createtable_safe(lstate, 0, len as i32);
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
        let mpval = &mpstack[backref - 1];
        let matched = match (&container, &mpval.data) {
            (MPContainerRef::Dict(d1), MPConvStackData::Dict { dict: d2, .. }) => d1 == d2,
            (MPContainerRef::List(l1), MPConvStackData::List { list: l2, .. }) => l1 == l2,
            (MPContainerRef::Pairs(l1), MPConvStackData::Pairs { list: l2, .. }) => l1 == l2,
            _ => false,
        };
        if matched {
            let stack_offset = -(((mpstack.len() - backref + 1) * 2) as i32);
            lua_pushvalue_safe(lstate, stack_offset);
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
    if !lua_checkstack_safe(lstate, 3) {
        semsg!(
            c"E5102: Lua failed to grow stack to %i",
            nlua_gettop(lstate) + 3,
        );
        return false;
    }
    let tv_ref = tv;
    match tv_ref.v_type {
        VarType::VarString => {
            let s_bytes = tv_ref.vval.v_string_bytes();
            lua_push_bytes(lstate, s_bytes);
        }
        VarType::VarNumber => {
            lua_pushnumber_safe(lstate, tv_ref.vval.v_number() as lua_Number);
        }
        VarType::VarFloat => {
            lua_pushnumber_safe(lstate, tv_ref.vval.v_float());
        }
        VarType::VarBlob => {
            let blob = tv_ref.vval.v_blob();
            let len = tv_blob_len(blob.opt()) as usize;
            if let Some(b) = blob.opt()
                && !b.bv_ga.borrow().is_empty()
            {
                lua_push_bytes(lstate, &b.bv_ga.borrow()[..len]);
            } else {
                lua_push_bytes(lstate, &[]);
            }
        }
        VarType::VarFunc => {
            if let Some(fun) = tv_ref.vval.v_string_cstr() {
                if let Some(fp) = find_func(Some(fun))
                    && (fp.uf_flags.get() & FC_LUAREF) != 0
                {
                    nlua_pushref(lstate, fp.uf_luaref.get());
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
                    && (fp.uf_flags.get() & FC_LUAREF) != 0
                {
                    nlua_pushref(lstate, fp.uf_luaref.get());
                } else {
                    lua_conv_nil(lstate);
                }
            } else {
                lua_conv_nil(lstate);
            }
        }
        VarType::VarList => {
            let list = tv_ref.vval.v_list();
            if list.is_null() || tv_list_len(list.opt()) == 0 {
                lua_createtable_safe(lstate, 0, 0);
            } else {
                let saved_copy_id = list.lv_copy_id.get();
                let mut copy_id_val = saved_copy_id;
                let self_ref = lua_check_self_reference(
                    lstate,
                    MPContainerRef::List(list.clone()),
                    &mut copy_id_val,
                    mpstack,
                    copy_id,
                );
                list.lv_copy_id.set(copy_id_val);
                if self_ref {
                    return true;
                }
                lua_conv_list_start(lstate, tv_list_len(list.opt()));
                mpstack.push(MPConvStackVal {
                    saved_copy_id,
                    data: MPConvStackData::List {
                        li: tv_list_first(list.opt()).into(),
                        list,
                    },
                });
            }
        }
        VarType::VarBool => {
            lua_pushboolean_safe(lstate, tv_ref.vval.v_bool() == K_BOOL_VAR_TRUE as i32);
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
                && let Some(type_di) = tv_dict_find(dict.opt(), c"_TYPE", 5)
                && type_di.di_tv.v_type == VarType::VarList
                && let Some(val_di) = tv_dict_find(dict.opt(), c"_VAL", 4)
            {
                let mut mp_idx = NUM_MSGPACK_TYPES;
                for i in 0..NUM_MSGPACK_TYPES {
                    if type_di.di_tv.vval.v_list() == eval_msgpack_type_list(i) {
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
                                lua_pushboolean_safe(lstate, val_di.di_tv.vval.v_number() != 0);
                                handled_special = true;
                            }
                        }
                        K_MP_INTEGER => {
                            if val_di.di_tv.v_type == VarType::VarList {
                                let val_list = val_di.di_tv.vval.v_list();
                                if tv_list_len(val_list.opt()) == 4 {
                                    let sign_li = tv_list_first(val_list.opt()).unwrap();
                                    let highest_li = sign_li.li_next.get();
                                    let high_li = highest_li.li_next.get();
                                    let low_li = tv_list_last(val_list.opt()).unwrap();
                                    let sign_tv = sign_li.tv();
                                    let highest_tv = highest_li.tv();
                                    let high_tv = high_li.tv();
                                    let low_tv = low_li.tv();
                                    if sign_tv.v_type == VarType::VarNumber
                                        && sign_tv.vval.v_number() != 0
                                        && highest_tv.v_type == VarType::VarNumber
                                        && highest_tv.vval.v_number() >= 0
                                        && high_tv.v_type == VarType::VarNumber
                                        && high_tv.vval.v_number() >= 0
                                        && low_tv.v_type == VarType::VarNumber
                                        && low_tv.vval.v_number() >= 0
                                    {
                                        let sign = sign_tv.vval.v_number();
                                        let highest = highest_tv.vval.v_number() as u64;
                                        let high = high_tv.vval.v_number() as u64;
                                        let low = low_tv.vval.v_number() as u64;
                                        let number = (highest << 62) | (high << 31) | low;
                                        if sign > 0 {
                                            lua_pushnumber_safe(lstate, number as lua_Number);
                                        } else {
                                            lua_pushnumber_safe(
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
                                lua_pushnumber_safe(lstate, val_di.di_tv.vval.v_float());
                                handled_special = true;
                            }
                        }
                        K_MP_STRING => {
                            if val_di.di_tv.v_type == VarType::VarList
                                && let Some(buf) =
                                    encode_vim_list_to_buf(val_di.di_tv.vval.v_list().opt())
                            {
                                lua_push_bytes(lstate, &buf);
                                handled_special = true;
                            }
                        }
                        K_MP_ARRAY => {
                            if val_di.di_tv.v_type == VarType::VarList {
                                let val_list = val_di.di_tv.vval.v_list();
                                let saved_copy_id = val_list.lv_copy_id.get();
                                let mut copy_id_val = saved_copy_id;
                                let self_ref = lua_check_self_reference(
                                    lstate,
                                    MPContainerRef::List(val_list.clone()),
                                    &mut copy_id_val,
                                    mpstack,
                                    copy_id,
                                );
                                val_list.lv_copy_id.set(copy_id_val);
                                if self_ref {
                                    return true;
                                }
                                lua_conv_list_start(lstate, tv_list_len(val_list.opt()));
                                mpstack.push(MPConvStackVal {
                                    saved_copy_id,
                                    data: MPConvStackData::List {
                                        li: tv_list_first(val_list.opt()).into(),
                                        list: val_list,
                                    },
                                });
                                handled_special = true;
                            }
                        }
                        K_MP_MAP => {
                            if val_di.di_tv.v_type == VarType::VarList {
                                let val_list = val_di.di_tv.vval.v_list();
                                if val_list.is_null() || tv_list_len(val_list.opt()) == 0 {
                                    lua_conv_empty_dict(lstate);
                                    handled_special = true;
                                } else {
                                    let mut all_pairs = true;
                                    let mut li = crate::eval::typval::ListitemId::from(
                                        tv_list_first(val_list.opt()),
                                    );
                                    while !li.is_null() {
                                        if li.tv().v_type != VarType::VarList
                                            || tv_list_len((li.tv().vval.v_list()).opt()) != 2
                                        {
                                            all_pairs = false;
                                            break;
                                        }
                                        li = li.li_next.get();
                                    }
                                    if all_pairs {
                                        let saved_copy_id = val_list.lv_copy_id.get();
                                        let mut copy_id_val = saved_copy_id;
                                        let self_ref = lua_check_self_reference(
                                            lstate,
                                            MPContainerRef::Pairs(val_list.clone()),
                                            &mut copy_id_val,
                                            mpstack,
                                            copy_id,
                                        );
                                        val_list.lv_copy_id.set(copy_id_val);
                                        if self_ref {
                                            return true;
                                        }
                                        lua_conv_dict_start(
                                            lstate,
                                            tv_list_len(val_list.opt()) as usize,
                                        );
                                        mpstack.push(MPConvStackVal {
                                            saved_copy_id,
                                            data: MPConvStackData::Pairs {
                                                li: tv_list_first(val_list.opt()).into(),
                                                list: val_list,
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
                                let first_tv = first.tv();
                                let last_tv = last.tv();
                                if first_tv.v_type == VarType::VarNumber
                                    && (i8::MIN as i64..=i8::MAX as i64)
                                        .contains(&first_tv.vval.v_number())
                                    && last_tv.v_type == VarType::VarList
                                    && encode_vim_list_to_buf(last_tv.vval.v_list().opt()).is_some()
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
                    saved_copy_id,
                    data: MPConvStackData::Dict {
                        dict,
                        hi_idx: 0,
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

fn encode_vim_to_lua(lstate: *mut lua_State, top_tv: &mut TypVal) -> bool {
    let copy_id = get_copy_id();
    let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);

    if !lua_convert_one_value(lstate, &mut mpstack, top_tv, copy_id) {
        return false;
    }

    while !mpstack.is_empty() {
        let cur_idx = mpstack.len() - 1;
        let saved_copy_id = mpstack[cur_idx].saved_copy_id;
        let next_tv: TypVal = match mpstack[cur_idx].data {
            MPConvStackData::Dict {
                mut dict,
                ref mut hi_idx,
                ref mut todo,
                ..
            } => {
                if *todo == 0 {
                    mpstack.pop();
                    dict.dv_copy_id = saved_copy_id;
                    lua_rawset_safe(lstate, -3);
                    continue;
                } else if *todo != dict.dv_hashtab.ht_used {
                    lua_rawset_safe(lstate, -3);
                }
                while hashitem_empty(&dict.dv_hashtab.as_slice()[*hi_idx]) {
                    *hi_idx += 1;
                }
                let hi = dict.dv_hashtab.as_slice()[*hi_idx];
                let di = crate::eval::typval::tv_dict_hi2di(&hi);
                *todo -= 1;
                *hi_idx += 1;
                lua_push_bytes(lstate, di.key_bytes());
                di.di_tv.clone()
            }
            MPConvStackData::List {
                ref list,
                ref mut li,
            } => {
                let cur_li = li.clone();
                if cur_li.is_null() {
                    let list = list.clone();
                    mpstack.pop();
                    list.lv_copy_id.set(saved_copy_id);
                    lua_rawset_safe(lstate, -3);
                    continue;
                } else if cur_li != crate::eval::typval::ListitemId::from(tv_list_first(list.opt()))
                {
                    let idx = lua_tonumber_safe(lstate, -2);
                    lua_rawset_safe(lstate, -3);
                    lua_pushnumber_safe(lstate, idx + 1.0);
                }
                *li = cur_li.li_next.get();
                cur_li.tv().clone()
            }
            MPConvStackData::Pairs { ref list, ref li } => {
                let li = li.clone();
                if li.is_null() {
                    let list = list.clone();
                    mpstack.pop();
                    list.lv_copy_id.set(saved_copy_id);
                    lua_rawset_safe(lstate, -3);
                    continue;
                } else if li != crate::eval::typval::ListitemId::from(tv_list_first(list.opt())) {
                    lua_rawset_safe(lstate, -3);
                }
                let kv_pair = li.tv().vval.v_list();
                let first_item = tv_list_first(kv_pair.opt()).unwrap();
                let mut first_tv = first_item.tv().clone();
                if !lua_convert_one_value(lstate, &mut mpstack, &mut first_tv, copy_id) {
                    return false;
                }
                let last_item = tv_list_last(kv_pair.opt()).unwrap();
                let next_li = li.li_next.get();
                if let MPConvStackData::Pairs {
                    li: ref mut li_ref, ..
                } = mpstack[cur_idx].data
                {
                    *li_ref = next_li;
                }
                last_item.tv().clone()
            }
            MPConvStackData::Partial { .. } | MPConvStackData::PartialList { .. } => {
                unreachable!()
            }
        };

        let mut next_tv = next_tv;
        if !lua_convert_one_value(lstate, &mut mpstack, &mut next_tv, copy_id) {
            return false;
        }
    }
    true
}

/// Convert Vimscript `TypVal` to Lua value (`nlua_push_typval`).
pub fn nlua_push_typval(lstate: *mut lua_State, tv: &mut TypVal, flags: i32) -> bool {
    TYPVAL_CONV_SPECIAL.set((flags & K_NLUA_PUSH_SPECIAL) != 0);
    let initial_size = nlua_gettop(lstate);

    if !encode_vim_to_lua(lstate, tv) {
        lua_settop_safe(lstate, initial_size);
        lua_pushnil_safe(lstate);
        return false;
    }
    assert_eq!(nlua_gettop(lstate), initial_size + 1);
    true
}

/// Convert given `String` to Lua string (`nlua_push_String`).
pub fn nlua_push_string(lstate: *mut lua_State, s: NvimString, _flags: i32) {
    lua_push_bytes(lstate, s.as_bytes());
}

/// Convert given `Integer` to Lua number (`nlua_push_Integer`).
pub fn nlua_push_integer(lstate: *mut lua_State, n: Integer, _flags: i32) {
    lua_pushnumber_safe(lstate, n as lua_Number);
}

/// Convert given `Float` to Lua number or typed table (`nlua_push_Float`).
pub fn nlua_push_float(lstate: *mut lua_State, f: Float, flags: i32) {
    if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
        nlua_create_typed_table(lstate, 0, 1, ObjectType::ObjectTypeFloat);
        nlua_push_val_idx(lstate);
        lua_pushnumber_safe(lstate, f);
        lua_rawset_safe(lstate, -3);
    } else {
        lua_pushnumber_safe(lstate, f);
    }
}

/// Convert given `Boolean` to Lua boolean (`nlua_push_Boolean`).
pub fn nlua_push_boolean(lstate: *mut lua_State, b: Boolean, _flags: i32) {
    lua_pushboolean_safe(lstate, b);
}

/// Convert given `Dict` to Lua table (`nlua_push_Dict`).
pub fn nlua_push_dict(lstate: *mut lua_State, mut dict: Dict, flags: i32) {
    lua_createtable_safe(lstate, 0, dict.size as i32);
    if dict.size == 0 {
        nlua_pushref(lstate, crate::globals::nlua_global_refs().empty_dict_ref);
        let _ = lua_setmetatable_safe(lstate, -2);
    }
    for item in dict.as_mut_slice() {
        nlua_push_string(lstate, item.key.clone(), flags);
        nlua_push_object(lstate, &mut item.value, flags);
        lua_rawset_safe(lstate, -3);
    }
}

/// Convert given `Array` to Lua table (`nlua_push_Array`).
pub fn nlua_push_array(lstate: *mut lua_State, mut array: Array, flags: i32) {
    lua_createtable_safe(lstate, array.size as i32, 0);
    for (i, item) in array.as_mut_slice().iter_mut().enumerate() {
        nlua_push_object(lstate, item, flags);
        lua_rawseti_safe(lstate, -2, (i as i32) + 1);
    }
}

/// Convert given `ObjHandle` to Lua number (`nlua_push_handle`).
pub fn nlua_push_handle(lstate: *mut lua_State, item: ObjHandle, _flags: i32) {
    lua_pushnumber_safe(lstate, item as lua_Number);
}

/// Convert given `Object` to Lua value (`nlua_push_Object`).
pub fn nlua_push_object(lstate: *mut lua_State, obj: &mut Object, flags: i32) {
    match obj.obj_type {
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {
            if (flags & K_NLUA_PUSH_SPECIAL) != 0 {
                lua_pushnil_safe(lstate);
            } else {
                nlua_pushref(lstate, crate::globals::nlua_global_refs().nil_ref);
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
    if lua_type_safe(lstate, -1) != LUA_TSTRING {
        lua_pop(lstate, 1);
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua string",);
        return NvimString::new();
    }
    let slice = lua_tolstring_bytes(lstate, -1);
    let ret = crate::api::private::helpers::arena_bytes_opt(arena, slice);
    lua_pop(lstate, 1);
    ret
}

/// Convert Lua value to `Integer` (`nlua_pop_Integer`).
pub fn nlua_pop_integer(lstate: *mut lua_State, _arena: &mut Arena, err: &mut Error) -> Integer {
    if lua_type_safe(lstate, -1) != LUA_TNUMBER {
        lua_pop(lstate, 1);
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua number",);
        return 0;
    }
    let n = lua_tonumber_safe(lstate, -1);
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
    let ret = lua_toboolean_safe(lstate, -1);
    lua_pop(lstate, 1);
    ret
}

/// Convert Lua value to `Boolean` strictly (`nlua_pop_Boolean_strict`).
pub fn nlua_pop_boolean_strict(lstate: *mut lua_State, err: &mut Error) -> Boolean {
    let mut ret = false;
    match lua_type_safe(lstate, -1) {
        LUA_TBOOLEAN => {
            ret = lua_toboolean_safe(lstate, -1);
        }
        LUA_TNUMBER => {
            ret = lua_tonumber_safe(lstate, -1) != 0.0;
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
    if lua_type_safe(lstate, -1) != LUA_TTABLE {
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
    if lua_type_safe(lstate, -1) == LUA_TNUMBER {
        let ret = lua_tonumber_safe(lstate, -1);
        lua_pop(lstate, 1);
        return ret;
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
        lua_rawgeti_safe(lstate, -1, i as i32);
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

    lua_pushnil_safe(lstate);
    let mut i = 0usize;
    while lua_next_safe(lstate, -2) != 0 && i < table_props.string_keys_num {
        if lua_type_safe(lstate, -2) == LUA_TSTRING {
            lua_pushvalue_safe(lstate, -2);
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

#[inline]
fn attach_popped_object(stack: &mut [ObjPopStackItem], ret: &mut Object, val: Object) {
    if let Some(parent) = stack.last_mut() {
        if parent.obj.obj_type == ObjectType::ObjectTypeDict {
            let idx = parent.obj.data.dict().size;
            parent.obj.data.dict_mut().size += 1;
            let item = &mut parent.obj.data.dict_mut()[idx];
            item.key = parent.next_key.take().unwrap();
            item.value = val;
        } else {
            let idx = parent.obj.data.array().size;
            parent.obj.data.array_mut().size += 1;
            parent.obj.data.array_mut()[idx] = val;
        }
    } else {
        *ret = val;
    }
}

pub fn nlua_pop_object_opt(
    lstate: *mut lua_State,
    ref_: bool,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    let mut ret = nil_obj();
    let initial_size = nlua_gettop(lstate);
    let mut stack: Vec<ObjPopStackItem> = Vec::with_capacity(2);
    let mut need_pop_value = true;

    while !error_set(err) && (need_pop_value || !stack.is_empty()) {
        if !need_pop_value {
            if !lua_checkstack_safe(lstate, 3) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Lua failed to grow stack",
                );
                break;
            }
            let cur = stack.last_mut().unwrap();
            if cur.obj.obj_type == ObjectType::ObjectTypeDict {
                if cur.obj.data.dict().size == cur.obj.data.dict().capacity {
                    lua_pop(lstate, 2);
                    let finished = stack.pop().unwrap().obj;
                    attach_popped_object(&mut stack, &mut ret, finished);
                    continue;
                }
                let mut next_key_found = false;
                while lua_next_safe(lstate, -2) != 0 {
                    if lua_type_safe(lstate, -2) == LUA_TSTRING {
                        next_key_found = true;
                        break;
                    }
                    lua_pop(lstate, 1);
                }
                if next_key_found {
                    let s_bytes = lua_tolstring_bytes(lstate, -2);
                    cur.next_key = Some(cbuf_to_arena_str_opt(arena.as_deref_mut(), s_bytes));
                } else {
                    lua_pop(lstate, 1);
                    let finished = stack.pop().unwrap().obj;
                    attach_popped_object(&mut stack, &mut ret, finished);
                    continue;
                }
            } else {
                if cur.obj.data.array().size == cur.obj.data.array().capacity {
                    lua_pop(lstate, 1);
                    let finished = stack.pop().unwrap().obj;
                    attach_popped_object(&mut stack, &mut ret, finished);
                    continue;
                }
                let idx = cur.obj.data.array().size;
                lua_rawgeti_safe(lstate, -1, (idx as i32) + 1);
            }
        }

        need_pop_value = false;
        let mut cur_obj = nil_obj();
        let mut pushed_container = false;
        match lua_type_safe(lstate, -1) {
            LUA_TNIL => {}
            LUA_TBOOLEAN => {
                cur_obj = Object {
                    obj_type: ObjectType::ObjectTypeBoolean,
                    data: ObjectData::Boolean(lua_toboolean_safe(lstate, -1)),
                };
            }
            LUA_TSTRING => {
                let s_bytes = lua_tolstring_bytes(lstate, -1);
                cur_obj = Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cbuf_to_arena_str_opt(arena.as_deref_mut(), s_bytes)),
                };
            }
            LUA_TNUMBER => {
                let n = lua_tonumber_safe(lstate, -1);
                if n > (Integer::MAX as lua_Number)
                    || n < (Integer::MIN as lua_Number)
                    || ((n as Integer) as lua_Number) != n
                {
                    cur_obj = Object {
                        obj_type: ObjectType::ObjectTypeFloat,
                        data: ObjectData::Floating(n),
                    };
                } else {
                    cur_obj = Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(n as Integer),
                    };
                }
            }
            LUA_TTABLE => {
                let table_props = nlua_traverse_table(lstate);
                match table_props.table_type {
                    ObjectType::ObjectTypeArray => {
                        if table_props.maxidx != 0 {
                            stack.push(ObjPopStackItem {
                                obj: Object {
                                    obj_type: ObjectType::ObjectTypeArray,
                                    data: ObjectData::Array(arena_array_opt(
                                        arena.as_deref_mut(),
                                        table_props.maxidx,
                                    )),
                                },
                                next_key: None,
                            });
                            pushed_container = true;
                        } else {
                            cur_obj = Object {
                                obj_type: ObjectType::ObjectTypeArray,
                                data: ObjectData::Array(Array::new()),
                            };
                        }
                    }
                    ObjectType::ObjectTypeDict => {
                        if table_props.string_keys_num != 0 {
                            stack.push(ObjPopStackItem {
                                obj: Object {
                                    obj_type: ObjectType::ObjectTypeDict,
                                    data: ObjectData::Dict(arena_dict_opt(
                                        arena.as_deref_mut(),
                                        table_props.string_keys_num,
                                    )),
                                },
                                next_key: None,
                            });
                            pushed_container = true;
                            lua_pushnil_safe(lstate);
                        } else {
                            cur_obj = Object {
                                obj_type: ObjectType::ObjectTypeDict,
                                data: ObjectData::Dict(Dict::new()),
                            };
                        }
                    }
                    ObjectType::ObjectTypeFloat => {
                        cur_obj = Object {
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
                    cur_obj = Object {
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
                nlua_pushref(lstate, crate::globals::nlua_global_refs().nil_ref);
                let is_nil = lua_rawequal_safe(lstate, -2, -1);
                lua_pop(lstate, 1);
                if is_nil {
                    cur_obj = nil_obj();
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
        if !pushed_container {
            lua_pop(lstate, 1);
            attach_popped_object(&mut stack, &mut ret, cur_obj);
        }
    }

    if error_set(err) {
        if arena.is_none() {
            for item in stack {
                api_free_object(item.obj);
            }
            api_free_object(ret);
        }
        ret = nil_obj();
        lua_pop(lstate, nlua_gettop(lstate) - initial_size + 1);
    }
    assert_eq!(nlua_gettop(lstate), initial_size - 1);
    ret
}

/// Convert Lua value to `LuaRef` (`nlua_pop_LuaRef`).
pub fn nlua_pop_lua_ref(lstate: *mut lua_State, _arena: &mut Arena, _err: &mut Error) -> LuaRef {
    let rv = nlua_ref_global(lstate, -1);
    lua_pop(lstate, 1);
    rv
}

/// Convert Lua value to `ObjHandle` (`nlua_pop_handle`).
pub fn nlua_pop_handle(lstate: *mut lua_State, _arena: &mut Arena, err: &mut Error) -> ObjHandle {
    let ret = if lua_type_safe(lstate, -1) != LUA_TNUMBER {
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua number",);
        -1
    } else {
        lua_tonumber_safe(lstate, -1) as ObjHandle
    };
    lua_pop(lstate, 1);
    ret
}

/// Record auxiliary values in `vim` module (`nlua_init_types`).
pub fn nlua_init_types(lstate: *mut lua_State) {
    lua_push_bytes(lstate, b"type_idx");
    nlua_push_type_idx(lstate);
    lua_rawset_safe(lstate, -3);

    lua_push_bytes(lstate, b"val_idx");
    nlua_push_val_idx(lstate);
    lua_rawset_safe(lstate, -3);

    lua_push_bytes(lstate, b"types");
    lua_createtable_safe(lstate, 0, 3);

    lua_push_bytes(lstate, b"float");
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeFloat as i32) as lua_Number);
    lua_rawset_safe(lstate, -3);
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeFloat as i32) as lua_Number);
    lua_push_bytes(lstate, b"float");
    lua_rawset_safe(lstate, -3);

    lua_push_bytes(lstate, b"array");
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeArray as i32) as lua_Number);
    lua_rawset_safe(lstate, -3);
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeArray as i32) as lua_Number);
    lua_push_bytes(lstate, b"array");
    lua_rawset_safe(lstate, -3);

    lua_push_bytes(lstate, b"dictionary");
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeDict as i32) as lua_Number);
    lua_rawset_safe(lstate, -3);
    lua_pushnumber_safe(lstate, (ObjectType::ObjectTypeDict as i32) as lua_Number);
    lua_push_bytes(lstate, b"dictionary");
    lua_rawset_safe(lstate, -3);

    lua_rawset_safe(lstate, -3);
}

/// Lua-specific variant of `api_dict_to_keydict` (`nlua_pop_keydict`).
pub fn nlua_pop_keydict<T: KeyDict>(
    lstate: *mut lua_State,
    retval: &mut T,
    hashy: FieldHashfn<T>,
    err_opt: &mut Option<&'static std::ffi::CStr>,
    arena: &mut Arena,
    err: &mut Error,
) {
    nlua_pop_keydict_opt(lstate, retval, hashy, err_opt, Some(arena), err);
}

pub fn nlua_pop_keydict_opt<T: KeyDict>(
    lstate: *mut lua_State,
    retval: &mut T,
    hashy: FieldHashfn<T>,
    err_opt: &mut Option<&'static std::ffi::CStr>,
    mut arena: Option<&mut Arena>,
    err: &mut Error,
) {
    if lua_type_safe(lstate, -1) != LUA_TTABLE {
        api_set_error!(err, ErrorType::ErrorTypeValidation, c"Expected Lua table",);
        lua_pop(lstate, 1);
        return;
    }

    let mut dummy_arena = Arena::default();
    let hashy_fn = hashy.unwrap();
    lua_pushnil_safe(lstate);
    while lua_next_safe(lstate, -2) != 0 {
        let key_bytes = lua_tolstring_bytes(lstate, -2);
        let key_cstr = lua_tolstring_opt_cstr(lstate, -2);
        let Some(field) = hashy_fn(key_bytes) else {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"invalid key: %.*s",
                key_bytes.len() as i32,
                key_cstr,
            );
            lua_pop(lstate, 3);
            return;
        };
        field.mark_set(retval);

        if field.key_type == (ObjectType::ObjectTypeNil as i32) {
            *field.field_mut::<Object>(retval) =
                nlua_pop_object_opt(lstate, true, arena.as_deref_mut(), err);
        } else if field.key_type == (ObjectType::ObjectTypeInteger as i32) {
            if field.is_hlgroup && lua_type_safe(lstate, -1) == LUA_TSTRING {
                let name_bytes = lua_tolstring_bytes(lstate, -1);
                lua_pop(lstate, 1);
                *field.field_mut::<Integer>(retval) = if !name_bytes.is_empty() {
                    syn_check_group(name_bytes) as Integer
                } else {
                    0
                };
            } else {
                *field.field_mut::<Integer>(retval) =
                    nlua_pop_integer(lstate, &mut dummy_arena, err);
            }
        } else if field.key_type == (ObjectType::ObjectTypeBoolean as i32) {
            *field.field_mut::<Boolean>(retval) = nlua_pop_boolean_strict(lstate, err);
        } else if field.key_type == (ObjectType::ObjectTypeString as i32) {
            *field.field_mut::<NvimString>(retval) =
                nlua_pop_string_opt(lstate, arena.as_deref_mut(), err);
        } else if field.key_type == (ObjectType::ObjectTypeFloat as i32) {
            *field.field_mut::<Float>(retval) = nlua_pop_float(lstate, &mut dummy_arena, err);
        } else if field.key_type == (ObjectType::ObjectTypeBuffer as i32)
            || field.key_type == (ObjectType::ObjectTypeWindow as i32)
            || field.key_type == (ObjectType::ObjectTypeTabpage as i32)
        {
            *field.field_mut::<ObjHandle>(retval) = nlua_pop_handle(lstate, &mut dummy_arena, err);
        } else if field.key_type == (ObjectType::ObjectTypeArray as i32) {
            *field.field_mut::<Array>(retval) =
                nlua_pop_array_opt(lstate, arena.as_deref_mut(), err);
        } else if field.key_type == (ObjectType::ObjectTypeDict as i32) {
            *field.field_mut::<Dict>(retval) =
                nlua_pop_dict_opt(lstate, false, arena.as_deref_mut(), err);
        } else if field.key_type == (ObjectType::ObjectTypeLuaRef as i32) {
            *field.field_mut::<LuaRef>(retval) = nlua_pop_lua_ref(lstate, &mut dummy_arena, err);
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

/// Push a `KeySet` struct as a Lua table (`nlua_push_keydict`).
pub fn nlua_push_keydict<T: KeyDict>(lstate: *mut lua_State, value: &T, table: &[KeySetLink<T>]) {
    lua_createtable_safe(lstate, 0, 0);
    for field in table {
        let Some(field_str) = field.str else {
            break;
        };
        if !field.is_field_set(value) {
            continue;
        }

        lua_pushstring(lstate, field_str);
        if field.key_type == (ObjectType::ObjectTypeNil as i32) {
            let mut obj = field.field_copy::<Object>(value);
            nlua_push_object(lstate, &mut obj, 0);
        } else if field.key_type == (ObjectType::ObjectTypeInteger as i32) {
            lua_pushinteger_safe(lstate, field.field_copy::<Integer>(value) as lua_Integer);
        } else if field.key_type == (ObjectType::ObjectTypeBuffer as i32)
            || field.key_type == (ObjectType::ObjectTypeWindow as i32)
            || field.key_type == (ObjectType::ObjectTypeTabpage as i32)
        {
            lua_pushinteger_safe(lstate, field.field_copy::<ObjHandle>(value) as lua_Integer);
        } else if field.key_type == (ObjectType::ObjectTypeFloat as i32) {
            lua_pushnumber_safe(lstate, field.field_copy::<Float>(value));
        } else if field.key_type == (ObjectType::ObjectTypeBoolean as i32) {
            lua_pushboolean_safe(lstate, field.field_copy::<Boolean>(value));
        } else if field.key_type == (ObjectType::ObjectTypeString as i32) {
            nlua_push_string(lstate, field.field_copy::<NvimString>(value), 0);
        } else if field.key_type == (ObjectType::ObjectTypeArray as i32) {
            nlua_push_array(lstate, field.field_copy::<Array>(value), 0);
        } else if field.key_type == (ObjectType::ObjectTypeDict as i32) {
            nlua_push_dict(lstate, field.field_copy::<Dict>(value), 0);
        } else if field.key_type == (ObjectType::ObjectTypeLuaRef as i32) {
            nlua_pushref(lstate, field.field_copy::<LuaRef>(value));
        } else {
            std::process::abort();
        }

        lua_rawset_safe(lstate, -3);
    }
}

impl TVPopStackItem {
    #[inline]
    fn tv_ref<'a>(&'a self, root: &'a TypVal) -> crate::eval::vars::LvalTvRef<'a> {
        match &self.tv {
            TVPopTarget::Root => crate::eval::vars::LvalTvRef::Dict(root),
            TVPopTarget::ListItem(li) => crate::eval::vars::LvalTvRef::List(li.tv()),
            TVPopTarget::DictItem(di) => crate::eval::vars::LvalTvRef::Dict(&di.borrow().di_tv),
        }
    }

    #[inline]
    fn tv_mut<'a>(&'a self, root: &'a mut TypVal) -> crate::eval::vars::LvalTvMut<'a> {
        match &self.tv {
            TVPopTarget::Root => crate::eval::vars::LvalTvMut::Dict(root),
            TVPopTarget::ListItem(li) => crate::eval::vars::LvalTvMut::List(li.tv_mut()),
            TVPopTarget::DictItem(di) => {
                crate::eval::vars::LvalTvMut::Dict(&mut di.borrow_mut().di_tv)
            }
        }
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
