//! Msgpack-RPC server address and socket watcher management (`src/nvim/msgpack_rpc/server.c`).

use crate::channel::channel_from_connection;
use crate::eval::vars::{VV_SEND_SERVER, get_vim_var_str, set_vim_var_string};
use crate::event::r#loop::{uv_freeaddrinfo, uv_handle_get_type, uv_strerror};
use crate::event::socket::{
    ADDRESS_MAX_SIZE, SocketWatcher, socket_watcher_close, socket_watcher_init,
    socket_watcher_start,
};
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::globals::*;
use crate::log::{LOGLVL_ERR, LOGLVL_WRN, logmsg};
use crate::memory::{cstr_to_cstring, xstrlcpy};
use crate::os::env::{os_env_exists, os_get_pid, os_getenv_cstr, os_unsetenv};
use crate::os::stdpaths::get_appname;
use crate::path::{fix_fname, to_slash, to_slash_save};
use crate::strings::{vim_snprintf};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

#[cfg(not(windows))]
use crate::os::stdpaths::{XDGVarType, stdpaths_get_xdg_var};

const MAX_CONNECTIONS: c_int = 32;

const ENV_LISTEN: &CStr = c"NVIM_LISTEN_ADDRESS";
const ENV_TEST_LOG: &CStr = c"__NVIM_TEST_LOG";

use crate::event::stream::UV_TCP;
use crate::globals::IOSIZE;

struct RpcServerState {
    watchers: Vec<*mut SocketWatcher>,
    server_addr_count: u32,
}

impl RpcServerState {
    const fn new() -> Self {
        Self {
            watchers: Vec::new(),
            server_addr_count: 0,
        }
    }
}

static SERVER_STATE: crate::globals::GlobalState<RpcServerState> = crate::globals::GlobalState::new(RpcServerState::new());

#[inline]
fn srv_st() -> &'static mut RpcServerState {
    unsafe { &mut *SERVER_STATE.as_ptr() }
}

#[inline]
fn watchers_mut() -> &'static mut Vec<*mut SocketWatcher> {
    &mut srv_st().watchers
}

pub fn server_init(mut listen_addr: *const c_char) -> bool {
    unsafe {
        let mut ok = true;
        let mut owned_addr: Option<std::ffi::CString> = None;
        let mut user_arg: TriState = kTrue;
        srv_st().watchers = Vec::new();

        if listen_addr.is_null() || *listen_addr == 0 {
            if os_env_exists(ENV_LISTEN, true) {
                user_arg = kFalse;
                let raw = os_getenv_cstr(ENV_LISTEN);
                owned_addr = to_slash_save(raw.as_ref().map_or(ptr::null(), |c| c.as_ptr()));
            } else {
                user_arg = kNone;
                owned_addr = Some(server_address_new(ptr::null()));
            }
            listen_addr = owned_addr.as_ref().unwrap().as_ptr();
        }

        let rv = server_start(listen_addr);

        if os_env_exists(ENV_TEST_LOG, false) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"server_init",
                58,
                true,
                c"test log message",
            );
        }

        if rv != 0 && user_arg != kNone {
            let fmt = if user_arg == kTrue {
                c"Failed to --listen: %s: \"%s\"".as_ptr()
            } else {
                c"Failed $NVIM_LISTEN_ADDRESS: %s: \"%s\"".as_ptr()
            };
            let reason = if rv < 0 {
                uv_strerror(rv)
            } else if rv == 1 {
                c"empty address".as_ptr()
            } else {
                c"?".as_ptr()
            };
            vim_snprintf!(
                ptr::addr_of_mut!(g_st().IObuff).cast(),
                IOSIZE,
                fmt,
                reason,
                listen_addr,
            );
            ok = false;
        }

        if os_env_exists(ENV_LISTEN, false) {
            os_unsetenv(ENV_LISTEN.as_ptr());
        }

        drop(owned_addr);

        ok
    }
}

fn set_vservername(srvs: &[*mut SocketWatcher]) {
    unsafe {
        let default_server = srvs
            .first()
            .map_or(ptr::null(), |&first| (*first).addr.as_ptr());
        set_vim_var_string(VV_SEND_SERVER, default_server, -1);
    }
}

pub fn server_teardown() {
    unsafe {
        for &watcher in watchers_mut().iter() {
            socket_watcher_close(&mut *watcher);
        }
        srv_st().watchers = Vec::new();
    }
}

pub fn server_address_new(name: *const c_char) -> std::ffi::CString {
     {
        let mut fmt: [c_char; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];
        let cur_count = srv_st().server_addr_count;
        srv_st().server_addr_count = srv_st().server_addr_count.wrapping_add(1);

        #[cfg(windows)]
        let r = {
            get_appname(true);
            let app = if !name.is_null() {
                name
            } else {
                ptr::addr_of!(g_st().NameBuff).cast()
            };
            vim_snprintf!(
                fmt.as_mut_ptr(),
                fmt.len(),
                c"//./pipe/%s.%llu.%u",
                app,
                os_get_pid() as u64,
                cur_count,
            )
        };

        #[cfg(not(windows))]
        let r = {
            let dir = stdpaths_get_xdg_var(XDGVarType::kXDGRuntimeDir);
            get_appname(true);
            let app = if !name.is_null() {
                name
            } else {
                ptr::addr_of!(g_st().NameBuff).cast()
            };
            vim_snprintf!(
                fmt.as_mut_ptr(),
                fmt.len(),
                c"%s/%s.%llu.%u",
                dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                app,
                os_get_pid() as u64,
                cur_count,
            )
        };

        if (r as usize) >= fmt.len() {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"server_address_new",
                133,
                true,
                c"truncated server address: %.40s...",
                fmt.as_ptr(),
            );
        }
        crate::memory::cstr_to_cstring(fmt.as_ptr())
    }
}

pub fn server_owns_pipe_address(address: *const c_char) -> bool {
    unsafe {
        let path = fix_fname(address);
        for &watcher in watchers_mut().iter() {
            let addr = fix_fname((*watcher).addr.as_ptr());
            if path == addr {
                return true;
            }
        }
        false
    }
}

pub fn server_start(addr: *const c_char) -> c_int {
    unsafe {
        if addr.is_null() || *addr == 0 {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_start",
                167,
                true,
                c"Empty or NULL address",
            );
            return 1;
        }

        let isname = !crate::memory::cstr_as_bytes(addr)
            .iter()
            .any(|b| matches!(b, b':' | b'/' | b'\\'));
        let addr_gen = if isname {
            Some(server_address_new(addr))
        } else {
            None
        };
        let watcher = Box::into_raw(Box::new(SocketWatcher::new()));
        let endpoint = addr_gen.as_ref().map_or(addr, |s| s.as_ptr());
        let mut result =
            socket_watcher_init(&mut *ptr::addr_of_mut!(g_st().main_loop).cast(), watcher, endpoint);
        drop(addr_gen);
        if result < 0 {
            let _ = Box::from_raw(watcher);
            return result;
        }

        let w = watchers_mut();
        for &existing in w.iter() {
            if crate::memory::cstr_as_bytes((*watcher).addr.as_ptr()) == crate::memory::cstr_as_bytes((*existing).addr.as_ptr()) {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"server_start",
                    184,
                    true,
                    c"Already listening on %s",
                    (*watcher).addr.as_ptr(),
                );
                if uv_handle_get_type((*watcher).stream_ptr().cast()) == UV_TCP {
                    uv_freeaddrinfo((*watcher).tcp_addrinfo.cast());
                }
                socket_watcher_close(&mut *watcher);
                return 2;
            }
        }

        result = socket_watcher_start(watcher, MAX_CONNECTIONS);
        if result < 0 {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_start",
                195,
                true,
                c"Failed to start server: %s: %s",
                uv_strerror(result),
                (*watcher).addr.as_ptr(),
            );
            socket_watcher_close(&mut *watcher);
            return result;
        }

        w.push(watcher);

        if crate::memory::cstr_as_bytes(get_vim_var_str(VV_SEND_SERVER)).is_empty() {
            set_vservername(w);
        }

        0
    }
}

pub fn server_stop(endpoint: *const c_char, keep_vservername: bool) -> bool {
    unsafe {
        let mut addr: [c_char; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];

        xstrlcpy(addr.as_mut_ptr(), endpoint, addr.len());
        to_slash(addr.as_mut_ptr());

        let w = watchers_mut();
        let Some(idx) = w
            .iter()
            .position(|&watcher| crate::memory::cstr_as_bytes(addr.as_ptr()) == crate::memory::cstr_as_bytes((*watcher).addr.as_ptr()))
        else {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_stop",
                238,
                true,
                c"Not listening on %s",
                addr.as_ptr(),
            );
            return false;
        };

        let watcher = w.swap_remove(idx);
        socket_watcher_close(&mut *watcher);

        if !keep_vservername && (crate::memory::cstr_as_opt_bytes(addr.as_ptr()) == crate::memory::cstr_as_opt_bytes(get_vim_var_str(VV_SEND_SERVER))) {
            set_vservername(w);
        }

        true
    }
}

pub fn server_address_list() -> Vec<std::ffi::CString> {
    unsafe {
        watchers_mut()
            .iter()
            .map(|&watcher| cstr_to_cstring((*watcher).addr.as_ptr()))
            .collect()
    }
}

pub(crate) fn connection_cb(watcher: *mut SocketWatcher, result: c_int) {
    unsafe {
        if result != 0 {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"connection_cb",
                276,
                true,
                c"Failed to accept connection: %s",
                uv_strerror(result),
            );
            return;
        }

        channel_from_connection(watcher);
    }
}
