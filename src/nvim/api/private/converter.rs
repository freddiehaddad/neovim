//! Conversions between Vimscript `TypVal` and API `Object` (`src/nvim/api/private/converter.c`).

use crate::api::private::helpers::{arena_array_opt, arena_bytes_opt, arena_dict_opt, nil_obj};
use crate::api::private::validate::{
    Array, Dict, Error, NvimString, Object, ObjectData, ObjectType,
};
use crate::eval::decode::decode_string;
use crate::eval::typval::{
    Listitem, TvDict, TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType, tv_dict_add,
    tv_dict_alloc, tv_dict_item_alloc, tv_list_alloc, tv_list_append_owned_tv,
};
use crate::eval::userfunc::{FC_LUAREF, find_func, register_luafunc};
use crate::eval::{get_copy_id, partial_name};
use crate::hashtab::{Hashitem, hashitem_empty};
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::api_new_luaref;
use crate::memory::Arena;

pub(crate) const K_SPECIAL_VAR_NULL: i64 = 0;
pub(crate) const K_BOOL_VAR_FALSE: i64 = 0;
pub(crate) const K_BOOL_VAR_TRUE: i64 = 1;

#[inline]
fn cbuf_to_string_obj(arena: Option<&mut Arena>, reuse_strdata: bool, bytes: &[u8]) -> Object {
    let s = if reuse_strdata {
        if bytes.is_empty() {
            NvimString {
                data: c"".as_ptr().cast_mut(),
                size: 0,
            }
        } else {
            NvimString::from_bytes(bytes)
        }
    } else {
        arena_bytes_opt(arena, bytes)
    };
    Object {
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(s),
    }
}

enum ConvStackEntry {
    List {
        list: *mut TvList,
        li: *mut Listitem,
        saved_copy_id: i32,
        first: bool,
    },
    Dict {
        dict: *mut TvDict,
        hi: *mut Hashitem,
        todo: usize,
        saved_copy_id: i32,
        first: bool,
    },
}

fn convert_one_value(
    stack: &mut Vec<Object>,
    mpstack: &mut Vec<ConvStackEntry>,
    arena: Option<&mut Arena>,
    reuse_strdata: bool,
    tv: *mut TypVal,
    copy_id: i32,
) {
    let tv = unsafe { &mut *tv };
    match tv.v_type {
        VarType::VarString => {
            let s = tv.vval.v_string();
            let bytes = crate::memory::cstr_as_bytes(s);
            stack.push(cbuf_to_string_obj(arena, reuse_strdata, bytes));
        }
        VarType::VarNumber => {
            let n = tv.vval.v_number();
            stack.push(Object {
                obj_type: ObjectType::ObjectTypeInteger,
                data: ObjectData::Integer(n),
            });
        }
        VarType::VarFloat => {
            let f = tv.vval.v_float();
            stack.push(Object {
                obj_type: ObjectType::ObjectTypeFloat,
                data: ObjectData::Floating(f),
            });
        }
        VarType::VarBlob => {
            let blob = tv.vval.v_blob();
            let bytes: &[u8] = if blob.is_null() {
                &[]
            } else {
                unsafe { &(*blob).bv_ga }
            };
            stack.push(cbuf_to_string_obj(arena, reuse_strdata, bytes));
        }
        VarType::VarFunc => {
            let fun = tv.vval.v_string();
            let obj = {
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && (unsafe { (*fp).uf_flags } & FC_LUAREF) != 0 {
                        Object {
                            obj_type: ObjectType::ObjectTypeLuaRef,
                            data: ObjectData::LuaRef(api_new_luaref(unsafe { (*fp).uf_luaref })),
                        }
                    } else {
                        nil_obj()
                    }
                } else {
                    nil_obj()
                }
            };
            stack.push(obj);
        }
        VarType::VarPartial => {
            let pt = tv.vval.v_partial();
            let obj = {
                if !pt.is_null() {
                    let fun = partial_name(unsafe { pt.as_ref() });
                    let fp = find_func(fun);
                    if !fp.is_null() && (unsafe { (*fp).uf_flags } & FC_LUAREF) != 0 {
                        Object {
                            obj_type: ObjectType::ObjectTypeLuaRef,
                            data: ObjectData::LuaRef(api_new_luaref(unsafe { (*fp).uf_luaref })),
                        }
                    } else {
                        nil_obj()
                    }
                } else {
                    nil_obj()
                }
            };
            stack.push(obj);
        }
        VarType::VarList => {
            let list = tv.vval.v_list();
            if list.is_null() || unsafe { (*list).lv_len == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(Array {
                        size: 0,
                        capacity: 0,
                        items: std::ptr::null_mut(),
                    }),
                });
                return;
            }
            let saved_copy_id = unsafe { (*list).lv_copy_id };
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            {
                unsafe {
                    (*list).lv_copy_id = copy_id;
                }
                let arr = arena_array_opt(arena, unsafe { (*list).lv_len } as usize);
                stack.push(Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(arr),
                });
                mpstack.push(ConvStackEntry::List {
                    list,
                    li: unsafe { (*list).lv_first },
                    saved_copy_id,
                    first: true,
                });
            }
        }
        VarType::VarBool => {
            let b = tv.vval.v_bool() == K_BOOL_VAR_TRUE as i32;
            stack.push(Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(b),
            });
        }
        VarType::VarSpecial => {
            stack.push(nil_obj());
        }
        VarType::VarDict => {
            let dict = tv.vval.v_dict();
            if dict.is_null() || unsafe { (*dict).dv_hashtab.ht_used == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(Dict {
                        size: 0,
                        capacity: 0,
                        items: std::ptr::null_mut(),
                    }),
                });
                return;
            }
            let saved_copy_id = unsafe { (*dict).dv_copy_id };
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            unsafe {
                (*dict).dv_copy_id = copy_id;
                let used = (*dict).dv_hashtab.ht_used;
                let d_obj = arena_dict_opt(arena, used);
                stack.push(Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(d_obj),
                });
                mpstack.push(ConvStackEntry::Dict {
                    dict,
                    hi: (*dict).dv_hashtab.array_ptr(),
                    todo: used,
                    saved_copy_id,
                    first: true,
                });
            }
        }
        VarType::VarUnknown => {
            stack.push(nil_obj());
        }
    }
}

#[inline]
fn pop_into_parent_list(stack: &mut Vec<Object>) {
    let item = stack.pop().unwrap();
    let list_obj = stack.last_mut().unwrap();
    debug_assert_eq!(list_obj.obj_type, ObjectType::ObjectTypeArray);
    {
        let arr = list_obj.data.array_mut();
        debug_assert!(arr.size < arr.capacity);
        *arr.next_slot_mut() = item;
        arr.size += 1;
    }
}

#[inline]
fn pop_into_parent_dict(stack: &mut Vec<Object>) {
    let val = stack.pop().unwrap();
    let dict_obj = stack.last_mut().unwrap();
    debug_assert_eq!(dict_obj.obj_type, ObjectType::ObjectTypeDict);
    {
        let d = dict_obj.data.dict_mut();
        debug_assert!(d.size < d.capacity);
        d.next_slot_mut().value = val;
        d.size += 1;
    }
}

/// Recursively convert a Vimscript `TypVal` into an API `Object` (`vim_to_object`).
pub fn vim_to_object(
    obj: *mut TypVal,
    mut arena: Option<&mut Arena>,
    reuse_strdata: bool,
) -> Object {
    let copy_id = get_copy_id();
    let mut stack: Vec<Object> = Vec::with_capacity(4);
    let mut mpstack: Vec<ConvStackEntry> = Vec::new();

    convert_one_value(
        &mut stack,
        &mut mpstack,
        arena.as_deref_mut(),
        reuse_strdata,
        obj,
        copy_id,
    );

    while let Some(top) = mpstack.last_mut() {
        let next_tv: *mut TypVal = match top {
            ConvStackEntry::List {
                list,
                li,
                saved_copy_id,
                first,
            } => {
                if li.is_null() {
                    let list_ptr = *list;
                    let saved = *saved_copy_id;
                    mpstack.pop();
                    unsafe {
                        (*list_ptr).lv_copy_id = saved;
                    }
                    pop_into_parent_list(&mut stack);
                    continue;
                }
                if !*first {
                    pop_into_parent_list(&mut stack);
                }
                *first = false;
                let cur_li = *li;
                *li = unsafe { (*cur_li).li_next };
                unsafe { &mut (*cur_li).li_tv }
            }
            ConvStackEntry::Dict {
                dict,
                hi,
                todo,
                saved_copy_id,
                first,
            } => {
                if *todo == 0 {
                    let dict_ptr = *dict;
                    let saved = *saved_copy_id;
                    mpstack.pop();
                    unsafe {
                        (*dict_ptr).dv_copy_id = saved;
                    }
                    pop_into_parent_dict(&mut stack);
                    continue;
                }
                if !*first {
                    pop_into_parent_dict(&mut stack);
                }
                *first = false;
                while hashitem_empty(unsafe { &**hi }) {
                    *hi = (*hi).wrapping_add(1);
                }
                let cur_hi = *hi;
                *hi = (*hi).wrapping_add(1);
                *todo -= 1;

                let di = crate::eval::typval::tv_dict_hi2di(cur_hi);
                let key_bytes = crate::memory::cstr_as_bytes(di.di_key.as_ptr());
                let key_obj = cbuf_to_string_obj(arena.as_deref_mut(), reuse_strdata, key_bytes);

                let dict_obj = stack.last_mut().unwrap();
                debug_assert_eq!(dict_obj.obj_type, ObjectType::ObjectTypeDict);
                let d = dict_obj.data.dict_mut();
                d.next_slot_mut().key = key_obj.data.string();

                &mut di.di_tv
            }
        };

        convert_one_value(
            &mut stack,
            &mut mpstack,
            arena.as_deref_mut(),
            reuse_strdata,
            next_tv,
            copy_id,
        );
    }

    assert_eq!(stack.len(), 1);
    stack.pop().unwrap()
}

/// Convert an API `Object` into a Vimscript `TypVal` (`object_to_vim`).
pub fn object_to_vim(mut obj: Object, tv: &mut TypVal, err: &mut Error) {
    object_to_vim_take_luaref(&mut obj, tv, false, err);
}

/// Convert an API `Object` into a Vimscript `TypVal`, optionally consuming nested `LuaRef`s
/// (`object_to_vim_take_luaref`).
pub fn object_to_vim_take_luaref(
    obj: &mut Object,
    tv: &mut TypVal,
    take_luaref: bool,
    _err: &mut Error,
) {
    let tv_ref = tv;
    tv_ref.v_type = VarType::VarUnknown;
    tv_ref.v_lock = VAR_UNLOCKED;

    match obj.obj_type {
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {
            tv_ref.v_type = VarType::VarSpecial;
            tv_ref.vval = TypvalVval::Number(K_SPECIAL_VAR_NULL);
        }
        ObjectType::ObjectTypeBoolean => {
            let b = obj.data.boolean();
            tv_ref.v_type = VarType::VarBool;
            tv_ref.vval = TypvalVval::Number(if b { K_BOOL_VAR_TRUE } else { K_BOOL_VAR_FALSE });
        }
        ObjectType::ObjectTypeBuffer
        | ObjectType::ObjectTypeWindow
        | ObjectType::ObjectTypeTabpage
        | ObjectType::ObjectTypeInteger => {
            let n = obj.data.integer();
            tv_ref.v_type = VarType::VarNumber;
            tv_ref.vval = TypvalVval::Number(n);
        }
        ObjectType::ObjectTypeFloat => {
            let f = obj.data.floating();
            tv_ref.v_type = VarType::VarFloat;
            tv_ref.vval = TypvalVval::Float(f);
        }
        ObjectType::ObjectTypeString => {
            let s = obj.data.string();
            *tv_ref = decode_string(s.as_bytes(), false);
        }
        ObjectType::ObjectTypeArray => {
            let mut arr = obj.data.array();
            let list = tv_list_alloc(arr.size as isize);
            for i in 0..arr.size {
                let mut li_tv = TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(0),
                };
                object_to_vim_take_luaref(&mut arr[i], &mut li_tv, take_luaref, _err);
                tv_list_append_owned_tv(list, li_tv);
            }
            list.lv_refcount += 1;
            tv_ref.v_type = VarType::VarList;
            tv_ref.vval = TypvalVval::List(list);
        }
        ObjectType::ObjectTypeDict => {
            let mut d = obj.data.dict();
            let dict = tv_dict_alloc();
            for item in d.as_mut_slice() {
                let di = tv_dict_item_alloc(item.key.data);
                object_to_vim_take_luaref(&mut item.value, &mut di.di_tv, take_luaref, _err);
                tv_dict_add(&mut *dict, di);
            }
            dict.dv_refcount += 1;
            tv_ref.v_type = VarType::VarDict;
            tv_ref.vval = TypvalVval::Dict(dict);
        }
        ObjectType::ObjectTypeLuaRef => {
            let mut ref_id = obj.data.luaref();
            if take_luaref {
                obj.data = ObjectData::LuaRef(LUA_NOREF);
            } else {
                ref_id = api_new_luaref(ref_id);
            }
            let name = register_luafunc(ref_id);
            crate::eval::typval::tv_set_string(tv_ref, name);
            tv_ref.v_type = VarType::VarFunc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_object_to_vim_conversions() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut tv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        {
            object_to_vim(
                Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(99),
                },
                &mut tv,
                &mut Error::new(),
            );
            assert_eq!(tv.v_type, VarType::VarNumber);
            assert_eq!(tv.vval.v_number(), 99);

            let obj = vim_to_object(&mut tv, None, false);
            assert_eq!(obj.obj_type, ObjectType::ObjectTypeInteger);
            assert_eq!(obj.data.integer(), 99);

            // NULL list -> empty Array, NULL dict -> empty Dict
            let mut null_list = TypVal {
                v_type: VarType::VarList,
                v_lock: 0,
                vval: TypvalVval::List(std::ptr::null_mut()),
            };
            let arr_obj = vim_to_object(&mut null_list, None, false);
            assert_eq!(arr_obj.obj_type, ObjectType::ObjectTypeArray);
            assert_eq!(arr_obj.data.array().size, 0);

            let mut null_dict = TypVal {
                v_type: VarType::VarDict,
                v_lock: 0,
                vval: TypvalVval::Dict(std::ptr::null_mut()),
            };
            let dict_obj = vim_to_object(&mut null_dict, None, false);
            assert_eq!(dict_obj.obj_type, ObjectType::ObjectTypeDict);
            assert_eq!(dict_obj.data.dict().size, 0);

            // Self-referencing list converts nested cycle to Nil
            let list = tv_list_alloc(1);
            list.lv_refcount += 1;
            let mut list_tv = TypVal {
                v_type: VarType::VarList,
                v_lock: 0,
                vval: TypvalVval::List(list),
            };
            tv_list_append_owned_tv(&mut *list, list_tv);
            let cyc_obj = vim_to_object(&mut list_tv, None, false);
            assert_eq!(cyc_obj.obj_type, ObjectType::ObjectTypeArray);
            assert_eq!(cyc_obj.data.array().size, 1);
            assert_eq!(cyc_obj.data.array()[0].obj_type, ObjectType::ObjectTypeNil);
            crate::api::private::helpers::api_free_object(cyc_obj);
            unsafe {
                (*list.lv_first).li_tv.v_type = VarType::VarUnknown;
            }
            crate::eval::typval::tv_list_free(list);
        }
    }
}
