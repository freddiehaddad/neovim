#![forbid(unsafe_code)]

//! Nvim's own UI client, which attaches to a child or remote Nvim server (`src/nvim/ui_client.c`).

use crate::api::private::dispatch::key_dict_highlight_get_field as highlight_get_field_cb;
use crate::api::private::helpers::{
    api_dict_to_keydict, api_error, api_free_array, api_metadata, boolean_obj, copy_array,
    cstr_as_obj, cstr_as_string, integer_obj,
};
use crate::api::private::validate::{Array, Dict, Error, ErrorType, KeyValuePair, Object};
use crate::api::ui::{ARRAY_INIT, K_LINE_FLAG_WRAP};
use crate::channel::{
    CallbackReader, K_CHANNEL_STDIN_PIPE_CONST, channel_connect, channel_job_start,
};
use crate::eval::typval::Callback;
use crate::event::r#loop::{loop_poll_events, loop_queues};
use crate::event::multiqueue::{
    Event, multiqueue_empty, multiqueue_process_events, multiqueue_put_event,
};
use crate::event::socket::socket_address_tcp_host_end;
use crate::fileio::close;
#[cfg(not(windows))]
use crate::fileio::dup;
use crate::globals::*;
use crate::highlight::{HLATTRS_INIT, HlAttrs, KeyDictHighlight, dict2hlattrs};
use crate::log::{LOGLVL_ERR, LOGLVL_INF, logmsg};
use crate::main_init::os_exit;
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::msgpack_rpc::packer::Integer;
use crate::msgpack_rpc::unpacker::GridLineEvent;
use crate::os::env::{os_env_exists, os_get_pid};
#[cfg(windows)]
use crate::os::os_win_console::os_open_conin_fd;
use crate::profile::{time_finish, time_msg};
use crate::tui::tui::{
    TUIData, tui_add_url, tui_bell, tui_busy_start, tui_busy_stop, tui_chdir,
    tui_default_colors_set, tui_flush, tui_free_all_mem, tui_grid_clear, tui_grid_cursor_goto,
    tui_grid_resize, tui_grid_scroll, tui_hl_attr_define, tui_is_stopped, tui_mode_change,
    tui_mode_info_set, tui_mouse_off, tui_mouse_on, tui_option_set, tui_raw_line, tui_screenshot,
    tui_set_icon, tui_set_title, tui_start, tui_stop, tui_suspend, tui_ui_send, tui_update_menu,
    tui_visual_bell,
};
use std::ffi::{CStr, CString};

pub(crate) const UI_CLIENT_STDIN_FD: i32 = 3;
use crate::highlight::KEYSET_OPTIDX_HIGHLIGHT_URL;

struct UiClientState {
    tui: std::cell::RefCell<Option<Box<TUIData>>>,
    tui_width: std::cell::Cell<i32>,
    tui_height: std::cell::Cell<i32>,
    tui_rgb: std::cell::Cell<bool>,
    detached_addr: std::cell::RefCell<Option<std::ffi::CString>>,
    restart_args: crate::api::private::validate::CloneCell<Array>,
    restart_pending: std::cell::Cell<bool>,
    restart_args_after_crash_exit: crate::api::private::validate::CloneCell<Array>,
    grid_line_buf_char: std::cell::RefCell<Vec<crate::grid::Schar>>,
    grid_line_buf_attr: std::cell::RefCell<Vec<crate::grid::Sattr>>,
}

impl UiClientState {
    const fn new() -> Self {
        Self {
            tui: std::cell::RefCell::new(None),
            tui_width: std::cell::Cell::new(0),
            tui_height: std::cell::Cell::new(0),
            tui_rgb: std::cell::Cell::new(false),
            detached_addr: std::cell::RefCell::new(None),
            restart_args: crate::api::private::validate::CloneCell::new(ARRAY_INIT),
            restart_pending: std::cell::Cell::new(false),
            restart_args_after_crash_exit: crate::api::private::validate::CloneCell::new(
                ARRAY_INIT,
            ),
            grid_line_buf_char: std::cell::RefCell::new(Vec::new()),
            grid_line_buf_attr: std::cell::RefCell::new(Vec::new()),
        }
    }
}

thread_local! {
    static UIC_STATE: UiClientState = const { UiClientState::new() };
    static UIC_TUI_TERM: std::cell::RefCell<Option<std::ffi::CString>> =
        const { std::cell::RefCell::new(None) };
}

#[inline]
pub(crate) fn with_tui<R>(f: impl FnOnce(&TUIData) -> R) -> R {
    UIC_STATE.with(|s| f(s.tui.borrow().as_deref().unwrap()))
}

#[inline]
pub(crate) fn with_tui_mut<R>(f: impl FnOnce(&mut TUIData) -> R) -> R {
    UIC_STATE.with(|s| f(s.tui.borrow_mut().as_deref_mut().unwrap()))
}

pub fn ui_client_start_server(exepath: &CStr, argv: &[CString]) -> u64 {
    let mut args = Vec::with_capacity(1 + argv.len());
    args.push(argv.first().cloned().unwrap_or_default());
    args.push(c"--embed".to_owned());
    for arg in argv.iter().skip(1) {
        args.push(arg.clone());
    }

    let on_stdout = CallbackReader::none();
    let mut on_err = CallbackReader::none();
    on_err.fwd_err = true;
    let on_exit: Callback = Callback::new();

    let mut exit_status: i64 = 0;
    let channel = channel_job_start(
        args,
        Some(exepath),
        on_stdout,
        on_err,
        on_exit,
        false,
        true,
        true,
        true,
        K_CHANNEL_STDIN_PIPE_CONST,
        None,
        0,
        0,
        None,
        &mut exit_status,
    );
    let Some(channel) = channel else {
        return 0;
    };
    channel.trust.set(false);

    if screen_state().ui_client_forward_stdin.get() {
        close(0);
        #[cfg(windows)]
        {
            os_open_conin_fd();
        }
        #[cfg(not(windows))]
        {
            let _ = dup(if exc_state().stderr_isatty.get() {
                2
            } else {
                1
            });
        }
    }

    channel.id
}

/// Attaches this client to the UI channel, and sets its client info (`ui_client_attach`).
pub fn ui_client_attach(width: i32, height: i32, term: Option<&std::ffi::CStr>, rgb: bool) {
    let mut opts_items: [KeyValuePair; 9] = [const { KeyValuePair::new() }; 9];
    let mut opts = Dict::from_buf(&mut opts_items);
    let mut put_opt = |k: &std::ffi::CStr, v: Object| {
        *opts.next_slot_mut() = KeyValuePair {
            key: cstr_as_string(Some(k)),
            value: v,
        };
        opts.size += 1;
    };
    put_opt(c"rgb", boolean_obj(rgb));
    put_opt(c"ext_linegrid", boolean_obj(true));
    put_opt(c"ext_termcolors", boolean_obj(true));
    if let Some(term) = term {
        put_opt(c"term_name", cstr_as_obj(Some(term)));
    }
    put_opt(
        c"term_colors",
        integer_obj(Integer::from(screen_state().t_colors.get())),
    );
    put_opt(c"stdin_tty", boolean_obj(exc_state().stdin_isatty.get()));
    put_opt(c"stdout_tty", boolean_obj(exc_state().stdout_isatty.get()));
    if screen_state().ui_client_forward_stdin.get() {
        put_opt(c"stdin_fd", integer_obj(Integer::from(UI_CLIENT_STDIN_FD)));
        screen_state().ui_client_forward_stdin.set(false);
    }

    let mut args_items: [Object; 3] = [
        integer_obj(Integer::from(width)),
        integer_obj(Integer::from(height)),
        Object::Dict(opts),
    ];
    let args = Array::from_slice(&mut args_items[..3]);

    rpc_send_event(
        screen_state().ui_client_channel_id.get(),
        c"nvim_ui_attach",
        args,
    );
    screen_state().ui_client_attached.set(true);

    if has_time_fd() {
        time_msg(c"nvim_ui_attach", None);
    }

    let m = api_metadata();
    let mut version = Dict::new();
    let Object::Dict(metadata) = &m else {
        unreachable!("API metadata is a dictionary");
    };
    debug_assert!(metadata.size > 0);
    for (i, item) in metadata.as_slice().iter().enumerate() {
        if item.key.opt_bytes() == Some(b"version") {
            let Object::Dict(value) = &item.value else {
                unreachable!("API version metadata is a dictionary");
            };
            version = value.clone();
            break;
        } else if i + 1 == metadata.size {
            std::process::abort();
        }
    }

    let mut info_items: [KeyValuePair; 3] = [
        KeyValuePair {
            key: cstr_as_string(Some(c"website")),
            value: cstr_as_obj(Some(c"https://neovim.io")),
        },
        KeyValuePair {
            key: cstr_as_string(Some(c"license")),
            value: cstr_as_obj(Some(c"Apache 2")),
        },
        KeyValuePair {
            key: cstr_as_string(Some(c"pid")),
            value: integer_obj(os_get_pid()),
        },
    ];
    let info = Dict::from_slice(&mut info_items[..3]);

    let mut args2_items: [Object; 5] = [
        cstr_as_obj(Some(c"nvim-tui")),
        Object::Dict(version),
        cstr_as_obj(Some(c"ui")),
        Object::Array(ARRAY_INIT),
        Object::Dict(info),
    ];
    let args2 = Array::from_slice(&mut args2_items[..5]);
    rpc_send_event(
        screen_state().ui_client_channel_id.get(),
        c"nvim_set_client_info",
        args2,
    );

    if has_time_fd() {
        time_msg(c"nvim_set_client_info", None);
    }
}

#[cfg(unix)]
pub fn ui_client_detach() {
    rpc_send_event(
        screen_state().ui_client_channel_id.get(),
        c"nvim_ui_detach",
        ARRAY_INIT,
    );
    screen_state().ui_client_attached.set(false);
}

/// Sets `addr` (nvim_chan_set) on the server: the address this client connected to, e.g. a tunnel
/// to the server. :restart sends it back, so the UI reconnects through it.
fn set_chan_addr(addr: &CStr) {
    let mut cfg_items: [KeyValuePair; 1] = [KeyValuePair {
        key: cstr_as_string(Some(c"addr")),
        value: cstr_as_obj(Some(addr)),
    }];
    let config = Dict::from_slice(&mut cfg_items[..1]);
    let mut args_items: [Object; 2] = [integer_obj(0), Object::Dict(config)];
    let args = Array::from_slice(&mut args_items[..2]);
    rpc_send_event(
        screen_state().ui_client_channel_id.get(),
        c"nvim_chan_set",
        args,
    );
}

/// @param server_addr Address of the server (`--server`), or NULL if the client started it (stdio).
pub fn ui_client_run(server_addr: Option<&CStr>) -> ! {
    if let Some(addr) = server_addr {
        set_chan_addr(addr);
    }
    UIC_STATE.with(|s| {
        *s.tui.borrow_mut() = Some(Box::new(TUIData::new()));
    });
    let mut tui_width = UIC_STATE.with(|s| s.tui_width.get());
    let mut tui_height = UIC_STATE.with(|s| s.tui_height.get());
    let mut tui_term = UIC_TUI_TERM.with(|t| t.borrow_mut().take());
    let mut tui_rgb = UIC_STATE.with(|s| s.tui_rgb.get());
    tui_start(&mut tui_width, &mut tui_height, &mut tui_term, &mut tui_rgb);
    UIC_STATE.with(|s| s.tui_width.set(tui_width));
    UIC_STATE.with(|s| s.tui_height.set(tui_height));
    UIC_STATE.with(|s| s.tui_rgb.set(tui_rgb));
    ui_client_attach(tui_width, tui_height, tui_term.as_deref(), tui_rgb);
    UIC_TUI_TERM.with(|t| *t.borrow_mut() = tui_term);

    if os_env_exists(c"NVIM_LOG_FILE", true) {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_run"),
            164,
            true,
            c"test log message",
        );
    }

    time_finish();

    loop {
        let events = crate::event::r#loop::main_loop_events();
        if !multiqueue_empty(events) {
            multiqueue_process_events(events);
        } else {
            loop_poll_events(crate::event::r#loop::main_loop(), -1);
            multiqueue_process_events(crate::event::r#loop::main_loop_events());
        }
    }
}

pub fn ui_client_stop() {
    screen_state().ui_client_attached.set(false);
    if !with_tui(tui_is_stopped) {
        tui_stop();
    }
    if let Some(addr) = UIC_STATE.with(|s| s.detached_addr.borrow_mut().take()) {
        let addr_lossy = addr.to_string_lossy();
        eprintln!(
            "Detached from: {addr_lossy}\nTo reattach run :connect or nvim +\"connect <address>\""
        );
    }
}

pub fn ui_client_set_size(width: i32, height: i32) {
    if screen_state().ui_client_attached.get() {
        let mut args_items: [Object; 2] = [
            integer_obj(Integer::from(width)),
            integer_obj(Integer::from(height)),
        ];
        let args = Array::from_slice(&mut args_items[..2]);
        rpc_send_event(
            screen_state().ui_client_channel_id.get(),
            c"nvim_ui_try_resize",
            args,
        );
    }
    UIC_STATE.with(|s| s.tui_width.set(width));
    UIC_STATE.with(|s| s.tui_height.set(height));
}

pub fn handle_ui_client_redraw(_channel_id: u64, _args: Array) -> Result<Object, Error> {
    Err(api_error!(
        ErrorType::ErrorTypeValidation,
        c"'redraw' cannot be sent as a request",
    ))
}

pub fn ui_client_dict2hlattrs(d: Dict, rgb: bool) -> HlAttrs {
    let mut dict = KeyDictHighlight::default();
    if api_dict_to_keydict(&mut dict, Some(highlight_get_field_cb), d).is_err() {
        return HLATTRS_INIT;
    }

    let mut attrs = dict2hlattrs(&mut dict, rgb, None, None).unwrap_or(HLATTRS_INIT);
    if (dict.is_set_highlight & (1u64 << KEYSET_OPTIDX_HIGHLIGHT_URL)) != 0 {
        attrs.url = with_tui_mut(|tui| tui_add_url(tui, dict.url));
    }
    attrs
}

pub fn ui_client_event_grid_resize(args: Array) {
    let [
        Object::Integer(grid),
        Object::Integer(width),
        Object::Integer(height),
        ..,
    ] = args.as_slice()
    else {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_event_grid_resize"),
            266,
            true,
            c"Invalid UI event: 'grid_resize' wrong type",
        );
        return;
    };
    let (grid, width, height) = (*grid, *width, *height);
    if width < 0
        || width > Integer::from(i32::MAX)
        || height < 0
        || height > Integer::from(i32::MAX)
    {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_event_grid_resize"),
            274,
            true,
            c"Invalid UI event: 'grid_resize' out of range",
        );
        return;
    }
    with_tui_mut(|tui| tui_grid_resize(tui, grid, width, height));

    let cols = width as usize;
    UIC_STATE.with(|s| {
        if s.grid_line_buf_char.borrow().len() < cols {
            *s.grid_line_buf_char.borrow_mut() = vec![0; cols];
            *s.grid_line_buf_attr.borrow_mut() = vec![0; cols];
        }
    });
}

#[inline]
pub(crate) fn ui_client_grid_line_len() -> usize {
    UIC_STATE.with(|s| s.grid_line_buf_char.borrow().len())
}

#[inline]
pub(crate) fn ui_client_set_grid_line_cell(
    off: usize,
    sc: crate::grid::Schar,
    attr: crate::grid::Sattr,
) {
    UIC_STATE.with(|s| {
        s.grid_line_buf_char.borrow_mut()[off] = sc;
        s.grid_line_buf_attr.borrow_mut()[off] = attr;
    });
}

pub fn ui_client_event_grid_line(_args: Array) {
    std::process::abort()
}

pub fn ui_client_event_raw_line(g: &GridLineEvent) {
    let grid = g.args[0];
    let row = g.args[1];
    let startcol = g.args[2];
    let endcol = Integer::from(startcol) + Integer::from(g.coloff);
    let clearcol = endcol + Integer::from(g.clear_width);
    let lineflags = if g.wrap { K_LINE_FLAG_WRAP } else { 0 };

    let (chars, attrs) = UIC_STATE.with(|s| {
        (
            s.grid_line_buf_char.borrow().clone(),
            s.grid_line_buf_attr.borrow().clone(),
        )
    });
    with_tui_mut(|tui| {
        tui_raw_line(
            tui,
            Integer::from(grid),
            Integer::from(row),
            Integer::from(startcol),
            endcol,
            clearcol,
            Integer::from(g.cur_attr),
            lineflags,
            &chars,
            &attrs,
        );
    });
}

pub fn ui_client_event_connect(args: Array) {
    let [Object::String(s), ..] = args.as_slice() else {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_event_connect"),
            281,
            true,
            c"Error handling UI event 'connect'",
        );
        return;
    };
    let bytes = if s.size == 0 || s.data.is_null() {
        &[][..]
    } else {
        s.as_bytes()
    };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let server_addr = std::ffi::CString::new(&bytes[..end]).unwrap();
    multiqueue_put_event(
        loop_queues(crate::event::r#loop::main_loop())
            .fast_events
            .get(),
        Event::UiClientConnect(server_addr),
    );
    screen_state().ui_client_channel_id.set(u64::MAX);
}

pub fn channel_connect_event(server_addr: std::ffi::CString) {
    let mut err: Option<&CStr> = None;
    let is_tcp = socket_address_tcp_host_end(server_addr.to_bytes()).is_some();
    let on_data = CallbackReader::none();
    let chan = channel_connect(is_tcp, &server_addr, true, false, on_data, 500, &mut err);

    if let Some(err_cstr) = err {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"channel_connect_event"),
            302,
            true,
            c"Cannot connect to server %s: %s",
            &server_addr,
            err_cstr,
        );
        ui_client_stop(); // Leave the alternate screen, so the message is visible.
        eprintln!(
            "Cannot connect to server {}: {}",
            server_addr.to_string_lossy(),
            err_cstr.to_string_lossy()
        );
        drop(server_addr);
        screen_state().ui_client_exit_status.set(1);
        os_exit(1);
    }

    UIC_STATE.with(|s| *s.detached_addr.borrow_mut() = None); // Attached to the new server instead.
    screen_state().ui_client_channel_id.set(chan);
    set_chan_addr(&server_addr);
    let term = UIC_TUI_TERM.with(|t| t.borrow().clone());
    ui_client_attach(
        UIC_STATE.with(|s| s.tui_width.get()),
        UIC_STATE.with(|s| s.tui_height.get()),
        term.as_deref(),
        UIC_STATE.with(|s| s.tui_rgb.get()),
    );

    logmsg!(
        LOGLVL_INF,
        None,
        Some(c"channel_connect_event"),
        312,
        true,
        c"Connected to server %s on channel %lld",
        &server_addr,
        chan as i64,
    );
}

pub fn ui_client_event_restart(args: Array) {
    api_free_array(UIC_STATE.with(|s| s.restart_args.get()));
    UIC_STATE.with(|s| s.restart_args.set(copy_array(args)));
    UIC_STATE.with(|s| s.restart_pending.set(true));
}

pub fn ui_client_event_set_restart_on_crash_exit(args: Array) {
    api_free_array(UIC_STATE.with(|s| s.restart_args_after_crash_exit.get()));
    UIC_STATE.with(|s| s.restart_args_after_crash_exit.set(copy_array(args)));
}

pub fn ui_client_attach_to_restarted_server(error_restart: bool) {
    let mut args = UIC_STATE.with(|s| s.restart_args.get());
    let mut start_server = false;
    if !UIC_STATE.with(|s| s.restart_pending.get()) {
        if error_restart
            && screen_state().ui_client_error_exit.get() == -1
            && UIC_STATE
                .with(|s| s.restart_args_after_crash_exit.get())
                .size
                > 0
        {
            start_server = true;
            args = UIC_STATE.with(|s| s.restart_args_after_crash_exit.get());
        } else {
            return;
        }
    }

    UIC_STATE.with(|s| s.restart_pending.set(false));

    let cleanup = || {
        api_free_array(UIC_STATE.with(|s| s.restart_args.get()));
        UIC_STATE.with(|s| s.restart_args.set(ARRAY_INIT));
        api_free_array(UIC_STATE.with(|s| s.restart_args_after_crash_exit.get()));
        UIC_STATE.with(|s| s.restart_args_after_crash_exit.set(ARRAY_INIT));
    };

    let [Object::String(first_arg), ..] = args.as_slice() else {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_attach_to_restarted_server"),
            362,
            true,
            c"Error handling ui event 'restart'",
        );
        cleanup();
        return;
    };
    let chan_id = if start_server {
        let Some(Object::Array(cmdargs)) = args.as_slice().get(1) else {
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"ui_client_attach_to_restarted_server"),
                370,
                true,
                c"Error handling ui event 'restart'",
            );
            cleanup();
            return;
        };
        let mut argv: Vec<CString> = Vec::with_capacity(cmdargs.size);
        for item in cmdargs.as_slice() {
            if let Object::String(value) = item {
                argv.push(crate::memory::bytes_to_cstring(value.cstr_bytes()));
            } else {
                argv.push(CString::default());
            }
        }
        let id = ui_client_start_server(first_arg.as_cstr(), &argv);
        screen_state().ui_client_error_exit.set(-1);
        id
    } else {
        let first_arg_cstr = first_arg.as_cstr();
        let is_tcp = socket_address_tcp_host_end(first_arg_cstr.to_bytes()).is_some();
        let mut err: Option<&CStr> = None;
        let on_data = CallbackReader::none();
        let id = channel_connect(is_tcp, first_arg_cstr, true, false, on_data, 500, &mut err);
        if let Some(err_cstr) = err {
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"ui_client_attach_to_restarted_server"),
                391,
                true,
                c"cannot connect to server %s: %s",
                first_arg,
                err_cstr,
            );
            cleanup();
            return;
        }
        if matches!(args.as_slice().get(1), Some(Object::Dict(_))) {
            let mut chan_args_items: [Object; 2] = [integer_obj(0), args[1].clone()];
            let chan_args = Array::from_slice(&mut chan_args_items[..2]);
            rpc_send_event(id, c"nvim_chan_set", chan_args);
        }
        id
    };

    screen_state().ui_client_channel_id.set(chan_id);
    let term = UIC_TUI_TERM.with(|t| t.borrow().clone());
    ui_client_attach(
        UIC_STATE.with(|s| s.tui_width.get()),
        UIC_STATE.with(|s| s.tui_height.get()),
        term.as_deref(),
        UIC_STATE.with(|s| s.tui_rgb.get()),
    );

    logmsg!(
        LOGLVL_INF,
        None,
        Some(c"ui_client_attach_to_restarted_server"),
        406,
        true,
        c"restarted server address=%s id=%lld",
        first_arg,
        chan_id as i64,
    );
    cleanup();
}

pub fn ui_client_event_error_exit(args: Array) {
    let [Object::Integer(status), ..] = args.as_slice() else {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"ui_client_event_error_exit"),
            419,
            true,
            c"Error handling ui event 'error_exit'",
        );
        return;
    };
    screen_state().ui_client_error_exit.set(*status as i32);
    let detached_addr = args
        .as_slice()
        .get(1)
        .and_then(Object::as_string)
        .filter(|value| value.size > 0)
        .map(|value| value.to_cstring());
    UIC_STATE.with(|s| *s.detached_addr.borrow_mut() = detached_addr);
}

pub fn ui_client_free_all_mem() {
    let tui = UIC_STATE.with(|s| s.tui.borrow_mut().take());
    tui_free_all_mem(tui);
    UIC_STATE.with(|s| {
        *s.detached_addr.borrow_mut() = None;
        let _ = std::mem::take(&mut *s.grid_line_buf_char.borrow_mut());
        let _ = std::mem::take(&mut *s.grid_line_buf_attr.borrow_mut());
    });
}

#[derive(Copy, Clone)]
pub struct UIClientHandler {
    pub name: &'static CStr,
    pub r#fn: Option<fn(args: Array)>,
}

fn log_ui_event_err(func_name: &std::ffi::CStr, ev_name: &std::ffi::CStr) {
    logmsg!(
        LOGLVL_ERR,
        None,
        Some(func_name),
        0,
        true,
        c"Error handling ui event '%s'",
        ev_name,
    );
}

pub fn ui_client_event_mode_info_set(args: Array) {
    let [Object::Boolean(enabled), Object::Array(info), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_mode_info_set", c"mode_info_set");
        return;
    };
    with_tui_mut(|tui| tui_mode_info_set(tui, *enabled, info.clone()));
}

pub fn ui_client_event_update_menu(_args: Array) {
    with_tui_mut(tui_update_menu);
}

pub fn ui_client_event_busy_start(_args: Array) {
    with_tui_mut(tui_busy_start);
}

pub fn ui_client_event_busy_stop(_args: Array) {
    with_tui_mut(tui_busy_stop);
}

pub fn ui_client_event_mouse_on(_args: Array) {
    with_tui_mut(tui_mouse_on);
}

pub fn ui_client_event_mouse_off(_args: Array) {
    with_tui_mut(tui_mouse_off);
}

pub fn ui_client_event_mode_change(args: Array) {
    let [Object::String(mode), Object::Integer(index), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_mode_change", c"mode_change");
        return;
    };
    with_tui_mut(|tui| tui_mode_change(tui, mode.clone(), *index));
}

pub fn ui_client_event_bell(_args: Array) {
    with_tui_mut(tui_bell);
}

pub fn ui_client_event_visual_bell(_args: Array) {
    with_tui_mut(tui_visual_bell);
}

pub fn ui_client_event_flush(_args: Array) {
    with_tui_mut(tui_flush);
}

pub fn ui_client_event_suspend(_args: Array) {
    with_tui_mut(tui_suspend);
}

pub fn ui_client_event_set_title(args: Array) {
    let [Object::String(title), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_set_title", c"set_title");
        return;
    };
    with_tui_mut(|tui| tui_set_title(tui, title.clone()));
}

pub fn ui_client_event_set_icon(args: Array) {
    let [Object::String(icon), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_set_icon", c"set_icon");
        return;
    };
    with_tui_mut(|tui| tui_set_icon(tui, icon.clone()));
}

pub fn ui_client_event_screenshot(args: Array) {
    let [Object::String(path), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_screenshot", c"screenshot");
        return;
    };
    with_tui_mut(|tui| tui_screenshot(tui, path.clone()));
}

pub fn ui_client_event_option_set(args: Array) {
    let [Object::String(name), value, ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_option_set", c"option_set");
        return;
    };
    with_tui_mut(|tui| tui_option_set(tui, name.clone(), value.clone()));
}

pub fn ui_client_event_chdir(args: Array) {
    let [Object::String(path), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_chdir", c"chdir");
        return;
    };
    with_tui_mut(|tui| tui_chdir(tui, path.clone()));
}

pub fn ui_client_event_ui_send(args: Array) {
    let [Object::String(content), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_ui_send", c"ui_send");
        return;
    };
    with_tui_mut(|tui| tui_ui_send(tui, content.clone()));
}

pub fn ui_client_event_default_colors_set(args: Array) {
    let [
        Object::Integer(rgb_fg),
        Object::Integer(rgb_bg),
        Object::Integer(rgb_sp),
        Object::Integer(cterm_fg),
        Object::Integer(cterm_bg),
        ..,
    ] = args.as_slice()
    else {
        log_ui_event_err(c"ui_client_event_default_colors_set", c"default_colors_set");
        return;
    };
    with_tui_mut(|tui| {
        tui_default_colors_set(tui, *rgb_fg, *rgb_bg, *rgb_sp, *cterm_fg, *cterm_bg);
    });
}

pub fn ui_client_event_hl_attr_define(args: Array) {
    let [
        Object::Integer(id),
        Object::Dict(rgb),
        Object::Dict(cterm),
        Object::Array(info),
        ..,
    ] = args.as_slice()
    else {
        log_ui_event_err(c"ui_client_event_hl_attr_define", c"hl_attr_define");
        return;
    };
    let rgb = ui_client_dict2hlattrs(rgb.clone(), true);
    let cterm = ui_client_dict2hlattrs(cterm.clone(), false);
    with_tui_mut(|tui| tui_hl_attr_define(tui, *id, rgb, cterm, info.clone()));
}

pub fn ui_client_event_grid_clear(args: Array) {
    let [Object::Integer(grid), ..] = args.as_slice() else {
        log_ui_event_err(c"ui_client_event_grid_clear", c"grid_clear");
        return;
    };
    with_tui_mut(|tui| tui_grid_clear(tui, *grid));
}

pub fn ui_client_event_grid_cursor_goto(args: Array) {
    let [
        Object::Integer(grid),
        Object::Integer(row),
        Object::Integer(col),
        ..,
    ] = args.as_slice()
    else {
        log_ui_event_err(c"ui_client_event_grid_cursor_goto", c"grid_cursor_goto");
        return;
    };
    with_tui_mut(|tui| tui_grid_cursor_goto(tui, *grid, *row, *col));
}

pub fn ui_client_event_grid_scroll(args: Array) {
    let [
        Object::Integer(grid),
        Object::Integer(top),
        Object::Integer(bottom),
        Object::Integer(left),
        Object::Integer(right),
        Object::Integer(rows),
        Object::Integer(cols),
        ..,
    ] = args.as_slice()
    else {
        log_ui_event_err(c"ui_client_event_grid_scroll", c"grid_scroll");
        return;
    };
    with_tui_mut(|tui| {
        tui_grid_scroll(tui, *grid, *top, *bottom, *left, *right, *rows, *cols);
    });
}

const EVENT_HANDLERS: [UIClientHandler; 28] = [
    UIClientHandler {
        name: c"bell",
        r#fn: Some(ui_client_event_bell),
    },
    UIClientHandler {
        name: c"chdir",
        r#fn: Some(ui_client_event_chdir),
    },
    UIClientHandler {
        name: c"flush",
        r#fn: Some(ui_client_event_flush),
    },
    UIClientHandler {
        name: c"connect",
        r#fn: Some(ui_client_event_connect),
    },
    UIClientHandler {
        name: c"restart",
        r#fn: Some(ui_client_event_restart),
    },
    UIClientHandler {
        name: c"suspend",
        r#fn: Some(ui_client_event_suspend),
    },
    UIClientHandler {
        name: c"ui_send",
        r#fn: Some(ui_client_event_ui_send),
    },
    UIClientHandler {
        name: c"mouse_on",
        r#fn: Some(ui_client_event_mouse_on),
    },
    UIClientHandler {
        name: c"set_icon",
        r#fn: Some(ui_client_event_set_icon),
    },
    UIClientHandler {
        name: c"busy_stop",
        r#fn: Some(ui_client_event_busy_stop),
    },
    UIClientHandler {
        name: c"grid_line",
        r#fn: Some(ui_client_event_grid_line),
    },
    UIClientHandler {
        name: c"mouse_off",
        r#fn: Some(ui_client_event_mouse_off),
    },
    UIClientHandler {
        name: c"set_title",
        r#fn: Some(ui_client_event_set_title),
    },
    UIClientHandler {
        name: c"busy_start",
        r#fn: Some(ui_client_event_busy_start),
    },
    UIClientHandler {
        name: c"error_exit",
        r#fn: Some(ui_client_event_error_exit),
    },
    UIClientHandler {
        name: c"grid_clear",
        r#fn: Some(ui_client_event_grid_clear),
    },
    UIClientHandler {
        name: c"option_set",
        r#fn: Some(ui_client_event_option_set),
    },
    UIClientHandler {
        name: c"screenshot",
        r#fn: Some(ui_client_event_screenshot),
    },
    UIClientHandler {
        name: c"mode_change",
        r#fn: Some(ui_client_event_mode_change),
    },
    UIClientHandler {
        name: c"update_menu",
        r#fn: Some(ui_client_event_update_menu),
    },
    UIClientHandler {
        name: c"visual_bell",
        r#fn: Some(ui_client_event_visual_bell),
    },
    UIClientHandler {
        name: c"grid_resize",
        r#fn: Some(ui_client_event_grid_resize),
    },
    UIClientHandler {
        name: c"grid_scroll",
        r#fn: Some(ui_client_event_grid_scroll),
    },
    UIClientHandler {
        name: c"mode_info_set",
        r#fn: Some(ui_client_event_mode_info_set),
    },
    UIClientHandler {
        name: c"hl_attr_define",
        r#fn: Some(ui_client_event_hl_attr_define),
    },
    UIClientHandler {
        name: c"grid_cursor_goto",
        r#fn: Some(ui_client_event_grid_cursor_goto),
    },
    UIClientHandler {
        name: c"default_colors_set",
        r#fn: Some(ui_client_event_default_colors_set),
    },
    UIClientHandler {
        name: c"_set_restart_on_crash_exit",
        r#fn: Some(ui_client_event_set_restart_on_crash_exit),
    },
];

pub fn ui_client_handler_hash(bytes: &[u8]) -> Option<usize> {
    let low: usize = match bytes.len() {
        4 => 0,
        5 => match bytes[0] {
            b'c' => 1,
            b'f' => 2,
            _ => return None,
        },
        7 => match bytes[0] {
            b'c' => 3,
            b'r' => 4,
            b's' => 5,
            b'u' => 6,
            _ => return None,
        },
        8 => match bytes[0] {
            b'm' => 7,
            b's' => 8,
            _ => return None,
        },
        9 => match bytes[0] {
            b'b' => 9,
            b'g' => 10,
            b'm' => 11,
            b's' => 12,
            _ => return None,
        },
        10 => match bytes[0] {
            b'b' => 13,
            b'e' => 14,
            b'g' => 15,
            b'o' => 16,
            b's' => 17,
            _ => return None,
        },
        11 => match bytes[5] {
            b'c' => 18,
            b'e' => 19,
            b'l' => 20,
            b'r' => 21,
            b's' => 22,
            _ => return None,
        },
        13 => 23,
        14 => 24,
        16 => 25,
        18 => 26,
        26 => 27,
        _ => return None,
    };
    (bytes == EVENT_HANDLERS[low].name.to_bytes()).then_some(low)
}

pub fn ui_client_get_redraw_handler(name: &[u8]) -> UIClientHandler {
    ui_client_handler_hash(name).map_or(
        UIClientHandler {
            name: c"",
            r#fn: None,
        },
        |hash| EVENT_HANDLERS[hash],
    )
}
