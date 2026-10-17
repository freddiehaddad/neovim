//! Msgpack-RPC server address and socket watcher management (`src/nvim/msgpack_rpc/server.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::extmark::TriState;
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::memory::{xcalloc, xfree, xmalloc, xstrdup, xstrlcpy};
#[cfg(not(windows))]
use crate::os::stdpaths::{XDGVarType, stdpaths_get_xdg_var};

const MAX_CONNECTIONS: c_int = 32;
const ADDRESS_MAX_SIZE: usize = 256;
const IOSIZE: usize = 1025;
const MAXPATHL: usize = 4096;

const UV_NAMED_PIPE: c_int = 7;
const UV_TCP: c_int = 12;

const VV_SEND_SERVER: c_int = 28;

const LOGLVL_WRN: c_int = 3;
const LOGLVL_ERR: c_int = 4;

const kNone: TriState = -1;
const kFalse: TriState = 0;
const kTrue: TriState = 1;

const ENV_LISTEN: &CStr = c"NVIM_LISTEN_ADDRESS";
const ENV_TEST_LOG: &CStr = c"__NVIM_TEST_LOG";

pub type Loop = c_void;
pub type SocketWatcher = c_void;
pub type uv_handle_t = c_void;
pub type socket_cb =
    Option<unsafe extern "C" fn(watcher: *mut SocketWatcher, result: c_int, data: *mut c_void)>;
pub type socket_close_cb =
    Option<unsafe extern "C" fn(watcher: *mut SocketWatcher, data: *mut c_void)>;

static mut watchers: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: 0,
    ga_growsize: 0,
    ga_data: ptr::null_mut(),
};
static mut server_addr_count: u32 = 0;

unsafe extern "C" {
    static mut main_loop: u8;
    static mut IObuff: [c_char; IOSIZE];
    static mut NameBuff: [c_char; MAXPATHL];

    fn uv_handle_size(h_type: c_int) -> usize;
    fn uv_handle_get_type(handle: *const uv_handle_t) -> c_int;
    fn uv_freeaddrinfo(ai: *mut c_void);
    fn uv_strerror(err: c_int) -> *const c_char;

    fn os_env_exists(name: *const c_char, nonempty: bool) -> bool;
    fn os_getenv_noalloc(name: *const c_char) -> *const c_char;
    fn os_unsetenv(name: *const c_char) -> c_int;
    fn os_get_pid() -> i64;
    fn get_appname(namelike: bool) -> bool;
    fn fix_fname(fname: *const c_char) -> *mut c_char;
    #[cfg(windows)]
    fn path_to_slash(path: *mut c_char);
    #[cfg(windows)]
    fn path_to_slash_save(path: *const c_char) -> *mut c_char;

    fn set_vim_var_string(idx: c_int, val: *const c_char, len: isize);
    fn get_vim_var_str(idx: c_int) -> *mut c_char;

    fn socket_watcher_init(
        loop_: *mut Loop,
        watcher: *mut SocketWatcher,
        endpoint: *const c_char,
    ) -> c_int;
    fn socket_watcher_start(watcher: *mut SocketWatcher, backlog: c_int, cb: socket_cb) -> c_int;
    fn socket_watcher_close(watcher: *mut SocketWatcher, cb: socket_close_cb);
    fn channel_from_connection(watcher: *mut SocketWatcher);

    fn logmsg(
        log_level: c_int,
        context: *const c_char,
        func_name: *const c_char,
        line_num: c_int,
        eol: bool,
        fmt: *const c_char,
        ...
    ) -> bool;

    fn snprintf(s: *mut c_char, n: usize, format: *const c_char, ...) -> c_int;
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strlen(s: *const c_char) -> usize;
    fn strstr(haystack: *const c_char, needle: *const c_char) -> *const c_char;
}

#[inline]
unsafe fn strequal(a: *const c_char, b: *const c_char) -> bool {
    unsafe { strcmp(a, b) == 0 }
}

#[inline]
unsafe fn to_slash(path: *mut c_char) {
    #[cfg(windows)]
    unsafe {
        path_to_slash(path);
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

#[inline]
unsafe fn to_slash_save(path: *const c_char) -> *mut c_char {
    #[cfg(windows)]
    unsafe {
        path_to_slash_save(path)
    }
    #[cfg(not(windows))]
    unsafe {
        xstrdup(path)
    }
}

#[inline]
unsafe fn socket_watcher_uv_size() -> usize {
    unsafe {
        let tcp_size = uv_handle_size(UV_TCP) + std::mem::size_of::<*mut c_void>();
        let pipe_size = uv_handle_size(UV_NAMED_PIPE);
        tcp_size.max(pipe_size)
    }
}

#[inline]
unsafe fn socket_watcher_size() -> usize {
    unsafe { ADDRESS_MAX_SIZE + socket_watcher_uv_size() + 5 * std::mem::size_of::<*mut c_void>() }
}

#[inline]
unsafe fn watcher_addr(watcher: *mut SocketWatcher) -> *mut c_char {
    watcher.cast::<c_char>()
}

#[inline]
unsafe fn watcher_stream(watcher: *mut SocketWatcher) -> *mut uv_handle_t {
    unsafe {
        *watcher
            .cast::<u8>()
            .add(ADDRESS_MAX_SIZE + socket_watcher_uv_size())
            .cast::<*mut uv_handle_t>()
    }
}

#[inline]
unsafe fn watcher_tcp_addrinfo(watcher: *mut SocketWatcher) -> *mut c_void {
    unsafe {
        *watcher
            .cast::<u8>()
            .add(ADDRESS_MAX_SIZE + uv_handle_size(UV_TCP))
            .cast::<*mut c_void>()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_init(mut listen_addr: *const c_char) -> bool {
    unsafe {
        let mut ok = true;
        let mut must_free = false;
        let mut user_arg: TriState = kTrue;
        ga_init(
            &raw mut watchers,
            std::mem::size_of::<*mut SocketWatcher>() as c_int,
            1,
        );

        if listen_addr.is_null() || *listen_addr == 0 {
            if os_env_exists(ENV_LISTEN.as_ptr(), true) {
                user_arg = kFalse;
                listen_addr = to_slash_save(os_getenv_noalloc(ENV_LISTEN.as_ptr())).cast_const();
            } else {
                user_arg = kNone;
                listen_addr = server_address_new(ptr::null()).cast_const();
            }
            must_free = true;
        }

        let rv = server_start(listen_addr);

        if os_env_exists(ENV_TEST_LOG.as_ptr(), false) {
            logmsg(
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
            snprintf(
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

        if must_free {
            xfree(listen_addr.cast_mut().cast());
        }

        ok
    }
}

unsafe fn set_vservername(srvs: *const garray_T) {
    unsafe {
        let default_server = if (*srvs).ga_len > 0 {
            let first = *((*srvs).ga_data.cast::<*mut SocketWatcher>());
            watcher_addr(first).cast_const()
        } else {
            ptr::null()
        };
        set_vim_var_string(VV_SEND_SERVER, default_server, -1);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_teardown() {
    unsafe {
        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        for i in 0..watchers.ga_len {
            let watcher = *data.add(i as usize);
            socket_watcher_close(watcher, Some(free_server));
        }
        ga_clear(&raw mut watchers);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_address_new(name: *const c_char) -> *mut c_char {
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
            snprintf(
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
            let dir = stdpaths_get_xdg_var(XDGVarType::kXDGRuntimeDir);
            get_appname(true);
            let app = if !name.is_null() {
                name
            } else {
                ptr::addr_of!(NameBuff).cast()
            };
            let r = snprintf(
                fmt.as_mut_ptr(),
                fmt.len(),
                c"%s/%s.%llu.%u".as_ptr(),
                dir,
                app,
                os_get_pid() as u64,
                cur_count,
            );
            xfree(dir.cast());
            r
        };

        if (r as usize) >= fmt.len() {
            logmsg(
                LOGLVL_ERR,
                ptr::null(),
                c"server_address_new".as_ptr(),
                133,
                true,
                c"truncated server address: %.40s...".as_ptr(),
                fmt.as_ptr(),
            );
        }
        xstrdup(fmt.as_ptr())
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_owns_pipe_address(address: *const c_char) -> bool {
    unsafe {
        let mut result = false;
        let path = fix_fname(address);
        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        for i in 0..watchers.ga_len {
            let watcher = *data.add(i as usize);
            let addr = fix_fname(watcher_addr(watcher));
            result = strequal(path, addr);
            xfree(addr.cast());
            if result {
                break;
            }
        }
        xfree(path.cast());
        result
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_start(addr: *const c_char) -> c_int {
    unsafe {
        if addr.is_null() || *addr == 0 {
            logmsg(
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
            server_address_new(addr)
        } else {
            ptr::null_mut()
        };
        let watcher = xmalloc(socket_watcher_size()).cast::<SocketWatcher>();
        let endpoint = if isname { addr_gen.cast_const() } else { addr };
        let mut result =
            socket_watcher_init(ptr::addr_of_mut!(main_loop).cast(), watcher, endpoint);
        xfree(addr_gen.cast());
        if result < 0 {
            xfree(watcher.cast());
            return result;
        }

        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        for i in 0..watchers.ga_len {
            let existing = *data.add(i as usize);
            if strcmp(watcher_addr(watcher), watcher_addr(existing)) == 0 {
                logmsg(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"server_start".as_ptr(),
                    184,
                    true,
                    c"Already listening on %s".as_ptr(),
                    watcher_addr(watcher),
                );
                if uv_handle_get_type(watcher_stream(watcher)) == UV_TCP {
                    uv_freeaddrinfo(watcher_tcp_addrinfo(watcher));
                }
                socket_watcher_close(watcher, Some(free_server));
                return 2;
            }
        }

        result = socket_watcher_start(watcher, MAX_CONNECTIONS, Some(connection_cb));
        if result < 0 {
            logmsg(
                LOGLVL_WRN,
                ptr::null(),
                c"server_start".as_ptr(),
                195,
                true,
                c"Failed to start server: %s: %s".as_ptr(),
                uv_strerror(result),
                watcher_addr(watcher),
            );
            socket_watcher_close(watcher, Some(free_server));
            return result;
        }

        ga_grow(&raw mut watchers, 1);
        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        *data.add(watchers.ga_len as usize) = watcher;
        watchers.ga_len += 1;

        if strlen(get_vim_var_str(VV_SEND_SERVER)) == 0 {
            set_vservername(&raw const watchers);
        }

        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_stop(endpoint: *const c_char, keep_vservername: bool) -> bool {
    unsafe {
        let mut watcher: *mut SocketWatcher = ptr::null_mut();
        let mut watcher_found = false;
        let mut addr: [c_char; ADDRESS_MAX_SIZE] = [0; ADDRESS_MAX_SIZE];

        xstrlcpy(addr.as_mut_ptr(), endpoint, addr.len());
        to_slash(addr.as_mut_ptr());

        let mut idx: c_int = 0;
        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        while idx < watchers.ga_len {
            watcher = *data.add(idx as usize);
            if strcmp(addr.as_ptr(), watcher_addr(watcher)) == 0 {
                watcher_found = true;
                break;
            }
            idx += 1;
        }

        if !watcher_found {
            logmsg(
                LOGLVL_WRN,
                ptr::null(),
                c"server_stop".as_ptr(),
                238,
                true,
                c"Not listening on %s".as_ptr(),
                addr.as_ptr(),
            );
            return false;
        }

        socket_watcher_close(watcher, Some(free_server));

        if idx != watchers.ga_len - 1 {
            *data.add(idx as usize) = *data.add((watchers.ga_len - 1) as usize);
        }
        watchers.ga_len -= 1;

        if !keep_vservername && strequal(addr.as_ptr(), get_vim_var_str(VV_SEND_SERVER)) {
            set_vservername(&raw const watchers);
        }

        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn server_address_list(size: *mut usize) -> *mut *mut c_char {
    unsafe {
        *size = watchers.ga_len as usize;
        if *size == 0 {
            return ptr::null_mut();
        }

        let addrs = xcalloc(
            watchers.ga_len as usize,
            std::mem::size_of::<*const c_char>(),
        )
        .cast::<*mut c_char>();
        let data = watchers.ga_data.cast::<*mut SocketWatcher>();
        for i in 0..watchers.ga_len {
            let watcher = *data.add(i as usize);
            *addrs.add(i as usize) = xstrdup(watcher_addr(watcher));
        }
        addrs
    }
}

unsafe extern "C" fn connection_cb(watcher: *mut SocketWatcher, result: c_int, _data: *mut c_void) {
    unsafe {
        if result != 0 {
            logmsg(
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

unsafe extern "C" fn free_server(watcher: *mut SocketWatcher, _data: *mut c_void) {
    unsafe {
        xfree(watcher.cast());
    }
}
