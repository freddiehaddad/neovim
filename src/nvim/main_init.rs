//! Main entry point, CLI argument parsing, startup, and shutdown (`src/nvim/main.c`).

use crate::api::private::helpers::{
    api_free_object, api_metadata_raw, boolean_obj, cstr_as_obj, cstr_as_string, integer_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::ui::remote_ui_wait_for_attach;
use crate::arglist::{alist_add, alist_name, gargcount, garglist};
use crate::autocmd::{
    EVENT_BUFENTER, EVENT_BUFUNLOAD, EVENT_BUFWINLEAVE, EVENT_VIMENTER, EVENT_VIMLEAVE,
    EVENT_VIMLEAVEPRE, apply_autocmds, autocmd_init, block_autocmds, is_autocmd_blocked,
    unblock_autocmds,
};
use crate::buffer::{
    BLN_LISTED, MAXLNUM, SEA_DIALOG, SEA_NONE, SEA_QUIT, VAR_FIXED, buf_get_changedtick,
    buf_is_empty, buf_set_changedtick, buf_valid, buflist_new, bufref_T, bufref_valid,
    do_autochdir, do_modelines, handle_swap_exists, open_buffer, set_buflisted, set_bufref,
    set_curbuf, setfname,
};
use crate::channel::{
    CallbackReader, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO, channel_connect,
    channel_from_stdio, channel_init, channel_teardown,
};
use crate::charset::ascii_isdigit;
use crate::diff::{diff_win_options, diffopt_horizontal};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, default_grid_alloc, redraw_all_later, redraw_later, screenclear,
};
use crate::errors::*;
use crate::eval::typval::{
    tv_clear, tv_list_alloc, tv_list_alloc_ret, tv_list_append_string, tv_list_set_lock,
    varnumber_T,
};
use crate::eval::userfunc::invoke_all_defer;
use crate::eval::vars::{
    VV_ARGF, VV_ERRMSG, VV_EXITING, VV_EXITREASON, VV_OLDFILES, VV_PROGNAME, VV_PROGPATH,
    VV_SEND_SERVER, VV_STARTTIME, VV_SWAPCOMMAND, VV_VIM_DID_ENTER, VV_VIM_DID_INIT,
    get_vim_var_list, get_vim_var_str, set_reg_var, set_vim_var_list, set_vim_var_nr,
    set_vim_var_string, set_vim_var_type,
};
use crate::eval::{
    eval_has_provider, eval_init, garbage_collect, kListLenMayKnow, set_argv_var, timer_teardown,
};
use crate::event::r#loop::{
    loop_close, loop_init, loop_poll_events, loop_queues, uv_guess_handle, uv_strerror,
};
use crate::event::multiqueue::{multiqueue_new_child, multiqueue_process_events};
use crate::event::proc::proc_teardown;
use crate::event::socket::socket_address_tcp_host_end;
use crate::event::stream::{UV_NAMED_PIPE, UV_TCP};
use crate::ex_cmds::{ECMD_HIDE, ECMD_LASTL, do_ecmd, kOptShortmess};
use crate::ex_docmd::{do_cmdline_cmd, filetype_maybe_enable, filetype_plugin_enable};
use crate::ex_getln::cmdline_init;
use crate::extmark::TriState;
use crate::fileio::{READ_NEW, READ_STDIN, fprintf, printf, readfile, shorten_fnames};
use crate::globals::*;
use crate::highlight::highlight_init;
use crate::highlight_group::init_highlight;
use crate::input::{open_scriptin, stuffcharReadbuff, vgetc};
use crate::keycodes::K_NOP;
use crate::log::{
    LOGLVL_DBG, LOGLVL_INF, LOGLVL_WRN, log_get_stderr, log_init, logmsg, logmsg_str,
};
use crate::lua::converter::{LUA_GLOBALSINDEX, lua_getfield, lua_pushstring, lua_tolstring};
use crate::lua::executor::{
    get_global_lstate, kRetObject, nlua_call_typval, nlua_exec, nlua_exec_file, nlua_init,
    nlua_init_defaults, nlua_pcall,
};
use crate::mark::setpcmark;
use crate::math::{FAIL, OK};
use crate::mcursor::mc_init;
use crate::memline::{ml_close_all, ml_close_notmod, ml_recover, ml_sync_all, recover_names};
use crate::memory::{mem_st, free_all_mem, strequal, striequal_str};
use crate::message::{msg_putchar, semsg, wait_return};
use crate::mouse::setmouse;
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::msgpack_rpc::server::{server_init, server_teardown};
use crate::normal::{
    check_scrollbind, init_normal_cmds, kOptCbFlagUnnamed, kOptCbFlagUnnamedplus, normal_enter,
};
use crate::option::{
    OptInt, kOptArabic, kOptKeymap, kOptRightleft, kOptWindow, reset_modifiable, set_init_1,
    set_init_2, set_init_3, set_init_tablocal, set_option_direct, set_option_value_give_err,
    set_options_bin,
};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::env::{
    env_init, init_homedir, os_get_pid, os_getenv_buf, os_getenv_cstr, os_hint_priority,
    vim_env_iter,
};
#[cfg(windows)]
use crate::os::env::{os_homedir, os_setenv_append_path};
#[cfg(windows)]
use crate::os::fs::os_dup_cloexec;
use crate::os::fs::{
    CASE_INSENSITIVE_FILENAME, os_exepath, os_fopen, os_isdir, os_path_exists, os_write,
};
use crate::os::input::{os_breakcheck, os_isatty};
use crate::os::lang::{init_locale, set_lang_var};
use crate::os::signal::{signal_init, signal_reject_deadly, signal_stop, signal_teardown};
use crate::os::stdpaths::{
    XDGVarType, appname_is_valid, get_appname, stdpaths_get_xdg_var, stdpaths_user_conf_subpath,
};
use crate::os::time::os_realtime;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    concat_fnames, kPathCmpExpand, path_equal, path_fix_case, path_guess_exepath, path_tail,
    vim_FullName,
};
use crate::profile::{profile_dump, time_finish, time_init, time_msg, time_start};
use crate::quickfix::{kOptErrorfile, qf_init, qf_init_stack, qf_jump};
use crate::register::get_default_register_name;
use crate::runtime::{
    DOSO_NONE, DOSO_VIMRC, ETYPE_ARGS, ETYPE_ENV, SID_CARG, SID_CMDARG, SID_ENV, do_source,
    estack_init, estack_pop, estack_push, load_plugins,
};
use crate::shada::{shada_read_everything, shada_write_file};
use crate::strings::{gettext, vim_snprintf};
use crate::syntax::syn_maybe_enable;
use crate::terminal::{terminal_init, terminal_teardown};
use crate::ui::{
    do_autocmd_uienter_all, ui_active, ui_call_error_exit, ui_call_set_title, ui_flush, ui_init,
};
use crate::ui_client::{ui_client_run, ui_client_start_server, ui_client_stop};
use crate::ui_compositor::ui_comp_syn_init;
use crate::version::{list_version, main_exitfree};
use crate::window::{
    goto_tabpage, make_tabpages, make_windows, only_one_window, tabpage_T, win_alloc_first,
    win_close, win_count, win_enter, win_equal, win_init_size, win_new_screensize, win_valid,
};
use std::ffi::{CStr, c_char, c_int};
use std::io::Write;
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::{os_icon_init, os_icon_reset, os_title_reset, os_title_save};

pub const MAX_ARG_CMDS: usize = 10;

const NO_BUFFERS: c_int = 1;

const WIN_HOR: c_int = 1;
const WIN_VER: c_int = 2;
const WIN_TABS: c_int = 3;

const EDIT_NONE: c_int = 0;
const EDIT_FILE: c_int = 1;
const EDIT_STDIN: c_int = 2;
const EDIT_TAG: c_int = 3;
const EDIT_QF: c_int = 4;

const kOptShadafile: OptIndex = 259;
const kOptVerbosefile: OptIndex = 344;

const PATHSEP: u8 = b'/';

const ERR_ARG_MISSING: &CStr = c"Argument missing after";
const ERR_OPT_GARBAGE: &CStr = c"Garbage after option argument";
const ERR_OPT_UNKNOWN: &CStr = c"Unknown option argument";
const ERR_TOO_MANY_ARGS: &CStr = c"Too many edit arguments";
const ERR_EXTRA_CMD: &CStr =
    c"Too many \"+command\", \"-c command\" or \"--cmd command\" arguments";

pub struct mparm_T {
    pub argc: c_int,
    pub argv: *mut *mut c_char,

    pub use_vimrc: Option<std::ffi::CString>,
    pub clean: bool,

    pub commands: Vec<std::ffi::CString>,
    pub pre_commands: Vec<std::ffi::CString>,
    pub luaf: Option<std::ffi::CString>,
    pub lua_arg0: c_int,

    pub edit_type: c_int,
    pub tagname: Option<std::ffi::CString>,
    pub use_ef: Option<std::ffi::CString>,

    pub exmode: bool,
    pub input_istext: bool,

    pub no_swap_file: c_int,
    pub use_debug_break_level: c_int,
    pub window_count: c_int,
    pub window_layout: c_int,

    pub diff_mode: c_int,

    pub listen_addr: Option<std::ffi::CString>,
    pub remote: c_int,
    pub server_addr: Option<std::ffi::CString>,
    pub scriptin: Option<std::ffi::CString>,
    pub scriptout: Option<std::ffi::CString>,
    pub scriptout_append: bool,
    pub had_stdin_file: bool,
}

impl mparm_T {
    pub const fn new() -> Self {
        Self {
            argc: 0,
            argv: ptr::null_mut(),
            use_vimrc: None,
            clean: false,
            commands: Vec::new(),
            pre_commands: Vec::new(),
            luaf: None,
            lua_arg0: 0,
            edit_type: 0,
            tagname: None,
            use_ef: None,
            exmode: false,
            input_istext: false,
            no_swap_file: 0,
            use_debug_break_level: 0,
            window_count: 0,
            window_layout: 0,
            diff_mode: 0,
            listen_addr: None,
            remote: 0,
            server_addr: None,
            scriptin: None,
            scriptout: None,
            scriptout_append: false,
            had_stdin_file: false,
        }
    }
}

impl Default for mparm_T {
    fn default() -> Self {
        Self::new()
    }
}

struct MainInitState {
    ARGV0: *mut c_char,
    REALLY_EXITING: bool,
}

impl MainInitState {
    const fn new() -> Self {
        Self {
            ARGV0: ptr::null_mut(),
            REALLY_EXITING: false,
        }
    }
}

static MAIN_INIT_STATE: crate::globals::GlobalState<MainInitState> = crate::globals::GlobalState::new(MainInitState::new());

#[inline]
unsafe fn main_init_st() -> &'static mut MainInitState {
    unsafe { &mut *MAIN_INIT_STATE.as_ptr() }
}

use crate::charset::NUL;
use crate::extmark::{kFalse, kNone, kTrue};
#[cfg(windows)]
use crate::fileio::_setmode;
use crate::fileio::setbuf;
#[cfg(unix)]
use crate::fileio::tcdrain;
#[cfg(windows)]
use crate::fileio::{close, dup2};
use crate::globals::IOSIZE;
use crate::os::fs::MAXPATHL;
#[cfg(windows)]
use crate::os::os_win_console::{GetVersionExA, OsVersionInfoA};

#[inline]
unsafe fn time_msg_if_enabled(s: *const c_char) {
    unsafe {
        if has_time_fd() {
            time_msg(s, ptr::null());
        }
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
unsafe fn strnicmp_ascii(a: *const c_char, b: &[u8]) -> bool {
    unsafe {
        if a.is_null() {
            return false;
        }
        let bytes = CStr::from_ptr(a).to_bytes();
        bytes.len() >= b.len() && bytes[..b.len()].eq_ignore_ascii_case(b)
    }
}

pub unsafe fn event_init() {
    unsafe {
        loop_init(&raw mut g_st().main_loop);
        env_init();
        let queues = loop_queues((&raw mut g_st().main_loop).cast());
        g_st().resize_events = multiqueue_new_child((*queues).events);

        autocmd_init();
        signal_init();
        channel_init();
        terminal_init();
        ui_init();
        time_msg_if_enabled(c"event init".as_ptr());
    }
}

unsafe fn event_teardown() -> bool {
    unsafe {
        let queues = loop_queues((&raw mut g_st().main_loop).cast());
        if (*queues).events.is_null() {
            return true;
        }

        multiqueue_process_events((*queues).events);
        loop_poll_events((&raw mut g_st().main_loop).cast(), 0);
        server_teardown();
        channel_teardown();
        proc_teardown((&raw mut g_st().main_loop).cast());
        timer_teardown();
        signal_teardown();
        terminal_teardown();

        loop_close((&raw mut g_st().main_loop).cast(), true)
    }
}

unsafe fn early_init(paramp: *mut mparm_T) {
    unsafe {
        os_hint_priority();
        estack_init();
        cmdline_init();
        eval_init();
        set_vim_var_nr(VV_STARTTIME, os_realtime() as varnumber_T);
        init_path(if !main_init_st().ARGV0.is_null() {
            main_init_st().ARGV0
        } else {
            c"nvim".as_ptr()
        });
        init_normal_cmds();
        highlight_init();
        mc_init();

        #[cfg(windows)]
        {
            let mut ovi = OsVersionInfoA::new();
            ovi.dwOSVersionInfoSize = std::mem::size_of::<OsVersionInfoA>() as u32;
            GetVersionExA(&raw mut ovi);
            let mut buf = Vec::with_capacity(20);
            let _ = write!(buf, "{}.{}\0", ovi.dwMajorVersion, ovi.dwMinorVersion);
            let copy_len = buf.len().min(20);
            ptr::copy_nonoverlapping(
                buf.as_ptr().cast::<c_char>(),
                (&raw mut g_st().windowsVersion).cast::<c_char>(),
                copy_len,
            );
            *((&raw mut g_st().windowsVersion).cast::<c_char>().add(19)) = 0;
        }

        time_msg_if_enabled(c"early init".as_ptr());

        init_locale();
        set_init_tablocal();

        win_alloc_first();
        time_msg_if_enabled(c"init first window".as_ptr());

        g_st().global_alist.id = 0;

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
}

pub unsafe fn nvim_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    unsafe {
        main_init_st().ARGV0 = *argv;
        to_slash_if_needed(main_init_st().ARGV0);

        if !appname_is_valid() {
            fprintf!(
                log_get_stderr(),
                c"$NVIM_APPNAME must be a name or relative path.\n".as_ptr(),
            );
            std::process::exit(1);
        }

        let mut fname: *mut c_char = ptr::null_mut();
        let mut params = mparm_T::new();

        init_params(&raw mut params, argc, argv);
        init_startuptime(&raw mut params);

        for i in 1..params.argc {
            if striequal_str(*params.argv.add(i as usize), "--clean") {
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
        if g_st().embedded_mode {
            startup_stderr_fd = os_dup_cloexec(STDERR_FILENO);
        }

        let stdin_type = uv_guess_handle(STDIN_FILENO);
        let stdio_rpc =
            params.listen_addr.is_none() || stdin_type == UV_NAMED_PIPE || stdin_type == UV_TCP;
        if g_st().embedded_mode && stdio_rpc {
            let mut err: *const c_char = ptr::null();
            let on_data = CallbackReader::none();
            if channel_from_stdio(true, on_data, &raw mut err) == 0 {
                std::process::abort();
            }
        }

        if gargcount() > 0 {
            fname = get_fname(&raw mut params);
        }

        if g_st().recoverymode && fname.is_null() {
            g_st().headless_mode = true;
        }

        #[cfg(windows)]
        let has_term = true;
        #[cfg(not(windows))]
        let has_term = g_st().stdin_isatty || g_st().stdout_isatty || g_st().stderr_isatty;

        let use_builtin_ui = has_term && !g_st().headless_mode && !g_st().embedded_mode && !g_st().silent_mode;

        if params.remote != 0 {
            let server_addr_ptr = params
                .server_addr
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr());
            remote_request(
                &raw mut params,
                params.remote,
                server_addr_ptr,
                argc,
                argv,
                use_builtin_ui,
            );
        }

        let remote_ui = g_st().ui_client_channel_id != 0;

        if use_builtin_ui && !remote_ui {
            g_st().ui_client_forward_stdin = !g_st().stdin_isatty;
            let rv = ui_client_start_server(
                get_vim_var_str(VV_PROGPATH),
                params.argc as usize,
                params.argv,
            );
            if rv == 0 {
                fprintf!(log_get_stderr(), c"Failed to start Nvim server!\n".as_ptr());
                os_exit(1);
            }
            g_st().ui_client_channel_id = rv;
        }

        if g_st().ui_client_channel_id != 0 {
            ui_client_run();
        }
        assert!(g_st().ui_client_channel_id == 0 && !use_builtin_ui);

        if !server_init(
            params
                .listen_addr
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr()),
        ) {
            #[cfg(windows)]
            if startup_stderr_fd >= 0 {
                dup2(startup_stderr_fd, STDERR_FILENO);
                close(startup_stderr_fd);
            }
            mainerr((&raw const g_st().IObuff).cast(), ptr::null(), ptr::null());
        }

        #[cfg(windows)]
        if startup_stderr_fd >= 0 {
            close(startup_stderr_fd);
        }

        time_msg_if_enabled(c"expanding arguments".as_ptr());

        if params.diff_mode != 0 && params.window_count == -1 {
            params.window_count = 0;
        }
        g_st().RedrawingDisabled += 1;

        setbuf(crate::log::log_get_stdout(), ptr::null_mut());

        g_st().full_screen = !g_st().silent_mode;

        win_init_size();
        if params.diff_mode != 0 {
            diff_win_options(g_st().firstwin, false);
        }

        assert!(
            opt_vars().p_ch >= 0 && i64::from(g_st().Rows) >= opt_vars().p_ch && i64::from(g_st().Rows) - opt_vars().p_ch <= i64::from(c_int::MAX)
        );
        g_st().cmdline_row = g_st().Rows - opt_vars().p_ch as c_int;
        g_st().msg_row = g_st().cmdline_row;
        default_grid_alloc();
        set_init_2(g_st().headless_mode);
        time_msg_if_enabled(c"inits 2".as_ptr());

        g_st().msg_scroll = 1;
        g_st().no_wait_return += 1;

        init_highlight(true, false);
        ui_comp_syn_init();
        time_msg_if_enabled(c"init highlight".as_ptr());

        g_st().debug_break_level = params.use_debug_break_level;

        let use_remote_ui = g_st().embedded_mode && !g_st().headless_mode;
        if use_remote_ui {
            if !stdio_rpc {
                fprintf!(
                    log_get_stderr(),
                    c"Nvim server (pid: %lld) waiting for UI, listening on: \"%s\"\n".as_ptr(),
                    os_get_pid() as std::ffi::c_longlong,
                    get_vim_var_str(VV_SEND_SERVER),
                );
            }
            time_msg_if_enabled(c"waiting for UI".as_ptr());
            remote_ui_wait_for_attach();
            time_msg_if_enabled(c"done waiting for UI".as_ptr());
            (*g_st().firstwin).w_prev_height = (*g_st().firstwin).w_height;
        }

        *starting.as_ptr() = NO_BUFFERS;
        screenclear();
        win_new_screensize();
        time_msg_if_enabled(c"clear screen".as_ptr());

        if edit_stdin(&raw mut params) {
            params.edit_type = EDIT_STDIN;
        }

        if let Some(ref scriptin) = params.scriptin
            && !open_scriptin(scriptin.as_ptr().cast_mut())
        {
            os_exit(2);
        }
        if let Some(ref scriptout_name) = params.scriptout {
            g_st().scriptout = crate::os::fs::CFile::from_raw(os_fopen(
                scriptout_name.as_ptr(),
                if params.scriptout_append {
                    c"ab".as_ptr()
                } else {
                    c"wb".as_ptr()
                },
            ));
            if !has_scriptout() {
                let stderr = log_get_stderr();
                fprintf!(
                    stderr,
                    gettext(c"Cannot open for script output: \"".as_ptr()),
                );
                fprintf!(stderr, c"%s\"\n".as_ptr(), scriptout_name.as_ptr());
                os_exit(2);
            }
        }

        nlua_init_defaults();
        time_msg_if_enabled(c"init default mappings & autocommands".as_ptr());

        let vimrc_none = params.use_vimrc.as_deref() == Some(c"NONE");

        if vimrc_none && !params.clean {
            opt_vars().p_lpl = 0;
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

        if g_st().recoverymode && fname.is_null() {
            let mut items_tv = crate::eval::typval::typval_T::new();
            tv_list_alloc_ret(&raw mut items_tv, 0);
            recover_names(ptr::null_mut(), false, items_tv.vval.v_list());
            let mut lua_args = [items_tv, crate::eval::typval::typval_T::new()];
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
            opt_vars().p_uc = 0;
        }

        if g_st().silent_mode {
            opt_vars().p_ut = 1;
        }

        if *opt_vars().p_shada != NUL {
            shada_read_everything(ptr::null(), false, true);
            time_msg_if_enabled(c"reading ShaDa".as_ptr());
        }
        if get_vim_var_list(VV_OLDFILES).is_null() {
            set_vim_var_list(VV_OLDFILES, tv_list_alloc(0));
        }

        handle_quickfix(&raw mut params);

        *starting.as_ptr() = NO_BUFFERS;
        g_st().no_wait_return = 0;
        g_st().msg_scroll = 0;

        if params.edit_type == EDIT_STDIN && !g_st().recoverymode {
            read_stdin();
        }

        setmouse();

        redraw_later(*curwin.as_ptr(), UPD_VALID);

        g_st().no_wait_return += 1;

        create_windows(&raw mut params);
        time_msg_if_enabled(c"opening buffers".as_ptr());

        set_vim_var_string(VV_SWAPCOMMAND, ptr::null(), -1);

        if params.exmode {
            (**curwin.as_ptr()).w_cursor.lnum = (*g_st().curbuf).b_ml.ml_line_count;
        }

        apply_autocmds(
            EVENT_BUFENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
        );
        time_msg_if_enabled(c"BufEnter autocommands".as_ptr());
        setpcmark();

        if params.edit_type == EDIT_QF {
            qf_jump(ptr::null_mut(), 0, 0, 0);
            time_msg_if_enabled(c"jump to first error".as_ptr());
        }

        edit_buffers(&raw mut params);

        if params.diff_mode != 0 {
            let mut wp = (*g_st().curtab).tp_firstwin;
            while !wp.is_null() {
                if !(*wp).w_arg_idx_invalid {
                    diff_win_options(wp, true);
                }
                wp = (*wp).w_next;
            }
        }

        shorten_fnames(0);

        handle_tag(params.tagname.as_deref());

        if !params.commands.is_empty() {
            exe_commands(&raw mut params);
        }

        *starting.as_ptr() = 0;

        g_st().RedrawingDisabled = 0;
        redraw_all_later(UPD_NOT_VALID);
        g_st().no_wait_return = 0;

        do_autochdir();

        set_vim_var_nr(VV_VIM_DID_ENTER, 1);
        apply_autocmds(
            EVENT_VIMENTER,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            g_st().curbuf,
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

        if (**curwin.as_ptr()).w_onebuf_opt.wo_diff != 0 && (**curwin.as_ptr()).w_onebuf_opt.wo_scb != 0 {
            update_topline(*curwin.as_ptr());
            check_scrollbind(0, 0);
            time_msg_if_enabled(c"diff scrollbinding".as_ptr());
        }

        if g_st().restart_edit != 0 {
            stuffcharReadbuff(K_NOP);
        }

        if (opt_vars().cb_flags & (kOptCbFlagUnnamed | kOptCbFlagUnnamedplus)) != 0 {
            eval_has_provider(c"clipboard".as_ptr(), false);
        }

        if let Some(ref luaf) = params.luaf {
            g_st().msg_scroll = 1;
            logmsg_str(
                LOGLVL_DBG,
                ptr::null(),
                c"main".as_ptr(),
                line!() as c_int,
                true,
                c"executing Lua -l script".as_ptr(),
            );
            let lua_ok = nlua_exec_file(luaf.as_ptr());
            time_msg_if_enabled(c"executing Lua -l script".as_ptr());
            if g_st().msg_didout {
                msg_putchar(b'\n' as c_int);
                g_st().msg_didout = false;
            }
            getout(if lua_ok { 0 } else { 1 });
        } else if g_st().silent_mode {
            g_st().msg_scroll = 1;
            g_st().no_wait_return += 1;
            if !params.input_istext && !g_st().stdin_isatty {
                logmsg_str(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"main".as_ptr(),
                    line!() as c_int,
                    true,
                    c"executing stdin as Ex commands".as_ptr(),
                );
                let mut args = [crate::eval::typval::typval_T::new(); 1];
                nlua_call_typval(
                    c"vim._core.exmode".as_ptr(),
                    c"run".as_ptr(),
                    args.as_mut_ptr(),
                    ptr::null_mut(),
                );
            }
            if g_st().msg_didout {
                msg_putchar(b'\n' as c_int);
                g_st().msg_didout = false;
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
}

/// Prompts/dialogs should not wait for a key (no chance of meaningful input).
pub unsafe fn input_unanswerable() -> bool {
    unsafe { g_st().v_dying != 0 || (g_st().exiting && ui_active() == 0) }
}

pub unsafe fn os_exit(mut r: c_int) -> ! {
    unsafe {
        g_st().exiting = true;
        signal_reject_deadly();

        if g_st().ui_client_channel_id != 0 {
            ui_client_stop();
            if r == 0 {
                r = g_st().ui_client_exit_status;
            }
        } else {
            ui_flush();
        }

        if !event_teardown() && r == 0 {
            r = 1;
        }
        if g_st().ui_client_channel_id != 0 {
            #[cfg(unix)]
            {
                if g_st().stdout_isatty {
                    tcdrain(STDOUT_FILENO);
                }
                if g_st().stderr_isatty {
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
}

pub unsafe fn getout(mut exitval: c_int) -> ! {
    unsafe {
        assert!(g_st().ui_client_channel_id == 0);
        g_st().exiting = true;

        time_finish();

        if g_st().exmode_active {
            exitval += g_st().ex_exitval;
        }

        set_vim_var_type(VV_EXITING, crate::eval::typval::VarType::VAR_NUMBER);
        set_vim_var_nr(VV_EXITING, varnumber_T::from(exitval));

        if *get_vim_var_str(VV_EXITREASON) == NUL {
            set_vim_var_string(VV_EXITREASON, c"quit".as_ptr(), 4);
        }

        invoke_all_defer();

        if g_st().v_dying <= 1 {
            let mut next_tp: *const tabpage_T;
            let mut tp: *const tabpage_T = g_st().first_tabpage;
            while !tp.is_null() {
                next_tp = (*tp).tp_next;
                let mut wp = if ptr::eq(tp, g_st().curtab) {
                    g_st().firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if (*wp).w_buffer.is_null() || !buf_valid((*wp).w_buffer) {
                        wp = (*wp).w_next;
                        continue;
                    }
                    let buf = (*wp).w_buffer;
                    if buf_get_changedtick(buf) != -1 {
                        let mut bufref: bufref_T = bufref_T::new();
                        set_bufref(&raw mut bufref, buf);
                        apply_autocmds(
                            EVENT_BUFWINLEAVE,
                            (*buf).b_fname,
                            (*buf).b_fname,
                            false,
                            buf,
                        );
                        if bufref_valid(&raw mut bufref) {
                            buf_set_changedtick(buf, -1);
                        }
                        next_tp = g_st().first_tabpage;
                        break;
                    }
                    wp = (*wp).w_next;
                }
                tp = next_tp;
            }

            let mut buf = g_st().firstbuf;
            while !buf.is_null() {
                if (*buf).b_ml.ml_mfp.is_some() {
                    let mut bufref: bufref_T = bufref_T::new();
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
                g_st().curbuf,
            );
            if unblock != 0 {
                block_autocmds();
            }
        }

        if !mem_st().entered_free_all_mem && !opt_vars().p_shada.is_null() && *opt_vars().p_shada != NUL {
            shada_write_file(ptr::null(), false);
        }

        if g_st().v_dying <= 1 {
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
                g_st().curbuf,
            );
            if unblock != 0 {
                block_autocmds();
            }
        }

        profile_dump();

        if g_st().did_emsg != 0 {
            if !g_st().headless_mode && input_unanswerable() {
                logmsg!(
                    LOGLVL_WRN,
                    ptr::null(),
                    c"getout".as_ptr(),
                    line!() as c_int,
                    true,
                    c"Exiting with error: \"%s\"".as_ptr(),
                    get_vim_var_str(VV_ERRMSG),
                );
            }
            g_st().no_wait_return = 0;
            wait_return(0);
        }

        if opt_vars().p_title != 0 && *opt_vars().p_titleold != NUL {
            ui_call_set_title(cstr_as_string(opt_vars().p_titleold));
        }

        if g_st().garbage_collect_at_exit {
            garbage_collect(false);
        }

        #[cfg(windows)]
        {
            os_icon_reset();
            os_title_reset();
        }

        os_exit(exitval);
    }
}

pub unsafe fn preserve_exit(errmsg: *const c_char) -> ! {
    unsafe {
        if main_init_st().REALLY_EXITING {
            std::process::exit(2);
        }
        main_init_st().REALLY_EXITING = true;
        g_st().exiting = true;
        signal_reject_deadly();

        if g_st().ui_client_channel_id != 0 {
            ui_client_stop();
        }
        let stderr = log_get_stderr();
        if !errmsg.is_null() && *errmsg != NUL {
            let bytes = CStr::from_ptr(errmsg).to_bytes();
            let has_eol = bytes.last() == Some(&b'\n');
            fprintf!(
                stderr,
                if has_eol {
                    c"%s".as_ptr()
                } else {
                    c"%s\n".as_ptr()
                },
                errmsg,
            );
        }
        if g_st().ui_client_channel_id != 0 {
            os_exit(1);
        }

        ml_close_notmod();

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            let mfp = (*buf).b_ml.mfp_ptr();
            if !mfp.is_null() && (*mfp).mf_fname.is_some() {
                if !errmsg.is_null() {
                    fprintf!(stderr, c"Nvim: preserving files...\n".as_ptr());
                }
                ml_sync_all(0, 0, true);
                break;
            }
            buf = (*buf).b_next;
        }

        ml_close_all(false);

        if !errmsg.is_null() {
            fprintf!(stderr, c"Nvim: Finished.\n".as_ptr());
        }

        getout(1);
    }
}

pub fn __ubsan_default_options() -> *const c_char {
    c"print_stacktrace=1".as_ptr()
}

pub fn __asan_default_options() -> *const c_char {
    c"handle_abort=1,handle_sigill=1".as_ptr()
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
    unsafe {
        if server_addr.is_null() {
            *errmsg = c"no address specified".as_ptr();
            return 0;
        }
        let on_data = CallbackReader::none();
        let mut error: *const c_char = ptr::null();
        let is_tcp = !socket_address_tcp_host_end(server_addr).is_null();
        let chan = channel_connect(is_tcp, server_addr, true, on_data, 500, &raw mut error);
        if !error.is_null() {
            *errmsg = error;
            return 0;
        }
        chan
    }
}

unsafe fn remote_request(
    params: *mut mparm_T,
    remote_args: c_int,
    server_addr: *const c_char,
    argc: c_int,
    argv: *mut *mut c_char,
    ui_only: bool,
) {
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
                fprintf!(
                    stderr,
                    c"Remote ui failed to start: %s\n".as_ptr(),
                    connect_error,
                );
                os_exit(1);
            } else {
                let mut nvim_env_buf = [0 as c_char; MAXPATHL];
                let nvim_env = os_getenv_buf(
                    c"NVIM".as_ptr(),
                    nvim_env_buf.as_mut_ptr(),
                    nvim_env_buf.len(),
                );
                if strequal(server_addr, nvim_env) {
                    fprintf!(
                        stderr,
                        c"%s".as_ptr(),
                        c"Cannot attach UI of :terminal child to its parent. ".as_ptr(),
                    );
                    fprintf!(
                        stderr,
                        c"%s\n".as_ptr(),
                        c"(Unset $NVIM to skip this check)".as_ptr(),
                    );
                    os_exit(1);
                }
            }
            g_st().ui_client_channel_id = chan;
            return;
        }

        let count = (argc - remote_args).max(0) as usize;
        let mut args_vec: Vec<Object> = (remote_args..argc)
            .map(|t_argc| cstr_as_obj(*argv.add(t_argc as usize)))
            .collect();
        let args = Array {
            items: if count > 0 {
                args_vec.as_mut_ptr()
            } else {
                ptr::null_mut()
            },
            size: count,
            capacity: count,
        };

        let mut err = Error::new();
        let mut a_items: [Object; 4] = [
            integer_obj(Integer::from(chan as c_int)),
            cstr_as_obj(server_addr),
            cstr_as_obj(connect_error),
            Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData::Array(args),
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
        drop(args_vec);
        if err.err_type != ErrorType::kErrorTypeNone {
            fprintf!(stderr, c"%s\n".as_ptr(), err.msg_ptr());
            os_exit(2);
        }

        if o.obj_type != ObjectType::kObjectTypeDict {
            fprintf!(
                stderr,
                c"vim._cs_remote returned unexpected value\n".as_ptr(),
            );
            os_exit(2);
        }
        let dict: Dict = o.data.dict();

        let mut should_exit: TriState = kNone;
        let mut tabbed: TriState = kNone;

        for i in 0..dict.size {
            let item = &*dict.items.add(i);
            if strequal(item.key.data, c"errmsg".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeString {
                    fprintf!(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'errmsg'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                fprintf!(stderr, c"%s\n".as_ptr(), item.value.data.string().data);
                os_exit(2);
            } else if strequal(item.key.data, c"result".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeString {
                    fprintf!(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'result'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                printf!(c"%s".as_ptr(), item.value.data.string().data);
            } else if strequal(item.key.data, c"tabbed".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeBoolean {
                    fprintf!(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'tabbed'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                tabbed = if item.value.data.boolean() {
                    kTrue
                } else {
                    kFalse
                };
            } else if strequal(item.key.data, c"should_exit".as_ptr()) {
                if item.value.obj_type != ObjectType::kObjectTypeBoolean {
                    fprintf!(
                        stderr,
                        c"vim._cs_remote returned an unexpected type for 'should_exit'\n".as_ptr(),
                    );
                    os_exit(2);
                }
                should_exit = if item.value.data.boolean() {
                    kTrue
                } else {
                    kFalse
                };
            }
        }
        if should_exit == kNone || tabbed == kNone {
            fprintf!(
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
}

unsafe fn edit_stdin(parmp: *mut mparm_T) -> bool {
    unsafe {
        let implicit = !g_st().headless_mode
            && !(g_st().embedded_mode && g_st().stdin_fd <= 0)
            && (!(*parmp).exmode || (*parmp).input_istext)
            && !g_st().stdin_isatty
            && (*parmp).edit_type <= EDIT_STDIN
            && (*parmp).scriptin.is_none();
        (*parmp).had_stdin_file || implicit
    }
}

unsafe fn command_line_scan(parmp: *mut mparm_T) {
    unsafe {
        let mut argc = (*parmp).argc - 1;
        let mut argv = (*parmp).argv.add(1);
        let mut argv_idx: c_int = 1;
        let mut had_minmin = false;

        while argc > 0 {
            let arg0 = *argv;
            if *arg0 == b'+' as c_char && !had_minmin {
                if (*parmp).commands.len() >= MAX_ARG_CMDS {
                    mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                }
                argv_idx = -1;
                if *arg0.add(1) == NUL {
                    (*parmp).commands.push(c"$".to_owned());
                } else {
                    (*parmp)
                        .commands
                        .push(CStr::from_ptr(arg0.add(1)).to_owned());
                }
            } else if *arg0 == b'-' as c_char && !had_minmin {
                let mut want_argument = false;
                let c = *arg0.add(argv_idx as usize) as u8;
                argv_idx += 1;
                match c {
                    0 => {
                        if (*parmp).exmode {
                            g_st().silent_mode = true;
                            g_st().exmode_active = true;
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
                        if striequal_str(sub, "help") {
                            usage();
                            os_exit(0);
                        } else if striequal_str(sub, "version") {
                            version();
                            os_exit(0);
                        } else if striequal_str(sub, "api-info") {
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
                        } else if striequal_str(sub, "headless") {
                            g_st().headless_mode = true;
                        } else if striequal_str(sub, "embed") {
                            g_st().embedded_mode = true;
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
                            opt_vars().p_lpl = 0;
                        } else if strnicmp_ascii(sub, b"cmd") {
                            want_argument = true;
                            argv_idx += 3;
                        } else if strnicmp_ascii(sub, b"startuptime") {
                            want_argument = true;
                            argv_idx += 11;
                        } else if strnicmp_ascii(sub, b"clean") {
                            (*parmp).use_vimrc = Some(c"NONE".to_owned());
                            (*parmp).clean = true;
                            set_option_value_give_err(
                                kOptShadafile,
                                cstr_as_obj(c"NONE".as_ptr()),
                                0,
                            );
                        } else if strnicmp_ascii(sub, b"luamod-dev") {
                            g_st().nlua_disable_preload = true;
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
                        set_option_value_give_err(kOptArabic, boolean_obj(true), 0);
                    }
                    b'b' => {
                        set_options_bin((*g_st().curbuf).b_p_bin, 1, 0);
                        (*g_st().curbuf).b_p_bin = 1;
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
                        set_option_value_give_err(kOptKeymap, cstr_as_obj(c"hebrew".as_ptr()), 0);
                        set_option_value_give_err(kOptRightleft, boolean_obj(true), 0);
                    }
                    b'M' => {
                        reset_modifiable();
                        opt_vars().p_write = 0;
                    }
                    b'm' => {
                        opt_vars().p_write = 0;
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
                            (*parmp).use_ef =
                                Some(CStr::from_ptr(arg0.add(argv_idx as usize)).to_owned());
                            argv_idx = -1;
                        } else if argc > 1 {
                            want_argument = true;
                        }
                    }
                    b'R' => {
                        g_st().readonlymode = true;
                        (*g_st().curbuf).b_p_ro = 1;
                        opt_vars().p_uc = 10000;
                    }
                    b'r' | b'L' => {
                        g_st().recoverymode = true;
                    }
                    b's' => {
                        if (*parmp).exmode {
                            g_st().silent_mode = true;
                            g_st().exmode_active = true;
                            (*parmp).no_swap_file = 1;
                            if opt_vars().p_shadafile.is_null() || *opt_vars().p_shadafile == NUL {
                                set_option_value_give_err(
                                    kOptShadafile,
                                    cstr_as_obj(c"NONE".as_ptr()),
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
                            (*parmp).tagname =
                                Some(CStr::from_ptr(arg0.add(argv_idx as usize)).to_owned());
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
                        opt_vars().p_verbose = i64::from(get_number_arg(arg0, &raw mut argv_idx, 10));
                        if *arg0.add(argv_idx as usize) != NUL {
                            set_option_value_give_err(
                                kOptVerbosefile,
                                cstr_as_obj(arg0.add(argv_idx as usize)),
                                0,
                            );
                            argv_idx = CStr::from_ptr(arg0).to_bytes().len() as c_int;
                        }
                    }
                    b'w' => {
                        if ascii_isdigit(*arg0.add(argv_idx as usize) as c_int) {
                            let n = get_number_arg(arg0, &raw mut argv_idx, 10);
                            set_option_value_give_err(kOptWindow, integer_obj(OptInt::from(n)), 0);
                        } else {
                            want_argument = true;
                        }
                    }
                    b'c' => {
                        if *arg0.add(argv_idx as usize) != NUL {
                            if (*parmp).commands.len() >= MAX_ARG_CMDS {
                                mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                            }
                            (*parmp)
                                .commands
                                .push(CStr::from_ptr(arg0.add(argv_idx as usize)).to_owned());
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
                            if (*parmp).commands.len() >= MAX_ARG_CMDS {
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
                                let mut s = Vec::with_capacity(3 + a_bytes.len());
                                s.extend_from_slice(b"so ");
                                s.extend_from_slice(a_bytes);
                                (*parmp)
                                    .commands
                                    .push(std::ffi::CString::new(s).unwrap_or_default());
                            } else {
                                (*parmp).commands.push(CStr::from_ptr(cur_arg).to_owned());
                            }
                        }
                        b'-' => {
                            if strequal(prev_arg, c"--cmd".as_ptr()) {
                                if (*parmp).pre_commands.len() >= MAX_ARG_CMDS {
                                    mainerr(ERR_EXTRA_CMD.as_ptr(), ptr::null(), ptr::null());
                                }
                                (*parmp)
                                    .pre_commands
                                    .push(CStr::from_ptr(cur_arg).to_owned());
                            } else if strequal(prev_arg, c"--listen".as_ptr()) {
                                to_slash_if_needed(cur_arg);
                                (*parmp).listen_addr = Some(CStr::from_ptr(cur_arg).to_owned());
                            } else if strequal(prev_arg, c"--server".as_ptr()) {
                                to_slash_if_needed(cur_arg);
                                (*parmp).server_addr = Some(CStr::from_ptr(cur_arg).to_owned());
                            }
                        }
                        b'q' => {
                            (*parmp).use_ef = Some(CStr::from_ptr(cur_arg).to_owned());
                        }
                        b'i' => {
                            set_option_value_give_err(kOptShadafile, cstr_as_obj(cur_arg), 0);
                        }
                        b'l' => {
                            g_st().headless_mode = true;
                            g_st().silent_mode = true;
                            opt_vars().p_verbose = 1;
                            (*parmp).no_swap_file = 1;
                            if (*parmp).use_vimrc.is_none() {
                                (*parmp).use_vimrc = Some(c"NONE".to_owned());
                            }
                            if opt_vars().p_shadafile.is_null() || *opt_vars().p_shadafile == NUL {
                                set_option_value_give_err(
                                    kOptShadafile,
                                    cstr_as_obj(c"NONE".as_ptr()),
                                    0,
                                );
                            }
                            (*parmp).luaf = Some(CStr::from_ptr(cur_arg).to_owned());
                            argc -= 1;
                            if argc >= 0 {
                                (*parmp).lua_arg0 = (*parmp).argc - argc;
                                argc = 0;
                            }
                        }
                        b's' => {
                            if (*parmp).scriptin.is_some() {
                                vim_snprintf!(
                                    (&raw mut g_st().IObuff).cast(),
                                    IOSIZE,
                                    gettext(
                                        c"Attempt to open script file again: \"%s %s\"\n".as_ptr(),
                                    ),
                                    prev_arg,
                                    cur_arg,
                                );
                                fprintf!(
                                    log_get_stderr(),
                                    c"%s".as_ptr(),
                                    (&raw const g_st().IObuff).cast::<c_char>(),
                                );
                                os_exit(2);
                            }
                            (*parmp).scriptin = Some(CStr::from_ptr(cur_arg).to_owned());
                        }
                        b't' => {
                            (*parmp).tagname = Some(CStr::from_ptr(cur_arg).to_owned());
                        }
                        b'u' => {
                            to_slash_if_needed(cur_arg);
                            (*parmp).use_vimrc = Some(CStr::from_ptr(cur_arg).to_owned());
                        }
                        b'U' => {}
                        b'w' | b'W' => {
                            if c == b'w' && ascii_isdigit(*cur_arg as c_int) {
                                argv_idx = 0;
                                let n = get_number_arg(cur_arg, &raw mut argv_idx, 10);
                                set_option_value_give_err(
                                    kOptWindow,
                                    integer_obj(OptInt::from(n)),
                                    0,
                                );
                                argv_idx = -1;
                            } else {
                                if (*parmp).scriptout.is_some() {
                                    vim_snprintf!(
                                        (&raw mut g_st().IObuff).cast(),
                                        IOSIZE,
                                        gettext(
                                            c"Attempt to open script file again: \"%s %s\"\n"
                                                .as_ptr(),
                                        ),
                                        prev_arg,
                                        cur_arg,
                                    );
                                    fprintf!(
                                        log_get_stderr(),
                                        c"%s".as_ptr(),
                                        (&raw const g_st().IObuff).cast::<c_char>(),
                                    );
                                    os_exit(2);
                                }
                                (*parmp).scriptout = Some(CStr::from_ptr(cur_arg).to_owned());
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

                let mut p_vec: Vec<c_char> = CStr::from_ptr(arg0)
                    .to_bytes_with_nul()
                    .iter()
                    .map(|&b| b as c_char)
                    .collect();

                #[cfg(windows)]
                {
                    to_slash_if_needed(p_vec.as_mut_ptr());
                    if p_vec[0] == b'~' as c_char && p_vec.get(1) == Some(&(b'/' as c_char)) {
                        let homedir = os_homedir();
                        let h_bytes = CStr::from_ptr(homedir).to_bytes();
                        let p_bytes = CStr::from_ptr(p_vec.as_ptr().add(1)).to_bytes();
                        let mut expanded = Vec::with_capacity(h_bytes.len() + p_bytes.len() + 1);
                        expanded.extend(h_bytes.iter().map(|&b| b as c_char));
                        expanded.extend(p_bytes.iter().map(|&b| b as c_char));
                        expanded.push(0);
                        p_vec = expanded;
                    }
                }

                if (*parmp).diff_mode != 0
                    && os_isdir(p_vec.as_ptr())
                    && gargcount() > 0
                    && !os_isdir(alist_name(garglist()))
                {
                    let tail = path_tail(alist_name(garglist()));
                    let joined =
                        concat_fnames(cstr_as_string(p_vec.as_ptr()), cstr_as_string(tail), true);
                    p_vec = joined
                        .as_bytes_with_nul()
                        .iter()
                        .map(|&b| b as c_char)
                        .collect();
                }

                if CASE_INSENSITIVE_FILENAME {
                    path_fix_case(p_vec.as_mut_ptr());
                }

                let alist_fnum_flag = if edit_stdin(parmp) { 1 } else { 2 };
                alist_add(
                    &raw mut g_st().global_alist,
                    CStr::from_ptr(p_vec.as_ptr()).to_owned(),
                    alist_fnum_flag,
                );
            }

            if argv_idx <= 0 || *(*argv).add(argv_idx as usize) == NUL {
                argc -= 1;
                argv = argv.add(1);
                argv_idx = 1;
            }
        }

        if g_st().embedded_mode && (g_st().silent_mode || (*parmp).luaf.is_some()) {
            mainerr(
                gettext(c"--embed conflicts with -es/-Es/-l".as_ptr()),
                ptr::null(),
                ptr::null(),
            );
        }

        if let Some(cmd0) = (*parmp).commands.first() {
            let cmd0_bytes = cmd0.as_bytes();
            let swcmd_len = cmd0_bytes.len() + 2;
            let mut swcmd = Vec::with_capacity(swcmd_len + 1);
            swcmd.push(b':');
            swcmd.extend_from_slice(cmd0_bytes);
            swcmd.push(b'\r');
            swcmd.push(0);
            set_vim_var_string(VV_SWAPCOMMAND, swcmd.as_ptr().cast(), swcmd_len as isize);
        }

        time_msg_if_enabled(c"parsing arguments".as_ptr());
    }
}

unsafe fn set_argf_var() {
    unsafe {
        let list = tv_list_alloc(kListLenMayKnow);
        let count = gargcount();
        for i in 0..count {
            let fname = alist_name(garglist().add(i as usize));
            if !fname.is_null() {
                let _ = vim_FullName(fname, (&raw mut g_st().NameBuff).cast(), MAXPATHL, false);
                tv_list_append_string(list, (&raw const g_st().NameBuff).cast(), -1);
            }
        }
        tv_list_set_lock(list, VAR_FIXED);
        set_vim_var_list(VV_ARGF, list);
    }
}

unsafe fn init_params(paramp: *mut mparm_T, argc: c_int, argv: *mut *mut c_char) {
    unsafe {
        *paramp = mparm_T::new();
        (*paramp).argc = argc;
        (*paramp).argv = argv;
        (*paramp).use_debug_break_level = -1;
        (*paramp).window_count = -1;
        (*paramp).listen_addr = None;
        (*paramp).server_addr = None;
        (*paramp).remote = 0;
        (*paramp).luaf = None;
        (*paramp).lua_arg0 = -1;
    }
}

unsafe fn init_startuptime(paramp: *mut mparm_T) {
    unsafe {
        let mut is_embed = false;
        for i in 1..((*paramp).argc - 1) {
            if striequal_str(*(*paramp).argv.add(i as usize), "--embed") {
                is_embed = true;
                break;
            }
        }
        for i in 1..((*paramp).argc - 1) {
            if striequal_str(*(*paramp).argv.add(i as usize), "--startuptime") {
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
}

unsafe fn check_and_set_isatty(_paramp: *mut mparm_T) {
    unsafe {
        g_st().stdin_isatty = os_isatty(STDIN_FILENO);
        g_st().stdout_isatty = os_isatty(STDOUT_FILENO);
        g_st().stderr_isatty = os_isatty(STDERR_FILENO);
        time_msg_if_enabled(c"window checked".as_ptr());
    }
}

unsafe fn init_path(exename: *const c_char) {
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
}

unsafe fn get_fname(_parmp: *mut mparm_T) -> *mut c_char {
    unsafe { alist_name(garglist()) }
}

unsafe fn set_window_layout(paramp: *mut mparm_T) {
    unsafe {
        if (*paramp).diff_mode != 0 && (*paramp).window_layout == 0 {
            (*paramp).window_layout = if diffopt_horizontal() {
                WIN_HOR
            } else {
                WIN_VER
            };
        }
    }
}

unsafe fn handle_quickfix(paramp: *mut mparm_T) {
    unsafe {
        if (*paramp).edit_type == EDIT_QF {
            if let Some(ref use_ef) = (*paramp).use_ef {
                set_option_direct(kOptErrorfile, cstr_as_obj(use_ef.as_ptr()), 0, SID_CARG);
            }
            vim_snprintf!((&raw mut g_st().IObuff).cast(), IOSIZE, c"cfile %s".as_ptr(), opt_vars().p_ef);
            if qf_init(
                ptr::null_mut(),
                opt_vars().p_ef,
                opt_vars().p_efm,
                1,
                (&raw const g_st().IObuff).cast(),
                opt_vars().p_menc,
            ) < 0
            {
                msg_putchar(b'\n' as c_int);
                os_exit(3);
            }
            time_msg_if_enabled(c"reading errorfile".as_ptr());
        }
    }
}

unsafe fn handle_tag(tagname: Option<&CStr>) {
    unsafe {
        if let Some(tagname) = tagname {
            g_st().swap_exists_did_quit = false;
            vim_snprintf!(
                (&raw mut g_st().IObuff).cast(),
                IOSIZE,
                c"ta %s".as_ptr(),
                tagname.as_ptr(),
            );
            do_cmdline_cmd((&raw const g_st().IObuff).cast());
            time_msg_if_enabled(c"jumping to tag".as_ptr());

            if g_st().swap_exists_did_quit {
                ui_call_error_exit(1);
                getout(1);
            }
        }
    }
}

unsafe fn read_stdin() {
    unsafe {
        g_st().swap_exists_action = SEA_DIALOG;
        g_st().no_wait_return = 1;
        let save_msg_didany = g_st().msg_didany;

        if !(*g_st().curbuf).b_ffname.is_null() {
            let stdin_buf = buflist_new(ptr::null_mut(), ptr::null_mut(), 0, BLN_LISTED);
            if stdin_buf.is_null() {
                semsg!(c"Failed to create buffer for stdin".as_ptr());
                return;
            }

            let initial_buf_handle: handle_T = (*g_st().curbuf).b_fnum;
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
            let stdin_buf_empty = buf_is_empty(g_st().curbuf);

            let mut buf = [0 as c_char; 100];
            vim_snprintf!(
                buf.as_mut_ptr(),
                buf.len(),
                c"silent! buffer %d".as_ptr(),
                initial_buf_handle,
            );
            do_cmdline_cmd(buf.as_ptr());

            if stdin_buf_empty {
                vim_snprintf!(
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
            if buf_is_empty(g_st().curbuf) && !(*g_st().curbuf).b_next.is_null() {
                do_cmdline_cmd(c"silent! bnext".as_ptr());
                do_cmdline_cmd(c"silent! bwipeout 1".as_ptr());
            }
        }

        g_st().no_wait_return = 0;
        g_st().msg_didany = save_msg_didany;
        time_msg_if_enabled(c"reading stdin".as_ptr());
        check_swap_exists_action();
    }
}

unsafe fn create_windows(parmp: *mut mparm_T) {
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
            } else if (*g_st().firstwin).w_next.is_null() || (*(*g_st().firstwin).w_next).w_floating {
                (*parmp).window_count =
                    make_windows((*parmp).window_count, (*parmp).window_layout == WIN_VER);
                time_msg_if_enabled(c"making windows".as_ptr());
            } else {
                (*parmp).window_count = win_count();
            }
        } else {
            (*parmp).window_count = 1;
        }

        if g_st().recoverymode {
            g_st().msg_scroll = 1;
            ml_recover(true);
            if (*g_st().curbuf).b_ml.ml_mfp.is_none() {
                getout(1);
            }
            do_modelines(0);
        } else {
            let mut done: c_int = 0;
            g_st().autocmd_no_enter += 1;
            g_st().autocmd_no_leave += 1;
            let startup_curwin = *curwin.as_ptr();
            let mut dorewind = true;
            while done < 1000 {
                done += 1;
                if dorewind {
                    if (*parmp).window_layout == WIN_TABS {
                        goto_tabpage(1);
                    } else {
                        *curwin.as_ptr() = g_st().firstwin;
                    }
                } else if (*parmp).window_layout == WIN_TABS {
                    if (*g_st().curtab).tp_next.is_null() {
                        break;
                    }
                    goto_tabpage(0);
                } else {
                    if (**curwin.as_ptr()).w_next.is_null() {
                        break;
                    }
                    *curwin.as_ptr() = (**curwin.as_ptr()).w_next;
                }
                dorewind = false;
                g_st().curbuf = (**curwin.as_ptr()).w_buffer;
                if (*g_st().curbuf).b_ml.ml_mfp.is_none() {
                    if opt_vars().p_fdls >= 0 {
                        (**curwin.as_ptr()).w_onebuf_opt.wo_fdl = opt_vars().p_fdls;
                    }
                    g_st().swap_exists_action = SEA_DIALOG;
                    set_buflisted(1);

                    open_buffer(false, ptr::null_mut(), 0);

                    if g_st().swap_exists_action == SEA_QUIT {
                        if g_st().got_int || only_one_window() {
                            g_st().did_emsg = 0;
                            ui_call_error_exit(1);
                            getout(1);
                        }
                        setfname(g_st().curbuf, ptr::null_mut(), ptr::null_mut(), false);
                        (**curwin.as_ptr()).w_arg_idx = -1;
                        g_st().swap_exists_action = SEA_NONE;
                    } else {
                        handle_swap_exists(ptr::null_mut());
                    }
                    dorewind = true;
                }
                os_breakcheck();
                if g_st().got_int {
                    vgetc();
                    break;
                }
            }
            if (*parmp).window_layout == WIN_TABS {
                goto_tabpage(1);
            } else if (*parmp).window_count > 1 || !win_valid(startup_curwin) {
                *curwin.as_ptr() = g_st().firstwin;
            } else {
                *curwin.as_ptr() = startup_curwin;
            }
            g_st().curbuf = (**curwin.as_ptr()).w_buffer;
            g_st().autocmd_no_enter -= 1;
            g_st().autocmd_no_leave -= 1;
        }
    }
}

unsafe fn edit_buffers(parmp: *mut mparm_T) {
    unsafe {
        let mut advance = true;
        let mut p_shm_save: Option<std::ffi::CString> = None;

        g_st().autocmd_no_enter += 1;
        g_st().autocmd_no_leave += 1;

        if (**curwin.as_ptr()).w_arg_idx == -1 {
            win_close(*curwin.as_ptr(), true, false);
            advance = false;
        }

        let mut arg_idx: c_int = 1;
        for i in 1..(*parmp).window_count {
            if (**curwin.as_ptr()).w_arg_idx == -1 {
                arg_idx += 1;
                win_close(*curwin.as_ptr(), true, false);
                advance = false;
                continue;
            }

            if advance {
                if (*parmp).window_layout == WIN_TABS {
                    if (*g_st().curtab).tp_next.is_null() {
                        break;
                    }
                    goto_tabpage(0);
                    if i == 1 {
                        let mut buf = [0 as c_char; 100];
                        p_shm_save = Some(CStr::from_ptr(opt_vars().p_shm).to_owned());
                        vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"F%s".as_ptr(), opt_vars().p_shm);
                        set_option_value_give_err(kOptShortmess, cstr_as_obj(buf.as_ptr()), 0);
                    }
                } else {
                    if (**curwin.as_ptr()).w_next.is_null() {
                        break;
                    }
                    win_enter((**curwin.as_ptr()).w_next, false);
                }
            }
            advance = true;

            if ptr::eq(g_st().curbuf, (*g_st().firstwin).w_buffer) || (*g_st().curbuf).b_ffname.is_null() {
                (**curwin.as_ptr()).w_arg_idx = arg_idx;
                g_st().swap_exists_did_quit = false;
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
                    *curwin.as_ptr(),
                );
                if g_st().swap_exists_did_quit {
                    if g_st().got_int || only_one_window() {
                        g_st().did_emsg = 0;
                        ui_call_error_exit(1);
                        getout(1);
                    }
                    win_close(*curwin.as_ptr(), true, false);
                    advance = false;
                }
                if arg_idx == gargcount() - 1 {
                    g_st().arg_had_last = true;
                }
                arg_idx += 1;
            }
            os_breakcheck();
            if g_st().got_int {
                vgetc();
                break;
            }
        }

        if let Some(ref p_shm_save) = p_shm_save {
            set_option_value_give_err(kOptShortmess, cstr_as_obj(p_shm_save.as_ptr()), 0);
        }

        if (*parmp).window_layout == WIN_TABS {
            goto_tabpage(1);
        }
        g_st().autocmd_no_enter -= 1;

        let mut win = if (*parmp).window_count > 1 {
            g_st().firstwin
        } else {
            *curwin.as_ptr()
        };
        while (*win).w_onebuf_opt.wo_pvw != 0 {
            win = (*win).w_next;
            if win.is_null() {
                win = g_st().firstwin;
                break;
            }
        }
        win_enter(win, false);

        g_st().autocmd_no_leave -= 1;
        time_msg_if_enabled(c"editing files in windows".as_ptr());
        if (*parmp).window_count > 1 && (*parmp).window_layout != WIN_TABS {
            win_equal(*curwin.as_ptr(), false, b'b' as c_int);
        }
    }
}

unsafe fn exe_pre_commands(parmp: *mut mparm_T) {
    unsafe {
        if (*parmp).pre_commands.is_empty() {
            return;
        }

        (**curwin.as_ptr()).w_cursor.lnum = 0;
        estack_push(ETYPE_ARGS, gettext(c"pre-vimrc command line".as_ptr()), 0);
        g_st().current_sctx.sc_sid = SID_CMDARG;
        for cmd in &(*parmp).pre_commands {
            do_cmdline_cmd(cmd.as_ptr());
        }
        estack_pop();
        g_st().current_sctx.sc_sid = 0;
        time_msg_if_enabled(c"--cmd commands".as_ptr());
    }
}

unsafe fn exe_commands(parmp: *mut mparm_T) {
    unsafe {
        g_st().msg_scroll = 1;
        if (*parmp).tagname.is_none() && (**curwin.as_ptr()).w_cursor.lnum <= 1 {
            (**curwin.as_ptr()).w_cursor.lnum = 0;
        }
        estack_push(ETYPE_ARGS, c"command line".as_ptr().cast_mut(), 0);
        g_st().current_sctx.sc_sid = SID_CARG;
        g_st().current_sctx.sc_seq = 0;
        let cmds = std::mem::take(&mut (*parmp).commands);
        for cmd in &cmds {
            do_cmdline_cmd(cmd.as_ptr());
        }
        estack_pop();
        g_st().current_sctx.sc_sid = 0;
        if (**curwin.as_ptr()).w_cursor.lnum == 0 {
            (**curwin.as_ptr()).w_cursor.lnum = 1;
        }

        g_st().msg_scroll = 0;

        if (*parmp).edit_type == EDIT_QF {
            qf_jump(ptr::null_mut(), 0, 0, 0);
        }
        time_msg_if_enabled(c"executing command arguments".as_ptr());
    }
}

unsafe fn do_system_initialization() {
    unsafe {
        if let Some(config_dirs) = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs) {
            let mut iter: *const c_char = ptr::null();
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
                    config_dirs.as_ptr(),
                    iter,
                    &raw mut dir,
                    &raw mut dir_len,
                );
                if dir.is_null() || dir_len == 0 {
                    break;
                }
                let path_len = dir_len + 1 + appname_bytes.len() + sysinit_suffix.len();
                let mut vimrc = Vec::with_capacity(path_len);
                vimrc.extend_from_slice(std::slice::from_raw_parts(dir.cast::<u8>(), dir_len));
                if vimrc.last() != Some(&PATHSEP) {
                    vimrc.push(PATHSEP);
                }
                vimrc.extend_from_slice(appname_bytes);
                vimrc.extend_from_slice(&sysinit_suffix);
                if do_source(vimrc.as_mut_ptr().cast(), false, DOSO_NONE, ptr::null_mut()) != FAIL {
                    return;
                }
                if iter.is_null() {
                    break;
                }
            }
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
    unsafe {
        let mut do_exrc = opt_vars().p_exrc != 0;
        if execute_env(c"VIMINIT".as_ptr().cast_mut()) == OK {
            do_exrc = opt_vars().p_exrc != 0;
            return do_exrc;
        }

        let init_lua_path = stdpaths_user_conf_subpath(c"init.lua".as_ptr());
        let user_vimrc = stdpaths_user_conf_subpath(c"init.vim".as_ptr());
        let init_lua_ptr = init_lua_path.as_ptr().cast_mut();
        let user_vimrc_ptr = user_vimrc.as_ptr().cast_mut();

        if os_path_exists(init_lua_ptr)
            && do_source(init_lua_ptr, true, DOSO_VIMRC, ptr::null_mut()) != 0
        {
            if os_path_exists(user_vimrc_ptr) {
                semsg!(
                    (&raw const e_conflicting_configs).cast(),
                    init_lua_ptr,
                    user_vimrc_ptr,
                );
            }
            do_exrc = opt_vars().p_exrc != 0;
            return do_exrc;
        }
        drop(init_lua_path);

        if do_source(user_vimrc_ptr, true, DOSO_VIMRC, ptr::null_mut()) != FAIL {
            do_exrc = opt_vars().p_exrc != 0;
            if do_exrc {
                do_exrc = !path_equal(c".nvimrc".as_ptr(), user_vimrc_ptr, kPathCmpExpand);
            }
            return do_exrc;
        }
        drop(user_vimrc);

        if let Some(config_dirs) = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs) {
            let appname = get_appname(false);
            let appname_bytes = CStr::from_ptr(appname).to_bytes();
            let init_lua_suffix: [u8; 10] =
                [PATHSEP, b'i', b'n', b'i', b't', b'.', b'l', b'u', b'a', 0];
            let init_vim_suffix: [u8; 10] =
                [PATHSEP, b'i', b'n', b'i', b't', b'.', b'v', b'i', b'm', 0];

            let mut iter: *const c_char = ptr::null();
            loop {
                let mut dir: *const c_char = ptr::null();
                let mut dir_len: usize = 0;
                iter = vim_env_iter(
                    b':' as c_char,
                    config_dirs.as_ptr(),
                    iter,
                    &raw mut dir,
                    &raw mut dir_len,
                );
                if dir.is_null() || dir_len == 0 {
                    break;
                }

                let init_lua_len = dir_len + 1 + appname_bytes.len() + init_lua_suffix.len();
                let mut init_lua = Vec::with_capacity(init_lua_len);
                init_lua.extend_from_slice(std::slice::from_raw_parts(dir.cast::<u8>(), dir_len));
                init_lua.push(PATHSEP);
                init_lua.extend_from_slice(appname_bytes);
                init_lua.extend_from_slice(&init_lua_suffix);

                let init_vim_len = dir_len + 1 + appname_bytes.len() + init_vim_suffix.len();
                let mut init_vim = Vec::with_capacity(init_vim_len);
                init_vim.extend_from_slice(std::slice::from_raw_parts(dir.cast::<u8>(), dir_len));
                init_vim.push(PATHSEP);
                init_vim.extend_from_slice(appname_bytes);
                init_vim.extend_from_slice(&init_vim_suffix);

                if os_path_exists(init_lua.as_ptr().cast())
                    && do_source(
                        init_lua.as_mut_ptr().cast(),
                        true,
                        DOSO_VIMRC,
                        ptr::null_mut(),
                    ) != 0
                {
                    if os_path_exists(init_vim.as_ptr().cast()) {
                        semsg!(
                            (&raw const e_conflicting_configs).cast(),
                            init_lua.as_ptr(),
                            init_vim.as_ptr(),
                        );
                    }
                    do_exrc = opt_vars().p_exrc != 0;
                    return do_exrc;
                }

                if do_source(
                    init_vim.as_mut_ptr().cast(),
                    true,
                    DOSO_VIMRC,
                    ptr::null_mut(),
                ) != FAIL
                {
                    do_exrc = opt_vars().p_exrc != 0;
                    if do_exrc {
                        do_exrc = !path_equal(
                            c".nvimrc".as_ptr(),
                            init_vim.as_ptr().cast(),
                            kPathCmpExpand,
                        );
                    }
                    return do_exrc;
                }

                if iter.is_null() {
                    break;
                }
            }
        }

        if execute_env(c"EXINIT".as_ptr().cast_mut()) == OK {
            do_exrc = opt_vars().p_exrc != 0;
            return do_exrc;
        }
        do_exrc
    }
}

unsafe fn do_exrc_initialization() {
    unsafe {
        let L = get_global_lstate();
        assert!(!L.is_null());

        lua_getfield(L, LUA_GLOBALSINDEX, c"require".as_ptr());
        lua_pushstring(L, c"vim._core.exrc".as_ptr());
        if nlua_pcall(L, 1, 0) != 0 {
            fprintf!(
                log_get_stderr(),
                c"%s\n".as_ptr(),
                lua_tolstring(L, -1, ptr::null_mut()),
            );
        }
    }
}

unsafe fn source_startup_scripts(parmp: *const mparm_T) {
    unsafe {
        if let Some(ref use_vimrc) = (*parmp).use_vimrc {
            if use_vimrc.as_c_str() == c"NONE" || use_vimrc.as_c_str() == c"NORC" {
                // Do nothing.
            } else if do_source(
                use_vimrc.as_ptr().cast_mut(),
                false,
                DOSO_NONE,
                ptr::null_mut(),
            ) != OK
            {
                semsg!(
                    gettext((&raw const e_cannot_read_from_str_2).cast()),
                    use_vimrc.as_ptr(),
                );
            }
        } else if !g_st().silent_mode {
            do_system_initialization();
            if do_user_initialization() {
                do_exrc_initialization();
            }
        }
        time_msg_if_enabled(c"sourcing vimrc file(s)".as_ptr());
    }
}

unsafe fn execute_env(env: *mut c_char) -> c_int {
    unsafe {
        let Some(initstr) = os_getenv_cstr(env) else {
            return FAIL;
        };

        estack_push(ETYPE_ENV, env, 0);
        let save_current_sctx = g_st().current_sctx;
        g_st().current_sctx.sc_sid = SID_ENV;
        g_st().current_sctx.sc_seq = 0;
        g_st().current_sctx.sc_lnum = 0;
        do_cmdline_cmd(initstr.as_ptr());

        estack_pop();
        g_st().current_sctx = save_current_sctx;

        OK
    }
}

unsafe fn mainerr(msg1: *const c_char, msg2: *const c_char, msg3: *const c_char) -> ! {
    unsafe {
        print_mainerr(msg1, msg2, msg3);
        os_exit(1);
    }
}

unsafe fn print_mainerr(msg1: *const c_char, msg2: *const c_char, msg3: *const c_char) {
    unsafe {
        let prgname = path_tail(main_init_st().ARGV0);
        signal_stop();

        let stderr = log_get_stderr();
        fprintf!(stderr, c"%s: %s".as_ptr(), prgname, gettext(msg1));
        if !msg2.is_null() {
            fprintf!(stderr, c": \"%s\"".as_ptr(), msg2);
        }
        if !msg3.is_null() {
            fprintf!(stderr, c": \"%s\"".as_ptr(), msg3);
        }
        fprintf!(stderr, gettext(c"\nMore info with \"".as_ptr()));
        fprintf!(stderr, c"%s -h\"\n".as_ptr(), prgname);
    }
}

unsafe fn version() {
    unsafe {
        nlua_init(ptr::null_mut(), 0, -1);
        g_st().info_message = true;
        list_version();
        msg_putchar(b'\n' as c_int);
        g_st().msg_didout = false;
    }
}

unsafe fn usage() {
    unsafe {
        signal_stop();

        printf!(gettext(c"Usage:\n".as_ptr()));
        printf!(gettext(c"  nvim [options] [file ...]\n".as_ptr()));
        printf!(gettext(c"\nOptions:\n".as_ptr()));
        printf!(gettext(
            c"  --cmd <cmd>           Execute <cmd> before any config\n".as_ptr(),
        ));
        printf!(gettext(
            c"  +<cmd>, -c <cmd>      Execute <cmd> after config and first file\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -l <script> [args...] Execute Lua <script> (with optional args)\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -S <session>          Source <session> after loading the first file\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -s <scriptin>         Read Normal mode commands from <scriptin>\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -u <config>           Use this config file\n".as_ptr(),
        ));
        printf!(c"\n".as_ptr());
        printf!(gettext(c"  -d                    Diff mode\n".as_ptr()));
        printf!(gettext(
            c"  -es, -Es              Silent (batch) mode\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -h, --help            Print this help message\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -i <shada>            Use this shada file\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -n                    No swap file, use memory only\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -o[N]                 Open N windows (default: one per file)\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -O[N]                 Open N vertical windows (default: one per file)\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -p[N]                 Open N tab pages (default: one per file)\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -R                    Read-only (view) mode\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -v, --version         Print version information\n".as_ptr(),
        ));
        printf!(gettext(
            c"  -V[N][file]           Verbose [level][file]\n".as_ptr(),
        ));
        printf!(c"\n".as_ptr());
        printf!(gettext(
            c"  --                    Only file names after this\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --api-info            Write msgpack-encoded API metadata to stdout\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --clean               \"Factory defaults\" (skip user config and plugins, shada)\n"
                .as_ptr(),
        ));
        printf!(gettext(
            c"  --embed               Use stdin/stdout as a msgpack-rpc channel\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --headless            Don't start a user interface\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --listen <address>    Serve RPC API from this address\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --remote[-subcommand] Execute commands remotely on a server\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --server <address>    Connect to this Nvim server\n".as_ptr(),
        ));
        printf!(gettext(
            c"  --startuptime <file>  Write startup timing messages to <file>\n".as_ptr(),
        ));
        printf!(gettext(
            c"\nSee \":help startup-options\" for all options.\n".as_ptr(),
        ));
    }
}

unsafe fn check_swap_exists_action() {
    unsafe {
        if g_st().swap_exists_action == SEA_QUIT {
            ui_call_error_exit(1);
            getout(1);
        }
        handle_swap_exists(ptr::null_mut());
    }
}

#[cfg(test)]
pub(crate) unsafe fn ensure_test_editor_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe {
        g_st().headless_mode = true;
        g_st().silent_mode = true;
        event_init();
        estack_init();
        cmdline_init();
        eval_init();
        init_path(c"nvim".as_ptr());
        init_normal_cmds();
        highlight_init();
        mc_init();
        init_locale();
        set_init_tablocal();
        win_alloc_first();
        g_st().global_alist.id = 0;
        init_homedir();
        set_init_1(true);
        set_lang_var();
        qf_init_stack();
        set_init_2(true);
        crate::highlight_group::init_highlight(true, false);
    });
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

            let mut params = mparm_T::new();
            init_params(&raw mut params, 3, ptr::null_mut());
            assert_eq!(params.argc, 3);
            assert_eq!(params.window_count, -1);
            assert_eq!(params.use_debug_break_level, -1);
            assert_eq!(params.lua_arg0, -1);
        }
    }
}
