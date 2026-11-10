#![forbid(unsafe_code)]
//! Lua bindings for Tree-sitter (`src/nvim/lua/treesitter.c`).

use crate::buffer::FileBuf;
use crate::context::handle_get_buffer;
use crate::cursor::Linenr;
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::lua::converter::{
    LUA_MINSTACK, LUA_REGISTRYINDEX, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE, lua_Integer, lua_Number,
    lua_State, lua_concat_safe, lua_createtable_safe, lua_error_safe, lua_getfenv_safe,
    lua_getfield, lua_isboolean, lua_isfunction, lua_isnil, lua_isstring_safe, lua_istable,
    lua_newtable, lua_newuserdata_typed, lua_objlen_safe, lua_pop, lua_push_bytes,
    lua_pushboolean_safe, lua_pushcclosure_safe, lua_pushcfunction_safe, lua_pushinteger_safe,
    lua_pushnil_safe, lua_pushnumber_safe, lua_pushstring, lua_pushstring_opt, lua_pushvalue_safe,
    lua_rawgeti_safe, lua_rawseti_safe, lua_register_funcs, lua_setfenv_safe, lua_setfield,
    lua_setmetatable_safe, lua_toboolean_safe, lua_tointeger_safe, lua_tolstring_bytes,
    lua_touserdata_typed, lua_type_safe, lua_upvalueindex, luaL_argcheck, luaL_argerror,
    luaL_checkbytes, luaL_checkint, luaL_checkinteger_safe, luaL_checknumber_safe,
    luaL_checkstring, luaL_checkudata_opt_typed, luaL_checkudata_typed, luaL_error_cstr,
    luaL_error_cstr_arg, luaL_error_str, luaL_newmetatable, luaL_ref_safe, nlua_gettop,
};
use crate::memline::{ml_get_buf_len, ml_get_buf_slice};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::os::dl::{DlLoadError, os_dlopen_ts_language};
use crate::strings::format_to_buf;
use std::ffi::CStr;
use std::ptr;

pub use crate::lua::ffi::*;

const BUFSIZE: usize = 256;

const TREE_SITTER_LANGUAGE_VERSION: u32 = 15;
const TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION: u32 = 13;

const TSSYMBOL_TYPE_ANONYMOUS: TSSymbolType = 1;
const TSSYMBOL_TYPE_AUXILIARY: TSSymbolType = 3;

const TSQUERY_PREDICATE_STEP_TYPE_DONE: TSQueryPredicateStepType = 0;
const TSQUERY_PREDICATE_STEP_TYPE_CAPTURE: TSQueryPredicateStepType = 1;
const TSQUERY_PREDICATE_STEP_TYPE_STRING: TSQueryPredicateStepType = 2;

const TSQUERY_ERROR_SYNTAX: TSQueryError = 1;
const TSQUERY_ERROR_NODE_TYPE: TSQueryError = 2;
const TSQUERY_ERROR_FIELD: TSQueryError = 3;
const TSQUERY_ERROR_CAPTURE: TSQueryError = 4;
const TSQUERY_ERROR_STRUCTURE: TSQueryError = 5;

const TS_META_PARSER: &CStr = c"treesitter_parser";
const TS_META_TREE: &CStr = c"treesitter_tree";
const TS_META_NODE: &CStr = c"treesitter_node";
const TS_META_QUERY: &CStr = c"treesitter_query";
const TS_META_QUERYCURSOR: &CStr = c"treesitter_querycursor";
const TS_META_QUERYMATCH: &CStr = c"treesitter_querymatch";

#[repr(C)]
struct TSLuaTree {
    tree: *const TSTree,
}

std::thread_local! {
    static TS_LANGS: std::cell::RefCell<std::collections::BTreeMap<Vec<u8>, *mut TSLanguage>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

static PARSER_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 8] = [
    (c"__gc", parser_gc),
    (c"__tostring", parser_tostring),
    (c"parse", parser_parse),
    (c"reset", parser_reset),
    (c"set_included_ranges", parser_set_ranges),
    (c"included_ranges", parser_get_ranges),
    (c"_set_logger", parser_set_logger),
    (c"_logger", parser_get_logger),
];

static TREE_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 6] = [
    (c"__gc", tree_gc),
    (c"__tostring", tree_tostring),
    (c"root", tree_root),
    (c"edit", tree_edit),
    (c"included_ranges", tree_get_ranges),
    (c"copy", tree_copy),
];

static NODE_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 35] = [
    (c"__tostring", node_tostring),
    (c"__eq", node_eq),
    (c"__len", node_child_count),
    (c"id", node_id),
    (c"range", node_range),
    (c"start", node_start),
    (c"end_", node_end),
    (c"type", node_type),
    (c"symbol", node_symbol),
    (c"field", node_field),
    (c"named", node_named),
    (c"missing", node_missing),
    (c"extra", node_extra),
    (c"has_changes", node_has_changes),
    (c"has_error", node_has_error),
    (c"sexpr", node_sexpr),
    (c"child_count", node_child_count),
    (c"named_child_count", node_named_child_count),
    (c"child", node_child),
    (c"named_child", node_named_child),
    (c"descendant_for_range", node_descendant_for_range),
    (
        c"named_descendant_for_range",
        node_named_descendant_for_range,
    ),
    (c"parent", node_parent),
    (c"__has_ancestor", __has_ancestor),
    (c"child_with_descendant", node_child_with_descendant),
    (c"iter_children", node_iter_children),
    (c"next_sibling", node_next_sibling),
    (c"prev_sibling", node_prev_sibling),
    (c"next_named_sibling", node_next_named_sibling),
    (c"prev_named_sibling", node_prev_named_sibling),
    (c"named_children", node_named_children),
    (c"root", node_root),
    (c"tree", node_tree),
    (c"byte_length", node_byte_length),
    (c"equal", node_equal),
];

static QUERYCURSOR_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 4] = [
    (c"remove_match", querycursor_remove_match),
    (c"next_capture", querycursor_next_capture),
    (c"next_match", querycursor_next_match),
    (c"__gc", querycursor_gc),
];

static QUERYMATCH_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 2] = [
    (c"info", querymatch_info),
    (c"captures", querymatch_captures),
];

static QUERY_META: [(&CStr, extern "C-unwind" fn(*mut lua_State) -> i32); 5] = [
    (c"__gc", query_gc),
    (c"__tostring", query_tostring),
    (c"inspect", query_inspect),
    (c"disable_capture", query_disable_capture),
    (c"disable_pattern", query_disable_pattern),
];

#[inline]
fn langs_has(lang_name: Option<&[u8]>) -> bool {
    lang_name.is_some_and(|name| TS_LANGS.with_borrow(|langs| langs.contains_key(name)))
}

#[inline]
fn langs_get(lang_name: Option<&[u8]>) -> *mut TSLanguage {
    lang_name
        .and_then(|name| TS_LANGS.with_borrow(|langs| langs.get(name).copied()))
        .unwrap_or(ptr::null_mut())
}

extern "C-unwind" fn tslua_has_language(lstate: *mut lua_State) -> i32 {
    let lang_name = luaL_checkstring(lstate, 1);
    let has = langs_has(Some(lang_name.to_bytes()));
    lua_pushboolean_safe(lstate, has);
    1
}

extern "C-unwind" fn tslua_add_language_from_object(lstate: *mut lua_State) -> i32 {
    add_language(lstate, false)
}

#[cfg(target_os = "emscripten")]
pub fn nvim_ts_get_parser(lang: Option<&[u8]>) -> *const TSLanguage {
    ts_get_static_parser(lang)
}

fn load_language_from_object(
    lstate: *mut lua_State,
    path: &CStr,
    lang_name: &CStr,
    symbol: &CStr,
) -> *const TSLanguage {
    #[cfg(target_os = "emscripten")]
    {
        let static_lang = nvim_ts_get_parser(Some(symbol.to_bytes()));
        if !static_lang.is_null() {
            return static_lang;
        }
    }

    let mut symbol_buf = [0u8; 128];
    format_to_buf!(&mut symbol_buf, c"tree_sitter_%s", symbol);
    let symbol_cstr = crate::memory::bytes_as_cstr(&symbol_buf);

    match os_dlopen_ts_language(path, symbol_cstr) {
        Ok(lang) => lang,
        Err(DlLoadError::OpenFailed(err_msg)) => {
            crate::memory::copy_bytes_to_c_buf(g_st().iobuff_bytes_mut(), err_msg.as_bytes());
            let msg = format!(
                "Failed to load parser for language '{}': uv_dlopen: {}",
                lang_name.to_string_lossy(),
                g_st().iobuff_cstr().to_string_lossy()
            );
            luaL_error_str(lstate, &msg);
            ptr::null()
        }
        Err(DlLoadError::SymFailed(err_msg)) => {
            crate::memory::copy_bytes_to_c_buf(g_st().iobuff_bytes_mut(), err_msg.as_bytes());
            luaL_error_cstr_arg(
                lstate,
                c"Failed to load parser: uv_dlsym: %s",
                g_st().iobuff_cstr(),
            );
            ptr::null()
        }
        Err(DlLoadError::NullLanguage) => {
            luaL_error_cstr_arg(lstate, c"Failed to load parser %s: internal error", path);
            ptr::null()
        }
    }
}

fn load_language_from_wasm(
    lstate: *mut lua_State,
    _path: &CStr,
    _lang_name: &CStr,
) -> *const TSLanguage {
    luaL_error_cstr(lstate, c"Not supported");
    ptr::null()
}

fn add_language(lstate: *mut lua_State, is_wasm: bool) -> i32 {
    let path = luaL_checkstring(lstate, 1);
    let lang_name = luaL_checkstring(lstate, 2);
    let mut symbol_name = lang_name;

    if !is_wasm && nlua_gettop(lstate) >= 3 && !lua_isnil(lstate, 3) {
        symbol_name = luaL_checkstring(lstate, 3);
    }

    if langs_has(Some(lang_name.to_bytes())) {
        lua_pushboolean_safe(lstate, true);
        return 1;
    }

    let lang = if is_wasm {
        load_language_from_wasm(lstate, path, lang_name)
    } else {
        load_language_from_object(lstate, path, lang_name, symbol_name)
    };

    let lang_version = ts_lang_abi_version(lang);
    if !(TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION..=TREE_SITTER_LANGUAGE_VERSION)
        .contains(&lang_version)
    {
        let msg = format!(
            "ABI version mismatch for {}: supported between {} and {}, found {}",
            path.to_string_lossy(),
            TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION,
            TREE_SITTER_LANGUAGE_VERSION,
            lang_version
        );
        return luaL_error_str(lstate, &msg);
    }

    TS_LANGS.with_borrow_mut(|langs| {
        langs.insert(lang_name.to_bytes().to_vec(), lang.cast_mut());
    });

    lua_pushboolean_safe(lstate, true);
    1
}

extern "C-unwind" fn tslua_remove_lang(lstate: *mut lua_State) -> i32 {
    let lang_name = luaL_checkstring(lstate, 1);
    let present = TS_LANGS.with_borrow_mut(|langs| langs.remove(lang_name.to_bytes()).is_some());
    lua_pushboolean_safe(lstate, present);
    1
}

fn lang_check(lstate: *mut lua_State, index: i32) -> *mut TSLanguage {
    let lang_name = luaL_checkstring(lstate, index);
    let lang = langs_get(Some(lang_name.to_bytes()));
    if lang.is_null() {
        luaL_error_cstr_arg(lstate, c"no such language: %s", lang_name);
    }
    lang
}

extern "C-unwind" fn tslua_inspect_lang(lstate: *mut lua_State) -> i32 {
    let lang = lang_check(lstate, 1);

    lua_createtable_safe(lstate, 0, 2);

    {
        let nsymbols = ts_lang_symbol_count(lang);
        debug_assert!(nsymbols < i32::MAX as u32);

        lua_createtable_safe(lstate, (nsymbols.wrapping_sub(1)) as i32, 1);
        for i in 0..nsymbols {
            let t = ts_lang_symbol_type(lang, i as TSSymbol);
            if t == TSSYMBOL_TYPE_AUXILIARY {
                continue;
            }
            let name_cstr = ts_lang_symbol_name(lang, i as TSSymbol).unwrap_or(c"");
            let named = t != TSSYMBOL_TYPE_ANONYMOUS;
            lua_pushboolean_safe(lstate, named);
            if !named {
                let mut buf = [0u8; 256];
                format_to_buf!(&mut buf, c"\"%s\"", name_cstr);
                lua_setfield(
                    lstate,
                    -2,
                    CStr::from_bytes_until_nul(&buf).unwrap_or_default(),
                );
            } else {
                lua_setfield(lstate, -2, name_cstr);
            }
        }

        lua_setfield(lstate, -2, c"symbols");
    }

    {
        let nfields = ts_lang_field_count(lang);
        lua_createtable_safe(lstate, nfields as i32, 1);
        for i in 1..=nfields {
            let fname = ts_lang_field_name(lang, i as TSFieldId);
            lua_pushstring_opt(lstate, fname);
            lua_rawseti_safe(lstate, -2, i as i32);
        }

        lua_setfield(lstate, -2, c"fields");
    }

    lua_pushboolean_safe(lstate, ts_lang_is_wasm(lang));
    lua_setfield(lstate, -2, c"_wasm");

    lua_pushinteger_safe(lstate, ts_lang_abi_version(lang) as lua_Integer);
    lua_setfield(lstate, -2, c"abi_version");

    if let Some(meta) = ts_lang_metadata(lang) {
        lua_createtable_safe(lstate, 0, 3);

        lua_pushinteger_safe(lstate, meta.major_version as lua_Integer);
        lua_setfield(lstate, -2, c"major_version");
        lua_pushinteger_safe(lstate, meta.minor_version as lua_Integer);
        lua_setfield(lstate, -2, c"minor_version");
        lua_pushinteger_safe(lstate, meta.patch_version as lua_Integer);
        lua_setfield(lstate, -2, c"patch_version");

        lua_setfield(lstate, -2, c"metadata");
    }

    lua_pushinteger_safe(lstate, ts_lang_state_count(lang) as lua_Integer);
    lua_setfield(lstate, -2, c"state_count");

    {
        let supertypes = ts_lang_supertypes(lang);

        lua_createtable_safe(lstate, 0, supertypes.len() as i32);
        for &supertype in supertypes {
            let (subtypes_ptr, nsubtypes) = ts_lang_subtypes_raw(lang, supertype);

            lua_createtable_safe(lstate, nsubtypes as i32, 0);
            for j in 1..=nsubtypes {
                let sname = ts_lang_symbol_name(lang, ts_lang_subtype_at(subtypes_ptr, j));
                lua_pushstring_opt(lstate, sname);
                lua_rawseti_safe(lstate, -2, j as i32);
            }

            let super_name = ts_lang_symbol_name(lang, supertype).unwrap_or(c"");
            lua_setfield(lstate, -2, super_name);
        }

        lua_setfield(lstate, -2, c"supertypes");
    }

    1
}

extern "C-unwind" fn tslua_push_parser(lstate: *mut lua_State) -> i32 {
    let lang = lang_check(lstate, 1);

    let parser_slot = lua_newuserdata_typed::<*mut TSParser>(lstate, ptr::null_mut());
    let Some(p) = ts_parser_create(lang) else {
        let lang_name = luaL_checkstring(lstate, 1);
        return luaL_error_cstr_arg(lstate, c"Failed to load language : %s", lang_name);
    };
    *parser_slot = p;

    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_PARSER);
    let _ = lua_setmetatable_safe(lstate, -2);
    1
}

fn parser_check(lstate: *mut lua_State, index: i32) -> *mut TSParser {
    let ud = luaL_checkudata_typed::<*mut TSParser>(lstate, index, TS_META_PARSER);
    luaL_argcheck(lstate, !(*ud).is_null(), index, c"Parser has been deleted");
    *ud
}

extern "C-unwind" fn parser_gc(lstate: *mut lua_State) -> i32 {
    let ud = luaL_checkudata_typed::<*mut TSParser>(lstate, 1, TS_META_PARSER);
    if !(*ud).is_null() {
        ts_parser_free(*ud);
        *ud = ptr::null_mut();
    }
    0
}

extern "C-unwind" fn parser_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"<parser>");
    1
}

fn input_cb(bp: &mut FileBuf, position: TSPoint, buf: &mut [u8; BUFSIZE]) -> u32 {
    if (position.row as Linenr) >= bp.b_ml.ml_line_count {
        return 0;
    }
    let lnum = (position.row as Linenr) + 1;
    let len = ml_get_buf_len(bp, lnum) as usize;
    let col = position.column as usize;
    if col > len {
        return 0;
    }
    let tocopy = (len - col).min(BUFSIZE);

    let line_slice = ml_get_buf_slice(bp, lnum);
    buf[..tocopy].copy_from_slice(&line_slice[col..col + tocopy]);
    for b in &mut buf[..tocopy] {
        if *b == b'\n' {
            *b = 0;
        }
    }
    let mut bytes_read = tocopy as u32;
    if tocopy < BUFSIZE
        && (lnum != bp.b_ml.ml_line_count
            || (!bp.b_p_bin && bp.b_p_fixeol)
            || (lnum != bp.b_no_eol_lnum && bp.b_p_eol))
    {
        buf[tocopy] = b'\n';
        bytes_read += 1;
    }
    bytes_read
}

fn push_ranges(lstate: *mut lua_State, ranges: &[TSRange], include_bytes: bool) {
    lua_createtable_safe(lstate, ranges.len() as i32, 0);
    for (i, &r) in ranges.iter().enumerate() {
        lua_createtable_safe(lstate, if include_bytes { 6 } else { 4 }, 0);
        let mut j: i32 = 1;
        lua_pushnumber_safe(lstate, r.start_point.row as lua_Number);
        lua_rawseti_safe(lstate, -2, j);
        j += 1;
        lua_pushnumber_safe(lstate, r.start_point.column as lua_Number);
        lua_rawseti_safe(lstate, -2, j);
        j += 1;
        if include_bytes {
            lua_pushnumber_safe(lstate, r.start_byte as lua_Number);
            lua_rawseti_safe(lstate, -2, j);
            j += 1;
        }
        lua_pushnumber_safe(lstate, r.end_point.row as lua_Number);
        lua_rawseti_safe(lstate, -2, j);
        j += 1;
        lua_pushnumber_safe(lstate, r.end_point.column as lua_Number);
        lua_rawseti_safe(lstate, -2, j);
        j += 1;
        if include_bytes {
            lua_pushnumber_safe(lstate, r.end_byte as lua_Number);
            lua_rawseti_safe(lstate, -2, j);
        }

        lua_rawseti_safe(lstate, -2, (i + 1) as i32);
    }
}

pub extern "C-unwind" fn nts_parser_parse_buf(
    p: *mut TSParser,
    old_tree: *const TSTree,
    bufnr: i32,
    timeout_ns: u64,
) -> *mut TSTree {
    let Some(buf) = handle_get_buffer(bufnr as ObjHandle) else {
        std::process::abort();
    };
    ts_parser_parse_with_input(p, old_tree, buf, input_cb, timeout_ns)
}

extern "C-unwind" fn parser_parse(lstate: *mut lua_State) -> i32 {
    let p = parser_check(lstate, 1);
    let mut old_tree: *const TSTree = ptr::null();
    if !lua_isnil(lstate, 2)
        && let Some(ud) = luaL_checkudata_opt_typed::<TSLuaTree>(lstate, 2, TS_META_TREE)
    {
        old_tree = ud.tree;
    }

    let new_tree: *mut TSTree = match lua_type_safe(lstate, 3) {
        LUA_TSTRING => {
            let str_bytes = lua_tolstring_bytes(lstate, 3);
            ts_parser_parse_bytes(p, old_tree, str_bytes)
        }
        LUA_TNUMBER => {
            let bufnr = lua_tointeger_safe(lstate, 3) as ObjHandle;
            if handle_get_buffer(bufnr).is_none() {
                let msg = format!("invalid buffer handle: {bufnr}\0");
                let msg_cstr = CStr::from_bytes_with_nul(msg.as_bytes()).unwrap();
                return luaL_argerror(lstate, 3, msg_cstr);
            }

            let timeout_ns = if lua_isnil(lstate, 5) {
                0
            } else {
                lua_tointeger_safe(lstate, 5) as u64
            };
            nts_parser_parse_buf(p, old_tree, bufnr, timeout_ns)
        }
        _ => {
            return luaL_argerror(lstate, 3, c"expected either string or buffer handle");
        }
    };

    let include_bytes = nlua_gettop(lstate) >= 4 && lua_toboolean_safe(lstate, 4);

    if new_tree.is_null() {
        if !ts_parser_has_language(p) {
            return luaL_error_cstr(lstate, c"Language was unset, or has an incompatible ABI.");
        }
        return 0;
    }

    let changed = if !old_tree.is_null() {
        ts_tree_changed_ranges_owned(old_tree, new_tree)
    } else {
        ts_tree_included_ranges_owned(new_tree)
    };

    push_tree(lstate, new_tree);
    push_ranges(lstate, changed.as_slice(), include_bytes);
    2
}

extern "C-unwind" fn parser_reset(lstate: *mut lua_State) -> i32 {
    let p = parser_check(lstate, 1);
    ts_parser_reset_safe(p);
    0
}

fn range_err(lstate: *mut lua_State) {
    luaL_error_cstr(
        lstate,
        c"Ranges can only be made from 6 element long tables or nodes.",
    );
}

fn lua_checkuint32(lstate: *mut lua_State, index: i32) -> u32 {
    let value = luaL_checknumber_safe(lstate, index);
    let converted = value as u32;
    if value < 0.0 || value > (u32::MAX as lua_Number) || (converted as lua_Number) != value {
        luaL_error_cstr(lstate, c"Range value out of bounds");
    }
    converted
}

fn range_from_lua(lstate: *mut lua_State, range: &mut TSRange) {
    let mut node = TSNode::empty();

    if lua_istable(lstate, -1) {
        if lua_objlen_safe(lstate, -1) != 6 {
            range_err(lstate);
        }

        lua_rawgeti_safe(lstate, -1, 1);
        let start_row = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        lua_rawgeti_safe(lstate, -1, 2);
        let start_col = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        lua_rawgeti_safe(lstate, -1, 3);
        let start_byte = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        lua_rawgeti_safe(lstate, -1, 4);
        let end_row = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        lua_rawgeti_safe(lstate, -1, 5);
        let end_col = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        lua_rawgeti_safe(lstate, -1, 6);
        let end_byte = lua_checkuint32(lstate, -1);
        lua_pop(lstate, 1);

        *range = TSRange {
            start_point: TSPoint {
                row: start_row,
                column: start_col,
            },
            end_point: TSPoint {
                row: end_row,
                column: end_col,
            },
            start_byte,
            end_byte,
        };
    } else if node_check_opt(lstate, -1, &mut node) {
        *range = TSRange {
            start_point: node.start_point(),
            end_point: node.end_point(),
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
        };
    } else {
        range_err(lstate);
    }
}

extern "C-unwind" fn parser_set_ranges(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 2 {
        return luaL_error_cstr(lstate, c"not enough args to parser:set_included_ranges()");
    }

    let p = parser_check(lstate, 1);
    luaL_argcheck(lstate, lua_istable(lstate, 2), 2, c"table expected.");

    let tbl_len = lua_objlen_safe(lstate, 2);
    let mut ranges: Vec<TSRange> = vec![TSRange::new(); tbl_len];

    for (index, range) in ranges.iter_mut().enumerate() {
        lua_rawgeti_safe(lstate, 2, (index + 1) as i32);
        range_from_lua(lstate, range);
        lua_pop(lstate, 1);
    }

    ts_parser_set_ranges_slice(p, &ranges);

    0
}

extern "C-unwind" fn parser_get_ranges(lstate: *mut lua_State) -> i32 {
    let p = parser_check(lstate, 1);
    let include_bytes = nlua_gettop(lstate) >= 2 && lua_toboolean_safe(lstate, 2);

    let ranges_slice = ts_parser_get_ranges_slice(p);
    push_ranges(lstate, ranges_slice, include_bytes);
    1
}

extern "C-unwind" fn parser_set_logger(lstate: *mut lua_State) -> i32 {
    let p = parser_check(lstate, 1);

    luaL_argcheck(lstate, lua_isboolean(lstate, 2), 2, c"boolean expected");
    luaL_argcheck(lstate, lua_isboolean(lstate, 3), 3, c"boolean expected");
    luaL_argcheck(lstate, lua_isfunction(lstate, 4), 4, c"function expected");

    lua_pushvalue_safe(lstate, 4);
    let ref_ = luaL_ref_safe(lstate, LUA_REGISTRYINDEX);

    let lex = lua_toboolean_safe(lstate, 2);
    let parse = lua_toboolean_safe(lstate, 3);
    ts_parser_set_lua_logger(p, lstate, lex, parse, ref_);
    0
}

extern "C-unwind" fn parser_get_logger(lstate: *mut lua_State) -> i32 {
    let p = parser_check(lstate, 1);
    if let Some(cb) = ts_parser_get_lua_logger_cb(p) {
        lua_rawgeti_safe(lstate, LUA_REGISTRYINDEX, cb);
    } else {
        lua_pushnil_safe(lstate);
    }
    1
}

fn push_tree(lstate: *mut lua_State, tree: *const TSTree) {
    if tree.is_null() {
        lua_pushnil_safe(lstate);
        return;
    }

    let _ = lua_newuserdata_typed(lstate, TSLuaTree { tree });
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_TREE);
    let _ = lua_setmetatable_safe(lstate, -2);
}

extern "C-unwind" fn tree_copy(lstate: *mut lua_State) -> i32 {
    let ud = luaL_checkudata_typed::<TSLuaTree>(lstate, 1, TS_META_TREE);
    let copy = ts_tree_clone(ud.tree);
    push_tree(lstate, copy);
    1
}

extern "C-unwind" fn tree_edit(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 10 {
        lua_pushstring(lstate, c"not enough args to tree:edit()");
        return lua_error_safe(lstate);
    }

    let ud = luaL_checkudata_typed::<TSLuaTree>(lstate, 1, TS_META_TREE);

    let start_byte = luaL_checkint(lstate, 2) as u32;
    let old_end_byte = luaL_checkint(lstate, 3) as u32;
    let new_end_byte = luaL_checkint(lstate, 4) as u32;
    let start_point = TSPoint {
        row: luaL_checkint(lstate, 5) as u32,
        column: luaL_checkint(lstate, 6) as u32,
    };
    let old_end_point = TSPoint {
        row: luaL_checkint(lstate, 7) as u32,
        column: luaL_checkint(lstate, 8) as u32,
    };
    let new_end_point = TSPoint {
        row: luaL_checkint(lstate, 9) as u32,
        column: luaL_checkint(lstate, 10) as u32,
    };

    let edit = TSInputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_point,
        old_end_point,
        new_end_point,
    };

    let new_tree = ts_tree_clone(ud.tree);
    ts_tree_edit_safe(new_tree, &edit);

    push_tree(lstate, new_tree);
    1
}

extern "C-unwind" fn tree_get_ranges(lstate: *mut lua_State) -> i32 {
    let ud = luaL_checkudata_typed::<TSLuaTree>(lstate, 1, TS_META_TREE);
    let include_bytes = nlua_gettop(lstate) >= 2 && lua_toboolean_safe(lstate, 2);

    let ranges = ts_tree_included_ranges_owned(ud.tree);
    push_ranges(lstate, ranges.as_slice(), include_bytes);
    1
}

extern "C-unwind" fn tree_gc(lstate: *mut lua_State) -> i32 {
    let ud = luaL_checkudata_typed::<TSLuaTree>(lstate, 1, TS_META_TREE);
    ts_tree_free(ud.tree.cast_mut());
    0
}

extern "C-unwind" fn tree_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"<tree>");
    1
}

extern "C-unwind" fn tree_root(lstate: *mut lua_State) -> i32 {
    let ud = luaL_checkudata_typed::<TSLuaTree>(lstate, 1, TS_META_TREE);
    let root = ts_tree_root(ud.tree);

    let _ = lua_newuserdata_typed(lstate, root);
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_NODE);
    let _ = lua_setmetatable_safe(lstate, -2);

    lua_createtable_safe(lstate, 1, 0);
    lua_pushvalue_safe(lstate, 1);
    lua_rawseti_safe(lstate, -2, 1);
    let _ = lua_setfenv_safe(lstate, -2);

    1
}

fn push_node(lstate: *mut lua_State, node: TSNode, uindex: i32) {
    debug_assert!(!(-LUA_MINSTACK..=0).contains(&uindex));
    if node.is_null() {
        lua_pushnil_safe(lstate);
        return;
    }

    let _ = lua_newuserdata_typed(lstate, node);
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_NODE);
    let _ = lua_setmetatable_safe(lstate, -2);

    lua_getfenv_safe(lstate, uindex);
    let _ = lua_setfenv_safe(lstate, -2);
}

fn node_check_opt(lstate: *mut lua_State, index: i32, res: &mut TSNode) -> bool {
    if let Some(ud) = luaL_checkudata_opt_typed::<TSNode>(lstate, index, TS_META_NODE) {
        *res = *ud;
        return true;
    }
    false
}

fn node_check(lstate: *mut lua_State, index: i32) -> TSNode {
    *luaL_checkudata_typed::<TSNode>(lstate, index, TS_META_NODE)
}

extern "C-unwind" fn node_tostring(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushstring(lstate, c"<node ");
    lua_pushstring_opt(lstate, node.type_cstr());
    lua_pushstring(lstate, c">");
    lua_concat_safe(lstate, 3);
    1
}

extern "C-unwind" fn node_eq(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let node2 = node_check(lstate, 2);
    lua_pushboolean_safe(lstate, node.eq_node(node2));
    1
}

extern "C-unwind" fn node_id(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let id_bytes = node.id_ne_bytes();
    lua_push_bytes(lstate, &id_bytes);
    1
}

extern "C-unwind" fn node_range(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let include_bytes = nlua_gettop(lstate) >= 2 && lua_toboolean_safe(lstate, 2);

    let start = node.start_point();
    let end = node.end_point();

    if include_bytes {
        lua_pushinteger_safe(lstate, start.row as lua_Integer);
        lua_pushinteger_safe(lstate, start.column as lua_Integer);
        lua_pushinteger_safe(lstate, node.start_byte() as lua_Integer);
        lua_pushinteger_safe(lstate, end.row as lua_Integer);
        lua_pushinteger_safe(lstate, end.column as lua_Integer);
        lua_pushinteger_safe(lstate, node.end_byte() as lua_Integer);
        return 6;
    }

    lua_pushinteger_safe(lstate, start.row as lua_Integer);
    lua_pushinteger_safe(lstate, start.column as lua_Integer);
    lua_pushinteger_safe(lstate, end.row as lua_Integer);
    lua_pushinteger_safe(lstate, end.column as lua_Integer);
    4
}

extern "C-unwind" fn node_start(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let start = node.start_point();
    let start_byte = node.start_byte();
    lua_pushinteger_safe(lstate, start.row as lua_Integer);
    lua_pushinteger_safe(lstate, start.column as lua_Integer);
    lua_pushinteger_safe(lstate, start_byte as lua_Integer);
    3
}

extern "C-unwind" fn node_end(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let end = node.end_point();
    let end_byte = node.end_byte();
    lua_pushinteger_safe(lstate, end.row as lua_Integer);
    lua_pushinteger_safe(lstate, end.column as lua_Integer);
    lua_pushinteger_safe(lstate, end_byte as lua_Integer);
    3
}

extern "C-unwind" fn node_child_count(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushinteger_safe(lstate, node.child_count() as lua_Integer);
    1
}

extern "C-unwind" fn node_named_child_count(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushinteger_safe(lstate, node.named_child_count() as lua_Integer);
    1
}

extern "C-unwind" fn node_type(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushstring_opt(lstate, node.type_cstr());
    1
}

extern "C-unwind" fn node_symbol(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushinteger_safe(lstate, node.symbol() as lua_Integer);
    1
}

extern "C-unwind" fn node_field(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let count = node.child_count();
    let mut curr_index: i32 = 0;

    let field_name_bytes = luaL_checkbytes(lstate, 2);

    lua_newtable(lstate);

    for i in 0..count {
        if node.field_name_for_child(i).map(CStr::to_bytes) == Some(field_name_bytes) {
            let child = node.child(i);
            push_node(lstate, child, 1);
            curr_index += 1;
            lua_rawseti_safe(lstate, -2, curr_index);
        }
    }

    1
}

extern "C-unwind" fn node_named(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushboolean_safe(lstate, node.is_named());
    1
}

extern "C-unwind" fn node_sexpr(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let allocated = node.sexpr_owned();
    lua_pushstring_opt(lstate, allocated.as_cstr());
    1
}

extern "C-unwind" fn node_missing(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushboolean_safe(lstate, node.is_missing());
    1
}

extern "C-unwind" fn node_extra(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushboolean_safe(lstate, node.is_extra());
    1
}

extern "C-unwind" fn node_has_changes(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushboolean_safe(lstate, node.has_changes());
    1
}

extern "C-unwind" fn node_has_error(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    lua_pushboolean_safe(lstate, node.has_error());
    1
}

extern "C-unwind" fn node_child(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let num = lua_tointeger_safe(lstate, 2) as u32;
    let child = node.child(num);
    push_node(lstate, child, 1);
    1
}

extern "C-unwind" fn node_named_child(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let num = lua_tointeger_safe(lstate, 2) as u32;
    let child = node.named_child(num);
    push_node(lstate, child, 1);
    1
}

extern "C-unwind" fn node_descendant_for_range(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let start = TSPoint {
        row: lua_tointeger_safe(lstate, 2) as u32,
        column: lua_tointeger_safe(lstate, 3) as u32,
    };
    let end = TSPoint {
        row: lua_tointeger_safe(lstate, 4) as u32,
        column: lua_tointeger_safe(lstate, 5) as u32,
    };
    let child = node.descendant_for_point_range(start, end);
    push_node(lstate, child, 1);
    1
}

extern "C-unwind" fn node_named_descendant_for_range(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let start = TSPoint {
        row: lua_tointeger_safe(lstate, 2) as u32,
        column: lua_tointeger_safe(lstate, 3) as u32,
    };
    let end = TSPoint {
        row: lua_tointeger_safe(lstate, 4) as u32,
        column: lua_tointeger_safe(lstate, 5) as u32,
    };
    let child = node.named_descendant_for_point_range(start, end);
    push_node(lstate, child, 1);
    1
}

extern "C-unwind" fn node_next_child(lstate: *mut lua_State) -> i32 {
    let child_index = lua_touserdata_typed::<u32>(lstate, lua_upvalueindex(1));
    let source = node_check(lstate, lua_upvalueindex(2));

    if *child_index >= source.child_count() {
        return 0;
    }

    let child = source.child(*child_index);
    push_node(lstate, child, lua_upvalueindex(2));

    if let Some(field) = source.field_name_for_child(*child_index) {
        lua_pushstring(lstate, field);
    } else {
        lua_pushnil_safe(lstate);
    }

    *child_index += 1;
    2
}

extern "C-unwind" fn node_iter_children(lstate: *mut lua_State) -> i32 {
    node_check(lstate, 1);
    let _ = lua_newuserdata_typed::<u32>(lstate, 0);

    lua_pushvalue_safe(lstate, 1);
    lua_pushcclosure_safe(lstate, node_next_child, 2);
    1
}

extern "C-unwind" fn node_parent(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let parent = node.parent();
    push_node(lstate, parent, 1);
    1
}

extern "C-unwind" fn __has_ancestor(lstate: *mut lua_State) -> i32 {
    let descendant = node_check(lstate, 1);
    if lua_type_safe(lstate, 2) != LUA_TTABLE {
        lua_pushboolean_safe(lstate, false);
        return 1;
    }
    let pred_len = lua_objlen_safe(lstate, 2) as i32;

    let mut node = ts_tree_root(descendant.tree);
    while node.id != descendant.id && !node.is_null() {
        let node_type_bytes = node.type_cstr().map_or(&[][..], CStr::to_bytes);

        for i in 3..=pred_len {
            lua_rawgeti_safe(lstate, 2, i);
            if lua_type_safe(lstate, -1) == LUA_TSTRING {
                let check_bytes = lua_tolstring_bytes(lstate, -1);
                if node_type_bytes == check_bytes {
                    lua_pushboolean_safe(lstate, true);
                    return 1;
                }
            }
            lua_pop(lstate, 1);
        }

        node = node.child_with_descendant(descendant);
    }

    lua_pushboolean_safe(lstate, false);
    1
}

extern "C-unwind" fn node_child_with_descendant(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let descendant = node_check(lstate, 2);
    let child = node.child_with_descendant(descendant);
    push_node(lstate, child, 1);
    1
}

extern "C-unwind" fn node_next_sibling(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let sibling = node.next_sibling();
    push_node(lstate, sibling, 1);
    1
}

extern "C-unwind" fn node_prev_sibling(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let sibling = node.prev_sibling();
    push_node(lstate, sibling, 1);
    1
}

extern "C-unwind" fn node_next_named_sibling(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let sibling = node.next_named_sibling();
    push_node(lstate, sibling, 1);
    1
}

extern "C-unwind" fn node_prev_named_sibling(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let sibling = node.prev_named_sibling();
    push_node(lstate, sibling, 1);
    1
}

extern "C-unwind" fn node_named_children(lstate: *mut lua_State) -> i32 {
    let source = node_check(lstate, 1);

    lua_newtable(lstate);
    let mut curr_index: i32 = 0;

    let n = source.child_count();
    for i in 0..n {
        let child = source.child(i);
        if child.is_named() {
            push_node(lstate, child, 1);
            curr_index += 1;
            lua_rawseti_safe(lstate, -2, curr_index);
        }
    }

    1
}

extern "C-unwind" fn node_root(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let root = ts_tree_root(node.tree);
    push_node(lstate, root, 1);
    1
}

extern "C-unwind" fn node_tree(lstate: *mut lua_State) -> i32 {
    node_check(lstate, 1);
    lua_getfenv_safe(lstate, 1);
    lua_rawgeti_safe(lstate, 2, 1);
    1
}

extern "C-unwind" fn node_byte_length(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    lua_pushinteger_safe(lstate, end_byte.wrapping_sub(start_byte) as lua_Integer);
    1
}

extern "C-unwind" fn node_equal(lstate: *mut lua_State) -> i32 {
    let node1 = node_check(lstate, 1);
    let node2 = node_check(lstate, 2);
    lua_pushboolean_safe(lstate, node1.eq_node(node2));
    1
}

extern "C-unwind" fn tslua_push_querycursor(lstate: *mut lua_State) -> i32 {
    let node = node_check(lstate, 1);
    let query = query_check(lstate, 2);
    let cursor = ts_query_cursor_create();

    if nlua_gettop(lstate) >= 3 && !lua_isnil(lstate, 3) {
        luaL_argcheck(lstate, lua_istable(lstate, 3), 3, c"table expected");
    }

    lua_getfield(lstate, 3, c"start_row");
    let start_row = luaL_checkinteger_safe(lstate, -1) as u32;
    lua_pop(lstate, 1);

    lua_getfield(lstate, 3, c"start_col");
    let start_col = luaL_checkinteger_safe(lstate, -1) as u32;
    lua_pop(lstate, 1);

    lua_getfield(lstate, 3, c"end_row");
    let end_row = luaL_checkinteger_safe(lstate, -1) as u32;
    lua_pop(lstate, 1);

    lua_getfield(lstate, 3, c"end_col");
    let end_col = luaL_checkinteger_safe(lstate, -1) as u32;
    lua_pop(lstate, 1);

    lua_getfield(lstate, 3, c"max_start_depth");
    let max_start_depth =
        (!lua_isnil(lstate, -1)).then(|| luaL_checkinteger_safe(lstate, -1) as u32);
    lua_pop(lstate, 1);

    lua_getfield(lstate, 3, c"match_limit");
    let match_limit = (!lua_isnil(lstate, -1)).then(|| luaL_checkinteger_safe(lstate, -1) as u32);
    lua_pop(lstate, 1);

    ts_query_cursor_configure_and_exec(
        cursor,
        query,
        node,
        TSPoint {
            row: start_row,
            column: start_col,
        },
        TSPoint {
            row: end_row,
            column: end_col,
        },
        max_start_depth,
        match_limit,
    );

    let _ = lua_newuserdata_typed(lstate, cursor);
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERYCURSOR);
    let _ = lua_setmetatable_safe(lstate, -2);

    lua_getfenv_safe(lstate, 1);
    let _ = lua_setfenv_safe(lstate, -2);

    1
}

extern "C-unwind" fn querycursor_remove_match(lstate: *mut lua_State) -> i32 {
    let cursor = querycursor_check(lstate, 1);
    let match_id = luaL_checkinteger_safe(lstate, 2) as u32;
    ts_query_cursor_remove_match_safe(cursor, match_id);
    0
}

extern "C-unwind" fn querycursor_next_capture(lstate: *mut lua_State) -> i32 {
    let cursor = querycursor_check(lstate, 1);
    let Some((match_, capture)) = ts_query_cursor_next_capture_safe(cursor) else {
        return 0;
    };

    lua_pushinteger_safe(lstate, (capture.index + 1) as lua_Integer);
    push_node(lstate, capture.node, 1);
    push_querymatch(lstate, &match_, 1);

    3
}

extern "C-unwind" fn querycursor_next_match(lstate: *mut lua_State) -> i32 {
    let cursor = querycursor_check(lstate, 1);
    let Some(match_) = ts_query_cursor_next_match_safe(cursor) else {
        return 0;
    };

    push_querymatch(lstate, &match_, 1);
    1
}

fn querycursor_check(lstate: *mut lua_State, index: i32) -> *mut TSQueryCursor {
    let ud = luaL_checkudata_typed::<*mut TSQueryCursor>(lstate, index, TS_META_QUERYCURSOR);
    luaL_argcheck(lstate, !(*ud).is_null(), index, c"TSQueryCursor expected");
    *ud
}

extern "C-unwind" fn querycursor_gc(lstate: *mut lua_State) -> i32 {
    let cursor = querycursor_check(lstate, 1);
    ts_query_cursor_free(cursor);
    0
}

fn push_querymatch(lstate: *mut lua_State, match_: &TSQueryMatch, uindex: i32) {
    let _ = lua_newuserdata_typed(lstate, *match_);
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERYMATCH);
    let _ = lua_setmetatable_safe(lstate, -2);

    lua_getfenv_safe(lstate, uindex);
    let _ = lua_setfenv_safe(lstate, -2);
}

extern "C-unwind" fn querymatch_info(lstate: *mut lua_State) -> i32 {
    let match_ = luaL_checkudata_typed::<TSQueryMatch>(lstate, 1, TS_META_QUERYMATCH);
    lua_pushinteger_safe(lstate, match_.id as lua_Integer);
    lua_pushinteger_safe(lstate, (match_.pattern_index + 1) as lua_Integer);
    2
}

extern "C-unwind" fn querymatch_captures(lstate: *mut lua_State) -> i32 {
    let match_ = luaL_checkudata_typed::<TSQueryMatch>(lstate, 1, TS_META_QUERYMATCH);
    lua_newtable(lstate);
    for &capture in match_.captures_slice() {
        let index = (capture.index + 1) as i32;

        lua_rawgeti_safe(lstate, -1, index);
        if lua_isnil(lstate, -1) {
            lua_pop(lstate, 1);
            lua_newtable(lstate);
        }
        push_node(lstate, capture.node, 1);
        lua_rawseti_safe(lstate, -2, (lua_objlen_safe(lstate, -2) + 1) as i32);
        lua_rawseti_safe(lstate, -2, index);
    }
    1
}

extern "C-unwind" fn tslua_parse_query(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) < 2 || !lua_isstring_safe(lstate, 1) || !lua_isstring_safe(lstate, 2) {
        return luaL_error_cstr(lstate, c"string expected");
    }

    let lang = lang_check(lstate, 1);
    let src_bytes = lua_tolstring_bytes(lstate, 2);

    g_st().tslua_query_parse_count += 1;
    let query = match ts_query_create(lang, src_bytes) {
        Ok(q) => q,
        Err((error_offset, error_type)) => {
            let err_cstr = query_err_string(src_bytes, error_offset, error_type);
            return luaL_error_cstr(lstate, &err_cstr);
        }
    };

    let _ = lua_newuserdata_typed(lstate, query);
    lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERY);
    let _ = lua_setmetatable_safe(lstate, -2);
    1
}

const fn query_err_to_string(error_type: TSQueryError) -> &'static str {
    match error_type {
        TSQUERY_ERROR_SYNTAX => "Invalid syntax:\n",
        TSQUERY_ERROR_NODE_TYPE => "Invalid node type ",
        TSQUERY_ERROR_FIELD => "Invalid field name ",
        TSQUERY_ERROR_CAPTURE => "Invalid capture name ",
        TSQUERY_ERROR_STRUCTURE => "Impossible pattern:\n",
        _ => "error",
    }
}

fn query_err_string(
    src: &[u8],
    error_offset: usize,
    error_type: TSQueryError,
) -> std::ffi::CString {
    use std::io::Write as _;
    let mut line_start: usize = 0;
    let mut row: usize = 0;
    let mut error_line: Option<&[u8]> = None;

    loop {
        let src_tmp = &src[line_start.min(src.len())..];
        let nl_pos = src_tmp.iter().position(|&b| b == b'\n');
        let line_length = nl_pos.unwrap_or(src_tmp.len());
        let line_end = line_start + line_length;
        if line_end > error_offset {
            error_line = Some(&src_tmp[..line_length]);
            break;
        }
        line_start = line_end + 1;
        row += 1;
        if nl_pos.is_none() {
            break;
        }
    }

    let column = error_offset.saturating_sub(line_start);
    let type_msg = query_err_to_string(error_type);
    let mut out = Vec::with_capacity(128);
    let _ = write!(out, "Query error at {}:{}. {type_msg}", row + 1, column + 1);

    if error_type == TSQUERY_ERROR_NODE_TYPE
        || error_type == TSQUERY_ERROR_FIELD
        || error_type == TSQUERY_ERROR_CAPTURE
    {
        let suffix = &src[error_offset.min(src.len())..];
        let is_anonymous = error_type == TSQUERY_ERROR_NODE_TYPE
            && error_offset > 0
            && src.get(error_offset - 1) == Some(&b'"');
        let mut suffix_len: usize = 0;
        if is_anonymous {
            let mut backslashes: i32 = 0;
            while suffix_len < suffix.len() {
                let c = suffix[suffix_len];
                if c == b'"' && backslashes % 2 == 0 {
                    break;
                }
                if c == b'\\' {
                    backslashes += 1;
                } else {
                    backslashes = 0;
                }
                suffix_len += 1;
            }
        } else {
            while suffix_len < suffix.len() {
                let c = suffix[suffix_len];
                if !(c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.') {
                    break;
                }
                suffix_len += 1;
            }
        }
        out.push(b'"');
        out.extend_from_slice(&suffix[..suffix_len]);
        out.extend_from_slice(b"\":\n");
    }

    if let Some(line) = error_line {
        out.extend_from_slice(line);
        out.push(b'\n');
        out.resize(out.len() + column, b' ');
        out.extend_from_slice(b"^\n");
    } else {
        out.extend_from_slice(b"Unexpected EOF\n");
    }

    if out.len() >= IOSIZE {
        out.truncate(IOSIZE - 1);
    }
    let nul_pos = out.iter().position(|&b| b == 0).unwrap_or(out.len());
    out.truncate(nul_pos);
    std::ffi::CString::new(out).unwrap_or_default()
}

fn query_check(lstate: *mut lua_State, index: i32) -> *mut TSQuery {
    let ud = luaL_checkudata_typed::<*mut TSQuery>(lstate, index, TS_META_QUERY);
    luaL_argcheck(lstate, !(*ud).is_null(), index, c"TSQuery expected");
    *ud
}

extern "C-unwind" fn query_gc(lstate: *mut lua_State) -> i32 {
    let query = query_check(lstate, 1);
    ts_query_free(query);
    0
}

extern "C-unwind" fn query_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"<query>");
    1
}

extern "C-unwind" fn query_inspect(lstate: *mut lua_State) -> i32 {
    let query = query_check(lstate, 1);

    lua_createtable_safe(lstate, 0, 2);

    let n_pat = ts_query_pattern_count_safe(query);
    lua_createtable_safe(lstate, n_pat as i32, 1);
    for i in 0..(n_pat as usize) {
        let steps = ts_query_predicates_slice(query, i as u32);
        if steps.is_empty() {
            continue;
        }
        lua_createtable_safe(lstate, (steps.len() / 4) as i32, 1);
        lua_createtable_safe(lstate, 3, 0);
        let mut nextpred: i32 = 1;
        let mut nextitem: i32 = 1;
        for &s in steps {
            if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_DONE {
                lua_rawseti_safe(lstate, -2, nextpred);
                nextpred += 1;
                lua_createtable_safe(lstate, 3, 0);
                nextitem = 1;
                continue;
            }

            if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_STRING {
                let bytes = ts_query_string_bytes(query, s.value_id);
                lua_push_bytes(lstate, bytes);
            } else if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_CAPTURE {
                lua_pushinteger_safe(lstate, (s.value_id + 1) as lua_Integer);
            } else {
                std::process::abort();
            }
            lua_rawseti_safe(lstate, -2, nextitem);
            nextitem += 1;
        }
        lua_pop(lstate, 1);
        lua_rawseti_safe(lstate, -2, (i + 1) as i32);
    }
    lua_setfield(lstate, -2, c"patterns");

    let n_captures = ts_query_capture_count_safe(query);
    lua_createtable_safe(lstate, n_captures as i32, 0);
    for i in 0..(n_captures as usize) {
        let bytes = ts_query_capture_name_bytes(query, i as u32);
        lua_push_bytes(lstate, bytes);
        lua_rawseti_safe(lstate, -2, (i + 1) as i32);
    }
    lua_setfield(lstate, -2, c"captures");

    1
}

extern "C-unwind" fn query_disable_capture(lstate: *mut lua_State) -> i32 {
    let query = query_check(lstate, 1);
    let name_bytes = luaL_checkbytes(lstate, 2);
    ts_query_disable_capture_bytes(query, name_bytes);
    0
}

extern "C-unwind" fn query_disable_pattern(lstate: *mut lua_State) -> i32 {
    let query = query_check(lstate, 1);
    let pattern_index = luaL_checkinteger_safe(lstate, 2) as u32;
    ts_query_disable_pattern_safe(query, pattern_index.wrapping_sub(1));
    0
}

fn build_meta(
    lstate: *mut lua_State,
    tname: &CStr,
    meta: &[(&'static CStr, extern "C-unwind" fn(*mut lua_State) -> i32)],
) {
    if luaL_newmetatable(lstate, tname) != 0 {
        lua_register_funcs(lstate, meta);
        lua_pushvalue_safe(lstate, -1);
        lua_setfield(lstate, -2, c"__index");
    }
    lua_pop(lstate, 1);
}

fn tslua_init(lstate: *mut lua_State) {
    build_meta(lstate, TS_META_PARSER, &PARSER_META);
    build_meta(lstate, TS_META_TREE, &TREE_META);
    build_meta(lstate, TS_META_NODE, &NODE_META);
    build_meta(lstate, TS_META_QUERY, &QUERY_META);
    build_meta(lstate, TS_META_QUERYCURSOR, &QUERYCURSOR_META);
    build_meta(lstate, TS_META_QUERYMATCH, &QUERYMATCH_META);

    ts_init_allocator();
}

extern "C-unwind" fn tslua_get_language_version(lstate: *mut lua_State) -> i32 {
    lua_pushnumber_safe(lstate, TREE_SITTER_LANGUAGE_VERSION as lua_Number);
    1
}

extern "C-unwind" fn tslua_get_minimum_language_version(lstate: *mut lua_State) -> i32 {
    lua_pushnumber_safe(
        lstate,
        TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION as lua_Number,
    );
    1
}

pub fn nlua_treesitter_init(lstate: *mut lua_State) {
    tslua_init(lstate);

    lua_pushcfunction_safe(lstate, tslua_push_parser);
    lua_setfield(lstate, -2, c"_create_ts_parser");

    lua_pushcfunction_safe(lstate, tslua_push_querycursor);
    lua_setfield(lstate, -2, c"_create_ts_querycursor");

    lua_pushcfunction_safe(lstate, tslua_add_language_from_object);
    lua_setfield(lstate, -2, c"_ts_add_language_from_object");

    lua_pushcfunction_safe(lstate, tslua_has_language);
    lua_setfield(lstate, -2, c"_ts_has_language");

    lua_pushcfunction_safe(lstate, tslua_remove_lang);
    lua_setfield(lstate, -2, c"_ts_remove_language");

    lua_pushcfunction_safe(lstate, tslua_inspect_lang);
    lua_setfield(lstate, -2, c"_ts_inspect_language");

    lua_pushcfunction_safe(lstate, tslua_parse_query);
    lua_setfield(lstate, -2, c"_ts_parse_query");

    lua_pushcfunction_safe(lstate, tslua_get_language_version);
    lua_setfield(lstate, -2, c"_ts_get_language_version");

    lua_pushcfunction_safe(lstate, tslua_get_minimum_language_version);
    lua_setfield(lstate, -2, c"_ts_get_minimum_language_version");
}
