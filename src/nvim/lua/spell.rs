//! Lua `vim.spell` bindings (`src/nvim/lua/spell.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::errors::*;
use crate::globals::*;
#[allow(unused_imports)]
use crate::lua::converter::luaL_register;
#[allow(unused_imports)]
use crate::lua::converter::{
    lua_createtable, lua_gettop, lua_pushinteger, lua_pushlstring, lua_pushstring, lua_rawseti,
    lua_tolstring, lua_type, luaL_argerror, luaL_error,
};
#[allow(unused_imports)]
use crate::message::emsg;
#[allow(unused_imports)]
use crate::spell::{parse_spelllang, spell_check};
#[allow(unused_imports)]
use crate::strings::gettext;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::eval::deprecated::synblock_T;
use crate::lua::base64::{lua_State, luaL_Reg};
use crate::lua::converter::lua_Integer;

pub use crate::ex_getln::hlf_T;

const NUL: c_char = 0;
const LUA_TSTRING: c_int = 4;

const HLF_SPB: hlf_T = 37;
const HLF_SPC: hlf_T = 38;
const HLF_SPR: hlf_T = 39;
const HLF_SPL: hlf_T = 40;
const HLF_COUNT: hlf_T = 76;

/// Check spelling in `str` (`nlua_spell_check`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C-unwind" fn nlua_spell_check(lstate: *mut lua_State) -> c_int {
    unsafe {
        if lua_gettop(lstate) < 1 {
            return luaL_error(lstate, c"Expected 1 argument".as_ptr());
        }

        if lua_type(lstate, 1) != LUA_TSTRING {
            luaL_argerror(lstate, 1, c"expected string".as_ptr());
        }

        let mut str_ = lua_tolstring(lstate, 1, ptr::null_mut());

        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;

        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }

        let w_s = (*curwin).w_s.cast::<synblock_T>();
        if *(*w_s).b_p_spl == NUL {
            emsg(gettext(e_no_spell.as_ptr()));
            (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
            return 0;
        }

        let mut pos: usize = 0;
        let mut capcol: c_int = -1;
        let mut no_res: c_int = 0;

        lua_createtable(lstate, 0, 0);

        while *str_ != NUL {
            let mut attr: hlf_T = HLF_COUNT;
            let len = spell_check(
                curwin,
                str_.cast_mut(),
                &raw mut attr,
                &raw mut capcol,
                false,
            );
            assert!(len <= c_int::MAX as usize);

            if attr != HLF_COUNT {
                lua_createtable(lstate, 3, 0);

                lua_pushlstring(lstate, str_, len);
                lua_rawseti(lstate, -2, 1);

                let result = match attr {
                    HLF_SPB => c"bad".as_ptr(),
                    HLF_SPR => c"rare".as_ptr(),
                    HLF_SPL => c"local".as_ptr(),
                    HLF_SPC => c"caps".as_ptr(),
                    _ => ptr::null(),
                };
                assert!(!result.is_null());

                lua_pushstring(lstate, result);
                lua_rawseti(lstate, -2, 2);

                lua_pushinteger(lstate, (pos as lua_Integer) + 1);
                lua_rawseti(lstate, -2, 3);

                no_res += 1;
                lua_rawseti(lstate, -2, no_res);
            }

            str_ = str_.add(len);
            pos += len;
            capcol -= len as c_int;
        }

        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
        1
    }
}

/// Open the `vim.spell` Lua library (`luaopen_spell`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn luaopen_spell(lstate: *mut lua_State) -> c_int {
    const SPELL_FUNCTIONS: [luaL_Reg; 2] = [
        luaL_Reg {
            name: c"check".as_ptr(),
            func: Some(nlua_spell_check),
        },
        luaL_Reg {
            name: ptr::null(),
            func: None,
        },
    ];
    unsafe {
        lua_createtable(lstate, 0, 0);
        luaL_register(lstate, ptr::null(), SPELL_FUNCTIONS.as_ptr());
        1
    }
}
