//! Floating point conversion routines (`src/cjson/fpconv.c`).

use std::ffi::{c_char, c_double, c_int, c_void};
use std::ptr;

pub const FPCONV_G_FMT_BUFSIZE: usize = 32;

static mut LOCALE_DECIMAL_POINT: c_char = b'.' as c_char;

use crate::strings::strchr;

#[cfg_attr(all(test, windows), link(name = "legacy_stdio_definitions"))]
unsafe extern "C" {
    fn snprintf(s: *mut c_char, maxlen: usize, format: *const c_char, ...) -> c_int;
    fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

unsafe fn fpconv_update_locale() {
    let mut buf = [0 as c_char; 8];
    unsafe {
        snprintf(buf.as_mut_ptr(), buf.len(), c"%g".as_ptr(), 0.5f64);
        if buf[0] != b'0' as c_char || buf[2] != b'5' as c_char || buf[3] != 0 {
            eprint!("Error: wide characters found or printf() bug.");
            abort();
        }
        LOCALE_DECIMAL_POINT = buf[1];
    }
}

#[inline]
fn valid_number_character(ch: c_char) -> bool {
    let u = ch as u8;
    if u.is_ascii_digit() || u == b'-' || u == b'+' || u == b'.' {
        return true;
    }
    let lower_ch = u | 0x20;
    (b'a'..=b'y').contains(&lower_ch)
}

unsafe fn strtod_buffer_size(s: *const c_char) -> c_int {
    let mut p = s;
    unsafe {
        while valid_number_character(*p) {
            p = p.add(1);
        }
        p.offset_from(s) as c_int
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fpconv_strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double {
    unsafe {
        if LOCALE_DECIMAL_POINT == b'.' as c_char {
            return strtod(nptr, endptr);
        }

        let buflen = strtod_buffer_size(nptr);
        if buflen == 0 {
            *endptr = nptr.cast_mut();
            return 0.0;
        }

        let mut localbuf = [0 as c_char; FPCONV_G_FMT_BUFSIZE];
        let buf = if (buflen as usize) >= FPCONV_G_FMT_BUFSIZE {
            let p = malloc((buflen as usize) + 1).cast::<c_char>();
            if p.is_null() {
                eprint!("Out of memory");
                abort();
            }
            p
        } else {
            localbuf.as_mut_ptr()
        };

        ptr::copy_nonoverlapping(nptr, buf, buflen as usize);
        *buf.offset(buflen as isize) = 0;

        let dp = strchr(buf, c_int::from(b'.'));
        if !dp.is_null() {
            *dp = LOCALE_DECIMAL_POINT;
        }

        let mut endbuf: *mut c_char = ptr::null_mut();
        let value = strtod(buf, &mut endbuf);
        *endptr = nptr.offset(endbuf.offset_from(buf)).cast_mut();
        if (buflen as usize) >= FPCONV_G_FMT_BUFSIZE {
            free(buf.cast::<c_void>());
        }

        value
    }
}

unsafe fn set_number_format(fmt: *mut c_char, precision: c_int) {
    debug_assert!((1..=16).contains(&precision));
    let d1 = precision / 10;
    let d2 = precision % 10;
    unsafe {
        *fmt = b'%' as c_char;
        *fmt.add(1) = b'.' as c_char;
        let mut i = 2usize;
        if d1 != 0 {
            *fmt.add(i) = (b'0' + d1 as u8) as c_char;
            i += 1;
        }
        *fmt.add(i) = (b'0' + d2 as u8) as c_char;
        i += 1;
        *fmt.add(i) = b'g' as c_char;
        i += 1;
        *fmt.add(i) = 0;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fpconv_g_fmt(
    mut str_out: *mut c_char,
    num: c_double,
    precision: c_int,
) -> c_int {
    let mut buf = [0 as c_char; FPCONV_G_FMT_BUFSIZE];
    let mut fmt = [0 as c_char; 6];

    unsafe {
        set_number_format(fmt.as_mut_ptr(), precision);

        if LOCALE_DECIMAL_POINT == b'.' as c_char {
            return snprintf(str_out, FPCONV_G_FMT_BUFSIZE, fmt.as_ptr(), num);
        }

        let len = snprintf(buf.as_mut_ptr(), FPCONV_G_FMT_BUFSIZE, fmt.as_ptr(), num);

        let mut b = buf.as_ptr();
        loop {
            let c = *b;
            *str_out = if c == LOCALE_DECIMAL_POINT {
                b'.' as c_char
            } else {
                c
            };
            str_out = str_out.add(1);
            b = b.add(1);
            if c == 0 {
                break;
            }
        }

        len
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn fpconv_init() {
    unsafe {
        fpconv_update_locale();
    }
}
