//! List support and container (`List`, `Dict`, `Blob`) functions (`src/nvim/eval/list.c`).

use crate::buffer::VAR_FIXED;
use crate::errors::*;
use crate::eval::fs::K_LIST_LEN_UNKNOWN;
use crate::eval::typval::{
    EvalFuncData, TV_TRANSLATE, TypvalVval, VAR_LOCKED, VAR_UNLOCKED, VarType, blob_T, dict_T,
    list_T, listitem_T, tv_blob_copy, tv_blob_get, tv_blob_len, tv_blob_remove,
    tv_check_for_string_or_list_or_blob_arg, tv_clear, tv_copy, tv_dict_add_tv, tv_dict_alloc_ret,
    tv_dict_copy, tv_dict_extend, tv_dict_item_remove, tv_dict_remove, tv_dict_unref, tv_equal,
    tv_get_bool_chk, tv_get_number_chk, tv_get_string, tv_get_string_chk, tv_list_alloc_ret,
    tv_list_append_owned_tv, tv_list_append_tv, tv_list_copy, tv_list_extend, tv_list_find,
    tv_list_first, tv_list_insert_tv, tv_list_item_remove, tv_list_len, tv_list_remove,
    tv_list_reverse, tv_list_set_lock, tv_list_set_ret, tv_list_unref, typval_T, value_check_lock,
};
use crate::eval::vars::{
    VV_KEY, VV_VAL, get_vim_var_tv, prepare_vimvar, restore_vimvar, set_vim_var_nr,
    set_vim_var_string, set_vim_var_type, tv_list_locked, var_check_fixed, var_check_ro,
};
use crate::eval::{eval_expr_typval, get_copyID, tv_blob_set_ret};
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::*;
use crate::hashtab::{hash_lock, hash_unlock, hashitem_empty};
use crate::mbyte::{mb_strnicmp, utf_ptr2len, utfc_ptr2len};
use crate::message::{emsg, semsg};
use crate::strings::reverse_text;
use std::ffi::{c_char, c_int};

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterMap {
    Filter,
    Map,
    MapNew,
    ForEach,
}

#[inline]
fn tv_blob_set(b: &mut blob_T, idx: c_int, c: u8) {
    (&mut b.bv_ga)[idx as usize] = c;
}

fn filter_map_one(
    tv: &mut typval_T,
    expr: &mut typval_T,
    filtermap: FilterMap,
    newtv: &mut typval_T,
    remp: &mut bool,
) -> bool {
    unsafe {
        let mut retval = false;
        tv_copy(&*tv, &mut *get_vim_var_tv(VV_VAL));

        newtv.v_type = VarType::VAR_UNKNOWN;
        if filtermap == FilterMap::ForEach && expr.v_type == VarType::VAR_STRING {
            do_cmdline_cmd(expr.vval.v_string());
            if g_st().did_emsg == 0 {
                retval = true;
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
                vval: TypvalVval::Number(0),
            },
        ];
        if !eval_expr_typval(&*expr, false, argv.as_mut_ptr(), 2, &mut *newtv) {
            tv_clear(get_vim_var_tv(VV_VAL));
            return false;
        }
        if filtermap == FilterMap::Filter {
            let mut error = false;
            *remp = tv_get_number_chk(&*newtv, Some(&mut error)) == 0;
            tv_clear(newtv);
            if error {
                tv_clear(get_vim_var_tv(VV_VAL));
                return false;
            }
        } else if filtermap == FilterMap::ForEach {
            tv_clear(newtv);
        }
        retval = true;
        tv_clear(get_vim_var_tv(VV_VAL));
        retval
    }
}

fn filter_map_dict(
    d: *mut dict_T,
    filtermap: FilterMap,
    arg_errmsg: *const c_char,
    expr: &mut typval_T,
    rettv: &mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            rettv.v_type = VarType::VAR_DICT;
            rettv.vval = crate::eval::typval::TypvalVval::Dict(std::ptr::null_mut());
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
            d_ret = rettv.vval.v_dict();
        }

        let prev_lock = (*d).dv_lock;
        if (*d).dv_lock == VAR_UNLOCKED {
            (*d).dv_lock = VAR_LOCKED;
        }
        hash_lock(&mut (*d).dv_hashtab);

        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.array_ptr();
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = crate::eval::typval::tv_dict_hi2di(hi);
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
                    vval: TypvalVval::Number(0),
                };
                let mut rem = false;
                let r = filter_map_one(&mut (*di).di_tv, expr, filtermap, &mut newtv, &mut rem);
                tv_clear(get_vim_var_tv(VV_KEY));
                if !r || g_st().did_emsg != 0 {
                    tv_clear(&mut newtv);
                    break;
                }
                if filtermap == FilterMap::Map {
                    tv_clear(&mut (*di).di_tv);
                    newtv.v_lock = VAR_UNLOCKED;
                    (*di).di_tv = newtv;
                } else if filtermap == FilterMap::MapNew {
                    let key_len = crate::memory::cstr_len(key_ptr);
                    let add_res = tv_dict_add_tv(d_ret, key_ptr, key_len, &mut newtv);
                    tv_clear(&mut newtv);
                    if !add_res {
                        break;
                    }
                } else if filtermap == FilterMap::Filter && rem {
                    if var_check_fixed((*di).di_flags as c_int, arg_errmsg, TV_TRANSLATE)
                        || var_check_ro((*di).di_flags as c_int, arg_errmsg, TV_TRANSLATE)
                    {
                        break;
                    }
                    tv_dict_item_remove(&mut *d,  &mut *di);
                }
            }
            hi = hi.add(1);
        }

        hash_unlock(&mut (*d).dv_hashtab);
        (*d).dv_lock = prev_lock;
    }
}

fn filter_map_blob(
    blob_arg: *mut blob_T,
    filtermap: FilterMap,
    expr: &mut typval_T,
    arg_errmsg: *const c_char,
    rettv: &mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            rettv.v_type = VarType::VAR_BLOB;
            rettv.vval = crate::eval::typval::TypvalVval::Blob(std::ptr::null_mut());
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
            tv_blob_copy(b, &mut *rettv);
            b_ret = rettv.vval.v_blob();
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let prev_lock = (*b).bv_lock;
        if (*b).bv_lock == 0 {
            (*b).bv_lock = VAR_LOCKED;
        }

        let mut i: c_int = 0;
        let mut idx: i64 = 0;
        while i < (*b).bv_ga.len() as c_int {
            let val = tv_blob_get( &*b, i) as i64;
            let mut tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(val),
            };
            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            let mut rem = false;
            if !filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem)
                || g_st().did_emsg != 0
            {
                break;
            }
            if filtermap != FilterMap::ForEach {
                if newtv.v_type != VarType::VAR_NUMBER && newtv.v_type != VarType::VAR_BOOL {
                    tv_clear(&mut newtv);
                    emsg(e_invalblob);
                    break;
                }
                if filtermap != FilterMap::Filter {
                    if newtv.vval.v_number() != val {
                        tv_blob_set(&mut *b_ret, i, newtv.vval.v_number() as u8);
                    }
                } else if rem {
                    let bv = &mut (*blob_arg).bv_ga;
                    bv.remove(i as usize);
                    i -= 1;
                }
            }
            i += 1;
            idx += 1;
        }

        (*b).bv_lock = prev_lock;
    }
}

fn filter_map_string(
    str_ptr: *const c_char,
    filtermap: FilterMap,
    expr: &mut typval_T,
    rettv: &mut typval_T,
) {
    unsafe {
        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(std::ptr::null_mut());

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let mut out: Vec<u8> = Vec::with_capacity(80);
        let mut idx: i64 = 0;
        let mut p = str_ptr;
        while *p != 0 {
            let len = utfc_ptr2len(p) as usize;
            let mut tv = typval_T::new();
            crate::eval::typval::tv_set_string_len(&mut tv, p, len);

            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            let mut rem = false;
            if !filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem)
                || g_st().did_emsg != 0
            {
                tv_clear(&mut newtv);
                tv_clear(&mut tv);
                break;
            }
            if filtermap == FilterMap::Map || filtermap == FilterMap::MapNew {
                if newtv.v_type != VarType::VAR_STRING {
                    tv_clear(&mut newtv);
                    tv_clear(&mut tv);
                    emsg(e_string_required);
                    break;
                } else if !newtv.vval.v_string().is_null() {
                    out.extend_from_slice(crate::memory::cstr_as_bytes(newtv.vval.v_string()));
                }
            } else if (filtermap == FilterMap::ForEach || !rem) && !tv.vval.v_string().is_null() {
                out.extend_from_slice(crate::memory::cstr_as_bytes(tv.vval.v_string()));
            }

            tv_clear(&mut newtv);
            tv_clear(&mut tv);

            idx += 1;
            p = p.add(len);
        }
        crate::eval::typval::tv_set_string_len(&mut *rettv, out.as_ptr().cast::<c_char>(), out.len());
    }
}

fn filter_map_list(
    l: *mut list_T,
    filtermap: FilterMap,
    arg_errmsg: *const c_char,
    expr: &mut typval_T,
    rettv: &mut typval_T,
) {
    unsafe {
        if filtermap == FilterMap::MapNew {
            rettv.v_type = VarType::VAR_LIST;
            rettv.vval = crate::eval::typval::TypvalVval::List(std::ptr::null_mut());
        }
        if l.is_null()
            || (filtermap == FilterMap::Filter
                && value_check_lock(tv_list_locked(l), arg_errmsg, TV_TRANSLATE))
        {
            return;
        }

        let mut l_ret: *mut list_T = std::ptr::null_mut();
        if filtermap == FilterMap::MapNew {
            tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_UNKNOWN);
            l_ret = rettv.vval.v_list();
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
                vval: TypvalVval::Number(0),
            };
            let mut rem = false;
            if !filter_map_one(&mut (*li).li_tv, expr, filtermap, &mut newtv, &mut rem) {
                break;
            }
            if g_st().did_emsg != 0 {
                tv_clear(&mut newtv);
                break;
            }
            if filtermap == FilterMap::Map {
                tv_clear(&mut (*li).li_tv);
                newtv.v_lock = VAR_UNLOCKED;
                (*li).li_tv = newtv;
            } else if filtermap == FilterMap::MapNew {
                tv_list_append_owned_tv( &mut *l_ret, newtv);
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

fn filter_map(argvars: &mut [typval_T],rettv: &mut typval_T, filtermap: FilterMap) {
     {
        let (func_name, arg_errmsg) = match filtermap {
            FilterMap::Map => (c"map()".as_ptr(), c"map() argument".as_ptr()),
            FilterMap::MapNew => (c"mapnew()".as_ptr(), c"mapnew() argument".as_ptr()),
            FilterMap::Filter => (c"filter()".as_ptr(), c"filter() argument".as_ptr()),
            FilterMap::ForEach => (c"foreach()".as_ptr(), c"foreach() argument".as_ptr()),
        };

        let (first, rest) = argvars.split_at_mut(1);
        let arg0 = &first[0];
        if filtermap != FilterMap::MapNew && arg0.v_type != VarType::VAR_STRING {
            tv_copy(arg0, rettv);
        }

        if !matches!(
            arg0.v_type,
            VarType::VAR_BLOB | VarType::VAR_LIST | VarType::VAR_DICT | VarType::VAR_STRING
        ) {
            semsg!(
                c"E1250: Argument of %s must be a List, String, Dictionary or Blob",
                func_name,
            );
            return;
        }

        let expr = &mut rest[0];
        if expr.v_type == VarType::VAR_UNKNOWN {
            return;
        }

        let mut save_val = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        let mut save_key = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };
        prepare_vimvar(VV_VAL, &mut save_val);
        prepare_vimvar(VV_KEY, &mut save_key);

        let save_did_emsg = g_st().did_emsg;
        g_st().did_emsg = 0;

        match arg0.v_type {
            VarType::VAR_DICT => {
                filter_map_dict(arg0.vval.v_dict(), filtermap, arg_errmsg, expr, rettv);
            }
            VarType::VAR_BLOB => {
                filter_map_blob(arg0.vval.v_blob(), filtermap, expr, arg_errmsg, rettv);
            }
            VarType::VAR_STRING => {
                filter_map_string(tv_get_string(arg0), filtermap, expr, rettv);
            }
            VarType::VAR_LIST => {
                filter_map_list(arg0.vval.v_list(), filtermap, arg_errmsg, expr, rettv);
            }
            _ => unreachable!(),
        }

        restore_vimvar(VV_KEY, &mut save_key);
        restore_vimvar(VV_VAL, &mut save_val);

        g_st().did_emsg |= save_did_emsg;
    }
}

/// `"filter()"` function (`f_filter`).
pub fn f_filter(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    filter_map(argvars, rettv, FilterMap::Filter);
}

/// `"map()"` function (`f_map`).
pub fn f_map(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    filter_map(argvars, rettv, FilterMap::Map);
}

/// `"mapnew()"` function (`f_mapnew`).
pub fn f_mapnew(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    filter_map(argvars, rettv, FilterMap::MapNew);
}

/// `"foreach()"` function (`f_foreach`).
pub fn f_foreach(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    filter_map(argvars, rettv, FilterMap::ForEach);
}

/// `"add(list, item)"` function (`f_add`).
pub fn f_add(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        let arg0 = &argvars[0];
        if arg0.v_type == VarType::VAR_LIST {
            let l = arg0.vval.v_list();
            if !value_check_lock(tv_list_locked(l), c"add() argument", TV_TRANSLATE) {
                tv_list_append_tv( &mut *l, &mut argvars[1]);
                tv_copy(&argvars[0], rettv);
            }
        } else if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_blob();
            if !b.is_null()
                && !value_check_lock((*b).bv_lock, c"add() argument", TV_TRANSLATE)
            {
                let mut error = false;
                let n = tv_get_number_chk(&argvars[1], Some(&mut error));
                if !error {
                    let bv = &mut (*b).bv_ga;
                    bv.push(n as u8);
                    tv_copy(&argvars[0], rettv);
                }
            }
        } else {
            emsg(e_listblobreq);
        }
    }
}

fn count_string(haystack: *const c_char, needle: *const c_char, ic: bool) -> i64 {
    unsafe {
        if haystack.is_null() || needle.is_null() || *needle == 0 {
            return 0;
        }
        let needle_bytes = crate::memory::cstr_as_bytes(needle);
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
            let hay_bytes = crate::memory::cstr_as_bytes(haystack);
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

fn count_list(l: *mut list_T, needle: &typval_T, idx: i64, ic: bool) -> i64 {
    unsafe {
        if tv_list_len(l) == 0 {
            return 0;
        }
        let mut li = tv_list_find(l, idx as c_int);
        if li.is_null() {
            semsg!(e_list_index_out_of_range_nr, idx);
            return 0;
        }
        let mut n: i64 = 0;
        while !li.is_null() {
            if tv_equal(&(*li).li_tv, needle, ic) {
                n += 1;
            }
            li = (*li).li_next;
        }
        n
    }
}

fn count_dict(d: *mut dict_T, needle: &typval_T, ic: bool) -> i64 {
    unsafe {
        if d.is_null() {
            return 0;
        }
        let mut n: i64 = 0;
        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.array_ptr();
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = crate::eval::typval::tv_dict_hi2di(hi);
                if tv_equal(&(*di).di_tv, needle, ic) {
                    n += 1;
                }
            }
            hi = hi.add(1);
        }
        n
    }
}

/// `"count()"` function (`f_count`).
pub fn f_count(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    let mut n: i64 = 0;
    let mut ic = false;
    let mut error = false;

    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    let arg2 = &argvars[2];
    let arg3 = &argvars[3];

    if arg2.v_type != VarType::VAR_UNKNOWN {
        ic = tv_get_bool_chk(arg2, Some(&mut error)) != 0;
    }

    if !error && arg0.v_type == VarType::VAR_STRING {
        n = count_string(arg0.vval.v_string(), tv_get_string_chk(arg1), ic);
    } else if !error && arg0.v_type == VarType::VAR_LIST {
        let mut idx: i64 = 0;
        if arg2.v_type != VarType::VAR_UNKNOWN && arg3.v_type != VarType::VAR_UNKNOWN {
            idx = tv_get_number_chk(arg3, Some(&mut error));
        }
        if !error {
            n = count_list(arg0.vval.v_list(), arg1, idx, ic);
        }
    } else if !error && arg0.v_type == VarType::VAR_DICT {
        let d = arg0.vval.v_dict();
        if !d.is_null() {
            if arg2.v_type != VarType::VAR_UNKNOWN && arg3.v_type != VarType::VAR_UNKNOWN {
                emsg(e_invarg);
            } else {
                n = count_dict(d, arg1, ic);
            }
        }
    } else if !error {
        semsg!(
            c"E706: Argument of %s must be a List, String or Dictionary",
            c"count()",
        );
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(n);
}

fn extend_dict(argvars: &mut [typval_T],
    arg_errmsg: *const c_char,
    is_new: bool,
    rettv: &mut typval_T,
) {
    unsafe {
        let mut d1 = argvars[0].vval.v_dict();
        if d1.is_null() {
            let locked = value_check_lock(VAR_FIXED, arg_errmsg, TV_TRANSLATE);
            assert!(locked);
            return;
        }

        if !is_new && value_check_lock((*d1).dv_lock, arg_errmsg, TV_TRANSLATE) {
            return;
        }

        if is_new {
            d1 = tv_dict_copy(None, d1, false, get_copyID());
            if d1.is_null() {
                return;
            }
        }

        let d2 = argvars[1].vval.v_dict();
        if !d2.is_null() {
            let mut action = c"force".as_ptr();
            let arg2 = &argvars[2];
            if arg2.v_type != VarType::VAR_UNKNOWN {
                action = tv_get_string_chk(arg2);
                if action.is_null() {
                    if is_new {
                        tv_dict_unref(d1);
                    }
                    return;
                }
                let action_bytes = crate::memory::cstr_as_bytes(action);
                if action_bytes != b"keep" && action_bytes != b"force" && action_bytes != b"error" {
                    if is_new {
                        tv_dict_unref(d1);
                    }
                    semsg!(e_invarg2, action);
                    return;
                }
            }
            tv_dict_extend(&mut *d1, &mut *d2, action);
        }

        if is_new {
            *rettv = typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Dict(d1),
            };
        } else {
            tv_copy(&argvars[0], &mut *rettv);
        }
    }
}

fn extend_list(argvars: &mut [typval_T],
    arg_errmsg: *const c_char,
    is_new: bool,
    rettv: &mut typval_T,
) {
     {
        let mut error = false;
        let mut l1 = argvars[0].vval.v_list();

        if !is_new && value_check_lock(tv_list_locked(l1), arg_errmsg, TV_TRANSLATE) {
            return;
        }

        if is_new {
            l1 = tv_list_copy(None, l1, false, get_copyID());
            if l1.is_null() {
                return;
            }
        }

        let l2 = argvars[1].vval.v_list();
        if !l2.is_null() {
            let mut item: *mut listitem_T = std::ptr::null_mut();
            let arg2 = &argvars[2];
            if arg2.v_type != VarType::VAR_UNKNOWN {
                let before = tv_get_number_chk(arg2, Some(&mut error)) as c_int;
                if error {
                    if is_new {
                        tv_list_unref(l1);
                    }
                    return;
                }
                if before != tv_list_len(l1) {
                    item = tv_list_find(l1, before);
                    if item.is_null() {
                        semsg!(e_list_index_out_of_range_nr, before as i64);
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
                vval: TypvalVval::List(l1),
            };
        } else {
            tv_copy(&argvars[0], &mut *rettv);
        }
    }
}

fn extend_impl(argvars: &mut [typval_T],
    rettv: &mut typval_T,
    arg_errmsg: &std::ffi::CStr,
    is_new: bool,
) {
     {
        let t0 = argvars[0].v_type;
        let t1 = argvars[1].v_type;
        if t0 == VarType::VAR_LIST && t1 == VarType::VAR_LIST {
            extend_list(argvars, arg_errmsg.as_ptr(), is_new, rettv);
        } else if t0 == VarType::VAR_DICT && t1 == VarType::VAR_DICT {
            extend_dict(argvars, arg_errmsg.as_ptr(), is_new, rettv);
        } else {
            semsg!(
                e_listdictarg,
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
pub fn f_extend(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    extend_impl(argvars, rettv, c"extend() argument", false);
}

/// `"extendnew()"` function (`f_extendnew`).
pub fn f_extendnew(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    extend_impl(argvars, rettv, c"extendnew() argument", true);
}

/// `"insert()"` function (`f_insert`).
pub fn f_insert(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut error = false;
        let [arg0, arg1, arg2, ..] = argvars else {
            unreachable!()
        };

        if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_blob();
            if b.is_null()
                || value_check_lock((*b).bv_lock, c"insert() argument", TV_TRANSLATE)
            {
                return;
            }

            let mut before: c_int = 0;
            let len = tv_blob_len(b);

            if arg2.v_type != VarType::VAR_UNKNOWN {
                before = tv_get_number_chk(arg2, Some(&mut error)) as c_int;
                if error {
                    return;
                }
                if before < 0 || before > len {
                    semsg!(e_invarg2, tv_get_string(arg2));
                    return;
                }
            }
            let val = tv_get_number_chk(arg1, Some(&mut error)) as c_int;
            if error {
                return;
            }
            if !(0..=255).contains(&val) {
                semsg!(e_invarg2, tv_get_string(arg1));
                return;
            }

            let _ = len;
            let bv = &mut (*b).bv_ga;
            bv.insert(before as usize, val as u8);

            tv_copy(arg0, rettv);
        } else if arg0.v_type != VarType::VAR_LIST {
            semsg!(e_listblobarg, c"insert()");
        } else {
            let mut l = arg0.vval.v_list();
            if value_check_lock(
                tv_list_locked(l),
                c"insert() argument",
                TV_TRANSLATE,
            ) {
                return;
            }

            let mut before: i64 = 0;
            if arg2.v_type != VarType::VAR_UNKNOWN {
                before = tv_get_number_chk(arg2, Some(&mut error));
            }
            if error {
                return;
            }

            let mut item: *mut listitem_T = std::ptr::null_mut();
            if before != (tv_list_len(l) as i64) {
                item = tv_list_find(l, before as c_int);
                if item.is_null() {
                    semsg!(e_list_index_out_of_range_nr, before);
                    l = std::ptr::null_mut();
                }
            }
            if !l.is_null() {
                tv_list_insert_tv(l, arg1, item);
                tv_copy(arg0, rettv);
            }
        }
    }
}

/// `"remove()"` function (`f_remove`).
pub fn f_remove(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let arg_errmsg = c"remove() argument".as_ptr();
        match argvars[0].v_type {
            VarType::VAR_DICT => tv_dict_remove(argvars, rettv, arg_errmsg),
            VarType::VAR_BLOB => tv_blob_remove(argvars, rettv, arg_errmsg),
            VarType::VAR_LIST => tv_list_remove(argvars, rettv, arg_errmsg),
            _ => {
                semsg!(e_listdictblobarg, c"remove()");
            }
        }
    }
}

/// `"reverse({list})"` function (`f_reverse`).
pub fn f_reverse(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        if !tv_check_for_string_or_list_or_blob_arg(argvars, 0) {
            return;
        }

        let arg0 = &argvars[0];
        if arg0.v_type == VarType::VAR_BLOB {
            let b = arg0.vval.v_blob();
            let len = tv_blob_len(b);
            for i in 0..(len / 2) {
                let tmp = tv_blob_get( &*b, i);
                tv_blob_set(&mut *b, i, tv_blob_get( &*b, len - i - 1));
                tv_blob_set(&mut *b, len - i - 1, tmp);
            }
            tv_blob_set_ret(rettv, b);
        } else if arg0.v_type == VarType::VAR_STRING {
            rettv.v_type = VarType::VAR_STRING;
            if !arg0.vval.v_string().is_null() {
                crate::eval::typval::tv_set_string(
                    rettv,
                    reverse_text(arg0.vval.v_string()).as_ptr(),
                );
            } else {
                rettv.vval = crate::eval::typval::TypvalVval::String(std::ptr::null_mut());
            }
        } else if arg0.v_type == VarType::VAR_LIST {
            let l = arg0.vval.v_list();
            if !value_check_lock(
                tv_list_locked(l),
                c"reverse() argument",
                TV_TRANSLATE,
            ) {
                tv_list_reverse( &mut *l);
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
        assert_eq!(tv_list_locked(std::ptr::null()), VAR_FIXED);
            assert_eq!(tv_list_len(std::ptr::null()), 0);
            assert!(tv_list_first(std::ptr::null()).is_null());
            assert_eq!(tv_blob_len(std::ptr::null()), 0);
    }
}
