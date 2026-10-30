//! String manipulation functions and Vimscript string builtins (`src/nvim/strings.c`).

use crate::charset::{
    STR2NR_BIN, STR2NR_FORCE, STR2NR_HEX, STR2NR_OCT, STR2NR_OOCT, STR2NR_QUOTE, ascii_isdigit,
    ascii_iswhite, rem_backslash, skipwhite, transstr, vim_str2nr,
};
use crate::errors::*;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::typval::{
    EvalFuncData, TvFloat, TypVal, VarType, Varnumber, tv_check_for_number_arg,
    tv_check_for_opt_bool_arg, tv_check_for_opt_number_arg, tv_check_for_opt_string_arg,
    tv_check_for_string_arg, tv_get_bool, tv_get_bool_chk, tv_get_number, tv_get_number_chk,
    tv_get_string, tv_get_string_buf_chk, tv_get_string_chk, tv_list_alloc_ret,
    tv_list_append_number,
};
use crate::ex_docmd::find_cmdline_var;
use crate::math::{xisinf, xisnan};
use crate::mbyte::{
    mb_copy_char, mb_cptr2char_adv, mb_ptr2char_adv, mb_string2cells, mb_tolower, mb_toupper,
    utf_char2bytes, utf_head_off, utf_ptr2_str_char_info, utf_ptr2cells, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
use crate::memory::xstrlcpy;
use crate::message::{emsg, semsg};
use crate::option::{csh_like_shell, fish_like_shell};
#[cfg(windows)]
use crate::option_vars::*;
use crate::plines::linetabsize_col;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

const CTRL_V: c_char = 0x16;

pub const TYPE_UNKNOWN: i32 = -1;
pub const TYPE_INT: i32 = 0;
pub const TYPE_LONGINT: i32 = 1;
pub const TYPE_LONGLONGINT: i32 = 2;
pub const TYPE_SIGNEDSIZET: i32 = 3;
pub const TYPE_UNSIGNEDINT: i32 = 4;
pub const TYPE_UNSIGNEDLONGINT: i32 = 5;
pub const TYPE_UNSIGNEDLONGLONGINT: i32 = 6;
pub const TYPE_SIZET: i32 = 7;
pub const TYPE_POINTER: i32 = 8;
pub const TYPE_PERCENT: i32 = 9;
pub const TYPE_CHAR: i32 = 10;
pub const TYPE_STRING: i32 = 11;
pub const TYPE_FLOAT: i32 = 12;

pub const MAX_ALLOWED_STRING_WIDTH: u32 = 1_048_576;

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

unsafe extern "C" {
    pub fn strcoll(s1: *const c_char, s2: *const c_char) -> i32;
    pub fn strerror(errnum: i32) -> *mut c_char;
}

pub trait IntoCCharPtr {
    fn into_c_char_ptr(self) -> *const c_char;
}

impl IntoCCharPtr for *const c_char {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self
    }
}

impl IntoCCharPtr for *mut c_char {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self
    }
}

impl IntoCCharPtr for &CStr {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self.as_ptr()
    }
}

impl IntoCCharPtr for &&CStr {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self.as_ptr()
    }
}

impl IntoCCharPtr for &std::ffi::CString {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self.as_ptr()
    }
}

impl IntoCCharPtr for Option<&CStr> {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self.map_or(ptr::null(), |s| s.as_ptr())
    }
}

impl IntoCCharPtr for Option<&std::ffi::CString> {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self.map_or(ptr::null(), |s| s.as_ptr())
    }
}

impl IntoCCharPtr for &c_char {
    #[inline]
    fn into_c_char_ptr(self) -> *const c_char {
        self
    }
}

#[inline]
pub fn tr<S: IntoCCharPtr>(s: S) -> *mut c_char {
    gettext(s.into_c_char_ptr())
}

#[inline]
pub fn gettext<S: IntoCCharPtr>(msgid: S) -> *mut c_char {
    let msgid = msgid.into_c_char_ptr();
    if crate::os::lang::HAVE_LIBINTL
        && let Some(tr) = crate::os::lang::c_gettext(msgid)
    {
        return tr.as_ptr().cast_mut();
    }
    msgid.cast_mut()
}

pub trait IntoNgettextCount {
    fn into_ngettext_count(self) -> u64;
}

impl IntoNgettextCount for u32 {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        u64::from(self)
    }
}

impl IntoNgettextCount for u64 {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        self
    }
}

impl IntoNgettextCount for usize {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        self as u64
    }
}

impl IntoNgettextCount for i32 {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        self as u64
    }
}

impl IntoNgettextCount for i64 {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        self as u64
    }
}

impl IntoNgettextCount for isize {
    #[inline]
    fn into_ngettext_count(self) -> u64 {
        self as u64
    }
}

#[inline]
pub fn ngettext<S1: IntoCCharPtr, S2: IntoCCharPtr, N: IntoNgettextCount>(
    msgid1: S1,
    msgid2: S2,
    n: N,
) -> *mut c_char {
    let msgid1 = msgid1.into_c_char_ptr();
    let msgid2 = msgid2.into_c_char_ptr();
    let count = n.into_ngettext_count();
    if crate::os::lang::HAVE_LIBINTL
        && let Some(tr) = crate::os::lang::c_ngettext(msgid1, count)
    {
        return tr.as_ptr().cast_mut();
    }
    if count == 1 {
        msgid1.cast_mut()
    } else {
        msgid2.cast_mut()
    }
}

pub fn strtod(nptr: impl crate::strings::IntoCCharPtr, endptr: *mut *mut c_char) -> f64 {
    let nptr = nptr.into_c_char_ptr();
    unsafe {
        let mut s = nptr;
        while matches!(*s as u8, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            s = s.wrapping_add(1);
        }
        let neg = if crate::memory::cstr_first_byte(s) == b'-' {
            s = s.wrapping_add(1);
            true
        } else {
            if crate::memory::cstr_first_byte(s) == b'+' {
                s = s.wrapping_add(1);
            }
            false
        };

        let starts_with_ci = |p: *const c_char, lit: &[u8]| -> bool {
            for (i, &b) in lit.iter().enumerate() {
                let c = crate::memory::cstr_byte_at(p, i);
                if c.to_ascii_lowercase() != b {
                    return false;
                }
            }
            true
        };

        if starts_with_ci(s, b"inf") {
            s = s.wrapping_add(3);
            if starts_with_ci(s, b"inity") {
                s = s.wrapping_add(5);
            }
            if !endptr.is_null() {
                *endptr = s.cast_mut();
            }
            return if neg { -f64::INFINITY } else { f64::INFINITY };
        }

        if starts_with_ci(s, b"nan") {
            s = s.wrapping_add(3);
            if crate::memory::cstr_first_byte(s) == b'(' {
                let mut p = s.wrapping_add(1);
                while (*p as u8).is_ascii_alphanumeric()
                    || crate::memory::cstr_first_byte(p) == b'_'
                {
                    p = p.wrapping_add(1);
                }
                if crate::memory::cstr_first_byte(p) == b')' {
                    s = p.wrapping_add(1);
                }
            }
            if !endptr.is_null() {
                *endptr = s.cast_mut();
            }
            return if neg { -f64::NAN } else { f64::NAN };
        }

        if crate::memory::cstr_first_byte(s) == b'0'
            && matches!(crate::memory::cstr_byte_at(s, 1), b'x' | b'X')
        {
            let mut p = s.wrapping_add(2);
            let mut mant: f64 = 0.0;
            let mut exp2: i32 = 0;
            let mut any_hex = false;
            while let Some(d) = (*p as u8 as char).to_digit(16) {
                any_hex = true;
                mant = mant * 16.0 + f64::from(d);
                p = p.wrapping_add(1);
            }
            if crate::memory::cstr_first_byte(p) == b'.' {
                p = p.wrapping_add(1);
                while let Some(d) = (*p as u8 as char).to_digit(16) {
                    any_hex = true;
                    mant = mant * 16.0 + f64::from(d);
                    exp2 = exp2.saturating_sub(4);
                    p = p.wrapping_add(1);
                }
            }
            if any_hex {
                if matches!(*p as u8, b'p' | b'P') {
                    let mut ep = p.wrapping_add(1);
                    let exp_neg = if crate::memory::cstr_first_byte(ep) == b'-' {
                        ep = ep.wrapping_add(1);
                        true
                    } else {
                        if crate::memory::cstr_first_byte(ep) == b'+' {
                            ep = ep.wrapping_add(1);
                        }
                        false
                    };
                    if (*ep as u8).is_ascii_digit() {
                        let mut p_exp: i32 = 0;
                        while (*ep as u8).is_ascii_digit() {
                            p_exp = p_exp
                                .saturating_mul(10)
                                .saturating_add(i32::from((*ep as u8) - b'0'));
                            ep = ep.wrapping_add(1);
                        }
                        exp2 = if exp_neg {
                            exp2.saturating_sub(p_exp)
                        } else {
                            exp2.saturating_add(p_exp)
                        };
                        p = ep;
                    }
                }
                if !endptr.is_null() {
                    *endptr = p.cast_mut();
                }
                let val = mant * 2.0f64.powi(exp2);
                return if neg { -val } else { val };
            }
        }

        let num_start = s;
        let mut digits = 0usize;
        while (*s as u8).is_ascii_digit() {
            s = s.wrapping_add(1);
            digits += 1;
        }
        if crate::memory::cstr_first_byte(s) == b'.' {
            s = s.wrapping_add(1);
            while (*s as u8).is_ascii_digit() {
                s = s.wrapping_add(1);
                digits += 1;
            }
        }
        if digits == 0 {
            if !endptr.is_null() {
                *endptr = nptr.cast_mut();
            }
            return 0.0;
        }
        if matches!(*s as u8, b'e' | b'E') {
            let mut ep = s.wrapping_add(1);
            if matches!(*ep as u8, b'+' | b'-') {
                ep = ep.wrapping_add(1);
            }
            if (*ep as u8).is_ascii_digit() {
                while (*ep as u8).is_ascii_digit() {
                    ep = ep.wrapping_add(1);
                }
                s = ep;
            }
        }
        if !endptr.is_null() {
            *endptr = s.cast_mut();
        }
        let len = (s as usize) - (num_start as usize);
        let slice = std::slice::from_raw_parts(num_start.cast::<u8>(), len);
        let num_str = std::str::from_utf8_unchecked(slice);
        let val = num_str.parse::<f64>().unwrap_or(0.0);
        if neg { -val } else { val }
    }
}

#[inline]
fn write_fmt_to_c_buf(dst: &mut [c_char], args: std::fmt::Arguments<'_>) -> usize {
    use std::io::Write;
    if dst.is_empty() {
        return 0;
    }
    let max_bytes = dst.len() - 1;
    let buf: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(dst.as_mut_ptr().cast::<u8>(), max_bytes) };
    let mut cursor = std::io::Cursor::new(buf);
    let _ = cursor.write_fmt(args);
    let written = cursor.position() as usize;
    dst[written] = NUL;
    written
}

pub fn vim_strsave_escaped<S1: IntoCCharPtr, S2: IntoCCharPtr>(
    string: S1,
    esc_chars: S2,
) -> std::ffi::CString {
    vim_strsave_escaped_ext(string, esc_chars, b'\\', false)
}

pub fn vim_strsave_escaped_ext<S1: IntoCCharPtr, S2: IntoCCharPtr>(
    string: S1,
    esc_chars: S2,
    cc: u8,
    bsl: bool,
) -> std::ffi::CString {
    let string = string.into_c_char_ptr();
    let esc_chars = esc_chars.into_c_char_ptr();
    let mut length: usize = 0;
    let mut p = string;
    while !crate::memory::cstr_is_empty(p) {
        let l = utfc_ptr2len(p) as usize;
        if l > 1 {
            length += l;
            p = p.wrapping_add(l);
            continue;
        }
        if !vim_strchr(esc_chars, (unsafe { *p } as u8) as i32).is_null()
            || (bsl && rem_backslash(p))
        {
            length += 1;
        }
        length += 1;
        p = p.wrapping_add(1);
    }

    let mut escaped = Vec::with_capacity(length);
    p = string;
    while !crate::memory::cstr_is_empty(p) {
        let l = utfc_ptr2len(p) as usize;
        if l > 1 {
            escaped.extend_from_slice(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), l) });
            p = p.wrapping_add(l);
            continue;
        }
        if !vim_strchr(esc_chars, (unsafe { *p } as u8) as i32).is_null()
            || (bsl && rem_backslash(p))
        {
            escaped.push(cc);
        }
        escaped.push(unsafe { *p } as u8);
        p = p.wrapping_add(1);
    }

    crate::memory::cstr_from_bytes(&escaped)
}

pub fn vim_strnsave_unquoted(bytes: &[u8]) -> std::ffi::CString {
    let mut ret = Vec::with_capacity(bytes.len());
    let mut inquote = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            inquote = !inquote;
        } else if bytes[i] == b'\\'
            && inquote
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b'\\' || bytes[i + 1] == b'"')
        {
            i += 1;
            ret.push(bytes[i]);
        } else {
            ret.push(bytes[i]);
        }
        i += 1;
    }

    crate::memory::cstr_from_bytes(&ret)
}

pub fn vim_strsave_shellescape<S: IntoCCharPtr>(
    string: S,
    do_special: bool,
    do_newline: bool,
) -> std::ffi::CString {
    unsafe {
        let string = string.into_c_char_ptr();
        let csh_like = csh_like_shell() != 0;
        let fish_like = fish_like_shell();
        #[cfg(windows)]
        let use_double_quote = opt_vars().p_ssl == 0;
        #[cfg(not(windows))]
        let use_double_quote = false;

        let mut length = crate::memory::cstr_len(string) + 3;
        let mut p = string;
        while !crate::memory::cstr_is_empty(p) {
            if use_double_quote {
                if crate::memory::cstr_first_byte(p) == b'"' {
                    length += 1;
                }
            } else if crate::memory::cstr_first_byte(p) == b'\'' {
                length += 3;
            }
            if (crate::memory::cstr_first_byte(p) == b'\n' && (csh_like || do_newline))
                || (crate::memory::cstr_first_byte(p) == b'!' && (csh_like || do_special))
            {
                length += 1;
                if csh_like && do_special {
                    length += 1;
                }
            }
            let mut l: usize = 0;
            if do_special && find_cmdline_var(p, &mut l) >= 0 {
                length += 1;
                p = p.wrapping_add(l - 1);
            }
            if crate::memory::cstr_first_byte(p) == b'\\' && fish_like {
                length += 1;
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
        }

        let mut escaped_vec = vec![0 as c_char; length];
        let escaped_string = escaped_vec.as_mut_ptr();
        let mut d = escaped_string;

        *d = if use_double_quote {
            b'"' as c_char
        } else {
            b'\'' as c_char
        };
        d = d.wrapping_add(1);

        p = string;
        while !crate::memory::cstr_is_empty(p) {
            if use_double_quote {
                if crate::memory::cstr_first_byte(p) == b'"' {
                    *d = b'"' as c_char;
                    *d.wrapping_add(1) = b'"' as c_char;
                    d = d.wrapping_add(2);
                    p = p.wrapping_add(1);
                    continue;
                }
            } else if crate::memory::cstr_first_byte(p) == b'\'' {
                *d = b'\'' as c_char;
                *d.wrapping_add(1) = b'\\' as c_char;
                *d.wrapping_add(2) = b'\'' as c_char;
                *d.wrapping_add(3) = b'\'' as c_char;
                d = d.wrapping_add(4);
                p = p.wrapping_add(1);
                continue;
            }
            if (crate::memory::cstr_first_byte(p) == b'\n' && (csh_like || do_newline))
                || (crate::memory::cstr_first_byte(p) == b'!' && (csh_like || do_special))
            {
                *d = b'\\' as c_char;
                d = d.wrapping_add(1);
                if csh_like && do_special {
                    *d = b'\\' as c_char;
                    d = d.wrapping_add(1);
                }
                *d = *p;
                d = d.wrapping_add(1);
                p = p.wrapping_add(1);
                continue;
            }
            let mut l: usize = 0;
            if do_special && find_cmdline_var(p, &mut l) >= 0 {
                *d = b'\\' as c_char;
                d = d.wrapping_add(1);
                ptr::copy_nonoverlapping(p, d, l);
                d = d.wrapping_add(l);
                p = p.wrapping_add(l);
                continue;
            }
            if crate::memory::cstr_first_byte(p) == b'\\' && fish_like {
                *d = b'\\' as c_char;
                *d.wrapping_add(1) = *p;
                d = d.wrapping_add(2);
                p = p.wrapping_add(1);
                continue;
            }

            mb_copy_char(&raw mut p, &raw mut d);
        }

        *d = if use_double_quote {
            b'"' as c_char
        } else {
            b'\'' as c_char
        };
        *d.wrapping_add(1) = NUL;
        let out_len = crate::memory::ptr_offset_from(d, escaped_string) as usize + 1;
        crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
            escaped_string.cast::<u8>(),
            out_len,
        ))
    }
}

#[cfg(windows)]
pub fn mb_strup_buf(src: *const c_char, dst: *mut c_char) -> usize {
    let mut i: usize = 0;
    let mut p = src;
    while !crate::memory::cstr_is_empty(p) {
        let ci = utf_ptr2_str_char_info(p).chr;
        let c = if ci.value < 0 {
            (unsafe { *p } as u8) as i32
        } else {
            ci.value
        };
        i += utf_char2bytes(mb_toupper(c), dst.wrapping_add(i)) as usize;
        p = p.wrapping_add(ci.len as usize);
    }
    unsafe {
        *dst.wrapping_add(i) = NUL;
    }
    i
}

pub fn strcase_save<S: IntoCCharPtr>(orig: S, upper: bool) -> std::ffi::CString {
    let mut p = orig.into_c_char_ptr();
    let orig_len = crate::memory::cstr_len(p);
    let mut res = Vec::<c_char>::with_capacity(orig_len + 1);

    while !crate::memory::cstr_is_empty(p) {
        let char_info = utf_ptr2_str_char_info(p).chr;
        let c = if char_info.value < 0 {
            (unsafe { *p } as u8) as i32
        } else {
            char_info.value
        };
        let newc = if upper { mb_toupper(c) } else { mb_tolower(c) };
        let mut mb = [0 as c_char; MB_MAXBYTES];
        let newl = utf_char2bytes(newc, mb.as_mut_ptr()) as usize;
        res.extend_from_slice(&mb[..newl]);
        p = p.wrapping_add(char_info.len as usize);
    }

    crate::memory::cstr_from_bytes(unsafe {
        std::slice::from_raw_parts(res.as_ptr().cast::<u8>(), res.len())
    })
}

pub fn del_trailing_spaces(ptr: *mut c_char) {
    let len = crate::memory::cstr_len(ptr);
    let mut q = ptr.wrapping_add(len);
    while q > ptr {
        q = q.wrapping_sub(1);
        if q <= ptr {
            break;
        }
        if ascii_iswhite(unsafe { *q })
            && unsafe { *q.wrapping_sub(1) } != b'\\' as c_char
            && unsafe { *q.wrapping_sub(1) } != CTRL_V
        {
            unsafe {
                *q = NUL;
            }
        } else {
            break;
        }
    }
}

#[inline]
pub fn vim_haschr<S: IntoCCharPtr>(string: S, c: i32) -> bool {
    !vim_strchr(string, c).is_null()
}

pub fn vim_strchr<S: IntoCCharPtr>(string: S, c: i32) -> *mut c_char {
    let string = string.into_c_char_ptr();
    {
        if string.is_null() || c <= 0 {
            ptr::null_mut()
        } else if c < 0x80 {
            let mut p = string;
            while !crate::memory::cstr_is_empty(p) {
                if (unsafe { *p } as u8) as i32 == c {
                    return p.cast_mut();
                }
                p = p.wrapping_add(1);
            }
            ptr::null_mut()
        } else {
            let mut u8char = [0 as c_char; MB_MAXBYTES + 1];
            let len = utf_char2bytes(c, u8char.as_mut_ptr()) as usize;
            let needle = unsafe { std::slice::from_raw_parts(u8char.as_ptr().cast::<u8>(), len) };
            let hay = crate::memory::cstr_as_bytes(string);
            if let Some(pos) = hay.windows(len).position(|w| w == needle) {
                string.wrapping_add(pos).cast_mut()
            } else {
                ptr::null_mut()
            }
        }
    }
}

pub fn sort_strings(files: &mut [std::ffi::CString]) {
    if files.len() > 1 {
        files.sort_unstable();
    }
}

pub fn concat_str<S1: IntoCCharPtr, S2: IntoCCharPtr>(str1: S1, str2: S2) -> std::ffi::CString {
    let b1 = crate::memory::cstr_as_bytes(str1.into_c_char_ptr());
    let b2 = crate::memory::cstr_as_bytes(str2.into_c_char_ptr());
    let mut dest = Vec::with_capacity(b1.len() + b2.len());
    dest.extend_from_slice(b1);
    dest.extend_from_slice(b2);
    crate::memory::cstr_from_bytes(&dest)
}

pub fn tv_nr(tvs: *mut TypVal, idxp: &mut i32) -> Varnumber {
    let idx = (*idxp - 1) as usize;
    let mut n: Varnumber = 0;
    if unsafe { (*tvs.wrapping_add(idx)).v_type } == VarType::VarUnknown {
        emsg(gettext(E_PRINTF));
    } else {
        *idxp += 1;
        let mut err = false;
        n = tv_get_number_chk(unsafe { &*tvs.wrapping_add(idx) }, Some(&mut err));
        if err {
            n = 0;
        }
    }
    n
}

pub fn tv_str(
    tvs: *mut TypVal,
    idxp: &mut i32,
    tofree: &mut Option<std::ffi::CString>,
) -> *const c_char {
    unsafe {
        let idx = (*idxp - 1) as usize;
        let mut s: *const c_char = ptr::null();
        if (*tvs.wrapping_add(idx)).v_type == VarType::VarUnknown {
            emsg(gettext(E_PRINTF));
        } else {
            *idxp += 1;
            let tv = tvs.wrapping_add(idx);
            if (*tv).v_type == VarType::VarString || (*tv).v_type == VarType::VarNumber {
                s = tv_get_string_chk(&*tv);
                *tofree = None;
            } else {
                let encoded = encode_tv2echo(&mut *tv, None);
                s = encoded.as_ptr();
                *tofree = Some(encoded);
            }
        }
        s
    }
}

pub fn tv_ptr(tvs: *const TypVal, idxp: &mut i32) -> *const c_void {
    unsafe {
        let idx = (*idxp - 1) as usize;
        if (*tvs.wrapping_add(idx)).v_type == VarType::VarUnknown {
            emsg(gettext(E_PRINTF));
            return ptr::null();
        }
        *idxp += 1;
        (*tvs.wrapping_add(idx)).vval.v_any_ptr()
    }
}

pub fn tv_float(tvs: *mut TypVal, idxp: &mut i32) -> TvFloat {
    let idx = (*idxp - 1) as usize;
    let mut f: TvFloat = 0.0;
    if unsafe { (*tvs.wrapping_add(idx)).v_type } == VarType::VarUnknown {
        emsg(gettext(E_PRINTF));
    } else {
        *idxp += 1;
        let tv = tvs.wrapping_add(idx);
        let tv = unsafe { &mut *tv };
        if tv.v_type == VarType::VarFloat {
            f = tv.vval.v_float();
        } else if tv.v_type == VarType::VarNumber {
            f = tv.vval.v_number() as TvFloat;
        } else {
            emsg(gettext(c"E807: Expected Float argument for printf()"));
        }
    }
    f
}

pub fn infinity_str(
    positive: bool,
    fmt_spec: c_char,
    force_sign: i32,
    space_for_positive: i32,
) -> &'static CStr {
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
    TABLE[idx]
}

pub fn format_typeof(type_: impl crate::strings::IntoCCharPtr) -> i32 {
    let mut type_ = type_.into_c_char_ptr();
    {
        let mut length_modifier: u8 = 0;
        let first = unsafe { *type_ } as u8;
        if matches!(first, b'h' | b'l' | b'z') {
            length_modifier = first;
            type_ = type_.wrapping_add(1);
            if length_modifier == b'l' && (unsafe { *type_ } as u8) == b'l' {
                length_modifier = b'L';
                type_ = type_.wrapping_add(1);
            }
        }
        let mut fmt_spec = unsafe { *type_ } as u8;
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

fn format_typename(type_: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let type_ = type_.into_c_char_ptr();
    match format_typeof(type_) {
        TYPE_INT => c"int".as_ptr(),
        TYPE_LONGINT => c"long int".as_ptr(),
        TYPE_LONGLONGINT => c"long long int".as_ptr(),
        TYPE_UNSIGNEDINT => c"unsigned int".as_ptr(),
        TYPE_SIGNEDSIZET => c"signed size_t".as_ptr(),
        TYPE_UNSIGNEDLONGINT => c"unsigned long int".as_ptr(),
        TYPE_UNSIGNEDLONGLONGINT => c"unsigned long long int".as_ptr(),
        TYPE_SIZET => c"size_t".as_ptr(),
        TYPE_POINTER => gettext(c"pointer"),
        TYPE_PERCENT => gettext(c"percent"),
        TYPE_CHAR => c"char".as_ptr(),
        TYPE_STRING => gettext(c"string"),
        TYPE_FLOAT => c"float".as_ptr(),
        _ => gettext(c"unknown"),
    }
}

fn adjust_types(
    ap_types: &mut Vec<*const c_char>,
    arg: i32,
    type_: impl crate::strings::IntoCCharPtr,
) -> bool {
    let type_ = type_.into_c_char_ptr();
    {
        if arg <= 0 {
            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), type_);
            return false;
        }

        if (arg as usize) > ap_types.len() {
            ap_types.resize(arg as usize, ptr::null());
        }

        let prev = ap_types[(arg - 1) as usize];
        if !prev.is_null() {
            if crate::memory::cstr_first_byte(prev) == b'*'
                || crate::memory::cstr_first_byte(type_) == b'*'
            {
                let mut pt = type_;
                if crate::memory::cstr_first_byte(pt) == b'*' {
                    pt = prev;
                }
                if crate::memory::cstr_first_byte(pt) != b'*'
                    && !matches!(unsafe { *pt } as u8, b'd' | b'i')
                {
                    semsg!(
                        gettext(E_POSITIONAL_FIELD_SPEC_REUSED),
                        arg,
                        format_typename(prev),
                        format_typename(type_),
                    );
                    return false;
                }
            } else if format_typeof(type_) != format_typeof(prev) {
                semsg!(
                    gettext(E_POSITIONAL_TYPE_INCONSISTENT),
                    arg,
                    format_typename(type_),
                    format_typename(prev),
                );
                return false;
            }
        }

        ap_types[(arg - 1) as usize] = type_;
        true
    }
}

pub fn format_overflow_error(pstart: impl crate::strings::IntoCCharPtr) {
    let pstart = pstart.into_c_char_ptr();
    {
        let mut p = pstart;
        while ascii_isdigit(unsafe { *p }) {
            p = p.wrapping_add(1);
        }
        semsg!(
            gettext(E_VAL_TOO_LARGE_LEN),
            crate::memory::ptr_offset_from(p, pstart) as i32,
            pstart,
        );
    }
}

pub fn get_unsigned_int(
    pstart: impl crate::strings::IntoCCharPtr,
    p: *mut *const c_char,
    uj: &mut u32,
    overflow_err: bool,
) -> bool {
    let pstart = pstart.into_c_char_ptr();
    unsafe {
        *uj = ((**p as u8) - b'0') as u32;
        *p = (*p).wrapping_add(1);

        while ascii_isdigit(**p) && *uj < MAX_ALLOWED_STRING_WIDTH {
            *uj = 10 * (*uj) + (((**p as u8) - b'0') as u32);
            *p = (*p).wrapping_add(1);
        }

        if *uj > MAX_ALLOWED_STRING_WIDTH {
            if overflow_err {
                format_overflow_error(pstart);
                return false;
            } else {
                *uj = MAX_ALLOWED_STRING_WIDTH;
            }
        }
        true
    }
}

pub fn parse_fmt_types(
    num_posarg: &mut i32,
    fmt: impl crate::strings::IntoCCharPtr,
    tvs: *mut TypVal,
) -> bool {
    let fmt = fmt.into_c_char_ptr();
    {
        if fmt.is_null() {
            return true;
        }
        let mut ap_types: Vec<*const c_char> = Vec::new();
        let mut p = fmt;
        let mut any_pos = false;
        let mut any_arg = false;

        macro_rules! check_pos_arg {
            () => {
                if any_pos && any_arg {
                    semsg!(gettext(E_CANNOT_MIX_POSITIONAL), fmt);
                    *num_posarg = 0;
                    return false;
                }
            };
        }

        macro_rules! fail {
            () => {
                *num_posarg = 0;
                return false;
            };
        }

        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) != b'%' {
                p = p.wrapping_add(1);
                while !crate::memory::cstr_is_empty(p) && unsafe { *p } != b'%' as c_char {
                    p = p.wrapping_add(1);
                }
            } else {
                let mut pos_arg: i32 = -1;
                let pstart = p.wrapping_add(1);
                p = p.wrapping_add(1);

                let mut ptype = p;
                while ascii_isdigit(unsafe { *ptype }) {
                    ptype = ptype.wrapping_add(1);
                }

                if crate::memory::cstr_first_byte(ptype) == b'$' {
                    if crate::memory::cstr_first_byte(p) == b'0' {
                        semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), fmt);
                        fail!();
                    }
                    let mut uj: u32 = 0;
                    if !get_unsigned_int(pstart, &raw mut p, &mut uj, !tvs.is_null()) {
                        fail!();
                    }
                    pos_arg = uj as i32;
                    any_pos = true;
                    check_pos_arg!();
                    p = p.wrapping_add(1);
                }

                while matches!(
                    unsafe { *p } as u8,
                    b'0' | b'-' | b'+' | b' ' | b'#' | b'\''
                ) {
                    p = p.wrapping_add(1);
                }

                let arg = p;
                if crate::memory::cstr_first_byte(arg) == b'*' {
                    p = p.wrapping_add(1);
                    if ascii_isdigit(unsafe { *p }) {
                        let mut uj: u32 = 0;
                        if !get_unsigned_int(
                            arg.wrapping_add(1),
                            &raw mut p,
                            &mut uj,
                            !tvs.is_null(),
                        ) {
                            fail!();
                        }
                        if crate::memory::cstr_first_byte(p) != b'$' {
                            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), fmt);
                            fail!();
                        }
                        p = p.wrapping_add(1);
                        any_pos = true;
                        check_pos_arg!();
                        if !adjust_types(&mut ap_types, uj as i32, arg) {
                            fail!();
                        }
                    } else {
                        any_arg = true;
                        check_pos_arg!();
                    }
                } else if ascii_isdigit(unsafe { *p }) {
                    let digstart = p;
                    let mut uj: u32 = 0;
                    if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                        fail!();
                    }
                    if crate::memory::cstr_first_byte(p) == b'$' {
                        semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), fmt);
                        fail!();
                    }
                }

                if crate::memory::cstr_first_byte(p) == b'.' {
                    p = p.wrapping_add(1);
                    let arg = p;
                    if crate::memory::cstr_first_byte(arg) == b'*' {
                        p = p.wrapping_add(1);
                        if ascii_isdigit(unsafe { *p }) {
                            let mut uj: u32 = 0;
                            if !get_unsigned_int(
                                arg.wrapping_add(1),
                                &raw mut p,
                                &mut uj,
                                !tvs.is_null(),
                            ) {
                                fail!();
                            }
                            if crate::memory::cstr_first_byte(p) == b'$' {
                                any_pos = true;
                                check_pos_arg!();
                                p = p.wrapping_add(1);
                                if !adjust_types(&mut ap_types, uj as i32, arg) {
                                    fail!();
                                }
                            } else {
                                semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), fmt);
                                fail!();
                            }
                        } else {
                            any_arg = true;
                            check_pos_arg!();
                        }
                    } else if ascii_isdigit(unsafe { *p }) {
                        let digstart = p;
                        let mut uj: u32 = 0;
                        if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                            fail!();
                        }
                        if crate::memory::cstr_first_byte(p) == b'$' {
                            semsg!(gettext(E_INVALID_FORMAT_SPECIFIER), fmt);
                            fail!();
                        }
                    }
                }

                if pos_arg != -1 {
                    any_pos = true;
                    check_pos_arg!();
                    ptype = p;
                }

                if matches!(unsafe { *p } as u8, b'h' | b'l' | b'z') {
                    let lm = unsafe { *p } as u8;
                    p = p.wrapping_add(1);
                    if lm == b'l' && (unsafe { *p } as u8) == b'l' {
                        p = p.wrapping_add(1);
                    }
                }

                match unsafe { *p } as u8 {
                    b'i' | b'*' | b'd' | b'u' | b'o' | b'D' | b'U' | b'O' | b'x' | b'X' | b'b'
                    | b'B' | b'c' | b's' | b'S' | b'p' | b'f' | b'F' | b'e' | b'E' | b'g'
                    | b'G' => {
                        if pos_arg != -1 {
                            if !adjust_types(&mut ap_types, pos_arg, ptype) {
                                fail!();
                            }
                        } else {
                            any_arg = true;
                            check_pos_arg!();
                        }
                    }
                    _ => {
                        if pos_arg != -1 {
                            semsg!(gettext(E_CANNOT_MIX_POSITIONAL), fmt);
                            fail!();
                        }
                    }
                }

                if !crate::memory::cstr_is_empty(p) {
                    p = p.wrapping_add(1);
                }
            }
        }

        *num_posarg = ap_types.len() as i32;
        for (arg_idx, &ap_type) in ap_types.iter().enumerate() {
            if ap_type.is_null() {
                semsg!(gettext(E_FMT_ARG_NR_UNUSED), (arg_idx + 1) as i32, fmt,);
                fail!();
            }
            if !tvs.is_null()
                && unsafe { (*tvs.wrapping_add(arg_idx)).v_type } == VarType::VarUnknown
            {
                semsg!(
                    gettext(E_POSITIONAL_OUT_OF_BOUNDS),
                    (arg_idx + 1) as i32,
                    fmt,
                );
                fail!();
            }
        }

        true
    }
}

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

impl IntoFmtVal for Option<&CStr> {
    #[inline]
    fn into_fmt_val(self) -> FmtVal {
        FmtVal::Ptr(self.map_or(ptr::null(), |s| s.as_ptr().cast()))
    }
}

#[inline]
fn fmt_arg_val(args: &[FmtVal], arg_idx: &mut i32) -> FmtVal {
    let idx = (*arg_idx - 1) as usize;
    *arg_idx += 1;
    args.get(idx).copied().unwrap_or(FmtVal::Signed(0))
}

fn vim_vsnprintf_typval_inner(
    str_out: impl crate::strings::IntoCCharPtr,
    str_m: usize,
    fmt: impl crate::strings::IntoCCharPtr,
    tvs: *mut TypVal,
    fmt_args: &[FmtVal],
) -> i32 {
    let str_out = str_out.into_c_char_ptr().cast_mut();
    let fmt = fmt.into_c_char_ptr();
    unsafe {
        let mut str_l: usize = 0;
        let mut str_avail = str_l < str_m;
        let mut p = fmt;
        let mut num_posarg: i32 = 0;
        let mut arg_idx: i32 = 1;

        if !parse_fmt_types(&mut num_posarg, fmt, tvs) {
            return 0;
        }

        if p.is_null() {
            p = c"".as_ptr();
        }

        let mut had_error = false;

        'fmt_loop: while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_first_byte(p) != b'%' {
                let rem = crate::memory::cstr_as_bytes(p.wrapping_add(1));
                let n = 1 + rem.iter().position(|&b| b == b'%').unwrap_or(rem.len());
                if str_avail {
                    let avail = str_m - str_l;
                    ptr::copy(p, str_out.wrapping_add(str_l), n.min(avail));
                    str_avail = n < avail;
                }
                p = p.wrapping_add(n);
                str_l += n;
            } else {
                let mut min_field_width: usize = 0;
                let mut precision: usize = 0;
                let mut zero_padding = false;
                let mut precision_specified = false;
                let mut justify_left = false;
                let mut alternate_form = false;
                let mut force_sign = false;
                let mut space_for_positive: i32 = 1;
                let mut length_modifier: c_char = NUL;
                let mut tmp = [0 as c_char; TMP_LEN];
                let mut str_arg: *const c_char = ptr::null();
                let mut str_arg_l: usize;
                let uchar_arg: u8;
                let mut number_of_zeros_to_pad: usize = 0;
                let mut zero_padding_insertion_ind: usize = 0;
                let mut tofree: Option<std::ffi::CString> = None;
                let mut pos_arg: i32 = -1;

                p = p.wrapping_add(1);

                let mut ptype = p;
                while ascii_isdigit(*ptype) {
                    ptype = ptype.wrapping_add(1);
                }

                if crate::memory::cstr_first_byte(ptype) == b'$' {
                    let digstart = p;
                    let mut uj: u32 = 0;
                    if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                        had_error = true;
                        break 'fmt_loop;
                    }
                    pos_arg = uj as i32;
                    p = p.wrapping_add(1);
                }

                loop {
                    match *p as u8 {
                        b'0' => {
                            zero_padding = true;
                            p = p.wrapping_add(1);
                        }
                        b'-' => {
                            justify_left = true;
                            p = p.wrapping_add(1);
                        }
                        b'+' => {
                            force_sign = true;
                            space_for_positive = 0;
                            p = p.wrapping_add(1);
                        }
                        b' ' => {
                            force_sign = true;
                            p = p.wrapping_add(1);
                        }
                        b'#' => {
                            alternate_form = true;
                            p = p.wrapping_add(1);
                        }
                        b'\'' => {
                            p = p.wrapping_add(1);
                        }
                        _ => break,
                    }
                }

                if crate::memory::cstr_first_byte(p) == b'*' {
                    let digstart = p.wrapping_add(1);
                    p = p.wrapping_add(1);

                    if ascii_isdigit(*p) {
                        let mut uj: u32 = 0;
                        if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                            had_error = true;
                            break 'fmt_loop;
                        }
                        arg_idx = uj as i32;
                        p = p.wrapping_add(1);
                    }

                    let mut j = if !tvs.is_null() {
                        tv_nr(tvs, &mut arg_idx) as i32
                    } else {
                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as i32
                    };

                    if j > MAX_ALLOWED_STRING_WIDTH as i32 {
                        if !tvs.is_null() {
                            format_overflow_error(digstart);
                            had_error = true;
                            break 'fmt_loop;
                        } else {
                            j = MAX_ALLOWED_STRING_WIDTH as i32;
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
                    let mut uj: u32 = 0;
                    if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                        had_error = true;
                        break 'fmt_loop;
                    }
                    min_field_width = uj as usize;
                }

                if crate::memory::cstr_first_byte(p) == b'.' {
                    p = p.wrapping_add(1);
                    precision_specified = true;

                    if ascii_isdigit(*p) {
                        let digstart = p;
                        let mut uj: u32 = 0;
                        if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                            had_error = true;
                            break 'fmt_loop;
                        }
                        precision = uj as usize;
                    } else if crate::memory::cstr_first_byte(p) == b'*' {
                        let digstart = p;
                        p = p.wrapping_add(1);

                        if ascii_isdigit(*p) {
                            let mut uj: u32 = 0;
                            if !get_unsigned_int(digstart, &raw mut p, &mut uj, !tvs.is_null()) {
                                had_error = true;
                                break 'fmt_loop;
                            }
                            arg_idx = uj as i32;
                            p = p.wrapping_add(1);
                        }

                        let mut j = if !tvs.is_null() {
                            tv_nr(tvs, &mut arg_idx) as i32
                        } else {
                            fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as i32
                        };

                        if j > MAX_ALLOWED_STRING_WIDTH as i32 {
                            if !tvs.is_null() {
                                format_overflow_error(digstart);
                                had_error = true;
                                break 'fmt_loop;
                            } else {
                                j = MAX_ALLOWED_STRING_WIDTH as i32;
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
                    p = p.wrapping_add(1);
                    if length_modifier == b'l' as c_char && *p == b'l' as c_char {
                        length_modifier = b'L' as c_char;
                        p = p.wrapping_add(1);
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
                                    tv_nr(tvs, &mut arg_idx) as i32
                                } else {
                                    fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as i32
                                };
                                uchar_arg = j as u8;
                                str_arg = (&raw const uchar_arg).cast::<c_char>();
                            }
                            b's' | b'S' => {
                                str_arg = if !tvs.is_null() {
                                    tv_str(tvs, &mut arg_idx, &mut tofree)
                                } else {
                                    fmt_arg_val(fmt_args, &mut arg_idx)
                                        .as_ptr()
                                        .cast::<c_char>()
                                };

                                if str_arg.is_null() {
                                    str_arg = c"[NULL]".as_ptr();
                                    str_arg_l = 6;
                                } else if !precision_specified {
                                    str_arg_l = crate::memory::cstr_len(str_arg);
                                } else if precision == 0 {
                                    str_arg_l = 0;
                                } else {
                                    str_arg_l =
                                        crate::memory::xstrnlen(str_arg, precision.min(0x7fffffff));
                                }

                                if fmt_spec == b'S' as c_char {
                                    let mut p1 = str_arg;
                                    let mut i: usize = 0;
                                    while !crate::memory::cstr_is_empty(p1) {
                                        let cell = utf_ptr2cells(p1) as usize;
                                        if precision_specified && i + cell > precision {
                                            break;
                                        }
                                        i += cell;
                                        p1 = p1.wrapping_add(utfc_ptr2len(p1) as usize);
                                    }
                                    str_arg_l =
                                        crate::memory::ptr_offset_from(p1, str_arg) as usize;
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
                        let mut arg_sign: i32 = 0;
                        let mut arg: i64 = 0;
                        let mut uarg: u64 = 0;
                        let mut ptr_arg: *const c_void = ptr::null();

                        if fmt_spec == b'p' as c_char {
                            ptr_arg = if !tvs.is_null() {
                                tv_ptr(tvs, &mut arg_idx)
                            } else {
                                fmt_arg_val(fmt_args, &mut arg_idx).as_ptr()
                            };
                            if !ptr_arg.is_null() {
                                arg_sign = 1;
                            }
                        } else if fmt_spec == b'b' as c_char || fmt_spec == b'B' as c_char {
                            uarg = if !tvs.is_null() {
                                tv_nr(tvs, &mut arg_idx) as u64
                            } else {
                                fmt_arg_val(fmt_args, &mut arg_idx).as_u64()
                            };
                            arg_sign = i32::from(uarg != 0);
                        } else if fmt_spec == b'd' as c_char {
                            match length_modifier as u8 {
                                0 => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as i32
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as i32
                                    };
                                    arg = i64::from(v);
                                }
                                b'h' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as i32
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64() as i32
                                    };
                                    arg = i64::from(v as i16);
                                }
                                b'l' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as i64
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64()
                                    };
                                    arg = v as i64;
                                }
                                b'L' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as i64
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_i64()
                                    };
                                    arg = v;
                                }
                                b'z' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as isize
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
                                        tv_nr(tvs, &mut arg_idx) as u32
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as u32
                                    };
                                    uarg = u64::from(v);
                                }
                                b'h' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as u32
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as u32
                                    };
                                    uarg = u64::from(v as u16);
                                }
                                b'l' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as u64
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64()
                                    };
                                    uarg = v as u64;
                                }
                                b'L' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as u64
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64()
                                    };
                                    uarg = v;
                                }
                                b'z' => {
                                    let v = if !tvs.is_null() {
                                        tv_nr(tvs, &mut arg_idx) as usize
                                    } else {
                                        fmt_arg_val(fmt_args, &mut arg_idx).as_u64() as usize
                                    };
                                    uarg = v as u64;
                                }
                                _ => {}
                            }
                            arg_sign = i32::from(uarg != 0);
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
                                    if ptr_arg.is_null() {
                                        #[cfg(target_os = "linux")]
                                        let nil_s = "(nil)";
                                        #[cfg(not(target_os = "linux"))]
                                        let nil_s = "0x0";
                                        str_arg_l += write_fmt_to_c_buf(
                                            &mut tmp[str_arg_l..],
                                            format_args!("{nil_s}"),
                                        );
                                    } else {
                                        str_arg_l += write_fmt_to_c_buf(
                                            &mut tmp[str_arg_l..],
                                            format_args!("{:#x}", ptr_arg as usize),
                                        );
                                    }
                                }
                                b'd' => {
                                    str_arg_l += write_fmt_to_c_buf(
                                        &mut tmp[str_arg_l..],
                                        format_args!("{arg}"),
                                    );
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
                                b'o' => {
                                    str_arg_l += write_fmt_to_c_buf(
                                        &mut tmp[str_arg_l..],
                                        format_args!("{uarg:o}"),
                                    );
                                }
                                b'x' => {
                                    str_arg_l += write_fmt_to_c_buf(
                                        &mut tmp[str_arg_l..],
                                        format_args!("{uarg:x}"),
                                    );
                                }
                                b'X' => {
                                    str_arg_l += write_fmt_to_c_buf(
                                        &mut tmp[str_arg_l..],
                                        format_args!("{uarg:X}"),
                                    );
                                }
                                _ => {
                                    str_arg_l += write_fmt_to_c_buf(
                                        &mut tmp[str_arg_l..],
                                        format_args!("{uarg}"),
                                    );
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
                        let mut remove_trailing_zeroes = false;

                        let f = if !tvs.is_null() {
                            tv_float(tvs, &mut arg_idx)
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
                                    i32::from(force_sign),
                                    space_for_positive,
                                ),
                                tmp.len(),
                            );
                            str_arg_l = crate::memory::cstr_len(tmp.as_ptr());
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
                            let sign_prefix = if f.is_sign_negative() {
                                "-"
                            } else if force_sign {
                                if space_for_positive != 0 { " " } else { "+" }
                            } else {
                                ""
                            };
                            let eff_prec = if precision_specified {
                                let mut max_prec = TMP_LEN - 10;
                                if matches!(fmt_spec as u8, b'f' | b'F') && abs_f > 1.0 {
                                    max_prec -= abs_f.log10() as usize;
                                }
                                precision.min(max_prec)
                            } else {
                                6
                            };

                            if matches!(fmt_spec as u8, b'f' | b'F') {
                                str_arg_l = write_fmt_to_c_buf(
                                    &mut tmp,
                                    format_args!("{sign_prefix}{abs_f:.eff_prec$}"),
                                );
                            } else {
                                let sci = format!("{abs_f:.eff_prec$e}");
                                let (mantissa, exp_str) =
                                    sci.split_once('e').unwrap_or((&sci, "0"));
                                let exp: i32 = exp_str.parse().unwrap_or(0);
                                let e_ch = if fmt_spec == b'E' as c_char { 'E' } else { 'e' };
                                str_arg_l = write_fmt_to_c_buf(
                                    &mut tmp,
                                    format_args!("{sign_prefix}{mantissa}{e_ch}{exp:+03}"),
                                );
                            }

                            if remove_trailing_zeroes {
                                let mut tp: *mut c_char;
                                if matches!(fmt_spec as u8, b'f' | b'F') {
                                    tp = tmp.as_mut_ptr().wrapping_add(str_arg_l - 1);
                                } else {
                                    tp = vim_strchr(
                                        tmp.as_ptr(),
                                        if fmt_spec == b'e' as c_char {
                                            b'e' as i32
                                        } else {
                                            b'E' as i32
                                        },
                                    );
                                    if !tp.is_null() {
                                        if crate::memory::cstr_byte_at(tp, 1) == b'+' {
                                            let rem_len =
                                                crate::memory::cstr_len(tp.wrapping_add(2)) + 1;
                                            ptr::copy(
                                                tp.wrapping_add(2),
                                                tp.wrapping_add(1),
                                                rem_len,
                                            );
                                            str_arg_l -= 1;
                                        }
                                        let i = if crate::memory::cstr_byte_at(tp, 1) == b'-' {
                                            2
                                        } else {
                                            1
                                        };
                                        while crate::memory::cstr_byte_at(tp, i) == b'0' {
                                            let rem_len =
                                                crate::memory::cstr_len(tp.wrapping_add(i + 1)) + 1;
                                            ptr::copy(
                                                tp.wrapping_add(i + 1),
                                                tp.wrapping_add(i),
                                                rem_len,
                                            );
                                            str_arg_l -= 1;
                                        }
                                        tp = tp.wrapping_sub(1);
                                    }
                                }

                                if !tp.is_null() && !precision_specified {
                                    while tp > tmp.as_mut_ptr().wrapping_add(2)
                                        && *tp == b'0' as c_char
                                        && *tp.wrapping_sub(1) != b'.' as c_char
                                    {
                                        let rem_len =
                                            crate::memory::cstr_len(tp.wrapping_add(1)) + 1;
                                        ptr::copy(tp.wrapping_add(1), tp, rem_len);
                                        tp = tp.wrapping_sub(1);
                                        str_arg_l -= 1;
                                    }
                                }
                            } else {
                                let tp = vim_strchr(
                                    tmp.as_ptr(),
                                    if fmt_spec == b'e' as c_char {
                                        b'e' as i32
                                    } else {
                                        b'E' as i32
                                    },
                                );
                                if !tp.is_null()
                                    && (crate::memory::cstr_byte_at(tp, 1) == b'+'
                                        || crate::memory::cstr_byte_at(tp, 1) == b'-')
                                    && crate::memory::cstr_byte_at(tp, 2) == b'0'
                                    && ascii_isdigit(crate::memory::cstr_char_at(tp, 3))
                                    && ascii_isdigit(crate::memory::cstr_char_at(tp, 4))
                                {
                                    let rem_len = crate::memory::cstr_len(tp.wrapping_add(3)) + 1;
                                    ptr::copy(tp.wrapping_add(3), tp.wrapping_add(2), rem_len);
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
                        str_arg_l = usize::from(!crate::memory::cstr_is_empty(p));
                    }
                }

                if !crate::memory::cstr_is_empty(p) {
                    p = p.wrapping_add(1);
                }

                if !justify_left && min_field_width > str_arg_l + number_of_zeros_to_pad {
                    let pn = min_field_width - (str_arg_l + number_of_zeros_to_pad);
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::write_bytes(
                            str_out.wrapping_add(str_l),
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
                            ptr::copy(str_arg, str_out.wrapping_add(str_l), zn.min(avail));
                            str_avail = zn < avail;
                        }
                        str_l += zn;
                    }

                    let zn = number_of_zeros_to_pad;
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::write_bytes(str_out.wrapping_add(str_l), b'0', zn.min(avail));
                        str_avail = zn < avail;
                    }
                    str_l += zn;
                }

                if str_arg_l > zero_padding_insertion_ind {
                    let sn = str_arg_l - zero_padding_insertion_ind;
                    if str_avail {
                        let avail = str_m - str_l;
                        ptr::copy(
                            str_arg.wrapping_add(zero_padding_insertion_ind),
                            str_out.wrapping_add(str_l),
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
                        ptr::write_bytes(str_out.wrapping_add(str_l), b' ', pn.min(avail));
                        str_avail = pn < avail;
                    }
                    str_l += pn;
                }

                drop(tofree);
            }
        }

        if !had_error {
            if str_m > 0 {
                *str_out.wrapping_add(str_l.min(str_m - 1)) = NUL;
            }

            if !tvs.is_null() {
                let check_idx = if num_posarg != 0 {
                    num_posarg as usize
                } else {
                    (arg_idx - 1) as usize
                };
                if (*tvs.wrapping_add(check_idx)).v_type != VarType::VarUnknown {
                    emsg(gettext(E_TOO_MANY_ARGS));
                }
            }
        }

        str_l as i32
    }
}

pub fn vim_snprintf_fmt(
    str_out: impl crate::strings::IntoCCharPtr,
    str_m: usize,
    fmt: impl crate::strings::IntoCCharPtr,
    args: &[FmtVal],
) -> i32 {
    let str_out = str_out.into_c_char_ptr().cast_mut();
    let fmt = fmt.into_c_char_ptr();
    vim_vsnprintf_typval_inner(str_out, str_m, fmt, ptr::null_mut(), args)
}

pub fn vim_snprintf_add_fmt(
    str_out: impl crate::strings::IntoCCharPtr,
    str_m: usize,
    fmt: impl crate::strings::IntoCCharPtr,
    args: &[FmtVal],
) -> i32 {
    let str_out = str_out.into_c_char_ptr().cast_mut();
    let fmt = fmt.into_c_char_ptr();
    {
        let len = crate::memory::cstr_len(str_out);
        let space = str_m.saturating_sub(len);
        vim_snprintf_fmt(str_out.wrapping_add(len), space, fmt, args)
    }
}

pub fn vim_snprintf_safelen_fmt(
    str_out: *mut c_char,
    str_m: usize,
    fmt: impl crate::strings::IntoCCharPtr,
    args: &[FmtVal],
) -> usize {
    let fmt = fmt.into_c_char_ptr();
    {
        if str_m == 0 {
            return 0;
        }
        let str_l = vim_snprintf_fmt(str_out, str_m, fmt, args);
        if str_l < 0 {
            unsafe {
                *str_out = NUL;
            }
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
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
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
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
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
            $crate::strings::IntoCCharPtr::into_c_char_ptr($fmt),
            &[$($crate::strings::IntoFmtVal::into_fmt_val($arg)),*],
        )
    };
}
use crate::charset::NUL;
use crate::cmdexpand::MB_MAXBYTES;
use crate::eval::K_LIST_LEN_UNKNOWN_CONST;
use crate::regexp::NUMBUFLEN;
pub use vim_snprintf_safelen;

pub fn vim_snprintf_typval_only(
    str_out: *mut c_char,
    str_m: usize,
    fmt: impl crate::strings::IntoCCharPtr,
    tvs: *mut TypVal,
) -> i32 {
    let fmt = fmt.into_c_char_ptr();
    vim_vsnprintf_typval_inner(str_out, str_m, fmt, tvs, &[])
}

pub fn reverse_text<S: IntoCCharPtr>(s: S) -> std::ffi::CString {
    let s = s.into_c_char_ptr();
    let len = crate::memory::cstr_len(s);
    let mut rev = vec![0u8; len];
    let mut s_i: usize = 0;
    let mut rev_i: usize = len;
    while s_i < len {
        let mb_len = utfc_ptr2len(s.wrapping_add(s_i)) as usize;
        rev_i -= mb_len;
        unsafe {
            ptr::copy(
                s.wrapping_add(s_i).cast::<u8>(),
                rev.as_mut_ptr().wrapping_add(rev_i),
                mb_len,
            )
        };
        s_i += mb_len;
    }
    crate::memory::cstr_from_bytes(&rev)
}

pub fn strrep<S1: IntoCCharPtr, S2: IntoCCharPtr, S3: IntoCCharPtr>(
    src: S1,
    what: S2,
    rep: S3,
) -> Option<std::ffi::CString> {
    let src_bytes = crate::memory::cstr_as_bytes(src.into_c_char_ptr());
    let what_bytes = crate::memory::cstr_as_bytes(what.into_c_char_ptr());
    let rep_bytes = crate::memory::cstr_as_bytes(rep.into_c_char_ptr());

    if what_bytes.is_empty() {
        return None;
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
        return None;
    }

    let total_len = (src_bytes.len() + count * rep_bytes.len()) - count * what_bytes.len();
    let mut ret = Vec::with_capacity(total_len);
    let mut cur: usize = 0;
    while cur + what_bytes.len() <= src_bytes.len() {
        if let Some(rel) = src_bytes[cur..]
            .windows(what_bytes.len())
            .position(|w| w == what_bytes)
        {
            ret.extend_from_slice(&src_bytes[cur..cur + rel]);
            ret.extend_from_slice(rep_bytes);
            cur += rel + what_bytes.len();
        } else {
            break;
        }
    }
    ret.extend_from_slice(&src_bytes[cur..]);

    Some(crate::memory::cstr_from_bytes(&ret))
}

fn byteidx_common(argvars: &mut [TypVal], rettv: &mut TypVal, comp: bool) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let str_ = tv_get_string_chk(&argvars[0]);
        let mut idx = tv_get_number_chk(&argvars[1], None);
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut utf16idx: Varnumber = 0;
        if argvars[2].v_type != VarType::VarUnknown {
            let mut error = false;
            utf16idx = tv_get_bool_chk(&argvars[2], Some(&mut error));
            if error {
                return;
            }
            if !(0..=1).contains(&utf16idx) {
                semsg!(gettext(E_USING_NUMBER_AS_BOOL_NR), utf16idx,);
                return;
            }
        }

        let ptr2len: fn(*const c_char) -> i32 = if comp { utf_ptr2len } else { utfc_ptr2len };

        let mut t = str_;
        while idx > 0 {
            if crate::memory::cstr_is_empty(t) {
                return;
            }
            if utf16idx != 0 {
                let clen = ptr2len(t);
                let c = if clen > 1 { utf_ptr2char(t) } else { *t as i32 };
                if c > 0xFFFF {
                    idx -= 1;
                }
                if idx > 0 {
                    t = t.wrapping_add(clen as usize);
                }
            } else {
                t = t.wrapping_add(ptr2len(t) as usize);
            }
            idx -= 1;
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(
            crate::memory::ptr_offset_from(t, str_) as Varnumber,
        );
    }
}

pub fn f_byteidx(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    byteidx_common(argvars, rettv, false);
}

pub fn f_byteidxcomp(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    byteidx_common(argvars, rettv, true);
}

pub fn f_charidx(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if !tv_check_for_string_arg(argvars, 0)
            || !tv_check_for_number_arg(argvars, 1)
            || !tv_check_for_opt_bool_arg(argvars, 2)
            || (argvars[2].v_type != VarType::VarUnknown && !tv_check_for_opt_bool_arg(argvars, 3))
        {
            return;
        }

        let str_ = tv_get_string_chk(&argvars[0]);
        let mut idx = tv_get_number_chk(&argvars[1], None);
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut countcc: Varnumber = 0;
        let mut utf16idx: Varnumber = 0;
        if argvars[2].v_type != VarType::VarUnknown {
            countcc = tv_get_bool(&argvars[2]);
            if argvars[3].v_type != VarType::VarUnknown {
                utf16idx = tv_get_bool(&argvars[3]);
            }
        }

        let ptr2len: fn(*const c_char) -> i32 = if countcc != 0 {
            utf_ptr2len
        } else {
            utfc_ptr2len
        };

        let mut p = str_;
        let mut len: i32 = 0;
        while if utf16idx != 0 {
            idx >= 0
        } else {
            p <= str_.wrapping_add(idx as usize)
        } {
            if crate::memory::cstr_is_empty(p) {
                if if utf16idx != 0 {
                    idx == 0
                } else {
                    p == str_.wrapping_add(idx as usize)
                } {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(len));
                }
                return;
            }
            if utf16idx != 0 {
                idx -= 1;
                let clen = ptr2len(p);
                let c = if clen > 1 { utf_ptr2char(p) } else { *p as i32 };
                if c > 0xFFFF {
                    idx -= 1;
                }
            }
            p = p.wrapping_add(ptr2len(p) as usize);
            len += 1;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(if len > 0 {
            Varnumber::from(len - 1)
        } else {
            0
        });
    }
}

pub fn f_str2list(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_UNKNOWN_CONST);
    let mut p = tv_get_string(&argvars[0]);
    while !crate::memory::cstr_is_empty(p) {
        tv_list_append_number(rettv.vval.v_list_ref(), Varnumber::from(utf_ptr2char(p)));
        p = p.wrapping_add(utf_ptr2len(p) as usize);
    }
}

pub fn f_str2nr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut base: i32 = 10;
    let mut what: i32 = 0;

    if argvars[1].v_type != VarType::VarUnknown {
        base = tv_get_number(&argvars[1]) as i32;
        if !matches!(base, 2 | 8 | 10 | 16) {
            emsg(gettext(E_INVARG));
            return;
        }
        if argvars[2].v_type != VarType::VarUnknown && tv_get_bool(&argvars[2]) != 0 {
            what |= STR2NR_QUOTE;
        }
    }

    let mut p = skipwhite(tv_get_string(&argvars[0]));
    let isneg = crate::memory::cstr_first_byte(p) == b'-';
    if crate::memory::cstr_first_byte(p) == b'+' || crate::memory::cstr_first_byte(p) == b'-' {
        p = skipwhite(p.wrapping_add(1));
    }
    match base {
        2 => what |= STR2NR_BIN | STR2NR_FORCE,
        8 => what |= STR2NR_OCT | STR2NR_OOCT | STR2NR_FORCE,
        16 => what |= STR2NR_HEX | STR2NR_FORCE,
        _ => {}
    }
    let mut n: Varnumber = 0;
    vim_str2nr(p, None, None, what, Some(&mut n), None, 0, false, None);
    rettv.vval = crate::eval::typval::TypvalVval::Number(if isneg { -n } else { n });
}

pub fn f_strgetchar(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let str_ = tv_get_string_chk(&argvars[0]);
    if str_.is_null() {
        return;
    }
    let mut error = false;
    let mut charidx = tv_get_number_chk(&argvars[1], Some(&mut error));
    if error {
        return;
    }

    let len = crate::memory::cstr_len(str_);
    let mut byteidx: usize = 0;
    while charidx >= 0 && byteidx < len {
        if charidx == 0 {
            rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(utf_ptr2char(
                str_.wrapping_add(byteidx),
            )));
            break;
        }
        charidx -= 1;
        byteidx += utf_ptr2len(str_.wrapping_add(byteidx)) as usize;
    }
}

pub fn f_stridx(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let mut buf = [0 as c_char; NUMBUFLEN];
    let needle = tv_get_string_chk(&argvars[1]);
    let mut haystack = tv_get_string_buf_chk(&argvars[0], buf.as_mut_ptr());
    let haystack_start = haystack;
    if needle.is_null() || haystack.is_null() {
        return;
    }

    let hay_len = crate::memory::cstr_len(haystack);
    if argvars[2].v_type != VarType::VarUnknown {
        let mut error = false;
        let start_idx = tv_get_number_chk(&argvars[2], Some(&mut error)) as isize;
        if error || start_idx >= hay_len as isize {
            return;
        }
        if start_idx >= 0 {
            haystack = haystack.wrapping_add(start_idx as usize);
        }
    }

    let hay_bytes = crate::memory::cstr_as_bytes(haystack);
    let needle_bytes = crate::memory::cstr_as_bytes(needle);
    if needle_bytes.is_empty() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(crate::memory::ptr_offset_from(
            haystack,
            haystack_start,
        ) as Varnumber);
    } else if let Some(pos) = hay_bytes
        .windows(needle_bytes.len())
        .position(|w| w == needle_bytes)
    {
        rettv.vval = crate::eval::typval::TypvalVval::Number(crate::memory::ptr_offset_from(
            haystack.wrapping_add(pos),
            haystack_start,
        ) as Varnumber);
    }
}

pub fn f_string(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, encode_tv2string(&mut argvars[0], None).as_ptr());
}

pub fn f_strlen(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(crate::memory::cstr_len(tv_get_string(
        &argvars[0],
    )) as Varnumber);
}

fn strchar_common(argvars: &mut [TypVal], rettv: &mut TypVal, skipcc: bool) {
    let mut s = tv_get_string(&argvars[0]);
    let mut len: Varnumber = 0;
    let func_mb_ptr2char_adv: fn(*mut *const c_char) -> i32 = if skipcc {
        mb_ptr2char_adv
    } else {
        mb_cptr2char_adv
    };
    while !crate::memory::cstr_is_empty(s) {
        func_mb_ptr2char_adv(&raw mut s);
        len += 1;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(len);
}

pub fn f_strcharlen(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    strchar_common(argvars, rettv, true);
}

pub fn f_strchars(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut skipcc: Varnumber = 0;
    if argvars[1].v_type != VarType::VarUnknown {
        let mut error = false;
        skipcc = tv_get_bool_chk(&argvars[1], Some(&mut error));
        if error {
            return;
        }
        if !(0..=1).contains(&skipcc) {
            semsg!(gettext(E_USING_NUMBER_AS_BOOL_NR), skipcc,);
            return;
        }
    }
    strchar_common(argvars, rettv, skipcc != 0);
}

pub fn f_strutf16len(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_string_arg(argvars, 0) || !tv_check_for_opt_bool_arg(argvars, 1) {
        return;
    }

    let mut countcc: Varnumber = 0;
    if argvars[1].v_type != VarType::VarUnknown {
        countcc = tv_get_bool(&argvars[1]);
    }

    let mut s = tv_get_string(&argvars[0]);
    let mut len: Varnumber = 0;
    let func_mb_ptr2char_adv: fn(*mut *const c_char) -> i32 = if countcc != 0 {
        mb_cptr2char_adv
    } else {
        mb_ptr2char_adv
    };
    while !crate::memory::cstr_is_empty(s) {
        let ch = func_mb_ptr2char_adv(&raw mut s);
        if ch > 0xFFFF {
            len += 1;
        }
        len += 1;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(len);
}

pub fn f_strdisplaywidth(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let s = tv_get_string(&argvars[0]);
    let mut col: i32 = 0;
    if argvars[1].v_type != VarType::VarUnknown {
        col = tv_get_number(&argvars[1]) as i32;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(
        linetabsize_col(col, s.cast_mut()) - col,
    ));
}

pub fn f_strwidth(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let s = tv_get_string(&argvars[0]);
    rettv.vval = crate::eval::typval::TypvalVval::Number(mb_string2cells(s) as Varnumber);
}

pub fn f_strcharpart(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let p = tv_get_string(&argvars[0]);
    let slen = crate::memory::cstr_len(p);

    let mut nbyte: i32 = 0;
    let mut skipcc: Varnumber = 0;
    let mut error = false;
    let mut nchar = tv_get_number_chk(&argvars[1], Some(&mut error));
    if !error {
        if argvars[2].v_type != VarType::VarUnknown && argvars[3].v_type != VarType::VarUnknown {
            skipcc = tv_get_bool_chk(&argvars[3], Some(&mut error));
            if error {
                return;
            }
            if !(0..=1).contains(&skipcc) {
                semsg!(gettext(E_USING_NUMBER_AS_BOOL_NR), skipcc,);
                return;
            }
        }

        if nchar > 0 {
            while nchar > 0 && (nbyte as usize) < slen {
                if skipcc != 0 {
                    nbyte += utfc_ptr2len(p.wrapping_add(nbyte as usize));
                } else {
                    nbyte += utf_ptr2len(p.wrapping_add(nbyte as usize));
                }
                nchar -= 1;
            }
        } else {
            nbyte = nchar as i32;
        }
    }
    let mut len: i32 = 0;
    if argvars[2].v_type != VarType::VarUnknown {
        let mut charlen = tv_get_number(&argvars[2]) as i32;
        while charlen > 0 && nbyte + len < slen as i32 {
            let off = nbyte + len;
            if off < 0 {
                len += 1;
            } else if skipcc != 0 {
                len += utfc_ptr2len(p.wrapping_add(off as usize));
            } else {
                len += utf_ptr2len(p.wrapping_add(off as usize));
            }
            charlen -= 1;
        }
    } else {
        len = (slen as i32) - nbyte;
    }

    if nbyte < 0 {
        len += nbyte;
        nbyte = 0;
    } else if (nbyte as usize) > slen {
        nbyte = slen as i32;
    }
    if len < 0 {
        len = 0;
    } else if nbyte + len > slen as i32 {
        len = (slen as i32) - nbyte;
    }

    crate::eval::typval::tv_set_string_len(rettv, p.wrapping_add(nbyte as usize), len as usize);
}

pub fn f_strpart(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut error = false;
    let p = tv_get_string(&argvars[0]);
    let slen = crate::memory::cstr_len(p) as Varnumber;

    let mut n = tv_get_number_chk(&argvars[1], Some(&mut error));
    let mut len = if error {
        0
    } else if argvars[2].v_type != VarType::VarUnknown {
        tv_get_number(&argvars[2])
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

    if argvars[2].v_type != VarType::VarUnknown && argvars[3].v_type != VarType::VarUnknown {
        let mut off = n;
        while off < slen && len > 0 {
            off += Varnumber::from(utfc_ptr2len(p.wrapping_add(off as usize)));
            len -= 1;
        }
        len = off - n;
    }

    crate::eval::typval::tv_set_string_len(rettv, p.wrapping_add(n as usize), len as usize);
}

pub fn f_strridx(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = [0 as c_char; NUMBUFLEN];
    let needle = tv_get_string_chk(&argvars[1]);
    let haystack = tv_get_string_buf_chk(&argvars[0], buf.as_mut_ptr());

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
    if needle.is_null() || haystack.is_null() {
        return;
    }

    let hay_bytes = crate::memory::cstr_as_bytes(haystack);
    let needle_bytes = crate::memory::cstr_as_bytes(needle);
    let haystack_len = hay_bytes.len();
    let end_idx: isize = if argvars[2].v_type != VarType::VarUnknown {
        let idx = tv_get_number_chk(&argvars[2], None) as isize;
        if idx < 0 {
            return;
        }
        idx
    } else {
        haystack_len as isize
    };

    if needle_bytes.is_empty() {
        rettv.vval = crate::eval::typval::TypvalVval::Number(
            end_idx.min(haystack_len as isize) as Varnumber
        );
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
        rettv.vval = crate::eval::typval::TypvalVval::Number(pos as Varnumber);
    }
}

pub fn f_strtrans(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(rettv, transstr(tv_get_string(&argvars[0]), true).as_ptr());
}

pub fn f_utf16idx(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if !tv_check_for_string_arg(argvars, 0)
            || !tv_check_for_opt_number_arg(argvars, 1)
            || !tv_check_for_opt_bool_arg(argvars, 2)
            || (argvars[2].v_type != VarType::VarUnknown && !tv_check_for_opt_bool_arg(argvars, 3))
        {
            return;
        }

        let str_ = tv_get_string_chk(&argvars[0]);
        let mut idx = tv_get_number_chk(&argvars[1], None);
        if str_.is_null() || idx < 0 {
            return;
        }

        let mut countcc: Varnumber = 0;
        let mut charidx: Varnumber = 0;
        if argvars[2].v_type != VarType::VarUnknown {
            countcc = tv_get_bool(&argvars[2]);
            if argvars[3].v_type != VarType::VarUnknown {
                charidx = tv_get_bool(&argvars[3]);
            }
        }

        let ptr2len: fn(*const c_char) -> i32 = if countcc != 0 {
            utf_ptr2len
        } else {
            utfc_ptr2len
        };

        let mut p = str_;
        let mut len: i32 = 0;
        let mut utf16idx: i32 = 0;
        while if charidx != 0 {
            idx >= 0
        } else {
            p <= str_.wrapping_add(idx as usize)
        } {
            if crate::memory::cstr_is_empty(p) {
                if if charidx != 0 {
                    idx == 0
                } else {
                    p == str_.wrapping_add(idx as usize)
                } {
                    rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(len));
                }
                return;
            }
            utf16idx = len;
            let clen = ptr2len(p);
            let c = if clen > 1 { utf_ptr2char(p) } else { *p as i32 };
            if c > 0xFFFF {
                len += 1;
            }
            p = p.wrapping_add(clen as usize);
            if charidx != 0 {
                idx -= 1;
            }
            len += 1;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(Varnumber::from(utf16idx));
    }
}

pub fn f_tolower(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(
        rettv,
        strcase_save(tv_get_string(&argvars[0]), false).as_ptr(),
    );
}

pub fn f_toupper(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    crate::eval::typval::tv_set_string(
        rettv,
        strcase_save(tv_get_string(&argvars[0]), true).as_ptr(),
    );
}

pub fn f_tr(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = [0 as c_char; NUMBUFLEN];
    let mut buf2 = [0 as c_char; NUMBUFLEN];

    let mut in_str = tv_get_string(&argvars[0]);
    let fromstr = tv_get_string_buf_chk(&argvars[1], buf.as_mut_ptr());
    let tostr = tv_get_string_buf_chk(&argvars[2], buf2.as_mut_ptr());

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    if fromstr.is_null() || tostr.is_null() {
        return;
    }
    let mut out: Vec<c_char> = Vec::with_capacity(80);

    let mut first = true;
    while !crate::memory::cstr_is_empty(in_str) {
        let mut cpstr = in_str;
        let inlen = utfc_ptr2len(in_str) as usize;
        let mut cplen = inlen;
        let mut idx: i32 = 0;
        let mut p = fromstr;
        while !crate::memory::cstr_is_empty(p) {
            let fromlen = utfc_ptr2len(p) as usize;
            let in_slice = unsafe { std::slice::from_raw_parts(in_str.cast::<u8>(), inlen) };
            let p_slice = unsafe { std::slice::from_raw_parts(p.cast::<u8>(), fromlen) };
            if fromlen == inlen && in_slice == p_slice {
                let mut tp = tostr;
                while !crate::memory::cstr_is_empty(tp) {
                    let tolen = utfc_ptr2len(tp) as usize;
                    if idx == 0 {
                        idx -= 1;
                        cplen = tolen;
                        cpstr = tp;
                        break;
                    }
                    idx -= 1;
                    tp = tp.wrapping_add(tolen);
                }
                if crate::memory::cstr_is_empty(tp) {
                    semsg!(gettext(E_INVARG2), fromstr);
                    return;
                }
                break;
            }
            idx += 1;
            p = p.wrapping_add(fromlen);
        }

        if first && ptr::eq(cpstr, in_str) {
            first = false;
            let mut tp = tostr;
            while !crate::memory::cstr_is_empty(tp) {
                let tolen = utfc_ptr2len(tp) as usize;
                idx -= 1;
                tp = tp.wrapping_add(tolen);
            }
            if idx != 0 {
                semsg!(gettext(E_INVARG2), fromstr);
                return;
            }
        }

        out.extend_from_slice(unsafe { std::slice::from_raw_parts(cpstr, cplen) });

        in_str = in_str.wrapping_add(inlen);
    }

    crate::eval::typval::tv_set_string_len(rettv, out.as_ptr(), out.len());
}

pub fn f_trim(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf1 = [0 as c_char; NUMBUFLEN];
    let mut buf2 = [0 as c_char; NUMBUFLEN];
    let mut head = tv_get_string_buf_chk(&argvars[0], buf1.as_mut_ptr());
    let mut mask: *const c_char = ptr::null();
    let mut dir: i32 = 0;

    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());
    if head.is_null() {
        return;
    }

    if !tv_check_for_opt_string_arg(argvars, 1) {
        return;
    }

    if argvars[1].v_type == VarType::VarString {
        mask = tv_get_string_buf_chk(&argvars[1], buf2.as_mut_ptr());
        if crate::memory::cstr_is_empty(mask) {
            mask = ptr::null();
        }

        if argvars[2].v_type != VarType::VarUnknown {
            let mut error = false;
            dir = tv_get_number_chk(&argvars[2], Some(&mut error)) as i32;
            if error {
                return;
            }
            if !(0..=2).contains(&dir) {
                semsg!(gettext(E_INVARG2), tv_get_string(&argvars[2]),);
                return;
            }
        }
    }

    if dir == 0 || dir == 1 {
        while !crate::memory::cstr_is_empty(head) {
            let c1 = utf_ptr2char(head);
            if mask.is_null() {
                if c1 > b' ' as i32 && c1 != 0xa0 {
                    break;
                }
            } else {
                let mut p = mask;
                while !crate::memory::cstr_is_empty(p) {
                    if c1 == utf_ptr2char(p) {
                        break;
                    }
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
            }
            head = head.wrapping_add(utfc_ptr2len(head) as usize);
        }
    }

    let mut tail = head.wrapping_add(crate::memory::cstr_len(head));
    if dir == 0 || dir == 2 {
        while tail > head {
            let prev = tail.wrapping_sub((utf_head_off(head, tail.wrapping_sub(1)) + 1) as usize);
            let c1 = utf_ptr2char(prev);
            if mask.is_null() {
                if c1 > b' ' as i32 && c1 != 0xa0 {
                    break;
                }
            } else {
                let mut p = mask;
                while !crate::memory::cstr_is_empty(p) {
                    if c1 == utf_ptr2char(p) {
                        break;
                    }
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }
                if crate::memory::cstr_is_empty(p) {
                    break;
                }
            }
            tail = prev;
        }
    }
    crate::eval::typval::tv_set_string_len(
        rettv,
        head,
        crate::memory::ptr_offset_from(tail, head) as usize,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_strnsave_unquoted_and_strrep_and_format_typeof_work() {
        let unquoted = vim_strnsave_unquoted(b"\"/bin/sh\"");
        assert_eq!(unquoted.to_str().unwrap(), "/bin/sh");

        let rep = strrep(c"foo bar foo", c"foo", c"baz").unwrap();
        assert_eq!(rep.to_str().unwrap(), "baz bar baz");

        let cat = concat_str(c"hello ", c"world");
        assert_eq!(cat.to_str().unwrap(), "hello world");

        assert_eq!(format_typeof(c"lld"), TYPE_LONGLONGINT);
        assert_eq!(format_typeof(c"zu"), TYPE_SIZET);

        assert_eq!(crate::charset::parse_i64_prefix(b"  -1234px"), -1234);
        assert_eq!(crate::charset::parse_i64_prefix(b" +56789"), 56789);
        let mut end: *mut c_char = ptr::null_mut();
        assert_eq!(strtod(c"  -12.5e2px", &raw mut end), -1250.0);
        assert_eq!(unsafe { *end } as u8, b'p');
        assert_eq!(strtod(c"0x1.8p+2", &raw mut end), 6.0);
        assert!(strtod(c"-INF", &raw mut end).is_infinite());
        assert!(strtod(c"NaN", &raw mut end).is_nan());
    }

    #[test]
    fn vim_snprintf_macros_format_correctly() {
        let mut buf = [0 as c_char; 64];
        let n = vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"%s %d %x %06b",
            c"val",
            -42,
            0xdeadbeef_u32,
            12_u64,
        );
        assert_eq!(n, 23);
        assert_eq!(
            crate::memory::cstr_from_ptr(buf.as_ptr()).to_str().unwrap(),
            "val -42 deadbeef 001100"
        );

        let added = vim_snprintf_add!(buf.as_mut_ptr(), buf.len(), c" %2$s %1$d", 7, c"ok");
        assert_eq!(added, 5);
        assert_eq!(
            crate::memory::cstr_from_ptr(buf.as_ptr()).to_str().unwrap(),
            "val -42 deadbeef 001100 ok 7"
        );

        let mut small = [0 as c_char; 5];
        let safelen = vim_snprintf_safelen!(small.as_mut_ptr(), small.len(), c"hello world",);
        assert_eq!(safelen, 4);
        assert_eq!(
            crate::memory::cstr_from_ptr(small.as_ptr())
                .to_str()
                .unwrap(),
            "hell"
        );
    }

    #[test]
    fn vim_strsave_escaped_unquoted_strchr_strcase_and_reverse() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::option_vars::opt_vars().cmp_flags =
                crate::mbyte::K_OPT_CMP_FLAG_INTERNAL | crate::mbyte::K_OPT_CMP_FLAG_KEEPASCII;

            for (s, esc, expected) in [
                (c"abcd", c"abcd", "\\a\\b\\c\\d"),
                (c"abcd", c"ab", "\\a\\bcd"),
                (c"text \n text", c"", "text \n text"),
                (c"", c"\r", ""),
                (c"some text", c"a", "some text"),
            ] {
                let res = vim_strsave_escaped(s, esc);
                assert_eq!(res.to_str().unwrap(), expected);
            }

            for (s, len, expected) in [
                (c"-c", 2usize, "-c"),
                (c"", 0, ""),
                (c"-c", 0, ""),
                (c"-c", 1, "-"),
                (c"\"-c", 2, "-"),
                (c"\"/bin/sh\"", 9, "/bin/sh"),
                (c"/Program\" \"Files/sh", 19, "/Program Files/sh"),
                (c"/\"\"Program\" \"Files/sh", 21, "/Program Files/sh"),
                (
                    c"/\"\\\"\"Program Files\"\\\"\"/sh",
                    25,
                    "/\"Program Files\"/sh",
                ),
                (
                    c"/\"\\\\\"Program Files\"\\\\foo\"/sh",
                    28,
                    "/\\Program Files\\foo/sh",
                ),
                (c"/Program\" Files/sh", 18, "/Program Files/sh"),
                (c"\"", 1, ""),
                (c"/Program\" Files/sh\\", 19, "/Program Files/sh\\"),
                (c"/Program\\ Files/sh\\", 19, "/Program\\ Files/sh\\"),
                (c"/Program\"\\n\"Files/sh", 20, "/Program\\nFiles/sh"),
            ] {
                let res = vim_strnsave_unquoted(&s.to_bytes()[..len]);
                assert_eq!(res.to_str().unwrap(), expected);
            }

            let s_abc = c"abc";
            assert!(!crate::strings::vim_haschr(s_abc.as_ptr(), 0));
            assert!(!crate::strings::vim_haschr(s_abc.as_ptr(), -1));
            assert_eq!(
                crate::memory::ptr_offset_from(
                    vim_strchr(s_abc.as_ptr(), b'b' as i32),
                    s_abc.as_ptr()
                ),
                1
            );
            let s_utf = c"„«»“";
            assert_eq!(
                crate::memory::ptr_offset_from(vim_strchr(s_utf.as_ptr(), 0xAB), s_utf.as_ptr()),
                3
            );
            assert_eq!(
                crate::memory::ptr_offset_from(vim_strchr(s_utf.as_ptr(), 0x201C), s_utf.as_ptr()),
                7
            );
            assert!(!crate::strings::vim_haschr(s_utf.as_ptr(), 0x201D));

            let overlong = c"\xc1\x81";
            let up = strcase_save(overlong, true);
            let lo = strcase_save(overlong, false);
            assert_eq!(up.to_str().unwrap(), "A");
            assert_eq!(lo.to_str().unwrap(), "a");

            for (inp, exp) in [
                (b"\0" as &[u8], b"" as &[u8]),
                (b"a\0", b"a"),
                (b"ab\0", b"ba"),
                ("öy nötöY\0".as_bytes(), "Yötön yö".as_bytes()),
                (b"ab\xc0\0", b"\xc0ba"),
                (b"ab\xc2\0", b"\xc2ba"),
            ] {
                let rev = reverse_text(CStr::from_bytes_with_nul(inp).unwrap());
                assert_eq!(rev.as_bytes(), exp);
            }
        }
    }
}
