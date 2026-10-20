//! Lua interpreter initialization, execution, and Vimscript/RPC bridges (`src/nvim/lua/executor.c`).

#![cfg_attr(test, allow(unused_mut))]
#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::command::{cmdmod_T, exarg_T, ucmd_T};
#[cfg(not(test))]
use crate::api::extmark::ns_initialized;
use crate::api::private::converter::sctx_T;
#[cfg(not(test))]
use crate::api::private::helpers::{
    HlMessageChunk, TryState, api_clear_error, api_free_array, api_set_error, api_set_sctx,
    arena_array, error_set, string_to_cstr, try_enter, try_leave,
};
#[cfg(not(test))]
use crate::api::private::validate::ErrorType;
use crate::api::private::validate::{Array, Error, Object, ObjectData, ObjectType};
#[cfg(not(test))]
use crate::change::inserted_bytes;
#[cfg(not(test))]
use crate::clipboard::TypvalVval;
use crate::clipboard::{VarType, typval_T};
#[cfg(not(test))]
use crate::cursor::{check_cursor, colnr_T};
#[cfg(not(test))]
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
#[cfg(not(test))]
use crate::errors::*;
#[cfg(not(test))]
use crate::eval::funcs::{find_internal_func, find_internal_func_lua};
#[cfg(not(test))]
use crate::eval::typval::tv_clear;
use crate::eval::userfunc::FCERR_NONE;
#[cfg(not(test))]
use crate::eval::userfunc::{EvalFuncDef, FCERR_OTHER, call_func, funcexe_T, register_luafunc};
#[cfg(not(test))]
use crate::event::r#loop::uv_thread_t;
#[cfg(not(test))]
use crate::event::r#loop::{
    loop_closing_ptr, loop_poll_events, loop_queues, loop_schedule_deferred,
};
#[cfg(not(test))]
use crate::event::multiqueue::{EVENT_HANDLER_MAX_ARGC, Event, MultiQueue, multiqueue_put_event};
#[cfg(not(test))]
use crate::event::multiqueue::{multiqueue_empty, multiqueue_process_events};
#[cfg(not(test))]
use crate::ex_cmds::check_secure;
#[cfg(not(test))]
use crate::ex_docmd::{do_cmdline, get_command_name};
#[cfg(not(test))]
use crate::ex_getln::{
    cmdpreview_get_bufnr, cmdpreview_get_ns, get_user_input, script_get,
    ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave,
};
#[cfg(not(test))]
use crate::fileio::fprintf;
#[cfg(not(test))]
use crate::garray::{ga_append, ga_clear, ga_concat_len, ga_grow, ga_init};
use crate::garray::{ga_concat_strings, garray_T};
#[cfg(not(test))]
use crate::globals::*;
#[cfg(not(test))]
use crate::input::vgetc;
#[cfg(not(test))]
use crate::keycodes::{special_to_buf, vim_unescape_ks};
#[cfg(not(test))]
use crate::log::log_get_stderr;
#[cfg(not(test))]
use crate::lua::base64::lua_CFunction;
use crate::lua::base64::lua_State;
use crate::lua::converter::{LuaRef, nlua_ref_state_t};
#[cfg(not(test))]
use crate::lua::converter::{
    lua_Integer, lua_checkstack, lua_concat, lua_createtable, lua_error, lua_getfield,
    lua_getmetatable, lua_gettop, lua_isnumber, lua_isstring, lua_newuserdata, lua_next, lua_pcall,
    lua_pushboolean, lua_pushcclosure, lua_pushinteger, lua_pushlstring, lua_pushnil,
    lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawgeti, lua_rawseti, lua_setfield,
    lua_setmetatable, lua_settop, lua_toboolean, lua_tointeger, lua_tolstring, lua_touserdata,
    lua_type, luaL_checkinteger, luaL_checklstring, luaL_error, luaL_ref, luaL_unref, luaL_where,
    nlua_init_types, nlua_pop_Array, nlua_pop_Integer, nlua_pop_Object, nlua_pop_typval,
    nlua_push_Array, nlua_push_Object, nlua_push_typval,
};
#[cfg(not(test))]
use crate::lua::stdlib::nlua_state_add_stdlib;
#[cfg(not(test))]
use crate::lua::treesitter::{nlua_treesitter_free, nlua_treesitter_init};
#[cfg(not(test))]
use crate::main_init::{getout, os_exit, preserve_exit};
use crate::map_glyph_cache::NvimString;
#[cfg(not(test))]
use crate::math::{FAIL, OK};
#[cfg(not(test))]
use crate::memline::{ml_get_buf, ml_get_buf_len, ml_replace};
use crate::memory::{Arena, xfree};
#[cfg(not(test))]
use crate::memory::{
    ArenaMem, arena_finish, arena_mem_free, strequal, xcalloc, xmalloc, xmallocz, xmemdupz, xstrdup,
};
#[cfg(not(test))]
use crate::message::{
    emsg, msg_ext_no_fast, msg_multihl, msg_putchar, msg_use_printf, redirecting, semsg_multiline,
};
#[cfg(not(test))]
use crate::msgpack_rpc::channel::{rpc_send_call, rpc_send_event};
#[cfg(not(test))]
use crate::msgpack_rpc::packer::handle_T;
#[cfg(not(test))]
use crate::option_vars::*;
use crate::os::env::expand_T;
#[cfg(not(test))]
use crate::os::env::home_replace_save;
#[cfg(all(not(test), windows))]
use crate::os::env::os_getenv_noalloc;
#[cfg(not(test))]
use crate::os::fileio::{FileDescriptor, file_close, file_open_stdin, file_read};
#[cfg(not(test))]
use crate::path::fix_fname;
#[cfg(not(test))]
use crate::profile::{proftime_T, scriptitem_T, time_msg, time_pop, time_push};
#[cfg(not(test))]
use crate::runtime::{
    cmd_source_buffer, find_script_by_name, new_script_item, runtime_get_named_thread,
    runtime_search_path_validate, script_is_lua,
};
#[cfg(not(test))]
use crate::strings::{gettext, vim_snprintf};
#[cfg(not(test))]
use crate::ui::{ui_add_cb, ui_flush, ui_has, ui_remove_cb};
#[cfg(not(test))]
use crate::undo::u_save;
#[cfg(not(test))]
use crate::usercmd::{uc_mods, uc_split_args_iter};
#[cfg(not(test))]
use std::ffi::c_void;
use std::ffi::{CStr, c_char, c_int};
#[cfg(not(test))]
use std::io::Write;
#[cfg(not(test))]
use std::mem::{size_of, zeroed};
use std::ptr;

pub use crate::api::vim::HlMessage;
pub use crate::cmdexpand::LineGetter;
pub type LuaRetMode = c_int;

#[cfg(not(test))]
type luv_CFpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        flags: c_int,
    ) -> c_int,
>;
#[cfg(not(test))]
type luv_CFcpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        func: lua_CFunction,
        ud: *mut c_void,
        flags: c_int,
    ) -> c_int,
>;
#[cfg(not(test))]
type luv_acquire_vm = Option<unsafe extern "C-unwind" fn() -> *mut lua_State>;
#[cfg(not(test))]
type luv_release_vm = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State)>;

pub const kRetObject: LuaRetMode = 0;
pub const kRetNilBool: LuaRetMode = 1;
pub const kRetLuaref: LuaRetMode = 2;
pub const kRetMulti: LuaRetMode = 3;
pub const kRetMultiStack: LuaRetMode = 4;

#[cfg(not(test))]
const NUL: c_char = 0;
#[cfg(not(test))]
const MAX_FUNC_ARGS: c_int = 20;
#[cfg(not(test))]
const MB_MAXBYTES: usize = 30;

const LUA_MULTRET: c_int = -1;
#[cfg(not(test))]
pub(crate) const LUA_REGISTRYINDEX: c_int = -10000;
#[cfg(not(test))]
pub(crate) const LUA_GLOBALSINDEX: c_int = -10002;

const LUA_NOREF: LuaRef = -2;
#[cfg(not(test))]
const LUA_REFNIL: LuaRef = -1;

#[cfg(not(test))]
const LUA_ERRRUN: c_int = 2;
#[cfg(not(test))]
const LUA_ERRMEM: c_int = 4;

#[cfg(not(test))]
const K_NLUA_PUSH_SPECIAL: c_int = 0x01;

#[cfg(not(test))]
const LUVF_CALLBACK_NOEXIT: c_int = 0x01;

#[cfg(not(test))]
const kCallback: isize = 0;
#[cfg(not(test))]
const kThread: isize = 1;
#[cfg(not(test))]
const kThreadCallback: isize = 2;

#[cfg(not(test))]
const VAR_FIXED: c_int = 2;

#[cfg(not(test))]
const kUICmdline: c_int = 0;
#[cfg(not(test))]
const kUIGlobalCount: usize = 5;

#[cfg(not(test))]
const WSP_VERT: c_int = 0x02;
#[cfg(not(test))]
const WSP_HOR: c_int = 0x04;
#[cfg(not(test))]
const WSP_TOP: c_int = 0x08;
#[cfg(not(test))]
const WSP_BOT: c_int = 0x10;
#[cfg(not(test))]
const WSP_BELOW: c_int = 0x40;
#[cfg(not(test))]
const WSP_ABOVE: c_int = 0x80;

#[cfg(not(test))]
const CMOD_SANDBOX: c_int = 0x0001;
#[cfg(not(test))]
const CMOD_SILENT: c_int = 0x0002;
#[cfg(not(test))]
const CMOD_ERRSILENT: c_int = 0x0004;
#[cfg(not(test))]
const CMOD_UNSILENT: c_int = 0x0008;
#[cfg(not(test))]
const CMOD_NOAUTOCMD: c_int = 0x0010;
#[cfg(not(test))]
const CMOD_HIDE: c_int = 0x0020;
#[cfg(not(test))]
const CMOD_BROWSE: c_int = 0x0040;
#[cfg(not(test))]
const CMOD_CONFIRM: c_int = 0x0080;
#[cfg(not(test))]
const CMOD_KEEPALT: c_int = 0x0100;
#[cfg(not(test))]
const CMOD_KEEPMARKS: c_int = 0x0200;
#[cfg(not(test))]
const CMOD_KEEPJUMPS: c_int = 0x0400;
#[cfg(not(test))]
const CMOD_LOCKMARKS: c_int = 0x0800;
#[cfg(not(test))]
const CMOD_KEEPPATTERNS: c_int = 0x1000;
#[cfg(not(test))]
const CMOD_NOSWAPFILE: c_int = 0x2000;

#[cfg(not(test))]
const EX_EXTRA: u32 = 0x00000004;
#[cfg(not(test))]
const EX_NOSPC: u32 = 0x00000010;
#[cfg(not(test))]
const EX_NEEDARG: u32 = 0x00000080;
#[cfg(not(test))]
const EX_ARGSPACE: u32 = 0x40000000;

#[cfg(not(test))]
const CMD_equal: c_int = 559;
#[cfg(not(test))]
const CMD_SIZE: c_int = 564;
const CMD_USER: c_int = -1;
const CMD_USER_BUF: c_int = -2;

const OBJECT_NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[derive(Clone, Copy)]
struct BuiltinModule {
    name: &'static CStr,
    chunk_name: &'static CStr,
    data: &'static [u8],
}

// NB: vim._init_packages and vim.inspect must be first and second respectively,
// otherwise --luamod-dev won't work properly.
const BUILTIN_MODULES: [BuiltinModule; 34] = [
    BuiltinModule {
        name: c"vim._init_packages",
        chunk_name: c"@vim/_init_packages",
        data: include_bytes!("../../../runtime/lua/vim/_init_packages.lua"),
    },
    BuiltinModule {
        name: c"vim.inspect",
        chunk_name: c"@vim/inspect",
        data: include_bytes!("../../../runtime/lua/vim/inspect.lua"),
    },
    BuiltinModule {
        name: c"vim.filetype",
        chunk_name: c"@vim/filetype",
        data: include_bytes!("../../../runtime/lua/vim/filetype.lua"),
    },
    BuiltinModule {
        name: c"vim.fs",
        chunk_name: c"@vim/fs",
        data: include_bytes!("../../../runtime/lua/vim/fs.lua"),
    },
    BuiltinModule {
        name: c"vim.F",
        chunk_name: c"@vim/F",
        data: include_bytes!("../../../runtime/lua/vim/F.lua"),
    },
    BuiltinModule {
        name: c"vim.keymap",
        chunk_name: c"vim/keymap",
        data: include_bytes!("../../../runtime/lua/vim/keymap.lua"),
    },
    BuiltinModule {
        name: c"vim.loader",
        chunk_name: c"@vim/loader",
        data: include_bytes!("../../../runtime/lua/vim/loader.lua"),
    },
    BuiltinModule {
        name: c"vim.text",
        chunk_name: c"@vim/text",
        data: include_bytes!("../../../runtime/lua/vim/text.lua"),
    },
    BuiltinModule {
        name: c"vim.tty",
        chunk_name: c"@vim/tty",
        data: include_bytes!("../../../runtime/lua/vim/tty.lua"),
    },
    BuiltinModule {
        name: c"vim._core.cmdwin",
        chunk_name: c"@vim/_core/cmdwin",
        data: include_bytes!("../../../runtime/lua/vim/_core/cmdwin.lua"),
    },
    BuiltinModule {
        name: c"vim._core.defaults",
        chunk_name: c"@vim/_core/defaults",
        data: include_bytes!("../../../runtime/lua/vim/_core/defaults.lua"),
    },
    BuiltinModule {
        name: c"vim._core.editor",
        chunk_name: c"vim/_core/editor",
        data: include_bytes!("../../../runtime/lua/vim/_core/editor.lua"),
    },
    BuiltinModule {
        name: c"vim._core.ex_cmd",
        chunk_name: c"@vim/_core/ex_cmd",
        data: include_bytes!("../../../runtime/lua/vim/_core/ex_cmd.lua"),
    },
    BuiltinModule {
        name: c"vim._core.exmode",
        chunk_name: c"@vim/_core/exmode",
        data: include_bytes!("../../../runtime/lua/vim/_core/exmode.lua"),
    },
    BuiltinModule {
        name: c"vim._core.exrc",
        chunk_name: c"@vim/_core/exrc",
        data: include_bytes!("../../../runtime/lua/vim/_core/exrc.lua"),
    },
    BuiltinModule {
        name: c"vim._core.help",
        chunk_name: c"@vim/_core/help",
        data: include_bytes!("../../../runtime/lua/vim/_core/help.lua"),
    },
    BuiltinModule {
        name: c"vim._core.log",
        chunk_name: c"@vim/_core/log",
        data: include_bytes!("../../../runtime/lua/vim/_core/log.lua"),
    },
    BuiltinModule {
        name: c"vim._core.marks",
        chunk_name: c"@vim/_core/marks",
        data: include_bytes!("../../../runtime/lua/vim/_core/marks.lua"),
    },
    BuiltinModule {
        name: c"vim._core.mcursor",
        chunk_name: c"@vim/_core/mcursor",
        data: include_bytes!("../../../runtime/lua/vim/_core/mcursor.lua"),
    },
    BuiltinModule {
        name: c"vim._core.options",
        chunk_name: c"vim/_core/options",
        data: include_bytes!("../../../runtime/lua/vim/_core/options.lua"),
    },
    BuiltinModule {
        name: c"vim._core.proc",
        chunk_name: c"@vim/_core/proc",
        data: include_bytes!("../../../runtime/lua/vim/_core/proc.lua"),
    },
    BuiltinModule {
        name: c"vim._core.server",
        chunk_name: c"@vim/_core/server",
        data: include_bytes!("../../../runtime/lua/vim/_core/server.lua"),
    },
    BuiltinModule {
        name: c"vim._core.shared",
        chunk_name: c"@vim/_core/shared",
        data: include_bytes!("../../../runtime/lua/vim/_core/shared.lua"),
    },
    BuiltinModule {
        name: c"vim._core.spell",
        chunk_name: c"@vim/_core/spell",
        data: include_bytes!("../../../runtime/lua/vim/_core/spell.lua"),
    },
    BuiltinModule {
        name: c"vim._core.stringbuffer",
        chunk_name: c"@vim/_core/stringbuffer",
        data: include_bytes!("../../../runtime/lua/vim/_core/stringbuffer.lua"),
    },
    BuiltinModule {
        name: c"vim._core.swapfile",
        chunk_name: c"@vim/_core/swapfile",
        data: include_bytes!("../../../runtime/lua/vim/_core/swapfile.lua"),
    },
    BuiltinModule {
        name: c"vim._core.system",
        chunk_name: c"@vim/_core/system",
        data: include_bytes!("../../../runtime/lua/vim/_core/system.lua"),
    },
    BuiltinModule {
        name: c"vim._core.table",
        chunk_name: c"@vim/_core/table",
        data: include_bytes!("../../../runtime/lua/vim/_core/table.lua"),
    },
    BuiltinModule {
        name: c"vim._core.tag",
        chunk_name: c"@vim/_core/tag",
        data: include_bytes!("../../../runtime/lua/vim/_core/tag.lua"),
    },
    BuiltinModule {
        name: c"vim._core.time",
        chunk_name: c"@vim/_core/time",
        data: include_bytes!("../../../runtime/lua/vim/_core/time.lua"),
    },
    BuiltinModule {
        name: c"vim._core.ui",
        chunk_name: c"@vim/_core/ui",
        data: include_bytes!("../../../runtime/lua/vim/_core/ui.lua"),
    },
    BuiltinModule {
        name: c"vim._core.ui2",
        chunk_name: c"@vim/_core/ui2",
        data: include_bytes!("../../../runtime/lua/vim/_core/ui2.lua"),
    },
    BuiltinModule {
        name: c"vim._core.util",
        chunk_name: c"@vim/_core/util",
        data: include_bytes!("../../../runtime/lua/vim/_core/util.lua"),
    },
    BuiltinModule {
        name: c"vim._core.vimfn",
        chunk_name: c"@vim/_core/vimfn",
        data: include_bytes!("../../../runtime/lua/vim/_core/vimfn.lua"),
    },
];

#[cfg(not(test))]
#[repr(C)]
struct ExDocmdCookie {
    getline: LuaRef,
    got_line: bool,
}

#[cfg(not(test))]
#[repr(C)]
struct lua_Debug {
    event: c_int,
    name: *const c_char,
    namewhat: *const c_char,
    what: *const c_char,
    source: *const c_char,
    currentline: c_int,
    nups: c_int,
    linedefined: c_int,
    lastlinedefined: c_int,
    short_src: [c_char; 60],
    i_ci: c_int,
    _pad: [u8; 32],
}

static mut IN_FAST_CALLBACK: c_int = 0;
static mut GLOBAL_LSTATE: *mut lua_State = ptr::null_mut();

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut active_lstate: *mut lua_State = ptr::null_mut();

#[cfg(not(test))]
static mut REQUIRE_REF: LuaRef = LUA_REFNIL;
#[cfg(not(test))]
static mut MAIN_THREAD: uv_thread_t = 0;
static mut EXPAND_RESULT_ARRAY: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};

#[cfg(not(test))]
unsafe extern "C-unwind" {
    fn luaL_newstate() -> *mut lua_State;
    fn luaL_openlibs(L: *mut lua_State);
    fn lua_close(L: *mut lua_State);
    fn lua_remove(L: *mut lua_State, idx: c_int);
    fn lua_insert(L: *mut lua_State, idx: c_int);
    fn lua_replace(L: *mut lua_State, idx: c_int);
    fn lua_iscfunction(L: *mut lua_State, idx: c_int) -> c_int;
    fn lua_tocfunction(L: *mut lua_State, idx: c_int) -> lua_CFunction;
    fn lua_pushlightuserdata(L: *mut lua_State, p: *mut c_void);
    fn lua_call(L: *mut lua_State, nargs: c_int, nresults: c_int);
    fn lua_getstack(L: *mut lua_State, level: c_int, ar: *mut lua_Debug) -> c_int;
    fn lua_getinfo(L: *mut lua_State, what: *const c_char, ar: *mut lua_Debug) -> c_int;

    fn luaL_checktype(L: *mut lua_State, narg: c_int, t: c_int);
    fn luaL_loadbuffer(
        L: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
    ) -> c_int;

    fn luaopen_luv(L: *mut lua_State) -> c_int;
}

pub use crate::api::private::dispatch::nlua_add_api_functions;

#[cfg(all(windows, not(test)))]
unsafe extern "system" {
    fn ExitThread(dwExitCode: u32) -> !;
}

#[cfg(not(test))]
use crate::api::autocmd::kCallbackNone;
#[cfg(not(test))]
use crate::api::buffer::LUA_INTERNAL_CALL;
#[cfg(not(test))]
use crate::api::options::IOSIZE;
#[cfg(not(test))]
use crate::lua::base64::LUA_TSTRING;
#[cfg(not(test))]
use crate::lua::converter::{LUA_TBOOLEAN, LUA_TFUNCTION, LUA_TNIL, LUA_TTABLE};
#[cfg(not(test))]
use crate::version::main_exitfree;

#[cfg(not(test))]
unsafe extern "C" {
    fn luv_set_loop(L: *mut lua_State, loop_: *mut c_void);
    fn luv_set_callback(L: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_thread(L: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_cthread(L: *mut lua_State, cpcall: luv_CFcpcall);
    fn luv_set_thread_cb(acquire: luv_acquire_vm, release: luv_release_vm);

    fn uv_thread_self() -> uv_thread_t;
    fn uv_thread_equal(t1: *const uv_thread_t, t2: *const uv_thread_t) -> c_int;

    #[cfg(unix)]
    fn pthread_exit(retval: *mut c_void) -> !;
}

#[cfg(not(test))]
#[inline]
const fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_GLOBALSINDEX - i
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_pop(lstate: *mut lua_State, n: c_int) {
    unsafe {
        lua_settop(lstate, -(n) - 1);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_newtable(lstate: *mut lua_State) {
    unsafe {
        lua_createtable(lstate, 0, 0);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_pushcfunction(lstate: *mut lua_State, f: lua_CFunction) {
    unsafe {
        lua_pushcclosure(lstate, f, 0);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_setglobal(lstate: *mut lua_State, s: *const c_char) {
    unsafe {
        lua_setfield(lstate, LUA_GLOBALSINDEX, s);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_getglobal(lstate: *mut lua_State, s: *const c_char) {
    unsafe {
        lua_getfield(lstate, LUA_GLOBALSINDEX, s);
    }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_tostring(lstate: *mut lua_State, i: c_int) -> *const c_char {
    unsafe { lua_tolstring(lstate, i, ptr::null_mut()) }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isfunction(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TFUNCTION }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_istable(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TTABLE }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isnil(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TNIL }
}

#[cfg(not(test))]
#[inline]
unsafe fn lua_isboolean(lstate: *mut lua_State, n: c_int) -> bool {
    unsafe { lua_type(lstate, n) == LUA_TBOOLEAN }
}

#[inline]
const fn is_user_cmdidx(idx: c_int) -> bool {
    idx == CMD_USER || idx == CMD_USER_BUF
}

#[cfg(not(test))]
#[inline]
unsafe fn put_event(
    mq: *mut MultiQueue,
    handler: unsafe extern "C" fn(*mut *mut c_void),
    arg0: *mut c_void,
    arg1: *mut c_void,
) {
    unsafe {
        if !mq.is_null() {
            let mut ev = Event {
                handler: Some(handler),
                argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
            };
            ev.argv[0] = arg0;
            ev.argv[1] = arg1;
            multiqueue_put_event(mq, ev);
        }
    }
}

#[cfg(not(test))]
unsafe fn push_all_typvals(
    lstate: *mut lua_State,
    args: *mut typval_T,
    argcount: c_int,
    special: bool,
) {
    unsafe {
        let flags = if special { K_NLUA_PUSH_SPECIAL } else { 0 };
        for i in 0..(argcount as usize) {
            let arg = args.add(i);
            if (*arg).v_type == VarType::VAR_UNKNOWN {
                lua_pushnil(lstate);
            } else {
                nlua_push_typval(lstate, arg, flags);
            }
        }
    }
}

#[cfg(not(test))]
unsafe fn nlua_push_cmdmod(lstate: *mut lua_State, cmod: *const cmdmod_T) {
    unsafe {
        lua_newtable(lstate);

        lua_pushinteger(lstate, ((*cmod).cmod_tab - 1) as lua_Integer);
        lua_setfield(lstate, -2, c"tab".as_ptr());

        lua_pushinteger(lstate, ((*cmod).cmod_verbose - 1) as lua_Integer);
        lua_setfield(lstate, -2, c"verbose".as_ptr());

        let split_str = if ((*cmod).cmod_split & WSP_ABOVE) != 0 {
            c"aboveleft"
        } else if ((*cmod).cmod_split & WSP_BELOW) != 0 {
            c"belowright"
        } else if ((*cmod).cmod_split & WSP_TOP) != 0 {
            c"topleft"
        } else if ((*cmod).cmod_split & WSP_BOT) != 0 {
            c"botright"
        } else {
            c""
        };
        lua_pushstring(lstate, split_str.as_ptr());
        lua_setfield(lstate, -2, c"split".as_ptr());

        lua_pushboolean(lstate, c_int::from(((*cmod).cmod_split & WSP_VERT) != 0));
        lua_setfield(lstate, -2, c"vertical".as_ptr());
        lua_pushboolean(lstate, c_int::from(((*cmod).cmod_split & WSP_HOR) != 0));
        lua_setfield(lstate, -2, c"horizontal".as_ptr());
        lua_pushboolean(lstate, c_int::from(((*cmod).cmod_flags & CMOD_SILENT) != 0));
        lua_setfield(lstate, -2, c"silent".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_ERRSILENT) != 0),
        );
        lua_setfield(lstate, -2, c"emsg_silent".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_UNSILENT) != 0),
        );
        lua_setfield(lstate, -2, c"unsilent".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_SANDBOX) != 0),
        );
        lua_setfield(lstate, -2, c"sandbox".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_NOAUTOCMD) != 0),
        );
        lua_setfield(lstate, -2, c"noautocmd".as_ptr());
        lua_pushboolean(lstate, c_int::from(((*cmod).cmod_flags & CMOD_BROWSE) != 0));
        lua_setfield(lstate, -2, c"browse".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_CONFIRM) != 0),
        );
        lua_setfield(lstate, -2, c"confirm".as_ptr());
        lua_pushboolean(lstate, c_int::from(((*cmod).cmod_flags & CMOD_HIDE) != 0));
        lua_setfield(lstate, -2, c"hide".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_KEEPALT) != 0),
        );
        lua_setfield(lstate, -2, c"keepalt".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_KEEPJUMPS) != 0),
        );
        lua_setfield(lstate, -2, c"keepjumps".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_KEEPMARKS) != 0),
        );
        lua_setfield(lstate, -2, c"keepmarks".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_KEEPPATTERNS) != 0),
        );
        lua_setfield(lstate, -2, c"keeppatterns".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_LOCKMARKS) != 0),
        );
        lua_setfield(lstate, -2, c"lockmarks".as_ptr());
        lua_pushboolean(
            lstate,
            c_int::from(((*cmod).cmod_flags & CMOD_NOSWAPFILE) != 0),
        );
        lua_setfield(lstate, -2, c"noswapfile".as_ptr());

        lua_newtable(lstate);
        lua_pushstring(
            lstate,
            if !(*cmod).cmod_filter_pat.is_null() {
                (*cmod).cmod_filter_pat
            } else {
                c"".as_ptr()
            },
        );
        lua_setfield(lstate, -2, c"pattern".as_ptr());
        lua_pushboolean(lstate, c_int::from((*cmod).cmod_filter_force));
        lua_setfield(lstate, -2, c"force".as_ptr());
        lua_setfield(lstate, -2, c"filter".as_ptr());
    }
}

#[cfg(not(test))]
unsafe fn nlua_push_eap(lstate: *mut lua_State, eap: *mut exarg_T, cmod: *const cmdmod_T) {
    unsafe {
        if !is_user_cmdidx((*eap).cmdidx) && (*eap).cmdidx < CMD_SIZE {
            lua_pushstring(lstate, get_command_name(ptr::null_mut(), (*eap).cmdidx));
            lua_setfield(lstate, -2, c"name".as_ptr());
        }

        let mut mods_buf = [0 as c_char; 200];
        uc_mods(mods_buf.as_mut_ptr(), cmod, false);
        lua_pushstring(lstate, mods_buf.as_ptr());
        lua_setfield(lstate, -2, c"mods".as_ptr());

        nlua_push_cmdmod(lstate, cmod);
        lua_setfield(lstate, -2, c"smods".as_ptr());

        lua_pushstring(lstate, (*eap).arg);
        lua_setfield(lstate, -2, c"args".as_ptr());

        lua_pushboolean(lstate, (*eap).forceit);
        lua_setfield(lstate, -2, c"bang".as_ptr());

        lua_pushinteger(lstate, (*eap).line1 as lua_Integer);
        lua_setfield(lstate, -2, c"line1".as_ptr());

        lua_pushinteger(lstate, (*eap).line2 as lua_Integer);
        lua_setfield(lstate, -2, c"line2".as_ptr());

        lua_pushinteger(lstate, (*eap).addr_count as lua_Integer);
        lua_setfield(lstate, -2, c"range".as_ptr());

        lua_pushinteger(lstate, (*eap).line2 as lua_Integer);
        lua_setfield(lstate, -2, c"count".as_ptr());

        let reg: [c_char; 2] = [(*eap).regname as c_char, NUL];
        lua_pushstring(lstate, reg.as_ptr());
        lua_setfield(lstate, -2, c"reg".as_ptr());

        if !(*eap).args.is_null() {
            lua_createtable(lstate, (*eap).argc as c_int, 0);
            for i in 0..(*eap).argc {
                lua_pushlstring(lstate, *(*eap).args.add(i), *(*eap).arglens.add(i));
                lua_rawseti(lstate, -2, (i + 1) as c_int);
            }
            lua_setfield(lstate, -2, c"fargs".as_ptr());
        } else {
            lua_newtable(lstate);
            let length = CStr::from_ptr((*eap).arg).to_bytes().len();
            if length > 0 {
                let mut end: usize = 0;
                let mut len: usize = 0;
                let mut i: c_int = 1;
                let buf = xcalloc(length, size_of::<c_char>()).cast::<c_char>();
                let mut done = false;
                while !done {
                    done = uc_split_args_iter((*eap).arg, length, &raw mut end, buf, &raw mut len);
                    if len > 0 {
                        lua_pushlstring(lstate, buf, len);
                        lua_rawseti(lstate, -2, i);
                        i += 1;
                    }
                }
                xfree(buf.cast());
            }
            lua_setfield(lstate, -2, c"fargs".as_ptr());
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_global_lstate() -> *mut lua_State {
    unsafe { GLOBAL_LSTATE }
}

#[cfg(not(test))]
unsafe fn nlua_get_error(lstate: *mut lua_State, len: *mut usize) -> *const c_char {
    unsafe {
        if lua_type(lstate, -1) != LUA_TSTRING {
            lua_getglobal(lstate, c"tostring".as_ptr());
            lua_pushvalue(lstate, -2);
            if lua_pcall(lstate, 1, 1, 0) != 0 || lua_type(lstate, -1) != LUA_TSTRING {
                lua_pop(lstate, 1);
                lua_pushstring(lstate, c"[UNPRINTABLE ERROR]".as_ptr());
            }
            lua_replace(lstate, -2);
        }
        lua_tolstring(lstate, -1, len)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_error(lstate: *mut lua_State, msg: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        let mut len: usize = 0;
        let str_ = nlua_get_error(lstate, &raw mut len);
        msg_ext_no_fast();
        semsg_multiline!(c"lua_error".as_ptr(), msg, len as c_int, str_);
        lua_pop(lstate, 1);
    }
    #[cfg(test)]
    {
        let _ = (lstate, msg);
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_no_traceback(_lstate: *mut lua_State) -> c_int {
    1
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_pcall(
    lstate: *mut lua_State,
    nargs: c_int,
    mut nresults: c_int,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        lua_getglobal(lstate, c"debug".as_ptr());
        if lua_istable(lstate, -1) {
            lua_getfield(lstate, -1, c"traceback".as_ptr());
            lua_remove(lstate, -2);
        }
        let no_traceback = !lua_isfunction(lstate, -1);
        if no_traceback {
            lua_pop(lstate, 1);
            lua_pushcfunction(lstate, Some(nlua_no_traceback));
        }
        lua_insert(lstate, -2 - nargs);
        let pre_top = lua_gettop(lstate);
        let status = lua_pcall(lstate, nargs, nresults, -2 - nargs);
        if status != 0 {
            lua_remove(lstate, -2);
            if no_traceback && status == LUA_ERRRUN && lua_isstring(lstate, -1) != 0 {
                lua_pushlstring(
                    lstate,
                    c"\nNo traceback: debug.traceback is not a function".as_ptr(),
                    48,
                );
                lua_concat(lstate, 2);
            }
        } else {
            if nresults == LUA_MULTRET {
                nresults = lua_gettop(lstate) - (pre_top - nargs - 1);
            }
            lua_remove(lstate, -1 - nresults);
        }
        status
    }
    #[cfg(test)]
    {
        let _ = (lstate, nargs, nresults);
        0
    }
}

#[cfg(not(test))]
unsafe extern "C" fn nlua_luv_error_event(argv: *mut *mut c_void) {
    unsafe {
        let error = (*argv).cast::<c_char>();
        let err_type = *argv.add(1) as isize;
        match err_type {
            kCallback => {
                msg_ext_no_fast();
                semsg_multiline!(c"lua_error".as_ptr(), c"Lua callback:\n%s".as_ptr(), error);
            }
            kThread => {
                semsg_multiline!(c"lua_error".as_ptr(), c"Luv thread:\n%s".as_ptr(), error);
            }
            kThreadCallback => {
                semsg_multiline!(
                    c"lua_error".as_ptr(),
                    c"Luv callback, thread:\n%s".as_ptr(),
                    error,
                );
            }
            _ => {}
        }
        xfree(error.cast());
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_fast_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    mut nresult: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        IN_FAST_CALLBACK += 1;

        let top = lua_gettop(lstate);
        let status = nlua_pcall(lstate, nargs, nresult);
        let retval = if status != 0 {
            if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
                preserve_exit((&raw const e_outofmem).cast());
            }

            let mut len: usize = 0;
            let error = nlua_get_error(lstate, &raw mut len);
            let queues = loop_queues((&raw mut main_loop).cast());
            put_event(
                (*queues).events,
                nlua_luv_error_event,
                if !error.is_null() {
                    xstrdup(error).cast()
                } else {
                    ptr::null_mut()
                },
                kCallback as *mut c_void,
            );
            lua_pop(lstate, 1);
            -status
        } else {
            if nresult == LUA_MULTRET {
                nresult = lua_gettop(lstate) - top + nargs + 1;
            }
            nresult
        };

        IN_FAST_CALLBACK -= 1;
        retval
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_luv_thread_cb_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    nresult: c_int,
    flags: c_int,
) -> c_int {
    unsafe { nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, true) }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_luv_thread_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    nresult: c_int,
    flags: c_int,
) -> c_int {
    unsafe { nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, false) }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_luv_thread_cfcpcall(
    lstate: *mut lua_State,
    func: lua_CFunction,
    ud: *mut c_void,
    flags: c_int,
) -> c_int {
    unsafe {
        lua_pushcfunction(lstate, func);
        lua_pushlightuserdata(lstate, ud);
        nlua_luv_thread_cfpcall(lstate, 1, 0, flags)
    }
}

#[cfg(not(test))]
unsafe fn nlua_luv_thread_common_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    mut nresult: c_int,
    flags: c_int,
    is_callback: bool,
) -> c_int {
    unsafe {
        let top = lua_gettop(lstate);
        let status = lua_pcall(lstate, nargs, nresult, 0);
        if status != 0 {
            if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
                fprintf(
                    log_get_stderr(),
                    c"%s\n".as_ptr(),
                    (&raw const e_outofmem).cast::<c_char>(),
                );
                lua_close(lstate);
                #[cfg(windows)]
                {
                    ExitThread(0);
                }
                #[cfg(not(windows))]
                {
                    pthread_exit(ptr::null_mut());
                }
            }
            let mut len: usize = 0;
            let error = nlua_get_error(lstate, &raw mut len);
            let mut ev = Event {
                handler: Some(nlua_luv_error_event),
                argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
            };
            ev.argv[0] = xmemdupz(error.cast(), len);
            ev.argv[1] = (if is_callback {
                kThreadCallback
            } else {
                kThread
            }) as *mut c_void;
            loop_schedule_deferred((&raw mut main_loop).cast(), ev);
            lua_pop(lstate, 1);
            -status
        } else {
            if nresult == LUA_MULTRET {
                nresult = lua_gettop(lstate) - top + nargs + 1;
            }
            nresult
        }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_thr_api_nvim__get_runtime(lstate: *mut lua_State) -> c_int {
    unsafe {
        if lua_gettop(lstate) != 3 {
            return luaL_error(lstate, c"Expected 3 arguments".as_ptr());
        }

        luaL_checktype(lstate, -1, LUA_TTABLE);
        lua_getfield(lstate, -1, c"is_lua".as_ptr());
        if !lua_isboolean(lstate, -1) {
            return luaL_error(lstate, c"is_lua is not a boolean".as_ptr());
        }
        let is_lua = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 2);

        luaL_checktype(lstate, -1, LUA_TBOOLEAN);
        let all = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 1);

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let pat = nlua_pop_Array(lstate, ptr::null_mut(), &raw mut err);
        if error_set(&raw const err) {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg);
            api_clear_error(&raw mut err);
            lua_concat(lstate, 2);
            return lua_error(lstate);
        }

        let ret = runtime_get_named_thread(is_lua, pat, all);
        nlua_push_Array(lstate, ret, K_NLUA_PUSH_SPECIAL);
        api_free_array(ret);
        api_free_array(pat);

        1
    }
}

#[cfg(not(test))]
unsafe fn nlua_init_argv(
    L: *mut lua_State,
    argv: *mut *mut c_char,
    argc: c_int,
    lua_arg0: c_int,
) -> c_int {
    unsafe {
        let mut i: c_int = 0;
        lua_newtable(L);

        if lua_arg0 > 0 {
            lua_pushstring(L, *argv.add((lua_arg0 - 1) as usize));
            lua_rawseti(L, -2, 0);

            while i + lua_arg0 < argc {
                lua_pushstring(L, *argv.add((i + lua_arg0) as usize));
                lua_rawseti(L, -2, i + 1);
                i += 1;
            }
        }

        lua_setglobal(L, c"arg".as_ptr());
        i
    }
}

#[cfg(not(test))]
unsafe extern "C" fn nlua_schedule_event(argv: *mut *mut c_void) {
    unsafe {
        let cb = *argv as isize as LuaRef;
        let ns_id = *argv.add(1) as usize as u32;
        let lstate = GLOBAL_LSTATE;

        nlua_pushref(lstate, cb);
        nlua_unref_global(lstate, cb);

        let save_expr_map_lock = expr_map_lock;
        let save_textlock = textlock;
        if ns_id > 0 {
            expr_map_lock = 0;
            textlock = 0;
        }
        if nlua_pcall(lstate, 0, 0) != 0 {
            nlua_error(lstate, gettext(c"vim.schedule callback: %.*s".as_ptr()));
            ui_remove_cb(ns_id, true);
        }
        expr_map_lock = save_expr_map_lock;
        textlock = save_textlock;
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_schedule(lstate: *mut lua_State) -> c_int {
    unsafe {
        if lua_type(lstate, 1) != LUA_TFUNCTION {
            lua_pushlstring(lstate, c"vim.schedule: expected function".as_ptr(), 31);
            return lua_error(lstate);
        }

        lua_pushnil(lstate);
        if *loop_closing_ptr((&raw mut main_loop).cast()) {
            lua_pushlstring(lstate, c"main loop is closing".as_ptr(), 20);
            return 2;
        }

        let cb = nlua_ref_global(lstate, 1);
        let queues = loop_queues((&raw mut main_loop).cast());
        put_event(
            (*queues).events,
            nlua_schedule_event,
            cb as isize as *mut c_void,
            ui_event_ns_id as usize as *mut c_void,
        );
        lua_pushnil(lstate);
        2
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_loop_poll(lstate: *mut lua_State) -> c_int {
    unsafe {
        let timeout = luaL_checkinteger(lstate, 1) as i64;
        let fast_only = lua_toboolean(lstate, 2) != 0;

        let queues = loop_queues((&raw mut main_loop).cast());
        let loop_events = if fast_only {
            (*queues).fast_events
        } else {
            (*queues).events
        };
        if !loop_events.is_null() && !multiqueue_empty(loop_events) {
            multiqueue_process_events(loop_events);
        } else {
            loop_poll_events((&raw mut main_loop).cast(), timeout);
        }

        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_ui_flush(_lstate: *mut lua_State) -> c_int {
    unsafe {
        ui_flush();
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_check_interrupt(lstate: *mut lua_State) -> c_int {
    unsafe {
        let interrupted = got_int;
        if got_int {
            got_int = false;
            vgetc();
        }
        lua_pushboolean(lstate, c_int::from(interrupted));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C" fn nlua_ex_getline(
    _c: c_int,
    cookie: *mut c_void,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
        let ecd = cookie.cast::<ExDocmdCookie>();
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, (*ecd).getline);
        if nlua_pcall(lstate, 0, 1) != 0 {
            nlua_error(
                lstate,
                gettext(c"Error executing Ex-mode line getter: %.*s".as_ptr()),
            );
            return ptr::null_mut();
        }
        if lua_type(lstate, -1) != LUA_TSTRING {
            lua_pop(lstate, 1);
            return ptr::null_mut();
        }
        let line = xstrdup(lua_tostring(lstate, -1));
        lua_pop(lstate, 1);
        (*ecd).got_line = true;
        line
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_ex_docmd(lstate: *mut lua_State) -> c_int {
    unsafe {
        luaL_checktype(lstate, 1, LUA_TFUNCTION);
        let mut cookie = ExDocmdCookie {
            getline: nlua_ref_global(lstate, 1),
            got_line: false,
        };
        msg_scroll = 1;
        need_wait_return = false;
        no_wait_return += 1;
        do_cmdline(
            ptr::null_mut(),
            Some(nlua_ex_getline),
            (&raw mut cookie).cast(),
            0,
        );
        no_wait_return -= 1;
        nlua_unref_global(lstate, cookie.getline);
        lua_pushboolean(lstate, c_int::from(cookie.got_line));
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_os_exit(lstate: *mut lua_State) -> c_int {
    unsafe {
        if IN_FAST_CALLBACK > 0 {
            return luaL_error(
                lstate,
                (&raw const e_fast_api_disabled).cast(),
                c"os.exit".as_ptr(),
            );
        }

        let mut status: c_int = 0;
        if lua_gettop(lstate) >= 1 && !lua_isnil(lstate, 1) {
            status = if lua_isboolean(lstate, 1) {
                if lua_toboolean(lstate, 1) != 0 { 0 } else { 1 }
            } else {
                luaL_checkinteger(lstate, 1) as c_int
            };
        }
        getout(status);
    }
}

#[cfg(not(test))]
unsafe fn nlua_new_ref_state(lstate: *mut lua_State, is_thread: bool) -> *mut nlua_ref_state_t {
    unsafe {
        let ref_state =
            lua_newuserdata(lstate, size_of::<nlua_ref_state_t>()).cast::<nlua_ref_state_t>();
        *ref_state = nlua_ref_state_t {
            nil_ref: LUA_NOREF,
            empty_dict_ref: LUA_NOREF,
            ref_count: 0,
        };
        if !is_thread {
            nlua_global_refs = ref_state;
        }
        ref_state
    }
}

#[cfg(not(test))]
unsafe fn nlua_get_ref_state(lstate: *mut lua_State) -> *mut nlua_ref_state_t {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state".as_ptr());
        let ref_state = lua_touserdata(lstate, -1).cast::<nlua_ref_state_t>();
        lua_pop(lstate, 1);
        ref_state
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_get_nil_ref(lstate: *mut lua_State) -> LuaRef {
    #[cfg(not(test))]
    unsafe {
        (*nlua_get_ref_state(lstate)).nil_ref
    }
    #[cfg(test)]
    {
        let _ = lstate;
        LUA_NOREF
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_get_empty_dict_ref(lstate: *mut lua_State) -> LuaRef {
    #[cfg(not(test))]
    unsafe {
        (*nlua_get_ref_state(lstate)).empty_dict_ref
    }
    #[cfg(test)]
    {
        let _ = lstate;
        LUA_NOREF
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_get_global_ref_count() -> c_int {
    #[cfg(not(test))]
    unsafe {
        (*nlua_global_refs).ref_count
    }
    #[cfg(test)]
    {
        0
    }
}

#[cfg(not(test))]
unsafe fn nlua_common_vim_init(lstate: *mut lua_State, is_thread: bool) {
    unsafe {
        let ref_state = nlua_new_ref_state(lstate, is_thread);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state".as_ptr());

        lua_pushboolean(lstate, c_int::from(is_thread));
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_is_thread));
        lua_setfield(lstate, -2, c"is_thread".as_ptr());

        lua_createtable(lstate, 0, 0);
        lua_setfield(lstate, -2, c"_core".as_ptr());

        lua_newuserdata(lstate, 0);
        lua_createtable(lstate, 0, 0);
        lua_pushcfunction(lstate, Some(nlua_nil_tostring));
        lua_setfield(lstate, -2, c"__tostring".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_nil_index));
        lua_setfield(lstate, -2, c"__index".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_nil_index));
        lua_setfield(lstate, -2, c"__newindex".as_ptr());
        lua_setmetatable(lstate, -2);
        (*ref_state).nil_ref = nlua_ref(lstate, ref_state, -1);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.NIL".as_ptr());
        lua_setfield(lstate, -2, c"NIL".as_ptr());

        lua_createtable(lstate, 0, 0);
        lua_pushcfunction(lstate, Some(nlua_empty_dict_tostring));
        lua_setfield(lstate, -2, c"__tostring".as_ptr());
        (*ref_state).empty_dict_ref = nlua_ref(lstate, ref_state, -1);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.empty_dict".as_ptr());
        lua_setfield(lstate, -2, c"_empty_dict_mt".as_ptr());

        if is_thread {
            luv_set_callback(lstate, Some(nlua_luv_thread_cb_cfpcall));
            luv_set_thread(lstate, Some(nlua_luv_thread_cfpcall));
            luv_set_cthread(lstate, Some(nlua_luv_thread_cfcpcall));
        } else {
            luv_set_loop(lstate, (&raw mut main_loop).cast());
            luv_set_callback(lstate, Some(nlua_fast_cfpcall));
        }
        luaopen_luv(lstate);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -3, c"uv".as_ptr());

        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -3, c"loop".as_ptr());

        lua_getglobal(lstate, c"package".as_ptr());
        lua_getfield(lstate, -1, c"loaded".as_ptr());
        lua_pushvalue(lstate, -3);
        lua_setfield(lstate, -2, c"luv".as_ptr());
        lua_pop(lstate, 3);
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_module_preloader(lstate: *mut lua_State) -> c_int {
    unsafe {
        let i = lua_tointeger(lstate, lua_upvalueindex(1)) as usize;
        let def = BUILTIN_MODULES[i];
        if luaL_loadbuffer(
            lstate,
            def.data.as_ptr().cast(),
            def.data.len(),
            def.chunk_name.as_ptr(),
        ) != 0
        {
            return lua_error(lstate);
        }
        lua_call(lstate, 0, 1);
        1
    }
}

#[cfg(not(test))]
unsafe fn nlua_init_packages(lstate: *mut lua_State) -> bool {
    unsafe {
        lua_getglobal(lstate, c"package".as_ptr());
        lua_getfield(lstate, -1, c"preload".as_ptr());
        for (i, def) in BUILTIN_MODULES.iter().enumerate() {
            lua_pushinteger(lstate, i as lua_Integer);
            lua_pushcclosure(lstate, Some(nlua_module_preloader), 1);
            lua_setfield(lstate, -2, def.name.as_ptr());

            if nlua_disable_preload && def.name == c"vim.inspect" {
                break;
            }
        }

        lua_pop(lstate, 2);

        lua_getglobal(lstate, c"require".as_ptr());
        lua_pushstring(lstate, c"vim._init_packages".as_ptr());
        if nlua_pcall(lstate, 1, 0) != 0 {
            fprintf(log_get_stderr(), c"%s\n".as_ptr(), lua_tostring(lstate, -1));
            return false;
        }

        true
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_ui_attach(lstate: *mut lua_State) -> c_int {
    unsafe {
        let ns_id = luaL_checkinteger(lstate, 1) as u32;

        if !ns_initialized(ns_id) {
            return luaL_error(lstate, c"invalid ns_id".as_ptr());
        }
        if !lua_istable(lstate, 2) {
            return luaL_error(lstate, c"opts must be a table".as_ptr());
        }
        if !lua_isfunction(lstate, 3) {
            return luaL_error(lstate, c"callback must be a Lua function".as_ptr());
        }

        let mut ext_widgets = [false; kUIGlobalCount];
        let mut tbl_has_true_val = false;
        let ext_names_ptr = (&raw const ui_ext_names).cast::<*const c_char>();

        lua_pushvalue(lstate, 2);
        lua_pushnil(lstate);
        while lua_next(lstate, -2) != 0 {
            let mut len: usize = 0;
            let s = lua_tolstring(lstate, -2, &raw mut len);
            let val = lua_toboolean(lstate, -1) != 0;

            let mut matched = false;
            if strequal(s, c"set_cmdheight".as_ptr()) {
                ui_refresh_cmdheight = val;
                matched = true;
            } else {
                for (i, slot) in ext_widgets.iter_mut().enumerate() {
                    if strequal(s, *ext_names_ptr.add(i)) {
                        if val {
                            tbl_has_true_val = true;
                        }
                        *slot = val;
                        matched = true;
                        break;
                    }
                }
            }

            if !matched {
                return luaL_error(lstate, c"Unexpected key: %s".as_ptr(), s);
            }
            lua_pop(lstate, 1);
        }

        if !tbl_has_true_val {
            return luaL_error(
                lstate,
                c"opts table must contain at least one 'true' ext_widget".as_ptr(),
            );
        }

        let ui_event_cb = nlua_ref_global(lstate, 3);
        ui_add_cb(ns_id, ui_event_cb, ext_widgets.as_mut_ptr());
        ui_refresh_cmdheight = true;
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_ui_detach(lstate: *mut lua_State) -> c_int {
    unsafe {
        let ns_id = luaL_checkinteger(lstate, 1) as u32;
        if !ns_initialized(ns_id) {
            return luaL_error(lstate, c"invalid ns_id".as_ptr());
        }
        ui_remove_cb(ns_id, false);
        0
    }
}

#[cfg(not(test))]
unsafe fn nlua_state_init(lstate: *mut lua_State) -> bool {
    unsafe {
        lua_pushcfunction(lstate, Some(nlua_print));
        lua_setglobal(lstate, c"print".as_ptr());

        lua_getglobal(lstate, c"debug".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_debug));
        lua_setfield(lstate, -2, c"debug".as_ptr());
        lua_pop(lstate, 1);

        #[cfg(windows)]
        {
            lua_getglobal(lstate, c"os".as_ptr());
            lua_pushcfunction(lstate, Some(nlua_getenv));
            lua_setfield(lstate, -2, c"getenv".as_ptr());
            lua_pop(lstate, 1);
        }

        lua_getglobal(lstate, c"os".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_os_exit));
        lua_setfield(lstate, -2, c"exit".as_ptr());
        lua_pop(lstate, 1);

        lua_newtable(lstate);

        nlua_add_api_functions(lstate);
        nlua_init_types(lstate);

        lua_pushcfunction(lstate, Some(nlua_schedule));
        lua_setfield(lstate, -2, c"schedule".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_in_fast_event));
        lua_setfield(lstate, -2, c"in_fast_event".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_call));
        lua_setfield(lstate, -2, c"call".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_rpcrequest));
        lua_setfield(lstate, -2, c"rpcrequest".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_rpcnotify));
        lua_setfield(lstate, -2, c"rpcnotify".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_ui_attach));
        lua_setfield(lstate, -2, c"ui_attach".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_ui_detach));
        lua_setfield(lstate, -2, c"ui_detach".as_ptr());

        nlua_common_vim_init(lstate, false);

        lua_getfield(lstate, -1, c"_core".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_loop_poll));
        lua_setfield(lstate, -2, c"loop_poll".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_ui_flush));
        lua_setfield(lstate, -2, c"ui_flush".as_ptr());
        lua_pushcfunction(lstate, Some(nlua_check_interrupt));
        lua_setfield(lstate, -2, c"check_interrupt".as_ptr());

        lua_pushcfunction(lstate, Some(nlua_ex_docmd));
        lua_setfield(lstate, -2, c"ex_docmd".as_ptr());
        lua_pop(lstate, 1);

        if !time_fd.is_null() {
            lua_getglobal(lstate, c"require".as_ptr());
            REQUIRE_REF = nlua_ref_global(lstate, -1);
            lua_pop(lstate, 1);
            lua_pushcfunction(lstate, Some(nlua_require));
            lua_setglobal(lstate, c"require".as_ptr());
        }

        nlua_treesitter_init(lstate);
        nlua_state_add_stdlib(lstate, false);

        lua_setglobal(lstate, c"vim".as_ptr());

        nlua_init_packages(lstate)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_init(argv: *mut *mut c_char, argc: c_int, lua_arg0: c_int) {
    #[cfg(not(test))]
    unsafe {
        let lstate = luaL_newstate();
        if lstate.is_null() {
            fprintf(
                log_get_stderr(),
                gettext(c"E970: Failed to initialize Lua interpreter\n".as_ptr()),
            );
            os_exit(1);
        }
        luaL_openlibs(lstate);
        if !nlua_state_init(lstate) {
            fprintf(
                log_get_stderr(),
                gettext(c"E970: Failed to initialize builtin Lua modules\n".as_ptr()),
            );
            if main_exitfree {
                nlua_common_free_all_mem(lstate);
            }
            os_exit(1);
        }

        luv_set_thread_cb(Some(nlua_thread_acquire_vm), Some(nlua_common_free_all_mem));
        GLOBAL_LSTATE = lstate;
        active_lstate = lstate;
        MAIN_THREAD = uv_thread_self();
        nlua_init_argv(lstate, argv, argc, lua_arg0);
    }
    #[cfg(test)]
    {
        let _ = (argv, argc, lua_arg0);
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_thread_acquire_vm() -> *mut lua_State {
    unsafe { nlua_init_state(true) }
}

#[cfg(not(test))]
unsafe fn nlua_init_state(thread: bool) -> *mut lua_State {
    unsafe {
        let self_ = uv_thread_self();
        if uv_thread_equal(&raw const MAIN_THREAD, &raw const self_) != 0 {
            runtime_search_path_validate();
        }

        let lstate = luaL_newstate();
        luaL_openlibs(lstate);

        lua_pushcfunction(lstate, Some(nlua_print));
        lua_setglobal(lstate, c"print".as_ptr());

        lua_pushinteger(lstate, 0);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.refcount".as_ptr());

        lua_newtable(lstate);

        nlua_common_vim_init(lstate, thread);
        nlua_state_add_stdlib(lstate, true);

        lua_createtable(lstate, 0, 0);
        lua_pushcfunction(lstate, Some(nlua_thr_api_nvim__get_runtime));
        lua_setfield(lstate, -2, c"nvim__get_runtime".as_ptr());
        lua_setfield(lstate, -2, c"api".as_ptr());

        lua_setglobal(lstate, c"vim".as_ptr());

        nlua_init_packages(lstate);

        lua_getglobal(lstate, c"package".as_ptr());
        lua_getfield(lstate, -1, c"loaded".as_ptr());
        lua_getglobal(lstate, c"vim".as_ptr());
        lua_setfield(lstate, -2, c"vim".as_ptr());
        lua_pop(lstate, 2);

        lstate
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_free_all_mem() {
    #[cfg(not(test))]
    unsafe {
        if GLOBAL_LSTATE.is_null() {
            return;
        }
        let lstate = GLOBAL_LSTATE;
        GLOBAL_LSTATE = ptr::null_mut();
        nlua_unref_global(lstate, REQUIRE_REF);
        nlua_common_free_all_mem(lstate);
        nlua_treesitter_free();
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_common_free_all_mem(lstate: *mut lua_State) {
    unsafe {
        let ref_state = nlua_get_ref_state(lstate);
        nlua_unref(lstate, ref_state, (*ref_state).nil_ref);
        nlua_unref(lstate, ref_state, (*ref_state).empty_dict_ref);
        lua_close(lstate);
    }
}

#[cfg(not(test))]
unsafe extern "C" fn nlua_print_event(argv: *mut *mut c_void) {
    unsafe {
        msg_ext_no_fast();
        let raw = msg_use_printf() != 0 && redirecting() == 0 && on_print.cb_type == kCallbackNone;

        let items = xmalloc(8 * size_of::<HlMessageChunk>()).cast::<HlMessageChunk>();
        *items = HlMessageChunk {
            text: NvimString {
                data: (*argv).cast(),
                size: (*argv.add(1) as usize).saturating_sub(1),
            },
            hl_id: 0,
        };
        let msg = HlMessage {
            size: 1,
            capacity: 8,
            items,
        };
        let mut needs_clear = false;
        msg_multihl(
            OBJECT_NIL,
            msg,
            c"lua_print".as_ptr(),
            true,
            false,
            ptr::null_mut(),
            &raw mut needs_clear,
            raw,
        );
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_print(lstate: *mut lua_State) -> c_int {
    unsafe {
        let nargs = lua_gettop(lstate);
        lua_getglobal(lstate, c"tostring".as_ptr());
        let mut errmsg_len: usize = 0;
        let mut msg_ga: garray_T = zeroed();
        ga_init(&raw mut msg_ga, 1, 80);
        let mut curargidx: c_int = 1;
        while curargidx <= nargs {
            lua_pushvalue(lstate, -1);
            lua_pushvalue(lstate, curargidx);
            if lua_pcall(lstate, 1, 1, 0) != 0 || lua_type(lstate, -1) != LUA_TSTRING {
                let errmsg = nlua_get_error(lstate, &raw mut errmsg_len);
                ga_clear(&raw mut msg_ga);
                let buff = xmalloc(IOSIZE).cast::<c_char>();
                let fmt = gettext(c"E5114: Converting print argument #%i: %.*s".as_ptr());
                let len = vim_snprintf!(buff, IOSIZE, fmt, curargidx, errmsg_len as c_int, errmsg)
                    as usize;
                lua_pushlstring(lstate, buff, len);
                xfree(buff.cast());
                return lua_error(lstate);
            }

            let mut len: usize = 0;
            let s = lua_tolstring(lstate, -1, &raw mut len);
            ga_concat_len(&raw mut msg_ga, s, len);
            if curargidx < nargs {
                ga_append(&raw mut msg_ga, b' ');
            }
            lua_pop(lstate, 1);
            curargidx += 1;
        }
        ga_append(&raw mut msg_ga, 0);

        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        let is_thread = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 1);

        if is_thread {
            let mut ev = Event {
                handler: Some(nlua_print_event),
                argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
            };
            ev.argv[0] = msg_ga.ga_data;
            ev.argv[1] = msg_ga.ga_len as isize as *mut c_void;
            loop_schedule_deferred((&raw mut main_loop).cast(), ev);
        } else if IN_FAST_CALLBACK != 0 {
            let queues = loop_queues((&raw mut main_loop).cast());
            put_event(
                (*queues).events,
                nlua_print_event,
                msg_ga.ga_data,
                msg_ga.ga_len as isize as *mut c_void,
            );
        } else {
            let mut argv: [*mut c_void; 2] =
                [msg_ga.ga_data, msg_ga.ga_len as isize as *mut c_void];
            nlua_print_event(argv.as_mut_ptr());
        }
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_require(lstate: *mut lua_State) -> c_int {
    unsafe {
        let name = luaL_checklstring(lstate, 1, ptr::null_mut());
        lua_settop(lstate, 1);

        lua_getfield(lstate, LUA_REGISTRYINDEX, c"_LOADED".as_ptr());
        lua_getfield(lstate, 2, name);
        if lua_toboolean(lstate, -1) != 0 {
            return 1;
        }
        lua_pop(lstate, 2);

        nlua_pushref(lstate, REQUIRE_REF);
        lua_insert(lstate, 1);

        if time_fd.is_null() {
            lua_getglobal(lstate, c"require".as_ptr());
            if lua_iscfunction(lstate, -1) != 0
                && lua_tocfunction(lstate, -1).is_some_and(|f| {
                    std::ptr::fn_addr_eq(
                        f,
                        nlua_require as unsafe extern "C-unwind" fn(*mut lua_State) -> c_int,
                    )
                })
            {
                lua_pushvalue(lstate, 1);
                lua_setglobal(lstate, c"require".as_ptr());
            }
            lua_pop(lstate, 1);
            lua_call(lstate, 1, 1);
            return 1;
        }

        let mut rel_time: proftime_T = 0;
        let mut start_time: proftime_T = 0;
        time_push(&raw mut rel_time, &raw mut start_time);
        let status = lua_pcall(lstate, 1, 1, 0);
        if status == 0 {
            vim_snprintf!(
                (&raw mut IObuff).cast(),
                IOSIZE,
                c"require('%s')".as_ptr(),
                name,
            );
            time_msg((&raw const IObuff).cast(), &raw const start_time);
        }
        time_pop(rel_time);

        if status == 0 { 1 } else { lua_error(lstate) }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_debug(lstate: *mut lua_State) -> c_int {
    unsafe {
        let input_args: [typval_T; 2] = [
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_string: c"lua_debug> ".as_ptr().cast_mut(),
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        loop {
            lua_settop(lstate, 0);
            let mut input: typval_T = zeroed();
            get_user_input(input_args.as_ptr(), &raw mut input, false, false);

            if ui_has(kUICmdline) {
                vim_snprintf!(
                    (&raw mut IObuff).cast(),
                    IOSIZE,
                    c"lua_debug> %s".as_ptr(),
                    input.vval.v_string,
                );
                ui_ext_cmdline_block_append(0, (&raw const IObuff).cast());
            } else {
                msg_putchar(b'\n' as c_int);
            }

            if input.v_type != VarType::VAR_STRING
                || input.vval.v_string.is_null()
                || *input.vval.v_string == NUL
                || CStr::from_ptr(input.vval.v_string) == c"cont"
            {
                tv_clear(&raw mut input);
                if ui_has(kUICmdline) {
                    ui_ext_cmdline_block_leave();
                }
                return 0;
            }

            let len = CStr::from_ptr(input.vval.v_string).to_bytes().len();
            if luaL_loadbuffer(
                lstate,
                input.vval.v_string,
                len,
                c"=(debug command)".as_ptr(),
            ) != 0
            {
                nlua_error(
                    lstate,
                    gettext(c"E5115: Loading Lua debug string: %.*s".as_ptr()),
                );
            } else if nlua_pcall(lstate, 0, 0) != 0 {
                nlua_error(
                    lstate,
                    gettext(c"E5116: Calling Lua debug string: %.*s".as_ptr()),
                );
            }
            tv_clear(&raw mut input);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C-unwind" fn nlua_in_fast_event(lstate: *mut lua_State) -> c_int {
    #[cfg(not(test))]
    unsafe {
        lua_pushboolean(lstate, c_int::from(IN_FAST_CALLBACK > 0));
        1
    }
    #[cfg(test)]
    {
        let _ = lstate;
        0
    }
}

#[cfg(not(test))]
unsafe fn viml_func_is_fast(name: *const c_char) -> bool {
    unsafe {
        let fdef: *const EvalFuncDef = find_internal_func(name);
        !fdef.is_null() && (*fdef).fast
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C-unwind" fn nlua_call(lstate: *mut lua_State) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut name_len: usize = 0;
        let name = luaL_checklstring(lstate, 1, &raw mut name_len);
        if !nlua_is_deferred_safe() && !viml_func_is_fast(name) {
            let length = name_len.min(100) + 22;
            vim_snprintf!(
                (&raw mut IObuff).cast(),
                length,
                c"Vimscript function \"%s\"".as_ptr(),
                name,
            );
            return luaL_error(
                lstate,
                (&raw const e_fast_api_disabled).cast(),
                (&raw const IObuff).cast::<c_char>(),
            );
        }

        let nargs = lua_gettop(lstate) - 1;
        if nargs > MAX_FUNC_ARGS {
            return luaL_error(lstate, c"Function called with too many arguments".as_ptr());
        }

        let func_lua = find_internal_func_lua(name);
        if !func_lua.is_null() {
            lua_getglobal(lstate, c"require".as_ptr());
            lua_pushlstring(lstate, c"vim._core.vimfn".as_ptr(), 15);
            if lua_pcall(lstate, 1, 1, 0) != 0 {
                return lua_error(lstate);
            }
            lua_getfield(lstate, -1, func_lua);
            lua_remove(lstate, -2);
            for j in 0..nargs {
                lua_pushvalue(lstate, j + 2);
            }
            if lua_pcall(lstate, nargs, 1, 0) != 0 {
                return lua_error(lstate);
            }
            return 1;
        }

        let mut vim_args: [typval_T; (MAX_FUNC_ARGS + 1) as usize] = zeroed();
        let mut i: usize = 0;
        let mut conv_failed = false;
        while (i as c_int) < nargs {
            lua_pushvalue(lstate, (i as c_int) + 2);
            if !nlua_pop_typval(lstate, &raw mut vim_args[i]) {
                api_set_error!(
                    &raw mut err,
                    ErrorType::kErrorTypeException,
                    c"error converting argument %d".as_ptr(),
                    (i + 1) as c_int,
                );
                conv_failed = true;
                break;
            }
            i += 1;
        }

        if !conv_failed {
            force_abort = false;
            suppress_errthrow = false;
            did_throw = false;
            did_emsg = 0;

            let mut rettv: typval_T = zeroed();
            let mut funcexe: funcexe_T = zeroed();
            funcexe.fe_firstline = (*curwin).w_cursor.lnum;
            funcexe.fe_lastline = (*curwin).w_cursor.lnum;
            funcexe.fe_evaluate = true;
            let save_current_sctx = api_set_sctx(LUA_INTERNAL_CALL);

            let mut tstate: TryState = zeroed();
            try_enter(&raw mut tstate);
            let _ = call_func(
                name,
                name_len as c_int,
                &raw mut rettv,
                nargs,
                vim_args.as_mut_ptr(),
                &raw mut funcexe,
            );
            try_leave(&raw const tstate, &raw mut err);
            current_sctx = save_current_sctx;

            if !error_set(&raw const err) {
                nlua_push_typval(lstate, &raw mut rettv, 0);
            }
            tv_clear(&raw mut rettv);
        }

        while i > 0 {
            i -= 1;
            tv_clear(&raw mut vim_args[i]);
        }
        if error_set(&raw const err) {
            lua_pushstring(lstate, err.msg);
            api_clear_error(&raw mut err);
            return lua_error(lstate);
        }
        1
    }
    #[cfg(test)]
    {
        let _ = lstate;
        0
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_rpcrequest(lstate: *mut lua_State) -> c_int {
    unsafe {
        if !nlua_is_deferred_safe() {
            return luaL_error(
                lstate,
                (&raw const e_fast_api_disabled).cast(),
                c"rpcrequest".as_ptr(),
            );
        }
        nlua_rpc(lstate, true)
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_rpcnotify(lstate: *mut lua_State) -> c_int {
    unsafe { nlua_rpc(lstate, false) }
}

#[cfg(not(test))]
unsafe fn nlua_rpc(lstate: *mut lua_State, request: bool) -> c_int {
    unsafe {
        let mut name_len: usize = 0;
        let chan_id = luaL_checkinteger(lstate, 1) as u64;
        let name = luaL_checklstring(lstate, 2, &raw mut name_len);
        let nargs = lua_gettop(lstate) - 2;
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut args = arena_array(&raw mut arena, nargs.max(0) as usize);
        for i in 0..nargs {
            lua_pushvalue(lstate, i + 3);
            let obj = nlua_pop_Object(lstate, false, &raw mut arena, &raw mut err);
            *args.items.add(args.size) = obj;
            args.size += 1;
            if error_set(&raw const err) {
                break;
            }
        }

        if !error_set(&raw const err) {
            if request {
                let mut res_mem: ArenaMem = ptr::null_mut();
                let mut result = rpc_send_call(chan_id, name, args, &raw mut res_mem, &raw mut err);
                if !error_set(&raw const err) {
                    nlua_push_Object(lstate, &raw mut result, 0);
                    arena_mem_free(res_mem);
                }
            } else if !rpc_send_event(chan_id, name, args) {
                let mut mbuf = [0u8; 64];
                let _ = write!(&mut mbuf[..], "Invalid channel: {chan_id}\0");
                api_set_error!(
                    &raw mut err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    mbuf.as_ptr().cast::<c_char>(),
                );
            }
        }

        arena_mem_free(arena_finish(&raw mut arena));

        if error_set(&raw const err) {
            lua_pushstring(lstate, err.msg);
            api_clear_error(&raw mut err);
            return lua_error(lstate);
        }

        if request { 1 } else { 0 }
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_nil_tostring(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushstring(lstate, c"vim.NIL".as_ptr());
        1
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_nil_index(lstate: *mut lua_State) -> c_int {
    unsafe { luaL_error(lstate, c"attempt to index vim.NIL".as_ptr()) }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_empty_dict_tostring(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushstring(lstate, c"vim.empty_dict()".as_ptr());
        1
    }
}

#[cfg(all(windows, not(test)))]
unsafe extern "C-unwind" fn nlua_getenv(lstate: *mut lua_State) -> c_int {
    unsafe {
        let name = luaL_checklstring(lstate, 1, ptr::null_mut());
        lua_pushstring(lstate, os_getenv_noalloc(name));
        1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_ref(
    lstate: *mut lua_State,
    ref_state: *mut nlua_ref_state_t,
    index: c_int,
) -> LuaRef {
    #[cfg(not(test))]
    unsafe {
        lua_pushvalue(lstate, index);
        let ref_ = luaL_ref(lstate, LUA_REGISTRYINDEX);
        if ref_ > 0 {
            (*ref_state).ref_count += 1;
        }
        ref_
    }
    #[cfg(test)]
    {
        let _ = (lstate, ref_state, index);
        LUA_NOREF
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_ref_global(lstate: *mut lua_State, index: c_int) -> LuaRef {
    #[cfg(not(test))]
    unsafe {
        nlua_ref(lstate, nlua_global_refs, index)
    }
    #[cfg(test)]
    {
        let _ = (lstate, index);
        LUA_NOREF
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_unref(
    lstate: *mut lua_State,
    ref_state: *mut nlua_ref_state_t,
    ref_: LuaRef,
) {
    #[cfg(not(test))]
    unsafe {
        if ref_ > 0 {
            (*ref_state).ref_count -= 1;
            luaL_unref(lstate, LUA_REGISTRYINDEX, ref_);
        }
    }
    #[cfg(test)]
    {
        let _ = (lstate, ref_state, ref_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_unref_global(lstate: *mut lua_State, ref_: LuaRef) {
    #[cfg(not(test))]
    unsafe {
        nlua_unref(lstate, nlua_global_refs, ref_);
    }
    #[cfg(test)]
    {
        let _ = (lstate, ref_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_free_luaref(ref_: LuaRef) {
    unsafe {
        nlua_unref_global(GLOBAL_LSTATE, ref_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_pushref(lstate: *mut lua_State, ref_: LuaRef) {
    #[cfg(not(test))]
    unsafe {
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, ref_);
    }
    #[cfg(test)]
    {
        let _ = (lstate, ref_);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_new_luaref(original_ref: LuaRef) -> LuaRef {
    if original_ref == LUA_NOREF {
        return LUA_NOREF;
    }
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, original_ref);
        let new_ref = nlua_ref_global(lstate, -1);
        lua_pop(lstate, 1);
        new_ref
    }
    #[cfg(test)]
    {
        LUA_NOREF
    }
}

unsafe fn nlua_exec_typval_fmt(
    fmt: *const c_char,
    body: *const c_char,
    bodylen: usize,
    name: *const c_char,
    args: *mut typval_T,
    argcount: c_int,
    special: bool,
    ret_tv: *mut typval_T,
) {
    #[cfg(not(test))]
    unsafe {
        static mut LCMD_BUF: [c_char; IOSIZE] = [0; IOSIZE];
        let buf_ptr = (&raw mut LCMD_BUF).cast::<c_char>();
        let mut lcmd = buf_ptr;
        let lcmd_len = vim_snprintf!(buf_ptr, IOSIZE, fmt, bodylen as c_int, body);
        if lcmd_len as usize >= IOSIZE {
            lcmd = xmalloc(lcmd_len as usize + 1).cast::<c_char>();
            vim_snprintf!(lcmd, lcmd_len as usize + 1, fmt, bodylen as c_int, body);
        }

        nlua_exec_typval(
            lcmd,
            lcmd_len as usize,
            name,
            args,
            argcount,
            special,
            ret_tv,
        );

        if !ptr::eq(lcmd, buf_ptr) {
            xfree(lcmd.cast());
        }
    }
    #[cfg(test)]
    {
        let _ = (fmt, body, bodylen, name, args, argcount, special, ret_tv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_luaeval(
    str_: NvimString,
    arg: *mut typval_T,
    ret_tv: *mut typval_T,
) {
    unsafe {
        nlua_exec_typval_fmt(
            c"local _A=select(1,...) return (%.*s)".as_ptr(),
            str_.data,
            str_.size,
            c"luaeval()".as_ptr(),
            arg,
            1,
            true,
            ret_tv,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_vlua(
    str_: *const c_char,
    len: usize,
    args: *mut typval_T,
    argcount: c_int,
    ret_tv: *mut typval_T,
) {
    unsafe {
        nlua_exec_typval_fmt(
            c"return %.*s(...)".as_ptr(),
            str_,
            len,
            c"v:lua".as_ptr(),
            args,
            argcount,
            false,
            ret_tv,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_user_expand_func(xp: *mut expand_T, ret_tv: *mut typval_T) {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;

        nlua_pushref(lstate, (*xp).xp_luaref);
        lua_pushstring(lstate, (*xp).xp_pattern);
        lua_pushstring(lstate, (*xp).xp_line);
        lua_pushinteger(lstate, (*xp).xp_col as lua_Integer);

        if nlua_pcall(lstate, 3, 1) != 0 {
            nlua_error(lstate, gettext(c"E5108: Lua function: %.*s".as_ptr()));
            return;
        }

        nlua_pop_typval(lstate, ret_tv);
    }
    #[cfg(test)]
    {
        let _ = (xp, ret_tv);
    }
}

unsafe fn nlua_exec_typval(
    lcmd: *const c_char,
    lcmd_len: usize,
    name: *const c_char,
    args: *mut typval_T,
    argcount: c_int,
    special: bool,
    ret_tv: *mut typval_T,
) {
    #[cfg(not(test))]
    unsafe {
        if check_secure() {
            if !ret_tv.is_null() {
                (*ret_tv).v_type = VarType::VAR_NUMBER;
                (*ret_tv).vval.v_number = 0;
            }
            return;
        }

        let lstate = GLOBAL_LSTATE;
        if luaL_loadbuffer(lstate, lcmd, lcmd_len, name) != 0 {
            nlua_error(lstate, gettext(c"E5107: Lua: %.*s".as_ptr()));
            return;
        }

        push_all_typvals(lstate, args, argcount, special);

        if nlua_pcall(lstate, argcount, if !ret_tv.is_null() { 1 } else { 0 }) != 0 {
            nlua_error(lstate, gettext(c"E5108: Lua: %.*s".as_ptr()));
            return;
        }

        if !ret_tv.is_null() {
            nlua_pop_typval(lstate, ret_tv);
        }
    }
    #[cfg(test)]
    {
        let _ = (lcmd, lcmd_len, name, args, argcount, special, ret_tv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_exec_ga(ga: *mut garray_T, name: *mut c_char) {
    unsafe {
        let code = ga_concat_strings(ga, c"\n".as_ptr());
        let len = CStr::from_ptr(code).to_bytes().len();
        nlua_exec_typval(code, len, name, ptr::null_mut(), 0, false, ptr::null_mut());
        xfree(code.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_exec_typval_callable(
    lua_cb: LuaRef,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, lua_cb);
        push_all_typvals(lstate, argvars, argcount, false);

        if nlua_pcall(lstate, argcount, 1) != 0 {
            nlua_error(lstate, gettext(c"Lua callback: %.*s".as_ptr()));
            return FCERR_OTHER;
        }

        nlua_pop_typval(lstate, rettv);
        FCERR_NONE
    }
    #[cfg(test)]
    {
        let _ = (lua_cb, argcount, argvars, rettv);
        FCERR_NONE
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_exec(
    str_: NvimString,
    chunkname: *const c_char,
    args: Array,
    mode: LuaRetMode,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        let top = lua_gettop(lstate);
        let name = if !chunkname.is_null() && *chunkname != NUL {
            chunkname
        } else {
            c"<nvim>".as_ptr()
        };
        if luaL_loadbuffer(lstate, str_.data, str_.size, name) != 0 {
            let mut len: usize = 0;
            let errstr = lua_tolstring(lstate, -1, &raw mut len);
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Lua: %.*s".as_ptr(),
                len as c_int,
                errstr,
            );
            return OBJECT_NIL;
        }

        for i in 0..args.size {
            nlua_push_Object(lstate, args.items.add(i), 0);
        }

        if nlua_pcall(lstate, args.size as c_int, 1) != 0 {
            let mut len: usize = 0;
            let errstr = lua_tolstring(lstate, -1, &raw mut len);
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Lua: %.*s".as_ptr(),
                len as c_int,
                errstr,
            );
            return OBJECT_NIL;
        }

        nlua_call_pop_retval(lstate, mode, arena, top, err)
    }
    #[cfg(test)]
    {
        let _ = (str_, chunkname, args, mode, arena, err);
        OBJECT_NIL
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_typval(
    module: *const c_char,
    func: *const c_char,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) {
    #[cfg(not(test))]
    unsafe {
        let mut argcount: c_int = 0;
        let mut tv = argvars;
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            argcount += 1;
            tv = tv.add(1);
        }

        let mut buf = [0 as c_char; 256];
        let len = vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"return require('%s').%s(...)".as_ptr(),
            module,
            func,
        ) as usize;
        nlua_exec_typval(buf.as_ptr(), len, module, argvars, argcount, false, rettv);
    }
    #[cfg(test)]
    {
        let _ = (module, func, argvars, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_excmd(
    module: *const c_char,
    func: *const c_char,
    eap: *mut exarg_T,
    cmod: *const cmdmod_T,
    extra: *mut typval_T,
) -> bool {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        lua_getglobal(lstate, c"require".as_ptr());
        lua_pushstring(lstate, module);
        if lua_pcall(lstate, 1, 1, 0) != 0 {
            nlua_error(lstate, c"E5108: %s".as_ptr());
            return false;
        }
        lua_getfield(lstate, -1, func);
        lua_remove(lstate, -2);

        lua_newtable(lstate);
        nlua_push_eap(lstate, eap, cmod);

        let mut nargs: c_int = 1;
        if !extra.is_null() {
            nlua_push_typval(lstate, extra, 0);
            nargs = 2;
        }

        if nlua_pcall(lstate, nargs, 0) != 0 {
            emsg(lua_tostring(lstate, -1));
            lua_pop(lstate, 1);
            return false;
        }
        true
    }
    #[cfg(test)]
    {
        let _ = (module, func, eap, cmod, extra);
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_ref_is_function(ref_: LuaRef) -> bool {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, ref_);
        let is_func = lua_type(lstate, -1) == LUA_TFUNCTION;
        lua_pop(lstate, 1);
        is_func
    }
    #[cfg(test)]
    {
        let _ = ref_;
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_ref(
    ref_: LuaRef,
    name: *const c_char,
    args: Array,
    mode: LuaRetMode,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nlua_call_ref_ctx(false, ref_, name, args, mode, arena, err) }
}

#[inline]
const fn mode_ret(mode: LuaRetMode) -> c_int {
    if mode == kRetMulti || mode == kRetMultiStack {
        LUA_MULTRET
    } else {
        1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_call_ref_ctx(
    fast: bool,
    ref_: LuaRef,
    name: *const c_char,
    args: Array,
    mode: LuaRetMode,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    #[cfg(not(test))]
    unsafe {
        let lstate = active_lstate;
        let top = lua_gettop(lstate);
        nlua_pushref(lstate, ref_);
        let mut nargs = args.size as c_int;
        if !name.is_null() {
            lua_pushstring(lstate, name);
            nargs += 1;
        }
        for i in 0..args.size {
            nlua_push_Object(lstate, args.items.add(i), 0);
        }

        if fast {
            if nlua_fast_cfpcall(lstate, nargs, mode_ret(mode), -1) < 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"fast context failure".as_ptr(),
                );
                return OBJECT_NIL;
            }
        } else if nlua_pcall(lstate, nargs, mode_ret(mode)) != 0 {
            if !err.is_null() {
                let mut len: usize = 0;
                let errstr = lua_tolstring(lstate, -1, &raw mut len);
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Lua: %.*s".as_ptr(),
                    len as c_int,
                    errstr,
                );
            } else {
                nlua_error(lstate, gettext(c"Lua callback: %.*s".as_ptr()));
            }
            return OBJECT_NIL;
        }

        nlua_call_pop_retval(lstate, mode, arena, top, err)
    }
    #[cfg(test)]
    {
        let _ = (fast, ref_, name, args, mode, arena, err);
        OBJECT_NIL
    }
}

#[cfg(not(test))]
unsafe fn nlua_call_pop_retval(
    lstate: *mut lua_State,
    mode: LuaRetMode,
    arena: *mut Arena,
    pretop: c_int,
    err: *mut Error,
) -> Object {
    unsafe {
        if mode != kRetMulti && mode != kRetMultiStack && lua_isnil(lstate, -1) {
            lua_pop(lstate, 1);
            return OBJECT_NIL;
        }
        let mut dummy = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let perr = if !err.is_null() { err } else { &raw mut dummy };

        match mode {
            kRetNilBool => {
                let bool_value = lua_toboolean(lstate, -1) != 0;
                lua_pop(lstate, 1);
                Object {
                    obj_type: ObjectType::kObjectTypeBoolean,
                    data: ObjectData {
                        boolean: bool_value,
                    },
                }
            }
            kRetLuaref => {
                let ref_ = nlua_ref_global(lstate, -1);
                lua_pop(lstate, 1);
                Object {
                    obj_type: ObjectType::kObjectTypeLuaRef,
                    data: ObjectData { luaref: ref_ },
                }
            }
            kRetObject => nlua_pop_Object(lstate, false, arena, perr),
            kRetMulti => {
                let nres = lua_gettop(lstate) - pretop;
                let mut res = arena_array(arena, nres.max(0) as usize);
                for i in 0..(nres as usize) {
                    *res.items.add((nres as usize) - i - 1) =
                        nlua_pop_Object(lstate, false, arena, perr);
                    if error_set(perr) {
                        return OBJECT_NIL;
                    }
                }
                res.size = nres.max(0) as usize;
                Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData { array: res },
                }
            }
            kRetMultiStack => OBJECT_NIL,
            _ => unreachable!(),
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_is_deferred_safe() -> bool {
    unsafe { IN_FAST_CALLBACK == 0 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_lua(eap: *mut exarg_T) {
    #[cfg(not(test))]
    unsafe {
        if *(*eap).arg == NUL {
            if (*eap).addr_count > 0 {
                cmd_source_buffer(eap.cast(), true);
            } else {
                emsg(gettext((&raw const e_argreq).cast()));
            }
            return;
        }

        let mut len: usize = 0;
        let mut code = script_get(eap, &raw mut len);
        if (*eap).skip != 0 || code.is_null() {
            xfree(code.cast());
            return;
        }

        if (*eap).cmdidx == CMD_equal || *code == b'=' as c_char {
            let off: usize = if (*eap).cmdidx == CMD_equal { 0 } else { 1 };
            len += 18 - off;
            let code_buf = xmallocz(len).cast::<c_char>();
            vim_snprintf!(
                code_buf,
                len + 1,
                c"vim._print(true, %s)".as_ptr(),
                code.add(off),
            );
            xfree(code.cast());
            code = code_buf;
        }

        nlua_exec_typval(
            code,
            len,
            c":lua".as_ptr(),
            ptr::null_mut(),
            0,
            false,
            ptr::null_mut(),
        );
        xfree(code.cast());
    }
    #[cfg(test)]
    {
        let _ = eap;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_luado(eap: *mut exarg_T) {
    #[cfg(not(test))]
    unsafe {
        if u_save((*eap).line1 - 1, (*eap).line2 + 1) == FAIL {
            emsg(gettext(c"cannot save undo information".as_ptr()));
            return;
        }
        let cmd = (*eap).arg;
        let cmd_len = CStr::from_ptr(cmd).to_bytes().len();
        let lstate = GLOBAL_LSTATE;

        const DOSTART: &[u8] = b"return function(line, linenr) ";
        const DOEND: &[u8] = b" end";
        let lcmd_len = cmd_len + DOSTART.len() + DOEND.len();
        static mut LCMD_BUF: [c_char; IOSIZE] = [0; IOSIZE];
        let buf_ptr = (&raw mut LCMD_BUF).cast::<c_char>();
        let lcmd = if lcmd_len < IOSIZE {
            buf_ptr
        } else {
            xmalloc(lcmd_len + 1).cast::<c_char>()
        };
        ptr::copy_nonoverlapping(DOSTART.as_ptr(), lcmd.cast::<u8>(), DOSTART.len());
        ptr::copy_nonoverlapping(
            cmd.cast::<u8>(),
            lcmd.cast::<u8>().add(DOSTART.len()),
            cmd_len,
        );
        ptr::copy_nonoverlapping(
            DOEND.as_ptr(),
            lcmd.cast::<u8>().add(DOSTART.len() + cmd_len),
            DOEND.len(),
        );

        if luaL_loadbuffer(lstate, lcmd, lcmd_len, c":luado".as_ptr()) != 0 {
            nlua_error(lstate, gettext(c"E5109: Lua: %.*s".as_ptr()));
            if !ptr::eq(lcmd, buf_ptr) {
                xfree(lcmd.cast());
            }
            return;
        }
        if !ptr::eq(lcmd, buf_ptr) {
            xfree(lcmd.cast());
        }
        if nlua_pcall(lstate, 0, 1) != 0 {
            nlua_error(lstate, gettext(c"E5110: Lua: %.*s".as_ptr()));
            return;
        }

        let was_curbuf = curbuf;
        for l in (*eap).line1..=(*eap).line2 {
            if l > (*curbuf).b_ml.ml_line_count {
                break;
            }

            lua_pushvalue(lstate, -1);
            let old_line = ml_get_buf(curbuf, l);
            let old_line_len: colnr_T = ml_get_buf_len(curbuf, l);
            lua_pushstring(lstate, old_line);
            lua_pushnumber(lstate, f64::from(l));
            if nlua_pcall(lstate, 2, 1) != 0 {
                nlua_error(lstate, gettext(c"E5111: Lua: %.*s".as_ptr()));
                break;
            }

            if !ptr::eq(curbuf, was_curbuf) || l > (*curbuf).b_ml.ml_line_count {
                break;
            }

            if lua_isstring(lstate, -1) != 0 {
                let mut new_line_len: usize = 0;
                let new_line = lua_tolstring(lstate, -1, &raw mut new_line_len);
                let new_line_transformed = xmemdupz(new_line.cast(), new_line_len).cast::<c_char>();
                for i in 0..new_line_len {
                    if *new_line_transformed.add(i) == NUL {
                        *new_line_transformed.add(i) = b'\n' as c_char;
                    }
                }
                ml_replace(l, new_line_transformed, false);
                inserted_bytes(l, 0, old_line_len, new_line_len as c_int);
            }
            lua_pop(lstate, 1);
        }

        lua_pop(lstate, 1);
        check_cursor(curwin);
        redraw_curbuf_later(UPD_NOT_VALID);
    }
    #[cfg(test)]
    {
        let _ = eap;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_luafile(eap: *mut exarg_T) {
    unsafe {
        let _ = nlua_exec_file((*eap).arg);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_exec_file(path: *const c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        if !strequal(path, c"-".as_ptr()) {
            lua_getglobal(lstate, c"loadfile".as_ptr());
            lua_pushstring(lstate, path);
        } else {
            let mut stdin_dup: FileDescriptor = zeroed();
            if file_open_stdin(&raw mut stdin_dup) != 0 {
                return false;
            }

            let mut sb: Vec<u8> = Vec::with_capacity(64);
            loop {
                if got_int {
                    file_close(&raw mut stdin_dup, false);
                    return false;
                }
                let read_size = file_read(&raw mut stdin_dup, (&raw mut IObuff).cast(), 64);
                if read_size < 0 {
                    file_close(&raw mut stdin_dup, false);
                    return false;
                }
                if read_size > 0 {
                    let slice = std::slice::from_raw_parts(
                        (&raw const IObuff).cast::<u8>(),
                        read_size as usize,
                    );
                    sb.extend_from_slice(slice);
                }
                if read_size < 64 {
                    break;
                }
            }
            sb.push(0);
            file_close(&raw mut stdin_dup, false);

            lua_getglobal(lstate, c"loadstring".as_ptr());
            lua_pushstring(lstate, sb.as_ptr().cast());
        }

        if nlua_pcall(lstate, 1, 2) != 0 {
            nlua_error(lstate, gettext(c"E5111: Lua: %.*s".as_ptr()));
            return false;
        }

        if lua_isnil(lstate, -2) {
            nlua_error(lstate, gettext(c"E5112: Lua chunk: %.*s".as_ptr()));
            assert!(lua_isnil(lstate, -1));
            lua_pop(lstate, 1);
            return false;
        }

        assert!(lua_isnil(lstate, -1));
        lua_pop(lstate, 1);

        if nlua_pcall(lstate, 0, 0) != 0 {
            nlua_error(lstate, gettext(c"E5113: Lua chunk: %.*s".as_ptr()));
            return false;
        }

        true
    }
    #[cfg(test)]
    {
        let _ = path;
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_expand_pat(xp: *mut expand_T) {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        let mut status = FAIL;

        lua_getglobal(lstate, c"vim".as_ptr());
        lua_getfield(lstate, -1, c"_expand_pat".as_ptr());
        luaL_checktype(lstate, -1, LUA_TFUNCTION);

        let pat = (*xp).xp_pattern;
        let line_end = (*xp).xp_line.add((*xp).xp_col as usize);
        assert!(line_end >= pat);
        let patlen = line_end.offset_from(pat);
        lua_pushlstring(lstate, pat, patlen as usize);

        if nlua_pcall(lstate, 1, 2) != 0 {
            nlua_error(lstate, gettext(c"vim._expand_pat: %.*s".as_ptr()));
            return;
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let prefix_len = nlua_pop_Integer(lstate, &raw mut arena, &raw mut err) as isize;
        if !error_set(&raw const err) && prefix_len <= patlen {
            let completions = nlua_pop_Array(lstate, &raw mut arena, &raw mut err);
            if !error_set(&raw const err) {
                ga_clear(&raw mut EXPAND_RESULT_ARRAY);
                ga_init(
                    &raw mut EXPAND_RESULT_ARRAY,
                    size_of::<*mut c_char>() as c_int,
                    80,
                );

                let mut all_strings = true;
                for i in 0..completions.size {
                    let v = *completions.items.add(i);
                    if v.obj_type != ObjectType::kObjectTypeString {
                        all_strings = false;
                        break;
                    }
                    ga_grow(&raw mut EXPAND_RESULT_ARRAY, 1);
                    let slot = EXPAND_RESULT_ARRAY
                        .ga_data
                        .cast::<*mut c_char>()
                        .add(EXPAND_RESULT_ARRAY.ga_len as usize);
                    *slot = string_to_cstr(v.data.string);
                    EXPAND_RESULT_ARRAY.ga_len += 1;
                }

                if all_strings {
                    (*xp).xp_pattern = (*xp).xp_pattern.offset(prefix_len);
                    status = OK;
                }
            }
            arena_mem_free(arena_finish(&raw mut arena));
        }

        if status == FAIL {
            ga_clear(&raw mut EXPAND_RESULT_ARRAY);
        }
    }
    #[cfg(test)]
    {
        let _ = xp;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_expand_get_matches(
    num_results: *mut c_int,
    results: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        *results = EXPAND_RESULT_ARRAY.ga_data.cast();
        *num_results = EXPAND_RESULT_ARRAY.ga_len;
        EXPAND_RESULT_ARRAY = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        c_int::from(*num_results > 0)
    }
}

#[cfg(not(test))]
unsafe extern "C-unwind" fn nlua_is_thread(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_is_table_from_lua(arg: *const typval_T) -> bool {
    unsafe {
        if (*arg).v_type == VarType::VAR_DICT {
            (*(*arg).vval.v_dict).lua_table_ref != LUA_NOREF
        } else if (*arg).v_type == VarType::VAR_LIST {
            (*(*arg).vval.v_list).lua_table_ref != LUA_NOREF
        } else {
            false
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_register_table_as_callable(arg: *const typval_T) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        let table_ref = if (*arg).v_type == VarType::VAR_DICT {
            (*(*arg).vval.v_dict).lua_table_ref
        } else if (*arg).v_type == VarType::VAR_LIST {
            (*(*arg).vval.v_list).lua_table_ref
        } else {
            LUA_NOREF
        };

        if table_ref == LUA_NOREF {
            return ptr::null_mut();
        }

        let lstate = GLOBAL_LSTATE;
        let top = lua_gettop(lstate);

        nlua_pushref(lstate, table_ref);
        if lua_getmetatable(lstate, -1) == 0 {
            lua_pop(lstate, 1);
            debug_assert_eq!(top, lua_gettop(lstate));
            return ptr::null_mut();
        }

        lua_getfield(lstate, -1, c"__call".as_ptr());
        if !lua_isfunction(lstate, -1) {
            lua_pop(lstate, 3);
            debug_assert_eq!(top, lua_gettop(lstate));
            return ptr::null_mut();
        }
        lua_pop(lstate, 2);

        let func = nlua_ref_global(lstate, -1);
        let name = register_luafunc(func);
        lua_pop(lstate, 1);
        debug_assert_eq!(top, lua_gettop(lstate));

        name
    }
    #[cfg(test)]
    {
        let _ = arg;
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_exec_on_key(c: c_int, typed_buf: *mut c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        static mut RECURSIVE: bool = false;
        if RECURSIVE {
            return false;
        }
        RECURSIVE = true;

        let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 4];
        let buf_len = special_to_buf(c, mod_mask, false, buf.as_mut_ptr());
        vim_unescape_ks(typed_buf);

        let lstate = GLOBAL_LSTATE;
        let top = lua_gettop(lstate);

        lua_getglobal(lstate, c"vim".as_ptr());
        lua_getfield(lstate, -1, c"_on_key".as_ptr());
        luaL_checktype(lstate, -1, LUA_TFUNCTION);

        lua_pushlstring(lstate, buf.as_ptr(), buf_len as usize);
        lua_pushstring(lstate, typed_buf);

        let save_got_int = got_int;
        got_int = false;
        let mut discard = false;
        if lua_pcall(lstate, 2, 1, 0) != 0 {
            nlua_error(lstate, gettext(c"vim.on_key() callbacks: %.*s".as_ptr()));
        } else {
            if lua_isboolean(lstate, -1) {
                discard = lua_toboolean(lstate, -1) != 0;
            }
            lua_pop(lstate, 1);
        }
        got_int |= save_got_int;

        lua_pop(lstate, 1);
        debug_assert_eq!(top, lua_gettop(lstate));

        RECURSIVE = false;
        discard
    }
    #[cfg(test)]
    {
        let _ = (c, typed_buf);
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_set_sctx(current: *mut sctx_T) {
    #[cfg(not(test))]
    unsafe {
        if !script_is_lua((*current).sc_sid) {
            return;
        }

        (*current).sc_lnum = 0;

        if p_verbose <= 0 {
            return;
        }
        let lstate = active_lstate;
        let info = xmalloc(size_of::<lua_Debug>()).cast::<lua_Debug>();

        let mut level: c_int = 1;
        let mut found = false;
        loop {
            if lua_getstack(lstate, level, info) != 1 {
                break;
            }
            if lua_getinfo(lstate, c"nSl".as_ptr(), info) == 0 {
                break;
            }
            if *(*info).what == b'C' as c_char || *(*info).source != b'@' as c_char {
                level += 1;
                continue;
            }
            found = true;
            break;
        }
        if found {
            let source_path = fix_fname((*info).source.add(1));
            let mut sid = find_script_by_name(source_path);
            if sid > 0 {
                xfree(source_path.cast());
            } else {
                let si: *mut scriptitem_T = new_script_item(source_path, &raw mut sid);
                (*si).sn_lua = true;
            }
            (*current).sc_sid = sid;
            (*current).sc_seq = -1;
            (*current).sc_lnum = (*info).currentline;
        }

        xfree(info.cast());
    }
    #[cfg(test)]
    {
        let _ = current;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_do_ucmd(cmd: *mut ucmd_T, eap: *mut exarg_T, preview: bool) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(
            lstate,
            if preview {
                (*cmd).uc_preview_luaref
            } else {
                (*cmd).uc_luaref
            },
        );

        lua_newtable(lstate);
        nlua_push_eap(lstate, eap, &raw const cmdmod);

        lua_pushstring(lstate, (*cmd).uc_name);
        lua_setfield(lstate, -2, c"name".as_ptr());

        if (*eap).addr_count == 0 {
            lua_pushinteger(lstate, (*cmd).uc_def as lua_Integer);
            lua_setfield(lstate, -2, c"count".as_ptr());
        }

        if ((*cmd).uc_argt & EX_NOSPC) != 0 {
            lua_createtable(lstate, 1, 0);
            if ((*cmd).uc_argt & EX_NEEDARG) != 0 || *(*eap).arg != NUL {
                lua_pushstring(lstate, (*eap).arg);
                lua_rawseti(lstate, -2, 1);
            }
            lua_setfield(lstate, -2, c"fargs".as_ptr());
        }

        let narg_ch = if ((*cmd).uc_argt & EX_EXTRA) != 0 {
            if ((*cmd).uc_argt & EX_NOSPC) != 0 {
                if ((*cmd).uc_argt & EX_ARGSPACE) != 0 {
                    b'_'
                } else if ((*cmd).uc_argt & EX_NEEDARG) != 0 {
                    b'1'
                } else {
                    b'?'
                }
            } else if ((*cmd).uc_argt & EX_NEEDARG) != 0 {
                b'+'
            } else {
                b'*'
            }
        } else {
            b'0'
        };
        let nargs: [c_char; 2] = [narg_ch as c_char, NUL];
        lua_pushstring(lstate, nargs.as_ptr());
        lua_setfield(lstate, -2, c"nargs".as_ptr());

        if preview {
            lua_pushinteger(lstate, cmdpreview_get_ns() as lua_Integer);
            let cmdpreview_bufnr: handle_T = cmdpreview_get_bufnr();
            if cmdpreview_bufnr != 0 {
                lua_pushinteger(lstate, cmdpreview_bufnr as lua_Integer);
            } else {
                lua_pushnil(lstate);
            }
        }

        if nlua_pcall(
            lstate,
            if preview { 3 } else { 1 },
            if preview { 1 } else { 0 },
        ) != 0
        {
            nlua_error(lstate, gettext(c"Lua :command callback: %.*s".as_ptr()));
            return 0;
        }

        let mut retv: c_int = 0;
        if preview && lua_isnumber(lstate, -1) != 0 {
            let v = lua_tointeger(lstate, -1) as c_int;
            if (0..=2).contains(&v) {
                retv = v;
                lua_pop(lstate, 1);
            }
        }

        retv
    }
    #[cfg(test)]
    {
        let _ = (cmd, eap, preview);
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_funcref_str(
    ref_: LuaRef,
    arena: *mut Arena,
    show_ref: bool,
) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        let lstate = GLOBAL_LSTATE;
        if lua_checkstack(lstate, 1) != 0 {
            nlua_pushref(lstate, ref_);
            if !lua_isfunction(lstate, -1) {
                lua_pop(lstate, 1);
            } else {
                let mut ar: lua_Debug = zeroed();
                if lua_getinfo(lstate, c">S".as_ptr(), &raw mut ar) != 0
                    && *ar.source == b'@' as c_char
                    && ar.linedefined >= 0
                {
                    let src = home_replace_save(ptr::null_mut(), ar.source.add(1));
                    let src_s = CStr::from_ptr(src).to_string_lossy();
                    let formatted = if show_ref {
                        format!("<Lua {ref_}: {src_s}:{}>", ar.linedefined)
                    } else {
                        format!("<Lua {src_s}:{}>", ar.linedefined)
                    };
                    xfree(src.cast());
                    return crate::memory::arena_memdupz(
                        arena,
                        formatted.as_ptr().cast(),
                        formatted.len(),
                    );
                }
            }
        }

        let formatted = if show_ref {
            format!("<Lua {ref_}>")
        } else {
            "<Lua>".to_string()
        };
        crate::memory::arena_memdupz(arena, formatted.as_ptr().cast(), formatted.len())
    }
    #[cfg(test)]
    {
        let _ = (ref_, arena, show_ref);
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_init_defaults() {
    #[cfg(not(test))]
    unsafe {
        let L = GLOBAL_LSTATE;
        assert!(!L.is_null());

        lua_getglobal(L, c"require".as_ptr());
        lua_pushstring(L, c"vim._core.defaults".as_ptr());
        if nlua_pcall(L, 1, 0) != 0 {
            fprintf(log_get_stderr(), c"%s\n".as_ptr(), lua_tostring(L, -1));
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nlua_func_exists(lua_funcname: *const c_char) -> bool {
    #[cfg(not(test))]
    unsafe {
        let length = CStr::from_ptr(lua_funcname).to_bytes().len() + 8;
        let str_ = xmalloc(length).cast::<c_char>();
        vim_snprintf!(str_, length, c"return %s".as_ptr(), lua_funcname);
        let mut args_items: [Object; 1] = [Object {
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData {
                string: NvimString {
                    data: str_,
                    size: length - 1,
                },
            },
        }];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let code = NvimString {
            data: c"return type(loadstring(...)()) == 'function'"
                .as_ptr()
                .cast_mut(),
            size: 44,
        };
        let result = nlua_exec(
            code,
            ptr::null(),
            args,
            kRetNilBool,
            ptr::null_mut(),
            &raw mut err,
        );
        xfree(str_.cast());

        api_clear_error(&raw mut err);
        result.obj_type == ObjectType::kObjectTypeBoolean && result.data.boolean
    }
    #[cfg(test)]
    {
        let _ = lua_funcname;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_ret_and_user_cmdidx_helpers_work() {
        assert_eq!(mode_ret(kRetObject), 1);
        assert_eq!(mode_ret(kRetMulti), LUA_MULTRET);
        assert_eq!(mode_ret(kRetMultiStack), LUA_MULTRET);
        assert!(is_user_cmdidx(CMD_USER));
        assert!(is_user_cmdidx(CMD_USER_BUF));
        assert!(!is_user_cmdidx(0));
        unsafe {
            assert!(nlua_is_deferred_safe());
            assert_eq!(api_new_luaref(LUA_NOREF), LUA_NOREF);
        }
        assert_eq!(BUILTIN_MODULES[0].name, c"vim._init_packages");
        assert_eq!(BUILTIN_MODULES[1].name, c"vim.inspect");
        for m in &BUILTIN_MODULES {
            assert!(!m.chunk_name.to_bytes().is_empty());
            assert!(!m.data.is_empty());
            assert!(std::str::from_utf8(m.data).is_ok());
        }
    }
}
