//! Nvim's own UI client, which attaches to a child or remote Nvim server (`src/nvim/ui_client.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use crate::api::private::dispatch::KeyDict_highlight_get_field as highlight_get_field_cb;
use crate::api::private::helpers::{
    OBJECT_NIL, api_dict_to_keydict, api_free_array, api_metadata, api_set_error, bool_obj,
    copy_array, cstr_as_string, cstr_obj, int_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::ui::{ARRAY_INIT, kLineFlagWrap};
use crate::channel::{CallbackReader, channel_connect, channel_job_start, kChannelStdinPipe};
use crate::eval::typval::Callback;
use crate::event::r#loop::{loop_poll_events, loop_queues};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, multiqueue_empty, multiqueue_process_events,
    multiqueue_put_event,
};
use crate::event::socket::socket_address_tcp_host_end;
#[cfg(not(windows))]
use crate::fileio::{close as close_fd, dup};
use crate::globals::*;
use crate::grid::{sattr_T, schar_T};
use crate::highlight::{HLATTRS_INIT, KeyDict_highlight, dict2hlattrs};
use crate::log::{LOGLVL_ERR, LOGLVL_INF, logmsg};
use crate::main_init::os_exit;
use crate::map::HlAttrs;
use crate::memory::{Arena, strequal, xcalloc, xfree, xmalloc, xmemdupz, xstrdup};
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
use std::ffi::{c_char, c_int, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

const UI_CLIENT_STDIN_FD: Integer = 3;
const KEYSET_OPTIDX_highlight__url: u64 = 5;

#[unsafe(no_mangle)]
pub static mut tui: *mut TUIData = ptr::null_mut();
static mut tui_width: c_int = 0;
static mut tui_height: c_int = 0;
static mut tui_term: *mut c_char = c"".as_ptr().cast_mut();
static mut tui_rgb: bool = false;

static mut restart_args: Array = ARRAY_INIT;
static mut restart_pending: bool = false;
static mut restart_args_after_crash_exit: Array = ARRAY_INIT;

#[cfg(windows)]
use crate::fileio::_close as close_fd;

use crate::event::r#loop::uv_os_getpid;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_start_server(
    exepath: *const c_char,
    argc: usize,
    argv: *mut *mut c_char,
) -> u64 {
    unsafe {
        let args = xmalloc((2 + argc) * size_of::<*mut c_char>()).cast::<*mut c_char>();
        let mut args_idx: usize = 0;
        *args.add(args_idx) = xstrdup(*argv);
        args_idx += 1;
        *args.add(args_idx) = xstrdup(c"--embed".as_ptr());
        args_idx += 1;
        for i in 1..argc {
            *args.add(args_idx) = xstrdup(*argv.add(i));
            args_idx += 1;
        }
        *args.add(args_idx) = ptr::null_mut();

        let on_stdout: CallbackReader = zeroed();
        let mut on_err: CallbackReader = zeroed();
        on_err.fwd_err = true;
        let on_exit: Callback = zeroed();

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

        if ui_client_forward_stdin {
            close_fd(0);
            #[cfg(windows)]
            {
                os_open_conin_fd();
            }
            #[cfg(not(windows))]
            {
                let _ = dup(if stderr_isatty { 2 } else { 1 });
            }
        }

        *(channel.cast::<u64>())
    }
}

/// Attaches this client to the UI channel, and sets its client info (`ui_client_attach`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_attach(
    width: c_int,
    height: c_int,
    term: *mut c_char,
    rgb: bool,
) {
    unsafe {
        let mut opts_items: [KeyValuePair; 9] = zeroed();
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
        put_opt(c"rgb".as_ptr(), bool_obj(rgb));
        put_opt(c"ext_linegrid".as_ptr(), bool_obj(true));
        put_opt(c"ext_termcolors".as_ptr(), bool_obj(true));
        if !term.is_null() {
            put_opt(c"term_name".as_ptr(), cstr_obj(term));
        }
        put_opt(c"term_colors".as_ptr(), int_obj(Integer::from(t_colors)));
        put_opt(c"stdin_tty".as_ptr(), bool_obj(stdin_isatty));
        put_opt(c"stdout_tty".as_ptr(), bool_obj(stdout_isatty));
        if ui_client_forward_stdin {
            put_opt(c"stdin_fd".as_ptr(), int_obj(UI_CLIENT_STDIN_FD));
            ui_client_forward_stdin = false;
        }

        let mut args_items: [Object; 3] = [
            int_obj(Integer::from(width)),
            int_obj(Integer::from(height)),
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: opts },
            },
        ];
        let args = Array {
            items: args_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };

        rpc_send_event(ui_client_channel_id, c"nvim_ui_attach".as_ptr(), args);
        ui_client_attached = true;

        if !time_fd.is_null() {
            time_msg(c"nvim_ui_attach".as_ptr(), ptr::null());
        }

        let m = api_metadata();
        let mut version = Dict {
            items: ptr::null_mut(),
            size: 0,
            capacity: 0,
        };
        debug_assert!(m.data.dict.size > 0);
        for i in 0..m.data.dict.size {
            let item = *m.data.dict.items.add(i);
            if strequal(item.key.data, c"version".as_ptr()) {
                version = item.value.data.dict;
                break;
            } else if i + 1 == m.data.dict.size {
                std::process::abort();
            }
        }

        let mut info_items: [KeyValuePair; 3] = [
            KeyValuePair {
                key: cstr_as_string(c"website".as_ptr()),
                value: cstr_obj(c"https://neovim.io".as_ptr()),
            },
            KeyValuePair {
                key: cstr_as_string(c"license".as_ptr()),
                value: cstr_obj(c"Apache 2".as_ptr()),
            },
            KeyValuePair {
                key: cstr_as_string(c"pid".as_ptr()),
                value: int_obj(Integer::from(uv_os_getpid())),
            },
        ];
        let info = Dict {
            items: info_items.as_mut_ptr(),
            size: 3,
            capacity: 3,
        };

        let mut args2_items: [Object; 5] = [
            cstr_obj(c"nvim-tui".as_ptr()),
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: version },
            },
            cstr_obj(c"ui".as_ptr()),
            Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData { array: ARRAY_INIT },
            },
            Object {
                obj_type: ObjectType::kObjectTypeDict,
                data: ObjectData { dict: info },
            },
        ];
        let args2 = Array {
            items: args2_items.as_mut_ptr(),
            size: 5,
            capacity: 5,
        };
        rpc_send_event(
            ui_client_channel_id,
            c"nvim_set_client_info".as_ptr(),
            args2,
        );

        if !time_fd.is_null() {
            time_msg(c"nvim_set_client_info".as_ptr(), ptr::null());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_detach() {
    unsafe {
        rpc_send_event(ui_client_channel_id, c"nvim_ui_detach".as_ptr(), ARRAY_INIT);
        ui_client_attached = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_run() -> ! {
    unsafe {
        tui_start(
            &raw mut tui,
            &raw mut tui_width,
            &raw mut tui_height,
            &raw mut tui_term,
            &raw mut tui_rgb,
        );
        ui_client_attach(tui_width, tui_height, tui_term, tui_rgb);

        if os_env_exists(c"NVIM_LOG_FILE".as_ptr(), true) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_run".as_ptr(),
                164,
                true,
                c"test log message".as_ptr(),
            );
        }

        time_finish();

        loop {
            let events = (*loop_queues((&raw mut main_loop).cast())).events;
            if !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events((&raw mut main_loop).cast(), -1);
                multiqueue_process_events(events);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_stop() {
    unsafe {
        ui_client_attached = false;
        if !tui_is_stopped(tui) {
            tui_stop(tui);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_set_size(width: c_int, height: c_int) {
    unsafe {
        if ui_client_attached {
            let mut args_items: [Object; 2] = [
                int_obj(Integer::from(width)),
                int_obj(Integer::from(height)),
            ];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 2,
                capacity: 2,
            };
            rpc_send_event(ui_client_channel_id, c"nvim_ui_try_resize".as_ptr(), args);
        }
        tui_width = width;
        tui_height = height;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn handle_ui_client_redraw(
    _channel_id: u64,
    _args: Array,
    _arena: *mut Arena,
    error: *mut Error,
) -> Object {
    unsafe {
        api_set_error!(
            error,
            ErrorType::kErrorTypeValidation,
            c"'redraw' cannot be sent as a request".as_ptr(),
        );
        OBJECT_NIL
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_dict2hlattrs(d: Dict, rgb: bool) -> HlAttrs {
    unsafe {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut dict: KeyDict_highlight = zeroed();
        if !api_dict_to_keydict(
            (&raw mut dict).cast(),
            Some(highlight_get_field_cb),
            d,
            &raw mut err,
        ) {
            return HLATTRS_INIT;
        }

        let mut attrs = dict2hlattrs(
            &raw mut dict,
            rgb,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut err,
        );
        if (dict.is_set__highlight_ & (1u64 << KEYSET_OPTIDX_highlight__url)) != 0 {
            attrs.url = tui_add_url(tui, dict.url.data);
        }
        attrs
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_grid_resize(args: Array) {
    unsafe {
        if args.size < 3
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(2)).obj_type != ObjectType::kObjectTypeInteger
        {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_grid_resize".as_ptr(),
                242,
                true,
                c"Error handling ui event 'grid_resize'".as_ptr(),
            );
            return;
        }

        let grid = (*args.items).data.integer;
        let width = (*args.items.add(1)).data.integer;
        let height = (*args.items.add(2)).data.integer;
        tui_grid_resize(tui, grid, width, height);

        if grid_line_buf_size < width as usize {
            xfree(grid_line_buf_char.cast());
            xfree(grid_line_buf_attr.cast());
            grid_line_buf_size = width as usize;
            grid_line_buf_char =
                xmalloc(grid_line_buf_size * size_of::<schar_T>()).cast::<schar_T>();
            grid_line_buf_attr =
                xmalloc(grid_line_buf_size * size_of::<sattr_T>()).cast::<sattr_T>();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_grid_line(_args: Array) {
    std::process::abort()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_raw_line(g: *mut GridLineEvent) {
    unsafe {
        let grid = (*g).args[0];
        let row = (*g).args[1];
        let startcol = (*g).args[2];
        let endcol = Integer::from(startcol) + Integer::from((*g).coloff);
        let clearcol = endcol + Integer::from((*g).clear_width);
        let lineflags = if (*g).wrap { kLineFlagWrap } else { 0 };

        tui_raw_line(
            tui,
            Integer::from(grid),
            Integer::from(row),
            Integer::from(startcol),
            endcol,
            clearcol,
            Integer::from((*g).cur_attr),
            lineflags,
            grid_line_buf_char,
            grid_line_buf_attr,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_connect(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_connect".as_ptr(),
                281,
                true,
                c"Error handling UI event 'connect'".as_ptr(),
            );
            return;
        }

        let s = (*args.items).data.string;
        let server_addr = xmemdupz(s.data.cast(), s.size).cast::<c_char>();
        let mut argv: [*mut c_void; EVENT_HANDLER_MAX_ARGC] =
            [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = server_addr.cast();
        multiqueue_put_event(
            (*loop_queues((&raw mut main_loop).cast())).fast_events,
            Event {
                handler: Some(channel_connect_event),
                argv,
            },
        );
        ui_client_channel_id = u64::MAX;
    }
}

unsafe extern "C" fn channel_connect_event(argv: *mut *mut c_void) {
    unsafe {
        let server_addr = (*argv).cast::<c_char>();
        let mut err: *const c_char = c"".as_ptr();
        let is_tcp = !socket_address_tcp_host_end(server_addr).is_null();
        let on_data: CallbackReader = zeroed();
        let chan = channel_connect(is_tcp, server_addr, true, on_data, 50, &raw mut err);

        if !strequal(err, c"".as_ptr()) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"channel_connect_event".as_ptr(),
                302,
                true,
                c"Cannot connect to server %s: %s".as_ptr(),
                server_addr,
                err,
            );
            xfree(server_addr.cast());
            ui_client_exit_status = 1;
            os_exit(1);
        }

        ui_client_channel_id = chan;
        ui_client_attach(tui_width, tui_height, tui_term, tui_rgb);

        logmsg!(
            LOGLVL_INF,
            ptr::null(),
            c"channel_connect_event".as_ptr(),
            312,
            true,
            c"Connected to server %s on channel %lld".as_ptr(),
            server_addr,
            chan as i64,
        );
        xfree(server_addr.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_restart(args: Array) {
    unsafe {
        api_free_array(restart_args);
        restart_args = copy_array(args, ptr::null_mut());
        restart_pending = true;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event__set_restart_on_crash_exit(args: Array) {
    unsafe {
        api_free_array(restart_args_after_crash_exit);
        restart_args_after_crash_exit = copy_array(args, ptr::null_mut());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_attach_to_restarted_server(error_restart: bool) {
    unsafe {
        let mut args = restart_args;
        let mut start_server = false;
        if !restart_pending {
            if error_restart && ui_client_error_exit == -1 && restart_args_after_crash_exit.size > 0
            {
                start_server = true;
                args = restart_args_after_crash_exit;
            } else {
                return;
            }
        }

        restart_pending = false;

        let cleanup = || {
            api_free_array(restart_args);
            restart_args = ARRAY_INIT;
            api_free_array(restart_args_after_crash_exit);
            restart_args_after_crash_exit = ARRAY_INIT;
        };

        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_attach_to_restarted_server".as_ptr(),
                362,
                true,
                c"Error handling ui event 'restart'".as_ptr(),
            );
            cleanup();
            return;
        }

        let first_arg = (*args.items).data.string.data;
        let chan_id = if start_server {
            if args.size < 2 || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeArray {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"ui_client_attach_to_restarted_server".as_ptr(),
                    370,
                    true,
                    c"Error handling ui event 'restart'".as_ptr(),
                );
                cleanup();
                return;
            }
            let cmdargs = (*args.items.add(1)).data.array;
            let argv = xcalloc(cmdargs.size + 1, size_of::<*mut c_char>()).cast::<*mut c_char>();
            for i in 0..cmdargs.size {
                let item = *cmdargs.items.add(i);
                if item.obj_type == ObjectType::kObjectTypeString {
                    *argv.add(i) = item.data.string.data;
                }
                if (*argv.add(i)).is_null() {
                    *argv.add(i) = c"".as_ptr().cast_mut();
                }
            }
            let id = ui_client_start_server(first_arg, cmdargs.size, argv);
            xfree(argv.cast());
            ui_client_error_exit = -1;
            id
        } else {
            let is_tcp = !socket_address_tcp_host_end(first_arg).is_null();
            let mut err: *const c_char = ptr::null();
            let on_data: CallbackReader = zeroed();
            let id = channel_connect(is_tcp, first_arg, true, on_data, 50, &raw mut err);
            if !err.is_null() {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"ui_client_attach_to_restarted_server".as_ptr(),
                    391,
                    true,
                    c"cannot connect to server %s: %s".as_ptr(),
                    first_arg,
                    err,
                );
                cleanup();
                return;
            }
            if args.size > 1 && (*args.items.add(1)).obj_type == ObjectType::kObjectTypeDict {
                let mut chan_args_items: [Object; 2] = [int_obj(0), *args.items.add(1)];
                let chan_args = Array {
                    items: chan_args_items.as_mut_ptr(),
                    size: 2,
                    capacity: 2,
                };
                rpc_send_event(id, c"nvim_chan_set".as_ptr(), chan_args);
            }
            id
        };

        ui_client_channel_id = chan_id;
        ui_client_attach(tui_width, tui_height, tui_term, tui_rgb);

        logmsg!(
            LOGLVL_INF,
            ptr::null(),
            c"ui_client_attach_to_restarted_server".as_ptr(),
            406,
            true,
            c"restarted server address=%s id=%lld".as_ptr(),
            first_arg,
            chan_id as i64,
        );
        cleanup();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_error_exit(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeInteger {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"ui_client_event_error_exit".as_ptr(),
                419,
                true,
                c"Error handling ui event 'error_exit'".as_ptr(),
            );
            return;
        }
        ui_client_error_exit = (*args.items).data.integer as c_int;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_free_all_mem() {
    unsafe {
        tui_free_all_mem(tui);
        xfree(grid_line_buf_char.cast());
        xfree(grid_line_buf_attr.cast());
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct UIClientHandler {
    pub name: *const c_char,
    pub r#fn: Option<unsafe extern "C" fn(args: Array)>,
}

unsafe fn log_ui_event_err(func_name: *const c_char, ev_name: *const c_char) {
    unsafe {
        logmsg!(
            LOGLVL_ERR,
            ptr::null(),
            func_name,
            0,
            true,
            c"Error handling ui event '%s'".as_ptr(),
            ev_name,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_mode_info_set(args: Array) {
    unsafe {
        if args.size < 2
            || (*args.items).obj_type != ObjectType::kObjectTypeBoolean
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeArray
        {
            log_ui_event_err(
                c"ui_client_event_mode_info_set".as_ptr(),
                c"mode_info_set".as_ptr(),
            );
            return;
        }
        tui_mode_info_set(
            tui,
            (*args.items).data.boolean,
            (*args.items.add(1)).data.array,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_update_menu(_args: Array) {
    unsafe {
        tui_update_menu(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_busy_start(_args: Array) {
    unsafe {
        tui_busy_start(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_busy_stop(_args: Array) {
    unsafe {
        tui_busy_stop(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_mouse_on(_args: Array) {
    unsafe {
        tui_mouse_on(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_mouse_off(_args: Array) {
    unsafe {
        tui_mouse_off(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_mode_change(args: Array) {
    unsafe {
        if args.size < 2
            || (*args.items).obj_type != ObjectType::kObjectTypeString
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_mode_change".as_ptr(),
                c"mode_change".as_ptr(),
            );
            return;
        }
        tui_mode_change(
            tui,
            (*args.items).data.string,
            (*args.items.add(1)).data.integer,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_bell(_args: Array) {
    unsafe {
        tui_bell(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_visual_bell(_args: Array) {
    unsafe {
        tui_visual_bell(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_flush(_args: Array) {
    unsafe {
        tui_flush(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_suspend(_args: Array) {
    unsafe {
        tui_suspend(tui);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_set_title(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_set_title".as_ptr(), c"set_title".as_ptr());
            return;
        }
        tui_set_title(tui, (*args.items).data.string);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_set_icon(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_set_icon".as_ptr(), c"set_icon".as_ptr());
            return;
        }
        tui_set_icon(tui, (*args.items).data.string);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_screenshot(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(
                c"ui_client_event_screenshot".as_ptr(),
                c"screenshot".as_ptr(),
            );
            return;
        }
        tui_screenshot(tui, (*args.items).data.string);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_option_set(args: Array) {
    unsafe {
        if args.size < 2 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(
                c"ui_client_event_option_set".as_ptr(),
                c"option_set".as_ptr(),
            );
            return;
        }
        tui_option_set(tui, (*args.items).data.string, *args.items.add(1));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_chdir(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_chdir".as_ptr(), c"chdir".as_ptr());
            return;
        }
        tui_chdir(tui, (*args.items).data.string);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_ui_send(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeString {
            log_ui_event_err(c"ui_client_event_ui_send".as_ptr(), c"ui_send".as_ptr());
            return;
        }
        tui_ui_send(tui, (*args.items).data.string);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_default_colors_set(args: Array) {
    unsafe {
        if args.size < 5
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(2)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(3)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(4)).obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_default_colors_set".as_ptr(),
                c"default_colors_set".as_ptr(),
            );
            return;
        }
        tui_default_colors_set(
            tui,
            (*args.items).data.integer,
            (*args.items.add(1)).data.integer,
            (*args.items.add(2)).data.integer,
            (*args.items.add(3)).data.integer,
            (*args.items.add(4)).data.integer,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_hl_attr_define(args: Array) {
    unsafe {
        if args.size < 4
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeDict
            || (*args.items.add(2)).obj_type != ObjectType::kObjectTypeDict
            || (*args.items.add(3)).obj_type != ObjectType::kObjectTypeArray
        {
            log_ui_event_err(
                c"ui_client_event_hl_attr_define".as_ptr(),
                c"hl_attr_define".as_ptr(),
            );
            return;
        }
        let arg_1 = (*args.items).data.integer;
        let arg_2 = ui_client_dict2hlattrs((*args.items.add(1)).data.dict, true);
        let arg_3 = ui_client_dict2hlattrs((*args.items.add(2)).data.dict, false);
        let arg_4 = (*args.items.add(3)).data.array;
        tui_hl_attr_define(tui, arg_1, arg_2, arg_3, arg_4);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_grid_clear(args: Array) {
    unsafe {
        if args.size < 1 || (*args.items).obj_type != ObjectType::kObjectTypeInteger {
            log_ui_event_err(
                c"ui_client_event_grid_clear".as_ptr(),
                c"grid_clear".as_ptr(),
            );
            return;
        }
        tui_grid_clear(tui, (*args.items).data.integer);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_grid_cursor_goto(args: Array) {
    unsafe {
        if args.size < 3
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(2)).obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_grid_cursor_goto".as_ptr(),
                c"grid_cursor_goto".as_ptr(),
            );
            return;
        }
        tui_grid_cursor_goto(
            tui,
            (*args.items).data.integer,
            (*args.items.add(1)).data.integer,
            (*args.items.add(2)).data.integer,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_event_grid_scroll(args: Array) {
    unsafe {
        if args.size < 7
            || (*args.items).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(2)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(3)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(4)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(5)).obj_type != ObjectType::kObjectTypeInteger
            || (*args.items.add(6)).obj_type != ObjectType::kObjectTypeInteger
        {
            log_ui_event_err(
                c"ui_client_event_grid_scroll".as_ptr(),
                c"grid_scroll".as_ptr(),
            );
            return;
        }
        tui_grid_scroll(
            tui,
            (*args.items).data.integer,
            (*args.items.add(1)).data.integer,
            (*args.items.add(2)).data.integer,
            (*args.items.add(3)).data.integer,
            (*args.items.add(4)).data.integer,
            (*args.items.add(5)).data.integer,
            (*args.items.add(6)).data.integer,
        );
    }
}

const EVENT_HANDLERS: [UIClientHandler; 28] = [
    UIClientHandler {
        name: c"bell".as_ptr(),
        r#fn: Some(ui_client_event_bell),
    },
    UIClientHandler {
        name: c"chdir".as_ptr(),
        r#fn: Some(ui_client_event_chdir),
    },
    UIClientHandler {
        name: c"flush".as_ptr(),
        r#fn: Some(ui_client_event_flush),
    },
    UIClientHandler {
        name: c"connect".as_ptr(),
        r#fn: Some(ui_client_event_connect),
    },
    UIClientHandler {
        name: c"restart".as_ptr(),
        r#fn: Some(ui_client_event_restart),
    },
    UIClientHandler {
        name: c"suspend".as_ptr(),
        r#fn: Some(ui_client_event_suspend),
    },
    UIClientHandler {
        name: c"ui_send".as_ptr(),
        r#fn: Some(ui_client_event_ui_send),
    },
    UIClientHandler {
        name: c"mouse_on".as_ptr(),
        r#fn: Some(ui_client_event_mouse_on),
    },
    UIClientHandler {
        name: c"set_icon".as_ptr(),
        r#fn: Some(ui_client_event_set_icon),
    },
    UIClientHandler {
        name: c"busy_stop".as_ptr(),
        r#fn: Some(ui_client_event_busy_stop),
    },
    UIClientHandler {
        name: c"grid_line".as_ptr(),
        r#fn: Some(ui_client_event_grid_line),
    },
    UIClientHandler {
        name: c"mouse_off".as_ptr(),
        r#fn: Some(ui_client_event_mouse_off),
    },
    UIClientHandler {
        name: c"set_title".as_ptr(),
        r#fn: Some(ui_client_event_set_title),
    },
    UIClientHandler {
        name: c"busy_start".as_ptr(),
        r#fn: Some(ui_client_event_busy_start),
    },
    UIClientHandler {
        name: c"error_exit".as_ptr(),
        r#fn: Some(ui_client_event_error_exit),
    },
    UIClientHandler {
        name: c"grid_clear".as_ptr(),
        r#fn: Some(ui_client_event_grid_clear),
    },
    UIClientHandler {
        name: c"option_set".as_ptr(),
        r#fn: Some(ui_client_event_option_set),
    },
    UIClientHandler {
        name: c"screenshot".as_ptr(),
        r#fn: Some(ui_client_event_screenshot),
    },
    UIClientHandler {
        name: c"mode_change".as_ptr(),
        r#fn: Some(ui_client_event_mode_change),
    },
    UIClientHandler {
        name: c"update_menu".as_ptr(),
        r#fn: Some(ui_client_event_update_menu),
    },
    UIClientHandler {
        name: c"visual_bell".as_ptr(),
        r#fn: Some(ui_client_event_visual_bell),
    },
    UIClientHandler {
        name: c"grid_resize".as_ptr(),
        r#fn: Some(ui_client_event_grid_resize),
    },
    UIClientHandler {
        name: c"grid_scroll".as_ptr(),
        r#fn: Some(ui_client_event_grid_scroll),
    },
    UIClientHandler {
        name: c"mode_info_set".as_ptr(),
        r#fn: Some(ui_client_event_mode_info_set),
    },
    UIClientHandler {
        name: c"hl_attr_define".as_ptr(),
        r#fn: Some(ui_client_event_hl_attr_define),
    },
    UIClientHandler {
        name: c"grid_cursor_goto".as_ptr(),
        r#fn: Some(ui_client_event_grid_cursor_goto),
    },
    UIClientHandler {
        name: c"default_colors_set".as_ptr(),
        r#fn: Some(ui_client_event_default_colors_set),
    },
    UIClientHandler {
        name: c"_set_restart_on_crash_exit".as_ptr(),
        r#fn: Some(ui_client_event__set_restart_on_crash_exit),
    },
];

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_handler_hash(str_in: *const c_char, len: usize) -> c_int {
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
    let target =
        unsafe { std::slice::from_raw_parts(EVENT_HANDLERS[low as usize].name.cast::<u8>(), len) };
    if bytes != target {
        return -1;
    }
    low as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ui_client_get_redraw_handler(
    name: *const c_char,
    name_len: usize,
    _error: *mut Error,
) -> UIClientHandler {
    unsafe {
        let hash = ui_client_handler_hash(name, name_len);
        if hash < 0 {
            return UIClientHandler {
                name: ptr::null(),
                r#fn: None,
            };
        }
        EVENT_HANDLERS[hash as usize]
    }
}
