//! List support and container (`List`, `Dict`, `Blob`) functions (`src/nvim/eval/list.c`).
#![forbid(unsafe_code)]

use crate::buffer::VAR_FIXED;
use crate::errors::*;
use crate::eval::fs::K_LIST_LEN_UNKNOWN;
use crate::eval::typval::{
    EvalFuncData, TV_TRANSLATE, TvBlob, TvDict, TvList, TvValue, TypVal, VAR_LOCKED, VAR_UNLOCKED,
    VarType, tv_blob_copy, tv_blob_get, tv_blob_len, tv_blob_remove,
    tv_check_for_string_or_list_or_blob_arg, tv_clear, tv_copy, tv_dict_add_tv, tv_dict_alloc_ret,
    tv_dict_copy, tv_dict_extend, tv_dict_item_free, tv_dict_remove, tv_dict_unref, tv_equal,
    tv_get_bool_chk, tv_get_number_chk, tv_get_string, tv_get_string_chk, tv_list_alloc_ret,
    tv_list_append_owned_tv, tv_list_append_tv, tv_list_copy, tv_list_extend, tv_list_find,
    tv_list_first, tv_list_insert_tv, tv_list_item_remove, tv_list_len, tv_list_remove,
    tv_list_reverse, tv_list_set_lock, tv_list_set_ret, tv_list_unref, tv_set_bytes,
    value_check_lock,
};
use crate::eval::vars::{
    VV_KEY, VV_VAL, get_vim_var_tv, prepare_vimvar, restore_vimvar, set_vim_var_nr,
    set_vim_var_string, tv_list_locked, var_check_fixed, var_check_ro,
};
use crate::eval::{eval_expr_typval, get_copy_id, tv_blob_set_ret};
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::*;
use crate::hashtab::{hash_lock, hash_unlock};
use crate::mbyte::{mb_strnicmp, utf_ptr2len, utfc_slice2len};
use crate::message::{emsg, semsg};
use crate::strings::reverse_text;
use std::ffi::CStr;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilterMap {
    Filter,
    Map,
    MapNew,
    ForEach,
}

#[inline]
fn tv_blob_set(b: &TvBlob, idx: i32, c: u8) {
    b.bv_ga.borrow_mut()[idx as usize] = c;
}

fn filter_map_one(
    tv: &mut TypVal,
    expr: &mut TypVal,
    filtermap: FilterMap,
    newtv: &mut TypVal,
    remp: &mut bool,
) -> bool {
    let mut retval = false;
    tv_copy(&*tv, &mut get_vim_var_tv(VV_VAL).tv_mut());

    newtv.value = crate::eval::typval::TvValue::Unknown;
    if filtermap == FilterMap::ForEach && expr.var_type() == VarType::VarString {
        do_cmdline_cmd(expr.value.string_bytes());
        if msg_state().did_emsg.get() == 0 {
            retval = true;
        }
        get_vim_var_tv(VV_VAL).clear_tv();
        return retval;
    }

    let mut argv = [
        get_vim_var_tv(VV_KEY).tv().clone(),
        get_vim_var_tv(VV_VAL).tv().clone(),
        TypVal {
            v_lock: VAR_UNLOCKED,
            value: crate::eval::typval::TvValue::Unknown,
        },
    ];
    if !eval_expr_typval(&*expr, false, &mut argv, 2, &mut *newtv) {
        get_vim_var_tv(VV_VAL).clear_tv();
        return false;
    }
    if filtermap == FilterMap::Filter {
        let mut error = false;
        *remp = tv_get_number_chk(&*newtv, Some(&mut error)) == 0;
        tv_clear(newtv);
        if error {
            get_vim_var_tv(VV_VAL).clear_tv();
            return false;
        }
    } else if filtermap == FilterMap::ForEach {
        tv_clear(newtv);
    }
    retval = true;
    get_vim_var_tv(VV_VAL).clear_tv();
    retval
}

fn filter_map_dict(
    d: Option<&TvDict>,
    filtermap: FilterMap,
    arg_errmsg: &CStr,
    expr: &mut TypVal,
    rettv: &mut TypVal,
) {
    {
        if filtermap == FilterMap::MapNew {
            rettv.value = crate::eval::typval::TvValue::Dict(crate::eval::typval::DictId::NULL);
        }
        let Some(d) = d else {
            return;
        };
        if filtermap == FilterMap::Filter
            && value_check_lock(d.dv_lock.get(), Some(arg_errmsg), TV_TRANSLATE)
        {
            return;
        }

        let mut d_ret: Option<&TvDict> = None;
        if filtermap == FilterMap::MapNew {
            tv_dict_alloc_ret(rettv);
            d_ret = rettv.value.dict_opt();
        }

        let prev_lock = d.dv_lock.get();
        if prev_lock == VAR_UNLOCKED {
            d.dv_lock.set(VAR_LOCKED);
        }
        hash_lock(&d.dv_hashtab);

        let mut todo = d.dv_hashtab.ht_used.get();
        let len = d.dv_hashtab.array_len();
        for i in 0..len {
            if todo == 0 {
                break;
            }
            let di_id = d.dv_hashtab.slot(i).hi_di;
            if let Some(di) = di_id.opt() {
                todo -= 1;
                if filtermap == FilterMap::Map
                    && (value_check_lock(di.tv().v_lock, Some(arg_errmsg), TV_TRANSLATE)
                        || var_check_ro(
                            i32::from(di.di_flags.get()),
                            arg_errmsg.to_bytes(),
                            TV_TRANSLATE,
                        ))
                {
                    break;
                }
                set_vim_var_string(VV_KEY, Some(di.key_cstr()), -1);
                let mut newtv = TypVal {
                    v_lock: VAR_UNLOCKED,
                    value: crate::eval::typval::TvValue::Unknown,
                };
                let mut rem = false;
                let mut di_tv = di.tv().clone();
                let r = filter_map_one(&mut di_tv, expr, filtermap, &mut newtv, &mut rem);
                get_vim_var_tv(VV_KEY).clear_tv();
                if !r || msg_state().did_emsg.get() != 0 {
                    tv_clear(&mut newtv);
                    break;
                }
                if filtermap == FilterMap::Map {
                    di.clear_tv();
                    newtv.v_lock = VAR_UNLOCKED;
                    di.set_tv(newtv);
                } else if filtermap == FilterMap::MapNew {
                    let add_res =
                        tv_dict_add_tv(d_ret.unwrap(), di.key_cstr().to_bytes(), &mut newtv);
                    tv_clear(&mut newtv);
                    if !add_res {
                        break;
                    }
                } else if filtermap == FilterMap::Filter && rem {
                    if var_check_fixed(
                        i32::from(di.di_flags.get()),
                        arg_errmsg.to_bytes(),
                        TV_TRANSLATE,
                    ) || var_check_ro(
                        i32::from(di.di_flags.get()),
                        arg_errmsg.to_bytes(),
                        TV_TRANSLATE,
                    ) {
                        break;
                    }
                    d.dv_hashtab.remove_at(i);
                    tv_dict_item_free(di);
                }
            }
        }

        hash_unlock(&d.dv_hashtab);
        d.dv_lock.set(prev_lock);
    }
}

fn filter_map_blob(
    blob_arg: Option<std::rc::Rc<TvBlob>>,
    filtermap: FilterMap,
    expr: &mut TypVal,
    arg_errmsg: &CStr,
    rettv: &mut TypVal,
) {
    if filtermap == FilterMap::MapNew {
        rettv.value = crate::eval::typval::TvValue::Blob(crate::eval::typval::BlobId::NULL);
    }
    let Some(b) = blob_arg else {
        return;
    };
    if filtermap == FilterMap::Filter
        && value_check_lock(b.bv_lock.get(), Some(arg_errmsg), TV_TRANSLATE)
    {
        return;
    }

    let b_ret = if filtermap == FilterMap::MapNew {
        tv_blob_copy(Some(&b), &mut *rettv);
        rettv.value.blob().unwrap_rc()
    } else {
        b.clone()
    };

    set_vim_var_nr(VV_KEY, 0);

    let prev_lock = b.bv_lock.get();
    if prev_lock == 0 {
        b.bv_lock.set(VAR_LOCKED);
    }

    let mut i: i32 = 0;
    let mut idx: i64 = 0;
    while i < b.bv_ga.borrow().len() as i32 {
        let val = tv_blob_get(&b, i) as i64;
        let mut tv = TypVal {
            v_lock: VAR_UNLOCKED,
            value: TvValue::Number(val),
        };
        set_vim_var_nr(VV_KEY, idx);
        let mut newtv = TypVal {
            v_lock: VAR_UNLOCKED,
            value: crate::eval::typval::TvValue::Unknown,
        };
        let mut rem = false;
        if !filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem)
            || msg_state().did_emsg.get() != 0
        {
            break;
        }
        if filtermap != FilterMap::ForEach {
            let new_value = match &newtv.value {
                TvValue::Number(value) => *value,
                TvValue::Bool(value) => i64::from(*value),
                _ => {
                    tv_clear(&mut newtv);
                    emsg(E_INVALBLOB);
                    break;
                }
            };
            if filtermap != FilterMap::Filter {
                if new_value != val {
                    tv_blob_set(&b_ret, i, new_value as u8);
                }
            } else if rem {
                b.bv_ga.borrow_mut().remove(i as usize);
                i -= 1;
            }
        }
        i += 1;
        idx += 1;
    }

    b.bv_lock.set(prev_lock);
}

fn filter_map_string(
    str_bytes: &[u8],
    filtermap: FilterMap,
    expr: &mut TypVal,
    rettv: &mut TypVal,
) {
    rettv.value = crate::eval::typval::TvValue::NULL_STRING;

    set_vim_var_nr(VV_KEY, 0);

    let mut out: Vec<u8> = Vec::with_capacity(80);
    let mut idx: i64 = 0;
    let mut off = 0usize;
    while off < str_bytes.len() {
        let len = utfc_slice2len(&str_bytes[off..]);
        let mut tv = TypVal::new();
        tv_set_bytes(&mut tv, &str_bytes[off..off + len]);

        set_vim_var_nr(VV_KEY, idx);
        let mut newtv = TypVal {
            v_lock: VAR_UNLOCKED,
            value: crate::eval::typval::TvValue::Unknown,
        };
        let mut rem = false;
        if !filter_map_one(&mut tv, expr, filtermap, &mut newtv, &mut rem)
            || msg_state().did_emsg.get() != 0
        {
            tv_clear(&mut newtv);
            tv_clear(&mut tv);
            break;
        }
        if filtermap == FilterMap::Map || filtermap == FilterMap::MapNew {
            if newtv.var_type() != VarType::VarString {
                tv_clear(&mut newtv);
                tv_clear(&mut tv);
                emsg(E_STRING_REQUIRED);
                break;
            } else if !newtv.value.string_is_null() {
                out.extend_from_slice(newtv.value.string_bytes());
            }
        } else if (filtermap == FilterMap::ForEach || !rem) && !tv.value.string_is_null() {
            out.extend_from_slice(tv.value.string_bytes());
        }

        tv_clear(&mut newtv);
        tv_clear(&mut tv);

        idx += 1;
        off += len;
    }
    tv_set_bytes(&mut *rettv, &out);
}

fn filter_map_list(
    l: Option<&TvList>,
    filtermap: FilterMap,
    arg_errmsg: &CStr,
    expr: &mut TypVal,
    rettv: &mut TypVal,
) {
    {
        if filtermap == FilterMap::MapNew {
            rettv.value = crate::eval::typval::TvValue::List(crate::eval::typval::ListId::NULL);
        }
        if l.is_none()
            || (filtermap == FilterMap::Filter
                && value_check_lock(tv_list_locked(l), Some(arg_errmsg), TV_TRANSLATE))
        {
            return;
        }

        let mut l_ret = crate::eval::typval::ListId::NULL;
        if filtermap == FilterMap::MapNew {
            tv_list_alloc_ret(&mut *rettv, K_LIST_LEN_UNKNOWN);
            l_ret = rettv.value.list();
        }
        set_vim_var_nr(VV_KEY, 0);

        let prev_lock = tv_list_locked(l);
        if prev_lock == VAR_UNLOCKED {
            tv_list_set_lock(l, VAR_LOCKED);
        }

        let mut idx: i64 = 0;
        let mut li = tv_list_first(l);
        while let Some(cur_li) = li {
            if filtermap == FilterMap::Map
                && value_check_lock(cur_li.tv().v_lock, Some(arg_errmsg), TV_TRANSLATE)
            {
                break;
            }
            set_vim_var_nr(VV_KEY, idx);
            let mut newtv = TypVal {
                v_lock: VAR_UNLOCKED,
                value: crate::eval::typval::TvValue::Unknown,
            };
            let mut rem = false;
            let mut cur_tv = cur_li.tv().clone();
            if !filter_map_one(&mut cur_tv, expr, filtermap, &mut newtv, &mut rem) {
                break;
            }
            if msg_state().did_emsg.get() != 0 {
                tv_clear(&mut newtv);
                break;
            }
            if filtermap == FilterMap::Map {
                cur_li.clear_tv();
                newtv.v_lock = VAR_UNLOCKED;
                cur_li.set_tv(newtv);
            } else if filtermap == FilterMap::MapNew {
                tv_list_append_owned_tv(&l_ret, newtv);
            }
            if filtermap == FilterMap::Filter && rem {
                li = tv_list_item_remove(l.unwrap(), &cur_li);
            } else {
                li = cur_li.next_opt();
            }
            idx += 1;
        }

        tv_list_set_lock(l, prev_lock);
    }
}

fn filter_map(argvars: &mut [TypVal], rettv: &mut TypVal, filtermap: FilterMap) {
    let (func_name, arg_errmsg) = match filtermap {
        FilterMap::Map => (c"map()", c"map() argument"),
        FilterMap::MapNew => (c"mapnew()", c"mapnew() argument"),
        FilterMap::Filter => (c"filter()", c"filter() argument"),
        FilterMap::ForEach => (c"foreach()", c"foreach() argument"),
    };

    let (first, rest) = argvars.split_at_mut(1);
    let arg0 = &first[0];
    if filtermap != FilterMap::MapNew && arg0.var_type() != VarType::VarString {
        tv_copy(arg0, rettv);
    }

    if !matches!(
        arg0.var_type(),
        VarType::VarBlob | VarType::VarList | VarType::VarDict | VarType::VarString
    ) {
        semsg!(
            c"E1250: Argument of %s must be a List, String, Dictionary or Blob",
            func_name,
        );
        return;
    }

    let expr = &mut rest[0];
    if expr.var_type() == VarType::VarUnknown {
        return;
    }

    let mut save_val = TypVal {
        v_lock: VAR_UNLOCKED,
        value: crate::eval::typval::TvValue::Unknown,
    };
    let mut save_key = TypVal {
        v_lock: VAR_UNLOCKED,
        value: crate::eval::typval::TvValue::Unknown,
    };
    prepare_vimvar(VV_VAL, &mut save_val);
    prepare_vimvar(VV_KEY, &mut save_key);

    let save_did_emsg = msg_state().did_emsg.get();
    msg_state().did_emsg.set(0);

    match arg0.var_type() {
        VarType::VarDict => {
            filter_map_dict(arg0.value.dict_opt(), filtermap, arg_errmsg, expr, rettv);
        }
        VarType::VarBlob => {
            filter_map_blob(arg0.value.blob().rc(), filtermap, expr, arg_errmsg, rettv);
        }
        VarType::VarString => {
            filter_map_string(tv_get_string(arg0).to_bytes(), filtermap, expr, rettv);
        }
        VarType::VarList => {
            filter_map_list(arg0.value.list_opt(), filtermap, arg_errmsg, expr, rettv);
        }
        _ => unreachable!(),
    }

    restore_vimvar(VV_KEY, &mut save_key);
    restore_vimvar(VV_VAL, &mut save_val);

    msg_state()
        .did_emsg
        .set(msg_state().did_emsg.get() | (save_did_emsg));
}

/// `"filter()"` function (`f_filter`).
pub fn f_filter(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    filter_map(argvars, rettv, FilterMap::Filter);
}

/// `"map()"` function (`f_map`).
pub fn f_map(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    filter_map(argvars, rettv, FilterMap::Map);
}

/// `"mapnew()"` function (`f_mapnew`).
pub fn f_mapnew(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    filter_map(argvars, rettv, FilterMap::MapNew);
}

/// `"foreach()"` function (`f_foreach`).
pub fn f_foreach(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    filter_map(argvars, rettv, FilterMap::ForEach);
}

/// `"add(list, item)"` function (`f_add`).
pub fn f_add(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.value = crate::eval::typval::TvValue::Number(1);
    if argvars[0].var_type() == VarType::VarList {
        let l = argvars[0].value.list();
        if !value_check_lock(
            tv_list_locked(l.opt()),
            Some(c"add() argument"),
            TV_TRANSLATE,
        ) {
            tv_list_append_tv(&l, &argvars[1]);
            tv_copy(&argvars[0], rettv);
        }
    } else if argvars[0].var_type() == VarType::VarBlob {
        if let Some(b) = argvars[0].value.blob().rc()
            && !value_check_lock(b.bv_lock.get(), Some(c"add() argument"), TV_TRANSLATE)
        {
            let mut error = false;
            let n = tv_get_number_chk(&argvars[1], Some(&mut error));
            if !error {
                b.bv_ga.borrow_mut().push(n as u8);
                tv_copy(&argvars[0], rettv);
            }
        }
    } else {
        emsg(E_LISTBLOBREQ);
    }
}

fn count_string(haystack: Option<&[u8]>, needle: Option<&[u8]>, ic: bool) -> i64 {
    let (Some(hay_bytes), Some(needle_bytes)) = (haystack, needle) else {
        return 0;
    };
    if needle_bytes.is_empty() {
        return 0;
    }
    let needlelen = needle_bytes.len();
    let mut n: i64 = 0;

    if ic {
        let hay_cstr = crate::memory::bytes_to_cstring(hay_bytes);
        let needle_cstr = crate::memory::bytes_to_cstring(needle_bytes);
        let mut p = hay_cstr.as_c_str().to_bytes();
        while !p.is_empty() {
            if mb_strnicmp(p, needle_cstr.as_bytes(), needlelen) == 0 {
                n += 1;
                p = &p[needlelen..];
            } else {
                let step = utf_ptr2len(p).max(1) as usize;
                p = &p[step.min(p.len())..];
            }
        }
    } else {
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

fn count_list(l: Option<&TvList>, needle: &TypVal, idx: i64, ic: bool) -> i64 {
    if tv_list_len(l) == 0 {
        return 0;
    }
    let li = tv_list_find(l, idx as i32);
    if li.is_none() {
        semsg!(E_LIST_INDEX_OUT_OF_RANGE_NR, idx);
        return 0;
    }
    let mut n: i64 = 0;
    for cur in crate::eval::typval::list_iter_ptr(li) {
        if tv_equal(&cur.tv(), needle, ic) {
            n += 1;
        }
    }
    n
}

fn count_dict(d: Option<&TvDict>, needle: &TypVal, ic: bool) -> i64 {
    let Some(d) = d else {
        return 0;
    };
    let mut n: i64 = 0;
    for (_, di) in d.dv_hashtab.iter_items() {
        if tv_equal(&di.tv(), needle, ic) {
            n += 1;
        }
    }
    n
}

/// `"count()"` function (`f_count`).
pub fn f_count(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut n: i64 = 0;
    let mut ic = false;
    let mut error = false;

    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    let arg2 = &argvars[2];
    let arg3 = &argvars[3];

    if arg2.var_type() != VarType::VarUnknown {
        ic = tv_get_bool_chk(arg2, Some(&mut error)) != 0;
    }

    if !error && arg0.var_type() == VarType::VarString {
        let arg1_cow = tv_get_string_chk(arg1);
        n = count_string(
            arg0.value.string_opt_bytes(),
            arg1_cow.as_deref().map(CStr::to_bytes),
            ic,
        );
    } else if !error && arg0.var_type() == VarType::VarList {
        let mut idx: i64 = 0;
        if arg2.var_type() != VarType::VarUnknown && arg3.var_type() != VarType::VarUnknown {
            idx = tv_get_number_chk(arg3, Some(&mut error));
        }
        if !error {
            n = count_list(arg0.value.list_opt(), arg1, idx, ic);
        }
    } else if !error && arg0.var_type() == VarType::VarDict {
        if let Some(d) = arg0.value.dict_opt() {
            if arg2.var_type() != VarType::VarUnknown && arg3.var_type() != VarType::VarUnknown {
                emsg(E_INVARG);
            } else {
                n = count_dict(Some(d), arg1, ic);
            }
        }
    } else if !error {
        semsg!(
            c"E706: Argument of %s must be a List, String or Dictionary",
            c"count()",
        );
    }
    rettv.value = crate::eval::typval::TvValue::Number(n);
}

fn extend_dict(argvars: &mut [TypVal], arg_errmsg: &CStr, is_new: bool, rettv: &mut TypVal) {
    {
        let Some(d1_ref) = argvars[0].value.dict_opt() else {
            let locked = value_check_lock(VAR_FIXED, Some(arg_errmsg), TV_TRANSLATE);
            assert!(locked);
            return;
        };

        if !is_new && value_check_lock(d1_ref.dv_lock.get(), Some(arg_errmsg), TV_TRANSLATE) {
            return;
        }

        let mut d1 = crate::eval::typval::DictId::from_ref(d1_ref);
        if is_new {
            d1 = tv_dict_copy(None, Some(d1_ref), false, get_copy_id())
                .unwrap_or(crate::eval::typval::DictId::NULL);
            if d1.is_null() {
                return;
            }
        }

        if let Some(d2) = argvars[1].value.dict_opt() {
            let act_cow;
            let mut action = c"force";
            let arg2 = &argvars[2];
            if arg2.var_type() != VarType::VarUnknown {
                let Some(act) = tv_get_string_chk(arg2) else {
                    if is_new {
                        tv_dict_unref(d1.opt());
                    }
                    return;
                };
                act_cow = act;
                action = &act_cow;
                let action_bytes = action.to_bytes();
                if action_bytes != b"keep" && action_bytes != b"force" && action_bytes != b"error" {
                    if is_new {
                        tv_dict_unref(d1.opt());
                    }
                    semsg!(E_INVARG2, action);
                    return;
                }
            }
            tv_dict_extend(&d1, d2, action.to_bytes());
        }

        if is_new {
            *rettv = TypVal {
                v_lock: VAR_UNLOCKED,
                value: TvValue::Dict(d1),
            };
        } else {
            tv_copy(&argvars[0], &mut *rettv);
        }
    }
}

fn extend_list(argvars: &mut [TypVal], arg_errmsg: &CStr, is_new: bool, rettv: &mut TypVal) {
    {
        let mut error = false;
        let l1_opt = argvars[0].value.list_opt();

        if !is_new && value_check_lock(tv_list_locked(l1_opt), Some(arg_errmsg), TV_TRANSLATE) {
            return;
        }

        let mut l1 = argvars[0].value.list();
        if is_new {
            l1 = tv_list_copy(None, l1_opt, false, get_copy_id()).into();
            if l1.is_null() {
                return;
            }
        }
        let l1_ref = &*l1;

        if let Some(l2) = argvars[1].value.list_opt() {
            let mut item = crate::eval::typval::ListitemId::NULL;
            let arg2 = &argvars[2];
            if arg2.var_type() != VarType::VarUnknown {
                let before = tv_get_number_chk(arg2, Some(&mut error)) as i32;
                if error {
                    if is_new {
                        tv_list_unref(l1.opt());
                    }
                    return;
                }
                if before != tv_list_len(Some(l1_ref)) {
                    item =
                        crate::eval::typval::ListitemId::from(tv_list_find(Some(l1_ref), before));
                    if item.is_null() {
                        semsg!(E_LIST_INDEX_OUT_OF_RANGE_NR, before as i64);
                        if is_new {
                            tv_list_unref(l1.opt());
                        }
                        return;
                    }
                }
            }
            tv_list_extend(l1_ref, Some(l2), item.opt());
        }

        if is_new {
            *rettv = TypVal {
                v_lock: VAR_UNLOCKED,
                value: TvValue::List(l1),
            };
        } else {
            tv_copy(&argvars[0], &mut *rettv);
        }
    }
}

fn extend_impl(
    argvars: &mut [TypVal],
    rettv: &mut TypVal,
    arg_errmsg: &std::ffi::CStr,
    is_new: bool,
) {
    let t0 = argvars[0].var_type();
    let t1 = argvars[1].var_type();
    if t0 == VarType::VarList && t1 == VarType::VarList {
        extend_list(argvars, arg_errmsg, is_new, rettv);
    } else if t0 == VarType::VarDict && t1 == VarType::VarDict {
        extend_dict(argvars, arg_errmsg, is_new, rettv);
    } else {
        semsg!(
            E_LISTDICTARG,
            if is_new { c"extendnew()" } else { c"extend()" },
        );
    }
}

/// `"extend()"` function (`f_extend`).
pub fn f_extend(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    extend_impl(argvars, rettv, c"extend() argument", false);
}

/// `"extendnew()"` function (`f_extendnew`).
pub fn f_extendnew(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    extend_impl(argvars, rettv, c"extendnew() argument", true);
}

/// `"insert()"` function (`f_insert`).
pub fn f_insert(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut error = false;
    let [arg0, arg1, arg2, ..] = argvars else {
        unreachable!()
    };

    if arg0.var_type() == VarType::VarBlob {
        let Some(b) = arg0.value.blob().rc() else {
            return;
        };
        if value_check_lock(b.bv_lock.get(), Some(c"insert() argument"), TV_TRANSLATE) {
            return;
        }

        let mut before: i32 = 0;
        let len = tv_blob_len(Some(&b));

        if arg2.var_type() != VarType::VarUnknown {
            before = tv_get_number_chk(arg2, Some(&mut error)) as i32;
            if error {
                return;
            }
            if before < 0 || before > len {
                semsg!(E_INVARG2, tv_get_string(arg2));
                return;
            }
        }
        let val = tv_get_number_chk(arg1, Some(&mut error)) as i32;
        if error {
            return;
        }
        if !(0..=255).contains(&val) {
            semsg!(E_INVARG2, tv_get_string(arg1));
            return;
        }

        let _ = len;
        b.bv_ga.borrow_mut().insert(before as usize, val as u8);

        tv_copy(arg0, rettv);
    } else if arg0.var_type() != VarType::VarList {
        semsg!(E_LISTBLOBARG, c"insert()");
    } else {
        let mut l = arg0.value.list_opt();
        if value_check_lock(tv_list_locked(l), Some(c"insert() argument"), TV_TRANSLATE) {
            return;
        }

        let mut before: i64 = 0;
        if arg2.var_type() != VarType::VarUnknown {
            before = tv_get_number_chk(arg2, Some(&mut error));
        }
        if error {
            return;
        }

        let mut item = crate::eval::typval::ListitemId::NULL;
        if before != (tv_list_len(l) as i64) {
            item = crate::eval::typval::ListitemId::from(tv_list_find(l, before as i32));
            if item.is_null() {
                semsg!(E_LIST_INDEX_OUT_OF_RANGE_NR, before);
                l = None;
            }
        }
        if let Some(l) = l {
            tv_list_insert_tv(l, arg1, item.opt());
            tv_copy(arg0, rettv);
        }
    }
}

/// `"remove()"` function (`f_remove`).
pub fn f_remove(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let arg_errmsg = c"remove() argument";
    match argvars[0].var_type() {
        VarType::VarDict => tv_dict_remove(argvars, rettv, arg_errmsg),
        VarType::VarBlob => tv_blob_remove(argvars, rettv, arg_errmsg),
        VarType::VarList => tv_list_remove(argvars, rettv, arg_errmsg),
        _ => {
            semsg!(E_LISTDICTBLOBARG, c"remove()");
        }
    }
}

/// `"reverse({list})"` function (`f_reverse`).
pub fn f_reverse(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if !tv_check_for_string_or_list_or_blob_arg(argvars, 0) {
        return;
    }

    let arg0 = &argvars[0];
    if arg0.var_type() == VarType::VarBlob {
        if let Some(b) = arg0.value.blob().rc() {
            b.bv_ga.borrow_mut().reverse();
            tv_blob_set_ret(rettv, Some(b));
        } else {
            tv_blob_set_ret(rettv, None);
        }
    } else if arg0.var_type() == VarType::VarString {
        rettv.value = crate::eval::typval::TvValue::NULL_STRING;
        if !arg0.value.string_is_null() {
            let s_bytes = arg0.value.string_bytes();
            let rev = reverse_text(s_bytes);
            crate::eval::typval::tv_set_string(rettv, Some(rev.as_c_str()));
        } else {
            rettv.value = crate::eval::typval::TvValue::NULL_STRING;
        }
    } else if arg0.var_type() == VarType::VarList {
        let l = arg0.value.list_opt();
        if !value_check_lock(tv_list_locked(l), Some(c"reverse() argument"), TV_TRANSLATE) {
            tv_list_reverse(l.unwrap());
            tv_list_set_ret(rettv, l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lock_and_len_helpers_handle_null() {
        assert_eq!(tv_list_locked(None), VAR_FIXED);
        assert_eq!(tv_list_len(None), 0);
        assert!(tv_list_first(None).is_none());
        assert_eq!(tv_blob_len(None), 0);
    }
}
