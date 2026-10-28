//! Code related to character sets (`src/nvim/charset.c`).

use crate::buffer::buf_T;
use crate::cursor::get_cursor_line_ptr;
use crate::globals::*;
use crate::mbyte::{
    mb_islower, mb_isupper, mb_ptr2char_adv, mb_tolower, utf_ClusterInfo, utf_char2bytes,
    utf_char2cells, utf_char2len, utf_class_tab, utf_printable, utf_ptr2StrCharInfo, utf_ptr2char,
    utf_ptr2len, utf8len_tab, utfc_ptr2len,
};
use crate::memory::{cstr_from_bytes};
use crate::option::{get_fileformat, skip_to_option_part};
use crate::option_vars::*;
use crate::path::path_has_wildcard;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint};
use std::ptr;

use crate::change::TAB;
use crate::eval::typval::varnumber_T;
use crate::keycodes::CAR;
use crate::math::{FAIL, OK};
use crate::regexp::MAXCOL;
pub type uvarnumber_T = u64;

pub(crate) const NUL: c_char = 0;
pub(crate) const NL: c_char = b'\n' as c_char;

pub trait AsciiInt {
    fn into_ascii_int(self) -> c_int;
}

impl AsciiInt for u8 {
    #[inline]
    fn into_ascii_int(self) -> c_int {
        c_int::from(self)
    }
}

impl AsciiInt for i8 {
    #[inline]
    fn into_ascii_int(self) -> c_int {
        c_int::from(self)
    }
}

impl AsciiInt for c_int {
    #[inline]
    fn into_ascii_int(self) -> c_int {
        self
    }
}

#[inline]
pub fn ascii_iswhite<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ch == b' ' as c_int || ch == b'\t' as c_int
}

#[inline]
pub fn ascii_isdigit<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'0' as c_int..=b'9' as c_int).contains(&ch)
}

#[inline]
pub fn ascii_isalpha<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'a' as c_int..=b'z' as c_int).contains(&ch) || (b'A' as c_int..=b'Z' as c_int).contains(&ch)
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
        || (b'a' as c_int..=b'f' as c_int).contains(&ch)
        || (b'A' as c_int..=b'F' as c_int).contains(&ch)
}

#[inline]
pub fn ascii_isspace<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (9..=13).contains(&ch) || ch == b' ' as c_int
}

#[inline]
pub fn ascii_isupper<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'A' as c_int..=b'Z' as c_int).contains(&ch)
}

#[inline]
pub fn ascii_islower<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    (b'a' as c_int..=b'z' as c_int).contains(&ch)
}

#[inline]
pub fn ascii_iswhite_or_nul<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_iswhite(ch) || ch == 0
}

#[inline]
pub fn ascii_isident<C: AsciiInt>(c: C) -> bool {
    let ch = c.into_ascii_int();
    ascii_isalnum(ch) || ch == b'_' as c_int
}

#[inline]
pub const fn toupper_asc(c: c_int) -> c_int {
    if c >= b'a' as c_int && c <= b'z' as c_int {
        c - 0x20
    } else {
        c
    }
}

#[inline]
pub const fn tolower_asc(c: c_int) -> c_int {
    if c >= b'A' as c_int && c <= b'Z' as c_int {
        c + 0x20
    } else {
        c
    }
}

#[inline]
pub const fn is_special(c: c_int) -> bool {
    c < 0
}

#[inline]
pub const fn to_special(a: c_int, b: c_int) -> c_int {
    if a == KS_SPECIAL {
        K_SPECIAL
    } else if a == KS_ZERO {
        crate::keycodes::K_ZERO
    } else {
        crate::keycodes::termcap2key(a, b)
    }
}

pub(crate) const Ctrl_V: c_char = 22;

pub(crate) const EOL_MAC: c_int = 2;
pub(crate) const kOptDyFlagUhex: c_uint = 0x04;

const K_SPECIAL: c_int = 0x80;
const KS_SPECIAL: c_int = 254;
const KS_ZERO: c_int = 255;

pub(crate) const STR2NR_BIN: c_int = 1 << 0;
pub(crate) const STR2NR_OCT: c_int = 1 << 1;
pub(crate) const STR2NR_HEX: c_int = 1 << 2;
pub(crate) const STR2NR_OOCT: c_int = 1 << 3;
pub(crate) const STR2NR_QUOTE: c_int = 1 << 4;
pub(crate) const STR2NR_FORCE: c_int = 1 << 7;

const VARNUMBER_MAX: uvarnumber_T = i64::MAX as u64;
pub(crate) const VARNUMBER_MIN: varnumber_T = i64::MIN;
const UVARNUMBER_MAX: uvarnumber_T = u64::MAX;

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

static CHARSET_STATE: crate::globals::GlobalState<CharsetModuleState> = crate::globals::GlobalState::new(CharsetModuleState::new());

#[inline]
fn charset_st() -> &'static mut CharsetModuleState {
    unsafe { &mut *CHARSET_STATE.as_ptr() }
}

#[inline]
fn set_chartab(chartab: *mut u64, c: c_int) {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.add(idx) |= 1u64 << (c & 0x3f);
    }
}

#[inline]
fn reset_chartab(chartab: *mut u64, c: c_int) {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.add(idx) &= !(1u64 << (c & 0x3f));
    }
}

#[inline]
fn get_chartab(chartab: *const u64, c: c_int) -> u64 {
    unsafe {
        let idx = ((c as u32) >> 6) as usize;
        *chartab.add(idx) & (1u64 << (c & 0x3f))
    }
}

#[inline]
pub const fn ascii_isbdigit(c: c_int) -> bool {
    c == b'0' as c_int || c == b'1' as c_int
}

#[inline]
const fn ascii_isodigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'7' as c_int
}

#[inline]
pub fn ascii2cells(b: c_int) -> c_int {
    if (b' ' as c_int..=b'~' as c_int).contains(&b) {
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

pub fn buf_init_isk_chartab(buf: &mut buf_T) -> c_int {
    let save_chartab = buf.b_chartab;
        buf.b_chartab = [0; 4];
        if buf.b_p_lisp != 0 {
            set_chartab(buf.b_chartab.as_mut_ptr(), b'-' as c_int);
        }
        if parse_isopt(buf.b_p_isk, buf.b_chartab.as_mut_ptr()) == FAIL {
            buf.b_chartab = save_chartab;
            return FAIL;
        }
        OK
}

pub fn init_isf_chartab() -> c_int {
    let save_chartab = charset_st().isf_chartab;
        charset_st().isf_chartab = [0, 0, 0xFFFF_FFFF_0000_0000, 0xFFFF_FFFF_FFFF_FFFF];
        if parse_isopt(opt_vars().p_isf, (&raw mut charset_st().isf_chartab).cast::<u64>()) == FAIL {
            charset_st().isf_chartab = save_chartab;
            return FAIL;
        }
        OK
}

pub fn init_isi_chartab() -> c_int {
    let save_chartab = charset_st().isi_chartab;
        charset_st().isi_chartab = [0; 4];
        if parse_isopt(opt_vars().p_isi, (&raw mut charset_st().isi_chartab).cast::<u64>()) == FAIL {
            charset_st().isi_chartab = save_chartab;
            return FAIL;
        }
        OK
}

pub fn check_isopt(var: *mut c_char) -> c_int {
    parse_isopt(var, ptr::null_mut())
}

pub fn parse_isopt(var: *const c_char, chartab: *mut u64) -> c_int {
    unsafe {
        let mut p = var;

        while *p != NUL {
            let mut tilde = false;
            let mut do_isalpha = false;

            if *p == b'^' as c_char && *p.add(1) != NUL {
                tilde = true;
                p = p.add(1);
            }

            let mut c = if ascii_isdigit(*p as c_int) {
                getdigits_int((&raw mut p).cast::<*mut c_char>(), false, 0)
            } else {
                mb_ptr2char_adv(&raw mut p)
            };
            let mut c2: c_int = -1;

            if *p == b'-' as c_char && *p.add(1) != NUL {
                p = p.add(1);
                c2 = if ascii_isdigit(*p as c_int) {
                    getdigits_int((&raw mut p).cast::<*mut c_char>(), false, 0)
                } else {
                    mb_ptr2char_adv(&raw mut p)
                };
            }

            if c <= 0
                || c >= 256
                || (c2 < c && c2 != -1)
                || c2 >= 256
                || !(*p == NUL || *p == b',' as c_char)
            {
                return FAIL;
            }

            let trail_comma = *p == b',' as c_char;
            p = skip_to_option_part(p);
            if trail_comma && *p == NUL {
                return FAIL;
            }

            if chartab.is_null() {
                continue;
            }

            if c2 == -1 {
                if c == b'@' as c_int {
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

        OK
    }
}

pub fn trans_characters(mut buf: *mut c_char, bufsize: c_int) {
    unsafe {
        let mut len = CStr::from_ptr(buf).to_bytes().len() as c_int;
        let mut room = bufsize - len;

        while *buf != NUL {
            let mut trs_len = utfc_ptr2len(buf);
            if trs_len > 1 {
                len -= trs_len;
            } else {
                let trs = transchar_byte(c_int::from(*buf as u8));
                trs_len = CStr::from_ptr(trs).to_bytes().len() as c_int;

                if trs_len > 1 {
                    room -= trs_len - 1;
                    if room <= 0 {
                        return;
                    }
                    ptr::copy(buf.add(1), buf.add(trs_len as usize), len as usize);
                }
                ptr::copy(trs, buf, trs_len as usize);
                len -= 1;
            }
            buf = buf.add(trs_len as usize);
        }
    }
}

pub fn transstr_len(s: *const c_char, untab: bool) -> usize {
    unsafe {
        let mut p = s;
        let mut len: usize = 0;

        while *p != NUL {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                if vim_isprintc(utf_ptr2char(p)) {
                    len += l;
                } else {
                    let mut off: usize = 0;
                    while off < l {
                        let c = utf_ptr2char(p.add(off));
                        let mut hexbuf = [0 as c_char; 9];
                        len += transchar_hex(hexbuf.as_mut_ptr(), c);
                        off += utf_ptr2len(p.add(off)) as usize;
                    }
                }
                p = p.add(l);
            } else if *p == TAB && !untab {
                len += 1;
                p = p.add(1);
            } else {
                let c = *p as u8;
                p = p.add(1);
                len += if c < 0x80 {
                    ascii2cells(c_int::from(c)) as usize
                } else {
                    4
                };
            }
        }
        len
    }
}

pub fn transstr_buf(
    s: *const c_char,
    slen: isize,
    buf: *mut c_char,
    buflen: usize,
    untab: bool,
) -> usize {
    unsafe {
        let mut p = s;
        let mut buf_p = buf;
        let buf_e = buf_p.add(buflen - 1);

        while (slen < 0 || p.offset_from(s) < slen) && *p != NUL && buf_p < buf_e {
            let l = utfc_ptr2len(p) as usize;
            if l > 1 {
                if buf_p.add(l) > buf_e {
                    break;
                }

                if vim_isprintc(utf_ptr2char(p)) {
                    ptr::copy(p, buf_p, l);
                    buf_p = buf_p.add(l);
                } else {
                    let mut off: usize = 0;
                    while off < l {
                        let c = utf_ptr2char(p.add(off));
                        let mut hexbuf = [0 as c_char; 9];
                        let hexlen = transchar_hex(hexbuf.as_mut_ptr(), c);
                        if buf_p.add(hexlen) > buf_e {
                            break;
                        }
                        ptr::copy_nonoverlapping(hexbuf.as_ptr(), buf_p, hexlen);
                        buf_p = buf_p.add(hexlen);
                        off += utf_ptr2len(p.add(off)) as usize;
                    }
                }
                p = p.add(l);
            } else if *p == TAB && !untab {
                *buf_p = *p;
                buf_p = buf_p.add(1);
                p = p.add(1);
            } else {
                let c = *p as u8;
                p = p.add(1);
                let tb = transchar_byte(c_int::from(c));
                let tb_len = CStr::from_ptr(tb).to_bytes().len();
                if buf_p.add(tb_len) > buf_e {
                    break;
                }
                ptr::copy(tb, buf_p, tb_len);
                buf_p = buf_p.add(tb_len);
            }
        }
        *buf_p = NUL;
        assert!(buf_p <= buf_e);
        buf_p.offset_from(buf) as usize
    }
}

pub fn transstr(s: *const c_char, untab: bool) -> CString {
    unsafe {
        let len = transstr_len(s, untab) + 1;
        let mut buf = vec![0 as c_char; len];
        let written = transstr_buf(s, -1, buf.as_mut_ptr(), len, untab);
        cstr_from_bytes(std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), written))
    }
}

pub fn kv_transstr(str_: &mut Vec<u8>, s: *const c_char, untab: bool) -> usize {
    unsafe {
        if s.is_null() {
            return 0;
        }
        let len = transstr_len(s, untab);
        let old_len = str_.len();
        str_.resize(old_len + len + 1, 0);
        transstr_buf(
            s,
            -1,
            str_.as_mut_ptr().add(old_len).cast::<c_char>(),
            len + 1,
            untab,
        );
        str_.truncate(old_len + len);
        len
    }
}

pub fn str_foldcase(
    str_: *mut c_char,
    orglen: c_int,
    buf: *mut c_char,
    buflen: c_int,
) -> Option<CString> {
    unsafe {
        let mut vec: Vec<c_char> = Vec::new();
        let mut len = orglen;

        if buf.is_null() {
            vec.reserve_exact((len as usize) + 1);
            vec.extend_from_slice(std::slice::from_raw_parts(str_, len as usize));
            vec.push(NUL);
        } else {
            if len >= buflen {
                len = buflen - 1;
            }
            ptr::copy(str_, buf, len as usize);
            *buf.add(len as usize) = NUL;
        }

        let str_ptr = |vec_ref: &mut Vec<c_char>, idx: usize| -> *mut c_char {
            if buf.is_null() {
                &raw mut vec_ref[idx]
            } else {
                buf.add(idx)
            }
        };

        let mut i: usize = 0;
        while *str_ptr(&mut vec, i) != NUL {
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
                        let src = base.add(olen as usize);
                        let dst = base.add(nlen as usize);
                        let tail_len = CStr::from_ptr(src).to_bytes().len() + 1;
                        ptr::copy(src, dst, tail_len);
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
            let out_len = CStr::from_ptr(vec.as_ptr()).to_bytes().len();
            Some(cstr_from_bytes(std::slice::from_raw_parts(
                vec.as_ptr().cast::<u8>(),
                out_len,
            )))
        } else {
            None
        }
    }
}

pub fn transchar(c: c_int) -> *mut c_char {
    transchar_buf(g_st().curbuf, c)
}

pub fn transchar_buf(buf: *const buf_T, mut c: c_int) -> *mut c_char {
    unsafe {
        let charbuf_ptr = (&raw mut charset_st().transchar_charbuf).cast::<u8>();
        let mut i: usize = 0;
        if is_special(c) {
            *charbuf_ptr = b'~';
            *charbuf_ptr.add(1) = b'@';
            i = 2;
            c = c_int::from(k_second(c));
        }

        if c <= 0xFF && vim_isprintc(c) {
            *charbuf_ptr.add(i) = c as u8;
            *charbuf_ptr.add(i + 1) = 0;
        } else if c <= 0xFF {
            transchar_nonprint(buf, charbuf_ptr.add(i).cast::<c_char>(), c);
        } else {
            transchar_hex(charbuf_ptr.add(i).cast::<c_char>(), c);
        }
        charbuf_ptr.cast::<c_char>()
    }
}

pub fn transchar_byte(c: c_int) -> *mut c_char {
    transchar_byte_buf(g_st().curbuf, c)
}

pub fn transchar_byte_buf(buf: *const buf_T, c: c_int) -> *mut c_char {
    if c >= 0x80 {
            let charbuf_ptr = (&raw mut charset_st().transchar_charbuf).cast::<c_char>();
            transchar_nonprint(buf, charbuf_ptr, c);
            return charbuf_ptr;
        }
        transchar_buf(buf, c)
}

pub fn transchar_nonprint(buf: *const buf_T, charbuf: *mut c_char, mut c: c_int) {
    unsafe {
        if c == c_int::from(NL) {
            c = 0;
        } else if !buf.is_null() && c == CAR && get_fileformat(&*buf) == EOL_MAC {
            c = c_int::from(NL);
        }
        assert!(c <= 0xff);

        if (opt_vars().dy_flags & kOptDyFlagUhex) != 0 || c > 0x7f {
            transchar_hex(charbuf, c);
        } else {
            *charbuf = b'^' as c_char;
            *charbuf.add(1) = ((c as u8) ^ 0x40) as c_char;
            *charbuf.add(2) = NUL;
        }
    }
}

pub fn transchar_hex(buf: *mut c_char, c: c_int) -> usize {
    unsafe {
        let mut i: usize = 0;
        let uc = c as u32;

        *buf.add(i) = b'<' as c_char;
        i += 1;
        if c > 0xFF {
            if c > 0xFFFF {
                *buf.add(i) = nr2hex(uc >> 20) as c_char;
                i += 1;
                *buf.add(i) = nr2hex(uc >> 16) as c_char;
                i += 1;
            }
            *buf.add(i) = nr2hex(uc >> 12) as c_char;
            i += 1;
            *buf.add(i) = nr2hex(uc >> 8) as c_char;
            i += 1;
        }
        *buf.add(i) = nr2hex(uc >> 4) as c_char;
        i += 1;
        *buf.add(i) = nr2hex(uc) as c_char;
        i += 1;
        *buf.add(i) = b'>' as c_char;
        i += 1;
        *buf.add(i) = NUL;
        i
    }
}

pub fn rl_mirror_ascii(str_: *mut c_char, end: *mut c_char) {
    unsafe {
        let mut p1 = str_;
        let mut p2 = if !end.is_null() {
            end
        } else {
            str_.add(CStr::from_ptr(str_).to_bytes().len())
        }
        .sub(1);
        while p1 < p2 {
            ptr::swap(p1, p2);
            p1 = p1.add(1);
            p2 = p2.sub(1);
        }
    }
}

pub fn char2cells(c: c_int) -> c_int {
    if c >= 0x80 {
            return utf_char2cells(c);
        }
        ascii2cells(c)
}

pub fn vim_strsize(s: *const c_char) -> c_int {
    vim_strnsize(s, MAXCOL)
}

pub fn vim_strnsize(s: *const c_char, mut len: c_int) -> c_int {
    unsafe {
        assert!(!s.is_null());
        let mut size: c_int = 0;
        let mut ci = utf_ptr2StrCharInfo(s);
        while *ci.ptr != NUL && len > 0 {
            let cli = utf_ClusterInfo(ci);
            size += cli.cells;
            len -= cli.next.ptr.offset_from(ci.ptr) as c_int;
            ci = cli.next;
        }
        size
    }
}

pub fn vim_isIDc(c: c_int) -> bool {
    c > 0 && c < 0x100 && get_chartab((&raw const charset_st().isi_chartab).cast::<u64>(), c) != 0
}

pub fn vim_iswordc(c: c_int) -> bool {
    unsafe { vim_iswordc_buf(c, &mut *g_st().curbuf) }
}

pub fn vim_iswordc_tab(c: c_int, chartab: *const u64) -> bool {
    if c >= 0x100 {
            utf_class_tab(c, chartab) >= 2
        } else {
            c > 0 && get_chartab(chartab, c) != 0
        }
}

pub fn vim_iswordc_buf(c: c_int, buf: &mut buf_T) -> bool {
    vim_iswordc_tab(c, buf.b_chartab.as_ptr())
}

pub fn vim_iswordp(p: *const c_char) -> bool {
    vim_iswordp_buf(p, g_st().curbuf)
}

pub fn vim_iswordp_buf(p: *const c_char, buf: *mut buf_T) -> bool {
    unsafe {
        let mut c = c_int::from(*p as u8);
        if utf8len_tab[c as usize] > 1 {
            c = utf_ptr2char(p);
        }
        vim_iswordc_buf(c, &mut *buf)
    }
}

pub fn vim_isfilec(c: c_int) -> bool {
    c >= 0x100 || (c > 0 && get_chartab((&raw const charset_st().isf_chartab).cast::<u64>(), c) != 0)
}

pub fn vim_is_fname_char(c: c_int) -> bool {
    vim_isfilec(c)
            || c == b',' as c_int
            || c == b' ' as c_int
            || c == b'@' as c_int
            || c == b':' as c_int
}

pub fn vim_isfilec_or_wc(c: c_int) -> bool {
    let buf: [c_char; 2] = [c as c_char, NUL];
        vim_isfilec(c) || c == b']' as c_int || path_has_wildcard(buf.as_ptr(), true)
}

pub fn vim_isprintc(c: c_int) -> bool {
    if c >= 0x100 {
            return utf_printable(c);
        }
        c > 0 && (c & 0x60) != 0 && c != 0x7f
}

pub fn skipwhite(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while ascii_iswhite(*p as c_int) {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

pub fn skipwhite_len(mut p: *const c_char, mut len: usize) -> *mut c_char {
    unsafe {
        while len > 0 && ascii_iswhite(*p as c_int) {
            p = p.add(1);
            len -= 1;
        }
        p.cast_mut()
    }
}

pub fn getwhitecols_curline() -> isize {
    getwhitecols(get_cursor_line_ptr())
}

pub fn getwhitecols(p: *const c_char) -> isize {
    unsafe { skipwhite(p).offset_from(p) }
}

pub fn skipdigits(q: *const c_char) -> *mut c_char {
    unsafe {
        let mut p = q;
        while ascii_isdigit(*p as c_int) {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

pub fn skipbin(q: *const c_char) -> *const c_char {
    unsafe {
        let mut p = q;
        while ascii_isbdigit(*p as c_int) {
            p = p.add(1);
        }
        p
    }
}

pub fn skiphex(q: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = q;
        while ascii_isxdigit(*p as c_int) {
            p = p.add(1);
        }
        p
    }
}

pub fn skiptodigit(q: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = q;
        while *p != NUL && !ascii_isdigit(*p as c_int) {
            p = p.add(1);
        }
        p
    }
}

pub fn skiptobin(q: *const c_char) -> *const c_char {
    unsafe {
        let mut p = q;
        while *p != NUL && !ascii_isbdigit(*p as c_int) {
            p = p.add(1);
        }
        p
    }
}

pub fn skiptohex(q: *mut c_char) -> *mut c_char {
    unsafe {
        let mut p = q;
        while *p != NUL && !ascii_isxdigit(*p as c_int) {
            p = p.add(1);
        }
        p
    }
}

pub fn skiptowhite(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while *p != b' ' as c_char && *p != b'\t' as c_char && *p != NUL {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

pub fn skiptowhite_esc(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while *p != b' ' as c_char && *p != b'\t' as c_char && *p != NUL {
            if (*p == b'\\' as c_char || *p == Ctrl_V) && *p.add(1) != NUL {
                p = p.add(1);
            }
            p = p.add(1);
        }
        p.cast_mut()
    }
}

pub fn skip_to_newline(p: *const c_char) -> *mut c_char {
    unsafe { { let _s = p; let _b = std::ffi::CStr::from_ptr(_s).to_bytes(); let _off = _b.iter().position(|&_x| _x == (NL) as u8).unwrap_or(_b.len()); _s.add(_off).cast_mut() } }
}

pub fn try_getdigits(pp: *mut *mut c_char, nr: &mut i64) -> bool {
    unsafe {
        let orig = *pp;
        let mut p = orig.cast::<u8>();

        while matches!(*p, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            p = p.add(1);
        }

        let mut neg = false;
        if *p == b'-' {
            neg = true;
            p = p.add(1);
        } else if *p == b'+' {
            p = p.add(1);
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
            p = p.add(1);
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

pub fn getdigits_int(pp: *mut *mut c_char, strict: bool, def: c_int) -> c_int {
    let number = getdigits(pp, strict, i64::from(def));
    match c_int::try_from(number) {
        Ok(v) => v,
        Err(_) => {
            assert!(!strict);
            def
        }
    }
}

pub fn getdigits_long(pp: *mut *mut c_char, strict: bool, def: c_long) -> c_long {
    let number = getdigits(pp, strict, def as _);
    match c_long::try_from(number) {
        Ok(v) => v,
        Err(_) => {
            assert!(!strict);
            def
        }
    }
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

pub fn vim_isblankline(lbuf: *mut c_char) -> bool {
    unsafe {
        let p = skipwhite(lbuf);
        *p == NUL || *p == b'\r' as c_char || *p == b'\n' as c_char
    }
}

pub fn vim_str2nr(
    start: *const c_char,
    prep: Option<&mut c_int>,
    mut len: Option<&mut c_int>,
    what: c_int,
    nptr: Option<&mut varnumber_T>,
    unptr: Option<&mut uvarnumber_T>,
    maxlen: c_int,
    strict: bool,
    mut overflow: Option<&mut bool>,
) {
    unsafe {
        let mut ptr = start;
        let string_ended = |p: *const c_char| -> bool {
            !(maxlen == 0 || (p.offset_from(start) as c_int) < maxlen)
        };
        let mut pre: c_int = 0;
        let negative = *ptr == b'-' as c_char;
        let mut un: uvarnumber_T = 0;

        if let Some(len) = len.as_deref_mut() {
            *len = 0;
        }

        if negative {
            ptr = ptr.add(1);
        }

        let base: uvarnumber_T;
        if (what & STR2NR_FORCE) != 0 {
            match what & !(STR2NR_FORCE | STR2NR_QUOTE) {
                STR2NR_HEX => {
                    if !string_ended(ptr.add(2))
                        && *ptr == b'0' as c_char
                        && (*ptr.add(1) == b'x' as c_char || *ptr.add(1) == b'X' as c_char)
                        && ascii_isxdigit(*ptr.add(2) as c_int)
                    {
                        ptr = ptr.add(2);
                    }
                    base = 16;
                }
                STR2NR_BIN => {
                    if !string_ended(ptr.add(2))
                        && *ptr == b'0' as c_char
                        && (*ptr.add(1) == b'b' as c_char || *ptr.add(1) == b'B' as c_char)
                        && ascii_isbdigit(*ptr.add(2) as c_int)
                    {
                        ptr = ptr.add(2);
                    }
                    base = 2;
                }
                x if x == STR2NR_OCT || x == STR2NR_OOCT || x == (STR2NR_OCT | STR2NR_OOCT) => {
                    if !string_ended(ptr.add(2))
                        && *ptr == b'0' as c_char
                        && (*ptr.add(1) == b'o' as c_char || *ptr.add(1) == b'O' as c_char)
                        && ascii_isodigit(*ptr.add(2) as c_int)
                    {
                        ptr = ptr.add(2);
                    }
                    base = 8;
                }
                0 => {
                    base = 10;
                }
                _ => std::process::abort(),
            }
        } else if (what & (STR2NR_HEX | STR2NR_OCT | STR2NR_OOCT | STR2NR_BIN)) != 0
            && !string_ended(ptr.add(1))
            && *ptr == b'0' as c_char
            && *ptr.add(1) != b'8' as c_char
            && *ptr.add(1) != b'9' as c_char
        {
            pre = c_int::from(*ptr.add(1) as u8);
            if (what & STR2NR_HEX) != 0
                && !string_ended(ptr.add(2))
                && (pre == b'X' as c_int || pre == b'x' as c_int)
                && ascii_isxdigit(*ptr.add(2) as c_int)
            {
                ptr = ptr.add(2);
                base = 16;
            } else if (what & STR2NR_BIN) != 0
                && !string_ended(ptr.add(2))
                && (pre == b'B' as c_int || pre == b'b' as c_int)
                && ascii_isbdigit(*ptr.add(2) as c_int)
            {
                ptr = ptr.add(2);
                base = 2;
            } else if (what & STR2NR_OOCT) != 0
                && !string_ended(ptr.add(2))
                && (pre == b'O' as c_int || pre == b'o' as c_int)
                && ascii_isodigit(*ptr.add(2) as c_int)
            {
                ptr = ptr.add(2);
                base = 8;
            } else {
                pre = 0;
                if (what & STR2NR_OCT) == 0 || !ascii_isodigit(*ptr.add(1) as c_int) {
                    base = 10;
                } else {
                    let mut is_oct = true;
                    let mut i: usize = 2;
                    while !string_ended(ptr.add(i)) && ascii_isdigit(*ptr.add(i) as c_int) {
                        if (*ptr.add(i) as u8) > b'7' {
                            is_oct = false;
                            break;
                        }
                        i += 1;
                    }
                    if is_oct {
                        pre = b'0' as c_int;
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
            let check_cond = |c: c_int| -> bool {
                match base {
                    2 => c == b'0' as c_int || c == b'1' as c_int,
                    8 => ascii_isodigit(c),
                    10 => ascii_isdigit(c),
                    16 => ascii_isxdigit(c),
                    _ => unreachable!(),
                }
            };
            if (what & STR2NR_QUOTE) != 0 && ptr > after_prefix && *ptr == b'\'' as c_char {
                ptr = ptr.add(1);
                if !string_ended(ptr) && check_cond(*ptr as c_int) {
                    continue;
                }
                ptr = ptr.sub(1);
            }
            let c = *ptr as c_int;
            if !check_cond(c) {
                break;
            }
            let digit = if base == 16 {
                hex2nr(c) as uvarnumber_T
            } else {
                (c - b'0' as c_int) as uvarnumber_T
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
            ptr = ptr.add(1);
        }

        if strict && (ptr.offset_from(start) as c_int) != maxlen && ascii_isalnum(*ptr as c_int) {
            return;
        }

        if let Some(prep) = prep {
            *prep = pre;
        }

        if let Some(len) = len {
            *len = ptr.offset_from(start) as c_int;
        }

        if let Some(nptr) = nptr {
            if negative {
                if un > VARNUMBER_MAX {
                    *nptr = VARNUMBER_MIN;
                    if let Some(overflow) = overflow.as_deref_mut() {
                        *overflow = true;
                    }
                } else {
                    *nptr = -(un as varnumber_T);
                }
            } else {
                if un > VARNUMBER_MAX {
                    un = VARNUMBER_MAX;
                    if let Some(overflow) = overflow {
                        *overflow = true;
                    }
                }
                *nptr = un as varnumber_T;
            }
        }

        if let Some(unptr) = unptr {
            *unptr = un;
        }
    }
}

pub const fn hex2nr(c: c_int) -> c_int {
    if c >= b'a' as c_int && c <= b'f' as c_int {
        c - b'a' as c_int + 10
    } else if c >= b'A' as c_int && c <= b'F' as c_int {
        c - b'A' as c_int + 10
    } else {
        c - b'0' as c_int
    }
}

pub fn hexhex2nr(p: *const c_char) -> c_int {
    unsafe {
        if !ascii_isxdigit(*p as c_int) || !ascii_isxdigit(*p.add(1) as c_int) {
            return -1;
        }
        (hex2nr(*p as c_int) << 4) + hex2nr(*p.add(1) as c_int)
    }
}

pub fn rem_backslash(str_: *const c_char) -> bool {
    unsafe {
        #[cfg(windows)]
        {
            let b0 = *str_ as u8;
            let b1 = *str_.add(1) as u8;
            b0 == b'\\'
                && b1 < 0x80
                && (b1 == b' '
                    || (b1 != 0 && b1 != b'*' && b1 != b'?' && !vim_isfilec(c_int::from(b1))))
        }
        #[cfg(not(windows))]
        {
            *str_ == b'\\' as c_char && *str_.add(1) != NUL
        }
    }
}

pub fn backslash_halve(mut p: *mut c_char) {
    unsafe {
        while *p != NUL && !rem_backslash(p) {
            p = p.add(1);
        }
        if *p != NUL {
            let mut dst = p;
            *dst = *p.add(1);
            dst = dst.add(1);
            p = p.add(2);
            while *p != NUL {
                if rem_backslash(p) {
                    *dst = *p.add(1);
                    dst = dst.add(1);
                    p = p.add(2);
                } else {
                    *dst = *p;
                    dst = dst.add(1);
                    p = p.add(1);
                }
            }
            *dst = NUL;
        }
    }
}

pub fn backslash_halve_save(mut p: *const c_char) -> CString {
    unsafe {
        let mut res = Vec::with_capacity(CStr::from_ptr(p).to_bytes().len());
        while *p != NUL {
            if rem_backslash(p) {
                res.push(*p.add(1) as u8);
                p = p.add(2);
            } else {
                res.push(*p as u8);
                p = p.add(1);
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

    #[test]
    fn to_special_handles_ks_special_and_ks_zero() {
        assert_eq!(to_special(KS_SPECIAL, KE_FILLER as c_int), K_SPECIAL);
        assert_eq!(to_special(KS_ZERO, KE_FILLER as c_int), K_ZERO);
        assert_eq!(to_special(b'k' as c_int, b'u' as c_int), K_UP);
    }

    #[test]
    fn vim_str2nr_parses_dec_bin_oct_hex_and_quotes() {
        let check = |s: &CStr,
                         what: c_int,
                         maxlen: c_int,
                         strict: bool,
                         exp_len: c_int,
                         exp_num: Option<varnumber_T>,
                         exp_unum: Option<uvarnumber_T>,
                         exp_pre: Option<c_int>| {
                let mut pre: c_int = -99;
                let mut len: c_int = -99;
                let mut num: varnumber_T = -99;
                let mut unum: uvarnumber_T = 99;
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
                Some(b'b' as c_int),
            );
            check(
                c"-0B101",
                STR2NR_ALL,
                0,
                true,
                6,
                Some(-5),
                Some(5),
                Some(b'B' as c_int),
            );
            check(
                c"054",
                STR2NR_ALL,
                0,
                true,
                3,
                Some(44),
                Some(44),
                Some(b'0' as c_int),
            );
            check(
                c"0o054",
                STR2NR_ALL,
                0,
                true,
                5,
                Some(44),
                Some(44),
                Some(b'o' as c_int),
            );
            check(
                c"0xABCDEF",
                STR2NR_ALL,
                0,
                true,
                8,
                Some(11259375),
                Some(11259375),
                Some(b'x' as c_int),
            );
            check(
                c"-0X101",
                STR2NR_ALL,
                0,
                true,
                6,
                Some(-257),
                Some(257),
                Some(b'X' as c_int),
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
                Some(b'x' as c_int),
            );
    }
}
