//! Conversions between Vimscript `typval_T` and API `Object` (`src/nvim/api/private/converter.c`).

use crate::api::private::helpers::{arena_array, arena_dict, nil_obj};
use crate::api::private::validate::{
    Array, Dict, Error, NvimString, Object, ObjectData, ObjectType,
};
use crate::eval::decode::decode_string;
use crate::eval::typval::{
    TypvalVval, VAR_UNLOCKED, VarType, dict_T, list_T, listitem_T, tv_dict_add, tv_dict_alloc,
    tv_dict_item_alloc, tv_list_alloc, tv_list_append_owned_tv, typval_T,
};
use crate::eval::userfunc::{FC_LUAREF, find_func, register_luafunc};
use crate::eval::{get_copyID, partial_name};
use crate::hashtab::{hashitem_T, hashitem_empty};
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::api_new_luaref;
use crate::memory::{Arena, arena_memdupz};
use std::ffi::{CStr, c_char, c_int};

pub(crate) const K_SPECIAL_VAR_NULL: i64 = 0;
pub(crate) const K_BOOL_VAR_FALSE: i64 = 0;
pub(crate) const K_BOOL_VAR_TRUE: i64 = 1;

#[inline]
unsafe fn cbuf_to_string_obj(
    arena: *mut Arena,
    reuse_strdata: bool,
    data: *const c_char,
    len: usize,
) -> Object {
    let ptr = if len == 0 || data.is_null() {
        c"".as_ptr()
    } else {
        data
    };
    let s = if reuse_strdata {
        NvimString {
            data: ptr.cast_mut(),
            size: len,
        }
    } else {
        let mem = unsafe { arena_memdupz(arena, ptr, len) };
        NvimString {
            data: mem,
            size: len,
        }
    };
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData::String(s),
    }
}

enum ConvStackEntry {
    List {
        list: *mut list_T,
        li: *mut listitem_T,
        saved_copy_id: c_int,
        first: bool,
    },
    Dict {
        dict: *mut dict_T,
        hi: *mut hashitem_T,
        todo: usize,
        saved_copy_id: c_int,
        first: bool,
    },
}

unsafe fn convert_one_value(
    stack: &mut Vec<Object>,
    mpstack: &mut Vec<ConvStackEntry>,
    arena: *mut Arena,
    reuse_strdata: bool,
    tv: *mut typval_T,
    copy_id: c_int,
) {
    let tv = unsafe { &mut *tv };
    match tv.v_type {
        VarType::VAR_STRING => {
            let s = tv.vval.v_string();
            let len = if s.is_null() {
                0
            } else {
                unsafe { CStr::from_ptr(s).to_bytes().len() }
            };
            stack.push(unsafe { cbuf_to_string_obj(arena, reuse_strdata, s, len) });
        }
        VarType::VAR_NUMBER => {
            let n = tv.vval.v_number();
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeInteger,
                data: ObjectData::Integer(n),
            });
        }
        VarType::VAR_FLOAT => {
            let f = tv.vval.v_float();
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeFloat,
                data: ObjectData::Floating(f),
            });
        }
        VarType::VAR_BLOB => {
            let blob = tv.vval.v_blob();
            let (data, len) = if blob.is_null() || unsafe { (*blob).bv_ga.is_empty() } {
                (c"".as_ptr(), 0usize)
            } else {
                unsafe { ((*blob).bv_ga.as_ptr().cast::<c_char>(), (*blob).bv_ga.len()) }
            };
            stack.push(unsafe { cbuf_to_string_obj(arena, reuse_strdata, data, len) });
        }
        VarType::VAR_FUNC => {
            let fun = tv.vval.v_string();
            let obj = unsafe {
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                        Object {
                            obj_type: ObjectType::kObjectTypeLuaRef,
                            data: ObjectData::LuaRef(api_new_luaref((*fp).uf_luaref)),
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
        VarType::VAR_PARTIAL => {
            let pt = tv.vval.v_partial();
            let fun = if pt.is_null() {
                std::ptr::null()
            } else {
                unsafe { partial_name(pt) }
            };
            let obj = unsafe {
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                        Object {
                            obj_type: ObjectType::kObjectTypeLuaRef,
                            data: ObjectData::LuaRef(api_new_luaref((*fp).uf_luaref)),
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
        VarType::VAR_LIST => {
            let list = tv.vval.v_list();
            if list.is_null() || unsafe { (*list).lv_len == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData::Array(Array {
                            size: 0,
                            capacity: 0,
                            items: std::ptr::null_mut(),
                        }),
                });
                return;
            }
            let saved_copy_id = unsafe { (*list).lv_copyID };
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            unsafe {
                (*list).lv_copyID = copy_id;
                let arr = arena_array(arena, (*list).lv_len as usize);
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData::Array(arr),
                });
                mpstack.push(ConvStackEntry::List {
                    list,
                    li: (*list).lv_first,
                    saved_copy_id,
                    first: true,
                });
            }
        }
        VarType::VAR_BOOL => {
            let b = tv.vval.v_bool() == K_BOOL_VAR_TRUE as c_int;
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeBoolean,
                data: ObjectData::Boolean(b),
            });
        }
        VarType::VAR_SPECIAL => {
            stack.push(nil_obj());
        }
        VarType::VAR_DICT => {
            let dict = tv.vval.v_dict();
            if dict.is_null() || unsafe { (*dict).dv_hashtab.ht_used == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData::Dict(Dict {
                            size: 0,
                            capacity: 0,
                            items: std::ptr::null_mut(),
                        }),
                });
                return;
            }
            let saved_copy_id = unsafe { (*dict).dv_copyID };
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            unsafe {
                (*dict).dv_copyID = copy_id;
                let used = (*dict).dv_hashtab.ht_used;
                let d_obj = arena_dict(arena, used);
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData::Dict(d_obj),
                });
                mpstack.push(ConvStackEntry::Dict {
                    dict,
                    hi: (*dict).dv_hashtab.ht_array,
                    todo: used,
                    saved_copy_id,
                    first: true,
                });
            }
        }
        VarType::VAR_UNKNOWN => {
            stack.push(nil_obj());
        }
    }
}

#[inline]
unsafe fn pop_into_parent_list(stack: &mut Vec<Object>) {
    let item = stack.pop().unwrap();
    let list_obj = stack.last_mut().unwrap();
    debug_assert_eq!(list_obj.obj_type, ObjectType::kObjectTypeArray);
    unsafe {
        let arr = list_obj.data.array_mut();
        debug_assert!(arr.size < arr.capacity);
        *arr.items.add(arr.size) = item;
        arr.size += 1;
    }
}

#[inline]
unsafe fn pop_into_parent_dict(stack: &mut Vec<Object>) {
    let val = stack.pop().unwrap();
    let dict_obj = stack.last_mut().unwrap();
    debug_assert_eq!(dict_obj.obj_type, ObjectType::kObjectTypeDict);
    unsafe {
        let d = dict_obj.data.dict_mut();
        debug_assert!(d.size < d.capacity);
        (*d.items.add(d.size)).value = val;
        d.size += 1;
    }
}

/// Recursively convert a Vimscript `typval_T` into an API `Object` (`vim_to_object`).
pub unsafe fn vim_to_object(obj: *mut typval_T, arena: *mut Arena, reuse_strdata: bool) -> Object {
    unsafe {
        let copy_id = get_copyID();
        let mut stack: Vec<Object> = Vec::with_capacity(4);
        let mut mpstack: Vec<ConvStackEntry> = Vec::new();

        convert_one_value(&mut stack, &mut mpstack, arena, reuse_strdata, obj, copy_id);

        while let Some(top) = mpstack.last_mut() {
            let next_tv: *mut typval_T = match top {
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
                        (*list_ptr).lv_copyID = saved;
                        pop_into_parent_list(&mut stack);
                        continue;
                    }
                    if !*first {
                        pop_into_parent_list(&mut stack);
                    }
                    *first = false;
                    let cur_li = *li;
                    *li = (*cur_li).li_next;
                    &mut (*cur_li).li_tv
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
                        (*dict_ptr).dv_copyID = saved;
                        pop_into_parent_dict(&mut stack);
                        continue;
                    }
                    if !*first {
                        pop_into_parent_dict(&mut stack);
                    }
                    *first = false;
                    while hashitem_empty(&**hi) {
                        *hi = (*hi).add(1);
                    }
                    let cur_hi = *hi;
                    *hi = (*hi).add(1);
                    *todo -= 1;

                    let di = crate::eval::typval::tv_dict_hi2di(cur_hi);
                    let key_ptr = (*di).di_key.as_ptr();
                    let key_len = CStr::from_ptr(key_ptr).to_bytes().len();
                    let key_obj = cbuf_to_string_obj(arena, reuse_strdata, key_ptr, key_len);

                    let dict_obj = stack.last_mut().unwrap();
                    debug_assert_eq!(dict_obj.obj_type, ObjectType::kObjectTypeDict);
                    let d = dict_obj.data.dict_mut();
                    (*d.items.add(d.size)).key = key_obj.data.string();

                    &mut (*di).di_tv
                }
            };

            convert_one_value(
                &mut stack,
                &mut mpstack,
                arena,
                reuse_strdata,
                next_tv,
                copy_id,
            );
        }

        assert_eq!(stack.len(), 1);
        stack.pop().unwrap()
    }
}

/// Convert an API `Object` into a Vimscript `typval_T` (`object_to_vim`).
pub unsafe fn object_to_vim(mut obj: Object, tv: *mut typval_T, err: *mut Error) {
    unsafe {
        object_to_vim_take_luaref(&mut obj, tv, false, err);
    }
}

/// Convert an API `Object` into a Vimscript `typval_T`, optionally consuming nested `LuaRef`s
/// (`object_to_vim_take_luaref`).
pub unsafe fn object_to_vim_take_luaref(
    obj: *mut Object,
    tv: *mut typval_T,
    take_luaref: bool,
    _err: *mut Error,
) {
    let obj = unsafe { &mut *obj };
    let tv_ref = unsafe { &mut *tv };
    tv_ref.v_type = VarType::VAR_UNKNOWN;
    tv_ref.v_lock = VAR_UNLOCKED;

    match obj.obj_type {
        ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {
            tv_ref.v_type = VarType::VAR_SPECIAL;
            tv_ref.vval = TypvalVval::Number(K_SPECIAL_VAR_NULL);
        }
        ObjectType::kObjectTypeBoolean => {
            let b = obj.data.boolean();
            tv_ref.v_type = VarType::VAR_BOOL;
            tv_ref.vval = TypvalVval::Number(if b { K_BOOL_VAR_TRUE } else { K_BOOL_VAR_FALSE });
        }
        ObjectType::kObjectTypeBuffer
        | ObjectType::kObjectTypeWindow
        | ObjectType::kObjectTypeTabpage
        | ObjectType::kObjectTypeInteger => {
            let n = obj.data.integer();
            tv_ref.v_type = VarType::VAR_NUMBER;
            tv_ref.vval = TypvalVval::Number(n);
        }
        ObjectType::kObjectTypeFloat => {
            let f = obj.data.floating();
            tv_ref.v_type = VarType::VAR_FLOAT;
            tv_ref.vval = TypvalVval::Float(f);
        }
        ObjectType::kObjectTypeString => {
            let s = obj.data.string();
            *tv_ref = unsafe { decode_string(s.data, s.size, false, false) };
        }
        ObjectType::kObjectTypeArray => unsafe {
            let arr = obj.data.array();
            let list = tv_list_alloc(arr.size as isize);
            for i in 0..arr.size {
                let mut li_tv = typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(0),
                };
                object_to_vim_take_luaref(arr.items.add(i), &mut li_tv, take_luaref, _err);
                tv_list_append_owned_tv(list, li_tv);
            }
            if !list.is_null() {
                (*list).lv_refcount += 1;
            }
            tv_ref.v_type = VarType::VAR_LIST;
            tv_ref.vval = TypvalVval::List(list);
        },
        ObjectType::kObjectTypeDict => unsafe {
            let d = obj.data.dict();
            let dict = tv_dict_alloc();
            for i in 0..d.size {
                let item = &mut *d.items.add(i);
                let di = tv_dict_item_alloc(item.key.data);
                object_to_vim_take_luaref(&mut item.value, &mut (*di).di_tv, take_luaref, _err);
                tv_dict_add(dict, di);
            }
            (*dict).dv_refcount += 1;
            tv_ref.v_type = VarType::VAR_DICT;
            tv_ref.vval = TypvalVval::Dict(dict);
        },
        ObjectType::kObjectTypeLuaRef => unsafe {
            let mut ref_id = obj.data.luaref();
            if take_luaref {
                obj.data = ObjectData::LuaRef(LUA_NOREF);
            } else {
                ref_id = api_new_luaref(ref_id);
            }
            let name = register_luafunc(ref_id);
            crate::eval::typval::tv_set_string(tv_ref, name);
            tv_ref.v_type = VarType::VAR_FUNC;
        },
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
        let mut tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        };
        unsafe {
            object_to_vim(
                Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData::Integer(99),
                },
                &mut tv,
                std::ptr::null_mut(),
            );
            assert_eq!(tv.v_type, VarType::VAR_NUMBER);
            assert_eq!(tv.vval.v_number(), 99);

            let obj = vim_to_object(&mut tv, std::ptr::null_mut(), false);
            assert_eq!(obj.obj_type, ObjectType::kObjectTypeInteger);
            assert_eq!(obj.data.integer(), 99);

            // NULL list -> empty Array, NULL dict -> empty Dict
            let mut null_list = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: 0,
                vval: TypvalVval::List(std::ptr::null_mut()),
            };
            let arr_obj = vim_to_object(&mut null_list, std::ptr::null_mut(), false);
            assert_eq!(arr_obj.obj_type, ObjectType::kObjectTypeArray);
            assert_eq!(arr_obj.data.array().size, 0);

            let mut null_dict = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: 0,
                vval: TypvalVval::Dict(std::ptr::null_mut()),
            };
            let dict_obj = vim_to_object(&mut null_dict, std::ptr::null_mut(), false);
            assert_eq!(dict_obj.obj_type, ObjectType::kObjectTypeDict);
            assert_eq!(dict_obj.data.dict().size, 0);

            // Self-referencing list converts nested cycle to Nil
            let list = tv_list_alloc(1);
            (*list).lv_refcount += 1;
            let mut list_tv = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: 0,
                vval: TypvalVval::List(list),
            };
            tv_list_append_owned_tv(list, list_tv);
            let cyc_obj = vim_to_object(&mut list_tv, std::ptr::null_mut(), false);
            assert_eq!(cyc_obj.obj_type, ObjectType::kObjectTypeArray);
            assert_eq!(cyc_obj.data.array().size, 1);
            assert_eq!(
                (*cyc_obj.data.array().items).obj_type,
                ObjectType::kObjectTypeNil
            );
            crate::api::private::helpers::api_free_object(cyc_obj);
            (*(*list).lv_first).li_tv.v_type = VarType::VAR_UNKNOWN;
            crate::eval::typval::tv_list_free(list);
        }
    }
}
