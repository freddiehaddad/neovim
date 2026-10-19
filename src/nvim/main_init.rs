//! Main entry point, CLI argument parsing, startup, and shutdown (`src/nvim/main.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_mut,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_metadata_raw;
#[allow(unused_imports)]
use crate::api::ui::remote_ui_wait_for_attach;
#[allow(unused_imports)]
use crate::buffer::{
    buf_set_changedtick, buflist_new, do_autochdir, do_modelines, handle_swap_exists,
    set_buflisted, set_curbuf, setfname,
};
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::{eval_init, set_argv_var, timer_teardown};
#[allow(unused_imports)]
use crate::ex_docmd::{do_cmdline_cmd, filetype_maybe_enable, filetype_plugin_enable};
#[allow(unused_imports)]
use crate::fileio::shorten_fnames;
use crate::globals::*;
#[allow(unused_imports)]
use crate::log::log_get_stderr;
#[allow(unused_imports)]
use crate::lua::executor::{
    get_global_lstate, nlua_exec_file, nlua_init, nlua_init_defaults, nlua_pcall,
};
#[allow(unused_imports)]
use crate::lua::executor::{nlua_call_typval, nlua_exec};
use crate::message::semsg;
#[allow(unused_imports)]
use crate::message::{msg_putchar, wait_return};
#[allow(unused_imports)]
use crate::option::set_option_direct;
#[allow(unused_imports)]
use crate::option::{
    reset_modifiable, set_init_1, set_init_2, set_init_3, set_init_tablocal,
    set_option_value_give_err, set_options_bin,
};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::os::env::{os_get_pid, os_hint_priority};
#[allow(unused_imports)]
use crate::os::fs::os_write;
#[allow(unused_imports)]
use crate::os::input::os_isatty;
#[allow(unused_imports)]
use crate::os::stdpaths::{appname_is_valid, get_appname};
#[allow(unused_imports)]
use crate::register::get_default_register_name;
#[allow(unused_imports)]
use crate::runtime::{do_source, load_plugins};
#[allow(unused_imports)]
use crate::syntax::syn_maybe_enable;
#[allow(unused_imports)]
use crate::ui::{ui_call_error_exit, ui_call_set_title};
#[allow(unused_imports)]
use crate::window::win_new_screensize;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::io::Write;
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::api::private::converter::sctx_T;
use crate::api::private::helpers::{api_free_object, cstr_as_string};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::{tabpage_T, win_T};
use crate::arglist::{aentry_T, alist_T, alist_add, alist_init, alist_name, linenr_T};
use crate::autocmd::{
    EVENT_BUFENTER, EVENT_BUFUNLOAD, EVENT_BUFWINLEAVE, EVENT_VIMENTER, EVENT_VIMLEAVE,
    EVENT_VIMLEAVEPRE, apply_autocmds, apply_autocmds_win, autocmd_init, block_autocmds,
    is_autocmd_blocked, unblock_autocmds,
};
use crate::buffer::{BLN_LISTED, buf_is_empty, buf_valid, bufref_T, bufref_valid, set_bufref};
use crate::channel::{channel_connect, channel_from_stdio, channel_init, channel_teardown};
use crate::diff::{diff_win_options, diffopt_horizontal};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, default_grid_alloc, redraw_all_later, redraw_later, screenclear,
};
use crate::eval::buffer::varnumber_T;
use crate::eval::deprecated::{CallbackReader, buf_T};
use crate::eval::typval::{tv_clear, tv_list_alloc, tv_list_alloc_ret, tv_list_append_string};
use crate::eval::vars::{
    OptIndex, OptInt, VV_ARGF, VV_EXITING, VV_EXITREASON, VV_OLDFILES, VV_PROGNAME, VV_PROGPATH,
    VV_SEND_SERVER, VV_STARTTIME, VV_SWAPCOMMAND, VV_VIM_DID_ENTER, VV_VIM_DID_INIT, VimVarIndex,
    get_vim_var_list, get_vim_var_str, set_reg_var, set_vim_var_list, set_vim_var_nr,
    set_vim_var_string, set_vim_var_type,
};
use crate::eval::{eval_has_provider, garbage_collect};
use crate::event::r#loop::{Loop, loop_close, loop_init, loop_poll_events, loop_queues};
use crate::event::multiqueue::{MultiQueue, multiqueue_new_child, multiqueue_process_events};
use crate::event::proc::proc_teardown;
use crate::event::socket::socket_address_tcp_host_end;
use crate::event::stream::{UV_NAMED_PIPE, UV_TCP};
use crate::ex_cmds::{ECMD_HIDE, ECMD_LASTL, do_ecmd};
use crate::ex_getln::cmdline_init;
use crate::extmark::TriState;
use crate::fileio::{READ_NEW, READ_STDIN};
use crate::garray::ga_grow;
use crate::hashtab::hash_debug_results;
use crate::highlight::highlight_init;
use crate::highlight_group::init_highlight;
use crate::input::{open_scriptin, stuffcharReadbuff, vgetc};
use crate::keycodes::K_NOP;
use crate::log::{LOGLVL_DBG, LOGLVL_INF, log_init, logmsg_str};
use crate::lua::base64::lua_State;
use crate::map_glyph_cache::NvimString;
use crate::mark::setpcmark;
use crate::math::{FAIL, OK};
use crate::mcursor::mc_init;
use crate::memfile::memfile_T;
use crate::memline::{ml_close_all, ml_close_notmod, ml_recover, ml_sync_all, recover_names};
use crate::memory::{Arena, entered_free_all_mem, strequal, xfree, xmalloc, xstrdup};
use crate::mouse::setmouse;
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::server::{server_init, server_teardown};
use crate::normal::{check_scrollbind, init_normal_cmds, normal_enter};
use crate::os::env::{
    env_init, init_homedir, os_getenv, os_getenv_noalloc, os_homedir, os_setenv_append_path,
    vim_env_iter,
};
use crate::os::fs::{os_dup_cloexec, os_exepath, os_fopen, os_isdir, os_path_exists};
use crate::os::input::os_breakcheck;
use crate::os::lang::{init_locale, set_lang_var};
#[cfg(windows)]
use crate::os::os_win_console::{os_icon_init, os_icon_reset, os_title_reset, os_title_save};
use crate::os::signal::{signal_init, signal_reject_deadly, signal_stop, signal_teardown};
use crate::os::stdpaths::{XDGVarType, stdpaths_get_xdg_var, stdpaths_user_conf_subpath};
use crate::os::time::os_realtime;
use crate::path::{
    concat_fnames, path_equal, path_fix_case, path_guess_exepath, path_tail, path_to_slash,
    vim_FullName,
};
use crate::profile::{FILE, profile_dump, time_finish, time_init, time_msg, time_start};
use crate::quickfix::{qf_init, qf_init_stack, qf_jump};
use crate::runtime::{
    DOSO_NONE, DOSO_VIMRC, ETYPE_ARGS, ETYPE_ENV, estack_init, estack_pop, estack_push,
    runtime_init,
};
use crate::shada::{shada_read_everything, shada_write_file};
use crate::terminal::{terminal_init, terminal_teardown};
use crate::ui::{do_autocmd_uienter_all, ui_flush, ui_init};
use crate::ui_client::{ui_client_run, ui_client_start_server, ui_client_stop};
use crate::ui_compositor::ui_comp_syn_init;
use crate::version::list_version;
use crate::window::{
    goto_tabpage, make_tabpages, make_windows, only_one_window, win_alloc_first, win_close,
    win_count, win_enter, win_equal, win_init_size, win_valid,
};

pub const MAX_ARG_CMDS: usize = 10;

const NUL: c_char = 0;
const IOSIZE: usize = 1025;
const MAXPATHL: usize = 4096;
const MAXLNUM: linenr_T = 0x7fffffff;

const STDIN_FILENO: c_int = 0;
const STDOUT_FILENO: c_int = 1;
const STDERR_FILENO: c_int = 2;

const NO_BUFFERS: c_int = 1;

const WIN_HOR: c_int = 1;
const WIN_VER: c_int = 2;
const WIN_TABS: c_int = 3;

const EDIT_NONE: c_int = 0;
const EDIT_FILE: c_int = 1;
const EDIT_STDIN: c_int = 2;
const EDIT_TAG: c_int = 3;
const EDIT_QF: c_int = 4;

const SEA_NONE: c_int = 0;
const SEA_DIALOG: c_int = 1;
const SEA_QUIT: c_int = 2;

const SID_CMDARG: c_int = -2;
const SID_CARG: c_int = -3;
const SID_ENV: c_int = -4;

const kListLenMayKnow: isize = -2;
const VAR_FIXED: c_int = 2;

const kNone: TriState = -1;
const kFalse: TriState = 0;
const kTrue: TriState = 1;

const kRetObject: c_int = 0;
const kPathCmpExpand: c_int = 2;
const KXDG_CONFIG_DIRS: c_int = 5;

const kOptArabic: OptIndex = 3;
const kOptErrorfile: OptIndex = 86;
const kOptKeymap: OptIndex = 159;
const kOptRightleft: OptIndex = 241;
const kOptShadafile: OptIndex = 259;
const kOptShortmess: OptIndex = 271;
const kOptVerbosefile: OptIndex = 344;
const kOptWindow: OptIndex = 362;

const kOptCbFlagUnnamed: c_uint = 0x01;
const kOptCbFlagUnnamedplus: c_uint = 0x02;

const LUA_GLOBALSINDEX: c_int = -10002;

#[cfg(windows)]
const PATHSEP: u8 = b'\\';
#[cfg(not(windows))]
const PATHSEP: u8 = b'/';

const CASE_INSENSITIVE_FILENAME: bool = cfg!(any(target_os = "windows", target_os = "macos"));

const ERR_ARG_MISSING: &CStr = c"Argument missing after";
const ERR_OPT_GARBAGE: &CStr = c"Garbage after option argument";
const ERR_OPT_UNKNOWN: &CStr = c"Unknown option argument";
const ERR_TOO_MANY_ARGS: &CStr = c"Too many edit arguments";
const ERR_EXTRA_CMD: &CStr =
    c"Too many \"+command\", \"-c command\" or \"--cmd command\" arguments";

#[repr(C)]
pub struct mparm_T {
    pub argc: c_int,
    pub argv: *mut *mut c_char,

    pub use_vimrc: *mut c_char,
    pub clean: bool,

    pub n_commands: c_int,
    pub commands: [*mut c_char; MAX_ARG_CMDS],
    pub cmds_tofree: [c_char; MAX_ARG_CMDS],
    pub n_pre_commands: c_int,
    pub pre_commands: [*mut c_char; MAX_ARG_CMDS],
    pub luaf: *mut c_char,
    pub lua_arg0: c_int,

    pub edit_type: c_int,
    pub tagname: *mut c_char,
    pub use_ef: *mut c_char,

    pub exmode: bool,
    pub input_istext: bool,

    pub no_swap_file: c_int,
    pub use_debug_break_level: c_int,
    pub window_count: c_int,
    pub window_layout: c_int,

    pub diff_mode: c_int,

    pub listen_addr: *mut c_char,
    pub remote: c_int,
    pub server_addr: *mut c_char,
    pub scriptin: *mut c_char,
    pub scriptout: *mut c_char,
    pub scriptout_append: bool,
    pub had_stdin_file: bool,
}

#[cfg(windows)]
#[repr(C)]
struct OsVersionInfoA {
    dwOSVersionInfoSize: u32,
    dwMajorVersion: u32,
    dwMinorVersion: u32,
    dwBuildNumber: u32,
    dwPlatformId: u32,
    szCSDVersion: [c_char; 128],
}

static mut ARGV0: *mut c_char = ptr::null_mut();
static mut REALLY_EXITING: bool = false;

#[cfg(not(test))]
unsafe extern "C-unwind" {
    fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    fn lua_pushstring(L: *mut lua_State, s: *const c_char);
    fn lua_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
}

#[cfg(all(windows, not(test)))]
unsafe extern "system" {
    fn GetVersionExA(lpVersionInformation: *mut OsVersionInfoA) -> c_int;
}

#[cfg(not(test))]
unsafe extern "C" {
    static mut main_loop: Loop;

    static version_sys_vimrc_file: *const c_char;
    static main_exitfree: bool;

    fn vim_snprintf(str_: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn gettext(msgid: *const c_char) -> *mut c_char;

    fn uv_guess_handle(file: c_int) -> c_int;
    fn uv_strerror(err: c_int) -> *const c_char;

    fn fprintf(stream: *mut FILE, format: *const c_char, ...) -> c_int;
    fn printf(format: *const c_char, ...) -> c_int;
    fn setbuf(stream: *mut FILE, buffer: *mut c_char);

    fn main_get_stdout() -> *mut FILE;

    fn readfile(
        fname: *mut c_char,
        sfname: *mut c_char,
        from: linenr_T,
        lines_to_skip: linenr_T,
        lines_to_read: linenr_T,
        eap: *mut c_void,
        flags: c_int,
        silent: bool,
    ) -> c_int;
    fn open_buffer(read_stdin: bool, eap: *mut c_void, flags: c_int) -> c_int;
    fn invoke_all_defer();
    fn free_all_mem();

    #[cfg_attr(windows, link_name = "_dup2")]
    fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    #[cfg_attr(windows, link_name = "_close")]
    fn close(fd: c_int) -> c_int;
    #[cfg(windows)]
    fn _setmode(fd: c_int, mode: c_int) -> c_int;
    #[cfg(unix)]
    fn tcdrain(fd: c_int) -> c_int;
}

#[inline]
const fn ascii_isdigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'9' as c_int
}

#[inline]
unsafe fn buf_get_changedtick(buf: *const buf_T) -> varnumber_T {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

#[inline]
unsafe fn tv_list_set_lock(l: *mut crate::clipboard::list_T, lock: c_int) {
    if !l.is_null() {
        unsafe {
            (*l).lv_lock = lock;
        }
    }
}

#[inline]
unsafe fn time_msg_if_enabled(s: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        if !time_fd.is_null() {
            time_msg(s, ptr::null());
        }
    }
    #[cfg(test)]
    {
        let _ = s;
    }
}

#[inline]
unsafe fn to_slash_if_needed(p: *mut c_char) {
    #[cfg(windows)]
    unsafe {
        if !p.is_null() {
            path_to_slash(p);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = p;
    }
}

#[inline]
unsafe fn stricmp_ascii(a: *const c_char, b: &[u8]) -> bool {
    unsafe {
        if a.is_null() {
            return false;
        }
        CStr::from_ptr(a).to_bytes().eq_ignore_ascii_case(b)
    }
}

#[inline]
unsafe fn strnicmp_ascii(a: *const c_char, b: &[u8]) -> bool {
    unsafe {
        if a.is_null() {
            return false;
        }
        let bytes = CStr::from_ptr(a).to_bytes();
        bytes.len() >= b.len() && bytes[..b.len()].eq_ignore_ascii_case(b)
    }
}

#[inline]
const fn bool_obj(b: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
const fn int_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
unsafe fn cstr_obj(s: *const c_char) -> Object {
    unsafe {
        Object {
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData {
                string: cstr_as_string(s),
            },
        }
    }
}

#[inline]
unsafe fn gargcount() -> c_int {
    #[cfg(not(test))]
    unsafe {
        global_alist.al_ga.ga_len
    }
    #[cfg(test)]
    {
        0
    }
}

#[inline]
unsafe fn garglist() -> *mut aentry_T {
    #[cfg(not(test))]
    unsafe {
        global_alist.al_ga.ga_data.cast::<aentry_T>()
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn event_init() {
    #[cfg(not(test))]
    unsafe {
        loop_init(&raw mut main_loop, ptr::null_mut());
        env_init();
        let queues = loop_queues(&raw mut main_loop);
        resize_events = multiqueue_new_child((*queues).events);

        autocmd_init();
        signal_init();
        channel_init();
        terminal_init();
        ui_init();
        time_msg_if_enabled(c"event init".as_ptr());
    }
}

unsafe fn event_teardown() -> bool {
    #[cfg(not(test))]
    unsafe {
        let queues = loop_queues(&raw mut main_loop);
        if (*queues).events.is_null() {
            return true;
        }

        multiqueue_process_events((*queues).events);
        loop_poll_events(&raw mut main_loop, 0);
        server_teardown();
        channel_teardown();
        proc_teardown(&raw mut main_loop);
        timer_teardown();
        signal_teardown();
        terminal_teardown();

        loop_close(&raw mut main_loop, true)
    }
    #[cfg(test)]
    {
        true
    }
}

unsafe fn early_init(paramp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        os_hint_priority();
        estack_init();
        cmdline_init();
        eval_init();
        set_vim_var_nr(VV_STARTTIME, os_realtime() as varnumber_T);
        init_path(if !ARGV0.is_null() {
            ARGV0
        } else {
            c"nvim".as_ptr()
        });
        init_normal_cmds();
        runtime_init();
        highlight_init();
        mc_init();

        #[cfg(windows)]
        {
            let mut ovi: OsVersionInfoA = zeroed();
            ovi.dwOSVersionInfoSize = size_of::<OsVersionInfoA>() as u32;
            GetVersionExA(&raw mut ovi);
            let mut buf = Vec::with_capacity(20);
            let _ = write!(buf, "{}.{}\0", ovi.dwMajorVersion, ovi.dwMinorVersion);
            let copy_len = buf.len().min(20);
            ptr::copy_nonoverlapping(
                buf.as_ptr().cast::<c_char>(),
                (&raw mut windowsVersion).cast::<c_char>(),
                copy_len,
            );
            *((&raw mut windowsVersion).cast::<c_char>().add(19)) = 0;
        }

        time_msg_if_enabled(c"early init".as_ptr());

        init_locale();
        set_init_tablocal();

        win_alloc_first();
        time_msg_if_enabled(c"init first window".as_ptr());

        alist_init(&raw mut global_alist);
        global_alist.id = 0;

        init_homedir();
        set_init_1(if !paramp.is_null() {
            (*paramp).clean
        } else {
            false
        });
        log_init();
        time_msg_if_enabled(c"inits 1".as_ptr());

        set_lang_var();
        qf_init_stack();
    }
    #[cfg(test)]
    {
        let _ = paramp;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        ARGV0 = *argv;
        to_slash_if_needed(ARGV0);

        if !appname_is_valid() {
            fprintf(
                log_get_stderr(),
                c"$NVIM_APPNAME must be a name or relative path.\n".as_ptr(),
            );
            std::process::exit(1);
        }

        let mut fname: *mut c_char = ptr::null_mut();
        let mut params: mparm_T = zeroed();

        init_params(&raw mut params, argc, argv);
        init_startuptime(&raw mut params);

        for i in 1..params.argc {
            if stricmp_ascii(*params.argv.add(i as usize), b"--clean") {
                params.clean = true;
                break;
            }
        }

        event_init();
        early_init(&raw mut params);
        set_argv_var(argv, argc);
        check_and_set_isatty(&raw mut params);
        command_line_scan(&raw mut params);
        set_argf_var();

        nlua_init(argv, argc, params.lua_arg0);
        time_msg_if_enabled(c"init lua interpreter".as_ptr());

        #[cfg(windows)]
        let mut startup_stderr_fd: c_int = -1;
        #[cfg(windows)]
        if embedded_mode {
            startup_stderr_fd = os_dup_cloexec(STDERR_FILENO);
        }

        let stdin_type = uv_guess_handle(STDIN_FILENO);
        let stdio_rpc =
            params.listen_addr.is_null() || stdin_type == UV_NAMED_PIPE || stdin_type == UV_TCP;
        if embedded_mode && stdio_rpc {
            let mut err: *const c_char = ptr::null();
            let on_data: CallbackReader = zeroed();
            if channel_from_stdio(true, on_data, &raw mut err) == 0 {
                std::process::abort();
            }
        }

        if gargcount() > 0 {
            fname = get_fname(&raw mut params);
        }

        if recoverymode && fname.is_null() {
            headless_mode = true;
        }

        #[cfg(windows)]
        let has_term = true;
        #[cfg(not(windows))]
        let has_term = stdin_isatty || stdout_isatty || stderr_isatty;

        let use_builtin_ui = has_term && !headless_mode && !embedded_mode && !silent_mode;

        if params.remote != 0 {
            remote_request(
                &raw mut params,
                params.remote,
                params.server_addr,
                argc,
                argv,
                use_builtin_ui,
            );
        }

        let remote_ui = ui_client_channel_id != 0;

        if use_builtin_ui && !remote_ui {
            ui_client_forward_stdin = !stdin_isatty;
            let rv = ui_client_start_server(
                get_vim_var_str(VV_PROGPATH),
                params.argc as usize,
                params.argv,
            );
            if rv == 0 {
                fprintf(log_get_stderr(), c"Failed to start Nvim server!\n".as_ptr());
                os_exit(1);
            }
            ui_client_channel_id = rv;
        }

        if ui_client_channel_id != 0 {
            ui_client_run();
        }
        assert!(ui_client_channel_id == 0 && !use_builtin_ui);

        if !server_init(params.listen_addr) {
            #[cfg(windows)]
            if startup_stderr_fd >= 0 {
                dup2(startup_stderr_fd, STDERR_FILENO);
                close(startup_stderr_fd);
            }
            mainerr((&raw const IObuff).cast(), ptr::null(), ptr::null());
        }

        #[cfg(windows)]
        if startup_stderr_fd >= 0 {
            close(startup_stderr_fd);
        }

        time_msg_if_enabled(c"expanding arguments".as_ptr());

        if params.diff_mode != 0 && params.window_count == -1 {
            params.window_count = 0;
        }
        RedrawingDisabled += 1;

        setbuf(main_get_stdout(), ptr::null_mut());

        full_screen = !silent_mode;

        win_init_size();
        if params.diff_mode != 0 {
            diff_win_options(firstwin, false);
        }

        assert!(
            p_ch >= 0 && i64::from(Rows) >= p_ch && i64::from(Rows) - p_ch <= i64::from(c_int::MAX)
        );
        cmdline_row = Rows - p_ch as c_int;
        msg_row = cmdline_row;
        default_grid_alloc();
        set_init_2(headless_mode);
        time_msg_if_enabled(c"inits 2".as_ptr());

        msg_scroll = 1;
        no_wait_return += 1;

        init_highlight(true, false);
        ui_comp_syn_init();
        time_msg_if_enabled(c"init highlight".as_ptr());

        debug_break_level = params.use_debug_break_level;

        let use_remote_ui = embedded_mode && !headless_mode;
        if use_remote_ui {
            if !stdio_rpc {
                fprintf(
                    log_get_stderr(),
                    c"Nvim server (pid: %lld) waiting for UI, listening on: \"%s\"\n".as_ptr(),
                    os_get_pid() as std::ffi::c_longlong,
                    get_vim_var_str(VV_SEND_SERVER),
                );
            }
            time_msg_if_enabled(c"waiting for UI".as_ptr());
            remote_ui_wait_for_attach();
            time_msg_if_enabled(c"done waiting for UI".as_ptr());
            (*firstwin).w_prev_height = (*firstwin).w_height;
        }

        starting = NO_BUFFERS;
        screenclear();
        win_new_screensize();
        time_msg_if_enabled(c"clear screen".as_ptr());

        if edit_stdin(&raw mut params) {
            params.edit_type = EDIT_STDIN;
        }

        if !params.scriptin.is_null() && !open_scriptin(params.scriptin) {
            os_exit(2);
        }
        if !params.scriptout.is_null() {
            scriptout = os_fopen(
                params.scriptout,
                if params.scriptout_append {
                    c"ab".as_ptr()
                } else {
                    c"wb".as_ptr()
                },
            );
            if scriptout.is_null() {
                let stderr = log_get_stderr();
                fprintf(
                    stderr,
                    gettext(c"Cannot open for script output: \"".as_ptr()),
                );
                fprintf(stderr, c"%s\"\n".as_ptr(), params.scriptout);
                os_exit(2);
            }
        }

        nlua_init_defaults();
        time_msg_if_enabled(c"init default mappings & autocommands".as_ptr());

        let vimrc_none = strequal(params.use_vimrc, c"NONE".as_ptr());

        if vimrc_none && !params.clean {
            p_lpl = 0;
        }

        exe_pre_commands(&raw mut params);

        if !vimrc_none || params.clean {
            filetype_plugin_enable();
        }

        source_startup_scripts(&raw const params);

        if !vimrc_none || params.clean {
            filetype_maybe_enable();
            syn_maybe_enable();
        }

        set_vim_var_nr(VV_VIM_DID_INIT, 1);

        load_plugins();

        set_window_layout(&raw mut params);

        if recoverymode && fname.is_null() {
            let mut items_tv: crate::clipboard::typval_T = zeroed();
            tv_list_alloc_ret(&raw mut items_tv, 0);
            recover_names(ptr::null_mut(), false, items_tv.vval.v_list);
            let mut lua_args: [crate::clipboard::typval_T; 2] = [items_tv, zeroed()];
            nlua_call_typval(
                c"vim._core.swapfile".as_ptr(),
                c"list_swaps".as_ptr(),
                lua_args.as_mut_ptr(),
                ptr::null_mut(),
            );
            tv_clear(&raw mut items_tv);
            os_exit(0);
        }

        set_init_3();
        time_msg_if_enabled(c"inits 3".as_ptr());

        if params.no_swap_file != 0 {
            p_uc = 0;
        }

        if silent_mode {
            p_ut = 1;
        }

        if *p_shada != NUL {
            shada_read_everything(ptr::null(), false, true);
            time_msg_if_enabled(c"reading ShaDa".as_ptr());
        }
        if get_vim_var_list(VV_OLDFILES).is_null() {
            set_vim_var_list(VV_OLDFILES, tv_list_alloc(0));
        }

        handle_quickfix(&raw mut params);

        starting = NO_BUFFERS;
        no_wait_return = 0;
        msg_scroll = 0;

        if params.edit_type == EDIT_STDIN && !recoverymode {
            read_stdin();
        }

        setmouse();

        redraw_later(curwin, UPD_VALID);

        no_wait_return += 1;

        create_windows(&raw mut params);
        time_msg_if_enabled(c"opening buffers".as_ptr());

        set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);

        if params.exmode {
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
        }

        apply_autocmds(
            EVENT_BUFENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        time_msg_if_enabled(c"BufEnter autocommands".as_ptr());
        setpcmark();

        if params.edit_type == EDIT_QF {
            qf_jump(ptr::null_mut(), 0, 0, 0);
            time_msg_if_enabled(c"jump to first error".as_ptr());
        }

        edit_buffers(&raw mut params);

        if params.diff_mode != 0 {
            let mut wp = (*curtab).tp_firstwin;
            while !wp.is_null() {
                if !(*wp).w_arg_idx_invalid {
                    diff_win_options(wp, true);
                }
                wp = (*wp).w_next;
            }
        }

        shorten_fnames(0);

        handle_tag(params.tagname);

        if params.n_commands > 0 {
            exe_commands(&raw mut params);
        }

        starting = 0;

        RedrawingDisabled = 0;
        redraw_all_later(UPD_NOT_VALID);
        no_wait_return = 0;

        do_autochdir();

        set_vim_var_nr(VV_VIM_DID_ENTER, 1);
        apply_autocmds(
            EVENT_VIMENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        time_msg_if_enabled(c"VimEnter autocommands".as_ptr());
        if use_remote_ui {
            do_autocmd_uienter_all();
            time_msg_if_enabled(c"UIEnter autocommands".as_ptr());
        }

        #[cfg(windows)]
        {
            if use_remote_ui {
                os_icon_init();
            }
            os_title_save();
        }

        set_reg_var(get_default_register_name());

        if (*curwin).w_onebuf_opt.wo_diff != 0 && (*curwin).w_onebuf_opt.wo_scb != 0 {
            update_topline(curwin);
            check_scrollbind(0, 0);
            time_msg_if_enabled(c"diff scrollbinding".as_ptr());
        }

        if restart_edit != 0 {
            stuffcharReadbuff(K_NOP);
        }

        if (cb_flags & (kOptCbFlagUnnamed | kOptCbFlagUnnamedplus)) != 0 {
            eval_has_provider(c"clipboard".as_ptr(), false);
        }

        if !params.luaf.is_null() {
            msg_scroll = 1;
            logmsg_str(
                LOGLVL_DBG,
                ptr::null(),
                c"main".as_ptr(),
                line!() as c_int,
                true,
                c"executing Lua -l script".as_ptr(),
            );
            let lua_ok = nlua_exec_file(params.luaf);
            time_msg_if_enabled(c"executing Lua -l script".as_ptr());
            if msg_didout {
                msg_putchar(b'\n' as c_int);
                msg_didout = false;
            }
            getout(if lua_ok { 0 } else { 1 });
        } else if silent_mode {
            msg_scroll = 1;
            no_wait_return += 1;
            if !params.input_istext && !stdin_isatty {
                logmsg_str(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"main".as_ptr(),
                    line!() as c_int,
                    true,
                    c"executing stdin as Ex commands".as_ptr(),
                );
                let mut args: [crate::clipboard::typval_T; 1] = [zeroed()];
                nlua_call_typval(
                    c"vim._core.exmode".as_ptr(),
                    c"run".as_ptr(),
                    args.as_mut_ptr(),
                    ptr::null_mut(),
                );
            }
            if msg_didout {
                msg_putchar(b'\n' as c_int);
                msg_didout = false;
            }
            getout(0);
        }

        if params.exmode {
            do_cmdline_cmd(c"exmode".as_ptr());
        }

        time_msg_if_enabled(c"before starting main loop".as_ptr());
        logmsg_str(
            LOGLVL_INF,
            ptr::null(),
            c"main".as_ptr(),
            line!() as c_int,
            true,
            c"starting main loop".as_ptr(),
        );

        normal_enter();

        0
    }
    #[cfg(test)]
    {
        let _ = (argc, argv);
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_exit(mut r: c_int) -> ! {
    #[cfg(not(test))]
    unsafe {
        exiting = true;
        signal_reject_deadly();

        if ui_client_channel_id != 0 {
            ui_client_stop();
            if r == 0 {
                r = ui_client_exit_status;
            }
        } else {
            ui_flush();
        }

        if !event_teardown() && r == 0 {
            r = 1;
        }
        if ui_client_channel_id != 0 {
            #[cfg(unix)]
            {
                if stdout_isatty {
                    tcdrain(STDOUT_FILENO);
                }
                if stderr_isatty {
                    tcdrain(STDERR_FILENO);
                }
            }
        } else {
            ml_close_all(true);
        }

        let mut msg = [0u8; 32];
        let _ = write!(&mut msg[..], "Nvim exit: {r}\0");
        logmsg_str(
            LOGLVL_INF,
            ptr::null(),
            c"os_exit".as_ptr(),
            line!() as c_int,
            true,
            msg.as_ptr().cast(),
        );

        if main_exitfree {
            free_all_mem();
        }

        std::process::exit(r);
    }
    #[cfg(test)]
    {
        std::process::exit(r);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getout(mut exitval: c_int) -> ! {
    #[cfg(not(test))]
    unsafe {
        assert!(ui_client_channel_id == 0);
        exiting = true;

        time_finish();

        if exmode_active {
            exitval += ex_exitval;
        }

        set_vim_var_type(VV_EXITING, crate::clipboard::VarType::VAR_NUMBER);
        set_vim_var_nr(VV_EXITING, varnumber_T::from(exitval));

        if *get_vim_var_str(VV_EXITREASON) == NUL {
            set_vim_var_string(VV_EXITREASON, c"quit".as_ptr(), 4);
        }

        invoke_all_defer();
        hash_debug_results();

        if v_dying <= 1 {
            let mut next_tp: *const tabpage_T;
            let mut tp: *const tabpage_T = first_tabpage;
            while !tp.is_null() {
                next_tp = (*tp).tp_next;
                let mut wp = (*tp).tp_firstwin;
                while !wp.is_null() {
                    if (*wp).w_buffer.is_null() || !buf_valid((*wp).w_buffer) {
                        wp = (*wp).w_next;
                        continue;
                    }
                    let buf = (*wp).w_buffer;
                    if buf_get_changedtick(buf) != -1 {
                        let mut bufref: bufref_T = zeroed();
                        set_bufref(&raw mut bufref, buf);
                        apply_autocmds_win(
                            EVENT_BUFWINLEAVE,
                            (*buf).b_fname,
                            (*buf).b_fname,
                            false,
                            buf,
                            wp,
                        );
                        if bufref_valid(&raw mut bufref) {
                            buf_set_changedtick(buf, -1);
                        }
                        next_tp = first_tabpage;
                        break;
                    }
                    wp = (*wp).w_next;
                }
                tp = next_tp;
            }

            let mut buf = firstbuf;
            while !buf.is_null() {
                if !(*buf).b_ml.ml_mfp.is_null() {
                    let mut bufref: bufref_T = zeroed();
                    set_bufref(&raw mut bufref, buf);
                    apply_autocmds(EVENT_BUFUNLOAD, (*buf).b_fname, (*buf).b_fname, false, buf);
                    if !bufref_valid(&raw mut bufref) {
                        break;
                    }
                }
                buf = (*buf).b_next;
            }

            let mut unblock = 0;
            if is_autocmd_blocked() {
                unblock_autocmds();
                unblock += 1;
            }
            apply_autocmds(
                EVENT_VIMLEAVEPRE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            if unblock != 0 {
                block_autocmds();
            }
        }

        if !entered_free_all_mem && !p_shada.is_null() && *p_shada != NUL {
            shada_write_file(ptr::null(), false);
        }

        if v_dying <= 1 {
            let mut unblock = 0;
            if is_autocmd_blocked() {
                unblock_autocmds();
                unblock += 1;
            }
            apply_autocmds(
                EVENT_VIMLEAVE,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                curbuf,
            );
            if unblock != 0 {
                block_autocmds();
            }
        }

        profile_dump();

        if did_emsg != 0 {
            no_wait_return = 0;
            wait_return(0);
        }

        if p_title != 0 && *p_titleold != NUL {
            ui_call_set_title(cstr_as_string(p_titleold));
        }

        if garbage_collect_at_exit {
            garbage_collect(false);
        }

        #[cfg(windows)]
        {
            os_icon_reset();
            os_title_reset();
        }

        os_exit(exitval);
    }
    #[cfg(test)]
    {
        std::process::exit(exitval);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn preserve_exit(errmsg: *const c_char) -> ! {
    #[cfg(not(test))]
    unsafe {
        if REALLY_EXITING {
            std::process::exit(2);
        }
        REALLY_EXITING = true;
        exiting = true;
        signal_reject_deadly();

        if ui_client_channel_id != 0 {
            ui_client_stop();
        }
        let stderr = log_get_stderr();
        if !errmsg.is_null() && *errmsg != NUL {
            let bytes = CStr::from_ptr(errmsg).to_bytes();
            let has_eol = bytes.last() == Some(&b'\n');
            fprintf(
                stderr,
                if has_eol {
                    c"%s".as_ptr()
                } else {
                    c"%s\n".as_ptr()
                },
                errmsg,
            );
        }
        if ui_client_channel_id != 0 {
            os_exit(1);
        }

        ml_close_notmod();

        let mut buf = firstbuf;
        while !buf.is_null() {
            let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
            if !mfp.is_null() && !(*mfp).mf_fname.is_null() {
                if !errmsg.is_null() {
                    fprintf(stderr, c"Nvim: preserving files...\n".as_ptr());
                }
                ml_sync_all(0, 0, true);
                break;
            }
            buf = (*buf).b_next;
        }

        ml_close_all(false);

        if !errmsg.is_null() {
            fprintf(stderr, c"Nvim: Finished.\n".as_ptr());
        }

        getout(1);
    }
    #[cfg(test)]
    {
        let _ = errmsg;
        std::process::exit(1);
    }
}

unsafe fn get_number_arg(p: *const c_char, idx: *mut c_int, mut def: c_int) -> c_int {
    unsafe {
        let mut i = *idx as usize;
        if ascii_isdigit(*p.add(i) as c_int) {
            def = 0;
            while ascii_isdigit(*p.add(i) as c_int) {
                def = def
                    .wrapping_mul(10)
                    .wrapping_add((*p.add(i) as u8 - b'0') as c_int);
                i += 1;
            }
            *idx = i as c_int;
        }
        def
    }
}

unsafe fn server_connect(server_addr: *const c_char, errmsg: *mut *const c_char) -> u64 {
    #[cfg(not(test))]
    unsafe {
        if server_addr.is_null() {
            *errmsg = c"no address specified".as_ptr();
            return 0;
        }
        let on_data: CallbackReader = zeroed();
        let mut error: *const c_char = ptr::null();
        let is_tcp = !socket_address_tcp_host_end(server_addr).is_null();
        let chan = channel_connect(is_tcp, server_addr, true, on_data, 500, &raw mut error);
        if !error.is_null() {
            *errmsg = error;
            return 0;
        }
        chan
    }
    #[cfg(test)]
    {
        let _ = (server_addr, errmsg);
        0
    }
}

unsafe fn remote_request(
    params: *mut mparm_T,
    remote_args: c_int,
    server_addr: *mut c_char,
    argc: c_int,
    argv: *mut *mut c_char,
    ui_only: bool,
) {
    #[cfg(not(test))]
    unsafe {
        let is_ui = strequal(*argv.add(remote_args as usize), c"--remote-ui".as_ptr());
        if ui_only && !is_ui {
            return;
        }

        let stderr = log_get_stderr();
        let mut connect_error: *const c_char = ptr::null();
        let chan = server_connect(server_addr, &raw mut connect_error);

        if is_ui {
            if chan == 0 {
                #[cfg(windows)]
                {
                    dup2(STDOUT_FILENO, STDERR_FILENO);
                }
                fprintf(
                    stderr,
                    c"Remote ui failed to start: %s\n".as_ptr(),
                    connect_error,
                );
                os_exit(1);
            } else if strequal(server_addr, os_getenv_noalloc(c"NVIM".as_ptr())) {
                fprintf(
                    stderr,
                    c"%s".as_ptr(),
                    c"Cannot attach UI of :terminal child to its parent. ".as_ptr(),
                );
                fprintf(
                    stderr,
                    c"%s\n".as_ptr(),
                    c"(Unset $NVIM to skip this check)".as_ptr(),
                );
                os_exit(1);
            }
            ui_client_channel_id = chan;
            return;
        }

        let count = (argc - remote_args).max(0) as usize;
        let args_items = if count > 0 {
            xmalloc(count * size_of::<Object>()).cast::<Object>()
        } else {
            ptr::null_mut()
        };
        for (idx, t_argc) in (remote_args..argc).enumerate() {
            *args_items.add(idx) = cstr_obj(*argv.add(t_argc as usize));
        }
        let args = Array {
            items: args_items,
            size: count,
            capacity: count,
        };

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut a_items: [Object; 4] = [
            int_obj(Integer::from(chan as c_int)),
            cstr_obj(server_addr),
            cstr_obj(connect_error),
            Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData { array: args },
            },
        ];
        let a = Array {
            items: a_items.as_mut_ptr(),
            size: 4,
            capacity: 4,
        };
        let s = NvimString {
            data: c"return vim._cs_remote(...)".as_ptr().cast_mut(),
            size: 26,
        };
        let o = nlua_exec(s, ptr::null(), a, kRetObject, ptr::null_mut(), &raw mut err);
        xfree(args.items.cast());
        if err.err_type != ErrorType::kErrorTypeNone {
            fprintf(stderr, c"%s\n".as_ptr(), err.msg);
            os_exit(2);
        }

        if o.obj_type != ObjectType::kObjectTypeDict {
            fprintf(
                stderr,
                c"vim._cs_remote returned unexpected value\n".as_ptr(),
            );
            os_exit(2);
        }
        let dict: Dict = o.data.dict;

        let mut should_exit: TriState = kNone;
        let mut tabbed: TriState = kNone;

        for i in 0..dict.size {
            let item = &*dict.items.add(i);
            if strequal(item.key.data, c"errmsg".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeString {
                    fprintf(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'errmsg'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                fprintf(stderr, c"%s\n".as_ptr(), item.value.data.string.data);
                os_exit(2);
            } else if strequal(item.key.data, c"result".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeString {
                    fprintf(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'result'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                printf(c"%s".as_ptr(), item.value.data.string.data);
            } else if strequal(item.key.data, c"tabbed".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeBoolean {
                    fprintf(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'tabbed'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                tabbed = if item.value.data.boolean {
                    kTrue
                } else {
                    kFalse
                };
            } else if strequal(item.key.data, c"should_exit".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeBoolean {
                    fprintf(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'should_exit'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                should_exit = if item.value.data.boolean {
                    kTrue
                } else {
                    kFalse
                };
            }
        }
        if should_exit == kNone || tabbed == kNone {
            fprintf(
                stderr,
                c"vim._cs_remote didn't return a value for should_exit or tabbed, bailing\n"
                    .as_ptr(),
            );
            os_exit(2);
        }
        api_free_object(o);

        if should_exit == kTrue {
            os_exit(0);
        }
        if tabbed == kTrue {
            (*params).window_count = argc - remote_args - 1;
            (*params).window_layout = WIN_TABS;
        }
    }
    #[cfg(test)]
    {
        let _ = (params, remote_args, server_addr, argc, argv, ui_only);
    }
}

unsafe fn edit_stdin(parmp: *mut mparm_T) -> bool {
    #[cfg(not(test))]
    unsafe {
        let implicit = !headless_mode
            && !(embedded_mode && stdin_fd <= 0)
            && (!(*parmp).exmode || (*parmp).input_istext)
            && !stdin_isatty
            && (*parmp).edit_type <= EDIT_STDIN
            && (*parmp).scriptin.is_null();
        (*parmp).had_stdin_file || implicit
    }
    #[cfg(test)]
    {
        let _ = parmp;
        false
    }
}

unsafe fn command_line_scan(parmp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        let mut argc = (*parmp).argc - 1;
        let mut argv = (*parmp).argv.add(1);
        let mut argv_idx: c_int = 1;
        let mut had_minmin = false;

        while argc > 0 {
            let arg0 = *argv;
            if *arg0 == b'+' as c_char && !had_minmin {
                if (*parmp).n_commands as usize >= MAX_ARG_CMDS {
                    mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                }
                argv_idx = -1;
                let idx = (*parmp).n_commands as usize;
                (*parmp).n_commands += 1;
                if *arg0.add(1) == NUL {
                    (*parmp).commands[idx] = c"$".as_ptr().cast_mut();
                } else {
                    (*parmp).commands[idx] = arg0.add(1);
                }
            } else if *arg0 == b'-' as c_char && !had_minmin {
                let mut want_argument = false;
                let c = *arg0.add(argv_idx as usize) as u8;
                argv_idx += 1;
                match c {
                    0 => {
                        if (*parmp).exmode {
                            silent_mode = true;
                            exmode_active = true;
                            (*parmp).no_swap_file = 1;
                        } else {
                            if (*parmp).edit_type > EDIT_STDIN {
                                mainerr(ERR_TOO_MANY_ARGS.as_ptr(), arg0, ptr::null());
                            }
                            (*parmp).had_stdin_file = true;
                            (*parmp).edit_type = EDIT_STDIN;
                        }
                        argv_idx = -1;
                    }
                    b'-' => {
                        let sub = arg0.add(argv_idx as usize);
                        if stricmp_ascii(sub, b"help") {
                            usage();
                            os_exit(0);
                        } else if stricmp_ascii(sub, b"version") {
                            version();
                            os_exit(0);
                        } else if stricmp_ascii(sub, b"api-info") {
                            #[cfg(windows)]
                            {
                                _setmode(STDOUT_FILENO, 0x8000);
                            }
                            let data = api_metadata_raw();
                            let written_bytes =
                                os_write(STDOUT_FILENO, data.data, data.size, false);
                            if written_bytes < 0 {
                                semsg!(
                                    gettext(c"E5420: Failed to write to file: %s".as_ptr()),
                                    uv_strerror(written_bytes as c_int),
                                );
                            }
                            os_exit(0);
                        } else if stricmp_ascii(sub, b"headless") {
                            headless_mode = true;
                        } else if stricmp_ascii(sub, b"embed") {
                            embedded_mode = true;
                        } else if strnicmp_ascii(sub, b"listen") {
                            want_argument = true;
                            argv_idx += 6;
                        } else if strnicmp_ascii(sub, b"literal") {
                            // Do nothing: file args are always literal.
                        } else if strnicmp_ascii(sub, b"remote") {
                            (*parmp).remote = (*parmp).argc - argc;
                        } else if strnicmp_ascii(sub, b"server") {
                            want_argument = true;
                            argv_idx += 6;
                        } else if strnicmp_ascii(sub, b"noplugin") {
                            p_lpl = 0;
                        } else if strnicmp_ascii(sub, b"cmd") {
                            want_argument = true;
                            argv_idx += 3;
                        } else if strnicmp_ascii(sub, b"startuptime") {
                            want_argument = true;
                            argv_idx += 11;
                        } else if strnicmp_ascii(sub, b"clean") {
                            (*parmp).use_vimrc = c"NONE".as_ptr().cast_mut();
                            (*parmp).clean = true;
                            set_option_value_give_err(kOptShadafile, cstr_obj(c"NONE".as_ptr()), 0);
                        } else if strnicmp_ascii(sub, b"luamod-dev") {
                            nlua_disable_preload = true;
                        } else {
                            if *sub != NUL {
                                mainerr(ERR_OPT_UNKNOWN.as_ptr(), arg0, ptr::null());
                            }
                            had_minmin = true;
                        }
                        if !want_argument {
                            argv_idx = -1;
                        }
                    }
                    b'A' => {
                        set_option_value_give_err(kOptArabic, bool_obj(true), 0);
                    }
                    b'b' => {
                        set_options_bin((*curbuf).b_p_bin, 1, 0);
                        (*curbuf).b_p_bin = 1;
                    }
                    b'D' => {
                        (*parmp).use_debug_break_level = 9999;
                    }
                    b'd' => {
                        (*parmp).diff_mode = 1;
                    }
                    b'e' => {
                        (*parmp).exmode = true;
                    }
                    b'E' => {
                        (*parmp).exmode = true;
                        (*parmp).input_istext = true;
                    }
                    b'f' | b'N' | b'X' => {}
                    b'?' | b'h' => {
                        usage();
                        os_exit(0);
                    }
                    b'H' => {
                        set_option_value_give_err(kOptKeymap, cstr_obj(c"hebrew".as_ptr()), 0);
                        set_option_value_give_err(kOptRightleft, bool_obj(true), 0);
                    }
                    b'M' => {
                        reset_modifiable();
                        p_write = 0;
                    }
                    b'm' => {
                        p_write = 0;
                    }
                    b'n' => {
                        (*parmp).no_swap_file = 1;
                    }
                    b'p' => {
                        (*parmp).window_count = get_number_arg(arg0, &raw mut argv_idx, 0);
                        (*parmp).window_layout = WIN_TABS;
                    }
                    b'o' => {
                        (*parmp).window_count = get_number_arg(arg0, &raw mut argv_idx, 0);
                        (*parmp).window_layout = WIN_HOR;
                    }
                    b'O' => {
                        (*parmp).window_count = get_number_arg(arg0, &raw mut argv_idx, 0);
                        (*parmp).window_layout = WIN_VER;
                    }
                    b'q' => {
                        if (*parmp).edit_type != EDIT_NONE {
                            mainerr(ERR_TOO_MANY_ARGS.as_ptr(), arg0, ptr::null());
                        }
                        (*parmp).edit_type = EDIT_QF;
                        if *arg0.add(argv_idx as usize) != NUL {
                            (*parmp).use_ef = arg0.add(argv_idx as usize);
                            argv_idx = -1;
                        } else if argc > 1 {
                            want_argument = true;
                        }
                    }
                    b'R' => {
                        readonlymode = true;
                        (*curbuf).b_p_ro = 1;
                        p_uc = 10000;
                    }
                    b'r' | b'L' => {
                        recoverymode = true;
                    }
                    b's' => {
                        if (*parmp).exmode {
                            silent_mode = true;
                            exmode_active = true;
                            (*parmp).no_swap_file = 1;
                            if p_shadafile.is_null() || *p_shadafile == NUL {
                                set_option_value_give_err(
                                    kOptShadafile,
                                    cstr_obj(c"NONE".as_ptr()),
                                    0,
                                );
                            }
                        } else {
                            want_argument = true;
                        }
                    }
                    b't' => {
                        if (*parmp).edit_type != EDIT_NONE {
                            mainerr(ERR_TOO_MANY_ARGS.as_ptr(), arg0, ptr::null());
                        }
                        (*parmp).edit_type = EDIT_TAG;
                        if *arg0.add(argv_idx as usize) != NUL {
                            (*parmp).tagname = arg0.add(argv_idx as usize);
                            argv_idx = -1;
                        } else {
                            want_argument = true;
                        }
                    }
                    b'v' => {
                        version();
                        os_exit(0);
                    }
                    b'V' => {
                        p_verbose = i64::from(get_number_arg(arg0, &raw mut argv_idx, 10));
                        if *arg0.add(argv_idx as usize) != NUL {
                            set_option_value_give_err(
                                kOptVerbosefile,
                                cstr_obj(arg0.add(argv_idx as usize)),
                                0,
                            );
                            argv_idx = CStr::from_ptr(arg0).to_bytes().len() as c_int;
                        }
                    }
                    b'w' => {
                        if ascii_isdigit(*arg0.add(argv_idx as usize) as c_int) {
                            let n = get_number_arg(arg0, &raw mut argv_idx, 10);
                            set_option_value_give_err(kOptWindow, int_obj(OptInt::from(n)), 0);
                        } else {
                            want_argument = true;
                        }
                    }
                    b'c' => {
                        if *arg0.add(argv_idx as usize) != NUL {
                            if (*parmp).n_commands as usize >= MAX_ARG_CMDS {
                                mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                            }
                            let idx = (*parmp).n_commands as usize;
                            (*parmp).n_commands += 1;
                            (*parmp).commands[idx] = arg0.add(argv_idx as usize);
                            argv_idx = -1;
                        } else {
                            want_argument = true;
                        }
                    }
                    b'S' | b'i' | b'l' | b'u' | b'U' | b'W' => {
                        want_argument = true;
                    }
                    _ => {
                        mainerr(ERR_OPT_UNKNOWN.as_ptr(), arg0, ptr::null());
                    }
                }

                if want_argument {
                    if *arg0.add(argv_idx as usize) != NUL {
                        mainerr(ERR_OPT_GARBAGE.as_ptr(), arg0, ptr::null());
                    }

                    argc -= 1;
                    if argc < 1 && c != b'S' {
                        mainerr(ERR_ARG_MISSING.as_ptr(), arg0, ptr::null());
                    }
                    argv = argv.add(1);
                    argv_idx = -1;

                    let cur_arg = if argc >= 1 { *argv } else { ptr::null_mut() };
                    let prev_arg = *argv.sub(1);

                    match c {
                        b'c' | b'S' => {
                            if (*parmp).n_commands as usize >= MAX_ARG_CMDS {
                                mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                            }
                            if c == b'S' {
                                let a: *const c_char = if argc < 1 {
                                    c"Session.vim".as_ptr()
                                } else if *cur_arg == b'-' as c_char {
                                    argc += 1;
                                    argv = argv.sub(1);
                                    c"Session.vim".as_ptr()
                                } else {
                                    cur_arg
                                };

                                let a_bytes = CStr::from_ptr(a).to_bytes();
                                let s_size = a_bytes.len() + 9;
                                let s = xmalloc(s_size).cast::<u8>();
                                ptr::copy_nonoverlapping(b"so ".as_ptr(), s, 3);
                                ptr::copy_nonoverlapping(a_bytes.as_ptr(), s.add(3), a_bytes.len());
                                *s.add(3 + a_bytes.len()) = 0;

                                let idx = (*parmp).n_commands as usize;
                                (*parmp).cmds_tofree[idx] = 1;
                                (*parmp).commands[idx] = s.cast();
                                (*parmp).n_commands += 1;
                            } else {
                                let idx = (*parmp).n_commands as usize;
                                (*parmp).commands[idx] = cur_arg;
                                (*parmp).n_commands += 1;
                            }
                        }
                        b'-' => {
                            if strequal(prev_arg, c"--cmd".as_ptr()) {
                                if (*parmp).n_pre_commands as usize >= MAX_ARG_CMDS {
                                    mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                                }
                                let idx = (*parmp).n_pre_commands as usize;
                                (*parmp).pre_commands[idx] = cur_arg;
                                (*parmp).n_pre_commands += 1;
                            } else if strequal(prev_arg, c"--listen".as_ptr()) {
                                to_slash_if_needed(cur_arg);
                                (*parmp).listen_addr = cur_arg;
                            } else if strequal(prev_arg, c"--server".as_ptr()) {
                                to_slash_if_needed(cur_arg);
                                (*parmp).server_addr = cur_arg;
                            }
                        }
                        b'q' => {
                            (*parmp).use_ef = cur_arg;
                        }
                        b'i' => {
                            set_option_value_give_err(kOptShadafile, cstr_obj(cur_arg), 0);
                        }
                        b'l' => {
                            headless_mode = true;
                            silent_mode = true;
                            p_verbose = 1;
                            (*parmp).no_swap_file = 1;
                            if (*parmp).use_vimrc.is_null() {
                                (*parmp).use_vimrc = c"NONE".as_ptr().cast_mut();
                            }
                            if p_shadafile.is_null() || *p_shadafile == NUL {
                                set_option_value_give_err(
                                    kOptShadafile,
                                    cstr_obj(c"NONE".as_ptr()),
                                    0,
                                );
                            }
                            (*parmp).luaf = cur_arg;
                            argc -= 1;
                            if argc >= 0 {
                                (*parmp).lua_arg0 = (*parmp).argc - argc;
                                argc = 0;
                            }
                        }
                        b's' => {
                            if !(*parmp).scriptin.is_null() {
                                vim_snprintf(
                                    (&raw mut IObuff).cast(),
                                    IOSIZE,
                                    gettext(
                                        c"Attempt to open script file again: \"%s %s\"\n".as_ptr(),
                                    ),
                                    prev_arg,
                                    cur_arg,
                                );
                                fprintf(
                                    log_get_stderr(),
                                    c"%s".as_ptr(),
                                    (&raw const IObuff).cast::<c_char>(),
                                );
                                os_exit(2);
                            }
                            (*parmp).scriptin = cur_arg;
                        }
                        b't' => {
                            (*parmp).tagname = cur_arg;
                        }
                        b'u' => {
                            (*parmp).use_vimrc = cur_arg;
                            to_slash_if_needed(cur_arg);
                        }
                        b'U' => {}
                        b'w' | b'W' => {
                            if c == b'w' && ascii_isdigit(*cur_arg as c_int) {
                                argv_idx = 0;
                                let n = get_number_arg(cur_arg, &raw mut argv_idx, 10);
                                set_option_value_give_err(kOptWindow, int_obj(OptInt::from(n)), 0);
                                argv_idx = -1;
                            } else {
                                if !(*parmp).scriptout.is_null() {
                                    vim_snprintf(
                                        (&raw mut IObuff).cast(),
                                        IOSIZE,
                                        gettext(
                                            c"Attempt to open script file again: \"%s %s\"\n"
                                                .as_ptr(),
                                        ),
                                        prev_arg,
                                        cur_arg,
                                    );
                                    fprintf(
                                        log_get_stderr(),
                                        c"%s".as_ptr(),
                                        (&raw const IObuff).cast::<c_char>(),
                                    );
                                    os_exit(2);
                                }
                                (*parmp).scriptout = cur_arg;
                                (*parmp).scriptout_append = c == b'w';
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                argv_idx = -1;

                if (*parmp).edit_type > EDIT_STDIN {
                    mainerr(ERR_TOO_MANY_ARGS.as_ptr(), arg0, ptr::null());
                }
                (*parmp).edit_type = EDIT_FILE;

                ga_grow(&raw mut global_alist.al_ga, 1);
                let mut p = xstrdup(arg0);

                #[cfg(windows)]
                {
                    to_slash_if_needed(p);
                    if *p == b'~' as c_char && *p.add(1) == b'/' as c_char {
                        let homedir = os_homedir();
                        let h_bytes = CStr::from_ptr(homedir).to_bytes();
                        let p_bytes = CStr::from_ptr(p.add(1)).to_bytes();
                        let size = h_bytes.len() + p_bytes.len() + 1;
                        let tilde_expanded = xmalloc(size).cast::<u8>();
                        ptr::copy_nonoverlapping(h_bytes.as_ptr(), tilde_expanded, h_bytes.len());
                        ptr::copy_nonoverlapping(
                            p_bytes.as_ptr(),
                            tilde_expanded.add(h_bytes.len()),
                            p_bytes.len(),
                        );
                        *tilde_expanded.add(h_bytes.len() + p_bytes.len()) = 0;
                        xfree(p.cast());
                        p = tilde_expanded.cast();
                    }
                }

                if (*parmp).diff_mode != 0
                    && os_isdir(p)
                    && gargcount() > 0
                    && !os_isdir(alist_name(garglist()))
                {
                    let tail = path_tail(alist_name(garglist()));
                    let ret = concat_fnames(cstr_as_string(p), cstr_as_string(tail), true);
                    xfree(p.cast());
                    p = ret.data;
                }

                if CASE_INSENSITIVE_FILENAME {
                    path_fix_case(p);
                }

                let alist_fnum_flag = if edit_stdin(parmp) { 1 } else { 2 };
                alist_add(&raw mut global_alist, p, alist_fnum_flag);
            }

            if argv_idx <= 0 || *(*argv).add(argv_idx as usize) == NUL {
                argc -= 1;
                argv = argv.add(1);
                argv_idx = 1;
            }
        }

        if embedded_mode && (silent_mode || !(*parmp).luaf.is_null()) {
            mainerr(
                gettext(c"--embed conflicts with -es/-Es/-l".as_ptr()),
                ptr::null(),
                ptr::null(),
            );
        }

        if (*parmp).n_commands > 0 {
            let cmd0_bytes = CStr::from_ptr((*parmp).commands[0]).to_bytes();
            let swcmd_len = cmd0_bytes.len() + 2;
            let swcmd = xmalloc(swcmd_len + 1).cast::<u8>();
            *swcmd = b':';
            ptr::copy_nonoverlapping(cmd0_bytes.as_ptr(), swcmd.add(1), cmd0_bytes.len());
            *swcmd.add(1 + cmd0_bytes.len()) = b'\r';
            *swcmd.add(swcmd_len) = 0;
            set_vim_var_string(VV_SWAPCOMMAND, swcmd.cast(), swcmd_len as isize);
            xfree(swcmd.cast());
        }

        time_msg_if_enabled(c"parsing arguments".as_ptr());
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn set_argf_var() {
    #[cfg(not(test))]
    unsafe {
        let list = tv_list_alloc(kListLenMayKnow);
        let count = gargcount();
        for i in 0..count {
            let fname = alist_name(garglist().add(i as usize));
            if !fname.is_null() {
                let _ = vim_FullName(fname, (&raw mut NameBuff).cast(), MAXPATHL, false);
                tv_list_append_string(list, (&raw const NameBuff).cast(), -1);
            }
        }
        tv_list_set_lock(list, VAR_FIXED);
        set_vim_var_list(VV_ARGF, list);
    }
}

unsafe fn init_params(paramp: *mut mparm_T, argc: c_int, argv: *mut *mut c_char) {
    unsafe {
        *paramp = zeroed();
        (*paramp).argc = argc;
        (*paramp).argv = argv;
        (*paramp).use_debug_break_level = -1;
        (*paramp).window_count = -1;
        (*paramp).listen_addr = ptr::null_mut();
        (*paramp).server_addr = ptr::null_mut();
        (*paramp).remote = 0;
        (*paramp).luaf = ptr::null_mut();
        (*paramp).lua_arg0 = -1;
    }
}

unsafe fn init_startuptime(paramp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        let mut is_embed = false;
        for i in 1..((*paramp).argc - 1) {
            if stricmp_ascii(*(*paramp).argv.add(i as usize), b"--embed") {
                is_embed = true;
                break;
            }
        }
        for i in 1..((*paramp).argc - 1) {
            if stricmp_ascii(*(*paramp).argv.add(i as usize), b"--startuptime") {
                time_init(
                    *(*paramp).argv.add((i + 1) as usize),
                    if is_embed {
                        c"Embedded".as_ptr()
                    } else {
                        c"Primary (or UI client)".as_ptr()
                    },
                );
                time_start(c"--- NVIM STARTING ---".as_ptr());
                break;
            }
        }
    }
    #[cfg(test)]
    {
        let _ = paramp;
    }
}

unsafe fn check_and_set_isatty(_paramp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        stdin_isatty = os_isatty(STDIN_FILENO);
        stdout_isatty = os_isatty(STDOUT_FILENO);
        stderr_isatty = os_isatty(STDERR_FILENO);
        time_msg_if_enabled(c"window checked".as_ptr());
    }
}

unsafe fn init_path(exename: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        let mut exepath = [0 as c_char; MAXPATHL];
        let mut exepathlen: usize = MAXPATHL;
        if os_exepath(exepath.as_mut_ptr(), &raw mut exepathlen) != 0 {
            path_guess_exepath(exename, exepath.as_mut_ptr(), exepath.len());
        }
        set_vim_var_string(VV_PROGPATH, exepath.as_ptr(), -1);
        set_vim_var_string(VV_PROGNAME, path_tail(exename), -1);

        #[cfg(windows)]
        {
            os_setenv_append_path(exepath.as_ptr());
        }
    }
    #[cfg(test)]
    {
        let _ = exename;
    }
}

unsafe fn get_fname(_parmp: *mut mparm_T) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        alist_name(garglist())
    }
    #[cfg(test)]
    {
        ptr::null_mut()
    }
}

unsafe fn set_window_layout(paramp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        if (*paramp).diff_mode != 0 && (*paramp).window_layout == 0 {
            (*paramp).window_layout = if diffopt_horizontal() {
                WIN_HOR
            } else {
                WIN_VER
            };
        }
    }
    #[cfg(test)]
    {
        let _ = paramp;
    }
}

unsafe fn handle_quickfix(paramp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        if (*paramp).edit_type == EDIT_QF {
            if !(*paramp).use_ef.is_null() {
                set_option_direct(kOptErrorfile, cstr_obj((*paramp).use_ef), 0, SID_CARG);
            }
            vim_snprintf((&raw mut IObuff).cast(), IOSIZE, c"cfile %s".as_ptr(), p_ef);
            if qf_init(
                ptr::null_mut(),
                p_ef,
                p_efm,
                1,
                (&raw const IObuff).cast(),
                p_menc,
            ) < 0
            {
                msg_putchar(b'\n' as c_int);
                os_exit(3);
            }
            time_msg_if_enabled(c"reading errorfile".as_ptr());
        }
    }
    #[cfg(test)]
    {
        let _ = paramp;
    }
}

unsafe fn handle_tag(tagname: *mut c_char) {
    #[cfg(not(test))]
    unsafe {
        if !tagname.is_null() {
            swap_exists_did_quit = false;
            vim_snprintf((&raw mut IObuff).cast(), IOSIZE, c"ta %s".as_ptr(), tagname);
            do_cmdline_cmd((&raw const IObuff).cast());
            time_msg_if_enabled(c"jumping to tag".as_ptr());

            if swap_exists_did_quit {
                ui_call_error_exit(1);
                getout(1);
            }
        }
    }
    #[cfg(test)]
    {
        let _ = tagname;
    }
}

unsafe fn read_stdin() {
    #[cfg(not(test))]
    unsafe {
        swap_exists_action = SEA_DIALOG;
        no_wait_return = 1;
        let save_msg_didany = msg_didany;

        if !(*curbuf).b_ffname.is_null() {
            let stdin_buf = buflist_new(ptr::null_mut(), ptr::null_mut(), 0, BLN_LISTED);
            if stdin_buf.is_null() {
                semsg!(c"Failed to create buffer for stdin".as_ptr());
                return;
            }

            let initial_buf_handle: handle_T = (*curbuf).b_fnum;
            set_curbuf(stdin_buf, 0, false);
            readfile(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                MAXLNUM,
                ptr::null_mut(),
                READ_NEW + READ_STDIN,
                true,
            );

            let stdin_buf_handle: handle_T = (*stdin_buf).b_fnum;
            let stdin_buf_empty = buf_is_empty(curbuf);

            let mut buf = [0 as c_char; 100];
            vim_snprintf(
                buf.as_mut_ptr(),
                buf.len(),
                c"silent! buffer %d".as_ptr(),
                initial_buf_handle,
            );
            do_cmdline_cmd(buf.as_ptr());

            if stdin_buf_empty {
                vim_snprintf(
                    buf.as_mut_ptr(),
                    buf.len(),
                    c"silent! bwipeout! %d".as_ptr(),
                    stdin_buf_handle,
                );
                do_cmdline_cmd(buf.as_ptr());
            }
        } else {
            set_buflisted(1);
            open_buffer(true, ptr::null_mut(), 0);
            if buf_is_empty(curbuf) && !(*curbuf).b_next.is_null() {
                do_cmdline_cmd(c"silent! bnext".as_ptr());
                do_cmdline_cmd(c"silent! bwipeout 1".as_ptr());
            }
        }

        no_wait_return = 0;
        msg_didany = save_msg_didany;
        time_msg_if_enabled(c"reading stdin".as_ptr());
        check_swap_exists_action();
    }
}

unsafe fn create_windows(parmp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        if (*parmp).window_count == -1 {
            (*parmp).window_count = 1;
        }
        if (*parmp).window_count == 0 {
            (*parmp).window_count = gargcount();
        }
        if (*parmp).window_count > 1 {
            if (*parmp).window_layout == 0 {
                (*parmp).window_layout = WIN_HOR;
            }
            if (*parmp).window_layout == WIN_TABS {
                (*parmp).window_count = make_tabpages((*parmp).window_count);
                time_msg_if_enabled(c"making tab pages".as_ptr());
            } else if (*firstwin).w_next.is_null() || (*(*firstwin).w_next).w_floating {
                (*parmp).window_count =
                    make_windows((*parmp).window_count, (*parmp).window_layout == WIN_VER);
                time_msg_if_enabled(c"making windows".as_ptr());
            } else {
                (*parmp).window_count = win_count();
            }
        } else {
            (*parmp).window_count = 1;
        }

        if recoverymode {
            msg_scroll = 1;
            ml_recover(true);
            if (*curbuf).b_ml.ml_mfp.is_null() {
                getout(1);
            }
            do_modelines(0);
        } else {
            let mut done: c_int = 0;
            autocmd_no_enter += 1;
            autocmd_no_leave += 1;
            let startup_curwin = curwin;
            let mut dorewind = true;
            while done < 1000 {
                done += 1;
                if dorewind {
                    if (*parmp).window_layout == WIN_TABS {
                        goto_tabpage(1);
                    } else {
                        curwin = firstwin;
                    }
                } else if (*parmp).window_layout == WIN_TABS {
                    if (*curtab).tp_next.is_null() {
                        break;
                    }
                    goto_tabpage(0);
                } else {
                    if (*curwin).w_next.is_null() {
                        break;
                    }
                    curwin = (*curwin).w_next;
                }
                dorewind = false;
                curbuf = (*curwin).w_buffer;
                if (*curbuf).b_ml.ml_mfp.is_null() {
                    if p_fdls >= 0 {
                        (*curwin).w_onebuf_opt.wo_fdl = p_fdls;
                    }
                    swap_exists_action = SEA_DIALOG;
                    set_buflisted(1);

                    open_buffer(false, ptr::null_mut(), 0);

                    if swap_exists_action == SEA_QUIT {
                        if got_int || only_one_window() {
                            did_emsg = 0;
                            ui_call_error_exit(1);
                            getout(1);
                        }
                        setfname(curbuf, ptr::null_mut(), ptr::null_mut(), false);
                        (*curwin).w_arg_idx = -1;
                        swap_exists_action = SEA_NONE;
                    } else {
                        handle_swap_exists(ptr::null_mut());
                    }
                    dorewind = true;
                }
                os_breakcheck();
                if got_int {
                    vgetc();
                    break;
                }
            }
            if (*parmp).window_layout == WIN_TABS {
                goto_tabpage(1);
            } else if (*parmp).window_count > 1 || !win_valid(startup_curwin) {
                curwin = firstwin;
            } else {
                curwin = startup_curwin;
            }
            curbuf = (*curwin).w_buffer;
            autocmd_no_enter -= 1;
            autocmd_no_leave -= 1;
        }
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn edit_buffers(parmp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        let mut advance = true;
        let mut p_shm_save: *mut c_char = ptr::null_mut();

        autocmd_no_enter += 1;
        autocmd_no_leave += 1;

        if (*curwin).w_arg_idx == -1 {
            win_close(curwin, true, false);
            advance = false;
        }

        let mut arg_idx: c_int = 1;
        for i in 1..(*parmp).window_count {
            if (*curwin).w_arg_idx == -1 {
                arg_idx += 1;
                win_close(curwin, true, false);
                advance = false;
                continue;
            }

            if advance {
                if (*parmp).window_layout == WIN_TABS {
                    if (*curtab).tp_next.is_null() {
                        break;
                    }
                    goto_tabpage(0);
                    if i == 1 {
                        let mut buf = [0 as c_char; 100];
                        p_shm_save = xstrdup(p_shm);
                        vim_snprintf(buf.as_mut_ptr(), buf.len(), c"F%s".as_ptr(), p_shm);
                        set_option_value_give_err(kOptShortmess, cstr_obj(buf.as_ptr()), 0);
                    }
                } else {
                    if (*curwin).w_next.is_null() {
                        break;
                    }
                    win_enter((*curwin).w_next, false);
                }
            }
            advance = true;

            if ptr::eq(curbuf, (*firstwin).w_buffer) || (*curbuf).b_ffname.is_null() {
                (*curwin).w_arg_idx = arg_idx;
                swap_exists_did_quit = false;
                let fname = if arg_idx < gargcount() {
                    alist_name(garglist().add(arg_idx as usize))
                } else {
                    ptr::null_mut()
                };
                do_ecmd(
                    0,
                    fname,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ECMD_LASTL,
                    ECMD_HIDE,
                    curwin,
                );
                if swap_exists_did_quit {
                    if got_int || only_one_window() {
                        did_emsg = 0;
                        ui_call_error_exit(1);
                        getout(1);
                    }
                    win_close(curwin, true, false);
                    advance = false;
                }
                if arg_idx == gargcount() - 1 {
                    arg_had_last = true;
                }
                arg_idx += 1;
            }
            os_breakcheck();
            if got_int {
                vgetc();
                break;
            }
        }

        if !p_shm_save.is_null() {
            set_option_value_give_err(kOptShortmess, cstr_obj(p_shm_save), 0);
            xfree(p_shm_save.cast());
        }

        if (*parmp).window_layout == WIN_TABS {
            goto_tabpage(1);
        }
        autocmd_no_enter -= 1;

        let mut win = if (*parmp).window_count > 1 {
            firstwin
        } else {
            curwin
        };
        while (*win).w_onebuf_opt.wo_pvw != 0 {
            win = (*win).w_next;
            if win.is_null() {
                win = firstwin;
                break;
            }
        }
        win_enter(win, false);

        autocmd_no_leave -= 1;
        time_msg_if_enabled(c"editing files in windows".as_ptr());
        if (*parmp).window_count > 1 && (*parmp).window_layout != WIN_TABS {
            win_equal(curwin, false, b'b' as c_int);
        }
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn exe_pre_commands(parmp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        let cnt = (*parmp).n_pre_commands;
        if cnt <= 0 {
            return;
        }

        (*curwin).w_cursor.lnum = 0;
        estack_push(ETYPE_ARGS, gettext(c"pre-vimrc command line".as_ptr()), 0);
        current_sctx.sc_sid = SID_CMDARG;
        for i in 0..(cnt as usize) {
            do_cmdline_cmd((*parmp).pre_commands[i]);
        }
        estack_pop();
        current_sctx.sc_sid = 0;
        time_msg_if_enabled(c"--cmd commands".as_ptr());
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn exe_commands(parmp: *mut mparm_T) {
    #[cfg(not(test))]
    unsafe {
        msg_scroll = 1;
        if (*parmp).tagname.is_null() && (*curwin).w_cursor.lnum <= 1 {
            (*curwin).w_cursor.lnum = 0;
        }
        estack_push(ETYPE_ARGS, c"command line".as_ptr().cast_mut(), 0);
        current_sctx.sc_sid = SID_CARG;
        current_sctx.sc_seq = 0;
        for i in 0..((*parmp).n_commands as usize) {
            do_cmdline_cmd((*parmp).commands[i]);
            if (*parmp).cmds_tofree[i] != 0 {
                xfree((*parmp).commands[i].cast());
            }
        }
        estack_pop();
        current_sctx.sc_sid = 0;
        if (*curwin).w_cursor.lnum == 0 {
            (*curwin).w_cursor.lnum = 1;
        }

        msg_scroll = 0;

        if (*parmp).edit_type == EDIT_QF {
            qf_jump(ptr::null_mut(), 0, 0, 0);
        }
        time_msg_if_enabled(c"executing command arguments".as_ptr());
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn do_system_initialization() {
    #[cfg(not(test))]
    unsafe {
        let config_dirs = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs);
        if !config_dirs.is_null() {
            let mut iter: *const c_void = ptr::null();
            let appname = get_appname(false);
            let appname_bytes = CStr::from_ptr(appname).to_bytes();
            let sysinit_suffix: [u8; 13] = [
                PATHSEP, b's', b'y', b's', b'i', b'n', b'i', b't', b'.', b'v', b'i', b'm', 0,
            ];
            loop {
                let mut dir: *const c_char = ptr::null();
                let mut dir_len: usize = 0;
                iter = vim_env_iter(
                    b':' as c_char,
                    config_dirs,
                    iter,
                    &raw mut dir,
                    &raw mut dir_len,
                );
                if dir.is_null() || dir_len == 0 {
                    break;
                }
                let path_len = dir_len + 1 + appname_bytes.len() + sysinit_suffix.len();
                let vimrc = xmalloc(path_len).cast::<u8>();
                ptr::copy_nonoverlapping(dir.cast::<u8>(), vimrc, dir_len);
                let mut cur_len = dir_len;
                if *vimrc.add(cur_len - 1) != PATHSEP {
                    *vimrc.add(cur_len) = PATHSEP;
                    cur_len += 1;
                }
                ptr::copy_nonoverlapping(
                    appname_bytes.as_ptr(),
                    vimrc.add(cur_len),
                    appname_bytes.len(),
                );
                ptr::copy_nonoverlapping(
                    sysinit_suffix.as_ptr(),
                    vimrc.add(cur_len + appname_bytes.len()),
                    sysinit_suffix.len(),
                );
                if do_source(vimrc.cast(), false, DOSO_NONE, ptr::null_mut()) != FAIL {
                    xfree(vimrc.cast());
                    xfree(config_dirs.cast());
                    return;
                }
                xfree(vimrc.cast());
                if iter.is_null() {
                    break;
                }
            }
            xfree(config_dirs.cast());
        }

        do_source(
            c"$VIM/sysinit.vim".as_ptr().cast_mut(),
            false,
            DOSO_NONE,
            ptr::null_mut(),
        );
    }
}

unsafe fn do_user_initialization() -> bool {
    #[cfg(not(test))]
    unsafe {
        let mut do_exrc = p_exrc != 0;
        if execute_env(c"VIMINIT".as_ptr().cast_mut()) == OK {
            do_exrc = p_exrc != 0;
            return do_exrc;
        }

        let init_lua_path = stdpaths_user_conf_subpath(c"init.lua".as_ptr());
        let user_vimrc = stdpaths_user_conf_subpath(c"init.vim".as_ptr());

        if os_path_exists(init_lua_path)
            && do_source(init_lua_path, true, DOSO_VIMRC, ptr::null_mut()) != 0
        {
            if os_path_exists(user_vimrc) {
                semsg!(
                    (&raw const e_conflicting_configs).cast(),
                    init_lua_path,
                    user_vimrc,
                );
            }
            xfree(user_vimrc.cast());
            xfree(init_lua_path.cast());
            do_exrc = p_exrc != 0;
            return do_exrc;
        }
        xfree(init_lua_path.cast());

        if do_source(user_vimrc, true, DOSO_VIMRC, ptr::null_mut()) != FAIL {
            do_exrc = p_exrc != 0;
            if do_exrc {
                do_exrc = !path_equal(c".nvimrc".as_ptr(), user_vimrc, kPathCmpExpand);
            }
            xfree(user_vimrc.cast());
            return do_exrc;
        }
        xfree(user_vimrc.cast());

        let config_dirs = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs);
        if !config_dirs.is_null() {
            let appname = get_appname(false);
            let appname_bytes = CStr::from_ptr(appname).to_bytes();
            let init_lua_suffix: [u8; 10] =
                [PATHSEP, b'i', b'n', b'i', b't', b'.', b'l', b'u', b'a', 0];
            let init_vim_suffix: [u8; 10] =
                [PATHSEP, b'i', b'n', b'i', b't', b'.', b'v', b'i', b'm', 0];

            let mut iter: *const c_void = ptr::null();
            loop {
                let mut dir: *const c_char = ptr::null();
                let mut dir_len: usize = 0;
                iter = vim_env_iter(
                    b':' as c_char,
                    config_dirs,
                    iter,
                    &raw mut dir,
                    &raw mut dir_len,
                );
                if dir.is_null() || dir_len == 0 {
                    break;
                }

                let init_lua_len = dir_len + 1 + appname_bytes.len() + init_lua_suffix.len();
                let init_lua = xmalloc(init_lua_len).cast::<u8>();
                ptr::copy_nonoverlapping(dir.cast::<u8>(), init_lua, dir_len);
                *init_lua.add(dir_len) = PATHSEP;
                ptr::copy_nonoverlapping(
                    appname_bytes.as_ptr(),
                    init_lua.add(dir_len + 1),
                    appname_bytes.len(),
                );
                ptr::copy_nonoverlapping(
                    init_lua_suffix.as_ptr(),
                    init_lua.add(dir_len + 1 + appname_bytes.len()),
                    init_lua_suffix.len(),
                );

                let init_vim_len = dir_len + 1 + appname_bytes.len() + init_vim_suffix.len();
                let init_vim = xmalloc(init_vim_len).cast::<u8>();
                ptr::copy_nonoverlapping(dir.cast::<u8>(), init_vim, dir_len);
                *init_vim.add(dir_len) = PATHSEP;
                ptr::copy_nonoverlapping(
                    appname_bytes.as_ptr(),
                    init_vim.add(dir_len + 1),
                    appname_bytes.len(),
                );
                ptr::copy_nonoverlapping(
                    init_vim_suffix.as_ptr(),
                    init_vim.add(dir_len + 1 + appname_bytes.len()),
                    init_vim_suffix.len(),
                );

                if os_path_exists(init_lua.cast())
                    && do_source(init_lua.cast(), true, DOSO_VIMRC, ptr::null_mut()) != 0
                {
                    if os_path_exists(init_vim.cast()) {
                        semsg!(
                            (&raw const e_conflicting_configs).cast(),
                            init_lua,
                            init_vim,
                        );
                    }
                    xfree(init_vim.cast());
                    xfree(init_lua.cast());
                    xfree(config_dirs.cast());
                    do_exrc = p_exrc != 0;
                    return do_exrc;
                }
                xfree(init_lua.cast());

                if do_source(init_vim.cast(), true, DOSO_VIMRC, ptr::null_mut()) != FAIL {
                    do_exrc = p_exrc != 0;
                    if do_exrc {
                        do_exrc = !path_equal(c".nvimrc".as_ptr(), init_vim.cast(), kPathCmpExpand);
                    }
                    xfree(init_vim.cast());
                    xfree(config_dirs.cast());
                    return do_exrc;
                }
                xfree(init_vim.cast());

                if iter.is_null() {
                    break;
                }
            }
            xfree(config_dirs.cast());
        }

        if execute_env(c"EXINIT".as_ptr().cast_mut()) == OK {
            do_exrc = p_exrc != 0;
            return do_exrc;
        }
        do_exrc
    }
    #[cfg(test)]
    {
        false
    }
}

unsafe fn do_exrc_initialization() {
    #[cfg(not(test))]
    unsafe {
        let L = get_global_lstate();
        assert!(!L.is_null());

        lua_getfield(L, LUA_GLOBALSINDEX, c"require".as_ptr());
        lua_pushstring(L, c"vim._core.exrc".as_ptr());
        if nlua_pcall(L, 1, 0) != 0 {
            fprintf(
                log_get_stderr(),
                c"%s\n".as_ptr(),
                lua_tolstring(L, -1, ptr::null_mut()),
            );
        }
    }
}

unsafe fn source_startup_scripts(parmp: *const mparm_T) {
    #[cfg(not(test))]
    unsafe {
        if !(*parmp).use_vimrc.is_null() {
            if strequal((*parmp).use_vimrc, c"NONE".as_ptr())
                || strequal((*parmp).use_vimrc, c"NORC".as_ptr())
            {
                // Do nothing.
            } else if do_source((*parmp).use_vimrc, false, DOSO_NONE, ptr::null_mut()) != OK {
                semsg!(
                    gettext((&raw const e_cannot_read_from_str_2).cast()),
                    (*parmp).use_vimrc,
                );
            }
        } else if !silent_mode {
            do_system_initialization();
            if do_user_initialization() {
                do_exrc_initialization();
            }
        }
        time_msg_if_enabled(c"sourcing vimrc file(s)".as_ptr());
    }
    #[cfg(test)]
    {
        let _ = parmp;
    }
}

unsafe fn execute_env(env: *mut c_char) -> c_int {
    #[cfg(not(test))]
    unsafe {
        let initstr = os_getenv(env);
        if initstr.is_null() {
            return FAIL;
        }

        estack_push(ETYPE_ENV, env, 0);
        let save_current_sctx = current_sctx;
        current_sctx.sc_sid = SID_ENV;
        current_sctx.sc_seq = 0;
        current_sctx.sc_lnum = 0;
        do_cmdline_cmd(initstr);

        estack_pop();
        current_sctx = save_current_sctx;

        xfree(initstr.cast());
        OK
    }
    #[cfg(test)]
    {
        let _ = env;
        FAIL
    }
}

unsafe fn mainerr(msg1: *const c_char, msg2: *const c_char, msg3: *const c_char) -> ! {
    unsafe {
        print_mainerr(msg1, msg2, msg3);
        os_exit(1);
    }
}

unsafe fn print_mainerr(msg1: *const c_char, msg2: *const c_char, msg3: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        let prgname = path_tail(ARGV0);
        signal_stop();

        let stderr = log_get_stderr();
        fprintf(stderr, c"%s: %s".as_ptr(), prgname, gettext(msg1));
        if !msg2.is_null() {
            fprintf(stderr, c": \"%s\"".as_ptr(), msg2);
        }
        if !msg3.is_null() {
            fprintf(stderr, c": \"%s\"".as_ptr(), msg3);
        }
        fprintf(stderr, gettext(c"\nMore info with \"".as_ptr()));
        fprintf(stderr, c"%s -h\"\n".as_ptr(), prgname);
    }
    #[cfg(test)]
    {
        let _ = (msg1, msg2, msg3);
    }
}

unsafe fn version() {
    #[cfg(not(test))]
    unsafe {
        nlua_init(ptr::null_mut(), 0, -1);
        info_message = true;
        list_version();
        msg_putchar(b'\n' as c_int);
        msg_didout = false;
    }
}

unsafe fn usage() {
    #[cfg(not(test))]
    unsafe {
        signal_stop();

        printf(gettext(c"Usage:\n".as_ptr()));
        printf(gettext(c"  nvim [options] [file ...]\n".as_ptr()));
        printf(gettext(c"\nOptions:\n".as_ptr()));
        printf(gettext(
            c"  --cmd <cmd>           Execute <cmd> before any config\n".as_ptr(),
        ));
        printf(gettext(
            c"  +<cmd>, -c <cmd>      Execute <cmd> after config and first file\n".as_ptr(),
        ));
        printf(gettext(
            c"  -l <script> [args...] Execute Lua <script> (with optional args)\n".as_ptr(),
        ));
        printf(gettext(
            c"  -S <session>          Source <session> after loading the first file\n".as_ptr(),
        ));
        printf(gettext(
            c"  -s <scriptin>         Read Normal mode commands from <scriptin>\n".as_ptr(),
        ));
        printf(gettext(
            c"  -u <config>           Use this config file\n".as_ptr(),
        ));
        printf(c"\n".as_ptr());
        printf(gettext(c"  -d                    Diff mode\n".as_ptr()));
        printf(gettext(
            c"  -es, -Es              Silent (batch) mode\n".as_ptr(),
        ));
        printf(gettext(
            c"  -h, --help            Print this help message\n".as_ptr(),
        ));
        printf(gettext(
            c"  -i <shada>            Use this shada file\n".as_ptr(),
        ));
        printf(gettext(
            c"  -n                    No swap file, use memory only\n".as_ptr(),
        ));
        printf(gettext(
            c"  -o[N]                 Open N windows (default: one per file)\n".as_ptr(),
        ));
        printf(gettext(
            c"  -O[N]                 Open N vertical windows (default: one per file)\n".as_ptr(),
        ));
        printf(gettext(
            c"  -p[N]                 Open N tab pages (default: one per file)\n".as_ptr(),
        ));
        printf(gettext(
            c"  -R                    Read-only (view) mode\n".as_ptr(),
        ));
        printf(gettext(
            c"  -v, --version         Print version information\n".as_ptr(),
        ));
        printf(gettext(
            c"  -V[N][file]           Verbose [level][file]\n".as_ptr(),
        ));
        printf(c"\n".as_ptr());
        printf(gettext(
            c"  --                    Only file names after this\n".as_ptr(),
        ));
        printf(gettext(
            c"  --api-info            Write msgpack-encoded API metadata to stdout\n".as_ptr(),
        ));
        printf(gettext(
            c"  --clean               \"Factory defaults\" (skip user config and plugins, shada)\n"
                .as_ptr(),
        ));
        printf(gettext(
            c"  --embed               Use stdin/stdout as a msgpack-rpc channel\n".as_ptr(),
        ));
        printf(gettext(
            c"  --headless            Don't start a user interface\n".as_ptr(),
        ));
        printf(gettext(
            c"  --listen <address>    Serve RPC API from this address\n".as_ptr(),
        ));
        printf(gettext(
            c"  --remote[-subcommand] Execute commands remotely on a server\n".as_ptr(),
        ));
        printf(gettext(
            c"  --server <address>    Connect to this Nvim server\n".as_ptr(),
        ));
        printf(gettext(
            c"  --startuptime <file>  Write startup timing messages to <file>\n".as_ptr(),
        ));
        printf(gettext(
            c"\nSee \":help startup-options\" for all options.\n".as_ptr(),
        ));
    }
}

unsafe fn check_swap_exists_action() {
    #[cfg(not(test))]
    unsafe {
        if swap_exists_action == SEA_QUIT {
            ui_call_error_exit(1);
            getout(1);
        }
        handle_swap_exists(ptr::null_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_number_arg_and_init_params_work() {
        unsafe {
            let mut idx: c_int = 1;
            let val = get_number_arg(c"o25x".as_ptr(), &raw mut idx, 0);
            assert_eq!(val, 25);
            assert_eq!(idx, 3);

            let mut params: mparm_T = zeroed();
            init_params(&raw mut params, 3, ptr::null_mut());
            assert_eq!(params.argc, 3);
            assert_eq!(params.window_count, -1);
            assert_eq!(params.use_debug_break_level, -1);
            assert_eq!(params.lua_arg0, -1);
        }
    }
}
