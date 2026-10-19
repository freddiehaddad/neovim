//! List support and container (`List`, `Dict`, `Blob`) functions (`src/nvim/eval/list.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_blob_copy, tv_clear, tv_copy, tv_dict_add_tv, tv_dict_alloc_ret, tv_dict_item_remove,
    tv_dict_unref, tv_list_alloc_ret, tv_list_append_owned_tv, tv_list_append_tv, tv_list_extend,
    tv_list_find, tv_list_insert_tv, tv_list_item_remove, tv_list_reverse, tv_list_unref,
};
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_blob_remove, tv_check_for_string_or_list_or_blob_arg, tv_dict_extend, tv_dict_remove,
    tv_equal, tv_get_string, tv_get_string_chk, tv_list_remove,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_get_bool_chk, tv_get_number_chk};
#[allow(unused_imports)]
use crate::eval::vars::{get_vim_var_tv, set_vim_var_nr, set_vim_var_string, set_vim_var_type};
#[allow(unused_imports)]
use crate::eval::vars::{prepare_vimvar, restore_vimvar, var_check_fixed, var_check_ro};
#[allow(unused_imports)]
use crate::eval::{eval_expr_typval, get_copyID};
#[allow(unused_imports)]
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::*;
#[allow(unused_imports)]
use crate::mbyte::{mb_strnicmp, utf_ptr2len, utfc_ptr2len};
#[allow(unused_imports)]
use crate::message::emsg;
use crate::message::semsg;
#[allow(unused_imports)]
use crate::strings::reverse_text;
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::EvalFuncData;
use crate::eval::executor::blob_T;
use crate::garray::{ga_append, ga_concat, ga_grow, ga_init, garray_T};
use crate::hashtab::{hash_lock, hash_unlock, hashitem_empty};
use crate::math::{FAIL, OK};
use crate::memory::xmemdupz;

const VAR_UNLOCKED: c_int = 0;
const VAR_LOCKED: c_int = 1;
const VAR_FIXED: c_int = 2;

const K_LIST_LEN_UNKNOWN: isize = -1;
const TV_TRANSLATE: usize = usize::MAX;

const VV_VAL: c_int = 35;
const VV_KEY: c_int = 36;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterMap {
    Filter,
    Map,
    MapNew,
    ForEach,
}

#[cfg(not(test))]
unsafe extern "C" {

    fn value_check_lock(lock: c_int, name: *const c_char, name_len: usize) -> bool;

    fn tv_dict_copy(
        conv: *const c_void,
        orig: *mut dict_T,
        deep: bool,
        copy_id: c_int,
    ) -> *mut dict_T;

    fn tv_list_copy(
        conv: *const c_void,
        orig: *mut list_T,
        deep: bool,
        copy_id: c_int,
    ) -> *mut list_T;

}

#[inline]
unsafe fn tv_list_locked(l: *const list_T) -> c_int {
    if l.is_null() {
        VAR_FIXED
    } else {
        unsafe { (*l).lv_lock }
    }
}

#[inline]
unsafe fn tv_list_set_lock(l: *mut list_T, lock: c_int) {
    if !l.is_null() {
        unsafe {
            (*l).lv_lock = lock;
        }
    }
}

#[inline]
unsafe fn tv_list_len(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_first(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

#[inline]
unsafe fn tv_list_set_ret(tv: *mut typval_T, l: *mut list_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).vval.v_list = l;
        if !l.is_null() {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_blob_len(b: *const blob_T) -> c_int {
    if b.is_null() {
        0
    } else {
        unsafe { (*b).bv_ga.ga_len }
    }
}

#[inline]
unsafe fn tv_blob_get(b: *const blob_T, idx: c_int) -> u8 {
    unsafe { *(*b).bv_ga.ga_data.cast::<u8>().add(idx as usize) }
}

#[inline]
unsafe fn tv_blob_set(b: *mut blob_T, idx: c_int, c: u8) {
    unsafe {
        *(*b).bv_ga.ga_data.cast::<u8>().add(idx as usize) = c;
    }
}

#[inline]
unsafe fn tv_blob_set_ret(tv: *mut typval_T, b: *mut blob_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_BLOB;
        (*tv).vval.v_ptr = b.cast::<c_void>();
        if !b.is_null() {
            (*b).bv_refcount += 1;
        }
    }
}

#[cfg(not(test))]
unsafe fn filter_map_one(
    tv: *mut typval_T,
    expr: *mut typval_T,
    filtermap: FilterMap,
    newtv: *mut typval_T,
    remp: *mut bool,
) -> c_int {
    unsafe {
        let mut retval = FAIL;
        tv_copy(tv, get_vim_var_tv(VV_VAL));

        (*newtv).v_type = VarType::VAR_UNKNOWN;
        if filtermap == FilterMap::ForEach && (*expr).v_type == VarType::VAR_STRING {
            do_cmdline_cmd((*expr).vval.v_string);
            if did_emsg == 0 {
                retval = OK;
            }
            tv_clear(get_vim_var_tv(VV_VAL));
            return retval;
        }

        let mut argv = [
            *get_vim_var_tv(VV_KEY),
            *get_vim_var_tv(VV_VAL),
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        if eval_expr_typval(expr, false, argv.as_mut_ptr(), 2, newtv) == FAIL {
            tv_clear(get_vim_var_tv(VV_VAL));
            return FAIL;
        }
        if filtermap == FilterMap::Filter {
            let mut error = false;
            *remp = tv_get_number_chk(newtv, &mut error) == 0;
            tv_clear(newtv);
            if error {
                tv_clear(get_vim_var_tv(VV_VAL));
                return FAIL;
            }
        } else if filtermap == FilterMap::ForEach {
            tv_clear(newtv);
        }
        retval = OK;
        tv_clear(get_vim_var_tv(VV_VAL));
        retval
    }
}

#[cfg(not(test))]
unsafe fn filter_map_dict(
    d: *mut dict_T,
    filtermap: FilterMap,
    arg_errmsg: *const c_char,
    expr: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            (*rettv).v_type = VarType::VAR_DICT;
            (*rettv).vval.v_ptr = std::ptr::null_mut();
        }
        if d.is_null()
            || (filtermap == FilterMap::Filter
                && value_check_lock((*d).dv_lock, arg_errmsg, TV_TRANSLATE))
        {
            return;
        }

        let mut d_ret: *mut dict_T = std::ptr::null_mut();
        if filtermap == FilterMap::MapNew {
            tv_dict_alloc_ret(rettv);
            d_ret = (*rettv).vval.v_ptr.cast::<dict_T>();
        }

        let prev_lock = (*d).dv_lock;
        if (*d).dv_lock == VAR_UNLOCKED {
            (*d).dv_lock = VAR_LOCKED;
        }
        hash_lock(&mut (*d).dv_hashtab);

        let key_off = std::mem::offset_of!(dictitem_T, di_key);
        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = (*hi).hi_key.sub(key_off).cast::<dictitem_T>();
                if filtermap == FilterMap::Map
                    && (value_check_lock((*di).di_tv.v_lock, arg_errmsg, TV_TRANSLATE)
                        || var_check_ro((*di).di_flags as c_int, arg_errmsg, TV_TRANSLATE))
                {
                    break;
                }
                let key_ptr = (*di).di_key.as_ptr();
                set_vim_var_string(VV_KEY, key_ptr, -1);
                let mut newtv = typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval { v_number: 0 },
                };
                let mut rem = false;
                let r = filter_map_one(&mut (*di).di_tv, expr, filtermap, &mut newtv, &mut rem);
                tv_clear(get_vim_var_tv(VV_KEY));
                if r == FAIL || did_emsg != 0 {
                    tv_clear(&mut newtv);
                    break;
                }
                if filtermap == FilterMap::Map {
                    tv_clear(&mut (*di).di_tv);
                    newtv.v_lock = VAR_UNLOCKED;
                    (*di).di_tv = newtv;
                } else if filtermap == FilterMap::MapNew {
                    let key_len = CStr::from_ptr(key_ptr).to_bytes().len();
                    let add_res = tv_dict_add_tv(d_ret, key_ptr, key_len, &mut newtv);
                    tv_clear(&mut newtv);
                    if add_res == FAIL {
                        break;
                    }
                } else if filtermap == FilterMap::Filter && rem {
                    if var_check_fixed((*di).di_flags as c_int, arg_errmsg, TV_TRANSLATE)
                        || var_check_ro((*di).di_flags as c_int, arg_errmsg, TV_TRANSLATE)
                    {
                        break;
                    }
                    tv_dict_item_remove(d, di);
                }
            }
            hi = hi.add(1);
        }

        hash_unlock(&mut (*d).dv_hashtab);
        (*d).dv_lock = prev_lock;
    }
}

#[cfg(not(test))]
unsafe fn filter_map_blob(
    blob_arg: *mut blob_T,
    filtermap: FilterMap,
    expr: *mut typval_T,
    arg_errmsg: *const c_char,
    rettv: *mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            (*rettv).v_type = VarType::VAR_BLOB;
            (*rettv).vval.v_ptr = std::ptr::null_mut();
        }
        let b = blob_arg;
        if b.is_null()
            || (filtermap == FilterMap::Filter
                && value_check_lock((*b).bv_lock, arg_errmsg, TV_TRANSLATE))
        {
            return;
        }

        let mut b_ret = b;
        if filtermap == FilterMap::MapNew {
            tv_blob_copy(b, rettv);
            b_ret = (*rettv).vval.v_ptr.cast::<blob_T>();
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let prev_lock = (*b).bv_lock;
        if (*b).bv_lock == 0 {
            (*b).bv_lock = VAR_LOCKED;
        }

        let mut i: c_int = 0;
        let mut idx: i64 = 0;
        while i < (*b).bv_ga.ga_len {
            let val = tv_blob_get(b, i) as i64;
            let mut tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: val },
            };
            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
            let mut rem = false;
            if filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem) == FAIL
                || did_emsg != 0
            {
                break;
            }
            if filtermap != FilterMap::ForEach {
                if newtv.v_type != VarType::VAR_NUMBER && newtv.v_type != VarType::VAR_BOOL {
                    tv_clear(&mut newtv);
                    emsg(c"E978: Invalid operation for Blob".as_ptr());
                    break;
                }
                if filtermap != FilterMap::Filter {
                    if newtv.vval.v_number != val {
                        tv_blob_set(b_ret, i, newtv.vval.v_number as u8);
                    }
                } else if rem {
                    let p = (*blob_arg).bv_ga.ga_data.cast::<u8>();
                    let tail_len = ((*b).bv_ga.ga_len - i - 1) as usize;
                    std::ptr::copy(p.add((i + 1) as usize), p.add(i as usize), tail_len);
                    (*b).bv_ga.ga_len -= 1;
                    i -= 1;
                }
            }
            i += 1;
            idx += 1;
        }

        (*b).bv_lock = prev_lock;
    }
}

#[cfg(not(test))]
unsafe fn filter_map_string(
    str_ptr: *const c_char,
    filtermap: FilterMap,
    expr: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = std::ptr::null_mut();

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: std::ptr::null_mut(),
        };
        ga_init(&mut ga, 1, 80);
        let mut idx: i64 = 0;
        let mut p = str_ptr;
        while *p != 0 {
            let len = utfc_ptr2len(p) as usize;
            let mut tv = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: xmemdupz(p.cast::<c_void>(), len).cast::<c_char>(),
                },
            };

            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
            let mut rem = false;
            if filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem) == FAIL
                || did_emsg != 0
            {
                tv_clear(&mut newtv);
                tv_clear(&mut tv);
                break;
            }
            if filtermap == FilterMap::Map || filtermap == FilterMap::MapNew {
                if newtv.v_type != VarType::VAR_STRING {
                    tv_clear(&mut newtv);
                    tv_clear(&mut tv);
                    emsg(c"E928: String required".as_ptr());
                    break;
                } else {
                    ga_concat(&mut ga, newtv.vval.v_string);
                }
            } else if filtermap == FilterMap::ForEach || !rem {
                ga_concat(&mut ga, tv.vval.v_string);
            }

            tv_clear(&mut newtv);
            tv_clear(&mut tv);

            idx += 1;
            p = p.add(len);
        }
        ga_append(&mut ga, 0);
        (*rettv).vval.v_string = ga.ga_data.cast::<c_char>();
    }
}

#[cfg(not(test))]
unsafe fn filter_map_list(
    l: *mut list_T,
    filtermap: FilterMap,
    arg_errmsg: *const c_char,
    expr: *mut typval_T,
    rettv: *mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            (*rettv).v_type = VarType::VAR_LIST;
            (*rettv).vval.v_list = std::ptr::null_mut();
        }
        if l.is_null()
            || (filtermap == FilterMap::Filter
                && value_check_lock(tv_list_locked(l), arg_errmsg, TV_TRANSLATE))
        {
            return;
        }

        let mut l_ret: *mut list_T = std::ptr::null_mut();
        if filtermap == FilterMap::MapNew {
            tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN);
            l_ret = (*rettv).vval.v_list;
        }
        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let prev_lock = tv_list_locked(l);
        if prev_lock == VAR_UNLOCKED {
            tv_list_set_lock(l, VAR_LOCKED);
        }

        let mut idx: i64 = 0;
        let mut li = tv_list_first(l);
        while !li.is_null() {
            if filtermap == FilterMap::Map
                && value_check_lock((*li).li_tv.v_lock, arg_errmsg, TV_TRANSLATE)
            {
                break;
            }
            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };
            let mut rem = false;
            if filter_map_one(&mut (*li).li_tv, expr, filtermap, &mut newtv, &mut rem) == FAIL {
                break;
            }
            if did_emsg != 0 {
                tv_clear(&mut newtv);
                break;
            }
            if filtermap == FilterMap::Map {
                tv_clear(&mut (*li).li_tv);
                newtv.v_lock = VAR_UNLOCKED;
                (*li).li_tv = newtv;
            } else if filtermap == FilterMap::MapNew {
                tv_list_append_owned_tv(l_ret, newtv);
            }
            if filtermap == FilterMap::Filter && rem {
                li = tv_list_item_remove(l, li);
            } else {
                li = (*li).li_next;
            }
            idx += 1;
        }

        tv_list_set_lock(l, prev_lock);
    }
}

#[cfg(not(test))]
unsafe fn filter_map(argvars: *mut typval_T, rettv: *mut typval_T, filtermap: FilterMap) {
    unsafe {
        let (func_name, arg_errmsg) = match filtermap {
            FilterMap::Map => (c"map()".as_ptr(), c"map() argument".as_ptr()),
            FilterMap::MapNew => (c"mapnew()".as_ptr(), c"mapnew() argument".as_ptr()),
            FilterMap::Filter => (c"filter()".as_ptr(), c"filter() argument".as_ptr()),
            FilterMap::ForEach => (c"foreach()".as_ptr(), c"foreach() argument".as_ptr()),
        };

        let arg0 = &*argvars;
        if filtermap != FilterMap::MapNew && arg0.v_type != VarType::VAR_STRING {
            tv_copy(argvars, rettv);
        }

        if !matches!(
            arg0.v_type,
            VarType::VAR_BLOB | VarType::VAR_LIST | VarType::VAR_DICT | VarType::VAR_STRING
        ) {
            semsg!(
                c"E1250: Argument of %s must be a List, String, Dictionary or Blob".as_ptr(),
                func_name,
            );
            return;
        }

        let expr = argvars.add(1);
        if (*expr).v_type == VarType::VAR_UNKNOWN {
            return;
        }

        let mut save_val = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        };
        let mut save_key = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval { v_number: 0 },
        };
        prepare_vimvar(VV_VAL, &mut save_val);
        prepare_vimvar(VV_KEY, &mut save_key);

        let save_did_emsg = did_emsg;
        did_emsg = 0;

        match arg0.v_type {
            VarType::VAR_DICT => {
                filter_map_dict(
                    arg0.vval.v_ptr.cast::<dict_T>(),
                    filtermap,
                    arg_errmsg,
                    expr,
                    rettv,
                );
            }
            VarType::VAR_BLOB => {
                filter_map_blob(
                    arg0.vval.v_ptr.cast::<blob_T>(),
                    filtermap,
                    expr,
                    arg_errmsg,
                    rettv,
                );
            }
            VarType::VAR_STRING => {
                filter_map_string(tv_get_string(argvars), filtermap, expr, rettv);
            }
            VarType::VAR_LIST => {
                filter_map_list(arg0.vval.v_list, filtermap, arg_errmsg, expr, rettv);
            }
            _ => unreachable!(),
        }

        restore_vimvar(VV_KEY, &mut save_key);
        restore_vimvar(VV_VAL, &mut save_val);

        did_emsg |= save_did_emsg;
    }
}

/// `"filter()"` function (`f_filter`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_filter(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        filter_map(argvars, rettv, FilterMap::Filter);
    }
}

/// `"map()"` function (`f_map`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_map(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        filter_map(argvars, rettv, FilterMap::Map);
    }
}

/// `"mapnew()"` function (`f_mapnew`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_mapnew(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        filter_map(argvars, rettv, FilterMap::MapNew);
    }
}

/// `"foreach()"` function (`f_foreach`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_foreach(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        filter_map(argvars, rettv, FilterMap::ForEach);
    }
}

/// `"add(list, item)"` function (`f_add`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_add(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = 1;
        let arg0 = &*argvars;
        if arg0.v_type == VarType::VAR_LIST {
            let l = arg0.vval.v_list;
            if !value_check_lock(tv_list_locked(l), c"add() argument".as_ptr(), TV_TRANSLATE) {
                tv_list_append_tv(l, argvars.add(1));
                tv_copy(argvars, rettv);
            }
        } else if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_ptr.cast::<blob_T>();
            if !b.is_null()
                && !value_check_lock((*b).bv_lock, c"add() argument".as_ptr(), TV_TRANSLATE)
            {
                let mut error = false;
                let n = tv_get_number_chk(argvars.add(1), &mut error);
                if !error {
                    ga_append(&mut (*b).bv_ga, n as u8);
                    tv_copy(argvars, rettv);
                }
            }
        } else {
            emsg(c"E897: List or Blob required".as_ptr());
        }
    }
}

#[cfg(not(test))]
unsafe fn count_string(haystack: *const c_char, needle: *const c_char, ic: bool) -> i64 {
    unsafe {
        if haystack.is_null() || needle.is_null() || *needle == 0 {
            return 0;
        }
        let needle_bytes = CStr::from_ptr(needle).to_bytes();
        let needlelen = needle_bytes.len();
        let mut n: i64 = 0;
        let mut p = haystack;

        if ic {
            while *p != 0 {
                if mb_strnicmp(p, needle, needlelen) == 0 {
                    n += 1;
                    p = p.add(needlelen);
                } else {
                    p = p.add(utf_ptr2len(p) as usize);
                }
            }
        } else {
            let hay_bytes = CStr::from_ptr(haystack).to_bytes();
            let mut offset = 0usize;
            while offset + needlelen <= hay_bytes.len() {
                if let Some(pos) = hay_bytes[offset..]
                    .windows(needlelen)
                    .position(|w| w == needle_bytes)
                {
                    n += 1;
                    offset += pos + needlelen;
                } else {
                    break;
                }
            }
        }
        n
    }
}

#[cfg(not(test))]
unsafe fn count_list(l: *mut list_T, needle: *mut typval_T, idx: i64, ic: bool) -> i64 {
    unsafe {
        if tv_list_len(l) == 0 {
            return 0;
        }
        let mut li = tv_list_find(l, idx as c_int);
        if li.is_null() {
            semsg!(e_list_index_out_of_range_nr.as_ptr(), idx);
            return 0;
        }
        let mut n: i64 = 0;
        while !li.is_null() {
            if tv_equal(&mut (*li).li_tv, needle, ic) {
                n += 1;
            }
            li = (*li).li_next;
        }
        n
    }
}

#[cfg(not(test))]
unsafe fn count_dict(d: *mut dict_T, needle: *mut typval_T, ic: bool) -> i64 {
    unsafe {
        if d.is_null() {
            return 0;
        }
        let mut n: i64 = 0;
        let key_off = std::mem::offset_of!(dictitem_T, di_key);
        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = (*hi).hi_key.sub(key_off).cast::<dictitem_T>();
                if tv_equal(&mut (*di).di_tv, needle, ic) {
                    n += 1;
                }
            }
            hi = hi.add(1);
        }
        n
    }
}

/// `"count()"` function (`f_count`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_count(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let mut n: i64 = 0;
        let mut ic = false;
        let mut error = false;

        let arg0 = &*argvars;
        let arg1 = argvars.add(1);
        let arg2 = &*argvars.add(2);
        let arg3 = &*argvars.add(3);

        if arg2.v_type != VarType::VAR_UNKNOWN {
            ic = tv_get_bool_chk(arg2, &mut error) != 0;
        }

        if !error && arg0.v_type == VarType::VAR_STRING {
            n = count_string(arg0.vval.v_string, tv_get_string_chk(arg1), ic);
        } else if !error && arg0.v_type == VarType::VAR_LIST {
            let mut idx: i64 = 0;
            if arg2.v_type != VarType::VAR_UNKNOWN && arg3.v_type != VarType::VAR_UNKNOWN {
                idx = tv_get_number_chk(arg3, &mut error);
            }
            if !error {
                n = count_list(arg0.vval.v_list, arg1, idx, ic);
            }
        } else if !error && arg0.v_type == VarType::VAR_DICT {
            let d = arg0.vval.v_ptr.cast::<dict_T>();
            if !d.is_null() {
                if arg2.v_type != VarType::VAR_UNKNOWN && arg3.v_type != VarType::VAR_UNKNOWN {
                    emsg(c"E474: Invalid argument".as_ptr());
                } else {
                    n = count_dict(d, arg1, ic);
                }
            }
        } else if !error {
            semsg!(
                c"E706: Argument of %s must be a List, String or Dictionary".as_ptr(),
                c"count()".as_ptr(),
            );
        }
        (*rettv).vval.v_number = n;
    }
}

#[cfg(not(test))]
unsafe fn extend_dict(
    argvars: *mut typval_T,
    arg_errmsg: *const c_char,
    is_new: bool,
    rettv: *mut typval_T,
) {
    unsafe {
        let mut d1 = (*argvars).vval.v_ptr.cast::<dict_T>();
        if d1.is_null() {
            let locked = value_check_lock(VAR_FIXED, arg_errmsg, TV_TRANSLATE);
            assert!(locked);
            return;
        }

        if !is_new && value_check_lock((*d1).dv_lock, arg_errmsg, TV_TRANSLATE) {
            return;
        }

        if is_new {
            d1 = tv_dict_copy(std::ptr::null(), d1, false, get_copyID());
            if d1.is_null() {
                return;
            }
        }

        let d2 = (*argvars.add(1)).vval.v_ptr.cast::<dict_T>();
        if !d2.is_null() {
            let mut action = c"force".as_ptr();
            let arg2 = &*argvars.add(2);
            if arg2.v_type != VarType::VAR_UNKNOWN {
                action = tv_get_string_chk(arg2);
                if action.is_null() {
                    if is_new {
                        tv_dict_unref(d1);
                    }
                    return;
                }
                let action_bytes = CStr::from_ptr(action).to_bytes();
                if action_bytes != b"keep" && action_bytes != b"force" && action_bytes != b"error" {
                    if is_new {
                        tv_dict_unref(d1);
                    }
                    semsg!(c"E475: Invalid argument: %s".as_ptr(), action);
                    return;
                }
            }
            tv_dict_extend(d1, d2, action);
        }

        if is_new {
            *rettv = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_ptr: d1.cast::<c_void>(),
                },
            };
        } else {
            tv_copy(argvars, rettv);
        }
    }
}

#[cfg(not(test))]
unsafe fn extend_list(
    argvars: *mut typval_T,
    arg_errmsg: *const c_char,
    is_new: bool,
    rettv: *mut typval_T,
) {
    unsafe {
        let mut error = false;
        let mut l1 = (*argvars).vval.v_list;

        if !is_new && value_check_lock(tv_list_locked(l1), arg_errmsg, TV_TRANSLATE) {
            return;
        }

        if is_new {
            l1 = tv_list_copy(std::ptr::null(), l1, false, get_copyID());
            if l1.is_null() {
                return;
            }
        }

        let l2 = (*argvars.add(1)).vval.v_list;
        if !l2.is_null() {
            let mut item: *mut listitem_T = std::ptr::null_mut();
            let arg2 = &*argvars.add(2);
            if arg2.v_type != VarType::VAR_UNKNOWN {
                let before = tv_get_number_chk(arg2, &mut error) as c_int;
                if error {
                    if is_new {
                        tv_list_unref(l1);
                    }
                    return;
                }
                if before != tv_list_len(l1) {
                    item = tv_list_find(l1, before);
                    if item.is_null() {
                        semsg!(e_list_index_out_of_range_nr.as_ptr(), before as i64);
                        if is_new {
                            tv_list_unref(l1);
                        }
                        return;
                    }
                }
            }
            tv_list_extend(l1, l2, item);
        }

        if is_new {
            *rettv = typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_list: l1 },
            };
        } else {
            tv_copy(argvars, rettv);
        }
    }
}

#[cfg(not(test))]
unsafe fn extend_impl(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
    is_new: bool,
) {
    unsafe {
        let t0 = (*argvars).v_type;
        let t1 = (*argvars.add(1)).v_type;
        if t0 == VarType::VAR_LIST && t1 == VarType::VAR_LIST {
            extend_list(argvars, arg_errmsg, is_new, rettv);
        } else if t0 == VarType::VAR_DICT && t1 == VarType::VAR_DICT {
            extend_dict(argvars, arg_errmsg, is_new, rettv);
        } else {
            semsg!(
                c"E712: Argument of %s must be a List or Dictionary".as_ptr(),
                if is_new {
                    c"extendnew()".as_ptr()
                } else {
                    c"extend()".as_ptr()
                },
            );
        }
    }
}

/// `"extend()"` function (`f_extend`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_extend(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        extend_impl(argvars, rettv, c"extend() argument".as_ptr(), false);
    }
}

/// `"extendnew()"` function (`f_extendnew`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_extendnew(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        extend_impl(argvars, rettv, c"extendnew() argument".as_ptr(), true);
    }
}

/// `"insert()"` function (`f_insert`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_insert(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let mut error = false;
        let arg0 = &*argvars;
        let arg1 = argvars.add(1);
        let arg2 = argvars.add(2);

        if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_ptr.cast::<blob_T>();
            if b.is_null()
                || value_check_lock((*b).bv_lock, c"insert() argument".as_ptr(), TV_TRANSLATE)
            {
                return;
            }

            let mut before: c_int = 0;
            let len = tv_blob_len(b);

            if (*arg2).v_type != VarType::VAR_UNKNOWN {
                before = tv_get_number_chk(arg2, &mut error) as c_int;
                if error {
                    return;
                }
                if before < 0 || before > len {
                    semsg!(c"E475: Invalid argument: %s".as_ptr(), tv_get_string(arg2));
                    return;
                }
            }
            let val = tv_get_number_chk(arg1, &mut error) as c_int;
            if error {
                return;
            }
            if !(0..=255).contains(&val) {
                semsg!(c"E475: Invalid argument: %s".as_ptr(), tv_get_string(arg1));
                return;
            }

            ga_grow(&mut (*b).bv_ga, 1);
            let p = (*b).bv_ga.ga_data.cast::<u8>();
            std::ptr::copy(
                p.add(before as usize),
                p.add((before + 1) as usize),
                (len - before) as usize,
            );
            *p.add(before as usize) = val as u8;
            (*b).bv_ga.ga_len += 1;

            tv_copy(argvars, rettv);
        } else if arg0.v_type != VarType::VAR_LIST {
            semsg!(
                c"E899: Argument of %s must be a List or Blob".as_ptr(),
                c"insert()".as_ptr(),
            );
        } else {
            let mut l = arg0.vval.v_list;
            if value_check_lock(
                tv_list_locked(l),
                c"insert() argument".as_ptr(),
                TV_TRANSLATE,
            ) {
                return;
            }

            let mut before: i64 = 0;
            if (*arg2).v_type != VarType::VAR_UNKNOWN {
                before = tv_get_number_chk(arg2, &mut error);
            }
            if error {
                return;
            }

            let mut item: *mut listitem_T = std::ptr::null_mut();
            if before != (tv_list_len(l) as i64) {
                item = tv_list_find(l, before as c_int);
                if item.is_null() {
                    semsg!(e_list_index_out_of_range_nr.as_ptr(), before);
                    l = std::ptr::null_mut();
                }
            }
            if !l.is_null() {
                tv_list_insert_tv(l, arg1, item);
                tv_copy(argvars, rettv);
            }
        }
    }
}

/// `"remove()"` function (`f_remove`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        let arg_errmsg = c"remove() argument".as_ptr();
        match (*argvars).v_type {
            VarType::VAR_DICT => tv_dict_remove(argvars, rettv, arg_errmsg),
            VarType::VAR_BLOB => tv_blob_remove(argvars, rettv, arg_errmsg),
            VarType::VAR_LIST => tv_list_remove(argvars, rettv, arg_errmsg),
            _ => {
                semsg!(
                    c"E896: Argument of %s must be a List, Dictionary or Blob".as_ptr(),
                    c"remove()".as_ptr(),
                );
            }
        }
    }
}

/// `"reverse({list})"` function (`f_reverse`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_reverse(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        if tv_check_for_string_or_list_or_blob_arg(argvars, 0) == FAIL {
            return;
        }

        let arg0 = &*argvars;
        if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_ptr.cast::<blob_T>();
            let len = tv_blob_len(b);
            for i in 0..(len / 2) {
                let tmp = tv_blob_get(b, i);
                tv_blob_set(b, i, tv_blob_get(b, len - i - 1));
                tv_blob_set(b, len - i - 1, tmp);
            }
            tv_blob_set_ret(rettv, b);
        } else if arg0.v_type == VarType::VAR_STRING {
            (*rettv).v_type = VarType::VAR_STRING;
            if !arg0.vval.v_string.is_null() {
                (*rettv).vval.v_string = reverse_text(arg0.vval.v_string);
            } else {
                (*rettv).vval.v_string = std::ptr::null_mut();
            }
        } else if arg0.v_type == VarType::VAR_LIST {
            let l = arg0.vval.v_list;
            if !value_check_lock(
                tv_list_locked(l),
                c"reverse() argument".as_ptr(),
                TV_TRANSLATE,
            ) {
                tv_list_reverse(l);
                tv_list_set_ret(rettv, l);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lock_and_len_helpers_handle_null() {
        unsafe {
            assert_eq!(tv_list_locked(std::ptr::null()), VAR_FIXED);
            assert_eq!(tv_list_len(std::ptr::null()), 0);
            assert!(tv_list_first(std::ptr::null()).is_null());
            assert_eq!(tv_blob_len(std::ptr::null()), 0);
        }
    }
}
