//! Fuzzy matching algorithm and functions (`src/nvim/fuzzy.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::charset::{ascii_isalnum, ascii_iswhite};
#[allow(unused_imports)]
use crate::charset::{vim_iswordc, vim_iswordp};
#[allow(unused_imports)]
use crate::cursor::equalpos;
#[allow(unused_imports)]
use crate::eval::callback_call;
#[allow(unused_imports)]
use crate::eval::typval::tv_list_len;
#[allow(unused_imports)]
use crate::eval::typval::{
    callback_free, tv_clear, tv_dict_find, tv_dict_has_key, tv_dict_unref, tv_list_alloc,
    tv_list_alloc_ret, tv_list_append_list, tv_list_append_tv, tv_list_find,
};
#[allow(unused_imports)]
use crate::eval::typval::{
    tv_check_for_nonnull_dict_arg, tv_dict_get_callback, tv_dict_get_string, tv_get_string,
};
#[allow(unused_imports)]
use crate::eval::typval::{tv_get_number_chk, tv_list_append_number};
use crate::globals::*;
#[allow(unused_imports)]
use crate::insexpand::{ctrl_x_mode_whole_line, find_line_end, find_word_end, find_word_start};
#[allow(unused_imports)]
use crate::mbyte::{
    mb_charlen, mb_islower, mb_isupper, mb_tolower, mb_toupper, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
#[allow(unused_imports)]
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::semsg;
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::strings::qsort;
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::cursor_shape::pos_T;
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{Callback, EvalFuncData, buf_T};
use crate::garray::{ga_append_via_ptr, ga_clear, ga_init, garray_T};
use crate::math::{FAIL, OK};
use crate::memory::{xcalloc, xfree, xmalloc, xstrdup};

pub const FUZZY_MATCH_MAX_LEN: usize = 1024;
pub const FUZZY_SCORE_NONE: c_int = c_int::MIN;

const MATCH_MAX_LEN: usize = FUZZY_MATCH_MAX_LEN;
const SCORE_MAX: f64 = f64::INFINITY;
const SCORE_MIN: f64 = f64::NEG_INFINITY;
const SCORE_SCALE: f64 = 1000.0;

const SCORE_GAP_LEADING: f64 = -0.005;
const SCORE_GAP_TRAILING: f64 = -0.005;
const SCORE_GAP_INNER: f64 = -0.01;
const SCORE_MATCH_CONSECUTIVE: f64 = 1.0;
const SCORE_MATCH_SLASH: f64 = 0.9;
const SCORE_MATCH_WORD: f64 = 0.8;
const SCORE_MATCH_CAPITAL: f64 = 0.7;
const SCORE_MATCH_DOT: f64 = 0.6;

const VAR_UNLOCKED: c_int = 0;
const K_CALLBACK_NONE: c_int = 0;
const K_LIST_LEN_UNKNOWN: isize = -1;
const K_LIST_LEN_MAY_KNOW: isize = -3;
const FORWARD: c_int = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fuzmatch_str_T {
    pub idx: c_int,
    pub str_ptr: *mut c_char,
    pub score: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct fuzzyItem_T {
    idx: c_int,
    item: *mut listitem_T,
    score: c_int,
    lmatchpos: *mut list_T,
    pat: *mut c_char,
    itemstr: *mut c_char,
    itemstr_allocated: bool,
    startpos: c_int,
}

struct MatchStruct {
    needle_len: usize,
    haystack_len: usize,
    lower_needle: [c_int; MATCH_MAX_LEN],
    lower_haystack: [c_int; MATCH_MAX_LEN],
    match_bonus: [f64; MATCH_MAX_LEN],
}

#[inline]
unsafe fn skipwhite(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        while matches!(*p as u8, b' ' | b'\t') {
            p = p.add(1);
        }
        p
    }
}

#[cfg(not(test))]
unsafe fn has_match(needle: *const c_char, haystack: *const c_char) -> c_int {
    unsafe {
        if needle.is_null() || haystack.is_null() || *needle == 0 {
            return FAIL;
        }

        let mut n_ptr = needle;
        let mut h_ptr = haystack;

        while *n_ptr != 0 {
            let n_char = utf_ptr2char(n_ptr);
            let n_upper = mb_toupper(n_char);
            let mut found = false;

            while *h_ptr != 0 {
                let h_char = utf_ptr2char(h_ptr);
                if h_char == n_char || h_char == n_upper {
                    found = true;
                    h_ptr = h_ptr.add(utfc_ptr2len(h_ptr) as usize);
                    break;
                }
                h_ptr = h_ptr.add(utfc_ptr2len(h_ptr) as usize);
            }

            if !found {
                return FAIL;
            }

            n_ptr = n_ptr.add(utfc_ptr2len(n_ptr) as usize);
        }

        OK
    }
}

#[cfg(not(test))]
unsafe fn compute_bonus_codepoint(last_c: c_int, c: c_int) -> f64 {
    unsafe {
        if ascii_isalnum(c) || vim_iswordc(c) {
            if last_c == (b'/' as c_int) {
                return SCORE_MATCH_SLASH;
            }
            if last_c == (b'-' as c_int) || last_c == (b'_' as c_int) || last_c == (b' ' as c_int) {
                return SCORE_MATCH_WORD;
            }
            if last_c == (b'.' as c_int) {
                return SCORE_MATCH_DOT;
            }
            if mb_isupper(c) && mb_islower(last_c) {
                return SCORE_MATCH_CAPITAL;
            }
        }
        0.0
    }
}

#[cfg(not(test))]
unsafe fn setup_match_struct(
    match_st: &mut MatchStruct,
    needle: *const c_char,
    haystack: *const c_char,
) {
    unsafe {
        let mut i = 0usize;
        let mut p = needle;
        while *p != 0 && i < MATCH_MAX_LEN {
            let c = utf_ptr2char(p);
            match_st.lower_needle[i] = mb_tolower(c);
            i += 1;
            p = p.add(utf_ptr2len(p) as usize);
        }
        match_st.needle_len = i;

        i = 0;
        p = haystack;
        let mut prev_c = b'/' as c_int;
        while *p != 0 && i < MATCH_MAX_LEN {
            let c = utf_ptr2char(p);
            match_st.lower_haystack[i] = mb_tolower(c);
            match_st.match_bonus[i] = compute_bonus_codepoint(prev_c, c);
            prev_c = c;
            p = p.add(utf_ptr2len(p) as usize);
            i += 1;
        }
        match_st.haystack_len = i;
    }
}

#[inline]
unsafe fn match_row(
    match_st: &MatchStruct,
    row: usize,
    curr_d: *mut f64,
    curr_m: *mut f64,
    last_d: *const f64,
    last_m: *const f64,
) {
    let n = match_st.needle_len;
    let m = match_st.haystack_len;
    let i = row;

    let mut prev_score = SCORE_MIN;
    let gap_score = if i == n - 1 {
        SCORE_GAP_TRAILING
    } else {
        SCORE_GAP_INNER
    };

    let mut prev_m = SCORE_MIN;
    let mut prev_d = SCORE_MIN;

    for j in 0..m {
        unsafe {
            if match_st.lower_needle[i] == match_st.lower_haystack[j] {
                let mut score = SCORE_MIN;
                if i == 0 {
                    score = ((j as f64) * SCORE_GAP_LEADING) + match_st.match_bonus[j];
                } else if j > 0 {
                    score =
                        (prev_m + match_st.match_bonus[j]).max(prev_d + SCORE_MATCH_CONSECUTIVE);
                }
                prev_d = *last_d.add(j);
                prev_m = *last_m.add(j);
                *curr_d.add(j) = score;
                prev_score = score.max(prev_score + gap_score);
                *curr_m.add(j) = prev_score;
            } else {
                prev_d = *last_d.add(j);
                prev_m = *last_m.add(j);
                *curr_d.add(j) = SCORE_MIN;
                prev_score += gap_score;
                *curr_m.add(j) = prev_score;
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn match_positions(
    needle: *const c_char,
    haystack: *const c_char,
    positions: *mut u32,
) -> f64 {
    unsafe {
        if needle.is_null() || haystack.is_null() || *needle == 0 {
            return SCORE_MIN;
        }

        let mut match_st = MatchStruct {
            needle_len: 0,
            haystack_len: 0,
            lower_needle: [0; MATCH_MAX_LEN],
            lower_haystack: [0; MATCH_MAX_LEN],
            match_bonus: [0.0; MATCH_MAX_LEN],
        };
        setup_match_struct(&mut match_st, needle, haystack);

        let n = match_st.needle_len;
        let m = match_st.haystack_len;

        if m > MATCH_MAX_LEN || n > m {
            return SCORE_MIN;
        } else if n == m {
            let mut equal = true;
            for i in 0..n {
                if match_st.lower_needle[i] != match_st.lower_haystack[i] {
                    equal = false;
                    break;
                }
            }
            if equal {
                if !positions.is_null() {
                    for i in 0..n {
                        *positions.add(i) = i as u32;
                    }
                }
                return SCORE_MAX;
            }
        }

        if n > (usize::MAX / std::mem::size_of::<f64>()) / MATCH_MAX_LEN / 2 {
            return SCORE_MIN;
        }

        let block = xmalloc(std::mem::size_of::<f64>() * MATCH_MAX_LEN * n * 2).cast::<f64>();
        let d_base = block;
        let m_base = block.add(MATCH_MAX_LEN * n);

        match_row(&match_st, 0, d_base, m_base, d_base, m_base);
        for i in 1..n {
            let row_off = i * MATCH_MAX_LEN;
            let prev_off = (i - 1) * MATCH_MAX_LEN;
            match_row(
                &match_st,
                i,
                d_base.add(row_off),
                m_base.add(row_off),
                d_base.add(prev_off),
                m_base.add(prev_off),
            );
        }

        if !positions.is_null() {
            let mut match_required = false;
            let mut j = (m as isize) - 1;
            for i in (0..n).rev() {
                while j >= 0 {
                    let ju = j as usize;
                    let dij = *d_base.add(i * MATCH_MAX_LEN + ju);
                    let mij = *m_base.add(i * MATCH_MAX_LEN + ju);
                    if dij != SCORE_MIN && (match_required || dij == mij) {
                        match_required = i > 0
                            && ju > 0
                            && mij
                                == *d_base.add((i - 1) * MATCH_MAX_LEN + (ju - 1))
                                    + SCORE_MATCH_CONSECUTIVE;
                        *positions.add(i) = ju as u32;
                        j -= 1;
                        break;
                    }
                    j -= 1;
                }
            }
        }

        let result = *m_base.add((n - 1) * MATCH_MAX_LEN + (m - 1));
        xfree(block.cast::<c_void>());
        result
    }
}

/// Fuzzy match `pat_arg` in `str` (`fuzzy_match`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzzy_match(
    str_ptr: *mut c_char,
    pat_arg: *const c_char,
    matchseq: bool,
    out_score: *mut c_int,
    matches: *mut u32,
    max_matches: c_int,
) -> bool {
    #[cfg(test)]
    {
        let _ = (str_ptr, pat_arg, matchseq, out_score, matches, max_matches);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let mut complete = false;
        let mut num_matches: c_int = 0;

        *out_score = 0;

        let save_pat = xstrdup(pat_arg);
        let mut pat = save_pat;
        let mut p = pat;

        loop {
            if matchseq {
                complete = true;
            } else {
                p = skipwhite(p);
                if *p == 0 {
                    break;
                }
                pat = p;
                while *p != 0 && !ascii_iswhite(utf_ptr2char(p)) {
                    p = p.add(utf_ptr2len(p) as usize);
                }
                if *p == 0 {
                    complete = true;
                }
                *p = 0;
            }

            let mut pat_chars = mb_charlen(pat);
            if pat_chars > max_matches {
                pat_chars = max_matches;
            }
            if num_matches > max_matches - pat_chars {
                num_matches = 0;
                *out_score = FUZZY_SCORE_NONE;
                break;
            }

            let mut score = FUZZY_SCORE_NONE;
            if has_match(pat, str_ptr) != FAIL {
                let fzy_score = match_positions(pat, str_ptr, matches.add(num_matches as usize));
                if fzy_score != SCORE_MIN {
                    score = if fzy_score == SCORE_MAX {
                        c_int::MAX
                    } else if fzy_score < 0.0 {
                        (fzy_score * SCORE_SCALE - 0.5).ceil() as c_int
                    } else {
                        (fzy_score * SCORE_SCALE + 0.5).floor() as c_int
                    };
                }
            }

            if score == FUZZY_SCORE_NONE {
                num_matches = 0;
                *out_score = FUZZY_SCORE_NONE;
                break;
            }

            if score > 0 && *out_score > c_int::MAX - score {
                *out_score = c_int::MAX;
            } else if score < 0 && *out_score < c_int::MIN + 1 - score {
                *out_score = c_int::MIN + 1;
            } else {
                *out_score += score;
            }

            num_matches += pat_chars;

            if complete || num_matches >= max_matches {
                break;
            }

            p = p.add(1);
        }

        xfree(save_pat.cast::<c_void>());
        num_matches != 0
    }
}

unsafe extern "C" fn fuzzy_match_item_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let item1 = &*s1.cast::<fuzzyItem_T>();
        let item2 = &*s2.cast::<fuzzyItem_T>();
        let v1 = item1.score;
        let v2 = item2.score;

        if v1 == v2 {
            let pat_bytes = CStr::from_ptr(item1.pat).to_bytes();
            let patlen = pat_bytes.len();

            let exact_match1 = if item1.startpos >= 0 {
                let s1_ptr = item1.itemstr.add(item1.startpos as usize);
                let s1_len = CStr::from_ptr(s1_ptr).to_bytes().len();
                s1_len >= patlen
                    && std::slice::from_raw_parts(s1_ptr.cast::<u8>(), patlen) == pat_bytes
            } else {
                false
            };

            let exact_match2 = if item2.startpos >= 0 {
                let s2_ptr = item2.itemstr.add(item2.startpos as usize);
                let s2_len = CStr::from_ptr(s2_ptr).to_bytes().len();
                s2_len >= patlen
                    && std::slice::from_raw_parts(s2_ptr.cast::<u8>(), patlen) == pat_bytes
            } else {
                false
            };

            if exact_match1 == exact_match2 {
                item1.idx.cmp(&item2.idx) as c_int
            } else if exact_match2 {
                1
            } else {
                -1
            }
        } else if v1 > v2 {
            -1
        } else {
            1
        }
    }
}

#[cfg(not(test))]
unsafe fn fuzzy_match_in_list(
    l: *mut list_T,
    str_ptr: *mut c_char,
    matchseq: bool,
    key: *const c_char,
    item_cb: *mut Callback,
    retmatchpos: bool,
    fmatchlist: *mut list_T,
    max_matches: c_int,
) {
    unsafe {
        let mut len = tv_list_len(l);
        if len == 0 {
            return;
        }
        if max_matches > 0 && len > max_matches {
            len = max_matches;
        }

        let items = xcalloc(len as usize, std::mem::size_of::<fuzzyItem_T>()).cast::<fuzzyItem_T>();
        let mut match_count: c_int = 0;
        let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

        let mut li = if l.is_null() {
            std::ptr::null_mut()
        } else {
            (*l).lv_first
        };
        while !li.is_null() {
            if max_matches > 0 && match_count >= max_matches {
                break;
            }

            let mut itemstr: *mut c_char = std::ptr::null_mut();
            let mut itemstr_allocate = false;
            let mut rettv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: 0 },
            };

            let tv = &(*li).li_tv;
            if tv.v_type == VarType::VAR_STRING {
                itemstr = tv.vval.v_string;
            } else if tv.v_type == VarType::VAR_DICT
                && (!key.is_null() || (*item_cb).cb_type != K_CALLBACK_NONE)
            {
                let d = tv.vval.v_ptr.cast::<dict_T>();
                if !key.is_null() {
                    itemstr = tv_dict_get_string(d, key, false);
                } else {
                    (*d).dv_refcount += 1;
                    let mut argv = [
                        typval_T {
                            v_type: VarType::VAR_DICT,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval {
                                v_ptr: d.cast::<c_void>(),
                            },
                        },
                        typval_T {
                            v_type: VarType::VAR_UNKNOWN,
                            v_lock: VAR_UNLOCKED,
                            vval: TypvalVval { v_number: 0 },
                        },
                    ];
                    if callback_call(item_cb, 1, argv.as_mut_ptr(), &mut rettv)
                        && rettv.v_type == VarType::VAR_STRING
                    {
                        itemstr = rettv.vval.v_string;
                        itemstr_allocate = true;
                    }
                    tv_dict_unref(d);
                }
            }

            let mut score: c_int = 0;
            if !itemstr.is_null()
                && fuzzy_match(
                    itemstr,
                    str_ptr,
                    matchseq,
                    &mut score,
                    matches.as_mut_ptr(),
                    FUZZY_MATCH_MAX_LEN as c_int,
                )
            {
                let itemstr_copy = if itemstr_allocate {
                    xstrdup(itemstr)
                } else {
                    itemstr
                };
                let mut match_positions_list: *mut list_T = std::ptr::null_mut();

                if retmatchpos {
                    match_positions_list = tv_list_alloc(K_LIST_LEN_MAY_KNOW);
                    let mut j = 0usize;
                    let mut p: *const c_char = str_ptr;
                    while *p != 0 && j < FUZZY_MATCH_MAX_LEN {
                        if !ascii_iswhite(utf_ptr2char(p)) || matchseq {
                            tv_list_append_number(match_positions_list, matches[j] as i64);
                            j += 1;
                        }
                        p = p.add(utf_ptr2len(p) as usize);
                    }
                }

                let dst = &mut *items.add(match_count as usize);
                dst.idx = match_count;
                dst.item = li;
                dst.score = score;
                dst.pat = str_ptr;
                dst.startpos = matches[0] as c_int;
                dst.itemstr = itemstr_copy;
                dst.itemstr_allocated = itemstr_allocate;
                dst.lmatchpos = match_positions_list;

                match_count += 1;
            }
            tv_clear(&mut rettv);
            li = (*li).li_next;
        }

        if match_count > 0 {
            qsort(
                items.cast::<c_void>(),
                match_count as usize,
                std::mem::size_of::<fuzzyItem_T>(),
                fuzzy_match_item_compare,
            );

            let mut retlist = if retmatchpos {
                let first_li = tv_list_find(fmatchlist, 0);
                assert!(!first_li.is_null() && !(*first_li).li_tv.vval.v_list.is_null());
                (*first_li).li_tv.vval.v_list
            } else {
                fmatchlist
            };

            for i in 0..(match_count as usize) {
                let item_li = (*items.add(i)).item;
                tv_list_append_tv(retlist, &mut (*item_li).li_tv);
            }

            if retmatchpos {
                let pos_li = tv_list_find(fmatchlist, -2);
                assert!(!pos_li.is_null() && !(*pos_li).li_tv.vval.v_list.is_null());
                retlist = (*pos_li).li_tv.vval.v_list;

                for i in 0..(match_count as usize) {
                    let cur = &mut *items.add(i);
                    assert!(!cur.lmatchpos.is_null());
                    tv_list_append_list(retlist, cur.lmatchpos);
                    cur.lmatchpos = std::ptr::null_mut();
                }

                let score_li = tv_list_find(fmatchlist, -1);
                assert!(!score_li.is_null() && !(*score_li).li_tv.vval.v_list.is_null());
                retlist = (*score_li).li_tv.vval.v_list;
                for i in 0..(match_count as usize) {
                    tv_list_append_number(retlist, (*items.add(i)).score as i64);
                }
            }
        }

        for i in 0..(match_count as usize) {
            let cur = &*items.add(i);
            if cur.itemstr_allocated {
                xfree(cur.itemstr.cast::<c_void>());
            }
            assert!(cur.lmatchpos.is_null());
        }
        xfree(items.cast::<c_void>());
    }
}

#[cfg(not(test))]
unsafe fn do_fuzzymatch(argvars: *const typval_T, rettv: *mut typval_T, retmatchpos: bool) {
    unsafe {
        let arg0 = &*argvars;
        let arg1 = &*argvars.add(1);
        let arg2 = &*argvars.add(2);

        if arg0.v_type != VarType::VAR_LIST || arg0.vval.v_list.is_null() {
            semsg!(
                c"E686: Argument of %s must be a List".as_ptr(),
                if retmatchpos {
                    c"matchfuzzypos()".as_ptr()
                } else {
                    c"matchfuzzy()".as_ptr()
                },
            );
            return;
        }
        if arg1.v_type != VarType::VAR_STRING || arg1.vval.v_string.is_null() {
            semsg!(c"E475: Invalid argument: %s".as_ptr(), tv_get_string(arg1));
            return;
        }

        let mut cb = Callback {
            data: std::ptr::null_mut(),
            cb_type: K_CALLBACK_NONE,
        };
        let mut key: *const c_char = std::ptr::null();
        let mut matchseq = false;
        let mut max_matches: c_int = 0;

        if arg2.v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_nonnull_dict_arg(argvars, 2) == FAIL {
                return;
            }

            let d = arg2.vval.v_ptr.cast::<dict_T>();
            let di_key = tv_dict_find(d, c"key".as_ptr(), -1);
            if !di_key.is_null() {
                if (*di_key).di_tv.v_type != VarType::VAR_STRING
                    || (*di_key).di_tv.vval.v_string.is_null()
                    || *(*di_key).di_tv.vval.v_string == 0
                {
                    semsg!(
                        c"E475: Invalid value for argument %s: %s".as_ptr(),
                        c"key".as_ptr(),
                        tv_get_string(&(*di_key).di_tv),
                    );
                    return;
                }
                key = tv_get_string(&(*di_key).di_tv);
            } else if !tv_dict_get_callback(d, c"text_cb".as_ptr(), -1, &mut cb) {
                semsg!(
                    c"E475: Invalid value for argument %s".as_ptr(),
                    c"text_cb".as_ptr(),
                );
                return;
            }

            let di_limit = tv_dict_find(d, c"limit".as_ptr(), -1);
            if !di_limit.is_null() {
                if (*di_limit).di_tv.v_type != VarType::VAR_NUMBER {
                    callback_free(&mut cb);
                    semsg!(
                        c"E475: Invalid value for argument %s".as_ptr(),
                        c"limit".as_ptr(),
                    );
                    return;
                }
                max_matches = tv_get_number_chk(&(*di_limit).di_tv, std::ptr::null_mut()) as c_int;
            }

            if tv_dict_has_key(d, c"matchseq".as_ptr()) {
                matchseq = true;
            }
        }

        tv_list_alloc_ret(rettv, if retmatchpos { 3 } else { K_LIST_LEN_UNKNOWN });
        let out_list = (*rettv).vval.v_list;
        if retmatchpos {
            tv_list_append_list(out_list, tv_list_alloc(K_LIST_LEN_UNKNOWN));
            tv_list_append_list(out_list, tv_list_alloc(K_LIST_LEN_UNKNOWN));
            tv_list_append_list(out_list, tv_list_alloc(K_LIST_LEN_UNKNOWN));
        }

        fuzzy_match_in_list(
            arg0.vval.v_list,
            tv_get_string(arg1).cast_mut(),
            matchseq,
            key,
            &mut cb,
            retmatchpos,
            out_list,
            max_matches,
        );

        callback_free(&mut cb);
    }
}

/// `"matchfuzzy()"` function (`f_matchfuzzy`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_matchfuzzy(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        do_fuzzymatch(argvars, rettv, false);
    }
}

/// `"matchfuzzypos()"` function (`f_matchfuzzypos`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_matchfuzzypos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        do_fuzzymatch(argvars, rettv, true);
    }
}

unsafe extern "C" fn fuzzy_match_str_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    let f1 = unsafe { &*s1.cast::<fuzmatch_str_T>() };
    let f2 = unsafe { &*s2.cast::<fuzmatch_str_T>() };

    if f1.score == f2.score {
        f1.idx.cmp(&f2.idx) as c_int
    } else if f1.score > f2.score {
        -1
    } else {
        1
    }
}

unsafe extern "C" fn fuzzy_match_func_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    let f1 = unsafe { &*s1.cast::<fuzmatch_str_T>() };
    let f2 = unsafe { &*s2.cast::<fuzmatch_str_T>() };
    let b1 = unsafe { *f1.str_ptr as u8 };
    let b2 = unsafe { *f2.str_ptr as u8 };

    if b1 != b'<' && b2 == b'<' {
        return -1;
    }
    if b1 == b'<' && b2 != b'<' {
        return 1;
    }
    if f1.score == f2.score {
        return f1.idx.cmp(&f2.idx) as c_int;
    }
    if f1.score > f2.score { -1 } else { 1 }
}

/// Fuzzy match `pat` in `str` (`fuzzy_match_str`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzzy_match_str(str_ptr: *mut c_char, pat: *const c_char) -> c_int {
    if str_ptr.is_null() || pat.is_null() {
        return 0;
    }

    let mut score = FUZZY_SCORE_NONE;
    let mut matchpos = [0u32; FUZZY_MATCH_MAX_LEN];
    unsafe {
        fuzzy_match(
            str_ptr,
            pat,
            true,
            &mut score,
            matchpos.as_mut_ptr(),
            FUZZY_MATCH_MAX_LEN as c_int,
        );
    }
    score
}

/// Fuzzy match positions of `pat` in `str` (`fuzzy_match_str_with_pos`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzzy_match_str_with_pos(
    str_ptr: *mut c_char,
    pat: *const c_char,
) -> *mut garray_T {
    #[cfg(test)]
    {
        let _ = (str_ptr, pat);
        std::ptr::null_mut()
    }
    #[cfg(not(test))]
    unsafe {
        if str_ptr.is_null() || pat.is_null() {
            return std::ptr::null_mut();
        }

        let match_positions = xmalloc(std::mem::size_of::<garray_T>()).cast::<garray_T>();
        ga_init(match_positions, std::mem::size_of::<u32>() as c_int, 10);

        let mut score = FUZZY_SCORE_NONE;
        let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];
        if !fuzzy_match(
            str_ptr,
            pat,
            false,
            &mut score,
            matches.as_mut_ptr(),
            FUZZY_MATCH_MAX_LEN as c_int,
        ) || score == FUZZY_SCORE_NONE
        {
            ga_clear(match_positions);
            xfree(match_positions.cast::<c_void>());
            return std::ptr::null_mut();
        }

        let mut j = 0usize;
        let mut p = pat;
        while *p != 0 {
            if !ascii_iswhite(utf_ptr2char(p)) {
                let slot =
                    ga_append_via_ptr(match_positions, std::mem::size_of::<u32>()).cast::<u32>();
                *slot = matches[j];
                j += 1;
            }
            p = p.add(utf_ptr2len(p) as usize);
        }

        match_positions
    }
}

/// Fuzzy match a word in line `*ptr` (`fuzzy_match_str_in_line`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzzy_match_str_in_line(
    ptr: *mut *mut c_char,
    pat: *mut c_char,
    len: *mut c_int,
    current_pos: *mut pos_T,
    score: *mut c_int,
) -> bool {
    #[cfg(test)]
    {
        let _ = (ptr, pat, len, current_pos, score);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let mut str_cur = *ptr;
        let str_begin = str_cur;
        let mut found = false;

        if str_cur.is_null() || pat.is_null() {
            return false;
        }
        let line_end = find_line_end(str_cur);

        while (str_cur as usize) < (line_end as usize) {
            let start = find_word_start(str_cur);
            if *start == 0 {
                break;
            }
            let end = find_word_end(start);

            let save_end = *end;
            *end = 0;
            *score = fuzzy_match_str(start, pat);
            *end = save_end;

            if *score != FUZZY_SCORE_NONE {
                *len = ((end as usize) - (start as usize)) as c_int;
                found = true;
                *ptr = start;
                if !current_pos.is_null() {
                    (*current_pos).col += ((end as usize) - (str_begin as usize)) as c_int;
                }
                break;
            }

            str_cur = end;
            while *str_cur != 0 && !vim_iswordp(str_cur) {
                str_cur = str_cur.add(utf_ptr2len(str_cur) as usize);
            }
        }

        if !found {
            *ptr = line_end;
        }

        found
    }
}

/// Search for the next fuzzy match in `buf` (`search_for_fuzzy_match`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn search_for_fuzzy_match(
    buf: *mut buf_T,
    pos: *mut pos_T,
    pattern: *mut c_char,
    dir: c_int,
    start_pos: *mut pos_T,
    len: *mut c_int,
    ptr: *mut *mut c_char,
    score: *mut c_int,
) -> bool {
    #[cfg(test)]
    {
        let _ = (buf, pos, pattern, dir, start_pos, len, ptr, score);
        false
    }
    #[cfg(not(test))]
    unsafe {
        let mut current_pos = *pos;
        let mut found_new_match = false;
        let mut looped_around = false;

        let whole_line = ctrl_x_mode_whole_line();
        let circly_end = if buf == curbuf {
            *start_pos
        } else {
            pos_T {
                lnum: (*buf).b_ml.ml_line_count,
                col: 0,
                coladd: 0,
            }
        };

        if whole_line && (*start_pos).lnum != (*pos).lnum {
            current_pos.lnum += dir;
        }

        loop {
            if looped_around
                && if whole_line {
                    current_pos.lnum == circly_end.lnum
                } else {
                    equalpos(current_pos, circly_end)
                }
            {
                break;
            }

            if current_pos.lnum >= 1 && current_pos.lnum <= (*buf).b_ml.ml_line_count {
                *ptr = ml_get_buf(buf, current_pos.lnum);
                if !whole_line {
                    *ptr = (*ptr).add(current_pos.col as usize);
                }

                if !(*ptr).is_null() && **ptr != 0 {
                    if !whole_line {
                        found_new_match =
                            fuzzy_match_str_in_line(ptr, pattern, len, &mut current_pos, score);
                        if found_new_match {
                            *pos = current_pos;
                            break;
                        } else if looped_around && current_pos.lnum == circly_end.lnum {
                            break;
                        }
                    } else {
                        let line = *ptr;
                        let p = skipwhite(line);
                        if fuzzy_match_str(p, pattern) != FUZZY_SCORE_NONE {
                            found_new_match = true;
                            *pos = current_pos;
                            *ptr = p;
                            *len = ml_get_buf_len(buf, current_pos.lnum)
                                - (((p as usize) - (line as usize)) as c_int);
                            break;
                        }
                    }
                }
            }

            if dir == FORWARD {
                current_pos.lnum += 1;
                if current_pos.lnum > (*buf).b_ml.ml_line_count {
                    if p_ws != 0 {
                        current_pos.lnum = 1;
                        looped_around = true;
                    } else {
                        break;
                    }
                }
            } else {
                current_pos.lnum -= 1;
                if current_pos.lnum < 1 {
                    if p_ws != 0 {
                        current_pos.lnum = (*buf).b_ml.ml_line_count;
                        looped_around = true;
                    } else {
                        break;
                    }
                }
            }
            current_pos.col = 0;
        }

        found_new_match
    }
}

/// Free an array of `fuzmatch_str_T` (`fuzmatch_str_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzmatch_str_free(fuzmatch: *mut fuzmatch_str_T, count: c_int) {
    if fuzmatch.is_null() {
        return;
    }
    unsafe {
        for i in 0..(count.max(0) as usize) {
            xfree((*fuzmatch.add(i)).str_ptr.cast::<c_void>());
        }
        xfree(fuzmatch.cast::<c_void>());
    }
}

/// Sort `fuzmatch` by score and copy strings into `*matches`, freeing `fuzmatch`
/// (`fuzzymatches_to_strmatches`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fuzzymatches_to_strmatches(
    fuzmatch: *mut fuzmatch_str_T,
    matches: *mut *mut *mut c_char,
    count: c_int,
    funcsort: bool,
) {
    unsafe {
        if count > 0 {
            let cnt = count as usize;
            *matches = xmalloc(cnt * std::mem::size_of::<*mut c_char>()).cast::<*mut c_char>();

            #[cfg(not(test))]
            {
                let cmp: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int = if funcsort {
                    fuzzy_match_func_compare
                } else {
                    fuzzy_match_str_compare
                };
                qsort(
                    fuzmatch.cast::<c_void>(),
                    cnt,
                    std::mem::size_of::<fuzmatch_str_T>(),
                    cmp,
                );
            }
            #[cfg(test)]
            {
                let slice = std::slice::from_raw_parts_mut(fuzmatch, cnt);
                slice.sort_by(|a, b| {
                    let c = if funcsort {
                        fuzzy_match_func_compare(
                            (a as *const fuzmatch_str_T).cast::<c_void>(),
                            (b as *const fuzmatch_str_T).cast::<c_void>(),
                        )
                    } else {
                        fuzzy_match_str_compare(
                            (a as *const fuzmatch_str_T).cast::<c_void>(),
                            (b as *const fuzmatch_str_T).cast::<c_void>(),
                        )
                    };
                    c.cmp(&0)
                });
            }

            for i in 0..cnt {
                *(*matches).add(i) = (*fuzmatch.add(i)).str_ptr;
            }
        }
        xfree(fuzmatch.cast::<c_void>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzymatches_to_strmatches_sorts_snr_functions_last() {
        unsafe {
            let fm = xcalloc(2, std::mem::size_of::<fuzmatch_str_T>()).cast::<fuzmatch_str_T>();
            *fm = fuzmatch_str_T {
                idx: 0,
                str_ptr: xstrdup(c"<SNR>1_foo".as_ptr()),
                score: 100,
            };
            *fm.add(1) = fuzmatch_str_T {
                idx: 1,
                str_ptr: xstrdup(c"Bar".as_ptr()),
                score: 50,
            };
            let mut matches: *mut *mut c_char = std::ptr::null_mut();
            fuzzymatches_to_strmatches(fm, &mut matches, 2, true);
            assert_eq!(CStr::from_ptr(*matches).to_bytes(), b"Bar");
            assert_eq!(CStr::from_ptr(*matches.add(1)).to_bytes(), b"<SNR>1_foo");
            xfree((*matches).cast::<c_void>());
            xfree((*matches.add(1)).cast::<c_void>());
            xfree(matches.cast::<c_void>());
        }
    }
}
