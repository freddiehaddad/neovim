//! Mathematical and bit-manipulation utilities (`src/nvim/math.c`).

/// Return value from Neovim functions (`src/nvim/vim_defs.h`).
pub const OK: i32 = 1;
pub const FAIL: i32 = 0;

pub const FP_NAN: i32 = if cfg!(any(
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)) {
    2
} else if cfg!(target_os = "macos") {
    1
} else {
    0
};

pub const FP_INFINITE: i32 = if cfg!(target_os = "macos") { 2 } else { 1 };

pub const FP_ZERO: i32 = if cfg!(target_os = "windows") {
    0
} else if cfg!(target_os = "macos") {
    3
} else if cfg!(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)) {
    0x10
} else {
    2
};

pub const FP_SUBNORMAL: i32 = if cfg!(target_os = "windows") {
    -2
} else if cfg!(target_os = "macos") {
    5
} else if cfg!(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)) {
    0x08
} else {
    3
};

pub const FP_NORMAL: i32 = if cfg!(target_os = "windows") { -1 } else { 4 };

/// Classify a `f64` into platform `<math.h>` `FP_*` categories (`xfpclassify`).
pub fn xfpclassify(d: f64) -> i32 {
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
pub fn xisinf(d: f64) -> i32 {
    i32::from(xfpclassify(d) == FP_INFINITE)
}

/// Return non-zero (1) if `d` is NaN (`xisnan`).
pub fn xisnan(d: f64) -> i32 {
    i32::from(xfpclassify(d) == FP_NAN)
}

/// Count trailing zeroes at the end of 64-bit bitfield (`xctz`).
pub fn xctz(x: u64) -> i32 {
    x.trailing_zeros() as i32
}

/// Count number of set bits in 64-bit bitfield (`xpopcount`).
pub fn xpopcount(x: u64) -> u32 {
    x.count_ones()
}

/// Append a decimal digit (`0..=9`) to `*value`, returning `FAIL` on `i32` overflow
/// (`vim_append_digit_int`).
pub fn vim_append_digit_int(value: &mut i32, digit: i32) -> bool {
    let x = *value;
    if x > (i32::MAX - digit) / 10 {
        return false;
    }
    *value = x * 10 + digit;
    true
}

/// Clamp a 64-bit signed integer into the range of `i32` (`trim_to_int`).
pub fn trim_to_int(x: i64) -> i32 {
    x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
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
        assert_eq!(xctz(4096), 12);
        assert_eq!(xctz(1 << 40), 40);
        assert_eq!(xctz(1 << 63), 63);

        assert_eq!(xpopcount(0), 0);
        assert_eq!(xpopcount(u64::MAX), 64);
        assert_eq!(xpopcount(0xf0), 4);
        assert_eq!(xpopcount(0b1010_1101), 5);
    }

    #[test]
    fn append_digit_and_trim() {
        let mut v: i32 = 0;
        for &d in &[2, 1, 4, 7, 4, 8, 3, 6, 4, 7] {
            assert!(vim_append_digit_int(&mut v, d));
        }
        assert_eq!(v, i32::MAX);
        assert!(!vim_append_digit_int(&mut v, 0));
        assert_eq!(v, i32::MAX);

        assert_eq!(trim_to_int(0), 0);
        assert_eq!(trim_to_int(i64::MAX), i32::MAX);
        assert_eq!(trim_to_int(i64::MIN), i32::MIN);
        assert_eq!(trim_to_int(12345), 12345);
    }
}
