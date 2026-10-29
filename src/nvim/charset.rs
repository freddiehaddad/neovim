//! Code related to character sets (`src/nvim/charset.c`).

use crate::buffer::FileBuf;
use crate::cursor::get_cursor_line_ptr;
use crate::globals::*;
use crate::mbyte::{
    UTF8LEN_TAB, mb_islower, mb_isupper, mb_ptr2char_adv, mb_tolower, utf_char2bytes,
    utf_char2cells, utf_char2len, utf_class_tab, utf_cluster_info, utf_printable,
    utf_ptr2_str_char_info, utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::memory::cstr_from_bytes;
use crate::option::{get_fileformat, skip_to_option_part};
use crate::option_vars::*;
use crate::path::path_has_wildcard;
use std::ffi::{CString, c_char};
use std::ptr;

use crate::change::TAB;
use crate::eval::typval::Varnumber;
use crate::keycodes::CAR;
use crate::regexp::MAXCOL;
pub type Uvarnumber = u64;

pub(crate) const NUL: c_char = 0;
pub(crate) const NL: c_char = b'\n' as c_char;

pub trait AsciiInt {
    fn into_ascii_int(self) -> i32;
}

impl AsciiInt for u8 {
    #[inline]
    fn into_ascii_int(self) -> i32 {
        i32::from(self)
    }
}

impl AsciiInt for i8 {
    #[inline]
    fn into_ascii_int(self) -> i32 {
        i32::from(self)
    }
}

impl AsciiInt for i32 {
    #[inline]
    fn into_ascii_int(self) -> i32 {
        self
    }
}

#[inline]
pub fn ascii_iswhite<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ch == b' ' as i32 || ch == b'\t' as i32
}

#[inline]
pub fn ascii_isdigit<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'0' as i32..=b'9' as i32).contains(&ch)
}

#[inline]
pub fn ascii_isalpha<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'a' as i32..=b'z' as i32).contains(&ch) || (b'A' as i32..=b'Z' as i32).contains(&ch)
}

#[inline]
pub fn ascii_isalnum<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_isalpha(ch) || ascii_isdigit(ch)
}

#[inline]
pub fn ascii_isxdigit<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_isdigit(ch)
        || (b'a' as i32..=b'f' as i32).contains(&ch)
        || (b'A' as i32..=b'F' as i32).contains(&ch)
}

#[inline]
pub fn ascii_isspace<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (9..=13).contains(&ch) || ch == b' ' as i32
}

#[inline]
pub fn ascii_isupper<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'A' as i32..=b'Z' as i32).contains(&ch)
}

#[inline]
pub fn ascii_islower<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'a' as i32..=b'z' as i32).contains(&ch)
}

#[inline]
pub fn ascii_iswhite_or_nul<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_iswhite(ch) || ch == 0
}

#[inline]
pub fn ascii_isident<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_isalnum(ch) || ch == b'_' as i32
}

#[inline]
pub const fn toupper_asc(c: i32) -> i32 {
    if c >= b'a' as i32 && c <= b'z' as i32 {
        c - 0x20
    } else {
        c
    }
}

#[inline]
pub const fn tolower_asc(c: i32) -> i32 {
    if c >= b'A' as i32 && c <= b'Z' as i32 {
        c + 0x20
    } else {
        c
    }
}

#[inline]
pub const fn is_special(c: i32) -> bool {
    c < 0
}

#[inline]
pub const fn to_special(a: i32, b: i32) -> i32 {
    if a == KS_SPECIAL {
        K_SPECIAL
    } else if a == KS_ZERO {
        crate::keycodes::K_ZERO
    } else {
        crate::keycodes::termcap2key(a, b)
    }
}

pub(crate) const CTRL_V_CONST: c_char = 22;

pub(crate) const EOL_MAC: i32 = 2;
pub(crate) const K_OPT_DY_FLAG_UHEX: u32 = 0x04;

const K_SPECIAL: i32 = 0x80;
const KS_SPECIAL: i32 = 254;
const KS_ZERO: i32 = 255;

pub(crate) const STR2NR_BIN: i32 = 1 << 0;
pub(crate) const STR2NR_OCT: i32 = 1 << 1;
pub(crate) const STR2NR_HEX: i32 = 1 << 2;
pub(crate) const STR2NR_OOCT: i32 = 1 << 3;
pub(crate) const STR2NR_QUOTE: i32 = 1 << 4;
pub(crate) const STR2NR_FORCE: i32 = 1 << 7;

const VARNUMBER_MAX: Uvarnumber = i64::MAX as u64;
pub(crate) const VARNUMBER_MIN: Varnumber = i64::MIN;
const UVARNUMBER_MAX: Uvarnumber = u64::MAX;

struct CharsetModuleState {
    isf_chartab: [u64; 4],
    isi_chartab: [u64; 4],
    transchar_charbuf: [u8; 11],
}

impl CharsetModuleState {
    const fn new() -> Self {
        Self {
            isf_chartab: [0; 4],
            isi_chartab: [0; 4],
            transchar_charbuf: [0; 11],
        }
    }
}

static CHARSET_STATE: crate::globals::GlobalState<CharsetModuleState> =
    crate::globals::GlobalState::new(CharsetModuleState::new());

#[inline]
fn charset_st() -> &'static mut CharsetModuleState {
    unsafe { &mut *CHARSET_STATE.as_ptr() }
}

#[inline]
fn set_chartab(chartab: *mut u64, c: i32) {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.wrapping_add(idx) |= 1u64 << (c & 0x3f);
    }
}

#[inline]
fn reset_chartab(chartab: *mut u64, c: i32) {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.wrapping_add(idx) &= !(1u64 << (c & 0x3f));
    }
}

#[inline]
fn get_chartab(chartab: *const u64, c: i32) -> u64 {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.wrapping_add(idx) & (1u64 << (c & 0x3f))
    }
}

#[inline]
pub const fn ascii_isbdigit(c: i32) -> bool {
    c == b'0' as i32 || c == b'1' as i32
}

#[inline]
const fn ascii_isodigit(c: i32) -> bool {
    c >= b'0' as i32 && c <= b'7' as i32
}

#[inline]
pub fn ascii2cells(b: i32) -> i32 {
    if (b' ' as i32..=b'~' as i32).contains(&b) {
        1
    } else {
        opt_vars().dy_escape_width
    }
}

#[inline]
const fn nr2hex(n: u32) -> u8 {
    let v = (n & 0xf) as u8;
    if v <= 9 { v + b'0' } else { v - 10 + b'a' }
}

use crate::keycodes::k_second;

pub fn buf_init_isk_chartab(buf: &mut FileBuf) -> bool {
    let save_chartab = buf.b_chartab;
    buf.b_chartab = [0; 4];
    if buf.b_p_lisp != 0 {
        set_chartab(buf.b_chartab.as_mut_ptr(), b'-' as i32);
    }
    if !parse_isopt(buf.b_p_isk, buf.b_chartab.as_mut_ptr()) {
        buf.b_chartab = save_chartab;
        return false;
    }
    true
}

pub fn init_isf_chartab() -> bool {
    let save_chartab = charset_st().isf_chartab;
    charset_st().isf_chartab = [0, 0, 0xFFFF_FFFF_0000_0000, 0xFFFF_FFFF_FFFF_FFFF];
    if !parse_isopt(
        opt_vars().p_isf,
        (&raw mut charset_st().isf_chartab).cast::<u64>(),
    ) {
        charset_st().isf_chartab = save_chartab;
        return false;
    }
    true
}

pub fn init_isi_chartab() -> bool {
    let save_chartab = charset_st().isi_chartab;
    charset_st().isi_chartab = [0; 4];
    if !parse_isopt(
        opt_vars().p_isi,
        (&raw mut charset_st().isi_chartab).cast::<u64>(),
    ) {
        charset_st().isi_chartab = save_chartab;
        return false;
    }
    true
}

pub fn check_isopt(var: impl crate::strings::IntoCCharPtr) -> bool {
    let var = var.into_c_char_ptr();
    parse_isopt(var, ptr::null_mut())
}

pub fn parse_isopt(var: impl crate::strings::IntoCCharPtr, chartab: *mut u64) -> bool {
    let var = var.into_c_char_ptr();
    {
        let mut p = var;

        while !crate::memory::cstr_is_empty(p) {
            let mut tilde = false;
            let mut do_isalpha = false;

            if crate::memory::cstr_first_byte(p) == b'^' && crate::memory::cstr_byte_at(p, 1) != 0 {
                tilde = true;
                p = p.wrapping_add(1);
            }

            let mut c = if ascii_isdigit(crate::memory::cstr_first_char(p)) {
                getdigits_int((&raw mut p).cast::<*mut c_char>(), false, 0)
            } else {
                mb_ptr2char_adv(&raw mut p)
            };
            let mut c2: i32 = -1;

            if crate::memory::cstr_first_byte(p) == b'-' && crate::memory::cstr_byte_at(p, 1) != 0 {
                p = p.wrapping_add(1);
                c2 = if ascii_isdigit(crate::memory::cstr_first_char(p)) {
                    getdigits_int((&raw mut p).cast::<*mut c_char>(), false, 0)
                } else {
                    mb_ptr2char_adv(&raw mut p)
                };
            }

            if c <= 0
                || c >= 256
                || (c2 < c && c2 != -1)
                || c2 >= 256
                || !(crate::memory::cstr_is_empty(p) || crate::memory::cstr_first_byte(p) == b',')
            {
                return false;
            }

            let trail_comma = crate::memory::cstr_first_byte(p) == b',';
            p = skip_to_option_part(p);
            if trail_comma && crate::memory::cstr_is_empty(p) {
                return false;
            }

            if chartab.is_null() {
                continue;
            }

            if c2 == -1 {
                if c == b'@' as i32 {
                    do_isalpha = true;
                    c = 1;
                    c2 = 255;
                } else {
                    c2 = c;
                }
            }

            while c <= c2 {
                if !do_isalpha || mb_islower(c) || mb_isupper(c) {
                    if tilde {
                        reset_chartab(chartab, c);
                    } else {
                        set_chartab(chartab, c);
                    }
                }
                c += 1;
            }
        }

        true
    }
}

pub fn trans_characters(mut buf: *mut c_char, bufsize: i32) {
    let mut len = crate::memory::cstr_len(buf) as i32;
    let mut room = bufsize - len;

    while !crate::memory::cstr_is_empty(buf) {
        let mut trs_len = utfc_ptr2len(buf);
        if trs_len > 1 {
            len -= trs_len;
        } else {
            let trs = transchar_byte(i32::from(crate::memory::cstr_first_byte(buf)));
            trs_len = crate::memory::cstr_len(trs) as i32;

            if trs_len > 1 {
                room -= trs_len - 1;
                if room <= 0 {
                    return;
                }
                unsafe {
                    ptr::copy(
                        buf.wrapping_add(1),
                        buf.wrapping_add(trs_len as usize),
                        len as usize,
                    )
                };
            }
            unsafe { ptr::copy(trs, buf, trs_len as usize) };
            len -= 1;
        }
        buf = buf.wrapping_add(trs_len as usize);
    }
}

pub fn transstr_len(s: impl crate::strings::IntoCCharPtr, untab: bool) -> usize {
    let s = s.into_c_char_ptr();
    {
        let mut p = s;
        let mut len: usize = 0;

        while !crate::memory::cstr_is_empty(p) {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                if vim_isprintc(utf_ptr2char(p)) {
                    len += l;
                } else {
                    let mut off: usize = 0;
                    while off < l {
                        let c = utf_ptr2char(p.wrapping_add(off));
                        let mut hexbuf = [0 as c_char; 9];
                        len += transchar_hex(hexbuf.as_mut_ptr(), c);
                        off += utf_ptr2len(p.wrapping_add(off)) as usize;
                    }
                }
                p = p.wrapping_add(l);
            } else if crate::memory::cstr_first_byte(p) == TAB && !untab {
                len += 1;
                p = p.wrapping_add(1);
            } else {
                let c = crate::memory::cstr_first_byte(p);
                p = p.wrapping_add(1);
                len += if c < 0x80 {
                    ascii2cells(i32::from(c)) as usize
                } else {
                    4
                };
            }
        }
        len
    }
}

pub fn transstr_buf(
    s: impl crate::strings::IntoCCharPtr,
    slen: isize,
    buf: *mut c_char,
    buflen: usize,
    untab: bool,
) -> usize {
    let s = s.into_c_char_ptr();
    {
        let mut p = s;
        let mut buf_p = buf;
        let buf_e = buf_p.wrapping_add(buflen - 1);

        while (slen < 0 || crate::memory::ptr_offset_from(p, s) < slen)
            && !crate::memory::cstr_is_empty(p)
            && buf_p < buf_e
        {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                if buf_p.wrapping_add(l) > buf_e {
                    break;
                }

                if vim_isprintc(utf_ptr2char(p)) {
                    unsafe { ptr::copy(p, buf_p, l) };
                    buf_p = buf_p.wrapping_add(l);
                } else {
                    let mut off: usize = 0;
                    while off < l {
                        let c = utf_ptr2char(p.wrapping_add(off));
                        let mut hexbuf = [0 as c_char; 9];
                        let hexlen = transchar_hex(hexbuf.as_mut_ptr(), c);
                        if buf_p.wrapping_add(hexlen) > buf_e {
                            break;
                        }
                        unsafe { ptr::copy_nonoverlapping(hexbuf.as_ptr(), buf_p, hexlen) };
                        buf_p = buf_p.wrapping_add(hexlen);
                        off += utf_ptr2len(p.wrapping_add(off)) as usize;
                    }
                }
                p = p.wrapping_add(l);
            } else if crate::memory::cstr_first_byte(p) == TAB && !untab {
                unsafe {
                    *buf_p = *p;
                }
                buf_p = buf_p.wrapping_add(1);
                p = p.wrapping_add(1);
            } else {
                let c = crate::memory::cstr_first_byte(p);
                p = p.wrapping_add(1);
                let tb = transchar_byte(i32::from(c));
                let tb_len = crate::memory::cstr_len(tb);
                if buf_p.wrapping_add(tb_len) > buf_e {
                    break;
                }
                unsafe { ptr::copy(tb, buf_p, tb_len) };
                buf_p = buf_p.wrapping_add(tb_len);
            }
        }
        unsafe {
            *buf_p = NUL;
        }
        assert!(buf_p <= buf_e);
        crate::memory::ptr_offset_from(buf_p, buf) as usize
    }
}

pub fn transstr(s: impl crate::strings::IntoCCharPtr, untab: bool) -> CString {
    let s = s.into_c_char_ptr();
    {
        let len = transstr_len(s, untab) + 1;
        let mut buf = vec![0 as c_char; len];
        let written = transstr_buf(s, -1, buf.as_mut_ptr(), len, untab);
        cstr_from_bytes(crate::memory::cstr_slice(buf.as_ptr(), written))
    }
}

pub fn kv_transstr(str_: &mut Vec<u8>, s: impl crate::strings::IntoCCharPtr, untab: bool) -> usize {
    let s = s.into_c_char_ptr();
    {
        if s.is_null() {
            return 0;
        }
        let len = transstr_len(s, untab);
        let old_len = str_.len();
        str_.resize(old_len + len + 1, 0);
        transstr_buf(
            s,
            -1,
            str_.as_mut_ptr().wrapping_add(old_len).cast::<c_char>(),
            len + 1,
            untab,
        );
        str_.truncate(old_len + len);
        len
    }
}

pub fn str_foldcase(
    str_: impl crate::strings::IntoCCharPtr,
    orglen: i32,
    buf: *mut c_char,
    buflen: i32,
) -> Option<CString> {
    let str_ = str_.into_c_char_ptr();
    {
        let mut vec: Vec<c_char> = Vec::new();
        let mut len = orglen;

        if buf.is_null() {
            vec.reserve_exact((len as usize) + 1);
            vec.extend_from_slice(unsafe { std::slice::from_raw_parts(str_, len as usize) });
            vec.push(NUL);
        } else {
            if len >= buflen {
                len = buflen - 1;
            }
            unsafe { ptr::copy(str_, buf, len as usize) };
            unsafe {
                *buf.wrapping_add(len as usize) = NUL;
            }
        }

        let str_ptr = |vec_ref: &mut Vec<c_char>, idx: usize| -> *mut c_char {
            if buf.is_null() {
                &raw mut vec_ref[idx]
            } else {
                buf.wrapping_add(idx)
            }
        };

        let mut i: usize = 0;
        while !crate::memory::cstr_is_empty(str_ptr(&mut vec, i)) {
            let p_cur = str_ptr(&mut vec, i);
            let c = utf_ptr2char(p_cur);
            let olen = utf_ptr2len(p_cur);
            let mut lc = mb_tolower(c);

            if (c < 0x80 || olen > 1) && c != lc {
                let mut nlen = utf_char2len(lc);
                if olen != nlen {
                    if nlen > olen {
                        if buf.is_null() {
                            vec.resize(vec.len() + (nlen - olen) as usize, 0);
                        } else if len + nlen - olen >= buflen {
                            lc = c;
                            nlen = olen;
                        }
                    }

                    if olen != nlen {
                        let base = str_ptr(&mut vec, i);
                        let src = base.wrapping_add(olen as usize);
                        let dst = base.wrapping_add(nlen as usize);
                        let tail_len = crate::memory::cstr_len(src) + 1;
                        unsafe { ptr::copy(src, dst, tail_len) };
                        if buf.is_null() {
                            if nlen < olen {
                                vec.truncate(vec.len() - (olen - nlen) as usize);
                            }
                        } else {
                            len += nlen - olen;
                        }
                    }
                }
                utf_char2bytes(lc, str_ptr(&mut vec, i));
            }

            i += utfc_ptr2len(str_ptr(&mut vec, i)) as usize;
        }

        if buf.is_null() {
            let out_len = crate::memory::cstr_len(vec.as_ptr());
            Some(cstr_from_bytes(unsafe {
                std::slice::from_raw_parts(vec.as_ptr().cast::<u8>(), out_len)
            }))
        } else {
            None
        }
    }
}

pub fn transchar(c: i32) -> *mut c_char {
    transchar_buf(Some(crate::globals::curbuf_mut()), c)
}

pub fn transchar_buf(buf: Option<&FileBuf>, mut c: i32) -> *mut c_char {
    let charbuf_ptr = (&raw mut charset_st().transchar_charbuf).cast::<u8>();
    let mut i: usize = 0;
    if is_special(c) {
        unsafe {
            *charbuf_ptr = b'~';
        }
        unsafe {
            *charbuf_ptr.wrapping_add(1) = b'@';
        }
        i = 2;
        c = i32::from(k_second(c));
    }

    if c <= 0xFF && vim_isprintc(c) {
        unsafe {
            *charbuf_ptr.wrapping_add(i) = c as u8;
        }
        unsafe {
            *charbuf_ptr.wrapping_add(i + 1) = 0;
        }
    } else if c <= 0xFF {
        transchar_nonprint(buf, charbuf_ptr.wrapping_add(i).cast::<c_char>(), c);
    } else {
        transchar_hex(charbuf_ptr.wrapping_add(i).cast::<c_char>(), c);
    }
    charbuf_ptr.cast::<c_char>()
}

pub fn transchar_byte(c: i32) -> *mut c_char {
    transchar_byte_buf(Some(crate::globals::curbuf_mut()), c)
}

pub fn transchar_byte_buf(buf: Option<&FileBuf>, c: i32) -> *mut c_char {
    if c >= 0x80 {
        let charbuf_ptr = (&raw mut charset_st().transchar_charbuf).cast::<c_char>();
        transchar_nonprint(buf, charbuf_ptr, c);
        return charbuf_ptr;
    }
    transchar_buf(buf, c)
}

pub fn transchar_nonprint(buf: Option<&FileBuf>, charbuf: *mut c_char, mut c: i32) {
    if c == i32::from(NL) {
        c = 0;
    } else if buf.is_some() && c == CAR && get_fileformat(buf.unwrap()) == EOL_MAC {
        c = i32::from(NL);
    }
    assert!(c <= 0xff);

    if (opt_vars().dy_flags & K_OPT_DY_FLAG_UHEX) != 0 || c > 0x7f {
        transchar_hex(charbuf, c);
    } else {
        unsafe {
            *charbuf = b'^' as c_char;
        }
        unsafe {
            *charbuf.wrapping_add(1) = ((c as u8) ^ 0x40) as c_char;
        }
        unsafe {
            *charbuf.wrapping_add(2) = NUL;
        }
    }
}

pub fn transchar_hex(buf: *mut c_char, c: i32) -> usize {
    unsafe {
        let mut i: usize = 0;
        let uc = c as u32;

        *buf.wrapping_add(i) = b'<' as c_char;
        i += 1;
        if c > 0xFF {
            if c > 0xFFFF {
                *buf.wrapping_add(i) = nr2hex(uc >> 20) as c_char;
                i += 1;
                *buf.wrapping_add(i) = nr2hex(uc >> 16) as c_char;
                i += 1;
            }
            *buf.wrapping_add(i) = nr2hex(uc >> 12) as c_char;
            i += 1;
            *buf.wrapping_add(i) = nr2hex(uc >> 8) as c_char;
            i += 1;
        }
        *buf.wrapping_add(i) = nr2hex(uc >> 4) as c_char;
        i += 1;
        *buf.wrapping_add(i) = nr2hex(uc) as c_char;
        i += 1;
        *buf.wrapping_add(i) = b'>' as c_char;
        i += 1;
        *buf.wrapping_add(i) = NUL;
        i
    }
}

pub fn rl_mirror_ascii(str_: *mut c_char, end: *mut c_char) {
    let mut p1 = str_;
    let mut p2 = if !end.is_null() {
        end
    } else {
        str_.wrapping_add(crate::memory::cstr_len(str_))
    }
    .wrapping_sub(1);
    while p1 < p2 {
        unsafe { ptr::swap(p1, p2) };
        p1 = p1.wrapping_add(1);
        p2 = p2.wrapping_sub(1);
    }
}

pub fn char2cells(c: i32) -> i32 {
    if c >= 0x80 {
        return utf_char2cells(c);
    }
    ascii2cells(c)
}

pub fn vim_strsize(s: impl crate::strings::IntoCCharPtr) -> i32 {
    let s = s.into_c_char_ptr();
    vim_strnsize(s, MAXCOL)
}

pub fn vim_strnsize(s: impl crate::strings::IntoCCharPtr, mut len: i32) -> i32 {
    let s = s.into_c_char_ptr();
    {
        assert!(!s.is_null());
        let mut size: i32 = 0;
        let mut ci = utf_ptr2_str_char_info(s);
        while !crate::memory::cstr_is_empty(ci.ptr) && len > 0 {
            let cli = utf_cluster_info(ci);
            size += cli.cells;
            len -= crate::memory::ptr_offset_from(cli.next.ptr, ci.ptr) as i32;
            ci = cli.next;
        }
        size
    }
}

pub fn vim_is_idc(c: i32) -> bool {
    c > 0 && c < 0x100 && get_chartab((&raw const charset_st().isi_chartab).cast::<u64>(), c) != 0
}

pub fn vim_iswordc(c: i32) -> bool {
    vim_iswordc_buf(c, curbuf_mut())
}

pub fn vim_iswordc_tab(c: i32, chartab: *const u64) -> bool {
    if c >= 0x100 {
        utf_class_tab(c, chartab) >= 2
    } else {
        c > 0 && get_chartab(chartab, c) != 0
    }
}

pub fn vim_iswordc_buf(c: i32, buf: &mut FileBuf) -> bool {
    vim_iswordc_tab(c, buf.b_chartab.as_ptr())
}

pub fn vim_iswordp(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr();
    vim_iswordp_buf(p, crate::globals::curbuf_mut())
}

pub fn vim_iswordp_buf(p: impl crate::strings::IntoCCharPtr, buf: &mut FileBuf) -> bool {
    let p = p.into_c_char_ptr();
    {
        let mut c = i32::from(crate::memory::cstr_first_byte(p));
        if UTF8LEN_TAB[c as usize] > 1 {
            c = utf_ptr2char(p);
        }
        vim_iswordc_buf(c, buf)
    }
}

pub fn vim_isfilec(c: i32) -> bool {
    c >= 0x100
        || (c > 0 && get_chartab((&raw const charset_st().isf_chartab).cast::<u64>(), c) != 0)
}

pub fn vim_is_fname_char(c: i32) -> bool {
    vim_isfilec(c) || c == b',' as i32 || c == b' ' as i32 || c == b'@' as i32 || c == b':' as i32
}

pub fn vim_isfilec_or_wc(c: i32) -> bool {
    let buf: [c_char; 2] = [c as c_char, NUL];
    vim_isfilec(c) || c == b']' as i32 || path_has_wildcard(buf.as_ptr(), true)
}

pub fn vim_isprintc(c: i32) -> bool {
    if c >= 0x100 {
        return utf_printable(c);
    }
    c > 0 && (c & 0x60) != 0 && c != 0x7f
}

pub fn skipwhite(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr();
    {
        while ascii_iswhite(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
        }
        p.cast_mut()
    }
}

pub fn skipwhite_len(p: impl crate::strings::IntoCCharPtr, mut len: usize) -> *mut c_char {
    let mut p = p.into_c_char_ptr();
    {
        while len > 0 && ascii_iswhite(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
            len -= 1;
        }
        p.cast_mut()
    }
}

pub fn getwhitecols_curline() -> isize {
    getwhitecols(get_cursor_line_ptr())
}

pub fn getwhitecols(p: impl crate::strings::IntoCCharPtr) -> isize {
    let p = p.into_c_char_ptr();
    crate::memory::ptr_offset_from(skipwhite(p), p)
}

pub fn skipdigits(q: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let q = q.into_c_char_ptr();
    {
        let mut p = q;
        while ascii_isdigit(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
        }
        p.cast_mut()
    }
}

pub fn skipbin(q: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let q = q.into_c_char_ptr();
    {
        let mut p = q;
        while ascii_isbdigit(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
        }
        p
    }
}

pub fn skiphex(q: *mut c_char) -> *mut c_char {
    let mut p = q;
    while ascii_isxdigit(crate::memory::cstr_first_char(p)) {
        p = p.wrapping_add(1);
    }
    p
}

pub fn skiptodigit(q: *mut c_char) -> *mut c_char {
    let mut p = q;
    while !crate::memory::cstr_is_empty(p) && !ascii_isdigit(crate::memory::cstr_first_char(p)) {
        p = p.wrapping_add(1);
    }
    p
}

pub fn skiptobin(q: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let q = q.into_c_char_ptr();
    {
        let mut p = q;
        while !crate::memory::cstr_is_empty(p) && !ascii_isbdigit(crate::memory::cstr_first_char(p))
        {
            p = p.wrapping_add(1);
        }
        p
    }
}

pub fn skiptohex(q: *mut c_char) -> *mut c_char {
    let mut p = q;
    while !crate::memory::cstr_is_empty(p) && !ascii_isxdigit(crate::memory::cstr_first_char(p)) {
        p = p.wrapping_add(1);
    }
    p
}

pub fn skiptowhite(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr();
    {
        while crate::memory::cstr_first_byte(p) != b' '
            && crate::memory::cstr_first_byte(p) != b'\t'
            && !crate::memory::cstr_is_empty(p)
        {
            p = p.wrapping_add(1);
        }
        p.cast_mut()
    }
}

pub fn skiptowhite_esc(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr();
    {
        while crate::memory::cstr_first_byte(p) != b' '
            && crate::memory::cstr_first_byte(p) != b'\t'
            && !crate::memory::cstr_is_empty(p)
        {
            if (crate::memory::cstr_first_byte(p) == b'\\' || unsafe { *p } == CTRL_V_CONST)
                && crate::memory::cstr_byte_at(p, 1) != 0
            {
                p = p.wrapping_add(1);
            }
            p = p.wrapping_add(1);
        }
        p.cast_mut()
    }
}

pub fn skip_to_newline(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let p = p.into_c_char_ptr();
    {
        let _s = p;
        let _b = crate::memory::cstr_as_bytes(_s);
        let _off = _b
            .iter()
            .position(|&_x| _x == (NL) as u8)
            .unwrap_or(_b.len());
        _s.wrapping_add(_off).cast_mut()
    }
}

pub fn try_getdigits(pp: *mut *mut c_char, nr: &mut i64) -> bool {
    unsafe {
        let orig = *pp;
        let mut p = orig.cast::<u8>();

        while matches!(*p, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            p = p.wrapping_add(1);
        }

        let mut neg = false;
        if *p == b'-' {
            neg = true;
            p = p.wrapping_add(1);
        } else if *p == b'+' {
            p = p.wrapping_add(1);
        }

        if !(*p).is_ascii_digit() {
            *pp = orig;
            *nr = 0;
            return true;
        }

        let mut acc: u64 = 0;
        let mut overflow = false;
        while (*p).is_ascii_digit() {
            let d = u64::from(*p - b'0');
            if let Some(next) = acc.checked_mul(10).and_then(|v| v.checked_add(d)) {
                acc = next;
            } else {
                overflow = true;
            }
            p = p.wrapping_add(1);
        }

        *pp = p.cast::<c_char>();
        if neg {
            const MIN_MAG: u64 = (i64::MAX as u64) + 1;
            if overflow || acc > MIN_MAG {
                *nr = i64::MIN;
                return false;
            } else if acc == MIN_MAG {
                *nr = i64::MIN;
            } else {
                *nr = -(acc as i64);
            }
        } else {
            if overflow || acc > (i64::MAX as u64) {
                *nr = i64::MAX;
                return false;
            }
            *nr = acc as i64;
        }
        true
    }
}

#[inline]
pub fn parse_i64_prefix(bytes: &[u8]) -> i64 {
    let mut s = bytes;
    while let Some((&b, rest)) = s.split_first() {
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            s = rest;
        } else {
            break;
        }
    }
    let neg = if let Some((&b'-', rest)) = s.split_first() {
        s = rest;
        true
    } else {
        if let Some((&b'+', rest)) = s.split_first() {
            s = rest;
        }
        false
    };
    let mut acc: i64 = 0;
    while let Some((&b, rest)) = s.split_first() {
        if b.is_ascii_digit() {
            acc = acc.wrapping_mul(10).wrapping_add(i64::from(b - b'0'));
            s = rest;
        } else {
            break;
        }
    }
    if neg { acc.wrapping_neg() } else { acc }
}

pub fn getdigits(pp: *mut *mut c_char, strict: bool, def: i64) -> i64 {
    let mut number: i64 = 0;
    let ok = try_getdigits(pp, &mut number);
    if strict && !ok {
        std::process::abort();
    }
    if ok { number } else { def }
}

pub fn getdigits_int(pp: *mut *mut c_char, strict: bool, def: i32) -> i32 {
    let number = getdigits(pp, strict, i64::from(def));
    match i32::try_from(number) {
        Ok(v) => v,
        Err(_) => {
            assert!(!strict);
            def
        }
    }
}

pub fn getdigits_long(pp: *mut *mut c_char, strict: bool, def: i64) -> i64 {
    getdigits(pp, strict, def)
}

pub fn getdigits_int32(pp: *mut *mut c_char, strict: bool, def: i32) -> i32 {
    let number = getdigits(pp, strict, i64::from(def));
    match i32::try_from(number) {
        Ok(v) => v,
        Err(_) => {
            assert!(!strict);
            def
        }
    }
}

pub fn vim_isblankline(lbuf: impl crate::strings::IntoCCharPtr) -> bool {
    let lbuf = lbuf.into_c_char_ptr();
    {
        let p = skipwhite(lbuf);
        crate::memory::cstr_is_empty(p)
            || crate::memory::cstr_first_byte(p) == b'\r'
            || crate::memory::cstr_first_byte(p) == b'\n'
    }
}

pub fn vim_slice2nr(
    bytes: &[u8],
    prep: Option<&mut i32>,
    len: Option<&mut i32>,
    what: i32,
    nptr: Option<&mut Varnumber>,
    unptr: Option<&mut Uvarnumber>,
    strict: bool,
    overflow: Option<&mut bool>,
) {
    vim_str2nr(
        bytes.as_ptr().cast::<c_char>(),
        prep,
        len,
        what,
        nptr,
        unptr,
        bytes.len() as i32,
        strict,
        overflow,
    );
}

pub fn vim_str2nr(
    start: impl crate::strings::IntoCCharPtr,
    prep: Option<&mut i32>,
    mut len: Option<&mut i32>,
    what: i32,
    nptr: Option<&mut Varnumber>,
    unptr: Option<&mut Uvarnumber>,
    maxlen: i32,
    strict: bool,
    mut overflow: Option<&mut bool>,
) {
    let start = start.into_c_char_ptr();
    {
        let mut ptr = start;
        let string_ended = |p: *const c_char| -> bool {
            !(maxlen == 0 || (crate::memory::ptr_offset_from(p, start) as i32) < maxlen)
        };
        let mut pre: i32 = 0;
        let negative = crate::memory::cstr_first_byte(ptr) == b'-';
        let mut un: Uvarnumber = 0;

        if let Some(len) = len.as_deref_mut() {
            *len = 0;
        }

        if negative {
            ptr = ptr.wrapping_add(1);
        }

        let base: Uvarnumber;
        if (what & STR2NR_FORCE) != 0 {
            match what & !(STR2NR_FORCE | STR2NR_QUOTE) {
                STR2NR_HEX => {
                    if !string_ended(ptr.wrapping_add(2))
                        && crate::memory::cstr_first_byte(ptr) == b'0'
                        && (crate::memory::cstr_byte_at(ptr, 1) == b'x'
                            || crate::memory::cstr_byte_at(ptr, 1) == b'X')
                        && ascii_isxdigit(crate::memory::cstr_char_at(ptr, 2))
                    {
                        ptr = ptr.wrapping_add(2);
                    }
                    base = 16;
                }
                STR2NR_BIN => {
                    if !string_ended(ptr.wrapping_add(2))
                        && crate::memory::cstr_first_byte(ptr) == b'0'
                        && (crate::memory::cstr_byte_at(ptr, 1) == b'b'
                            || crate::memory::cstr_byte_at(ptr, 1) == b'B')
                        && ascii_isbdigit(crate::memory::cstr_char_at(ptr, 2))
                    {
                        ptr = ptr.wrapping_add(2);
                    }
                    base = 2;
                }
                x if x == STR2NR_OCT || x == STR2NR_OOCT || x == (STR2NR_OCT | STR2NR_OOCT) => {
                    if !string_ended(ptr.wrapping_add(2))
                        && crate::memory::cstr_first_byte(ptr) == b'0'
                        && (crate::memory::cstr_byte_at(ptr, 1) == b'o'
                            || crate::memory::cstr_byte_at(ptr, 1) == b'O')
                        && ascii_isodigit(crate::memory::cstr_char_at(ptr, 2))
                    {
                        ptr = ptr.wrapping_add(2);
                    }
                    base = 8;
                }
                0 => {
                    base = 10;
                }
                _ => std::process::abort(),
            }
        } else if (what & (STR2NR_HEX | STR2NR_OCT | STR2NR_OOCT | STR2NR_BIN)) != 0
            && !string_ended(ptr.wrapping_add(1))
            && crate::memory::cstr_first_byte(ptr) == b'0'
            && crate::memory::cstr_byte_at(ptr, 1) != b'8'
            && crate::memory::cstr_byte_at(ptr, 1) != b'9'
        {
            pre = i32::from(crate::memory::cstr_byte_at(ptr, 1));
            if (what & STR2NR_HEX) != 0
                && !string_ended(ptr.wrapping_add(2))
                && (pre == b'X' as i32 || pre == b'x' as i32)
                && ascii_isxdigit(crate::memory::cstr_char_at(ptr, 2))
            {
                ptr = ptr.wrapping_add(2);
                base = 16;
            } else if (what & STR2NR_BIN) != 0
                && !string_ended(ptr.wrapping_add(2))
                && (pre == b'B' as i32 || pre == b'b' as i32)
                && ascii_isbdigit(crate::memory::cstr_char_at(ptr, 2))
            {
                ptr = ptr.wrapping_add(2);
                base = 2;
            } else if (what & STR2NR_OOCT) != 0
                && !string_ended(ptr.wrapping_add(2))
                && (pre == b'O' as i32 || pre == b'o' as i32)
                && ascii_isodigit(crate::memory::cstr_char_at(ptr, 2))
            {
                ptr = ptr.wrapping_add(2);
                base = 8;
            } else {
                pre = 0;
                if (what & STR2NR_OCT) == 0 || !ascii_isodigit(crate::memory::cstr_char_at(ptr, 1))
                {
                    base = 10;
                } else {
                    let mut is_oct = true;
                    let mut i: usize = 2;
                    while !string_ended(ptr.wrapping_add(i))
                        && ascii_isdigit(crate::memory::cstr_char_at(ptr, i))
                    {
                        if (crate::memory::cstr_byte_at(ptr, i)) > b'7' {
                            is_oct = false;
                            break;
                        }
                        i += 1;
                    }
                    if is_oct {
                        pre = b'0' as i32;
                        base = 8;
                    } else {
                        base = 10;
                    }
                }
            }
        } else {
            base = 10;
        }

        let after_prefix = ptr;
        while !string_ended(ptr) {
            let check_cond = |c: i32| -> bool {
                match base {
                    2 => c == b'0' as i32 || c == b'1' as i32,
                    8 => ascii_isodigit(c),
                    10 => ascii_isdigit(c),
                    16 => ascii_isxdigit(c),
                    _ => unreachable!(),
                }
            };
            if (what & STR2NR_QUOTE) != 0
                && ptr > after_prefix
                && crate::memory::cstr_first_byte(ptr) == b'\''
            {
                ptr = ptr.wrapping_add(1);
                if !string_ended(ptr) && check_cond(crate::memory::cstr_first_char(ptr)) {
                    continue;
                }
                ptr = ptr.wrapping_sub(1);
            }
            let c = crate::memory::cstr_first_char(ptr);
            if !check_cond(c) {
                break;
            }
            let digit = if base == 16 {
                hex2nr(c) as Uvarnumber
            } else {
                (c - b'0' as i32) as Uvarnumber
            };
            if un < UVARNUMBER_MAX / base
                || (un == UVARNUMBER_MAX / base && (base != 10 || digit <= UVARNUMBER_MAX % 10))
            {
                un = base * un + digit;
            } else {
                un = UVARNUMBER_MAX;
                if let Some(overflow) = overflow.as_deref_mut() {
                    *overflow = true;
                }
            }
            ptr = ptr.wrapping_add(1);
        }

        if strict
            && (crate::memory::ptr_offset_from(ptr, start) as i32) != maxlen
            && ascii_isalnum(crate::memory::cstr_first_char(ptr))
        {
            return;
        }

        if let Some(prep) = prep {
            *prep = pre;
        }

        if let Some(len) = len {
            *len = crate::memory::ptr_offset_from(ptr, start) as i32;
        }

        if let Some(nptr) = nptr {
            if negative {
                if un > VARNUMBER_MAX {
                    *nptr = VARNUMBER_MIN;
                    if let Some(overflow) = overflow.as_deref_mut() {
                        *overflow = true;
                    }
                } else {
                    *nptr = -(un as Varnumber);
                }
            } else {
                if un > VARNUMBER_MAX {
                    un = VARNUMBER_MAX;
                    if let Some(overflow) = overflow {
                        *overflow = true;
                    }
                }
                *nptr = un as Varnumber;
            }
        }

        if let Some(unptr) = unptr {
            *unptr = un;
        }
    }
}

pub const fn hex2nr(c: i32) -> i32 {
    if c >= b'a' as i32 && c <= b'f' as i32 {
        c - b'a' as i32 + 10
    } else if c >= b'A' as i32 && c <= b'F' as i32 {
        c - b'A' as i32 + 10
    } else {
        c - b'0' as i32
    }
}

pub fn hexhex2nr(p: impl crate::strings::IntoCCharPtr) -> i32 {
    let p = p.into_c_char_ptr();
    {
        if !ascii_isxdigit(crate::memory::cstr_first_char(p))
            || !ascii_isxdigit(crate::memory::cstr_char_at(p, 1))
        {
            return -1;
        }
        (hex2nr(crate::memory::cstr_first_char(p)) << 4) + hex2nr(crate::memory::cstr_char_at(p, 1))
    }
}

pub fn rem_backslash(str_: impl crate::strings::IntoCCharPtr) -> bool {
    let str_ = str_.into_c_char_ptr();
    {
        #[cfg(windows)]
        {
            let b0 = crate::memory::cstr_first_byte(str_);
            let b1 = crate::memory::cstr_byte_at(str_, 1);
            b0 == b'\\'
                && b1 < 0x80
                && (b1 == b' '
                    || (b1 != 0 && b1 != b'*' && b1 != b'?' && !vim_isfilec(i32::from(b1))))
        }
        #[cfg(not(windows))]
        {
            crate::memory::cstr_first_byte(str_) == b'\\'
                && crate::memory::cstr_byte_at(str_, 1) != 0
        }
    }
}

pub fn backslash_halve(mut p: *mut c_char) {
    unsafe {
        while !crate::memory::cstr_is_empty(p) && !rem_backslash(p) {
            p = p.wrapping_add(1);
        }
        if !crate::memory::cstr_is_empty(p) {
            let mut dst = p;
            *dst = *p.wrapping_add(1);
            dst = dst.wrapping_add(1);
            p = p.wrapping_add(2);
            while !crate::memory::cstr_is_empty(p) {
                if rem_backslash(p) {
                    *dst = *p.wrapping_add(1);
                    dst = dst.wrapping_add(1);
                    p = p.wrapping_add(2);
                } else {
                    *dst = *p;
                    dst = dst.wrapping_add(1);
                    p = p.wrapping_add(1);
                }
            }
            *dst = NUL;
        }
    }
}

pub fn backslash_halve_save(p: impl crate::strings::IntoCCharPtr) -> CString {
    let mut p = p.into_c_char_ptr();
    {
        let mut res = Vec::with_capacity(crate::memory::cstr_len(p));
        while !crate::memory::cstr_is_empty(p) {
            if rem_backslash(p) {
                res.push(crate::memory::cstr_byte_at(p, 1));
                p = p.wrapping_add(2);
            } else {
                res.push(crate::memory::cstr_first_byte(p));
                p = p.wrapping_add(1);
            }
        }
        cstr_from_bytes(&res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::STR2NR_ALL;
    use crate::keycodes::{K_UP, K_ZERO, KE_FILLER};
    use std::ffi::CStr;

    #[test]
    fn to_special_handles_ks_special_and_ks_zero() {
        assert_eq!(to_special(KS_SPECIAL, KE_FILLER as i32), K_SPECIAL);
        assert_eq!(to_special(KS_ZERO, KE_FILLER as i32), K_ZERO);
        assert_eq!(to_special(b'k' as i32, b'u' as i32), K_UP);
    }

    #[test]
    fn vim_str2nr_parses_dec_bin_oct_hex_and_quotes() {
        let check = |s: &CStr,
                     what: i32,
                     maxlen: i32,
                     strict: bool,
                     exp_len: i32,
                     exp_num: Option<Varnumber>,
                     exp_unum: Option<Uvarnumber>,
                     exp_pre: Option<i32>| {
            let mut pre: i32 = -99;
            let mut len: i32 = -99;
            let mut num: Varnumber = -99;
            let mut unum: Uvarnumber = 99;
            vim_str2nr(
                s.as_ptr(),
                Some(&mut pre),
                Some(&mut len),
                what,
                Some(&mut num),
                Some(&mut unum),
                maxlen,
                strict,
                None,
            );
            assert_eq!(len, exp_len, "s={s:?} what={what}");
            if let Some(en) = exp_num {
                assert_eq!(num, en, "s={s:?} what={what}");
            }
            if let Some(eu) = exp_unum {
                assert_eq!(unum, eu, "s={s:?} what={what}");
            }
            if let Some(ep) = exp_pre {
                assert_eq!(pre, ep, "s={s:?} what={what}");
            }
        };

        check(c"", STR2NR_ALL, 0, true, 0, Some(0), Some(0), Some(0));
        check(
            c"12345",
            STR2NR_ALL,
            0,
            true,
            5,
            Some(12345),
            Some(12345),
            Some(0),
        );
        check(c"12345A", STR2NR_ALL, 0, true, 0, None, None, None);
        check(
            c"12345A",
            STR2NR_ALL,
            0,
            false,
            5,
            Some(12345),
            Some(12345),
            Some(0),
        );
        check(c"-42", STR2NR_ALL, 3, true, 3, Some(-42), Some(42), Some(0));
        check(
            c"0b101",
            STR2NR_ALL,
            0,
            true,
            5,
            Some(5),
            Some(5),
            Some(b'b' as i32),
        );
        check(
            c"-0B101",
            STR2NR_ALL,
            0,
            true,
            6,
            Some(-5),
            Some(5),
            Some(b'B' as i32),
        );
        check(
            c"054",
            STR2NR_ALL,
            0,
            true,
            3,
            Some(44),
            Some(44),
            Some(b'0' as i32),
        );
        check(
            c"0o054",
            STR2NR_ALL,
            0,
            true,
            5,
            Some(44),
            Some(44),
            Some(b'o' as i32),
        );
        check(
            c"0xABCDEF",
            STR2NR_ALL,
            0,
            true,
            8,
            Some(11259375),
            Some(11259375),
            Some(b'x' as i32),
        );
        check(
            c"-0X101",
            STR2NR_ALL,
            0,
            true,
            6,
            Some(-257),
            Some(257),
            Some(b'X' as i32),
        );
        check(
            c"1'2'3'4",
            STR2NR_QUOTE,
            0,
            true,
            7,
            Some(1234),
            Some(1234),
            Some(0),
        );
        check(
            c"7''331",
            STR2NR_QUOTE,
            0,
            true,
            1,
            Some(7),
            Some(7),
            Some(0),
        );
        check(
            c"0xab''cd",
            STR2NR_HEX | STR2NR_QUOTE,
            0,
            true,
            4,
            Some(171),
            Some(171),
            Some(b'x' as i32),
        );
    }
}
