//! C-indentation functions (`src/nvim/indent_c.c`).
use crate::buffer::{MAXLNUM, buf_T};
use crate::change::{
    COM_END, COM_LEFT, COM_MAX_LEN, COM_MIDDLE, COM_RIGHT, COM_START, KEY_OPEN_BACK, KEY_OPEN_FORW,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, getdigits_int, getwhitecols_curline, skiptowhite, skipwhite,
    vim_isIDc, vim_iswordc, vim_iswordp, vim_strsize,
};
use crate::cursor::{
    colnr_T, get_cursor_line_ptr, get_cursor_pos_ptr, linenr_T, lpos_T, lt, pos_T,
};
use crate::eval::typval::{EvalFuncData, kCallbackNone, tv_get_lnum, typval_T};
use crate::globals::*;
use crate::indent::{fixthisline, get_expr_indent, get_indent, get_indent_lnum, get_sw_value};
use crate::keycodes::get_special_key_code;
use crate::math::trim_to_int;
use crate::mbyte::{mb_prevptr, mb_strnicmp, utfc_ptr2len};
use crate::memline::{ml_get, ml_get_pos};
use crate::option::{copy_option_part, skip_to_option_part};
use crate::option_vars::*;
use crate::plines::getvcol;
use crate::regexp::MAXCOL;
use crate::search::{findmatchlimit, linewhite};
use crate::state::MODE_INSERT;
use crate::strings::{vim_strchr};
use std::ffi::{c_char, c_int};
use std::ptr;
pub(crate) const FM_BACKWARD: c_int = 0x01;
pub(crate) const FM_BLOCKSTOP: c_int = 0x04;
pub(crate) const FM_SKIPCOMM: c_int = 0x08;
pub(crate) const KEY_COMPLETE: c_int = 0x103;
const FIND_NAMESPACE_LIM: linenr_T = 20;
const BRACE_IN_COL0: c_int = 1;
const BRACE_AT_START: c_int = 2;
const BRACE_AT_END: c_int = 3;
const LOOKFOR_INITIAL: c_int = 0;
const LOOKFOR_IF: c_int = 1;
const LOOKFOR_DO: c_int = 2;
const LOOKFOR_CASE: c_int = 3;
const LOOKFOR_ANY: c_int = 4;
const LOOKFOR_TERM: c_int = 5;
const LOOKFOR_UNTERM: c_int = 6;
const LOOKFOR_SCOPEDECL: c_int = 7;
const LOOKFOR_NOBREAK: c_int = 8;
const LOOKFOR_CPP_BASECLASS: c_int = 9;
const LOOKFOR_ENUM_OR_INIT: c_int = 10;
const LOOKFOR_JS_KEY: c_int = 11;
const LOOKFOR_COMMA: c_int = 12;
#[repr(C)]
#[derive(Clone, Copy)]
struct cpp_baseclass_cache_T {
    found: c_int,
    lpos: lpos_T,
}
struct IndentCState {
    comment_pos_copy: pos_T,
    find_line_comment_pos: pos_T,
    start_brace_pos_copy: pos_T,
    match_char_pos_copy: pos_T,
}

impl IndentCState {
    const fn new() -> Self {
        Self {
            comment_pos_copy: pos_T { lnum: 0, col: 0, coladd: 0 },
            find_line_comment_pos: pos_T { lnum: 0, col: 0, coladd: 0 },
            start_brace_pos_copy: pos_T { lnum: 0, col: 0, coladd: 0 },
            match_char_pos_copy: pos_T { lnum: 0, col: 0, coladd: 0 },
        }
    }
}

static INDC_STATE: GlobalState<IndentCState> = GlobalState::new(IndentCState::new());

#[inline]
fn indc_st() -> &'static mut IndentCState {
    unsafe { &mut *INDC_STATE.as_ptr() }
}
use crate::keycodes::ctrl_chr;
/// Check if line[] contains a "//" comment, ignoring matches inside strings.
/// Return MAXCOL if not, otherwise return the column.
pub fn check_linecomment(line: *const c_char) -> c_int {
    unsafe {
        let mut p = line;
        if (*g_st().curbuf).b_p_lisp != 0 {
            if !vim_strchr(p, b';' as c_int).is_null() {
                let mut in_str = false;
                while {
                    p = crate::memory::cstr_as_bytes(p).iter().position(|&b| b == b'"' || b == b';').map_or(std::ptr::null(), |i| p.add(i));
                    !p.is_null()
                } {
                    if *p == b'"' as c_char {
                        if in_str {
                            if *p.sub(1) != b'\\' as c_char {
                                in_str = false;
                            }
                        } else if p == line
                            || (p.offset_from(line) >= 2
                                && *p.sub(1) != b'\\' as c_char
                                && *p.sub(2) != b'#' as c_char)
                        {
                            in_str = true;
                        }
                    } else if !in_str
                        && (p.offset_from(line) < 2
                            || (*p.sub(1) != b'\\' as c_char && *p.sub(2) != b'#' as c_char))
                        && is_pos_in_string(line, p.offset_from(line) as colnr_T) == 0
                    {
                        break;
                    }
                    p = p.add(1);
                }
            } else {
                p = std::ptr::null();
            }
        } else {
            while *p != NUL {
                p = skip_string(p);
                if *p == NUL {
                    break;
                }
                if *p == b'/' as c_char
                    && *p.add(1) == b'/' as c_char
                    && (p == line || *p.sub(1) != b'*' as c_char || *p.add(2) != b'*' as c_char)
                {
                    break;
                }
                p = p.add(1);
            }
            if *p == NUL {
                p = std::ptr::null();
            }
        }
        if p.is_null() {
            return MAXCOL;
        }
        p.offset_from(line) as c_int
    }
}
fn ind_find_start_comment() -> *mut pos_T {
    unsafe { find_start_comment((*g_st().curbuf).b_ind_maxcomment) }
}
pub fn find_start_comment(ind_maxcomment: c_int) -> *mut pos_T {
    unsafe {
        let mut pos: *mut pos_T;
        let mut cur_maxcomment = ind_maxcomment as i64;
        loop {
            pos = findmatchlimit(ptr::null_mut(), b'*' as c_int, FM_BACKWARD, cur_maxcomment);
            if pos.is_null() {
                break;
            }
            if is_pos_in_string(ml_get((*pos).lnum), (*pos).col) == 0 {
                break;
            }
            cur_maxcomment = ((**curwin.as_ptr()).w_cursor.lnum - (*pos).lnum - 1) as i64;
            if cur_maxcomment <= 0 {
                pos = ptr::null_mut();
                break;
            }
        }
        pos
    }
}
fn ind_find_start_CORS(is_raw: Option<&mut linenr_T>) -> *mut pos_T {
    unsafe {
        let mut comment_pos = find_start_comment((*g_st().curbuf).b_ind_maxcomment);
        if !comment_pos.is_null() {
            indc_st().comment_pos_copy = *comment_pos;
            comment_pos = &raw mut indc_st().comment_pos_copy;
        }
        let rs_pos = find_start_rawstring((*g_st().curbuf).b_ind_maxcomment);
        if comment_pos.is_null() || (!rs_pos.is_null() && lt(*rs_pos, *comment_pos)) {
            if !rs_pos.is_null()
                && let Some(is_raw) = is_raw
            {
                *is_raw = (*rs_pos).lnum;
            }
            return rs_pos;
        }
        comment_pos
    }
}
fn find_start_rawstring(ind_maxcomment: c_int) -> *mut pos_T {
    unsafe {
        let mut pos: *mut pos_T;
        let mut cur_maxcomment = ind_maxcomment;
        loop {
            pos = findmatchlimit(
                ptr::null_mut(),
                b'R' as c_int,
                FM_BACKWARD,
                cur_maxcomment as i64,
            );
            if pos.is_null() {
                break;
            }
            if is_pos_in_string(ml_get((*pos).lnum), (*pos).col) == 0 {
                break;
            }
            cur_maxcomment = (**curwin.as_ptr()).w_cursor.lnum - (*pos).lnum - 1;
            if cur_maxcomment <= 0 {
                pos = ptr::null_mut();
                break;
            }
        }
        pos
    }
}
fn skip_string(mut p: *const c_char) -> *const c_char {
    unsafe {
        loop {
            if *p == b'\'' as c_char {
                if *p.add(1) == NUL {
                    break;
                }
                let mut i: usize = 2;
                if *p.add(1) == b'\\' as c_char && *p.add(2) != NUL {
                    i += 1;
                    while ascii_isdigit(*p.add(i - 1)) {
                        i += 1;
                    }
                }
                if *p.add(i - 1) != NUL && *p.add(i) == b'\'' as c_char {
                    p = p.add(i + 1);
                    continue;
                }
            } else if *p == b'"' as c_char {
                p = p.add(1);
                while *p != NUL {
                    if *p == b'\\' as c_char && *p.add(1) != NUL {
                        p = p.add(1);
                    } else if *p == b'"' as c_char {
                        break;
                    }
                    p = p.add(1);
                }
                if *p == b'"' as c_char {
                    p = p.add(1);
                    continue;
                }
            } else if *p == b'R' as c_char && *p.add(1) == b'"' as c_char {
                let delim = p.add(2);
                let paren = vim_strchr(delim, b'(' as c_int);
                if !paren.is_null() {
                    let delim_len = paren.offset_from(delim) as usize;
                    p = p.add(3);
                    while *p != NUL {
                        if *p == b')' as c_char
                            && (crate::memory::cstr_prefix_bytes(p.add(1), delim_len) == crate::memory::cstr_prefix_bytes(delim, delim_len))
                            && *p.add(delim_len + 1) == b'"' as c_char
                        {
                            p = p.add(delim_len + 1);
                            break;
                        }
                        p = p.add(1);
                    }
                    if *p == b'"' as c_char {
                        p = p.add(1);
                        continue;
                    }
                }
            }
            break;
        }
        if *p == NUL {
            p = p.sub(1);
        }
        p
    }
}
pub fn is_pos_in_string(line: *const c_char, col: colnr_T) -> c_int {
    unsafe {
        let mut p = line;
        while *p != NUL && (p.offset_from(line) as colnr_T) < col {
            p = skip_string(p);
            p = p.add(1);
        }
        c_int::from(!((p.offset_from(line) as colnr_T) <= col))
    }
}
pub fn cin_is_cinword(mut line: *const c_char) -> bool {
    unsafe {
        let mut retval = false;
        let cinw_len = crate::memory::cstr_len((*g_st().curbuf).b_p_cinw) + 1;
        let mut cinw_buf = vec![0 as c_char; cinw_len];
        line = skipwhite(line);
        let mut cinw = (*g_st().curbuf).b_p_cinw;
        while *cinw != NUL {
            let len = copy_option_part(
                &raw mut cinw,
                cinw_buf.as_mut_ptr(),
                cinw_len,
                c",".as_ptr(),
            );
            if (crate::memory::cstr_prefix_bytes(line, len) == crate::memory::cstr_prefix_bytes(cinw_buf.as_ptr(), len))
                && (!vim_iswordc(*line.add(len) as u8 as c_int)
                    || (len > 0 && !vim_iswordc(*line.add(len - 1) as u8 as c_int)))
            {
                retval = true;
                break;
            }
        }
        retval
    }
}
pub fn cindent_on() -> bool {
    unsafe {
        opt_vars().p_paste == 0 && ((*g_st().curbuf).b_p_cin != 0 || (*g_st().curbuf).b_p_inde.cb_type() != kCallbackNone)
    }
}
fn cin_skipcomment(mut s: *const c_char) -> *const c_char {
    unsafe {
        while *s != NUL {
            let prev_s = s;
            s = skipwhite(s);
            if (*g_st().curbuf).b_ind_hash_comment != 0 && s != prev_s && *s == b'#' as c_char {
                s = s.add(crate::memory::cstr_len(s));
                break;
            }
            if *s != b'/' as c_char {
                break;
            }
            s = s.add(1);
            if *s == b'/' as c_char {
                s = s.add(crate::memory::cstr_len(s));
                break;
            }
            if *s != b'*' as c_char {
                break;
            }
            s = s.add(1);
            while *s != NUL {
                if *s == b'*' as c_char && *s.add(1) == b'/' as c_char {
                    s = s.add(2);
                    break;
                }
                s = s.add(1);
            }
        }
        s
    }
}
fn cin_nocode(s: *const c_char) -> bool {
    unsafe { *cin_skipcomment(s) == NUL }
}
fn find_line_comment() -> *mut pos_T {
    unsafe {
        indc_st().find_line_comment_pos = (**curwin.as_ptr()).w_cursor;
        loop {
            indc_st().find_line_comment_pos.lnum -= 1;
            if indc_st().find_line_comment_pos.lnum <= 0 {
                break;
            }
            let line = ml_get(indc_st().find_line_comment_pos.lnum);
            let p = skipwhite(line);
            if cin_islinecomment(p) {
                indc_st().find_line_comment_pos.col = p.offset_from(line) as colnr_T;
                return &raw mut indc_st().find_line_comment_pos;
            }
            if *p != NUL {
                break;
            }
        }
        ptr::null_mut()
    }
}
fn cin_has_js_key(text: *const c_char) -> bool {
    unsafe {
        let mut s: *const c_char = skipwhite(text);
        let mut quote: c_char = 0;
        if *s == b'\'' as c_char || *s == b'"' as c_char {
            quote = *s;
            s = s.add(1);
        }
        if !vim_isIDc(*s as u8 as c_int) {
            return false;
        }
        while vim_isIDc(*s as u8 as c_int) {
            s = s.add(1);
        }
        if *s != NUL && *s == quote {
            s = s.add(1);
        }
        s = cin_skipcomment(s);
        *s == b':' as c_char && *s.add(1) != b':' as c_char
    }
}
fn cin_islabel_skip(s: *mut *const c_char) -> bool {
    unsafe {
        if !vim_isIDc(*(*s) as u8 as c_int) {
            return false;
        }
        while vim_isIDc(*(*s) as u8 as c_int) {
            *s = (*s).add(utfc_ptr2len(*s) as usize);
        }
        *s = cin_skipcomment(*s);
        if *(*s) == b':' as c_char {
            *s = (*s).add(1);
            *(*s) != b':' as c_char
        } else {
            false
        }
    }
}
fn cin_islabel() -> bool {
    unsafe {
        let mut s = cin_skipcomment(get_cursor_line_ptr());
        if cin_isdefault(s) {
            return false;
        }
        if cin_isscopedecl(s) {
            return false;
        }
        if !cin_islabel_skip(&raw mut s) {
            return false;
        }
        if !ind_find_start_CORS(None).is_null() {
            return false;
        }
        let cursor_save = (**curwin.as_ptr()).w_cursor;
        while (**curwin.as_ptr()).w_cursor.lnum > 1 {
            (**curwin.as_ptr()).w_cursor.lnum -= 1;
            (**curwin.as_ptr()).w_cursor.col = 0;
            let trypos = ind_find_start_CORS(None);
            if !trypos.is_null() {
                (**curwin.as_ptr()).w_cursor = *trypos;
            }
            let mut line: *const c_char = get_cursor_line_ptr();
            if cin_ispreproc(line) {
                continue;
            }
            line = cin_skipcomment(line);
            if *line == NUL {
                continue;
            }
            (**curwin.as_ptr()).w_cursor = cursor_save;
            if cin_isterminated(line, true, false) != 0
                || cin_isscopedecl(line)
                || cin_iscase(line, true)
                || (cin_islabel_skip(&raw mut line) && cin_nocode(line))
            {
                return true;
            }
            return false;
        }
        (**curwin.as_ptr()).w_cursor = cursor_save;
        true
    }
}
fn cin_skip_comment_and_string(s: *const c_char) -> *const c_char {
    unsafe {
        let mut r: *const c_char;
        let mut p = s;
        loop {
            r = p;
            p = cin_skipcomment(p);
            if *p != NUL {
                p = skip_string(p);
            }
            if p == r {
                break;
            }
        }
        p
    }
}
fn cin_is_compound_init(s: *const c_char) -> bool {
    unsafe {
        let mut p = s;
        let mut r: *const c_char = std::ptr::null();
        while *p != NUL {
            if *p == b'=' as c_char {
                r = cin_skipcomment(p.add(1));
                p = r;
            } else if (crate::memory::cstr_prefix_bytes(p, 6) == &b"return"[..6])
                && !vim_isIDc(*p.add(6) as u8 as c_int)
                && (p == s || (p > s && !vim_isIDc(*p.sub(1) as u8 as c_int)))
            {
                r = cin_skipcomment(p.add(6));
                p = r;
            } else {
                p = cin_skip_comment_and_string(p.add(1));
            }
        }
        if r.is_null() {
            return false;
        }
        p = r;
        if cin_nocode(p) {
            return true;
        }
        if *p == b'&' as c_char {
            p = cin_skipcomment(p.add(1));
        }
        if *p == b'(' as c_char {
            let mut open_count: c_int = 1;
            loop {
                p = cin_skip_comment_and_string(p.add(1));
                if cin_nocode(p) {
                    return true;
                }
                open_count += c_int::from(*p == b'(' as c_char) - c_int::from(*p == b')' as c_char);
                if open_count == 0 {
                    break;
                }
            }
            p = cin_skipcomment(p.add(1));
            if cin_nocode(p) {
                return true;
            }
        }
        while *p == b'{' as c_char {
            p = cin_skipcomment(p.add(1));
        }
        cin_nocode(p)
    }
}
fn cin_isinit() -> bool {
    unsafe {
        const SKIP: [(&[u8], *const c_char); 4] = [
            (b"static", c"static".as_ptr()),
            (b"public", c"public".as_ptr()),
            (b"protected", c"protected".as_ptr()),
            (b"private", c"private".as_ptr()),
        ];
        let mut s = cin_skipcomment(get_cursor_line_ptr());
        if cin_starts_with(s, c"typedef".as_ptr()) {
            s = cin_skipcomment(s.add(7));
        }
        loop {
            let mut matched = false;
            for &(bytes, word) in &SKIP {
                if cin_starts_with(s, word) {
                    s = cin_skipcomment(s.add(bytes.len()));
                    matched = true;
                    break;
                }
            }
            if !matched {
                break;
            }
        }
        if cin_starts_with(s, c"enum".as_ptr()) {
            return true;
        }
        cin_is_compound_init(s)
    }
}
fn cin_iscase(mut s: *const c_char, strict: bool) -> bool {
    unsafe {
        s = cin_skipcomment(s);
        if cin_starts_with(s, c"case".as_ptr()) {
            s = s.add(4);
            while *s != NUL {
                s = cin_skipcomment(s);
                if *s == NUL {
                    break;
                }
                if *s == b':' as c_char {
                    if *s.add(1) == b':' as c_char {
                        s = s.add(1);
                    } else {
                        return true;
                    }
                }
                if *s == b'\'' as c_char && *s.add(1) != NUL && *s.add(2) == b'\'' as c_char {
                    s = s.add(2);
                } else if *s == b'/' as c_char
                    && (*s.add(1) == b'*' as c_char || *s.add(1) == b'/' as c_char)
                {
                    return false;
                } else if *s == b'"' as c_char {
                    if strict {
                        return false;
                    }
                    return true;
                }
                s = s.add(1);
            }
            return false;
        }
        if cin_isdefault(s) {
            return true;
        }
        false
    }
}
fn cin_isdefault(mut s: *const c_char) -> bool {
    unsafe {
        if !(crate::memory::cstr_prefix_bytes(s, 7) == &b"default"[..7]) {
            return false;
        }
        s = cin_skipcomment(s.add(7));
        *s == b':' as c_char && *s.add(1) != b':' as c_char
    }
}
fn cin_isscopedecl(p: *const c_char) -> bool {
    unsafe {
        let s = cin_skipcomment(p);
        let cinsd_len = crate::memory::cstr_len((*g_st().curbuf).b_p_cinsd) + 1;
        let mut cinsd_buf = vec![0 as c_char; cinsd_len];
        let mut found = false;
        let mut cinsd = (*g_st().curbuf).b_p_cinsd;
        while *cinsd != NUL {
            let len = copy_option_part(
                &raw mut cinsd,
                cinsd_buf.as_mut_ptr(),
                cinsd_len,
                c",".as_ptr(),
            );
            if crate::memory::cstr_prefix_bytes(s, len) == crate::memory::cstr_prefix_bytes(cinsd_buf.as_ptr(), len) {
                let skip = cin_skipcomment(s.add(len));
                if *skip == b':' as c_char && *skip.add(1) != b':' as c_char {
                    found = true;
                    break;
                }
            }
        }
        found
    }
}
fn cin_is_cpp_namespace(mut s: *const c_char) -> bool {
    unsafe {
        let mut has_name = false;
        let mut has_name_start = false;
        s = cin_skipcomment(s);
        while ((crate::memory::cstr_prefix_bytes(s, 6) == &b"inline"[..6]) || (crate::memory::cstr_prefix_bytes(s, 6) == &b"export"[..6]))
            && (*s.add(6) == NUL || !vim_iswordc(*s.add(6) as u8 as c_int))
        {
            s = cin_skipcomment(skipwhite(s.add(6)));
        }
        if (crate::memory::cstr_prefix_bytes(s, 9) == &b"namespace"[..9])
            && (*s.add(9) == NUL || !vim_iswordc(*s.add(9) as u8 as c_int))
        {
            let mut p = cin_skipcomment(skipwhite(s.add(9)));
            while *p != NUL {
                if ascii_iswhite(*p) {
                    has_name = true;
                    p = cin_skipcomment(skipwhite(p));
                } else if *p == b'{' as c_char {
                    break;
                } else if vim_iswordc(*p as u8 as c_int) {
                    has_name_start = true;
                    if has_name {
                        return false;
                    }
                    p = p.add(1);
                } else if *p == b':' as c_char
                    && *p.add(1) == b':' as c_char
                    && vim_iswordc(*p.add(2) as u8 as c_int)
                {
                    if !has_name_start || has_name {
                        return false;
                    }
                    p = p.add(3);
                } else {
                    return false;
                }
            }
            return true;
        }
        false
    }
}
fn after_label(mut l: *const c_char) -> *const c_char {
    unsafe {
        while *l != NUL {
            if *l == b':' as c_char {
                if *l.add(1) == b':' as c_char {
                    l = l.add(1);
                } else if !cin_iscase(l.add(1), false) {
                    break;
                }
            } else if *l == b'\'' as c_char && *l.add(1) != NUL && *l.add(2) == b'\'' as c_char {
                l = l.add(2);
            }
            l = l.add(1);
        }
        if *l == NUL {
            return std::ptr::null();
        }
        l = cin_skipcomment(l.add(1));
        if *l == NUL {
            return std::ptr::null();
        }
        l
    }
}
fn get_indent_nolabel(lnum: linenr_T) -> c_int {
    unsafe {
        let l = ml_get(lnum);
        let p = after_label(l);
        if p.is_null() {
            return 0;
        }
        let mut fp = pos_T {
            lnum,
            col: p.offset_from(l) as colnr_T,
            coladd: 0,
        };
        let mut col: colnr_T = 0;
        getvcol(
            &mut **curwin.as_ptr(),
            &mut fp,
            Some(&mut col),
            None,
            None,
            0,
        );
        col
    }
}
fn skip_label(lnum: linenr_T, pp: *mut *const c_char) -> c_int {
    unsafe {
        let cursor_save = (**curwin.as_ptr()).w_cursor;
        (**curwin.as_ptr()).w_cursor.lnum = lnum;
        let mut l: *const c_char = get_cursor_line_ptr();
        let amount = if cin_iscase(l, false) || cin_isscopedecl(l) || cin_islabel() {
            let amt = get_indent_nolabel(lnum);
            l = after_label(get_cursor_line_ptr());
            if l.is_null() {
                l = get_cursor_line_ptr();
            }
            amt
        } else {
            let amt = get_indent();
            l = get_cursor_line_ptr();
            amt
        };
        *pp = l;
        (**curwin.as_ptr()).w_cursor = cursor_save;
        amount
    }
}
fn cin_first_id_amount() -> c_int {
    unsafe {
        let line = get_cursor_line_ptr();
        let mut p = skipwhite(line);
        let mut len = skiptowhite(p).offset_from(p) as c_int;
        if len == 6 && (crate::memory::cstr_prefix_bytes(p, 6) == &b"static"[..6]) {
            p = skipwhite(p.add(6));
            len = skiptowhite(p).offset_from(p) as c_int;
        }
        if len == 6 && (crate::memory::cstr_prefix_bytes(p, 6) == &b"struct"[..6]) {
            p = skipwhite(p.add(6));
        } else if len == 4 && (crate::memory::cstr_prefix_bytes(p, 4) == &b"enum"[..4]) {
            p = skipwhite(p.add(4));
        } else if (len == 8 && (crate::memory::cstr_prefix_bytes(p, 8) == &b"unsigned"[..8]))
            || (len == 6 && (crate::memory::cstr_prefix_bytes(p, 6) == &b"signed"[..6]))
        {
            let s = skipwhite(p.add(len as usize));
            if ((crate::memory::cstr_prefix_bytes(s, 3) == &b"int"[..3]) && ascii_iswhite(*s.add(3)))
                || ((crate::memory::cstr_prefix_bytes(s, 4) == &b"long"[..4]) && ascii_iswhite(*s.add(4)))
                || ((crate::memory::cstr_prefix_bytes(s, 5) == &b"short"[..5]) && ascii_iswhite(*s.add(5)))
                || ((crate::memory::cstr_prefix_bytes(s, 4) == &b"char"[..4]) && ascii_iswhite(*s.add(4)))
            {
                p = s;
            }
        }
        len = 0;
        while vim_isIDc(*p.add(len as usize) as u8 as c_int) {
            len += 1;
        }
        if len == 0 || !ascii_iswhite(*p.add(len as usize)) || cin_nocode(p) {
            return 0;
        }
        p = skipwhite(p.add(len as usize));
        let mut fp = pos_T {
            lnum: (**curwin.as_ptr()).w_cursor.lnum,
            col: p.offset_from(line) as colnr_T,
            coladd: 0,
        };
        let mut col: colnr_T = 0;
        getvcol(
            &mut **curwin.as_ptr(),
            &mut fp,
            Some(&mut col),
            None,
            None,
            0,
        );
        col
    }
}
fn cin_get_equal_amount(lnum: linenr_T) -> c_int {
    unsafe {
        if lnum > 1 {
            let line = ml_get(lnum - 1);
            if *line != NUL && *line.add(crate::memory::cstr_len(line) - 1) == b'\\' as c_char {
                return -1;
            }
        }
        let line: *const c_char = ml_get(lnum);
        let mut s = line;
        while *s != NUL && vim_strchr(c"=;{}\"'".as_ptr(), *s as u8 as c_int).is_null() {
            if cin_iscomment(s) {
                s = cin_skipcomment(s);
            } else {
                s = s.add(1);
            }
        }
        if *s != b'=' as c_char {
            return 0;
        }
        s = skipwhite(s.add(1));
        if cin_nocode(s) {
            return 0;
        }
        if *s == b'"' as c_char {
            s = s.add(1);
        }
        let mut fp = pos_T {
            lnum,
            col: s.offset_from(line) as colnr_T,
            coladd: 0,
        };
        let mut col: colnr_T = 0;
        getvcol(
            &mut **curwin.as_ptr(),
            &mut fp,
            Some(&mut col),
            None,
            None,
            0,
        );
        col
    }
}
fn cin_ispreproc(s: *const c_char) -> bool {
    unsafe { *skipwhite(s) == b'#' as c_char }
}
fn cin_ispreproc_cont(
    pp: *mut *const c_char,
    lnump: &mut linenr_T,
    amount: &mut c_int,
) -> bool {
    unsafe {
        let mut line = *pp;
        let mut lnum = *lnump;
        let mut retval = false;
        let mut candidate_amount = *amount;
        if *line != NUL && *line.add(crate::memory::cstr_len(line) - 1) == b'\\' as c_char {
            candidate_amount = get_indent_lnum(lnum);
        }
        loop {
            if cin_ispreproc(line) {
                retval = true;
                *lnump = lnum;
                break;
            }
            if lnum == 1 {
                break;
            }
            lnum -= 1;
            line = ml_get(lnum);
            if *line == NUL || *line.add(crate::memory::cstr_len(line) - 1) != b'\\' as c_char {
                break;
            }
        }
        if lnum != *lnump {
            *pp = ml_get(*lnump);
        }
        if retval {
            *amount = candidate_amount;
        }
        retval
    }
}
#[inline]
fn cin_iscomment(p: *const c_char) -> bool {
    unsafe { *p == b'/' as c_char && (*p.add(1) == b'*' as c_char || *p.add(1) == b'/' as c_char) }
}
#[inline]
fn cin_islinecomment(p: *const c_char) -> bool {
    unsafe { *p == b'/' as c_char && *p.add(1) == b'/' as c_char }
}
fn cin_isterminated(mut s: *const c_char, incl_open: bool, incl_comma: bool) -> c_char {
    unsafe {
        let mut found_start: c_char = 0;
        let mut n_open: u32 = 0;
        let mut is_else = false;
        s = cin_skipcomment(s);
        if *s == b'{' as c_char || (*s == b'}' as c_char && !cin_iselse(s)) {
            found_start = *s;
        }
        if found_start == 0 {
            is_else = cin_iselse(s);
        }
        while *s != NUL {
            s = skip_string(cin_skipcomment(s));
            if *s == b'}' as c_char && n_open > 0 {
                n_open -= 1;
            }
            if (!is_else || n_open == 0)
                && (*s == b';' as c_char
                    || *s == b'}' as c_char
                    || (incl_comma && *s == b',' as c_char))
                && cin_nocode(s.add(1))
            {
                return *s;
            } else if *s == b'{' as c_char {
                if incl_open && cin_nocode(s.add(1)) {
                    return *s;
                } else {
                    n_open += 1;
                }
            }
            if *s != NUL {
                s = s.add(1);
            }
        }
        found_start
    }
}
fn cin_isfuncdecl(sp: *mut *const c_char, first_lnum: linenr_T, min_lnum: linenr_T) -> bool {
    unsafe {
        let mut s: *const c_char = if sp.is_null() {
            ml_get(first_lnum)
        } else {
            *sp
        };
        let mut lnum = first_lnum;
        let save_lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let mut retval = false;
        let mut just_started = true;
        (**curwin.as_ptr()).w_cursor.lnum = lnum;
        if find_last_paren(s, b'(' as c_char, b')' as c_char) {
            let trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
            if !trypos.is_null() {
                lnum = (*trypos).lnum;
                if lnum < min_lnum {
                    (**curwin.as_ptr()).w_cursor.lnum = save_lnum;
                    return false;
                }
                s = ml_get(lnum);
            }
        }
        (**curwin.as_ptr()).w_cursor.lnum = save_lnum;
        if cin_ispreproc(s) {
            return false;
        }
        while *s != NUL
            && *s != b'(' as c_char
            && *s != b';' as c_char
            && *s != b'\'' as c_char
            && *s != b'"' as c_char
        {
            if cin_iscomment(s) {
                s = cin_skipcomment(s);
            } else if *s == b':' as c_char {
                if *s.add(1) == b':' as c_char {
                    s = s.add(2);
                } else {
                    return false;
                }
            } else {
                s = s.add(1);
            }
        }
        if *s != b'(' as c_char {
            return false;
        }
        'done: {
            while *s != NUL && *s != b';' as c_char && *s != b'\'' as c_char && *s != b'"' as c_char
            {
                if *s == b')' as c_char && cin_nocode(s.add(1)) {
                    lnum = first_lnum - 1;
                    s = ml_get(lnum);
                    if *s == NUL || *s.add(crate::memory::cstr_len(s) - 1) != b'\\' as c_char {
                        retval = true;
                    }
                    break 'done;
                }
                if (*s == b',' as c_char && cin_nocode(s.add(1)))
                    || *s.add(1) == NUL
                    || cin_nocode(s)
                {
                    let comma = *s == b',' as c_char;
                    loop {
                        if lnum >= (*g_st().curbuf).b_ml.ml_line_count {
                            break;
                        }
                        lnum += 1;
                        s = ml_get(lnum);
                        if !cin_ispreproc(s) {
                            break;
                        }
                    }
                    if lnum >= (*g_st().curbuf).b_ml.ml_line_count {
                        break;
                    }
                    s = skipwhite(s);
                    if !just_started && (!comma && *s != b',' as c_char && *s != b')' as c_char) {
                        break;
                    }
                    just_started = false;
                } else if cin_iscomment(s) {
                    s = cin_skipcomment(s);
                } else {
                    s = s.add(1);
                    just_started = false;
                }
            }
        }
        if lnum != first_lnum && !sp.is_null() {
            *sp = ml_get(first_lnum);
        }
        retval
    }
}
fn cin_isif(p: *const c_char) -> bool {
    cin_starts_with(p, c"if".as_ptr())
}
fn cin_iselse(mut p: *const c_char) -> bool {
    unsafe {
        if *p == b'}' as c_char {
            p = cin_skipcomment(p.add(1));
        }
        cin_starts_with(p, c"else".as_ptr())
    }
}
fn cin_isdo(p: *const c_char) -> bool {
    cin_starts_with(p, c"do".as_ptr())
}
fn cin_iswhileofdo(mut p: *const c_char, lnum: linenr_T) -> bool {
    unsafe {
        let mut retval = false;
        p = cin_skipcomment(p);
        if *p == b'}' as c_char {
            p = cin_skipcomment(p.add(1));
        }
        if cin_starts_with(p, c"while".as_ptr()) {
            let cursor_save = (**curwin.as_ptr()).w_cursor;
            (**curwin.as_ptr()).w_cursor.lnum = lnum;
            (**curwin.as_ptr()).w_cursor.col = 0;
            p = get_cursor_line_ptr();
            while *p != NUL && *p != b'w' as c_char {
                p = p.add(1);
                (**curwin.as_ptr()).w_cursor.col += 1;
            }
            let trypos = findmatchlimit(
                ptr::null_mut(),
                0,
                FM_SKIPCOMM,
                (*g_st().curbuf).b_ind_maxparen as i64,
            );
            if !trypos.is_null() && *cin_skipcomment(ml_get_pos(&*trypos).add(1)) == b';' as c_char {
                retval = true;
            }
            (**curwin.as_ptr()).w_cursor = cursor_save;
        }
        retval
    }
}
fn cin_is_if_for_while_before_offset(line: *const c_char, poffset: &mut colnr_T) -> c_int {
    unsafe {
        let mut offset = *poffset;
        let prev_offset = offset;
        offset -= 1;
        if prev_offset < 2 {
            return 0;
        }
        while offset > 2 && ascii_iswhite(*line.add(offset as usize)) {
            offset -= 1;
        }
        offset -= 1;
        let mut probably_found = false;
        if crate::memory::cstr_prefix_bytes(line.add(offset as usize), 2) == &b"if"[..2] {
            probably_found = true;
        } else if offset >= 1 {
            offset -= 1;
            if crate::memory::cstr_prefix_bytes(line.add(offset as usize), 3) == &b"for"[..3] {
                probably_found = true;
            } else if offset >= 2 {
                offset -= 2;
                if crate::memory::cstr_prefix_bytes(line.add(offset as usize), 5) == &b"while"[..5] {
                    probably_found = true;
                }
            }
        }
        if !probably_found {
            return 0;
        }
        if offset == 0 || !vim_isIDc(*line.add((offset - 1) as usize) as u8 as c_int) {
            *poffset = offset;
            return 1;
        }
        0
    }
}
fn cin_iswhileofdo_end(terminated: c_int) -> bool {
    unsafe {
        if terminated != b';' as c_int {
            return false;
        }
        let mut line: *const c_char = get_cursor_line_ptr();
        let mut p = line;
        while *p != NUL {
            p = cin_skipcomment(p);
            if *p == b')' as c_char {
                let s = skipwhite(p.add(1));
                if *s == b';' as c_char && cin_nocode(s.add(1)) {
                    let i = p.offset_from(line) as colnr_T;
                    (**curwin.as_ptr()).w_cursor.col = i;
                    let trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                    if !trypos.is_null() {
                        let mut s2 = cin_skipcomment(ml_get((*trypos).lnum));
                        if *s2 == b'}' as c_char {
                            s2 = cin_skipcomment(s2.add(1));
                        }
                        if cin_starts_with(s2, c"while".as_ptr()) {
                            (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum;
                            return true;
                        }
                    }
                    line = get_cursor_line_ptr();
                    p = line.add(i as usize);
                }
            }
            if *p != NUL {
                p = p.add(1);
            }
        }
        false
    }
}
fn cin_isbreak(p: *const c_char) -> bool {
    cin_starts_with(p, c"break".as_ptr())
}
fn cin_is_cpp_baseclass(cached: &mut cpp_baseclass_cache_T) -> bool {
    unsafe {
        let pos = &raw mut cached.lpos;
        let mut lnum = (**curwin.as_ptr()).w_cursor.lnum;
        let mut line: *const c_char = get_cursor_line_ptr();
        if (*pos).lnum <= lnum {
            return cached.found != 0;
        }
        (*pos).col = 0;
        let mut s: *const c_char = skipwhite(line);
        if *s == b'#' as c_char {
            return false;
        }
        s = cin_skipcomment(s);
        if *s == NUL {
            return false;
        }
        let mut cpp_base_class = false;
        let mut lookfor_ctor_init = false;
        let mut class_or_struct = false;
        while lnum > 1 {
            line = ml_get(lnum - 1);
            s = skipwhite(line);
            if *s == b'#' as c_char || *s == NUL {
                break;
            }
            while *s != NUL {
                s = cin_skipcomment(s);
                if *s == b'{' as c_char
                    || *s == b'}' as c_char
                    || (*s == b';' as c_char && cin_nocode(s.add(1)))
                {
                    break;
                }
                if *s != NUL {
                    s = s.add(1);
                }
            }
            if *s != NUL {
                break;
            }
            lnum -= 1;
        }
        (*pos).lnum = lnum;
        line = ml_get(lnum);
        s = line;
        loop {
            if *s == NUL {
                if lnum == (**curwin.as_ptr()).w_cursor.lnum {
                    break;
                }
                lnum += 1;
                line = ml_get(lnum);
                s = line;
            }
            if s == line {
                if cin_iscase(s, false) {
                    break;
                }
                s = cin_skipcomment(line);
                if *s == NUL {
                    continue;
                }
            }
            if *s == b'"' as c_char || (*s == b'R' as c_char && *s.add(1) == b'"' as c_char) {
                s = skip_string(s).add(1);
            } else if *s == b':' as c_char {
                if *s.add(1) == b':' as c_char {
                    lookfor_ctor_init = false;
                    s = cin_skipcomment(s.add(2));
                } else if lookfor_ctor_init || class_or_struct {
                    cpp_base_class = true;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                    (*pos).col = 0;
                    s = cin_skipcomment(s.add(1));
                } else {
                    s = cin_skipcomment(s.add(1));
                }
            } else if ((crate::memory::cstr_prefix_bytes(s, 5) == &b"class"[..5])
                && !vim_isIDc(*s.add(5) as u8 as c_int))
                || ((crate::memory::cstr_prefix_bytes(s, 6) == &b"struct"[..6]) && !vim_isIDc(*s.add(6) as u8 as c_int))
            {
                class_or_struct = true;
                lookfor_ctor_init = false;
                if *s == b'c' as c_char {
                    s = cin_skipcomment(s.add(5));
                } else {
                    s = cin_skipcomment(s.add(6));
                }
            } else {
                if *s == b'{' as c_char || *s == b'}' as c_char || *s == b';' as c_char {
                    cpp_base_class = false;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                } else if *s == b')' as c_char {
                    class_or_struct = false;
                    lookfor_ctor_init = true;
                } else if *s == b'?' as c_char {
                    return false;
                } else if !vim_isIDc(*s as u8 as c_int) {
                    class_or_struct = false;
                    lookfor_ctor_init = false;
                } else if (*pos).col == 0 {
                    lookfor_ctor_init = false;
                    if cpp_base_class {
                        (*pos).col = s.offset_from(line) as colnr_T;
                    }
                }
                if lnum == (**curwin.as_ptr()).w_cursor.lnum && *s == b',' as c_char && cin_nocode(s.add(1)) {
                    (*pos).col = 0;
                }
                s = cin_skipcomment(s.add(1));
            }
        }
        cached.found = c_int::from(cpp_base_class);
        if cpp_base_class {
            (*pos).lnum = lnum;
        }
        cpp_base_class
    }
}
fn get_baseclass_amount(col: colnr_T) -> c_int {
    unsafe {
        let mut amount: c_int;
        let mut vcol: colnr_T = 0;
        if col == 0 {
            amount = get_indent();
            if find_last_paren(get_cursor_line_ptr(), b'(' as c_char, b')' as c_char) {
                let trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                if !trypos.is_null() {
                    amount = get_indent_lnum((*trypos).lnum);
                }
            }
            if !cin_ends_in(get_cursor_line_ptr(), c",".as_ptr()) {
                amount += (*g_st().curbuf).b_ind_cpp_baseclass;
            }
        } else {
            (**curwin.as_ptr()).w_cursor.col = col;
            getvcol(
                &mut **curwin.as_ptr(),
                &mut (**curwin.as_ptr()).w_cursor,
                Some(&mut vcol),
                None,
                None,
                0,
            );
            amount = vcol;
        }
        if amount < (*g_st().curbuf).b_ind_cpp_baseclass {
            amount = (*g_st().curbuf).b_ind_cpp_baseclass;
        }
        amount
    }
}
fn cin_ends_in(s: *const c_char, find: *const c_char) -> bool {
    unsafe {
        let mut p = s;
        let len = crate::memory::cstr_len(find);
        while *p != NUL {
            p = cin_skipcomment(p);
            if crate::memory::cstr_prefix_bytes(p, len) == crate::memory::cstr_prefix_bytes(find, len) {
                let r = skipwhite(p.add(len));
                if cin_nocode(r) {
                    return true;
                }
            }
            if *p != NUL {
                p = p.add(1);
            }
        }
        false
    }
}
fn cin_starts_with(s: *const c_char, word: *const c_char) -> bool {
    unsafe {
        let wb = crate::memory::cstr_as_bytes(word);
        let l = wb.len();
        crate::memory::xstrnlen(s, l) == l
            && std::slice::from_raw_parts(s.cast::<u8>(), l) == wb
            && !vim_isIDc(*s.add(l) as u8 as c_int)
    }
}
fn cin_is_cpp_extern_c(mut s: *const c_char) -> bool {
    unsafe {
        let mut has_string_literal = false;
        s = cin_skipcomment(s);
        if (crate::memory::cstr_prefix_bytes(s, 6) == &b"extern"[..6])
            && (*s.add(6) == NUL || !vim_iswordc(*s.add(6) as u8 as c_int))
        {
            let mut p = cin_skipcomment(skipwhite(s.add(6)));
            while *p != NUL {
                if ascii_iswhite(*p) {
                    p = cin_skipcomment(skipwhite(p));
                } else if *p == b'{' as c_char {
                    break;
                } else if *p == b'"' as c_char
                    && *p.add(1) == b'C' as c_char
                    && *p.add(2) == b'"' as c_char
                {
                    if has_string_literal {
                        return false;
                    }
                    has_string_literal = true;
                    p = p.add(3);
                } else if *p == b'"' as c_char
                    && *p.add(1) == b'C' as c_char
                    && *p.add(2) == b'+' as c_char
                    && *p.add(3) == b'+' as c_char
                    && *p.add(4) == b'"' as c_char
                {
                    if has_string_literal {
                        return false;
                    }
                    has_string_literal = true;
                    p = p.add(5);
                } else {
                    return false;
                }
            }
            return has_string_literal;
        }
        false
    }
}
fn cin_skip2pos(trypos: &mut pos_T) -> c_int {
    unsafe {
        let line: *const c_char = ml_get(trypos.lnum);
        let mut p = line;
        while *p != NUL && (p.offset_from(line) as colnr_T) < trypos.col {
            if cin_iscomment(p) {
                p = cin_skipcomment(p);
            } else {
                let new_p = skip_string(p);
                if new_p == p {
                    p = p.add(1);
                } else {
                    p = new_p;
                }
            }
        }
        p.offset_from(line) as c_int
    }
}
fn find_start_brace() -> *mut pos_T {
    unsafe {
        let cursor_save = (**curwin.as_ptr()).w_cursor;
        let mut trypos: *mut pos_T;
        loop {
            trypos = findmatchlimit(
                ptr::null_mut(),
                b'{' as c_int,
                FM_BLOCKSTOP | FM_SKIPCOMM,
                0,
            );
            if trypos.is_null() {
                break;
            }
            indc_st().start_brace_pos_copy = *trypos;
            trypos = &raw mut indc_st().start_brace_pos_copy;
            (**curwin.as_ptr()).w_cursor = *trypos;
            let mut pos: *mut pos_T = ptr::null_mut();
            if cin_skip2pos(&mut *trypos) as colnr_T == (*trypos).col && {
                pos = ind_find_start_CORS(None);
                pos.is_null()
            } {
                break;
            }
            if !pos.is_null() {
                (**curwin.as_ptr()).w_cursor = *pos;
            }
        }
        (**curwin.as_ptr()).w_cursor = cursor_save;
        trypos
    }
}
fn find_match_paren(ind_maxparen: c_int) -> *mut pos_T {
    find_match_char(b'(' as c_char, ind_maxparen)
}
fn find_match_char(c: c_char, ind_maxparen: c_int) -> *mut pos_T {
    unsafe {
        let cursor_save = (**curwin.as_ptr()).w_cursor;
        let mut ind_maxp_wk = ind_maxparen;
        let mut trypos: *mut pos_T;
        loop {
            trypos = findmatchlimit(
                ptr::null_mut(),
                c as u8 as c_int,
                FM_SKIPCOMM,
                ind_maxp_wk as i64,
            );
            if !trypos.is_null() {
                if (cin_skip2pos(&mut *trypos) as colnr_T) > (*trypos).col {
                    ind_maxp_wk = ind_maxparen - (cursor_save.lnum - (*trypos).lnum);
                    if ind_maxp_wk > 0 {
                        (**curwin.as_ptr()).w_cursor = *trypos;
                        (**curwin.as_ptr()).w_cursor.col = 0;
                        continue;
                    }
                    trypos = ptr::null_mut();
                } else {
                    indc_st().match_char_pos_copy = *trypos;
                    trypos = &raw mut indc_st().match_char_pos_copy;
                    (**curwin.as_ptr()).w_cursor = *trypos;
                    let trypos_wk = ind_find_start_CORS(None);
                    if !trypos_wk.is_null() {
                        ind_maxp_wk = ind_maxparen - (cursor_save.lnum - (*trypos_wk).lnum);
                        if ind_maxp_wk > 0 {
                            (**curwin.as_ptr()).w_cursor = *trypos_wk;
                            continue;
                        }
                        trypos = ptr::null_mut();
                    }
                }
            }
            break;
        }
        (**curwin.as_ptr()).w_cursor = cursor_save;
        trypos
    }
}
fn find_match_paren_after_brace(ind_maxparen: c_int) -> *mut pos_T {
    unsafe {
        let mut trypos = find_match_paren(ind_maxparen);
        if trypos.is_null() {
            return ptr::null_mut();
        }
        let tryposBrace = find_start_brace();
        if !tryposBrace.is_null()
            && (if (*trypos).lnum != (*tryposBrace).lnum {
                (*trypos).lnum < (*tryposBrace).lnum
            } else {
                (*trypos).col < (*tryposBrace).col
            })
        {
            trypos = ptr::null_mut();
        }
        trypos
    }
}
fn corr_ind_maxparen(startpos: &pos_T) -> c_int {
    unsafe {
        let n = startpos.lnum - (**curwin.as_ptr()).w_cursor.lnum;
        if n > 0 && n < (*g_st().curbuf).b_ind_maxparen / 2 {
            return (*g_st().curbuf).b_ind_maxparen - n;
        }
        (*g_st().curbuf).b_ind_maxparen
    }
}
fn find_last_paren(l: *const c_char, start: c_char, end: c_char) -> bool {
    unsafe {
        let mut retval = false;
        let mut open_count: c_int = 0;
        (**curwin.as_ptr()).w_cursor.col = 0;
        let mut i: usize = 0;
        while *l.add(i) != NUL {
            i = cin_skipcomment(l.add(i)).offset_from(l) as usize;
            if *l.add(i) == NUL {
                break;
            }
            i = skip_string(l.add(i)).offset_from(l) as usize;
            if *l.add(i) == start {
                open_count += 1;
            } else if *l.add(i) == end {
                if open_count > 0 {
                    open_count -= 1;
                } else {
                    (**curwin.as_ptr()).w_cursor.col = i as colnr_T;
                    retval = true;
                }
            }
            i += 1;
        }
        retval
    }
}
/// Parse 'cinoptions' and set the values in "buf".
pub fn parse_cino(buf: &mut buf_T) {
    unsafe {
        let mut fraction: c_int = 0;
        let sw = get_sw_value(buf);
        buf.b_ind_level = sw;
        buf.b_ind_open_imag = 0;
        buf.b_ind_no_brace = 0;
        buf.b_ind_first_open = 0;
        buf.b_ind_open_extra = 0;
        buf.b_ind_close_extra = 0;
        buf.b_ind_open_left_imag = 0;
        buf.b_ind_jump_label = -1;
        buf.b_ind_case = sw;
        buf.b_ind_case_code = sw;
        buf.b_ind_case_break = 0;
        buf.b_ind_scopedecl = sw;
        buf.b_ind_scopedecl_code = sw;
        buf.b_ind_param = sw;
        buf.b_ind_func_type = sw;
        buf.b_ind_cpp_baseclass = sw;
        buf.b_ind_continuation = sw;
        buf.b_ind_unclosed = sw * 2;
        buf.b_ind_unclosed2 = sw;
        buf.b_ind_unclosed_noignore = 0;
        buf.b_ind_unclosed_wrapped = 0;
        buf.b_ind_unclosed_whiteok = 0;
        buf.b_ind_matching_paren = 0;
        buf.b_ind_paren_prev = 0;
        buf.b_ind_comment = 0;
        buf.b_ind_in_comment = 3;
        buf.b_ind_in_comment2 = 0;
        buf.b_ind_maxparen = 20;
        buf.b_ind_maxcomment = 70;
        buf.b_ind_java = 0;
        buf.b_ind_js = 0;
        buf.b_ind_keep_case_label = 0;
        buf.b_ind_cpp_namespace = 0;
        buf.b_ind_if_for_while = 0;
        buf.b_ind_hash_comment = 0;
        buf.b_ind_cpp_extern_c = 0;
        buf.b_ind_pragma = 0;
        let mut p = buf.b_p_cino;
        while *p != NUL {
            let l = p;
            p = p.add(1);
            if *p == b'-' as c_char {
                p = p.add(1);
            }
            let digits_start = p;
            let mut n: i64 = getdigits_int(&raw mut p, true, 0) as i64;
            let mut divider: c_int = 0;
            if *p == b'.' as c_char {
                p = p.add(1);
                fraction = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(p)) as c_int;
                while ascii_isdigit(*p) {
                    p = p.add(1);
                    if divider != 0 {
                        divider *= 10;
                    } else {
                        divider = 10;
                    }
                }
            }
            if *p == b's' as c_char {
                if p == digits_start {
                    n = sw as i64;
                } else {
                    n *= sw as i64;
                    if divider != 0 {
                        n += (sw as i64 * fraction as i64 + (divider / 2) as i64) / divider as i64;
                    }
                }
                p = p.add(1);
            }
            if *l.add(1) == b'-' as c_char {
                n = -n;
            }
            let val = trim_to_int(n);
            match *l as u8 {
                b'>' => buf.b_ind_level = val,
                b'e' => buf.b_ind_open_imag = val,
                b'n' => buf.b_ind_no_brace = val,
                b'f' => buf.b_ind_first_open = val,
                b'{' => buf.b_ind_open_extra = val,
                b'}' => buf.b_ind_close_extra = val,
                b'^' => buf.b_ind_open_left_imag = val,
                b'L' => buf.b_ind_jump_label = val,
                b':' => buf.b_ind_case = val,
                b'=' => buf.b_ind_case_code = val,
                b'b' => buf.b_ind_case_break = val,
                b'p' => buf.b_ind_param = val,
                b't' => buf.b_ind_func_type = val,
                b'/' => buf.b_ind_comment = val,
                b'c' => buf.b_ind_in_comment = val,
                b'C' => buf.b_ind_in_comment2 = val,
                b'i' => buf.b_ind_cpp_baseclass = val,
                b'+' => buf.b_ind_continuation = val,
                b'(' => buf.b_ind_unclosed = val,
                b'u' => buf.b_ind_unclosed2 = val,
                b'U' => buf.b_ind_unclosed_noignore = val,
                b'W' => buf.b_ind_unclosed_wrapped = val,
                b'w' => buf.b_ind_unclosed_whiteok = val,
                b'm' => buf.b_ind_matching_paren = val,
                b'M' => buf.b_ind_paren_prev = val,
                b')' => buf.b_ind_maxparen = val,
                b'*' => buf.b_ind_maxcomment = val,
                b'g' => buf.b_ind_scopedecl = val,
                b'h' => buf.b_ind_scopedecl_code = val,
                b'j' => buf.b_ind_java = val,
                b'J' => buf.b_ind_js = val,
                b'l' => buf.b_ind_keep_case_label = val,
                b'#' => buf.b_ind_hash_comment = val,
                b'N' => buf.b_ind_cpp_namespace = val,
                b'k' => buf.b_ind_if_for_while = val,
                b'E' => buf.b_ind_cpp_extern_c = val,
                b'P' => buf.b_ind_pragma = val,
                _ => {}
            }
            if *p == b',' as c_char {
                p = p.add(1);
            }
        }
    }
}
/// Return the desired indent for C code.
/// Return -1 if the indent should be left alone (inside a raw string).
pub fn get_c_indent() -> c_int {
    unsafe {
        let mut amount: c_int;
        let mut scope_amount: c_int;
        let mut cur_amount: c_int = MAXCOL;
        let mut col: colnr_T = 0;
        let mut trypos: *mut pos_T;
        let mut comment_pos: *mut pos_T;
        let mut tryposBrace: *mut pos_T = ptr::null_mut();
        let mut tryposCopy: pos_T;
        let mut our_paren_pos: pos_T;
        let start: *mut c_char;
        let start_brace: c_int;
        let mut ourscope: linenr_T;
        let mut l: *const c_char = std::ptr::null();
        let mut look: *const c_char = std::ptr::null();
        let mut terminated: c_char;
        let mut lookfor: c_int;
        let mut whilelevel: c_int;
        let mut lnum: linenr_T;
        let mut n: c_int;
        let mut lookfor_break: bool;
        let mut lookfor_cpp_namespace = false;
        let mut cont_amount: c_int = 0;
        let mut added_to_amount: c_int = 0;
        let mut raw_string_start: linenr_T = 0;
        let mut cache_cpp_baseclass = cpp_baseclass_cache_T {
            found: 0,
            lpos: lpos_T {
                lnum: MAXLNUM,
                col: 0,
            },
        };
        let mut ind_continuation = (*g_st().curbuf).b_ind_continuation;
        let cur_curpos = (**curwin.as_ptr()).w_cursor;
        if cur_curpos.lnum == 1 {
            return 0;
        }
        let mut linecopy_bytes = crate::memory::cstr_to_cstring(ml_get(cur_curpos.lnum))
            .into_bytes_with_nul();
        let linecopy = linecopy_bytes.as_mut_ptr().cast::<c_char>();
        if (g_st().State & MODE_INSERT) != 0
            && ((**curwin.as_ptr()).w_cursor.col as usize) < crate::memory::cstr_len(linecopy)
            && *linecopy.add((**curwin.as_ptr()).w_cursor.col as usize) == b')' as c_char
        {
            *linecopy.add((**curwin.as_ptr()).w_cursor.col as usize) = NUL;
        }
        let theline = skipwhite(linecopy);
        (**curwin.as_ptr()).w_cursor.col = 0;
        let original_line_islabel = cin_islabel();
        comment_pos = ind_find_start_comment();
        if !comment_pos.is_null() {
            tryposCopy = *comment_pos;
            comment_pos = &raw mut tryposCopy;
        }
        trypos = find_start_rawstring((*g_st().curbuf).b_ind_maxcomment);
        'laterend: {
            if !trypos.is_null() && (comment_pos.is_null() || lt(*trypos, *comment_pos)) {
                amount = -1;
                break 'laterend;
            }
            'theend: {
                if *theline == b'#' as c_char
                    && (*linecopy == b'#' as c_char
                        || in_cinkeys(b'#' as c_int, b' ' as c_int, true))
                {
                    let directive = skipwhite(theline.add(1));
                    if (*g_st().curbuf).b_ind_pragma == 0 || !(crate::memory::cstr_prefix_bytes(directive, 6) == &b"pragma"[..6])
                    {
                        amount = (*g_st().curbuf).b_ind_hash_comment;
                        break 'theend;
                    }
                }
                if original_line_islabel
                    && (*g_st().curbuf).b_ind_js == 0
                    && (*g_st().curbuf).b_ind_jump_label < 0
                {
                    amount = 0;
                    break 'theend;
                }
                if cin_islinecomment(theline) {
                    let mut linecomment_pos = pos_T {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    };
                    trypos = find_line_comment();
                    if trypos.is_null() && (**curwin.as_ptr()).w_cursor.lnum > 1 {
                        linecomment_pos.col =
                            check_linecomment(ml_get((**curwin.as_ptr()).w_cursor.lnum - 1));
                        if linecomment_pos.col != MAXCOL {
                            trypos = &raw mut linecomment_pos;
                            (*trypos).lnum = (**curwin.as_ptr()).w_cursor.lnum - 1;
                        }
                    }
                    if !trypos.is_null() {
                        getvcol(
                            &mut **curwin.as_ptr(),
                            &mut *trypos,
                            Some(&mut col),
                            None,
                            None,
                            0,
                        );
                        amount = col;
                        break 'theend;
                    }
                }
                if !cin_iscomment(theline) && !comment_pos.is_null() {
                    let mut lead_start_len: c_int = 2;
                    let mut lead_middle_len: c_int = 1;
                    let mut lead_start: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
                    let mut lead_middle: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
                    let mut lead_end: [c_char; COM_MAX_LEN] = [0; COM_MAX_LEN];
                    let mut lead_end_len: c_int;
                    let mut start_align: c_int = 0;
                    let mut start_off: c_int = 0;
                    let mut done = false;
                    getvcol(
                        &mut **curwin.as_ptr(),
                        &mut *comment_pos,
                        Some(&mut col),
                        None,
                        None,
                        0,
                    );
                    amount = col;
                    lead_start[0] = NUL;
                    lead_middle[0] = NUL;
                    let mut p = (*g_st().curbuf).b_p_com;
                    while *p != NUL {
                        let mut align: c_int = 0;
                        let mut off: c_int = 0;
                        let mut what: c_int = 0;
                        while *p != NUL && *p != b':' as c_char {
                            if *p == COM_START || *p == COM_END || *p == COM_MIDDLE {
                                what = *p as u8 as c_int;
                                p = p.add(1);
                            } else if *p == COM_LEFT || *p == COM_RIGHT {
                                align = *p as u8 as c_int;
                                p = p.add(1);
                            } else if ascii_isdigit(*p) || *p == b'-' as c_char {
                                off = getdigits_int(&raw mut p, true, 0);
                            } else {
                                p = p.add(1);
                            }
                        }
                        if *p == b':' as c_char {
                            p = p.add(1);
                        }
                        lead_end_len = copy_option_part(
                            &raw mut p,
                            (&raw mut lead_end).cast(),
                            COM_MAX_LEN,
                            c",".as_ptr(),
                        ) as c_int;
                        if what == COM_START as u8 as c_int {
                            crate::memory::cstr_cpy((&raw mut lead_start).cast(), (&raw const lead_end).cast());
                            lead_start_len = lead_end_len;
                            start_off = off;
                            start_align = align;
                        } else if what == COM_MIDDLE as u8 as c_int {
                            crate::memory::cstr_cpy((&raw mut lead_middle).cast(), (&raw const lead_end).cast());
                            lead_middle_len = lead_end_len;
                        } else if what == COM_END as u8 as c_int {
                            if (crate::memory::cstr_prefix_bytes(theline, lead_middle_len as usize) == crate::memory::cstr_prefix_bytes((&raw const lead_middle).cast::<c_char>(), lead_middle_len as usize))
                                && (crate::memory::cstr_prefix_bytes(theline, lead_end_len as usize) != crate::memory::cstr_prefix_bytes((&raw const lead_end).cast::<c_char>(), lead_end_len as usize))
                            {
                                done = true;
                                if (**curwin.as_ptr()).w_cursor.lnum > 1 {
                                    look = skipwhite(ml_get((**curwin.as_ptr()).w_cursor.lnum - 1));
                                    if crate::memory::cstr_prefix_bytes(look, lead_start_len as usize) == crate::memory::cstr_prefix_bytes((&raw const lead_start).cast::<c_char>(), lead_start_len as usize)
                                    {
                                        amount = get_indent_lnum((**curwin.as_ptr()).w_cursor.lnum - 1);
                                    } else if crate::memory::cstr_prefix_bytes(look, lead_middle_len as usize) == crate::memory::cstr_prefix_bytes((&raw const lead_middle).cast::<c_char>(), lead_middle_len as usize)
                                    {
                                        amount = get_indent_lnum((**curwin.as_ptr()).w_cursor.lnum - 1);
                                        break;
                                    } else if crate::memory::cstr_prefix_bytes(ml_get((*comment_pos).lnum)
                                            .add((*comment_pos).col as usize), lead_start_len as usize) != crate::memory::cstr_prefix_bytes((&raw const lead_start).cast::<c_char>(), lead_start_len as usize)
                                    {
                                        continue;
                                    }
                                }
                                if start_off != 0 {
                                    amount += start_off;
                                } else if start_align == COM_RIGHT as u8 as c_int {
                                    amount += vim_strsize((&raw const lead_start).cast())
                                        - vim_strsize((&raw const lead_middle).cast());
                                }
                                break;
                            }
                            if (crate::memory::cstr_prefix_bytes(theline, lead_middle_len as usize) != crate::memory::cstr_prefix_bytes((&raw const lead_middle).cast::<c_char>(), lead_middle_len as usize))
                                && (crate::memory::cstr_prefix_bytes(theline, lead_end_len as usize) == crate::memory::cstr_prefix_bytes((&raw const lead_end).cast::<c_char>(), lead_end_len as usize))
                            {
                                amount = get_indent_lnum((**curwin.as_ptr()).w_cursor.lnum - 1);
                                if off != 0 {
                                    amount += off;
                                } else if align == COM_RIGHT as u8 as c_int {
                                    amount += vim_strsize((&raw const lead_start).cast())
                                        - vim_strsize((&raw const lead_middle).cast());
                                }
                                done = true;
                                break;
                            }
                        }
                    }
                    if done {
                        // skip
                    } else if *theline == b'*' as c_char {
                        amount += 1;
                    } else {
                        amount = -1;
                        lnum = cur_curpos.lnum - 1;
                        while lnum > (*comment_pos).lnum {
                            if linewhite(lnum) {
                                lnum -= 1;
                                continue;
                            }
                            amount = get_indent_lnum(lnum);
                            break;
                        }
                        if amount == -1 {
                            if (*g_st().curbuf).b_ind_in_comment2 == 0 {
                                start = ml_get((*comment_pos).lnum);
                                look = start.add((*comment_pos).col as usize + 2);
                                if *look != NUL {
                                    (*comment_pos).col =
                                        skipwhite(look).offset_from(start) as colnr_T;
                                }
                            }
                            getvcol(
                                &mut **curwin.as_ptr(),
                                &mut *comment_pos,
                                Some(&mut col),
                                None,
                                None,
                                0,
                            );
                            amount = col;
                            if (*g_st().curbuf).b_ind_in_comment2 != 0 || *look == NUL {
                                amount += (*g_st().curbuf).b_ind_in_comment;
                            }
                        }
                    }
                    break 'theend;
                }
                if *skipwhite(theline) == b']' as c_char && {
                    trypos = find_match_char(b'[' as c_char, (*g_st().curbuf).b_ind_maxparen);
                    !trypos.is_null()
                } {
                    amount = get_indent_lnum((*trypos).lnum);
                    break 'theend;
                }
                if ({
                    trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                    !trypos.is_null() && (*g_st().curbuf).b_ind_java == 0
                }) || ({
                    tryposBrace = find_start_brace();
                    !tryposBrace.is_null()
                }) || !trypos.is_null()
                {
                    if !trypos.is_null() && !tryposBrace.is_null() {
                        if if (*trypos).lnum != (*tryposBrace).lnum {
                            (*trypos).lnum < (*tryposBrace).lnum
                        } else {
                            (*trypos).col < (*tryposBrace).col
                        } {
                            trypos = ptr::null_mut();
                        } else {
                            tryposBrace = ptr::null_mut();
                        }
                    }
                    if !trypos.is_null() {
                        our_paren_pos = *trypos;
                        if *theline == b')' as c_char && (*g_st().curbuf).b_ind_paren_prev != 0 {
                            amount = get_indent_lnum((**curwin.as_ptr()).w_cursor.lnum - 1);
                        } else {
                            amount = -1;
                            lnum = cur_curpos.lnum - 1;
                            while lnum > our_paren_pos.lnum {
                                l = skipwhite(ml_get(lnum));
                                if cin_nocode(l) {
                                    lnum -= 1;
                                    continue;
                                }
                                if cin_ispreproc_cont(&raw mut l, &mut lnum, &mut amount) {
                                    lnum -= 1;
                                    continue;
                                }
                                (**curwin.as_ptr()).w_cursor.lnum = lnum;
                                trypos = ind_find_start_CORS(None);
                                if !trypos.is_null() {
                                    lnum = (*trypos).lnum;
                                    continue;
                                }
                                trypos = find_match_paren(corr_ind_maxparen(&cur_curpos));
                                if !trypos.is_null()
                                    && (*trypos).lnum == our_paren_pos.lnum
                                    && (*trypos).col == our_paren_pos.col
                                {
                                    amount = get_indent_lnum(lnum);
                                    if *theline == b')' as c_char {
                                        if our_paren_pos.lnum != lnum && cur_amount > amount {
                                            cur_amount = amount;
                                        }
                                        amount = -1;
                                    }
                                    break;
                                }
                                lnum -= 1;
                            }
                        }
                        if amount == -1 {
                            let mut ignore_paren_col: c_int = 0;
                            let mut is_if_for_while: c_int = 0;
                            if (*g_st().curbuf).b_ind_if_for_while != 0 {
                                let cursor_save = (**curwin.as_ptr()).w_cursor;
                                let mut outermost: pos_T;
                                trypos = &raw mut our_paren_pos;
                                loop {
                                    outermost = *trypos;
                                    (**curwin.as_ptr()).w_cursor.lnum = outermost.lnum;
                                    (**curwin.as_ptr()).w_cursor.col = outermost.col;
                                    trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                                    if trypos.is_null() || (*trypos).lnum != outermost.lnum {
                                        break;
                                    }
                                }
                                (**curwin.as_ptr()).w_cursor = cursor_save;
                                let line = ml_get(outermost.lnum);
                                is_if_for_while =
                                    cin_is_if_for_while_before_offset(line, &mut outermost.col);
                            }
                            amount = skip_label(our_paren_pos.lnum, &raw mut look);
                            look = skipwhite(look);
                            if *look == b'(' as c_char {
                                let save_lnum = (**curwin.as_ptr()).w_cursor.lnum;
                                (**curwin.as_ptr()).w_cursor.lnum = our_paren_pos.lnum;
                                let line: *const c_char = get_cursor_line_ptr();
                                let look_col = look.offset_from(line) as colnr_T;
                                (**curwin.as_ptr()).w_cursor.col = look_col + 1;
                                trypos = findmatchlimit(
                                    ptr::null_mut(),
                                    b')' as c_int,
                                    FM_SKIPCOMM,
                                    (*g_st().curbuf).b_ind_maxparen as i64,
                                );
                                if !trypos.is_null()
                                    && (*trypos).lnum == our_paren_pos.lnum
                                    && (*trypos).col < our_paren_pos.col
                                {
                                    ignore_paren_col = (*trypos).col + 1;
                                }
                                (**curwin.as_ptr()).w_cursor.lnum = save_lnum;
                                look = ml_get(our_paren_pos.lnum).add(look_col as usize);
                            }
                            if *theline == b')' as c_char
                                || ((*g_st().curbuf).b_ind_unclosed == 0 && is_if_for_while == 0)
                                || ((*g_st().curbuf).b_ind_unclosed_noignore == 0
                                    && *look == b'(' as c_char
                                    && ignore_paren_col == 0)
                            {
                                if *theline != b')' as c_char {
                                    cur_amount = MAXCOL;
                                    l = ml_get(our_paren_pos.lnum);
                                    if (*g_st().curbuf).b_ind_unclosed_wrapped != 0
                                        && cin_ends_in(l, c"(".as_ptr())
                                    {
                                        n = 1;
                                        col = 0;
                                        while col < our_paren_pos.col {
                                            match *l.add(col as usize) as u8 {
                                                b'(' | b'{' => n += 1,
                                                b')' | b'}' if n > 1 => n -= 1,
                                                _ => {}
                                            }
                                            col += 1;
                                        }
                                        our_paren_pos.col = 0;
                                        amount += n * (*g_st().curbuf).b_ind_unclosed_wrapped;
                                    } else if (*g_st().curbuf).b_ind_unclosed_whiteok != 0 {
                                        our_paren_pos.col += 1;
                                    } else {
                                        col = our_paren_pos.col + 1;
                                        while ascii_iswhite(*l.add(col as usize)) {
                                            col += 1;
                                        }
                                        if *l.add(col as usize) != NUL {
                                            our_paren_pos.col = col;
                                        } else {
                                            our_paren_pos.col += 1;
                                        }
                                    }
                                }
                                if our_paren_pos.col > 0 {
                                    getvcol(
                                        &mut **curwin.as_ptr(),
                                        &mut our_paren_pos,
                                        Some(&mut col),
                                        None,
                                        None,
                                        0,
                                    );
                                    if cur_amount > col {
                                        cur_amount = col;
                                    }
                                }
                            }
                            if *theline == b')' as c_char && (*g_st().curbuf).b_ind_matching_paren != 0 {
                                // Line up with the start of the matching paren line.
                            } else if ((*g_st().curbuf).b_ind_unclosed == 0 && is_if_for_while == 0)
                                || ((*g_st().curbuf).b_ind_unclosed_noignore == 0
                                    && *look == b'(' as c_char
                                    && ignore_paren_col == 0)
                            {
                                if cur_amount != MAXCOL {
                                    amount = cur_amount;
                                }
                            } else {
                                col = our_paren_pos.col;
                                while our_paren_pos.col > ignore_paren_col {
                                    our_paren_pos.col -= 1;
                                    match *ml_get_pos(&our_paren_pos) as u8 {
                                        b'(' => {
                                            amount += (*g_st().curbuf).b_ind_unclosed2;
                                            col = our_paren_pos.col;
                                        }
                                        b')' => {
                                            amount -= (*g_st().curbuf).b_ind_unclosed2;
                                            col = MAXCOL;
                                        }
                                        _ => {}
                                    }
                                }
                                if col == MAXCOL {
                                    amount += (*g_st().curbuf).b_ind_unclosed;
                                } else {
                                    (**curwin.as_ptr()).w_cursor.lnum = our_paren_pos.lnum;
                                    (**curwin.as_ptr()).w_cursor.col = col;
                                    if !find_match_paren_after_brace((*g_st().curbuf).b_ind_maxparen)
                                        .is_null()
                                    {
                                        amount += (*g_st().curbuf).b_ind_unclosed2;
                                    } else if is_if_for_while != 0 {
                                        amount += (*g_st().curbuf).b_ind_if_for_while;
                                    } else {
                                        amount += (*g_st().curbuf).b_ind_unclosed;
                                    }
                                }
                                if cur_amount < amount {
                                    amount = cur_amount;
                                }
                            }
                        }
                        if cin_iscomment(theline) {
                            amount += (*g_st().curbuf).b_ind_comment;
                        }
                    } else {
                        tryposCopy = *tryposBrace;
                        tryposBrace = &raw mut tryposCopy;
                        trypos = tryposBrace;
                        ourscope = (*trypos).lnum;
                        start = ml_get(ourscope);
                        look = skipwhite(start);
                        if *look == b'{' as c_char {
                            getvcol(
                                &mut **curwin.as_ptr(),
                                &mut *trypos,
                                Some(&mut col),
                                None,
                                None,
                                0,
                            );
                            amount = col;
                            if *start == b'{' as c_char {
                                start_brace = BRACE_IN_COL0;
                            } else {
                                start_brace = BRACE_AT_START;
                            }
                        } else {
                            (**curwin.as_ptr()).w_cursor.lnum = ourscope;
                            lnum = ourscope;
                            if find_last_paren(start, b'(' as c_char, b')' as c_char) {
                                trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                                if !trypos.is_null() {
                                    lnum = (*trypos).lnum;
                                }
                            }
                            if ((*g_st().curbuf).b_ind_js != 0 || (*g_st().curbuf).b_ind_keep_case_label != 0)
                                && cin_iscase(skipwhite(get_cursor_line_ptr()), false)
                            {
                                amount = get_indent();
                            } else if (*g_st().curbuf).b_ind_js != 0 {
                                amount = get_indent_lnum(lnum);
                            } else {
                                amount = skip_label(lnum, &raw mut l);
                            }
                            start_brace = BRACE_AT_END;
                        }
                        let mut js_cur_has_key = if (*g_st().curbuf).b_ind_js != 0 {
                            cin_has_js_key(theline)
                        } else {
                            false
                        };
                        if *theline == b'}' as c_char {
                            amount += (*g_st().curbuf).b_ind_close_extra;
                        } else {
                            lookfor = LOOKFOR_INITIAL;
                            if cin_iselse(theline) {
                                lookfor = LOOKFOR_IF;
                            } else if cin_iswhileofdo(theline, cur_curpos.lnum) {
                                lookfor = LOOKFOR_DO;
                            }
                            if lookfor != LOOKFOR_INITIAL {
                                (**curwin.as_ptr()).w_cursor.lnum = cur_curpos.lnum;
                                if find_match(lookfor, ourscope) {
                                    amount = get_indent();
                                    break 'theend;
                                }
                            }
                            if start_brace == BRACE_IN_COL0 {
                                amount = (*g_st().curbuf).b_ind_open_left_imag;
                                lookfor_cpp_namespace = true;
                            } else if start_brace == BRACE_AT_START && lookfor_cpp_namespace {
                                lookfor_cpp_namespace = true;
                            } else if start_brace == BRACE_AT_END {
                                amount += (*g_st().curbuf).b_ind_open_imag;
                                l = skipwhite(get_cursor_line_ptr());
                                if cin_is_cpp_namespace(l) {
                                    amount += (*g_st().curbuf).b_ind_cpp_namespace;
                                } else if cin_is_cpp_extern_c(l) {
                                    amount += (*g_st().curbuf).b_ind_cpp_extern_c;
                                }
                            } else {
                                amount -= (*g_st().curbuf).b_ind_open_extra;
                                if amount < 0 {
                                    amount = 0;
                                }
                            }
                            lookfor_break = false;
                            if cin_iscase(theline, false) {
                                lookfor = LOOKFOR_CASE;
                                amount += (*g_st().curbuf).b_ind_case;
                            } else if cin_isscopedecl(theline) {
                                lookfor = LOOKFOR_SCOPEDECL;
                                amount += (*g_st().curbuf).b_ind_scopedecl;
                            } else {
                                if (*g_st().curbuf).b_ind_case_break != 0 && cin_isbreak(theline) {
                                    lookfor_break = true;
                                }
                                lookfor = LOOKFOR_INITIAL;
                                amount += (*g_st().curbuf).b_ind_level;
                            }
                            scope_amount = amount;
                            whilelevel = 0;
                            (**curwin.as_ptr()).w_cursor = cur_curpos;
                            'scope_loop: loop {
                                (**curwin.as_ptr()).w_cursor.lnum -= 1;
                                (**curwin.as_ptr()).w_cursor.col = 0;
                                if (**curwin.as_ptr()).w_cursor.lnum <= ourscope {
                                    if lookfor == LOOKFOR_ENUM_OR_INIT {
                                        if (**curwin.as_ptr()).w_cursor.lnum == 0
                                            || (**curwin.as_ptr()).w_cursor.lnum
                                                < ourscope - (*g_st().curbuf).b_ind_maxparen
                                        {
                                            if cont_amount > 0 {
                                                amount = cont_amount;
                                            } else if (*g_st().curbuf).b_ind_js == 0 {
                                                amount += ind_continuation;
                                            }
                                            break;
                                        }
                                        trypos = ind_find_start_CORS(None);
                                        if !trypos.is_null() {
                                            (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                                            (**curwin.as_ptr()).w_cursor.col = 0;
                                            continue;
                                        }
                                        l = get_cursor_line_ptr();
                                        if cin_ispreproc_cont(
                                            &raw mut l,
                                            &mut (**curwin.as_ptr()).w_cursor.lnum,
                                            &mut amount,
                                        ) {
                                            continue;
                                        }
                                        if cin_nocode(l) {
                                            continue;
                                        }
                                        terminated = cin_isterminated(l, false, true);
                                        if start_brace != BRACE_IN_COL0
                                            || !cin_isfuncdecl(
                                                &raw mut l,
                                                (**curwin.as_ptr()).w_cursor.lnum,
                                                0,
                                            )
                                        {
                                            if terminated == b',' as c_char {
                                                break;
                                            }
                                            if terminated != b';' as c_char && cin_isinit() {
                                                break;
                                            }
                                            if terminated == 0 || terminated == b'{' as c_char {
                                                continue;
                                            }
                                        }
                                        if terminated != b';' as c_char {
                                            trypos = ptr::null_mut();
                                            if find_last_paren(l, b'(' as c_char, b')' as c_char) {
                                                trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                                            }
                                            if trypos.is_null()
                                                && find_last_paren(
                                                    l,
                                                    b'{' as c_char,
                                                    b'}' as c_char,
                                                )
                                            {
                                                trypos = find_start_brace();
                                            }
                                            if !trypos.is_null() {
                                                (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                                                (**curwin.as_ptr()).w_cursor.col = 0;
                                                continue;
                                            }
                                        }
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                    } else if lookfor == LOOKFOR_UNTERM {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                    } else {
                                        if lookfor != LOOKFOR_TERM
                                            && lookfor != LOOKFOR_CPP_BASECLASS
                                            && lookfor != LOOKFOR_COMMA
                                        {
                                            amount = scope_amount;
                                            if *theline == b'{' as c_char {
                                                amount += (*g_st().curbuf).b_ind_open_extra;
                                                added_to_amount = (*g_st().curbuf).b_ind_open_extra;
                                            }
                                        }
                                        if lookfor_cpp_namespace {
                                            if (**curwin.as_ptr()).w_cursor.lnum == ourscope {
                                                continue;
                                            }
                                            if (**curwin.as_ptr()).w_cursor.lnum == 0
                                                || (**curwin.as_ptr()).w_cursor.lnum
                                                    < ourscope - FIND_NAMESPACE_LIM
                                            {
                                                break;
                                            }
                                            trypos = ind_find_start_CORS(None);
                                            if !trypos.is_null() {
                                                (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                                                (**curwin.as_ptr()).w_cursor.col = 0;
                                                continue;
                                            }
                                            l = get_cursor_line_ptr();
                                            if cin_ispreproc_cont(
                                                &raw mut l,
                                                &mut (**curwin.as_ptr()).w_cursor.lnum,
                                                &mut amount,
                                            ) {
                                                continue;
                                            }
                                            if cin_is_cpp_namespace(l) {
                                                amount +=
                                                    (*g_st().curbuf).b_ind_cpp_namespace - added_to_amount;
                                                break;
                                            } else if cin_is_cpp_extern_c(l) {
                                                amount +=
                                                    (*g_st().curbuf).b_ind_cpp_extern_c - added_to_amount;
                                                break;
                                            }
                                            if cin_nocode(l) {
                                                continue;
                                            }
                                        }
                                    }
                                    break;
                                }
                                trypos = ind_find_start_CORS(Some(&mut raw_string_start));
                                if !trypos.is_null() {
                                    (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                                    (**curwin.as_ptr()).w_cursor.col = 0;
                                    continue;
                                }
                                l = get_cursor_line_ptr();
                                let mut iscase = cin_iscase(l, false);
                                if iscase || cin_isscopedecl(l) {
                                    if lookfor == LOOKFOR_CPP_BASECLASS {
                                        break;
                                    }
                                    if whilelevel > 0 {
                                        continue;
                                    }
                                    if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT
                                    {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    }
                                    if (iscase && lookfor == LOOKFOR_CASE)
                                        || (iscase && lookfor_break)
                                        || (!iscase && lookfor == LOOKFOR_SCOPEDECL)
                                    {
                                        trypos = find_start_brace();
                                        if trypos.is_null() || (*trypos).lnum == ourscope {
                                            amount = get_indent();
                                            break;
                                        }
                                        continue;
                                    }
                                    n = get_indent_nolabel((**curwin.as_ptr()).w_cursor.lnum);
                                    if lookfor == LOOKFOR_TERM {
                                        if n != 0 {
                                            amount = n;
                                        }
                                        if !lookfor_break {
                                            break;
                                        }
                                    }
                                    if n != 0 {
                                        amount = n;
                                        l = after_label(get_cursor_line_ptr());
                                        if !l.is_null() && cin_is_cinword(l) {
                                            if *theline == b'{' as c_char {
                                                amount += (*g_st().curbuf).b_ind_open_extra;
                                            } else {
                                                amount += (*g_st().curbuf).b_ind_level
                                                    + (*g_st().curbuf).b_ind_no_brace;
                                            }
                                        }
                                        break;
                                    }
                                    scope_amount = get_indent()
                                        + if iscase {
                                            (*g_st().curbuf).b_ind_case_code
                                        } else {
                                            (*g_st().curbuf).b_ind_scopedecl_code
                                        };
                                    lookfor = if (*g_st().curbuf).b_ind_case_break != 0 {
                                        LOOKFOR_NOBREAK
                                    } else {
                                        LOOKFOR_ANY
                                    };
                                    continue;
                                }
                                if lookfor == LOOKFOR_CASE || lookfor == LOOKFOR_SCOPEDECL {
                                    if find_last_paren(l, b'{' as c_char, b'}' as c_char) {
                                        trypos = find_start_brace();
                                        if !trypos.is_null() {
                                            (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                                            (**curwin.as_ptr()).w_cursor.col = 0;
                                        }
                                    }
                                    continue;
                                }
                                if (*g_st().curbuf).b_ind_js == 0 && cin_islabel() {
                                    l = after_label(get_cursor_line_ptr());
                                    if l.is_null() || cin_nocode(l) {
                                        continue;
                                    }
                                }
                                l = get_cursor_line_ptr();
                                if cin_ispreproc_cont(
                                    &raw mut l,
                                    &mut (**curwin.as_ptr()).w_cursor.lnum,
                                    &mut amount,
                                ) || cin_nocode(l)
                                {
                                    continue;
                                }
                                n = 0;
                                if lookfor != LOOKFOR_TERM && (*g_st().curbuf).b_ind_cpp_baseclass > 0 {
                                    n = c_int::from(cin_is_cpp_baseclass(
                                        &mut cache_cpp_baseclass,
                                    ));
                                    l = get_cursor_line_ptr();
                                }
                                if n != 0 {
                                    if lookfor == LOOKFOR_UNTERM {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                    } else if *theline == b'{' as c_char {
                                        lookfor = LOOKFOR_UNTERM;
                                        ind_continuation = 0;
                                        continue;
                                    } else {
                                        amount = get_baseclass_amount(cache_cpp_baseclass.lpos.col);
                                    }
                                    break;
                                } else if lookfor == LOOKFOR_CPP_BASECLASS {
                                    if cin_isterminated(l, true, false) != 0 {
                                        break;
                                    } else {
                                        continue;
                                    }
                                }
                                terminated = cin_isterminated(l, false, true);
                                if js_cur_has_key {
                                    js_cur_has_key = false;
                                    if (*g_st().curbuf).b_ind_js != 0 && terminated == b',' as c_char {
                                        lookfor = LOOKFOR_JS_KEY;
                                    }
                                }
                                if lookfor == LOOKFOR_JS_KEY && cin_has_js_key(l) {
                                    amount = get_indent();
                                    break;
                                }
                                if lookfor == LOOKFOR_COMMA {
                                    if !tryposBrace.is_null()
                                        && (*tryposBrace).lnum >= (**curwin.as_ptr()).w_cursor.lnum
                                    {
                                        break;
                                    }
                                    if terminated == b',' as c_char {
                                        break;
                                    }
                                    amount = get_indent();
                                    if (**curwin.as_ptr()).w_cursor.lnum - 1 == ourscope {
                                        break;
                                    }
                                }
                                if terminated == 0
                                    || (lookfor != LOOKFOR_UNTERM && terminated == b',' as c_char)
                                {
                                    if lookfor != LOOKFOR_ENUM_OR_INIT
                                        && (*skipwhite(l) == b'[' as c_char
                                            || (*l != NUL
                                                && *l.add(crate::memory::cstr_len(l) - 1) == b'[' as c_char))
                                    {
                                        amount += ind_continuation;
                                    }
                                    let _ = find_last_paren(l, b'(' as c_char, b')' as c_char);
                                    trypos =
                                        find_match_paren(corr_ind_maxparen(&cur_curpos));
                                    if !trypos.is_null()
                                        && ((*trypos).lnum < (*tryposBrace).lnum
                                            || ((*trypos).lnum == (*tryposBrace).lnum
                                                && (*trypos).col < (*tryposBrace).col))
                                    {
                                        trypos = ptr::null_mut();
                                    }
                                    l = get_cursor_line_ptr();
                                    if trypos.is_null() && terminated == b',' as c_char {
                                        if find_last_paren(l, b'{' as c_char, b'}' as c_char) {
                                            trypos = find_start_brace();
                                        }
                                        l = get_cursor_line_ptr();
                                    }
                                    if !trypos.is_null() {
                                        (**curwin.as_ptr()).w_cursor = *trypos;
                                        l = get_cursor_line_ptr();
                                        if cin_iscase(l, false) || cin_isscopedecl(l) {
                                            (**curwin.as_ptr()).w_cursor.lnum += 1;
                                            (**curwin.as_ptr()).w_cursor.col = 0;
                                            continue;
                                        }
                                    }
                                    if terminated == b',' as c_char {
                                        while (**curwin.as_ptr()).w_cursor.lnum > 1 {
                                            l = ml_get((**curwin.as_ptr()).w_cursor.lnum - 1);
                                            if *l == NUL || *l.add(crate::memory::cstr_len(l) - 1) != b'\\' as c_char
                                            {
                                                break;
                                            }
                                            (**curwin.as_ptr()).w_cursor.lnum -= 1;
                                            (**curwin.as_ptr()).w_cursor.col = 0;
                                        }
                                        l = get_cursor_line_ptr();
                                    }
                                    if (*g_st().curbuf).b_ind_js != 0 {
                                        cur_amount = get_indent();
                                    } else {
                                        cur_amount =
                                            skip_label((**curwin.as_ptr()).w_cursor.lnum, &raw mut l);
                                    }
                                    if terminated != b',' as c_char
                                        && lookfor != LOOKFOR_TERM
                                        && *theline == b'{' as c_char
                                    {
                                        amount = cur_amount;
                                        if *skipwhite(l) != b'{' as c_char {
                                            amount += (*g_st().curbuf).b_ind_open_extra;
                                        }
                                        if (*g_st().curbuf).b_ind_cpp_baseclass != 0
                                            && (*g_st().curbuf).b_ind_js == 0
                                        {
                                            lookfor = LOOKFOR_CPP_BASECLASS;
                                            continue;
                                        }
                                        break;
                                    }
                                    if cin_is_cinword(l) || cin_iselse(skipwhite(l)) {
                                        if lookfor == LOOKFOR_UNTERM
                                            || lookfor == LOOKFOR_ENUM_OR_INIT
                                        {
                                            if cont_amount > 0 {
                                                amount = cont_amount;
                                            } else {
                                                amount += ind_continuation;
                                            }
                                            break;
                                        }
                                        amount = cur_amount;
                                        if *theline == b'{' as c_char {
                                            amount += (*g_st().curbuf).b_ind_open_extra;
                                        }
                                        if lookfor != LOOKFOR_TERM {
                                            amount +=
                                                (*g_st().curbuf).b_ind_level + (*g_st().curbuf).b_ind_no_brace;
                                            break;
                                        }
                                        l = skipwhite(get_cursor_line_ptr());
                                        if cin_isdo(l) {
                                            if whilelevel == 0 {
                                                break;
                                            }
                                            whilelevel -= 1;
                                        }
                                        if cin_iselse(l) && whilelevel == 0 {
                                            if *l == b'}' as c_char {
                                                (**curwin.as_ptr()).w_cursor.col =
                                                    l.offset_from(get_cursor_line_ptr()) as colnr_T
                                                        + 1;
                                            }
                                            trypos = find_start_brace();
                                            if trypos.is_null()
                                                || !find_match(LOOKFOR_IF, (*trypos).lnum)
                                            {
                                                break;
                                            }
                                        }
                                    } else if lookfor == LOOKFOR_UNTERM {
                                        if terminated == b',' as c_char {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    } else if lookfor == LOOKFOR_ENUM_OR_INIT {
                                        if terminated == b',' as c_char {
                                            if (*g_st().curbuf).b_ind_cpp_baseclass == 0 {
                                                break;
                                            }
                                            lookfor = LOOKFOR_CPP_BASECLASS;
                                            continue;
                                        }
                                        if amount > cur_amount {
                                            amount = cur_amount;
                                        }
                                    } else {
                                        l = get_cursor_line_ptr();
                                        amount = cur_amount;
                                        n = crate::memory::cstr_len(l) as c_int;
                                        if (*g_st().curbuf).b_ind_js != 0
                                            && terminated == b',' as c_char
                                            && (*skipwhite(l) == b']' as c_char
                                                || (n >= 2
                                                    && *l.add((n - 2) as usize) == b']' as c_char))
                                        {
                                            break;
                                        }
                                        if lookfor == LOOKFOR_INITIAL
                                            && terminated == b',' as c_char
                                        {
                                            if (*g_st().curbuf).b_ind_js != 0 {
                                                if cin_iscomment(skipwhite(l)) {
                                                    break;
                                                }
                                                lookfor = LOOKFOR_COMMA;
                                                trypos = find_match_char(
                                                    b'[' as c_char,
                                                    (*g_st().curbuf).b_ind_maxparen,
                                                );
                                                if !trypos.is_null() {
                                                    if (*trypos).lnum == (**curwin.as_ptr()).w_cursor.lnum - 1
                                                    {
                                                        break;
                                                    }
                                                    ourscope = (*trypos).lnum;
                                                }
                                            } else {
                                                lookfor = LOOKFOR_ENUM_OR_INIT;
                                                cont_amount = cin_first_id_amount();
                                            }
                                        } else {
                                            if lookfor == LOOKFOR_INITIAL
                                                && *l != NUL
                                                && *l.add(crate::memory::cstr_len(l) - 1) == b'\\' as c_char
                                            {
                                                cont_amount =
                                                    cin_get_equal_amount((**curwin.as_ptr()).w_cursor.lnum);
                                            }
                                            if lookfor != LOOKFOR_TERM
                                                && lookfor != LOOKFOR_JS_KEY
                                                && lookfor != LOOKFOR_COMMA
                                                && raw_string_start != (**curwin.as_ptr()).w_cursor.lnum
                                            {
                                                lookfor = LOOKFOR_UNTERM;
                                            }
                                        }
                                    }
                                } else if cin_iswhileofdo_end(terminated as u8 as c_int) {
                                    if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT
                                    {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    }
                                    if whilelevel == 0 {
                                        lookfor = LOOKFOR_TERM;
                                        amount = get_indent();
                                        if *theline == b'{' as c_char {
                                            amount += (*g_st().curbuf).b_ind_open_extra;
                                        }
                                    }
                                    whilelevel += 1;
                                } else {
                                    if lookfor == LOOKFOR_NOBREAK
                                        && cin_isbreak(skipwhite(get_cursor_line_ptr()))
                                    {
                                        lookfor = LOOKFOR_ANY;
                                        continue;
                                    }
                                    if whilelevel > 0 {
                                        l = cin_skipcomment(get_cursor_line_ptr());
                                        if cin_isdo(l) {
                                            amount = get_indent();
                                            whilelevel -= 1;
                                            continue;
                                        }
                                    }
                                    if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT
                                    {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    }
                                    if lookfor == LOOKFOR_TERM {
                                        if !lookfor_break && whilelevel == 0 {
                                            break;
                                        }
                                    } else {
                                        loop {
                                            l = get_cursor_line_ptr();
                                            if find_last_paren(l, b'(' as c_char, b')' as c_char) {
                                                trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                                                if !trypos.is_null() {
                                                    (**curwin.as_ptr()).w_cursor = *trypos;
                                                    l = get_cursor_line_ptr();
                                                    if cin_iscase(l, false) || cin_isscopedecl(l) {
                                                        (**curwin.as_ptr()).w_cursor.lnum += 1;
                                                        (**curwin.as_ptr()).w_cursor.col = 0;
                                                        continue 'scope_loop;
                                                    }
                                                }
                                            }
                                            iscase = (*g_st().curbuf).b_ind_keep_case_label != 0
                                                && cin_iscase(l, false);
                                            amount =
                                                skip_label((**curwin.as_ptr()).w_cursor.lnum, &raw mut l);
                                            if *theline == b'{' as c_char {
                                                amount += (*g_st().curbuf).b_ind_open_extra;
                                            }
                                            l = skipwhite(l);
                                            if *l == b'{' as c_char {
                                                amount -= (*g_st().curbuf).b_ind_open_extra;
                                            }
                                            lookfor =
                                                if iscase { LOOKFOR_ANY } else { LOOKFOR_TERM };
                                            if lookfor == LOOKFOR_TERM
                                                && *l != b'}' as c_char
                                                && cin_iselse(l)
                                                && whilelevel == 0
                                            {
                                                trypos = find_start_brace();
                                                if trypos.is_null()
                                                    || !find_match(LOOKFOR_IF, (*trypos).lnum)
                                                {
                                                    break 'scope_loop;
                                                }
                                                continue 'scope_loop;
                                            }
                                            l = get_cursor_line_ptr();
                                            if find_last_paren(l, b'{' as c_char, b'}' as c_char) {
                                                trypos = find_start_brace();
                                                if !trypos.is_null() {
                                                    (**curwin.as_ptr()).w_cursor = *trypos;
                                                    l = cin_skipcomment(get_cursor_line_ptr());
                                                    if *l == b'}' as c_char || !cin_iselse(l) {
                                                        continue;
                                                    }
                                                    (**curwin.as_ptr()).w_cursor.lnum += 1;
                                                    (**curwin.as_ptr()).w_cursor.col = 0;
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if cin_iscomment(theline) {
                        amount += (*g_st().curbuf).b_ind_comment;
                    }
                    if (*g_st().curbuf).b_ind_jump_label > 0 && original_line_islabel {
                        amount -= (*g_st().curbuf).b_ind_jump_label;
                    }
                    break 'theend;
                }
                if *theline == b'{' as c_char {
                    amount = (*g_st().curbuf).b_ind_first_open;
                    break 'theend;
                }
                if cur_curpos.lnum < (*g_st().curbuf).b_ml.ml_line_count
                    && !cin_nocode(theline)
                    && vim_strchr(theline, b'{' as c_int).is_null()
                    && vim_strchr(theline, b'}' as c_int).is_null()
                    && !cin_ends_in(theline, c":".as_ptr())
                    && !cin_ends_in(theline, c",".as_ptr())
                    && cin_isfuncdecl(ptr::null_mut(), cur_curpos.lnum + 1, cur_curpos.lnum + 1)
                    && cin_isterminated(theline, false, true) == 0
                {
                    amount = (*g_st().curbuf).b_ind_func_type;
                    break 'theend;
                }
                amount = 0;
                (**curwin.as_ptr()).w_cursor = cur_curpos;
                while (**curwin.as_ptr()).w_cursor.lnum > 1 {
                    (**curwin.as_ptr()).w_cursor.lnum -= 1;
                    (**curwin.as_ptr()).w_cursor.col = 0;
                    l = get_cursor_line_ptr();
                    trypos = ind_find_start_CORS(None);
                    if !trypos.is_null() {
                        (**curwin.as_ptr()).w_cursor.lnum = (*trypos).lnum + 1;
                        (**curwin.as_ptr()).w_cursor.col = 0;
                        continue;
                    }
                    n = 0;
                    if (*g_st().curbuf).b_ind_cpp_baseclass != 0 {
                        n = c_int::from(cin_is_cpp_baseclass(&mut cache_cpp_baseclass));
                        l = get_cursor_line_ptr();
                    }
                    if n != 0 {
                        amount = get_baseclass_amount(cache_cpp_baseclass.lpos.col);
                        break;
                    }
                    if cin_ispreproc_cont(
                        &raw mut l,
                        &mut (**curwin.as_ptr()).w_cursor.lnum,
                        &mut amount,
                    ) {
                        continue;
                    }
                    if cin_nocode(l) {
                        continue;
                    }
                    if cin_ends_in(l, c",".as_ptr())
                        || (*l != NUL && {
                            n = *l.add(crate::memory::cstr_len(l) - 1) as u8 as c_int;
                            n == b'\\' as c_int
                        })
                    {
                        if find_last_paren(l, b'(' as c_char, b')' as c_char) {
                            trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                            if !trypos.is_null() {
                                (**curwin.as_ptr()).w_cursor = *trypos;
                            }
                        }
                        while n == 0 && (**curwin.as_ptr()).w_cursor.lnum > 1 {
                            l = ml_get((**curwin.as_ptr()).w_cursor.lnum - 1);
                            if *l == NUL || *l.add(crate::memory::cstr_len(l) - 1) != b'\\' as c_char {
                                break;
                            }
                            (**curwin.as_ptr()).w_cursor.lnum -= 1;
                            (**curwin.as_ptr()).w_cursor.col = 0;
                        }
                        amount = get_indent();
                        if amount == 0 {
                            amount = cin_first_id_amount();
                        }
                        if amount == 0 {
                            amount = ind_continuation;
                        }
                        break;
                    }
                    if cin_isfuncdecl(ptr::null_mut(), cur_curpos.lnum, 0) {
                        break;
                    }
                    l = get_cursor_line_ptr();
                    if *skipwhite(l) == b'}' as c_char {
                        break;
                    }
                    if cin_ends_in(l, c"};".as_ptr()) {
                        break;
                    }
                    if cin_ends_in(l, c"[".as_ptr()) {
                        amount = get_indent() + ind_continuation;
                        break;
                    }
                    look = skipwhite(l);
                    if *look == b';' as c_char && cin_nocode(look.add(1)) {
                        let curpos_save = (**curwin.as_ptr()).w_cursor;
                        while (**curwin.as_ptr()).w_cursor.lnum > 1 {
                            (**curwin.as_ptr()).w_cursor.lnum -= 1;
                            look = ml_get((**curwin.as_ptr()).w_cursor.lnum);
                            if !(cin_nocode(look)
                                || cin_ispreproc_cont(
                                    &raw mut look,
                                    &mut (**curwin.as_ptr()).w_cursor.lnum,
                                    &mut amount,
                                ))
                            {
                                break;
                            }
                        }
                        if (**curwin.as_ptr()).w_cursor.lnum > 0 && cin_ends_in(look, c"}".as_ptr()) {
                            break;
                        }
                        (**curwin.as_ptr()).w_cursor = curpos_save;
                    }
                    if cin_isfuncdecl(&raw mut l, (**curwin.as_ptr()).w_cursor.lnum, 0) {
                        amount = (*g_st().curbuf).b_ind_param;
                        break;
                    }
                    if cin_ends_in(l, c";".as_ptr()) {
                        l = ml_get((**curwin.as_ptr()).w_cursor.lnum - 1);
                        if cin_ends_in(l, c",".as_ptr())
                            || (*l != NUL && *l.add(crate::memory::cstr_len(l) - 1) == b'\\' as c_char)
                        {
                            break;
                        }
                        l = get_cursor_line_ptr();
                    }
                    let _ = find_last_paren(l, b'(' as c_char, b')' as c_char);
                    trypos = find_match_paren((*g_st().curbuf).b_ind_maxparen);
                    if !trypos.is_null() {
                        (**curwin.as_ptr()).w_cursor = *trypos;
                    }
                    amount = get_indent();
                    break;
                }
                if cin_iscomment(theline) {
                    amount += (*g_st().curbuf).b_ind_comment;
                }
                if cur_curpos.lnum > 1 {
                    l = ml_get(cur_curpos.lnum - 1);
                    if *l != NUL && *l.add(crate::memory::cstr_len(l) - 1) == b'\\' as c_char {
                        cur_amount = cin_get_equal_amount(cur_curpos.lnum - 1);
                        if cur_amount > 0 {
                            amount = cur_amount;
                        } else if cur_amount == 0 {
                            amount += ind_continuation;
                        }
                    }
                }
            }
            if amount < 0 {
                amount = 0;
            }
        }
        (**curwin.as_ptr()).w_cursor = cur_curpos;
        amount
    }
}
fn find_match(lookfor: c_int, ourscope: linenr_T) -> bool {
    unsafe {
        let mut elselevel: c_int;
        let mut whilelevel: c_int;
        if lookfor == LOOKFOR_IF {
            elselevel = 1;
            whilelevel = 0;
        } else {
            elselevel = 0;
            whilelevel = 1;
        }
        (**curwin.as_ptr()).w_cursor.col = 0;
        while (**curwin.as_ptr()).w_cursor.lnum > ourscope + 1 {
            (**curwin.as_ptr()).w_cursor.lnum -= 1;
            (**curwin.as_ptr()).w_cursor.col = 0;
            let mut look = cin_skipcomment(get_cursor_line_ptr());
            if !cin_iselse(look)
                && !cin_isif(look)
                && !cin_isdo(look)
                && !cin_iswhileofdo(look, (**curwin.as_ptr()).w_cursor.lnum)
            {
                continue;
            }
            let theirscope = find_start_brace();
            if theirscope.is_null() {
                break;
            }
            if (*theirscope).lnum < ourscope {
                break;
            }
            if (*theirscope).lnum > ourscope {
                continue;
            }
            look = cin_skipcomment(get_cursor_line_ptr());
            if !(lookfor == LOOKFOR_IF && whilelevel != 0) {
                if cin_iselse(look) {
                    let mightbeif = cin_skipcomment(look.add(4));
                    if !cin_isif(mightbeif) {
                        elselevel += 1;
                    }
                    continue;
                }
                if cin_isif(look) {
                    elselevel -= 1;
                    if elselevel == 0 && lookfor == LOOKFOR_IF {
                        whilelevel = 0;
                    }
                }
            }
            if cin_iswhileofdo(look, (**curwin.as_ptr()).w_cursor.lnum) {
                whilelevel += 1;
                continue;
            }
            if cin_isdo(look) {
                whilelevel -= 1;
            }
            if elselevel <= 0 && whilelevel <= 0 {
                return true;
            }
        }
        false
    }
}
/// Check that "cinkeys" contains the key "keytyped".
pub fn in_cinkeys(keytyped: c_int, when: c_int, line_is_empty: bool) -> bool {
    unsafe {
        if keytyped == NUL as c_int {
            return false;
        }
        let mut look: *const c_char = if (*g_st().curbuf).b_p_inde.cb_type() != kCallbackNone {
            (*g_st().curbuf).b_p_indk
        } else {
            (*g_st().curbuf).b_p_cink
        };
        while *look != NUL {
            let mut try_match = match when as u8 {
                b'*' => *look == b'*' as c_char,
                b'!' => *look == b'!' as c_char,
                _ => *look != b'*' as c_char,
            };
            if *look == b'*' as c_char || *look == b'!' as c_char {
                look = look.add(1);
            }
            let try_match_word = if *look == b'0' as c_char {
                let tmw = try_match;
                if !line_is_empty {
                    try_match = false;
                }
                look = look.add(1);
                tmw
            } else {
                false
            };
            if *look == b'^' as c_char
                && *look.add(1) >= b'?' as c_char
                && *look.add(1) <= b'_' as c_char
            {
                if try_match && keytyped == ctrl_chr(c_int::from(*look.add(1) as u8)) {
                    return true;
                }
                look = look.add(2);
            } else if *look == b'o' as c_char {
                if try_match && keytyped == KEY_OPEN_FORW {
                    return true;
                }
                look = look.add(1);
            } else if *look == b'O' as c_char {
                if try_match && keytyped == KEY_OPEN_BACK {
                    return true;
                }
                look = look.add(1);
            } else if *look == b'e' as c_char {
                if try_match && keytyped == b'e' as c_int && (**curwin.as_ptr()).w_cursor.col >= 4 {
                    let p = get_cursor_line_ptr();
                    if skipwhite(p) == p.add((**curwin.as_ptr()).w_cursor.col as usize - 4)
                        && (crate::memory::cstr_prefix_bytes(p.add((**curwin.as_ptr()).w_cursor.col as usize - 4), 4) == &b"else"[..4])
                    {
                        return true;
                    }
                }
                look = look.add(1);
            } else if *look == b':' as c_char {
                if try_match && keytyped == b':' as c_int {
                    let mut p = get_cursor_line_ptr();
                    if cin_iscase(p, false) || cin_isscopedecl(p) || cin_islabel() {
                        return true;
                    }
                    p = get_cursor_line_ptr();
                    if (**curwin.as_ptr()).w_cursor.col > 2
                        && *p.add((**curwin.as_ptr()).w_cursor.col as usize - 1) == b':' as c_char
                        && *p.add((**curwin.as_ptr()).w_cursor.col as usize - 2) == b':' as c_char
                    {
                        *p.add((**curwin.as_ptr()).w_cursor.col as usize - 1) = b' ' as c_char;
                        let i = cin_iscase(p, false) || cin_isscopedecl(p) || cin_islabel();
                        p = get_cursor_line_ptr();
                        *p.add((**curwin.as_ptr()).w_cursor.col as usize - 1) = b':' as c_char;
                        if i {
                            return true;
                        }
                    }
                }
                look = look.add(1);
            } else if *look == b'<' as c_char {
                if try_match {
                    if !vim_strchr(c"<>!*oOe0:".as_ptr(), *look.add(1) as u8 as c_int).is_null()
                        && keytyped == *look.add(1) as c_int
                    {
                        return true;
                    }
                    if keytyped == get_special_key_code(look.add(1)) {
                        return true;
                    }
                }
                while *look != NUL && *look != b'>' as c_char {
                    look = look.add(1);
                }
                while *look == b'>' as c_char {
                    look = look.add(1);
                }
            } else if *look == b'=' as c_char
                && *look.add(1) != b',' as c_char
                && *look.add(1) != NUL
            {
                look = look.add(1);
                let icase = if *look == b'~' as c_char {
                    look = look.add(1);
                    true
                } else {
                    false
                };
                let mut p: *const c_char = vim_strchr(look, b',' as c_int);
                if p.is_null() {
                    p = look.add(crate::memory::cstr_len(look));
                }
                let word_len = p.offset_from(look) as usize;
                if (try_match || try_match_word) && ((**curwin.as_ptr()).w_cursor.col as usize) >= word_len {
                    let mut matched = false;
                    if keytyped == KEY_COMPLETE {
                        let line = get_cursor_line_ptr();
                        let mut s: *mut c_char = line.add((**curwin.as_ptr()).w_cursor.col as usize);
                        while s > line {
                            let n = mb_prevptr(line, s);
                            if !vim_iswordp(n) {
                                break;
                            }
                            s = n;
                        }
                        if s.add(word_len) <= line.add((**curwin.as_ptr()).w_cursor.col as usize)
                            && (if icase {
                                mb_strnicmp(s, look, word_len)
                            } else {
                                crate::memory::cstr_prefix_bytes(s, word_len).cmp(crate::memory::cstr_prefix_bytes(look, word_len)) as c_int
                            }) == 0
                        {
                            matched = true;
                        }
                    } else if keytyped == *p.sub(1) as u8 as c_int
                        || (icase
                            && (0..256).contains(&keytyped)
                            && (keytyped as u8).eq_ignore_ascii_case(&(*p.sub(1) as u8)))
                    {
                        let line = get_cursor_pos_ptr();
                        if ((**curwin.as_ptr()).w_cursor.col as usize == word_len
                            || !vim_iswordc(*line.sub(word_len + 1) as u8 as c_int))
                            && (if icase {
                                mb_strnicmp(line.sub(word_len), look, word_len)
                            } else {
                                crate::memory::cstr_prefix_bytes(line.sub(word_len), word_len).cmp(crate::memory::cstr_prefix_bytes(look, word_len)) as c_int
                            }) == 0
                        {
                            matched = true;
                        }
                    }
                    if matched
                        && try_match_word
                        && !try_match
                        && getwhitecols_curline() as c_int
                            != ((**curwin.as_ptr()).w_cursor.col as c_int - word_len as c_int)
                    {
                        matched = false;
                    }
                    if matched {
                        return true;
                    }
                }
                look = p;
            } else {
                if try_match && *look as u8 as c_int == keytyped {
                    return true;
                }
                if *look != NUL {
                    look = look.add(1);
                }
            }
            look = skip_to_option_part(look);
        }
        false
    }
}
/// Do C or expression indenting on the current line.
pub fn do_c_expr_indent() {
    unsafe {
        if (*g_st().curbuf).b_p_inde.cb_type() != kCallbackNone {
            fixthisline(get_expr_indent);
        } else {
            fixthisline(get_c_indent);
        }
    }
}
/// "cindent(lnum)" function
pub fn f_cindent(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let pos = (**curwin.as_ptr()).w_cursor;
        let lnum = tv_get_lnum(&argvars[0]);
        if lnum >= 1 && lnum <= (*g_st().curbuf).b_ml.ml_line_count {
            (**curwin.as_ptr()).w_cursor.lnum = lnum;
            rettv.vval = crate::eval::typval::TypvalVval::Number(get_c_indent() as i64);
            (**curwin.as_ptr()).w_cursor = pos;
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctrl_chr_and_pos_ordering_work() {
        assert_eq!(ctrl_chr(c_int::from(b'?')), 0x7f);
        assert_eq!(ctrl_chr(c_int::from(b'a')), 1);
        assert_eq!(ctrl_chr(c_int::from(b'A')), 1);
        let a = pos_T {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let b = pos_T {
            lnum: 1,
            col: 5,
            coladd: 0,
        };
        assert!(lt(a, b));
        assert!(!lt(b, a));
    }
}
