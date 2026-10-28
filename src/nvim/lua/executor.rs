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
use crate::cursor::{Colnr, check_cursor};
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
use crate::errors::*;
use crate::eval::funcs::{find_internal_func, find_internal_func_lua};
use crate::eval::typval::{TypVal, TypvalVval, VarType, tv_clear};
use crate::eval::userfunc::{
    EvalFuncDef, FCERR_NONE, FCERR_OTHER, Funcexe, call_func, register_luafunc,
};
use crate::event::r#loop::{
    loop_closing_ptr, loop_poll_events, loop_queues, loop_schedule_deferred, uv_thread_t,
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
    K_NLUA_PUSH_SPECIAL, LUA_ERRMEM, LUA_ERRRUN, LUA_MULTRET, LUA_REGISTRYINDEX, LUA_TBOOLEAN,
    LUA_TFUNCTION, LUA_TSTRING, LUA_TTABLE, LuaRef, NluaRefState, lua_CFunction, lua_Integer,
    lua_State, lua_call, lua_checkstack, lua_close, lua_concat, lua_createtable, lua_error,
    lua_getfield, lua_getglobal, lua_getmetatable, lua_gettop, lua_insert, lua_isboolean,
    lua_iscfunction, lua_isfunction, lua_isnil, lua_isnumber, lua_isstring, lua_istable,
    lua_newtable, lua_newuserdata, lua_next, lua_pcall, lua_pop, lua_pushboolean, lua_pushcclosure,
    lua_pushcfunction, lua_pushinteger, lua_pushlightuserdata, lua_pushlstring, lua_pushnil,
    lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawgeti, lua_rawseti, lua_remove,
    lua_replace, lua_setfield, lua_setglobal, lua_setmetatable, lua_settop, lua_toboolean,
    lua_tocfunction, lua_tointeger, lua_tolstring, lua_tostring, lua_touserdata, lua_type,
    lua_upvalueindex, luaL_checkinteger, luaL_checklstring, luaL_checktype, luaL_error,
    luaL_loadbuffer, luaL_newstate, luaL_openlibs, luaL_ref, luaL_unref, luaL_where,
    nlua_init_types, nlua_pop_array, nlua_pop_array_opt, nlua_pop_integer, nlua_pop_object,
    nlua_pop_object_opt, nlua_pop_typval, nlua_push_array, nlua_push_object, nlua_push_typval,
};
use crate::lua::stdlib::nlua_state_add_stdlib;
use crate::lua::treesitter::nlua_treesitter_init;
use crate::main_init::{getout, os_exit, preserve_exit};
use crate::math::{FAIL, OK};
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
use crate::profile::{Proftime, Scriptitem, time_msg, time_pop, time_push};
use crate::runtime::{
    Sctx, cmd_source_buffer, find_script_by_name, new_script_item, runtime_get_named_thread,
    runtime_search_path_validate, script_is_lua,
};
use crate::strings::{gettext, vim_snprintf};
use crate::ui::K_UIGLOBAL_COUNT;
use crate::ui::{ui_add_cb, ui_flush, ui_has, ui_remove_cb};
use crate::undo::u_save;
use crate::usercmd::{Ucmd, uc_mods, uc_split_args_iter};
use std::ffi::{CStr, c_char, c_void};
use std::io::Write;
use std::mem::size_of;
use std::ptr;

use crate::api::vim::HlMessage;
pub type LuaRetMode = i32;

#[allow(non_camel_case_types)]
type luv_CFpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        nargs: i32,
        nresults: i32,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
type luv_CFcpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        func: lua_CFunction,
        ud: *mut c_void,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
type luv_acquire_vm = Option<unsafe extern "C-unwind" fn() -> *mut lua_State>;
#[allow(non_camel_case_types)]
type luv_release_vm = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State)>;

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

const CMD_EQUAL: i32 = 559;

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

#[repr(C)]
pub struct ExDocmdCookie {
    getline: LuaRef,
    got_line: bool,
}

use crate::event::r#loop::{uv_thread_equal, uv_thread_self};
use crate::lua::converter::{lua_Debug, lua_getinfo, lua_getstack};

pub(crate) struct LuaExecutorState {
    pub(crate) active_lstate: *mut lua_State,
    in_fast_callback: i32,
    global_lstate: *mut lua_State,
    require_ref: LuaRef,
    main_thread: uv_thread_t,
    expand_result_array: Vec<std::ffi::CString>,
}

impl LuaExecutorState {
    const fn new() -> Self {
        Self {
            active_lstate: ptr::null_mut(),
            in_fast_callback: 0,
            global_lstate: ptr::null_mut(),
            require_ref: LUA_REFNIL,
            main_thread: 0,
            expand_result_array: Vec::new(),
        }
    }
}

static NLUA_STATE: GlobalState<LuaExecutorState> = GlobalState::new(LuaExecutorState::new());

#[inline]
pub(crate) fn nlua_st() -> &'static mut LuaExecutorState {
    unsafe { &mut *NLUA_STATE.as_ptr() }
}

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::api::private::dispatch::nlua_add_api_functions;
use crate::buffer::VAR_FIXED;
use crate::charset::NUL;
use crate::cmdexpand::{K_UICMDLINE, MB_MAXBYTES};
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_docmd::{
    CMD_SIZE, CMOD_BROWSE, CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS,
    CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE,
    CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, EX_ARGSPACE, EX_EXTRA, EX_NEEDARG, EX_NOSPC,
};
use crate::globals::IOSIZE;
use crate::lua::converter::LUA_NOREF;
#[cfg(windows)]
use crate::os::os_win_console::ExitThread;
use crate::version::MAIN_EXITFREE;
use crate::window::{WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT};

unsafe extern "C-unwind" {
    fn luaopen_luv(lstate: *mut lua_State) -> i32;
}

unsafe extern "C" {
    fn luv_set_loop(lstate: *mut lua_State, loop_: *mut c_void);
    fn luv_set_callback(lstate: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_thread(lstate: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_cthread(lstate: *mut lua_State, cpcall: luv_CFcpcall);
    fn luv_set_thread_cb(acquire: luv_acquire_vm, release: luv_release_vm);

    #[cfg(unix)]
    fn pthread_exit(retval: *mut c_void) -> !;
}

#[inline]
fn put_event(mq: *mut MultiQueue, ev: Event) {
    if !mq.is_null() {
        multiqueue_put_event(unsafe { &mut *mq }, ev);
    }
}

fn push_all_typvals(lstate: *mut lua_State, args: *mut TypVal, argcount: i32, special: bool) {
    let flags = if special { K_NLUA_PUSH_SPECIAL } else { 0 };
    for i in 0..(argcount as usize) {
        let arg = args.wrapping_add(i);
        if unsafe { (*arg).v_type } == VarType::VarUnknown {
            unsafe { lua_pushnil(lstate) };
        } else {
            nlua_push_typval(lstate, arg, flags);
        }
    }
}

fn nlua_push_cmdmod(lstate: *mut lua_State, cmod: &Cmdmod) {
    unsafe {
        lua_newtable(lstate);

        lua_pushinteger(lstate, (cmod.cmod_tab - 1) as lua_Integer);
        lua_setfield(lstate, -2, c"tab");

        lua_pushinteger(lstate, (cmod.cmod_verbose - 1) as lua_Integer);
        lua_setfield(lstate, -2, c"verbose");

        let split_str = if (cmod.cmod_split & WSP_ABOVE) != 0 {
            c"aboveleft"
        } else if (cmod.cmod_split & WSP_BELOW) != 0 {
            c"belowright"
        } else if (cmod.cmod_split & WSP_TOP) != 0 {
            c"topleft"
        } else if (cmod.cmod_split & WSP_BOT) != 0 {
            c"botright"
        } else {
            c""
        };
        lua_pushstring(lstate, split_str.as_ptr());
        lua_setfield(lstate, -2, c"split");

        lua_pushboolean(lstate, i32::from((cmod.cmod_split & WSP_VERT) != 0));
        lua_setfield(lstate, -2, c"vertical");
        lua_pushboolean(lstate, i32::from((cmod.cmod_split & WSP_HOR) != 0));
        lua_setfield(lstate, -2, c"horizontal");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_SILENT) != 0));
        lua_setfield(lstate, -2, c"silent");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_ERRSILENT) != 0));
        lua_setfield(lstate, -2, c"emsg_silent");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_UNSILENT) != 0));
        lua_setfield(lstate, -2, c"unsilent");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_SANDBOX) != 0));
        lua_setfield(lstate, -2, c"sandbox");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_NOAUTOCMD) != 0));
        lua_setfield(lstate, -2, c"noautocmd");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_BROWSE) != 0));
        lua_setfield(lstate, -2, c"browse");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_CONFIRM) != 0));
        lua_setfield(lstate, -2, c"confirm");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_HIDE) != 0));
        lua_setfield(lstate, -2, c"hide");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_KEEPALT) != 0));
        lua_setfield(lstate, -2, c"keepalt");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_KEEPJUMPS) != 0));
        lua_setfield(lstate, -2, c"keepjumps");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_KEEPMARKS) != 0));
        lua_setfield(lstate, -2, c"keepmarks");
        lua_pushboolean(
            lstate,
            i32::from((cmod.cmod_flags & CMOD_KEEPPATTERNS) != 0),
        );
        lua_setfield(lstate, -2, c"keeppatterns");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_LOCKMARKS) != 0));
        lua_setfield(lstate, -2, c"lockmarks");
        lua_pushboolean(lstate, i32::from((cmod.cmod_flags & CMOD_NOSWAPFILE) != 0));
        lua_setfield(lstate, -2, c"noswapfile");

        lua_newtable(lstate);
        lua_pushstring(
            lstate,
            cmod.cmod_filter_pat
                .as_ref()
                .map_or(c"".as_ptr(), |s| s.as_ptr()),
        );
        lua_setfield(lstate, -2, c"pattern");
        lua_pushboolean(lstate, i32::from(cmod.cmod_filter_force));
        lua_setfield(lstate, -2, c"force");
        lua_setfield(lstate, -2, c"filter");
    }
}

fn nlua_push_eap(lstate: *mut lua_State, eap: &Exarg, cmod: &Cmdmod) {
    if !is_user_cmdidx(eap.cmdidx) && eap.cmdidx < CMD_SIZE {
        lua_pushstring(
            lstate,
            get_command_name(&mut crate::os::env::ExpandCtx::new(), eap.cmdidx),
        );
        lua_setfield(lstate, -2, c"name");
    }

    let mut mods_buf = [0 as c_char; 200];
    uc_mods(mods_buf.as_mut_ptr(), cmod, false);
    lua_pushstring(lstate, mods_buf.as_ptr());
    lua_setfield(lstate, -2, c"mods");

    nlua_push_cmdmod(lstate, cmod);
    lua_setfield(lstate, -2, c"smods");

    lua_pushstring(lstate, eap.arg);
    lua_setfield(lstate, -2, c"args");

    unsafe { lua_pushboolean(lstate, eap.forceit) };
    lua_setfield(lstate, -2, c"bang");

    unsafe { lua_pushinteger(lstate, eap.line1 as lua_Integer) };
    lua_setfield(lstate, -2, c"line1");

    unsafe { lua_pushinteger(lstate, eap.line2 as lua_Integer) };
    lua_setfield(lstate, -2, c"line2");

    unsafe { lua_pushinteger(lstate, eap.addr_count as lua_Integer) };
    lua_setfield(lstate, -2, c"range");

    unsafe { lua_pushinteger(lstate, eap.line2 as lua_Integer) };
    lua_setfield(lstate, -2, c"count");

    let reg: [c_char; 2] = [eap.regname as c_char, NUL];
    lua_pushstring(lstate, reg.as_ptr());
    lua_setfield(lstate, -2, c"reg");

    if !eap.args.is_empty() {
        unsafe { lua_createtable(lstate, eap.args.len() as i32, 0) };
        for (i, (&arg, &arglen)) in eap.args.iter().zip(&eap.arglens).enumerate() {
            unsafe { lua_pushlstring(lstate, arg, arglen) };
            unsafe { lua_rawseti(lstate, -2, (i + 1) as i32) };
        }
        lua_setfield(lstate, -2, c"fargs");
    } else {
        lua_newtable(lstate);
        let length = crate::memory::cstr_len(eap.arg);
        if length > 0 {
            let mut end: usize = 0;
            let mut len: usize = 0;
            let mut i: i32 = 1;
            let mut buf = vec![0 as c_char; length];
            let mut done = false;
            while !done {
                done = uc_split_args_iter(eap.arg, length, &mut end, buf.as_mut_ptr(), &mut len);
                if len > 0 {
                    unsafe { lua_pushlstring(lstate, buf.as_ptr(), len) };
                    unsafe { lua_rawseti(lstate, -2, i) };
                    i += 1;
                }
            }
        }
        lua_setfield(lstate, -2, c"fargs");
    }
}

pub fn get_global_lstate() -> *mut lua_State {
    nlua_st().global_lstate
}

fn nlua_get_error(lstate: *mut lua_State, len: &mut usize) -> *const c_char {
    unsafe {
        if lua_type(lstate, -1) != LUA_TSTRING {
            lua_getglobal(lstate, c"tostring");
            lua_pushvalue(lstate, -2);
            if lua_pcall(lstate, 1, 1, 0) != 0 || lua_type(lstate, -1) != LUA_TSTRING {
                lua_pop(lstate, 1);
                lua_pushstring(lstate, c"[UNPRINTABLE ERROR]");
            }
            lua_replace(lstate, -2);
        }
        lua_tolstring(lstate, -1, len)
    }
}

pub fn nlua_error(lstate: *mut lua_State, msg: impl crate::strings::IntoCCharPtr) {
    let msg = msg.into_c_char_ptr();
    let mut len: usize = 0;
    let str_ = nlua_get_error(lstate, &mut len);
    msg_ext_no_fast();
    semsg_multiline!(c"lua_error", msg, len as i32, str_);
    lua_pop(lstate, 1);
}

unsafe extern "C-unwind" fn nlua_no_traceback(_lstate: *mut lua_State) -> i32 {
    1
}

pub fn nlua_pcall(lstate: *mut lua_State, nargs: i32, mut nresults: i32) -> i32 {
    unsafe {
        lua_getglobal(lstate, c"debug");
        if lua_istable(lstate, -1) {
            lua_getfield(lstate, -1, c"traceback");
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
}

pub fn nlua_luv_error_event(error: Option<std::ffi::CString>, err_type: isize) {
    let error_ptr = error.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    match err_type {
        K_CALLBACK => {
            msg_ext_no_fast();
            semsg_multiline!(c"lua_error", c"Lua callback:\n%s", error_ptr);
        }
        K_THREAD => {
            semsg_multiline!(c"lua_error", c"Luv thread:\n%s", error_ptr);
        }
        K_THREAD_CALLBACK => {
            semsg_multiline!(c"lua_error", c"Luv callback, thread:\n%s", error_ptr,);
        }
        _ => {}
    }
}

unsafe extern "C-unwind" fn nlua_fast_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    mut nresult: i32,
    flags: i32,
) -> i32 {
    unsafe {
        nlua_st().in_fast_callback += 1;

        let top = lua_gettop(lstate);
        let status = nlua_pcall(lstate, nargs, nresult);
        let retval = if status != 0 {
            if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
                preserve_exit(E_OUTOFMEM);
            }

            let mut len: usize = 0;
            let error = nlua_get_error(lstate, &mut len);
            let queues = loop_queues(&mut g_st().main_loop);
            put_event(
                queues.events,
                Event::NluaLuvError {
                    msg: (!error.is_null()).then(|| crate::memory::cstr_to_cstring(error)),
                    kind: K_CALLBACK,
                },
            );
            lua_pop(lstate, 1);
            -status
        } else {
            if nresult == LUA_MULTRET {
                nresult = { lua_gettop(lstate) } - top + nargs + 1;
            }
            nresult
        };

        nlua_st().in_fast_callback -= 1;
        retval
    }
}

unsafe extern "C-unwind" fn nlua_luv_thread_cb_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    nresult: i32,
    flags: i32,
) -> i32 {
    nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, true)
}

unsafe extern "C-unwind" fn nlua_luv_thread_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    nresult: i32,
    flags: i32,
) -> i32 {
    nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, false)
}

unsafe extern "C-unwind" fn nlua_luv_thread_cfcpcall(
    lstate: *mut lua_State,
    func: lua_CFunction,
    ud: *mut c_void,
    flags: i32,
) -> i32 {
    unsafe {
        lua_pushcfunction(lstate, func);
        lua_pushlightuserdata(lstate, ud);
        nlua_luv_thread_cfpcall(lstate, 1, 0, flags)
    }
}

fn nlua_luv_thread_common_cfpcall(
    lstate: *mut lua_State,
    nargs: i32,
    mut nresult: i32,
    flags: i32,
    is_callback: bool,
) -> i32 {
    let top = unsafe { lua_gettop(lstate) };
    let status = unsafe { lua_pcall(lstate, nargs, nresult, 0) };
    if status != 0 {
        if status == LUA_ERRMEM && (flags & LUVF_CALLBACK_NOEXIT) == 0 {
            fprintf!(
                log_get_stderr(),
                c"%s\n",
                (&raw const E_OUTOFMEM).cast::<c_char>(),
            );
            unsafe { lua_close(lstate) };
            #[cfg(windows)]
            {
                unsafe { ExitThread(0) };
            }
            #[cfg(not(windows))]
            {
                unsafe { pthread_exit(ptr::null_mut()) };
            }
        }
        let mut len: usize = 0;
        let error = nlua_get_error(lstate, &mut len);
        let ev = Event::NluaLuvError {
            msg: (!error.is_null()).then(|| {
                let slice = crate::memory::cstr_slice(error, len);
                let end = slice.iter().position(|&b| b == 0).unwrap_or(len);
                std::ffi::CString::new(&slice[..end]).unwrap()
            }),
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
            nresult = unsafe { lua_gettop(lstate) } - top + nargs + 1;
        }
        nresult
    }
}

unsafe extern "C-unwind" fn nlua_thr_api_nvim_get_runtime(lstate: *mut lua_State) -> i32 {
    unsafe {
        if lua_gettop(lstate) != 3 {
            return luaL_error(lstate, c"Expected 3 arguments".as_ptr());
        }

        luaL_checktype(lstate, -1, LUA_TTABLE);
        lua_getfield(lstate, -1, c"is_lua");
        if !lua_isboolean(lstate, -1) {
            return luaL_error(lstate, c"is_lua is not a boolean".as_ptr());
        }
        let is_lua = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 2);

        luaL_checktype(lstate, -1, LUA_TBOOLEAN);
        let all = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 1);

        let mut err = Error::new();
        let pat = nlua_pop_array_opt(lstate, None, &mut err);
        if error_set(&err) {
            luaL_where(lstate, 1);
            lua_pushstring(lstate, err.msg_ptr());
            api_clear_error(&mut err);
            lua_concat(lstate, 2);
            return lua_error(lstate);
        }

        let ret = runtime_get_named_thread(is_lua, pat, all);
        nlua_push_array(lstate, ret, K_NLUA_PUSH_SPECIAL);
        api_free_array(ret);
        api_free_array(pat);

        1
    }
}

fn nlua_init_argv(lstate: *mut lua_State, argv: *mut *mut c_char, argc: i32, lua_arg0: i32) -> i32 {
    let mut i: i32 = 0;
    lua_newtable(lstate);

    if lua_arg0 > 0 {
        lua_pushstring(lstate, unsafe {
            *argv.wrapping_add((lua_arg0 - 1) as usize)
        });
        unsafe { lua_rawseti(lstate, -2, 0) };

        while i + lua_arg0 < argc {
            lua_pushstring(lstate, unsafe {
                *argv.wrapping_add((i + lua_arg0) as usize)
            });
            unsafe { lua_rawseti(lstate, -2, i + 1) };
            i += 1;
        }
    }

    lua_setglobal(lstate, c"arg");
    i
}

pub fn nlua_schedule_event(cb: LuaRef, ns_id: u32) {
    let lstate = nlua_st().global_lstate;

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

unsafe extern "C-unwind" fn nlua_schedule(lstate: *mut lua_State) -> i32 {
    unsafe {
        if lua_type(lstate, 1) != LUA_TFUNCTION {
            lua_pushlstring(lstate, c"vim.schedule: expected function".as_ptr(), 31);
            return lua_error(lstate);
        }

        lua_pushnil(lstate);
        if *loop_closing_ptr(&mut g_st().main_loop) {
            lua_pushlstring(lstate, c"main loop is closing".as_ptr(), 20);
            return 2;
        }

        let cb = nlua_ref_global(lstate, 1);
        let queues = loop_queues(&mut g_st().main_loop);
        put_event(
            queues.events,
            Event::NluaSchedule {
                cb,
                ns_id: g_st().ui_event_ns_id,
            },
        );
        lua_pushnil(lstate);
        2
    }
}

unsafe extern "C-unwind" fn nlua_loop_poll(lstate: *mut lua_State) -> i32 {
    unsafe {
        let timeout = { luaL_checkinteger(lstate, 1) } as i64;
        let fast_only = { lua_toboolean(lstate, 2) } != 0;

        let queues = loop_queues(&mut g_st().main_loop);
        let loop_events = if fast_only {
            queues.fast_events
        } else {
            queues.events
        };
        if !loop_events.is_null() && !multiqueue_empty(&*loop_events) {
            multiqueue_process_events(&mut *loop_events);
        } else {
            loop_poll_events(&mut g_st().main_loop, timeout);
        }

        0
    }
}

unsafe extern "C-unwind" fn nlua_ui_flush(_lstate: *mut lua_State) -> i32 {
    ui_flush();
    0
}

unsafe extern "C-unwind" fn nlua_check_interrupt(lstate: *mut lua_State) -> i32 {
    unsafe {
        let interrupted = g_st().got_int;
        if g_st().got_int {
            g_st().got_int = false;
            vgetc();
        }
        lua_pushboolean(lstate, i32::from(interrupted));
        1
    }
}

pub fn nlua_ex_getline(
    _c: i32,
    ecd: &mut ExDocmdCookie,
    _indent: i32,
    _do_concat: bool,
) -> Option<std::ffi::CString> {
    let lstate = nlua_st().global_lstate;
    nlua_pushref(lstate, ecd.getline);
    if nlua_pcall(lstate, 0, 1) != 0 {
        nlua_error(
            lstate,
            gettext(c"Error executing Ex-mode line getter: %.*s"),
        );
        return None;
    }
    if unsafe { lua_type(lstate, -1) } != LUA_TSTRING {
        lua_pop(lstate, 1);
        return None;
    }
    let line = crate::memory::cstr_to_cstring(lua_tostring(lstate, -1));
    lua_pop(lstate, 1);
    ecd.got_line = true;
    Some(line)
}

unsafe extern "C-unwind" fn nlua_ex_docmd(lstate: *mut lua_State) -> i32 {
    unsafe {
        luaL_checktype(lstate, 1, LUA_TFUNCTION);
        let mut cookie = ExDocmdCookie {
            getline: nlua_ref_global(lstate, 1),
            got_line: false,
        };
        g_st().msg_scroll = 1;
        g_st().need_wait_return = false;
        g_st().no_wait_return += 1;
        do_cmdline(
            ptr::null_mut(),
            crate::cmdexpand::LineGetter::LuaEx(&raw mut cookie),
            0,
        );
        g_st().no_wait_return -= 1;
        nlua_unref_global(lstate, cookie.getline);
        lua_pushboolean(lstate, i32::from(cookie.got_line));
        1
    }
}

unsafe extern "C-unwind" fn nlua_os_exit(lstate: *mut lua_State) -> i32 {
    unsafe {
        if nlua_st().in_fast_callback > 0 {
            return luaL_error(lstate, E_FAST_API_DISABLED.as_ptr(), c"os.exit");
        }

        let mut status: i32 = 0;
        if lua_gettop(lstate) >= 1 && !lua_isnil(lstate, 1) {
            status = if lua_isboolean(lstate, 1) {
                if lua_toboolean(lstate, 1) != 0 { 0 } else { 1 }
            } else {
                luaL_checkinteger(lstate, 1) as i32
            };
        }
        getout(status);
    }
}

fn nlua_new_ref_state(lstate: *mut lua_State, is_thread: bool) -> *mut NluaRefState {
    unsafe {
        let ref_state = lua_newuserdata(lstate, size_of::<NluaRefState>()).cast::<NluaRefState>();
        *ref_state = NluaRefState {
            nil_ref: LUA_NOREF,
            empty_dict_ref: LUA_NOREF,
            ref_count: 0,
        };
        if !is_thread {
            g_st().nlua_global_refs = ref_state;
        }
        ref_state
    }
}

fn nlua_get_ref_state(lstate: *mut lua_State) -> &'static mut NluaRefState {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state");
        let ref_state = lua_touserdata(lstate, -1).cast::<NluaRefState>();
        lua_pop(lstate, 1);
        &mut *ref_state
    }
}

pub fn nlua_get_nil_ref(lstate: *mut lua_State) -> LuaRef {
    nlua_get_ref_state(lstate).nil_ref
}

pub fn nlua_get_empty_dict_ref(lstate: *mut lua_State) -> LuaRef {
    nlua_get_ref_state(lstate).empty_dict_ref
}

pub fn nlua_get_global_ref_count() -> i32 {
    crate::globals::nlua_global_refs_mut().ref_count
}

fn nlua_common_vim_init(lstate: *mut lua_State, is_thread: bool) {
    unsafe {
        let ref_state = nlua_new_ref_state(lstate, is_thread);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state");

        lua_pushboolean(lstate, i32::from(is_thread));
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
        lua_pushcfunction(lstate, Some(nlua_is_thread));
        lua_setfield(lstate, -2, c"is_thread");

        lua_createtable(lstate, 0, 0);
        lua_setfield(lstate, -2, c"_core");

        lua_newuserdata(lstate, 0);
        lua_createtable(lstate, 0, 0);
        lua_pushcfunction(lstate, Some(nlua_nil_tostring));
        lua_setfield(lstate, -2, c"__tostring");
        lua_pushcfunction(lstate, Some(nlua_nil_index));
        lua_setfield(lstate, -2, c"__index");
        lua_pushcfunction(lstate, Some(nlua_nil_index));
        lua_setfield(lstate, -2, c"__newindex");
        lua_setmetatable(lstate, -2);
        (*ref_state).nil_ref = nlua_ref(lstate, &mut *ref_state, -1);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.NIL");
        lua_setfield(lstate, -2, c"NIL");

        lua_createtable(lstate, 0, 0);
        lua_pushcfunction(lstate, Some(nlua_empty_dict_tostring));
        lua_setfield(lstate, -2, c"__tostring");
        (*ref_state).empty_dict_ref = nlua_ref(lstate, &mut *ref_state, -1);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, LUA_REGISTRYINDEX, c"mpack.empty_dict");
        lua_setfield(lstate, -2, c"_empty_dict_mt");

        if is_thread {
            luv_set_callback(lstate, Some(nlua_luv_thread_cb_cfpcall));
            luv_set_thread(lstate, Some(nlua_luv_thread_cfpcall));
            luv_set_cthread(lstate, Some(nlua_luv_thread_cfcpcall));
        } else {
            luv_set_loop(lstate, (&raw mut g_st().main_loop).cast());
            luv_set_callback(lstate, Some(nlua_fast_cfpcall));
        }
        luaopen_luv(lstate);
        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -3, c"uv");

        lua_pushvalue(lstate, -1);
        lua_setfield(lstate, -3, c"loop");

        lua_getglobal(lstate, c"package");
        lua_getfield(lstate, -1, c"loaded");
        lua_pushvalue(lstate, -3);
        lua_setfield(lstate, -2, c"luv");
        lua_pop(lstate, 3);
    }
}

unsafe extern "C-unwind" fn nlua_module_preloader(lstate: *mut lua_State) -> i32 {
    unsafe {
        let i = { lua_tointeger(lstate, lua_upvalueindex(1)) } as usize;
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

fn nlua_init_packages(lstate: *mut lua_State) -> bool {
    lua_getglobal(lstate, c"package");
    lua_getfield(lstate, -1, c"preload");
    for (i, def) in BUILTIN_MODULES.iter().enumerate() {
        unsafe { lua_pushinteger(lstate, i as lua_Integer) };
        unsafe { lua_pushcclosure(lstate, Some(nlua_module_preloader), 1) };
        lua_setfield(lstate, -2, def.name.as_ptr());

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

unsafe extern "C-unwind" fn nlua_ui_attach(lstate: *mut lua_State) -> i32 {
    let ns_id = unsafe { luaL_checkinteger(lstate, 1) } as u32;

    if !ns_initialized(ns_id) {
        return unsafe { luaL_error(lstate, c"invalid ns_id".as_ptr()) };
    }
    if !lua_istable(lstate, 2) {
        return unsafe { luaL_error(lstate, c"opts must be a table".as_ptr()) };
    }
    if !lua_isfunction(lstate, 3) {
        return unsafe { luaL_error(lstate, c"callback must be a Lua function".as_ptr()) };
    }

    let mut ext_widgets = [false; K_UIGLOBAL_COUNT];
    let mut tbl_has_true_val = false;

    unsafe { lua_pushvalue(lstate, 2) };
    unsafe { lua_pushnil(lstate) };
    while unsafe { lua_next(lstate, -2) } != 0 {
        let mut len: usize = 0;
        let s = unsafe { lua_tolstring(lstate, -2, &raw mut len) };
        let val = unsafe { lua_toboolean(lstate, -1) } != 0;

        let mut matched = false;
        if crate::memory::cstr_as_opt_bytes(s) == Some(b"set_cmdheight") {
            g_st().ui_refresh_cmdheight = val;
            matched = true;
        } else {
            for (i, slot) in ext_widgets.iter_mut().enumerate() {
                if crate::memory::cstr_as_opt_bytes(s)
                    == crate::memory::cstr_as_opt_bytes(UI_EXT_NAMES[i].as_ptr())
                {
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
            return unsafe { luaL_error(lstate, c"Unexpected key: %s".as_ptr(), s) };
        }
        lua_pop(lstate, 1);
    }

    if !tbl_has_true_val {
        return unsafe {
            luaL_error(
                lstate,
                c"opts table must contain at least one 'true' ext_widget".as_ptr(),
            )
        };
    }

    let ui_event_cb = nlua_ref_global(lstate, 3);
    ui_add_cb(ns_id, ui_event_cb, &ext_widgets);
    g_st().ui_refresh_cmdheight = true;
    0
}

unsafe extern "C-unwind" fn nlua_ui_detach(lstate: *mut lua_State) -> i32 {
    unsafe {
        let ns_id = luaL_checkinteger(lstate, 1) as u32;
        if !ns_initialized(ns_id) {
            return luaL_error(lstate, c"invalid ns_id".as_ptr());
        }
        ui_remove_cb(ns_id, false);
        0
    }
}

fn nlua_state_init(lstate: *mut lua_State) -> bool {
    lua_pushcfunction(lstate, Some(nlua_print));
    lua_setglobal(lstate, c"print");

    lua_getglobal(lstate, c"debug");
    lua_pushcfunction(lstate, Some(nlua_debug));
    lua_setfield(lstate, -2, c"debug");
    lua_pop(lstate, 1);

    #[cfg(windows)]
    {
        lua_getglobal(lstate, c"os");
        lua_pushcfunction(lstate, Some(nlua_getenv));
        lua_setfield(lstate, -2, c"getenv");
        lua_pop(lstate, 1);
    }

    lua_getglobal(lstate, c"os");
    lua_pushcfunction(lstate, Some(nlua_os_exit));
    lua_setfield(lstate, -2, c"exit");
    lua_pop(lstate, 1);

    lua_newtable(lstate);

    nlua_add_api_functions(lstate);
    nlua_init_types(lstate);

    lua_pushcfunction(lstate, Some(nlua_schedule));
    lua_setfield(lstate, -2, c"schedule");

    lua_pushcfunction(lstate, Some(nlua_in_fast_event));
    lua_setfield(lstate, -2, c"in_fast_event");

    lua_pushcfunction(lstate, Some(nlua_call));
    lua_setfield(lstate, -2, c"call");

    lua_pushcfunction(lstate, Some(nlua_rpcrequest));
    lua_setfield(lstate, -2, c"rpcrequest");

    lua_pushcfunction(lstate, Some(nlua_rpcnotify));
    lua_setfield(lstate, -2, c"rpcnotify");

    lua_pushcfunction(lstate, Some(nlua_ui_attach));
    lua_setfield(lstate, -2, c"ui_attach");

    lua_pushcfunction(lstate, Some(nlua_ui_detach));
    lua_setfield(lstate, -2, c"ui_detach");

    nlua_common_vim_init(lstate, false);

    lua_getfield(lstate, -1, c"_core");
    lua_pushcfunction(lstate, Some(nlua_loop_poll));
    lua_setfield(lstate, -2, c"loop_poll");
    lua_pushcfunction(lstate, Some(nlua_ui_flush));
    lua_setfield(lstate, -2, c"ui_flush");
    lua_pushcfunction(lstate, Some(nlua_check_interrupt));
    lua_setfield(lstate, -2, c"check_interrupt");

    lua_pushcfunction(lstate, Some(nlua_ex_docmd));
    lua_setfield(lstate, -2, c"ex_docmd");
    lua_pop(lstate, 1);

    if has_time_fd() {
        lua_getglobal(lstate, c"require");
        nlua_st().require_ref = nlua_ref_global(lstate, -1);
        lua_pop(lstate, 1);
        lua_pushcfunction(lstate, Some(nlua_require));
        lua_setglobal(lstate, c"require");
    }

    nlua_treesitter_init(lstate);
    nlua_state_add_stdlib(lstate, false);

    lua_setglobal(lstate, c"vim");

    nlua_init_packages(lstate)
}

pub fn nlua_init(argv: *mut *mut c_char, argc: i32, lua_arg0: i32) {
    let lstate = unsafe { luaL_newstate() };
    if lstate.is_null() {
        fprintf!(
            log_get_stderr(),
            gettext(c"E970: Failed to initialize Lua interpreter\n"),
        );
        os_exit(1);
    }
    unsafe { luaL_openlibs(lstate) };
    if !nlua_state_init(lstate) {
        fprintf!(
            log_get_stderr(),
            gettext(c"E970: Failed to initialize builtin Lua modules\n"),
        );
        if MAIN_EXITFREE {
            unsafe { nlua_common_free_all_mem(lstate) };
        }
        os_exit(1);
    }

    unsafe { luv_set_thread_cb(Some(nlua_thread_acquire_vm), Some(nlua_common_free_all_mem)) };
    nlua_st().global_lstate = lstate;
    nlua_st().active_lstate = lstate;
    nlua_st().main_thread = unsafe { uv_thread_self() };
    nlua_init_argv(lstate, argv, argc, lua_arg0);
}

unsafe extern "C-unwind" fn nlua_thread_acquire_vm() -> *mut lua_State {
    nlua_init_state(true)
}

fn nlua_init_state(thread: bool) -> *mut lua_State {
    let self_ = unsafe { uv_thread_self() };
    if unsafe { uv_thread_equal(&raw const nlua_st().main_thread, &raw const self_) } != 0 {
        runtime_search_path_validate();
    }

    let lstate = unsafe { luaL_newstate() };
    unsafe { luaL_openlibs(lstate) };

    lua_pushcfunction(lstate, Some(nlua_print));
    lua_setglobal(lstate, c"print");

    unsafe { lua_pushinteger(lstate, 0) };
    lua_setfield(lstate, LUA_REGISTRYINDEX, c"nlua.refcount");

    lua_newtable(lstate);

    nlua_common_vim_init(lstate, thread);
    nlua_state_add_stdlib(lstate, true);

    unsafe { lua_createtable(lstate, 0, 0) };
    lua_pushcfunction(lstate, Some(nlua_thr_api_nvim_get_runtime));
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
    if nlua_st().global_lstate.is_null() {
        return;
    }
    let lstate = nlua_st().global_lstate;
    nlua_st().global_lstate = ptr::null_mut();
    nlua_unref_global(lstate, nlua_st().require_ref);
    unsafe { nlua_common_free_all_mem(lstate) };
}

unsafe extern "C-unwind" fn nlua_common_free_all_mem(lstate: *mut lua_State) {
    unsafe {
        let ref_state = nlua_get_ref_state(lstate);
        nlua_unref(lstate, ref_state, ref_state.nil_ref);
        nlua_unref(lstate, ref_state, ref_state.empty_dict_ref);
        lua_close(lstate);
    }
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
        c"lua_print",
        true,
        false,
        None,
        &mut needs_clear,
        raw,
    );
}

unsafe extern "C-unwind" fn nlua_print(lstate: *mut lua_State) -> i32 {
    let nargs = unsafe { lua_gettop(lstate) };
    lua_getglobal(lstate, c"tostring");
    let mut errmsg_len: usize = 0;
    let mut msg_vec: Vec<c_char> = Vec::with_capacity(80);
    let mut curargidx: i32 = 1;
    while curargidx <= nargs {
        unsafe { lua_pushvalue(lstate, -1) };
        unsafe { lua_pushvalue(lstate, curargidx) };
        if unsafe { lua_pcall(lstate, 1, 1, 0) } != 0
            || unsafe { lua_type(lstate, -1) } != LUA_TSTRING
        {
            let errmsg = nlua_get_error(lstate, &mut errmsg_len);
            drop(msg_vec);
            let mut buff = [0 as c_char; IOSIZE];
            let fmt = gettext(c"E5114: Converting print argument #%i: %.*s");
            let len = vim_snprintf!(
                buff.as_mut_ptr(),
                IOSIZE,
                fmt,
                curargidx,
                errmsg_len as i32,
                errmsg,
            ) as usize;
            unsafe { lua_pushlstring(lstate, buff.as_ptr(), len) };
            return unsafe { lua_error(lstate) };
        }

        let mut len: usize = 0;
        let s = unsafe { lua_tolstring(lstate, -1, &raw mut len) };
        if !s.is_null() && len > 0 {
            msg_vec.extend_from_slice(unsafe { std::slice::from_raw_parts(s, len) });
        }
        if curargidx < nargs {
            msg_vec.push(b' ' as c_char);
        }
        lua_pop(lstate, 1);
        curargidx += 1;
    }
    let msg_str = crate::api::private::helpers::cbuf_to_string(msg_vec.as_ptr(), msg_vec.len());
    drop(msg_vec);

    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
    let is_thread = unsafe { lua_toboolean(lstate, -1) } != 0;
    lua_pop(lstate, 1);

    if is_thread {
        loop_schedule_deferred(&mut g_st().main_loop, Event::NluaPrint(msg_str));
    } else if nlua_st().in_fast_callback != 0 {
        let queues = loop_queues(&mut g_st().main_loop);
        put_event(queues.events, Event::NluaPrint(msg_str));
    } else {
        nlua_print_event(msg_str);
    }
    0
}

unsafe extern "C-unwind" fn nlua_require(lstate: *mut lua_State) -> i32 {
    let name = unsafe { luaL_checklstring(lstate, 1, ptr::null_mut()) };
    unsafe { lua_settop(lstate, 1) };

    lua_getfield(lstate, LUA_REGISTRYINDEX, c"_LOADED");
    lua_getfield(lstate, 2, name);
    if unsafe { lua_toboolean(lstate, -1) } != 0 {
        return 1;
    }
    lua_pop(lstate, 2);

    nlua_pushref(lstate, nlua_st().require_ref);
    unsafe { lua_insert(lstate, 1) };

    if !has_time_fd() {
        lua_getglobal(lstate, c"require");
        if unsafe { lua_iscfunction(lstate, -1) } != 0
            && unsafe { lua_tocfunction(lstate, -1) }.is_some_and(|f| {
                std::ptr::fn_addr_eq(
                    f,
                    nlua_require as unsafe extern "C-unwind" fn(*mut lua_State) -> i32,
                )
            })
        {
            unsafe { lua_pushvalue(lstate, 1) };
            lua_setglobal(lstate, c"require");
        }
        lua_pop(lstate, 1);
        unsafe { lua_call(lstate, 1, 1) };
        return 1;
    }

    let mut rel_time: Proftime = 0;
    let mut start_time: Proftime = 0;
    time_push(&mut rel_time, &mut start_time);
    let status = unsafe { lua_pcall(lstate, 1, 1, 0) };
    if status == 0 {
        vim_snprintf!(g_st().iobuff.as_mut_ptr(), IOSIZE, c"require('%s')", name,);
        time_msg(g_st().iobuff.as_ptr(), Some(&start_time));
    }
    time_pop(rel_time);

    if status == 0 {
        1
    } else {
        unsafe { lua_error(lstate) }
    }
}

unsafe extern "C-unwind" fn nlua_debug(lstate: *mut lua_State) -> i32 {
    unsafe {
        let input_args: [TypVal; 2] = [
            TypVal {
                v_type: VarType::VarString,
                v_lock: VAR_FIXED,
                vval: TypvalVval::String(c"lua_debug> ".as_ptr().cast_mut()),
            },
            TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];
        loop {
            lua_settop(lstate, 0);
            let mut input: TypVal = TypVal::new();
            get_user_input(&input_args, &mut input, false, false);

            if ui_has(K_UICMDLINE) {
                vim_snprintf!(
                    g_st().iobuff.as_mut_ptr(),
                    IOSIZE,
                    c"lua_debug> %s",
                    input.vval.v_string(),
                );
                ui_ext_cmdline_block_append(0, g_st().iobuff.as_ptr());
            } else {
                msg_putchar(b'\n' as i32);
            }

            if input.v_type != VarType::VarString
                || input.vval.v_string().is_null()
                || crate::memory::cstr_is_empty(input.vval.v_string())
                || crate::memory::cstr_as_bytes(input.vval.v_string()) == b"cont"
            {
                tv_clear(&mut input);
                if ui_has(K_UICMDLINE) {
                    ui_ext_cmdline_block_leave();
                }
                return 0;
            }

            let len = crate::memory::cstr_len(input.vval.v_string());
            if luaL_loadbuffer(lstate, input.vval.v_string(), len, c"=(debug command)") != 0 {
                nlua_error(lstate, gettext(c"E5115: Loading Lua debug string: %.*s"));
            } else if nlua_pcall(lstate, 0, 0) != 0 {
                nlua_error(lstate, gettext(c"E5116: Calling Lua debug string: %.*s"));
            }
            tv_clear(&mut input);
        }
    }
}

pub unsafe extern "C-unwind" fn nlua_in_fast_event(lstate: *mut lua_State) -> i32 {
    unsafe {
        lua_pushboolean(lstate, i32::from(nlua_st().in_fast_callback > 0));
        1
    }
}

fn viml_func_is_fast(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let fdef: *const EvalFuncDef = find_internal_func(name);
        !fdef.is_null() && (*fdef).fast
    }
}

pub unsafe extern "C-unwind" fn nlua_call(lstate: *mut lua_State) -> i32 {
    let mut err = Error::new();
    let mut name_len: usize = 0;
    let name = unsafe { luaL_checklstring(lstate, 1, &raw mut name_len) };
    if !nlua_is_deferred_safe() && !viml_func_is_fast(name) {
        let length = name_len.min(100) + 22;
        vim_snprintf!(
            g_st().iobuff.as_mut_ptr(),
            length,
            c"Vimscript function \"%s\"",
            name,
        );
        return unsafe { luaL_error(lstate, E_FAST_API_DISABLED.as_ptr(), g_st().iobuff.as_ptr()) };
    }

    let nargs = unsafe { lua_gettop(lstate) } - 1;
    if nargs > MAX_FUNC_ARGS {
        return unsafe { luaL_error(lstate, c"Function called with too many arguments".as_ptr()) };
    }

    let func_lua = find_internal_func_lua(name);
    if !func_lua.is_null() {
        lua_getglobal(lstate, c"require");
        unsafe { lua_pushlstring(lstate, c"vim._core.vimfn".as_ptr(), 15) };
        if unsafe { lua_pcall(lstate, 1, 1, 0) } != 0 {
            return unsafe { lua_error(lstate) };
        }
        lua_getfield(lstate, -1, func_lua);
        unsafe { lua_remove(lstate, -2) };
        for j in 0..nargs {
            unsafe { lua_pushvalue(lstate, j + 2) };
        }
        if unsafe { lua_pcall(lstate, nargs, 1, 0) } != 0 {
            return unsafe { lua_error(lstate) };
        }
        return 1;
    }

    let mut vim_args: [TypVal; (MAX_FUNC_ARGS + 1) as usize] =
        [TypVal::new(); (MAX_FUNC_ARGS + 1) as usize];
    let mut i: usize = 0;
    let mut conv_failed = false;
    while (i as i32) < nargs {
        unsafe { lua_pushvalue(lstate, (i as i32) + 2) };
        if !nlua_pop_typval(lstate, &raw mut vim_args[i]) {
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
            name,
            name_len as i32,
            &mut rettv,
            nargs,
            vim_args.as_mut_ptr(),
            &mut funcexe,
        );
        try_leave(&tstate, &mut err);
        g_st().current_sctx = save_current_sctx;

        if !error_set(&err) {
            nlua_push_typval(lstate, &raw mut rettv, 0);
        }
        tv_clear(&mut rettv);
    }

    while i > 0 {
        i -= 1;
        tv_clear(&mut vim_args[i]);
    }
    if error_set(&err) {
        lua_pushstring(lstate, err.msg_ptr());
        api_clear_error(&mut err);
        return unsafe { lua_error(lstate) };
    }
    1
}

unsafe extern "C-unwind" fn nlua_rpcrequest(lstate: *mut lua_State) -> i32 {
    unsafe {
        if !nlua_is_deferred_safe() {
            return luaL_error(lstate, E_FAST_API_DISABLED.as_ptr(), c"rpcrequest");
        }
        nlua_rpc(lstate, true)
    }
}

unsafe extern "C-unwind" fn nlua_rpcnotify(lstate: *mut lua_State) -> i32 {
    nlua_rpc(lstate, false)
}

fn nlua_rpc(lstate: *mut lua_State, request: bool) -> i32 {
    let mut name_len: usize = 0;
    let chan_id = unsafe { luaL_checkinteger(lstate, 1) } as u64;
    let name = unsafe { luaL_checklstring(lstate, 2, &raw mut name_len) };
    let nargs = unsafe { lua_gettop(lstate) } - 2;
    let mut err = Error::new();
    let mut arena = Arena {
        cur_blk: ptr::null_mut(),
        pos: 0,
        size: 0,
    };

    let mut args = arena_array(&mut arena, nargs.max(0) as usize);
    for i in 0..nargs {
        unsafe { lua_pushvalue(lstate, i + 3) };
        let obj = nlua_pop_object(lstate, false, &mut arena, &mut err);
        *args.next_slot_mut() = obj;
        args.size += 1;
        if error_set(&err) {
            break;
        }
    }

    if !error_set(&err) {
        if request {
            let mut res_mem: ArenaMem = ptr::null_mut();
            let mut result = rpc_send_call(chan_id, name, args, &mut res_mem, &mut err);
            if !error_set(&err) {
                nlua_push_object(lstate, &raw mut result, 0);
                arena_mem_free(res_mem);
            }
        } else if !rpc_send_event(chan_id, name, args) {
            let mut mbuf = [0u8; 64];
            let _ = write!(&mut mbuf[..], "Invalid channel: {chan_id}\0");
            api_set_error!(
                &mut err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                mbuf.as_ptr().cast::<c_char>(),
            );
        }
    }

    arena_mem_free(arena_finish(&mut arena));

    if error_set(&err) {
        lua_pushstring(lstate, err.msg_ptr());
        api_clear_error(&mut err);
        return unsafe { lua_error(lstate) };
    }

    if request { 1 } else { 0 }
}

unsafe extern "C-unwind" fn nlua_nil_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"vim.NIL");
    1
}

unsafe extern "C-unwind" fn nlua_nil_index(lstate: *mut lua_State) -> i32 {
    unsafe { luaL_error(lstate, c"attempt to index vim.NIL".as_ptr()) }
}

unsafe extern "C-unwind" fn nlua_empty_dict_tostring(lstate: *mut lua_State) -> i32 {
    lua_pushstring(lstate, c"vim.empty_dict()");
    1
}

#[cfg(windows)]
unsafe extern "C-unwind" fn nlua_getenv(lstate: *mut lua_State) -> i32 {
    unsafe {
        let name = luaL_checklstring(lstate, 1, ptr::null_mut());
        let val = os_getenv_cstr(name);
        lua_pushstring(lstate, val.as_ref().map_or(ptr::null(), |c| c.as_ptr()));
        1
    }
}

pub fn nlua_ref(lstate: *mut lua_State, ref_state: &mut NluaRefState, index: i32) -> LuaRef {
    unsafe {
        lua_pushvalue(lstate, index);
        let ref_ = luaL_ref(lstate, LUA_REGISTRYINDEX);
        if ref_ > 0 {
            ref_state.ref_count += 1;
        }
        ref_
    }
}

pub fn nlua_ref_global(lstate: *mut lua_State, index: i32) -> LuaRef {
    unsafe { nlua_ref(lstate, &mut *g_st().nlua_global_refs, index) }
}

pub fn nlua_unref(lstate: *mut lua_State, ref_state: &mut NluaRefState, ref_: LuaRef) {
    if ref_ > 0 {
        ref_state.ref_count -= 1;
        unsafe { luaL_unref(lstate, LUA_REGISTRYINDEX, ref_) };
    }
}

pub fn nlua_unref_global(lstate: *mut lua_State, ref_: LuaRef) {
    unsafe {
        nlua_unref(lstate, &mut *g_st().nlua_global_refs, ref_);
    }
}

pub fn api_free_luaref(ref_: LuaRef) {
    nlua_unref_global(nlua_st().global_lstate, ref_);
}

pub fn nlua_pushref(lstate: *mut lua_State, ref_: LuaRef) {
    unsafe {
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, ref_);
    }
}

pub fn api_new_luaref(original_ref: LuaRef) -> LuaRef {
    if original_ref == LUA_NOREF {
        return LUA_NOREF;
    }
    let lstate = nlua_st().global_lstate;
    nlua_pushref(lstate, original_ref);
    let new_ref = nlua_ref_global(lstate, -1);
    lua_pop(lstate, 1);
    new_ref
}

fn nlua_exec_typval_fmt(
    fmt: &std::ffi::CStr,
    body: impl crate::strings::IntoCCharPtr,
    bodylen: usize,
    name: &std::ffi::CStr,
    args: *mut TypVal,
    argcount: i32,
    special: bool,
    ret_tv: &mut TypVal,
) {
    let body = body.into_c_char_ptr();
    let mut lcmd_buf: [c_char; IOSIZE] = [0; IOSIZE];
    let buf_ptr = lcmd_buf.as_mut_ptr();
    let mut lcmd = buf_ptr;
    let mut lcmd_vec: Vec<c_char> = Vec::new();
    let lcmd_len = vim_snprintf!(buf_ptr, IOSIZE, fmt.as_ptr(), bodylen as i32, body);
    if lcmd_len as usize >= IOSIZE {
        lcmd_vec.resize(lcmd_len as usize + 1, 0);
        lcmd = lcmd_vec.as_mut_ptr();
        vim_snprintf!(
            lcmd,
            lcmd_len as usize + 1,
            fmt.as_ptr(),
            bodylen as i32,
            body
        );
    }

    nlua_exec_typval(
        lcmd,
        lcmd_len as usize,
        name.as_ptr(),
        args,
        argcount,
        special,
        Some(&mut *ret_tv),
    );
}

pub fn nlua_call_luaeval(str_: NvimString, arg: &mut TypVal, ret_tv: &mut TypVal) {
    nlua_exec_typval_fmt(
        c"local _A=select(1,...) return (%.*s)",
        str_.data,
        str_.size,
        c"luaeval()",
        arg,
        1,
        true,
        &mut *ret_tv,
    );
}

pub fn nlua_call_vlua(
    str_: impl crate::strings::IntoCCharPtr,
    len: usize,
    args: *mut TypVal,
    argcount: i32,
    ret_tv: &mut TypVal,
) {
    let str_ = str_.into_c_char_ptr();
    nlua_exec_typval_fmt(
        c"return %.*s(...)",
        str_,
        len,
        c"v:lua",
        args,
        argcount,
        false,
        ret_tv,
    );
}

pub fn nlua_call_user_expand_func(xp: &mut ExpandCtx, ret_tv: &mut TypVal) {
    let lstate = nlua_st().global_lstate;

    nlua_pushref(lstate, xp.xp_luaref);
    lua_pushstring(lstate, xp.xp_pattern);
    lua_pushstring(lstate, xp.xp_line);
    unsafe { lua_pushinteger(lstate, xp.xp_col as lua_Integer) };

    if nlua_pcall(lstate, 3, 1) != 0 {
        nlua_error(lstate, gettext(c"E5108: Lua function: %.*s"));
        return;
    }

    nlua_pop_typval(lstate, ret_tv);
}

fn nlua_exec_typval(
    lcmd: impl crate::strings::IntoCCharPtr,
    lcmd_len: usize,
    name: impl crate::strings::IntoCCharPtr,
    args: *mut TypVal,
    argcount: i32,
    special: bool,
    mut ret_tv: Option<&mut TypVal>,
) {
    let lcmd = lcmd.into_c_char_ptr();
    let name = name.into_c_char_ptr();
    {
        if check_secure() {
            if ret_tv.is_some() {
                ret_tv.as_deref_mut().unwrap().v_type = VarType::VarNumber;
                ret_tv.as_deref_mut().unwrap().vval = crate::eval::typval::TypvalVval::Number(0);
            }
            return;
        }

        let lstate = nlua_st().global_lstate;
        if luaL_loadbuffer(lstate, lcmd, lcmd_len, name) != 0 {
            nlua_error(lstate, gettext(c"E5107: Lua: %.*s"));
            return;
        }

        push_all_typvals(lstate, args, argcount, special);

        if nlua_pcall(lstate, argcount, if ret_tv.is_some() { 1 } else { 0 }) != 0 {
            nlua_error(lstate, gettext(c"E5108: Lua: %.*s"));
            return;
        }

        if let Some(ret_tv) = ret_tv {
            nlua_pop_typval(lstate, &raw mut *ret_tv);
        }
    }
}

pub fn nlua_exec_lines(lines: &[std::ffi::CString], name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    let mut code: Vec<u8> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            code.push(b'\n');
        }
        code.extend_from_slice(line.to_bytes());
    }
    let len = code.len();
    code.push(0);
    nlua_exec_typval(
        code.as_ptr().cast(),
        len,
        name,
        ptr::null_mut(),
        0,
        false,
        None,
    );
}

pub fn nlua_exec_typval_callable(
    lua_cb: LuaRef,
    argcount: i32,
    argvars: *mut TypVal,
    rettv: &mut TypVal,
) -> i32 {
    let lstate = nlua_st().global_lstate;
    nlua_pushref(lstate, lua_cb);
    push_all_typvals(lstate, argvars, argcount, false);

    if nlua_pcall(lstate, argcount, 1) != 0 {
        nlua_error(lstate, gettext(c"Lua callback: %.*s"));
        return FCERR_OTHER;
    }

    nlua_pop_typval(lstate, rettv);
    FCERR_NONE
}

pub fn nlua_exec(
    str_: NvimString,
    chunkname: impl crate::strings::IntoCCharPtr,
    args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: &mut Error,
) -> Object {
    let chunkname = chunkname.into_c_char_ptr();
    {
        let lstate = nlua_st().global_lstate;
        let top = unsafe { lua_gettop(lstate) };
        let name = if !crate::memory::cstr_is_empty(chunkname) {
            chunkname
        } else {
            c"<nvim>".as_ptr()
        };
        if luaL_loadbuffer(lstate, str_.data, str_.size, name) != 0 {
            let mut len: usize = 0;
            let errstr = unsafe { lua_tolstring(lstate, -1, &raw mut len) };
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Lua: %.*s",
                len as i32,
                errstr,
            );
            return OBJECT_NIL;
        }

        for i in 0..args.size {
            nlua_push_object(lstate, args.items.wrapping_add(i), 0);
        }

        if nlua_pcall(lstate, args.size as i32, 1) != 0 {
            let mut len: usize = 0;
            let errstr = unsafe { lua_tolstring(lstate, -1, &raw mut len) };
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Lua: %.*s",
                len as i32,
                errstr,
            );
            return OBJECT_NIL;
        }

        nlua_call_pop_retval(lstate, mode, arena, top, Some(err))
    }
}

pub fn nlua_call_typval<S: crate::strings::IntoCCharPtr>(
    module: &std::ffi::CStr,
    func: S,
    argvars: *mut TypVal,
    rettv: Option<&mut TypVal>,
) {
    let func = func.into_c_char_ptr();
    {
        let mut argcount: i32 = 0;
        let mut tv = argvars;
        while unsafe { (*tv).v_type } != VarType::VarUnknown {
            argcount += 1;
            tv = tv.wrapping_add(1);
        }

        let mut buf = [0 as c_char; 256];
        let len = vim_snprintf!(
            buf.as_mut_ptr(),
            buf.len(),
            c"return require('%s').%s(...)",
            module.as_ptr(),
            func,
        ) as usize;
        nlua_exec_typval(
            buf.as_ptr(),
            len,
            module.as_ptr(),
            argvars,
            argcount,
            false,
            (rettv.is_some()).then(|| rettv.unwrap()),
        );
    }
}

pub fn nlua_call_excmd(
    module: &std::ffi::CStr,
    func: &std::ffi::CStr,
    eap: &Exarg,
    cmod: &Cmdmod,
    extra: Option<&mut TypVal>,
) -> bool {
    let lstate = nlua_st().global_lstate;
    lua_getglobal(lstate, c"require");
    lua_pushstring(lstate, module.as_ptr());
    if unsafe { lua_pcall(lstate, 1, 1, 0) } != 0 {
        nlua_error(lstate, c"E5108: %s");
        return false;
    }
    lua_getfield(lstate, -1, func.as_ptr());
    unsafe { lua_remove(lstate, -2) };

    lua_newtable(lstate);
    nlua_push_eap(lstate, eap, cmod);

    let mut nargs: i32 = 1;
    if let Some(extra) = extra {
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

pub fn nlua_ref_is_function(ref_: LuaRef) -> bool {
    let lstate = nlua_st().global_lstate;
    nlua_pushref(lstate, ref_);
    let is_func = unsafe { lua_type(lstate, -1) } == LUA_TFUNCTION;
    lua_pop(lstate, 1);
    is_func
}

pub fn nlua_call_ref(
    ref_: LuaRef,
    name: impl crate::strings::IntoCCharPtr,
    args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: Option<&mut Error>,
) -> Object {
    let name = name.into_c_char_ptr();
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
    name: impl crate::strings::IntoCCharPtr,
    args: Array,
    mode: LuaRetMode,
    arena: Option<&mut Arena>,
    err: Option<&mut Error>,
) -> Object {
    let name = name.into_c_char_ptr();
    {
        let lstate = nlua_st().active_lstate;
        let top = unsafe { lua_gettop(lstate) };
        nlua_pushref(lstate, ref_);
        let mut nargs = args.size as i32;
        if !name.is_null() {
            lua_pushstring(lstate, name);
            nargs += 1;
        }
        for i in 0..args.size {
            nlua_push_object(lstate, args.items.wrapping_add(i), 0);
        }

        if fast {
            if unsafe { nlua_fast_cfpcall(lstate, nargs, mode_ret(mode), -1) } < 0 {
                if let Some(err) = err {
                    api_set_error!(err, ErrorType::ErrorTypeException, c"fast context failure",);
                }
                return OBJECT_NIL;
            }
        } else if nlua_pcall(lstate, nargs, mode_ret(mode)) != 0 {
            if let Some(err) = err {
                let mut len: usize = 0;
                let errstr = unsafe { lua_tolstring(lstate, -1, &raw mut len) };
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Lua: %.*s",
                    len as i32,
                    errstr,
                );
            } else {
                nlua_error(lstate, gettext(c"Lua callback: %.*s"));
            }
            return OBJECT_NIL;
        }

        nlua_call_pop_retval(lstate, mode, arena, top, err)
    }
}

fn nlua_call_pop_retval(
    lstate: *mut lua_State,
    mode: LuaRetMode,
    mut arena: Option<&mut Arena>,
    pretop: i32,
    err: Option<&mut Error>,
) -> Object {
    unsafe {
        if mode != K_RET_MULTI && mode != K_RET_MULTI_STACK && lua_isnil(lstate, -1) {
            lua_pop(lstate, 1);
            return OBJECT_NIL;
        }
        let mut dummy = Error::new();
        let perr = err.unwrap_or(&mut dummy);

        match mode {
            K_RET_NIL_BOOL => {
                let bool_value = lua_toboolean(lstate, -1) != 0;
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
                let nres = lua_gettop(lstate) - pretop;
                let mut res = crate::api::private::helpers::arena_array_opt(
                    arena.as_deref_mut(),
                    nres.max(0) as usize,
                );
                for i in 0..(nres as usize) {
                    *res.items.wrapping_add((nres as usize) - i - 1) =
                        nlua_pop_object_opt(lstate, false, arena.as_deref_mut(), perr);
                    if error_set(perr) {
                        return OBJECT_NIL;
                    }
                }
                res.size = nres.max(0) as usize;
                Object {
                    obj_type: ObjectType::ObjectTypeArray,
                    data: ObjectData::Array(res),
                }
            }
            K_RET_MULTI_STACK => OBJECT_NIL,
            _ => unreachable!(),
        }
    }
}

pub fn nlua_is_deferred_safe() -> bool {
    nlua_st().in_fast_callback == 0
}

pub fn ex_lua(eap: &mut Exarg) {
    if crate::memory::cstr_is_empty(eap.arg) {
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
    if eap.skip != 0 {
        return;
    }

    let code_cstr = if eap.cmdidx == CMD_EQUAL
        || crate::memory::cstr_first_byte(raw_cstr.as_ptr()) == b'='
    {
        let off: usize = if eap.cmdidx == CMD_EQUAL { 0 } else { 1 };
        let body = unsafe {
            std::slice::from_raw_parts(raw_cstr.as_ptr().wrapping_add(off).cast::<u8>(), len - off)
        };
        let mut buf = Vec::with_capacity(18 + body.len());
        buf.extend_from_slice(b"vim._print(true, ");
        buf.extend_from_slice(body);
        buf.push(b')');
        len = buf.len();
        std::ffi::CString::new(buf).unwrap()
    } else {
        raw_cstr
    };

    nlua_exec_typval(
        code_cstr.as_ptr(),
        len,
        c":lua",
        ptr::null_mut(),
        0,
        false,
        None,
    );
}

pub fn ex_luado(eap: &mut Exarg) {
    if !u_save(eap.line1 - 1, eap.line2 + 1) {
        emsg(gettext(c"cannot save undo information"));
        return;
    }
    let cmd = eap.arg;
    let cmd_len = crate::memory::cstr_len(cmd);
    let lstate = nlua_st().global_lstate;

    const DOSTART: &[u8] = b"return function(line, linenr) ";
    const DOEND: &[u8] = b" end";
    let lcmd_len = cmd_len + DOSTART.len() + DOEND.len();
    let mut lcmd_buf: [c_char; IOSIZE] = [0; IOSIZE];
    let buf_ptr = lcmd_buf.as_mut_ptr();
    let mut lcmd_vec: Vec<c_char> = Vec::new();
    let lcmd = if lcmd_len < IOSIZE {
        buf_ptr
    } else {
        lcmd_vec.resize(lcmd_len + 1, 0);
        lcmd_vec.as_mut_ptr()
    };
    unsafe { ptr::copy_nonoverlapping(DOSTART.as_ptr(), lcmd.cast::<u8>(), DOSTART.len()) };
    unsafe {
        ptr::copy_nonoverlapping(
            cmd.cast::<u8>(),
            lcmd.cast::<u8>().wrapping_add(DOSTART.len()),
            cmd_len,
        )
    };
    unsafe {
        ptr::copy_nonoverlapping(
            DOEND.as_ptr(),
            lcmd.cast::<u8>().wrapping_add(DOSTART.len() + cmd_len),
            DOEND.len(),
        )
    };

    if luaL_loadbuffer(lstate, lcmd, lcmd_len, c":luado") != 0 {
        nlua_error(lstate, gettext(c"E5109: Lua: %.*s"));
        return;
    }
    drop(lcmd_vec);
    if nlua_pcall(lstate, 0, 1) != 0 {
        nlua_error(lstate, gettext(c"E5110: Lua: %.*s"));
        return;
    }

    let was_curbuf = g_st().curbuf;
    for l in eap.line1..=eap.line2 {
        if l > curbuf_mut().b_ml.ml_line_count {
            break;
        }

        unsafe { lua_pushvalue(lstate, -1) };
        let old_line = ml_get_buf(crate::globals::curbuf_mut(), l);
        let old_line_len: Colnr = ml_get_buf_len(crate::globals::curbuf_mut(), l);
        lua_pushstring(lstate, old_line);
        unsafe { lua_pushnumber(lstate, f64::from(l)) };
        if nlua_pcall(lstate, 2, 1) != 0 {
            nlua_error(lstate, gettext(c"E5111: Lua: %.*s"));
            break;
        }

        if !ptr::eq(g_st().curbuf, was_curbuf) || l > curbuf_mut().b_ml.ml_line_count {
            break;
        }

        if unsafe { lua_isstring(lstate, -1) } != 0 {
            let mut new_line_len: usize = 0;
            let new_line = unsafe { lua_tolstring(lstate, -1, &raw mut new_line_len) };
            let mut new_line_vec = vec![0 as c_char; new_line_len + 1];
            if new_line_len > 0 {
                unsafe {
                    ptr::copy_nonoverlapping(new_line, new_line_vec.as_mut_ptr(), new_line_len)
                };
            }
            for b in &mut new_line_vec[..new_line_len] {
                if *b == NUL {
                    *b = b'\n' as c_char;
                }
            }
            crate::memline::ml_replace_len(l, new_line_vec.as_mut_ptr(), new_line_len, true);
            inserted_bytes(l, 0, old_line_len, new_line_len as i32);
        }
        lua_pop(lstate, 1);
    }

    lua_pop(lstate, 1);
    check_cursor(curwin_mut());
    redraw_curbuf_later(UPD_NOT_VALID);
}

pub fn ex_luafile(eap: &mut Exarg) {
    let _ = nlua_exec_file(eap.arg);
}

pub fn nlua_exec_file(path: impl crate::strings::IntoCCharPtr) -> bool {
    let path = path.into_c_char_ptr();
    {
        let lstate = nlua_st().global_lstate;
        if !(crate::memory::cstr_as_opt_bytes(path) == Some(b"-")) {
            lua_getglobal(lstate, c"loadfile");
            lua_pushstring(lstate, path);
        } else {
            let mut stdin_dup = FileDescriptor::new();
            if file_open_stdin(&mut stdin_dup) != 0 {
                return false;
            }

            let mut sb: Vec<u8> = Vec::with_capacity(64);
            loop {
                if g_st().got_int {
                    file_close(&mut stdin_dup, false);
                    return false;
                }
                let read_size = file_read(&raw mut stdin_dup, g_st().iobuff.as_mut_ptr(), 64);
                if read_size < 0 {
                    file_close(&mut stdin_dup, false);
                    return false;
                }
                if read_size > 0 {
                    let slice =
                        crate::memory::cstr_slice(g_st().iobuff.as_ptr(), read_size as usize);
                    sb.extend_from_slice(slice);
                }
                if read_size < 64 {
                    break;
                }
            }
            sb.push(0);
            file_close(&mut stdin_dup, false);

            lua_getglobal(lstate, c"loadstring");
            lua_pushstring(lstate, sb.as_ptr().cast());
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
}

pub fn nlua_expand_pat(xp: &mut ExpandCtx) {
    let lstate = nlua_st().global_lstate;
    let mut status = FAIL;

    lua_getglobal(lstate, c"vim");
    lua_getfield(lstate, -1, c"_expand_pat");
    unsafe { luaL_checktype(lstate, -1, LUA_TFUNCTION) };

    let pat = xp.xp_pattern;
    let line_end = xp.xp_line.wrapping_add(xp.xp_col as usize);
    assert!(line_end >= pat);
    let patlen = crate::memory::ptr_offset_from(line_end, pat);
    unsafe { lua_pushlstring(lstate, pat, patlen as usize) };

    if nlua_pcall(lstate, 1, 2) != 0 {
        nlua_error(lstate, gettext(c"vim._expand_pat: %.*s"));
        return;
    }

    let mut err = Error::new();
    let mut arena = Arena {
        cur_blk: ptr::null_mut(),
        pos: 0,
        size: 0,
    };
    let prefix_len = nlua_pop_integer(lstate, &mut arena, &mut err) as isize;
    if !error_set(&err) && prefix_len <= patlen {
        let completions = nlua_pop_array(lstate, &mut arena, &mut err);
        if !error_set(&err) {
            nlua_st().expand_result_array = Vec::with_capacity(completions.size);

            let mut all_strings = true;
            for i in 0..completions.size {
                let v = completions[i];
                if v.obj_type != ObjectType::ObjectTypeString {
                    all_strings = false;
                    break;
                }
                nlua_st()
                    .expand_result_array
                    .push(crate::memory::cstr_from_bytes(crate::memory::cstr_slice(
                        v.data.string().data,
                        v.data.string().size,
                    )));
            }

            if all_strings {
                xp.xp_pattern = xp.xp_pattern.wrapping_offset(prefix_len);
                status = OK;
            }
        }
        arena_mem_free(arena_finish(&mut arena));
    }

    if status == FAIL {
        nlua_st().expand_result_array.clear();
    }
}

pub fn nlua_expand_get_matches(results: &mut Vec<std::ffi::CString>) -> bool {
    *results = std::mem::take(&mut nlua_st().expand_result_array);
    !results.is_empty()
}

unsafe extern "C-unwind" fn nlua_is_thread(lstate: *mut lua_State) -> i32 {
    lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread");
    1
}

pub fn nlua_is_table_from_lua(arg: &TypVal) -> bool {
    unsafe {
        if arg.v_type == VarType::VarDict {
            (*arg.vval.v_dict()).lua_table_ref != LUA_NOREF
        } else if arg.v_type == VarType::VarList {
            (*arg.vval.v_list()).lua_table_ref != LUA_NOREF
        } else {
            false
        }
    }
}

pub fn nlua_register_table_as_callable(arg: &TypVal) -> *mut c_char {
    let table_ref = if arg.v_type == VarType::VarDict {
        unsafe { (*arg.vval.v_dict()).lua_table_ref }
    } else if arg.v_type == VarType::VarList {
        unsafe { (*arg.vval.v_list()).lua_table_ref }
    } else {
        LUA_NOREF
    };

    if table_ref == LUA_NOREF {
        return ptr::null_mut();
    }

    let lstate = nlua_st().global_lstate;
    let top = unsafe { lua_gettop(lstate) };

    nlua_pushref(lstate, table_ref);
    if unsafe { lua_getmetatable(lstate, -1) } == 0 {
        lua_pop(lstate, 1);
        debug_assert_eq!(top, unsafe { lua_gettop(lstate) });
        return ptr::null_mut();
    }

    lua_getfield(lstate, -1, c"__call");
    if !lua_isfunction(lstate, -1) {
        lua_pop(lstate, 3);
        debug_assert_eq!(top, unsafe { lua_gettop(lstate) });
        return ptr::null_mut();
    }
    lua_pop(lstate, 2);

    let func = nlua_ref_global(lstate, -1);
    let name = register_luafunc(func);
    lua_pop(lstate, 1);
    debug_assert_eq!(top, unsafe { lua_gettop(lstate) });

    name
}

pub fn nlua_exec_on_key(c: i32, typed_buf: *mut c_char) -> bool {
    static RECURSIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RECURSIVE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }

    let mut buf = [0 as c_char; MB_MAXBYTES * 3 + 4];
    let buf_len = special_to_buf(c, g_st().mod_mask, false, buf.as_mut_ptr());
    vim_unescape_ks(typed_buf);

    let lstate = nlua_st().global_lstate;
    let top = unsafe { lua_gettop(lstate) };

    lua_getglobal(lstate, c"vim");
    lua_getfield(lstate, -1, c"_on_key");
    unsafe { luaL_checktype(lstate, -1, LUA_TFUNCTION) };

    unsafe { lua_pushlstring(lstate, buf.as_ptr(), buf_len as usize) };
    lua_pushstring(lstate, typed_buf);

    let save_got_int = g_st().got_int;
    g_st().got_int = false;
    let mut discard = false;
    if unsafe { lua_pcall(lstate, 2, 1, 0) } != 0 {
        nlua_error(lstate, gettext(c"vim.on_key() callbacks: %.*s"));
    } else {
        if lua_isboolean(lstate, -1) {
            discard = unsafe { lua_toboolean(lstate, -1) } != 0;
        }
        lua_pop(lstate, 1);
    }
    g_st().got_int |= save_got_int;

    lua_pop(lstate, 1);
    debug_assert_eq!(top, unsafe { lua_gettop(lstate) });

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
    let lstate = nlua_st().active_lstate;
    let mut info = lua_Debug::new();

    let mut level: i32 = 1;
    let mut found = false;
    loop {
        if unsafe { lua_getstack(lstate, level, &raw mut info) } != 1 {
            break;
        }
        if lua_getinfo(lstate, c"nSl", &raw mut info) == 0 {
            break;
        }
        if crate::memory::cstr_first_byte(info.what) == b'C'
            || crate::memory::cstr_first_byte(info.source) != b'@'
        {
            level += 1;
            continue;
        }
        found = true;
        break;
    }
    if found && let Some(source_path) = fix_fname(info.source.wrapping_add(1)) {
        let mut sid = find_script_by_name(source_path.as_ptr().cast_mut());
        if sid <= 0 {
            let si: *mut Scriptitem = new_script_item(Some(source_path), &mut sid);
            unsafe {
                (*si).sn_lua = true;
            }
        }
        current.sc_sid = sid;
        current.sc_seq = -1;
        current.sc_lnum = info.currentline;
    }
}

pub fn nlua_do_ucmd(cmd: &mut Ucmd, eap: &mut Exarg, preview: bool) -> i32 {
    let lstate = nlua_st().global_lstate;
    nlua_pushref(
        lstate,
        if preview {
            cmd.uc_preview_luaref
        } else {
            cmd.uc_luaref
        },
    );

    lua_newtable(lstate);
    nlua_push_eap(lstate, &*eap, &g_st().cmdmod);

    lua_pushstring(lstate, cmd.uc_name.as_ptr());
    lua_setfield(lstate, -2, c"name");

    if eap.addr_count == 0 {
        unsafe { lua_pushinteger(lstate, cmd.uc_def as lua_Integer) };
        lua_setfield(lstate, -2, c"count");
    }

    if (cmd.uc_argt & EX_NOSPC) != 0 {
        unsafe { lua_createtable(lstate, 1, 0) };
        if (cmd.uc_argt & EX_NEEDARG) != 0 || !crate::memory::cstr_is_empty(eap.arg) {
            lua_pushstring(lstate, eap.arg);
            unsafe { lua_rawseti(lstate, -2, 1) };
        }
        lua_setfield(lstate, -2, c"fargs");
    }

    let narg_ch = if (cmd.uc_argt & EX_EXTRA) != 0 {
        if (cmd.uc_argt & EX_NOSPC) != 0 {
            if (cmd.uc_argt & EX_ARGSPACE) != 0 {
                b'_'
            } else if (cmd.uc_argt & EX_NEEDARG) != 0 {
                b'1'
            } else {
                b'?'
            }
        } else if (cmd.uc_argt & EX_NEEDARG) != 0 {
            b'+'
        } else {
            b'*'
        }
    } else {
        b'0'
    };
    let nargs: [c_char; 2] = [narg_ch as c_char, NUL];
    lua_pushstring(lstate, nargs.as_ptr());
    lua_setfield(lstate, -2, c"nargs");

    if preview {
        unsafe { lua_pushinteger(lstate, cmdpreview_get_ns() as lua_Integer) };
        let cmdpreview_bufnr: ObjHandle = cmdpreview_get_bufnr();
        if cmdpreview_bufnr != 0 {
            unsafe { lua_pushinteger(lstate, cmdpreview_bufnr as lua_Integer) };
        } else {
            unsafe { lua_pushnil(lstate) };
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
    if preview && unsafe { lua_isnumber(lstate, -1) } != 0 {
        let v = unsafe { lua_tointeger(lstate, -1) } as i32;
        if (0..=2).contains(&v) {
            retv = v;
            lua_pop(lstate, 1);
        }
    }

    retv
}

pub fn nlua_funcref_str(ref_: LuaRef, show_ref: bool) -> std::ffi::CString {
    let lstate = nlua_st().global_lstate;
    if unsafe { lua_checkstack(lstate, 1) } != 0 {
        nlua_pushref(lstate, ref_);
        if !lua_isfunction(lstate, -1) {
            lua_pop(lstate, 1);
        } else {
            let mut ar = lua_Debug::new();
            if lua_getinfo(lstate, c">S", &raw mut ar) != 0
                && crate::memory::cstr_first_byte(ar.source) == b'@'
                && ar.linedefined >= 0
            {
                let src = home_replace_save(None, ar.source.wrapping_add(1));
                let src_s = src.to_string_lossy();
                let formatted = if show_ref {
                    format!("<Lua {ref_}: {src_s}:{}>", ar.linedefined)
                } else {
                    format!("<Lua {src_s}:{}>", ar.linedefined)
                };
                return crate::memory::cstr_from_bytes(formatted.as_bytes());
            }
        }
    }

    let formatted = if show_ref {
        format!("<Lua {ref_}>")
    } else {
        "<Lua>".to_string()
    };
    crate::memory::cstr_from_bytes(formatted.as_bytes())
}

pub fn nlua_init_defaults() {
    let lstate = nlua_st().global_lstate;
    assert!(!lstate.is_null());

    lua_getglobal(lstate, c"require");
    lua_pushstring(lstate, c"vim._core.defaults");
    if nlua_pcall(lstate, 1, 0) != 0 {
        fprintf!(log_get_stderr(), c"%s\n", lua_tostring(lstate, -1));
    }
}

pub fn nlua_func_exists(lua_funcname: impl crate::strings::IntoCCharPtr) -> bool {
    let lua_funcname = lua_funcname.into_c_char_ptr();
    {
        let fname_bytes = crate::memory::cstr_as_bytes(lua_funcname);
        let mut str_bytes = Vec::with_capacity(7 + fname_bytes.len());
        str_bytes.extend_from_slice(b"return ");
        str_bytes.extend_from_slice(fname_bytes);
        let str_cstr = std::ffi::CString::new(str_bytes).unwrap();
        let mut args_items: [Object; 1] = [Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(NvimString {
                data: str_cstr.as_ptr().cast_mut(),
                size: str_cstr.as_bytes().len(),
            }),
        }];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 1,
            capacity: 1,
        };
        let mut err = Error::new();
        let code = NvimString {
            data: c"return type(loadstring(...)()) == 'function'"
                .as_ptr()
                .cast_mut(),
            size: 44,
        };
        let result = nlua_exec(code, ptr::null(), args, K_RET_NIL_BOOL, None, &mut err);
        drop(str_cstr);

        api_clear_error(&mut err);
        result.obj_type == ObjectType::ObjectTypeBoolean && result.data.boolean()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::command::is_user_cmdidx;
    use crate::ex_docmd::{CMD_USER, CMD_USER_BUF};

    #[test]
    fn mode_ret_and_user_cmdidx_helpers_work() {
        assert_eq!(mode_ret(K_RET_OBJECT), 1);
        assert_eq!(mode_ret(K_RET_MULTI), LUA_MULTRET);
        assert_eq!(mode_ret(K_RET_MULTI_STACK), LUA_MULTRET);
        assert!(is_user_cmdidx(CMD_USER));
        assert!(is_user_cmdidx(CMD_USER_BUF));
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
