//! Mathematical and bit-manipulation utilities (`src/nvim/math.c`).

use std::ffi::{c_int, c_uint};

/// Return value from Neovim functions (`src/nvim/vim_defs.h`).
pub const OK: c_int = 1;
pub const FAIL: c_int = 0;

#[cfg(target_os = "windows")]
mod fp_consts {
    use std::ffi::c_int;
    pub const FP_INFINITE: c_int = 1;
    pub const FP_NAN: c_int = 2;
    pub const FP_NORMAL: c_int = -1;
    pub const FP_SUBNORMAL: c_int = -2;
    pub const FP_ZERO: c_int = 0;
}

#[cfg(target_os = "macos")]
mod fp_consts {
    pub const FP_NAN: c_int = 1;
    pub const FP_INFINITE: c_int = 2;
    pub const FP_ZERO: c_int = 3;
    pub const FP_NORMAL: c_int = 4;
    pub const FP_SUBNORMAL: c_int = 5;
}

#[cfg(any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
mod fp_consts {
    use std::ffi::c_int;
    pub const FP_INFINITE: c_int = 0x01;
    pub const FP_NAN: c_int = 0x02;
    pub const FP_NORMAL: c_int = 0x04;
    pub const FP_SUBNORMAL: c_int = 0x08;
    pub const FP_ZERO: c_int = 0x10;
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
mod fp_consts {
    use std::ffi::c_int;
    pub const FP_NAN: c_int = 0;
    pub const FP_INFINITE: c_int = 1;
    pub const FP_ZERO: c_int = 2;
    pub const FP_SUBNORMAL: c_int = 3;
    pub const FP_NORMAL: c_int = 4;
}

pub use fp_consts::{FP_INFINITE, FP_NAN, FP_NORMAL, FP_SUBNORMAL, FP_ZERO};

/// Classify a `f64` into platform `<math.h>` `FP_*` categories (`xfpclassify`).
#[unsafe(no_mangle)]
pub extern "C" fn xfpclassify(d: f64) -> c_int {
    let bits = d.to_bits();
    let e = (bits >> 52) & 0x7ff;
    let m = bits & 0x000f_ffff_ffff_ffff;

    match e {
        0x000 => {
            if m != 0 {
                FP_SUBNORMAL
            } else {
                FP_ZERO
            }
        }
        0x7ff => {
            if m != 0 {
                FP_NAN
            } else {
                FP_INFINITE
            }
        }
        _ => FP_NORMAL,
    }
}

/// Return non-zero (1) if `d` is positive or negative infinity (`xisinf`).
#[unsafe(no_mangle)]
pub extern "C" fn xisinf(d: f64) -> c_int {
    c_int::from(xfpclassify(d) == FP_INFINITE)
}

/// Return non-zero (1) if `d` is NaN (`xisnan`).
#[unsafe(no_mangle)]
pub extern "C" fn xisnan(d: f64) -> c_int {
    c_int::from(xfpclassify(d) == FP_NAN)
}

/// Count trailing zeroes at the end of 64-bit bitfield (`xctz`).
#[unsafe(no_mangle)]
pub extern "C" fn xctz(x: u64) -> c_int {
    x.trailing_zeros() as c_int
}

/// Count number of set bits in 64-bit bitfield (`xpopcount`).
#[unsafe(no_mangle)]
pub extern "C" fn xpopcount(x: u64) -> c_uint {
    x.count_ones() as c_uint
}

/// Append a decimal digit (`0..=9`) to `*value`, returning `FAIL` on `c_int` overflow
/// (`vim_append_digit_int`).
///
/// # Safety
/// `value` must be a valid pointer to a writable `c_int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vim_append_digit_int(value: *mut c_int, digit: c_int) -> c_int {
    let x = unsafe { *value };
    if x > (c_int::MAX - digit) / 10 {
        return FAIL;
    }
    unsafe {
        *value = x * 10 + digit;
    }
    OK
}

/// Clamp a 64-bit signed integer into the range of `c_int` (`trim_to_int`).
#[unsafe(no_mangle)]
pub extern "C" fn trim_to_int(x: i64) -> c_int {
    x.clamp(i64::from(c_int::MIN), i64::from(c_int::MAX)) as c_int
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fpclassify_and_predicates() {
        assert_eq!(xfpclassify(0.0), FP_ZERO);
        assert_eq!(xfpclassify(-0.0), FP_ZERO);
        assert_eq!(xfpclassify(1.0), FP_NORMAL);
        assert_eq!(xfpclassify(-42.5), FP_NORMAL);
        assert_eq!(xfpclassify(f64::from_bits(1)), FP_SUBNORMAL);
        assert_eq!(xfpclassify(f64::INFINITY), FP_INFINITE);
        assert_eq!(xfpclassify(f64::NEG_INFINITY), FP_INFINITE);
        assert_eq!(xfpclassify(f64::NAN), FP_NAN);

        assert_eq!(xisinf(f64::INFINITY), 1);
        assert_eq!(xisinf(f64::NEG_INFINITY), 1);
        assert_eq!(xisinf(f64::NAN), 0);
        assert_eq!(xisinf(0.0), 0);

        assert_eq!(xisnan(f64::NAN), 1);
        assert_eq!(xisnan(f64::INFINITY), 0);
        assert_eq!(xisnan(1.0), 0);
    }

    #[test]
    fn bit_counts() {
        assert_eq!(xctz(0), 64);
        assert_eq!(xctz(1), 0);
        assert_eq!(xctz(1 << 40), 40);
        assert_eq!(xctz(1 << 63), 63);

        assert_eq!(xpopcount(0), 0);
        assert_eq!(xpopcount(u64::MAX), 64);
        assert_eq!(xpopcount(0b1010_1101), 5);
    }

    #[test]
    fn append_digit_and_trim() {
        let mut v: c_int = 0;
        for &d in &[2, 1, 4, 7, 4, 8, 3, 6, 4, 7] {
            assert_eq!(unsafe { vim_append_digit_int(&mut v, d) }, OK);
        }
        assert_eq!(v, c_int::MAX);
        assert_eq!(unsafe { vim_append_digit_int(&mut v, 0) }, FAIL);
        assert_eq!(v, c_int::MAX);

        assert_eq!(trim_to_int(0), 0);
        assert_eq!(trim_to_int(i64::MAX), c_int::MAX);
        assert_eq!(trim_to_int(i64::MIN), c_int::MIN);
        assert_eq!(trim_to_int(12345), 12345);
    }
}
