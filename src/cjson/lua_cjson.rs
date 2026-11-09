//! JSON support for Lua (`src/cjson/lua_cjson.c`).

use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;

use crate::cjson::fpconv::{FPCONV_G_FMT_BUFSIZE, fpconv_g_fmt, fpconv_strtod};
use crate::cjson::strbuf::{
    Strbuf, strbuf_append_char, strbuf_append_char_unsafe, strbuf_append_mem,
    strbuf_append_mem_unsafe, strbuf_append_string, strbuf_ensure_empty_length, strbuf_ensure_null,
    strbuf_init, strbuf_length, strbuf_reset, strbuf_set_length, strbuf_string,
};
use crate::lua::converter::{
    LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TLIGHTUSERDATA, LUA_TNIL, LUA_TNUMBER, LUA_TSTRING,
    LUA_TTABLE, LUA_TUSERDATA, lua_Integer, lua_Number, lua_State, lua_call, lua_checkstack,
    lua_getfield, lua_getmetatable, lua_gettable, lua_gettop, lua_isnil, lua_newtable,
    lua_newuserdata, lua_next, lua_objlen, lua_pop, lua_pushboolean, lua_pushcclosure,
    lua_pushcfunction, lua_pushinteger, lua_pushlightuserdata, lua_pushlstring, lua_pushnil,
    lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawequal, lua_rawget, lua_rawgeti,
    lua_rawset, lua_rawseti, lua_setfield, lua_setmetatable, lua_toboolean, lua_tointeger,
    lua_tolstring, lua_tonumber, lua_tostring, lua_touserdata, lua_type, lua_typename,
    lua_upvalueindex, luaL_Reg, luaL_checklstring, luaL_checkstack, luaL_checkstring,
    luaL_checktype, luaL_error, luaL_getmetafield,
};
use crate::lua::executor::{nlua_get_empty_dict_ref, nlua_get_nil_ref, nlua_pushref};

const CJSON_MODNAME: &[u8] = b"cjson";
const CJSON_VERSION: &[u8] = b"2.1.0.11";

const DEFAULT_SPARSE_RATIO: i32 = 2;
const DEFAULT_SPARSE_SAFE: i32 = 10;
const DEFAULT_ENCODE_MAX_DEPTH: i32 = 1000;
const DEFAULT_DECODE_MAX_DEPTH: i32 = 1000;
const DEFAULT_ENCODE_NUMBER_PRECISION: i32 = 16;
const DEFAULT_ENCODE_SORT_KEYS: i32 = 0;

const KEYBUF_DEFAULT_CAPACITY: usize = 32;

static JSON_EMPTY_ARRAY: u8 = 0;
static JSON_ARRAY: u8 = 0;

#[inline]
fn json_lightudata_mask(ludata: *const u8) -> *mut u8 {
    #[cfg(target_pointer_width = "64")]
    {
        ((ludata as usize) & ((1usize << 47) - 1)) as *mut u8
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        ludata.cast_mut()
    }
}

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
enum JsonTokenType {
    TObjBegin = 0,
    TObjEnd = 1,
    TArrBegin = 2,
    TArrEnd = 3,
    TString = 4,
    TNumber = 5,
    TInteger = 6,
    TBoolean = 7,
    TNull = 8,
    TColon = 9,
    TComma = 10,
    TEnd = 11,
    TWhitespace = 12,
    TError = 13,
    TUnknown = 14,
}

use JsonTokenType::*;

const JSON_TOKEN_TYPE_NAME: [&CStr; 15] = [
    c"T_OBJ_BEGIN",
    c"T_OBJ_END",
    c"T_ARR_BEGIN",
    c"T_ARR_END",
    c"T_STRING",
    c"T_NUMBER",
    c"T_INTEGER",
    c"T_BOOLEAN",
    c"T_NULL",
    c"T_COLON",
    c"T_COMMA",
    c"T_END",
    c"T_WHITESPACE",
    c"T_ERROR",
    c"T_UNKNOWN",
];

#[derive(Copy, Clone)]
enum KeyEntryRaw {
    Number(lua_Number),
    String(Option<&'static CStr>),
}

#[repr(C)]
#[derive(Copy, Clone)]
struct KeyEntry {
    offset: usize,
    length: usize,
    raw: KeyEntryRaw,
}

#[repr(C)]
struct Keybuf {
    buf: Strbuf,
    keys: Vec<KeyEntry>,
}

impl Keybuf {
    const fn new() -> Self {
        Self {
            buf: Strbuf::new(),
            keys: Vec::new(),
        }
    }
}

impl Default for Keybuf {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
struct JsonConfig {
    encode_buf: Strbuf,
}

const CH2TOKEN: [JsonTokenType; 256] = {
    let mut tbl = [TError; 256];
    tbl[b'{' as usize] = TObjBegin;
    tbl[b'}' as usize] = TObjEnd;
    tbl[b'[' as usize] = TArrBegin;
    tbl[b']' as usize] = TArrEnd;
    tbl[b',' as usize] = TComma;
    tbl[b':' as usize] = TColon;
    tbl[b'\0' as usize] = TEnd;
    tbl[b' ' as usize] = TWhitespace;
    tbl[b'\t' as usize] = TWhitespace;
    tbl[b'\n' as usize] = TWhitespace;
    tbl[b'\r' as usize] = TWhitespace;
    tbl[b'f' as usize] = TUnknown;
    tbl[b'i' as usize] = TUnknown;
    tbl[b'I' as usize] = TUnknown;
    tbl[b'n' as usize] = TUnknown;
    tbl[b'N' as usize] = TUnknown;
    tbl[b't' as usize] = TUnknown;
    tbl[b'"' as usize] = TUnknown;
    tbl[b'+' as usize] = TUnknown;
    tbl[b'-' as usize] = TUnknown;
    let mut i = 0usize;
    while i < 10 {
        tbl[(b'0' as usize) + i] = TUnknown;
        i += 1;
    }
    tbl
};

const ESCAPE2CHAR: [u8; 256] = {
    let mut tbl = [0u8; 256];
    tbl[b'"' as usize] = b'"';
    tbl[b'\\' as usize] = b'\\';
    tbl[b'/' as usize] = b'/';
    tbl[b'b' as usize] = 0x08;
    tbl[b't' as usize] = b'\t';
    tbl[b'n' as usize] = b'\n';
    tbl[b'f' as usize] = 0x0c;
    tbl[b'r' as usize] = b'\r';
    tbl[b'u' as usize] = b'u';
    tbl
};

struct JsonEncodeOptions {
    char2escape: [Option<&'static CStr>; 256],
    indent: Option<&'static CStr>,
    sort_keys: i32,
    keybuf: Keybuf,
}

struct JsonEncode<'a> {
    options: JsonEncodeOptions,
    json: &'a mut Strbuf,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct JsonOptions {
    luanil_object: bool,
    luanil_array: bool,
    skip_comments: bool,
}

struct JsonParse<'a> {
    data: &'a [u8],
    pos: usize,
    tmp: Strbuf,
    options: JsonOptions,
    current_depth: i32,
}

#[derive(Copy, Clone, Default)]
enum JsonTokenValue {
    #[default]
    None,
    Error(&'static CStr),
    Number(f64),
    Integer(lua_Integer),
    Boolean(i32),
}

impl JsonTokenValue {
    const fn as_error(self) -> &'static CStr {
        match self {
            Self::Error(s) => s,
            _ => c"",
        }
    }

    const fn as_number(self) -> f64 {
        match self {
            Self::Number(n) => n,
            _ => 0.0,
        }
    }

    const fn as_integer(self) -> lua_Integer {
        match self {
            Self::Integer(i) => i,
            _ => 0,
        }
    }

    const fn as_boolean(self) -> i32 {
        match self {
            Self::Boolean(b) => b,
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct JsonToken {
    r#type: JsonTokenType,
    index: usize,
    value: JsonTokenValue,
    string_len: usize,
}

impl JsonToken {
    const fn new() -> Self {
        Self {
            r#type: TObjBegin,
            index: 0,
            value: JsonTokenValue::None,
            string_len: 0,
        }
    }
}

impl Default for JsonToken {
    fn default() -> Self {
        Self::new()
    }
}

const fn init_char2escape() -> [Option<&'static CStr>; 256] {
    let mut table: [Option<&'static CStr>; 256] = [None; 256];
    table[0x00] = Some(c"\\u0000");
    table[0x01] = Some(c"\\u0001");
    table[0x02] = Some(c"\\u0002");
    table[0x03] = Some(c"\\u0003");
    table[0x04] = Some(c"\\u0004");
    table[0x05] = Some(c"\\u0005");
    table[0x06] = Some(c"\\u0006");
    table[0x07] = Some(c"\\u0007");
    table[0x08] = Some(c"\\b");
    table[0x09] = Some(c"\\t");
    table[0x0a] = Some(c"\\n");
    table[0x0b] = Some(c"\\u000b");
    table[0x0c] = Some(c"\\f");
    table[0x0d] = Some(c"\\r");
    table[0x0e] = Some(c"\\u000e");
    table[0x0f] = Some(c"\\u000f");
    table[0x10] = Some(c"\\u0010");
    table[0x11] = Some(c"\\u0011");
    table[0x12] = Some(c"\\u0012");
    table[0x13] = Some(c"\\u0013");
    table[0x14] = Some(c"\\u0014");
    table[0x15] = Some(c"\\u0015");
    table[0x16] = Some(c"\\u0016");
    table[0x17] = Some(c"\\u0017");
    table[0x18] = Some(c"\\u0018");
    table[0x19] = Some(c"\\u0019");
    table[0x1a] = Some(c"\\u001a");
    table[0x1b] = Some(c"\\u001b");
    table[0x1c] = Some(c"\\u001c");
    table[0x1d] = Some(c"\\u001d");
    table[0x1e] = Some(c"\\u001e");
    table[0x1f] = Some(c"\\u001f");
    table[b'"' as usize] = Some(c"\\\"");
    table[b'\\' as usize] = Some(c"\\\\");
    table[0x7f] = Some(c"\\u007f");
    table
}

const CHAR2ESCAPE: [Option<&'static CStr>; 256] = init_char2escape();

use std::process::abort;

fn json_fetch_config(l: *mut lua_State) -> &'static mut JsonConfig {
    unsafe {
        let cfg = lua_touserdata(l, lua_upvalueindex(1)).cast::<JsonConfig>();
        if cfg.is_null() {
            luaL_error(l, c"BUG: Unable to fetch CJSON configuration".as_ptr());
        }
        &mut *cfg
    }
}

unsafe extern "C-unwind" fn json_destroy_config(l: *mut lua_State) -> i32 {
    unsafe {
        {
            let cfg = lua_touserdata(l, 1).cast::<JsonConfig>();
            if !cfg.is_null() {
                ptr::drop_in_place(cfg);
            }
        }
        0
    }
}

fn json_create_config(l: *mut lua_State) {
    unsafe {
        let cfg = lua_newuserdata(l, size_of::<JsonConfig>()).cast::<JsonConfig>();
        if cfg.is_null() {
            abort();
        }
        ptr::write(
            cfg,
            JsonConfig {
                encode_buf: Strbuf::new(),
            },
        );

        lua_newtable(l);
        lua_pushcfunction(l, Some(json_destroy_config));
        lua_setfield(l, -2, c"__gc");
        lua_setmetatable(l, -2);

        strbuf_init(&mut (*cfg).encode_buf, 0);
    }
}

fn json_encode_exception(
    l: *mut lua_State,
    ctx: &mut JsonEncode<'_>,
    lindex: i32,
    reason: &std::ffi::CStr,
) -> ! {
    unsafe {
        if ctx.options.sort_keys != 0 {
            ctx.options.keybuf = Keybuf::new();
        }

        luaL_error(
            l,
            c"Cannot serialise %s: %s".as_ptr(),
            lua_typename(l, lua_type(l, lindex)),
            reason.as_ptr(),
        );
        unreachable!()
    }
}

fn json_append_string_contents(
    l: *mut lua_State,
    ctx: &mut JsonEncode<'_>,
    lindex: i32,
    use_keybuf: bool,
) {
    unsafe {
        let mut len: usize = 0;
        let str_ptr = lua_tolstring(l, lindex, &mut len);
        let str_bytes = if str_ptr.is_null() || len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(str_ptr.cast::<u8>(), len)
        };

        if len >= usize::MAX / 6 {
            abort();
        }
        let escape_table = ctx.options.char2escape;
        let json: &mut Strbuf = if use_keybuf {
            &mut ctx.options.keybuf.buf
        } else {
            ctx.json
        };
        strbuf_ensure_empty_length(json, len * 6);

        for &ch in str_bytes {
            if let Some(escstr) = escape_table[ch as usize] {
                strbuf_append_string(json, escstr);
            } else {
                strbuf_append_char_unsafe(json, ch);
            }
        }
    }
}

fn json_append_string(l: *mut lua_State, ctx: &mut JsonEncode<'_>, lindex: i32) {
    strbuf_append_char(ctx.json, b'"');
    json_append_string_contents(l, ctx, lindex, false);
    strbuf_append_char(ctx.json, b'"');
}

fn lua_array_length(l: *mut lua_State, ctx: &mut JsonEncode<'_>) -> i32 {
    unsafe {
        let mut max: i32 = 0;
        let mut items: i32 = 0;

        lua_pushnil(l);
        while lua_next(l, -2) != 0 {
            if lua_type(l, -2) == LUA_TNUMBER {
                let k = lua_tonumber(l, -2);
                if k != 0.0 && k.floor() == k && k >= 1.0 {
                    if k > f64::from(max) {
                        max = k as i32;
                    }
                    items += 1;
                    lua_pop(l, 1);
                    continue;
                }
            }

            lua_pop(l, 2);
            return -1;
        }

        if max > items * DEFAULT_SPARSE_RATIO && max > DEFAULT_SPARSE_SAFE {
            json_encode_exception(l, ctx, -1, c"excessively sparse array");
        }

        max
    }
}

fn json_check_encode_depth(l: *mut lua_State, ctx: &mut JsonEncode<'_>, current_depth: i32) {
    unsafe {
        if current_depth <= DEFAULT_ENCODE_MAX_DEPTH && lua_checkstack(l, 3) != 0 {
            return;
        }

        if ctx.options.sort_keys != 0 {
            ctx.options.keybuf = Keybuf::new();
        }

        luaL_error(
            l,
            c"Cannot serialise, excessive nesting (%d)".as_ptr(),
            current_depth,
        );
    }
}

fn json_append_newline_and_indent(ctx: &mut JsonEncode<'_>, depth: i32) {
    strbuf_append_char(ctx.json, b'\n');
    let indent_cstr = ctx.options.indent.unwrap_or(c"");
    for _ in 0..depth {
        strbuf_append_string(ctx.json, indent_cstr);
    }
}

fn json_append_array(
    l: *mut lua_State,
    ctx: &mut JsonEncode<'_>,
    current_depth: i32,
    array_length: i32,
    raw: i32,
) {
    unsafe {
        let mut has_items = false;

        strbuf_append_char(ctx.json, b'[');

        let mut comma: i32 = 0;
        for i in 1..=array_length {
            has_items = true;

            let json_pos = strbuf_length(ctx.json) as i32;
            if comma > 0 {
                strbuf_append_char(ctx.json, b',');
            }
            comma += 1;

            if ctx.options.indent.is_some() {
                json_append_newline_and_indent(ctx, current_depth);
            }

            if raw != 0 {
                lua_rawgeti(l, -1, i);
            } else {
                lua_pushinteger(l, i as lua_Integer);
                lua_gettable(l, -2);
            }

            let err = json_append_data(l, ctx, current_depth);
            if err != 0 {
                strbuf_set_length(ctx.json, json_pos);
                if comma == 1 {
                    comma = 0;
                }
            }
            lua_pop(l, 1);
        }

        if has_items && ctx.options.indent.is_some() {
            json_append_newline_and_indent(ctx, current_depth - 1);
        }

        strbuf_append_char(ctx.json, b']');
    }
}

fn json_append_number(l: *mut lua_State, ctx: &mut JsonEncode<'_>, lindex: i32, use_keybuf: bool) {
    unsafe {
        let num = lua_tonumber(l, lindex);

        if num.is_infinite() || num.is_nan() {
            json_encode_exception(l, ctx, lindex, c"must not be NaN or Infinity");
        }

        let mut buf = [0u8; FPCONV_G_FMT_BUFSIZE];
        let len = fpconv_g_fmt(&mut buf, num, DEFAULT_ENCODE_NUMBER_PRECISION);
        let json: &mut Strbuf = if use_keybuf {
            &mut ctx.options.keybuf.buf
        } else {
            ctx.json
        };
        strbuf_append_mem(json, &buf[..len]);
    }
}

fn json_append_object(l: *mut lua_State, ctx: &mut JsonEncode<'_>, current_depth: i32) {
    unsafe {
        let mut has_items = false;

        strbuf_append_char(ctx.json, b'{');

        let mut comma: i32 = 0;
        lua_pushnil(l);
        if ctx.options.sort_keys != 0 {
            let init_keybuf_size = ctx.options.keybuf.keys.len();
            let init_keybuf_length = strbuf_length(&ctx.options.keybuf.buf);

            while lua_next(l, -2) != 0 {
                has_items = true;

                let keytype = lua_type(l, -2);
                let offset = strbuf_length(&ctx.options.keybuf.buf);
                let raw = if keytype == LUA_TSTRING {
                    json_append_string_contents(l, ctx, -2, true);
                    KeyEntryRaw::String(lua_tostring(l, -2))
                } else if keytype == LUA_TNUMBER {
                    json_append_number(l, ctx, -2, true);
                    KeyEntryRaw::Number(lua_tointeger(l, -2) as lua_Number)
                } else {
                    json_encode_exception(l, ctx, -2, c"table key must be number or string");
                };
                let length = strbuf_length(&ctx.options.keybuf.buf) - offset;
                ctx.options.keybuf.keys.push(KeyEntry {
                    offset,
                    length,
                    raw,
                });
                lua_pop(l, 1);
            }

            let keys_count = ctx.options.keybuf.keys.len() - init_keybuf_size;
            if keys_count > 0 {
                let key_bytes = &ctx.options.keybuf.buf.buf;
                let slice = &mut ctx.options.keybuf.keys.as_mut_slice()[init_keybuf_size..];
                slice.sort_by(|ka, kb| {
                    key_bytes[ka.offset..ka.offset + ka.length]
                        .cmp(&key_bytes[kb.offset..kb.offset + kb.length])
                });
            }

            for i in init_keybuf_size..(init_keybuf_size + keys_count) {
                let current_key = ctx.options.keybuf.keys.as_slice()[i];
                let json_pos = strbuf_length(ctx.json) as i32;
                if comma > 0 {
                    strbuf_append_char(ctx.json, b',');
                }
                comma += 1;

                if ctx.options.indent.is_some() {
                    json_append_newline_and_indent(ctx, current_depth);
                }

                strbuf_ensure_empty_length(ctx.json, current_key.length + 3);
                strbuf_append_char_unsafe(ctx.json, b'"');
                strbuf_append_mem_unsafe(
                    ctx.json,
                    &ctx.options.keybuf.buf.buf
                        [current_key.offset..current_key.offset + current_key.length],
                );
                strbuf_append_mem_unsafe(ctx.json, b"\":");

                if ctx.options.indent.is_some() {
                    strbuf_append_char(ctx.json, b' ');
                }

                match current_key.raw {
                    KeyEntryRaw::String(s) => {
                        lua_pushstring(l, s.unwrap_or(c""));
                    }
                    KeyEntryRaw::Number(n) => lua_pushnumber(l, n),
                }

                lua_gettable(l, -2);
                let err = json_append_data(l, ctx, current_depth);
                if err != 0 {
                    strbuf_set_length(ctx.json, json_pos);
                    if comma == 1 {
                        comma = 0;
                    }
                }
                lua_pop(l, 1);
            }
            strbuf_set_length(&mut ctx.options.keybuf.buf, init_keybuf_length as i32);
            ctx.options.keybuf.keys.truncate(init_keybuf_size);
        } else {
            while lua_next(l, -2) != 0 {
                has_items = true;

                let json_pos = strbuf_length(ctx.json) as i32;
                if comma > 0 {
                    strbuf_append_char(ctx.json, b',');
                }
                comma += 1;

                if ctx.options.indent.is_some() {
                    json_append_newline_and_indent(ctx, current_depth);
                }

                let keytype = lua_type(l, -2);
                if keytype == LUA_TNUMBER {
                    strbuf_append_char(ctx.json, b'"');
                    json_append_number(l, ctx, -2, false);
                    strbuf_append_mem(ctx.json, b"\":");
                } else if keytype == LUA_TSTRING {
                    json_append_string(l, ctx, -2);
                    strbuf_append_char(ctx.json, b':');
                } else {
                    json_encode_exception(l, ctx, -2, c"table key must be a number or string");
                }
                if ctx.options.indent.is_some() {
                    strbuf_append_char(ctx.json, b' ');
                }

                let err = json_append_data(l, ctx, current_depth);
                if err != 0 {
                    strbuf_set_length(ctx.json, json_pos);
                    if comma == 1 {
                        comma = 0;
                    }
                }

                lua_pop(l, 1);
            }
        }

        if has_items && ctx.options.indent.is_some() {
            json_append_newline_and_indent(ctx, current_depth - 1);
        }

        strbuf_append_char(ctx.json, b'}');
    }
}

fn json_append_data(l: *mut lua_State, ctx: &mut JsonEncode<'_>, mut current_depth: i32) -> i32 {
    unsafe {
        match lua_type(l, -1) {
            LUA_TSTRING => {
                json_append_string(l, ctx, -1);
            }
            LUA_TNUMBER => {
                json_append_number(l, ctx, -1, false);
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(l, -1) != 0 {
                    strbuf_append_mem(ctx.json, b"true");
                } else {
                    strbuf_append_mem(ctx.json, b"false");
                }
            }
            LUA_TTABLE => {
                current_depth += 1;
                json_check_encode_depth(l, ctx, current_depth);

                let mut as_array: i32 = 0;
                let mut as_empty_dict = false;
                let mut raw: i32 = 1;
                let has_metatable = lua_getmetatable(l, -1) != 0;

                if has_metatable {
                    nlua_pushref(l, nlua_get_empty_dict_ref(l));
                    if lua_rawequal(l, -2, -1) != 0 {
                        as_empty_dict = true;
                    } else {
                        lua_pop(l, 1);
                        lua_pushlightuserdata(
                            l,
                            json_lightudata_mask(&raw const JSON_ARRAY).cast(),
                        );
                        lua_rawget(l, LUA_REGISTRYINDEX);
                        as_array = lua_rawequal(l, -1, -2);
                    }
                    if as_array != 0 {
                        raw = 1;
                        lua_pop(l, 2);
                    } else {
                        raw = 0;
                        lua_pop(l, 2);
                        if luaL_getmetafield(l, -1, c"__len") != 0 {
                            lua_pushvalue(l, -2);
                            lua_call(l, 1, 1);
                            let _ = lua_tonumber(l, -1);
                            lua_pop(l, 1);
                            as_array = 1;
                        }
                    }
                }

                if as_array != 0 {
                    let len = lua_objlen(l, -1) as i32;
                    json_append_array(l, ctx, current_depth, len, raw);
                } else {
                    let len = lua_array_length(l, ctx);
                    if len > 0 || (len == 0 && !as_empty_dict) {
                        json_append_array(l, ctx, current_depth, len, raw);
                    } else {
                        if has_metatable {
                            lua_getmetatable(l, -1);
                            lua_pushlightuserdata(
                                l,
                                json_lightudata_mask(&raw const JSON_EMPTY_ARRAY).cast(),
                            );
                            lua_rawget(l, LUA_REGISTRYINDEX);
                            as_array = lua_rawequal(l, -1, -2);
                            lua_pop(l, 2);
                            if as_array != 0 {
                                let len = lua_objlen(l, -1) as i32;
                                json_append_array(l, ctx, current_depth, len, 1);
                                return 0;
                            }
                        }
                        json_append_object(l, ctx, current_depth);
                    }
                }
            }
            LUA_TNIL => {
                strbuf_append_mem(ctx.json, b"null");
            }
            LUA_TLIGHTUSERDATA => {
                if lua_touserdata(l, -1) == (&raw const JSON_ARRAY).cast_mut().cast() {
                    json_append_array(l, ctx, current_depth, 0, 1);
                }
            }
            LUA_TUSERDATA => {
                nlua_pushref(l, nlua_get_nil_ref(l));
                let is_nil = lua_rawequal(l, -2, -1) != 0;
                lua_pop(l, 1);
                if is_nil {
                    strbuf_append_mem(ctx.json, b"null");
                } else {
                    json_encode_exception(l, ctx, -1, c"type not supported");
                }
            }
            _ => {
                json_encode_exception(l, ctx, -1, c"type not supported");
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn json_encode(l: *mut lua_State) -> i32 {
    unsafe {
        let cfg = json_fetch_config(l);
        strbuf_reset(&mut cfg.encode_buf);
        let mut ctx = JsonEncode {
            options: JsonEncodeOptions {
                char2escape: CHAR2ESCAPE,
                indent: None,
                sort_keys: DEFAULT_ENCODE_SORT_KEYS,
                keybuf: Keybuf::new(),
            },
            json: &mut cfg.encode_buf,
        };

        match lua_gettop(l) {
            1 => {}
            2 => {
                luaL_checktype(l, 2, LUA_TTABLE);

                lua_getfield(l, 2, c"escape_slash");
                if !lua_isnil(l, -1) {
                    luaL_checktype(l, -1, LUA_TBOOLEAN);
                    let escape_slash = lua_toboolean(l, -1);
                    if escape_slash != 0 {
                        ctx.options.char2escape[b'/' as usize] = Some(c"\\/");
                    }
                }
                lua_pop(l, 1);

                lua_getfield(l, 2, c"indent");
                if !lua_isnil(l, -1) {
                    let indent = luaL_checkstring(l, -1);
                    ctx.options.indent = (!indent.is_empty()).then_some(indent);
                }
                lua_pop(l, 1);

                lua_getfield(l, 2, c"sort_keys");
                if !lua_isnil(l, -1) {
                    luaL_checktype(l, -1, LUA_TBOOLEAN);
                    let sort_keys = lua_toboolean(l, -1);
                    if sort_keys != 0 {
                        ctx.options.sort_keys = sort_keys;
                        strbuf_init(&mut ctx.options.keybuf.buf, 0);
                        ctx.options.keybuf.keys = Vec::with_capacity(KEYBUF_DEFAULT_CAPACITY);
                    }
                }
                lua_pop(l, 2);
            }
            _ => {
                return luaL_error(l, c"expected 1 or 2 arguments".as_ptr());
            }
        }

        json_append_data(l, &mut ctx, 0);
        let mut len: usize = 0;
        let json = strbuf_string(ctx.json, &mut len);

        lua_pushlstring(l, json.as_ptr().cast(), len);

        1
    }
}

fn hexdigit2int(hex: u8) -> i32 {
    if hex.is_ascii_digit() {
        return i32::from(hex - b'0');
    }
    let lower = hex | 0x20;
    if (b'a'..=b'f').contains(&lower) {
        return i32::from(10 + lower - b'a');
    }
    -1
}

fn decode_hex4(hex: &[u8]) -> i32 {
    let mut digit = [0_i32; 4];
    for (i, d) in digit.iter_mut().enumerate() {
        *d = hexdigit2int(hex.get(i).copied().unwrap_or(0));
        if *d < 0 {
            return -1;
        }
    }
    (digit[0] << 12) + (digit[1] << 8) + (digit[2] << 4) + digit[3]
}

fn codepoint_to_utf8(utf8: &mut [u8; 4], codepoint: i32) -> usize {
    if codepoint <= 0x7F {
        utf8[0] = codepoint as u8;
        return 1;
    }
    if codepoint <= 0x7FF {
        utf8[0] = ((codepoint >> 6) | 0xC0) as u8;
        utf8[1] = ((codepoint & 0x3F) | 0x80) as u8;
        return 2;
    }
    if codepoint <= 0xFFFF {
        utf8[0] = ((codepoint >> 12) | 0xE0) as u8;
        utf8[1] = (((codepoint >> 6) & 0x3F) | 0x80) as u8;
        utf8[2] = ((codepoint & 0x3F) | 0x80) as u8;
        return 3;
    }
    if codepoint <= 0x1FFFFF {
        utf8[0] = ((codepoint >> 18) | 0xF0) as u8;
        utf8[1] = (((codepoint >> 12) & 0x3F) | 0x80) as u8;
        utf8[2] = (((codepoint >> 6) & 0x3F) | 0x80) as u8;
        utf8[3] = ((codepoint & 0x3F) | 0x80) as u8;
        return 4;
    }
    0
}

fn json_append_unicode_escape(json: &mut JsonParse<'_>) -> i32 {
    let mut utf8 = [0u8; 4];
    let mut escape_len: usize = 6;

    let rem = &json.data[json.pos..];
    let mut codepoint = decode_hex4(rem.get(2..).unwrap_or(&[]));
    if codepoint < 0 {
        return -1;
    }

    if (codepoint & 0xF800) == 0xD800 {
        if (codepoint & 0x400) != 0 {
            return -1;
        }
        if !rem.get(escape_len..).unwrap_or(&[]).starts_with(b"\\u") {
            return -1;
        }

        let mut surrogate_low = decode_hex4(rem.get(2 + escape_len..).unwrap_or(&[]));
        if surrogate_low < 0 {
            return -1;
        }
        if (surrogate_low & 0xFC00) != 0xDC00 {
            return -1;
        }

        codepoint = (codepoint & 0x3FF) << 10;
        surrogate_low &= 0x3FF;
        codepoint = (codepoint | surrogate_low) + 0x10000;
        escape_len = 12;
    }

    let len = codepoint_to_utf8(&mut utf8, codepoint);
    if len == 0 {
        return -1;
    }

    strbuf_append_mem_unsafe(&mut json.tmp, &utf8[..len]);
    json.pos += escape_len;

    0
}

fn json_set_token_error(token: &mut JsonToken, json: &JsonParse<'_>, errtype: &'static CStr) {
    token.r#type = TError;
    token.index = json.pos;
    token.value = JsonTokenValue::Error(errtype);
}

fn json_next_string_token(json: &mut JsonParse<'_>, token: &mut JsonToken) {
    debug_assert_eq!(json.data.get(json.pos).copied(), Some(b'"'));

    json.pos += 1;
    strbuf_reset(&mut json.tmp);

    loop {
        let mut ch = json.data.get(json.pos).copied().unwrap_or(0);
        if ch == b'"' {
            break;
        }
        if ch == 0 {
            json_set_token_error(token, json, c"unexpected end of string");
            return;
        }

        if ch == b'\\' {
            ch = json.data.get(json.pos + 1).copied().unwrap_or(0);
            ch = ESCAPE2CHAR[ch as usize];
            if ch == b'u' {
                if json_append_unicode_escape(json) == 0 {
                    continue;
                }
                json_set_token_error(token, json, c"invalid unicode escape code");
                return;
            }
            if ch == 0 {
                json_set_token_error(token, json, c"invalid escape code");
                return;
            }
            json.pos += 1;
        }

        strbuf_append_char_unsafe(&mut json.tmp, ch);
        json.pos += 1;
    }
    json.pos += 1;

    strbuf_ensure_null(&mut json.tmp);

    token.r#type = TString;
    token.string_len = strbuf_length(&json.tmp);
    token.value = JsonTokenValue::None;
}

#[inline]
fn starts_with_ignore_ascii_case_3(p: &[u8], target: &[u8; 3]) -> bool {
    let b0 = p.first().copied().unwrap_or(0);
    if b0 == 0 || (b0 | 0x20) != target[0] {
        return false;
    }
    let b1 = p.get(1).copied().unwrap_or(0);
    if b1 == 0 || (b1 | 0x20) != target[1] {
        return false;
    }
    let b2 = p.get(2).copied().unwrap_or(0);
    b2 != 0 && (b2 | 0x20) == target[2]
}

fn json_is_invalid_number(json: &JsonParse<'_>) -> bool {
    let mut p = &json.data[json.pos..];
    if p.first() == Some(&b'+') {
        return true;
    }
    if p.first() == Some(&b'-') {
        p = &p[1..];
    }
    let b0 = p.first().copied().unwrap_or(0);
    if b0 == b'0' {
        let ch2 = p.get(1).copied().unwrap_or(0);
        if (ch2 | 0x20) == b'x' || ch2.is_ascii_digit() {
            return true;
        }
        return false;
    } else if b0 <= b'9' {
        return false;
    }

    starts_with_ignore_ascii_case_3(p, b"inf") || starts_with_ignore_ascii_case_3(p, b"nan")
}

fn json_next_number_token(json: &mut JsonParse<'_>, token: &mut JsonToken) {
    let rem = &json.data[json.pos..];
    let mut s = 0usize;
    while matches!(
        rem.get(s).copied().unwrap_or(0),
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
    ) {
        s += 1;
    }
    let neg = if rem.get(s) == Some(&b'-') {
        s += 1;
        true
    } else {
        if rem.get(s) == Some(&b'+') {
            s += 1;
        }
        false
    };
    let cutoff = if neg {
        (i64::MAX as u64) + 1
    } else {
        i64::MAX as u64
    };
    let mut acc: u64 = 0;
    let mut any = false;
    let mut overflow = false;
    while rem.get(s).is_some_and(u8::is_ascii_digit) {
        any = true;
        let digit = u64::from(rem[s] - b'0');
        if !overflow {
            if let Some(next) = acc.checked_mul(10).and_then(|v| v.checked_add(digit))
                && next <= cutoff
            {
                acc = next;
            } else {
                overflow = true;
            }
        }
        s += 1;
    }
    let mut end_off = if any { s } else { 0 };
    let tmpval: i64 = if overflow {
        if neg { i64::MIN } else { i64::MAX }
    } else if neg {
        acc.wrapping_neg() as i64
    } else {
        acc as i64
    };
    let end_ch = rem.get(end_off).copied().unwrap_or(0);
    if end_off == 0 || end_ch == b'.' || end_ch == b'e' || end_ch == b'E' || end_ch == b'x' {
        token.r#type = TNumber;
        let cstr = CStr::from_bytes_until_nul(rem).unwrap_or(c"");
        let (val, rest) = fpconv_strtod(cstr);
        end_off = cstr.to_bytes().len() - rest.to_bytes().len();
        token.value = JsonTokenValue::Number(val);
        if end_off == 0 {
            json_set_token_error(token, json, c"invalid number");
            return;
        }
    } else if tmpval > (isize::MAX as i64) || tmpval < (isize::MIN as i64) {
        token.r#type = TNumber;
        token.value = JsonTokenValue::Number(tmpval as f64);
    } else {
        token.r#type = TInteger;
        token.value = JsonTokenValue::Integer(tmpval as lua_Integer);
    }
    json.pos += end_off;
}

fn json_next_token(json: &mut JsonParse<'_>, token: &mut JsonToken) {
    let mut ch: usize;

    loop {
        loop {
            ch = usize::from(json.data.get(json.pos).copied().unwrap_or(0));
            token.r#type = CH2TOKEN[ch];
            if token.r#type != TWhitespace {
                break;
            }
            json.pos += 1;
        }

        if !json.options.skip_comments {
            break;
        }

        let c0 = json.data.get(json.pos).copied().unwrap_or(0);
        if c0 != b'/' {
            break;
        }
        let c1 = json.data.get(json.pos + 1).copied().unwrap_or(0);
        if c1 != b'/' && c1 != b'*' {
            break;
        }

        if c1 == b'/' {
            json.pos += 2;
            while let Some(&b) = json.data.get(json.pos)
                && b != 0
                && b != b'\n'
            {
                json.pos += 1;
            }
        } else {
            json.pos += 2;
            loop {
                if json.data.get(json.pos).copied().unwrap_or(0) == 0 {
                    json_set_token_error(token, json, c"unclosed multi-line comment");
                    return;
                }
                if json.data[json.pos..].starts_with(b"*/") {
                    json.pos += 2;
                    break;
                }
                json.pos += 1;
            }
        }
    }

    token.index = json.pos;

    if token.r#type == TError {
        json_set_token_error(token, json, c"invalid token");
        return;
    }

    if token.r#type == TEnd {
        return;
    }

    if token.r#type != TUnknown {
        json.pos += 1;
        return;
    }

    let ch_u8 = ch as u8;
    let ptr_bytes = &json.data[json.pos..];
    if ch_u8 == b'"' {
        json_next_string_token(json, token);
        return;
    } else if ch_u8 == b'-' || ch_u8.is_ascii_digit() {
        json_next_number_token(json, token);
        return;
    } else if ptr_bytes.starts_with(b"true") {
        token.r#type = TBoolean;
        token.value = JsonTokenValue::Boolean(1);
        json.pos += 4;
        return;
    } else if ptr_bytes.starts_with(b"false") {
        token.r#type = TBoolean;
        token.value = JsonTokenValue::Boolean(0);
        json.pos += 5;
        return;
    } else if ptr_bytes.starts_with(b"null") {
        token.r#type = TNull;
        json.pos += 4;
        return;
    } else if json_is_invalid_number(json) {
        json_next_number_token(json, token);
        return;
    }

    json_set_token_error(token, json, c"invalid token");
}

fn json_throw_parse_error(
    l: *mut lua_State,
    json: &mut JsonParse<'_>,
    exp: &std::ffi::CStr,
    token: &JsonToken,
) -> ! {
    unsafe {
        json.tmp = Strbuf::new();
        let found = if token.r#type == TError {
            token.value.as_error()
        } else {
            JSON_TOKEN_TYPE_NAME[token.r#type as usize]
        };

        luaL_error(
            l,
            c"Expected %s but found %s at character %d".as_ptr(),
            exp.as_ptr(),
            found.as_ptr(),
            (token.index + 1) as i32,
        );
        unreachable!()
    }
}

#[inline]
fn json_decode_ascend(json: &mut JsonParse<'_>) {
    json.current_depth -= 1;
}

fn json_decode_descend(l: *mut lua_State, json: &mut JsonParse<'_>, slots: i32) {
    unsafe {
        json.current_depth += 1;
        if json.current_depth <= DEFAULT_DECODE_MAX_DEPTH && lua_checkstack(l, slots) != 0 {
            return;
        }

        json.tmp = Strbuf::new();
        luaL_error(
            l,
            c"Found too many nested data structures (%d) at character %d".as_ptr(),
            json.current_depth,
            json.pos as i32,
        );
    }
}

fn json_parse_object_context(l: *mut lua_State, json: &mut JsonParse<'_>) {
    unsafe {
        let mut token = JsonToken::new();

        json_decode_descend(l, json, 3);
        lua_newtable(l);
        json_next_token(json, &mut token);

        if token.r#type == TObjEnd {
            nlua_pushref(l, nlua_get_empty_dict_ref(l));
            lua_setmetatable(l, -2);
            json_decode_ascend(json);
            return;
        }

        loop {
            if token.r#type != TString {
                json_throw_parse_error(l, json, c"object key string", &token);
            }

            lua_pushlstring(l, json.tmp.buf.as_ptr().cast(), token.string_len);

            json_next_token(json, &mut token);
            if token.r#type != TColon {
                json_throw_parse_error(l, json, c"colon", &token);
            }

            json_next_token(json, &mut token);
            let luanil_object = json.options.luanil_object;
            json_process_value(l, json, &token, luanil_object);

            lua_rawset(l, -3);

            json_next_token(json, &mut token);
            if token.r#type == TObjEnd {
                json_decode_ascend(json);
                return;
            }
            if token.r#type != TComma {
                json_throw_parse_error(l, json, c"comma or object end", &token);
            }

            json_next_token(json, &mut token);
        }
    }
}

fn json_parse_array_context(l: *mut lua_State, json: &mut JsonParse<'_>) {
    unsafe {
        let mut token = JsonToken::new();

        json_decode_descend(l, json, 2);
        lua_newtable(l);

        json_next_token(json, &mut token);

        if token.r#type == TArrEnd {
            json_decode_ascend(json);
            return;
        }

        let mut i: i32 = 1;
        loop {
            let luanil_array = json.options.luanil_array;
            json_process_value(l, json, &token, luanil_array);
            lua_rawseti(l, -2, i);

            json_next_token(json, &mut token);
            if token.r#type == TArrEnd {
                json_decode_ascend(json);
                return;
            }
            if token.r#type != TComma {
                json_throw_parse_error(l, json, c"comma or array end", &token);
            }

            json_next_token(json, &mut token);
            i += 1;
        }
    }
}

fn json_process_value(
    l: *mut lua_State,
    json: &mut JsonParse<'_>,
    token: &JsonToken,
    use_luanil: bool,
) {
    unsafe {
        match token.r#type {
            TString => {
                lua_pushlstring(l, json.tmp.buf.as_ptr().cast(), token.string_len);
            }
            TNumber => {
                lua_pushnumber(l, token.value.as_number());
            }
            TInteger => {
                lua_pushinteger(l, token.value.as_integer());
            }
            TBoolean => {
                lua_pushboolean(l, token.value.as_boolean());
            }
            TObjBegin => {
                json_parse_object_context(l, json);
            }
            TArrBegin => {
                json_parse_array_context(l, json);
            }
            TNull => {
                if use_luanil {
                    lua_pushnil(l);
                } else {
                    nlua_pushref(l, nlua_get_nil_ref(l));
                }
            }
            _ => {
                json_throw_parse_error(l, json, c"value", token);
            }
        }
    }
}

unsafe extern "C-unwind" fn json_decode(l: *mut lua_State) -> i32 {
    unsafe {
        let mut options = JsonOptions {
            luanil_object: false,
            luanil_array: false,
            skip_comments: false,
        };

        match lua_gettop(l) {
            1 => {}
            2 => {
                luaL_checktype(l, 2, LUA_TTABLE);

                lua_getfield(l, 2, c"skip_comments");
                options.skip_comments = lua_toboolean(l, -1) != 0;
                lua_pop(l, 1);

                lua_getfield(l, 2, c"luanil");
                if lua_isnil(l, -1) {
                    lua_pop(l, 1);
                } else {
                    luaL_checktype(l, -1, LUA_TTABLE);

                    lua_getfield(l, -1, c"object");
                    options.luanil_object = lua_toboolean(l, -1) != 0;
                    lua_pop(l, 1);

                    lua_getfield(l, -1, c"array");
                    options.luanil_array = lua_toboolean(l, -1) != 0;
                    lua_pop(l, 2);
                }
            }
            _ => {
                return luaL_error(l, c"expected 1 or 2 arguments".as_ptr());
            }
        }

        let mut json_len: usize = 0;
        let data_ptr = luaL_checklstring(l, 1, &mut json_len);
        let data = std::slice::from_raw_parts(data_ptr.cast::<u8>(), json_len + 1);

        if json_len >= 2 && (data[0] == 0 || data[1] == 0) {
            luaL_error(l, c"JSON parser does not support UTF-16 or UTF-32".as_ptr());
        }

        let mut json = JsonParse {
            data,
            pos: 0,
            tmp: Strbuf::new(),
            options,
            current_depth: 0,
        };
        strbuf_init(&mut json.tmp, json_len);

        let mut token = JsonToken::new();
        json_next_token(&mut json, &mut token);
        let luanil_object = json.options.luanil_object;
        json_process_value(l, &mut json, &token, luanil_object);

        json_next_token(&mut json, &mut token);
        if token.r#type != TEnd {
            json_throw_parse_error(l, &mut json, c"the end", &token);
        }

        1
    }
}

fn compat_lua_l_setfuncs(l: *mut lua_State, reg: &[luaL_Reg], nup: i32) {
    unsafe {
        luaL_checkstack(l, nup, c"too many upvalues");
        for entry in reg {
            if entry.name.is_null() {
                break;
            }
            for _ in 0..nup {
                lua_pushvalue(l, -nup);
            }
            lua_pushcclosure(l, entry.func, nup);
            lua_setfield(l, -(nup + 2), CStr::from_ptr(entry.name));
        }
        lua_pop(l, nup);
    }
}

pub unsafe extern "C-unwind" fn lua_cjson_new(l: *mut lua_State) -> i32 {
    unsafe {
        let reg = [
            luaL_Reg {
                name: c"encode".as_ptr(),
                func: Some(json_encode),
            },
            luaL_Reg {
                name: c"decode".as_ptr(),
                func: Some(json_decode),
            },
            luaL_Reg {
                name: c"new".as_ptr(),
                func: Some(lua_cjson_new),
            },
            luaL_Reg {
                name: ptr::null(),
                func: None,
            },
        ];

        {
            lua_pushlightuserdata(l, json_lightudata_mask(&raw const JSON_EMPTY_ARRAY).cast());
            lua_rawget(l, LUA_REGISTRYINDEX);
            if lua_isnil(l, -1) {
                lua_pop(l, 1);

                lua_pushlightuserdata(l, json_lightudata_mask(&raw const JSON_EMPTY_ARRAY).cast());
                lua_newtable(l);
                lua_rawset(l, LUA_REGISTRYINDEX);

                lua_pushlightuserdata(l, json_lightudata_mask(&raw const JSON_ARRAY).cast());
                lua_newtable(l);
                lua_rawset(l, LUA_REGISTRYINDEX);
            }

            lua_newtable(l);

            json_create_config(l);
            compat_lua_l_setfuncs(l, &reg, 1);

            lua_pushlstring(l, CJSON_MODNAME.as_ptr().cast(), CJSON_MODNAME.len());
            lua_setfield(l, -2, c"_NAME");
            lua_pushlstring(l, CJSON_VERSION.as_ptr().cast(), CJSON_VERSION.len());
            lua_setfield(l, -2, c"_VERSION");
        }

        1
    }
}

pub fn luaopen_cjson(l: *mut lua_State) -> i32 {
    unsafe { lua_cjson_new(l) }
}
