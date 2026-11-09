#![forbid(unsafe_code)]

//! Lua BitOp — a bit operations library for Lua 5.1/5.2 (`src/bit.c`).

#[cfg(feature = "vendor-bit")]
use crate::lua::converter::{LUA_TNONE, lua_State, luaL_typerror};
#[cfg(feature = "vendor-bit")]
use crate::lua::ffi::{
    lua_isnumber_safe, lua_push_bytes, lua_pushnumber_safe, lua_tonumber_safe, lua_type_safe,
    luaL_error_cstr_arg, luaL_register_safe, nlua_gettop,
};
pub type SBits = i32;
pub type UBits = u32;

#[inline]
pub const fn f64_to_ubits(n: f64) -> UBits {
    // 2^52 + 2^51 = 6755399441055744.0
    (n + 6_755_399_441_055_744.0).to_bits() as UBits
}

#[inline]
pub fn format_tohex(mut b: UBits, mut n: UBits) -> ([u8; 8], usize) {
    let mut hexdigits: &[u8; 16] = b"0123456789abcdef";
    if (n as SBits) < 0 {
        n = (!n).wrapping_add(1);
        hexdigits = b"0123456789ABCDEF";
    }
    if n > 8 {
        n = 8;
    }
    let len = n as usize;
    let mut buf = [0u8; 8];
    let mut i = len;
    while i > 0 {
        i -= 1;
        buf[i] = hexdigits[(b & 15) as usize];
        b >>= 4;
    }
    (buf, len)
}

#[cfg(feature = "vendor-bit")]
mod lua_c_api {
    use super::*;
    use std::ffi::CStr;

    #[inline]
    fn barg(lstate: *mut lua_State, idx: i32) -> UBits {
        let n = lua_tonumber_safe(lstate, idx);
        let b = f64_to_ubits(n);
        if b == 0 && !lua_isnumber_safe(lstate, idx) {
            luaL_typerror(lstate, idx, c"number");
        }
        b
    }

    #[inline]
    fn bret(lstate: *mut lua_State, b: UBits) -> i32 {
        lua_pushnumber_safe(lstate, f64::from(b as SBits));
        1
    }

    extern "C-unwind" fn bit_tobit(lstate: *mut lua_State) -> i32 {
        bret(lstate, barg(lstate, 1))
    }

    extern "C-unwind" fn bit_bnot(lstate: *mut lua_State) -> i32 {
        bret(lstate, !barg(lstate, 1))
    }

    extern "C-unwind" fn bit_band(lstate: *mut lua_State) -> i32 {
        let mut b = barg(lstate, 1);
        let mut i = nlua_gettop(lstate);
        while i > 1 {
            b &= barg(lstate, i);
            i -= 1;
        }
        bret(lstate, b)
    }

    extern "C-unwind" fn bit_bor(lstate: *mut lua_State) -> i32 {
        let mut b = barg(lstate, 1);
        let mut i = nlua_gettop(lstate);
        while i > 1 {
            b |= barg(lstate, i);
            i -= 1;
        }
        bret(lstate, b)
    }

    extern "C-unwind" fn bit_bxor(lstate: *mut lua_State) -> i32 {
        let mut b = barg(lstate, 1);
        let mut i = nlua_gettop(lstate);
        while i > 1 {
            b ^= barg(lstate, i);
            i -= 1;
        }
        bret(lstate, b)
    }

    extern "C-unwind" fn bit_lshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.wrapping_shl(n))
    }

    extern "C-unwind" fn bit_rshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.wrapping_shr(n))
    }

    extern "C-unwind" fn bit_arshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, ((b as SBits).wrapping_shr(n)) as UBits)
    }

    extern "C-unwind" fn bit_rol(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.rotate_left(n))
    }

    extern "C-unwind" fn bit_ror(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.rotate_right(n))
    }

    extern "C-unwind" fn bit_bswap(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        bret(lstate, b.swap_bytes())
    }

    extern "C-unwind" fn bit_tohex(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = if lua_type_safe(lstate, 2) == LUA_TNONE {
            8
        } else {
            barg(lstate, 2)
        };
        let (buf, len) = format_tohex(b, n);
        lua_push_bytes(lstate, &buf[..len]);
        1
    }

    static BIT_FUNCS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 12] = [
        (c"tobit", bit_tobit),
        (c"bnot", bit_bnot),
        (c"band", bit_band),
        (c"bor", bit_bor),
        (c"bxor", bit_bxor),
        (c"lshift", bit_lshift),
        (c"rshift", bit_rshift),
        (c"arshift", bit_arshift),
        (c"rol", bit_rol),
        (c"ror", bit_ror),
        (c"bswap", bit_bswap),
        (c"tohex", bit_tohex),
    ];

    pub fn nvim_luaopen_bit(lstate: *mut lua_State) -> i32 {
        lua_pushnumber_safe(lstate, 1_437_217_655.0);
        let b = barg(lstate, -1);
        if b != 1_437_217_655 {
            let mut msg = c"compiled with incompatible luaconf.h";
            if cfg!(windows) && b == 1_610_612_736 {
                msg = c"use D3DCREATE_FPU_PRESERVE with DirectX";
            }
            if b == 1_127_743_488 {
                msg = c"not compiled with SWAPPED_DOUBLE";
            }
            luaL_error_cstr_arg(lstate, c"bit library self-test failed (%s)", msg);
        }
        luaL_register_safe(lstate, Some(c"bit"), &BIT_FUNCS);
        1
    }
}

#[cfg(feature = "vendor-bit")]
pub use lua_c_api::nvim_luaopen_bit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_to_ubits_and_tohex_match_bitop_semantics() {
        assert_eq!(f64_to_ubits(1_437_217_655.0), 1_437_217_655);
        assert_eq!(f64_to_ubits(-1.0), 0xffff_ffff);
        assert_eq!(f64_to_ubits(0x8000_0000u32 as f64) as SBits, -2_147_483_648);

        let (buf, len) = format_tohex(0xdead_beef, 8);
        assert_eq!(&buf[..len], b"deadbeef");

        let (buf_up, len_up) = format_tohex(0xdead_beef, (-4i32) as u32);
        assert_eq!(&buf_up[..len_up], b"BEEF");
    }
}
