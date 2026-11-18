//! Conversions between Vimscript `TypVal` and API `Object` (`src/nvim/api/private/converter.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::nil_obj;
use crate::api::private::validate::{Array, Dict, NvimString, Object};
use crate::eval::decode::decode_string;
use crate::eval::typval::{
    TypVal, TypvalVval, VAR_UNLOCKED, VarType, tv_dict_add, tv_dict_alloc, tv_list_alloc,
    tv_list_append_owned_tv,
};
use crate::eval::userfunc::{FC_LUAREF, find_func, register_luafunc};
use crate::eval::{get_copy_id, partial_name};
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::api_new_luaref;

pub(crate) const K_SPECIAL_VAR_NULL: i64 = 0;
pub(crate) const K_BOOL_VAR_FALSE: i64 = 0;
pub(crate) const K_BOOL_VAR_TRUE: i64 = 1;

#[inline]
fn cbuf_to_string_obj(bytes: &[u8]) -> Object {
    Object::String(NvimString::from_bytes(bytes))
}

enum ConvStackEntry {
    List {
        list: crate::eval::typval::ListId,
        li: crate::eval::typval::ListitemId,
        saved_copy_id: i32,
        first: bool,
    },
    Dict {
        dict: crate::eval::typval::DictId,
        hi_idx: usize,
        todo: usize,
        saved_copy_id: i32,
        first: bool,
    },
}

fn convert_one_value(
    stack: &mut Vec<Object>,
    mpstack: &mut Vec<ConvStackEntry>,
    tv: &TypVal,
    copy_id: i32,
) {
    match tv.v_type {
        VarType::VarString => {
            let bytes = tv.vval.v_string_bytes();
            stack.push(cbuf_to_string_obj(bytes));
        }
        VarType::VarNumber => {
            let n = tv.vval.v_number();
            stack.push(Object::Integer(n));
        }
        VarType::VarFloat => {
            let f = tv.vval.v_float();
            stack.push(Object::Float(f));
        }
        VarType::VarBlob => {
            let blob = tv.vval.v_blob_opt();
            let obj = match blob {
                Some(b) => cbuf_to_string_obj(&b.bv_ga.borrow()),
                None => cbuf_to_string_obj(&[]),
            };
            stack.push(obj);
        }
        VarType::VarFunc => {
            let obj = if let Some(fun) = tv.vval.v_string_cstr()
                && let Some(fp) = find_func(Some(fun.to_bytes()))
                && (fp.uf_flags.get() & FC_LUAREF) != 0
            {
                Object::LuaRef(api_new_luaref(fp.uf_luaref.get()))
            } else {
                nil_obj()
            };
            stack.push(obj);
        }
        VarType::VarPartial => {
            let obj = if let Some(pt) = tv.vval.v_partial_opt() {
                let fun = partial_name(Some(pt));
                if let Some(fp) = find_func(Some(fun.to_bytes()))
                    && (fp.uf_flags.get() & FC_LUAREF) != 0
                {
                    Object::LuaRef(api_new_luaref(fp.uf_luaref.get()))
                } else {
                    nil_obj()
                }
            } else {
                nil_obj()
            };
            stack.push(obj);
        }
        VarType::VarList => {
            let list_ptr = tv.vval.v_list();
            let Some(list) = tv.vval.v_list_opt() else {
                stack.push(Object::Array(Array::new()));
                return;
            };
            if list.lv_len.get() == 0 {
                stack.push(Object::Array(Array::new()));
                return;
            };
            let saved_copy_id = list.lv_copy_id.get();
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            list.lv_copy_id.set(copy_id);
            let arr = Array::with_capacity(list.lv_len.get() as usize);
            stack.push(Object::Array(arr));
            mpstack.push(ConvStackEntry::List {
                list: list_ptr,
                li: list.lv_first.get(),
                saved_copy_id,
                first: true,
            });
        }
        VarType::VarBool => {
            let b = tv.vval.v_bool() == K_BOOL_VAR_TRUE as i32;
            stack.push(Object::Boolean(b));
        }
        VarType::VarSpecial => {
            stack.push(nil_obj());
        }
        VarType::VarDict => {
            let dict_ptr = tv.vval.v_dict();
            let Some(dict) = tv.vval.v_dict_opt() else {
                stack.push(Object::Dict(Dict::new()));
                return;
            };
            if dict.dv_hashtab.ht_used.get() == 0 {
                stack.push(Object::Dict(Dict::new()));
                return;
            }
            let saved_copy_id = dict.dv_copy_id.get();
            if saved_copy_id == copy_id {
                stack.push(nil_obj());
                return;
            }
            dict.dv_copy_id.set(copy_id);
            let used = dict.dv_hashtab.ht_used.get();
            let d_obj = Dict::with_capacity(used);
            stack.push(Object::Dict(d_obj));
            mpstack.push(ConvStackEntry::Dict {
                dict: dict_ptr,
                hi_idx: 0,
                todo: used,
                saved_copy_id,
                first: true,
            });
        }
        VarType::VarUnknown => {
            stack.push(nil_obj());
        }
    }
}

#[inline]
fn pop_into_parent_list(stack: &mut Vec<Object>) {
    let item = stack.pop().unwrap();
    let Some(Object::Array(arr)) = stack.last_mut() else {
        unreachable!("list conversion requires an array parent");
    };
    debug_assert!(arr.size < arr.capacity);
    *arr.next_slot_mut() = item;
    arr.size += 1;
}

#[inline]
fn pop_into_parent_dict(stack: &mut Vec<Object>) {
    let val = stack.pop().unwrap();
    let Some(Object::Dict(dict)) = stack.last_mut() else {
        unreachable!("dictionary conversion requires a dictionary parent");
    };
    debug_assert!(dict.size < dict.capacity);
    dict.next_slot_mut().value = val;
    dict.size += 1;
}

/// Recursively convert a Vimscript `TypVal` into an API `Object` (`vim_to_object`).
pub fn vim_to_object(obj: &TypVal) -> Object {
    let copy_id = get_copy_id();
    let mut stack: Vec<Object> = Vec::with_capacity(4);
    let mut mpstack: Vec<ConvStackEntry> = Vec::new();

    convert_one_value(&mut stack, &mut mpstack, obj, copy_id);

    while let Some(top) = mpstack.last_mut() {
        match top {
            ConvStackEntry::List {
                list,
                li,
                saved_copy_id,
                first,
            } => {
                if li.is_null() {
                    let list_ptr = list.clone();
                    let saved = *saved_copy_id;
                    mpstack.pop();

                    list_ptr.lv_copy_id.set(saved);

                    pop_into_parent_list(&mut stack);
                    continue;
                }
                if !*first {
                    pop_into_parent_list(&mut stack);
                }
                *first = false;
                let cur_li = li.clone();
                *li = cur_li.li_next.get();
                let tv_ref = cur_li.tv();
                convert_one_value(&mut stack, &mut mpstack, &tv_ref, copy_id);
            }
            ConvStackEntry::Dict {
                dict,
                hi_idx,
                todo,
                saved_copy_id,
                first,
            } => {
                if *todo == 0 {
                    let dict_ptr = dict.clone();
                    let saved = *saved_copy_id;
                    mpstack.pop();

                    dict_ptr.dv_copy_id.set(saved);

                    pop_into_parent_dict(&mut stack);
                    continue;
                }
                if !*first {
                    pop_into_parent_dict(&mut stack);
                }
                *first = false;
                let (next_idx, di) = dict.dv_hashtab.next_used_di(*hi_idx);
                *hi_idx = next_idx;
                *todo -= 1;

                let Some(Object::Dict(dict)) = stack.last_mut() else {
                    unreachable!("dictionary conversion requires a dictionary parent");
                };
                dict.next_slot_mut().key = NvimString::from_bytes(di.key_bytes());

                let tv_ref = di.tv();
                convert_one_value(&mut stack, &mut mpstack, &tv_ref, copy_id);
            }
        }
    }

    assert_eq!(stack.len(), 1);
    stack.pop().unwrap()
}

/// Convert an API `Object` into a Vimscript `TypVal` (`object_to_vim`).
pub fn object_to_vim(obj: Object, tv: &mut TypVal) {
    object_to_vim_ref(&obj, tv);
}

fn object_to_vim_ref(obj: &Object, tv: &mut TypVal) {
    tv.v_type = VarType::VarUnknown;
    tv.v_lock = VAR_UNLOCKED;

    match obj {
        Object::Unset | Object::Nil => {
            tv.v_type = VarType::VarSpecial;
            tv.vval = TypvalVval::Number(K_SPECIAL_VAR_NULL);
        }
        Object::Boolean(value) => {
            tv.v_type = VarType::VarBool;
            tv.vval = TypvalVval::Number(if *value {
                K_BOOL_VAR_TRUE
            } else {
                K_BOOL_VAR_FALSE
            });
        }
        Object::Buffer(value)
        | Object::Window(value)
        | Object::Tabpage(value)
        | Object::Integer(value) => {
            tv.v_type = VarType::VarNumber;
            tv.vval = TypvalVval::Number(*value);
        }
        Object::Float(value) => {
            tv.v_type = VarType::VarFloat;
            tv.vval = TypvalVval::Float(*value);
        }
        Object::String(value) => {
            *tv = decode_string(value.as_bytes(), false);
        }
        Object::Array(value) => {
            let slice = value.as_slice();
            let list = tv_list_alloc(slice.len() as isize);
            for item in slice {
                let mut li_tv = TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(0),
                };
                object_to_vim_ref(item, &mut li_tv);
                tv_list_append_owned_tv(&list, li_tv);
            }
            list.lv_refcount.set(list.lv_refcount.get() + 1);
            tv.v_type = VarType::VarList;
            tv.vval = TypvalVval::List(list);
        }
        Object::Dict(value) => {
            let slice = value.as_slice();
            let dict = tv_dict_alloc();
            for item in slice {
                let di = crate::eval::typval::tv_dict_item_alloc_len(item.key.as_bytes());
                object_to_vim_ref(&item.value, &mut di.tv_mut());
                tv_dict_add(&dict, &di);
            }
            dict.dv_refcount.set(dict.dv_refcount.get() + 1);
            tv.v_type = VarType::VarDict;
            tv.vval = TypvalVval::Dict(dict);
        }
        Object::LuaRef(value) => {
            let ref_id = api_new_luaref(*value);
            let name = register_luafunc(ref_id);
            crate::eval::typval::tv_set_string(tv, Some(&name));
            tv.v_type = VarType::VarFunc;
        }
    }
}

/// Convert an API `Object` into a Vimscript `TypVal`, optionally consuming nested `LuaRef`s
/// (`object_to_vim_take_luaref`).
pub fn object_to_vim_take_luaref(obj: &mut Object, tv: &mut TypVal, take_luaref: bool) {
    if !take_luaref {
        object_to_vim_ref(obj, tv);
        return;
    }
    object_to_vim_take_luaref_impl(obj, tv);
}

fn object_to_vim_take_luaref_impl(obj: &mut Object, tv: &mut TypVal) {
    let tv_ref = tv;
    tv_ref.v_type = VarType::VarUnknown;
    tv_ref.v_lock = VAR_UNLOCKED;

    match obj {
        Object::Unset | Object::Nil => {
            tv_ref.v_type = VarType::VarSpecial;
            tv_ref.vval = TypvalVval::Number(K_SPECIAL_VAR_NULL);
        }
        Object::Boolean(value) => {
            tv_ref.v_type = VarType::VarBool;
            tv_ref.vval = TypvalVval::Number(if *value {
                K_BOOL_VAR_TRUE
            } else {
                K_BOOL_VAR_FALSE
            });
        }
        Object::Buffer(value)
        | Object::Window(value)
        | Object::Tabpage(value)
        | Object::Integer(value) => {
            tv_ref.v_type = VarType::VarNumber;
            tv_ref.vval = TypvalVval::Number(*value);
        }
        Object::Float(value) => {
            tv_ref.v_type = VarType::VarFloat;
            tv_ref.vval = TypvalVval::Float(*value);
        }
        Object::String(value) => {
            *tv_ref = decode_string(value.as_bytes(), false);
        }
        Object::Array(arr) => {
            let list = tv_list_alloc(arr.size as isize);
            for item in arr.as_mut_slice() {
                let mut li_tv = TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(0),
                };
                object_to_vim_take_luaref_impl(item, &mut li_tv);
                tv_list_append_owned_tv(&list, li_tv);
            }
            list.lv_refcount.set(list.lv_refcount.get() + 1);
            tv_ref.v_type = VarType::VarList;
            tv_ref.vval = TypvalVval::List(list);
        }
        Object::Dict(d) => {
            let dict = tv_dict_alloc();
            for item in d.as_mut_slice() {
                let di = crate::eval::typval::tv_dict_item_alloc_len(item.key.as_bytes());
                object_to_vim_take_luaref_impl(&mut item.value, &mut di.tv_mut());
                tv_dict_add(&dict, &di);
            }
            dict.dv_refcount.set(dict.dv_refcount.get() + 1);
            tv_ref.v_type = VarType::VarDict;
            tv_ref.vval = TypvalVval::Dict(dict);
        }
        Object::LuaRef(value) => {
            let ref_id = std::mem::replace(value, LUA_NOREF);
            let name = register_luafunc(ref_id);
            crate::eval::typval::tv_set_string(tv_ref, Some(&name));
            tv_ref.v_type = VarType::VarFunc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::validate::ObjectType;

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
            object_to_vim(Object::Integer(99), &mut tv);
            assert_eq!(tv.v_type, VarType::VarNumber);
            assert_eq!(tv.vval.v_number(), 99);

            let obj = vim_to_object(&tv);
            assert_eq!(obj.obj_type(), ObjectType::ObjectTypeInteger);
            assert_eq!(obj.as_integer().expect("integer Object"), 99);

            // NULL list -> empty Array, NULL dict -> empty Dict
            let null_list = TypVal {
                v_type: VarType::VarList,
                v_lock: 0,
                vval: TypvalVval::List(crate::eval::typval::ListId::NULL),
            };
            let arr_obj = vim_to_object(&null_list);
            assert_eq!(arr_obj.obj_type(), ObjectType::ObjectTypeArray);
            assert_eq!(arr_obj.as_array().expect("array Object").clone().size, 0);

            let null_dict = TypVal {
                v_type: VarType::VarDict,
                v_lock: 0,
                vval: TypvalVval::Dict(crate::eval::typval::DictId::NULL),
            };
            let dict_obj = vim_to_object(&null_dict);
            assert!(matches!(dict_obj, Object::Dict(dict) if dict.size == 0));

            // Self-referencing list converts nested cycle to Nil
            let list = tv_list_alloc(1);
            list.lv_refcount.set(list.lv_refcount.get() + 1);
            let list_tv = TypVal {
                v_type: VarType::VarList,
                v_lock: 0,
                vval: TypvalVval::List(list.clone()),
            };
            tv_list_append_owned_tv(&list, list_tv.clone());
            let cyc_obj = vim_to_object(&list_tv);
            assert_eq!(cyc_obj.obj_type(), ObjectType::ObjectTypeArray);
            assert_eq!(cyc_obj.as_array().expect("array Object").clone().size, 1);
            assert_eq!(
                cyc_obj.as_array().expect("array Object").clone()[0].obj_type(),
                ObjectType::ObjectTypeNil
            );
            crate::api::private::helpers::api_free_object(cyc_obj);

            list.lv_first.get().tv_mut().v_type = VarType::VarUnknown;

            crate::eval::typval::tv_list_free(Some(&list));
        }
    }
}
