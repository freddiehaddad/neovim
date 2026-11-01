//! Fuzzy matching algorithm and functions (`src/nvim/fuzzy.c`).

use crate::buffer::FileBuf;
use crate::charset::{ascii_isalnum, ascii_iswhite, skipwhite, vim_iswordc, vim_iswordp};
use crate::cursor::{CursorPos, equalpos};
use crate::errors::{E_INVARG_NVAL, E_INVARG2, E_INVARGVAL, E_LISTARG};
use crate::eval::callback_call;
use crate::eval::fs::K_LIST_LEN_UNKNOWN;
use crate::eval::typval::{
    Callback, EvalFuncData, Listitem, TvList, TypVal, TypvalVval, VAR_UNLOCKED, VarType,
    callback_free, tv_check_for_nonnull_dict_arg, tv_clear, tv_dict_find_opt, tv_dict_get_callback,
    tv_dict_get_string, tv_dict_has_key, tv_dict_unref, tv_get_number_chk, tv_get_string,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_list, tv_list_append_number,
    tv_list_append_tv, tv_list_len,
};
use crate::globals::*;
use crate::insexpand::{ctrl_x_mode_whole_line, find_line_end, find_word_end, find_word_start};
use crate::mbyte::{
    mb_islower, mb_isupper, mb_tolower, mb_toupper, utf_slice2char, utf_slice2len, utfc_slice2len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::semsg;
use crate::option_vars::*;
use crate::search::FORWARD;
use std::ffi::{CStr, CString};

pub const FUZZY_MATCH_MAX_LEN: usize = 1024;
pub const FUZZY_SCORE_NONE: i32 = i32::MIN;

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

const K_CALLBACK_NONE: i32 = 0;

#[repr(C)]
#[derive(Clone)]
pub struct FuzmatchStr {
    pub idx: i32,
    pub str_cstr: CString,
    pub score: i32,
}

struct FuzzyItem {
    idx: i32,
    item: &'static mut Listitem,
    score: i32,
    matchpos: Vec<i64>,
    itemstr: CString,
    startpos: i32,
}

struct MatchStruct {
    needle_len: usize,
    haystack_len: usize,
    lower_needle: [i32; MATCH_MAX_LEN],
    lower_haystack: [i32; MATCH_MAX_LEN],
    match_bonus: [f64; MATCH_MAX_LEN],
}

fn has_match(needle: &[u8], haystack: &[u8]) -> bool {
    if needle.is_empty() || haystack.is_empty() {
        return false;
    }

    let mut n_idx = 0usize;
    let mut h_idx = 0usize;

    while n_idx < needle.len() && needle[n_idx] != 0 {
        let n_rest = &needle[n_idx..];
        let n_char = utf_slice2char(n_rest);
        let n_upper = mb_toupper(n_char);
        let mut found = false;

        while h_idx < haystack.len() && haystack[h_idx] != 0 {
            let h_rest = &haystack[h_idx..];
            let h_char = utf_slice2char(h_rest);
            h_idx += utfc_slice2len(h_rest);
            if h_char == n_char || h_char == n_upper {
                found = true;
                break;
            }
        }

        if !found {
            return false;
        }

        n_idx += utfc_slice2len(n_rest);
    }

    true
}

fn compute_bonus_codepoint(last_c: i32, c: i32) -> f64 {
    if ascii_isalnum(c) || vim_iswordc(c) {
        if last_c == (b'/' as i32) {
            return SCORE_MATCH_SLASH;
        }
        if last_c == (b'-' as i32) || last_c == (b'_' as i32) || last_c == (b' ' as i32) {
            return SCORE_MATCH_WORD;
        }
        if last_c == (b'.' as i32) {
            return SCORE_MATCH_DOT;
        }
        if mb_isupper(c) && mb_islower(last_c) {
            return SCORE_MATCH_CAPITAL;
        }
    }
    0.0
}

fn setup_match_struct(match_st: &mut MatchStruct, needle: &[u8], haystack: &[u8]) {
    let mut i = 0usize;
    let mut off = 0usize;
    while off < needle.len() && needle[off] != 0 && i < MATCH_MAX_LEN {
        let rest = &needle[off..];
        let c = utf_slice2char(rest);
        match_st.lower_needle[i] = mb_tolower(c);
        i += 1;
        off += utf_slice2len(rest);
    }
    match_st.needle_len = i;

    i = 0;
    off = 0;
    let mut prev_c = b'/' as i32;
    while off < haystack.len() && haystack[off] != 0 && i < MATCH_MAX_LEN {
        let rest = &haystack[off..];
        let c = utf_slice2char(rest);
        match_st.lower_haystack[i] = mb_tolower(c);
        match_st.match_bonus[i] = compute_bonus_codepoint(prev_c, c);
        prev_c = c;
        off += utf_slice2len(rest);
        i += 1;
    }
    match_st.haystack_len = i;
}

#[inline]
fn match_row(
    match_st: &MatchStruct,
    row: usize,
    curr_d: &mut [f64],
    curr_m: &mut [f64],
    last_d: &[f64],
    last_m: &[f64],
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
        if match_st.lower_needle[i] == match_st.lower_haystack[j] {
            let mut score = SCORE_MIN;
            if i == 0 {
                score = ((j as f64) * SCORE_GAP_LEADING) + match_st.match_bonus[j];
            } else if j > 0 {
                score = (prev_m + match_st.match_bonus[j]).max(prev_d + SCORE_MATCH_CONSECUTIVE);
            }
            prev_d = last_d[j];
            prev_m = last_m[j];
            curr_d[j] = score;
            prev_score = score.max(prev_score + gap_score);
            curr_m[j] = prev_score;
        } else {
            prev_d = last_d[j];
            prev_m = last_m[j];
            curr_d[j] = SCORE_MIN;
            prev_score += gap_score;
            curr_m[j] = prev_score;
        }
    }
}

fn match_positions(needle: &[u8], haystack: &[u8], positions: &mut [u32]) -> f64 {
    if needle.is_empty() || haystack.is_empty() {
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
    } else if n == m && match_st.lower_needle[..n] == match_st.lower_haystack[..n] {
        for (i, slot) in positions.iter_mut().take(n).enumerate() {
            *slot = i as u32;
        }
        return SCORE_MAX;
    }

    if n > (usize::MAX / std::mem::size_of::<f64>()) / MATCH_MAX_LEN / 2 {
        return SCORE_MIN;
    }

    let mut d_table = vec![0.0f64; MATCH_MAX_LEN * n];
    let mut m_table = vec![0.0f64; MATCH_MAX_LEN * n];
    let zero_row = [0.0f64; MATCH_MAX_LEN];

    match_row(
        &match_st,
        0,
        &mut d_table[..MATCH_MAX_LEN],
        &mut m_table[..MATCH_MAX_LEN],
        &zero_row,
        &zero_row,
    );
    for i in 1..n {
        let row_off = i * MATCH_MAX_LEN;
        let (d_prev, d_curr) = d_table.split_at_mut(row_off);
        let (m_prev, m_curr) = m_table.split_at_mut(row_off);
        let prev_off = (i - 1) * MATCH_MAX_LEN;
        match_row(
            &match_st,
            i,
            &mut d_curr[..MATCH_MAX_LEN],
            &mut m_curr[..MATCH_MAX_LEN],
            &d_prev[prev_off..prev_off + MATCH_MAX_LEN],
            &m_prev[prev_off..prev_off + MATCH_MAX_LEN],
        );
    }

    let mut match_required = false;
    let mut j = (m as isize) - 1;
    for i in (0..n).rev() {
        while j >= 0 {
            let ju = j as usize;
            let dij = d_table[i * MATCH_MAX_LEN + ju];
            let mij = m_table[i * MATCH_MAX_LEN + ju];
            if dij != SCORE_MIN && (match_required || dij == mij) {
                match_required = i > 0
                    && ju > 0
                    && mij == d_table[(i - 1) * MATCH_MAX_LEN + (ju - 1)] + SCORE_MATCH_CONSECUTIVE;
                if let Some(slot) = positions.get_mut(i) {
                    *slot = ju as u32;
                }
                j -= 1;
                break;
            }
            j -= 1;
        }
    }

    m_table[(n - 1) * MATCH_MAX_LEN + (m - 1)]
}

fn mb_charlen_slice(bytes: &[u8]) -> usize {
    let mut count = 0usize;
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != 0 {
        i += utfc_slice2len(&bytes[i..]);
        count += 1;
    }
    count
}

pub fn fuzzy_match_slice(
    str_bytes: &[u8],
    pat_bytes: &[u8],
    matchseq: bool,
    out_score: &mut i32,
    matches: &mut [u32],
) -> bool {
    let max_matches = matches.len() as i32;
    let mut complete = false;
    let mut num_matches: i32 = 0;

    *out_score = 0;

    let mut p_idx = 0usize;

    loop {
        let pat_slice: &[u8] = if matchseq {
            complete = true;
            pat_bytes
        } else {
            while p_idx < pat_bytes.len() && ascii_iswhite(i32::from(pat_bytes[p_idx])) {
                p_idx += 1;
            }
            if p_idx >= pat_bytes.len() || pat_bytes[p_idx] == 0 {
                break;
            }
            let start = p_idx;
            while p_idx < pat_bytes.len()
                && pat_bytes[p_idx] != 0
                && !ascii_iswhite(utf_slice2char(&pat_bytes[p_idx..]))
            {
                p_idx += utf_slice2len(&pat_bytes[p_idx..]);
            }
            if p_idx >= pat_bytes.len() || pat_bytes[p_idx] == 0 {
                complete = true;
            }
            &pat_bytes[start..p_idx]
        };

        let mut pat_chars = mb_charlen_slice(pat_slice) as i32;
        if pat_chars > max_matches {
            pat_chars = max_matches;
        }
        if num_matches > max_matches - pat_chars {
            num_matches = 0;
            *out_score = FUZZY_SCORE_NONE;
            break;
        }

        let mut score = FUZZY_SCORE_NONE;
        if has_match(pat_slice, str_bytes) {
            let fzy_score =
                match_positions(pat_slice, str_bytes, &mut matches[num_matches as usize..]);
            if fzy_score != SCORE_MIN {
                score = if fzy_score == SCORE_MAX {
                    i32::MAX
                } else if fzy_score < 0.0 {
                    (fzy_score * SCORE_SCALE - 0.5).ceil() as i32
                } else {
                    (fzy_score * SCORE_SCALE + 0.5).floor() as i32
                };
            }
        }

        if score == FUZZY_SCORE_NONE {
            num_matches = 0;
            *out_score = FUZZY_SCORE_NONE;
            break;
        }

        if score > 0 && *out_score > i32::MAX - score {
            *out_score = i32::MAX;
        } else if score < 0 && *out_score < i32::MIN + 1 - score {
            *out_score = i32::MIN + 1;
        } else {
            *out_score += score;
        }

        num_matches += pat_chars;

        if complete || num_matches >= max_matches {
            break;
        }

        p_idx += 1;
    }

    num_matches != 0
}

/// Fuzzy match `pat_arg` in `str` (`fuzzy_match`).
pub fn fuzzy_match(
    str_ptr: impl crate::strings::IntoCCharPtr,
    pat_arg: impl crate::strings::IntoCCharPtr,
    matchseq: bool,
    out_score: &mut i32,
    matches: &mut [u32],
) -> bool {
    let str_bytes = crate::memory::cstr_as_bytes(str_ptr.into_c_char_ptr());
    let pat_bytes = crate::memory::cstr_as_bytes(pat_arg.into_c_char_ptr());
    fuzzy_match_slice(str_bytes, pat_bytes, matchseq, out_score, matches)
}

fn fuzzy_match_item_compare(
    item1: &FuzzyItem,
    item2: &FuzzyItem,
    pat_bytes: &[u8],
) -> std::cmp::Ordering {
    let v1 = item1.score;
    let v2 = item2.score;

    if v1 == v2 {
        let exact_match1 = item1.startpos >= 0
            && item1
                .itemstr
                .to_bytes()
                .get(item1.startpos as usize..)
                .is_some_and(|s| s.starts_with(pat_bytes));

        let exact_match2 = item2.startpos >= 0
            && item2
                .itemstr
                .to_bytes()
                .get(item2.startpos as usize..)
                .is_some_and(|s| s.starts_with(pat_bytes));

        exact_match2
            .cmp(&exact_match1)
            .then_with(|| item1.idx.cmp(&item2.idx))
    } else {
        v2.cmp(&v1)
    }
}

fn fuzzy_match_in_list(
    l: Option<&TvList>,
    pat_bytes: &[u8],
    matchseq: bool,
    key: Option<&CStr>,
    item_cb: &mut Callback,
    retmatchpos: bool,
    fmatchlist: &mut TvList,
    max_matches: i32,
) {
    let mut len = tv_list_len(l);
    if len == 0 {
        if retmatchpos {
            tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
            tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
            tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
        }
        return;
    }
    if max_matches > 0 && len > max_matches {
        len = max_matches;
    }

    let mut items: Vec<FuzzyItem> = Vec::with_capacity(len as usize);
    let mut match_count: i32 = 0;
    let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];

    for li in crate::eval::typval::list_iter_ptr(l.map_or(std::ptr::null_mut(), |l| l.lv_first)) {
        if max_matches > 0 && match_count >= max_matches {
            break;
        }

        let mut itemstr: Option<&CStr> = None;
        let mut rettv = TypVal {
            v_type: VarType::VarUnknown,
            v_lock: VAR_UNLOCKED,
            vval: TypvalVval::Number(0),
        };

        let tv = &li.li_tv;
        if tv.v_type == VarType::VarString {
            itemstr = tv.vval.v_string_cstr();
        } else if tv.v_type == VarType::VarDict
            && (key.is_some() || item_cb.cb_type() != K_CALLBACK_NONE)
        {
            let d = tv.vval.v_dict_ref();
            if let Some(k) = key {
                itemstr = tv_dict_get_string(Some(&*d), k.as_ptr());
            } else {
                d.dv_refcount += 1;
                let d_ptr = &raw mut *d;
                let mut argv = [
                    TypVal {
                        v_type: VarType::VarDict,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Dict(d_ptr),
                    },
                    TypVal {
                        v_type: VarType::VarUnknown,
                        v_lock: VAR_UNLOCKED,
                        vval: TypvalVval::Number(0),
                    },
                ];
                if callback_call(item_cb, 1, argv.as_mut_ptr(), &mut rettv)
                    && rettv.v_type == VarType::VarString
                {
                    itemstr = rettv.vval.v_string_cstr();
                }
                tv_dict_unref(d_ptr);
            }
        }

        let mut score: i32 = 0;
        if let Some(item_cstr) = itemstr
            && fuzzy_match_slice(
                item_cstr.to_bytes(),
                pat_bytes,
                matchseq,
                &mut score,
                &mut matches,
            )
        {
            let itemstr_copy = item_cstr.to_owned();
            let mut matchpos: Vec<i64> = Vec::new();

            if retmatchpos {
                let mut j = 0usize;
                let mut off = 0usize;
                while off < pat_bytes.len() && j < FUZZY_MATCH_MAX_LEN {
                    let rest = &pat_bytes[off..];
                    if !ascii_iswhite(utf_slice2char(rest)) || matchseq {
                        matchpos.push(i64::from(matches[j]));
                        j += 1;
                    }
                    off += utf_slice2len(rest);
                }
            }

            items.push(FuzzyItem {
                idx: match_count,
                item: li,
                score,
                startpos: matches[0] as i32,
                itemstr: itemstr_copy,
                matchpos,
            });

            match_count += 1;
        }
        tv_clear(&mut rettv);
    }

    if !items.is_empty() {
        items.sort_unstable_by(|a, b| fuzzy_match_item_compare(a, b, pat_bytes));

        if retmatchpos {
            let match_list = tv_list_alloc(K_LIST_LEN_UNKNOWN);
            let pos_list = tv_list_alloc(K_LIST_LEN_UNKNOWN);
            let score_list = tv_list_alloc(K_LIST_LEN_UNKNOWN);

            for cur in &mut items {
                tv_list_append_tv(match_list, &mut cur.item.li_tv);

                let match_positions_list = tv_list_alloc(cur.matchpos.len() as isize);
                for &pos in &cur.matchpos {
                    tv_list_append_number(match_positions_list, pos);
                }
                tv_list_append_list(pos_list, match_positions_list);

                tv_list_append_number(score_list, i64::from(cur.score));
            }

            tv_list_append_list(fmatchlist, match_list);
            tv_list_append_list(fmatchlist, pos_list);
            tv_list_append_list(fmatchlist, score_list);
        } else {
            for cur in &mut items {
                tv_list_append_tv(fmatchlist, &mut cur.item.li_tv);
            }
        }
    } else if retmatchpos {
        tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
        tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
        tv_list_append_list(fmatchlist, tv_list_alloc(K_LIST_LEN_UNKNOWN));
    }
}

fn do_fuzzymatch(argvars: &[TypVal], rettv: &mut TypVal, retmatchpos: bool) {
    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    let arg2 = &argvars[2];

    if arg0.v_type != VarType::VarList || arg0.vval.v_list().is_null() {
        semsg!(
            E_LISTARG,
            if retmatchpos {
                c"matchfuzzypos()".as_ptr()
            } else {
                c"matchfuzzy()".as_ptr()
            },
        );
        return;
    }
    if arg1.v_type != VarType::VarString || arg1.vval.v_string().is_null() {
        semsg!(E_INVARG2, tv_get_string(arg1));
        return;
    }

    let mut cb = Callback::NONE;
    let mut key: Option<&CStr> = None;
    let mut matchseq = false;
    let mut max_matches: i32 = 0;

    if arg2.v_type != VarType::VarUnknown {
        if !tv_check_for_nonnull_dict_arg(argvars, 2) {
            return;
        }

        let d = arg2.vval.v_dict_ref();
        if let Some(di_key) = tv_dict_find_opt(Some(&*d), c"key", -1) {
            let key_cstr = di_key.di_tv.vval.v_string_cstr();
            if di_key.di_tv.v_type != VarType::VarString
                || key_cstr.is_none_or(|s| s.to_bytes().is_empty())
            {
                semsg!(E_INVARG_NVAL, c"key", tv_get_string(&di_key.di_tv));
                return;
            }
            key = key_cstr;
        } else if !tv_dict_get_callback(Some(&mut *d), c"text_cb", -1, &mut cb) {
            semsg!(E_INVARGVAL, c"text_cb");
            return;
        }

        if let Some(di_limit) = tv_dict_find_opt(Some(&*d), c"limit", -1) {
            if di_limit.di_tv.v_type != VarType::VarNumber {
                callback_free(&mut cb);
                semsg!(E_INVARGVAL, c"limit");
                return;
            }
            max_matches = tv_get_number_chk(&di_limit.di_tv, None) as i32;
        }

        if tv_dict_has_key(Some(&*d), c"matchseq") {
            matchseq = true;
        }
    }

    tv_list_alloc_ret(
        &mut *rettv,
        if retmatchpos { 3 } else { K_LIST_LEN_UNKNOWN },
    );
    let out_list = rettv.vval.v_list_ref();
    let in_list = arg0.vval.v_list_ref();
    let pat_bytes = tv_get_string(arg1).to_bytes();

    fuzzy_match_in_list(
        Some(&*in_list),
        pat_bytes,
        matchseq,
        key,
        &mut cb,
        retmatchpos,
        out_list,
        max_matches,
    );

    callback_free(&mut cb);
}

/// `"matchfuzzy()"` function (`f_matchfuzzy`).
pub fn f_matchfuzzy(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    do_fuzzymatch(argvars, rettv, false);
}

/// `"matchfuzzypos()"` function (`f_matchfuzzypos`).
pub fn f_matchfuzzypos(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    do_fuzzymatch(argvars, rettv, true);
}

fn fuzzy_match_str_compare(f1: &FuzmatchStr, f2: &FuzmatchStr) -> std::cmp::Ordering {
    f2.score.cmp(&f1.score).then_with(|| f1.idx.cmp(&f2.idx))
}

fn fuzzy_match_func_compare(f1: &FuzmatchStr, f2: &FuzmatchStr) -> i32 {
    let b1 = f1.str_cstr.as_bytes().first().copied().unwrap_or(0);
    let b2 = f2.str_cstr.as_bytes().first().copied().unwrap_or(0);

    if b1 != b'<' && b2 == b'<' {
        return -1;
    }
    if b1 == b'<' && b2 != b'<' {
        return 1;
    }
    if f1.score == f2.score {
        return f1.idx.cmp(&f2.idx) as i32;
    }
    if f1.score > f2.score { -1 } else { 1 }
}

/// Fuzzy match `pat` in `str` (`fuzzy_match_str`).
pub fn fuzzy_match_str(
    str_ptr: impl crate::strings::IntoCCharPtr,
    pat: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let str_ptr = str_ptr.into_c_char_ptr();
    let pat = pat.into_c_char_ptr();
    if str_ptr.is_null() || pat.is_null() {
        return 0;
    }

    let mut score = FUZZY_SCORE_NONE;
    let mut matchpos = [0u32; FUZZY_MATCH_MAX_LEN];
    fuzzy_match(str_ptr, pat, true, &mut score, &mut matchpos);
    score
}

/// Fuzzy match positions of `pat` in `str` (`fuzzy_match_str_with_pos`).
pub fn fuzzy_match_str_with_pos(
    str_ptr: impl crate::strings::IntoCCharPtr,
    pat: impl crate::strings::IntoCCharPtr,
) -> Option<Vec<u32>> {
    let str_ptr = str_ptr.into_c_char_ptr();
    let pat = pat.into_c_char_ptr();
    if str_ptr.is_null() || pat.is_null() {
        return None;
    }

    let mut score = FUZZY_SCORE_NONE;
    let mut matches = [0u32; FUZZY_MATCH_MAX_LEN];
    if !fuzzy_match(str_ptr, pat, false, &mut score, &mut matches) || score == FUZZY_SCORE_NONE {
        return None;
    }

    let pat_bytes = crate::memory::cstr_as_bytes(pat);
    let mut match_positions = Vec::with_capacity(10);
    let mut j = 0usize;
    let mut off = 0usize;
    while off < pat_bytes.len() {
        let rest = &pat_bytes[off..];
        if !ascii_iswhite(utf_slice2char(rest)) {
            match_positions.push(matches[j]);
            j += 1;
        }
        off += utf_slice2len(rest);
    }

    Some(match_positions)
}

/// Fuzzy match a word in line `*ptr` (`fuzzy_match_str_in_line`).
pub fn fuzzy_match_str_in_line(
    ptr: &mut *mut u8,
    pat: impl crate::strings::IntoCCharPtr,
    len: &mut i32,
    current_pos: Option<&mut CursorPos>,
    score: &mut i32,
) -> bool {
    let pat = pat.into_c_char_ptr();
    let mut str_cur = *ptr;
    let str_begin = str_cur;
    let mut found = false;

    if str_cur.is_null() || pat.is_null() {
        return false;
    }
    let pat_bytes = crate::memory::cstr_as_bytes(pat);
    let line_end = find_line_end(str_cur);

    while (str_cur as usize) < (line_end as usize) {
        let start = find_word_start(str_cur);
        if crate::memory::cstr_is_empty(start.cast()) {
            break;
        }
        let end = find_word_end(start);
        let word_len = (end as usize) - (start as usize);
        let word_slice = crate::memory::cstr_slice(start.cast(), word_len);

        let mut matchpos = [0u32; FUZZY_MATCH_MAX_LEN];
        *score = FUZZY_SCORE_NONE;
        fuzzy_match_slice(word_slice, pat_bytes, true, score, &mut matchpos);

        if *score != FUZZY_SCORE_NONE {
            *len = word_len as i32;
            found = true;
            *ptr = start;
            if let Some(current_pos) = current_pos {
                current_pos.col += ((end as usize) - (str_begin as usize)) as i32;
            }
            break;
        }

        str_cur = end;
        while !crate::memory::cstr_is_empty(str_cur.cast()) && !vim_iswordp(str_cur.cast()) {
            let rest = crate::memory::cstr_as_bytes(str_cur.cast());
            str_cur = str_cur.wrapping_add(utf_slice2len(rest));
        }
    }

    if !found {
        *ptr = line_end;
    }

    found
}

/// Search for the next fuzzy match in `buf` (`search_for_fuzzy_match`).
pub fn search_for_fuzzy_match(
    buf: &mut FileBuf,
    pos: &mut CursorPos,
    pattern: impl crate::strings::IntoCCharPtr,
    dir: i32,
    start_pos: &CursorPos,
    len: &mut i32,
    ptr: &mut *mut u8,
    score: &mut i32,
) -> bool {
    let pattern = pattern.into_c_char_ptr();
    let mut current_pos = *pos;
    let mut found_new_match = false;
    let mut looped_around = false;

    let whole_line = ctrl_x_mode_whole_line();
    let circly_end = if std::ptr::eq(buf, g_st().curbuf) {
        *start_pos
    } else {
        CursorPos {
            lnum: buf.b_ml.ml_line_count,
            col: 0,
            coladd: 0,
        }
    };

    if whole_line && start_pos.lnum != pos.lnum {
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

        if current_pos.lnum >= 1 && current_pos.lnum <= buf.b_ml.ml_line_count {
            *ptr = ml_get_buf(buf, current_pos.lnum).cast();
            if !whole_line {
                *ptr = (*ptr).wrapping_add(current_pos.col as usize);
            }

            if !crate::memory::cstr_is_empty((*ptr).cast()) {
                if !whole_line {
                    found_new_match =
                        fuzzy_match_str_in_line(ptr, pattern, len, Some(&mut current_pos), score);
                    if found_new_match {
                        *pos = current_pos;
                        break;
                    } else if looped_around && current_pos.lnum == circly_end.lnum {
                        break;
                    }
                } else {
                    let line = *ptr;
                    let p = skipwhite(line.cast()).cast::<u8>();
                    if fuzzy_match_str(p.cast(), pattern) != FUZZY_SCORE_NONE {
                        found_new_match = true;
                        *pos = current_pos;
                        *ptr = p;
                        *len = ml_get_buf_len(buf, current_pos.lnum)
                            - (((p as usize) - (line as usize)) as i32);
                        break;
                    }
                }
            }
        }

        if dir == FORWARD {
            current_pos.lnum += 1;
            if current_pos.lnum > buf.b_ml.ml_line_count {
                if opt_vars().p_ws != 0 {
                    current_pos.lnum = 1;
                    looped_around = true;
                } else {
                    break;
                }
            }
        } else {
            current_pos.lnum -= 1;
            if current_pos.lnum < 1 {
                if opt_vars().p_ws != 0 {
                    current_pos.lnum = buf.b_ml.ml_line_count;
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

/// Sort `fuzmatch` by score and move strings into `matches`.
pub fn fuzzymatches_slice_to_strmatches(
    fuzmatch: &mut [FuzmatchStr],
    matches: &mut Vec<CString>,
    funcsort: bool,
) {
    matches.clear();
    if !fuzmatch.is_empty() {
        if funcsort {
            fuzmatch.sort_by(|a, b| fuzzy_match_func_compare(a, b).cmp(&0));
        } else {
            fuzmatch.sort_unstable_by(fuzzy_match_str_compare);
        }

        matches.extend(
            fuzmatch
                .iter_mut()
                .map(|item| std::mem::take(&mut item.str_cstr)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzymatches_to_strmatches_sorts_snr_functions_last() {
        let mut fm = [
            FuzmatchStr {
                idx: 0,
                str_cstr: c"<SNR>1_foo".to_owned(),
                score: 100,
            },
            FuzmatchStr {
                idx: 1,
                str_cstr: c"Bar".to_owned(),
                score: 50,
            },
        ];
        let mut matches: Vec<CString> = Vec::new();
        fuzzymatches_slice_to_strmatches(&mut fm, &mut matches, true);
        assert_eq!(matches[0].as_bytes(), b"Bar");
        assert_eq!(matches[1].as_bytes(), b"<SNR>1_foo");
    }
}
