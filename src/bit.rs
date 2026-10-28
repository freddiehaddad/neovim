//! Lua BitOp — a bit operations library for Lua 5.1/5.2 (`src/bit.c`).

use std::ffi::c_char;
#[cfg(feature = "vendor-bit")]
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
pub fn format_tohex(mut b: UBits, mut n: UBits) -> ([c_char; 8], usize) {
    let mut hexdigits: &[u8; 16] = b"0123456789abcdef";
    if (n as SBits) < 0 {
        n = (!n).wrapping_add(1);
        hexdigits = b"0123456789ABCDEF";
    }
    if n > 8 {
        n = 8;
    }
    let len = n as usize;
    let mut buf = [0 as c_char; 8];
    let mut i = len;
    while i > 0 {
        i -= 1;
        buf[i] = hexdigits[(b & 15) as usize] as c_char;
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
            let n = lua_tonumber(L, idx);
            let b = f64_to_ubits(n);
            if b == 0 && lua_isnumber(L, idx) == 0 {
                luaL_typerror(L, idx, c"number");
            }
            b
        }
    }

    #[inline]
    fn bret(lstate: *mut lua_State, b: UBits) -> i32 {
        unsafe {
            lua_pushnumber(L, f64::from(b as SBits));
            1
        }
    }

    fn bit_tobit(lstate: *mut lua_State) -> i32 {
        unsafe { bret(L, barg(L, 1)) }
    }

    fn bit_bnot(lstate: *mut lua_State) -> i32 {
        unsafe { bret(L, !barg(L, 1)) }
    }

    fn bit_band(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(L, 1);
            let mut i = lua_gettop(L);
            while i > 1 {
                b &= barg(L, i);
                i -= 1;
            }
            bret(L, b)
        }
    }

    fn bit_bor(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(L, 1);
            let mut i = lua_gettop(L);
            while i > 1 {
                b |= barg(L, i);
                i -= 1;
            }
            bret(L, b)
        }
    }

    fn bit_bxor(lstate: *mut lua_State) -> i32 {
        unsafe {
            let mut b = barg(L, 1);
            let mut i = lua_gettop(L);
            while i > 1 {
                b ^= barg(L, i);
                i -= 1;
            }
            bret(L, b)
        }
    }

    fn bit_lshift(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = barg(L, 2) & 31;
            bret(L, b.wrapping_shl(n))
        }
    }

    fn bit_rshift(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = barg(L, 2) & 31;
            bret(L, b.wrapping_shr(n))
        }
    }

    fn bit_arshift(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = barg(L, 2) & 31;
            bret(L, ((b as SBits).wrapping_shr(n)) as UBits)
        }
    }

    fn bit_rol(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = barg(L, 2) & 31;
            bret(L, b.rotate_left(n))
        }
    }

    fn bit_ror(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = barg(L, 2) & 31;
            bret(L, b.rotate_right(n))
        }
    }

    fn bit_bswap(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            bret(L, b.swap_bytes())
        }
    }

    fn bit_tohex(lstate: *mut lua_State) -> i32 {
        unsafe {
            let b = barg(L, 1);
            let n = if lua_type(L, 2) == LUA_TNONE {
                8
            } else {
                barg(L, 2)
            };
            let (buf, len) = format_tohex(b, n);
            lua_pushlstring(L, buf.as_ptr(), len);
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
            lua_pushnumber(L, 1_437_217_655.0);
            let b = barg(L, -1);
            if b != 1_437_217_655 {
                let mut msg = c"compiled with incompatible luaconf.h".as_ptr();
                if cfg!(windows) && b == 1_610_612_736 {
                    msg = c"use D3DCREATE_FPU_PRESERVE with DirectX".as_ptr();
                }
                if b == 1_127_743_488 {
                    msg = c"not compiled with SWAPPED_DOUBLE".as_ptr();
                }
                luaL_error(L, c"bit library self-test failed (%s)".as_ptr(), msg);
            }
            luaL_register(L, c"bit", BIT_FUNCS.as_ptr());
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
        let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
        assert_eq!(&bytes, b"deadbeef");

        let (buf_up, len_up) = format_tohex(0xdead_beef, (-4i32) as u32);
        let bytes_up: Vec<u8> = buf_up[..len_up].iter().map(|&c| c as u8).collect();
        assert_eq!(&bytes_up, b"BEEF");
    }
}
