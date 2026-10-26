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
use crate::memory::{cstr_take, strequal, xstrdup, xstrlcpy};
use crate::os::env::{os_env_exists, os_get_pid, os_getenv_cstr, os_unsetenv};
use crate::os::stdpaths::get_appname;
use crate::path::{fix_fname, to_slash, to_slash_save};
use crate::strings::{strcmp, strlen, strstr, vim_snprintf};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

#[cfg(not(windows))]
use crate::os::stdpaths::{XDGVarType, stdpaths_get_xdg_var};

const MAX_CONNECTIONS: c_int = 32;

const ENV_LISTEN: &CStr = c"NVIM_LISTEN_ADDRESS";
const ENV_TEST_LOG: &CStr = c"__NVIM_TEST_LOG";

use crate::event::stream::UV_TCP;
use crate::globals::IOSIZE;

static mut watchers: Vec<*mut SocketWatcher> = Vec::new();
static mut server_addr_count: u32 = 0;

#[inline]
unsafe fn watchers_mut() -> &'static mut Vec<*mut SocketWatcher> {
    let ptr = &raw mut watchers;
    unsafe { &mut *ptr }
}

pub unsafe fn server_init(mut listen_addr: *const c_char) -> bool {
    unsafe {
        let mut ok = true;
        let mut owned_addr: Option<std::ffi::CString> = None;
        let mut user_arg: TriState = kTrue;
        watchers = Vec::new();

        if listen_addr.is_null() || *listen_addr == 0 {
            if os_env_exists(ENV_LISTEN.as_ptr(), true) {
                user_arg = kFalse;
                let raw = os_getenv_cstr(ENV_LISTEN.as_ptr());
                owned_addr = cstr_take(to_slash_save(
                    raw.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                ));
            } else {
                user_arg = kNone;
                owned_addr = Some(server_address_new(ptr::null()));
            }
            listen_addr = owned_addr.as_ref().unwrap().as_ptr();
        }

        let rv = server_start(listen_addr);

        if os_env_exists(ENV_TEST_LOG.as_ptr(), false) {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"server_init".as_ptr(),
                58,
                true,
                c"test log message".as_ptr(),
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
                ptr::addr_of_mut!(IObuff).cast(),
                IOSIZE,
                fmt,
                reason,
                listen_addr,
            );
            ok = false;
        }

        if os_env_exists(ENV_LISTEN.as_ptr(), false) {
            os_unsetenv(ENV_LISTEN.as_ptr());
        }

        drop(owned_addr);

        ok
    }
}

unsafe fn set_vservername(srvs: &[*mut SocketWatcher]) {
    unsafe {
        let default_server = srvs
            .first()
            .map_or(ptr::null(), |&first| (*first).addr.as_ptr());
        set_vim_var_string(VV_SEND_SERVER, default_server, -1);
    }
}

pub unsafe fn server_teardown() {
    unsafe {
        for &watcher in watchers_mut().iter() {
            socket_watcher_close(watcher, Some(free_server));
        }
        watchers = Vec::new();
    }
}

pub unsafe fn server_address_new(name: *const c_char) -> std::ffi::CString {
    unsafe {
        let mut fmt: [c_char; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];
        let cur_count = server_addr_count;
        server_addr_count = server_addr_count.wrapping_add(1);

        #[cfg(windows)]
        let r = {
            get_appname(true);
            let app = if !name.is_null() {
                name
            } else {
                ptr::addr_of!(NameBuff).cast()
            };
            vim_snprintf!(
                fmt.as_mut_ptr(),
                fmt.len(),
                c"//./pipe/%s.%llu.%u".as_ptr(),
                app,
                os_get_pid() as u64,
                cur_count,
            )
        };

        #[cfg(not(windows))]
        let r = {
            let dir = cstr_take(stdpaths_get_xdg_var(XDGVarType::kXDGRuntimeDir));
            get_appname(true);
            let app = if !name.is_null() {
                name
            } else {
                ptr::addr_of!(NameBuff).cast()
            };
            vim_snprintf!(
                fmt.as_mut_ptr(),
                fmt.len(),
                c"%s/%s.%llu.%u".as_ptr(),
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
                c"server_address_new".as_ptr(),
                133,
                true,
                c"truncated server address: %.40s...".as_ptr(),
                fmt.as_ptr(),
            );
        }
        std::ffi::CStr::from_ptr(fmt.as_ptr()).to_owned()
    }
}

pub unsafe fn server_owns_pipe_address(address: *const c_char) -> bool {
    unsafe {
        let path = cstr_take(fix_fname(address));
        for &watcher in watchers_mut().iter() {
            let addr = cstr_take(fix_fname((*watcher).addr.as_ptr()));
            if path == addr {
                return true;
            }
        }
        false
    }
}

pub unsafe fn server_start(addr: *const c_char) -> c_int {
    unsafe {
        if addr.is_null() || *addr == 0 {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_start".as_ptr(),
                167,
                true,
                c"Empty or NULL address".as_ptr(),
            );
            return 1;
        }

        let isname = strstr(addr, c":".as_ptr()).is_null()
            && strstr(addr, c"/".as_ptr()).is_null()
            && strstr(addr, c"\\".as_ptr()).is_null();
        let addr_gen = if isname {
            Some(server_address_new(addr))
        } else {
            None
        };
        let watcher = Box::into_raw(Box::new(SocketWatcher::new()));
        let endpoint = addr_gen.as_ref().map_or(addr, |s| s.as_ptr());
        let mut result =
            socket_watcher_init(ptr::addr_of_mut!(main_loop).cast(), watcher, endpoint);
        drop(addr_gen);
        if result < 0 {
            let _ = Box::from_raw(watcher);
            return result;
        }

        let w = watchers_mut();
        for &existing in w.iter() {
            if strcmp((*watcher).addr.as_ptr(), (*existing).addr.as_ptr()) == 0 {
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"server_start".as_ptr(),
                    184,
                    true,
                    c"Already listening on %s".as_ptr(),
                    (*watcher).addr.as_ptr(),
                );
                if uv_handle_get_type((*watcher).stream.cast()) == UV_TCP {
                    uv_freeaddrinfo((*watcher).tcp_addrinfo.cast());
                }
                socket_watcher_close(watcher, Some(free_server));
                return 2;
            }
        }

        result = socket_watcher_start(watcher, MAX_CONNECTIONS, Some(connection_cb));
        if result < 0 {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_start".as_ptr(),
                195,
                true,
                c"Failed to start server: %s: %s".as_ptr(),
                uv_strerror(result),
                (*watcher).addr.as_ptr(),
            );
            socket_watcher_close(watcher, Some(free_server));
            return result;
        }

        w.push(watcher);

        if strlen(get_vim_var_str(VV_SEND_SERVER)) == 0 {
            set_vservername(w);
        }

        0
    }
}

pub unsafe fn server_stop(endpoint: *const c_char, keep_vservername: bool) -> bool {
    unsafe {
        let mut addr: [c_char; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];

        xstrlcpy(addr.as_mut_ptr(), endpoint, addr.len());
        to_slash(addr.as_mut_ptr());

        let w = watchers_mut();
        let Some(idx) = w
            .iter()
            .position(|&watcher| strcmp(addr.as_ptr(), (*watcher).addr.as_ptr()) == 0)
        else {
            logmsg!(
                LOGLVL_WRN,
                ptr::null(),
                c"server_stop".as_ptr(),
                238,
                true,
                c"Not listening on %s".as_ptr(),
                addr.as_ptr(),
            );
            return false;
        };

        let watcher = w.swap_remove(idx);
        socket_watcher_close(watcher, Some(free_server));

        if !keep_vservername && strequal(addr.as_ptr(), get_vim_var_str(VV_SEND_SERVER)) {
            set_vservername(w);
        }

        true
    }
}

pub unsafe fn server_address_list() -> Vec<*mut c_char> {
    unsafe {
        watchers_mut()
            .iter()
            .map(|&watcher| xstrdup((*watcher).addr.as_ptr()))
            .collect()
    }
}

unsafe fn connection_cb(watcher: *mut SocketWatcher, result: c_int) {
    unsafe {
        if result != 0 {
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"connection_cb".as_ptr(),
                276,
                true,
                c"Failed to accept connection: %s".as_ptr(),
                uv_strerror(result),
            );
            return;
        }

        channel_from_connection(watcher);
    }
}

unsafe fn free_server(watcher: *mut SocketWatcher) {
    unsafe {
        let _ = Box::from_raw(watcher);
    }
}
