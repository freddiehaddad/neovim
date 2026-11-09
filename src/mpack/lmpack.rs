#![forbid(unsafe_code)]

//! Lua bindings for MessagePack (`src/mpack/lmpack.c`).

use std::ffi::CStr;
use std::mem::size_of;

use crate::lua::converter::{
    LUA_NOREF, LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE,
    LUA_TUSERDATA, lua_Integer, lua_Number, lua_State, lua_getfield, lua_isboolean, lua_isfunction,
    lua_isnil, lua_isnoneornil, lua_istable, lua_newtable, lua_pop, lua_pushstring, lua_setfield,
    luaL_Buffer, luaL_argcheck, luaL_getmetatable, luaL_newmetatable,
};
use crate::lua::ffi::{
    lua_call_safe, lua_createtable_safe, lua_getmetatable_safe, lua_gettable_safe, lua_insert_safe,
    lua_isnumber_safe, lua_isstring_safe, lua_isuserdata_safe, lua_newuserdata_size,
    lua_newuserdata_typed, lua_next_safe, lua_objlen_safe, lua_push_bytes, lua_pushboolean_safe,
    lua_pushcfunction_safe, lua_pushfstring_topointer, lua_pushinteger_safe, lua_pushnil_safe,
    lua_pushnumber_safe, lua_pushvalue_safe, lua_rawequal_safe, lua_rawgeti_safe, lua_remove_safe,
    lua_replace_safe, lua_setmetatable_safe, lua_settable_safe, lua_toboolean_safe,
    lua_tolstring_bytes, lua_tonumber_safe, lua_type_safe, luaL_addsize, luaL_buffinit_safe,
    luaL_checkbytes, luaL_checknumber_safe, luaL_checkudata_drop, luaL_checkudata_typed,
    luaL_error_cstr, luaL_error_int, luaL_prepbuffer_slice, luaL_pushresult_safe, luaL_ref_safe,
    luaL_register_safe, luaL_unref_safe, nlua_gettop,
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

fn lmpack_ref(l: *mut lua_State, reg: i32) -> i32 {
    lua_rawgeti_safe(l, LUA_REGISTRYINDEX, reg);
    lua_pushvalue_safe(l, -2);
    let rv = luaL_ref_safe(l, -2);
    lua_pop(l, 2);
    rv
}

fn lmpack_unref(l: *mut lua_State, reg: i32, r#ref: i32) {
    lua_rawgeti_safe(l, LUA_REGISTRYINDEX, reg);
    luaL_unref_safe(l, -1, r#ref);
    lua_pop(l, 1);
}

fn lmpack_geti(l: *mut lua_State, reg: i32, r#ref: i32) {
    lua_rawgeti_safe(l, LUA_REGISTRYINDEX, reg);
    lua_rawgeti_safe(l, -1, r#ref);
    lua_replace_safe(l, -2);
}

fn lmpack_shallow_copy(l: *mut lua_State) {
    lua_newtable(l);
    lua_pushnil_safe(l);
    while lua_next_safe(l, -3) != 0 {
        lua_pushvalue_safe(l, -2);
        lua_insert_safe(l, -2);
        lua_settable_safe(l, -4);
    }
    lua_remove_safe(l, -2);
}

fn lmpack_check_unpacker(l: *mut lua_State, index: i32) -> &'static mut LmpackUnpacker {
    luaL_checkudata_typed(l, index, UNPACKER_META_NAME)
}

fn lmpack_check_packer(l: *mut lua_State, index: i32) -> &'static mut Packer {
    luaL_checkudata_typed(l, index, PACKER_META_NAME)
}

fn lmpack_check_session(l: *mut lua_State, index: i32) -> &'static mut Session {
    luaL_checkudata_typed(l, index, SESSION_META_NAME)
}

fn lmpack_isnil(l: *mut lua_State, index: i32) -> bool {
    if !lua_isuserdata_safe(l, index) {
        return false;
    }
    lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
    let rv = lua_rawequal_safe(l, -1, -2);
    lua_pop(l, 1);
    rv
}

fn lmpack_isunpacker(l: *mut lua_State, index: i32) -> bool {
    if !lua_isuserdata_safe(l, index) || !lua_getmetatable_safe(l, index) {
        return false;
    }
    luaL_getmetatable(l, UNPACKER_META_NAME);
    let rv = lua_rawequal_safe(l, -1, -2);
    lua_pop(l, 2);
    rv
}

fn lmpack_pushnil(l: *mut lua_State) {
    lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
}

fn lmpack_objlen(l: *mut lua_State, is_array: Option<&mut i32>) -> MpackUint32 {
    let mut len: usize;
    if lua_type_safe(l, -1) != LUA_TTABLE {
        len = lua_objlen_safe(l, -1);
    } else {
        len = 0;
        let mut max: usize = 0;
        let mut isarr = true;
        lua_pushnil_safe(l);

        while lua_next_safe(l, -2) != 0 {
            lua_pop(l, 1);
            let mut n: lua_Number = 0.0;
            isarr = isarr
                && lua_type_safe(l, -1) == LUA_TNUMBER
                && {
                    n = lua_tonumber_safe(l, -1);
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

extern "C-unwind" fn lmpack_unpacker_new(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) > 1 {
        return luaL_error_cstr(l, c"expecting at most 1 table argument");
    }

    let rv = lua_newuserdata_typed(
        l,
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
    rv.parser.data = MpackData::LmpackUnpacker(crate::globals::GraphPtr::from(&mut *rv));
    luaL_getmetatable(l, UNPACKER_META_NAME);
    lua_setmetatable_safe(l, -2);

    lua_newtable(l);
    rv.reg = luaL_ref_safe(l, LUA_REGISTRYINDEX);

    lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
    rv.mtdict = lmpack_ref(l, rv.reg);

    if lua_istable(l, 1) {
        lua_getfield(l, 1, c"ext");
        if !lua_isnil(l, -1) {
            if !lua_istable(l, -1) {
                return luaL_error_cstr(l, c"\"ext\" option must be a table");
            }
            lmpack_shallow_copy(l);
        }
        rv.ext = lmpack_ref(l, rv.reg);
    }

    1
}

extern "C-unwind" fn lmpack_unpacker_delete(l: *mut lua_State) -> i32 {
    {
        let unpacker = lmpack_check_unpacker(l, 1);
        if unpacker.ext != LUA_NOREF {
            lmpack_unref(l, unpacker.reg, unpacker.ext);
        }
        luaL_unref_safe(l, LUA_REGISTRYINDEX, unpacker.reg);
    }
    luaL_checkudata_drop::<LmpackUnpacker>(l, 1, UNPACKER_META_NAME);
    0
}

fn lmpack_parse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    let unpacker = parser.data.lmpack_unpacker();
    let l = unpacker.lstate;

    match node.tok.r#type {
        MpackTokenNil => {
            lmpack_pushnil(l);
        }
        MpackTokenBoolean => {
            lua_pushboolean_safe(l, mpack_unpack_boolean(&node.tok));
        }
        MpackTokenUint | MpackTokenSint | MpackTokenFloat => {
            lua_pushnumber_safe(l, mpack_unpack_number(&node.tok));
        }
        MpackTokenChunk => {
            let parent = mpack_parent_node(&mut *node);
            let chunk = node.tok.data.chunk_bytes();
            let pos = parent.pos;
            unpacker.string_buffer[pos..pos + chunk.len()].copy_from_slice(chunk);
        }
        MpackTokenBin | MpackTokenStr | MpackTokenExt => {
            unpacker.string_buffer.clear();
            unpacker.string_buffer.resize(node.tok.length as usize, 0);
        }
        MpackTokenArray | MpackTokenMap => {
            lua_newtable(l);
            node.data[0] = crate::mpack::object::MpackData::Int(MpackSintmax::from(lmpack_ref(
                l,
                unpacker.reg,
            )));
        }
        _ => {}
    }
}

fn lmpack_parse_exit(parser: &mut MpackParser, node: &mut MpackNode) {
    let unpacker = parser.data.lmpack_unpacker();
    let l = unpacker.lstate;
    let mut parent = mpack_parent_node(&mut *node);

    match node.tok.r#type {
        MpackTokenBin | MpackTokenStr | MpackTokenExt => {
            lua_push_bytes(l, &unpacker.string_buffer[..node.tok.length as usize]);
            unpacker.string_buffer.clear();
            if node.tok.r#type == MpackTokenExt && unpacker.ext != LUA_NOREF {
                lmpack_geti(l, unpacker.reg, unpacker.ext);
                lua_rawgeti_safe(l, -1, node.tok.data.ext_type());
                if lua_isfunction(l, -1) {
                    lua_pushinteger_safe(l, node.tok.data.ext_type() as lua_Integer);
                    lua_pushvalue_safe(l, -4);
                    lua_call_safe(l, 2, 1);
                    lua_replace_safe(l, -3);
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
                lua_setmetatable_safe(l, -2);
            }
        }
        _ => {}
    }

    if !parent.is_null() && parent.tok.r#type < MpackTokenBin {
        lmpack_geti(l, unpacker.reg, parent.data[0].i() as i32);

        if parent.tok.r#type == MpackTokenArray {
            lua_pushnumber_safe(l, parent.pos as lua_Number);
            lua_pushvalue_safe(l, -3);
            lua_settable_safe(l, -3);
        } else {
            debug_assert_eq!(parent.tok.r#type, MpackTokenMap);
            if parent.key_visited != 0 {
                lua_pushvalue_safe(l, -2);
                parent.data[1] = crate::mpack::object::MpackData::Int(MpackSintmax::from(
                    lmpack_ref(l, unpacker.reg),
                ));
            } else {
                let key_ref = parent.data[1].i() as i32;
                lmpack_geti(l, unpacker.reg, key_ref);
                lmpack_unref(l, unpacker.reg, key_ref);
                lua_pushvalue_safe(l, -3);
                lua_settable_safe(l, -3);
            }
        }
        lua_pop(l, 2);
    }
}

fn lmpack_unpacker_unpack_str(
    l: *mut lua_State,
    unpacker: &mut LmpackUnpacker,
    buf: &mut &[u8],
) -> i32 {
    if unpacker.unpacking != 0 {
        return luaL_error_cstr(
            l,
            c"Unpacker instance already working. Use another Unpacker or mpack.decode() if you need to decode from the ext handler",
        );
    }

    let mut rv: i32;
    loop {
        unpacker.unpacking = 1;
        rv = mpack_parse(
            &mut unpacker.parser,
            buf,
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
        return luaL_error_cstr(l, c"invalid msgpack string");
    }

    rv
}

extern "C-unwind" fn lmpack_unpacker_unpack(l: *mut lua_State) -> i32 {
    let argc = nlua_gettop(l);
    if !(2..=3).contains(&argc) {
        return luaL_error_cstr(l, c"expecting between 2 and 3 arguments");
    }

    let unpacker = lmpack_check_unpacker(l, 1);
    unpacker.lstate = l;

    let full_slice = luaL_checkbytes(l, 2);
    let len = full_slice.len();
    let startpos = if argc == 3 {
        luaL_checknumber_safe(l, 3)
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
    let mut buf = &full_slice[offset..];
    let result = lmpack_unpacker_unpack_str(l, &mut *unpacker, &mut buf);

    if result == MPACK_EOF {
        lua_pushnil_safe(l);
    }

    let next_pos = (len - buf.len()) + 1;
    lua_pushinteger_safe(l, next_pos as lua_Integer);
    debug_assert_eq!(nlua_gettop(l), argc + 2);
    2
}

extern "C-unwind" fn lmpack_packer_new(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) > 1 {
        return luaL_error_cstr(l, c"expecting at most 1 table argument");
    }

    let rv = lua_newuserdata_typed(
        l,
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
    rv.parser.data = MpackData::LmpackPacker(crate::globals::GraphPtr::from(&mut *rv));
    luaL_getmetatable(l, PACKER_META_NAME);
    lua_setmetatable_safe(l, -2);

    lua_newtable(l);
    rv.reg = luaL_ref_safe(l, LUA_REGISTRYINDEX);
    rv.ext = LUA_NOREF;

    lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
    rv.mtdict = lmpack_ref(l, rv.reg);

    if lua_istable(l, 1) {
        lua_getfield(l, 1, c"ext");
        if !lua_isnil(l, -1) {
            if !lua_istable(l, -1) {
                return luaL_error_cstr(l, c"\"ext\" option must be a table");
            }
            lmpack_shallow_copy(l);
        }
        rv.ext = lmpack_ref(l, rv.reg);

        lua_getfield(l, 1, c"is_bin");
        if !lua_isnil(l, -1) {
            if !lua_isboolean(l, -1) && !lua_isfunction(l, -1) {
                return luaL_error_cstr(l, c"\"is_bin\" option must be a boolean or function");
            }
            rv.is_bin = i32::from(lua_toboolean_safe(l, -1));
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

extern "C-unwind" fn lmpack_packer_delete(l: *mut lua_State) -> i32 {
    {
        let packer = lmpack_check_packer(l, 1);
        if packer.ext != LUA_NOREF {
            lmpack_unref(l, packer.reg, packer.ext);
        }
        luaL_unref_safe(l, LUA_REGISTRYINDEX, packer.reg);
    }
    luaL_checkudata_drop::<Packer>(l, 1, PACKER_META_NAME);
    0
}

fn lmpack_unparse_enter(parser: &mut MpackParser, node: &mut MpackNode) {
    let packer = parser.data.lmpack_packer();
    let l = packer.lstate;
    let mut parent = mpack_parent_node(&mut *node);

    if !parent.is_null() {
        lmpack_geti(l, packer.reg, parent.data[0].i() as i32);

        if parent.tok.r#type > MpackTokenMap {
            let len = parent.tok.length as usize;
            let bytes = lua_tolstring_bytes(l, -1);
            let chunk = if len == 0 || bytes.is_empty() {
                &[]
            } else {
                &bytes[..len.min(bytes.len())]
            };
            node.tok = mpack_pack_chunk(chunk);
            lua_pop(l, 1);
            return;
        }

        if parent.tok.r#type == MpackTokenArray {
            lua_pushnumber_safe(l, (parent.pos + 1) as lua_Number);
            lua_gettable_safe(l, -2);
        } else if parent.tok.r#type == MpackTokenMap {
            lmpack_geti(l, packer.reg, parent.data[1].i() as i32);
            let result = lua_next_safe(l, -2);
            debug_assert_ne!(result, 0);
            if parent.key_visited != 0 {
                lmpack_unref(l, packer.reg, parent.data[1].i() as i32);
                lua_pushvalue_safe(l, -2);
                parent.data[1] = crate::mpack::object::MpackData::Int(MpackSintmax::from(
                    lmpack_ref(l, packer.reg),
                ));
                lua_replace_safe(l, -2);
            } else {
                lua_pop(l, 1);
            }
        }
        lua_remove_safe(l, -2);
    } else {
        lmpack_geti(l, packer.reg, packer.root);
    }

    let r#type = lua_type_safe(l, -1);

    'done: {
        match r#type {
            LUA_TBOOLEAN => {
                node.tok = mpack_pack_boolean(u32::from(lua_toboolean_safe(l, -1)));
            }
            LUA_TNUMBER => {
                node.tok = mpack_pack_number(lua_tonumber_safe(l, -1));
            }
            LUA_TSTRING => {
                let mut is_bin = packer.is_bin;
                if is_bin != 0 && packer.is_bin_fn != LUA_NOREF {
                    lmpack_geti(l, packer.reg, packer.is_bin_fn);
                    lua_pushvalue_safe(l, -2);
                    lua_call_safe(l, 1, 1);
                    is_bin = i32::from(lua_toboolean_safe(l, -1));
                    lua_pop(l, 1);
                }
                if is_bin != 0 {
                    node.tok = mpack_pack_bin(lmpack_objlen(l, None));
                } else {
                    node.tok = mpack_pack_str(lmpack_objlen(l, None));
                }
            }
            LUA_TTABLE => {
                let has_meta = lua_getmetatable_safe(l, -1);
                let mut has_mtdict = false;
                if has_meta && packer.mtdict != LUA_NOREF {
                    lmpack_geti(l, packer.reg, packer.mtdict);
                    has_mtdict = lua_rawequal_safe(l, -1, -2);
                    lua_pop(l, 1);
                }
                if packer.ext != LUA_NOREF && has_meta && !has_mtdict {
                    lmpack_geti(l, packer.reg, packer.ext);
                    lua_pushvalue_safe(l, -2);
                    lua_gettable_safe(l, -2);
                    if lua_isfunction(l, -1) {
                        lua_pushvalue_safe(l, -4);
                        lua_call_safe(l, 1, 2);
                        let mut ext: lua_Number = -1.0;
                        if !lua_isnumber_safe(l, -2)
                            || {
                                ext = lua_tonumber_safe(l, -2);
                                !(0.0..=127.0).contains(&ext)
                            }
                            || (ext as i32 as lua_Number) != ext
                        {
                            luaL_error_cstr(
                                l,
                                c"the first result from ext packer must be an integer between 0 and 127",
                            );
                        }
                        if !lua_isstring_safe(l, -1) {
                            luaL_error_cstr(
                                l,
                                c"the second result from ext packer must be a string",
                            );
                        }
                        node.tok = mpack_pack_ext(ext as i32, lmpack_objlen(l, None));
                        lua_replace_safe(l, -5);
                        lua_pop(l, 3);
                        break 'done;
                    } else {
                        lua_pop(l, 1);
                    }
                }

                if has_meta {
                    lua_pop(l, 1);
                }

                let mut n = mpack_parent_node(&mut *node);
                while !n.is_null() {
                    lmpack_geti(l, packer.reg, n.data[0].i() as i32);
                    if lua_rawequal_safe(l, -1, -2) {
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
                luaL_error_int(l, c"can't serialize object of type %d", r#type);
            }
        }
    }

    node.data[0] =
        crate::mpack::object::MpackData::Int(MpackSintmax::from(lmpack_ref(l, packer.reg)));
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

extern "C-unwind" fn lmpack_packer_pack(l: *mut lua_State) -> i32 {
    let argc = nlua_gettop(l);
    if argc != 2 {
        return luaL_error_cstr(l, c"expecting exactly 2 arguments");
    }

    let packer = lmpack_check_packer(l, 1);
    packer.lstate = l;
    packer.root = lmpack_ref(l, packer.reg);
    let mut buffer = luaL_Buffer::new();
    luaL_buffinit_safe(l, &mut buffer);
    let mut b = luaL_prepbuffer_slice(&mut buffer);

    if packer.packing != 0 {
        return luaL_error_cstr(
            l,
            c"Packer instance already working. Use another Packer or mpack.encode() if you need to encode from the ext handler",
        );
    }

    loop {
        let bl_init = b.len();
        packer.packing = 1;
        let result = mpack_unparse(
            &mut packer.parser,
            &mut b,
            Some(lmpack_unparse_enter),
            Some(lmpack_unparse_exit),
        );
        packer.packing = 0;

        if result == MPACK_NOMEM {
            mpack_parser_grow(&mut packer.parser);
        }

        let bl = b.len();
        luaL_addsize(&mut buffer, bl_init - bl);

        if bl == 0 {
            b = luaL_prepbuffer_slice(&mut buffer);
        }

        if result != MPACK_EOF && result != MPACK_NOMEM {
            break;
        }
    }

    lmpack_unref(l, packer.reg, packer.root);
    luaL_pushresult_safe(&mut buffer);
    debug_assert_eq!(nlua_gettop(l), argc);
    1
}

extern "C-unwind" fn lmpack_session_new(l: *mut lua_State) -> i32 {
    let rv = lua_newuserdata_typed(
        l,
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
    lua_setmetatable_safe(l, -2);

    lua_newtable(l);
    rv.reg = luaL_ref_safe(l, LUA_REGISTRYINDEX);

    if lua_istable(l, 1) {
        lua_getfield(l, 1, c"unpack");
        if !lmpack_isunpacker(l, -1) {
            return luaL_error_cstr(l, c"\"unpack\" option must be a mpack.Unpacker instance");
        }
        rv.unpacker = lmpack_ref(l, rv.reg);
    }

    1
}

extern "C-unwind" fn lmpack_session_delete(l: *mut lua_State) -> i32 {
    {
        let session = lmpack_check_session(l, 1);
        lmpack_unref(l, session.reg, session.unpacker);
        luaL_unref_safe(l, LUA_REGISTRYINDEX, session.reg);
    }
    luaL_checkudata_drop::<Session>(l, 1, SESSION_META_NAME);
    0
}

extern "C-unwind" fn lmpack_session_receive(l: *mut lua_State) -> i32 {
    let argc = nlua_gettop(l);
    if !(2..=3).contains(&argc) {
        return luaL_error_cstr(l, c"expecting between 2 and 3 arguments");
    }

    let mut rcount: i32 = 3;
    let session = lmpack_check_session(l, 1);
    let full_slice = luaL_checkbytes(l, 2);
    let len = full_slice.len();
    let startpos = if argc == 3 {
        luaL_checknumber_safe(l, 3)
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
    let mut buf = &full_slice[offset..];

    let mut unpacker: crate::globals::GraphPtr<LmpackUnpacker> = crate::globals::GraphPtr::NULL;
    if session.unpacker != LUA_REFNIL {
        lmpack_geti(l, session.reg, session.unpacker);
        unpacker = crate::globals::GraphPtr::from(lmpack_check_unpacker(l, -1));
        unpacker.lstate = l;
        rcount += 2;
        lua_pop(l, 1);
    }

    loop {
        if session.unpacked.r#type == MPACK_EOF {
            session.unpacked.r#type =
                mpack_rpc_receive(&mut session.session, &mut buf, &mut session.unpacked.msg);

            if unpacker.is_null() || session.unpacked.r#type == MPACK_EOF {
                break;
            }
        }

        let result = lmpack_unpacker_unpack_str(l, unpacker.borrow_mut(), &mut buf);
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
        lua_pushnil_safe(l);
        lua_pushnil_safe(l);
        if !unpacker.is_null() {
            lua_pushnil_safe(l);
            lua_pushnil_safe(l);
        }
    } else {
        match session.unpacked.r#type {
            MPACK_RPC_REQUEST => {
                lua_pushstring(l, c"request");
                lua_pushnumber_safe(l, lua_Number::from(session.unpacked.msg.id));
            }
            MPACK_RPC_RESPONSE => {
                lua_pushstring(l, c"response");
                lmpack_geti(l, session.reg, session.unpacked.msg.data.i() as i32);
            }
            MPACK_RPC_NOTIFICATION => {
                lua_pushstring(l, c"notification");
                lua_pushnil_safe(l);
            }
            _ => {
                return luaL_error_cstr(l, c"invalid msgpack-rpc string");
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

    let next_pos = (len - buf.len()) + 1;
    lua_pushinteger_safe(l, next_pos as lua_Integer);
    rcount
}

extern "C-unwind" fn lmpack_session_request(l: *mut lua_State) -> i32 {
    if !(1..=2).contains(&nlua_gettop(l)) {
        return luaL_error_cstr(l, c"expecting 1 or 2 arguments");
    }

    let mut buf = [0u8; 16];
    let mut b = &mut buf[..];
    let session = lmpack_check_session(l, 1);
    let data = MpackData::Int(MpackSintmax::from(if lua_isnoneornil(l, 2) {
        LUA_NOREF
    } else {
        lmpack_ref(l, session.reg)
    }));

    let mut result: i32;
    loop {
        result = mpack_rpc_request(&mut session.session, &mut b, data.clone());
        if result == MPACK_NOMEM {
            mpack_rpc_session_grow(&mut session.session);
        } else {
            break;
        }
    }

    debug_assert_eq!(result, MPACK_OK);
    let written = 16 - b.len();
    lua_push_bytes(l, &buf[..written]);
    1
}

extern "C-unwind" fn lmpack_session_reply(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) != 2 {
        return luaL_error_cstr(l, c"expecting exactly 2 arguments");
    }

    let mut buf = [0u8; 16];
    let mut b = &mut buf[..];
    let session = lmpack_check_session(l, 1);
    let id = lua_tonumber_safe(l, 2);
    luaL_argcheck(
        l,
        (id as usize as lua_Number) == id && (0.0..=4294967295.0).contains(&id),
        2,
        c"invalid request id",
    );
    let result = mpack_rpc_reply(&mut session.session, &mut b, id as MpackUint32);
    debug_assert_eq!(result, MPACK_OK);
    let written = 16 - b.len();
    lua_push_bytes(l, &buf[..written]);
    1
}

extern "C-unwind" fn lmpack_session_notify(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) != 1 {
        return luaL_error_cstr(l, c"expecting exactly 1 argument");
    }

    let mut buf = [0u8; 16];
    let mut b = &mut buf[..];
    let session = lmpack_check_session(l, 1);
    let result = mpack_rpc_notify(&mut session.session, &mut b);
    debug_assert_eq!(result, MPACK_OK);
    let written = 16 - b.len();
    lua_push_bytes(l, &buf[..written]);
    1
}

extern "C-unwind" fn lmpack_nil_tostring(l: *mut lua_State) -> i32 {
    lua_pushfstring_topointer(l, NIL_NAME, 1);
    1
}

extern "C-unwind" fn lmpack_unpack(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) != 1 {
        return luaL_error_cstr(l, c"expecting exactly 1 argument");
    }

    let mut buf = luaL_checkbytes(l, 1);

    lua_newtable(l);
    let reg = luaL_ref_safe(l, LUA_REGISTRYINDEX);
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
    unpacker.parser.data = MpackData::LmpackUnpacker(crate::globals::GraphPtr::from(&mut unpacker));

    lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
    unpacker.mtdict = lmpack_ref(l, unpacker.reg);

    let result = mpack_parse(
        &mut unpacker.parser,
        &mut buf,
        Some(lmpack_parse_enter),
        Some(lmpack_parse_exit),
    );

    luaL_unref_safe(l, LUA_REGISTRYINDEX, unpacker.reg);

    if result == MPACK_NOMEM {
        return luaL_error_cstr(l, c"object was too deep to unpack");
    } else if result == MPACK_EOF {
        return luaL_error_cstr(l, c"incomplete msgpack string");
    } else if result == MPACK_ERROR {
        return luaL_error_cstr(l, c"invalid msgpack string");
    } else if result == MPACK_OK && !buf.is_empty() {
        return luaL_error_cstr(l, c"trailing data in msgpack string");
    }

    debug_assert_eq!(result, MPACK_OK);
    1
}

extern "C-unwind" fn lmpack_pack(l: *mut lua_State) -> i32 {
    if nlua_gettop(l) != 1 {
        return luaL_error_cstr(l, c"expecting exactly 1 argument");
    }

    lua_newtable(l);
    let reg = luaL_ref_safe(l, LUA_REGISTRYINDEX);
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
    packer.parser.data = MpackData::LmpackPacker(crate::globals::GraphPtr::from(&mut packer));

    lua_getfield(l, LUA_REGISTRYINDEX, EMPTY_DICT_NAME);
    packer.mtdict = lmpack_ref(l, packer.reg);

    let mut buffer = luaL_Buffer::new();
    luaL_buffinit_safe(l, &mut buffer);
    let mut b = luaL_prepbuffer_slice(&mut buffer);

    loop {
        let bl_init = b.len();
        let result = mpack_unparse(
            &mut packer.parser,
            &mut b,
            Some(lmpack_unparse_enter),
            Some(lmpack_unparse_exit),
        );

        if result == MPACK_NOMEM {
            lmpack_unref(l, packer.reg, packer.root);
            luaL_unref_safe(l, LUA_REGISTRYINDEX, packer.reg);
            return luaL_error_cstr(l, c"object was too deep to pack");
        }

        let bl = b.len();
        luaL_addsize(&mut buffer, bl_init - bl);

        if bl == 0 {
            b = luaL_prepbuffer_slice(&mut buffer);
        }

        if result != MPACK_EOF {
            break;
        }
    }

    lmpack_unref(l, packer.reg, packer.root);
    luaL_unref_safe(l, LUA_REGISTRYINDEX, packer.reg);
    luaL_pushresult_safe(&mut buffer);
    1
}

const UNPACKER_METHODS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 2] = [
    (c"__call", lmpack_unpacker_unpack),
    (c"__gc", lmpack_unpacker_delete),
];

const PACKER_METHODS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 2] = [
    (c"__call", lmpack_packer_pack),
    (c"__gc", lmpack_packer_delete),
];

const SESSION_METHODS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 5] = [
    (c"receive", lmpack_session_receive),
    (c"request", lmpack_session_request),
    (c"reply", lmpack_session_reply),
    (c"notify", lmpack_session_notify),
    (c"__gc", lmpack_session_delete),
];

const MPACK_FUNCTIONS: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 5] = [
    (c"Unpacker", lmpack_unpacker_new),
    (c"Packer", lmpack_packer_new),
    (c"Session", lmpack_session_new),
    (c"decode", lmpack_unpack),
    (c"encode", lmpack_pack),
];

pub fn luaopen_mpack(l: *mut lua_State) -> i32 {
    luaL_newmetatable(l, UNPACKER_META_NAME);
    lua_pushvalue_safe(l, -1);
    lua_setfield(l, -2, c"__index");
    luaL_register_safe(l, None, &UNPACKER_METHODS);
    lua_pop(l, 1);

    luaL_newmetatable(l, PACKER_META_NAME);
    lua_pushvalue_safe(l, -1);
    lua_setfield(l, -2, c"__index");
    luaL_register_safe(l, None, &PACKER_METHODS);
    lua_pop(l, 1);

    luaL_newmetatable(l, SESSION_META_NAME);
    lua_pushvalue_safe(l, -1);
    lua_setfield(l, -2, c"__index");
    luaL_register_safe(l, None, &SESSION_METHODS);
    lua_pop(l, 1);

    lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
    if lua_isnil(l, -1) {
        lua_newuserdata_size(l, size_of::<usize>());
        lua_createtable_safe(l, 0, 1);
        lua_pushstring(l, c"__tostring");
        lua_pushcfunction_safe(l, lmpack_nil_tostring);
        lua_settable_safe(l, -3);
        lua_setmetatable_safe(l, -2);
        lua_setfield(l, LUA_REGISTRYINDEX, NIL_NAME);
    }
    lua_pop(l, 1);

    lua_newtable(l);
    luaL_register_safe(l, None, &MPACK_FUNCTIONS);
    lua_getfield(l, LUA_REGISTRYINDEX, NIL_NAME);
    lua_setfield(l, -2, c"NIL");
    1
}
