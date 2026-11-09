//! Raw `LuaJIT`, `luv`, and `tree-sitter` C FFI types and `extern "C"` declarations.

use std::ffi::{c_char, c_void};

pub type Boolean = bool;
pub type Float = f64;
pub type LuaRef = i32;
pub(crate) const LUA_NOREF: LuaRef = -2;
#[allow(non_camel_case_types)]
pub type lua_State = c_void;
#[allow(non_camel_case_types)]
pub type lua_CFunction = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State) -> i32>;
#[allow(non_camel_case_types)]
pub type lua_Number = f64;
#[allow(non_camel_case_types)]
pub type lua_Integer = isize;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

unsafe impl Sync for luaL_Reg {}

#[repr(C)]
pub struct luaL_Buffer {
    pub p: *mut c_char,
    pub lvl: i32,
    pub lstate: *mut lua_State,
    pub buffer: [c_char; 8192],
}

impl luaL_Buffer {
    pub const fn new() -> Self {
        Self {
            p: std::ptr::null_mut(),
            lvl: 0,
            lstate: std::ptr::null_mut(),
            buffer: [0; 8192],
        }
    }
}

impl Default for luaL_Buffer {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct lua_Debug {
    pub event: i32,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub currentline: i32,
    pub nups: i32,
    pub linedefined: i32,
    pub lastlinedefined: i32,
    pub short_src: [c_char; 60],
    pub i_ci: i32,
    pub _pad: [u8; 32],
}

impl lua_Debug {
    pub const fn new() -> Self {
        Self {
            event: 0,
            name: std::ptr::null(),
            namewhat: std::ptr::null(),
            what: std::ptr::null(),
            source: std::ptr::null(),
            currentline: 0,
            nups: 0,
            linedefined: 0,
            lastlinedefined: 0,
            short_src: [0; 60],
            i_ci: 0,
            _pad: [0; 32],
        }
    }
}

impl Default for lua_Debug {
    fn default() -> Self {
        Self::new()
    }
}

pub const LUA_MULTRET: i32 = -1;
pub const LUA_REGISTRYINDEX: i32 = -10000;
pub const LUA_GLOBALSINDEX: i32 = -10002;
pub(crate) const LUA_MINSTACK: i32 = 20;

#[cfg(feature = "vendor-bit")]
pub(crate) const LUA_TNONE: i32 = -1;
pub(crate) const LUA_TNIL: i32 = 0;
pub(crate) const LUA_TBOOLEAN: i32 = 1;
pub(crate) const LUA_TLIGHTUSERDATA: i32 = 2;
pub(crate) const LUA_TNUMBER: i32 = 3;
pub(crate) const LUA_TSTRING: i32 = 4;
pub(crate) const LUA_TTABLE: i32 = 5;
pub(crate) const LUA_TFUNCTION: i32 = 6;
pub(crate) const LUA_TUSERDATA: i32 = 7;

pub(crate) const LUA_ERRRUN: i32 = 2;
pub(crate) const LUA_ERRMEM: i32 = 4;

pub(crate) const LUAL_BUFFERSIZE: usize = if cfg!(windows) { 512 } else { 8192 };

#[allow(non_camel_case_types)]
pub type luv_CFpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        nargs: i32,
        nresults: i32,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
pub type luv_CFcpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        func: lua_CFunction,
        ud: *mut c_void,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
pub type luv_acquire_vm = Option<unsafe extern "C-unwind" fn() -> *mut lua_State>;
#[allow(non_camel_case_types)]
pub type luv_release_vm = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State)>;

unsafe extern "C-unwind" {
    pub fn lua_gettop(lstate: *mut lua_State) -> i32;
    pub fn lua_settop(lstate: *mut lua_State, idx: i32);
    pub fn lua_pushvalue(lstate: *mut lua_State, idx: i32);
    pub fn lua_remove(lstate: *mut lua_State, idx: i32);
    pub fn lua_insert(lstate: *mut lua_State, idx: i32);
    pub fn lua_replace(lstate: *mut lua_State, idx: i32);
    pub fn lua_checkstack(lstate: *mut lua_State, sz: i32) -> i32;
    pub fn lua_type(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_typename(lstate: *mut lua_State, tp: i32) -> *const c_char;
    pub fn lua_equal(lstate: *mut lua_State, idx1: i32, idx2: i32) -> i32;
    pub fn lua_rawequal(lstate: *mut lua_State, idx1: i32, idx2: i32) -> i32;
    pub fn lua_tonumber(lstate: *mut lua_State, idx: i32) -> lua_Number;
    pub fn lua_toboolean(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_tolstring(lstate: *mut lua_State, idx: i32, len: *mut usize) -> *const c_char;
    pub fn lua_objlen(lstate: *mut lua_State, idx: i32) -> usize;
    pub fn lua_tocfunction(lstate: *mut lua_State, idx: i32) -> lua_CFunction;
    pub fn lua_topointer(lstate: *mut lua_State, idx: i32) -> *const c_void;
    pub fn lua_pushnil(lstate: *mut lua_State);
    pub fn lua_pushnumber(lstate: *mut lua_State, n: lua_Number);
    pub fn lua_pushinteger(lstate: *mut lua_State, n: lua_Integer);
    pub fn lua_pushlstring(lstate: *mut lua_State, s: *const c_char, l: usize);
    #[link_name = "lua_pushstring"]
    pub(crate) fn c_lua_pushstring(lstate: *mut lua_State, s: *const c_char);
    pub fn lua_pushfstring(lstate: *mut lua_State, fmt: *const c_char, ...) -> *const c_char;
    pub fn lua_pushboolean(lstate: *mut lua_State, b: i32);
    pub fn lua_pushlightuserdata(lstate: *mut lua_State, p: *mut c_void);
    pub fn lua_gettable(lstate: *mut lua_State, idx: i32);
    #[link_name = "lua_getfield"]
    pub(crate) fn c_lua_getfield(lstate: *mut lua_State, idx: i32, k: *const c_char);
    pub fn lua_settable(lstate: *mut lua_State, idx: i32);
    #[link_name = "lua_setfield"]
    pub(crate) fn c_lua_setfield(lstate: *mut lua_State, idx: i32, k: *const c_char);
    pub fn lua_rawget(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawgeti(lstate: *mut lua_State, idx: i32, n: i32);
    pub fn lua_createtable(lstate: *mut lua_State, narr: i32, nrec: i32);
    pub fn lua_newuserdata(lstate: *mut lua_State, sz: usize) -> *mut c_void;
    pub fn lua_getmetatable(lstate: *mut lua_State, objindex: i32) -> i32;
    pub fn lua_getfenv(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawset(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawseti(lstate: *mut lua_State, idx: i32, n: i32);
    pub fn lua_setmetatable(lstate: *mut lua_State, objindex: i32) -> i32;
    pub fn lua_setfenv(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_call(lstate: *mut lua_State, nargs: i32, nresults: i32);
    pub fn lua_pcall(lstate: *mut lua_State, nargs: i32, nresults: i32, errfunc: i32) -> i32;
    pub fn lua_error(lstate: *mut lua_State) -> i32;
    pub fn lua_next(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_concat(lstate: *mut lua_State, n: i32);
    pub fn lua_pushcclosure(lstate: *mut lua_State, fn_: lua_CFunction, n: i32);
    pub fn lua_isnumber(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_isstring(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_iscfunction(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_isuserdata(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_tointeger(lstate: *mut lua_State, idx: i32) -> lua_Integer;
    pub fn lua_touserdata(lstate: *mut lua_State, idx: i32) -> *mut c_void;
    pub fn luaL_newstate() -> *mut lua_State;
    pub fn luaL_openlibs(lstate: *mut lua_State);
    pub fn lua_close(lstate: *mut lua_State);
    pub fn luaL_ref(lstate: *mut lua_State, t: i32) -> i32;
    pub fn luaL_unref(lstate: *mut lua_State, t: i32, ref_: i32);
    #[link_name = "luaL_newmetatable"]
    pub(crate) fn c_luaL_newmetatable(lstate: *mut lua_State, tname: *const c_char) -> i32;
    #[link_name = "luaL_getmetafield"]
    pub(crate) fn c_luaL_getmetafield(lstate: *mut lua_State, obj: i32, e: *const c_char) -> i32;
    #[link_name = "luaL_checkudata"]
    pub(crate) fn c_luaL_checkudata(
        lstate: *mut lua_State,
        ud: i32,
        tname: *const c_char,
    ) -> *mut c_void;
    #[link_name = "luaL_register"]
    pub(crate) fn c_luaL_register(
        lstate: *mut lua_State,
        libname: *const c_char,
        l: *const luaL_Reg,
    );
    #[link_name = "luaL_checkstack"]
    pub(crate) fn c_luaL_checkstack(lstate: *mut lua_State, sz: i32, msg: *const c_char);
    pub fn luaL_checktype(lstate: *mut lua_State, narg: i32, t: i32);
    pub fn luaL_checkany(lstate: *mut lua_State, narg: i32);
    pub fn luaL_checklstring(lstate: *mut lua_State, numArg: i32, l: *mut usize) -> *const c_char;
    pub fn luaL_checknumber(lstate: *mut lua_State, numArg: i32) -> lua_Number;
    pub fn luaL_checkinteger(lstate: *mut lua_State, numArg: i32) -> lua_Integer;
    pub fn luaL_optinteger(lstate: *mut lua_State, nArg: i32, def: lua_Integer) -> lua_Integer;
    #[cfg(feature = "vendor-bit")]
    #[link_name = "luaL_typerror"]
    pub(crate) fn c_luaL_typerror(lstate: *mut lua_State, narg: i32, tname: *const c_char) -> i32;
    #[link_name = "luaL_argerror"]
    pub(crate) fn c_luaL_argerror(
        lstate: *mut lua_State,
        numarg: i32,
        extramsg: *const c_char,
    ) -> i32;
    pub fn luaL_where(lstate: *mut lua_State, lvl: i32);
    pub fn luaL_error(lstate: *mut lua_State, fmt: *const c_char, ...) -> i32;
    #[link_name = "luaL_loadbuffer"]
    pub(crate) fn c_luaL_loadbuffer(
        lstate: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
    ) -> i32;
    pub fn luaL_buffinit(lstate: *mut lua_State, B: *mut luaL_Buffer);
    pub fn luaL_prepbuffer(B: *mut luaL_Buffer) -> *mut c_char;
    pub fn luaL_addlstring(B: *mut luaL_Buffer, s: *const c_char, l: usize);
    pub fn luaL_addvalue(B: *mut luaL_Buffer);
    pub fn luaL_pushresult(B: *mut luaL_Buffer);
    pub fn lua_getstack(lstate: *mut lua_State, level: i32, ar: *mut lua_Debug) -> i32;
    #[link_name = "lua_getinfo"]
    pub(crate) fn c_lua_getinfo(
        lstate: *mut lua_State,
        what: *const c_char,
        ar: *mut lua_Debug,
    ) -> i32;

    pub fn luaopen_luv(lstate: *mut lua_State) -> i32;
}

unsafe extern "C" {
    pub fn luv_set_loop(lstate: *mut lua_State, loop_: *mut c_void);
    pub fn luv_set_callback(lstate: *mut lua_State, pcall: luv_CFpcall);
    pub fn luv_set_thread(lstate: *mut lua_State, pcall: luv_CFpcall);
    pub fn luv_set_cthread(lstate: *mut lua_State, cpcall: luv_CFcpcall);
    pub fn luv_set_thread_cb(acquire: luv_acquire_vm, release: luv_release_vm);

    #[cfg(unix)]
    pub fn pthread_exit(retval: *mut c_void) -> !;
}

// Tree-sitter C API FFI types and declarations
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
        ) -> *const u8,
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
    pub log:
        Option<unsafe extern "C" fn(payload: *mut c_void, log_type: TSLogType, buffer: *const u8)>,
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

unsafe extern "C-unwind" {
    pub fn ts_parser_parse(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
    ) -> *mut TSTree;
    pub fn ts_parser_parse_with_options(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
        parse_options: TSParseOptions,
    ) -> *mut TSTree;
}

unsafe extern "C" {
    pub fn ts_parser_new() -> *mut TSParser;
    pub fn ts_parser_delete(self_: *mut TSParser);
    pub fn ts_parser_language(self_: *const TSParser) -> *const TSLanguage;
    pub fn ts_parser_set_language(self_: *mut TSParser, language: *const TSLanguage) -> bool;
    pub fn ts_parser_set_included_ranges(
        self_: *mut TSParser,
        ranges: *const TSRange,
        count: u32,
    ) -> bool;
    pub fn ts_parser_included_ranges(self_: *const TSParser, count: &mut u32) -> *const TSRange;
    pub fn ts_parser_parse_string(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        string: *const u8,
        length: u32,
    ) -> *mut TSTree;
    pub fn ts_parser_reset(self_: *mut TSParser);
    pub fn ts_parser_set_logger(self_: *mut TSParser, logger: TSLogger);
    pub fn ts_parser_logger(self_: *const TSParser) -> TSLogger;

    pub fn ts_tree_copy(self_: *const TSTree) -> *mut TSTree;
    pub fn ts_tree_delete(self_: *mut TSTree);
    pub fn ts_tree_root_node(self_: *const TSTree) -> TSNode;
    pub fn ts_tree_included_ranges(self_: *const TSTree, length: &mut u32) -> *mut TSRange;
    pub fn ts_tree_edit(self_: *mut TSTree, edit: &TSInputEdit);
    pub fn ts_tree_get_changed_ranges(
        old_tree: *const TSTree,
        new_tree: *const TSTree,
        length: &mut u32,
    ) -> *mut TSRange;

    pub fn ts_node_type(self_: TSNode) -> *const u8;
    pub fn ts_node_symbol(self_: TSNode) -> TSSymbol;
    pub fn ts_node_start_byte(self_: TSNode) -> u32;
    pub fn ts_node_start_point(self_: TSNode) -> TSPoint;
    pub fn ts_node_end_byte(self_: TSNode) -> u32;
    pub fn ts_node_end_point(self_: TSNode) -> TSPoint;
    pub fn ts_node_string(self_: TSNode) -> *mut u8;
    pub fn ts_node_is_null(self_: TSNode) -> bool;
    pub fn ts_node_is_named(self_: TSNode) -> bool;
    pub fn ts_node_is_missing(self_: TSNode) -> bool;
    pub fn ts_node_is_extra(self_: TSNode) -> bool;
    pub fn ts_node_has_changes(self_: TSNode) -> bool;
    pub fn ts_node_has_error(self_: TSNode) -> bool;
    pub fn ts_node_parent(self_: TSNode) -> TSNode;
    pub fn ts_node_child_with_descendant(self_: TSNode, descendant: TSNode) -> TSNode;
    pub fn ts_node_child(self_: TSNode, child_index: u32) -> TSNode;
    pub fn ts_node_field_name_for_child(self_: TSNode, child_index: u32) -> *const u8;
    pub fn ts_node_child_count(self_: TSNode) -> u32;
    pub fn ts_node_named_child(self_: TSNode, child_index: u32) -> TSNode;
    pub fn ts_node_named_child_count(self_: TSNode) -> u32;
    pub fn ts_node_next_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_prev_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_next_named_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_prev_named_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_descendant_for_point_range(
        self_: TSNode,
        start: TSPoint,
        end: TSPoint,
    ) -> TSNode;
    pub fn ts_node_named_descendant_for_point_range(
        self_: TSNode,
        start: TSPoint,
        end: TSPoint,
    ) -> TSNode;
    pub fn ts_node_eq(self_: TSNode, other: TSNode) -> bool;

    pub fn ts_query_new(
        language: *const TSLanguage,
        source: *const u8,
        source_len: u32,
        error_offset: &mut u32,
        error_type: &mut TSQueryError,
    ) -> *mut TSQuery;
    pub fn ts_query_delete(self_: *mut TSQuery);
    pub fn ts_query_pattern_count(self_: *const TSQuery) -> u32;
    pub fn ts_query_capture_count(self_: *const TSQuery) -> u32;
    pub fn ts_query_predicates_for_pattern(
        self_: *const TSQuery,
        pattern_index: u32,
        step_count: &mut u32,
    ) -> *const TSQueryPredicateStep;
    pub fn ts_query_capture_name_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
    ) -> *const u8;
    pub fn ts_query_string_value_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
    ) -> *const u8;
    pub fn ts_query_disable_capture(self_: *mut TSQuery, name: *const u8, length: u32);
    pub fn ts_query_disable_pattern(self_: *mut TSQuery, pattern_index: u32);

    pub fn ts_query_cursor_new() -> *mut TSQueryCursor;
    pub fn ts_query_cursor_delete(self_: *mut TSQueryCursor);
    pub fn ts_query_cursor_exec(self_: *mut TSQueryCursor, query: *const TSQuery, node: TSNode);
    pub fn ts_query_cursor_set_match_limit(self_: *mut TSQueryCursor, limit: u32);
    pub fn ts_query_cursor_set_point_range(
        self_: *mut TSQueryCursor,
        start_point: TSPoint,
        end_point: TSPoint,
    ) -> bool;
    pub fn ts_query_cursor_next_match(self_: *mut TSQueryCursor, match_: &mut TSQueryMatch)
    -> bool;
    pub fn ts_query_cursor_remove_match(self_: *mut TSQueryCursor, match_id: u32);
    pub fn ts_query_cursor_next_capture(
        self_: *mut TSQueryCursor,
        match_: &mut TSQueryMatch,
        capture_index: &mut u32,
    ) -> bool;
    pub fn ts_query_cursor_set_max_start_depth(self_: *mut TSQueryCursor, max_start_depth: u32);

    pub fn ts_language_symbol_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_state_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_field_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_field_name_for_id(self_: *const TSLanguage, id: TSFieldId) -> *const u8;
    pub fn ts_language_supertypes(self_: *const TSLanguage, length: &mut u32) -> *const TSSymbol;
    pub fn ts_language_subtypes(
        self_: *const TSLanguage,
        supertype: TSSymbol,
        length: &mut u32,
    ) -> *const TSSymbol;
    pub fn ts_language_symbol_name(self_: *const TSLanguage, symbol: TSSymbol) -> *const u8;
    pub fn ts_language_symbol_type(self_: *const TSLanguage, symbol: TSSymbol) -> TSSymbolType;
    pub fn ts_language_abi_version(self_: *const TSLanguage) -> u32;
    pub fn ts_language_metadata(self_: *const TSLanguage) -> *const TSLanguageMetadata;
    pub fn ts_language_is_wasm(self_: *const TSLanguage) -> bool;

    pub fn ts_set_allocator(
        new_malloc: Option<unsafe extern "C" fn(usize) -> *mut c_void>,
        new_calloc: Option<unsafe extern "C" fn(usize, usize) -> *mut c_void>,
        new_realloc: Option<unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void>,
        new_free: Option<unsafe extern "C" fn(*mut c_void)>,
    );

    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_lua() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_c() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_diff() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_vim() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_markdown() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_markdown_inline() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_query() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_vimdoc() -> *const TSLanguage;
}

pub struct TSOwnedRanges {
    ptr: *mut TSRange,
    len: usize,
}

impl TSOwnedRanges {
    #[inline]
    pub fn as_slice(&self) -> &[TSRange] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

impl Drop for TSOwnedRanges {
    fn drop(&mut self) {
        crate::memory::raw_dealloc(self.ptr.cast());
    }
}

pub struct TSOwnedCString {
    ptr: *mut u8,
}

impl TSOwnedCString {
    #[inline]
    pub fn as_cstr(&self) -> Option<&std::ffi::CStr> {
        (!self.ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(self.ptr.cast()) })
    }
}

impl Drop for TSOwnedCString {
    fn drop(&mut self) {
        crate::memory::raw_dealloc(self.ptr.cast());
    }
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

unsafe extern "C" fn ts_malloc(size: usize) -> *mut c_void {
    crate::memory::raw_alloc(size)
}

unsafe extern "C" fn ts_calloc(count: usize, size: usize) -> *mut c_void {
    crate::memory::raw_calloc(count, size)
}

unsafe extern "C" fn ts_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    crate::memory::raw_realloc(ptr, size)
}

unsafe extern "C" fn ts_free(ptr: *mut c_void) {
    crate::memory::raw_dealloc(ptr)
}

#[inline]
pub fn ts_init_allocator() {
    unsafe {
        ts_set_allocator(
            Some(ts_malloc),
            Some(ts_calloc),
            Some(ts_realloc),
            Some(ts_free),
        );
    }
}

#[cfg(target_os = "emscripten")]
pub fn ts_get_static_parser(lang: Option<&[u8]>) -> *const TSLanguage {
    let Some(lang) = lang else {
        return std::ptr::null();
    };
    unsafe {
        match lang {
            b"lua" => tree_sitter_lua(),
            b"c" => tree_sitter_c(),
            b"diff" => tree_sitter_diff(),
            b"vim" => tree_sitter_vim(),
            b"markdown" => tree_sitter_markdown(),
            b"markdown_inline" => tree_sitter_markdown_inline(),
            b"query" => tree_sitter_query(),
            b"vimdoc" => tree_sitter_vimdoc(),
            _ => std::ptr::null(),
        }
    }
}

#[inline]
pub fn ts_lang_abi_version(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_abi_version(lang) }
}

#[inline]
pub fn ts_lang_symbol_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_symbol_count(lang) }
}

#[inline]
pub fn ts_lang_symbol_type(lang: *const TSLanguage, symbol: TSSymbol) -> TSSymbolType {
    unsafe { ts_language_symbol_type(lang, symbol) }
}

#[inline]
pub fn ts_lang_symbol_name(
    lang: *const TSLanguage,
    symbol: TSSymbol,
) -> Option<&'static std::ffi::CStr> {
    let ptr = unsafe { ts_language_symbol_name(lang, symbol) };
    (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
}

#[inline]
pub fn ts_lang_field_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_field_count(lang) }
}

#[inline]
pub fn ts_lang_field_name(
    lang: *const TSLanguage,
    id: TSFieldId,
) -> Option<&'static std::ffi::CStr> {
    let ptr = unsafe { ts_language_field_name_for_id(lang, id) };
    (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
}

#[inline]
pub fn ts_lang_is_wasm(lang: *const TSLanguage) -> bool {
    unsafe { ts_language_is_wasm(lang) }
}

#[inline]
pub fn ts_lang_metadata(lang: *const TSLanguage) -> Option<TSLanguageMetadata> {
    let ptr = unsafe { ts_language_metadata(lang) };
    (!ptr.is_null()).then(|| unsafe { *ptr })
}

#[inline]
pub fn ts_lang_state_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_state_count(lang) }
}

#[inline]
pub fn ts_lang_supertypes(lang: *const TSLanguage) -> &'static [TSSymbol] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_language_supertypes(lang, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_lang_subtypes_raw(
    lang: *const TSLanguage,
    supertype: TSSymbol,
) -> (*const TSSymbol, u32) {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_language_subtypes(lang, supertype, &mut len) };
    (ptr, len)
}

#[inline]
pub fn ts_lang_subtype_at(ptr: *const TSSymbol, idx: u32) -> TSSymbol {
    unsafe { *ptr.wrapping_add(idx as usize) }
}

#[inline]
pub fn ts_parser_create(lang: *const TSLanguage) -> Option<*mut TSParser> {
    unsafe {
        let p = ts_parser_new();
        if !ts_parser_set_language(p, lang) {
            ts_parser_delete(p);
            None
        } else {
            Some(p)
        }
    }
}

#[inline]
pub fn ts_parser_free(p: *mut TSParser) {
    unsafe {
        let logger = ts_parser_logger(p);
        if logger.log.is_some() {
            drop(Box::from_raw(logger.payload.cast::<TSLuaLoggerOpts>()));
        }
        ts_parser_delete(p);
    }
}

#[inline]
pub fn ts_parser_has_language(p: *const TSParser) -> bool {
    unsafe { !ts_parser_language(p).is_null() }
}

#[inline]
pub fn ts_parser_reset_safe(p: *mut TSParser) {
    unsafe {
        ts_parser_reset(p);
    }
}

#[inline]
pub fn ts_parser_parse_bytes(
    p: *mut TSParser,
    old_tree: *const TSTree,
    bytes: &[u8],
) -> *mut TSTree {
    unsafe {
        ts_parser_parse_string(
            p,
            old_tree,
            if bytes.is_empty() {
                c"".as_ptr().cast()
            } else {
                bytes.as_ptr()
            },
            bytes.len() as u32,
        )
    }
}

#[repr(C)]
struct TSLuaParserCallbackPayload {
    parse_start_time: u64,
    timeout_threshold_ns: u64,
}

unsafe extern "C" fn on_parser_progress(state: *mut TSParseState) -> bool {
    unsafe {
        let payload = (*state).payload.cast::<TSLuaParserCallbackPayload>();
        let parse_time = crate::os::time::os_hrtime().wrapping_sub((*payload).parse_start_time);
        parse_time >= (*payload).timeout_threshold_ns
    }
}

#[repr(C)]
struct TSInputCallbackContext {
    buf: *mut crate::buffer::FileBuf,
    read_cb: fn(&mut crate::buffer::FileBuf, TSPoint) -> (&'static [u8], u32),
}

unsafe extern "C" fn ts_input_read_trampoline(
    payload: *mut c_void,
    _byte_index: u32,
    position: TSPoint,
    bytes_read: *mut u32,
) -> *const u8 {
    unsafe {
        let ctx = &mut *payload.cast::<TSInputCallbackContext>();
        let (slice, count) = (ctx.read_cb)(&mut *ctx.buf, position);
        *bytes_read = count;
        if slice.is_empty() {
            c"".as_ptr().cast()
        } else {
            slice.as_ptr()
        }
    }
}

pub fn ts_parser_parse_with_input(
    p: *mut TSParser,
    old_tree: *const TSTree,
    buf: &mut crate::buffer::FileBuf,
    read_cb: fn(&mut crate::buffer::FileBuf, TSPoint) -> (&'static [u8], u32),
    timeout_ns: u64,
) -> *mut TSTree {
    unsafe {
        let mut ctx = TSInputCallbackContext {
            buf: std::ptr::from_mut(buf),
            read_cb,
        };
        let input = TSInput {
            payload: (&raw mut ctx).cast::<c_void>(),
            read: Some(ts_input_read_trampoline),
            encoding: 0,
            decode: None,
        };

        if timeout_ns == 0 {
            return ts_parser_parse(p, old_tree, input);
        }

        let mut payload = TSLuaParserCallbackPayload {
            parse_start_time: crate::os::time::os_hrtime(),
            timeout_threshold_ns: timeout_ns,
        };

        let parse_options = TSParseOptions {
            payload: (&raw mut payload).cast::<c_void>(),
            progress_callback: Some(on_parser_progress),
        };

        ts_parser_parse_with_options(p, old_tree, input, parse_options)
    }
}

#[inline]
pub fn ts_parser_set_ranges_slice(p: *mut TSParser, ranges: &[TSRange]) {
    unsafe {
        ts_parser_set_included_ranges(p, ranges.as_ptr(), ranges.len() as u32);
    }
}

#[inline]
pub fn ts_parser_get_ranges_slice(p: *const TSParser) -> &'static [TSRange] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_parser_included_ranges(p, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

unsafe extern "C" fn ts_logger_cb(payload: *mut c_void, logtype: TSLogType, s: *const u8) {
    const TSLOG_TYPE_PARSE: TSLogType = 0;
    const TSLOG_TYPE_LEX: TSLogType = 1;
    unsafe {
        let opts = &mut *payload.cast::<TSLuaLoggerOpts>();
        if (!opts.lex && logtype == TSLOG_TYPE_LEX) || (!opts.parse && logtype == TSLOG_TYPE_PARSE)
        {
            return;
        }

        let lstate = opts.lstate;
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, opts.cb);
        crate::lua::converter::lua_pushstring(
            lstate,
            if logtype == TSLOG_TYPE_PARSE {
                c"parse"
            } else {
                c"lex"
            },
        );
        crate::lua::converter::lua_pushstring_opt(
            lstate,
            (!s.is_null()).then(|| std::ffi::CStr::from_ptr(s.cast())),
        );
        if lua_pcall(lstate, 2, 0, 0) != 0 {
            luaL_error(lstate, c"treesitter logger callback failed".as_ptr());
        }
    }
}

#[inline]
pub fn ts_parser_set_lua_logger(
    p: *mut TSParser,
    lstate: *mut lua_State,
    lex: bool,
    parse: bool,
    cb: LuaRef,
) {
    let opts = Box::into_raw(Box::new(TSLuaLoggerOpts {
        cb,
        lstate,
        lex,
        parse,
    }));
    let logger = TSLogger {
        payload: opts.cast::<c_void>(),
        log: Some(ts_logger_cb),
    };
    unsafe {
        ts_parser_set_logger(p, logger);
    }
}

#[inline]
pub fn ts_parser_get_lua_logger_cb(p: *const TSParser) -> Option<LuaRef> {
    unsafe {
        let logger = ts_parser_logger(p);
        logger.log.map(|_| {
            let opts = logger.payload.cast::<TSLuaLoggerOpts>();
            (*opts).cb
        })
    }
}

#[inline]
pub fn ts_tree_clone(tree: *const TSTree) -> *mut TSTree {
    unsafe { ts_tree_copy(tree) }
}

#[inline]
pub fn ts_tree_free(tree: *mut TSTree) {
    unsafe {
        ts_tree_delete(tree);
    }
}

#[inline]
pub fn ts_tree_root(tree: *const TSTree) -> TSNode {
    unsafe { ts_tree_root_node(tree) }
}

#[inline]
pub fn ts_tree_edit_safe(tree: *mut TSTree, edit: &TSInputEdit) {
    unsafe {
        ts_tree_edit(tree, edit);
    }
}

#[inline]
pub fn ts_tree_included_ranges_owned(tree: *const TSTree) -> TSOwnedRanges {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_tree_included_ranges(tree, &mut len) };
    TSOwnedRanges {
        ptr,
        len: len as usize,
    }
}

#[inline]
pub fn ts_tree_changed_ranges_owned(
    old_tree: *const TSTree,
    new_tree: *const TSTree,
) -> TSOwnedRanges {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_tree_get_changed_ranges(old_tree, new_tree, &mut len) };
    TSOwnedRanges {
        ptr,
        len: len as usize,
    }
}

impl TSNode {
    #[inline]
    pub const fn empty() -> Self {
        Self {
            context: [0; 4],
            id: std::ptr::null(),
            tree: std::ptr::null(),
        }
    }

    #[inline]
    pub fn is_null(self) -> bool {
        unsafe { ts_node_is_null(self) }
    }

    #[inline]
    pub fn type_cstr(self) -> Option<&'static std::ffi::CStr> {
        let ptr = unsafe { ts_node_type(self) };
        (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
    }

    #[inline]
    pub fn symbol(self) -> TSSymbol {
        unsafe { ts_node_symbol(self) }
    }

    #[inline]
    pub fn start_point(self) -> TSPoint {
        unsafe { ts_node_start_point(self) }
    }

    #[inline]
    pub fn end_point(self) -> TSPoint {
        unsafe { ts_node_end_point(self) }
    }

    #[inline]
    pub fn start_byte(self) -> u32 {
        unsafe { ts_node_start_byte(self) }
    }

    #[inline]
    pub fn end_byte(self) -> u32 {
        unsafe { ts_node_end_byte(self) }
    }

    #[inline]
    pub fn child_count(self) -> u32 {
        unsafe { ts_node_child_count(self) }
    }

    #[inline]
    pub fn named_child_count(self) -> u32 {
        unsafe { ts_node_named_child_count(self) }
    }

    #[inline]
    pub fn child(self, idx: u32) -> Self {
        unsafe { ts_node_child(self, idx) }
    }

    #[inline]
    pub fn named_child(self, idx: u32) -> Self {
        unsafe { ts_node_named_child(self, idx) }
    }

    #[inline]
    pub fn field_name_for_child(self, idx: u32) -> Option<&'static std::ffi::CStr> {
        let ptr = unsafe { ts_node_field_name_for_child(self, idx) };
        (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
    }

    #[inline]
    pub fn is_named(self) -> bool {
        unsafe { ts_node_is_named(self) }
    }

    #[inline]
    pub fn is_missing(self) -> bool {
        unsafe { ts_node_is_missing(self) }
    }

    #[inline]
    pub fn is_extra(self) -> bool {
        unsafe { ts_node_is_extra(self) }
    }

    #[inline]
    pub fn has_changes(self) -> bool {
        unsafe { ts_node_has_changes(self) }
    }

    #[inline]
    pub fn has_error(self) -> bool {
        unsafe { ts_node_has_error(self) }
    }

    #[inline]
    pub fn sexpr_owned(self) -> TSOwnedCString {
        TSOwnedCString {
            ptr: unsafe { ts_node_string(self) },
        }
    }

    #[inline]
    pub fn descendant_for_point_range(self, start: TSPoint, end: TSPoint) -> Self {
        unsafe { ts_node_descendant_for_point_range(self, start, end) }
    }

    #[inline]
    pub fn named_descendant_for_point_range(self, start: TSPoint, end: TSPoint) -> Self {
        unsafe { ts_node_named_descendant_for_point_range(self, start, end) }
    }

    #[inline]
    pub fn parent(self) -> Self {
        unsafe { ts_node_parent(self) }
    }

    #[inline]
    pub fn child_with_descendant(self, descendant: Self) -> Self {
        unsafe { ts_node_child_with_descendant(self, descendant) }
    }

    #[inline]
    pub fn next_sibling(self) -> Self {
        unsafe { ts_node_next_sibling(self) }
    }

    #[inline]
    pub fn prev_sibling(self) -> Self {
        unsafe { ts_node_prev_sibling(self) }
    }

    #[inline]
    pub fn next_named_sibling(self) -> Self {
        unsafe { ts_node_next_named_sibling(self) }
    }

    #[inline]
    pub fn prev_named_sibling(self) -> Self {
        unsafe { ts_node_prev_named_sibling(self) }
    }

    #[inline]
    pub fn eq_node(self, other: Self) -> bool {
        unsafe { ts_node_eq(self, other) }
    }

    #[inline]
    pub fn id_ne_bytes(self) -> [u8; size_of::<*const c_void>()] {
        (self.id as usize).to_ne_bytes()
    }
}

impl TSQueryMatch {
    #[inline]
    pub fn captures_slice(&self) -> &[TSQueryCapture] {
        let len = self.capture_count as usize;
        if self.captures.is_null() || len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.captures, len) }
        }
    }
}

#[inline]
pub fn ts_query_create(
    lang: *const TSLanguage,
    src: &[u8],
) -> Result<*mut TSQuery, (usize, TSQueryError)> {
    let mut error_offset: u32 = 0;
    let mut error_type: TSQueryError = 0;
    let q = unsafe {
        ts_query_new(
            lang,
            if src.is_empty() {
                c"".as_ptr().cast()
            } else {
                src.as_ptr()
            },
            src.len() as u32,
            &mut error_offset,
            &mut error_type,
        )
    };
    if q.is_null() {
        Err((error_offset as usize, error_type))
    } else {
        Ok(q)
    }
}

#[inline]
pub fn ts_query_free(q: *mut TSQuery) {
    unsafe {
        ts_query_delete(q);
    }
}

#[inline]
pub fn ts_query_pattern_count_safe(q: *const TSQuery) -> u32 {
    unsafe { ts_query_pattern_count(q) }
}

#[inline]
pub fn ts_query_predicates_slice(
    q: *const TSQuery,
    pattern_index: u32,
) -> &'static [TSQueryPredicateStep] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_predicates_for_pattern(q, pattern_index, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_string_bytes(q: *const TSQuery, id: u32) -> &'static [u8] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_string_value_for_id(q, id, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_capture_count_safe(q: *const TSQuery) -> u32 {
    unsafe { ts_query_capture_count(q) }
}

#[inline]
pub fn ts_query_capture_name_bytes(q: *const TSQuery, index: u32) -> &'static [u8] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_capture_name_for_id(q, index, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_disable_capture_bytes(q: *mut TSQuery, name: &[u8]) {
    unsafe {
        ts_query_disable_capture(
            q,
            if name.is_empty() {
                c"".as_ptr().cast()
            } else {
                name.as_ptr()
            },
            name.len() as u32,
        );
    }
}

#[inline]
pub fn ts_query_disable_pattern_safe(q: *mut TSQuery, pattern_index: u32) {
    unsafe {
        ts_query_disable_pattern(q, pattern_index);
    }
}

#[inline]
pub fn ts_query_cursor_create() -> *mut TSQueryCursor {
    unsafe { ts_query_cursor_new() }
}

#[inline]
pub fn ts_query_cursor_free(c: *mut TSQueryCursor) {
    unsafe {
        ts_query_cursor_delete(c);
    }
}

#[inline]
pub fn ts_query_cursor_configure_and_exec(
    c: *mut TSQueryCursor,
    q: *const TSQuery,
    node: TSNode,
    start: TSPoint,
    end: TSPoint,
    max_start_depth: Option<u32>,
    match_limit: Option<u32>,
) {
    unsafe {
        ts_query_cursor_set_point_range(c, start, end);
        if let Some(depth) = max_start_depth {
            ts_query_cursor_set_max_start_depth(c, depth);
        }
        if let Some(limit) = match_limit {
            ts_query_cursor_set_match_limit(c, limit);
        }
        ts_query_cursor_exec(c, q, node);
    }
}

#[inline]
pub fn ts_query_cursor_remove_match_safe(c: *mut TSQueryCursor, match_id: u32) {
    unsafe {
        ts_query_cursor_remove_match(c, match_id);
    }
}

#[inline]
pub fn ts_query_cursor_next_capture_safe(
    c: *mut TSQueryCursor,
) -> Option<(TSQueryMatch, TSQueryCapture)> {
    let mut match_ = TSQueryMatch {
        id: 0,
        pattern_index: 0,
        capture_count: 0,
        captures: std::ptr::null(),
    };
    let mut capture_index: u32 = 0;
    unsafe {
        if !ts_query_cursor_next_capture(c, &mut match_, &mut capture_index) {
            return None;
        }
        let capture = *match_.captures.wrapping_add(capture_index as usize);
        Some((match_, capture))
    }
}

#[inline]
pub fn ts_query_cursor_next_match_safe(c: *mut TSQueryCursor) -> Option<TSQueryMatch> {
    let mut match_ = TSQueryMatch {
        id: 0,
        pattern_index: 0,
        capture_count: 0,
        captures: std::ptr::null(),
    };
    unsafe { ts_query_cursor_next_match(c, &mut match_).then_some(match_) }
}
