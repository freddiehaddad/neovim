//! C-indentation functions (`src/nvim/indent_c.c`).
use crate::buffer::{FileBuf, MAXLNUM};
use crate::change::{
    COM_END, COM_LEFT, COM_MAX_LEN, COM_MIDDLE, COM_RIGHT, COM_START, KEY_OPEN_BACK, KEY_OPEN_FORW,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, getdigits_int, getwhitecols_curline, skiptowhite, skipwhite,
    vim_is_idc, vim_iswordc, vim_iswordp, vim_strsize,
};
use crate::cursor::{Colnr, CursorPos, Linenr, Lpos, get_cursor_line_ptr, get_cursor_pos_ptr, lt};
use crate::eval::typval::{EvalFuncData, K_CALLBACK_NONE_CONST, TypVal, tv_get_lnum};
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
use crate::strings::vim_strchr;
use std::ffi::{c_char, c_int};
use std::ptr;
pub(crate) const FM_BACKWARD: c_int = 0x01;
pub(crate) const FM_BLOCKSTOP: c_int = 0x04;
pub(crate) const FM_SKIPCOMM: c_int = 0x08;
pub(crate) const KEY_COMPLETE: c_int = 0x103;
const FIND_NAMESPACE_LIM: Linenr = 20;
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
struct CppBaseclassCache {
    found: c_int,
    lpos: Lpos,
}
struct IndentCState {
    comment_pos_copy: CursorPos,
    find_line_comment_pos: CursorPos,
    start_brace_pos_copy: CursorPos,
    match_char_pos_copy: CursorPos,
}

impl IndentCState {
    const fn new() -> Self {
        Self {
            comment_pos_copy: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            find_line_comment_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            start_brace_pos_copy: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            match_char_pos_copy: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
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
pub fn check_linecomment(line: impl crate::strings::IntoCCharPtr) -> c_int {
    let line = line.into_c_char_ptr();
    unsafe {
        let mut p = line;
        if curbuf_mut().b_p_lisp != 0 {
            if !vim_strchr(p, b';' as c_int).is_null() {
                let mut in_str = false;
                while {
                    p = crate::memory::cstr_as_bytes(p)
                        .iter()
                        .position(|&b| b == b'"' || b == b';')
                        .map_or(std::ptr::null(), |i| p.add(i));
                    !p.is_null()
                } {
                    if crate::memory::cstr_first_byte(p) == b'"' {
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
                        && is_pos_in_string(line, p.offset_from(line) as Colnr) == 0
                    {
                        break;
                    }
                    p = p.add(1);
                }
            } else {
                p = std::ptr::null();
            }
        } else {
            while !crate::memory::cstr_is_empty(p) {
                p = skip_string(p);
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
                if crate::memory::cstr_first_byte(p) == b'/'
                    && crate::memory::cstr_byte_at(p, 1) == b'/'
                    && (p == line
                        || *p.sub(1) != b'*' as c_char
                        || crate::memory::cstr_byte_at(p, 2) != b'*')
                {
                    break;
                }
                p = p.add(1);
            }
            if crate::memory::cstr_is_empty(p) {
                p = std::ptr::null();
            }
        }
        if p.is_null() {
            return MAXCOL;
        }
        p.offset_from(line) as c_int
    }
}
fn ind_find_start_comment() -> *mut CursorPos {
    find_start_comment(curbuf_mut().b_ind_maxcomment)
}
pub fn find_start_comment(ind_maxcomment: c_int) -> *mut CursorPos {
    unsafe {
        let mut pos: *mut CursorPos;
        let mut cur_maxcomment = ind_maxcomment as i64;
        loop {
            pos = findmatchlimit(None, b'*' as c_int, FM_BACKWARD, cur_maxcomment);
            if pos.is_null() {
                break;
            }
            if is_pos_in_string(ml_get((*pos).lnum), (*pos).col) == 0 {
                break;
            }
            cur_maxcomment = (curwin_mut().w_cursor.lnum - (*pos).lnum - 1) as i64;
            if cur_maxcomment <= 0 {
                pos = ptr::null_mut();
                break;
            }
        }
        pos
    }
}
fn ind_find_start_cors(is_raw: Option<&mut Linenr>) -> *mut CursorPos {
    unsafe {
        let mut comment_pos = find_start_comment(curbuf_mut().b_ind_maxcomment);
        if !comment_pos.is_null() {
            indc_st().comment_pos_copy = *comment_pos;
            comment_pos = &raw mut indc_st().comment_pos_copy;
        }
        let rs_pos = find_start_rawstring(curbuf_mut().b_ind_maxcomment);
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
fn find_start_rawstring(ind_maxcomment: c_int) -> *mut CursorPos {
    unsafe {
        let mut pos: *mut CursorPos;
        let mut cur_maxcomment = ind_maxcomment;
        loop {
            pos = findmatchlimit(None, b'R' as c_int, FM_BACKWARD, cur_maxcomment as i64);
            if pos.is_null() {
                break;
            }
            if is_pos_in_string(ml_get((*pos).lnum), (*pos).col) == 0 {
                break;
            }
            cur_maxcomment = curwin_mut().w_cursor.lnum - (*pos).lnum - 1;
            if cur_maxcomment <= 0 {
                pos = ptr::null_mut();
                break;
            }
        }
        pos
    }
}
fn skip_string(p: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let mut p = p.into_c_char_ptr();
    unsafe {
        loop {
            if crate::memory::cstr_first_byte(p) == b'\'' {
                if crate::memory::cstr_byte_at(p, 1) == 0 {
                    break;
                }
                let mut i: usize = 2;
                if crate::memory::cstr_byte_at(p, 1) == b'\\'
                    && crate::memory::cstr_byte_at(p, 2) != 0
                {
                    i += 1;
                    while ascii_isdigit(crate::memory::cstr_char_at(p, i - 1)) {
                        i += 1;
                    }
                }
                if crate::memory::cstr_byte_at(p, i - 1) != 0
                    && crate::memory::cstr_byte_at(p, i) == b'\''
                {
                    p = p.add(i + 1);
                    continue;
                }
            } else if crate::memory::cstr_first_byte(p) == b'"' {
                p = p.add(1);
                while !crate::memory::cstr_is_empty(p) {
                    if crate::memory::cstr_first_byte(p) == b'\\'
                        && crate::memory::cstr_byte_at(p, 1) != 0
                    {
                        p = p.add(1);
                    } else if crate::memory::cstr_first_byte(p) == b'"' {
                        break;
                    }
                    p = p.add(1);
                }
                if crate::memory::cstr_first_byte(p) == b'"' {
                    p = p.add(1);
                    continue;
                }
            } else if crate::memory::cstr_first_byte(p) == b'R'
                && crate::memory::cstr_byte_at(p, 1) == b'"'
            {
                let delim = p.add(2);
                let paren = vim_strchr(delim, b'(' as c_int);
                if !paren.is_null() {
                    let delim_len = paren.offset_from(delim) as usize;
                    p = p.add(3);
                    while !crate::memory::cstr_is_empty(p) {
                        if crate::memory::cstr_first_byte(p) == b')'
                            && (crate::memory::cstr_prefix_bytes(p.add(1), delim_len)
                                == crate::memory::cstr_prefix_bytes(delim, delim_len))
                            && crate::memory::cstr_byte_at(p, delim_len + 1) == b'"'
                        {
                            p = p.add(delim_len + 1);
                            break;
                        }
                        p = p.add(1);
                    }
                    if crate::memory::cstr_first_byte(p) == b'"' {
                        p = p.add(1);
                        continue;
                    }
                }
            }
            break;
        }
        if crate::memory::cstr_is_empty(p) {
            p = p.sub(1);
        }
        p
    }
}
pub fn is_pos_in_string(line: impl crate::strings::IntoCCharPtr, col: Colnr) -> c_int {
    let line = line.into_c_char_ptr();
    unsafe {
        let mut p = line;
        while !crate::memory::cstr_is_empty(p) && (p.offset_from(line) as Colnr) < col {
            p = skip_string(p);
            p = p.add(1);
        }
        c_int::from(!((p.offset_from(line) as Colnr) <= col))
    }
}
pub fn cin_is_cinword(line: impl crate::strings::IntoCCharPtr) -> bool {
    let mut line = line.into_c_char_ptr();
    {
        let mut retval = false;
        let cinw_len = crate::memory::cstr_len(curbuf_mut().b_p_cinw) + 1;
        let mut cinw_buf = vec![0 as c_char; cinw_len];
        line = skipwhite(line);
        let mut cinw = curbuf_mut().b_p_cinw;
        while !crate::memory::cstr_is_empty(cinw) {
            let len = copy_option_part(&raw mut cinw, cinw_buf.as_mut_ptr(), cinw_len, c",");
            if (crate::memory::cstr_prefix_bytes(line, len)
                == crate::memory::cstr_prefix_bytes(cinw_buf.as_ptr(), len))
                && (!vim_iswordc(crate::memory::cstr_byte_at(line, len) as c_int)
                    || (len > 0
                        && !vim_iswordc(crate::memory::cstr_byte_at(line, len - 1) as c_int)))
            {
                retval = true;
                break;
            }
        }
        retval
    }
}
pub fn cindent_on() -> bool {
    {
        opt_vars().p_paste == 0
            && (curbuf_mut().b_p_cin != 0
                || curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST)
    }
}
fn cin_skipcomment(s: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let mut s = s.into_c_char_ptr();
    unsafe {
        while !crate::memory::cstr_is_empty(s) {
            let prev_s = s;
            s = skipwhite(s);
            if curbuf_mut().b_ind_hash_comment != 0
                && s != prev_s
                && crate::memory::cstr_first_byte(s) == b'#'
            {
                s = s.add(crate::memory::cstr_len(s));
                break;
            }
            if crate::memory::cstr_first_byte(s) != b'/' {
                break;
            }
            s = s.add(1);
            if crate::memory::cstr_first_byte(s) == b'/' {
                s = s.add(crate::memory::cstr_len(s));
                break;
            }
            if crate::memory::cstr_first_byte(s) != b'*' {
                break;
            }
            s = s.add(1);
            while !crate::memory::cstr_is_empty(s) {
                if crate::memory::cstr_first_byte(s) == b'*'
                    && crate::memory::cstr_byte_at(s, 1) == b'/'
                {
                    s = s.add(2);
                    break;
                }
                s = s.add(1);
            }
        }
        s
    }
}
fn cin_nocode(s: impl crate::strings::IntoCCharPtr) -> bool {
    let s = s.into_c_char_ptr();
    unsafe { *cin_skipcomment(s) == NUL }
}
fn find_line_comment() -> *mut CursorPos {
    unsafe {
        indc_st().find_line_comment_pos = curwin_mut().w_cursor;
        loop {
            indc_st().find_line_comment_pos.lnum -= 1;
            if indc_st().find_line_comment_pos.lnum <= 0 {
                break;
            }
            let line = ml_get(indc_st().find_line_comment_pos.lnum);
            let p = skipwhite(line);
            if cin_islinecomment(p) {
                indc_st().find_line_comment_pos.col = p.offset_from(line) as Colnr;
                return &raw mut indc_st().find_line_comment_pos;
            }
            if !crate::memory::cstr_is_empty(p) {
                break;
            }
        }
        ptr::null_mut()
    }
}
fn cin_has_js_key(text: impl crate::strings::IntoCCharPtr) -> bool {
    let text = text.into_c_char_ptr();
    unsafe {
        let mut s: *const c_char = skipwhite(text);
        let mut quote: c_char = 0;
        if crate::memory::cstr_first_byte(s) == b'\'' || crate::memory::cstr_first_byte(s) == b'"' {
            quote = *s;
            s = s.add(1);
        }
        if !vim_is_idc(crate::memory::cstr_first_byte(s) as c_int) {
            return false;
        }
        while vim_is_idc(crate::memory::cstr_first_byte(s) as c_int) {
            s = s.add(1);
        }
        if !crate::memory::cstr_is_empty(s) && *s == quote {
            s = s.add(1);
        }
        s = cin_skipcomment(s);
        crate::memory::cstr_first_byte(s) == b':' && crate::memory::cstr_byte_at(s, 1) != b':'
    }
}
fn cin_islabel_skip(s: *mut *const c_char) -> bool {
    unsafe {
        if !vim_is_idc(crate::memory::cstr_first_byte(*s) as c_int) {
            return false;
        }
        while vim_is_idc(crate::memory::cstr_first_byte(*s) as c_int) {
            *s = (*s).add(utfc_ptr2len(*s) as usize);
        }
        *s = cin_skipcomment(*s);
        if crate::memory::cstr_first_byte(*s) == b':' {
            *s = (*s).add(1);
            crate::memory::cstr_first_byte(*s) != b':'
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
        if !ind_find_start_cors(None).is_null() {
            return false;
        }
        let cursor_save = curwin_mut().w_cursor;
        while curwin_mut().w_cursor.lnum > 1 {
            curwin_mut().w_cursor.lnum -= 1;
            curwin_mut().w_cursor.col = 0;
            let trypos = ind_find_start_cors(None);
            if !trypos.is_null() {
                curwin_mut().w_cursor = *trypos;
            }
            let mut line: *const c_char = get_cursor_line_ptr();
            if cin_ispreproc(line) {
                continue;
            }
            line = cin_skipcomment(line);
            if crate::memory::cstr_is_empty(line) {
                continue;
            }
            curwin_mut().w_cursor = cursor_save;
            if cin_isterminated(line, true, false) != 0
                || cin_isscopedecl(line)
                || cin_iscase(line, true)
                || (cin_islabel_skip(&raw mut line) && cin_nocode(line))
            {
                return true;
            }
            return false;
        }
        curwin_mut().w_cursor = cursor_save;
        true
    }
}
fn cin_skip_comment_and_string(s: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let s = s.into_c_char_ptr();
    {
        let mut r: *const c_char;
        let mut p = s;
        loop {
            r = p;
            p = cin_skipcomment(p);
            if !crate::memory::cstr_is_empty(p) {
                p = skip_string(p);
            }
            if p == r {
                break;
            }
        }
        p
    }
}
fn cin_is_compound_init(s: impl crate::strings::IntoCCharPtr) -> bool {
    let s = s.into_c_char_ptr();
    unsafe {
        let mut p = s;
        let mut r: *const c_char = std::ptr::null();
        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) == b'=' {
                r = cin_skipcomment(p.add(1));
                p = r;
            } else if (crate::memory::cstr_prefix_bytes(p, 6) == &b"return"[..6])
                && !vim_is_idc(crate::memory::cstr_byte_at(p, 6) as c_int)
                && (p == s || (p > s && !vim_is_idc(*p.sub(1) as u8 as c_int)))
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
        if crate::memory::cstr_first_byte(p) == b'&' {
            p = cin_skipcomment(p.add(1));
        }
        if crate::memory::cstr_first_byte(p) == b'(' {
            let mut open_count: c_int = 1;
            loop {
                p = cin_skip_comment_and_string(p.add(1));
                if cin_nocode(p) {
                    return true;
                }
                open_count += c_int::from(crate::memory::cstr_first_byte(p) == b'(')
                    - c_int::from(crate::memory::cstr_first_byte(p) == b')');
                if open_count == 0 {
                    break;
                }
            }
            p = cin_skipcomment(p.add(1));
            if cin_nocode(p) {
                return true;
            }
        }
        while crate::memory::cstr_first_byte(p) == b'{' {
            p = cin_skipcomment(p.add(1));
        }
        cin_nocode(p)
    }
}
fn cin_isinit() -> bool {
    unsafe {
        const SKIP: [&std::ffi::CStr; 4] = [c"static", c"public", c"protected", c"private"];
        let mut s = cin_skipcomment(get_cursor_line_ptr());
        if cin_starts_with(s, c"typedef") {
            s = cin_skipcomment(s.add(7));
        }
        loop {
            let mut matched = false;
            for &word in &SKIP {
                if cin_starts_with(s, word) {
                    s = cin_skipcomment(s.add(word.to_bytes().len()));
                    matched = true;
                    break;
                }
            }
            if !matched {
                break;
            }
        }
        if cin_starts_with(s, c"enum") {
            return true;
        }
        cin_is_compound_init(s)
    }
}
fn cin_iscase(s: impl crate::strings::IntoCCharPtr, strict: bool) -> bool {
    let mut s = s.into_c_char_ptr();
    unsafe {
        s = cin_skipcomment(s);
        if cin_starts_with(s, c"case") {
            s = s.add(4);
            while !crate::memory::cstr_is_empty(s) {
                s = cin_skipcomment(s);
                if crate::memory::cstr_is_empty(s) {
                    break;
                }
                if crate::memory::cstr_first_byte(s) == b':' {
                    if crate::memory::cstr_byte_at(s, 1) == b':' {
                        s = s.add(1);
                    } else {
                        return true;
                    }
                }
                if crate::memory::cstr_first_byte(s) == b'\''
                    && crate::memory::cstr_byte_at(s, 1) != 0
                    && crate::memory::cstr_byte_at(s, 2) == b'\''
                {
                    s = s.add(2);
                } else if crate::memory::cstr_first_byte(s) == b'/'
                    && (crate::memory::cstr_byte_at(s, 1) == b'*'
                        || crate::memory::cstr_byte_at(s, 1) == b'/')
                {
                    return false;
                } else if crate::memory::cstr_first_byte(s) == b'"' {
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
fn cin_isdefault(s: impl crate::strings::IntoCCharPtr) -> bool {
    let mut s = s.into_c_char_ptr();
    unsafe {
        if !(crate::memory::cstr_prefix_bytes(s, 7) == &b"default"[..7]) {
            return false;
        }
        s = cin_skipcomment(s.add(7));
        crate::memory::cstr_first_byte(s) == b':' && crate::memory::cstr_byte_at(s, 1) != b':'
    }
}
fn cin_isscopedecl(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    unsafe {
        let s = cin_skipcomment(p);
        let cinsd_len = crate::memory::cstr_len(curbuf_mut().b_p_cinsd) + 1;
        let mut cinsd_buf = vec![0 as c_char; cinsd_len];
        let mut found = false;
        let mut cinsd = curbuf_mut().b_p_cinsd;
        while !crate::memory::cstr_is_empty(cinsd) {
            let len = copy_option_part(&raw mut cinsd, cinsd_buf.as_mut_ptr(), cinsd_len, c",");
            if crate::memory::cstr_prefix_bytes(s, len)
                == crate::memory::cstr_prefix_bytes(cinsd_buf.as_ptr(), len)
            {
                let skip = cin_skipcomment(s.add(len));
                if crate::memory::cstr_first_byte(skip) == b':'
                    && crate::memory::cstr_byte_at(skip, 1) != b':'
                {
                    found = true;
                    break;
                }
            }
        }
        found
    }
}
fn cin_is_cpp_namespace(s: impl crate::strings::IntoCCharPtr) -> bool {
    let mut s = s.into_c_char_ptr();
    unsafe {
        let mut has_name = false;
        let mut has_name_start = false;
        s = cin_skipcomment(s);
        while ((crate::memory::cstr_prefix_bytes(s, 6) == &b"inline"[..6])
            || (crate::memory::cstr_prefix_bytes(s, 6) == &b"export"[..6]))
            && (crate::memory::cstr_byte_at(s, 6) == 0
                || !vim_iswordc(crate::memory::cstr_byte_at(s, 6) as c_int))
        {
            s = cin_skipcomment(skipwhite(s.add(6)));
        }
        if (crate::memory::cstr_prefix_bytes(s, 9) == &b"namespace"[..9])
            && (crate::memory::cstr_byte_at(s, 9) == 0
                || !vim_iswordc(crate::memory::cstr_byte_at(s, 9) as c_int))
        {
            let mut p = cin_skipcomment(skipwhite(s.add(9)));
            while !crate::memory::cstr_is_empty(p) {
                if ascii_iswhite(crate::memory::cstr_first_char(p)) {
                    has_name = true;
                    p = cin_skipcomment(skipwhite(p));
                } else if crate::memory::cstr_first_byte(p) == b'{' {
                    break;
                } else if vim_iswordc(crate::memory::cstr_first_byte(p) as c_int) {
                    has_name_start = true;
                    if has_name {
                        return false;
                    }
                    p = p.add(1);
                } else if crate::memory::cstr_first_byte(p) == b':'
                    && crate::memory::cstr_byte_at(p, 1) == b':'
                    && vim_iswordc(crate::memory::cstr_byte_at(p, 2) as c_int)
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
fn after_label(l: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let mut l = l.into_c_char_ptr();
    unsafe {
        while !crate::memory::cstr_is_empty(l) {
            if crate::memory::cstr_first_byte(l) == b':' {
                if crate::memory::cstr_byte_at(l, 1) == b':' {
                    l = l.add(1);
                } else if !cin_iscase(l.add(1), false) {
                    break;
                }
            } else if crate::memory::cstr_first_byte(l) == b'\''
                && crate::memory::cstr_byte_at(l, 1) != 0
                && crate::memory::cstr_byte_at(l, 2) == b'\''
            {
                l = l.add(2);
            }
            l = l.add(1);
        }
        if crate::memory::cstr_is_empty(l) {
            return std::ptr::null();
        }
        l = cin_skipcomment(l.add(1));
        if crate::memory::cstr_is_empty(l) {
            return std::ptr::null();
        }
        l
    }
}
fn get_indent_nolabel(lnum: Linenr) -> c_int {
    unsafe {
        let l = ml_get(lnum);
        let p = after_label(l);
        if p.is_null() {
            return 0;
        }
        let mut fp = CursorPos {
            lnum,
            col: p.offset_from(l) as Colnr,
            coladd: 0,
        };
        let mut col: Colnr = 0;
        getvcol(curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
        col
    }
}
fn skip_label(lnum: Linenr, pp: *mut *const c_char) -> c_int {
    unsafe {
        let cursor_save = curwin_mut().w_cursor;
        curwin_mut().w_cursor.lnum = lnum;
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
        curwin_mut().w_cursor = cursor_save;
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
            if ((crate::memory::cstr_prefix_bytes(s, 3) == &b"int"[..3])
                && ascii_iswhite(crate::memory::cstr_char_at(s, 3)))
                || ((crate::memory::cstr_prefix_bytes(s, 4) == &b"long"[..4])
                    && ascii_iswhite(crate::memory::cstr_char_at(s, 4)))
                || ((crate::memory::cstr_prefix_bytes(s, 5) == &b"short"[..5])
                    && ascii_iswhite(crate::memory::cstr_char_at(s, 5)))
                || ((crate::memory::cstr_prefix_bytes(s, 4) == &b"char"[..4])
                    && ascii_iswhite(crate::memory::cstr_char_at(s, 4)))
            {
                p = s;
            }
        }
        len = 0;
        while vim_is_idc(crate::memory::cstr_byte_at(p, len as usize) as c_int) {
            len += 1;
        }
        if len == 0 || !ascii_iswhite(crate::memory::cstr_char_at(p, len as usize)) || cin_nocode(p)
        {
            return 0;
        }
        p = skipwhite(p.add(len as usize));
        let mut fp = CursorPos {
            lnum: curwin_mut().w_cursor.lnum,
            col: p.offset_from(line) as Colnr,
            coladd: 0,
        };
        let mut col: Colnr = 0;
        getvcol(curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
        col
    }
}
fn cin_get_equal_amount(lnum: Linenr) -> c_int {
    unsafe {
        if lnum > 1 {
            let line = ml_get(lnum - 1);
            if !crate::memory::cstr_is_empty(line)
                && *line.add(crate::memory::cstr_len(line) - 1) == b'\\' as c_char
            {
                return -1;
            }
        }
        let line: *const c_char = ml_get(lnum);
        let mut s = line;
        while !crate::memory::cstr_is_empty(s)
            && vim_strchr(c"=;{}\"'", crate::memory::cstr_first_byte(s) as c_int).is_null()
        {
            if cin_iscomment(s) {
                s = cin_skipcomment(s);
            } else {
                s = s.add(1);
            }
        }
        if crate::memory::cstr_first_byte(s) != b'=' {
            return 0;
        }
        s = skipwhite(s.add(1));
        if cin_nocode(s) {
            return 0;
        }
        if crate::memory::cstr_first_byte(s) == b'"' {
            s = s.add(1);
        }
        let mut fp = CursorPos {
            lnum,
            col: s.offset_from(line) as Colnr,
            coladd: 0,
        };
        let mut col: Colnr = 0;
        getvcol(curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
        col
    }
}
fn cin_ispreproc(s: impl crate::strings::IntoCCharPtr) -> bool {
    let s = s.into_c_char_ptr();
    unsafe { *skipwhite(s) == b'#' as c_char }
}
fn cin_ispreproc_cont(pp: *mut *const c_char, lnump: &mut Linenr, amount: &mut c_int) -> bool {
    unsafe {
        let mut line = *pp;
        let mut lnum = *lnump;
        let mut retval = false;
        let mut candidate_amount = *amount;
        if !crate::memory::cstr_is_empty(line)
            && *line.add(crate::memory::cstr_len(line) - 1) == b'\\' as c_char
        {
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
            if crate::memory::cstr_is_empty(line)
                || *line.add(crate::memory::cstr_len(line) - 1) != b'\\' as c_char
            {
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
fn cin_iscomment(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    crate::memory::cstr_first_byte(p) == b'/'
        && (crate::memory::cstr_byte_at(p, 1) == b'*' || crate::memory::cstr_byte_at(p, 1) == b'/')
}
#[inline]
fn cin_islinecomment(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    crate::memory::cstr_first_byte(p) == b'/' && crate::memory::cstr_byte_at(p, 1) == b'/'
}
fn cin_isterminated(
    s: impl crate::strings::IntoCCharPtr,
    incl_open: bool,
    incl_comma: bool,
) -> c_char {
    let mut s = s.into_c_char_ptr();
    unsafe {
        let mut found_start: c_char = 0;
        let mut n_open: u32 = 0;
        let mut is_else = false;
        s = cin_skipcomment(s);
        if crate::memory::cstr_first_byte(s) == b'{'
            || (crate::memory::cstr_first_byte(s) == b'}' && !cin_iselse(s))
        {
            found_start = *s;
        }
        if found_start == 0 {
            is_else = cin_iselse(s);
        }
        while !crate::memory::cstr_is_empty(s) {
            s = skip_string(cin_skipcomment(s));
            if crate::memory::cstr_first_byte(s) == b'}' && n_open > 0 {
                n_open -= 1;
            }
            if (!is_else || n_open == 0)
                && (crate::memory::cstr_first_byte(s) == b';'
                    || crate::memory::cstr_first_byte(s) == b'}'
                    || (incl_comma && crate::memory::cstr_first_byte(s) == b','))
                && cin_nocode(s.add(1))
            {
                return *s;
            } else if crate::memory::cstr_first_byte(s) == b'{' {
                if incl_open && cin_nocode(s.add(1)) {
                    return *s;
                } else {
                    n_open += 1;
                }
            }
            if !crate::memory::cstr_is_empty(s) {
                s = s.add(1);
            }
        }
        found_start
    }
}
fn cin_isfuncdecl(sp: *mut *const c_char, first_lnum: Linenr, min_lnum: Linenr) -> bool {
    unsafe {
        let mut s: *const c_char = if sp.is_null() {
            ml_get(first_lnum)
        } else {
            *sp
        };
        let mut lnum = first_lnum;
        let save_lnum = curwin_mut().w_cursor.lnum;
        let mut retval = false;
        let mut just_started = true;
        curwin_mut().w_cursor.lnum = lnum;
        if find_last_paren(s, b'(' as c_char, b')' as c_char) {
            let trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
            if !trypos.is_null() {
                lnum = (*trypos).lnum;
                if lnum < min_lnum {
                    curwin_mut().w_cursor.lnum = save_lnum;
                    return false;
                }
                s = ml_get(lnum);
            }
        }
        curwin_mut().w_cursor.lnum = save_lnum;
        if cin_ispreproc(s) {
            return false;
        }
        while !crate::memory::cstr_is_empty(s)
            && crate::memory::cstr_first_byte(s) != b'('
            && crate::memory::cstr_first_byte(s) != b';'
            && crate::memory::cstr_first_byte(s) != b'\''
            && crate::memory::cstr_first_byte(s) != b'"'
        {
            if cin_iscomment(s) {
                s = cin_skipcomment(s);
            } else if crate::memory::cstr_first_byte(s) == b':' {
                if crate::memory::cstr_byte_at(s, 1) == b':' {
                    s = s.add(2);
                } else {
                    return false;
                }
            } else {
                s = s.add(1);
            }
        }
        if crate::memory::cstr_first_byte(s) != b'(' {
            return false;
        }
        'done: {
            while !crate::memory::cstr_is_empty(s)
                && crate::memory::cstr_first_byte(s) != b';'
                && crate::memory::cstr_first_byte(s) != b'\''
                && crate::memory::cstr_first_byte(s) != b'"'
            {
                if crate::memory::cstr_first_byte(s) == b')' && cin_nocode(s.add(1)) {
                    lnum = first_lnum - 1;
                    s = ml_get(lnum);
                    if crate::memory::cstr_is_empty(s)
                        || *s.add(crate::memory::cstr_len(s) - 1) != b'\\' as c_char
                    {
                        retval = true;
                    }
                    break 'done;
                }
                if (crate::memory::cstr_first_byte(s) == b',' && cin_nocode(s.add(1)))
                    || crate::memory::cstr_byte_at(s, 1) == 0
                    || cin_nocode(s)
                {
                    let comma = crate::memory::cstr_first_byte(s) == b',';
                    loop {
                        if lnum >= curbuf_mut().b_ml.ml_line_count {
                            break;
                        }
                        lnum += 1;
                        s = ml_get(lnum);
                        if !cin_ispreproc(s) {
                            break;
                        }
                    }
                    if lnum >= curbuf_mut().b_ml.ml_line_count {
                        break;
                    }
                    s = skipwhite(s);
                    if !just_started
                        && (!comma
                            && crate::memory::cstr_first_byte(s) != b','
                            && crate::memory::cstr_first_byte(s) != b')')
                    {
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
fn cin_isif(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    cin_starts_with(p, c"if")
}
fn cin_iselse(p: impl crate::strings::IntoCCharPtr) -> bool {
    let mut p = p.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_first_byte(p) == b'}' {
            p = cin_skipcomment(p.add(1));
        }
        cin_starts_with(p, c"else")
    }
}
fn cin_isdo(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    cin_starts_with(p, c"do")
}
fn cin_iswhileofdo(p: impl crate::strings::IntoCCharPtr, lnum: Linenr) -> bool {
    let mut p = p.into_c_char_ptr();
    unsafe {
        let mut retval = false;
        p = cin_skipcomment(p);
        if crate::memory::cstr_first_byte(p) == b'}' {
            p = cin_skipcomment(p.add(1));
        }
        if cin_starts_with(p, c"while") {
            let cursor_save = curwin_mut().w_cursor;
            curwin_mut().w_cursor.lnum = lnum;
            curwin_mut().w_cursor.col = 0;
            p = get_cursor_line_ptr();
            while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'w' {
                p = p.add(1);
                curwin_mut().w_cursor.col += 1;
            }
            let trypos = findmatchlimit(None, 0, FM_SKIPCOMM, curbuf_mut().b_ind_maxparen as i64);
            if !trypos.is_null() && *cin_skipcomment(ml_get_pos(&*trypos).add(1)) == b';' as c_char
            {
                retval = true;
            }
            curwin_mut().w_cursor = cursor_save;
        }
        retval
    }
}
fn cin_is_if_for_while_before_offset(
    line: impl crate::strings::IntoCCharPtr,
    poffset: &mut Colnr,
) -> c_int {
    let line = line.into_c_char_ptr();
    unsafe {
        let mut offset = *poffset;
        let prev_offset = offset;
        offset -= 1;
        if prev_offset < 2 {
            return 0;
        }
        while offset > 2 && ascii_iswhite(crate::memory::cstr_char_at(line, offset as usize)) {
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
                if crate::memory::cstr_prefix_bytes(line.add(offset as usize), 5) == &b"while"[..5]
                {
                    probably_found = true;
                }
            }
        }
        if !probably_found {
            return 0;
        }
        if offset == 0 || !vim_is_idc(*line.add((offset - 1) as usize) as u8 as c_int) {
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
        while !crate::memory::cstr_is_empty(p) {
            p = cin_skipcomment(p);
            if crate::memory::cstr_first_byte(p) == b')' {
                let s = skipwhite(p.add(1));
                if crate::memory::cstr_first_byte(s) == b';' && cin_nocode(s.add(1)) {
                    let i = p.offset_from(line) as Colnr;
                    curwin_mut().w_cursor.col = i;
                    let trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                    if !trypos.is_null() {
                        let mut s2 = cin_skipcomment(ml_get((*trypos).lnum));
                        if crate::memory::cstr_first_byte(s2) == b'}' {
                            s2 = cin_skipcomment(s2.add(1));
                        }
                        if cin_starts_with(s2, c"while") {
                            curwin_mut().w_cursor.lnum = (*trypos).lnum;
                            return true;
                        }
                    }
                    line = get_cursor_line_ptr();
                    p = line.add(i as usize);
                }
            }
            if !crate::memory::cstr_is_empty(p) {
                p = p.add(1);
            }
        }
        false
    }
}
fn cin_isbreak(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    cin_starts_with(p, c"break")
}
fn cin_is_cpp_baseclass(cached: &mut CppBaseclassCache) -> bool {
    unsafe {
        let pos = &mut cached.lpos;
        let mut lnum = curwin_mut().w_cursor.lnum;
        let mut line: *const c_char = get_cursor_line_ptr();
        if pos.lnum <= lnum {
            return cached.found != 0;
        }
        pos.col = 0;
        let mut s: *const c_char = skipwhite(line);
        if crate::memory::cstr_first_byte(s) == b'#' {
            return false;
        }
        s = cin_skipcomment(s);
        if crate::memory::cstr_is_empty(s) {
            return false;
        }
        let mut cpp_base_class = false;
        let mut lookfor_ctor_init = false;
        let mut class_or_struct = false;
        while lnum > 1 {
            line = ml_get(lnum - 1);
            s = skipwhite(line);
            if crate::memory::cstr_first_byte(s) == b'#' || crate::memory::cstr_is_empty(s) {
                break;
            }
            while !crate::memory::cstr_is_empty(s) {
                s = cin_skipcomment(s);
                if crate::memory::cstr_first_byte(s) == b'{'
                    || crate::memory::cstr_first_byte(s) == b'}'
                    || (crate::memory::cstr_first_byte(s) == b';' && cin_nocode(s.add(1)))
                {
                    break;
                }
                if !crate::memory::cstr_is_empty(s) {
                    s = s.add(1);
                }
            }
            if !crate::memory::cstr_is_empty(s) {
                break;
            }
            lnum -= 1;
        }
        pos.lnum = lnum;
        line = ml_get(lnum);
        s = line;
        loop {
            if crate::memory::cstr_is_empty(s) {
                if lnum == curwin_mut().w_cursor.lnum {
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
                if crate::memory::cstr_is_empty(s) {
                    continue;
                }
            }
            if crate::memory::cstr_first_byte(s) == b'"'
                || (crate::memory::cstr_first_byte(s) == b'R'
                    && crate::memory::cstr_byte_at(s, 1) == b'"')
            {
                s = skip_string(s).add(1);
            } else if crate::memory::cstr_first_byte(s) == b':' {
                if crate::memory::cstr_byte_at(s, 1) == b':' {
                    lookfor_ctor_init = false;
                    s = cin_skipcomment(s.add(2));
                } else if lookfor_ctor_init || class_or_struct {
                    cpp_base_class = true;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                    pos.col = 0;
                    s = cin_skipcomment(s.add(1));
                } else {
                    s = cin_skipcomment(s.add(1));
                }
            } else if ((crate::memory::cstr_prefix_bytes(s, 5) == &b"class"[..5])
                && !vim_is_idc(crate::memory::cstr_byte_at(s, 5) as c_int))
                || ((crate::memory::cstr_prefix_bytes(s, 6) == &b"struct"[..6])
                    && !vim_is_idc(crate::memory::cstr_byte_at(s, 6) as c_int))
            {
                class_or_struct = true;
                lookfor_ctor_init = false;
                if crate::memory::cstr_first_byte(s) == b'c' {
                    s = cin_skipcomment(s.add(5));
                } else {
                    s = cin_skipcomment(s.add(6));
                }
            } else {
                if crate::memory::cstr_first_byte(s) == b'{'
                    || crate::memory::cstr_first_byte(s) == b'}'
                    || crate::memory::cstr_first_byte(s) == b';'
                {
                    cpp_base_class = false;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                } else if crate::memory::cstr_first_byte(s) == b')' {
                    class_or_struct = false;
                    lookfor_ctor_init = true;
                } else if crate::memory::cstr_first_byte(s) == b'?' {
                    return false;
                } else if !vim_is_idc(crate::memory::cstr_first_byte(s) as c_int) {
                    class_or_struct = false;
                    lookfor_ctor_init = false;
                } else if pos.col == 0 {
                    lookfor_ctor_init = false;
                    if cpp_base_class {
                        pos.col = s.offset_from(line) as Colnr;
                    }
                }
                if lnum == curwin_mut().w_cursor.lnum
                    && crate::memory::cstr_first_byte(s) == b','
                    && cin_nocode(s.add(1))
                {
                    pos.col = 0;
                }
                s = cin_skipcomment(s.add(1));
            }
        }
        cached.found = c_int::from(cpp_base_class);
        if cpp_base_class {
            pos.lnum = lnum;
        }
        cpp_base_class
    }
}
fn get_baseclass_amount(col: Colnr) -> c_int {
    unsafe {
        let mut amount: c_int;
        let mut vcol: Colnr = 0;
        if col == 0 {
            amount = get_indent();
            if find_last_paren(get_cursor_line_ptr(), b'(' as c_char, b')' as c_char) {
                let trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                if !trypos.is_null() {
                    amount = get_indent_lnum((*trypos).lnum);
                }
            }
            if !cin_ends_in(get_cursor_line_ptr(), c",") {
                amount += curbuf_mut().b_ind_cpp_baseclass;
            }
        } else {
            curwin_mut().w_cursor.col = col;
            getvcol(
                curwin_mut(),
                &mut curwin_mut().w_cursor,
                Some(&mut vcol),
                None,
                None,
                0,
            );
            amount = vcol;
        }
        if amount < curbuf_mut().b_ind_cpp_baseclass {
            amount = curbuf_mut().b_ind_cpp_baseclass;
        }
        amount
    }
}
fn cin_ends_in(s: impl crate::strings::IntoCCharPtr, find: &std::ffi::CStr) -> bool {
    let s = s.into_c_char_ptr();
    unsafe {
        let mut p = s;
        let len = find.to_bytes().len();
        while !crate::memory::cstr_is_empty(p) {
            p = cin_skipcomment(p);
            if crate::memory::cstr_prefix_bytes(p, len)
                == crate::memory::cstr_prefix_bytes(find.as_ptr(), len)
            {
                let r = skipwhite(p.add(len));
                if cin_nocode(r) {
                    return true;
                }
            }
            if !crate::memory::cstr_is_empty(p) {
                p = p.add(1);
            }
        }
        false
    }
}
fn cin_starts_with(s: impl crate::strings::IntoCCharPtr, word: &std::ffi::CStr) -> bool {
    let s = s.into_c_char_ptr();
    {
        let wb = word.to_bytes();
        let l = wb.len();
        crate::memory::xstrnlen(s, l) == l
            && crate::memory::cstr_slice(s, l) == wb
            && !vim_is_idc(crate::memory::cstr_byte_at(s, l) as c_int)
    }
}
fn cin_is_cpp_extern_c(s: impl crate::strings::IntoCCharPtr) -> bool {
    let mut s = s.into_c_char_ptr();
    unsafe {
        let mut has_string_literal = false;
        s = cin_skipcomment(s);
        if (crate::memory::cstr_prefix_bytes(s, 6) == &b"extern"[..6])
            && (crate::memory::cstr_byte_at(s, 6) == 0
                || !vim_iswordc(crate::memory::cstr_byte_at(s, 6) as c_int))
        {
            let mut p = cin_skipcomment(skipwhite(s.add(6)));
            while !crate::memory::cstr_is_empty(p) {
                if ascii_iswhite(crate::memory::cstr_first_char(p)) {
                    p = cin_skipcomment(skipwhite(p));
                } else if crate::memory::cstr_first_byte(p) == b'{' {
                    break;
                } else if crate::memory::cstr_first_byte(p) == b'"'
                    && crate::memory::cstr_byte_at(p, 1) == b'C'
                    && crate::memory::cstr_byte_at(p, 2) == b'"'
                {
                    if has_string_literal {
                        return false;
                    }
                    has_string_literal = true;
                    p = p.add(3);
                } else if crate::memory::cstr_first_byte(p) == b'"'
                    && crate::memory::cstr_byte_at(p, 1) == b'C'
                    && crate::memory::cstr_byte_at(p, 2) == b'+'
                    && crate::memory::cstr_byte_at(p, 3) == b'+'
                    && crate::memory::cstr_byte_at(p, 4) == b'"'
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
fn cin_skip2pos(trypos: &mut CursorPos) -> c_int {
    unsafe {
        let line: *const c_char = ml_get(trypos.lnum);
        let mut p = line;
        while !crate::memory::cstr_is_empty(p) && (p.offset_from(line) as Colnr) < trypos.col {
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
fn find_start_brace() -> *mut CursorPos {
    unsafe {
        let cursor_save = curwin_mut().w_cursor;
        let mut trypos: *mut CursorPos;
        loop {
            trypos = findmatchlimit(None, b'{' as c_int, FM_BLOCKSTOP | FM_SKIPCOMM, 0);
            if trypos.is_null() {
                break;
            }
            indc_st().start_brace_pos_copy = *trypos;
            trypos = &raw mut indc_st().start_brace_pos_copy;
            curwin_mut().w_cursor = *trypos;
            let mut pos: *mut CursorPos = ptr::null_mut();
            if cin_skip2pos(&mut *trypos) as Colnr == (*trypos).col && {
                pos = ind_find_start_cors(None);
                pos.is_null()
            } {
                break;
            }
            if !pos.is_null() {
                curwin_mut().w_cursor = *pos;
            }
        }
        curwin_mut().w_cursor = cursor_save;
        trypos
    }
}
fn find_match_paren(ind_maxparen: c_int) -> *mut CursorPos {
    find_match_char(b'(' as c_char, ind_maxparen)
}
fn find_match_char(c: c_char, ind_maxparen: c_int) -> *mut CursorPos {
    unsafe {
        let cursor_save = curwin_mut().w_cursor;
        let mut ind_maxp_wk = ind_maxparen;
        let mut trypos: *mut CursorPos;
        loop {
            trypos = findmatchlimit(None, c as u8 as c_int, FM_SKIPCOMM, ind_maxp_wk as i64);
            if !trypos.is_null() {
                if (cin_skip2pos(&mut *trypos) as Colnr) > (*trypos).col {
                    ind_maxp_wk = ind_maxparen - (cursor_save.lnum - (*trypos).lnum);
                    if ind_maxp_wk > 0 {
                        curwin_mut().w_cursor = *trypos;
                        curwin_mut().w_cursor.col = 0;
                        continue;
                    }
                    trypos = ptr::null_mut();
                } else {
                    indc_st().match_char_pos_copy = *trypos;
                    trypos = &raw mut indc_st().match_char_pos_copy;
                    curwin_mut().w_cursor = *trypos;
                    let trypos_wk = ind_find_start_cors(None);
                    if !trypos_wk.is_null() {
                        ind_maxp_wk = ind_maxparen - (cursor_save.lnum - (*trypos_wk).lnum);
                        if ind_maxp_wk > 0 {
                            curwin_mut().w_cursor = *trypos_wk;
                            continue;
                        }
                        trypos = ptr::null_mut();
                    }
                }
            }
            break;
        }
        curwin_mut().w_cursor = cursor_save;
        trypos
    }
}
fn find_match_paren_after_brace(ind_maxparen: c_int) -> *mut CursorPos {
    unsafe {
        let mut trypos = find_match_paren(ind_maxparen);
        if trypos.is_null() {
            return ptr::null_mut();
        }
        let trypos_brace = find_start_brace();
        if !trypos_brace.is_null()
            && (if (*trypos).lnum != (*trypos_brace).lnum {
                (*trypos).lnum < (*trypos_brace).lnum
            } else {
                (*trypos).col < (*trypos_brace).col
            })
        {
            trypos = ptr::null_mut();
        }
        trypos
    }
}
fn corr_ind_maxparen(startpos: &CursorPos) -> c_int {
    {
        let n = startpos.lnum - curwin_mut().w_cursor.lnum;
        if n > 0 && n < curbuf_mut().b_ind_maxparen / 2 {
            return curbuf_mut().b_ind_maxparen - n;
        }
        curbuf_mut().b_ind_maxparen
    }
}
fn find_last_paren(l: impl crate::strings::IntoCCharPtr, start: c_char, end: c_char) -> bool {
    let l = l.into_c_char_ptr();
    unsafe {
        let mut retval = false;
        let mut open_count: c_int = 0;
        curwin_mut().w_cursor.col = 0;
        let mut i: usize = 0;
        while crate::memory::cstr_byte_at(l, i) != 0 {
            i = cin_skipcomment(l.add(i)).offset_from(l) as usize;
            if crate::memory::cstr_byte_at(l, i) == 0 {
                break;
            }
            i = skip_string(l.add(i)).offset_from(l) as usize;
            if *l.add(i) == start {
                open_count += 1;
            } else if *l.add(i) == end {
                if open_count > 0 {
                    open_count -= 1;
                } else {
                    curwin_mut().w_cursor.col = i as Colnr;
                    retval = true;
                }
            }
            i += 1;
        }
        retval
    }
}
/// Parse 'cinoptions' and set the values in "buf".
pub fn parse_cino(buf: &mut FileBuf) {
    unsafe {
        let mut fraction: c_int = 0;
        let sw = get_sw_value(&mut *buf);
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
        while !crate::memory::cstr_is_empty(p) {
            let l = p;
            p = p.add(1);
            if crate::memory::cstr_first_byte(p) == b'-' {
                p = p.add(1);
            }
            let digits_start = p;
            let mut n: i64 = getdigits_int(&raw mut p, true, 0) as i64;
            let mut divider: c_int = 0;
            if crate::memory::cstr_first_byte(p) == b'.' {
                p = p.add(1);
                fraction =
                    crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(p)) as c_int;
                while ascii_isdigit(crate::memory::cstr_first_char(p)) {
                    p = p.add(1);
                    if divider != 0 {
                        divider *= 10;
                    } else {
                        divider = 10;
                    }
                }
            }
            if crate::memory::cstr_first_byte(p) == b's' {
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
            if crate::memory::cstr_byte_at(l, 1) == b'-' {
                n = -n;
            }
            let val = trim_to_int(n);
            match crate::memory::cstr_first_byte(l) {
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
            if crate::memory::cstr_first_byte(p) == b',' {
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
        let mut col: Colnr = 0;
        let mut trypos: *mut CursorPos;
        let mut comment_pos: *mut CursorPos;
        let mut trypos_brace: *mut CursorPos = ptr::null_mut();
        let mut trypos_copy: CursorPos;
        let mut our_paren_pos: CursorPos;
        let start: *mut c_char;
        let start_brace: c_int;
        let mut ourscope: Linenr;
        let mut l: *const c_char = std::ptr::null();
        let mut look: *const c_char = std::ptr::null();
        let mut terminated: c_char;
        let mut lookfor: c_int;
        let mut whilelevel: c_int;
        let mut lnum: Linenr;
        let mut n: c_int;
        let mut lookfor_break: bool;
        let mut lookfor_cpp_namespace = false;
        let mut cont_amount: c_int = 0;
        let mut added_to_amount: c_int = 0;
        let mut raw_string_start: Linenr = 0;
        let mut cache_cpp_baseclass = CppBaseclassCache {
            found: 0,
            lpos: Lpos {
                lnum: MAXLNUM,
                col: 0,
            },
        };
        let mut ind_continuation = curbuf_mut().b_ind_continuation;
        let cur_curpos = curwin_mut().w_cursor;
        if cur_curpos.lnum == 1 {
            return 0;
        }
        let mut linecopy_bytes =
            crate::memory::cstr_to_cstring(ml_get(cur_curpos.lnum)).into_bytes_with_nul();
        let linecopy = linecopy_bytes.as_mut_ptr().cast::<c_char>();
        if (g_st().state_global & MODE_INSERT) != 0
            && (curwin_mut().w_cursor.col as usize) < crate::memory::cstr_len(linecopy)
            && *linecopy.add(curwin_mut().w_cursor.col as usize) == b')' as c_char
        {
            *linecopy.add(curwin_mut().w_cursor.col as usize) = NUL;
        }
        let theline = skipwhite(linecopy);
        curwin_mut().w_cursor.col = 0;
        let original_line_islabel = cin_islabel();
        comment_pos = ind_find_start_comment();
        if !comment_pos.is_null() {
            trypos_copy = *comment_pos;
            comment_pos = &raw mut trypos_copy;
        }
        trypos = find_start_rawstring(curbuf_mut().b_ind_maxcomment);
        'laterend: {
            if !trypos.is_null() && (comment_pos.is_null() || lt(*trypos, *comment_pos)) {
                amount = -1;
                break 'laterend;
            }
            'theend: {
                if crate::memory::cstr_first_byte(theline) == b'#'
                    && (crate::memory::cstr_first_byte(linecopy) == b'#'
                        || in_cinkeys(b'#' as c_int, b' ' as c_int, true))
                {
                    let directive = skipwhite(theline.add(1));
                    if curbuf_mut().b_ind_pragma == 0
                        || !(crate::memory::cstr_prefix_bytes(directive, 6) == &b"pragma"[..6])
                    {
                        amount = curbuf_mut().b_ind_hash_comment;
                        break 'theend;
                    }
                }
                if original_line_islabel
                    && curbuf_mut().b_ind_js == 0
                    && curbuf_mut().b_ind_jump_label < 0
                {
                    amount = 0;
                    break 'theend;
                }
                if cin_islinecomment(theline) {
                    let mut linecomment_pos = CursorPos {
                        lnum: 0,
                        col: 0,
                        coladd: 0,
                    };
                    trypos = find_line_comment();
                    if trypos.is_null() && curwin_mut().w_cursor.lnum > 1 {
                        linecomment_pos.col =
                            check_linecomment(ml_get(curwin_mut().w_cursor.lnum - 1));
                        if linecomment_pos.col != MAXCOL {
                            trypos = &raw mut linecomment_pos;
                            (*trypos).lnum = curwin_mut().w_cursor.lnum - 1;
                        }
                    }
                    if !trypos.is_null() {
                        getvcol(curwin_mut(), &mut *trypos, Some(&mut col), None, None, 0);
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
                        curwin_mut(),
                        &mut *comment_pos,
                        Some(&mut col),
                        None,
                        None,
                        0,
                    );
                    amount = col;
                    lead_start[0] = NUL;
                    lead_middle[0] = NUL;
                    let mut p = curbuf_mut().b_p_com;
                    while !crate::memory::cstr_is_empty(p) {
                        let mut align: c_int = 0;
                        let mut off: c_int = 0;
                        let mut what: c_int = 0;
                        while !crate::memory::cstr_is_empty(p)
                            && crate::memory::cstr_first_byte(p) != b':'
                        {
                            if *p == COM_START || *p == COM_END || *p == COM_MIDDLE {
                                what = crate::memory::cstr_first_byte(p) as c_int;
                                p = p.add(1);
                            } else if *p == COM_LEFT || *p == COM_RIGHT {
                                align = crate::memory::cstr_first_byte(p) as c_int;
                                p = p.add(1);
                            } else if ascii_isdigit(crate::memory::cstr_first_char(p))
                                || crate::memory::cstr_first_byte(p) == b'-'
                            {
                                off = getdigits_int(&raw mut p, true, 0);
                            } else {
                                p = p.add(1);
                            }
                        }
                        if crate::memory::cstr_first_byte(p) == b':' {
                            p = p.add(1);
                        }
                        lead_end_len = copy_option_part(
                            &raw mut p,
                            (&raw mut lead_end).cast(),
                            COM_MAX_LEN,
                            c",",
                        ) as c_int;
                        if what == COM_START as u8 as c_int {
                            crate::memory::cstr_cpy(
                                (&raw mut lead_start).cast(),
                                (&raw const lead_end).cast(),
                            );
                            lead_start_len = lead_end_len;
                            start_off = off;
                            start_align = align;
                        } else if what == COM_MIDDLE as u8 as c_int {
                            crate::memory::cstr_cpy(
                                (&raw mut lead_middle).cast(),
                                (&raw const lead_end).cast(),
                            );
                            lead_middle_len = lead_end_len;
                        } else if what == COM_END as u8 as c_int {
                            if (crate::memory::cstr_prefix_bytes(theline, lead_middle_len as usize)
                                == crate::memory::cstr_prefix_bytes(
                                    (&raw const lead_middle).cast::<c_char>(),
                                    lead_middle_len as usize,
                                ))
                                && (crate::memory::cstr_prefix_bytes(
                                    theline,
                                    lead_end_len as usize,
                                ) != crate::memory::cstr_prefix_bytes(
                                    (&raw const lead_end).cast::<c_char>(),
                                    lead_end_len as usize,
                                ))
                            {
                                done = true;
                                if curwin_mut().w_cursor.lnum > 1 {
                                    look = skipwhite(ml_get(curwin_mut().w_cursor.lnum - 1));
                                    if crate::memory::cstr_prefix_bytes(
                                        look,
                                        lead_start_len as usize,
                                    ) == crate::memory::cstr_prefix_bytes(
                                        (&raw const lead_start).cast::<c_char>(),
                                        lead_start_len as usize,
                                    ) {
                                        amount = get_indent_lnum(curwin_mut().w_cursor.lnum - 1);
                                    } else if crate::memory::cstr_prefix_bytes(
                                        look,
                                        lead_middle_len as usize,
                                    ) == crate::memory::cstr_prefix_bytes(
                                        (&raw const lead_middle).cast::<c_char>(),
                                        lead_middle_len as usize,
                                    ) {
                                        amount = get_indent_lnum(curwin_mut().w_cursor.lnum - 1);
                                        break;
                                    } else if crate::memory::cstr_prefix_bytes(
                                        ml_get((*comment_pos).lnum)
                                            .add((*comment_pos).col as usize),
                                        lead_start_len as usize,
                                    ) != crate::memory::cstr_prefix_bytes(
                                        (&raw const lead_start).cast::<c_char>(),
                                        lead_start_len as usize,
                                    ) {
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
                            if (crate::memory::cstr_prefix_bytes(theline, lead_middle_len as usize)
                                != crate::memory::cstr_prefix_bytes(
                                    (&raw const lead_middle).cast::<c_char>(),
                                    lead_middle_len as usize,
                                ))
                                && (crate::memory::cstr_prefix_bytes(
                                    theline,
                                    lead_end_len as usize,
                                ) == crate::memory::cstr_prefix_bytes(
                                    (&raw const lead_end).cast::<c_char>(),
                                    lead_end_len as usize,
                                ))
                            {
                                amount = get_indent_lnum(curwin_mut().w_cursor.lnum - 1);
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
                    } else if crate::memory::cstr_first_byte(theline) == b'*' {
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
                            if curbuf_mut().b_ind_in_comment2 == 0 {
                                start = ml_get((*comment_pos).lnum);
                                look = start.add((*comment_pos).col as usize + 2);
                                if !crate::memory::cstr_is_empty(look) {
                                    (*comment_pos).col =
                                        skipwhite(look).offset_from(start) as Colnr;
                                }
                            }
                            getvcol(
                                curwin_mut(),
                                &mut *comment_pos,
                                Some(&mut col),
                                None,
                                None,
                                0,
                            );
                            amount = col;
                            if curbuf_mut().b_ind_in_comment2 != 0
                                || crate::memory::cstr_is_empty(look)
                            {
                                amount += curbuf_mut().b_ind_in_comment;
                            }
                        }
                    }
                    break 'theend;
                }
                if *skipwhite(theline) == b']' as c_char && {
                    trypos = find_match_char(b'[' as c_char, curbuf_mut().b_ind_maxparen);
                    !trypos.is_null()
                } {
                    amount = get_indent_lnum((*trypos).lnum);
                    break 'theend;
                }
                if ({
                    trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                    !trypos.is_null() && curbuf_mut().b_ind_java == 0
                }) || ({
                    trypos_brace = find_start_brace();
                    !trypos_brace.is_null()
                }) || !trypos.is_null()
                {
                    if !trypos.is_null() && !trypos_brace.is_null() {
                        if if (*trypos).lnum != (*trypos_brace).lnum {
                            (*trypos).lnum < (*trypos_brace).lnum
                        } else {
                            (*trypos).col < (*trypos_brace).col
                        } {
                            trypos = ptr::null_mut();
                        } else {
                            trypos_brace = ptr::null_mut();
                        }
                    }
                    if !trypos.is_null() {
                        our_paren_pos = *trypos;
                        if crate::memory::cstr_first_byte(theline) == b')'
                            && curbuf_mut().b_ind_paren_prev != 0
                        {
                            amount = get_indent_lnum(curwin_mut().w_cursor.lnum - 1);
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
                                curwin_mut().w_cursor.lnum = lnum;
                                trypos = ind_find_start_cors(None);
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
                                    if crate::memory::cstr_first_byte(theline) == b')' {
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
                            if curbuf_mut().b_ind_if_for_while != 0 {
                                let cursor_save = curwin_mut().w_cursor;
                                let mut outermost: CursorPos;
                                trypos = &raw mut our_paren_pos;
                                loop {
                                    outermost = *trypos;
                                    curwin_mut().w_cursor.lnum = outermost.lnum;
                                    curwin_mut().w_cursor.col = outermost.col;
                                    trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                                    if trypos.is_null() || (*trypos).lnum != outermost.lnum {
                                        break;
                                    }
                                }
                                curwin_mut().w_cursor = cursor_save;
                                let line = ml_get(outermost.lnum);
                                is_if_for_while =
                                    cin_is_if_for_while_before_offset(line, &mut outermost.col);
                            }
                            amount = skip_label(our_paren_pos.lnum, &raw mut look);
                            look = skipwhite(look);
                            if crate::memory::cstr_first_byte(look) == b'(' {
                                let save_lnum = curwin_mut().w_cursor.lnum;
                                curwin_mut().w_cursor.lnum = our_paren_pos.lnum;
                                let line: *const c_char = get_cursor_line_ptr();
                                let look_col = look.offset_from(line) as Colnr;
                                curwin_mut().w_cursor.col = look_col + 1;
                                trypos = findmatchlimit(
                                    None,
                                    b')' as c_int,
                                    FM_SKIPCOMM,
                                    curbuf_mut().b_ind_maxparen as i64,
                                );
                                if !trypos.is_null()
                                    && (*trypos).lnum == our_paren_pos.lnum
                                    && (*trypos).col < our_paren_pos.col
                                {
                                    ignore_paren_col = (*trypos).col + 1;
                                }
                                curwin_mut().w_cursor.lnum = save_lnum;
                                look = ml_get(our_paren_pos.lnum).add(look_col as usize);
                            }
                            if crate::memory::cstr_first_byte(theline) == b')'
                                || (curbuf_mut().b_ind_unclosed == 0 && is_if_for_while == 0)
                                || (curbuf_mut().b_ind_unclosed_noignore == 0
                                    && crate::memory::cstr_first_byte(look) == b'('
                                    && ignore_paren_col == 0)
                            {
                                if crate::memory::cstr_first_byte(theline) != b')' {
                                    cur_amount = MAXCOL;
                                    l = ml_get(our_paren_pos.lnum);
                                    if curbuf_mut().b_ind_unclosed_wrapped != 0
                                        && cin_ends_in(l, c"(")
                                    {
                                        n = 1;
                                        col = 0;
                                        while col < our_paren_pos.col {
                                            match crate::memory::cstr_byte_at(l, col as usize) {
                                                b'(' | b'{' => n += 1,
                                                b')' | b'}' if n > 1 => n -= 1,
                                                _ => {}
                                            }
                                            col += 1;
                                        }
                                        our_paren_pos.col = 0;
                                        amount += n * curbuf_mut().b_ind_unclosed_wrapped;
                                    } else if curbuf_mut().b_ind_unclosed_whiteok != 0 {
                                        our_paren_pos.col += 1;
                                    } else {
                                        col = our_paren_pos.col + 1;
                                        while ascii_iswhite(crate::memory::cstr_char_at(
                                            l,
                                            col as usize,
                                        )) {
                                            col += 1;
                                        }
                                        if crate::memory::cstr_byte_at(l, col as usize) != 0 {
                                            our_paren_pos.col = col;
                                        } else {
                                            our_paren_pos.col += 1;
                                        }
                                    }
                                }
                                if our_paren_pos.col > 0 {
                                    getvcol(
                                        curwin_mut(),
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
                            if crate::memory::cstr_first_byte(theline) == b')'
                                && curbuf_mut().b_ind_matching_paren != 0
                            {
                                // Line up with the start of the matching paren line.
                            } else if (curbuf_mut().b_ind_unclosed == 0 && is_if_for_while == 0)
                                || (curbuf_mut().b_ind_unclosed_noignore == 0
                                    && crate::memory::cstr_first_byte(look) == b'('
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
                                            amount += curbuf_mut().b_ind_unclosed2;
                                            col = our_paren_pos.col;
                                        }
                                        b')' => {
                                            amount -= curbuf_mut().b_ind_unclosed2;
                                            col = MAXCOL;
                                        }
                                        _ => {}
                                    }
                                }
                                if col == MAXCOL {
                                    amount += curbuf_mut().b_ind_unclosed;
                                } else {
                                    curwin_mut().w_cursor.lnum = our_paren_pos.lnum;
                                    curwin_mut().w_cursor.col = col;
                                    if !find_match_paren_after_brace(curbuf_mut().b_ind_maxparen)
                                        .is_null()
                                    {
                                        amount += curbuf_mut().b_ind_unclosed2;
                                    } else if is_if_for_while != 0 {
                                        amount += curbuf_mut().b_ind_if_for_while;
                                    } else {
                                        amount += curbuf_mut().b_ind_unclosed;
                                    }
                                }
                                if cur_amount < amount {
                                    amount = cur_amount;
                                }
                            }
                        }
                        if cin_iscomment(theline) {
                            amount += curbuf_mut().b_ind_comment;
                        }
                    } else {
                        trypos_copy = *trypos_brace;
                        trypos_brace = &raw mut trypos_copy;
                        trypos = trypos_brace;
                        ourscope = (*trypos).lnum;
                        start = ml_get(ourscope);
                        look = skipwhite(start);
                        if crate::memory::cstr_first_byte(look) == b'{' {
                            getvcol(curwin_mut(), &mut *trypos, Some(&mut col), None, None, 0);
                            amount = col;
                            if crate::memory::cstr_first_byte(start) == b'{' {
                                start_brace = BRACE_IN_COL0;
                            } else {
                                start_brace = BRACE_AT_START;
                            }
                        } else {
                            curwin_mut().w_cursor.lnum = ourscope;
                            lnum = ourscope;
                            if find_last_paren(start, b'(' as c_char, b')' as c_char) {
                                trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                                if !trypos.is_null() {
                                    lnum = (*trypos).lnum;
                                }
                            }
                            if (curbuf_mut().b_ind_js != 0
                                || curbuf_mut().b_ind_keep_case_label != 0)
                                && cin_iscase(skipwhite(get_cursor_line_ptr()), false)
                            {
                                amount = get_indent();
                            } else if curbuf_mut().b_ind_js != 0 {
                                amount = get_indent_lnum(lnum);
                            } else {
                                amount = skip_label(lnum, &raw mut l);
                            }
                            start_brace = BRACE_AT_END;
                        }
                        let mut js_cur_has_key = if curbuf_mut().b_ind_js != 0 {
                            cin_has_js_key(theline)
                        } else {
                            false
                        };
                        if crate::memory::cstr_first_byte(theline) == b'}' {
                            amount += curbuf_mut().b_ind_close_extra;
                        } else {
                            lookfor = LOOKFOR_INITIAL;
                            if cin_iselse(theline) {
                                lookfor = LOOKFOR_IF;
                            } else if cin_iswhileofdo(theline, cur_curpos.lnum) {
                                lookfor = LOOKFOR_DO;
                            }
                            if lookfor != LOOKFOR_INITIAL {
                                curwin_mut().w_cursor.lnum = cur_curpos.lnum;
                                if find_match(lookfor, ourscope) {
                                    amount = get_indent();
                                    break 'theend;
                                }
                            }
                            if start_brace == BRACE_IN_COL0 {
                                amount = curbuf_mut().b_ind_open_left_imag;
                                lookfor_cpp_namespace = true;
                            } else if start_brace == BRACE_AT_START && lookfor_cpp_namespace {
                                lookfor_cpp_namespace = true;
                            } else if start_brace == BRACE_AT_END {
                                amount += curbuf_mut().b_ind_open_imag;
                                l = skipwhite(get_cursor_line_ptr());
                                if cin_is_cpp_namespace(l) {
                                    amount += curbuf_mut().b_ind_cpp_namespace;
                                } else if cin_is_cpp_extern_c(l) {
                                    amount += curbuf_mut().b_ind_cpp_extern_c;
                                }
                            } else {
                                amount -= curbuf_mut().b_ind_open_extra;
                                if amount < 0 {
                                    amount = 0;
                                }
                            }
                            lookfor_break = false;
                            if cin_iscase(theline, false) {
                                lookfor = LOOKFOR_CASE;
                                amount += curbuf_mut().b_ind_case;
                            } else if cin_isscopedecl(theline) {
                                lookfor = LOOKFOR_SCOPEDECL;
                                amount += curbuf_mut().b_ind_scopedecl;
                            } else {
                                if curbuf_mut().b_ind_case_break != 0 && cin_isbreak(theline) {
                                    lookfor_break = true;
                                }
                                lookfor = LOOKFOR_INITIAL;
                                amount += curbuf_mut().b_ind_level;
                            }
                            scope_amount = amount;
                            whilelevel = 0;
                            curwin_mut().w_cursor = cur_curpos;
                            'scope_loop: loop {
                                curwin_mut().w_cursor.lnum -= 1;
                                curwin_mut().w_cursor.col = 0;
                                if curwin_mut().w_cursor.lnum <= ourscope {
                                    if lookfor == LOOKFOR_ENUM_OR_INIT {
                                        if curwin_mut().w_cursor.lnum == 0
                                            || curwin_mut().w_cursor.lnum
                                                < ourscope - curbuf_mut().b_ind_maxparen
                                        {
                                            if cont_amount > 0 {
                                                amount = cont_amount;
                                            } else if curbuf_mut().b_ind_js == 0 {
                                                amount += ind_continuation;
                                            }
                                            break;
                                        }
                                        trypos = ind_find_start_cors(None);
                                        if !trypos.is_null() {
                                            curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                                            curwin_mut().w_cursor.col = 0;
                                            continue;
                                        }
                                        l = get_cursor_line_ptr();
                                        if cin_ispreproc_cont(
                                            &raw mut l,
                                            &mut curwin_mut().w_cursor.lnum,
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
                                                curwin_mut().w_cursor.lnum,
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
                                                trypos =
                                                    find_match_paren(curbuf_mut().b_ind_maxparen);
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
                                                curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                                                curwin_mut().w_cursor.col = 0;
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
                                            if crate::memory::cstr_first_byte(theline) == b'{' {
                                                amount += curbuf_mut().b_ind_open_extra;
                                                added_to_amount = curbuf_mut().b_ind_open_extra;
                                            }
                                        }
                                        if lookfor_cpp_namespace {
                                            if curwin_mut().w_cursor.lnum == ourscope {
                                                continue;
                                            }
                                            if curwin_mut().w_cursor.lnum == 0
                                                || curwin_mut().w_cursor.lnum
                                                    < ourscope - FIND_NAMESPACE_LIM
                                            {
                                                break;
                                            }
                                            trypos = ind_find_start_cors(None);
                                            if !trypos.is_null() {
                                                curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                                                curwin_mut().w_cursor.col = 0;
                                                continue;
                                            }
                                            l = get_cursor_line_ptr();
                                            if cin_ispreproc_cont(
                                                &raw mut l,
                                                &mut curwin_mut().w_cursor.lnum,
                                                &mut amount,
                                            ) {
                                                continue;
                                            }
                                            if cin_is_cpp_namespace(l) {
                                                amount += curbuf_mut().b_ind_cpp_namespace
                                                    - added_to_amount;
                                                break;
                                            } else if cin_is_cpp_extern_c(l) {
                                                amount += curbuf_mut().b_ind_cpp_extern_c
                                                    - added_to_amount;
                                                break;
                                            }
                                            if cin_nocode(l) {
                                                continue;
                                            }
                                        }
                                    }
                                    break;
                                }
                                trypos = ind_find_start_cors(Some(&mut raw_string_start));
                                if !trypos.is_null() {
                                    curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                                    curwin_mut().w_cursor.col = 0;
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
                                    n = get_indent_nolabel(curwin_mut().w_cursor.lnum);
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
                                            if crate::memory::cstr_first_byte(theline) == b'{' {
                                                amount += curbuf_mut().b_ind_open_extra;
                                            } else {
                                                amount += curbuf_mut().b_ind_level
                                                    + curbuf_mut().b_ind_no_brace;
                                            }
                                        }
                                        break;
                                    }
                                    scope_amount = get_indent()
                                        + if iscase {
                                            curbuf_mut().b_ind_case_code
                                        } else {
                                            curbuf_mut().b_ind_scopedecl_code
                                        };
                                    lookfor = if curbuf_mut().b_ind_case_break != 0 {
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
                                            curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                                            curwin_mut().w_cursor.col = 0;
                                        }
                                    }
                                    continue;
                                }
                                if curbuf_mut().b_ind_js == 0 && cin_islabel() {
                                    l = after_label(get_cursor_line_ptr());
                                    if l.is_null() || cin_nocode(l) {
                                        continue;
                                    }
                                }
                                l = get_cursor_line_ptr();
                                if cin_ispreproc_cont(
                                    &raw mut l,
                                    &mut curwin_mut().w_cursor.lnum,
                                    &mut amount,
                                ) || cin_nocode(l)
                                {
                                    continue;
                                }
                                n = 0;
                                if lookfor != LOOKFOR_TERM && curbuf_mut().b_ind_cpp_baseclass > 0 {
                                    n = c_int::from(cin_is_cpp_baseclass(&mut cache_cpp_baseclass));
                                    l = get_cursor_line_ptr();
                                }
                                if n != 0 {
                                    if lookfor == LOOKFOR_UNTERM {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                    } else if crate::memory::cstr_first_byte(theline) == b'{' {
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
                                    if curbuf_mut().b_ind_js != 0 && terminated == b',' as c_char {
                                        lookfor = LOOKFOR_JS_KEY;
                                    }
                                }
                                if lookfor == LOOKFOR_JS_KEY && cin_has_js_key(l) {
                                    amount = get_indent();
                                    break;
                                }
                                if lookfor == LOOKFOR_COMMA {
                                    if !trypos_brace.is_null()
                                        && (*trypos_brace).lnum >= curwin_mut().w_cursor.lnum
                                    {
                                        break;
                                    }
                                    if terminated == b',' as c_char {
                                        break;
                                    }
                                    amount = get_indent();
                                    if curwin_mut().w_cursor.lnum - 1 == ourscope {
                                        break;
                                    }
                                }
                                if terminated == 0
                                    || (lookfor != LOOKFOR_UNTERM && terminated == b',' as c_char)
                                {
                                    if lookfor != LOOKFOR_ENUM_OR_INIT
                                        && (*skipwhite(l) == b'[' as c_char
                                            || (!crate::memory::cstr_is_empty(l)
                                                && *l.add(crate::memory::cstr_len(l) - 1)
                                                    == b'[' as c_char))
                                    {
                                        amount += ind_continuation;
                                    }
                                    let _ = find_last_paren(l, b'(' as c_char, b')' as c_char);
                                    trypos = find_match_paren(corr_ind_maxparen(&cur_curpos));
                                    if !trypos.is_null()
                                        && ((*trypos).lnum < (*trypos_brace).lnum
                                            || ((*trypos).lnum == (*trypos_brace).lnum
                                                && (*trypos).col < (*trypos_brace).col))
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
                                        curwin_mut().w_cursor = *trypos;
                                        l = get_cursor_line_ptr();
                                        if cin_iscase(l, false) || cin_isscopedecl(l) {
                                            curwin_mut().w_cursor.lnum += 1;
                                            curwin_mut().w_cursor.col = 0;
                                            continue;
                                        }
                                    }
                                    if terminated == b',' as c_char {
                                        while curwin_mut().w_cursor.lnum > 1 {
                                            l = ml_get(curwin_mut().w_cursor.lnum - 1);
                                            if crate::memory::cstr_is_empty(l)
                                                || *l.add(crate::memory::cstr_len(l) - 1)
                                                    != b'\\' as c_char
                                            {
                                                break;
                                            }
                                            curwin_mut().w_cursor.lnum -= 1;
                                            curwin_mut().w_cursor.col = 0;
                                        }
                                        l = get_cursor_line_ptr();
                                    }
                                    if curbuf_mut().b_ind_js != 0 {
                                        cur_amount = get_indent();
                                    } else {
                                        cur_amount =
                                            skip_label(curwin_mut().w_cursor.lnum, &raw mut l);
                                    }
                                    if terminated != b',' as c_char
                                        && lookfor != LOOKFOR_TERM
                                        && crate::memory::cstr_first_byte(theline) == b'{'
                                    {
                                        amount = cur_amount;
                                        if *skipwhite(l) != b'{' as c_char {
                                            amount += curbuf_mut().b_ind_open_extra;
                                        }
                                        if curbuf_mut().b_ind_cpp_baseclass != 0
                                            && curbuf_mut().b_ind_js == 0
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
                                        if crate::memory::cstr_first_byte(theline) == b'{' {
                                            amount += curbuf_mut().b_ind_open_extra;
                                        }
                                        if lookfor != LOOKFOR_TERM {
                                            amount += curbuf_mut().b_ind_level
                                                + curbuf_mut().b_ind_no_brace;
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
                                            if crate::memory::cstr_first_byte(l) == b'}' {
                                                curwin_mut().w_cursor.col =
                                                    l.offset_from(get_cursor_line_ptr()) as Colnr
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
                                            if curbuf_mut().b_ind_cpp_baseclass == 0 {
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
                                        if curbuf_mut().b_ind_js != 0
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
                                            if curbuf_mut().b_ind_js != 0 {
                                                if cin_iscomment(skipwhite(l)) {
                                                    break;
                                                }
                                                lookfor = LOOKFOR_COMMA;
                                                trypos = find_match_char(
                                                    b'[' as c_char,
                                                    curbuf_mut().b_ind_maxparen,
                                                );
                                                if !trypos.is_null() {
                                                    let trypos = { &mut *trypos };
                                                    if trypos.lnum == curwin_mut().w_cursor.lnum - 1
                                                    {
                                                        break;
                                                    }
                                                    ourscope = trypos.lnum;
                                                }
                                            } else {
                                                lookfor = LOOKFOR_ENUM_OR_INIT;
                                                cont_amount = cin_first_id_amount();
                                            }
                                        } else {
                                            if lookfor == LOOKFOR_INITIAL
                                                && !crate::memory::cstr_is_empty(l)
                                                && *l.add(crate::memory::cstr_len(l) - 1)
                                                    == b'\\' as c_char
                                            {
                                                cont_amount = cin_get_equal_amount(
                                                    curwin_mut().w_cursor.lnum,
                                                );
                                            }
                                            if lookfor != LOOKFOR_TERM
                                                && lookfor != LOOKFOR_JS_KEY
                                                && lookfor != LOOKFOR_COMMA
                                                && raw_string_start != curwin_mut().w_cursor.lnum
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
                                        if crate::memory::cstr_first_byte(theline) == b'{' {
                                            amount += curbuf_mut().b_ind_open_extra;
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
                                                trypos =
                                                    find_match_paren(curbuf_mut().b_ind_maxparen);
                                                if !trypos.is_null() {
                                                    curwin_mut().w_cursor = *trypos;
                                                    l = get_cursor_line_ptr();
                                                    if cin_iscase(l, false) || cin_isscopedecl(l) {
                                                        curwin_mut().w_cursor.lnum += 1;
                                                        curwin_mut().w_cursor.col = 0;
                                                        continue 'scope_loop;
                                                    }
                                                }
                                            }
                                            iscase = curbuf_mut().b_ind_keep_case_label != 0
                                                && cin_iscase(l, false);
                                            amount =
                                                skip_label(curwin_mut().w_cursor.lnum, &raw mut l);
                                            if crate::memory::cstr_first_byte(theline) == b'{' {
                                                amount += curbuf_mut().b_ind_open_extra;
                                            }
                                            l = skipwhite(l);
                                            if crate::memory::cstr_first_byte(l) == b'{' {
                                                amount -= curbuf_mut().b_ind_open_extra;
                                            }
                                            lookfor =
                                                if iscase { LOOKFOR_ANY } else { LOOKFOR_TERM };
                                            if lookfor == LOOKFOR_TERM
                                                && crate::memory::cstr_first_byte(l) != b'}'
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
                                                    curwin_mut().w_cursor = *trypos;
                                                    l = cin_skipcomment(get_cursor_line_ptr());
                                                    if crate::memory::cstr_first_byte(l) == b'}'
                                                        || !cin_iselse(l)
                                                    {
                                                        continue;
                                                    }
                                                    curwin_mut().w_cursor.lnum += 1;
                                                    curwin_mut().w_cursor.col = 0;
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
                        amount += curbuf_mut().b_ind_comment;
                    }
                    if curbuf_mut().b_ind_jump_label > 0 && original_line_islabel {
                        amount -= curbuf_mut().b_ind_jump_label;
                    }
                    break 'theend;
                }
                if crate::memory::cstr_first_byte(theline) == b'{' {
                    amount = curbuf_mut().b_ind_first_open;
                    break 'theend;
                }
                if cur_curpos.lnum < curbuf_mut().b_ml.ml_line_count
                    && !cin_nocode(theline)
                    && vim_strchr(theline, b'{' as c_int).is_null()
                    && vim_strchr(theline, b'}' as c_int).is_null()
                    && !cin_ends_in(theline, c":")
                    && !cin_ends_in(theline, c",")
                    && cin_isfuncdecl(ptr::null_mut(), cur_curpos.lnum + 1, cur_curpos.lnum + 1)
                    && cin_isterminated(theline, false, true) == 0
                {
                    amount = curbuf_mut().b_ind_func_type;
                    break 'theend;
                }
                amount = 0;
                curwin_mut().w_cursor = cur_curpos;
                while curwin_mut().w_cursor.lnum > 1 {
                    curwin_mut().w_cursor.lnum -= 1;
                    curwin_mut().w_cursor.col = 0;
                    l = get_cursor_line_ptr();
                    trypos = ind_find_start_cors(None);
                    if !trypos.is_null() {
                        curwin_mut().w_cursor.lnum = (*trypos).lnum + 1;
                        curwin_mut().w_cursor.col = 0;
                        continue;
                    }
                    n = 0;
                    if curbuf_mut().b_ind_cpp_baseclass != 0 {
                        n = c_int::from(cin_is_cpp_baseclass(&mut cache_cpp_baseclass));
                        l = get_cursor_line_ptr();
                    }
                    if n != 0 {
                        amount = get_baseclass_amount(cache_cpp_baseclass.lpos.col);
                        break;
                    }
                    if cin_ispreproc_cont(&raw mut l, &mut curwin_mut().w_cursor.lnum, &mut amount)
                    {
                        continue;
                    }
                    if cin_nocode(l) {
                        continue;
                    }
                    if cin_ends_in(l, c",")
                        || (!crate::memory::cstr_is_empty(l) && {
                            n = *l.add(crate::memory::cstr_len(l) - 1) as u8 as c_int;
                            n == b'\\' as c_int
                        })
                    {
                        if find_last_paren(l, b'(' as c_char, b')' as c_char) {
                            trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                            if !trypos.is_null() {
                                curwin_mut().w_cursor = *trypos;
                            }
                        }
                        while n == 0 && curwin_mut().w_cursor.lnum > 1 {
                            l = ml_get(curwin_mut().w_cursor.lnum - 1);
                            if crate::memory::cstr_is_empty(l)
                                || *l.add(crate::memory::cstr_len(l) - 1) != b'\\' as c_char
                            {
                                break;
                            }
                            curwin_mut().w_cursor.lnum -= 1;
                            curwin_mut().w_cursor.col = 0;
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
                    if cin_ends_in(l, c"};") {
                        break;
                    }
                    if cin_ends_in(l, c"[") {
                        amount = get_indent() + ind_continuation;
                        break;
                    }
                    look = skipwhite(l);
                    if crate::memory::cstr_first_byte(look) == b';' && cin_nocode(look.add(1)) {
                        let curpos_save = curwin_mut().w_cursor;
                        while curwin_mut().w_cursor.lnum > 1 {
                            curwin_mut().w_cursor.lnum -= 1;
                            look = ml_get(curwin_mut().w_cursor.lnum);
                            if !(cin_nocode(look)
                                || cin_ispreproc_cont(
                                    &raw mut look,
                                    &mut curwin_mut().w_cursor.lnum,
                                    &mut amount,
                                ))
                            {
                                break;
                            }
                        }
                        if curwin_mut().w_cursor.lnum > 0 && cin_ends_in(look, c"}") {
                            break;
                        }
                        curwin_mut().w_cursor = curpos_save;
                    }
                    if cin_isfuncdecl(&raw mut l, curwin_mut().w_cursor.lnum, 0) {
                        amount = curbuf_mut().b_ind_param;
                        break;
                    }
                    if cin_ends_in(l, c";") {
                        l = ml_get(curwin_mut().w_cursor.lnum - 1);
                        if cin_ends_in(l, c",")
                            || (!crate::memory::cstr_is_empty(l)
                                && *l.add(crate::memory::cstr_len(l) - 1) == b'\\' as c_char)
                        {
                            break;
                        }
                        l = get_cursor_line_ptr();
                    }
                    let _ = find_last_paren(l, b'(' as c_char, b')' as c_char);
                    trypos = find_match_paren(curbuf_mut().b_ind_maxparen);
                    if !trypos.is_null() {
                        curwin_mut().w_cursor = *trypos;
                    }
                    amount = get_indent();
                    break;
                }
                if cin_iscomment(theline) {
                    amount += curbuf_mut().b_ind_comment;
                }
                if cur_curpos.lnum > 1 {
                    l = ml_get(cur_curpos.lnum - 1);
                    if !crate::memory::cstr_is_empty(l)
                        && *l.add(crate::memory::cstr_len(l) - 1) == b'\\' as c_char
                    {
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
        curwin_mut().w_cursor = cur_curpos;
        amount
    }
}
fn find_match(lookfor: c_int, ourscope: Linenr) -> bool {
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
        curwin_mut().w_cursor.col = 0;
        while curwin_mut().w_cursor.lnum > ourscope + 1 {
            curwin_mut().w_cursor.lnum -= 1;
            curwin_mut().w_cursor.col = 0;
            let mut look = cin_skipcomment(get_cursor_line_ptr());
            if !cin_iselse(look)
                && !cin_isif(look)
                && !cin_isdo(look)
                && !cin_iswhileofdo(look, curwin_mut().w_cursor.lnum)
            {
                continue;
            }
            let theirscope = find_start_brace();
            if theirscope.is_null() {
                break;
            }
            let theirscope = { &mut *theirscope };
            if theirscope.lnum < ourscope {
                break;
            }
            if theirscope.lnum > ourscope {
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
            if cin_iswhileofdo(look, curwin_mut().w_cursor.lnum) {
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
        let mut look: *const c_char = if curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST {
            curbuf_mut().b_p_indk
        } else {
            curbuf_mut().b_p_cink
        };
        while !crate::memory::cstr_is_empty(look) {
            let mut try_match = match when as u8 {
                b'*' => crate::memory::cstr_first_byte(look) == b'*',
                b'!' => crate::memory::cstr_first_byte(look) == b'!',
                _ => crate::memory::cstr_first_byte(look) != b'*',
            };
            if crate::memory::cstr_first_byte(look) == b'*'
                || crate::memory::cstr_first_byte(look) == b'!'
            {
                look = look.add(1);
            }
            let try_match_word = if crate::memory::cstr_first_byte(look) == b'0' {
                let tmw = try_match;
                if !line_is_empty {
                    try_match = false;
                }
                look = look.add(1);
                tmw
            } else {
                false
            };
            if crate::memory::cstr_first_byte(look) == b'^'
                && *look.add(1) >= b'?' as c_char
                && *look.add(1) <= b'_' as c_char
            {
                if try_match
                    && keytyped == ctrl_chr(c_int::from(crate::memory::cstr_byte_at(look, 1)))
                {
                    return true;
                }
                look = look.add(2);
            } else if crate::memory::cstr_first_byte(look) == b'o' {
                if try_match && keytyped == KEY_OPEN_FORW {
                    return true;
                }
                look = look.add(1);
            } else if crate::memory::cstr_first_byte(look) == b'O' {
                if try_match && keytyped == KEY_OPEN_BACK {
                    return true;
                }
                look = look.add(1);
            } else if crate::memory::cstr_first_byte(look) == b'e' {
                if try_match && keytyped == b'e' as c_int && curwin_mut().w_cursor.col >= 4 {
                    let p = get_cursor_line_ptr();
                    if skipwhite(p) == p.add(curwin_mut().w_cursor.col as usize - 4)
                        && (crate::memory::cstr_prefix_bytes(
                            p.add(curwin_mut().w_cursor.col as usize - 4),
                            4,
                        ) == &b"else"[..4])
                    {
                        return true;
                    }
                }
                look = look.add(1);
            } else if crate::memory::cstr_first_byte(look) == b':' {
                if try_match && keytyped == b':' as c_int {
                    let mut p = get_cursor_line_ptr();
                    if cin_iscase(p, false) || cin_isscopedecl(p) || cin_islabel() {
                        return true;
                    }
                    p = get_cursor_line_ptr();
                    if curwin_mut().w_cursor.col > 2
                        && *p.add(curwin_mut().w_cursor.col as usize - 1) == b':' as c_char
                        && *p.add(curwin_mut().w_cursor.col as usize - 2) == b':' as c_char
                    {
                        *p.add(curwin_mut().w_cursor.col as usize - 1) = b' ' as c_char;
                        let i = cin_iscase(p, false) || cin_isscopedecl(p) || cin_islabel();
                        p = get_cursor_line_ptr();
                        *p.add(curwin_mut().w_cursor.col as usize - 1) = b':' as c_char;
                        if i {
                            return true;
                        }
                    }
                }
                look = look.add(1);
            } else if crate::memory::cstr_first_byte(look) == b'<' {
                if try_match {
                    if !vim_strchr(c"<>!*oOe0:", crate::memory::cstr_byte_at(look, 1) as c_int)
                        .is_null()
                        && keytyped == crate::memory::cstr_char_at(look, 1)
                    {
                        return true;
                    }
                    if keytyped == get_special_key_code(look.add(1)) {
                        return true;
                    }
                }
                while !crate::memory::cstr_is_empty(look)
                    && crate::memory::cstr_first_byte(look) != b'>'
                {
                    look = look.add(1);
                }
                while crate::memory::cstr_first_byte(look) == b'>' {
                    look = look.add(1);
                }
            } else if crate::memory::cstr_first_byte(look) == b'='
                && crate::memory::cstr_byte_at(look, 1) != b','
                && crate::memory::cstr_byte_at(look, 1) != 0
            {
                look = look.add(1);
                let icase = if crate::memory::cstr_first_byte(look) == b'~' {
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
                if (try_match || try_match_word) && (curwin_mut().w_cursor.col as usize) >= word_len
                {
                    let mut matched = false;
                    if keytyped == KEY_COMPLETE {
                        let line = get_cursor_line_ptr();
                        let mut s: *mut c_char = line.add(curwin_mut().w_cursor.col as usize);
                        while s > line {
                            let n = mb_prevptr(line, s);
                            if !vim_iswordp(n) {
                                break;
                            }
                            s = n;
                        }
                        if s.add(word_len) <= line.add(curwin_mut().w_cursor.col as usize)
                            && (if icase {
                                mb_strnicmp(s, look, word_len)
                            } else {
                                crate::memory::cstr_prefix_bytes(s, word_len)
                                    .cmp(crate::memory::cstr_prefix_bytes(look, word_len))
                                    as c_int
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
                        if (curwin_mut().w_cursor.col as usize == word_len
                            || !vim_iswordc(*line.sub(word_len + 1) as u8 as c_int))
                            && (if icase {
                                mb_strnicmp(line.sub(word_len), look, word_len)
                            } else {
                                crate::memory::cstr_prefix_bytes(line.sub(word_len), word_len)
                                    .cmp(crate::memory::cstr_prefix_bytes(look, word_len))
                                    as c_int
                            }) == 0
                        {
                            matched = true;
                        }
                    }
                    if matched
                        && try_match_word
                        && !try_match
                        && getwhitecols_curline() as c_int
                            != (curwin_mut().w_cursor.col as c_int - word_len as c_int)
                    {
                        matched = false;
                    }
                    if matched {
                        return true;
                    }
                }
                look = p;
            } else {
                if try_match && crate::memory::cstr_first_byte(look) as c_int == keytyped {
                    return true;
                }
                if !crate::memory::cstr_is_empty(look) {
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
    {
        if curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST {
            fixthisline(get_expr_indent);
        } else {
            fixthisline(get_c_indent);
        }
    }
}
/// "cindent(lnum)" function
pub fn f_cindent(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let pos = curwin_mut().w_cursor;
        let lnum = tv_get_lnum(&argvars[0]);
        if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count {
            curwin_mut().w_cursor.lnum = lnum;
            rettv.vval = crate::eval::typval::TypvalVval::Number(get_c_indent() as i64);
            curwin_mut().w_cursor = pos;
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
        let a = CursorPos {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let b = CursorPos {
            lnum: 1,
            col: 5,
            coladd: 0,
        };
        assert!(lt(a, b));
        assert!(!lt(b, a));
    }
}
