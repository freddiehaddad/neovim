//! Lua bindings for MessagePack (`src/mpack/lmpack.c`).

use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;

use crate::lua::converter::{
    LUA_NOREF, LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE,
    LUA_TUSERDATA, LUAL_BUFFERSIZE, lua_Integer, lua_Number, lua_State, lua_call, lua_createtable,
    lua_getfield, lua_getmetatable, lua_gettable, lua_gettop, lua_insert, lua_isboolean,
    lua_isfunction, lua_isnil, lua_isnoneornil, lua_isnumber, lua_isstring, lua_istable,
    lua_isuserdata, lua_newtable, lua_newuserdata, lua_next, lua_objlen, lua_pop, lua_pushboolean,
    lua_pushcfunction, lua_pushfstring, lua_pushinteger, lua_pushlstring, lua_pushnil,
    lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawequal, lua_rawgeti, lua_remove,
    lua_replace, lua_setfield, lua_setmetatable, lua_settable, lua_toboolean, lua_tolstring,
    lua_tonumber, lua_topointer, lua_type, luaL_Buffer, luaL_Reg, luaL_argcheck, luaL_buffinit,
    luaL_checklstring, luaL_checknumber, luaL_checkudata, luaL_error, luaL_getmetatable,
    luaL_newmetatable, luaL_prepbuffer, luaL_pushresult, luaL_ref, luaL_register, luaL_unref,
};
use crate::mpack::conv::{
    mpack_pack_array, mpack_pack_bin, mpack_pack_boolean, mpack_pack_chunk, mpack_pack_ext,
    mpack_pack_map, mpack_pack_nil, mpack_pack_number, mpack_pack_str, mpack_unpack_boolean,
    mpack_unpack_number,
};
use crate::mpack::mpack_core::{
    MPACK_EOF, MPACK_ERROR, MPACK_OK, MpackSintmax, MpackTokenArray, MpackTokenBin,
    MpackTokenBoolean, MpackTokenChunk, MpackTokenExt, MpackTokenFloat, MpackTokenMap,
    MpackTokenNil, MpackTokenSint, MpackTokenStr, MpackTokenUint, MpackUint32,
};
use crate::mpack::object::{
    MPACK_NOMEM, MpackData, MpackNode, MpackParser, mpack_parent_node, mpack_parse,
    mpack_parser_grow, mpack_parser_init, mpack_unparse,
};
use crate::mpack::rpc::{
    MPACK_RPC_NOTIFICATION, MPACK_RPC_REQUEST, MPACK_RPC_RESPONSE, MpackRpcMessage,
    MpackRpcSession, mpack_rpc_notify, mpack_rpc_receive, mpack_rpc_reply, mpack_rpc_request,
    mpack_rpc_session_grow,
};
use crate::strings::vim_snprintf;

const LUA_REFNIL: i32 = -1;

const UNPACKER_META_NAME: &CStr = c"mpack.Unpacker";
const PACKER_META_NAME: &CStr = c"mpack.Packer";
const SESSION_META_NAME: &CStr = c"mpack.Session";
const NIL_NAME: &CStr = c"mpack.NIL";
const EMPTY_DICT_NAME: &CStr = c"mpack.empty_dict";

#[repr(C)]
pub struct LmpackUnpacker {
    lstate: *mut lua_State,
    parser: MpackParser,
    reg: i32,
    ext: i32,
    unpacking: i32,
    mtdict: i32,
    string_buffer: Vec<u8>,
}

#[repr(C)]
pub struct Packer {
    lstate: *mut lua_State,
    parser: MpackParser,
    reg: i32,
    ext: i32,
    root: i32,
    packing: i32,
    mtdict: i32,
    is_bin: i32,
    is_bin_fn: i32,
}

#[repr(C)]
struct UnpackedState {
    r#type: i32,
    msg: MpackRpcMessage,
    method_or_error: i32,
    args_or_result: i32,
}

#[repr(C)]
struct Session {
    lstate: *mut lua_State,
    reg: i32,
    session: MpackRpcSession,
    unpacked: UnpackedState,
    unpacker: i32,
}

#[inline]
#[allow(non_snake_case)]
fn luaL_addsize(b: &mut luaL_Buffer, n: usize) {
    b.p = b.p.wrapping_add(n);
}

fn lmpack_ref(l: *mut lua_State, reg: i32) -> i32 {
    unsafe {
        lua_rawgeti(l, LUA_REGISTRYINDEX, reg);
        lua_pushvalue(l, -2);
        let rv = luaL_ref(l, -2);
        lua_pop(l, 2);
        rv
    }
}

fn lmpack_unref(l: *mut lua_State, reg: i32, r#ref: i32) {
    unsafe {
        lua_rawgeti(l, LUA_REGISTRYINDEX, reg);
        luaL_unref(l, -1, r#ref);
        lua_pop(l, 1);
    }
}

fn lmpack_geti(l: *mut lua_State, reg: i32, r#ref: i32) {
    unsafe {
        lua_rawgeti(l, LUA_REGISTRYINDEX, reg);
        lua_rawgeti(l, -1, r#ref);
        lua_replace(l, -2);
    }
}

fn lmpack_shallow_copy(l: *mut lua_State) {
    unsafe {
        lua_newtable(l);
        lua_pushnil(l);
        while lua_next(l, -3) != 0 {
            lua_pushvalue(l, -2);
            lua_insert(l, -2);
            lua_settable(l, -4);
        }
        lua_remove(l, -2);
    }
}

fn lmpack_check_unpacker(l: *mut lua_State, index: i32) -> &'static mut LmpackUnpacker {
    unsafe { &mut *luaL_checkudata(l, index, UNPACKER_META_NAME).cast::<LmpackUnpacker>() }
}

fn lmpack_check_packer(l: *mut lua_State, index: i32) -> &'static mut Packer {
    unsafe { &mut *luaL_checkudata(l, index, PACKER_META_NAME).cast::<Packer>() }
}

fn lmpack_check_session(l: *mut lua_State, index: i32) -> &'static mut Session {
    unsafe { &mut *luaL_checkudata(l, index, SESSION_META_NAME).cast::<Session>() }
}

fn lmpack_isnil(l: *mut lua_State, index: i32) -> bool {
    unsafe {
        if lua_isuserdata(l, index) == 0 {
            return false;
        }
        lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
        let rv = lua_rawequal(l, -1, -2) != 0;
        lua_pop(l, 1);
        rv
    }
}

fn lmpack_isunpacker(l: *mut lua_State, index: i32) -> bool {
    unsafe {
        if lua_isuserdata(l, index) == 0 || lua_getmetatable(l, index) == 0 {
            return false;
        }
        luaL_getmetatable(l, UNPACKER_META_NAME);
        let rv = lua_rawequal(l, -1, -2) != 0;
        lua_pop(l, 2);
        rv
    }
}

fn lmpack_pushnil(l: *mut lua_State) {
    lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
}

fn lmpack_objlen(l: *mut lua_State, is_array: Option<&mut i32>) -> MpackUint32 {
    unsafe {
        let mut len: usize;
        if lua_type(l, -1) != LUA_TTABLE {
            len = lua_objlen(l, -1);
        } else {
            len = 0;
            let mut max: usize = 0;
            let mut isarr = true;
            lua_pushnil(l);

            while lua_next(l, -2) != 0 {
                lua_pop(l, 1);
                let mut n: lua_Number = 0.0;
                isarr = isarr
                    && lua_type(l, -1) == LUA_TNUMBER
                    && {
                        n = lua_tonumber(l, -1);
                        n > 0.0
                    }
                    && (n as usize as lua_Number) == n;
                if isarr && (n as usize) > max {
                    max = n as usize;
                }
                len += 1;
            }

            if len > 0
                && let Some(is_array) = is_array
            {
                *is_array = i32::from(isarr && max == len);
            }
        }

        if len > (MpackUint32::MAX as usize) {
            len = MpackUint32::MAX as usize;
        }
        len as MpackUint32
    }
}

unsafe extern "C-unwind" fn lmpack_unpacker_new(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) > 1 {
            return luaL_error(l, c"expecting at most 1 table argument".as_ptr());
        }

        let rv = lua_newuserdata(l, size_of::<LmpackUnpacker>()).cast::<LmpackUnpacker>();
        let rv = &mut *rv;
        ptr::write(
            rv,
            LmpackUnpacker {
                lstate: l,
                parser: MpackParser::new(0),
                reg: LUA_NOREF,
                ext: LUA_NOREF,
                unpacking: 0,
                mtdict: 0,
                string_buffer: Vec::new(),
            },
        );
        mpack_parser_init(&mut rv.parser, 0);
        rv.parser.data = MpackData::LmpackUnpacker(rv);
        luaL_getmetatable(l, UNPACKER_META_NAME);
        lua_setmetatable(l, -2);

        lua_newtable(l);
        rv.reg = luaL_ref(l, LUA_REGISTRYINDEX);

        lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
        rv.mtdict = lmpack_ref(l, rv.reg);

        if lua_istable(l, 1) {
            lua_getfield(l, 1, c"ext");
            if !lua_isnil(l, -1) {
                if !lua_istable(l, -1) {
                    return luaL_error(l, c"\"ext\" option must be a table".as_ptr());
                }
                lmpack_shallow_copy(l);
            }
            rv.ext = lmpack_ref(l, rv.reg);
        }

        1
    }
}

unsafe extern "C-unwind" fn lmpack_unpacker_delete(l: *mut lua_State) -> i32 {
    unsafe {
        let unpacker = lmpack_check_unpacker(l, 1);
        if unpacker.ext != LUA_NOREF {
            lmpack_unref(l, unpacker.reg, unpacker.ext);
        }
        luaL_unref(l, LUA_REGISTRYINDEX, unpacker.reg);
        ptr::drop_in_place(unpacker);
        0
    }
}

fn lmpack_parse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    unsafe {
        let unpacker = parser.data.lmpack_unpacker();
        let l = unpacker.lstate;

        match node.tok.r#type {
            MpackTokenNil => {
                lmpack_pushnil(l);
            }
            MpackTokenBoolean => {
                lua_pushboolean(l, i32::from(mpack_unpack_boolean(node.tok)));
            }
            MpackTokenUint | MpackTokenSint | MpackTokenFloat => {
                lua_pushnumber(l, mpack_unpack_number(node.tok));
            }
            MpackTokenChunk => {
                let parent = mpack_parent_node(&raw mut *node);
                ptr::copy_nonoverlapping(
                    node.tok.data.chunk_ptr(),
                    unpacker
                        .string_buffer
                        .as_mut_ptr()
                        .wrapping_add((*parent).pos),
                    node.tok.length as usize,
                );
            }
            MpackTokenBin | MpackTokenStr | MpackTokenExt => {
                unpacker.string_buffer.clear();
                unpacker.string_buffer.resize(node.tok.length as usize, 0);
            }
            MpackTokenArray | MpackTokenMap => {
                lua_newtable(l);
                node.data[0] = crate::mpack::object::MpackData::Int(MpackSintmax::from(
                    lmpack_ref(l, unpacker.reg),
                ));
            }
            _ => {}
        }
    }
}

fn lmpack_parse_exit(parser: &mut MpackParser, node: &mut MpackNode) {
    unsafe {
        let unpacker = parser.data.lmpack_unpacker();
        let l = unpacker.lstate;
        let parent = mpack_parent_node(&raw mut *node);

        match node.tok.r#type {
            MpackTokenBin | MpackTokenStr | MpackTokenExt => {
                lua_pushlstring(
                    l,
                    unpacker.string_buffer.as_ptr().cast(),
                    node.tok.length as usize,
                );
                unpacker.string_buffer.clear();
                if node.tok.r#type == MpackTokenExt && unpacker.ext != LUA_NOREF {
                    lmpack_geti(l, unpacker.reg, unpacker.ext);
                    lua_rawgeti(l, -1, node.tok.data.ext_type());
                    if lua_isfunction(l, -1) {
                        lua_pushinteger(l, node.tok.data.ext_type() as lua_Integer);
                        lua_pushvalue(l, -4);
                        lua_call(l, 2, 1);
                        lua_replace(l, -3);
                    } else {
                        lua_pop(l, 1);
                    }
                    lua_pop(l, 1);
                }
            }
            MpackTokenArray | MpackTokenMap => {
                let ref_id = node.data[0].i() as i32;
                lmpack_geti(l, unpacker.reg, ref_id);
                lmpack_unref(l, unpacker.reg, ref_id);
                if node.key_visited == 0 && node.tok.r#type == MpackTokenMap {
                    lmpack_geti(l, unpacker.reg, unpacker.mtdict);
                    lua_setmetatable(l, -2);
                }
            }
            _ => {}
        }

        if !parent.is_null() && (*parent).tok.r#type < MpackTokenBin {
            lmpack_geti(l, unpacker.reg, (*parent).data[0].i() as i32);

            if (*parent).tok.r#type == MpackTokenArray {
                lua_pushnumber(l, (*parent).pos as lua_Number);
                lua_pushvalue(l, -3);
                lua_settable(l, -3);
            } else {
                debug_assert_eq!((*parent).tok.r#type, MpackTokenMap);
                if (*parent).key_visited != 0 {
                    lua_pushvalue(l, -2);
                    (*parent).data[1] = crate::mpack::object::MpackData::Int(MpackSintmax::from(
                        lmpack_ref(l, unpacker.reg),
                    ));
                } else {
                    let key_ref = (*parent).data[1].i() as i32;
                    lmpack_geti(l, unpacker.reg, key_ref);
                    lmpack_unref(l, unpacker.reg, key_ref);
                    lua_pushvalue(l, -3);
                    lua_settable(l, -3);
                }
            }
            lua_pop(l, 2);
        }
    }
}

fn lmpack_unpacker_unpack_str(
    l: *mut lua_State,
    unpacker: &mut LmpackUnpacker,
    str_ptr: &mut *const u8,
    len: &mut usize,
) -> i32 {
    unsafe {
        if unpacker.unpacking != 0 {
            return luaL_error(
                l,
                c"Unpacker instance already working. Use another Unpacker or mpack.decode() if you need to decode from the ext handler".as_ptr(),
            );
        }

        let mut rv: i32;
        loop {
            unpacker.unpacking = 1;
            rv = mpack_parse(
                &mut unpacker.parser,
                str_ptr,
                len,
                Some(lmpack_parse_enter),
                Some(lmpack_parse_exit),
            );
            unpacker.unpacking = 0;

            if rv == MPACK_NOMEM {
                mpack_parser_grow(&mut unpacker.parser);
            } else {
                break;
            }
        }

        if rv == MPACK_ERROR {
            return luaL_error(l, c"invalid msgpack string".as_ptr());
        }

        rv
    }
}

unsafe extern "C-unwind" fn lmpack_unpacker_unpack(l: *mut lua_State) -> i32 {
    unsafe {
        let argc = lua_gettop(l);
        if !(2..=3).contains(&argc) {
            return luaL_error(l, c"expecting between 2 and 3 arguments".as_ptr());
        }

        let unpacker = lmpack_check_unpacker(l, 1);
        unpacker.lstate = l;

        let mut len: usize = 0;
        let str_init = luaL_checklstring(l, 2, &mut len).cast::<u8>();
        let mut str_ptr = str_init;
        let startpos = if argc == 3 {
            luaL_checknumber(l, 3)
        } else {
            1.0
        };

        luaL_argcheck(
            l,
            startpos > 0.0,
            3,
            c"start position must be greater than zero",
        );
        luaL_argcheck(
            l,
            (startpos as usize as lua_Number) == startpos,
            3,
            c"start position must be an integer",
        );
        luaL_argcheck(
            l,
            (startpos as usize) <= len,
            3,
            c"start position must be less than or equal to the input string length",
        );

        let offset = (startpos as usize) - 1;
        str_ptr = str_ptr.wrapping_add(offset);
        len -= offset;
        let result = lmpack_unpacker_unpack_str(l, &mut *unpacker, &mut str_ptr, &mut len);

        if result == MPACK_EOF {
            lua_pushnil(l);
        }

        lua_pushinteger(l, (str_ptr.offset_from(str_init) + 1) as lua_Integer);
        debug_assert_eq!(lua_gettop(l), argc + 2);
        2
    }
}

unsafe extern "C-unwind" fn lmpack_packer_new(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) > 1 {
            return luaL_error(l, c"expecting at most 1 table argument".as_ptr());
        }

        let rv = &mut *lua_newuserdata(l, size_of::<Packer>()).cast::<Packer>();
        ptr::write(
            rv,
            Packer {
                lstate: l,
                parser: MpackParser::new(0),
                reg: LUA_NOREF,
                ext: LUA_NOREF,
                root: LUA_NOREF,
                packing: 0,
                mtdict: 0,
                is_bin: 0,
                is_bin_fn: LUA_NOREF,
            },
        );
        mpack_parser_init(&mut rv.parser, 0);
        rv.parser.data = MpackData::LmpackPacker(rv);
        luaL_getmetatable(l, PACKER_META_NAME);
        lua_setmetatable(l, -2);

        lua_newtable(l);
        rv.reg = luaL_ref(l, LUA_REGISTRYINDEX);
        rv.ext = LUA_NOREF;

        lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
        rv.mtdict = lmpack_ref(l, rv.reg);

        if lua_istable(l, 1) {
            lua_getfield(l, 1, c"ext");
            if !lua_isnil(l, -1) {
                if !lua_istable(l, -1) {
                    return luaL_error(l, c"\"ext\" option must be a table".as_ptr());
                }
                lmpack_shallow_copy(l);
            }
            rv.ext = lmpack_ref(l, rv.reg);

            lua_getfield(l, 1, c"is_bin");
            if !lua_isnil(l, -1) {
                if !lua_isboolean(l, -1) && !lua_isfunction(l, -1) {
                    return luaL_error(
                        l,
                        c"\"is_bin\" option must be a boolean or function".as_ptr(),
                    );
                }
                rv.is_bin = lua_toboolean(l, -1);
                if lua_isfunction(l, -1) {
                    rv.is_bin_fn = lmpack_ref(l, rv.reg);
                } else {
                    lua_pop(l, 1);
                }
            } else {
                lua_pop(l, 1);
            }
        }

        1
    }
}

unsafe extern "C-unwind" fn lmpack_packer_delete(l: *mut lua_State) -> i32 {
    unsafe {
        let packer = lmpack_check_packer(l, 1);
        if packer.ext != LUA_NOREF {
            lmpack_unref(l, packer.reg, packer.ext);
        }
        luaL_unref(l, LUA_REGISTRYINDEX, packer.reg);
        ptr::drop_in_place(packer);
        0
    }
}

fn lmpack_unparse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    unsafe {
        let packer = parser.data.lmpack_packer();
        let l = packer.lstate;
        let parent = mpack_parent_node(&raw mut *node);

        if !parent.is_null() {
            let parent = &mut *parent;
            lmpack_geti(l, packer.reg, parent.data[0].i() as i32);

            if parent.tok.r#type > MpackTokenMap {
                let str_ptr = lua_tolstring(l, -1, ptr::null_mut()).cast::<u8>();
                node.tok = mpack_pack_chunk(str_ptr, parent.tok.length);
                lua_pop(l, 1);
                return;
            }

            if parent.tok.r#type == MpackTokenArray {
                lua_pushnumber(l, (parent.pos + 1) as lua_Number);
                lua_gettable(l, -2);
            } else if parent.tok.r#type == MpackTokenMap {
                lmpack_geti(l, packer.reg, parent.data[1].i() as i32);
                let result = lua_next(l, -2);
                debug_assert_ne!(result, 0);
                if parent.key_visited != 0 {
                    lmpack_unref(l, packer.reg, parent.data[1].i() as i32);
                    lua_pushvalue(l, -2);
                    parent.data[1] = crate::mpack::object::MpackData::Int(MpackSintmax::from(
                        lmpack_ref(l, packer.reg),
                    ));
                    lua_replace(l, -2);
                } else {
                    lua_pop(l, 1);
                }
            }
            lua_remove(l, -2);
        } else {
            lmpack_geti(l, packer.reg, packer.root);
        }

        let r#type = lua_type(l, -1);

        'done: {
            match r#type {
                LUA_TBOOLEAN => {
                    node.tok = mpack_pack_boolean(lua_toboolean(l, -1) as u32);
                }
                LUA_TNUMBER => {
                    node.tok = mpack_pack_number(lua_tonumber(l, -1));
                }
                LUA_TSTRING => {
                    let mut is_bin = packer.is_bin;
                    if is_bin != 0 && packer.is_bin_fn != LUA_NOREF {
                        lmpack_geti(l, packer.reg, packer.is_bin_fn);
                        lua_pushvalue(l, -2);
                        lua_call(l, 1, 1);
                        is_bin = lua_toboolean(l, -1);
                        lua_pop(l, 1);
                    }
                    if is_bin != 0 {
                        node.tok = mpack_pack_bin(lmpack_objlen(l, None));
                    } else {
                        node.tok = mpack_pack_str(lmpack_objlen(l, None));
                    }
                }
                LUA_TTABLE => {
                    let has_meta = lua_getmetatable(l, -1) != 0;
                    let mut has_mtdict = false;
                    if has_meta && packer.mtdict != LUA_NOREF {
                        lmpack_geti(l, packer.reg, packer.mtdict);
                        has_mtdict = lua_rawequal(l, -1, -2) != 0;
                        lua_pop(l, 1);
                    }
                    if packer.ext != LUA_NOREF && has_meta && !has_mtdict {
                        lmpack_geti(l, packer.reg, packer.ext);
                        lua_pushvalue(l, -2);
                        lua_gettable(l, -2);
                        if lua_isfunction(l, -1) {
                            lua_pushvalue(l, -4);
                            lua_call(l, 1, 2);
                            let mut ext: lua_Number = -1.0;
                            if lua_isnumber(l, -2) == 0
                                || {
                                    ext = lua_tonumber(l, -2);
                                    !(0.0..=127.0).contains(&ext)
                                }
                                || (ext as i32 as lua_Number) != ext
                            {
                                luaL_error(
                                    l,
                                    c"the first result from ext packer must be an integer between 0 and 127".as_ptr(),
                                );
                            }
                            if lua_isstring(l, -1) == 0 {
                                luaL_error(
                                    l,
                                    c"the second result from ext packer must be a string".as_ptr(),
                                );
                            }
                            node.tok = mpack_pack_ext(ext as i32, lmpack_objlen(l, None));
                            lua_replace(l, -5);
                            lua_pop(l, 3);
                            break 'done;
                        } else {
                            lua_pop(l, 1);
                        }
                    }

                    if has_meta {
                        lua_pop(l, 1);
                    }

                    let mut n = mpack_parent_node(&raw mut *node);
                    while !n.is_null() {
                        lmpack_geti(l, packer.reg, (*n).data[0].i() as i32);
                        if lua_rawequal(l, -1, -2) != 0 {
                            node.tok = mpack_pack_nil();
                            lua_pop(l, 2);
                            lmpack_pushnil(l);
                            break 'done;
                        }
                        lua_pop(l, 1);
                        n = mpack_parent_node(n);
                    }

                    let mut is_array = i32::from(!has_mtdict);
                    let len = lmpack_objlen(l, Some(&mut is_array));
                    if is_array != 0 {
                        node.tok = mpack_pack_array(len);
                    } else {
                        node.tok = mpack_pack_map(len);
                        node.data[1] =
                            crate::mpack::object::MpackData::Int(MpackSintmax::from(LUA_REFNIL));
                    }
                }
                LUA_TUSERDATA if lmpack_isnil(l, -1) => {
                    node.tok = mpack_pack_nil();
                }
                _ => {
                    let mut errmsg = [0u8; 50];
                    vim_snprintf!(
                        errmsg.as_mut_ptr().cast(),
                        50,
                        c"can't serialize object of type %d",
                        r#type,
                    );
                    luaL_error(l, errmsg.as_ptr().cast());
                }
            }
        }

        node.data[0] =
            crate::mpack::object::MpackData::Int(MpackSintmax::from(lmpack_ref(l, packer.reg)));
    }
}

fn lmpack_unparse_exit(parser: &mut MpackParser, node: &mut MpackNode) {
    let packer = parser.data.lmpack_packer();
    let l = packer.lstate;
    if node.tok.r#type != MpackTokenChunk {
        lmpack_unref(l, packer.reg, node.data[0].i() as i32);
        if node.tok.r#type == MpackTokenMap {
            lmpack_unref(l, packer.reg, node.data[1].i() as i32);
        }
    }
}

unsafe extern "C-unwind" fn lmpack_packer_pack(l: *mut lua_State) -> i32 {
    unsafe {
        let argc = lua_gettop(l);
        if argc != 2 {
            return luaL_error(l, c"expecting exactly 2 arguments".as_ptr());
        }

        let packer = lmpack_check_packer(l, 1);
        packer.lstate = l;
        packer.root = lmpack_ref(l, packer.reg);
        let mut buffer = luaL_Buffer::new();
        luaL_buffinit(l, &mut buffer);
        let mut b = luaL_prepbuffer(&mut buffer).cast::<u8>();
        let mut bl = LUAL_BUFFERSIZE;

        if packer.packing != 0 {
            return luaL_error(
                l,
                c"Packer instance already working. Use another Packer or mpack.encode() if you need to encode from the ext handler".as_ptr(),
            );
        }

        loop {
            let bl_init = bl;
            packer.packing = 1;
            let result = mpack_unparse(
                &mut packer.parser,
                &mut b,
                &mut bl,
                Some(lmpack_unparse_enter),
                Some(lmpack_unparse_exit),
            );
            packer.packing = 0;

            if result == MPACK_NOMEM {
                mpack_parser_grow(&mut packer.parser);
            }

            luaL_addsize(&mut buffer, bl_init - bl);

            if bl == 0 {
                b = luaL_prepbuffer(&mut buffer).cast::<u8>();
                bl = LUAL_BUFFERSIZE;
            }

            if result != MPACK_EOF && result != MPACK_NOMEM {
                break;
            }
        }

        lmpack_unref(l, packer.reg, packer.root);
        luaL_pushresult(&mut buffer);
        debug_assert_eq!(lua_gettop(l), argc);
        1
    }
}

unsafe extern "C-unwind" fn lmpack_session_new(l: *mut lua_State) -> i32 {
    unsafe {
        let rv = lua_newuserdata(l, size_of::<Session>()).cast::<Session>();
        let rv = &mut *rv;
        ptr::write(
            rv,
            Session {
                lstate: l,
                reg: LUA_NOREF,
                session: MpackRpcSession::new(0),
                unpacked: UnpackedState {
                    r#type: MPACK_EOF,
                    msg: MpackRpcMessage {
                        id: 0,
                        data: MpackData::new(),
                    },
                    method_or_error: LUA_NOREF,
                    args_or_result: LUA_NOREF,
                },
                unpacker: LUA_REFNIL,
            },
        );
        luaL_getmetatable(l, SESSION_META_NAME);
        lua_setmetatable(l, -2);

        lua_newtable(l);
        rv.reg = luaL_ref(l, LUA_REGISTRYINDEX);

        if lua_istable(l, 1) {
            lua_getfield(l, 1, c"unpack");
            if !lmpack_isunpacker(l, -1) {
                return luaL_error(
                    l,
                    c"\"unpack\" option must be a mpack.Unpacker instance".as_ptr(),
                );
            }
            rv.unpacker = lmpack_ref(l, rv.reg);
        }

        1
    }
}

unsafe extern "C-unwind" fn lmpack_session_delete(l: *mut lua_State) -> i32 {
    unsafe {
        let session = lmpack_check_session(l, 1);
        lmpack_unref(l, session.reg, session.unpacker);
        luaL_unref(l, LUA_REGISTRYINDEX, session.reg);
        ptr::drop_in_place(session);
        0
    }
}

unsafe extern "C-unwind" fn lmpack_session_receive(l: *mut lua_State) -> i32 {
    unsafe {
        let argc = lua_gettop(l);
        if !(2..=3).contains(&argc) {
            return luaL_error(l, c"expecting between 2 and 3 arguments".as_ptr());
        }

        let mut rcount: i32 = 3;
        let session = lmpack_check_session(l, 1);
        let mut len: usize = 0;
        let str_init = luaL_checklstring(l, 2, &mut len).cast::<u8>();
        let mut str_ptr = str_init;
        let startpos = if argc == 3 {
            luaL_checknumber(l, 3)
        } else {
            1.0
        };

        luaL_argcheck(
            l,
            startpos > 0.0,
            3,
            c"start position must be greater than zero",
        );
        luaL_argcheck(
            l,
            (startpos as usize as lua_Number) == startpos,
            3,
            c"start position must be an integer",
        );
        luaL_argcheck(
            l,
            (startpos as usize) <= len,
            3,
            c"start position must be less than or equal to the input string length",
        );

        let offset = (startpos as usize) - 1;
        str_ptr = str_ptr.wrapping_add(offset);
        len -= offset;

        let mut unpacker: *mut LmpackUnpacker = ptr::null_mut();
        if session.unpacker != LUA_REFNIL {
            lmpack_geti(l, session.reg, session.unpacker);
            unpacker = lmpack_check_unpacker(l, -1);
            (*unpacker).lstate = l;
            rcount += 2;
            lua_pop(l, 1);
        }

        loop {
            if session.unpacked.r#type == MPACK_EOF {
                session.unpacked.r#type = mpack_rpc_receive(
                    &mut session.session,
                    &mut str_ptr,
                    &mut len,
                    &mut session.unpacked.msg,
                );

                if unpacker.is_null() || session.unpacked.r#type == MPACK_EOF {
                    break;
                }
            }

            let result = lmpack_unpacker_unpack_str(l, &mut *unpacker, &mut str_ptr, &mut len);
            if result == MPACK_EOF {
                break;
            }

            if session.unpacked.method_or_error == LUA_NOREF {
                session.unpacked.method_or_error = lmpack_ref(l, session.reg);
            } else {
                session.unpacked.args_or_result = lmpack_ref(l, session.reg);
                break;
            }
        }

        let done = session.unpacked.r#type != MPACK_EOF
            && (session.unpacked.args_or_result != LUA_NOREF || unpacker.is_null());

        if !done {
            lua_pushnil(l);
            lua_pushnil(l);
            if !unpacker.is_null() {
                lua_pushnil(l);
                lua_pushnil(l);
            }
        } else {
            match session.unpacked.r#type {
                MPACK_RPC_REQUEST => {
                    lua_pushstring(l, c"request");
                    lua_pushnumber(l, lua_Number::from(session.unpacked.msg.id));
                }
                MPACK_RPC_RESPONSE => {
                    lua_pushstring(l, c"response");
                    lmpack_geti(l, session.reg, session.unpacked.msg.data.i() as i32);
                }
                MPACK_RPC_NOTIFICATION => {
                    lua_pushstring(l, c"notification");
                    lua_pushnil(l);
                }
                _ => {
                    return luaL_error(l, c"invalid msgpack-rpc string".as_ptr());
                }
            }

            session.unpacked.r#type = MPACK_EOF;

            if !unpacker.is_null() {
                lmpack_geti(l, session.reg, session.unpacked.method_or_error);
                lmpack_geti(l, session.reg, session.unpacked.args_or_result);
                lmpack_unref(l, session.reg, session.unpacked.method_or_error);
                lmpack_unref(l, session.reg, session.unpacked.args_or_result);
                session.unpacked.method_or_error = LUA_NOREF;
                session.unpacked.args_or_result = LUA_NOREF;
            }
        }

        lua_pushinteger(l, (str_ptr.offset_from(str_init) + 1) as lua_Integer);
        rcount
    }
}

unsafe extern "C-unwind" fn lmpack_session_request(l: *mut lua_State) -> i32 {
    unsafe {
        if !(1..=2).contains(&lua_gettop(l)) {
            return luaL_error(l, c"expecting 1 or 2 arguments".as_ptr());
        }

        let mut buf = [0u8; 16];
        let mut b = buf.as_mut_ptr();
        let mut bl = buf.len();
        let session = lmpack_check_session(l, 1);
        let data = MpackData::Int(MpackSintmax::from(if lua_isnoneornil(l, 2) {
            LUA_NOREF
        } else {
            lmpack_ref(l, session.reg)
        }));

        let mut result: i32;
        loop {
            result = mpack_rpc_request(&mut session.session, &mut b, &mut bl, data.clone());
            if result == MPACK_NOMEM {
                mpack_rpc_session_grow(&mut session.session);
            } else {
                break;
            }
        }

        debug_assert_eq!(result, MPACK_OK);
        lua_pushlstring(l, buf.as_ptr().cast(), buf.len() - bl);
        1
    }
}

unsafe extern "C-unwind" fn lmpack_session_reply(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) != 2 {
            return luaL_error(l, c"expecting exactly 2 arguments".as_ptr());
        }

        let mut buf = [0u8; 16];
        let mut b = buf.as_mut_ptr();
        let mut bl = buf.len();
        let session = lmpack_check_session(l, 1);
        let id = lua_tonumber(l, 2);
        luaL_argcheck(
            l,
            (id as usize as lua_Number) == id && (0.0..=4294967295.0).contains(&id),
            2,
            c"invalid request id",
        );
        let result = mpack_rpc_reply(&mut session.session, &mut b, &mut bl, id as MpackUint32);
        debug_assert_eq!(result, MPACK_OK);
        lua_pushlstring(l, buf.as_ptr().cast(), buf.len() - bl);
        1
    }
}

unsafe extern "C-unwind" fn lmpack_session_notify(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) != 1 {
            return luaL_error(l, c"expecting exactly 1 argument".as_ptr());
        }

        let mut buf = [0u8; 16];
        let mut b = buf.as_mut_ptr();
        let mut bl = buf.len();
        let session = lmpack_check_session(l, 1);
        let result = mpack_rpc_notify(&mut session.session, &mut b, &mut bl);
        debug_assert_eq!(result, MPACK_OK);
        lua_pushlstring(l, buf.as_ptr().cast(), buf.len() - bl);
        1
    }
}

unsafe extern "C-unwind" fn lmpack_nil_tostring(l: *mut lua_State) -> i32 {
    unsafe {
        {
            lua_pushfstring(l, NIL_NAME.as_ptr(), lua_topointer(l, 1));
        }
        1
    }
}

unsafe extern "C-unwind" fn lmpack_unpack(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) != 1 {
            return luaL_error(l, c"expecting exactly 1 argument".as_ptr());
        }

        let mut len: usize = 0;
        let mut str_ptr = luaL_checklstring(l, 1, &mut len).cast::<u8>();

        lua_newtable(l);
        let reg = luaL_ref(l, LUA_REGISTRYINDEX);
        let mut unpacker = LmpackUnpacker {
            lstate: l,
            parser: MpackParser::new(0),
            reg,
            ext: LUA_NOREF,
            unpacking: 0,
            mtdict: 0,
            string_buffer: Vec::new(),
        };
        mpack_parser_init(&mut unpacker.parser, 0);
        unpacker.parser.data = MpackData::LmpackUnpacker(&raw mut unpacker);

        lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
        unpacker.mtdict = lmpack_ref(l, unpacker.reg);

        let result = mpack_parse(
            &mut unpacker.parser,
            &mut str_ptr,
            &mut len,
            Some(lmpack_parse_enter),
            Some(lmpack_parse_exit),
        );

        luaL_unref(l, LUA_REGISTRYINDEX, unpacker.reg);

        if result == MPACK_NOMEM {
            return luaL_error(l, c"object was too deep to unpack".as_ptr());
        } else if result == MPACK_EOF {
            return luaL_error(l, c"incomplete msgpack string".as_ptr());
        } else if result == MPACK_ERROR {
            return luaL_error(l, c"invalid msgpack string".as_ptr());
        } else if result == MPACK_OK && len != 0 {
            return luaL_error(l, c"trailing data in msgpack string".as_ptr());
        }

        debug_assert_eq!(result, MPACK_OK);
        1
    }
}

unsafe extern "C-unwind" fn lmpack_pack(l: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(l) != 1 {
            return luaL_error(l, c"expecting exactly 1 argument".as_ptr());
        }

        lua_newtable(l);
        let reg = luaL_ref(l, LUA_REGISTRYINDEX);
        let root = lmpack_ref(l, reg);
        let mut packer = Packer {
            lstate: l,
            parser: MpackParser::new(0),
            reg,
            ext: LUA_NOREF,
            root,
            packing: 0,
            mtdict: 0,
            is_bin: 0,
            is_bin_fn: LUA_NOREF,
        };
        mpack_parser_init(&mut packer.parser, 0);
        packer.parser.data = MpackData::LmpackPacker(&raw mut packer);

        lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
        packer.mtdict = lmpack_ref(l, packer.reg);

        let mut buffer = luaL_Buffer::new();
        luaL_buffinit(l, &mut buffer);
        let mut b = luaL_prepbuffer(&mut buffer).cast::<u8>();
        let mut bl = LUAL_BUFFERSIZE;

        loop {
            let bl_init = bl;
            let result = mpack_unparse(
                &mut packer.parser,
                &mut b,
                &mut bl,
                Some(lmpack_unparse_enter),
                Some(lmpack_unparse_exit),
            );

            if result == MPACK_NOMEM {
                lmpack_unref(l, packer.reg, packer.root);
                luaL_unref(l, LUA_REGISTRYINDEX, packer.reg);
                return luaL_error(l, c"object was too deep to pack".as_ptr());
            }

            luaL_addsize(&mut buffer, bl_init - bl);

            if bl == 0 {
                b = luaL_prepbuffer(&mut buffer).cast::<u8>();
                bl = LUAL_BUFFERSIZE;
            }

            if result != MPACK_EOF {
                break;
            }
        }

        lmpack_unref(l, packer.reg, packer.root);
        luaL_unref(l, LUA_REGISTRYINDEX, packer.reg);
        luaL_pushresult(&mut buffer);
        1
    }
}

const UNPACKER_METHODS: [luaL_Reg; 3] = [
    luaL_Reg {
        name: c"__call".as_ptr(),
        func: Some(lmpack_unpacker_unpack),
    },
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(lmpack_unpacker_delete),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

const PACKER_METHODS: [luaL_Reg; 3] = [
    luaL_Reg {
        name: c"__call".as_ptr(),
        func: Some(lmpack_packer_pack),
    },
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(lmpack_packer_delete),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

const SESSION_METHODS: [luaL_Reg; 6] = [
    luaL_Reg {
        name: c"receive".as_ptr(),
        func: Some(lmpack_session_receive),
    },
    luaL_Reg {
        name: c"request".as_ptr(),
        func: Some(lmpack_session_request),
    },
    luaL_Reg {
        name: c"reply".as_ptr(),
        func: Some(lmpack_session_reply),
    },
    luaL_Reg {
        name: c"notify".as_ptr(),
        func: Some(lmpack_session_notify),
    },
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(lmpack_session_delete),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

const MPACK_FUNCTIONS: [luaL_Reg; 6] = [
    luaL_Reg {
        name: c"Unpacker".as_ptr(),
        func: Some(lmpack_unpacker_new),
    },
    luaL_Reg {
        name: c"Packer".as_ptr(),
        func: Some(lmpack_packer_new),
    },
    luaL_Reg {
        name: c"Session".as_ptr(),
        func: Some(lmpack_session_new),
    },
    luaL_Reg {
        name: c"decode".as_ptr(),
        func: Some(lmpack_unpack),
    },
    luaL_Reg {
        name: c"encode".as_ptr(),
        func: Some(lmpack_pack),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

pub fn luaopen_mpack(l: *mut lua_State) -> i32 {
    unsafe {
        luaL_newmetatable(l, UNPACKER_META_NAME);
        lua_pushvalue(l, -1);
        lua_setfield(l, -2, c"__index");
        luaL_register(l, ptr::null(), UNPACKER_METHODS.as_ptr());
        lua_pop(l, 1);

        luaL_newmetatable(l, PACKER_META_NAME);
        lua_pushvalue(l, -1);
        lua_setfield(l, -2, c"__index");
        luaL_register(l, ptr::null(), PACKER_METHODS.as_ptr());
        lua_pop(l, 1);

        luaL_newmetatable(l, SESSION_META_NAME);
        lua_pushvalue(l, -1);
        lua_setfield(l, -2, c"__index");
        luaL_register(l, ptr::null(), SESSION_METHODS.as_ptr());
        lua_pop(l, 1);

        lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
        if lua_isnil(l, -1) {
            let _ = lua_newuserdata(l, size_of::<usize>());
            lua_createtable(l, 0, 1);
            lua_pushstring(l, c"__tostring");
            lua_pushcfunction(l, Some(lmpack_nil_tostring));
            lua_settable(l, -3);
            lua_setmetatable(l, -2);
            lua_setfield(l, LUA_REGISTRYINDEX, NIL_NAME);
        }
        lua_pop(l, 1);

        lua_newtable(l);
        luaL_register(l, ptr::null(), MPACK_FUNCTIONS.as_ptr());
        lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
        lua_setfield(l, -2, c"NIL");
        1
    }
}
