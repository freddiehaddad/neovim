//! MessagePack primitive value packing and unpacking (`src/mpack/conv.c`).

use std::ffi::{c_char, c_double, c_int, c_uint};

use crate::mpack::mpack_core::{
    MPACK_TOKEN_ARRAY, MPACK_TOKEN_BIN, MPACK_TOKEN_BOOLEAN, MPACK_TOKEN_CHUNK, MPACK_TOKEN_EXT,
    MPACK_TOKEN_FLOAT, MPACK_TOKEN_MAP, MPACK_TOKEN_NIL, MPACK_TOKEN_SINT, MPACK_TOKEN_STR,
    MPACK_TOKEN_UINT, mpack_sint32_t, mpack_sintmax_t, mpack_token_data_t, mpack_token_t,
    mpack_uint32_t, mpack_uintmax_t, mpack_value_t,
};

#[inline]
fn pow2(n: c_uint) -> c_double {
    f64::from(1u32 << (n / 2)) * f64::from(1u32 << (n / 2)) * f64::from(1u32 << (n % 2))
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_nil() -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_NIL,
        length: 0,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_boolean(v: c_uint) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_BOOLEAN,
        length: 0,
        data: mpack_token_data_t {
            value: mpack_value_t {
                lo: u32::from(v != 0),
                hi: 0,
            },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_uint(v: mpack_uintmax_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_UINT,
        length: 0,
        data: mpack_token_data_t {
            value: mpack_value_t {
                lo: (v & 0xffffffff) as mpack_uint32_t,
                hi: ((v >> 31) >> 1) as mpack_uint32_t,
            },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_sint(v: mpack_sintmax_t) -> mpack_token_t {
    if v < 0 {
        let tc = v as mpack_uintmax_t;
        let mut rv = mpack_pack_uint(tc);
        rv.r#type = MPACK_TOKEN_SINT;
        return rv;
    }

    mpack_pack_uint(v as mpack_uintmax_t)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_float_compat(v: c_double) -> mpack_token_t {
    let (length, value) = if mpack_fits_single(v) {
        (4, mpack_pack_ieee754(v, 23, 8))
    } else {
        (8, mpack_pack_ieee754(v, 52, 11))
    };

    mpack_token_t {
        r#type: MPACK_TOKEN_FLOAT,
        length,
        data: mpack_token_data_t { value },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_float_fast(v: c_double) -> mpack_token_t {
    let (length, value) = if mpack_fits_single(v) {
        (
            4,
            mpack_value_t {
                lo: (v as f32).to_bits(),
                hi: 0,
            },
        )
    } else {
        let bits = v.to_bits();
        (
            8,
            mpack_value_t {
                lo: (bits & 0xffffffff) as mpack_uint32_t,
                hi: (bits >> 32) as mpack_uint32_t,
            },
        )
    };

    mpack_token_t {
        r#type: MPACK_TOKEN_FLOAT,
        length,
        data: mpack_token_data_t { value },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_number(v: c_double) -> mpack_token_t {
    if !(-9223372036854775808.0..18446744073709551616.0).contains(&v) {
        return mpack_pack_float_fast(v);
    }
    let vabs = if v < 0.0 { -v } else { v };
    let mut hi = (vabs / pow2(32)) as mpack_uint32_t;
    let mut lo = mpack_fmod_pow2_32(vabs) as mpack_uint32_t;
    let r#type;
    let length;

    if v < 0.0 {
        r#type = MPACK_TOKEN_SINT;
        hi = !hi;
        lo = (!lo).wrapping_add(1);
        if lo == 0 {
            hi = hi.wrapping_add(1);
        }
        if lo == 0 && hi == 0 {
            length = 1;
        } else if hi != 0xffffffff || lo < 0x80000000 {
            length = 8;
        } else if lo < 0xffff8000 {
            length = 4;
        } else if lo < 0xffffff80 {
            length = 2;
        } else {
            length = 1;
        }
    } else {
        r#type = MPACK_TOKEN_UINT;
        if hi != 0 {
            length = 8;
        } else if lo > 0xffff {
            length = 4;
        } else if lo > 0xff {
            length = 2;
        } else {
            length = 1;
        }
    }

    let tok = mpack_token_t {
        r#type,
        length,
        data: mpack_token_data_t {
            value: mpack_value_t { lo, hi },
        },
    };

    if mpack_unpack_number(tok) != v {
        return mpack_pack_float_fast(v);
    }

    tok
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_chunk(p: *const c_char, l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_CHUNK,
        length: l,
        data: mpack_token_data_t { chunk_ptr: p },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_str(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_STR,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_bin(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_BIN,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_ext(t: c_int, l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_EXT,
        length: l,
        data: mpack_token_data_t { ext_type: t },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_array(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_ARRAY,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_pack_map(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_MAP,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_boolean(t: mpack_token_t) -> bool {
    let val = unsafe { t.data.value };
    val.lo != 0 || val.hi != 0
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_uint(t: mpack_token_t) -> mpack_uintmax_t {
    let val = unsafe { t.data.value };
    (((val.hi as mpack_uintmax_t) << 31) << 1) | (val.lo as mpack_uintmax_t)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_sint(t: mpack_token_t) -> mpack_sintmax_t {
    let val = unsafe { t.data.value };
    let hi = val.hi;
    let lo = val.lo;
    let mut rv = lo as mpack_uintmax_t;
    debug_assert!((t.length as usize) <= std::mem::size_of::<mpack_sintmax_t>());

    if t.length == 8 {
        rv |= ((hi as mpack_uintmax_t) << 31) << 1;
    }
    let mask = ((1 as mpack_uintmax_t) << ((t.length * 8) - 1)).wrapping_sub(1);
    rv = (!rv & mask).wrapping_add(1);
    -((rv.wrapping_sub(1)) as mpack_sintmax_t) - 1
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_float_compat(t: mpack_token_t) -> c_double {
    let val = unsafe { t.data.value };
    if val.lo == 0 && val.hi == 0 {
        return 0.0;
    }

    let (mantbits, expbits): (c_uint, c_uint) = if t.length == 4 { (23, 8) } else { (52, 11) };
    let bias: mpack_sint32_t = (1 << (expbits - 1)) - 1;

    let (sign, mut exponent, mut mant) = if mantbits == 52 {
        let s = val.hi >> 31;
        let e = ((val.hi >> 20) & ((1 << 11) - 1)) as mpack_sint32_t;
        let m = f64::from(val.hi & ((1 << 20) - 1)) * pow2(32) + f64::from(val.lo);
        (s, e, m)
    } else {
        let s = val.lo >> 31;
        let e = ((val.lo >> 23) & ((1 << 8) - 1)) as mpack_sint32_t;
        let m = f64::from(val.lo & ((1 << 23) - 1));
        (s, e, m)
    };

    mant /= pow2(mantbits);
    if exponent != 0 {
        mant += 1.0;
    } else {
        exponent = 1;
    }
    exponent -= bias;

    while exponent > 0 {
        mant *= 2.0;
        exponent -= 1;
    }
    while exponent < 0 {
        mant /= 2.0;
        exponent += 1;
    }
    mant * if sign != 0 { -1.0 } else { 1.0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_float_fast(t: mpack_token_t) -> c_double {
    let val = unsafe { t.data.value };
    if t.length == 4 {
        f64::from(f32::from_bits(val.lo))
    } else {
        let bits = (u64::from(val.hi) << 32) | u64::from(val.lo);
        f64::from_bits(bits)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn mpack_unpack_number(t: mpack_token_t) -> c_double {
    if t.r#type == MPACK_TOKEN_FLOAT {
        return mpack_unpack_float_fast(t);
    }
    debug_assert!(t.r#type == MPACK_TOKEN_UINT || t.r#type == MPACK_TOKEN_SINT);
    let val = unsafe { t.data.value };
    let mut hi = val.hi;
    let mut lo = val.lo;
    if t.r#type == MPACK_TOKEN_SINT {
        if hi == 0 {
            debug_assert!(t.length <= 4);
            let mask = (1u32 << ((t.length * 8) - 1)).wrapping_sub(1);
            lo = !lo & mask;
        } else {
            hi = !hi;
            lo = !lo;
        }
        lo = lo.wrapping_add(1);
        if lo == 0 {
            hi = hi.wrapping_add(1);
        }
    }
    let rv = f64::from(lo) + pow2(32) * f64::from(hi);
    if t.r#type == MPACK_TOKEN_SINT {
        -rv
    } else {
        rv
    }
}

#[inline]
fn mpack_fits_single(v: c_double) -> bool {
    f64::from(v as f32) == v
}

fn mpack_pack_ieee754(v: c_double, mantbits: c_uint, expbits: c_uint) -> mpack_value_t {
    let mut rv = mpack_value_t { lo: 0, hi: 0 };
    let bias: mpack_sint32_t = (1 << (expbits - 1)) - 1;

    if v == 0.0 {
        return rv;
    }

    let (sign, mut mant) = if v < 0.0 { (1u32, -v) } else { (0u32, v) };
    let mut exponent: mpack_sint32_t = 0;

    while mant >= 2.0 {
        mant /= 2.0;
        exponent += 1;
    }
    while mant < 1.0 && exponent > -(bias - 1) {
        mant *= 2.0;
        exponent -= 1;
    }

    if mant < 1.0 {
        exponent = -bias;
    } else {
        mant -= 1.0;
    }
    exponent += bias;
    mant *= pow2(mantbits);

    if mantbits == 52 {
        rv.hi = (mant / pow2(32)) as mpack_uint32_t;
        rv.lo = (mant - f64::from(rv.hi) * pow2(32)) as mpack_uint32_t;
        rv.hi |= ((exponent as mpack_uint32_t) << 20) | (sign << 31);
    } else if mantbits == 23 {
        rv.hi = 0;
        rv.lo = mant as mpack_uint32_t;
        rv.lo |= ((exponent as mpack_uint32_t) << 23) | (sign << 31);
    }

    rv
}

#[inline]
fn mpack_fmod_pow2_32(a: c_double) -> c_double {
    a - (f64::from((a / pow2(32)) as mpack_uint32_t) * pow2(32))
}
