//! Lua bindings for Tree-sitter (`src/nvim/lua/treesitter.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

#[cfg(not(test))]
use crate::context::handle_get_buffer;
#[cfg(not(test))]
use crate::cursor::linenr_T;
#[cfg(not(test))]
use crate::eval::deprecated::buf_T;
#[cfg(not(test))]
use crate::event::r#loop::{uv_dlclose, uv_dlerror, uv_dlopen, uv_dlsym};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::lua::base64::lua_CFunction;
use crate::lua::base64::lua_State;
#[cfg(not(test))]
use crate::lua::base64::luaL_Reg;
#[cfg(not(test))]
use crate::lua::converter::LuaRef;
#[cfg(not(test))]
use crate::lua::converter::{
    lua_Integer, lua_Number, lua_concat, lua_createtable, lua_error, lua_getfield, lua_gettop,
    lua_isstring, lua_newuserdata, lua_objlen, lua_pcall, lua_pushboolean, lua_pushcclosure,
    lua_pushinteger, lua_pushlstring, lua_pushnil, lua_pushnumber, lua_pushstring, lua_pushvalue,
    lua_rawgeti, lua_rawseti, lua_setfield, lua_setmetatable, lua_settop, lua_toboolean,
    lua_tointeger, lua_tolstring, lua_touserdata, lua_type, luaL_argerror, luaL_checkinteger,
    luaL_checklstring, luaL_checkudata, luaL_error, luaL_newmetatable, luaL_ref, luaL_register,
    luaL_unref,
};
#[cfg(not(test))]
use crate::map::{Map, Set, cstr_t, mh_get_cstr_t, ptr_t};
#[cfg(not(test))]
use crate::map::{map_del_cstr_t_ptr_t, map_put_ref_cstr_t_ptr_t};
#[cfg(not(test))]
use crate::map_glyph_cache::{MH_TOMBSTONE, MapHash};
#[cfg(not(test))]
use crate::memline::{ml_get_buf, ml_get_buf_len};
#[cfg(not(test))]
use crate::memory::{strequal, xcalloc, xfree, xmalloc, xrealloc, xstrdup, xstrlcpy};
#[cfg(not(test))]
use crate::msgpack_rpc::packer::handle_T;
#[cfg(not(test))]
use crate::os::dl::UvLib;
#[cfg(not(test))]
use crate::os::time::os_hrtime;
#[cfg(not(test))]
use crate::strings::vim_snprintf;
#[cfg(not(test))]
use std::ffi::CStr;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

pub type TSSymbol = u16;
pub type TSFieldId = u16;
pub type TSLanguage = c_void;
pub type TSParser = c_void;
pub type TSTree = c_void;
pub type TSQuery = c_void;
pub type TSQueryCursor = c_void;

pub type TSInputEncoding = c_int;
pub type TSSymbolType = c_int;
pub type TSLogType = c_int;
pub type TSQueryPredicateStepType = c_int;
pub type TSQueryError = c_int;

pub type TSDecodeFunction =
    Option<unsafe extern "C" fn(string: *const u8, length: u32, code_point: *mut i32) -> u32>;

#[cfg(not(test))]
const NUL: c_char = 0;
#[cfg(not(test))]
const IOSIZE: usize = 1025;
#[cfg(not(test))]
const BUFSIZE: usize = 256;

#[cfg(not(test))]
const TREE_SITTER_LANGUAGE_VERSION: u32 = 15;
#[cfg(not(test))]
const TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION: u32 = 13;

#[cfg(not(test))]
const TSInputEncodingUTF8: TSInputEncoding = 0;

#[cfg(not(test))]
const TSSymbolTypeAnonymous: TSSymbolType = 1;
#[cfg(not(test))]
const TSSymbolTypeAuxiliary: TSSymbolType = 3;

#[cfg(not(test))]
const TSLogTypeParse: TSLogType = 0;
#[cfg(not(test))]
const TSLogTypeLex: TSLogType = 1;

#[cfg(not(test))]
const TSQueryPredicateStepTypeDone: TSQueryPredicateStepType = 0;
#[cfg(not(test))]
const TSQueryPredicateStepTypeCapture: TSQueryPredicateStepType = 1;
#[cfg(not(test))]
const TSQueryPredicateStepTypeString: TSQueryPredicateStepType = 2;

#[cfg(not(test))]
const TSQueryErrorNone: TSQueryError = 0;
#[cfg(not(test))]
const TSQueryErrorSyntax: TSQueryError = 1;
#[cfg(not(test))]
const TSQueryErrorNodeType: TSQueryError = 2;
#[cfg(not(test))]
const TSQueryErrorField: TSQueryError = 3;
#[cfg(not(test))]
const TSQueryErrorCapture: TSQueryError = 4;
#[cfg(not(test))]
const TSQueryErrorStructure: TSQueryError = 5;

#[cfg(not(test))]
const LUA_REGISTRYINDEX: c_int = -10000;
#[cfg(not(test))]
const LUA_GLOBALSINDEX: c_int = -10002;
#[cfg(not(test))]
const LUA_MINSTACK: c_int = 20;

#[cfg(not(test))]
const LUA_TNIL: c_int = 0;
#[cfg(not(test))]
const LUA_TBOOLEAN: c_int = 1;
#[cfg(not(test))]
const LUA_TNUMBER: c_int = 3;
#[cfg(not(test))]
const LUA_TSTRING: c_int = 4;
#[cfg(not(test))]
const LUA_TTABLE: c_int = 5;
#[cfg(not(test))]
const LUA_TFUNCTION: c_int = 6;

#[cfg(not(test))]
const TS_META_PARSER: *const c_char = c"treesitter_parser".as_ptr();
#[cfg(not(test))]
const TS_META_TREE: *const c_char = c"treesitter_tree".as_ptr();
#[cfg(not(test))]
const TS_META_NODE: *const c_char = c"treesitter_node".as_ptr();
#[cfg(not(test))]
const TS_META_QUERY: *const c_char = c"treesitter_query".as_ptr();
#[cfg(not(test))]
const TS_META_QUERYCURSOR: *const c_char = c"treesitter_querycursor".as_ptr();
#[cfg(not(test))]
const TS_META_QUERYMATCH: *const c_char = c"treesitter_querymatch".as_ptr();

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSPoint {
    pub row: u32,
    pub column: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSRange {
    pub start_point: TSPoint,
    pub end_point: TSPoint,
    pub start_byte: u32,
    pub end_byte: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSInput {
    pub payload: *mut c_void,
    pub read: Option<
        unsafe extern "C" fn(
            payload: *mut c_void,
            byte_index: u32,
            position: TSPoint,
            bytes_read: *mut u32,
        ) -> *const c_char,
    >,
    pub encoding: TSInputEncoding,
    pub decode: TSDecodeFunction,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSParseState {
    pub payload: *mut c_void,
    pub current_byte_offset: u32,
    pub has_error: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSParseOptions {
    pub payload: *mut c_void,
    pub progress_callback: Option<unsafe extern "C" fn(state: *mut TSParseState) -> bool>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSLogger {
    pub payload: *mut c_void,
    pub log: Option<
        unsafe extern "C" fn(payload: *mut c_void, log_type: TSLogType, buffer: *const c_char),
    >,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSInputEdit {
    pub start_byte: u32,
    pub old_end_byte: u32,
    pub new_end_byte: u32,
    pub start_point: TSPoint,
    pub old_end_point: TSPoint,
    pub new_end_point: TSPoint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSNode {
    pub context: [u32; 4],
    pub id: *const c_void,
    pub tree: *const TSTree,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryCapture {
    pub node: TSNode,
    pub index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryMatch {
    pub id: u32,
    pub pattern_index: u16,
    pub capture_count: u16,
    pub captures: *const TSQueryCapture,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryPredicateStep {
    pub type_: TSQueryPredicateStepType,
    pub value_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSLanguageMetadata {
    pub major_version: u8,
    pub minor_version: u8,
    pub patch_version: u8,
}

#[cfg(not(test))]
#[repr(C)]
struct TSLuaLoggerOpts {
    cb: LuaRef,
    lstate: *mut lua_State,
    lex: bool,
    parse: bool,
}

#[cfg(not(test))]
#[repr(C)]
struct TSLuaTree {
    tree: *const TSTree,
}

#[cfg(not(test))]
#[repr(C)]
struct TSLuaParserCallbackPayload {
    parse_start_time: u64,
    timeout_threshold_ns: u64,
}

#[cfg(not(test))]
static mut LANGS: Map<cstr_t, ptr_t> = Map {
    set: Set {
        h: MapHash {
            n_buckets: 0,
            size: 0,
            n_occupied: 0,
            upper_bound: 0,
            n_keys: 0,
            keys_capacity: 0,
            hash: ptr::null_mut(),
        },
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

#[cfg(not(test))]
static mut INPUT_BUF: [c_char; BUFSIZE] = [0; BUFSIZE];

#[cfg(not(test))]
static PARSER_META: [luaL_Reg; 9] = [
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(parser_gc),
    },
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(parser_tostring),
    },
    luaL_Reg {
        name: c"parse".as_ptr(),
        func: Some(parser_parse),
    },
    luaL_Reg {
        name: c"reset".as_ptr(),
        func: Some(parser_reset),
    },
    luaL_Reg {
        name: c"set_included_ranges".as_ptr(),
        func: Some(parser_set_ranges),
    },
    luaL_Reg {
        name: c"included_ranges".as_ptr(),
        func: Some(parser_get_ranges),
    },
    luaL_Reg {
        name: c"_set_logger".as_ptr(),
        func: Some(parser_set_logger),
    },
    luaL_Reg {
        name: c"_logger".as_ptr(),
        func: Some(parser_get_logger),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
static TREE_META: [luaL_Reg; 7] = [
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(tree_gc),
    },
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(tree_tostring),
    },
    luaL_Reg {
        name: c"root".as_ptr(),
        func: Some(tree_root),
    },
    luaL_Reg {
        name: c"edit".as_ptr(),
        func: Some(tree_edit),
    },
    luaL_Reg {
        name: c"included_ranges".as_ptr(),
        func: Some(tree_get_ranges),
    },
    luaL_Reg {
        name: c"copy".as_ptr(),
        func: Some(tree_copy),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
static NODE_META: [luaL_Reg; 36] = [
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(node_tostring),
    },
    luaL_Reg {
        name: c"__eq".as_ptr(),
        func: Some(node_eq),
    },
    luaL_Reg {
        name: c"__len".as_ptr(),
        func: Some(node_child_count),
    },
    luaL_Reg {
        name: c"id".as_ptr(),
        func: Some(node_id),
    },
    luaL_Reg {
        name: c"range".as_ptr(),
        func: Some(node_range),
    },
    luaL_Reg {
        name: c"start".as_ptr(),
        func: Some(node_start),
    },
    luaL_Reg {
        name: c"end_".as_ptr(),
        func: Some(node_end),
    },
    luaL_Reg {
        name: c"type".as_ptr(),
        func: Some(node_type),
    },
    luaL_Reg {
        name: c"symbol".as_ptr(),
        func: Some(node_symbol),
    },
    luaL_Reg {
        name: c"field".as_ptr(),
        func: Some(node_field),
    },
    luaL_Reg {
        name: c"named".as_ptr(),
        func: Some(node_named),
    },
    luaL_Reg {
        name: c"missing".as_ptr(),
        func: Some(node_missing),
    },
    luaL_Reg {
        name: c"extra".as_ptr(),
        func: Some(node_extra),
    },
    luaL_Reg {
        name: c"has_changes".as_ptr(),
        func: Some(node_has_changes),
    },
    luaL_Reg {
        name: c"has_error".as_ptr(),
        func: Some(node_has_error),
    },
    luaL_Reg {
        name: c"sexpr".as_ptr(),
        func: Some(node_sexpr),
    },
    luaL_Reg {
        name: c"child_count".as_ptr(),
        func: Some(node_child_count),
    },
    luaL_Reg {
        name: c"named_child_count".as_ptr(),
        func: Some(node_named_child_count),
    },
    luaL_Reg {
        name: c"child".as_ptr(),
        func: Some(node_child),
    },
    luaL_Reg {
        name: c"named_child".as_ptr(),
        func: Some(node_named_child),
    },
    luaL_Reg {
        name: c"descendant_for_range".as_ptr(),
        func: Some(node_descendant_for_range),
    },
    luaL_Reg {
        name: c"named_descendant_for_range".as_ptr(),
        func: Some(node_named_descendant_for_range),
    },
    luaL_Reg {
        name: c"parent".as_ptr(),
        func: Some(node_parent),
    },
    luaL_Reg {
        name: c"__has_ancestor".as_ptr(),
        func: Some(__has_ancestor),
    },
    luaL_Reg {
        name: c"child_with_descendant".as_ptr(),
        func: Some(node_child_with_descendant),
    },
    luaL_Reg {
        name: c"iter_children".as_ptr(),
        func: Some(node_iter_children),
    },
    luaL_Reg {
        name: c"next_sibling".as_ptr(),
        func: Some(node_next_sibling),
    },
    luaL_Reg {
        name: c"prev_sibling".as_ptr(),
        func: Some(node_prev_sibling),
    },
    luaL_Reg {
        name: c"next_named_sibling".as_ptr(),
        func: Some(node_next_named_sibling),
    },
    luaL_Reg {
        name: c"prev_named_sibling".as_ptr(),
        func: Some(node_prev_named_sibling),
    },
    luaL_Reg {
        name: c"named_children".as_ptr(),
        func: Some(node_named_children),
    },
    luaL_Reg {
        name: c"root".as_ptr(),
        func: Some(node_root),
    },
    luaL_Reg {
        name: c"tree".as_ptr(),
        func: Some(node_tree),
    },
    luaL_Reg {
        name: c"byte_length".as_ptr(),
        func: Some(node_byte_length),
    },
    luaL_Reg {
        name: c"equal".as_ptr(),
        func: Some(node_equal),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
static QUERYCURSOR_META: [luaL_Reg; 5] = [
    luaL_Reg {
        name: c"remove_match".as_ptr(),
        func: Some(querycursor_remove_match),
    },
    luaL_Reg {
        name: c"next_capture".as_ptr(),
        func: Some(querycursor_next_capture),
    },
    luaL_Reg {
        name: c"next_match".as_ptr(),
        func: Some(querycursor_next_match),
    },
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(querycursor_gc),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
static QUERYMATCH_META: [luaL_Reg; 3] = [
    luaL_Reg {
        name: c"info".as_ptr(),
        func: Some(querymatch_info),
    },
    luaL_Reg {
        name: c"captures".as_ptr(),
        func: Some(querymatch_captures),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
static QUERY_META: [luaL_Reg; 6] = [
    luaL_Reg {
        name: c"__gc".as_ptr(),
        func: Some(query_gc),
    },
    luaL_Reg {
        name: c"__tostring".as_ptr(),
        func: Some(query_tostring),
    },
    luaL_Reg {
        name: c"inspect".as_ptr(),
        func: Some(query_inspect),
    },
    luaL_Reg {
        name: c"disable_capture".as_ptr(),
        func: Some(query_disable_capture),
    },
    luaL_Reg {
        name: c"disable_pattern".as_ptr(),
        func: Some(query_disable_pattern),
    },
    luaL_Reg {
        name: ptr::null(),
        func: None,
    },
];

#[cfg(not(test))]
unsafe extern "C-unwind" {
    fn lua_getfenv(L: *mut lua_State, idx: c_int);
    fn lua_setfenv(L: *mut lua_State, idx: c_int) -> c_int;
    fn luaL_checknumber(L: *mut lua_State, numArg: c_int) -> lua_Number;

    fn ts_parser_parse(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
    ) -> *mut TSTree;
    fn ts_parser_parse_with_options(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
        parse_options: TSParseOptions,
    ) -> *mut TSTree;
}

#[cfg(not(test))]
unsafe extern "C" {

    fn ts_parser_new() -> *mut TSParser;
    fn ts_parser_delete(self_: *mut TSParser);
    fn ts_parser_language(self_: *const TSParser) -> *const TSLanguage;
    fn ts_parser_set_language(self_: *mut TSParser, language: *const TSLanguage) -> bool;
    fn ts_parser_set_included_ranges(
        self_: *mut TSParser,
        ranges: *const TSRange,
        count: u32,
    ) -> bool;
    fn ts_parser_included_ranges(self_: *const TSParser, count: *mut u32) -> *const TSRange;
    fn ts_parser_parse_string(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        string: *const c_char,
        length: u32,
    ) -> *mut TSTree;
    fn ts_parser_reset(self_: *mut TSParser);
    fn ts_parser_set_logger(self_: *mut TSParser, logger: TSLogger);
    fn ts_parser_logger(self_: *const TSParser) -> TSLogger;

    fn ts_tree_copy(self_: *const TSTree) -> *mut TSTree;
    fn ts_tree_delete(self_: *mut TSTree);
    fn ts_tree_root_node(self_: *const TSTree) -> TSNode;
    fn ts_tree_included_ranges(self_: *const TSTree, length: *mut u32) -> *mut TSRange;
    fn ts_tree_edit(self_: *mut TSTree, edit: *const TSInputEdit);
    fn ts_tree_get_changed_ranges(
        old_tree: *const TSTree,
        new_tree: *const TSTree,
        length: *mut u32,
    ) -> *mut TSRange;

    fn ts_node_type(self_: TSNode) -> *const c_char;
    fn ts_node_symbol(self_: TSNode) -> TSSymbol;
    fn ts_node_start_byte(self_: TSNode) -> u32;
    fn ts_node_start_point(self_: TSNode) -> TSPoint;
    fn ts_node_end_byte(self_: TSNode) -> u32;
    fn ts_node_end_point(self_: TSNode) -> TSPoint;
    fn ts_node_string(self_: TSNode) -> *mut c_char;
    fn ts_node_is_null(self_: TSNode) -> bool;
    fn ts_node_is_named(self_: TSNode) -> bool;
    fn ts_node_is_missing(self_: TSNode) -> bool;
    fn ts_node_is_extra(self_: TSNode) -> bool;
    fn ts_node_has_changes(self_: TSNode) -> bool;
    fn ts_node_has_error(self_: TSNode) -> bool;
    fn ts_node_parent(self_: TSNode) -> TSNode;
    fn ts_node_child_with_descendant(self_: TSNode, descendant: TSNode) -> TSNode;
    fn ts_node_child(self_: TSNode, child_index: u32) -> TSNode;
    fn ts_node_field_name_for_child(self_: TSNode, child_index: u32) -> *const c_char;
    fn ts_node_child_count(self_: TSNode) -> u32;
    fn ts_node_named_child(self_: TSNode, child_index: u32) -> TSNode;
    fn ts_node_named_child_count(self_: TSNode) -> u32;
    fn ts_node_next_sibling(self_: TSNode) -> TSNode;
    fn ts_node_prev_sibling(self_: TSNode) -> TSNode;
    fn ts_node_next_named_sibling(self_: TSNode) -> TSNode;
    fn ts_node_prev_named_sibling(self_: TSNode) -> TSNode;
    fn ts_node_descendant_for_point_range(self_: TSNode, start: TSPoint, end: TSPoint) -> TSNode;
    fn ts_node_named_descendant_for_point_range(
        self_: TSNode,
        start: TSPoint,
        end: TSPoint,
    ) -> TSNode;
    fn ts_node_eq(self_: TSNode, other: TSNode) -> bool;

    fn ts_query_new(
        language: *const TSLanguage,
        source: *const c_char,
        source_len: u32,
        error_offset: *mut u32,
        error_type: *mut TSQueryError,
    ) -> *mut TSQuery;
    fn ts_query_delete(self_: *mut TSQuery);
    fn ts_query_pattern_count(self_: *const TSQuery) -> u32;
    fn ts_query_capture_count(self_: *const TSQuery) -> u32;
    fn ts_query_predicates_for_pattern(
        self_: *const TSQuery,
        pattern_index: u32,
        step_count: *mut u32,
    ) -> *const TSQueryPredicateStep;
    fn ts_query_capture_name_for_id(
        self_: *const TSQuery,
        index: u32,
        length: *mut u32,
    ) -> *const c_char;
    fn ts_query_string_value_for_id(
        self_: *const TSQuery,
        index: u32,
        length: *mut u32,
    ) -> *const c_char;
    fn ts_query_disable_capture(self_: *mut TSQuery, name: *const c_char, length: u32);
    fn ts_query_disable_pattern(self_: *mut TSQuery, pattern_index: u32);

    fn ts_query_cursor_new() -> *mut TSQueryCursor;
    fn ts_query_cursor_delete(self_: *mut TSQueryCursor);
    fn ts_query_cursor_exec(self_: *mut TSQueryCursor, query: *const TSQuery, node: TSNode);
    fn ts_query_cursor_set_match_limit(self_: *mut TSQueryCursor, limit: u32);
    fn ts_query_cursor_set_point_range(
        self_: *mut TSQueryCursor,
        start_point: TSPoint,
        end_point: TSPoint,
    ) -> bool;
    fn ts_query_cursor_next_match(self_: *mut TSQueryCursor, match_: *mut TSQueryMatch) -> bool;
    fn ts_query_cursor_remove_match(self_: *mut TSQueryCursor, match_id: u32);
    fn ts_query_cursor_next_capture(
        self_: *mut TSQueryCursor,
        match_: *mut TSQueryMatch,
        capture_index: *mut u32,
    ) -> bool;
    fn ts_query_cursor_set_max_start_depth(self_: *mut TSQueryCursor, max_start_depth: u32);

    fn ts_language_symbol_count(self_: *const TSLanguage) -> u32;
    fn ts_language_state_count(self_: *const TSLanguage) -> u32;
    fn ts_language_field_count(self_: *const TSLanguage) -> u32;
    fn ts_language_field_name_for_id(self_: *const TSLanguage, id: TSFieldId) -> *const c_char;
    fn ts_language_supertypes(self_: *const TSLanguage, length: *mut u32) -> *const TSSymbol;
    fn ts_language_subtypes(
        self_: *const TSLanguage,
        supertype: TSSymbol,
        length: *mut u32,
    ) -> *const TSSymbol;
    fn ts_language_symbol_name(self_: *const TSLanguage, symbol: TSSymbol) -> *const c_char;
    fn ts_language_symbol_type(self_: *const TSLanguage, symbol: TSSymbol) -> TSSymbolType;
    fn ts_language_abi_version(self_: *const TSLanguage) -> u32;
    fn ts_language_metadata(self_: *const TSLanguage) -> *const TSLanguageMetadata;
    fn ts_language_is_wasm(self_: *const TSLanguage) -> bool;

    fn ts_set_allocator(
        new_malloc: Option<unsafe extern "C" fn(usize) -> *mut c_void>,
        new_calloc: Option<unsafe extern "C" fn(usize, usize) -> *mut c_void>,
        new_realloc: Option<unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void>,
        new_free: Option<unsafe extern "C" fn(*mut c_void)>,
    );
}

#[cfg(not(test))]
#[inline]
const fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_GLOBALSINDEX - i
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_pop(L: *mut lua_State, n: c_int) {
    unsafe {
        lua_settop(L, -n - 1);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_newtable(L: *mut lua_State) {
    unsafe {
        lua_createtable(L, 0, 0);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_pushcfunction(L: *mut lua_State, f: lua_CFunction) {
    unsafe {
        lua_pushcclosure(L, f, 0);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isnil(L: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(L, n) == LUA_TNIL }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isboolean(L: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(L, n) == LUA_TBOOLEAN }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_istable(L: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(L, n) == LUA_TTABLE }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isfunction(L: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(L, n) == LUA_TFUNCTION }
}

#[cfg(not(test))]
#[inline]
unsafe fn luaL_checkstring(L: *mut lua_State, n: c_int) -> *const c_char {
    unsafe { luaL_checklstring(L, n, ptr::null_mut()) }
}

#[cfg(not(test))]
#[inline]
unsafe fn luaL_checkint(L: *mut lua_State, n: c_int) -> c_int {
    unsafe { luaL_checkinteger(L, n) as c_int }
}

#[cfg(not(test))]
#[inline]
unsafe fn luaL_argcheck(L: *mut lua_State, cond: bool, numarg: c_int, extramsg: *const c_char) {
    if !cond {
        unsafe {
            luaL_argerror(L, numarg, extramsg);
        }
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn langs_has(lang_name: *const c_char) -> bool {
    unsafe { mh_get_cstr_t(&raw mut LANGS.set, lang_name) != MH_TOMBSTONE }
}

#[cfg(not(test))]
#[inline]
unsafe fn langs_get(lang_name: *const c_char) -> *mut TSLanguage {
    unsafe {
        let k = mh_get_cstr_t(&raw mut LANGS.set, lang_name);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*LANGS.values.add(k as usize)).cast::<TSLanguage>()
        }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_has_language(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let lang_name = luaL_checkstring(L, 1);
        lua_pushboolean(L, c_int::from(langs_has(lang_name)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_add_language_from_object(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        add_language(L, false)
    }
}

#[cfg(target_os = "emscripten")]
unsafe extern "C" {
    fn tree_sitter_lua() -> *const TSLanguage;
    fn tree_sitter_c() -> *const TSLanguage;
    fn tree_sitter_diff() -> *const TSLanguage;
    fn tree_sitter_vim() -> *const TSLanguage;
    fn tree_sitter_markdown() -> *const TSLanguage;
    fn tree_sitter_markdown_inline() -> *const TSLanguage;
    fn tree_sitter_query() -> *const TSLanguage;
    fn tree_sitter_vimdoc() -> *const TSLanguage;
}

#[cfg(target_os = "emscripten")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ts_get_parser(lang: *const c_char) -> *const TSLanguage {
    if lang.is_null() {
        return ptr::null();
    }
    unsafe {
        match CStr::from_ptr(lang).to_bytes() {
            b"lua" => tree_sitter_lua(),
            b"c" => tree_sitter_c(),
            b"diff" => tree_sitter_diff(),
            b"vim" => tree_sitter_vim(),
            b"markdown" => tree_sitter_markdown(),
            b"markdown_inline" => tree_sitter_markdown_inline(),
            b"query" => tree_sitter_query(),
            b"vimdoc" => tree_sitter_vimdoc(),
            _ => ptr::null(),
        }
    }
}

#[cfg(not(test))]
unsafe fn load_language_from_object(
    L: *mut lua_State,
    path: *const c_char,
    lang_name: *const c_char,
    symbol: *const c_char,
) -> *const TSLanguage {
    unsafe {
        #[cfg(target_os = "emscripten")]
        {
            let static_lang = nvim_ts_get_parser(symbol);
            if !static_lang.is_null() {
                return static_lang;
            }
        }

        let iobuff = (&raw mut IObuff).cast::<c_char>();
        let mut lib = UvLib {
            _opaque: [ptr::null_mut(); 4],
        };
        if uv_dlopen(path, &mut lib) != 0 {
            xstrlcpy(iobuff, uv_dlerror(&lib), IOSIZE);
            uv_dlclose(&mut lib);
            luaL_error(
                L,
                c"Failed to load parser for language '%s': uv_dlopen: %s".as_ptr(),
                lang_name,
                iobuff,
            );
        }

        let mut symbol_buf = [0 as c_char; 128];
        vim_snprintf!(
            symbol_buf.as_mut_ptr(),
            symbol_buf.len(),
            c"tree_sitter_%s".as_ptr(),
            symbol,
        );

        let mut fn_ptr: *mut c_void = ptr::null_mut();
        if uv_dlsym(&mut lib, symbol_buf.as_ptr(), &mut fn_ptr) != 0 {
            xstrlcpy(iobuff, uv_dlerror(&lib), IOSIZE);
            uv_dlclose(&mut lib);
            luaL_error(L, c"Failed to load parser: uv_dlsym: %s".as_ptr(), iobuff);
        }

        let lang_parser: unsafe extern "C" fn() -> *mut TSLanguage = std::mem::transmute(fn_ptr);
        let lang = lang_parser();

        if lang.is_null() {
            uv_dlclose(&mut lib);
            luaL_error(
                L,
                c"Failed to load parser %s: internal error".as_ptr(),
                path,
            );
        }

        lang
    }
}

#[cfg(not(test))]
unsafe fn load_language_from_wasm(
    L: *mut lua_State,
    _path: *const c_char,
    _lang_name: *const c_char,
) -> *const TSLanguage {
    unsafe {
        luaL_error(L, c"Not supported".as_ptr());
        ptr::null()
    }
}

#[cfg(not(test))]
unsafe fn add_language(L: *mut lua_State, is_wasm: bool) -> c_int {
    unsafe {
        let path = luaL_checkstring(L, 1);
        let lang_name = luaL_checkstring(L, 2);
        let mut symbol_name = lang_name;

        if !is_wasm && lua_gettop(L) >= 3 && !lua_isnil(L, 3) {
            symbol_name = luaL_checkstring(L, 3);
        }

        if langs_has(lang_name) {
            lua_pushboolean(L, 1);
            return 1;
        }

        let lang = if is_wasm {
            load_language_from_wasm(L, path, lang_name)
        } else {
            load_language_from_object(L, path, lang_name, symbol_name)
        };

        let lang_version = ts_language_abi_version(lang);
        if !(TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION..=TREE_SITTER_LANGUAGE_VERSION)
            .contains(&lang_version)
        {
            return luaL_error(
                L,
                c"ABI version mismatch for %s: supported between %d and %d, found %d".as_ptr(),
                path,
                TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION as c_int,
                TREE_SITTER_LANGUAGE_VERSION as c_int,
                lang_version as c_int,
            );
        }

        let val_slot = map_put_ref_cstr_t_ptr_t(
            &raw mut LANGS,
            xstrdup(lang_name),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        *val_slot = lang.cast_mut();

        lua_pushboolean(L, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_remove_lang(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let lang_name = luaL_checkstring(L, 1);
        let present = langs_has(lang_name);
        if present {
            let mut key: cstr_t = ptr::null();
            map_del_cstr_t_ptr_t(&raw mut LANGS, lang_name, &mut key);
            xfree(key.cast_mut().cast::<c_void>());
        }
        lua_pushboolean(L, c_int::from(present));
        1
    }
}

#[cfg(not(test))]
unsafe fn lang_check(L: *mut lua_State, index: c_int) -> *mut TSLanguage {
    unsafe {
        let lang_name = luaL_checkstring(L, index);
        let lang = langs_get(lang_name);
        if lang.is_null() {
            luaL_error(L, c"no such language: %s".as_ptr(), lang_name);
        }
        lang
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_inspect_lang(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let lang = lang_check(L, 1);

        lua_createtable(L, 0, 2);

        {
            let nsymbols = ts_language_symbol_count(lang);
            debug_assert!(nsymbols < c_int::MAX as u32);

            lua_createtable(L, (nsymbols.wrapping_sub(1)) as c_int, 1);
            for i in 0..nsymbols {
                let t = ts_language_symbol_type(lang, i as TSSymbol);
                if t == TSSymbolTypeAuxiliary {
                    continue;
                }
                let name = ts_language_symbol_name(lang, i as TSSymbol);
                let named = t != TSSymbolTypeAnonymous;
                lua_pushboolean(L, c_int::from(named));
                if !named {
                    let mut buf = [0 as c_char; 256];
                    vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"\"%s\"".as_ptr(), name);
                    lua_setfield(L, -2, buf.as_ptr());
                } else {
                    lua_setfield(L, -2, name);
                }
            }

            lua_setfield(L, -2, c"symbols".as_ptr());
        }

        {
            let nfields = ts_language_field_count(lang);
            lua_createtable(L, nfields as c_int, 1);
            for i in 1..=nfields {
                lua_pushstring(L, ts_language_field_name_for_id(lang, i as TSFieldId));
                lua_rawseti(L, -2, i as c_int);
            }

            lua_setfield(L, -2, c"fields".as_ptr());
        }

        lua_pushboolean(L, c_int::from(ts_language_is_wasm(lang)));
        lua_setfield(L, -2, c"_wasm".as_ptr());

        lua_pushinteger(L, ts_language_abi_version(lang) as lua_Integer);
        lua_setfield(L, -2, c"abi_version".as_ptr());

        {
            let meta = ts_language_metadata(lang);
            if !meta.is_null() {
                lua_createtable(L, 0, 3);

                lua_pushinteger(L, (*meta).major_version as lua_Integer);
                lua_setfield(L, -2, c"major_version".as_ptr());
                lua_pushinteger(L, (*meta).minor_version as lua_Integer);
                lua_setfield(L, -2, c"minor_version".as_ptr());
                lua_pushinteger(L, (*meta).patch_version as lua_Integer);
                lua_setfield(L, -2, c"patch_version".as_ptr());

                lua_setfield(L, -2, c"metadata".as_ptr());
            }
        }

        lua_pushinteger(L, ts_language_state_count(lang) as lua_Integer);
        lua_setfield(L, -2, c"state_count".as_ptr());

        {
            let mut nsupertypes: u32 = 0;
            let supertypes = ts_language_supertypes(lang, &mut nsupertypes);

            lua_createtable(L, 0, nsupertypes as c_int);
            for i in 0..nsupertypes {
                let supertype = *supertypes.add(i as usize);

                let mut nsubtypes: u32 = 0;
                let subtypes = ts_language_subtypes(lang, supertype, &mut nsubtypes);

                lua_createtable(L, nsubtypes as c_int, 0);
                for j in 1..=nsubtypes {
                    lua_pushstring(L, ts_language_symbol_name(lang, *subtypes.add(j as usize)));
                    lua_rawseti(L, -2, j as c_int);
                }

                lua_setfield(L, -2, ts_language_symbol_name(lang, supertype));
            }

            lua_setfield(L, -2, c"supertypes".as_ptr());
        }

        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_push_parser(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let lang = lang_check(L, 1);

        let parser = lua_newuserdata(L, size_of::<*mut TSParser>()).cast::<*mut TSParser>();
        *parser = ts_parser_new();

        if !ts_parser_set_language(*parser, lang) {
            ts_parser_delete(*parser);
            let lang_name = luaL_checkstring(L, 1);
            return luaL_error(L, c"Failed to load language : %s".as_ptr(), lang_name);
        }

        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_PARSER);
        lua_setmetatable(L, -2);
        1
    }
}

#[cfg(not(test))]
unsafe fn parser_check(L: *mut lua_State, index: c_int) -> *mut TSParser {
    unsafe {
        let ud = luaL_checkudata(L, index, TS_META_PARSER).cast::<*mut TSParser>();
        luaL_argcheck(
            L,
            !(*ud).is_null(),
            index,
            c"Parser has been deleted".as_ptr(),
        );
        *ud
    }
}

#[cfg(not(test))]
unsafe fn logger_gc(logger: TSLogger) {
    if logger.log.is_none() {
        return;
    }

    unsafe {
        let opts = logger.payload.cast::<TSLuaLoggerOpts>();
        luaL_unref((*opts).lstate, LUA_REGISTRYINDEX, (*opts).cb);
        xfree(opts.cast::<c_void>());
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_gc(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let ud = luaL_checkudata(L, 1, TS_META_PARSER).cast::<*mut TSParser>();
        if !(*ud).is_null() {
            logger_gc(ts_parser_logger(*ud));
            ts_parser_delete(*ud);
            *ud = ptr::null_mut();
        }
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_tostring(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        lua_pushstring(L, c"<parser>".as_ptr());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C" fn input_cb(
    payload: *mut c_void,
    _byte_index: u32,
    position: TSPoint,
    bytes_read: *mut u32,
) -> *const c_char {
    #[cfg(test)]
    {
        let _ = (payload, position, bytes_read);
        c"".as_ptr()
    }
    #[cfg(not(test))]
    unsafe {
        let bp = payload.cast::<buf_T>();
        let buf_ptr = (&raw mut INPUT_BUF).cast::<c_char>();

        if (position.row as linenr_T) >= (*bp).b_ml.ml_line_count {
            *bytes_read = 0;
            return c"".as_ptr();
        }
        let lnum = (position.row as linenr_T) + 1;
        let line = ml_get_buf(bp, lnum);
        let len = ml_get_buf_len(bp, lnum) as usize;
        let col = position.column as usize;
        if col > len {
            *bytes_read = 0;
            return c"".as_ptr();
        }
        let tocopy = (len - col).min(BUFSIZE);

        ptr::copy_nonoverlapping(line.add(col), buf_ptr, tocopy);
        for i in 0..tocopy {
            let p = buf_ptr.add(i);
            if *p == b'\n' as c_char {
                *p = NUL;
            }
        }
        *bytes_read = tocopy as u32;
        if tocopy < BUFSIZE
            && (lnum != (*bp).b_ml.ml_line_count
                || ((*bp).b_p_bin == 0 && (*bp).b_p_fixeol != 0)
                || (lnum != (*bp).b_no_eol_lnum && (*bp).b_p_eol != 0))
        {
            *buf_ptr.add(tocopy) = b'\n' as c_char;
            *bytes_read += 1;
        }
        buf_ptr
    }
}

#[cfg(not(test))]
unsafe fn push_ranges(
    L: *mut lua_State,
    ranges: *const TSRange,
    length: usize,
    include_bytes: bool,
) {
    unsafe {
        lua_createtable(L, length as c_int, 0);
        for i in 0..length {
            let r = *ranges.add(i);
            lua_createtable(L, if include_bytes { 6 } else { 4 }, 0);
            let mut j: c_int = 1;
            lua_pushnumber(L, r.start_point.row as lua_Number);
            lua_rawseti(L, -2, j);
            j += 1;
            lua_pushnumber(L, r.start_point.column as lua_Number);
            lua_rawseti(L, -2, j);
            j += 1;
            if include_bytes {
                lua_pushnumber(L, r.start_byte as lua_Number);
                lua_rawseti(L, -2, j);
                j += 1;
            }
            lua_pushnumber(L, r.end_point.row as lua_Number);
            lua_rawseti(L, -2, j);
            j += 1;
            lua_pushnumber(L, r.end_point.column as lua_Number);
            lua_rawseti(L, -2, j);
            j += 1;
            if include_bytes {
                lua_pushnumber(L, r.end_byte as lua_Number);
                lua_rawseti(L, -2, j);
            }

            lua_rawseti(L, -2, (i + 1) as c_int);
        }
    }
}

#[cfg(not(test))]
unsafe extern "C" fn on_parser_progress(state: *mut TSParseState) -> bool {
    unsafe {
        let payload = (*state).payload.cast::<TSLuaParserCallbackPayload>();
        let parse_time = os_hrtime().wrapping_sub((*payload).parse_start_time);
        parse_time >= (*payload).timeout_threshold_ns
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nts_parser_parse_buf(
    p: *mut TSParser,
    old_tree: *const TSTree,
    bufnr: c_int,
    timeout_ns: u64,
) -> *mut TSTree {
    #[cfg(test)]
    {
        let _ = (p, old_tree, bufnr, timeout_ns);
        ptr::null_mut()
    }
    #[cfg(not(test))]
    unsafe {
        let buf = handle_get_buffer(bufnr as handle_T);
        if buf.is_null() {
            std::process::abort();
        }

        let input = TSInput {
            payload: buf.cast::<c_void>(),
            read: Some(input_cb),
            encoding: TSInputEncodingUTF8,
            decode: None,
        };

        if timeout_ns == 0 {
            return ts_parser_parse(p, old_tree, input);
        }

        let mut payload = TSLuaParserCallbackPayload {
            parse_start_time: os_hrtime(),
            timeout_threshold_ns: timeout_ns,
        };

        let parse_options = TSParseOptions {
            payload: (&raw mut payload).cast::<c_void>(),
            progress_callback: Some(on_parser_progress),
        };

        ts_parser_parse_with_options(p, old_tree, input, parse_options)
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_parse(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let p = parser_check(L, 1);
        let mut old_tree: *const TSTree = ptr::null();
        if !lua_isnil(L, 2) {
            let ud = luaL_checkudata(L, 2, TS_META_TREE).cast::<TSLuaTree>();
            old_tree = if !ud.is_null() {
                (*ud).tree
            } else {
                ptr::null()
            };
        }

        let new_tree: *mut TSTree = match lua_type(L, 3) {
            LUA_TSTRING => {
                let mut len: usize = 0;
                let str_ptr = lua_tolstring(L, 3, &mut len);
                ts_parser_parse_string(p, old_tree, str_ptr, len as u32)
            }
            LUA_TNUMBER => {
                let bufnr = lua_tointeger(L, 3) as handle_T;
                let buf = handle_get_buffer(bufnr);
                if buf.is_null() {
                    let mut ebuf = [0 as c_char; IOSIZE];
                    vim_snprintf!(
                        ebuf.as_mut_ptr(),
                        IOSIZE,
                        c"invalid buffer handle: %d".as_ptr(),
                        bufnr,
                    );
                    return luaL_argerror(L, 3, ebuf.as_ptr());
                }

                let timeout_ns = if lua_isnil(L, 5) {
                    0
                } else {
                    lua_tointeger(L, 5) as u64
                };
                nts_parser_parse_buf(p, old_tree, bufnr, timeout_ns)
            }
            _ => {
                return luaL_argerror(L, 3, c"expected either string or buffer handle".as_ptr());
            }
        };

        let include_bytes = lua_gettop(L) >= 4 && lua_toboolean(L, 4) != 0;

        if new_tree.is_null() {
            if ts_parser_language(p).is_null() {
                return luaL_error(
                    L,
                    c"Language was unset, or has an incompatible ABI.".as_ptr(),
                );
            }
            return 0;
        }

        let mut n_ranges: u32 = 0;
        let changed = if !old_tree.is_null() {
            ts_tree_get_changed_ranges(old_tree, new_tree, &mut n_ranges)
        } else {
            ts_tree_included_ranges(new_tree, &mut n_ranges)
        };

        push_tree(L, new_tree);
        push_ranges(L, changed, n_ranges as usize, include_bytes);

        xfree(changed.cast::<c_void>());
        2
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_reset(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let p = parser_check(L, 1);
        ts_parser_reset(p);
        0
    }
}

#[cfg(not(test))]
unsafe fn range_err(L: *mut lua_State) {
    unsafe {
        luaL_error(
            L,
            c"Ranges can only be made from 6 element long tables or nodes.".as_ptr(),
        );
    }
}

#[cfg(not(test))]
unsafe fn lua_checkuint32(L: *mut lua_State, index: c_int) -> u32 {
    unsafe {
        let value = luaL_checknumber(L, index);
        let converted = value as u32;
        if value < 0.0 || value > (u32::MAX as lua_Number) || (converted as lua_Number) != value {
            luaL_error(L, c"Range value out of bounds".as_ptr());
        }
        converted
    }
}

#[cfg(not(test))]
unsafe fn range_from_lua(L: *mut lua_State, range: *mut TSRange) {
    unsafe {
        let mut node = TSNode {
            context: [0; 4],
            id: ptr::null(),
            tree: ptr::null(),
        };

        if lua_istable(L, -1) {
            if lua_objlen(L, -1) != 6 {
                range_err(L);
            }

            lua_rawgeti(L, -1, 1);
            let start_row = lua_checkuint32(L, -1);
            lua_pop(L, 1);

            lua_rawgeti(L, -1, 2);
            let start_col = lua_checkuint32(L, -1);
            lua_pop(L, 1);

            lua_rawgeti(L, -1, 3);
            let start_byte = lua_checkuint32(L, -1);
            lua_pop(L, 1);

            lua_rawgeti(L, -1, 4);
            let end_row = lua_checkuint32(L, -1);
            lua_pop(L, 1);

            lua_rawgeti(L, -1, 5);
            let end_col = lua_checkuint32(L, -1);
            lua_pop(L, 1);

            lua_rawgeti(L, -1, 6);
            let end_byte = lua_checkuint32(L, -1);
            lua_pop(L, 1);

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
        } else if node_check_opt(L, -1, &mut node) {
            *range = TSRange {
                start_point: ts_node_start_point(node),
                end_point: ts_node_end_point(node),
                start_byte: ts_node_start_byte(node),
                end_byte: ts_node_end_byte(node),
            };
        } else {
            range_err(L);
        }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_set_ranges(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        if lua_gettop(L) < 2 {
            return luaL_error(
                L,
                c"not enough args to parser:set_included_ranges()".as_ptr(),
            );
        }

        let p = parser_check(L, 1);
        luaL_argcheck(L, lua_istable(L, 2), 2, c"table expected.".as_ptr());

        let tbl_len = lua_objlen(L, 2);
        let ranges = xmalloc(size_of::<TSRange>() * tbl_len).cast::<TSRange>();

        for index in 0..tbl_len {
            lua_rawgeti(L, 2, (index + 1) as c_int);
            range_from_lua(L, ranges.add(index));
            lua_pop(L, 1);
        }

        ts_parser_set_included_ranges(p, ranges, tbl_len as u32);
        xfree(ranges.cast::<c_void>());

        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_get_ranges(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let p = parser_check(L, 1);
        let include_bytes = lua_gettop(L) >= 2 && lua_toboolean(L, 2) != 0;

        let mut len: u32 = 0;
        let ranges = ts_parser_included_ranges(p, &mut len);

        push_ranges(L, ranges, len as usize, include_bytes);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C" fn logger_cb(payload: *mut c_void, logtype: TSLogType, s: *const c_char) {
    #[cfg(test)]
    {
        let _ = (payload, logtype, s);
    }
    #[cfg(not(test))]
    unsafe {
        let opts = payload.cast::<TSLuaLoggerOpts>();
        if (!(*opts).lex && logtype == TSLogTypeLex)
            || (!(*opts).parse && logtype == TSLogTypeParse)
        {
            return;
        }

        let lstate = (*opts).lstate;
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, (*opts).cb);
        lua_pushstring(
            lstate,
            if logtype == TSLogTypeParse {
                c"parse".as_ptr()
            } else {
                c"lex".as_ptr()
            },
        );
        lua_pushstring(lstate, s);
        if lua_pcall(lstate, 2, 0, 0) != 0 {
            luaL_error(lstate, c"treesitter logger callback failed".as_ptr());
        }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_set_logger(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let p = parser_check(L, 1);

        luaL_argcheck(L, lua_isboolean(L, 2), 2, c"boolean expected".as_ptr());
        luaL_argcheck(L, lua_isboolean(L, 3), 3, c"boolean expected".as_ptr());
        luaL_argcheck(L, lua_isfunction(L, 4), 4, c"function expected".as_ptr());

        let opts = xmalloc(size_of::<TSLuaLoggerOpts>()).cast::<TSLuaLoggerOpts>();
        lua_pushvalue(L, 4);
        let ref_ = luaL_ref(L, LUA_REGISTRYINDEX);

        *opts = TSLuaLoggerOpts {
            lex: lua_toboolean(L, 2) != 0,
            parse: lua_toboolean(L, 3) != 0,
            cb: ref_,
            lstate: L,
        };

        let logger = TSLogger {
            payload: opts.cast::<c_void>(),
            log: Some(logger_cb),
        };

        ts_parser_set_logger(p, logger);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn parser_get_logger(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let p = parser_check(L, 1);
        let logger = ts_parser_logger(p);
        if logger.log.is_some() {
            let opts = logger.payload.cast::<TSLuaLoggerOpts>();
            lua_rawgeti(L, LUA_REGISTRYINDEX, (*opts).cb);
        } else {
            lua_pushnil(L);
        }
        1
    }
}

#[cfg(not(test))]
unsafe fn push_tree(L: *mut lua_State, tree: *const TSTree) {
    unsafe {
        if tree.is_null() {
            lua_pushnil(L);
            return;
        }

        let ud = lua_newuserdata(L, size_of::<TSLuaTree>()).cast::<TSLuaTree>();
        (*ud).tree = tree;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_TREE);
        lua_setmetatable(L, -2);
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_copy(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let ud = luaL_checkudata(L, 1, TS_META_TREE).cast::<TSLuaTree>();
        let copy = ts_tree_copy((*ud).tree);
        push_tree(L, copy);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_edit(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        if lua_gettop(L) < 10 {
            lua_pushstring(L, c"not enough args to tree:edit()".as_ptr());
            return lua_error(L);
        }

        let ud = luaL_checkudata(L, 1, TS_META_TREE).cast::<TSLuaTree>();

        let start_byte = luaL_checkint(L, 2) as u32;
        let old_end_byte = luaL_checkint(L, 3) as u32;
        let new_end_byte = luaL_checkint(L, 4) as u32;
        let start_point = TSPoint {
            row: luaL_checkint(L, 5) as u32,
            column: luaL_checkint(L, 6) as u32,
        };
        let old_end_point = TSPoint {
            row: luaL_checkint(L, 7) as u32,
            column: luaL_checkint(L, 8) as u32,
        };
        let new_end_point = TSPoint {
            row: luaL_checkint(L, 9) as u32,
            column: luaL_checkint(L, 10) as u32,
        };

        let edit = TSInputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point,
        };

        let new_tree = ts_tree_copy((*ud).tree);
        ts_tree_edit(new_tree, &edit);

        push_tree(L, new_tree);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_get_ranges(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let ud = luaL_checkudata(L, 1, TS_META_TREE).cast::<TSLuaTree>();
        let include_bytes = lua_gettop(L) >= 2 && lua_toboolean(L, 2) != 0;

        let mut len: u32 = 0;
        let ranges = ts_tree_included_ranges((*ud).tree, &mut len);

        push_ranges(L, ranges, len as usize, include_bytes);
        xfree(ranges.cast::<c_void>());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_gc(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let ud = luaL_checkudata(L, 1, TS_META_TREE).cast::<TSLuaTree>();
        let tree = (*ud).tree.cast_mut();
        ts_tree_delete(tree);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_tostring(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        lua_pushstring(L, c"<tree>".as_ptr());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tree_root(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let ud = luaL_checkudata(L, 1, TS_META_TREE).cast::<TSLuaTree>();
        let root = ts_tree_root_node((*ud).tree);

        let node_ud = lua_newuserdata(L, size_of::<TSNode>()).cast::<TSNode>();
        *node_ud = root;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_NODE);
        lua_setmetatable(L, -2);

        lua_createtable(L, 1, 0);
        lua_pushvalue(L, 1);
        lua_rawseti(L, -2, 1);
        lua_setfenv(L, -2);

        1
    }
}

#[cfg(not(test))]
unsafe fn push_node(L: *mut lua_State, node: TSNode, uindex: c_int) {
    unsafe {
        debug_assert!(!(-LUA_MINSTACK..=0).contains(&uindex));
        if ts_node_is_null(node) {
            lua_pushnil(L);
            return;
        }

        let ud = lua_newuserdata(L, size_of::<TSNode>()).cast::<TSNode>();
        *ud = node;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_NODE);
        lua_setmetatable(L, -2);

        lua_getfenv(L, uindex);
        lua_setfenv(L, -2);
    }
}

#[cfg(not(test))]
unsafe fn node_check_opt(L: *mut lua_State, index: c_int, res: *mut TSNode) -> bool {
    unsafe {
        let ud = luaL_checkudata(L, index, TS_META_NODE).cast::<TSNode>();
        if !ud.is_null() {
            *res = *ud;
            return true;
        }
        false
    }
}

#[cfg(not(test))]
unsafe fn node_check(L: *mut lua_State, index: c_int) -> TSNode {
    unsafe {
        let ud = luaL_checkudata(L, index, TS_META_NODE).cast::<TSNode>();
        *ud
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_tostring(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushstring(L, c"<node ".as_ptr());
        lua_pushstring(L, ts_node_type(node));
        lua_pushstring(L, c">".as_ptr());
        lua_concat(L, 3);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_eq(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let node2 = node_check(L, 2);
        lua_pushboolean(L, c_int::from(ts_node_eq(node, node2)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_id(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushlstring(
            L,
            (&raw const node.id).cast::<c_char>(),
            size_of_val(&node.id),
        );
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_range(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let include_bytes = lua_gettop(L) >= 2 && lua_toboolean(L, 2) != 0;

        let start = ts_node_start_point(node);
        let end = ts_node_end_point(node);

        if include_bytes {
            lua_pushinteger(L, start.row as lua_Integer);
            lua_pushinteger(L, start.column as lua_Integer);
            lua_pushinteger(L, ts_node_start_byte(node) as lua_Integer);
            lua_pushinteger(L, end.row as lua_Integer);
            lua_pushinteger(L, end.column as lua_Integer);
            lua_pushinteger(L, ts_node_end_byte(node) as lua_Integer);
            return 6;
        }

        lua_pushinteger(L, start.row as lua_Integer);
        lua_pushinteger(L, start.column as lua_Integer);
        lua_pushinteger(L, end.row as lua_Integer);
        lua_pushinteger(L, end.column as lua_Integer);
        4
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_start(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let start = ts_node_start_point(node);
        let start_byte = ts_node_start_byte(node);
        lua_pushinteger(L, start.row as lua_Integer);
        lua_pushinteger(L, start.column as lua_Integer);
        lua_pushinteger(L, start_byte as lua_Integer);
        3
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_end(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let end = ts_node_end_point(node);
        let end_byte = ts_node_end_byte(node);
        lua_pushinteger(L, end.row as lua_Integer);
        lua_pushinteger(L, end.column as lua_Integer);
        lua_pushinteger(L, end_byte as lua_Integer);
        3
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_child_count(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let count = ts_node_child_count(node);
        lua_pushinteger(L, count as lua_Integer);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_named_child_count(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let count = ts_node_named_child_count(node);
        lua_pushinteger(L, count as lua_Integer);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_type(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushstring(L, ts_node_type(node));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_symbol(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let symbol = ts_node_symbol(node);
        lua_pushinteger(L, symbol as lua_Integer);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_field(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let count = ts_node_child_count(node);
        let mut curr_index: c_int = 0;

        let mut name_len: usize = 0;
        let field_name = luaL_checklstring(L, 2, &mut name_len);

        lua_newtable(L);

        for i in 0..count {
            let child_field_name = ts_node_field_name_for_child(node, i);
            if strequal(field_name, child_field_name) {
                let child = ts_node_child(node, i);
                push_node(L, child, 1);
                curr_index += 1;
                lua_rawseti(L, -2, curr_index);
            }
        }

        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_named(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushboolean(L, c_int::from(ts_node_is_named(node)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_sexpr(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let allocated = ts_node_string(node);
        lua_pushstring(L, allocated);
        xfree(allocated.cast::<c_void>());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_missing(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushboolean(L, c_int::from(ts_node_is_missing(node)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_extra(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushboolean(L, c_int::from(ts_node_is_extra(node)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_has_changes(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushboolean(L, c_int::from(ts_node_has_changes(node)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_has_error(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        lua_pushboolean(L, c_int::from(ts_node_has_error(node)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_child(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let num = lua_tointeger(L, 2) as u32;
        let child = ts_node_child(node, num);
        push_node(L, child, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_named_child(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let num = lua_tointeger(L, 2) as u32;
        let child = ts_node_named_child(node, num);
        push_node(L, child, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_descendant_for_range(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let start = TSPoint {
            row: lua_tointeger(L, 2) as u32,
            column: lua_tointeger(L, 3) as u32,
        };
        let end = TSPoint {
            row: lua_tointeger(L, 4) as u32,
            column: lua_tointeger(L, 5) as u32,
        };
        let child = ts_node_descendant_for_point_range(node, start, end);
        push_node(L, child, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_named_descendant_for_range(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let start = TSPoint {
            row: lua_tointeger(L, 2) as u32,
            column: lua_tointeger(L, 3) as u32,
        };
        let end = TSPoint {
            row: lua_tointeger(L, 4) as u32,
            column: lua_tointeger(L, 5) as u32,
        };
        let child = ts_node_named_descendant_for_point_range(node, start, end);
        push_node(L, child, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_next_child(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let child_index = lua_touserdata(L, lua_upvalueindex(1)).cast::<u32>();
        let source = node_check(L, lua_upvalueindex(2));

        if *child_index >= ts_node_child_count(source) {
            return 0;
        }

        let child = ts_node_child(source, *child_index);
        push_node(L, child, lua_upvalueindex(2));

        let field = ts_node_field_name_for_child(source, *child_index);
        if !field.is_null() {
            lua_pushstring(L, field);
        } else {
            lua_pushnil(L);
        }

        *child_index += 1;
        2
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_iter_children(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        node_check(L, 1);
        let child_index = lua_newuserdata(L, size_of::<u32>()).cast::<u32>();
        *child_index = 0;

        lua_pushvalue(L, 1);
        lua_pushcclosure(L, Some(node_next_child), 2);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_parent(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let parent = ts_node_parent(node);
        push_node(L, parent, 1);
        1
    }
}

unsafe extern "C-unwind" fn __has_ancestor(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let descendant = node_check(L, 1);
        if lua_type(L, 2) != LUA_TTABLE {
            lua_pushboolean(L, 0);
            return 1;
        }
        let pred_len = lua_objlen(L, 2) as c_int;

        let mut node = ts_tree_root_node(descendant.tree);
        while node.id != descendant.id && !ts_node_is_null(node) {
            let node_type = ts_node_type(node);
            let node_type_bytes = CStr::from_ptr(node_type).to_bytes();

            for i in 3..=pred_len {
                lua_rawgeti(L, 2, i);
                if lua_type(L, -1) == LUA_TSTRING {
                    let mut check_len: usize = 0;
                    let check_str = lua_tolstring(L, -1, &mut check_len);
                    let check_bytes = std::slice::from_raw_parts(check_str.cast::<u8>(), check_len);
                    if node_type_bytes == check_bytes {
                        lua_pushboolean(L, 1);
                        return 1;
                    }
                }
                lua_pop(L, 1);
            }

            node = ts_node_child_with_descendant(node, descendant);
        }

        lua_pushboolean(L, 0);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_child_with_descendant(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let descendant = node_check(L, 2);
        let child = ts_node_child_with_descendant(node, descendant);
        push_node(L, child, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_next_sibling(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let sibling = ts_node_next_sibling(node);
        push_node(L, sibling, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_prev_sibling(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let sibling = ts_node_prev_sibling(node);
        push_node(L, sibling, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_next_named_sibling(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let sibling = ts_node_next_named_sibling(node);
        push_node(L, sibling, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_prev_named_sibling(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let sibling = ts_node_prev_named_sibling(node);
        push_node(L, sibling, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_named_children(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let source = node_check(L, 1);

        lua_newtable(L);
        let mut curr_index: c_int = 0;

        let n = ts_node_child_count(source);
        for i in 0..n {
            let child = ts_node_child(source, i);
            if ts_node_is_named(child) {
                push_node(L, child, 1);
                curr_index += 1;
                lua_rawseti(L, -2, curr_index);
            }
        }

        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_root(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let root = ts_tree_root_node(node.tree);
        push_node(L, root, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_tree(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        node_check(L, 1);
        lua_getfenv(L, 1);
        lua_rawgeti(L, 2, 1);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_byte_length(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let start_byte = ts_node_start_byte(node);
        let end_byte = ts_node_end_byte(node);
        lua_pushinteger(L, end_byte.wrapping_sub(start_byte) as lua_Integer);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn node_equal(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node1 = node_check(L, 1);
        let node2 = node_check(L, 2);
        lua_pushboolean(L, c_int::from(ts_node_eq(node1, node2)));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_push_querycursor(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let node = node_check(L, 1);
        let query = query_check(L, 2);
        let cursor = ts_query_cursor_new();

        if lua_gettop(L) >= 3 && !lua_isnil(L, 3) {
            luaL_argcheck(L, lua_istable(L, 3), 3, c"table expected".as_ptr());
        }

        lua_getfield(L, 3, c"start_row".as_ptr());
        let start_row = luaL_checkinteger(L, -1) as u32;
        lua_pop(L, 1);

        lua_getfield(L, 3, c"start_col".as_ptr());
        let start_col = luaL_checkinteger(L, -1) as u32;
        lua_pop(L, 1);

        lua_getfield(L, 3, c"end_row".as_ptr());
        let end_row = luaL_checkinteger(L, -1) as u32;
        lua_pop(L, 1);

        lua_getfield(L, 3, c"end_col".as_ptr());
        let end_col = luaL_checkinteger(L, -1) as u32;
        lua_pop(L, 1);

        ts_query_cursor_set_point_range(
            cursor,
            TSPoint {
                row: start_row,
                column: start_col,
            },
            TSPoint {
                row: end_row,
                column: end_col,
            },
        );

        lua_getfield(L, 3, c"max_start_depth".as_ptr());
        if !lua_isnil(L, -1) {
            let max_start_depth = luaL_checkinteger(L, -1) as u32;
            ts_query_cursor_set_max_start_depth(cursor, max_start_depth);
        }
        lua_pop(L, 1);

        lua_getfield(L, 3, c"match_limit".as_ptr());
        if !lua_isnil(L, -1) {
            let match_limit = luaL_checkinteger(L, -1) as u32;
            ts_query_cursor_set_match_limit(cursor, match_limit);
        }
        lua_pop(L, 1);

        ts_query_cursor_exec(cursor, query, node);

        let ud = lua_newuserdata(L, size_of::<*mut TSQueryCursor>()).cast::<*mut TSQueryCursor>();
        *ud = cursor;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_QUERYCURSOR);
        lua_setmetatable(L, -2);

        lua_getfenv(L, 1);
        lua_setfenv(L, -2);

        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querycursor_remove_match(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let cursor = querycursor_check(L, 1);
        let match_id = luaL_checkinteger(L, 2) as u32;
        ts_query_cursor_remove_match(cursor, match_id);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querycursor_next_capture(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let cursor = querycursor_check(L, 1);
        let mut match_ = TSQueryMatch {
            id: 0,
            pattern_index: 0,
            capture_count: 0,
            captures: ptr::null(),
        };
        let mut capture_index: u32 = 0;
        if !ts_query_cursor_next_capture(cursor, &mut match_, &mut capture_index) {
            return 0;
        }

        let capture = *match_.captures.add(capture_index as usize);
        lua_pushinteger(L, (capture.index + 1) as lua_Integer);
        push_node(L, capture.node, 1);
        push_querymatch(L, &match_, 1);

        3
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querycursor_next_match(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let cursor = querycursor_check(L, 1);
        let mut match_ = TSQueryMatch {
            id: 0,
            pattern_index: 0,
            capture_count: 0,
            captures: ptr::null(),
        };
        if !ts_query_cursor_next_match(cursor, &mut match_) {
            return 0;
        }

        push_querymatch(L, &match_, 1);
        1
    }
}

#[cfg(not(test))]
unsafe fn querycursor_check(L: *mut lua_State, index: c_int) -> *mut TSQueryCursor {
    unsafe {
        let ud = luaL_checkudata(L, index, TS_META_QUERYCURSOR).cast::<*mut TSQueryCursor>();
        luaL_argcheck(
            L,
            !(*ud).is_null(),
            index,
            c"TSQueryCursor expected".as_ptr(),
        );
        *ud
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querycursor_gc(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let cursor = querycursor_check(L, 1);
        ts_query_cursor_delete(cursor);
        0
    }
}

#[cfg(not(test))]
unsafe fn push_querymatch(L: *mut lua_State, match_: *const TSQueryMatch, uindex: c_int) {
    unsafe {
        let ud = lua_newuserdata(L, size_of::<TSQueryMatch>()).cast::<TSQueryMatch>();
        *ud = *match_;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_QUERYMATCH);
        lua_setmetatable(L, -2);

        lua_getfenv(L, uindex);
        lua_setfenv(L, -2);
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querymatch_info(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let match_ = luaL_checkudata(L, 1, TS_META_QUERYMATCH).cast::<TSQueryMatch>();
        lua_pushinteger(L, (*match_).id as lua_Integer);
        lua_pushinteger(L, ((*match_).pattern_index + 1) as lua_Integer);
        2
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn querymatch_captures(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let match_ = luaL_checkudata(L, 1, TS_META_QUERYMATCH).cast::<TSQueryMatch>();
        lua_newtable(L);
        for i in 0..((*match_).capture_count as usize) {
            let capture = *(*match_).captures.add(i);
            let index = (capture.index + 1) as c_int;

            lua_rawgeti(L, -1, index);
            if lua_isnil(L, -1) {
                lua_pop(L, 1);
                lua_newtable(L);
            }
            push_node(L, capture.node, 1);
            lua_rawseti(L, -2, (lua_objlen(L, -2) + 1) as c_int);
            lua_rawseti(L, -2, index);
        }
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_parse_query(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        if lua_gettop(L) < 2 || lua_isstring(L, 1) == 0 || lua_isstring(L, 2) == 0 {
            return luaL_error(L, c"string expected".as_ptr());
        }

        let lang = lang_check(L, 1);

        let mut len: usize = 0;
        let src = lua_tolstring(L, 2, &mut len);

        tslua_query_parse_count += 1;
        let mut error_offset: u32 = 0;
        let mut error_type: TSQueryError = TSQueryErrorNone;
        let query = ts_query_new(lang, src, len as u32, &mut error_offset, &mut error_type);

        if query.is_null() {
            let mut err_buf = [0 as c_char; IOSIZE];
            query_err_string(
                src,
                error_offset as c_int,
                error_type,
                err_buf.as_mut_ptr(),
                err_buf.len(),
            );
            return luaL_error(L, c"%s".as_ptr(), err_buf.as_ptr());
        }

        let ud = lua_newuserdata(L, size_of::<*mut TSQuery>()).cast::<*mut TSQuery>();
        *ud = query;
        lua_getfield(L, LUA_REGISTRYINDEX, TS_META_QUERY);
        lua_setmetatable(L, -2);
        1
    }
}

#[cfg(not(test))]
fn query_err_to_string(error_type: TSQueryError) -> *const c_char {
    match error_type {
        TSQueryErrorSyntax => c"Invalid syntax:\n".as_ptr(),
        TSQueryErrorNodeType => c"Invalid node type ".as_ptr(),
        TSQueryErrorField => c"Invalid field name ".as_ptr(),
        TSQueryErrorCapture => c"Invalid capture name ".as_ptr(),
        TSQueryErrorStructure => c"Impossible pattern:\n".as_ptr(),
        _ => c"error".as_ptr(),
    }
}

#[cfg(not(test))]
unsafe fn query_err_string(
    src: *const c_char,
    error_offset: c_int,
    error_type: TSQueryError,
    mut err_data: *mut c_char,
    mut err_size: usize,
) {
    unsafe {
        let mut line_start: c_int = 0;
        let mut row: c_int = 0;
        let mut error_line: *const c_char = ptr::null();
        let mut error_line_len: c_int = 0;

        loop {
            let src_tmp = src.offset(line_start as isize);
            let src_tmp_bytes = CStr::from_ptr(src_tmp).to_bytes();
            let nl_pos = src_tmp_bytes.iter().position(|&b| b == b'\n');
            let line_length = nl_pos.unwrap_or(src_tmp_bytes.len()) as c_int;
            let line_end = line_start + line_length;
            if line_end > error_offset {
                error_line = src_tmp;
                error_line_len = line_length;
                break;
            }
            line_start = line_end + 1;
            row += 1;
            if nl_pos.is_none() {
                break;
            }
        }

        let column = error_offset - line_start;
        let type_msg = query_err_to_string(error_type);
        vim_snprintf!(
            err_data,
            err_size,
            c"Query error at %d:%d. %s".as_ptr(),
            row + 1,
            column + 1,
            type_msg,
        );
        let offset = CStr::from_ptr(err_data).to_bytes().len();
        err_size = err_size.saturating_sub(offset);
        err_data = err_data.add(offset);

        if error_type == TSQueryErrorNodeType
            || error_type == TSQueryErrorField
            || error_type == TSQueryErrorCapture
        {
            let suffix = src.offset(error_offset as isize);
            let is_anonymous = error_type == TSQueryErrorNodeType
                && error_offset > 0
                && *suffix.offset(-1) == b'"' as c_char;
            let mut suffix_len: usize = 0;
            let mut c = *suffix.add(suffix_len) as u8;
            if is_anonymous {
                let mut backslashes: c_int = 0;
                while c != b'"' || backslashes % 2 != 0 {
                    if c == b'\\' {
                        backslashes += 1;
                    } else {
                        backslashes = 0;
                    }
                    suffix_len += 1;
                    c = *suffix.add(suffix_len) as u8;
                }
            } else {
                while c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' {
                    suffix_len += 1;
                    c = *suffix.add(suffix_len) as u8;
                }
            }
            vim_snprintf!(
                err_data,
                err_size,
                c"\"%.*s\":\n".as_ptr(),
                suffix_len as c_int,
                suffix,
            );
            let offset = CStr::from_ptr(err_data).to_bytes().len();
            err_size = err_size.saturating_sub(offset);
            err_data = err_data.add(offset);
        }

        if error_line.is_null() {
            vim_snprintf!(err_data, err_size, c"Unexpected EOF\n".as_ptr());
            return;
        }

        vim_snprintf!(
            err_data,
            err_size,
            c"%.*s\n%*s^\n".as_ptr(),
            error_line_len,
            error_line,
            column,
            c"".as_ptr(),
        );
    }
}

#[cfg(not(test))]
unsafe fn query_check(L: *mut lua_State, index: c_int) -> *mut TSQuery {
    unsafe {
        let ud = luaL_checkudata(L, index, TS_META_QUERY).cast::<*mut TSQuery>();
        luaL_argcheck(L, !(*ud).is_null(), index, c"TSQuery expected".as_ptr());
        *ud
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn query_gc(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let query = query_check(L, 1);
        ts_query_delete(query);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn query_tostring(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        lua_pushstring(L, c"<query>".as_ptr());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn query_inspect(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let query = query_check(L, 1);

        lua_createtable(L, 0, 2);

        let n_pat = ts_query_pattern_count(query);
        lua_createtable(L, n_pat as c_int, 1);
        for i in 0..(n_pat as usize) {
            let mut len: u32 = 0;
            let step = ts_query_predicates_for_pattern(query, i as u32, &mut len);
            if len == 0 {
                continue;
            }
            lua_createtable(L, (len / 4) as c_int, 1);
            lua_createtable(L, 3, 0);
            let mut nextpred: c_int = 1;
            let mut nextitem: c_int = 1;
            for k in 0..(len as usize) {
                let s = *step.add(k);
                if s.type_ == TSQueryPredicateStepTypeDone {
                    lua_rawseti(L, -2, nextpred);
                    nextpred += 1;
                    lua_createtable(L, 3, 0);
                    nextitem = 1;
                    continue;
                }

                if s.type_ == TSQueryPredicateStepTypeString {
                    let mut strlen: u32 = 0;
                    let str_ptr = ts_query_string_value_for_id(query, s.value_id, &mut strlen);
                    lua_pushlstring(L, str_ptr, strlen as usize);
                } else if s.type_ == TSQueryPredicateStepTypeCapture {
                    lua_pushinteger(L, (s.value_id + 1) as lua_Integer);
                } else {
                    std::process::abort();
                }
                lua_rawseti(L, -2, nextitem);
                nextitem += 1;
            }
            lua_pop(L, 1);
            lua_rawseti(L, -2, (i + 1) as c_int);
        }
        lua_setfield(L, -2, c"patterns".as_ptr());

        let n_captures = ts_query_capture_count(query);
        lua_createtable(L, n_captures as c_int, 0);
        for i in 0..(n_captures as usize) {
            let mut strlen: u32 = 0;
            let str_ptr = ts_query_capture_name_for_id(query, i as u32, &mut strlen);
            lua_pushlstring(L, str_ptr, strlen as usize);
            lua_rawseti(L, -2, (i + 1) as c_int);
        }
        lua_setfield(L, -2, c"captures".as_ptr());

        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn query_disable_capture(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let query = query_check(L, 1);
        let mut name_len: usize = 0;
        let name = luaL_checklstring(L, 2, &mut name_len);
        ts_query_disable_capture(query, name, name_len as u32);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn query_disable_pattern(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        let query = query_check(L, 1);
        let pattern_index = luaL_checkinteger(L, 2) as u32;
        ts_query_disable_pattern(query, pattern_index.wrapping_sub(1));
        0
    }
}

#[cfg(not(test))]
unsafe fn build_meta(L: *mut lua_State, tname: *const c_char, meta: *const luaL_Reg) {
    unsafe {
        if luaL_newmetatable(L, tname) != 0 {
            luaL_register(L, ptr::null(), meta);
            lua_pushvalue(L, -1);
            lua_setfield(L, -2, c"__index".as_ptr());
        }
        lua_pop(L, 1);
    }
}

#[cfg(not(test))]
unsafe fn tslua_init(L: *mut lua_State) {
    unsafe {
        build_meta(L, TS_META_PARSER, PARSER_META.as_ptr());
        build_meta(L, TS_META_TREE, TREE_META.as_ptr());
        build_meta(L, TS_META_NODE, NODE_META.as_ptr());
        build_meta(L, TS_META_QUERY, QUERY_META.as_ptr());
        build_meta(L, TS_META_QUERYCURSOR, QUERYCURSOR_META.as_ptr());
        build_meta(L, TS_META_QUERYMATCH, QUERYMATCH_META.as_ptr());

        ts_set_allocator(Some(xmalloc), Some(xcalloc), Some(xrealloc), Some(xfree));
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_get_language_version(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        lua_pushnumber(L, TREE_SITTER_LANGUAGE_VERSION as lua_Number);
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn tslua_get_minimum_language_version(L: *mut lua_State) -> c_int {
    #[cfg(test)]
    {
        let _ = L;
        0
    }
    #[cfg(not(test))]
    unsafe {
        lua_pushnumber(L, TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION as lua_Number);
        1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nlua_treesitter_free() {}

#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nlua_treesitter_init(lstate: *mut lua_State) {
    #[cfg(test)]
    {
        let _ = lstate;
    }
    #[cfg(not(test))]
    unsafe {
        tslua_init(lstate);

        lua_pushcfunction(lstate, Some(tslua_push_parser));
        lua_setfield(lstate, -2, c"_create_ts_parser".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_push_querycursor));
        lua_setfield(lstate, -2, c"_create_ts_querycursor".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_add_language_from_object));
        lua_setfield(lstate, -2, c"_ts_add_language_from_object".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_has_language));
        lua_setfield(lstate, -2, c"_ts_has_language".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_remove_lang));
        lua_setfield(lstate, -2, c"_ts_remove_language".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_inspect_lang));
        lua_setfield(lstate, -2, c"_ts_inspect_language".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_parse_query));
        lua_setfield(lstate, -2, c"_ts_parse_query".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_get_language_version));
        lua_setfield(lstate, -2, c"_ts_get_language_version".as_ptr());

        lua_pushcfunction(lstate, Some(tslua_get_minimum_language_version));
        lua_setfield(lstate, -2, c"_ts_get_minimum_language_version".as_ptr());
    }
}
