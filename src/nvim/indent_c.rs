//! C-indentation functions (`src/nvim/indent_c.c`).
#![forbid(unsafe_code)]
use crate::buffer::{FileBuf, MAXLNUM};
use crate::change::{
    COM_END, COM_LEFT, COM_MAX_LEN, COM_MIDDLE, COM_RIGHT, COM_START, KEY_OPEN_BACK, KEY_OPEN_FORW,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, getdigits_int, getwhitecols_curline, vim_is_idc,
    vim_iswordc, vim_iswordp, vim_strsize,
};
use crate::cursor::{Colnr, CursorPos, Linenr, Lpos, get_cursor_line_ptr, lt};
use crate::eval::typval::{EvalFuncData, K_CALLBACK_NONE_CONST, TypVal, tv_get_lnum};
use crate::globals::*;
use crate::indent::{fixthisline, get_expr_indent, get_indent, get_indent_lnum, get_sw_value};
use crate::keycodes::get_special_key_code;
use crate::math::trim_to_int;
use crate::mbyte::mb_strnicmp;
use crate::option::copy_option_part;
use crate::option_vars::*;
use crate::plines::getvcol;
use crate::regexp::MAXCOL;
use crate::search::{findmatchlimit, linewhite};
use crate::state::MODE_INSERT;
pub(crate) const FM_BACKWARD: i32 = 0x01;
pub(crate) const FM_BLOCKSTOP: i32 = 0x04;
pub(crate) const FM_SKIPCOMM: i32 = 0x08;
pub(crate) const KEY_COMPLETE: i32 = 0x103;
const FIND_NAMESPACE_LIM: Linenr = 20;
const BRACE_IN_COL0: i32 = 1;
const BRACE_AT_START: i32 = 2;
const BRACE_AT_END: i32 = 3;
const LOOKFOR_INITIAL: i32 = 0;
const LOOKFOR_IF: i32 = 1;
const LOOKFOR_DO: i32 = 2;
const LOOKFOR_CASE: i32 = 3;
const LOOKFOR_ANY: i32 = 4;
const LOOKFOR_TERM: i32 = 5;
const LOOKFOR_UNTERM: i32 = 6;
const LOOKFOR_SCOPEDECL: i32 = 7;
const LOOKFOR_NOBREAK: i32 = 8;
const LOOKFOR_CPP_BASECLASS: i32 = 9;
const LOOKFOR_ENUM_OR_INIT: i32 = 10;
const LOOKFOR_JS_KEY: i32 = 11;
const LOOKFOR_COMMA: i32 = 12;
#[repr(C)]
#[derive(Clone, Copy)]
struct CppBaseclassCache {
    found: i32,
    lpos: Lpos,
}
use crate::keycodes::ctrl_chr;
/// Check if line[] contains a "//" comment, ignoring matches inside strings.
/// Return MAXCOL if not, otherwise return the column.
pub fn check_linecomment(bytes: &[u8]) -> i32 {
    if curbuf_mut().b_p_lisp.get() {
        if bytes.contains(&b';') {
            let mut in_str = false;
            let mut idx = 0usize;
            while let Some(rel) = bytes[idx..].iter().position(|&b| b == b'"' || b == b';') {
                idx += rel;
                if bytes[idx] == b'"' {
                    if in_str {
                        if bytes[idx - 1] != b'\\' {
                            in_str = false;
                        }
                    } else if idx == 0
                        || (idx >= 2 && bytes[idx - 1] != b'\\' && bytes[idx - 2] != b'#')
                    {
                        in_str = true;
                    }
                } else if !in_str
                    && (idx < 2 || (bytes[idx - 1] != b'\\' && bytes[idx - 2] != b'#'))
                    && is_pos_in_string_slice(bytes, idx as Colnr) == 0
                {
                    return idx as i32;
                }
                idx += 1;
            }
        }
        return MAXCOL;
    }

    let mut idx = 0usize;
    while idx < bytes.len() {
        idx = skip_string_idx(bytes, idx);
        if idx >= bytes.len() {
            break;
        }
        if bytes[idx] == b'/'
            && bytes.get(idx + 1) == Some(&b'/')
            && (idx == 0 || bytes[idx - 1] != b'*' || bytes.get(idx + 2) != Some(&b'*'))
        {
            return idx as i32;
        }
        idx += 1;
    }
    MAXCOL
}
fn ind_find_start_comment() -> Option<CursorPos> {
    find_start_comment(curbuf_mut().b_ind_maxcomment.get())
}
pub fn find_start_comment(ind_maxcomment: i32) -> Option<CursorPos> {
    let mut pos: Option<CursorPos>;
    let mut cur_maxcomment = ind_maxcomment as i64;
    loop {
        pos = findmatchlimit(None, b'*' as i32, FM_BACKWARD, cur_maxcomment);
        let Some(pos_ref) = pos else {
            break;
        };
        if is_pos_in_string_slice(crate::memline::ml_get_slice(pos_ref.lnum), pos_ref.col) == 0 {
            break;
        }
        cur_maxcomment = (curwin_mut().w_cursor.get().lnum - pos_ref.lnum - 1) as i64;
        if cur_maxcomment <= 0 {
            pos = None;
            break;
        }
    }
    pos
}
fn ind_find_start_cors(is_raw: Option<&mut Linenr>) -> Option<CursorPos> {
    let comment_pos = find_start_comment(curbuf_mut().b_ind_maxcomment.get());
    let rs_pos = find_start_rawstring(curbuf_mut().b_ind_maxcomment.get());
    if comment_pos.is_none_or(|cp| rs_pos.is_some_and(|rs| lt(rs, cp))) {
        if let Some(rs) = rs_pos
            && let Some(is_raw) = is_raw
        {
            *is_raw = rs.lnum;
        }
        return rs_pos;
    }
    comment_pos
}
fn find_start_rawstring(ind_maxcomment: i32) -> Option<CursorPos> {
    let mut pos: Option<CursorPos>;
    let mut cur_maxcomment = ind_maxcomment;
    loop {
        pos = findmatchlimit(None, b'R' as i32, FM_BACKWARD, cur_maxcomment as i64);
        let Some(pos_ref) = pos else {
            break;
        };
        if is_pos_in_string_slice(crate::memline::ml_get_slice(pos_ref.lnum), pos_ref.col) == 0 {
            break;
        }
        cur_maxcomment = curwin_mut().w_cursor.get().lnum - pos_ref.lnum - 1;
        if cur_maxcomment <= 0 {
            pos = None;
            break;
        }
    }
    pos
}
fn skip_string_idx(bytes: &[u8], mut p: usize) -> usize {
    loop {
        if bytes.get(p) == Some(&b'\'') {
            if p + 1 >= bytes.len() {
                break;
            }
            let mut i: usize = 2;
            if bytes[p + 1] == b'\\' && p + 2 < bytes.len() {
                i += 1;
                while bytes.get(p + i - 1).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            if p + i < bytes.len() && bytes[p + i - 1] != 0 && bytes[p + i] == b'\'' {
                p += i + 1;
                continue;
            }
        } else if bytes.get(p) == Some(&b'"') {
            p += 1;
            while p < bytes.len() {
                if bytes[p] == b'\\' && p + 1 < bytes.len() {
                    p += 1;
                } else if bytes[p] == b'"' {
                    break;
                }
                p += 1;
            }
            if bytes.get(p) == Some(&b'"') {
                p += 1;
                continue;
            }
        } else if bytes.get(p) == Some(&b'R') && bytes.get(p + 1) == Some(&b'"') {
            let delim_start = p + 2;
            if let Some(rel_paren) = bytes[delim_start..].iter().position(|&b| b == b'(') {
                let delim = &bytes[delim_start..delim_start + rel_paren];
                let delim_len = delim.len();
                p += 3;
                while p < bytes.len() {
                    if bytes[p] == b')'
                        && bytes.get(p + 1..p + 1 + delim_len) == Some(delim)
                        && bytes.get(p + 1 + delim_len) == Some(&b'"')
                    {
                        p += delim_len + 1;
                        break;
                    }
                    p += 1;
                }
                if bytes.get(p) == Some(&b'"') {
                    p += 1;
                    continue;
                }
            }
        }
        break;
    }
    if p >= bytes.len() {
        p = bytes.len().saturating_sub(1);
    }
    p
}
fn is_pos_in_string_slice(bytes: &[u8], col: Colnr) -> i32 {
    let mut idx = 0usize;
    while idx < bytes.len() && (idx as Colnr) < col {
        idx = skip_string_idx(bytes, idx) + 1;
    }
    i32::from(!((idx as Colnr) <= col))
}
pub fn cin_is_cinword(line: &[u8]) -> bool {
    let line = crate::charset::skipwhite_slice(line);
    let cinw_rc = curbuf_mut().b_p_cinw.as_bytes();
    let cinw_len = cinw_rc.len() + 1;
    let mut cinw_buf = vec![0u8; cinw_len];
    let mut cinw = cinw_rc.as_slice();
    while !cinw.is_empty() {
        let len = copy_option_part(&mut cinw, &mut cinw_buf, c",");
        if line.starts_with(&cinw_buf[..len])
            && (!line.get(len).is_some_and(|&b| vim_iswordc(i32::from(b)))
                || (len > 0 && !vim_iswordc(i32::from(line[len - 1]))))
        {
            return true;
        }
    }
    false
}
pub fn cindent_on() -> bool {
    !opt_vars().p_paste.get()
        && (curbuf_mut().b_p_cin.get() || curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST)
}
fn cin_skipcomment_slice(mut s: &[u8]) -> &[u8] {
    while !s.is_empty() {
        let prev_len = s.len();
        s = crate::charset::skipwhite_slice(s);
        if curbuf_mut().b_ind_hash_comment.get() != 0
            && s.len() != prev_len
            && s.first() == Some(&b'#')
        {
            return &s[s.len()..];
        }
        if s.first() != Some(&b'/') {
            break;
        }
        s = &s[1..];
        if s.first() == Some(&b'/') {
            return &s[s.len()..];
        }
        if s.first() != Some(&b'*') {
            break;
        }
        s = &s[1..];
        while !s.is_empty() {
            if s.starts_with(b"*/") {
                s = &s[2..];
                break;
            }
            s = &s[1..];
        }
    }
    s
}
#[inline]
fn cin_nocode_slice(s: &[u8]) -> bool {
    cin_skipcomment_slice(s).is_empty()
}
fn find_line_comment() -> Option<CursorPos> {
    let mut pos = curwin_mut().w_cursor.get();
    loop {
        pos.lnum -= 1;
        if pos.lnum <= 0 {
            break;
        }
        let line = crate::memline::ml_get_slice(pos.lnum);
        let p = crate::charset::skipwhite_slice(line);
        if cin_islinecomment_slice(p) {
            pos.col = (line.len() - p.len()) as Colnr;
            return Some(pos);
        }
        if !p.is_empty() {
            break;
        }
    }
    None
}
fn cin_has_js_key(text: &[u8]) -> bool {
    let mut s = crate::charset::skipwhite_slice(text);
    let mut quote: u8 = 0;
    if let Some(&first @ (b'\'' | b'"')) = s.first() {
        quote = first;
        s = &s[1..];
    }
    if !s.first().is_some_and(|&b| vim_is_idc(i32::from(b))) {
        return false;
    }
    while s.first().is_some_and(|&b| vim_is_idc(i32::from(b))) {
        s = &s[1..];
    }
    if quote != 0 && s.first() == Some(&quote) {
        s = &s[1..];
    }
    s = cin_skipcomment_slice(s);
    s.first() == Some(&b':') && s.get(1) != Some(&b':')
}
fn cin_islabel_skip_slice(s: &mut &[u8]) -> bool {
    if !s.first().is_some_and(|&b| vim_is_idc(i32::from(b))) {
        return false;
    }
    while s.first().is_some_and(|&b| vim_is_idc(i32::from(b))) {
        let clen = crate::mbyte::utfc_slice2len(s).max(1);
        *s = &s[clen.min(s.len())..];
    }
    *s = cin_skipcomment_slice(s);
    if let Some(after_colon) = s.strip_prefix(b":") {
        *s = after_colon;
        s.first() != Some(&b':')
    } else {
        false
    }
}
fn cin_islabel() -> bool {
    let mut s = cin_skipcomment_slice(crate::cursor::get_cursor_line_slice());
    if cin_isdefault_slice(s) {
        return false;
    }
    if cin_isscopedecl_slice(s) {
        return false;
    }
    if !cin_islabel_skip_slice(&mut s) {
        return false;
    }
    if ind_find_start_cors(None).is_some() {
        return false;
    }
    let cursor_save = curwin_mut().w_cursor.get();
    while curwin_mut().w_cursor.get().lnum > 1 {
        curwin_mut().w_cursor.sub_lnum(1);
        curwin_mut().w_cursor.set_col(0);
        if let Some(trypos) = ind_find_start_cors(None) {
            curwin_mut().w_cursor.set(trypos);
        }
        let mut line = crate::cursor::get_cursor_line_slice();
        if cin_ispreproc_slice(line) {
            continue;
        }
        line = cin_skipcomment_slice(line);
        if line.is_empty() {
            continue;
        }
        curwin_mut().w_cursor.set(cursor_save);
        return cin_isterminated_slice(line, true, false) != 0
            || cin_isscopedecl_slice(line)
            || cin_iscase_slice(line, true)
            || (cin_islabel_skip_slice(&mut line) && cin_nocode_slice(line));
    }
    curwin_mut().w_cursor.set(cursor_save);
    true
}
fn cin_skip_comment_and_string_idx(bytes: &[u8], mut idx: usize) -> usize {
    loop {
        let r = idx;
        let rem = cin_skipcomment_slice(&bytes[idx..]);
        idx = bytes.len() - rem.len();
        if idx < bytes.len() {
            idx = skip_string_idx(bytes, idx);
        }
        if idx == r {
            break;
        }
    }
    idx
}
fn cin_is_compound_init_slice(s: &[u8]) -> bool {
    let mut idx = 0usize;
    let mut r_idx: Option<usize> = None;
    while idx < s.len() {
        if s[idx] == b'=' {
            let rem = cin_skipcomment_slice(&s[idx + 1..]);
            idx = s.len() - rem.len();
            r_idx = Some(idx);
        } else if s[idx..].starts_with(b"return")
            && !s.get(idx + 6).is_some_and(|&b| vim_is_idc(i32::from(b)))
            && (idx == 0 || !vim_is_idc(i32::from(s[idx - 1])))
        {
            let rem = cin_skipcomment_slice(&s[idx + 6..]);
            idx = s.len() - rem.len();
            r_idx = Some(idx);
        } else {
            idx = cin_skip_comment_and_string_idx(s, idx + 1);
        }
    }
    let Some(mut idx) = r_idx else {
        return false;
    };
    if cin_nocode_slice(&s[idx..]) {
        return true;
    }
    if s.get(idx) == Some(&b'&') {
        let rem = cin_skipcomment_slice(&s[idx + 1..]);
        idx = s.len() - rem.len();
    }
    if s.get(idx) == Some(&b'(') {
        let mut open_count: i32 = 1;
        loop {
            idx = cin_skip_comment_and_string_idx(s, idx + 1);
            if cin_nocode_slice(&s[idx..]) {
                return true;
            }
            open_count +=
                i32::from(s.get(idx) == Some(&b'(')) - i32::from(s.get(idx) == Some(&b')'));
            if open_count == 0 {
                break;
            }
        }
        let rem = cin_skipcomment_slice(&s[(idx + 1).min(s.len())..]);
        idx = s.len() - rem.len();
        if cin_nocode_slice(&s[idx..]) {
            return true;
        }
    }
    while s.get(idx) == Some(&b'{') {
        let rem = cin_skipcomment_slice(&s[idx + 1..]);
        idx = s.len() - rem.len();
    }
    cin_nocode_slice(&s[idx..])
}
fn cin_isinit() -> bool {
    const SKIP: [&[u8]; 4] = [b"static", b"public", b"protected", b"private"];
    let mut s = cin_skipcomment_slice(crate::cursor::get_cursor_line_slice());
    if cin_starts_with_slice(s, b"typedef") {
        s = cin_skipcomment_slice(&s[7..]);
    }
    loop {
        let mut matched = false;
        for &word in &SKIP {
            if cin_starts_with_slice(s, word) {
                s = cin_skipcomment_slice(&s[word.len()..]);
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }
    if cin_starts_with_slice(s, b"enum") {
        return true;
    }
    cin_is_compound_init_slice(s)
}
fn cin_iscase_slice(mut s: &[u8], strict: bool) -> bool {
    s = cin_skipcomment_slice(s);
    if cin_starts_with_slice(s, b"case") {
        s = &s[4..];
        while !s.is_empty() {
            s = cin_skipcomment_slice(s);
            if s.is_empty() {
                break;
            }
            if s[0] == b':' {
                if s.get(1) == Some(&b':') {
                    s = &s[1..];
                } else {
                    return true;
                }
            }
            if s[0] == b'\'' && s.len() >= 3 && s[1] != 0 && s[2] == b'\'' {
                s = &s[2..];
            } else if s[0] == b'/' && matches!(s.get(1), Some(&b'*' | &b'/')) {
                return false;
            } else if s[0] == b'"' {
                return !strict;
            }
            s = &s[1..];
        }
        return false;
    }
    cin_isdefault_slice(s)
}
fn cin_isdefault_slice(s: &[u8]) -> bool {
    let Some(after) = s.strip_prefix(b"default") else {
        return false;
    };
    let s = cin_skipcomment_slice(after);
    s.first() == Some(&b':') && s.get(1) != Some(&b':')
}
fn cin_isscopedecl_slice(p: &[u8]) -> bool {
    let s = cin_skipcomment_slice(p);
    let cinsd_rc = curbuf_mut().b_p_cinsd.as_bytes();
    let cinsd_len = cinsd_rc.len() + 1;
    let mut cinsd_buf = vec![0u8; cinsd_len];
    let mut cinsd = cinsd_rc.as_slice();
    while !cinsd.is_empty() {
        let len = copy_option_part(&mut cinsd, &mut cinsd_buf, c",");
        if s.starts_with(&cinsd_buf[..len]) {
            let skip = cin_skipcomment_slice(&s[len..]);
            if skip.first() == Some(&b':') && skip.get(1) != Some(&b':') {
                return true;
            }
        }
    }
    false
}
fn cin_is_cpp_namespace_slice(s: &[u8]) -> bool {
    let mut has_name = false;
    let mut has_name_start = false;
    let mut s = cin_skipcomment_slice(s);
    while (s.starts_with(b"inline") || s.starts_with(b"export"))
        && !s.get(6).is_some_and(|&b| vim_iswordc(i32::from(b)))
    {
        s = cin_skipcomment_slice(crate::charset::skipwhite_slice(&s[6..]));
    }
    if s.starts_with(b"namespace") && !s.get(9).is_some_and(|&b| vim_iswordc(i32::from(b))) {
        let mut p = cin_skipcomment_slice(crate::charset::skipwhite_slice(&s[9..]));
        while !p.is_empty() {
            if ascii_iswhite(i32::from(p[0])) {
                has_name = true;
                p = cin_skipcomment_slice(crate::charset::skipwhite_slice(p));
            } else if p[0] == b'{' {
                break;
            } else if vim_iswordc(i32::from(p[0])) {
                has_name_start = true;
                if has_name {
                    return false;
                }
                p = &p[1..];
            } else if p.starts_with(b"::") && p.get(2).is_some_and(|&b| vim_iswordc(i32::from(b))) {
                if !has_name_start || has_name {
                    return false;
                }
                p = &p[3..];
            } else {
                return false;
            }
        }
        return true;
    }
    false
}
fn after_label_slice(mut l: &[u8]) -> Option<&[u8]> {
    while !l.is_empty() {
        if l[0] == b':' {
            if l.get(1) == Some(&b':') {
                l = &l[1..];
            } else if !cin_iscase_slice(&l[1..], false) {
                break;
            }
        } else if l[0] == b'\'' && l.len() >= 3 && l[1] != 0 && l[2] == b'\'' {
            l = &l[2..];
        }
        l = &l[1..];
    }
    if l.is_empty() {
        return None;
    }
    l = cin_skipcomment_slice(&l[1..]);
    if l.is_empty() {
        return None;
    }
    Some(l)
}
fn get_indent_nolabel(lnum: Linenr) -> i32 {
    let l = crate::memline::ml_get_slice(lnum);
    let Some(p) = after_label_slice(l) else {
        return 0;
    };
    let mut fp = CursorPos {
        lnum,
        col: (l.len() - p.len()) as Colnr,
        coladd: 0,
    };
    let mut col: Colnr = 0;
    getvcol(&curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
    col
}
fn skip_label(lnum: Linenr, pp: &mut &'static [u8]) -> i32 {
    let cursor_save = curwin_mut().w_cursor.get();
    curwin_mut().w_cursor.set_lnum(lnum);
    let l_slice = crate::cursor::get_cursor_line_slice();
    let amount =
        if cin_iscase_slice(l_slice, false) || cin_isscopedecl_slice(l_slice) || cin_islabel() {
            let amt = get_indent_nolabel(lnum);
            let cur_slice = crate::cursor::get_cursor_line_slice();
            *pp = after_label_slice(cur_slice).unwrap_or(cur_slice);
            amt
        } else {
            let amt = get_indent();
            *pp = crate::cursor::get_cursor_line_slice();
            amt
        };
    curwin_mut().w_cursor.set(cursor_save);
    amount
}
fn cin_first_id_amount() -> i32 {
    let line = crate::cursor::get_cursor_line_slice();
    let mut p = crate::charset::skipwhite_slice(line);
    let mut len = p.len() - crate::charset::skiptowhite_slice(p).len();
    if len == 6 && p.starts_with(b"static") {
        p = crate::charset::skipwhite_slice(&p[6..]);
        len = p.len() - crate::charset::skiptowhite_slice(p).len();
    }
    if len == 6 && p.starts_with(b"struct") {
        p = crate::charset::skipwhite_slice(&p[6..]);
    } else if len == 4 && p.starts_with(b"enum") {
        p = crate::charset::skipwhite_slice(&p[4..]);
    } else if (len == 8 && p.starts_with(b"unsigned")) || (len == 6 && p.starts_with(b"signed")) {
        let s = crate::charset::skipwhite_slice(&p[len..]);
        if (s.starts_with(b"int") && s.get(3).is_some_and(|&b| ascii_iswhite(i32::from(b))))
            || (s.starts_with(b"long") && s.get(4).is_some_and(|&b| ascii_iswhite(i32::from(b))))
            || (s.starts_with(b"short") && s.get(5).is_some_and(|&b| ascii_iswhite(i32::from(b))))
            || (s.starts_with(b"char") && s.get(4).is_some_and(|&b| ascii_iswhite(i32::from(b))))
        {
            p = s;
        }
    }
    len = p.iter().take_while(|&&b| vim_is_idc(i32::from(b))).count();
    if len == 0 || !p.get(len).is_some_and(|&b| ascii_iswhite(i32::from(b))) || cin_nocode_slice(p)
    {
        return 0;
    }
    p = crate::charset::skipwhite_slice(&p[len..]);
    let mut fp = CursorPos {
        lnum: curwin_mut().w_cursor.get().lnum,
        col: (line.len() - p.len()) as Colnr,
        coladd: 0,
    };
    let mut col: Colnr = 0;
    getvcol(&curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
    col
}
fn cin_get_equal_amount(lnum: Linenr) -> i32 {
    if lnum > 1 {
        let line = crate::memline::ml_get_slice(lnum - 1);
        if line.last() == Some(&b'\\') {
            return -1;
        }
    }
    let line = crate::memline::ml_get_slice(lnum);
    let mut s = line;
    while !s.is_empty() && !matches!(s[0], b'=' | b';' | b'{' | b'}' | b'"' | b'\'') {
        if cin_iscomment_slice(s) {
            s = cin_skipcomment_slice(s);
        } else {
            s = &s[1..];
        }
    }
    let Some(after_eq) = s.strip_prefix(b"=") else {
        return 0;
    };
    s = crate::charset::skipwhite_slice(after_eq);
    if cin_nocode_slice(s) {
        return 0;
    }
    if let Some(after_quote) = s.strip_prefix(b"\"") {
        s = after_quote;
    }
    let mut fp = CursorPos {
        lnum,
        col: (line.len() - s.len()) as Colnr,
        coladd: 0,
    };
    let mut col: Colnr = 0;
    getvcol(&curwin_mut(), &mut fp, Some(&mut col), None, None, 0);
    col
}
#[inline]
fn cin_ispreproc_slice(s: &[u8]) -> bool {
    crate::charset::skipwhite_slice(s).first() == Some(&b'#')
}
fn cin_ispreproc_cont(pp: &mut &'static [u8], lnump: &mut Linenr, amount: &mut i32) -> bool {
    let mut line: &[u8] = pp;
    let mut lnum = *lnump;
    let mut retval = false;
    let mut candidate_amount = *amount;
    if line.last() == Some(&b'\\') {
        candidate_amount = get_indent_lnum(lnum);
    }
    loop {
        if cin_ispreproc_slice(line) {
            retval = true;
            *lnump = lnum;
            break;
        }
        if lnum == 1 {
            break;
        }
        lnum -= 1;
        line = crate::memline::ml_get_slice(lnum);
        if line.last() != Some(&b'\\') {
            break;
        }
    }
    if lnum != *lnump {
        *pp = crate::memline::ml_get_slice(*lnump);
    }
    if retval {
        *amount = candidate_amount;
    }
    retval
}
#[inline]
fn cin_iscomment_slice(s: &[u8]) -> bool {
    s.starts_with(b"/*") || s.starts_with(b"//")
}
#[inline]
fn cin_islinecomment_slice(s: &[u8]) -> bool {
    s.starts_with(b"//")
}
fn cin_isterminated_slice(s: &[u8], incl_open: bool, incl_comma: bool) -> u8 {
    let mut found_start: u8 = 0;
    let mut n_open: u32 = 0;
    let mut is_else = false;
    let s = cin_skipcomment_slice(s);
    if s.first() == Some(&b'{') || (s.first() == Some(&b'}') && !cin_iselse_slice(s)) {
        found_start = s[0];
    }
    if found_start == 0 {
        is_else = cin_iselse_slice(s);
    }
    let mut idx = 0usize;
    while idx < s.len() {
        let rem = cin_skipcomment_slice(&s[idx..]);
        idx = s.len() - rem.len();
        if idx < s.len() {
            idx = skip_string_idx(s, idx);
        }
        if idx >= s.len() {
            break;
        }
        let b = s[idx];
        if b == b'}' && n_open > 0 {
            n_open -= 1;
        }
        if (!is_else || n_open == 0)
            && (b == b';' || b == b'}' || (incl_comma && b == b','))
            && cin_nocode_slice(&s[idx + 1..])
        {
            return b;
        } else if b == b'{' {
            if incl_open && cin_nocode_slice(&s[idx + 1..]) {
                return b;
            } else {
                n_open += 1;
            }
        }
        idx += 1;
    }
    found_start
}
fn cin_isfuncdecl(sp: Option<&mut &'static [u8]>, first_lnum: Linenr, min_lnum: Linenr) -> bool {
    let mut s: &[u8] = match sp.as_deref() {
        Some(slice) => slice,
        None => crate::memline::ml_get_slice(first_lnum),
    };
    let mut lnum = first_lnum;
    let save_lnum = curwin_mut().w_cursor.get().lnum;
    let mut retval = false;
    let mut just_started = true;
    curwin_mut().w_cursor.set_lnum(lnum);
    if find_last_paren_slice(s, b'(', b')')
        && let Some(trypos) = find_match_paren(curbuf_mut().b_ind_maxparen.get())
    {
        lnum = trypos.lnum;
        if lnum < min_lnum {
            curwin_mut().w_cursor.set_lnum(save_lnum);
            return false;
        }
        s = crate::memline::ml_get_slice(lnum);
    }
    curwin_mut().w_cursor.set_lnum(save_lnum);
    if cin_ispreproc_slice(s) {
        return false;
    }
    while !s.is_empty() && !matches!(s[0], b'(' | b';' | b'\'' | b'"') {
        if cin_iscomment_slice(s) {
            s = cin_skipcomment_slice(s);
        } else if s[0] == b':' {
            if s.get(1) == Some(&b':') {
                s = &s[2..];
            } else {
                return false;
            }
        } else {
            s = &s[1..];
        }
    }
    if s.first() != Some(&b'(') {
        return false;
    }
    'done: {
        while !s.is_empty() && !matches!(s[0], b';' | b'\'' | b'"') {
            if s[0] == b')' && cin_nocode_slice(&s[1..]) {
                lnum = first_lnum - 1;
                let prev_s = crate::memline::ml_get_slice(lnum);
                if prev_s.last() != Some(&b'\\') {
                    retval = true;
                }
                break 'done;
            }
            if (s[0] == b',' && cin_nocode_slice(&s[1..])) || s.len() == 1 || cin_nocode_slice(s) {
                let comma = s[0] == b',';
                loop {
                    if lnum >= curbuf_mut().b_ml.ml_line_count.get() {
                        break;
                    }
                    lnum += 1;
                    s = crate::memline::ml_get_slice(lnum);
                    if !cin_ispreproc_slice(s) {
                        break;
                    }
                }
                if lnum >= curbuf_mut().b_ml.ml_line_count.get() {
                    break;
                }
                s = crate::charset::skipwhite_slice(s);
                if !just_started && !comma && !matches!(s.first(), Some(&b',' | &b')')) {
                    break;
                }
                just_started = false;
            } else if cin_iscomment_slice(s) {
                s = cin_skipcomment_slice(s);
            } else {
                s = &s[1..];
                just_started = false;
            }
        }
    }
    if lnum != first_lnum
        && let Some(sp) = sp
    {
        *sp = crate::memline::ml_get_slice(first_lnum);
    }
    retval
}
#[inline]
fn cin_isif_slice(p: &[u8]) -> bool {
    cin_starts_with_slice(p, b"if")
}
fn cin_iselse_slice(mut p: &[u8]) -> bool {
    if let Some(after) = p.strip_prefix(b"}") {
        p = cin_skipcomment_slice(after);
    }
    cin_starts_with_slice(p, b"else")
}
#[inline]
fn cin_isdo_slice(p: &[u8]) -> bool {
    cin_starts_with_slice(p, b"do")
}
fn cin_iswhileofdo(p: &[u8], lnum: Linenr) -> bool {
    let mut s = cin_skipcomment_slice(p);
    if let Some(after) = s.strip_prefix(b"}") {
        s = cin_skipcomment_slice(after);
    }
    if !cin_starts_with_slice(s, b"while") {
        return false;
    }
    let cursor_save = curwin_mut().w_cursor.get();
    curwin_mut().w_cursor.set_lnum(lnum);
    let line = crate::cursor::get_cursor_line_slice();
    let w_pos = line.iter().position(|&b| b == b'w').unwrap_or(line.len());
    curwin_mut().w_cursor.set_col(w_pos as Colnr);
    let trypos = findmatchlimit(
        None,
        0,
        FM_SKIPCOMM,
        curbuf_mut().b_ind_maxparen.get() as i64,
    );
    let retval = trypos.is_some_and(|ref tp| {
        cin_skipcomment_slice(crate::memline::ml_get_pos_slice(tp).get(1..).unwrap_or(&[])).first()
            == Some(&b';')
    });
    curwin_mut().w_cursor.set(cursor_save);
    retval
}
fn cin_is_if_for_while_before_offset(bytes: &[u8], poffset: &mut Colnr) -> i32 {
    let mut offset = *poffset;
    if offset < 2 {
        return 0;
    }
    offset -= 1;
    while offset > 2
        && bytes
            .get(offset as usize)
            .is_some_and(|&b| ascii_iswhite(i32::from(b)))
    {
        offset -= 1;
    }
    offset -= 1;
    let mut probably_found = false;
    if bytes
        .get(offset as usize..)
        .is_some_and(|s| s.starts_with(b"if"))
    {
        probably_found = true;
    } else if offset >= 1 {
        offset -= 1;
        if bytes
            .get(offset as usize..)
            .is_some_and(|s| s.starts_with(b"for"))
        {
            probably_found = true;
        } else if offset >= 2 {
            offset -= 2;
            if bytes
                .get(offset as usize..)
                .is_some_and(|s| s.starts_with(b"while"))
            {
                probably_found = true;
            }
        }
    }
    if !probably_found {
        return 0;
    }
    if offset == 0 || !vim_is_idc(i32::from(bytes[(offset - 1) as usize])) {
        *poffset = offset;
        return 1;
    }
    0
}
fn cin_iswhileofdo_end(terminated: i32) -> bool {
    if terminated != b';' as i32 {
        return false;
    }
    let mut line = crate::cursor::get_cursor_line_slice();
    let mut p = line;
    while !p.is_empty() {
        p = cin_skipcomment_slice(p);
        if p.first() == Some(&b')') {
            let s = crate::charset::skipwhite_slice(&p[1..]);
            if s.first() == Some(&b';') && cin_nocode_slice(&s[1..]) {
                let i = (line.len() - p.len()) as Colnr;
                curwin_mut().w_cursor.set_col(i);
                if let Some(trypos) = find_match_paren(curbuf_mut().b_ind_maxparen.get()) {
                    let tp_lnum = trypos.lnum;
                    let mut s2 = cin_skipcomment_slice(crate::memline::ml_get_slice(tp_lnum));
                    if let Some(after) = s2.strip_prefix(b"}") {
                        s2 = cin_skipcomment_slice(after);
                    }
                    if cin_starts_with_slice(s2, b"while") {
                        curwin_mut().w_cursor.set_lnum(tp_lnum);
                        return true;
                    }
                }
                line = crate::cursor::get_cursor_line_slice();
                p = &line[(i as usize).min(line.len())..];
            }
        }
        if !p.is_empty() {
            p = &p[1..];
        }
    }
    false
}
#[inline]
fn cin_isbreak_slice(p: &[u8]) -> bool {
    cin_starts_with_slice(p, b"break")
}
fn cin_is_cpp_baseclass(cached: &mut CppBaseclassCache) -> bool {
    let pos = &mut cached.lpos;
    let mut lnum = curwin_mut().w_cursor.get().lnum;
    let mut line = crate::cursor::get_cursor_line_slice();
    if pos.lnum <= lnum {
        return cached.found != 0;
    }
    pos.col = 0;
    let mut s = crate::charset::skipwhite_slice(line);
    if s.first() == Some(&b'#') {
        return false;
    }
    s = cin_skipcomment_slice(s);
    if s.is_empty() {
        return false;
    }
    let mut cpp_base_class = false;
    let mut lookfor_ctor_init = false;
    let mut class_or_struct = false;
    while lnum > 1 {
        line = crate::memline::ml_get_slice(lnum - 1);
        s = crate::charset::skipwhite_slice(line);
        if s.first() == Some(&b'#') || s.is_empty() {
            break;
        }
        while !s.is_empty() {
            s = cin_skipcomment_slice(s);
            if matches!(s.first(), Some(&b'{' | &b'}'))
                || (s.first() == Some(&b';') && cin_nocode_slice(&s[1..]))
            {
                break;
            }
            if !s.is_empty() {
                s = &s[1..];
            }
        }
        if !s.is_empty() {
            break;
        }
        lnum -= 1;
    }
    pos.lnum = lnum;
    line = crate::memline::ml_get_slice(lnum);
    s = line;
    loop {
        if s.is_empty() {
            if lnum == curwin_mut().w_cursor.get().lnum {
                break;
            }
            lnum += 1;
            line = crate::memline::ml_get_slice(lnum);
            s = line;
        }
        if s.len() == line.len() {
            if cin_iscase_slice(s, false) {
                break;
            }
            s = cin_skipcomment_slice(line);
            if s.is_empty() {
                continue;
            }
        }
        if s[0] == b'"' || s.starts_with(b"R\"") {
            let skipped = skip_string_idx(s, 0);
            s = &s[(skipped + 1).min(s.len())..];
        } else if s[0] == b':' {
            if s.get(1) == Some(&b':') {
                lookfor_ctor_init = false;
                s = cin_skipcomment_slice(&s[2..]);
            } else if lookfor_ctor_init || class_or_struct {
                cpp_base_class = true;
                lookfor_ctor_init = false;
                class_or_struct = false;
                pos.col = 0;
                s = cin_skipcomment_slice(&s[1..]);
            } else {
                s = cin_skipcomment_slice(&s[1..]);
            }
        } else if (s.starts_with(b"class") && !s.get(5).is_some_and(|&b| vim_is_idc(i32::from(b))))
            || (s.starts_with(b"struct") && !s.get(6).is_some_and(|&b| vim_is_idc(i32::from(b))))
        {
            class_or_struct = true;
            lookfor_ctor_init = false;
            if s[0] == b'c' {
                s = cin_skipcomment_slice(&s[5..]);
            } else {
                s = cin_skipcomment_slice(&s[6..]);
            }
        } else {
            if matches!(s[0], b'{' | b'}' | b';') {
                cpp_base_class = false;
                lookfor_ctor_init = false;
                class_or_struct = false;
            } else if s[0] == b')' {
                class_or_struct = false;
                lookfor_ctor_init = true;
            } else if s[0] == b'?' {
                return false;
            } else if !vim_is_idc(i32::from(s[0])) {
                class_or_struct = false;
                lookfor_ctor_init = false;
            } else if pos.col == 0 {
                lookfor_ctor_init = false;
                if cpp_base_class {
                    pos.col = (line.len() - s.len()) as Colnr;
                }
            }
            if lnum == curwin_mut().w_cursor.get().lnum && s[0] == b',' && cin_nocode_slice(&s[1..])
            {
                pos.col = 0;
            }
            s = cin_skipcomment_slice(&s[1..]);
        }
    }
    cached.found = i32::from(cpp_base_class);
    if cpp_base_class {
        pos.lnum = lnum;
    }
    cpp_base_class
}
fn get_baseclass_amount(col: Colnr) -> i32 {
    let mut amount: i32;
    let mut vcol: Colnr = 0;
    if col == 0 {
        amount = get_indent();
        if find_last_paren_slice(crate::cursor::get_cursor_line_slice(), b'(', b')')
            && let Some(trypos) = find_match_paren(curbuf_mut().b_ind_maxparen.get())
        {
            amount = get_indent_lnum(trypos.lnum);
        }
        if !cin_ends_in_slice(crate::cursor::get_cursor_line_slice(), b",") {
            amount += curbuf_mut().b_ind_cpp_baseclass.get();
        }
    } else {
        curwin_mut().w_cursor.set_col(col);
        let mut cursor = curwin_mut().w_cursor.get();
        getvcol(&curwin_mut(), &mut cursor, Some(&mut vcol), None, None, 0);
        curwin_mut().w_cursor.set(cursor);
        amount = vcol;
    }
    if amount < curbuf_mut().b_ind_cpp_baseclass.get() {
        amount = curbuf_mut().b_ind_cpp_baseclass.get();
    }
    amount
}
fn cin_ends_in_slice(s: &[u8], find: &[u8]) -> bool {
    let mut p = s;
    let len = find.len();
    while !p.is_empty() {
        p = cin_skipcomment_slice(p);
        if p.starts_with(find) {
            let r = crate::charset::skipwhite_slice(&p[len..]);
            if cin_nocode_slice(r) {
                return true;
            }
        }
        if !p.is_empty() {
            p = &p[1..];
        }
    }
    false
}
#[inline]
fn cin_starts_with_slice(s: &[u8], word: &[u8]) -> bool {
    s.starts_with(word) && !s.get(word.len()).is_some_and(|&b| vim_is_idc(i32::from(b)))
}
fn cin_is_cpp_extern_c_slice(s: &[u8]) -> bool {
    let mut has_string_literal = false;
    let s = cin_skipcomment_slice(s);
    if s.starts_with(b"extern") && !s.get(6).is_some_and(|&b| vim_iswordc(i32::from(b))) {
        let mut p = cin_skipcomment_slice(crate::charset::skipwhite_slice(&s[6..]));
        while !p.is_empty() {
            if ascii_iswhite(i32::from(p[0])) {
                p = cin_skipcomment_slice(crate::charset::skipwhite_slice(p));
            } else if p[0] == b'{' {
                break;
            } else if p.starts_with(b"\"C\"") {
                if has_string_literal {
                    return false;
                }
                has_string_literal = true;
                p = &p[3..];
            } else if p.starts_with(b"\"C++\"") {
                if has_string_literal {
                    return false;
                }
                has_string_literal = true;
                p = &p[5..];
            } else {
                return false;
            }
        }
        return has_string_literal;
    }
    false
}
fn cin_skip2pos(trypos: &mut CursorPos) -> i32 {
    let bytes = crate::memline::ml_get_slice(trypos.lnum);
    let mut idx = 0usize;
    while idx < bytes.len() && (idx as Colnr) < trypos.col {
        if cin_iscomment_slice(&bytes[idx..]) {
            let rem = cin_skipcomment_slice(&bytes[idx..]);
            idx = bytes.len() - rem.len();
        } else {
            let new_idx = skip_string_idx(bytes, idx);
            if new_idx == idx {
                idx += 1;
            } else {
                idx = new_idx;
            }
        }
    }
    idx as i32
}
fn find_start_brace() -> Option<CursorPos> {
    let cursor_save = curwin_mut().w_cursor.get();
    let mut trypos: Option<CursorPos>;
    loop {
        trypos = findmatchlimit(None, b'{' as i32, FM_BLOCKSTOP | FM_SKIPCOMM, 0);
        let Some(mut tp_copy) = trypos else {
            break;
        };
        curwin_mut().w_cursor.set(tp_copy);
        let mut pos: Option<CursorPos> = None;
        if cin_skip2pos(&mut tp_copy) as Colnr == tp_copy.col && {
            pos = ind_find_start_cors(None);
            pos.is_none()
        } {
            trypos = Some(tp_copy);
            break;
        }
        if let Some(pos_ref) = pos {
            curwin_mut().w_cursor.set(pos_ref);
        }
    }
    curwin_mut().w_cursor.set(cursor_save);
    trypos
}
fn find_match_paren(ind_maxparen: i32) -> Option<CursorPos> {
    find_match_char(b'(', ind_maxparen)
}
fn find_match_char(c: u8, ind_maxparen: i32) -> Option<CursorPos> {
    let cursor_save = curwin_mut().w_cursor.get();
    let mut ind_maxp_wk = ind_maxparen;
    let mut trypos: Option<CursorPos>;
    loop {
        trypos = findmatchlimit(None, c as i32, FM_SKIPCOMM, ind_maxp_wk as i64);
        if let Some(ref mut tp_ref) = trypos {
            if (cin_skip2pos(tp_ref) as Colnr) > tp_ref.col {
                ind_maxp_wk = ind_maxparen - (cursor_save.lnum - tp_ref.lnum);
                if ind_maxp_wk > 0 {
                    curwin_mut().w_cursor.set(*tp_ref);
                    curwin_mut().w_cursor.set_col(0);
                    continue;
                }
                trypos = None;
            } else {
                let tp_val = *tp_ref;
                curwin_mut().w_cursor.set(tp_val);
                if let Some(wk_ref) = ind_find_start_cors(None) {
                    ind_maxp_wk = ind_maxparen - (cursor_save.lnum - wk_ref.lnum);
                    if ind_maxp_wk > 0 {
                        curwin_mut().w_cursor.set(wk_ref);
                        continue;
                    }
                    trypos = None;
                }
            }
        }
        break;
    }
    curwin_mut().w_cursor.set(cursor_save);
    trypos
}
fn find_match_paren_after_brace(ind_maxparen: i32) -> Option<CursorPos> {
    let trypos = find_match_paren(ind_maxparen)?;
    if let Some(tb) = find_start_brace()
        && if trypos.lnum != tb.lnum {
            trypos.lnum < tb.lnum
        } else {
            trypos.col < tb.col
        }
    {
        return None;
    }
    Some(trypos)
}
fn corr_ind_maxparen(startpos: &CursorPos) -> i32 {
    let n = startpos.lnum - curwin_mut().w_cursor.get().lnum;
    if n > 0 && n < curbuf_mut().b_ind_maxparen.get() / 2 {
        return curbuf_mut().b_ind_maxparen.get() - n;
    }
    curbuf_mut().b_ind_maxparen.get()
}
fn find_last_paren_slice(bytes: &[u8], start: u8, end: u8) -> bool {
    let mut retval = false;
    let mut open_count: i32 = 0;
    curwin_mut().w_cursor.set_col(0);
    let mut i: usize = 0;
    while i < bytes.len() {
        let rem = cin_skipcomment_slice(&bytes[i..]);
        i = bytes.len() - rem.len();
        if i >= bytes.len() {
            break;
        }
        i = skip_string_idx(bytes, i);
        if bytes[i] == start {
            open_count += 1;
        } else if bytes[i] == end {
            if open_count > 0 {
                open_count -= 1;
            } else {
                curwin_mut().w_cursor.set_col(i as Colnr);
                retval = true;
            }
        }
        i += 1;
    }
    retval
}
/// Parse 'cinoptions' and set the values in "buf".
pub fn parse_cino(buf: &FileBuf) {
    let mut fraction: i32 = 0;
    let sw = get_sw_value(buf);
    buf.b_ind_level.set(sw);
    buf.b_ind_open_imag.set(0);
    buf.b_ind_no_brace.set(0);
    buf.b_ind_first_open.set(0);
    buf.b_ind_open_extra.set(0);
    buf.b_ind_close_extra.set(0);
    buf.b_ind_open_left_imag.set(0);
    buf.b_ind_jump_label.set(-1);
    buf.b_ind_case.set(sw);
    buf.b_ind_case_code.set(sw);
    buf.b_ind_case_break.set(0);
    buf.b_ind_scopedecl.set(sw);
    buf.b_ind_scopedecl_code.set(sw);
    buf.b_ind_param.set(sw);
    buf.b_ind_func_type.set(sw);
    buf.b_ind_cpp_baseclass.set(sw);
    buf.b_ind_continuation.set(sw);
    buf.b_ind_unclosed.set(sw * 2);
    buf.b_ind_unclosed2.set(sw);
    buf.b_ind_unclosed_noignore.set(0);
    buf.b_ind_unclosed_wrapped.set(0);
    buf.b_ind_unclosed_whiteok.set(0);
    buf.b_ind_matching_paren.set(0);
    buf.b_ind_paren_prev.set(0);
    buf.b_ind_comment.set(0);
    buf.b_ind_in_comment.set(3);
    buf.b_ind_in_comment2.set(0);
    buf.b_ind_maxparen.set(20);
    buf.b_ind_maxcomment.set(70);
    buf.b_ind_java.set(0);
    buf.b_ind_js.set(0);
    buf.b_ind_keep_case_label.set(0);
    buf.b_ind_cpp_namespace.set(0);
    buf.b_ind_if_for_while.set(0);
    buf.b_ind_hash_comment.set(0);
    buf.b_ind_cpp_extern_c.set(0);
    buf.b_ind_pragma.set(0);
    let cino_rc = buf.b_p_cino.as_bytes();
    let mut rem = cino_rc.as_slice();
    while !rem.is_empty() {
        let l0 = rem[0];
        let is_neg = rem.get(1) == Some(&b'-');
        rem = &rem[1 + usize::from(is_neg)..];
        let digits_start_len = rem.len();
        let mut n: i64 = getdigits_int(&mut rem, true, 0) as i64;
        let mut divider: i32 = 0;
        if let Some(after_dot) = rem.strip_prefix(b".") {
            rem = after_dot;
            fraction = crate::charset::parse_i64_prefix(rem) as i32;
            while rem.first().is_some_and(u8::is_ascii_digit) {
                rem = &rem[1..];
                if divider != 0 {
                    divider *= 10;
                } else {
                    divider = 10;
                }
            }
        }
        if let Some(after_s) = rem.strip_prefix(b"s") {
            if rem.len() == digits_start_len {
                n = sw as i64;
            } else {
                n *= sw as i64;
                if divider != 0 {
                    n += (sw as i64 * fraction as i64 + (divider / 2) as i64) / divider as i64;
                }
            }
            rem = after_s;
        }
        if is_neg {
            n = -n;
        }
        let val = trim_to_int(n);
        match l0 {
            b'>' => buf.b_ind_level.set(val),
            b'e' => buf.b_ind_open_imag.set(val),
            b'n' => buf.b_ind_no_brace.set(val),
            b'f' => buf.b_ind_first_open.set(val),
            b'{' => buf.b_ind_open_extra.set(val),
            b'}' => buf.b_ind_close_extra.set(val),
            b'^' => buf.b_ind_open_left_imag.set(val),
            b'L' => buf.b_ind_jump_label.set(val),
            b':' => buf.b_ind_case.set(val),
            b'=' => buf.b_ind_case_code.set(val),
            b'b' => buf.b_ind_case_break.set(val),
            b'p' => buf.b_ind_param.set(val),
            b't' => buf.b_ind_func_type.set(val),
            b'/' => buf.b_ind_comment.set(val),
            b'c' => buf.b_ind_in_comment.set(val),
            b'C' => buf.b_ind_in_comment2.set(val),
            b'i' => buf.b_ind_cpp_baseclass.set(val),
            b'+' => buf.b_ind_continuation.set(val),
            b'(' => buf.b_ind_unclosed.set(val),
            b'u' => buf.b_ind_unclosed2.set(val),
            b'U' => buf.b_ind_unclosed_noignore.set(val),
            b'W' => buf.b_ind_unclosed_wrapped.set(val),
            b'w' => buf.b_ind_unclosed_whiteok.set(val),
            b'm' => buf.b_ind_matching_paren.set(val),
            b'M' => buf.b_ind_paren_prev.set(val),
            b')' => buf.b_ind_maxparen.set(val),
            b'*' => buf.b_ind_maxcomment.set(val),
            b'g' => buf.b_ind_scopedecl.set(val),
            b'h' => buf.b_ind_scopedecl_code.set(val),
            b'j' => buf.b_ind_java.set(val),
            b'J' => buf.b_ind_js.set(val),
            b'l' => buf.b_ind_keep_case_label.set(val),
            b'#' => buf.b_ind_hash_comment.set(val),
            b'N' => buf.b_ind_cpp_namespace.set(val),
            b'k' => buf.b_ind_if_for_while.set(val),
            b'E' => buf.b_ind_cpp_extern_c.set(val),
            b'P' => buf.b_ind_pragma.set(val),
            _ => {}
        }
        if let Some(after_comma) = rem.strip_prefix(b",") {
            rem = after_comma;
        }
    }
}
/// Return the desired indent for C code.
/// Return -1 if the indent should be left alone (inside a raw string).
pub fn get_c_indent() -> i32 {
    let mut amount: i32;
    let mut scope_amount: i32;
    let mut cur_amount: i32 = MAXCOL;
    let mut col: Colnr = 0;
    let mut trypos: Option<CursorPos>;
    let mut trypos_brace: Option<CursorPos> = None;
    let mut our_paren_pos: CursorPos;
    let start_brace: i32;
    let mut ourscope: Linenr;
    let mut l: &'static [u8] = &[];
    let mut look: &'static [u8] = &[];
    let mut terminated: u8;
    let mut lookfor: i32;
    let mut whilelevel: i32;
    let mut lnum: Linenr;
    let mut n: i32;
    let mut lookfor_break: bool;
    let mut lookfor_cpp_namespace = false;
    let mut cont_amount: i32 = 0;
    let mut added_to_amount: i32 = 0;
    let mut raw_string_start: Linenr = 0;
    let mut cache_cpp_baseclass = CppBaseclassCache {
        found: 0,
        lpos: Lpos {
            lnum: MAXLNUM,
            col: 0,
        },
    };
    let mut ind_continuation = curbuf_mut().b_ind_continuation.get();
    let cur_curpos = curwin_mut().w_cursor.get();
    if cur_curpos.lnum == 1 {
        return 0;
    }
    let mut linecopy_bytes = crate::memline::ml_get_slice(cur_curpos.lnum).to_vec();
    linecopy_bytes.push(0);
    if (g_st().state_global.get() & MODE_INSERT) != 0
        && (curwin_mut().w_cursor.get().col as usize) < linecopy_bytes.len().saturating_sub(1)
        && linecopy_bytes[curwin_mut().w_cursor.get().col as usize] == b')'
    {
        linecopy_bytes[curwin_mut().w_cursor.get().col as usize] = 0;
    }
    let linecopy_len = linecopy_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(linecopy_bytes.len());
    let linecopy_slice = &linecopy_bytes[..linecopy_len];
    let theline_slice = crate::charset::skipwhite_slice(linecopy_slice);
    let theline_b0 = theline_slice.first().copied().unwrap_or(0);
    curwin_mut().w_cursor.set_col(0);
    let original_line_islabel = cin_islabel();
    let comment_pos = ind_find_start_comment();
    trypos = find_start_rawstring(curbuf_mut().b_ind_maxcomment.get());
    'laterend: {
        if let Some(tp) = trypos
            && comment_pos.is_none_or(|cp| lt(tp, cp))
        {
            amount = -1;
            break 'laterend;
        }
        'theend: {
            if theline_b0 == b'#'
                && (linecopy_slice.first() == Some(&b'#')
                    || in_cinkeys(b'#' as i32, b' ' as i32, true))
            {
                let directive = crate::charset::skipwhite_slice(&theline_slice[1..]);
                if curbuf_mut().b_ind_pragma.get() == 0 || !directive.starts_with(b"pragma") {
                    amount = curbuf_mut().b_ind_hash_comment.get();
                    break 'theend;
                }
            }
            if original_line_islabel
                && curbuf_mut().b_ind_js.get() == 0
                && curbuf_mut().b_ind_jump_label.get() < 0
            {
                amount = 0;
                break 'theend;
            }
            if cin_islinecomment_slice(theline_slice) {
                trypos = find_line_comment();
                if trypos.is_none() && curwin_mut().w_cursor.get().lnum > 1 {
                    let lc_col = check_linecomment(crate::memline::ml_get_slice(
                        curwin_mut().w_cursor.get().lnum - 1,
                    ));
                    if lc_col != MAXCOL {
                        trypos = Some(CursorPos {
                            lnum: curwin_mut().w_cursor.get().lnum - 1,
                            col: lc_col,
                            coladd: 0,
                        });
                    }
                }
                if let Some(mut tp) = trypos {
                    getvcol(&curwin_mut(), &mut tp, Some(&mut col), None, None, 0);

                    amount = col;
                    break 'theend;
                }
            }
            if !cin_iscomment_slice(theline_slice)
                && let Some(mut cp) = comment_pos
            {
                let mut lead_start_len: usize = 2;
                let mut lead_middle_len: usize = 1;
                let mut lead_start: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
                let mut lead_middle: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
                let mut lead_end: [u8; COM_MAX_LEN] = [0; COM_MAX_LEN];
                let mut lead_end_len: usize;
                let mut start_align: i32 = 0;
                let mut start_off: i32 = 0;
                let mut done = false;

                getvcol(&curwin_mut(), &mut cp, Some(&mut col), None, None, 0);

                amount = col;
                lead_start[0] = 0;
                lead_middle[0] = 0;
                let com_rc = curbuf_mut().b_p_com.as_bytes();
                let mut p_bytes = com_rc.as_slice();
                while !p_bytes.is_empty() {
                    let mut align: i32 = 0;
                    let mut off: i32 = 0;
                    let mut what: i32 = 0;
                    while !p_bytes.is_empty() && p_bytes[0] != b':' {
                        let b = p_bytes[0];
                        if b == COM_START || b == COM_END || b == COM_MIDDLE {
                            what = i32::from(b);
                            p_bytes = &p_bytes[1..];
                        } else if b == COM_LEFT || b == COM_RIGHT {
                            align = i32::from(b);
                            p_bytes = &p_bytes[1..];
                        } else if ascii_isdigit(i32::from(b)) || b == b'-' {
                            off = getdigits_int(&mut p_bytes, true, 0);
                        } else {
                            p_bytes = &p_bytes[1..];
                        }
                    }
                    if p_bytes.first() == Some(&b':') {
                        p_bytes = &p_bytes[1..];
                    }
                    lead_end_len =
                        copy_option_part(&mut p_bytes, &mut lead_end[..COM_MAX_LEN], c",");
                    if what == i32::from(COM_START) {
                        lead_start = lead_end;
                        lead_start_len = lead_end_len;
                        start_off = off;
                        start_align = align;
                    } else if what == i32::from(COM_MIDDLE) {
                        lead_middle = lead_end;
                        lead_middle_len = lead_end_len;
                    } else if what == i32::from(COM_END) {
                        if theline_slice.starts_with(&lead_middle[..lead_middle_len])
                            && !theline_slice.starts_with(&lead_end[..lead_end_len])
                        {
                            done = true;
                            if curwin_mut().w_cursor.get().lnum > 1 {
                                let prev_look =
                                    crate::charset::skipwhite_slice(crate::memline::ml_get_slice(
                                        curwin_mut().w_cursor.get().lnum - 1,
                                    ));
                                if prev_look.starts_with(&lead_start[..lead_start_len]) {
                                    amount = get_indent_lnum(curwin_mut().w_cursor.get().lnum - 1);
                                } else if prev_look.starts_with(&lead_middle[..lead_middle_len]) {
                                    amount = get_indent_lnum(curwin_mut().w_cursor.get().lnum - 1);
                                    break;
                                } else {
                                    let cp_line = crate::memline::ml_get_slice(cp.lnum);
                                    let cp_col = (cp.col as usize).min(cp_line.len());
                                    if !cp_line[cp_col..].starts_with(&lead_start[..lead_start_len])
                                    {
                                        continue;
                                    }
                                }
                            }
                            if start_off != 0 {
                                amount += start_off;
                            } else if start_align == i32::from(COM_RIGHT) {
                                amount += vim_strsize(crate::memory::bytes_as_cstr(&lead_start))
                                    - vim_strsize(crate::memory::bytes_as_cstr(&lead_middle));
                            }
                            break;
                        }
                        if !theline_slice.starts_with(&lead_middle[..lead_middle_len])
                            && theline_slice.starts_with(&lead_end[..lead_end_len])
                        {
                            amount = get_indent_lnum(curwin_mut().w_cursor.get().lnum - 1);
                            if off != 0 {
                                amount += off;
                            } else if align == i32::from(COM_RIGHT) {
                                amount += vim_strsize(crate::memory::bytes_as_cstr(&lead_start))
                                    - vim_strsize(crate::memory::bytes_as_cstr(&lead_middle));
                            }
                            done = true;
                            break;
                        }
                    }
                }
                if done {
                    // skip
                } else if theline_b0 == b'*' {
                    amount += 1;
                } else {
                    amount = -1;
                    lnum = cur_curpos.lnum - 1;
                    while lnum > cp.lnum {
                        if linewhite(lnum) {
                            lnum -= 1;
                            continue;
                        }
                        amount = get_indent_lnum(lnum);
                        break;
                    }
                    if amount == -1 {
                        if curbuf_mut().b_ind_in_comment2.get() == 0 {
                            let start_slice = crate::memline::ml_get_slice(cp.lnum);
                            let off = (cp.col as usize + 2).min(start_slice.len());
                            look = &start_slice[off..];
                            if !look.is_empty() {
                                let trimmed = crate::charset::skipwhite_slice(look);

                                cp.col = (start_slice.len() - trimmed.len()) as Colnr;
                            }
                        }

                        getvcol(&curwin_mut(), &mut cp, Some(&mut col), None, None, 0);

                        amount = col;
                        if curbuf_mut().b_ind_in_comment2.get() != 0 || look.is_empty() {
                            amount += curbuf_mut().b_ind_in_comment.get();
                        }
                    }
                }
                break 'theend;
            }
            if theline_b0 == b']'
                && let Some(tp) = find_match_char(b'[', curbuf_mut().b_ind_maxparen.get())
            {
                amount = get_indent_lnum(tp.lnum);

                break 'theend;
            }
            if ({
                trypos = find_match_paren(curbuf_mut().b_ind_maxparen.get());
                trypos.is_some() && curbuf_mut().b_ind_java.get() == 0
            }) || ({
                trypos_brace = find_start_brace();
                trypos_brace.is_some()
            }) || trypos.is_some()
            {
                if let (Some(tp), Some(tb)) = (trypos, trypos_brace) {
                    if if tp.lnum != tb.lnum {
                        tp.lnum < tb.lnum
                    } else {
                        tp.col < tb.col
                    } {
                        trypos = None;
                    } else {
                        trypos_brace = None;
                    }
                }
                if let Some(tp) = trypos {
                    our_paren_pos = tp;

                    if theline_b0 == b')' && curbuf_mut().b_ind_paren_prev.get() != 0 {
                        amount = get_indent_lnum(curwin_mut().w_cursor.get().lnum - 1);
                    } else {
                        amount = -1;
                        lnum = cur_curpos.lnum - 1;
                        while lnum > our_paren_pos.lnum {
                            l = crate::charset::skipwhite_slice(crate::memline::ml_get_slice(lnum));
                            if cin_nocode_slice(l) {
                                lnum -= 1;
                                continue;
                            }
                            if cin_ispreproc_cont(&mut l, &mut lnum, &mut amount) {
                                lnum -= 1;
                                continue;
                            }
                            curwin_mut().w_cursor.set_lnum(lnum);
                            if let Some(tp) = ind_find_start_cors(None) {
                                lnum = tp.lnum;

                                continue;
                            }
                            if let Some(tp) = find_match_paren(corr_ind_maxparen(&cur_curpos))
                                && tp.lnum == our_paren_pos.lnum
                                && tp.col == our_paren_pos.col
                            {
                                amount = get_indent_lnum(lnum);
                                if theline_b0 == b')' {
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
                        let mut ignore_paren_col: i32 = 0;
                        let mut is_if_for_while: i32 = 0;
                        if curbuf_mut().b_ind_if_for_while.get() != 0 {
                            let cursor_save = curwin_mut().w_cursor.get();
                            let mut outermost: CursorPos;
                            trypos = Some(our_paren_pos);
                            loop {
                                outermost = trypos.unwrap();

                                curwin_mut().w_cursor.set_lnum(outermost.lnum);
                                curwin_mut().w_cursor.set_col(outermost.col);
                                trypos = find_match_paren(curbuf_mut().b_ind_maxparen.get());
                                if trypos.is_none_or(|tp| tp.lnum != outermost.lnum) {
                                    break;
                                }
                            }
                            curwin_mut().w_cursor.set(cursor_save);
                            let line = crate::memline::ml_get_slice(outermost.lnum);
                            is_if_for_while =
                                cin_is_if_for_while_before_offset(line, &mut outermost.col);
                        }
                        amount = skip_label(our_paren_pos.lnum, &mut look);
                        look = crate::charset::skipwhite_slice(look);
                        if look.first() == Some(&b'(') {
                            let save_lnum = curwin_mut().w_cursor.get().lnum;
                            curwin_mut().w_cursor.set_lnum(our_paren_pos.lnum);
                            let line = crate::cursor::get_cursor_line_slice();
                            let look_col = (line.len() - look.len()) as Colnr;
                            curwin_mut().w_cursor.set_col(look_col + 1);
                            if let Some(tp) = findmatchlimit(
                                None,
                                b')' as i32,
                                FM_SKIPCOMM,
                                curbuf_mut().b_ind_maxparen.get() as i64,
                            ) && tp.lnum == our_paren_pos.lnum
                                && tp.col < our_paren_pos.col
                            {
                                ignore_paren_col = tp.col + 1;
                            }
                            curwin_mut().w_cursor.set_lnum(save_lnum);
                            let paren_line = crate::memline::ml_get_slice(our_paren_pos.lnum);
                            look = &paren_line[(look_col as usize).min(paren_line.len())..];
                        }
                        if theline_b0 == b')'
                            || (curbuf_mut().b_ind_unclosed.get() == 0 && is_if_for_while == 0)
                            || (curbuf_mut().b_ind_unclosed_noignore.get() == 0
                                && look.first() == Some(&b'(')
                                && ignore_paren_col == 0)
                        {
                            if theline_b0 != b')' {
                                cur_amount = MAXCOL;
                                let l_slice = crate::memline::ml_get_slice(our_paren_pos.lnum);
                                if curbuf_mut().b_ind_unclosed_wrapped.get() != 0
                                    && cin_ends_in_slice(l_slice, b"(")
                                {
                                    n = 1;
                                    for &b in
                                        &l_slice[..(our_paren_pos.col as usize).min(l_slice.len())]
                                    {
                                        match b {
                                            b'(' | b'{' => n += 1,
                                            b')' | b'}' if n > 1 => n -= 1,
                                            _ => {}
                                        }
                                    }
                                    our_paren_pos.col = 0;
                                    amount += n * curbuf_mut().b_ind_unclosed_wrapped.get();
                                } else if curbuf_mut().b_ind_unclosed_whiteok.get() != 0 {
                                    our_paren_pos.col += 1;
                                } else {
                                    col = our_paren_pos.col + 1;
                                    while l_slice
                                        .get(col as usize)
                                        .is_some_and(|&b| ascii_iswhite(i32::from(b)))
                                    {
                                        col += 1;
                                    }
                                    if (col as usize) < l_slice.len() {
                                        our_paren_pos.col = col;
                                    } else {
                                        our_paren_pos.col += 1;
                                    }
                                }
                            }
                            if our_paren_pos.col > 0 {
                                getvcol(
                                    &curwin_mut(),
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
                        if theline_b0 == b')' && curbuf_mut().b_ind_matching_paren.get() != 0 {
                            // Line up with the start of the matching paren line.
                        } else if (curbuf_mut().b_ind_unclosed.get() == 0 && is_if_for_while == 0)
                            || (curbuf_mut().b_ind_unclosed_noignore.get() == 0
                                && look.first() == Some(&b'(')
                                && ignore_paren_col == 0)
                        {
                            if cur_amount != MAXCOL {
                                amount = cur_amount;
                            }
                        } else {
                            col = our_paren_pos.col;
                            while our_paren_pos.col > ignore_paren_col {
                                our_paren_pos.col -= 1;
                                match crate::memline::ml_get_pos_slice(&our_paren_pos)
                                    .first()
                                    .copied()
                                    .unwrap_or(0)
                                {
                                    b'(' => {
                                        amount += curbuf_mut().b_ind_unclosed2.get();
                                        col = our_paren_pos.col;
                                    }
                                    b')' => {
                                        amount -= curbuf_mut().b_ind_unclosed2.get();
                                        col = MAXCOL;
                                    }
                                    _ => {}
                                }
                            }
                            if col == MAXCOL {
                                amount += curbuf_mut().b_ind_unclosed.get();
                            } else {
                                curwin_mut().w_cursor.set_lnum(our_paren_pos.lnum);
                                curwin_mut().w_cursor.set_col(col);
                                if find_match_paren_after_brace(curbuf_mut().b_ind_maxparen.get())
                                    .is_some()
                                {
                                    amount += curbuf_mut().b_ind_unclosed2.get();
                                } else if is_if_for_while != 0 {
                                    amount += curbuf_mut().b_ind_if_for_while.get();
                                } else {
                                    amount += curbuf_mut().b_ind_unclosed.get();
                                }
                            }
                            if cur_amount < amount {
                                amount = cur_amount;
                            }
                        }
                    }
                    if cin_iscomment_slice(theline_slice) {
                        amount += curbuf_mut().b_ind_comment.get();
                    }
                } else {
                    let mut tb = trypos_brace.unwrap();
                    ourscope = tb.lnum;

                    let start_slice = crate::memline::ml_get_slice(ourscope);
                    let look_slice = crate::charset::skipwhite_slice(start_slice);
                    if look_slice.first() == Some(&b'{') {
                        getvcol(&curwin_mut(), &mut tb, Some(&mut col), None, None, 0);

                        amount = col;
                        if start_slice.first() == Some(&b'{') {
                            start_brace = BRACE_IN_COL0;
                        } else {
                            start_brace = BRACE_AT_START;
                        }
                    } else {
                        curwin_mut().w_cursor.set_lnum(ourscope);
                        lnum = ourscope;
                        if find_last_paren_slice(start_slice, b'(', b')')
                            && let Some(tp) = find_match_paren(curbuf_mut().b_ind_maxparen.get())
                        {
                            lnum = tp.lnum;
                        }
                        if (curbuf_mut().b_ind_js.get() != 0
                            || curbuf_mut().b_ind_keep_case_label.get() != 0)
                            && cin_iscase_slice(
                                crate::charset::skipwhite_slice(
                                    crate::cursor::get_cursor_line_slice(),
                                ),
                                false,
                            )
                        {
                            amount = get_indent();
                        } else if curbuf_mut().b_ind_js.get() != 0 {
                            amount = get_indent_lnum(lnum);
                        } else {
                            amount = skip_label(lnum, &mut l);
                        }
                        start_brace = BRACE_AT_END;
                    }
                    let mut js_cur_has_key = if curbuf_mut().b_ind_js.get() != 0 {
                        cin_has_js_key(theline_slice)
                    } else {
                        false
                    };
                    if theline_b0 == b'}' {
                        amount += curbuf_mut().b_ind_close_extra.get();
                    } else {
                        lookfor = LOOKFOR_INITIAL;
                        if cin_iselse_slice(theline_slice) {
                            lookfor = LOOKFOR_IF;
                        } else if cin_iswhileofdo(theline_slice, cur_curpos.lnum) {
                            lookfor = LOOKFOR_DO;
                        }
                        if lookfor != LOOKFOR_INITIAL {
                            curwin_mut().w_cursor.set_lnum(cur_curpos.lnum);
                            if find_match(lookfor, ourscope) {
                                amount = get_indent();
                                break 'theend;
                            }
                        }
                        if start_brace == BRACE_IN_COL0 {
                            amount = curbuf_mut().b_ind_open_left_imag.get();
                            lookfor_cpp_namespace = true;
                        } else if start_brace == BRACE_AT_START && lookfor_cpp_namespace {
                            lookfor_cpp_namespace = true;
                        } else if start_brace == BRACE_AT_END {
                            amount += curbuf_mut().b_ind_open_imag.get();
                            let l_slice = crate::charset::skipwhite_slice(
                                crate::cursor::get_cursor_line_slice(),
                            );
                            if cin_is_cpp_namespace_slice(l_slice) {
                                amount += curbuf_mut().b_ind_cpp_namespace.get();
                            } else if cin_is_cpp_extern_c_slice(l_slice) {
                                amount += curbuf_mut().b_ind_cpp_extern_c.get();
                            }
                        } else {
                            amount -= curbuf_mut().b_ind_open_extra.get();
                            if amount < 0 {
                                amount = 0;
                            }
                        }
                        lookfor_break = false;
                        if cin_iscase_slice(theline_slice, false) {
                            lookfor = LOOKFOR_CASE;
                            amount += curbuf_mut().b_ind_case.get();
                        } else if cin_isscopedecl_slice(theline_slice) {
                            lookfor = LOOKFOR_SCOPEDECL;
                            amount += curbuf_mut().b_ind_scopedecl.get();
                        } else {
                            if curbuf_mut().b_ind_case_break.get() != 0
                                && cin_isbreak_slice(theline_slice)
                            {
                                lookfor_break = true;
                            }
                            lookfor = LOOKFOR_INITIAL;
                            amount += curbuf_mut().b_ind_level.get();
                        }
                        scope_amount = amount;
                        whilelevel = 0;
                        curwin_mut().w_cursor.set(cur_curpos);
                        'scope_loop: loop {
                            curwin_mut().w_cursor.sub_lnum(1);
                            curwin_mut().w_cursor.set_col(0);
                            if curwin_mut().w_cursor.get().lnum <= ourscope {
                                if lookfor == LOOKFOR_ENUM_OR_INIT {
                                    if curwin_mut().w_cursor.get().lnum == 0
                                        || curwin_mut().w_cursor.get().lnum
                                            < ourscope - curbuf_mut().b_ind_maxparen.get()
                                    {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else if curbuf_mut().b_ind_js.get() == 0 {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    }
                                    if let Some(tp) = ind_find_start_cors(None) {
                                        curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                                        curwin_mut().w_cursor.set_col(0);
                                        continue;
                                    }
                                    l = crate::cursor::get_cursor_line_slice();
                                    if curwin_mut().w_cursor.update_mut(|c| {
                                        cin_ispreproc_cont(&mut l, &mut c.lnum, &mut amount)
                                    }) {
                                        continue;
                                    }
                                    if cin_nocode_slice(l) {
                                        continue;
                                    }
                                    terminated = cin_isterminated_slice(l, false, true);
                                    if start_brace != BRACE_IN_COL0
                                        || !cin_isfuncdecl(
                                            Some(&mut l),
                                            curwin_mut().w_cursor.get().lnum,
                                            0,
                                        )
                                    {
                                        if terminated == b',' {
                                            break;
                                        }
                                        if terminated != b';' && cin_isinit() {
                                            break;
                                        }
                                        if terminated == 0 || terminated == b'{' {
                                            continue;
                                        }
                                    }
                                    if terminated != b';' {
                                        trypos = None;
                                        if find_last_paren_slice(l, b'(', b')') {
                                            trypos =
                                                find_match_paren(curbuf_mut().b_ind_maxparen.get());
                                        }
                                        if trypos.is_none() && find_last_paren_slice(l, b'{', b'}')
                                        {
                                            trypos = find_start_brace();
                                        }
                                        if let Some(tp) = trypos {
                                            curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                                            curwin_mut().w_cursor.set_col(0);
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
                                        if theline_b0 == b'{' {
                                            amount += curbuf_mut().b_ind_open_extra.get();
                                            added_to_amount = curbuf_mut().b_ind_open_extra.get();
                                        }
                                    }
                                    if lookfor_cpp_namespace {
                                        if curwin_mut().w_cursor.get().lnum == ourscope {
                                            continue;
                                        }
                                        if curwin_mut().w_cursor.get().lnum == 0
                                            || curwin_mut().w_cursor.get().lnum
                                                < ourscope - FIND_NAMESPACE_LIM
                                        {
                                            break;
                                        }
                                        if let Some(tp) = ind_find_start_cors(None) {
                                            curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                                            curwin_mut().w_cursor.set_col(0);
                                            continue;
                                        }
                                        l = crate::cursor::get_cursor_line_slice();
                                        if curwin_mut().w_cursor.update_mut(|c| {
                                            cin_ispreproc_cont(&mut l, &mut c.lnum, &mut amount)
                                        }) {
                                            continue;
                                        }
                                        if cin_is_cpp_namespace_slice(l) {
                                            amount += curbuf_mut().b_ind_cpp_namespace.get()
                                                - added_to_amount;
                                            break;
                                        } else if cin_is_cpp_extern_c_slice(l) {
                                            amount += curbuf_mut().b_ind_cpp_extern_c.get()
                                                - added_to_amount;
                                            break;
                                        }
                                        if cin_nocode_slice(l) {
                                            continue;
                                        }
                                    }
                                }
                                break;
                            }
                            if let Some(tp) = ind_find_start_cors(Some(&mut raw_string_start)) {
                                curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                                curwin_mut().w_cursor.set_col(0);
                                continue;
                            }
                            let l_slice = crate::cursor::get_cursor_line_slice();
                            let mut iscase = cin_iscase_slice(l_slice, false);
                            if iscase || cin_isscopedecl_slice(l_slice) {
                                if lookfor == LOOKFOR_CPP_BASECLASS {
                                    break;
                                }
                                if whilelevel > 0 {
                                    continue;
                                }
                                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
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
                                    if trypos.is_none_or(|tp| tp.lnum == ourscope) {
                                        amount = get_indent();
                                        break;
                                    }
                                    continue;
                                }
                                n = get_indent_nolabel(curwin_mut().w_cursor.get().lnum);
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
                                    if let Some(after_l) =
                                        after_label_slice(crate::cursor::get_cursor_line_slice())
                                        && cin_is_cinword(after_l)
                                    {
                                        if theline_b0 == b'{' {
                                            amount += curbuf_mut().b_ind_open_extra.get();
                                        } else {
                                            amount += curbuf_mut().b_ind_level.get()
                                                + curbuf_mut().b_ind_no_brace.get();
                                        }
                                    }
                                    break;
                                }
                                scope_amount = get_indent()
                                    + if iscase {
                                        curbuf_mut().b_ind_case_code.get()
                                    } else {
                                        curbuf_mut().b_ind_scopedecl_code.get()
                                    };
                                lookfor = if curbuf_mut().b_ind_case_break.get() != 0 {
                                    LOOKFOR_NOBREAK
                                } else {
                                    LOOKFOR_ANY
                                };
                                continue;
                            }
                            if lookfor == LOOKFOR_CASE || lookfor == LOOKFOR_SCOPEDECL {
                                if find_last_paren_slice(l_slice, b'{', b'}')
                                    && let Some(tp) = find_start_brace()
                                {
                                    curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                                    curwin_mut().w_cursor.set_col(0);
                                }
                                continue;
                            }
                            if curbuf_mut().b_ind_js.get() == 0 && cin_islabel() {
                                let Some(after_l) =
                                    after_label_slice(crate::cursor::get_cursor_line_slice())
                                else {
                                    continue;
                                };
                                if cin_nocode_slice(after_l) {
                                    continue;
                                }
                            }
                            l = crate::cursor::get_cursor_line_slice();
                            if curwin_mut().w_cursor.update_mut(|c| {
                                cin_ispreproc_cont(&mut l, &mut c.lnum, &mut amount)
                            }) || cin_nocode_slice(l)
                            {
                                continue;
                            }
                            n = 0;
                            if lookfor != LOOKFOR_TERM && curbuf_mut().b_ind_cpp_baseclass.get() > 0
                            {
                                n = i32::from(cin_is_cpp_baseclass(&mut cache_cpp_baseclass));
                                l = crate::cursor::get_cursor_line_slice();
                            }
                            if n != 0 {
                                if lookfor == LOOKFOR_UNTERM {
                                    if cont_amount > 0 {
                                        amount = cont_amount;
                                    } else {
                                        amount += ind_continuation;
                                    }
                                } else if theline_b0 == b'{' {
                                    lookfor = LOOKFOR_UNTERM;
                                    ind_continuation = 0;
                                    continue;
                                } else {
                                    amount = get_baseclass_amount(cache_cpp_baseclass.lpos.col);
                                }
                                break;
                            } else if lookfor == LOOKFOR_CPP_BASECLASS {
                                if cin_isterminated_slice(l, true, false) != 0 {
                                    break;
                                } else {
                                    continue;
                                }
                            }
                            terminated = cin_isterminated_slice(l, false, true);
                            if js_cur_has_key {
                                js_cur_has_key = false;
                                if curbuf_mut().b_ind_js.get() != 0 && terminated == b',' {
                                    lookfor = LOOKFOR_JS_KEY;
                                }
                            }
                            if lookfor == LOOKFOR_JS_KEY && cin_has_js_key(l) {
                                amount = get_indent();
                                break;
                            }
                            if lookfor == LOOKFOR_COMMA {
                                if trypos_brace
                                    .is_some_and(|tb| tb.lnum >= curwin_mut().w_cursor.get().lnum)
                                {
                                    break;
                                }
                                if terminated == b',' {
                                    break;
                                }
                                amount = get_indent();
                                if curwin_mut().w_cursor.get().lnum - 1 == ourscope {
                                    break;
                                }
                            }
                            if terminated == 0 || (lookfor != LOOKFOR_UNTERM && terminated == b',')
                            {
                                if lookfor != LOOKFOR_ENUM_OR_INIT
                                    && (crate::charset::skipwhite_slice(l).first() == Some(&b'[')
                                        || l.last() == Some(&b'['))
                                {
                                    amount += ind_continuation;
                                }
                                let _ = find_last_paren_slice(l, b'(', b')');
                                trypos = find_match_paren(corr_ind_maxparen(&cur_curpos));
                                if let (Some(tp), Some(tb)) = (trypos, trypos_brace)
                                    && (tp.lnum < tb.lnum
                                        || (tp.lnum == tb.lnum && tp.col < tb.col))
                                {
                                    trypos = None;
                                }
                                if trypos.is_none()
                                    && terminated == b','
                                    && find_last_paren_slice(
                                        crate::cursor::get_cursor_line_slice(),
                                        b'{',
                                        b'}',
                                    )
                                {
                                    trypos = find_start_brace();
                                }
                                if let Some(tp) = trypos {
                                    curwin_mut().w_cursor.set(tp);

                                    let cur_s = crate::cursor::get_cursor_line_slice();
                                    if cin_iscase_slice(cur_s, false)
                                        || cin_isscopedecl_slice(cur_s)
                                    {
                                        curwin_mut().w_cursor.add_lnum(1);
                                        curwin_mut().w_cursor.set_col(0);
                                        continue;
                                    }
                                }
                                if terminated == b',' {
                                    while curwin_mut().w_cursor.get().lnum > 1 {
                                        let prev_s = crate::memline::ml_get_slice(
                                            curwin_mut().w_cursor.get().lnum - 1,
                                        );
                                        if prev_s.last() != Some(&b'\\') {
                                            break;
                                        }
                                        curwin_mut().w_cursor.sub_lnum(1);
                                        curwin_mut().w_cursor.set_col(0);
                                    }
                                }
                                l = crate::cursor::get_cursor_line_slice();
                                if curbuf_mut().b_ind_js.get() != 0 {
                                    cur_amount = get_indent();
                                } else {
                                    cur_amount =
                                        skip_label(curwin_mut().w_cursor.get().lnum, &mut l);
                                }
                                if terminated != b','
                                    && lookfor != LOOKFOR_TERM
                                    && theline_b0 == b'{'
                                {
                                    amount = cur_amount;
                                    if crate::charset::skipwhite_slice(l).first() != Some(&b'{') {
                                        amount += curbuf_mut().b_ind_open_extra.get();
                                    }
                                    if curbuf_mut().b_ind_cpp_baseclass.get() != 0
                                        && curbuf_mut().b_ind_js.get() == 0
                                    {
                                        lookfor = LOOKFOR_CPP_BASECLASS;
                                        continue;
                                    }
                                    break;
                                }
                                if cin_is_cinword(l)
                                    || cin_iselse_slice(crate::charset::skipwhite_slice(l))
                                {
                                    if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT
                                    {
                                        if cont_amount > 0 {
                                            amount = cont_amount;
                                        } else {
                                            amount += ind_continuation;
                                        }
                                        break;
                                    }
                                    amount = cur_amount;
                                    if theline_b0 == b'{' {
                                        amount += curbuf_mut().b_ind_open_extra.get();
                                    }
                                    if lookfor != LOOKFOR_TERM {
                                        amount += curbuf_mut().b_ind_level.get()
                                            + curbuf_mut().b_ind_no_brace.get();
                                        break;
                                    }
                                    let cur_s = crate::cursor::get_cursor_line_slice();
                                    let l_trim = crate::charset::skipwhite_slice(cur_s);
                                    if cin_isdo_slice(l_trim) {
                                        if whilelevel == 0 {
                                            break;
                                        }
                                        whilelevel -= 1;
                                    }
                                    if cin_iselse_slice(l_trim) && whilelevel == 0 {
                                        if l_trim.first() == Some(&b'}') {
                                            curwin_mut()
                                                .w_cursor
                                                .set_col((cur_s.len() - l_trim.len()) as Colnr + 1);
                                        }
                                        trypos = find_start_brace();
                                        if trypos.is_none_or(|tp| !find_match(LOOKFOR_IF, tp.lnum))
                                        {
                                            break;
                                        }
                                    }
                                } else if lookfor == LOOKFOR_UNTERM {
                                    if terminated == b',' {
                                        amount += ind_continuation;
                                    }
                                    break;
                                } else if lookfor == LOOKFOR_ENUM_OR_INIT {
                                    if terminated == b',' {
                                        if curbuf_mut().b_ind_cpp_baseclass.get() == 0 {
                                            break;
                                        }
                                        lookfor = LOOKFOR_CPP_BASECLASS;
                                        continue;
                                    }
                                    if amount > cur_amount {
                                        amount = cur_amount;
                                    }
                                } else {
                                    let cur_s = crate::cursor::get_cursor_line_slice();
                                    amount = cur_amount;
                                    let n_len = cur_s.len();
                                    if curbuf_mut().b_ind_js.get() != 0
                                        && terminated == b','
                                        && (crate::charset::skipwhite_slice(cur_s).first()
                                            == Some(&b']')
                                            || (n_len >= 2 && cur_s[n_len - 2] == b']'))
                                    {
                                        break;
                                    }
                                    if lookfor == LOOKFOR_INITIAL && terminated == b',' {
                                        if curbuf_mut().b_ind_js.get() != 0 {
                                            if cin_iscomment_slice(crate::charset::skipwhite_slice(
                                                cur_s,
                                            )) {
                                                break;
                                            }
                                            lookfor = LOOKFOR_COMMA;
                                            if let Some(tp) = find_match_char(
                                                b'[',
                                                curbuf_mut().b_ind_maxparen.get(),
                                            ) {
                                                if tp.lnum == curwin_mut().w_cursor.get().lnum - 1 {
                                                    break;
                                                }
                                                ourscope = tp.lnum;
                                            }
                                        } else {
                                            lookfor = LOOKFOR_ENUM_OR_INIT;
                                            cont_amount = cin_first_id_amount();
                                        }
                                    } else {
                                        if lookfor == LOOKFOR_INITIAL
                                            && cur_s.last() == Some(&b'\\')
                                        {
                                            cont_amount = cin_get_equal_amount(
                                                curwin_mut().w_cursor.get().lnum,
                                            );
                                        }
                                        if lookfor != LOOKFOR_TERM
                                            && lookfor != LOOKFOR_JS_KEY
                                            && lookfor != LOOKFOR_COMMA
                                            && raw_string_start != curwin_mut().w_cursor.get().lnum
                                        {
                                            lookfor = LOOKFOR_UNTERM;
                                        }
                                    }
                                }
                            } else if cin_iswhileofdo_end(i32::from(terminated)) {
                                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
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
                                    if theline_b0 == b'{' {
                                        amount += curbuf_mut().b_ind_open_extra.get();
                                    }
                                }
                                whilelevel += 1;
                            } else {
                                if lookfor == LOOKFOR_NOBREAK
                                    && cin_isbreak_slice(crate::charset::skipwhite_slice(
                                        crate::cursor::get_cursor_line_slice(),
                                    ))
                                {
                                    lookfor = LOOKFOR_ANY;
                                    continue;
                                }
                                if whilelevel > 0 {
                                    let cur_s = cin_skipcomment_slice(
                                        crate::cursor::get_cursor_line_slice(),
                                    );
                                    if cin_isdo_slice(cur_s) {
                                        amount = get_indent();
                                        whilelevel -= 1;
                                        continue;
                                    }
                                }
                                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
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
                                        let cur_s = crate::cursor::get_cursor_line_slice();
                                        if find_last_paren_slice(cur_s, b'(', b')')
                                            && let Some(tp) =
                                                find_match_paren(curbuf_mut().b_ind_maxparen.get())
                                        {
                                            curwin_mut().w_cursor.set(tp);

                                            let tp_s = crate::cursor::get_cursor_line_slice();
                                            if cin_iscase_slice(tp_s, false)
                                                || cin_isscopedecl_slice(tp_s)
                                            {
                                                curwin_mut().w_cursor.add_lnum(1);
                                                curwin_mut().w_cursor.set_col(0);
                                                continue 'scope_loop;
                                            }
                                        }
                                        let cur_s = crate::cursor::get_cursor_line_slice();
                                        iscase = curbuf_mut().b_ind_keep_case_label.get() != 0
                                            && cin_iscase_slice(cur_s, false);
                                        amount =
                                            skip_label(curwin_mut().w_cursor.get().lnum, &mut l);
                                        if theline_b0 == b'{' {
                                            amount += curbuf_mut().b_ind_open_extra.get();
                                        }
                                        let l_trim = crate::charset::skipwhite_slice(l);
                                        if l_trim.first() == Some(&b'{') {
                                            amount -= curbuf_mut().b_ind_open_extra.get();
                                        }
                                        lookfor = if iscase { LOOKFOR_ANY } else { LOOKFOR_TERM };
                                        if lookfor == LOOKFOR_TERM
                                            && l_trim.first() != Some(&b'}')
                                            && cin_iselse_slice(l_trim)
                                            && whilelevel == 0
                                        {
                                            trypos = find_start_brace();
                                            if trypos
                                                .is_none_or(|tp| !find_match(LOOKFOR_IF, tp.lnum))
                                            {
                                                break 'scope_loop;
                                            }
                                            continue 'scope_loop;
                                        }
                                        let cur_s = crate::cursor::get_cursor_line_slice();
                                        if find_last_paren_slice(cur_s, b'{', b'}')
                                            && let Some(tp) = find_start_brace()
                                        {
                                            curwin_mut().w_cursor.set(tp);

                                            let tp_s = cin_skipcomment_slice(
                                                crate::cursor::get_cursor_line_slice(),
                                            );
                                            if tp_s.first() == Some(&b'}')
                                                || !cin_iselse_slice(tp_s)
                                            {
                                                continue;
                                            }
                                            curwin_mut().w_cursor.add_lnum(1);
                                            curwin_mut().w_cursor.set_col(0);
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                if cin_iscomment_slice(theline_slice) {
                    amount += curbuf_mut().b_ind_comment.get();
                }
                if curbuf_mut().b_ind_jump_label.get() > 0 && original_line_islabel {
                    amount -= curbuf_mut().b_ind_jump_label.get();
                }
                break 'theend;
            }
            if theline_b0 == b'{' {
                amount = curbuf_mut().b_ind_first_open.get();
                break 'theend;
            }
            if cur_curpos.lnum < curbuf_mut().b_ml.ml_line_count.get()
                && !cin_nocode_slice(theline_slice)
                && !theline_slice.contains(&b'{')
                && !theline_slice.contains(&b'}')
                && !cin_ends_in_slice(theline_slice, b":")
                && !cin_ends_in_slice(theline_slice, b",")
                && cin_isfuncdecl(None, cur_curpos.lnum + 1, cur_curpos.lnum + 1)
                && cin_isterminated_slice(theline_slice, false, true) == 0
            {
                amount = curbuf_mut().b_ind_func_type.get();
                break 'theend;
            }
            amount = 0;
            curwin_mut().w_cursor.set(cur_curpos);
            while curwin_mut().w_cursor.get().lnum > 1 {
                curwin_mut().w_cursor.sub_lnum(1);
                curwin_mut().w_cursor.set_col(0);
                l = crate::cursor::get_cursor_line_slice();
                if let Some(tp) = ind_find_start_cors(None) {
                    curwin_mut().w_cursor.set_lnum(tp.lnum + 1);

                    curwin_mut().w_cursor.set_col(0);
                    continue;
                }
                n = 0;
                if curbuf_mut().b_ind_cpp_baseclass.get() != 0 {
                    n = i32::from(cin_is_cpp_baseclass(&mut cache_cpp_baseclass));
                    l = crate::cursor::get_cursor_line_slice();
                }
                if n != 0 {
                    amount = get_baseclass_amount(cache_cpp_baseclass.lpos.col);
                    break;
                }
                if curwin_mut()
                    .w_cursor
                    .update_mut(|c| cin_ispreproc_cont(&mut l, &mut c.lnum, &mut amount))
                {
                    continue;
                }
                if cin_nocode_slice(l) {
                    continue;
                }
                if cin_ends_in_slice(l, b",") || {
                    n = l.last().copied().map_or(0, i32::from);
                    n == b'\\' as i32
                } {
                    if find_last_paren_slice(l, b'(', b')')
                        && let Some(tp) = find_match_paren(curbuf_mut().b_ind_maxparen.get())
                    {
                        curwin_mut().w_cursor.set(tp);
                    }
                    while n == 0 && curwin_mut().w_cursor.get().lnum > 1 {
                        let prev_s =
                            crate::memline::ml_get_slice(curwin_mut().w_cursor.get().lnum - 1);
                        if prev_s.last() != Some(&b'\\') {
                            break;
                        }
                        curwin_mut().w_cursor.sub_lnum(1);
                        curwin_mut().w_cursor.set_col(0);
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
                if cin_isfuncdecl(None, cur_curpos.lnum, 0) {
                    break;
                }
                let l_bytes = crate::cursor::get_cursor_line_slice();
                if crate::charset::skipwhite_slice(l_bytes).first() == Some(&b'}') {
                    break;
                }
                if cin_ends_in_slice(l_bytes, b"};") {
                    break;
                }
                if cin_ends_in_slice(l_bytes, b"[") {
                    amount = get_indent() + ind_continuation;
                    break;
                }
                let look_s = crate::charset::skipwhite_slice(l_bytes);
                if look_s.first() == Some(&b';') && cin_nocode_slice(&look_s[1..]) {
                    let curpos_save = curwin_mut().w_cursor.get();
                    while curwin_mut().w_cursor.get().lnum > 1 {
                        curwin_mut().w_cursor.sub_lnum(1);
                        look = crate::memline::ml_get_slice(curwin_mut().w_cursor.get().lnum);
                        if !(cin_nocode_slice(look)
                            || curwin_mut().w_cursor.update_mut(|c| {
                                cin_ispreproc_cont(&mut look, &mut c.lnum, &mut amount)
                            }))
                        {
                            break;
                        }
                    }
                    if curwin_mut().w_cursor.get().lnum > 0 && cin_ends_in_slice(look, b"}") {
                        break;
                    }
                    curwin_mut().w_cursor.set(curpos_save);
                }
                l = crate::cursor::get_cursor_line_slice();
                if cin_isfuncdecl(Some(&mut l), curwin_mut().w_cursor.get().lnum, 0) {
                    amount = curbuf_mut().b_ind_param.get();
                    break;
                }
                if cin_ends_in_slice(l, b";") {
                    let prev_s = crate::memline::ml_get_slice(curwin_mut().w_cursor.get().lnum - 1);
                    if cin_ends_in_slice(prev_s, b",") || prev_s.last() == Some(&b'\\') {
                        break;
                    }
                }
                let _ = find_last_paren_slice(crate::cursor::get_cursor_line_slice(), b'(', b')');
                if let Some(tp) = find_match_paren(curbuf_mut().b_ind_maxparen.get()) {
                    curwin_mut().w_cursor.set(tp);
                }
                amount = get_indent();
                break;
            }
            if cin_iscomment_slice(theline_slice) {
                amount += curbuf_mut().b_ind_comment.get();
            }
            if cur_curpos.lnum > 1 {
                let prev_s = crate::memline::ml_get_slice(cur_curpos.lnum - 1);
                if prev_s.last() == Some(&b'\\') {
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
    curwin_mut().w_cursor.set(cur_curpos);
    amount
}
fn find_match(lookfor: i32, ourscope: Linenr) -> bool {
    let mut elselevel: i32;
    let mut whilelevel: i32;
    if lookfor == LOOKFOR_IF {
        elselevel = 1;
        whilelevel = 0;
    } else {
        elselevel = 0;
        whilelevel = 1;
    }
    curwin_mut().w_cursor.set_col(0);
    while curwin_mut().w_cursor.get().lnum > ourscope + 1 {
        curwin_mut().w_cursor.sub_lnum(1);
        curwin_mut().w_cursor.set_col(0);
        let look = cin_skipcomment_slice(crate::cursor::get_cursor_line_slice());
        if !cin_iselse_slice(look)
            && !cin_isif_slice(look)
            && !cin_isdo_slice(look)
            && !cin_iswhileofdo(look, curwin_mut().w_cursor.get().lnum)
        {
            continue;
        }
        let Some(theirscope) = find_start_brace() else {
            break;
        };
        if theirscope.lnum < ourscope {
            break;
        }
        if theirscope.lnum > ourscope {
            continue;
        }
        let look = cin_skipcomment_slice(crate::cursor::get_cursor_line_slice());
        if !(lookfor == LOOKFOR_IF && whilelevel != 0) {
            if cin_iselse_slice(look) {
                let mightbeif = cin_skipcomment_slice(&look[4.min(look.len())..]);
                if !cin_isif_slice(mightbeif) {
                    elselevel += 1;
                }
                continue;
            }
            if cin_isif_slice(look) {
                elselevel -= 1;
                if elselevel == 0 && lookfor == LOOKFOR_IF {
                    whilelevel = 0;
                }
            }
        }
        if cin_iswhileofdo(look, curwin_mut().w_cursor.get().lnum) {
            whilelevel += 1;
            continue;
        }
        if cin_isdo_slice(look) {
            whilelevel -= 1;
        }
        if elselevel <= 0 && whilelevel <= 0 {
            return true;
        }
    }
    false
}
/// Check that "cinkeys" contains the key "keytyped".
pub fn in_cinkeys(keytyped: i32, when: i32, line_is_empty: bool) -> bool {
    if keytyped == NUL as i32 {
        return false;
    }
    let look_full = if curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST {
        curbuf_mut().b_p_indk.as_bytes()
    } else {
        curbuf_mut().b_p_cink.as_bytes()
    };
    let mut rem = look_full.as_slice();
    while !rem.is_empty() {
        let mut try_match = match when as u8 {
            b'*' => rem[0] == b'*',
            b'!' => rem[0] == b'!',
            _ => rem[0] != b'*',
        };
        if matches!(rem[0], b'*' | b'!') {
            rem = &rem[1..];
        }
        let try_match_word = if let Some(after_zero) = rem.strip_prefix(b"0") {
            let tmw = try_match;
            if !line_is_empty {
                try_match = false;
            }
            rem = after_zero;
            tmw
        } else {
            false
        };
        let b0 = rem.first().copied().unwrap_or(0);
        if b0 == b'^' && rem.get(1).is_some_and(|&b1| (b'?'..=b'_').contains(&b1)) {
            if try_match && keytyped == ctrl_chr(i32::from(rem[1])) {
                return true;
            }
            rem = &rem[2..];
        } else if b0 == b'o' {
            if try_match && keytyped == KEY_OPEN_FORW {
                return true;
            }
            rem = &rem[1..];
        } else if b0 == b'O' {
            if try_match && keytyped == KEY_OPEN_BACK {
                return true;
            }
            rem = &rem[1..];
        } else if b0 == b'e' {
            if try_match && keytyped == b'e' as i32 && curwin_mut().w_cursor.get().col >= 4 {
                let line = crate::cursor::get_cursor_line_slice();
                let col_usize = curwin_mut().w_cursor.get().col as usize;
                if col_usize <= line.len()
                    && crate::charset::skipwhite_slice(line).len() == line.len() - (col_usize - 4)
                    && &line[col_usize - 4..col_usize] == b"else"
                {
                    return true;
                }
            }
            rem = &rem[1..];
        } else if b0 == b':' {
            if try_match && keytyped == b':' as i32 {
                let line_slice = crate::cursor::get_cursor_line_slice();
                if cin_iscase_slice(line_slice, false)
                    || cin_isscopedecl_slice(line_slice)
                    || cin_islabel()
                {
                    return true;
                }
                let col_usize = curwin_mut().w_cursor.get().col as usize;
                let line_slice = crate::cursor::get_cursor_line_slice();
                if col_usize > 2
                    && line_slice.get(col_usize - 1) == Some(&b':')
                    && line_slice.get(col_usize - 2) == Some(&b':')
                {
                    crate::cursor::with_cursor_line_mut_slice(|s| s[col_usize - 1] = b' ');

                    let line_slice = crate::cursor::get_cursor_line_slice();
                    let i = cin_iscase_slice(line_slice, false)
                        || cin_isscopedecl_slice(line_slice)
                        || cin_islabel();

                    crate::cursor::with_cursor_line_mut_slice(|s| s[col_usize - 1] = b':');

                    if i {
                        return true;
                    }
                }
            }
            rem = &rem[1..];
        } else if b0 == b'<' {
            if try_match {
                let b1 = rem.get(1).copied().unwrap_or(0);
                if matches!(
                    b1,
                    b'<' | b'>' | b'!' | b'*' | b'o' | b'O' | b'e' | b'0' | b':'
                ) && keytyped == i32::from(b1)
                {
                    return true;
                }
                let look_slice = &look_full[look_full.len() - rem.len() + 1..];
                if keytyped == get_special_key_code(look_slice) {
                    return true;
                }
            }
            while !rem.is_empty() && rem[0] != b'>' {
                rem = &rem[1..];
            }
            while rem.first() == Some(&b'>') {
                rem = &rem[1..];
            }
        } else if b0 == b'=' && !matches!(rem.get(1), None | Some(&b',')) {
            rem = &rem[1..];
            let icase = if let Some(after_tilde) = rem.strip_prefix(b"~") {
                rem = after_tilde;
                true
            } else {
                false
            };
            let word_len = rem.iter().position(|&b| b == b',').unwrap_or(rem.len());
            let word = &rem[..word_len];
            if (try_match || try_match_word)
                && (curwin_mut().w_cursor.get().col as usize) >= word_len
            {
                let mut matched = false;
                if keytyped == KEY_COMPLETE {
                    let line_bytes = crate::cursor::get_cursor_line_slice();
                    let mut s_off = curwin_mut().w_cursor.get().col as usize;
                    while s_off > 0 {
                        let n_off = s_off
                            - 1
                            - crate::mbyte::utf_head_off_slice(line_bytes, s_off - 1) as usize;
                        if !vim_iswordp(line_bytes.get(n_off..).unwrap_or(&[])) {
                            break;
                        }
                        s_off = n_off;
                    }
                    let line = get_cursor_line_ptr().as_ptr().cast_mut();
                    let s = line.wrapping_add(s_off);
                    let s_off = ((s as isize) - (line as isize)) as usize;
                    let col_usize = curwin_mut().w_cursor.get().col as usize;
                    if s_off + word_len <= col_usize {
                        let line_slice = crate::cursor::get_cursor_line_slice();
                        let cand = &line_slice[s_off..s_off + word_len];
                        let eq = if icase {
                            mb_strnicmp(
                                crate::memory::bytes_until_nul(cand),
                                crate::memory::bytes_until_nul(word),
                                word_len,
                            ) == 0
                        } else {
                            cand == word
                        };
                        if eq {
                            matched = true;
                        }
                    }
                } else if word_len > 0 {
                    let prev_b = word[word_len - 1];
                    if keytyped == i32::from(prev_b)
                        || (icase
                            && (0..256).contains(&keytyped)
                            && (keytyped as u8).eq_ignore_ascii_case(&prev_b))
                    {
                        let line_slice = crate::cursor::get_cursor_line_slice();
                        let col_usize =
                            (curwin_mut().w_cursor.get().col as usize).min(line_slice.len());
                        if col_usize >= word_len
                            && (col_usize == word_len
                                || !vim_iswordc(i32::from(line_slice[col_usize - word_len - 1])))
                        {
                            let cand = &line_slice[col_usize - word_len..col_usize];
                            let eq = if icase {
                                mb_strnicmp(
                                    crate::memory::bytes_until_nul(cand),
                                    crate::memory::bytes_until_nul(word),
                                    word_len,
                                ) == 0
                            } else {
                                cand == word
                            };
                            if eq {
                                matched = true;
                            }
                        }
                    }
                }
                if matched
                    && try_match_word
                    && !try_match
                    && getwhitecols_curline() as i32
                        != (curwin_mut().w_cursor.get().col - word_len as i32)
                {
                    matched = false;
                }
                if matched {
                    return true;
                }
            }
            rem = &rem[word_len..];
        } else {
            if try_match && i32::from(b0) == keytyped {
                return true;
            }
            if !rem.is_empty() {
                rem = &rem[1..];
            }
        }
        rem = crate::option::skip_to_option_part_slice(rem);
    }
    false
}
/// Do C or expression indenting on the current line.
pub fn do_c_expr_indent() {
    if curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST {
        fixthisline(get_expr_indent);
    } else {
        fixthisline(get_c_indent);
    }
}
/// "cindent(lnum)" function
pub fn f_cindent(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let pos = curwin_mut().w_cursor.get();
    let lnum = tv_get_lnum(&argvars[0]);
    if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count.get() {
        curwin_mut().w_cursor.set_lnum(lnum);
        rettv.vval = crate::eval::typval::TypvalVval::Number(get_c_indent() as i64);
        curwin_mut().w_cursor.set(pos);
    } else {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctrl_chr_and_pos_ordering_work() {
        assert_eq!(ctrl_chr(i32::from(b'?')), 0x7f);
        assert_eq!(ctrl_chr(i32::from(b'a')), 1);
        assert_eq!(ctrl_chr(i32::from(b'A')), 1);
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
