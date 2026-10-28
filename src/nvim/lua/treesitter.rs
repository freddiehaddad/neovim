//! Lua bindings for Tree-sitter (`src/nvim/lua/treesitter.c`).

use crate::buffer::FileBuf;
use crate::charset::NUL;
use crate::context::handle_get_buffer;
use crate::cursor::Linenr;
use crate::event::r#loop::{uv_dlclose, uv_dlerror, uv_dlopen, uv_dlsym};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::lua::converter::{
    LUA_MINSTACK, LUA_REGISTRYINDEX, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE, LuaRef, lua_Integer,
    lua_Number, lua_State, lua_concat, lua_createtable, lua_error, lua_getfenv, lua_getfield,
    lua_gettop, lua_isboolean, lua_isfunction, lua_isnil, lua_isstring, lua_istable, lua_newtable,
    lua_newuserdata, lua_objlen, lua_pcall, lua_pop, lua_pushboolean, lua_pushcclosure,
    lua_pushcfunction, lua_pushinteger, lua_pushlstring, lua_pushnil, lua_pushnumber,
    lua_pushstring, lua_pushvalue, lua_rawgeti, lua_rawseti, lua_setfenv, lua_setfield,
    lua_setmetatable, lua_toboolean, lua_tointeger, lua_tolstring, lua_touserdata, lua_type,
    lua_upvalueindex, luaL_Reg, luaL_argcheck, luaL_argerror, luaL_checkint, luaL_checkinteger,
    luaL_checklstring, luaL_checknumber, luaL_checkstring, luaL_checkudata, luaL_error,
    luaL_newmetatable, luaL_ref, luaL_register, luaL_unref,
};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{raw_alloc, raw_calloc, raw_dealloc, raw_realloc, xstrlcpy};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::os::dl::UvLib;
use crate::os::time::os_hrtime;
use crate::strings::vim_snprintf;
use std::ffi::{c_char, c_void};
use std::ptr;

pub type TSSymbol = u16;
pub type TSFieldId = u16;
pub type TSLanguage = c_void;
pub type TSParser = c_void;
pub type TSTree = c_void;
pub type TSQuery = c_void;
pub type TSQueryCursor = c_void;

pub type TSInputEncoding = i32;
pub type TSSymbolType = i32;
pub type TSLogType = i32;
pub type TSQueryPredicateStepType = i32;
pub type TSQueryError = i32;

pub type TSDecodeFunction =
    Option<unsafe extern "C" fn(string: *const u8, length: u32, code_point: *mut i32) -> u32>;

const BUFSIZE: usize = 256;

const TREE_SITTER_LANGUAGE_VERSION: u32 = 15;
const TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION: u32 = 13;

const TSINPUT_ENCODING_UTF8: TSInputEncoding = 0;

const TSSYMBOL_TYPE_ANONYMOUS: TSSymbolType = 1;
const TSSYMBOL_TYPE_AUXILIARY: TSSymbolType = 3;

const TSLOG_TYPE_PARSE: TSLogType = 0;
const TSLOG_TYPE_LEX: TSLogType = 1;

const TSQUERY_PREDICATE_STEP_TYPE_DONE: TSQueryPredicateStepType = 0;
const TSQUERY_PREDICATE_STEP_TYPE_CAPTURE: TSQueryPredicateStepType = 1;
const TSQUERY_PREDICATE_STEP_TYPE_STRING: TSQueryPredicateStepType = 2;

const TSQUERY_ERROR_NONE: TSQueryError = 0;
const TSQUERY_ERROR_SYNTAX: TSQueryError = 1;
const TSQUERY_ERROR_NODE_TYPE: TSQueryError = 2;
const TSQUERY_ERROR_FIELD: TSQueryError = 3;
const TSQUERY_ERROR_CAPTURE: TSQueryError = 4;
const TSQUERY_ERROR_STRUCTURE: TSQueryError = 5;

const TS_META_PARSER: *const c_char = c"treesitter_parser".as_ptr();
const TS_META_TREE: *const c_char = c"treesitter_tree".as_ptr();
const TS_META_NODE: *const c_char = c"treesitter_node".as_ptr();
const TS_META_QUERY: *const c_char = c"treesitter_query".as_ptr();
const TS_META_QUERYCURSOR: *const c_char = c"treesitter_querycursor".as_ptr();
const TS_META_QUERYMATCH: *const c_char = c"treesitter_querymatch".as_ptr();

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TSPoint {
    pub row: u32,
    pub column: u32,
}

impl TSPoint {
    pub const fn new() -> Self {
        Self { row: 0, column: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TSRange {
    pub start_point: TSPoint,
    pub end_point: TSPoint,
    pub start_byte: u32,
    pub end_byte: u32,
}

impl TSRange {
    pub const fn new() -> Self {
        Self {
            start_point: TSPoint::new(),
            end_point: TSPoint::new(),
            start_byte: 0,
            end_byte: 0,
        }
    }
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

#[repr(C)]
struct TSLuaLoggerOpts {
    cb: LuaRef,
    lstate: *mut lua_State,
    lex: bool,
    parse: bool,
}

impl Drop for TSLuaLoggerOpts {
    fn drop(&mut self) {
        unsafe {
            luaL_unref(self.lstate, LUA_REGISTRYINDEX, self.cb);
        }
    }
}

#[repr(C)]
struct TSLuaTree {
    tree: *const TSTree,
}

#[repr(C)]
struct TSLuaParserCallbackPayload {
    parse_start_time: u64,
    timeout_threshold_ns: u64,
}

struct TreesitterModuleState {
    langs: std::collections::BTreeMap<Vec<u8>, *mut TSLanguage>,
    input_buf: [c_char; BUFSIZE],
}

impl TreesitterModuleState {
    const fn new() -> Self {
        Self {
            langs: std::collections::BTreeMap::new(),
            input_buf: [0; BUFSIZE],
        }
    }
}

static TS_STATE: crate::globals::GlobalState<TreesitterModuleState> =
    crate::globals::GlobalState::new(TreesitterModuleState::new());

#[inline]
fn ts_st() -> &'static mut TreesitterModuleState {
    unsafe { &mut *TS_STATE.as_ptr() }
}

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

unsafe extern "C-unwind" {
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
    fn ts_parser_included_ranges(self_: *const TSParser, count: &mut u32) -> *const TSRange;
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
    fn ts_tree_included_ranges(self_: *const TSTree, length: &mut u32) -> *mut TSRange;
    fn ts_tree_edit(self_: *mut TSTree, edit: &TSInputEdit);
    fn ts_tree_get_changed_ranges(
        old_tree: *const TSTree,
        new_tree: *const TSTree,
        length: &mut u32,
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
        error_offset: &mut u32,
        error_type: &mut TSQueryError,
    ) -> *mut TSQuery;
    fn ts_query_delete(self_: *mut TSQuery);
    fn ts_query_pattern_count(self_: *const TSQuery) -> u32;
    fn ts_query_capture_count(self_: *const TSQuery) -> u32;
    fn ts_query_predicates_for_pattern(
        self_: *const TSQuery,
        pattern_index: u32,
        step_count: &mut u32,
    ) -> *const TSQueryPredicateStep;
    fn ts_query_capture_name_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
    ) -> *const c_char;
    fn ts_query_string_value_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
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
    fn ts_query_cursor_next_match(self_: *mut TSQueryCursor, match_: &mut TSQueryMatch) -> bool;
    fn ts_query_cursor_remove_match(self_: *mut TSQueryCursor, match_id: u32);
    fn ts_query_cursor_next_capture(
        self_: *mut TSQueryCursor,
        match_: &mut TSQueryMatch,
        capture_index: &mut u32,
    ) -> bool;
    fn ts_query_cursor_set_max_start_depth(self_: *mut TSQueryCursor, max_start_depth: u32);

    fn ts_language_symbol_count(self_: *const TSLanguage) -> u32;
    fn ts_language_state_count(self_: *const TSLanguage) -> u32;
    fn ts_language_field_count(self_: *const TSLanguage) -> u32;
    fn ts_language_field_name_for_id(self_: *const TSLanguage, id: TSFieldId) -> *const c_char;
    fn ts_language_supertypes(self_: *const TSLanguage, length: &mut u32) -> *const TSSymbol;
    fn ts_language_subtypes(
        self_: *const TSLanguage,
        supertype: TSSymbol,
        length: &mut u32,
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

#[inline]
fn langs_has(lang_name: impl crate::strings::IntoCCharPtr) -> bool {
    let lang_name = lang_name.into_c_char_ptr();
    if lang_name.is_null() {
        return false;
    }
    {
        ts_st()
            .langs
            .contains_key(crate::memory::cstr_as_bytes(lang_name))
    }
}

#[inline]
fn langs_get(lang_name: impl crate::strings::IntoCCharPtr) -> *mut TSLanguage {
    let lang_name = lang_name.into_c_char_ptr();
    if lang_name.is_null() {
        return ptr::null_mut();
    }
    {
        ts_st()
            .langs
            .get(crate::memory::cstr_as_bytes(lang_name))
            .copied()
            .unwrap_or(ptr::null_mut())
    }
}

unsafe extern "C-unwind" fn tslua_has_language(lstate: *mut lua_State) -> i32 {
    {
        let lang_name = luaL_checkstring(lstate, 1);
        unsafe { lua_pushboolean(lstate, i32::from(langs_has(lang_name))) };
        1
    }
}

unsafe extern "C-unwind" fn tslua_add_language_from_object(lstate: *mut lua_State) -> i32 {
    add_language(lstate, false)
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
pub fn nvim_ts_get_parser(lang: impl crate::strings::IntoCCharPtr) -> *const TSLanguage {
    let lang = lang.into_c_char_ptr();
    if lang.is_null() {
        return ptr::null();
    }
    unsafe {
        match crate::memory::cstr_as_bytes(lang) {
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

fn load_language_from_object(
    lstate: *mut lua_State,
    path: impl crate::strings::IntoCCharPtr,
    lang_name: impl crate::strings::IntoCCharPtr,
    symbol: impl crate::strings::IntoCCharPtr,
) -> *const TSLanguage {
    let path = path.into_c_char_ptr();
    let lang_name = lang_name.into_c_char_ptr();
    let symbol = symbol.into_c_char_ptr();
    unsafe {
        #[cfg(target_os = "emscripten")]
        {
            let static_lang = nvim_ts_get_parser(symbol);
            if !static_lang.is_null() {
                return static_lang;
            }
        }

        let iobuff = g_st().iobuff.as_mut_ptr();
        let mut lib = UvLib {
            _opaque: [ptr::null_mut(); 4],
        };
        if uv_dlopen(path, &mut lib) != 0 {
            xstrlcpy(iobuff, uv_dlerror(&lib), IOSIZE);
            uv_dlclose(&mut lib);
            luaL_error(
                lstate,
                c"Failed to load parser for language '%s': uv_dlopen: %s".as_ptr(),
                lang_name,
                iobuff,
            );
        }

        let mut symbol_buf = [0 as c_char; 128];
        vim_snprintf!(
            symbol_buf.as_mut_ptr(),
            symbol_buf.len(),
            c"tree_sitter_%s",
            symbol,
        );

        let mut fn_ptr: *mut c_void = ptr::null_mut();
        if uv_dlsym(&mut lib, symbol_buf.as_ptr(), &mut fn_ptr) != 0 {
            xstrlcpy(iobuff, uv_dlerror(&lib), IOSIZE);
            uv_dlclose(&mut lib);
            luaL_error(
                lstate,
                c"Failed to load parser: uv_dlsym: %s".as_ptr(),
                iobuff,
            );
        }

        let lang_parser: unsafe extern "C" fn() -> *mut TSLanguage = std::mem::transmute(fn_ptr);
        let lang = lang_parser();

        if lang.is_null() {
            uv_dlclose(&mut lib);
            luaL_error(
                lstate,
                c"Failed to load parser %s: internal error".as_ptr(),
                path,
            );
        }

        lang
    }
}

fn load_language_from_wasm(
    lstate: *mut lua_State,
    _path: *const c_char,
    _lang_name: *const c_char,
) -> *const TSLanguage {
    unsafe {
        luaL_error(lstate, c"Not supported".as_ptr());
        ptr::null()
    }
}

fn add_language(lstate: *mut lua_State, is_wasm: bool) -> i32 {
    {
        let path = luaL_checkstring(lstate, 1);
        let lang_name = luaL_checkstring(lstate, 2);
        let mut symbol_name = lang_name;

        if !is_wasm && unsafe { lua_gettop(lstate) } >= 3 && !lua_isnil(lstate, 3) {
            symbol_name = luaL_checkstring(lstate, 3);
        }

        if langs_has(lang_name) {
            unsafe { lua_pushboolean(lstate, 1) };
            return 1;
        }

        let lang = if is_wasm {
            load_language_from_wasm(lstate, path, lang_name)
        } else {
            load_language_from_object(lstate, path, lang_name, symbol_name)
        };

        let lang_version = unsafe { ts_language_abi_version(lang) };
        if !(TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION..=TREE_SITTER_LANGUAGE_VERSION)
            .contains(&lang_version)
        {
            return unsafe {
                luaL_error(
                    lstate,
                    c"ABI version mismatch for %s: supported between %d and %d, found %d".as_ptr(),
                    path,
                    TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION as i32,
                    TREE_SITTER_LANGUAGE_VERSION as i32,
                    lang_version as i32,
                )
            };
        }

        ts_st().langs.insert(
            crate::memory::cstr_as_bytes(lang_name).to_vec(),
            lang.cast_mut(),
        );

        unsafe { lua_pushboolean(lstate, 1) };
        1
    }
}

unsafe extern "C-unwind" fn tslua_remove_lang(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let lang_name = luaL_checkstring(lstate, 1);
            let langs_ptr = &raw mut ts_st().langs;
            let present = !lang_name.is_null()
                && (*langs_ptr)
                    .remove(crate::memory::cstr_as_bytes(lang_name))
                    .is_some();
            lua_pushboolean(lstate, i32::from(present));
            1
        }
    }
}

fn lang_check(lstate: *mut lua_State, index: i32) -> *mut TSLanguage {
    {
        let lang_name = luaL_checkstring(lstate, index);
        let lang = langs_get(lang_name);
        if lang.is_null() {
            unsafe { luaL_error(lstate, c"no such language: %s".as_ptr(), lang_name) };
        }
        lang
    }
}

unsafe extern "C-unwind" fn tslua_inspect_lang(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let lang = lang_check(lstate, 1);

            lua_createtable(lstate, 0, 2);

            {
                let nsymbols = ts_language_symbol_count(lang);
                debug_assert!(nsymbols < i32::MAX as u32);

                lua_createtable(lstate, (nsymbols.wrapping_sub(1)) as i32, 1);
                for i in 0..nsymbols {
                    let t = ts_language_symbol_type(lang, i as TSSymbol);
                    if t == TSSYMBOL_TYPE_AUXILIARY {
                        continue;
                    }
                    let name = ts_language_symbol_name(lang, i as TSSymbol);
                    let named = t != TSSYMBOL_TYPE_ANONYMOUS;
                    lua_pushboolean(lstate, i32::from(named));
                    if !named {
                        let mut buf = [0 as c_char; 256];
                        vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"\"%s\"", name);
                        lua_setfield(lstate, -2, buf.as_ptr());
                    } else {
                        lua_setfield(lstate, -2, name);
                    }
                }

                lua_setfield(lstate, -2, c"symbols");
            }

            {
                let nfields = ts_language_field_count(lang);
                lua_createtable(lstate, nfields as i32, 1);
                for i in 1..=nfields {
                    lua_pushstring(lstate, ts_language_field_name_for_id(lang, i as TSFieldId));
                    lua_rawseti(lstate, -2, i as i32);
                }

                lua_setfield(lstate, -2, c"fields");
            }

            lua_pushboolean(lstate, i32::from(ts_language_is_wasm(lang)));
            lua_setfield(lstate, -2, c"_wasm");

            lua_pushinteger(lstate, ts_language_abi_version(lang) as lua_Integer);
            lua_setfield(lstate, -2, c"abi_version");

            {
                let meta = ts_language_metadata(lang);
                if !meta.is_null() {
                    let meta = &*meta;
                    lua_createtable(lstate, 0, 3);

                    lua_pushinteger(lstate, meta.major_version as lua_Integer);
                    lua_setfield(lstate, -2, c"major_version");
                    lua_pushinteger(lstate, meta.minor_version as lua_Integer);
                    lua_setfield(lstate, -2, c"minor_version");
                    lua_pushinteger(lstate, meta.patch_version as lua_Integer);
                    lua_setfield(lstate, -2, c"patch_version");

                    lua_setfield(lstate, -2, c"metadata");
                }
            }

            lua_pushinteger(lstate, ts_language_state_count(lang) as lua_Integer);
            lua_setfield(lstate, -2, c"state_count");

            {
                let mut nsupertypes: u32 = 0;
                let supertypes = ts_language_supertypes(lang, &mut nsupertypes);

                lua_createtable(lstate, 0, nsupertypes as i32);
                for i in 0..nsupertypes {
                    let supertype = *supertypes.wrapping_add(i as usize);

                    let mut nsubtypes: u32 = 0;
                    let subtypes = ts_language_subtypes(lang, supertype, &mut nsubtypes);

                    lua_createtable(lstate, nsubtypes as i32, 0);
                    for j in 1..=nsubtypes {
                        lua_pushstring(
                            lstate,
                            ts_language_symbol_name(lang, *subtypes.wrapping_add(j as usize)),
                        );
                        lua_rawseti(lstate, -2, j as i32);
                    }

                    lua_setfield(lstate, -2, ts_language_symbol_name(lang, supertype));
                }

                lua_setfield(lstate, -2, c"supertypes");
            }

            1
        }
    }
}

unsafe extern "C-unwind" fn tslua_push_parser(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let lang = lang_check(lstate, 1);

            let parser =
                lua_newuserdata(lstate, size_of::<*mut TSParser>()).cast::<*mut TSParser>();
            *parser = ts_parser_new();

            if !ts_parser_set_language(*parser, lang) {
                ts_parser_delete(*parser);
                let lang_name = luaL_checkstring(lstate, 1);
                return luaL_error(lstate, c"Failed to load language : %s".as_ptr(), lang_name);
            }

            lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_PARSER);
            lua_setmetatable(lstate, -2);
            1
        }
    }
}

fn parser_check(lstate: *mut lua_State, index: i32) -> *mut TSParser {
    unsafe {
        let ud = luaL_checkudata(lstate, index, TS_META_PARSER).cast::<*mut TSParser>();
        luaL_argcheck(lstate, !(*ud).is_null(), index, c"Parser has been deleted");
        *ud
    }
}

fn logger_gc(logger: TSLogger) {
    if logger.log.is_none() {
        return;
    }

    unsafe {
        let opts = logger.payload.cast::<TSLuaLoggerOpts>();
        drop(Box::from_raw(opts));
    }
}

unsafe extern "C-unwind" fn parser_gc(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let ud = luaL_checkudata(lstate, 1, TS_META_PARSER).cast::<*mut TSParser>();
            if !(*ud).is_null() {
                logger_gc(ts_parser_logger(*ud));
                ts_parser_delete(*ud);
                *ud = ptr::null_mut();
            }
            0
        }
    }
}

unsafe extern "C-unwind" fn parser_tostring(lstate: *mut lua_State) -> i32 {
    {
        lua_pushstring(lstate, c"<parser>");
        1
    }
}

unsafe extern "C" fn input_cb(
    payload: *mut c_void,
    _byte_index: u32,
    position: TSPoint,
    bytes_read: *mut u32,
) -> *const c_char {
    unsafe {
        {
            let bp = payload.cast::<FileBuf>();
            let bp = { &mut *bp };
            let buf_ptr = (&raw mut ts_st().input_buf).cast::<c_char>();

            if (position.row as Linenr) >= bp.b_ml.ml_line_count {
                *bytes_read = 0;
                return c"".as_ptr();
            }
            let lnum = (position.row as Linenr) + 1;
            let line = ml_get_buf(&mut *bp, lnum);
            let len = ml_get_buf_len(&mut *bp, lnum) as usize;
            let col = position.column as usize;
            if col > len {
                *bytes_read = 0;
                return c"".as_ptr();
            }
            let tocopy = (len - col).min(BUFSIZE);

            ptr::copy_nonoverlapping(line.wrapping_add(col), buf_ptr, tocopy);
            for i in 0..tocopy {
                let p = buf_ptr.wrapping_add(i);
                if crate::memory::cstr_first_byte(p) == b'\n' {
                    *p = NUL;
                }
            }
            *bytes_read = tocopy as u32;
            if tocopy < BUFSIZE
                && (lnum != bp.b_ml.ml_line_count
                    || (bp.b_p_bin == 0 && bp.b_p_fixeol != 0)
                    || (lnum != bp.b_no_eol_lnum && bp.b_p_eol != 0))
            {
                *buf_ptr.wrapping_add(tocopy) = b'\n' as c_char;
                *bytes_read += 1;
            }
            buf_ptr
        }
    }
}

fn push_ranges(lstate: *mut lua_State, ranges: *const TSRange, length: usize, include_bytes: bool) {
    unsafe {
        lua_createtable(lstate, length as i32, 0);
        for i in 0..length {
            let r = *ranges.wrapping_add(i);
            lua_createtable(lstate, if include_bytes { 6 } else { 4 }, 0);
            let mut j: i32 = 1;
            lua_pushnumber(lstate, r.start_point.row as lua_Number);
            lua_rawseti(lstate, -2, j);
            j += 1;
            lua_pushnumber(lstate, r.start_point.column as lua_Number);
            lua_rawseti(lstate, -2, j);
            j += 1;
            if include_bytes {
                lua_pushnumber(lstate, r.start_byte as lua_Number);
                lua_rawseti(lstate, -2, j);
                j += 1;
            }
            lua_pushnumber(lstate, r.end_point.row as lua_Number);
            lua_rawseti(lstate, -2, j);
            j += 1;
            lua_pushnumber(lstate, r.end_point.column as lua_Number);
            lua_rawseti(lstate, -2, j);
            j += 1;
            if include_bytes {
                lua_pushnumber(lstate, r.end_byte as lua_Number);
                lua_rawseti(lstate, -2, j);
            }

            lua_rawseti(lstate, -2, (i + 1) as i32);
        }
    }
}

unsafe extern "C" fn on_parser_progress(state: *mut TSParseState) -> bool {
    unsafe {
        {
            let payload = (*state).payload.cast::<TSLuaParserCallbackPayload>();
            let parse_time = os_hrtime().wrapping_sub((*payload).parse_start_time);
            parse_time >= (*payload).timeout_threshold_ns
        }
    }
}

pub unsafe extern "C-unwind" fn nts_parser_parse_buf(
    p: *mut TSParser,
    old_tree: *const TSTree,
    bufnr: i32,
    timeout_ns: u64,
) -> *mut TSTree {
    {
        let buf = handle_get_buffer(bufnr as ObjHandle);
        if buf.is_null() {
            std::process::abort();
        }

        let input = TSInput {
            payload: buf.cast::<c_void>(),
            read: Some(input_cb),
            encoding: TSINPUT_ENCODING_UTF8,
            decode: None,
        };

        if timeout_ns == 0 {
            return unsafe { ts_parser_parse(p, old_tree, input) };
        }

        let mut payload = TSLuaParserCallbackPayload {
            parse_start_time: os_hrtime(),
            timeout_threshold_ns: timeout_ns,
        };

        let parse_options = TSParseOptions {
            payload: (&raw mut payload).cast::<c_void>(),
            progress_callback: Some(on_parser_progress),
        };

        unsafe { ts_parser_parse_with_options(p, old_tree, input, parse_options) }
    }
}

unsafe extern "C-unwind" fn parser_parse(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let p = parser_check(lstate, 1);
            let mut old_tree: *const TSTree = ptr::null();
            if !lua_isnil(lstate, 2) {
                let ud = luaL_checkudata(lstate, 2, TS_META_TREE).cast::<TSLuaTree>();
                old_tree = if !ud.is_null() {
                    (*ud).tree
                } else {
                    ptr::null()
                };
            }

            let new_tree: *mut TSTree = match lua_type(lstate, 3) {
                LUA_TSTRING => {
                    let mut len: usize = 0;
                    let str_ptr = lua_tolstring(lstate, 3, &mut len);
                    ts_parser_parse_string(p, old_tree, str_ptr, len as u32)
                }
                LUA_TNUMBER => {
                    let bufnr = lua_tointeger(lstate, 3) as ObjHandle;
                    let buf = handle_get_buffer(bufnr);
                    if buf.is_null() {
                        let mut ebuf = [0 as c_char; IOSIZE];
                        vim_snprintf!(
                            ebuf.as_mut_ptr(),
                            IOSIZE,
                            c"invalid buffer handle: %d",
                            bufnr,
                        );
                        return luaL_argerror(lstate, 3, ebuf.as_ptr());
                    }

                    let timeout_ns = if lua_isnil(lstate, 5) {
                        0
                    } else {
                        lua_tointeger(lstate, 5) as u64
                    };
                    nts_parser_parse_buf(p, old_tree, bufnr, timeout_ns)
                }
                _ => {
                    return luaL_argerror(lstate, 3, c"expected either string or buffer handle");
                }
            };

            let include_bytes = lua_gettop(lstate) >= 4 && lua_toboolean(lstate, 4) != 0;

            if new_tree.is_null() {
                if ts_parser_language(p).is_null() {
                    return luaL_error(
                        lstate,
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

            push_tree(lstate, new_tree);
            push_ranges(lstate, changed, n_ranges as usize, include_bytes);

            ts_free(changed.cast::<c_void>());
            2
        }
    }
}

unsafe extern "C-unwind" fn parser_reset(lstate: *mut lua_State) -> i32 {
    {
        let p = parser_check(lstate, 1);
        unsafe { ts_parser_reset(p) };
        0
    }
}

fn range_err(lstate: *mut lua_State) {
    {
        unsafe {
            luaL_error(
                lstate,
                c"Ranges can only be made from 6 element long tables or nodes.".as_ptr(),
            )
        };
    }
}

fn lua_checkuint32(lstate: *mut lua_State, index: i32) -> u32 {
    unsafe {
        let value = luaL_checknumber(lstate, index);
        let converted = value as u32;
        if value < 0.0 || value > (u32::MAX as lua_Number) || (converted as lua_Number) != value {
            luaL_error(lstate, c"Range value out of bounds".as_ptr());
        }
        converted
    }
}

fn range_from_lua(lstate: *mut lua_State, range: *mut TSRange) {
    unsafe {
        let mut node = TSNode {
            context: [0; 4],
            id: ptr::null(),
            tree: ptr::null(),
        };

        if lua_istable(lstate, -1) {
            if lua_objlen(lstate, -1) != 6 {
                range_err(lstate);
            }

            lua_rawgeti(lstate, -1, 1);
            let start_row = lua_checkuint32(lstate, -1);
            lua_pop(lstate, 1);

            lua_rawgeti(lstate, -1, 2);
            let start_col = lua_checkuint32(lstate, -1);
            lua_pop(lstate, 1);

            lua_rawgeti(lstate, -1, 3);
            let start_byte = lua_checkuint32(lstate, -1);
            lua_pop(lstate, 1);

            lua_rawgeti(lstate, -1, 4);
            let end_row = lua_checkuint32(lstate, -1);
            lua_pop(lstate, 1);

            lua_rawgeti(lstate, -1, 5);
            let end_col = lua_checkuint32(lstate, -1);
            lua_pop(lstate, 1);

            lua_rawgeti(lstate, -1, 6);
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
                start_point: ts_node_start_point(node),
                end_point: ts_node_end_point(node),
                start_byte: ts_node_start_byte(node),
                end_byte: ts_node_end_byte(node),
            };
        } else {
            range_err(lstate);
        }
    }
}

unsafe extern "C-unwind" fn parser_set_ranges(lstate: *mut lua_State) -> i32 {
    {
        if unsafe { lua_gettop(lstate) } < 2 {
            return unsafe {
                luaL_error(
                    lstate,
                    c"not enough args to parser:set_included_ranges()".as_ptr(),
                )
            };
        }

        let p = parser_check(lstate, 1);
        luaL_argcheck(lstate, lua_istable(lstate, 2), 2, c"table expected.");

        let tbl_len = unsafe { lua_objlen(lstate, 2) };
        let mut ranges: Vec<TSRange> = vec![TSRange::new(); tbl_len];

        for (index, range) in ranges.iter_mut().enumerate() {
            unsafe { lua_rawgeti(lstate, 2, (index + 1) as i32) };
            range_from_lua(lstate, range);
            lua_pop(lstate, 1);
        }

        unsafe { ts_parser_set_included_ranges(p, ranges.as_ptr(), tbl_len as u32) };

        0
    }
}

unsafe extern "C-unwind" fn parser_get_ranges(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let p = parser_check(lstate, 1);
            let include_bytes = lua_gettop(lstate) >= 2 && lua_toboolean(lstate, 2) != 0;

            let mut len: u32 = 0;
            let ranges = ts_parser_included_ranges(p, &mut len);

            push_ranges(lstate, ranges, len as usize, include_bytes);
            1
        }
    }
}

unsafe extern "C" fn logger_cb(payload: *mut c_void, logtype: TSLogType, s: *const c_char) {
    {
        let opts = payload.cast::<TSLuaLoggerOpts>();
        let opts = { unsafe { &mut *opts } };
        if (!opts.lex && logtype == TSLOG_TYPE_LEX) || (!opts.parse && logtype == TSLOG_TYPE_PARSE)
        {
            return;
        }

        let lstate = opts.lstate;
        unsafe { lua_rawgeti(lstate, LUA_REGISTRYINDEX, opts.cb) };
        lua_pushstring(
            lstate,
            if logtype == TSLOG_TYPE_PARSE {
                c"parse".as_ptr()
            } else {
                c"lex".as_ptr()
            },
        );
        lua_pushstring(lstate, s);
        if unsafe { lua_pcall(lstate, 2, 0, 0) } != 0 {
            unsafe { luaL_error(lstate, c"treesitter logger callback failed".as_ptr()) };
        }
    }
}

unsafe extern "C-unwind" fn parser_set_logger(lstate: *mut lua_State) -> i32 {
    {
        let p = parser_check(lstate, 1);

        luaL_argcheck(lstate, lua_isboolean(lstate, 2), 2, c"boolean expected");
        luaL_argcheck(lstate, lua_isboolean(lstate, 3), 3, c"boolean expected");
        luaL_argcheck(lstate, lua_isfunction(lstate, 4), 4, c"function expected");

        unsafe { lua_pushvalue(lstate, 4) };
        let ref_ = unsafe { luaL_ref(lstate, LUA_REGISTRYINDEX) };

        let opts = Box::into_raw(Box::new(TSLuaLoggerOpts {
            lex: unsafe { lua_toboolean(lstate, 2) } != 0,
            parse: unsafe { lua_toboolean(lstate, 3) } != 0,
            cb: ref_,
            lstate,
        }));

        let logger = TSLogger {
            payload: opts.cast::<c_void>(),
            log: Some(logger_cb),
        };

        unsafe { ts_parser_set_logger(p, logger) };
        0
    }
}

unsafe extern "C-unwind" fn parser_get_logger(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let p = parser_check(lstate, 1);
            let logger = ts_parser_logger(p);
            if logger.log.is_some() {
                let opts = logger.payload.cast::<TSLuaLoggerOpts>();
                lua_rawgeti(lstate, LUA_REGISTRYINDEX, (*opts).cb);
            } else {
                lua_pushnil(lstate);
            }
            1
        }
    }
}

fn push_tree(lstate: *mut lua_State, tree: *const TSTree) {
    unsafe {
        if tree.is_null() {
            lua_pushnil(lstate);
            return;
        }

        let ud = lua_newuserdata(lstate, size_of::<TSLuaTree>()).cast::<TSLuaTree>();
        (*ud).tree = tree;
        lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_TREE);
        lua_setmetatable(lstate, -2);
    }
}

unsafe extern "C-unwind" fn tree_copy(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let ud = luaL_checkudata(lstate, 1, TS_META_TREE).cast::<TSLuaTree>();
            let copy = ts_tree_copy((*ud).tree);
            push_tree(lstate, copy);
            1
        }
    }
}

unsafe extern "C-unwind" fn tree_edit(lstate: *mut lua_State) -> i32 {
    {
        if unsafe { lua_gettop(lstate) } < 10 {
            lua_pushstring(lstate, c"not enough args to tree:edit()");
            return unsafe { lua_error(lstate) };
        }

        let ud = luaL_checkudata(lstate, 1, TS_META_TREE).cast::<TSLuaTree>();

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

        let new_tree = unsafe { ts_tree_copy((*ud).tree) };
        unsafe { ts_tree_edit(new_tree, &edit) };

        push_tree(lstate, new_tree);
        1
    }
}

unsafe extern "C-unwind" fn tree_get_ranges(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let ud = luaL_checkudata(lstate, 1, TS_META_TREE).cast::<TSLuaTree>();
            let include_bytes = lua_gettop(lstate) >= 2 && lua_toboolean(lstate, 2) != 0;

            let mut len: u32 = 0;
            let ranges = ts_tree_included_ranges((*ud).tree, &mut len);

            push_ranges(lstate, ranges, len as usize, include_bytes);
            ts_free(ranges.cast::<c_void>());
            1
        }
    }
}

unsafe extern "C-unwind" fn tree_gc(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let ud = luaL_checkudata(lstate, 1, TS_META_TREE).cast::<TSLuaTree>();
            let tree = (*ud).tree.cast_mut();
            ts_tree_delete(tree);
            0
        }
    }
}

unsafe extern "C-unwind" fn tree_tostring(lstate: *mut lua_State) -> i32 {
    {
        lua_pushstring(lstate, c"<tree>");
        1
    }
}

unsafe extern "C-unwind" fn tree_root(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let ud = luaL_checkudata(lstate, 1, TS_META_TREE).cast::<TSLuaTree>();
            let root = ts_tree_root_node((*ud).tree);

            let node_ud = lua_newuserdata(lstate, size_of::<TSNode>()).cast::<TSNode>();
            *node_ud = root;
            lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_NODE);
            lua_setmetatable(lstate, -2);

            lua_createtable(lstate, 1, 0);
            lua_pushvalue(lstate, 1);
            lua_rawseti(lstate, -2, 1);
            lua_setfenv(lstate, -2);

            1
        }
    }
}

fn push_node(lstate: *mut lua_State, node: TSNode, uindex: i32) {
    unsafe {
        debug_assert!(!(-LUA_MINSTACK..=0).contains(&uindex));
        if ts_node_is_null(node) {
            lua_pushnil(lstate);
            return;
        }

        let ud = lua_newuserdata(lstate, size_of::<TSNode>()).cast::<TSNode>();
        *ud = node;
        lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_NODE);
        lua_setmetatable(lstate, -2);

        lua_getfenv(lstate, uindex);
        lua_setfenv(lstate, -2);
    }
}

fn node_check_opt(lstate: *mut lua_State, index: i32, res: &mut TSNode) -> bool {
    {
        let ud = luaL_checkudata(lstate, index, TS_META_NODE).cast::<TSNode>();
        if !ud.is_null() {
            *res = unsafe { *ud };
            return true;
        }
        false
    }
}

fn node_check(lstate: *mut lua_State, index: i32) -> TSNode {
    unsafe {
        let ud = luaL_checkudata(lstate, index, TS_META_NODE).cast::<TSNode>();
        *ud
    }
}

unsafe extern "C-unwind" fn node_tostring(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushstring(lstate, c"<node ");
            lua_pushstring(lstate, ts_node_type(node));
            lua_pushstring(lstate, c">");
            lua_concat(lstate, 3);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_eq(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let node2 = node_check(lstate, 2);
            lua_pushboolean(lstate, i32::from(ts_node_eq(node, node2)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_id(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        unsafe {
            lua_pushlstring(
                lstate,
                (&raw const node.id).cast::<c_char>(),
                size_of_val(&node.id),
            )
        };
        1
    }
}

unsafe extern "C-unwind" fn node_range(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let include_bytes = lua_gettop(lstate) >= 2 && lua_toboolean(lstate, 2) != 0;

            let start = ts_node_start_point(node);
            let end = ts_node_end_point(node);

            if include_bytes {
                lua_pushinteger(lstate, start.row as lua_Integer);
                lua_pushinteger(lstate, start.column as lua_Integer);
                lua_pushinteger(lstate, ts_node_start_byte(node) as lua_Integer);
                lua_pushinteger(lstate, end.row as lua_Integer);
                lua_pushinteger(lstate, end.column as lua_Integer);
                lua_pushinteger(lstate, ts_node_end_byte(node) as lua_Integer);
                return 6;
            }

            lua_pushinteger(lstate, start.row as lua_Integer);
            lua_pushinteger(lstate, start.column as lua_Integer);
            lua_pushinteger(lstate, end.row as lua_Integer);
            lua_pushinteger(lstate, end.column as lua_Integer);
            4
        }
    }
}

unsafe extern "C-unwind" fn node_start(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let start = ts_node_start_point(node);
            let start_byte = ts_node_start_byte(node);
            lua_pushinteger(lstate, start.row as lua_Integer);
            lua_pushinteger(lstate, start.column as lua_Integer);
            lua_pushinteger(lstate, start_byte as lua_Integer);
            3
        }
    }
}

unsafe extern "C-unwind" fn node_end(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let end = ts_node_end_point(node);
            let end_byte = ts_node_end_byte(node);
            lua_pushinteger(lstate, end.row as lua_Integer);
            lua_pushinteger(lstate, end.column as lua_Integer);
            lua_pushinteger(lstate, end_byte as lua_Integer);
            3
        }
    }
}

unsafe extern "C-unwind" fn node_child_count(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let count = ts_node_child_count(node);
            lua_pushinteger(lstate, count as lua_Integer);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_named_child_count(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let count = ts_node_named_child_count(node);
            lua_pushinteger(lstate, count as lua_Integer);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_type(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        lua_pushstring(lstate, unsafe { ts_node_type(node) });
        1
    }
}

unsafe extern "C-unwind" fn node_symbol(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let symbol = ts_node_symbol(node);
            lua_pushinteger(lstate, symbol as lua_Integer);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_field(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let count = unsafe { ts_node_child_count(node) };
        let mut curr_index: i32 = 0;

        let mut name_len: usize = 0;
        let field_name = unsafe { luaL_checklstring(lstate, 2, &mut name_len) };

        lua_newtable(lstate);

        for i in 0..count {
            let child_field_name = unsafe { ts_node_field_name_for_child(node, i) };
            if crate::memory::cstr_as_opt_bytes(field_name)
                == crate::memory::cstr_as_opt_bytes(child_field_name)
            {
                let child = unsafe { ts_node_child(node, i) };
                push_node(lstate, child, 1);
                curr_index += 1;
                unsafe { lua_rawseti(lstate, -2, curr_index) };
            }
        }

        1
    }
}

unsafe extern "C-unwind" fn node_named(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushboolean(lstate, i32::from(ts_node_is_named(node)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_sexpr(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let allocated = ts_node_string(node);
            lua_pushstring(lstate, allocated);
            ts_free(allocated.cast::<c_void>());
            1
        }
    }
}

unsafe extern "C-unwind" fn node_missing(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushboolean(lstate, i32::from(ts_node_is_missing(node)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_extra(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushboolean(lstate, i32::from(ts_node_is_extra(node)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_has_changes(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushboolean(lstate, i32::from(ts_node_has_changes(node)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_has_error(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            lua_pushboolean(lstate, i32::from(ts_node_has_error(node)));
            1
        }
    }
}

unsafe extern "C-unwind" fn node_child(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let num = lua_tointeger(lstate, 2) as u32;
            let child = ts_node_child(node, num);
            push_node(lstate, child, 1);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_named_child(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let num = lua_tointeger(lstate, 2) as u32;
            let child = ts_node_named_child(node, num);
            push_node(lstate, child, 1);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_descendant_for_range(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let start = TSPoint {
                row: lua_tointeger(lstate, 2) as u32,
                column: lua_tointeger(lstate, 3) as u32,
            };
            let end = TSPoint {
                row: lua_tointeger(lstate, 4) as u32,
                column: lua_tointeger(lstate, 5) as u32,
            };
            let child = ts_node_descendant_for_point_range(node, start, end);
            push_node(lstate, child, 1);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_named_descendant_for_range(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let start = TSPoint {
                row: lua_tointeger(lstate, 2) as u32,
                column: lua_tointeger(lstate, 3) as u32,
            };
            let end = TSPoint {
                row: lua_tointeger(lstate, 4) as u32,
                column: lua_tointeger(lstate, 5) as u32,
            };
            let child = ts_node_named_descendant_for_point_range(node, start, end);
            push_node(lstate, child, 1);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_next_child(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let child_index = lua_touserdata(lstate, lua_upvalueindex(1)).cast::<u32>();
            let source = node_check(lstate, lua_upvalueindex(2));

            if *child_index >= ts_node_child_count(source) {
                return 0;
            }

            let child = ts_node_child(source, *child_index);
            push_node(lstate, child, lua_upvalueindex(2));

            let field = ts_node_field_name_for_child(source, *child_index);
            if !field.is_null() {
                lua_pushstring(lstate, field);
            } else {
                lua_pushnil(lstate);
            }

            *child_index += 1;
            2
        }
    }
}

unsafe extern "C-unwind" fn node_iter_children(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            node_check(lstate, 1);
            let child_index = lua_newuserdata(lstate, size_of::<u32>()).cast::<u32>();
            *child_index = 0;

            lua_pushvalue(lstate, 1);
            lua_pushcclosure(lstate, Some(node_next_child), 2);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_parent(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let parent = unsafe { ts_node_parent(node) };
        push_node(lstate, parent, 1);
        1
    }
}

unsafe extern "C-unwind" fn __has_ancestor(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let descendant = node_check(lstate, 1);
            if lua_type(lstate, 2) != LUA_TTABLE {
                lua_pushboolean(lstate, 0);
                return 1;
            }
            let pred_len = lua_objlen(lstate, 2) as i32;

            let mut node = ts_tree_root_node(descendant.tree);
            while node.id != descendant.id && !ts_node_is_null(node) {
                let node_type = ts_node_type(node);
                let node_type_bytes = crate::memory::cstr_as_bytes(node_type);

                for i in 3..=pred_len {
                    lua_rawgeti(lstate, 2, i);
                    if lua_type(lstate, -1) == LUA_TSTRING {
                        let mut check_len: usize = 0;
                        let check_str = lua_tolstring(lstate, -1, &mut check_len);
                        let check_bytes = crate::memory::cstr_slice(check_str, check_len);
                        if node_type_bytes == check_bytes {
                            lua_pushboolean(lstate, 1);
                            return 1;
                        }
                    }
                    lua_pop(lstate, 1);
                }

                node = ts_node_child_with_descendant(node, descendant);
            }

            lua_pushboolean(lstate, 0);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_child_with_descendant(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let descendant = node_check(lstate, 2);
        let child = unsafe { ts_node_child_with_descendant(node, descendant) };
        push_node(lstate, child, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_next_sibling(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let sibling = unsafe { ts_node_next_sibling(node) };
        push_node(lstate, sibling, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_prev_sibling(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let sibling = unsafe { ts_node_prev_sibling(node) };
        push_node(lstate, sibling, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_next_named_sibling(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let sibling = unsafe { ts_node_next_named_sibling(node) };
        push_node(lstate, sibling, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_prev_named_sibling(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let sibling = unsafe { ts_node_prev_named_sibling(node) };
        push_node(lstate, sibling, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_named_children(lstate: *mut lua_State) -> i32 {
    {
        let source = node_check(lstate, 1);

        lua_newtable(lstate);
        let mut curr_index: i32 = 0;

        let n = unsafe { ts_node_child_count(source) };
        for i in 0..n {
            let child = unsafe { ts_node_child(source, i) };
            if unsafe { ts_node_is_named(child) } {
                push_node(lstate, child, 1);
                curr_index += 1;
                unsafe { lua_rawseti(lstate, -2, curr_index) };
            }
        }

        1
    }
}

unsafe extern "C-unwind" fn node_root(lstate: *mut lua_State) -> i32 {
    {
        let node = node_check(lstate, 1);
        let root = unsafe { ts_tree_root_node(node.tree) };
        push_node(lstate, root, 1);
        1
    }
}

unsafe extern "C-unwind" fn node_tree(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            node_check(lstate, 1);
            lua_getfenv(lstate, 1);
            lua_rawgeti(lstate, 2, 1);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_byte_length(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let start_byte = ts_node_start_byte(node);
            let end_byte = ts_node_end_byte(node);
            lua_pushinteger(lstate, end_byte.wrapping_sub(start_byte) as lua_Integer);
            1
        }
    }
}

unsafe extern "C-unwind" fn node_equal(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node1 = node_check(lstate, 1);
            let node2 = node_check(lstate, 2);
            lua_pushboolean(lstate, i32::from(ts_node_eq(node1, node2)));
            1
        }
    }
}

unsafe extern "C-unwind" fn tslua_push_querycursor(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let node = node_check(lstate, 1);
            let query = query_check(lstate, 2);
            let cursor = ts_query_cursor_new();

            if lua_gettop(lstate) >= 3 && !lua_isnil(lstate, 3) {
                luaL_argcheck(lstate, lua_istable(lstate, 3), 3, c"table expected");
            }

            lua_getfield(lstate, 3, c"start_row");
            let start_row = luaL_checkinteger(lstate, -1) as u32;
            lua_pop(lstate, 1);

            lua_getfield(lstate, 3, c"start_col");
            let start_col = luaL_checkinteger(lstate, -1) as u32;
            lua_pop(lstate, 1);

            lua_getfield(lstate, 3, c"end_row");
            let end_row = luaL_checkinteger(lstate, -1) as u32;
            lua_pop(lstate, 1);

            lua_getfield(lstate, 3, c"end_col");
            let end_col = luaL_checkinteger(lstate, -1) as u32;
            lua_pop(lstate, 1);

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

            lua_getfield(lstate, 3, c"max_start_depth");
            if !lua_isnil(lstate, -1) {
                let max_start_depth = luaL_checkinteger(lstate, -1) as u32;
                ts_query_cursor_set_max_start_depth(cursor, max_start_depth);
            }
            lua_pop(lstate, 1);

            lua_getfield(lstate, 3, c"match_limit");
            if !lua_isnil(lstate, -1) {
                let match_limit = luaL_checkinteger(lstate, -1) as u32;
                ts_query_cursor_set_match_limit(cursor, match_limit);
            }
            lua_pop(lstate, 1);

            ts_query_cursor_exec(cursor, query, node);

            let ud = lua_newuserdata(lstate, size_of::<*mut TSQueryCursor>())
                .cast::<*mut TSQueryCursor>();
            *ud = cursor;
            lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERYCURSOR);
            lua_setmetatable(lstate, -2);

            lua_getfenv(lstate, 1);
            lua_setfenv(lstate, -2);

            1
        }
    }
}

unsafe extern "C-unwind" fn querycursor_remove_match(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let cursor = querycursor_check(lstate, 1);
            let match_id = luaL_checkinteger(lstate, 2) as u32;
            ts_query_cursor_remove_match(cursor, match_id);
            0
        }
    }
}

unsafe extern "C-unwind" fn querycursor_next_capture(lstate: *mut lua_State) -> i32 {
    {
        let cursor = querycursor_check(lstate, 1);
        let mut match_ = TSQueryMatch {
            id: 0,
            pattern_index: 0,
            capture_count: 0,
            captures: ptr::null(),
        };
        let mut capture_index: u32 = 0;
        if !unsafe { ts_query_cursor_next_capture(cursor, &mut match_, &mut capture_index) } {
            return 0;
        }

        let capture = unsafe { *match_.captures.wrapping_add(capture_index as usize) };
        unsafe { lua_pushinteger(lstate, (capture.index + 1) as lua_Integer) };
        push_node(lstate, capture.node, 1);
        push_querymatch(lstate, &match_, 1);

        3
    }
}

unsafe extern "C-unwind" fn querycursor_next_match(lstate: *mut lua_State) -> i32 {
    {
        let cursor = querycursor_check(lstate, 1);
        let mut match_ = TSQueryMatch {
            id: 0,
            pattern_index: 0,
            capture_count: 0,
            captures: ptr::null(),
        };
        if !unsafe { ts_query_cursor_next_match(cursor, &mut match_) } {
            return 0;
        }

        push_querymatch(lstate, &match_, 1);
        1
    }
}

fn querycursor_check(lstate: *mut lua_State, index: i32) -> *mut TSQueryCursor {
    unsafe {
        let ud = luaL_checkudata(lstate, index, TS_META_QUERYCURSOR).cast::<*mut TSQueryCursor>();
        luaL_argcheck(lstate, !(*ud).is_null(), index, c"TSQueryCursor expected");
        *ud
    }
}

unsafe extern "C-unwind" fn querycursor_gc(lstate: *mut lua_State) -> i32 {
    {
        let cursor = querycursor_check(lstate, 1);
        unsafe { ts_query_cursor_delete(cursor) };
        0
    }
}

fn push_querymatch(lstate: *mut lua_State, match_: &TSQueryMatch, uindex: i32) {
    unsafe {
        let ud = lua_newuserdata(lstate, size_of::<TSQueryMatch>()).cast::<TSQueryMatch>();
        *ud = *match_;
        lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERYMATCH);
        lua_setmetatable(lstate, -2);

        lua_getfenv(lstate, uindex);
        lua_setfenv(lstate, -2);
    }
}

unsafe extern "C-unwind" fn querymatch_info(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let match_ = luaL_checkudata(lstate, 1, TS_META_QUERYMATCH).cast::<TSQueryMatch>();
            lua_pushinteger(lstate, (*match_).id as lua_Integer);
            lua_pushinteger(lstate, ((*match_).pattern_index + 1) as lua_Integer);
            2
        }
    }
}

unsafe extern "C-unwind" fn querymatch_captures(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let match_ = luaL_checkudata(lstate, 1, TS_META_QUERYMATCH).cast::<TSQueryMatch>();
            lua_newtable(lstate);
            for i in 0..((*match_).capture_count as usize) {
                let capture = *(*match_).captures.wrapping_add(i);
                let index = (capture.index + 1) as i32;

                lua_rawgeti(lstate, -1, index);
                if lua_isnil(lstate, -1) {
                    lua_pop(lstate, 1);
                    lua_newtable(lstate);
                }
                push_node(lstate, capture.node, 1);
                lua_rawseti(lstate, -2, (lua_objlen(lstate, -2) + 1) as i32);
                lua_rawseti(lstate, -2, index);
            }
            1
        }
    }
}

unsafe extern "C-unwind" fn tslua_parse_query(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            if lua_gettop(lstate) < 2
                || lua_isstring(lstate, 1) == 0
                || lua_isstring(lstate, 2) == 0
            {
                return luaL_error(lstate, c"string expected".as_ptr());
            }

            let lang = lang_check(lstate, 1);

            let mut len: usize = 0;
            let src = lua_tolstring(lstate, 2, &mut len);

            g_st().tslua_query_parse_count += 1;
            let mut error_offset: u32 = 0;
            let mut error_type: TSQueryError = TSQUERY_ERROR_NONE;
            let query = ts_query_new(lang, src, len as u32, &mut error_offset, &mut error_type);

            if query.is_null() {
                let mut err_buf = [0 as c_char; IOSIZE];
                query_err_string(
                    src,
                    error_offset as i32,
                    error_type,
                    err_buf.as_mut_ptr(),
                    err_buf.len(),
                );
                return luaL_error(lstate, c"%s".as_ptr(), err_buf.as_ptr());
            }

            let ud = lua_newuserdata(lstate, size_of::<*mut TSQuery>()).cast::<*mut TSQuery>();
            *ud = query;
            lua_getfield(lstate, LUA_REGISTRYINDEX, TS_META_QUERY);
            lua_setmetatable(lstate, -2);
            1
        }
    }
}

const fn query_err_to_string(error_type: TSQueryError) -> &'static std::ffi::CStr {
    match error_type {
        TSQUERY_ERROR_SYNTAX => c"Invalid syntax:\n",
        TSQUERY_ERROR_NODE_TYPE => c"Invalid node type ",
        TSQUERY_ERROR_FIELD => c"Invalid field name ",
        TSQUERY_ERROR_CAPTURE => c"Invalid capture name ",
        TSQUERY_ERROR_STRUCTURE => c"Impossible pattern:\n",
        _ => c"error",
    }
}

fn query_err_string(
    src: impl crate::strings::IntoCCharPtr,
    error_offset: i32,
    error_type: TSQueryError,
    mut err_data: *mut c_char,
    mut err_size: usize,
) {
    let src = src.into_c_char_ptr();
    {
        let mut line_start: i32 = 0;
        let mut row: i32 = 0;
        let mut error_line: *const c_char = ptr::null();
        let mut error_line_len: i32 = 0;

        loop {
            let src_tmp = src.wrapping_offset(line_start as isize);
            let src_tmp_bytes = crate::memory::cstr_as_bytes(src_tmp);
            let nl_pos = src_tmp_bytes.iter().position(|&b| b == b'\n');
            let line_length = nl_pos.unwrap_or(src_tmp_bytes.len()) as i32;
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
            c"Query error at %d:%d. %s",
            row + 1,
            column + 1,
            type_msg,
        );
        let offset = crate::memory::cstr_len(err_data);
        err_size = err_size.saturating_sub(offset);
        err_data = err_data.wrapping_add(offset);

        if error_type == TSQUERY_ERROR_NODE_TYPE
            || error_type == TSQUERY_ERROR_FIELD
            || error_type == TSQUERY_ERROR_CAPTURE
        {
            let suffix = src.wrapping_offset(error_offset as isize);
            let is_anonymous = error_type == TSQUERY_ERROR_NODE_TYPE
                && error_offset > 0
                && unsafe { *suffix.wrapping_offset(-1) } == b'"' as c_char;
            let mut suffix_len: usize = 0;
            let mut c = crate::memory::cstr_byte_at(suffix, suffix_len);
            if is_anonymous {
                let mut backslashes: i32 = 0;
                while c != b'"' || backslashes % 2 != 0 {
                    if c == b'\\' {
                        backslashes += 1;
                    } else {
                        backslashes = 0;
                    }
                    suffix_len += 1;
                    c = crate::memory::cstr_byte_at(suffix, suffix_len);
                }
            } else {
                while c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' {
                    suffix_len += 1;
                    c = crate::memory::cstr_byte_at(suffix, suffix_len);
                }
            }
            vim_snprintf!(
                err_data,
                err_size,
                c"\"%.*s\":\n",
                suffix_len as i32,
                suffix,
            );
            let offset = crate::memory::cstr_len(err_data);
            err_size = err_size.saturating_sub(offset);
            err_data = err_data.wrapping_add(offset);
        }

        if error_line.is_null() {
            vim_snprintf!(err_data, err_size, c"Unexpected EOF\n");
            return;
        }

        vim_snprintf!(
            err_data,
            err_size,
            c"%.*s\n%*s^\n",
            error_line_len,
            error_line,
            column,
            c"",
        );
    }
}

fn query_check(lstate: *mut lua_State, index: i32) -> *mut TSQuery {
    unsafe {
        let ud = luaL_checkudata(lstate, index, TS_META_QUERY).cast::<*mut TSQuery>();
        luaL_argcheck(lstate, !(*ud).is_null(), index, c"TSQuery expected");
        *ud
    }
}

unsafe extern "C-unwind" fn query_gc(lstate: *mut lua_State) -> i32 {
    {
        let query = query_check(lstate, 1);
        unsafe { ts_query_delete(query) };
        0
    }
}

unsafe extern "C-unwind" fn query_tostring(lstate: *mut lua_State) -> i32 {
    {
        lua_pushstring(lstate, c"<query>");
        1
    }
}

unsafe extern "C-unwind" fn query_inspect(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let query = query_check(lstate, 1);

            lua_createtable(lstate, 0, 2);

            let n_pat = ts_query_pattern_count(query);
            lua_createtable(lstate, n_pat as i32, 1);
            for i in 0..(n_pat as usize) {
                let mut len: u32 = 0;
                let step = ts_query_predicates_for_pattern(query, i as u32, &mut len);
                if len == 0 {
                    continue;
                }
                lua_createtable(lstate, (len / 4) as i32, 1);
                lua_createtable(lstate, 3, 0);
                let mut nextpred: i32 = 1;
                let mut nextitem: i32 = 1;
                for k in 0..(len as usize) {
                    let s = *step.wrapping_add(k);
                    if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_DONE {
                        lua_rawseti(lstate, -2, nextpred);
                        nextpred += 1;
                        lua_createtable(lstate, 3, 0);
                        nextitem = 1;
                        continue;
                    }

                    if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_STRING {
                        let mut strlen: u32 = 0;
                        let str_ptr = ts_query_string_value_for_id(query, s.value_id, &mut strlen);
                        lua_pushlstring(lstate, str_ptr, strlen as usize);
                    } else if s.type_ == TSQUERY_PREDICATE_STEP_TYPE_CAPTURE {
                        lua_pushinteger(lstate, (s.value_id + 1) as lua_Integer);
                    } else {
                        std::process::abort();
                    }
                    lua_rawseti(lstate, -2, nextitem);
                    nextitem += 1;
                }
                lua_pop(lstate, 1);
                lua_rawseti(lstate, -2, (i + 1) as i32);
            }
            lua_setfield(lstate, -2, c"patterns");

            let n_captures = ts_query_capture_count(query);
            lua_createtable(lstate, n_captures as i32, 0);
            for i in 0..(n_captures as usize) {
                let mut strlen: u32 = 0;
                let str_ptr = ts_query_capture_name_for_id(query, i as u32, &mut strlen);
                lua_pushlstring(lstate, str_ptr, strlen as usize);
                lua_rawseti(lstate, -2, (i + 1) as i32);
            }
            lua_setfield(lstate, -2, c"captures");

            1
        }
    }
}

unsafe extern "C-unwind" fn query_disable_capture(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let query = query_check(lstate, 1);
            let mut name_len: usize = 0;
            let name = luaL_checklstring(lstate, 2, &mut name_len);
            ts_query_disable_capture(query, name, name_len as u32);
            0
        }
    }
}

unsafe extern "C-unwind" fn query_disable_pattern(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            let query = query_check(lstate, 1);
            let pattern_index = luaL_checkinteger(lstate, 2) as u32;
            ts_query_disable_pattern(query, pattern_index.wrapping_sub(1));
            0
        }
    }
}

fn build_meta(
    lstate: *mut lua_State,
    tname: impl crate::strings::IntoCCharPtr,
    meta: *const luaL_Reg,
) {
    let tname = tname.into_c_char_ptr();
    {
        if luaL_newmetatable(lstate, tname) != 0 {
            luaL_register(lstate, ptr::null(), meta);
            unsafe { lua_pushvalue(lstate, -1) };
            lua_setfield(lstate, -2, c"__index");
        }
        lua_pop(lstate, 1);
    }
}

unsafe extern "C" fn ts_malloc(size: usize) -> *mut c_void {
    raw_alloc(size)
}

unsafe extern "C" fn ts_calloc(count: usize, size: usize) -> *mut c_void {
    raw_calloc(count, size)
}

unsafe extern "C" fn ts_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    raw_realloc(ptr, size)
}

unsafe extern "C" fn ts_free(ptr: *mut c_void) {
    raw_dealloc(ptr)
}

fn tslua_init(lstate: *mut lua_State) {
    {
        build_meta(lstate, TS_META_PARSER, PARSER_META.as_ptr());
        build_meta(lstate, TS_META_TREE, TREE_META.as_ptr());
        build_meta(lstate, TS_META_NODE, NODE_META.as_ptr());
        build_meta(lstate, TS_META_QUERY, QUERY_META.as_ptr());
        build_meta(lstate, TS_META_QUERYCURSOR, QUERYCURSOR_META.as_ptr());
        build_meta(lstate, TS_META_QUERYMATCH, QUERYMATCH_META.as_ptr());

        unsafe {
            ts_set_allocator(
                Some(ts_malloc),
                Some(ts_calloc),
                Some(ts_realloc),
                Some(ts_free),
            )
        };
    }
}

unsafe extern "C-unwind" fn tslua_get_language_version(lstate: *mut lua_State) -> i32 {
    unsafe {
        {
            lua_pushnumber(lstate, TREE_SITTER_LANGUAGE_VERSION as lua_Number);
            1
        }
    }
}

unsafe extern "C-unwind" fn tslua_get_minimum_language_version(lstate: *mut lua_State) -> i32 {
    {
        unsafe {
            lua_pushnumber(
                lstate,
                TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION as lua_Number,
            )
        };
        1
    }
}

pub fn nlua_treesitter_init(lstate: *mut lua_State) {
    {
        tslua_init(lstate);

        lua_pushcfunction(lstate, Some(tslua_push_parser));
        lua_setfield(lstate, -2, c"_create_ts_parser");

        lua_pushcfunction(lstate, Some(tslua_push_querycursor));
        lua_setfield(lstate, -2, c"_create_ts_querycursor");

        lua_pushcfunction(lstate, Some(tslua_add_language_from_object));
        lua_setfield(lstate, -2, c"_ts_add_language_from_object");

        lua_pushcfunction(lstate, Some(tslua_has_language));
        lua_setfield(lstate, -2, c"_ts_has_language");

        lua_pushcfunction(lstate, Some(tslua_remove_lang));
        lua_setfield(lstate, -2, c"_ts_remove_language");

        lua_pushcfunction(lstate, Some(tslua_inspect_lang));
        lua_setfield(lstate, -2, c"_ts_inspect_language");

        lua_pushcfunction(lstate, Some(tslua_parse_query));
        lua_setfield(lstate, -2, c"_ts_parse_query");

        lua_pushcfunction(lstate, Some(tslua_get_language_version));
        lua_setfield(lstate, -2, c"_ts_get_language_version");

        lua_pushcfunction(lstate, Some(tslua_get_minimum_language_version));
        lua_setfield(lstate, -2, c"_ts_get_minimum_language_version");
    }
}
