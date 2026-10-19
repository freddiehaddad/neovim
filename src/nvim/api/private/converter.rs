//! Conversions between Vimscript `typval_T` and API `Object` (`src/nvim/api/private/converter.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc,
    clippy::only_used_in_recursion
)]

#[allow(unused_imports)]
use crate::api::private::helpers::{arena_array, arena_dict};
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_dict_add, tv_dict_alloc, tv_dict_item_alloc, tv_list_alloc, tv_list_append_owned_tv,
};
#[allow(unused_imports)]
use crate::eval::userfunc::find_func;
#[allow(unused_imports)]
use crate::eval::userfunc::register_luafunc;
#[allow(unused_imports)]
use crate::eval::{get_copyID, partial_name};
#[allow(unused_imports)]
use crate::lua::executor::api_new_luaref;
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::api::private::validate::{
    Array, Dict, Error, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::eval::decode::{decode_string, dict_T, dictitem_T};
use crate::eval::executor::blob_T;
use crate::garray::garray_T;
use crate::hashtab::{hashitem_T, hashitem_empty};
use crate::map_glyph_cache::NvimString;
use crate::memory::{Arena, arena_memdupz, xstrdup};

const VAR_UNLOCKED: c_int = 0;
const K_SPECIAL_VAR_NULL: i64 = 0;
const K_BOOL_VAR_FALSE: i64 = 0;
const K_BOOL_VAR_TRUE: i64 = 1;
const LUA_NOREF: c_int = -2;
const FC_LUAREF: c_int = 0x800;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct sctx_T {
    pub sc_sid: c_int,
    pub sc_seq: c_int,
    pub sc_lnum: i32,
    pub sc_chan: u64,
}

#[repr(C)]
pub struct ufunc_T {
    pub uf_varargs: c_int,
    pub uf_flags: c_int,
    pub uf_calls: c_int,
    pub uf_cleared: bool,
    pub uf_args: garray_T,
    pub uf_def_args: garray_T,
    pub uf_lines: garray_T,
    pub uf_profiling: c_int,
    pub uf_prof_initialized: c_int,
    pub uf_luaref: c_int,
    pub uf_tm_count: c_int,
    pub uf_tm_total: u64,
    pub uf_tm_self: u64,
    pub uf_tm_children: u64,
    pub uf_tml_count: *mut c_int,
    pub uf_tml_total: *mut u64,
    pub uf_tml_self: *mut u64,
    pub uf_tml_start: u64,
    pub uf_tml_children: u64,
    pub uf_tml_wait: u64,
    pub uf_tml_idx: c_int,
    pub uf_tml_execed: c_int,
    pub uf_script_ctx: sctx_T,
    pub uf_refcount: c_int,
    pub uf_scoped: *mut c_void,
    pub uf_name_exp: *mut c_char,
    pub uf_namelen: usize,
    pub uf_name: [c_char; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct partial_T {
    pub pt_refcount: c_int,
    pub pt_copyID: c_int,
    pub pt_name: *mut c_char,
    pub pt_func: *mut ufunc_T,
    pub pt_auto: bool,
    pub pt_argc: c_int,
    pub pt_argv: *mut typval_T,
    pub pt_dict: *mut dict_T,
}

#[inline]
const fn nil_obj() -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeNil,
        data: ObjectData { integer: 0 },
    }
}

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
        data: ObjectData { string: s },
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

#[cfg(not(test))]
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
            let s = unsafe { tv.vval.v_string };
            let len = if s.is_null() {
                0
            } else {
                unsafe { CStr::from_ptr(s).to_bytes().len() }
            };
            stack.push(unsafe { cbuf_to_string_obj(arena, reuse_strdata, s, len) });
        }
        VarType::VAR_NUMBER => {
            let n = unsafe { tv.vval.v_number };
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeInteger,
                data: ObjectData { integer: n },
            });
        }
        VarType::VAR_FLOAT => {
            let f = unsafe { tv.vval.v_float };
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeFloat,
                data: ObjectData { floating: f },
            });
        }
        VarType::VAR_BLOB => {
            let blob = unsafe { tv.vval.v_ptr.cast::<blob_T>() };
            let (data, len) = if blob.is_null() || unsafe { (*blob).bv_ga.ga_len <= 0 } {
                (c"".as_ptr(), 0usize)
            } else {
                unsafe {
                    (
                        (*blob).bv_ga.ga_data.cast::<c_char>().cast_const(),
                        (*blob).bv_ga.ga_len as usize,
                    )
                }
            };
            stack.push(unsafe { cbuf_to_string_obj(arena, reuse_strdata, data, len) });
        }
        VarType::VAR_FUNC => {
            let fun = unsafe { tv.vval.v_string };
            let obj = unsafe {
                if !fun.is_null() {
                    let fp = find_func(fun);
                    if !fp.is_null() && ((*fp).uf_flags & FC_LUAREF) != 0 {
                        Object {
                            obj_type: ObjectType::kObjectTypeLuaRef,
                            data: ObjectData {
                                luaref: api_new_luaref((*fp).uf_luaref),
                            },
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
            let pt = unsafe { tv.vval.v_ptr.cast::<partial_T>() };
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
                            data: ObjectData {
                                luaref: api_new_luaref((*fp).uf_luaref),
                            },
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
            let list = unsafe { tv.vval.v_list };
            if list.is_null() || unsafe { (*list).lv_len == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData {
                        array: Array {
                            size: 0,
                            capacity: 0,
                            items: std::ptr::null_mut(),
                        },
                    },
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
                    data: ObjectData { array: arr },
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
            let b = unsafe { tv.vval.v_bool == K_BOOL_VAR_TRUE as c_int };
            stack.push(Object {
                obj_type: ObjectType::kObjectTypeBoolean,
                data: ObjectData { boolean: b },
            });
        }
        VarType::VAR_SPECIAL => {
            stack.push(nil_obj());
        }
        VarType::VAR_DICT => {
            let dict = unsafe { tv.vval.v_ptr.cast::<dict_T>() };
            if dict.is_null() || unsafe { (*dict).dv_hashtab.ht_used == 0 } {
                stack.push(Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: Dict {
                            size: 0,
                            capacity: 0,
                            items: std::ptr::null_mut(),
                        },
                    },
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
                    data: ObjectData { dict: d_obj },
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
        let arr = &mut list_obj.data.array;
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
        let d = &mut dict_obj.data.dict;
        debug_assert!(d.size < d.capacity);
        (*d.items.add(d.size)).value = val;
        d.size += 1;
    }
}

/// Recursively convert a Vimscript `typval_T` into an API `Object` (`vim_to_object`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_to_object(
    obj: *mut typval_T,
    arena: *mut Arena,
    reuse_strdata: bool,
) -> Object {
    #[cfg(test)]
    {
        let _ = (obj, arena, reuse_strdata);
        nil_obj()
    }

    #[cfg(not(test))]
    unsafe {
        let copy_id = get_copyID();
        let mut stack: Vec<Object> = Vec::with_capacity(4);
        let mut mpstack: Vec<ConvStackEntry> = Vec::new();
        let key_off = std::mem::offset_of!(dictitem_T, di_key);

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

                    let di = (*cur_hi).hi_key.sub(key_off).cast::<dictitem_T>();
                    let key_ptr = (*di).di_key.as_ptr();
                    let key_len = CStr::from_ptr(key_ptr).to_bytes().len();
                    let key_obj = cbuf_to_string_obj(arena, reuse_strdata, key_ptr, key_len);

                    let dict_obj = stack.last_mut().unwrap();
                    debug_assert_eq!(dict_obj.obj_type, ObjectType::kObjectTypeDict);
                    let d = &mut dict_obj.data.dict;
                    (*d.items.add(d.size)).key = key_obj.data.string;

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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn object_to_vim(mut obj: Object, tv: *mut typval_T, err: *mut Error) {
    unsafe {
        object_to_vim_take_luaref(&mut obj, tv, false, err);
    }
}

/// Convert an API `Object` into a Vimscript `typval_T`, optionally consuming nested `LuaRef`s
/// (`object_to_vim_take_luaref`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn object_to_vim_take_luaref(
    obj: *mut Object,
    tv: *mut typval_T,
    take_luaref: bool,
    err: *mut Error,
) {
    let obj = unsafe { &mut *obj };
    let tv_ref = unsafe { &mut *tv };
    tv_ref.v_type = VarType::VAR_UNKNOWN;
    tv_ref.v_lock = VAR_UNLOCKED;

    match obj.obj_type {
        ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {
            tv_ref.v_type = VarType::VAR_SPECIAL;
            tv_ref.vval = TypvalVval {
                v_number: K_SPECIAL_VAR_NULL,
            };
        }
        ObjectType::kObjectTypeBoolean => {
            let b = unsafe { obj.data.boolean };
            tv_ref.v_type = VarType::VAR_BOOL;
            tv_ref.vval = TypvalVval {
                v_number: if b { K_BOOL_VAR_TRUE } else { K_BOOL_VAR_FALSE },
            };
        }
        ObjectType::kObjectTypeBuffer
        | ObjectType::kObjectTypeWindow
        | ObjectType::kObjectTypeTabpage
        | ObjectType::kObjectTypeInteger => {
            let n = unsafe { obj.data.integer };
            tv_ref.v_type = VarType::VAR_NUMBER;
            tv_ref.vval = TypvalVval { v_number: n };
        }
        ObjectType::kObjectTypeFloat => {
            let f = unsafe { obj.data.floating };
            tv_ref.v_type = VarType::VAR_FLOAT;
            tv_ref.vval = TypvalVval { v_float: f };
        }
        ObjectType::kObjectTypeString => {
            let s = unsafe { obj.data.string };
            *tv_ref = unsafe { decode_string(s.data, s.size, false, false) };
        }
        ObjectType::kObjectTypeArray => {
            #[cfg(not(test))]
            unsafe {
                let arr = obj.data.array;
                let list = tv_list_alloc(arr.size as isize);
                for i in 0..arr.size {
                    let mut li_tv = typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval { v_number: 0 },
                    };
                    object_to_vim_take_luaref(arr.items.add(i), &mut li_tv, take_luaref, err);
                    tv_list_append_owned_tv(list, li_tv);
                }
                if !list.is_null() {
                    (*list).lv_refcount += 1;
                }
                tv_ref.v_type = VarType::VAR_LIST;
                tv_ref.vval = TypvalVval { v_list: list };
            }
            #[cfg(test)]
            let _ = (take_luaref, err);
        }
        ObjectType::kObjectTypeDict => {
            #[cfg(not(test))]
            unsafe {
                let d = obj.data.dict;
                let dict = tv_dict_alloc();
                for i in 0..d.size {
                    let item = &mut *d.items.add(i);
                    let di = tv_dict_item_alloc(item.key.data);
                    object_to_vim_take_luaref(&mut item.value, &mut (*di).di_tv, take_luaref, err);
                    tv_dict_add(dict, di);
                }
                (*dict).dv_refcount += 1;
                tv_ref.v_type = VarType::VAR_DICT;
                tv_ref.vval = TypvalVval {
                    v_ptr: dict.cast::<c_void>(),
                };
            }
        }
        ObjectType::kObjectTypeLuaRef => {
            #[cfg(not(test))]
            unsafe {
                let mut ref_id = obj.data.luaref;
                if take_luaref {
                    obj.data.luaref = LUA_NOREF;
                } else {
                    ref_id = api_new_luaref(ref_id);
                }
                let name = register_luafunc(ref_id);
                tv_ref.v_type = VarType::VAR_FUNC;
                tv_ref.vval = TypvalVval {
                    v_string: xstrdup(name),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_object_to_vim_conversions() {
        let mut tv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        };
        unsafe {
            object_to_vim(
                Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData { integer: 99 },
                },
                &mut tv,
                std::ptr::null_mut(),
            );
            assert_eq!(tv.v_type, VarType::VAR_NUMBER);
            assert_eq!(tv.vval.v_number, 99);
        }
    }
}
