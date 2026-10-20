//! String manipulation functions and Vimscript string builtins (`src/nvim/strings.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[cfg(not(test))]
use crate::charset::{
    STR2NR_BIN, STR2NR_FORCE, STR2NR_HEX, STR2NR_OCT, STR2NR_OOCT, STR2NR_QUOTE, skipwhite,
    transstr, vim_str2nr,
};
use crate::charset::{ascii_isdigit, ascii_iswhite, rem_backslash};
use crate::clipboard::{VarType, typval_T};
use crate::errors::*;
use crate::eval::buffer::varnumber_T;
use crate::eval::deprecated::EvalFuncData;
#[cfg(not(test))]
use crate::eval::encode::encode_tv2echo;
use crate::eval::encode::encode_tv2string;
use crate::eval::typval::float_T;
#[cfg(not(test))]
use crate::eval::typval::{
    tv_check_for_number_arg, tv_check_for_opt_bool_arg, tv_check_for_opt_number_arg,
    tv_check_for_opt_string_arg, tv_check_for_string_arg, tv_get_bool, tv_get_bool_chk,
    tv_get_number, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk,
    tv_list_alloc_ret, tv_list_append_number,
};
#[cfg(not(test))]
use crate::ex_docmd::find_cmdline_var;
#[cfg(not(test))]
use crate::garray::{ga_append, ga_clear, ga_grow, ga_init, garray_T};
use crate::math::{FAIL, OK, xisinf, xisnan};
#[cfg(not(test))]
use crate::mbyte::{
    mb_copy_char, mb_cptr2char_adv, mb_ptr2char_adv, mb_string2cells, utf_head_off, utf_ptr2char,
    utf_ptr2len,
};
use crate::mbyte::{
    mb_tolower, mb_toupper, utf_char2bytes, utf_char2len, utf_ptr2StrCharInfo, utf_ptr2cells,
    utfc_ptr2len,
};
#[cfg(not(test))]
use crate::memory::xmemdupz;
use crate::memory::{xcalloc, xfree, xmalloc, xmallocz, xmemscan, xrealloc, xstrchrnul, xstrlcpy};
#[cfg(not(test))]
use crate::message::emsg;
use crate::message::semsg;
#[cfg(not(test))]
use crate::option::{csh_like_shell, fish_like_shell};
#[cfg(all(not(test), windows))]
use crate::option_vars::*;
#[cfg(not(test))]
use crate::plines::linetabsize_col;
use std::ffi::{
    CStr, c_char, c_double, c_int, c_long, c_longlong, c_uint, c_ulong, c_ulonglong, c_void,
};
use std::mem::size_of;
use std::ptr;

const CTRL_V: c_char = 0x16;

pub const TYPE_UNKNOWN: c_int = -1;
pub const TYPE_INT: c_int = 0;
pub const TYPE_LONGINT: c_int = 1;
pub const TYPE_LONGLONGINT: c_int = 2;
pub const TYPE_SIGNEDSIZET: c_int = 3;
pub const TYPE_UNSIGNEDINT: c_int = 4;
pub const TYPE_UNSIGNEDLONGINT: c_int = 5;
pub const TYPE_UNSIGNEDLONGLONGINT: c_int = 6;
pub const TYPE_SIZET: c_int = 7;
pub const TYPE_POINTER: c_int = 8;
pub const TYPE_PERCENT: c_int = 9;
pub const TYPE_CHAR: c_int = 10;
pub const TYPE_STRING: c_int = 11;
pub const TYPE_FLOAT: c_int = 12;

pub const MAX_ALLOWED_STRING_WIDTH: c_uint = 1_048_576;

#[cfg(not(test))]
const E_PRINTF: &CStr = c"E766: Insufficient arguments for printf()";
const E_CANNOT_MIX_POSITIONAL: &CStr =
    c"E1500: Cannot mix positional and non-positional arguments: %s";
const E_FMT_ARG_NR_UNUSED: &CStr = c"E1501: format argument %d unused in $-style format: %s";
const E_POSITIONAL_FIELD_SPEC_REUSED: &CStr =
    c"E1502: Positional argument %d used as field width reused as different type: %s/%s";
const E_POSITIONAL_OUT_OF_BOUNDS: &CStr = c"E1503: Positional argument %d out of bounds: %s";
const E_POSITIONAL_TYPE_INCONSISTENT: &CStr =
    c"E1504: Positional argument %d type used inconsistently: %s/%s";
const E_INVALID_FORMAT_SPECIFIER: &CStr = c"E1505: Invalid format specifier: %s";

#[repr(C)]
pub struct keyvalue_T {
    pub key: c_int,
    pub value: *mut c_char,
    pub length: usize,
}

#[cfg_attr(all(test, windows), link(name = "legacy_stdio_definitions"))]
unsafe extern "C" {
    pub(crate) fn snprintf(s: *mut c_char, maxlen: usize, format: *const c_char, ...) -> c_int;
    #[link_name = "qsort"]
    fn c_qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
    );
    pub fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    pub fn strcoll(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

#[inline]
pub unsafe fn gettext(msgid: *const c_char) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "gettext"]
            fn c_gettext(msgid: *const c_char) -> *mut c_char;
        }
        c_gettext(msgid)
    }
    #[cfg(test)]
    {
        msgid.cast_mut()
    }
}

pub trait IntoNgettextCount {
    fn into_ngettext_count(self) -> c_ulong;
}

impl IntoNgettextCount for u32 {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

impl IntoNgettextCount for u64 {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

impl IntoNgettextCount for usize {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

impl IntoNgettextCount for i32 {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

impl IntoNgettextCount for i64 {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

impl IntoNgettextCount for isize {
    #[inline]
    fn into_ngettext_count(self) -> c_ulong {
        self as c_ulong
    }
}

#[inline]
pub unsafe fn ngettext<N: IntoNgettextCount>(
    msgid1: *const c_char,
    msgid2: *const c_char,
    n: N,
) -> *mut c_char {
    let count = n.into_ngettext_count();
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "ngettext"]
            fn c_ngettext(msgid1: *const c_char, msgid2: *const c_char, n: c_ulong) -> *mut c_char;
        }
        c_ngettext(msgid1, msgid2, count)
    }
    #[cfg(test)]
    {
        if count == 1 {
            msgid1.cast_mut()
        } else {
            msgid2.cast_mut()
        }
    }
}

#[inline]
pub unsafe fn strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

#[inline]
pub unsafe fn strcmp(mut s1: *const c_char, mut s2: *const c_char) -> c_int {
    unsafe {
        loop {
            let b1 = *s1 as u8;
            let b2 = *s2 as u8;
            if b1 != b2 || b1 == 0 {
                return c_int::from(b1) - c_int::from(b2);
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
        }
    }
}

#[inline]
pub unsafe fn strncmp(mut s1: *const c_char, mut s2: *const c_char, mut n: usize) -> c_int {
    unsafe {
        while n > 0 {
            let b1 = *s1 as u8;
            let b2 = *s2 as u8;
            if b1 != b2 || b1 == 0 {
                return c_int::from(b1) - c_int::from(b2);
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            n -= 1;
        }
        0
    }
}

#[inline]
pub unsafe fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    unsafe {
        let len = CStr::from_ptr(src).to_bytes_with_nul().len();
        ptr::copy_nonoverlapping(src, dst, len);
        dst
    }
}

#[inline]
pub unsafe fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    unsafe {
        let dst_len = CStr::from_ptr(dst).to_bytes().len();
        let src_len = CStr::from_ptr(src).to_bytes_with_nul().len();
        ptr::copy_nonoverlapping(src, dst.add(dst_len), src_len);
        dst
    }
}

#[inline]
pub unsafe fn strchr(mut s: *const c_char, c: c_int) -> *mut c_char {
    unsafe {
        let target = c as c_char;
        loop {
            let cur = *s;
            if cur == target {
                return s.cast_mut();
            }
            if cur == NUL {
                return ptr::null_mut();
            }
            s = s.add(1);
        }
    }
}

#[inline]
pub unsafe fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    unsafe {
        let n_bytes = CStr::from_ptr(needle).to_bytes();
        if n_bytes.is_empty() {
            return haystack.cast_mut();
        }
        let h_bytes = CStr::from_ptr(haystack).to_bytes();
        match h_bytes.windows(n_bytes.len()).position(|w| w == n_bytes) {
            Some(pos) => haystack.add(pos).cast_mut(),
            None => ptr::null_mut(),
        }
    }
}

#[inline]
pub unsafe fn strpbrk(mut s: *const c_char, accept: *const c_char) -> *mut c_char {
    unsafe {
        while *s != NUL {
            let mut a = accept;
            while *a != NUL {
                if *s == *a {
                    return s.cast_mut();
                }
                a = a.add(1);
            }
            s = s.add(1);
        }
        ptr::null_mut()
    }
}

#[inline]
pub unsafe fn atol(mut s: *const c_char) -> c_long {
    unsafe {
        while matches!(*s as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            s = s.add(1);
        }
        let neg = if *s == b'-' as c_char {
            s = s.add(1);
            true
        } else {
            if *s == b'+' as c_char {
                s = s.add(1);
            }
            false
        };
        let mut acc: c_long = 0;
        while (*s as u8).is_ascii_digit() {
            acc = acc
                .wrapping_mul(10)
                .wrapping_add(c_long::from((*s as u8) - b'0'));
            s = s.add(1);
        }
        if neg { acc.wrapping_neg() } else { acc }
    }
}

#[inline]
pub unsafe fn atoi(s: *const c_char) -> c_int {
    unsafe { atol(s) as c_int }
}

#[inline]
pub unsafe fn qsort(
    base: *mut c_void,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
) {
    unsafe {
        c_qsort(base, nmemb, size, Some(compar));
    }
}

#[inline]
pub unsafe fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    unsafe {
        let mut i = 0;
        while i < n && *src.add(i) != NUL {
            *dst.add(i) = *src.add(i);
            i += 1;
        }
        if i < n {
            ptr::write_bytes(dst.add(i), 0, n - i);
        }
        dst
    }
}

#[inline]
pub unsafe fn tolower(c: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "tolower"]
            fn c_tolower(c: c_int) -> c_int;
        }
        c_tolower(c)
    }
    #[cfg(test)]
    {
        if (b'A' as c_int..=b'Z' as c_int).contains(&c) {
            c + 0x20
        } else {
            c
        }
    }
}

#[inline]
pub unsafe fn isalpha(c: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "isalpha"]
            fn c_isalpha(c: c_int) -> c_int;
        }
        c_isalpha(c)
    }
    #[cfg(test)]
    {
        c_int::from((c as u8).is_ascii_alphabetic())
    }
}

#[inline]
pub unsafe fn strcspn(mut s: *const c_char, reject: *const c_char) -> usize {
    unsafe {
        let mut count = 0;
        while *s != NUL {
            let mut r = reject;
            while *r != NUL {
                if *s == *r {
                    return count;
                }
                r = r.add(1);
            }
            s = s.add(1);
            count += 1;
        }
        count
    }
}

#[inline]
pub unsafe fn strtok_r(
    s: *mut c_char,
    delim: *const c_char,
    saveptr: *mut *mut c_char,
) -> *mut c_char {
    unsafe {
        let mut cur = if !s.is_null() { s } else { *saveptr };
        if cur.is_null() {
            return ptr::null_mut();
        }
        let is_delim = |ch: c_char| -> bool {
            let mut d = delim;
            while *d != NUL {
                if *d == ch {
                    return true;
                }
                d = d.add(1);
            }
            false
        };
        while *cur != NUL && is_delim(*cur) {
            cur = cur.add(1);
        }
        if *cur == NUL {
            *saveptr = cur;
            return ptr::null_mut();
        }
        let token = cur;
        while *cur != NUL && !is_delim(*cur) {
            cur = cur.add(1);
        }
        if *cur == NUL {
            *saveptr = cur;
        } else {
            *cur = NUL;
            *saveptr = cur.add(1);
        }
        token
    }
}

#[inline]
pub unsafe fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, mut base: c_int) -> c_ulong {
    unsafe {
        let mut s = nptr;
        while matches!(*s as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            s = s.add(1);
        }
        let neg = if *s == b'-' as c_char {
            s = s.add(1);
            true
        } else {
            if *s == b'+' as c_char {
                s = s.add(1);
            }
            false
        };
        if (base == 0 || base == 16)
            && *s == b'0' as c_char
            && matches!(*s.add(1) as u8, b'x' | b'X')
        {
            s = s.add(2);
            base = 16;
        } else if base == 0 {
            base = if *s == b'0' as c_char { 8 } else { 10 };
        }
        let radix = base as u32;
        let mut acc: c_ulong = 0;
        let mut any = false;
        loop {
            let ch = *s as u8 as char;
            let Some(digit) = ch.to_digit(radix) else {
                break;
            };
            any = true;
            acc = acc
                .wrapping_mul(radix as c_ulong)
                .wrapping_add(digit as c_ulong);
            s = s.add(1);
        }
        if !endptr.is_null() {
            *endptr = if any { s.cast_mut() } else { nptr.cast_mut() };
        }
        if neg { acc.wrapping_neg() } else { acc }
    }
}

#[inline]
pub unsafe fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long {
    unsafe { strtoul(nptr, endptr, base) as c_long }
}

#[inline]
pub unsafe fn toupper(c: c_int) -> c_int {
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "toupper"]
            fn c_toupper(c: c_int) -> c_int;
        }
        c_toupper(c)
    }
    #[cfg(test)]
    {
        if (b'a' as c_int..=b'z' as c_int).contains(&c) {
            c - 0x20
        } else {
            c
        }
    }
}

#[inline]
pub unsafe fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if n > 0 {
        unsafe { ptr::copy_nonoverlapping(src.cast::<u8>(), dst.cast::<u8>(), n) };
    }
    dst
}

#[inline]
pub unsafe fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    if n > 0 {
        unsafe { ptr::copy(src.cast::<u8>(), dst.cast::<u8>(), n) };
    }
    dst
}

#[inline]
pub unsafe fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    if n > 0 {
        unsafe { ptr::write_bytes(s.cast::<u8>(), c as u8, n) };
    }
    s
}

#[inline]
pub unsafe fn strrchr(mut s: *const c_char, c: c_int) -> *mut c_char {
    unsafe {
        let target = c as c_char;
        let mut last: *mut c_char = ptr::null_mut();
        loop {
            let cur = *s;
            if cur == target {
                last = s.cast_mut();
            }
            if cur == NUL {
                return last;
            }
            s = s.add(1);
        }
    }
}

#[inline]
pub unsafe fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    if n == 0 {
        return 0;
    }
    unsafe {
        let b1 = std::slice::from_raw_parts(s1.cast::<u8>(), n);
        let b2 = std::slice::from_raw_parts(s2.cast::<u8>(), n);
        for (&x, &y) in b1.iter().zip(b2.iter()) {
            if x != y {
                return c_int::from(x) - c_int::from(y);
            }
        }
        0
    }
}

#[inline]
pub unsafe fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    if n == 0 {
        return ptr::null_mut();
    }
    unsafe {
        let target = c as u8;
        let bytes = std::slice::from_raw_parts(s.cast::<u8>(), n);
        match bytes.iter().position(|&b| b == target) {
            Some(pos) => s.cast::<u8>().add(pos).cast_mut().cast(),
            None => ptr::null_mut(),
        }
    }
}

#[inline]
pub unsafe fn wcslen(mut s: *const u16) -> usize {
    unsafe {
        let mut len = 0usize;
        while *s != 0 {
            len += 1;
            s = s.add(1);
        }
        len
    }
}

#[inline]
pub unsafe fn strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int {
    unsafe { vim_stricmp(s1, s2) }
}

#[inline]
pub unsafe fn strncasecmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    unsafe { vim_strnicmp(s1, s2, n) }
}

#[inline]
pub unsafe fn strerror(errnum: c_int) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        unsafe extern "C" {
            #[link_name = "strerror"]
            fn c_strerror(errnum: c_int) -> *mut c_char;
        }
        c_strerror(errnum)
    }
    #[cfg(test)]
    {
        let _ = errnum;
        c"error".as_ptr().cast_mut()
    }
}

#[inline]
const fn tolower_asc(c: c_char) -> c_int {
    let b = c as u8;
    if b.is_ascii_uppercase() {
        (b + 0x20) as c_int
    } else {
        b as c_int
    }
}

/// Copy up to `len` bytes of `string` into newly allocated memory and terminate with a NUL.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xstrnsave(string: *const c_char, len: usize) -> *mut c_char {
    unsafe {
        let dst = xmallocz(len).cast::<c_char>();
        let mut i = 0;
        while i < len {
            let b = *string.add(i);
            if b == NUL {
                break;
            }
            *dst.add(i) = b;
            i += 1;
        }
        while i < len {
            *dst.add(i) = NUL;
            i += 1;
        }
        dst
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strsave_escaped(
    string: *const c_char,
    esc_chars: *const c_char,
) -> *mut c_char {
    unsafe { vim_strsave_escaped_ext(string, esc_chars, b'\\' as c_char, false) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strsave_escaped_ext(
    string: *const c_char,
    esc_chars: *const c_char,
    cc: c_char,
    bsl: bool,
) -> *mut c_char {
    unsafe {
        let mut length: usize = 1;
        let mut p = string;
        while *p != NUL {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                length += l;
                p = p.add(l);
                continue;
            }
            if !vim_strchr(esc_chars, (*p as u8) as c_int).is_null() || (bsl && rem_backslash(p)) {
                length += 1;
            }
            length += 1;
            p = p.add(1);
        }

        let escaped_string = xmalloc(length).cast::<c_char>();
        let mut p2 = escaped_string;
        p = string;
        while *p != NUL {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                ptr::copy_nonoverlapping(p, p2, l);
                p2 = p2.add(l);
                p = p.add(l);
                continue;
            }
            if !vim_strchr(esc_chars, (*p as u8) as c_int).is_null() || (bsl && rem_backslash(p)) {
                *p2 = cc;
                p2 = p2.add(1);
            }
            *p2 = *p;
            p2 = p2.add(1);
            p = p.add(1);
        }
        *p2 = NUL;

        escaped_string
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strnsave_unquoted(
    string: *const c_char,
    length: usize,
) -> *mut c_char {
    unsafe {
        let mut ret_length: usize = 0;
        let mut inquote = false;
        let string_end = string.add(length);
        let mut p = string;
        while p < string_end {
            if *p == b'"' as c_char {
                inquote = !inquote;
            } else if *p == b'\\' as c_char
                && inquote
                && p.add(1) < string_end
                && (*p.add(1) == b'\\' as c_char || *p.add(1) == b'"' as c_char)
            {
                ret_length += 1;
                p = p.add(1);
            } else {
                ret_length += 1;
            }
            p = p.add(1);
        }

        let ret = xmallocz(ret_length).cast::<c_char>();
        let mut rp = ret;
        inquote = false;
        p = string;
        while p < string_end {
            if *p == b'"' as c_char {
                inquote = !inquote;
            } else if *p == b'\\' as c_char
                && inquote
                && p.add(1) < string_end
                && (*p.add(1) == b'\\' as c_char || *p.add(1) == b'"' as c_char)
            {
                p = p.add(1);
                *rp = *p;
                rp = rp.add(1);
            } else {
                *rp = *p;
                rp = rp.add(1);
            }
            p = p.add(1);
        }

        ret
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strsave_shellescape(
    string: *const c_char,
    do_special: bool,
    do_newline: bool,
) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        let csh_like = csh_like_shell() != 0;
        let fish_like = fish_like_shell();
        #[cfg(windows)]
        let use_double_quote = p_ssl == 0;
        #[cfg(not(windows))]
        let use_double_quote = false;

        let mut length = CStr::from_ptr(string).to_bytes().len() + 3;
        let mut p = string;
        while *p != NUL {
            if use_double_quote {
                if *p == b'"' as c_char {
                    length += 1;
                }
            } else if *p == b'\'' as c_char {
                length += 3;
            }
            if (*p == b'\n' as c_char && (csh_like || do_newline))
                || (*p == b'!' as c_char && (csh_like || do_special))
            {
                length += 1;
                if csh_like && do_special {
                    length += 1;
                }
            }
            let mut l: usize = 0;
            if do_special && find_cmdline_var(p, &raw mut l) >= 0 {
                length += 1;
                p = p.add(l - 1);
            }
            if *p == b'\\' as c_char && fish_like {
                length += 1;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        let escaped_string = xmalloc(length).cast::<c_char>();
        let mut d = escaped_string;

        *d = if use_double_quote {
            b'"' as c_char
        } else {
            b'\'' as c_char
        };
        d = d.add(1);

        p = string;
        while *p != NUL {
            if use_double_quote {
                if *p == b'"' as c_char {
                    *d = b'"' as c_char;
                    *d.add(1) = b'"' as c_char;
                    d = d.add(2);
                    p = p.add(1);
                    continue;
                }
            } else if *p == b'\'' as c_char {
                *d = b'\'' as c_char;
                *d.add(1) = b'\\' as c_char;
                *d.add(2) = b'\'' as c_char;
                *d.add(3) = b'\'' as c_char;
                d = d.add(4);
                p = p.add(1);
                continue;
            }
            if (*p == b'\n' as c_char && (csh_like || do_newline))
                || (*p == b'!' as c_char && (csh_like || do_special))
            {
                *d = b'\\' as c_char;
                d = d.add(1);
                if csh_like && do_special {
                    *d = b'\\' as c_char;
                    d = d.add(1);
                }
                *d = *p;
                d = d.add(1);
                p = p.add(1);
                continue;
            }
            let mut l: usize = 0;
            if do_special && find_cmdline_var(p, &raw mut l) >= 0 {
                *d = b'\\' as c_char;
                d = d.add(1);
                ptr::copy_nonoverlapping(p, d, l);
                d = d.add(l);
                p = p.add(l);
                continue;
            }
            if *p == b'\\' as c_char && fish_like {
                *d = b'\\' as c_char;
                *d.add(1) = *p;
                d = d.add(2);
                p = p.add(1);
                continue;
            }

            mb_copy_char(&raw mut p, &raw mut d);
        }

        *d = if use_double_quote {
            b'"' as c_char
        } else {
            b'\'' as c_char
        };
        *d.add(1) = NUL;

        escaped_string
    }
    #[cfg(test)]
    {
        let _ = (string, do_special, do_newline);
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strsave_up(string: *const c_char) -> *mut c_char {
    unsafe {
        let len = CStr::from_ptr(string).to_bytes().len();
        let p1 = xmalloc(len + 1).cast::<c_char>();
        vim_strcpy_up(p1, string);
        p1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strnsave_up(string: *const c_char, len: usize) -> *mut c_char {
    unsafe {
        let p1 = xmalloc(len + 1).cast::<c_char>();
        vim_strncpy_up(p1, string, len);
        p1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strup(mut p: *mut c_char) {
    unsafe {
        loop {
            let c = *p as u8;
            if c == 0 {
                break;
            }
            *p = c.to_ascii_uppercase() as c_char;
            p = p.add(1);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strcpy_up(mut dst: *mut c_char, mut src: *const c_char) {
    unsafe {
        loop {
            let c = *src as u8;
            src = src.add(1);
            if c == 0 {
                break;
            }
            *dst = c.to_ascii_uppercase() as c_char;
            dst = dst.add(1);
        }
        *dst = NUL;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strncpy_up(
    mut dst: *mut c_char,
    mut src: *const c_char,
    mut n: usize,
) {
    unsafe {
        while n > 0 {
            n -= 1;
            let c = *src as u8;
            src = src.add(1);
            if c == 0 {
                break;
            }
            *dst = c.to_ascii_uppercase() as c_char;
            dst = dst.add(1);
        }
        *dst = NUL;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_memcpy_up(mut dst: *mut c_char, mut src: *const c_char, mut n: usize) {
    unsafe {
        while n > 0 {
            n -= 1;
            let c = *src as u8;
            src = src.add(1);
            *dst = c.to_ascii_uppercase() as c_char;
            dst = dst.add(1);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mb_strup_buf(src: *const c_char, dst: *mut c_char) -> usize {
    unsafe {
        let mut i: usize = 0;
        let mut p = src;
        while *p != NUL {
            let ci = utf_ptr2StrCharInfo(p).chr;
            let c = if ci.value < 0 {
                (*p as u8) as c_int
            } else {
                ci.value
            };
            i += utf_char2bytes(mb_toupper(c), dst.add(i)) as usize;
            p = p.add(ci.len as usize);
        }
        *dst.add(i) = NUL;
        i
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strcase_save(orig: *const c_char, upper: bool) -> *mut c_char {
    unsafe {
        let mut orig_len = CStr::from_ptr(orig).to_bytes().len();
        let mut res = xmalloc(orig_len + 1).cast::<c_char>();
        let mut res_index: usize = 0;
        let mut p = orig;

        while *p != NUL {
            let char_info = utf_ptr2StrCharInfo(p).chr;
            let c = if char_info.value < 0 {
                (*p as u8) as c_int
            } else {
                char_info.value
            };
            let newc = if upper { mb_toupper(c) } else { mb_tolower(c) };
            let newl = utf_char2len(newc) as usize;

            if res_index + newl > orig_len {
                let new_size = res_index + newl + 1;
                res = xrealloc(res.cast(), new_size).cast::<c_char>();
                orig_len = new_size - 1;
            }

            utf_char2bytes(newc, res.add(res_index));
            res_index += newl;
            p = p.add(char_info.len as usize);
        }

        *res.add(res_index) = NUL;
        res
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn del_trailing_spaces(ptr: *mut c_char) {
    unsafe {
        let len = CStr::from_ptr(ptr).to_bytes().len();
        let mut q = ptr.add(len);
        while q > ptr {
            q = q.sub(1);
            if q <= ptr {
                break;
            }
            if ascii_iswhite(*q) && *q.sub(1) != b'\\' as c_char && *q.sub(1) != CTRL_V {
                *q = NUL;
            } else {
                break;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_stricmp(mut s1: *const c_char, mut s2: *const c_char) -> c_int {
    unsafe {
        loop {
            #[cfg(not(test))]
            let i = tolower((*s1 as u8) as c_int) - tolower((*s2 as u8) as c_int);
            #[cfg(test)]
            let i = tolower_asc(*s1) - tolower_asc(*s2);
            if i != 0 {
                return i;
            }
            if *s1 == NUL {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
        }
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strnicmp(
    mut s1: *const c_char,
    mut s2: *const c_char,
    mut len: usize,
) -> c_int {
    unsafe {
        while len > 0 {
            #[cfg(not(test))]
            let i = tolower((*s1 as u8) as c_int) - tolower((*s2 as u8) as c_int);
            #[cfg(test)]
            let i = tolower_asc(*s1) - tolower_asc(*s2);
            if i != 0 {
                return i;
            }
            if *s1 == NUL {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            len -= 1;
        }
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn striequal(a: *const c_char, b: *const c_char) -> bool {
    unsafe {
        (a.is_null() && b.is_null()) || (!a.is_null() && !b.is_null() && vim_stricmp(a, b) == 0)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strnicmp_asc(
    mut s1: *const c_char,
    mut s2: *const c_char,
    mut len: usize,
) -> c_int {
    unsafe {
        let mut i: c_int = 0;
        while len > 0 {
            i = tolower_asc(*s1) - tolower_asc(*s2);
            if i != 0 || *s1 == NUL {
                break;
            }
            s1 = s1.add(1);
            s2 = s2.add(1);
            len -= 1;
        }
        i
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char {
    unsafe {
        if c <= 0 {
            ptr::null_mut()
        } else if c < 0x80 {
            let mut p = string;
            while *p != NUL {
                if (*p as u8) as c_int == c {
                    return p.cast_mut();
                }
                p = p.add(1);
            }
            ptr::null_mut()
        } else {
            let mut u8char = [0 as c_char; MB_MAXBYTES + 1];
            let len = utf_char2bytes(c, u8char.as_mut_ptr()) as usize;
            let needle = std::slice::from_raw_parts(u8char.as_ptr().cast::<u8>(), len);
            let hay = CStr::from_ptr(string).to_bytes();
            if let Some(pos) = hay.windows(len).position(|w| w == needle) {
                string.add(pos).cast_mut()
            } else {
                ptr::null_mut()
            }
        }
    }
}

#[cfg(not(test))]
unsafe extern "C" fn sort_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let a = *s1.cast::<*const c_char>();
        let b = *s2.cast::<*const c_char>();
        match CStr::from_ptr(a)
            .to_bytes()
            .cmp(CStr::from_ptr(b).to_bytes())
        {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn sort_strings(files: *mut *mut c_char, count: c_int) {
    #[cfg(not(test))]
    unsafe {
        if count > 1 && !files.is_null() {
            qsort(
                files.cast(),
                count as usize,
                size_of::<*mut c_char>(),
                sort_compare,
            );
        }
    }
    #[cfg(test)]
    unsafe {
        if count > 1 && !files.is_null() {
            let slice = std::slice::from_raw_parts_mut(files, count as usize);
            slice.sort_by(|&a, &b| {
                CStr::from_ptr(a)
                    .to_bytes()
                    .cmp(CStr::from_ptr(b).to_bytes())
            });
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn has_non_ascii(s: *const c_char) -> bool {
    unsafe {
        if !s.is_null() {
            let mut p = s;
            while *p != NUL {
                if (*p as u8) >= 128 {
                    return true;
                }
                p = p.add(1);
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn has_non_ascii_len(s: *const c_char, len: usize) -> bool {
    unsafe {
        if !s.is_null() {
            for i in 0..len {
                if (*s.add(i) as u8) >= 128 {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn concat_str(str1: *const c_char, str2: *const c_char) -> *mut c_char {
    unsafe {
        let b1 = CStr::from_ptr(str1).to_bytes();
        let b2 = CStr::from_ptr(str2).to_bytes();
        let dest = xmalloc(b1.len() + b2.len() + 1).cast::<u8>();
        ptr::copy_nonoverlapping(b1.as_ptr(), dest, b1.len());
        ptr::copy_nonoverlapping(b2.as_ptr(), dest.add(b1.len()), b2.len());
        *dest.add(b1.len() + b2.len()) = 0;
        dest.cast()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_nr(tvs: *mut typval_T, idxp: *mut c_int) -> varnumber_T {
    #[cfg(not(test))]
    unsafe {
        let idx = (*idxp - 1) as usize;
        let mut n: varnumber_T = 0;
        if (*tvs.add(idx)).v_type == VarType::VAR_UNKNOWN {
            emsg(gettext(E_PRINTF.as_ptr()));
        } else {
            *idxp += 1;
            let mut err = false;
            n = tv_get_number_chk(tvs.add(idx), &raw mut err);
            if err {
                n = 0;
            }
        }
        n
    }
    #[cfg(test)]
    {
        let _ = (tvs, idxp);
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_str(
    tvs: *mut typval_T,
    idxp: *mut c_int,
    tofree: *mut *mut c_char,
) -> *const c_char {
    #[cfg(not(test))]
    unsafe {
        let idx = (*idxp - 1) as usize;
        let mut s: *const c_char = ptr::null();
        if (*tvs.add(idx)).v_type == VarType::VAR_UNKNOWN {
            emsg(gettext(E_PRINTF.as_ptr()));
        } else {
            *idxp += 1;
            let tv = tvs.add(idx);
            if (*tv).v_type == VarType::VAR_STRING || (*tv).v_type == VarType::VAR_NUMBER {
                s = tv_get_string_chk(tv);
                *tofree = ptr::null_mut();
            } else {
                let encoded = encode_tv2echo(tv, ptr::null_mut());
                *tofree = encoded;
                s = encoded;
            }
        }
        s
    }
    #[cfg(test)]
    {
        let _ = (tvs, idxp, tofree);
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_ptr(tvs: *const typval_T, idxp: *mut c_int) -> *const c_void {
    #[cfg(not(test))]
    unsafe {
        let idx = (*idxp - 1) as usize;
        if (*tvs.add(idx)).v_type == VarType::VAR_UNKNOWN {
            emsg(gettext(E_PRINTF.as_ptr()));
            return ptr::null();
        }
        *idxp += 1;
        (*tvs.add(idx)).vval.v_string.cast()
    }
    #[cfg(test)]
    {
        let _ = (tvs, idxp);
        ptr::null()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_float(tvs: *mut typval_T, idxp: *mut c_int) -> float_T {
    #[cfg(not(test))]
    unsafe {
        let idx = (*idxp - 1) as usize;
        let mut f: float_T = 0.0;
        if (*tvs.add(idx)).v_type == VarType::VAR_UNKNOWN {
            emsg(gettext(E_PRINTF.as_ptr()));
        } else {
            *idxp += 1;
            let tv = tvs.add(idx);
            if (*tv).v_type == VarType::VAR_FLOAT {
                f = (*tv).vval.v_float;
            } else if (*tv).v_type == VarType::VAR_NUMBER {
                f = (*tv).vval.v_number as float_T;
            } else {
                emsg(gettext(
                    c"E807: Expected Float argument for printf()".as_ptr(),
                ));
            }
        }
        f
    }
    #[cfg(test)]
    {
        let _ = (tvs, idxp);
        0.0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn infinity_str(
    positive: bool,
    fmt_spec: c_char,
    force_sign: c_int,
    space_for_positive: c_int,
) -> *const c_char {
    const TABLE: [&CStr; 8] = [
        c"-inf", c"inf", c"+inf", c" inf", c"-INF", c"INF", c"+INF", c" INF",
    ];
    let mut idx = if positive {
        (1 + force_sign + force_sign * space_for_positive) as usize
    } else {
        0
    };
    if (fmt_spec as u8).is_ascii_uppercase() {
        idx += 4;
    }
    TABLE[idx].as_ptr()
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn format_typeof(mut type_: *const c_char) -> c_int {
    unsafe {
        let mut length_modifier: u8 = 0;
        let first = *type_ as u8;
        if matches!(first, b'h' | b'l' | b'z') {
            length_modifier = first;
            type_ = type_.add(1);
            if length_modifier == b'l' && (*type_ as u8) == b'l' {
                length_modifier = b'L';
                type_ = type_.add(1);
            }
        }
        let mut fmt_spec = *type_ as u8;
        match fmt_spec {
            b'i' => fmt_spec = b'd',
            b'*' => {
                fmt_spec = b'd';
                length_modifier = b'h';
            }
            b'D' => {
                fmt_spec = b'd';
                length_modifier = b'l';
            }
            b'U' => {
                fmt_spec = b'u';
                length_modifier = b'l';
            }
            b'O' => {
                fmt_spec = b'o';
                length_modifier = b'l';
            }
            _ => {}
        }

        match fmt_spec {
            b'%' => TYPE_PERCENT,
            b'c' => TYPE_CHAR,
            b's' | b'S' => TYPE_STRING,
            b'p' => TYPE_POINTER,
            b'b' | b'B' => TYPE_UNSIGNEDLONGLONGINT,
            b'd' => match length_modifier {
                0 | b'h' => TYPE_INT,
                b'l' => TYPE_LONGINT,
                b'L' => TYPE_LONGLONGINT,
                b'z' => TYPE_SIGNEDSIZET,
                _ => TYPE_UNKNOWN,
            },
            b'u' | b'o' | b'x' | b'X' => match length_modifier {
                0 | b'h' => TYPE_UNSIGNEDINT,
                b'l' => TYPE_UNSIGNEDLONGINT,
                b'L' => TYPE_UNSIGNEDLONGLONGINT,
                b'z' => TYPE_SIZET,
                _ => TYPE_UNKNOWN,
            },
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => TYPE_FLOAT,
            _ => TYPE_UNKNOWN,
        }
    }
}

unsafe fn format_typename(type_: *const c_char) -> *const c_char {
    unsafe {
        match format_typeof(type_) {
            TYPE_INT => c"int".as_ptr(),
            TYPE_LONGINT => c"long int".as_ptr(),
            TYPE_LONGLONGINT => c"long long int".as_ptr(),
            TYPE_UNSIGNEDINT => c"unsigned int".as_ptr(),
            TYPE_SIGNEDSIZET => c"signed size_t".as_ptr(),
            TYPE_UNSIGNEDLONGINT => c"unsigned long int".as_ptr(),
            TYPE_UNSIGNEDLONGLONGINT => c"unsigned long long int".as_ptr(),
            TYPE_SIZET => c"size_t".as_ptr(),
            TYPE_POINTER => gettext(c"pointer".as_ptr()),
            TYPE_PERCENT => gettext(c"percent".as_ptr()),
            TYPE_CHAR => c"char".as_ptr(),
            TYPE_STRING => gettext(c"string".as_ptr()),
            TYPE_FLOAT => c"float".as_ptr(),
            _ => gettext(c"unknown".as_ptr()),
        }
    }
}

unsafe fn adjust_types(
    ap_types: *mut *mut *const c_char,
    arg: c_int,
    num_posarg: *mut c_int,
    type_: *const c_char,
) -> c_int {
    unsafe {
        if arg <= 0 {
            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), type_);
            return FAIL;
        }

        if (*ap_types).is_null() || *num_posarg < arg {
            let new_types = if (*ap_types).is_null() {
                xcalloc(arg as usize, size_of::<*const c_char>()).cast::<*const c_char>()
            } else {
                xrealloc(
                    (*ap_types).cast(),
                    (arg as usize) * size_of::<*const c_char>(),
                )
                .cast::<*const c_char>()
            };
            for idx in (*num_posarg as usize)..(arg as usize) {
                *new_types.add(idx) = ptr::null();
            }
            *ap_types = new_types;
            *num_posarg = arg;
        }

        let prev = *(*ap_types).add((arg - 1) as usize);
        if !prev.is_null() {
            if *prev == b'*' as c_char || *type_ == b'*' as c_char {
                let mut pt = type_;
                if *pt == b'*' as c_char {
                    pt = prev;
                }
                if *pt != b'*' as c_char && !matches!(*pt as u8, b'd' | b'i') {
                    semsg!(
                        gettext(E_POSITIONAL_FIELD_SPEC_REUSED.as_ptr()),
                        arg,
                        format_typename(prev),
                        format_typename(type_),
                    );
                    return FAIL;
                }
            } else if format_typeof(type_) != format_typeof(prev) {
                semsg!(
                    gettext(E_POSITIONAL_TYPE_INCONSISTENT.as_ptr()),
                    arg,
                    format_typename(type_),
                    format_typename(prev),
                );
                return FAIL;
            }
        }

        *(*ap_types).add((arg - 1) as usize) = type_;
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn format_overflow_error(pstart: *const c_char) {
    unsafe {
        let mut p = pstart;
        while ascii_isdigit(*p) {
            p = p.add(1);
        }
        semsg!(
            gettext((&raw const e_val_too_large_len).cast()),
            p.offset_from(pstart) as c_int,
            pstart,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_unsigned_int(
    pstart: *const c_char,
    p: *mut *const c_char,
    uj: *mut c_uint,
    overflow_err: bool,
) -> c_int {
    unsafe {
        *uj = ((**p as u8) - b'0') as c_uint;
        *p = (*p).add(1);

        while ascii_isdigit(**p) && *uj < MAX_ALLOWED_STRING_WIDTH {
            *uj = 10 * (*uj) + (((**p as u8) - b'0') as c_uint);
            *p = (*p).add(1);
        }

        if *uj > MAX_ALLOWED_STRING_WIDTH {
            if overflow_err {
                format_overflow_error(pstart);
                return FAIL;
            } else {
                *uj = MAX_ALLOWED_STRING_WIDTH;
            }
        }
        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_fmt_types(
    ap_types: *mut *mut *const c_char,
    num_posarg: *mut c_int,
    fmt: *const c_char,
    tvs: *mut typval_T,
) -> c_int {
    unsafe {
        if fmt.is_null() {
            return OK;
        }
        let mut p = fmt;
        let mut any_pos = false;
        let mut any_arg = false;

        macro_rules! check_pos_arg {
            () => {
                if any_pos && any_arg {
                    semsg!(gettext(E_CANNOT_MIX_POSITIONAL.as_ptr()), fmt);
                    xfree((*ap_types).cast());
                    *ap_types = ptr::null_mut();
                    *num_posarg = 0;
                    return FAIL;
                }
            };
        }

        macro_rules! fail {
            () => {
                xfree((*ap_types).cast());
                *ap_types = ptr::null_mut();
                *num_posarg = 0;
                return FAIL;
            };
        }

        while *p != NUL {
            if *p != b'%' as c_char {
                p = p.add(1);
                while *p != NUL && *p != b'%' as c_char {
                    p = p.add(1);
                }
            } else {
                let mut pos_arg: c_int = -1;
                let pstart = p.add(1);
                p = p.add(1);

                let mut ptype = p;
                while ascii_isdigit(*ptype) {
                    ptype = ptype.add(1);
                }

                if *ptype == b'$' as c_char {
                    if *p == b'0' as c_char {
                        semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), fmt);
                        fail!();
                    }
                    let mut uj: c_uint = 0;
                    if get_unsigned_int(pstart, &raw mut p, &raw mut uj, !tvs.is_null()) == FAIL {
                        fail!();
                    }
                    pos_arg = uj as c_int;
                    any_pos = true;
                    check_pos_arg!();
                    p = p.add(1);
                }

                while matches!(*p as u8, b'0' | b'-' | b'+' | b' ' | b'#' | b'\'') {
                    p = p.add(1);
                }

                let arg = p;
                if *arg == b'*' as c_char {
                    p = p.add(1);
                    if ascii_isdigit(*p) {
                        let mut uj: c_uint = 0;
                        if get_unsigned_int(arg.add(1), &raw mut p, &raw mut uj, !tvs.is_null())
                            == FAIL
                        {
                            fail!();
                        }
                        if *p != b'$' as c_char {
                            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), fmt);
                            fail!();
                        }
                        p = p.add(1);
                        any_pos = true;
                        check_pos_arg!();
                        if adjust_types(ap_types, uj as c_int, num_posarg, arg) == FAIL {
                            fail!();
                        }
                    } else {
                        any_arg = true;
                        check_pos_arg!();
                    }
                } else if ascii_isdigit(*p) {
                    let digstart = p;
                    let mut uj: c_uint = 0;
                    if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null()) == FAIL {
                        fail!();
                    }
                    if *p == b'$' as c_char {
                        semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), fmt);
                        fail!();
                    }
                }

                if *p == b'.' as c_char {
                    p = p.add(1);
                    let arg = p;
                    if *arg == b'*' as c_char {
                        p = p.add(1);
                        if ascii_isdigit(*p) {
                            let mut uj: c_uint = 0;
                            if get_unsigned_int(arg.add(1), &raw mut p, &raw mut uj, !tvs.is_null())
                                == FAIL
                            {
                                fail!();
                            }
                            if *p == b'$' as c_char {
                                any_pos = true;
                                check_pos_arg!();
                                p = p.add(1);
                                if adjust_types(ap_types, uj as c_int, num_posarg, arg) == FAIL {
                                    fail!();
                                }
                            } else {
                                semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), fmt);
                                fail!();
                            }
                        } else {
                            any_arg = true;
                            check_pos_arg!();
                        }
                    } else if ascii_isdigit(*p) {
                        let digstart = p;
                        let mut uj: c_uint = 0;
                        if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null())
                            == FAIL
                        {
                            fail!();
                        }
                        if *p == b'$' as c_char {
                            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER.as_ptr()), fmt);
                            fail!();
                        }
                    }
                }

                if pos_arg != -1 {
                    any_pos = true;
                    check_pos_arg!();
                    ptype = p;
                }

                if matches!(*p as u8, b'h' | b'l' | b'z') {
                    let lm = *p as u8;
                    p = p.add(1);
                    if lm == b'l' && (*p as u8) == b'l' {
                        p = p.add(1);
                    }
                }

                match *p as u8 {
                    b'i' | b'*' | b'd' | b'u' | b'o' | b'D' | b'U' | b'O' | b'x' | b'X' | b'b'
                    | b'B' | b'c' | b's' | b'S' | b'p' | b'f' | b'F' | b'e' | b'E' | b'g'
                    | b'G' => {
                        if pos_arg != -1 {
                            if adjust_types(ap_types, pos_arg, num_posarg, ptype) == FAIL {
                                fail!();
                            }
                        } else {
                            any_arg = true;
                            check_pos_arg!();
                        }
                    }
                    _ => {
                        if pos_arg != -1 {
                            semsg!(gettext(E_CANNOT_MIX_POSITIONAL.as_ptr()), fmt);
                            fail!();
                        }
                    }
                }

                if *p != NUL {
                    p = p.add(1);
                }
            }
        }

        for arg_idx in 0..(*num_posarg as usize) {
            if (*(*ap_types).add(arg_idx)).is_null() {
                semsg!(
                    gettext(E_FMT_ARG_NR_UNUSED.as_ptr()),
                    (arg_idx + 1) as c_int,
                    fmt,
                );
                fail!();
            }
            if !tvs.is_null() && (*tvs.add(arg_idx)).v_type == VarType::VAR_UNKNOWN {
                semsg!(
                    gettext(E_POSITIONAL_OUT_OF_BOUNDS.as_ptr()),
                    (arg_idx + 1) as c_int,
                    fmt,
                );
                fail!();
            }
        }

        OK
    }
}

#[cfg(not(test))]
const E_TOO_MANY_ARGS: &CStr = c"E767: Too many arguments to printf()";
const TMP_LEN: usize = 350;

#[derive(Clone, Copy)]
pub enum FmtVal {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Ptr(*const c_void),
}

impl FmtVal {
    #[inline]
    fn as_i64(self) -> i64 {
        match self {
            Self::Signed(v) => v,
            Self::Unsigned(v) => v as i64,
            Self::Float(v) => v as i64,
            Self::Ptr(p) => p as usize as i64,
        }
    }

    #[inline]
    fn as_u64(self) -> u64 {
        match self {
            Self::Signed(v) => v as u64,
            Self::Unsigned(v) => v,
            Self::Float(v) => v as u64,
            Self::Ptr(p) => p as usize as u64,
        }
    }

    #[inline]
    fn as_f64(self) -> f64 {
        match self {
            Self::Float(v) => v,
            Self::Signed(v) => v as f64,
            Self::Unsigned(v) => v as f64,
            Self::Ptr(_) => 0.0,
        }
    }

    #[inline]
    fn as_ptr(self) -> *const c_void {
        match self {
            Self::Ptr(p) => p,
            Self::Signed(v) => v as usize as *const c_void,
            Self::Unsigned(v) => v as usize as *const c_void,
            Self::Float(_) => ptr::null(),
        }
    }
}

pub trait IntoFmtVal {
    fn into_fmt_val(self) -> FmtVal;
}

impl IntoFmtVal for FmtVal {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        self
    }
}

impl IntoFmtVal for i8 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Signed(i64::from(self))
    }
}

impl IntoFmtVal for i16 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Signed(i64::from(self))
    }
}

impl IntoFmtVal for i32 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Signed(i64::from(self))
    }
}

impl IntoFmtVal for i64 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Signed(self)
    }
}

impl IntoFmtVal for isize {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Signed(self as i64)
    }
}

impl IntoFmtVal for u8 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(u64::from(self))
    }
}

impl IntoFmtVal for u16 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(u64::from(self))
    }
}

impl IntoFmtVal for u32 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(u64::from(self))
    }
}

impl IntoFmtVal for u64 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(self)
    }
}

impl IntoFmtVal for usize {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(self as u64)
    }
}

impl IntoFmtVal for bool {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Unsigned(u64::from(self))
    }
}

impl IntoFmtVal for f32 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Float(f64::from(self))
    }
}

impl IntoFmtVal for f64 {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Float(self)
    }
}

impl<T> IntoFmtVal for *const T {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Ptr(self.cast())
    }
}

impl<T> IntoFmtVal for *mut T {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Ptr(self.cast())
    }
}

impl IntoFmtVal for &CStr {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Ptr(self.as_ptr().cast())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VimFmtArg {
    pub tag: c_int,
    pub i: i64,
    pub u: u64,
    pub f: f64,
    pub p: *const c_void,
}

#[inline]
fn fmt_arg_val(args: &[FmtVal], arg_idx: &mut c_int) -> FmtVal {
    let idx = (*arg_idx - 1) as usize;
    *arg_idx += 1;
    args.get(idx).copied().unwrap_or(FmtVal::Signed(0))
}

unsafe fn vim_vsnprintf_typval_inner(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    tvs: *mut typval_T,
    fmt_args: &[FmtVal],
) -> c_int {
    unsafe {
        let mut str_l: usize = 0;
        let mut str_avail = str_l < str_m;
        let mut p = fmt;
        let mut num_posarg: c_int = 0;
        let mut arg_idx: c_int = 1;
        let mut ap_types: *mut *const c_char = ptr::null_mut();

        if parse_fmt_types(&raw mut ap_types, &raw mut num_posarg, fmt, tvs) == FAIL {
            return 0;
        }

        if p.is_null() {
            p = c"".as_ptr();
        }

        let mut had_error = false;

        'fmt_loop: while *p != NUL {
            if *p != b'%' as c_char {
                let next_pct = xstrchrnul(p.add(1), b'%' as c_char);
                let n = next_pct.offset_from(p) as usize;
                if str_avail {
                    let avail = str_m - str_l;
                    ptr::copy(p, str_out.add(str_l), n.min(avail));
                    str_avail = n < avail;
                }
                p = p.add(n);
                str_l += n;
            } else {
                let mut min_field_width: usize = 0;
                let mut precision: usize = 0;
                let mut zero_padding = false;
                let mut precision_specified = false;
                let mut justify_left = false;
                let mut alternate_form = false;
                let mut force_sign = false;
                let mut space_for_positive: c_int = 1;
                let mut length_modifier: c_char = NUL;
                let mut tmp = [0 as c_char; TMP_LEN];
                let mut str_arg: *const c_char = ptr::null();
                let mut str_arg_l: usize;
                let uchar_arg: u8;
                let mut number_of_zeros_to_pad: usize = 0;
                let mut zero_padding_insertion_ind: usize = 0;
                let mut tofree: *mut c_char = ptr::null_mut();
                let mut pos_arg: c_int = -1;

                p = p.add(1);

                let mut ptype = p;
                while ascii_isdigit(*ptype) {
                    ptype = ptype.add(1);
                }

                if *ptype == b'$' as c_char {
                    let digstart = p;
                    let mut uj: c_uint = 0;
                    if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null()) == FAIL {
                        had_error = true;
                        break 'fmt_loop;
                    }
                    pos_arg = uj as c_int;
                    p = p.add(1);
                }

                loop {
                    match *p as u8 {
                        b'0' => {
                            zero_padding = true;
                            p = p.add(1);
                        }
                        b'-' => {
                            justify_left = true;
                            p = p.add(1);
                        }
                        b'+' => {
                            force_sign = true;
                            space_for_positive = 0;
                            p = p.add(1);
                        }
                        b' ' => {
                            force_sign = true;
                            p = p.add(1);
                        }
                        b'#' => {
                            alternate_form = true;
                            p = p.add(1);
                        }
                        b'\'' => {
                            p = p.add(1);
                        }
                        _ => break,
                    }
                }

                if *p == b'*' as c_char {
                    let digstart = p.add(1);
                    p = p.add(1);

                    if ascii_isdigit(*p) {
                        let mut uj: c_uint = 0;
                        if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null())
                            == FAIL
                        {
                            had_error = true;
                            break 'fmt_loop;
                        }
                        arg_idx = uj as c_int;
                        p = p.add(1);
                    }

                    let mut j = if !tvs.is_null() {
                        tv_nr(tvs, &raw mut arg_idx) as c_int
                    } else {
                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_int
                    };

                    if j > MAX_ALLOWED_STRING_WIDTH as c_int {
                        if !tvs.is_null() {
                            format_overflow_error(digstart);
                            had_error = true;
                            break 'fmt_loop;
                        } else {
                            j = MAX_ALLOWED_STRING_WIDTH as c_int;
                        }
                    }

                    if j >= 0 {
                        min_field_width = j as usize;
                    } else {
                        min_field_width = (-j) as usize;
                        justify_left = true;
                    }
                } else if ascii_isdigit(*p) {
                    let digstart = p;
                    let mut uj: c_uint = 0;
                    if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null()) == FAIL {
                        had_error = true;
                        break 'fmt_loop;
                    }
                    min_field_width = uj as usize;
                }

                if *p == b'.' as c_char {
                    p = p.add(1);
                    precision_specified = true;

                    if ascii_isdigit(*p) {
                        let digstart = p;
                        let mut uj: c_uint = 0;
                        if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null())
                            == FAIL
                        {
                            had_error = true;
                            break 'fmt_loop;
                        }
                        precision = uj as usize;
                    } else if *p == b'*' as c_char {
                        let digstart = p;
                        p = p.add(1);

                        if ascii_isdigit(*p) {
                            let mut uj: c_uint = 0;
                            if get_unsigned_int(digstart, &raw mut p, &raw mut uj, !tvs.is_null())
                                == FAIL
                            {
                                had_error = true;
                                break 'fmt_loop;
                            }
                            arg_idx = uj as c_int;
                            p = p.add(1);
                        }

                        let mut j = if !tvs.is_null() {
                            tv_nr(tvs, &raw mut arg_idx) as c_int
                        } else {
                            fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_int
                        };

                        if j > MAX_ALLOWED_STRING_WIDTH as c_int {
                            if !tvs.is_null() {
                                format_overflow_error(digstart);
                                had_error = true;
                                break 'fmt_loop;
                            } else {
                                j = MAX_ALLOWED_STRING_WIDTH as c_int;
                            }
                        }

                        if j >= 0 {
                            precision = j as usize;
                        } else {
                            precision_specified = false;
                            precision = 0;
                        }
                    }
                }

                if matches!(*p as u8, b'h' | b'l' | b'z') {
                    length_modifier = *p;
                    p = p.add(1);
                    if length_modifier == b'l' as c_char && *p == b'l' as c_char {
                        length_modifier = b'L' as c_char;
                        p = p.add(1);
                    }
                }

                let mut fmt_spec = *p;
                match fmt_spec as u8 {
                    b'i' => fmt_spec = b'd' as c_char,
                    b'D' => {
                        fmt_spec = b'd' as c_char;
                        length_modifier = b'l' as c_char;
                    }
                    b'U' => {
                        fmt_spec = b'u' as c_char;
                        length_modifier = b'l' as c_char;
                    }
                    b'O' => {
                        fmt_spec = b'o' as c_char;
                        length_modifier = b'l' as c_char;
                    }
                    _ => {}
                }

                if matches!(fmt_spec as u8, b'd' | b'u' | b'o' | b'x' | b'X')
                    && !tvs.is_null()
                    && length_modifier == NUL
                {
                    length_modifier = b'L' as c_char;
                }

                if pos_arg != -1 {
                    arg_idx = pos_arg;
                }

                match fmt_spec as u8 {
                    b'%' | b'c' | b's' | b'S' => {
                        str_arg_l = 1;
                        match fmt_spec as u8 {
                            b'%' => {
                                str_arg = p;
                            }
                            b'c' => {
                                let j = if !tvs.is_null() {
                                    tv_nr(tvs, &raw mut arg_idx) as c_int
                                } else {
                                    fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_int
                                };
                                uchar_arg = j as u8;
                                str_arg = (&raw const uchar_arg).cast::<c_char>();
                            }
                            b's' | b'S' => {
                                str_arg = if !tvs.is_null() {
                                    tv_str(tvs, &raw mut arg_idx, &raw mut tofree)
                                } else {
                                    fmt_arg_val(fmt_args, &mut arg_idx)
                                        .as_ptr()
                                        .cast::<c_char>()
                                };

                                if str_arg.is_null() {
                                    str_arg = c"[NULL]".as_ptr();
                                    str_arg_l = 6;
                                } else if !precision_specified {
                                    str_arg_l = CStr::from_ptr(str_arg).to_bytes().len();
                                } else if precision == 0 {
                                    str_arg_l = 0;
                                } else {
                                    let end =
                                        xmemscan(str_arg.cast(), NUL, precision.min(0x7fffffff))
                                            .cast::<c_char>();
                                    str_arg_l = end.offset_from(str_arg) as usize;
                                }

                                if fmt_spec == b'S' as c_char {
                                    let mut p1 = str_arg;
                                    let mut i: usize = 0;
                                    while *p1 != NUL {
                                        let cell = utf_ptr2cells(p1) as usize;
                                        if precision_specified && i + cell > precision {
                                            break;
                                        }
                                        i += cell;
                                        p1 = p1.add(utfc_ptr2len(p1) as usize);
                                    }
                                    str_arg_l = p1.offset_from(str_arg) as usize;
                                    if min_field_width != 0 {
                                        min_field_width =
                                            min_field_width.wrapping_add(str_arg_l).wrapping_sub(i);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    b'd' | b'u' | b'b' | b'B' | b'o' | b'x' | b'X' | b'p' => {
                        let mut arg_sign: c_int = 0;
                        let mut arg: i64 = 0;
                        let mut uarg: u64 = 0;
                        let mut ptr_arg: *const c_void = ptr::null();

                        if fmt_spec == b'p' as c_char {
                            ptr_arg = if !tvs.is_null() {
                                tv_ptr(tvs, &raw mut arg_idx)
                            } else {
                                fmt_arg_val(fmt_args, &mut arg_idx).as_ptr()
                            };
                            if !ptr_arg.is_null() {
                                arg_sign = 1;
                            }
                        } else if fmt_spec == b'b' as c_char || fmt_spec == b'B' as c_char {
                            uarg = if !tvs.is_null() {
                                tv_nr(tvs, &raw mut arg_idx) as c_ulonglong
                            } else {
                                fmt_arg_val(fmt_args, &mut arg_idx).as_u64()
                            };
                            arg_sign = c_int::from(uarg != 0);
                        } else if fmt_spec == b'd' as c_char {
                            match length_modifier as u8 {
                                0 => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_int
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_int
                                    };
                                    arg = i64::from(v);
                                }
                                b'h' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_int
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_int
                                    };
                                    arg = i64::from(v as i16);
                                }
                                b'l' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_long
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as c_long
                                    };
                                    arg = v as i64;
                                }
                                b'L' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_longlong
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64()
                                    };
                                    arg = v;
                                }
                                b'z' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as isize
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as isize
                                    };
                                    arg = v as i64;
                                }
                                _ => {}
                            }
                            if arg > 0 {
                                arg_sign = 1;
                            } else if arg < 0 {
                                arg_sign = -1;
                            }
                        } else {
                            match length_modifier as u8 {
                                0 => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_uint
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as c_uint
                                    };
                                    uarg = u64::from(v);
                                }
                                b'h' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_uint
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as c_uint
                                    };
                                    uarg = u64::from(v as u16);
                                }
                                b'l' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_ulong
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as c_ulong
                                    };
                                    uarg = v as u64;
                                }
                                b'L' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as c_ulonglong
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64()
                                    };
                                    uarg = v;
                                }
                                b'z' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &raw mut arg_idx) as usize
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as usize
                                    };
                                    uarg = v as u64;
                                }
                                _ => {}
                            }
                            arg_sign = c_int::from(uarg != 0);
                        }

                        str_arg = tmp.as_ptr();
                        str_arg_l = 0;

                        if precision_specified {
                            zero_padding = false;
                        }

                        if fmt_spec == b'd' as c_char {
                            if force_sign && arg_sign >= 0 {
                                tmp[str_arg_l] = if space_for_positive != 0 {
                                    b' ' as c_char
                                } else {
                                    b'+' as c_char
                                };
                                str_arg_l += 1;
                            }
                        } else if alternate_form
                            && arg_sign != 0
                            && matches!(fmt_spec as u8, b'x' | b'X' | b'b' | b'B')
                        {
                            tmp[str_arg_l] = b'0' as c_char;
                            str_arg_l += 1;
                            tmp[str_arg_l] = fmt_spec;
                            str_arg_l += 1;
                        }

                        zero_padding_insertion_ind = str_arg_l;
                        if !precision_specified {
                            precision = 1;
                        }
                        if precision == 0 && arg_sign == 0 {
                            // empty
                        } else {
                            match fmt_spec as u8 {
                                b'p' => {
                                    str_arg_l += snprintf(
                                        tmp.as_mut_ptr().add(str_arg_l),
                                        tmp.len() - str_arg_l,
                                        c"%p".as_ptr(),
                                        ptr_arg,
                                    ) as usize;
                                }
                                b'd' => {
                                    str_arg_l += snprintf(
                                        tmp.as_mut_ptr().add(str_arg_l),
                                        tmp.len() - str_arg_l,
                                        c"%lld".as_ptr(),
                                        arg as c_longlong,
                                    ) as usize;
                                }
                                b'b' | b'B' => {
                                    let mut bits = 64usize;
                                    while bits > 0 {
                                        if ((uarg >> (bits - 1)) & 1) != 0 {
                                            break;
                                        }
                                        bits -= 1;
                                    }
                                    while bits > 0 {
                                        bits -= 1;
                                        tmp[str_arg_l] = if ((uarg >> bits) & 1) != 0 {
                                            b'1' as c_char
                                        } else {
                                            b'0' as c_char
                                        };
                                        str_arg_l += 1;
                                    }
                                }
                                _ => {
                                    let mut f = [
                                        b'%' as c_char,
                                        b'l' as c_char,
                                        b'l' as c_char,
                                        fmt_spec,
                                        0,
                                    ];
                                    str_arg_l += snprintf(
                                        tmp.as_mut_ptr().add(str_arg_l),
                                        tmp.len() - str_arg_l,
                                        f.as_mut_ptr(),
                                        uarg as c_ulonglong,
                                    ) as usize;
                                }
                            }

                            if zero_padding_insertion_ind < str_arg_l
                                && tmp[zero_padding_insertion_ind] == b'-' as c_char
                            {
                                zero_padding_insertion_ind += 1;
                            }
                            if zero_padding_insertion_ind + 1 < str_arg_l
                                && tmp[zero_padding_insertion_ind] == b'0' as c_char
                                && matches!(
                                    tmp[zero_padding_insertion_ind + 1] as u8,
                                    b'x' | b'X' | b'b' | b'B'
                                )
                            {
                                zero_padding_insertion_ind += 2;
                            }
                        }

                        {
                            let num_of_digits = str_arg_l - zero_padding_insertion_ind;
                            if alternate_form
                                && fmt_spec == b'o' as c_char
                                && !(zero_padding_insertion_ind < str_arg_l
                                    && tmp[zero_padding_insertion_ind] == b'0' as c_char)
                                && (!precision_specified || precision < num_of_digits + 1)
                            {
                                precision = num_of_digits + 1;
                            }
                            if num_of_digits < precision {
                                number_of_zeros_to_pad = precision - num_of_digits;
                            }
                        }

                        if !justify_left && zero_padding {
                            let n = (min_field_width as isize)
                                - ((str_arg_l + number_of_zeros_to_pad) as isize);
                            if n > 0 {
                                number_of_zeros_to_pad += n as usize;
                            }
                        }
                    }
                    b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                        let mut format = [0 as c_char; 40];
                        let mut remove_trailing_zeroes = false;

                        let f = if !tvs.is_null() {
                            tv_float(tvs, &raw mut arg_idx)
                        } else {
                            fmt_arg_val(fmt_args, &mut arg_idx).as_f64()
                        };

                        let abs_f = f.abs();

                        if fmt_spec == b'g' as c_char || fmt_spec == b'G' as c_char {
                            if (0.001..10_000_000.0).contains(&abs_f) || abs_f == 0.0 {
                                fmt_spec = if (fmt_spec as u8).is_ascii_uppercase() {
                                    b'F' as c_char
                                } else {
                                    b'f' as c_char
                                };
                            } else {
                                fmt_spec = if fmt_spec == b'g' as c_char {
                                    b'e' as c_char
                                } else {
                                    b'E' as c_char
                                };
                            }
                            remove_trailing_zeroes = true;
                        }

                        if xisinf(f) != 0
                            || (matches!(fmt_spec as u8, b'f' | b'F') && abs_f > 1.0e307)
                        {
                            xstrlcpy(
                                tmp.as_mut_ptr(),
                                infinity_str(
                                    f > 0.0,
                                    fmt_spec,
                                    c_int::from(force_sign),
                                    space_for_positive,
                                ),
                                tmp.len(),
                            );
                            str_arg_l = CStr::from_ptr(tmp.as_ptr()).to_bytes().len();
                            zero_padding = false;
                        } else if xisnan(f) != 0 {
                            let nan_s = if (fmt_spec as u8).is_ascii_uppercase() {
                                c"NAN"
                            } else {
                                c"nan"
                            };
                            ptr::copy_nonoverlapping(nan_s.as_ptr(), tmp.as_mut_ptr(), 4);
                            str_arg_l = 3;
                            zero_padding = false;
                        } else {
                            format[0] = b'%' as c_char;
                            let mut l = 1usize;
                            if force_sign {
                                format[l] = if space_for_positive != 0 {
                                    b' ' as c_char
                                } else {
                                    b'+' as c_char
                                };
                                l += 1;
                            }
                            if precision_specified {
                                let mut max_prec = TMP_LEN - 10;
                                if matches!(fmt_spec as u8, b'f' | b'F') && abs_f > 1.0 {
                                    max_prec -= abs_f.log10() as usize;
                                }
                                if precision > max_prec {
                                    precision = max_prec;
                                }
                                l += snprintf(
                                    format.as_mut_ptr().add(l),
                                    format.len() - l,
                                    c".%d".as_ptr(),
                                    precision as c_int,
                                ) as usize;
                            }

                            format[l] = if fmt_spec == b'F' as c_char {
                                b'f' as c_char
                            } else {
                                fmt_spec
                            };
                            format[l + 1] = NUL;

                            str_arg_l =
                                snprintf(tmp.as_mut_ptr(), tmp.len(), format.as_ptr(), f) as usize;

                            if remove_trailing_zeroes {
                                let mut tp: *mut c_char;
                                if matches!(fmt_spec as u8, b'f' | b'F') {
                                    tp = tmp.as_mut_ptr().add(str_arg_l - 1);
                                } else {
                                    tp = vim_strchr(
                                        tmp.as_ptr(),
                                        if fmt_spec == b'e' as c_char {
                                            b'e' as c_int
                                        } else {
                                            b'E' as c_int
                                        },
                                    );
                                    if !tp.is_null() {
                                        if *tp.add(1) == b'+' as c_char {
                                            let rem_len =
                                                CStr::from_ptr(tp.add(2)).to_bytes().len() + 1;
                                            ptr::copy(tp.add(2), tp.add(1), rem_len);
                                            str_arg_l -= 1;
                                        }
                                        let i = if *tp.add(1) == b'-' as c_char { 2 } else { 1 };
                                        while *tp.add(i) == b'0' as c_char {
                                            let rem_len =
                                                CStr::from_ptr(tp.add(i + 1)).to_bytes().len() + 1;
                                            ptr::copy(tp.add(i + 1), tp.add(i), rem_len);
                                            str_arg_l -= 1;
                                        }
                                        tp = tp.sub(1);
                                    }
                                }

                                if !tp.is_null() && !precision_specified {
                                    while tp > tmp.as_mut_ptr().add(2)
                                        && *tp == b'0' as c_char
                                        && *tp.sub(1) != b'.' as c_char
                                    {
                                        let rem_len =
                                            CStr::from_ptr(tp.add(1)).to_bytes().len() + 1;
                                        ptr::copy(tp.add(1), tp, rem_len);
                                        tp = tp.sub(1);
                                        str_arg_l -= 1;
                                    }
                                }
                            } else {
                                let tp = vim_strchr(
                                    tmp.as_ptr(),
                                    if fmt_spec == b'e' as c_char {
                                        b'e' as c_int
                                    } else {
                                        b'E' as c_int
                                    },
                                );
                                if !tp.is_null()
                                    && (*tp.add(1) == b'+' as c_char
                                        || *tp.add(1) == b'-' as c_char)
                                    && *tp.add(2) == b'0' as c_char
                                    && ascii_isdigit(*tp.add(3))
                                    && ascii_isdigit(*tp.add(4))
                                {
                                    let rem_len = CStr::from_ptr(tp.add(3)).to_bytes().len() + 1;
                                    ptr::copy(tp.add(3), tp.add(2), rem_len);
                                    str_arg_l -= 1;
                                }
                            }
                        }

                        if zero_padding
                            && min_field_width > str_arg_l
                            && (tmp[0] == b'-' as c_char || force_sign)
                        {
                            number_of_zeros_to_pad = min_field_width - str_arg_l;
                            zero_padding_insertion_ind = 1;
                        }
                        str_arg = tmp.as_ptr();
                    }
                    _ => {
                        zero_padding = false;
                        justify_left = true;
                        min_field_width = 0;
                        str_arg = p;
                        str_arg_l = usize::from(*p != NUL);
                    }
                }

                if *p != NUL {
                    p = p.add(1);
                }

                if !justify_left && min_field_width > str_arg_l + number_of_zeros_to_pad {
                    let pn = min_field_width - (str_arg_l + number_of_zeros_to_pad);
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::write_bytes(
                            str_out.add(str_l),
                            if zero_padding { b'0' } else { b' ' },
                            pn.min(avail),
                        );
                        str_avail = pn < avail;
                    }
                    str_l += pn;
                }

                if number_of_zeros_to_pad == 0 {
                    zero_padding_insertion_ind = 0;
                } else {
                    if zero_padding_insertion_ind > 0 {
                        let zn = zero_padding_insertion_ind;
                        if str_avail {
                            let avail = str_m - str_l;
                            ptr::copy(str_arg, str_out.add(str_l), zn.min(avail));
                            str_avail = zn < avail;
                        }
                        str_l += zn;
                    }

                    let zn = number_of_zeros_to_pad;
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::write_bytes(str_out.add(str_l), b'0', zn.min(avail));
                        str_avail = zn < avail;
                    }
                    str_l += zn;
                }

                if str_arg_l > zero_padding_insertion_ind {
                    let sn = str_arg_l - zero_padding_insertion_ind;
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::copy(
                            str_arg.add(zero_padding_insertion_ind),
                            str_out.add(str_l),
                            sn.min(avail),
                        );
                        str_avail = sn < avail;
                    }
                    str_l += sn;
                }

                if justify_left && min_field_width > str_arg_l + number_of_zeros_to_pad {
                    let pn = min_field_width - (str_arg_l + number_of_zeros_to_pad);
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::write_bytes(str_out.add(str_l), b' ', pn.min(avail));
                        str_avail = pn < avail;
                    }
                    str_l += pn;
                }

                xfree(tofree.cast());
            }
        }

        if !had_error {
            if str_m > 0 {
                *str_out.add(str_l.min(str_m - 1)) = NUL;
            }

            #[cfg(not(test))]
            if !tvs.is_null() {
                let check_idx = if num_posarg != 0 {
                    num_posarg as usize
                } else {
                    (arg_idx - 1) as usize
                };
                if (*tvs.add(check_idx)).v_type != VarType::VAR_UNKNOWN {
                    emsg(gettext(E_TOO_MANY_ARGS.as_ptr()));
                }
            }
        }

        xfree(ap_types.cast());
        str_l as c_int
    }
}

pub unsafe fn vim_snprintf_fmt(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    args: &[FmtVal],
) -> c_int {
    unsafe { vim_vsnprintf_typval_inner(str_out, str_m, fmt, ptr::null_mut(), args) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_snprintf_args(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    args: *const VimFmtArg,
    argc: usize,
) -> c_int {
    let mut vals = Vec::with_capacity(argc);
    if !args.is_null() {
        for idx in 0..argc {
            let a = unsafe { *args.add(idx) };
            let v = match a.tag {
                1 => FmtVal::Unsigned(a.u),
                2 => FmtVal::Float(a.f),
                3 => FmtVal::Ptr(a.p),
                _ => FmtVal::Signed(a.i),
            };
            vals.push(v);
        }
    }
    unsafe { vim_snprintf_fmt(str_out, str_m, fmt, &vals) }
}

pub unsafe fn vim_snprintf_add_fmt(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    args: &[FmtVal],
) -> c_int {
    unsafe {
        let len = CStr::from_ptr(str_out).to_bytes().len();
        let space = str_m.saturating_sub(len);
        vim_snprintf_fmt(str_out.add(len), space, fmt, args)
    }
}

pub unsafe fn vim_snprintf_safelen_fmt(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    args: &[FmtVal],
) -> usize {
    unsafe {
        if str_m == 0 {
            return 0;
        }
        let str_l = vim_snprintf_fmt(str_out, str_m, fmt, args);
        if str_l < 0 {
            *str_out = NUL;
            return 0;
        }
        let len = str_l as usize;
        if len >= str_m { str_m - 1 } else { len }
    }
}

#[macro_export]
macro_rules! vim_snprintf {
    ($str_out:expr, $str_m:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::strings::vim_snprintf_fmt(
            $str_out,
            $str_m,
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
pub use vim_snprintf;

#[macro_export]
macro_rules! vim_snprintf_add {
    ($str_out:expr, $str_m:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::strings::vim_snprintf_add_fmt(
            $str_out,
            $str_m,
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
pub use vim_snprintf_add;

#[macro_export]
macro_rules! vim_snprintf_safelen {
    ($str_out:expr, $str_m:expr, $fmt:expr $(, $arg:expr)* $(,)?) => {
        $crate::strings::vim_snprintf_safelen_fmt(
            $str_out,
            $str_m,
            $fmt,
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
use crate::api::buffer::NUL;
use crate::cmdexpand::MB_MAXBYTES;
#[cfg(not(test))]
use crate::eval::kListLenUnknown;
#[cfg(not(test))]
use crate::regexp::NUMBUFLEN;
pub use vim_snprintf_safelen;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_snprintf_typval_only(
    str_out: *mut c_char,
    str_m: usize,
    fmt: *const c_char,
    tvs: *mut typval_T,
) -> c_int {
    unsafe { vim_vsnprintf_typval_inner(str_out, str_m, fmt, tvs, &[]) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn reverse_text(s: *mut c_char) -> *mut c_char {
    unsafe {
        let len = CStr::from_ptr(s).to_bytes().len();
        let rev = xmalloc(len + 1).cast::<c_char>();
        let mut s_i: usize = 0;
        let mut rev_i: usize = len;
        while s_i < len {
            let mb_len = utfc_ptr2len(s.add(s_i)) as usize;
            rev_i -= mb_len;
            ptr::copy(s.add(s_i), rev.add(rev_i), mb_len);
            s_i += mb_len;
        }
        *rev.add(len) = NUL;
        rev
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strrep(
    src: *const c_char,
    what: *const c_char,
    rep: *const c_char,
) -> *mut c_char {
    unsafe {
        let src_bytes = CStr::from_ptr(src).to_bytes();
        let what_bytes = CStr::from_ptr(what).to_bytes();
        let rep_bytes = CStr::from_ptr(rep).to_bytes();

        if what_bytes.is_empty() {
            return ptr::null_mut();
        }

        let mut count: usize = 0;
        let mut idx: usize = 0;
        while idx + what_bytes.len() <= src_bytes.len() {
            if let Some(rel) = src_bytes[idx..]
                .windows(what_bytes.len())
                .position(|w| w == what_bytes)
            {
                count += 1;
                idx += rel + what_bytes.len();
            } else {
                break;
            }
        }

        if count == 0 {
            return ptr::null_mut();
        }

        let total_len = (src_bytes.len() + count * rep_bytes.len()) - count * what_bytes.len();
        let ret = xmalloc(total_len + 1).cast::<u8>();
        let mut dst = ret;
        let mut cur: usize = 0;
        while cur + what_bytes.len() <= src_bytes.len() {
            if let Some(rel) = src_bytes[cur..]
                .windows(what_bytes.len())
                .position(|w| w == what_bytes)
            {
                ptr::copy_nonoverlapping(src_bytes.as_ptr().add(cur), dst, rel);
                dst = dst.add(rel);
                ptr::copy_nonoverlapping(rep_bytes.as_ptr(), dst, rep_bytes.len());
                dst = dst.add(rep_bytes.len());
                cur += rel + what_bytes.len();
            } else {
                break;
            }
        }
        let rem = src_bytes.len() - cur;
        ptr::copy_nonoverlapping(src_bytes.as_ptr().add(cur), dst, rem);
        *dst.add(rem) = 0;

        ret.cast()
    }
}

unsafe fn byteidx_common(argvars: *mut typval_T, rettv: *mut typval_T, comp: bool) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        let str_ = tv_get_string_chk(argvars);
        let mut idx = tv_get_number_chk(argvars.add(1), ptr::null_mut());
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut utf16idx: varnumber_T = 0;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;
            utf16idx = tv_get_bool_chk(argvars.add(2), &raw mut error);
            if error {
                return;
            }
            if !(0..=1).contains(&utf16idx) {
                semsg!(
                    gettext((&raw const e_using_number_as_bool_nr).cast()),
                    utf16idx,
                );
                return;
            }
        }

        let ptr2len: unsafe extern "C" fn(*const c_char) -> c_int =
            if comp { utf_ptr2len } else { utfc_ptr2len };

        let mut t = str_;
        while idx > 0 {
            if *t == NUL {
                return;
            }
            if utf16idx != 0 {
                let clen = ptr2len(t);
                let c = if clen > 1 {
                    utf_ptr2char(t)
                } else {
                    *t as c_int
                };
                if c > 0xFFFF {
                    idx -= 1;
                }
                if idx > 0 {
                    t = t.add(clen as usize);
                }
            } else {
                t = t.add(ptr2len(t) as usize);
            }
            idx -= 1;
        }
        (*rettv).vval.v_number = t.offset_from(str_) as varnumber_T;
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv, comp);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_byteidx(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        byteidx_common(argvars, rettv, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_byteidxcomp(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        byteidx_common(argvars, rettv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_charidx(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_number_arg(argvars, 1) == FAIL
            || tv_check_for_opt_bool_arg(argvars, 2) == FAIL
            || ((*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
                && tv_check_for_opt_bool_arg(argvars, 3) == FAIL)
        {
            return;
        }

        let str_ = tv_get_string_chk(argvars);
        let mut idx = tv_get_number_chk(argvars.add(1), ptr::null_mut());
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut countcc: varnumber_T = 0;
        let mut utf16idx: varnumber_T = 0;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            countcc = tv_get_bool(argvars.add(2));
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                utf16idx = tv_get_bool(argvars.add(3));
            }
        }

        let ptr2len: unsafe extern "C" fn(*const c_char) -> c_int = if countcc != 0 {
            utf_ptr2len
        } else {
            utfc_ptr2len
        };

        let mut p = str_;
        let mut len: c_int = 0;
        while if utf16idx != 0 {
            idx >= 0
        } else {
            p <= str_.add(idx as usize)
        } {
            if *p == NUL {
                if if utf16idx != 0 {
                    idx == 0
                } else {
                    p == str_.add(idx as usize)
                } {
                    (*rettv).vval.v_number = varnumber_T::from(len);
                }
                return;
            }
            if utf16idx != 0 {
                idx -= 1;
                let clen = ptr2len(p);
                let c = if clen > 1 {
                    utf_ptr2char(p)
                } else {
                    *p as c_int
                };
                if c > 0xFFFF {
                    idx -= 1;
                }
            }
            p = p.add(ptr2len(p) as usize);
            len += 1;
        }

        (*rettv).vval.v_number = if len > 0 {
            varnumber_T::from(len - 1)
        } else {
            0
        };
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_str2list(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let mut p = tv_get_string(argvars);
        while *p != NUL {
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(utf_ptr2char(p)));
            p = p.add(utf_ptr2len(p) as usize);
        }
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_str2nr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let mut base: c_int = 10;
        let mut what: c_int = 0;

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            base = tv_get_number(argvars.add(1)) as c_int;
            if !matches!(base, 2 | 8 | 10 | 16) {
                emsg(gettext((&raw const e_invarg).cast()));
                return;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars.add(2)) != 0
            {
                what |= STR2NR_QUOTE;
            }
        }

        let mut p = skipwhite(tv_get_string(argvars));
        let isneg = *p == b'-' as c_char;
        if *p == b'+' as c_char || *p == b'-' as c_char {
            p = skipwhite(p.add(1));
        }
        match base {
            2 => what |= STR2NR_BIN | STR2NR_FORCE,
            8 => what |= STR2NR_OCT | STR2NR_OOCT | STR2NR_FORCE,
            16 => what |= STR2NR_HEX | STR2NR_FORCE,
            _ => {}
        }
        let mut n: varnumber_T = 0;
        vim_str2nr(
            p,
            ptr::null_mut(),
            ptr::null_mut(),
            what,
            &raw mut n,
            ptr::null_mut(),
            0,
            false,
            ptr::null_mut(),
        );
        (*rettv).vval.v_number = if isneg { -n } else { n };
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strgetchar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        let str_ = tv_get_string_chk(argvars);
        if str_.is_null() {
            return;
        }
        let mut error = false;
        let mut charidx = tv_get_number_chk(argvars.add(1), &raw mut error);
        if error {
            return;
        }

        let len = CStr::from_ptr(str_).to_bytes().len();
        let mut byteidx: usize = 0;
        while charidx >= 0 && byteidx < len {
            if charidx == 0 {
                (*rettv).vval.v_number = varnumber_T::from(utf_ptr2char(str_.add(byteidx)));
                break;
            }
            charidx -= 1;
            byteidx += utf_ptr2len(str_.add(byteidx)) as usize;
        }
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_stridx(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        let mut buf = [0 as c_char; NUMBUFLEN];
        let needle = tv_get_string_chk(argvars.add(1));
        let mut haystack = tv_get_string_buf_chk(argvars, buf.as_mut_ptr());
        let haystack_start = haystack;
        if needle.is_null() || haystack.is_null() {
            return;
        }

        let hay_len = CStr::from_ptr(haystack).to_bytes().len();
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;
            let start_idx = tv_get_number_chk(argvars.add(2), &raw mut error) as isize;
            if error || start_idx >= hay_len as isize {
                return;
            }
            if start_idx >= 0 {
                haystack = haystack.add(start_idx as usize);
            }
        }

        let hay_bytes = CStr::from_ptr(haystack).to_bytes();
        let needle_bytes = CStr::from_ptr(needle).to_bytes();
        if needle_bytes.is_empty() {
            (*rettv).vval.v_number = haystack.offset_from(haystack_start) as varnumber_T;
        } else if let Some(pos) = hay_bytes
            .windows(needle_bytes.len())
            .position(|w| w == needle_bytes)
        {
            (*rettv).vval.v_number = haystack.add(pos).offset_from(haystack_start) as varnumber_T;
        }
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_string(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = encode_tv2string(argvars, ptr::null_mut());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strlen(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number =
            CStr::from_ptr(tv_get_string(argvars)).to_bytes().len() as varnumber_T;
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

unsafe fn strchar_common(argvars: *mut typval_T, rettv: *mut typval_T, skipcc: bool) {
    #[cfg(not(test))]
    unsafe {
        let mut s = tv_get_string(argvars);
        let mut len: varnumber_T = 0;
        let func_mb_ptr2char_adv: unsafe extern "C" fn(*mut *const c_char) -> c_int = if skipcc {
            mb_ptr2char_adv
        } else {
            mb_cptr2char_adv
        };
        while *s != NUL {
            func_mb_ptr2char_adv(&raw mut s);
            len += 1;
        }
        (*rettv).vval.v_number = len;
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv, skipcc);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strcharlen(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        strchar_common(argvars, rettv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strchars(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let mut skipcc: varnumber_T = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;
            skipcc = tv_get_bool_chk(argvars.add(1), &raw mut error);
            if error {
                return;
            }
            if !(0..=1).contains(&skipcc) {
                semsg!(
                    gettext((&raw const e_using_number_as_bool_nr).cast()),
                    skipcc,
                );
                return;
            }
        }
        strchar_common(argvars, rettv, skipcc != 0);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strutf16len(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_opt_bool_arg(argvars, 1) == FAIL
        {
            return;
        }

        let mut countcc: varnumber_T = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            countcc = tv_get_bool(argvars.add(1));
        }

        let mut s = tv_get_string(argvars);
        let mut len: varnumber_T = 0;
        let func_mb_ptr2char_adv: unsafe extern "C" fn(*mut *const c_char) -> c_int =
            if countcc != 0 {
                mb_cptr2char_adv
            } else {
                mb_ptr2char_adv
            };
        while *s != NUL {
            let ch = func_mb_ptr2char_adv(&raw mut s);
            if ch > 0xFFFF {
                len += 1;
            }
            len += 1;
        }
        (*rettv).vval.v_number = len;
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strdisplaywidth(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let s = tv_get_string(argvars);
        let mut col: c_int = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            col = tv_get_number(argvars.add(1)) as c_int;
        }
        (*rettv).vval.v_number = varnumber_T::from(linetabsize_col(col, s.cast_mut()) - col);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strwidth(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let s = tv_get_string(argvars);
        (*rettv).vval.v_number = mb_string2cells(s) as varnumber_T;
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strcharpart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let p = tv_get_string(argvars);
        let slen = CStr::from_ptr(p).to_bytes().len();

        let mut nbyte: c_int = 0;
        let mut skipcc: varnumber_T = 0;
        let mut error = false;
        let mut nchar = tv_get_number_chk(argvars.add(1), &raw mut error);
        if !error {
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN
            {
                skipcc = tv_get_bool_chk(argvars.add(3), &raw mut error);
                if error {
                    return;
                }
                if !(0..=1).contains(&skipcc) {
                    semsg!(
                        gettext((&raw const e_using_number_as_bool_nr).cast()),
                        skipcc,
                    );
                    return;
                }
            }

            if nchar > 0 {
                while nchar > 0 && (nbyte as usize) < slen {
                    if skipcc != 0 {
                        nbyte += utfc_ptr2len(p.add(nbyte as usize));
                    } else {
                        nbyte += utf_ptr2len(p.add(nbyte as usize));
                    }
                    nchar -= 1;
                }
            } else {
                nbyte = nchar as c_int;
            }
        }
        let mut len: c_int = 0;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut charlen = tv_get_number(argvars.add(2)) as c_int;
            while charlen > 0 && nbyte + len < slen as c_int {
                let off = nbyte + len;
                if off < 0 {
                    len += 1;
                } else if skipcc != 0 {
                    len += utfc_ptr2len(p.add(off as usize));
                } else {
                    len += utf_ptr2len(p.add(off as usize));
                }
                charlen -= 1;
            }
        } else {
            len = (slen as c_int) - nbyte;
        }

        if nbyte < 0 {
            len += nbyte;
            nbyte = 0;
        } else if (nbyte as usize) > slen {
            nbyte = slen as c_int;
        }
        if len < 0 {
            len = 0;
        } else if nbyte + len > slen as c_int {
            len = (slen as c_int) - nbyte;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(p.add(nbyte as usize).cast(), len as usize).cast();
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strpart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let mut error = false;
        let p = tv_get_string(argvars);
        let slen = CStr::from_ptr(p).to_bytes().len() as varnumber_T;

        let mut n = tv_get_number_chk(argvars.add(1), &raw mut error);
        let mut len = if error {
            0
        } else if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            tv_get_number(argvars.add(2))
        } else {
            slen - n
        };

        if n < 0 {
            len += n;
            n = 0;
        } else if n > slen {
            n = slen;
        }
        if len < 0 {
            len = 0;
        } else if n + len > slen {
            len = slen - n;
        }

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            && (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN
        {
            let mut off = n;
            while off < slen && len > 0 {
                off += varnumber_T::from(utfc_ptr2len(p.add(off as usize)));
                len -= 1;
            }
            len = off - n;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(p.add(n as usize).cast(), len as usize).cast();
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strridx(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let needle = tv_get_string_chk(argvars.add(1));
        let haystack = tv_get_string_buf_chk(argvars, buf.as_mut_ptr());

        (*rettv).vval.v_number = -1;
        if needle.is_null() || haystack.is_null() {
            return;
        }

        let hay_bytes = CStr::from_ptr(haystack).to_bytes();
        let needle_bytes = CStr::from_ptr(needle).to_bytes();
        let haystack_len = hay_bytes.len();
        let end_idx: isize = if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let idx = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as isize;
            if idx < 0 {
                return;
            }
            idx
        } else {
            haystack_len as isize
        };

        if needle_bytes.is_empty() {
            (*rettv).vval.v_number = end_idx.min(haystack_len as isize) as varnumber_T;
            return;
        }

        let max_start = (end_idx as usize).min(haystack_len);
        let mut lastmatch: Option<usize> = None;
        let mut cur: usize = 0;
        while cur <= max_start && cur + needle_bytes.len() <= haystack_len {
            if let Some(rel) = hay_bytes[cur..]
                .windows(needle_bytes.len())
                .position(|w| w == needle_bytes)
            {
                let pos = cur + rel;
                if pos > max_start {
                    break;
                }
                lastmatch = Some(pos);
                cur = pos + 1;
            } else {
                break;
            }
        }

        if let Some(pos) = lastmatch {
            (*rettv).vval.v_number = pos as varnumber_T;
        }
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strtrans(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = transstr(tv_get_string(argvars), true);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_utf16idx(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_opt_number_arg(argvars, 1) == FAIL
            || tv_check_for_opt_bool_arg(argvars, 2) == FAIL
            || ((*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
                && tv_check_for_opt_bool_arg(argvars, 3) == FAIL)
        {
            return;
        }

        let str_ = tv_get_string_chk(argvars);
        let mut idx = tv_get_number_chk(argvars.add(1), ptr::null_mut());
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut countcc: varnumber_T = 0;
        let mut charidx: varnumber_T = 0;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            countcc = tv_get_bool(argvars.add(2));
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                charidx = tv_get_bool(argvars.add(3));
            }
        }

        let ptr2len: unsafe extern "C" fn(*const c_char) -> c_int = if countcc != 0 {
            utf_ptr2len
        } else {
            utfc_ptr2len
        };

        let mut p = str_;
        let mut len: c_int = 0;
        let mut utf16idx: c_int = 0;
        while if charidx != 0 {
            idx >= 0
        } else {
            p <= str_.add(idx as usize)
        } {
            if *p == NUL {
                if if charidx != 0 {
                    idx == 0
                } else {
                    p == str_.add(idx as usize)
                } {
                    (*rettv).vval.v_number = varnumber_T::from(len);
                }
                return;
            }
            utf16idx = len;
            let clen = ptr2len(p);
            let c = if clen > 1 {
                utf_ptr2char(p)
            } else {
                *p as c_int
            };
            if c > 0xFFFF {
                len += 1;
            }
            p = p.add(clen as usize);
            if charidx != 0 {
                idx -= 1;
            }
            len += 1;
        }

        (*rettv).vval.v_number = varnumber_T::from(utf16idx);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tolower(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = strcase_save(tv_get_string(argvars), false);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_toupper(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = strcase_save(tv_get_string(argvars), true);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tr(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    #[cfg(not(test))]
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];

        let mut in_str = tv_get_string(argvars);
        let fromstr = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
        let tostr = tv_get_string_buf_chk(argvars.add(2), buf2.as_mut_ptr());

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if fromstr.is_null() || tostr.is_null() {
            return;
        }
        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, size_of::<c_char>() as c_int, 80);

        let mut first = true;
        while *in_str != NUL {
            let mut cpstr = in_str;
            let inlen = utfc_ptr2len(in_str) as usize;
            let mut cplen = inlen;
            let mut idx: c_int = 0;
            let mut p = fromstr;
            while *p != NUL {
                let fromlen = utfc_ptr2len(p) as usize;
                let in_slice = std::slice::from_raw_parts(in_str.cast::<u8>(), inlen);
                let p_slice = std::slice::from_raw_parts(p.cast::<u8>(), fromlen);
                if fromlen == inlen && in_slice == p_slice {
                    let mut tp = tostr;
                    while *tp != NUL {
                        let tolen = utfc_ptr2len(tp) as usize;
                        if idx == 0 {
                            idx -= 1;
                            cplen = tolen;
                            cpstr = tp;
                            break;
                        }
                        idx -= 1;
                        tp = tp.add(tolen);
                    }
                    if *tp == NUL {
                        semsg!(gettext((&raw const e_invarg2).cast()), fromstr);
                        ga_clear(&raw mut ga);
                        return;
                    }
                    break;
                }
                idx += 1;
                p = p.add(fromlen);
            }

            if first && ptr::eq(cpstr, in_str) {
                first = false;
                let mut tp = tostr;
                while *tp != NUL {
                    let tolen = utfc_ptr2len(tp) as usize;
                    idx -= 1;
                    tp = tp.add(tolen);
                }
                if idx != 0 {
                    semsg!(gettext((&raw const e_invarg2).cast()), fromstr);
                    ga_clear(&raw mut ga);
                    return;
                }
            }

            ga_grow(&raw mut ga, cplen as c_int);
            ptr::copy(
                cpstr,
                ga.ga_data.cast::<c_char>().add(ga.ga_len as usize),
                cplen,
            );
            ga.ga_len += cplen as c_int;

            in_str = in_str.add(inlen);
        }

        ga_append(&raw mut ga, 0);
        (*rettv).vval.v_string = ga.ga_data.cast();
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_trim(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    #[cfg(not(test))]
    unsafe {
        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let mut head = tv_get_string_buf_chk(argvars, buf1.as_mut_ptr());
        let mut mask: *const c_char = ptr::null();
        let mut dir: c_int = 0;

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if head.is_null() {
            return;
        }

        if tv_check_for_opt_string_arg(argvars, 1) == FAIL {
            return;
        }

        if (*argvars.add(1)).v_type == VarType::VAR_STRING {
            mask = tv_get_string_buf_chk(argvars.add(1), buf2.as_mut_ptr());
            if *mask == NUL {
                mask = ptr::null();
            }

            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                let mut error = false;
                dir = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                if error {
                    return;
                }
                if !(0..=2).contains(&dir) {
                    semsg!(
                        gettext((&raw const e_invarg2).cast()),
                        tv_get_string(argvars.add(2)),
                    );
                    return;
                }
            }
        }

        if dir == 0 || dir == 1 {
            while *head != NUL {
                let c1 = utf_ptr2char(head);
                if mask.is_null() {
                    if c1 > b' ' as c_int && c1 != 0xa0 {
                        break;
                    }
                } else {
                    let mut p = mask;
                    while *p != NUL {
                        if c1 == utf_ptr2char(p) {
                            break;
                        }
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                    if *p == NUL {
                        break;
                    }
                }
                head = head.add(utfc_ptr2len(head) as usize);
            }
        }

        let mut tail = head.add(CStr::from_ptr(head).to_bytes().len());
        if dir == 0 || dir == 2 {
            while tail > head {
                let prev = tail.sub((utf_head_off(head, tail.sub(1)) + 1) as usize);
                let c1 = utf_ptr2char(prev);
                if mask.is_null() {
                    if c1 > b' ' as c_int && c1 != 0xa0 {
                        break;
                    }
                } else {
                    let mut p = mask;
                    while *p != NUL {
                        if c1 == utf_ptr2char(p) {
                            break;
                        }
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                    if *p == NUL {
                        break;
                    }
                }
                tail = prev;
            }
        }
        (*rettv).vval.v_string = xstrnsave(head, tail.offset_from(head) as usize);
    }
    #[cfg(test)]
    {
        let _ = (argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmp_keyvalue_value(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let kv1 = a.cast::<keyvalue_T>();
        let kv2 = b.cast::<keyvalue_T>();
        strcmp((*kv1).value, (*kv2).value)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmp_keyvalue_value_n(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let kv1 = a.cast::<keyvalue_T>();
        let kv2 = b.cast::<keyvalue_T>();
        strncmp((*kv1).value, (*kv2).value, (*kv1).length.max((*kv2).length))
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmp_keyvalue_value_i(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let kv1 = a.cast::<keyvalue_T>();
        let kv2 = b.cast::<keyvalue_T>();
        vim_stricmp((*kv1).value, (*kv2).value)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmp_keyvalue_value_ni(a: *const c_void, b: *const c_void) -> c_int {
    unsafe {
        let kv1 = a.cast::<keyvalue_T>();
        let kv2 = b.cast::<keyvalue_T>();
        vim_strnicmp((*kv1).value, (*kv2).value, (*kv1).length.max((*kv2).length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_strnsave_unquoted_and_strrep_and_format_typeof_work() {
        unsafe {
            let unquoted = vim_strnsave_unquoted(c"\"/bin/sh\"".as_ptr(), 9);
            assert_eq!(CStr::from_ptr(unquoted).to_str().unwrap(), "/bin/sh");
            xfree(unquoted.cast());

            let rep = strrep(c"foo bar foo".as_ptr(), c"foo".as_ptr(), c"baz".as_ptr());
            assert_eq!(CStr::from_ptr(rep).to_str().unwrap(), "baz bar baz");
            xfree(rep.cast());

            let cat = concat_str(c"hello ".as_ptr(), c"world".as_ptr());
            assert_eq!(CStr::from_ptr(cat).to_str().unwrap(), "hello world");
            xfree(cat.cast());

            assert_eq!(vim_strnicmp_asc(c"AbC".as_ptr(), c"aBc".as_ptr(), 3), 0);
            assert!(!has_non_ascii(c"ascii".as_ptr()));
            assert_eq!(format_typeof(c"lld".as_ptr()), TYPE_LONGLONGINT);
            assert_eq!(format_typeof(c"zu".as_ptr()), TYPE_SIZET);

            assert_eq!(strlen(c"hello".as_ptr()), 5);
            assert_eq!(strcmp(c"abc".as_ptr(), c"abc".as_ptr()), 0);
            assert!(strcmp(c"abc".as_ptr(), c"abd".as_ptr()) < 0);
            assert_eq!(strncmp(c"abcdef".as_ptr(), c"abcxyz".as_ptr(), 3), 0);
            assert_eq!(atoi(c"  -1234px".as_ptr()), -1234);
            assert_eq!(atol(c" +56789".as_ptr()), 56789);
            assert!(!strchr(c"hello".as_ptr(), b'e' as c_int).is_null());
            assert!(!strstr(c"hello world".as_ptr(), c"world".as_ptr()).is_null());
            assert!(!strpbrk(c"hello".as_ptr(), c"xyzl".as_ptr()).is_null());
            assert_eq!(strcspn(c"foo,bar".as_ptr(), c",".as_ptr()), 3);
            assert_eq!(strtol(c"ff00aa".as_ptr(), ptr::null_mut(), 16), 0xff00aa);
            let mut end: *mut c_char = ptr::null_mut();
            let s = c"9.1.0123";
            assert_eq!(strtoul(s.as_ptr(), &raw mut end, 10), 9);
            assert_eq!(*end as u8, b'.');
            let mut tok_buf = *b"en_US\nde_DE\n\0";
            let mut saveptr: *mut c_char = ptr::null_mut();
            let t1 = strtok_r(
                tok_buf.as_mut_ptr().cast(),
                c"\n".as_ptr(),
                &raw mut saveptr,
            );
            assert_eq!(CStr::from_ptr(t1).to_bytes(), b"en_US");
            let t2 = strtok_r(ptr::null_mut(), c"\n".as_ptr(), &raw mut saveptr);
            assert_eq!(CStr::from_ptr(t2).to_bytes(), b"de_DE");
            assert!(strtok_r(ptr::null_mut(), c"\n".as_ptr(), &raw mut saveptr).is_null());
        }
    }

    #[test]
    fn vim_snprintf_macros_format_correctly() {
        unsafe {
            let mut buf = [0 as c_char; 64];
            let n = vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"%s %d %x %06b".as_ptr(),
                c"val",
                -42,
                0xdeadbeef_u32,
                12_u64,
            );
            assert_eq!(n, 23);
            assert_eq!(
                CStr::from_ptr(buf.as_ptr()).to_str().unwrap(),
                "val -42 deadbeef 001100"
            );

            let added = vim_snprintf_add!(
                buf.as_mut_ptr(),
                buf.len(),
                c" %2$s %1$d".as_ptr(),
                7,
                c"ok"
            );
            assert_eq!(added, 5);
            assert_eq!(
                CStr::from_ptr(buf.as_ptr()).to_str().unwrap(),
                "val -42 deadbeef 001100 ok 7"
            );

            let mut small = [0 as c_char; 5];
            let safelen =
                vim_snprintf_safelen!(small.as_mut_ptr(), small.len(), c"hello world".as_ptr(),);
            assert_eq!(safelen, 4);
            assert_eq!(CStr::from_ptr(small.as_ptr()).to_str().unwrap(), "hell");
        }
    }
}
