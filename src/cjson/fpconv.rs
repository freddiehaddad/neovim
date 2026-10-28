//! Floating point conversion routines (`src/cjson/fpconv.c`).

use std::ffi::c_char;
use std::io::Write;

use crate::strings::strtod;

pub const FPCONV_G_FMT_BUFSIZE: usize = 32;

pub fn fpconv_strtod(nptr: impl crate::strings::IntoCCharPtr, endptr: *mut *mut c_char) -> f64 {
    let nptr = nptr.into_c_char_ptr();
    strtod(nptr, endptr)
}

pub fn fpconv_g_fmt(buf: &mut [u8; FPCONV_G_FMT_BUFSIZE], num: f64, precision: i32) -> usize {
    let prec = precision.max(1) as usize;
    let mut pos = 0usize;

    if num.is_sign_negative() {
        buf[pos] = b'-';
        pos += 1;
    }
    let abs_num = num.abs();

    if abs_num.is_nan() {
        buf[pos..pos + 3].copy_from_slice(b"nan");
        pos += 3;
    } else if abs_num.is_infinite() {
        buf[pos..pos + 3].copy_from_slice(b"inf");
        pos += 3;
    } else {
        let sci = format!("{abs_num:.*e}", prec - 1);
        let (mantissa, exp_str) = sci.split_once('e').unwrap_or((&sci, "0"));
        let exp: i32 = exp_str.parse().unwrap_or(0);

        let mut digits = [0u8; 32];
        let mut dlen = 0usize;
        for &b in mantissa.as_bytes() {
            if b != b'.' && dlen < digits.len() {
                digits[dlen] = b;
                dlen += 1;
            }
        }
        while dlen > 1 && digits[dlen - 1] == b'0' {
            dlen -= 1;
        }

        if exp < -4 || exp >= prec as i32 {
            buf[pos] = digits[0];
            pos += 1;
            if dlen > 1 {
                buf[pos] = b'.';
                pos += 1;
                buf[pos..pos + (dlen - 1)].copy_from_slice(&digits[1..dlen]);
                pos += dlen - 1;
            }
            let mut cursor = std::io::Cursor::new(&mut buf[pos..FPCONV_G_FMT_BUFSIZE - 1]);
            let _ = write!(cursor, "e{exp:+03}");
            pos += cursor.position() as usize;
        } else if exp < 0 {
            buf[pos..pos + 2].copy_from_slice(b"0.");
            pos += 2;
            let zeros = (-1 - exp) as usize;
            buf[pos..pos + zeros].fill(b'0');
            pos += zeros;
            buf[pos..pos + dlen].copy_from_slice(&digits[..dlen]);
            pos += dlen;
        } else {
            let int_digits = (exp as usize) + 1;
            if int_digits >= dlen {
                buf[pos..pos + dlen].copy_from_slice(&digits[..dlen]);
                pos += dlen;
                let zeros = int_digits - dlen;
                buf[pos..pos + zeros].fill(b'0');
                pos += zeros;
            } else {
                buf[pos..pos + int_digits].copy_from_slice(&digits[..int_digits]);
                pos += int_digits;
                buf[pos] = b'.';
                pos += 1;
                let frac_len = dlen - int_digits;
                buf[pos..pos + frac_len].copy_from_slice(&digits[int_digits..dlen]);
                pos += frac_len;
            }
        }
    }

    buf[pos] = 0;
    pos
}
