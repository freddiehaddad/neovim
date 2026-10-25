//! Lua interpreter initialization, execution, and Vimscript/RPC bridges (`src/nvim/lua/executor.c`).

use crate::api::command::is_user_cmdidx;
use crate::api::extmark::ns_initialized;
use crate::api::private::helpers::{
    HlMessageChunk, OBJECT_NIL, TryState, api_clear_error, api_free_array, api_set_error,
    api_set_sctx, arena_array, error_set, string_to_cstr, try_enter, try_leave,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::change::inserted_bytes;
use crate::cursor::{check_cursor, colnr_T};
use crate::drawscreen::{UPD_NOT_VALID, redraw_curbuf_later};
use crate::errors::*;
use crate::eval::funcs::{find_internal_func, find_internal_func_lua};
use crate::eval::typval::{TypvalVval, VarType, tv_clear, typval_T};
use crate::eval::userfunc::{
    EvalFuncDef, FCERR_NONE, FCERR_OTHER, call_func, funcexe_T, register_luafunc,
};
use crate::event::r#loop::{
    loop_closing_ptr, loop_poll_events, loop_queues, loop_schedule_deferred, uv_thread_t,
};
use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::ex_cmds::check_secure;
use crate::ex_docmd::{cmdmod_T, do_cmdline, exarg_T, get_command_name};
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
    LUA_TFUNCTION, LUA_TSTRING, LUA_TTABLE, LuaRef, lua_CFunction, lua_Integer, lua_State,
    lua_call, lua_checkstack, lua_close, lua_concat, lua_createtable, lua_error, lua_getfield,
    lua_getglobal, lua_getmetatable, lua_gettop, lua_insert, lua_isboolean, lua_iscfunction,
    lua_isfunction, lua_isnil, lua_isnumber, lua_isstring, lua_istable, lua_newtable,
    lua_newuserdata, lua_next, lua_pcall, lua_pop, lua_pushboolean, lua_pushcclosure,
    lua_pushcfunction, lua_pushinteger, lua_pushlightuserdata, lua_pushlstring, lua_pushnil,
    lua_pushnumber, lua_pushstring, lua_pushvalue, lua_rawgeti, lua_rawseti, lua_remove,
    lua_replace, lua_setfield, lua_setglobal, lua_setmetatable, lua_settop, lua_toboolean,
    lua_tocfunction, lua_tointeger, lua_tolstring, lua_tostring, lua_touserdata, lua_type,
    lua_upvalueindex, luaL_checkinteger, luaL_checklstring, luaL_checktype, luaL_error,
    luaL_loadbuffer, luaL_newstate, luaL_openlibs, luaL_ref, luaL_unref, luaL_where,
    nlua_init_types, nlua_pop_Array, nlua_pop_Integer, nlua_pop_Object, nlua_pop_typval,
    nlua_push_Array, nlua_push_Object, nlua_push_typval, nlua_ref_state_t,
};
use crate::lua::stdlib::nlua_state_add_stdlib;
use crate::lua::treesitter::{nlua_treesitter_free, nlua_treesitter_init};
use crate::main_init::{getout, os_exit, preserve_exit};
use crate::math::{FAIL, OK};
use crate::memline::{ml_get_buf, ml_get_buf_len, ml_replace};
use crate::memory::{
    Arena, ArenaMem, arena_finish, arena_mem_free, strequal, xfree, xmalloc, xmallocz,
    xmemdupz, xstrdup,
};
use crate::message::{
    emsg, msg_ext_no_fast, msg_multihl, msg_putchar, msg_use_printf, redirecting, semsg_multiline,
};
use crate::msgpack_rpc::channel::{rpc_send_call, rpc_send_event};
use crate::msgpack_rpc::packer::handle_T;
use crate::option_vars::*;
#[cfg(windows)]
use crate::os::env::os_getenv;
use crate::os::env::{expand_T, home_replace_save};
use crate::os::fileio::{FileDescriptor, file_close, file_open_stdin, file_read};
use crate::path::fix_fname;
use crate::profile::{proftime_T, scriptitem_T, time_msg, time_pop, time_push};
use crate::runtime::{
    cmd_source_buffer, find_script_by_name, new_script_item, runtime_get_named_thread,
    runtime_search_path_validate, script_is_lua, sctx_T,
};
use crate::strings::{gettext, vim_snprintf};
use crate::ui::kUIGlobalCount;
use crate::ui::{ui_add_cb, ui_flush, ui_has, ui_remove_cb};
use crate::undo::u_save;
use crate::usercmd::{uc_mods, uc_split_args_iter, ucmd_T};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::Write;
use std::mem::size_of;
use std::ptr;

use crate::api::vim::HlMessage;
pub type LuaRetMode = c_int;

type luv_CFpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        flags: c_int,
    ) -> c_int,
>;
type luv_CFcpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        func: lua_CFunction,
        ud: *mut c_void,
        flags: c_int,
    ) -> c_int,
>;
type luv_acquire_vm = Option<unsafe extern "C-unwind" fn() -> *mut lua_State>;
type luv_release_vm = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State)>;

pub const kRetObject: LuaRetMode = 0;
pub const kRetNilBool: LuaRetMode = 1;
pub const kRetLuaref: LuaRetMode = 2;
pub const kRetMulti: LuaRetMode = 3;
pub const kRetMultiStack: LuaRetMode = 4;

const MAX_FUNC_ARGS: c_int = 20;

const LUA_REFNIL: LuaRef = -1;

const LUVF_CALLBACK_NOEXIT: c_int = 0x01;

const kCallback: isize = 0;
const kThread: isize = 1;
const kThreadCallback: isize = 2;

const CMD_equal: c_int = 559;

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

static mut IN_FAST_CALLBACK: c_int = 0;
static mut GLOBAL_LSTATE: *mut lua_State = ptr::null_mut();

pub static mut active_lstate: *mut lua_State = ptr::null_mut();

static mut REQUIRE_REF: LuaRef = LUA_REFNIL;
static mut MAIN_THREAD: uv_thread_t = 0;
static mut EXPAND_RESULT_ARRAY: Vec<*mut c_char> = Vec::new();

use crate::api::buffer::LUA_INTERNAL_CALL;
use crate::api::private::dispatch::nlua_add_api_functions;
use crate::buffer::VAR_FIXED;
use crate::charset::NUL;
use crate::cmdexpand::{MB_MAXBYTES, kUICmdline};
use crate::eval::typval::kCallbackNone;
use crate::ex_docmd::{
    CMD_SIZE, CMOD_BROWSE, CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS,
    CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE,
    CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, EX_ARGSPACE, EX_EXTRA, EX_NEEDARG, EX_NOSPC,
};
use crate::globals::IOSIZE;
use crate::lua::converter::LUA_NOREF;
#[cfg(windows)]
use crate::os::os_win_console::ExitThread;
use crate::version::main_exitfree;
use crate::window::{WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT};

unsafe extern "C-unwind" {
    fn luaopen_luv(L: *mut lua_State) -> c_int;
}

unsafe extern "C" {
    fn luv_set_loop(L: *mut lua_State, loop_: *mut c_void);
    fn luv_set_callback(L: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_thread(L: *mut lua_State, pcall: luv_CFpcall);
    fn luv_set_cthread(L: *mut lua_State, cpcall: luv_CFcpcall);
    fn luv_set_thread_cb(acquire: luv_acquire_vm, release: luv_release_vm);

    #[cfg(unix)]
    fn pthread_exit(retval: *mut c_void) -> !;
}

#[inline]
unsafe fn put_event(mq: *mut MultiQueue, ev: Event) {
    unsafe {
        if !mq.is_null() {
            multiqueue_put_event(mq, ev);
        }
    }
}

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
                let mut buf = vec![0 as c_char; length];
                let mut done = false;
                while !done {
                    done = uc_split_args_iter(
                        (*eap).arg,
                        length,
                        &raw mut end,
                        buf.as_mut_ptr(),
                        &raw mut len,
                    );
                    if len > 0 {
                        lua_pushlstring(lstate, buf.as_ptr(), len);
                        lua_rawseti(lstate, -2, i);
                        i += 1;
                    }
                }
            }
            lua_setfield(lstate, -2, c"fargs".as_ptr());
        }
    }
}

pub unsafe fn get_global_lstate() -> *mut lua_State {
    unsafe { GLOBAL_LSTATE }
}

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

pub unsafe fn nlua_error(lstate: *mut lua_State, msg: *const c_char) {
    unsafe {
        let mut len: usize = 0;
        let str_ = nlua_get_error(lstate, &raw mut len);
        msg_ext_no_fast();
        semsg_multiline!(c"lua_error".as_ptr(), msg, len as c_int, str_);
        lua_pop(lstate, 1);
    }
}

unsafe extern "C-unwind" fn nlua_no_traceback(_lstate: *mut lua_State) -> c_int {
    1
}

pub unsafe fn nlua_pcall(lstate: *mut lua_State, nargs: c_int, mut nresults: c_int) -> c_int {
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
}

pub unsafe fn nlua_luv_error_event(error: *mut c_char, err_type: isize) {
    unsafe {
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
                Event::NluaLuvError {
                    msg: if !error.is_null() {
                        xstrdup(error)
                    } else {
                        ptr::null_mut()
                    },
                    kind: kCallback,
                },
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

unsafe extern "C-unwind" fn nlua_luv_thread_cb_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    nresult: c_int,
    flags: c_int,
) -> c_int {
    unsafe { nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, true) }
}

unsafe extern "C-unwind" fn nlua_luv_thread_cfpcall(
    lstate: *mut lua_State,
    nargs: c_int,
    nresult: c_int,
    flags: c_int,
) -> c_int {
    unsafe { nlua_luv_thread_common_cfpcall(lstate, nargs, nresult, flags, false) }
}

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
                fprintf!(
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
            let ev = Event::NluaLuvError {
                msg: xmemdupz(error.cast(), len).cast(),
                kind: if is_callback {
                    kThreadCallback
                } else {
                    kThread
                },
            };
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

pub unsafe fn nlua_schedule_event(cb: LuaRef, ns_id: u32) {
    unsafe {
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
            Event::NluaSchedule {
                cb,
                ns_id: ui_event_ns_id,
            },
        );
        lua_pushnil(lstate);
        2
    }
}

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

unsafe extern "C-unwind" fn nlua_ui_flush(_lstate: *mut lua_State) -> c_int {
    unsafe {
        ui_flush();
        0
    }
}

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

pub unsafe fn nlua_ex_getline(
    _c: c_int,
    ecd: *mut ExDocmdCookie,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
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
            crate::cmdexpand::LineGetter::LuaEx(&raw mut cookie),
            0,
        );
        no_wait_return -= 1;
        nlua_unref_global(lstate, cookie.getline);
        lua_pushboolean(lstate, c_int::from(cookie.got_line));
        1
    }
}

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

unsafe fn nlua_get_ref_state(lstate: *mut lua_State) -> *mut nlua_ref_state_t {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nlua.ref_state".as_ptr());
        let ref_state = lua_touserdata(lstate, -1).cast::<nlua_ref_state_t>();
        lua_pop(lstate, 1);
        ref_state
    }
}

pub unsafe fn nlua_get_nil_ref(lstate: *mut lua_State) -> LuaRef {
    unsafe { (*nlua_get_ref_state(lstate)).nil_ref }
}

pub unsafe fn nlua_get_empty_dict_ref(lstate: *mut lua_State) -> LuaRef {
    unsafe { (*nlua_get_ref_state(lstate)).empty_dict_ref }
}

pub unsafe fn nlua_get_global_ref_count() -> c_int {
    unsafe { (*nlua_global_refs).ref_count }
}

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
            fprintf!(log_get_stderr(), c"%s\n".as_ptr(), lua_tostring(lstate, -1));
            return false;
        }

        true
    }
}

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

pub unsafe fn nlua_init(argv: *mut *mut c_char, argc: c_int, lua_arg0: c_int) {
    unsafe {
        let lstate = luaL_newstate();
        if lstate.is_null() {
            fprintf!(
                log_get_stderr(),
                gettext(c"E970: Failed to initialize Lua interpreter\n".as_ptr()),
            );
            os_exit(1);
        }
        luaL_openlibs(lstate);
        if !nlua_state_init(lstate) {
            fprintf!(
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
}

unsafe extern "C-unwind" fn nlua_thread_acquire_vm() -> *mut lua_State {
    unsafe { nlua_init_state(true) }
}

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

pub unsafe fn nlua_free_all_mem() {
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

unsafe extern "C-unwind" fn nlua_common_free_all_mem(lstate: *mut lua_State) {
    unsafe {
        let ref_state = nlua_get_ref_state(lstate);
        nlua_unref(lstate, ref_state, (*ref_state).nil_ref);
        nlua_unref(lstate, ref_state, (*ref_state).empty_dict_ref);
        lua_close(lstate);
    }
}

pub unsafe fn nlua_print_event(data: *mut c_char, len: usize) {
    unsafe {
        msg_ext_no_fast();
        let raw = msg_use_printf() != 0 && redirecting() == 0 && on_print.cb_type() == kCallbackNone;

        let mut msg: HlMessage = vec![HlMessageChunk {
            text: NvimString {
                data,
                size: len.saturating_sub(1),
            },
            hl_id: 0,
        }];
        let mut needs_clear = false;
        msg_multihl(
            OBJECT_NIL,
            &mut msg,
            c"lua_print".as_ptr(),
            true,
            false,
            ptr::null_mut(),
            &raw mut needs_clear,
            raw,
        );
    }
}

unsafe extern "C-unwind" fn nlua_print(lstate: *mut lua_State) -> c_int {
    unsafe {
        let nargs = lua_gettop(lstate);
        lua_getglobal(lstate, c"tostring".as_ptr());
        let mut errmsg_len: usize = 0;
        let mut msg_vec: Vec<c_char> = Vec::with_capacity(80);
        let mut curargidx: c_int = 1;
        while curargidx <= nargs {
            lua_pushvalue(lstate, -1);
            lua_pushvalue(lstate, curargidx);
            if lua_pcall(lstate, 1, 1, 0) != 0 || lua_type(lstate, -1) != LUA_TSTRING {
                let errmsg = nlua_get_error(lstate, &raw mut errmsg_len);
                drop(msg_vec);
                let mut buff = [0 as c_char; IOSIZE];
                let fmt = gettext(c"E5114: Converting print argument #%i: %.*s".as_ptr());
                let len = vim_snprintf!(
                    buff.as_mut_ptr(),
                    IOSIZE,
                    fmt,
                    curargidx,
                    errmsg_len as c_int,
                    errmsg,
                ) as usize;
                lua_pushlstring(lstate, buff.as_ptr(), len);
                return lua_error(lstate);
            }

            let mut len: usize = 0;
            let s = lua_tolstring(lstate, -1, &raw mut len);
            if !s.is_null() && len > 0 {
                msg_vec.extend_from_slice(std::slice::from_raw_parts(s, len));
            }
            if curargidx < nargs {
                msg_vec.push(b' ' as c_char);
            }
            lua_pop(lstate, 1);
            curargidx += 1;
        }
        let msg_len = msg_vec.len() + 1;
        let msg_data: *mut c_char = xmemdupz(msg_vec.as_ptr().cast(), msg_vec.len()).cast();
        drop(msg_vec);

        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        let is_thread = lua_toboolean(lstate, -1) != 0;
        lua_pop(lstate, 1);

        if is_thread {
            loop_schedule_deferred(
                (&raw mut main_loop).cast(),
                Event::NluaPrint {
                    data: msg_data,
                    len: msg_len,
                },
            );
        } else if IN_FAST_CALLBACK != 0 {
            let queues = loop_queues((&raw mut main_loop).cast());
            put_event(
                (*queues).events,
                Event::NluaPrint {
                    data: msg_data,
                    len: msg_len,
                },
            );
        } else {
            nlua_print_event(msg_data, msg_len);
        }
        0
    }
}

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
            let mut input: typval_T = typval_T::new();
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

pub unsafe extern "C-unwind" fn nlua_in_fast_event(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushboolean(lstate, c_int::from(IN_FAST_CALLBACK > 0));
        1
    }
}

unsafe fn viml_func_is_fast(name: *const c_char) -> bool {
    unsafe {
        let fdef: *const EvalFuncDef = find_internal_func(name);
        !fdef.is_null() && (*fdef).fast
    }
}

pub unsafe extern "C-unwind" fn nlua_call(lstate: *mut lua_State) -> c_int {
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

        let mut vim_args: [typval_T; (MAX_FUNC_ARGS + 1) as usize] = [typval_T::new(); (MAX_FUNC_ARGS + 1) as usize];
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

            let mut rettv: typval_T = typval_T::new();
            let mut funcexe: funcexe_T = funcexe_T::new();
            funcexe.fe_firstline = (*curwin).w_cursor.lnum;
            funcexe.fe_lastline = (*curwin).w_cursor.lnum;
            funcexe.fe_evaluate = true;
            let save_current_sctx = api_set_sctx(LUA_INTERNAL_CALL);

            let mut tstate = TryState::new();
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
}

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

unsafe extern "C-unwind" fn nlua_rpcnotify(lstate: *mut lua_State) -> c_int {
    unsafe { nlua_rpc(lstate, false) }
}

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

unsafe extern "C-unwind" fn nlua_nil_tostring(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushstring(lstate, c"vim.NIL".as_ptr());
        1
    }
}

unsafe extern "C-unwind" fn nlua_nil_index(lstate: *mut lua_State) -> c_int {
    unsafe { luaL_error(lstate, c"attempt to index vim.NIL".as_ptr()) }
}

unsafe extern "C-unwind" fn nlua_empty_dict_tostring(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_pushstring(lstate, c"vim.empty_dict()".as_ptr());
        1
    }
}

#[cfg(windows)]
unsafe extern "C-unwind" fn nlua_getenv(lstate: *mut lua_State) -> c_int {
    unsafe {
        let name = luaL_checklstring(lstate, 1, ptr::null_mut());
        let val = os_getenv(name);
        lua_pushstring(lstate, val);
        xfree(val.cast());
        1
    }
}

pub unsafe fn nlua_ref(
    lstate: *mut lua_State,
    ref_state: *mut nlua_ref_state_t,
    index: c_int,
) -> LuaRef {
    unsafe {
        lua_pushvalue(lstate, index);
        let ref_ = luaL_ref(lstate, LUA_REGISTRYINDEX);
        if ref_ > 0 {
            (*ref_state).ref_count += 1;
        }
        ref_
    }
}

pub unsafe fn nlua_ref_global(lstate: *mut lua_State, index: c_int) -> LuaRef {
    unsafe { nlua_ref(lstate, nlua_global_refs, index) }
}

pub unsafe fn nlua_unref(lstate: *mut lua_State, ref_state: *mut nlua_ref_state_t, ref_: LuaRef) {
    unsafe {
        if ref_ > 0 {
            (*ref_state).ref_count -= 1;
            luaL_unref(lstate, LUA_REGISTRYINDEX, ref_);
        }
    }
}

pub unsafe fn nlua_unref_global(lstate: *mut lua_State, ref_: LuaRef) {
    unsafe {
        nlua_unref(lstate, nlua_global_refs, ref_);
    }
}

pub unsafe fn api_free_luaref(ref_: LuaRef) {
    unsafe {
        nlua_unref_global(GLOBAL_LSTATE, ref_);
    }
}

pub unsafe fn nlua_pushref(lstate: *mut lua_State, ref_: LuaRef) {
    unsafe {
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, ref_);
    }
}

pub unsafe fn api_new_luaref(original_ref: LuaRef) -> LuaRef {
    if original_ref == LUA_NOREF {
        return LUA_NOREF;
    }
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, original_ref);
        let new_ref = nlua_ref_global(lstate, -1);
        lua_pop(lstate, 1);
        new_ref
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
    unsafe {
        static mut LCMD_BUF: [c_char; IOSIZE] = [0; IOSIZE];
        let buf_ptr = (&raw mut LCMD_BUF).cast::<c_char>();
        let mut lcmd = buf_ptr;
        let mut lcmd_vec: Vec<c_char> = Vec::new();
        let lcmd_len = vim_snprintf!(buf_ptr, IOSIZE, fmt, bodylen as c_int, body);
        if lcmd_len as usize >= IOSIZE {
            lcmd_vec.resize(lcmd_len as usize + 1, 0);
            lcmd = lcmd_vec.as_mut_ptr();
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
    }
}

pub unsafe fn nlua_call_luaeval(str_: NvimString, arg: *mut typval_T, ret_tv: *mut typval_T) {
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

pub unsafe fn nlua_call_vlua(
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

pub unsafe fn nlua_call_user_expand_func(xp: *mut expand_T, ret_tv: *mut typval_T) {
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
}

pub unsafe fn nlua_exec_lines(lines: &[std::ffi::CString], name: *mut c_char) {
    unsafe {
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
            ptr::null_mut(),
        );
    }
}

pub unsafe fn nlua_exec_typval_callable(
    lua_cb: LuaRef,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) -> c_int {
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
}

pub unsafe fn nlua_exec(
    str_: NvimString,
    chunkname: *const c_char,
    args: Array,
    mode: LuaRetMode,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
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
}

pub unsafe fn nlua_call_typval(
    module: *const c_char,
    func: *const c_char,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) {
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
}

pub unsafe fn nlua_call_excmd(
    module: *const c_char,
    func: *const c_char,
    eap: *mut exarg_T,
    cmod: *const cmdmod_T,
    extra: *mut typval_T,
) -> bool {
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
}

pub unsafe fn nlua_ref_is_function(ref_: LuaRef) -> bool {
    unsafe {
        let lstate = GLOBAL_LSTATE;
        nlua_pushref(lstate, ref_);
        let is_func = lua_type(lstate, -1) == LUA_TFUNCTION;
        lua_pop(lstate, 1);
        is_func
    }
}

pub unsafe fn nlua_call_ref(
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

pub unsafe fn nlua_call_ref_ctx(
    fast: bool,
    ref_: LuaRef,
    name: *const c_char,
    args: Array,
    mode: LuaRetMode,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
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
}

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

pub unsafe fn nlua_is_deferred_safe() -> bool {
    unsafe { IN_FAST_CALLBACK == 0 }
}

pub unsafe fn ex_lua(eap: *mut exarg_T) {
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
}

pub unsafe fn ex_luado(eap: *mut exarg_T) {
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
        let mut lcmd_vec: Vec<c_char> = Vec::new();
        let lcmd = if lcmd_len < IOSIZE {
            buf_ptr
        } else {
            lcmd_vec.resize(lcmd_len + 1, 0);
            lcmd_vec.as_mut_ptr()
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
            return;
        }
        drop(lcmd_vec);
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
}

pub unsafe fn ex_luafile(eap: *mut exarg_T) {
    unsafe {
        let _ = nlua_exec_file((*eap).arg);
    }
}

pub unsafe fn nlua_exec_file(path: *const c_char) -> bool {
    unsafe {
        let lstate = GLOBAL_LSTATE;
        if !strequal(path, c"-".as_ptr()) {
            lua_getglobal(lstate, c"loadfile".as_ptr());
            lua_pushstring(lstate, path);
        } else {
            let mut stdin_dup = FileDescriptor::new();
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
}

pub unsafe fn nlua_expand_pat(xp: *mut expand_T) {
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
        let er = &raw mut EXPAND_RESULT_ARRAY;
        if !error_set(&raw const err) && prefix_len <= patlen {
            let completions = nlua_pop_Array(lstate, &raw mut arena, &raw mut err);
            if !error_set(&raw const err) {
                for &p in &*er {
                    xfree(p.cast());
                }
                *er = Vec::with_capacity(completions.size);

                let mut all_strings = true;
                for i in 0..completions.size {
                    let v = *completions.items.add(i);
                    if v.obj_type != ObjectType::kObjectTypeString {
                        all_strings = false;
                        break;
                    }
                    (*er).push(string_to_cstr(v.data.string));
                }

                if all_strings {
                    (*xp).xp_pattern = (*xp).xp_pattern.offset(prefix_len);
                    status = OK;
                }
            }
            arena_mem_free(arena_finish(&raw mut arena));
        }

        if status == FAIL {
            for &p in &*er {
                xfree(p.cast());
            }
            *er = Vec::new();
        }
    }
}

pub unsafe fn nlua_expand_get_matches(
    num_results: *mut c_int,
    results: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let er = &raw mut EXPAND_RESULT_ARRAY;
        if (*er).is_empty() {
            *results = ptr::null_mut();
            *num_results = 0;
        } else {
            let len = (*er).len();
            let out = xmalloc(len * size_of::<*mut c_char>()).cast::<*mut c_char>();
            ptr::copy_nonoverlapping((*er).as_ptr(), out, len);
            *results = out;
            *num_results = len as c_int;
        }
        *er = Vec::new();
        c_int::from(*num_results > 0)
    }
}

unsafe extern "C-unwind" fn nlua_is_thread(lstate: *mut lua_State) -> c_int {
    unsafe {
        lua_getfield(lstate, LUA_REGISTRYINDEX, c"nvim.thread".as_ptr());
        1
    }
}

pub unsafe fn nlua_is_table_from_lua(arg: *const typval_T) -> bool {
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

pub unsafe fn nlua_register_table_as_callable(arg: *const typval_T) -> *mut c_char {
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
}

pub unsafe fn nlua_exec_on_key(c: c_int, typed_buf: *mut c_char) -> bool {
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
}

pub unsafe fn nlua_set_sctx(current: *mut sctx_T) {
    unsafe {
        if !script_is_lua((*current).sc_sid) {
            return;
        }

        (*current).sc_lnum = 0;

        if p_verbose <= 0 {
            return;
        }
        let lstate = active_lstate;
        let mut info = lua_Debug::new();

        let mut level: c_int = 1;
        let mut found = false;
        loop {
            if lua_getstack(lstate, level, &raw mut info) != 1 {
                break;
            }
            if lua_getinfo(lstate, c"nSl".as_ptr(), &raw mut info) == 0 {
                break;
            }
            if *info.what == b'C' as c_char || *info.source != b'@' as c_char {
                level += 1;
                continue;
            }
            found = true;
            break;
        }
        if found {
            let source_path = fix_fname(info.source.add(1));
            let mut sid = find_script_by_name(source_path);
            if sid > 0 {
                xfree(source_path.cast());
            } else {
                let si: *mut scriptitem_T = new_script_item(source_path, &raw mut sid);
                (*si).sn_lua = true;
            }
            (*current).sc_sid = sid;
            (*current).sc_seq = -1;
            (*current).sc_lnum = info.currentline;
        }
    }
}

pub unsafe fn nlua_do_ucmd(cmd: *mut ucmd_T, eap: *mut exarg_T, preview: bool) -> c_int {
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
}

pub unsafe fn nlua_funcref_str(ref_: LuaRef, arena: *mut Arena, show_ref: bool) -> *mut c_char {
    unsafe {
        let lstate = GLOBAL_LSTATE;
        if lua_checkstack(lstate, 1) != 0 {
            nlua_pushref(lstate, ref_);
            if !lua_isfunction(lstate, -1) {
                lua_pop(lstate, 1);
            } else {
                let mut ar = lua_Debug::new();
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
}

pub unsafe fn nlua_init_defaults() {
    unsafe {
        let L = GLOBAL_LSTATE;
        assert!(!L.is_null());

        lua_getglobal(L, c"require".as_ptr());
        lua_pushstring(L, c"vim._core.defaults".as_ptr());
        if nlua_pcall(L, 1, 0) != 0 {
            fprintf!(log_get_stderr(), c"%s\n".as_ptr(), lua_tostring(L, -1));
        }
    }
}

pub unsafe fn nlua_func_exists(lua_funcname: *const c_char) -> bool {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::command::is_user_cmdidx;
    use crate::ex_docmd::{CMD_USER, CMD_USER_BUF};

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
