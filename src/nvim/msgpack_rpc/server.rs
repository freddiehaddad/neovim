#![forbid(unsafe_code)]

//! Msgpack-RPC server address and socket watcher management (`src/nvim/msgpack_rpc/server.c`).

use crate::channel::channel_from_connection;
use crate::eval::vars::{VV_SEND_SERVER, get_vim_var_str, set_vim_var_string};
use crate::event::r#loop::uv_strerror;
use crate::event::socket::{
    ADDRESS_MAX_SIZE, SocketWatcher, socket_watcher_close, socket_watcher_init,
    socket_watcher_start,
};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::globals::*;
use crate::log::{LOGLVL_ERR, LOGLVL_WRN, logmsg};
use crate::os::env::{os_env_exists, os_get_pid, os_getenv_cstr, os_unsetenv};
use crate::os::stdpaths::get_appname;
use crate::path::{fix_fname, to_slash, to_slash_save};
use crate::strings::format_to_buf;
use std::ffi::CStr;

#[cfg(not(windows))]
use crate::os::stdpaths::{XDGVarType, stdpaths_get_xdg_var};

const MAX_CONNECTIONS: i32 = 32;

const ENV_LISTEN: &CStr = c"NVIM_LISTEN_ADDRESS";
const ENV_TEST_LOG: &CStr = c"__NVIM_TEST_LOG";

struct RpcServerState {
    #[expect(clippy::vec_box)]
    watchers: std::cell::RefCell<Vec<Box<SocketWatcher>>>,
    server_addr_count: std::cell::Cell<u32>,
}

impl RpcServerState {
    const fn new() -> Self {
        Self {
            watchers: std::cell::RefCell::new(Vec::new()),
            server_addr_count: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static SERVER_STATE: RpcServerState = const { RpcServerState::new() };
}

pub fn server_init(listen_addr: Option<&CStr>) -> bool {
    let mut ok = true;
    let mut owned_addr: Option<std::ffi::CString> = None;
    let mut user_arg: TriState = K_TRUE;
    SERVER_STATE.with(|st| st.watchers.borrow_mut().clear());

    if listen_addr.is_none_or(|s| s.to_bytes().is_empty()) {
        if os_env_exists(ENV_LISTEN, true) {
            user_arg = K_FALSE;
            let raw = os_getenv_cstr(ENV_LISTEN);
            owned_addr = to_slash_save(raw.as_deref());
        } else {
            user_arg = K_NONE;
            owned_addr = Some(server_address_new(None));
        }
    }
    let listen_addr = listen_addr
        .filter(|s| !s.to_bytes().is_empty())
        .or(owned_addr.as_deref());

    let rv = server_start(listen_addr);

    if os_env_exists(ENV_TEST_LOG, false) {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"server_init"),
            58,
            true,
            c"test log message",
        );
    }

    if rv != 0 && user_arg != K_NONE {
        let fmt = if user_arg == K_TRUE {
            c"Failed to --listen: %s: \"%s\""
        } else {
            c"Failed $NVIM_LISTEN_ADDRESS: %s: \"%s\""
        };
        let reason = if rv < 0 {
            uv_strerror(rv)
        } else if rv == 1 {
            c"empty address"
        } else {
            c"?"
        };
        format_to_buf!(&mut g_st().iobuff, fmt, reason, listen_addr);
        ok = false;
    }

    if os_env_exists(ENV_LISTEN, false) {
        os_unsetenv(ENV_LISTEN);
    }

    ok
}

fn set_vservername() {
    let default_server = SERVER_STATE.with(|st| {
        st.watchers
            .borrow()
            .first()
            .map(|first| first.addr_cstr().to_owned())
    });
    set_vim_var_string(VV_SEND_SERVER, default_server.as_deref(), -1);
}

pub fn server_teardown() {
    let watchers = SERVER_STATE.with(|st| std::mem::take(&mut *st.watchers.borrow_mut()));
    for watcher in watchers {
        socket_watcher_close(Box::leak(watcher));
    }
}

pub fn server_address_new(name: Option<&CStr>) -> std::ffi::CString {
    let mut fmt: [u8; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];
    let cur_count = SERVER_STATE.with(|st| {
        let c = st.server_addr_count.get();
        st.server_addr_count.set(c.wrapping_add(1));
        c
    });

    #[cfg(windows)]
    let r = {
        get_appname(true);
        let app = name.unwrap_or_else(|| g_st().name_buff_cstr());
        format_to_buf!(
            &mut fmt,
            c"//./pipe/%s.%llu.%u",
            app,
            os_get_pid() as u64,
            cur_count,
        )
    };

    #[cfg(not(windows))]
    let r = {
        let dir = stdpaths_get_xdg_var(XDGVarType::XDGRuntimeDir);
        get_appname(true);
        let name_buff = &g_st().name_buff;
        let app = name.unwrap_or_else(|| crate::memory::bytes_as_cstr(name_buff));
        format_to_buf!(
            &mut fmt,
            c"%s/%s.%llu.%u",
            dir.as_deref(),
            app,
            os_get_pid() as u64,
            cur_count,
        )
    };

    let addr_cstr = CStr::from_bytes_until_nul(&fmt).unwrap_or(c"");
    if (r as usize) >= fmt.len() {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"server_address_new"),
            133,
            true,
            c"truncated server address: %.40s...",
            addr_cstr,
        );
    }
    addr_cstr.to_owned()
}

pub fn server_owns_pipe_address(address: &CStr) -> bool {
    let path = fix_fname(Some(address));
    let addrs: Vec<std::ffi::CString> = SERVER_STATE.with(|st| {
        st.watchers
            .borrow()
            .iter()
            .map(|w| w.addr_cstr().to_owned())
            .collect()
    });
    for watcher_addr in addrs {
        let addr = fix_fname(Some(&watcher_addr));
        if path == addr {
            return true;
        }
    }
    false
}

pub fn server_start(addr: Option<&CStr>) -> i32 {
    let Some(addr) = addr.filter(|s| !s.to_bytes().is_empty()) else {
        logmsg!(
            LOGLVL_WRN,
            None,
            Some(c"server_start"),
            167,
            true,
            c"Empty or NULL address",
        );
        return 1;
    };

    let isname = !addr
        .to_bytes()
        .iter()
        .any(|b| matches!(b, b':' | b'/' | b'\\'));
    let addr_gen = if isname {
        Some(server_address_new(Some(addr)))
    } else {
        None
    };
    let mut watcher = Box::new(SocketWatcher::new());
    let endpoint = addr_gen.as_deref().unwrap_or(addr);
    let mut result = socket_watcher_init(&mut g_st().main_loop, &mut watcher, endpoint);
    drop(addr_gen);
    if result < 0 {
        return result;
    }

    let already_listening = SERVER_STATE.with(|st| {
        st.watchers
            .borrow()
            .iter()
            .any(|existing| watcher.addr_bytes() == existing.addr_bytes())
    });
    if already_listening {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"server_start"),
            184,
            true,
            c"Already listening on %s",
            watcher.addr_cstr(),
        );

        watcher.free_tcp_addrinfo();

        socket_watcher_close(Box::leak(watcher));

        return 2;
    }

    result = socket_watcher_start(&mut watcher, MAX_CONNECTIONS);

    if result < 0 {
        logmsg!(
            LOGLVL_WRN,
            None,
            Some(c"server_start"),
            195,
            true,
            c"Failed to start server: %s: %s",
            uv_strerror(result),
            watcher.addr_cstr(),
        );

        socket_watcher_close(Box::leak(watcher));

        return result;
    }

    SERVER_STATE.with(|st| {
        st.watchers.borrow_mut().push(watcher);
    });

    if get_vim_var_str(VV_SEND_SERVER).to_bytes().is_empty() {
        set_vservername();
    }

    0
}

pub fn server_stop(endpoint: &CStr, keep_vservername: bool) -> bool {
    let mut addr: [u8; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];

    crate::memory::copy_bytes_to_c_buf(&mut addr, endpoint.to_bytes());
    to_slash(&mut addr);
    let addr_cstr = CStr::from_bytes_until_nul(&addr).unwrap_or(c"");
    let addr_bytes = addr_cstr.to_bytes();

    let watcher_opt = SERVER_STATE.with(|st| {
        let mut w = st.watchers.borrow_mut();
        w.iter()
            .position(|watcher| addr_bytes == watcher.addr_bytes())
            .map(|idx| w.swap_remove(idx))
    });
    let Some(watcher) = watcher_opt else {
        logmsg!(
            LOGLVL_WRN,
            None,
            Some(c"server_stop"),
            238,
            true,
            c"Not listening on %s",
            addr_cstr,
        );
        return false;
    };

    socket_watcher_close(Box::leak(watcher));

    if !keep_vservername && Some(addr_bytes) == Some(get_vim_var_str(VV_SEND_SERVER).to_bytes()) {
        set_vservername();
    }

    true
}

pub fn server_address_list() -> Vec<std::ffi::CString> {
    SERVER_STATE.with(|st| {
        st.watchers
            .borrow()
            .iter()
            .map(|watcher| watcher.addr_cstr().to_owned())
            .collect()
    })
}

pub(crate) fn connection_cb(watcher: &mut SocketWatcher, result: i32) {
    if result != 0 {
        logmsg!(
            LOGLVL_ERR,
            None,
            Some(c"connection_cb"),
            276,
            true,
            c"Failed to accept connection: %s",
            uv_strerror(result),
        );
        return;
    }

    channel_from_connection(watcher);
}
