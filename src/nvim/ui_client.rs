//! Nvim's own UI client, which attaches to a child or remote Nvim server (`src/nvim/ui_client.c`).

use crate::api::private::dispatch::KeyDict_highlight_get_field as highlight_get_field_cb;
use crate::api::private::helpers::{
    OBJECT_NIL, api_dict_to_keydict, api_free_array, api_metadata, api_set_error, boolean_obj,
    copy_array, cstr_as_obj, cstr_as_string, integer_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::ui::{ARRAY_INIT, kLineFlagWrap};
use crate::channel::{CallbackReader, channel_connect, channel_job_start, kChannelStdinPipe};
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
use crate::highlight::{HLATTRS_INIT, HlAttrs, KeyDict_highlight, dict2hlattrs};
use crate::log::{LOGLVL_ERR, LOGLVL_INF, logmsg};
use crate::main_init::os_exit;
use crate::memory::{Arena};
use crate::msgpack_rpc::channel::rpc_send_event;
use crate::msgpack_rpc::packer::Integer;
use crate::msgpack_rpc::unpacker::GridLineEvent;
use crate::os::env::os_env_exists;
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
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub(crate) const UI_CLIENT_STDIN_FD: c_int = 3;
use crate::highlight::KEYSET_OPTIDX_highlight__url;

struct UiClientState {
    tui: Option<Box<TUIData>>,
    tui_width: c_int,
    tui_height: c_int,
    tui_term: Option<std::ffi::CString>,
    tui_rgb: bool,
    restart_args: Array,
    restart_pending: bool,
    restart_args_after_crash_exit: Array,
}

impl UiClientState {
    const fn new() -> Self {
        Self {
            tui: None,
            tui_width: 0,
            tui_height: 0,
            tui_term: None,
            tui_rgb: false,
            restart_args: ARRAY_INIT,
            restart_pending: false,
            restart_args_after_crash_exit: ARRAY_INIT,
        }
    }
}

static UIC_STATE: GlobalState<UiClientState> = GlobalState::new(UiClientState::new());

#[inline]
fn uic_st() -> &'static mut UiClientState {
    unsafe { &mut *UIC_STATE.as_ptr() }
}

#[inline]
fn tui_ptr() -> &'static mut TUIData {
    uic_st().tui.as_deref_mut().unwrap()
}

use crate::event::r#loop::uv_os_getpid;

pub fn ui_client_start_server(
    exepath: *const c_char,
    argc: usize,
    argv: *mut *mut c_char,
) -> u64 {
    unsafe {
        let mut args = Vec::with_capacity(1 + argc);
        args.push(crate::memory::cstr_to_cstring(*argv));
        args.push(c"--embed".to_owned());
        for i in 1..argc {
            args.push(crate::memory::cstr_to_cstring(*argv.add(i)));
        }

        let on_stdout = CallbackReader::none();
        let mut on_err = CallbackReader::none();
        on_err.fwd_err = true;
        let on_exit: Callback = Callback::new();

        let mut exit_status: i64 = 0;
        let channel = channel_job_start(
            args,
            exepath,
            on_stdout,
            on_err,
            on_exit,
            false,
            true,
            true,
            true,
            kChannelStdinPipe,
            ptr::null(),
            0,
            0,
            ptr::null_mut(),
            &raw mut exit_status,
        );
        if channel.is_null() {
            return 0;
        }

        if g_st().ui_client_forward_stdin {
            close(0);
            #[cfg(windows)]
            {
                os_open_conin_fd();
            }
            #[cfg(not(windows))]
            {
                let _ = dup(if g_st().stderr_isatty { 2 } else { 1 });
            }
        }

        *(channel.cast::<u64>())
    }
}

/// Attaches this client to the UI channel, and sets its client info (`ui_client_attach`).
pub fn ui_client_attach(
    width: c_int,
    height: c_int,
    term: Option<&std::ffi::CStr>,
    rgb: bool,
) {
    unsafe {
        let mut opts_items: [KeyValuePair; 9] = [KeyValuePair::new(); 9];
        let mut opts = Dict {
            items: opts_items.as_mut_ptr(),
            size: 0,
            capacity: 9,
        };
        let mut put_opt = |k: *const c_char, v: Object| {
            *opts.items.add(opts.size) = KeyValuePair {
                key: cstr_as_string(k),
                value: v,
            };
            opts.size += 1;
        };
        put_opt(c"rgb".as_ptr(), boolean_obj(rgb));
        put_opt(c"ext_linegrid".as_ptr(), boolean_obj(true));
        put_opt(c"ext_termcolors".as_ptr(), boolean_obj(true));
        if let Some(term) = term {
            put_opt(c"term_name".as_ptr(), cstr_as_obj(term.as_ptr()));
        }
        put_opt(
            c"term_colors".as_ptr(),
            integer_obj(Integer::from(g_st().t_colors)),
        );
        put_opt(c"stdin_tty".as_ptr(), boolean_obj(g_st().stdin_isatty));
        put_opt(c"stdout_tty".as_ptr(), boolean_obj(g_st().stdout_isatty));
        if g_st().ui_client_forward_stdin {
            put_opt(
                c"stdin_fd".as_ptr(),
                integer_obj(Integer::from(UI_CLIENT_STDIN_FD)),
            );
            g_st().ui_client_forward_stdin = false;
        }

        let mut args_items: [Object; 3] = [
            integer_obj(Integer::from(width)),
            integer_obj(Integer::from(height)),
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData::Dict(opts),
            },
        ];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };

        rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_attach", args);
        g_st().ui_client_attached = true;

        if has_time_fd() {
            time_msg(c"nvim_ui_attach", None);
        }

        let m = api_metadata();
        let mut version = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        debug_assert!(m.data.dict().size > 0);
        for i in 0..m.data.dict().size {
            let item = *m.data.dict().items.add(i);
            if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"version") {
                version = item.value.data.dict();
                break;
            } else if i + 1 == m.data.dict().size {
                std::process::abort();
            }
        }

        let mut info_items: [KeyValuePair; 3] = [
            KeyValuePair {
                key: cstr_as_string(c"website"),
                value: cstr_as_obj(c"https://neovim.io"),
            },
            KeyValuePair {
                key: cstr_as_string(c"license"),
                value: cstr_as_obj(c"Apache 2"),
            },
            KeyValuePair {
                key: cstr_as_string(c"pid"),
                value: integer_obj(Integer::from(uv_os_getpid())),
            },
        ];
        let info = Dict {
            items: info_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };

        let mut args2_items: [Object; 5] = [
            cstr_as_obj(c"nvim-tui"),
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData::Dict(version),
            },
            cstr_as_obj(c"ui"),
            Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData::Array(ARRAY_INIT),
            },
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData::Dict(info),
            },
        ];
        let args2 = Array {
            items: args2_items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        };
        rpc_send_event(
            g_st().ui_client_channel_id,
            c"nvim_set_client_info",
            args2,
        );

        if has_time_fd() {
            time_msg(c"nvim_set_client_info", None);
        }
    }
}

#[cfg(unix)]
pub fn ui_client_detach() {
    rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_detach", ARRAY_INIT);
    g_st().ui_client_attached = false;
}

pub fn ui_client_run() -> ! {
    unsafe {
        tui_start(
            &raw mut uic_st().tui,
            &mut uic_st().tui_width,
            &mut uic_st().tui_height,
            &raw mut uic_st().tui_term,
            &mut uic_st().tui_rgb,
        );
        ui_client_attach(
            uic_st().tui_width,
            uic_st().tui_height,
            uic_st().tui_term.as_deref(),
            uic_st().tui_rgb,
        );

        if os_env_exists(c"NVIM_LOG_FILE", true) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_run",
                164,
                true,
                c"test log message",
            );
        }

        time_finish();

        loop {
            let events = (*loop_queues(&mut *(&raw mut g_st().main_loop).cast())).events;
            if !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events(&mut *(&raw mut g_st().main_loop).cast(), -1);
                multiqueue_process_events(events);
            }
        }
    }
}

pub fn ui_client_stop() {
    g_st().ui_client_attached = false;
        let t = tui_ptr();
        if !tui_is_stopped(t) {
            tui_stop(t);
        }
}

pub fn ui_client_set_size(width: c_int, height: c_int) {
    if g_st().ui_client_attached {
            let mut args_items: [Object; 2] = [
                integer_obj(Integer::from(width)),
                integer_obj(Integer::from(height)),
            ];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            rpc_send_event(g_st().ui_client_channel_id, c"nvim_ui_try_resize", args);
        }
        uic_st().tui_width = width;
        uic_st().tui_height = height;
}

pub fn handle_ui_client_redraw(
    _channel_id: u64,
    _args: Array,
    _arena: &mut Arena,
    error: &mut Error,
) -> Object {
    api_set_error!(
            error,
            ErrorType::kErrorTypeValidation,
            c"'redraw' cannot be sent as a request",
        );
        OBJECT_NIL
}

pub fn ui_client_dict2hlattrs(d: Dict, rgb: bool) -> HlAttrs {
    let mut err = Error::new();
        let mut dict = KeyDict_highlight::default();
        if !api_dict_to_keydict(
            (&raw mut dict).cast(),
            Some(highlight_get_field_cb),
            d,
            &mut err,
        ) {
            return HLATTRS_INIT;
        }

        let mut attrs = dict2hlattrs(
            &mut dict,
            rgb,
            None,
            ptr::null_mut(),
            &mut err,
        );
        if (dict.is_set__highlight_ & (1u64 << KEYSET_OPTIDX_highlight__url)) != 0 {
            attrs.url = tui_add_url(tui_ptr(), dict.url.data);
        }
        attrs
}

pub fn ui_client_event_grid_resize(args: Array) {
    unsafe {
        if args.size < 3
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || args[1].obj_type != ObjectType::kObjectTypeInteger
            || args[2].obj_type != ObjectType::kObjectTypeInteger
        {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_grid_resize",
                242,
                true,
                c"Error handling ui event 'grid_resize'",
            );
            return;
        }

        let grid = (*args.items).data.integer();
        let width = args[1].data.integer();
        let height = args[2].data.integer();
        tui_grid_resize(tui_ptr(), grid, width, height);

        let cols = width as usize;
        let char_buf = &raw const g_st().grid_line_buf_char;
        if (*char_buf).len() < cols {
            g_st().grid_line_buf_char = vec![0; cols];
            g_st().grid_line_buf_attr = vec![0; cols];
        }
    }
}

pub fn ui_client_event_grid_line(_args: Array) {
    std::process::abort()
}

pub fn ui_client_event_raw_line(g: &mut GridLineEvent) {
    unsafe {
        let grid = g.args[0];
        let row = g.args[1];
        let startcol = g.args[2];
        let endcol = Integer::from(startcol) + Integer::from(g.coloff);
        let clearcol = endcol + Integer::from(g.clear_width);
        let lineflags = if g.wrap { kLineFlagWrap } else { 0 };

        let char_buf = &raw mut g_st().grid_line_buf_char;
        let attr_buf = &raw mut g_st().grid_line_buf_attr;
        tui_raw_line(
            tui_ptr(),
            Integer::from(grid),
            Integer::from(row),
            Integer::from(startcol),
            endcol,
            clearcol,
            Integer::from(g.cur_attr),
            lineflags,
            (*char_buf).as_mut_ptr(),
            (*attr_buf).as_mut_ptr(),
        );
    }
}

pub fn ui_client_event_connect(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_connect",
                281,
                true,
                c"Error handling UI event 'connect'",
            );
            return;
        }

        let s = (*args.items).data.string();
        let bytes = if s.size == 0 || s.data.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(s.data.cast::<u8>(), s.size)
        };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let server_addr = std::ffi::CString::new(&bytes[..end]).unwrap();
        multiqueue_put_event(
            (*loop_queues(&mut *(&raw mut g_st().main_loop).cast())).fast_events,
            Event::UiClientConnect(server_addr),
        );
        g_st().ui_client_channel_id = u64::MAX;
    }
}

pub fn channel_connect_event(server_addr: std::ffi::CString) {
    let mut err: *const c_char = c"".as_ptr();
        let addr_ptr = server_addr.as_ptr();
        let is_tcp = !socket_address_tcp_host_end(addr_ptr).is_null();
        let on_data = CallbackReader::none();
        let chan = channel_connect(is_tcp, addr_ptr, true, on_data, 50, &mut err);

        if !(crate::memory::cstr_as_opt_bytes(err) == Some(b"")) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"channel_connect_event",
                302,
                true,
                c"Cannot connect to server %s: %s",
                addr_ptr,
                err,
            );
            drop(server_addr);
            g_st().ui_client_exit_status = 1;
            os_exit(1);
        }

        g_st().ui_client_channel_id = chan;
        ui_client_attach(
            uic_st().tui_width,
            uic_st().tui_height,
            uic_st().tui_term.as_deref(),
            uic_st().tui_rgb,
        );

        logmsg!(
            LOGLVL_INF,
            ptr::null(),
            c"channel_connect_event",
            312,
            true,
            c"Connected to server %s on channel %lld",
            addr_ptr,
            chan as i64,
        );
}

pub fn ui_client_event_restart(args: Array) {
    api_free_array(uic_st().restart_args);
        uic_st().restart_args = copy_array(args, None);
        uic_st().restart_pending = true;
}

pub fn ui_client_event__set_restart_on_crash_exit(args: Array) {
    api_free_array(uic_st().restart_args_after_crash_exit);
        uic_st().restart_args_after_crash_exit = copy_array(args, None);
}

pub fn ui_client_attach_to_restarted_server(error_restart: bool) {
    unsafe {
        let mut args = uic_st().restart_args;
        let mut start_server = false;
        if !uic_st().restart_pending {
            if error_restart && g_st().ui_client_error_exit == -1 && uic_st().restart_args_after_crash_exit.size > 0
            {
                start_server = true;
                args = uic_st().restart_args_after_crash_exit;
            } else {
                return;
            }
        }

        uic_st().restart_pending = false;

        let cleanup = || {
            api_free_array(uic_st().restart_args);
            uic_st().restart_args = ARRAY_INIT;
            api_free_array(uic_st().restart_args_after_crash_exit);
            uic_st().restart_args_after_crash_exit = ARRAY_INIT;
        };

        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_attach_to_restarted_server",
                362,
                true,
                c"Error handling ui event 'restart'",
            );
            cleanup();
            return;
        }

        let first_arg = (*args.items).data.string().data;
        let chan_id = if start_server {
            if args.size < 2 || args[1].obj_type != ObjectType::kObjectTypeArray {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"ui_client_attach_to_restarted_server",
                    370,
                    true,
                    c"Error handling ui event 'restart'",
                );
                cleanup();
                return;
            }
            let cmdargs = args[1].data.array();
            let mut argv: Vec<*mut c_char> = vec![ptr::null_mut(); cmdargs.size + 1];
            for (i, dst) in argv.iter_mut().enumerate().take(cmdargs.size) {
                let item = cmdargs[i];
                if item.obj_type == ObjectType::kObjectTypeString {
                    *dst = item.data.string().data;
                }
                if (*dst).is_null() {
                    *dst = c"".as_ptr().cast_mut();
                }
            }
            let id = ui_client_start_server(first_arg, cmdargs.size, argv.as_mut_ptr());
            g_st().ui_client_error_exit = -1;
            id
        } else {
            let is_tcp = !socket_address_tcp_host_end(first_arg).is_null();
            let mut err: *const c_char = ptr::null();
            let on_data = CallbackReader::none();
            let id = channel_connect(is_tcp, first_arg, true, on_data, 50, &mut err);
            if !err.is_null() {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"ui_client_attach_to_restarted_server",
                    391,
                    true,
                    c"cannot connect to server %s: %s",
                    first_arg,
                    err,
                );
                cleanup();
                return;
            }
            if args.size > 1 && args[1].obj_type == ObjectType::kObjectTypeDict {
                let mut chan_args_items: [Object; 2] = [integer_obj(0), args[1]];
                let chan_args = Array {
                    items: chan_args_items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                rpc_send_event(id, c"nvim_chan_set", chan_args);
            }
            id
        };

        g_st().ui_client_channel_id = chan_id;
        ui_client_attach(
            uic_st().tui_width,
            uic_st().tui_height,
            uic_st().tui_term.as_deref(),
            uic_st().tui_rgb,
        );

        logmsg!(
            LOGLVL_INF,
            ptr::null(),
            c"ui_client_attach_to_restarted_server",
            406,
            true,
            c"restarted server address=%s id=%lld",
            first_arg,
            chan_id as i64,
        );
        cleanup();
    }
}

pub fn ui_client_event_error_exit(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeInteger {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_error_exit",
                419,
                true,
                c"Error handling ui event 'error_exit'",
            );
            return;
        }
        g_st().ui_client_error_exit = (*args.items).data.integer() as c_int;
    }
}

pub fn ui_client_free_all_mem() {
    unsafe {
        let tui_out = &raw mut uic_st().tui;
        tui_free_all_mem((*tui_out).take());
        g_st().grid_line_buf_char = Vec::new();
        g_st().grid_line_buf_attr = Vec::new();
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct UIClientHandler {
    pub name: &'static CStr,
    pub r#fn: Option<fn(args: Array)>,
}

fn log_ui_event_err(func_name: &std::ffi::CStr, ev_name: &std::ffi::CStr) {
    logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            func_name.as_ptr(),
            0,
            true,
            c"Error handling ui event '%s'",
            ev_name.as_ptr(),
        );
}

pub fn ui_client_event_mode_info_set(args: Array) {
    unsafe {
        if args.size < 2
            || (*args.items).obj_type != ObjectType::kObjectTypeBoolean
            || args[1].obj_type != ObjectType::kObjectTypeArray
        {
            log_ui_event_err(
                c"ui_client_event_mode_info_set",
                c"mode_info_set",
            );
            return;
        }
        tui_mode_info_set(
            tui_ptr(),
            (*args.items).data.boolean(),
            args[1].data.array(),
        );
    }
}

pub fn ui_client_event_update_menu(_args: Array) {
    tui_update_menu(tui_ptr());
}

pub fn ui_client_event_busy_start(_args: Array) {
    tui_busy_start(tui_ptr());
}

pub fn ui_client_event_busy_stop(_args: Array) {
    tui_busy_stop(tui_ptr());
}

pub fn ui_client_event_mouse_on(_args: Array) {
    tui_mouse_on(tui_ptr());
}

pub fn ui_client_event_mouse_off(_args: Array) {
    tui_mouse_off(tui_ptr());
}

pub fn ui_client_event_mode_change(args: Array) {
    unsafe {
        if args.size < 2
            || (*args.items).obj_type != ObjectType::kObjectTypeString
            || args[1].obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_mode_change",
                c"mode_change",
            );
            return;
        }
        tui_mode_change(
            tui_ptr(),
            (*args.items).data.string(),
            args[1].data.integer(),
        );
    }
}

pub fn ui_client_event_bell(_args: Array) {
    tui_bell(tui_ptr());
}

pub fn ui_client_event_visual_bell(_args: Array) {
    tui_visual_bell(tui_ptr());
}

pub fn ui_client_event_flush(_args: Array) {
    tui_flush(tui_ptr());
}

pub fn ui_client_event_suspend(_args: Array) {
    tui_suspend(tui_ptr());
}

pub fn ui_client_event_set_title(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_set_title", c"set_title");
            return;
        }
        tui_set_title(tui_ptr(), (*args.items).data.string());
    }
}

pub fn ui_client_event_set_icon(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_set_icon", c"set_icon");
            return;
        }
        tui_set_icon(tui_ptr(), (*args.items).data.string());
    }
}

pub fn ui_client_event_screenshot(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(
                c"ui_client_event_screenshot",
                c"screenshot",
            );
            return;
        }
        tui_screenshot(tui_ptr(), (*args.items).data.string());
    }
}

pub fn ui_client_event_option_set(args: Array) {
    unsafe {
        if args.size < 2 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(
                c"ui_client_event_option_set",
                c"option_set",
            );
            return;
        }
        tui_option_set(tui_ptr(), (*args.items).data.string(), args[1]);
    }
}

pub fn ui_client_event_chdir(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_chdir", c"chdir");
            return;
        }
        tui_chdir(tui_ptr(), (*args.items).data.string());
    }
}

pub fn ui_client_event_ui_send(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_ui_send", c"ui_send");
            return;
        }
        tui_ui_send(tui_ptr(), (*args.items).data.string());
    }
}

pub fn ui_client_event_default_colors_set(args: Array) {
    unsafe {
        if args.size < 5
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || args[1].obj_type != ObjectType::kObjectTypeInteger
            || args[2].obj_type != ObjectType::kObjectTypeInteger
            || args[3].obj_type != ObjectType::kObjectTypeInteger
            || args[4].obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_default_colors_set",
                c"default_colors_set",
            );
            return;
        }
        tui_default_colors_set(
            tui_ptr(),
            (*args.items).data.integer(),
            args[1].data.integer(),
            args[2].data.integer(),
            args[3].data.integer(),
            args[4].data.integer(),
        );
    }
}

pub fn ui_client_event_hl_attr_define(args: Array) {
    unsafe {
        if args.size < 4
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || args[1].obj_type != ObjectType::kObjectTypeDict
            || args[2].obj_type != ObjectType::kObjectTypeDict
            || args[3].obj_type != ObjectType::kObjectTypeArray
        {
            log_ui_event_err(
                c"ui_client_event_hl_attr_define",
                c"hl_attr_define",
            );
            return;
        }
        let arg_1 = (*args.items).data.integer();
        let arg_2 = ui_client_dict2hlattrs(args[1].data.dict(), true);
        let arg_3 = ui_client_dict2hlattrs(args[2].data.dict(), false);
        let arg_4 = args[3].data.array();
        tui_hl_attr_define(tui_ptr(), arg_1, arg_2, arg_3, arg_4);
    }
}

pub fn ui_client_event_grid_clear(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeInteger {
            log_ui_event_err(
                c"ui_client_event_grid_clear",
                c"grid_clear",
            );
            return;
        }
        tui_grid_clear(tui_ptr(), (*args.items).data.integer());
    }
}

pub fn ui_client_event_grid_cursor_goto(args: Array) {
    unsafe {
        if args.size < 3
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || args[1].obj_type != ObjectType::kObjectTypeInteger
            || args[2].obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_grid_cursor_goto",
                c"grid_cursor_goto",
            );
            return;
        }
        tui_grid_cursor_goto(
            tui_ptr(),
            (*args.items).data.integer(),
            args[1].data.integer(),
            args[2].data.integer(),
        );
    }
}

pub fn ui_client_event_grid_scroll(args: Array) {
    unsafe {
        if args.size < 7
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || args[1].obj_type != ObjectType::kObjectTypeInteger
            || args[2].obj_type != ObjectType::kObjectTypeInteger
            || args[3].obj_type != ObjectType::kObjectTypeInteger
            || args[4].obj_type != ObjectType::kObjectTypeInteger
            || args[5].obj_type != ObjectType::kObjectTypeInteger
            || args[6].obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_grid_scroll",
                c"grid_scroll",
            );
            return;
        }
        tui_grid_scroll(
            tui_ptr(),
            (*args.items).data.integer(),
            args[1].data.integer(),
            args[2].data.integer(),
            args[3].data.integer(),
            args[4].data.integer(),
            args[5].data.integer(),
            args[6].data.integer(),
        );
    }
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
        r#fn: Some(ui_client_event__set_restart_on_crash_exit),
    },
];

pub fn ui_client_handler_hash(str_in: *const c_char, len: usize) -> c_int {
    if str_in.is_null() {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(str_in.cast::<u8>(), len) };
    let low: isize = match len {
        4 => 0,
        5 => match bytes[0] {
            b'c' => 1,
            b'f' => 2,
            _ => -1,
        },
        7 => match bytes[0] {
            b'c' => 3,
            b'r' => 4,
            b's' => 5,
            b'u' => 6,
            _ => -1,
        },
        8 => match bytes[0] {
            b'm' => 7,
            b's' => 8,
            _ => -1,
        },
        9 => match bytes[0] {
            b'b' => 9,
            b'g' => 10,
            b'm' => 11,
            b's' => 12,
            _ => -1,
        },
        10 => match bytes[0] {
            b'b' => 13,
            b'e' => 14,
            b'g' => 15,
            b'o' => 16,
            b's' => 17,
            _ => -1,
        },
        11 => match bytes[5] {
            b'c' => 18,
            b'e' => 19,
            b'l' => 20,
            b'r' => 21,
            b's' => 22,
            _ => -1,
        },
        13 => 23,
        14 => 24,
        16 => 25,
        18 => 26,
        26 => 27,
        _ => -1,
    };
    if low < 0 {
        return -1;
    }
    if bytes != EVENT_HANDLERS[low as usize].name.to_bytes() {
        return -1;
    }
    low as c_int
}

pub fn ui_client_get_redraw_handler(
    name: *const c_char,
    name_len: usize,
) -> UIClientHandler {
    let hash = ui_client_handler_hash(name, name_len);
        if hash < 0 {
            return UIClientHandler {
                name: c"",
                r#fn: None,
            };
        }
        EVENT_HANDLERS[hash as usize]
}
