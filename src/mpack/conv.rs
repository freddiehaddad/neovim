//! MessagePack primitive value packing and unpacking (`src/mpack/conv.c`).

use std::ffi::{c_char, c_double, c_int, c_uint};

use crate::mpack::mpack_core::{
    MPACK_TOKEN_ARRAY, MPACK_TOKEN_BIN, MPACK_TOKEN_BOOLEAN, MPACK_TOKEN_CHUNK, MPACK_TOKEN_EXT,
    MPACK_TOKEN_FLOAT, MPACK_TOKEN_MAP, MPACK_TOKEN_NIL, MPACK_TOKEN_SINT, MPACK_TOKEN_STR,
    MPACK_TOKEN_UINT, mpack_sintmax_t, mpack_token_data_t, mpack_token_t, mpack_uint32_t,
    mpack_uintmax_t, mpack_value_t,
};

#[inline]
fn pow2(n: c_uint) -> c_double {
    f64::from(1u32 << (n / 2)) * f64::from(1u32 << (n / 2)) * f64::from(1u32 << (n % 2))
}

pub fn mpack_pack_nil() -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_NIL,
        length: 0,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

pub fn mpack_pack_boolean(v: c_uint) -> mpack_token_t {
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

pub fn mpack_pack_float_fast(v: c_double) -> mpack_token_t {
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

#[unsafe(no_mangle)]
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

pub fn mpack_pack_chunk(p: *const c_char, l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_CHUNK,
        length: l,
        data: mpack_token_data_t { chunk_ptr: p },
    }
}

pub fn mpack_pack_str(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_STR,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

pub fn mpack_pack_bin(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_BIN,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

pub fn mpack_pack_ext(t: c_int, l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_EXT,
        length: l,
        data: mpack_token_data_t { ext_type: t },
    }
}

pub fn mpack_pack_array(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_ARRAY,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

pub fn mpack_pack_map(l: mpack_uint32_t) -> mpack_token_t {
    mpack_token_t {
        r#type: MPACK_TOKEN_MAP,
        length: l,
        data: mpack_token_data_t {
            value: mpack_value_t { lo: 0, hi: 0 },
        },
    }
}

pub fn mpack_unpack_boolean(t: mpack_token_t) -> bool {
    let val = unsafe { t.data.value };
    val.lo != 0 || val.hi != 0
}

pub fn mpack_unpack_uint(t: mpack_token_t) -> mpack_uintmax_t {
    let val = unsafe { t.data.value };
    (((val.hi as mpack_uintmax_t) << 31) << 1) | (val.lo as mpack_uintmax_t)
}

pub fn mpack_unpack_sint(t: mpack_token_t) -> mpack_sintmax_t {
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

pub fn mpack_unpack_float_fast(t: mpack_token_t) -> c_double {
    let val = unsafe { t.data.value };
    if t.length == 4 {
        f64::from(f32::from_bits(val.lo))
    } else {
        let bits = (u64::from(val.hi) << 32) | u64::from(val.lo);
        f64::from_bits(bits)
    }
}

pub fn mpack_unpack_number(t: mpack_token_t) -> c_double {
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

#[inline]
fn mpack_fmod_pow2_32(a: c_double) -> c_double {
    a - (f64::from((a / pow2(32)) as mpack_uint32_t) * pow2(32))
}
