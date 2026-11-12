//! Code related to character sets (`src/nvim/charset.c`).
#![forbid(unsafe_code)]

use crate::buffer::FileBuf;
use crate::globals::*;
use crate::mbyte::{
    UTF8LEN_TAB, mb_islower, mb_isupper, mb_tolower, utf_char2cells, utf_char2len, utf_char2slice,
    utf_class_tab, utf_cluster_info, utf_printable, utf_ptr2_str_char_info, utf_slice2char,
    utf_slice2len, utfc_slice2len,
};
use crate::memory::bytes_to_cstring;
use crate::option::{get_fileformat, skip_to_option_part_slice};
use crate::option_vars::*;
use crate::path::path_has_wildcard;
use crate::strings::RawCChar;
use std::ffi::{CStr, CString};

use crate::change::TAB;
use crate::eval::typval::Varnumber;
use crate::keycodes::CAR;
use crate::regexp::MAXCOL;
pub type Uvarnumber = u64;

pub(crate) const NUL: RawCChar = 0;
pub(crate) const NL: RawCChar = b'\n' as RawCChar;

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

pub(crate) const CTRL_V_CONST: RawCChar = 22;

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
    isf_chartab: std::cell::Cell<[u64; 4]>,
    isi_chartab: std::cell::Cell<[u64; 4]>,
}

impl CharsetModuleState {
    const fn new() -> Self {
        Self {
            isf_chartab: std::cell::Cell::new([0; 4]),
            isi_chartab: std::cell::Cell::new([0; 4]),
        }
    }
}

std::thread_local! {
    static CHARSET_STATE: CharsetModuleState = const { CharsetModuleState::new() };
}

#[inline]
fn charset_st<R>(f: impl FnOnce(&CharsetModuleState) -> R) -> R {
    CHARSET_STATE.with(f)
}

#[inline]
fn set_chartab(chartab: &mut [u64; 4], c: i32) {
    let idx = ((c as u32) >> 6) as usize;
    chartab[idx] |= 1u64 << (c & 0x3f);
}

#[inline]
fn reset_chartab(chartab: &mut [u64; 4], c: i32) {
    let idx = ((c as u32) >> 6) as usize;
    chartab[idx] &= !(1u64 << (c & 0x3f));
}

#[inline]
fn get_chartab(chartab: &[u64; 4], c: i32) -> u64 {
    let idx = ((c as u32) >> 6) as usize;
    chartab[idx] & (1u64 << (c & 0x3f))
}

#[inline]
pub const fn ascii_isbdigit(c: i32) -> bool {
    c == b'0' as i32 || c == b'1' as i32
}

#[inline]
pub fn ascii2cells(b: i32) -> i32 {
    if (b' ' as i32..=b'~' as i32).contains(&b) {
        1
    } else {
        opt_vars().dy_escape_width.get()
    }
}

#[inline]
const fn nr2hex(n: u32) -> u8 {
    let v = (n & 0xf) as u8;
    if v <= 9 { v + b'0' } else { v - 10 + b'a' }
}

use crate::keycodes::k_second;

pub fn buf_init_isk_chartab(buf: &FileBuf) -> bool {
    let mut chartab = [0; 4];
    if buf.b_p_lisp.get() {
        set_chartab(&mut chartab, b'-' as i32);
    }
    if !parse_isopt(buf.b_p_isk.as_bytes(), Some(&mut chartab)) {
        return false;
    }
    buf.b_chartab.set(chartab);
    true
}

pub fn init_isf_chartab() -> bool {
    let mut tab = [0, 0, 0xFFFF_FFFF_0000_0000, 0xFFFF_FFFF_FFFF_FFFF];
    if !parse_isopt(opt_vars().p_isf.as_bytes(), Some(&mut tab)) {
        return false;
    }
    charset_st(|st| st.isf_chartab.set(tab));
    true
}

pub fn init_isi_chartab() -> bool {
    let mut tab = [0; 4];
    if !parse_isopt(opt_vars().p_isi.as_bytes(), Some(&mut tab)) {
        return false;
    }
    charset_st(|st| st.isi_chartab.set(tab));
    true
}

pub fn check_isopt(var: &[u8]) -> bool {
    parse_isopt(var, None)
}

pub fn parse_isopt(mut p: &[u8], mut chartab: Option<&mut [u64; 4]>) -> bool {
    while !p.is_empty() {
        let mut tilde = false;
        let mut do_isalpha = false;

        if p[0] == b'^' && p.get(1).copied().unwrap_or(0) != 0 {
            tilde = true;
            p = &p[1..];
        }

        let mut c = if p[0].is_ascii_digit() {
            getdigits_int(&mut p, false, 0)
        } else {
            crate::mbyte::mb_slice2char_adv(&mut p)
        };
        let mut c2: i32 = -1;

        if p.first() == Some(&b'-') && p.get(1).copied().unwrap_or(0) != 0 {
            p = &p[1..];
            c2 = if p.first().is_some_and(u8::is_ascii_digit) {
                getdigits_int(&mut p, false, 0)
            } else {
                crate::mbyte::mb_slice2char_adv(&mut p)
            };
        }

        if c <= 0
            || c >= 256
            || (c2 < c && c2 != -1)
            || c2 >= 256
            || !(p.is_empty() || p[0] == b',')
        {
            return false;
        }

        let trail_comma = p.first() == Some(&b',');
        p = skip_to_option_part_slice(p);
        if trail_comma && p.is_empty() {
            return false;
        }

        let Some(ref mut tab) = chartab else {
            continue;
        };

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
                    reset_chartab(tab, c);
                } else {
                    set_chartab(tab, c);
                }
            }
            c += 1;
        }
    }

    true
}

pub fn trans_characters(buf: &mut [u8]) {
    let mut len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let mut pos = 0usize;

    while pos < len {
        let mut trs_len = utfc_slice2len(&buf[pos..len]);
        if trs_len <= 1 {
            let trs = transchar_byte(i32::from(buf[pos]));
            let trs_bytes = trs.to_bytes();
            trs_len = trs_bytes.len();

            if trs_len > 1 {
                let extra = trs_len - 1;
                if len + extra >= buf.len() {
                    return;
                }
                buf.copy_within(pos + 1..=len, pos + trs_len);
                len += extra;
            }
            buf[pos..pos + trs_len].copy_from_slice(trs_bytes);
        }
        pos += trs_len;
    }
}

pub fn transstr_len(mut bytes: &[u8], untab: bool) -> usize {
    let mut len: usize = 0;

    while !bytes.is_empty() {
        let l = utfc_slice2len(bytes);
        if l > 1 {
            if vim_isprintc(utf_slice2char(bytes)) {
                len += l;
            } else {
                let mut off: usize = 0;
                while off < l {
                    let c = utf_slice2char(&bytes[off..]);
                    let mut hexbuf = [0u8; 9];
                    len += transchar_hex(&mut hexbuf, c);
                    off += utf_slice2len(&bytes[off..]);
                }
            }
            bytes = &bytes[l..];
        } else if bytes[0] == TAB && !untab {
            len += 1;
            bytes = &bytes[1..];
        } else {
            let c = bytes[0];
            bytes = &bytes[1..];
            len += if c < 0x80 {
                ascii2cells(i32::from(c)) as usize
            } else {
                4
            };
        }
    }
    len
}

pub fn transstr_buf(mut bytes: &[u8], buf_slice: &mut [u8], untab: bool) -> usize {
    let max_out = buf_slice.len().saturating_sub(1);
    let mut out = 0usize;
    if bytes.is_empty() {
        buf_slice[0] = 0;
        return 0;
    }

    while !bytes.is_empty() && out < max_out {
        let l = utfc_slice2len(bytes);
        if l > 1 {
            if out + l > max_out {
                break;
            }

            if vim_isprintc(utf_slice2char(bytes)) {
                buf_slice[out..out + l].copy_from_slice(&bytes[..l]);
                out += l;
            } else {
                let mut off: usize = 0;
                while off < l {
                    let c = utf_slice2char(&bytes[off..]);
                    let mut hexbuf = [0u8; 9];
                    let hexlen = transchar_hex(&mut hexbuf, c);
                    if out + hexlen > max_out {
                        break;
                    }
                    buf_slice[out..out + hexlen].copy_from_slice(&hexbuf[..hexlen]);
                    out += hexlen;
                    off += utf_slice2len(&bytes[off..]);
                }
            }
            bytes = &bytes[l..];
        } else if bytes[0] == TAB && !untab {
            buf_slice[out] = bytes[0];
            out += 1;
            bytes = &bytes[1..];
        } else {
            let c = bytes[0];
            bytes = &bytes[1..];
            let tb_buf = transchar_byte(i32::from(c));
            let tb = tb_buf.to_bytes();
            if out + tb.len() > max_out {
                break;
            }
            buf_slice[out..out + tb.len()].copy_from_slice(tb);
            out += tb.len();
        }
    }
    buf_slice[out] = 0;
    out
}

pub fn transstr(s: &[u8], untab: bool) -> CString {
    let bytes = crate::memory::bytes_until_nul(s);
    let len = transstr_len(bytes, untab) + 1;
    let mut buf = vec![0u8; len];
    let written = transstr_buf(bytes, &mut buf, untab);
    bytes_to_cstring(&buf[..written])
}

pub fn kv_transstr(str_: &mut Vec<u8>, bytes: &[u8], untab: bool) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    let len = transstr_len(bytes, untab);
    let old_len = str_.len();
    str_.resize(old_len + len + 1, 0);
    transstr_buf(bytes, &mut str_[old_len..], untab);
    str_.truncate(old_len + len);
    len
}

pub fn str_foldcase(src: &[u8], mut buf: Option<&mut [u8]>) -> Option<CString> {
    let mut vec: Vec<u8> = Vec::new();
    let mut len: usize;

    if let Some(ref mut b) = buf {
        let max_len = b.len().saturating_sub(1);
        len = src.len().min(max_len);
        b[..len].copy_from_slice(&src[..len]);
        b[len] = 0;
    } else {
        len = src.len();
        vec.reserve_exact(len + 1);
        vec.extend_from_slice(src);
        vec.push(0);
    }

    let mut i: usize = 0;
    loop {
        let cur_slice: &[u8] = match buf {
            Some(ref b) => &b[..len],
            None => &vec[..len],
        };
        if i >= len || cur_slice[i] == 0 {
            break;
        }
        let rest = &cur_slice[i..];
        let c = utf_slice2char(rest);
        let olen = utf_slice2len(rest);
        let mut lc = mb_tolower(c);

        if (c < 0x80 || olen > 1) && c != lc {
            let mut nlen = utf_char2len(lc) as usize;
            if olen != nlen {
                if nlen > olen {
                    let extra = nlen - olen;
                    if let Some(ref b) = buf {
                        if len + extra >= b.len() {
                            lc = c;
                            nlen = olen;
                        }
                    } else {
                        vec.resize(vec.len() + extra, 0);
                    }
                }

                if olen != nlen {
                    match buf {
                        Some(ref mut b) => {
                            b.copy_within(i + olen..=len, i + nlen);
                        }
                        None => {
                            vec.copy_within(i + olen..=len, i + nlen);
                            if nlen < olen {
                                vec.truncate(vec.len() - (olen - nlen));
                            }
                        }
                    }
                    len = (len as isize + nlen as isize - olen as isize) as usize;
                }
            }
            match buf {
                Some(ref mut b) => {
                    utf_char2slice(lc, &mut b[i..]);
                }
                None => {
                    utf_char2slice(lc, &mut vec[i..]);
                }
            }
        }

        let updated_slice: &[u8] = match buf {
            Some(ref b) => &b[i..len],
            None => &vec[i..len],
        };
        i += utfc_slice2len(updated_slice);
    }

    if buf.is_none() {
        let out_len = vec.iter().position(|&b| b == 0).unwrap_or(len);
        Some(bytes_to_cstring(&vec[..out_len]))
    } else {
        None
    }
}

pub type TransCharBuf = crate::strings::InlineCStr<11>;

pub fn transchar(c: i32) -> TransCharBuf {
    transchar_buf(Some(&crate::globals::curbuf_mut()), c)
}

pub fn transchar_buf(buf: Option<&FileBuf>, mut c: i32) -> TransCharBuf {
    let mut charbuf = TransCharBuf::new();
    let mut i: usize = 0;
    if is_special(c) {
        charbuf.0[0] = b'~';
        charbuf.0[1] = b'@';
        i = 2;
        c = i32::from(k_second(c));
    }

    if c <= 0xFF && vim_isprintc(c) {
        charbuf.0[i] = c as u8;
        charbuf.0[i + 1] = 0;
    } else if c <= 0xFF {
        transchar_nonprint(buf, &mut charbuf.0[i..], c);
    } else {
        transchar_hex(&mut charbuf.0[i..], c);
    }
    charbuf
}

pub fn transchar_byte<C: AsciiInt>(c: C) -> TransCharBuf {
    transchar_byte_buf(Some(&crate::globals::curbuf_mut()), c.into_ascii_int())
}

pub fn transchar_byte_buf(buf: Option<&FileBuf>, c: i32) -> TransCharBuf {
    if c >= 0x80 {
        let mut charbuf = TransCharBuf::new();
        transchar_nonprint(buf, &mut charbuf.0, c);
        return charbuf;
    }
    transchar_buf(buf, c)
}

pub fn transchar_nonprint(buf: Option<&FileBuf>, charbuf: &mut [u8], mut c: i32) {
    if c == i32::from(NL) {
        c = 0;
    } else if buf.is_some() && c == CAR && get_fileformat(buf.unwrap()) == EOL_MAC {
        c = i32::from(NL);
    }
    assert!(c <= 0xff);

    if (opt_vars().dy_flags.get() & K_OPT_DY_FLAG_UHEX) != 0 || c > 0x7f {
        transchar_hex(charbuf, c);
    } else {
        charbuf[0] = b'^';
        charbuf[1] = (c as u8) ^ 0x40;
        charbuf[2] = 0;
    }
}

pub fn transchar_hex(buf: &mut [u8], c: i32) -> usize {
    let mut i: usize = 0;
    let uc = c as u32;

    buf[i] = b'<';
    i += 1;
    if c > 0xFF {
        if c > 0xFFFF {
            buf[i] = nr2hex(uc >> 20);
            i += 1;
            buf[i] = nr2hex(uc >> 16);
            i += 1;
        }
        buf[i] = nr2hex(uc >> 12);
        i += 1;
        buf[i] = nr2hex(uc >> 8);
        i += 1;
    }
    buf[i] = nr2hex(uc >> 4);
    i += 1;
    buf[i] = nr2hex(uc);
    i += 1;
    buf[i] = b'>';
    i += 1;
    buf[i] = 0;
    i
}

pub fn rl_mirror_ascii(buf: &mut [u8]) {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf[..end].reverse();
}

pub fn char2cells(c: i32) -> i32 {
    if c >= 0x80 {
        return utf_char2cells(c);
    }
    ascii2cells(c)
}

pub fn vim_strsize(s: &CStr) -> i32 {
    vim_strnsize(s.to_bytes(), MAXCOL)
}

pub fn vim_strnsize(s: &[u8], mut len: i32) -> i32 {
    let s_bytes = crate::memory::bytes_until_nul(s);
    let max_len = (len.max(0) as usize).min(s_bytes.len());
    let slice = &s_bytes[..max_len];
    if slice.is_empty() {
        return 0;
    }
    let cstr = crate::memory::bytes_to_cstring(slice);
    let mut size: i32 = 0;
    let mut ci = utf_ptr2_str_char_info(&cstr);
    while !ci.ptr.is_empty() && len > 0 {
        let cli = utf_cluster_info(ci);
        size += cli.cells;
        len -= (ci.ptr.len() - cli.next.ptr.len()) as i32;
        ci = cli.next;
    }
    size
}

pub fn vim_is_idc(c: i32) -> bool {
    c > 0 && c < 0x100 && get_chartab(&charset_st(|st| st.isi_chartab.get()), c) != 0
}

pub fn vim_iswordc(c: i32) -> bool {
    vim_iswordc_buf(c, &curbuf_mut())
}

pub fn vim_iswordc_tab(c: i32, chartab: &[u64; 4]) -> bool {
    if c >= 0x100 {
        utf_class_tab(c, chartab) >= 2
    } else {
        c > 0 && get_chartab(chartab, c) != 0
    }
}

pub fn vim_iswordc_buf(c: i32, buf: &FileBuf) -> bool {
    vim_iswordc_tab(c, &buf.b_chartab.get())
}

pub fn vim_iswordp(bytes: &[u8]) -> bool {
    vim_iswordp_buf(bytes, &crate::globals::curbuf_mut())
}

pub fn vim_iswordp_buf(bytes: &[u8], buf: &FileBuf) -> bool {
    let b0 = bytes.first().copied().unwrap_or(0);
    let c = if UTF8LEN_TAB[b0 as usize] > 1 {
        utf_slice2char(bytes)
    } else {
        i32::from(b0)
    };
    vim_iswordc_buf(c, buf)
}

pub fn vim_isfilec(c: i32) -> bool {
    c >= 0x100 || (c > 0 && get_chartab(&charset_st(|st| st.isf_chartab.get()), c) != 0)
}

pub fn vim_is_fname_char(c: i32) -> bool {
    vim_isfilec(c) || c == b',' as i32 || c == b' ' as i32 || c == b'@' as i32 || c == b':' as i32
}

pub fn vim_isfilec_or_wc(c: i32) -> bool {
    vim_isfilec(c) || c == b']' as i32 || path_has_wildcard(&[c as u8], true)
}

pub fn vim_isprintc(c: i32) -> bool {
    if c >= 0x100 {
        return utf_printable(c);
    }
    c > 0 && (c & 0x60) != 0 && c != 0x7f
}

#[inline]
pub fn skipwhite_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == b' ' || b == b'\t' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

#[inline]
pub fn skiptowhite_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == b' ' || b == b'\t' || b == 0 {
            break;
        }
        s = rest;
    }
    s
}

#[inline]
pub fn skiptowhite_esc_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if matches!(b, b' ' | b'\t' | 0) {
            break;
        }
        if (b == b'\\' || b == CTRL_V_CONST as u8)
            && let Some((&next, rest2)) = rest.split_first()
            && next != 0
        {
            s = rest2;
        } else {
            s = rest;
        }
    }
    s
}

#[inline]
pub fn skipdigits_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b.is_ascii_digit() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

#[inline]
pub fn skiptodigit_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == 0 || b.is_ascii_digit() {
            break;
        }
        s = rest;
    }
    s
}

#[inline]
pub fn skiphex_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b.is_ascii_hexdigit() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

#[inline]
pub fn skiptohex_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == 0 || b.is_ascii_hexdigit() {
            break;
        }
        s = rest;
    }
    s
}

#[inline]
pub fn skipbin_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == b'0' || b == b'1' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

#[inline]
pub fn skiptobin_slice(mut s: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = s.split_first() {
        if b == 0 || b == b'0' || b == b'1' {
            break;
        }
        s = rest;
    }
    s
}

pub fn skipwhite(p: &CStr) -> &CStr {
    let bytes = p.to_bytes();
    let rem = skipwhite_slice(bytes);
    crate::memory::CStrSubslice::advance(p, bytes.len() - rem.len())
}

pub fn skipwhite_at(p: &CStr, offset: usize) -> &CStr {
    let bytes = p.to_bytes();
    let off = offset.min(bytes.len());
    let rem = skipwhite_slice(&bytes[off..]);
    crate::memory::CStrSubslice::advance(p, bytes.len() - rem.len())
}

#[inline]
pub fn skipwhite_len(bytes: &[u8]) -> usize {
    let b = crate::memory::bytes_until_nul(bytes);
    b.len() - skipwhite_slice(b).len()
}

pub fn getwhitecols_curline() -> isize {
    getwhitecols(crate::cursor::get_cursor_line_slice())
}

pub fn getwhitecols(bytes: &[u8]) -> isize {
    (bytes.len() - skipwhite_slice(bytes).len()) as isize
}

#[inline]
pub fn skipdigits_len(bytes: &[u8]) -> usize {
    let b = crate::memory::bytes_until_nul(bytes);
    b.len() - skipdigits_slice(b).len()
}

#[inline]
pub fn skiptowhite_len(bytes: &[u8]) -> usize {
    let b = crate::memory::bytes_until_nul(bytes);
    b.len() - skiptowhite_slice(b).len()
}

#[inline]
pub fn skiptowhite_esc_len(bytes: &[u8]) -> usize {
    let b = crate::memory::bytes_until_nul(bytes);
    b.len() - skiptowhite_esc_slice(b).len()
}

pub fn try_getdigits(s: &mut &[u8], nr: &mut i64) -> bool {
    let orig = *s;
    let mut p = orig;

    while let Some((&b, rest)) = p.split_first() {
        if matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            p = rest;
        } else {
            break;
        }
    }

    let mut neg = false;
    if let Some((&b'-', rest)) = p.split_first() {
        neg = true;
        p = rest;
    } else if let Some((&b'+', rest)) = p.split_first() {
        p = rest;
    }

    if !p.first().is_some_and(u8::is_ascii_digit) {
        *s = orig;
        *nr = 0;
        return true;
    }

    let mut acc: u64 = 0;
    let mut overflow = false;
    while let Some((&b, rest)) = p.split_first() {
        if !b.is_ascii_digit() {
            break;
        }
        let d = u64::from(b - b'0');
        if let Some(next) = acc.checked_mul(10).and_then(|v| v.checked_add(d)) {
            acc = next;
        } else {
            overflow = true;
        }
        p = rest;
    }

    *s = p;
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

pub fn getdigits(s: &mut &[u8], strict: bool, def: i64) -> i64 {
    let mut number: i64 = 0;
    let ok = try_getdigits(s, &mut number);
    if strict && !ok {
        std::process::abort();
    }
    if ok { number } else { def }
}

pub fn getdigits_int(s: &mut &[u8], strict: bool, def: i32) -> i32 {
    let number = getdigits(s, strict, i64::from(def));
    match i32::try_from(number) {
        Ok(v) => v,
        Err(_) => {
            assert!(!strict);
            def
        }
    }
}

pub fn getdigits_long(s: &mut &[u8], strict: bool, def: i64) -> i64 {
    getdigits(s, strict, def)
}

pub fn getdigits_int32(s: &mut &[u8], strict: bool, def: i32) -> i32 {
    getdigits_int(s, strict, def)
}

pub fn vim_isblankline(bytes: &[u8]) -> bool {
    matches!(
        skipwhite_slice(bytes).first(),
        None | Some(&b'\r') | Some(&b'\n')
    )
}

pub fn vim_slice2nr(
    bytes: &[u8],
    prep: Option<&mut i32>,
    mut len: Option<&mut i32>,
    what: i32,
    nptr: Option<&mut Varnumber>,
    unptr: Option<&mut Uvarnumber>,
    strict: bool,
    mut overflow: Option<&mut bool>,
) {
    let mut idx: usize = 0;
    let mut pre: i32 = 0;
    let negative = bytes.first() == Some(&b'-');
    let mut un: Uvarnumber = 0;

    if let Some(len) = len.as_deref_mut() {
        *len = 0;
    }

    if negative {
        idx = 1;
    }

    let rem = &bytes[idx..];
    let base: Uvarnumber;
    if (what & STR2NR_FORCE) != 0 {
        match what & !(STR2NR_FORCE | STR2NR_QUOTE) {
            STR2NR_HEX => {
                if rem.len() > 2
                    && rem[0] == b'0'
                    && matches!(rem[1], b'x' | b'X')
                    && rem[2].is_ascii_hexdigit()
                {
                    idx += 2;
                }
                base = 16;
            }
            STR2NR_BIN => {
                if rem.len() > 2
                    && rem[0] == b'0'
                    && matches!(rem[1], b'b' | b'B')
                    && matches!(rem[2], b'0' | b'1')
                {
                    idx += 2;
                }
                base = 2;
            }
            x if x == STR2NR_OCT || x == STR2NR_OOCT || x == (STR2NR_OCT | STR2NR_OOCT) => {
                if rem.len() > 2
                    && rem[0] == b'0'
                    && matches!(rem[1], b'o' | b'O')
                    && matches!(rem[2], b'0'..=b'7')
                {
                    idx += 2;
                }
                base = 8;
            }
            0 => {
                base = 10;
            }
            _ => std::process::abort(),
        }
    } else if (what & (STR2NR_HEX | STR2NR_OCT | STR2NR_OOCT | STR2NR_BIN)) != 0
        && rem.len() > 1
        && rem[0] == b'0'
        && rem[1] != b'8'
        && rem[1] != b'9'
    {
        pre = i32::from(rem[1]);
        if (what & STR2NR_HEX) != 0
            && rem.len() > 2
            && matches!(rem[1], b'X' | b'x')
            && rem[2].is_ascii_hexdigit()
        {
            idx += 2;
            base = 16;
        } else if (what & STR2NR_BIN) != 0
            && rem.len() > 2
            && matches!(rem[1], b'B' | b'b')
            && matches!(rem[2], b'0' | b'1')
        {
            idx += 2;
            base = 2;
        } else if (what & STR2NR_OOCT) != 0
            && rem.len() > 2
            && matches!(rem[1], b'O' | b'o')
            && matches!(rem[2], b'0'..=b'7')
        {
            idx += 2;
            base = 8;
        } else {
            pre = 0;
            if (what & STR2NR_OCT) == 0 || !matches!(rem[1], b'0'..=b'7') {
                base = 10;
            } else {
                let is_oct = rem[2..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_digit())
                    .all(|&b| b <= b'7');
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

    let after_prefix = idx;
    let check_cond = |b: u8| -> bool {
        match base {
            2 => matches!(b, b'0' | b'1'),
            8 => matches!(b, b'0'..=b'7'),
            10 => b.is_ascii_digit(),
            16 => b.is_ascii_hexdigit(),
            _ => unreachable!(),
        }
    };
    while idx < bytes.len() {
        if (what & STR2NR_QUOTE) != 0
            && idx > after_prefix
            && bytes[idx] == b'\''
            && bytes.get(idx + 1).is_some_and(|&b| check_cond(b))
        {
            idx += 1;
            continue;
        }
        let b = bytes[idx];
        if !check_cond(b) {
            break;
        }
        let digit = if base == 16 {
            hex2nr(i32::from(b)) as Uvarnumber
        } else {
            Uvarnumber::from(b - b'0')
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
        idx += 1;
    }

    if strict && bytes.get(idx).is_some_and(u8::is_ascii_alphanumeric) {
        return;
    }

    if let Some(prep) = prep {
        *prep = pre;
    }

    if let Some(len) = len {
        *len = idx as i32;
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

pub fn vim_str2nr(
    start: &[u8],
    prep: Option<&mut i32>,
    len: Option<&mut i32>,
    what: i32,
    nptr: Option<&mut Varnumber>,
    unptr: Option<&mut Uvarnumber>,
    maxlen: i32,
    strict: bool,
    overflow: Option<&mut bool>,
) {
    let s = crate::memory::bytes_until_nul(start);
    let bytes = if maxlen > 0 {
        &s[..(maxlen as usize).min(s.len())]
    } else {
        s
    };
    vim_slice2nr(bytes, prep, len, what, nptr, unptr, strict, overflow);
}

pub fn hex2nr<C: AsciiInt>(c: C) -> i32 {
    let ch = c.into_ascii_int();
    if ch >= b'a' as i32 && ch <= b'f' as i32 {
        ch - b'a' as i32 + 10
    } else if ch >= b'A' as i32 && ch <= b'F' as i32 {
        ch - b'A' as i32 + 10
    } else {
        ch - b'0' as i32
    }
}

pub fn hexhex2nr(bytes: &[u8]) -> i32 {
    if bytes.len() < 2 || !bytes[0].is_ascii_hexdigit() || !bytes[1].is_ascii_hexdigit() {
        return -1;
    }
    (hex2nr(bytes[0]) << 4) + hex2nr(bytes[1])
}

pub fn rem_backslash(bytes: &[u8]) -> bool {
    #[cfg(windows)]
    {
        let b0 = bytes.first().copied().unwrap_or(0);
        let b1 = bytes.get(1).copied().unwrap_or(0);
        b0 == b'\\'
            && b1 < 0x80
            && (b1 == b' ' || (b1 != 0 && b1 != b'*' && b1 != b'?' && !vim_isfilec(i32::from(b1))))
    }
    #[cfg(not(windows))]
    {
        bytes.first() == Some(&b'\\') && bytes.len() > 1
    }
}

pub fn backslash_halve(p: &mut [u8]) {
    let len = crate::memory::bytes_until_nul(p).len();
    let mut i = 0usize;
    while i < len && !rem_backslash(&p[i..len]) {
        i += 1;
    }
    if i < len {
        let mut dst = i;
        p[dst] = p[i + 1];
        dst += 1;
        i += 2;
        while i < len {
            if rem_backslash(&p[i..len]) {
                p[dst] = p[i + 1];
                dst += 1;
                i += 2;
            } else {
                p[dst] = p[i];
                dst += 1;
                i += 1;
            }
        }
        if dst < p.len() {
            p[dst] = 0;
        }
    }
}

pub fn backslash_halve_save(bytes: &[u8]) -> CString {
    let mut res = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if rem_backslash(&bytes[i..]) {
            res.push(bytes[i + 1]);
            i += 2;
        } else {
            res.push(bytes[i]);
            i += 1;
        }
    }
    bytes_to_cstring(&res)
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
                s.to_bytes(),
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
