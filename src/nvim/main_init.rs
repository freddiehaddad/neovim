#![forbid(unsafe_code)]

//! Main entry point, CLI argument parsing, startup, and shutdown (`src/nvim/main.c`).

use crate::api::private::helpers::{
    api_free_object, api_metadata_raw, boolean_obj, cstr_as_obj, cstr_as_string, integer_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::api::ui::remote_ui_wait_for_attach;
use crate::arglist::{alist_add, alist_name, garg_entry, gargcount, global_alist_rc};
use crate::autocmd::{
    EVENT_BUFENTER, EVENT_BUFUNLOAD, EVENT_BUFWINLEAVE, EVENT_VIMENTER, EVENT_VIMLEAVE,
    EVENT_VIMLEAVEPRE, apply_autocmds, autocmd_init, block_autocmds, is_autocmd_blocked,
    unblock_autocmds,
};
use crate::buffer::{
    BLN_LISTED, Bufref, MAXLNUM, SEA_DIALOG, SEA_NONE, SEA_QUIT, VAR_FIXED, buf_get_changedtick,
    buf_is_empty, buf_set_changedtick, buf_valid, buflist_new, bufref_valid, do_autochdir,
    do_modelines, handle_swap_exists, open_buffer, set_buflisted, set_bufref, set_curbuf, setfname,
};
use crate::channel::{
    CallbackReader, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO, channel_connect,
    channel_from_stdio, channel_init, channel_teardown,
};
use crate::diff::{diff_win_options, diffopt_horizontal};
use crate::drawscreen::{
    UPD_NOT_VALID, UPD_VALID, default_grid_alloc, redraw_all_later, redraw_later, screenclear,
};
use crate::errors::*;
use crate::eval::typval::{
    Varnumber, tv_clear, tv_list_alloc, tv_list_alloc_ret, tv_list_append_string, tv_list_set_lock,
};
use crate::eval::userfunc::invoke_all_defer;
use crate::eval::vars::{
    VV_ARGF, VV_ERRMSG, VV_EXITING, VV_EXITREASON, VV_OLDFILES, VV_PROGNAME, VV_PROGPATH,
    VV_SEND_SERVER, VV_STARTTIME, VV_SWAPCOMMAND, VV_VIM_DID_ENTER, VV_VIM_DID_INIT,
    get_vim_var_list, get_vim_var_str, set_reg_var, set_vim_var_list, set_vim_var_nr,
    set_vim_var_string, set_vim_var_type,
};
use crate::eval::{
    K_LIST_LEN_MAY_KNOW_CONST, eval_has_provider, eval_init, garbage_collect, set_argv_var,
    timer_teardown,
};
use crate::event::r#loop::{loop_close, loop_init, loop_poll_events, loop_queues, uv_strerror};
use crate::event::multiqueue::{multiqueue_new_child, multiqueue_process_events};
use crate::event::proc::proc_teardown;
use crate::event::socket::socket_address_tcp_host_end;
use crate::event::stream::{UV_NAMED_PIPE, UV_TCP};
use crate::ex_cmds::{ECMD_HIDE, ECMD_LASTL, K_OPT_SHORTMESS, do_ecmd};
use crate::ex_docmd::{do_cmdline_cmd, filetype_maybe_enable, filetype_plugin_enable};
use crate::ex_getln::cmdline_init;
use crate::extmark::TriState;
use crate::fileio::{READ_NEW, READ_STDIN, fprintf, printf, readfile, shorten_fnames};
use crate::globals::*;
use crate::highlight::highlight_init;
use crate::highlight_group::init_highlight;
use crate::input::{open_scriptin, stuffchar_readbuff, vgetc};
use crate::keycodes::K_NOP;
use crate::log::{
    LOGLVL_DBG, LOGLVL_INF, LOGLVL_WRN, log_get_stderr, log_init, logmsg, logmsg_str,
};
use crate::lua::converter::{LUA_GLOBALSINDEX, lua_getfield, lua_pushstring, lua_tolstring_cstr};
use crate::lua::executor::{
    K_RET_OBJECT, get_global_lstate, nlua_call_typval, nlua_exec, nlua_exec_file, nlua_init,
    nlua_init_defaults, nlua_pcall,
};
use crate::mark::setpcmark;
use crate::mcursor::mc_init;
use crate::memline::{ml_close_all, ml_close_notmod, ml_recover, ml_sync_all, recover_names};
use crate::memory::{entered_free_all_mem, free_all_mem};
use crate::message::{msg_putchar, semsg, wait_return};
use crate::mouse::setmouse;
use crate::r#move::update_topline;
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::msgpack_rpc::server::{server_init, server_teardown};
use crate::normal::{
    K_OPT_CB_FLAG_UNNAMED_CONST, K_OPT_CB_FLAG_UNNAMEDPLUS_CONST, check_scrollbind,
    init_normal_cmds, normal_enter,
};
use crate::option::{
    K_OPT_ARABIC, K_OPT_KEYMAP, K_OPT_RIGHTLEFT, K_OPT_WINDOW, OptInt, reset_modifiable,
    set_init_1, set_init_2, set_init_3, set_init_tablocal, set_option_direct,
    set_option_value_give_err, set_options_bin,
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
    CASE_INSENSITIVE_FILENAME, CFile, os_exepath, os_guess_handle, os_isdir, os_path_exists,
    os_write,
};
use crate::os::input::{os_breakcheck, os_isatty};
use crate::os::lang::{init_locale, set_lang_var};
#[cfg(unix)]
use crate::os::signal::os_reset_sigpipe;
use crate::os::signal::{signal_init, signal_reject_deadly, signal_stop, signal_teardown};
use crate::os::stdpaths::{
    XDGVarType, appname_is_valid, get_appname, stdpaths_get_xdg_var, stdpaths_user_conf_subpath,
};
use crate::os::time::os_realtime;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    K_PATH_CMP_EXPAND, concat_fnames, path_equal, path_fix_case, path_guess_exepath, path_tail,
    vim_full_name,
};
use crate::profile::{profile_dump, time_finish, time_init, time_msg, time_start};
use crate::quickfix::{K_OPT_ERRORFILE, qf_init, qf_init_stack, qf_jump};
use crate::register::get_default_register_name;
use crate::runtime::{
    DOSO_NONE, DOSO_VIMRC, ETYPE_ARGS, ETYPE_ENV, SID_CARG, SID_CMDARG, SID_ENV, do_source,
    estack_init, estack_pop, estack_push, load_plugins,
};
use crate::shada::{shada_read_everything, shada_write_file};
use crate::strings::{format_to_buf, gettext};
use crate::syntax::syn_maybe_enable;
use crate::terminal::{terminal_init, terminal_teardown};
use crate::ui::{
    do_autocmd_uienter_all, ui_active, ui_call_error_exit, ui_call_set_title, ui_flush, ui_init,
};
use crate::ui_client::{ui_client_run, ui_client_start_server, ui_client_stop};
use crate::ui_compositor::ui_comp_syn_init;
use crate::version::{MAIN_EXITFREE, list_version};
use crate::window::{
    goto_tabpage, make_tabpages, make_windows, only_one_window, win_alloc_first, win_close,
    win_count, win_enter, win_equal, win_init_size, win_new_screensize, win_valid,
};
use std::ffi::CStr;
use std::io::Write;
use std::ptr;

#[cfg(windows)]
use crate::os::os_win_console::{
    os_fill_windows_version, os_icon_init, os_icon_reset, os_title_reset, os_title_save,
};

pub const MAX_ARG_CMDS: usize = 10;

const NO_BUFFERS: i32 = 1;

const WIN_HOR: i32 = 1;
const WIN_VER: i32 = 2;
const WIN_TABS: i32 = 3;

const EDIT_NONE: i32 = 0;
const EDIT_FILE: i32 = 1;
const EDIT_STDIN: i32 = 2;
const EDIT_TAG: i32 = 3;
const EDIT_QF: i32 = 4;

const K_OPT_SHADAFILE: OptIndex = 259;
const K_OPT_VERBOSEFILE: OptIndex = 344;

const PATHSEP: u8 = b'/';

const ERR_ARG_MISSING: &CStr = c"Argument missing after";
const ERR_OPT_GARBAGE: &CStr = c"Garbage after option argument";
const ERR_OPT_UNKNOWN: &CStr = c"Unknown option argument";
const ERR_TOO_MANY_ARGS: &CStr = c"Too many edit arguments";
const ERR_EXTRA_CMD: &CStr =
    c"Too many \"+command\", \"-c command\" or \"--cmd command\" arguments";

pub struct Mparm {
    pub argc: i32,
    pub argv: Vec<std::ffi::CString>,

    pub use_vimrc: Option<std::ffi::CString>,
    pub clean: bool,

    pub commands: Vec<std::ffi::CString>,
    pub pre_commands: Vec<std::ffi::CString>,
    pub luaf: Option<std::ffi::CString>,
    pub lua_arg0: i32,

    pub edit_type: i32,
    pub tagname: Option<std::ffi::CString>,
    pub use_ef: Option<std::ffi::CString>,

    pub exmode: bool,
    pub input_istext: bool,

    pub no_swap_file: bool,
    pub use_debug_break_level: i32,
    pub window_count: i32,
    pub window_layout: i32,

    pub diff_mode: i32,

    pub listen_addr: Option<std::ffi::CString>,
    pub remote: i32,
    pub server_addr: Option<std::ffi::CString>,
    pub scriptin: Option<std::ffi::CString>,
    pub scriptout: Option<std::ffi::CString>,
    pub scriptout_append: bool,
    pub had_stdin_file: bool,
}

impl Mparm {
    pub const fn new() -> Self {
        Self {
            argc: 0,
            argv: Vec::new(),
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
            no_swap_file: false,
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

impl Default for Mparm {
    fn default() -> Self {
        Self::new()
    }
}

struct MainInitState {
    argv0: std::cell::RefCell<Option<std::ffi::CString>>,
    really_exiting: std::cell::Cell<bool>,
}

impl MainInitState {
    const fn new() -> Self {
        Self {
            argv0: std::cell::RefCell::new(None),
            really_exiting: std::cell::Cell::new(false),
        }
    }
}

std::thread_local! {
    static MAIN_INIT_STATE: MainInitState = const { MainInitState::new() };
}

#[inline]
fn main_init_st<R>(f: impl FnOnce(&MainInitState) -> R) -> R {
    MAIN_INIT_STATE.with(f)
}

use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
#[cfg(windows)]
use crate::fileio::_setmode;
#[cfg(unix)]
use crate::fileio::tcdrain;
#[cfg(windows)]
use crate::fileio::{close, dup2};
use crate::os::fs::MAXPATHL;

#[inline]
fn time_msg_if_enabled(s: &std::ffi::CStr) {
    if has_time_fd() {
        time_msg(s, None);
    }
}

#[cfg(windows)]
#[inline]
fn to_slash_if_needed(buf: &mut [u8]) {
    if !buf.is_empty() {
        path_to_slash(buf);
    }
}

#[inline]
fn cstring_to_slash_if_needed(s: &mut std::ffi::CString) {
    #[cfg(windows)]
    {
        let mut bytes = std::mem::take(s).into_bytes_with_nul();
        to_slash_if_needed(&mut bytes);
        *s = std::ffi::CString::from_vec_with_nul(bytes).unwrap_or_default();
    }
    #[cfg(not(windows))]
    {
        let _ = s;
    }
}

#[inline]
fn strnicmp_ascii(a: &[u8], b: &[u8]) -> bool {
    a.len() >= b.len() && a[..b.len()].eq_ignore_ascii_case(b)
}

pub fn event_init() {
    loop_init(&mut g_st().main_loop);
    env_init();
    let queues = loop_queues(&mut g_st().main_loop);
    g_st().resize_events = multiqueue_new_child(queues.events);

    autocmd_init();
    signal_init();
    channel_init();
    terminal_init();
    ui_init();
    time_msg_if_enabled(c"event init");
}

fn event_teardown() -> bool {
    let queues = loop_queues(&mut g_st().main_loop);
    if queues.events.is_null() {
        return true;
    }

    multiqueue_process_events(queues.events.borrow_mut());
    loop_poll_events(&mut g_st().main_loop, 0);
    server_teardown();
    channel_teardown();
    proc_teardown(&mut g_st().main_loop);
    timer_teardown();
    signal_teardown();
    terminal_teardown();

    loop_close(&mut g_st().main_loop, true)
}

fn early_init(paramp: &mut Mparm) {
    os_hint_priority();
    estack_init();
    cmdline_init();
    eval_init();
    set_vim_var_nr(VV_STARTTIME, os_realtime() as Varnumber);
    let argv0_clone = main_init_st(|st| st.argv0.borrow().clone());
    init_path(argv0_clone.as_deref().unwrap_or(c"nvim"));
    init_normal_cmds();
    highlight_init();
    mc_init();

    #[cfg(windows)]
    {
        os_fill_windows_version(&mut g_st().windows_version);
    }

    time_msg_if_enabled(c"early init");

    init_locale();
    set_init_tablocal();

    win_alloc_first();
    time_msg_if_enabled(c"init first window");

    global_alist_rc().id.set(0);

    init_homedir();
    set_init_1(paramp.clean);
    log_init();
    time_msg_if_enabled(c"inits 1");

    set_lang_var();
    qf_init_stack();
}

pub fn nvim_main() -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        os_reset_sigpipe();

        let mut c_args: Vec<std::ffi::CString> = std::env::args_os()
            .map(|arg| crate::memory::bytes_to_cstring(&arg.into_vec()))
            .collect();
        if c_args.is_empty() {
            c_args.push(c"nvim".to_owned());
        }
        nvim_main_inner(c_args)
    }
    #[cfg(windows)]
    {
        let mut c_args: Vec<std::ffi::CString> = std::env::args_os()
            .map(|arg| crate::memory::bytes_to_cstring(arg.to_string_lossy().as_bytes()))
            .collect();
        if c_args.is_empty() {
            c_args.push(c"nvim".to_owned());
        }
        nvim_main_inner(c_args)
    }
}

fn nvim_main_inner(mut argv: Vec<std::ffi::CString>) -> i32 {
    if let Some(first) = argv.first_mut() {
        cstring_to_slash_if_needed(first);
        main_init_st(|st| *st.argv0.borrow_mut() = Some(first.clone()));
    } else {
        main_init_st(|st| *st.argv0.borrow_mut() = None);
    }

    if !appname_is_valid() {
        fprintf!(
            log_get_stderr(),
            c"$NVIM_APPNAME must be a name or relative path.\n",
        );
        std::process::exit(1);
    }

    let mut fname: Option<std::ffi::CString> = None;
    let mut params = Mparm::new();

    init_params(&mut params, argv);
    init_startuptime(&mut params);

    for arg in params.argv.iter().skip(1) {
        if arg.as_bytes().eq_ignore_ascii_case(b"--clean") {
            params.clean = true;
            break;
        }
    }

    event_init();
    early_init(&mut params);
    set_argv_var(&params.argv);
    check_and_set_isatty(&mut params);
    command_line_scan(&mut params);
    set_argf_var();

    let argv_vec = params.argv.clone();
    nlua_init(&argv_vec, params.lua_arg0);
    time_msg_if_enabled(c"init lua interpreter");

    #[cfg(windows)]
    let mut startup_stderr_fd: i32 = -1;
    #[cfg(windows)]
    if g_st().embedded_mode {
        startup_stderr_fd = os_dup_cloexec(STDERR_FILENO);
    }

    let stdin_type = os_guess_handle(STDIN_FILENO);
    let stdio_rpc =
        params.listen_addr.is_none() || stdin_type == UV_NAMED_PIPE || stdin_type == UV_TCP;
    if g_st().embedded_mode && stdio_rpc {
        let mut err = None;
        let on_data = CallbackReader::none();
        if channel_from_stdio(true, on_data, &mut err) == 0 {
            std::process::abort();
        }
    }

    if gargcount() > 0 {
        fname = get_fname(&mut params);
    }

    if g_st().recoverymode && fname.is_none() {
        g_st().headless_mode = true;
    }

    #[cfg(windows)]
    let has_term = true;
    #[cfg(not(windows))]
    let has_term = g_st().stdin_isatty || g_st().stdout_isatty || g_st().stderr_isatty;

    let use_builtin_ui =
        has_term && !g_st().headless_mode && !g_st().embedded_mode && !g_st().silent_mode;

    if params.remote != 0 {
        let server_addr = params.server_addr.clone();
        let remote = params.remote;
        remote_request(&mut params, remote, server_addr.as_deref(), use_builtin_ui);
    }

    let remote_ui = g_st().ui_client_channel_id != 0;

    if use_builtin_ui && !remote_ui {
        g_st().ui_client_forward_stdin = !g_st().stdin_isatty;
        let rv = ui_client_start_server(&get_vim_var_str(VV_PROGPATH), &params.argv);
        if rv == 0 {
            fprintf!(log_get_stderr(), c"Failed to start Nvim server!\n");
            os_exit(1);
        }
        g_st().ui_client_channel_id = rv;
    }

    if g_st().ui_client_channel_id != 0 {
        ui_client_run(if remote_ui {
            params.server_addr.as_deref()
        } else {
            None
        });
    }
    assert!(g_st().ui_client_channel_id == 0 && !use_builtin_ui);

    if !server_init(params.listen_addr.as_deref()) {
        #[cfg(windows)]
        if startup_stderr_fd >= 0 {
            dup2(startup_stderr_fd, STDERR_FILENO);
            close(startup_stderr_fd);
        }
        mainerr(g_st().iobuff_cstr(), None, None);
    }

    #[cfg(windows)]
    if startup_stderr_fd >= 0 {
        close(startup_stderr_fd);
    }

    time_msg_if_enabled(c"expanding arguments");

    if params.diff_mode != 0 && params.window_count == -1 {
        params.window_count = 0;
    }
    g_st().redrawing_disabled += 1;

    crate::log::log_get_stdout().set_unbuffered();

    g_st().full_screen = !g_st().silent_mode;

    win_init_size();
    if params.diff_mode != 0 {
        diff_win_options(firstwin_mut(), false);
    }

    assert!(
        opt_vars().p_ch >= 0
            && i64::from(g_st().rows_global) >= opt_vars().p_ch
            && i64::from(g_st().rows_global) - opt_vars().p_ch <= i64::from(i32::MAX)
    );
    g_st().cmdline_row = g_st().rows_global - opt_vars().p_ch as i32;
    g_st().msg_row = g_st().cmdline_row;
    default_grid_alloc();
    set_init_2(g_st().headless_mode);
    time_msg_if_enabled(c"inits 2");

    g_st().msg_scroll = 1;
    g_st().no_wait_return += 1;

    init_highlight(true, false);
    ui_comp_syn_init();
    time_msg_if_enabled(c"init highlight");

    g_st().debug_break_level = params.use_debug_break_level;

    let use_remote_ui = g_st().embedded_mode && !g_st().headless_mode;
    if use_remote_ui {
        if !stdio_rpc {
            fprintf!(
                log_get_stderr(),
                c"Nvim server (pid: %lld) waiting for UI, listening on: \"%s\"\n",
                os_get_pid(),
                get_vim_var_str(VV_SEND_SERVER),
            );
        }
        time_msg_if_enabled(c"waiting for UI");
        remote_ui_wait_for_attach();
        time_msg_if_enabled(c"done waiting for UI");
        firstwin_mut().w_prev_height = firstwin_mut().w_height;
    }

    set_starting(NO_BUFFERS);
    screenclear();
    win_new_screensize();
    time_msg_if_enabled(c"clear screen");

    if edit_stdin(&mut params) {
        params.edit_type = EDIT_STDIN;
    }

    if let Some(ref scriptin) = params.scriptin
        && !open_scriptin(scriptin)
    {
        os_exit(2);
    }
    if let Some(ref scriptout_name) = params.scriptout {
        g_st().scriptout = CFile::open(
            scriptout_name,
            if params.scriptout_append {
                c"ab"
            } else {
                c"wb"
            },
        );
        if !has_scriptout() {
            let stderr = log_get_stderr();
            fprintf!(stderr, gettext(c"Cannot open for script output: \""),);
            fprintf!(stderr, c"%s\"\n", scriptout_name);
            os_exit(2);
        }
    }

    nlua_init_defaults();
    time_msg_if_enabled(c"init default mappings & autocommands");

    let vimrc_none = params.use_vimrc.as_deref() == Some(c"NONE");

    if vimrc_none && !params.clean {
        opt_vars().p_lpl = false;
    }

    exe_pre_commands(&mut params);

    if !vimrc_none || params.clean {
        filetype_plugin_enable();
    }

    source_startup_scripts(&params);

    if !vimrc_none || params.clean {
        filetype_maybe_enable();
        syn_maybe_enable();
    }

    set_vim_var_nr(VV_VIM_DID_INIT, 1);

    load_plugins();

    set_window_layout(&mut params);

    if g_st().recoverymode && fname.is_none() {
        let mut items_tv = crate::eval::typval::TypVal::new();
        tv_list_alloc_ret(&mut items_tv, 0);
        recover_names(None, false, items_tv.vval.v_list().borrow_mut());
        let mut lua_args = [items_tv, crate::eval::typval::TypVal::new()];
        nlua_call_typval(c"vim._core.swapfile", c"list_swaps", &mut lua_args, None);
        tv_clear(&mut lua_args[0]);
        os_exit(0);
    }

    set_init_3();
    time_msg_if_enabled(c"inits 3");

    if params.no_swap_file {
        opt_vars().p_uc = 0;
    }

    if g_st().silent_mode {
        opt_vars().p_ut = 1;
    }

    if !opt_vars().p_shada.is_empty() {
        shada_read_everything(None, false, true);
        time_msg_if_enabled(c"reading ShaDa");
    }
    if get_vim_var_list(VV_OLDFILES).is_none() {
        set_vim_var_list(VV_OLDFILES, Some(tv_list_alloc(0)));
    }

    handle_quickfix(&mut params);

    set_starting(NO_BUFFERS);
    g_st().no_wait_return = 0;
    g_st().msg_scroll = 0;

    if params.edit_type == EDIT_STDIN && !g_st().recoverymode {
        read_stdin();
    }

    setmouse();

    redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);

    g_st().no_wait_return += 1;

    create_windows(&mut params);
    time_msg_if_enabled(c"opening buffers");

    set_vim_var_string(VV_SWAPCOMMAND, None, -1);

    if params.exmode {
        curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
    }

    apply_autocmds(EVENT_BUFENTER, None, None, false, curbuf_opt_mut());
    time_msg_if_enabled(c"BufEnter autocommands");
    setpcmark();

    if params.edit_type == EDIT_QF {
        qf_jump(None, 0, 0, false);
        time_msg_if_enabled(c"jump to first error");
    }

    edit_buffers(&mut params);

    if params.diff_mode != 0 {
        for wp in win_iter_ptr(curtab_mut().tp_firstwin.get().as_ptr()) {
            if !wp.w_arg_idx_invalid {
                diff_win_options(&mut *wp, true);
            }
        }
    }

    shorten_fnames(false);

    handle_tag(params.tagname.as_deref());

    if !params.commands.is_empty() {
        exe_commands(&mut params);
    }

    set_starting(0);

    g_st().redrawing_disabled = 0;
    redraw_all_later(UPD_NOT_VALID);
    g_st().no_wait_return = 0;

    do_autochdir();

    set_vim_var_nr(VV_VIM_DID_ENTER, 1);
    apply_autocmds(EVENT_VIMENTER, None, None, false, curbuf_opt_mut());
    time_msg_if_enabled(c"VimEnter autocommands");
    if use_remote_ui {
        do_autocmd_uienter_all();
        time_msg_if_enabled(c"UIEnter autocommands");
    }

    #[cfg(windows)]
    {
        if use_remote_ui {
            os_icon_init();
        }
        os_title_save();
    }

    set_reg_var(get_default_register_name());

    if curwin_mut().w_onebuf_opt.wo_diff && curwin_mut().w_onebuf_opt.wo_scb {
        update_topline(curwin_mut());
        check_scrollbind(0, 0);
        time_msg_if_enabled(c"diff scrollbinding");
    }

    if g_st().restart_edit != 0 {
        stuffchar_readbuff(K_NOP);
    }

    if (opt_vars().cb_flags & (K_OPT_CB_FLAG_UNNAMED_CONST | K_OPT_CB_FLAG_UNNAMEDPLUS_CONST)) != 0
    {
        eval_has_provider(b"clipboard", false);
    }

    if let Some(ref luaf) = params.luaf {
        g_st().msg_scroll = 1;
        logmsg_str(
            LOGLVL_DBG,
            None,
            Some(c"main"),
            line!() as i32,
            true,
            c"executing Lua -l script",
        );
        let lua_ok = nlua_exec_file(luaf.as_c_str());
        time_msg_if_enabled(c"executing Lua -l script");
        if g_st().msg_didout {
            msg_putchar(b'\n' as i32);
            g_st().msg_didout = false;
        }
        getout(if lua_ok { 0 } else { 1 });
    } else if g_st().silent_mode {
        g_st().msg_scroll = 1;
        g_st().no_wait_return += 1;
        if !params.input_istext && !g_st().stdin_isatty {
            logmsg_str(
                LOGLVL_DBG,
                None,
                Some(c"main"),
                line!() as i32,
                true,
                c"executing stdin as Ex commands",
            );
            let mut args = [crate::eval::typval::TypVal::new(); 1];
            nlua_call_typval(c"vim._core.exmode", c"run", &mut args, None);
        }
        if g_st().msg_didout {
            msg_putchar(b'\n' as i32);
            g_st().msg_didout = false;
        }
        getout(0);
    }

    if params.exmode {
        do_cmdline_cmd(c"exmode");
    }

    time_msg_if_enabled(c"before starting main loop");
    logmsg_str(
        LOGLVL_INF,
        None,
        Some(c"main"),
        line!() as i32,
        true,
        c"starting main loop",
    );

    normal_enter();

    0
}

/// Prompts/dialogs should not wait for a key (no chance of meaningful input).
pub fn input_unanswerable() -> bool {
    g_st().v_dying != 0 || (g_st().exiting && ui_active() == 0)
}

pub fn os_exit(mut r: i32) -> ! {
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
        None,
        Some(c"os_exit"),
        line!() as i32,
        true,
        CStr::from_bytes_until_nul(&msg).unwrap_or(c""),
    );

    if MAIN_EXITFREE {
        free_all_mem();
    }

    std::process::exit(r);
}

pub fn getout(mut exitval: i32) -> ! {
    assert!(g_st().ui_client_channel_id == 0);
    g_st().exiting = true;

    time_finish();

    if g_st().exmode_active {
        exitval += g_st().ex_exitval;
    }

    set_vim_var_type(VV_EXITING, crate::eval::typval::VarType::VarNumber);
    set_vim_var_nr(VV_EXITING, Varnumber::from(exitval));

    if get_vim_var_str(VV_EXITREASON).is_empty() {
        set_vim_var_string(VV_EXITREASON, Some(c"quit"), 4);
    }

    invoke_all_defer();

    if g_st().v_dying <= 1 {
        let mut next_tp: Option<crate::globals::TabpagePtr>;
        let mut tp_opt = (!g_st().first_tabpage.is_null()).then(|| g_st().first_tabpage.clone());
        while let Some(tp) = tp_opt {
            next_tp = tp.next_opt();
            let first_wp = if g_st().curtab == tp {
                g_st().firstwin.as_ptr()
            } else {
                tp.tp_firstwin.get().as_ptr()
            };
            for wp in win_iter_mut(first_wp) {
                if !buf_valid(wp.buf_opt()) {
                    continue;
                }
                let buf = wp.buf_mut();
                let buf_ptr = crate::globals::BufPtr::from(&*buf);
                if buf_get_changedtick(buf) != -1 {
                    let mut bufref: Bufref = Bufref::new();
                    set_bufref(&mut bufref, buf_ptr.opt_mut());
                    let b_fname = buf.fname_opt();
                    apply_autocmds(
                        EVENT_BUFWINLEAVE,
                        b_fname,
                        b_fname,
                        false,
                        buf_ptr.opt_mut(),
                    );
                    if let Some(buf_now) = bufref.buf_opt().filter(|_| bufref_valid(Some(&bufref)))
                    {
                        buf_set_changedtick(buf_now, -1);
                    }
                    next_tp =
                        (!g_st().first_tabpage.is_null()).then(|| g_st().first_tabpage.clone());
                    break;
                }
            }
            tp_opt = next_tp;
        }

        for buf in for_all_buffers_ptr() {
            if buf.b_ml.ml_mfp.is_some() {
                let mut bufref: Bufref = Bufref::new();
                set_bufref(&mut bufref, Some(&mut *buf));
                let b_fname = buf.fname_opt().map(CStr::to_owned);
                apply_autocmds(
                    EVENT_BUFUNLOAD,
                    b_fname.as_deref(),
                    b_fname.as_deref(),
                    false,
                    Some(&mut *buf),
                );
                if !bufref_valid(Some(&bufref)) {
                    break;
                }
            }
        }

        let mut unblock = 0;
        if is_autocmd_blocked() {
            unblock_autocmds();
            unblock += 1;
        }
        apply_autocmds(EVENT_VIMLEAVEPRE, None, None, false, curbuf_opt_mut());
        if unblock != 0 {
            block_autocmds();
        }
    }

    if !entered_free_all_mem() && !opt_vars().p_shada.is_null() && !opt_vars().p_shada.is_empty() {
        shada_write_file(None, false);
    }

    if g_st().v_dying <= 1 {
        let mut unblock = 0;
        if is_autocmd_blocked() {
            unblock_autocmds();
            unblock += 1;
        }
        apply_autocmds(EVENT_VIMLEAVE, None, None, false, curbuf_opt_mut());
        if unblock != 0 {
            block_autocmds();
        }
    }

    profile_dump();

    if g_st().did_emsg != 0 {
        if !g_st().headless_mode && input_unanswerable() {
            logmsg!(
                LOGLVL_WRN,
                None,
                Some(c"getout"),
                line!() as i32,
                true,
                c"Exiting with error: \"%s\"",
                get_vim_var_str(VV_ERRMSG),
            );
        }
        g_st().no_wait_return = 0;
        wait_return(0);
    }

    if opt_vars().p_title && !opt_vars().p_titleold.is_empty() {
        ui_call_set_title(cstr_as_string(Some(opt_vars().p_titleold.as_cstr())));
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

pub fn preserve_exit(errmsg: Option<&CStr>) -> ! {
    if main_init_st(|st| st.really_exiting.replace(true)) {
        std::process::exit(2);
    }
    g_st().exiting = true;
    signal_reject_deadly();

    if g_st().ui_client_channel_id != 0 {
        ui_client_stop();
    }
    let stderr = log_get_stderr();
    if let Some(errmsg) = errmsg
        && !errmsg.is_empty()
    {
        let bytes = errmsg.to_bytes();
        let has_eol = bytes.last() == Some(&b'\n');
        fprintf!(stderr, if has_eol { c"%s" } else { c"%s\n" }, errmsg,);
    }
    if g_st().ui_client_channel_id != 0 {
        os_exit(1);
    }

    ml_close_notmod();

    for buf in for_all_buffers_mut() {
        if let Some(mfp) = buf.b_ml.ml_mfp.as_ref()
            && mfp.mf_fname.is_some()
        {
            if errmsg.is_some() {
                fprintf!(stderr, c"Nvim: preserving files...\n");
            }
            ml_sync_all(0, 0, true);
            break;
        }
    }

    ml_close_all(false);

    if errmsg.is_some() {
        fprintf!(stderr, c"Nvim: Finished.\n");
    }

    getout(1);
}

fn get_number_arg(p: &[u8], idx: &mut i32, mut def: i32) -> i32 {
    let mut i = *idx as usize;
    if p.get(i).is_some_and(u8::is_ascii_digit) {
        def = 0;
        while let Some(&b) = p.get(i)
            && b.is_ascii_digit()
        {
            def = def.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
            i += 1;
        }
        *idx = i as i32;
    }
    def
}

fn server_connect(server_addr: Option<&CStr>, errmsg: &mut Option<&'static CStr>) -> u64 {
    let Some(server_addr_cstr) = server_addr else {
        *errmsg = Some(c"no address specified");
        return 0;
    };
    let on_data = CallbackReader::none();
    let mut error = None;
    let is_tcp = socket_address_tcp_host_end(server_addr_cstr.to_bytes()).is_some();
    let chan = channel_connect(
        is_tcp,
        server_addr_cstr,
        true,
        false,
        on_data,
        500,
        &mut error,
    );
    if let Some(err) = error {
        *errmsg = Some(err);
        return 0;
    }
    chan
}

fn remote_request(params: &mut Mparm, remote_args: i32, server_addr: Option<&CStr>, ui_only: bool) {
    let argc = params.argc;
    let is_ui = params
        .argv
        .get(remote_args as usize)
        .is_some_and(|s| s.as_bytes() == b"--remote-ui");
    if ui_only && !is_ui {
        return;
    }

    let stderr = log_get_stderr();
    let mut connect_error: Option<&'static CStr> = None;
    let chan = server_connect(server_addr, &mut connect_error);

    if is_ui {
        if chan == 0 {
            #[cfg(windows)]
            {
                dup2(STDOUT_FILENO, STDERR_FILENO);
            }
            fprintf!(
                stderr,
                c"Remote ui failed to start: %s\n",
                connect_error.unwrap_or(c""),
            );
            os_exit(1);
        } else {
            let mut nvim_env_buf = [0u8; MAXPATHL];
            let nvim_env = os_getenv_buf(c"NVIM", &mut nvim_env_buf);
            if server_addr.map(CStr::to_bytes) == nvim_env.map(CStr::to_bytes) {
                fprintf!(
                    stderr,
                    c"%s",
                    c"Cannot attach UI of :terminal child to its parent. ",
                );
                fprintf!(stderr, c"%s\n", c"(Unset $NVIM to skip this check)",);
                os_exit(1);
            }
        }
        g_st().ui_client_channel_id = chan;
        return;
    }

    let count = (argc - remote_args).max(0) as usize;
    let mut args_vec: Vec<Object> = (remote_args..argc)
        .map(|t_argc| cstr_as_obj(Some(params.argv[t_argc as usize].as_c_str())))
        .collect();
    let args = Array::from_slice(&mut args_vec[..count]);

    let mut err = Error::new();
    let mut a_items: [Object; 4] = [
        integer_obj(Integer::from(chan as i32)),
        cstr_as_obj(server_addr),
        cstr_as_obj(connect_error),
        Object {
            obj_type: ObjectType::ObjectTypeArray,
            data: ObjectData::Array(args),
        },
    ];
    let a = Array::from_slice(&mut a_items[..4]);
    let s = NvimString::from_cstr(c"return vim._cs_remote(...)");
    let o = nlua_exec(s, None, a, K_RET_OBJECT, None, &mut err);
    drop(args_vec);
    if err.err_type != ErrorType::ErrorTypeNone {
        fprintf!(stderr, c"%s\n", err.msg_cstr());
        os_exit(2);
    }

    if o.obj_type != ObjectType::ObjectTypeDict {
        fprintf!(stderr, c"vim._cs_remote returned unexpected value\n",);
        os_exit(2);
    }
    let dict: Dict = o.data.dict();

    let mut should_exit: TriState = K_NONE;
    let mut tabbed: TriState = K_NONE;

    for i in 0..dict.size {
        let item = &dict[i];
        if item.key.opt_bytes() == Some(b"errmsg") {
            if item.value.obj_type != ObjectType::ObjectTypeString {
                fprintf!(
                    stderr,
                    c"vim._cs_remote returned an unexpected type for 'errmsg'\n",
                );
                os_exit(2);
            }
            fprintf!(stderr, c"%s\n", item.value.data.string());
            os_exit(2);
        } else if item.key.opt_bytes() == Some(b"result") {
            if item.value.obj_type != ObjectType::ObjectTypeString {
                fprintf!(
                    stderr,
                    c"vim._cs_remote returned an unexpected type for 'result'\n",
                );
                os_exit(2);
            }
            printf!(c"%s", item.value.data.string());
        } else if item.key.opt_bytes() == Some(b"tabbed") {
            if item.value.obj_type != ObjectType::ObjectTypeBoolean {
                fprintf!(
                    stderr,
                    c"vim._cs_remote returned an unexpected type for 'tabbed'\n",
                );
                os_exit(2);
            }
            tabbed = if item.value.data.boolean() {
                K_TRUE
            } else {
                K_FALSE
            };
        } else if item.key.opt_bytes() == Some(b"should_exit") {
            if item.value.obj_type != ObjectType::ObjectTypeBoolean {
                fprintf!(
                    stderr,
                    c"vim._cs_remote returned an unexpected type for 'should_exit'\n",
                );
                os_exit(2);
            }
            should_exit = if item.value.data.boolean() {
                K_TRUE
            } else {
                K_FALSE
            };
        }
    }
    if should_exit == K_NONE || tabbed == K_NONE {
        fprintf!(
            stderr,
            c"vim._cs_remote didn't return a value for should_exit or tabbed, bailing\n",
        );
        os_exit(2);
    }
    api_free_object(o);

    if should_exit == K_TRUE {
        os_exit(0);
    }
    if tabbed == K_TRUE {
        params.window_count = argc - remote_args - 1;
        params.window_layout = WIN_TABS;
    }
}

fn edit_stdin(parmp: &mut Mparm) -> bool {
    let implicit = !g_st().headless_mode
        && !(g_st().embedded_mode && g_st().stdin_fd <= 0)
        && (!parmp.exmode || parmp.input_istext)
        && !g_st().stdin_isatty
        && parmp.edit_type <= EDIT_STDIN
        && parmp.scriptin.is_none();
    parmp.had_stdin_file || implicit
}

fn command_line_scan(parmp: &mut Mparm) {
    let raw_len = parmp.argv.len();
    let mut arg_idx: usize = 1;
    let mut argv_idx: i32 = 1;
    let mut had_minmin = false;

    while arg_idx < raw_len {
        let arg0_cstr = parmp.argv[arg_idx].clone();
        let arg0 = arg0_cstr.as_bytes();
        if arg0.first() == Some(&b'+') && !had_minmin {
            if parmp.commands.len() >= MAX_ARG_CMDS {
                mainerr(ERR_EXTRA_CMD, None, None);
            }
            argv_idx = -1;
            if arg0.len() == 1 {
                parmp.commands.push(c"$".to_owned());
            } else {
                parmp
                    .commands
                    .push(crate::memory::bytes_to_cstring(&arg0[1..]));
            }
        } else if arg0.first() == Some(&b'-') && !had_minmin {
            let mut want_argument = false;
            let c = arg0.get(argv_idx as usize).copied().unwrap_or(0);
            argv_idx += 1;
            match c {
                0 => {
                    if parmp.exmode {
                        g_st().silent_mode = true;
                        g_st().exmode_active = true;
                        parmp.no_swap_file = true;
                    } else {
                        if parmp.edit_type > EDIT_STDIN {
                            mainerr(ERR_TOO_MANY_ARGS, Some(&arg0_cstr), None);
                        }
                        parmp.had_stdin_file = true;
                        parmp.edit_type = EDIT_STDIN;
                    }
                    argv_idx = -1;
                }
                b'-' => {
                    let sub = arg0.get(argv_idx as usize..).unwrap_or(&[]);
                    if sub.eq_ignore_ascii_case(b"help") {
                        usage();
                        os_exit(0);
                    } else if sub.eq_ignore_ascii_case(b"version") {
                        version();
                        os_exit(0);
                    } else if sub.eq_ignore_ascii_case(b"api-info") {
                        #[cfg(windows)]
                        {
                            _setmode(STDOUT_FILENO, 0x8000);
                        }
                        let data = api_metadata_raw();
                        let written_bytes = os_write(STDOUT_FILENO, data.as_bytes(), false);
                        if written_bytes < 0 {
                            semsg!(
                                gettext(c"E5420: Failed to write to file: %s"),
                                uv_strerror(written_bytes as i32),
                            );
                        }
                        os_exit(0);
                    } else if sub.eq_ignore_ascii_case(b"headless") {
                        g_st().headless_mode = true;
                    } else if sub.eq_ignore_ascii_case(b"embed") {
                        g_st().embedded_mode = true;
                    } else if strnicmp_ascii(sub, b"listen") {
                        want_argument = true;
                        argv_idx += 6;
                    } else if strnicmp_ascii(sub, b"literal") {
                        // Do nothing: file args are always literal.
                    } else if strnicmp_ascii(sub, b"remote") {
                        parmp.remote = arg_idx as i32;
                    } else if strnicmp_ascii(sub, b"server") {
                        want_argument = true;
                        argv_idx += 6;
                    } else if strnicmp_ascii(sub, b"noplugin") {
                        opt_vars().p_lpl = false;
                    } else if strnicmp_ascii(sub, b"cmd") {
                        want_argument = true;
                        argv_idx += 3;
                    } else if strnicmp_ascii(sub, b"startuptime") {
                        want_argument = true;
                        argv_idx += 11;
                    } else if strnicmp_ascii(sub, b"clean") {
                        parmp.use_vimrc = Some(c"NONE".to_owned());
                        parmp.clean = true;
                        set_option_value_give_err(K_OPT_SHADAFILE, cstr_as_obj(Some(c"NONE")), 0);
                    } else if strnicmp_ascii(sub, b"luamod-dev") {
                        g_st().nlua_disable_preload = true;
                    } else {
                        if !sub.is_empty() {
                            mainerr(ERR_OPT_UNKNOWN, Some(&arg0_cstr), None);
                        }
                        had_minmin = true;
                    }
                    if !want_argument {
                        argv_idx = -1;
                    }
                }
                b'A' => {
                    set_option_value_give_err(K_OPT_ARABIC, boolean_obj(true), 0);
                }
                b'b' => {
                    set_options_bin(i32::from(curbuf_mut().b_p_bin), 1, 0);
                    curbuf_mut().b_p_bin = true;
                }
                b'D' => {
                    parmp.use_debug_break_level = 9999;
                }
                b'd' => {
                    parmp.diff_mode = 1;
                }
                b'e' => {
                    parmp.exmode = true;
                }
                b'E' => {
                    parmp.exmode = true;
                    parmp.input_istext = true;
                }
                b'f' | b'N' | b'X' => {}
                b'?' | b'h' => {
                    usage();
                    os_exit(0);
                }
                b'H' => {
                    set_option_value_give_err(K_OPT_KEYMAP, cstr_as_obj(Some(c"hebrew")), 0);
                    set_option_value_give_err(K_OPT_RIGHTLEFT, boolean_obj(true), 0);
                }
                b'M' => {
                    reset_modifiable();
                    opt_vars().p_write = false;
                }
                b'm' => {
                    opt_vars().p_write = false;
                }
                b'n' => {
                    parmp.no_swap_file = true;
                }
                b'p' => {
                    parmp.window_count = get_number_arg(arg0, &mut argv_idx, 0);
                    parmp.window_layout = WIN_TABS;
                }
                b'o' => {
                    parmp.window_count = get_number_arg(arg0, &mut argv_idx, 0);
                    parmp.window_layout = WIN_HOR;
                }
                b'O' => {
                    parmp.window_count = get_number_arg(arg0, &mut argv_idx, 0);
                    parmp.window_layout = WIN_VER;
                }
                b'q' => {
                    if parmp.edit_type != EDIT_NONE {
                        mainerr(ERR_TOO_MANY_ARGS, Some(&arg0_cstr), None);
                    }
                    parmp.edit_type = EDIT_QF;
                    if (argv_idx as usize) < arg0.len() {
                        parmp.use_ef =
                            Some(crate::memory::bytes_to_cstring(&arg0[argv_idx as usize..]));
                        argv_idx = -1;
                    } else if arg_idx + 1 < raw_len {
                        want_argument = true;
                    }
                }
                b'R' => {
                    g_st().readonlymode = true;
                    curbuf_mut().b_p_ro = true;
                    opt_vars().p_uc = 10000;
                }
                b'r' | b'L' => {
                    g_st().recoverymode = true;
                }
                b's' => {
                    if parmp.exmode {
                        g_st().silent_mode = true;
                        g_st().exmode_active = true;
                        parmp.no_swap_file = true;
                        if opt_vars().p_shadafile.is_null() || opt_vars().p_shadafile.is_empty() {
                            set_option_value_give_err(
                                K_OPT_SHADAFILE,
                                cstr_as_obj(Some(c"NONE")),
                                0,
                            );
                        }
                    } else {
                        want_argument = true;
                    }
                }
                b't' => {
                    if parmp.edit_type != EDIT_NONE {
                        mainerr(ERR_TOO_MANY_ARGS, Some(&arg0_cstr), None);
                    }
                    parmp.edit_type = EDIT_TAG;
                    if (argv_idx as usize) < arg0.len() {
                        parmp.tagname =
                            Some(crate::memory::bytes_to_cstring(&arg0[argv_idx as usize..]));
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
                    opt_vars().p_verbose = i64::from(get_number_arg(arg0, &mut argv_idx, 10));
                    if (argv_idx as usize) < arg0.len() {
                        set_option_value_give_err(
                            K_OPT_VERBOSEFILE,
                            cstr_as_obj(Some(crate::memory::CStrSubslice::advance(
                                arg0_cstr.as_c_str(),
                                argv_idx as usize,
                            ))),
                            0,
                        );
                        argv_idx = arg0.len() as i32;
                    }
                }
                b'w' => {
                    if arg0.get(argv_idx as usize).is_some_and(u8::is_ascii_digit) {
                        let n = get_number_arg(arg0, &mut argv_idx, 10);
                        set_option_value_give_err(K_OPT_WINDOW, integer_obj(OptInt::from(n)), 0);
                    } else {
                        want_argument = true;
                    }
                }
                b'c' => {
                    if (argv_idx as usize) < arg0.len() {
                        if parmp.commands.len() >= MAX_ARG_CMDS {
                            mainerr(ERR_EXTRA_CMD, None, None);
                        }
                        parmp
                            .commands
                            .push(crate::memory::bytes_to_cstring(&arg0[argv_idx as usize..]));
                        argv_idx = -1;
                    } else {
                        want_argument = true;
                    }
                }
                b'S' | b'i' | b'l' | b'u' | b'U' | b'W' => {
                    want_argument = true;
                }
                _ => {
                    mainerr(ERR_OPT_UNKNOWN, Some(&arg0_cstr), None);
                }
            }

            if want_argument {
                if (argv_idx as usize) < arg0.len() {
                    mainerr(ERR_OPT_GARBAGE, Some(&arg0_cstr), None);
                }

                arg_idx += 1;
                if arg_idx >= raw_len && c != b'S' {
                    mainerr(ERR_ARG_MISSING, Some(&arg0_cstr), None);
                }
                argv_idx = -1;

                let cur_cstr_opt = parmp.argv.get(arg_idx).cloned();
                let cur_bytes = cur_cstr_opt.as_ref().map_or(&[][..], |s| s.as_bytes());
                let prev_arg = arg0_cstr.as_c_str();
                let prev_bytes = arg0;

                match c {
                    b'c' | b'S' => {
                        if parmp.commands.len() >= MAX_ARG_CMDS {
                            mainerr(ERR_EXTRA_CMD, None, None);
                        }
                        if c == b'S' {
                            let a_bytes: &[u8] = if arg_idx >= raw_len {
                                b"Session.vim"
                            } else if cur_bytes.first() == Some(&b'-') {
                                arg_idx -= 1;
                                b"Session.vim"
                            } else {
                                cur_bytes
                            };

                            let mut s = Vec::with_capacity(3 + a_bytes.len());
                            s.extend_from_slice(b"so ");
                            s.extend_from_slice(a_bytes);
                            parmp
                                .commands
                                .push(std::ffi::CString::new(s).unwrap_or_default());
                        } else {
                            parmp
                                .commands
                                .push(crate::memory::bytes_to_cstring(cur_bytes));
                        }
                    }
                    b'-' => {
                        if prev_bytes == b"--cmd" {
                            if parmp.pre_commands.len() >= MAX_ARG_CMDS {
                                mainerr(ERR_EXTRA_CMD, None, None);
                            }
                            parmp
                                .pre_commands
                                .push(crate::memory::bytes_to_cstring(cur_bytes));
                        } else if prev_bytes == b"--listen" {
                            if let Some(cur_slot) = parmp.argv.get_mut(arg_idx) {
                                cstring_to_slash_if_needed(cur_slot);
                                parmp.listen_addr = Some(cur_slot.clone());
                            }
                        } else if prev_bytes == b"--server"
                            && let Some(cur_slot) = parmp.argv.get_mut(arg_idx)
                        {
                            cstring_to_slash_if_needed(cur_slot);
                            parmp.server_addr = Some(cur_slot.clone());
                        }
                    }
                    b'q' => {
                        parmp.use_ef = Some(crate::memory::bytes_to_cstring(cur_bytes));
                    }
                    b'i' => {
                        set_option_value_give_err(
                            K_OPT_SHADAFILE,
                            cstr_as_obj(cur_cstr_opt.as_deref()),
                            0,
                        );
                    }
                    b'l' => {
                        g_st().headless_mode = true;
                        g_st().silent_mode = true;
                        opt_vars().p_verbose = 1;
                        parmp.no_swap_file = true;
                        if parmp.use_vimrc.is_none() {
                            parmp.use_vimrc = Some(c"NONE".to_owned());
                        }
                        if opt_vars().p_shadafile.is_null() || opt_vars().p_shadafile.is_empty() {
                            set_option_value_give_err(
                                K_OPT_SHADAFILE,
                                cstr_as_obj(Some(c"NONE")),
                                0,
                            );
                        }
                        parmp.luaf = Some(crate::memory::bytes_to_cstring(cur_bytes));
                        if arg_idx < raw_len {
                            parmp.lua_arg0 = (arg_idx + 1) as i32;
                            arg_idx = raw_len;
                        }
                    }
                    b's' => {
                        if parmp.scriptin.is_some() {
                            format_to_buf!(
                                &mut g_st().iobuff,
                                gettext(c"Attempt to open script file again: \"%s %s\"\n"),
                                prev_arg,
                                cur_cstr_opt.as_deref().unwrap_or(c""),
                            );
                            fprintf!(log_get_stderr(), c"%s", &g_st().iobuff,);
                            os_exit(2);
                        }
                        parmp.scriptin = Some(crate::memory::bytes_to_cstring(cur_bytes));
                    }
                    b't' => {
                        parmp.tagname = Some(crate::memory::bytes_to_cstring(cur_bytes));
                    }
                    b'u' => {
                        if let Some(cur_slot) = parmp.argv.get_mut(arg_idx) {
                            cstring_to_slash_if_needed(cur_slot);
                            parmp.use_vimrc = Some(cur_slot.clone());
                        }
                    }
                    b'U' => {}
                    b'w' | b'W' => {
                        if c == b'w' && cur_bytes.first().is_some_and(u8::is_ascii_digit) {
                            argv_idx = 0;
                            let n = get_number_arg(cur_bytes, &mut argv_idx, 10);
                            set_option_value_give_err(
                                K_OPT_WINDOW,
                                integer_obj(OptInt::from(n)),
                                0,
                            );
                            argv_idx = -1;
                        } else {
                            if parmp.scriptout.is_some() {
                                format_to_buf!(
                                    &mut g_st().iobuff,
                                    gettext(c"Attempt to open script file again: \"%s %s\"\n"),
                                    prev_arg,
                                    cur_cstr_opt.as_deref().unwrap_or(c""),
                                );
                                fprintf!(log_get_stderr(), c"%s", &g_st().iobuff,);
                                os_exit(2);
                            }
                            parmp.scriptout = Some(crate::memory::bytes_to_cstring(cur_bytes));
                            parmp.scriptout_append = c == b'w';
                        }
                    }
                    _ => {}
                }
            }
        } else {
            argv_idx = -1;

            if parmp.edit_type > EDIT_STDIN {
                mainerr(ERR_TOO_MANY_ARGS, Some(&arg0_cstr), None);
            }
            parmp.edit_type = EDIT_FILE;

            let mut p_vec: Vec<u8> = arg0_cstr.to_bytes_with_nul().to_vec();

            #[cfg(windows)]
            {
                to_slash_if_needed(&mut p_vec);
                if p_vec[0] == b'~' && p_vec.get(1) == Some(&b'/') {
                    let homedir = os_homedir();
                    let h_bytes = homedir.as_deref().map_or(&[][..], CStr::to_bytes);
                    let p_bytes = &p_vec[1..p_vec.len() - 1];
                    let mut expanded = Vec::with_capacity(h_bytes.len() + p_bytes.len() + 1);
                    expanded.extend_from_slice(h_bytes);
                    expanded.extend_from_slice(p_bytes);
                    expanded.push(0);
                    p_vec = expanded;
                }
            }

            if parmp.diff_mode != 0
                && os_isdir(crate::memory::bytes_as_cstr(&p_vec))
                && gargcount() > 0
            {
                let e0 = garg_entry(0);
                let name0 = alist_name(&e0).unwrap_or(c"");
                if !os_isdir(name0) {
                    let tail = path_tail(name0);
                    let joined = concat_fnames(
                        cstr_as_string(Some(crate::memory::bytes_as_cstr(&p_vec))),
                        cstr_as_string(Some(tail)),
                        true,
                    );
                    p_vec = joined.as_bytes_with_nul().to_vec();
                }
            }

            if CASE_INSENSITIVE_FILENAME {
                path_fix_case(&mut p_vec);
            }

            let alist_fnum_flag = if edit_stdin(parmp) { 1 } else { 2 };
            alist_add(
                &global_alist_rc(),
                crate::memory::cstring_until_nul(&p_vec),
                alist_fnum_flag,
            );
        }

        if argv_idx <= 0
            || parmp
                .argv
                .get(arg_idx)
                .is_none_or(|p| p.as_bytes().len() <= argv_idx as usize)
        {
            arg_idx += 1;
            argv_idx = 1;
        }
    }

    if g_st().embedded_mode && (g_st().silent_mode || parmp.luaf.is_some()) {
        mainerr(gettext(c"--embed conflicts with -es/-Es/-l"), None, None);
    }

    if let Some(cmd0) = parmp.commands.first() {
        let cmd0_bytes = cmd0.as_bytes();
        let swcmd_len = cmd0_bytes.len() + 2;
        let mut swcmd = Vec::with_capacity(swcmd_len + 1);
        swcmd.push(b':');
        swcmd.extend_from_slice(cmd0_bytes);
        swcmd.push(b'\r');
        swcmd.push(0);
        set_vim_var_string(
            VV_SWAPCOMMAND,
            Some(crate::memory::bytes_as_cstr(&swcmd)),
            swcmd_len as isize,
        );
    }

    time_msg_if_enabled(c"parsing arguments");
}

fn set_argf_var() {
    let list = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    let count = gargcount();
    for i in 0..count {
        let entry = garg_entry(i as usize);
        if let Some(fname) = alist_name(&entry) {
            let _ = vim_full_name(Some(fname), g_st().name_buff_bytes_mut(), false);
            tv_list_append_string(&mut *list, Some(g_st().name_buff_cstr()), -1);
        }
    }
    tv_list_set_lock(Some(&mut *list), VAR_FIXED);
    set_vim_var_list(VV_ARGF, Some(list));
}

fn init_params(paramp: &mut Mparm, argv: Vec<std::ffi::CString>) {
    *paramp = Mparm::new();
    paramp.argc = argv.len() as i32;
    paramp.argv = argv;
    paramp.use_debug_break_level = -1;
    paramp.window_count = -1;
    paramp.listen_addr = None;
    paramp.server_addr = None;
    paramp.remote = 0;
    paramp.luaf = None;
    paramp.lua_arg0 = -1;
}

fn init_startuptime(paramp: &mut Mparm) {
    let raw_args = &paramp.argv;
    let args = raw_args
        .get(1..raw_args.len().saturating_sub(1))
        .unwrap_or(&[]);
    let is_embed = args
        .iter()
        .any(|a| a.as_bytes().eq_ignore_ascii_case(b"--embed"));
    for (idx, arg) in args.iter().enumerate() {
        if arg.as_bytes().eq_ignore_ascii_case(b"--startuptime") {
            time_init(
                raw_args[idx + 2].as_c_str(),
                if is_embed {
                    c"Embedded"
                } else {
                    c"Primary (or UI client)"
                },
            );
            time_start(c"--- NVIM STARTING ---");
            break;
        }
    }
}

fn check_and_set_isatty(_paramp: &mut Mparm) {
    g_st().stdin_isatty = os_isatty(STDIN_FILENO);
    g_st().stdout_isatty = os_isatty(STDOUT_FILENO);
    g_st().stderr_isatty = os_isatty(STDERR_FILENO);
    time_msg_if_enabled(c"window checked");
}

fn init_path(exename: &CStr) {
    let mut exepath = [0u8; MAXPATHL];
    let mut exepathlen: usize = MAXPATHL;
    if os_exepath(&mut exepath, &mut exepathlen) != 0 {
        path_guess_exepath(exename, &mut exepath);
    }
    set_vim_var_string(
        VV_PROGPATH,
        Some(crate::memory::bytes_as_cstr(&exepath)),
        -1,
    );
    set_vim_var_string(VV_PROGNAME, Some(path_tail(exename)), -1);

    #[cfg(windows)]
    {
        os_setenv_append_path(crate::memory::bytes_as_cstr(&exepath));
    }
}

fn get_fname(_parmp: &mut Mparm) -> Option<std::ffi::CString> {
    let entry = garg_entry(0);
    alist_name(&entry).map(std::ffi::CStr::to_owned)
}

fn set_window_layout(paramp: &mut Mparm) {
    if paramp.diff_mode != 0 && paramp.window_layout == 0 {
        paramp.window_layout = if diffopt_horizontal() {
            WIN_HOR
        } else {
            WIN_VER
        };
    }
}

fn handle_quickfix(paramp: &mut Mparm) {
    if paramp.edit_type == EDIT_QF {
        if let Some(ref use_ef) = paramp.use_ef {
            set_option_direct(K_OPT_ERRORFILE, cstr_as_obj(Some(use_ef)), 0, SID_CARG);
        }
        format_to_buf!(&mut g_st().iobuff, c"cfile %s", &opt_vars().p_ef);
        if qf_init(
            None,
            Some(opt_vars().p_ef.as_cstr()),
            opt_vars().p_efm.as_cstr(),
            1,
            Some(crate::memory::bytes_as_cstr(g_st().iobuff_bytes())),
            Some(opt_vars().p_menc.as_cstr()),
        ) < 0
        {
            msg_putchar(b'\n' as i32);
            os_exit(3);
        }
        time_msg_if_enabled(c"reading errorfile");
    }
}

fn handle_tag(tagname: Option<&CStr>) {
    if let Some(tagname) = tagname {
        g_st().swap_exists_did_quit = false;
        format_to_buf!(&mut g_st().iobuff, c"ta %s", tagname);
        do_cmdline_cmd(crate::memory::bytes_as_cstr(g_st().iobuff_bytes()));
        time_msg_if_enabled(c"jumping to tag");

        if g_st().swap_exists_did_quit {
            ui_call_error_exit(1);
            getout(1);
        }
    }
}

fn read_stdin() {
    g_st().swap_exists_action = SEA_DIALOG;
    g_st().no_wait_return = 1;
    let save_msg_didany = g_st().msg_didany;

    if curbuf_mut().b_ffname.is_some() {
        let Some(stdin_buf_ref) = buflist_new(None, None, 0, BLN_LISTED) else {
            semsg!(c"Failed to create buffer for stdin");
            return;
        };
        let stdin_buf = crate::globals::BufPtr::from(&*stdin_buf_ref);

        let initial_buf_handle: ObjHandle = curbuf_mut().b_fnum;
        set_curbuf(stdin_buf.borrow_mut(), 0, false);
        readfile(None, None, 0, 0, MAXLNUM, None, READ_NEW + READ_STDIN, true);

        let stdin_buf_handle: ObjHandle = stdin_buf.b_fnum;
        let stdin_buf_empty = buf_is_empty(curbuf_mut());

        let mut buf = [0u8; 100];
        format_to_buf!(&mut buf, c"silent! buffer %d", initial_buf_handle);
        do_cmdline_cmd(crate::memory::bytes_as_cstr(&buf));

        if stdin_buf_empty {
            format_to_buf!(&mut buf, c"silent! bwipeout! %d", stdin_buf_handle);
            do_cmdline_cmd(crate::memory::bytes_as_cstr(&buf));
        }
    } else {
        set_buflisted(1);
        open_buffer(true, None, 0);
        if buf_is_empty(curbuf_mut()) && !curbuf_mut().b_next.is_null() {
            do_cmdline_cmd(c"silent! bnext");
            do_cmdline_cmd(c"silent! bwipeout 1");
        }
    }

    g_st().no_wait_return = 0;
    g_st().msg_didany = save_msg_didany;
    time_msg_if_enabled(c"reading stdin");
    check_swap_exists_action();
}

fn create_windows(parmp: &mut Mparm) {
    if parmp.window_count == -1 {
        parmp.window_count = 1;
    }
    if parmp.window_count == 0 {
        parmp.window_count = gargcount();
    }
    if parmp.window_count > 1 {
        if parmp.window_layout == 0 {
            parmp.window_layout = WIN_HOR;
        }
        if parmp.window_layout == WIN_TABS {
            parmp.window_count = make_tabpages(parmp.window_count);
            time_msg_if_enabled(c"making tab pages");
        } else if firstwin_mut().next_opt().is_none_or(|nw| nw.w_floating) {
            parmp.window_count = make_windows(parmp.window_count, parmp.window_layout == WIN_VER);
            time_msg_if_enabled(c"making windows");
        } else {
            parmp.window_count = win_count();
        }
    } else {
        parmp.window_count = 1;
    }

    if g_st().recoverymode {
        g_st().msg_scroll = 1;
        ml_recover(true);
        if curbuf_mut().b_ml.ml_mfp.is_none() {
            getout(1);
        }
        do_modelines(0);
    } else {
        let mut done: i32 = 0;
        g_st().autocmd_no_enter += 1;
        g_st().autocmd_no_leave += 1;
        let startup_curwin = curwin_ptr();
        let mut dorewind = true;
        while done < 1000 {
            done += 1;
            if dorewind {
                if parmp.window_layout == WIN_TABS {
                    goto_tabpage(1);
                } else {
                    set_curwin(g_st().firstwin.as_ptr());
                }
            } else if parmp.window_layout == WIN_TABS {
                if curtab_mut().tp_next.get().is_null() {
                    break;
                }
                goto_tabpage(0);
            } else {
                if curwin_mut().w_next.is_null() {
                    break;
                }
                set_curwin(curwin_mut().w_next.as_ptr());
            }
            dorewind = false;
            g_st().curbuf = curwin_mut().w_buffer;
            if curbuf_mut().b_ml.ml_mfp.is_none() {
                if opt_vars().p_fdls >= 0 {
                    curwin_mut().w_onebuf_opt.wo_fdl = opt_vars().p_fdls;
                }
                g_st().swap_exists_action = SEA_DIALOG;
                set_buflisted(1);

                open_buffer(false, None, 0);

                if g_st().swap_exists_action == SEA_QUIT {
                    if g_st().got_int || only_one_window() {
                        g_st().did_emsg = 0;
                        ui_call_error_exit(1);
                        getout(1);
                    }
                    setfname(curbuf_mut(), None, None, false);
                    curwin_mut().w_arg_idx = -1;
                    g_st().swap_exists_action = SEA_NONE;
                } else {
                    handle_swap_exists(None);
                }
                dorewind = true;
            }
            os_breakcheck();
            if g_st().got_int {
                vgetc();
                break;
            }
        }
        if parmp.window_layout == WIN_TABS {
            goto_tabpage(1);
        } else if parmp.window_count > 1 || !win_valid(startup_curwin.opt()) {
            set_curwin(g_st().firstwin.as_ptr());
        } else {
            set_curwin(startup_curwin);
        }
        g_st().curbuf = curwin_mut().w_buffer;
        g_st().autocmd_no_enter -= 1;
        g_st().autocmd_no_leave -= 1;
    }
}

fn edit_buffers(parmp: &mut Mparm) {
    let mut advance = true;
    let mut p_shm_save: Option<std::ffi::CString> = None;

    g_st().autocmd_no_enter += 1;
    g_st().autocmd_no_leave += 1;

    if curwin_mut().w_arg_idx == -1 {
        win_close(curwin_mut(), true, false);
        advance = false;
    }

    let mut arg_idx: i32 = 1;
    for i in 1..parmp.window_count {
        if curwin_mut().w_arg_idx == -1 {
            arg_idx += 1;
            win_close(curwin_mut(), true, false);
            advance = false;
            continue;
        }

        if advance {
            if parmp.window_layout == WIN_TABS {
                if curtab_mut().tp_next.get().is_null() {
                    break;
                }
                goto_tabpage(0);
                if i == 1 {
                    let mut buf = [0u8; 100];
                    p_shm_save = Some(opt_vars().p_shm.to_owned());
                    format_to_buf!(&mut buf, c"F%s", &opt_vars().p_shm);
                    set_option_value_give_err(
                        K_OPT_SHORTMESS,
                        cstr_as_obj(Some(crate::memory::bytes_as_cstr(&buf))),
                        0,
                    );
                }
            } else {
                if curwin_mut().w_next.is_null() {
                    break;
                }
                win_enter(curwin_mut().w_next.borrow_mut(), false);
            }
        }
        advance = true;

        if ptr::eq(g_st().curbuf.as_ptr(), firstwin_mut().w_buffer.as_ptr())
            || curbuf_mut().b_ffname.is_none()
        {
            curwin_mut().w_arg_idx = arg_idx;
            g_st().swap_exists_did_quit = false;
            let entry_opt = (arg_idx < gargcount()).then(|| garg_entry(arg_idx as usize));
            let fname = entry_opt.as_ref().and_then(|e| alist_name(e));
            do_ecmd(
                0,
                fname,
                None,
                None,
                ECMD_LASTL,
                ECMD_HIDE,
                Some(curwin_mut()),
            );
            if g_st().swap_exists_did_quit {
                if g_st().got_int || only_one_window() {
                    g_st().did_emsg = 0;
                    ui_call_error_exit(1);
                    getout(1);
                }
                win_close(curwin_mut(), true, false);
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
        set_option_value_give_err(K_OPT_SHORTMESS, cstr_as_obj(Some(p_shm_save)), 0);
    }

    if parmp.window_layout == WIN_TABS {
        goto_tabpage(1);
    }
    g_st().autocmd_no_enter -= 1;

    let mut win = if parmp.window_count > 1 {
        g_st().firstwin
    } else {
        curwin_ptr()
    };
    while win.w_onebuf_opt.wo_pvw {
        win = win.w_next;
        if win.is_null() {
            win = g_st().firstwin;
            break;
        }
    }
    win_enter(win.borrow_mut(), false);

    g_st().autocmd_no_leave -= 1;
    time_msg_if_enabled(c"editing files in windows");
    if parmp.window_count > 1 && parmp.window_layout != WIN_TABS {
        win_equal(Some(curwin_mut()), false, b'b' as i32);
    }
}

fn exe_pre_commands(parmp: &mut Mparm) {
    if parmp.pre_commands.is_empty() {
        return;
    }

    curwin_mut().w_cursor.lnum = 0;
    estack_push(ETYPE_ARGS, Some(gettext(c"pre-vimrc command line")), 0);
    g_st().current_sctx.sc_sid = SID_CMDARG;
    for cmd in &parmp.pre_commands {
        do_cmdline_cmd(cmd);
    }
    estack_pop();
    g_st().current_sctx.sc_sid = 0;
    time_msg_if_enabled(c"--cmd commands");
}

fn exe_commands(parmp: &mut Mparm) {
    g_st().msg_scroll = 1;
    if parmp.tagname.is_none() && curwin_mut().w_cursor.lnum <= 1 {
        curwin_mut().w_cursor.lnum = 0;
    }
    estack_push(ETYPE_ARGS, Some(c"command line"), 0);
    g_st().current_sctx.sc_sid = SID_CARG;
    g_st().current_sctx.sc_seq = 0;
    let cmds = std::mem::take(&mut parmp.commands);
    for cmd in &cmds {
        do_cmdline_cmd(cmd);
    }
    estack_pop();
    g_st().current_sctx.sc_sid = 0;
    if curwin_mut().w_cursor.lnum == 0 {
        curwin_mut().w_cursor.lnum = 1;
    }

    g_st().msg_scroll = 0;

    if parmp.edit_type == EDIT_QF {
        qf_jump(None, 0, 0, false);
    }
    time_msg_if_enabled(c"executing command arguments");
}

fn do_system_initialization() {
    if let Some(config_dirs) = stdpaths_get_xdg_var(XDGVarType::XDGConfigDirs) {
        let config_bytes = config_dirs.as_bytes();
        let mut iter = None;
        let appname = get_appname(false);
        let appname_bytes = appname.to_bytes();
        let sysinit_suffix: [u8; 13] = [
            PATHSEP, b's', b'y', b's', b'i', b'n', b'i', b't', b'.', b'v', b'i', b'm', 0,
        ];
        loop {
            let mut dir: &[u8] = &[];
            iter = vim_env_iter(b':', config_bytes, iter, &mut dir);
            if dir.is_empty() {
                break;
            }
            let path_len = dir.len() + 1 + appname_bytes.len() + sysinit_suffix.len();
            let mut vimrc = Vec::with_capacity(path_len);
            vimrc.extend_from_slice(dir);
            if vimrc.last() != Some(&PATHSEP) {
                vimrc.push(PATHSEP);
            }
            vimrc.extend_from_slice(appname_bytes);
            vimrc.extend_from_slice(&sysinit_suffix);
            if do_source(crate::memory::bytes_as_cstr(&vimrc), false, DOSO_NONE, None) {
                return;
            }
            if iter.is_none() {
                break;
            }
        }
    }

    do_source(c"$VIM/sysinit.vim", false, DOSO_NONE, None);
}

fn do_user_initialization() -> bool {
    let mut do_exrc = opt_vars().p_exrc;
    if execute_env(c"VIMINIT") {
        do_exrc = opt_vars().p_exrc;
        return do_exrc;
    }

    let init_lua_path = stdpaths_user_conf_subpath(c"init.lua");
    let user_vimrc = stdpaths_user_conf_subpath(c"init.vim");

    if os_path_exists(init_lua_path.as_c_str())
        && do_source(init_lua_path.as_c_str(), true, DOSO_VIMRC, None)
    {
        if os_path_exists(user_vimrc.as_c_str()) {
            semsg!(E_CONFLICTING_CONFIGS, &init_lua_path, &user_vimrc);
        }
        do_exrc = opt_vars().p_exrc;
        return do_exrc;
    }
    drop(init_lua_path);

    if do_source(user_vimrc.as_c_str(), true, DOSO_VIMRC, None) {
        do_exrc = opt_vars().p_exrc;
        if do_exrc {
            do_exrc = !path_equal(c".nvimrc", user_vimrc.as_c_str(), K_PATH_CMP_EXPAND);
        }
        return do_exrc;
    }
    drop(user_vimrc);

    if let Some(config_dirs) = stdpaths_get_xdg_var(XDGVarType::XDGConfigDirs) {
        let appname = get_appname(false);
        let appname_bytes = appname.to_bytes();
        let init_lua_suffix: [u8; 10] =
            [PATHSEP, b'i', b'n', b'i', b't', b'.', b'l', b'u', b'a', 0];
        let init_vim_suffix: [u8; 10] =
            [PATHSEP, b'i', b'n', b'i', b't', b'.', b'v', b'i', b'm', 0];

        let config_bytes = config_dirs.as_bytes();
        let mut iter = None;
        loop {
            let mut dir: &[u8] = &[];
            iter = vim_env_iter(b':', config_bytes, iter, &mut dir);
            if dir.is_empty() {
                break;
            }

            let init_lua_len = dir.len() + 1 + appname_bytes.len() + init_lua_suffix.len();
            let mut init_lua = Vec::with_capacity(init_lua_len);
            init_lua.extend_from_slice(dir);
            init_lua.push(PATHSEP);
            init_lua.extend_from_slice(appname_bytes);
            init_lua.extend_from_slice(&init_lua_suffix);

            let init_vim_len = dir.len() + 1 + appname_bytes.len() + init_vim_suffix.len();
            let mut init_vim = Vec::with_capacity(init_vim_len);
            init_vim.extend_from_slice(dir);
            init_vim.push(PATHSEP);
            init_vim.extend_from_slice(appname_bytes);
            init_vim.extend_from_slice(&init_vim_suffix);

            if os_path_exists(crate::memory::bytes_as_cstr(&init_lua))
                && do_source(
                    crate::memory::bytes_as_cstr(&init_lua),
                    true,
                    DOSO_VIMRC,
                    None,
                )
            {
                if os_path_exists(crate::memory::bytes_as_cstr(&init_vim)) {
                    semsg!(E_CONFLICTING_CONFIGS, &init_lua, &init_vim);
                }
                do_exrc = opt_vars().p_exrc;
                return do_exrc;
            }

            if do_source(
                crate::memory::bytes_as_cstr(&init_vim),
                true,
                DOSO_VIMRC,
                None,
            ) {
                do_exrc = opt_vars().p_exrc;
                if do_exrc {
                    do_exrc = !path_equal(
                        c".nvimrc",
                        crate::memory::bytes_as_cstr(&init_vim),
                        K_PATH_CMP_EXPAND,
                    );
                }
                return do_exrc;
            }

            if iter.is_none() {
                break;
            }
        }
    }

    if execute_env(c"EXINIT") {
        do_exrc = opt_vars().p_exrc;
        return do_exrc;
    }
    do_exrc
}

fn do_exrc_initialization() {
    let lstate = get_global_lstate();
    assert!(!lstate.is_null());

    lua_getfield(lstate, LUA_GLOBALSINDEX, c"require");
    lua_pushstring(lstate, c"vim._core.exrc");
    if nlua_pcall(lstate, 1, 0) != 0 {
        let err_cstr = lua_tolstring_cstr(lstate, -1);
        fprintf!(log_get_stderr(), c"%s\n", err_cstr);
    }
}

fn source_startup_scripts(parmp: &Mparm) {
    if let Some(ref use_vimrc) = parmp.use_vimrc {
        if use_vimrc.as_c_str() == c"NONE" || use_vimrc.as_c_str() == c"NORC" {
            // Do nothing.
        } else if !do_source(use_vimrc.as_c_str(), false, DOSO_NONE, None) {
            semsg!(gettext(E_CANNOT_READ_FROM_STR_2), use_vimrc,);
        }
    } else if !g_st().silent_mode {
        do_system_initialization();
        if do_user_initialization() {
            do_exrc_initialization();
        }
    }
    time_msg_if_enabled(c"sourcing vimrc file(s)");
}

fn execute_env(env: &std::ffi::CStr) -> bool {
    let Some(initstr) = os_getenv_cstr(env) else {
        return false;
    };

    estack_push(ETYPE_ENV, Some(env), 0);
    let save_current_sctx = g_st().current_sctx;
    g_st().current_sctx.sc_sid = SID_ENV;
    g_st().current_sctx.sc_seq = 0;
    g_st().current_sctx.sc_lnum = 0;
    do_cmdline_cmd(&initstr);

    estack_pop();
    g_st().current_sctx = save_current_sctx;

    true
}

fn mainerr(msg1: &CStr, msg2: Option<&CStr>, msg3: Option<&CStr>) -> ! {
    print_mainerr(msg1, msg2, msg3);
    os_exit(1);
}

fn print_mainerr(msg1: &CStr, msg2: Option<&CStr>, msg3: Option<&CStr>) {
    let argv0 = main_init_st(|st| st.argv0.borrow().clone());
    let prgname = path_tail(argv0.as_deref().unwrap_or(c""));
    signal_stop();

    let stderr = log_get_stderr();
    fprintf!(stderr, c"%s: %s", prgname, gettext(msg1));
    if let Some(msg2) = msg2 {
        fprintf!(stderr, c": \"%s\"", msg2);
    }
    if let Some(msg3) = msg3 {
        fprintf!(stderr, c": \"%s\"", msg3);
    }
    fprintf!(stderr, gettext(c"\nMore info with \""));
    fprintf!(stderr, c"%s -h\"\n", prgname);
}

fn version() {
    nlua_init(&[], -1);
    g_st().info_message = true;
    list_version();
    msg_putchar(b'\n' as i32);
    g_st().msg_didout = false;
}

fn usage() {
    signal_stop();

    printf!(gettext(c"Usage:\n"));
    printf!(gettext(c"  nvim [options] [file ...]\n"));
    printf!(gettext(c"\nOptions:\n"));
    printf!(gettext(
        c"  --cmd <cmd>           Execute <cmd> before any config\n",
    ));
    printf!(gettext(
        c"  +<cmd>, -c <cmd>      Execute <cmd> after config and first file\n",
    ));
    printf!(gettext(
        c"  -l <script> [args...] Execute Lua <script> (with optional args)\n",
    ));
    printf!(gettext(
        c"  -S <session>          Source <session> after loading the first file\n",
    ));
    printf!(gettext(
        c"  -s <scriptin>         Read Normal mode commands from <scriptin>\n",
    ));
    printf!(gettext(c"  -u <config>           Use this config file\n",));
    printf!(c"\n");
    printf!(gettext(c"  -d                    Diff mode\n"));
    printf!(gettext(c"  -es, -Es              Silent (batch) mode\n",));
    printf!(gettext(
        c"  -h, --help            Print this help message\n",
    ));
    printf!(gettext(c"  -i <shada>            Use this shada file\n",));
    printf!(gettext(
        c"  -n                    No swap file, use memory only\n",
    ));
    printf!(gettext(
        c"  -o[N]                 Open N windows (default: one per file)\n",
    ));
    printf!(gettext(
        c"  -O[N]                 Open N vertical windows (default: one per file)\n",
    ));
    printf!(gettext(
        c"  -p[N]                 Open N tab pages (default: one per file)\n",
    ));
    printf!(gettext(c"  -R                    Read-only (view) mode\n",));
    printf!(gettext(
        c"  -v, --version         Print version information\n",
    ));
    printf!(gettext(c"  -V[N][file]           Verbose [level][file]\n",));
    printf!(c"\n");
    printf!(gettext(
        c"  --                    Only file names after this\n",
    ));
    printf!(gettext(
        c"  --api-info            Write msgpack-encoded API metadata to stdout\n",
    ));
    printf!(gettext(
        c"  --clean               \"Factory defaults\" (skip user config and plugins, shada)\n",
    ));
    printf!(gettext(
        c"  --embed               Use stdin/stdout as a msgpack-rpc channel\n",
    ));
    printf!(gettext(
        c"  --headless            Don't start a user interface\n",
    ));
    printf!(gettext(
        c"  --listen <address>    Serve RPC API from this address\n",
    ));
    printf!(gettext(
        c"  --remote[-subcommand] Execute commands remotely on a server\n",
    ));
    printf!(gettext(
        c"  --server <address>    Connect to this Nvim server\n",
    ));
    printf!(gettext(
        c"  --startuptime <file>  Write startup timing messages to <file>\n",
    ));
    printf!(gettext(
        c"\nSee \":help startup-options\" for all options.\n",
    ));
}

fn check_swap_exists_action() {
    if g_st().swap_exists_action == SEA_QUIT {
        ui_call_error_exit(1);
        getout(1);
    }
    handle_swap_exists(None);
}

#[cfg(test)]
pub(crate) fn ensure_test_editor_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    thread_local! {
        static THREAD_VARS_INIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    INIT.call_once(|| {
        g_st().headless_mode = true;
        g_st().silent_mode = true;
        event_init();
        estack_init();
        cmdline_init();
        eval_init();
        THREAD_VARS_INIT.set(true);
        init_path(c"nvim");
        init_normal_cmds();
        highlight_init();
        mc_init();
        init_locale();
        set_init_tablocal();
        win_alloc_first();
        global_alist_rc().id.set(0);
        init_homedir();
        set_init_1(true);
        set_lang_var();
        qf_init_stack();
        set_init_2(true);
        crate::highlight_group::init_highlight(true, false);
    });
    if !THREAD_VARS_INIT.get() {
        estack_init();
        eval_init();
        init_path(c"nvim");
        THREAD_VARS_INIT.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_number_arg_and_init_params_work() {
        let mut idx: i32 = 1;
        let val = get_number_arg(b"o25x", &mut idx, 0);
        assert_eq!(val, 25);
        assert_eq!(idx, 3);

        let mut params = Mparm::new();
        init_params(
            &mut params,
            vec![c"a".to_owned(), c"b".to_owned(), c"c".to_owned()],
        );
        assert_eq!(params.argc, 3);
        assert_eq!(params.window_count, -1);
        assert_eq!(params.use_debug_break_level, -1);
        assert_eq!(params.lua_arg0, -1);
    }

    #[test]
    fn command_line_scan_parses_flags_commands_and_lua_args() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        ensure_test_editor_init();
        let c_args: Vec<std::ffi::CString> = [
            "nvim",
            "-dn",
            "-O3",
            "+",
            "+set nu",
            "-ccall Foo()",
            "--cmd",
            "let g:pre = 1",
            "-S",
            "-R",
            "-l",
            "script.lua",
            "--unparsed-flag",
            "arg2",
        ]
        .into_iter()
        .map(|s| std::ffi::CString::new(s).unwrap())
        .collect();
        let mut params = Mparm::new();
        init_params(&mut params, c_args);
        command_line_scan(&mut params);

        assert_eq!(params.diff_mode, 1);
        assert!(params.no_swap_file);
        assert_eq!(params.window_layout, WIN_VER);
        assert_eq!(params.window_count, 3);
        assert_eq!(
            params.commands,
            vec![
                c"$".to_owned(),
                c"set nu".to_owned(),
                c"call Foo()".to_owned(),
                c"so Session.vim".to_owned(),
            ]
        );
        assert_eq!(params.pre_commands, vec![c"let g:pre = 1".to_owned()]);
        assert_eq!(params.luaf.as_deref(), Some(c"script.lua"));
        assert_eq!(params.lua_arg0, 12);
    }
}
