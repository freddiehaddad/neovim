#![forbid(unsafe_code)]
//! Lua interpreter initialization, execution, and Vimscript/RPC bridges (`src/nvim/lua/executor.c`).

use crate::api::command::is_user_cmdidx;
use crate::api::extmark::ns_initialized;
use crate::api::private::helpers::{
    HlMessageChunk, OBJECT_NIL, TryState, api_clear_error, api_free_array, api_set_error,
    api_set_sctx, arena_array, error_set, try_enter, try_leave,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::change::inserted_bytes;
use crate::cmdexpand::Cmdidx;
use crate::cursor::{Colnr, check_cursor};
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
use crate::errors::*;
use crate::eval::funcs::{find_internal_func, find_internal_func_lua};
use crate::eval::typval::{TypVal, TypvalVval, VarType, tv_clear};
use crate::eval::userfunc::{FCERR_NONE, FCERR_OTHER, Funcexe, call_func, register_luafunc};
use crate::event::r#loop::{
    loop_poll_events, loop_queues, loop_schedule_deferred, uv_thread_equal_safe,
    uv_thread_self_safe,
};
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{Cmdmod, Exarg, do_cmdline, get_command_name};
use crate::ex_getln::{
    cmdpreview_get_bufnr, cmdpreview_get_ns, get_user_input, script_get,
    ui_ext_cmdline_block_append, ui_ext_cmdline_block_leave,
};
use crate::fileio::fprintf;
use crate::globals::*;
use crate::input::vgetc;
use crate::keycodes::{special_to_buf, vim_unescape_ks};
use crate::log::log_get_stderr;
use crate::lua::converter::{
    K_NLUA_PUSH_SPECIAL, LUA_ERRMEM, LUA_ERRRUN, LUA_MULTRET, LUA_NOREF, LUA_REGISTRYINDEX,
    LUA_TBOOLEAN, LUA_TFUNCTION, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE, LuaRef, NluaRefState,
    lua_Debug, lua_Integer, lua_State, lua_call_safe, lua_checkstack_safe, lua_close_safe,
    lua_concat_safe, lua_createtable_safe, lua_error_safe, lua_getfield, lua_getglobal,
    lua_getinfo, lua_getmetatable_safe, lua_getstack_safe, lua_insert_safe, lua_isboolean,
    lua_iscfunction_eq, lua_isfunction, lua_isnil, lua_isnumber_safe, lua_isstring_safe,
    lua_istable, lua_newtable, lua_newuserdata_empty, lua_newuserdata_typed, lua_next_safe,
    lua_pcall_safe, lua_pop, lua_push_bytes, lua_pushboolean_safe, lua_pushcclosure_safe,
    lua_pushcfunction_safe, lua_pushinteger_safe, lua_pushnil_safe, lua_pushnumber_safe,
    lua_pushstring, lua_pushstring_opt, lua_pushvalue_safe, lua_rawgeti_safe, lua_rawseti_safe,
    lua_remove_safe, lua_replace_safe, lua_setfield, lua_setglobal, lua_setmetatable_safe,
    lua_settop_safe, lua_toboolean_safe, lua_tointeger_safe, lua_tolstring_bytes,
    lua_tolstring_cstr_with_len, lua_tolstring_opt_cstr, lua_tostring, lua_touserdata_typed,
    lua_type_safe, lua_upvalueindex, luaL_argerror, luaL_checkbytes_with_nul,
    luaL_checkinteger_safe, luaL_checkstring, luaL_checktype_safe, luaL_error_cstr,
    luaL_error_cstr_arg, luaL_loadbuffer, luaL_newstate_safe, luaL_openlibs_safe, luaL_ref_safe,
    luaL_unref_safe, luaL_where_safe, luv_init_vim, luv_set_thread_cb_safe,
    nlua_exit_thread_on_oom, nlua_fast_api_disabled_error, nlua_gettop, nlua_init_types,
    nlua_pop_array, nlua_pop_array_opt, nlua_pop_integer, nlua_pop_object, nlua_pop_object_opt,
    nlua_pop_typval, nlua_push_array, nlua_push_object, nlua_push_typval,
};
use crate::lua::stdlib::nlua_state_add_stdlib;
use crate::lua::treesitter::nlua_treesitter_init;
use crate::main_init::{getout, os_exit, preserve_exit};
use crate::memline::{ml_get_buf, ml_get_buf_len};
use crate::memory::{Arena, ArenaMem, arena_finish, arena_mem_free};
use crate::message::{
    emsg, msg_ext_no_fast, msg_multihl, msg_putchar, msg_use_printf, redirecting, semsg_multiline,
};
use crate::msgpack_rpc::channel::{rpc_send_call, rpc_send_event};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::option_vars::*;
#[cfg(windows)]
use crate::os::env::os_getenv_cstr;
use crate::os::env::{ExpandCtx, home_replace_save};
use crate::os::fileio::{FileDescriptor, file_close, file_open_stdin, file_read};
use crate::path::fix_fname;
use crate::profile::{Proftime, time_msg, time_pop, time_push};
use crate::runtime::{
    Sctx, cmd_source_buffer, find_script_by_name, new_script_item, runtime_get_named_thread,
    runtime_search_path_validate, script_is_lua,
};
use crate::strings::{format_to_buf, gettext};
use crate::ui::K_UIGLOBAL_COUNT;
use crate::ui::{ui_add_cb, ui_flush, ui_has, ui_remove_cb};
use crate::undo::u_save;
use crate::usercmd::{uc_mods, uc_split_args_iter};
use std::ffi::{CStr, CString};
use std::io::Write;
use std::ptr;

use crate::api::vim::HlMessage;
pub type LuaRetMode = i32;

pub const K_RET_OBJECT: LuaRetMode = 0;
pub const K_RET_NIL_BOOL: LuaRetMode = 1;
pub const K_RET_LUAREF: LuaRetMode = 2;
pub const K_RET_MULTI: LuaRetMode = 3;
pub const K_RET_MULTI_STACK: LuaRetMode = 4;

const MAX_FUNC_ARGS: i32 = 20;

const LUA_REFNIL: LuaRef = -1;

const LUVF_CALLBACK_NOEXIT: i32 = 0x01;

const K_CALLBACK: isize = 0;
const K_THREAD: isize = 1;
const K_THREAD_CALLBACK: isize = 2;

#[derive(Clone, Copy)]
struct BuiltinModule {
    name: &'static CStr,
    chunk_name: &'static CStr,
    data: &'static [u8],
}

// NB: vim._init_packages and vim.inspect must be first and second respectively,
// otherwise --luamod-dev won't work properly.
const BUILTIN_MODULES: [BuiltinModule; 35] = [
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
        name: c"vim.uri",
        chunk_name: c"@vim/uri",
        data: include_bytes!("../../../runtime/lua/vim/uri.lua"),
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

#[repr(C)]
pub struct ExDocmdCookie {
    getline: LuaRef,
    got_line: std::cell::Cell<bool>,
}

static MAIN_THREAD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) struct LuaExecutorState {
    pub(crate) active_lstate: std::cell::Cell<*mut lua_State>,
    in_fast_callback: std::cell::Cell<i32>,
    global_lstate: std::cell::Cell<*mut lua_State>,
    require_ref: std::cell::Cell<LuaRef>,
    global_refs: std::cell::Cell<NluaRefState>,
}

impl LuaExecutorState {
    const fn new() -> Self {
        Self {
            active_lstate: std::cell::Cell::new(ptr::null_mut()),
            in_fast_callback: std::cell::Cell::new(0),
            global_lstate: std::cell::Cell::new(ptr::null_mut()),
            require_ref: std::cell::Cell::new(LUA_REFNIL),
            global_refs: std::cell::Cell::new(NluaRefState {
                nil_ref: LUA_NOREF,
                empty_dict_ref: LUA_NOREF,
                ref_count: 0,
            }),
        }
    }
}

thread_local! {
    static NLUA_STATE: LuaExecutorState = const { LuaExecutorState::new() };
    static NLUA_EXPAND_RESULTS: std::cell::RefCell<Vec<std::ffi::CString>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[inline]
pub(crate) fn active_lstate() -> *mut lua_State {
    NLUA_STATE.with(|s| s.active_lstate.get())
}

#[inline]
pub(crate) fn set_active_lstate(lstate: *mut lua_State) {
    NLUA_STATE.with(|s| s.active_lstate.set(lstate));
}

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::api::private::dispatch::nlua_add_api_functions;
use crate::buffer::VAR_FIXED;
use crate::cmdexpand::{K_UICMDLINE, MB_MAXBYTES};
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_docmd::{
    CMOD_BROWSE, CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS,
    CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE,
    CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, EX_ARGSPACE, EX_EXTRA, EX_NEEDARG, EX_NOSPC,
};
use crate::globals::IOSIZE;
use crate::version::MAIN_EXITFREE;
use crate::window::{WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT};

#[inline]
fn put_event(mq: Option<MultiQueue>, ev: Event) {
    if let Some(mq) = mq {
        multiqueue_put_event(mq, ev);
    }
}

fn push_all_typvals(lstate: *mut lua_State, args: &mut [TypVal], special: bool) {
    let flags = if special { K_NLUA_PUSH_SPECIAL } else { 0 };
    for arg in args {
        if arg.v_type == VarType::VarUnknown {
            lua_pushnil_safe(lstate);
        } else {
            nlua_push_typval(lstate, arg, flags);
        }
    }
}

fn nlua_push_cmdmod(lstate: *mut lua_State, cmod: &Cmdmod) {
    lua_newtable(lstate);

    lua_pushinteger_safe(lstate, (cmod.cmod_tab - 1) as lua_Integer);
    lua_setfield(lstate, -2, c"tab");

    lua_pushinteger_safe(lstate, (cmod.cmod_verbose - 1) as lua_Integer);
    lua_setfield(lstate, -2, c"verbose");

    let split_str = if (cmod.cmod_split & WSP_ABOVE) != 0 {
        c"aboveleft"
    } else if (cmod.cmod_split & WSP_BELOW) != 0 {
        c"belowright"
    } else if (cmod.cmod_split & WSP_TOP) != 0 {
        c"top"
    } else if (cmod.cmod_split & WSP_BOT) != 0 {
        c"botright"
    } else {
        c""
    };
    let split_str = if (cmod.cmod_split & WSP_TOP) != 0 {
        c"topleft"
    } else {
        split_str
    };
    lua_pushstring(lstate, split_str);
    lua_setfield(lstate, -2, c"split");

    lua_pushboolean_safe(lstate, (cmod.cmod_split & WSP_VERT) != 0);
    lua_setfield(lstate, -2, c"vertical");
    lua_pushboolean_safe(lstate, (cmod.cmod_split & WSP_HOR) != 0);
    lua_setfield(lstate, -2, c"horizontal");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_SILENT) != 0);
    lua_setfield(lstate, -2, c"silent");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_ERRSILENT) != 0);
    lua_setfield(lstate, -2, c"emsg_silent");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_UNSILENT) != 0);
    lua_setfield(lstate, -2, c"unsilent");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_SANDBOX) != 0);
    lua_setfield(lstate, -2, c"sandbox");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_NOAUTOCMD) != 0);
    lua_setfield(lstate, -2, c"noautocmd");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_BROWSE) != 0);
    lua_setfield(lstate, -2, c"browse");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_CONFIRM) != 0);
    lua_setfield(lstate, -2, c"confirm");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_HIDE) != 0);
    lua_setfield(lstate, -2, c"hide");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_KEEPALT) != 0);
    lua_setfield(lstate, -2, c"keepalt");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_KEEPJUMPS) != 0);
    lua_setfield(lstate, -2, c"keepjumps");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_KEEPMARKS) != 0);
    lua_setfield(lstate, -2, c"keepmarks");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_KEEPPATTERNS) != 0);
    lua_setfield(lstate, -2, c"keeppatterns");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_LOCKMARKS) != 0);
    lua_setfield(lstate, -2, c"lockmarks");
    lua_pushboolean_safe(lstate, (cmod.cmod_flags & CMOD_NOSWAPFILE) != 0);
    lua_setfield(lstate, -2, c"noswapfile");

    lua_newtable(lstate);
    lua_pushstring(lstate, cmod.cmod_filter_pat.as_deref().unwrap_or(c""));
    lua_setfield(lstate, -2, c"pattern");
    lua_pushboolean_safe(lstate, cmod.cmod_filter_force);
    lua_setfield(lstate, -2, c"force");
    lua_setfield(lstate, -2, c"filter");
}

fn nlua_push_eap(lstate: *mut lua_State, eap: &Exarg, cmod: &Cmdmod) {
    if !is_user_cmdidx(eap.cmdidx) && eap.cmdidx < Cmdidx::CmdSize {
        lua_pushstring_opt(
            lstate,
            get_command_name(&mut crate::os::env::ExpandCtx::new(), eap.cmdidx as i32),
        );
        lua_setfield(lstate, -2, c"name");
    }

    let mut mods_buf = Vec::new();
    uc_mods(&mut mods_buf, cmod, false);
    mods_buf.push(0);
    lua_pushstring(
        lstate,
        CStr::from_bytes_until_nul(&mods_buf).unwrap_or_default(),
    );
    lua_setfield(lstate, -2, c"mods");

    nlua_push_cmdmod(lstate, cmod);
    lua_setfield(lstate, -2, c"smods");

    lua_pushstring_opt(lstate, eap.arg.as_opt_cstr());
    lua_setfield(lstate, -2, c"args");

    lua_pushboolean_safe(lstate, eap.forceit);
    lua_setfield(lstate, -2, c"bang");

    lua_pushinteger_safe(lstate, eap.line1 as lua_Integer);
    lua_setfield(lstate, -2, c"line1");

    lua_pushinteger_safe(lstate, eap.line2 as lua_Integer);
    lua_setfield(lstate, -2, c"line2");

    lua_pushinteger_safe(lstate, eap.addr_count as lua_Integer);
    lua_setfield(lstate, -2, c"range");

    lua_pushinteger_safe(lstate, eap.line2 as lua_Integer);
    lua_setfield(lstate, -2, c"count");

    let reg: [u8; 2] = [eap.regname as u8, 0];
    lua_pushstring(lstate, CStr::from_bytes_until_nul(&reg).unwrap_or_default());
    lua_setfield(lstate, -2, c"reg");

    if !eap.args.is_empty() {
        lua_createtable_safe(lstate, eap.args.len() as i32, 0);
        for (i, (arg, &arglen)) in eap.args.iter().zip(&eap.arglens).enumerate() {
            lua_push_bytes(lstate, arg.slice(arglen));
            lua_rawseti_safe(lstate, -2, (i + 1) as i32);
        }
        lua_setfield(lstate, -2, c"fargs");
    } else {
        lua_newtable(lstate);
        let length = eap.arg.len();
        if length > 0 {
            let mut end: usize = 0;
            let mut len: usize = 0;
            let mut i: i32 = 1;
            let mut buf = vec![0u8; length];
            let mut done = false;
            let arg_slice = eap.arg.slice(length);
            while !done {
                done = uc_split_args_iter(arg_slice, &mut end, &mut buf, &mut len);
                if len > 0 {
                    lua_push_bytes(lstate, &buf[..len]);
                    lua_rawseti_safe(lstate, -2, i);
                    i += 1;
                }
            }
        }
        lua_setfield(lstate, -2, c"fargs");
    }
}

pub fn get_global_lstate() -> *mut lua_State {
    NLUA_STATE.with(|s| s.global_lstate.get())
}

fn nlua_get_error<'a>(lstate: *mut lua_State) -> Option<(&'a CStr, usize)> {
    if lua_type_safe(lstate, -1) != LUA_TSTRING {
        lua_getglobal(lstate, c"tostring");
        lua_pushvalue_safe(lstate, -2);
        if lua_pcall_safe(lstate, 1, 1, 0) != 0 || lua_type_safe(lstate, -1) != LUA_TSTRING {
            lua_pop(lstate, 1);
            lua_pushstring(lstate, c"[UNPRINTABLE ERROR]");
        }
        lua_replace_safe(lstate, -2);
    }
    lua_tolstring_cstr_with_len(lstate, -1)
}

pub fn nlua_error(lstate: *mut lua_State, msg: &CStr) {
    let (str_opt, len) = nlua_get_error(lstate).map_or((None, 0), |(s, l)| (Some(s), l));
    msg_ext_no_fast();
    semsg_multiline!(c"lua_error", msg, len as i32, str_opt);
    lua_pop(lstate, 1);
}

extern "C-unwind" fn nlua_no_traceback(_lstate: *mut lua_State) -> i32 {
    1
}

pub fn nlua_pcall(lstate: *mut lua_State, nargs: i32, mut nresults: i32) -> i32 {
    lua_getglobal(lstate, c"debug");
    if lua_istable(lstate, -1) {
        lua_getfield(lstate, -1, c"traceback");
        lua_remove_safe(lstate, -2);
    }
    let no_traceback = !lua_isfunction(lstate, -1);
    if no_traceback {
        lua_pop(lstate, 1);
        lua_pushcfunction_safe(lstate, nlua_no_traceback);
    }
    lua_insert_safe(lstate, -2 - nargs);
    let pre_top = nlua_gettop(lstate);
    let status = lua_pcall_safe(lstate, nargs, nresults, -2 - nargs);
    if status != 0 {
        lua_remove_safe(lstate, -2);
        if no_traceback && status == LUA_ERRRUN && lua_isstring_safe(lstate, -1) {
            lua_push_bytes(lstate, b"\nNo traceback: debug.traceback is not a function");
            lua_concat_safe(lstate, 2);
        }
    } else {
        if nresults == LUA_MULTRET {
            nresults = nlua_gettop(lstate) - (pre_top - nargs - 1);
        }
        lua_remove_safe(lstate, -1 - nresults);
    }
    status
}

pub fn nlua_luv_error_event(error: Option<std::ffi::CString>, err_type: isize) {
    match err_type {
        K_CALLBACK => {
            msg_ext_no_fast();
            semsg_multiline!(c"lua_error", c"Lua callback:\n%s", error);
        }
        K_THREAD => {
            semsg_multiline!(c"lua_error", c"Luv thread:\n%s", error);
        }
        K_THREAD_CALLBACK => {
            semsg_multiline!(c"lua_error", c"Luv callback, thread:\n%s", error);
        }
        _ => {}
    }
}

extern "C-unwind" fn nlua_fast_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    mut nresult: i32,
    flags: i32,
) -> i32 {
    NLUA_STATE.with(|s| s.in_fast_callback.set(s.in_fast_callback.get() + (1)));

    let top = nlua_gettop(lstate);
    let status = nlua_pcall(lstate, nargs, nresult);
    let retval = if status != 0 {
        if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
            preserve_exit(Some(E_OUTOFMEM));
        }

        let msg = nlua_get_error(lstate).map(|(s, _)| s.to_owned());
        let queues = loop_queues(&mut g_st().main_loop);
        put_event(
            queues.events.opt_mut(),
            Event::NluaLuvError {
                msg,
                kind: K_CALLBACK,
            },
        );
        lua_pop(lstate, 1);
        -status
    } else {
        if nresult == LUA_MULTRET {
            nresult = nlua_gettop(lstate) - top + nargs + 1;
        }
        nresult
    };

    NLUA_STATE.with(|s| s.in_fast_callback.set(s.in_fast_callback.get() - (1)));
    retval
}

extern "C-unwind" fn nlua_luv_thread_cb_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    nresult: i32,
    flags: i32,
) -> i32 {
    nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, true)
}

pub(crate) extern "C-unwind" fn nlua_luv_thread_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    nresult: i32,
    flags: i32,
) -> i32 {
    nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, false)
}

fn nlua_luv_thread_common_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    mut nresult: i32,
    flags: i32,
    is_callback: bool,
) -> i32 {
    let top = nlua_gettop(lstate);
    let status = lua_pcall_safe(lstate, nargs, nresult, 0);
    if status != 0 {
        if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
            fprintf!(log_get_stderr(), c"%s\n", E_OUTOFMEM,);
            nlua_exit_thread_on_oom(lstate);
        }
        let msg = nlua_get_error(lstate).map(|(s, _)| s.to_owned());
        let ev = Event::NluaLuvError {
            msg,
            kind: if is_callback {
                K_THREAD_CALLBACK
            } else {
                K_THREAD
            },
        };
        loop_schedule_deferred(&mut g_st().main_loop, ev);
        lua_pop(lstate, 1);
        -status
    } else {
        if nresult == LUA_MULTRET {
            nresult = nlua_gettop(lstate) - top + nargs + 1;
        }
        nresult
    }
}

extern "C-unwind" fn nlua_thr_api_nvim_get_runtime(lstate: *mut lua_State) -> i32 {
    if nlua_gettop(lstate) != 3 {
        return luaL_error_cstr(lstate, c"Expected 3 arguments");
    }

    luaL_checktype_safe(lstate, -1, LUA_TTABLE);
    lua_getfield(lstate, -1, c"is_lua");
    if !lua_isboolean(lstate, -1) {
        return luaL_error_cstr(lstate, c"is_lua is not a boolean");
    }
    let is_lua = lua_toboolean_safe(lstate, -1);
    lua_pop(lstate, 2);

    luaL_checktype_safe(lstate, -1, LUA_TBOOLEAN);
    let all = lua_toboolean_safe(lstate, -1);
    lua_pop(lstate, 1);

    let mut err = Error::new();
    let pat = nlua_pop_array_opt(lstate, None, &mut err);
    if error_set(&err) {
        luaL_where_safe(lstate, 1);
        lua_pushstring_opt(lstate, err.msg_cstr());
        api_clear_error(&mut err);
        lua_concat_safe(lstate, 2);
        return lua_error_safe(lstate);
    }

    let ret = runtime_get_named_thread(is_lua, pat.clone(), all);
    nlua_push_array(lstate, ret.clone(), K_NLUA_PUSH_SPECIAL);
    api_free_array(ret);
    api_free_array(pat);

    1
}

fn nlua_init_argv(lstate: *mut lua_State, argv: &[CString], lua_arg0: i32) -> i32 {
    let mut i: i32 = 0;
    lua_newtable(lstate);

    let argc = argv.len() as i32;
    if lua_arg0 > 0 {
        lua_pushstring(lstate, argv[(lua_arg0 - 1) as usize].as_c_str());
        lua_rawseti_safe(lstate, -2, 0);

        while i + lua_arg0 < argc {
            lua_pushstring(lstate, argv[(i + lua_arg0) as usize].as_c_str());
            lua_rawseti_safe(lstate, -2, i + 1);
            i += 1;
        }
    }

    lua_setglobal(lstate, c"arg");
    i
}

pub fn nlua_schedule_event(cb: LuaRef, ns_id: u32) {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());

    nlua_pushref(lstate, cb);
    nlua_unref_global(lstate, cb);

    let save_expr_map_lock = g_st().expr_map_lock;
    let save_textlock = g_st().textlock;
    if ns_id > 0 {
        g_st().expr_map_lock = 0;
        g_st().textlock = 0;
    }
    if nlua_pcall(lstate, 0, 0) != 0 {
        nlua_error(lstate, gettext(c"vim.schedule callback: %.*s"));
        ui_remove_cb(ns_id, true);
    }
    g_st().expr_map_lock = save_expr_map_lock;
    g_st().textlock = save_textlock;
}

extern "C-unwind" fn nlua_schedule(lstate: *mut lua_State) -> i32 {
    if lua_type_safe(lstate, 1) != LUA_TFUNCTION {
        lua_push_bytes(lstate, b"vim.schedule: expected function");
        return lua_error_safe(lstate);
    }

    lua_pushnil_safe(lstate);
    if g_st().main_loop.closing {
        lua_push_bytes(lstate, b"main loop is closing");
        return 2;
    }

    let cb = nlua_ref_global(lstate, 1);
    let queues = loop_queues(&mut g_st().main_loop);
    put_event(
        queues.events.opt_mut(),
        Event::NluaSchedule {
            cb,
            ns_id: g_st().ui_event_ns_id,
        },
    );
    lua_pushnil_safe(lstate);
    2
}

extern "C-unwind" fn nlua_loop_poll(lstate: *mut lua_State) -> i32 {
    let timeout = luaL_checkinteger_safe(lstate, 1) as i64;
    let fast_only = lua_toboolean_safe(lstate, 2);

    let queues = loop_queues(&mut g_st().main_loop);
    let loop_events = if fast_only {
        queues.fast_events
    } else {
        queues.events
    };
    if !loop_events.is_null() && !multiqueue_empty(loop_events) {
        multiqueue_process_events(loop_events);
    } else {
        loop_poll_events(&mut g_st().main_loop, timeout);
    }

    0
}

extern "C-unwind" fn nlua_ui_flush(_lstate: *mut lua_State) -> i32 {
    ui_flush();
    0
}

extern "C-unwind" fn nlua_check_interrupt(lstate: *mut lua_State) -> i32 {
    let interrupted = g_st().got_int;
    if g_st().got_int {
        g_st().got_int = false;
        vgetc();
    }
    lua_pushboolean_safe(lstate, interrupted);
    1
}

pub fn nlua_ex_getline(
    _c: i32,
    ecd: &ExDocmdCookie,
    _indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    nlua_pushref(lstate, ecd.getline);
    if nlua_pcall(lstate, 0, 1) != 0 {
        nlua_error(
            lstate,
            gettext(c"Error executing Ex-mode line getter: %.*s"),
        );
        return None;
    }
    if lua_type_safe(lstate, -1) != LUA_TSTRING {
        lua_pop(lstate, 1);
        return None;
    }
    let line = lua_tostring(lstate, -1).map_or_else(CString::default, CStr::to_owned);
    lua_pop(lstate, 1);
    ecd.got_line.set(true);
    Some(line)
}

extern "C-unwind" fn nlua_ex_docmd(lstate: *mut lua_State) -> i32 {
    luaL_checktype_safe(lstate, 1, LUA_TFUNCTION);
    let cookie = std::rc::Rc::new(ExDocmdCookie {
        getline: nlua_ref_global(lstate, 1),
        got_line: std::cell::Cell::new(false),
    });
    g_st().msg_scroll = 1;
    g_st().need_wait_return = false;
    g_st().no_wait_return += 1;
    do_cmdline(None, crate::cmdexpand::LineGetter::LuaEx(cookie.clone()), 0);
    g_st().no_wait_return -= 1;
    nlua_unref_global(lstate, cookie.getline);
    lua_pushboolean_safe(lstate, cookie.got_line.get());
    1
}

extern "C-unwind" fn nlua_os_exit(lstate: *mut lua_State) -> i32 {
    if NLUA_STATE.with(|s| s.in_fast_callback.get()) > 0 {
        return nlua_fast_api_disabled_error(lstate, c"os.exit");
    }

    let mut status: i32 = 0;
    if nlua_gettop(lstate) >= 1 && !lua_isnil(lstate, 1) {
        status = if lua_isboolean(lstate, 1) {
            if lua_toboolean_safe(lstate, 1) { 0 } else { 1 }
        } else {
            luaL_checkinteger_safe(lstate, 1) as i32
        };
    }
    getout(status);
}

fn nlua_new_ref_state(lstate: *mut lua_State) -> &'static mut NluaRefState {
    lua_newuserdata_typed(
        lstate,
        NluaRefState {
            nil_ref: LUA_NOREF,
            empty_dict_ref: LUA_NOREF,
            ref_count: 0,
        },
    )
}

fn nlua_get_ref_state(lstate: *mut lua_State) -> &'static mut NluaRefState {
    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state");
    let ref_state = lua_touserdata_typed::<NluaRefState>(lstate, -1);
    lua_pop(lstate, 1);
    ref_state
}

pub fn nlua_get_nil_ref(lstate: *mut lua_State) -> LuaRef {
    nlua_get_ref_state(lstate).nil_ref
}

pub fn nlua_get_empty_dict_ref(lstate: *mut lua_State) -> LuaRef {
    nlua_get_ref_state(lstate).empty_dict_ref
}

#[inline]
pub fn nlua_global_refs() -> NluaRefState {
    NLUA_STATE.with(|s| s.global_refs.get())
}

pub fn nlua_get_global_ref_count() -> i32 {
    nlua_global_refs().ref_count
}

fn nlua_common_vim_init(lstate: *mut lua_State, is_thread: bool) {
    let ref_state = nlua_new_ref_state(lstate);
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state");

    lua_pushboolean_safe(lstate, is_thread);
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
    lua_pushcfunction_safe(lstate, nlua_is_thread);
    lua_setfield(lstate, -2, c"is_thread");

    lua_createtable_safe(lstate, 0, 0);
    lua_setfield(lstate, -2, c"_core");

    lua_newuserdata_empty(lstate);
    lua_createtable_safe(lstate, 0, 0);
    lua_pushcfunction_safe(lstate, nlua_nil_tostring);
    lua_setfield(lstate, -2, c"__tostring");
    lua_pushcfunction_safe(lstate, nlua_nil_index);
    lua_setfield(lstate, -2, c"__index");
    lua_pushcfunction_safe(lstate, nlua_nil_index);
    lua_setfield(lstate, -2, c"__newindex");
    let _ = lua_setmetatable_safe(lstate, -2);
    ref_state.nil_ref = nlua_ref(lstate, ref_state, -1);
    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.NIL");
    lua_setfield(lstate, -2, c"NIL");

    lua_createtable_safe(lstate, 0, 0);
    lua_pushcfunction_safe(lstate, nlua_empty_dict_tostring);
    lua_setfield(lstate, -2, c"__tostring");
    ref_state.empty_dict_ref = nlua_ref(lstate, ref_state, -1);
    if !is_thread {
        NLUA_STATE.with(|s| s.global_refs.set(*ref_state));
    }
    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.empty_dict");
    lua_setfield(lstate, -2, c"_empty_dict_mt");

    luv_init_vim(
        lstate,
        is_thread,
        &mut g_st().main_loop,
        nlua_fast_cfpcall,
        nlua_luv_thread_cfpcall,
        nlua_luv_thread_cb_cfpcall,
    );
    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, -3, c"uv");

    lua_pushvalue_safe(lstate, -1);
    lua_setfield(lstate, -3, c"loop");

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    lua_pushvalue_safe(lstate, -3);
    lua_setfield(lstate, -2, c"luv");
    lua_pop(lstate, 3);
}

extern "C-unwind" fn nlua_module_preloader(lstate: *mut lua_State) -> i32 {
    let i = lua_tointeger_safe(lstate, lua_upvalueindex(1)) as usize;
    let def = BUILTIN_MODULES[i];
    if luaL_loadbuffer(lstate, def.data, def.chunk_name) != 0 {
        return lua_error_safe(lstate);
    }
    lua_call_safe(lstate, 0, 1);
    1
}

fn nlua_init_packages(lstate: *mut lua_State) -> bool {
    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"preload");
    for (i, def) in BUILTIN_MODULES.iter().enumerate() {
        lua_pushinteger_safe(lstate, i as lua_Integer);
        lua_pushcclosure_safe(lstate, nlua_module_preloader, 1);
        lua_setfield(lstate, -2, def.name);

        if g_st().nlua_disable_preload && def.name == c"vim.inspect" {
            break;
        }
    }

    lua_pop(lstate, 2);

    lua_getglobal(lstate, c"require");
    lua_pushstring(lstate, c"vim._init_packages");
    if nlua_pcall(lstate, 1, 0) != 0 {
        fprintf!(log_get_stderr(), c"%s\n", lua_tostring(lstate, -1));
        return false;
    }

    true
}

extern "C-unwind" fn nlua_ui_attach(lstate: *mut lua_State) -> i32 {
    let ns_id = luaL_checkinteger_safe(lstate, 1) as u32;

    if !ns_initialized(ns_id) {
        return luaL_error_cstr(lstate, c"invalid ns_id");
    }
    if !lua_istable(lstate, 2) {
        return luaL_error_cstr(lstate, c"opts must be a table");
    }
    if !lua_isfunction(lstate, 3) {
        return luaL_error_cstr(lstate, c"callback must be a Lua function");
    }

    let mut ext_widgets = [false; K_UIGLOBAL_COUNT];
    let mut tbl_has_true_val = false;

    lua_pushvalue_safe(lstate, 2);
    lua_pushnil_safe(lstate);
    while lua_next_safe(lstate, -2) != 0 {
        let s_opt = lua_tolstring_opt_cstr(lstate, -2);
        let val = lua_toboolean_safe(lstate, -1);
        let s_bytes = s_opt.map(CStr::to_bytes);

        let mut matched = false;
        if s_bytes == Some(b"set_cmdheight") {
            g_st().ui_refresh_cmdheight = val;
            matched = true;
        } else {
            for (i, slot) in ext_widgets.iter_mut().enumerate() {
                if s_bytes == Some(UI_EXT_NAMES[i].to_bytes()) {
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
            return luaL_error_cstr_arg(lstate, c"Unexpected key: %s", s_opt.unwrap_or(c""));
        }
        lua_pop(lstate, 1);
    }

    if !tbl_has_true_val {
        return luaL_error_cstr(
            lstate,
            c"opts table must contain at least one 'true' ext_widget",
        );
    }

    let ui_event_cb = nlua_ref_global(lstate, 3);
    ui_add_cb(ns_id, ui_event_cb, &ext_widgets);
    g_st().ui_refresh_cmdheight = true;
    0
}

extern "C-unwind" fn nlua_ui_detach(lstate: *mut lua_State) -> i32 {
    let ns_id = luaL_checkinteger_safe(lstate, 1) as u32;
    if !ns_initialized(ns_id) {
        return luaL_error_cstr(lstate, c"invalid ns_id");
    }
    ui_remove_cb(ns_id, false);
    0
}

fn nlua_state_init(lstate: *mut lua_State) -> bool {
    lua_pushcfunction_safe(lstate, nlua_print);
    lua_setglobal(lstate, c"print");

    lua_getglobal(lstate, c"debug");
    lua_pushcfunction_safe(lstate, nlua_debug);
    lua_setfield(lstate, -2, c"debug");
    lua_pop(lstate, 1);

    #[cfg(windows)]
    {
        lua_getglobal(lstate, c"os");
        lua_pushcfunction_safe(lstate, nlua_getenv);
        lua_setfield(lstate, -2, c"getenv");
        lua_pop(lstate, 1);
    }

    lua_getglobal(lstate, c"os");
    lua_pushcfunction_safe(lstate, nlua_os_exit);
    lua_setfield(lstate, -2, c"exit");
    lua_pop(lstate, 1);

    lua_newtable(lstate);

    nlua_add_api_functions(lstate);
    nlua_init_types(lstate);

    lua_pushcfunction_safe(lstate, nlua_schedule);
    lua_setfield(lstate, -2, c"schedule");

    lua_pushcfunction_safe(lstate, nlua_in_fast_event);
    lua_setfield(lstate, -2, c"in_fast_event");

    lua_pushcfunction_safe(lstate, nlua_call);
    lua_setfield(lstate, -2, c"call");

    lua_pushcfunction_safe(lstate, nlua_rpcrequest);
    lua_setfield(lstate, -2, c"rpcrequest");

    lua_pushcfunction_safe(lstate, nlua_rpcnotify);
    lua_setfield(lstate, -2, c"rpcnotify");

    lua_pushcfunction_safe(lstate, nlua_ui_attach);
    lua_setfield(lstate, -2, c"ui_attach");

    lua_pushcfunction_safe(lstate, nlua_ui_detach);
    lua_setfield(lstate, -2, c"ui_detach");

    nlua_common_vim_init(lstate, false);

    lua_getfield(lstate, -1, c"_core");
    lua_pushcfunction_safe(lstate, nlua_loop_poll);
    lua_setfield(lstate, -2, c"loop_poll");
    lua_pushcfunction_safe(lstate, nlua_ui_flush);
    lua_setfield(lstate, -2, c"ui_flush");
    lua_pushcfunction_safe(lstate, nlua_check_interrupt);
    lua_setfield(lstate, -2, c"check_interrupt");

    lua_pushcfunction_safe(lstate, nlua_ex_docmd);
    lua_setfield(lstate, -2, c"ex_docmd");
    lua_pop(lstate, 1);

    if has_time_fd() {
        lua_getglobal(lstate, c"require");
        NLUA_STATE.with(|s| s.require_ref.set(nlua_ref_global(lstate, -1)));
        lua_pop(lstate, 1);
        lua_pushcfunction_safe(lstate, nlua_require);
        lua_setglobal(lstate, c"require");
    }

    nlua_treesitter_init(lstate);
    nlua_state_add_stdlib(lstate, false);

    lua_setglobal(lstate, c"vim");

    nlua_init_packages(lstate)
}

pub fn nlua_init(argv: &[CString], lua_arg0: i32) {
    let lstate = luaL_newstate_safe();
    if lstate.is_null() {
        fprintf!(
            log_get_stderr(),
            gettext(c"E970: Failed to initialize Lua interpreter\n"),
        );
        os_exit(1);
    }
    luaL_openlibs_safe(lstate);
    if !nlua_state_init(lstate) {
        fprintf!(
            log_get_stderr(),
            gettext(c"E970: Failed to initialize builtin Lua modules\n"),
        );
        if MAIN_EXITFREE {
            nlua_common_free_all_mem(lstate);
        }
        os_exit(1);
    }

    luv_set_thread_cb_safe(nlua_thread_acquire_vm, nlua_common_free_all_mem);
    NLUA_STATE.with(|s| s.global_lstate.set(lstate));
    NLUA_STATE.with(|s| s.active_lstate.set(lstate));
    MAIN_THREAD.store(uv_thread_self_safe(), std::sync::atomic::Ordering::Relaxed);
    nlua_init_argv(lstate, argv, lua_arg0);
}

extern "C-unwind" fn nlua_thread_acquire_vm() -> *mut lua_State {
    nlua_init_state(true)
}

fn nlua_init_state(thread: bool) -> *mut lua_State {
    let self_ = uv_thread_self_safe();
    if uv_thread_equal_safe(
        MAIN_THREAD.load(std::sync::atomic::Ordering::Relaxed),
        self_,
    ) {
        runtime_search_path_validate();
    }

    let lstate = luaL_newstate_safe();
    luaL_openlibs_safe(lstate);

    lua_pushcfunction_safe(lstate, nlua_print);
    lua_setglobal(lstate, c"print");

    lua_pushinteger_safe(lstate, 0);
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.refcount");

    lua_newtable(lstate);

    nlua_common_vim_init(lstate, thread);
    nlua_state_add_stdlib(lstate, true);

    lua_createtable_safe(lstate, 0, 0);
    lua_pushcfunction_safe(lstate, nlua_thr_api_nvim_get_runtime);
    lua_setfield(lstate, -2, c"nvim__get_runtime");
    lua_setfield(lstate, -2, c"api");

    lua_setglobal(lstate, c"vim");

    nlua_init_packages(lstate);

    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"loaded");
    lua_getglobal(lstate, c"vim");
    lua_setfield(lstate, -2, c"vim");
    lua_pop(lstate, 2);

    lstate
}

pub fn nlua_free_all_mem() {
    if NLUA_STATE.with(|s| s.global_lstate.get()).is_null() {
        return;
    }
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    NLUA_STATE.with(|s| s.global_lstate.set(ptr::null_mut()));
    nlua_unref_global(lstate, NLUA_STATE.with(|s| s.require_ref.get()));
    nlua_common_free_all_mem(lstate);
}

extern "C-unwind" fn nlua_common_free_all_mem(lstate: *mut lua_State) {
    let ref_state = nlua_get_ref_state(lstate);
    nlua_unref(lstate, ref_state, ref_state.nil_ref);
    nlua_unref(lstate, ref_state, ref_state.empty_dict_ref);
    lua_close_safe(lstate);
}

pub fn nlua_print_event(text: NvimString) {
    msg_ext_no_fast();
    let raw = msg_use_printf() != 0
        && redirecting() == 0
        && crate::eval::typval::Callback::raw_cb_type(&g_st().on_print) == K_CALLBACK_NONE_CONST;

    let mut msg: HlMessage = vec![HlMessageChunk { text, hl_id: 0 }];
    let mut needs_clear = false;
    msg_multihl(
        OBJECT_NIL,
        &mut msg,
        Some(c"lua_print"),
        true,
        false,
        None,
        &mut needs_clear,
        raw,
    );
}

extern "C-unwind" fn nlua_print(lstate: *mut lua_State) -> i32 {
    let nargs = nlua_gettop(lstate);
    lua_getglobal(lstate, c"tostring");
    let mut msg_vec: Vec<u8> = Vec::with_capacity(80);
    let mut curargidx: i32 = 1;
    while curargidx <= nargs {
        lua_pushvalue_safe(lstate, -1);
        lua_pushvalue_safe(lstate, curargidx);
        if lua_pcall_safe(lstate, 1, 1, 0) != 0 || lua_type_safe(lstate, -1) != LUA_TSTRING {
            let (errmsg, errmsg_len) =
                nlua_get_error(lstate).map_or((None, 0), |(s, l)| (Some(s), l));
            drop(msg_vec);
            let mut buff = [0u8; IOSIZE];
            let fmt = gettext(c"E5114: Converting print argument #%i: %.*s");
            let len = format_to_buf!(&mut buff, fmt, curargidx, errmsg_len as i32, errmsg) as usize;
            lua_push_bytes(lstate, &buff[..len]);
            return lua_error_safe(lstate);
        }

        let s_slice = lua_tolstring_bytes(lstate, -1);
        if !s_slice.is_empty() {
            msg_vec.extend_from_slice(s_slice);
        }
        if curargidx < nargs {
            msg_vec.push(b' ');
        }
        lua_pop(lstate, 1);
        curargidx += 1;
    }
    let msg_str = crate::api::private::helpers::cbuf_to_string(&msg_vec);
    drop(msg_vec);

    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
    let is_thread = lua_toboolean_safe(lstate, -1);
    lua_pop(lstate, 1);

    if is_thread {
        loop_schedule_deferred(&mut g_st().main_loop, Event::NluaPrint(msg_str));
    } else if NLUA_STATE.with(|s| s.in_fast_callback.get()) != 0 {
        let queues = loop_queues(&mut g_st().main_loop);
        put_event(queues.events.opt_mut(), Event::NluaPrint(msg_str));
    } else {
        nlua_print_event(msg_str);
    }
    0
}

extern "C-unwind" fn nlua_require(lstate: *mut lua_State) -> i32 {
    let name_cstr = luaL_checkstring(lstate, 1);
    lua_settop_safe(lstate, 1);

    lua_getfield(lstate, LUA_REGISTRYINDEX, c"_LOADED");
    lua_getfield(lstate, 2, name_cstr);
    if lua_toboolean_safe(lstate, -1) {
        return 1;
    }
    lua_pop(lstate, 2);

    nlua_pushref(lstate, NLUA_STATE.with(|s| s.require_ref.get()));
    lua_insert_safe(lstate, 1);

    if !has_time_fd() {
        lua_getglobal(lstate, c"require");
        if lua_iscfunction_eq(lstate, -1, nlua_require) {
            lua_pushvalue_safe(lstate, 1);
            lua_setglobal(lstate, c"require");
        }
        lua_pop(lstate, 1);
        lua_call_safe(lstate, 1, 1);
        return 1;
    }

    let mut rel_time: Proftime = 0;
    let mut start_time: Proftime = 0;
    time_push(&mut rel_time, &mut start_time);
    let status = lua_pcall_safe(lstate, 1, 1, 0);
    if status == 0 {
        format_to_buf!(&mut g_st().iobuff, c"require('%s')", name_cstr);
        time_msg(g_st().iobuff_cstr(), Some(&start_time));
    }
    time_pop(rel_time);

    if status == 0 {
        1
    } else {
        lua_error_safe(lstate)
    }
}

extern "C-unwind" fn nlua_debug(lstate: *mut lua_State) -> i32 {
    let input_args: [TypVal; 2] = [
        TypVal {
            v_type: VarType::VarString,
            v_lock: VAR_FIXED,
            vval: TypvalVval::from_cstr(c"lua_debug> "),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    loop {
        lua_settop_safe(lstate, 0);
        let mut input: TypVal = TypVal::new();
        get_user_input(&input_args, &mut input, false, false);

        if ui_has(K_UICMDLINE) {
            format_to_buf!(
                &mut g_st().iobuff,
                c"lua_debug> %s",
                input.vval.v_string_cstr_or_empty(),
            );
            ui_ext_cmdline_block_append(0, g_st().iobuff_cstr().to_bytes());
        } else {
            msg_putchar(b'\n' as i32);
        }

        if input.v_type != VarType::VarString
            || input.vval.v_string_is_empty()
            || input.vval.v_string_bytes() == b"cont"
        {
            tv_clear(&mut input);
            if ui_has(K_UICMDLINE) {
                ui_ext_cmdline_block_leave();
            }
            return 0;
        }

        let input_bytes = input.vval.v_string_bytes();
        if luaL_loadbuffer(lstate, input_bytes, c"=(debug command)") != 0 {
            nlua_error(lstate, gettext(c"E5115: Loading Lua debug string: %.*s"));
        } else if nlua_pcall(lstate, 0, 0) != 0 {
            nlua_error(lstate, gettext(c"E5116: Calling Lua debug string: %.*s"));
        }
        tv_clear(&mut input);
    }
}

pub extern "C-unwind" fn nlua_in_fast_event(lstate: *mut lua_State) -> i32 {
    lua_pushboolean_safe(lstate, NLUA_STATE.with(|s| s.in_fast_callback.get()) > 0);
    1
}

fn viml_func_is_fast(name: &CStr) -> bool {
    find_internal_func(name).is_some_and(|fdef| fdef.fast)
}

pub extern "C-unwind" fn nlua_call(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let (name_bytes, name_with_nul) = luaL_checkbytes_with_nul(lstate, 1);
    let name_len = name_bytes.len();
    let name_cstr = crate::memory::bytes_as_cstr(name_with_nul);
    if !nlua_is_deferred_safe() && !viml_func_is_fast(name_cstr) {
        let length = name_len.min(100) + 22;
        format_to_buf!(
            &mut g_st().iobuff[..length],
            c"Vimscript function \"%s\"",
            name_cstr,
        );
        return nlua_fast_api_disabled_error(lstate, g_st().iobuff_cstr());
    }

    let nargs = nlua_gettop(lstate) - 1;
    if nargs > MAX_FUNC_ARGS {
        return luaL_error_cstr(lstate, c"Function called with too many arguments");
    }

    if let Some(func_lua) = find_internal_func_lua(name_cstr) {
        lua_getglobal(lstate, c"require");
        lua_push_bytes(lstate, b"vim._core.vimfn");
        if lua_pcall_safe(lstate, 1, 1, 0) != 0 {
            return lua_error_safe(lstate);
        }
        lua_getfield(lstate, -1, func_lua);
        lua_remove_safe(lstate, -2);
        for j in 0..nargs {
            lua_pushvalue_safe(lstate, j + 2);
        }
        if lua_pcall_safe(lstate, nargs, 1, 0) != 0 {
            return lua_error_safe(lstate);
        }
        return 1;
    }

    let mut vim_args: [TypVal; (MAX_FUNC_ARGS + 1) as usize] =
        [const { TypVal::new() }; (MAX_FUNC_ARGS + 1) as usize];
    let mut i: usize = 0;
    let mut conv_failed = false;
    while (i as i32) < nargs {
        lua_pushvalue_safe(lstate, (i as i32) + 2);
        if !nlua_pop_typval(lstate, &mut vim_args[i]) {
            api_set_error!(
                &mut err,
                ErrorType::ErrorTypeException,
                c"error converting argument %d",
                (i + 1) as i32,
            );
            conv_failed = true;
            break;
        }
        i += 1;
    }

    if !conv_failed {
        g_st().force_abort = false;
        g_st().suppress_errthrow = false;
        g_st().did_throw = false;
        g_st().did_emsg = 0;

        let mut rettv: TypVal = TypVal::new();
        let mut funcexe: Funcexe = Funcexe::new();
        funcexe.fe_firstline = curwin_mut().w_cursor.lnum;
        funcexe.fe_lastline = curwin_mut().w_cursor.lnum;
        funcexe.fe_evaluate = true;
        let save_current_sctx = api_set_sctx(LUA_INTERNAL_CALL);

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let _ = call_func(
            name_cstr,
            name_len as i32,
            &mut rettv,
            nargs,
            &mut vim_args,
            &mut funcexe,
        );
        try_leave(&tstate, &mut err);
        g_st().current_sctx = save_current_sctx;

        if !error_set(&err) {
            nlua_push_typval(lstate, &mut rettv, 0);
        }
        tv_clear(&mut rettv);
    }

    while i > 0 {
        i -= 1;
        tv_clear(&mut vim_args[i]);
    }
    if error_set(&err) {
        lua_pushstring_opt(lstate, err.msg_cstr());
        api_clear_error(&mut err);
        return lua_error_safe(lstate);
    }
    1
}

extern "C-unwind" fn nlua_rpcrequest(lstate: *mut lua_State) -> i32 {
    if !nlua_is_deferred_safe() {
        return nlua_fast_api_disabled_error(lstate, c"rpcrequest");
    }
    nlua_rpc(lstate, true)
}

extern "C-unwind" fn nlua_rpcnotify(lstate: *mut lua_State) -> i32 {
    nlua_rpc(lstate, false)
}

fn nlua_rpc(lstate: *mut lua_State, request: bool) -> i32 {
    let chan_id = luaL_checkinteger_safe(lstate, 1) as u64;

    let has_opts = request && (lua_isnil(lstate, 2) || lua_istable(lstate, 2));
    let name_idx = if has_opts { 3 } else { 2 };
    let mut timeout: i32 = -1;
    if has_opts && lua_istable(lstate, 2) {
        lua_getfield(lstate, 2, c"timeout");
        if !lua_isnil(lstate, -1) {
            if lua_type_safe(lstate, -1) != LUA_TNUMBER {
                return luaL_argerror(lstate, 2, c"timeout: expected integer");
            }
            timeout = lua_tointeger_safe(lstate, -1) as i32;
        }
        lua_pop(lstate, 1);
    }

    let name_cstr = luaL_checkstring(lstate, name_idx);
    let nargs = nlua_gettop(lstate) - name_idx;
    let mut err = Error::new();
    let mut arena = Arena::new();

    let mut args = arena_array(&mut arena, nargs.max(0) as usize);
    for i in 0..nargs {
        lua_pushvalue_safe(lstate, name_idx + 1 + i);
        let obj = nlua_pop_object(lstate, false, &mut arena, &mut err);
        *args.next_slot_mut() = obj;
        args.size += 1;
        if error_set(&err) {
            break;
        }
    }

    if !error_set(&err) {
        if request {
            let mut res_mem = ArenaMem::NULL;
            let mut result =
                rpc_send_call(chan_id, name_cstr, args, timeout, &mut res_mem, &mut err);
            if !error_set(&err) {
                nlua_push_object(lstate, &mut result, 0);
                arena_mem_free(res_mem);
            }
        } else if !rpc_send_event(chan_id, name_cstr, args) {
            let mut mbuf = [0u8; 64];
            let _ = write!(&mut mbuf[..], "Invalid channel: {chan_id}\0");
            let msg_cstr = CStr::from_bytes_until_nul(&mbuf).unwrap_or(c"");
            api_set_error!(&mut err, ErrorType::ErrorTypeValidation, c"%s", msg_cstr,);
        }
    }

    arena_mem_free(arena_finish(&mut arena));

    if error_set(&err) {
        lua_pushstring_opt(lstate, err.msg_cstr());
        api_clear_error(&mut err);
        return lua_error_safe(lstate);
    }

    if request { 1 } else { 0 }
}

extern "C-unwind" fn nlua_nil_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"vim.NIL");
    1
}

extern "C-unwind" fn nlua_nil_index(lstate: *mut lua_State) -> i32 {
    luaL_error_cstr(lstate, c"attempt to index vim.NIL")
}

extern "C-unwind" fn nlua_empty_dict_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"vim.empty_dict()");
    1
}

#[cfg(windows)]
extern "C-unwind" fn nlua_getenv(lstate: *mut lua_State) -> i32 {
    let name = luaL_checkstring(lstate, 1);
    let val = os_getenv_cstr(name);
    lua_pushstring_opt(lstate, val.as_deref());
    1
}

pub fn nlua_ref(lstate: *mut lua_State, ref_state: &mut NluaRefState, index: i32) -> LuaRef {
    lua_pushvalue_safe(lstate, index);
    let ref_ = luaL_ref_safe(lstate, LUA_REGISTRYINDEX);
    if ref_ > 0 {
        ref_state.ref_count += 1;
    }
    ref_
}

pub fn nlua_ref_global(lstate: *mut lua_State, index: i32) -> LuaRef {
    NLUA_STATE.with(|s| {
        let mut refs = s.global_refs.get();
        let r = nlua_ref(lstate, &mut refs, index);
        s.global_refs.set(refs);
        r
    })
}

pub fn nlua_unref(lstate: *mut lua_State, ref_state: &mut NluaRefState, ref_: LuaRef) {
    if ref_ > 0 {
        ref_state.ref_count -= 1;
        luaL_unref_safe(lstate, LUA_REGISTRYINDEX, ref_);
    }
}

pub fn nlua_unref_global(lstate: *mut lua_State, ref_: LuaRef) {
    NLUA_STATE.with(|s| {
        let mut refs = s.global_refs.get();
        nlua_unref(lstate, &mut refs, ref_);
        s.global_refs.set(refs);
    });
}

pub fn api_free_luaref(ref_: LuaRef) {
    if let Ok(lstate) = NLUA_STATE.try_with(|s| s.global_lstate.get()) {
        nlua_unref_global(lstate, ref_);
    }
}

pub fn nlua_pushref(lstate: *mut lua_State, ref_: LuaRef) {
    lua_rawgeti_safe(lstate, LUA_REGISTRYINDEX, ref_);
}

pub fn api_new_luaref(original_ref: LuaRef) -> LuaRef {
    if original_ref == LUA_NOREF {
        return LUA_NOREF;
    }
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    nlua_pushref(lstate, original_ref);
    let new_ref = nlua_ref_global(lstate, -1);
    lua_pop(lstate, 1);
    new_ref
}

fn nlua_exec_typval_fmt(
    prefix: &[u8],
    body: &[u8],
    suffix: &[u8],
    name: &std::ffi::CStr,
    args: &mut [TypVal],
    special: bool,
    ret_tv: &mut TypVal,
) {
    let body_end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
    let body = &body[..body_end];
    let mut lcmd = Vec::with_capacity(prefix.len() + body.len() + suffix.len());
    lcmd.extend_from_slice(prefix);
    lcmd.extend_from_slice(body);
    lcmd.extend_from_slice(suffix);

    nlua_exec_typval(&lcmd, name, args, special, Some(&mut *ret_tv));
}

pub fn nlua_call_luaeval(str_: NvimString, arg: &mut TypVal, ret_tv: &mut TypVal) {
    let body = str_.as_bytes();
    nlua_exec_typval_fmt(
        b"local _A=select(1,...) return (",
        body,
        b")",
        c"luaeval()",
        std::slice::from_mut(arg),
        true,
        &mut *ret_tv,
    );
}

pub fn nlua_call_vlua(body: &[u8], args: &mut [TypVal], ret_tv: &mut TypVal) {
    nlua_exec_typval_fmt(b"return ", body, b"(...)", c"v:lua", args, false, ret_tv);
}

pub fn nlua_call_user_expand_func(xp: &mut ExpandCtx, ret_tv: &mut TypVal) {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());

    nlua_pushref(lstate, xp.xp_luaref);
    lua_pushstring_opt(lstate, xp.xp_pattern.as_opt_cstr());
    lua_pushstring_opt(lstate, xp.xp_line.as_opt_cstr());
    lua_pushinteger_safe(lstate, xp.xp_col as lua_Integer);

    if nlua_pcall(lstate, 3, 1) != 0 {
        nlua_error(lstate, gettext(c"E5108: Lua function: %.*s"));
        return;
    }

    nlua_pop_typval(lstate, ret_tv);
}

fn nlua_exec_typval(
    lcmd: &[u8],
    name: &CStr,
    args: &mut [TypVal],
    special: bool,
    mut ret_tv: Option<&mut TypVal>,
) {
    if check_secure() {
        if let Some(ret_tv) = ret_tv.as_deref_mut() {
            ret_tv.v_type = VarType::VarNumber;
            ret_tv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
        return;
    }

    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    if luaL_loadbuffer(lstate, lcmd, name) != 0 {
        nlua_error(lstate, gettext(c"E5107: Lua: %.*s"));
        return;
    }

    let argcount = args.len() as i32;
    push_all_typvals(lstate, args, special);

    if nlua_pcall(lstate, argcount, if ret_tv.is_some() { 1 } else { 0 }) != 0 {
        nlua_error(lstate, gettext(c"E5108: Lua: %.*s"));
        return;
    }

    if let Some(ret_tv) = ret_tv {
        nlua_pop_typval(lstate, ret_tv);
    }
}

pub fn nlua_exec_lines(lines: &[std::ffi::CString], name: &CStr) {
    let mut code: Vec<u8> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            code.push(b'\n');
        }
        code.extend_from_slice(line.to_bytes());
    }
    nlua_exec_typval(&code, name, &mut [], false, None);
}

pub fn nlua_exec_typval_callable(
    lua_cb: LuaRef,
    argvars: &mut [TypVal],
    rettv: &mut TypVal,
) -> i32 {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    nlua_pushref(lstate, lua_cb);
    let argcount = argvars.len() as i32;
    push_all_typvals(lstate, argvars, false);

    if nlua_pcall(lstate, argcount, 1) != 0 {
        nlua_error(lstate, gettext(c"Lua callback: %.*s"));
        return FCERR_OTHER;
    }

    nlua_pop_typval(lstate, rettv);
    FCERR_NONE
}

pub fn nlua_exec(
    str_: NvimString,
    chunkname: Option<&CStr>,
    mut args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    let top = nlua_gettop(lstate);
    let name = match chunkname {
        Some(cn) if !cn.to_bytes().is_empty() => cn,
        _ => c"<nvim>",
    };
    let str_slice = str_.as_bytes();
    if luaL_loadbuffer(lstate, str_slice, name) != 0 {
        let (errstr, len) =
            lua_tolstring_cstr_with_len(lstate, -1).map_or((None, 0), |(s, l)| (Some(s), l));
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Lua: %.*s",
            len as i32,
            errstr
        );
        return OBJECT_NIL;
    }

    for i in 0..args.size {
        nlua_push_object(lstate, &mut args[i], 0);
    }

    if nlua_pcall(lstate, args.size as i32, 1) != 0 {
        let (errstr, len) =
            lua_tolstring_cstr_with_len(lstate, -1).map_or((None, 0), |(s, l)| (Some(s), l));
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Lua: %.*s",
            len as i32,
            errstr
        );
        return OBJECT_NIL;
    }

    nlua_call_pop_retval(lstate, mode, arena, top, Some(err))
}

pub fn nlua_call_typval(
    module: &std::ffi::CStr,
    func: &std::ffi::CStr,
    argvars: &mut [TypVal],
    rettv: Option<&mut TypVal>,
) {
    let argcount = argvars
        .iter()
        .position(|tv| tv.v_type == VarType::VarUnknown)
        .unwrap_or(argvars.len());

    let mod_bytes = module.to_bytes();
    let func_bytes = func.to_bytes();
    let mut buf = Vec::with_capacity(24 + mod_bytes.len() + func_bytes.len());
    buf.extend_from_slice(b"return require('");
    buf.extend_from_slice(mod_bytes);
    buf.extend_from_slice(b"').");
    buf.extend_from_slice(func_bytes);
    buf.extend_from_slice(b"(...)");

    nlua_exec_typval(&buf, module, &mut argvars[..argcount], false, rettv);
}

pub fn nlua_call_excmd(
    module: &std::ffi::CStr,
    func: &std::ffi::CStr,
    eap: &Exarg,
    cmod: &Cmdmod,
    extra: Option<&mut TypVal>,
) -> bool {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    lua_getglobal(lstate, c"require");
    lua_pushstring(lstate, module);
    if lua_pcall_safe(lstate, 1, 1, 0) != 0 {
        nlua_error(lstate, c"E5108: %s");
        return false;
    }
    lua_getfield(lstate, -1, func);
    lua_remove_safe(lstate, -2);

    lua_newtable(lstate);
    nlua_push_eap(lstate, eap, cmod);

    let mut nargs: i32 = 1;
    if let Some(extra) = extra {
        nlua_push_typval(lstate, extra, 0);
        nargs = 2;
    }

    if nlua_pcall(lstate, nargs, 0) != 0 {
        emsg(lua_tostring(lstate, -1).unwrap_or(c""));
        lua_pop(lstate, 1);
        return false;
    }
    true
}

pub fn nlua_ref_is_function(ref_: LuaRef) -> bool {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    nlua_pushref(lstate, ref_);
    let is_func = lua_type_safe(lstate, -1) == LUA_TFUNCTION;
    lua_pop(lstate, 1);
    is_func
}

pub fn nlua_call_ref(
    ref_: LuaRef,
    name: Option<&CStr>,
    args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: Option<&mut Error>,
) -> Object {
    nlua_call_ref_ctx(false, ref_, name, args, mode, arena, err)
}

#[inline]
const fn mode_ret(mode: LuaRetMode) -> i32 {
    if mode == K_RET_MULTI || mode == K_RET_MULTI_STACK {
        LUA_MULTRET
    } else {
        1
    }
}

pub fn nlua_call_ref_ctx(
    fast: bool,
    ref_: LuaRef,
    name: Option<&CStr>,
    mut args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: Option<&mut Error>,
) -> Object {
    let lstate = NLUA_STATE.with(|s| s.active_lstate.get());
    let top = nlua_gettop(lstate);
    nlua_pushref(lstate, ref_);
    let mut nargs = args.size as i32;
    if let Some(name) = name {
        lua_pushstring(lstate, name);
        nargs += 1;
    }
    for i in 0..args.size {
        nlua_push_object(lstate, &mut args[i], 0);
    }

    if fast {
        if nlua_fast_cfpcall(lstate, nargs, mode_ret(mode), -1) < 0 {
            if let Some(err) = err {
                api_set_error!(err, ErrorType::ErrorTypeException, c"fast context failure",);
            }
            return OBJECT_NIL;
        }
    } else if nlua_pcall(lstate, nargs, mode_ret(mode)) != 0 {
        if let Some(err) = err {
            let (errstr, len) =
                lua_tolstring_cstr_with_len(lstate, -1).map_or((None, 0), |(s, l)| (Some(s), l));
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Lua: %.*s",
                len as i32,
                errstr
            );
        } else {
            nlua_error(lstate, gettext(c"Lua callback: %.*s"));
        }
        return OBJECT_NIL;
    }

    nlua_call_pop_retval(lstate, mode, arena, top, err)
}

fn nlua_call_pop_retval(
    lstate: *mut lua_State,
    mode: LuaRetMode,
    mut arena: Option<&mut Arena>,
    pretop: i32,
    err: Option<&mut Error>,
) -> Object {
    if mode != K_RET_MULTI && mode != K_RET_MULTI_STACK && lua_isnil(lstate, -1) {
        lua_pop(lstate, 1);
        return OBJECT_NIL;
    }
    let mut dummy = Error::new();
    let perr = err.unwrap_or(&mut dummy);

    match mode {
        K_RET_NIL_BOOL => {
            let bool_value = lua_toboolean_safe(lstate, -1);
            lua_pop(lstate, 1);
            Object {
                obj_type: ObjectType::ObjectTypeBoolean,
                data: ObjectData::Boolean(bool_value),
            }
        }
        K_RET_LUAREF => {
            let ref_ = nlua_ref_global(lstate, -1);
            lua_pop(lstate, 1);
            Object {
                obj_type: ObjectType::ObjectTypeLuaRef,
                data: ObjectData::LuaRef(ref_),
            }
        }
        K_RET_OBJECT => nlua_pop_object_opt(lstate, false, arena, perr),
        K_RET_MULTI => {
            let nres = nlua_gettop(lstate) - pretop;
            let nres_u = nres.max(0) as usize;
            let mut res =
                crate::api::private::helpers::arena_array_opt(arena.as_deref_mut(), nres_u);
            res.size = nres_u;
            for i in 0..nres_u {
                res[nres_u - i - 1] =
                    nlua_pop_object_opt(lstate, false, arena.as_deref_mut(), perr);
                if error_set(perr) {
                    return OBJECT_NIL;
                }
            }
            Object {
                obj_type: ObjectType::ObjectTypeArray,
                data: ObjectData::Array(res),
            }
        }
        K_RET_MULTI_STACK => OBJECT_NIL,
        _ => unreachable!(),
    }
}

pub fn nlua_is_deferred_safe() -> bool {
    NLUA_STATE.with(|s| s.in_fast_callback.get()) == 0
}

pub fn ex_lua(eap: &mut Exarg) {
    if eap.arg.is_empty() {
        if eap.addr_count > 0 {
            cmd_source_buffer(Some(eap), true);
        } else {
            emsg(gettext(E_ARGREQ));
        }
        return;
    }

    let mut len: usize = 0;
    let raw_code = script_get(eap, &mut len);
    let Some(raw_cstr) = raw_code else {
        return;
    };
    if eap.skip {
        return;
    }

    let raw_bytes = raw_cstr.to_bytes();
    if eap.cmdidx == Cmdidx::CmdEqual || raw_bytes.first() == Some(&b'=') {
        let off: usize = if eap.cmdidx == Cmdidx::CmdEqual { 0 } else { 1 };
        let body = &raw_bytes[off..];
        let mut buf = Vec::with_capacity(18 + body.len());
        buf.extend_from_slice(b"vim._print(true, ");
        buf.extend_from_slice(body);
        buf.push(b')');
        nlua_exec_typval(&buf, c":lua", &mut [], false, None);
    } else {
        nlua_exec_typval(raw_bytes, c":lua", &mut [], false, None);
    }
}

pub fn ex_luado(eap: &mut Exarg) {
    if !u_save(eap.line1 - 1, eap.line2 + 1) {
        emsg(gettext(c"cannot save undo information"));
        return;
    }
    let cmd_bytes = eap.arg.as_bytes();
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());

    const DOSTART: &[u8] = b"return function(line, linenr) ";
    const DOEND: &[u8] = b" end";
    let mut lcmd = Vec::with_capacity(cmd_bytes.len() + DOSTART.len() + DOEND.len());
    lcmd.extend_from_slice(DOSTART);
    lcmd.extend_from_slice(cmd_bytes);
    lcmd.extend_from_slice(DOEND);

    if luaL_loadbuffer(lstate, &lcmd, c":luado") != 0 {
        nlua_error(lstate, gettext(c"E5109: Lua: %.*s"));
        return;
    }
    drop(lcmd);
    if nlua_pcall(lstate, 0, 1) != 0 {
        nlua_error(lstate, gettext(c"E5110: Lua: %.*s"));
        return;
    }

    let was_curbuf = g_st().curbuf;
    for l in eap.line1..=eap.line2 {
        if l > curbuf_mut().b_ml.ml_line_count {
            break;
        }

        lua_pushvalue_safe(lstate, -1);
        let old_line_cstr = ml_get_buf(crate::globals::curbuf_mut(), l);
        let old_line_len: Colnr = ml_get_buf_len(crate::globals::curbuf_mut(), l);
        lua_pushstring(lstate, old_line_cstr);
        lua_pushnumber_safe(lstate, f64::from(l));
        if nlua_pcall(lstate, 2, 1) != 0 {
            nlua_error(lstate, gettext(c"E5111: Lua: %.*s"));
            break;
        }

        if g_st().curbuf != was_curbuf || l > curbuf_mut().b_ml.ml_line_count {
            break;
        }

        if lua_isstring_safe(lstate, -1) {
            let new_line_slice = lua_tolstring_bytes(lstate, -1);
            let new_line_len = new_line_slice.len();
            let mut new_line_vec = vec![0u8; new_line_len + 1];
            new_line_vec[..new_line_len].copy_from_slice(new_line_slice);
            for b in &mut new_line_vec[..new_line_len] {
                if *b == 0 {
                    *b = b'\n';
                }
            }
            crate::memline::ml_replace_len(l, &new_line_vec[..new_line_len], new_line_len, true);
            inserted_bytes(l, 0, old_line_len, new_line_len as i32);
        }
        lua_pop(lstate, 1);
    }

    lua_pop(lstate, 1);
    check_cursor(curwin_mut());
    redraw_curbuf_later(UPD_NOT_VALID);
}

pub fn ex_luafile(eap: &mut Exarg) {
    let _ = nlua_exec_file(eap.arg.as_cstr());
}

pub fn nlua_exec_file(path: &CStr) -> bool {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    if path.to_bytes() != b"-" {
        lua_getglobal(lstate, c"loadfile");
        lua_pushstring(lstate, path);
    } else {
        let mut stdin_dup = FileDescriptor::new();
        if file_open_stdin(&mut stdin_dup) != 0 {
            return false;
        }

        let mut sb: Vec<u8> = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        loop {
            if g_st().got_int {
                file_close(&mut stdin_dup, false);
                return false;
            }
            let read_size = file_read(&mut stdin_dup, &mut buf);
            if read_size < 0 {
                file_close(&mut stdin_dup, false);
                return false;
            }
            if read_size > 0 {
                sb.extend_from_slice(&buf[..read_size as usize]);
            }
            if read_size < 64 {
                break;
            }
        }
        sb.push(0);
        file_close(&mut stdin_dup, false);

        lua_getglobal(lstate, c"loadstring");
        lua_pushstring(lstate, CStr::from_bytes_until_nul(&sb).unwrap_or_default());
    }

    if nlua_pcall(lstate, 1, 2) != 0 {
        nlua_error(lstate, gettext(c"E5111: Lua: %.*s"));
        return false;
    }

    if lua_isnil(lstate, -2) {
        nlua_error(lstate, gettext(c"E5112: Lua chunk: %.*s"));
        assert!(lua_isnil(lstate, -1));
        lua_pop(lstate, 1);
        return false;
    }

    assert!(lua_isnil(lstate, -1));
    lua_pop(lstate, 1);

    if nlua_pcall(lstate, 0, 0) != 0 {
        nlua_error(lstate, gettext(c"E5113: Lua chunk: %.*s"));
        return false;
    }

    true
}

pub fn nlua_expand_pat(xp: &mut ExpandCtx) {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    let mut status = false;

    lua_getglobal(lstate, c"vim");
    lua_getfield(lstate, -1, c"_expand_pat");
    luaL_checktype_safe(lstate, -1, LUA_TFUNCTION);

    let line_end = xp.xp_line.wrapping_add(xp.xp_col as usize);
    assert!(line_end.addr() >= xp.xp_pattern.addr());
    let patlen = line_end.offset_from(&xp.xp_pattern) as isize;
    lua_push_bytes(lstate, xp.xp_pattern.slice(patlen as usize));

    if nlua_pcall(lstate, 1, 2) != 0 {
        nlua_error(lstate, gettext(c"vim._expand_pat: %.*s"));
        return;
    }

    let mut err = Error::new();
    let mut arena = Arena::new();
    let prefix_len = nlua_pop_integer(lstate, &mut arena, &mut err) as isize;
    if !error_set(&err) && prefix_len <= patlen {
        let completions = nlua_pop_array(lstate, &mut arena, &mut err);
        if !error_set(&err) {
            let mut results = Vec::with_capacity(completions.size);

            let mut all_strings = true;
            for v in completions.as_slice() {
                if v.obj_type != ObjectType::ObjectTypeString {
                    all_strings = false;
                    break;
                }
                results.push(crate::memory::bytes_to_cstring(v.data.as_bytes()));
            }
            NLUA_EXPAND_RESULTS.with(|r| *r.borrow_mut() = results);

            if all_strings {
                xp.xp_pattern = xp.xp_pattern.offset_in(&xp.xp_line, prefix_len);
                status = true;
            }
        }
        arena_mem_free(arena_finish(&mut arena));
    }

    if !status {
        NLUA_EXPAND_RESULTS.with(|r| r.borrow_mut().clear());
    }
}

pub fn nlua_expand_get_matches(results: &mut Vec<std::ffi::CString>) -> bool {
    *results = NLUA_EXPAND_RESULTS.with(|r| std::mem::take(&mut *r.borrow_mut()));
    !results.is_empty()
}

extern "C-unwind" fn nlua_is_thread(lstate: *mut lua_State) -> i32 {
    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
    1
}

pub fn nlua_is_table_from_lua(arg: &TypVal) -> bool {
    if arg.v_type == VarType::VarDict {
        arg.vval.v_dict().lua_table_ref != LUA_NOREF
    } else if arg.v_type == VarType::VarList {
        arg.vval.v_list().lua_table_ref.get() != LUA_NOREF
    } else {
        false
    }
}

pub fn nlua_register_table_as_callable(arg: &TypVal) -> Option<std::ffi::CString> {
    let table_ref = if arg.v_type == VarType::VarDict {
        arg.vval.v_dict().lua_table_ref
    } else if arg.v_type == VarType::VarList {
        arg.vval.v_list().lua_table_ref.get()
    } else {
        LUA_NOREF
    };

    if table_ref == LUA_NOREF {
        return None;
    }

    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    let top = nlua_gettop(lstate);

    nlua_pushref(lstate, table_ref);
    if !lua_getmetatable_safe(lstate, -1) {
        lua_pop(lstate, 1);
        debug_assert_eq!(top, nlua_gettop(lstate));
        return None;
    }

    lua_getfield(lstate, -1, c"__call");
    if !lua_isfunction(lstate, -1) {
        lua_pop(lstate, 3);
        debug_assert_eq!(top, nlua_gettop(lstate));
        return None;
    }
    lua_pop(lstate, 2);

    let func = nlua_ref_global(lstate, -1);
    let name = register_luafunc(func);
    lua_pop(lstate, 1);
    debug_assert_eq!(top, nlua_gettop(lstate));

    Some(name)
}

pub fn nlua_exec_on_key(c: i32, typed_buf: &mut [u8]) -> bool {
    static RECURSIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RECURSIVE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }

    let mut buf = [0u8; MB_MAXBYTES * 3 + 4];
    let buf_len = special_to_buf(c, g_st().mod_mask, false, &mut buf) as usize;
    vim_unescape_ks(typed_buf);

    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    let top = nlua_gettop(lstate);

    lua_getglobal(lstate, c"vim");
    lua_getfield(lstate, -1, c"_on_key");
    luaL_checktype_safe(lstate, -1, LUA_TFUNCTION);

    lua_push_bytes(lstate, &buf[..buf_len]);
    lua_pushstring(
        lstate,
        CStr::from_bytes_until_nul(typed_buf).unwrap_or_default(),
    );

    let save_got_int = g_st().got_int;
    g_st().got_int = false;
    let mut discard = false;
    if lua_pcall_safe(lstate, 2, 1, 0) != 0 {
        nlua_error(lstate, gettext(c"vim.on_key() callbacks: %.*s"));
    } else {
        if lua_isboolean(lstate, -1) {
            discard = lua_toboolean_safe(lstate, -1);
        }
        lua_pop(lstate, 1);
    }
    g_st().got_int |= save_got_int;

    lua_pop(lstate, 1);
    debug_assert_eq!(top, nlua_gettop(lstate));

    RECURSIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    discard
}

pub fn nlua_set_sctx(current: &mut Sctx) {
    if !script_is_lua(current.sc_sid) {
        return;
    }

    current.sc_lnum = 0;

    if opt_vars().p_verbose <= 0 {
        return;
    }
    let lstate = NLUA_STATE.with(|s| s.active_lstate.get());
    let mut info = lua_Debug::new();

    let mut level: i32 = 1;
    let mut found = false;
    loop {
        if lua_getstack_safe(lstate, level, &mut info) != 1 {
            break;
        }
        if lua_getinfo(lstate, c"nSl", &mut info) == 0 {
            break;
        }
        if info.is_c_what() || info.file_source_cstr().is_none() {
            level += 1;
            continue;
        }
        found = true;
        break;
    }
    if found
        && let Some(src_cstr) = info.file_source_cstr()
        && let Some(source_path) = fix_fname(Some(src_cstr))
    {
        let mut sid = find_script_by_name(source_path.as_c_str());
        if sid <= 0 {
            new_script_item(Some(source_path), &mut sid);
            crate::runtime::with_script_item_mut(sid, |si| si.sn_lua = true);
        }
        current.sc_sid = sid;
        current.sc_seq = -1;
        current.sc_lnum = info.currentline;
    }
}

pub fn nlua_do_ucmd(
    uc_name: &CStr,
    luaref: LuaRef,
    uc_argt: u32,
    uc_def: i64,
    eap: &mut Exarg,
    preview: bool,
) -> i32 {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    nlua_pushref(lstate, luaref);

    lua_newtable(lstate);
    nlua_push_eap(lstate, &*eap, &g_st().cmdmod);

    lua_pushstring(lstate, uc_name);
    lua_setfield(lstate, -2, c"name");

    if eap.addr_count == 0 {
        lua_pushinteger_safe(lstate, uc_def as lua_Integer);
        lua_setfield(lstate, -2, c"count");
    }

    if (uc_argt & EX_NOSPC) != 0 {
        lua_createtable_safe(lstate, 1, 0);
        if (uc_argt & EX_NEEDARG) != 0 || !eap.arg.is_empty() {
            lua_pushstring_opt(lstate, eap.arg.as_opt_cstr());
            lua_rawseti_safe(lstate, -2, 1);
        }
        lua_setfield(lstate, -2, c"fargs");
    }

    let narg_ch = if (uc_argt & EX_EXTRA) != 0 {
        if (uc_argt & EX_NOSPC) != 0 {
            if (uc_argt & EX_ARGSPACE) != 0 {
                b'_'
            } else if (uc_argt & EX_NEEDARG) != 0 {
                b'1'
            } else {
                b'?'
            }
        } else if (uc_argt & EX_NEEDARG) != 0 {
            b'+'
        } else {
            b'*'
        }
    } else {
        b'0'
    };
    let nargs: [u8; 2] = [narg_ch, 0];
    lua_pushstring(
        lstate,
        CStr::from_bytes_until_nul(&nargs).unwrap_or_default(),
    );
    lua_setfield(lstate, -2, c"nargs");

    if preview {
        lua_pushinteger_safe(lstate, cmdpreview_get_ns() as lua_Integer);
        let cmdpreview_bufnr: ObjHandle = cmdpreview_get_bufnr();
        if cmdpreview_bufnr != 0 {
            lua_pushinteger_safe(lstate, cmdpreview_bufnr as lua_Integer);
        } else {
            lua_pushnil_safe(lstate);
        }
    }

    if nlua_pcall(
        lstate,
        if preview { 3 } else { 1 },
        if preview { 1 } else { 0 },
    ) != 0
    {
        nlua_error(lstate, gettext(c"Lua :command callback: %.*s"));
        return 0;
    }

    let mut retv: i32 = 0;
    if preview && lua_isnumber_safe(lstate, -1) {
        let v = lua_tointeger_safe(lstate, -1) as i32;
        if (0..=2).contains(&v) {
            retv = v;
            lua_pop(lstate, 1);
        }
    }

    retv
}

pub fn nlua_funcref_str(ref_: LuaRef, show_ref: bool) -> std::ffi::CString {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    if lua_checkstack_safe(lstate, 1) {
        nlua_pushref(lstate, ref_);
        if !lua_isfunction(lstate, -1) {
            lua_pop(lstate, 1);
        } else {
            let mut ar = lua_Debug::new();
            if lua_getinfo(lstate, c">S", &mut ar) != 0
                && let Some(src_cstr) = ar.file_source_cstr()
                && ar.linedefined >= 0
            {
                let src = home_replace_save(None, Some(src_cstr));
                let src_s = src.to_string_lossy();
                let formatted = if show_ref {
                    format!("<Lua {ref_}: {src_s}:{}>", ar.linedefined)
                } else {
                    format!("<Lua {src_s}:{}>", ar.linedefined)
                };
                return crate::memory::bytes_to_cstring(formatted.as_bytes());
            }
        }
    }

    let formatted = if show_ref {
        format!("<Lua {ref_}>")
    } else {
        "<Lua>".to_string()
    };
    crate::memory::bytes_to_cstring(formatted.as_bytes())
}

pub fn nlua_init_defaults() {
    let lstate = NLUA_STATE.with(|s| s.global_lstate.get());
    assert!(!lstate.is_null());

    lua_getglobal(lstate, c"require");
    lua_pushstring(lstate, c"vim._core.defaults");
    if nlua_pcall(lstate, 1, 0) != 0 {
        fprintf!(log_get_stderr(), c"%s\n", lua_tostring(lstate, -1));
    }
}

pub fn nlua_func_exists(lua_funcname: &CStr) -> bool {
    let fname_bytes = lua_funcname.to_bytes();
    let mut str_bytes = Vec::with_capacity(7 + fname_bytes.len());
    str_bytes.extend_from_slice(b"return ");
    str_bytes.extend_from_slice(fname_bytes);
    let str_cstr = std::ffi::CString::new(str_bytes).unwrap();
    let mut args_items: [Object; 1] = [Object {
        obj_type: ObjectType::ObjectTypeString,
        data: ObjectData::String(NvimString::from_cstr(&str_cstr)),
    }];
    let args = Array::from_slice(&mut args_items[..1]);
    let mut err = Error::new();
    let code = NvimString::from_cstr(c"return type(loadstring(...)()) == 'function'");
    let result = nlua_exec(code, None, args, K_RET_NIL_BOOL, None, &mut err);
    drop(str_cstr);

    api_clear_error(&mut err);
    result.obj_type == ObjectType::ObjectTypeBoolean && result.data.boolean()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::command::is_user_cmdidx;

    #[test]
    fn mode_ret_and_user_cmdidx_helpers_work() {
        assert_eq!(mode_ret(K_RET_OBJECT), 1);
        assert_eq!(mode_ret(K_RET_MULTI), LUA_MULTRET);
        assert_eq!(mode_ret(K_RET_MULTI_STACK), LUA_MULTRET);
        assert!(is_user_cmdidx(Cmdidx::CmdUser));
        assert!(is_user_cmdidx(Cmdidx::CmdUserBuf));
        assert!(!is_user_cmdidx(0));
        assert!(nlua_is_deferred_safe());
        assert_eq!(api_new_luaref(LUA_NOREF), LUA_NOREF);
        assert_eq!(BUILTIN_MODULES[0].name, c"vim._init_packages");
        assert_eq!(BUILTIN_MODULES[1].name, c"vim.inspect");
        for m in &BUILTIN_MODULES {
            assert!(!m.chunk_name.to_bytes().is_empty());
            assert!(!m.data.is_empty());
            assert!(std::str::from_utf8(m.data).is_ok());
        }
    }
}
