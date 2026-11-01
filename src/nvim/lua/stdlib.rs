//! Lua standard library and internal `vim.*` C bindings (`src/nvim/lua/stdlib.c`).

use crate::api::private::helpers::{
    TryState, api_clear_error, dict_check_writable, find_buffer_by_handle, find_tab_by_handle,
    find_window_by_handle, try_enter, try_leave, tv_dict_is_watched,
};
use crate::api::private::validate::{Error, ErrorType, NvimString};
use crate::api::win_config::K_CTX_NO_DISPLAY;
use crate::api::window::{K_CTX_KEEP_CWD, K_CTX_VALIDATE};
use crate::buffer::FileBuf;
use crate::charset::NUL;
use crate::context::K_CTX_KEEP_DIRS;
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, handle_get_buffer, handle_get_window};
use crate::cursor::Linenr;
use crate::eval::typval::{
    TvDict, TypVal, tv_clear, tv_copy, tv_dict_add, tv_dict_find, tv_dict_item_alloc_len,
    tv_dict_item_remove, tv_dict_watcher_notify,
};
use crate::eval::vars::{before_set_vvar, get_globvar_dict, get_vimvar_dict};
use crate::ex_docmd::{
    CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS,
    CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, apply_cmdmod,
    undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::extmark::K_TRUE;
use crate::fold::fold_update;
use crate::globals::*;
use crate::keycodes::{KeyChord, MOD_MASK_TABLE, vim_strsave_escape_ks};
use crate::log::log_st;
use crate::lua::base64::luaopen_base64;
use crate::lua::converter::{
    LUA_MULTRET, LUA_REGISTRYINDEX, LUA_TSTRING, lua_Integer, lua_State, lua_concat,
    lua_createtable, lua_error, lua_getfield, lua_getglobal, lua_gettop, lua_isnil,
    lua_isnoneornil, lua_istable, lua_newtable, lua_newuserdata, lua_next, lua_objlen, lua_pcall,
    lua_pop, lua_pushcfunction, lua_pushinteger, lua_pushlstring, lua_pushnil, lua_pushnumber,
    lua_pushstring, lua_pushvalue, lua_rawseti, lua_setfield, lua_setmetatable, lua_toboolean,
    lua_tolstring, lua_tostring, lua_type, luaL_Reg, luaL_argerror, luaL_checkinteger,
    luaL_checklstring, luaL_checkstring, luaL_checkudata, luaL_error, luaL_newmetatable,
    luaL_register, luaL_where, nlua_pop_typval, nlua_push_typval,
};
use crate::lua::spell::luaopen_spell;
use crate::lua::xdiff::nlua_xdl_diff;
use crate::mbyte::{
    Vimconv, convert_setup_ext, enc_canonize, enc_skip, mb_stricmp, mb_utf_index_to_bytes,
    mb_utflen, string_convert, utf_cp_bounds_len, utf_ptr2len_len,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::message::str2special;
use crate::msgpack_rpc::packer::ObjHandle;
use crate::regexp::{
    RE_AUTO, RE_MAGIC, RE_NOBREAK, RE_STRICT, Regmatch, Regprog, vim_regcomp, vim_regexec,
    vim_regfree,
};
use crate::runtime::script_autoload;
use crate::viml::parser::parser::CONV_NONE;
use crate::window::{WinInfoInfo, win_find_tabpage};
use std::ffi::CStr;
use std::ptr;

static REGEX_META: [luaL_Reg; 5] = [
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(regex_gc),
    },
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(regex_tostring),
    },
    luaL_Reg {
        name: c"match_str".as_ptr(),
        func: Some(regex_match_str),
    },
    luaL_Reg {
        name: c"match_line".as_ptr(),
        func: Some(regex_match_line),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

use crate::cjson::lua_cjson::lua_cjson_new;
use crate::lpeg::lptree::luaopen_lpeg;
use crate::lua::converter::lua_isstring;
use crate::mpack::lmpack::luaopen_mpack;

fn regex_check(lstate: *mut lua_State) -> *mut *mut Regprog {
    luaL_checkudata(lstate, 1, c"nvim_regex").cast::<*mut Regprog>()
}

fn regex_match(lstate: *mut lua_State, prog: *mut *mut Regprog, str_: &CStr) -> i32 {
    let str_ptr = str_.as_ptr();
    let mut rm: Regmatch = Regmatch::new();
    rm.regprog = unsafe { *prog };
    rm.rm_ic = false;
    let matched = vim_regexec(&mut rm, str_ptr, 0);
    unsafe {
        *prog = rm.regprog;
    }

    if matched {
        unsafe {
            lua_pushinteger(
                lstate,
                ((rm.startp[0] as isize) - (str_ptr as isize)) as lua_Integer,
            )
        };
        unsafe {
            lua_pushinteger(
                lstate,
                ((rm.endp[0] as isize) - (str_ptr as isize)) as lua_Integer,
            )
        };
        return 2;
    }
    0
}

unsafe extern "C-unwind" fn regex_match_str(lstate: *mut lua_State) -> i32 {
    unsafe {
        let prog = regex_check(lstate);
        let str_ = luaL_checkstring(lstate, 2);
        let nret = regex_match(lstate, prog, crate::memory::cstr_from_ptr(str_));

        if (*prog).is_null() {
            return luaL_error(lstate, c"regex: internal error".as_ptr());
        }

        nret
    }
}

unsafe extern "C-unwind" fn regex_match_line(lstate: *mut lua_State) -> i32 {
    unsafe {
        let prog = regex_check(lstate);

        let narg = lua_gettop(lstate);
        if narg < 3 {
            return luaL_error(lstate, c"not enough args".as_ptr());
        }

        let bufnr = luaL_checkinteger(lstate, 2) as ObjHandle;
        let rownr = luaL_checkinteger(lstate, 3) as Linenr;
        let mut start: i32 = 0;
        let mut end: i32 = -1;
        if narg >= 4 {
            start = luaL_checkinteger(lstate, 4) as i32;
        }
        if narg >= 5 {
            end = luaL_checkinteger(lstate, 5) as i32;
            if end < 0 {
                return luaL_error(lstate, c"invalid end".as_ptr());
            }
        }

        let buf = if bufnr != 0 {
            handle_get_buffer(bufnr)
        } else {
            g_st().curbuf
        };
        if buf.is_null() || (*buf).b_ml.ml_mfp.is_none() {
            return luaL_error(lstate, c"invalid buffer".as_ptr());
        }

        if rownr >= (*buf).b_ml.ml_line_count {
            return luaL_error(lstate, c"invalid row".as_ptr());
        }

        let line = ml_get_buf(&mut *buf, rownr + 1);
        let len = ml_get_buf_len(&mut *buf, rownr + 1);

        if start < 0 || start > len {
            return luaL_error(lstate, c"invalid start".as_ptr());
        }

        let mut save = NUL;
        if end >= 0 {
            if end > len || end < start {
                return luaL_error(lstate, c"invalid end".as_ptr());
            }
            save = *line.wrapping_add(end as usize);
            *line.wrapping_add(end as usize) = NUL;
        }

        let nret = regex_match(
            lstate,
            prog,
            crate::memory::cstr_from_ptr(line.wrapping_add(start as usize)),
        );

        if end >= 0 {
            *line.wrapping_add(end as usize) = save;
        }

        if (*prog).is_null() {
            return luaL_error(lstate, c"regex: internal error".as_ptr());
        }

        nret
    }
}

unsafe extern "C-unwind" fn regex_gc(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let prog = regex_check(lstate);
            vim_regfree(*prog);
        }
        0
    }
}

unsafe extern "C-unwind" fn regex_tostring(lstate: *mut lua_State) -> i32 {
    {
        lua_pushstring(lstate, c"<regex>");
    }
    1
}

/// Convert byte index to UTF-32 and UTF-16 indices.
pub unsafe extern "C-unwind" fn nlua_str_utfindex(lstate: *mut lua_State) -> i32 {
    let mut s1_len: usize = 0;
    let s1 = unsafe { luaL_checklstring(lstate, 1, &raw mut s1_len) };
    let idx: isize = if lua_isnoneornil(lstate, 2) {
        s1_len as isize
    } else {
        let i = unsafe { luaL_checkinteger(lstate, 2) };
        if i < 0 || (i as usize) > s1_len {
            unsafe { lua_pushnil(lstate) };
            unsafe { lua_pushnil(lstate) };
            return 2;
        }
        i
    };

    let mut codepoints: usize = 0;
    let mut codeunits: usize = 0;
    mb_utflen(s1, idx as usize, &mut codepoints, &mut codeunits);

    unsafe { lua_pushinteger(lstate, codepoints as lua_Integer) };
    unsafe { lua_pushinteger(lstate, codeunits as lua_Integer) };

    2
}

unsafe extern "C-unwind" fn nlua_str_utf_pos(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        lua_newtable(lstate);

        let mut idx: i32 = 1;
        let mut i: usize = 0;
        while i < s1_len && crate::memory::cstr_byte_at(s1, i) != 0 {
            let clen = utf_ptr2len_len(s1.wrapping_add(i), (s1_len - i) as i32) as usize;
            lua_pushinteger(lstate, (i as lua_Integer) + 1);
            lua_rawseti(lstate, -2, idx);
            idx += 1;
            i += clen;
        }

        1
    }
}

unsafe extern "C-unwind" fn nlua_str_utf_start(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let offset = luaL_checkinteger(lstate, 2);
        if offset <= 0 || (offset as usize) > s1_len {
            return luaL_error(lstate, c"index out of range".as_ptr());
        }
        let off = (offset as usize) - 1;
        let head_off = -i32::from(
            utf_cp_bounds_len(s1, s1.wrapping_add(off), (s1_len - off) as i32).begin_off,
        );
        lua_pushinteger(lstate, head_off as lua_Integer);
        1
    }
}

unsafe extern "C-unwind" fn nlua_str_utf_end(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let offset = luaL_checkinteger(lstate, 2);
        if offset <= 0 || (offset as usize) > s1_len {
            return luaL_error(lstate, c"index out of range".as_ptr());
        }
        let off = (offset as usize) - 1;
        let tail_off =
            i32::from(utf_cp_bounds_len(s1, s1.wrapping_add(off), (s1_len - off) as i32).end_off)
                - 1;
        lua_pushinteger(lstate, tail_off as lua_Integer);
        1
    }
}

/// Convert UTF-32 or UTF-16 indices to byte index.
pub unsafe extern "C-unwind" fn nlua_str_byteindex(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut s1_len: usize = 0;
        let s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let idx = luaL_checkinteger(lstate, 2);
        if idx < 0 {
            lua_pushnil(lstate);
            return 1;
        }
        let mut use_utf16 = false;
        if lua_gettop(lstate) >= 3 {
            use_utf16 = lua_toboolean(lstate, 3) != 0;
        }

        let byteidx = mb_utf_index_to_bytes(s1, s1_len, idx as usize, use_utf16);
        if byteidx == -1 {
            lua_pushnil(lstate);
            return 1;
        }

        lua_pushinteger(lstate, byteidx as lua_Integer);
        1
    }
}

pub unsafe extern "C-unwind" fn nlua_regex(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut err = Error::new();
        let text = luaL_checkstring(lstate, 1);

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let prog = vim_regcomp(text, RE_AUTO | RE_MAGIC | RE_STRICT | RE_NOBREAK);
        try_leave(&tstate, &mut err);

        if err.err_type != ErrorType::ErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"couldn't parse regex: ".as_ptr().cast(), 22);
            lua_pushstring(lstate, err.msg_cstr());
            lua_concat(lstate, 3);
            api_clear_error(&mut err);
            return lua_error(lstate);
        } else if prog.is_null() {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"couldn't parse regex".as_ptr().cast(), 20);
            lua_concat(lstate, 2);
            return lua_error(lstate);
        }

        let p = lua_newuserdata(lstate, size_of::<*mut Regprog>()).cast::<*mut Regprog>();
        *p = prog;

        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim_regex");
        let _ = lua_setmetatable(lstate, -2);
        1
    }
}

fn nlua_get_var_scope(lstate: *mut lua_State) -> *mut TvDict {
    unsafe {
        let scope = luaL_checkstring(lstate, 1);
        let handle = luaL_checkinteger(lstate, 2) as ObjHandle;
        let mut dict: *mut TvDict = ptr::null_mut();
        let mut err = Error::new();

        match crate::memory::cstr_as_bytes(scope) {
            b"g" => dict = get_globvar_dict(),
            b"v" => dict = get_vimvar_dict(),
            b"b" => {
                let buf = find_buffer_by_handle(handle, &mut err);
                if !buf.is_null() {
                    dict = (*buf).b_vars();
                }
            }
            b"w" => {
                let win = find_window_by_handle(handle, &mut err);
                if !win.is_null() {
                    dict = (*win).w_vars();
                }
            }
            b"t" => {
                let tabpage = find_tab_by_handle(handle, &mut err);
                if !tabpage.is_null() {
                    dict = (*tabpage).tp_vars();
                }
            }
            _ => {
                luaL_error(lstate, c"invalid scope".as_ptr());
                return ptr::null_mut();
            }
        }

        if err.err_type != ErrorType::ErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushlstring(lstate, b"scoped variable: ".as_ptr().cast(), 17);
            lua_pushstring(lstate, err.msg_cstr());
            lua_concat(lstate, 3);
            api_clear_error(&mut err);
            lua_error(lstate);
            return ptr::null_mut();
        }
        dict
    }
}

pub unsafe extern "C-unwind" fn nlua_setvar(lstate: *mut lua_State) -> i32 {
    unsafe {
        let dict = nlua_get_var_scope(lstate);
        let mut key_size: usize = 0;
        let key_data = luaL_checklstring(lstate, 3, &raw mut key_size).cast_mut();
        let key = NvimString {
            data: key_data,
            size: key_size,
        };

        let del = lua_gettop(lstate) < 4 || lua_isnil(lstate, 4);

        let mut err = Error::new();
        let mut di = dict_check_writable(&mut *dict, key, del, &mut err);
        if err.err_type != ErrorType::ErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg_cstr());
            lua_concat(lstate, 2);
            api_clear_error(&mut err);
            lua_error(lstate);
            return 0;
        }

        let watched = tv_dict_is_watched(dict.as_ref());

        if del {
            if di.is_null() {
                return 0;
            }
            if watched {
                tv_dict_watcher_notify(&mut *dict, key.data, None, Some(&(*di).di_tv));
            }
            tv_dict_item_remove(&mut *dict, &mut *di);
        } else {
            let mut tv: TypVal = TypVal::new();

            lua_pushvalue(lstate, 4);
            if !nlua_pop_typval(lstate, &raw mut tv) {
                return luaL_error(lstate, c"Couldn't convert lua value".as_ptr());
            }

            let mut oldtv: TypVal = TypVal::new();

            if di.is_null() {
                di = tv_dict_item_alloc_len(key.data, key.size);
                let _ = tv_dict_add(&mut *dict, di);
            } else {
                let mut type_error = false;
                if dict == get_vimvar_dict()
                    && !before_set_vvar(key.data, &mut *di, &mut tv, true, watched, &mut type_error)
                {
                    tv_clear(&mut tv);
                    if type_error {
                        return luaL_error(
                            lstate,
                            c"Setting v:%s to value with wrong type".as_ptr(),
                            key.data,
                        );
                    }
                    return 0;
                }
                if watched {
                    tv_copy(&(*di).di_tv, &mut oldtv);
                }
                tv_clear(&mut (*di).di_tv);
            }

            tv_copy(&tv, &mut (*di).di_tv);

            if watched {
                tv_dict_watcher_notify(&mut *dict, key.data, Some(&tv), Some(&oldtv));
                tv_clear(&mut oldtv);
            }

            tv_clear(&mut tv);
        }
        0
    }
}

pub unsafe extern "C-unwind" fn nlua_getvar(lstate: *mut lua_State) -> i32 {
    unsafe {
        let dict = nlua_get_var_scope(lstate);
        let mut len: usize = 0;
        let name = luaL_checklstring(lstate, 3, &raw mut len);

        let mut di = tv_dict_find(dict.as_ref(), name, len as isize);
        if di.is_null() && dict == get_globvar_dict() {
            if !script_autoload(name, len, false) || aborting() {
                return 0;
            }
            di = tv_dict_find(dict.as_ref(), name, len as isize);
        }
        if di.is_null() {
            return 0;
        }
        nlua_push_typval(lstate, &raw mut (*di).di_tv, 0);
        1
    }
}

unsafe extern "C-unwind" fn nlua_stricmp(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut s1_len: usize = 0;
        let mut s2_len: usize = 0;
        let mut s1 = luaL_checklstring(lstate, 1, &raw mut s1_len);
        let mut s2 = luaL_checklstring(lstate, 2, &raw mut s2_len);
        let mut ret: i32;
        assert_eq!({ *s1.wrapping_add(s1_len) }, NUL);
        assert_eq!({ *s2.wrapping_add(s2_len) }, NUL);

        loop {
            let slice1 = crate::memory::cstr_slice(s1, s1_len);
            let slice2 = crate::memory::cstr_slice(s2, s2_len);
            let nul1 = slice1.iter().position(|&b| b == 0);
            let nul2 = slice2.iter().position(|&b| b == 0);
            ret = mb_stricmp(s1, s2);
            if ret == 0 {
                if nul1.is_some() != nul2.is_some() {
                    ret = i32::from(nul1.is_some()) - i32::from(nul2.is_some());
                    break;
                }
                if let (Some(p1), Some(p2)) = (nul1, nul2) {
                    s1_len -= p1 + 1;
                    s2_len -= p2 + 1;
                    s1 = s1.wrapping_add(p1 + 1);
                    s2 = s2.wrapping_add(p2 + 1);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        lua_pop(lstate, 2);
        lua_pushnumber(lstate, f64::from(i32::from(ret > 0) - i32::from(ret < 0)));
        1
    }
}

unsafe extern "C-unwind" fn nlua_iconv(lstate: *mut lua_State) -> i32 {
    let narg = unsafe { lua_gettop(lstate) };
    if narg < 3 {
        return unsafe { luaL_error(lstate, c"Expected at least 3 arguments".as_ptr()) };
    }

    for i in 1..=3 {
        if unsafe { lua_type(lstate, i) } != LUA_TSTRING {
            return luaL_argerror(lstate, i, c"expected string");
        }
    }

    let mut str_len: usize = 0;
    let str_ = unsafe { lua_tolstring(lstate, 1, &raw mut str_len) };

    let from = enc_canonize(enc_skip(
        unsafe { lua_tolstring(lstate, 2, ptr::null_mut()) }.cast_mut(),
    ));
    let to = enc_canonize(enc_skip(
        unsafe { lua_tolstring(lstate, 3, ptr::null_mut()) }.cast_mut(),
    ));

    let mut vimconv = Vimconv {
        vc_type: CONV_NONE,
        vc_factor: 0,
        vc_fd: None,
        vc_fail: false,
    };
    let _ = convert_setup_ext(
        &mut vimconv,
        from.as_ptr().cast_mut(),
        false,
        to.as_ptr().cast_mut(),
        false,
    );

    let ret = string_convert(&vimconv, str_.cast_mut(), Some(&mut str_len));

    match ret {
        None => unsafe { lua_pushnil(lstate) },
        Some(bytes) => unsafe { lua_pushlstring(lstate, bytes.as_ptr().cast(), str_len) },
    }

    1
}

unsafe extern "C-unwind" fn nlua_foldupdate(lstate: *mut lua_State) -> i32 {
    unsafe {
        let window = luaL_checkinteger(lstate, 1) as ObjHandle;
        let win = handle_get_window(window);
        if win.is_null() {
            return luaL_error(lstate, c"invalid window".as_ptr());
        }
        let top = (luaL_checkinteger(lstate, 2) as Linenr) + 1;
        if top < 1 {
            return luaL_error(lstate, c"invalid top".as_ptr());
        }
        let bot = luaL_checkinteger(lstate, 3) as Linenr;
        if top > bot {
            return luaL_error(lstate, c"invalid bot".as_ptr());
        }

        fold_update(&mut *(win), top, bot);
        0
    }
}

unsafe extern "C-unwind" fn nlua_with(lstate: *mut lua_State) -> i32 {
    unsafe {
        let mut flags: i32 = 0;
        let mut buf: *mut FileBuf = ptr::null_mut();
        let mut win: *mut WinInfoInfo = ptr::null_mut();
        let mut keepcwd = false;
        let mut log_level: i32 = -1;

        if !lua_istable(lstate, 1) {
            return luaL_argerror(lstate, 1, c"table expected");
        }
        lua_pushnil(lstate);
        while lua_next(lstate, 1) != 0 {
            if lua_type(lstate, -2) == LUA_TSTRING {
                let k = crate::memory::cstr_as_bytes(lua_tostring(lstate, -2));
                let v = lua_toboolean(lstate, -1) != 0;
                match k {
                    b"buf" => buf = handle_get_buffer(luaL_checkinteger(lstate, -1) as i32),
                    b"win" => win = handle_get_window(luaL_checkinteger(lstate, -1) as i32),
                    b"keepcwd" => keepcwd = v,
                    b"log_level" => log_level = luaL_checkinteger(lstate, -1) as i32,
                    b"sandbox" if v => flags |= CMOD_SANDBOX,
                    b"silent" if v => flags |= CMOD_SILENT,
                    b"emsg_silent" if v => flags |= CMOD_ERRSILENT,
                    b"unsilent" if v => flags |= CMOD_UNSILENT,
                    b"noautocmd" if v => flags |= CMOD_NOAUTOCMD,
                    b"hide" if v => flags |= CMOD_HIDE,
                    b"keepalt" if v => flags |= CMOD_KEEPALT,
                    b"keepmarks" if v => flags |= CMOD_KEEPMARKS,
                    b"keepjumps" if v => flags |= CMOD_KEEPJUMPS,
                    b"lockmarks" if v => flags |= CMOD_LOCKMARKS,
                    b"keeppatterns" if v => flags |= CMOD_KEEPPATTERNS,
                    _ => {}
                }
            }
            lua_pop(lstate, 1);
        }

        let mut status: i32 = 0;
        let mut rets: i32 = 0;

        if (flags & CMOD_ERRSILENT) != 0 {
            flags |= CMOD_SILENT;
        }

        let save_min_log_level = log_st().g_min_log_level;
        if log_level >= 0 {
            log_st().g_min_log_level = log_level;
        }
        let save_cmdmod = ptr::replace(&raw mut g_st().cmdmod, crate::ex_docmd::Cmdmod::new());
        g_st().cmdmod.cmod_flags = flags;
        apply_cmdmod(&mut g_st().cmdmod);

        let mut err = Error::new();
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        {
            let mut cs: CtxSwitch = CtxSwitch::new();
            let mut switched = true;

            if !win.is_null() {
                buf = ptr::null_mut();
            }

            if !win.is_null() || !buf.is_null() || keepcwd {
                let dirs = if keepcwd {
                    K_CTX_KEEP_DIRS
                } else {
                    K_CTX_KEEP_CWD
                };
                let tab = if !win.is_null() {
                    win_find_tabpage(win)
                } else {
                    ptr::null_mut()
                };
                switched = ctx_switch(
                    &mut cs,
                    win,
                    tab,
                    buf,
                    K_CTX_NO_DISPLAY | dirs | if !win.is_null() { K_CTX_VALIDATE } else { 0 },
                );
            }

            if switched {
                let s = lua_gettop(lstate);
                lua_pushvalue(lstate, 2);
                status = lua_pcall(lstate, 0, LUA_MULTRET, 0);
                rets = lua_gettop(lstate) - s;
            }

            ctx_restore(&mut cs);
        }
        try_leave(&tstate, &mut err);

        undo_cmdmod(&mut g_st().cmdmod);
        g_st().cmdmod = save_cmdmod;
        if log_level >= 0 {
            log_st().g_min_log_level = save_min_log_level;
        }

        if status != 0 {
            return lua_error(lstate);
        } else if err.err_type != ErrorType::ErrorTypeNone {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg_cstr());
            lua_concat(lstate, 2);
            api_clear_error(&mut err);
            return lua_error(lstate);
        }

        rets
    }
}

unsafe extern "C-unwind" fn nlua_keyparse(lstate: *mut lua_State) -> i32 {
    if unsafe { lua_isstring(lstate, 1) } == 0 {
        return luaL_argerror(lstate, 1, c"string expected");
    }

    let escaped = vim_strsave_escape_ks(lua_tostring(lstate, 1));
    let mut p = escaped.as_ptr();

    lua_newtable(lstate);

    let mut data = KeyChord::new();

    while !crate::memory::cstr_is_empty(p) {
        let keys = str2special(&raw mut p, true, K_TRUE, Some(&mut data));

        unsafe { lua_createtable(lstate, 0, 3) };

        lua_newtable(lstate);
        let mut i: usize = 0;
        while MOD_MASK_TABLE[i].name != b'A' {
            if (data.mods & i32::from(MOD_MASK_TABLE[i].mod_mask))
                == i32::from(MOD_MASK_TABLE[i].mod_flag)
            {
                unsafe { lua_pushlstring(lstate, (&raw const MOD_MASK_TABLE[i].name).cast(), 1) };
                unsafe { lua_rawseti(lstate, -2, (lua_objlen(lstate, -2) as i32) + 1) };
            }
            i += 1;
        }
        lua_setfield(lstate, -2, c"mod");

        unsafe { lua_pushlstring(lstate, data.key.data, data.key.size) };
        lua_setfield(lstate, -2, c"key");

        lua_pushstring(lstate, keys);
        lua_setfield(lstate, -2, c"keys");

        if data.key_alt.size != 0 {
            unsafe { lua_pushlstring(lstate, data.key_alt.data, data.key_alt.size) };
            lua_setfield(lstate, -2, c"key_alt");
        }

        unsafe { lua_rawseti(lstate, -2, (lua_objlen(lstate, -2) as i32) + 1) };
    }

    1
}

fn nlua_state_add_internal(lstate: *mut lua_State) {
    lua_pushcfunction(lstate, Some(nlua_getvar));
    lua_setfield(lstate, -2, c"_getvar");

    lua_pushcfunction(lstate, Some(nlua_setvar));
    lua_setfield(lstate, -2, c"_setvar");

    lua_pushcfunction(lstate, Some(nlua_foldupdate));
    lua_setfield(lstate, -2, c"_foldupdate");

    lua_pushcfunction(lstate, Some(nlua_with));
    lua_setfield(lstate, -2, c"_with_c");

    lua_getfield(lstate, -1, c"_core");
    lua_pushcfunction(lstate, Some(nlua_keyparse));
    lua_setfield(lstate, -2, c"keyparse");
    lua_pop(lstate, 1);
}

pub fn nlua_state_add_stdlib(lstate: *mut lua_State, is_thread: bool) {
    if !is_thread {
        lua_pushcfunction(lstate, Some(nlua_stricmp));
        lua_setfield(lstate, -2, c"stricmp");

        lua_pushcfunction(lstate, Some(nlua_str_utfindex));
        lua_setfield(lstate, -2, c"_str_utfindex");

        lua_pushcfunction(lstate, Some(nlua_str_byteindex));
        lua_setfield(lstate, -2, c"_str_byteindex");

        lua_pushcfunction(lstate, Some(nlua_str_utf_pos));
        lua_setfield(lstate, -2, c"str_utf_pos");

        lua_pushcfunction(lstate, Some(nlua_str_utf_start));
        lua_setfield(lstate, -2, c"str_utf_start");

        lua_pushcfunction(lstate, Some(nlua_str_utf_end));
        lua_setfield(lstate, -2, c"str_utf_end");

        lua_pushcfunction(lstate, Some(nlua_regex));
        lua_setfield(lstate, -2, c"regex");
        let _ = luaL_newmetatable(lstate, c"nvim_regex");
        luaL_register(lstate, ptr::null(), (&raw const REGEX_META).cast());

        unsafe { lua_pushvalue(lstate, -1) };
        lua_setfield(lstate, -2, c"__index");
        lua_pop(lstate, 1);

        let _ = unsafe { luaopen_spell(lstate) };
        lua_setfield(lstate, -2, c"spell");

        lua_pushcfunction(lstate, Some(nlua_iconv));
        lua_setfield(lstate, -2, c"iconv");

        let _ = unsafe { luaopen_base64(lstate) };
        lua_setfield(lstate, -2, c"base64");

        nlua_state_add_internal(lstate);
    }

    let _ = luaopen_mpack(lstate);
    unsafe { lua_pushvalue(lstate, -1) };
    lua_setfield(lstate, -3, c"mpack");

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    unsafe { lua_pushvalue(lstate, -3) };
    lua_setfield(lstate, -2, c"mpack");
    lua_pop(lstate, 3);

    let _ = luaopen_lpeg(lstate);
    unsafe { lua_pushvalue(lstate, -1) };
    lua_setfield(lstate, -4, c"lpeg");

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    unsafe { lua_pushvalue(lstate, -3) };
    lua_setfield(lstate, -2, c"lpeg");
    lua_pop(lstate, 4);

    lua_pushcfunction(lstate, Some(nlua_xdl_diff));
    lua_setfield(lstate, -2, c"diff");

    let _ = unsafe { lua_cjson_new(lstate) };
    lua_setfield(lstate, -2, c"json");

    #[cfg(feature = "vendor-bit")]
    unsafe {
        let top = lua_gettop(lstate);
        let _ = crate::bit::nvim_luaopen_bit(lstate);
        crate::lua::converter::lua_settop(lstate, top);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_meta_table_has_expected_entries() {
        assert_eq!(REGEX_META.len(), 5);
        assert!(REGEX_META[4].name.is_null());
        assert!(REGEX_META[4].func.is_none());
    }
}
