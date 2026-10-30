//! Lua BitOp — a bit operations library for Lua 5.1/5.2 (`src/bit.c`).

#[cfg(feature = "vendor-bit")]
use std::ptr;

#[cfg(feature = "vendor-bit")]
use crate::lua::converter::{
    LUA_TNONE, lua_State, lua_gettop, lua_isnumber, lua_pushlstring, lua_pushnumber, lua_tonumber,
    lua_type, luaL_Reg, luaL_error, luaL_register, luaL_typerror,
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

    #[inline]
    fn barg(lstate: *mut lua_State, idx: i32) -> UBits {
        unsafe {
            let n = lua_tonumber(lstate, idx);
            let b = f64_to_ubits(n);
            if b == 0 && lua_isnumber(lstate, idx) == 0 {
                luaL_typerror(lstate, idx, c"number".as_ptr());
            }
            b
        }
    }

    #[inline]
    fn bret(lstate: *mut lua_State, b: UBits) -> i32 {
        unsafe {
            lua_pushnumber(lstate, f64::from(b as SBits));
            1
        }
    }

    unsafe extern "C-unwind" fn bit_tobit(lstate: *mut lua_State) -> i32 {
        bret(lstate, barg(lstate, 1))
    }

    unsafe extern "C-unwind" fn bit_bnot(lstate: *mut lua_State) -> i32 {
        bret(lstate, !barg(lstate, 1))
    }

    unsafe extern "C-unwind" fn bit_band(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(lstate, 1);
            let mut i = lua_gettop(lstate);
            while i > 1 {
                b &= barg(lstate, i);
                i -= 1;
            }
            bret(lstate, b)
        }
    }

    unsafe extern "C-unwind" fn bit_bor(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(lstate, 1);
            let mut i = lua_gettop(lstate);
            while i > 1 {
                b |= barg(lstate, i);
                i -= 1;
            }
            bret(lstate, b)
        }
    }

    unsafe extern "C-unwind" fn bit_bxor(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(lstate, 1);
            let mut i = lua_gettop(lstate);
            while i > 1 {
                b ^= barg(lstate, i);
                i -= 1;
            }
            bret(lstate, b)
        }
    }

    unsafe extern "C-unwind" fn bit_lshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.wrapping_shl(n))
    }

    unsafe extern "C-unwind" fn bit_rshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.wrapping_shr(n))
    }

    unsafe extern "C-unwind" fn bit_arshift(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, ((b as SBits).wrapping_shr(n)) as UBits)
    }

    unsafe extern "C-unwind" fn bit_rol(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.rotate_left(n))
    }

    unsafe extern "C-unwind" fn bit_ror(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        let n = barg(lstate, 2) & 31;
        bret(lstate, b.rotate_right(n))
    }

    unsafe extern "C-unwind" fn bit_bswap(lstate: *mut lua_State) -> i32 {
        let b = barg(lstate, 1);
        bret(lstate, b.swap_bytes())
    }

    unsafe extern "C-unwind" fn bit_tohex(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(lstate, 1);
            let n = if lua_type(lstate, 2) == LUA_TNONE {
                8
            } else {
                barg(lstate, 2)
            };
            let (buf, len) = format_tohex(b, n);
            lua_pushlstring(lstate, buf.as_ptr().cast(), len);
            1
        }
    }

    static BIT_FUNCS: [luaL_Reg; 13] = [
        luaL_Reg {
            name: c"tobit".as_ptr(),
            func: Some(bit_tobit),
        },
        luaL_Reg {
            name: c"bnot".as_ptr(),
            func: Some(bit_bnot),
        },
        luaL_Reg {
            name: c"band".as_ptr(),
            func: Some(bit_band),
        },
        luaL_Reg {
            name: c"bor".as_ptr(),
            func: Some(bit_bor),
        },
        luaL_Reg {
            name: c"bxor".as_ptr(),
            func: Some(bit_bxor),
        },
        luaL_Reg {
            name: c"lshift".as_ptr(),
            func: Some(bit_lshift),
        },
        luaL_Reg {
            name: c"rshift".as_ptr(),
            func: Some(bit_rshift),
        },
        luaL_Reg {
            name: c"arshift".as_ptr(),
            func: Some(bit_arshift),
        },
        luaL_Reg {
            name: c"rol".as_ptr(),
            func: Some(bit_rol),
        },
        luaL_Reg {
            name: c"ror".as_ptr(),
            func: Some(bit_ror),
        },
        luaL_Reg {
            name: c"bswap".as_ptr(),
            func: Some(bit_bswap),
        },
        luaL_Reg {
            name: c"tohex".as_ptr(),
            func: Some(bit_tohex),
        },
        luaL_Reg {
            name: ptr::null(),
            func: None,
        },
    ];

    pub fn nvim_luaopen_bit(lstate: *mut lua_State) -> i32 {
        unsafe {
            lua_pushnumber(lstate, 1_437_217_655.0);
            let b = barg(lstate, -1);
            if b != 1_437_217_655 {
                let mut msg = c"compiled with incompatible luaconf.h".as_ptr();
                if cfg!(windows) && b == 1_610_612_736 {
                    msg = c"use D3DCREATE_FPU_PRESERVE with DirectX".as_ptr();
                }
                if b == 1_127_743_488 {
                    msg = c"not compiled with SWAPPED_DOUBLE".as_ptr();
                }
                luaL_error(lstate, c"bit library self-test failed (%s)".as_ptr(), msg);
            }
            luaL_register(lstate, c"bit".as_ptr(), BIT_FUNCS.as_ptr());
            1
        }
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
