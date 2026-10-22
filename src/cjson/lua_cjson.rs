//! JSON support for Lua (`src/cjson/lua_cjson.c`).

use std::ffi::{CStr, c_char, c_double, c_int, c_longlong, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::cjson::fpconv::{FPCONV_G_FMT_BUFSIZE, fpconv_g_fmt, fpconv_init, fpconv_strtod};
use crate::cjson::strbuf::{
    strbuf_append_char, strbuf_append_char_unsafe, strbuf_append_mem, strbuf_append_mem_unsafe,
    strbuf_append_string, strbuf_empty_ptr, strbuf_ensure_empty_length, strbuf_ensure_null,
    strbuf_extend_length, strbuf_free, strbuf_init, strbuf_length, strbuf_new, strbuf_reset,
    strbuf_set_length, strbuf_string, strbuf_t,
};
use crate::lua::converter::{
    LUA_ERRRUN, LUA_REGISTRYINDEX, LUA_TBOOLEAN, LUA_TLIGHTUSERDATA, LUA_TNIL, LUA_TNUMBER,
    LUA_TSTRING, LUA_TTABLE, LUA_TUSERDATA, lua_Integer, lua_Number, lua_State, lua_call,
    lua_checkstack, lua_getfield, lua_getmetatable, lua_gettable, lua_gettop, lua_insert,
    lua_isnil, lua_newtable, lua_newuserdata, lua_next, lua_objlen, lua_pcall, lua_pop,
    lua_pushboolean, lua_pushcclosure, lua_pushcfunction, lua_pushinteger, lua_pushlightuserdata,
    lua_pushlstring, lua_pushnil, lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawequal,
    lua_rawget, lua_rawgeti, lua_rawset, lua_rawseti, lua_setfield, lua_setmetatable,
    lua_toboolean, lua_tointeger, lua_tolstring, lua_tonumber, lua_tostring, lua_touserdata,
    lua_type, lua_typename, lua_upvalueindex, luaL_Reg, luaL_argerror, luaL_checklstring,
    luaL_checkstack, luaL_checkstring, luaL_checktype, luaL_error, luaL_getmetafield,
};
use crate::lua::executor::{nlua_get_empty_dict_ref, nlua_get_nil_ref, nlua_pushref};

const CJSON_MODNAME: &[u8] = b"cjson";
const CJSON_VERSION: &[u8] = b"2.1.0.11";

const DEFAULT_SPARSE_CONVERT: c_int = 0;
const DEFAULT_SPARSE_RATIO: c_int = 2;
const DEFAULT_SPARSE_SAFE: c_int = 10;
const DEFAULT_ENCODE_MAX_DEPTH: c_int = 1000;
const DEFAULT_DECODE_MAX_DEPTH: c_int = 1000;
const DEFAULT_ENCODE_INVALID_NUMBERS: c_int = 0;
const DEFAULT_DECODE_INVALID_NUMBERS: c_int = 1;
const DEFAULT_ENCODE_KEEP_BUFFER: c_int = 1;
const DEFAULT_ENCODE_NUMBER_PRECISION: c_int = 16;
const DEFAULT_ENCODE_EMPTY_TABLE_AS_OBJECT: c_int = 0;
const DEFAULT_DECODE_ARRAY_WITH_ARRAY_MT: c_int = 0;
const DEFAULT_DECODE_SKIP_COMMENTS: c_int = 0;
const DEFAULT_ENCODE_ESCAPE_FORWARD_SLASH: c_int = 1;
const DEFAULT_ENCODE_SKIP_UNSUPPORTED_VALUE_TYPES: c_int = 0;
const DEFAULT_ENCODE_INDENT: *const c_char = ptr::null();
const DEFAULT_ENCODE_SORT_KEYS: c_int = 0;

const KEYBUF_DEFAULT_CAPACITY: usize = 32;

static mut JSON_EMPTY_ARRAY: *const *const c_char = ptr::null();
static mut JSON_ARRAY: *const *const c_char = ptr::null();

#[inline]
fn json_lightudata_mask(ludata: *const c_void) -> *mut c_void {
    #[cfg(target_pointer_width = "64")]
    {
        ((ludata as usize) & ((1usize << 47) - 1)) as *mut c_void
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        ludata.cast_mut()
    }
}

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
enum json_token_type_t {
    T_OBJ_BEGIN = 0,
    T_OBJ_END = 1,
    T_ARR_BEGIN = 2,
    T_ARR_END = 3,
    T_STRING = 4,
    T_NUMBER = 5,
    T_INTEGER = 6,
    T_BOOLEAN = 7,
    T_NULL = 8,
    T_COLON = 9,
    T_COMMA = 10,
    T_END = 11,
    T_WHITESPACE = 12,
    T_ERROR = 13,
    T_UNKNOWN = 14,
}

use json_token_type_t::*;

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

#[repr(C)]
#[derive(Copy, Clone)]
union KeyEntryRaw {
    number: lua_Number,
    string: *const c_char,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct key_entry_t {
    buf: *mut strbuf_t,
    offset: usize,
    length: usize,
    raw_typ: c_int,
    raw: KeyEntryRaw,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct keybuf_t {
    buf: strbuf_t,
    keys: *mut key_entry_t,
    size: usize,
    capacity: usize,
}

#[repr(C)]
struct json_config_t {
    ch2token: [json_token_type_t; 256],
    escape2char: [c_char; 256],
    encode_buf: strbuf_t,
    encode_keybuf: keybuf_t,
    encode_sparse_convert: c_int,
    encode_sparse_ratio: c_int,
    encode_sparse_safe: c_int,
    encode_max_depth: c_int,
    encode_invalid_numbers: c_int,
    encode_number_precision: c_int,
    encode_keep_buffer: c_int,
    encode_empty_table_as_object: c_int,
    encode_escape_forward_slash: c_int,
    encode_indent: *const c_char,
    encode_sort_keys: c_int,
    decode_invalid_numbers: c_int,
    decode_max_depth: c_int,
    decode_array_with_array_mt: c_int,
    decode_skip_comments: c_int,
    encode_skip_unsupported_value_types: c_int,
}

#[repr(C)]
struct json_encode_options_t {
    char2escape: *const *const c_char,
    indent: *const c_char,
    sort_keys: c_int,
    keybuf: keybuf_t,
}

#[repr(C)]
struct json_encode_t {
    cfg: *mut json_config_t,
    options: *mut json_encode_options_t,
    json: *mut strbuf_t,
}

#[repr(C)]
struct json_options_t {
    luanil_object: bool,
    luanil_array: bool,
    skip_comments: bool,
}

#[repr(C)]
struct json_parse_t {
    data: *const c_char,
    ptr: *const c_char,
    tmp: *mut strbuf_t,
    cfg: *mut json_config_t,
    options: *mut json_options_t,
    current_depth: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
union JsonTokenValue {
    string: *const c_char,
    number: c_double,
    integer: lua_Integer,
    boolean: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct json_token_t {
    r#type: json_token_type_t,
    index: usize,
    value: JsonTokenValue,
    string_len: usize,
}

const fn init_char2escape() -> [*const c_char; 256] {
    let mut table = [ptr::null(); 256];
    table[0x00] = c"\\u0000".as_ptr();
    table[0x01] = c"\\u0001".as_ptr();
    table[0x02] = c"\\u0002".as_ptr();
    table[0x03] = c"\\u0003".as_ptr();
    table[0x04] = c"\\u0004".as_ptr();
    table[0x05] = c"\\u0005".as_ptr();
    table[0x06] = c"\\u0006".as_ptr();
    table[0x07] = c"\\u0007".as_ptr();
    table[0x08] = c"\\b".as_ptr();
    table[0x09] = c"\\t".as_ptr();
    table[0x0a] = c"\\n".as_ptr();
    table[0x0b] = c"\\u000b".as_ptr();
    table[0x0c] = c"\\f".as_ptr();
    table[0x0d] = c"\\r".as_ptr();
    table[0x0e] = c"\\u000e".as_ptr();
    table[0x0f] = c"\\u000f".as_ptr();
    table[0x10] = c"\\u0010".as_ptr();
    table[0x11] = c"\\u0011".as_ptr();
    table[0x12] = c"\\u0012".as_ptr();
    table[0x13] = c"\\u0013".as_ptr();
    table[0x14] = c"\\u0014".as_ptr();
    table[0x15] = c"\\u0015".as_ptr();
    table[0x16] = c"\\u0016".as_ptr();
    table[0x17] = c"\\u0017".as_ptr();
    table[0x18] = c"\\u0018".as_ptr();
    table[0x19] = c"\\u0019".as_ptr();
    table[0x1a] = c"\\u001a".as_ptr();
    table[0x1b] = c"\\u001b".as_ptr();
    table[0x1c] = c"\\u001c".as_ptr();
    table[0x1d] = c"\\u001d".as_ptr();
    table[0x1e] = c"\\u001e".as_ptr();
    table[0x1f] = c"\\u001f".as_ptr();
    table[b'"' as usize] = c"\\\"".as_ptr();
    table[b'\\' as usize] = c"\\\\".as_ptr();
    table[0x7f] = c"\\u007f".as_ptr();
    table
}

const CHAR2ESCAPE: [*const c_char; 256] = init_char2escape();

use crate::memory::{try_malloc, xfree, xrealloc};
use crate::strings::{memcmp, sort_slice_by, strncmp, strtoll};
use std::process::abort;

unsafe fn json_fetch_config(l: *mut lua_State) -> *mut json_config_t {
    unsafe {
        let cfg = lua_touserdata(l, lua_upvalueindex(1)).cast::<json_config_t>();
        if cfg.is_null() {
            luaL_error(l, c"BUG: Unable to fetch CJSON configuration".as_ptr());
        }
        cfg
    }
}

unsafe extern "C-unwind" fn json_destroy_config(l: *mut lua_State) -> c_int {
    unsafe {
        let cfg = lua_touserdata(l, 1).cast::<json_config_t>();
        if !cfg.is_null() {
            strbuf_free(&mut (*cfg).encode_buf);
        }
    }
    0
}

unsafe fn json_create_config(l: *mut lua_State) {
    unsafe {
        let cfg = lua_newuserdata(l, size_of::<json_config_t>()).cast::<json_config_t>();
        if cfg.is_null() {
            abort();
        }
        ptr::write_bytes(cfg, 0, 1);

        lua_newtable(l);
        lua_pushcfunction(l, Some(json_destroy_config));
        lua_setfield(l, -2, c"__gc".as_ptr());
        lua_setmetatable(l, -2);

        (*cfg).encode_sparse_convert = DEFAULT_SPARSE_CONVERT;
        (*cfg).encode_sparse_ratio = DEFAULT_SPARSE_RATIO;
        (*cfg).encode_sparse_safe = DEFAULT_SPARSE_SAFE;
        (*cfg).encode_max_depth = DEFAULT_ENCODE_MAX_DEPTH;
        (*cfg).decode_max_depth = DEFAULT_DECODE_MAX_DEPTH;
        (*cfg).encode_invalid_numbers = DEFAULT_ENCODE_INVALID_NUMBERS;
        (*cfg).decode_invalid_numbers = DEFAULT_DECODE_INVALID_NUMBERS;
        (*cfg).encode_keep_buffer = DEFAULT_ENCODE_KEEP_BUFFER;
        (*cfg).encode_number_precision = DEFAULT_ENCODE_NUMBER_PRECISION;
        (*cfg).encode_empty_table_as_object = DEFAULT_ENCODE_EMPTY_TABLE_AS_OBJECT;
        (*cfg).decode_array_with_array_mt = DEFAULT_DECODE_ARRAY_WITH_ARRAY_MT;
        (*cfg).decode_skip_comments = DEFAULT_DECODE_SKIP_COMMENTS;
        (*cfg).encode_escape_forward_slash = DEFAULT_ENCODE_ESCAPE_FORWARD_SLASH;
        (*cfg).encode_skip_unsupported_value_types = DEFAULT_ENCODE_SKIP_UNSUPPORTED_VALUE_TYPES;
        (*cfg).encode_indent = DEFAULT_ENCODE_INDENT;
        (*cfg).encode_sort_keys = DEFAULT_ENCODE_SORT_KEYS;

        if DEFAULT_ENCODE_KEEP_BUFFER > 0 {
            strbuf_init(&mut (*cfg).encode_buf, 0);
        }

        for i in 0..256 {
            (*cfg).ch2token[i] = T_ERROR;
        }

        (*cfg).ch2token[b'{' as usize] = T_OBJ_BEGIN;
        (*cfg).ch2token[b'}' as usize] = T_OBJ_END;
        (*cfg).ch2token[b'[' as usize] = T_ARR_BEGIN;
        (*cfg).ch2token[b']' as usize] = T_ARR_END;
        (*cfg).ch2token[b',' as usize] = T_COMMA;
        (*cfg).ch2token[b':' as usize] = T_COLON;
        (*cfg).ch2token[b'\0' as usize] = T_END;
        (*cfg).ch2token[b' ' as usize] = T_WHITESPACE;
        (*cfg).ch2token[b'\t' as usize] = T_WHITESPACE;
        (*cfg).ch2token[b'\n' as usize] = T_WHITESPACE;
        (*cfg).ch2token[b'\r' as usize] = T_WHITESPACE;

        (*cfg).ch2token[b'f' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'i' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'I' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'n' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'N' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b't' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'"' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'+' as usize] = T_UNKNOWN;
        (*cfg).ch2token[b'-' as usize] = T_UNKNOWN;
        for i in 0..10usize {
            (*cfg).ch2token[(b'0' as usize) + i] = T_UNKNOWN;
        }

        for i in 0..256 {
            (*cfg).escape2char[i] = 0;
        }
        (*cfg).escape2char[b'"' as usize] = b'"' as c_char;
        (*cfg).escape2char[b'\\' as usize] = b'\\' as c_char;
        (*cfg).escape2char[b'/' as usize] = b'/' as c_char;
        (*cfg).escape2char[b'b' as usize] = 0x08 as c_char;
        (*cfg).escape2char[b't' as usize] = b'\t' as c_char;
        (*cfg).escape2char[b'n' as usize] = b'\n' as c_char;
        (*cfg).escape2char[b'f' as usize] = 0x0c as c_char;
        (*cfg).escape2char[b'r' as usize] = b'\r' as c_char;
        (*cfg).escape2char[b'u' as usize] = b'u' as c_char;
    }
}

unsafe fn json_encode_exception(
    l: *mut lua_State,
    ctx: *mut json_encode_t,
    lindex: c_int,
    reason: *const c_char,
) -> ! {
    unsafe {
        if (*(*ctx).cfg).encode_keep_buffer == 0 {
            strbuf_free((*ctx).json);
        }

        if (*(*ctx).options).sort_keys != 0 {
            strbuf_free(&mut (*(*ctx).options).keybuf.buf);
            xfree((*(*ctx).options).keybuf.keys.cast::<c_void>());
        }

        luaL_error(
            l,
            c"Cannot serialise %s: %s".as_ptr(),
            lua_typename(l, lua_type(l, lindex)),
            reason,
        );
        unreachable!()
    }
}

unsafe fn json_append_string_contents(
    l: *mut lua_State,
    ctx: *mut json_encode_t,
    lindex: c_int,
    use_keybuf: bool,
) {
    unsafe {
        let json = if use_keybuf {
            &raw mut (*(*ctx).options).keybuf.buf
        } else {
            (*ctx).json
        };

        let mut len: usize = 0;
        let str_ptr = lua_tolstring(l, lindex, &mut len);

        if len >= usize::MAX / 6 {
            abort();
        }
        strbuf_ensure_empty_length(json, len * 6);

        let escape_table = (*(*ctx).options).char2escape;
        for i in 0..len {
            let ch = *str_ptr.add(i);
            let escstr = *escape_table.add((ch as u8) as usize);
            if !escstr.is_null() {
                strbuf_append_string(json, escstr);
            } else {
                strbuf_append_char_unsafe(json, ch);
            }
        }
    }
}

unsafe fn json_append_string(l: *mut lua_State, ctx: *mut json_encode_t, lindex: c_int) {
    unsafe {
        strbuf_append_char((*ctx).json, b'"' as c_char);
        json_append_string_contents(l, ctx, lindex, false);
        strbuf_append_char((*ctx).json, b'"' as c_char);
    }
}

unsafe fn lua_array_length(l: *mut lua_State, ctx: *mut json_encode_t) -> c_int {
    unsafe {
        let cfg = (*ctx).cfg;
        let mut max: c_int = 0;
        let mut items: c_int = 0;

        lua_pushnil(l);
        while lua_next(l, -2) != 0 {
            if lua_type(l, -2) == LUA_TNUMBER {
                let k = lua_tonumber(l, -2);
                if k != 0.0 && k.floor() == k && k >= 1.0 {
                    if k > f64::from(max) {
                        max = k as c_int;
                    }
                    items += 1;
                    lua_pop(l, 1);
                    continue;
                }
            }

            lua_pop(l, 2);
            return -1;
        }

        if (*cfg).encode_sparse_ratio > 0
            && max > items * (*cfg).encode_sparse_ratio
            && max > (*cfg).encode_sparse_safe
        {
            if (*cfg).encode_sparse_convert == 0 {
                json_encode_exception(l, ctx, -1, c"excessively sparse array".as_ptr());
            }
            return -1;
        }

        max
    }
}

unsafe fn json_check_encode_depth(
    l: *mut lua_State,
    cfg: *mut json_config_t,
    current_depth: c_int,
    json: *mut strbuf_t,
) {
    unsafe {
        if current_depth <= (*cfg).encode_max_depth && lua_checkstack(l, 3) != 0 {
            return;
        }

        if (*cfg).encode_keep_buffer == 0 {
            strbuf_free(json);
        }

        luaL_error(
            l,
            c"Cannot serialise, excessive nesting (%d)".as_ptr(),
            current_depth,
        );
    }
}

unsafe fn json_append_newline_and_indent(
    json: *mut strbuf_t,
    ctx: *mut json_encode_t,
    depth: c_int,
) {
    unsafe {
        strbuf_append_char(json, b'\n' as c_char);
        for _ in 0..depth {
            strbuf_append_string(json, (*(*ctx).options).indent);
        }
    }
}

unsafe fn json_append_array(
    l: *mut lua_State,
    ctx: *mut json_encode_t,
    current_depth: c_int,
    array_length: c_int,
    raw: c_int,
) {
    unsafe {
        let json = (*ctx).json;
        let mut has_items = false;

        strbuf_append_char(json, b'[' as c_char);

        let mut comma: c_int = 0;
        for i in 1..=array_length {
            has_items = true;

            let json_pos = strbuf_length(json) as c_int;
            if comma > 0 {
                strbuf_append_char(json, b',' as c_char);
            }
            comma += 1;

            if !(*(*ctx).options).indent.is_null() {
                json_append_newline_and_indent(json, ctx, current_depth);
            }

            if raw != 0 {
                lua_rawgeti(l, -1, i);
            } else {
                lua_pushinteger(l, i as lua_Integer);
                lua_gettable(l, -2);
            }

            let err = json_append_data(l, ctx, current_depth);
            if err != 0 {
                strbuf_set_length(json, json_pos);
                if comma == 1 {
                    comma = 0;
                }
            }
            lua_pop(l, 1);
        }

        if has_items && !(*(*ctx).options).indent.is_null() {
            json_append_newline_and_indent(json, ctx, current_depth - 1);
        }

        strbuf_append_char(json, b']' as c_char);
    }
}

unsafe fn json_append_number(
    l: *mut lua_State,
    ctx: *mut json_encode_t,
    lindex: c_int,
    use_keybuf: bool,
) {
    unsafe {
        let num = lua_tonumber(l, lindex);
        let cfg = (*ctx).cfg;
        let json = if use_keybuf {
            &raw mut (*(*ctx).options).keybuf.buf
        } else {
            (*ctx).json
        };

        if (*cfg).encode_invalid_numbers == 0 {
            if num.is_infinite() || num.is_nan() {
                json_encode_exception(l, ctx, lindex, c"must not be NaN or Infinity".as_ptr());
            }
        } else if (*cfg).encode_invalid_numbers == 1 {
            if num.is_nan() {
                strbuf_append_mem(json, c"NaN".as_ptr(), 3);
                return;
            }
            if num.is_infinite() {
                if num < 0.0 {
                    strbuf_append_mem(json, c"-Infinity".as_ptr(), 9);
                } else {
                    strbuf_append_mem(json, c"Infinity".as_ptr(), 8);
                }
                return;
            }
        } else if num.is_infinite() || num.is_nan() {
            strbuf_append_mem(json, c"null".as_ptr(), 4);
            return;
        }

        strbuf_ensure_empty_length(json, FPCONV_G_FMT_BUFSIZE);
        let len = fpconv_g_fmt(strbuf_empty_ptr(json), num, (*cfg).encode_number_precision);
        strbuf_extend_length(json, len as usize);
    }
}

unsafe fn cmp_key_entries(ka: &key_entry_t, kb: &key_entry_t) -> c_int {
    unsafe {
        let min_len = ka.length.min(kb.length);
        let res = memcmp(
            (*ka.buf).buf.add(ka.offset).cast::<c_void>(),
            (*kb.buf).buf.add(kb.offset).cast::<c_void>(),
            min_len,
        );
        if res == 0 {
            (ka.length.wrapping_sub(kb.length)) as c_int
        } else {
            res
        }
    }
}

unsafe fn json_append_object(l: *mut lua_State, ctx: *mut json_encode_t, current_depth: c_int) {
    unsafe {
        let json = (*ctx).json;
        let mut has_items = false;

        strbuf_append_char(json, b'{' as c_char);

        let mut comma: c_int = 0;
        lua_pushnil(l);
        if (*(*ctx).options).sort_keys != 0 {
            let keybuf = &raw mut (*(*ctx).options).keybuf;
            let init_keybuf_size = (*keybuf).size;
            let init_keybuf_length = strbuf_length(&raw mut (*keybuf).buf);

            while lua_next(l, -2) != 0 {
                has_items = true;
                if (*keybuf).size == (*keybuf).capacity {
                    (*keybuf).capacity *= 2;
                    (*keybuf).keys = xrealloc(
                        (*keybuf).keys.cast::<c_void>(),
                        (*keybuf).capacity * size_of::<key_entry_t>(),
                    )
                    .cast::<key_entry_t>();
                }

                let keytype = lua_type(l, -2);
                let mut key_entry = key_entry_t {
                    buf: &raw mut (*keybuf).buf,
                    offset: strbuf_length(&raw mut (*keybuf).buf),
                    length: 0,
                    raw_typ: keytype,
                    raw: KeyEntryRaw { number: 0.0 },
                };
                if keytype == LUA_TSTRING {
                    json_append_string_contents(l, ctx, -2, true);
                    key_entry.raw.string = lua_tostring(l, -2);
                } else if keytype == LUA_TNUMBER {
                    json_append_number(l, ctx, -2, true);
                    key_entry.raw.number = lua_tointeger(l, -2) as lua_Number;
                } else {
                    json_encode_exception(
                        l,
                        ctx,
                        -2,
                        c"table key must be number or string".as_ptr(),
                    );
                }
                key_entry.length = strbuf_length(&raw mut (*keybuf).buf) - key_entry.offset;
                *(*keybuf).keys.add((*keybuf).size) = key_entry;
                (*keybuf).size += 1;
                lua_pop(l, 1);
            }

            let keys_count = (*keybuf).size - init_keybuf_size;
            if keys_count > 0 {
                let slice = std::slice::from_raw_parts_mut(
                    (*keybuf).keys.add(init_keybuf_size),
                    keys_count,
                );
                sort_slice_by(slice, |a, b| cmp_key_entries(a, b));
            }

            for i in init_keybuf_size..(init_keybuf_size + keys_count) {
                let current_key = (*keybuf).keys.add(i);
                let json_pos = strbuf_length(json) as c_int;
                if comma > 0 {
                    strbuf_append_char(json, b',' as c_char);
                }
                comma += 1;

                if !(*(*ctx).options).indent.is_null() {
                    json_append_newline_and_indent(json, ctx, current_depth);
                }

                strbuf_ensure_empty_length(json, (*current_key).length + 3);
                strbuf_append_char_unsafe(json, b'"' as c_char);
                strbuf_append_mem_unsafe(
                    json,
                    (*keybuf).buf.buf.add((*current_key).offset),
                    (*current_key).length,
                );
                strbuf_append_mem_unsafe(json, c"\":".as_ptr(), 2);

                if !(*(*ctx).options).indent.is_null() {
                    strbuf_append_char(json, b' ' as c_char);
                }

                if (*current_key).raw_typ == LUA_TSTRING {
                    lua_pushstring(l, (*current_key).raw.string);
                } else {
                    lua_pushnumber(l, (*current_key).raw.number);
                }

                lua_gettable(l, -2);
                let err = json_append_data(l, ctx, current_depth);
                if err != 0 {
                    strbuf_set_length(json, json_pos);
                    if comma == 1 {
                        comma = 0;
                    }
                }
                lua_pop(l, 1);
            }
            strbuf_set_length(&raw mut (*keybuf).buf, init_keybuf_length as c_int);
            (*keybuf).size = init_keybuf_size;
        } else {
            while lua_next(l, -2) != 0 {
                has_items = true;

                let json_pos = strbuf_length(json) as c_int;
                if comma > 0 {
                    strbuf_append_char(json, b',' as c_char);
                }
                comma += 1;

                if !(*(*ctx).options).indent.is_null() {
                    json_append_newline_and_indent(json, ctx, current_depth);
                }

                let keytype = lua_type(l, -2);
                if keytype == LUA_TNUMBER {
                    strbuf_append_char(json, b'"' as c_char);
                    json_append_number(l, ctx, -2, false);
                    strbuf_append_mem(json, c"\":".as_ptr(), 2);
                } else if keytype == LUA_TSTRING {
                    json_append_string(l, ctx, -2);
                    strbuf_append_char(json, b':' as c_char);
                } else {
                    json_encode_exception(
                        l,
                        ctx,
                        -2,
                        c"table key must be a number or string".as_ptr(),
                    );
                }
                if !(*(*ctx).options).indent.is_null() {
                    strbuf_append_char(json, b' ' as c_char);
                }

                let err = json_append_data(l, ctx, current_depth);
                if err != 0 {
                    strbuf_set_length(json, json_pos);
                    if comma == 1 {
                        comma = 0;
                    }
                }

                lua_pop(l, 1);
            }
        }

        if has_items && !(*(*ctx).options).indent.is_null() {
            json_append_newline_and_indent(json, ctx, current_depth - 1);
        }

        strbuf_append_char(json, b'}' as c_char);
    }
}

unsafe fn json_append_data(
    l: *mut lua_State,
    ctx: *mut json_encode_t,
    mut current_depth: c_int,
) -> c_int {
    unsafe {
        let cfg = (*ctx).cfg;
        let json = (*ctx).json;

        match lua_type(l, -1) {
            LUA_TSTRING => {
                json_append_string(l, ctx, -1);
            }
            LUA_TNUMBER => {
                json_append_number(l, ctx, -1, false);
            }
            LUA_TBOOLEAN => {
                if lua_toboolean(l, -1) != 0 {
                    strbuf_append_mem(json, c"true".as_ptr(), 4);
                } else {
                    strbuf_append_mem(json, c"false".as_ptr(), 5);
                }
            }
            LUA_TTABLE => {
                current_depth += 1;
                json_check_encode_depth(l, cfg, current_depth, json);

                let mut as_array: c_int = 0;
                let mut as_empty_dict = false;
                let mut raw: c_int = 1;
                let has_metatable = lua_getmetatable(l, -1) != 0;

                if has_metatable {
                    nlua_pushref(l, nlua_get_empty_dict_ref(l));
                    if lua_rawequal(l, -2, -1) != 0 {
                        as_empty_dict = true;
                    } else {
                        lua_pop(l, 1);
                        lua_pushlightuserdata(
                            l,
                            json_lightudata_mask((&raw const JSON_ARRAY).cast::<c_void>()),
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
                        if luaL_getmetafield(l, -1, c"__len".as_ptr()) != 0 {
                            lua_pushvalue(l, -2);
                            lua_call(l, 1, 1);
                            let _ = lua_tonumber(l, -1);
                            lua_pop(l, 1);
                            as_array = 1;
                        }
                    }
                }

                if as_array != 0 {
                    let len = lua_objlen(l, -1) as c_int;
                    json_append_array(l, ctx, current_depth, len, raw);
                } else {
                    let len = lua_array_length(l, ctx);
                    if len > 0
                        || (len == 0 && (*cfg).encode_empty_table_as_object == 0 && !as_empty_dict)
                    {
                        json_append_array(l, ctx, current_depth, len, raw);
                    } else {
                        if has_metatable {
                            lua_getmetatable(l, -1);
                            lua_pushlightuserdata(
                                l,
                                json_lightudata_mask(
                                    (&raw const JSON_EMPTY_ARRAY).cast::<c_void>(),
                                ),
                            );
                            lua_rawget(l, LUA_REGISTRYINDEX);
                            as_array = lua_rawequal(l, -1, -2);
                            lua_pop(l, 2);
                            if as_array != 0 {
                                let len = lua_objlen(l, -1) as c_int;
                                json_append_array(l, ctx, current_depth, len, 1);
                                return 0;
                            }
                        }
                        json_append_object(l, ctx, current_depth);
                    }
                }
            }
            LUA_TNIL => {
                strbuf_append_mem(json, c"null".as_ptr(), 4);
            }
            LUA_TLIGHTUSERDATA => {
                if lua_touserdata(l, -1) == (&raw const JSON_ARRAY).cast::<c_void>().cast_mut() {
                    json_append_array(l, ctx, current_depth, 0, 1);
                }
            }
            LUA_TUSERDATA => {
                nlua_pushref(l, nlua_get_nil_ref(l));
                let is_nil = lua_rawequal(l, -2, -1) != 0;
                lua_pop(l, 1);
                if is_nil {
                    strbuf_append_mem(json, c"null".as_ptr(), 4);
                } else if (*cfg).encode_skip_unsupported_value_types != 0 {
                    return 1;
                } else {
                    json_encode_exception(l, ctx, -1, c"type not supported".as_ptr());
                }
            }
            _ => {
                if (*cfg).encode_skip_unsupported_value_types != 0 {
                    return 1;
                } else {
                    json_encode_exception(l, ctx, -1, c"type not supported".as_ptr());
                }
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn json_encode(l: *mut lua_State) -> c_int {
    unsafe {
        let cfg = json_fetch_config(l);
        let mut custom_char2escape = CHAR2ESCAPE;
        let mut options = json_encode_options_t {
            char2escape: custom_char2escape.as_ptr(),
            indent: DEFAULT_ENCODE_INDENT,
            sort_keys: DEFAULT_ENCODE_SORT_KEYS,
            keybuf: zeroed(),
        };
        let mut ctx = json_encode_t {
            cfg,
            options: &mut options,
            json: ptr::null_mut(),
        };
        let mut local_encode_buf: strbuf_t = zeroed();

        match lua_gettop(l) {
            1 => {}
            2 => {
                luaL_checktype(l, 2, LUA_TTABLE);

                lua_getfield(l, 2, c"escape_slash".as_ptr());
                if !lua_isnil(l, -1) {
                    luaL_checktype(l, -1, LUA_TBOOLEAN);
                    let escape_slash = lua_toboolean(l, -1);
                    if escape_slash != 0 {
                        custom_char2escape[b'/' as usize] = c"\\/".as_ptr();
                        (*ctx.options).char2escape = custom_char2escape.as_ptr();
                    }
                }
                lua_pop(l, 1);

                lua_getfield(l, 2, c"indent".as_ptr());
                if !lua_isnil(l, -1) {
                    options.indent = luaL_checkstring(l, -1);
                    if *options.indent == 0 {
                        options.indent = ptr::null();
                    }
                }
                lua_pop(l, 1);

                lua_getfield(l, 2, c"sort_keys".as_ptr());
                if !lua_isnil(l, -1) {
                    luaL_checktype(l, -1, LUA_TBOOLEAN);
                    let sort_keys = lua_toboolean(l, -1);
                    if sort_keys != 0 {
                        options.sort_keys = sort_keys;
                        strbuf_init(&mut options.keybuf.buf, 0);
                        options.keybuf.size = 0;
                        options.keybuf.capacity = KEYBUF_DEFAULT_CAPACITY;
                        options.keybuf.keys =
                            try_malloc(options.keybuf.capacity * size_of::<key_entry_t>())
                                .cast::<key_entry_t>();
                        if options.keybuf.keys.is_null() {
                            return luaL_error(l, c"out of memory".as_ptr());
                        }
                    }
                }
                lua_pop(l, 2);
            }
            _ => {
                return luaL_error(l, c"expected 1 or 2 arguments".as_ptr());
            }
        }

        let encode_buf = if (*cfg).encode_keep_buffer == 0 {
            strbuf_init(&mut local_encode_buf, 0);
            &raw mut local_encode_buf
        } else {
            let b = &raw mut (*cfg).encode_buf;
            strbuf_reset(b);
            b
        };

        ctx.json = encode_buf;
        json_append_data(l, &mut ctx, 0);
        let mut len: usize = 0;
        let json = strbuf_string(encode_buf, &mut len);

        lua_pushlstring(l, json, len);

        if (*cfg).encode_keep_buffer == 0 {
            strbuf_free(encode_buf);
        }

        if options.sort_keys != 0 {
            strbuf_free(&mut options.keybuf.buf);
            xfree(options.keybuf.keys.cast::<c_void>());
        }

        1
    }
}

fn hexdigit2int(hex: c_char) -> c_int {
    let u = hex as u8;
    if u.is_ascii_digit() {
        return c_int::from(u - b'0');
    }
    let lower = u | 0x20;
    if (b'a'..=b'f').contains(&lower) {
        return c_int::from(10 + lower - b'a');
    }
    -1
}

unsafe fn decode_hex4(hex: *const c_char) -> c_int {
    let mut digit = [0 as c_int; 4];
    unsafe {
        for (i, d) in digit.iter_mut().enumerate() {
            *d = hexdigit2int(*hex.add(i));
            if *d < 0 {
                return -1;
            }
        }
    }
    (digit[0] << 12) + (digit[1] << 8) + (digit[2] << 4) + digit[3]
}

unsafe fn codepoint_to_utf8(utf8: *mut c_char, codepoint: c_int) -> c_int {
    unsafe {
        if codepoint <= 0x7F {
            *utf8 = codepoint as c_char;
            return 1;
        }
        if codepoint <= 0x7FF {
            *utf8 = (((codepoint >> 6) | 0xC0) as u8) as c_char;
            *utf8.add(1) = (((codepoint & 0x3F) | 0x80) as u8) as c_char;
            return 2;
        }
        if codepoint <= 0xFFFF {
            *utf8 = (((codepoint >> 12) | 0xE0) as u8) as c_char;
            *utf8.add(1) = ((((codepoint >> 6) & 0x3F) | 0x80) as u8) as c_char;
            *utf8.add(2) = (((codepoint & 0x3F) | 0x80) as u8) as c_char;
            return 3;
        }
        if codepoint <= 0x1FFFFF {
            *utf8 = (((codepoint >> 18) | 0xF0) as u8) as c_char;
            *utf8.add(1) = ((((codepoint >> 12) & 0x3F) | 0x80) as u8) as c_char;
            *utf8.add(2) = ((((codepoint >> 6) & 0x3F) | 0x80) as u8) as c_char;
            *utf8.add(3) = (((codepoint & 0x3F) | 0x80) as u8) as c_char;
            return 4;
        }
    }
    0
}

unsafe fn json_append_unicode_escape(json: *mut json_parse_t) -> c_int {
    let mut utf8 = [0 as c_char; 4];
    let mut escape_len: usize = 6;

    unsafe {
        let mut codepoint = decode_hex4((*json).ptr.add(2));
        if codepoint < 0 {
            return -1;
        }

        if (codepoint & 0xF800) == 0xD800 {
            if (codepoint & 0x400) != 0 {
                return -1;
            }
            if *(*json).ptr.add(escape_len) != b'\\' as c_char
                || *(*json).ptr.add(escape_len + 1) != b'u' as c_char
            {
                return -1;
            }

            let mut surrogate_low = decode_hex4((*json).ptr.add(2 + escape_len));
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

        let len = codepoint_to_utf8(utf8.as_mut_ptr(), codepoint);
        if len == 0 {
            return -1;
        }

        strbuf_append_mem_unsafe((*json).tmp, utf8.as_ptr(), len as usize);
        (*json).ptr = (*json).ptr.add(escape_len);
    }

    0
}

unsafe fn json_set_token_error(
    token: *mut json_token_t,
    json: *mut json_parse_t,
    errtype: *const c_char,
) {
    unsafe {
        (*token).r#type = T_ERROR;
        (*token).index = (*json).ptr.offset_from((*json).data) as usize;
        (*token).value.string = errtype;
    }
}

unsafe fn json_next_string_token(json: *mut json_parse_t, token: *mut json_token_t) {
    unsafe {
        let escape2char = (*(*json).cfg).escape2char.as_ptr();
        debug_assert_eq!(*(*json).ptr, b'"' as c_char);

        (*json).ptr = (*json).ptr.add(1);
        strbuf_reset((*json).tmp);

        loop {
            let mut ch = *(*json).ptr;
            if ch == b'"' as c_char {
                break;
            }
            if ch == 0 {
                json_set_token_error(token, json, c"unexpected end of string".as_ptr());
                return;
            }

            if ch == b'\\' as c_char {
                ch = *(*json).ptr.add(1);
                ch = *escape2char.add((ch as u8) as usize);
                if ch == b'u' as c_char {
                    if json_append_unicode_escape(json) == 0 {
                        continue;
                    }
                    json_set_token_error(token, json, c"invalid unicode escape code".as_ptr());
                    return;
                }
                if ch == 0 {
                    json_set_token_error(token, json, c"invalid escape code".as_ptr());
                    return;
                }
                (*json).ptr = (*json).ptr.add(1);
            }

            strbuf_append_char_unsafe((*json).tmp, ch);
            (*json).ptr = (*json).ptr.add(1);
        }
        (*json).ptr = (*json).ptr.add(1);

        strbuf_ensure_null((*json).tmp);

        (*token).r#type = T_STRING;
        (*token).value.string = strbuf_string((*json).tmp, &mut (*token).string_len);
    }
}

#[inline]
unsafe fn starts_with_ignore_ascii_case_3(p: *const c_char, target: &[u8; 3]) -> bool {
    unsafe {
        let b0 = *p as u8;
        if b0 == 0 || (b0 | 0x20) != target[0] {
            return false;
        }
        let b1 = *p.add(1) as u8;
        if b1 == 0 || (b1 | 0x20) != target[1] {
            return false;
        }
        let b2 = *p.add(2) as u8;
        b2 != 0 && (b2 | 0x20) == target[2]
    }
}

unsafe fn json_is_invalid_number(json: *mut json_parse_t) -> bool {
    unsafe {
        let mut p = (*json).ptr;
        if *p == b'+' as c_char {
            return true;
        }
        if *p == b'-' as c_char {
            p = p.add(1);
        }
        if *p == b'0' as c_char {
            let ch2 = *p.add(1) as u8;
            if (ch2 | 0x20) == b'x' || ch2.is_ascii_digit() {
                return true;
            }
            return false;
        } else if *p <= (b'9' as c_char) {
            return false;
        }

        if starts_with_ignore_ascii_case_3(p, b"inf") || starts_with_ignore_ascii_case_3(p, b"nan")
        {
            return true;
        }
        false
    }
}

unsafe fn json_next_number_token(json: *mut json_parse_t, token: *mut json_token_t) {
    unsafe {
        let mut endptr: *mut c_char = ptr::null_mut();
        let tmpval = strtoll((*json).ptr, &mut endptr, 10);
        let end_ch = *endptr as u8;
        if (*json).ptr == endptr.cast_const()
            || end_ch == b'.'
            || end_ch == b'e'
            || end_ch == b'E'
            || end_ch == b'x'
        {
            (*token).r#type = T_NUMBER;
            (*token).value.number = fpconv_strtod((*json).ptr, &mut endptr);
            if (*json).ptr == endptr.cast_const() {
                json_set_token_error(token, json, c"invalid number".as_ptr());
                return;
            }
        } else if tmpval > (isize::MAX as c_longlong) || tmpval < (isize::MIN as c_longlong) {
            (*token).r#type = T_NUMBER;
            (*token).value.number = tmpval as c_double;
        } else {
            (*token).r#type = T_INTEGER;
            (*token).value.integer = tmpval as lua_Integer;
        }
        (*json).ptr = endptr;
    }
}

unsafe fn json_next_token(json: *mut json_parse_t, token: *mut json_token_t) {
    unsafe {
        let ch2token = (*(*json).cfg).ch2token.as_ptr();
        let mut ch: usize;

        loop {
            loop {
                ch = (*(*json).ptr as u8) as usize;
                (*token).r#type = *ch2token.add(ch);
                if (*token).r#type != T_WHITESPACE {
                    break;
                }
                (*json).ptr = (*json).ptr.add(1);
            }

            if !(*(*json).options).skip_comments {
                break;
            }

            let c0 = *(*json).ptr as u8;
            if c0 != b'/' {
                break;
            }
            let c1 = *(*json).ptr.add(1) as u8;
            if c1 != b'/' && c1 != b'*' {
                break;
            }

            if c1 == b'/' {
                (*json).ptr = (*json).ptr.add(2);
                while *(*json).ptr != 0 && *(*json).ptr != b'\n' as c_char {
                    (*json).ptr = (*json).ptr.add(1);
                }
            } else {
                (*json).ptr = (*json).ptr.add(2);
                loop {
                    if *(*json).ptr == 0 {
                        json_set_token_error(token, json, c"unclosed multi-line comment".as_ptr());
                        return;
                    }
                    if *(*json).ptr == b'*' as c_char && *(*json).ptr.add(1) == b'/' as c_char {
                        (*json).ptr = (*json).ptr.add(2);
                        break;
                    }
                    (*json).ptr = (*json).ptr.add(1);
                }
            }
        }

        (*token).index = (*json).ptr.offset_from((*json).data) as usize;

        if (*token).r#type == T_ERROR {
            json_set_token_error(token, json, c"invalid token".as_ptr());
            return;
        }

        if (*token).r#type == T_END {
            return;
        }

        if (*token).r#type != T_UNKNOWN {
            (*json).ptr = (*json).ptr.add(1);
            return;
        }

        let ch_u8 = ch as u8;
        if ch_u8 == b'"' {
            json_next_string_token(json, token);
            return;
        } else if ch_u8 == b'-' || ch_u8.is_ascii_digit() {
            if (*(*json).cfg).decode_invalid_numbers == 0 && json_is_invalid_number(json) {
                json_set_token_error(token, json, c"invalid number".as_ptr());
                return;
            }
            json_next_number_token(json, token);
            return;
        } else if strncmp((*json).ptr, c"true".as_ptr(), 4) == 0 {
            (*token).r#type = T_BOOLEAN;
            (*token).value.boolean = 1;
            (*json).ptr = (*json).ptr.add(4);
            return;
        } else if strncmp((*json).ptr, c"false".as_ptr(), 5) == 0 {
            (*token).r#type = T_BOOLEAN;
            (*token).value.boolean = 0;
            (*json).ptr = (*json).ptr.add(5);
            return;
        } else if strncmp((*json).ptr, c"null".as_ptr(), 4) == 0 {
            (*token).r#type = T_NULL;
            (*json).ptr = (*json).ptr.add(4);
            return;
        } else if (*(*json).cfg).decode_invalid_numbers != 0 && json_is_invalid_number(json) {
            json_next_number_token(json, token);
            return;
        }

        json_set_token_error(token, json, c"invalid token".as_ptr());
    }
}

unsafe fn json_throw_parse_error(
    l: *mut lua_State,
    json: *mut json_parse_t,
    exp: *const c_char,
    token: *mut json_token_t,
) -> ! {
    unsafe {
        strbuf_free((*json).tmp);
        let found = if (*token).r#type == T_ERROR {
            (*token).value.string
        } else {
            JSON_TOKEN_TYPE_NAME[(*token).r#type as usize].as_ptr()
        };

        luaL_error(
            l,
            c"Expected %s but found %s at character %d".as_ptr(),
            exp,
            found,
            ((*token).index + 1) as c_int,
        );
        unreachable!()
    }
}

#[inline]
unsafe fn json_decode_ascend(json: *mut json_parse_t) {
    unsafe {
        (*json).current_depth -= 1;
    }
}

unsafe fn json_decode_descend(l: *mut lua_State, json: *mut json_parse_t, slots: c_int) {
    unsafe {
        (*json).current_depth += 1;
        if (*json).current_depth <= (*(*json).cfg).decode_max_depth && lua_checkstack(l, slots) != 0
        {
            return;
        }

        strbuf_free((*json).tmp);
        luaL_error(
            l,
            c"Found too many nested data structures (%d) at character %d".as_ptr(),
            (*json).current_depth,
            (*json).ptr.offset_from((*json).data) as c_int,
        );
    }
}

unsafe fn json_parse_object_context(l: *mut lua_State, json: *mut json_parse_t) {
    unsafe {
        let mut token: json_token_t = zeroed();

        json_decode_descend(l, json, 3);
        lua_newtable(l);
        json_next_token(json, &mut token);

        if token.r#type == T_OBJ_END {
            nlua_pushref(l, nlua_get_empty_dict_ref(l));
            lua_setmetatable(l, -2);
            json_decode_ascend(json);
            return;
        }

        loop {
            if token.r#type != T_STRING {
                json_throw_parse_error(l, json, c"object key string".as_ptr(), &mut token);
            }

            lua_pushlstring(l, token.value.string, token.string_len);

            json_next_token(json, &mut token);
            if token.r#type != T_COLON {
                json_throw_parse_error(l, json, c"colon".as_ptr(), &mut token);
            }

            json_next_token(json, &mut token);
            json_process_value(l, json, &mut token, (*(*json).options).luanil_object);

            lua_rawset(l, -3);

            json_next_token(json, &mut token);
            if token.r#type == T_OBJ_END {
                json_decode_ascend(json);
                return;
            }
            if token.r#type != T_COMMA {
                json_throw_parse_error(l, json, c"comma or object end".as_ptr(), &mut token);
            }

            json_next_token(json, &mut token);
        }
    }
}

unsafe fn json_parse_array_context(l: *mut lua_State, json: *mut json_parse_t) {
    unsafe {
        let mut token: json_token_t = zeroed();

        json_decode_descend(l, json, 2);
        lua_newtable(l);

        if (*(*json).cfg).decode_array_with_array_mt != 0 {
            lua_pushlightuserdata(
                l,
                json_lightudata_mask((&raw const JSON_ARRAY).cast::<c_void>()),
            );
            lua_rawget(l, LUA_REGISTRYINDEX);
            lua_setmetatable(l, -2);
        }

        json_next_token(json, &mut token);

        if token.r#type == T_ARR_END {
            json_decode_ascend(json);
            return;
        }

        let mut i: c_int = 1;
        loop {
            json_process_value(l, json, &mut token, (*(*json).options).luanil_array);
            lua_rawseti(l, -2, i);

            json_next_token(json, &mut token);
            if token.r#type == T_ARR_END {
                json_decode_ascend(json);
                return;
            }
            if token.r#type != T_COMMA {
                json_throw_parse_error(l, json, c"comma or array end".as_ptr(), &mut token);
            }

            json_next_token(json, &mut token);
            i += 1;
        }
    }
}

unsafe fn json_process_value(
    l: *mut lua_State,
    json: *mut json_parse_t,
    token: *mut json_token_t,
    use_luanil: bool,
) {
    unsafe {
        match (*token).r#type {
            T_STRING => {
                lua_pushlstring(l, (*token).value.string, (*token).string_len);
            }
            T_NUMBER => {
                lua_pushnumber(l, (*token).value.number);
            }
            T_INTEGER => {
                lua_pushinteger(l, (*token).value.integer);
            }
            T_BOOLEAN => {
                lua_pushboolean(l, (*token).value.boolean);
            }
            T_OBJ_BEGIN => {
                json_parse_object_context(l, json);
            }
            T_ARR_BEGIN => {
                json_parse_array_context(l, json);
            }
            T_NULL => {
                if use_luanil {
                    lua_pushnil(l);
                } else {
                    nlua_pushref(l, nlua_get_nil_ref(l));
                }
            }
            _ => {
                json_throw_parse_error(l, json, c"value".as_ptr(), token);
            }
        }
    }
}

unsafe extern "C-unwind" fn json_decode(l: *mut lua_State) -> c_int {
    unsafe {
        let mut options = json_options_t {
            luanil_object: false,
            luanil_array: false,
            skip_comments: false,
        };

        match lua_gettop(l) {
            1 => {}
            2 => {
                luaL_checktype(l, 2, LUA_TTABLE);

                lua_getfield(l, 2, c"skip_comments".as_ptr());
                options.skip_comments = lua_toboolean(l, -1) != 0;
                lua_pop(l, 1);

                lua_getfield(l, 2, c"luanil".as_ptr());
                if lua_isnil(l, -1) {
                    lua_pop(l, 1);
                } else {
                    luaL_checktype(l, -1, LUA_TTABLE);

                    lua_getfield(l, -1, c"object".as_ptr());
                    options.luanil_object = lua_toboolean(l, -1) != 0;
                    lua_pop(l, 1);

                    lua_getfield(l, -1, c"array".as_ptr());
                    options.luanil_array = lua_toboolean(l, -1) != 0;
                    lua_pop(l, 2);
                }
            }
            _ => {
                return luaL_error(l, c"expected 1 or 2 arguments".as_ptr());
            }
        }

        let mut json_len: usize = 0;
        let data = luaL_checklstring(l, 1, &mut json_len);
        let mut json = json_parse_t {
            data,
            ptr: data,
            tmp: ptr::null_mut(),
            cfg: json_fetch_config(l),
            options: &mut options,
            current_depth: 0,
        };

        if json_len >= 2 && (*json.data == 0 || *json.data.add(1) == 0) {
            luaL_error(l, c"JSON parser does not support UTF-16 or UTF-32".as_ptr());
        }

        json.tmp = strbuf_new(json_len);

        let mut token: json_token_t = zeroed();
        json_next_token(&mut json, &mut token);
        json_process_value(l, &mut json, &mut token, (*json.options).luanil_object);

        json_next_token(&mut json, &mut token);
        if token.r#type != T_END {
            json_throw_parse_error(l, &mut json, c"the end".as_ptr(), &mut token);
        }

        strbuf_free(json.tmp);
        1
    }
}

unsafe fn compat_luaL_setfuncs(l: *mut lua_State, mut reg: *const luaL_Reg, nup: c_int) {
    unsafe {
        luaL_checkstack(l, nup, c"too many upvalues".as_ptr());
        while !(*reg).name.is_null() {
            for _ in 0..nup {
                lua_pushvalue(l, -nup);
            }
            lua_pushcclosure(l, (*reg).func, nup);
            lua_setfield(l, -(nup + 2), (*reg).name);
            reg = reg.add(1);
        }
        lua_pop(l, nup);
    }
}

unsafe extern "C-unwind" fn json_protect_conversion(l: *mut lua_State) -> c_int {
    unsafe {
        if lua_gettop(l) != 1 {
            luaL_argerror(l, 1, c"expected 1 argument".as_ptr());
        }

        lua_pushvalue(l, lua_upvalueindex(1));
        lua_insert(l, 1);
        let err = lua_pcall(l, 1, 1, 0);
        if err == 0 {
            return 1;
        }
        if err == LUA_ERRRUN {
            lua_pushnil(l);
            lua_insert(l, -2);
            return 2;
        }

        luaL_error(
            l,
            c"Memory allocation error in CJSON protected call".as_ptr(),
        )
    }
}

pub unsafe extern "C-unwind" fn lua_cjson_new(l: *mut lua_State) -> c_int {
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

    unsafe {
        lua_getfield(l, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        let is_thread = lua_toboolean(l, -1) != 0;
        lua_pop(l, 1);

        if !is_thread {
            fpconv_init();
        }

        lua_pushlightuserdata(
            l,
            json_lightudata_mask((&raw const JSON_EMPTY_ARRAY).cast::<c_void>()),
        );
        lua_rawget(l, LUA_REGISTRYINDEX);
        if lua_isnil(l, -1) {
            lua_pop(l, 1);

            lua_pushlightuserdata(
                l,
                json_lightudata_mask((&raw const JSON_EMPTY_ARRAY).cast::<c_void>()),
            );
            lua_newtable(l);
            lua_rawset(l, LUA_REGISTRYINDEX);

            lua_pushlightuserdata(
                l,
                json_lightudata_mask((&raw const JSON_ARRAY).cast::<c_void>()),
            );
            lua_newtable(l);
            lua_rawset(l, LUA_REGISTRYINDEX);
        }

        lua_newtable(l);

        json_create_config(l);
        compat_luaL_setfuncs(l, reg.as_ptr(), 1);

        lua_pushlstring(
            l,
            CJSON_MODNAME.as_ptr().cast::<c_char>(),
            CJSON_MODNAME.len(),
        );
        lua_setfield(l, -2, c"_NAME".as_ptr());
        lua_pushlstring(
            l,
            CJSON_VERSION.as_ptr().cast::<c_char>(),
            CJSON_VERSION.len(),
        );
        lua_setfield(l, -2, c"_VERSION".as_ptr());
    }

    1
}

unsafe extern "C-unwind" fn lua_cjson_safe_new(l: *mut lua_State) -> c_int {
    const FUNCS: [*const c_char; 2] = [c"decode".as_ptr(), c"encode".as_ptr()];
    unsafe {
        lua_cjson_new(l);

        lua_pushcfunction(l, Some(lua_cjson_safe_new));
        lua_setfield(l, -2, c"new".as_ptr());

        for &f in &FUNCS {
            lua_getfield(l, -1, f);
            lua_pushcclosure(l, Some(json_protect_conversion), 1);
            lua_setfield(l, -2, f);
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_cjson(l: *mut lua_State) -> c_int {
    unsafe {
        lua_cjson_new(l);
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_cjson_safe(l: *mut lua_State) -> c_int {
    unsafe {
        lua_cjson_safe_new(l);
    }
    1
}
